//! Bounded, verified route latency history for safe empirical tier selection.
//! Only exact origin/intent/warmth/task-size cohorts can influence a route.
use crate::capability_router::ExecutionTier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_PROFILES: usize = 512;
const MAX_SAMPLES_PER_PROFILE: usize = 32;
const MIN_CLEAN_SAMPLES: usize = 16;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCostRouter {
    profiles: BTreeMap<String, Vec<LatencySample>>,
    #[serde(default)]
    profile_recency: BTreeMap<String, u64>,
    #[serde(default)]
    next_profile_recency: u64,
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
        store.seed_missing_recency();
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
            || !matches!(
                context.warmth,
                "warm_browser_first_seen_site"
                    | "warm_browser_seen_site"
                    | "warm_qualified_route"
                    | "cold_browser"
                    | "unknown"
            )
            || !matches!(context.task_size, "small" | "medium" | "large")
            || elapsed_ms == 0
        {
            return Err("invalid route cost sample");
        }
        let key = profile_key(context, tier);
        if !self.profiles.contains_key(&key)
            && self.profiles.len() >= MAX_PROFILES
            && let Some(oldest) = self.least_recent_profile()
        {
            self.profiles.remove(&oldest);
            self.profile_recency.remove(&oldest);
        }
        self.touch_profile(&key);
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

    /// A route may override the static tier only after sixteen consecutive
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
                let rank = (latencies.len() * 90).div_ceil(100).max(1);
                Some((*tier, latencies[rank - 1]))
            })
            .min_by_key(|(_, p90)| *p90)
            .map(|(tier, _)| tier)
    }

    fn touch_profile(&mut self, key: &str) {
        self.next_profile_recency = self.next_profile_recency.saturating_add(1);
        self.profile_recency
            .insert(key.to_owned(), self.next_profile_recency);
    }

    fn least_recent_profile(&self) -> Option<String> {
        self.profiles
            .keys()
            .min_by_key(|key| {
                (
                    self.profile_recency.get(*key).copied().unwrap_or(0),
                    key.as_str(),
                )
            })
            .cloned()
    }

    fn seed_missing_recency(&mut self) {
        for key in self.profiles.keys().cloned().collect::<Vec<_>>() {
            if !self.profile_recency.contains_key(&key) {
                self.touch_profile(&key);
            }
        }
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
            warmth: "warm_qualified_route",
            task_size: "small",
        }
    }

    #[test]
    fn learned_route_requires_sixteen_verified_clean_exact_cohort_samples() {
        let mut router = ExecutionCostRouter::default();
        let fast = ExecutionTier::QualifiedSkill;
        let slow = ExecutionTier::SemanticProgram;
        for elapsed in [
            20, 22, 21, 24, 23, 19, 25, 18, 20, 22, 21, 24, 23, 19, 25, 18,
        ] {
            router
                .record(&context(), fast, elapsed, true, true)
                .unwrap();
        }
        for _ in 0..16 {
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
    fn learned_route_waits_for_sixteen_clean_samples_and_uses_nearest_rank_p90() {
        let mut router = ExecutionCostRouter::default();
        let fast = ExecutionTier::QualifiedSkill;
        let slow = ExecutionTier::SemanticProgram;
        for elapsed in [10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10] {
            router
                .record(&context(), fast, elapsed, true, true)
                .unwrap();
        }
        for _ in 0..15 {
            router.record(&context(), slow, 50, true, true).unwrap();
        }
        assert_eq!(
            router.select(&context(), &[slow, fast]),
            None,
            "a route must not qualify before 16 clean samples"
        );
        router.record(&context(), fast, 250, true, true).unwrap();
        router.record(&context(), slow, 50, true, true).unwrap();
        assert_eq!(
            router.select(&context(), &[slow, fast]),
            Some(fast),
            "one slow outlier must not make the recent maximum masquerade as p95"
        );
    }

    #[test]
    fn profile_capacity_evicts_least_recently_recorded_profile() {
        let mut router = ExecutionCostRouter::default();
        for index in 0..MAX_PROFILES {
            let key = if index == 0 {
                "z-old".to_owned()
            } else {
                format!("profile-{index:04}")
            };
            router.profiles.insert(key.clone(), Vec::new());
            router.profile_recency.insert(key, index as u64 + 1);
        }
        router.profile_recency.insert("z-old".into(), 0);
        router
            .record(&context(), ExecutionTier::SemanticProgram, 1, true, true)
            .unwrap();
        assert!(!router.profiles.contains_key("z-old"));
    }

    #[test]
    fn failed_or_unsafe_recent_sample_disqualifies_learning_and_unavailable_routes() {
        let mut router = ExecutionCostRouter::default();
        let fast = ExecutionTier::QualifiedSkill;
        for _ in 0..15 {
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
