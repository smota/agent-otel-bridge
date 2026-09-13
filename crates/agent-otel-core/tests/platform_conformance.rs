/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::model::HookEvent;
use agent_otel_core::platform::{
    find_platform_by_name, find_platform_by_wire_id, HookResponse, PlatformDescriptor,
    BUILTIN_PLATFORMS,
};
use agent_otel_core::quota::QuotaSnapshot;
use agent_otel_core::validation::{PlatformDynamicValidator, PlatformStaticValidator};

#[test]
fn test_builtin_platforms_pass_static_validation() {
    let result = PlatformStaticValidator::validate_descriptors(BUILTIN_PLATFORMS);
    assert!(
        result.is_ok(),
        "Built-in platforms failed static validation: {:?}",
        result.err()
    );
}

#[test]
fn test_builtin_platforms_pass_dynamic_wire_roundtrip() {
    let events = [
        HookEvent::PreInvocation,
        HookEvent::PostInvocation,
        HookEvent::PreToolUse,
        HookEvent::PostToolUse,
        HookEvent::Stop,
    ];

    for platform in BUILTIN_PLATFORMS {
        for event in events {
            let res = PlatformDynamicValidator::verify_wire_roundtrip(*platform, event);
            assert!(
                res.is_ok(),
                "Wire roundtrip failed for {}: {:?}",
                platform.id(),
                res.err()
            );
        }
    }
}

#[test]
fn test_builtin_platforms_hook_response_is_valid_json() {
    for platform in BUILTIN_PLATFORMS {
        let res = PlatformDynamicValidator::verify_hook_response_json(*platform);
        assert!(
            res.is_ok(),
            "Invalid hook response JSON for {}: {:?}",
            platform.id(),
            res.err()
        );
    }
}

#[test]
fn test_lookup_platform_by_name_and_alias() {
    assert!(find_platform_by_name("antigravity").is_some());
    assert!(find_platform_by_name("gemini").is_some());
    assert!(find_platform_by_name("agy").is_some());
    assert!(find_platform_by_name("claude").is_some());
    assert!(find_platform_by_name("claude-code").is_some());
    assert!(find_platform_by_name("codex").is_some());
    assert!(find_platform_by_name("openai").is_some());
    assert!(find_platform_by_name("grok").is_some());
    assert!(find_platform_by_name("xai").is_some());
    assert!(find_platform_by_name("pi").is_some());
    assert!(find_platform_by_name("pi-cli").is_some());
    assert!(find_platform_by_name("non-existent-agent").is_none());
}

#[test]
fn test_lookup_platform_by_wire_id() {
    for expected_id in 1..=5 {
        let p = find_platform_by_wire_id(expected_id);
        assert!(p.is_some());
        assert_eq!(p.unwrap().wire_client_id(), expected_id);
    }
    assert!(find_platform_by_wire_id(0).is_none());
    assert!(find_platform_by_wire_id(15).is_none());
}

#[test]
fn test_static_validator_detects_malformed_and_colliding_descriptors() {
    struct DuplicateIdPlatform;
    impl PlatformDescriptor for DuplicateIdPlatform {
        fn id(&self) -> &'static str {
            "claude"
        }
        fn display_name(&self) -> &'static str {
            "Duplicate Claude"
        }
        fn aliases(&self) -> &'static [&'static str] {
            &[]
        }
        fn wire_client_id(&self) -> u8 {
            10
        }
    }

    struct BadWireIdPlatform;
    impl PlatformDescriptor for BadWireIdPlatform {
        fn id(&self) -> &'static str {
            "hermes-overflow"
        }
        fn display_name(&self) -> &'static str {
            "Hermes Overflow"
        }
        fn aliases(&self) -> &'static [&'static str] {
            &[]
        }
        fn wire_client_id(&self) -> u8 {
            16
        } // Exceeds 4-bit limit!
    }

    struct WireCollisionPlatform;
    impl PlatformDescriptor for WireCollisionPlatform {
        fn id(&self) -> &'static str {
            "hermes-collision"
        }
        fn display_name(&self) -> &'static str {
            "Hermes Collision"
        }
        fn aliases(&self) -> &'static [&'static str] {
            &[]
        }
        fn wire_client_id(&self) -> u8 {
            1
        } // Collides with Antigravity
    }

    let test_set: &[&dyn PlatformDescriptor] = &[
        &DuplicateIdPlatform,
        &BadWireIdPlatform,
        &WireCollisionPlatform,
        BUILTIN_PLATFORMS[0],
    ];

    let result = PlatformStaticValidator::validate_descriptors(test_set);
    assert!(result.is_err());
    let errs = result.err().unwrap();
    assert!(errs
        .iter()
        .any(|e| e.contains("outside valid 1..=15 range")));
    assert!(errs.iter().any(|e| e.contains("Wire client ID collision")));
}

#[test]
fn test_dynamic_validator_catches_invalid_quota_snapshots() {
    let valid_snap = QuotaSnapshot {
        remaining_fraction: 0.8,
        seconds_to_reset: 1200.0,
        observed_at_unix_nano: 12345,
        bucket: "hermes".to_string(),
        group: "nous".to_string(),
    };
    assert!(PlatformDynamicValidator::verify_quota_snapshot_invariants(&valid_snap).is_ok());

    let negative_snap = QuotaSnapshot {
        remaining_fraction: -0.1,
        ..valid_snap.clone()
    };
    assert!(PlatformDynamicValidator::verify_quota_snapshot_invariants(&negative_snap).is_err());

    let overflow_snap = QuotaSnapshot {
        remaining_fraction: 1.5,
        ..valid_snap.clone()
    };
    assert!(PlatformDynamicValidator::verify_quota_snapshot_invariants(&overflow_snap).is_err());

    let nan_snap = QuotaSnapshot {
        remaining_fraction: f64::NAN,
        ..valid_snap.clone()
    };
    assert!(PlatformDynamicValidator::verify_quota_snapshot_invariants(&nan_snap).is_err());

    let negative_reset = QuotaSnapshot {
        seconds_to_reset: -5.0,
        ..valid_snap
    };
    assert!(PlatformDynamicValidator::verify_quota_snapshot_invariants(&negative_reset).is_err());
}

#[test]
fn test_mock_hermes_platform_conformance() {
    // Demonstration of how a new platform like Hermes conforms
    pub struct HermesDescriptor;
    impl PlatformDescriptor for HermesDescriptor {
        fn id(&self) -> &'static str {
            "hermes"
        }
        fn display_name(&self) -> &'static str {
            "Hermes Agent"
        }
        fn aliases(&self) -> &'static [&'static str] {
            &["nous-hermes", "hermes-cli"]
        }
        fn wire_client_id(&self) -> u8 {
            6
        }
        fn pre_tool_response(&self) -> HookResponse {
            HookResponse::AllowJson
        }
    }

    let mut all_platforms: Vec<&dyn PlatformDescriptor> = BUILTIN_PLATFORMS.to_vec();
    all_platforms.push(&HermesDescriptor);

    // 1. Static validation of full fleet with Hermes
    let static_res = PlatformStaticValidator::validate_descriptors(&all_platforms);
    assert!(static_res.is_ok(), "Hermes failed static validation");

    // 2. Dynamic wire roundtrip
    assert!(PlatformDynamicValidator::verify_wire_roundtrip(
        &HermesDescriptor,
        HookEvent::PreToolUse
    )
    .is_ok());

    // 3. Dynamic JSON hook response validation
    assert!(PlatformDynamicValidator::verify_hook_response_json(&HermesDescriptor).is_ok());
}
