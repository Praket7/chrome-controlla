//! The single text-input strategy selector.
//!
//! Public V3 modes are mapped here once. `input_planner` compiles a canonical
//! transaction from this decision; both shared-extension and direct-CDP routes
//! execute the same transaction rather than maintaining parallel enums.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    #[default]
    Auto,
    Fast,
    Strict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicTypingMode {
    Block,
    FastKeys,
    HumanKeys,
    Ime,
    AppAdapter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputBackend {
    NativeSetter,
    InsertText,
    ChunkedInsert,
    KeyEvents,
    ImeComposition,
    ClipboardPaste,
    AppAdapter(String),
}

impl InputBackend {
    pub fn label(&self) -> &str {
        match self {
            Self::NativeSetter => "native_setter",
            Self::InsertText => "insert_text",
            Self::ChunkedInsert => "chunked_insert",
            Self::KeyEvents => "key_events",
            Self::ImeComposition => "ime_composition",
            Self::ClipboardPaste => "clipboard_paste",
            Self::AppAdapter(id) => id,
        }
    }

    pub fn is_app_adapter(&self) -> bool {
        matches!(self, Self::AppAdapter(_))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputPace {
    Immediate,
    Adaptive { min_ms: u16, max_ms: u16 },
    Human { delay_ms: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputOperation {
    Replace,
    Append,
    InsertAtCaret,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputExpectation {
    ExactValue(String),
    Suffix(String),
    CaretAnchor { offset: u32 },
    Unchecked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputTransaction {
    pub operation: InputOperation,
    pub expected: InputExpectation,
    pub text: String,
    pub backend: InputBackend,
    pub pace: InputPace,
    pub chunk_chars: usize,
    pub verify_every_chunks: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputCharacteristics {
    pub ordinary_text_control: bool,
    pub masked: bool,
    pub requires_trusted_events: bool,
    pub contenteditable: bool,
    pub ime_required: bool,
    pub insertion: bool,
    pub readonly: bool,
    pub disabled: bool,
    pub value_length: usize,
    pub explicit_text: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrategyDecision {
    pub backend: Option<InputBackend>,
    pub chunk_chars: usize,
    pub verify_every_chunks: u16,
    pub requires_app_adapter: bool,
}

impl StrategyDecision {
    pub fn legacy_strategy(&self) -> Option<InputStrategy> {
        self.backend.as_ref().map(|backend| match backend {
            InputBackend::NativeSetter => InputStrategy::NativeSetter,
            InputBackend::InsertText => InputStrategy::InsertText,
            InputBackend::ChunkedInsert => InputStrategy::ChunkedInsert,
            InputBackend::KeyEvents => InputStrategy::SequentialKeys,
            InputBackend::ImeComposition => InputStrategy::Ime,
            InputBackend::ClipboardPaste => InputStrategy::ClipboardPaste,
            InputBackend::AppAdapter(_) => InputStrategy::AppAdapter,
        })
    }
}

/// Compatibility alias retained for callers that imported the old enum.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputStrategy {
    NativeSetter,
    InsertText,
    ChunkedInsert,
    SequentialKeys,
    Ime,
    ClipboardPaste,
    AppAdapter,
}

pub fn select_input_strategy(mode: InputMode, input: InputCharacteristics) -> StrategyDecision {
    select_backend(mode, input, false)
}

pub fn select_backend(
    mode: InputMode,
    input: InputCharacteristics,
    composition_sensitive: bool,
) -> StrategyDecision {
    let blocked = input.masked || input.readonly || input.disabled;
    if blocked {
        return StrategyDecision {
            backend: None,
            chunk_chars: 1,
            verify_every_chunks: 1,
            requires_app_adapter: true,
        };
    }
    if input.ime_required || composition_sensitive {
        return StrategyDecision {
            backend: Some(InputBackend::ImeComposition),
            chunk_chars: 1,
            verify_every_chunks: 1,
            requires_app_adapter: false,
        };
    }
    if mode == InputMode::Strict {
        return StrategyDecision {
            backend: Some(InputBackend::KeyEvents),
            chunk_chars: 16,
            verify_every_chunks: 8,
            requires_app_adapter: false,
        };
    }
    if input.contenteditable || input.requires_trusted_events {
        // Generic rich editors first try trusted insertion. This is not an
        // automatic app-adapter requirement.
        return StrategyDecision {
            backend: Some(if input.requires_trusted_events {
                InputBackend::KeyEvents
            } else {
                InputBackend::InsertText
            }),
            chunk_chars: if input.requires_trusted_events { 32 } else { 256 },
            verify_every_chunks: 1,
            requires_app_adapter: false,
        };
    }
    match mode {
        InputMode::Fast | InputMode::Auto
            if input.ordinary_text_control && !input.insertion =>
        {
            StrategyDecision {
                backend: Some(InputBackend::NativeSetter),
                chunk_chars: input.value_length.max(1),
                verify_every_chunks: 1,
                requires_app_adapter: false,
            }
        }
        InputMode::Fast | InputMode::Auto
            if input.ordinary_text_control && input.insertion =>
        {
            StrategyDecision {
                backend: Some(if input.value_length > 4096 {
                    InputBackend::ChunkedInsert
                } else {
                    InputBackend::InsertText
                }),
                chunk_chars: 256,
                verify_every_chunks: 1,
                requires_app_adapter: false,
            }
        }
        InputMode::Fast | InputMode::Auto => StrategyDecision {
            backend: None,
            chunk_chars: 1,
            verify_every_chunks: 1,
            requires_app_adapter: true,
        },
        InputMode::Strict => unreachable!("strict mode is handled above"),
    }
}

pub fn pace_for(mode: PublicTypingMode, delay_ms: u16) -> InputPace {
    match mode {
        PublicTypingMode::HumanKeys => InputPace::Human {
            delay_ms: delay_ms.min(250),
        },
        PublicTypingMode::FastKeys => InputPace::Adaptive {
            min_ms: 0,
            max_ms: 4,
        },
        PublicTypingMode::Block | PublicTypingMode::Ime | PublicTypingMode::AppAdapter => {
            InputPace::Immediate
        }
    }
}

pub fn compile_transaction(
    mode: PublicTypingMode,
    operation: InputOperation,
    expected: InputExpectation,
    text: &str,
    _input: InputCharacteristics,
    delay_ms: u16,
) -> Result<InputTransaction, &'static str> {
    if text.is_empty() || text.len() > 16_384 {
        return Err("typing text must contain 1..=16384 bytes");
    }
    if mode == PublicTypingMode::HumanKeys
        && text.chars().any(|character| character.is_control() && character != '\n')
    {
        return Err("HumanKeys does not support control characters other than normalized newline");
    }
    let decision = match mode {
        PublicTypingMode::Block => StrategyDecision {
            backend: Some(InputBackend::InsertText),
            chunk_chars: 256,
            verify_every_chunks: 1,
            requires_app_adapter: false,
        },
        PublicTypingMode::FastKeys | PublicTypingMode::HumanKeys => StrategyDecision {
            backend: Some(InputBackend::KeyEvents),
            chunk_chars: if mode == PublicTypingMode::HumanKeys { 8 } else { 32 },
            verify_every_chunks: if mode == PublicTypingMode::HumanKeys { 1 } else { 4 },
            requires_app_adapter: false,
        },
        PublicTypingMode::Ime => StrategyDecision {
            backend: Some(InputBackend::ImeComposition),
            chunk_chars: 1,
            verify_every_chunks: 1,
            requires_app_adapter: false,
        },
        PublicTypingMode::AppAdapter => StrategyDecision {
            backend: Some(InputBackend::AppAdapter("generic".into())),
            chunk_chars: 1,
            verify_every_chunks: 1,
            requires_app_adapter: true,
        },
    };
    let Some(backend) = decision.backend else {
        return Err("no qualified input backend for this target");
    };
    Ok(InputTransaction {
        operation,
        expected,
        text: text.to_owned(),
        backend,
        pace: pace_for(mode, delay_ms),
        chunk_chars: decision.chunk_chars.max(1),
        verify_every_chunks: decision.verify_every_chunks,
    })
}

pub fn plan_legacy(
    text: &str,
    requires_trusted: bool,
    prefers_composition: bool,
) -> StrategyDecision {
    let characteristics = InputCharacteristics {
        ordinary_text_control: !requires_trusted,
        requires_trusted_events: requires_trusted,
        ime_required: prefers_composition || !text.is_ascii(),
        value_length: text.len(),
        ..InputCharacteristics::default()
    };
    select_backend(InputMode::Auto, characteristics, prefers_composition)
}

pub fn adapt_chunk_size(current: usize, stable_rtt: bool, diverged: bool) -> usize {
    if diverged && current > 8 {
        current / 2
    } else if stable_rtt && current < 256 {
        (current + 16).min(256)
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn characteristics() -> InputCharacteristics {
        InputCharacteristics {
            ordinary_text_control: true,
            ..InputCharacteristics::default()
        }
    }

    #[test]
    fn auto_prefers_native_setter_for_safe_ordinary_fill() {
        let decision = select_input_strategy(InputMode::Auto, characteristics());
        assert_eq!(decision.backend, Some(InputBackend::NativeSetter));
        assert!(!decision.requires_app_adapter);
    }

    #[test]
    fn contenteditable_uses_generic_insert_text_not_an_adapter() {
        let decision = select_input_strategy(
            InputMode::Auto,
            InputCharacteristics {
                contenteditable: true,
                ..InputCharacteristics::default()
            },
        );
        assert_eq!(decision.backend, Some(InputBackend::InsertText));
        assert!(!decision.requires_app_adapter);
    }

    #[test]
    fn trusted_masked_readonly_and_unsupported_controls_fail_closed() {
        for (input, expected_adapter) in [
            (
                InputCharacteristics {
                    requires_trusted_events: true,
                    ordinary_text_control: true,
                    ..InputCharacteristics::default()
                },
                false,
            ),
            (
                InputCharacteristics {
                    masked: true,
                    ordinary_text_control: true,
                    ..InputCharacteristics::default()
                },
                true,
            ),
            (
                InputCharacteristics {
                    readonly: true,
                    ordinary_text_control: true,
                    ..InputCharacteristics::default()
                },
                true,
            ),
            (InputCharacteristics::default(), true),
        ] {
            let decision = select_input_strategy(InputMode::Auto, input);
            if expected_adapter {
                assert!(decision.backend.is_none());
                assert!(decision.requires_app_adapter);
            } else {
                assert_eq!(decision.backend, Some(InputBackend::KeyEvents));
            }
        }
    }

    #[test]
    fn explicit_modes_win_over_auto() {
        assert_eq!(
            select_input_strategy(InputMode::Strict, characteristics()).backend,
            Some(InputBackend::KeyEvents)
        );
        assert_eq!(
            select_input_strategy(
                InputMode::Auto,
                InputCharacteristics {
                    ime_required: true,
                    ..characteristics()
                }
            )
            .backend,
            Some(InputBackend::ImeComposition)
        );
    }

    #[test]
    fn table_driven_transaction_compilation_is_canonical() {
        let cases = [
            (PublicTypingMode::Block, InputBackend::NativeSetter),
            (PublicTypingMode::FastKeys, InputBackend::KeyEvents),
            (PublicTypingMode::HumanKeys, InputBackend::KeyEvents),
            (PublicTypingMode::Ime, InputBackend::ImeComposition),
        ];
        for (mode, backend) in cases {
            let transaction = compile_transaction(
                mode,
                InputOperation::Append,
                InputExpectation::ExactValue("old".into()),
                "café\n🙂",
                characteristics(),
                16,
            )
            .unwrap();
            assert_eq!(transaction.backend, backend);
            assert!(transaction.chunk_chars >= 1);
            assert!(transaction.verify_every_chunks >= 1);
        }
        assert!(compile_transaction(
            PublicTypingMode::Block,
            InputOperation::Append,
            InputExpectation::Unchecked,
            "",
            characteristics(),
            0,
        )
        .is_err());
    }

    #[test]
    fn unicode_and_newlines_are_not_app_adapter_exclusives() {
        let decision = plan_legacy("こんにちは", false, false);
        assert_eq!(decision.backend, Some(InputBackend::ImeComposition));
        let transaction = compile_transaction(
            PublicTypingMode::Ime,
            InputOperation::Replace,
            InputExpectation::Unchecked,
            "CJK 🙂\ncombining é",
            characteristics(),
            0,
        )
        .unwrap();
        assert_eq!(transaction.backend, InputBackend::ImeComposition);
    }

    #[test]
    fn chunk_adaptation_has_a_finite_ceiling() {
        assert_eq!(adapt_chunk_size(16, true, false), 32);
        assert_eq!(adapt_chunk_size(250, true, false), 256);
        assert_eq!(adapt_chunk_size(64, false, true), 32);
        assert_eq!(adapt_chunk_size(4, false, true), 4);
    }
}
