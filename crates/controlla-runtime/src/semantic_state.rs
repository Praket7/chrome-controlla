use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SemanticDocumentKey {
    pub frame_id: String,
    pub loader_id: String,
    pub url: String,
    pub frame_revision: Option<String>,
    pub document_revision: Option<String>,
}

impl SemanticDocumentKey {
    pub fn from_snapshot(snapshot: &Value) -> Self {
        fn string_at(snapshot: &Value, path: &str) -> String {
            snapshot
                .pointer(path)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        }
        fn revision_at(snapshot: &Value, path: &str) -> Option<String> {
            snapshot.pointer(path).and_then(|value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            })
        }
        Self {
            frame_id: string_at(snapshot, "/document/frame_id"),
            loader_id: string_at(snapshot, "/document/loader_id"),
            url: string_at(snapshot, "/document/url"),
            frame_revision: revision_at(snapshot, "/document/frame_revision"),
            document_revision: revision_at(snapshot, "/document/document_revision"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SnapshotDelta {
    pub full: bool,
    pub document_changed: bool,
    pub upsert: Vec<Value>,
    pub removed: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SemanticSnapshotUpdate {
    pub snapshot_revision: String,
    pub items: Vec<Value>,
    pub delta: SnapshotDelta,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum SemanticStateError {
    #[error("semantic snapshot item omitted its retained reference")]
    MissingRetainedReference,
    #[error("reference must be an exact @cN semantic reference")]
    InvalidReference,
    #[error("reference is stale or unknown for the current document")]
    StaleReference,
}

#[derive(Clone, Debug, Default)]
pub struct SemanticMemory {
    document: Option<SemanticDocumentKey>,
    next_ref: u32,
    stable_to_short: BTreeMap<String, String>,
    short_to_legacy: BTreeMap<String, String>,
    items: BTreeMap<String, Value>,
    snapshot_revision: String,
}

impl SemanticMemory {
    pub fn document(&self) -> Option<&SemanticDocumentKey> {
        self.document.as_ref()
    }

    pub fn snapshot_revision(&self) -> &str {
        &self.snapshot_revision
    }

    pub fn resolve(&self, reference: &str) -> Result<&str, SemanticStateError> {
        if !reference.starts_with("@c") || reference[2..].parse::<u32>().is_err() {
            return Err(SemanticStateError::InvalidReference);
        }
        self.short_to_legacy
            .get(reference)
            .map(String::as_str)
            .ok_or(SemanticStateError::StaleReference)
    }

    pub fn item(&self, reference: &str) -> Result<&Value, SemanticStateError> {
        if !reference.starts_with("@c") || reference[2..].parse::<u32>().is_err() {
            return Err(SemanticStateError::InvalidReference);
        }
        self.items
            .get(reference)
            .ok_or(SemanticStateError::StaleReference)
    }

    pub fn ingest(
        &mut self,
        document: SemanticDocumentKey,
        snapshot_revision: impl Into<String>,
        items: Vec<Value>,
    ) -> Result<SemanticSnapshotUpdate, SemanticStateError> {
        let had_document = self.document.is_some();
        let document_changed = self
            .document
            .as_ref()
            .is_some_and(|current| current != &document);
        if self.document.as_ref() != Some(&document) {
            self.document = Some(document);
            self.next_ref = 1;
            self.stable_to_short.clear();
            self.short_to_legacy.clear();
            self.items.clear();
            self.snapshot_revision.clear();
        }
        if self.next_ref == 0 {
            self.next_ref = 1;
        }

        let previous = self.items.clone();
        let snapshot_revision = snapshot_revision.into();
        let stable_keys = stable_item_keys(&items, &snapshot_revision);
        self.short_to_legacy.clear();
        self.items.clear();
        let mut public_items = Vec::with_capacity(items.len());

        for (index, (mut item, stable_key)) in items.into_iter().zip(stable_keys).enumerate() {
            let legacy_reference = item
                .get("reference")
                .and_then(Value::as_str)
                .ok_or(SemanticStateError::MissingRetainedReference)?
                .to_owned();
            let short = if let Some(existing) = self.stable_to_short.get(&stable_key) {
                existing.clone()
            } else {
                let reference = format!("@c{}", self.next_ref);
                self.next_ref = self.next_ref.saturating_add(1);
                self.stable_to_short.insert(stable_key, reference.clone());
                reference
            };
            item["reference"] = json!(short);
            item["index"] = json!(index);
            self.short_to_legacy.insert(short.clone(), legacy_reference);
            self.items.insert(short, item.clone());
            public_items.push(item);
        }

        let upsert = self
            .items
            .iter()
            .filter_map(|(reference, item)| match previous.get(reference) {
                Some(old) if !changed_fields(old, item) => None,
                _ => Some(item.clone()),
            })
            .collect();
        let removed = previous
            .keys()
            .filter(|reference| !self.items.contains_key(*reference))
            .cloned()
            .collect();
        self.snapshot_revision = snapshot_revision;
        Ok(SemanticSnapshotUpdate {
            snapshot_revision: self.snapshot_revision.clone(),
            items: public_items,
            delta: SnapshotDelta {
                full: !had_document || document_changed,
                document_changed,
                upsert,
                removed,
            },
        })
    }
}

pub fn stable_item_keys(items: &[Value], revision: &str) -> Vec<String> {
    let bases = items
        .iter()
        .map(|item| {
            let field = |name| item.get(name).and_then(Value::as_str).unwrap_or_default();
            let ancestor = item
                .pointer("/semantic_ancestor/id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let structure = item
                .get("dom_path_fingerprint")
                .and_then(Value::as_str)
                .unwrap_or_default();
            format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
                field("source_frame"),
                field("source_frame_revision"),
                field("role"),
                field("name"),
                field("href"),
                field("datetime"),
                ancestor,
                structure,
                field("editable_kind"),
                item.get("multiline")
                    .and_then(Value::as_bool)
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                item.get("requires_trusted_events")
                    .and_then(Value::as_bool)
                    .map(|v| v.to_string())
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    let mut counts = BTreeMap::<String, usize>::new();
    for base in &bases {
        *counts.entry(base.clone()).or_default() += 1;
    }
    let mut seen = BTreeMap::<String, usize>::new();
    bases
        .into_iter()
        .map(|base| {
            let ordinal = seen.entry(base.clone()).or_default();
            // Duplicate controls with no distinguishing evidence cannot retain identity across snapshots.
            let key = if counts[base.as_str()] > 1 {
                format!("{base}\u{1f}{revision}\u{1f}{ordinal}")
            } else {
                base
            };
            *ordinal += 1;
            key
        })
        .collect()
}

fn changed_fields(previous: &Value, current: &Value) -> bool {
    [
        "role",
        "name",
        "raw_value",
        "href",
        "expanded",
        "selected",
        "disabled",
        "present",
        "rendered",
        "in_viewport",
        "pointer_events_none",
        "actionability",
        "editable_kind",
        "multiline",
        "requires_trusted_events",
    ]
    .into_iter()
    .any(|field| previous.get(field) != current.get(field))
}

pub fn choose_newest(items: &[Value], coverage_complete: bool) -> Result<Value, &'static str> {
    if !coverage_complete || items.is_empty() || items.len() > 200 {
        return Err("unresolved_latest");
    }
    let mut ranked = items
        .iter()
        .map(|item| {
            let time = item
                .get("datetime_utc_ms")
                .and_then(Value::as_i64)
                .ok_or("unresolved_latest")?;
            if item
                .get("href")
                .and_then(Value::as_str)
                .filter(|href| !href.is_empty())
                .is_none()
            {
                return Err("unresolved_latest");
            }
            Ok((time, item))
        })
        .collect::<Result<Vec<_>, _>>()?;
    ranked.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
    let ties = ranked
        .iter()
        .take_while(|(time, _)| *time == ranked[0].0)
        .map(|(_, item)| item["href"].clone())
        .collect::<Vec<_>>();
    Ok(
        json!({"selected":ranked[0].1,"freshness_evidence":{"datetime":ranked[0].1["datetime"],"utc_ms":ranked[0].0,"candidate_count":items.len(),"basis":"machine_readable_datetime","unique_latest":ties.len()==1,"tie_count":ties.len(),"equally_newest_hrefs":ties,"tie_break":"page_order"}}),
    )
}

pub fn rank_candidates(
    items: impl Iterator<Item = Value>,
    query: &str,
    role: Option<&str>,
    href_hint: Option<&str>,
    ancestor_id: Option<&str>,
    exact: bool,
) -> Vec<Value> {
    let query = query.trim().to_lowercase();
    let mut ranked = items.filter_map(|mut item| {
        let name = item["name"].as_str().unwrap_or_default().to_lowercase();
        let href = item["href"].as_str().unwrap_or_default();
        let raw_value = item["raw_value"].as_str().unwrap_or_default().to_lowercase();
        if role.is_some_and(|wanted| item["role"].as_str() != Some(wanted))
            || href_hint.is_some_and(|wanted| !href.contains(wanted))
            || ancestor_id.is_some_and(|wanted| item.pointer("/semantic_ancestor/id").and_then(Value::as_str) != Some(wanted))
        { return None; }
        let exact_name = name == query;
        if exact && !exact_name { return None; }
        let contained = name.contains(&query);
        let href_match = href.to_lowercase().contains(&query);
        let value_match = raw_value.contains(&query);
        if !exact_name && !contained && !href_match && !value_match { return None; }
        let name_score = if exact_name { 100 } else if contained { 60 } else { 0 };
        let href_score = if href_match { 20 } else { 0 };
        let role_score = if role.is_some() { 10 } else { 0 };
        let hint_score = if href_hint.is_some() { 15 } else { 0 };
        let in_viewport_score = if item["in_viewport"] == true || item["visible"] == true { 5 } else { 0 };
        let score = name_score + href_score + role_score + hint_score + in_viewport_score;
        item["score"] = json!(score);
        item["score_components"] = json!({"name":name_score,"href":href_score,"role":role_score,"href_hint":hint_score,"in_viewport":in_viewport_score});
        Some(item)
    }).collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        b["score"]
            .as_i64()
            .cmp(&a["score"].as_i64())
            .then_with(|| a["reference"].as_str().cmp(&b["reference"].as_str()))
    });
    let top = ranked
        .first()
        .and_then(|item| item["score"].as_i64())
        .unwrap_or(0);
    let second = ranked
        .get(1)
        .and_then(|item| item["score"].as_i64())
        .unwrap_or(0);
    for item in &mut ranked {
        item["confidence"] = json!(if item["score"].as_i64() == Some(top)
            && top >= 100
            && top - second >= 20
        {
            "high"
        } else if item["score"].as_i64().unwrap_or(0) >= 60 {
            "medium"
        } else {
            "low"
        });
    }
    ranked
}

pub fn workflow_find_needs_refresh(found: &Value) -> bool {
    found["matches"].as_array().is_some_and(Vec::is_empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(url: &str) -> SemanticDocumentKey {
        SemanticDocumentKey {
            frame_id: "frame".into(),
            loader_id: format!("loader:{url}"),
            url: url.into(),
            frame_revision: Some("7".into()),
            document_revision: Some("11".into()),
        }
    }

    #[test]
    fn workflow_refreshes_only_after_an_empty_find_result() {
        assert!(workflow_find_needs_refresh(&json!({"matches":[]})));
        assert!(!workflow_find_needs_refresh(
            &json!({"matches":[{"reference":"@c1"}]})
        ));
        assert!(!workflow_find_needs_refresh(
            &json!({"error":"invalid_selector"})
        ));
    }

    #[test]
    fn find_matches_current_value() {
        let found = rank_candidates(vec![json!({"reference":"@c1","role":"textbox","name":"Email","raw_value":"user@example.test","visible":true})].into_iter(), "example.test", Some("textbox"), None, None, false);
        assert_eq!(found[0]["reference"], "@c1");
    }

    #[test]
    fn refs_survive_value_changes_but_delta_reports_changed_value() {
        let mut memory = SemanticMemory::default();
        let first = memory
            .ingest(
                document("https://example.test/form"),
                "s1",
                vec![
                    json!({"reference":"legacy:0","role":"textbox","name":"Email","raw_value":""}),
                ],
            )
            .unwrap();
        assert_eq!(first.items[0]["reference"], "@c1");
        assert!(first.delta.full);
        let second = memory
            .ingest(
                document("https://example.test/form"),
                "s2",
                vec![json!({"reference":"legacy2:0","role":"textbox","name":"Email","raw_value":"a@b.test"})],
            )
            .unwrap();
        assert_eq!(second.items[0]["reference"], "@c1");
        assert_eq!(second.delta.upsert.len(), 1);
        assert!(!second.delta.full);
        assert_eq!(memory.resolve("@c1").unwrap(), "legacy2:0");
    }

    #[test]
    fn document_change_invalidates_old_refs_and_restarts_namespace() {
        let mut memory = SemanticMemory::default();
        memory
            .ingest(
                document("https://example.test/a"),
                "s1",
                vec![json!({"reference":"old:0","role":"button","name":"Save"})],
            )
            .unwrap();
        let update = memory
            .ingest(
                document("https://example.test/b"),
                "s2",
                vec![json!({"reference":"new:0","role":"button","name":"Continue"})],
            )
            .unwrap();
        assert!(update.delta.document_changed);
        assert!(update.delta.full);
        assert_eq!(update.items[0]["reference"], "@c1");
        assert_eq!(memory.resolve("@c1").unwrap(), "new:0");
        assert!(memory.resolve("@c2").is_err());
    }

    #[test]
    fn duplicate_controls_are_disambiguated_without_mutable_values() {
        let mut memory = SemanticMemory::default();
        let first = memory
            .ingest(
                document("https://example.test/form"),
                "s1",
                vec![
                    json!({"reference":"l:0","role":"textbox","name":"Item","raw_value":"a"}),
                    json!({"reference":"l:1","role":"textbox","name":"Item","raw_value":"b"}),
                ],
            )
            .unwrap();
        let refs = first
            .items
            .iter()
            .map(|item| item["reference"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let second = memory
            .ingest(
                document("https://example.test/form"),
                "s2",
                vec![
                    json!({"reference":"n:0","role":"textbox","name":"Item","raw_value":"new-a"}),
                    json!({"reference":"n:1","role":"textbox","name":"Item","raw_value":"new-b"}),
                ],
            )
            .unwrap();
        assert_ne!(refs[0], second.items[0]["reference"]);
        assert_ne!(refs[1], second.items[1]["reference"]);
        assert_ne!(refs[0], refs[1]);
    }

    #[test]
    fn editor_refs_use_structure_and_survive_text_changes() {
        let mut memory = SemanticMemory::default();
        let editors = |refs: [&str; 2], values: [&str; 2]| {
            vec![
                json!({"reference":refs[0],"role":"textbox","name":"Editor","raw_value":values[0],"editable_kind":"rich_text","multiline":true,"dom_path_fingerprint":"div:0>div:0"}),
                json!({"reference":refs[1],"role":"textbox","name":"Editor","raw_value":values[1],"editable_kind":"rich_text","multiline":true,"dom_path_fingerprint":"div:0>div:1"}),
            ]
        };
        let first = memory
            .ingest(
                document("https://example.test/form"),
                "s1",
                editors(["a:0", "a:1"], ["", ""]),
            )
            .unwrap();
        assert_ne!(first.items[0]["reference"], first.items[1]["reference"]);
        let second = memory
            .ingest(
                document("https://example.test/form"),
                "s2",
                editors(["b:0", "b:1"], ["Draft one", "Draft two"]),
            )
            .unwrap();
        assert_eq!(first.items[0]["reference"], second.items[0]["reference"]);
        assert_eq!(first.items[1]["reference"], second.items[1]["reference"]);
    }

    #[test]
    fn removed_controls_are_explicit_in_delta() {
        let mut memory = SemanticMemory::default();
        memory
            .ingest(
                document("https://example.test/form"),
                "s1",
                vec![
                    json!({"reference":"l:0","role":"button","name":"One"}),
                    json!({"reference":"l:1","role":"button","name":"Two"}),
                ],
            )
            .unwrap();
        let update = memory
            .ingest(
                document("https://example.test/form"),
                "s2",
                vec![json!({"reference":"n:0","role":"button","name":"One"})],
            )
            .unwrap();
        assert_eq!(update.delta.removed, vec!["@c2"]);
    }

    #[test]
    fn reordered_duplicate_cards_never_keep_each_others_reference() {
        let mut memory = SemanticMemory::default();
        let first = memory.ingest(document("https://example.test/news"), "s1", vec![
            json!({"reference":"old:0","role":"link","name":"Story","href":"/story","semantic_ancestor":{"id":"card-a"}}),
            json!({"reference":"old:1","role":"link","name":"Story","href":"/story","semantic_ancestor":{"id":"card-b"}}),
        ]).unwrap();
        let second = memory.ingest(document("https://example.test/news"), "s2", vec![
            json!({"reference":"new:0","role":"link","name":"Story","href":"/story","semantic_ancestor":{"id":"card-b"}}),
            json!({"reference":"new:1","role":"link","name":"Story","href":"/story","semantic_ancestor":{"id":"card-a"}}),
        ]).unwrap();
        assert_eq!(first.items[0]["reference"], second.items[1]["reference"]);
        assert_eq!(first.items[1]["reference"], second.items[0]["reference"]);
    }

    #[test]
    fn newest_selection_requires_complete_unambiguous_timestamps() {
        let items = vec![
            json!({"reference":"@c1","href":"/old","datetime":"2026-10-09T00:00:00Z","datetime_utc_ms":1791504000000_i64}),
            json!({"reference":"@c2","href":"/new","datetime":"2026-10-10T00:00:00Z","datetime_utc_ms":1791590400000_i64}),
        ];
        assert_eq!(
            choose_newest(&items, true).unwrap()["selected"]["reference"],
            "@c2"
        );
        assert!(choose_newest(&items, false).is_err());
        let mut missing = items.clone();
        missing[0]
            .as_object_mut()
            .unwrap()
            .remove("datetime_utc_ms");
        assert!(choose_newest(&missing, true).is_err());
        let mut tie = items.clone();
        tie[0]["datetime_utc_ms"] = tie[1]["datetime_utc_ms"].clone();
        let chosen = choose_newest(&tie, true).unwrap();
        assert_eq!(chosen["selected"]["reference"], "@c1");
        assert_eq!(chosen["freshness_evidence"]["tie_count"], 2);
        assert_eq!(chosen["freshness_evidence"]["unique_latest"], false);
    }

    #[test]
    fn ranking_keeps_ambiguous_titles_separate_until_href_hint_resolves_them() {
        let items = vec![
            json!({"reference":"@c1","role":"link","name":"Bills story","href":"/story/old","visible":true}),
            json!({"reference":"@c2","role":"link","name":"Bills story","href":"/story/new","visible":true}),
            json!({"reference":"@c3","role":"link","name":"Bills gamecast","href":"/game/3","visible":true}),
        ];
        let ambiguous = rank_candidates(
            items.clone().into_iter(),
            "Bills story",
            Some("link"),
            None,
            None,
            false,
        );
        assert_eq!(ambiguous.len(), 2);
        assert_eq!(ambiguous[0]["confidence"], "medium");
        let resolved = rank_candidates(
            items.into_iter(),
            "Bills story",
            Some("link"),
            Some("/story/new"),
            None,
            false,
        );
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0]["reference"], "@c2");
        assert_eq!(resolved[0]["confidence"], "high");
    }
}
