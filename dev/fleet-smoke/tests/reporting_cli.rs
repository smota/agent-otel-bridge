use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agent-otel-fleet-smoke"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn jsonl_publishes_identity_before_terminal_report() {
    let output = run(&["run", "--output", "jsonl"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows[0]["event"], "run_started");
    assert_eq!(rows.last().unwrap()["event"], "run_finished");
    assert_eq!(rows[0]["trace_id"].as_str().unwrap().len(), 32);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["sequence"], index + 1);
        assert_eq!(row["trace_id"], rows[0]["trace_id"]);
    }
    let report = &rows.last().unwrap()["data"];
    let schema: Value = serde_json::from_str(include_str!("../report-v2.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(
        validator.is_valid(report),
        "{}",
        validator
            .iter_errors(report)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    );
    let mut invalid = report.clone();
    invalid["summary"]
        .as_object_mut()
        .unwrap()
        .remove("trace_id");
    assert!(!validator.is_valid(&invalid));
    assert_eq!(report["schema_version"], 2);
    assert_eq!(report["summary"]["artifact_retention"], "not_created");
    assert_eq!(report["summary"]["backend_visibility"], "not_checked");
}

fn collector(reject: bool) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1/traces", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut payloads = Vec::new();
        let mut last = Instant::now();
        while last.elapsed() < Duration::from_secs(3) {
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(split) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
                    let size: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= split + 4 + size {
                        payloads.push(
                            serde_json::from_slice(&bytes[split + 4..split + 4 + size]).unwrap(),
                        );
                        break;
                    }
                }
            }
            let status = if reject {
                "500 Internal Server Error"
            } else {
                "200 OK"
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            )
            .unwrap();
            last = Instant::now();
        }
        payloads
    });
    (url, handle)
}

#[test]
fn transport_failure_preserves_partial_json_and_root_identity() {
    let (endpoint, server) = collector(true);
    let output = run(&["run", "--otlp-endpoint", &endpoint]);
    let payloads = server.join().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let reports: Value = serde_json::from_slice(&output.stdout).unwrap();
    let report = &reports[0];
    assert_eq!(report["summary"]["state"], "failed");
    assert!(!report["summary"]["time"]["end_utc"].is_null());
    assert_eq!(report["summary"]["task_counts"]["started"], 1);
    assert!(!report["summary"]["not_executed"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!report["run"]["spans"].as_array().unwrap().is_empty());
    assert_eq!(report["export"][0]["result"], "rejected");
    assert!(!payloads.is_empty());
}

#[test]
fn repeats_isolate_receipts_and_export_metadata() {
    let (endpoint, server) = collector(false);
    let output = run(&["run", "--repeat", "2", "--otlp-endpoint", &endpoint]);
    let payloads = server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reports: Value = serde_json::from_slice(&output.stdout).unwrap();
    for report in reports.as_array().unwrap() {
        for receipt in report["export"].as_array().unwrap() {
            assert_eq!(receipt["run_id"], report["run"]["run_id"]);
            assert_eq!(receipt["result"], "accepted");
        }
        assert_eq!(report["export"][0]["sequence"], 1);
    }
    assert_eq!(
        payloads.len(),
        reports
            .as_array()
            .unwrap()
            .iter()
            .map(|report| report["export"].as_array().unwrap().len())
            .sum::<usize>()
    );
    for payload in payloads {
        let attrs = payload["resourceSpans"][0]["resource"]["attributes"]
            .as_array()
            .unwrap();
        assert!(attrs.iter().any(|a| a["key"] == "agent.smoke.run_id"));
    }
}
