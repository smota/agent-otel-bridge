/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::otlp::build_resource_with;
use agent_otel_core::semconv::DEPLOYMENT_ENVIRONMENT;
use opentelemetry_proto::tonic::resource::v1::Resource;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalExportConfig {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub gzip: bool,
    pub disabled_reason: Option<&'static str>,
    pub timeout: Duration,
    pub certificate: Option<String>,
}

impl SignalExportConfig {
    pub fn http_protobuf(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            gzip: false,
            disabled_reason: None,
            timeout: Duration::from_millis(5000),
            certificate: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub otlp_endpoint: String,
    pub service_name: String,
    pub environment: String,
    pub service_version: String,
    pub pipe_name: String,
    pub batch_size: usize,
    pub batch_timeout: Duration,
    pub quota_interval: Duration,
    pub idle_timeout: Duration,
    pub emit_legacy_aliases: bool,
    pub traces: SignalExportConfig,
    pub metrics: SignalExportConfig,
    pub sdk_disabled: bool,
    pub resource_attributes: Vec<(String, String)>,
    pub service_instance_id: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl DaemonConfig {
    pub fn from_env() -> Self {
        Self::from_vars(&|key| std::env::var(key).ok())
    }

    /// Builds a config from an explicit key/value source. Does not read process
    /// environment except for the platform default socket path when that key is
    /// absent, and for the one-time instance id fallback.
    pub fn from_vars(get: &dyn Fn(&str) -> Option<String>) -> Self {
        let export = parse_otlp_env(get);
        #[cfg(not(unix))]
        let pipe_name = non_empty(get("AGENT_OTEL_PIPE"))
            .or_else(|| non_empty(get("AGY_OTEL_PIPE")))
            .unwrap_or_else(|| r"\\.\pipe\agent-otel".to_string());
        #[cfg(unix)]
        let pipe_name = non_empty(get("AGENT_OTEL_SOCKET"))
            .unwrap_or_else(|| "/tmp/agent_otel_bridge.sock".to_string());

        let batch_size = get("AGENT_OTEL_BATCH_SIZE")
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);
        let batch_timeout_ms = get("AGENT_OTEL_BATCH_TIMEOUT_MS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);
        let quota_interval_secs = get("AGENT_OTEL_QUOTA_INTERVAL_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        let idle_timeout_secs = get("AGENT_OTEL_IDLE_TIMEOUT_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let idle_timeout = if idle_timeout_secs == 0 {
            Duration::MAX
        } else {
            Duration::from_secs(idle_timeout_secs)
        };
        let emit_legacy_aliases = get("AGENT_OTEL_LEGACY_ATTRIBUTES")
            .or_else(|| get("AGENT_OTEL_EMIT_AGY_ALIASES"))
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Self {
            otlp_endpoint: export.base_endpoint,
            service_name: export.service_name,
            environment: export.environment,
            service_version: export.service_version,
            pipe_name,
            batch_size,
            batch_timeout: Duration::from_millis(batch_timeout_ms),
            quota_interval: Duration::from_secs(quota_interval_secs),
            idle_timeout,
            emit_legacy_aliases,
            traces: export.traces,
            metrics: export.metrics,
            sdk_disabled: export.sdk_disabled,
            resource_attributes: export.resource_attributes,
            service_instance_id: new_instance_id(),
        }
    }

    pub fn resource(&self) -> Resource {
        let extra: Vec<(&str, &str)> = self
            .resource_attributes
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        build_resource_with(
            &self.service_name,
            &self.environment,
            &self.service_version,
            &self.service_instance_id,
            &extra,
        )
    }

    pub fn traces_url(&self) -> String {
        self.traces.url.clone()
    }

    pub fn metrics_url(&self) -> String {
        self.metrics.url.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedOtlpEnv {
    pub base_endpoint: String,
    pub service_name: String,
    pub service_version: String,
    pub environment: String,
    pub resource_attributes: Vec<(String, String)>,
    pub traces: SignalExportConfig,
    pub metrics: SignalExportConfig,
    pub sdk_disabled: bool,
}

pub fn parse_otlp_env(get: &dyn Fn(&str) -> Option<String>) -> ParsedOtlpEnv {
    let base_endpoint = non_empty(get("OTEL_EXPORTER_OTLP_ENDPOINT"))
        .unwrap_or_else(|| "http://127.0.0.1:4318".to_string());
    let service_name =
        non_empty(get("OTEL_SERVICE_NAME")).unwrap_or_else(|| "agent-otel-bridge".to_string());
    let service_version = non_empty(get("OTEL_SERVICE_VERSION"))
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let resource_attributes = get("OTEL_RESOURCE_ATTRIBUTES")
        .map(|raw| split_baggage(&raw))
        .unwrap_or_default();
    let environment = resource_attributes
        .iter()
        .rev()
        .find(|(key, _)| key == DEPLOYMENT_ENVIRONMENT)
        .filter(|(_, value)| !value.is_empty())
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    ParsedOtlpEnv {
        traces: parse_signal(get, &base_endpoint, "TRACES", "v1/traces"),
        metrics: parse_signal(get, &base_endpoint, "METRICS", "v1/metrics"),
        sdk_disabled: get("OTEL_SDK_DISABLED")
            .map(|value| value.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        base_endpoint,
        service_name,
        service_version,
        environment,
        resource_attributes,
    }
}

fn parse_signal(
    get: &dyn Fn(&str) -> Option<String>,
    base_endpoint: &str,
    signal: &str,
    suffix: &str,
) -> SignalExportConfig {
    let url = match non_empty(get(&format!("OTEL_EXPORTER_OTLP_{signal}_ENDPOINT"))) {
        Some(endpoint) => endpoint,
        None => {
            let base = base_endpoint.strip_suffix('/').unwrap_or(base_endpoint);
            format!("{base}/{suffix}")
        }
    };
    let headers = merge_headers(
        &parse_headers(&get("OTEL_EXPORTER_OTLP_HEADERS").unwrap_or_default()),
        &parse_headers(&get(&format!("OTEL_EXPORTER_OTLP_{signal}_HEADERS")).unwrap_or_default()),
    );
    let compression = parse_compression(signal_override(
        get,
        &format!("OTEL_EXPORTER_OTLP_{signal}_COMPRESSION"),
        "OTEL_EXPORTER_OTLP_COMPRESSION",
    ));
    let protocol_ok = protocol_supported(signal_override(
        get,
        &format!("OTEL_EXPORTER_OTLP_{signal}_PROTOCOL"),
        "OTEL_EXPORTER_OTLP_PROTOCOL",
    ));
    let disabled_reason = if !protocol_ok {
        Some("unsupported_protocol")
    } else if compression == CompressionChoice::Unsupported {
        Some("unsupported_compression")
    } else {
        None
    };
    SignalExportConfig {
        url,
        headers,
        gzip: compression == CompressionChoice::Gzip && disabled_reason.is_none(),
        disabled_reason,
        timeout: parse_timeout(get, signal),
        certificate: signal_override(
            get,
            &format!("OTEL_EXPORTER_OTLP_{signal}_CERTIFICATE"),
            "OTEL_EXPORTER_OTLP_CERTIFICATE",
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompressionChoice {
    None,
    Gzip,
    Unsupported,
}

fn parse_compression(value: Option<String>) -> CompressionChoice {
    match value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        None => CompressionChoice::None,
        Some(v) if v.eq_ignore_ascii_case("none") => CompressionChoice::None,
        Some(v) if v.eq_ignore_ascii_case("gzip") => CompressionChoice::Gzip,
        Some(_) => CompressionChoice::Unsupported,
    }
}

fn protocol_supported(value: Option<String>) -> bool {
    match value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        None => true,
        Some(v) if v.eq_ignore_ascii_case("http/protobuf") => true,
        Some(_) => false,
    }
}

fn parse_timeout(get: &dyn Fn(&str) -> Option<String>, signal: &str) -> Duration {
    let specific = format!("OTEL_EXPORTER_OTLP_{signal}_TIMEOUT");
    let raw = match get(&specific) {
        Some(value) if !value.trim().is_empty() => Some(value),
        _ => non_empty(get("OTEL_EXPORTER_OTLP_TIMEOUT")),
    };
    match raw {
        Some(value) => value
            .trim()
            .parse::<u64>()
            .ok()
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_millis(5000)),
        None => Duration::from_millis(5000),
    }
}

fn signal_override(
    get: &dyn Fn(&str) -> Option<String>,
    specific: &str,
    general: &str,
) -> Option<String> {
    match get(specific) {
        Some(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        _ => non_empty(get(general)),
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub fn split_baggage(raw: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        pairs.push((key.to_string(), percent_decode(value.trim())));
    }
    pairs
}

pub fn parse_headers(raw: &str) -> Vec<(String, String)> {
    split_baggage(raw)
        .into_iter()
        .filter(|(key, value)| valid_header_name(key) && valid_header_value(value))
        .collect()
}

fn merge_headers(
    general: &[(String, String)],
    specific: &[(String, String)],
) -> Vec<(String, String)> {
    let mut merged = general.to_vec();
    for (key, value) in specific {
        if let Some(existing) = merged
            .iter_mut()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
        {
            *existing = (key.clone(), value.clone());
        } else {
            merged.push((key.clone(), value.clone()));
        }
    }
    merged
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            matches!(
                byte,
                b'0'..=b'9'
                    | b'a'..=b'z'
                    | b'A'..=b'Z'
                    | b'!'
                    | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'|'
                    | b'~'
            )
        })
}

fn valid_header_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte == b'\t' || (32..127).contains(&byte))
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Some(decoded) = hex_byte(&bytes[index + 1..index + 3]) {
                out.push(decoded);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_byte(hex: &[u8]) -> Option<u8> {
    let high = hex_value(hex.first().copied()?)?;
    let low = hex_value(hex.get(1).copied()?)?;
    Some((high << 4) | low)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn new_instance_id() -> String {
    let mut bytes = [0_u8; 16];
    if getrandom::getrandom(&mut bytes).is_err() {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::time::SystemTime::now().hash(&mut hasher);
        std::process::id().hash(&mut hasher);
        if let Ok(host) = std::env::var("HOSTNAME") {
            host.hash(&mut hasher);
        }
        let mixed = hasher.finish();
        bytes[..8].copy_from_slice(&mixed.to_le_bytes());
        bytes[8..].copy_from_slice(&mixed.rotate_left(17).to_le_bytes());
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut id = String::with_capacity(32);
    for byte in bytes {
        id.push(HEX[(byte >> 4) as usize] as char);
        id.push(HEX[(byte & 0x0f) as usize] as char);
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_otel_core::semconv::{
        DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_INSTANCE_ID, SERVICE_NAME, TELEMETRY_DISTRO_NAME,
        TELEMETRY_SDK_LANGUAGE, TELEMETRY_SDK_NAME, TELEMETRY_SDK_VERSION,
    };
    use opentelemetry_proto::tonic::common::v1::any_value;

    fn var_map<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .rev()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        }
    }

    fn attr<'a>(resource: &'a Resource, key: &str) -> Option<&'a str> {
        resource
            .attributes
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.as_ref())
            .and_then(|value| match &value.value {
                Some(any_value::Value::StringValue(text)) => Some(text.as_str()),
                _ => None,
            })
    }

    #[test]
    fn signal_endpoint_is_verbatim_and_base_appends_paths() {
        let config = DaemonConfig::from_vars(&var_map(&[
            (
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                "http://127.0.0.1:4318/v1/traces/",
            ),
            (
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                "https://traces.example/custom",
            ),
        ]));
        assert_eq!(config.traces_url(), "https://traces.example/custom");
        assert_eq!(
            config.metrics_url(),
            "http://127.0.0.1:4318/v1/traces/v1/metrics"
        );

        let base = DaemonConfig::from_vars(&var_map(&[(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://collector:4318/",
        )]));
        assert_eq!(base.traces_url(), "http://collector:4318/v1/traces");
        assert_eq!(base.metrics_url(), "http://collector:4318/v1/metrics");

        let defaulted = DaemonConfig::from_vars(&var_map(&[]));
        assert_eq!(defaulted.traces_url(), "http://127.0.0.1:4318/v1/traces");
        assert_eq!(defaulted.metrics_url(), "http://127.0.0.1:4318/v1/metrics");
    }

    #[test]
    fn headers_merge_and_percent_decode() {
        let config = DaemonConfig::from_vars(&var_map(&[
            (
                "OTEL_EXPORTER_OTLP_HEADERS",
                "Authorization=Bearer%20abc, X-Test=old, bad name=skip, empty=, ok=a%2Cb%3Dc%25",
            ),
            ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", "x-test=new"),
        ]));
        assert_eq!(
            config.traces.headers,
            vec![
                ("Authorization".to_string(), "Bearer abc".to_string()),
                ("x-test".to_string(), "new".to_string()),
                ("empty".to_string(), String::new()),
                ("ok".to_string(), "a,b=c%".to_string()),
            ]
        );
        assert_eq!(
            config.metrics.headers,
            vec![
                ("Authorization".to_string(), "Bearer abc".to_string()),
                ("X-Test".to_string(), "old".to_string()),
                ("empty".to_string(), String::new()),
                ("ok".to_string(), "a,b=c%".to_string()),
            ]
        );
    }

    #[test]
    fn timeout_parse_falls_back_to_five_seconds() {
        let invalid = DaemonConfig::from_vars(&var_map(&[("OTEL_EXPORTER_OTLP_TIMEOUT", "nope")]));
        assert_eq!(invalid.traces.timeout, Duration::from_millis(5000));
        let specific = DaemonConfig::from_vars(&var_map(&[
            ("OTEL_EXPORTER_OTLP_TIMEOUT", "1500"),
            ("OTEL_EXPORTER_OTLP_TRACES_TIMEOUT", "2500"),
            ("OTEL_EXPORTER_OTLP_METRICS_TIMEOUT", "later"),
        ]));
        assert_eq!(specific.traces.timeout, Duration::from_millis(2500));
        assert_eq!(specific.metrics.timeout, Duration::from_millis(5000));
        let unset = DaemonConfig::from_vars(&var_map(&[]));
        assert_eq!(unset.traces.timeout, Duration::from_millis(5000));
    }

    #[test]
    fn resource_uses_service_name_and_attribute_merge() {
        let config = DaemonConfig::from_vars(&var_map(&[
            ("OTEL_SERVICE_NAME", "explicit"),
            ("OTEL_SERVICE_VERSION", "1.2.3"),
            (
                "OTEL_RESOURCE_ATTRIBUTES",
                "service.name=from-list,deployment.environment.name=prod,telemetry.sdk.name=opentelemetry,service.version=from-attrs,service.instance.id=operator-choice",
            ),
        ]));
        assert!(config.environment.is_empty());
        let resource = config.resource();
        assert_eq!(attr(&resource, SERVICE_NAME), Some("explicit"));
        assert_eq!(attr(&resource, DEPLOYMENT_ENVIRONMENT_NAME), Some("prod"));
        assert_eq!(attr(&resource, DEPLOYMENT_ENVIRONMENT), None);
        assert_eq!(
            attr(&resource, TELEMETRY_SDK_NAME),
            Some("agent-otel-bridge")
        );
        assert_eq!(attr(&resource, TELEMETRY_SDK_LANGUAGE), Some("rust"));
        assert_eq!(attr(&resource, TELEMETRY_SDK_VERSION), Some("1.2.3"));
        assert_eq!(
            attr(&resource, TELEMETRY_DISTRO_NAME),
            Some("agent-otel-bridge")
        );
        assert_eq!(attr(&resource, "service.version"), Some("from-attrs"));
        assert_eq!(
            attr(&resource, SERVICE_INSTANCE_ID),
            Some("operator-choice")
        );
        assert_eq!(config.service_instance_id.len(), 32);
        assert!(config
            .service_instance_id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn unsupported_protocol_or_compression_disables_only_that_signal() {
        let config = DaemonConfig::from_vars(&var_map(&[
            ("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "grpc"),
            ("OTEL_EXPORTER_OTLP_TRACES_COMPRESSION", "gzip"),
            ("OTEL_EXPORTER_OTLP_METRICS_COMPRESSION", "deflate"),
        ]));
        assert_eq!(config.traces.disabled_reason, Some("unsupported_protocol"));
        assert!(!config.traces.gzip);
        assert_eq!(
            config.metrics.disabled_reason,
            Some("unsupported_compression")
        );
        assert!(!config.metrics.gzip);
        assert!(!config.sdk_disabled);
    }
}
