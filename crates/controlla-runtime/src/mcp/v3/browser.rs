//! Browser-level V3 request classification. This module deliberately contains
//! no dispatch code; it keeps the public action vocabulary separate from the
//! extension transport.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserRequestKind {
    Open,
    Task,
    Snapshot,
    Act,
    Workflow,
    Verify,
    Extract,
    Tabs,
    Attach,
    Release,
    Diagnostics,
}

pub fn classify_action(action: &str) -> Option<BrowserRequestKind> {
    Some(match action {
        "open" => BrowserRequestKind::Open,
        "task" => BrowserRequestKind::Task,
        "snapshot" => BrowserRequestKind::Snapshot,
        "act" => BrowserRequestKind::Act,
        "workflow" => BrowserRequestKind::Workflow,
        "verify" => BrowserRequestKind::Verify,
        "extract" => BrowserRequestKind::Extract,
        "tabs" => BrowserRequestKind::Tabs,
        "attach" | "adopt" => BrowserRequestKind::Attach,
        "release" | "close" => BrowserRequestKind::Release,
        "describe" | "diagnostics" | "capabilities" => BrowserRequestKind::Diagnostics,
        _ => return None,
    })
}

pub fn action_is_read_only(kind: BrowserRequestKind) -> bool {
    matches!(
        kind,
        BrowserRequestKind::Snapshot
            | BrowserRequestKind::Verify
            | BrowserRequestKind::Extract
            | BrowserRequestKind::Diagnostics
    )
}

pub fn context_reuse_hint(has_context: bool) -> &'static str {
    if has_context {
        "Reuse the returned context for the next step."
    } else {
        "Capture the returned context; it is opaque and avoids re-pairing."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_tools_and_compatibility_actions_classify() {
        assert_eq!(classify_action("task"), Some(BrowserRequestKind::Task));
        assert_eq!(classify_action("snapshot"), Some(BrowserRequestKind::Snapshot));
        assert_eq!(classify_action("act"), Some(BrowserRequestKind::Act));
        assert_eq!(classify_action("workflow"), Some(BrowserRequestKind::Workflow));
        assert_eq!(classify_action("extract"), Some(BrowserRequestKind::Extract));
        assert_eq!(classify_action("verify"), Some(BrowserRequestKind::Verify));
        assert_eq!(classify_action("unknown"), None);
        assert!(action_is_read_only(BrowserRequestKind::Snapshot));
        assert!(!action_is_read_only(BrowserRequestKind::Act));
    }
}
