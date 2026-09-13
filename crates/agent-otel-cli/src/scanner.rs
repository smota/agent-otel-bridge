/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hooks::{resolve_canonical_hook_binary, HookScope, CLIENT_ADAPTERS};

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub roots: Vec<PathBuf>,
    pub all_drives: bool,
    pub max_depth: usize,
    pub dry_run: bool,
    pub binary: Option<String>,
}

#[derive(Debug, Default)]
pub struct ScanStats {
    pub directories_scanned: usize,
    pub projects_found: usize,
    pub projects_synced: usize,
    pub projects_already_aligned: usize,
    pub errors: usize,
}

/// Discovers default developer root paths when none are explicitly provided.
pub fn discover_default_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        let home_path = PathBuf::from(home);
        let candidates = [
            "code",
            "projects",
            "dev",
            "repos",
            "source",
            "workspace",
            "workspaces",
            "src",
        ];
        for candidate in candidates {
            let p = home_path.join(candidate);
            if p.is_dir() {
                roots.push(p);
            }
        }
    }

    // Common root drive paths on Windows
    #[cfg(windows)]
    {
        for letter in *b"CDE" {
            let path = PathBuf::from(format!("{}:\\code", letter as char));
            if path.is_dir() && !roots.contains(&path) {
                roots.push(path);
            }
        }
    }

    if let Ok(current) = std::env::current_dir() {
        if !roots.iter().any(|r| current.starts_with(r)) {
            roots.push(current);
        }
    }

    roots
}

/// Discovers available disk roots when --all-drives is requested.
pub fn discover_all_drive_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut drives = Vec::new();
        for letter in b'C'..=b'Z' {
            let path = PathBuf::from(format!("{}:\\", letter as char));
            if path.exists() {
                drives.push(path);
            }
        }
        drives
    }
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/")]
    }
}

/// Checks if a directory should be completely skipped during traversal to ensure high performance and safety.
pub fn is_pruned_dir_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        ".git"
            | "node_modules"
            | "target"
            | "vendor"
            | ".cargo"
            | ".rustup"
            | ".vscode"
            | ".idea"
            | "appdata"
            | "application data"
            | "windows"
            | "program files"
            | "program files (x86)"
            | "$recycle.bin"
            | "system volume information"
            | "local settings"
            | ".gradle"
            | ".m2"
            | "dist"
            | "build"
            | ".next"
            | ".turbo"
            | ".cache"
            | ".nuget"
    )
}

/// Determines if a directory represents a project root.
pub fn is_project_root(path: &Path) -> bool {
    // 1. Recognized standard markers of software repositories
    if path.join(".git").exists()
        || path.join("Cargo.toml").is_file()
        || path.join("package.json").is_file()
        || path.join("pyproject.toml").is_file()
        || path.join("go.mod").is_file()
        || path.join("pom.xml").is_file()
        || path.join("build.gradle").is_file()
    {
        return true;
    }

    // 2. Dynamically check workspace markers registered across all platform adapters
    CLIENT_ADAPTERS
        .iter()
        .flat_map(|a| a.workspace_markers)
        .any(|&marker| path.join(marker).exists())
}

/// Recursively scans candidate roots for projects, respecting max_depth and pruned directories.
pub fn scan_for_projects(
    roots: &[PathBuf],
    max_depth: usize,
    stats: &mut ScanStats,
) -> Vec<PathBuf> {
    let mut discovered = Vec::new();
    let mut visited = HashSet::new();

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        if visited.insert(canonical.clone()) {
            walk_dir(
                &canonical,
                0,
                max_depth,
                stats,
                &mut discovered,
                &mut visited,
            );
        }
    }

    discovered
}

fn walk_dir(
    current: &Path,
    current_depth: usize,
    max_depth: usize,
    stats: &mut ScanStats,
    discovered: &mut Vec<PathBuf>,
    visited: &mut HashSet<PathBuf>,
) {
    stats.directories_scanned += 1;

    let is_proj = is_project_root(current);
    if is_proj && !discovered.contains(&current.to_path_buf()) {
        discovered.push(current.to_path_buf());
    }

    if current_depth >= max_depth {
        return;
    }

    let entries = match fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let file_name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };

        if is_pruned_dir_name(&file_name) {
            continue;
        }

        // Avoid symlink cycles
        let canonical = match path.canonicalize() {
            Ok(c) => c,
            Err(_) => path.clone(),
        };

        if visited.insert(canonical.clone()) {
            walk_dir(
                &canonical,
                current_depth + 1,
                max_depth,
                stats,
                discovered,
                visited,
            );
        }
    }
}

/// Executes scan and synchronization across all discovered projects.
pub fn run_scan_all(opts: ScanOptions) -> Result<(), Box<dyn std::error::Error>> {
    let resolved_binary = resolve_canonical_hook_binary(opts.binary.as_deref())?;
    let binary = &resolved_binary;

    let roots = if opts.all_drives {
        discover_all_drive_roots()
    } else if !opts.roots.is_empty() {
        opts.roots.clone()
    } else {
        discover_default_roots()
    };

    println!("\n=== agent-otel-bridge hooks scan-all ===");
    println!("Target binary: {}", binary);
    println!("Max scan depth: {}", opts.max_depth);
    if opts.dry_run {
        println!("Mode: DRY-RUN (no files will be modified)");
    }
    println!("Scan root(s):");
    for r in &roots {
        println!("  - {}", r.display());
    }
    println!();

    let mut stats = ScanStats::default();

    print!("Discovering projects across workstation...");
    let projects = scan_for_projects(&roots, opts.max_depth, &mut stats);
    println!(" found {} project(s).\n", projects.len());
    stats.projects_found = projects.len();

    if projects.is_empty() {
        println!("No projects found in specified search roots.\n");
        return Ok(());
    }

    for (idx, project) in projects.iter().enumerate() {
        let proj_display = project.display().to_string();
        println!("[{}/{}] Project: {}", idx + 1, projects.len(), proj_display);

        let mut project_updated = false;

        for adapter in CLIENT_ADAPTERS {
            let proj_cfg = (adapter.project_config_fn)(Some(project));
            let Some(cfg_path) = proj_cfg else {
                continue;
            };

            match adapter.scope {
                HookScope::GlobalOnly => {
                    // Purely global, no action needed in projects
                }
                HookScope::NamespaceMerged => {
                    if cfg_path.exists() {
                        if (adapter.is_registered_fn)(&cfg_path) {
                            println!(
                                "  {:<20} [ok] Up-to-date in project ({})",
                                adapter.display_name,
                                cfg_path.display()
                            );
                        } else {
                            if opts.dry_run {
                                println!(
                                    "  {:<20} [dry-run] Would sync into project ({})",
                                    adapter.display_name,
                                    cfg_path.display()
                                );
                            } else {
                                match (adapter.install_fn)(&cfg_path, binary, adapter.client_tag) {
                                    Ok(_) => {
                                        println!(
                                            "  {:<20} [synced] Injected into project ({})",
                                            adapter.display_name,
                                            cfg_path.display()
                                        );
                                        project_updated = true;
                                    }
                                    Err(e) => {
                                        println!(
                                            "  {:<20} [error] Failed to sync ({}): {}",
                                            adapter.display_name,
                                            cfg_path.display(),
                                            e
                                        );
                                        stats.errors += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                HookScope::ProjectShadowsGlobal => {
                    if cfg_path.exists() {
                        if (adapter.is_registered_fn)(&cfg_path) {
                            println!(
                                "  {:<20} [ok] Up-to-date in project ({})",
                                adapter.display_name,
                                cfg_path.display()
                            );
                        } else {
                            if opts.dry_run {
                                println!(
                                    "  {:<20} [dry-run] Local config shadows global! Would sync ({})",
                                    adapter.display_name,
                                    cfg_path.display()
                                );
                            } else {
                                println!(
                                    "  {:<20} [shadow] Local config shadows global! Syncing...",
                                    adapter.display_name
                                );
                                match (adapter.install_fn)(&cfg_path, binary, adapter.client_tag) {
                                    Ok(_) => {
                                        println!(
                                            "  {:<20} [synced] Updated project configuration ({})",
                                            adapter.display_name,
                                            cfg_path.display()
                                        );
                                        project_updated = true;
                                    }
                                    Err(e) => {
                                        println!(
                                            "  {:<20} [error] Failed to sync ({}): {}",
                                            adapter.display_name,
                                            cfg_path.display(),
                                            e
                                        );
                                        stats.errors += 1;
                                    }
                                }
                            }
                        }
                    } else {
                        // No local config file, global config applies cleanly
                        println!(
                            "  {:<20} [clean] No local override (global hooks active)",
                            adapter.display_name
                        );
                    }
                }
            }
        }

        if project_updated {
            stats.projects_synced += 1;
        } else {
            stats.projects_already_aligned += 1;
        }
        println!();
    }

    println!("=== Scan & Sync Summary ===");
    println!("Directories scanned:        {}", stats.directories_scanned);
    println!("Projects found:             {}", stats.projects_found);
    if opts.dry_run {
        println!("Mode:                       DRY-RUN");
    } else {
        println!("Projects updated:           {}", stats.projects_synced);
        println!(
            "Projects already aligned:   {}",
            stats.projects_already_aligned
        );
        if stats.errors > 0 {
            println!("Errors encountered:         {}", stats.errors);
        }
    }
    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_pruned_dir_name() {
        assert!(is_pruned_dir_name(".git"));
        assert!(is_pruned_dir_name("node_modules"));
        assert!(is_pruned_dir_name("target"));
        assert!(is_pruned_dir_name("AppData"));
        assert!(is_pruned_dir_name("$Recycle.Bin"));
        assert!(!is_pruned_dir_name("src"));
        assert!(!is_pruned_dir_name("crates"));
        assert!(!is_pruned_dir_name("agentflow-sdlc"));
    }

    #[test]
    fn test_is_project_root_detection() {
        let temp_dir = std::env::temp_dir().join(format!("scan_test_proj_{}", std::process::id()));
        fs::create_dir_all(&temp_dir).unwrap();

        assert!(!is_project_root(&temp_dir));

        let cargo_toml = temp_dir.join("Cargo.toml");
        fs::write(&cargo_toml, "[package]\nname = \"foo\"").unwrap();
        assert!(is_project_root(&temp_dir));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_scan_for_projects_with_pruning() {
        let temp_dir = std::env::temp_dir().join(format!("scan_hierarchy_{}", std::process::id()));
        let p1 = temp_dir.join("workspace_a");
        let p2 = temp_dir.join("workspace_b");
        let pruned = temp_dir.join("node_modules").join("nested_lib");

        fs::create_dir_all(&p1).unwrap();
        fs::create_dir_all(&p2).unwrap();
        fs::create_dir_all(&pruned).unwrap();

        fs::write(p1.join("package.json"), "{}").unwrap();
        fs::write(p2.join("Cargo.toml"), "[package]").unwrap();
        fs::write(pruned.join("package.json"), "{}").unwrap();

        let mut stats = ScanStats::default();
        let found = scan_for_projects(std::slice::from_ref(&temp_dir), 4, &mut stats);

        assert_eq!(found.len(), 2);
        let found_strs: Vec<String> = found.iter().map(|p| p.display().to_string()).collect();
        assert!(found_strs.iter().any(|s| s.contains("workspace_a")));
        assert!(found_strs.iter().any(|s| s.contains("workspace_b")));
        assert!(!found_strs.iter().any(|s| s.contains("nested_lib")));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
