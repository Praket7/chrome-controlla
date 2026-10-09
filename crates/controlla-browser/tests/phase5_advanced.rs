use controlla_browser::{
    ExpansionControl, ExtractionSpec, ScreenshotCrop,
    providers::DedicatedChromeProvider,
    sessions::{IdentityRevisions, ProviderGrants, SessionMode, SessionRegistry, SessionSpec},
};
use std::{collections::BTreeMap, path::Path, time::Duration};

#[tokio::test]
#[ignore = "requires installed Google Chrome; uses an isolated headless profile"]
async fn chrome_ax_crop_hidden_sections_and_resumable_extraction() {
    let executable = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    assert!(executable.is_file());
    let html = r#"<!doctype html><meta charset="utf-8"><button id="open" aria-expanded="false">Open</button><section id="bucket" hidden><div id="list"><div id="rows"></div><span class="end">end</span></div></section><button id="blocked" aria-expanded="false" disabled>Blocked</button><main id="blocked-content" hidden>no</main><div id="account">fixture-user</div><script>
      const open=document.querySelector('#open');open.onclick=()=>{open.setAttribute('aria-expanded','true');document.querySelector('#bucket').hidden=false};
      const rows=document.querySelector('#rows'),list=document.querySelector('#list');list.style.cssText='height:100px;overflow:auto';rows.style.height='320px';for(let i=1;i<=8;i++){let r=document.createElement('article');r.className='row';r.style.height='40px';r.innerHTML='<span class="id">r'+i+'</span>';rows.append(r)}
    </script>"#;
    let encoded = html
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect::<String>();
    let mut registry = SessionRegistry::new(ProviderGrants {
        dedicated_headless: true,
        ..Default::default()
    });
    let handle = registry
        .create_session(
            SessionSpec {
                mode: SessionMode::Headless,
                selected_target_ids: vec![],
            },
            "phase5-advanced-fixture",
        )
        .unwrap();
    let provider = DedicatedChromeProvider::new(executable);
    let session = provider
        .launch(&mut registry, &handle, &format!("data:text/html,{encoded}"))
        .await
        .unwrap();
    let connection = session.connection().clone();
    let target_id = session.target_id().to_owned();
    let frame_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(frame) = connection
                .frames
                .read()
                .await
                .frames
                .values()
                .find(|f| f.target_id == target_id && f.parent_id.is_none())
            {
                break frame.id.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let reference = connection
        .capture_target_ref(&registry, &handle, &target_id, &frame_id, 9, 2)
        .await
        .unwrap();
    let revisions = IdentityRevisions {
        account: 9,
        document: 2,
    };
    let accessibility = connection
        .observe_accessibility(
            &registry,
            &reference,
            &handle.principal,
            revisions,
            "#open",
            65_536,
        )
        .await
        .unwrap();
    assert!(!accessibility.nodes.is_empty());
    let screenshot = connection
        .observe_screenshot(
            &registry,
            &reference,
            &handle.principal,
            revisions,
            ScreenshotCrop {
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
                scale: 1.0,
            },
            65_536,
        )
        .await
        .unwrap();
    assert!(!screenshot.data_base64.is_empty());
    let make_spec = |expand: Vec<ExpansionControl>, cursor| ExtractionSpec {
        container: "#list".into(),
        record: ".row".into(),
        fields: BTreeMap::from([("id".into(), ".id".into())]),
        id_field: "id".into(),
        max_steps: 1,
        max_records: 20,
        max_text_chars: 100,
        max_bytes: 16_384,
        expected_count: Some(8),
        account_marker: Some(("#account".into(), "fixture-user".into())),
        terminal_selector: Some(".end".into()),
        expand,
        cursor,
    };
    let first = connection
        .extract(
            &registry,
            &reference,
            &handle.principal,
            revisions,
            &make_spec(
                vec![ExpansionControl {
                    selector: "#open".into(),
                    content_selector: "#bucket .row".into(),
                }],
                None,
            ),
        )
        .await
        .unwrap();
    assert!(first.cursor_is_resumable);
    let mut changed_reference = reference.clone();
    changed_reference.target_revision.push_str("-stale");
    let stale_cursor = connection
        .extract(
            &registry,
            &changed_reference,
            &handle.principal,
            revisions,
            &make_spec(
                vec![ExpansionControl {
                    selector: "#open".into(),
                    content_selector: "#bucket .row".into(),
                }],
                first.cursor.clone(),
            ),
        )
        .await;
    assert!(
        stale_cursor.is_err(),
        "cursor accepted a changed target revision"
    );
    let mut cursor = first.cursor;
    let mut final_result = None;
    for _ in 0..6 {
        let result = connection
            .extract(
                &registry,
                &reference,
                &handle.principal,
                revisions,
                &make_spec(
                    vec![ExpansionControl {
                        selector: "#open".into(),
                        content_selector: "#bucket .row".into(),
                    }],
                    cursor,
                ),
            )
            .await
            .unwrap();
        cursor = result.cursor.clone();
        if !result.cursor_is_resumable {
            final_result = Some(result);
            break;
        }
    }
    let complete = final_result.expect("bounded resumes did not finish");
    assert_eq!(complete.unique_count, 8);
    assert_eq!(
        complete.completeness,
        controlla_browser::Completeness::Complete,
        "{complete:?}"
    );
    let blocked = connection
        .extract(
            &registry,
            &reference,
            &handle.principal,
            revisions,
            &ExtractionSpec {
                container: "#blocked-content".into(),
                record: ".never".into(),
                fields: BTreeMap::from([("id".into(), ".id".into())]),
                id_field: "id".into(),
                max_steps: 1,
                max_records: 2,
                max_text_chars: 32,
                max_bytes: 4096,
                expected_count: Some(1),
                account_marker: Some(("#account".into(), "fixture-user".into())),
                terminal_selector: Some(".end".into()),
                expand: vec![ExpansionControl {
                    selector: "#blocked".into(),
                    content_selector: "#blocked-content .row".into(),
                }],
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        blocked.completeness,
        controlla_browser::Completeness::Unknown
    );
    assert!(
        blocked
            .missing
            .iter()
            .any(|m| m.contains("expansion blocked")),
        "{blocked:?}"
    );
    struct FixtureObserver;
    impl controlla_browser::sessions::IndependentTargetObserver for FixtureObserver {
        fn verify_unchanged(
            &self,
            _: &controlla_browser::sessions::CleanupObservation,
        ) -> Result<(), String> {
            Ok(())
        }
    }
    let outcome = session
        .shutdown(&mut registry, Some(&FixtureObserver))
        .await;
    assert!(
        outcome.cleanup_error.is_none(),
        "{:?}",
        outcome.cleanup_error
    );
}
