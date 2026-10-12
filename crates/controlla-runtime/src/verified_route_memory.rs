//! Persistent qualified direct-route memory for low-latency public navigation.
//!
//! A route is replayable only after distinct training and validation successes
//! plus an independent runtime verification. Every replay still requires a
//! cheap page-identity verification. Structural drift, safety failures, expiry,
//! and poor rolling verification quarantine the fast path.

use crate::verifier::Verification;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

pub const ROUTE_SCHEMA_VERSION: u32 = 1;
const OUTCOME_WINDOW: usize = 20;
const MIN_PASS_PERCENT: usize = 80;
const MAX_ROUTES: usize = 2048;
const MAX_STORE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteStatus {
    Candidate,
    Qualified,
    Quarantined,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteDefinition {
    pub route_id: String,
    pub schema_version: u32,
    /// Exact origin, such as `https://www.espn.com`.
    pub site_scope: String,
    /// Stable task/entity fingerprint, such as `espn:nfl:team:buf`.
    pub intent_fingerprint: String,
    /// Exact verified public read destination.
    pub destination_url: String,
    /// Entity/page identity signatures used by the independent verifier.
    pub structural_signatures: Vec<String>,
    pub version: u32,
}

impl RouteDefinition {
    pub fn valid(&self) -> bool {
        self.schema_version == ROUTE_SCHEMA_VERSION
            && self.version > 0
            && bounded_text(&self.route_id, 256)
            && bounded_text(&self.intent_fingerprint, 512)
            && !self.structural_signatures.is_empty()
            && self.structural_signatures.len() <= 32
            && self
                .structural_signatures
                .iter()
                .all(|value| bounded_text(value, 1024))
            && public_read_route(&self.site_scope, &self.destination_url)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteRecord {
    pub definition: RouteDefinition,
    pub status: RouteStatus,
    pub training_run_ids: BTreeSet<String>,
    pub validation_run_ids: BTreeSet<String>,
    pub recent_outcomes: VecDeque<bool>,
    pub qualified_at_ms: Option<u64>,
    pub expires_at_ms: Option<u64>,
    pub quarantine_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RouteResolution {
    pub route_id: String,
    pub destination_url: String,
    pub version: u32,
    pub verification_required: bool,
    pub internal_model_calls: u32,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum RouteMemoryError {
    #[error("route definition is invalid or unsafe for public direct navigation")]
    InvalidDefinition,
    #[error("training and validation require distinct successful runs")]
    MissingQualificationRuns,
    #[error("runtime verification did not pass")]
    RuntimeVerification,
    #[error("qualification expiry must follow qualification time")]
    InvalidExpiry,
    #[error("route memory I/O failed: {0}")]
    Io(String),
    #[error("route memory JSON is invalid: {0}")]
    Json(String),
    #[error("route memory exceeds its bounded capacity")]
    Capacity,
    #[error("route memory lock is poisoned")]
    LockPoisoned,
    #[error("unknown route_id")]
    NotFound,
}

impl RouteRecord {
    pub fn candidate(definition: RouteDefinition) -> Result<Self, RouteMemoryError> {
        if !definition.valid() {
            return Err(RouteMemoryError::InvalidDefinition);
        }
        Ok(Self {
            definition,
            status: RouteStatus::Candidate,
            training_run_ids: BTreeSet::new(),
            validation_run_ids: BTreeSet::new(),
            recent_outcomes: VecDeque::with_capacity(OUTCOME_WINDOW),
            qualified_at_ms: None,
            expires_at_ms: None,
            quarantine_reason: None,
        })
    }

    pub fn record_training_success(&mut self, run_id: impl Into<String>) {
        let run_id = run_id.into();
        if bounded_text(&run_id, 256) {
            self.training_run_ids.insert(run_id);
        }
    }

    pub fn record_validation_success(&mut self, run_id: impl Into<String>) {
        let run_id = run_id.into();
        if bounded_text(&run_id, 256) {
            self.validation_run_ids.insert(run_id);
        }
    }

    pub fn qualify(
        &mut self,
        verification: Verification,
        qualified_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<(), RouteMemoryError> {
        if expires_at_ms <= qualified_at_ms {
            return Err(RouteMemoryError::InvalidExpiry);
        }
        let distinct_runs = self.training_run_ids.iter().any(|training| {
            self.validation_run_ids
                .iter()
                .any(|validation| validation != training)
        });
        if !distinct_runs {
            return Err(RouteMemoryError::MissingQualificationRuns);
        }
        if verification != Verification::Passed {
            return Err(RouteMemoryError::RuntimeVerification);
        }
        self.status = RouteStatus::Qualified;
        self.qualified_at_ms = Some(qualified_at_ms);
        self.expires_at_ms = Some(expires_at_ms);
        self.quarantine_reason = None;
        Ok(())
    }

    pub fn replay_allowed(&mut self, now_ms: u64) -> bool {
        if self.status != RouteStatus::Qualified {
            return false;
        }
        match (self.qualified_at_ms, self.expires_at_ms) {
            (Some(start), Some(end)) if now_ms >= start && now_ms < end => true,
            _ => {
                self.status = RouteStatus::Expired;
                false
            }
        }
    }

    pub fn record_runtime_verification(
        &mut self,
        verification: Verification,
        severe_safety_failure: bool,
        structural_drift: bool,
    ) {
        if severe_safety_failure {
            self.quarantine("severe safety failure");
            return;
        }
        if structural_drift {
            self.quarantine("structural drift");
            return;
        }
        if self.recent_outcomes.len() == OUTCOME_WINDOW {
            self.recent_outcomes.pop_front();
        }
        self.recent_outcomes
            .push_back(verification == Verification::Passed);
        if self.recent_outcomes.len() == OUTCOME_WINDOW {
            let passed = self.recent_outcomes.iter().filter(|value| **value).count();
            if passed * 100 < MIN_PASS_PERCENT * OUTCOME_WINDOW {
                self.quarantine("rolling route verification pass rate below 80 percent");
            }
        }
    }

    fn quarantine(&mut self, reason: impl Into<String>) {
        self.status = RouteStatus::Quarantined;
        self.quarantine_reason = Some(reason.into());
    }
}

#[derive(Clone, Debug)]
pub struct RouteMemory {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl RouteMemory {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> Result<MutexGuard<'_, ()>, RouteMemoryError> {
        self.lock.lock().map_err(|_| RouteMemoryError::LockPoisoned)
    }

    fn load_unlocked(&self) -> Result<BTreeMap<String, RouteRecord>, RouteMemoryError> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        let metadata = std::fs::metadata(&self.path)
            .map_err(|error| RouteMemoryError::Io(error.to_string()))?;
        if metadata.len() > MAX_STORE_BYTES {
            return Err(RouteMemoryError::Capacity);
        }
        let bytes =
            std::fs::read(&self.path).map_err(|error| RouteMemoryError::Io(error.to_string()))?;
        let records = serde_json::from_slice::<BTreeMap<String, RouteRecord>>(&bytes)
            .map_err(|error| RouteMemoryError::Json(error.to_string()))?;
        if records.len() > MAX_ROUTES {
            return Err(RouteMemoryError::Capacity);
        }
        Ok(records)
    }

    fn save_unlocked(
        &self,
        records: &BTreeMap<String, RouteRecord>,
    ) -> Result<(), RouteMemoryError> {
        if records.len() > MAX_ROUTES {
            return Err(RouteMemoryError::Capacity);
        }
        let bytes = serde_json::to_vec_pretty(records)
            .map_err(|error| RouteMemoryError::Json(error.to_string()))?;
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(RouteMemoryError::Capacity);
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| RouteMemoryError::Io(error.to_string()))?;
        }
        let temp = self
            .path
            .with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&temp, bytes).map_err(|error| RouteMemoryError::Io(error.to_string()))?;
        if let Err(rename_error) = std::fs::rename(&temp, &self.path) {
            if self.path.exists() {
                std::fs::remove_file(&self.path)
                    .map_err(|error| RouteMemoryError::Io(error.to_string()))?;
                std::fs::rename(&temp, &self.path)
                    .map_err(|error| RouteMemoryError::Io(error.to_string()))?;
            } else {
                return Err(RouteMemoryError::Io(rename_error.to_string()));
            }
        }
        Ok(())
    }

    pub fn register_candidate(
        &self,
        definition: RouteDefinition,
    ) -> Result<RouteRecord, RouteMemoryError> {
        let _guard = self.lock()?;
        let record = RouteRecord::candidate(definition)?;
        let mut records = self.load_unlocked()?;
        if !records.contains_key(&record.definition.route_id) && records.len() >= MAX_ROUTES {
            return Err(RouteMemoryError::Capacity);
        }
        records.insert(record.definition.route_id.clone(), record.clone());
        self.save_unlocked(&records)?;
        Ok(record)
    }

    pub fn record_training_success(
        &self,
        route_id: &str,
        run_id: impl Into<String>,
    ) -> Result<RouteRecord, RouteMemoryError> {
        self.update(route_id, |record| {
            record.record_training_success(run_id);
            Ok(())
        })
    }

    pub fn record_validation_success(
        &self,
        route_id: &str,
        run_id: impl Into<String>,
    ) -> Result<RouteRecord, RouteMemoryError> {
        self.update(route_id, |record| {
            record.record_validation_success(run_id);
            Ok(())
        })
    }

    pub fn qualify(
        &self,
        route_id: &str,
        verification: Verification,
        qualified_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<RouteRecord, RouteMemoryError> {
        self.update(route_id, |record| {
            record.qualify(verification, qualified_at_ms, expires_at_ms)
        })
    }

    /// Resolve a qualified route with zero internal model calls. The browser
    /// caller must verify page identity after navigation before trusting it.
    pub fn resolve(
        &self,
        site_scope: &str,
        intent_fingerprint: &str,
        now_ms: u64,
    ) -> Result<Option<RouteResolution>, RouteMemoryError> {
        let _guard = self.lock()?;
        let mut records = self.load_unlocked()?;
        let mut changed = false;
        let mut candidates = Vec::new();
        for record in records.values_mut() {
            if record.definition.site_scope != site_scope
                || record.definition.intent_fingerprint != intent_fingerprint
            {
                continue;
            }
            let before = record.status;
            if record.replay_allowed(now_ms) {
                candidates.push(record.clone());
            }
            changed |= before != record.status;
        }
        if changed {
            self.save_unlocked(&records)?;
        }
        candidates.sort_by_key(|record| {
            (
                std::cmp::Reverse(record.definition.version),
                std::cmp::Reverse(record.qualified_at_ms.unwrap_or_default()),
                record.definition.route_id.clone(),
            )
        });
        Ok(candidates.into_iter().next().map(|record| RouteResolution {
            route_id: record.definition.route_id,
            destination_url: record.definition.destination_url,
            version: record.definition.version,
            verification_required: true,
            internal_model_calls: 0,
        }))
    }

    pub fn record_runtime_verification(
        &self,
        route_id: &str,
        verification: Verification,
        severe_safety_failure: bool,
        structural_drift: bool,
    ) -> Result<RouteRecord, RouteMemoryError> {
        self.update(route_id, |record| {
            record.record_runtime_verification(
                verification,
                severe_safety_failure,
                structural_drift,
            );
            Ok(())
        })
    }

    fn update(
        &self,
        route_id: &str,
        update: impl FnOnce(&mut RouteRecord) -> Result<(), RouteMemoryError>,
    ) -> Result<RouteRecord, RouteMemoryError> {
        let _guard = self.lock()?;
        let mut records = self.load_unlocked()?;
        let record = records
            .get_mut(route_id)
            .ok_or(RouteMemoryError::NotFound)?;
        update(record)?;
        let result = record.clone();
        self.save_unlocked(&records)?;
        Ok(result)
    }
}

fn bounded_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn public_read_route(site_scope: &str, destination: &str) -> bool {
    let (Some(scope_origin), Some(destination_origin)) =
        (http_origin(site_scope), http_origin(destination))
    else {
        return false;
    };
    if scope_origin != destination_origin || has_sensitive_query(destination) {
        return false;
    }
    let path = destination
        .split_once("//")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.find('/').map(|index| &rest[index..]))
        .unwrap_or("/")
        .split(['?', '#'])
        .next()
        .unwrap_or("/")
        .to_ascii_lowercase();
    ![
        "/submit",
        "/delete",
        "/remove",
        "/purchase",
        "/checkout",
        "/logout",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
}

fn http_origin(value: &str) -> Option<String> {
    if value.len() > 4096 || value.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    let (scheme, rest) = if let Some(rest) = lower.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = lower.strip_prefix("http://") {
        ("http", rest)
    } else {
        return None;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

fn has_sensitive_query(value: &str) -> bool {
    value
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default()
        .split('&')
        .map(|part| part.split_once('=').map_or(part, |(key, _)| key))
        .map(str::to_ascii_lowercase)
        .any(|key| {
            matches!(
                key.as_str(),
                "token"
                    | "auth"
                    | "password"
                    | "passwd"
                    | "session"
                    | "sessionid"
                    | "api_key"
                    | "apikey"
                    | "code"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn memory() -> RouteMemory {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        RouteMemory::new(std::env::temp_dir().join(format!(
            "controlla-route-memory-{}-{nonce}/routes.json",
            std::process::id()
        )))
    }

    fn definition() -> RouteDefinition {
        RouteDefinition {
            route_id: "espn-bills".into(),
            schema_version: ROUTE_SCHEMA_VERSION,
            site_scope: "https://www.espn.com".into(),
            intent_fingerprint: "espn:nfl:team:buf".into(),
            destination_url: "https://www.espn.com/nfl/team/_/name/buf/buffalo-bills".into(),
            structural_signatures: vec!["heading:Buffalo Bills".into()],
            version: 1,
        }
    }

    fn qualify(memory: &RouteMemory) {
        memory.register_candidate(definition()).unwrap();
        memory
            .record_training_success("espn-bills", "train-1")
            .unwrap();
        memory
            .record_validation_success("espn-bills", "validate-1")
            .unwrap();
        memory
            .qualify("espn-bills", Verification::Passed, 100, 1000)
            .unwrap();
    }

    #[test]
    fn qualified_route_resolves_without_model_calls_and_still_requires_verification() {
        let memory = memory();
        qualify(&memory);
        let route = memory
            .resolve("https://www.espn.com", "espn:nfl:team:buf", 200)
            .unwrap()
            .unwrap();
        assert_eq!(route.internal_model_calls, 0);
        assert!(route.verification_required);
        assert!(route.destination_url.contains("buffalo-bills"));
        let _ = std::fs::remove_dir_all(memory.path().parent().unwrap());
    }

    #[test]
    fn qualification_requires_distinct_runs() {
        let memory = memory();
        memory.register_candidate(definition()).unwrap();
        memory
            .record_training_success("espn-bills", "same")
            .unwrap();
        memory
            .record_validation_success("espn-bills", "same")
            .unwrap();
        assert_eq!(
            memory.qualify("espn-bills", Verification::Passed, 100, 1000),
            Err(RouteMemoryError::MissingQualificationRuns)
        );
        let _ = std::fs::remove_dir_all(memory.path().parent().unwrap());
    }

    #[test]
    fn unsafe_routes_are_rejected() {
        for destination in [
            "https://evil.example/nfl/team/_/name/buf/buffalo-bills",
            "https://www.espn.com/account?token=secret",
            "https://www.espn.com/api/delete",
            "https://user:pass@www.espn.com/nfl/team/_/name/buf/buffalo-bills",
        ] {
            let mut candidate = definition();
            candidate.destination_url = destination.into();
            assert_eq!(
                RouteRecord::candidate(candidate),
                Err(RouteMemoryError::InvalidDefinition)
            );
        }
    }

    #[test]
    fn structural_drift_quarantines_and_removes_fast_path() {
        let memory = memory();
        qualify(&memory);
        let record = memory
            .record_runtime_verification("espn-bills", Verification::Failed, false, true)
            .unwrap();
        assert_eq!(record.status, RouteStatus::Quarantined);
        assert!(
            memory
                .resolve("https://www.espn.com", "espn:nfl:team:buf", 300)
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(memory.path().parent().unwrap());
    }
}
