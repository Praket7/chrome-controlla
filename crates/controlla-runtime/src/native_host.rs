//! Chrome native-messaging transport for the shared extension provider.

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

const MAX_MESSAGE: usize = 1_048_576;

#[derive(Debug, Deserialize)]
struct Pairing {
    endpoint: String,
    token: String,
    tab_ids: Vec<u32>,
    request_id: String,
    #[serde(default)]
    expected_urls: std::collections::BTreeMap<u32, String>,
    #[serde(default)]
    expected_document_ids: std::collections::BTreeMap<u32, String>,
    expires_at_unix: u64,
}

#[derive(Debug, Deserialize)]
struct NativeReply {
    #[serde(rename = "type")]
    kind: String,
    targets: Option<Vec<String>>,
    extension_version: Option<String>,
    document_identity: Option<bool>,
}

pub async fn run() -> Result<(), String> {
    let mut host_bytes = [0u8; 16];
    getrandom::fill(&mut host_bytes).map_err(|_| "cannot generate native host ID")?;
    let host_id = host_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let pairing_path = crate::native_setup::pairing_path(&host_id)?;
    let mut input = tokio::io::stdin();
    let mut output = tokio::io::stdout();
    write_message(&mut output, &json!({"type":"ready"})).await?;
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
    let mut last_pairing_error = String::new();
    let mut inventory_poll = tokio::time::interval(Duration::from_secs(5));
    let mut pairing_poll = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = inventory_poll.tick() => request_inventory(&mut output).await?,
            _ = pairing_poll.tick() => {
                match find_new_pairing(&pairing_path, &mut consumed).await {
                    Ok(Some(pairing)) => {
                        let result = if let Err(error) = validate_pairing(&pairing) {
                            if !pairing.token.is_empty() && validate_endpoint(&pairing.endpoint).is_ok() {
                                let _ = notify_pair_error(&pairing, &error).await;
                            }
                            Err(error)
                        } else {
                            let mut provider_ready = false;
                            let connected = pair_and_proxy(&mut native_rx, &mut output, &pairing, &host_id, &mut provider_ready).await;
                            let _ = write_message(&mut output, &json!({"type":"release"})).await;
                            if let Err(error) = &connected && !provider_ready {
                                let _ = notify_pair_error(&pairing, error).await;
                            }
                            connected
                        };
                        if let Err(error) = result {
                            if error != last_pairing_error { eprintln!("Native pairing failed: {error}"); }
                            last_pairing_error = error;
                        } else { last_pairing_error.clear(); }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if error != last_pairing_error { eprintln!("Native pairing skipped: {error}"); }
                        last_pairing_error = error;
                    }
                }
            }
            native = native_rx.recv() => handle_idle_message(native.ok_or("native port closed")??, &host_id).await?,
        }
    }
}

async fn request_inventory<W: AsyncWrite + Unpin>(output: &mut W) -> Result<(), String> {
    write_message(
        output,
        &json!({"type":"list_tabs","request_id":"inventory"}),
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

async fn handle_idle_message(message: Value, host_id: &str) -> Result<(), String> {
    match message["type"].as_str() {
        Some("tabs") if message["request_id"] == "inventory" => {
            save_tabs_snapshot(&message, host_id).await
        }
        Some("tabs_error") if message["request_id"] == "inventory" => Ok(()),
        // Ignore unsolicited results, old pair replies, and unknown messages while idle.
        _ => Ok(()),
    }
}

async fn pair_and_proxy<W>(
    native_rx: &mut mpsc::Receiver<Result<Value, String>>,
    output: &mut W,
    pairing: &Pairing,
    host_id: &str,
    provider_ready: &mut bool,
) -> Result<(), String>
where
    W: AsyncWrite + Unpin,
{
    write_message(
        output,
        &json!(
            {
            "type":"pair", "request_id":pairing.request_id, "tab_ids":pairing.tab_ids,
            "expected_urls":pairing.expected_urls,
            "expected_document_ids":pairing.expected_document_ids
        }),
    )
    .await?;
    let mut inventory_poll = tokio::time::interval(Duration::from_secs(5));
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
                        Some("tabs") if message["request_id"] == "inventory" => save_tabs_snapshot(&message, host_id).await?,
                        _ => {}
                    }
                }
                _ = inventory_poll.tick() => request_inventory(output).await?,
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
    if extension_version != env!("CARGO_PKG_VERSION") {
        return Err("extension/server version mismatch".into());
    }
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
                "extension_version":extension_version, "document_identity":true
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
            if response["type"] != "ready"
                || response["server_version"] != env!("CARGO_PKG_VERSION")
            {
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
    proxy(native_rx, output, &mut socket, host_id).await
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
    if pairing.tab_ids.iter().any(|id| {
        pairing.expected_urls.get(id).is_none_or(String::is_empty)
            || pairing
                .expected_document_ids
                .get(id)
                .is_none_or(|value| value.is_empty() || value.len() > 128)
    }) {
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
    native_rx: &mut mpsc::Receiver<Result<Value, String>>,
    output: &mut W,
    socket: &mut S,
    host_id: &str,
) -> Result<(), String>
where
    W: AsyncWrite + Unpin,
    S: futures_util::Sink<Message>
        + futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
    S::Error: std::fmt::Display,
{
    let mut tabs_poll = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            native = native_rx.recv() => {
                let message = native.ok_or("native port closed")??;
                match message["type"].as_str() {
                    Some("result") if message["id"].as_u64().is_some() => {
                        socket.send(Message::Text(message.to_string().into())).await.map_err(|error| error.to_string())?;
                    }
                    Some("tabs") if message["request_id"] == "inventory" => save_tabs_snapshot(&message, host_id).await?,
                    Some("tabs_error") if message["request_id"] == "inventory" => {}
                    _ => return Err("unexpected native message during paired session".into()),
                }
            }
            _ = tabs_poll.tick() => {
                write_message(output, &json!({"type":"list_tabs","request_id":"inventory"})).await?;
            }
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if text.len() > MAX_MESSAGE { return Err("provider message exceeds size limit".into()); }
                    let value: Value = serde_json::from_str(&text).map_err(|_| "provider sent invalid JSON".to_owned())?;
                    if value["type"] != "command" { return Err("provider sent an unsupported message".into()); }
                    write_message(output, &value).await?;
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
    if tabs.len() > 100 {
        return Err("extension tabs snapshot exceeds limit".into());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before Unix epoch".to_owned())?
        .as_secs();
    let bytes = serde_json::to_vec(&json!({
        "tabs":tabs,"last_seen_unix":now,"host_id":host_id,
        "truncated":message["truncated"].as_bool().unwrap_or(false)
    }))
    .map_err(|_| "cannot encode tabs snapshot".to_owned())?;
    if bytes.len() > 256_000 {
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
            json!({"type":"list_tabs","request_id":"inventory"})
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
}
