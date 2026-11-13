/* Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0 */

use agent_otel_core::link_id::{hex_decode, hex_encode, is_valid_ascii_slug};
use agent_otel_core::model::{ExecutionMode, HookEvent};
use agent_otel_core::otlp::ResolvedSpanMetadata;
use agent_otel_core::trace_id::{derive_trace_id, parse_w3c_traceparent_with_flags};
use agent_otel_daemon::config::WorkspacePathMode;
use agent_otel_daemon::context_cache::ContextCache;
use agent_otel_daemon::privacy::CaptureMode;
use agent_otel_daemon::session_link::SessionLinkConfig;
use agent_otel_daemon::transform::production_transform_with_session_link;
use agent_otel_ipc::frame::{encode_context_payload, MsgType, WireHeader};
use opentelemetry_proto::tonic::common::v1::any_value;
use opentelemetry_proto::tonic::trace::v1::Span;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

const KEY: &str = "c4ed91ad0f2d1b87a067093c0a2ecea930c784850d759756530a98a29fb331ff";

fn vars() -> HashMap<String, String> {
    [
        ("AGENT_OTEL_LINK_KEY_HEX", KEY),
        ("AGENT_OTEL_LINK_ORG", "ORG_SENTINEL"),
        ("AGENT_OTEL_LINK_ENDPOINT_SCOPE", "ENDPOINT_SENTINEL"),
        ("AGENT_OTEL_LINK_TEAM", "TEAM_SENTINEL"),
        ("AGENT_OTEL_LINK_EPOCH", "2026-Q4"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

fn config(vars: &HashMap<String, String>) -> Result<SessionLinkConfig, &'static str> {
    SessionLinkConfig::from_vars(&|key| vars.get(key).cloned())
}

fn run(
    config: Option<&SessionLinkConfig>,
    client: u16,
    event: HookEvent,
    body: &Value,
    inherited: Option<&str>,
    mode: CaptureMode,
) -> Span {
    let frame = encode_context_payload(
        WireHeader::new(client, event.to_wire()),
        inherited,
        &serde_json::to_vec(body).unwrap(),
    );
    production_transform_with_session_link(
        MsgType::HookPayloadWithContext,
        &frame,
        &ContextCache::new(),
        7,
        ExecutionMode::Interactive,
        false,
        false,
        ResolvedSpanMetadata::default(),
        Instant::now(),
        42,
        WorkspacePathMode::Hash,
        mode,
        None,
        config,
    )
    .unwrap()
    .span
}

fn attr<'a>(span: &'a Span, key: &str) -> Option<&'a str> {
    span.attributes.iter().find_map(|kv| {
        if kv.key != key {
            return None;
        }
        match kv.value.as_ref()?.value.as_ref()? {
            any_value::Value::StringValue(s) => Some(s.as_str()),
            _ => None,
        }
    })
}

fn suite(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/contracts/fact-export-v1/link-id-v1/vectors")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn shared_vectors_through_production_daemon_boundary() {
    let mut linked = 0;
    let mut unlinked = 0;
    let mut refused = 0;
    for name in ["basic.json", "g0a.json", "negative.json"] {
        for vector in suite(name)["vectors"].as_array().unwrap() {
            let input = &vector["inputs"];
            let mut provision = HashMap::new();
            for (env, field) in [
                ("AGENT_OTEL_LINK_ORG", "org"),
                ("AGENT_OTEL_LINK_ENDPOINT_SCOPE", "endpoint_scope"),
                ("AGENT_OTEL_LINK_TEAM", "team"),
                ("AGENT_OTEL_LINK_EPOCH", "epoch"),
            ] {
                if let Some(s) = input[field].as_str() {
                    provision.insert(env.into(), s.into());
                }
            }
            if let Some(key) = vector["intermediates"]["k_link_hex"].as_str() {
                provision.insert("AGENT_OTEL_LINK_KEY_HEX".into(), key.into());
            }
            let configured = config(&provision);
            // Administrative grammar violations are refused at startup, before event ingress.
            if ["org", "endpoint_scope", "team", "epoch"]
                .iter()
                .any(|field| {
                    input[*field]
                        .as_str()
                        .is_some_and(|s| !is_valid_ascii_slug(s))
                })
            {
                assert!(configured.is_err(), "{} provisioning", vector["id"]);
                refused += 1;
                continue;
            }
            let configured = configured.unwrap();
            let wire = input["wire_client_id"].as_u64().unwrap_or(0) as u16;
            // The two-source resolver's conflict/alias checks do not apply to A2:
            // nonzero wire IDs are authoritative; payload agent_name may be a role (SPEC 5.2.3).
            if wire != 0
                && matches!(
                    vector["expected"]["unlinked_reason"].as_str(),
                    Some("conflicting_harness" | "unknown_harness")
                )
                && wire <= 5
            {
                continue;
            }
            let body = json!({"session_id":input["native_session"], "agent_name":input["raw_harness"], "origin":input["origin"], "model":"gemini-poison", "provider":"google", "gen_ai.system":"google"});
            let span = run(
                Some(&configured),
                wire,
                HookEvent::PostToolUse,
                &body,
                None,
                CaptureMode::Minimal,
            );
            assert_eq!(
                attr(&span, "agent.session.link_id"),
                vector["expected"]["link_id"].as_str(),
                "{name}:{}",
                vector["id"]
            );
            assert_eq!(
                attr(&span, "agent.session.unlinked_reason"),
                vector["expected"]["unlinked_reason"].as_str(),
                "{name}:{}",
                vector["id"]
            );
            if vector["expected"]["unlinked_reason"].is_null() {
                linked += 1;
            } else {
                unlinked += 1;
            }
        }
    }
    assert!(linked >= 10 && unlinked >= 10 && refused >= 5);
}

#[test]
fn g0a_claude_s2_sessions_are_distinct_and_events_stable() {
    let doc = suite("g0a.json");
    let sessions: Vec<_> = doc["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| {
            v["inputs"]["wire_client_id"].as_u64() == Some(2)
                && v["expected"]["unlinked_reason"].is_null()
        })
        .collect();
    let mut ids = HashMap::new();
    for vector in sessions {
        let i = &vector["inputs"];
        let mut provision = vars();
        for (env, field) in [
            ("AGENT_OTEL_LINK_ORG", "org"),
            ("AGENT_OTEL_LINK_ENDPOINT_SCOPE", "endpoint_scope"),
            ("AGENT_OTEL_LINK_TEAM", "team"),
            ("AGENT_OTEL_LINK_EPOCH", "epoch"),
        ] {
            provision.insert(env.into(), i[field].as_str().unwrap().into());
        }
        provision.insert(
            "AGENT_OTEL_LINK_KEY_HEX".into(),
            vector["intermediates"]["k_link_hex"]
                .as_str()
                .unwrap()
                .into(),
        );
        let configured = config(&provision).unwrap();
        for event in [
            HookEvent::PreToolUse,
            HookEvent::PostToolUse,
            HookEvent::Stop,
        ] {
            let span = run(
                Some(&configured),
                2,
                event,
                &json!({"session_id":i["native_session"], "agent_name":"planner", "provider":"google"}),
                None,
                CaptureMode::Minimal,
            );
            let id = attr(&span, "agent.session.link_id").unwrap();
            assert_eq!(id, vector["expected"]["link_id"].as_str().unwrap());
            ids.insert(
                i["native_session"].as_str().unwrap().to_owned(),
                id.to_owned(),
            );
        }
    }
    assert!(ids.len() >= 2);
    assert_eq!(
        ids.values().collect::<std::collections::HashSet<_>>().len(),
        ids.len()
    );
}

#[test]
fn provider_poisoning_and_subagent_roles_do_not_change_harness() {
    let cfg = config(&vars()).unwrap();
    let normal = run(
        Some(&cfg),
        2,
        HookEvent::PostToolUse,
        &json!({"session_id":"session-sentinel", "agent_name":"claude-code"}),
        None,
        CaptureMode::Minimal,
    );
    for mode in [
        CaptureMode::Minimal,
        CaptureMode::MetadataOnly,
        CaptureMode::Full,
    ] {
        let poison = run(
            Some(&cfg),
            2,
            HookEvent::PostToolUse,
            &json!({"session_id":"session-sentinel", "agent_name":"planner", "model":"gemini-2.5-pro", "provider":"google", "gen_ai.provider.name":"google", "gen_ai.system":"google"}),
            None,
            mode,
        );
        assert_eq!(
            attr(&normal, "agent.session.link_id"),
            attr(&poison, "agent.session.link_id")
        );
        assert_ne!(attr(&poison, "agent.session.link_id"), Some("unlinked"));
    }
    let legacy = run(
        Some(&cfg),
        0,
        HookEvent::PostToolUse,
        &json!({"session_id":"session-sentinel", "agent_name":"claude-code"}),
        None,
        CaptureMode::Minimal,
    );
    assert_eq!(
        attr(&normal, "agent.session.link_id"),
        attr(&legacy, "agent.session.link_id")
    );
    for wire in [0, 6, 99] {
        let unknown = run(
            Some(&cfg),
            wire,
            HookEvent::PostToolUse,
            &json!({"session_id":"session-sentinel", "agent_name":"google"}),
            None,
            CaptureMode::Minimal,
        );
        assert_eq!(
            attr(&unknown, "agent.session.unlinked_reason"),
            Some("unknown_harness")
        );
    }
}

#[test]
fn inherited_trace_and_parent_and_flags_are_preserved_with_link() {
    let cfg = config(&vars()).unwrap();
    let origin = "00-11111111111111111111111111111111-2222222222222222-00";
    let explicit = "00-33333333333333333333333333333333-4444444444444444-01";
    for payload in [None, Some(explicit), Some("invalid")] {
        let body = json!({"session_id":"SESSION_SENTINEL with spaces", "traceparent":payload});
        let before = run(
            None,
            2,
            HookEvent::PostToolUse,
            &body,
            Some(origin),
            CaptureMode::Minimal,
        );
        let after = run(
            Some(&cfg),
            2,
            HookEvent::PostToolUse,
            &body,
            Some(origin),
            CaptureMode::Minimal,
        );
        let expected =
            parse_w3c_traceparent_with_flags(payload.filter(|s| *s != "invalid").unwrap_or(origin))
                .unwrap();
        assert_eq!(after.trace_id, expected.trace_id);
        assert_eq!(after.parent_span_id, expected.parent_span_id.unwrap());
        assert_eq!(before.trace_id, after.trace_id);
        assert_eq!(before.parent_span_id, after.parent_span_id);
        assert_eq!(before.flags, after.flags);
        assert_eq!(after.flags & 1, u32::from(expected.trace_flags & 1));
        assert_eq!(attr(&after, "agent.session.link_id").unwrap().len(), 64);
    }
}

#[test]
fn keyed_fallback_trace_and_minimal_serialized_output_reveal_no_plaintext() {
    use prost::Message;
    let cfg = config(&vars()).unwrap();
    let native = "SESSION_SENTINEL with spaces";
    let body = json!({"session_id":native, "org":"ORG_SENTINEL", "team":"TEAM_SENTINEL", "endpoint_scope":"ENDPOINT_SENTINEL"});
    let mut span = run(
        Some(&cfg),
        2,
        HookEvent::PostToolUse,
        &body,
        None,
        CaptureMode::Minimal,
    );
    let linked = attr(&span, "agent.session.link_id").unwrap().to_owned();
    assert_eq!(span.trace_id, hex_decode(&linked[..32]).unwrap());
    assert_ne!(span.trace_id, derive_trace_id(Some(native)));
    assert!(span.parent_span_id.is_empty());
    agent_otel_daemon::privacy::project_span(&mut span, CaptureMode::Minimal);
    assert_eq!(attr(&span, "agent.session.link_id"), Some(linked.as_str()));
    let bytes = span.encode_to_vec();
    let debug = format!("{span:?}");
    for secret in [
        native,
        "ORG_SENTINEL",
        "ENDPOINT_SENTINEL",
        "TEAM_SENTINEL",
        KEY,
    ] {
        assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
        assert!(!debug.contains(secret));
    }
    let unkeyed = run(
        None,
        2,
        HookEvent::PostToolUse,
        &body,
        None,
        CaptureMode::Minimal,
    );
    assert_eq!(unkeyed.trace_id, derive_trace_id(Some(native)));
    assert_eq!(
        attr(&unkeyed, "agent.session.unlinked_reason"),
        Some("invalid_key_material")
    );
    // A valid key still protects fallback traces when a link cannot be formed.
    let unknown = run(
        Some(&cfg),
        99,
        HookEvent::PostToolUse,
        &body,
        None,
        CaptureMode::Minimal,
    );
    assert_ne!(unknown.trace_id, unkeyed.trace_id);
    assert_eq!(attr(&unknown, "agent.session.link_id"), Some("unlinked"));
}

#[test]
fn provisioning_refuses_root_keys_and_invalid_slugs_and_redacts_debug() {
    for root_env in ["AGENT_OTEL_LINK_K_ORG_HEX", "AGENT_OTEL_K_ORG_HEX"] {
        let mut source = vars();
        source.insert(root_env.into(), "ROOT_SECRET_SENTINEL".into());
        let error = config(&source).unwrap_err();
        assert!(!error.contains("ROOT_SECRET_SENTINEL"));
    }
    for field in ["ORG", "ENDPOINT_SCOPE", "TEAM", "EPOCH"] {
        for invalid in [" ", "bad slug", "ümlaut", "\tbad"] {
            let mut source = vars();
            source.insert(format!("AGENT_OTEL_LINK_{field}"), invalid.into());
            assert!(config(&source).is_err());
        }
    }
    for invalid in ["", "not-hex", "00", &"a".repeat(65)] {
        let mut source = vars();
        source.insert("AGENT_OTEL_LINK_KEY_HEX".into(), invalid.into());
        assert!(config(&source).is_err());
    }
    for field in ["TEAM", "EPOCH"] {
        let mut source = vars();
        source.remove(&format!("AGENT_OTEL_LINK_{field}"));
        assert!(config(&source).is_err());
    }
    let cfg = config(&vars()).unwrap();
    let debug = format!("{cfg:?}");
    for secret in [
        "ORG_SENTINEL",
        "ENDPOINT_SENTINEL",
        "TEAM_SENTINEL",
        KEY,
        &format!("{:?}", hex_decode(KEY).unwrap()),
    ] {
        assert!(!debug.contains(secret));
    }
    // Identity comes from the startup snapshot, never the hook's provisioning claims.
    let before = run(
        Some(&cfg),
        2,
        HookEvent::Stop,
        &json!({"session_id":"abc"}),
        None,
        CaptureMode::Minimal,
    );
    let after = run(
        Some(&cfg),
        2,
        HookEvent::Stop,
        &json!({"session_id":"abc", "org":"different", "endpoint_scope":"different", "team":"different", "epoch":"different", "k_link":hex_encode(&[0u8;32])}),
        None,
        CaptureMode::Minimal,
    );
    assert_eq!(
        attr(&before, "agent.session.link_id"),
        attr(&after, "agent.session.link_id")
    );
}

#[test]
fn minimal_never_exports_opaque_native_session_or_root() {
    use agent_otel_daemon::pseudonym_key::PseudonymKey;
    use prost::Message;
    let cfg = config(&vars()).unwrap();
    let home = std::env::temp_dir().join(format!(
        "aob-a2-privacy-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let key = PseudonymKey::load_or_create(&home).unwrap();
    let native = "550e8400-e29b-41d4-a716-446655440000";
    let body = json!({"session_id":native, "agentRootId":native});
    let frame = encode_context_payload(
        WireHeader::new(2, HookEvent::PostToolUse.to_wire()),
        None,
        &serde_json::to_vec(&body).unwrap(),
    );
    for pseudonym in [None, Some(&key)] {
        for link in [None, Some(&cfg)] {
            for latest in [false, true] {
                let span = production_transform_with_session_link(
                    MsgType::HookPayloadWithContext,
                    &frame,
                    &ContextCache::new(),
                    1,
                    ExecutionMode::Interactive,
                    false,
                    latest,
                    ResolvedSpanMetadata::default(),
                    Instant::now(),
                    1,
                    WorkspacePathMode::Hash,
                    CaptureMode::Minimal,
                    pseudonym,
                    link,
                )
                .unwrap()
                .span;
                let bytes = span.encode_to_vec();
                assert!(!bytes.windows(native.len()).any(|w| w == native.as_bytes()));
                if link.is_some() {
                    assert_eq!(attr(&span, "agent.session.link_id").unwrap().len(), 64);
                }
                if pseudonym.is_some() {
                    assert!(attr(&span, "gen_ai.conversation.id")
                        .unwrap()
                        .starts_with("conv_"));
                    let root_key = if latest {
                        "agent.lineage.root_id"
                    } else {
                        "gen_ai.agent.root_id"
                    };
                    assert!(attr(&span, root_key).unwrap().starts_with("root_"));
                } else {
                    assert_eq!(attr(&span, "gen_ai.conversation.id"), None);
                }
            }
        }
    }
    drop(key);
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn provisioning_snapshot_is_immutable_and_rotated_keys_change_links() {
    let mut source = vars();
    let configured = config(&source).unwrap();
    let body = json!({"session_id":"immutable-session"});
    let before = run(
        Some(&configured),
        2,
        HookEvent::Stop,
        &body,
        None,
        CaptureMode::Minimal,
    );
    source.insert("AGENT_OTEL_LINK_KEY_HEX".into(), "11".repeat(32));
    source.insert("AGENT_OTEL_LINK_EPOCH".into(), "2027-Q1".into());
    let same_snapshot = run(
        Some(&configured),
        2,
        HookEvent::Stop,
        &body,
        None,
        CaptureMode::Minimal,
    );
    assert_eq!(before.trace_id, same_snapshot.trace_id);
    assert_eq!(
        attr(&before, "agent.session.link_id"),
        attr(&same_snapshot, "agent.session.link_id")
    );
    let rotated = config(&source).unwrap();
    let after = run(
        Some(&rotated),
        2,
        HookEvent::Stop,
        &body,
        None,
        CaptureMode::Minimal,
    );
    assert_ne!(before.trace_id, after.trace_id);
    assert_ne!(
        attr(&before, "agent.session.link_id"),
        attr(&after, "agent.session.link_id")
    );
    source.insert("AGENT_OTEL_LINK_KEY_HEX".into(), "f".repeat(63) + "g");
    assert!(config(&source).is_err());
}
