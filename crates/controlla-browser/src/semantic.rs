//! Browser-side semantic snapshot contracts. Runtime state assigns durable short references.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SemanticSnapshotSpec {
    pub selector: String,
    pub max_items: usize,
    pub max_text_chars: usize,
    pub max_bytes: usize,
}

impl Default for SemanticSnapshotSpec {
    fn default() -> Self {
        Self {
            selector: "body".into(),
            max_items: 120,
            max_text_chars: 4_000,
            max_bytes: 60_000,
        }
    }
}

impl SemanticSnapshotSpec {
    pub fn bounded(mut self) -> Self {
        self.max_items = self.max_items.clamp(1, 500);
        self.max_text_chars = self.max_text_chars.clamp(1, 10_000);
        self.max_bytes = self.max_bytes.clamp(4_096, 1_000_000);
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SemanticNode {
    pub retained_reference: String,
    pub role: String,
    pub name: String,
    pub raw_value: Option<String>,
    pub href: Option<String>,
    pub disabled: bool,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    pub frame_id: String,
    pub frame_revision: Option<String>,
    pub document_revision: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SemanticSnapshot {
    pub snapshot_id: String,
    pub url: String,
    pub loader_id: String,
    pub nodes: Vec<SemanticNode>,
    pub completeness: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_is_bounded_before_browser_use() {
        let spec = SemanticSnapshotSpec {
            selector: "body".into(),
            max_items: usize::MAX,
            max_text_chars: usize::MAX,
            max_bytes: usize::MAX,
        }
        .bounded();
        assert_eq!(spec.max_items, 500);
        assert_eq!(spec.max_text_chars, 10_000);
        assert_eq!(spec.max_bytes, 1_000_000);
    }
}
