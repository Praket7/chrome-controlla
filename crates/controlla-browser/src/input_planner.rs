//! Compatibility facade and transaction compiler over `input_strategy`.
//!
//! This module no longer owns a second set of strategy rules. It maps the
//! legacy public modes to the canonical backend and compiles a microprogram for
//! callers that still need the old shape.

pub use crate::input_strategy::{
    InputBackend, InputExpectation, InputOperation, InputPace, InputTransaction,
    PublicTypingMode, adapt_chunk_size, compile_transaction, select_backend,
    select_input_strategy,
};
use crate::input_strategy::{InputCharacteristics, InputMode as CanonicalMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Block,
    FastKeys,
    HumanKeys,
    Ime,
    AppAdapter,
}

#[derive(Clone, Debug)]
pub struct InputPlan {
    pub mode: InputMode,
    pub backend: InputBackend,
    pub chunk_size: usize,
    pub delay_ms: u64,
    pub verify_every_chunks: u16,
}

pub fn plan(text: &str, requires_trusted: bool, prefers_composition: bool) -> InputPlan {
    let characteristics = InputCharacteristics {
        ordinary_text_control: !requires_trusted,
        requires_trusted_events: requires_trusted,
        ime_required: prefers_composition || !text.is_ascii(),
        value_length: text.len(),
        ..InputCharacteristics::default()
    };
    let decision = select_backend(CanonicalMode::Fast, characteristics, prefers_composition);
    let (mode, delay_ms) = match decision.backend {
        Some(InputBackend::ImeComposition) => (InputMode::Ime, 0),
        Some(InputBackend::KeyEvents) if requires_trusted => (InputMode::FastKeys, 0),
        Some(InputBackend::KeyEvents) => (InputMode::HumanKeys, 180),
        Some(InputBackend::AppAdapter(_)) => (InputMode::AppAdapter, 0),
        Some(_) | None => (InputMode::Block, 0),
    };
    InputPlan {
        mode,
        backend: decision.backend.unwrap_or(InputBackend::AppAdapter("unqualified".into())),
        chunk_size: decision.chunk_chars.max(1),
        delay_ms,
        verify_every_chunks: decision.verify_every_chunks.max(1),
    }
}

#[derive(Clone, Debug)]
pub struct Microprogram {
    pub steps: Vec<MicroStep>,
    pub deadline_ms: u64,
}

#[derive(Clone, Debug)]
pub enum MicroStep {
    FocusCheck,
    ValueCheck,
    DispatchKey { ch: char },
    Composition { text: String },
}

pub fn compile_microprogram(text: &str, mode: InputMode) -> Microprogram {
    let public_mode = match mode {
        InputMode::Block => PublicTypingMode::Block,
        InputMode::FastKeys => PublicTypingMode::FastKeys,
        InputMode::HumanKeys => PublicTypingMode::HumanKeys,
        InputMode::Ime => PublicTypingMode::Ime,
        InputMode::AppAdapter => PublicTypingMode::AppAdapter,
    };
    let transaction = compile_transaction(
        public_mode,
        InputOperation::Append,
        InputExpectation::Unchecked,
        if text.is_empty() { " " } else { text },
        InputCharacteristics::default(),
        0,
    )
    .expect("the built-in transaction plan is valid");
    let mut steps = vec![MicroStep::FocusCheck, MicroStep::ValueCheck];
    match transaction.backend {
        InputBackend::NativeSetter
        | InputBackend::InsertText
        | InputBackend::ChunkedInsert
        | InputBackend::ImeComposition => {
            steps.push(MicroStep::Composition {
                text: text.to_owned(),
            });
        }
        InputBackend::KeyEvents | InputBackend::ClipboardPaste | InputBackend::AppAdapter(_) => {
            for ch in text.chars() {
                steps.push(MicroStep::DispatchKey { ch });
            }
            steps.push(MicroStep::ValueCheck);
        }
    }
    Microprogram {
        steps,
        deadline_ms: 60_000,
    }
}

/// Convert a canonical transaction to the existing guarded-input action. The
/// direct-CDP route and the extension route therefore share one transaction
/// representation even though their dispatch mechanics differ.
pub fn transaction_action(transaction: &InputTransaction) -> super::input::InputAction {
    match (&transaction.backend, transaction.operation) {
        (InputBackend::NativeSetter, InputOperation::Replace) => {
            super::input::InputAction::Fill(transaction.text.clone())
        }
        (InputBackend::NativeSetter, _) => super::input::InputAction::Insert(transaction.text.clone()),
        (InputBackend::InsertText | InputBackend::ChunkedInsert, _) => {
            super::input::InputAction::Insert(transaction.text.clone())
        }
        (InputBackend::KeyEvents, _) => {
            super::input::InputAction::SequentialKeys(transaction.text.clone())
        }
        (InputBackend::ImeComposition, _) => {
            super::input::InputAction::ImeText(transaction.text.clone())
        }
        (InputBackend::ClipboardPaste, _) => super::input::InputAction::PasteInternalClipboard,
        (InputBackend::AppAdapter(_), _) => super::input::InputAction::Insert(transaction.text.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_chooses_canonical_modes() {
        assert_eq!(plan("hello", false, false).mode, InputMode::Block);
        assert_eq!(plan("hello", true, false).mode, InputMode::FastKeys);
        assert_eq!(plan("こんにちは", false, false).mode, InputMode::Ime);
    }

    #[test]
    fn microprogram_uses_one_transaction_shape() {
        let program = compile_microprogram("hello", InputMode::Block);
        assert_eq!(program.steps.len(), 3);
        assert!(matches!(program.steps[2], MicroStep::Composition { .. }));
        let keys = compile_microprogram("ab", InputMode::FastKeys);
        assert_eq!(
            keys.steps
                .iter()
                .filter(|step| matches!(step, MicroStep::DispatchKey { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn canonical_transaction_maps_to_existing_action() {
        let transaction = compile_transaction(
            PublicTypingMode::Block,
            InputOperation::Replace,
            InputExpectation::Unchecked,
            "hello",
            InputCharacteristics {
                ordinary_text_control: true,
                ..InputCharacteristics::default()
            },
            0,
        )
        .unwrap();
        assert!(matches!(
            transaction_action(&transaction),
            super::super::input::InputAction::Fill(value) if value == "hello"
        ));
    }
}
