use crate::verifier::Verification;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, VecDeque};

pub const SKILL_SCHEMA_VERSION: u32 = 1;
pub const OUTCOME_WINDOW: usize = 20;
pub const MINIMUM_PASS_PERCENT: usize = 60;
const MAX_REVERSIBLE_SKILL_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const MAX_CONSEQUENTIAL_SKILL_TTL_MS: u64 = 60 * 60 * 1000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    Candidate,
    Qualified,
    Quarantined,
    Expired,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct SkillDefinition {
    pub skill_id: String,
    pub schema_version: u32,
    pub site_scope: String,
    pub intent_fingerprint: String,
    /// Optional installation-keyed HMAC of an explicitly verified account marker.
    /// Raw marker selector/text must never be persisted in a reusable skill.
    #[serde(default)]
    pub account_identity_fingerprint: Option<String>,
    pub parameter_names: Vec<String>,
    pub preconditions: Vec<String>,
    pub workflow: Value,
    pub postconditions: Vec<String>,
    #[serde(default)]
    pub failure_guards: Vec<String>,
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
            && self
                .account_identity_fingerprint
                .as_deref()
                .is_none_or(valid_account_identity_fingerprint)
            && !self.workflow.is_null()
            && !self.preconditions.is_empty()
            && !self.postconditions.is_empty()
            && self.failure_guards.len() <= 64
            && self
                .failure_guards
                .iter()
                .all(|guard| !guard.is_empty() && guard.len() <= 256)
            && !self.structural_signatures.is_empty()
    }

    /// Infer risk from the executable workflow itself; caller labels and prose
    /// cannot downgrade a consequential action.
    fn risk_tier(&self) -> SkillRisk {
        let encoded = self.workflow.to_string().to_ascii_lowercase();
        if encoded.contains("form_fill_save") {
            return SkillRisk::ReversibleMutation;
        }
        if [
            "submit",
            "purchase",
            "checkout",
            "delete",
            "send",
            "transfer",
            "publish",
            "account_change",
            "save",
        ]
        .iter()
        .any(|token| encoded.contains(token))
        {
            SkillRisk::Consequential
        } else if ["fill", "type", "click", "upload", "form"]
            .iter()
            .any(|token| encoded.contains(token))
        {
            SkillRisk::ReversibleMutation
        } else {
            SkillRisk::ReadOnly
        }
    }

    /// Whether replay must be bound to a verified current account identity.
    pub fn requires_account_identity(&self) -> bool {
        self.risk_tier() != SkillRisk::ReadOnly
    }
}

pub(crate) fn valid_account_identity_fingerprint(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("hmac-sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SkillRisk {
    ReadOnly,
    ReversibleMutation,
    Consequential,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SkillRecord {
    pub definition: SkillDefinition,
    pub status: SkillStatus,
    pub training_run_ids: BTreeSet<String>,
    pub validation_run_ids: BTreeSet<String>,
    #[serde(default)]
    pub training_input_signatures: BTreeSet<String>,
    #[serde(default)]
    pub validation_input_signatures: BTreeSet<String>,
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
            training_input_signatures: BTreeSet::new(),
            validation_input_signatures: BTreeSet::new(),
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

    pub fn record_training_example(
        &mut self,
        run_id: impl Into<String>,
        input_signature: impl Into<String>,
    ) {
        let run_id = run_id.into();
        let signature = input_signature.into();
        if !run_id.trim().is_empty() && valid_input_signature(&signature) {
            self.training_run_ids.insert(run_id);
            self.training_input_signatures.insert(signature);
        }
    }

    pub fn record_validation_example(
        &mut self,
        run_id: impl Into<String>,
        input_signature: impl Into<String>,
    ) {
        let run_id = run_id.into();
        let signature = input_signature.into();
        if !run_id.trim().is_empty() && valid_input_signature(&signature) {
            self.validation_run_ids.insert(run_id);
            self.validation_input_signatures.insert(signature);
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
        let distinct_runs = self.training_run_ids.iter().any(|training| {
            self.validation_run_ids
                .iter()
                .any(|validation| validation != training)
        });
        let (minimum_training, minimum_validation, max_ttl) = match self.definition.risk_tier() {
            SkillRisk::ReadOnly => (2, 1, MAX_REVERSIBLE_SKILL_TTL_MS),
            SkillRisk::ReversibleMutation => (3, 2, MAX_REVERSIBLE_SKILL_TTL_MS),
            SkillRisk::Consequential => (4, 3, MAX_CONSEQUENTIAL_SKILL_TTL_MS),
        };
        let varied_inputs = self.training_input_signatures.len() >= minimum_training
            && self.validation_input_signatures.len() >= minimum_validation
            && self
                .validation_input_signatures
                .iter()
                .any(|signature| !self.training_input_signatures.contains(signature));
        if !distinct_runs
            || !varied_inputs
            || expires_at_ms.saturating_sub(qualified_at_ms) > max_ttl
        {
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

fn valid_input_signature(signature: &str) -> bool {
    !signature.trim().is_empty() && signature.len() <= 256
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
            account_identity_fingerprint: None,
            parameter_names: vec!["email".into()],
            preconditions: vec!["signed_in".into()],
            workflow: json!({"steps":["find","fill","verify"]}),
            postconditions: vec!["email_matches".into()],
            failure_guards: Vec::new(),
            structural_signatures: vec!["form:registration:v1".into()],
            version: 1,
        }
    }

    #[test]
    fn qualification_requires_distinct_training_validation_and_runtime_proof() {
        let mut record = SkillRecord::candidate(definition()).unwrap();
        record.record_training_example("run-a", "input-a");
        record.record_training_example("run-c", "input-c");
        record.record_training_example("run-d", "input-d");
        record.record_validation_example("run-a", "input-a");
        assert_eq!(
            record.qualify(Verification::Passed, 100, 200),
            Err(SkillError::MissingQualificationRuns)
        );
        record.record_validation_example("run-b", "input-b");
        record.record_validation_example("run-e", "input-e");
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
    fn account_identity_scope_accepts_only_bounded_hmac_fingerprints() {
        let mut definition = definition();
        definition.account_identity_fingerprint = Some(format!("hmac-sha256:{}", "a".repeat(64)));
        assert!(definition.valid());
        definition.account_identity_fingerprint = Some("person@example.test".into());
        assert!(!definition.valid());
        definition.account_identity_fingerprint = Some(format!("hmac-sha256:{}", "A".repeat(64)));
        assert!(!definition.valid());
    }

    #[test]
    fn consequential_workflows_require_stronger_evidence_and_short_expiry() {
        let mut definition = definition();
        definition.workflow = json!({"steps":["fill","submit"]});
        let mut record = SkillRecord::candidate(definition).unwrap();
        for (run, input) in [("a", "i1"), ("b", "i2"), ("c", "i3"), ("d", "i4")] {
            record.record_training_example(run, input);
        }
        for (run, input) in [("e", "i5"), ("f", "i6"), ("g", "i7")] {
            record.record_validation_example(run, input);
        }
        assert_eq!(
            record.qualify(
                Verification::Passed,
                100,
                100 + MAX_CONSEQUENTIAL_SKILL_TTL_MS + 1
            ),
            Err(SkillError::MissingQualificationRuns)
        );
        record
            .qualify(
                Verification::Passed,
                100,
                100 + MAX_CONSEQUENTIAL_SKILL_TTL_MS,
            )
            .unwrap();
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
