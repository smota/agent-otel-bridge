/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

//! Bounded in-memory pairing of Pre and Post hook spans on the daemon path.
//!
//! The map is owned by the daemon run loop. Expired or displaced Pres are
//! emitted with equal start and end. Nothing is written to disk.

use agent_otel_core::model::HookEvent;
use agent_otel_core::otlp::kv_bool;
use agent_otel_core::semconv::{
    AGENT_DECISION, AGENT_ERROR_CATEGORY, AGENT_FULLY_IDLE, AGENT_HOOK_EVENT, AGENT_SPAN_PAIRED,
    AGENT_STEP_INDEX, AGENT_SUCCESS, AGENT_TERMINATION_REASON, AGENT_TOOL_COMPRESSION_RATIO,
    AGENT_TOOL_TOKENS_SAVED, AGY_TERMINATION_REASON, CAPABILITY_CONSECUTIVE_RETRIES,
    CAPABILITY_IS_WASTE, CAPABILITY_KIND, CAPABILITY_NAME, CAPABILITY_NAMESPACE,
    CAPABILITY_RESPONSE_BYTES, CAPABILITY_RESPONSE_TOKENS, CAPABILITY_SCHEMA_TOKENS, ERROR_TYPE,
    GEN_AI_CONVERSATION_ID, GEN_AI_RESPONSE_FINISH_REASONS, GEN_AI_RESPONSE_MODEL,
    GEN_AI_TOOL_CALL_ID, GEN_AI_TOOL_NAME, GEN_AI_USAGE_CACHE_READ_TOKENS,
    GEN_AI_USAGE_INPUT_TOKENS, GEN_AI_USAGE_OUTPUT_TOKENS,
};
use opentelemetry_proto::tonic::common::v1::{any_value, KeyValue};
use opentelemetry_proto::tonic::trace::v1::Span;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const OPEN_SPAN_CAPACITY: usize = 1024;
pub const OPEN_SPAN_TTL: Duration = Duration::from_secs(60);

const OUTCOME_KEYS: &[&str] = &[
    GEN_AI_USAGE_INPUT_TOKENS,
    GEN_AI_USAGE_OUTPUT_TOKENS,
    GEN_AI_USAGE_CACHE_READ_TOKENS,
    GEN_AI_RESPONSE_FINISH_REASONS,
    GEN_AI_RESPONSE_MODEL,
    AGENT_TERMINATION_REASON,
    AGY_TERMINATION_REASON,
    AGENT_SUCCESS,
    AGENT_DECISION,
    AGENT_ERROR_CATEGORY,
    ERROR_TYPE,
    AGENT_FULLY_IDLE,
];

/// Tool-result fields copied from Post only when Pre did not already set them.
const TOOL_RESULT_IF_ABSENT: &[&str] = &[
    GEN_AI_TOOL_NAME,
    CAPABILITY_RESPONSE_BYTES,
    CAPABILITY_RESPONSE_TOKENS,
    CAPABILITY_CONSECUTIVE_RETRIES,
    CAPABILITY_IS_WASTE,
    CAPABILITY_KIND,
    CAPABILITY_NAMESPACE,
    CAPABILITY_NAME,
    CAPABILITY_SCHEMA_TOKENS,
    AGENT_TOOL_TOKENS_SAVED,
    AGENT_TOOL_COMPRESSION_RATIO,
];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PairKey {
    ToolCall(String),
    ToolFallback {
        conversation: String,
        step: Option<i64>,
        tool: String,
    },
    Invoke(String),
}

struct OpenSpan {
    span: Span,
    opened: Instant,
}

enum Action {
    Immediate,
    Open(PairKey),
    Close(PairKey),
    Stop(PairKey),
}

/// In-memory Pre/Post assembler. Tests construct this directly; the daemon
/// keeps one instance in its run loop.
pub struct SpanAssembler {
    open: HashMap<PairKey, OpenSpan>,
    capacity: usize,
    ttl: Duration,
}

impl Default for SpanAssembler {
    fn default() -> Self {
        Self::new(OPEN_SPAN_CAPACITY, OPEN_SPAN_TTL)
    }
}

impl SpanAssembler {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let capacity = capacity.max(1);
        Self {
            open: HashMap::with_capacity(capacity.min(OPEN_SPAN_CAPACITY)),
            capacity,
            ttl,
        }
    }

    /// Spans to export now, oldest eviction first. A stored Pre returns nothing.
    pub fn observe(&mut self, span: Span, now: Instant) -> Vec<Span> {
        let mut out = Vec::new();
        self.flush_expired(now, &mut out);
        match classify(&span) {
            Action::Immediate => out.push(emit_unpaired(span)),
            Action::Open(key) => {
                if let Some(previous) = self.open.remove(&key) {
                    out.push(emit_unpaired(previous.span));
                }
                self.evict_if_full(&mut out);
                self.open.insert(key, OpenSpan { span, opened: now });
            }
            Action::Close(key) => {
                if let Some(previous) = self.open.remove(&key) {
                    out.push(pair(previous.span, span));
                } else {
                    out.push(emit_unpaired(span));
                }
            }
            Action::Stop(key) => {
                if let Some(previous) = self.open.remove(&key) {
                    out.push(pair(previous.span, span));
                }
            }
        }
        out
    }

    fn flush_expired(&mut self, now: Instant, out: &mut Vec<Span>) {
        let ttl = self.ttl;
        if self
            .open
            .values()
            .all(|open| now.saturating_duration_since(open.opened) < ttl)
        {
            return;
        }
        let expired: Vec<PairKey> = self
            .open
            .iter()
            .filter(|(_, open)| now.saturating_duration_since(open.opened) >= ttl)
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            if let Some(open) = self.open.remove(&key) {
                out.push(emit_unpaired(open.span));
            }
        }
    }

    fn evict_if_full(&mut self, out: &mut Vec<Span>) {
        if self.open.len() < self.capacity {
            return;
        }
        let oldest = self
            .open
            .iter()
            .min_by_key(|(_, open)| open.opened)
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest {
            if let Some(open) = self.open.remove(&key) {
                out.push(emit_unpaired(open.span));
            }
        }
    }
}

fn classify(span: &Span) -> Action {
    let event = attr_str(span, AGENT_HOOK_EVENT)
        .map(HookEvent::from_str_name)
        .unwrap_or(HookEvent::Unknown);
    match event {
        HookEvent::PreToolUse => Action::Open(tool_key(span)),
        HookEvent::PostToolUse => Action::Close(tool_key(span)),
        HookEvent::PreInvocation => Action::Open(invoke_key(span)),
        HookEvent::PostInvocation => Action::Close(invoke_key(span)),
        HookEvent::Stop => Action::Stop(invoke_key(span)),
        HookEvent::Unknown => Action::Immediate,
    }
}

fn tool_key(span: &Span) -> PairKey {
    if let Some(id) = attr_str(span, GEN_AI_TOOL_CALL_ID).filter(|id| !id.is_empty()) {
        return PairKey::ToolCall(id.to_string());
    }
    PairKey::ToolFallback {
        conversation: attr_str(span, GEN_AI_CONVERSATION_ID)
            .unwrap_or("")
            .to_string(),
        step: attr_int(span, AGENT_STEP_INDEX),
        tool: attr_str(span, GEN_AI_TOOL_NAME).unwrap_or("").to_string(),
    }
}

fn invoke_key(span: &Span) -> PairKey {
    PairKey::Invoke(
        attr_str(span, GEN_AI_CONVERSATION_ID)
            .unwrap_or("")
            .to_string(),
    )
}

fn emit_unpaired(mut span: Span) -> Span {
    span.end_time_unix_nano = span.start_time_unix_nano;
    set_paired(&mut span, false);
    span
}

fn pair(mut pre: Span, post: Span) -> Span {
    let post_ts = post.end_time_unix_nano;
    pre.end_time_unix_nano = if post_ts >= pre.start_time_unix_nano {
        post_ts
    } else {
        pre.start_time_unix_nano
    };
    overlay_outcome(&mut pre, &post);
    pre.status = post.status.clone();
    set_paired(&mut pre, true);
    pre
}

fn overlay_outcome(pre: &mut Span, post: &Span) {
    for key in OUTCOME_KEYS {
        if let Some(kv) = find_attr(post, key) {
            upsert(pre, kv.clone());
        }
    }
    for key in TOOL_RESULT_IF_ABSENT {
        if find_attr(pre, key).is_some() {
            continue;
        }
        if let Some(kv) = find_attr(post, key) {
            pre.attributes.push(kv.clone());
        }
    }
}

fn set_paired(span: &mut Span, paired: bool) {
    upsert(span, kv_bool(AGENT_SPAN_PAIRED, paired));
}

fn upsert(span: &mut Span, kv: KeyValue) {
    if let Some(existing) = span.attributes.iter_mut().find(|attr| attr.key == kv.key) {
        *existing = kv;
    } else {
        span.attributes.push(kv);
    }
}

fn find_attr<'a>(span: &'a Span, key: &str) -> Option<&'a KeyValue> {
    span.attributes.iter().find(|attr| attr.key == key)
}

fn attr_str<'a>(span: &'a Span, key: &str) -> Option<&'a str> {
    match find_attr(span, key)?.value.as_ref()?.value.as_ref()? {
        any_value::Value::StringValue(value) => Some(value.as_str()),
        _ => None,
    }
}

fn attr_int(span: &Span, key: &str) -> Option<i64> {
    match find_attr(span, key)?.value.as_ref()?.value.as_ref()? {
        any_value::Value::IntValue(value) => Some(*value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_otel_core::model::{AgentHookInput, ToolCallInfo};
    use agent_otel_core::otlp::{build_span_from_resolved, ResolvedSpanMetadata};
    use agent_otel_core::trace_id::resolve_trace_context;
    use opentelemetry_proto::tonic::trace::v1::status::StatusCode;
    use std::hint::black_box;

    #[allow(clippy::too_many_arguments)]
    fn hook_span(
        event: HookEvent,
        conversation: &str,
        step: u64,
        tool: Option<&str>,
        call_id: Option<&str>,
        input_tokens: Option<i64>,
        error: Option<&str>,
        nanos: u64,
        salt: u32,
    ) -> Span {
        let input = AgentHookInput {
            conversation_id: Some(conversation.to_string()),
            step_idx: Some(step),
            tool_name: tool.map(str::to_string),
            tool_call: call_id.map(|id| ToolCallInfo {
                id: Some(id.to_string()),
                name: tool.map(str::to_string),
                arguments: None,
            }),
            input_tokens,
            error: error.map(str::to_string),
            agent_name: Some("codex".to_string()),
            ..Default::default()
        };
        build_span_from_resolved(
            event,
            &input,
            nanos,
            nanos,
            salt,
            resolve_trace_context(None, None, Some(conversation)),
            ResolvedSpanMetadata::default(),
            false,
            false,
        )
    }

    fn bool_attr(span: &Span, key: &str) -> Option<bool> {
        match find_attr(span, key)?.value.as_ref()?.value.as_ref()? {
            any_value::Value::BoolValue(value) => Some(*value),
            _ => None,
        }
    }

    #[test]
    fn pre_post_tool_call_id_exports_one_paired_span() {
        let mut assembler = SpanAssembler::default();
        let now = Instant::now();
        let pre = hook_span(
            HookEvent::PreToolUse,
            "conv",
            1,
            Some("read_file"),
            Some("call-1"),
            None,
            None,
            1_000,
            1,
        );
        let pre_id = pre.span_id.clone();
        let pre_trace = pre.trace_id.clone();
        assert!(assembler.observe(pre, now).is_empty());

        let post = hook_span(
            HookEvent::PostToolUse,
            "conv",
            9,
            None,
            Some("call-1"),
            Some(42),
            Some("boom"),
            5_000,
            2,
        );
        let out = assembler.observe(post, now + Duration::from_millis(5));
        assert_eq!(out.len(), 1);
        let span = &out[0];
        assert_eq!(span.span_id, pre_id);
        assert_eq!(span.trace_id, pre_trace);
        assert_eq!(span.name, "execute_tool read_file");
        assert!(span.end_time_unix_nano > span.start_time_unix_nano);
        assert_eq!(span.start_time_unix_nano, 1_000);
        assert_eq!(span.end_time_unix_nano, 5_000);
        assert_eq!(bool_attr(span, AGENT_SPAN_PAIRED), Some(true));
        assert_eq!(attr_str(span, GEN_AI_TOOL_NAME), Some("read_file"));
        assert_eq!(attr_int(span, GEN_AI_USAGE_INPUT_TOKENS), Some(42));
        assert_eq!(
            span.status.as_ref().map(|status| status.code),
            Some(StatusCode::Error as i32)
        );
    }

    #[test]
    fn post_without_pre_is_unpaired_zero_duration() {
        let mut assembler = SpanAssembler::default();
        let post = hook_span(
            HookEvent::PostToolUse,
            "conv",
            1,
            Some("read_file"),
            Some("missing"),
            None,
            None,
            8_000,
            3,
        );
        let post_id = post.span_id.clone();
        let out = assembler.observe(post, Instant::now());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].span_id, post_id);
        assert_eq!(out[0].start_time_unix_nano, out[0].end_time_unix_nano);
        assert_eq!(bool_attr(&out[0], AGENT_SPAN_PAIRED), Some(false));
    }

    #[test]
    fn second_pre_exports_first_unpaired_and_post_pairs_second() {
        let mut assembler = SpanAssembler::default();
        let now = Instant::now();
        let first = hook_span(
            HookEvent::PreToolUse,
            "conv",
            1,
            Some("read_file"),
            Some("call-2"),
            None,
            None,
            1_000,
            1,
        );
        let first_id = first.span_id.clone();
        assert!(assembler.observe(first, now).is_empty());

        let second = hook_span(
            HookEvent::PreToolUse,
            "conv",
            1,
            Some("read_file"),
            Some("call-2"),
            None,
            None,
            2_000,
            2,
        );
        let second_id = second.span_id.clone();
        let displaced = assembler.observe(second, now + Duration::from_millis(1));
        assert_eq!(displaced.len(), 1);
        assert_eq!(displaced[0].span_id, first_id);
        assert_eq!(
            displaced[0].start_time_unix_nano,
            displaced[0].end_time_unix_nano
        );
        assert_eq!(bool_attr(&displaced[0], AGENT_SPAN_PAIRED), Some(false));

        let post = hook_span(
            HookEvent::PostToolUse,
            "conv",
            1,
            Some("read_file"),
            Some("call-2"),
            None,
            None,
            4_000,
            3,
        );
        let paired = assembler.observe(post, now + Duration::from_millis(2));
        assert_eq!(paired.len(), 1);
        assert_eq!(paired[0].span_id, second_id);
        assert!(paired[0].end_time_unix_nano > paired[0].start_time_unix_nano);
        assert_eq!(bool_attr(&paired[0], AGENT_SPAN_PAIRED), Some(true));
    }

    #[test]
    fn pre_invocation_stop_exports_one_invoke_agent() {
        let mut assembler = SpanAssembler::default();
        let now = Instant::now();
        let alone = hook_span(
            HookEvent::Stop,
            "conv-stop",
            0,
            None,
            None,
            None,
            None,
            500,
            1,
        );
        assert!(assembler.observe(alone, now).is_empty());

        let pre = hook_span(
            HookEvent::PreInvocation,
            "conv-stop",
            0,
            None,
            None,
            None,
            None,
            1_000,
            2,
        );
        let pre_id = pre.span_id.clone();
        assert!(assembler.observe(pre, now).is_empty());

        let stop = hook_span(
            HookEvent::Stop,
            "conv-stop",
            4,
            None,
            None,
            Some(7),
            None,
            9_000,
            3,
        );
        let out = assembler.observe(stop, now + Duration::from_millis(9));
        assert_eq!(out.len(), 1);
        assert!(out[0].name.starts_with("invoke_agent"));
        assert_ne!(out[0].name, "agent.stop");
        assert_eq!(out[0].span_id, pre_id);
        assert!(out[0].end_time_unix_nano > out[0].start_time_unix_nano);
        assert_eq!(bool_attr(&out[0], AGENT_SPAN_PAIRED), Some(true));
        assert!(out.iter().all(|span| span.name != "agent.stop"));
    }

    #[test]
    fn capacity_evicts_one_unpaired_span_on_the_1025th_pre() {
        let mut assembler = SpanAssembler::default();
        let now = Instant::now();
        for index in 0..OPEN_SPAN_CAPACITY {
            let pre = hook_span(
                HookEvent::PreToolUse,
                "conv",
                index as u64,
                Some("read_file"),
                Some(&format!("call-{index}")),
                None,
                None,
                1_000 + index as u64,
                (index as u32).saturating_add(1),
            );
            assert!(
                assembler.observe(pre, now).is_empty(),
                "open pre {index} should stay buffered"
            );
        }
        let extra = hook_span(
            HookEvent::PreToolUse,
            "conv",
            9_999,
            Some("read_file"),
            Some("call-extra"),
            None,
            None,
            50_000,
            9,
        );
        let evicted = assembler.observe(extra, now);
        assert_eq!(evicted.len(), 1);
        assert_eq!(
            evicted[0].start_time_unix_nano,
            evicted[0].end_time_unix_nano
        );
        assert_eq!(bool_attr(&evicted[0], AGENT_SPAN_PAIRED), Some(false));
    }

    #[test]
    fn open_pre_older_than_ttl_exports_unpaired_on_the_next_event() {
        let mut assembler = SpanAssembler::default();
        let opened = Instant::now();
        let pre = hook_span(
            HookEvent::PreToolUse,
            "conv",
            1,
            Some("read_file"),
            Some("stale"),
            None,
            None,
            1_000,
            1,
        );
        let pre_id = pre.span_id.clone();
        assert!(assembler.observe(pre, opened).is_empty());

        let unknown = hook_span(
            HookEvent::Unknown,
            "conv",
            1,
            None,
            None,
            None,
            None,
            2_000,
            2,
        );
        let out = assembler.observe(unknown, opened + OPEN_SPAN_TTL + Duration::from_nanos(1));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].span_id, pre_id);
        assert_eq!(out[0].start_time_unix_nano, out[0].end_time_unix_nano);
        assert_eq!(bool_attr(&out[0], AGENT_SPAN_PAIRED), Some(false));
        assert_eq!(out[1].name, "agent.unknown");
        assert_eq!(bool_attr(&out[1], AGENT_SPAN_PAIRED), Some(false));
    }

    #[test]
    fn unknown_exports_one_unpaired_span() {
        let mut assembler = SpanAssembler::default();
        let unknown = hook_span(
            HookEvent::Unknown,
            "conv",
            0,
            None,
            None,
            None,
            None,
            3_000,
            1,
        );
        let out = assembler.observe(unknown, Instant::now());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "agent.unknown");
        assert_eq!(out[0].start_time_unix_nano, out[0].end_time_unix_nano);
        assert_eq!(bool_attr(&out[0], AGENT_SPAN_PAIRED), Some(false));
    }

    #[test]
    fn pre_post_without_call_id_pairs_on_conversation_step_and_tool() {
        let mut assembler = SpanAssembler::default();
        let now = Instant::now();
        let pre = hook_span(
            HookEvent::PreToolUse,
            "conv-fb",
            4,
            Some("grep"),
            None,
            None,
            None,
            100,
            1,
        );
        let pre_id = pre.span_id.clone();
        assert!(assembler.observe(pre, now).is_empty());
        let post = hook_span(
            HookEvent::PostToolUse,
            "conv-fb",
            4,
            Some("grep"),
            None,
            None,
            None,
            400,
            2,
        );
        let out = assembler.observe(post, now + Duration::from_millis(1));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].span_id, pre_id);
        assert_eq!(bool_attr(&out[0], AGENT_SPAN_PAIRED), Some(true));
        assert!(out[0].end_time_unix_nano > out[0].start_time_unix_nano);
    }

    #[test]
    #[ignore]
    fn pairing_p99_under_1ms() {
        let mut assembler = SpanAssembler::default();
        let base = Instant::now();
        let pairs: Vec<(Span, Span)> = (0..10_000)
            .map(|index| {
                let id = format!("bench-{index}");
                let pre = hook_span(
                    HookEvent::PreToolUse,
                    "bench",
                    index,
                    Some("read_file"),
                    Some(&id),
                    None,
                    None,
                    1_000,
                    1,
                );
                let post = hook_span(
                    HookEvent::PostToolUse,
                    "bench",
                    index,
                    Some("read_file"),
                    Some(&id),
                    Some(3),
                    None,
                    2_000,
                    2,
                );
                (pre, post)
            })
            .collect();

        for index in 0..64u64 {
            let id = format!("warm-{index}");
            let _ = assembler.observe(
                hook_span(
                    HookEvent::PreToolUse,
                    "warm",
                    index,
                    Some("read_file"),
                    Some(&id),
                    None,
                    None,
                    1,
                    1,
                ),
                base,
            );
            let _ = assembler.observe(
                hook_span(
                    HookEvent::PostToolUse,
                    "warm",
                    index,
                    Some("read_file"),
                    Some(&id),
                    Some(1),
                    None,
                    2,
                    2,
                ),
                base,
            );
        }

        let mut samples = Vec::with_capacity(pairs.len());
        for (index, (pre, post)) in pairs.into_iter().enumerate() {
            let observed = base + Duration::from_nanos(index as u64);
            assert!(assembler.observe(pre, observed).is_empty());
            let started = Instant::now();
            let paired = assembler.observe(post, observed + Duration::from_nanos(1));
            let elapsed = started.elapsed();
            assert_eq!(paired.len(), 1);
            black_box(&paired);
            samples.push(elapsed);
        }
        samples.sort_unstable();
        let p99 = samples[(samples.len() * 99) / 100];
        eprintln!("pairing_p99_ns={}", p99.as_nanos());
        assert!(
            p99 < Duration::from_millis(1),
            "pairing p99 {p99:?} exceeds 1ms"
        );
    }
}
