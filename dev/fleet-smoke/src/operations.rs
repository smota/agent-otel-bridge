use crate::{
    adapters::{spawn_bounded, CommandSpec},
    fixtures::{
        HttpFault, ScriptedHttpService, SeededWorkspace, MCP_INITIALIZE_REQUEST,
        MCP_INITIALIZE_RESPONSE,
    },
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    time::Duration,
};

pub fn perform(
    ws: &SeededWorkspace,
    operation: &str,
    expected: Option<String>,
) -> io::Result<(Option<String>, Option<String>)> {
    let observed = match operation {
        "valid-json" => {
            let value: Value =
                serde_json::from_str(&ws.read("event.json")?).map_err(io::Error::other)?;
            if value.get("event").and_then(Value::as_str).is_none()
                || value.get("step").and_then(Value::as_u64).is_none()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "valid fixture lacks event/step",
                ));
            }
            None
        }
        "invalid-json" => serde_json::from_str::<Value>(&ws.read("broken.json")?)
            .err()
            .map(|_| "invalid_json".into()),
        "function-defect" | "function-test" => run_function_test(ws)?,
        "mcp-initialize" => {
            let request: Value =
                serde_json::from_str(MCP_INITIALIZE_REQUEST).map_err(io::Error::other)?;
            let response: Value =
                serde_json::from_str(MCP_INITIALIZE_RESPONSE).map_err(io::Error::other)?;
            if request["method"] != "initialize"
                || response["result"]["serverInfo"]["name"].as_str() != Some("fleet-smoke-fixture")
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid MCP initialize contract",
                ));
            }
            None
        }
        "http-recovery" => http_recovery()?,
        "http-timeout" => http_timeout()?,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown fixture operation",
            ))
        }
    };
    Ok((expected, observed))
}

fn run_function_test(ws: &SeededWorkspace) -> io::Result<Option<String>> {
    let source = format!(
        "{}\nfn main() {{ assert_eq!(add(2, 3), 5); }}",
        ws.read("defect.rs")?
    );
    let source_path = ws.root().join("fixture_program.rs");
    let binary = ws.root().join(if cfg!(windows) {
        "fixture_program.exe"
    } else {
        "fixture_program"
    });
    fs::write(&source_path, source)?;
    let compile_spec = CommandSpec {
        program: "rustc".into(),
        args: vec![
            "--edition=2021".into(),
            source_path.as_os_str().to_os_string(),
            "-o".into(),
            binary.as_os_str().to_os_string(),
        ],
        env: BTreeMap::new(),
        workspace: ws.root().to_path_buf(),
        timeout: Duration::from_secs(2),
        stdout_limit_bytes: 64 * 1024,
        retry_limit: 0,
    };
    let compile = spawn_bounded(&compile_spec)
        .map_err(|e| io::Error::other(format!("fixture compiler spawn: {e:?}")))?;
    if compile.timed_out || compile.output_limited || compile.status != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "fixture source failed to compile",
        ));
    }
    let spec = CommandSpec {
        program: binary.into_os_string(),
        args: Vec::new(),
        env: BTreeMap::new(),
        workspace: ws.root().to_path_buf(),
        timeout: Duration::from_secs(2),
        stdout_limit_bytes: 64 * 1024,
        retry_limit: 0,
    };
    let result =
        spawn_bounded(&spec).map_err(|e| io::Error::other(format!("fixture spawn: {e:?}")))?;
    if result.timed_out {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "fixture assertion timed out",
        ));
    }
    if result.output_limited {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "fixture assertion output exceeded cap",
        ));
    }
    Ok((result.status != 0).then(|| "test_failure".into()))
}

fn http_recovery() -> io::Result<Option<String>> {
    let service = ScriptedHttpService::start(vec![HttpFault::Status(500), HttpFault::Success])?;
    let mut statuses = Vec::new();
    for _ in 0..2 {
        let mut stream = TcpStream::connect_timeout(&service.address(), Duration::from_secs(1))?;
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        stream.set_write_timeout(Some(Duration::from_secs(1)))?;
        stream.write_all(b"GET / HTTP/1.1\r\nConnection: close\r\n\r\n")?;
        stream.shutdown(Shutdown::Write)?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        statuses.push(response.lines().next().unwrap_or_default().to_owned());
    }
    let evidence = service.evidence();
    Ok((statuses
        .first()
        .is_some_and(|s| s.starts_with("HTTP/1.1 500"))
        && statuses
            .get(1)
            .is_some_and(|s| s.starts_with("HTTP/1.1 200"))
        && evidence.status_counts.get(&500) == Some(&1)
        && evidence.status_counts.get(&200) == Some(&1))
    .then(|| "http_500_recovered".into()))
}

fn http_timeout() -> io::Result<Option<String>> {
    let service = ScriptedHttpService::start(vec![HttpFault::Delay(Duration::from_millis(500))])?;
    let mut stream = TcpStream::connect_timeout(&service.address(), Duration::from_secs(1))?;
    stream.set_read_timeout(Some(Duration::from_millis(10)))?;
    stream.write_all(b"GET / HTTP/1.1\r\n\r\n")?;
    let mut response = [0; 16];
    let timed_out = matches!(stream.read(&mut response), Err(error) if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock));
    let accepted = service.evidence().accepted == 1;
    Ok((timed_out && accepted).then(|| "timeout".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_json_requires_fields() {
        let ws = SeededWorkspace::create().unwrap();
        assert_eq!(perform(&ws, "valid-json", None).unwrap().1, None);
    }
    #[test]
    fn invalid_json_is_observed() {
        let ws = SeededWorkspace::create().unwrap();
        assert_eq!(
            perform(&ws, "invalid-json", Some("invalid_json".into()))
                .unwrap()
                .1,
            Some("invalid_json".into())
        );
    }
    #[test]
    fn defect_and_mcp_are_local() {
        let ws = SeededWorkspace::create().unwrap();
        assert_eq!(
            perform(&ws, "function-defect", Some("test_failure".into()))
                .unwrap()
                .1,
            Some("test_failure".into())
        );
        assert_eq!(perform(&ws, "mcp-initialize", None).unwrap().1, None);
    }
    #[test]
    fn malformed_rust_is_a_compile_error() {
        let ws = SeededWorkspace::create().unwrap();
        fs::write(ws.root().join("defect.rs"), "fn add( {").unwrap();
        assert!(perform(&ws, "function-defect", Some("test_failure".into())).is_err());
    }
    #[test]
    fn repaired_rust_has_no_observed_fault() {
        let ws = SeededWorkspace::create().unwrap();
        fs::write(
            ws.root().join("defect.rs"),
            "fn add(a: i32, b: i32) -> i32 { a + b }",
        )
        .unwrap();
        assert_eq!(perform(&ws, "function-defect", None).unwrap().1, None);
    }
    #[test]
    fn missing_json_fields_are_rejected() {
        let ws = SeededWorkspace::create().unwrap();
        fs::write(ws.root().join("event.json"), "{\"event\":\"tool.start\"}").unwrap();
        assert!(perform(&ws, "valid-json", None).is_err());
    }
    #[test]
    fn repaired_invalid_json_has_no_observed_fault() {
        let ws = SeededWorkspace::create().unwrap();
        fs::write(
            ws.root().join("broken.json"),
            "{\"event\":\"tool.start\",\"step\":1}",
        )
        .unwrap();
        assert_eq!(perform(&ws, "invalid-json", None).unwrap().1, None);
    }
    #[test]
    fn http_faults_require_wire_recovery() {
        assert_eq!(http_recovery().unwrap(), Some("http_500_recovered".into()));
        assert_eq!(http_timeout().unwrap(), Some("timeout".into()));
    }
}
