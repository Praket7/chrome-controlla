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
        let stable_keys = stable_item_keys(&items);
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
                self.stable_to_short
                    .insert(stable_key, reference.clone());
                reference
            };
            item["reference"] = json!(short);
            item["index"] = json!(index);
            self.short_to_legacy
                .insert(short.clone(), legacy_reference);
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
        self.snapshot_revision = snapshot_revision.into();
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

fn stable_item_keys(items: &[Value]) -> Vec<String> {
    let mut seen = BTreeMap::<String, usize>::new();
    items
        .iter()
        .map(|item| {
            let role = item.get("role").and_then(Value::as_str).unwrap_or_default();
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            let href = item.get("href").and_then(Value::as_str).unwrap_or_default();
            let base = format!("{role}\u{1f}{name}\u{1f}{href}");
            let ordinal = seen.entry(base.clone()).or_default();
            let key = format!("{base}\u{1f}{ordinal}");
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
    ]
    .into_iter()
    .any(|field| previous.get(field) != current.get(field))
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
    fn refs_survive_value_changes_but_delta_reports_changed_value() {
        let mut memory = SemanticMemory::default();
        let first = memory
            .ingest(
                document("https://example.test/form"),
                "s1",
                vec![json!({"reference":"legacy:0","role":"textbox","name":"Email","raw_value":""})],
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
        assert_eq!(refs[0], second.items[0]["reference"]);
        assert_eq!(refs[1], second.items[1]["reference"]);
        assert_ne!(refs[0], refs[1]);
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
}
