# Semantic convention snapshot

`registry.yaml` lists attribute and metric names this bridge emits on the default path and when `gen_ai_latest_experimental` is set. It is a project snapshot, not a claim that upstream OpenTelemetry semantic conventions have released these names.

`weaver registry check -r semconv` is the upstream command once the Weaver binary is installed. The Rust test `registry_lists_attribute_constants` checks that every dotted public constant in `crates/agent-otel-core/src/semconv.rs` appears in the snapshot.
