use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn write_native(writer: &mut impl Write, message: Value) {
    let bytes = serde_json::to_vec(&message).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_ne_bytes())
        .unwrap();
    writer.write_all(&bytes).unwrap();
    writer.flush().unwrap();
}

#[test]
fn native_host_lists_tabs_before_pairing_and_recovers_from_bad_request() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let home = std::env::temp_dir().join(format!("controlla-native-{stamp}"));
    let state = home.join(".chrome-controlla");
    fs::create_dir_all(&home).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_controlla"))
        .arg("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
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
            if tx
                .send(serde_json::from_slice::<Value>(&bytes).unwrap())
                .is_err()
            {
                break;
            }
        }
    });
    let next = || rx.recv_timeout(Duration::from_secs(7)).unwrap();
    assert_eq!(next()["type"], "ready");
    assert_eq!(next()["type"], "list_tabs");
    write_native(
        &mut input,
        json!({"type":"tabs","request_id":"inventory","tabs":[
            {"id":42,"title":"Fixture","url":"https://example.test/"}
        ]}),
    );
    let snapshot = state.join("native-tabs.json");
    for _ in 0..50 {
        if snapshot.is_file() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let inventory = serde_json::from_slice::<Value>(&fs::read(snapshot).unwrap()).unwrap();
    assert_eq!(inventory["tabs"][0]["id"], 42);
    let host_id = inventory["host_id"].as_str().unwrap();
    let pairing = state.join(format!("native-pairing-{host_id}.json"));
    fs::write(&pairing, b"bad json").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pairing, fs::Permissions::from_mode(0o600)).unwrap();
    }
    std::thread::sleep(Duration::from_millis(350));
    assert!(
        child.try_wait().unwrap().is_none(),
        "bad request must not kill host"
    );
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60;
    fs::write(
        &pairing,
        json!({"endpoint":"ws://127.0.0.1:12345/","token":"fixture-token",
        "tab_ids":[42],"request_id":"r1","expires_at_unix":expires})
        .to_string(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pairing, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let pair = loop {
        let message = next();
        if message["type"] == "pair" {
            break message;
        }
    };
    assert_eq!(pair["tab_ids"], json!([42]));
    write_native(
        &mut input,
        json!({"type":"pair_error","request_id":"r1","error":"fixture refusal"}),
    );
    let release = loop {
        let message = next();
        if message["type"] == "release" {
            break message;
        }
    };
    assert_eq!(release["type"], "release");
    assert!(!pairing.exists(), "request is consumed before dispatch");
    child.kill().unwrap();
    child.wait().unwrap();
    fs::remove_dir_all(home).unwrap();
}
