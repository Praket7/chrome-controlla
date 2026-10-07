use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const CACHE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Provenance {
    pub training: Option<String>,
    pub validation: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QualifiedWorkflow {
    pub signature: String,
    pub schema_version: u32,
    pub principal: String,
    pub session: String,
    pub target: String,
    pub revision: String,
    pub environment: String,
    pub authority: String,
    pub predicate_hash: String,
    pub qualified_at_ms: u64,
    pub expires_at_ms: u64,
    pub provenance: Provenance,
}

#[derive(Clone, Debug)]
pub struct LookupContext {
    pub principal: String,
    pub session: String,
    pub target: String,
    pub revision: String,
    pub environment: String,
    pub authority: String,
    pub predicate_hash: String,
    pub now_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheLookup {
    Hit(Box<QualifiedWorkflow>),
    Miss(&'static str),
    Quarantined(String),
}

#[derive(Default)]
pub struct WorkflowCache {
    workflows: HashMap<String, QualifiedWorkflow>,
    quarantine: HashMap<String, String>,
}

impl WorkflowCache {
    pub fn insert(&mut self, workflow: QualifiedWorkflow) -> bool {
        if workflow.signature.is_empty()
            || workflow.schema_version != CACHE_SCHEMA_VERSION
            || [
                &workflow.principal,
                &workflow.session,
                &workflow.target,
                &workflow.revision,
                &workflow.environment,
                &workflow.authority,
                &workflow.predicate_hash,
            ]
            .iter()
            .any(|value| value.is_empty())
            || workflow.expires_at_ms <= workflow.qualified_at_ms
            || workflow
                .provenance
                .training
                .as_deref()
                .is_none_or(str::is_empty)
            || workflow
                .provenance
                .validation
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return false;
        }
        self.quarantine.remove(&workflow.signature);
        self.workflows.insert(workflow.signature.clone(), workflow);
        true
    }

    pub fn lookup(&self, signature: &str, context: &LookupContext) -> CacheLookup {
        if let Some(reason) = self.quarantine.get(signature) {
            return CacheLookup::Quarantined(reason.clone());
        }
        let Some(workflow) = self.workflows.get(signature) else {
            return CacheLookup::Miss("unknown workflow");
        };
        if workflow.schema_version != CACHE_SCHEMA_VERSION {
            return CacheLookup::Miss("cache version changed");
        }
        if workflow.principal != context.principal
            || workflow.session != context.session
            || workflow.target != context.target
            || workflow.revision != context.revision
            || workflow.environment != context.environment
            || workflow.authority != context.authority
            || workflow.predicate_hash != context.predicate_hash
        {
            return CacheLookup::Miss("precondition drift");
        }
        if context.now_ms < workflow.qualified_at_ms || context.now_ms >= workflow.expires_at_ms {
            return CacheLookup::Miss("expired qualification");
        }
        CacheLookup::Hit(Box::new(workflow.clone()))
    }

    pub fn quarantine(&mut self, signature: &str, reason: impl Into<String>) {
        if self.workflows.contains_key(signature) {
            self.quarantine.insert(signature.into(), reason.into());
        }
    }
}
