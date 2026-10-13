use controlla_runtime::observation_router::{
    ObservationDecision, ObservationMode, ObservationRequest, choose_observation,
};

#[test]
fn observation_router_uses_compact_then_escalates_for_ambiguity_or_layout() {
    assert_eq!(
        choose_observation(ObservationRequest::default()).mode,
        ObservationMode::SemanticCompact
    );
    assert_eq!(
        choose_observation(ObservationRequest {
            ambiguity: 0.4,
            ..Default::default()
        })
        .mode,
        ObservationMode::SemanticRich
    );
    let decision = choose_observation(ObservationRequest {
        ambiguity: 0.8,
        needs_layout: true,
        ..Default::default()
    });
    assert_eq!(decision.mode, ObservationMode::SemanticRich);
    assert_eq!(
        decision.reason,
        "coverage or ambiguity requires a richer semantic view"
    );
}

#[test]
fn explicit_frame_and_scope_choices_have_stable_precedence() {
    assert_eq!(
        choose_observation(ObservationRequest {
            frame_id: true,
            needs_layout: true,
            ambiguity: 1.0,
            ..Default::default()
        }),
        ObservationDecision {
            mode: ObservationMode::AccessibilityFrame,
            reason: "caller selected a child accessibility frame"
        }
    );
    assert_eq!(
        choose_observation(ObservationRequest {
            scoped_selector: true,
            rich_context: true,
            ..Default::default()
        })
        .mode,
        ObservationMode::HtmlScoped
    );
    assert_eq!(
        choose_observation(ObservationRequest {
            needs_layout: true,
            ambiguity: 0.9,
            scoped_selector: true,
            ..Default::default()
        })
        .mode,
        ObservationMode::VisualTargeted
    );
}

#[test]
fn task_and_context_profiles_route_scoped_dashboard_observations() {
    let decision = choose_observation(ObservationRequest {
        task_class: controlla_runtime::observation_router::TaskClass::FilterSortDashboard,
        model_profile: Some(controlla_runtime::observation_router::ModelProfile::RichContext),
        scoped_selector: true,
        ..Default::default()
    });
    assert_eq!(decision.mode, ObservationMode::HtmlScoped);
    assert!(decision.reason.contains("dashboard"));
    assert_eq!(
        choose_observation(ObservationRequest {
            task_class: controlla_runtime::observation_router::TaskClass::Mutation,
            ..Default::default()
        })
        .mode,
        ObservationMode::SemanticRich
    );
}

#[test]
fn evidence_relevance_filter_matches_the_query_without_reclassifying_page_text() {
    let mut trail = controlla_runtime::visual_probe::EvidenceTrail::default();
    trail.record(
        "page_text",
        None,
        None,
        "Buffalo Bills injury report updated today",
        false,
    );
    trail.record(
        "page_text",
        None,
        None,
        "Weather forecast for tomorrow",
        false,
    );
    let relevant = trail.recent_relevant("Buffalo Bills latest injury report", 8);
    assert_eq!(relevant.len(), 1);
    assert_eq!(
        relevant[0].fact,
        "Buffalo Bills injury report updated today"
    );
    assert!(
        !relevant[0].verified,
        "relevance filtering must not upgrade page evidence trust"
    );
}
