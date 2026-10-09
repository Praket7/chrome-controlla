use crate::verifier::Verification;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStrength {
    FreshPageState,
    ReloadedState,
    Visual,
    Artifact,
    ExternalState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RuntimeObservation {
    pub binding_revision: String,
    pub observer_id: String,
    pub observed_at_ms: u64,
    pub strength: VerificationStrength,
    pub state: Value,
}

pub trait RuntimeObserver {
    fn observe(&self, binding_revision: &str) -> Result<RuntimeObservation, String>;
}

pub struct FreshStateObserver<F> {
    observer_id: String,
    strength: VerificationStrength,
    sampler: F,
}

impl<F> FreshStateObserver<F> {
    pub fn new(observer_id: impl Into<String>, strength: VerificationStrength, sampler: F) -> Self {
        Self {
            observer_id: observer_id.into(),
            strength,
            sampler,
        }
    }
}

impl<F> RuntimeObserver for FreshStateObserver<F>
where
    F: Fn(&str) -> Result<(u64, Value), String>,
{
    fn observe(&self, binding_revision: &str) -> Result<RuntimeObservation, String> {
        if self.observer_id.trim().is_empty() || binding_revision.trim().is_empty() {
            return Err("observer id and binding revision must be non-empty".into());
        }
        let (observed_at_ms, state) = (self.sampler)(binding_revision)?;
        Ok(RuntimeObservation {
            binding_revision: binding_revision.to_owned(),
            observer_id: self.observer_id.clone(),
            observed_at_ms,
            strength: self.strength,
            state,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerificationPolicy {
    pub max_age_ms: u64,
    pub minimum_strength: VerificationStrength,
}

pub fn validate_observation(
    policy: VerificationPolicy,
    expected_revision: &str,
    now_ms: u64,
    observation: &RuntimeObservation,
) -> Verification {
    if expected_revision.is_empty()
        || observation.binding_revision != expected_revision
        || observation.observer_id.trim().is_empty()
        || observation.observed_at_ms > now_ms
        || now_ms.saturating_sub(observation.observed_at_ms) > policy.max_age_ms
        || observation.strength < policy.minimum_strength
    {
        Verification::Inconclusive
    } else {
        Verification::Passed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_page_state(_: &str) -> Result<(u64, Value), String> {
        Ok((100, json!({"saved":true})))
    }

    #[test]
    fn fresh_runtime_owned_observation_passes_binding_policy() {
        let observer = FreshStateObserver::new(
            "shared-page-runtime",
            VerificationStrength::FreshPageState,
            sample_page_state,
        );
        let observation = observer.observe("r7").unwrap();
        assert_eq!(
            validate_observation(
                VerificationPolicy {
                    max_age_ms: 50,
                    minimum_strength: VerificationStrength::FreshPageState,
                },
                "r7",
                120,
                &observation,
            ),
            Verification::Passed
        );
    }

    #[test]
    fn stale_or_wrong_revision_observation_is_inconclusive() {
        let observation = RuntimeObservation {
            binding_revision: "r6".into(),
            observer_id: "runtime".into(),
            observed_at_ms: 10,
            strength: VerificationStrength::ExternalState,
            state: json!({}),
        };
        let policy = VerificationPolicy {
            max_age_ms: 5,
            minimum_strength: VerificationStrength::FreshPageState,
        };
        assert_eq!(
            validate_observation(policy, "r7", 20, &observation),
            Verification::Inconclusive
        );
    }
}
