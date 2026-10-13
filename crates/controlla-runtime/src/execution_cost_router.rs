//! Bounded, verified route latency history for safe empirical tier selection.
//! Only exact origin/intent/warmth/task-size cohorts can influence a route.
use crate::capability_router::ExecutionTier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_PROFILES: usize = 512;
const MAX_SAMPLES_PER_PROFILE: usize = 32;
const MIN_CLEAN_SAMPLES: usize = 8;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCostRouter {
    profiles: BTreeMap<String, Vec<LatencySample>>,
    #[serde(skip)]
    path: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LatencySample {
    elapsed_ms: u64,
    verified: bool,
    safety_clean: bool,
}

#[derive(Clone, Debug)]
pub struct RouteCostContext<'a> {
    pub origin: &'a str,
    pub intent: &'a str,
    pub warmth: &'a str,
    pub task_size: &'a str,
}

impl ExecutionCostRouter {
    pub fn open(path: impl Into<std::path::PathBuf>) -> Self {
        let path = path.into();
        let mut store = match std::fs::read(&path) {
            Ok(bytes) if bytes.len() <= 1024 * 1024 => {
                serde_json::from_slice(&bytes).unwrap_or_default()
            }
            _ => Self::default(),
        };
        store.path = Some(path);
        store
    }

    pub fn record(
        &mut self,
        context: &RouteCostContext<'_>,
        tier: ExecutionTier,
        elapsed_ms: u64,
        verified: bool,
        safety_clean: bool,
    ) -> Result<(), &'static str> {
        if context.origin.is_empty()
            || context.origin.len() > 512
            || context.intent.is_empty()
            || context.intent.len() > 128
            || !matches!(context.warmth, "cold" | "warm" | "unknown")
            || !matches!(context.task_size, "small" | "medium" | "large")
            || elapsed_ms == 0
        {
            return Err("invalid route cost sample");
        }
        let key = profile_key(context, tier);
        if !self.profiles.contains_key(&key)
            && self.profiles.len() >= MAX_PROFILES
            && let Some(oldest) = self.profiles.keys().next().cloned()
        {
            self.profiles.remove(&oldest);
        }
        let samples = self.profiles.entry(key).or_default();
        samples.push(LatencySample {
            elapsed_ms,
            verified,
            safety_clean,
        });
        if samples.len() > MAX_SAMPLES_PER_PROFILE {
            samples.remove(0);
        }
        self.persist()
    }

    /// A route may override the static tier only after eight consecutive
    /// independently verified, safety-clean observations for that exact cohort.
    /// Unmeasured routes keep the conservative static ordering.
    pub fn select(
        &self,
        context: &RouteCostContext<'_>,
        available: &[ExecutionTier],
    ) -> Option<ExecutionTier> {
        available
            .iter()
            .filter_map(|tier| {
                let samples = self.profiles.get(&profile_key(context, *tier))?;
                if samples.len() < MIN_CLEAN_SAMPLES {
                    return None;
                }
                let recent = &samples[samples.len() - MIN_CLEAN_SAMPLES..];
                if recent
                    .iter()
                    .any(|sample| !sample.verified || !sample.safety_clean)
                {
                    return None;
                }
                let mut latencies = recent
                    .iter()
                    .map(|sample| sample.elapsed_ms)
                    .collect::<Vec<_>>();
                latencies.sort_unstable();
                Some((*tier, latencies[latencies.len() - 1]))
            })
            .min_by_key(|(_, p95)| *p95)
            .map(|(tier, _)| tier)
    }

    fn persist(&self) -> Result<(), &'static str> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = path.parent().ok_or("cost router path has no parent")?;
        std::fs::create_dir_all(parent).map_err(|_| "cannot create cost router directory")?;
        let bytes = serde_json::to_vec(self).map_err(|_| "cannot serialize cost router")?;
        if bytes.len() > 1024 * 1024 {
            return Err("cost router exceeds 1 MiB");
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(|_| "cannot write cost router")?;
        std::fs::rename(&temp, path).map_err(|_| "cannot commit cost router")
    }
}

fn profile_key(context: &RouteCostContext<'_>, tier: ExecutionTier) -> String {
    let tier = serde_json::to_value(tier)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        context.origin, context.intent, tier, context.warmth, context.task_size
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RouteCostContext<'static> {
        RouteCostContext {
            origin: "https://example.test",
            intent: "intent-a",
            warmth: "warm",
            task_size: "small",
        }
    }

    #[test]
    fn learned_route_requires_eight_verified_clean_exact_cohort_samples() {
        let mut router = ExecutionCostRouter::default();
        let fast = ExecutionTier::QualifiedSkill;
        let slow = ExecutionTier::SemanticProgram;
        for elapsed in [20, 22, 21, 24, 23, 19, 25, 18] {
            router
                .record(&context(), fast, elapsed, true, true)
                .unwrap();
        }
        for _ in 0..8 {
            router.record(&context(), slow, 100, true, true).unwrap();
        }
        assert_eq!(router.select(&context(), &[slow, fast]), Some(fast));
        let other = RouteCostContext {
            origin: "https://other.test",
            ..context()
        };
        assert_eq!(router.select(&other, &[slow, fast]), None);
    }

    #[test]
    fn failed_or_unsafe_recent_sample_disqualifies_learning_and_unavailable_routes() {
        let mut router = ExecutionCostRouter::default();
        let fast = ExecutionTier::QualifiedSkill;
        for _ in 0..7 {
            router.record(&context(), fast, 10, true, true).unwrap();
        }
        router.record(&context(), fast, 5, true, false).unwrap();
        assert_eq!(router.select(&context(), &[fast]), None);
        assert_eq!(
            router.record(&context(), fast, 0, true, true),
            Err("invalid route cost sample")
        );
    }
}
