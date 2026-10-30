//! Chrome native-messaging transport for the shared extension provider.

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinSet;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

const MAX_MESSAGE: usize = 1_048_576;

/// Reports desktop input capability without probing or sending input.
pub async fn desktop_capability_report() -> Value {
    crate::desktop_platform::capability_report().await
}

const MAX_SCREENSHOT_CHARS: usize = 8 * 1024 * 1024;
const SCREENSHOT_CHUNK_CHARS: usize = 512 * 1024;
const MAX_SCREENSHOT_EDGE: u64 = 4096;
const MAX_SCREENSHOT_PIXELS: u64 = 16_777_216;
const MAX_SCREENSHOT_CHUNKS: usize = 16;
const MAX_SCREENSHOT_TRANSFERS: usize = 4;
const SCREENSHOT_TRANSFER_TTL: Duration = Duration::from_secs(15);

struct ScreenshotTransfer {
    lease_id: String,
    total_chunks: usize,
    total_chars: usize,
    base64_sha256: String,
    visual_evidence: Value,
    chunks: Vec<Option<String>>,
    started_at: tokio::time::Instant,
}

#[derive(Default)]
struct ScreenshotAssembler {
    transfers: HashMap<(String, u64, String), ScreenshotTransfer>,
}

impl ScreenshotAssembler {
    fn expire(&mut self) {
        let now = tokio::time::Instant::now();
        self.transfers
            .retain(|_, transfer| transfer.started_at + SCREENSHOT_TRANSFER_TTL > now);
    }

    fn abort(&mut self, message: &Value) -> Result<(), &'static str> {
        let id = message["id"]
            .as_u64()
            .ok_or("screenshot abort ID is invalid")?;
        let lease = message["lease_id"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or("screenshot abort lease is missing")?;
        let transfer = message["transfer_id"]
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or("screenshot abort transfer ID is invalid")?;
        self.transfers
            .remove(&(lease.to_owned(), id, transfer.to_owned()));
        Ok(())
    }

    fn push(&mut self, message: &Value) -> Result<Option<Value>, String> {
        self.expire();
        let now = tokio::time::Instant::now();
        let id = message["id"]
            .as_u64()
            .ok_or("screenshot chunk id is invalid")?;
        let lease_id = message["lease_id"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or("screenshot chunk lease is missing")?
            .to_owned();
        let transfer_id = message["transfer_id"]
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or("screenshot transfer ID is invalid")?
            .to_owned();
        let index = message["index"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("screenshot chunk index is invalid")?;
        let total_chunks = message["total_chunks"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| (2..=MAX_SCREENSHOT_CHUNKS).contains(value))
            .ok_or("screenshot chunk count is out of bounds")?;
        let total_chars = message["total_chars"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| (*value > SCREENSHOT_CHUNK_CHARS) && (*value <= MAX_SCREENSHOT_CHARS))
            .ok_or("screenshot transfer length is out of bounds")?;
        let base64_sha256 = message["base64_sha256"]
            .as_str()
            .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or("screenshot transfer digest is invalid")?
            .to_ascii_lowercase();
        let visual_evidence = message["visual_evidence"].clone();
        let dimensions = &visual_evidence["dimensions"];
        if !visual_evidence.is_object()
            || visual_evidence["target_id"]
                .as_str()
                .is_none_or(str::is_empty)
            || visual_evidence["target_id"]
                .as_str()
                .is_some_and(|value| value.len() > 32)
            || visual_evidence["document_id"]
                .as_str()
                .is_none_or(str::is_empty)
            || visual_evidence["document_id"]
                .as_str()
                .is_some_and(|value| value.len() > 256)
            || visual_evidence["url"]
                .as_str()
                .is_none_or(|value| value.len() > 2048)
            || visual_evidence["captured_at_ms"].as_u64().is_none()
            || visual_evidence["capture_started_at_ms"].as_u64().is_none()
            || visual_evidence["target_lifecycle"].as_str() != Some("active")
            || visual_evidence["render_revision"]
                .as_str()
                .is_some_and(|revision| {
                    revision.is_empty()
                        || revision.len() > 512
                        || !revision.starts_with(&format!(
                            "{}:",
                            visual_evidence["document_id"].as_str().unwrap_or_default()
                        ))
                })
            || dimensions["width"]
                .as_u64()
                .is_none_or(|value| value == 0 || value > MAX_SCREENSHOT_EDGE)
            || dimensions["height"]
                .as_u64()
                .is_none_or(|value| value == 0 || value > MAX_SCREENSHOT_EDGE)
            || dimensions["width"]
                .as_u64()
                .unwrap_or_default()
                .saturating_mul(dimensions["height"].as_u64().unwrap_or_default())
                > MAX_SCREENSHOT_PIXELS
            || visual_evidence["visual_revision"]
                .as_str()
                .is_none_or(|value| {
                    value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            || visual_evidence["image_sha256"]
                .as_str()
                .is_none_or(|value| {
                    value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        {
            return Err("screenshot visual evidence is missing or inconsistent".into());
        }
        let chunk = message["chunk"]
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= SCREENSHOT_CHUNK_CHARS)
            .ok_or("screenshot chunk is empty or oversized")?;
        if index >= total_chunks {
            return Err("screenshot chunk index exceeds chunk count".into());
        }
        let expected_chunks = total_chars.div_ceil(SCREENSHOT_CHUNK_CHARS);
        let expected_len = if index + 1 == expected_chunks {
            total_chars - index * SCREENSHOT_CHUNK_CHARS
        } else {
            SCREENSHOT_CHUNK_CHARS
        };
        if total_chunks != expected_chunks || chunk.len() != expected_len {
            return Err("screenshot chunk geometry is inconsistent".into());
        }
        let key = (lease_id.clone(), id, transfer_id.clone());
        if !self.transfers.contains_key(&key) && self.transfers.len() >= MAX_SCREENSHOT_TRANSFERS {
            return Err("too many concurrent screenshot transfers".into());
        }
        let transfer = self
            .transfers
            .entry(key.clone())
            .or_insert_with(|| ScreenshotTransfer {
                lease_id,
                total_chunks,
                total_chars,
                base64_sha256: base64_sha256.clone(),
                visual_evidence: visual_evidence.clone(),
                chunks: vec![None; total_chunks],
                started_at: now,
            });
        if transfer.total_chunks != total_chunks
            || transfer.total_chars != total_chars
            || transfer.base64_sha256 != base64_sha256
            || transfer.visual_evidence != visual_evidence
            || transfer.chunks[index].is_some()
        {
            self.transfers.remove(&key);
            return Err("screenshot chunk metadata changed or index was repeated".into());
        }
        transfer.chunks[index] = Some(chunk.to_owned());
        if transfer.chunks.iter().any(Option::is_none) {
            return Ok(None);
        }
        let transfer = self
            .transfers
            .remove(&key)
            .expect("completed transfer exists");
        let data = transfer.chunks.into_iter().flatten().collect::<String>();
        if data.len() != transfer.total_chars {
            return Err("screenshot transfer length did not match its declaration".into());
        }
        let digest = Sha256::digest(data.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if digest != transfer.base64_sha256 {
            return Err("screenshot transfer digest mismatch".into());
        }
        let png = base64::engine::general_purpose::STANDARD
            .decode(&data)
            .map_err(|_| "screenshot payload is not valid base64")?;
        if png.len() < 24
            || png.get(..8) != Some(b"\x89PNG\r\n\x1a\n")
            || png.get(8..12) != Some(13_u32.to_be_bytes().as_slice())
            || png.get(12..16) != Some(b"IHDR")
        {
            return Err("screenshot payload is not a PNG image".into());
        }
        let image_digest = Sha256::digest(&png)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let width = u32::from_be_bytes(png[16..20].try_into().expect("PNG width header"));
        let height = u32::from_be_bytes(png[20..24].try_into().expect("PNG height header"));
        if image_digest
            != transfer.visual_evidence["image_sha256"]
                .as_str()
                .unwrap_or_default()
            || image_digest
                != transfer.visual_evidence["visual_revision"]
                    .as_str()
                    .unwrap_or_default()
            || width == 0
            || u64::from(width) > MAX_SCREENSHOT_EDGE
            || height == 0
            || u64::from(height) > MAX_SCREENSHOT_EDGE
            || u64::from(width).saturating_mul(u64::from(height)) > MAX_SCREENSHOT_PIXELS
            || transfer.visual_evidence["dimensions"]["width"].as_u64() != Some(width.into())
            || transfer.visual_evidence["dimensions"]["height"].as_u64() != Some(height.into())
        {
            return Err("screenshot image digest or dimensions mismatch".into());
        }
        Ok(Some(
            json!({"type":"result","id":id,"lease_id":transfer.lease_id,"result":{"data":data,"visual_evidence":transfer.visual_evidence}}),
        ))
    }
}

#[derive(Clone, Debug, Deserialize)]
struct Pairing {
    endpoint: String,
    token: String,
    tab_ids: Vec<u32>,
    request_id: String,
    #[serde(default)]
    expected_urls: std::collections::BTreeMap<u32, String>,
    #[serde(default)]
    expected_document_ids: std::collections::BTreeMap<u32, String>,
    #[serde(default)]
    unrestricted: bool,
    expires_at_unix: u64,
}

#[derive(Debug, Deserialize)]
struct NativeReply {
    #[serde(rename = "type")]
    kind: String,
    targets: Option<Vec<String>>,
    extension_version: Option<String>,
    extension_build_id: Option<String>,
    document_identity: Option<bool>,
    #[serde(default)]
    batch_execution: bool,
    #[serde(default)]
    batch_deadline: bool,
    #[serde(default)]
    protocol_version: u32,
}

pub async fn run() -> Result<(), String> {
    let mut host_bytes = [0u8; 16];
    getrandom::fill(&mut host_bytes).map_err(|_| "cannot generate native host ID")?;
    let host_id = host_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    crate::native_setup::publish_current_host(&host_id)?;
    let pairing_path = crate::native_setup::pairing_path(&host_id)?;
    let mut input = tokio::io::stdin();
    let output = Arc::new(Mutex::new(tokio::io::stdout()));
    write_shared_message(&output, &json!({"type":"ready"})).await?;
    let (native_tx, mut native_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        loop {
            let message = read_message::<_, Value>(&mut input).await;
            let done = message.is_err() || native_tx.send(message).await.is_err();
            if done {
                break;
            }
        }
    });
    let mut consumed = HashSet::new();
    let mut active = HashMap::<String, mpsc::Sender<Result<Value, String>>>::new();
    let mut pairing_tasks = JoinSet::<(String, Result<(), String>, bool)>::new();
    let mut extension_protocol_version = 0_u32;
    let mut inventory_tabs = Vec::<Value>::new();
    let mut inventory_in_progress = false;
    let mut inventory_started_at = None::<tokio::time::Instant>;
    // Chrome events mark the inventory dirty immediately. Keep the safety
    // poll below the 15-second freshness bound enforced by read_tabs(), so a
    // quiet browser cannot become unusable between missed MV3 wakeups.
    let mut inventory_poll = tokio::time::interval(Duration::from_secs(5));
    // Pairing requests are tiny local files. Check often enough to keep cold
    // pairing responsive without adding a filesystem watcher dependency.
    let mut pairing_poll = tokio::time::interval(Duration::from_millis(75));
    loop {
        tokio::select! {
            _ = inventory_poll.tick() => {
                if let Err(error) = crate::native_setup::publish_current_host(&host_id) {
                    eprintln!("Native host heartbeat failed: {error}");
                }
                // A service worker can be restarting while the native port is
                // already open. If the first inventory request is lost at that
                // boundary, do not leave the host permanently stuck in the
                // in-progress state; retry on the next poll cycle.
                if inventory_in_progress
                    && inventory_started_at
                        .is_some_and(|started| started.elapsed() < Duration::from_secs(4))
                {
                    continue;
                }
                inventory_tabs.clear();
                inventory_in_progress = true;
                inventory_started_at = Some(tokio::time::Instant::now());
                request_inventory_shared(&output).await?;
            },
            _ = pairing_poll.tick() => {
                match find_new_create_requests(&pairing_path).await {
                    Ok(requests) => for request in requests {
                        let mut command = request;
                        command["type"] = json!("create_tab");
                        if let Some(request_id) = command["request_id"].as_str() {
                            let _ = crate::native_setup::record_create_event(&host_id, request_id, "dispatch");
                        }
                        if let Err(error) = write_shared_message(&output, &command).await {
                            eprintln!("Native tab creation dispatch failed: {error}");
                        }
                    },
                    Err(error) => eprintln!("Native tab creation scan failed: {error}"),
                }
                let max_pairings = if extension_protocol_version >= 2 { usize::MAX } else { 1 };
                match find_new_pairings(&pairing_path, &mut consumed, max_pairings).await {
                    Ok(pairings) => for pairing in pairings {
                        let lease_id = pairing.request_id.clone();
                        if active.contains_key(&lease_id) { continue; }
                        if extension_protocol_version < 2 && !active.is_empty() {
                            let _ = notify_pair_error(&pairing, "This extension supports one active session. Reload the updated Controlla extension to enable simultaneous sessions.").await;
                            continue;
                        }
                        if let Err(error) = validate_pairing(&pairing) {
                            if !pairing.token.is_empty() && validate_endpoint(&pairing.endpoint).is_ok() { let _ = notify_pair_error(&pairing, &error).await; }
                            eprintln!("Native pairing failed: {error}");
                            continue;
                        }
                        let (session_tx, session_rx) = mpsc::channel(64);
                        active.insert(lease_id.clone(), session_tx);
                        let output = Arc::clone(&output);
                        pairing_tasks.spawn(async move {
                            let mut provider_ready = false;
                            let result = pair_and_proxy(session_rx, output.clone(), &pairing, &mut provider_ready).await;
                            let _ = write_shared_message(&output, &json!({"type":"release","lease_id":lease_id.clone()})).await;
                            if let Err(error) = &result && !provider_ready { let _ = notify_pair_error(&pairing, error).await; }
                            (lease_id, result, provider_ready)
                        });
                    }
                    Err(error) => eprintln!("Native pairing scan failed: {error}"),
                }
            }
            native = native_rx.recv() => {
                let message = native.ok_or("native port closed")??;
                match message["type"].as_str() {
                    Some("capabilities") => {
                        extension_protocol_version = message["protocol_version"].as_u64().and_then(|version| u32::try_from(version).ok()).unwrap_or(0);
                        // A freshly restarted service worker may not emit a
                        // topology event after the handshake. Request a
                        // complete inventory immediately so the on-disk
                        // snapshot is fresh before the first MCP call.
                        // Discard any request sent before the worker finished
                        // its handshake. That request may have been lost
                        // while MV3 was starting, so the handshake itself is
                        // the authoritative refresh boundary.
                        inventory_tabs.clear();
                        inventory_in_progress = true;
                        inventory_started_at = Some(tokio::time::Instant::now());
                        if let Err(error) = request_inventory_shared(&output).await {
                            eprintln!("Native inventory refresh request failed: {error}");
                            inventory_in_progress = false;
                            inventory_started_at = None;
                        }
                    },
                    Some("inventory_dirty") => {
                        if !inventory_in_progress {
                            inventory_tabs.clear();
                            inventory_in_progress = true;
                            inventory_started_at = Some(tokio::time::Instant::now());
                            if let Err(error) = request_inventory_shared(&output).await {
                                eprintln!("Native inventory refresh request failed: {error}");
                                inventory_in_progress = false;
                                inventory_started_at = None;
                            }
                        }
                    },
                    Some("tabs") if message["request_id"] == "inventory" => {
                        let Some(page) = message["tabs"].as_array() else { inventory_in_progress = false; inventory_started_at = None; continue; };
                        if inventory_tabs.len().saturating_add(page.len()) > 5000 {
                            inventory_in_progress = false;
                            inventory_started_at = None;
                            inventory_tabs.clear();
                            continue;
                        }
                        inventory_tabs.extend(page.iter().cloned());
                        if inventory_tabs.len() == 5000 {
                            inventory_in_progress = false;
                            inventory_started_at = None;
                            let mut bounded = message.clone();
                            bounded["tabs"] = json!(std::mem::take(&mut inventory_tabs));
                            bounded["offset"] = json!(0);
                            bounded["truncated"] = json!(message["next_offset"].is_number());
                            save_tabs_snapshot(&bounded, &host_id).await?;
                        } else if let Some(next_offset) = message["next_offset"].as_u64() {
                            write_shared_message(&output, &json!({"type":"list_tabs","request_id":"inventory","offset":next_offset,"page_size":100})).await?;
                        } else {
                            inventory_in_progress = false;
                            inventory_started_at = None;
                            let mut complete = message.clone();
                            complete["tabs"] = json!(std::mem::take(&mut inventory_tabs));
                            complete["offset"] = json!(0);
                            complete["truncated"] = json!(false);
                            save_tabs_snapshot(&complete, &host_id).await?;
                        }
                    },
                    Some("tabs_error") if message["request_id"] == "inventory" => { inventory_in_progress = false; inventory_started_at = None; inventory_tabs.clear(); },
                    Some("created_tab") => {
                        if let Some(request_id) = message["request_id"].as_str() {
                            let _ = crate::native_setup::record_create_event(&host_id, request_id, "response");
                        }
                        write_create_result(&pairing_path, &message).await?;
                        // The new document must be present in the inventory
                        // before the MCP server can pair to it.
                        inventory_tabs.clear();
                        inventory_in_progress = true;
                        inventory_started_at = Some(tokio::time::Instant::now());
                        request_inventory_shared(&output).await?;
                    },
                    Some("recovery_status") => {
                        // A native reconnect cannot safely replay a lease: the
                        // original provider token/session is intentionally not
                        // persisted in the extension journal. Acknowledge the
                        // sequenced status so the extension can clear the
                        // inspection record, and make the required repair
                        // action explicit to the owning client.
                        if message["schema_version"].as_u64() == Some(1)
                            && message["status"].as_str() == Some("recovery_required")
                            && message["sequence"].as_u64().is_some()
                        {
                            write_shared_message(&output, &json!({
                                "type":"recovery_ack",
                                "schema_version":1,
                                "sequence":message["sequence"],
                                "accepted":true,
                                "recovery_action":"rediscover_and_repair",
                                "replay_supported":false
                            })).await?;
                        }
                    },
                    _ => {
                        if message["type"] == "paired" {
                            extension_protocol_version = extension_protocol_version.max(message["protocol_version"].as_u64().and_then(|version| u32::try_from(version).ok()).unwrap_or(0));
                        }
                        let lease_id = message["lease_id"].as_str().or_else(|| message["request_id"].as_str());
                        if let Some(session_tx) = lease_id.and_then(|id| active.get(id)).or_else(|| (active.len() == 1).then(|| active.values().next().unwrap())) { let _ = session_tx.send(Ok(message)).await; }
                    }
                }
            }
            completed = pairing_tasks.join_next(), if !pairing_tasks.is_empty() => {
                if let Some(Ok((lease_id, result, _))) = completed {
                    active.remove(&lease_id);
                    if let Err(error) = result { eprintln!("Native paired session ended: {error}"); }
                }
            }
        }
    }
}

async fn write_shared_message<W: AsyncWrite + Unpin>(
    output: &Arc<Mutex<W>>,
    message: &Value,
) -> Result<(), String> {
    write_message(&mut *output.lock().await, message).await
}

async fn request_inventory_shared<W: AsyncWrite + Unpin>(
    output: &Arc<Mutex<W>>,
) -> Result<(), String> {
    write_shared_message(
        output,
        &json!({"type":"list_tabs","request_id":"inventory","offset":0,"page_size":100}),
    )
    .await
}

async fn find_new_create_requests(pairing_path: &Path) -> Result<Vec<Value>, String> {
    let dir = pairing_path
        .parent()
        .ok_or("native bridge directory is missing")?;
    let host_id = pairing_path
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("native-pairing-"))
        .ok_or("native host ID is missing")?;
    let prefix = format!("native-create-{host_id}-");
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut requests = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        if requests.len() >= 8 {
            break;
        }
        let filename = entry.file_name();
        let Some(name) = filename.to_str() else {
            continue;
        };
        let Some(request_id) = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".json"))
        else {
            continue;
        };
        if request_id.len() != 32 || !request_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let path = entry.path();
        let claimed = path.with_extension(format!("json.{}.claim", std::process::id()));
        if tokio::fs::rename(&path, &claimed).await.is_err() {
            continue;
        }
        let metadata = tokio::fs::symlink_metadata(&claimed)
            .await
            .map_err(|error| error.to_string())?;
        if !metadata.file_type().is_file() || metadata.len() > 4096 {
            let _ = tokio::fs::remove_file(&claimed).await;
            continue;
        }
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o077 != 0 {
            let _ = tokio::fs::remove_file(&claimed).await;
            continue;
        }
        let bytes = tokio::fs::read(&claimed)
            .await
            .map_err(|error| error.to_string())?;
        let _ = tokio::fs::remove_file(&claimed).await;
        if bytes.len() > 4096 {
            continue;
        }
        let Ok(request) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if request["request_id"] != request_id
            || request["expires_at_unix"]
                .as_u64()
                .is_none_or(|expires| expires < now || expires > now + 30)
        {
            continue;
        }
        requests.push(request);
    }
    Ok(requests)
}

async fn write_create_result(pairing_path: &Path, message: &Value) -> Result<(), String> {
    let host_id = pairing_path
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("native-pairing-"))
        .ok_or("native host ID is missing")?;
    let request_id = message["request_id"]
        .as_str()
        .ok_or("tab creation response has no request ID")?;
    if request_id.len() != 32 || !request_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("tab creation response ID is invalid".into());
    }
    let dir = pairing_path
        .parent()
        .ok_or("native bridge directory is missing")?;
    let path = dir.join(format!("native-create-result-{host_id}-{request_id}.json"));
    let temp = dir.join(format!("native-create-result-{host_id}-{request_id}.tmp"));
    let bytes = serde_json::to_vec(message).map_err(|error| error.to_string())?;
    if bytes.len() > 8192 {
        return Err("tab creation receipt exceeds size limit".into());
    }
    tokio::fs::write(&temp, bytes)
        .await
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    tokio::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))
        .await
        .map_err(|error| error.to_string())?;
    tokio::fs::rename(temp, path)
        .await
        .map_err(|error| error.to_string())
}

async fn find_new_pairings(
    path: &Path,
    consumed: &mut HashSet<String>,
    limit: usize,
) -> Result<Vec<Pairing>, String> {
    let mut found = Vec::new();
    if let Some(pairing) = find_new_pairing(path, consumed).await? {
        found.push(pairing);
    }
    if found.len() >= limit {
        return Ok(found);
    }
    let parent = path.parent().ok_or("native pairing path has no parent")?;
    let host_id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("native-pairing-"))
        .ok_or("native pairing path has no host ID")?;
    let prefix = format!("native-pairing-{host_id}-");
    let mut entries = match tokio::fs::read_dir(parent).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(error) => return Err(error.to_string()),
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(&prefix)
            && name.ends_with(".json")
            && let Some(pairing) = find_new_pairing(&entry.path(), consumed).await?
        {
            found.push(pairing);
            if found.len() >= limit {
                break;
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
async fn request_inventory<W: AsyncWrite + Unpin>(output: &mut W) -> Result<(), String> {
    write_message(
        output,
        &json!({"type":"list_tabs","request_id":"inventory","offset":0,"page_size":100}),
    )
    .await
}

async fn find_new_pairing(
    path: &Path,
    consumed: &mut HashSet<String>,
) -> Result<Option<Pairing>, String> {
    let claimed = path.with_extension(format!("json.{}.claim", std::process::id()));
    match tokio::fs::rename(path, &claimed).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot claim pairing file: {error}")),
    }
    let result = read_pairing(&claimed).await;
    let _ = tokio::fs::remove_file(&claimed).await;
    let pairing = match result {
        Ok(pairing) => pairing,
        Err(error) => return Err(error),
    };
    if consumed.contains(&pairing.token) {
        return Ok(None);
    }
    if consumed.len() >= 4096 {
        return Err("native host pairing limit reached; restart the extension".into());
    }
    consumed.insert(pairing.token.clone());
    if expired(pairing.expires_at_unix)? {
        return Ok(None);
    }
    Ok(Some(pairing))
}

async fn pair_and_proxy<W>(
    mut native_rx: mpsc::Receiver<Result<Value, String>>,
    output: Arc<Mutex<W>>,
    pairing: &Pairing,
    provider_ready: &mut bool,
) -> Result<(), String>
where
    W: AsyncWrite + Unpin,
{
    write_shared_message(
        &output,
        &json!(
            {
            "type":"pair", "request_id":pairing.request_id, "lease_id":pairing.request_id, "tab_ids":pairing.tab_ids,
            "expected_urls":pairing.expected_urls,
            "expected_document_ids":pairing.expected_document_ids
            ,"unrestricted":pairing.unrestricted
        }),
    )
    .await?;
    let paired = tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            tokio::select! {
                native = native_rx.recv() => {
                    let message = native.ok_or("native port closed")??;
                    match message["type"].as_str() {
                        Some("paired") if message["request_id"] == pairing.request_id => break Ok::<Value, String>(message),
                        Some("pair_error") if message["request_id"] == pairing.request_id => {
                            let error = message["error"].as_str().unwrap_or("extension could not attach selected tabs");
                            break Err(error.chars().take(256).collect());
                        }
                        _ => {}
                    }
                }
            }
        }
    }).await.map_err(|_| "extension pairing timed out".to_owned())??;
    let paired: NativeReply =
        serde_json::from_value(paired).map_err(|_| "invalid extension pairing reply".to_owned())?;
    if paired.kind != "paired" {
        return Err("unexpected extension pairing reply".into());
    }
    let targets = paired.targets.ok_or("paired reply omitted targets")?;
    let extension_version = paired
        .extension_version
        .ok_or("paired reply omitted extension_version")?;
    let protocol_version = if paired.protocol_version == 0 {
        1
    } else {
        paired.protocol_version
    };
    let expected_build_id = validate_extension_compatibility(
        protocol_version,
        &extension_version,
        paired.extension_build_id.as_deref(),
    )?;
    if paired.document_identity != Some(true) {
        return Err(
            "extension lacks durable document identity support; reload the updated extension"
                .into(),
        );
    }
    validate_targets(&pairing.tab_ids, &targets)?;
    if expired(pairing.expires_at_unix)? {
        return Err("pairing expired".into());
    }

    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_MESSAGE);
    config.max_frame_size = Some(MAX_MESSAGE);
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(5),
        connect_async_with_config(&pairing.endpoint, Some(config), false),
    )
    .await
    .map_err(|_| "provider connection timed out".to_owned())?
    .map_err(|error| format!("provider connection failed: {error}"))?;
    socket
        .send(Message::Text(
            json!({
                "type":"hello", "token":pairing.token, "targets":targets,
                "extension_version":extension_version, "extension_build_id":expected_build_id, "document_identity":true,
                "protocol_version":protocol_version,
                "batch_execution":paired.batch_execution, "batch_deadline":paired.batch_deadline
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| error.to_string())?;
    match tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .map_err(|_| "provider handshake timed out".to_owned())?
    {
        Some(Ok(Message::Text(text))) if text.len() <= MAX_MESSAGE => {
            let response: Value =
                serde_json::from_str(&text).map_err(|_| "provider sent invalid JSON".to_owned())?;
            let protocol_matches = response["protocol_version"].as_u64()
                == Some(protocol_version as u64)
                || (protocol_version == 1
                    && response["protocol_version"].is_null()
                    && response["server_version"] == env!("CARGO_PKG_VERSION"));
            if response["type"] != "ready" || !protocol_matches {
                return Err("provider rejected pairing or version mismatch".into());
            }
        }
        Some(Ok(Message::Text(_))) => {
            return Err("provider handshake message exceeds size limit".into());
        }
        Some(Ok(_)) => return Err("provider sent an invalid handshake response".into()),
        Some(Err(error)) => return Err(format!("provider handshake failed: {error}")),
        None => return Err("provider closed during handshake".into()),
    }
    *provider_ready = true;
    proxy(native_rx, output, &mut socket, &pairing.request_id).await
}

fn validate_extension_compatibility(
    protocol_version: u32,
    extension_version: &str,
    extension_build_id: Option<&str>,
) -> Result<String, String> {
    if protocol_version < 2 {
        return Err(format!(
            "extension protocol {protocol_version} is unsupported; reload the matching Controlla extension"
        ));
    }
    if extension_version != env!("CARGO_PKG_VERSION") {
        return Err(format!(
            "extension version mismatch: expected {}, received {extension_version}",
            env!("CARGO_PKG_VERSION")
        ));
    }
    let expected_build_id = env!("CONTROLLA_EXTENSION_BUILD_ID").to_owned();
    let runtime_fallback = format!("controlla-chrome-extension:{extension_version}:runtime");
    if extension_build_id != Some(expected_build_id.as_str())
        && extension_build_id != Some(runtime_fallback.as_str())
    {
        // Name both identities: "reload the extension" is unactionable if the
        // installed build cannot be compared with the one this host expects.
        let received = extension_build_id
            .map(|value| value.chars().take(96).collect::<String>())
            .unwrap_or_else(|| "none".to_owned());
        return Err(format!(
            "extension build identity mismatch: expected {expected_build_id}, received {received}; reload the matching Controlla extension in chrome://extensions"
        ));
    }
    Ok(expected_build_id)
}

async fn notify_pair_error(pairing: &Pairing, error: &str) -> Result<(), String> {
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(2),
        tokio_tungstenite::connect_async(&pairing.endpoint),
    )
    .await
    .map_err(|_| "pair error notification timed out".to_owned())?
    .map_err(|error| format!("pair error notification failed: {error}"))?;
    socket.send(Message::Text(json!({
        "type":"pair_error", "token":pairing.token, "error":error.chars().take(256).collect::<String>()
    }).to_string().into())).await.map_err(|error| error.to_string())
}

async fn read_pairing(path: &Path) -> Result<Pairing, String> {
    let metadata = tokio::fs::symlink_metadata(path).await.map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            "pairing file is not present".to_owned()
        } else {
            format!("cannot inspect pairing file: {error}")
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("pairing path must be a regular, non-symlink file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err("pairing file permissions must be 0600".into());
        }
    }
    if metadata.len() > MAX_MESSAGE as u64 {
        return Err("pairing file exceeds size limit".into());
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| format!("cannot read pairing file: {error}"))?;
    if bytes.len() > MAX_MESSAGE {
        return Err("pairing file exceeds size limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "pairing file contains invalid JSON".into())
}

fn validate_pairing(pairing: &Pairing) -> Result<(), String> {
    if pairing.token.is_empty() || pairing.request_id.is_empty() || pairing.tab_ids.is_empty() {
        return Err("pairing file is missing required values".into());
    }
    let mut ids = pairing.tab_ids.clone();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != pairing.tab_ids.len() {
        return Err("pairing tab_ids must be unique".into());
    }
    if !pairing.unrestricted
        && pairing.tab_ids.iter().any(|id| {
            pairing.expected_urls.get(id).is_none_or(String::is_empty)
                || pairing
                    .expected_document_ids
                    .get(id)
                    .is_none_or(|value| value.is_empty() || value.len() > 128)
        })
    {
        return Err("pairing is missing a selected tab URL or durable document ID".into());
    }
    validate_endpoint(&pairing.endpoint)?;
    if expired(pairing.expires_at_unix)? {
        return Err("pairing has expired".into());
    }
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> Result<(), String> {
    let rest = endpoint
        .strip_prefix("ws://127.0.0.1:")
        .ok_or("provider endpoint must be a plain loopback ws URL")?;
    let (port, suffix) = rest.split_once('/').unwrap_or((rest, ""));
    if port.parse::<u16>().ok().filter(|port| *port != 0).is_none()
        || suffix.contains(['?', '#', '@'])
        || endpoint.contains('@')
    {
        return Err("provider endpoint must be a plain loopback ws URL".into());
    }
    Ok(())
}

fn expired(expires_at: u64) -> Result<bool, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before Unix epoch".to_owned())?
        .as_secs();
    Ok(expires_at <= now)
}

fn validate_targets(tab_ids: &[u32], targets: &[String]) -> Result<(), String> {
    let mut expected = tab_ids.iter().map(u32::to_string).collect::<Vec<_>>();
    let mut received = targets.to_vec();
    expected.sort();
    received.sort();
    received.dedup();
    if received != expected {
        return Err("extension paired targets do not match requested tabs".into());
    }
    Ok(())
}

async fn proxy<W, S>(
    mut native_rx: mpsc::Receiver<Result<Value, String>>,
    output: Arc<Mutex<W>>,
    socket: &mut S,
    lease_id: &str,
) -> Result<(), String>
where
    W: AsyncWrite + Unpin,
    S: futures_util::Sink<Message>
        + futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
    S::Error: std::fmt::Display,
{
    let mut screenshot_transfers = ScreenshotAssembler::default();
    let mut cleanup_tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = cleanup_tick.tick() => screenshot_transfers.expire(),
            native = native_rx.recv() => {
                let message = native.ok_or("native port closed")??;
                match message["type"].as_str() {
                    Some("result_chunk") => match screenshot_transfers.push(&message) {
                        Ok(Some(assembled)) => socket.send(Message::Text(assembled.to_string().into())).await.map_err(|error| error.to_string())?,
                        Ok(None) => {},
                        Err(error) => socket.send(Message::Text(json!({"type":"result","id":message["id"],
                            "lease_id":lease_id,"error":error}).to_string().into())).await.map_err(|send_error| send_error.to_string())?,
                    },
                    Some("result_chunk_abort") => {
                        screenshot_transfers.abort(&message).map_err(str::to_owned)?;
                        socket.send(Message::Text(json!({"type":"result","id":message["id"],
                            "lease_id":lease_id,"error":"Screenshot transfer was explicitly aborted.","reason_code":"screenshot_transfer_aborted"}).to_string().into())).await.map_err(|error| error.to_string())?;
                    }
                    Some("result" | "batch_result") if message["id"].is_u64() || message["id"].is_string() => {
                        socket.send(Message::Text(message.to_string().into())).await.map_err(|error| error.to_string())?;
                    }
                    _ => return Err("unexpected native message during paired session".into()),
                }
            }
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if text.len() > MAX_MESSAGE { return Err("provider message exceeds size limit".into()); }
                    let mut value: Value = serde_json::from_str(&text).map_err(|_| "provider sent invalid JSON".to_owned())?;
                    if value["type"] == "cancel_screenshot" {
                        let id = value["id"].as_u64().ok_or("screenshot cancellation ID is invalid")?;
                        value["lease_id"] = json!(lease_id);
                        value["id"] = json!(id);
                        write_shared_message(&output, &value).await?;
                        continue;
                    }
                    if !matches!(value["type"].as_str(), Some("command" | "batch")) { return Err("provider sent an unsupported message".into()); }
                    value["lease_id"] = json!(lease_id);
                    write_shared_message(&output, &value).await?;
                }
                Some(Ok(Message::Ping(payload))) => socket.send(Message::Pong(payload)).await.map_err(|error| error.to_string())?,
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Ok(_)) => return Err("provider sent an unsupported websocket frame".into()),
                Some(Err(error)) => return Err(format!("provider connection failed: {error}")),
            }
        }
    }
}

async fn save_tabs_snapshot(message: &Value, host_id: &str) -> Result<(), String> {
    let Some(tabs) = message["tabs"].as_array() else {
        return Err("extension sent an invalid tabs snapshot".into());
    };
    if tabs.len() > 5000 {
        return Err("extension tabs snapshot exceeds limit".into());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before Unix epoch".to_owned())?
        .as_secs();
    let bytes = serde_json::to_vec(&json!({
        "tabs":tabs,"last_seen_unix":now,"host_id":host_id,
        "truncated":message["truncated"].as_bool().unwrap_or(false),
        "last_focused_window_id":message["last_focused_window_id"].as_u64(),
        "windows":message["windows"].as_array().cloned().unwrap_or_default(),
        "windows_total":message["windows_total"].as_u64(),
        "windows_truncated":message["windows_truncated"].as_bool().unwrap_or(false),
        "groups":message["groups"].as_array().cloned().unwrap_or_default(),
        "groups_total":message["groups_total"].as_u64(),
        "groups_truncated":message["groups_truncated"].as_bool().unwrap_or(false)
    }))
    .map_err(|_| "cannot encode tabs snapshot".to_owned())?;
    if bytes.len() > 16_000_000 {
        return Err("tabs snapshot exceeds size limit".into());
    }
    let path = crate::native_setup::pairing_path(host_id)?.with_file_name("native-tabs.json");
    let dir = path.parent().ok_or("invalid tabs snapshot path")?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| format!("cannot create bridge state directory: {error}"))?;
    #[cfg(unix)]
    tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .await
        .map_err(|error| format!("cannot secure bridge state directory: {error}"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let temporary = path.with_extension(format!("json.{}.{}.tmp", std::process::id(), nonce));
    #[cfg(unix)]
    {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .await
            .map_err(|error| format!("cannot create tabs snapshot: {error}"))?;
        file.write_all(&bytes)
            .await
            .map_err(|error| format!("cannot write tabs snapshot: {error}"))?;
        file.flush()
            .await
            .map_err(|error| format!("cannot flush tabs snapshot: {error}"))?;
    }
    #[cfg(not(unix))]
    tokio::fs::write(&temporary, &bytes)
        .await
        .map_err(|error| format!("cannot write tabs snapshot: {error}"))?;
    tokio::fs::rename(&temporary, &path)
        .await
        .map_err(|error| format!("cannot publish tabs snapshot: {error}"))
}

async fn read_message<R, T>(reader: &mut R) -> Result<T, String>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut header = [0u8; 4];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|error| format!("native message header failed: {error}"))?;
    let length = u32::from_ne_bytes(header) as usize;
    if length == 0 || length > MAX_MESSAGE {
        return Err("native message size is invalid".into());
    }
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|error| format!("native message body failed: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|_| "native message contains invalid JSON".into())
}

async fn write_message<W, T>(writer: &mut W, message: &T) -> Result<(), String>
where
    W: AsyncWrite + Unpin,
    T: serde::Serialize,
{
    let bytes =
        serde_json::to_vec(message).map_err(|_| "cannot encode native message".to_owned())?;
    if bytes.is_empty() || bytes.len() > MAX_MESSAGE {
        return Err("native message size is invalid".into());
    }
    writer
        .write_all(&(bytes.len() as u32).to_ne_bytes())
        .await
        .map_err(|error| format!("native message header failed: {error}"))?;
    writer
        .write_all(&bytes)
        .await
        .map_err(|error| format!("native message body failed: {error}"))?;
    writer
        .flush()
        .await
        .map_err(|error| format!("native message flush failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn ownerless_native_create_requests_are_host_bound_and_one_shot() {
        let host = "0123456789abcdef0123456789abcdef";
        let dir =
            std::env::temp_dir().join(format!("controlla-native-create-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let pairing = dir.join(format!("native-pairing-{host}.json"));
        let request_id = "abcdef0123456789abcdef0123456789";
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let request = json!({"request_id":request_id,"url":"https://example.test/","idempotency_key":"one-logical-open",
            "active":false,"expires_at_unix":now + 10});
        let request_path = dir.join(format!("native-create-{host}-{request_id}.json"));
        tokio::fs::write(&request_path, request.to_string())
            .await
            .unwrap();
        #[cfg(unix)]
        tokio::fs::set_permissions(&request_path, std::fs::Permissions::from_mode(0o600))
            .await
            .unwrap();
        let found = find_new_create_requests(&pairing).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["request_id"], request_id);
        assert!(find_new_create_requests(&pairing).await.unwrap().is_empty());
        write_create_result(
            &pairing,
            &json!({"type":"created_tab","request_id":request_id,
            "result":{"tab_id":"23","document_id":"doc-23"}}),
        )
        .await
        .unwrap();
        let receipt = tokio::fs::read_to_string(
            dir.join(format!("native-create-result-{host}-{request_id}.json")),
        )
        .await
        .unwrap();
        assert!(receipt.contains("doc-23"));
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn extension_pairing_requires_matching_protocol_version_and_build_identity() {
        let version = env!("CARGO_PKG_VERSION");
        let build = env!("CONTROLLA_EXTENSION_BUILD_ID").to_owned();
        assert_eq!(
            validate_extension_compatibility(2, version, Some(&build)).unwrap(),
            build
        );
        assert!(
            validate_extension_compatibility(1, version, Some(&build))
                .unwrap_err()
                .contains("protocol 1")
        );
        assert!(
            validate_extension_compatibility(2, "0.0.0", Some(&build))
                .unwrap_err()
                .contains("version mismatch")
        );
        assert!(
            validate_extension_compatibility(2, version, None)
                .unwrap_err()
                .contains("build identity mismatch")
        );
        assert!(
            validate_extension_compatibility(2, version, Some("stale-build"))
                .unwrap_err()
                .contains("build identity mismatch")
        );
        let stale_same_version = format!("controlla-chrome-extension:{version}:{}", "0".repeat(64));
        assert_ne!(build, stale_same_version);
        assert!(
            validate_extension_compatibility(2, version, Some(&stale_same_version))
                .unwrap_err()
                .contains("build identity mismatch")
        );
    }

    #[test]
    fn extension_build_id_matches_every_shipped_runtime_asset() {
        let assets: &[(&str, &[u8])] = &[
            (
                "background.js",
                include_bytes!("../../../extensions/chrome-controlla/background.js"),
            ),
            (
                "manifest.json",
                include_bytes!("../../../extensions/chrome-controlla/manifest.json"),
            ),
            (
                "open-recovery.html",
                include_bytes!("../../../extensions/chrome-controlla/open-recovery.html"),
            ),
            (
                "popup.html",
                include_bytes!("../../../extensions/chrome-controlla/popup.html"),
            ),
            (
                "popup.js",
                include_bytes!("../../../extensions/chrome-controlla/popup.js"),
            ),
            (
                "secure-input.css",
                include_bytes!("../../../extensions/chrome-controlla/secure-input.css"),
            ),
            (
                "secure-input.html",
                include_bytes!("../../../extensions/chrome-controlla/secure-input.html"),
            ),
            (
                "secure-input.js",
                include_bytes!("../../../extensions/chrome-controlla/secure-input.js"),
            ),
            (
                "lib/transport.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/transport.js"),
            ),
            (
                "lib/frame-tree.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/frame-tree.js"),
            ),
            (
                "lib/debugger-sessions.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/debugger-sessions.js"),
            ),
            (
                "lib/presentation-input.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/presentation-input.js"),
            ),
            (
                "lib/input.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/input.js"),
            ),
            (
                "lib/editor-adapters.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/editor-adapters.js"),
            ),
            (
                "lib/observation.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/observation.js"),
            ),
            (
                "lib/pointer.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/pointer.js"),
            ),
            (
                "lib/resource-keys.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/resource-keys.js"),
            ),
            (
                "lib/owned-resources.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/owned-resources.js"),
            ),
            (
                "lib/presentation-slots.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/presentation-slots.js"),
            ),
            (
                "lib/tab-control.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/tab-control.js"),
            ),
            (
                "lib/owned-open.js",
                include_bytes!("../../../extensions/chrome-controlla/lib/owned-open.js"),
            ),
        ];
        let extension_source = include_str!("../../../extensions/chrome-controlla/background.js");
        let asset_declaration = extension_source
            .split_once("const CONTROLLA_EXTENSION_BUILD_ASSETS = [")
            .and_then(|(_, rest)| rest.split_once("];"))
            .map(|(assets, _)| assets)
            .expect("extension declares its build asset set");
        let extension_assets = asset_declaration
            .split('"')
            .skip(1)
            .step_by(2)
            .collect::<Vec<_>>();
        assert_eq!(
            extension_assets,
            assets.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            "native-host and extension build asset lists must be identical and ordered"
        );
        let mut digest = Sha256::new();
        for (name, bytes) in assets {
            digest.update(name.as_bytes());
            digest.update([0]);
            digest.update(bytes);
            digest.update([0]);
        }
        let digest = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            env!("CONTROLLA_EXTENSION_BUILD_ID"),
            format!(
                "controlla-chrome-extension:{}:{digest}",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[tokio::test]
    async fn native_message_round_trips_and_rejects_oversize() {
        let (mut writer, mut reader) = duplex(64);
        let expected = json!({"type":"pair","request_id":"r1"});
        let (write, actual) = tokio::join!(
            write_message(&mut writer, &expected),
            read_message::<_, Value>(&mut reader)
        );
        write.unwrap();
        assert_eq!(actual.unwrap(), expected);
        let (mut writer, mut reader) = duplex(8);
        writer
            .write_all(&((MAX_MESSAGE as u32 + 1).to_ne_bytes()))
            .await
            .unwrap();
        assert_eq!(
            read_message::<_, Value>(&mut reader).await.unwrap_err(),
            "native message size is invalid"
        );
    }

    #[test]
    fn endpoint_and_pairing_guards_reject_non_loopback_and_expired_values() {
        assert!(validate_endpoint("ws://127.0.0.1:8123/").is_ok());
        for bad in [
            "ws://localhost:8123/",
            "ws://192.168.1.2:8123/",
            "wss://127.0.0.1:8123/",
        ] {
            assert!(validate_endpoint(bad).is_err());
        }
        assert!(expired(1).unwrap());
    }

    #[test]
    fn unrestricted_pairing_accepts_tabs_without_discovery_identity() {
        let mut pairing = Pairing {
            endpoint: "ws://127.0.0.1:8123/".into(),
            token: "token".into(),
            tab_ids: vec![17],
            request_id: "request".into(),
            expected_urls: Default::default(),
            expected_document_ids: Default::default(),
            unrestricted: false,
            expires_at_unix: u64::MAX,
        };
        assert!(validate_pairing(&pairing).is_err());
        pairing.unrestricted = true;
        assert!(validate_pairing(&pairing).is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn idle_inventory_continues_until_a_fresh_pairing_request_arrives() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("native-idle-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("native-pairing.json");
        let mut consumed = HashSet::new();
        assert!(
            find_new_pairing(&path, &mut consumed)
                .await
                .unwrap()
                .is_none()
        );

        let (mut writer, mut reader) = duplex(128);
        request_inventory(&mut writer).await.unwrap();
        let request: Value = read_message(&mut reader).await.unwrap();
        assert_eq!(
            request,
            json!({"type":"list_tabs","request_id":"inventory","offset":0,"page_size":100})
        );

        let expires = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60;
        tokio::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "endpoint":"ws://127.0.0.1:8123/", "token":"secret", "tab_ids":[17],
                "request_id":"fresh-1", "expires_at_unix":expires
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .await
            .unwrap();
        assert_eq!(
            find_new_pairing(&path, &mut consumed)
                .await
                .unwrap()
                .unwrap()
                .request_id,
            "fresh-1"
        );
        assert!(
            find_new_pairing(&path, &mut consumed)
                .await
                .unwrap()
                .is_none()
        );
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn host_claims_two_independent_pairing_files_without_waiting_for_first_session() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("native-multi-pair-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let host_id = "0123456789abcdef0123456789abcdef";
        let base = dir.join(format!("native-pairing-{host_id}.json"));
        let expires = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60;
        for (index, request_id) in [
            "11111111111111111111111111111111",
            "22222222222222222222222222222222",
        ]
        .into_iter()
        .enumerate()
        {
            let path = dir.join(format!("native-pairing-{host_id}-{request_id}.json"));
            tokio::fs::write(&path, serde_json::to_vec(&json!({
                "endpoint":"ws://127.0.0.1:8123/", "token":format!("token-{index}"), "tab_ids":[17 + index],
                "request_id":request_id, "expires_at_unix":expires
            })).unwrap()).await.unwrap();
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .await
                .unwrap();
        }
        let mut consumed = HashSet::new();
        let first = find_new_pairings(&base, &mut consumed, 1).await.unwrap();
        assert_eq!(first.len(), 1);
        let remaining = find_new_pairings(&base, &mut consumed, usize::MAX)
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            find_new_pairings(&base, &mut consumed, usize::MAX)
                .await
                .unwrap()
                .len(),
            0
        );
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pairing_file_requires_private_regular_file() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("native-pairing-{}", std::process::id()));
        tokio::fs::write(&path, br#"{"endpoint":"ws://127.0.0.1:8123/","token":"x","tab_ids":[1],"request_id":"r","expires_at_unix":9999999999}"#).await.unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .await
            .unwrap();
        assert!(read_pairing(&path).await.is_ok());
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();
        assert!(read_pairing(&path).await.is_err());
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .await
            .unwrap();
        let link = path.with_extension("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read_pairing(&link).await.is_err());
        let _ = tokio::fs::remove_file(link).await;
        let _ = tokio::fs::remove_file(path).await;
    }

    #[test]
    fn screenshot_chunks_reassemble_out_of_order_and_verify_digest() {
        let mut png = vec![0; 393_219];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[8..12].copy_from_slice(&13_u32.to_be_bytes());
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&4096_u32.to_be_bytes());
        png[20..24].copy_from_slice(&4096_u32.to_be_bytes());
        let data = base64::engine::general_purpose::STANDARD.encode(&png);
        let base64_digest = Sha256::digest(data.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let image_digest = Sha256::digest(&png)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let evidence = || json!({"target_id":"41","document_id":"document-41","url":"https://example.test/","captured_at_ms":1001,"capture_started_at_ms":1000,"visual_revision":image_digest,"image_sha256":image_digest,"target_lifecycle":"active","render_revision":null,"dimensions":{"width":4096,"height":4096}});
        let chunk = |index: usize, value: &str| {
            json!({
                "type":"result_chunk","id":7,"lease_id":"lease-a","transfer_id":"transfer-a",
                "index":index,"total_chunks":2,"total_chars":data.len(),"base64_sha256":base64_digest,"visual_evidence":evidence(),"chunk":value
            })
        };
        let mut assembler = ScreenshotAssembler::default();
        assert!(
            assembler
                .push(&chunk(1, &data[SCREENSHOT_CHUNK_CHARS..]))
                .unwrap()
                .is_none()
        );
        let complete = assembler
            .push(&chunk(0, &data[..SCREENSHOT_CHUNK_CHARS]))
            .unwrap()
            .unwrap();
        assert_eq!(complete["type"], "result");
        assert_eq!(complete["id"], 7);
        assert_eq!(complete["result"]["data"], data);
        assert_eq!(
            complete["result"]["visual_evidence"]["document_id"],
            "document-41"
        );
    }

    #[test]
    fn screenshot_chunk_abort_releases_partial_payload_immediately() {
        let data = format!("{}{}", "A".repeat(SCREENSHOT_CHUNK_CHARS), "BCDEFG");
        let digest = Sha256::digest(data.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let first = json!({"id":19,"lease_id":"lease-x","transfer_id":"transfer-x","index":0,
            "total_chunks":2,"total_chars":data.len(),"base64_sha256":digest,"visual_evidence":{"target_id":"41","document_id":"doc","url":"https://example.test/","captured_at_ms":1001,"capture_started_at_ms":1000,"visual_revision":digest,"image_sha256":digest,"target_lifecycle":"active","render_revision":null,"dimensions":{"width":320,"height":180}},"chunk":"A".repeat(SCREENSHOT_CHUNK_CHARS)});
        let mut assembler = ScreenshotAssembler::default();
        assert!(assembler.push(&first).unwrap().is_none());
        assert_eq!(assembler.transfers.len(), 1);
        assembler
            .abort(&json!({"id":19,"lease_id":"lease-x","transfer_id":"transfer-x"}))
            .unwrap();
        assert!(
            assembler.transfers.is_empty(),
            "aborted screenshot bytes are discarded immediately"
        );
    }

    #[test]
    fn screenshot_chunk_transfer_expiry_releases_partial_payload() {
        let key = (
            "lease-expired".to_owned(),
            27,
            "transfer-expired".to_owned(),
        );
        let mut assembler = ScreenshotAssembler::default();
        assembler.transfers.insert(
            key.clone(),
            ScreenshotTransfer {
                lease_id: key.0.clone(),
                total_chunks: 2,
                total_chars: SCREENSHOT_CHUNK_CHARS + 1,
                base64_sha256: "0".repeat(64),
                visual_evidence: json!({}),
                chunks: vec![Some("A".repeat(SCREENSHOT_CHUNK_CHARS)), None],
                started_at: tokio::time::Instant::now()
                    - SCREENSHOT_TRANSFER_TTL
                    - Duration::from_millis(1),
            },
        );
        assembler.expire();
        assert!(
            assembler.transfers.is_empty(),
            "expired partial screenshot bytes are dropped"
        );
    }

    #[test]
    fn screenshot_chunks_reject_replays_corruption_and_bad_geometry() {
        let data = format!("{}{}", "A".repeat(SCREENSHOT_CHUNK_CHARS), "BCDEFG");
        let digest = Sha256::digest(data.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let message = |index: usize, chunk: &str, hash: &str, total_chars: usize| {
            json!({
                "type":"result_chunk","id":8,"lease_id":"lease-a","transfer_id":"transfer-b",
                "index":index,"total_chunks":2,"total_chars":total_chars,"base64_sha256":hash,
                "visual_evidence":{"target_id":"41","document_id":"doc","url":"https://example.test/","captured_at_ms":1001,"capture_started_at_ms":1000,"visual_revision":hash,"image_sha256":hash,"target_lifecycle":"active","render_revision":null,"dimensions":{"width":320,"height":180}},"chunk":chunk
            })
        };
        let mut max_supported = message(0, &data[..SCREENSHOT_CHUNK_CHARS], &digest, data.len());
        max_supported["visual_evidence"]["dimensions"] = json!({"width":4096,"height":4096});
        assert!(
            ScreenshotAssembler::default()
                .push(&max_supported)
                .unwrap()
                .is_none(),
            "the native bridge must accept captures within the shared 4096-edge/16-MP contract"
        );
        let mut oversized = message(0, &data[..SCREENSHOT_CHUNK_CHARS], &digest, data.len());
        oversized["visual_evidence"]["dimensions"] = json!({"width":4097,"height":1});
        assert!(
            ScreenshotAssembler::default().push(&oversized).is_err(),
            "the native bridge must reject dimensions above the shared maximum before retaining chunks"
        );
        let mut assembler = ScreenshotAssembler::default();
        let first = message(0, &data[..SCREENSHOT_CHUNK_CHARS], &digest, data.len());
        assert!(assembler.push(&first).unwrap().is_none());
        assert!(assembler.push(&first).is_err());
        assert!(assembler.transfers.is_empty());
        assert!(
            assembler
                .push(&message(1, "BCDEFG", &"0".repeat(64), data.len()))
                .unwrap()
                .is_none()
        );
        assert!(
            assembler
                .push(&message(
                    0,
                    &data[..SCREENSHOT_CHUNK_CHARS],
                    &"0".repeat(64),
                    data.len()
                ))
                .is_err()
        );
        assert!(assembler.transfers.is_empty());
        assert!(
            assembler
                .push(&message(0, "short", &digest, data.len()))
                .is_err()
        );
    }
}
