//! Targeted visual-probe identities. Full-page vision is deliberately not the default path.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProbeRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl ProbeRegion {
    pub fn valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width > 0.0
            && self.height > 0.0
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VisualTargetRef {
    pub target_id: String,
    pub target_revision: u64,
    pub frame_id: String,
    pub frame_revision: u64,
    pub document_revision: u64,
    pub region: ProbeRegion,
    pub content_hash: String,
    pub observed_at_ms: u64,
}

impl VisualTargetRef {
    pub fn valid_for(
        &self,
        target_id: &str,
        target_revision: u64,
        frame_id: &str,
        frame_revision: u64,
        document_revision: u64,
        now_ms: u64,
        max_age_ms: u64,
    ) -> bool {
        !self.content_hash.is_empty()
            && self.region.valid()
            && self.target_id == target_id
            && self.target_revision == target_revision
            && self.frame_id == frame_id
            && self.frame_revision == frame_revision
            && self.document_revision == document_revision
            && self.observed_at_ms <= now_ms
            && now_ms.saturating_sub(self.observed_at_ms) <= max_age_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_refs_expire_on_document_revision_or_age() {
        let reference = VisualTargetRef {
            target_id: "t".into(), target_revision: 2, frame_id: "f".into(), frame_revision: 3,
            document_revision: 4, region: ProbeRegion { x: 1.0, y: 2.0, width: 20.0, height: 10.0 },
            content_hash: "abc".into(), observed_at_ms: 100,
        };
        assert!(reference.valid_for("t", 2, "f", 3, 4, 120, 50));
        assert!(!reference.valid_for("t", 2, "f", 3, 5, 120, 50));
        assert!(!reference.valid_for("t", 2, "f", 3, 4, 200, 50));
    }
}
