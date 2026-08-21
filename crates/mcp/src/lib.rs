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

pub mod protocol;

pub use protocol::{ToolDescriptor, ToolResult};
