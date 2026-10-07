//! App-specific preflight contracts. These build exact plans but do not connect to vendor APIs.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum App {
    GoogleSlides,
    Canva,
    CapCutWeb,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Qualification {
    FixtureOnly,
    RequiresConnection,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppCapability {
    pub operation: String,
    pub route: String,
    pub qualification: Qualification,
    pub reason: String,
}

/// Describes only implemented route contracts; it never upgrades fixture evidence to live support.
pub fn capabilities(app: App) -> Vec<AppCapability> {
    match app {
        App::GoogleSlides => ["replace_range", "insert_into_object"]
            .into_iter()
            .map(|operation| AppCapability {
                operation: operation.into(),
                route: "slides_api_batch_update".into(),
                qualification: Qualification::RequiresConnection,
                reason: "OAuth, exact presentation identity, and revision-bound API dispatch are not connected".into(),
            })
            .collect(),
        App::Canva => vec![AppCapability {
            operation: "edit_page_and_sync".into(),
            route: "canva_apps_sdk".into(),
            qualification: Qualification::RequiresConnection,
            reason: "A hosted Canva Apps SDK session is required; sync is a write and must be authorized as one".into(),
        }],
        App::CapCutWeb => ["import_media", "trim", "split", "correct_captions", "export"]
            .into_iter()
            .map(|operation| AppCapability {
                operation: operation.into(),
                route: "bounded_chrome_ui_recipe".into(),
                qualification: Qualification::Unsupported,
                reason: "No live, versioned control map or independent timeline/export verifier is qualified".into(),
            })
            .collect(),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SlidesBinding {
    pub principal: String,
    pub account_id: String,
    pub presentation_id: String,
    pub required_revision_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SlidesTextEdit {
    ReplaceRange {
        object_id: String,
        start_index: usize,
        end_index: usize,
        expected_text: String,
        new_text: String,
    },
    InsertIntoObject {
        object_id: String,
        text: String,
        insertion_index: usize,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SlidesPlan {
    pub principal: String,
    pub account_id: String,
    pub presentation_id: String,
    pub required_revision_id: String,
    pub method: String,
    pub http_method: String,
    pub url: String,
    pub content_type: String,
    pub required_oauth_scopes: Vec<String>,
    pub request_body: Value,
    pub precondition: Value,
    pub qualification: Qualification,
}

/// Builds an atomic Slides API request with a required revision. The returned request still needs
/// OAuth and authenticated dispatch; a successful plan is not a completed edit.
pub fn plan_slides_text_edit(
    binding: &SlidesBinding,
    edit: SlidesTextEdit,
) -> Result<SlidesPlan, &'static str> {
    if binding.principal.is_empty()
        || binding.account_id.is_empty()
        || binding.presentation_id.is_empty()
        || binding.required_revision_id.is_empty()
    {
        return Err("Slides edit requires principal, account, presentation, and revision identity");
    }
    let (requests, precondition) = match edit {
        SlidesTextEdit::ReplaceRange {
            object_id,
            start_index,
            end_index,
            expected_text,
            new_text,
        } => {
            if !valid_object_id(&object_id)
                || start_index < 1
                || end_index <= start_index
                || end_index - start_index != expected_text.encode_utf16().count()
                || expected_text.is_empty()
                || expected_text.len() > 32_768
                || new_text.is_empty()
                || new_text.len() > 32_768
            {
                return Err(
                    "Slides replacement range requires an exact object, UTF-16 span, and bounded text",
                );
            }
            (
                json!([
                    {"deleteText": {"objectId":object_id,"textRange":{"type":"FIXED_RANGE","startIndex":start_index,"endIndex":end_index}}},
                    {"insertText": {"objectId":object_id,"text":new_text,"insertionIndex":start_index}}
                ]),
                json!({"object_id":object_id,"text_range":[start_index,end_index],"expected_text":expected_text}),
            )
        }
        SlidesTextEdit::InsertIntoObject {
            object_id,
            text,
            insertion_index,
        } => {
            if !valid_object_id(&object_id)
                || text.is_empty()
                || text.len() > 32_768
                || insertion_index > 1_000_000
            {
                return Err("Slides object insertion has an invalid ID, text, or insertion index");
            }
            (
                json!([{"insertText":{"objectId":object_id,"text":text,"insertionIndex":insertion_index}}]),
                json!({"object_id":object_id,"insertion_index":insertion_index}),
            )
        }
    };
    Ok(SlidesPlan {
        principal: binding.principal.clone(),
        account_id: binding.account_id.clone(),
        presentation_id: binding.presentation_id.clone(),
        required_revision_id: binding.required_revision_id.clone(),
        method: "presentations.batchUpdate".into(),
        http_method: "POST".into(),
        url: format!(
            "https://slides.googleapis.com/v1/presentations/{}:batchUpdate",
            encode_path_segment(&binding.presentation_id)
        ),
        content_type: "application/json".into(),
        required_oauth_scopes: vec!["https://www.googleapis.com/auth/presentations".into()],
        request_body: json!({
            "requests": requests,
            "writeControl": {"requiredRevisionId": binding.required_revision_id}
        }),
        precondition,
        qualification: Qualification::RequiresConnection,
    })
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    encoded
}

fn valid_object_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-:".contains(&byte))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CanvaPageType {
    Absolute,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CanvaSessionSnapshot {
    pub principal: String,
    pub account_id: String,
    pub design_id: String,
    pub page_id: String,
    pub revision: String,
    pub page_type: CanvaPageType,
    pub locked: bool,
    pub opened_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CanvaSyncPlan {
    pub principal: String,
    pub account_id: String,
    pub design_id: String,
    pub page_id: String,
    pub expected_revision: String,
    pub effect: String,
    pub qualification: Qualification,
}

/// Validates Canva session preconditions. Calling sync applies edits, so this plan is always a
/// mutation and is never returned for a stale, locked, or unsupported page.
pub fn plan_canva_sync(
    snapshot: &CanvaSessionSnapshot,
    now_ms: u64,
) -> Result<CanvaSyncPlan, &'static str> {
    if snapshot.principal.is_empty()
        || snapshot.account_id.is_empty()
        || snapshot.design_id.is_empty()
        || snapshot.page_id.is_empty()
        || snapshot.revision.is_empty()
    {
        return Err(
            "Canva sync requires exact account, design, page, revision, and principal identity",
        );
    }
    if now_ms < snapshot.opened_at_ms || now_ms - snapshot.opened_at_ms >= 60_000 {
        return Err("Canva editing session is stale; reopen and reobserve before planning a write");
    }
    if snapshot.page_type != CanvaPageType::Absolute || snapshot.locked {
        return Err("Canva page is locked or unsupported");
    }
    Ok(CanvaSyncPlan {
        principal: snapshot.principal.clone(),
        account_id: snapshot.account_id.clone(),
        design_id: snapshot.design_id.clone(),
        page_id: snapshot.page_id.clone(),
        expected_revision: snapshot.revision.clone(),
        effect: "external_write".into(),
        qualification: Qualification::RequiresConnection,
    })
}

/// CapCut recipes intentionally stay unexecutable until live controls and independent outcomes
/// are observed for the target version. This prevents generic labels from becoming fake support.
pub fn capcut_web_plan(operation: &str) -> Result<Value, &'static str> {
    let allowed = [
        "import_media",
        "trim",
        "split",
        "correct_captions",
        "export",
    ];
    if !allowed.contains(&operation) {
        return Err("unsupported CapCut Web operation");
    }
    Err("CapCut Web operation requires a live versioned control map and verifier")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slides_binding() -> SlidesBinding {
        SlidesBinding {
            principal: "test-principal".into(),
            account_id: "test-account".into(),
            presentation_id: "presentation-1".into(),
            required_revision_id: "revision-7".into(),
        }
    }

    #[test]
    fn capability_operations_match_planning_routes_and_remain_unqualified() {
        let capabilities = capabilities(App::GoogleSlides);
        assert_eq!(
            capabilities
                .iter()
                .map(|c| c.operation.as_str())
                .collect::<Vec<_>>(),
            ["replace_range", "insert_into_object"]
        );
        assert!(
            capabilities
                .iter()
                .all(|c| c.qualification == Qualification::RequiresConnection)
        );
    }

    #[test]
    fn slides_plan_uses_object_scoped_utf16_range_and_required_revision() {
        let plan = plan_slides_text_edit(
            &slides_binding(),
            SlidesTextEdit::ReplaceRange {
                object_id: "shape_1".into(),
                start_index: 1,
                end_index: 6,
                expected_text: "Draft".into(),
                new_text: "Final".into(),
            },
        )
        .unwrap();
        assert_eq!(plan.presentation_id, "presentation-1");
        assert_eq!(plan.http_method, "POST");
        assert_eq!(
            plan.url,
            "https://slides.googleapis.com/v1/presentations/presentation-1:batchUpdate"
        );
        assert_eq!(plan.content_type, "application/json");
        assert_eq!(
            plan.required_oauth_scopes,
            ["https://www.googleapis.com/auth/presentations"]
        );
        assert_eq!(
            plan.request_body["writeControl"]["requiredRevisionId"],
            "revision-7"
        );
        assert_eq!(plan.request_body["requests"].as_array().unwrap().len(), 2);
        assert_eq!(
            plan.request_body["requests"][0]["deleteText"]["objectId"],
            "shape_1"
        );
        assert_eq!(
            plan.request_body["requests"][1]["insertText"]["insertionIndex"],
            1
        );
        assert_eq!(plan.precondition["expected_text"], "Draft");
        assert_eq!(plan.qualification, Qualification::RequiresConnection);
    }

    #[test]
    fn slides_endpoint_encodes_presentation_id_as_one_path_segment() {
        let mut binding = slides_binding();
        binding.presentation_id = "a/b?c".into();
        let plan = plan_slides_text_edit(
            &binding,
            SlidesTextEdit::InsertIntoObject {
                object_id: "shape_1".into(),
                text: "x".into(),
                insertion_index: 0,
            },
        )
        .unwrap();
        assert!(plan.url.ends_with("/a%2Fb%3Fc:batchUpdate"));
    }

    #[test]
    fn slides_plan_rejects_missing_identity_and_unsafe_object_ids() {
        let mut binding = slides_binding();
        binding.required_revision_id.clear();
        assert!(
            plan_slides_text_edit(
                &binding,
                SlidesTextEdit::ReplaceRange {
                    object_id: "shape_1".into(),
                    start_index: 1,
                    end_index: 2,
                    expected_text: "a".into(),
                    new_text: "b".into()
                }
            )
            .is_err()
        );
        let supplementary = plan_slides_text_edit(
            &slides_binding(),
            SlidesTextEdit::ReplaceRange {
                object_id: "shape_1".into(),
                start_index: 1,
                end_index: 3,
                expected_text: "😀".into(),
                new_text: "x".into(),
            },
        )
        .unwrap();
        assert_eq!(supplementary.precondition["text_range"], json!([1, 3]));
        assert!(
            plan_slides_text_edit(
                &slides_binding(),
                SlidesTextEdit::ReplaceRange {
                    object_id: "shape_1".into(),
                    start_index: 1,
                    end_index: 4,
                    expected_text: "😀".into(),
                    new_text: "x".into()
                }
            )
            .is_err(),
            "range lengths use UTF-16 indexing"
        );
        assert!(
            plan_slides_text_edit(
                &slides_binding(),
                SlidesTextEdit::InsertIntoObject {
                    object_id: "bad id".into(),
                    text: "x".into(),
                    insertion_index: 0
                }
            )
            .is_err()
        );
    }

    #[test]
    fn canva_sync_is_write_and_rejects_stale_locked_and_unsupported_sessions() {
        let snapshot = CanvaSessionSnapshot {
            principal: "p".into(),
            account_id: "a".into(),
            design_id: "d".into(),
            page_id: "pg".into(),
            revision: "r1".into(),
            page_type: CanvaPageType::Absolute,
            locked: false,
            opened_at_ms: 100,
        };
        let plan = plan_canva_sync(&snapshot, 10_000).unwrap();
        assert_eq!(plan.effect, "external_write");
        assert!(plan_canva_sync(&snapshot, 60_100).is_err());
        assert!(
            plan_canva_sync(
                &CanvaSessionSnapshot {
                    locked: true,
                    ..snapshot.clone()
                },
                10_000
            )
            .is_err()
        );
        assert!(
            plan_canva_sync(
                &CanvaSessionSnapshot {
                    page_type: CanvaPageType::Unsupported,
                    ..snapshot
                },
                10_000
            )
            .is_err()
        );
    }

    #[test]
    fn capcut_does_not_advertise_unqualified_controls_as_implemented() {
        for operation in [
            "import_media",
            "trim",
            "split",
            "correct_captions",
            "export",
        ] {
            assert_eq!(
                capcut_web_plan(operation),
                Err("CapCut Web operation requires a live versioned control map and verifier")
            );
        }
        assert_eq!(
            capcut_web_plan("delete_account"),
            Err("unsupported CapCut Web operation")
        );
        assert!(
            capabilities(App::CapCutWeb)
                .iter()
                .all(|capability| capability.qualification == Qualification::Unsupported)
        );
    }
}
