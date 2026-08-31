//! HTTP transport.
//!
//! A remote server at a user-configured URL. Unlike stdio there is no process to confine,
//! so the protections are different: every request goes through the egress chokepoint, so
//! the policy gate sees it and each redirect hop is revalidated.
//!
//! That matters more here than for the model endpoint. MCP servers are arbitrary URLs a
//! user adds, not one hardcoded host, so a server that redirects elsewhere is a realistic
//! way to reach an unintended destination.

use crate::protocol::{
    RpcRequest, RpcResponse, ToolDescriptor, ToolList, ToolResult, call_params, initialize_params,
};
use crate::{McpError, McpResult};
use bravebot_core::capability::Capability;
use bravebot_core::event::Sink;
use bravebot_core::policy::Policy;
use bravebot_core::value::Labelled;
use bravebot_net::{Egress, Request};
use serde_json::Value;

/// A server reached over HTTP.
#[derive(Debug)]
pub struct HttpServer {
    url: String,
    name: String,
    next_id: u64,
    /// Set from the initialize response, and echoed on later requests. Servers that keep
    /// state across calls require it.
    session: Option<String>,
}

impl HttpServer {
    /// Configure a server. No request is made yet.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            name: name.into(),
            next_id: 1,
            session: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    fn send<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        egress: &Egress,
        method: &str,
        params: Option<Value>,
    ) -> McpResult<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let body = serde_json::to_vec(&RpcRequest::new(id, method, params))
            .map_err(|e| McpError::Transport(format!("could not encode {method}: {e}")))?;

        let mut request = Request::post(&self.url, body)
            .header("content-type", "application/json")
            // Servers may reply with either, and the streaming form is accepted so a
            // server that prefers it is not rejected outright.
            .header("accept", "application/json, text/event-stream");

        if let Some(session) = &self.session {
            request = request.header("mcp-session-id", session);
        }

        // Untrusted-public: a remote server's reply is third-party content.
        let response = egress
            .fetch(
                policy,
                request,
                bravebot_core::label::Label::untrusted_public(),
            )
            .map_err(|e| match e {
                bravebot_net::EgressError::Denied(d) => McpError::Denied(d),
                other => McpError::Transport(other.to_string()),
            })?;

        // Decoding the envelope needs the bytes; the label is reapplied to extracted
        // content by the caller.
        let (bytes, _label) = response.body.into_parts_for_decoding();

        // A server may frame its reply as SSE even when JSON was requested.
        let text = String::from_utf8_lossy(&bytes);
        let payload = extract_json(&text).ok_or_else(|| {
            McpError::Transport(format!(
                "{method} returned no json payload ({} bytes)",
                bytes.len()
            ))
        })?;

        let parsed: RpcResponse = serde_json::from_str(payload)
            .map_err(|e| McpError::Transport(format!("malformed reply to {method}: {e}")))?;

        if let Some(error) = parsed.error {
            return Err(McpError::Server {
                code: error.code,
                message: error.message,
            });
        }

        parsed.result.ok_or_else(|| {
            McpError::Transport(format!("{method} returned neither a result nor an error"))
        })
    }

    /// Complete the handshake.
    pub fn initialize<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        egress: &Egress,
        client_name: &str,
        client_version: &str,
    ) -> McpResult<()> {
        self.send(
            policy,
            egress,
            "initialize",
            Some(initialize_params(client_name, client_version)),
        )?;
        Ok(())
    }

    /// List the tools this server offers.
    pub fn list_tools<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        egress: &Egress,
    ) -> McpResult<Vec<ToolDescriptor>> {
        let result = self.send(policy, egress, "tools/list", None)?;
        let list: ToolList = serde_json::from_value(result)
            .map_err(|e| McpError::Transport(format!("malformed tool list: {e}")))?;
        Ok(list.tools)
    }

    /// Call a tool.
    pub fn call_tool<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        egress: &Egress,
        tool: &str,
        arguments: Value,
    ) -> McpResult<Labelled<String>> {
        policy
            .before_capability(Capability::McpCall)
            .map_err(McpError::Denied)?;

        let result = self.send(
            policy,
            egress,
            "tools/call",
            Some(call_params(tool, arguments)),
        )?;

        let parsed: ToolResult = serde_json::from_value(result)
            .map_err(|e| McpError::Transport(format!("malformed tool result: {e}")))?;

        if parsed.is_error {
            return Err(McpError::ToolFailed {
                tool: tool.to_string(),
                detail: parsed.text(),
            });
        }

        let label = policy
            .observe(Capability::McpCall)
            .map_err(McpError::Denied)?;

        Ok(Labelled::new(parsed.text(), label))
    }
}

/// Pull the JSON payload out of a reply that may be plain JSON or SSE-framed.
///
/// SSE puts the payload on a `data:` line. Handling both means a server that upgrades to
/// streaming does not break the client.
fn extract_json(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        return Some(trimmed);
    }

    // Last data line wins: earlier ones may be progress notifications.
    trimmed
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .rfind(|payload| payload.starts_with('{'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_json_is_extracted_as_is() {
        let payload = extract_json(r#"{"jsonrpc":"2.0","id":1}"#).expect("json");
        assert!(payload.starts_with('{'));
    }

    #[test]
    fn whitespace_around_json_is_tolerated() {
        assert!(extract_json("\n  {\"id\":1}  \n").is_some());
    }

    #[test]
    fn an_sse_framed_reply_is_unwrapped() {
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let payload = extract_json(sse).expect("payload");
        assert_eq!(payload, r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
    }

    /// Progress notifications may precede the real reply, so the last payload wins.
    #[test]
    fn the_last_sse_payload_wins() {
        let sse = "data: {\"method\":\"progress\"}\n\ndata: {\"id\":1,\"result\":{}}\n\n";
        let payload = extract_json(sse).expect("payload");
        assert!(payload.contains("result"));
    }

    #[test]
    fn a_reply_with_no_json_is_none() {
        assert!(extract_json("event: ping\n\n").is_none());
        assert!(extract_json("").is_none());
        assert!(extract_json("not json at all").is_none());
    }

    #[test]
    fn a_server_records_its_configuration() {
        let server = HttpServer::new("remote", "https://mcp.example/api");
        assert_eq!(server.name(), "remote");
        assert_eq!(server.url(), "https://mcp.example/api");
    }
}
