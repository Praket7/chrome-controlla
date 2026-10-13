//! Shared task-guided filtering for untrusted page evidence.
use serde_json::Value;
use std::collections::BTreeSet;
pub const MAX_RELEVANCE_ITEMS: usize = 200;
pub fn score_text(query: &str, content: &str) -> f32 {
    let terms = query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|s| s.len() > 2)
        .collect::<BTreeSet<_>>();
    if terms.is_empty() {
        return 1.0;
    }
    let words = content
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .collect::<BTreeSet<_>>();
    terms.intersection(&words).count() as f32 / terms.len() as f32
}

/// Conservative, deterministic gate for speculative retrieval. Only actual
/// page evidence text contributes; JSON field names and runtime receipts do not.
pub fn speculative_result_matches(query: &str, result: &Value) -> bool {
    fn collect(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for key in [
                    "text",
                    "name",
                    "title",
                    "description",
                    "snippet",
                    "datetime",
                ] {
                    if let Some(text) = map.get(key).and_then(Value::as_str) {
                        out.push(text.to_owned());
                    }
                }
                for (key, child) in map {
                    if ![
                        "text",
                        "name",
                        "title",
                        "description",
                        "snippet",
                        "datetime",
                    ]
                    .contains(&key.as_str())
                    {
                        collect(child, out);
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| collect(item, out)),
            _ => {}
        }
    }
    if query.trim().is_empty() {
        return false;
    }
    let mut evidence = Vec::new();
    collect(result, &mut evidence);
    let evidence = evidence.join(" ");
    !evidence.trim().is_empty() && score_text(query, &evidence) >= 0.75
}
pub fn filter_items(items: &[Value], query: Option<&str>, limit: usize) -> Vec<Value> {
    let mut ranked = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let content = ["name", "text", "role", "value", "href", "datetime"]
                .iter()
                .filter_map(|k| item.get(k).and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            (score_text(query.unwrap_or_default(), &content), i, item)
        })
        .collect::<Vec<_>>();
    if query.is_some_and(|q| !q.trim().is_empty()) {
        // Keep a deterministic low-relevance tail when the query vocabulary
        // has no overlap. Dropping every zero-score node can erase useful
        // controls or warnings when page wording differs from the task.
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    ranked
        .into_iter()
        .take(limit.clamp(1, MAX_RELEVANCE_ITEMS))
        .map(|(_, _, v)| v.clone())
        .collect()
}
pub fn mark_untrusted(envelope: &mut Value) {
    envelope["page_content_is_untrusted"] = Value::Bool(true);
    envelope["provenance"] = Value::String("page_untrusted".into());
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn relevant_items_rank_without_removing_identity_or_trust_boundary() {
        let x = vec![
            json!({"reference":"@c1","name":"schedule"}),
            json!({"reference":"@c2","name":"Bills latest story"}),
        ];
        let y = filter_items(&x, Some("Bills story"), 10);
        assert_eq!(y[0]["reference"], "@c2");
        let mut envelope = json!({"items":y,"document":{"frame_id":"f"}});
        mark_untrusted(&mut envelope);
        assert_eq!(envelope["provenance"], "page_untrusted");
        assert_eq!(envelope["document"]["frame_id"], "f");
    }

    #[test]
    fn zero_overlap_does_not_turn_a_bounded_observation_into_empty_content() {
        let items = vec![
            json!({"reference":"@c1","role":"button","name":"Continue"}),
            json!({"reference":"@c2","role":"alert","name":"Billing warning"}),
        ];
        let kept = filter_items(&items, Some("find recent stories"), 2);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().any(|item| item["role"] == "alert"));
    }

    #[test]
    fn speculative_verification_requires_relevant_page_text_not_json_keys() {
        assert!(!speculative_result_matches(
            "Buffalo Bills latest story",
            &json!({"status":"completed","fields":{"type":"story"}}),
        ));
        assert!(!speculative_result_matches(
            "Buffalo Bills latest story",
            &json!({"items":[{"title":"Buffalo Bills"}]}),
        ));
        assert!(speculative_result_matches(
            "Buffalo Bills latest story",
            &json!({"items":[{"title":"Latest Buffalo Bills story"}]}),
        ));
    }
}
