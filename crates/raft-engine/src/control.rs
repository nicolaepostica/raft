//! Control-channel protocol: one JSON object per line.
//!
//! Requests:
//! - `{"method": "run", "command": "search.set", "params": {...}}`
//! - `{"method": "state"}`
//! - `{"method": "commands"}`
//! - `{"method": "ui.screenshot", "path": "/tmp/shot.png"}`
//! - `{"method": "results", "offset": 0, "limit": 20}`: the books found, a page at a time
//! - `{"method": "book", "id": "…"}`: one book with every field
//!
//! Replies: `{"ok": true, "result": ...}` or `{"ok": false, "error": "..."}`.

use serde_json::{Value, json};

use crate::registry;

/// Longest accepted request line, in bytes.
pub const MAX_LINE: usize = 64 * 1024;
/// Most books in one `results` page.
pub const MAX_RESULTS: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Run { command: String, params: Value },
    State,
    Commands,
    Screenshot { path: String },
    Results { offset: usize, limit: usize },
    Book { id: String },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ControlError {
    #[error("request is too long (max {MAX_LINE} bytes)")]
    TooLong,
    #[error("request is not valid JSON: {0}")]
    Json(String),
    #[error("bad request: {0}")]
    Bad(String),
}

/// Parses one request line.
pub fn parse_request(line: &[u8]) -> Result<Request, ControlError> {
    if line.len() > MAX_LINE {
        return Err(ControlError::TooLong);
    }
    let v: Value = serde_json::from_slice(line).map_err(|e| ControlError::Json(e.to_string()))?;
    let field = |k: &str| v.get(k).and_then(Value::as_str);
    match field("method") {
        Some("run") => {
            let command = field("command")
                .ok_or_else(|| ControlError::Bad("`run` needs `command`".into()))?
                .to_owned();
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            if !(params.is_object() || params.is_null()) {
                return Err(ControlError::Bad("`params` must be an object".into()));
            }
            Ok(Request::Run { command, params })
        }
        Some("state") => Ok(Request::State),
        Some("commands") => Ok(Request::Commands),
        Some("ui.screenshot") => {
            let path = field("path")
                .ok_or_else(|| ControlError::Bad("`ui.screenshot` needs `path`".into()))?;
            if !path.ends_with(".png") {
                return Err(ControlError::Bad("`path` must end with .png".into()));
            }
            Ok(Request::Screenshot {
                path: path.to_owned(),
            })
        }
        Some("results") => {
            let num = |k: &str, default: usize| match v.get(k) {
                None | Some(Value::Null) => Ok(default),
                Some(n) => n
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| ControlError::Bad(format!("`{k}` must be a whole number"))),
            };
            Ok(Request::Results {
                offset: num("offset", 0)?,
                limit: num("limit", 20)?.min(MAX_RESULTS),
            })
        }
        Some("book") => {
            let id = field("id").ok_or_else(|| ControlError::Bad("`book` needs `id`".into()))?;
            Ok(Request::Book { id: id.to_owned() })
        }
        Some(other) => Err(ControlError::Bad(format!("unknown method `{other}`"))),
        None => Err(ControlError::Bad("missing `method`".into())),
    }
}

pub fn ok(result: Value) -> Value {
    json!({ "ok": true, "result": result })
}

pub fn error(msg: impl std::fmt::Display) -> Value {
    json!({ "ok": false, "error": msg.to_string() })
}

/// The command list, for `{"method": "commands"}`.
fn shortcut_name(s: crate::Shortcut) -> String {
    let mut k = String::new();
    if s.command {
        k.push_str("Ctrl+");
    }
    if s.shift {
        k.push_str("Shift+");
    }
    if s.alt {
        k.push_str("Alt+");
    }
    k.push_str(s.key);
    k
}

pub fn commands_json() -> Value {
    Value::Array(
        registry()
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "label": c.label,
                    "menu": c.menu,
                    "shortcut": c.shortcut.map(shortcut_name),
                    "also": c.also.iter().copied().map(shortcut_name).collect::<Vec<_>>(),
                    "params": c.params,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parses_requests() {
        assert_eq!(
            parse_request(br#"{"method":"run","command":"search.set","params":{"text":"go"}}"#),
            Ok(Request::Run {
                command: "search.set".into(),
                params: json!({"text": "go"})
            })
        );
        assert_eq!(parse_request(br#"{"method":"state"}"#), Ok(Request::State));
        assert!(parse_request(br#"{"method":"ui.screenshot","path":"/x.txt"}"#).is_err());
        assert!(parse_request(br#"{"method":"run","command":"x","params":[1]}"#).is_err());
        assert!(parse_request(b"nope").is_err());
        assert!(parse_request(&vec![b' '; MAX_LINE + 1]).is_err());
        assert!(commands_json().as_array().is_some_and(|a| !a.is_empty()));
        assert_eq!(
            parse_request(br#"{"method":"results"}"#),
            Ok(Request::Results {
                offset: 0,
                limit: 20
            })
        );
        assert_eq!(
            parse_request(br#"{"method":"results","offset":40,"limit":100000}"#),
            Ok(Request::Results {
                offset: 40,
                limit: MAX_RESULTS
            })
        );
        assert!(parse_request(br#"{"method":"results","limit":-1}"#).is_err());
        assert_eq!(
            parse_request(br#"{"method":"book","id":"abc"}"#),
            Ok(Request::Book { id: "abc".into() })
        );
        assert!(parse_request(br#"{"method":"book"}"#).is_err());
    }

    proptest! {
        #[test]
        fn never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = parse_request(&bytes);
        }

        #[test]
        fn never_panics_on_json_shapes(method in ".{0,16}", command in ".{0,16}", path in ".{0,32}") {
            let line = json!({"method": method, "command": command, "path": path}).to_string();
            let _ = parse_request(line.as_bytes());
        }
    }
}
