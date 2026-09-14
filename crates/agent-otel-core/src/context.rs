/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DEFAULT_CONTEXT_MAX_ANCESTORS: usize = 15;
pub const DEFAULT_CONTEXT_MAX_FILE_BYTES: usize = 64 * 1024;
pub const DEFAULT_CONTEXT_BUDGET: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarvestLimits {
    pub max_ancestors: usize,
    pub max_file_bytes: usize,
    pub budget: Duration,
}

impl Default for HarvestLimits {
    fn default() -> Self {
        Self {
            max_ancestors: DEFAULT_CONTEXT_MAX_ANCESTORS,
            max_file_bytes: DEFAULT_CONTEXT_MAX_FILE_BYTES,
            budget: DEFAULT_CONTEXT_BUDGET,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextHarvest {
    pub context: WorkspaceContext,
    /// False means the cooperative budget elapsed between filesystem calls.
    /// An individual blocked syscall cannot be cancelled by this contract.
    pub completed: bool,
}

struct HarvestBudget {
    deadline: Instant,
    exhausted: Cell<bool>,
}

impl HarvestBudget {
    fn new(duration: Duration) -> Self {
        Self {
            deadline: Instant::now() + duration,
            exhausted: Cell::new(false),
        }
    }

    fn available(&self) -> bool {
        if Instant::now() >= self.deadline {
            self.exhausted.set(true);
            false
        } else {
            true
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceContext {
    pub current_dir: String,
    pub launch_dir: Option<String>,
    pub project_name: Option<String>,
    pub project_root: Option<String>,
    pub project_type: Option<String>,
    pub vcs_system: Option<String>,
    pub vcs_repository: Option<String>,
    pub vcs_branch: Option<String>,
    pub vcs_commit: Option<String>,
    pub vcs_worktree: Option<bool>,
}

impl WorkspaceContext {
    /// Harvests workspace and project context starting from a directory.
    /// Fast and non-blocking: uses direct stat/file reads with zero external subprocess calls.
    pub fn harvest_from_dir(dir: &Path) -> Self {
        let mut context = Self::harvest_from_dir_with_limits(dir, HarvestLimits::default()).context;
        context.launch_dir = std::env::var("INIT_CWD").ok().or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().to_string())
        });
        context
    }

    /// Bounded context probe used by daemon refresh workers. The deadline is
    /// cooperative: it is checked between direct filesystem operations.
    pub fn harvest_from_dir_with_limits(dir: &Path, limits: HarvestLimits) -> ContextHarvest {
        let budget = HarvestBudget::new(limits.budget);
        let current_dir = dir
            .canonicalize()
            .unwrap_or_else(|_| dir.to_path_buf())
            .to_string_lossy()
            .to_string();

        let mut ctx = Self {
            current_dir,
            ..Default::default()
        };

        // Ascend parent directories to detect project markers
        if let Some((root, ptype, name)) = find_project_root(dir, limits, &budget) {
            ctx.project_root = Some(root.to_string_lossy().to_string());
            ctx.project_type = Some(ptype);
            ctx.project_name = name;
        } else {
            // Fallback project name to the directory basename
            if let Some(file_name) = dir.file_name() {
                ctx.project_name = Some(file_name.to_string_lossy().to_string());
            }
        }

        // Detect VCS (Git) hints starting from dir and ascending
        if let Some(vcs) = probe_git_metadata(dir, limits, &budget) {
            ctx.vcs_system = Some("git".to_string());
            ctx.vcs_repository = vcs.repository;
            ctx.vcs_branch = vcs.branch;
            ctx.vcs_commit = vcs.commit;
            ctx.vcs_worktree = Some(vcs.is_worktree);
        } else {
            ctx.vcs_system = Some("none".to_string());
        }

        ContextHarvest {
            context: ctx,
            completed: !budget.exhausted.get(),
        }
    }

    /// Harvests context from current process working directory.
    pub fn harvest_current() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::harvest_from_dir(&cwd)
    }
}

/// Project markers with their corresponding ecosystem types
const PROJECT_MARKERS: &[(&str, &str)] = &[
    ("Cargo.toml", "rust"),
    ("package.json", "node"),
    ("pyproject.toml", "python"),
    ("requirements.txt", "python"),
    ("setup.py", "python"),
    ("go.mod", "go"),
    ("pom.xml", "java"),
    ("build.gradle", "java"),
    ("CMakeLists.txt", "cpp"),
    (".gemini", "antigravity_workspace"),
    (".claude", "claude_workspace"),
    (".codex", "codex_workspace"),
];

fn find_project_root(
    start_dir: &Path,
    limits: HarvestLimits,
    budget: &HarvestBudget,
) -> Option<(PathBuf, String, Option<String>)> {
    let mut curr = start_dir.to_path_buf();
    for _ in 0..limits.max_ancestors {
        if !budget.available() {
            return None;
        }
        for &(marker, ptype) in PROJECT_MARKERS {
            if !budget.available() {
                return None;
            }
            let marker_path = curr.join(marker);
            if marker_path.exists() {
                let name = extract_project_name(&curr, marker, limits.max_file_bytes, budget);
                return Some((curr, ptype.to_string(), name));
            }
        }
        if !curr.pop() {
            break;
        }
    }
    None
}

fn extract_project_name(
    root: &Path,
    marker: &str,
    max_file_bytes: usize,
    budget: &HarvestBudget,
) -> Option<String> {
    if marker == "Cargo.toml" {
        if let Some(content) = read_text_limited(&root.join("Cargo.toml"), max_file_bytes, budget) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with("name =") || line.starts_with("name=") {
                    let parts: Vec<&str> = line.split('=').collect();
                    if parts.len() == 2 {
                        let name = parts[1].trim().trim_matches('"').trim_matches('\'');
                        if !name.is_empty() {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
    } else if marker == "package.json" {
        if let Some(content) = read_text_limited(&root.join("package.json"), max_file_bytes, budget)
        {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with("\"name\":") || line.starts_with("\"name\" :") {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 2 {
                        let name = parts[1].trim().trim_matches(',').trim().trim_matches('"');
                        if !name.is_empty() {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
    }
    root.file_name().map(|n| n.to_string_lossy().to_string())
}

#[derive(Default)]
struct GitProbeResult {
    repository: Option<String>,
    branch: Option<String>,
    commit: Option<String>,
    is_worktree: bool,
}

fn probe_git_metadata(
    start_dir: &Path,
    limits: HarvestLimits,
    budget: &HarvestBudget,
) -> Option<GitProbeResult> {
    let mut curr = start_dir.to_path_buf();
    for _ in 0..limits.max_ancestors {
        if !budget.available() {
            return None;
        }
        let git_marker = curr.join(".git");
        if git_marker.is_dir() {
            return Some(read_git_dir(&git_marker, false, limits, budget));
        } else if git_marker.is_file() {
            // Git worktree pointer: format is "gitdir: /path/to/.git/worktrees/<name>"
            if let Some(content) = read_text_limited(&git_marker, limits.max_file_bytes, budget) {
                let line = content.trim();
                if let Some(target) = line.strip_prefix("gitdir:") {
                    let target_path = target.trim();
                    let resolved = if Path::new(target_path).is_absolute() {
                        PathBuf::from(target_path)
                    } else {
                        curr.join(target_path)
                    };
                    if resolved.exists() {
                        return Some(read_git_dir(&resolved, true, limits, budget));
                    }
                }
            }
            return Some(read_git_dir(&git_marker, true, limits, budget));
        }
        if !curr.pop() {
            break;
        }
    }
    None
}

fn read_git_dir(
    git_dir: &Path,
    is_worktree: bool,
    limits: HarvestLimits,
    budget: &HarvestBudget,
) -> GitProbeResult {
    let mut result = GitProbeResult {
        is_worktree,
        ..Default::default()
    };

    let common_git_dir = if is_worktree {
        read_text_limited(&git_dir.join("commondir"), limits.max_file_bytes, budget)
            .map(|commondir| git_dir.join(commondir.trim()))
            .unwrap_or_else(|| git_dir.to_path_buf())
    } else {
        git_dir.to_path_buf()
    };

    // 1. Read HEAD
    let head_path = git_dir.join("HEAD");
    if let Some(head_content) = read_text_limited(&head_path, limits.max_file_bytes, budget) {
        let head_line = head_content.trim();
        if let Some(branch_ref) = head_line.strip_prefix("ref: refs/heads/") {
            let branch = branch_ref.trim().to_string();
            result.branch = Some(branch.clone());

            // Try reading commit from refs/heads/<branch>
            let ref_path = common_git_dir.join("refs").join("heads").join(&branch);
            if let Some(sha) = read_text_limited(&ref_path, limits.max_file_bytes, budget) {
                let sha = sha.trim();
                if sha.len() >= 7 {
                    result.commit = Some(sha.to_string());
                }
            } else if let Some(packed_refs) = read_text_limited(
                &common_git_dir.join("packed-refs"),
                limits.max_file_bytes,
                budget,
            ) {
                let wanted = format!("refs/heads/{branch}");
                result.commit = packed_refs.lines().find_map(|line| {
                    let mut parts = line.split_whitespace();
                    let sha = parts.next()?;
                    let reference = parts.next()?;
                    (reference == wanted && sha.chars().all(|c| c.is_ascii_hexdigit()))
                        .then(|| sha.to_string())
                });
            }
        } else if head_line.len() >= 7 && head_line.chars().all(|c| c.is_ascii_hexdigit()) {
            // Detached HEAD
            result.commit = Some(head_line.to_string());
            result.branch = Some("detached".to_string());
        }
    }

    // 2. Read git config for remote origin
    let config_path = common_git_dir.join("config");

    if let Some(config_content) = read_text_limited(&config_path, limits.max_file_bytes, budget) {
        let mut in_origin = false;
        for line in config_content.lines() {
            let line = line.trim();
            if line.starts_with("[remote \"origin\"]") {
                in_origin = true;
                continue;
            }
            if in_origin {
                if line.starts_with('[') {
                    break;
                }
                if line.starts_with("url =") || line.starts_with("url=") {
                    let parts: Vec<&str> = line.split('=').collect();
                    if parts.len() >= 2 {
                        let raw_url = parts[1].trim();
                        let sanitized_url = sanitize_git_url(raw_url);
                        result.repository = extract_repo_name(&sanitized_url);
                        break;
                    }
                }
            }
        }
    }

    result
}

fn read_text_limited(path: &Path, max_file_bytes: usize, budget: &HarvestBudget) -> Option<String> {
    if !budget.available() {
        return None;
    }
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > max_file_bytes as u64 || !budget.available() {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(max_file_bytes));
    file.take(max_file_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > max_file_bytes || !budget.available() {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn sanitize_git_url(raw_url: &str) -> String {
    // Strip user/tokens like https://x-access-token:ghp_xxx@github.com/...
    if let Some(at_idx) = raw_url.find('@') {
        if let Some(proto_idx) = raw_url.find("://") {
            if proto_idx < at_idx {
                let proto = &raw_url[..proto_idx + 3];
                let host_and_path = &raw_url[at_idx + 1..];
                return format!("{proto}{host_and_path}");
            }
        }
    }
    raw_url.to_string()
}

fn extract_repo_name(url: &str) -> Option<String> {
    let clean = url.trim_end_matches(".git").trim_end_matches('/');
    clean
        .split(['/', ':', '\\'])
        .next_back()
        .map(|s| s.to_string())
}

/// Harvests developer user email from git config or user profile with zero subprocess spawning (< 30 µs).
pub fn harvest_user_email() -> Option<String> {
    if let Ok(email) = std::env::var("USER_EMAIL") {
        if !email.trim().is_empty() {
            return Some(email.trim().to_string());
        }
    }
    if let Ok(email) = std::env::var("GIT_AUTHOR_EMAIL") {
        if !email.trim().is_empty() {
            return Some(email.trim().to_string());
        }
    }
    // Probe local .git/config
    if let Ok(content) = fs::read_to_string(".git/config") {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("email =") || trimmed.starts_with("email=") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    let email = val.trim();
                    if !email.is_empty() {
                        return Some(email.to_string());
                    }
                }
            }
        }
    }
    // Probe global ~/.gitconfig
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        let global_config = Path::new(&home).join(".gitconfig");
        if let Ok(content) = fs::read_to_string(global_config) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("email =") || trimmed.starts_with("email=") {
                    if let Some(val) = trimmed.split('=').nth(1) {
                        let email = val.trim();
                        if !email.is_empty() {
                            return Some(email.to_string());
                        }
                    }
                }
            }
        }
    }
    // Fallback to machine username
    if let Ok(user) = std::env::var("USERNAME").or_else(|_| std::env::var("USER")) {
        let trimmed = user.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harvest_current() {
        let ctx = WorkspaceContext::harvest_current();
        assert!(!ctx.current_dir.is_empty());
        assert_eq!(ctx.vcs_system.as_deref(), Some("git"));
        assert!(ctx.project_root.is_some());
    }

    #[test]
    fn test_sanitize_git_url() {
        let secret_url = "https://oauth2:ghp_12345SECRET@github.com/smota/agent-otel-bridge.git";
        let sanitized = sanitize_git_url(secret_url);
        assert_eq!(sanitized, "https://github.com/smota/agent-otel-bridge.git");
        assert_eq!(
            extract_repo_name(&sanitized),
            Some("agent-otel-bridge".to_string())
        );
    }

    #[test]
    fn test_harvest_user_email() {
        let email_or_user = harvest_user_email();
        assert!(email_or_user.is_some());
    }
}
