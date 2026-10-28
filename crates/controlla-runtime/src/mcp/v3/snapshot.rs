//! Snapshot budgets and task-conditioned observation shaping.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ObservationRequest {
    pub task_hint: Option<String>,
    pub query: Option<String>,
    pub required_roles: Vec<String>,
    pub scope: Option<String>,
    pub since_snapshot_id: Option<String>,
    pub byte_budget: usize,
}

impl Default for ObservationRequest {
    fn default() -> Self {
        Self {
            task_hint: None,
            query: None,
            required_roles: Vec::new(),
            scope: None,
            since_snapshot_id: None,
            byte_budget: 12_000,
        }
    }
}

impl ObservationRequest {
    pub fn bounded(mut self) -> Self {
        self.byte_budget = self.byte_budget.clamp(4_096, 256_000);
        if self.query.as_ref().is_some_and(|query| query.chars().count() > 512) {
            self.query = self.query.take().map(|query| query.chars().take(512).collect());
        }
        if self.required_roles.len() > 32 {
            self.required_roles.truncate(32);
        }
        self
    }

    pub fn wants_delta(&self) -> bool {
        self.since_snapshot_id.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_requests_are_bounded() {
        let request = ObservationRequest {
            query: Some("x".repeat(600)),
            required_roles: (0..40).map(|index| format!("role-{index}")).collect(),
            byte_budget: 1,
            ..ObservationRequest::default()
        }
        .bounded();
        assert_eq!(request.byte_budget, 4_096);
        assert_eq!(request.query.unwrap().chars().count(), 512);
        assert_eq!(request.required_roles.len(), 32);
    }

    #[test]
    fn delta_requires_a_prior_snapshot() {
        assert!(!ObservationRequest::default().wants_delta());
        assert!(ObservationRequest {
            since_snapshot_id: Some("snap-1".into()),
            ..ObservationRequest::default()
        }
        .wants_delta());
    }
}
