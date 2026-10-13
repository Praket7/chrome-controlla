//! Bounded cross-target DAG compilation and deterministic read scheduling policy.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_DAG_NODES: usize = 64;
pub const DEFAULT_PARALLEL_SETUP_COST_MS: u64 = 300;
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DagEffect {
    ReadOnly,
    Mutation,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DagNode {
    pub id: String,
    pub target_id: String,
    pub deps: Vec<String>,
    pub effect: DagEffect,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledDag {
    pub layers: Vec<Vec<DagNode>>,
}

/// Run a read wave concurrently only when estimated saved execution time beats
/// the attach/alignment overhead. Small waves remain serial by design.
pub fn parallel_wave_is_worthwhile(estimated_node_ms: &[u64], setup_cost_ms: u64) -> bool {
    if estimated_node_ms.len() < 4 || estimated_node_ms.iter().any(|duration| *duration == 0) {
        return false;
    }
    let serial = estimated_node_ms
        .iter()
        .copied()
        .fold(0_u64, u64::saturating_add);
    let parallel = estimated_node_ms.iter().copied().max().unwrap_or_default();
    serial.saturating_sub(parallel) > setup_cost_ms
}

pub fn compile(nodes: Vec<DagNode>) -> Result<CompiledDag, &'static str> {
    if nodes.is_empty() || nodes.len() > MAX_DAG_NODES {
        return Err("DAG node count is outside the bound");
    }
    let mut by_id = BTreeMap::new();
    for node in nodes {
        if node.id.is_empty()
            || node.id.len() > 64
            || node.target_id.is_empty()
            || node.target_id.len() > 128
            || by_id.insert(node.id.clone(), node).is_some()
        {
            return Err("DAG node identity is invalid or duplicated");
        }
    }
    for node in by_id.values() {
        if node.deps.len() > MAX_DAG_NODES
            || node
                .deps
                .iter()
                .any(|dep| dep == &node.id || !by_id.contains_key(dep))
        {
            return Err("DAG dependency is invalid");
        }
    }
    let mut done = BTreeSet::new();
    let mut layers = Vec::new();
    while done.len() < by_id.len() {
        let mut ready = by_id
            .values()
            .filter(|n| !done.contains(&n.id) && n.deps.iter().all(|d| done.contains(d)))
            .cloned()
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err("DAG contains a cycle");
        }
        // Read-only nodes can share a layer. Mutations are ordered and isolated per target.
        ready.sort_by(|a, b| a.target_id.cmp(&b.target_id).then(a.id.cmp(&b.id)));
        let read_layer = ready.iter().any(|n| n.effect == DagEffect::ReadOnly);
        let mut layer = Vec::new();
        let mut mutating_targets = BTreeSet::new();
        for n in ready {
            if (read_layer && n.effect == DagEffect::ReadOnly)
                || (!read_layer && mutating_targets.insert(n.target_id.clone()))
            {
                done.insert(n.id.clone());
                layer.push(n);
            }
        }
        layers.push(layer);
    }
    Ok(CompiledDag { layers })
}

/// Execute one topological layer. Read-only nodes may overlap; mutation nodes are
/// parallel only when they target distinct tabs. Return order follows sorted DAG order.
pub async fn execute<F, Fut, T, E>(
    dag: &CompiledDag,
    execute_node: F,
) -> Result<Vec<(String, T)>, E>
where
    F: Fn(DagNode) -> Fut + Clone,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut results = Vec::new();
    for layer in &dag.layers {
        let mut pending = layer.clone();
        while !pending.is_empty() {
            let reads = pending.iter().any(|n| n.effect == DagEffect::ReadOnly);
            let mut wave = Vec::new();
            let mut targets = BTreeSet::new();
            let mut rest = Vec::new();
            for node in pending {
                if (reads && node.effect == DagEffect::ReadOnly)
                    || (!reads && targets.insert(node.target_id.clone()))
                {
                    wave.push(node)
                } else {
                    rest.push(node)
                }
            }
            let completed = futures_util::future::join_all(wave.into_iter().map(|node| {
                let f = execute_node.clone();
                async move {
                    let id = node.id.clone();
                    f(node).await.map(|value| (id, value))
                }
            }))
            .await;
            for value in completed {
                results.push(value?);
            }
            pending = rest;
        }
    }
    Ok(results)
}

/// Produce a deterministic compact evidence index from DAG branch receipts.
/// Every item retains its originating node and tab; page-derived contents remain
/// explicitly untrusted and are never promoted to a verified fact here.
pub fn merge_branch_evidence(
    receipts: &BTreeMap<String, Value>,
    targets: &BTreeMap<String, String>,
) -> Vec<Value> {
    let mut merged = Vec::new();
    for (node_id, receipt) in receipts {
        let Some(steps) = receipt["receipts"].as_array() else {
            continue;
        };
        let target_id = targets.get(node_id).map(String::as_str).unwrap_or("");
        for (index, step) in steps.iter().enumerate() {
            let evidence_items = ["items", "records", "matches"]
                .into_iter()
                .find_map(|field| step[field].as_array().map(|items| (field, items)))
                .map(|(field, items)| {
                    let bounded = items
                        .iter()
                        .take(4)
                        .map(|item| {
                            let mut summary = serde_json::Map::new();
                            for key in ["reference", "role", "name", "title", "text", "href"] {
                                if let Some(value) = item.get(key).and_then(Value::as_str) {
                                    let limit = if key == "text" { 240 } else { 128 };
                                    summary.insert(
                                        key.into(),
                                        json!(value.chars().take(limit).collect::<String>()),
                                    );
                                }
                            }
                            Value::Object(summary)
                        })
                        .collect::<Vec<_>>();
                    json!({"collection":field,"count":items.len(),"sample":bounded})
                });
            let text = step["text"]
                .as_str()
                .map(|text| text.chars().take(512).collect::<String>());
            let evidence_fact = step["evidence_fact"]
                .as_str()
                .map(|fact| fact.chars().take(512).collect::<String>());
            merged.push(json!({
                "node_id": node_id,
                "target_id": target_id,
                "step": index,
                "kind": step["step"],
                "status": step["status"],
                "verification": step["verification"],
                "evidence": evidence_items,
                "text": text,
                "evidence_fact": evidence_fact,
                "page_content_is_untrusted": true
            }));
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merged_branch_evidence_is_ordered_and_keeps_target_provenance_untrusted() {
        let receipts = BTreeMap::from([
            (
                "b".into(),
                json!({"receipts":[{"step":"extract","status":"verified","items":[
                    {"reference":"@c1","title":"Quarterly report","text":"Revenue rose."},
                    {"reference":"@c2","title":"Overflow","text":"second"}]}]}),
            ),
            (
                "a".into(),
                json!({"receipts":[{"step":"snapshot","status":"observed"}]}),
            ),
        ]);
        let targets = BTreeMap::from([("a".into(), "tab-1".into()), ("b".into(), "tab-2".into())]);
        let merged = merge_branch_evidence(&receipts, &targets);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0]["node_id"], "a");
        assert_eq!(merged[0]["target_id"], "tab-1");
        assert_eq!(merged[1]["node_id"], "b");
        assert_eq!(merged[1]["target_id"], "tab-2");
        assert_eq!(merged[1]["evidence"]["count"], 2);
        assert_eq!(
            merged[1]["evidence"]["sample"][0]["title"],
            "Quarterly report"
        );
        assert!(
            merged
                .iter()
                .all(|item| item["page_content_is_untrusted"] == true)
        );
    }
    #[test]
    fn independent_reads_across_tabs_share_deterministic_layer() {
        let c = compile(vec![
            DagNode {
                id: "b".into(),
                target_id: "2".into(),
                deps: vec![],
                effect: DagEffect::ReadOnly,
            },
            DagNode {
                id: "a".into(),
                target_id: "1".into(),
                deps: vec![],
                effect: DagEffect::ReadOnly,
            },
        ])
        .unwrap();
        assert_eq!(
            c.layers[0]
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn parallel_wave_grows_only_when_predicted_savings_exceed_setup_cost() {
        assert!(!parallel_wave_is_worthwhile(&[200, 200, 200], 100));
        assert!(!parallel_wave_is_worthwhile(&[100, 100, 100, 100], 300));
        assert!(parallel_wave_is_worthwhile(&[300, 300, 300, 300], 300));
        assert!(!parallel_wave_is_worthwhile(&[0, 300, 300, 300], 100));
    }
    #[test]
    fn cycles_are_rejected_and_mutations_to_same_target_are_separated() {
        let cycle = vec![
            DagNode {
                id: "a".into(),
                target_id: "1".into(),
                deps: vec!["b".into()],
                effect: DagEffect::ReadOnly,
            },
            DagNode {
                id: "b".into(),
                target_id: "1".into(),
                deps: vec!["a".into()],
                effect: DagEffect::ReadOnly,
            },
        ];
        assert!(compile(cycle).is_err());
        let c = compile(vec![
            DagNode {
                id: "a".into(),
                target_id: "1".into(),
                deps: vec![],
                effect: DagEffect::Mutation,
            },
            DagNode {
                id: "b".into(),
                target_id: "1".into(),
                deps: vec![],
                effect: DagEffect::Mutation,
            },
        ])
        .unwrap();
        assert_eq!(c.layers.len(), 2);
    }
    #[tokio::test]
    async fn executor_runs_layers_and_keeps_deterministic_receipt_order() {
        let dag = compile(vec![
            DagNode {
                id: "b".into(),
                target_id: "b".into(),
                deps: vec![],
                effect: DagEffect::ReadOnly,
            },
            DagNode {
                id: "a".into(),
                target_id: "a".into(),
                deps: vec![],
                effect: DagEffect::ReadOnly,
            },
            DagNode {
                id: "c".into(),
                target_id: "c".into(),
                deps: vec!["a".into(), "b".into()],
                effect: DagEffect::Mutation,
            },
        ])
        .unwrap();
        let result = execute(&dag, |node| async move { Ok::<_, ()>(node.target_id) })
            .await
            .unwrap();
        assert_eq!(
            result.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }
}
