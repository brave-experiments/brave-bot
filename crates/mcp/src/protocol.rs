//! JSON-RPC 2.0 messages for the Model Context Protocol.
//!
//! Only what a client needs: initialise, list tools, call a tool. Server-to-client
//! requests are not handled: a server that asks the client to do something is not
//! supported, which keeps the trust direction one-way.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSONRPC_VERSION: &str = "2.0";

/// Protocol revision this client implements.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Clone, Serialize)]
pub struct RpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl RpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            method: method.into(),
            params,
        }
    }
}

/// A notification carries no id and expects no reply.
#[derive(Debug, Clone, Serialize)]
pub struct RpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl RpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RpcResponse {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// One tool a server offers.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema for the arguments. Passed through to the model unchanged.
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct ToolList {
    #[serde(default)]
    pub tools: Vec<ToolDescriptor>,
}

/// One piece of a tool result.
///
/// Only text is extracted. Other content types are ignored rather than rejected, so an
/// unfamiliar block does not fail the call.
#[derive(Debug, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ToolResult {
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    /// Servers signal tool-level failure here rather than with a JSON-RPC error.
    #[serde(default, rename = "isError")]
    pub is_error: bool,
}

impl ToolResult {
    /// All text blocks joined. Everything else is dropped.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Params for `initialize`.
pub fn initialize_params(client_name: &str, client_version: &str) -> Value {
    serde_json::json!({
        "protocolVersion": PROTOCOL_VERSION,
        // No capabilities are advertised: this client does not accept
        // server-initiated requests such as sampling or roots.
        "capabilities": {},
        "clientInfo": { "name": client_name, "version": client_version },
    })
}

/// Params for `tools/call`.
pub fn call_params(name: &str, arguments: Value) -> Value {
    serde_json::json!({ "name": name, "arguments": arguments })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_carries_the_jsonrpc_version() {
        let request = RpcRequest::new(1, "tools/list", None);
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["method"], "tools/list");
        // Omitted rather than sent as null.
        assert!(json.get("params").is_none());
    }

    #[test]
    fn a_notification_has_no_id() {
        let notification = RpcNotification::new("notifications/initialized", None);
        let json = serde_json::to_value(&notification).unwrap();
        assert!(json.get("id").is_none());
        assert_eq!(json["method"], "notifications/initialized");
    }

    #[test]
    fn a_successful_response_parses() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let response: RpcResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(response.id, Some(1));
        assert!(response.error.is_none());
        assert!(response.result.is_some());
    }

    #[test]
    fn an_error_response_parses() {
        let raw =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found"}}"#;
        let response: RpcResponse = serde_json::from_str(raw).unwrap();
        let error = response.error.expect("error present");
        assert_eq!(error.code, -32601);
        assert_eq!(error.message, "method not found");
    }

    #[test]
    fn a_tool_list_parses_with_schemas() {
        let raw = r#"{"tools":[
            {"name":"search","description":"search the web","inputSchema":{"type":"object"}},
            {"name":"bare"}
        ]}"#;
        let list: ToolList = serde_json::from_str(raw).unwrap();
        assert_eq!(list.tools.len(), 2);
        assert_eq!(list.tools[0].name, "search");
        assert!(list.tools[0].input_schema.is_some());
        // A tool without a description or schema is still usable.
        assert!(list.tools[1].description.is_none());
    }

    #[test]
    fn tool_result_text_is_joined() {
        let raw = r#"{"content":[{"type":"text","text":"first"},{"type":"text","text":"second"}]}"#;
        let result: ToolResult = serde_json::from_str(raw).unwrap();
        assert_eq!(result.text(), "first\nsecond");
        assert!(!result.is_error);
    }

    /// An unfamiliar content type must not break the call.
    #[test]
    fn non_text_content_is_ignored() {
        let raw = r#"{"content":[
            {"type":"image","data":"base64..."},
            {"type":"text","text":"caption"}
        ]}"#;
        let result: ToolResult = serde_json::from_str(raw).unwrap();
        assert_eq!(result.text(), "caption");
    }

    #[test]
    fn a_tool_level_error_is_visible() {
        let raw = r#"{"content":[{"type":"text","text":"it failed"}],"isError":true}"#;
        let result: ToolResult = serde_json::from_str(raw).unwrap();
        assert!(result.is_error);
        assert_eq!(result.text(), "it failed");
    }

    /// No capabilities are advertised, so a server cannot ask this client to act.
    #[test]
    fn initialize_advertises_no_capabilities() {
        let params = initialize_params("bravebot", "0.1.0");
        assert_eq!(params["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(params["capabilities"], serde_json::json!({}));
        assert_eq!(params["clientInfo"]["name"], "bravebot");
    }

    #[test]
    fn call_params_carry_the_name_and_arguments() {
        let params = call_params("search", serde_json::json!({"query": "rust"}));
        assert_eq!(params["name"], "search");
        assert_eq!(params["arguments"]["query"], "rust");
    }
}
