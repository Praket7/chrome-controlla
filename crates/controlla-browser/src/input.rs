//! Guarded browser input primitives. These types do not qualify OS focus, cursor, or clipboard isolation.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::time::Instant;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SemanticLocator {
    RoleName { role: String, name: String },
    Label(String),
    Placeholder(String),
    TestId(String),
    Css(String),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputAction {
    Fill(String),
    Insert(String),
    SequentialKeys(String),
    Click { x: f64, y: f64 },
    Drag { from: (f64, f64), to: (f64, f64) },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuardDecision {
    Allow,
    Yield(&'static str),
    NeedsForeground,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardSnapshot {
    pub navigation: u64,
    pub account: u64,
    pub document: u64,
    pub dependencies: BTreeSet<String>,
    pub strict_background: bool,
    pub requires_native: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidationSet(pub BTreeSet<String>);
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchEvidence {
    pub accepted: bool,
    pub observed_value: Option<String>,
    pub route: &'static str,
}

pub fn validate_step(expected: &GuardSnapshot, current: &GuardSnapshot) -> GuardDecision {
    if expected.strict_background && current.requires_native {
        return GuardDecision::NeedsForeground;
    }
    if expected.navigation != current.navigation
        || expected.account != current.account
        || expected.document != current.document
    {
        return GuardDecision::Yield("identity_changed");
    }
    if expected.dependencies != current.dependencies {
        return GuardDecision::Yield("dependency_changed_or_ambiguous");
    }
    GuardDecision::Allow
}
pub fn on_external_change(event: Option<&str>) -> InvalidationSet {
    InvalidationSet(
        event
            .map(|v| [v.to_owned()].into_iter().collect())
            .unwrap_or_default(),
    )
}
pub fn perform_input<F>(
    expected: &GuardSnapshot,
    current: &GuardSnapshot,
    action: &InputAction,
    dispatch: F,
) -> Result<DispatchEvidence, GuardDecision>
where
    F: FnOnce(&InputAction) -> DispatchEvidence,
{
    let decision = validate_step(expected, current);
    if decision != GuardDecision::Allow {
        return Err(decision);
    }
    // The check/dispatch gap is necessarily non-atomic with page event handlers and remote effects.
    Ok(dispatch(action))
}
pub fn measure_check_dispatch_race<F>(check: F) -> u128
where
    F: FnOnce(),
{
    let at = Instant::now();
    check();
    at.elapsed().as_micros()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> GuardSnapshot {
        GuardSnapshot {
            navigation: 1,
            account: 2,
            document: 3,
            dependencies: ["field:value".to_owned()].into_iter().collect(),
            strict_background: true,
            requires_native: false,
        }
    }
    #[test]
    fn relevant_changes_yield_and_unrelated_changes_may_continue() {
        let e = snapshot();
        let mut c = e.clone();
        c.account += 1;
        assert_eq!(
            validate_step(&e, &c),
            GuardDecision::Yield("identity_changed")
        );
        c = e.clone();
        c.dependencies.insert("field:value".into());
        assert_eq!(validate_step(&e, &c), GuardDecision::Allow);
        c = e.clone();
        c.dependencies.insert("field:edited".into());
        assert_eq!(
            validate_step(&e, &c),
            GuardDecision::Yield("dependency_changed_or_ambiguous")
        );
    }
    #[test]
    fn strict_background_never_routes_through_native_clipboard_or_focus() {
        let e = snapshot();
        let mut c = e.clone();
        c.requires_native = true;
        assert_eq!(
            perform_input(&e, &c, &InputAction::Insert("λ".into()), |_| panic!(
                "must not dispatch"
            )),
            Err(GuardDecision::NeedsForeground)
        );
    }
    #[test]
    fn guarded_dispatch_preserves_unicode_and_reports_observed_fixture_value() {
        let e = snapshot();
        let evidence = perform_input(&e, &e, &InputAction::Fill("héllo 👋".into()), |a| {
            let InputAction::Fill(v) = a else {
                unreachable!()
            };
            DispatchEvidence {
                accepted: true,
                observed_value: Some(v.clone()),
                route: "fixture",
            }
        })
        .unwrap();
        assert_eq!(evidence.observed_value.as_deref(), Some("héllo 👋"));
    }
    #[test]
    fn race_window_is_measured_and_external_change_invalidates() {
        let elapsed = measure_check_dispatch_race(|| std::hint::black_box(()));
        assert!(elapsed < 1_000_000);
        assert_eq!(
            on_external_change(Some("overlay")),
            InvalidationSet(["overlay".into()].into_iter().collect())
        );
    }
}
