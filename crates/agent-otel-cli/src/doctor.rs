/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_ipc::client::try_send;
use agent_otel_ipc::frame::MsgType;
use reqwest::{StatusCode, Url};
use std::time::Duration;

fn sanitize_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw) else {
        return "<invalid URL redacted>".to_string();
    };

    let _ = url.set_username("");
    let _ = url.set_password(None);
    if url.query().is_some() {
        url.set_query(Some("redacted"));
    }
    if url.fragment().is_some() {
        url.set_fragment(Some("<redacted>"));
    }
    url.to_string()
}

fn sanitize_resource_attributes(raw: &str) -> String {
    raw.split(',')
        .map(|attribute| {
            attribute
                .split_once('=')
                .map(|(key, _)| format!("{key}=<redacted>"))
                .unwrap_or_else(|| "<redacted>".to_string())
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn format_http_probe(level: &str, service: &str, url: &str, status: StatusCode) -> String {
    if status.is_success() {
        format!("  [{level}] {service} reachable at {url} (HTTP status: {status})")
    } else {
        format!(
            "  [warn] {service} responded at {url} (HTTP status: {status}; request was not accepted)"
        )
    }
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== agent-otel-bridge doctor ===");
    println!("Diagnosing station telemetry pipeline & OpenTelemetry invariants\n");

    // 1. Environment variables check
    println!("[1/5] Checking environment contract variables...");
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:4318".to_string());
    println!(
        "  [ok] OTEL_EXPORTER_OTLP_ENDPOINT = {}",
        sanitize_url(&endpoint)
    );

    match std::env::var("OTEL_SERVICE_NAME") {
        Ok(v) => println!("  [ok] OTEL_SERVICE_NAME = {} (per-process)", v),
        Err(_) => {
            println!("  [info] OTEL_SERVICE_NAME is unset (using default 'agent-otel-bridge')")
        }
    }

    match std::env::var("OTEL_RESOURCE_ATTRIBUTES") {
        Ok(v) => println!(
            "  [ok] OTEL_RESOURCE_ATTRIBUTES = {}",
            sanitize_resource_attributes(&v)
        ),
        Err(_) => println!("  [info] OTEL_RESOURCE_ATTRIBUTES unset (defaulting to 'deployment.environment=homelab')"),
    }

    // 2. Named Pipe IPC check
    let pipe_name = std::env::var("AGENT_OTEL_PIPE")
        .or_else(|_| std::env::var("AGY_OTEL_PIPE"))
        .unwrap_or_else(|_| agent_otel_ipc::frame::DEFAULT_PIPE_NAME.to_string());
    println!("\n[2/5] Checking named pipe IPC ({})...", pipe_name);
    let ping_res = try_send(MsgType::HealthPing, b"doctor-probe");
    match ping_res {
        Ok(()) => println!("  [ok] Daemon is RUNNING and responding on named pipe!"),
        Err(()) => {
            println!("  [warn] Daemon is NOT currently running (pipe not found or busy).");
            println!(
                "         Run 'agent-otel-bridge start' or invoke an agent hook to auto-launch it."
            );
        }
    }

    // 3. OTLP Collector connectivity check
    println!("\n[3/5] Checking OTLP Collector HTTP endpoint...");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;

    let traces_url = format!("{}/v1/traces", endpoint.trim_end_matches('/'));
    match client.post(&traces_url).body(vec![]).send().await {
        Ok(resp) => {
            println!(
                "{}",
                format_http_probe(
                    "ok",
                    "OTLP Collector",
                    &sanitize_url(&traces_url),
                    resp.status()
                )
            );
        }
        Err(e) => {
            println!(
                "  [fail] OTLP Collector UNREACHABLE at {}: {}",
                sanitize_url(&traces_url),
                e.without_url()
            );
        }
    }

    // 4. Observability Dashboard / UI reachability check
    let ui_url = std::env::var("OTEL_UI_URL")
        .or_else(|_| std::env::var("SIGNOZ_UI_URL"))
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    println!(
        "\n[4/5] Checking Observability UI reachability ({})...",
        sanitize_url(&ui_url)
    );
    match client.get(&ui_url).send().await {
        Ok(resp) => {
            println!(
                "{}",
                format_http_probe(
                    "ok",
                    "Observability UI",
                    &sanitize_url(&ui_url),
                    resp.status()
                )
            );
            if ui_url.contains("localhost:8080") {
                println!(
                    "  [dashboard] {}/dashboard/01a091c1-c5c7-7105-9963-436f893fa832 (AI Agent Observability)",
                    sanitize_url(ui_url.trim_end_matches('/'))
                );
            }
        }
        Err(e) => {
            println!(
                "  [info] Observability UI not reachable at {} (optional): {}",
                sanitize_url(&ui_url),
                e.without_url()
            );
        }
    }

    // 5. Client Hook Registrations & Local Runtime check
    println!("\n[5/5] Checking Client Hook Registrations & Local Runtime...");
    let canonical_hook = crate::local::get_canonical_hook_path();
    let canonical_bridge = crate::local::get_canonical_bridge_path();
    let local_ok = canonical_hook.is_file() && canonical_bridge.is_file();
    println!(
        "  Local runtime installation: {}",
        if local_ok {
            format!(
                "[ok] present at {}",
                crate::local::get_canonical_bin_dir().display()
            )
        } else {
            "[info] not installed (run 'agent-otel-bridge local install')".to_string()
        }
    );

    let hook_in_path = crate::hooks::check_binary_in_path("agent-hook");
    println!(
        "  agent-hook in PATH:         {}",
        if hook_in_path {
            "[ok] present"
        } else {
            "[info] not in PATH (not required when absolute path is configured)"
        }
    );

    for adapter in crate::hooks::CLIENT_ADAPTERS {
        if let Some(p) = (adapter.global_config_fn)() {
            let configured = (adapter.is_registered_fn)(&p);
            println!(
                "  {:<20} {}",
                format!("{}:", adapter.display_name),
                if configured {
                    format!("[ok] registered ({})", p.display())
                } else {
                    format!("[info] not registered ({})", p.display())
                }
            );
        }
    }

    // Workspace shadowing diagnostic for current directory
    if let Ok(current_dir) = std::env::current_dir() {
        for adapter in crate::hooks::CLIENT_ADAPTERS {
            if adapter.scope == crate::hooks::HookScope::ProjectShadowsGlobal {
                if let Some(p) = (adapter.project_config_fn)(Some(&current_dir)) {
                    if p.exists() && !(adapter.is_registered_fn)(&p) {
                        println!(
                            "\n  [warn] Workspace has local {} without bridge hook (shadows global telemetry!)",
                            p.display()
                        );
                        println!(
                            "         Run 'agent-otel-bridge hooks sync' to activate telemetry in this workspace."
                        );
                    }
                }
            }
        }
    }

    println!("\nDoctor check completed.\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_http_errors_are_not_reported_as_ok() {
        let output = format_http_probe(
            "ok",
            "OTLP Collector",
            "http://collector/v1/traces",
            StatusCode::INTERNAL_SERVER_ERROR,
        );
        assert!(output.starts_with("  [warn]"));
        assert!(output.contains("request was not accepted"));

        let output = format_http_probe(
            "ok",
            "OTLP Collector",
            "http://collector/v1/traces",
            StatusCode::UNAUTHORIZED,
        );
        assert!(output.starts_with("  [warn]"));
        assert!(!output.contains("[ok]"));
    }

    #[test]
    fn sensitive_url_parts_are_redacted() {
        let output =
            sanitize_url("https://user:password@example.test/v1/traces?api_key=secret&token=abc");
        assert_eq!(output, "https://example.test/v1/traces?redacted");
        assert!(!output.contains("password"));
        assert!(!output.contains("secret"));
        assert_eq!(
            sanitize_url("not a URL user:password@example.test"),
            "<invalid URL redacted>"
        );
    }

    #[test]
    fn resource_attribute_values_are_redacted() {
        let output =
            sanitize_resource_attributes("deployment.environment=production,api.key=secret");
        assert_eq!(
            output,
            "deployment.environment=<redacted>,api.key=<redacted>"
        );
        assert!(!output.contains("production"));
        assert!(!output.contains("secret"));
    }
}
