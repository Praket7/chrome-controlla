//! Host platform capability adapter. Each OS advertises only capabilities
//! implemented by its provider and currently available under its permissions.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

use crate::desktop_fallback::{
    DesktopCapability, DesktopFallbackProvider, UnavailableDesktopProvider,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopPlatform {
    MacOs,
    Windows,
    Linux,
    Unsupported,
}

pub fn current_platform() -> DesktopPlatform {
    #[cfg(target_os = "macos")]
    {
        return DesktopPlatform::MacOs;
    }
    #[cfg(target_os = "windows")]
    {
        return DesktopPlatform::Windows;
    }
    #[cfg(target_os = "linux")]
    {
        return DesktopPlatform::Linux;
    }
    #[allow(unreachable_code)]
    DesktopPlatform::Unsupported
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlatformDesktopProvider {
    unavailable: UnavailableDesktopProvider,
    #[cfg(target_os = "macos")]
    macos: macos::MacOsDesktopProvider,
}

impl DesktopFallbackProvider for PlatformDesktopProvider {
    fn capability_available(&self, capability: DesktopCapability) -> bool {
        #[cfg(target_os = "macos")]
        {
            return self.macos.capability_available(capability);
        }
        #[allow(unreachable_code)]
        self.unavailable.capability_available(capability)
    }
    fn dispatch<'a>(
        &'a self,
        envelope: &'a crate::desktop_fallback::DesktopActionEnvelope,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = crate::desktop_fallback::DesktopActionReceipt>
                + Send
                + 'a,
        >,
    > {
        #[cfg(target_os = "macos")]
        {
            return Box::pin(self.macos.dispatch(envelope));
        }
        #[allow(unreachable_code)]
        self.unavailable.dispatch(envelope)
    }
}

#[cfg(target_os = "macos")]
pub async fn capability_report() -> serde_json::Value {
    macos::MacOsDesktopProvider.capability_report().await
}

#[cfg(not(target_os = "macos"))]
pub async fn capability_report() -> serde_json::Value {
    serde_json::json!({
        "platform":format!("{:?}", current_platform()).to_ascii_lowercase(),
        "status":"unavailable",
        "capabilities":{
            "desktop_pointer":"capability_unavailable",
            "desktop_keyboard":"capability_unavailable",
            "desktop_window":"capability_unavailable"
        },
        "reason":"no platform adapter has passed permission, ownership, focus, and dispatch tests on this host"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn platform_adapter_advertises_implemented_capabilities_only() {
        let provider = PlatformDesktopProvider::default();
        #[cfg(target_os = "macos")]
        {
            assert!(!provider.capability_available(DesktopCapability::DesktopPointer));
            assert!(!provider.capability_available(DesktopCapability::DesktopKeyboard));
            assert!(!provider.capability_available(DesktopCapability::DesktopWindow));
            // Runtime permission and target ownership are reported separately.
            let report = capability_report().await;
            assert_eq!(report["platform"], "macos");
            assert!(
                report["capabilities"]["desktop_pointer"] == "capability_available"
                    || report["capabilities"]["desktop_pointer"] == "capability_unavailable"
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            for capability in [
                DesktopCapability::DesktopPointer,
                DesktopCapability::DesktopKeyboard,
                DesktopCapability::DesktopWindow,
            ] {
                assert!(!provider.capability_available(capability));
            }
        }
        #[cfg(not(target_os = "macos"))]
        assert_eq!(capability_report().await["status"], "unavailable");
    }
}
