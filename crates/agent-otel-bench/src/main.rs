/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};

mod metadata;
mod parse_bench;
mod pipe_bench;
mod report;
mod spawn_bench;
mod stats;
mod submit;

#[derive(Parser, Debug)]
#[command(
    name = "agent-otel-bench",
    about = "High-Precision Benchmark Suite & Hardware Submitter for agent-otel-bridge",
    version
)]
pub struct BenchArgs {
    /// Iterations for Win32 Named Pipe RTT benchmark
    #[arg(long, default_value_t = 1000)]
    pub iterations_pipe: usize,

    /// Iterations for ProtoJSON / OTLP Protobuf parse & build benchmark
    #[arg(long, default_value_t = 10000)]
    pub iterations_parse: usize,

    /// Iterations for real process lifecycle spawn benchmark
    #[arg(long, default_value_t = 250)]
    pub iterations_spawn: usize,

    /// Output full benchmark report as JSON
    #[arg(long)]
    pub json: bool,

    /// Output benchmark report as Markdown
    #[arg(long)]
    pub markdown: bool,

    /// Export benchmark report to file (format inferred from .json or .md)
    #[arg(long)]
    pub export: Option<PathBuf>,

    /// Interactively submit benchmark results to community repository
    #[arg(long)]
    pub submit: bool,

    /// Automatically open web browser when submitting benchmark
    #[arg(long)]
    pub open_browser: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = BenchArgs::parse();

    if !args.json {
        println!("============================================================");
        println!("  agent-otel-bridge Performance Benchmark Suite");
        println!("============================================================\n");
    }

    // Collect system metadata
    let meta = metadata::SystemMetadata::collect();

    if !args.json {
        println!("System Environment:");
        println!(
            "  OS:   {} {} (Build {}, {})",
            meta.os_name, meta.os_version, meta.os_build, meta.os_arch
        );
        println!("  CPU:  {} ({} cores)", meta.cpu_brand, meta.cpu_cores);
        println!("  RAM:  {:.1} GB", meta.ram_gb);
        println!("  Rust: {}\n", meta.rustc_version);
    }

    // 1. Win32 Named Pipe RTT benchmark
    if !args.json {
        println!(
            "[1/3] Benchmarking Win32 Named Pipe round-trip latency ({} iterations)...",
            args.iterations_pipe
        );
    }
    let pipe_stats = pipe_bench::run(args.iterations_pipe).await?;
    if !args.json {
        println!(
            "  -> p50: {} µs, p95: {} µs, p99: {} µs",
            pipe_stats.p50(),
            pipe_stats.p95(),
            pipe_stats.p99()
        );
    }

    // 2. ProtoJSON parsing & OTLP span builder throughput
    if !args.json {
        println!("\n[2/3] Benchmarking ProtoJSON parsing & OTLP Protobuf span construction ({} iterations)...", args.iterations_parse);
    }
    let parse_result = parse_bench::run(args.iterations_parse);
    if !args.json {
        println!(
            "  -> p50: {} µs, p95: {} µs, p99: {} µs, throughput: {:.0} spans/sec",
            parse_result.stats.p50(),
            parse_result.stats.p95(),
            parse_result.stats.p99(),
            parse_result.spans_per_sec
        );
    }

    // 3. Real Windows process spawn benchmark
    if !args.json {
        println!(
            "\n[3/3] Benchmarking real Windows process lifecycle (agent-hook.exe, {} iterations)...",
            args.iterations_spawn
        );
    }
    let spawn_stats = spawn_bench::run(args.iterations_spawn, args.json)?;
    if !args.json {
        println!(
            "  -> p50: {} µs ({:.2} ms), p95: {} µs ({:.2} ms), p99: {} µs ({:.2} ms)",
            spawn_stats.p50(),
            spawn_stats.p50() as f64 / 1000.0,
            spawn_stats.p95(),
            spawn_stats.p95() as f64 / 1000.0,
            spawn_stats.p99(),
            spawn_stats.p99() as f64 / 1000.0
        );
    }

    // 4. Build report
    let report = report::BenchmarkReport {
        metadata: meta,
        pipe: pipe_stats,
        parse: parse_result,
        spawn: spawn_stats,
        sla_target_us: 3000, // 3ms SLA
    };

    // 5. Handle output formats
    if args.json {
        println!("{}", report.to_json());
        return Ok(());
    }

    let md = report.to_markdown();

    // Export to file if requested
    if let Some(export_path) = args.export {
        if export_path.extension().and_then(|e| e.to_str()) == Some("json") {
            fs::write(&export_path, report.to_json())?;
        } else {
            fs::write(&export_path, &md)?;
        }
        println!(
            "\n  [ok] Exported benchmark report to: {}",
            export_path.display()
        );
    }

    // Save default docs/BENCHMARKS.md if running locally in workspace
    let docs_dir = Path::new("docs");
    if docs_dir.exists() {
        let _ = fs::write("docs/BENCHMARKS.md", &md);
    }

    if args.markdown {
        println!("\n{}", md);
    } else {
        println!("\n============================================================");
        println!("Benchmark completed!");
        println!("============================================================\n");
        println!("{}", md);
    }

    // 6. Handle submission workflow if requested
    if args.submit {
        submit::handle_submit(&report, args.open_browser)?;
    }

    Ok(())
}
