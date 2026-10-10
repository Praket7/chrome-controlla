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

/// Runs every case inside one headless page and returns `{case: guard_result}`.
fn run_cases() -> serde_json::Value {
    let guard = click_guard_script(controlla_runtime::mcp::SHARED_CLICK_UNSAFE_PREDICATE);
    let page = format!(
        r##"<!doctype html><html><head><meta charset="utf-8"><style>body{{margin:0}}</style></head><body></body>
<pre id="out"></pre>
<script>
try {{
const snapshotFn = ({SNAPSHOT_JS});
const guardFn = ({guard});
const cases = {{
  ok: {{html:'<button>Go</button>'}},
  unsafe_page: {{html:'<input type="checkbox" aria-label="Go">'}},
  stale_reference: {{html:'<button>Go</button>', index:99}},
  changed_target: {{html:'<button aria-label="Go">Go</button>', mutate:()=>document.querySelector('button').setAttribute('aria-label','Other')}},
  hidden: {{html:'<button>Go</button>', mutate:()=>document.querySelector('button').style.display='none'}},
  disabled: {{html:'<button>Go</button>', mutate:()=>document.querySelector('button').disabled=true}},
  aria_disabled: {{html:'<button>Go</button>', mutate:()=>document.querySelector('button').setAttribute('aria-disabled','true')}},
  masked_boundary: {{html:'<div><button>Go</button></div>', mutate:()=>document.querySelector('div').setAttribute('data-masked','')}},
  trusted_boundary: {{html:'<div><button>Go</button></div>', mutate:()=>document.querySelector('div').setAttribute('data-requires-trusted','')}},
  pointer_events_none: {{html:'<button>Go</button>', mutate:()=>document.querySelector('button').style.pointerEvents='none'}},
  outside_viewport: {{html:'<div style="height:4000px"></div><button>Go</button>', opts:{{scroll:'none'}}}},
  no_hit_target: {{html:'<button>Go</button>', mutate:()=>{{document.elementFromPoint=()=>null;}}}},
  obscured: {{html:'<button>Go</button>', mutate:()=>document.body.insertAdjacentHTML('beforeend','<div role="dialog" style="position:fixed;inset:0;z-index:9999;background:#0008">SECRET-OVERLAY-TEXT</div>')}},
  obscured_protected: {{html:'<button>Go</button>', mutate:()=>document.body.insertAdjacentHTML('beforeend','<div data-masked style="position:fixed;inset:0;z-index:9999">MASKED-SENTINEL</div>')}},
  detached: {{html:'<button>Go</button>', mutate:()=>document.querySelector('button').remove()}},
  document_changed: {{html:'<button>Go</button>', mutate:()=>{{const d=document.implementation.createHTMLDocument('x');d.body.appendChild(document.querySelector('button'));}}}},
}};
const results = {{}};
const realFromPoint = document.elementFromPoint.bind(document);
for (const [name, c] of Object.entries(cases)) {{
  document.elementFromPoint = realFromPoint;
  document.body.innerHTML = c.html;
  scrollTo(0, 0);
  const ctx = snapshotFn.call(null, {{selector:'body', token:'t', max_items:50, max_bytes:200000, max_text_chars:1000}});
  if (c.mutate) c.mutate();
  const index = c.index ?? 0;
  try {{ results[name] = guardFn.call(ctx, index, c.outcome || {{kind:'navigation'}}, c.opts); }}
  catch (error) {{ results[name] = {{threw: String(error)}}; }}
}}
document.getElementById('out').textContent = JSON.stringify(results);
}} catch (error) {{ document.getElementById('out').textContent = JSON.stringify({{page_error:String(error)}}); }}
</script></body></html>"##
    );
    let dir = std::env::temp_dir().join(format!("controlla-guard-diag-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("guard.html");
    std::fs::write(&file, page).unwrap();
    let output = Command::new(chrome())
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
        .output()
        .expect("launch Chrome");
    let dom = String::from_utf8_lossy(&output.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    let start = dom
        .find("<pre id=\"out\">")
        .unwrap_or_else(|| panic!("guard output element missing; stdout head: {}", &dom[..dom.len().min(800)]))
        + 14;
    let end = start + dom[start..].find("</pre>").expect("guard output end");
    let json = dom[start..end]
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    serde_json::from_str(&json).unwrap_or_else(|error| panic!("{error}: {json}"))
}

#[test]
fn every_guard_predicate_reports_its_own_reason_code() {
    let results = run_cases();
    assert_eq!(results["ok"]["ok"], true, "{}", results["ok"]);
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
