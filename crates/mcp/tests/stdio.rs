//! stdio transport tests, driving a real subprocess.
//!
//! The fake server is a shell script speaking newline-delimited JSON-RPC. Using a real
//! process rather than an in-memory pipe is the point: it exercises the sandbox spawn
//! path, which is where a confinement failure would surface.

use bravebot_core::capability::{Capability, CapabilitySet};
use bravebot_core::event::RecordingSink;
use bravebot_core::label::Label;
use bravebot_core::policy::{Policy, ReleasePlan, Routing};
use bravebot_mcp::{McpError, StdioServer};
use bravebot_sandbox::policy::SandboxPolicy;
use bravebot_sandbox::{Sandbox, Unavailable};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// Serialises the tests in this binary, which all write a script and then execute it.
///
/// Linux refuses to execute a file any process still holds open for writing. These tests run on
/// threads of one process and each spawns a child, and a child forked while another thread is
/// part-way through writing its script inherits that write handle: the sibling's exec then fails
/// with `Text file busy`, on whichever test happened to lose the race. It failed in CI on a
/// commit that touched a prompt string, which is the tell.
///
/// One at a time closes the window, since no write is ever in flight while a fork happens.
static SPAWNING: Mutex<()> = Mutex::new(());

/// Hold this for the length of a test that spawns a server.
///
/// Poisoning is ignored on purpose: a test that panicked while holding it left nothing behind
/// that the next one cannot overwrite.
fn one_at_a_time() -> MutexGuard<'static, ()> {
    SPAWNING.lock().unwrap_or_else(|held| held.into_inner())
}

/// Write a fake MCP server that replies to each request by id.
fn fake_server(name: &str, script: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("bravebot-mcp-fake-{name}.sh"));
    let mut file = std::fs::File::create(&path).expect("create script");
    file.write_all(script.as_bytes()).expect("write script");
    // Closed before the mode is set, and well before anything tries to execute it.
    drop(file);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }

    path
}

/// A server that handles initialize, tools/list, and tools/call.
const WORKING_SERVER: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"fake","version":"1"}}}\n' "$id"
      ;;
    *'"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"echoes","inputSchema":{"type":"object"}}]}}\n' "$id"
      ;;
    *'"tools/call"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"tool output here"}]}}\n' "$id"
      ;;
    *'"notifications/initialized"'*)
      ;;
  esac
done
"#;

fn routing() -> Routing {
    let mut r = Routing::new();
    r.insert_trusted("task", "use a tool");
    r
}

/// A policy permissive enough for a shell script to run, while still confining it.
///
/// Network is granted, which is not what a real processor policy would do. The Linux
/// backend refuses a policy requiring network denial because that is not implemented
/// there yet, and refusing is the correct behaviour, so a test that wants a successful
/// spawn on both platforms has to ask for the weaker policy the backend can honour.
fn sandbox_policy() -> SandboxPolicy {
    SandboxPolicy::strict()
        .allow_network_egress()
        .allow_read("/usr")
        .allow_read("/bin")
        .allow_read("/lib")
        .allow_read("/lib64")
        // macOS puts the temporary directory under /private/var; Linux uses /tmp.
        .allow_read("/private/var/folders")
        .allow_read("/tmp")
        .allow_read("/var")
        .allow_subprocesses()
}

/// Skip where no real backend exists, since these tests need a spawn to succeed.
fn sandbox_or_skip() -> Option<Box<dyn Sandbox>> {
    match bravebot_sandbox::for_current_platform() {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("SKIPPED (no confinement backend): {e}");
            None
        }
    }
}

#[test]
fn a_confined_server_completes_the_handshake_and_lists_tools() {
    let _spawning = one_at_a_time();
    let Some(sandbox) = sandbox_or_skip() else {
        return;
    };
    let script = fake_server("handshake", WORKING_SERVER);

    let mut server = StdioServer::launch(
        "fake",
        script.to_str().expect("path"),
        &[],
        sandbox.as_ref(),
        &sandbox_policy(),
    )
    .expect("server launches under confinement");

    server.initialize("bravebot", "0.1.0").expect("handshake");

    let tools = server.list_tools().expect("tools listed");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
    assert!(tools[0].input_schema.is_some());

    let _ = std::fs::remove_file(&script);
}

/// A tool result is untrusted content, whatever the server says it is.
#[test]
fn a_tool_result_is_labelled_untrusted() {
    let _spawning = one_at_a_time();
    let Some(sandbox) = sandbox_or_skip() else {
        return;
    };
    let script = fake_server("call", WORKING_SERVER);

    let mut server = StdioServer::launch(
        "fake",
        script.to_str().expect("path"),
        &[],
        sandbox.as_ref(),
        &sandbox_policy(),
    )
    .expect("server launches");
    server.initialize("bravebot", "0.1.0").expect("handshake");

    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::McpCall]),
        &mut sink,
    )
    .expect("policy");

    let result = server
        .call_tool(&mut policy, "echo", serde_json::json!({"text": "hi"}))
        .expect("tool call succeeds");

    assert_eq!(result.label(), Label::untrusted_public());
    assert!(policy.finish());

    let _ = std::fs::remove_file(&script);
}

/// Without the capability the tool must not be callable.
#[test]
fn a_tool_call_without_the_capability_is_refused() {
    let _spawning = one_at_a_time();
    let Some(sandbox) = sandbox_or_skip() else {
        return;
    };
    let script = fake_server("no-capability", WORKING_SERVER);

    let mut server = StdioServer::launch(
        "fake",
        script.to_str().expect("path"),
        &[],
        sandbox.as_ref(),
        &sandbox_policy(),
    )
    .expect("server launches");
    server.initialize("bravebot", "0.1.0").expect("handshake");

    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::none(),
        &mut sink,
    )
    .expect("policy");

    let error = server
        .call_tool(&mut policy, "echo", serde_json::json!({}))
        .expect_err("must be refused");
    assert!(error.to_string().contains("mcp_call"), "got: {error}");

    let _ = std::fs::remove_file(&script);
}

/// The rule that matters most for stdio: no confinement means no server.
#[test]
fn a_server_is_not_launched_without_confinement() {
    let _spawning = one_at_a_time();
    let script = fake_server("unconfined", WORKING_SERVER);

    let error = StdioServer::launch(
        "fake",
        script.to_str().expect("path"),
        &[],
        &Unavailable,
        &SandboxPolicy::strict(),
    )
    .expect_err("must refuse to launch");

    assert!(matches!(error, McpError::Confinement(_)));
    assert!(
        error.to_string().contains("refusing to launch"),
        "got: {error}"
    );

    let _ = std::fs::remove_file(&script);
}

/// A JSON-RPC error must surface as an error rather than being read as a result.
#[test]
fn a_server_error_is_reported() {
    let _spawning = one_at_a_time();
    let Some(sandbox) = sandbox_or_skip() else {
        return;
    };
    let script = fake_server(
        "error",
        r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"capabilities":{}}}\n' "$id"
      ;;
    *'"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"not implemented"}}\n' "$id"
      ;;
  esac
done
"#,
    );

    let mut server = StdioServer::launch(
        "fake",
        script.to_str().expect("path"),
        &[],
        sandbox.as_ref(),
        &sandbox_policy(),
    )
    .expect("server launches");
    server.initialize("bravebot", "0.1.0").expect("handshake");

    let error = server.list_tools().expect_err("must report the error");
    assert!(
        matches!(error, McpError::Server { code: -32601, .. }),
        "got: {error}"
    );

    let _ = std::fs::remove_file(&script);
}

/// A server that dies must produce an error, not a hang.
#[test]
fn a_server_that_exits_early_is_an_error() {
    let _spawning = one_at_a_time();
    let Some(sandbox) = sandbox_or_skip() else {
        return;
    };
    let script = fake_server("exits", "#!/bin/sh\nexit 0\n");

    let mut server = StdioServer::launch(
        "fake",
        script.to_str().expect("path"),
        &[],
        sandbox.as_ref(),
        &sandbox_policy(),
    )
    .expect("launch succeeds even though the server exits");

    let error = server
        .initialize("bravebot", "0.1.0")
        .expect_err("a dead server cannot handshake");
    assert!(matches!(error, McpError::Transport(_)), "got: {error}");

    let _ = std::fs::remove_file(&script);
}
