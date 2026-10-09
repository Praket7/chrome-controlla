//! Explicit policy for choosing the cheapest qualified text-input strategy.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    #[default]
    Auto,
    Fast,
    Strict,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputStrategy {
    NativeSetter,
    InsertText,
    ChunkedInsert,
    SequentialKeys,
    Ime,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputCharacteristics {
    pub ordinary_text_control: bool,
    pub masked: bool,
    pub requires_trusted_events: bool,
    pub contenteditable: bool,
    pub ime_required: bool,
    pub insertion: bool,
    pub value_length: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StrategyDecision {
    pub strategy: Option<InputStrategy>,
    pub requires_app_adapter: bool,
}

pub fn select_input_strategy(mode: InputMode, input: InputCharacteristics) -> StrategyDecision {
    if input.masked || input.requires_trusted_events || input.contenteditable {
        return StrategyDecision {
            strategy: None,
            requires_app_adapter: true,
        };
    }
    if input.ime_required {
        return StrategyDecision {
            strategy: Some(InputStrategy::Ime),
            requires_app_adapter: false,
        };
    }
    match mode {
        InputMode::Strict => StrategyDecision {
            strategy: Some(InputStrategy::SequentialKeys),
            requires_app_adapter: false,
        },
        InputMode::Fast if !input.ordinary_text_control => StrategyDecision {
            strategy: None,
            requires_app_adapter: true,
        },
        InputMode::Fast | InputMode::Auto if input.ordinary_text_control && !input.insertion => {
            StrategyDecision {
                strategy: Some(InputStrategy::NativeSetter),
                requires_app_adapter: false,
            }
        }
        InputMode::Fast | InputMode::Auto if input.ordinary_text_control && input.insertion => {
            StrategyDecision {
                strategy: Some(if input.value_length > 4096 {
                    InputStrategy::ChunkedInsert
                } else {
                    InputStrategy::InsertText
                }),
                requires_app_adapter: false,
            }
        }
        InputMode::Auto => StrategyDecision {
            strategy: None,
            requires_app_adapter: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_prefers_native_setter_for_safe_ordinary_fill() {
        let decision = select_input_strategy(
            InputMode::Auto,
            InputCharacteristics {
                ordinary_text_control: true,
                ..Default::default()
            },
        );
        assert_eq!(decision.strategy, Some(InputStrategy::NativeSetter));
        assert!(!decision.requires_app_adapter);
    }

    #[test]
    fn trusted_masked_and_custom_controls_do_not_silently_use_fast_input() {
        for characteristics in [
            InputCharacteristics {
                ordinary_text_control: true,
                masked: true,
                ..Default::default()
            },
            InputCharacteristics {
                ordinary_text_control: true,
                requires_trusted_events: true,
                ..Default::default()
            },
            InputCharacteristics {
                contenteditable: true,
                ..Default::default()
            },
        ] {
            assert!(select_input_strategy(InputMode::Auto, characteristics).requires_app_adapter);
        }
    }

    #[test]
    fn strict_and_ime_modes_are_explicit() {
        assert_eq!(
            select_input_strategy(
                InputMode::Strict,
                InputCharacteristics {
                    ordinary_text_control: true,
                    ..Default::default()
                }
            )
            .strategy,
            Some(InputStrategy::SequentialKeys)
        );
        assert_eq!(
            select_input_strategy(
                InputMode::Auto,
                InputCharacteristics {
                    ordinary_text_control: true,
                    ime_required: true,
                    ..Default::default()
                }
            )
            .strategy,
            Some(InputStrategy::Ime)
        );
    }
}
