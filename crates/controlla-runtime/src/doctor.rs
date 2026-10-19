use crate::capability::{DiagnosticState as State, DoctorReport};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROBE_TIMEOUT: Duration = Duration::from_millis(400);
const MAX_RESPONSE_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema_version: u32,
    service_pid: u32,
    loopback_addr: String,
    bearer_token: String,
    principal_id: String,
    heartbeat_unix_seconds: u64,
    heartbeat_max_age_seconds: u64,
    policy_revision: u64,
}

#[derive(Deserialize)]
struct HealthReply {
    authenticated: bool,
    principal_id: String,
    policy_revision: u64,
}

#[derive(Deserialize)]
struct StdioRuntime {
    schema_version: u32,
    service_pid: u32,
    heartbeat_unix_seconds: u64,
    binary_version: String,
    process_generation: Option<String>,
    extension_build_id: Option<String>,
}

fn process_generation() -> &'static str {
    static GENERATION: OnceLock<String> = OnceLock::new();
    GENERATION.get_or_init(|| {
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("{}-{started}", std::process::id())
    })
}

pub fn publish_stdio_runtime(state_dir: &Path, version: &str) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let payload = serde_json::json!({"schema_version":2,"service_pid":std::process::id(),
        "heartbeat_unix_seconds":now,"binary_version":version,
        "process_generation":process_generation(),
        "extension_build_id":option_env!("CONTROLLA_EXTENSION_BUILD_ID").unwrap_or("unpaired")});
    let path = state_dir.join("stdio-runtime.json");
    let temp = state_dir.join("stdio-runtime.json.tmp");
    std::fs::write(
        &temp,
        serde_json::to_vec(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    std::fs::rename(temp, path).map_err(|error| error.to_string())
}

fn probe_stdio_runtime(state_dir: &Path, report: &mut DoctorReport) {
    let path = state_dir.join("stdio-runtime.json");
    let Ok(bytes) = std::fs::read(&path) else {
        report.errors.push(format!(
            "no HTTP or stdio runtime record in {}",
            state_dir.display()
        ));
        return;
    };
    let Ok(runtime) = serde_json::from_slice::<StdioRuntime>(&bytes) else {
        report.configured = State::Failed;
        report.errors.push("stdio runtime record is invalid".into());
        return;
    };
    if !(runtime.schema_version == 1 || runtime.schema_version == 2)
        || runtime.service_pid == 0
        || runtime.binary_version != report.binary_version
    {
        report.configured = State::Failed;
        report
            .errors
            .push("stdio runtime record does not match this binary".into());
        return;
    }
    report.configured = State::Healthy;
    report.process_generation = runtime.process_generation;
    report.extension_build_id = Some(runtime.extension_build_id.unwrap_or_else(|| {
        option_env!("CONTROLLA_EXTENSION_BUILD_ID")
            .unwrap_or("unpaired")
            .to_owned()
    }));
    let (process, error) = process_probe(runtime.service_pid);
    report.process = process;
    if let Some(error) = error {
        report.errors.push(error);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    report.heartbeat_age_seconds = now.checked_sub(runtime.heartbeat_unix_seconds);
    report.heartbeat = if report.heartbeat_age_seconds.is_some_and(|age| age <= 15) {
        State::Healthy
    } else {
        State::Failed
    };
    match crate::native_setup::read_tabs() {
        Ok(snapshot) => {
            report.transport = State::Healthy;
            report.principal_correlation = snapshot["host_id"].as_str().map(str::to_owned);
            report.bridge_host_id = snapshot["host_id"].as_str().map(str::to_owned);
            report.bridge_tab_count = snapshot["tabs"].as_array().map(Vec::len);
        }
        Err(error) => {
            report.transport = State::Failed;
            report
                .errors
                .push(format!("native bridge probe failed: {error}"));
        }
    }
    // A fresh inventory proves the extension bridge, not that the MCP client
    // loaded this server or that a tool round trip succeeded.
    report.live_round_trip = State::Unknown;
}

#[cfg(test)]
mod stdio_tests {
    use super::*;

    #[test]
    fn stdio_runtime_record_gives_doctor_process_and_heartbeat_evidence() {
        let dir =
            std::env::temp_dir().join(format!("controlla-stdio-doctor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        publish_stdio_runtime(&dir, "test-version").unwrap();
        let report = report(&dir, "test-version");
        assert_eq!(report.configured, State::Healthy);
        assert_eq!(report.process, State::Healthy);
        assert_eq!(report.heartbeat, State::Healthy);
        assert_eq!(report.live_round_trip, State::Unknown);
        assert!(
            report.process_generation.is_some(),
            "doctor must expose the live worker generation"
        );
        assert!(
            report.extension_build_id.is_some(),
            "doctor must expose the extension build identity"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

pub fn report(state_dir: &Path, version: &str) -> DoctorReport {
    report_for_client(state_dir, version, None)
}

pub fn report_for_client(state_dir: &Path, version: &str, client: Option<&str>) -> DoctorReport {
    let config_path = state_dir.join("runtime.json");
    let mut errors = Vec::new();
    let mut report = DoctorReport {
        configured: State::Unknown,
        process: State::Unknown,
        binary_version: version.to_owned(),
        binary_hash: binary_hash(&mut errors),
        state_dir: state_dir.to_string_lossy().into_owned(),
        principal_correlation: None,
        transport: State::Unknown,
        authentication: State::Unknown,
        heartbeat: State::Unknown,
        heartbeat_age_seconds: None,
        process_generation: None,
        extension_build_id: Some(
            option_env!("CONTROLLA_EXTENSION_BUILD_ID")
                .unwrap_or("unpaired")
                .to_owned(),
        ),
        bridge_host_id: None,
        bridge_tab_count: None,
        live_round_trip: State::Unknown,
        policy_revision: None,
        expected_mcp_surface: "v3-compact".to_owned(),
        requested_client: client.map(str::to_owned),
        loaded_client_tool_state: State::Unknown,
        client_tool_acknowledgement: format!(
            "unverified: server health does not prove {} loaded its tools; no client startup acknowledgement is available",
            client.unwrap_or("the MCP client")
        ),
        errors,
    };

    let bytes = match std::fs::read(&config_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            if error.kind() == std::io::ErrorKind::NotFound {
                probe_stdio_runtime(state_dir, &mut report);
                return report;
            }
            report.errors.push(format!(
                "configuration read failed: {}: {error}",
                config_path.display()
            ));
            return report;
        }
    };
    let config: Config = match serde_json::from_slice(&bytes) {
        Ok(config) => config,
        Err(error) => {
            report.configured = State::Failed;
            report
                .errors
                .push(format!("configuration schema invalid: {error}"));
            return report;
        }
    };
    let addr: SocketAddr = match config.loopback_addr.parse::<SocketAddr>() {
        Ok(addr) if addr.ip().is_loopback() => addr,
        Ok(_) => {
            report.configured = State::Failed;
            report
                .errors
                .push("configuration loopback_addr must be a loopback socket address".to_owned());
            return report;
        }
        Err(error) => {
            report.configured = State::Failed;
            report
                .errors
                .push(format!("configuration loopback_addr invalid: {error}"));
            return report;
        }
    };
    if config.schema_version != 1
        || config.service_pid == 0
        || config.bearer_token.is_empty()
        || !config
            .bearer_token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/=".contains(&byte))
        || config.principal_id.is_empty()
        || config.heartbeat_max_age_seconds == 0
        || config.heartbeat_max_age_seconds > 86_400
        || addr.port() == 0
    {
        report.configured = State::Failed;
        report
            .errors
            .push("configuration values are outside supported bounds".to_owned());
        return report;
    }
    report.configured = State::Healthy;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if config.heartbeat_unix_seconds > now {
        report.heartbeat = State::Failed;
        report
            .errors
            .push("configured heartbeat timestamp is in the future".to_owned());
    } else {
        let age = now - config.heartbeat_unix_seconds;
        report.heartbeat_age_seconds = Some(age);
        report.heartbeat = if age <= config.heartbeat_max_age_seconds {
            State::Healthy
        } else {
            State::Failed
        };
    }

    let (process, process_error) = process_probe(config.service_pid);
    report.process = process;
    if let Some(error) = process_error {
        report.errors.push(error);
    }

    let probe = health_probe(addr, &config);
    report.transport = probe.transport;
    report.authentication = probe.authentication;
    report.live_round_trip = probe.live_round_trip;
    report.policy_revision = probe.policy_revision;
    report.principal_correlation = probe.principal_correlation;
    report.errors.extend(probe.errors);
    report
}

struct ProbeResult {
    transport: State,
    authentication: State,
    live_round_trip: State,
    policy_revision: Option<u64>,
    principal_correlation: Option<String>,
    errors: Vec<String>,
}

fn health_probe(addr: SocketAddr, config: &Config) -> ProbeResult {
    let mut result = ProbeResult {
        transport: State::Failed,
        authentication: State::Unknown,
        live_round_trip: State::Failed,
        policy_revision: None,
        principal_correlation: None,
        errors: Vec::new(),
    };
    let mut stream = match TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) {
        Ok(stream) => stream,
        Err(error) => {
            result
                .errors
                .push(format!("loopback health connect failed: {error}"));
            return result;
        }
    };
    let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    let request = format!(
        "GET /health HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
        config.bearer_token
    );
    if let Err(error) = stream.write_all(request.as_bytes()) {
        result
            .errors
            .push(format!("loopback health request failed: {error}"));
        return result;
    }
    let mut response = Vec::new();
    if let Err(error) = stream
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut response)
    {
        result.transport = State::Healthy;
        result
            .errors
            .push(format!("loopback health response read failed: {error}"));
        return result;
    }
    result.transport = State::Healthy;
    if response.len() > MAX_RESPONSE_BYTES {
        result
            .errors
            .push("loopback health response exceeded 16 KiB".to_owned());
        return result;
    }
    let Some(split) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        result
            .errors
            .push("loopback health response has invalid HTTP framing".to_owned());
        return result;
    };
    let headers = String::from_utf8_lossy(&response[..split]);
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok());
    if status != Some(200) {
        result.authentication = if matches!(status, Some(401 | 403)) {
            State::Failed
        } else {
            State::Unknown
        };
        result.errors.push(format!(
            "loopback health returned HTTP {}",
            status.map_or("invalid".to_owned(), |value| value.to_string())
        ));
        return result;
    }
    let body = &response[split + 4..];
    let reply: HealthReply = match serde_json::from_slice(body) {
        Ok(reply) => reply,
        Err(error) => {
            result
                .errors
                .push(format!("loopback health body invalid: {error}"));
            return result;
        }
    };
    result.policy_revision = Some(reply.policy_revision);
    result.principal_correlation = Some(redacted_principal(&reply.principal_id));
    let identity_matches = reply.principal_id == config.principal_id;
    result.authentication = if reply.authenticated && identity_matches {
        State::Healthy
    } else {
        State::Failed
    };
    if !reply.authenticated {
        result
            .errors
            .push("loopback health did not authenticate the configured token".to_owned());
    }
    if !identity_matches {
        result
            .errors
            .push("loopback health principal did not match configured principal".to_owned());
    }
    if reply.policy_revision != config.policy_revision {
        result
            .errors
            .push("loopback health policy revision differs from configuration".to_owned());
    }
    result.live_round_trip = if reply.authenticated
        && identity_matches
        && reply.policy_revision == config.policy_revision
    {
        State::Healthy
    } else {
        State::Failed
    };
    result
}

#[cfg(unix)]
fn process_probe(pid: u32) -> (State, Option<String>) {
    process_probe_with(pid, |safe_pid| {
        Command::new("/bin/kill")
            .arg("-0")
            .arg(safe_pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
    })
}

#[cfg(unix)]
fn process_probe_with(
    pid: u32,
    probe: impl FnOnce(i32) -> std::io::Result<bool>,
) -> (State, Option<String>) {
    let Ok(pid) = i32::try_from(pid) else {
        return (
            State::Failed,
            Some("configured process ID exceeds the supported positive pid_t range".to_owned()),
        );
    };
    if pid <= 0 {
        return (
            State::Failed,
            Some("configured process ID must be positive".to_owned()),
        );
    }
    match probe(pid) {
        Ok(true) => (State::Healthy, None),
        Ok(false) => (
            State::Failed,
            Some("configured local process is not alive or not probeable".to_owned()),
        ),
        Err(error) => (
            State::Unknown,
            Some(format!("local process probe unavailable: {error}")),
        ),
    }
}

#[cfg(not(unix))]
fn process_probe(pid: u32) -> (State, Option<String>) {
    let _ = pid;
    (
        State::Unknown,
        Some("local process probe unavailable on this operating system".to_owned()),
    )
}

fn binary_hash(errors: &mut Vec<String>) -> Option<String> {
    let path = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            errors.push(format!("binary path unavailable: {error}"));
            return None;
        }
    };
    match std::fs::read(path) {
        Ok(bytes) => Some(hex(Sha256::digest(bytes).iter().copied())),
        Err(error) => {
            errors.push(format!("binary SHA-256 unavailable: {error}"));
            None
        }
    }
}

fn redacted_principal(principal: &str) -> String {
    let digest = Sha256::digest(principal.as_bytes());
    format!("sha256:{}", hex(digest.iter().copied().take(8)))
}

fn hex(bytes: impl Iterator<Item = u8>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::new();
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::{State, process_probe_with};
    use std::cell::Cell;

    #[test]
    fn oversized_pid_is_rejected_before_os_probe() {
        let invoked = Cell::new(false);
        let (state, error) = process_probe_with(i32::MAX as u32 + 1, |_| {
            invoked.set(true);
            Ok(true)
        });
        assert_eq!(state, State::Failed);
        assert!(!invoked.get());
        assert!(error.unwrap().contains("pid_t range"));
    }
}
