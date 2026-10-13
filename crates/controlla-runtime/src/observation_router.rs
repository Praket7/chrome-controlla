use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationMode {
    SemanticCompact,
    SemanticRich,
    AccessibilityFrame,
    HtmlScoped,
    VisualTargeted,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TaskClass {
    #[default]
    General,
    Research,
    FilterSortDashboard,
    Mutation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelProfile {
    CompactContext,
    RichContext,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ObservationRequest {
    pub task_class: TaskClass,
    pub model_profile: Option<ModelProfile>,
    pub exhaustive: bool,
    pub needs_layout: bool,
    pub ambiguity: f32,
    pub rich_context: bool,
    pub frame_id: bool,
    pub scoped_selector: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationDecision {
    pub mode: ObservationMode,
    pub reason: &'static str,
}

pub fn choose_observation(request: ObservationRequest) -> ObservationDecision {
    let (mode, reason) = if request.frame_id {
        (
            ObservationMode::AccessibilityFrame,
            "caller selected a child accessibility frame",
        )
    } else if request.needs_layout && request.ambiguity > 0.6 && request.scoped_selector {
        (
            ObservationMode::VisualTargeted,
            "layout ambiguity requires a bounded targeted visual probe",
        )
    } else if request.task_class == TaskClass::FilterSortDashboard
        && request.model_profile == Some(ModelProfile::RichContext)
        && request.scoped_selector
    {
        (
            ObservationMode::HtmlScoped,
            "dashboard task and rich-context profile benefit from scoped structure",
        )
    } else if request.scoped_selector && (request.rich_context || request.needs_layout) {
        (
            ObservationMode::HtmlScoped,
            "caller requested a bounded scoped HTML view",
        )
    } else if request.exhaustive
        || request.ambiguity > 0.3
        || request.rich_context
        || request.task_class == TaskClass::Mutation
    {
        (
            ObservationMode::SemanticRich,
            if request.scoped_selector {
                "scoped semantic view with richer context was requested"
            } else {
                "coverage or ambiguity requires a richer semantic view"
            },
        )
    } else {
        (
            ObservationMode::SemanticCompact,
            "compact semantic evidence is sufficient",
        )
    };
    ObservationDecision { mode, reason }
}
