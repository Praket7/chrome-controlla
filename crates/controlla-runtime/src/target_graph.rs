//! Revisioned target graph used to bind actions to a discovered browser state.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetGraph {
    pub revision: u64,
    pub targets: BTreeMap<String, TargetBinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetBinding {
    pub target_id: String,
    pub window_id: Option<i32>,
    pub tab_id: Option<i32>,
    pub group_id: Option<i32>,
    pub document_id: Option<String>,
    pub loader_id: Option<String>,
    pub interaction_revision: u64,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingError {
    Missing,
    Stale,
}

impl TargetGraph {
    pub fn replace(&mut self, bindings: impl IntoIterator<Item = TargetBinding>) {
        self.revision = self.revision.saturating_add(1);
        self.targets = bindings
            .into_iter()
            .map(|binding| (binding.target_id.clone(), binding))
            .collect();
    }

    pub fn bump_target(&mut self, target_id: &str) {
        self.revision = self.revision.saturating_add(1);
        if let Some(binding) = self.targets.get_mut(target_id) {
            binding.interaction_revision = binding.interaction_revision.saturating_add(1);
        }
    }

    pub fn resolve(
        &self,
        target_id: &str,
        expected_revision: u64,
    ) -> Result<&TargetBinding, BindingError> {
        if self.revision != expected_revision {
            return Err(BindingError::Stale);
        }
        self.targets.get(target_id).ok_or(BindingError::Missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_rejects_stale_bindings_and_bumps_changed_targets() {
        let mut graph = TargetGraph::default();
        graph.replace([TargetBinding {
            target_id: "t".into(),
            window_id: Some(1),
            tab_id: Some(2),
            group_id: None,
            document_id: Some("d".into()),
            loader_id: None,
            interaction_revision: 1,
            active: true,
        }]);
        let revision = graph.revision;
        graph.bump_target("t");
        assert_eq!(graph.resolve("t", revision), Err(BindingError::Stale));
        assert_eq!(graph.targets["t"].interaction_revision, 2);
    }
}
