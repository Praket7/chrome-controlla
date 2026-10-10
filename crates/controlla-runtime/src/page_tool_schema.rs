//! Bounded recursive native page-tool schema validation.
use serde_json::Value;

pub const MAX_SCHEMA_BYTES: usize = 8192;
pub const MAX_DEPTH: usize = 4;
pub const MAX_PROPERTIES: usize = 32;
pub const MAX_ENUM_VALUES: usize = 32;

#[derive(Clone, Debug)]
pub enum SchemaKind {
    String,
    Boolean,
    Number,
    Enum(Vec<String>),
    Array(Box<Schema>),
    Object(BTreeMap<String, Schema>),
}

use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Schema {
    pub kind: SchemaKind,
    pub required: bool,
}

pub fn validate_schema(value: &Value) -> Result<Schema, String> {
    let s = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if s.len() > MAX_SCHEMA_BYTES {
        return Err("schema exceeds size limit".into());
    }
    validate_value(value, 0)
}

fn validate_value(value: &Value, depth: usize) -> Result<Schema, String> {
    if depth > MAX_DEPTH {
        return Err("schema depth exceeded".into());
    }
    let obj = value.as_object().ok_or("schema must be object")?;
    let ty = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or("missing type")?;
    Ok(match ty {
        "string" => Schema {
            kind: SchemaKind::String,
            required: obj
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        "boolean" => Schema {
            kind: SchemaKind::Boolean,
            required: false,
        },
        "number" => Schema {
            kind: SchemaKind::Number,
            required: false,
        },
        "enum" => {
            let vals = obj
                .get("values")
                .and_then(Value::as_array)
                .ok_or("enum missing values")?;
            if vals.len() > MAX_ENUM_VALUES {
                return Err("too many enum values".into());
            }
            let mut v = Vec::new();
            for val in vals {
                v.push(val.as_str().ok_or("enum value not string")?.to_owned());
            }
            Schema {
                kind: SchemaKind::Enum(v),
                required: false,
            }
        }
        "array" => {
            let items = obj.get("items").ok_or("array missing items")?;
            Schema {
                kind: SchemaKind::Array(Box::new(validate_value(items, depth + 1)?)),
                required: false,
            }
        }
        "object" => {
            let props = obj
                .get("properties")
                .and_then(Value::as_object)
                .ok_or("object missing properties")?;
            if props.len() > MAX_PROPERTIES {
                return Err("too many properties".into());
            }
            let mut map = BTreeMap::new();
            for (k, v) in props {
                map.insert(k.clone(), validate_value(v, depth + 1)?);
            }
            Schema {
                kind: SchemaKind::Object(map),
                required: false,
            }
        }
        _ => return Err(format!("unknown type {ty}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn bounded_objects_and_arrays() {
        let schema = json!({"type":"object","properties":{"name":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}}}});
        assert!(validate_schema(&schema).is_ok());
        let deep = json!({"type":"object","properties":{"a":{"type":"object","properties":{"b":{"type":"object","properties":{"c":{"type":"object","properties":{"d":{"type":"object","properties":{"e":{"type":"string"}}}}}}}}}}});
        assert!(validate_schema(&deep).is_err());
    }
}
