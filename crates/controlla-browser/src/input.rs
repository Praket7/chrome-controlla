//! Guarded browser input primitives. These types do not qualify OS focus, cursor, or clipboard isolation.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::time::Instant;

pub type SemanticLocator = super::Locator;
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum InputAction {
    Fill(String),
    Insert(String),
    /// Insert per-session plain text through CDP; never reads the OS clipboard.
    PasteInternalClipboard,
    /// Dispatch and commit a CDP composition sequence on a supported text control.
    ImeText(String),
    SequentialKeys(String),
    Click {
        x: f64,
        y: f64,
        postcondition: ClickPostcondition,
    },
    Drag {
        from: (f64, f64),
        to: (f64, f64),
        postcondition: DragPostcondition,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClickPostcondition {
    ActiveElement,
    Value(String),
    Attribute { name: String, value: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DragPostcondition {
    pub left: f64,
    pub top: f64,
    pub tolerance: f64,
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
pub enum InputOutcome {
    Applied {
        observed_value: Option<String>,
        postcondition_verified: bool,
        elapsed_micros: u128,
    },
    Unsupported(&'static str),
    Stale(&'static str),
    NeedsForeground,
}
pub struct GuardedInput<'a> {
    pub expected: &'a GuardSnapshot,
    pub current: &'a GuardSnapshot,
    pub locator: &'a SemanticLocator,
    pub action: &'a InputAction,
    pub expected_value: &'a str,
}

fn resolve_element_script(locator: &SemanticLocator) -> Result<String, super::BrowserError> {
    let v = serde_json::to_string(locator)
        .map_err(|e| super::BrowserError::InvalidResponse(e.to_string()))?;
    Ok(format!(
        r#"(()=>{{const l={v};let es=[];if(l.Css)es=[...document.querySelectorAll(l.Css)];else if(l.TestId)es=[...document.querySelectorAll('[data-testid]')].filter(e=>e.getAttribute('data-testid')===l.TestId);else if(l.Placeholder)es=[...document.querySelectorAll('[placeholder]')].filter(e=>e.getAttribute('placeholder')===l.Placeholder);else if(l.Label)es=[...document.querySelectorAll('label')].filter(e=>e.innerText.trim()===l.Label).map(e=>e.control).filter(Boolean);else if(l.Text)es=[...document.querySelectorAll('button,a,[role],label,[data-testid]')].filter(e=>e.innerText.trim()===l.Text);else if(l.AltText)es=[...document.querySelectorAll('[alt]')].filter(e=>e.getAttribute('alt')===l.AltText);else if(l.Href)es=[...document.querySelectorAll('a[href]')].filter(e=>e.getAttribute('href').includes(l.Href));else if(l.RoleName)es=[...document.querySelectorAll('[role],button,input,textarea,a')].filter(e=>(e.getAttribute('role')||({{BUTTON:'button',INPUT:'textbox',TEXTAREA:'textbox',A:'link'}}[e.tagName])||'')===l.RoleName.role&&(e.getAttribute('aria-label')||e.innerText||e.value||'').trim()===l.RoleName.name);if(es.length!==1)return {{ok:false,reason:es.length?'ambiguous':'no_match',count:es.length}};return {{ok:true,e:es[0]}};}})()"#
    ))
}

fn locator_script(locator: &SemanticLocator) -> Result<String, super::BrowserError> {
    let resolve = resolve_element_script(locator)?;
    Ok(format!(
        r#"(()=>{{const r={resolve};if(!r.ok)return r;const e=r.e,b=e.getBoundingClientRect(),s=getComputedStyle(e),x=b.left+b.width/2,y=b.top+b.height/2,h=document.elementFromPoint(x,y);return {{ok:true,count:1,tag:e.tagName,type:e.type||'',editable:e.isContentEditable,masked:e.hasAttribute('data-masked'),requiresTrustedEvents:e.hasAttribute('data-requires-trusted'),disabled:!!e.disabled,visible:b.width>0&&b.height>0&&s.visibility!=='hidden'&&s.display!=='none',hit:!!h&&(h===e||e.contains(h)),value:e.type==='password'?'':e.value??'',selectionStart:e.selectionStart,selectionEnd:e.selectionEnd}};}})()"#
    ))
}

fn click_postcondition_script(
    locator: &SemanticLocator,
    condition: &ClickPostcondition,
) -> Result<String, super::BrowserError> {
    let resolve = resolve_element_script(locator)?;
    let predicate = match condition {
        ClickPostcondition::ActiveElement => "document.activeElement===e".to_owned(),
        ClickPostcondition::Value(value) => format!(
            "(e.value??e.innerText??'')==={}",
            serde_json::to_string(value)
                .map_err(|x| super::BrowserError::InvalidResponse(x.to_string()))?
        ),
        ClickPostcondition::Attribute { name, value } => format!(
            "e.getAttribute({})==={}",
            serde_json::to_string(name)
                .map_err(|x| super::BrowserError::InvalidResponse(x.to_string()))?,
            serde_json::to_string(value)
                .map_err(|x| super::BrowserError::InvalidResponse(x.to_string()))?
        ),
    };
    Ok(format!(
        "(()=>{{const r={resolve};if(!r.ok)return false;const e=r.e;return {predicate};}})()"
    ))
}

fn drag_postcondition_script(
    locator: &SemanticLocator,
    condition: DragPostcondition,
) -> Result<String, super::BrowserError> {
    let resolve = resolve_element_script(locator)?;
    if !condition.left.is_finite()
        || !condition.top.is_finite()
        || !condition.tolerance.is_finite()
        || condition.tolerance < 0.0
    {
        return Err(super::BrowserError::InvalidResponse(
            "drag postcondition bounds must be finite with non-negative tolerance".into(),
        ));
    }
    Ok(format!(
        r#"(()=>{{const r={resolve};if(!r.ok)return false;const b=r.e.getBoundingClientRect();return Math.abs(b.left-{})<={}&&Math.abs(b.top-{})<={};}})()"#,
        condition.left, condition.tolerance, condition.top, condition.tolerance
    ))
}

fn validate_match_result(value: &serde_json::Value) -> Result<(), super::BrowserError> {
    if value["ok"] == true && value["count"] == 1 {
        Ok(())
    } else if value["reason"] == "ambiguous" || value["count"].as_u64().is_some_and(|n| n > 1) {
        Err(super::BrowserError::StaleReference(
            "ambiguous semantic locator".into(),
        ))
    } else {
        Err(super::BrowserError::StaleReference(
            "semantic locator has no match".into(),
        ))
    }
}

fn replacement_value(before: &str, start: u64, end: u64, inserted: &str) -> Option<String> {
    let units: Vec<u16> = before.encode_utf16().collect();
    let (start, end) = (usize::try_from(start).ok()?, usize::try_from(end).ok()?);
    if start > end || end > units.len() {
        return None;
    }
    let prefix = String::from_utf16(&units[..start]).ok()?;
    let suffix = String::from_utf16(&units[end..]).ok()?;
    Some(format!("{prefix}{inserted}{suffix}"))
}

fn sequential_caret(value: &str, start: u64, end: u64) -> Option<u64> {
    let end_of_value = value.encode_utf16().count() as u64;
    (start == end && start == end_of_value).then_some(start)
}

impl super::BrowserConnection {
    /// Resolve one locator and dispatch supported text input through the guarded CDP target route.
    /// Page scripts can still mutate state between any validation and its remote effect.
    pub async fn perform_guarded_input(
        &self,
        sessions: &super::sessions::SessionRegistry,
        reference: &super::sessions::TargetRef,
        principal: &str,
        revisions: super::sessions::IdentityRevisions,
        input: GuardedInput<'_>,
    ) -> Result<InputOutcome, super::BrowserError> {
        let GuardedInput {
            expected,
            current,
            locator,
            action,
            expected_value,
        } = input;
        if expected.strict_background && current.requires_native {
            return Ok(InputOutcome::NeedsForeground);
        }
        if expected.strict_background
            && matches!(action, InputAction::Click { .. } | InputAction::Drag { .. })
        {
            return Ok(InputOutcome::NeedsForeground);
        }
        if validate_step(expected, current) != GuardDecision::Allow {
            return Err(super::BrowserError::StaleReference(
                "input guard changed".into(),
            ));
        }
        locator.validate()?;
        if matches!(locator, SemanticLocator::BackendNodeId(_)) {
            return Ok(InputOutcome::Unsupported(
                "backend node locators need an independently validated DOM node route",
            ));
        }
        let start = Instant::now();
        let probe = self.target_ref_command(sessions, reference, principal, revisions, "Runtime.evaluate", serde_json::json!({"expression":locator_script(locator)?,"returnByValue":true,"awaitPromise":false})).await?;
        let p = &probe["result"]["value"];
        validate_match_result(p)?;
        // Password values are never part of the guarded-input observation contract.
        if p["tag"] == "INPUT" && p["type"] == "password" {
            return Ok(InputOutcome::Unsupported(
                "password controls are not supported by guarded input",
            ));
        }
        if p["value"].as_str() != Some(expected_value) {
            return Err(super::BrowserError::StaleReference(
                "field value changed since the guarded plan".into(),
            ));
        }
        if p["visible"] != true || p["disabled"] == true {
            return Err(super::BrowserError::StaleReference(
                "input target is not actionable".into(),
            ));
        }
        let clipboard_text = if matches!(action, InputAction::PasteInternalClipboard) {
            match sessions.internal_clipboard_text_for(&reference.session_id, principal) {
                Ok(Some(text)) => Some(text.to_owned()),
                Ok(None) => {
                    return Ok(InputOutcome::Unsupported(
                        "this session has no internal clipboard text",
                    ));
                }
                Err(_) => {
                    return Err(super::BrowserError::StaleReference(
                        "internal clipboard session or principal is no longer authorized".into(),
                    ));
                }
            }
        } else {
            None
        };
        let value = match action {
            InputAction::Fill(v)
            | InputAction::Insert(v)
            | InputAction::ImeText(v)
            | InputAction::SequentialKeys(v) => Some(v.as_str()),
            InputAction::PasteInternalClipboard => clipboard_text.as_deref(),
            InputAction::Click { .. } | InputAction::Drag { .. } => None,
        };
        let expected_value = if let InputAction::Fill(text) = action {
            Some(text.clone())
        } else if let Some(text) = value {
            let (Some(before), Some(start), Some(end)) = (
                p["value"].as_str(),
                p["selectionStart"].as_u64(),
                p["selectionEnd"].as_u64(),
            ) else {
                return Ok(InputOutcome::Unsupported(
                    "input selection range is unavailable",
                ));
            };
            let Some(result) = replacement_value(before, start, end, text) else {
                return Ok(InputOutcome::Unsupported(
                    "insertion splits an unsupported UTF-16 selection boundary",
                ));
            };
            Some(result)
        } else {
            None
        };
        let insertion_text = match action {
            InputAction::Insert(text) => Some(text.as_str()),
            InputAction::PasteInternalClipboard => clipboard_text.as_deref(),
            _ => None,
        };
        let tag = p["tag"].as_str().unwrap_or_default();
        let typ = p["type"].as_str().unwrap_or_default();
        let contenteditable = p["editable"] == true;
        if tag == "CANVAS" && matches!(action, InputAction::Drag { .. }) {
            return Ok(InputOutcome::Unsupported(
                "canvas drag outcomes require an app-specific verifier",
            ));
        }
        if value.is_some() && p["masked"] == true {
            return Ok(InputOutcome::Unsupported(
                "masked controls require an app-specific input adapter",
            ));
        }
        if value.is_some() && p["requiresTrustedEvents"] == true {
            return Ok(InputOutcome::Unsupported(
                "event-dependent controls require an app-specific input adapter",
            ));
        }
        if value.is_some()
            && (contenteditable
                || !matches!(
                    (tag, typ),
                    ("TEXTAREA", _) | ("INPUT", "text" | "search" | "email" | "url" | "tel")
                ))
        {
            return Ok(InputOutcome::Unsupported(
                "only ordinary text inputs and textareas are qualified",
            ));
        }
        if matches!(action, InputAction::SequentialKeys(s) if !s.chars().all(|c| c.is_ascii_graphic() || c == ' '))
        {
            return Ok(InputOutcome::Unsupported(
                "sequential key events support ASCII only; use fill or insert for Unicode text",
            ));
        }
        if matches!(action, InputAction::SequentialKeys(_)) {
            let Some(before) = p["value"].as_str() else {
                return Ok(InputOutcome::Unsupported("input value is unavailable"));
            };
            if sequential_caret(
                before,
                p["selectionStart"].as_u64().unwrap_or(u64::MAX),
                p["selectionEnd"].as_u64().unwrap_or(u64::MAX),
            )
            .is_none()
            {
                return Ok(InputOutcome::Stale(
                    "sequential typing requires a collapsed caret at the end of the field",
                ));
            }
        }
        if matches!(action, InputAction::ImeText(text) if text.is_empty()) {
            return Ok(InputOutcome::Unsupported("IME text must not be empty"));
        }
        if matches!(action, InputAction::ImeText(_)) {
            let Some(before) = p["value"].as_str() else {
                return Ok(InputOutcome::Unsupported("input value is unavailable"));
            };
            if sequential_caret(
                before,
                p["selectionStart"].as_u64().unwrap_or(u64::MAX),
                p["selectionEnd"].as_u64().unwrap_or(u64::MAX),
            )
            .is_none()
            {
                return Ok(InputOutcome::Stale(
                    "IME text requires a collapsed caret at the end of the field",
                ));
            }
        }
        let resolve = resolve_element_script(locator)?;
        let focused = if value.is_some() {
            self.target_ref_command(sessions,reference,principal,revisions,"Runtime.evaluate",serde_json::json!({"expression":format!("(()=>{{const r={resolve};if(!r.ok)return false;r.e.focus();return document.activeElement===r.e;}})()"),"returnByValue":true})).await?
        } else {
            serde_json::json!({"result":{"value":true}})
        };
        if focused["result"]["value"] != true {
            return Err(super::BrowserError::StaleReference(
                "target became ambiguous or could not be focused in page".into(),
            ));
        }
        let valid_point = |x: f64, y: f64| x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0;
        if let Some((x, y)) = match action {
            InputAction::Click { x, y, .. } => Some((*x, *y)),
            InputAction::Drag { from, .. } => Some(*from),
            _ => None,
        } {
            if !valid_point(x, y) {
                return Err(super::BrowserError::InvalidResponse(
                    "mouse coordinates must be finite and non-negative".into(),
                ));
            }
            let check = format!(
                "(()=>{{const r={resolve};if(!r.ok||{x}>=innerWidth||{y}>=innerHeight)return false;const e=r.e,b=e.getBoundingClientRect(),h=document.elementFromPoint({x},{y});return b.width>0&&b.height>0&&{x}>=b.left&&{x}<=b.right&&{y}>=b.top&&{y}<=b.bottom&&!!h&&(h===e||e.contains(h));}})()"
            );
            let valid = self
                .target_ref_command(
                    sessions,
                    reference,
                    principal,
                    revisions,
                    "Runtime.evaluate",
                    serde_json::json!({"expression":check,"returnByValue":true}),
                )
                .await?;
            if valid["result"]["value"] != true {
                return Err(super::BrowserError::StaleReference(
                    "click/drag geometry or hit target changed".into(),
                ));
            }
        }
        if let InputAction::Drag { to: (x, y), .. } = action {
            if !valid_point(*x, *y) {
                return Err(super::BrowserError::InvalidResponse(
                    "mouse coordinates must be finite and non-negative".into(),
                ));
            }
            // A drag destination can be outside the source element's original bounds.
            let check = format!(
                "(()=>{{const r={resolve};return r.ok&&{x}<innerWidth&&{y}<innerHeight&&!!document.elementFromPoint({x},{y});}})()"
            );
            let valid = self
                .target_ref_command(
                    sessions,
                    reference,
                    principal,
                    revisions,
                    "Runtime.evaluate",
                    serde_json::json!({"expression":check,"returnByValue":true}),
                )
                .await?;
            if valid["result"]["value"] != true {
                return Err(super::BrowserError::StaleReference(
                    "drag destination geometry or hit target changed".into(),
                ));
            }
        }
        // Give already-arrived lifecycle events a scheduling turn before the
        // next target_ref_command performs its final revision check.
        tokio::task::yield_now().await;
        match action {
            InputAction::Fill(text) => {
                let text = serde_json::to_string(text)
                    .map_err(|e| super::BrowserError::InvalidResponse(e.to_string()))?;
                let expected = serde_json::to_string(input.expected_value)
                    .map_err(|e| super::BrowserError::InvalidResponse(e.to_string()))?;
                let expression = format!(
                    r#"(() => {{const r={resolve};if(!r.ok)return {{stale:false}};const e=r.e;if(e.value!=={expected})return {{stale:true}};const d=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value');d?.set?.call(e,{text});e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return {{stale:false,applied:true}};}})()"#
                );
                let sent = self
                    .target_ref_command(
                        sessions,
                        reference,
                        principal,
                        revisions,
                        "Runtime.evaluate",
                        serde_json::json!({"expression":expression,"returnByValue":true}),
                    )
                    .await?;
                if sent["result"]["value"]["stale"] == true {
                    return Ok(InputOutcome::Stale(
                        "field value changed immediately before fill",
                    ));
                }
                if sent["result"]["value"]["applied"] != true {
                    return Ok(InputOutcome::Stale("fill target could not be resolved"));
                }
            }
            InputAction::Insert(_) | InputAction::PasteInternalClipboard => {
                let Some(text) = insertion_text else {
                    return Ok(InputOutcome::Unsupported(
                        "this session has no internal clipboard text",
                    ));
                };
                if !self
                    .text_guard_matches(
                        sessions,
                        reference,
                        principal,
                        revisions,
                        &resolve,
                        (input.expected_value, None),
                    )
                    .await?
                {
                    return Ok(InputOutcome::Stale(
                        "field value or focus changed immediately before insert",
                    ));
                }
                self.target_ref_command(
                    sessions,
                    reference,
                    principal,
                    revisions,
                    "Input.insertText",
                    serde_json::json!({"text":text}),
                )
                .await?;
            }
            InputAction::ImeText(text) => {
                let original = input.expected_value;
                let original_caret = original.encode_utf16().count() as u64;
                if !self
                    .text_guard_matches(
                        sessions,
                        reference,
                        principal,
                        revisions,
                        &resolve,
                        (original, Some(original_caret)),
                    )
                    .await?
                {
                    return Ok(InputOutcome::Stale(
                        "field value, focus, or caret changed immediately before IME composition",
                    ));
                }
                let composition_len = text.encode_utf16().count() as u64;
                if let Err(error) = self
                    .target_ref_command(
                        sessions,
                        reference,
                        principal,
                        revisions,
                        "Input.imeSetComposition",
                        serde_json::json!({"text":text,"selectionStart":composition_len,"selectionEnd":composition_len}),
                    )
                    .await
                {
                    return match self
                        .cancel_ime_composition(sessions, reference, principal, revisions)
                        .await
                    {
                        Ok(()) => Err(error),
                        Err(cleanup_error) => Err(super::BrowserError::InvalidResponse(format!(
                            "IME composition failed: {error}; cancellation failed: {cleanup_error}"
                        ))),
                    };
                }
                let Some(composed_value) = expected_value.as_deref() else {
                    return Ok(InputOutcome::Unsupported(
                        "IME composition requires a readable expected value",
                    ));
                };
                let composed_caret = composed_value.encode_utf16().count() as u64;
                let composed = self
                    .text_guard_matches(
                        sessions,
                        reference,
                        principal,
                        revisions,
                        &resolve,
                        (composed_value, Some(composed_caret)),
                    )
                    .await;
                if !matches!(composed, Ok(true)) {
                    self.cancel_ime_composition(sessions, reference, principal, revisions)
                        .await?;
                    return match composed {
                        Ok(false) => Ok(InputOutcome::Stale(
                            "field value or focus changed during IME composition",
                        )),
                        Err(error) => Err(error),
                        Ok(true) => unreachable!(),
                    };
                }
                if let Err(error) = self
                    .target_ref_command(
                        sessions,
                        reference,
                        principal,
                        revisions,
                        "Input.insertText",
                        serde_json::json!({"text":text}),
                    )
                    .await
                {
                    return match self
                        .cancel_ime_composition(sessions, reference, principal, revisions)
                        .await
                    {
                        Ok(()) => Err(error),
                        Err(cleanup_error) => Err(super::BrowserError::InvalidResponse(format!(
                            "IME commit failed: {error}; cancellation failed: {cleanup_error}"
                        ))),
                    };
                }
            }
            InputAction::SequentialKeys(text) => {
                let mut live_value = input.expected_value.to_owned();
                let mut caret = live_value.encode_utf16().count() as u64;
                for c in text.chars() {
                    if !self
                        .text_guard_matches(
                            sessions,
                            reference,
                            principal,
                            revisions,
                            &resolve,
                            (&live_value, Some(caret)),
                        )
                        .await?
                    {
                        return Ok(InputOutcome::Stale(
                            "field value or focus changed immediately before key event",
                        ));
                    }
                    let key_up = serde_json::json!({"type":"keyUp","key":c.to_string()});
                    // Cleanup releases this keyDown only; it sends no text and stays subject to target authority.
                    if let Err(error) = self
                        .target_ref_command(
                            sessions,
                            reference,
                            principal,
                            revisions,
                            "Input.dispatchKeyEvent",
                            serde_json::json!({"type":"keyDown","key":c.to_string()}),
                        )
                        .await
                    {
                        let _ = self
                            .target_ref_command(
                                sessions,
                                reference,
                                principal,
                                revisions,
                                "Input.dispatchKeyEvent",
                                key_up.clone(),
                            )
                            .await;
                        return Err(error);
                    }
                    let after_key_down = self
                        .text_guard_matches(
                            sessions,
                            reference,
                            principal,
                            revisions,
                            &resolve,
                            (&live_value, Some(caret)),
                        )
                        .await;
                    if !matches!(after_key_down, Ok(true)) {
                        let _ = self
                            .target_ref_command(
                                sessions,
                                reference,
                                principal,
                                revisions,
                                "Input.dispatchKeyEvent",
                                key_up.clone(),
                            )
                            .await;
                        if !after_key_down? {
                            return Ok(InputOutcome::Stale(
                                "field value, focus, or caret changed before character event",
                            ));
                        }
                    }
                    if let Err(error) = self.target_ref_command(sessions,reference,principal,revisions,"Input.dispatchKeyEvent",serde_json::json!({"type":"char","text":c.to_string(),"unmodifiedText":c.to_string()})).await {
                        let _ = self.target_ref_command(sessions, reference, principal, revisions, "Input.dispatchKeyEvent", key_up.clone()).await;
                        return Err(error);
                    }
                    live_value.push(c);
                    caret += c.len_utf16() as u64;
                    let before_key_up = self
                        .text_guard_matches(
                            sessions,
                            reference,
                            principal,
                            revisions,
                            &resolve,
                            (&live_value, Some(caret)),
                        )
                        .await;
                    self.target_ref_command(
                        sessions,
                        reference,
                        principal,
                        revisions,
                        "Input.dispatchKeyEvent",
                        key_up,
                    )
                    .await?;
                    if !before_key_up? {
                        return Ok(InputOutcome::Stale(
                            "field value, focus, or caret changed before key release",
                        ));
                    }
                }
            }
            InputAction::Click {
                x,
                y,
                postcondition,
            } => {
                self.target_ref_command(sessions,reference,principal,revisions,"Input.dispatchMouseEvent",serde_json::json!({"type":"mousePressed","x":x,"y":y,"button":"left","clickCount":1})).await?;
                self.target_ref_command(sessions,reference,principal,revisions,"Input.dispatchMouseEvent",serde_json::json!({"type":"mouseReleased","x":x,"y":y,"button":"left","clickCount":1})).await?;
                let expression = click_postcondition_script(locator, postcondition)?;
                let result = self
                    .target_ref_command(
                        sessions,
                        reference,
                        principal,
                        revisions,
                        "Runtime.evaluate",
                        serde_json::json!({"expression":expression,"returnByValue":true}),
                    )
                    .await?;
                if result["result"]["value"] != true {
                    return Err(super::BrowserError::InvalidResponse(
                        "click postcondition failed".into(),
                    ));
                }
                return Ok(InputOutcome::Applied {
                    observed_value: None,
                    postcondition_verified: true,
                    elapsed_micros: start.elapsed().as_micros(),
                });
            }
            InputAction::Drag {
                from,
                to,
                postcondition,
            } => {
                self.target_ref_command(sessions,reference,principal,revisions,"Input.dispatchMouseEvent",serde_json::json!({"type":"mousePressed","x":from.0,"y":from.1,"button":"left","buttons":1,"clickCount":1})).await?;
                self.target_ref_command(sessions,reference,principal,revisions,"Input.dispatchMouseEvent",serde_json::json!({"type":"mouseMoved","x":to.0,"y":to.1,"button":"left","buttons":1})).await?;
                self.target_ref_command(sessions,reference,principal,revisions,"Input.dispatchMouseEvent",serde_json::json!({"type":"mouseReleased","x":to.0,"y":to.1,"button":"left","buttons":0,"clickCount":1})).await?;
                let expression = drag_postcondition_script(locator, *postcondition)?;
                let result = self
                    .target_ref_command(
                        sessions,
                        reference,
                        principal,
                        revisions,
                        "Runtime.evaluate",
                        serde_json::json!({"expression":expression,"returnByValue":true}),
                    )
                    .await?;
                if result["result"]["value"] != true {
                    return Err(super::BrowserError::InvalidResponse(
                        "drag position postcondition failed".into(),
                    ));
                }
                return Ok(InputOutcome::Applied {
                    observed_value: None,
                    postcondition_verified: true,
                    elapsed_micros: start.elapsed().as_micros(),
                });
            }
        }
        let readback = self
            .target_ref_command(
                sessions,
                reference,
                principal,
                revisions,
                "Runtime.evaluate",
                serde_json::json!({"expression":locator_script(locator)?,"returnByValue":true}),
            )
            .await?;
        let observed = readback["result"]["value"]["value"]
            .as_str()
            .map(str::to_owned);
        if observed != expected_value {
            return Err(super::BrowserError::InvalidResponse(
                "input postcondition did not match requested value".into(),
            ));
        }
        if matches!(action, InputAction::ImeText(_)) {
            let caret = observed
                .as_deref()
                .unwrap_or_default()
                .encode_utf16()
                .count() as u64;
            if readback["result"]["value"]["selectionStart"].as_u64() != Some(caret)
                || readback["result"]["value"]["selectionEnd"].as_u64() != Some(caret)
            {
                return Err(super::BrowserError::InvalidResponse(
                    "IME text caret postcondition did not match committed value".into(),
                ));
            }
        }
        Ok(InputOutcome::Applied {
            observed_value: observed,
            postcondition_verified: true,
            elapsed_micros: start.elapsed().as_micros(),
        })
    }

    async fn text_guard_matches(
        &self,
        sessions: &super::sessions::SessionRegistry,
        reference: &super::sessions::TargetRef,
        principal: &str,
        revisions: super::sessions::IdentityRevisions,
        resolve: &str,
        expected: (&str, Option<u64>),
    ) -> Result<bool, super::BrowserError> {
        let (expected_value, expected_caret) = expected;
        let expected = serde_json::to_string(expected_value)
            .map_err(|e| super::BrowserError::InvalidResponse(e.to_string()))?;
        let caret = expected_caret.map_or_else(String::new, |n| {
            format!("&&r.e.selectionStart==={n}&&r.e.selectionEnd==={n}")
        });
        let expression = format!(
            "(()=>{{const r={resolve};return r.ok&&r.e.value==={expected}&&document.activeElement===r.e{caret};}})()"
        );
        let result = self
            .target_ref_command(
                sessions,
                reference,
                principal,
                revisions,
                "Runtime.evaluate",
                serde_json::json!({"expression":expression,"returnByValue":true}),
            )
            .await?;
        Ok(result["result"]["value"] == true)
    }

    async fn cancel_ime_composition(
        &self,
        sessions: &super::sessions::SessionRegistry,
        reference: &super::sessions::TargetRef,
        principal: &str,
        revisions: super::sessions::IdentityRevisions,
    ) -> Result<(), super::BrowserError> {
        self.target_ref_command(
            sessions,
            reference,
            principal,
            revisions,
            "Input.imeSetComposition",
            serde_json::json!({"text":"","selectionStart":0,"selectionEnd":0}),
        )
        .await?;
        Ok(())
    }
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
    fn sequential_keys_require_collapsed_end_caret() {
        assert_eq!(sequential_caret("hello", 5, 5), Some(5));
        assert_eq!(sequential_caret("hello", 2, 2), None);
        assert_eq!(sequential_caret("hello", 2, 4), None);
        assert_eq!(sequential_caret("hé🙂", 4, 4), Some(4));
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
        c.document += 1;
        assert_eq!(
            validate_step(&e, &c),
            GuardDecision::Yield("identity_changed")
        );
        c = e.clone();
        c.navigation += 1;
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
        assert_eq!(validate_step(&e, &c), GuardDecision::NeedsForeground);
    }
    #[test]
    fn external_change_invalidates() {
        assert_eq!(
            on_external_change(Some("overlay")),
            InvalidationSet(["overlay".into()].into_iter().collect())
        );
    }
    #[test]
    fn semantic_matches_fail_closed_on_zero_or_multiple_results() {
        assert!(validate_match_result(&serde_json::json!({"ok":true,"count":1})).is_ok());
        assert!(matches!(
            validate_match_result(&serde_json::json!({"ok":false,"reason":"no_match","count":0})),
            Err(super::super::BrowserError::StaleReference(_))
        ));
        assert!(
            matches!(validate_match_result(&serde_json::json!({"ok":false,"reason":"ambiguous","count":2})),Err(super::super::BrowserError::StaleReference(reason)) if reason.contains("ambiguous"))
        );
    }
    #[test]
    fn insertion_postcondition_uses_utf16_selection_indices() {
        assert_eq!(
            replacement_value("A👋B", 3, 3, "λ").as_deref(),
            Some("A👋λB")
        );
        assert_eq!(replacement_value("A👋B", 2, 2, "x"), None);
    }
}
