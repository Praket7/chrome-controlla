use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_PROFILES: usize = 256;
const MAX_SAMPLES: usize = 32;
const MIN_QUALIFIED_SAMPLES: usize = 8;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationMode {
    SemanticCompact,
    SemanticRich,
    AccessibilityFrame,
    HtmlScoped,
    VisualTargeted,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TaskClass {
    #[default]
    General,
    Research,
    FilterSortDashboard,
    Mutation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelProfile {
    CompactContext,
    RichContext,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ObservationRequest {
    pub task_class: TaskClass,
    pub model_profile: Option<ModelProfile>,
    pub exhaustive: bool,
    pub needs_layout: bool,
    pub ambiguity: f32,
    pub rich_context: bool,
    pub frame_id: bool,
    pub scoped_selector: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationDecision {
    pub mode: ObservationMode,
    pub reason: &'static str,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationHistory {
    profiles: BTreeMap<String, Vec<ObservationSample>>,
    #[serde(skip)]
    path: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ObservationSample {
    mode: ObservationMode,
    elapsed_ms: u64,
    bytes: u64,
    usable: bool,
}

impl ObservationHistory {
    pub fn open(path: impl Into<std::path::PathBuf>) -> Self {
        let path = path.into();
        let mut history = match std::fs::read(&path) {
            Ok(bytes) if bytes.len() <= 512 * 1024 => {
                serde_json::from_slice(&bytes).unwrap_or_default()
            }
            _ => Self::default(),
        };
        history.path = Some(path);
        history
    }

    pub fn record(
        &mut self,
        request: ObservationRequest,
        decision: ObservationDecision,
        elapsed_ms: u64,
        bytes: u64,
        usable: bool,
    ) {
        if elapsed_ms == 0 || bytes > 1_000_000 {
            return;
        }
        let key = profile_key(request);
        if !self.profiles.contains_key(&key)
            && self.profiles.len() >= MAX_PROFILES
            && let Some(oldest) = self.profiles.keys().next().cloned()
        {
            self.profiles.remove(&oldest);
        }
        let samples = self.profiles.entry(key).or_default();
        samples.push(ObservationSample {
            mode: decision.mode,
            elapsed_ms,
            bytes,
            usable,
        });
        if samples.len() > MAX_SAMPLES {
            samples.remove(0);
        }
        if samples.len().is_multiple_of(8) {
            self.persist();
        }
    }

    fn best_mode(
        &self,
        request: ObservationRequest,
        maximum: ObservationMode,
    ) -> Option<ObservationMode> {
        if request.frame_id || request.exhaustive || request.needs_layout && request.ambiguity > 0.6
        {
            return None;
        }
        let samples = self.profiles.get(&profile_key(request))?;
        let eligible = [
            ObservationMode::SemanticCompact,
            ObservationMode::SemanticRich,
            ObservationMode::HtmlScoped,
        ]
        .into_iter()
        .filter(|mode| mode_rank(*mode) <= mode_rank(maximum));
        eligible
            .filter_map(|mode| {
                let matching = samples
                    .iter()
                    .filter(|sample| sample.mode == mode)
                    .rev()
                    .take(MIN_QUALIFIED_SAMPLES)
                    .collect::<Vec<_>>();
                if matching.len() < MIN_QUALIFIED_SAMPLES
                    || matching.iter().any(|sample| !sample.usable)
                {
                    return None;
                }
                let mut latency = matching
                    .iter()
                    .map(|sample| sample.elapsed_ms)
                    .collect::<Vec<_>>();
                latency.sort_unstable();
                let mut bytes = matching
                    .iter()
                    .map(|sample| sample.bytes)
                    .collect::<Vec<_>>();
                bytes.sort_unstable();
                // Deterministic wall-time utility: p95 operation latency plus
                // bounded serialization cost. No model-based reduction is used.
                let utility =
                    latency[latency.len() - 1].saturating_add(bytes[bytes.len() - 1] / 128);
                Some((mode, utility))
            })
            .min_by_key(|(_, utility)| *utility)
            .map(|(mode, _)| mode)
    }

    fn persist(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(self) else {
            return;
        };
        if bytes.len() > 512 * 1024 {
            return;
        }
        let temp = path.with_extension("json.tmp");
        if std::fs::write(&temp, bytes).is_ok() {
            let _ = std::fs::rename(temp, path);
        }
    }
}

fn profile_key(request: ObservationRequest) -> String {
    // Pool comparable tasks across requested richness/scope so the history
    // can compare modes observed in naturally varied runs. Hard requirements
    // (frame, exhaustive coverage, and visual layout evidence) are still
    // enforced by `best_mode` and never learned away.
    format!("{:?}|{:?}", request.task_class, request.model_profile)
}

fn mode_rank(mode: ObservationMode) -> u8 {
    match mode {
        ObservationMode::SemanticCompact => 0,
        ObservationMode::SemanticRich => 1,
        ObservationMode::HtmlScoped => 2,
        ObservationMode::AccessibilityFrame => 3,
        ObservationMode::VisualTargeted => 4,
    }
}

pub fn choose_observation_with_history(
    request: ObservationRequest,
    history: &ObservationHistory,
) -> ObservationDecision {
    let baseline = choose_observation(request);
    history.best_mode(request, baseline.mode).map_or(baseline, |mode| ObservationDecision {
        mode,
        reason: "eight usable observations qualified this lower expected wall-time mode for the matching task profile",
    })
}

pub fn choose_observation(request: ObservationRequest) -> ObservationDecision {
    let (mode, reason) = if request.frame_id {
        (
            ObservationMode::AccessibilityFrame,
            "caller selected a child accessibility frame",
        )
    } else if request.needs_layout && request.ambiguity > 0.6 && request.scoped_selector {
        (
            ObservationMode::VisualTargeted,
            "layout ambiguity requires a bounded targeted visual probe",
        )
    } else if request.task_class == TaskClass::FilterSortDashboard
        && request.model_profile == Some(ModelProfile::RichContext)
        && request.scoped_selector
    {
        (
            ObservationMode::HtmlScoped,
            "dashboard task and rich-context profile benefit from scoped structure",
        )
    } else if request.scoped_selector && (request.rich_context || request.needs_layout) {
        (
            ObservationMode::HtmlScoped,
            "caller requested a bounded scoped HTML view",
        )
    } else if request.exhaustive
        || request.ambiguity > 0.3
        || request.rich_context
        || request.task_class == TaskClass::Mutation
    {
        (
            ObservationMode::SemanticRich,
            if request.scoped_selector {
                "scoped semantic view with richer context was requested"
            } else {
                "coverage or ambiguity requires a richer semantic view"
            },
        )
    } else {
        (
            ObservationMode::SemanticCompact,
            "compact semantic evidence is sufficient",
        )
    };
    ObservationDecision { mode, reason }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn measured_history_can_choose_a_faster_usable_mode_but_keeps_hard_visual_escalation() {
        let request = ObservationRequest {
            rich_context: true,
            task_class: TaskClass::Research,
            ..Default::default()
        };
        let mut history = ObservationHistory::default();
        let compact = ObservationDecision {
            mode: ObservationMode::SemanticCompact,
            reason: "test",
        };
        for _ in 0..8 {
            history.record(request, compact, 10, 512, true);
        }
        assert_eq!(
            choose_observation_with_history(request, &history).mode,
            ObservationMode::SemanticCompact
        );
        let visual = ObservationRequest {
            needs_layout: true,
            ambiguity: 0.9,
            scoped_selector: true,
            ..request
        };
        assert_eq!(
            choose_observation_with_history(visual, &history).mode,
            ObservationMode::VisualTargeted
        );
    }

    #[test]
    fn history_does_not_qualify_modes_with_failed_or_insufficient_observations() {
        let request = ObservationRequest {
            rich_context: true,
            ..Default::default()
        };
        let mut history = ObservationHistory::default();
        let compact = ObservationDecision {
            mode: ObservationMode::SemanticCompact,
            reason: "test",
        };
        for _ in 0..7 {
            history.record(request, compact, 1, 1, true);
        }
        assert_ne!(
            choose_observation_with_history(request, &history).mode,
            ObservationMode::SemanticCompact
        );
    }
}
