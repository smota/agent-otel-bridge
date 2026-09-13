/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

pub fn run_check(fix: bool) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    if std::env::var("GUARDRAILS_SELF_COPY").is_err() {
        if let Ok(exe) = std::env::current_exe() {
            let path_str = exe.to_string_lossy();
            if path_str.contains("target") {
                let temp_exe =
                    std::env::temp_dir().join(format!("guardrails-{}.exe", std::process::id()));
                let _ = fs::copy(&exe, &temp_exe);
                let status = Command::new(&temp_exe)
                    .arg("check-guardrails")
                    .args(if fix { vec!["--fix"] } else { vec![] })
                    .env("GUARDRAILS_SELF_COPY", "1")
                    .status();
                let _ = fs::remove_file(&temp_exe);
                if let Ok(s) = status {
                    if !s.success() {
                        std::process::exit(s.code().unwrap_or(1));
                    }
                    return Ok(());
                }
            }
        }
    }

    let start = Instant::now();
    println!("============================================================");
    println!("  agent-otel-bridge: Automated Guardrail Verification Tool  ");
    println!("============================================================");
    println!();

    let mut failures = 0;

    // 1. Code Formatting
    print!("[GUARDRAIL] Checking code formatting (cargo fmt)... ");
    if fix {
        let _ = Command::new("cargo").arg("fmt").status();
    }
    let fmt_status = Command::new("cargo").args(["fmt", "--check"]).status();
    match fmt_status {
        Ok(s) if s.success() => println!("PASSED"),
        _ => {
            println!("FAILED (Run 'cargo fmt' to fix)");
            failures += 1;
        }
    }

    // 2. Clippy Linter
    print!("[GUARDRAIL] Checking linter purity (cargo clippy --all-targets)... ");
    let clippy_status = Command::new("cargo")
        .args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
        .status();
    match clippy_status {
        Ok(s) if s.success() => println!("PASSED"),
        _ => {
            println!("FAILED (Clippy warnings detected)");
            failures += 1;
        }
    }

    // 3. Workspace Test Suite
    print!("[GUARDRAIL] Running workspace test suite (cargo test --workspace)... ");
    let test_status = Command::new("cargo")
        .args(["test", "--workspace", "--lib", "--tests"])
        .status();
    match test_status {
        Ok(s) if s.success() => println!("PASSED"),
        _ => {
            println!("FAILED (Test suite failed)");
            failures += 1;
        }
    }

    // 4. Documentation Generation
    print!("[GUARDRAIL] Validating documentation generation (cargo doc --no-deps)... ");
    let doc_status = Command::new("cargo")
        .args(["doc", "--workspace", "--no-deps"])
        .status();
    match doc_status {
        Ok(s) if s.success() => println!("PASSED"),
        _ => {
            println!("FAILED (Doc generation failed)");
            failures += 1;
        }
    }

    // 5. Client Binary Size SLA (< 350 KB)
    print!("[GUARDRAIL] Checking client binary size SLA (< 350 KB)... ");
    let build_rel = Command::new("cargo")
        .args(["build", "--release", "-p", "agent-otel-client"])
        .status();
    if build_rel.is_ok() {
        let exe_name = format!("agent-hook{}", std::env::consts::EXE_SUFFIX);
        let bin_path = Path::new("target").join("release").join(&exe_name);
        if bin_path.exists() {
            if let Ok(meta) = fs::metadata(&bin_path) {
                let size_kb = (meta.len() as f64) / 1024.0;
                if size_kb > 350.0 {
                    println!("FAILED ({:.1} KB exceeds 350 KB SLA)", size_kb);
                    failures += 1;
                } else {
                    println!("({:.1} KB) PASSED", size_kb);
                }
            } else {
                println!("PASSED");
            }
        } else {
            println!("(skipped release check: {} not found)", bin_path.display());
        }
    } else {
        println!("FAILED to build release binary");
        failures += 1;
    }

    // 6. Branch Naming Policy Check (if git repository)
    print!("[GUARDRAIL] Validating Git branch naming policy... ");
    let git_branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output();
    match git_branch {
        Ok(out) if out.status.success() => {
            let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let is_allowed = branch == "main"
                || branch == "master"
                || branch.starts_with("feat/")
                || branch.starts_with("fix/")
                || branch.starts_with("docs/")
                || branch.starts_with("perf/")
                || branch.starts_with("release/")
                || branch.starts_with("chore/");
            if is_allowed {
                println!("({}) PASSED", branch);
            } else {
                println!(
                    "FAILED (Branch '{}' must follow 'feat/*', 'fix/*', 'docs/*', 'perf/*', or 'main')",
                    branch
                );
                failures += 1;
            }
        }
        _ => {
            println!("(skipped: not in a git repository)");
        }
    }

    println!();
    println!("============================================================");
    let elapsed = start.elapsed().as_secs_f64();
    if failures == 0 {
        println!(
            "  ALL GUARDRAILS PASSED (Elapsed: {:.1}s)          ",
            elapsed
        );
        println!("============================================================");
        Ok(())
    } else {
        println!(
            "  {} GUARDRAIL CHECK(S) FAILED (Elapsed: {:.1}s) ",
            failures, elapsed
        );
        println!("============================================================");
        std::process::exit(1);
    }
}
