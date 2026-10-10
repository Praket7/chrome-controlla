//! Single source of truth for block, fast-key, human-key, IME strategy.
//! Adaptive FastKeys chunk size: begin 16/32, grow/shrink, no intentional delay.

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
    pub chunk_size: usize,
    pub delay_ms: u64,
}

pub fn plan(text: &str, requires_trusted: bool, prefers_composition: bool) -> InputPlan {
    if prefers_composition || !text.is_ascii() {
        InputPlan {
            mode: InputMode::Ime,
            chunk_size: 1,
            delay_ms: 0,
        }
    } else if requires_trusted {
        InputPlan {
            mode: InputMode::FastKeys,
            chunk_size: 16,
            delay_ms: 0,
        }
    } else {
        InputPlan {
            mode: InputMode::Block,
            chunk_size: text.len().max(1),
            delay_ms: 0,
        }
    }
}

pub fn adapt_chunk_size(current: usize, stable_rtt: bool, diverged: bool) -> usize {
    if diverged && current > 8 {
        current / 2
    } else if stable_rtt && current < 32 {
        current + 4
    } else {
        current
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
    let mut steps = Vec::new();
    steps.push(MicroStep::FocusCheck);
    steps.push(MicroStep::ValueCheck);
    match mode {
        InputMode::Block | InputMode::Ime => steps.push(MicroStep::Composition {
            text: text.to_owned(),
        }),
        InputMode::FastKeys | InputMode::HumanKeys | InputMode::AppAdapter => {
            for ch in text.chars() {
                steps.push(MicroStep::DispatchKey { ch });
                steps.push(MicroStep::ValueCheck);
            }
        }
    }
    Microprogram {
        steps,
        deadline_ms: 60_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn planner_chooses_modes() {
        assert_eq!(plan("hello", false, false).mode, InputMode::Block);
        assert_eq!(plan("hello", true, false).mode, InputMode::FastKeys);
        assert_eq!(plan("こんにちは", false, false).mode, InputMode::Ime);
    }
}
