use crate::verifier::Verification;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, VecDeque};

pub const SKILL_SCHEMA_VERSION: u32 = 1;
pub const OUTCOME_WINDOW: usize = 20;
pub const MINIMUM_PASS_PERCENT: usize = 60;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    Candidate,
    Qualified,
    Quarantined,
    Expired,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SkillDefinition {
    pub skill_id: String,
    pub schema_version: u32,
    pub site_scope: String,
    pub intent_fingerprint: String,
    pub parameter_names: Vec<String>,
    pub preconditions: Vec<String>,
    pub workflow: Value,
    pub postconditions: Vec<String>,
    pub structural_signatures: Vec<String>,
    pub version: u32,
}

impl SkillDefinition {
    pub fn valid(&self) -> bool {
        self.schema_version == SKILL_SCHEMA_VERSION
            && self.version > 0
            && !self.skill_id.trim().is_empty()
            && !self.site_scope.trim().is_empty()
            && !self.intent_fingerprint.trim().is_empty()
            && !self.workflow.is_null()
            && !self.preconditions.is_empty()
            && !self.postconditions.is_empty()
            && !self.structural_signatures.is_empty()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SkillRecord {
    pub definition: SkillDefinition,
    pub status: SkillStatus,
    pub training_run_ids: BTreeSet<String>,
    pub validation_run_ids: BTreeSet<String>,
    pub recent_outcomes: VecDeque<bool>,
    pub qualified_at_ms: Option<u64>,
    pub expires_at_ms: Option<u64>,
    pub quarantine_reason: Option<String>,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum SkillError {
    #[error("skill definition is incomplete or uses an unsupported schema")]
    InvalidDefinition,
    #[error("training and validation require distinct successful runs")]
    MissingQualificationRuns,
    #[error("runtime verification did not pass")]
    RuntimeVerification,
    #[error("qualification expiry must follow qualification time")]
    InvalidExpiry,
}

impl SkillRecord {
    pub fn candidate(definition: SkillDefinition) -> Result<Self, SkillError> {
        if !definition.valid() {
            return Err(SkillError::InvalidDefinition);
        }
        Ok(Self {
            definition,
            status: SkillStatus::Candidate,
            training_run_ids: BTreeSet::new(),
            validation_run_ids: BTreeSet::new(),
            recent_outcomes: VecDeque::with_capacity(OUTCOME_WINDOW),
            qualified_at_ms: None,
            expires_at_ms: None,
            quarantine_reason: None,
        })
    }

    pub fn record_training_success(&mut self, run_id: impl Into<String>) {
        let run_id = run_id.into();
        if !run_id.trim().is_empty() {
            self.training_run_ids.insert(run_id);
        }
    }

    pub fn record_validation_success(&mut self, run_id: impl Into<String>) {
        let run_id = run_id.into();
        if !run_id.trim().is_empty() {
            self.validation_run_ids.insert(run_id);
        }
    }

    pub fn qualify(
        &mut self,
        runtime_verification: Verification,
        qualified_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<(), SkillError> {
        if expires_at_ms <= qualified_at_ms {
            return Err(SkillError::InvalidExpiry);
        }
        let distinct = self.training_run_ids.iter().any(|training| {
            self.validation_run_ids
                .iter()
                .any(|validation| validation != training)
        });
        if !distinct {
            return Err(SkillError::MissingQualificationRuns);
        }
        if runtime_verification != Verification::Passed {
            return Err(SkillError::RuntimeVerification);
        }
        self.status = SkillStatus::Qualified;
        self.qualified_at_ms = Some(qualified_at_ms);
        self.expires_at_ms = Some(expires_at_ms);
        self.quarantine_reason = None;
        Ok(())
    }

    pub fn record_runtime_verification(
        &mut self,
        verification: Verification,
        severe_safety_failure: bool,
        structural_drift: bool,
    ) {
        if severe_safety_failure {
            self.quarantine("severe safety failure");
            return;
        }
        if structural_drift {
            self.quarantine("structural drift");
            return;
        }
        if self.recent_outcomes.len() == OUTCOME_WINDOW {
            self.recent_outcomes.pop_front();
        }
        self.recent_outcomes
            .push_back(verification == Verification::Passed);
        if self.recent_outcomes.len() == OUTCOME_WINDOW {
            let passed = self
                .recent_outcomes
                .iter()
                .filter(|passed| **passed)
                .count();
            if passed * 100 < MINIMUM_PASS_PERCENT * OUTCOME_WINDOW {
                self.quarantine("rolling verification pass rate below 60 percent");
            }
        }
    }

    pub fn replay_allowed(&mut self, now_ms: u64) -> bool {
        if self.status != SkillStatus::Qualified {
            return false;
        }
        match (self.qualified_at_ms, self.expires_at_ms) {
            (Some(start), Some(end)) if now_ms >= start && now_ms < end => true,
            _ => {
                self.status = SkillStatus::Expired;
                false
            }
        }
    }

    pub fn quarantine(&mut self, reason: impl Into<String>) {
        self.status = SkillStatus::Quarantined;
        self.quarantine_reason = Some(reason.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn definition() -> SkillDefinition {
        SkillDefinition {
            skill_id: "fill-registration".into(),
            schema_version: SKILL_SCHEMA_VERSION,
            site_scope: "https://example.test".into(),
            intent_fingerprint: "registration-form".into(),
            parameter_names: vec!["email".into()],
            preconditions: vec!["signed_in".into()],
            workflow: json!({"steps":["find","fill","verify"]}),
            postconditions: vec!["email_matches".into()],
            structural_signatures: vec!["form:registration:v1".into()],
            version: 1,
        }
    }

    #[test]
    fn qualification_requires_distinct_training_validation_and_runtime_proof() {
        let mut record = SkillRecord::candidate(definition()).unwrap();
        record.record_training_success("run-a");
        record.record_validation_success("run-a");
        assert_eq!(
            record.qualify(Verification::Passed, 100, 200),
            Err(SkillError::MissingQualificationRuns)
        );
        record.record_validation_success("run-b");
        assert_eq!(
            record.qualify(Verification::Inconclusive, 100, 200),
            Err(SkillError::RuntimeVerification)
        );
        record.qualify(Verification::Passed, 100, 200).unwrap();
        assert!(record.replay_allowed(150));
        assert!(!record.replay_allowed(200));
        assert_eq!(record.status, SkillStatus::Expired);
    }

    #[test]
    fn severe_failure_and_structural_drift_quarantine_immediately() {
        let mut record = SkillRecord::candidate(definition()).unwrap();
        record.record_runtime_verification(Verification::Passed, true, false);
        assert_eq!(record.status, SkillStatus::Quarantined);
        let mut record = SkillRecord::candidate(definition()).unwrap();
        record.record_runtime_verification(Verification::Passed, false, true);
        assert_eq!(record.status, SkillStatus::Quarantined);
    }

    #[test]
    fn rolling_window_quarantines_only_after_full_window_below_threshold() {
        let mut record = SkillRecord::candidate(definition()).unwrap();
        for index in 0..OUTCOME_WINDOW {
            let result = if index < 11 {
                Verification::Passed
            } else {
                Verification::Failed
            };
            record.record_runtime_verification(result, false, false);
        }
        assert_eq!(record.status, SkillStatus::Quarantined);
    }
}
