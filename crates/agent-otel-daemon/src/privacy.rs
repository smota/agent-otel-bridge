/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

//! Ingress privacy projection and capture mode enforcement (AOB#17 / F5).
//!
//! Enforces an explicit [`CaptureMode`] on the daemon side before OTLP export
//! across spans, span events, resources, and quota metrics.

use crate::config::WorkspacePathMode;
use crate::pseudonym_key::PseudonymKey;
use agent_otel_core::model::AgentHookInput;
use agent_otel_core::otlp::{kv_string, ResolvedSpanMetadata};
use agent_otel_core::semconv::{AGENT_CAPTURE_MODE, AGENT_ERROR_CATEGORY, ERROR_TYPE};
use opentelemetry_proto::tonic::common::v1::any_value;
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{status::StatusCode, Span};

/// Explicit daemon-side privacy capture mode (`AGENT_OTEL_CAPTURE_MODE`).
///
/// Unset and unrecognized values fail closed to [`CaptureMode::Minimal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureMode {
    /// Content-free default with per-install keyed pseudonyms (not anonymity).
    /// Drops content and replaces raw identifiers with pseudonyms or closed categories.
    #[default]
    Minimal,
    /// Opt-in operational metadata mode (`metadata_only` / `metadata-only`).
    /// Preserves operational identifiers while dropping content and scrubbing credentials.
    MetadataOnly,
    /// Opt-in diagnostic mode (`full`).
    /// Preserves metadata and bounded error status text while scrubbing credentials.
    Full,
}

impl CaptureMode {
    /// Canonical attribute value stamped on `agent.capture.mode`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::MetadataOnly => "metadata_only",
            Self::Full => "full",
        }
    }

    /// Parses `AGENT_OTEL_CAPTURE_MODE`, defaulting to [`CaptureMode::Minimal`].
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim) {
            Some(v) if v.eq_ignore_ascii_case("minimal") => Self::Minimal,
            Some(v)
                if v.eq_ignore_ascii_case("metadata_only")
                    || v.eq_ignore_ascii_case("metadata-only") =>
            {
                Self::MetadataOnly
            }
            Some(v) if v.eq_ignore_ascii_case("full") => Self::Full,
            _ => Self::Minimal,
        }
    }
}

const CLOSED_ERROR_CATEGORIES: &[&str] = &[
    "provider_quota_exhausted",
    "user_interrupted",
    "schema_validation_error",
    "tool_verification_failed",
];

const SAFE_TOOL_BINARIES: &[&str] = &[
    "awk",
    "bash",
    "bat",
    "black",
    "bun",
    "cargo",
    "cat",
    "chmod",
    "chown",
    "clippy-driver",
    "cmake",
    "cmd",
    "cp",
    "curl",
    "cut",
    "delta",
    "deno",
    "diff",
    "dir",
    "docker",
    "dotnet",
    "echo",
    "env",
    "false",
    "fd",
    "find",
    "fish",
    "flake8",
    "gh",
    "git",
    "go",
    "gradle",
    "grep",
    "gunzip",
    "gzip",
    "head",
    "helm",
    "jq",
    "kill",
    "kubectl",
    "less",
    "ls",
    "make",
    "mkdir",
    "mv",
    "mvn",
    "mypy",
    "ninja",
    "node",
    "npm",
    "npx",
    "pip",
    "pnpm",
    "podman",
    "poetry",
    "powershell",
    "ps",
    "pwd",
    "pwsh",
    "pytest",
    "python",
    "python3",
    "rg",
    "rm",
    "rmdir",
    "rsync",
    "rtk",
    "ruff",
    "rustc",
    "rustfmt",
    "rustup",
    "scp",
    "sed",
    "sh",
    "sleep",
    "sort",
    "ssh",
    "stdbuf",
    "tail",
    "tar",
    "tee",
    "terraform",
    "test",
    "time",
    "timeout",
    "touch",
    "tr",
    "true",
    "uniq",
    "unzip",
    "uv",
    "wc",
    "wget",
    "where",
    "which",
    "xargs",
    "yarn",
    "yq",
    "zip",
    "zsh",
];

const SAFE_BUILTIN_TOOLS: &[&str] = &[
    "Agent",
    "Bash",
    "Edit",
    "ExitPlanMode",
    "Glob",
    "Grep",
    "LS",
    "MultiEdit",
    "NotebookEdit",
    "NotebookRead",
    "Read",
    "Skill",
    "SlashCommand",
    "Task",
    "TodoRead",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
    "Write",
    "apply_patch",
    "ask_question",
    "browser_subagent",
    "command_status",
    "define_subagent",
    "edit_file",
    "exec_command",
    "find_by_name",
    "generate_image",
    "grep",
    "grep_search",
    "invoke_subagent",
    "list_dir",
    "manage_subagents",
    "manage_task",
    "multi_replace_file_content",
    "notify_user",
    "read_file",
    "read_url_content",
    "replace_file_content",
    "request_user_input",
    "run_command",
    "schedule",
    "search_web",
    "send_command_input",
    "send_message",
    "shell",
    "swarm",
    "task_boundary",
    "test_tool",
    "unknown_tool",
    "update_plan",
    "view_code_item",
    "view_content_chunk",
    "view_file",
    "view_file_outline",
    "view_image",
    "write_file",
    "write_stdin",
];

const SAFE_AGENT_NAMES: &[&str] = &[
    "aider",
    "antigravity",
    "builder",
    "claude",
    "claude-code",
    "codex",
    "coordinator",
    "copilot",
    "cursor",
    "default",
    "explorer",
    "gemini",
    "grok",
    "pi",
    "planner",
    "research",
    "researcher",
    "reviewer",
    "self",
    "subagent",
    "unknown",
    "unknown_agent",
    "verifier",
    "windsurf",
    "worker",
];

const SAFE_TERMINAL_TYPES: &[&str] = &[
    "alacritty",
    "apple_terminal",
    "conhost",
    "cursor",
    "ghostty",
    "iterm2",
    "jetbrains",
    "kitty",
    "other",
    "screen",
    "test-terminal",
    "tmux",
    "vscode",
    "wezterm",
    "windows-terminal",
    "windsurf",
    "xterm",
    "xterm-256color",
];

const SAFE_PROJECT_TYPES: &[&str] = &[
    "c", "cpp", "dotnet", "elixir", "go", "haskell", "java", "kotlin", "node", "other", "php",
    "polyglot", "python", "ruby", "rust", "scala", "swift", "unknown", "zig",
];

fn pseudonym(key: Option<&PseudonymKey>, prefix: &str, raw: &str) -> String {
    // Empty projections are removed below. Never fall back to an unkeyed hash.
    key.map(|key| key.pseudonym(prefix, raw))
        .unwrap_or_default()
}

/// Maps arbitrary Git branch names to a closed category set in [`CaptureMode::Minimal`].
pub fn categorize_branch(branch: &str) -> &'static str {
    let trimmed = branch.trim().trim_start_matches("refs/heads/");
    let lower = trimmed.to_ascii_lowercase();
    if matches!(lower.as_str(), "main" | "master" | "trunk") {
        "main"
    } else if matches!(lower.as_str(), "develop" | "dev") {
        "develop"
    } else if matches!(lower.as_str(), "feat" | "feature")
        || lower.starts_with("feat/")
        || lower.starts_with("feature/")
        || lower.starts_with("feat-")
        || lower.starts_with("feature-")
    {
        "feature"
    } else if matches!(lower.as_str(), "fix" | "bugfix" | "hotfix")
        || lower.starts_with("fix/")
        || lower.starts_with("bugfix/")
        || lower.starts_with("hotfix/")
        || lower.starts_with("fix-")
        || lower.starts_with("bugfix-")
        || lower.starts_with("hotfix-")
    {
        "fix"
    } else if lower == "release" || lower.starts_with("release/") || lower.starts_with("release-") {
        "release"
    } else if lower == "docs" || lower.starts_with("docs/") || lower.starts_with("docs-") {
        "docs"
    } else if lower == "chore" || lower.starts_with("chore/") || lower.starts_with("chore-") {
        "chore"
    } else if matches!(lower.as_str(), "detached" | "head") {
        "detached"
    } else {
        "other"
    }
}

/// Maps termination reasons to a closed category set in [`CaptureMode::Minimal`].
pub fn categorize_termination_reason(reason: &str) -> &'static str {
    let trimmed = reason.trim();
    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "completed" => "completed",
        "stop" => "stop",
        "end_turn" => "end_turn",
        "max_tokens" => "max_tokens",
        "length" => "length",
        "tool_calls" => "tool_calls",
        "tool_use" => "tool_use",
        "error" => "error",
        "cancelled" => "cancelled",
        "canceled" => "canceled",
        "interrupted" => "interrupted",
        "timeout" => "timeout",
        "quota_exhausted" => "quota_exhausted",
        "rate_limited" => "rate_limited",
        "content_filter" => "content_filter",
        _ if lower.contains("error") || lower.contains("fail") || lower.contains("fatal") => {
            "error"
        }
        _ if lower.contains("cancel") || lower.contains("interrupt") => "interrupted",
        _ if lower.contains("quota") || lower.contains("rate") => "quota_exhausted",
        _ if lower.contains("timeout") || lower.contains("timed out") => "timeout",
        _ => "other",
    }
}

fn categorize_decision(decision: &str) -> &'static str {
    match decision.trim().to_ascii_lowercase().as_str() {
        "allow" => "allow",
        "deny" => "deny",
        "ask" => "ask",
        "block" => "block",
        "approve" => "approve",
        "reject" => "reject",
        _ => "other",
    }
}

/// Classifies an error string into the closed 4-category set.
pub fn classify_closed_error_category(
    existing_category: Option<&str>,
    raw_error: Option<&str>,
) -> &'static str {
    if let Some(cat) = existing_category {
        for &allowed in CLOSED_ERROR_CATEGORIES {
            if cat == allowed {
                return allowed;
            }
        }
    }
    if let Some(err) = raw_error {
        let lower = err.to_ascii_lowercase();
        if lower.contains("429")
            || lower.contains("quota")
            || lower.contains("rate limit")
            || lower.contains("resource_exhausted")
            || lower.contains("too many requests")
        {
            "provider_quota_exhausted"
        } else if lower.contains("interrupt")
            || lower.contains("cancel")
            || lower.contains("aborted by user")
            || lower.contains("sigint")
        {
            "user_interrupted"
        } else if lower.contains("schema")
            || lower.contains("invalid argument")
            || lower.contains("missing required")
            || lower.contains("type mismatch")
            || lower.contains("validation error")
        {
            "schema_validation_error"
        } else {
            "tool_verification_failed"
        }
    } else {
        "tool_verification_failed"
    }
}

/// Scrubs credentials, bearer tokens, URL userinfo, and secret query parameters from text.
pub fn scrub_credentials(raw: &str) -> String {
    let authorization = redact_auth_scheme(raw);
    let named = scrub_named_secrets(&authorization);
    let raw = named.as_str();
    let mut out = String::with_capacity(raw.len());
    for (idx, token) in raw.split_whitespace().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        out.push_str(&scrub_single_token(token));
    }
    // Redact Bearer / Basic authorization values if present across whitespace
    redact_auth_scheme(&out)
}

// Best-effort text scrubbing for common assignments, JSON/YAML and CLI flags.
// This cannot recognize arbitrary unlabeled secrets; opt-in modes still expose metadata.
fn scrub_named_secrets(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        if !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'-') {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'-'))
        {
            i += 1;
        }
        let name = raw[start..i].trim_start_matches('-').to_ascii_lowercase();
        if !matches!(
            name.as_str(),
            "password"
                | "passwd"
                | "pwd"
                | "token"
                | "access_token"
                | "api_key"
                | "api-key"
                | "apikey"
                | "x-api-key"
                | "secret"
                | "auth"
                | "authorization"
                | "client_secret"
                | "private_key"
                | "aws_secret_access_key"
                | "aws_session_token"
                | "aws_access_key_id"
                | "secretaccesskey"
                | "sessiontoken"
        ) {
            continue;
        }
        let mut j = i;
        if j < bytes.len() && matches!(bytes[j], b'"' | b'\'') {
            j += 1;
        }
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < bytes.len() && matches!(bytes[j], b':' | b'=') {
            j += 1;
        } else if !raw[start..i].starts_with("--") || j == i {
            continue;
        }
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let quote = bytes.get(j).copied().filter(|b| matches!(b, b'"' | b'\''));
        if quote.is_some() {
            j += 1;
        }
        let value_start = j;
        while j < bytes.len() {
            if quote == Some(bytes[j])
                || (quote.is_none()
                    && (bytes[j].is_ascii_whitespace()
                        || matches!(bytes[j], b',' | b';' | b'}' | b'&')))
            {
                break;
            }
            j += 1;
        }
        if j > value_start {
            out.push_str(&raw[copied..value_start]);
            out.push_str("[REDACTED]");
            copied = j;
            i = j;
        }
    }
    out.push_str(&raw[copied..]);
    out
}

fn redact_auth_scheme(input: &str) -> String {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.len() < 2 {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < tokens.len() {
        if i > 0 {
            out.push(' ');
        }
        let t = tokens[i];
        let clean = t.trim_end_matches([':', '=', ',']);
        if (clean.eq_ignore_ascii_case("bearer") || clean.eq_ignore_ascii_case("basic"))
            && i + 1 < tokens.len()
        {
            out.push_str(t);
            out.push_str(" [REDACTED]");
            i += 2;
            continue;
        }
        out.push_str(t);
        i += 1;
    }
    out
}

fn scrub_single_token(token: &str) -> String {
    // Handle key=value pairs where value may be a URL or secret
    if let Some((k, v)) = token.split_once('=') {
        if !k.contains("://") {
            let k_lower = k
                .trim_matches(['"', '\'', ',', ';', '('])
                .to_ascii_lowercase();
            if matches!(
                k_lower.as_str(),
                "token"
                    | "access_token"
                    | "api_key"
                    | "apikey"
                    | "secret"
                    | "password"
                    | "passwd"
                    | "pwd"
                    | "auth"
                    | "authorization"
                    | "client_secret"
                    | "private_key"
            ) {
                return format!("{k}=[REDACTED]");
            }
            return format!("{k}={}", scrub_single_token(v));
        }
    }

    let trimmed = token.trim_matches(['"', '\'', ',', ';', '(', ')', '[', ']', '<', '>']);
    if is_secret_token_literal(trimmed) {
        return "[REDACTED]".to_string();
    }

    // URL userinfo scrubbing: proto://user:pass@host/path
    if let Some(proto_idx) = token.find("://") {
        let after_proto = &token[proto_idx + 3..];
        let authority_end = after_proto
            .find(['/', '?', '#'])
            .unwrap_or(after_proto.len());
        let authority = &after_proto[..authority_end];
        let mut sanitized = if let Some(at_rel) = authority.rfind('@') {
            format!("{}{}", &token[..proto_idx + 3], &after_proto[at_rel + 1..])
        } else {
            token.to_string()
        };
        if let Some(q_idx) = sanitized.find('?') {
            let base = &sanitized[..q_idx];
            let query = &sanitized[q_idx + 1..];
            sanitized = format!("{base}?{}", scrub_query_string(query));
        }
        return sanitized;
    }

    // SCP-style git URL with user:secret@host:path
    if let Some(at_idx) = token.find('@') {
        let before_at = &token[..at_idx];
        let after_at = &token[at_idx + 1..];
        if before_at.contains(':') && after_at.contains(':') {
            return format!("git@{after_at}");
        }
    }

    token.to_string()
}

fn scrub_query_string(query: &str) -> String {
    let (q_part, frag_part) = match query.split_once('#') {
        Some((q, f)) => (q, Some(f)),
        None => (query, None),
    };
    let mut pairs = Vec::new();
    for pair in q_part.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let k_lower = k.to_ascii_lowercase();
            if k_lower.contains("token")
                || k_lower.contains("key")
                || k_lower.contains("secret")
                || k_lower.contains("pass")
                || k_lower.contains("sig")
                || k_lower.contains("auth")
                || k_lower.contains("cred")
                || is_secret_token_literal(v)
            {
                pairs.push(format!("{k}=REDACTED"));
            } else {
                pairs.push(pair.to_string());
            }
        } else {
            pairs.push(pair.to_string());
        }
    }
    let joined = pairs.join("&");
    match frag_part {
        Some(_) => format!("{joined}#REDACTED"),
        None => joined,
    }
}

fn is_secret_token_literal(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.starts_with("ghp_")
        || lower.starts_with("gho_")
        || lower.starts_with("ghu_")
        || lower.starts_with("ghs_")
        || lower.starts_with("ghr_")
        || lower.starts_with("github_pat_")
        || lower.starts_with("glpat-")
        || lower.starts_with("sk-")
        || lower.starts_with("xoxb-")
        || lower.starts_with("xoxp-")
        || lower.starts_with("xoxa-")
        || lower.starts_with("xoxr-")
        || lower.starts_with("xoxs-")
        || ((s.starts_with("AKIA") || s.starts_with("ASIA")) && s.len() >= 16)
        || (s.starts_with("AIza") && s.len() >= 20)
        || (s.starts_with("eyJ") && s.matches('.').count() >= 2 && s.len() >= 24)
}

fn extract_clean_repo_name(raw: &str) -> String {
    let without_query = raw.split(['?', '#']).next().unwrap_or(raw);
    let clean = without_query
        .trim()
        .trim_end_matches(".git")
        .trim_end_matches('/');
    let name = clean
        .split(['/', ':', '\\'])
        .next_back()
        .filter(|s| !s.is_empty())
        .unwrap_or(clean);
    scrub_credentials(name)
}

fn is_valid_commit_sha(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_safe_opaque_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !is_secret_token_literal(s)
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn normalize_terminal_type_minimal(raw: &str) -> &'static str {
    let trimmed = raw.trim();
    for &known in SAFE_TERMINAL_TYPES {
        if trimmed.eq_ignore_ascii_case(known) {
            return known;
        }
    }
    "other"
}

fn normalize_project_type_minimal(raw: &str) -> &'static str {
    let trimmed = raw.trim();
    for &known in SAFE_PROJECT_TYPES {
        if trimmed.eq_ignore_ascii_case(known) {
            return known;
        }
    }
    "other"
}

fn normalize_vcs_system_minimal(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "git" => "git",
        "none" => "none",
        _ => "other",
    }
}

fn normalize_tool_binary_minimal(raw: &str) -> Option<String> {
    let stem = std::path::Path::new(raw.trim())
        .file_stem()
        .and_then(|s| s.to_str())?;
    for &safe in SAFE_TOOL_BINARIES {
        if stem.eq_ignore_ascii_case(safe) {
            return Some(safe.to_string());
        }
    }
    None
}

fn normalize_tool_binary_opt_in(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') || trimmed.contains("://") {
        return None;
    }
    let stem = std::path::Path::new(trimmed)
        .file_stem()
        .and_then(|s| s.to_str())?
        .to_ascii_lowercase();
    if stem.is_empty() || is_secret_token_literal(&stem) {
        return None;
    }
    Some(scrub_credentials(&stem))
}

fn project_tool_name_minimal(raw: &str, key: Option<&PseudonymKey>) -> String {
    let trimmed = raw.trim();
    if SAFE_BUILTIN_TOOLS.contains(&trimmed) {
        return trimmed.to_string();
    }
    for &known in SAFE_BUILTIN_TOOLS {
        if trimmed.eq_ignore_ascii_case(known) {
            return trimmed.to_string();
        }
    }
    pseudonym(key, "tool", trimmed)
}

/// Projects an agent name according to `mode`.
fn project_agent_name(
    raw: &str,
    mode: CaptureMode,
    key: Option<&PseudonymKey>,
    domain: &str,
) -> String {
    let trimmed = raw.trim();
    match mode {
        CaptureMode::Minimal => {
            for &known in SAFE_AGENT_NAMES {
                if trimmed.eq_ignore_ascii_case(known) {
                    return known.to_string();
                }
            }
            pseudonym(key, domain, trimmed)
        }
        CaptureMode::MetadataOnly | CaptureMode::Full => scrub_credentials(trimmed),
    }
}

/// Projects a resource before export, stamping `agent.capture.mode` and enforcing
/// mode-appropriate attribute redaction.
pub fn project_resource(resource: &mut Resource, mode: CaptureMode) {
    if mode == CaptureMode::Minimal {
        // Exact keys only: no future OTEL_RESOURCE_ATTRIBUTES key is implicitly trusted.
        resource.attributes.retain(|kv| {
            matches!(
                kv.key.as_str(),
                "service.name"
                    | "service.version"
                    | "service.namespace"
                    | "deployment.environment"
                    | "deployment.environment.name"
                    | "telemetry.sdk.name"
                    | "telemetry.sdk.language"
                    | "telemetry.sdk.version"
                    | "telemetry.distro.name"
                    | "telemetry.distro.version"
                    | "os.type"
                    | AGENT_CAPTURE_MODE
            )
        });
        resource.attributes.retain(|kv| {
            kv.key != "service.namespace"
                || matches!(kv.value.as_ref().and_then(|v| v.value.as_ref()),
                Some(any_value::Value::StringValue(s)) if s == "agent-otel-bridge")
        });
        // Reject malformed/non-string resource overrides as well.
        resource.attributes.retain(|kv| {
            matches!(
                kv.value.as_ref().and_then(|v| v.value.as_ref()),
                Some(any_value::Value::StringValue(_))
            )
        });
    }

    for kv in &mut resource.attributes {
        if let Some(val) = &mut kv.value {
            if let Some(any_value::Value::StringValue(s)) = &mut val.value {
                *s = match mode {
                    CaptureMode::Minimal => scrub_minimal_resource_value(&kv.key, s),
                    CaptureMode::MetadataOnly | CaptureMode::Full => scrub_credentials(s),
                };
            }
        }
    }

    let mode_kv = kv_string(AGENT_CAPTURE_MODE, mode.as_str());
    if let Some(existing) = resource
        .attributes
        .iter_mut()
        .find(|kv| kv.key == AGENT_CAPTURE_MODE)
    {
        *existing = mode_kv;
    } else {
        resource.attributes.push(mode_kv);
    }
}

fn scrub_minimal_resource_value(key: &str, value: &str) -> String {
    match key {
        "service.name" | "telemetry.sdk.name" | "telemetry.distro.name" | "service.namespace" => {
            "agent-otel-bridge".to_string()
        }
        "service.version" | "telemetry.sdk.version" | "telemetry.distro.version" => {
            env!("CARGO_PKG_VERSION").to_string()
        }
        "telemetry.sdk.language" => "rust".to_string(),
        "deployment.environment" | "deployment.environment.name" => match value {
            "development" | "dev" | "test" | "staging" | "production" | "prod" | "local" => {
                value.to_string()
            }
            _ => "other".to_string(),
        },
        "os.type" => match value {
            "windows" | "linux" | "darwin" | "freebsd" | "openbsd" | "netbsd" | "dragonfly"
            | "solaris" | "illumos" | "android" => value.to_string(),
            _ => "other".to_string(),
        },
        _ => "other".to_string(),
    }
}

/// Projects normalized [`AgentHookInput`] and [`ResolvedSpanMetadata`] at daemon ingress
/// before span construction.
pub fn project_input_and_metadata(
    input: &mut AgentHookInput,
    metadata: &mut ResolvedSpanMetadata<'_>,
    mode: CaptureMode,
    workspace_path_mode: WorkspacePathMode,
    key: Option<&PseudonymKey>,
) {
    input.tool_archetype = input
        .tool_archetype
        .as_deref()
        .map(|s| agent_otel_core::archetype::ToolArchetype::normalize_str_name(s).to_string());
    input.capability_kind = input
        .capability_kind
        .as_deref()
        .map(|s| agent_otel_core::capability::CapabilityKind::normalize_str_name(s).to_string());
    // 1. Closed error category enforcement (all modes)
    if input.error.is_some() || input.error_category.is_some() {
        let closed_cat =
            classify_closed_error_category(input.error_category.as_deref(), input.error.as_deref());
        input.error_category = Some(closed_cat.to_string());
    }

    // 2. Always sanitize vcs_repository URL credentials/host before mode-specific projection
    if let Some(repo) = input.vcs_repository.take() {
        let cleaned = extract_clean_repo_name(&repo);
        if !cleaned.is_empty() {
            input.vcs_repository = Some(cleaned);
        }
    }

    match mode {
        CaptureMode::Minimal => {
            // Drop operator email completely
            input.user_email = None;
            metadata.user_email = None;

            // Closed terminal type vocabulary
            input.terminal_type = input
                .terminal_type
                .as_deref()
                .map(|t| normalize_terminal_type_minimal(t).to_string());
            metadata.terminal_type = metadata.terminal_type.map(normalize_terminal_type_minimal);

            // Paths are omitted, not dictionary-guessable hashes.
            input.workspace_path = None;
            input.project_root = None;
            if let Some(name) = input.project_name.take() {
                input.project_name = Some(pseudonym(key, "proj", &name));
            }
            if let Some(ptype) = input.project_type.take() {
                input.project_type = Some(normalize_project_type_minimal(&ptype).to_string());
            }

            // VCS fields
            if let Some(sys) = input.vcs_system.take() {
                input.vcs_system = Some(normalize_vcs_system_minimal(&sys).to_string());
            }
            if let Some(repo) = input.vcs_repository.take() {
                input.vcs_repository = Some(pseudonym(key, "repo", &repo));
            }
            if let Some(branch) = input.vcs_branch.take() {
                input.vcs_branch = Some(categorize_branch(&branch).to_string());
            }
            if let Some(commit) = input.vcs_commit.take() {
                let trimmed = commit.trim();
                if is_valid_commit_sha(trimmed) {
                    input.vcs_commit =
                        Some(pseudonym(key, "commit", &trimmed.to_ascii_lowercase()));
                }
            }

            // Tool binaries restricted to closed allowlist
            input.tool_binary = input
                .tool_binary
                .as_deref()
                .and_then(normalize_tool_binary_minimal);
            input.tool_wrapped_binary = input
                .tool_wrapped_binary
                .as_deref()
                .and_then(normalize_tool_binary_minimal);

            // Tool name & capabilities
            let raw_tool = input.resolved_tool_name().map(str::to_owned);
            if let Some(t) = raw_tool {
                let projected_tool = project_tool_name_minimal(&t, key);
                let is_safe_builtin = projected_tool == t.trim();
                input.tool_name = Some(projected_tool.clone());
                if let Some(tc) = &mut input.tool_call {
                    tc.name = Some(projected_tool.clone());
                    tc.arguments = None;
                    if let Some(id) = tc.id.take() {
                        tc.id = Some(if is_safe_opaque_id(&id) {
                            id
                        } else {
                            pseudonym(key, "call", &id)
                        });
                    }
                }
                if is_safe_builtin {
                    input.capability_namespace =
                        Some(match input.capability_namespace.as_deref() {
                            Some("agent") => "agent".to_string(),
                            _ => "core".to_string(),
                        });
                    input.capability_name = Some(projected_tool);
                } else {
                    if let Some(ns) = input.capability_namespace.take() {
                        input.capability_namespace =
                            Some(if matches!(ns.as_str(), "core" | "agent") {
                                ns
                            } else {
                                pseudonym(key, "ns", &ns)
                            });
                    }
                    if let Some(cap) = input.capability_name.take() {
                        input.capability_name = Some(pseudonym(key, "cap", &cap));
                    }
                }
            } else {
                if let Some(ns) = input.capability_namespace.take() {
                    input.capability_namespace = Some(if matches!(ns.as_str(), "core" | "agent") {
                        ns
                    } else {
                        pseudonym(key, "ns", &ns)
                    });
                }
                if let Some(cap) = input.capability_name.take() {
                    input.capability_name = Some(if SAFE_BUILTIN_TOOLS.contains(&cap.as_str()) {
                        cap
                    } else {
                        pseudonym(key, "cap", &cap)
                    });
                }
            }

            // Drop raw tool_input and transcript_path
            input.tool_input = None;
            input.transcript_path = None;
            input.workspace_paths = None;

            // Agent & lineage identifiers
            if let Some(agent) = input.agent_name.take() {
                input.agent_name = Some(project_agent_name(
                    &agent,
                    CaptureMode::Minimal,
                    key,
                    "agent",
                ));
            }
            if let Some(parent) = input.agent_parent_name.take() {
                input.agent_parent_name = Some(project_agent_name(
                    &parent,
                    CaptureMode::Minimal,
                    key,
                    "parent_agent",
                ));
            }
            if let Some(root_id) = input.agent_root_id.take() {
                input.agent_root_id = key.map(|key| key.pseudonym("root", &root_id));
            }
            if let Some(conv_id) = input.conversation_id.take() {
                // A2: the harness-native session is private even when it is opaque.
                input.conversation_id = key.map(|key| key.pseudonym("conv", &conv_id));
            }

            // Model name
            if let Some(model) = input.model.take() {
                let trimmed = model.trim();
                if trimmed.contains('/')
                    || trimmed.contains('\\')
                    || trimmed.contains('@')
                    || trimmed.contains(char::is_whitespace)
                    || is_secret_token_literal(trimmed)
                {
                    input.model = Some(pseudonym(key, "model", trimmed));
                } else {
                    input.model = Some(scrub_credentials(trimmed));
                }
            }

            // Closed termination reason & decision
            if let Some(reason) = input.termination_reason.take() {
                input.termination_reason = Some(categorize_termination_reason(&reason).to_string());
            }
            if let Some(decision) = input.decision.take() {
                input.decision = Some(categorize_decision(&decision).to_string());
            }

            // Replace raw error text with the closed error category before span construction
            if input.error.is_some() {
                let closed = input
                    .error_category
                    .clone()
                    .unwrap_or_else(|| "tool_verification_failed".to_string());
                input.error = Some(closed);
            }
            // Missing key: omit each field requiring a pseudonym and still build the span.
            for field in [
                &mut input.project_name,
                &mut input.vcs_repository,
                &mut input.vcs_commit,
                &mut input.tool_name,
                &mut input.capability_namespace,
                &mut input.capability_name,
                &mut input.agent_name,
                &mut input.agent_parent_name,
                &mut input.agent_root_id,
                &mut input.conversation_id,
                &mut input.model,
            ] {
                if field.as_deref() == Some("") {
                    *field = None;
                }
            }
            if let Some(tc) = &mut input.tool_call {
                if tc.name.as_deref() == Some("") {
                    tc.name = None;
                }
                if tc.id.as_deref() == Some("") {
                    tc.id = None;
                }
            }
        }
        CaptureMode::MetadataOnly => {
            apply_workspace_path_mode(input, workspace_path_mode, key);
            input.tool_input = None;
            input.transcript_path = None;
            input.workspace_paths = None;

            input.tool_binary = input
                .tool_binary
                .as_deref()
                .and_then(normalize_tool_binary_opt_in);
            input.tool_wrapped_binary = input
                .tool_wrapped_binary
                .as_deref()
                .and_then(normalize_tool_binary_opt_in);

            // Replace raw error text with closed error category in MetadataOnly mode too
            if input.error.is_some() {
                let closed = input
                    .error_category
                    .clone()
                    .unwrap_or_else(|| "tool_verification_failed".to_string());
                input.error = Some(closed);
            }

            scrub_all_input_strings(input);
        }
        CaptureMode::Full => {
            apply_workspace_path_mode(input, workspace_path_mode, key);
            input.tool_input = None;
            input.transcript_path = None;
            input.workspace_paths = None;

            input.tool_binary = input
                .tool_binary
                .as_deref()
                .and_then(normalize_tool_binary_opt_in);
            input.tool_wrapped_binary = input
                .tool_wrapped_binary
                .as_deref()
                .and_then(normalize_tool_binary_opt_in);

            if let Some(err) = input.error.take() {
                input.error = Some(truncate_utf8(&scrub_credentials(&err), 160));
            }

            scrub_all_input_strings(input);
        }
    }
}

fn scrub_all_input_strings(input: &mut AgentHookInput) {
    for field in [
        &mut input.conversation_id,
        &mut input.tool_name,
        &mut input.termination_reason,
        &mut input.model,
        &mut input.agent_name,
        &mut input.user_email,
        &mut input.terminal_type,
        &mut input.decision,
        &mut input.workspace_path,
        &mut input.project_name,
        &mut input.project_root,
        &mut input.project_type,
        &mut input.vcs_system,
        &mut input.vcs_repository,
        &mut input.vcs_branch,
        &mut input.vcs_commit,
        &mut input.capability_namespace,
        &mut input.capability_name,
        &mut input.agent_parent_name,
        &mut input.agent_root_id,
    ] {
        if let Some(val) = field.take() {
            *field = Some(scrub_credentials(&val));
        }
    }
    if let Some(tc) = &mut input.tool_call {
        tc.arguments = None;
        if let Some(id) = tc.id.take() {
            tc.id = Some(scrub_credentials(&id));
        }
        if let Some(name) = tc.name.take() {
            tc.name = Some(scrub_credentials(&name));
        }
    }
}

/// Applies [`WorkspacePathMode`] to `workspace_path` and `project_root`.
pub fn apply_workspace_path_mode(
    input: &mut AgentHookInput,
    mode: WorkspacePathMode,
    key: Option<&PseudonymKey>,
) {
    match mode {
        WorkspacePathMode::Absolute => {}
        WorkspacePathMode::Relative => apply_relative_workspace(input),
        WorkspacePathMode::Hash => {
            if let Some(path) = input.workspace_path.clone() {
                input.workspace_path = key.map(|key| key.pseudonym("path", &path));
            }
            if let Some(root) = input.project_root.clone() {
                input.project_root = key.map(|key| key.pseudonym("root_path", &root));
            }
        }
    }
}

fn apply_relative_workspace(input: &mut AgentHookInput) {
    let root = input.project_root.clone();
    input.workspace_path = match (input.workspace_path.as_deref(), root.as_deref()) {
        (Some(path), Some(root)) => relative_workspace_path(path, root),
        (Some(_), None) => None,
        (None, _) => None,
    };
    if let Some(root) = root.as_deref() {
        input.project_root = final_path_component(root);
    }
}

fn relative_workspace_path(path: &str, root: &str) -> Option<String> {
    let relative = std::path::Path::new(path)
        .strip_prefix(std::path::Path::new(root))
        .ok()?;
    if relative.as_os_str().is_empty() {
        Some(".".to_string())
    } else {
        relative.to_str().map(str::to_owned)
    }
}

fn final_path_component(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Post-construction span defense-in-depth projection.
/// Ensures status message, span events, and attributes adhere to `mode`.
pub fn project_span(span: &mut Span, mode: CaptureMode) {
    match mode {
        CaptureMode::Minimal | CaptureMode::MetadataOnly => {
            span.events.clear();
            if let Some(status) = &mut span.status {
                if status.code == StatusCode::Error as i32 && !status.message.is_empty() {
                    let closed = span
                        .attributes
                        .iter()
                        .find(|kv| kv.key == AGENT_ERROR_CATEGORY || kv.key == ERROR_TYPE)
                        .and_then(|kv| match kv.value.as_ref()?.value.as_ref()? {
                            any_value::Value::StringValue(s) => Some(s.as_str()),
                            _ => None,
                        });
                    status.message =
                        classify_closed_error_category(closed, Some(&status.message)).to_string();
                }
            }
        }
        CaptureMode::Full => {
            if let Some(status) = &mut span.status {
                if !status.message.is_empty() {
                    status.message = truncate_utf8(&scrub_credentials(&status.message), 160);
                }
            }
            for event in &mut span.events {
                event.name = scrub_credentials(&event.name);
                for kv in &mut event.attributes {
                    if let Some(val) = &mut kv.value {
                        if let Some(any_value::Value::StringValue(s)) = &mut val.value {
                            *s = scrub_credentials(s);
                        }
                    }
                }
            }
        }
    }
    if mode == CaptureMode::Minimal {
        span.attributes
            .retain(|kv| MINIMAL_SPAN_KEYS.contains(&kv.key.as_str()));
    }
}

/// Reviewed Minimal span vocabulary. New attributes require an explicit privacy decision.
const MINIMAL_SPAN_KEYS: &[&str] = &[
    "agent.session.link_id",
    "agent.session.unlinked_reason",
    "gen_ai.operation.name",
    "gen_ai.system",
    "gen_ai.provider.name",
    "gen_ai.request.model",
    "gen_ai.conversation.id",
    "gen_ai.tool.name",
    "gen_ai.tool.call.id",
    "gen_ai.agent.name",
    "gen_ai.agent.depth",
    "gen_ai.agent.parent_name",
    "gen_ai.agent.root_id",
    "gen_ai.agent.is_root",
    "gen_ai.agent.step_index",
    "gen_ai.usage.input_tokens",
    "gen_ai.usage.output_tokens",
    "gen_ai.usage.cache_read.input_tokens",
    "gen_ai.usage.cache_creation.input_tokens",
    "gen_ai.response.finish_reasons",
    "agent.hook.event",
    "agent.step.index",
    "agent.execution.mode",
    "agent.termination_reason",
    "agent.decision",
    "agent.error.category",
    "error.type",
    "terminal.type",
    "workspace.project_name",
    "workspace.project_type",
    "vcs.system",
    "vcs.repository.name",
    "vcs.branch.name",
    "vcs.ref.head.name",
    "vcs.commit.sha",
    "vcs.ref.head.revision",
    "vcs.worktree",
    "agent.tool.archetype",
    "agent.tool.binary",
    "agent.tool.pipeline_depth",
    "agent.tool.wrapped_binary",
    "agent.tool.exit_code",
    "agent.tool.raw_bytes",
    "agent.tool.compressed_bytes",
    "agent.tool.tokens_saved",
    "capability.kind",
    "capability.namespace",
    "capability.name",
    "capability.schema_tokens",
    "agent.lineage.depth",
    "agent.lineage.parent_name",
    "agent.lineage.root_id",
    "agent.lineage.is_root",
    // Static context diagnostics and assembler outcomes added after input projection.
    "agent.context.state",
    "agent.context.source",
    "agent.context.age_ms",
    "agent.span.paired",
    "agent.span.duration_source",
    "agent.span.incomplete_reason",
    "gen_ai.usage.cache_read_tokens",
    "agent.success",
    "agent.git.lines_added",
    "agent.git.lines_deleted",
    "agent.git.files_changed",
    "agent.git.self_revert",
    "vcs.worktree.active",
    "agent.tool.compression_ratio",
    "capability.response_bytes",
    "capability.response_tokens",
    "capability.consecutive_retries",
    "capability.is_waste",
    "agent.execution.num",
    "agent.fully_idle",
    "agy.hook.event",
    "agy.termination_reason",
    "agy.execution.num",
    "agy.fully_idle",
    "agy.step.index",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_commit_pseudonyms_require_full_sha_and_key() {
        let dir = std::env::temp_dir().join(format!("aob-commit-key-{}", std::process::id()));
        let key = PseudonymKey::load_or_create(&dir).unwrap();
        for raw in ["ABCDEF0123456789ABCDEF0123456789ABCDEF01", &"AB".repeat(32)] {
            for available_key in [Some(&key), None] {
                let mut input = AgentHookInput {
                    vcs_commit: Some(raw.into()),
                    ..Default::default()
                };
                project_input_and_metadata(
                    &mut input,
                    &mut ResolvedSpanMetadata::default(),
                    CaptureMode::Minimal,
                    WorkspacePathMode::Hash,
                    available_key,
                );
                assert_eq!(
                    input.vcs_commit,
                    available_key.map(|key| key.pseudonym("commit", &raw.to_ascii_lowercase()))
                );
            }
        }
        for raw in ["abcdef0", &"a".repeat(41), &"z".repeat(40)] {
            let mut input = AgentHookInput {
                vcs_commit: Some(raw.into()),
                ..Default::default()
            };
            project_input_and_metadata(
                &mut input,
                &mut ResolvedSpanMetadata::default(),
                CaptureMode::Minimal,
                WorkspacePathMode::Hash,
                Some(&key),
            );
            assert_eq!(input.vcs_commit, None);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resource_projection_is_default_deny_and_fixes_bridge_identity() {
        let mut resource = Resource {
            attributes: [
                "enduser.id",
                "team",
                "host.id",
                "k8s.pod.name",
                "os.description",
                "host.name",
                "service.instance.id",
                "future.secret",
                "telemetry.future.secret",
            ]
            .into_iter()
            .map(|key| kv_string(key, "SENTINEL"))
            .collect(),
            ..Default::default()
        };
        resource.attributes.extend([
            kv_string("service.name", "SENTINEL"),
            kv_string("service.namespace", "SENTINEL"),
            kv_string("service.version", "SENTINEL"),
            kv_string("deployment.environment.name", "SENTINEL"),
            kv_string("telemetry.sdk.name", "SENTINEL"),
            kv_string("os.type", "linux"),
        ]);
        project_resource(&mut resource, CaptureMode::Minimal);
        assert_eq!(resource.attributes.len(), 6);
        assert!(!format!("{resource:?}").contains("SENTINEL"));
        assert!(resource
            .attributes
            .iter()
            .any(|kv| kv == &kv_string("service.name", "agent-otel-bridge")));
        assert!(resource
            .attributes
            .iter()
            .any(|kv| kv == &kv_string("deployment.environment.name", "other")));
    }

    #[test]
    fn span_projection_drops_future_attributes() {
        let mut span = Span {
            attributes: vec![
                kv_string("future.secret", "SENTINEL"),
                kv_string("agent.hook.event", "PreToolUse"),
            ],
            ..Default::default()
        };
        project_span(&mut span, CaptureMode::Minimal);
        assert_eq!(
            span.attributes,
            vec![kv_string("agent.hook.event", "PreToolUse")]
        );
    }

    #[test]
    fn scrubber_covers_assignments_json_headers_cli_and_aws_secrets() {
        for raw in [
            "password: SIMPLE_SENTINEL",
            r#"{"token":"SIMPLE_SENTINEL","other":"visible"}"#,
            "--token SIMPLE_SENTINEL",
            "x-api-key: aabbccddSIMPLE_SENTINEL",
            "AWS_SECRET_ACCESS_KEY=SIMPLE_SENTINEL",
            "aws_secret_access_key: SIMPLE_SENTINEL",
            r#"{"SecretAccessKey": "SIMPLE_SENTINEL"}"#,
            "AWS_SESSION_TOKEN=SIMPLE_SENTINEL",
            "--api-key SIMPLE_SENTINEL",
            "Authorization: Bearer SIMPLE_SENTINEL",
        ] {
            let scrubbed = scrub_credentials(raw);
            assert!(!scrubbed.contains("SIMPLE_SENTINEL"), "{scrubbed}");
            assert!(scrubbed.contains("REDACTED"));
        }
        assert_eq!(scrub_credentials("harmless text"), "harmless text");
    }

    #[test]
    fn enums_are_closed_in_all_modes_even_for_constructed_inputs() {
        for mode in [
            CaptureMode::Minimal,
            CaptureMode::MetadataOnly,
            CaptureMode::Full,
        ] {
            let mut input = AgentHookInput {
                tool_archetype: Some("SENTINEL".into()),
                capability_kind: Some("SENTINEL".into()),
                ..Default::default()
            };
            project_input_and_metadata(
                &mut input,
                &mut ResolvedSpanMetadata::default(),
                mode,
                WorkspacePathMode::Hash,
                None,
            );
            assert_eq!(input.tool_archetype.as_deref(), Some("other"));
            assert_eq!(input.capability_kind.as_deref(), Some("other"));
        }
    }
}
