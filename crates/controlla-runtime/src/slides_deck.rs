//! Offline compiler for the Phase 8 Google Slides acceptance brief.
//!
//! It builds a revision-bound API request only. It has no transport or OAuth path.

use crate::apps::{Qualification, SlidesBinding};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
pub struct SlidesDeckPlan {
    pub principal: String,
    pub account_id: String,
    pub presentation_id: String,
    pub required_revision_id: String,
    pub slide_titles: Vec<String>,
    pub object_ids: Vec<String>,
    pub request_body: Value,
    pub qualification: Qualification,
}

/// Compile the checked-in 10-slide Urban Heat brief into editable native shapes and text.
/// Chart figures are intentionally illustrative and are not presented as local measurements.
pub fn compile_urban_heat_deck(binding: &SlidesBinding) -> Result<SlidesDeckPlan, &'static str> {
    if binding.principal.is_empty()
        || binding.account_id.is_empty()
        || binding.presentation_id.is_empty()
        || binding.required_revision_id.is_empty()
    {
        return Err("Slides deck requires principal, account, presentation, and revision identity");
    }
    if !safe_segment(&binding.presentation_id) || binding.required_revision_id.len() > 256 {
        return Err("Slides presentation identifier is unsafe or revision identifier is too long");
    }

    let slides = [
        (
            "Urban Heat: A Neighborhood Field Guide",
            "A practical guide to understanding heat risk and taking three neighborhood actions.",
        ),
        (
            "What is urban heat?",
            "Pavement, roofs, and buildings absorb and release heat. Dense areas can stay warmer than nearby greener places, especially after sunset.",
        ),
        (
            "Why neighborhoods differ",
            "Shade, tree cover, building materials, traffic, and access to cooling all shape how hot a block feels.",
        ),
        (
            "A simple example of afternoon surface temperature",
            "ILLUSTRATIVE VALUES ONLY — not local measurements. Example surfaces: shaded grass 75°F; tree shade 82°F; asphalt 105°F; dark roof 110°F. Surface temperature differs from air temperature.",
        ),
        (
            "Heat can affect health",
            "High heat can cause dehydration, heat exhaustion, and heat stroke. Some medicines and health conditions increase risk. Check on one another and seek urgent help for severe symptoms.",
        ),
        (
            "Who may face greater exposure?",
            "Outdoor workers, older adults, infants, people with chronic illness, residents without reliable cooling, and people who spend time in areas with little shade may face added risk.",
        ),
        (
            "Action 1: Make shade where people walk",
            "Map the hottest walking routes. Protect existing trees, plant suitable trees where they can thrive, and add maintained shade at stops and gathering places.",
        ),
        (
            "Action 2: Keep buildings cooler",
            "Ask building owners about reflective or green roof options, exterior shade, insulation, and efficient cooling. Choose upgrades that fit the building and local conditions.",
        ),
        (
            "Action 3: Organize a neighborhood heat plan",
            "1. Identify hot routes and vulnerable neighbors.  2. Choose one shade or building project and find local partners.  3. Share cooling locations, check-in plans, and emergency guidance before the next heat wave.",
        ),
        (
            "Sources and a next step",
            "Learn more: U.S. EPA, Heat Island Effect — https://www.epa.gov/heatislands\nNASA, Urban Heat Islands — https://earthobservatory.nasa.gov/features/UrbanHeat\nCDC, About Heat and Your Health — https://www.cdc.gov/heat-health/about/index.html\nNext step: invite neighbors to map one hot route and choose one practical project.",
        ),
    ];

    let mut requests = Vec::new();
    let mut object_ids = Vec::new();
    let mut slide_titles = Vec::new();
    for (index, (title, body)) in slides.iter().enumerate() {
        let n = index + 1;
        let slide_id = format!("cc_heat_s{n:02}");
        slide_titles.push((*title).to_string());
        requests.push(json!({"createSlide": {
            "objectId": slide_id,
            "slideLayoutReference": {"predefinedLayout": "BLANK"}
        }}));
        add_text_box(
            &mut requests,
            &mut object_ids,
            &slide_id,
            &format!("cc_heat_s{n:02}_title"),
            title,
            TextBoxLayout {
                x: 42.0,
                y: 34.0,
                width: 636.0,
                height: 64.0,
                font_size: 24.0,
                bold: true,
            },
        );
        let body_height = if n == 4 {
            72.0
        } else if n == 10 {
            215.0
        } else {
            245.0
        };
        add_text_box(
            &mut requests,
            &mut object_ids,
            &slide_id,
            &format!("cc_heat_s{n:02}_body"),
            body,
            TextBoxLayout {
                x: 48.0,
                y: 112.0,
                width: 624.0,
                height: body_height,
                font_size: 16.0,
                bold: false,
            },
        );
        if n == 4 {
            add_illustrative_chart(&mut requests, &mut object_ids, &slide_id);
        }
    }

    Ok(SlidesDeckPlan {
        principal: binding.principal.clone(),
        account_id: binding.account_id.clone(),
        presentation_id: binding.presentation_id.clone(),
        required_revision_id: binding.required_revision_id.clone(),
        slide_titles,
        object_ids,
        request_body: json!({
            "requests": requests,
            "writeControl": {"requiredRevisionId": binding.required_revision_id}
        }),
        qualification: Qualification::RequiresConnection,
    })
}

struct TextBoxLayout {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    font_size: f64,
    bold: bool,
}

fn add_text_box(
    requests: &mut Vec<Value>,
    ids: &mut Vec<String>,
    slide_id: &str,
    object_id: &str,
    text: &str,
    layout: TextBoxLayout,
) {
    ids.push(object_id.to_string());
    requests.push(create_shape(
        slide_id,
        object_id,
        "TEXT_BOX",
        layout.x,
        layout.y,
        layout.width,
        layout.height,
    ));
    requests.push(json!({"insertText": {"objectId": object_id, "text": text}}));
    requests.push(json!({"updateTextStyle": {
        "objectId": object_id,
        "textRange": {"type": "ALL"},
        "style": {
            "fontFamily": "Arial",
            "fontSize": {"magnitude": layout.font_size, "unit": "PT"},
            "bold": layout.bold,
            "foregroundColor": {"opaqueColor": {"rgbColor": if layout.bold { rgb(0.08, 0.24, 0.29) } else { rgb(0.16, 0.20, 0.21) }}}
        },
        "fields": "fontFamily,fontSize,bold,foregroundColor"
    }}));
}

fn add_illustrative_chart(requests: &mut Vec<Value>, ids: &mut Vec<String>, slide_id: &str) {
    let bars = [
        ("grass", 75.0, 0.35),
        ("tree shade", 82.0, 0.48),
        ("asphalt", 105.0, 0.78),
        ("dark roof", 110.0, 0.86),
    ];
    let base_y = 348.0;
    let left = 72.0;
    let gap = 148.0;
    for (index, (label, value, intensity)) in bars.iter().enumerate() {
        let x = left + gap * index as f64;
        let bar_height = (value - 60.0) * 2.0;
        let bar_y = base_y - bar_height;
        let id = format!("cc_heat_s04_bar{}", index + 1);
        ids.push(id.clone());
        requests.push(create_shape(
            slide_id,
            &id,
            "RECTANGLE",
            x,
            bar_y,
            86.0,
            bar_height,
        ));
        requests.push(json!({"updateShapeProperties": {
            "objectId": id,
            "shapeProperties": {"shapeBackgroundFill": {"solidFill": {"color": {"rgbColor": rgb(0.10 + intensity * 0.75, 0.45 - intensity * 0.30, 0.40 - intensity * 0.30)}}}},
            "fields": "shapeBackgroundFill.solidFill.color"
        }}));
        let value_id = format!("cc_heat_s04_value{}", index + 1);
        add_text_box(
            requests,
            ids,
            slide_id,
            &value_id,
            &format!("{value:.0}°F"),
            TextBoxLayout {
                x,
                y: bar_y - 27.0,
                width: 86.0,
                height: 26.0,
                font_size: 13.0,
                bold: true,
            },
        );
        let label_id = format!("cc_heat_s04_label{}", index + 1);
        add_text_box(
            requests,
            ids,
            slide_id,
            &label_id,
            label,
            TextBoxLayout {
                x: x - 10.0,
                y: base_y + 4.0,
                width: 106.0,
                height: 32.0,
                font_size: 11.0,
                bold: false,
            },
        );
    }
}

fn create_shape(
    slide_id: &str,
    object_id: &str,
    shape_type: &str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Value {
    json!({"createShape": {
        "objectId": object_id,
        "shapeType": shape_type,
        "elementProperties": {
            "pageObjectId": slide_id,
            "size": {"width": {"magnitude": width, "unit": "PT"}, "height": {"magnitude": height, "unit": "PT"}},
            "transform": {"scaleX": 1, "scaleY": 1, "translateX": x, "translateY": y, "unit": "PT"}
        }
    }})
}

fn rgb(red: f64, green: f64, blue: f64) -> Value {
    json!({"red": red, "green": green, "blue": blue})
}

fn safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> SlidesBinding {
        SlidesBinding {
            principal: "p".into(),
            account_id: "a".into(),
            presentation_id: "test_deck-1".into(),
            required_revision_id: "rev_7".into(),
        }
    }

    #[test]
    fn compiles_ten_editable_slides_with_revision_bound_illustrative_chart_and_sources() {
        let plan = compile_urban_heat_deck(&binding()).unwrap();
        assert_eq!(plan.slide_titles.len(), 10);
        assert_eq!(
            plan.object_ids
                .iter()
                .filter(|id| id.starts_with("cc_heat_s04_bar"))
                .count(),
            4
        );
        assert_eq!(
            plan.request_body["writeControl"]["requiredRevisionId"],
            "rev_7"
        );
        let requests = plan.request_body["requests"].as_array().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.get("createSlide").is_some())
                .count(),
            10
        );
        assert!(requests.iter().any(|r| {
            r.get("insertText")
                .and_then(|v| v.get("text"))
                .and_then(Value::as_str)
                .is_some_and(|s| s.contains("ILLUSTRATIVE VALUES ONLY"))
        }));
        assert!(requests.iter().any(|r| {
            r.get("insertText")
                .and_then(|v| v.get("text"))
                .and_then(Value::as_str)
                .is_some_and(|s| s.contains("https://www.epa.gov/heatislands"))
        }));
        assert!(requests.iter().all(|r| {
            !r.get("createShape")
                .is_some_and(|v| v.get("shapeType").and_then(Value::as_str) == Some("IMAGE"))
        }));
        assert_eq!(plan.qualification, Qualification::RequiresConnection);
    }

    #[test]
    fn stable_safe_object_ids_and_request_order_are_deterministic() {
        let first = compile_urban_heat_deck(&binding()).unwrap();
        let second = compile_urban_heat_deck(&binding()).unwrap();
        assert_eq!(first.object_ids, second.object_ids);
        assert_eq!(first.request_body, second.request_body);
        assert!(first.object_ids.iter().all(|id| safe_segment(id)));
    }

    #[test]
    fn rejects_missing_or_unsafe_binding_before_plan_construction() {
        let mut bad = binding();
        bad.required_revision_id.clear();
        assert!(compile_urban_heat_deck(&bad).is_err());
        let mut bad = binding();
        bad.presentation_id = "id/path".into();
        assert!(compile_urban_heat_deck(&bad).is_err());
    }
}
