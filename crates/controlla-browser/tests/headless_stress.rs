use controlla_browser::{
    providers::DedicatedChromeProvider,
    sessions::{ProviderGrants, SessionMode, SessionRegistry, SessionSpec, StaleTarget},
};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct ExpectedTarget {
    id: String,
    generation: u64,
    revision: String,
}

impl controlla_browser::sessions::IndependentTargetObserver for ExpectedTarget {
    fn verify_unchanged(
        &self,
        observation: &controlla_browser::sessions::CleanupObservation,
    ) -> Result<(), String> {
        if observation.target_id == self.id
            && observation.browser_generation == self.generation
            && observation.target_revision == self.revision
        {
            Ok(())
        } else {
            Err("isolated headless target identity changed".into())
        }
    }
}

#[tokio::test]
#[ignore = "requires installed Google Chrome; launches isolated headless profiles"]
async fn repeated_headless_launch_evaluate_and_cleanup_never_reuses_profile_state() {
    #[cfg(target_os = "macos")]
    let executable = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    #[cfg(target_os = "linux")]
    let executable = Path::new("/usr/bin/google-chrome-stable");
    #[cfg(target_os = "windows")]
    let executable = Path::new("C:/Program Files/Google/Chrome/Application/chrome.exe");
    let configured = std::env::var("CONTROLLA_CHROME").ok();
    let executable = configured.as_deref().map(Path::new).unwrap_or(executable);
    assert!(
        executable.is_file(),
        "Chrome missing at {}",
        executable.display()
    );

    // Serve a real HTTP origin: managed Chrome installations may prohibit
    // data: navigation even when ordinary web pages are allowed. Keeping the
    // same origin across launches also makes localStorage isolation meaningful.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture_origin = format!("http://{}", listener.local_addr().unwrap());
    let fixture_server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let size = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let run = path.strip_prefix("/run-").unwrap_or("unknown");
            let body = format!(
                "<!doctype html><title>run-{run}</title><main aria-label=\"run-{run}\">{run}</main>"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    let provider = DedicatedChromeProvider::new(executable);
    for run in 0..8 {
        let mut registry = SessionRegistry::new(ProviderGrants {
            dedicated_headless: true,
            ..ProviderGrants::default()
        });
        let handle = registry
            .create_session(
                SessionSpec {
                    mode: SessionMode::Headless,
                    selected_target_ids: vec![],
                },
                "headless-stress-fixture",
            )
            .unwrap();
        let html = format!("{fixture_origin}/run-{run}");
        let session = provider
            .launch(&mut registry, &handle, &html)
            .await
            .unwrap();
        let connection = session.connection();
        let target_id = session.target_id().to_owned();

        // A newly launched Chrome can publish an initial root frame and then
        // replace it when the requested document commits. Read-only references
        // are allowed to reacquire across that transition; this stress test is
        // about profile isolation/cleanup, not asserting that the first frame
        // event is already final.
        let mut readiness = "root frame absent".to_owned();
        let observed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let frame_id = loop {
                    if let Some(frame) = connection
                        .frames
                        .read()
                        .await
                        .frames
                        .values()
                        .find(|frame| frame.target_id == target_id && frame.parent_id.is_none())
                    {
                        break frame.id.clone();
                    }
                    tokio::task::yield_now().await;
                };
                readiness = format!("capturing frame {frame_id}");
                let reference = match connection
                    .capture_target_ref(&registry, &handle, &target_id, &frame_id, 1, 1)
                    .await
                {
                    Ok(reference) => reference,
                    Err(StaleTarget::FrameChanged) => {
                        readiness = "capture frame changed".to_owned();
                        tokio::task::yield_now().await;
                        continue;
                    }
                    Err(error) => {
                        panic!("headless run {run} failed to capture target: {error:?}")
                    }
                };
                readiness = format!("observing frame {frame_id}");
                match connection
                    .observe_accessibility(
                        &registry,
                        &reference,
                        &handle.principal,
                        controlla_browser::sessions::IdentityRevisions {
                            account: 1,
                            document: 1,
                        },
                        "main",
                        65_536,
                    )
                    .await
                {
                    Ok(observed) => {
                        let isolation = connection.target_ref_command(
                            &registry, &reference, &handle.principal,
                            controlla_browser::sessions::IdentityRevisions { account: 1, document: 1 },
                            "Runtime.evaluate",
                            serde_json::json!({
                                "expression": "(() => { const previous = localStorage.getItem('headless-stress'); localStorage.setItem('headless-stress', document.title); return {previous, stored: localStorage.getItem('headless-stress'), title: document.title}; })()",
                                "returnByValue": true
                            }),
                        ).await;
                        break (observed, isolation);
                    },
                    Err(error)
                        if error.to_string().contains("FrameChanged")
                            || error
                                .to_string()
                                .contains("AX selector matched no DOM node") =>
                    {
                        readiness = error.to_string();
                        // Chrome can commit the root frame before its first
                        // document nodes are queryable. This is a read-only
                        // readiness miss, so reacquire and retry within the
                        // enclosing five-second startup bound.
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("headless run {run} observation failed: {error}"),
                }
            }
        })
        .await;
        if observed.is_err() {
            eprintln!(
                "headless readiness: {readiness}; frames: {:?}",
                connection.frames.read().await.frames
            );
        }
        let (generation, targets) = connection.target_snapshot().await;
        let revision = targets
            .iter()
            .find(|target| target.id == target_id)
            .unwrap()
            .revision
            .clone();
        let observer = ExpectedTarget {
            id: target_id,
            generation,
            revision,
        };
        let cleanup = session.shutdown(&mut registry, Some(&observer)).await;
        // Always release the disposable browser, including a failed readiness
        // assertion, so a failing stress run cannot leave profile/process state.
        let (observed, isolation) = observed
            .unwrap_or_else(|_| panic!("headless root frame did not stabilize: {readiness}"));
        assert!(
            !observed.nodes.is_empty(),
            "headless run {run} missed its fixture main element"
        );
        assert_eq!(observed.nodes[0]["name"]["value"], format!("run-{run}"));
        let isolation = isolation.expect("headless fixture evaluation failed");
        assert!(isolation.get("exceptionDetails").is_none(), "{isolation:?}");
        let value = &isolation["result"]["value"];
        assert_eq!(value["title"], format!("run-{run}"));
        assert!(
            value["previous"].is_null(),
            "run {run} reused profile storage: {value:?}"
        );
        assert_eq!(value["stored"], format!("run-{run}"));
        assert!(
            cleanup.cleanup_error.is_none(),
            "run {run}: {:?}",
            cleanup.cleanup_error
        );
        assert!(
            cleanup.recovery.is_none(),
            "run {run} retained a Chrome process/profile"
        );
    }
    fixture_server.abort();
}
