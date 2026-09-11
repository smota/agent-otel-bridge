/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::report::BenchmarkReport;
use std::io::Write;

pub fn handle_submit(
    report: &BenchmarkReport,
    open_browser: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n============================================================");
    println!("  agent-otel-bridge — Benchmark Submission Helper");
    println!("============================================================\n");

    let json_str = report.to_json();
    let md_str = report.to_markdown();

    let out_dir = std::path::Path::new("benchmarks").join("reports");
    let _ = std::fs::create_dir_all(&out_dir);
    let cpu_sanitized = report
        .metadata
        .cpu_brand
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let filename = format!("benchmark-{}-{}.json", cpu_sanitized, timestamp);
    let report_path = out_dir.join(&filename);
    let _ = std::fs::write(&report_path, &json_str);
    println!(
        "  [ok] Local benchmark report saved to: {}",
        report_path.display()
    );

    let issue_title = format!(
        "Benchmark: {} on {}",
        report.metadata.cpu_brand, report.metadata.os_name
    );

    let gh_available = check_gh_cli();

    if gh_available {
        println!("\n  GitHub CLI (`gh`) detected on your system!");
        print!("  Would you like to submit this benchmark as a GitHub issue directly? [y/N]: ");
        let _ = std::io::stdout().flush();
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() && input.trim().eq_ignore_ascii_case("y")
        {
            println!("  Submitting issue via `gh issue create`...");
            let res = std::process::Command::new("gh")
                .args([
                    "issue",
                    "create",
                    "--repo",
                    "smota/agent-otel-bridge",
                    "--title",
                    &issue_title,
                    "--body",
                    &md_str,
                    "--label",
                    "benchmark",
                ])
                .output();

            match res {
                Ok(out) if out.status.success() => {
                    let issue_url = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    println!("\n  🎉 Success! Benchmark issue created: {}", issue_url);
                    println!("     Our GitHub Actions bot will validate and verify your benchmark shortly.");
                    return Ok(());
                }
                Ok(out) => {
                    println!(
                        "  [warn] `gh issue create` exited with code {}: {}",
                        out.status,
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                Err(e) => println!("  [warn] Failed to execute `gh`: {}", e),
            }
        }
    }

    let web_url = build_github_issue_url(&report.metadata.cpu_brand, &report.metadata.os_name);

    println!("\n  To submit via the GitHub Web Issue Form:");
    println!("  1. Open the pre-filled issue link below in your browser:");
    println!("\n  {}\n", web_url);
    println!(
        "  2. Paste your JSON report from: {}",
        report_path.display()
    );
    println!("  3. Click 'Submit new issue'.");
    println!("  4. Our automated GitHub Action bot will validate and record your benchmark in COMMUNITY_BENCHMARKS.md!");

    if open_browser {
        println!("\n  Opening browser automatically...");
        open_in_browser(&web_url);
    }

    Ok(())
}

fn check_gh_cli() -> bool {
    std::process::Command::new("gh")
        .arg("auth")
        .arg("status")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn build_github_issue_url(cpu: &str, os: &str) -> String {
    let title = format!("Benchmark: {} on {}", cpu, os);
    let base = "https://github.com/smota/agent-otel-bridge/issues/new";
    let template = "benchmark_submission.yml";

    format!(
        "{}?template={}&title={}&cpu={}&os={}",
        base,
        template,
        urlencoding(title.as_bytes()),
        urlencoding(cpu.as_bytes()),
        urlencoding(os.as_bytes()),
    )
}

fn urlencoding(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn open_in_browser(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}
