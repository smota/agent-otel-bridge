/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::fs;
use std::path::Path;

mod parse_bench;
mod pipe_bench;
mod report;
mod spawn_bench;
mod stats;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("============================================================");
    println!("  agent-otel-bridge Performance Benchmark Suite");
    println!("============================================================\n");

    // 1. Win32 Named Pipe RTT benchmark
    println!("[1/3] Benchmarking Win32 Named Pipe round-trip latency (1,000 iterations)...");
    let pipe_stats = pipe_bench::run(1000).await?;
    println!(
        "  -> p50: {} µs, p95: {} µs, p99: {} µs",
        pipe_stats.p50(),
        pipe_stats.p95(),
        pipe_stats.p99()
    );

    // 2. ProtoJSON parsing & OTLP span builder throughput
    println!("\n[2/3] Benchmarking ProtoJSON parsing & OTLP Protobuf span construction (10,000 iterations)...");
    let parse_result = parse_bench::run(10000);
    println!(
        "  -> p50: {} µs, p95: {} µs, p99: {} µs, throughput: {:.0} spans/sec",
        parse_result.stats.p50(),
        parse_result.stats.p95(),
        parse_result.stats.p99(),
        parse_result.spans_per_sec
    );

    // 3. Real Windows process spawn benchmark
    println!("\n[3/3] Benchmarking real Windows process lifecycle (agent-hook.exe, 250 iterations)...");
    let spawn_stats = spawn_bench::run(250)?;
    println!(
        "  -> p50: {} µs ({:.2} ms), p95: {} µs ({:.2} ms), p99: {} µs ({:.2} ms)",
        spawn_stats.p50(),
        spawn_stats.p50() as f64 / 1000.0,
        spawn_stats.p95(),
        spawn_stats.p95() as f64 / 1000.0,
        spawn_stats.p99(),
        spawn_stats.p99() as f64 / 1000.0
    );

    // 4. Generate report
    let report = report::BenchmarkReport {
        pipe: pipe_stats,
        parse: parse_result,
        spawn: spawn_stats,
        sla_target_us: 3000, // 3ms SLA
    };

    let md = report.to_markdown();

    // Ensure docs directory exists
    let docs_dir = Path::new("docs");
    if !docs_dir.exists() {
        fs::create_dir_all(docs_dir)?;
    }
    fs::write("docs/BENCHMARKS.md", &md)?;

    println!("\n============================================================");
    println!("Benchmark completed! Full report saved to docs/BENCHMARKS.md");
    println!("============================================================\n");
    println!("{}", md);

    Ok(())
}
