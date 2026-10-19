//! Linux adapter remains unavailable until its display server, input access,
//! and owned-window policy are verified for X11 and Wayland separately.
pub const SUPPORT_STATUS: &str = "capability_unavailable";
pub const REQUIRED_PERMISSIONS: &[&str] = &["supported X11 or Wayland input session"];
