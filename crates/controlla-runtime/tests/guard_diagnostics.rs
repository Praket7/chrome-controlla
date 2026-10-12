//! One regression per guard predicate. Each case builds a tiny DOM in real headless Chrome,
//! takes the production semantic snapshot, optionally mutates the page, then runs the
//! production click guard and asserts the exact reason code, retry class, and detail safety.
#![cfg(target_os = "macos")]

use controlla_runtime::guard_reason::{GuardReason, RetryClass, click_guard_script};
use std::{path::PathBuf, process::Command};

const SNAPSHOT_JS: &str = include_str!("../src/shared_page/snapshot.js");

fn chrome() -> PathBuf {
    let path = PathBuf::from(
        std::env::var("COMPTROL_CHROME_EXECUTABLE").unwrap_or_else(|_| {
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into()
        }),
    );
    assert!(
        path.exists(),
        "installed Chrome required: {}",
        path.display()
    );
    path
}

/// Runs every case once inside one headless page (cached) and returns `{case: guard_result}`.
fn run_cases() -> serde_json::Value {
    static RESULTS: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    RESULTS.get_or_init(run_cases_uncached).clone()
}

fn run_cases_uncached() -> serde_json::Value {
    let guard = click_guard_script(controlla_runtime::mcp::SHARED_CLICK_UNSAFE_PREDICATE);
    let page = format!(
        r##"<!doctype html><html><head><meta charset="utf-8"><style>body{{margin:0}}</style></head><body><main id="stage"></main><pre id="out"></pre>
<script>
try {{
const stage = document.getElementById('stage');
const snapshotFn = {SNAPSHOT_JS};
const guardFn = {guard};
const cases = {{
  ok: {{html:'<button>Go</button>'}},
  unsafe_page: {{html:'<input type="checkbox" aria-label="Go">'}},
  stale_reference: {{html:'<button>Go</button>', index:99}},
  changed_target: {{html:'<button aria-label="Go">Go</button>', mutate:()=>stage.querySelector('button').setAttribute('aria-label','Other')}},
  hidden: {{html:'<button>Go</button>', mutate:()=>stage.querySelector('button').style.display='none'}},
  disabled: {{html:'<button>Go</button>', mutate:()=>stage.querySelector('button').disabled=true}},
  aria_disabled: {{html:'<button>Go</button>', mutate:()=>stage.querySelector('button').setAttribute('aria-disabled','true')}},
  masked_boundary: {{html:'<div><button>Go</button></div>', mutate:()=>stage.querySelector('div').setAttribute('data-masked','')}},
  trusted_boundary: {{html:'<div><button>Go</button></div>', mutate:()=>stage.querySelector('div').setAttribute('data-requires-trusted','')}},
  pointer_events_none: {{html:'<button>Go</button>', mutate:()=>stage.querySelector('button').style.pointerEvents='none'}},
  outside_viewport: {{html:'<div style="height:4000px"></div><button>Go</button>', opts:{{scroll:'none'}}}},
  no_hit_target: {{html:'<button>Go</button>', mutate:()=>{{document.elementFromPoint=()=>null;}}}},
  obscured: {{html:'<button>Go</button>', mutate:()=>stage.insertAdjacentHTML('beforeend','<div role="dialog" style="position:fixed;inset:0;z-index:9999;background:#0008">SECRET-OVERLAY-TEXT</div>')}},
  obscured_protected: {{html:'<button>Go</button>', mutate:()=>stage.insertAdjacentHTML('beforeend','<div data-masked style="position:fixed;inset:0;z-index:9999">MASKED-SENTINEL</div>')}},
  detached: {{html:'<button>Go</button>', mutate:()=>stage.querySelector('button').remove()}},
  rerender_unique: {{html:'<button>Continue</button>', mutate:()=>{{const old=stage.querySelector('button');old.replaceWith(old.cloneNode(true));}}}},
  rerender_ambiguous: {{html:'<button>Continue</button>', mutate:()=>{{const old=stage.querySelector('button');old.replaceWith(old.cloneNode(true),old.cloneNode(true));}}}},
  document_changed: {{html:'<button>Go</button>', mutate:()=>{{const d=document.implementation.createHTMLDocument('x');d.body.appendChild(stage.querySelector('button'));}}}},
}};
const results = {{}};
const realFromPoint = document.elementFromPoint.bind(document);
for (const [name, c] of Object.entries(cases)) {{
  document.elementFromPoint = realFromPoint;
  stage.innerHTML = c.html;
  scrollTo(0, 0);
  const ctx = snapshotFn.call(null, {{selector:'body', token:'t', max_items:50, max_bytes:200000, max_text_chars:1000}});
  if (c.mutate) c.mutate();
  const index = c.index ?? 0;
  try {{ results[name] = guardFn.call(ctx, index, c.outcome || {{kind:'navigation'}}, c.opts); }}
  catch (error) {{ results[name] = {{threw: String(error)}}; }}
}}
const capture = (html, opts={{}}) => {{
  stage.innerHTML = html;
  const result = snapshotFn.call(null, {{selector:'body', token:'coverage', max_items:opts.max_items ?? 50,
    max_bytes:200000, max_text_chars:opts.max_text_chars ?? 1000, href_hint:opts.href_hint}});
  return result.public;
}};
results.snapshot_completeness = {{
  full: capture('<button>Go</button>'),
  text: capture('<p>'+('long text '.repeat(20))+'</p>', {{max_text_chars:8}}),
  items: capture('<button>One</button><button>Two</button>', {{max_items:1}}),
  frame: capture('<iframe src="https://example.com/"></iframe>'),
  scoped: capture('<a href="/story/latest">Latest story</a><iframe src="https://example.com/"></iframe>', {{href_hint:'story'}})
}};
results.presentation = capture('<button style="position:absolute;left:-10000px;top:0">Offscreen</button>');
results.editors = capture('<div contenteditable="true" aria-label="Announcement" style="width:100px;height:20px">Hello</div><div contenteditable="true" aria-label="Second" style="width:100px;height:20px"></div><div contenteditable="false" aria-label="Read only">No</div><div contenteditable="false" role="textbox" aria-label="Readonly role textbox">Locked role</div><div role="textbox" aria-label="Custom textbox" aria-multiline="true" style="width:100px;height:20px"></div><input role="textbox" aria-label="Native textbox" value="Plain value"><div role="textbox" aria-label="Readonly ARIA" aria-readonly="true">Locked</div><input type="checkbox" aria-label="Checkbox"><div contenteditable="true" aria-label="Masked" style="width:100px;height:20px"><span data-masked>SECRET</span></div><label for="password">Password</label><input id="password" type="password" value="PASSWORD-SENTINEL"><input type="file" aria-label="File">');
results.empty_editor = capture('<div contenteditable="true" aria-label="Empty" style="width:100px;height:20px"><br></div>');
stage.innerHTML = '<div id="shadow-host"></div>';
stage.querySelector('#shadow-host').attachShadow({{mode:'open'}}).innerHTML = '<div contenteditable="true" aria-label="Shadow editor" style="width:100px;height:20px"></div>';
results.shadow_editor = snapshotFn.call(null, {{selector:'#stage', token:'shadow', max_items:50, max_bytes:200000, max_text_chars:1000}}).public;
const originalGetComputedStyle = window.getComputedStyle.bind(window);
let styleReads = 0;
window.getComputedStyle = (...args) => {{ styleReads++; return originalGetComputedStyle(...args); }};
stage.innerHTML = '<span></span>'.repeat(200)+'<button></button>';
snapshotFn.call(null, {{selector:'#stage', token:'perf', max_items:50, max_bytes:200000, max_text_chars:1000}});
results.presentation_style_reads = styleReads;
window.getComputedStyle = originalGetComputedStyle;
document.getElementById('out').textContent = JSON.stringify(results);
}} catch (error) {{ document.getElementById('out').textContent = JSON.stringify({{page_error:String(error)}}); }}
</script></body></html>"##
    );
    let dir = std::env::temp_dir().join(format!("controlla-guard-diag-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("guard.html");
    std::fs::write(&file, &page).unwrap();
    if let Ok(copy) = std::env::var("CONTROLLA_GUARD_HTML_COPY") {
        std::fs::write(copy, &page).unwrap();
    }
    // Chrome helpers can outlive the main process and hold inherited pipes open, so write the
    // DOM to a file and never wait on a pipe.
    let dump = dir.join("dom.txt");
    let mut child = Command::new(chrome())
        .args([
            "--headless=new",
            "--disable-gpu",
            "--no-first-run",
            "--window-size=800,600",
            "--virtual-time-budget=3000",
        ])
        .arg(format!("--user-data-dir={}", dir.join("profile").display()))
        .arg("--dump-dom")
        .arg(format!("file://{}", file.display()))
        .stdout(std::fs::File::create(&dump).unwrap())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("launch Chrome");
    let started = std::time::Instant::now();
    let dom = loop {
        let dom = std::fs::read_to_string(&dump).unwrap_or_default();
        if let Some(start) = dom.find("<pre id=\"out\">") {
            let output = &dom[start + 14..];
            if output
                .find("</pre>")
                .is_some_and(|end| !output[..end].is_empty())
            {
                let _ = child.kill();
                let _ = child.wait();
                break dom;
            }
        }
        if child.try_wait().unwrap().is_some() {
            break dom;
        }
        if started.elapsed() >= std::time::Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Chrome guard page timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let _ = std::fs::remove_dir_all(&dir);
    let start = dom.find("<pre id=\"out\">").unwrap_or_else(|| {
        panic!(
            "guard output element missing; stdout head: {}",
            &dom[..dom.len().min(800)]
        )
    }) + 14;
    let end = start + dom[start..].find("</pre>").expect("guard output end");
    let json = dom[start..end]
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    serde_json::from_str(&json).unwrap_or_else(|error| panic!("{error}: {json}"))
}

#[test]
#[ignore]
fn every_guard_predicate_reports_its_own_reason_code() {
    let results = run_cases();
    assert_eq!(results["ok"]["ok"], true, "{}", results["ok"]);
    assert_eq!(
        results["rerender_unique"]["ok"], true,
        "{}",
        results["rerender_unique"]
    );
    assert_eq!(
        results["rerender_ambiguous"]["reason"], "detached",
        "{}",
        results["rerender_ambiguous"]
    );
    let expected = [
        ("unsafe_page", GuardReason::UnsafePage),
        ("stale_reference", GuardReason::StaleReference),
        ("changed_target", GuardReason::ChangedTarget),
        ("hidden", GuardReason::Hidden),
        ("disabled", GuardReason::Disabled),
        ("aria_disabled", GuardReason::AriaDisabled),
        ("masked_boundary", GuardReason::MaskedBoundary),
        ("trusted_boundary", GuardReason::TrustedBoundary),
        ("pointer_events_none", GuardReason::PointerEventsNone),
        ("outside_viewport", GuardReason::OutsideViewport),
        ("no_hit_target", GuardReason::NoHitTarget),
        ("obscured", GuardReason::Obscured),
        ("detached", GuardReason::Detached),
        ("document_changed", GuardReason::DocumentChanged),
    ];
    for (case, reason) in expected {
        let result = &results[case];
        assert_eq!(result["ok"], false, "{case}: {result}");
        assert_eq!(result["reason"], reason.code(), "{case}: {result}");
        assert_ne!(result["reason"], "blocked", "{case}");
        // JS advisory class must agree with the canonical Rust class.
        assert_eq!(
            result["retry_class"],
            reason.retry_class().code(),
            "{case}: {result}"
        );
    }
    // Every code the plan names is covered by at least one case above.
    assert_eq!(expected.len(), 14);
}

#[test]
#[ignore]
fn guard_detail_never_leaks_page_text_or_protected_blockers() {
    let results = run_cases();
    let rendered = results.to_string();
    assert!(!rendered.contains("SECRET-OVERLAY-TEXT"), "{rendered}");
    assert!(!rendered.contains("MASKED-SENTINEL"), "{rendered}");
    let obscured = &results["obscured"];
    assert_eq!(obscured["detail"]["blocker"]["tag"], "div");
    assert_eq!(obscured["detail"]["blocker"]["position"], "fixed");
    assert!(
        obscured["detail"]["blocker"]["covers_percent"]
            .as_u64()
            .unwrap()
            >= 90
    );
    let protected = &results["obscured_protected"];
    assert_eq!(protected["reason"], "obscured");
    assert_eq!(protected["detail"]["blocker"]["protected"], true);
    assert!(protected["detail"]["blocker"].get("tag").is_none());
}

#[test]
#[ignore]
fn only_viewport_and_obscured_are_internally_repairable() {
    let results = run_cases();
    for (case, value) in results.as_object().unwrap() {
        if value["ok"] == true {
            continue;
        }
        let internal = value["retry_class"] == RetryClass::SafeInternal.code();
        assert_eq!(
            internal,
            matches!(
                case.as_str(),
                "outside_viewport" | "obscured" | "obscured_protected"
            ),
            "{case}: {value}"
        );
    }
}

#[test]
#[ignore]
fn snapshot_completeness_covers_text_items_and_unread_frames() {
    let cases = &run_cases()["snapshot_completeness"];
    assert_eq!(cases["full"]["complete"], true, "{}", cases["full"]);
    assert_eq!(cases["full"]["completeness"]["controls"], "complete");
    assert_eq!(cases["text"]["text_truncated"], true, "{}", cases["text"]);
    assert_eq!(cases["text"]["complete"], false, "{}", cases["text"]);
    assert_eq!(cases["items"]["omitted_by_limit"].as_u64(), Some(1));
    assert_eq!(cases["items"]["complete"], false, "{}", cases["items"]);
    assert!(
        cases["frame"]["cross_origin_frames_unread"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "{}",
        cases["frame"]
    );
    assert_eq!(cases["frame"]["completeness"]["frames"], "partial");
    assert_eq!(cases["frame"]["complete"], false, "{}", cases["frame"]);
    assert_eq!(cases["scoped"]["complete"], false, "{}", cases["scoped"]);
    assert_eq!(
        cases["scoped"]["scope_complete"], true,
        "{}",
        cases["scoped"]
    );
}

#[test]
#[ignore]
fn snapshot_separates_rendered_from_viewport_presentation() {
    let item = &run_cases()["presentation"]["items"][0];
    assert_eq!(item["present"], true, "{item}");
    assert_eq!(item["rendered"], true, "{item}");
    assert_eq!(item["in_viewport"], false, "{item}");
    assert_eq!(item["actionability"], "offscreen", "{item}");
    assert_eq!(item["visible"], serde_json::Value::Null, "{item}");
}

#[test]
#[ignore]
fn snapshot_only_computes_presentation_for_controls() {
    let reads = run_cases()["presentation_style_reads"].as_u64().unwrap();
    assert!(
        reads <= 2,
        "presentation style reads should scale with candidates, got {reads}"
    );
}

#[test]
#[ignore]
fn snapshot_discovers_editors_but_excludes_protected_content() {
    let results = run_cases();
    let editors = results["editors"]["items"].as_array().unwrap();
    let announcement = editors
        .iter()
        .find(|item| item["name"] == "Announcement")
        .unwrap();
    assert_eq!(announcement["role"], "textbox");
    assert_eq!(announcement["editable_kind"], "rich_text");
    assert_eq!(announcement["raw_value"], "Hello");
    assert_eq!(announcement["multiline"], true);
    assert!(editors.iter().any(|item| item["name"] == "Second"));
    assert_eq!(
        results["empty_editor"]["items"][0]["raw_value"], "",
        "a browser-rendered line break in an empty rich editor must match the action guard's empty value"
    );
    let custom = editors
        .iter()
        .find(|item| item["name"] == "Custom textbox")
        .unwrap();
    assert_eq!(custom["editable_kind"], "rich_text");
    assert_eq!(custom["multiline"], true);
    let native = editors
        .iter()
        .find(|item| item["name"] == "Native textbox")
        .unwrap();
    assert_eq!(native["editable_kind"], "plain_text");
    assert_eq!(native["raw_value"], "Plain value");
    assert_eq!(
        editors
            .iter()
            .find(|item| item["name"] == "Readonly ARIA")
            .unwrap()["editable_kind"],
        serde_json::Value::Null
    );
    assert_eq!(
        editors
            .iter()
            .find(|item| item["name"] == "Readonly role textbox")
            .unwrap()["editable_kind"],
        serde_json::Value::Null
    );
    assert_eq!(
        editors
            .iter()
            .find(|item| item["name"] == "Checkbox")
            .unwrap()["editable_kind"],
        serde_json::Value::Null
    );
    for excluded in ["Read only", "Masked", "File"] {
        assert!(
            !editors.iter().any(|item| item["name"] == excluded),
            "{excluded}: {editors:?}"
        );
    }
    assert!(
        results["shadow_editor"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "Shadow editor")
    );
    assert!(!results.to_string().contains("SECRET"));
}

#[test]
fn snapshot_surfaces_password_target_without_reading_its_value() {
    let results = run_cases();
    let editors = &results["editors"]["items"];
    let password = editors
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["credential_input"] == true)
        .expect("visible password field has a snapshot reference");
    assert_eq!(password["name"], "Password");
    assert_eq!(password["raw_value"], serde_json::Value::Null);
    assert_eq!(password["editable_kind"], serde_json::Value::Null);
    assert_eq!(password["requires_user_input"], true);
    assert!(!editors.to_string().contains("PASSWORD-SENTINEL"));
}
