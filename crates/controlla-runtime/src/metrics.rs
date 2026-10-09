use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheStatus {
    #[default]
    Miss,
    Candidate,
    Hit,
    Quarantined,
    Expired,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteClass {
    NativeApp,
    QualifiedSkill,
    #[default]
    Semantic,
    VisualProbe,
    StrictInput,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrowserOperationMetrics {
    pub cdp_commands: u64,
    pub cdp_bytes: u64,
    pub queue_us: u64,
    pub execution_us: u64,
    pub dom_nodes_scanned: u64,
    pub observation_bytes: u64,
    pub screenshot_bytes: u64,
    pub retries: u64,
}

impl BrowserOperationMetrics {
    pub fn merge(&mut self, other: &Self) {
        self.cdp_commands = self.cdp_commands.saturating_add(other.cdp_commands);
        self.cdp_bytes = self.cdp_bytes.saturating_add(other.cdp_bytes);
        self.queue_us = self.queue_us.saturating_add(other.queue_us);
        self.execution_us = self.execution_us.saturating_add(other.execution_us);
        self.dom_nodes_scanned = self
            .dom_nodes_scanned
            .saturating_add(other.dom_nodes_scanned);
        self.observation_bytes = self
            .observation_bytes
            .saturating_add(other.observation_bytes);
        self.screenshot_bytes = self.screenshot_bytes.saturating_add(other.screenshot_bytes);
        self.retries = self.retries.saturating_add(other.retries);
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionMetrics {
    pub browser: BrowserOperationMetrics,
    pub model_calls: u64,
    pub model_input_tokens: u64,
    pub model_output_tokens: u64,
    pub cache_status: CacheStatus,
    pub route: RouteClass,
    pub verification_us: u64,
    pub recovery_count: u64,
}

impl Default for ExecutionMetrics {
    fn default() -> Self {
        Self {
            browser: BrowserOperationMetrics::default(),
            model_calls: 0,
            model_input_tokens: 0,
            model_output_tokens: 0,
            cache_status: CacheStatus::Miss,
            route: RouteClass::Semantic,
            verification_us: 0,
            recovery_count: 0,
        }
    }
}

impl ExecutionMetrics {
    pub fn record_model_call(&mut self, input_tokens: u64, output_tokens: u64) {
        self.model_calls = self.model_calls.saturating_add(1);
        self.model_input_tokens = self.model_input_tokens.saturating_add(input_tokens);
        self.model_output_tokens = self.model_output_tokens.saturating_add(output_tokens);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_metrics_merge_saturates_instead_of_wrapping() {
        let mut metrics = BrowserOperationMetrics {
            cdp_commands: u64::MAX,
            ..Default::default()
        };
        metrics.merge(&BrowserOperationMetrics {
            cdp_commands: 1,
            observation_bytes: 42,
            ..Default::default()
        });
        assert_eq!(metrics.cdp_commands, u64::MAX);
        assert_eq!(metrics.observation_bytes, 42);
    }

    #[test]
    fn model_calls_are_counted_explicitly() {
        let mut metrics = ExecutionMetrics::default();
        metrics.record_model_call(100, 20);
        metrics.record_model_call(50, 10);
        assert_eq!(metrics.model_calls, 2);
        assert_eq!(metrics.model_input_tokens, 150);
        assert_eq!(metrics.model_output_tokens, 30);
    }
}
