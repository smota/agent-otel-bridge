//! Deterministic, local-only fixtures for the fleet smoke lab.
//!
//! The fixtures deliberately use the standard library so the smoke lab can be
//! run on Windows, Linux, and macOS without shell scripts or external services.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A small named payload consumed by the runner/verifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fixture {
    pub name: &'static str,
    pub content: &'static str,
    pub expected_fault: Option<&'static str>,
}

/// Stable catalog of payloads used by the smoke lab.
pub fn fixture_catalog() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "valid-json",
            content: r#"{"event":"tool.start","step":1}"#,
            expected_fault: None,
        },
        Fixture {
            name: "invalid-json",
            content: r#"{"event":"tool.start","step":}"#,
            expected_fault: Some("invalid_json"),
        },
        Fixture {
            name: "function-defect",
            content: "fn add(a: i32, b: i32) -> i32 { a - b }",
            expected_fault: Some("test_failure"),
        },
        Fixture {
            name: "function-test",
            content: "assert_eq!(add(2, 3), 5);",
            expected_fault: Some("test_failure"),
        },
        Fixture {
            name: "mcp-initialize",
            content: r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            expected_fault: None,
        },
    ]
}

/// A temporary workspace containing the smallest useful valid/invalid inputs.
pub struct SeededWorkspace {
    root: Option<PathBuf>,
}

impl SeededWorkspace {
    pub fn create() -> std::io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self::create_with_stamp(stamp)
    }

    fn create_with_stamp(stamp: u128) -> std::io::Result<Self> {
        static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);
        let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "agent-otel-fleet-fixture-{}-{}-{}",
            std::process::id(),
            stamp,
            sequence
        ));
        fs::create_dir(&root)?;
        let result = (|| {
            fs::write(root.join("event.json"), fixture_catalog()[0].content)?;
            fs::write(root.join("broken.json"), fixture_catalog()[1].content)?;
            fs::write(root.join("defect.rs"), fixture_catalog()[2].content)?;
            fs::write(root.join("defect_test.rs"), fixture_catalog()[3].content)
        })();
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&root);
            return Err(error);
        }
        Ok(Self { root: Some(root) })
    }

    pub fn root(&self) -> &Path {
        self.root.as_deref().expect("workspace already cleaned")
    }

    pub fn read(&self, name: &str) -> std::io::Result<String> {
        if Path::new(name)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "fixture path must contain normal components only",
            ));
        }
        let root = self.root();
        let candidate = root.join(name);
        let canonical_root = root.canonicalize()?;
        let canonical_candidate = candidate.canonicalize()?;
        if !canonical_candidate.starts_with(&canonical_root) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "fixture path escapes workspace",
            ));
        }
        fs::read_to_string(canonical_candidate)
    }

    pub fn cleanup(mut self) -> std::io::Result<()> {
        self.root.take().map(fs::remove_dir_all).unwrap_or(Ok(()))
    }
}

impl Drop for SeededWorkspace {
    fn drop(&mut self) {
        if let Some(root) = self.root.take() {
            let _ = fs::remove_dir_all(root);
        }
    }
}

/// A deterministic HTTP script. `Close` drops the connection without a response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HttpFault {
    Status(u16),
    Delay(Duration),
    Close,
    Success,
}

#[derive(Clone, Debug, Default)]
pub struct HttpEvidence {
    pub accepted: usize,
    pub completed: usize,
    pub status_counts: BTreeMap<u16, usize>,
    pub closed: usize,
}

struct HttpCounters {
    accepted: AtomicUsize,
    completed: AtomicUsize,
    closed: AtomicUsize,
    statuses: Mutex<BTreeMap<u16, usize>>,
}

/// Local loopback server with finite, scripted behaviour and cooperative cancellation.
pub struct ScriptedHttpService {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    counters: Arc<HttpCounters>,
    worker: Option<JoinHandle<()>>,
}

impl ScriptedHttpService {
    pub fn start(script: Vec<HttpFault>) -> std::io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(HttpCounters {
            accepted: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            closed: AtomicUsize::new(0),
            statuses: Mutex::new(BTreeMap::new()),
        });
        let worker_stop = Arc::clone(&stop);
        let worker_counters = Arc::clone(&counters);
        let worker = thread::spawn(move || {
            let mut index = 0;
            while !worker_stop.load(Ordering::Acquire) && index < script.len() {
                match listener.accept() {
                    Ok((stream, _)) => {
                        worker_counters.accepted.fetch_add(1, Ordering::Relaxed);
                        serve_one(stream, &script[index], &worker_counters, &worker_stop);
                        index += 1;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            address,
            stop,
            counters,
            worker: Some(worker),
        })
    }

    pub fn address(&self) -> std::net::SocketAddr {
        self.address
    }

    pub fn evidence(&self) -> HttpEvidence {
        let status_counts = self
            .counters
            .statuses
            .lock()
            .expect("status mutex poisoned")
            .clone();
        HttpEvidence {
            accepted: self.counters.accepted.load(Ordering::Relaxed),
            completed: self.counters.completed.load(Ordering::Relaxed),
            status_counts,
            closed: self.counters.closed.load(Ordering::Relaxed),
        }
    }
}

fn serve_one(mut stream: TcpStream, fault: &HttpFault, counters: &HttpCounters, stop: &AtomicBool) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let mut request = [0; 512];
    let _ = stream.read(&mut request);
    match fault {
        HttpFault::Delay(duration) => {
            let started = std::time::Instant::now();
            while started.elapsed() < *duration && !stop.load(Ordering::Acquire) {
                thread::sleep(
                    duration
                        .saturating_sub(started.elapsed())
                        .min(Duration::from_millis(5)),
                );
            }
            if stop.load(Ordering::Acquire) {
                return;
            }
        }
        HttpFault::Close => {
            counters.closed.fetch_add(1, Ordering::Relaxed);
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
        _ => {}
    }
    let status = match fault {
        HttpFault::Status(code) => *code,
        _ => 200,
    };
    let body = if status == 200 {
        "{\"ok\":true}"
    } else {
        "{\"ok\":false}"
    };
    let reason = match status {
        200 => "OK",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Fixture",
    };
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        body.len(),
        body
    );
    if stream.write_all(response.as_bytes()).is_ok() {
        counters.completed.fetch_add(1, Ordering::Relaxed);
        *counters
            .statuses
            .lock()
            .expect("status mutex poisoned")
            .entry(status)
            .or_default() += 1;
    }
}

impl Drop for ScriptedHttpService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Minimal JSON-RPC MCP fixture, suitable for stdio adapters or an HTTP body.
pub const MCP_INITIALIZE_REQUEST: &str =
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
pub const MCP_INITIALIZE_RESPONSE: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"fleet-smoke-fixture","version":"1.0.0"}}}"#;
pub const MCP_TOOLS_LIST_REQUEST: &str =
    r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
pub const MCP_TOOLS_LIST_RESPONSE: &str = r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"fixture_echo","description":"Returns deterministic fixture text","inputSchema":{"type":"object"}}]}}"#;
pub const MCP_TOOLS_CALL_REQUEST: &str = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"fixture_echo","arguments":{}}}"#;
pub const MCP_TOOLS_CALL_RESPONSE: &str =
    r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"fixture-ok"}]}}"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;

    #[test]
    fn same_clock_tick_workspaces_remain_isolated() {
        let first = SeededWorkspace::create_with_stamp(0).unwrap();
        let second = SeededWorkspace::create_with_stamp(0).unwrap();
        assert_ne!(first.root(), second.root());
        fs::write(first.root().join("event.json"), "first only").unwrap();
        assert_ne!(
            first.read("event.json").unwrap(),
            second.read("event.json").unwrap()
        );
        first.cleanup().unwrap();
        assert!(second.root().is_dir());
    }

    #[test]
    fn workspace_is_seeded_and_cleaned() {
        let workspace = SeededWorkspace::create().unwrap();
        let root = workspace.root().to_path_buf();
        let sentinel = root.parent().unwrap().join(format!(
            "{}-sentinel",
            root.file_name().unwrap().to_string_lossy()
        ));
        fs::write(&sentinel, "keep").unwrap();
        assert!(workspace.read("event.json").unwrap().contains("tool.start"));
        assert!(workspace.read("broken.json").unwrap().ends_with("step\":}"));
        assert!(workspace.read("../broken.json").is_err());
        workspace.cleanup().unwrap();
        assert!(!root.exists());
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
        fs::remove_file(sentinel).unwrap();
    }

    #[test]
    fn http_script_records_faults_and_recovery() {
        let service = ScriptedHttpService::start(vec![
            HttpFault::Status(500),
            HttpFault::Status(429),
            HttpFault::Success,
        ])
        .unwrap();
        for expected in [500, 429, 200] {
            let mut stream = TcpStream::connect(service.address()).unwrap();
            stream.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with(&format!("HTTP/1.1 {expected}")));
        }
        let evidence = service.evidence();
        assert_eq!(evidence.accepted, 3);
        assert_eq!(evidence.completed, 3);
        assert_eq!(evidence.status_counts.get(&500), Some(&1));
        assert_eq!(evidence.status_counts.get(&429), Some(&1));
        assert_eq!(evidence.status_counts.get(&200), Some(&1));
    }

    #[test]
    fn http_close_is_finite_and_cleanup_joins_worker() {
        let service = ScriptedHttpService::start(vec![HttpFault::Close]).unwrap();
        let mut stream = TcpStream::connect(service.address()).unwrap();
        stream.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        assert!(response.is_empty());
        assert_eq!(service.evidence().closed, 1);
        drop(service);
    }

    #[test]
    fn http_delay_is_observable_and_cancellation_is_bounded() {
        let service =
            ScriptedHttpService::start(vec![HttpFault::Delay(Duration::from_millis(20))]).unwrap();
        let started = std::time::Instant::now();
        let mut stream = TcpStream::connect(service.address()).unwrap();
        stream.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(15));
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        drop(service);

        let service =
            ScriptedHttpService::start(vec![HttpFault::Delay(Duration::from_secs(5))]).unwrap();
        let mut stream = TcpStream::connect(service.address()).unwrap();
        stream.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        let started = std::time::Instant::now();
        drop(service);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn http_drop_after_worker_exit_is_bounded() {
        let service = ScriptedHttpService::start(vec![]).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !service.worker.as_ref().unwrap().is_finished()
            && std::time::Instant::now() < deadline
        {
            thread::yield_now();
        }
        assert!(service.worker.as_ref().unwrap().is_finished());

        let started = std::time::Instant::now();
        drop(service);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
