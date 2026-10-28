//! Canonical V3 typing policy and transaction representation.
//!
//! This module is the single place that maps public typing modes to an input
//! backend, normalizes text, and builds the one-shot extension transaction.

use serde::{Deserialize, Serialize};

pub const MAX_TRANSACTION_TEXT: usize = 16_384;
pub const DEFAULT_CHUNK_CHARS: usize = 256;
pub const DEFAULT_KEY_CHUNK_CHARS: usize = 32;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputBackend {
    NativeSetter,
    InsertText,
    KeyEvents,
    ImeComposition,
    ClipboardPaste,
    AppAdapter,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputPace {
    Immediate,
    Adaptive { min_ms: u16, max_ms: u16 },
    Human { delay_ms: u16 },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputOperation {
    Replace,
    Append,
    InsertAtCaret,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputExpectation {
    ExactValue { value: String },
    Suffix { value: String },
    CaretAnchor { offset: u32 },
    Unchecked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InputTransaction {
    pub operation: InputOperation,
    pub expected: InputExpectation,
    pub text: String,
    pub backend: InputBackend,
    pub pace: InputPace,
    pub chunk_chars: usize,
    pub verify_every_chunks: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicTypingMode {
    Block,
    FastKeys,
    HumanKeys,
    Ime,
    Auto,
}

pub fn normalize_key_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn human_keys_control_character(text: &str) -> Option<char> {
    text.chars()
        .find(|character| character.is_control() && *character != '\n')
}

pub fn backend_for_mode(mode: PublicTypingMode) -> InputBackend {
    match mode {
        PublicTypingMode::Block => InputBackend::InsertText,
        PublicTypingMode::FastKeys | PublicTypingMode::HumanKeys => InputBackend::KeyEvents,
        PublicTypingMode::Ime => InputBackend::ImeComposition,
        PublicTypingMode::Auto => InputBackend::InsertText,
    }
}

pub fn pace_for_mode(mode: PublicTypingMode, delay_ms: u16) -> InputPace {
    match mode {
        PublicTypingMode::HumanKeys => InputPace::Human {
            delay_ms: delay_ms.min(250),
        },
        PublicTypingMode::FastKeys => InputPace::Adaptive {
            min_ms: 0,
            max_ms: 4,
        },
        PublicTypingMode::Block | PublicTypingMode::Ime | PublicTypingMode::Auto => InputPace::Immediate,
    }
}

pub fn chunk_chars_for(backend: InputBackend) -> usize {
    match backend {
        InputBackend::KeyEvents => DEFAULT_KEY_CHUNK_CHARS,
        _ => DEFAULT_CHUNK_CHARS,
    }
}

pub fn verify_every_for(mode: PublicTypingMode) -> u16 {
    match mode {
        PublicTypingMode::HumanKeys => 1,
        PublicTypingMode::FastKeys => 4,
        PublicTypingMode::Block | PublicTypingMode::Ime | PublicTypingMode::Auto => 1,
    }
}

pub fn compile_transaction(
    mode: PublicTypingMode,
    operation: InputOperation,
    expected: InputExpectation,
    text: &str,
    delay_ms: u16,
) -> Result<InputTransaction, &'static str> {
    let text = normalize_key_text(text);
    if text.is_empty() || text.len() > MAX_TRANSACTION_TEXT {
        return Err("typing text must contain 1..=16384 bytes");
    }
    if mode == PublicTypingMode::HumanKeys && human_keys_control_character(&text).is_some() {
        return Err("HumanKeys does not support control characters other than normalized newline");
    }
    let backend = backend_for_mode(mode);
    Ok(InputTransaction {
        operation,
        expected,
        text,
        backend,
        pace: pace_for_mode(mode, delay_ms),
        chunk_chars: chunk_chars_for(backend),
        verify_every_chunks: verify_every_for(mode),
    })
}

pub fn transaction_payload(transaction: &InputTransaction) -> serde_json::Value {
    serde_json::json!({
        "operation": transaction.operation,
        "expected": transaction.expected,
        "text": transaction.text,
        "strategy": transaction.backend,
        "chunk_chars": transaction.chunk_chars,
        "verify_every_chunks": transaction.verify_every_chunks,
        "pacing_ms": match transaction.pace {
            InputPace::Human { delay_ms } => delay_ms,
            InputPace::Adaptive { .. } | InputPace::Immediate => 0,
        },
    })
}

pub fn receipt_is_verified(receipt: &serde_json::Value) -> bool {
    receipt["status"] == "verified" && receipt["dispatch_may_have_occurred"] == true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_text_and_control_checks_are_canonical() {
        assert_eq!(normalize_key_text("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(human_keys_control_character("café🙂\n"), None);
        assert_eq!(human_keys_control_character("a\u{0001}b"), Some('\u{0001}'));
    }

    #[test]
    fn public_modes_map_to_one_backend_and_pace() {
        assert_eq!(backend_for_mode(PublicTypingMode::Block), InputBackend::InsertText);
        assert_eq!(backend_for_mode(PublicTypingMode::FastKeys), InputBackend::KeyEvents);
        assert_eq!(backend_for_mode(PublicTypingMode::HumanKeys), InputBackend::KeyEvents);
        assert_eq!(backend_for_mode(PublicTypingMode::Ime), InputBackend::ImeComposition);
        assert_eq!(pace_for_mode(PublicTypingMode::HumanKeys, 999), InputPace::Human { delay_ms: 250 });
    }

    #[test]
    fn transaction_compilation_is_bounded_and_mode_aware() {
        let transaction = compile_transaction(
            PublicTypingMode::Auto,
            InputOperation::Append,
            InputExpectation::ExactValue { value: "old".into() },
            "new",
            0,
        )
        .unwrap();
        assert_eq!(transaction.backend, InputBackend::InsertText);
        assert_eq!(transaction.chunk_chars, DEFAULT_CHUNK_CHARS);
        assert!(compile_transaction(
            PublicTypingMode::Auto,
            InputOperation::Append,
            InputExpectation::Unchecked,
            "",
            0,
        )
        .is_err());
        assert!(compile_transaction(
            PublicTypingMode::HumanKeys,
            InputOperation::Append,
            InputExpectation::Unchecked,
            "bad\u{0001}",
            0,
        )
        .is_err());
    }
}
