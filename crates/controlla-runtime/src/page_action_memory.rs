//! Exact-identity hints for previously verified page affordances.

use crate::verifier::Verification;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

pub const MAX_ACTION_MEMORY_ENTRIES: usize = 512;
pub const MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub const MAX_OPERATION_STEPS: usize = 24;
pub const MIN_OPERATION_PROMOTIONS: u8 = 2;
const OPERATION_ACTIONS: &[&str] = &[
    "navigate",
    "click",
    "double_click",
    "fill",
    "type",
    "press",
    "select",
    "scroll",
    "scroll_to",
    "hover",
    "extract",
    "verify_visible",
    "upload",
    "download",
    "save_draft",
    "wait_for",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplayStatus {
    Hit,
    Miss,
    Stale,
    Disabled,
}

/// Bounded structural identity for a validated operation target. Raw user
/// input never belongs here; callers re-resolve a live `@cN` before dispatch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetTemplate {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub href: Option<String>,
    #[serde(default)]
    pub ancestor: Option<String>,
    #[serde(default)]
    pub test_id: Option<String>,
}

/// Postcondition stored with a learned operation. Predicates carry identity
/// patterns, never the user's typed values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VerifyPredicate {
    UrlMatches { pattern: String },
    TextPresent { text: String },
    Visible { target: TargetTemplate },
    Hidden { target: TargetTemplate },
    AppState { adapter: String },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationEffect {
    #[default]
    ReadOnly,
    Mutating,
}

/// One verified state transition inside a learned operation. Target identity is
/// structural only; user-entered values stay in their value slots and never in
/// memory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationStep {
    pub action: String,
    #[serde(default)]
    pub target: Option<TargetTemplate>,
    #[serde(default)]
    pub postcondition: Option<VerifyPredicate>,
    #[serde(default)]
    pub value_slot: Option<String>,
}

/// A bounded multi-step operation promoted only after repeated independent
/// verification. It is an execution hint: callers must still re-resolve live
/// targets and re-check every stored postcondition before dispatch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationPlan {
    pub steps: Vec<OperationStep>,
    #[serde(default)]
    pub effect: OperationEffect,
    #[serde(default)]
    pub successes: u8,
    #[serde(default)]
    pub promoted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplayPlan {
    Navigate {
        url_template: String,
    },
    Click {
        target: TargetTemplate,
        outcome: VerifyPredicate,
    },
    /// A verified click whose outcome was only checked by the live action
    /// receipt. It is a target hint, not an independently replayable outcome.
    ClickHint {
        target: TargetTemplate,
    },
    FillHint {
        target: TargetTemplate,
        value_slot: String,
    },
    SelectHint {
        target: TargetTemplate,
        value_slot: String,
    },
    Fill {
        target: TargetTemplate,
        value_slot: String,
        outcome: VerifyPredicate,
    },
    Type {
        target: TargetTemplate,
        value_slot: String,
        #[serde(default)]
        strategy_hint: Option<String>,
    },
    Select {
        target: TargetTemplate,
        value_slot: String,
        outcome: VerifyPredicate,
    },
    Extract {
        selector: String,
    },
    Tool {
        capability_id: String,
        schema_hash: String,
    },
}

/// Proof that a cached operation was repaired after a successful reactive
/// fallback instead of being replayed blindly.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairMetadata {
    pub repaired_at_ms: u64,
    pub reason: String,
    #[serde(default)]
    pub previous_target: Option<TargetTemplate>,
}

#[cfg(test)]
mod tests {
    use super::{ActionMemory, ActionMemoryEntry, ActionMemoryKey, ReplayPlan};
    use crate::verifier::Verification;
    use serde_json::json;

    fn key() -> ActionMemoryKey {
        ActionMemoryKey {
            origin: "https://example.test".into(),
            structural_signature: "sig".into(),
            intent: "research:abc".into(),
        }
    }

    fn entry(selector: &str) -> ActionMemoryEntry {
        ActionMemoryEntry {
            semantic_target: json!({"selector": selector}),
            action: "extract".into(),
            expected_postcondition: json!({"kind":"nonempty_complete_extraction"}),
            replay: Some(ReplayPlan::Extract {
                selector: selector.into(),
            }),
            operation: None,
            preconditions: None,
            repair: None,
            last_verified_ms: 100,
        }
    }

    #[test]
    fn verified_replay_record_round_trips_and_can_be_invalidated() {
        let mut memory = ActionMemory::default();
        let key = key();
        assert!(memory.record(key.clone(), entry("main article"), Verification::Passed));
        assert!(memory.lookup(&key, 101).is_some());
        assert!(memory.invalidate(&key));
        assert!(memory.lookup(&key, 101).is_none());
    }

    #[test]
    fn replay_record_rejects_secret_bearing_targets() {
        let mut memory = ActionMemory::default();
        assert!(!memory.record(
            key(),
            entry("input[type=password]").clone(),
            Verification::Passed,
        ));
        assert!(!memory.record(
            key(),
            ActionMemoryEntry {
                semantic_target: json!({"selector":"main"}),
                action: "fill".into(),
                expected_postcondition: json!({"kind":"filled","value":"super-secret"}),
                replay: None,
                operation: None,
                preconditions: None,
                repair: None,
                last_verified_ms: 100,
            },
            Verification::Passed,
        ));
    }

    #[test]
    fn stale_records_are_not_replayable() {
        let mut memory = ActionMemory::default();
        let key = key();
        assert!(memory.record(key.clone(), entry("main"), Verification::Passed));
        assert!(memory.lookup(&key, 100 + super::MAX_AGE_MS + 1).is_none());
        assert_eq!(
            memory.replay_status(&key, 100 + super::MAX_AGE_MS + 1),
            super::ReplayStatus::Stale
        );
    }

    #[test]
    fn validated_operation_plans_store_slots_not_raw_values() {
        let mut memory = ActionMemory::default();
        let key = key();
        let target = super::TargetTemplate {
            role: Some("textbox".into()),
            name: Some("Email".into()),
            href: None,
            ancestor: Some("form#signup".into()),
            test_id: Some("signup-email".into()),
        };
        let plan = ReplayPlan::Fill {
            target: target.clone(),
            value_slot: "email_address".into(),
            outcome: super::VerifyPredicate::TextPresent {
                text: "Check your inbox".into(),
            },
        };
        let entry = ActionMemoryEntry {
            semantic_target: json!({"role":"textbox","name":"Email"}),
            action: "fill".into(),
            expected_postcondition: json!({"kind":"text_present"}),
            replay: Some(plan.clone()),
            operation: None,
            preconditions: Some(json!({"form":"signup","step":1})),
            repair: None,
            last_verified_ms: 100,
        };
        assert!(memory.record(key.clone(), entry, Verification::Passed));
        let stored = memory.lookup(&key, 101).unwrap();
        assert_eq!(stored.replay.as_ref(), Some(&plan));
        let encoded = serde_json::to_string(stored).unwrap();
        assert!(!encoded.contains("person@example.test"));
        assert_eq!(memory.replay_status(&key, 101), super::ReplayStatus::Hit);

        // A raw user value in the slot position is rejected by the slot grammar.
        let raw_value = ActionMemoryEntry {
            semantic_target: json!({"role":"textbox"}),
            action: "fill".into(),
            expected_postcondition: json!({"kind":"filled"}),
            replay: Some(ReplayPlan::Fill {
                target: target.clone(),
                value_slot: "person@example.test".into(),
                outcome: super::VerifyPredicate::UrlMatches {
                    pattern: "https://example.test/".into(),
                },
            }),
            operation: None,
            preconditions: None,
            repair: None,
            last_verified_ms: 100,
        };
        let raw_key = ActionMemoryKey {
            intent: "fill-raw".into(),
            ..key.clone()
        };
        assert!(!memory.record(raw_key, raw_value, Verification::Passed));
    }

    #[test]
    fn repaired_targets_keep_bounded_provenance() {
        let mut memory = ActionMemory::default();
        let key = key();
        assert!(memory.record(
            key.clone(),
            ActionMemoryEntry {
                semantic_target: json!({"role":"button","name":"Save"}),
                action: "click".into(),
                expected_postcondition: json!({"kind":"text_present","text":"Saved"}),
                replay: Some(ReplayPlan::Click {
                    target: super::TargetTemplate {
                        role: Some("button".into()),
                        name: Some("Save".into()),
                        href: None,
                        ancestor: None,
                        test_id: None,
                    },
                    outcome: super::VerifyPredicate::TextPresent {
                        text: "Saved".into(),
                    },
                }),
                operation: None,
                preconditions: None,
                repair: None,
                last_verified_ms: 100,
            },
            Verification::Passed
        ));
        assert!(memory.repair_target(
            &key,
            json!({"role":"button","name":"Save","ancestor":"dialog"}),
            Some(super::TargetTemplate {
                role: Some("button".into()),
                name: Some("Save".into()),
                href: None,
                ancestor: None,
                test_id: None,
            }),
            "reactive fallback verified Saved after dialog rerender",
            200,
        ));
        let repaired = memory.lookup(&key, 201).unwrap();
        assert_eq!(repaired.last_verified_ms, 200);
        assert_eq!(
            repaired.repair.as_ref().unwrap().previous_target,
            Some(super::TargetTemplate {
                role: Some("button".into()),
                name: Some("Save".into()),
                href: None,
                ancestor: None,
                test_id: None,
            })
        );
        assert!(!memory.repair_target(
            &key,
            json!({"role":"button"}),
            None,
            "password=secret",
            300,
        ));
    }

    fn operation_plan() -> super::OperationPlan {
        super::OperationPlan {
            steps: vec![
                super::OperationStep {
                    action: "navigate".into(),
                    target: None,
                    postcondition: None,
                    value_slot: None,
                },
                super::OperationStep {
                    action: "fill".into(),
                    target: Some(super::TargetTemplate {
                        role: Some("textbox".into()),
                        name: Some("Search".into()),
                        href: None,
                        ancestor: None,
                        test_id: None,
                    }),
                    postcondition: None,
                    value_slot: Some("value_1".into()),
                },
                super::OperationStep {
                    action: "click".into(),
                    target: Some(super::TargetTemplate {
                        role: Some("button".into()),
                        name: Some("Search".into()),
                        href: None,
                        ancestor: None,
                        test_id: None,
                    }),
                    postcondition: Some(super::VerifyPredicate::TextPresent {
                        text: "Results".into(),
                    }),
                    value_slot: None,
                },
            ],
            effect: super::OperationEffect::ReadOnly,
            successes: 1,
            promoted: false,
        }
    }

    #[test]
    fn operation_memory_promotes_only_after_two_independent_verified_passes() {
        let key = key();
        let mut memory = ActionMemory::default();
        assert!(memory.record_operation(key.clone(), operation_plan(), Verification::Passed));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(memory.lookup_operation(&key, now).is_none());
        assert_eq!(
            memory.replay_status(&key, now),
            super::ReplayStatus::Disabled
        );
        assert!(memory.record_operation(key.clone(), operation_plan(), Verification::Passed));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let promoted = memory.lookup_operation(&key, now).unwrap();
        let plan = promoted.operation.as_ref().unwrap();
        assert!(plan.promoted);
        assert_eq!(plan.successes, 2);
        assert_eq!(memory.replay_status(&key, now), super::ReplayStatus::Hit);
        assert!(memory.has_promoted_operation("https://example.test", "research:abc", now));
        assert!(!memory.has_promoted_operation("https://other.test", "research:abc", now));
        assert!(
            memory
                .lookup_operation(&key, now + super::MAX_AGE_MS + 1)
                .is_none()
        );
        assert!(!memory.record_operation(key.clone(), operation_plan(), Verification::Failed));
        assert!(memory.lookup(&key, now).is_none());
    }

    #[test]
    fn operation_memory_rejects_raw_values_and_unbounded_step_shapes() {
        let key = key();
        let mut memory = ActionMemory::default();
        let mut secret_slot = operation_plan();
        secret_slot.steps[1].value_slot = Some("person@example.test".into());
        assert!(!memory.record_operation(key.clone(), secret_slot, Verification::Passed));
        let mut unknown_action = operation_plan();
        unknown_action.steps[0].action = "password".into();
        assert!(!memory.record_operation(key.clone(), unknown_action, Verification::Passed));
        let mut oversized = operation_plan();
        oversized.steps = (0..super::MAX_OPERATION_STEPS + 1)
            .map(|_| oversized.steps[0].clone())
            .collect();
        assert!(!memory.record_operation(key, oversized, Verification::Passed));
    }

    #[test]
    fn secret_bearing_preconditions_are_rejected() {
        let mut memory = ActionMemory::default();
        let key = key();
        assert!(!memory.record(
            key,
            ActionMemoryEntry {
                semantic_target: json!({"role":"textbox"}),
                action: "fill".into(),
                expected_postcondition: json!({"kind":"filled"}),
                replay: None,
                operation: None,
                preconditions: Some(json!({"password":"hunter2"})),
                repair: None,
                last_verified_ms: 100,
            },
            Verification::Passed
        ));
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionMemoryKey {
    pub origin: String,
    pub structural_signature: String,
    pub intent: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionMemoryEntry {
    pub semantic_target: Value,
    pub action: String,
    pub expected_postcondition: Value,
    /// A bounded, non-secret replay description. Older persisted entries omit it.
    #[serde(default)]
    pub replay: Option<ReplayPlan>,
    /// Multi-step operation memory. Older persisted entries omit it.
    #[serde(default)]
    pub operation: Option<OperationPlan>,
    /// Required state observed immediately before the validated operation.
    #[serde(default)]
    pub preconditions: Option<Value>,
    /// Bounded proof that a drifted target was repaired after successful fallback.
    #[serde(default)]
    pub repair: Option<RepairMetadata>,
    pub last_verified_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ActionMemory {
    entries: BTreeMap<ActionMemoryKey, ActionMemoryEntry>,
    path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct StoredActionMemory {
    version: u32,
    entries: Vec<(ActionMemoryKey, ActionMemoryEntry)>,
}

impl ActionMemory {
    /// Opens a bounded, cross-session action-memory file. Malformed or oversized
    /// state is rejected instead of silently treated as trusted memory.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let mut memory = Self {
            entries: BTreeMap::new(),
            path: Some(path.clone()),
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 512 * 1024 {
                    return Err("action memory exceeds its byte budget".into());
                }
                let stored: StoredActionMemory =
                    serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                if stored.version != 1
                    || stored.entries.len() > MAX_ACTION_MEMORY_ENTRIES
                    || stored
                        .entries
                        .iter()
                        .any(|(key, entry)| !valid_key(key) || !valid_entry(entry))
                {
                    return Err("action memory failed schema or bounds validation".into());
                }
                memory.entries.extend(stored.entries);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        Ok(memory)
    }

    /// Store only independently passed evidence. A failed run invalidates the exact hint.
    pub fn record(
        &mut self,
        key: ActionMemoryKey,
        entry: ActionMemoryEntry,
        verification: Verification,
    ) -> bool {
        if verification != Verification::Passed {
            self.entries.remove(&key);
            let _ = self.persist();
            return false;
        }
        if !valid_key(&key)
            || entry.action.is_empty()
            || entry.action.len() > 64
            || entry.semantic_target.is_null()
            || entry.expected_postcondition.is_null()
            || !valid_entry(&entry)
        {
            return false;
        }
        if !self.entries.contains_key(&key)
            && self.entries.len() >= MAX_ACTION_MEMORY_ENTRIES
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_verified_ms)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(key, entry);
        self.persist().is_ok()
    }

    /// Records one independently verified execution of a bounded operation.
    /// The first pass is a candidate; the second distinct verification promotes
    /// it. A failure or drift invalidates the exact operation instead of leaving
    /// a partially trusted replay behind.
    pub fn record_operation(
        &mut self,
        key: ActionMemoryKey,
        mut operation: OperationPlan,
        verification: Verification,
    ) -> bool {
        if verification != Verification::Passed {
            self.entries.remove(&key);
            let _ = self.persist();
            return false;
        }
        if !valid_key(&key)
            || operation.steps.is_empty()
            || operation.steps.len() > MAX_OPERATION_STEPS
            || !valid_operation(&operation)
        {
            return false;
        }
        operation.promoted = false;
        operation.successes = operation.successes.clamp(1, u8::MAX - 1);
        if let Some(previous) = self.entries.get(&key)
            && let Some(previous_operation) = previous.operation.as_ref()
            && operation_digest(previous_operation) == operation_digest(&operation)
        {
            operation.successes = previous_operation.successes.saturating_add(1);
        }
        operation.promoted = operation.successes >= MIN_OPERATION_PROMOTIONS;
        let entry = ActionMemoryEntry {
            semantic_target: serde_json::json!({
                "kind": "operation",
                "step_count": operation.steps.len(),
                "effect": operation.effect,
            }),
            action: "operation".into(),
            expected_postcondition: serde_json::json!({
                "kind": "all_operation_steps_verified",
                "successes": operation.successes,
                "promoted": operation.promoted,
            }),
            replay: None,
            operation: Some(operation),
            preconditions: None,
            repair: None,
            last_verified_ms: now_unix_ms(),
        };
        if !valid_entry(&entry) {
            return false;
        }
        if !self.entries.contains_key(&key)
            && self.entries.len() >= MAX_ACTION_MEMORY_ENTRIES
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_verified_ms)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(key, entry);
        self.persist().is_ok()
    }

    /// Returns a promoted operation only when it is fresh. An unpromoted
    /// candidate is deliberately unavailable to execution.
    pub fn lookup_operation(
        &self,
        key: &ActionMemoryKey,
        now_ms: u64,
    ) -> Option<&ActionMemoryEntry> {
        self.lookup(key, now_ms)
            .filter(|entry| entry.operation.as_ref().is_some_and(|plan| plan.promoted))
    }

    /// Fast prefilter for the operation-memory path.
    pub fn has_promoted_operation(&self, origin: &str, intent: &str, now_ms: u64) -> bool {
        self.entries.iter().any(|(key, entry)| {
            key.origin == origin
                && key.intent == intent
                && entry.operation.as_ref().is_some_and(|plan| plan.promoted)
                && entry.last_verified_ms <= now_ms
                && now_ms.saturating_sub(entry.last_verified_ms) <= MAX_AGE_MS
        })
    }

    /// Returns a bounded grounding hint only; callers must still revalidate the live target.
    pub fn lookup(&self, key: &ActionMemoryKey, now_ms: u64) -> Option<&ActionMemoryEntry> {
        self.entries.get(key).filter(|entry| {
            entry.last_verified_ms <= now_ms
                && now_ms.saturating_sub(entry.last_verified_ms) <= MAX_AGE_MS
        })
    }

    pub fn replay_status(&self, key: &ActionMemoryKey, now_ms: u64) -> ReplayStatus {
        let Some(entry) = self.entries.get(key) else {
            return ReplayStatus::Miss;
        };
        if entry.replay.is_none()
            && !entry
                .operation
                .as_ref()
                .is_some_and(|operation| operation.promoted)
        {
            return ReplayStatus::Disabled;
        }
        if entry.last_verified_ms > now_ms
            || now_ms.saturating_sub(entry.last_verified_ms) > MAX_AGE_MS
        {
            return ReplayStatus::Stale;
        }
        ReplayStatus::Hit
    }

    /// Repair a cached operation target after a reactive fallback succeeded and
    /// the postcondition was freshly verified. The previous target is retained
    /// only as bounded structural identity so later repairs stay explainable.
    pub fn repair_target(
        &mut self,
        key: &ActionMemoryKey,
        target: Value,
        previous_target: Option<TargetTemplate>,
        reason: &str,
        now_ms: u64,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if target.is_null() || contains_secret_marker(&target) || !valid_plan_string(reason, 512) {
            return false;
        }
        entry.semantic_target = target;
        entry.repair = Some(RepairMetadata {
            repaired_at_ms: now_ms,
            reason: reason.to_owned(),
            previous_target,
        });
        entry.last_verified_ms = now_ms;
        if !valid_entry(entry) {
            return false;
        }
        self.persist().is_ok()
    }

    /// Removes one exact hint after structural drift or an unverified result.
    pub fn invalidate(&mut self, key: &ActionMemoryKey) -> bool {
        let removed = self.entries.remove(key).is_some();
        if removed {
            let _ = self.persist();
        }
        removed
    }

    /// Fast prefilter for the common miss path. Callers can avoid a live
    /// structural snapshot unless a fresh entry could match this origin/intent.
    pub fn has_fresh_origin_intent(&self, origin: &str, intent: &str, now_ms: u64) -> bool {
        self.entries.iter().any(|(key, entry)| {
            key.origin == origin
                && key.intent == intent
                && entry.last_verified_ms <= now_ms
                && now_ms.saturating_sub(entry.last_verified_ms) <= MAX_AGE_MS
        })
    }

    /// Returns only fresh hints for an exact page identity. Hints remain advisory;
    /// execution must resolve a new live semantic target before dispatch.
    pub fn lookup_page(
        &self,
        origin: &str,
        structural_signature: &str,
        now_ms: u64,
        limit: usize,
    ) -> Vec<(ActionMemoryKey, ActionMemoryEntry)> {
        let mut entries = self
            .entries
            .iter()
            .filter(|(key, entry)| {
                key.origin == origin
                    && key.structural_signature == structural_signature
                    && entry.last_verified_ms <= now_ms
                    && now_ms.saturating_sub(entry.last_verified_ms) <= MAX_AGE_MS
            })
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.last_verified_ms));
        entries.truncate(limit.min(16));
        entries
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let stored = StoredActionMemory {
            version: 1,
            entries: self
                .entries
                .iter()
                .map(|(key, entry)| (key.clone(), entry.clone()))
                .collect(),
        };
        let bytes = serde_json::to_vec(&stored).map_err(|error| error.to_string())?;
        if bytes.len() > 512 * 1024 {
            return Err("action memory reached its byte budget".into());
        }
        let parent = path.parent().ok_or("action memory path has no parent")?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp = path.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&temp).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        std::fs::rename(&temp, path).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            error.to_string()
        })
    }
}

fn valid_entry(entry: &ActionMemoryEntry) -> bool {
    let preconditions_valid = entry.preconditions.as_ref().is_none_or(|value| {
        !value.is_null()
            && !contains_secret_marker(value)
            && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 1024)
    });
    let repair_valid = entry.repair.as_ref().is_none_or(|repair| {
        !repair.reason.is_empty()
            && repair.reason.len() <= 512
            && !contains_secret_marker(&Value::String(repair.reason.clone()))
            && repair
                .previous_target
                .as_ref()
                .is_none_or(valid_target_template)
    });
    !entry.action.is_empty()
        && entry.action.len() <= 64
        && !entry.semantic_target.is_null()
        && !entry.expected_postcondition.is_null()
        && !contains_secret_marker(&entry.semantic_target)
        && !contains_secret_marker(&entry.expected_postcondition)
        && preconditions_valid
        && repair_valid
        && entry.replay.as_ref().is_none_or(valid_replay_plan)
        && entry.operation.as_ref().is_none_or(valid_operation)
        && serde_json::to_vec(entry).is_ok_and(|bytes| bytes.len() <= 8192)
}

fn valid_operation(operation: &OperationPlan) -> bool {
    !operation.steps.is_empty()
        && operation.steps.len() <= MAX_OPERATION_STEPS
        && operation.steps.iter().all(|step| {
            OPERATION_ACTIONS.contains(&step.action.as_str())
                && step.target.as_ref().is_none_or(valid_target_template)
                && step.postcondition.as_ref().is_none_or(valid_predicate)
                && step
                    .value_slot
                    .as_ref()
                    .is_none_or(|slot| valid_value_slot(slot))
        })
        && serde_json::to_vec(operation).is_ok_and(|bytes| bytes.len() <= 4096)
}

fn operation_digest(operation: &OperationPlan) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(serde_json::to_vec(operation).unwrap_or_default()).into()
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn valid_plan_string(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && !value.contains('\0')
        && !contains_secret_marker(&Value::String(value.to_owned()))
}

fn valid_value_slot(slot: &str) -> bool {
    !slot.is_empty()
        && slot.len() <= 64
        && slot
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn valid_target_template(target: &TargetTemplate) -> bool {
    let bounded = |value: &Option<String>, max: usize| {
        value
            .as_ref()
            .is_none_or(|value| valid_plan_string(value, max))
    };
    bounded(&target.role, 64)
        && bounded(&target.name, 512)
        && bounded(&target.href, 2048)
        && bounded(&target.ancestor, 512)
        && bounded(&target.test_id, 128)
}

fn valid_predicate(predicate: &VerifyPredicate) -> bool {
    match predicate {
        VerifyPredicate::UrlMatches { pattern } => valid_plan_string(pattern, 2048),
        VerifyPredicate::TextPresent { text } => valid_plan_string(text, 1024),
        VerifyPredicate::Visible { target } | VerifyPredicate::Hidden { target } => {
            valid_target_template(target)
        }
        VerifyPredicate::AppState { adapter } => valid_plan_string(adapter, 128),
    }
}

fn valid_replay_plan(plan: &ReplayPlan) -> bool {
    match plan {
        ReplayPlan::Extract { selector } => valid_plan_string(selector, 512),
        ReplayPlan::Navigate { url_template } => valid_plan_string(url_template, 4096),
        ReplayPlan::Click { target, outcome } => {
            valid_target_template(target) && valid_predicate(outcome)
        }
        ReplayPlan::ClickHint { target } => valid_target_template(target),
        ReplayPlan::FillHint { target, value_slot }
        | ReplayPlan::SelectHint { target, value_slot } => {
            valid_target_template(target) && valid_value_slot(value_slot)
        }
        ReplayPlan::Fill {
            target,
            value_slot,
            outcome,
        }
        | ReplayPlan::Select {
            target,
            value_slot,
            outcome,
        } => {
            valid_target_template(target)
                && valid_value_slot(value_slot)
                && valid_predicate(outcome)
        }
        ReplayPlan::Type {
            target,
            value_slot,
            strategy_hint,
        } => {
            valid_target_template(target)
                && valid_value_slot(value_slot)
                && strategy_hint.as_ref().is_none_or(|hint| {
                    matches!(
                        hint.as_str(),
                        "block" | "fast_keys" | "human_keys" | "ime" | "auto"
                    )
                })
        }
        ReplayPlan::Tool {
            capability_id,
            schema_hash,
        } => valid_plan_string(capability_id, 128) && valid_plan_string(schema_hash, 128),
    }
}

fn contains_secret_marker(value: &Value) -> bool {
    fn visit(value: &Value) -> bool {
        match value {
            Value::String(text) => {
                let lower = text.to_ascii_lowercase();
                [
                    "password",
                    "passwd",
                    "secret",
                    "credential",
                    "api_key",
                    "access_token",
                ]
                .iter()
                .any(|marker| lower.contains(marker))
            }
            Value::Object(object) => object.iter().any(|(key, child)| {
                let lower = key.to_ascii_lowercase();
                [
                    "password",
                    "passwd",
                    "secret",
                    "credential",
                    "raw_value",
                    "access_token",
                ]
                .iter()
                .any(|marker| lower.contains(marker))
                    || visit(child)
            }),
            Value::Array(items) => items.iter().any(visit),
            _ => false,
        }
    }
    visit(value)
}

fn valid_key(key: &ActionMemoryKey) -> bool {
    !key.origin.is_empty()
        && key.origin.len() <= 512
        && !key.structural_signature.is_empty()
        && key.structural_signature.len() <= 256
        && !key.intent.is_empty()
        && key.intent.len() <= 256
}
