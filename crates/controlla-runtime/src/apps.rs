//! App-specific preflight contracts. These build exact plans but do not connect to vendor APIs.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Discovers a Canva slide manifest from one fresh, complete accessibility snapshot.
/// Candidate identities and selected state are observed UI signals only; this does
/// not bind screenshots to a render or establish hidden pages/persistence.
pub fn verify_canva_slide_discovery_v1(
    predicate: &Value,
    items: &BTreeMap<String, Value>,
    scope_complete: bool,
    page_url: &str,
) -> Result<(bool, Value), &'static str> {
    if predicate["version"].as_u64() != Some(1) {
        return Err("Canva slide discovery version must be 1");
    }
    let expected = predicate["expected_count"]
        .as_u64()
        .filter(|count| (1..=100).contains(count))
        .ok_or("Canva discovery expected_count must be 1..100")? as usize;
    let design_url = predicate["design_url"]
        .as_str()
        .filter(|url| url.len() <= 2048 && url.starts_with("https://www.canva.com/design/"))
        .ok_or("Canva discovery requires an exact HTTPS design_url")?;
    let mut found = BTreeMap::<usize, Value>::new();
    let mut references = std::collections::BTreeSet::new();
    let mut stable_ids = std::collections::BTreeSet::new();
    for (reference, item) in items {
        let role = item["role"].as_str().unwrap_or_default();
        if !matches!(role, "button" | "tab" | "option" | "link") {
            continue;
        }
        let name = item["name"].as_str().unwrap_or_default();
        let href = item["href"].as_str();
        let id = item["id"].as_str();
        let identity = href
            .filter(|value| value.split('#').next() == Some(design_url))
            .map(|value| ("href", value))
            .or_else(|| id.map(|value| ("id", value)));
        let Some((id_field, stable_id)) = identity else {
            continue;
        };
        let page_number = href
            .and_then(|value| value.split("#page=").nth(1))
            .and_then(|value| value.split(['&', '#']).next())
            .and_then(|value| value.parse::<usize>().ok())
            .or_else(|| {
                name.strip_prefix("Page ")
                    .or_else(|| name.strip_prefix("Slide "))
                    .and_then(|value| value.split([' ', ',', ':']).next())
                    .and_then(|value| value.parse::<usize>().ok())
            });
        let Some(index) = page_number.filter(|index| (1..=expected).contains(index)) else {
            continue;
        };
        if reference.is_empty()
            || reference.len() > 128
            || name.trim().is_empty()
            || name.len() > 512
            || stable_id.is_empty()
            || stable_id.len() > 1024
        {
            return Err(
                "Canva slide candidate lacks a bounded reference, name, or stable identity",
            );
        }
        if !references.insert(reference) || !stable_ids.insert((id_field, stable_id)) {
            return Err(
                "Canva slide discovery is ambiguous: duplicate reference or stable identity",
            );
        }
        if found.insert(index, json!({"reference":reference,"index":index,"name":name,"role":role,"id_field":id_field,"stable_id":stable_id,"active":selected_state(item)})).is_some() {
            return Err("Canva slide discovery is ambiguous: multiple visible controls identify the same page");
        }
    }
    let active_count = found
        .values()
        .filter(|slide| slide["active"] == true)
        .count();
    if active_count > 1 {
        return Err("Canva slide discovery is ambiguous: multiple thumbnails are selected");
    }
    let missing = (1..=expected)
        .filter(|index| !found.contains_key(index))
        .collect::<Vec<_>>();
    let slides = found.into_values().collect::<Vec<_>>();
    let url_match = page_url == design_url;
    let complete = scope_complete && missing.is_empty() && active_count == 1 && url_match;
    let completeness = if !scope_complete {
        "incomplete_snapshot"
    } else if !url_match {
        "design_url_mismatch"
    } else if !missing.is_empty() {
        "missing_slides"
    } else if active_count == 0 {
        "active_slide_unobserved"
    } else {
        "complete"
    };
    Ok((
        complete,
        json!({"adapter":"canva_slide_discovery_v1","evidence_scope":"fresh_complete_accessibility_snapshot_ui_only","design_url":design_url,"page_url":page_url,"expected_count":expected,"observed_count":slides.len(),"missing_slide_indexes":missing,"scope_complete":scope_complete,"active_slide_count":active_count,"completeness":completeness,"slides":slides,"render_freshness":"unproven","hidden_pages":"not_claimed"}),
    ))
}

/// Evaluates the versioned Canva semantic slide manifest against one fresh snapshot.
/// The result proves caller-visible UI correspondence only, never hidden slide
/// state, a saved design, or server persistence.
pub fn verify_canva_slide_manifest_v1(
    predicate: &Value,
    items: &BTreeMap<String, Value>,
    scope_complete: bool,
    page_url: &str,
) -> Result<(bool, Value), &'static str> {
    if predicate["version"].as_u64() != Some(1) {
        return Err("Canva slide manifest version must be 1");
    }
    validate_canva_slide_manifest_contract(predicate)?;
    let expected_count = predicate["expected_count"]
        .as_u64()
        .filter(|n| (1..=100).contains(n))
        .ok_or("Canva manifest expected_count must be 1..100")? as usize;
    let design_url = predicate["design_url"]
        .as_str()
        .filter(|s| s.len() <= 2048 && s.starts_with("https://www.canva.com/design/"))
        .ok_or("Canva manifest requires an exact HTTPS design_url")?;
    let slides = predicate["slides"]
        .as_array()
        .filter(|a| a.len() == expected_count)
        .ok_or("Canva slide list must equal expected_count and stay within the 100-slide bound")?;
    let mut refs = std::collections::BTreeSet::new();
    let mut stable_ids = std::collections::BTreeSet::new();
    let mut observed = Vec::with_capacity(slides.len());
    let mut passed = scope_complete && page_url == design_url;
    let mut previous_item_order = None;
    for (position, slide) in slides.iter().enumerate() {
        let reference = bounded_ref(slide.get("reference").and_then(Value::as_str))
            .ok_or("Canva slide requires a bounded semantic reference")?;
        let index = slide["index"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("Canva slide index must be positive")? as usize;
        let name = slide["name"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.len() <= 512)
            .ok_or("Canva slide name must contain 1..512 bytes")?;
        let id_field = slide["id_field"]
            .as_str()
            .filter(|s| matches!(*s, "href" | "id"))
            .ok_or("Canva slide requires observed href or id as its stable identity field")?;
        let stable_id = slide["stable_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 1024)
            .ok_or("Canva slide stable_id must contain 1..1024 bytes")?;
        if index != position + 1
            || !refs.insert(reference)
            || !stable_ids.insert((id_field, stable_id))
        {
            return Err(
                "Canva manifest references and stable slide IDs must be unique and indexes ordered 1..N",
            );
        }
        let item = items.get(reference);
        let item_order = item.and_then(|x| x["index"].as_u64());
        let order_match = item_order
            .is_some_and(|current| previous_item_order.is_none_or(|previous| current > previous));
        previous_item_order = item_order;
        let role = item.and_then(|x| x["role"].as_str()).unwrap_or_default();
        let expected_role = slide["role"].as_str().unwrap_or_default();
        let name_match = item.and_then(|x| x["name"].as_str()) == Some(name);
        let id_match = item.and_then(|x| x[id_field].as_str()) == Some(stable_id);
        let role_match =
            matches!(expected_role, "button" | "tab" | "option" | "link") && role == expected_role;
        passed &= order_match && name_match && id_match && role_match;
        observed.push(json!({"reference":reference,"index":index,"observed_item_order":item_order,"name":item.and_then(|x|x["name"].as_str()),"expected_role":expected_role,"role":role,"stable_id_field":id_field,"stable_id":item.and_then(|x|x[id_field].as_str()),"matched":order_match && name_match && id_match && role_match,
            "image_sources":slide["image_sources"],"ai_generated_image_labels":slide["ai_generated_image_labels"],
            "source_metadata_scope":"caller_declared_not_independently_verified"}));
    }
    let missing_slides = observed
        .iter()
        .filter(|slide| slide["matched"] != true)
        .map(|slide| slide["index"].clone())
        .collect::<Vec<_>>();
    Ok((
        passed,
        json!({"adapter":"canva_slide_manifest_v1","evidence_scope":"caller_visible_ui_only","page_url":page_url,"design_url":design_url,"expected_count":expected_count,"observed_count":slides.len(),"missing_slide_indexes":missing_slides,"scope_complete":scope_complete,"slides":observed}),
    ))
}

/// Verifies that the exact caller-grounded stable slide identity is currently
/// selected in a fresh complete visible snapshot. This proves UI selection only.
pub fn verify_canva_active_slide_v1(
    predicate: &Value,
    items: &BTreeMap<String, Value>,
    scope_complete: bool,
    page_url: &str,
) -> Result<(bool, Value), &'static str> {
    if predicate["version"].as_u64() != Some(1) {
        return Err("Canva active-slide predicate version must be 1");
    }
    let design_url = predicate["design_url"]
        .as_str()
        .filter(|url| url.len() <= 2048 && url.starts_with("https://www.canva.com/design/"))
        .ok_or("Canva active-slide predicate requires an exact HTTPS design_url")?;
    let id_field = predicate["id_field"]
        .as_str()
        .filter(|field| matches!(*field, "href" | "id"))
        .ok_or("Canva active-slide predicate requires href or id identity")?;
    let stable_id = predicate["stable_id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 1024)
        .ok_or("Canva active-slide predicate requires a bounded stable identity")?;
    let name = predicate["name"]
        .as_str()
        .filter(|name| !name.trim().is_empty() && name.len() <= 512)
        .ok_or("Canva active-slide predicate requires a bounded slide name")?;
    let matching = items
        .values()
        .filter(|item| item[id_field].as_str() == Some(stable_id))
        .collect::<Vec<_>>();
    let selected = matching
        .iter()
        .filter(|item| {
            item["name"].as_str() == Some(name)
                && matches!(
                    item["role"].as_str(),
                    Some("button" | "tab" | "option" | "link")
                )
                && selected_state(item)
        })
        .count();
    let passed = scope_complete && page_url == design_url && matching.len() == 1 && selected == 1;
    Ok((
        passed,
        json!({"adapter":"canva_active_slide_v1","evidence_scope":"caller_visible_ui_only","page_url":page_url,"design_url":design_url,"stable_id_field":id_field,"stable_id":stable_id,"name":name,"matching_controls":matching.len(),"selected_matching_controls":selected,"scope_complete":scope_complete,"selected":passed}),
    ))
}

/// Checks caller-visible class, assignment, account, and attachment references
/// in a fresh complete semantic snapshot. This is not server acceptance or proof
/// that an upload was persisted.
pub fn verify_classroom_attachment_v1(
    predicate: &Value,
    items: &BTreeMap<String, Value>,
    scope_complete: bool,
) -> Result<(bool, Value), &'static str> {
    if predicate["version"].as_u64() != Some(1) {
        return Err("Classroom attachment predicate version must be 1");
    }
    let account = identity_item(&predicate["account"], items, "account")?;
    let class = identity_item(&predicate["class"], items, "class")?;
    let assignment = identity_item(&predicate["assignment"], items, "assignment")?;
    let stage = predicate["stage"].as_str().unwrap_or("attachment");
    if !matches!(stage, "destination" | "attachment") {
        return Err("Classroom attachment stage must be destination or attachment");
    }
    let identity_refs = [account.0, class.0, assignment.0];
    let identities_unique = identity_refs
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        == 3;
    if stage == "destination" {
        return Ok((
            scope_complete && identities_unique && account.1 && class.1 && assignment.1,
            json!({"adapter":"google_classroom_attachment_v1","stage":"destination","evidence_scope":"caller_visible_ui_only","scope_complete":scope_complete,"account":account.2,"class":class.2,"assignment":assignment.2,"server_acceptance_proven":false,"persistence_proven":false}),
        ));
    }
    let attachment_ref = bounded_ref(predicate["attachment"]["reference"].as_str())
        .ok_or("Classroom attachment requires a bounded semantic reference")?;
    let filename = predicate["attachment"]["filename"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 512)
        .ok_or("attachment filename must contain 1..512 bytes")?;
    let size = predicate["attachment"]["size"]
        .as_u64()
        .ok_or("attachment size must be an exact nonnegative integer")?;
    let attachment = items.get(attachment_ref);
    let role = attachment
        .and_then(|x| x["role"].as_str())
        .unwrap_or_default();
    let observed_name =
        attachment.and_then(|x| x["filename"].as_str().or_else(|| x["name"].as_str()));
    let observed_size = attachment.and_then(|x| x["size"].as_u64());
    let attachment_match = matches!(role, "attachment" | "listitem" | "option")
        && observed_name == Some(filename)
        && observed_size == Some(size);
    let refs_unique = [account.0, class.0, assignment.0, attachment_ref]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        == 4;
    let passed =
        scope_complete && refs_unique && account.1 && class.1 && assignment.1 && attachment_match;
    Ok((
        passed,
        json!({"adapter":"google_classroom_attachment_v1","stage":"attachment","evidence_scope":"caller_visible_ui_only","scope_complete":scope_complete,"account":account.2,"class":class.2,"assignment":assignment.2,"attachment":{"reference":attachment_ref,"filename":observed_name,"size":observed_size,"role":role,"matched":attachment_match},"server_acceptance_proven":false,"persistence_proven":false}),
    ))
}

/// Builds generic workflow steps for a caller-grounded Canva slide review.
/// The references and ordered manifest must come from one complete fresh snapshot.
/// Each slide is selected and captured through the existing generic workflow; this
/// does not establish save state or server persistence, and it intentionally does
/// not restore a prior selection that may have changed through human input.
pub fn canva_slide_review_steps_v1(predicate: &Value) -> Result<Vec<Value>, &'static str> {
    validate_canva_slide_manifest_contract(predicate)?;
    let slides = predicate["slides"].as_array().unwrap();
    let restore = predicate.get("restore_after_review");
    if let Some(restore) = restore {
        let name = restore["name"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.len() <= 512)
            .ok_or("Canva restore_after_review requires a bounded slide name")?;
        let id_field = restore["id_field"]
            .as_str()
            .filter(|s| matches!(*s, "href" | "id"))
            .ok_or("Canva restore_after_review requires href or id identity")?;
        let stable_id = restore["stable_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 1024)
            .ok_or("Canva restore_after_review requires a bounded stable identity")?;
        let matches = slides
            .iter()
            .filter(|slide| {
                slide["name"].as_str() == Some(name)
                    && slide["id_field"].as_str() == Some(id_field)
                    && slide["stable_id"].as_str() == Some(stable_id)
            })
            .count();
        if matches != 1 {
            return Err(
                "Canva restore_after_review must identify exactly one slide in the verified manifest",
            );
        }
    }
    let step_count = slides.len() * 5 + 1 + usize::from(restore.is_some()) * 3;
    if step_count > 256 {
        return Err("Canva slide review exceeds the 256-step workflow command bound");
    }
    let mut steps = Vec::with_capacity(step_count);
    steps.push(json!({"kind":"verify","predicate":{"kind":"app_specific","adapter":"canva_slide_manifest_v1","predicate":predicate}}));
    for slide in slides {
        let index = slide["index"].as_u64().unwrap();
        let name = slide["name"].as_str().unwrap();
        let reference_variable = format!("canva_slide_ref_{index}");
        steps.push(json!({"kind":"find","query":name,"role":slide["role"],"match_mode":"exact","save_as":reference_variable}));
        steps.push(json!({"kind":"click","reference":format!("${reference_variable}")}));
        steps.push(json!({"kind":"verify","predicate":{"kind":"app_specific","adapter":"canva_active_slide_v1","predicate":{"version":1,"design_url":predicate["design_url"],"name":name,"id_field":slide["id_field"],"stable_id":slide["stable_id"]}}}));
        let evidence_context = json!({"kind":"canva_slide_capture","design_url":predicate["design_url"],
                "slide_name":name,"stable_id_field":slide["id_field"],"stable_id":slide["stable_id"],
                "identity_scope":"runtime_verified_selection_before_and_after_capture","renderer_freshness":"two_animation_frames_before_compositor_capture",
                "image_sources":slide["image_sources"],"ai_generated_image_labels":slide["ai_generated_image_labels"],
                "source_metadata_scope":"caller_declared_not_independently_verified"}).to_string();
        steps.push(
            json!({"kind":"probe","save_artifact":true,"omit_image_payload":false,
            "evidence_context":evidence_context,
            "save_as":format!("canva_slide_{index}")}),
        );
        steps.push(json!({"kind":"verify","predicate":{"kind":"app_specific","adapter":"canva_active_slide_v1","predicate":{"version":1,"design_url":predicate["design_url"],"name":name,"id_field":slide["id_field"],"stable_id":slide["stable_id"]}}}));
    }
    if let Some(restore) = restore {
        let name = restore["name"].as_str().unwrap();
        let id_field = restore["id_field"].as_str().unwrap();
        let stable_id = restore["stable_id"].as_str().unwrap();
        let restore_slide = slides
            .iter()
            .find(|slide| {
                slide["name"].as_str() == Some(name)
                    && slide["id_field"].as_str() == Some(id_field)
                    && slide["stable_id"].as_str() == Some(stable_id)
            })
            .unwrap();
        let restore_ref = format!(
            "canva_restore_ref_{}",
            restore_slide["index"].as_u64().unwrap()
        );
        // The preceding post-capture verification is the ownership fence: if a
        // human changed the selected slide during review, this workflow stops and
        // never performs the restoration click.
        steps.push(json!({"kind":"find","query":name,"role":restore_slide["role"],"match_mode":"exact","save_as":restore_ref}));
        steps.push(json!({"kind":"click","reference":format!("${restore_ref}")}));
        steps.push(json!({"kind":"verify","predicate":{"kind":"app_specific","adapter":"canva_active_slide_v1","predicate":{"version":1,"design_url":predicate["design_url"],"name":name,"id_field":id_field,"stable_id":stable_id}}}));
    }
    Ok(steps)
}

/// Builds a caller-grounded Canva draft edit followed by a visible save-state
/// check. The confirmation is UI evidence only; unknown outcomes stop for manual
/// review and are never automatically undone or replayed.
pub fn canva_draft_edit_save_steps_v1(predicate: &Value) -> Result<Vec<Value>, &'static str> {
    if predicate["version"].as_u64() != Some(1) {
        return Err("Canva draft edit/save contract version must be 1");
    }
    let design_url = predicate["design_url"]
        .as_str()
        .filter(|url| url.len() <= 2048 && url.starts_with("https://www.canva.com/design/"))
        .ok_or("Canva draft edit/save requires an exact HTTPS design URL")?;
    let fields = predicate["fields"]
        .as_array()
        .filter(|fields| !fields.is_empty() && fields.len() <= 32)
        .ok_or("Canva draft edit/save requires 1..32 explicit fields")?;
    let mut refs = std::collections::BTreeSet::new();
    let mut compiled_fields = Vec::with_capacity(fields.len());
    for field in fields {
        let reference = bounded_ref(field["reference"].as_str())
            .ok_or("Canva edit field requires a bounded fresh reference")?;
        let expected_value = field["expected_value"]
            .as_str()
            .filter(|value| value.len() <= 16_384)
            .ok_or("Canva edit field requires a bounded expected value")?;
        let value = field["value"]
            .as_str()
            .filter(|value| value.len() <= 16_384)
            .ok_or("Canva edit field requires a bounded replacement value")?;
        if !refs.insert(reference) {
            return Err("Canva draft edit references must be unique");
        }
        compiled_fields.push(json!({"reference":reference,"expected_value":expected_value,"value":value,"select":false}));
    }
    let save_reference = bounded_ref(predicate["save_control_reference"].as_str())
        .ok_or("Canva draft edit/save requires a fresh save-control reference")?;
    let selector = predicate["saved_status"]["selector"]
        .as_str()
        .filter(|selector| !selector.trim().is_empty() && selector.len() <= 512)
        .ok_or("Canva draft edit/save requires a bounded visible save-status selector")?;
    let text = predicate["saved_status"]["text"]
        .as_str()
        .filter(|text| !text.trim().is_empty() && text.len() <= 512)
        .ok_or("Canva draft edit/save requires bounded saved-status text")?;
    if refs.contains(save_reference) {
        return Err("Canva save control cannot also be an edited field");
    }
    Ok(vec![
        json!({"kind":"verify","predicate":{"kind":"url","equals":design_url}}),
        json!({"kind":"fill_form","fields":compiled_fields}),
        json!({"kind":"click","reference":save_reference,"outcome":{"kind":"text","selector":selector,"text":text}}),
        json!({"kind":"verify","predicate":{"kind":"text_present","scope":selector,"text":text}}),
    ])
}

/// Builds the safe generic upload sequence for a Classroom draft. It requires a caller
/// supplied artifact, exact account marker, destination identities, and a visible
/// attachment predicate. It verifies destination identities before upload and
/// includes post-upload visible attachment acceptance. It never creates a publish/post step.
pub fn classroom_attachment_step_v1(
    artifact_handle: &str,
    trigger_reference: &str,
    account_selector: &str,
    account_expected: &str,
    predicate: &Value,
) -> Result<Value, &'static str> {
    if artifact_handle.is_empty()
        || artifact_handle.len() > 256
        || artifact_handle.chars().any(char::is_control)
    {
        return Err("Classroom upload requires a bounded artifact handle");
    }
    if bounded_ref(Some(trigger_reference)).is_none() {
        return Err("Classroom upload requires a bounded fresh trigger reference");
    }
    if account_selector.is_empty()
        || account_selector.len() > 512
        || account_selector.chars().any(char::is_control)
        || account_expected.trim().is_empty()
        || account_expected.len() > 512
        || account_expected.chars().any(char::is_control)
    {
        return Err("Classroom upload requires a bounded exact visible account marker");
    }
    validate_classroom_attachment_contract(predicate)?;
    let destination_predicate = json!({"version":1,"stage":"destination","account":predicate["account"],"class":predicate["class"],"assignment":predicate["assignment"]});
    Ok(
        json!([{"kind":"verify","predicate":{"kind":"app_specific","adapter":"google_classroom_attachment_v1","predicate":destination_predicate}},
        {"kind":"upload","artifact_handle":artifact_handle,"trigger_reference":trigger_reference,
        "account_marker":[account_selector,account_expected],
        "acceptance":{"kind":"app_specific","adapter":"google_classroom_attachment_v1","predicate":predicate}}]),
    )
}

/// Plans selection of one exact, visible Drive item inside an already-open
/// Google picker. This never signs in, infers an account from a URL index, or
/// claims that the destination application accepted or persisted the item.
pub fn google_drive_picker_steps_v1(predicate: &Value) -> Result<Vec<Value>, &'static str> {
    if predicate["version"].as_u64() != Some(1) {
        return Err("Google Drive picker contract version must be 1");
    }
    let picker_url = predicate["picker_url"]
        .as_str()
        .filter(|url| is_google_picker_url(url))
        .ok_or("Drive picker requires the exact observed HTTPS Google picker URL")?;
    let account_ref = bounded_ref(predicate["account"]["reference"].as_str())
        .ok_or("Drive picker requires a bounded visible account reference")?;
    let account_name = predicate["account"]["name"]
        .as_str()
        .filter(|value| !value.trim().is_empty() && value.len() <= 512)
        .ok_or("Drive picker requires an exact visible account name")?;
    let account_field = predicate["account"]["id_field"]
        .as_str()
        .filter(|field| matches!(*field, "href" | "id"))
        .ok_or("Drive picker account requires observed href or id identity")?;
    let account_id = predicate["account"]["stable_id"]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 1024)
        .ok_or("Drive picker account requires an exact stable identity")?;
    let file = &predicate["file"];
    let file_ref = bounded_ref(file["reference"].as_str())
        .ok_or("Drive picker requires a bounded visible file reference")?;
    let file_name = file["name"]
        .as_str()
        .filter(|value| !value.trim().is_empty() && value.len() <= 512)
        .ok_or("Drive picker requires an exact visible filename")?;
    let file_field = file["id_field"]
        .as_str()
        .filter(|field| matches!(*field, "href" | "id"))
        .ok_or("Drive picker file requires observed href or id identity")?;
    let file_id = file["stable_id"]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 1024)
        .ok_or("Drive picker file requires an exact stable identity")?;
    let role = file["role"]
        .as_str()
        .filter(|role| matches!(*role, "button" | "row" | "option" | "gridcell"))
        .ok_or("Drive picker file requires its observed selectable role")?;
    let confirm = &predicate["confirm"];
    let confirm_ref = bounded_ref(confirm["reference"].as_str())
        .ok_or("Drive picker requires a fresh confirmation-control reference")?;
    let destination = predicate
        .get("destination")
        .ok_or("Drive picker requires a destination acceptance predicate")?;
    let destination_url = destination["url"]
        .as_str()
        .filter(|url| is_exact_https_url(url))
        .ok_or("Drive picker destination requires an exact credential-free HTTPS URL")?;
    let destination_account_ref = bounded_ref(destination["account"]["reference"].as_str())
        .ok_or("Drive destination requires a bounded visible account reference")?;
    let destination_attachment_ref =
        bounded_ref(destination["attachment"]["reference"].as_str())
            .ok_or("Drive destination requires a bounded attached-file reference")?;
    let destination_attachment_name = destination["attachment"]["name"]
        .as_str()
        .filter(|value| !value.trim().is_empty() && value.len() <= 512)
        .ok_or("Drive destination requires the exact attached filename")?;
    let destination_attachment_size = destination["attachment"]["size"]
        .as_u64()
        .ok_or("Drive destination requires the exact attached file size")?;
    let destination_attachment_role = destination["attachment"]["role"]
        .as_str()
        .filter(|role| matches!(*role, "attachment" | "listitem" | "option"))
        .ok_or("Drive destination requires the observed attachment role")?;
    let mut refs = std::collections::BTreeSet::new();
    for reference in [
        account_ref,
        file_ref,
        confirm_ref,
        destination_account_ref,
        destination_attachment_ref,
    ] {
        if !refs.insert(reference) {
            return Err("Drive picker and destination references must be distinct");
        }
    }
    let selection = json!({"version":1,"stage":"selection","picker_url":picker_url,
        "account":{"reference":account_ref,"name":account_name,"id_field":account_field,"stable_id":account_id},
        "file":{"reference":file_ref,"name":file_name,"id_field":file_field,"stable_id":file_id,"role":role}});
    let selected = json!({"version":1,"stage":"selected","picker_url":picker_url,
        "account":{"reference":account_ref,"name":account_name,"id_field":account_field,"stable_id":account_id},
        "file":{"reference":file_ref,"name":file_name,"id_field":file_field,"stable_id":file_id,"role":role}});
    Ok(vec![
        json!({"kind":"verify","predicate":{"kind":"app_specific","adapter":"google_drive_picker_v1","predicate":selection}}),
        json!({"kind":"click","reference":file_ref}),
        json!({"kind":"verify","predicate":{"kind":"app_specific","adapter":"google_drive_picker_v1","predicate":selected}}),
        json!({"kind":"click","reference":confirm_ref,"outcome":{"kind":"focused"}}),
        json!({"kind":"verify","predicate":{"kind":"app_specific","adapter":"google_drive_attachment_v1","predicate":{
            "version":1,"url":destination_url,"account":destination["account"],
            "attachment":{"reference":destination_attachment_ref,"name":destination_attachment_name,
                "size":destination_attachment_size,"role":destination_attachment_role}
        }}}),
    ])
}

/// Verifies that the exact picked resource is now shown as an attachment in its
/// destination. This is caller-visible application acceptance, not persistence.
pub fn verify_google_drive_attachment_v1(
    predicate: &Value,
    items: &BTreeMap<String, Value>,
    scope_complete: bool,
    page_url: &str,
) -> Result<(bool, Value), &'static str> {
    if predicate["version"].as_u64() != Some(1) {
        return Err("Google Drive attachment contract version must be 1");
    }
    let destination_url = predicate["url"]
        .as_str()
        .filter(|url| is_exact_https_url(url))
        .ok_or("Google Drive attachment requires an exact credential-free HTTPS URL")?;
    let account = identity_item(&predicate["account"], items, "account")?;
    let attachment_ref = bounded_ref(predicate["attachment"]["reference"].as_str())
        .ok_or("Google Drive attachment requires a bounded visible reference")?;
    let attachment_name = predicate["attachment"]["name"]
        .as_str()
        .filter(|value| !value.trim().is_empty() && value.len() <= 512)
        .ok_or("Google Drive attachment requires an exact filename")?;
    let attachment_size = predicate["attachment"]["size"]
        .as_u64()
        .ok_or("Google Drive attachment requires an exact nonnegative size")?;
    let attachment_role = predicate["attachment"]["role"]
        .as_str()
        .filter(|role| matches!(*role, "attachment" | "listitem" | "option"))
        .ok_or("Google Drive attachment requires its observed semantic role")?;
    let item = items.get(attachment_ref);
    let item_name = item.and_then(|value| {
        value["filename"]
            .as_str()
            .or_else(|| value["name"].as_str())
    });
    let item_size = item.and_then(|value| value["size"].as_u64());
    let item_role = item.and_then(|value| value["role"].as_str());
    let attachment_matches = item_name == Some(attachment_name)
        && item_size == Some(attachment_size)
        && item_role == Some(attachment_role);
    let distinct_references = account.0 != attachment_ref;
    let destination_matches = page_url == destination_url;
    let accepted = scope_complete
        && destination_matches
        && distinct_references
        && account.1
        && attachment_matches;
    Ok((
        accepted,
        json!({"adapter":"google_drive_attachment_v1","evidence_scope":"caller_visible_destination_ui_only",
        "scope_complete":scope_complete,"destination_url":destination_url,"page_url":page_url,
        "account":account.2,"attachment":{"reference":attachment_ref,"name":item_name,
            "size":item_size,"role":item_role,"matched":attachment_matches},
        "destination_acceptance_proven":accepted,"server_acceptance_proven":false,"persistence_proven":false}),
    ))
}

/// Verifies exact account and resource identity in a complete visible picker
/// snapshot. `closed` means the picker disappeared after confirmation; it does
/// not prove host-app acceptance or persistence.
pub fn verify_google_drive_picker_v1(
    predicate: &Value,
    items: &BTreeMap<String, Value>,
    scope_complete: bool,
    page_url: &str,
) -> Result<(bool, Value), &'static str> {
    if predicate["version"].as_u64() != Some(1) {
        return Err("Google Drive picker predicate version must be 1");
    }
    let stage = predicate["stage"].as_str().unwrap_or_default();
    if !matches!(stage, "selection" | "selected") {
        return Err("Google Drive picker stage must be selection or selected");
    }
    let picker_url = predicate["picker_url"]
        .as_str()
        .filter(|url| is_google_picker_url(url))
        .ok_or("Google Drive picker requires an exact observed HTTPS Google URL")?;
    let account = identity_item(&predicate["account"], items, "Drive account")?;
    let file_ref = bounded_ref(predicate["file"]["reference"].as_str())
        .ok_or("Drive picker file requires a bounded reference")?;
    let file_name = predicate["file"]["name"]
        .as_str()
        .filter(|name| !name.trim().is_empty() && name.len() <= 512)
        .ok_or("Drive picker file requires an exact name")?;
    let id_field = predicate["file"]["id_field"]
        .as_str()
        .filter(|field| matches!(*field, "href" | "id"))
        .ok_or("Drive picker file requires href or id identity")?;
    let stable_id = predicate["file"]["stable_id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 1024)
        .ok_or("Drive picker file requires an exact stable identity")?;
    let role = predicate["file"]["role"]
        .as_str()
        .filter(|role| matches!(*role, "button" | "row" | "option" | "gridcell"))
        .ok_or("Drive picker file requires its observed selectable role")?;
    let item = items.get(file_ref);
    let file_matches = item.is_some_and(|item| {
        item["name"].as_str() == Some(file_name)
            && item["role"].as_str() == Some(role)
            && item[id_field].as_str() == Some(stable_id)
    });
    let selected = item.is_some_and(|item| item["selected"].as_str() == Some("true"));
    let url_match = page_url == picker_url;
    let passed = scope_complete
        && account.1
        && url_match
        && match stage {
            "selection" => file_matches,
            "selected" => file_matches && selected,
            _ => false,
        };
    Ok((
        passed,
        json!({"adapter":"google_drive_picker_v1","stage":stage,
        "evidence_scope":"caller_visible_ui_only","scope_complete":scope_complete,
        "picker_url":picker_url,"page_url":page_url,"account":account.2,
        "file":{"reference":file_ref,"name":item.and_then(|x|x["name"].as_str()),
            "stable_id_field":id_field,"stable_id":item.and_then(|x|x[id_field].as_str()),
            "role":item.and_then(|x|x["role"].as_str()),"matched":file_matches,"selected":selected},
        "picker_closed":false,"destination_acceptance_proven":false,"persistence_proven":false}),
    ))
}

fn is_google_picker_url(value: &str) -> bool {
    if value.len() > 2048 {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|host| host == "google.com" || host.ends_with(".google.com"))
}

fn is_exact_https_url(value: &str) -> bool {
    if value.len() > 2048 {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
}

fn validate_canva_slide_manifest_contract(predicate: &Value) -> Result<(), &'static str> {
    if predicate["version"].as_u64() != Some(1) {
        return Err("Canva slide manifest version must be 1");
    }
    let count = predicate["expected_count"]
        .as_u64()
        .filter(|n| (1..=100).contains(n))
        .ok_or("Canva manifest expected_count must be 1..100")? as usize;
    let slides = predicate["slides"]
        .as_array()
        .filter(|slides| slides.len() == count)
        .ok_or("Canva slide list must equal expected_count and stay within the 100-slide bound")?;
    let mut refs = std::collections::BTreeSet::new();
    let mut stable_ids = std::collections::BTreeSet::new();
    for (position, slide) in slides.iter().enumerate() {
        for field in ["image_sources", "ai_generated_image_labels"] {
            if let Some(values) = slide.get(field)
                && (!values.is_array()
                    || values.as_array().is_some_and(|values| {
                        values.len() > 64
                            || values.iter().any(|value| {
                                value
                                    .as_str()
                                    .is_none_or(|text| text.trim().is_empty() || text.len() > 2048)
                            })
                    }))
            {
                return Err(
                    "Canva source metadata must contain at most 64 bounded nonempty strings",
                );
            }
        }
        if bounded_ref(slide["reference"].as_str()).is_none()
            || slide["index"].as_u64() != Some((position + 1) as u64)
            || slide["name"]
                .as_str()
                .is_none_or(|name| name.trim().is_empty() || name.len() > 512)
            || !matches!(
                slide["role"].as_str(),
                Some("button" | "tab" | "option" | "link")
            )
            || !matches!(slide["id_field"].as_str(), Some("href" | "id"))
            || slide["stable_id"]
                .as_str()
                .is_none_or(|id| id.is_empty() || id.len() > 1024)
            || !refs.insert(slide["reference"].as_str().unwrap())
            || !stable_ids.insert((
                slide["id_field"].as_str().unwrap(),
                slide["stable_id"].as_str().unwrap(),
            ))
        {
            return Err(
                "Canva slide review requires unique ordered references and stable identities",
            );
        }
    }
    if predicate["design_url"]
        .as_str()
        .is_none_or(|url| url.len() > 2048 || !url.starts_with("https://www.canva.com/design/"))
    {
        return Err("Canva manifest requires an exact HTTPS design_url");
    }
    Ok(())
}

fn validate_classroom_attachment_contract(predicate: &Value) -> Result<(), &'static str> {
    if predicate["version"].as_u64() != Some(1) {
        return Err("Classroom attachment predicate version must be 1");
    }
    for label in ["account", "class", "assignment"] {
        let value = &predicate[label];
        if bounded_ref(value["reference"].as_str()).is_none()
            || value["name"]
                .as_str()
                .is_none_or(|name| name.trim().is_empty() || name.len() > 512)
            || !matches!(value["id_field"].as_str(), Some("href" | "id"))
            || value["stable_id"]
                .as_str()
                .is_none_or(|id| id.is_empty() || id.len() > 1024)
        {
            return Err("Classroom requires exact account, class, and assignment identities");
        }
    }
    if bounded_ref(predicate["attachment"]["reference"].as_str()).is_none()
        || predicate["attachment"]["filename"]
            .as_str()
            .is_none_or(|name| name.trim().is_empty() || name.len() > 512)
        || predicate["attachment"]["size"].as_u64().is_none()
    {
        return Err("Classroom requires exact attachment reference, filename, and size");
    }
    let refs = ["account", "class", "assignment"]
        .map(|key| predicate[key]["reference"].as_str().unwrap())
        .into_iter()
        .chain([predicate["attachment"]["reference"].as_str().unwrap()])
        .collect::<std::collections::BTreeSet<_>>();
    if refs.len() != 4 {
        return Err("Classroom identity and attachment references must be unique");
    }
    Ok(())
}

fn bounded_ref(value: Option<&str>) -> Option<&str> {
    value.filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
}

fn selected_state(item: &Value) -> bool {
    item["selected"].as_bool() == Some(true) || item["selected"].as_str() == Some("true")
}

fn identity_item<'a>(
    value: &'a Value,
    items: &'a BTreeMap<String, Value>,
    label: &str,
) -> Result<(&'a str, bool, Value), &'static str> {
    let reference = bounded_ref(value["reference"].as_str()).ok_or(match label {
        "account" => "Classroom account requires a bounded reference",
        "class" => "Classroom class requires a bounded reference",
        _ => "Classroom assignment requires a bounded reference",
    })?;
    let name = value["name"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 512)
        .ok_or("identity name must contain 1..512 bytes")?;
    let id_field = value["id_field"]
        .as_str()
        .filter(|s| matches!(*s, "href" | "id"))
        .ok_or("identity requires observed href or id as its stable identity field")?;
    let stable_id = value["stable_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1024)
        .ok_or("identity stable_id must contain 1..1024 bytes")?;
    let item = items.get(reference);
    let role = item.and_then(|x| x["role"].as_str()).unwrap_or_default();
    let matched = item.and_then(|x| x["name"].as_str()) == Some(name)
        && item.and_then(|x| x[id_field].as_str()) == Some(stable_id)
        && !matches!(role, "alert" | "status" | "log");
    Ok((
        reference,
        matched,
        json!({"reference":reference,"name":item.and_then(|x|x["name"].as_str()),"role":role,"stable_id_field":id_field,"stable_id":item.and_then(|x|x[id_field].as_str()),"matched":matched}),
    ))
}

#[cfg(test)]
mod semantic_adapter_tests {
    use super::*;
    use serde_json::json;

    fn slide_predicate() -> Value {
        json!({"version":1,"design_url":"https://www.canva.com/design/abc/edit","expected_count":2,"slides":[
            {"reference":"@c1","index":1,"name":"Cover","role":"button","id_field":"href","stable_id":"https://www.canva.com/design/abc/edit#page=1","image_sources":["https://cdn.example.test/cover.png"],"ai_generated_image_labels":["cover illustration"]},
            {"reference":"@c2","index":2,"name":"Plan","role":"tab","id_field":"href","stable_id":"https://www.canva.com/design/abc/edit#page=2"}
        ]})
    }
    fn slide_items() -> BTreeMap<String, Value> {
        [
            ("@c1".into(), json!({"index":1,"role":"button","name":"Cover","href":"https://www.canva.com/design/abc/edit#page=1"})),
            ("@c2".into(), json!({"index":2,"role":"tab","name":"Plan","href":"https://www.canva.com/design/abc/edit#page=2"})),
        ].into_iter().collect()
    }

    #[test]
    fn canva_discovery_derives_ordered_ids_and_fails_closed_on_gaps_or_incomplete_scope() {
        let items = [
            ("@t1".into(), json!({"index":4,"role":"button","name":"Cover","href":"https://www.canva.com/design/abc/edit#page=1","selected":true})),
            ("@t2".into(), json!({"index":8,"role":"button","name":"Plan","href":"https://www.canva.com/design/abc/edit#page=2","selected":false})),
            ("@other".into(), json!({"index":9,"role":"button","name":"Share","href":"https://www.canva.com/design/abc/edit#share","selected":false})),
        ].into_iter().collect();
        let predicate = json!({"version":1,"design_url":"https://www.canva.com/design/abc/edit","expected_count":2});
        let (passed, evidence) = verify_canva_slide_discovery_v1(
            &predicate,
            &items,
            true,
            "https://www.canva.com/design/abc/edit",
        )
        .unwrap();
        assert!(passed);
        assert_eq!(
            evidence["slides"][0]["stable_id"],
            "https://www.canva.com/design/abc/edit#page=1"
        );
        assert_eq!(evidence["slides"][0]["active"], true);
        assert_eq!(evidence["missing_slide_indexes"], json!([]));
        let (passed, gap) = verify_canva_slide_discovery_v1(
            &predicate,
            &BTreeMap::from([("@t1".into(), items["@t1"].clone())]),
            true,
            "https://www.canva.com/design/abc/edit",
        )
        .unwrap();
        assert!(!passed);
        assert_eq!(gap["missing_slide_indexes"], json!([2]));
        let (passed, partial) = verify_canva_slide_discovery_v1(
            &predicate,
            &items,
            false,
            "https://www.canva.com/design/abc/edit",
        )
        .unwrap();
        assert!(!passed);
        assert_eq!(partial["completeness"], "incomplete_snapshot");
        let mut ambiguous = items.clone();
        ambiguous.insert("@duplicate".into(), items["@t1"].clone());
        assert!(
            verify_canva_slide_discovery_v1(
                &predicate,
                &ambiguous,
                true,
                "https://www.canva.com/design/abc/edit"
            )
            .is_err()
        );
    }

    #[test]
    fn adapters_accept_fields_from_the_public_snapshot_schema() {
        let snapshot_source = include_str!("shared_page/snapshot.js");
        assert!(snapshot_source.contains("id:e.id || null,size:fileSizeOf(e)"));
        assert!(snapshot_source.contains("getAttribute?.('data-file-size')"));
        let canva_snapshot: Value = serde_json::from_str(&json!({"items":[
            {"reference":"@slide1","index":1,"role":"button","name":"Cover","href":"https://www.canva.com/design/abc/edit#page=1","selected":"true","id":"page-1","size":null},
            {"reference":"@slide2","index":2,"role":"button","name":"Plan","href":"https://www.canva.com/design/abc/edit#page=2","selected":"false","id":"page-2","size":null}
        ]}).to_string()).unwrap();
        let canva_items = canva_snapshot["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| (item["reference"].as_str().unwrap().to_owned(), item.clone()))
            .collect::<BTreeMap<_, _>>();
        let discovery = json!({"version":1,"design_url":"https://www.canva.com/design/abc/edit","expected_count":2});
        let (passed, evidence) = verify_canva_slide_discovery_v1(
            &discovery,
            &canva_items,
            true,
            "https://www.canva.com/design/abc/edit",
        )
        .unwrap();
        assert!(
            passed,
            "ARIA selected strings from snapshot must be recognized"
        );
        assert_eq!(evidence["active_slide_count"], 1);

        let public_snapshot: Value = serde_json::from_str(&json!({"items":[
            {"reference":"@account","role":"button","name":"teacher@example.test","id":"account-42","size":null},
            {"reference":"@file","role":"row","name":"lesson.pdf","id":"file-9","selected":"true","size":null},
            {"reference":"@class","role":"link","name":"Biology","href":"https://classroom.google.com/c/class-1","id":"class-1","size":null},
            {"reference":"@assignment","role":"link","name":"Lab","href":"https://classroom.google.com/c/class-1/a/assignment-2","id":"assignment-2","size":null},
            {"reference":"@attachment","role":"listitem","name":"lesson.pdf","size":1234,"id":null}
        ]}).to_string()).unwrap();
        let public_items = public_snapshot["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| (item["reference"].as_str().unwrap().to_owned(), item.clone()))
            .collect::<BTreeMap<_, _>>();
        let picker = json!({"version":1,"stage":"selected","picker_url":"https://docs.google.com/picker?authuser=1",
            "account":{"reference":"@account","name":"teacher@example.test","id_field":"id","stable_id":"account-42"},
            "file":{"reference":"@file","name":"lesson.pdf","id_field":"id","stable_id":"file-9","role":"row"}});
        assert!(
            verify_google_drive_picker_v1(
                &picker,
                &public_items,
                true,
                "https://docs.google.com/picker?authuser=1",
            )
            .unwrap()
            .0,
            "DOM ids and ARIA selection must match the public snapshot fields"
        );

        let classroom = json!({"version":1,"stage":"attachment",
            "account":{"reference":"@account","name":"teacher@example.test","id_field":"id","stable_id":"account-42"},
            "class":{"reference":"@class","name":"Biology","id_field":"href","stable_id":"https://classroom.google.com/c/class-1"},
            "assignment":{"reference":"@assignment","name":"Lab","id_field":"href","stable_id":"https://classroom.google.com/c/class-1/a/assignment-2"},
            "attachment":{"reference":"@attachment","filename":"lesson.pdf","size":1234}});
        assert!(
            verify_classroom_attachment_v1(&classroom, &public_items, true)
                .unwrap()
                .0,
            "explicit observed size metadata must match the public snapshot field"
        );
    }

    #[test]
    fn canva_manifest_requires_complete_ordered_exact_fresh_references_and_url() {
        let predicate = slide_predicate();
        let items = slide_items();
        let (passed, evidence) = verify_canva_slide_manifest_v1(
            &predicate,
            &items,
            true,
            "https://www.canva.com/design/abc/edit",
        )
        .unwrap();
        assert!(passed);
        assert_eq!(evidence["evidence_scope"], "caller_visible_ui_only");
        assert!(
            !verify_canva_slide_manifest_v1(
                &predicate,
                &items,
                false,
                "https://www.canva.com/design/abc/edit"
            )
            .unwrap()
            .0
        );
        assert!(
            !verify_canva_slide_manifest_v1(
                &predicate,
                &items,
                true,
                "https://www.canva.com/design/other/edit"
            )
            .unwrap()
            .0
        );
        let mut changed = items;
        changed.get_mut("@c2").unwrap()["href"] =
            json!("https://www.canva.com/design/abc/edit#page=3");
        assert!(
            !verify_canva_slide_manifest_v1(
                &predicate,
                &changed,
                true,
                "https://www.canva.com/design/abc/edit"
            )
            .unwrap()
            .0
        );
        let mut truncated = predicate.clone();
        truncated["expected_count"] = json!(101);
        assert!(
            verify_canva_slide_manifest_v1(
                &truncated,
                &slide_items(),
                true,
                "https://www.canva.com/design/abc/edit"
            )
            .is_err()
        );
        let mut duplicate_id = predicate.clone();
        duplicate_id["slides"][1]["stable_id"] = duplicate_id["slides"][0]["stable_id"].clone();
        assert!(
            verify_canva_slide_manifest_v1(
                &duplicate_id,
                &slide_items(),
                true,
                "https://www.canva.com/design/abc/edit"
            )
            .is_err(),
            "two distinct slide refs cannot share one stable slide identity"
        );
    }

    #[test]
    fn canva_review_contract_expands_to_generic_select_capture_and_verify_steps() {
        let steps = canva_slide_review_steps_v1(&slide_predicate()).unwrap();
        assert_eq!(steps.len(), 11);
        assert_eq!(steps[0]["kind"], "verify");
        assert_eq!(steps[0]["predicate"]["adapter"], "canva_slide_manifest_v1");
        assert_eq!(
            steps[0]["predicate"]["predicate"]["slides"][0]["reference"],
            "@c1"
        );
        assert_eq!(
            steps[0]["predicate"]["predicate"]["slides"][0]["stable_id"],
            "https://www.canva.com/design/abc/edit#page=1"
        );
        assert_eq!(
            steps[0]["predicate"]["predicate"]["slides"][1]["reference"],
            "@c2"
        );
        assert_eq!(
            steps[0]["predicate"]["predicate"]["slides"][1]["stable_id"],
            "https://www.canva.com/design/abc/edit#page=2"
        );
        assert_eq!(
            steps[1],
            json!({"kind":"find","query":"Cover","role":"button","match_mode":"exact","save_as":"canva_slide_ref_1"})
        );
        assert_eq!(
            steps[6]["role"], "tab",
            "planner must find the observed accessible role"
        );
        assert_eq!(
            steps[2],
            json!({"kind":"click","reference":"$canva_slide_ref_1"})
        );
        assert_eq!(steps[3]["predicate"]["adapter"], "canva_active_slide_v1");
        assert_eq!(
            steps[3]["predicate"]["predicate"]["stable_id"],
            "https://www.canva.com/design/abc/edit#page=1"
        );
        assert_eq!(steps[4]["kind"], "probe");
        assert_eq!(steps[4]["save_artifact"], true);
        let evidence_context: Value =
            serde_json::from_str(steps[4]["evidence_context"].as_str().unwrap()).unwrap();
        assert_eq!(evidence_context["kind"], "canva_slide_capture");
        assert_eq!(
            evidence_context["stable_id"],
            "https://www.canva.com/design/abc/edit#page=1"
        );
        assert_eq!(
            evidence_context["identity_scope"],
            "runtime_verified_selection_before_and_after_capture"
        );
        assert_eq!(
            evidence_context["renderer_freshness"],
            "two_animation_frames_before_compositor_capture"
        );
        assert_eq!(
            evidence_context["image_sources"][0],
            "https://cdn.example.test/cover.png"
        );
        assert_eq!(
            evidence_context["source_metadata_scope"],
            "caller_declared_not_independently_verified"
        );
        assert_eq!(steps[4]["omit_image_payload"], false);
        assert_eq!(steps[4]["save_as"], "canva_slide_1");
        assert_eq!(steps[5]["predicate"]["adapter"], "canva_active_slide_v1");
        assert_eq!(steps[6]["kind"], "find");
        assert_eq!(steps[6]["role"], "tab");
        assert_eq!(
            steps[7],
            json!({"kind":"click","reference":"$canva_slide_ref_2"})
        );
        assert_eq!(steps[8]["predicate"]["adapter"], "canva_active_slide_v1");
        assert_eq!(steps[9]["save_as"], "canva_slide_2");
        assert_eq!(steps[10]["predicate"]["adapter"], "canva_active_slide_v1");
        let mut malformed = slide_predicate();
        malformed["slides"][1]["reference"] = json!("@c1");
        assert!(canva_slide_review_steps_v1(&malformed).is_err());
        let mut duplicate_id = slide_predicate();
        duplicate_id["slides"][1]["stable_id"] = duplicate_id["slides"][0]["stable_id"].clone();
        assert!(canva_slide_review_steps_v1(&duplicate_id).is_err());
    }

    #[test]
    fn canva_seven_slide_capture_manifest_keeps_order_and_identity_fences() {
        let mut predicate = slide_predicate();
        let slides = (1..=7)
            .map(|index| {
                json!({
                    "reference":format!("@slide{index}"),
                    "index":index,
                    "name":format!("Slide {index}"),
                    "role":"button",
                    "id_field":"href",
                    "stable_id":format!("https://www.canva.com/design/abc/edit#page={index}")
                })
            })
            .collect::<Vec<_>>();
        predicate["expected_count"] = json!(7);
        predicate["slides"] = json!(slides);

        let steps = canva_slide_review_steps_v1(&predicate).unwrap();
        assert_eq!(
            steps.len(),
            36,
            "one manifest check plus five guarded steps per slide"
        );
        assert_eq!(steps[0]["predicate"]["predicate"]["expected_count"], 7);
        for index in 1..=7 {
            let first_step = 1 + (index - 1) * 5;
            assert_eq!(steps[first_step]["kind"], "find");
            assert_eq!(steps[first_step + 1]["kind"], "click");
            assert_eq!(
                steps[first_step + 2]["predicate"]["adapter"],
                "canva_active_slide_v1"
            );
            assert_eq!(steps[first_step + 3]["kind"], "probe");
            assert_eq!(
                steps[first_step + 3]["save_as"],
                format!("canva_slide_{index}")
            );
            assert_eq!(
                steps[first_step + 4]["predicate"]["adapter"],
                "canva_active_slide_v1"
            );
            assert_eq!(
                steps[first_step + 2]["predicate"]["predicate"]["stable_id"],
                format!("https://www.canva.com/design/abc/edit#page={index}")
            );
        }
    }

    #[test]
    fn canva_draft_edit_save_plan_is_bounded_and_requires_visible_save_evidence() {
        let predicate = json!({
            "version":1,
            "design_url":"https://www.canva.com/design/abc/edit",
            "fields":[{"reference":"@c2","expected_value":"Old title","value":"New title"}],
            "save_control_reference":"@c3",
            "saved_status":{"selector":"[aria-label='Save status']","text":"All changes saved"}
        });
        let steps = canva_draft_edit_save_steps_v1(&predicate).unwrap();
        assert_eq!(steps.len(), 4);
        assert_eq!(
            steps[0],
            json!({"kind":"verify","predicate":{"kind":"url","equals":"https://www.canva.com/design/abc/edit"}})
        );
        assert_eq!(steps[1]["kind"], "fill_form");
        assert_eq!(steps[1]["fields"][0]["expected_value"], "Old title");
        assert_eq!(steps[1]["fields"][0]["value"], "New title");
        assert_eq!(steps[2]["kind"], "click");
        assert_eq!(steps[2]["outcome"]["text"], "All changes saved");
        assert_eq!(steps[3]["predicate"]["kind"], "text_present");
        assert_eq!(steps[3]["predicate"]["scope"], "[aria-label='Save status']");

        let mut missing_status = predicate.clone();
        missing_status
            .as_object_mut()
            .unwrap()
            .remove("saved_status");
        assert!(canva_draft_edit_save_steps_v1(&missing_status).is_err());
        let mut duplicate_fields = predicate;
        duplicate_fields["fields"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "reference":"@c2","expected_value":"Old title","value":"Another title"
            }));
        assert!(canva_draft_edit_save_steps_v1(&duplicate_fields).is_err());
    }

    #[test]
    fn canva_review_can_restore_only_a_manifest_slide_after_final_selection_fence() {
        let mut predicate = slide_predicate();
        predicate["restore_after_review"] = json!({
            "name":"Plan",
            "id_field":"href",
            "stable_id":"https://www.canva.com/design/abc/edit#page=2"
        });
        let steps = canva_slide_review_steps_v1(&predicate).unwrap();
        assert_eq!(steps.len(), 14);
        assert_eq!(steps[10]["predicate"]["adapter"], "canva_active_slide_v1");
        assert_eq!(
            steps[11]["role"], "tab",
            "restore uses the manifest's observed role"
        );
        assert_eq!(
            steps[11],
            json!({"kind":"find","query":"Plan","role":"tab","match_mode":"exact","save_as":"canva_restore_ref_2"})
        );
        assert_eq!(
            steps[12],
            json!({"kind":"click","reference":"$canva_restore_ref_2"})
        );
        assert_eq!(
            steps[13]["predicate"]["predicate"]["stable_id"],
            "https://www.canva.com/design/abc/edit#page=2"
        );

        predicate["restore_after_review"]["stable_id"] =
            json!("https://www.canva.com/design/abc/edit#page=99");
        assert!(
            canva_slide_review_steps_v1(&predicate).is_err(),
            "restore must be grounded in the verified manifest"
        );
    }

    #[test]
    fn google_drive_picker_is_account_and_file_bound_and_never_claims_acceptance() {
        let predicate = json!({"version":1,"picker_url":"https://docs.google.com/picker?authuser=1",
            "account":{"reference":"@account","name":"teacher@example.test","id_field":"id","stable_id":"account-42"},
            "file":{"reference":"@file","name":"lesson.pdf","id_field":"id","stable_id":"file-9","role":"row"},
            "confirm":{"reference":"@select"},
            "destination":{"url":"https://classroom.google.com/c/class-1/a/assignment-2","account":{"reference":"@destination-account","name":"teacher@example.test","id_field":"id","stable_id":"account-42"},
                "attachment":{"reference":"@attachment","name":"lesson.pdf","size":1234,"role":"attachment"}}});
        let steps = google_drive_picker_steps_v1(&predicate).unwrap();
        assert_eq!(steps.len(), 5);
        assert_eq!(steps[0]["predicate"]["adapter"], "google_drive_picker_v1");
        assert_eq!(steps[1]["reference"], "@file");
        assert_eq!(steps[2]["predicate"]["predicate"]["stage"], "selected");
        assert_eq!(steps[3]["reference"], "@select");
        assert_eq!(
            steps[4]["predicate"]["adapter"],
            "google_drive_attachment_v1"
        );

        let items = BTreeMap::from([
            (
                "@account".into(),
                json!({"role":"button","name":"teacher@example.test","id":"account-42"}),
            ),
            (
                "@file".into(),
                json!({"role":"row","name":"lesson.pdf","id":"file-9","selected":"true"}),
            ),
        ]);
        let selected_predicate = json!({"version":1,"stage":"selected","picker_url":predicate["picker_url"],
            "account":predicate["account"],"file":predicate["file"]});
        let (passed, evidence) = verify_google_drive_picker_v1(
            &selected_predicate,
            &items,
            true,
            "https://docs.google.com/picker?authuser=1",
        )
        .unwrap();
        assert!(passed);
        assert_eq!(evidence["destination_acceptance_proven"], false);
        assert_eq!(evidence["persistence_proven"], false);
        assert!(
            !verify_google_drive_picker_v1(
                &selected_predicate,
                &items,
                true,
                "https://evil-google.com/picker",
            )
            .unwrap()
            .0
        );
        let mut wrong_account = items.clone();
        wrong_account.get_mut("@account").unwrap()["id"] = json!("account-other");
        assert!(
            !verify_google_drive_picker_v1(
                &selected_predicate,
                &wrong_account,
                true,
                "https://docs.google.com/picker?authuser=1",
            )
            .unwrap()
            .0
        );
        let mut unsafe_domain = predicate.clone();
        unsafe_domain["picker_url"] = json!("https://evil-google.com/picker");
        assert!(google_drive_picker_steps_v1(&unsafe_domain).is_err());

        let destination_items = BTreeMap::from([
            (
                "@destination-account".into(),
                json!({"role":"button","name":"teacher@example.test","id":"account-42"}),
            ),
            (
                "@attachment".into(),
                json!({"role":"attachment","name":"lesson.pdf","size":1234}),
            ),
        ]);
        let destination_predicate = json!({"version":1,"url":predicate["destination"]["url"],
            "account":predicate["destination"]["account"],"attachment":predicate["destination"]["attachment"]});
        let (accepted, destination_evidence) = verify_google_drive_attachment_v1(
            &destination_predicate,
            &destination_items,
            true,
            "https://classroom.google.com/c/class-1/a/assignment-2",
        )
        .unwrap();
        assert!(accepted);
        assert!(destination_evidence["destination_acceptance_proven"] == true);
        assert!(destination_evidence["server_acceptance_proven"] == false);
        assert!(destination_evidence["persistence_proven"] == false);
        assert!(
            !verify_google_drive_attachment_v1(
                &destination_predicate,
                &destination_items,
                false,
                "https://classroom.google.com/c/class-1/a/assignment-2",
            )
            .unwrap()
            .0,
            "partial destination snapshots cannot prove attachment acceptance"
        );
    }

    #[test]
    fn canva_review_planner_refuses_to_expand_past_workflow_step_budget() {
        let mut predicate = slide_predicate();
        predicate["expected_count"] = json!(52);
        let slides = (1..=52)
            .map(|index| {
                json!({
                    "reference":format!("@slide{index}"), "index":index,
                    "name":format!("Slide {index}"), "id_field":"id",
                    "stable_id":format!("slide-{index}")
                })
            })
            .collect::<Vec<_>>();
        predicate["slides"] = json!(slides);
        assert!(canva_slide_review_steps_v1(&predicate).is_err());
    }

    #[test]
    fn canva_active_slide_requires_one_exact_selected_stable_identity() {
        let predicate = json!({"version":1,"design_url":"https://www.canva.com/design/abc/edit","name":"Cover","id_field":"href","stable_id":"https://www.canva.com/design/abc/edit#page=1"});
        let items = BTreeMap::from([
            (
                "@fresh".into(),
                json!({"role":"button","name":"Cover","href":"https://www.canva.com/design/abc/edit#page=1","selected":"true"}),
            ),
            (
                "@other".into(),
                json!({"role":"button","name":"Plan","href":"https://www.canva.com/design/abc/edit#page=2","selected":"false"}),
            ),
        ]);
        assert!(
            verify_canva_active_slide_v1(
                &predicate,
                &items,
                true,
                "https://www.canva.com/design/abc/edit"
            )
            .unwrap()
            .0
        );
        let mut inactive = items.clone();
        inactive.get_mut("@fresh").unwrap()["selected"] = json!("false");
        assert!(
            !verify_canva_active_slide_v1(
                &predicate,
                &inactive,
                true,
                "https://www.canva.com/design/abc/edit"
            )
            .unwrap()
            .0
        );
        assert!(
            !verify_canva_active_slide_v1(
                &predicate,
                &items,
                false,
                "https://www.canva.com/design/abc/edit"
            )
            .unwrap()
            .0
        );
        assert!(
            !verify_canva_active_slide_v1(
                &predicate,
                &items,
                true,
                "https://www.canva.com/design/other/edit"
            )
            .unwrap()
            .0
        );
        let mut duplicate = items;
        duplicate.insert("@duplicate".into(), duplicate["@fresh"].clone());
        assert!(
            !verify_canva_active_slide_v1(
                &predicate,
                &duplicate,
                true,
                "https://www.canva.com/design/abc/edit"
            )
            .unwrap()
            .0
        );
    }

    #[test]
    fn classroom_attachment_evidence_requires_exact_visible_identity_and_never_claims_persistence()
    {
        let predicate = json!({"version":1,
            "account":{"reference":"@c1","name":"teacher@example.test","id_field":"href","stable_id":"https://classroom.google.com/u/0/"},
            "class":{"reference":"@c2","name":"Testing class","id_field":"href","stable_id":"https://classroom.google.com/c/123"},
            "assignment":{"reference":"@c3","name":"Draft post","id_field":"href","stable_id":"https://classroom.google.com/c/123/a/456"},
            "attachment":{"reference":"@c4","filename":"handout.pdf","size":42}});
        let items: BTreeMap<String, Value> = [
            ("@c1".into(), json!({"role":"button","name":"teacher@example.test","href":"https://classroom.google.com/u/0/"})),
            ("@c2".into(), json!({"role":"link","name":"Testing class","href":"https://classroom.google.com/c/123"})),
            ("@c3".into(), json!({"role":"link","name":"Draft post","href":"https://classroom.google.com/c/123/a/456"})),
            ("@c4".into(), json!({"role":"attachment","name":"handout.pdf","filename":"handout.pdf","size":42})),
        ].into_iter().collect();
        let (passed, evidence) = verify_classroom_attachment_v1(&predicate, &items, true).unwrap();
        assert!(passed);
        assert_eq!(evidence["evidence_scope"], "caller_visible_ui_only");
        assert_eq!(evidence["server_acceptance_proven"], false);
        assert_eq!(evidence["persistence_proven"], false);
        let destination_predicate = json!({"version":1,"stage":"destination","account":predicate["account"],"class":predicate["class"],"assignment":predicate["assignment"]});
        let (destination_passed, destination_evidence) =
            verify_classroom_attachment_v1(&destination_predicate, &items, true).unwrap();
        assert!(destination_passed);
        assert_eq!(destination_evidence["stage"], "destination");
        assert_eq!(
            destination_evidence["evidence_scope"],
            "caller_visible_ui_only"
        );
        assert_eq!(destination_evidence["server_acceptance_proven"], false);
        assert!(
            !verify_classroom_attachment_v1(&predicate, &items, false)
                .unwrap()
                .0
        );
        let mut mismatch = items;
        mismatch.get_mut("@c4").unwrap()["size"] = json!(43);
        assert!(
            !verify_classroom_attachment_v1(&predicate, &mismatch, true)
                .unwrap()
                .0
        );
    }

    #[test]
    fn classroom_attachment_contract_emits_only_guarded_generic_upload() {
        let predicate = json!({"version":1,
            "account":{"reference":"@c1","name":"teacher@example.test","id_field":"href","stable_id":"https://classroom.google.com/u/0/"},
            "class":{"reference":"@c2","name":"Testing class","id_field":"href","stable_id":"https://classroom.google.com/c/123"},
            "assignment":{"reference":"@c3","name":"Draft post","id_field":"href","stable_id":"https://classroom.google.com/c/123/a/456"},
            "attachment":{"reference":"@c4","filename":"handout.pdf","size":42}});
        let steps = classroom_attachment_step_v1(
            "artifact:abc",
            "@c5",
            "#account",
            "teacher@example.test",
            &predicate,
        )
        .unwrap();
        assert_eq!(steps.as_array().unwrap().len(), 2);
        assert_eq!(steps[0]["kind"], "verify");
        assert_eq!(steps[0]["predicate"]["predicate"]["stage"], "destination");
        assert_eq!(steps[1]["kind"], "upload");
        assert_eq!(steps[1]["artifact_handle"], "artifact:abc");
        assert_eq!(steps[1]["trigger_reference"], "@c5");
        assert_eq!(
            steps[1]["account_marker"],
            json!(["#account", "teacher@example.test"])
        );
        assert_eq!(
            steps[1]["acceptance"]["adapter"],
            "google_classroom_attachment_v1"
        );
        assert!(
            steps
                .as_array()
                .unwrap()
                .iter()
                .all(|step| step.get("publish").is_none())
        );
        assert!(
            classroom_attachment_step_v1("artifact:abc", "@c5", "#account", "teacher", &predicate)
                .is_ok()
        );
        assert!(
            classroom_attachment_step_v1(
                "artifact:abc",
                "@c5",
                "#account",
                "teacher",
                &json!({"version":1})
            )
            .is_err()
        );
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum App {
    GoogleSlides,
    Canva,
    CapCutWeb,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, rmcp::schemars::JsonSchema)]
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
