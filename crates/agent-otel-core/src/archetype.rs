/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolArchetype {
    FilterCompressor,
    StructuredParser,
    InspectorDiff,
    SearchRetrieval,
    BuildTestVerify,
    StateMutation,
    EnvPkgManager,
    VcsLifecycle,
    NetworkTransfer,
    #[serde(other)]
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
            ToolArchetype::StateMutation => "state_mutation",
            ToolArchetype::EnvPkgManager => "env_pkg_manager",
            ToolArchetype::VcsLifecycle => "vcs_lifecycle",
            ToolArchetype::NetworkTransfer => "network_transfer",
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
            "state_mutation" | "mutation" | "mutate" | "fs_mutation" | "write" => {
                ToolArchetype::StateMutation
            }
            "env_pkg_manager" | "pkg_manager" | "package" | "pkg" | "deps" | "dependency" => {
                ToolArchetype::EnvPkgManager
            }
            "vcs_lifecycle" | "vcs" | "git" | "source_control" => ToolArchetype::VcsLifecycle,
            "network_transfer" | "network" | "remote_fetch" | "http" | "download" => {
                ToolArchetype::NetworkTransfer
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

        // 1. Calculate pipeline depth (number of commands connected by `|`, quote-aware)
        let pipeline_depth = count_pipeline_segments(trimmed);

        // 2. Extract leading command/binary and arguments from first pipeline segment
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
            // 4. In-memory heuristic classification (< 5 µs hot path, zero disk I/O)
            (infer_archetype(&binary, trimmed, &tokens), None)
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
    let chars: Vec<char> = cmd.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];
        match ch {
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            '|' if !in_single_quote && !in_double_quote => {
                // If this is `||`, skip both characters so logical OR is not counted as pipe
                if i + 1 < len && chars[i + 1] == '|' {
                    i += 1;
                } else {
                    count += 1;
                }
            }
            _ => {}
        }
        i += 1;
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

fn extract_subcommand<'a>(tokens: &'a [&'a str]) -> Option<&'a str> {
    // Iterate from tokens[1] onwards, skipping options and their arguments
    let mut i = 1;
    while i < tokens.len() {
        let tok = tokens[i];
        if tok.starts_with('+') {
            // e.g. cargo +nightly or +stable
            i += 1;
            continue;
        }
        if tok.starts_with('-') {
            // Check flags that consume a following argument:
            // -C <dir>, -p <pkg>, --prefix <dir>, --manifest-path <file>, -m <module>, --cwd <dir>
            if matches!(
                tok,
                "-C" | "-p" | "--prefix" | "--manifest-path" | "-m" | "--cwd"
            ) {
                i += 2;
                continue;
            }
            // If flag with equals, e.g. --prefix=/path or -C=/path
            if tok.contains('=') {
                i += 1;
                continue;
            }
            // General boolean flag like --no-pager, -v, --verbose, --all
            i += 1;
            continue;
        }
        // First non-flag token is our subcommand
        return Some(tok);
    }
    None
}

fn infer_archetype(bin: &str, raw_cmd: &str, tokens: &[&str]) -> ToolArchetype {
    // 1. Check if the command line contains structured extraction pipes (e.g. `cat ... | jq`)
    if raw_cmd.contains("| jq") || raw_cmd.contains("| yq") || raw_cmd.contains("| dasel") {
        return ToolArchetype::StructuredParser;
    }

    // 2. Subcommand extraction for multi-tool binaries (git, gh, npm, cargo, etc.)
    let subcmd = extract_subcommand(tokens).unwrap_or("");

    match bin {
        // --- Version Control & Collaboration ---
        "git" => match subcmd {
            "diff" | "log" | "show" | "status" | "blame" | "whatchanged" => {
                ToolArchetype::InspectorDiff
            }
            "grep" => ToolArchetype::SearchRetrieval,
            "commit" | "push" | "pull" | "fetch" | "checkout" | "switch" | "branch" | "merge"
            | "rebase" | "stash" | "tag" | "reset" | "restore" | "cherry-pick" | "init"
            | "clone" | "remote" | "submodule" => ToolArchetype::VcsLifecycle,
            _ => ToolArchetype::VcsLifecycle,
        },
        "gh" => match subcmd {
            "api" => ToolArchetype::NetworkTransfer,
            "pr" | "issue" | "repo" | "workflow" | "run" | "release" | "gist" => {
                ToolArchetype::VcsLifecycle
            }
            _ => ToolArchetype::VcsLifecycle,
        },

        // --- Package & Environment Management ---
        "npm" | "pnpm" | "yarn" | "bun" => match subcmd {
            "test" | "run-script" if raw_cmd.contains("test") => ToolArchetype::BuildTestVerify,
            "lint" | "check" => ToolArchetype::BuildTestVerify,
            "install" | "i" | "add" | "update" | "up" | "remove" | "rm" | "uninstall" | "audit"
            | "outdated" | "link" | "ci" => ToolArchetype::EnvPkgManager,
            _ => {
                if raw_cmd.contains("test") {
                    ToolArchetype::BuildTestVerify
                } else {
                    ToolArchetype::EnvPkgManager
                }
            }
        },
        "cargo" => match subcmd {
            "test" | "check" | "clippy" | "build" | "bench" | "doc" | "verify" | "fmt" => {
                ToolArchetype::BuildTestVerify
            }
            "add" | "rm" | "install" | "uninstall" | "update" | "upgrade" | "generate-lockfile" => {
                ToolArchetype::EnvPkgManager
            }
            _ => ToolArchetype::BuildTestVerify,
        },
        "pip" | "pip3" | "poetry" | "uv" | "conda" | "mamba" | "pipenv" | "gem" | "apt"
        | "apt-get" | "brew" | "dnf" | "yum" | "pacman" | "nuget" => ToolArchetype::EnvPkgManager,
        "go" => match subcmd {
            "test" | "vet" | "build" => ToolArchetype::BuildTestVerify,
            "get" | "install" | "mod" => ToolArchetype::EnvPkgManager,
            _ => ToolArchetype::BuildTestVerify,
        },
        "python" | "python3" => {
            if raw_cmd.contains("-m pip") || raw_cmd.contains("-m uv") {
                ToolArchetype::EnvPkgManager
            } else if raw_cmd.contains("-m pytest")
                || raw_cmd.contains("-m unittest")
                || raw_cmd.contains("test")
            {
                ToolArchetype::BuildTestVerify
            } else {
                ToolArchetype::GenericExec
            }
        }

        // --- Network & Remote Transfer ---
        "curl" | "wget" | "httpie" | "http" | "fetch" | "ftp" | "sftp" | "rsync" | "scp" => {
            ToolArchetype::NetworkTransfer
        }

        // --- Filesystem & State Mutations ---
        "touch" | "mkdir" | "rm" | "rmdir" | "cp" | "mv" | "patch" | "truncate" | "chmod"
        | "chown" | "ln" | "unlink" | "write_to_file" => ToolArchetype::StateMutation,
        "sed" if raw_cmd.contains("-i") || raw_cmd.contains("--in-place") => {
            ToolArchetype::StateMutation
        }

        // --- Filter & Compression ---
        "rtk" | "head" | "tail" | "cut" | "sed" | "awk" | "fold" | "summarize" => {
            ToolArchetype::FilterCompressor
        }

        // --- Structured Parsers ---
        "jq" | "yq" | "dasel" | "fx" | "gron" | "xmllint" | "jmespath" => {
            ToolArchetype::StructuredParser
        }

        // --- Viewers & Differs ---
        "cat" | "bat" | "delta" | "diff" | "colordiff" | "less" | "more" | "view_file" | "nl"
        | "od" => ToolArchetype::InspectorDiff,

        // --- Search & Retrieval ---
        "rg" | "grep" | "findstr" | "fd" | "find" | "ag" | "ack" | "ast-grep" | "sg" | "locate"
        | "where" | "which" => ToolArchetype::SearchRetrieval,

        // --- Dedicated Build, Test & Lint Tools ---
        "pytest" | "tsc" | "eslint" | "ruff" | "golangci-lint" | "make" | "ninja" | "cmake"
        | "mvn" | "gradle" | "vitest" | "jest" | "mocha" => ToolArchetype::BuildTestVerify,

        _ => ToolArchetype::GenericExec,
    }
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

        // Git diff & log with options
        let git_diff = "git -C /repo diff HEAD~1";
        let res3 = ClassifiedCommand::classify(git_diff);
        assert_eq!(res3.archetype, ToolArchetype::InspectorDiff);
        assert_eq!(res3.binary, "git");

        let git_log = "git --no-pager log -n 5";
        let res4 = ClassifiedCommand::classify(git_log);
        assert_eq!(res4.archetype, ToolArchetype::InspectorDiff);
    }

    #[test]
    fn test_classify_build_verify() {
        let cmd = "cargo clippy --all-targets";
        let res = ClassifiedCommand::classify(cmd);
        assert_eq!(res.archetype, ToolArchetype::BuildTestVerify);
        assert_eq!(res.binary, "cargo");

        let cargo_test = "cargo +nightly test --workspace";
        let res2 = ClassifiedCommand::classify(cargo_test);
        assert_eq!(res2.archetype, ToolArchetype::BuildTestVerify);

        let npm_test = "npm test";
        let res3 = ClassifiedCommand::classify(npm_test);
        assert_eq!(res3.archetype, ToolArchetype::BuildTestVerify);
    }

    #[test]
    fn test_classify_vcs_lifecycle() {
        let commit = "git commit -m \"feat: new capability\"";
        let res = ClassifiedCommand::classify(commit);
        assert_eq!(res.archetype, ToolArchetype::VcsLifecycle);

        let push = "git push origin main";
        let res2 = ClassifiedCommand::classify(push);
        assert_eq!(res2.archetype, ToolArchetype::VcsLifecycle);

        let gh_pr = "gh pr create --title \"fix\"";
        let res3 = ClassifiedCommand::classify(gh_pr);
        assert_eq!(res3.archetype, ToolArchetype::VcsLifecycle);
    }

    #[test]
    fn test_classify_env_pkg_manager() {
        let npm_inst = "npm --prefix client install express";
        let res = ClassifiedCommand::classify(npm_inst);
        assert_eq!(res.archetype, ToolArchetype::EnvPkgManager);

        let cargo_add = "cargo add serde --features derive";
        let res2 = ClassifiedCommand::classify(cargo_add);
        assert_eq!(res2.archetype, ToolArchetype::EnvPkgManager);

        let uv_add = "uv add fastapi";
        let res3 = ClassifiedCommand::classify(uv_add);
        assert_eq!(res3.archetype, ToolArchetype::EnvPkgManager);

        let py_pip = "python -m pip install pydantic";
        let res4 = ClassifiedCommand::classify(py_pip);
        assert_eq!(res4.archetype, ToolArchetype::EnvPkgManager);
    }

    #[test]
    fn test_classify_state_mutation() {
        let rm = "rm -rf target/release";
        let res = ClassifiedCommand::classify(rm);
        assert_eq!(res.archetype, ToolArchetype::StateMutation);

        let mkdir = "mkdir -p src/commands";
        let res2 = ClassifiedCommand::classify(mkdir);
        assert_eq!(res2.archetype, ToolArchetype::StateMutation);

        let sed_inplace = "sed -i 's/foo/bar/g' config.toml";
        let res3 = ClassifiedCommand::classify(sed_inplace);
        assert_eq!(res3.archetype, ToolArchetype::StateMutation);
    }

    #[test]
    fn test_classify_network_transfer() {
        let curl = "curl -sSL https://get.pnpm.io/install.sh";
        let res = ClassifiedCommand::classify(curl);
        assert_eq!(res.archetype, ToolArchetype::NetworkTransfer);

        let wget = "wget -q https://example.com/data.json";
        let res2 = ClassifiedCommand::classify(wget);
        assert_eq!(res2.archetype, ToolArchetype::NetworkTransfer);
    }

    #[test]
    fn test_pipeline_quoting_and_logical_or() {
        // Pipes inside double quotes should not split
        let cmd1 = "grep \"foo|bar\" file.txt";
        let res1 = ClassifiedCommand::classify(cmd1);
        assert_eq!(res1.pipeline_depth, 1);

        // Logical OR `||` should not increase pipeline depth
        let cmd2 = "cargo test || echo 'failed'";
        let res2 = ClassifiedCommand::classify(cmd2);
        assert_eq!(res2.pipeline_depth, 1);

        // Real pipeline
        let cmd3 = "cat out.log | grep error | head -n 10";
        let res3 = ClassifiedCommand::classify(cmd3);
        assert_eq!(res3.pipeline_depth, 3);
    }
}
