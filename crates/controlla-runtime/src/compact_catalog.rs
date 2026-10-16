//! Compact public concepts over the compatibility tool surface.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicConcept {
    Run,
    Observe,
    Artifact,
}

pub const PUBLIC_CONCEPTS: &[&str] = &["run", "observe", "artifact"];

pub fn concept_for_legacy_tool(tool: &str) -> Option<PublicConcept> {
    match tool {
        "browser" | "act" | "workflow" => Some(PublicConcept::Run),
        "snapshot" | "extract" | "verify" => Some(PublicConcept::Observe),
        "evidence_read" | "artifact_register" => Some(PublicConcept::Artifact),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_tools_map_to_three_concepts() {
        assert_eq!(concept_for_legacy_tool("act"), Some(PublicConcept::Run));
        assert_eq!(
            concept_for_legacy_tool("snapshot"),
            Some(PublicConcept::Observe)
        );
        assert_eq!(
            concept_for_legacy_tool("evidence_read"),
            Some(PublicConcept::Artifact)
        );
    }
}
