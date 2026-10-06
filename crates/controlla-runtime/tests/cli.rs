use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

fn finish_help_before_one_second(mut child: std::process::Child) -> std::process::Output {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("help command exceeded the one-second limit");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn help_exits_while_stdin_is_open_without_creating_state() {
    let state = std::env::temp_dir().join(format!("controlla-help-{}", std::process::id()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_controlla"))
        .args(["--state-dir", state.to_str().unwrap(), "--help"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _stdin = child.stdin.take().unwrap();
    let output = finish_help_before_one_second(child);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("doctor"));
    assert!(!state.exists());
}

#[test]
fn subcommand_help_also_exits_while_stdin_is_open() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_controlla"))
        .args(["doctor", "--help"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _stdin = child.stdin.take().unwrap();
    let output = finish_help_before_one_second(child);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("doctor"));
}

#[test]
fn unknown_command_is_rejected_before_subcommand_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_controlla"))
        .args(["unknown", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown command"));
}

#[test]
fn invalid_flags_exit_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_controlla"))
        .arg("--not-a-flag")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown argument"));
}

fn serve_authenticated_health(principal: &'static str, revision: u64) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let len = stream.read(&mut request).unwrap();
        assert!(
            String::from_utf8_lossy(&request[..len])
                .contains("Authorization: Bearer fixture-secret")
        );
        let body = format!(
            r#"{{"authenticated":true,"principal_id":"{principal}","policy_revision":{revision}}}"#
        );
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    addr
}

fn write_doctor_config(
    root: &std::path::Path,
    addr: std::net::SocketAddr,
    pid: u32,
    heartbeat: u64,
) {
    let config = serde_json::json!({
        "schema_version": 1,
        "service_pid": pid,
        "loopback_addr": addr.to_string(),
        "bearer_token": "fixture-secret",
        "principal_id": "fixture-principal",
        "heartbeat_unix_seconds": heartbeat,
        "heartbeat_max_age_seconds": 60,
        "policy_revision": 4
    });
    std::fs::write(
        root.join("runtime.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
}

fn run_doctor(root: &std::path::Path) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_controlla"))
        .args(["--state-dir", root.to_str().unwrap(), "doctor"])
        .output()
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn doctor_reports_stale_heartbeat_separately_from_live_process_and_authenticated_health() {
    let root = std::env::temp_dir().join(format!("controlla doctor {}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let addr = serve_authenticated_health("fixture-principal", 4);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    write_doctor_config(&root, addr, std::process::id(), now - 900);
    let report = run_doctor(&root);
    assert_eq!(report["configured"], "Healthy");
    // The current local-process probe is implemented on Unix. On other
    // platforms, the doctor must report Unknown rather than imply liveness.
    #[cfg(unix)]
    assert_eq!(report["process"], "Healthy");
    #[cfg(not(unix))]
    assert_eq!(report["process"], "Unknown");
    assert_eq!(report["transport"], "Healthy");
    assert_eq!(report["authentication"], "Healthy");
    assert_eq!(report["live_round_trip"], "Healthy");
    assert_eq!(report["heartbeat"], "Failed");
    assert!(report["heartbeat_age_seconds"].as_u64().unwrap() >= 900);
    assert!(report["binary_hash"].as_str().unwrap().len() == 64);
    let correlation = report["principal_correlation"].as_str().unwrap();
    assert!(!correlation.contains("fixture-principal"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn doctor_reports_fresh_heartbeat_but_dead_process_and_unreachable_loopback() {
    let root = std::env::temp_dir().join(format!("controlla doctor dead {}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    write_doctor_config(&root, addr, u32::MAX, now);
    let report = run_doctor(&root);
    let unavailable_process_probe = if cfg!(unix) { "Failed" } else { "Unknown" };
    assert_eq!(report["process"], unavailable_process_probe);
    assert_eq!(report["transport"], "Failed");
    assert_eq!(report["authentication"], "Unknown");
    assert_eq!(report["heartbeat"], "Healthy");
    assert_ne!(report["heartbeat_age_seconds"], serde_json::Value::Null);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn doctor_rejects_arbitrary_json_as_configuration() {
    let root =
        std::env::temp_dir().join(format!("controlla doctor invalid {}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("runtime.json"),
        r#"{"process_reachable":true,"authenticated":true}"#,
    )
    .unwrap();
    let report = run_doctor(&root);
    assert_eq!(report["configured"], "Failed");
    assert!(
        report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("schema_version"))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn launcher_uses_only_its_sibling_binary() {
    let root = std::env::temp_dir().join(format!("controlla package {}", std::process::id()));
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let launcher = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/chrome-controlla/bin/controlla");
    let target = bin.join("controlla");
    std::fs::copy(launcher, &target).unwrap();
    let executable = bin.join("controlla-core");
    std::fs::copy(env!("CARGO_BIN_EXE_controlla"), &executable).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(&target)
        .env("PATH", "")
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}
