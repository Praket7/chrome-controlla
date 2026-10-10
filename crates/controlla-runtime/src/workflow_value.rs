use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowValue {
    Reference(String),
    References(Vec<String>),
    String(String),
    Number(f64),
    Bool(bool),
    Array(Vec<WorkflowValue>),
    Object(BTreeMap<String, WorkflowValue>),
    Null,
}

impl WorkflowValue {
    pub fn from_json(value: Value) -> Result<Self, String> {
        match value {
            Value::String(s) if s.starts_with("@c") => Ok(Self::Reference(s)),
            Value::String(s) => Ok(Self::String(s)),
            Value::Number(n) => Ok(Self::Number(n.as_f64().ok_or("invalid number")?)),
            Value::Bool(b) => Ok(Self::Bool(b)),
            Value::Null => Ok(Self::Null),
            Value::Array(arr) => {
                if arr.len() > 128 {
                    return Err("array exceeds bound 128".into());
                }
                let mut out = Vec::with_capacity(arr.len());
                for v in arr {
                    out.push(Self::from_json(v)?);
                }
                Ok(Self::Array(out))
            }
            Value::Object(map) => {
                if map.len() > 64 {
                    return Err("object exceeds bound 64".into());
                }
                let mut out = BTreeMap::new();
                for (k, v) in map {
                    if k.len() > 128 {
                        return Err("object key too long".into());
                    }
                    out.insert(k, Self::from_json(v)?);
                }
                Ok(Self::Object(out))
            }
        }
    }
    pub fn to_json(&self) -> Value {
        match self {
            Self::Reference(s) => Value::String(s.clone()),
            Self::References(v) => {
                Value::Array(v.iter().map(|s| Value::String(s.clone())).collect())
            }
            Self::String(s) => Value::String(s.clone()),
            Self::Number(n) => serde_json::json!(*n),
            Self::Bool(b) => Value::Bool(*b),
            Self::Array(arr) => Value::Array(arr.iter().map(|v| v.to_json()).collect()),
            Self::Object(map) => {
                let mut o = serde_json::Map::new();
                for (k, v) in map {
                    o.insert(k.clone(), v.to_json());
                }
                Value::Object(o)
            }
            Self::Null => Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_array_rejects_oversize() {
        let big = Value::Array((0..129).map(Value::from).collect());
        assert!(WorkflowValue::from_json(big).is_err());
    }
}
