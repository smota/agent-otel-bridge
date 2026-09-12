/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolArchetype {
    FilterCompressor,
    StructuredParser,
    InspectorDiff,
    SearchRetrieval,
    BuildTestVerify,
    GenericExec,
}

impl ToolArchetype {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolArchetype::FilterCompressor => "filter_compressor",
            ToolArchetype::StructuredParser => "structured_parser",
            ToolArchetype::InspectorDiff => "inspector_diff",
            ToolArchetype::SearchRetrieval => "search_retrieval",
            ToolArchetype::BuildTestVerify => "build_test_verify",
            ToolArchetype::GenericExec => "generic_exec",
        }
    }

    pub fn from_str_name(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "filter_compressor" | "filter" | "compressor" | "proxy" => {
                ToolArchetype::FilterCompressor
            }
            "structured_parser" | "parser" | "json" | "yaml" => ToolArchetype::StructuredParser,
            "inspector_diff" | "inspector" | "diff" | "viewer" => ToolArchetype::InspectorDiff,
            "search_retrieval" | "search" | "retrieval" | "finder" => {
                ToolArchetype::SearchRetrieval
            }
            "build_test_verify" | "build" | "test" | "verify" | "linter" => {
                ToolArchetype::BuildTestVerify
            }
            _ => ToolArchetype::GenericExec,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifiedCommand {
    pub archetype: ToolArchetype,
    pub binary: String,
    pub pipeline_depth: usize,
    pub wrapped_binary: Option<String>,
}

impl ClassifiedCommand {
    pub fn classify(raw_cmd: &str) -> Self {
        let trimmed = raw_cmd.trim();
        if trimmed.is_empty() {
            return Self {
                archetype: ToolArchetype::GenericExec,
                binary: String::new(),
                pipeline_depth: 1,
                wrapped_binary: None,
            };
        }

        // 1. Calculate pipeline depth (number of commands connected by `|`)
        let pipeline_depth = count_pipeline_segments(trimmed);

        // 2. Extract leading command/binary
        let first_segment = trimmed.split('|').next().unwrap_or(trimmed).trim();
        let tokens: Vec<&str> = first_segment.split_whitespace().collect();
        let raw_binary = tokens.first().copied().unwrap_or("");
        let binary = normalize_binary(raw_binary);

        // 3. Check for proxy wrappers (e.g. `rtk <cmd>`, `tee`, `time <cmd>`)
        let (archetype, wrapped_binary) = if is_proxy_wrapper(&binary) {
            let wrapped = if tokens.len() > 1 {
                Some(normalize_binary(tokens[1]))
            } else {
                None
            };
            (ToolArchetype::FilterCompressor, wrapped)
        } else {
            // 4. Try workspace config overrides if present
            if let Some(arch) = check_user_config_override(&binary) {
                (arch, None)
            } else {
                // 5. Default heuristic classification
                (infer_archetype(&binary, trimmed), None)
            }
        };

        Self {
            archetype,
            binary,
            pipeline_depth,
            wrapped_binary,
        }
    }
}

fn count_pipeline_segments(cmd: &str) -> usize {
    let mut count = 1;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for ch in cmd.chars() {
        match ch {
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            '|' if !in_single_quote && !in_double_quote => count += 1,
            _ => {}
        }
    }
    count
}

fn normalize_binary(raw: &str) -> String {
    let clean = raw.trim_matches(['"', '\'', '`']);
    let path = Path::new(clean);
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(clean)
        .to_ascii_lowercase();
    name
}

fn is_proxy_wrapper(bin: &str) -> bool {
    matches!(bin, "rtk" | "tee" | "time" | "stdbuf" | "timeout")
}

fn infer_archetype(bin: &str, raw_cmd: &str) -> ToolArchetype {
    // Check if the command line contains structured extraction flags
    if raw_cmd.contains("| jq") || raw_cmd.contains("| yq") || raw_cmd.contains("| dasel") {
        return ToolArchetype::StructuredParser;
    }

    match bin {
        // Filter & Compression
        "rtk" | "head" | "tail" | "cut" | "sed" | "awk" | "fold" | "summarize" => {
            ToolArchetype::FilterCompressor
        }

        // Structured Parsers
        "jq" | "yq" | "dasel" | "fx" | "gron" | "xmllint" | "jmespath" => {
            ToolArchetype::StructuredParser
        }

        // Viewers & Differs
        "cat" | "bat" | "delta" | "diff" | "colordiff" | "less" | "more" | "view_file" | "nl"
        | "od" => ToolArchetype::InspectorDiff,

        // Search & Retrieval
        "rg" | "grep" | "findstr" | "fd" | "find" | "ag" | "ack" | "ast-grep" | "sg" | "locate"
        | "where" | "which" => ToolArchetype::SearchRetrieval,

        // Build, Test & Verification
        "cargo" | "npm" | "pnpm" | "yarn" | "npx" | "pytest" | "python"
            if raw_cmd.contains("test") =>
        {
            ToolArchetype::BuildTestVerify
        }
        "cargo" | "npm" | "pnpm" | "yarn" | "npx" | "pytest" | "tsc" | "eslint" | "ruff"
        | "golangci-lint" | "make" | "ninja" | "cmake" | "mvn" | "gradle" | "go" => {
            ToolArchetype::BuildTestVerify
        }

        _ => ToolArchetype::GenericExec,
    }
}

fn check_user_config_override(_bin: &str) -> Option<ToolArchetype> {
    // Check .agent-otel/tools.json in current directory or user home
    let local_config = Path::new(".agent-otel/tools.json");
    if local_config.exists() {
        if let Ok(content) = fs::read_to_string(local_config) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(mappings) = json.get("mappings").and_then(|m| m.as_object()) {
                    if let Some(arch_str) = mappings.get(_bin).and_then(|v| v.as_str()) {
                        return Some(ToolArchetype::from_str_name(arch_str));
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_proxy_filter() {
        let cmd = "rtk cargo test --lib";
        let res = ClassifiedCommand::classify(cmd);
        assert_eq!(res.archetype, ToolArchetype::FilterCompressor);
        assert_eq!(res.binary, "rtk");
        assert_eq!(res.wrapped_binary.as_deref(), Some("cargo"));
        assert_eq!(res.pipeline_depth, 1);
    }

    #[test]
    fn test_classify_structured_parser() {
        let cmd = "cat data.json | jq '.items[0]' | head -n 5";
        let res = ClassifiedCommand::classify(cmd);
        assert_eq!(res.archetype, ToolArchetype::StructuredParser);
        assert_eq!(res.pipeline_depth, 3);
    }

    #[test]
    fn test_classify_search_retrieval() {
        let cmd = "rg -i \"resolve_trace\" crates/";
        let res = ClassifiedCommand::classify(cmd);
        assert_eq!(res.archetype, ToolArchetype::SearchRetrieval);
        assert_eq!(res.binary, "rg");
    }

    #[test]
    fn test_classify_inspector_diff() {
        let cmd = "bat --paging=never Cargo.toml";
        let res = ClassifiedCommand::classify(cmd);
        assert_eq!(res.archetype, ToolArchetype::InspectorDiff);
        assert_eq!(res.binary, "bat");

        let delta = "delta --diff-so-fancy HEAD~1";
        let res2 = ClassifiedCommand::classify(delta);
        assert_eq!(res2.archetype, ToolArchetype::InspectorDiff);
        assert_eq!(res2.binary, "delta");
    }

    #[test]
    fn test_classify_build_verify() {
        let cmd = "cargo clippy --all-targets";
        let res = ClassifiedCommand::classify(cmd);
        assert_eq!(res.archetype, ToolArchetype::BuildTestVerify);
        assert_eq!(res.binary, "cargo");
    }
}
