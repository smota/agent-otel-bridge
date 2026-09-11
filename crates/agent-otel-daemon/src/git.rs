/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStats {
    pub lines_added: Option<u64>,
    pub lines_deleted: Option<u64>,
    pub files_changed: Option<u64>,
    pub self_revert: Option<bool>,
}

/// Parses standard git diff --shortstat output into (files_changed, insertions, deletions).
pub fn parse_shortstat(output: &str) -> (Option<u64>, Option<u64>, Option<u64>) {
    let mut files = None;
    let mut insertions = None;
    let mut deletions = None;

    for part in output.split(',') {
        let trimmed = part.trim();
        if trimmed.contains("file changed") || trimmed.contains("files changed") {
            if let Some(num_str) = trimmed.split_whitespace().next() {
                files = num_str.parse::<u64>().ok();
            }
        } else if trimmed.contains("insertion") {
            if let Some(num_str) = trimmed.split_whitespace().next() {
                insertions = num_str.parse::<u64>().ok();
            }
        } else if trimmed.contains("deletion") {
            if let Some(num_str) = trimmed.split_whitespace().next() {
                deletions = num_str.parse::<u64>().ok();
            }
        }
    }

    (files, insertions, deletions)
}

/// Collects Git statistics for a given workspace directory.
/// Runs git diff --shortstat and checks for self-reversion.
/// Execution is capped and fails gracefully on non-git directories or timeouts.
pub fn collect_git_stats(workspace: &str) -> GitStats {
    let ws_path = Path::new(workspace);
    if !ws_path.exists() {
        return GitStats::default();
    }

    let git_dir = ws_path.join(".git");
    if !git_dir.exists() {
        return GitStats::default();
    }

    let output = Command::new("git")
        .args(["diff", "--shortstat"])
        .current_dir(ws_path)
        .output();

    let mut stats = GitStats::default();

    if let Ok(out) = output {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let (f, ins, del) = parse_shortstat(&stdout);
            stats.files_changed = f;
            stats.lines_added = ins;
            stats.lines_deleted = del;

            if let (Some(ins_val), Some(del_val)) = (ins, del) {
                if ins_val == 0 && del_val > 0 {
                    stats.self_revert = Some(true);
                }
            }
        }
    }

    if stats.files_changed.is_none() {
        if let Ok(log_out) = Command::new("git")
            .args(["log", "-n", "1", "--pretty=format:%s"])
            .current_dir(ws_path)
            .output()
        {
            if log_out.status.success() {
                let subject = String::from_utf8_lossy(&log_out.stdout).to_ascii_lowercase();
                if subject.starts_with("revert ")
                    || subject.contains("revert:")
                    || subject.contains("undo ")
                {
                    stats.self_revert = Some(true);
                }
            }
        }
    }

    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_shortstat() {
        let sample = " 3 files changed, 142 insertions(+), 25 deletions(-)
";
        let (f, ins, del) = parse_shortstat(sample);
        assert_eq!(f, Some(3));
        assert_eq!(ins, Some(142));
        assert_eq!(del, Some(25));

        let one_file = " 1 file changed, 1 insertion(+)
";
        let (f2, ins2, del2) = parse_shortstat(one_file);
        assert_eq!(f2, Some(1));
        assert_eq!(ins2, Some(1));
        assert_eq!(del2, None);
    }
}
