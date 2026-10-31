//! Progressive observation: start compact, escalate once only when the first
//! packet cannot satisfy the task's explicit requirements.
//!
//! The escalation decision is deterministic and based on the returned evidence,
//! not on page instructions. It never loops: one observation may escalate at
//! most one stage, so a hostile page cannot force unbounded resampling.

use crate::observation_router::{ObservationDecision, ObservationMode};
use serde_json::{Value, json};

pub const ESCALATED_ITEMS: usize = 200;
pub const ESCALATED_TEXT_CHARS: usize = 6_000;

#[derive(Clone, Debug)]
pub struct ProgressiveObservation {
    pub stages: u8,
    pub escalated: bool,
    pub reason: Option<&'static str>,
    pub initial_mode: ObservationMode,
}

impl ProgressiveObservation {
    pub fn new(initial_mode: ObservationMode) -> Self {
        Self {
            stages: 1,
            escalated: false,
            reason: None,
            initial_mode,
        }
    }

    /// Returns the bounded reason the first packet is insufficient, or `None`
    /// when it can satisfy the declared task requirements.
    pub fn escalation_reason(
        &self,
        value: &Value,
        required_roles: &[String],
        query_required: bool,
    ) -> Option<&'static str> {
        if self.escalated || self.initial_mode != ObservationMode::SemanticCompact {
            return None;
        }
        if matches!(
            value["status"].as_str(),
            Some("failed" | "unknown" | "not_dispatched")
        ) {
            return Some("observation_status_unusable");
        }
        if value["truncated"] == true
            || value["omitted_by_limit"].as_u64().unwrap_or(0) > 0
            || value["omitted_oversized"].as_u64().unwrap_or(0) > 0
        {
            return Some("collection_truncated");
        }
        if value["scope_complete"] == false && value["global_complete"] != true {
            return Some("scope_incomplete");
        }
        if query_required && value["goal_satisfied"] == false {
            return Some("goal_not_satisfied");
        }
        let items = value["items"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        if items.is_empty() && value["global_complete"] != true {
            return Some("no_controls_observed");
        }
        for role in required_roles {
            let role = role.to_ascii_lowercase();
            if !items.iter().any(|item| {
                item["role"]
                    .as_str()
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&role))
            }) {
                return Some("required_role_missing");
            }
        }
        None
    }

    pub fn escalate(&mut self, reason: &'static str) {
        if self.escalated {
            return;
        }
        self.escalated = true;
        self.stages = 2;
        self.reason = Some(reason);
    }

    pub fn effective_decision(&self, decision: ObservationDecision) -> ObservationDecision {
        if self.escalated {
            ObservationDecision {
                mode: ObservationMode::SemanticRich,
                reason: "progressive observation escalated once after the compact packet could not satisfy its task requirements",
            }
        } else {
            decision
        }
    }

    pub fn receipt(&self) -> Value {
        json!({
            "stages": self.stages,
            "escalated": self.escalated,
            "reason": self.reason,
            "initial_mode": self.initial_mode,
            "final_mode": if self.escalated { ObservationMode::SemanticRich } else { self.initial_mode },
            "max_stages": 2,
            "bounded": true,
            "page_instructions_can_escalate": false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact() -> ProgressiveObservation {
        ProgressiveObservation::new(ObservationMode::SemanticCompact)
    }

    #[test]
    fn complete_compact_packet_does_not_escalate() {
        let value = json!({
            "status":"completed",
            "items":[{"role":"button","name":"Save"}],
            "scope_complete":true,
            "global_complete":true,
            "goal_satisfied":true
        });
        assert_eq!(
            compact().escalation_reason(&value, &["button".into()], true),
            None
        );
    }

    #[test]
    fn truncation_missing_role_and_missing_goal_escalate_once() {
        let truncated = json!({
            "status":"completed",
            "items":[{"role":"button","name":"Save"}],
            "truncated":true,
            "scope_complete":true
        });
        assert_eq!(
            compact().escalation_reason(&truncated, &[], false),
            Some("collection_truncated")
        );
        let missing_role = json!({
            "status":"completed",
            "items":[{"role":"button","name":"Save"}],
            "scope_complete":true
        });
        assert_eq!(
            compact().escalation_reason(&missing_role, &["textbox".into()], false),
            Some("required_role_missing")
        );
        let missing_goal = json!({
            "status":"completed",
            "items":[{"role":"textbox","name":"Search"}],
            "scope_complete":true,
            "goal_satisfied":false
        });
        assert_eq!(
            compact().escalation_reason(&missing_goal, &[], true),
            Some("goal_not_satisfied")
        );
        let mut progressive = compact();
        progressive.escalate("collection_truncated");
        assert_eq!(
            progressive.escalation_reason(&truncated, &[], false),
            None,
            "one observation escalates at most once"
        );
        assert_eq!(progressive.receipt()["stages"], 2);
        assert_eq!(
            progressive.receipt()["page_instructions_can_escalate"],
            false
        );
    }

    #[test]
    fn rich_or_failed_observations_do_not_start_a_second_stage() {
        let rich = ProgressiveObservation::new(ObservationMode::SemanticRich);
        let value = json!({"status":"failed","items":[]});
        assert_eq!(rich.escalation_reason(&value, &[], false), None);
        let compact = compact();
        assert_eq!(
            compact.escalation_reason(&json!({"status":"unknown","items":[]}), &[], false),
            Some("observation_status_unusable")
        );
    }
}
