//! Controlla v3 execution policy primitives.
//!
//! These types deliberately keep policy separate from transport.  The MCP v3
//! surface and the browser providers both consume the same rules so foreground,
//! background, headless, fast typing, interference handling, and multi-client
//! ownership cannot silently drift apart.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMode {
    Foreground,
    Background,
    Headless,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BrowserModePolicy {
    pub uses_real_profile: bool,
    pub may_activate_window: bool,
    pub requires_visible_window: bool,
    pub semantic_parity_required: bool,
}

impl BrowserMode {
    pub fn policy(self) -> BrowserModePolicy {
        match self {
            Self::Foreground => BrowserModePolicy {
                uses_real_profile: true,
                may_activate_window: true,
                requires_visible_window: true,
                semantic_parity_required: true,
            },
            Self::Background => BrowserModePolicy {
                uses_real_profile: true,
                may_activate_window: false,
                requires_visible_window: true,
                semantic_parity_required: true,
            },
            Self::Headless => BrowserModePolicy {
                uses_real_profile: false,
                may_activate_window: false,
                requires_visible_window: false,
                semantic_parity_required: true,
            },
        }
    }

    pub fn default_provider(self) -> &'static str {
        match self {
            Self::Foreground | Self::Background => "shared_extension",
            Self::Headless => "dedicated_headless",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypingMode {
    /// O(1) native setter / insert operation with input+change semantics.
    Block,
    /// Real ordered key events with no intentional inter-key delay.
    FastKeys,
    /// Real ordered key events with an explicit bounded delay.
    HumanKeys,
    /// Composition-oriented insertion for text that cannot be represented as
    /// simple US-keyboard key events.
    Ime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct TypingPolicy {
    pub mode: TypingMode,
    pub delay_ms: u64,
    pub revalidate_every: usize,
}

impl TypingPolicy {
    pub fn new(mode: TypingMode, delay_ms: Option<u64>) -> Result<Self, &'static str> {
        let delay_ms = match mode {
            TypingMode::Block | TypingMode::FastKeys | TypingMode::Ime => 0,
            TypingMode::HumanKeys => delay_ms.unwrap_or(18),
        };
        if delay_ms > 250 {
            return Err("typing delay must be <= 250ms");
        }
        Ok(Self {
            mode,
            delay_ms,
            revalidate_every: match mode {
                TypingMode::FastKeys => 16,
                TypingMode::HumanKeys => 8,
                TypingMode::Block | TypingMode::Ime => 1,
            },
        })
    }

    pub fn choose(text: &str, requires_key_events: bool, composition_sensitive: bool) -> Self {
        let mode = if composition_sensitive || !text.is_ascii() {
            TypingMode::Ime
        } else if requires_key_events {
            TypingMode::FastKeys
        } else {
            TypingMode::Block
        };
        Self::new(mode, None).expect("built-in typing policies are valid")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InteractionEpoch {
    pub browser_generation: u64,
    pub target_id: String,
    pub target_revision: String,
    pub frame_id: String,
    pub loader_id: String,
    pub document_id: String,
    pub focused_backend_node_id: Option<i64>,
    pub selection_fingerprint: Option<String>,
    pub semantic_revision: String,
    pub viewport_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EpochUse {
    Read,
    PointerMutation,
    TextMutation,
}

impl InteractionEpoch {
    pub fn compatible_with(&self, fresh: &Self, use_case: EpochUse) -> bool {
        let document_same = self.browser_generation == fresh.browser_generation
            && self.target_id == fresh.target_id
            && self.target_revision == fresh.target_revision
            && self.frame_id == fresh.frame_id
            && self.loader_id == fresh.loader_id
            && self.document_id == fresh.document_id;
        if !document_same {
            return false;
        }
        match use_case {
            EpochUse::Read => true,
            EpochUse::PointerMutation => {
                self.semantic_revision == fresh.semantic_revision
                    && self.viewport_revision == fresh.viewport_revision
            }
            EpochUse::TextMutation => {
                self.semantic_revision == fresh.semantic_revision
                    && self.focused_backend_node_id == fresh.focused_backend_node_id
                    && self.selection_fingerprint == fresh.selection_fingerprint
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct TargetKey {
    pub browser_id: String,
    pub tab_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TargetLease {
    pub owner: String,
    pub acquired_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct LeaseTable {
    leases: BTreeMap<TargetKey, TargetLease>,
}

impl LeaseTable {
    pub fn acquire(
        &mut self,
        target: TargetKey,
        owner: &str,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<TargetLease, TargetLease> {
        self.expire(now_ms);
        if let Some(existing) = self.leases.get(&target)
            && existing.owner != owner
        {
            return Err(existing.clone());
        }
        let lease = TargetLease {
            owner: owner.to_owned(),
            acquired_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(ttl_ms.clamp(250, 30_000)),
        };
        self.leases.insert(target, lease.clone());
        Ok(lease)
    }

    pub fn release(&mut self, target: &TargetKey, owner: &str) -> bool {
        if self
            .leases
            .get(target)
            .is_some_and(|lease| lease.owner == owner)
        {
            self.leases.remove(target);
            true
        } else {
            false
        }
    }

    pub fn owner(&mut self, target: &TargetKey, now_ms: u64) -> Option<String> {
        self.expire(now_ms);
        self.leases.get(target).map(|lease| lease.owner.clone())
    }

    pub fn expire(&mut self, now_ms: u64) {
        self.leases.retain(|_, lease| lease.expires_at_ms > now_ms);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ClientSession {
    pub id: String,
    pub label: String,
    pub last_heartbeat_ms: u64,
    pub targets: BTreeSet<TargetKey>,
}

#[derive(Clone, Debug, Default)]
pub struct ClientSessions {
    sessions: BTreeMap<String, ClientSession>,
}

impl ClientSessions {
    pub fn register(&mut self, id: &str, label: &str, now_ms: u64) -> &ClientSession {
        self.sessions
            .entry(id.to_owned())
            .or_insert_with(|| ClientSession {
                id: id.to_owned(),
                label: label.to_owned(),
                last_heartbeat_ms: now_ms,
                targets: BTreeSet::new(),
            })
    }

    pub fn heartbeat(&mut self, id: &str, now_ms: u64) -> bool {
        if let Some(session) = self.sessions.get_mut(id) {
            session.last_heartbeat_ms = now_ms;
            true
        } else {
            false
        }
    }

    pub fn bind_target(&mut self, id: &str, target: TargetKey) -> bool {
        self.sessions
            .get_mut(id)
            .is_some_and(|session| session.targets.insert(target))
    }

    pub fn reap(&mut self, now_ms: u64, stale_after_ms: u64) -> Vec<ClientSession> {
        let stale = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                now_ms.saturating_sub(session.last_heartbeat_ms) > stale_after_ms
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        stale
            .into_iter()
            .filter_map(|id| self.sessions.remove(&id))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryClass {
    IdempotentRead,
    SideEffecting,
    UnknownDelivery,
}

pub fn may_retry(class: RecoveryClass, fresh_observation: bool) -> bool {
    match class {
        RecoveryClass::IdempotentRead => true,
        RecoveryClass::SideEffecting => false,
        RecoveryClass::UnknownDelivery => fresh_observation,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPrimitive {
    Research,
    FormFillNoSubmit,
    RepeatStructuredEntry,
    Upload,
    Download,
    SaveDraft,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrimitivePlan {
    pub primitive: TaskPrimitive,
    pub steps: Vec<&'static str>,
    pub allows_submit: bool,
    pub requires_final_verification: bool,
}

impl TaskPrimitive {
    pub fn plan(&self) -> PrimitivePlan {
        let steps = match self {
            Self::Research => vec!["snapshot", "extract", "checkpoint", "verify_sources"],
            Self::FormFillNoSubmit => vec!["snapshot", "find", "fill", "verify"],
            Self::RepeatStructuredEntry => vec!["snapshot", "skill_or_workflow", "verify_each"],
            Self::Upload => vec!["snapshot", "register_artifact", "upload", "verify"],
            Self::Download => vec!["snapshot", "trigger_download", "verify_artifact"],
            Self::SaveDraft => vec!["snapshot", "fill", "save_draft", "verify_persistence"],
        };
        PrimitivePlan {
            primitive: self.clone(),
            steps,
            allows_submit: false,
            requires_final_verification: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PageToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub read_only: bool,
    pub consequential: bool,
}

impl PageToolDescriptor {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.is_empty() || self.name.len() > 128 {
            return Err("page tool name must be 1..128 characters");
        }
        if self.description.len() > 2048 {
            return Err("page tool description exceeds limit");
        }
        if !self.input_schema.is_object() {
            return Err("page tool input schema must be an object");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CommandBudget {
    pub max_model_round_trips: u32,
    pub max_browser_commands: u32,
    pub max_wall_ms: u64,
}

impl CommandBudget {
    pub const fn interactive_default() -> Self {
        Self {
            max_model_round_trips: 4,
            max_browser_commands: 64,
            max_wall_ms: 30_000,
        }
    }
}
