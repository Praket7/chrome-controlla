//! Bounded targeted visual probes under six-tool surface.
//! Kept inside snapshot/workflow; no seventh tool.

use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::BTreeMap;
use std::collections::VecDeque;

pub const MAX_PROBES_PER_WORKFLOW: usize = 3;
pub const MAX_EVIDENCE_BYTES: usize = 512 * 1024;
pub const MAX_TASK_EVIDENCE_ENTRIES: usize = 64;
pub const MAX_TASK_EVIDENCE_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvidenceEntry {
    pub step: u32,
    pub kind: String,
    pub public_ref: Option<String>,
    pub frame_id: Option<String>,
    pub fact: String,
    pub evidence_hash: String,
    pub verified: bool,
}

/// Bounded, session-local memory of verified actions and explicit evidence.
#[derive(Clone, Debug, Default)]
pub struct EvidenceTrail {
    entries: VecDeque<EvidenceEntry>,
    bytes: usize,
    next_step: u32,
}

impl EvidenceTrail {
    pub fn from_entries(entries: impl IntoIterator<Item = EvidenceEntry>) -> Self {
        let mut trail = Self::default();
        for entry in entries {
            if entry.fact.len() > 2048
                || entry.kind.len() > 64
                || entry
                    .public_ref
                    .as_ref()
                    .is_some_and(|value| value.len() > 128)
            {
                continue;
            }
            let size = serde_json::to_vec(&entry).map_or(0, |value| value.len());
            if size > MAX_TASK_EVIDENCE_BYTES {
                continue;
            }
            while trail.entries.len() >= MAX_TASK_EVIDENCE_ENTRIES
                || trail.bytes + size > MAX_TASK_EVIDENCE_BYTES
            {
                if let Some(old) = trail.entries.pop_front() {
                    trail.bytes = trail
                        .bytes
                        .saturating_sub(serde_json::to_vec(&old).map_or(0, |value| value.len()));
                } else {
                    break;
                }
            }
            trail.next_step = trail.next_step.max(entry.step.saturating_add(1));
            trail.bytes += size;
            trail.entries.push_back(entry);
        }
        trail
    }

    pub fn record(
        &mut self,
        kind: impl Into<String>,
        public_ref: Option<String>,
        frame_id: Option<String>,
        fact: impl Into<String>,
        verified: bool,
    ) {
        let fact = fact.into();
        let kind = kind.into();
        if fact.len() > 2048
            || kind.len() > 64
            || public_ref.as_ref().is_some_and(|value| value.len() > 128)
        {
            return;
        }
        let digest = sha2::Sha256::digest(fact.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let entry = EvidenceEntry {
            step: self.next_step,
            kind,
            public_ref,
            frame_id,
            fact,
            evidence_hash: digest,
            verified,
        };
        self.next_step = self.next_step.saturating_add(1);
        let size = serde_json::to_vec(&entry).map_or(0, |value| value.len());
        if size > MAX_TASK_EVIDENCE_BYTES {
            return;
        }
        while self.entries.len() >= MAX_TASK_EVIDENCE_ENTRIES
            || self.bytes + size > MAX_TASK_EVIDENCE_BYTES
        {
            if let Some(old) = self.entries.pop_front() {
                self.bytes = self
                    .bytes
                    .saturating_sub(serde_json::to_vec(&old).map_or(0, |value| value.len()));
            } else {
                break;
            }
        }
        self.bytes += size;
        self.entries.push_back(entry);
    }

    pub fn recent(&self, limit: usize) -> Vec<EvidenceEntry> {
        self.entries
            .iter()
            .rev()
            .take(limit.min(16))
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// Return matching history hints without changing their trust or verification fields.
    pub fn recent_relevant(&self, intent: &str, limit: usize) -> Vec<EvidenceEntry> {
        let terms = intent
            .split(|character: char| !character.is_alphanumeric())
            .map(str::to_lowercase)
            .filter(|term| term.len() >= 3)
            .collect::<std::collections::BTreeSet<_>>();
        if terms.is_empty() {
            return self.recent(limit);
        }
        let mut ranked = self
            .entries
            .iter()
            .filter_map(|entry| {
                let content = format!(
                    "{} {} {}",
                    entry.kind,
                    entry.fact,
                    entry.public_ref.as_deref().unwrap_or_default()
                );
                let words = content
                    .split(|character: char| !character.is_alphanumeric())
                    .map(str::to_lowercase)
                    .collect::<std::collections::BTreeSet<_>>();
                let score = terms.intersection(&words).count();
                (score > 0).then_some((score, entry.step, entry))
            })
            .collect::<Vec<_>>();
        ranked
            .sort_by_key(|(score, step, _)| (std::cmp::Reverse(*score), std::cmp::Reverse(*step)));
        ranked
            .into_iter()
            .take(limit.min(16))
            .map(|(_, _, entry)| entry.clone())
            .collect()
    }

    pub fn byte_len(&self) -> usize {
        self.bytes
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ProbeRequest {
    ElementFocus { reference: String },
    RegionFocus { x: i32, y: i32, w: u32, h: u32 },
    RegisterVisual { label: String, evidence_id: String },
    CommitEvidence { evidence_id: String },
}

#[derive(Clone, Debug, Default)]
pub struct EvidenceMemory {
    entries: Vec<(String, Vec<u8>)>,
    total_bytes: usize,
}

/// Short-lived coordinate authority; semantic targets always take precedence.
#[derive(Clone, Debug, Default)]
pub struct VisualRegistry {
    entries: BTreeMap<String, controlla_browser::scheduler::probe::VisualTargetRef>,
    next: u64,
}

impl VisualRegistry {
    pub fn register(
        &mut self,
        reference: controlla_browser::scheduler::probe::VisualTargetRef,
        binding: controlla_browser::scheduler::probe::VisualBinding<'_>,
        now_ms: u64,
        max_age_ms: u64,
        semantic_candidates: usize,
    ) -> Result<String, &'static str> {
        if semantic_candidates != 0 {
            return Err("semantic_target_preferred");
        }
        if !reference.valid_for(binding, now_ms, max_age_ms) {
            return Err("visual_reference_stale_or_mismatched");
        }
        if self.entries.len() >= 32 {
            return Err("visual_reference_capacity_exceeded");
        }
        self.next = self.next.saturating_add(1);
        let id = format!("@v{}", self.next);
        self.entries.insert(id.clone(), reference);
        Ok(id)
    }

    pub fn resolve(
        &self,
        id: &str,
        binding: controlla_browser::scheduler::probe::VisualBinding<'_>,
        now_ms: u64,
        max_age_ms: u64,
    ) -> Result<&controlla_browser::scheduler::probe::VisualTargetRef, &'static str> {
        let reference = self.entries.get(id).ok_or("unknown_visual_reference")?;
        if !reference.valid_for(binding, now_ms, max_age_ms) {
            return Err("visual_reference_stale_or_mismatched");
        }
        Ok(reference)
    }

    /// Returns a center point only after fresh identity validation; the caller must still
    /// perform hit-test, protected-control checks, and immediate postcondition verification.
    pub fn guarded_center(
        reference: &controlla_browser::scheduler::probe::VisualTargetRef,
    ) -> (f64, f64) {
        (
            reference.region.x + reference.region.width / 2.0,
            reference.region.y + reference.region.height / 2.0,
        )
    }
}

impl EvidenceMemory {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, id: impl Into<String>, bytes: Vec<u8>) -> Result<(), String> {
        if self.entries.len() >= MAX_PROBES_PER_WORKFLOW {
            return Err("probe budget exceeded".into());
        }
        if self.total_bytes + bytes.len() > MAX_EVIDENCE_BYTES {
            return Err("evidence bytes exceeded".into());
        }
        self.total_bytes += bytes.len();
        self.entries.push((id.into(), bytes));
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn escalation_policy(has_semantics: bool, probe_count: usize) -> &'static str {
    if has_semantics {
        "use semantics"
    } else if probe_count < MAX_PROBES_PER_WORKFLOW {
        "targeted visual"
    } else {
        "full screenshot fallback"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_budget_enforced() {
        let mut mem = EvidenceMemory::new();
        for i in 0..MAX_PROBES_PER_WORKFLOW {
            assert!(mem.push(format!("e{i}"), vec![0; 1000]).is_ok());
        }
        assert!(mem.push("extra", vec![0; 100]).is_err());
    }

    #[test]
    fn task_evidence_is_bounded_and_keeps_only_the_recent_delta() {
        let mut trail = EvidenceTrail::default();
        for step in 0..MAX_TASK_EVIDENCE_ENTRIES + 5 {
            trail.record(
                "click",
                Some(format!("@c{step}")),
                None,
                format!("verified step {step}"),
                true,
            );
        }
        let recent = trail.recent(3);
        assert_eq!(recent.len(), 3);
        assert!(recent[0].fact.contains("step 66"));
        assert!(recent[2].verified);
        assert!(trail.byte_len() <= MAX_TASK_EVIDENCE_BYTES);
    }

    #[test]
    fn evidence_trail_restores_a_bounded_recent_history_and_continues_step_numbers() {
        let mut original = EvidenceTrail::default();
        original.record("commit", None, None, "verified invoice", true);
        let mut restored = EvidenceTrail::from_entries(original.recent(8));
        restored.record("click", Some("@c2".into()), None, "status=verified", true);
        assert_eq!(restored.recent(8)[0].fact, "verified invoice");
        assert!(restored.recent(8)[1].step > restored.recent(8)[0].step);
        assert!(restored.byte_len() <= MAX_TASK_EVIDENCE_BYTES);
    }

    #[test]
    fn visual_registration_prefers_semantics_and_expires_on_identity_or_age() {
        use controlla_browser::scheduler::probe::{ProbeRegion, VisualBinding, VisualTargetRef};
        let reference = VisualTargetRef {
            target_id: "t".into(),
            target_revision: 1,
            frame_id: "f".into(),
            frame_revision: 2,
            document_revision: 3,
            region: ProbeRegion {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 10.0,
            },
            content_hash: "hash".into(),
            observed_at_ms: 100,
        };
        let binding = VisualBinding {
            target_id: "t",
            target_revision: 1,
            frame_id: "f",
            frame_revision: 2,
            document_revision: 3,
        };
        let mut registry = VisualRegistry::default();
        assert_eq!(
            registry.register(reference.clone(), binding, 101, 100, 1),
            Err("semantic_target_preferred")
        );
        let id = registry.register(reference, binding, 101, 100, 0).unwrap();
        assert_eq!(
            VisualRegistry::guarded_center(registry.resolve(&id, binding, 102, 100).unwrap()),
            (10.0, 5.0)
        );
        assert_eq!(
            registry.resolve(
                &id,
                VisualBinding {
                    document_revision: 4,
                    ..binding
                },
                102,
                100
            ),
            Err("visual_reference_stale_or_mismatched")
        );
    }
}
