//! Windows UI Automation/SendInput adapter remains unavailable until its
//! focused-window ownership and dispatch receipts pass platform tests.
pub const SUPPORT_STATUS: &str = "capability_unavailable";
pub const REQUIRED_PERMISSIONS: &[&str] = &["interactive desktop session"];
