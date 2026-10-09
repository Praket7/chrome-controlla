//! Controlla v3 execution runtime primitives.
//!
//! This module turns the policy-only v3 types into deterministic plans that can
//! be consumed by the browser bridge and MCP surface without adding a second
//! model loop. The invariants are deliberately fail-closed: stale interaction
//! epochs, ambiguous mutation delivery, lease conflicts, oversized page-tool
//! results, and exhausted reconnect budgets all stop execution.

use crate::v3::{BrowserMode, EpochUse, InteractionEpoch, RecoveryClass, TypingMode, TypingPolicy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const DEFAULT_AGENT_TOOLS: [&str; 6] = [
    "browser", "snapshot", "act", "workflow", "extract", "verify",
];
pub const MAX_BATCH_ACTIONS: usize = 64;
pub const MAX_PAGE_TOOL_OUTPUT_BYTES: usize = 256 * 1024;
pub const MAX_AGENT_RESULT_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BrowserLaunchPlan {
    pub mode: BrowserMode,
    pub provider: &'static str,
    pub chrome_args: Vec<&'static str>,
    pub may_activate_window: bool,
}

impl BrowserLaunchPlan {
    pub fn for_mode(mode: BrowserMode) -> Self {
        let mut chrome_args = vec![
            "--remote-debugging-address=127.0.0.1",
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
        ];
        match mode {
            BrowserMode::Foreground => {}
            BrowserMode::Background => {
                chrome_args.push("--start-minimized");
            }
            BrowserMode::Headless => {
                chrome_args.push("--headless=new");
                chrome_args.push("--disable-background-timer-throttling");
            }
        }
        Self {
            mode,
            provider: mode.default_provider(),
            chrome_args,
            may_activate_window: mode.policy().may_activate_window,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchActionKind {
    Read,
    PointerMutation,
    TextMutation,
}

impl BatchActionKind {
    fn epoch_use(self) -> EpochUse {
        match self {
            Self::Read => EpochUse::Read,
            Self::PointerMutation => EpochUse::PointerMutation,
            Self::TextMutation => EpochUse::TextMutation,
        }
    }

    pub fn is_mutation(self) -> bool {
        !matches!(self, Self::Read)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BatchAction {
    pub id: String,
    pub kind: BatchActionKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BatchReceipt {
    pub completed: Vec<String>,
    pub stopped_before: Option<String>,
    pub host_round_trips: u32,
    pub in_browser_actions: u32,
    pub stale_transition: bool,
}

pub fn execute_guarded_batch(
    grounded: &InteractionEpoch,
    fresh_epochs: &[InteractionEpoch],
    actions: &[BatchAction],
) -> Result<BatchReceipt, &'static str> {
    if actions.is_empty() || actions.len() > MAX_BATCH_ACTIONS {
        return Err("batch must contain 1..64 actions");
    }
    if fresh_epochs.len() != actions.len() {
        return Err("each batch action requires a fresh browser epoch");
    }
    let mut completed = Vec::with_capacity(actions.len());
    for (action, fresh) in actions.iter().zip(fresh_epochs) {
        if !grounded.compatible_with(fresh, action.kind.epoch_use()) {
            return Ok(BatchReceipt {
                completed,
                stopped_before: Some(action.id.clone()),
                host_round_trips: 1,
                in_browser_actions: actions.len().try_into().unwrap_or(u32::MAX),
                stale_transition: true,
            });
        }
        completed.push(action.id.clone());
    }
    Ok(BatchReceipt {
        completed,
        stopped_before: None,
        host_round_trips: 1,
        in_browser_actions: actions.len().try_into().unwrap_or(u32::MAX),
        stale_transition: false,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyEventKind {
    KeyDown,
    BeforeInput,
    Input,
    KeyUp,
    CompositionStart,
    CompositionUpdate,
    CompositionEnd,
    BlockInsert,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KeyEvent {
    pub kind: KeyEventKind,
    pub text: String,
    pub delay_after_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TypingPlan {
    pub policy: TypingPolicy,
    pub events: Vec<KeyEvent>,
    pub protocol_batches: usize,
    pub requires_focus_revalidation: bool,
}

impl TypingPlan {
    pub fn build(
        text: &str,
        mode: TypingMode,
        human_delay_ms: Option<u64>,
    ) -> Result<Self, &'static str> {
        let policy = TypingPolicy::new(mode, human_delay_ms)?;
        let mut events = Vec::new();
        match mode {
            TypingMode::Block => events.push(KeyEvent {
                kind: KeyEventKind::BlockInsert,
                text: text.to_owned(),
                delay_after_ms: 0,
            }),
            TypingMode::FastKeys | TypingMode::HumanKeys => {
                for ch in text.chars() {
                    let text = ch.to_string();
                    for kind in [
                        KeyEventKind::KeyDown,
                        KeyEventKind::BeforeInput,
                        KeyEventKind::Input,
                        KeyEventKind::KeyUp,
                    ] {
                        events.push(KeyEvent {
                            kind,
                            text: text.clone(),
                            delay_after_ms: if kind == KeyEventKind::KeyUp {
                                policy.delay_ms
                            } else {
                                0
                            },
                        });
                    }
                }
            }
            TypingMode::Ime => {
                events.push(KeyEvent {
                    kind: KeyEventKind::CompositionStart,
                    text: String::new(),
                    delay_after_ms: 0,
                });
                events.push(KeyEvent {
                    kind: KeyEventKind::CompositionUpdate,
                    text: text.to_owned(),
                    delay_after_ms: 0,
                });
                events.push(KeyEvent {
                    kind: KeyEventKind::Input,
                    text: text.to_owned(),
                    delay_after_ms: 0,
                });
                events.push(KeyEvent {
                    kind: KeyEventKind::CompositionEnd,
                    text: text.to_owned(),
                    delay_after_ms: 0,
                });
            }
        }
        let protocol_batches = match mode {
            TypingMode::Block | TypingMode::Ime => 1,
            TypingMode::FastKeys => text.chars().count().div_ceil(policy.revalidate_every),
            TypingMode::HumanKeys => text.chars().count(),
        };
        Ok(Self {
            policy,
            events,
            protocol_batches,
            requires_focus_revalidation: !matches!(mode, TypingMode::Block),
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageToolAuthority {
    ReadOnly,
    Mutating,
    Consequential,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PageToolCall {
    pub name: String,
    pub authority: PageToolAuthority,
    pub input: Value,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}

impl PageToolCall {
    pub fn validate(&self, allowed_tools: &BTreeSet<String>) -> Result<(), &'static str> {
        if self.name.is_empty() || self.name.len() > 128 || !allowed_tools.contains(&self.name) {
            return Err("page tool is not explicitly allowed");
        }
        if !self.input.is_object() {
            return Err("page tool input must be a JSON object");
        }
        if self.timeout_ms == 0 || self.timeout_ms > 120_000 {
            return Err("page tool timeout must be within 1..120000ms");
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > MAX_PAGE_TOOL_OUTPUT_BYTES {
            return Err("page tool output budget exceeded");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ReconnectDecision {
    RetryRead,
    ObserveBeforeDecision,
    StopUnknownMutation,
    Terminal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReconnectController {
    max_attempts: u8,
    attempts: u8,
}

impl ReconnectController {
    pub fn new(max_attempts: u8) -> Result<Self, &'static str> {
        if max_attempts == 0 || max_attempts > 8 {
            return Err("reconnect attempts must be within 1..8");
        }
        Ok(Self {
            max_attempts,
            attempts: 0,
        })
    }

    pub fn on_disconnect(&mut self, class: RecoveryClass) -> ReconnectDecision {
        if self.attempts >= self.max_attempts {
            return ReconnectDecision::Terminal;
        }
        self.attempts += 1;
        match class {
            RecoveryClass::IdempotentRead => ReconnectDecision::RetryRead,
            RecoveryClass::SideEffecting => ReconnectDecision::StopUnknownMutation,
            RecoveryClass::UnknownDelivery => ReconnectDecision::ObserveBeforeDecision,
        }
    }

    pub fn attempts(&self) -> u8 {
        self.attempts
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Freebuff,
    OpenCode,
    ClaudeCode,
    Codex,
    ChatGptDesktop,
    GenericMcp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ClientQualification {
    pub client: ClientKind,
    pub startup: bool,
    pub persistent_session: bool,
    pub foreground: bool,
    pub background: bool,
    pub headless: bool,
    pub typed_input: bool,
    pub multi_tab: bool,
    pub interference_safe: bool,
    pub upload_download: bool,
    pub research_extract: bool,
    pub verified_mutations: bool,
}

impl ClientQualification {
    pub fn qualified(&self) -> bool {
        self.startup
            && self.persistent_session
            && self.foreground
            && self.background
            && self.headless
            && self.typed_input
            && self.multi_tab
            && self.interference_safe
            && self.upload_download
            && self.research_extract
            && self.verified_mutations
    }
}

pub fn compact_tool_surface_valid(serialized_schema_bytes: usize) -> bool {
    serialized_schema_bytes <= 48 * 1024
        && DEFAULT_AGENT_TOOLS.len() == 6
        && DEFAULT_AGENT_TOOLS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            == 6
}

pub fn result_within_budget(value: &Value) -> bool {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len() <= MAX_AGENT_RESULT_BYTES)
        .unwrap_or(false)
}
