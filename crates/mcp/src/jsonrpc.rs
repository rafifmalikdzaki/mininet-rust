//! JSON-RPC 2.0 envelope helpers.
//!
//! MCP's stdio transport is newline-delimited JSON-RPC 2.0: each line on
//! stdin/stdout is one self-contained JSON message. This module only builds
//! and inspects the envelope (`jsonrpc`/`id`/`result`/`error`); method
//! dispatch lives in [`crate::server`].

use serde_json::{json, Value};

/// The `jsonrpc` field value every message must carry.
pub const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC 2.0 reserved error codes (the ones MCP servers use).
pub mod code {
    /// Invalid JSON was received by the server (-32700).
    pub const PARSE_ERROR: i64 = -32700;
    /// The JSON sent is not a valid Request object (-32600).
    pub const INVALID_REQUEST: i64 = -32600;
    /// The method does not exist / is not available (-32601).
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Invalid method parameter(s) (-32602; MCP uses this for unknown
    /// tool names in `tools/call` too).
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal JSON-RPC error (-32603).
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// A success response: `{"jsonrpc":"2.0","id":id,"result":result}`.
pub fn result_response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": JSONRPC_VERSION, "id": id, "result": result})
}

/// An error response:
/// `{"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}`.
pub fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": {"code": code, "message": message.into()},
    })
}

/// Build an error response for a message whose `id` may or may not be
/// present (notifications carry none; then the spec-mandated `null` is used).
pub fn error_for(id: Option<Value>, code: i64, message: impl Into<String>) -> Value {
    error_response(id.unwrap_or(Value::Null), code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_response_shape() {
        let r = result_response(json!(1), json!({"ok": true}));
        assert_eq!(r["jsonrpc"], "2.0");
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["ok"], true);
        assert!(r.get("error").is_none());
    }

    #[test]
    fn error_response_shape() {
        let r = error_response(json!("abc"), code::METHOD_NOT_FOUND, "nope");
        assert_eq!(r["error"]["code"], -32601);
        assert_eq!(r["error"]["message"], "nope");
        assert_eq!(r["id"], "abc");
        assert!(r.get("result").is_none());
    }

    #[test]
    fn error_for_defaults_id_to_null() {
        let r = error_for(None, code::PARSE_ERROR, "Parse error");
        assert_eq!(r["id"], Value::Null);
        assert_eq!(r["error"]["code"], -32700);
    }
}
