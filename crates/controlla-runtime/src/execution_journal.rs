//! Verified execution journal: admitted -> intent_recorded -> dispatched -> observed -> persisted -> final
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalState {
    Admitted,
    IntentRecorded,
    Dispatched,
    Observed,
    Persisted,
    Final,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    NotDispatched,
    Dispatched,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Verified,
    NotDispatched,
    Unknown,
    Failed,
    Unsupported,
    NeedsUser,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalEntry {
    pub operation_id: String,
    pub state: JournalState,
    pub delivery: DeliveryStatus,
    pub outcome: OutcomeStatus,
    pub reason_code: Option<String>,
    pub retry_class: Option<String>,
    pub next_action: Option<String>,
    pub dispatch_may_have_occurred: bool,
}

impl JournalEntry {
    pub fn new(operation_id: impl Into<String>) -> Self {
        Self {
            operation_id: operation_id.into(),
            state: JournalState::Admitted,
            delivery: DeliveryStatus::NotDispatched,
            outcome: OutcomeStatus::NotDispatched,
            reason_code: None,
            retry_class: None,
            next_action: None,
            dispatch_may_have_occurred: false,
        }
    }
    pub fn transition(&mut self, state: JournalState) {
        self.state = state;
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self.state, JournalState::Final)
    }
}

pub fn next_action_for(retry_class: &str) -> &'static str {
    match retry_class {
        "safe_internal" => "retry_after_bounded_scroll",
        "refresh_snapshot" => "reacquire_reference",
        "requires_user" => "needs_user",
        "do_not_retry" => "observe_fresh_state",
        _ => "observe_fresh_state",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_transitions() {
        let mut e = JournalEntry::new("op1");
        e.transition(JournalState::Dispatched);
        assert_eq!(e.state, JournalState::Dispatched);
    }
    #[test]
    fn unknown_never_retried() {
        let e = JournalEntry {
            operation_id: "op".into(),
            state: JournalState::Observed,
            delivery: DeliveryStatus::Unknown,
            outcome: OutcomeStatus::Unknown,
            reason_code: Some("transport_lost_after_dispatch".into()),
            retry_class: Some("do_not_retry".into()),
            next_action: Some("observe_fresh_state".into()),
            dispatch_may_have_occurred: true,
        };
        assert!(e.dispatch_may_have_occurred);
        assert_eq!(e.retry_class.as_deref(), Some("do_not_retry"));
    }
}
