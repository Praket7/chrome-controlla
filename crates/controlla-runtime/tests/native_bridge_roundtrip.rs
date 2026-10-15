use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio_tungstenite::{accept_async, tungstenite::Message};

const TIMEOUT: Duration = Duration::from_secs(8);

fn write_native(writer: &mut impl Write, message: Value) {
    let bytes = serde_json::to_vec(&message).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_ne_bytes())
        .unwrap();
    writer.write_all(&bytes).unwrap();
    writer.flush().unwrap();
}

fn stop(child: &mut Child) {
    if child.try_wait().unwrap().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[test]
fn native_host_roundtrips_authenticated_provider_command_and_releases() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let home = std::env::temp_dir().join(format!("controlla-roundtrip-{stamp}"));
    let state = home.join(".chrome-controlla");
    fs::create_dir_all(&state).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let (provider_tx, provider_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let provider = thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            runtime.block_on(async move {
                let (stream, _) = tokio::time::timeout(TIMEOUT, tokio::task::spawn_blocking(move || loop { match listener.accept() { Ok(v) => break Ok(v), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(10)), Err(e) => break Err(e) } })).await.map_err(|_| "provider accept timed out".to_owned())?.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
                stream.set_nonblocking(true).map_err(|e| e.to_string())?;
                let stream = tokio::net::TcpStream::from_std(stream).map_err(|e| e.to_string())?;
                let mut socket = accept_async(stream).await.map_err(|e| e.to_string())?;
                let hello = tokio::time::timeout(TIMEOUT, socket.next()).await.map_err(|_| "hello timed out".to_owned())?.ok_or("provider socket closed before hello")?.map_err(|e| e.to_string())?;
                let hello: Value = serde_json::from_str(hello.to_text().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                if hello != json!({"type":"hello","token":"fixture-secret","targets":["42"],"extension_version":env!("CARGO_PKG_VERSION"),"extension_build_id":env!("CONTROLLA_EXTENSION_BUILD_ID"),"document_identity":true,"protocol_version":2,"batch_execution":true,"batch_deadline":true}) {
                    return Err(format!("unexpected provider hello: {hello}"));
                }
                socket.send(Message::Text(json!({"type":"ready","server_version":env!("CARGO_PKG_VERSION"),"protocol_version":2}).to_string().into())).await.map_err(|e| e.to_string())?;
                ready_tx.send(()).map_err(|e| e.to_string())?;
                socket.send(Message::Text(json!({"type":"command","id":7,"target_id":"42","method":"Runtime.evaluate","params":{"expression":"1+1"}}).to_string().into())).await.map_err(|e| e.to_string())?;
                let result = tokio::time::timeout(TIMEOUT, socket.next()).await.map_err(|_| "command result timed out".to_owned())?.ok_or("provider socket closed before result")?.map_err(|e| e.to_string())?;
                let result: Value = serde_json::from_str(result.to_text().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                if result != json!({"type":"result","id":7,"result":{"result":{"type":"number","value":2}}}) {
                    return Err(format!("unexpected command result: {result}"));
                }
                socket.send(Message::Text(json!({"type":"command","id":9,"target_id":"42","method":"Page.captureScreenshot","params":{"format":"png","fromSurface":true,"captureBeyondViewport":false,"clip":{"x":0,"y":0,"width":320,"height":180,"scale":1}}}).to_string().into())).await.map_err(|e| e.to_string())?;
                let screenshot = tokio::time::timeout(TIMEOUT, socket.next()).await.map_err(|_| "screenshot result timed out".to_owned())?.ok_or("provider socket closed before screenshot result")?.map_err(|e| e.to_string())?;
                let screenshot: Value = serde_json::from_str(screenshot.to_text().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                let mut png = vec![0_u8; 393_219];
                png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
                png[8..12].copy_from_slice(&13_u32.to_be_bytes());
                png[12..16].copy_from_slice(b"IHDR");
                png[16..20].copy_from_slice(&320_u32.to_be_bytes());
                png[20..24].copy_from_slice(&180_u32.to_be_bytes());
                let expected_data = base64::engine::general_purpose::STANDARD.encode(&png);
                let expected_image_digest = Sha256::digest(&png)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let expected_visual_evidence = json!({"target_id":"42","document_id":"document-42","url":"https://example.test/","captured_at_ms":1001,"capture_started_at_ms":1000,"visual_revision":expected_image_digest,"image_sha256":expected_image_digest,"target_lifecycle":"active","render_revision":null,"dimensions":{"width":320,"height":180}});
                if screenshot != json!({"type":"result","id":9,"lease_id":"roundtrip-1","result":{"data":expected_data,"visual_evidence":expected_visual_evidence}}) {
                    return Err(format!("unexpected native-host screenshot assembly: id={}, data_len={}", screenshot["id"], screenshot["result"]["data"].as_str().map(str::len).unwrap_or(0)));
                }
                socket.send(Message::Text(json!({"type":"batch","id":8,"target_id":"42","actions":[{"method":"Input.dispatchKeyEvent","params":{"type":"keyDown","key":"x"}}],"deadline_ms":5000}).to_string().into())).await.map_err(|e| e.to_string())?;
                let batch = tokio::time::timeout(TIMEOUT, socket.next()).await.map_err(|_| "batch result timed out".to_owned())?.ok_or("provider socket closed before batch result")?.map_err(|e| e.to_string())?;
                let batch: Value = serde_json::from_str(batch.to_text().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                if batch != json!({"type":"batch_result","id":8,"result":{"completed":1}}) {
                    return Err(format!("unexpected batch result: {batch}"));
                }
                Ok(())
            })
        })();
        if let Err(error) = &result {
            let _ = err_tx.send(error.clone());
        }
        let _ = provider_tx.send(result);
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_controlla"))
        .arg("chrome-extension://abcdefghijklmnopabcdefghijklmnop")
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let mut input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        let _ = stderr_tx.send(text);
    });
    let (native_tx, native_rx) = mpsc::channel();
    thread::spawn(move || {
        loop {
            let mut length = [0; 4];
            if output.read_exact(&mut length).is_err() {
                break;
            }
            let length = u32::from_ne_bytes(length) as usize;
            if length > 1_048_576 {
                break;
            }
            let mut bytes = vec![0; length];
            if output.read_exact(&mut bytes).is_err() {
                break;
            }
            if native_tx
                .send(serde_json::from_slice::<Value>(&bytes).unwrap())
                .is_err()
            {
                break;
            }
        }
    });
    let next = || native_rx.recv_timeout(TIMEOUT).unwrap();

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_eq!(next()["type"], "ready");
        assert_eq!(next()["type"], "list_tabs");
        write_native(
            &mut input,
            json!({"type":"tabs","request_id":"inventory","tabs":[{"id":42,"title":"Fixture","url":"https://example.test/"}]}),
        );
        let snapshot = state.join("native-tabs.json");
        for _ in 0..100 {
            if snapshot.is_file() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let inventory: Value = serde_json::from_slice(&fs::read(snapshot).unwrap()).unwrap();
        let host_id = inventory["host_id"].as_str().unwrap();
        let pairing = state.join(format!("native-pairing-{host_id}.json"));
        fs::write(&pairing, json!({"endpoint":format!("ws://127.0.0.1:{}/", address.port()),"token":"fixture-secret","tab_ids":[42],"request_id":"roundtrip-1","expected_urls":{"42":"https://example.test/"},"expected_document_ids":{"42":"document-42"},"expires_at_unix":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()+60}).to_string()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&pairing, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let pair = loop {
            let msg = next();
            if msg["type"] == "pair" {
                break msg;
            }
        };
        assert_eq!(
            pair,
            json!({"type":"pair","request_id":"roundtrip-1","lease_id":"roundtrip-1","tab_ids":[42],"expected_urls":{"42":"https://example.test/"},"expected_document_ids":{"42":"document-42"},"unrestricted":false})
        );
        write_native(
            &mut input,
            json!({"type":"paired","request_id":"roundtrip-1","targets":["42"],"extension_version":env!("CARGO_PKG_VERSION"),"extension_build_id":env!("CONTROLLA_EXTENSION_BUILD_ID"),"protocol_version":2,"document_identity":true,"batch_execution":true,"batch_deadline":true}),
        );
        ready_rx.recv_timeout(TIMEOUT).unwrap_or_else(|e| {
            let _ = std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .status();
            panic!(
                "provider handshake failed: {e}; provider error: {:?}; host stderr: {:?}",
                err_rx.try_recv(),
                stderr_rx.try_recv()
            )
        });
        let command = loop {
            let msg = next();
            if msg["type"] == "command" {
                break msg;
            }
        };
        assert_eq!(command["id"], 7);
        assert_eq!(command["target_id"], "42");
        write_native(
            &mut input,
            json!({"type":"result","id":7,"result":{"result":{"type":"number","value":2}}}),
        );
        let screenshot_command = loop {
            let msg = next();
            if msg["type"] == "command" && msg["id"] == 9 {
                break msg;
            }
        };
        assert_eq!(screenshot_command["method"], "Page.captureScreenshot");
        let mut png = vec![0_u8; 393_219];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[8..12].copy_from_slice(&13_u32.to_be_bytes());
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&320_u32.to_be_bytes());
        png[20..24].copy_from_slice(&180_u32.to_be_bytes());
        let screenshot_data = base64::engine::general_purpose::STANDARD.encode(&png);
        let digest = Sha256::digest(screenshot_data.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let image_digest = Sha256::digest(&png)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let visual_evidence = json!({"target_id":"42","document_id":"document-42","url":"https://example.test/","captured_at_ms":1001,"capture_started_at_ms":1000,"visual_revision":image_digest,"image_sha256":image_digest,"target_lifecycle":"active","render_revision":null,"dimensions":{"width":320,"height":180}});
        write_native(
            &mut input,
            json!({"type":"result_chunk","id":9,"lease_id":"roundtrip-1","transfer_id":"native-survival","index":1,"total_chunks":2,"total_chars":screenshot_data.len(),"base64_sha256":digest,"visual_evidence":visual_evidence,"chunk":&screenshot_data[512 * 1024..]}),
        );
        write_native(
            &mut input,
            json!({"type":"result_chunk","id":9,"lease_id":"roundtrip-1","transfer_id":"native-survival","index":0,"total_chunks":2,"total_chars":screenshot_data.len(),"base64_sha256":digest,"visual_evidence":visual_evidence,"chunk":&screenshot_data[..512 * 1024]}),
        );
        let batch_deadline = Instant::now() + TIMEOUT;
        let batch = loop {
            assert!(
                Instant::now() < batch_deadline,
                "native host did not forward provider batch; provider error: {:?}; host stderr: {:?}",
                err_rx.try_recv(),
                stderr_rx.try_recv()
            );
            let msg = next();
            if msg["type"] == "batch" {
                break msg;
            }
        };
        assert_eq!(batch["id"], 8);
        assert_eq!(batch["actions"].as_array().unwrap().len(), 1);
        write_native(
            &mut input,
            json!({"type":"batch_result","id":8,"result":{"completed":1}}),
        );
        provider_rx.recv_timeout(TIMEOUT).unwrap().unwrap();
        assert_eq!(
            loop {
                let msg = next();
                if msg["type"] == "release" {
                    break msg;
                }
            }["type"],
            "release"
        );
        assert!(
            child.try_wait().unwrap().is_none(),
            "host must remain alive after release"
        );
        assert!(!pairing.exists(), "pairing file is consumed");
    }));
    stop(&mut child);
    let _ = fs::remove_dir_all(&home);
    provider.join().unwrap();
    outcome.unwrap();
}
