//! Bounded, verified route latency history for safe empirical tier selection.
//! Only exact origin/intent/warmth/task-size cohorts can influence a route.
use crate::capability_router::ExecutionTier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_PROFILES: usize = 512;
const MAX_SAMPLES_PER_PROFILE: usize = 32;
const MIN_CLEAN_SAMPLES: usize = 16;
const MAX_SAMPLE_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1_000;

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
    #[serde(default)]
    recorded_at_ms: u64,
    verified: bool,
    safety_clean: bool,
    #[serde(default)]
    false_success: bool,
}

#[derive(Clone, Debug)]
pub struct RouteCostContext<'a> {
    pub origin: &'a str,
    pub intent: &'a str,
    pub provider: &'a str,
    pub warmth: &'a str,
    pub task_size: &'a str,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RouteCohortStats {
    pub sample_count: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub false_success_count: usize,
    pub success_rate: f64,
    pub ema_ms: f64,
    pub p50_ms: u64,
    pub p95_ms: u64,
    /// Standard error of observed latency, in milliseconds.
    pub uncertainty_ms: f64,
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
        self.record_sample(context, tier, elapsed_ms, verified, safety_clean, false)
    }

    /// Record a result independently determined to be a false success. Such a
    /// cohort remains visible in statistics and is disqualified from routing.
    pub fn record_false_success(
        &mut self,
        context: &RouteCostContext<'_>,
        tier: ExecutionTier,
        elapsed_ms: u64,
    ) -> Result<(), &'static str> {
        self.record_sample(context, tier, elapsed_ms, false, false, true)
    }

    fn record_sample(
        &mut self,
        context: &RouteCostContext<'_>,
        tier: ExecutionTier,
        elapsed_ms: u64,
        verified: bool,
        safety_clean: bool,
        false_success: bool,
    ) -> Result<(), &'static str> {
        if context.origin.is_empty()
            || context.origin.len() > 512
            || context.intent.is_empty()
            || context.intent.len() > 128
            || context.provider.is_empty()
            || context.provider.len() > 64
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
            recorded_at_ms: unix_time_ms(),
            verified,
            safety_clean,
            false_success,
        });
        if samples.len() > MAX_SAMPLES_PER_PROFILE {
            samples.remove(0);
        }
        self.persist()
    }

    pub fn stats(
        &self,
        context: &RouteCostContext<'_>,
        tier: ExecutionTier,
    ) -> Option<RouteCohortStats> {
        self.stats_at(context, tier, unix_time_ms())
    }

    fn stats_at(
        &self,
        context: &RouteCostContext<'_>,
        tier: ExecutionTier,
        now_ms: u64,
    ) -> Option<RouteCohortStats> {
        let samples = self.live_samples(context, tier, now_ms)?;
        if samples.is_empty() {
            return None;
        }
        let sample_count = samples.len();
        let success_count = samples
            .iter()
            .filter(|sample| sample.verified && sample.safety_clean && !sample.false_success)
            .count();
        let false_success_count = samples.iter().filter(|sample| sample.false_success).count();
        let failure_count = sample_count.saturating_sub(success_count + false_success_count);
        let mut latencies = samples
            .iter()
            .map(|sample| sample.elapsed_ms)
            .collect::<Vec<_>>();
        latencies.sort_unstable();
        let percentile = |percent: usize| -> u64 {
            let rank = (latencies.len() * percent).div_ceil(100).max(1);
            latencies[rank - 1]
        };
        // Newer measurements receive more weight while the full distribution
        // remains available for qualification and confidence reporting.
        let ema_ms = samples
            .iter()
            .fold(samples[0].elapsed_ms as f64, |ema, sample| {
                ema + 0.3 * (sample.elapsed_ms as f64 - ema)
            });
        let mean = latencies.iter().map(|value| *value as f64).sum::<f64>() / sample_count as f64;
        let variance = latencies
            .iter()
            .map(|value| (*value as f64 - mean).powi(2))
            .sum::<f64>()
            / sample_count as f64;
        Some(RouteCohortStats {
            sample_count,
            success_count,
            failure_count,
            false_success_count,
            success_rate: success_count as f64 / sample_count as f64,
            ema_ms,
            p50_ms: percentile(50),
            p95_ms: percentile(95),
            uncertainty_ms: variance.sqrt() / (sample_count as f64).sqrt(),
        })
    }

    /// A route may override the static tier only after sixteen consecutive
    /// independently verified, safety-clean observations for that exact cohort.
    /// Unmeasured routes keep the conservative static ordering.
    pub fn select(
        &self,
        context: &RouteCostContext<'_>,
        available: &[ExecutionTier],
    ) -> Option<ExecutionTier> {
        self.select_at(context, available, unix_time_ms())
    }

    /// Select from statistically qualified cohorts using observed conservative
    /// latency plus an explicit per-route estimate for non-latency overhead.
    pub fn select_with_overhead(
        &self,
        context: &RouteCostContext<'_>,
        available: &[ExecutionTier],
        overhead_ms: impl Fn(ExecutionTier) -> u64,
    ) -> Option<ExecutionTier> {
        self.select_scored_at(context, available, unix_time_ms(), overhead_ms)
    }

    fn select_at(
        &self,
        context: &RouteCostContext<'_>,
        available: &[ExecutionTier],
        now_ms: u64,
    ) -> Option<ExecutionTier> {
        self.select_scored_at(context, available, now_ms, |_| 0)
    }

    fn select_scored_at(
        &self,
        context: &RouteCostContext<'_>,
        available: &[ExecutionTier],
        now_ms: u64,
        overhead_ms: impl Fn(ExecutionTier) -> u64,
    ) -> Option<ExecutionTier> {
        available
            .iter()
            .filter_map(|tier| {
                let samples = self.live_samples(context, *tier, now_ms)?;
                if samples.len() < MIN_CLEAN_SAMPLES {
                    return None;
                }
                let recent = &samples[samples.len() - MIN_CLEAN_SAMPLES..];
                if recent
                    .iter()
                    .any(|sample| !sample.verified || !sample.safety_clean || sample.false_success)
                {
                    return None;
                }
                let mut latencies = recent
                    .iter()
                    .map(|sample| sample.elapsed_ms)
                    .collect::<Vec<_>>();
                latencies.sort_unstable();
                let rank = (latencies.len() * 95).div_ceil(100).max(1);
                let mean = latencies.iter().map(|value| *value as f64).sum::<f64>()
                    / latencies.len() as f64;
                let variance = latencies
                    .iter()
                    .map(|value| (*value as f64 - mean).powi(2))
                    .sum::<f64>()
                    / latencies.len() as f64;
                // Penalize noisy cohorts so one fast but unstable route does not
                // displace a predictable route on a small sample.
                let conservative_ms = latencies[rank - 1] as f64
                    + 1.96 * variance.sqrt() / (latencies.len() as f64).sqrt();
                Some((*tier, conservative_ms + overhead_ms(*tier) as f64))
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(tier, _)| tier)
    }

    fn live_samples(
        &self,
        context: &RouteCostContext<'_>,
        tier: ExecutionTier,
        now_ms: u64,
    ) -> Option<Vec<&LatencySample>> {
        let samples = self.profiles.get(&profile_key(context, tier))?;
        let live = samples
            .iter()
            .filter(|sample| {
                sample.recorded_at_ms != 0
                    && sample.recorded_at_ms <= now_ms
                    && now_ms - sample.recorded_at_ms <= MAX_SAMPLE_AGE_MS
            })
            .collect::<Vec<_>>();
        (!live.is_empty()).then_some(live)
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

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn profile_key(context: &RouteCostContext<'_>, tier: ExecutionTier) -> String {
    let tier = serde_json::to_value(tier)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        context.origin, context.intent, context.provider, tier, context.warmth, context.task_size
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RouteCostContext<'static> {
        RouteCostContext {
            origin: "https://example.test",
            intent: "intent-a",
            provider: "chrome",
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
    fn learned_route_waits_for_sixteen_clean_samples_and_uses_p95() {
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
            Some(slow),
            "p95 must retain a slow tail rather than hiding it as an average"
        );
    }

    #[test]
    fn expired_cohort_samples_are_not_reported_or_used_for_routing() {
        let mut router = ExecutionCostRouter::default();
        let tier = ExecutionTier::QualifiedSkill;
        let now = MAX_SAMPLE_AGE_MS + 10_000;
        let old = LatencySample {
            elapsed_ms: 1,
            recorded_at_ms: now - MAX_SAMPLE_AGE_MS - 1,
            verified: true,
            safety_clean: true,
            false_success: false,
        };
        let fresh = LatencySample {
            recorded_at_ms: now,
            ..old
        };
        router
            .profiles
            .insert(profile_key(&context(), tier), vec![old, fresh]);

        let stats = router.stats_at(&context(), tier, now).unwrap();
        assert_eq!(stats.sample_count, 1);
        assert_eq!(stats.p50_ms, 1);
        assert_eq!(router.select_at(&context(), &[tier], now), None);
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

    #[test]
    fn cohort_reports_failures_false_successes_and_latency_distribution() {
        let mut router = ExecutionCostRouter::default();
        let tier = ExecutionTier::SemanticProgram;
        for elapsed in [10, 20, 30, 40, 50, 60, 70, 80, 90, 100] {
            router
                .record(&context(), tier, elapsed, true, true)
                .unwrap();
        }
        router.record(&context(), tier, 200, false, true).unwrap();
        router.record_false_success(&context(), tier, 300).unwrap();

        let stats = router.stats(&context(), tier).unwrap();
        assert_eq!(stats.sample_count, 12);
        assert_eq!(stats.success_count, 10);
        assert_eq!(stats.failure_count, 1);
        assert_eq!(stats.false_success_count, 1);
        assert_eq!(stats.p50_ms, 60);
        assert_eq!(stats.p95_ms, 300);
        assert!(stats.ema_ms >= 10.0);
        assert!(stats.uncertainty_ms > 0.0);
    }

    #[test]
    fn cohort_stats_are_scoped_by_provider_as_well_as_site_and_task() {
        let mut router = ExecutionCostRouter::default();
        let chrome = context();
        for _ in 0..16 {
            router
                .record(&chrome, ExecutionTier::SemanticProgram, 10, true, true)
                .unwrap();
        }
        let other_provider = RouteCostContext {
            provider: "playwright",
            ..chrome
        };
        assert!(
            router
                .stats(&other_provider, ExecutionTier::SemanticProgram)
                .is_none()
        );
    }
}
