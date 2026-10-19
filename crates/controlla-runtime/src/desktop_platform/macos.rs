//! macOS foreground Chrome bounded viewport input fallback.
//!
//! The experimental adapter uses Apple Events addressed to Chrome, never
//! global keyboard events. It is not advertised or dispatched until its helper
//! has a stable signed app identity for macOS permission checks.

use crate::desktop_fallback::{
    DesktopAction, DesktopActionEnvelope, DesktopActionReceipt, DesktopActionStatus,
    DesktopCapability, DesktopMode, uncertain_delivery_receipt,
};
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

pub const SUPPORT_STATUS: &str = "disabled_until_stable_signed_helper_identity";
pub const REQUIRED_PERMISSIONS: &[&str] =
    &["stable signed helper app identity before native input permissions are supported"];
const HELPER_TIMEOUT: Duration = Duration::from_secs(3);

const SWIFT_HELPER: &str = r#"
import AppKit
import ApplicationServices
import CoreGraphics
import Darwin
import Foundation
import ScriptingBridge

func emit(_ value: [String: Any]) {
  let data = (try? JSONSerialization.data(withJSONObject: value)) ?? Data("{}".utf8)
  FileHandle.standardOutput.write(data)
}
let data = FileHandle.standardInput.readDataToEndOfFile()
guard let input = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
  emit(["status":"refused", "reason":"invalid_input"]); exit(0)
}
let frontmost = NSWorkspace.shared.frontmostApplication
let bundle = frontmost?.bundleIdentifier ?? ""
let chromeBundles: Set<String> = ["com.google.Chrome", "com.google.Chrome.beta", "com.google.Chrome.dev", "com.google.Chrome.canary", "org.chromium.Chromium"]
let chromeFocused = chromeBundles.contains(bundle)
let expectedProcessId = (input["expected_process_id"] as? NSNumber)?.int32Value ?? -1
let processMatches = expectedProcessId > 0 && frontmost?.processIdentifier == expectedProcessId
let automationAuthorized: Bool = {
  guard expectedProcessId > 0 else { return false }
  let target = NSAppleEventDescriptor(processIdentifier: expectedProcessId)
  guard let descriptor = target.aeDesc else { return false }
  return AEDeterminePermissionToAutomateTarget(descriptor, typeWildCard, typeWildCard, false) == noErr
}()
let accessibilityAuthorized = AXIsProcessTrusted()
let eventPostingAuthorized = CGPreflightPostEventAccess()
let helperBundleId = Bundle.main.bundleIdentifier ?? ""
let helperIdentitySupported = !helperBundleId.isEmpty
let permissionIdentity: [String: Any] = [
  "process_id": Int(getpid()),
  "process_name": ProcessInfo.processInfo.processName,
  "executable_path": CommandLine.arguments.first ?? "",
  "bundle_id": helperBundleId
]
if input["operation"] as? String == "probe" {
  emit(["status": chromeFocused && processMatches && automationAuthorized && helperIdentitySupported ? "ready" : "unavailable",
        "chrome_focused":chromeFocused,
        "process_matches":processMatches, "automation_authorized":automationAuthorized,
        "accessibility_authorized":accessibilityAuthorized,
        "event_posting_authorized":eventPostingAuthorized,
        "helper_identity_supported":helperIdentitySupported,
        "permission_identity":permissionIdentity,
        "process_id":frontmost?.processIdentifier ?? 0])
  exit(0)
}
guard chromeFocused else {
  emit(["status":"refused", "reason":"chrome_not_frontmost"])
  exit(0)
}
guard processMatches else {
  emit(["status":"refused", "reason":"owned_chrome_process_not_frontmost"]); exit(0)
}
guard automationAuthorized else {
  emit(["status":"refused", "reason":"automation_permission_required"]); exit(0)
}
if input["operation"] as? String == "input" {
  guard helperIdentitySupported else {
    emit(["status":"refused", "reason":"native_helper_identity_unstable"]); exit(0)
  }
  guard accessibilityAuthorized else {
    emit(["status":"refused", "reason":"accessibility_permission_required"]); exit(0)
  }
  guard eventPostingAuthorized else {
    emit(["status":"refused", "reason":"input_monitoring_permission_required"]); exit(0)
  }
}
guard let expected = input["expected_url"] as? String,
      let expectedTabId = input["expected_tab_id"] as? String,
      expected.utf8.count <= 8192,
      !expectedTabId.isEmpty, expectedTabId.utf8.count <= 32,
      !expected.contains("\n"), !expected.contains("\r") else {
  emit(["status":"refused", "reason":"invalid_native_target"]); exit(0)
}
func code(_ value: String) -> UInt32 { value.utf8.reduce(UInt32(0)) { ($0 << 8) | UInt32($1) } }
guard let app = SBApplication(processIdentifier: expectedProcessId), app.isRunning else {
  emit(["status":"refused", "reason":"owned_chrome_process_not_running"]); exit(0)
}
let windows = app.elementArray(withCode: code("cwin"))
guard windows.count == 1 else {
  emit(["status":"refused", "reason":"window_count_mismatch"]); exit(0)
}
guard let window = windows.object(at: 0) as? SBObject else {
  emit(["status":"refused", "reason":"window_unavailable"]); exit(0)
}
let tabs = window.elementArray(withCode: code("CrTb"))
guard tabs.count > 0, tabs.count <= 1000,
      let tab = window.property(withCode: code("acTa")).get() as? SBObject else {
  emit(["status":"refused", "reason":"tab_count_mismatch"]); exit(0)
}
let tabId = String(describing: tab.property(withCode: code("ID  ")).get() ?? "")
guard tabId == expectedTabId else {
  emit(["status":"refused", "reason":"owned_chrome_tab_id_mismatch"]); exit(0)
}
let sourceURL = String(describing: tab.property(withCode: code("URL ")).get() ?? "")
guard sourceURL == expected else {
  emit(["status":"refused", "reason":"source_url_mismatch"]); exit(0)
}
guard input["operation"] as? String == "input",
      let action = input["action"] as? String,
      let viewportWidth = input["viewport_width"] as? Double,
      let viewportHeight = input["viewport_height"] as? Double,
      viewportWidth > 0, viewportHeight > 0,
      viewportWidth <= 16384, viewportHeight <= 16384 else {
  emit(["status":"refused", "reason":"invalid_native_input"]); exit(0)
}

func axValue(_ element: AXUIElement, _ name: CFString) -> CFTypeRef? {
  var value: CFTypeRef?
  guard AXUIElementCopyAttributeValue(element, name, &value) == .success else { return nil }
  return value
}
func webAreaRect(_ element: AXUIElement, depth: Int, visited: inout Int) -> CGRect? {
  guard depth <= 20, visited < 10000 else { return nil }
  visited += 1
  if (axValue(element, kAXRoleAttribute as CFString) as? String) == "AXWebArea" {
    guard let posValue = axValue(element, kAXPositionAttribute as CFString),
          let sizeValue = axValue(element, kAXSizeAttribute as CFString) else { return nil }
    var origin = CGPoint.zero
    var size = CGSize.zero
    guard AXValueGetValue(posValue as! AXValue, .cgPoint, &origin), AXValueGetValue(sizeValue as! AXValue, .cgSize, &size),
          size.width > 0, size.height > 0 else { return nil }
    return CGRect(origin: origin, size: size)
  }
  guard let children = axValue(element, kAXChildrenAttribute as CFString) as? [AXUIElement] else { return nil }
  for child in children.prefix(1000) {
    if let rect = webAreaRect(child, depth: depth + 1, visited: &visited) { return rect }
    if visited >= 10000 { break }
  }
  return nil
}
let axApp = AXUIElementCreateApplication(expectedProcessId)
guard let axWindows = axValue(axApp, kAXWindowsAttribute as CFString) as? [AXUIElement],
      axWindows.count == 1 else {
  emit(["status":"refused", "reason":"window_count_mismatch"]); exit(0)
}
var visited = 0
guard let webRect = webAreaRect(axWindows[0], depth: 0, visited: &visited) else {
  emit(["status":"refused", "reason":"browser_viewport_bounds_unavailable"]); exit(0)
}
let scaleX = webRect.width / viewportWidth
let scaleY = webRect.height / viewportHeight
guard scaleX.isFinite, scaleY.isFinite, scaleX > 0.2, scaleX < 5,
      scaleY > 0.2, scaleY < 5,
      abs(scaleX - scaleY) / max(scaleX, scaleY) <= 0.12 else {
  emit(["status":"refused", "reason":"browser_viewport_geometry_mismatch"]); exit(0)
}
func screenPoint(_ x: Double, _ y: Double) -> CGPoint {
  CGPoint(x: webRect.minX + x * scaleX, y: webRect.minY + y * scaleY)
}
func postMouse(_ type: CGEventType, _ point: CGPoint, _ button: CGMouseButton = .left) -> Bool {
  guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: button) else { return false }
  event.postToPid(expectedProcessId)
  return true
}
func postKey(_ keyCode: CGKeyCode, _ flags: CGEventFlags = []) -> Bool {
  guard let down = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: true),
        let up = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: false) else { return false }
  down.flags = flags; up.flags = flags
  down.postToPid(expectedProcessId); up.postToPid(expectedProcessId)
  return true
}
func pointFromAction(_ xName: String, _ yName: String) -> CGPoint? {
  guard let x = input[xName] as? Double, let y = input[yName] as? Double,
        x >= 0, y >= 0, x < viewportWidth, y < viewportHeight else { return nil }
  return screenPoint(x, y)
}
var dispatched = false
switch action {
case "move":
  if let point = pointFromAction("x", "y") { dispatched = postMouse(.mouseMoved, point) }
case "scroll":
  if let point = pointFromAction("x", "y"),
     let dx = input["delta_x"] as? Int32, let dy = input["delta_y"] as? Int32,
     let event = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2, wheel1: dy, wheel2: dx, wheel3: 0) {
    event.location = point
    event.postToPid(expectedProcessId)
    dispatched = true
  }
case "key":
  if let key = input["key"] as? String {
    let codes: [String: CGKeyCode] = ["Escape":53, "Tab":48, "Control+L":37, "Meta+L":37,
      "ArrowUp":126, "ArrowDown":125, "ArrowLeft":123, "ArrowRight":124]
    if key == "Shift+Tab" { dispatched = postKey(48, .maskShift) }
    else if key == "Control+L" { dispatched = postKey(37, .maskControl) }
    else if key == "Meta+L" { dispatched = postKey(37, .maskCommand) }
    else if let code = codes[key] { dispatched = postKey(code) }
  }
default: break
}
guard dispatched else {
  emit(["status":"refused", "reason":"native_input_not_dispatched"]); exit(0)
}
emit(["status":"submitted", "process_id":expectedProcessId, "tab_id":tabId, "action":action])
"#;

#[derive(Clone, Copy, Debug, Default)]
pub struct MacOsDesktopProvider;

impl MacOsDesktopProvider {
    pub fn capability_available(&self, _capability: DesktopCapability) -> bool {
        false
    }

    pub async fn capability_report(&self) -> Value {
        let expected_process_id = crate::native_setup::current_host_id()
            .ok()
            .and_then(|host_id| crate::native_setup::browser_process_id_for_host(&host_id).ok());
        let probe = match expected_process_id {
            Some(pid) => run(json!({"operation":"probe","expected_process_id":pid}))
                .await
                .ok(),
            None => None,
        };
        let ready = probe
            .as_ref()
            .is_some_and(|probe| probe["status"] == "ready");
        let pointer_ready = ready
            && probe.as_ref().is_some_and(|probe| {
                probe["accessibility_authorized"] == true
                    && probe["event_posting_authorized"] == true
            });
        let reason = if expected_process_id.is_none() {
            "pair the current Chrome instance through its native host"
        } else if !probe
            .as_ref()
            .is_some_and(|probe| probe["chrome_focused"] == true)
        {
            "focus the provider-owned Chrome window"
        } else if !probe
            .as_ref()
            .is_some_and(|probe| probe["process_matches"] == true)
        {
            "the focused Chrome process does not match the paired native host"
        } else if !probe
            .as_ref()
            .is_some_and(|probe| probe["helper_identity_supported"] == true)
        {
            "native fallback is disabled because the Swift helper has no stable signed app identity; use the Chrome extension route"
        } else if !probe
            .as_ref()
            .is_some_and(|probe| probe["automation_authorized"] == true)
        {
            "grant Automation permission to the helper process in helper_permission_identity"
        } else if !probe
            .as_ref()
            .is_some_and(|probe| probe["accessibility_authorized"] == true)
        {
            "grant Accessibility permission to the helper process in helper_permission_identity"
        } else if !probe
            .as_ref()
            .is_some_and(|probe| probe["event_posting_authorized"] == true)
        {
            "grant Input Monitoring permission to the helper process in helper_permission_identity"
        } else {
            "exact-process foreground viewport input is ready"
        };
        json!({
            "platform":"macos",
            "status":if ready {"partial"} else {"unavailable"},
            "capabilities":{
                "desktop_pointer":if pointer_ready {"available"} else {"capability_unavailable"},
                "desktop_keyboard":if pointer_ready {"available"} else {"capability_unavailable"},
                "desktop_window":"capability_unavailable"
            },
            "permission_gates":{
                "automation":probe.as_ref().map(|probe| probe["automation_authorized"].clone()).unwrap_or(Value::Null),
                "accessibility":probe.as_ref().map(|probe| probe["accessibility_authorized"].clone()).unwrap_or(Value::Null),
                "input_monitoring":probe.as_ref().map(|probe| probe["event_posting_authorized"].clone()).unwrap_or(Value::Null)
            },
            "helper_identity_supported":probe.as_ref().is_some_and(|probe| probe["helper_identity_supported"] == true),
            "helper_permission_identity":probe.as_ref().map(|probe| probe["permission_identity"].clone()).unwrap_or(Value::Null),
            "target_process_id":expected_process_id,
            "process_matches_frontmost":probe.as_ref().map(|probe| probe["process_matches"].clone()).unwrap_or(Value::Null),
            "chrome_frontmost":probe.as_ref().map(|probe| probe["chrome_focused"].clone()).unwrap_or(Value::Null),
            "constraints":[
                "foreground Chrome only",
                "exact paired Chrome process and extension tab ID",
                "exactly one Chrome window; exact selected tab must match the paired extension tab ID",
                "source URL must match fresh visual evidence",
                "pointer movement, scrolling, and non-submitting keys target only the exact paired Chrome process",
                "coordinate clicks, drags, and page-field typing are refused; use semantic extension actions",
                "native input is mapped only into the accessibility-reported browser viewport",
                "helper deadline is three seconds; uncertain delivery is never retried"
            ],
            "reason":reason,
            "probe_error":probe.is_none().then_some("no current paired native-host process or helper probe failed")
        })
    }

    pub async fn dispatch(&self, envelope: &DesktopActionEnvelope) -> DesktopActionReceipt {
        dispatch(envelope).await
    }
}

async fn run(input: Value) -> Result<Value, RunFailure> {
    let mut child = Command::new("swift")
        .args(["-e", SWIFT_HELPER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| RunFailure::NotStarted("swift_runtime_unavailable"))?;
    let payload =
        serde_json::to_vec(&input).map_err(|_| RunFailure::NotStarted("invalid_native_request"))?;
    let operation = async {
        let mut stdin = child
            .stdin
            .take()
            .ok_or(RunFailure::Uncertain("native_stdin_unavailable"))?;
        stdin
            .write_all(&payload)
            .await
            .map_err(|_| RunFailure::Uncertain("native_request_write_uncertain"))?;
        drop(stdin);
        let output = child
            .wait_with_output()
            .await
            .map_err(|_| RunFailure::Uncertain("native_helper_failed"))?;
        if !output.status.success() {
            return Err(RunFailure::Uncertain("native_helper_failed"));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|_| RunFailure::Uncertain("native_helper_response_invalid"))
    };
    match timeout(HELPER_TIMEOUT, operation).await {
        Ok(result) => result,
        Err(_) => Err(RunFailure::Uncertain("native_helper_timeout")),
    }
}

enum RunFailure {
    NotStarted(&'static str),
    Uncertain(&'static str),
}

async fn dispatch(envelope: &DesktopActionEnvelope) -> DesktopActionReceipt {
    let capability = envelope.capability;
    if crate::desktop_fallback::validate_desktop_envelope(
        envelope,
        true,
        &envelope.visual_identity,
        envelope.viewport_width,
        envelope.viewport_height,
    )
    .is_err()
    {
        return not_dispatched(capability, "invalid_native_action_bounds_or_identity");
    }
    if envelope.mode != DesktopMode::Foreground
        || envelope.expected_url.is_empty()
        || envelope.expected_url.len() > 8192
        || envelope.expected_process_id == 0
    {
        return not_dispatched(capability, "unsupported_or_forbidden_native_action");
    }
    let operation = match &envelope.action {
        DesktopAction::Move { .. } | DesktopAction::Scroll { .. }
            if capability == DesktopCapability::DesktopPointer =>
        {
            "input"
        }
        DesktopAction::Key { .. } if capability == DesktopCapability::DesktopKeyboard => "input",
        _ => return not_dispatched(capability, "unsupported_or_forbidden_native_action"),
    };
    let mut request = json!({
        "operation":operation,
        "expected_url":envelope.expected_url,
        "expected_process_id":envelope.expected_process_id,
        "expected_tab_id":envelope.visual_identity.tab_id,
        "viewport_width":envelope.viewport_width,
        "viewport_height":envelope.viewport_height,
    });
    if let Ok(action_value) = serde_json::to_value(&envelope.action)
        && let (Some(request), Some(action)) = (request.as_object_mut(), action_value.as_object())
    {
        if let Some(kind) = action.get("kind") {
            request.insert("action".into(), kind.clone());
        }
        for (key, value) in action {
            if key != "kind" {
                request.insert(key.clone(), value.clone());
            }
        }
    }
    match run(request).await {
        Ok(result) if result["status"] == "submitted" => {
            let mut receipt = uncertain_delivery_receipt(capability);
            receipt.reason_code = Some("native_input_submitted_reconciliation_required".into());
            receipt.next_action =
                Some("verify the declared page outcome once; never replay native input".into());
            receipt
        }
        Ok(result) if result["status"] == "unknown" => uncertain(
            result["reason"]
                .as_str()
                .unwrap_or("native_delivery_unknown"),
            capability,
        ),
        Ok(result) if result["status"] == "refused" => not_dispatched(
            capability,
            result["reason"].as_str().unwrap_or("native_action_refused"),
        ),
        Ok(_) => uncertain("native_helper_response_invalid", capability),
        Err(RunFailure::NotStarted(reason)) => not_dispatched(capability, reason),
        Err(RunFailure::Uncertain(reason)) => uncertain(reason, capability),
    }
}

fn not_dispatched(capability: DesktopCapability, reason: &str) -> DesktopActionReceipt {
    DesktopActionReceipt {
        status: DesktopActionStatus::NotDispatched,
        capability,
        dispatch_acknowledged: false,
        dispatch_may_have_occurred: false,
        reason_code: Some(reason.chars().take(96).collect()),
        retry_class: Some("permission_or_target_mismatch".into()),
        next_action: Some("confirm Chrome Automation permission, owned foreground session, and fresh matching visual evidence".into()),
    }
}

fn uncertain(reason: &str, capability: DesktopCapability) -> DesktopActionReceipt {
    let mut receipt = uncertain_delivery_receipt(capability);
    receipt.reason_code = Some(reason.chars().take(96).collect());
    receipt.next_action = Some("inspect the declared page outcome once; native delivery may have occurred and must never be replayed".into());
    receipt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_fallback::DesktopVisualIdentity;

    fn envelope(action: DesktopAction) -> DesktopActionEnvelope {
        DesktopActionEnvelope {
            version: 1,
            request_id: "test".into(),
            lease_id: "test".into(),
            capability: DesktopCapability::DesktopKeyboard,
            mode: DesktopMode::Foreground,
            visual_identity: DesktopVisualIdentity {
                session_id: "s".into(),
                tab_id: "t".into(),
                document_id: "d".into(),
                visual_revision: "v".into(),
            },
            viewport_width: 800,
            viewport_height: 600,
            expected_url: "https://example.com/".into(),
            expected_process_id: 1234,
            action,
        }
    }

    #[test]
    fn helper_is_scoped_to_chrome_and_rejects_ambiguous_windows() {
        assert!(SWIFT_HELPER.contains("elementArray(withCode: code(\"cwin\"))"));
        assert!(SWIFT_HELPER.contains("owned_chrome_tab_id_mismatch"));
        assert!(SWIFT_HELPER.contains("source_url_mismatch"));
        assert!(SWIFT_HELPER.contains("SBApplication(processIdentifier: expectedProcessId)"));
        assert!(!SWIFT_HELPER.contains("globalSession"));
        assert!(SWIFT_HELPER.contains("CGPreflightPostEventAccess"));
        assert!(SWIFT_HELPER.contains("postToPid(expectedProcessId)"));
        assert!(SWIFT_HELPER.contains("AXIsProcessTrusted"));
        assert!(SWIFT_HELPER.contains("native_helper_identity_unstable"));
        assert!(SWIFT_HELPER.contains("browser_viewport_bounds_unavailable"));
        assert!(!SWIFT_HELPER.contains("case \"click\""));
        assert!(!SWIFT_HELPER.contains("case \"drag\""));
        assert!(!SWIFT_HELPER.contains("case \"type\""));
    }

    #[test]
    fn macos_provider_fails_closed_without_a_stable_signed_helper_identity() {
        let provider = MacOsDesktopProvider;
        assert!(!provider.capability_available(DesktopCapability::DesktopPointer));
        assert!(!provider.capability_available(DesktopCapability::DesktopKeyboard));
        assert!(!provider.capability_available(DesktopCapability::DesktopWindow));
    }

    #[test]
    fn swift_helper_compiles_and_probe_is_read_only() {
        let result = std::process::Command::new("swift")
            .args(["-e", SWIFT_HELPER])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let Ok(mut child) = result else { return };
        use std::io::Write as _;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(br#"{"operation":"probe"}"#)
            .unwrap();
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "Swift helper failed to compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let response: Value =
            serde_json::from_slice(&output.stdout).expect("structured probe response");
        assert!(response["status"] == "ready" || response["status"] == "unavailable");
        assert!(response["chrome_focused"].is_boolean());
        assert!(
            response["permission_identity"]["process_id"]
                .as_i64()
                .is_some()
        );
        assert!(
            response["permission_identity"]["process_name"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(
            response["permission_identity"]["executable_path"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert_eq!(response["helper_identity_supported"], false);
    }

    #[tokio::test]
    async fn unsupported_native_targets_are_not_dispatched() {
        let provider = MacOsDesktopProvider;
        let receipt = provider
            .dispatch(&envelope(DesktopAction::Key {
                key: "Enter".into(),
            }))
            .await;
        assert_eq!(receipt.status, DesktopActionStatus::NotDispatched);
    }
}
