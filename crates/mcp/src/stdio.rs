//! stdio transport.
//!
//! The server is a subprocess we launch, which makes it a **confinement target** and
//! not merely a source of untrusted content. A server binary is third-party code
//! running with our privileges unless something stops it, so it is spawned through the
//! sandbox and refused outright when confinement cannot be established.
//!
//! Messages are newline-delimited JSON on stdin/stdout. The server's stderr is left
//! attached to ours so its diagnostics stay visible.

use crate::protocol::{
    RpcNotification, RpcRequest, RpcResponse, ToolDescriptor, ToolList, ToolResult, call_params,
    initialize_params,
};
use crate::{McpError, McpResult};
use bravebot_core::event::Sink;
use bravebot_core::label::Label;
use bravebot_core::policy::Policy;
use bravebot_core::value::Labelled;
use bravebot_sandbox::Sandbox;
use bravebot_sandbox::policy::SandboxPolicy;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};

/// A server reached over stdin/stdout.
pub struct StdioServer {
    /// Kept so the child is killed when this is dropped.
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    name: String,
}

/// Shows the server's identity but nothing it has sent, so a log line cannot leak tool
/// output.
impl std::fmt::Debug for StdioServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdioServer")
            .field("name", &self.name)
            .field("pid", &self.child.id())
            .finish_non_exhaustive()
    }
}

impl Drop for StdioServer {
    fn drop(&mut self) {
        // A server that ignores a closed stdin would otherwise linger.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl StdioServer {
    /// Launch a server under confinement.
    ///
    /// `sandbox` must be a real backend. If confinement cannot be applied the server is
    /// not started: running unconfined third-party code would silently remove the
    /// guarantee the caller believes it has.
    pub fn launch(
        name: impl Into<String>,
        program: &str,
        args: &[String],
        sandbox: &dyn Sandbox,
        policy: &SandboxPolicy,
    ) -> McpResult<Self> {
        // The sandbox builds the command so stdio can be configured on the process that
        // actually runs; a wrapped command would lose these pipes.
        let mut child = sandbox
            .command(program, args, policy)
            .map_err(|e| McpError::Confinement(e.to_string()))?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr is inherited so server diagnostics reach the user.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| McpError::Transport(format!("could not start the server: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Transport("the server's stdin was not available".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Transport("the server's stdout was not available".into()))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            name: name.into(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn send_request(&mut self, method: &str, params: Option<Value>) -> McpResult<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let request = RpcRequest::new(id, method, params);
        let line = serde_json::to_string(&request)
            .map_err(|e| McpError::Transport(format!("could not encode {method}: {e}")))?;

        writeln!(self.stdin, "{line}")
            .and_then(|()| self.stdin.flush())
            .map_err(|e| McpError::Transport(format!("could not send {method}: {e}")))?;

        // Skip anything that is not the reply to this request: servers may interleave
        // notifications, and a stray line must not be mistaken for a result.
        loop {
            let mut line = String::new();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| McpError::Transport(format!("could not read a reply: {e}")))?;
            if read == 0 {
                return Err(McpError::Transport(
                    "the server closed its output before replying".into(),
                ));
            }
            if line.trim().is_empty() {
                continue;
            }

            let response: RpcResponse = match serde_json::from_str(&line) {
                Ok(r) => r,
                // Not JSON-RPC we understand; ignore rather than fail the call.
                Err(_) => continue,
            };

            if response.id != Some(id) {
                continue;
            }

            if let Some(error) = response.error {
                return Err(McpError::Server {
                    code: error.code,
                    message: error.message,
                });
            }

            return response.result.ok_or_else(|| {
                McpError::Transport(format!("{method} returned neither a result nor an error"))
            });
        }
    }

    fn notify(&mut self, method: &str) -> McpResult<()> {
        let notification = RpcNotification::new(method, None);
        let line =
            serde_json::to_string(&notification).map_err(|e| McpError::Transport(e.to_string()))?;
        writeln!(self.stdin, "{line}")
            .and_then(|()| self.stdin.flush())
            .map_err(|e| McpError::Transport(format!("could not send {method}: {e}")))
    }

    /// Complete the handshake.
    pub fn initialize(&mut self, client_name: &str, client_version: &str) -> McpResult<()> {
        self.send_request(
            "initialize",
            Some(initialize_params(client_name, client_version)),
        )?;
        self.notify("notifications/initialized")
    }

    /// List the tools this server offers.
    pub fn list_tools(&mut self) -> McpResult<Vec<ToolDescriptor>> {
        let result = self.send_request("tools/list", None)?;
        let list: ToolList = serde_json::from_value(result)
            .map_err(|e| McpError::Transport(format!("malformed tool list: {e}")))?;
        Ok(list.tools)
    }

    /// Call a tool.
    ///
    /// The result is labelled untrusted-public: it is third-party output, and this
    /// client does not know what the server read to produce it. A server handling the
    /// user's private data should be given a higher label by its configuration.
    pub fn call_tool<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        tool: &str,
        arguments: Value,
    ) -> McpResult<Labelled<String>> {
        policy
            .before_capability(bravebot_core::capability::Capability::McpCall)
            .map_err(McpError::Denied)?;

        let result = self.send_request("tools/call", Some(call_params(tool, arguments)))?;

        let parsed: ToolResult = serde_json::from_value(result)
            .map_err(|e| McpError::Transport(format!("malformed tool result: {e}")))?;

        if parsed.is_error {
            return Err(McpError::ToolFailed {
                tool: tool.to_string(),
                detail: parsed.text(),
            });
        }

        let label = policy
            .observe(bravebot_core::capability::Capability::McpCall)
            .map_err(McpError::Denied)?;

        Ok(Labelled::new(parsed.text(), label))
    }
}

/// The label MCP results carry.
pub fn result_label() -> Label {
    Label::untrusted_public()
}
