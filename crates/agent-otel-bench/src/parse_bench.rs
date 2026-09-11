/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::stats::Stats;
use agent_otel_core::model::{AntigravityHookInput, HookEvent};
use agent_otel_core::otlp::build_span_from_hook;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ParseBenchResult {
    pub stats: Stats,
    pub spans_per_sec: f64,
}

impl ParseBenchResult {
    pub fn summary(&self) -> ParseBenchSummary {
        ParseBenchSummary {
            stats: self.stats.summary(),
            spans_per_sec: (self.spans_per_sec * 10.0).round() / 10.0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParseBenchSummary {
    pub stats: crate::stats::StatsSummary,
    pub spans_per_sec: f64,
}

pub fn run(iterations: usize) -> ParseBenchResult {
    let payload = br#"{
        "conversationId": "54a597ba-e7ee-46b0-846b-8058238f58f9",
        "stepIdx": 142,
        "toolCall": {
            "id": "call_12345",
            "name": "run_command",
            "arguments": {
                "CommandLine": "cargo build --release"
            }
        },
        "modelName": "gemini-2.5-pro",
        "executionNum": 4,
        "fullyIdle": false
    }"#;

    let mut samples = Vec::with_capacity(iterations);
    let total_start = Instant::now();

    for i in 0..iterations {
        let start = Instant::now();

        let input = AntigravityHookInput::parse_slice(payload).unwrap();
        let span = build_span_from_hook(
            HookEvent::PostToolUse,
            &input,
            1_000_000,
            2_000_000,
            i as u32,
        );
        std::hint::black_box(&span);

        let elapsed = start.elapsed();
        samples.push(elapsed.as_micros() as u64);
    }

    let total_duration = total_start.elapsed().as_secs_f64();
    let spans_per_sec = iterations as f64 / total_duration;

    ParseBenchResult {
        stats: Stats::new(samples),
        spans_per_sec,
    }
}
