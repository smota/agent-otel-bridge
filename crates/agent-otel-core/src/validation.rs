/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::collections::{HashMap, HashSet};

use crate::model::{HookEvent, WireHeader};
use crate::platform::PlatformDescriptor;
use crate::quota::QuotaSnapshot;

pub struct PlatformStaticValidator;

impl PlatformStaticValidator {
    /// Statically verifies a collection of platform descriptors against core invariants:
    /// - Non-empty, sanitized primary IDs (lowercase alphanumeric, hyphens, underscores).
    /// - Non-empty display names.
    /// - Strict wire client ID boundaries (1..=65535, 0 is reserved for Unspecified).
    /// - Zero collision of primary IDs.
    /// - Zero collision of wire client IDs.
    /// - Zero collision across platform aliases.
    pub fn validate_descriptors(platforms: &[&dyn PlatformDescriptor]) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut seen_wire_ids = HashMap::new();
        let mut seen_aliases = HashMap::new();

        for platform in platforms {
            let id = platform.id();
            let display_name = platform.display_name();
            let wire_id = platform.wire_client_id();

            // 1. Validate ID format
            if id.is_empty() {
                errors.push("Platform ID cannot be empty".to_string());
            } else if !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
            {
                errors.push(format!(
                    "Platform ID '{}' contains invalid characters. Must be lowercase ASCII alphanumeric, hyphens, or underscores.",
                    id
                ));
            }

            if !seen_ids.insert(id) {
                errors.push(format!("Duplicate platform ID detected: '{}'", id));
            }

            // 2. Validate display name
            if display_name.trim().is_empty() {
                errors.push(format!("Platform '{}' has an empty display name", id));
            }

            // 3. Validate wire client ID bounds (1..=65535, 0 reserved)
            if wire_id == 0 {
                errors.push(format!(
                    "Platform '{}' specifies wire_client_id 0, which is reserved for Unspecified",
                    id
                ));
            } else if let Some(existing_owner) = seen_wire_ids.insert(wire_id, id) {
                errors.push(format!(
                    "Wire client ID collision on {}: both '{}' and '{}' use the same wire ID",
                    wire_id, existing_owner, id
                ));
            }

            // 4. Validate aliases
            for &alias in platform.aliases() {
                let trimmed = alias.trim().to_ascii_lowercase();
                if trimmed.is_empty() {
                    errors.push(format!("Platform '{}' contains an empty alias", id));
                    continue;
                }
                if let Some(existing_owner) = seen_aliases.insert(trimmed.clone(), id) {
                    errors.push(format!(
                        "Alias collision on '{}': claimed by both '{}' and '{}'",
                        trimmed, existing_owner, id
                    ));
                }
            }
        }

        // Check if any alias collides with another platform's primary ID
        for (alias, owner) in &seen_aliases {
            if seen_ids.contains(alias.as_str()) && *owner != alias.as_str() {
                errors.push(format!(
                    "Alias collision: '{}' claimed as alias by '{}' matches primary ID of another platform",
                    alias, owner
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

pub struct PlatformDynamicValidator;

impl PlatformDynamicValidator {
    /// Validates binary 3-byte wire header packing and unpacking roundtrip.
    pub fn verify_wire_roundtrip(
        descriptor: &dyn PlatformDescriptor,
        event: HookEvent,
    ) -> Result<(), String> {
        let wire_id = descriptor.wire_client_id();
        let event_id = event.to_wire();

        let header = WireHeader::new(wire_id, event_id);
        let encoded = header.encode();

        let decoded = WireHeader::decode(&encoded).ok_or_else(|| {
            format!(
                "Wire decode failed for {}: could not decode 3-byte header",
                descriptor.id()
            )
        })?;

        if decoded.client_id != wire_id {
            return Err(format!(
                "Wire roundtrip failed for {}: expected client ID {}, got {}",
                descriptor.id(),
                wire_id,
                decoded.client_id
            ));
        }

        let decoded_event = HookEvent::from_wire(decoded.event_id);
        if decoded_event != event {
            return Err(format!(
                "Wire roundtrip failed for {}: expected event {:?}, got {:?}",
                descriptor.id(),
                event,
                decoded_event
            ));
        }

        Ok(())
    }

    /// Validates that PreToolUse hook response is syntactically valid JSON.
    pub fn verify_hook_response_json(descriptor: &dyn PlatformDescriptor) -> Result<(), String> {
        let resp = descriptor.pre_tool_response();
        let json_str = resp.as_str();

        let parsed: Result<serde_json::Value, _> = serde_json::from_str(json_str);
        match parsed {
            Ok(v) if v.is_object() => Ok(()),
            Ok(_) => Err(format!(
                "Hook response for {} is not a JSON object: '{}'",
                descriptor.id(),
                json_str
            )),
            Err(e) => Err(format!(
                "Hook response for {} failed JSON parsing: {}: '{}'",
                descriptor.id(),
                e,
                json_str
            )),
        }
    }

    /// Verifies mathematical and structural invariants of a QuotaSnapshot.
    pub fn verify_quota_snapshot_invariants(snapshot: &QuotaSnapshot) -> Result<(), String> {
        if !snapshot.remaining_fraction.is_finite() {
            return Err(format!(
                "QuotaSnapshot for bucket '{}' has non-finite remaining_fraction: {}",
                snapshot.bucket, snapshot.remaining_fraction
            ));
        }

        if !(0.0..=1.0).contains(&snapshot.remaining_fraction) {
            return Err(format!(
                "QuotaSnapshot for bucket '{}' has out-of-range remaining_fraction (must be 0.0..=1.0): {}",
                snapshot.bucket, snapshot.remaining_fraction
            ));
        }

        if snapshot.seconds_to_reset < 0.0 || !snapshot.seconds_to_reset.is_finite() {
            return Err(format!(
                "QuotaSnapshot for bucket '{}' has invalid seconds_to_reset: {}",
                snapshot.bucket, snapshot.seconds_to_reset
            ));
        }

        if snapshot.bucket.trim().is_empty() {
            return Err("QuotaSnapshot has an empty bucket identifier".to_string());
        }

        if snapshot.group.trim().is_empty() {
            return Err("QuotaSnapshot has an empty group/provider identifier".to_string());
        }

        Ok(())
    }
}
