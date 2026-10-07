//! Offline, identity-bound Canva design planning. Plans are not dispatched.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct CanvaIdentity {
    pub principal: String,
    pub account_id: String,
    pub workspace_id: String,
    pub design_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct CanvaPlanBinding {
    pub identity: CanvaIdentity,
    pub session_id: String,
    pub expected_version: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanvaPageType {
    Absolute,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct CanvaSession {
    pub identity: CanvaIdentity,
    pub session_id: String,
    pub current_version: String,
    pub opened_at_ms: u64,
    pub expires_at_ms: u64,
    pub page_type: CanvaPageType,
    pub locked: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct CanvaVisualTokens {
    pub background: String,
    pub ink: String,
    pub accent: String,
    pub surface: String,
    pub typeface: String,
    pub page_margin: u16,
    pub footer: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanvaElementKind {
    Text,
    Rectangle,
    Line,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct CanvaElement {
    pub id: String,
    pub kind: CanvaElementKind,
    pub editable: bool,
    pub text: Option<String>,
    pub fill_token: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct CanvaPage {
    pub id: String,
    pub title: String,
    pub elements: Vec<CanvaElement>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanvaEffect {
    Mutation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct CanvaSyncMutation {
    pub operation: String,
    pub effect: CanvaEffect,
    pub identity: CanvaIdentity,
    pub session_id: String,
    pub expected_version: String,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct CanvaDesignPlan {
    pub title: String,
    pub width: u16,
    pub height: u16,
    pub tokens: CanvaVisualTokens,
    pub pages: Vec<CanvaPage>,
    pub sync: CanvaSyncMutation,
}

/// Builds a fixed five-page editable design plan after checking exact target and session state.
/// This is local planning only; the mutation record is not a Canva API request.
pub fn plan_heat_ready_design(
    binding: &CanvaPlanBinding,
    session: &CanvaSession,
    now_ms: u64,
) -> Result<CanvaDesignPlan, &'static str> {
    let identity = &binding.identity;
    let observed = &session.identity;
    if [
        identity.principal.as_str(),
        identity.account_id.as_str(),
        identity.workspace_id.as_str(),
        identity.design_id.as_str(),
        binding.session_id.as_str(),
        binding.expected_version.as_str(),
        observed.principal.as_str(),
        observed.account_id.as_str(),
        observed.workspace_id.as_str(),
        observed.design_id.as_str(),
        session.session_id.as_str(),
        session.current_version.as_str(),
    ]
    .iter()
    .any(|value| value.is_empty())
    {
        return Err(
            "Canva plan requires exact principal, account, workspace, design, session, and version identity",
        );
    }
    if identity != observed || binding.session_id != session.session_id {
        return Err("Canva observed identity or session does not match the requested target");
    }
    if binding.expected_version != session.current_version {
        return Err("Canva design version is stale; reobserve before planning a write");
    }
    if now_ms < session.opened_at_ms
        || now_ms >= session.expires_at_ms
        || now_ms - session.opened_at_ms >= 60_000
    {
        return Err(
            "Canva session is stale or expired; reopen and reobserve before planning a write",
        );
    }
    if session.locked || session.page_type != CanvaPageType::Absolute {
        return Err("Canva page is locked or unsupported");
    }

    let tokens = CanvaVisualTokens {
        background: "#FFF8E8".into(),
        ink: "#18323A".into(),
        accent: "#D95832".into(),
        surface: "#FFFFFF".into(),
        typeface: "DM Sans".into(),
        page_margin: 64,
        footer: "NEIGHBORHOOD HEAT-READY KIT".into(),
    };
    let pages = [
        ("01-cover", "A cooler, safer summer", "A simple heat-ready plan for every neighbor."),
        ("02-actions", "Three ways to stay safer", "Drink water often.\nCheck on a neighbor.\nCool down in an air-conditioned place."),
        ("03-checklist", "Neighborhood checklist", "☐ Water and refillable bottles\n☐ A cool place to go\n☐ Check-in buddy\n☐ Charged phone and key contacts"),
        ("04-event", "Come make a heat plan", "DATE: [Add date]\nTIME: [Add time]\nLOCATION: [Add location]\nBring a neighbor. Everyone is welcome."),
        ("05-next-steps", "Stay connected", "Choose a check-in buddy. Share the plan.\n\nCONTACT: [Add organization]\nPHONE / EMAIL: [Add contact details]"),
    ]
    .into_iter()
    .map(|(id, title, copy)| CanvaPage {
        id: id.into(),
        title: title.into(),
        elements: vec![
            CanvaElement {
                id: format!("{id}-accent"),
                kind: CanvaElementKind::Rectangle,
                editable: true,
                text: None,
                fill_token: Some("accent".into()),
            },
            CanvaElement {
                id: format!("{id}-title"),
                kind: CanvaElementKind::Text,
                editable: true,
                text: Some(title.into()),
                fill_token: Some("ink".into()),
            },
            CanvaElement {
                id: format!("{id}-body"),
                kind: CanvaElementKind::Text,
                editable: true,
                text: Some(copy.into()),
                fill_token: Some("ink".into()),
            },
            CanvaElement {
                id: format!("{id}-footer"),
                kind: CanvaElementKind::Text,
                editable: true,
                text: Some(tokens.footer.clone()),
                fill_token: Some("ink".into()),
            },
        ],
    })
    .collect();

    Ok(CanvaDesignPlan {
        title: "Neighborhood Heat-Ready Kit".into(),
        width: 1280,
        height: 720,
        tokens,
        pages,
        sync: CanvaSyncMutation {
            operation: "sync".into(),
            effect: CanvaEffect::Mutation,
            identity: identity.clone(),
            session_id: binding.session_id.clone(),
            expected_version: binding.expected_version.clone(),
            expires_at_ms: session.expires_at_ms,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (CanvaPlanBinding, CanvaSession) {
        let identity = CanvaIdentity {
            principal: "test-principal".into(),
            account_id: "test-account".into(),
            workspace_id: "test-team".into(),
            design_id: "design-42".into(),
        };
        (
            CanvaPlanBinding {
                identity: identity.clone(),
                session_id: "session-9".into(),
                expected_version: "version-3".into(),
            },
            CanvaSession {
                identity,
                session_id: "session-9".into(),
                current_version: "version-3".into(),
                opened_at_ms: 1_000,
                expires_at_ms: 60_000,
                page_type: CanvaPageType::Absolute,
                locked: false,
            },
        )
    }

    #[test]
    fn plan_is_five_ordered_editable_pages_and_marks_sync_as_mutation() {
        let (binding, session) = fixture();
        let plan = plan_heat_ready_design(&binding, &session, 2_000).unwrap();
        assert_eq!(
            plan.pages.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            [
                "01-cover",
                "02-actions",
                "03-checklist",
                "04-event",
                "05-next-steps"
            ]
        );
        assert!(
            plan.pages
                .iter()
                .flat_map(|p| &p.elements)
                .all(|e| e.editable)
        );
        assert!(
            plan.pages[3].elements.iter().any(|e| e
                .text
                .as_deref()
                .unwrap_or("")
                .contains("[Add date]"))
        );
        assert_eq!(plan.tokens.footer, "NEIGHBORHOOD HEAT-READY KIT");
        assert_eq!(plan.sync.operation, "sync");
        assert_eq!(plan.sync.effect, CanvaEffect::Mutation);
        assert_eq!(plan.sync.session_id, binding.session_id);
        assert_eq!(plan.sync.expected_version, binding.expected_version);
        assert_eq!(plan.sync.identity, binding.identity);
    }

    #[test]
    fn plan_refuses_identity_version_expiry_and_page_precondition_failures() {
        let (binding, session) = fixture();
        let mut wrong_identity = session.clone();
        wrong_identity.identity.account_id = "other-account".into();
        assert!(plan_heat_ready_design(&binding, &wrong_identity, 2_000).is_err());

        let mut wrong_session = session.clone();
        wrong_session.session_id = "other-session".into();
        assert!(plan_heat_ready_design(&binding, &wrong_session, 2_000).is_err());

        let mut stale_version = session.clone();
        stale_version.current_version = "version-4".into();
        assert!(plan_heat_ready_design(&binding, &stale_version, 2_000).is_err());
        assert!(plan_heat_ready_design(&binding, &session, 60_000).is_err());
        assert!(plan_heat_ready_design(&binding, &session, 61_000).is_err());
        assert!(plan_heat_ready_design(&binding, &session, 62_000).is_err());

        let mut locked = session.clone();
        locked.locked = true;
        assert!(plan_heat_ready_design(&binding, &locked, 2_000).is_err());
        let mut unsupported = session;
        unsupported.page_type = CanvaPageType::Unsupported;
        assert!(plan_heat_ready_design(&binding, &unsupported, 2_000).is_err());
    }
}
