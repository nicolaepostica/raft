//! MCP (Model Context Protocol) server over the control channel: JSON-RPC 2.0, one
//! message per line (the stdio transport). Each tool becomes a control request, so an
//! agent reaches the same commands as the menus, the keyboard and `--control`.
//!
//! The transport and the connection to the running app are the binary's job
//! (`raft mcp`); this module only turns messages into replies, through a [`Bridge`].

use serde_json::{Map, Value, json};

use crate::control::{MAX_LINE, MAX_RESULTS};

/// The protocol revision this server speaks; offered when the client asks for another.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Revisions the server accepts as they are.
const KNOWN_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// The running app, as the MCP server sees it.
pub trait Bridge {
    /// Sends one control request (`{"method": "run", …}`) and returns its result, or the
    /// error the app or the connection gave.
    fn call(&mut self, request: &Value) -> Result<Value, String>;
    /// A PNG of the app's window.
    fn screenshot(&mut self) -> Result<Vec<u8>, String>;
}

/// Answers one line from the client; `None` for a notification (nothing to send back).
pub fn handle(line: &[u8], bridge: &mut dyn Bridge, version: &str) -> Option<Value> {
    if line.len() > MAX_LINE {
        return Some(rpc_error(
            &Value::Null,
            INVALID_REQUEST,
            "message is too long",
        ));
    }
    let msg: Value = match serde_json::from_slice(line) {
        Ok(v) => v,
        Err(e) => return Some(rpc_error(&Value::Null, PARSE_ERROR, &e.to_string())),
    };
    let Some(obj) = msg.as_object() else {
        // Batches were dropped from the protocol in 2025-06-18.
        return Some(rpc_error(
            &Value::Null,
            INVALID_REQUEST,
            "expected one JSON object",
        ));
    };
    let method = obj.get("method").and_then(Value::as_str);
    let Some(id) = obj.get("id").filter(|id| id.is_string() || id.is_number()) else {
        // A notification (`notifications/initialized`, `…/cancelled`) or a reply to us.
        return None;
    };
    let Some(method) = method else {
        return Some(rpc_error(id, INVALID_REQUEST, "missing `method`"));
    };
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(initialize(&params, version)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call_tool(&params, bridge),
        other => Err((METHOD_NOT_FOUND, format!("unknown method `{other}`"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => rpc_error(id, code, &message),
    })
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize(params: &Value, version: &str) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let protocol = asked
        .filter(|v| KNOWN_VERSIONS.contains(v))
        .unwrap_or(PROTOCOL_VERSION);
    json!({
        "protocolVersion": protocol,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "raft", "title": "Raft", "version": version },
        "instructions": "Raft is a desktop book library. Every user action is a command: \
            raft_commands lists them with their params, raft_run runs one (search.set, \
            filter.*, view.book, book.read, …). raft_results pages through the books the \
            current search and filters find, raft_book shows one book, raft_state the app's \
            state, raft_screenshot the window. Changes go into the app's undo history \
            (edit.undo).",
    })
}

/// The tools, each a control request.
fn tools() -> Value {
    json!([
        {
            "name": "raft_commands",
            "title": "List Raft's commands",
            "description": "Every command with its id, label, menu, shortcut and a description of its params.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "raft_run",
            "title": "Run a command",
            "description": "Runs one of Raft's commands, as its menu or shortcut would. Examples: \
                {\"command\": \"search.set\", \"params\": {\"text\": \"rust\"}}, \
                {\"command\": \"view.book\", \"params\": {\"id\": \"<book id>\"}}, \
                {\"command\": \"filter.clear\"}. Undo with {\"command\": \"edit.undo\"}.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Command id, from raft_commands" },
                    "params": { "type": "object", "description": "The command's params, if it takes any" },
                },
                "required": ["command"],
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
        },
        {
            "name": "raft_state",
            "title": "Raft's state",
            "description": "The current screen (route), query and filters, result count, undo/redo, theme, language, library folders and settings.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "raft_results",
            "title": "Books found",
            "description": "A page of the books the current search and filters find, in their order: id, title, authors, year, format, language, pages, series, categories, file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "offset": { "type": "integer", "minimum": 0, "default": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_RESULTS, "default": 20 },
                },
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "raft_book",
            "title": "One book",
            "description": "Every field of one book (with hand corrections applied), by id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "The book's id (md5 of its file)" } },
                "required": ["id"],
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "raft_screenshot",
            "title": "Screenshot of Raft",
            "description": "A PNG of Raft's window as it is now.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true },
        },
    ])
}

fn call_tool(params: &Value, bridge: &mut dyn Bridge) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| (INVALID_PARAMS, "`tools/call` needs `name`".to_owned()))?;
    let empty = Map::new();
    let args = match params.get("arguments") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(m)) => m,
        Some(_) => return Err((INVALID_PARAMS, "`arguments` must be an object".into())),
    };
    let request = match name {
        "raft_commands" => json!({ "method": "commands" }),
        "raft_state" => json!({ "method": "state" }),
        "raft_run" => {
            let Some(command) = args.get("command").and_then(Value::as_str) else {
                return Ok(tool_error(
                    "`command` is required (an id from raft_commands)",
                ));
            };
            let params = args.get("params").cloned().unwrap_or(Value::Null);
            if !(params.is_object() || params.is_null()) {
                return Ok(tool_error("`params` must be an object"));
            }
            json!({ "method": "run", "command": command, "params": params })
        }
        "raft_results" => json!({
            "method": "results",
            "offset": args.get("offset").cloned().unwrap_or(Value::Null),
            "limit": args.get("limit").cloned().unwrap_or(Value::Null),
        }),
        "raft_book" => {
            let Some(id) = args.get("id").and_then(Value::as_str) else {
                return Ok(tool_error("`id` is required"));
            };
            json!({ "method": "book", "id": id })
        }
        "raft_screenshot" => {
            return Ok(match bridge.screenshot() {
                Ok(png) => json!({
                    "content": [{ "type": "image", "data": base64(&png), "mimeType": "image/png" }],
                }),
                Err(e) => tool_error(&e),
            });
        }
        other => return Err((INVALID_PARAMS, format!("unknown tool `{other}`"))),
    };
    Ok(match bridge.call(&request) {
        Ok(result) => {
            let text = serde_json::to_string_pretty(&result).unwrap_or_default();
            json!({ "content": [{ "type": "text", "text": text }] })
        }
        Err(e) => tool_error(&e),
    })
}

/// A tool that ran and failed: the agent sees the message and can correct itself.
fn tool_error(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

/// Standard base64 with padding (RFC 4648 §4).
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let sym = |n: u32| char::from(ABC.get((n & 63) as usize).copied().unwrap_or(b'A'));
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (b(0) << 16) | (b(1) << 8) | b(2);
        out.push(sym(n >> 18));
        out.push(sym(n >> 12));
        out.push(if chunk.len() > 1 { sym(n >> 6) } else { '=' });
        out.push(if chunk.len() > 2 { sym(n) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Records the requests and answers from a script.
    #[derive(Default)]
    struct Fake {
        sent: Vec<Value>,
        fail: Option<String>,
    }

    impl Bridge for Fake {
        fn call(&mut self, request: &Value) -> Result<Value, String> {
            self.sent.push(request.clone());
            match &self.fail {
                Some(e) => Err(e.clone()),
                None => Ok(json!({ "echo": request })),
            }
        }
        fn screenshot(&mut self) -> Result<Vec<u8>, String> {
            Ok(b"PNG!".to_vec())
        }
    }

    fn ask(bridge: &mut Fake, msg: Value) -> Option<Value> {
        handle(msg.to_string().as_bytes(), bridge, "9.9.9")
    }

    #[test]
    fn initialize_and_list_tools() {
        let mut f = Fake::default();
        let r = ask(
            &mut f,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"protocolVersion": "2025-03-26", "capabilities": {}}}),
        )
        .unwrap();
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["version"], "9.9.9");
        let r = ask(
            &mut f,
            json!({"jsonrpc": "2.0", "id": 2, "method": "initialize",
                   "params": {"protocolVersion": "2099-01-01"}}),
        )
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
        // Notifications get no reply.
        assert_eq!(
            ask(
                &mut f,
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
            ),
            None
        );
        let r = ask(
            &mut f,
            json!({"jsonrpc": "2.0", "id": "x", "method": "tools/list"}),
        )
        .unwrap();
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "raft_commands",
                "raft_run",
                "raft_state",
                "raft_results",
                "raft_book",
                "raft_screenshot"
            ]
        );
        assert!(f.sent.is_empty());
    }

    #[test]
    fn tools_become_control_requests() {
        let mut f = Fake::default();
        let call = |f: &mut Fake, name: &str, args: Value| {
            ask(
                f,
                json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}),
            )
            .unwrap()
        };
        let r = call(
            &mut f,
            "raft_run",
            json!({"command": "search.set", "params": {"text": "go"}}),
        );
        assert_eq!(
            f.sent.last(),
            Some(&json!({"method": "run", "command": "search.set", "params": {"text": "go"}}))
        );
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("search.set")
        );
        assert_eq!(r["result"].get("isError"), None);
        call(&mut f, "raft_results", json!({"limit": 5}));
        assert_eq!(
            f.sent.last(),
            Some(&json!({"method": "results", "offset": null, "limit": 5}))
        );
        call(&mut f, "raft_book", json!({"id": "abc"}));
        assert_eq!(f.sent.last(), Some(&json!({"method": "book", "id": "abc"})));
        call(&mut f, "raft_state", Value::Null);
        assert_eq!(f.sent.last(), Some(&json!({"method": "state"})));

        // Mistakes in the arguments are tool errors the agent can read, not protocol errors.
        let n = f.sent.len();
        let r = call(&mut f, "raft_run", json!({"params": {}}));
        assert_eq!(r["result"]["isError"], true);
        let r = call(&mut f, "raft_run", json!({"command": "x", "params": [1]}));
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(f.sent.len(), n);
        // An unknown tool is a protocol error.
        let r = call(&mut f, "nope", json!({}));
        assert_eq!(r["error"]["code"], INVALID_PARAMS);

        // The app's errors (an unknown command, no app running) come back as tool errors.
        f.fail = Some("unknown command `x`".into());
        let r = call(&mut f, "raft_run", json!({"command": "x"}));
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["content"][0]["text"], "unknown command `x`");

        let r = call(&mut f, "raft_screenshot", json!({}));
        assert_eq!(r["result"]["content"][0]["type"], "image");
        assert_eq!(r["result"]["content"][0]["data"], "UE5HIQ==");
    }

    #[test]
    fn bad_messages_get_json_rpc_errors() {
        let mut f = Fake::default();
        let r = handle(b"{nope", &mut f, "1").unwrap();
        assert_eq!(r["error"]["code"], PARSE_ERROR);
        assert_eq!(r["id"], Value::Null);
        let r = handle(b"[]", &mut f, "1").unwrap();
        assert_eq!(r["error"]["code"], INVALID_REQUEST);
        let r = ask(
            &mut f,
            json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}),
        )
        .unwrap();
        assert_eq!(r["error"]["code"], METHOD_NOT_FOUND);
        let r = ask(&mut f, json!({"jsonrpc": "2.0", "id": 4})).unwrap();
        assert_eq!(r["error"]["code"], INVALID_REQUEST);
        let r = ask(&mut f, json!({"jsonrpc": "2.0", "id": 5, "method": "ping"})).unwrap();
        assert_eq!(r["result"], json!({}));
        let long = vec![b' '; MAX_LINE + 1];
        assert!(handle(&long, &mut f, "1").is_some());
    }

    #[test]
    fn base64_matches_rfc_4648() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, coded) in cases {
            assert_eq!(base64(plain.as_bytes()), coded);
        }
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    proptest! {
        #[test]
        fn never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = handle(&bytes, &mut Fake::default(), "1");
        }

        #[test]
        fn never_panics_on_tool_calls(name in ".{0,16}", key in ".{0,8}", value in ".{0,16}", id in 0i64..5) {
            let msg = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                             "params": {"name": name, "arguments": {key: value}}});
            let _ = handle(msg.to_string().as_bytes(), &mut Fake::default(), "1");
        }
    }
}
