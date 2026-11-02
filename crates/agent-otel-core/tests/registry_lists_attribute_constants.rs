/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::path::Path;

#[test]
fn registry_lists_attribute_constants() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let registry = std::fs::read_to_string(manifest.join("../../semconv/registry.yaml"))
        .expect("read semconv/registry.yaml");
    let source = std::fs::read_to_string(manifest.join("src/semconv.rs")).expect("read semconv.rs");
    let mut checked = 0;
    for line in source.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub const ") else {
            continue;
        };
        let Some(value_at) = rest.find("= \"") else {
            continue;
        };
        let literal = &rest[value_at + 3..];
        let Some(end) = literal.find('"') else {
            continue;
        };
        let value = &literal[..end];
        if !value.contains('.') {
            continue;
        }
        assert!(
            registry.contains(value),
            "semconv/registry.yaml is missing {value}"
        );
        checked += 1;
    }
    assert!(checked > 40, "expected attribute names, checked {checked}");
}
