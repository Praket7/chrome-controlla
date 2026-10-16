//! Per-operation runtime latency objectives.
//!
//! These are engineering gates for runtime work only. Page/network time and
//! caller model latency are intentionally outside the measurement boundary.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    SemanticClick,
    Fill,
    Scroll,
    SnapshotDelta,
    BackgroundScreenshot,
    ReplayWorkflow,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LatencySlo {
    pub operation: OperationKind,
    pub target_us: u64,
    pub excludes_page_and_network_time: bool,
}

pub fn slo_for(operation: OperationKind) -> LatencySlo {
    let target_us = match operation {
        OperationKind::SemanticClick => 50_000,
        OperationKind::Fill => 40_000,
        OperationKind::Scroll => 40_000,
        OperationKind::SnapshotDelta => 50_000,
        OperationKind::BackgroundScreenshot => 100_000,
        OperationKind::ReplayWorkflow => 150_000,
    };
    LatencySlo {
        operation,
        target_us,
        excludes_page_and_network_time: true,
    }
}

pub fn within_slo(operation: OperationKind, elapsed_us: u64) -> bool {
    elapsed_us <= slo_for(operation).target_us
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_match_the_release_contract() {
        assert_eq!(slo_for(OperationKind::SemanticClick).target_us, 50_000);
        assert_eq!(slo_for(OperationKind::Fill).target_us, 40_000);
        assert_eq!(slo_for(OperationKind::Scroll).target_us, 40_000);
        assert_eq!(slo_for(OperationKind::SnapshotDelta).target_us, 50_000);
        assert_eq!(
            slo_for(OperationKind::BackgroundScreenshot).target_us,
            100_000
        );
        assert_eq!(slo_for(OperationKind::ReplayWorkflow).target_us, 150_000);
    }

    #[test]
    fn boundary_is_inclusive_and_page_time_is_outside_contract() {
        let slo = slo_for(OperationKind::BackgroundScreenshot);
        assert!(slo.excludes_page_and_network_time);
        assert!(within_slo(
            OperationKind::BackgroundScreenshot,
            slo.target_us
        ));
        assert!(!within_slo(
            OperationKind::BackgroundScreenshot,
            slo.target_us + 1
        ));
    }
}
