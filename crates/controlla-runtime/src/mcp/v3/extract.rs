//! Extraction budgets used by direct task results.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExtractionBudget {
    pub max_items: usize,
    pub max_text_chars: usize,
    pub max_bytes: usize,
}

impl Default for ExtractionBudget {
    fn default() -> Self {
        Self {
            max_items: 100,
            max_text_chars: 2_000,
            max_bytes: 64_000,
        }
    }
}

impl ExtractionBudget {
    pub fn bounded(mut self) -> Self {
        self.max_items = self.max_items.clamp(1, 500);
        self.max_text_chars = self.max_text_chars.clamp(0, 10_000);
        self.max_bytes = self.max_bytes.clamp(4_096, 1_000_000);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extraction_budgets_clamp_to_public_bounds() {
        let budget = ExtractionBudget {
            max_items: 0,
            max_text_chars: 99_999,
            max_bytes: 1,
        }
        .bounded();
        assert_eq!(budget.max_items, 1);
        assert_eq!(budget.max_text_chars, 10_000);
        assert_eq!(budget.max_bytes, 4_096);
    }
}
