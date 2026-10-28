//! Public workflow control-flow limits. The existing VM owns execution; this
//! module supplies the bounded shape that the public compiler may emit.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlFlowLimits {
    pub max_foreach_items: usize,
    pub max_nesting: usize,
    pub max_steps: usize,
}

impl Default for ControlFlowLimits {
    fn default() -> Self {
        Self {
            max_foreach_items: 32,
            max_nesting: 16,
            max_steps: 128,
        }
    }
}

pub fn validate_loop(
    items: usize,
    nesting: usize,
    limits: ControlFlowLimits,
) -> Result<(), &'static str> {
    if items == 0 || items > limits.max_foreach_items {
        return Err("foreach requires 1..=32 concrete items");
    }
    if nesting == 0 || nesting > limits.max_nesting {
        return Err("workflow nesting must be 1..=16");
    }
    Ok(())
}

pub fn validate_fallback(primary_steps: usize, fallback_steps: usize) -> Result<(), &'static str> {
    if primary_steps == 0 || fallback_steps == 0 {
        return Err("try/fallback requires both branches");
    }
    if primary_steps.saturating_add(fallback_steps) > 64 {
        return Err("try/fallback branches exceed the expanded-step bound");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_bounds_are_explicit() {
        let limits = ControlFlowLimits::default();
        assert!(validate_loop(1, 1, limits).is_ok());
        assert!(validate_loop(0, 1, limits).is_err());
        assert!(validate_loop(33, 1, limits).is_err());
        assert!(validate_loop(1, 17, limits).is_err());
    }

    #[test]
    fn fallback_branches_are_both_bounded() {
        assert!(validate_fallback(1, 1).is_ok());
        assert!(validate_fallback(0, 1).is_err());
        assert!(validate_fallback(40, 40).is_err());
    }
}
