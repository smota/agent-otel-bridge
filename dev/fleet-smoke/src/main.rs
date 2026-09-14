use agent_otel_fleet_smoke::{
    adapters::Platform,
    plan::seeded_profile,
    report::{read_artifact, summarize, trace_url, RunContext},
    runner::retain_artifact,
    telemetry::{export_http, export_snapshot, parse_observation},
    RunMode, Runner, Verifier,
};
use clap::{Parser, Subcommand};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Parser)]
#[command(
    name = "fleet-smoke",
    about = "Development-only fleet trace smoke laboratory"
)]
struct Cli {
    #[arg(long, global = true, default_value_t = 0x5eed_u64)]
    seed: u64,
    #[arg(long, global = true, default_value = "baseline")]
    profile: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Plan,
    Preflight,
    Run {
        #[arg(long)]
        live: bool,
        #[arg(long)]
        keep_artifact: bool,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=16))]
        repeat: u8,
        #[arg(long)]
        otlp_endpoint: Option<String>,
        #[arg(long, value_parser = ["json", "jsonl"], default_value = "json")]
        output: String,
        #[arg(long)]
        trace_url_template: Option<String>,
        #[arg(long)]
        codex_program: Option<PathBuf>,
        #[arg(long)]
        grok_program: Option<PathBuf>,
        #[arg(long)]
        antigravity_program: Option<PathBuf>,
    },
    Verify {
        artifact: PathBuf,
        #[arg(long)]
        observed_otlp: Option<PathBuf>,
    },
}

struct RunOptions {
    live: bool,
    keep_artifact: bool,
    repeat: u8,
    endpoint: Option<String>,
    output: String,
    trace_url_template: Option<String>,
    codex_program: Option<PathBuf>,
    grok_program: Option<PathBuf>,
    antigravity_program: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Plan => match seeded_profile(&cli.profile, cli.seed) {
            Some(plan) => emit(&plan, ExitCode::SUCCESS),
            None => invalid_profile(&cli.profile),
        },
        Command::Preflight => emit(&preflight(), ExitCode::SUCCESS),
        Command::Run {
            live,
            keep_artifact,
            repeat,
            otlp_endpoint,
            output,
            trace_url_template,
            codex_program,
            grok_program,
            antigravity_program,
        } => run_command(
            cli.seed,
            &cli.profile,
            RunOptions {
                live,
                keep_artifact,
                repeat,
                endpoint: otlp_endpoint,
                output,
                trace_url_template,
                codex_program,
                grok_program,
                antigravity_program,
            },
        ),
        Command::Verify {
            artifact,
            observed_otlp,
        } => verify_command(&artifact, observed_otlp.as_deref()),
    }
}

fn run_command(seed: u64, profile: &str, options: RunOptions) -> ExitCode {
    let RunOptions {
        live,
        keep_artifact,
        repeat,
        endpoint,
        output,
        trace_url_template,
        codex_program,
        grok_program,
        antigravity_program,
    } = options;
    let cancellation = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancellation);
    if let Err(error) = ctrlc::set_handler(move || {
        signal.store(true, Ordering::SeqCst);
    }) {
        return cli_error(format!("cannot register cancellation handler: {error}"));
    }
    let mut runner = Runner::new(seed)
        .with_profile(profile)
        .with_cancellation(cancellation);
    for (platform, path) in [
        ("codex", codex_program),
        ("grok", grok_program),
        ("antigravity", antigravity_program),
    ] {
        if let Some(path) = path {
            if is_wrapper(&path) {
                return cli_error(format!("refusing wrapper program {}", path.display()));
            }
            runner = runner.with_native_program(platform, path);
        }
    }
    if seeded_profile(profile, seed).is_none() {
        return invalid_profile(profile);
    }
    let mode = if live {
        RunMode::Live
    } else {
        RunMode::Synthetic
    };
    let mut reports = Vec::with_capacity(repeat as usize);
    let mut exit = ExitCode::SUCCESS;
    for repetition in 1..=repeat {
        let receipts = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let observed_receipts = Arc::clone(&receipts);
        let seen = Mutex::new(BTreeSet::<String>::new());
        let sequence = Arc::new(AtomicU64::new(0));
        let observed_sequence = Arc::clone(&sequence);
        let observer_endpoint = endpoint.clone();
        let jsonl = output == "jsonl";
        runner = runner.with_event_observer(Arc::new(move |event, artifact| {
            let context = RunContext::from_artifact(artifact);
            if jsonl && event != "run_finished" {
                emit_event(event, &context, &observed_sequence, serde_json::json!({"spans_observed":artifact.spans.len()}))?;
            }
            if event == "run_started" { return Ok(()); }
            let Some(endpoint) = &observer_endpoint else { return Ok(()); };
            let mut seen = seen.lock().map_err(|_| std::io::Error::other("span lock poisoned"))?;
            let batch: Vec<_> = artifact.spans.iter().filter(|span| span.phase != "root-start" && seen.insert(span.span_id.clone())).cloned().collect();
            if batch.is_empty() { return Ok(()); }
            let mut payload = export_snapshot(&context, &batch, &artifact.spans);
            let mapping_failures = agent_otel_fleet_smoke::telemetry_validation::validate_export(&context, &batch, &payload);
            if !mapping_failures.is_empty() { return Err(std::io::Error::other(mapping_failures.join("; "))); }
            if event == "run_finished" {
                let assertions = Verifier::assertions(&artifact.plan, &artifact.spans);
                let divergence_count = assertions.iter().filter(|item| item["result"] != "PASS").count();
                payload["resourceSpans"][0]["resource"]["attributes"].as_array_mut().expect("export resource attributes").push(serde_json::json!({"key":"agent.smoke.divergence_count","value":{"intValue":divergence_count.to_string()}}));
            }
            let started = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
            let timer = Instant::now();
            let result = export_http(endpoint, &payload);
            let accepted = result.as_ref().is_ok_and(|r| r.transport_accepted);
            let mut receipts = observed_receipts.lock().map_err(|_| std::io::Error::other("receipt lock poisoned"))?;
            let batch_index = receipts.len() + 1;
            let receipt = serde_json::json!({
                "batch_id":format!("{}:{batch_index}",context.run_id), "run_id":context.run_id,"trace_id":context.trace_id,
                "sequence":batch_index,"span_ids":batch.iter().map(|span| &span.span_id).collect::<Vec<_>>(),"sent_spans":batch.len(),
                "start_unix_nanos":started.to_string(),"duration_micros":timer.elapsed().as_micros(),
                "http_status":result.as_ref().ok().map(|r|r.http_status),"rejected_spans":result.as_ref().ok().map(|r|r.rejected_spans),
                "transport_accepted":accepted,"result":if accepted {"accepted"} else if result.is_ok() {"rejected"} else {"unknown"},
                "error":result.as_ref().err(),"backend_visibility":"not_checked"
            });
            receipts.push(receipt.clone());
            if jsonl { emit_event("export_receipt", &context, &observed_sequence, receipt)?; }
            if accepted { Ok(()) } else { Err(std::io::Error::other("OTLP export not accepted; see batch receipt")) }
        }));
        let mut outcome = match runner.run_outcome(mode) {
            Ok(outcome) => outcome,
            Err(error) => {
                return emit(
                    &serde_json::json!({"schema_version":2,"state":"failed","error":error.to_string()}),
                    ExitCode::from(2),
                );
            }
        };
        let run = &mut outcome.artifact;
        let verification = Verifier::verify_plan(&run.plan, &run.spans);
        let export = receipts.lock().expect("receipt lock").clone();
        let mut summary = summarize(run, repetition as usize, repeat as usize);
        summary["state"] = serde_json::json!(outcome.state);
        summary["errors"] = serde_json::json!(outcome.errors);
        summary["operational_errors"] = serde_json::json!(outcome.errors.iter().map(|error|serde_json::json!({
            "category":if error.contains("cancelled") {"cancelled"} else if error.contains("observer") {"telemetry_observer"} else if error.starts_with("task ") {"task_execution"} else {"runner"},
            "message":error,"evidence_origin":"fleet-smoke-lab"
        })).collect::<Vec<_>>());
        summary["not_executed"] = serde_json::json!(outcome.not_executed);
        summary["backend_visibility"] = serde_json::json!("not_checked");
        summary["assertions"] = serde_json::json!(Verifier::assertions(&run.plan, &run.spans));
        summary["evaluation"]["cases"] = serde_json::json!(if summary["assertions"]
            .as_array()
            .expect("assertions")
            .iter()
            .all(|item| item["result"] == "PASS")
        {
            "PASS"
        } else if summary["assertions"]
            .as_array()
            .expect("assertions")
            .iter()
            .any(|item| item["result"] == "FAIL")
        {
            "FAIL"
        } else {
            "INCONCLUSIVE"
        });
        summary["evaluation"]["trace_structure"] =
            serde_json::json!(Verifier::verify_structure(&run.plan, &run.spans));
        summary["evaluation"]["transport"] =
            serde_json::json!(if endpoint.is_none() || export.is_empty() {
                "not_attempted"
            } else if export.iter().all(|item| item["result"] == "accepted") {
                "accepted"
            } else {
                "not_accepted"
            });
        summary["artifact_retention"] = serde_json::json!("not_created");
        if let Some(template) = &trace_url_template {
            match trace_url(template, run) {
                Ok(url) => {
                    summary["navigation"]["links"] = serde_json::json!([url]);
                }
                Err(error) => {
                    summary["navigation_error"] = serde_json::json!(error);
                    summary["state"] = serde_json::json!("failed");
                    exit = ExitCode::from(2);
                }
            }
        }
        if keep_artifact {
            match retain_artifact(run) {
                Ok(()) => {
                    summary["artifact_retention"] = serde_json::json!("retained");
                    summary["navigation"]["artifact_retention"] = serde_json::json!("retained");
                    summary["navigation"]["artifact_path"] = serde_json::json!(run.artifact_path);
                    summary["artifact_path"] = serde_json::json!(run.artifact_path);
                    let mut retained = serde_json::to_value(&*run).expect("artifact serializable");
                    retained["schema_version"] = serde_json::json!(2);
                    retained["summary"] = summary.clone();
                    retained["verification"] = serde_json::json!(verification);
                    retained["export"] = serde_json::json!(export);
                    if let Err(error) = fs::write(
                        &run.artifact_path,
                        serde_json::to_vec_pretty(&retained).expect("report serializable"),
                    ) {
                        summary["artifact_retention"] = serde_json::json!("write_failed");
                        summary["navigation"]["artifact_retention"] =
                            serde_json::json!("write_failed");
                        summary["state"] = serde_json::json!("failed");
                        summary["retention_error"] = serde_json::json!(error.to_string());
                        exit = ExitCode::from(2);
                    }
                }
                Err(error) => {
                    summary["retention_error"] = serde_json::json!(error.to_string());
                    summary["state"] = serde_json::json!("failed");
                    exit = ExitCode::from(2);
                }
            }
        }
        if !verification.passed && exit == ExitCode::SUCCESS {
            exit = ExitCode::from(1);
        }
        if outcome.state != "completed" {
            exit = ExitCode::from(2);
        }
        let report = serde_json::json!({"schema_version":2,"summary":summary,"run":run,"verification":verification,"export":export});
        if jsonl
            && emit_event(
                "run_finished",
                &RunContext::from_artifact(run),
                &sequence,
                report.clone(),
            )
            .is_err()
        {
            return ExitCode::from(2);
        }
        reports.push(report);
        if outcome.state == "cancelled" {
            break;
        }
    }
    if output == "jsonl" {
        exit
    } else {
        emit(&reports, exit)
    }
}

fn emit_event(
    event: &str,
    context: &RunContext,
    sequence: &AtomicU64,
    data: serde_json::Value,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    let value = serde_json::json!({"schema_version":2,"event":event,"run_id":context.run_id,"trace_id":context.trace_id,"root_span_id":context.root_span_id,"sequence":sequence.fetch_add(1,Ordering::SeqCst)+1,"data":data});
    serde_json::to_writer(&mut stdout, &value)?;
    writeln!(stdout)?;
    stdout.flush()
}

fn verify_command(path: &Path, observed_path: Option<&Path>) -> ExitCode {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            return cli_error(format!(
                "could not read artifact {}: {error}",
                path.display()
            ))
        }
    };
    let run = match read_artifact(&raw) {
        Ok(run) => run,
        Err(error) => {
            return cli_error(format!(
                "could not parse artifact {}: {error}",
                path.display()
            ))
        }
    };
    if seeded_profile(&run.profile, run.seed).as_ref() != Some(&run.plan) {
        return cli_error("artifact plan does not match built-in seed/profile plan");
    }
    let report = match observed_path {
        Some(path) => {
            let raw = match fs::read_to_string(path) {
                Ok(raw) => raw,
                Err(error) => {
                    return cli_error(format!(
                        "could not read observed OTLP {}: {error}",
                        path.display()
                    ))
                }
            };
            let observed = match parse_observation(&raw) {
                Ok(value) => value,
                Err(error) => return cli_error(format!("could not parse observed OTLP: {error}")),
            };
            Verifier::verify_native(&run.plan, &run.spans, &observed)
        }
        None => Verifier::verify_plan(&run.plan, &run.spans),
    };
    emit(
        &report,
        if report.passed {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        },
    )
}

fn preflight() -> serde_json::Value {
    let adapters = [("codex", Platform::Codex), ("grok", Platform::Grok), ("antigravity", Platform::Antigravity)].into_iter().map(|(name, platform)| serde_json::json!({"platform":name, "model":platform.model(), "program":platform.program(), "available":executable_available(platform.program()), "auth":"not_checked"})).collect::<Vec<_>>();
    serde_json::json!({"mode":"planning-only; no commands or inference executed", "adapters":adapters, "claude":"excluded"})
}

fn executable_available(program: &str) -> bool {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return path.is_file();
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .any(|dir| {
            [program.to_owned(), format!("{program}.exe")]
                .into_iter()
                .map(|name| dir.join(name))
                .any(|candidate| candidate.is_file())
        })
}
fn is_wrapper(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("ps1" | "cmd" | "bat" | "sh")
    )
}
fn invalid_profile(profile: &str) -> ExitCode {
    cli_error(format!(
        "unknown profile `{profile}`; choose baseline, mixed, or long-trace"
    ))
}
fn cli_error(message: impl Into<String>) -> ExitCode {
    eprintln!("fleet-smoke: {}", message.into());
    ExitCode::from(2)
}
fn emit(value: &impl serde::Serialize, code: ExitCode) -> ExitCode {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serializable CLI output")
    );
    code
}
