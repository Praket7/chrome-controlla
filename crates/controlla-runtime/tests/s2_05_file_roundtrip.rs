//! Local contract fixture for Stage 2 S2-05. This exercises byte integrity and
//! unknown-delivery idempotency against a local receipt server; it does not
//! claim to exercise Chrome, the extension, or Controlla's live upload route.
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const FIXTURE: &[u8] = include_bytes!("fixtures/s2-05-roundtrip.bin");
const FILENAME: &str = "acceptance-fixture.bin";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

#[derive(Default)]
struct State {
    uploads: HashMap<(String, String), (Vec<u8>, String)>,
    attempts: HashMap<String, usize>,
}

async fn serve_one(mut stream: TcpStream, state: Arc<Mutex<State>>) {
    let mut request = Vec::new();
    let mut buf = [0; 4096];
    let (head_end, content_length) = loop {
        let n = stream.read(&mut buf).await.unwrap();
        assert!(n > 0, "client closed before request headers");
        request.extend_from_slice(&buf[..n]);
        if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..pos]);
            let len = headers
                .lines()
                .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            break (pos + 4, len);
        }
    };
    while request.len() < head_end + content_length {
        let n = stream.read(&mut buf).await.unwrap();
        assert!(n > 0, "client closed before request body");
        request.extend_from_slice(&buf[..n]);
    }
    let head = String::from_utf8_lossy(&request[..head_end]);
    let body = &request[head_end..head_end + content_length];
    let (status, content_type, response) = if head.starts_with("GET /fixture ") {
        ("200 OK", "application/octet-stream", FIXTURE.to_vec())
    } else if head.starts_with("POST /upload/ shared ")
        || head.starts_with("POST /upload/shared ")
        || head.starts_with("POST /upload/headless ")
    {
        let mode = if head.contains("/upload/shared ") {
            "shared"
        } else {
            "headless"
        };
        let key = head
            .lines()
            .find_map(|line| {
                let (k, v) = line.split_once(':')?;
                k.eq_ignore_ascii_case("idempotency-key")
                    .then(|| v.trim().to_owned())
            })
            .expect("upload idempotency key");
        let filename = head
            .lines()
            .find_map(|line| {
                let (k, v) = line.split_once(':')?;
                k.eq_ignore_ascii_case("x-filename")
                    .then(|| v.trim().to_owned())
            })
            .expect("upload filename");
        let mut state = state.lock().unwrap();
        *state.attempts.entry(mode.into()).or_default() += 1;
        let entry = state
            .uploads
            .entry((mode.into(), key))
            .or_insert_with(|| (body.to_vec(), digest(body)));
        assert_eq!(entry.0, body, "same idempotency key cannot change payload");
        let receipt = format!(
            "{{\"mode\":\"{mode}\",\"filename\":\"{filename}\",\"size\":{},\"sha256\":\"{}\",\"accepted\":true}}",
            entry.0.len(),
            entry.1
        );
        ("200 OK", "application/json", receipt.into_bytes())
    } else {
        ("404 Not Found", "text/plain", b"not found".to_vec())
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    );
    stream.write_all(header.as_bytes()).await.unwrap();
    stream.write_all(&response).await.unwrap();
}

async fn request(
    addr: SocketAddr,
    method_path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Vec<u8> {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut head = format!(
        "{method_path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    response[split..].to_vec()
}

#[tokio::test]
async fn s2_05_fixture_bytes_roundtrip_in_shared_and_headless_modes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        for _ in 0..5 {
            let (stream, _) = listener.accept().await.unwrap();
            serve_one(stream, server_state.clone()).await;
        }
    });

    for mode in ["shared", "headless"] {
        let downloaded = request(addr, "GET /fixture", &[], &[]).await;
        assert_eq!(
            downloaded, FIXTURE,
            "downloaded bytes changed in {mode} mode"
        );
        let inspected_filename = FILENAME;
        let inspected_size = downloaded.len();
        let inspected_sha256 = digest(&downloaded);
        assert_eq!(inspected_filename, FILENAME);
        assert_eq!(inspected_size, FIXTURE.len());
        assert_eq!(inspected_sha256, digest(FIXTURE));

        let key = format!("s2-05-{mode}-same-operation");
        let path = format!("POST /upload/{mode}");
        let headers = [("Idempotency-Key", key.as_str()), ("X-Filename", FILENAME)];
        let receipt = request(addr, &path, &headers, &downloaded).await;
        let value: serde_json::Value = serde_json::from_slice(&receipt).unwrap();
        assert_eq!(value["mode"], mode);
        assert_eq!(value["filename"], FILENAME);
        assert_eq!(value["size"], FIXTURE.len());
        assert_eq!(value["sha256"], inspected_sha256);
        assert_eq!(value["accepted"], true);
        if mode == "shared" {
            // Simulate a lost response / unknown completion: reconcile using the same
            // operation key. The fixture server returns the original receipt and stores
            // only one accepted upload effect.
            let replay = request(addr, &path, &headers, &downloaded).await;
            assert_eq!(replay, receipt);
        }
    }
    server.await.unwrap();
    let state = state.lock().unwrap();
    for mode in ["shared", "headless"] {
        let (bytes, hash) = state
            .uploads
            .get(&(mode.into(), format!("s2-05-{mode}-same-operation")))
            .unwrap();
        assert_eq!(
            bytes, FIXTURE,
            "server received different bytes in {mode} mode"
        );
        assert_eq!(
            hash,
            &digest(FIXTURE),
            "server independently hashed different bytes in {mode} mode"
        );
        assert_eq!(state.attempts[mode], if mode == "shared" { 2 } else { 1 });
    }
    assert_eq!(
        state.uploads.len(),
        2,
        "unknown reconciliation must not duplicate accepted effects"
    );
}
