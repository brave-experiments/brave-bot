//! Model Context Protocol client.
//!
//! MCP is the extension boundary: task-specific tools arrive as servers rather than
//! being compiled in. Two transports, with different threat profiles.
//!
//! - **HTTP** — a remote server at a user-configured URL. Requests go through the
//!   egress chokepoint like any other network traffic.
//! - **stdio** — a local subprocess we launch, so it is also a *confinement* target,
//!   not merely a content source.
//!
//! Everything a server returns is untrusted content and is labelled as such. Results
//! are never parsed to decide what happens next.
//!
//! A small set of primitives stays native rather than moving behind MCP: the kernel
//! needs to label parts of a call separately — a file path as routing, its contents as
//! content — and an opaque MCP call would erase that distinction.

pub mod http;
pub mod protocol;
pub mod stdio;

pub use http::HttpServer;
pub use protocol::{ToolDescriptor, ToolResult};
pub use stdio::StdioServer;

use bua_core::policy::Denial;
use std::fmt;

pub type McpResult<T> = Result<T, McpError>;

#[derive(Debug)]
pub enum McpError {
    /// Confinement could not be established, so the server was not launched.
    Confinement(String),
    /// The policy refused the call.
    Denied(Denial),
    /// The transport failed, or the server sent something unusable.
    Transport(String),
    /// The server returned a JSON-RPC error.
    Server { code: i64, message: String },
    /// The tool ran and reported failure.
    ToolFailed { tool: String, detail: String },
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confinement(detail) => write!(
                f,
                "refusing to launch an mcp server without confinement: {detail}"
            ),
            Self::Denied(d) => write!(f, "{d}"),
            Self::Transport(detail) => write!(f, "mcp transport failed: {detail}"),
            Self::Server { code, message } => write!(f, "mcp server error {code}: {message}"),
            Self::ToolFailed { tool, detail } => write!(f, "tool '{tool}' failed: {detail}"),
        }
    }
}

impl std::error::Error for McpError {}
