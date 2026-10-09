//! Low-level browser execution telemetry used by higher-level benchmark and routing receipts.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationMetrics {
    pub cdp_commands: u64,
    pub cdp_bytes: u64,
    pub queue_us: u64,
    pub execution_us: u64,
    pub dom_nodes_scanned: u64,
    pub observation_bytes: u64,
    pub screenshot_bytes: u64,
    pub retries: u64,
}

impl OperationMetrics {
    pub fn merge(&mut self, other: &Self) {
        self.cdp_commands = self.cdp_commands.saturating_add(other.cdp_commands);
        self.cdp_bytes = self.cdp_bytes.saturating_add(other.cdp_bytes);
        self.queue_us = self.queue_us.saturating_add(other.queue_us);
        self.execution_us = self.execution_us.saturating_add(other.execution_us);
        self.dom_nodes_scanned = self.dom_nodes_scanned.saturating_add(other.dom_nodes_scanned);
        self.observation_bytes = self.observation_bytes.saturating_add(other.observation_bytes);
        self.screenshot_bytes = self.screenshot_bytes.saturating_add(other.screenshot_bytes);
        self.retries = self.retries.saturating_add(other.retries);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_merge_is_saturating() {
        let mut metrics = OperationMetrics { cdp_commands: u64::MAX, ..Default::default() };
        metrics.merge(&OperationMetrics { cdp_commands: 1, queue_us: 7, ..Default::default() });
        assert_eq!(metrics.cdp_commands, u64::MAX);
        assert_eq!(metrics.queue_us, 7);
    }
}
