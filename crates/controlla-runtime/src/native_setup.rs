use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const HOST_NAME: &str = "chrome_controlla_bridge";

fn current_host_record_path() -> Result<PathBuf, String> {
    Ok(home()?.join(".chrome-controlla/native-host-current.json"))
}

pub fn publish_current_host(host_id: &str) -> Result<(), String> {
    let path = current_host_record_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
    }
    let temp = path.with_extension("json.tmp");
    let mut value = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter(|current| current["host_id"] == host_id)
        .unwrap_or_else(|| json!({}));
    value["schema_version"] = json!(1);
    value["host_id"] = json!(host_id);
    value["pid"] = json!(std::process::id());
    value["heartbeat_unix"] = json!(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs()
    );
    fs::write(
        &temp,
        serde_json::to_vec(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    fs::rename(temp, path).map_err(|e| e.to_string())
}

/// Keep bounded, non-sensitive create-tab observability beside the host lease.
/// This is intentionally request-ID/timestamp only so a stalled ownerless open
/// can be diagnosed without persisting URLs, credentials, or page content.
pub fn record_create_event(host_id: &str, request_id: &str, field: &str) -> Result<(), String> {
    if request_id.len() != 32 || !request_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid native create request ID".into());
    }
    if !matches!(field, "dispatch" | "response" | "error") {
        return Err("invalid native create event".into());
    }
    let path = current_host_record_path()?;
    let mut value = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter(|current| current["host_id"] == host_id)
        .unwrap_or_else(|| json!({"schema_version":1,"host_id":host_id,"pid":std::process::id()}));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    value["heartbeat_unix"] = json!(now.as_secs());
    value[format!("last_create_{field}_request_id")] = json!(request_id);
    value[format!("last_create_{field}_unix_ms")] =
        json!(now.as_millis().min(u64::MAX as u128) as u64);
    let temp = path.with_extension("json.tmp");
    fs::write(
        &temp,
        serde_json::to_vec(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    fs::rename(temp, path).map_err(|e| e.to_string())
}

pub fn current_host_id() -> Result<String, String> {
    let path = current_host_record_path()?;
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > 1024 {
        return Err("current native host record is invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("current native host record permissions are too broad".into());
        }
    }
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let seen = value["heartbeat_unix"]
        .as_u64()
        .ok_or("current native host record has no heartbeat")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    if now.saturating_sub(seen) > 15 || seen > now + 2 {
        return Err("current native host record is stale".into());
    }
    let host_id = value["host_id"]
        .as_str()
        .ok_or("current native host record has no host ID")?;
    pairing_path(host_id)?;
    Ok(host_id.to_owned())
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "HOME or USERPROFILE is required for Chrome native messaging".to_owned())
}

pub fn pairing_path(host_id: &str) -> Result<PathBuf, String> {
    if host_id.len() != 32 || !host_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid native host ID".into());
    }
    Ok(home()?.join(format!(".chrome-controlla/native-pairing-{host_id}.json")))
}

pub fn pairing_request_path(host_id: &str, request_id: &str) -> Result<PathBuf, String> {
    if request_id.len() != 32 || !request_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid native pairing request ID".into());
    }
    let path = pairing_path(host_id)?;
    Ok(path.with_file_name(format!("native-pairing-{host_id}-{request_id}.json")))
}

pub fn new_pairing_request_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "cannot generate native pairing ID")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn remove_pairing_request(host_id: &str, request_id: &str) -> Result<(), String> {
    let path = pairing_request_path(host_id, request_id)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn read_tabs() -> Result<serde_json::Value, String> {
    let path = home()?.join(".chrome-controlla/native-tabs.json");
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > 16_000_000 {
        return Err("native tab inventory is invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("native tab inventory permissions are too broad".into());
        }
    }
    let body: serde_json::Value =
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let seen = body["last_seen_unix"]
        .as_u64()
        .ok_or("tab inventory is missing a timestamp")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    if seen > now + 2 || now.saturating_sub(seen) > 15 {
        return Err("native tab inventory is stale".into());
    }
    if !body["tabs"]
        .as_array()
        .is_some_and(|tabs| tabs.len() <= 5000)
    {
        return Err("native tab inventory has invalid tabs".into());
    }
    pairing_path(
        body["host_id"]
            .as_str()
            .ok_or("native tab inventory has no host ID")?,
    )?;
    Ok(body)
}

/// Ask the already-connected native host to create an owned Chrome tab before
/// pairing. No existing user tab is attached or used as a command transport.
pub async fn create_owned_tab(
    host_id: &str,
    url: &str,
    idempotency_key: &str,
    active: bool,
    create_background_window: bool,
    target_window_id: Option<u64>,
    target_window_policy: Option<&str>,
) -> Result<serde_json::Value, String> {
    let live_host_id = current_host_id().unwrap_or_else(|_| host_id.to_owned());
    let base = pairing_path(&live_host_id)?;
    if idempotency_key.len() < 8
        || idempotency_key.len() > 256
        || idempotency_key.chars().any(char::is_control)
    {
        return Err("idempotency_key must be 8..256 printable characters".into());
    }
    let request_id = new_pairing_request_id()?;
    let dir = base.parent().ok_or("invalid native bridge directory")?;
    let request = dir.join(format!("native-create-{live_host_id}-{request_id}.json"));
    let response = dir.join(format!(
        "native-create-result-{live_host_id}-{request_id}.json"
    ));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let body = json!({"request_id":request_id,"url":url,"idempotency_key":idempotency_key,
        "active":active,"create_background_window":create_background_window,
        "target_window_id":target_window_id,"target_window_policy":target_window_policy,
        "expires_at_unix":now + 20});
    let temp = dir.join(format!("native-create-{host_id}-{request_id}.tmp"));
    fs::write(
        &temp,
        serde_json::to_vec(&body).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    fs::rename(temp, &request).map_err(|error| error.to_string())?;
    let mut created = None;
    for _ in 0..400 {
        if created.is_none()
            && let Ok(bytes) = fs::read(&response)
        {
            let _ = fs::remove_file(&response);
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            if value["request_id"] != request_id {
                return Err("native create response ID changed".into());
            }
            if let Some(error) = value["error"].as_str() {
                return Err(error.to_owned());
            }
            if value["result"]["tab_id"].as_str().is_none() {
                return Err("native create response omitted tab ID".into());
            }
            created = Some(value["result"].clone());
        }
        if let Some(result) = &created {
            let tab_id = result["tab_id"]
                .as_str()
                .ok_or("native create response omitted tab ID")?;
            // Pairing still requires a fresh extension inventory that includes
            // the new document identity; do not trust only the create ACK.
            if let Ok(snapshot) = read_tabs()
                && snapshot["host_id"] == live_host_id
                && snapshot["tabs"].as_array().is_some_and(|tabs| {
                    tabs.iter().any(|tab| {
                        tab["id"].as_u64().map(|id| id.to_string()).as_deref() == Some(tab_id)
                            && tab["document_id"] == result["document_id"]
                    })
                })
            {
                return Ok(result.clone());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let _ = fs::remove_file(request);
    Err(if let Some(result) = created {
        format!(
            "created tab {} is not in a fresh native inventory; inspect it before retrying the same key",
            result["tab_id"]
        )
    } else {
        "native tab creation timed out; inspect the browser before retrying with the same idempotency_key".into()
    })
}

pub fn bootstrap_tab_id(snapshot: &serde_json::Value) -> Option<String> {
    snapshot["tabs"].as_array()?.iter().find_map(|tab| {
        let id = u32::try_from(tab["id"].as_u64()?).ok()?;
        let url = tab["url"].as_str()?;
        let document_id = tab["document_id"].as_str()?;
        if (url.starts_with("http://") || url.starts_with("https://")) && !document_id.is_empty() {
            Some(id.to_string())
        } else {
            None
        }
    })
}

// shortcut: one internal caller, group pairing fields if another caller needs this API.
#[allow(clippy::too_many_arguments)]
pub fn write_pairing(
    host_id: &str,
    endpoint: &str,
    token: &str,
    tab_ids: &[String],
    request_id: &str,
    expected_urls: &serde_json::Map<String, serde_json::Value>,
    expected_document_ids: &serde_json::Map<String, serde_json::Value>,
    unrestricted: bool,
) -> Result<(), String> {
    let path = pairing_request_path(host_id, request_id)?;
    let dir = path.parent().ok_or("invalid pairing path")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let now = since_epoch.as_secs();
    let ids = tab_ids
        .iter()
        .map(|id| id.parse::<u32>().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let body = json!({
        "endpoint":endpoint,
        "token":token,
        "tab_ids":ids,
        "request_id":request_id,
        "expected_urls":expected_urls,
        "expected_document_ids":expected_document_ids,
        "unrestricted":unrestricted,
        "expires_at_unix":now + 300,
    });
    let temp = dir.join(format!(
        "native-pairing.{}.{}.tmp",
        std::process::id(),
        since_epoch.as_nanos()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    file.write_all(body.to_string().as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&temp, path).map_err(|e| e.to_string())
}

pub fn install_host(extension_id: &str) -> Result<PathBuf, String> {
    if !valid_extension_id(extension_id) {
        return Err("extension ID must be exactly 32 characters from a-p".into());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let host_dir = host_dir()?;
    fs::create_dir_all(&host_dir).map_err(|e| e.to_string())?;
    let path = host_dir.join(format!("{HOST_NAME}.json"));
    let manifest = json!({
        "name":HOST_NAME,
        "description":"Chrome Controlla local browser bridge",
        "path":executable,
        "type":"stdio",
        "allowed_origins":[format!("chrome-extension://{extension_id}/")],
    });
    fs::write(&path, format!("{}\n", manifest)).map_err(|e| e.to_string())?;
    #[cfg(target_os = "windows")]
    {
        let key = r"HKCU\Software\Google\Chrome\NativeMessagingHosts\chrome_controlla_bridge";
        let result = Command::new("reg.exe")
            .args(["add", key, "/ve", "/t", "REG_SZ", "/d"])
            .arg(path.as_os_str())
            .args(["/f"])
            .output()
            .map_err(|e| format!("cannot register Chrome native host with reg.exe: {e}"))?;
        if !result.status.success() {
            return Err(format!(
                "Chrome native host registry registration failed: {}",
                String::from_utf8_lossy(&result.stderr).trim()
            ));
        }
    }
    if !host_registered() {
        return Err("Chrome native host registration was written but failed verification".into());
    }
    Ok(path)
}

fn host_dir() -> Result<PathBuf, String> {
    if cfg!(target_os = "macos") {
        Ok(home()?.join("Library/Application Support/Google/Chrome/NativeMessagingHosts"))
    } else if cfg!(target_os = "linux") {
        Ok(home()?.join(".config/google-chrome/NativeMessagingHosts"))
    } else if cfg!(target_os = "windows") {
        let local = std::env::var_os("LOCALAPPDATA")
            .or_else(|| {
                std::env::var_os("USERPROFILE").map(|profile| {
                    PathBuf::from(profile)
                        .join("AppData/Local")
                        .into_os_string()
                })
            })
            .map(PathBuf::from)
            .ok_or_else(|| {
                "LOCALAPPDATA or USERPROFILE is required for Chrome native messaging".to_owned()
            })?;
        Ok(local.join("Google/Chrome/NativeMessagingHosts"))
    } else {
        Err("native host registration is unsupported on this platform".into())
    }
}

fn valid_extension_id(extension_id: &str) -> bool {
    extension_id.len() == 32 && extension_id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

pub fn host_registered() -> bool {
    let Ok(path) = host_dir().map(|dir| dir.join(format!("{HOST_NAME}.json"))) else {
        return false;
    };
    #[cfg(target_os = "windows")]
    {
        let key = r"HKCU\Software\Google\Chrome\NativeMessagingHosts\chrome_controlla_bridge";
        let Ok(result) = Command::new("reg.exe").args(["query", key, "/ve"]).output() else {
            return false;
        };
        if !result.status.success()
            || registry_default(&String::from_utf8_lossy(&result.stdout)).as_deref()
                != Some(path.to_string_lossy().as_ref())
        {
            return false;
        }
    }
    let Ok(body) = fs::read(&path) else {
        return false;
    };
    let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return false;
    };
    let Some(executable) = manifest["path"].as_str() else {
        return false;
    };
    let executable = PathBuf::from(executable);
    let executable_exists = executable.is_file();
    #[cfg(unix)]
    let executable_exists = executable_exists && {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(&executable).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    };
    manifest["name"] == HOST_NAME
        && manifest["type"] == "stdio"
        && executable_exists
        && manifest["allowed_origins"]
            .as_array()
            .is_some_and(|origins| {
                origins.len() == 1
                    && origins[0].as_str().is_some_and(|origin| {
                        origin
                            .strip_prefix("chrome-extension://")
                            .and_then(|id| id.strip_suffix('/'))
                            .is_some_and(valid_extension_id)
                    })
            })
}

#[cfg(target_os = "windows")]
fn registry_default(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (_, value) = line.split_once("REG_SZ")?;
        Some(value.trim().to_owned())
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn cold_open_chooses_a_stable_web_tab_from_the_native_inventory() {
        let inventory = serde_json::json!({"tabs":[
            {"id":10,"url":"https://loading.example/"},
            {"id":11,"url":"chrome://settings/","document_id":"x"},
            {"id":12,"url":"https://ready.example/","document_id":"document-12"}
        ]});
        assert_eq!(super::bootstrap_tab_id(&inventory), Some("12".to_owned()));
        assert_eq!(
            super::bootstrap_tab_id(&serde_json::json!({"tabs":[]})),
            None
        );
    }

    #[test]
    fn extension_id_must_be_exact() {
        assert!(!super::valid_extension_id("x"));
        assert!(!super::valid_extension_id(
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
        ));
        assert!(super::valid_extension_id(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn registry_default_reads_manifest_path() {
        assert_eq!(
            super::registry_default("    (Default)    REG_SZ    C:\\host.json\n"),
            Some("C:\\host.json".into())
        );
        assert_eq!(super::registry_default("ERROR: key not found"), None);
    }
}
