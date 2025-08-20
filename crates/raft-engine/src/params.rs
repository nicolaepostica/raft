//! Typed access to JSON command parameters (untrusted).

use serde_json::Value;

use crate::EngineError;

/// Longest accepted string parameter, in bytes.
pub const MAX_PARAM_STR: usize = 4096;

pub fn opt_str<'a>(p: &'a Value, key: &str) -> Result<Option<&'a str>, EngineError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= MAX_PARAM_STR => Ok(Some(s)),
        Some(Value::String(_)) => Err(EngineError::BadParams(format!("`{key}` is too long"))),
        Some(_) => Err(EngineError::BadParams(format!("`{key}` must be a string"))),
    }
}

pub fn str<'a>(p: &'a Value, key: &str) -> Result<&'a str, EngineError> {
    opt_str(p, key)?.ok_or_else(|| EngineError::BadParams(format!("missing `{key}`")))
}

pub fn opt_i64(p: &Value, key: &str) -> Result<Option<i64>, EngineError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| EngineError::BadParams(format!("`{key}` must be an integer"))),
    }
}

pub fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, EngineError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(EngineError::BadParams(format!("`{key}` must be a boolean"))),
    }
}
