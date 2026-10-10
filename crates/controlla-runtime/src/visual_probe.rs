//! Bounded targeted visual probes under six-tool surface.
//! Kept inside snapshot/workflow; no seventh tool.

use serde::{Deserialize, Serialize};

pub const MAX_PROBES_PER_WORKFLOW: usize = 3;
pub const MAX_EVIDENCE_BYTES: usize = 512 * 1024;

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
}
