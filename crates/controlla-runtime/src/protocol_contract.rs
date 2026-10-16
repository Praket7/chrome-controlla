//! Single Rust-owned description of the extension command contract.

pub const CONTRACT_VERSION: u16 = 3;
pub const MAX_NATIVE_MESSAGE_BYTES: usize = 900 * 1024;
pub const AUTHORIZED_METHODS: &[&str] = &[
    "Page.getFrameTree",
    "Page.getLayoutMetrics",
    "Page.captureScreenshot",
    "Runtime.evaluate",
    "Runtime.callFunctionOn",
    "Runtime.releaseObject",
    "Input.dispatchMouseEvent",
    "Input.dispatchDragEvent",
    "Input.dispatchTouchEvent",
    "Input.dispatchKeyEvent",
    "Input.imeSetComposition",
    "Input.insertText",
    "DOM.getDocument",
    "DOM.querySelector",
    "DOM.enable",
    "DOM.describeNode",
    "DOM.resolveNode",
    "DOM.setFileInputFiles",
    "Page.enable",
    "Page.setInterceptFileChooserDialog",
    "Accessibility.getPartialAXTree",
    "Accessibility.getFullAXTree",
    "Controlla.waitForFileChooser",
    "Controlla.downloadLifecycle",
    "Controlla.scrollToTarget",
];

pub fn method_is_authorized(method: &str) -> bool {
    AUTHORIZED_METHODS.contains(&method)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract_has_one_bounded_allowlist() {
        assert_eq!(CONTRACT_VERSION, 3);
        assert!(AUTHORIZED_METHODS.len() > 10);
        assert!(!method_is_authorized("Network.getAllCookies"));
        const { assert!(MAX_NATIVE_MESSAGE_BYTES < 1024 * 1024) };
    }
}
