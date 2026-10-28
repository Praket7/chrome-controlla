//! Canonical V3 typing policy facade.
//!
//! The transaction representation and backend vocabulary live in
//! `controlla_browser::input_strategy`; this module only maps the public V3
//! typing modes onto that single engine.

pub use controlla_browser::input_strategy::{
    InputBackend, InputExpectation, InputOperation, InputPace, InputTransaction,
};
use controlla_browser::input_strategy::{
    InputCharacteristics, PublicTypingMode as CanonicalTypingMode,
    compile_transaction as compile_canonical,
};

pub const MAX_TRANSACTION_TEXT: usize = 16_384;
pub const DEFAULT_CHUNK_CHARS: usize = 256;
pub const DEFAULT_KEY_CHUNK_CHARS: usize = 32;

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
        PublicTypingMode::Block | PublicTypingMode::Ime | PublicTypingMode::Auto => {
            InputPace::Immediate
        }
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
    let public_mode = match mode {
        PublicTypingMode::Block => CanonicalTypingMode::Block,
        PublicTypingMode::FastKeys => CanonicalTypingMode::FastKeys,
        PublicTypingMode::HumanKeys => CanonicalTypingMode::HumanKeys,
        PublicTypingMode::Ime => CanonicalTypingMode::Ime,
        PublicTypingMode::Auto => CanonicalTypingMode::Block,
    };
    compile_canonical(
        public_mode,
        operation,
        expected,
        &text,
        InputCharacteristics {
            ordinary_text_control: true,
            contenteditable: false,
            ..InputCharacteristics::default()
        },
        delay_ms,
    )
}


pub fn operation_label(operation: InputOperation) -> &'static str {
    match operation {
        InputOperation::Replace => "replace",
        InputOperation::Append => "append",
        InputOperation::InsertAtCaret => "insert_at_caret",
    }
}

pub fn transaction_payload(transaction: &InputTransaction) -> serde_json::Value {
    serde_json::json!({
        "operation": operation_label(transaction.operation),
        "expected": match &transaction.expected {
            InputExpectation::ExactValue(value) => serde_json::json!({"kind":"exact_value","value":value}),
            InputExpectation::Suffix(value) => serde_json::json!({"kind":"suffix","value":value}),
            InputExpectation::CaretAnchor { offset } => serde_json::json!({"kind":"caret_anchor","offset":offset}),
            InputExpectation::Unchecked => serde_json::json!({"kind":"unchecked"}),
        },
        "text": transaction.text,
        "strategy": transaction.backend.label(),
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

#[derive(Clone, Debug)]
pub enum TransactionTarget {
    Object { object_id: String, index: usize },
    Frame {
        frame_id: String,
        loader_id: String,
        backend_node_id: i64,
    },
}

pub fn strategy_for(mode: PublicTypingMode) -> &'static str {
    match mode {
        PublicTypingMode::Block | PublicTypingMode::Auto => "insert_text",
        PublicTypingMode::FastKeys | PublicTypingMode::HumanKeys => "key_events",
        PublicTypingMode::Ime => "ime",
    }
}

pub fn extension_transaction_payload(
    target: &TransactionTarget,
    expected: &str,
    text: &str,
    mode: PublicTypingMode,
    delay_ms: u16,
    deadline_ms: u64,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "expected": {"kind": "exact_value", "value": expected},
        "operation": "append",
        "text": text,
        "strategy": strategy_for(mode),
        "chunk_chars": if mode == PublicTypingMode::HumanKeys { 8 } else { 64 },
        "verify_every_chunks": if mode == PublicTypingMode::HumanKeys { 1 } else { 4 },
        "pacing_ms": if mode == PublicTypingMode::HumanKeys { delay_ms } else { 0 },
        "normalize_paragraphs": true,
        "deadline_ms": deadline_ms.clamp(1, 600_000),
    });
    if let Some(object) = payload.as_object_mut() {
        match target {
            TransactionTarget::Object { object_id, index } => {
                object.insert("object_id".into(), serde_json::json!(object_id));
                object.insert("index".into(), serde_json::json!(index));
            }
            TransactionTarget::Frame {
                frame_id,
                loader_id,
                backend_node_id,
            } => {
                object.insert("frame_id".into(), serde_json::json!(frame_id));
                object.insert("loader_id".into(), serde_json::json!(loader_id));
                object.insert("backend_node_id".into(), serde_json::json!(backend_node_id));
            }
        }
    }
    payload
}

pub fn compact_transaction_receipt(receipt: &serde_json::Value, mode: PublicTypingMode) -> serde_json::Value {
    let status = receipt["status"].as_str().unwrap_or("unknown");
    serde_json::json!({
        "status": status,
        "action": "type",
        "typing_mode": match mode {
            PublicTypingMode::Block | PublicTypingMode::Auto => "block",
            PublicTypingMode::FastKeys => "fast_keys",
            PublicTypingMode::HumanKeys => "human_keys",
            PublicTypingMode::Ime => "ime",
        },
        "input_method": receipt["backend"],
        "completed_chars": receipt["completed_chars"],
        "key_events": receipt["key_events"],
        "cdp_commands": receipt["cdp_commands"],
        "dispatch_acknowledged": receipt["dispatch_may_have_occurred"],
        "dispatch_may_have_occurred": receipt["dispatch_may_have_occurred"],
        "readback_length": receipt["readback_length"],
        "readback_hash": receipt["readback_hash"],
        "adapter": receipt["adapter"],
        "reason_code": receipt["reason"],
        "interruption": receipt["interruption"],
        "next_action": if status == "unknown" {
            serde_json::Value::from("Inspect the field read-only; do not replay the whole text transaction.")
        } else if status == "not_dispatched" {
            serde_json::Value::from("Refresh the target and retry once.")
        } else {
            serde_json::Value::Null
        },
    })
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
        assert_eq!(
            pace_for_mode(PublicTypingMode::HumanKeys, 999),
            InputPace::Human { delay_ms: 250 }
        );
    }

    #[test]
    fn transaction_compilation_is_bounded_and_mode_aware() {
        let transaction = compile_transaction(
            PublicTypingMode::Auto,
            InputOperation::Append,
            InputExpectation::ExactValue("old".into()),
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

    #[test]
    fn frame_ime_payload_keeps_one_transaction_and_frame_identity() {
        let payload = extension_transaction_payload(
            &TransactionTarget::Frame {
                frame_id: "frame-oopif".into(),
                loader_id: "loader-1".into(),
                backend_node_id: 92,
            },
            "old",
            "こんにちは",
            PublicTypingMode::Ime,
            0,
            5_000,
        );
        assert_eq!(payload["strategy"], "ime");
        assert_eq!(payload["frame_id"], "frame-oopif");
        assert_eq!(payload["loader_id"], "loader-1");
        assert_eq!(payload["backend_node_id"], 92);
        assert_eq!(payload["expected"]["kind"], "exact_value");
        assert!(payload.get("object_id").is_none());
    }

    #[test]
    fn payload_exposes_canonical_expectation_shape() {
        let transaction = compile_transaction(
            PublicTypingMode::Ime,
            InputOperation::Append,
            InputExpectation::Suffix("old".into()),
            "こんにちは",
            0,
        )
        .unwrap();
        let payload = transaction_payload(&transaction);
        assert_eq!(payload["strategy"], "ime_composition");
        assert_eq!(payload["expected"]["kind"], "suffix");
    }
}
