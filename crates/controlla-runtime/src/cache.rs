use crate::verifier::Verification;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const CACHE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Provenance {
    pub training_run_id: String,
    pub validation_run_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkflowDefinition {
    pub workflow_id: String,
    pub schema_version: u32,
    pub principal: String,
    pub session: String,
    pub target: String,
    pub revision: String,
    pub site_scope: String,
    pub app_signature: String,
    pub input_schema: Value,
    pub content_hash: String,
    pub expected_permissions: Vec<String>,
    pub identity_requirements: Vec<String>,
    pub read_write_footprint: Vec<String>,
    pub verifier_revision: String,
    pub failure_policy: String,
    pub browser_version: String,
    pub app_version: String,
    pub route_version: String,
    pub environment: String,
    pub authority_preconditions: Vec<String>,
    pub predicate_hash: String,
}

impl WorkflowDefinition {
    /// Hashes every cache precondition; set-like vectors are sorted before hashing.
    pub fn signature(&self) -> String {
        let mut canonical = self.clone();
        canonical.expected_permissions.sort();
        canonical.expected_permissions.dedup();
        canonical.identity_requirements.sort();
        canonical.identity_requirements.dedup();
        canonical.read_write_footprint.sort();
        canonical.read_write_footprint.dedup();
        canonical.authority_preconditions.sort();
        canonical.authority_preconditions.dedup();
        let bytes = serde_json::to_vec(&canonical).expect("workflow definition serializes");
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn valid(&self) -> bool {
        self.schema_version == CACHE_SCHEMA_VERSION
            && [
                &self.workflow_id,
                &self.principal,
                &self.session,
                &self.target,
                &self.revision,
                &self.site_scope,
                &self.app_signature,
                &self.content_hash,
                &self.verifier_revision,
                &self.failure_policy,
                &self.browser_version,
                &self.app_version,
                &self.route_version,
                &self.environment,
                &self.predicate_hash,
            ]
            .iter()
            .all(|value| !value.is_empty())
            && !self.input_schema.is_null()
            && !self.expected_permissions.is_empty()
            && !self.identity_requirements.is_empty()
            && !self.authority_preconditions.is_empty()
    }
}

/// A run receipt is constructed only by runtime qualification code after it observes a suite.
/// This crate currently has no production observer or suite runner, so these receipts cannot
/// yet be produced outside internal unit tests.
pub struct QualificationRun {
    workflow_signature: String,
    run_id: String,
    split: &'static str,
    result: Verification,
}

impl QualificationRun {
    #[cfg(test)]
    fn observed(
        workflow_signature: String,
        run_id: String,
        split: &'static str,
        result: Verification,
    ) -> Self {
        Self {
            workflow_signature,
            run_id,
            split,
            result,
        }
    }
}

pub struct QualificationSuite {
    training: QualificationRun,
    validation: QualificationRun,
}

impl QualificationSuite {
    #[cfg(test)]
    fn observed(training: QualificationRun, validation: QualificationRun) -> Self {
        Self {
            training,
            validation,
        }
    }
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum QualificationError {
    #[error("workflow definition is incomplete or uses an unsupported schema")]
    InvalidWorkflow,
    #[error("qualification expiry must follow the qualification time")]
    InvalidExpiry,
    #[error("training and validation receipts must bind the exact workflow and split")]
    ReceiptBinding,
    #[error("training and validation receipts must be distinct successful runs")]
    UnsuccessfulSuite,
}

struct CacheEntry {
    definition: WorkflowDefinition,
    signature: String,
    provenance: Provenance,
    qualified_at_ms: u64,
    expires_at_ms: u64,
}

/// Opaque, single-use admission proof. There is no public constructor or serialization path.
pub struct QualificationToken(CacheEntry);

impl std::fmt::Debug for QualificationToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("QualificationToken")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheLookup {
    Hit(Provenance),
    Miss(&'static str),
    Quarantined(String),
}

#[derive(Default)]
pub struct WorkflowCache {
    entries: HashMap<String, CacheEntry>,
    quarantine: HashMap<String, String>,
}

impl WorkflowCache {
    pub fn qualify(
        &self,
        definition: WorkflowDefinition,
        suite: QualificationSuite,
        qualified_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<QualificationToken, QualificationError> {
        if !definition.valid() {
            return Err(QualificationError::InvalidWorkflow);
        }
        if expires_at_ms <= qualified_at_ms {
            return Err(QualificationError::InvalidExpiry);
        }
        let signature = definition.signature();
        if suite.training.workflow_signature != signature
            || suite.validation.workflow_signature != signature
            || suite.training.split != "training"
            || suite.validation.split != "validation"
        {
            return Err(QualificationError::ReceiptBinding);
        }
        if suite.training.result != Verification::Passed
            || suite.validation.result != Verification::Passed
            || suite.training.run_id.is_empty()
            || suite.validation.run_id.is_empty()
            || suite.training.run_id == suite.validation.run_id
        {
            return Err(QualificationError::UnsuccessfulSuite);
        }
        Ok(QualificationToken(CacheEntry {
            definition,
            signature,
            provenance: Provenance {
                training_run_id: suite.training.run_id,
                validation_run_id: suite.validation.run_id,
            },
            qualified_at_ms,
            expires_at_ms,
        }))
    }

    pub fn insert(&mut self, token: QualificationToken) -> String {
        let entry = token.0;
        let id = entry.definition.workflow_id.clone();
        self.quarantine.remove(&id);
        self.entries.insert(id.clone(), entry);
        id
    }

    /// Any changed precondition retires the entry. Re-admission requires a new qualification token.
    pub fn lookup(
        &mut self,
        workflow_id: &str,
        current: &WorkflowDefinition,
        now_ms: u64,
    ) -> CacheLookup {
        if let Some(reason) = self.quarantine.get(workflow_id) {
            return CacheLookup::Quarantined(reason.clone());
        }
        let Some(entry) = self.entries.get(workflow_id) else {
            return CacheLookup::Miss("unknown workflow");
        };
        let reason = if !current.valid() || current.workflow_id != workflow_id {
            Some("invalid current workflow identity")
        } else if entry.signature != current.signature() {
            Some("workflow precondition drift")
        } else if now_ms < entry.qualified_at_ms || now_ms >= entry.expires_at_ms {
            Some("qualification expired")
        } else {
            None
        };
        if let Some(reason) = reason {
            self.quarantine(workflow_id, reason);
            return CacheLookup::Quarantined(reason.to_owned());
        }
        CacheLookup::Hit(entry.provenance.clone())
    }

    pub fn report_verification(&mut self, workflow_id: &str, result: Verification) {
        if result != Verification::Passed {
            self.quarantine(workflow_id, format!("outcome verification: {result:?}"));
        }
    }

    pub fn quarantine(&mut self, workflow_id: &str, reason: impl Into<String>) {
        if self.entries.remove(workflow_id).is_some() || self.quarantine.contains_key(workflow_id) {
            self.quarantine.insert(workflow_id.into(), reason.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            workflow_id: "save-document".into(),
            schema_version: CACHE_SCHEMA_VERSION,
            principal: "alice".into(),
            session: "session-1".into(),
            target: "tab-1".into(),
            revision: "rev-7".into(),
            site_scope: "https://fixture.test".into(),
            app_signature: "fixture-app-v3".into(),
            input_schema: json!({"title":"string"}),
            content_hash: "workflow-content-hash".into(),
            expected_permissions: vec!["read".into(), "write".into()],
            identity_requirements: vec!["account:alice".into()],
            read_write_footprint: vec!["document.title".into()],
            verifier_revision: "verifier-v1".into(),
            failure_policy: "quarantine".into(),
            browser_version: "chrome-154".into(),
            app_version: "fixture-app-v3".into(),
            route_version: "direct-cdp-v1".into(),
            environment: "macos-arm64".into(),
            authority_preconditions: vec!["edit:fixture.test".into()],
            predicate_hash: "saved-title-predicate".into(),
        }
    }

    fn suite(definition: &WorkflowDefinition) -> QualificationSuite {
        let signature = definition.signature();
        QualificationSuite::observed(
            QualificationRun::observed(
                signature.clone(),
                "training-1".into(),
                "training",
                Verification::Passed,
            ),
            QualificationRun::observed(
                signature,
                "validation-1".into(),
                "validation",
                Verification::Passed,
            ),
        )
    }

    fn admit(cache: &mut WorkflowCache, definition: WorkflowDefinition) {
        let token = cache
            .qualify(definition, suite(&workflow()), 1_000, 5_000)
            .unwrap();
        cache.insert(token);
    }

    #[test]
    fn canonical_signature_covers_cache_contract_and_normalizes_set_fields() {
        let baseline = workflow();
        let mut reordered = baseline.clone();
        reordered.expected_permissions.reverse();
        assert_eq!(baseline.signature(), reordered.signature());

        for changed in [
            WorkflowDefinition {
                site_scope: "https://other.test".into(),
                ..baseline.clone()
            },
            WorkflowDefinition {
                app_signature: "changed-app".into(),
                ..baseline.clone()
            },
            WorkflowDefinition {
                input_schema: json!({"title":"number"}),
                ..baseline.clone()
            },
            WorkflowDefinition {
                content_hash: "changed-content".into(),
                ..baseline.clone()
            },
            WorkflowDefinition {
                expected_permissions: vec!["read".into()],
                ..baseline.clone()
            },
            WorkflowDefinition {
                identity_requirements: vec!["account:bob".into()],
                ..baseline.clone()
            },
            WorkflowDefinition {
                read_write_footprint: vec!["document.body".into()],
                ..baseline.clone()
            },
            WorkflowDefinition {
                verifier_revision: "verifier-v2".into(),
                ..baseline.clone()
            },
            WorkflowDefinition {
                failure_policy: "retry".into(),
                ..baseline.clone()
            },
        ] {
            assert_ne!(baseline.signature(), changed.signature());
        }
    }

    #[test]
    fn only_successful_bound_training_and_validation_receipts_mint_tokens() {
        let cache = WorkflowCache::default();
        let definition = workflow();
        let wrong_signature = QualificationSuite::observed(
            QualificationRun::observed(
                "wrong".into(),
                "training-1".into(),
                "training",
                Verification::Passed,
            ),
            QualificationRun::observed(
                "wrong".into(),
                "validation-1".into(),
                "validation",
                Verification::Passed,
            ),
        );
        assert_eq!(
            cache
                .qualify(definition.clone(), wrong_signature, 1_000, 5_000)
                .unwrap_err(),
            QualificationError::ReceiptBinding
        );
        let failed = QualificationSuite::observed(
            QualificationRun::observed(
                definition.signature(),
                "training-2".into(),
                "training",
                Verification::Passed,
            ),
            QualificationRun::observed(
                definition.signature(),
                "validation-2".into(),
                "validation",
                Verification::Inconclusive,
            ),
        );
        assert_eq!(
            cache.qualify(definition, failed, 1_000, 5_000).unwrap_err(),
            QualificationError::UnsuccessfulSuite
        );
    }

    #[test]
    fn drift_and_verification_failure_quarantine_until_new_qualification() {
        let mut cache = WorkflowCache::default();
        let definition = workflow();
        admit(&mut cache, definition.clone());
        assert!(matches!(
            cache.lookup("save-document", &definition, 2_000),
            CacheLookup::Hit(_)
        ));

        let drifted = WorkflowDefinition {
            target: "reused-tab".into(),
            ..definition.clone()
        };
        assert!(matches!(
            cache.lookup("save-document", &drifted, 2_000),
            CacheLookup::Quarantined(_)
        ));
        assert!(matches!(
            cache.lookup("save-document", &definition, 2_000),
            CacheLookup::Quarantined(_)
        ));

        admit(&mut cache, definition.clone());
        cache.report_verification("save-document", Verification::Inconclusive);
        assert!(matches!(
            cache.lookup("save-document", &definition, 2_000),
            CacheLookup::Quarantined(_)
        ));
        admit(&mut cache, definition.clone());
        cache.report_verification("save-document", Verification::Failed);
        assert!(matches!(
            cache.lookup("save-document", &definition, 2_000),
            CacheLookup::Quarantined(_)
        ));
        admit(&mut cache, definition.clone());
        assert!(matches!(
            cache.lookup("save-document", &definition, 2_000),
            CacheLookup::Hit(_)
        ));
    }
}
