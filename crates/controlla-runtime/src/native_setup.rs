use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const HOST_NAME: &str = "chrome_controlla_bridge";

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
    if !metadata.file_type().is_file() || metadata.len() > 256_000 {
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
        .is_some_and(|tabs| tabs.len() <= 100)
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
