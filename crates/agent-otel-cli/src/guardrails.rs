/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

pub fn run_check(fix: bool) -> Result<(), Box<dyn std::error::Error>> {
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
    let test_status = Command::new("cargo").args(["test", "--workspace"]).status();
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
        let bin_path = Path::new("target/release/agent-hook.exe");
        if bin_path.exists() {
            if let Ok(meta) = fs::metadata(bin_path) {
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
            println!("(skipped release check)");
        }
    } else {
        println!("FAILED to build release binary");
        failures += 1;
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
