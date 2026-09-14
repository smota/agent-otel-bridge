//! End-to-end candidate-process check for context propagation.
//!
//! Run explicitly with `AGENT_OTEL_TEST_HOOK` pointing at the candidate
//! `agent-hook` binary. It is ignored by default because it starts processes
//! and binds local sockets or named pipes.

use agent_otel_core::trace_id::derive_trace_id;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;
#[cfg(windows)]
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

const PROCESS_TIMEOUT: Duration = Duration::from_secs(3);
const COLLECTOR_TIMEOUT: Duration = Duration::from_secs(5);
const TRACE_A: &str = "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-1111111111111111-01";
const TRACE_B: &str = "00-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-2222222222222222-00";
const TRACE_C: &str = "00-cccccccccccccccccccccccccccccccc-3333333333333333-01";
const TRACE_D: &str = "00-dddddddddddddddddddddddddddddddd-4444444444444444-00";
const TRACE_E: &str = "00-eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-5555555555555555-01";

#[derive(Default)]
struct CollectorState {
    traces: Vec<ExportTraceServiceRequest>,
}

struct TestRuntime {
    root: PathBuf,
    daemon: Child,
    collector_stop: Arc<AtomicBool>,
    collector: Option<JoinHandle<()>>,
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        self.collector_stop.store(true, Ordering::Release);
        terminate_child(&mut self.daemon);
        if let Some(collector) = self.collector.take() {
            let _ = collector.join();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
#[ignore = "requires AGENT_OTEL_TEST_HOOK and starts a local daemon/collector"]
fn native_context_process_propagates_trace_context() {
    let hook = std::env::var_os("AGENT_OTEL_TEST_HOOK")
        .map(PathBuf::from)
        .expect("set AGENT_OTEL_TEST_HOOK to the candidate agent-hook binary");
    assert!(
        hook.is_file(),
        "AGENT_OTEL_TEST_HOOK is not a file: {}",
        hook.display()
    );
    let root = std::env::temp_dir().join(format!(
        "aoc-{}-{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir(&root).expect("create isolated test directory");
    let transport = Transport::new(&root);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind collector");
    let endpoint = format!(
        "http://{}",
        listener.local_addr().expect("collector address")
    );
    let state = Arc::new(Mutex::new(CollectorState::default()));
    let collector_stop = Arc::new(AtomicBool::new(false));
    let collector = spawn_collector(listener, Arc::clone(&state), Arc::clone(&collector_stop));

    let bridge = PathBuf::from(env!("CARGO_BIN_EXE_agent-otel-bridge"));
    let mut command = Command::new(&bridge);
    command
        .arg("daemon")
        .env("TRACEPARENT", TRACE_C)
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint)
        .env("AGENT_OTEL_BATCH_SIZE", "1")
        .env("AGENT_OTEL_BATCH_TIMEOUT_MS", "20")
        .env("AGENT_OTEL_QUOTA_INTERVAL_SECS", "3600")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    transport.configure(&mut command);
    hide_window(&mut command);
    let daemon = command.spawn().expect("spawn isolated daemon");
    let mut runtime = TestRuntime {
        root,
        daemon,
        collector_stop,
        collector: Some(collector),
    };

    transport.wait_ready();
    thread::scope(|scope| {
        scope.spawn(|| run_hook(&hook, &transport, "e2e-a", Some(TRACE_A), None));
        scope.spawn(|| run_hook(&hook, &transport, "e2e-b", Some(TRACE_B), None));
    });
    run_hook(&hook, &transport, "e2e-d", Some(TRACE_A), Some(TRACE_D));
    run_hook(&hook, &transport, "e2e-fallback", None, None);
    run_cli_hook(&bridge, &transport, "e2e-cli", TRACE_E);

    let deadline = Instant::now() + COLLECTOR_TIMEOUT;
    while trace_count(&state) < 5 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let spans = observed_spans(&state);
    assert_eq!(
        spans.len(),
        5,
        "expected five hook spans, got {}: {spans:?}",
        spans.len()
    );
    assert_span(&spans, 0xaa, 0x11, 1);
    assert_span(&spans, 0xbb, 0x22, 0);
    assert_span(&spans, 0xdd, 0x44, 0);
    assert_span(&spans, 0xee, 0x55, 1);
    let fallback = derive_trace_id(Some("e2e-fallback"));
    assert!(spans.iter().any(|(trace, parent, flags)| {
        trace.as_slice() == fallback.as_slice() && parent.is_empty() && *flags == 1
    }));
    assert!(spans
        .iter()
        .all(|(trace, _, _)| trace.as_slice() != [0xcc; 16]));

    runtime.collector_stop.store(true, Ordering::Release);
    terminate_child(&mut runtime.daemon);
}

struct Transport {
    #[cfg(unix)]
    socket: PathBuf,
    #[cfg(windows)]
    pipe_name: String,
}

impl Transport {
    fn new(root: &Path) -> Self {
        #[cfg(unix)]
        {
            Self {
                socket: root.join("bridge.sock"),
            }
        }
        #[cfg(windows)]
        {
            let _ = root;
            Self {
                pipe_name: format!(r"\\.\pipe\agent-otel-native-context-{}", std::process::id()),
            }
        }
    }

    fn configure(&self, command: &mut Command) {
        #[cfg(windows)]
        command.env("AGENT_OTEL_PIPE", &self.pipe_name);
        #[cfg(unix)]
        {
            command
                .env("AGENT_OTEL_SOCKET", &self.socket)
                .env_remove("AGENT_OTEL_PIPE")
                .env_remove("AGY_OTEL_PIPE");
        }
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + PROCESS_TIMEOUT;
        loop {
            if self.connect_ready().is_ok() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "daemon did not create isolated IPC transport"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    fn connect_ready(&self) -> io::Result<()> {
        std::os::unix::net::UnixStream::connect(&self.socket).map(|_| ())
    }
    #[cfg(windows)]
    fn connect_ready(&self) -> io::Result<()> {
        OpenOptions::new()
            .write(true)
            .open(&self.pipe_name)
            .map(|_| ())
    }
}

fn run_hook(
    hook: &Path,
    transport: &Transport,
    conversation: &str,
    origin: Option<&str>,
    explicit: Option<&str>,
) {
    let input = match explicit {
        Some(traceparent) => format!(r#"{{"hook_event":"PostInvocation","conversation_id":"{conversation}","traceparent":"{traceparent}"}}"#).into_bytes(),
        None => format!(r#"{{"hook_event":"PostInvocation","conversation_id":"{conversation}"}}"#).into_bytes(),
    };
    let mut command = Command::new(hook);
    command
        .arg("PostInvocation")
        .arg("--client")
        .arg("codex")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_remove("TRACEPARENT");
    transport.configure(&mut command);
    hide_window(&mut command);
    if let Some(origin) = origin {
        command.env("TRACEPARENT", origin);
    }
    let mut child = command.spawn().expect("spawn hook");
    child
        .stdin
        .take()
        .expect("hook stdin")
        .write_all(&input)
        .expect("write hook JSON");
    wait_success(&mut child, "hook");
}

fn run_cli_hook(bridge: &Path, transport: &Transport, conversation: &str, traceparent: &str) {
    let input = format!(r#"{{"hook_event":"PostInvocation","conversation_id":"{conversation}"}}"#);
    let mut command = Command::new(bridge);
    command
        .arg("hook")
        .arg("PostInvocation")
        .env("TRACEPARENT", traceparent)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    transport.configure(&mut command);
    hide_window(&mut command);
    let mut child = command.spawn().expect("spawn CLI hook");
    child
        .stdin
        .take()
        .expect("CLI hook stdin")
        .write_all(input.as_bytes())
        .expect("write CLI hook JSON");
    wait_success(&mut child, "CLI hook");
}

fn wait_success(child: &mut Child, name: &str) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            assert!(status.success(), "{name} exited with {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{name} did not exit within {PROCESS_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn terminate_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

fn spawn_collector(
    listener: TcpListener,
    state: Arc<Mutex<CollectorState>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    listener
        .set_nonblocking(true)
        .expect("nonblocking collector listener");
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => collect_request(stream, &state),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(_) => return,
            }
        }
    })
}

fn collect_request(mut stream: TcpStream, state: &Arc<Mutex<CollectorState>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let Ok(request) = read_http_request(&mut stream) else {
        return;
    };
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("validated HTTP header")
        + 4;
    let (header, body) = request.split_at(header_end);
    if header.starts_with(b"POST /v1/traces") {
        if let Ok(value) = ExportTraceServiceRequest::decode(body) {
            state.lock().expect("collector state").traces.push(value);
        }
    }
    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
}

fn read_http_request(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    const MAX_HEADER: usize = 16 * 1024;
    const MAX_BODY: usize = 2 * 1024 * 1024;
    let mut request = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP header incomplete",
            ));
        }
        request.extend_from_slice(&chunk[..count]);
        if request.len() > MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP header too large",
            ));
        }
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let content_length = std::str::from_utf8(&request[..header_end])
        .ok()
        .and_then(|header| {
            header.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("Content-Length").then_some(value)
            })
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|length| *length <= MAX_BODY)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid Content-Length"))?;
    let target = header_end + content_length;
    while request.len() < target {
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP body incomplete",
            ));
        }
        request.extend_from_slice(&chunk[..count]);
    }
    request.truncate(target);
    Ok(request)
}

fn trace_count(state: &Arc<Mutex<CollectorState>>) -> usize {
    observed_spans(state).len()
}
fn observed_spans(state: &Arc<Mutex<CollectorState>>) -> Vec<(Vec<u8>, Vec<u8>, u32)> {
    state
        .lock()
        .expect("collector state")
        .traces
        .iter()
        .flat_map(|request| &request.resource_spans)
        .flat_map(|resource| &resource.scope_spans)
        .flat_map(|scope| &scope.spans)
        .map(|span| {
            (
                span.trace_id.clone(),
                span.parent_span_id.clone(),
                span.flags,
            )
        })
        .collect()
}
fn assert_span(spans: &[(Vec<u8>, Vec<u8>, u32)], trace: u8, parent: u8, flags: u32) {
    assert!(spans
        .iter()
        .any(|(actual_trace, actual_parent, actual_flags)| {
            actual_trace.as_slice() == [trace; 16]
                && actual_parent.as_slice() == [parent; 8]
                && *actual_flags == flags
        }));
}
