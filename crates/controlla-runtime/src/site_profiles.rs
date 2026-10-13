//! Qualified, read-only URL templates for speculative research warm starts.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const MAX_PROFILES: usize = 256;
pub const MAX_PROFILE_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SiteProfile {
    pub id: String,
    pub origin: String,
    pub intent: String,
    #[serde(default)]
    pub semantic_query: String,
    pub url_template: String,
    pub result_schema: Value,
    pub structural_signature: String,
    pub qualified_at_ms: u64,
    pub expires_at_ms: u64,
    pub independent_successes: u8,
}

#[derive(Clone, Debug, Default)]
pub struct SiteProfileStore {
    profiles: BTreeMap<String, SiteProfile>,
    training: BTreeMap<String, std::collections::BTreeSet<String>>,
    path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Speculation {
    Unavailable,
    Candidate { url: String },
    Verified { url: String, result: Value },
    WarmStart { url: String, reason: &'static str },
}

impl SiteProfileStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let profiles = match std::fs::read(&path) {
            Ok(bytes) if bytes.len() <= 1024 * 1024 => {
                serde_json::from_slice::<BTreeMap<String, SiteProfile>>(&bytes)
                    .map_err(|e| format!("site profile store is invalid: {e}"))?
            }
            Ok(_) => return Err("site profile store exceeds 1 MiB".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(format!("cannot read site profile store: {error}")),
        };
        if profiles.len() > MAX_PROFILES
            || profiles
                .iter()
                .any(|(id, p)| id != &p.id || p.independent_successes < 3)
        {
            return Err("site profile store failed qualification validation".into());
        }
        Ok(Self {
            profiles,
            training: BTreeMap::new(),
            path: Some(path),
        })
    }

    pub fn qualify(&mut self, profile: SiteProfile) -> Result<(), &'static str> {
        if profile.id.is_empty()
            || profile.id.len() > 128
            || profile.origin.len() > 512
            || profile.intent.is_empty()
            || profile.intent.len() > 256
            || profile.semantic_query.trim().is_empty()
            || profile.semantic_query.len() > 512
            || profile.url_template.len() > 2048
            || !profile.url_template.starts_with("https://")
            || url::Url::parse(&profile.origin).is_err()
            || url::Url::parse(&profile.origin).is_ok_and(|u| {
                u.scheme() != "https" || u.origin().ascii_serialization() != profile.origin
            })
            || profile.result_schema.as_object().is_none()
            || profile.structural_signature.is_empty()
            || profile.structural_signature.len() > 256
            || profile.independent_successes < 3
            || profile.expires_at_ms <= profile.qualified_at_ms
            || self.profiles.len() >= MAX_PROFILES && !self.profiles.contains_key(&profile.id)
        {
            return Err(
                "site profile does not meet bounded independent qualification requirements",
            );
        }
        self.profiles.insert(profile.id.clone(), profile);
        self.persist()?;
        Ok(())
    }

    fn persist(&self) -> Result<(), &'static str> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = path.parent().ok_or("site profile path has no parent")?;
        std::fs::create_dir_all(parent).map_err(|_| "cannot create site profile directory")?;
        let bytes =
            serde_json::to_vec(&self.profiles).map_err(|_| "cannot serialize site profiles")?;
        if bytes.len() > 1024 * 1024 {
            return Err("site profile store exceeds 1 MiB");
        }
        let mut temp = path.clone();
        temp.set_extension("tmp");
        std::fs::write(&temp, &bytes).map_err(|_| "cannot write site profile temporary file")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "cannot restrict site profile permissions")?;
        }
        std::fs::rename(&temp, path).map_err(|_| "cannot commit site profile store")?;
        Ok(())
    }

    pub fn candidate_for_intent(
        &self,
        origin: &str,
        intent: &str,
        now_ms: u64,
    ) -> Option<&SiteProfile> {
        self.profiles.values().find(|p| {
            p.origin == origin
                && p.intent == intent
                && p.qualified_at_ms <= now_ms
                && now_ms < p.expires_at_ms
                && now_ms.saturating_sub(p.qualified_at_ms) <= MAX_PROFILE_AGE_MS
        })
    }

    /// Resolve a profile selected by the capability registry while rechecking
    /// its origin, intent, and age at dispatch time.
    pub fn qualified_by_id(
        &self,
        id: &str,
        origin: &str,
        intent: &str,
        now_ms: u64,
    ) -> Option<&SiteProfile> {
        self.profiles.get(id).filter(|p| {
            p.origin == origin
                && p.intent == intent
                && p.qualified_at_ms <= now_ms
                && now_ms < p.expires_at_ms
                && now_ms.saturating_sub(p.qualified_at_ms) <= MAX_PROFILE_AGE_MS
        })
    }

    /// Learn a reusable same-origin operation URL only from distinct, verified
    /// query examples. Query values are retained as hashes in memory only;
    /// the persisted profile contains a `{query}` template, never training text.
    pub fn observe_verified_operation(
        &mut self,
        intent: &str,
        url: &str,
        structural_signature: &str,
        now_ms: u64,
    ) -> Result<Option<SiteProfile>, &'static str> {
        let query_key_is_operation: &[&str] = match intent {
            "search" => &["q", "query", "search", "term"],
            "filter" => &["filter", "category", "date"],
            "sort" => &["sort", "order", "direction"],
            "paginate" => &["page", "offset", "cursor"],
            _ => return Ok(None),
        };
        if structural_signature.is_empty() || structural_signature.len() > 256 {
            return Ok(None);
        }
        let parsed = url::Url::parse(url).map_err(|_| "invalid profile training URL")?;
        if parsed.scheme() != "https"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Ok(None);
        }
        let pairs = parsed.query_pairs().collect::<Vec<_>>();
        if pairs.len() != 1 {
            return Ok(None);
        }
        let (key, value) = (&pairs[0].0, &pairs[0].1);
        if !query_key_is_operation.contains(&key.to_ascii_lowercase().as_str())
            || value.trim().is_empty()
            || value.len() > 512
        {
            return Ok(None);
        }
        let origin = parsed.origin().ascii_serialization();
        let path = parsed.path();
        if path.len() > 512 {
            return Ok(None);
        }
        let template = format!("{origin}{path}?{key}={{query}}");
        if !self.training.contains_key(&format!(
            "{origin}|{intent}|{structural_signature}|{template}"
        )) && self.training.len() >= MAX_PROFILES
        {
            return Ok(None);
        }
        let mut signature = sha2::Sha256::new();
        use sha2::Digest as _;
        signature.update(value.as_bytes());
        let input_hash = signature
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let cohort_key = format!("{origin}|{intent}|{structural_signature}|{template}");
        let samples = self.training.entry(cohort_key.clone()).or_default();
        if samples.len() < 3 {
            samples.insert(input_hash);
        }
        if samples.len() < 3 {
            return Ok(None);
        }
        let mut id_hasher = sha2::Sha256::new();
        id_hasher.update(cohort_key.as_bytes());
        let id = format!(
            "profile-{}",
            id_hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let profile = SiteProfile {
            id,
            origin,
            intent: intent.to_owned(),
            semantic_query: "search".into(),
            url_template: template,
            result_schema: serde_json::json!({"type":"object"}),
            structural_signature: structural_signature.to_owned(),
            qualified_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(MAX_PROFILE_AGE_MS),
            independent_successes: samples.len().min(u8::MAX as usize) as u8,
        };
        self.qualify(profile.clone())?;
        Ok(Some(profile))
    }

    /// Instantiate only a same-origin HTTPS candidate. `{query}` is encoded as
    /// one URL component so task text cannot inject a path, host, or fragment.
    pub fn instantiate(profile: &SiteProfile, query: &str) -> Option<String> {
        let candidate = if profile.url_template.contains("{query}") {
            if query.trim().is_empty() || query.len() > 512 {
                return None;
            }
            profile.url_template.replace(
                "{query}",
                &url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>(),
            )
        } else {
            profile.url_template.clone()
        };
        let base = url::Url::parse(&profile.origin).ok()?;
        let parsed = url::Url::parse(&candidate).ok()?;
        (base.scheme() == "https" && parsed.scheme() == "https" && base.origin() == parsed.origin())
            .then_some(parsed.to_string())
    }

    pub fn candidate(
        &self,
        origin: &str,
        intent: &str,
        structure: &str,
        now_ms: u64,
    ) -> Option<String> {
        self.profiles
            .values()
            .find(|p| {
                p.origin == origin
                    && p.intent == intent
                    && p.structural_signature == structure
                    && p.qualified_at_ms <= now_ms
                    && now_ms < p.expires_at_ms
                    && now_ms.saturating_sub(p.qualified_at_ms) <= MAX_PROFILE_AGE_MS
            })
            .map(|p| p.url_template.clone())
    }

    /// Speculative results become success only after both bounded schema and task verification pass.
    pub fn verify_read(
        profile: &SiteProfile,
        candidate_url: String,
        result: Value,
        semantic_ok: bool,
    ) -> Speculation {
        let origin_prefix = format!("{}/", profile.origin.trim_end_matches('/'));
        if candidate_url != profile.origin && !candidate_url.starts_with(&origin_prefix) {
            return Speculation::WarmStart {
                url: candidate_url,
                reason: "candidate_origin_mismatch",
            };
        }
        if result.to_string().len() > 256 * 1024 {
            return Speculation::WarmStart {
                url: candidate_url,
                reason: "result_exceeded_bound",
            };
        }
        if !schema_matches(&result, &profile.result_schema) {
            return Speculation::WarmStart {
                url: candidate_url,
                reason: "schema_rejected",
            };
        }
        if !semantic_ok {
            return Speculation::WarmStart {
                url: candidate_url,
                reason: "task_verification_rejected",
            };
        }
        Speculation::Verified {
            url: candidate_url,
            result,
        }
    }
}

pub fn schema_matches(value: &Value, schema: &Value) -> bool {
    let type_ok = match schema["type"].as_str() {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("number") => value.is_number(),
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        Some("boolean") => value.is_boolean(),
        Some("null") => value.is_null(),
        _ => false,
    };
    if !type_ok {
        return false;
    }
    if let Some(required) = schema["required"].as_array()
        && required
            .iter()
            .any(|key| key.as_str().is_none_or(|key| value.get(key).is_none()))
    {
        return false;
    }
    if let Some(max) = schema["maxItems"].as_u64()
        && value.as_array().is_some_and(|a| a.len() as u64 > max)
    {
        return false;
    }
    if let Some(props) = schema["properties"].as_object() {
        for (key, subschema) in props {
            if let Some(item) = value.get(key)
                && !schema_matches(item, subschema)
            {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn profile() -> SiteProfile {
        SiteProfile {
            id: "search".into(),
            origin: "https://example.test".into(),
            intent: "latest".into(),
            semantic_query: "Bills story".into(),
            url_template: "https://example.test/search?q={query}".into(),
            result_schema: json!({"type":"object"}),
            structural_signature: "s1".into(),
            qualified_at_ms: 100,
            expires_at_ms: 10_000,
            independent_successes: 3,
        }
    }
    #[test]
    fn profile_requires_multiple_independent_successes_and_exact_structure() {
        let mut store = SiteProfileStore::default();
        let mut p = profile();
        p.independent_successes = 2;
        assert!(store.qualify(p.clone()).is_err());
        p.independent_successes = 3;
        store.qualify(p).unwrap();
        assert!(
            store
                .candidate("https://example.test", "latest", "s1", 101)
                .is_some()
        );
        assert!(
            store
                .candidate("https://example.test", "latest", "drift", 101)
                .is_none()
        );
    }
    #[test]
    fn rejected_speculation_preserves_candidate_as_warm_start() {
        assert_eq!(
            SiteProfileStore::verify_read(
                &profile(),
                "https://example.test/search".into(),
                json!({"ok":true}),
                false
            ),
            Speculation::WarmStart {
                url: "https://example.test/search".into(),
                reason: "task_verification_rejected"
            }
        );
    }

    #[test]
    fn query_template_is_encoded_and_cannot_escape_profile_origin() {
        let p = profile();
        assert_eq!(
            SiteProfileStore::instantiate(&p, "Bills & scores"),
            Some("https://example.test/search?q=Bills+%26+scores".into())
        );
        let mut hostile = p.clone();
        hostile.url_template = "https://attacker.test/?q={query}".into();
        assert_eq!(SiteProfileStore::instantiate(&hostile, "Bills"), None);
        let mut malformed = p;
        malformed.origin = "https://example.test/path".into();
        assert!(SiteProfileStore::default().qualify(malformed).is_err());
    }

    #[test]
    fn stored_profile_is_not_eligible_outside_qualification_window() {
        let mut store = SiteProfileStore::default();
        let mut p = profile();
        p.intent = "research:body".into();
        store.qualify(p).unwrap();
        assert!(
            store
                .candidate_for_intent("https://example.test", "research:body", 100)
                .is_some()
        );
        assert!(
            store
                .candidate_for_intent("https://example.test", "research:body", 10_000)
                .is_none()
        );
    }

    #[test]
    fn registry_selected_profile_is_revalidated_at_dispatch() {
        let mut store = SiteProfileStore::default();
        let mut profile = profile();
        profile.intent = "search".into();
        store.qualify(profile).unwrap();
        assert!(
            store
                .qualified_by_id("search", "https://example.test", "search", 101)
                .is_some()
        );
        assert!(
            store
                .qualified_by_id("search", "https://other.test", "search", 101)
                .is_none()
        );
        assert!(
            store
                .qualified_by_id("search", "https://example.test", "filter", 101)
                .is_none()
        );
        assert!(
            store
                .qualified_by_id("search", "https://example.test", "search", 10_000)
                .is_none()
        );
    }

    #[test]
    fn profile_learning_needs_three_distinct_verified_search_inputs() {
        let mut store = SiteProfileStore::default();
        let observe = |store: &mut SiteProfileStore, query: &str| {
            store.observe_verified_operation(
                "search",
                &format!("https://example.test/find?q={query}"),
                "structure-v1",
                100,
            )
        };
        assert!(observe(&mut store, "alpha").unwrap().is_none());
        assert!(observe(&mut store, "alpha").unwrap().is_none());
        assert!(observe(&mut store, "bravo").unwrap().is_none());
        let learned = observe(&mut store, "charlie").unwrap().unwrap();
        assert_eq!(learned.independent_successes, 3);
        assert_eq!(learned.url_template, "https://example.test/find?q={query}");
        assert!(!learned.url_template.contains("alpha"));
        assert!(
            store
                .candidate_for_intent("https://example.test", "search", 101)
                .is_some()
        );
    }

    #[test]
    fn profile_learning_rejects_unbounded_or_ambiguous_search_urls() {
        let mut store = SiteProfileStore::default();
        assert!(
            store
                .observe_verified_operation("search", "http://example.test/find?q=alpha", "s1", 100)
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .observe_verified_operation(
                    "search",
                    "https://example.test/find?q=alpha&lang=en",
                    "s1",
                    100
                )
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .observe_verified_operation(
                    "search",
                    "https://example.test/find?token=secret",
                    "s1",
                    100
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn profile_learning_uses_operation_specific_query_keys() {
        for (intent, key) in [
            ("search", "q"),
            ("filter", "category"),
            ("sort", "sort"),
            ("paginate", "page"),
        ] {
            let mut store = SiteProfileStore::default();
            for query in ["one", "two", "three"] {
                let url = format!("https://example.test/list?{key}={query}");
                let _ = store
                    .observe_verified_operation(intent, &url, "structure", 100)
                    .unwrap();
            }
            assert!(
                store
                    .candidate_for_intent("https://example.test", intent, 101)
                    .is_some()
            );
        }
    }
}
