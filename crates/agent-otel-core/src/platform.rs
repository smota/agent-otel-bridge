/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookResponse {
    /// Standard allow JSON response: `{"decision":"allow"}`
    AllowJson,
    /// Minimal empty JSON response: `{}`
    EmptyJson,
}

impl HookResponse {
    #[inline]
    pub fn as_bytes(&self) -> &'static [u8] {
        match self {
            HookResponse::AllowJson => b"{\"decision\":\"allow\"}",
            HookResponse::EmptyJson => b"{}",
        }
    }

    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            HookResponse::AllowJson => "{\"decision\":\"allow\"}",
            HookResponse::EmptyJson => "{}",
        }
    }
}

/// Core interface representing an AI agent platform harness descriptor.
pub trait PlatformDescriptor: Send + Sync + 'static {
    /// Canonical platform identifier (e.g. "antigravity", "claude", "codex", "hermes").
    fn id(&self) -> &'static str;

    /// Human-friendly display name (e.g. "Google Antigravity", "Anthropic Claude Code").
    fn display_name(&self) -> &'static str;

    /// Alternate aliases recognized for this platform (e.g. `["gemini", "agy"]`).
    fn aliases(&self) -> &'static [&'static str];

    /// Wire client identifier used in binary IPC framing (strictly 1..=15, 4-bit header).
    fn wire_client_id(&self) -> u8;

    /// PreToolUse hook response policy expected by this agent CLI.
    fn pre_tool_response(&self) -> HookResponse {
        HookResponse::AllowJson
    }

    /// Checks if a given query matches this platform's primary id or any alias.
    fn matches_name(&self, name: &str) -> bool {
        let lower = name.trim().to_ascii_lowercase();
        self.id() == lower || self.aliases().iter().any(|&a| a == lower)
    }
}

// Built-in Platform Descriptors

pub struct AntigravityDescriptor;
impl PlatformDescriptor for AntigravityDescriptor {
    fn id(&self) -> &'static str {
        "antigravity"
    }
    fn display_name(&self) -> &'static str {
        "Google Antigravity"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["agy", "gemini"]
    }
    fn wire_client_id(&self) -> u8 {
        1
    }
    fn pre_tool_response(&self) -> HookResponse {
        HookResponse::AllowJson
    }
}

pub struct ClaudeCodeDescriptor;
impl PlatformDescriptor for ClaudeCodeDescriptor {
    fn id(&self) -> &'static str {
        "claude"
    }
    fn display_name(&self) -> &'static str {
        "Claude Code"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["claude-code", "claudecode"]
    }
    fn wire_client_id(&self) -> u8 {
        2
    }
    fn pre_tool_response(&self) -> HookResponse {
        HookResponse::AllowJson
    }
}

pub struct CodexDescriptor;
impl PlatformDescriptor for CodexDescriptor {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn display_name(&self) -> &'static str {
        "OpenAI Codex"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["openai", "codex-cli"]
    }
    fn wire_client_id(&self) -> u8 {
        3
    }
    fn pre_tool_response(&self) -> HookResponse {
        HookResponse::EmptyJson
    }
}

pub struct GrokDescriptor;
impl PlatformDescriptor for GrokDescriptor {
    fn id(&self) -> &'static str {
        "grok"
    }
    fn display_name(&self) -> &'static str {
        "xAI Grok"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["xai", "grok-cli"]
    }
    fn wire_client_id(&self) -> u8 {
        4
    }
    fn pre_tool_response(&self) -> HookResponse {
        HookResponse::AllowJson
    }
}

pub struct PiDescriptor;
impl PlatformDescriptor for PiDescriptor {
    fn id(&self) -> &'static str {
        "pi"
    }
    fn display_name(&self) -> &'static str {
        "Pi Agent"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["pi-cli"]
    }
    fn wire_client_id(&self) -> u8 {
        5
    }
    fn pre_tool_response(&self) -> HookResponse {
        HookResponse::AllowJson
    }
}

pub static BUILTIN_PLATFORMS: &[&dyn PlatformDescriptor] = &[
    &AntigravityDescriptor,
    &ClaudeCodeDescriptor,
    &CodexDescriptor,
    &GrokDescriptor,
    &PiDescriptor,
];

pub fn find_platform_by_name(name: &str) -> Option<&'static (dyn PlatformDescriptor + 'static)> {
    BUILTIN_PLATFORMS
        .iter()
        .copied()
        .find(|p| p.matches_name(name))
}

pub fn find_platform_by_wire_id(id: u8) -> Option<&'static (dyn PlatformDescriptor + 'static)> {
    BUILTIN_PLATFORMS
        .iter()
        .copied()
        .find(|p| p.wire_client_id() == id)
}
