use controlla_browser::{
    providers::DedicatedChromeProvider,
    sessions::{ProviderGrants, SessionMode, SessionRegistry, SessionSpec},
};
use std::path::Path;

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
    assert!(
        executable.is_file(),
        "Chrome missing at {}",
        executable.display()
    );

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
        let html =
            format!("data:text/html,%3Ctitle%3Erun-{run}%3C/title%3E%3Cmain%3E{run}%3C/main%3E");
        let session = provider
            .launch(&mut registry, &handle, &html)
            .await
            .unwrap();
        let connection = session.connection();
        let target_id = session.target_id().to_owned();
        let frame_id = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
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
            }
        })
        .await
        .unwrap();
        let reference = connection
            .capture_target_ref(&registry, &handle, &target_id, &frame_id, 1, 1)
            .await
            .unwrap();
        let observed = connection
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
            .unwrap();
        assert!(
            !observed.nodes.is_empty(),
            "headless run {run} missed its fixture main element"
        );
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
}
