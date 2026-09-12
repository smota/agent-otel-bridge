/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Native,
    Skill,
    Mcp,
    Subagent,
}

impl CapabilityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CapabilityKind::Native => "native",
            CapabilityKind::Skill => "skill",
            CapabilityKind::Mcp => "mcp",
            CapabilityKind::Subagent => "subagent",
        }
    }

    pub fn from_str_name(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "mcp" | "model_context_protocol" => CapabilityKind::Mcp,
            "skill" | "skillz" | "skills" => CapabilityKind::Skill,
            "subagent" | "agent" | "delegation" => CapabilityKind::Subagent,
            _ => CapabilityKind::Native,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifiedCapability {
    pub kind: CapabilityKind,
    pub namespace: String,
    pub operation: String,
}

impl ClassifiedCapability {
    pub fn classify(tool_name: &str) -> Self {
        let trimmed = tool_name.trim();

        // 1. Check for Claude Code & general MCP naming: mcp__<server>__<tool>
        if let Some(rest) = trimmed.strip_prefix("mcp__") {
            let parts: Vec<&str> = rest.splitn(2, "__").collect();
            if parts.len() == 2 {
                return Self {
                    kind: CapabilityKind::Mcp,
                    namespace: parts[0].to_string(),
                    operation: parts[1].to_string(),
                };
            } else {
                return Self {
                    kind: CapabilityKind::Mcp,
                    namespace: parts[0].to_string(),
                    operation: parts[0].to_string(),
                };
            }
        }

        // 2. Check for skills: skill__<skill_name>__<tool>
        if let Some(rest) = trimmed.strip_prefix("skill__") {
            let parts: Vec<&str> = rest.splitn(2, "__").collect();
            if parts.len() == 2 {
                return Self {
                    kind: CapabilityKind::Skill,
                    namespace: parts[0].to_string(),
                    operation: parts[1].to_string(),
                };
            } else {
                return Self {
                    kind: CapabilityKind::Skill,
                    namespace: parts[0].to_string(),
                    operation: parts[0].to_string(),
                };
            }
        }

        // 3. Subagent invocations
        if trimmed == "invoke_subagent"
            || trimmed == "manage_subagents"
            || trimmed == "define_subagent"
            || trimmed == "run_subagent"
        {
            return Self {
                kind: CapabilityKind::Subagent,
                namespace: "fleet".to_string(),
                operation: trimmed.to_string(),
            };
        }

        // 4. Default native built-in tool
        Self {
            kind: CapabilityKind::Native,
            namespace: "core".to_string(),
            operation: trimmed.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CapabilityWasteMetrics {
    pub schema_tokens: Option<i64>,
    pub response_bytes: Option<u64>,
    pub response_tokens: Option<i64>,
    pub consecutive_retries: Option<u32>,
    pub is_waste: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_mcp() {
        let res = ClassifiedCapability::classify("mcp__github__create_issue");
        assert_eq!(res.kind, CapabilityKind::Mcp);
        assert_eq!(res.namespace, "github");
        assert_eq!(res.operation, "create_issue");
    }

    #[test]
    fn test_classify_skill() {
        let res = ClassifiedCapability::classify("skill__agy-customizations__explain");
        assert_eq!(res.kind, CapabilityKind::Skill);
        assert_eq!(res.namespace, "agy-customizations");
        assert_eq!(res.operation, "explain");
    }

    #[test]
    fn test_classify_subagent() {
        let res = ClassifiedCapability::classify("invoke_subagent");
        assert_eq!(res.kind, CapabilityKind::Subagent);
        assert_eq!(res.namespace, "fleet");
        assert_eq!(res.operation, "invoke_subagent");
    }

    #[test]
    fn test_classify_native() {
        let res = ClassifiedCapability::classify("view_file");
        assert_eq!(res.kind, CapabilityKind::Native);
        assert_eq!(res.namespace, "core");
        assert_eq!(res.operation, "view_file");
    }
}
