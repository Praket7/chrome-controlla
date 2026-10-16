//! Conservative offline proposals from multiple verified and safely failed traces.

use crate::skills::{SKILL_SCHEMA_VERSION, SkillDefinition};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const MIN_SUCCESS_TRACES: usize = 3;
const MAX_TRACES: usize = 128;
const MAX_STEPS: usize = 64;
const MAX_TRACE_BYTES: usize = 2 * 1024 * 1024;
const MAX_STORED_TRACES: usize = 512;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceStep {
    pub fingerprint: String,
    pub action: Value,
    /// Explicit dependency/input provenance from the typed task contract.
    /// Empty provenance is retained for old journals but never induces replay.
    #[serde(default)]
    pub dataflow_evidence: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    NotDispatched,
    ReadFailure,
    UnknownMutation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkillTrace {
    pub run_id: String,
    pub site_scope: String,
    pub intent_fingerprint: String,
    pub structural_signature: String,
    pub input_fingerprint: String,
    /// Fingerprint of an explicit, previously verified account marker. Never
    /// store the marker selector or visible account text itself.
    #[serde(default)]
    pub account_identity_fingerprint: Option<String>,
    pub verified: bool,
    pub preconditions: BTreeSet<String>,
    pub postconditions: BTreeSet<String>,
    pub steps: Vec<TraceStep>,
    pub failure: Option<(FailureClass, String)>,
}

impl SkillTrace {
    pub fn failure(
        run_id: impl Into<String>,
        site_scope: impl Into<String>,
        intent_fingerprint: impl Into<String>,
        structural_signature: impl Into<String>,
        class: FailureClass,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            site_scope: site_scope.into(),
            intent_fingerprint: intent_fingerprint.into(),
            structural_signature: structural_signature.into(),
            input_fingerprint: String::new(),
            account_identity_fingerprint: None,
            verified: false,
            preconditions: BTreeSet::new(),
            postconditions: BTreeSet::new(),
            steps: Vec::new(),
            failure: Some((class, reason.into())),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SkillEvidenceSet {
    pub traces: Vec<SkillTrace>,
}

/// Bounded, local-only journal of structured, already-redacted task traces.
/// It is intentionally separate from browser contents and secret-bearing task values.
pub struct SkillTraceStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl SkillTraceStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let store = Self {
            path: path.into(),
            lock: Mutex::new(()),
        };
        store.read()?;
        Ok(store)
    }

    pub fn record(&self, trace: SkillTrace) -> Result<(), String> {
        if !valid_trace(&trace) {
            return Err("trace failed schema or privacy bounds".into());
        }
        let _guard = self.lock.lock().map_err(|_| "trace store lock poisoned")?;
        let mut traces = self.read()?;
        if traces.iter().any(|stored| stored.run_id == trace.run_id) {
            return Err("trace run_id already exists".into());
        }
        traces.push(trace);
        if traces.len() > MAX_STORED_TRACES {
            traces.drain(..traces.len() - MAX_STORED_TRACES);
        }
        let bytes = serde_json::to_vec(&traces).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_TRACE_BYTES {
            return Err("trace journal reached its byte budget".into());
        }
        let parent = self.path.parent().ok_or("trace path has no parent")?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp = self
            .path
            .with_extension(format!("tmp-{}-{nonce}", std::process::id()));
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
        std::fs::rename(&temp, &self.path).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            error.to_string()
        })?;
        Ok(())
    }

    pub fn evidence(
        &self,
        site_scope: &str,
        intent_fingerprint: &str,
        account_identity_fingerprint: Option<&str>,
    ) -> Result<SkillEvidenceSet, String> {
        let _guard = self.lock.lock().map_err(|_| "trace store lock poisoned")?;
        let traces = self.read()?;
        Ok(SkillEvidenceSet {
            traces: traces
                .into_iter()
                .filter(|trace| {
                    trace.site_scope == site_scope
                        && trace.intent_fingerprint == intent_fingerprint
                        && trace.account_identity_fingerprint.as_deref()
                            == account_identity_fingerprint
                })
                .rev()
                .take(MAX_TRACES)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
        })
    }

    fn read(&self) -> Result<Vec<SkillTrace>, String> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        if bytes.len() > MAX_TRACE_BYTES {
            return Err("trace journal exceeds its byte budget".into());
        }
        let traces: Vec<SkillTrace> =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if traces.len() > MAX_STORED_TRACES || traces.iter().any(|trace| !valid_trace(trace)) {
            return Err("trace journal failed schema or bounds validation".into());
        }
        Ok(traces)
    }
}

fn valid_trace(trace: &SkillTrace) -> bool {
    !trace.run_id.is_empty()
        && trace.run_id.len() <= 128
        && !trace.site_scope.is_empty()
        && trace.site_scope.len() <= 512
        && !trace.intent_fingerprint.is_empty()
        && trace.intent_fingerprint.len() <= 256
        && !trace.structural_signature.is_empty()
        && trace.structural_signature.len() <= 256
        && trace.input_fingerprint.len() <= 256
        && trace
            .account_identity_fingerprint
            .as_deref()
            .is_none_or(crate::skills::valid_account_identity_fingerprint)
        && trace.steps.len() <= MAX_STEPS
        && trace.steps.iter().all(|step| {
            !step.fingerprint.is_empty()
                && step.fingerprint.len() <= 256
                && step.dataflow_evidence.len() <= MAX_STEPS
                && step
                    .dataflow_evidence
                    .iter()
                    .all(|item| !item.is_empty() && item.len() <= 256)
                && serde_json::to_vec(&step.action).is_ok_and(|bytes| bytes.len() <= 4096)
        })
        && trace
            .preconditions
            .iter()
            .all(|item| !item.is_empty() && item.len() <= 256)
        && trace
            .postconditions
            .iter()
            .all(|item| !item.is_empty() && item.len() <= 256)
        && trace
            .failure
            .as_ref()
            .is_none_or(|(_, reason)| !reason.is_empty() && reason.len() <= 256)
}

/// Produces a bounded, domain-separated cryptographic fingerprint from an
/// explicit visible account marker. Raw marker values are never returned.
pub fn account_identity_fingerprint(key: &[u8], selector: &str, expected: &str) -> Option<String> {
    if key.len() != 32
        || selector.trim().is_empty()
        || selector.len() > 512
        || selector.chars().any(char::is_control)
        || expected.trim().is_empty()
        || expected.len() > 512
        || expected.chars().any(char::is_control)
    {
        return None;
    }
    use sha2::Digest as _;
    let mut inner_pad = [0x36u8; 64];
    let mut outer_pad = [0x5cu8; 64];
    for (index, byte) in key.iter().copied().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let mut inner = sha2::Sha256::new();
    inner.update(inner_pad);
    inner.update(b"controlla-account-identity-v1\0");
    inner.update((selector.len() as u64).to_be_bytes());
    inner.update(selector.as_bytes());
    inner.update((expected.len() as u64).to_be_bytes());
    inner.update(expected.as_bytes());
    let inner_digest = inner.finalize();
    let mut digest = sha2::Sha256::new();
    digest.update(outer_pad);
    digest.update(inner_digest);
    Some(format!(
        "hmac-sha256:{}",
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkillCandidate {
    pub definition: SkillDefinition,
    pub failure_guards: Vec<String>,
}

/// Creates a candidate only from three varied verified successes and at least
/// one known safe failure. Unknown mutation traces always veto induction.
pub fn induce_candidate(evidence: &SkillEvidenceSet) -> Option<SkillCandidate> {
    if evidence.traces.is_empty() || evidence.traces.len() > MAX_TRACES {
        return None;
    }
    let first = &evidence.traces[0];
    let mut run_ids = BTreeSet::new();
    if evidence.traces.iter().any(|trace| {
        trace.run_id.is_empty()
            || trace.run_id.len() > 128
            || !run_ids.insert(trace.run_id.as_str())
            || trace.site_scope != first.site_scope
            || trace.intent_fingerprint != first.intent_fingerprint
            || trace.account_identity_fingerprint != first.account_identity_fingerprint
            || trace.site_scope.is_empty()
            || trace.site_scope.len() > 512
            || trace.intent_fingerprint.is_empty()
            || trace.intent_fingerprint.len() > 256
            || trace.structural_signature.is_empty()
            || trace.structural_signature.len() > 256
            || trace
                .failure
                .as_ref()
                .is_some_and(|(_, reason)| reason.is_empty() || reason.len() > 256)
    }) {
        return None;
    }
    if evidence.traces.iter().any(|trace| {
        trace
            .failure
            .as_ref()
            .is_some_and(|(class, _)| *class == FailureClass::UnknownMutation)
    }) {
        return None;
    }
    let successes = evidence
        .traces
        .iter()
        .filter(|trace| trace.failure.is_none() && trace.verified)
        .collect::<Vec<_>>();
    let failures = evidence
        .traces
        .iter()
        .filter_map(|trace| {
            trace.failure.as_ref().map(|(class, reason)| {
                let class = match class {
                    FailureClass::NotDispatched => "not_dispatched",
                    FailureClass::ReadFailure => "read_failure",
                    FailureClass::UnknownMutation => "unknown_mutation",
                };
                format!("{class}:{reason}")
            })
        })
        .collect::<BTreeSet<_>>();
    if successes.len() < MIN_SUCCESS_TRACES
        || failures.is_empty()
        || successes.iter().any(|trace| {
            trace.steps.is_empty()
                || trace.steps.len() > MAX_STEPS
                || trace
                    .steps
                    .iter()
                    .any(|step| step.dataflow_evidence.is_empty())
        })
    {
        return None;
    }
    let first = successes[0];
    if successes.iter().any(|trace| {
        trace.site_scope != first.site_scope || trace.intent_fingerprint != first.intent_fingerprint
    }) || successes
        .iter()
        .map(|trace| trace.run_id.as_str())
        .collect::<BTreeSet<_>>()
        .len()
        != successes.len()
        || successes
            .iter()
            .map(|trace| trace.input_fingerprint.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            < MIN_SUCCESS_TRACES
    {
        return None;
    }

    let common_steps = ordered_common_steps(first, &successes[1..]);
    if common_steps.is_empty() {
        return None;
    }
    let preconditions = common_set(&successes, |trace| &trace.preconditions);
    let postconditions = common_set(&successes, |trace| &trace.postconditions);
    if preconditions.is_empty() || postconditions.is_empty() {
        return None;
    }

    let signatures = successes
        .iter()
        .map(|trace| trace.structural_signature.clone())
        .collect::<BTreeSet<_>>();
    if signatures.is_empty()
        || signatures
            .iter()
            .any(|signature| signature.is_empty() || signature.len() > 256)
    {
        return None;
    }
    // Failure guards are only useful when observed against a structure that
    // this candidate has also successfully handled. Otherwise an unrelated
    // revision of the same site/intent could poison replay policy.
    if evidence
        .traces
        .iter()
        .filter(|trace| trace.failure.is_some())
        .any(|trace| !signatures.contains(&trace.structural_signature))
    {
        return None;
    }
    // Runtime traces may induce only templates the V3 replay paths understand.
    // Generic receipt summaries are useful telemetry, but are not executable skills.
    if common_steps.len() != 1 {
        return None;
    }
    let workflow = common_steps[0].action.clone();
    let parameter_names = replay_template_parameters(&workflow)?;
    let key = format!(
        "{}\u{1f}{}\u{1f}{}",
        first.site_scope,
        first.intent_fingerprint,
        first
            .account_identity_fingerprint
            .as_deref()
            .unwrap_or("unscoped")
    );
    let digest = sha2::Sha256::digest(key.as_bytes());
    let id = format!(
        "induced-{}",
        digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let failure_guards = failures.into_iter().collect::<Vec<_>>();
    let definition = SkillDefinition {
        skill_id: id,
        schema_version: SKILL_SCHEMA_VERSION,
        site_scope: first.site_scope.clone(),
        intent_fingerprint: first.intent_fingerprint.clone(),
        account_identity_fingerprint: first.account_identity_fingerprint.clone(),
        parameter_names: parameter_names.into_iter().collect(),
        preconditions: preconditions.into_iter().collect(),
        workflow,
        postconditions: postconditions.into_iter().collect(),
        failure_guards: failure_guards.clone(),
        structural_signatures: signatures.into_iter().collect(),
        version: 1,
    };
    definition.valid().then_some(SkillCandidate {
        definition,
        failure_guards,
    })
}

fn ordered_common_steps<'a>(first: &'a SkillTrace, rest: &[&SkillTrace]) -> Vec<&'a TraceStep> {
    let mut cursors = vec![0usize; rest.len()];
    let mut common = Vec::new();
    for step in &first.steps {
        let mut positions = Vec::with_capacity(rest.len());
        for (index, trace) in rest.iter().enumerate() {
            let Some(position) = trace.steps[cursors[index]..].iter().position(|candidate| {
                candidate.fingerprint == step.fingerprint
                    && candidate.action == step.action
                    && candidate.dataflow_evidence == step.dataflow_evidence
            }) else {
                break;
            };
            positions.push(cursors[index] + position);
        }
        if positions.len() == rest.len() {
            for (cursor, position) in cursors.iter_mut().zip(positions) {
                *cursor = position + 1;
            }
            common.push(step);
        }
    }
    common
}

fn common_set<'a>(
    traces: &[&'a SkillTrace],
    select: impl Fn(&'a SkillTrace) -> &'a BTreeSet<String>,
) -> BTreeSet<String> {
    let mut common = select(traces[0]).clone();
    for trace in &traces[1..] {
        common.retain(|value| select(trace).contains(value));
    }
    common
}

fn replay_template_parameters(workflow: &Value) -> Option<Vec<String>> {
    let kind = workflow.get("kind")?.as_str()?;
    let slots = match kind {
        "form_fill" | "form_fill_save" => {
            let fields = workflow.get("fields")?.as_array()?;
            if fields.is_empty()
                || fields.len() > 16
                || workflow.as_object()?.len() != if kind == "form_fill" { 2 } else { 3 }
            {
                return None;
            }
            if fields.iter().enumerate().any(|(index, field)| {
                field.as_object().is_none_or(|object| object.len() != 1)
                    || field.get("slot").and_then(Value::as_u64) != Some(index as u64)
            }) {
                return None;
            }
            if kind == "form_fill_save" {
                let confirmation = workflow.get("confirmation")?.as_object()?;
                if confirmation.len() != 2
                    || confirmation.get("selector")?.as_str()?.is_empty()
                    || confirmation.get("selector")?.as_str()?.len() > 512
                    || confirmation.get("text")?.as_str()?.is_empty()
                    || confirmation.get("text")?.as_str()?.len() > 512
                {
                    return None;
                }
            }
            fields.len()
        }
        "mutation" => {
            if workflow.as_object()?.len() != 2 {
                return None;
            }
            let steps = workflow.get("steps")?.as_array()?;
            if steps.is_empty() || steps.len() > 32 {
                return None;
            }
            let mut fill_count = 0usize;
            for (index, step) in steps.iter().enumerate() {
                let action = step.get("action")?.as_str()?;
                if step.get("step")?.as_u64() != Some(index as u64) {
                    return None;
                }
                match action {
                    "fill" => {
                        if step.as_object()?.len() != 3
                            || step.get("value_slot")?.as_u64() != Some(fill_count as u64)
                        {
                            return None;
                        }
                        fill_count += 1;
                    }
                    "click" => {
                        let outcome = step.get("outcome")?.as_object()?;
                        let outcome_kind = outcome.get("kind")?.as_str()?;
                        if step.as_object()?.len() != 3
                            || outcome.len() > 3
                            || !matches!(
                                outcome_kind,
                                "navigation"
                                    | "new_tab"
                                    | "focused"
                                    | "visible"
                                    | "text"
                                    | "expanded"
                                    | "selected"
                            )
                            || outcome
                                .values()
                                .any(|value| value.as_str().is_some_and(|text| text.len() > 512))
                        {
                            return None;
                        }
                    }
                    _ => return None,
                }
            }
            fill_count
        }
        "repeat_click" => {
            if workflow.as_object()?.len() != 2
                || workflow
                    .get("count")?
                    .as_u64()
                    .is_none_or(|count| !(1..=64).contains(&count))
            {
                return None;
            }
            0
        }
        _ => return None,
    };
    Some((0..slots).map(|index| format!("value_{index}")).collect())
}

use sha2::Digest;

#[cfg(test)]
mod account_scope_tests {
    use super::*;
    use serde_json::json;

    const KEY: [u8; 32] = [7; 32];

    fn trace(run_id: &str, input: &str, identity: Option<&str>, failure: bool) -> SkillTrace {
        SkillTrace {
            run_id: run_id.into(),
            site_scope: "https://example.test".into(),
            intent_fingerprint: "registration".into(),
            structural_signature: "form-v1".into(),
            input_fingerprint: input.into(),
            account_identity_fingerprint: identity.map(str::to_owned),
            verified: !failure,
            preconditions: BTreeSet::from(["form-present".into()]),
            postconditions: BTreeSet::from(["form-saved".into()]),
            steps: vec![TraceStep {
                fingerprint: "verified-template:form-fill".into(),
                action: json!({"kind":"form_fill","fields":[{"slot":0}]}),
                dataflow_evidence: BTreeSet::from(["input_binding:field_slot:0".into()]),
            }],
            failure: failure.then(|| (FailureClass::NotDispatched, "target_not_ready".into())),
        }
    }

    #[test]
    fn account_markers_are_fingerprinted_and_induction_separates_identities() {
        let first =
            account_identity_fingerprint(&KEY, "#account", "person-a@example.test").unwrap();
        let second =
            account_identity_fingerprint(&KEY, "#account", "person-b@example.test").unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with("hmac-sha256:") && first.len() == 76);

        let evidence_for = |identity: &str| SkillEvidenceSet {
            traces: vec![
                trace("a1", "input-a", Some(identity), false),
                trace("a2", "input-b", Some(identity), false),
                trace("a3", "input-c", Some(identity), false),
                trace("a4", "input-d", Some(identity), true),
            ],
        };
        let candidate_a = induce_candidate(&evidence_for(&first)).unwrap();
        let candidate_b = induce_candidate(&evidence_for(&second)).unwrap();
        assert_ne!(
            candidate_a.definition.skill_id,
            candidate_b.definition.skill_id
        );
        assert_eq!(
            candidate_a
                .definition
                .account_identity_fingerprint
                .as_deref(),
            Some(first.as_str())
        );

        let mut mixed = evidence_for(&first);
        mixed.traces[3].account_identity_fingerprint = Some(second);
        assert!(induce_candidate(&mixed).is_none());
    }

    #[test]
    fn persisted_trace_contains_only_the_bounded_account_fingerprint() {
        let marker_text = "person-a@example.test";
        let fingerprint = account_identity_fingerprint(&KEY, "#account", marker_text).unwrap();
        let path = std::env::temp_dir().join(format!(
            "controlla-account-traces-{}-{}.json",
            std::process::id(),
            fingerprint[7..15].replace(':', "")
        ));
        let store = SkillTraceStore::open(&path).unwrap();
        let mut sample = trace("a1", "input-a", Some(&fingerprint), false);
        sample.steps[0].action = json!({"kind":"form_fill","fields":[{"slot":0}]});
        store.record(sample).unwrap();
        let persisted = std::fs::read_to_string(&path).unwrap();
        assert!(persisted.contains(&fingerprint));
        assert!(!persisted.contains(marker_text));
        let _ = std::fs::remove_file(path);
    }
}
