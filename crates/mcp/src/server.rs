//! The MCP server: JSON-RPC 2.0 over stdio with newline-delimited JSON
//! framing.
//!
//! Supported methods: `initialize`, `notifications/initialized` (no-op),
//! `ping`, `tools/list`, `tools/call`. [`Server::run`] drives any
//! reader/writer pair, so tests use in-memory pipes; [`Server::handle_line`]
//! exposes single-message handling for finer-grained tests.

use std::io::{self, BufRead, Write};

use serde::Serialize;
use serde_json::{json, Value};

use crate::jsonrpc::{code, error_for, error_response, result_response, JSONRPC_VERSION};
use crate::session::Session;
use crate::tools::{error_content, registry, Tool};

/// MCP protocol version this server speaks (2024-11-05).
pub const PROTOCOL_VERSION: &str = "2024-11-05";
/// `serverInfo.name`.
pub const SERVER_NAME: &str = "mn-mcp";

/// One entry of the `tools/list` result (serialized with the MCP
/// `inputSchema` field name).
#[derive(Serialize)]
struct ToolInfo<'a> {
    name: &'a str,
    description: &'a str,
    #[serde(rename = "inputSchema")]
    input_schema: &'a Value,
}

/// The MCP server: a tool registry plus one [`Session`].
#[derive(Debug)]
pub struct Server {
    /// The per-connection session (holds the optional built [`Mininet`]).
    pub session: Session,
}

impl Server {
    /// New server with a fresh empty session.
    pub fn new() -> Self {
        Self {
            session: Session::new(),
        }
    }

    /// New server with a pre-populated session (e.g. `--spec` preload).
    pub fn with_session(session: Session) -> Self {
        Self { session }
    }

    /// Serve newline-delimited JSON-RPC messages from `reader` to `writer`
    /// until EOF. Blank lines are skipped; notifications produce no output.
    pub fn run<R: BufRead, W: Write>(&mut self, mut reader: R, mut writer: W) -> io::Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return Ok(()); // EOF: client closed the pipe
            }
            let msg = line.trim();
            if msg.is_empty() {
                continue;
            }
            if let Some(resp) = self.handle_line(msg) {
                serde_json::to_writer(&mut writer, &resp)?;
                writer.write_all(b"\n")?;
                writer.flush()?;
            }
        }
    }

    /// Handle one raw line (one JSON message). Returns the response to send
    /// back, or `None` for notifications. Malformed JSON gets a -32700
    /// response with `id: null`.
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        match serde_json::from_str::<Value>(line) {
            Err(_) => Some(error_response(
                Value::Null,
                code::PARSE_ERROR,
                "Parse error: not valid JSON",
            )),
            Ok(msg) => self.handle_message(msg),
        }
    }

    /// Dispatch one parsed JSON-RPC message.
    pub fn handle_message(&mut self, msg: Value) -> Option<Value> {
        let Some(obj) = msg.as_object() else {
            return Some(error_for(
                None,
                code::INVALID_REQUEST,
                "Invalid Request: not an object",
            ));
        };
        let id = obj.get("id").cloned();
        if obj.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
            return Some(error_for(
                id,
                code::INVALID_REQUEST,
                "Invalid Request: \"jsonrpc\" must be \"2.0\"",
            ));
        }
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            return Some(error_for(
                id,
                code::INVALID_REQUEST,
                "Invalid Request: \"method\" must be a string",
            ));
        };
        // Notification no-op: a message without an "id" member is a
        // notification and is NEVER answered, even on error (JSON-RPC
        // 2.0). MCP's "notifications/initialized" handshake is explicitly
        // no-oped HERE, before the request handling below — the old code
        // did `let id = id?;` first and then had a dedicated
        // "notifications/initialized" match arm that was unreachable
        // (the `?` had already returned None for it).
        let id = id?;
        let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
        Some(match method {
            "initialize" => result_response(id, self.initialize_result()),
            "ping" => result_response(id, json!({})),
            "tools/list" => result_response(id, self.tools_list_result()),
            "tools/call" => self.tools_call(id, &params),
            other => error_response(
                id,
                code::METHOD_NOT_FOUND,
                format!("Method not found: {other}"),
            ),
        })
    }

    /// The `initialize` result: protocol version, tool capability, server
    /// info (we always answer with the version we support).
    fn initialize_result(&self) -> Value {
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
        })
    }

    /// The `tools/list` result.
    fn tools_list_result(&self) -> Value {
        let reg = registry();
        let tools: Vec<ToolInfo<'_>> = reg
            .iter()
            .map(|t: &Tool| ToolInfo {
                name: t.name,
                description: t.description,
                input_schema: &t.input_schema,
            })
            .collect();
        json!({"tools": tools})
    }

    /// The `tools/call` dispatch: validate params (-32602), find the tool
    /// (-32602 for unknown names), run the handler (its `Err` becomes an
    /// `isError: true` result).
    fn tools_call(&mut self, id: Value, params: &Value) -> Value {
        if !params.is_object() {
            return error_response(
                id,
                code::INVALID_PARAMS,
                "Invalid params: expected an object with 'name' and optional 'arguments'",
            );
        }
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return error_response(
                id,
                code::INVALID_PARAMS,
                "Invalid params: 'name' must be a string",
            );
        };
        let Some(tool) = registry().into_iter().find(|t| t.name == name) else {
            return error_response(
                id,
                code::INVALID_PARAMS,
                format!("Invalid params: unknown tool '{name}'"),
            );
        };
        let args = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(a) if a.is_object() => a.clone(),
            Some(_) => {
                return error_response(
                    id,
                    code::INVALID_PARAMS,
                    "Invalid params: 'arguments' must be an object",
                )
            }
        };
        match (tool.handler)(&mut self.session, &args) {
            Ok(result) => result_response(id, result),
            Err(msg) => result_response(id, error_content(msg)),
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}
