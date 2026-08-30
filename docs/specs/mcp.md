---
id: MCP
title: Model Context Protocol servers
status: normative
governs:
  - crates/mcp/src/lib.rs
  - crates/mcp/src/stdio.rs
  - crates/mcp/src/http.rs
  - crates/mcp/src/protocol.rs
---

## Scope

Tools that come from outside this repository, over stdio or HTTP. What such a server returns, what
it is allowed to do, and why the tools bravebot ships are not built this way. Confinement of a
stdio server is [sandboxing.md](sandboxing.md).

## Why the built-in tools stay native

An MCP call is opaque: the whole call goes out and a result comes back. That erases the split
between the part of a call that decides where it lands and the part that is merely carried, which
the built-in tools depend on, since a path is one and a file's contents are the other. A primitive
that needs its parts labelled separately stays native rather than moving behind this boundary.

## Clauses

### MCP-1: what a server returns is untrusted

A tool result is content from outside, so it is labelled untrusted and quarantined like anything
else nobody vouched for. Nothing a server says about itself changes that.

`verified-by: bravebot_mcp::stdio::a_tool_result_is_labelled_untrusted`

### MCP-2: a call needs the capability, like any other effect

A tool call without the capability granted is refused, so adding a server does not widen what the
agent may do.

`verified-by: bravebot_mcp::stdio::a_tool_call_without_the_capability_is_refused`

### MCP-3: a stdio server is not launched without confinement

If the process cannot be confined it is not started, rather than started unconfined.

**Why.** A stdio server runs code we did not write, which is precisely the case an
operating-system boundary exists for.

`verified-by: bravebot_mcp::stdio::a_server_is_not_launched_without_confinement`
`verified-by: bravebot_mcp::stdio::a_confined_server_completes_the_handshake_and_lists_tools`

### MCP-4: bravebot advertises no capabilities of its own to a server

The handshake offers nothing, so a server cannot ask this process to do anything on its behalf.

`verified-by: bravebot_mcp::protocol::initialize_advertises_no_capabilities`
`verified-by: bravebot_mcp::protocol::call_params_carry_the_name_and_arguments`

### MCP-5: a failure is reported, never treated as an empty result

A server error, a tool-level error, and a server that exits early are all reported as failures. A
reply carrying no JSON is nothing rather than an empty success.

**Why.** An error read as "no results" would have the planner conclude a thing does not exist when
the truth is that nobody asked successfully.

`verified-by: bravebot_mcp::stdio::a_server_error_is_reported`
`verified-by: bravebot_mcp::stdio::a_server_that_exits_early_is_an_error`
`verified-by: bravebot_mcp::protocol::a_tool_level_error_is_visible`
`verified-by: bravebot_mcp::protocol::an_error_response_parses`
`verified-by: bravebot_mcp::http::a_reply_with_no_json_is_none`

### MCP-6: only text content is taken from a result

Non-text content is ignored rather than guessed at, and several text parts are joined.

`verified-by: bravebot_mcp::protocol::tool_result_text_is_joined`
`verified-by: bravebot_mcp::protocol::non_text_content_is_ignored`

### MCP-7: the transport is parsed strictly

A request carries the protocol version, a notification has no id, and an HTTP reply framed as
server-sent events is unwrapped with the last payload winning.

`verified-by: bravebot_mcp::protocol::a_request_carries_the_jsonrpc_version`
`verified-by: bravebot_mcp::protocol::a_notification_has_no_id`
`verified-by: bravebot_mcp::protocol::a_successful_response_parses`
`verified-by: bravebot_mcp::protocol::a_tool_list_parses_with_schemas`
`verified-by: bravebot_mcp::http::an_sse_framed_reply_is_unwrapped`
`verified-by: bravebot_mcp::http::the_last_sse_payload_wins`
`verified-by: bravebot_mcp::http::plain_json_is_extracted_as_is`
`verified-by: bravebot_mcp::http::whitespace_around_json_is_tolerated`
`verified-by: bravebot_mcp::http::a_server_records_its_configuration`
