//! Runtime-owned registry for site capabilities qualified against live structure.
//!
//! WebMCP descriptors are inputs only; their names and schemas never supply effect or qualification.

use crate::workflow::EffectClass;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const MAX_CAPABILITIES: usize = 512;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapabilityImplementation {
    WebMcp { tool_name: String },
    QualifiedSkill { skill_id: String },
    SiteProfile { profile_id: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityRouteKind {
    WebMcp,
    QualifiedSkill,
    SiteProfile,
}

impl CapabilityImplementation {
    pub fn route_kind(&self) -> CapabilityRouteKind {
        match self {
            Self::WebMcp { .. } => CapabilityRouteKind::WebMcp,
            Self::QualifiedSkill { .. } => CapabilityRouteKind::QualifiedSkill,
            Self::SiteProfile { .. } => CapabilityRouteKind::SiteProfile,
        }
    }
}

impl CapabilityImplementation {
    fn rank(&self) -> u8 {
        match self {
            Self::WebMcp { .. } => 0,
            Self::QualifiedSkill { .. } => 1,
            Self::SiteProfile { .. } => 2,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SiteCapability {
    pub name: String,
    pub site_scope: String,
    pub structural_signature: String,
    pub input_schema: Value,
    pub implementation: CapabilityImplementation,
    pub effect: EffectClass,
    pub independently_verified: bool,
    pub version: u32,
}

#[derive(Clone, Debug, Default)]
pub struct CapabilityRegistry {
    capabilities: BTreeMap<(String, String, String, String), SiteCapability>,
    path: Option<std::path::PathBuf>,
}

impl CapabilityRegistry {
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    pub fn open(path: impl Into<std::path::PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let stored = match std::fs::read(&path) {
            Ok(bytes) if bytes.len() <= 1024 * 1024 => {
                serde_json::from_slice::<Vec<SiteCapability>>(&bytes)
                    .map_err(|error| format!("capability registry is invalid: {error}"))?
            }
            Ok(_) => return Err("capability registry exceeds 1 MiB".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(format!("cannot read capability registry: {error}")),
        };
        if stored.len() > MAX_CAPABILITIES {
            return Err("capability registry exceeds its entry bound".into());
        }
        let mut registry = Self {
            capabilities: BTreeMap::new(),
            path: Some(path),
        };
        for capability in stored {
            registry.validate(&capability).map_err(str::to_owned)?;
            registry
                .capabilities
                .insert(capability_key(&capability), capability);
        }
        Ok(registry)
    }

    pub fn register(&mut self, capability: SiteCapability) -> Result<(), &'static str> {
        self.validate(&capability)?;
        let key = capability_key(&capability);
        let previous = self.capabilities.insert(key.clone(), capability);
        if let Err(error) = self.persist() {
            if let Some(previous) = previous {
                self.capabilities.insert(key, previous);
            } else {
                self.capabilities.remove(&key);
            }
            return Err(error);
        }
        Ok(())
    }

    fn validate(&self, capability: &SiteCapability) -> Result<(), &'static str> {
        if capability.name.is_empty()
            || capability.name.len() > 128
            || capability.site_scope.len() > 512
            || capability.structural_signature.is_empty()
            || capability.structural_signature.len() > 256
            || capability.version == 0
            || capability.input_schema.as_object().is_none()
            || implementation_id(&capability.implementation).is_empty()
            || implementation_id(&capability.implementation).len() > 256
        {
            return Err("invalid or unbounded site capability");
        }
        if !self.capabilities.contains_key(&capability_key(capability))
            && self.capabilities.len() >= MAX_CAPABILITIES
        {
            return Err("site capability registry is full");
        }
        Ok(())
    }

    fn persist(&self) -> Result<(), &'static str> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = path
            .parent()
            .ok_or("capability registry path has no parent")?;
        std::fs::create_dir_all(parent)
            .map_err(|_| "cannot create capability registry directory")?;
        let mut entries = self.capabilities.values().cloned().collect::<Vec<_>>();
        entries.sort_by_key(capability_key);
        let bytes =
            serde_json::to_vec(&entries).map_err(|_| "cannot serialize capability registry")?;
        if bytes.len() > 1024 * 1024 {
            return Err("capability registry exceeds 1 MiB");
        }
        let mut temp = path.clone();
        temp.set_extension("tmp");
        std::fs::write(&temp, &bytes)
            .map_err(|_| "cannot write capability registry temporary file")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "cannot restrict capability registry permissions")?;
        }
        std::fs::rename(&temp, path).map_err(|_| "cannot commit capability registry")
    }

    /// Selects only a capability bound to the exact current site and structure.
    /// Read-only routing refuses unverified entries and all mutating effects.
    pub fn resolve(
        &self,
        name: &str,
        site_scope: &str,
        structural_signature: &str,
        read_only: bool,
    ) -> Option<&SiteCapability> {
        self.capabilities
            .values()
            .filter(|capability| {
                capability.name == name
                    && capability.site_scope == site_scope
                    && capability.structural_signature == structural_signature
                    && capability.independently_verified
                    && (!read_only || capability.effect == EffectClass::ReadOnly)
            })
            .min_by_key(|capability| capability.implementation.rank())
    }

    /// Returns an exact-site, independently verified WebMCP candidate by name.
    /// Its saved schema is only a warm-start: the page invocation must compare
    /// the live schema before dispatch and fail closed on drift.
    pub fn resolve_webmcp_candidate(
        &self,
        name: &str,
        site_scope: &str,
    ) -> Option<&SiteCapability> {
        self.capabilities.values().find(|capability| {
            capability.name == name
                && capability.site_scope == site_scope
                && capability.independently_verified
                && matches!(
                    capability.implementation,
                    CapabilityImplementation::WebMcp { .. }
                )
        })
    }

    /// Finds a previously verified read-only WebMCP operation at the exact
    /// origin. The saved schema fingerprint is checked against fresh page
    /// discovery by the caller before any dispatch.
    pub fn resolve_read_only_webmcp_operation(
        &self,
        operation: &str,
        site_scope: &str,
    ) -> Option<&SiteCapability> {
        let operation = normalize_operation(operation)?;
        self.capabilities.values().find(|capability| {
            capability.name == operation
                && capability.site_scope == site_scope
                && capability.independently_verified
                && capability.effect == EffectClass::ReadOnly
                && matches!(
                    &capability.implementation,
                    CapabilityImplementation::WebMcp { tool_name }
                        if tool_name == operation
                            && read_only_webmcp_operation(tool_name, &capability.input_schema) == Some(operation)
                )
        })
    }

    /// Resolves a runtime-recognized operation against an exact origin and
    /// structure. The operation name comes from a bounded intent vocabulary;
    /// page-provided descriptions never participate in routing.
    pub fn resolve_operation(
        &self,
        operation: &str,
        site_scope: &str,
        structural_signature: &str,
        read_only: bool,
    ) -> Option<&SiteCapability> {
        let operation = normalize_operation(operation)?;
        self.resolve(operation, site_scope, structural_signature, read_only)
    }
}

/// Return one unambiguous, supported operation mentioned in a task goal.
/// Ambiguous goals deliberately fall back to normal semantic planning.
pub fn operation_from_goal(goal: &str) -> Option<&'static str> {
    if goal.is_empty() || goal.len() > 512 {
        return None;
    }
    let words = goal
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let mut found = None;
    for operation in ["search", "filter", "sort", "paginate"] {
        if words.iter().any(|word| word == operation) {
            if found.is_some() {
                return None;
            }
            found = Some(operation);
        }
    }
    found
}

/// Construct a narrow operation input from task text and the saved tool
/// schema. Only a single required scalar field from a runtime-owned vocabulary
/// is supported; ambiguous or richer schemas use semantic UI planning.
pub fn operation_input(operation: &str, goal: &str, schema: &Value) -> Option<Value> {
    let operation = normalize_operation(operation)?;
    if goal.is_empty() || goal.len() > 512 || schema.get("type")?.as_str()? != "object" {
        return None;
    }
    let properties = schema.get("properties")?.as_object()?;
    let required = schema.get("required").and_then(Value::as_array)?;
    if required.len() != 1 {
        return None;
    }
    let name = required.first()?.as_str()?;
    let property = properties.get(name)?;
    if property.get("type").and_then(Value::as_str)? != "string" {
        return None;
    }
    let allowed = match operation {
        "search" => ["query", "q", "term", "search"].as_slice(),
        "filter" => ["filter", "query", "value"].as_slice(),
        "sort" => ["sort", "order", "value"].as_slice(),
        "paginate" => ["page", "cursor", "value"].as_slice(),
        _ => return None,
    };
    if !allowed.contains(&name) || properties.keys().any(|key| key != name) {
        return None;
    }
    let lower = goal.to_ascii_lowercase();
    let input = match operation {
        "search" => ["search for ", "search ", "find ", "look for "]
            .iter()
            .find_map(|prefix| {
                lower.find(prefix).map(|index| {
                    goal[index + prefix.len()..]
                        .trim()
                        .trim_matches(['?', '.', ' '])
                        .to_owned()
                })
            })
            .filter(|value| !value.is_empty()),
        "filter" => lower
            .find("filter ")
            .map(|index| {
                goal[index + 7..]
                    .trim()
                    .trim_matches(['?', '.', ' '])
                    .to_owned()
            })
            .filter(|value| !value.is_empty()),
        "sort" => lower
            .find("sort ")
            .map(|index| {
                goal[index + 5..]
                    .trim()
                    .trim_matches(['?', '.', ' '])
                    .to_owned()
            })
            .filter(|value| !value.is_empty()),
        "paginate" => goal
            .split_ascii_whitespace()
            .rev()
            .find(|word| word.chars().all(|ch| ch.is_ascii_digit()))
            .map(str::to_owned),
        _ => None,
    }?;
    (input.len() <= 512).then(|| serde_json::json!({name:input}))
}

/// Runtime-owned WebMCP read-operation allowlist. Page descriptions and
/// annotations are intentionally excluded; only exact names from the V3 task
/// vocabulary and a single bounded string input can qualify for task routing.
pub fn read_only_webmcp_operation(name: &str, schema: &Value) -> Option<&'static str> {
    let operation = normalize_operation(name)?;
    let properties = schema.get("properties")?.as_object()?;
    let required = schema.get("required")?.as_array()?;
    if schema.get("type")?.as_str()? != "object"
        || required.len() != 1
        || properties.len() != 1
        || !schema
            .get("additionalProperties")
            .is_some_and(|value| value == false)
    {
        return None;
    }
    let field = required.first()?.as_str()?;
    let field_schema = properties.get(field)?;
    if field_schema.get("type").and_then(Value::as_str)? != "string"
        || field_schema
            .get("maxLength")
            .and_then(Value::as_u64)
            .is_some_and(|max| max > 512)
    {
        return None;
    }
    let allowed = match operation {
        "search" => ["query", "q", "term", "search"].as_slice(),
        "filter" => ["filter", "query", "value"].as_slice(),
        "sort" => ["sort", "order", "value"].as_slice(),
        "paginate" => ["page", "cursor", "value"].as_slice(),
        _ => return None,
    };
    allowed.contains(&field).then_some(operation)
}

fn normalize_operation(operation: &str) -> Option<&'static str> {
    match operation {
        "search" => Some("search"),
        "filter" => Some("filter"),
        "sort" => Some("sort"),
        "paginate" => Some("paginate"),
        _ => None,
    }
}

fn capability_key(capability: &SiteCapability) -> (String, String, String, String) {
    (
        capability.site_scope.clone(),
        capability.structural_signature.clone(),
        capability.name.clone(),
        implementation_id(&capability.implementation).to_owned(),
    )
}

fn implementation_id(implementation: &CapabilityImplementation) -> &str {
    match implementation {
        CapabilityImplementation::WebMcp { tool_name } => tool_name,
        CapabilityImplementation::QualifiedSkill { skill_id } => skill_id,
        CapabilityImplementation::SiteProfile { profile_id } => profile_id,
    }
}
