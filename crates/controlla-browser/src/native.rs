//! Read-only observation of selected macOS desktop state.

#[derive(Clone, Debug, PartialEq)]
pub struct NativeSnapshot {
    pub frontmost_bundle_id: Option<String>,
    pub cursor_x: f64,
    pub cursor_y: f64,
    pub pasteboard_change_count: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("native snapshot is unsupported: {0}")]
pub struct NativeSnapshotUnsupported(pub String);

impl NativeSnapshot {
    pub fn capture() -> Result<Self, NativeSnapshotUnsupported> {
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("swift")
                .args(["-e", SNAPSHOT_SCRIPT])
                .output()
                .map_err(|error| {
                    NativeSnapshotUnsupported(format!("Swift unavailable: {error}"))
                })?;
            if !output.status.success() {
                return Err(NativeSnapshotUnsupported(
                    String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                ));
            }
            let value: serde_json::Value =
                serde_json::from_slice(&output.stdout).map_err(|error| {
                    NativeSnapshotUnsupported(format!("invalid snapshot JSON: {error}"))
                })?;
            let cursor_x = value["cursorX"].as_f64().ok_or_else(|| {
                NativeSnapshotUnsupported("Quartz returned no cursor location".into())
            })?;
            let cursor_y = value["cursorY"].as_f64().ok_or_else(|| {
                NativeSnapshotUnsupported("Quartz returned no cursor location".into())
            })?;
            let pasteboard_change_count =
                value["pasteboardChangeCount"].as_i64().ok_or_else(|| {
                    NativeSnapshotUnsupported("AppKit returned no pasteboard change count".into())
                })?;
            let frontmost_bundle_id = value["frontmostBundleId"].as_str().map(str::to_owned);
            Ok(Self {
                frontmost_bundle_id,
                cursor_x,
                cursor_y,
                pasteboard_change_count,
            })
        }
        #[cfg(not(target_os = "macos"))]
        Err(NativeSnapshotUnsupported(
            "read-only native snapshot currently requires macOS".into(),
        ))
    }
}

#[cfg(target_os = "macos")]
const SNAPSHOT_SCRIPT: &str = r#"
import AppKit
import Foundation
import Quartz

guard let point = CGEvent(source: nil)?.location else {
    fputs("Quartz could not read cursor location", stderr)
    exit(2)
}
var snapshot: [String: Any] = [
    "cursorX": point.x,
    "cursorY": point.y,
    "pasteboardChangeCount": NSPasteboard.general.changeCount,
]
if let bundleId = NSWorkspace.shared.frontmostApplication?.bundleIdentifier {
    snapshot["frontmostBundleId"] = bundleId
} else {
    snapshot["frontmostBundleId"] = NSNull()
}
let data = try JSONSerialization.data(withJSONObject: snapshot)
FileHandle.standardOutput.write(data)
"#;

#[cfg(target_os = "macos")]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_snapshot_is_available() {
        let snapshot = NativeSnapshot::capture().expect("macOS snapshot should be available");
        assert!(snapshot.cursor_x.is_finite());
        assert!(snapshot.cursor_y.is_finite());
    }
}
