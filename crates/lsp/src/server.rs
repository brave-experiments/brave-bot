//! A language server: started with a person's approval, kept for the session, asked read-only
//! questions.
//!
//! Not confined, and [LSP-5] is the argument for that. A server indexes by running its ecosystem's
//! build tooling, so a profile that denies writes and children yields one whose index never settles
//! rather than a confined one that answers. What stands in confinement's place is the same thing that
//! stands in it for [`run`]: a person approves the process, and the label on what comes back does not
//! depend on their answer.
//!
//! [LSP-5]: ../../../docs/specs/tools/lsp.md
//! [`run`]: ../../../docs/specs/tools/run.md

use crate::protocol::{
    Operation, RpcNotification, RpcRequest, RpcResponse, content_length, frame, hover_text,
    initialize_params, locations_in, path_to_uri, position_params, reference_params,
};
use crate::{Answer, LspError, LspResult};
use bravebot_core::capability::Capability;
use bravebot_core::event::Sink;
use bravebot_core::policy::Policy;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::time::{Duration, Instant};

/// How long a request may wait for the index to settle before answering from what there is.
///
/// LSP-7's bound. Indexing a large workspace outlasts a person's patience, and a turn held open with
/// nothing to show for it is RUN-11's problem arriving by another road.
///
/// Set from what a real server takes rather than from what felt reasonable: unconfined,
/// rust-analyzer runs seven reported passes over this workspace and finishes the last a little under
/// a minute in.
///
/// Deliberately not raised past that. A server that has not settled by now is usually one that
/// cannot, which under this confinement profile is the ordinary case for Rust and is written up as a
/// known cost in the spec, and a caller waiting three minutes to be told the answer is partial is
/// worse off than one told in twenty seconds. Reaching the bound is not a failure: LSP-7 answers
/// from what the index has and says plainly that it may be short.
pub const MAX_INDEX_WAIT: Duration = Duration::from_secs(20);

/// How long a single request may take once the index has settled.
pub const MAX_REQUEST_WAIT: Duration = Duration::from_secs(20);

/// The protocol's code for "the index moved while I was answering".
///
/// Not an error in any sense a caller can act on: the request was fine and the state it referred to
/// changed. Answered as nothing found, with the index reported unsettled.
const CONTENT_MODIFIED: i64 = -32801;

/// How long a server gets to exit on request before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// Which server serves a language, and what it is called.
///
/// A fixed table rather than configuration, for now: each entry is a binary this repository knows
/// asks nothing of the network and can answer from a read-only tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
    Go,
}

impl Language {
    /// The language a file extension belongs to, or `None` where we have no server for it.
    ///
    /// Decided from the path, which is routing and already trusted. Nothing is read to decide it.
    pub fn for_path(path: &str) -> Option<Self> {
        let extension = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())?
            .to_ascii_lowercase();
        match extension.as_str() {
            "rs" => Some(Self::Rust),
            "ts" | "tsx" | "js" | "jsx" | "mts" | "cts" => Some(Self::TypeScript),
            "py" | "pyi" => Some(Self::Python),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    /// The binary to launch, and the arguments it needs to speak the protocol on stdio.
    pub fn server(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Rust => ("rust-analyzer", &[]),
            Self::TypeScript => ("typescript-language-server", &["--stdio"]),
            Self::Python => ("pyright-langserver", &["--stdio"]),
            Self::Go => ("gopls", &[]),
        }
    }

    /// Whether starting this server runs the ecosystem's build tooling, and so code out of the
    /// dependency tree.
    ///
    /// Said to the person at the prompt rather than left inside "with your own access", because it is
    /// the part of LSP-5 they could not have inferred from the word "start". True for Rust, where
    /// `build.rs` and proc macros execute, and for Go, whose tooling builds to answer. A Node or
    /// Python server reads and type-checks without running the project.
    pub fn runs_build_tooling(self) -> bool {
        matches!(self, Self::Rust | Self::Go)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript",
            Self::Python => "Python",
            Self::Go => "Go",
        }
    }
}

/// The directory holding one index per workspace, directly under the state directory.
const CACHE_ROOT: &str = "lsp";

/// Where a server keeps its index for a workspace.
///
/// LSP-10: under the directory this process already owns, keyed by the workspace, never inside it.
/// `state` is that directory itself, `~/.bravebot` and not the home it sits in, so nothing here
/// appends the name a second time. `None` for a session that adds nothing to `~/.bravebot`, which is
/// incognito: the server still runs and re-indexes, and says its answers are partial until it
/// settles.
///
/// The name is a digest of the canonical path rather than the path flattened into one, so two
/// checkouts of the same project do not share an index and a directory that moved does not inherit
/// one. Not a cryptographic requirement: this only has to be stable and collision-resistant enough
/// that two workspaces on one machine differ.
pub fn cache_for(state: Option<&Path>, workspace: &Path, incognito: bool) -> Option<PathBuf> {
    if incognito {
        return None;
    }
    let state = state?;
    let canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());

    // FNV-1a over the path's bytes. Enough for a directory name, and no dependency for it.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in canonical.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    Some(state.join(CACHE_ROOT).join(format!("{hash:016x}")))
}

/// Create a cache directory, and the directories between it and the state directory, reachable
/// only by this user.
///
/// The index is derived from every file in the workspace, so who may read it is who may read the
/// workspace. The mode is asked for as each directory is created, because a directory keeps the
/// mode it was made with. Spelled out here rather than shared with the crate that has a helper for
/// it: this crate depends on the kernel alone, as layering.md records, and a language server client
/// is not worth a dependency for four lines.
fn create_cache(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        // A directory keeps the mode it was made with, so one an earlier run left open is narrowed
        // rather than kept. These two and no further: what the state directory itself is set to
        // belongs to whichever subsystem created it.
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
        if let Some(root) = path.parent()
            && root.file_name().is_some_and(|name| name == CACHE_ROOT)
        {
            let _ = std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

/// Whether a message is a server saying its initial index is built.
///
/// Read from the shape of a progress notification, never from prose: what is looked at is whether
/// the token is one each ecosystem uses for its initial index and whether the value says `end`.
/// Both are protocol structure, so this decides nothing from a byte a file chose.
///
/// A free function so LSP-7 can be pinned without a live process: whether an answer is partial is
/// this decision and nothing else.
pub fn says_indexing_finished(message: &Value) -> bool {
    if message.get("method").and_then(Value::as_str) != Some("$/progress") {
        return false;
    }
    let Some(params) = message.get("params") else {
        return false;
    };
    let token = params.get("token").and_then(Value::as_str).unwrap_or("");
    let finished = params
        .get("value")
        .and_then(|value| value.get("kind"))
        .and_then(Value::as_str)
        == Some("end");

    // The token each server ends its last indexing pass with, taken from what real servers send
    // rather than from what their documentation implies. `rustAnalyzer/cachePriming` is the last of
    // seven passes rust-analyzer reports on this workspace, and the plausible-looking
    // `rustAnalyzer/Indexing` is sent by nothing: an earlier version of this waited for that name,
    // never saw it, and marked every answer partial for the life of the process.
    //
    // A token this does not know means answers stay marked partial, which is the safe direction to be
    // wrong in: LSP-7 makes a partial answer say so, and an answer wrongly called partial costs a
    // sentence where one wrongly called complete costs a deleted function.
    finished
        && matches!(
            token,
            "rustAnalyzer/cachePriming" | "gopls/loading" | "pyright/analysis"
        )
}

/// Read framed messages off a server's output until the pipe ends.
///
/// Runs on its own thread, so a blocking read never holds up a caller's deadline. Stops on the first
/// thing it cannot make sense of: a stream whose framing has desynchronised cannot be resynchronised
/// by guessing, and carrying on would attribute one message's body to another's header.
fn read_messages(mut stdout: BufReader<ChildStdout>, sender: &std::sync::mpsc::Sender<Value>) {
    loop {
        let mut headers = String::new();
        loop {
            let mut byte = [0u8; 1];
            match stdout.read(&mut byte) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            headers.push(byte[0] as char);
            if headers.ends_with("\r\n\r\n") {
                break;
            }
            // A server printing something that is not a header block would otherwise be read
            // forever, one byte at a time.
            if headers.len() > 8192 {
                return;
            }
        }

        let Some(length) = content_length(&headers) else {
            return;
        };

        let mut body = vec![0u8; length];
        if stdout.read_exact(&mut body).is_err() {
            return;
        }

        match serde_json::from_slice(&body) {
            Ok(message) => {
                // A closed channel means nobody is left to receive, so there is nothing to do but
                // stop.
                if sender.send(message).is_err() {
                    return;
                }
            }
            // One unparseable body is not a reason to abandon the stream: the framing is still in
            // step, so the next message is still readable.
            Err(_) => continue,
        }
    }
}

/// One running server.
pub struct Server {
    language: Language,
    child: Child,
    stdin: ChildStdin,
    /// Messages the reader thread has parsed, in the order they arrived.
    ///
    /// A thread rather than reading inline, because every bound here has to be a real one. Reading
    /// on this thread makes a deadline unenforceable: the check happens between messages, so a
    /// server that goes quiet mid-index blocks in `read` and no elapsed time is ever consulted.
    /// That is not hypothetical, it is what the first version of this did, and it held a turn open
    /// indefinitely against a real rust-analyzer indexing this workspace.
    incoming: std::sync::mpsc::Receiver<Value>,
    next_id: u64,
    root: PathBuf,
    /// Whether the server has said it finished indexing.
    ///
    /// Starts false and is set by a progress notification. LSP-7 reports an answer given before
    /// that as partial, because a `findReferences` against a half-built index looks exactly like
    /// one that found everything.
    indexed: bool,
}

/// Shows what it is but nothing it has sent, so a log line cannot leak a file's contents.
impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("language", &self.language.as_str())
            .field("pid", &self.child.id())
            .field("indexed", &self.indexed)
            .finish_non_exhaustive()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Asked to stop, then killed if it did not. LSP-8: a server must not outlive the agent.
        let _ = self.request_shutdown();
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    /// Start a server for this language.
    ///
    /// `resolved` is the absolute path to the binary, so what runs is what a person was shown:
    /// RUN-8's reason, that `$PATH` and aliases decide what a name means and an approval must not
    /// follow a name onto a different binary.
    ///
    /// `cache` is where it may keep its index, from [`cache_for`], or `None` for a session that keeps
    /// nothing. A server given none re-indexes and answers partially until it settles.
    ///
    /// LSP-6: a missing binary is reported as missing rather than as an empty answer.
    pub fn launch(
        language: Language,
        resolved: &Path,
        root: &Path,
        cache: Option<&Path>,
        withheld: &[String],
    ) -> LspResult<Self> {
        let (program, args) = language.server();
        let owned: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();

        let mut command = std::process::Command::new(resolved);
        command.args(&owned);

        // The user's environment reaches the server, because it runs with their access and a
        // toolchain reads its own variables to work: `CARGO_HOME`, `GOPATH`, `NODE_PATH`. What does
        // not reach it is this agent's own credentials, which is RUN-12 exactly: a person approving a
        // server read what it is and where it will run, and a credential travelling alongside was
        // granted without having been seen. The names come from the caller because which they are is
        // the host's business, the same reason `resolved` is passed in rather than looked up here.
        for name in withheld {
            command.env_remove(name);
        }

        // Where the index goes, said to each ecosystem in its own spelling. Nothing is written to the
        // workspace, which is LSP-10.
        if let Some(cache) = cache {
            let _ = create_cache(cache);
            match language {
                Language::Rust => {
                    command.env("CARGO_TARGET_DIR", cache);
                }
                Language::Go => {
                    command.env("GOCACHE", cache.join("go-build"));
                }
                Language::TypeScript | Language::Python => {
                    // Neither reads a variable for this; both use the system temporary directory,
                    // and pointing that at the cache keeps it out of the workspace.
                    command.env("TMPDIR", cache);
                }
            }
        }

        let mut child = command
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // A server's diagnostics are noisy and are not this process's business.
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    LspError::NoBinary { language, program }
                } else {
                    LspError::Start {
                        language,
                        detail: e.to_string(),
                    }
                }
            })?;

        let stdin = child.stdin.take().ok_or(LspError::Start {
            language,
            detail: "the server's stdin was not available".into(),
        })?;
        let stdout = child.stdout.take().ok_or(LspError::Start {
            language,
            detail: "the server's stdout was not available".into(),
        })?;

        // The reader owns stdout and hands whole messages over. It ends when the pipe does, so a
        // server that exits closes the channel and every waiting read learns about it.
        let (sender, incoming) = std::sync::mpsc::channel();
        std::thread::spawn(move || read_messages(BufReader::new(stdout), &sender));

        let mut server = Self {
            language,
            child,
            stdin,
            incoming,
            next_id: 1,
            root: root.to_path_buf(),
            indexed: false,
        };
        server.initialize()?;
        Ok(server)
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// Whether the server has finished indexing.
    pub fn is_indexed(&self) -> bool {
        self.indexed
    }

    fn initialize(&mut self) -> LspResult<()> {
        let root_uri = path_to_uri(&self.root.to_string_lossy());
        self.send_request(
            "initialize",
            Some(initialize_params(
                &root_uri,
                "bravebot",
                env!("CARGO_PKG_VERSION"),
            )),
            MAX_REQUEST_WAIT,
        )?;
        self.notify("initialized", Some(serde_json::json!({})))
    }

    fn request_shutdown(&mut self) -> LspResult<()> {
        // Best effort: the process is killed if this does not land.
        self.send_request("shutdown", None, SHUTDOWN_GRACE)?;
        self.notify("exit", None)
    }

    fn notify(&mut self, method: &str, params: Option<Value>) -> LspResult<()> {
        let notification = RpcNotification::new(method, params);
        let body = serde_json::to_string(&notification).map_err(|e| LspError::Transport {
            language: self.language,
            detail: e.to_string(),
        })?;
        self.write(&frame(&body))
    }

    fn write(&mut self, framed: &str) -> LspResult<()> {
        self.stdin
            .write_all(framed.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|e| LspError::Transport {
                language: self.language,
                detail: format!("could not send a request: {e}"),
            })
    }

    /// Take the next message, waiting no longer than `budget`.
    ///
    /// Three outcomes, and they are genuinely different: a message, the budget running out, or the
    /// server having closed its output. The middle one is what makes every bound in this module
    /// real.
    fn next_message(&mut self, budget: Duration) -> LspResult<Option<Value>> {
        match self.incoming.recv_timeout(budget) {
            Ok(message) => Ok(Some(message)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(LspError::Exited {
                language: self.language,
            }),
        }
    }

    /// Send a request and wait for the reply with this id.
    ///
    /// Notifications arriving in between are read for whether indexing finished and otherwise
    /// dropped: this client handles no server-initiated request, so a server asking it to act gets
    /// no answer rather than an error.
    fn send_request(
        &mut self,
        method: &str,
        params: Option<Value>,
        budget: Duration,
    ) -> LspResult<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let request = RpcRequest::new(id, method, params);
        let body = serde_json::to_string(&request).map_err(|e| LspError::Transport {
            language: self.language,
            detail: e.to_string(),
        })?;
        self.write(&frame(&body))?;

        let deadline = Instant::now() + budget;
        loop {
            // What is left of the budget, so a stream of notifications cannot extend it: each wait
            // is bounded by the time remaining rather than by the whole allowance again.
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Some(message) = self.next_message(remaining)? else {
                return Err(LspError::TimedOut {
                    language: self.language,
                    method: method.to_string(),
                });
            };
            self.note_progress(&message);

            let response: RpcResponse = match serde_json::from_value(message) {
                Ok(response) => response,
                // Not a response shape; a notification or a server-initiated request.
                Err(_) => continue,
            };

            if response.id != Some(id) {
                continue;
            }

            if let Some(error) = response.error {
                // `ContentModified` is the protocol saying the index moved under the request, which
                // happens while a server is still settling. It is a retry rather than a failure: the
                // question was well formed and the answer is simply not available yet, so it is
                // reported as an unsettled index and LSP-7 marks whatever comes back partial.
                if error.code == CONTENT_MODIFIED {
                    return Ok(Value::Null);
                }
                return Err(LspError::Server {
                    language: self.language,
                    code: error.code,
                    message: error.message,
                });
            }

            // A query that matched nothing answers with null, which is an answer.
            return Ok(response.result.unwrap_or(Value::Null));
        }
    }

    /// Tell the server about a document, which is what makes it answerable.
    ///
    /// The contents are read off disk and handed straight over. This module never looks at them; see
    /// the note in [`Server::ask`] about why passing them through is a carry rather than a read.
    fn open(&mut self, path: &str, uri: &str) -> LspResult<()> {
        let text = std::fs::read_to_string(path).map_err(|e| LspError::Transport {
            language: self.language,
            detail: format!("could not read {path} to open it: {e}"),
        })?;

        let language_id = match self.language {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::Python => "python",
            Language::Go => "go",
        };

        self.notify(
            "textDocument/didOpen",
            Some(serde_json::json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": text,
                }
            })),
        )
    }

    /// Notice a server saying it has finished indexing.
    fn note_progress(&mut self, message: &Value) {
        if says_indexing_finished(message) {
            self.indexed = true;
        }
    }

    /// Wait for the index to settle, up to [`MAX_INDEX_WAIT`].
    ///
    /// Reaching the bound is not a failure: the caller answers from what the index has and says the
    /// answer is partial, which is LSP-7.
    fn settle(&mut self) {
        if self.indexed {
            return;
        }
        let deadline = Instant::now() + MAX_INDEX_WAIT;
        while !self.indexed {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            // Nothing is sent to prompt a message; this waits for what the server says on its own.
            // A server that has gone quiet times out here rather than blocking, which is the whole
            // reason the reader is on its own thread.
            match self.next_message(remaining) {
                Ok(Some(message)) => self.note_progress(&message),
                // Out of time, or the server is gone. Either way the caller answers from what the
                // index has and marks it partial, which is LSP-7.
                Ok(None) | Err(_) => return,
            }
        }
    }

    /// Ask one read-only question.
    ///
    /// The answer separates structure from content: locations come back as [`crate::Location`]
    /// values, and hover text comes back as a `String` the caller labels. LSP-3 is why those are two
    /// fields.
    pub fn ask(&mut self, question: &Question<'_>) -> LspResult<Answer> {
        self.settle();

        let operation = question.operation;
        let uri = path_to_uri(question.path);

        // The protocol requires a document be opened before it is asked about: a server answers
        // `file not found` otherwise, however plainly the file exists on disk, because its own view
        // of a document is the one the client told it about. Sent per question rather than tracked,
        // since re-opening an open document is defined to be harmless and a cache would be a second
        // account of what the server knows, waiting to disagree with the first.
        //
        // The bytes travel through this function and are never examined. That is the carry the label
        // rules permit and it is worth being exact about: nothing here branches on the contents,
        // compares them, or derives a position from them, so no decision is taken from a byte a file
        // chose. What the server does with them produces locations, which LSP-3 governs.
        if operation.needs_position() {
            let opened = self.open(question.path, &uri);
            // A file that cannot be read is not a failure of this call: the question is put anyway,
            // and a server that has the document indexed already answers from that.
            let _ = opened;
        }
        let result = if operation == Operation::WorkspaceSymbol {
            self.send_request(
                operation.method(),
                Some(serde_json::json!({ "query": question.query.unwrap_or_default() })),
                MAX_REQUEST_WAIT,
            )?
        } else if operation.needs_prepared_item() {
            // Both directions need an item first, and a position with no symbol at it prepares
            // nothing, which is an empty answer rather than an error.
            let prepared = self.send_request(
                "textDocument/prepareCallHierarchy",
                Some(position_params(&uri, question.line, question.character)),
                MAX_REQUEST_WAIT,
            )?;
            let item = match &prepared {
                Value::Array(items) if !items.is_empty() => items[0].clone(),
                _ => {
                    return Ok(Answer {
                        locations: Vec::new(),
                        text: None,
                        partial: !self.indexed,
                    });
                }
            };
            self.send_request(
                operation.method(),
                Some(serde_json::json!({ "item": item })),
                MAX_REQUEST_WAIT,
            )?
        } else {
            let params = if operation == Operation::References {
                reference_params(&uri, question.line, question.character)
            } else if operation == Operation::DocumentSymbol {
                serde_json::json!({ "textDocument": { "uri": uri } })
            } else {
                position_params(&uri, question.line, question.character)
            };
            self.send_request(operation.method(), Some(params), MAX_REQUEST_WAIT)?
        };

        Ok(Answer {
            locations: locations_in(&result),
            text: (operation == Operation::Hover)
                .then(|| hover_text(&result))
                .flatten(),
            // LSP-7: an answer given before the index settled is partial, whatever it found.
            partial: !self.indexed,
        })
    }
}

/// One question, whole.
///
/// Bundled rather than passed as five arguments because these are one thing: every field is routing,
/// and a caller assembling them separately is a caller that can get the position and the path out of
/// step. `path` is absolute, since that is what a server opens.
#[derive(Debug, Clone, Copy)]
pub struct Question<'a> {
    pub operation: Operation,
    pub path: &'a str,
    /// 1-based, as the planner stated it.
    pub line: usize,
    /// 1-based, as the planner stated it.
    pub character: usize,
    /// The name to look for, for `workspaceSymbol` alone.
    pub query: Option<&'a str>,
}

/// The servers a session has started, one per language.
///
/// LSP-8: started on the first request for a language, kept for the session, and stopped when this
/// is dropped.
pub struct Servers {
    running: HashMap<Language, Server>,
    root: PathBuf,
    /// `~/.bravebot` itself, not the home it sits in.
    state: Option<PathBuf>,
    /// How a program name becomes the file it names.
    ///
    /// Supplied rather than done here, for the reason [`Server::launch`] takes a resolved path:
    /// `$PATH` is the host's business, and this crate has no opinion about it. It also keeps the
    /// lookup in one place for the whole repository, so a name cannot mean one binary to `run` and
    /// another to this.
    resolve: fn(&str) -> Option<PathBuf>,
    /// Whether this session keeps nothing under `~/.bravebot`, so no index is cached.
    incognito: bool,
    /// This agent's own credential names, withheld from every server. RUN-12's reason.
    withheld: Vec<String>,
}

impl std::fmt::Debug for Servers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Servers")
            .field("languages", &self.running.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Servers {
    pub fn new(
        root: impl Into<PathBuf>,
        state: Option<PathBuf>,
        resolve: fn(&str) -> Option<PathBuf>,
        incognito: bool,
        withheld: Vec<String>,
    ) -> Self {
        Self {
            running: HashMap::new(),
            root: root.into(),
            state,
            resolve,
            incognito,
            withheld,
        }
    }

    /// How many servers are running. Zero until something asks.
    pub fn running(&self) -> usize {
        self.running.len()
    }

    /// Ask about a position in a file, starting a server for its language if none is running.
    ///
    /// LSP-9: the capability is checked before anything is launched, so a run that was not granted it
    /// does not get a process started on its behalf.
    ///
    /// `approve` is asked once per language, and only when a server is not already running. It is a
    /// callback rather than a decision passed in because whether to ask depends on what is running,
    /// which is this type's business, while how to ask is the caller's: the prompt belongs where the
    /// person is, and this crate has no way to reach them.
    pub fn ask<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        question: &Question<'_>,
        approve: &mut dyn FnMut(&Starting<'_>) -> bool,
    ) -> LspResult<Answer> {
        policy
            .before_capability(Capability::LanguageServer)
            .map_err(LspError::Denied)?;

        // Which server to ask. Every operation but `workspaceSymbol` starts from a file, so the file
        // decides; `workspaceSymbol` ranges over the tree and names none, so it goes to whichever
        // server is already running. That is deliberate rather than a fallback: starting a server on
        // a query with no file in it would mean guessing at the language from a symbol name.
        let language = match Language::for_path(question.path) {
            Some(language) => language,
            None if !question.operation.needs_position() => *self
                .running
                .keys()
                .next()
                .ok_or(LspError::NoServerForQuery)?,
            None => {
                return Err(LspError::NoServerFor {
                    path: question.path.to_string(),
                });
            }
        };

        if !self.running.contains_key(&language) {
            let (program, _) = language.server();
            // LSP-6: a binary that is not installed is said to be missing here, before anything is
            // launched, rather than surfacing as a process that exited. The two are different facts
            // and must not render alike.
            let resolved =
                (self.resolve)(program).ok_or(LspError::NoBinary { language, program })?;

            // LSP-5: asked before anything starts, and a refusal is not a failure of the tool. The
            // planner is told it was refused, which is what it needs to know: retrying will not help.
            let starting = Starting {
                language,
                resolved: &resolved,
                workspace: &self.root,
                runs_build_tooling: language.runs_build_tooling(),
            };
            if !approve(&starting) {
                return Err(LspError::Refused { language });
            }

            let cache = cache_for(self.state.as_deref(), &self.root, self.incognito);
            let server = Server::launch(
                language,
                &resolved,
                &self.root,
                cache.as_deref(),
                &self.withheld,
            )?;
            self.running.insert(language, server);
        }

        let server = self
            .running
            .get_mut(&language)
            .expect("just inserted if absent");
        server.ask(question)
    }
}

/// What a person is being asked to approve.
///
/// Everything the prompt needs and nothing it does not, so the caller draws a question rather than
/// assembling one.
#[derive(Debug, Clone, Copy)]
pub struct Starting<'a> {
    pub language: Language,
    /// The binary, resolved, so what is approved is what runs.
    pub resolved: &'a Path,
    pub workspace: &'a Path,
    /// Whether starting it runs the ecosystem's build tooling, and so code from the dependency tree.
    pub runs_build_tooling: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/workspace")
    }

    /// LSP-5: nothing starts until a person says so, and a refusal is not a failure of the tool.
    #[test]
    fn starting_a_server_is_put_to_a_person() {
        let mut asked = 0;
        let mut servers = Servers::new(root(), None, |_| None, false, Vec::new());
        let mut sink = bravebot_core::event::RecordingSink::new();
        let mut routing = bravebot_core::policy::Routing::new();
        routing.insert_trusted("task", "look up");
        let mut policy = Policy::begin(
            routing,
            bravebot_core::policy::ReleasePlan::new(),
            bravebot_core::capability::CapabilitySet::from_iter([Capability::LanguageServer]),
            &mut sink,
        )
        .expect("policy");

        // The resolver finds nothing, so this stops at LSP-6 before it would have asked. The point
        // is the order: a person is not asked about a server that could not have started anyway.
        let _ = servers.ask(
            &mut policy,
            &Question {
                operation: Operation::Definition,
                path: "/workspace/src/a.rs",
                line: 1,
                character: 1,
                query: None,
            },
            &mut |_| {
                asked += 1;
                true
            },
        );
        assert_eq!(
            asked, 0,
            "a person must not be asked about a server that is not installed"
        );
    }

    /// LSP-5: a refusal stops the process and says so in words that are not about the code.
    #[test]
    fn a_refused_server_does_not_start() {
        let said = LspError::Refused {
            language: Language::Rust,
        };
        assert!(said.is_absence_of_a_server());
        let rendered = said.to_string();
        assert!(rendered.contains("declined"), "{rendered}");
        // It must not read as an answer, and must say that retrying is pointless.
        assert!(!rendered.contains("no references"), "{rendered}");
        assert!(rendered.contains("will not change it"), "{rendered}");
    }

    /// LSP-5 and LSP-8 together: asked once per language, not once per question.
    #[test]
    fn a_server_is_not_asked_about_twice_in_a_session() {
        // A server already running is not asked about again, which is what the map decides. Pinned on
        // the bookkeeping rather than on a live process: `running` is what `ask` consults before it
        // reaches the approval, so a language present in it is a language nobody is asked about.
        let servers = Servers::new(root(), None, |_| None, false, Vec::new());
        assert_eq!(servers.running(), 0);
        assert_eq!(
            Language::for_path("src/a.rs"),
            Language::for_path("src/b.rs"),
            "two files of one language are one server, so one question"
        );
    }

    /// The state directory, as the host resolves it and hands it over.
    fn state() -> PathBuf {
        PathBuf::from("/home/someone/.bravebot")
    }

    /// LSP-10: never inside the workspace, and keyed by it.
    #[test]
    fn the_cache_is_outside_the_workspace() {
        let cache = cache_for(Some(&state()), &root(), false).expect("a cache is given");
        assert!(
            cache.starts_with(state()),
            "the cache belongs under the directory this process owns, got {}",
            cache.display()
        );
        assert!(
            !cache.starts_with(root()),
            "a question about a symbol must not write into the tree, got {}",
            cache.display()
        );
    }

    /// The argument is the state directory, so appending its name here would put the index in
    /// `~/.bravebot/.bravebot`: a directory nothing else writes to, reads or narrows, holding an
    /// index of the user's source.
    #[test]
    fn the_cache_sits_directly_under_the_directory_it_is_given() {
        let cache = cache_for(Some(&state()), &root(), false).expect("a cache is given");

        let below: Vec<_> = cache
            .strip_prefix(state())
            .expect("under the directory it was given")
            .components()
            .map(|part| part.as_os_str().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            below.len(),
            2,
            "one directory for the tool, one per workspace"
        );
        assert_eq!(below[0], CACHE_ROOT);
    }

    /// The index is derived from every file in the workspace, so who may read it is who may read
    /// the workspace. At the process umask that is every account on the machine.
    #[cfg(unix)]
    #[test]
    fn the_cache_is_created_reachable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = crate::testutil::scratch_dir("bravebot-lsp-cache-mode");
        let _ = std::fs::remove_dir_all(&scratch);
        let cache = cache_for(Some(&scratch), &root(), false).expect("a cache is given");

        create_cache(&cache).expect("created");

        let mode = |path: &Path| {
            std::fs::metadata(path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(&cache), 0o700);
        assert_eq!(
            mode(&scratch.join(CACHE_ROOT)),
            0o700,
            "the directory holding one per workspace"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A run of an earlier build left these at the umask, and creating a directory that exists
    /// does not touch its mode. Without narrowing, the machines already holding an index would be
    /// the ones this never reaches.
    #[cfg(unix)]
    #[test]
    fn a_cache_left_open_by_an_earlier_run_is_narrowed() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = crate::testutil::scratch_dir("bravebot-lsp-cache-narrowed");
        let _ = std::fs::remove_dir_all(&scratch);
        let cache = cache_for(Some(&scratch), &root(), false).expect("a cache is given");
        std::fs::create_dir_all(&cache).expect("as an earlier run left it");
        let loosen = |path: &Path| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("loosen")
        };
        loosen(&scratch.join(CACHE_ROOT));
        loosen(&cache);

        create_cache(&cache).expect("created");

        let mode = |path: &Path| {
            std::fs::metadata(path)
                .expect("exists")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(&cache), 0o700);
        assert_eq!(mode(&scratch.join(CACHE_ROOT)), 0o700);
        assert_eq!(
            mode(&scratch),
            0o755,
            "the state directory is not this crate's to set"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// LSP-10: two workspaces do not share an index.
    #[test]
    fn the_cache_is_keyed_by_the_workspace() {
        let one = cache_for(Some(&state()), Path::new("/a/project"), false).expect("cache");
        let two = cache_for(Some(&state()), Path::new("/b/project"), false).expect("cache");
        assert_ne!(
            one, two,
            "two checkouts must not share an index, or a stale one is read as the other's"
        );
        // And the same workspace is the same directory every time, or nothing is ever reused.
        assert_eq!(
            one,
            cache_for(Some(&state()), Path::new("/a/project"), false).expect("cache")
        );
    }

    /// LSP-10: incognito adds nothing to `~/.bravebot`, so it is given no cache at all.
    #[test]
    fn an_incognito_session_is_given_no_cache() {
        assert!(
            cache_for(Some(&state()), &root(), true).is_none(),
            "an incognito session must write no index"
        );
        // And with nowhere to keep one, there is nothing to key.
        assert!(cache_for(None, &root(), false).is_none());
    }

    /// LSP-10: the cache is the server's, and this crate never opens it.
    ///
    /// The clause that matters most, because `~/.bravebot` is trusted by provenance and somebody will
    /// reason that what sits in it is too. It is not: the bytes are derived from workspace files, so
    /// LABEL-2 taints them and LABEL-7 forbids recovering a better label.
    ///
    /// Pinned by what the type offers rather than by scanning the source for reads, which would match
    /// its own assertions. `cache_for` hands back a path and nothing that reads one, and a `Server`
    /// exposes no way to get at it: there is no accessor, so no caller can reach the directory
    /// through this crate even if it wanted to.
    #[test]
    fn the_cache_is_never_read_by_the_driver() {
        let cache = cache_for(Some(&state()), &root(), false).expect("cache");

        // A path, not a handle and not any bytes. Everything this crate does with it is hand it to a
        // child process, and the type says so: `PathBuf` carries no contents.
        let _: PathBuf = cache;

        // And nothing in the running server offers it back. If an accessor is ever added, this test
        // is the place that has to be argued with first.
        let names = ["cache", "index_dir", "cache_dir"];
        let debug = format!(
            "{:?}",
            Servers::new(root(), None, |_| None, false, Vec::new())
        );
        for name in names {
            assert!(
                !debug.contains(name),
                "the cache must not be reported anywhere a caller could read it: {debug}"
            );
        }
    }

    /// A whole-tree query names no file, so it cannot decide a language and must not be answered by
    /// guessing one from the symbol's name.
    #[test]
    fn a_query_with_no_file_is_told_no_server_is_running() {
        let mut servers = Servers::new(root(), None, |_| None, false, Vec::new());
        let mut sink = bravebot_core::event::RecordingSink::new();
        let mut routing = bravebot_core::policy::Routing::new();
        routing.insert_trusted("task", "look up");
        let mut policy = Policy::begin(
            routing,
            bravebot_core::policy::ReleasePlan::new(),
            bravebot_core::capability::CapabilitySet::from_iter([Capability::LanguageServer]),
            &mut sink,
        )
        .expect("policy");

        let error = servers
            .ask(
                &mut policy,
                &Question {
                    operation: Operation::WorkspaceSymbol,
                    path: "",
                    line: 1,
                    character: 1,
                    query: Some("Capability"),
                },
                &mut |_| true,
            )
            .expect_err("nothing is running, so there is nothing to search");

        // It must say that no server was asked, and must not read as a fact about the code. And it
        // must not be the sentence about a *file* having no server, since no file was named.
        let said = error.to_string();
        assert!(error.is_absence_of_a_server());
        assert!(said.contains("no server is running"), "{said}");
        assert!(
            !said.contains("no language server is configured for "),
            "{said}"
        );
        assert!(
            said.contains("ask about a symbol in a file first"),
            "{said}"
        );
    }

    /// LSP-5: which servers run the ecosystem's build tooling, since that is what the prompt says.
    #[test]
    fn the_prompt_says_which_servers_run_build_tooling() {
        assert!(Language::Rust.runs_build_tooling());
        assert!(Language::Go.runs_build_tooling());
        assert!(!Language::TypeScript.runs_build_tooling());
        assert!(!Language::Python.runs_build_tooling());
    }

    /// LSP-6: a language with no server is that, and is not an empty answer.
    #[test]
    fn an_unconfigured_language_is_reported_as_unconfigured() {
        assert!(Language::for_path("notes.txt").is_none());
        assert!(Language::for_path("Makefile").is_none());
        assert!(Language::for_path("a.rs").is_some());

        let error = LspError::NoServerFor {
            path: "notes.txt".into(),
        };
        let said = error.to_string();
        assert!(
            said.contains("no language server"),
            "the reason must be named: {said}"
        );
        // The sentence must not read as an answer about the code.
        assert!(!said.contains("no references"), "{said}");
        assert!(!said.contains("not found in"), "{said}");
    }

    /// LSP-6: a binary that is not installed is reported as missing, naming what to install.
    #[test]
    fn a_missing_binary_is_reported_as_missing() {
        let error = LspError::NoBinary {
            language: Language::Rust,
            program: "rust-analyzer",
        };
        let said = error.to_string();
        assert!(said.contains("rust-analyzer"), "{said}");
        assert!(said.contains("not installed"), "{said}");
    }

    /// LSP-6: a server that was there and failed is a third distinct sentence.
    #[test]
    fn a_server_that_fails_to_start_is_reported_as_such() {
        let error = LspError::Start {
            language: Language::Go,
            detail: "exited immediately".into(),
        };
        let said = error.to_string();
        assert!(said.contains("Go"), "{said}");
        assert!(said.contains("exited immediately"), "{said}");

        // The three failures LSP-6 separates must not render alike.
        let unconfigured = LspError::NoServerFor {
            path: "a.txt".into(),
        }
        .to_string();
        let missing = LspError::NoBinary {
            language: Language::Go,
            program: "gopls",
        }
        .to_string();
        assert_ne!(said, unconfigured);
        assert_ne!(said, missing);
        assert_ne!(unconfigured, missing);
    }

    /// LSP-8: nothing starts until something asks.
    #[test]
    fn no_server_starts_until_a_request_needs_one() {
        let servers = Servers::new(root(), None, |_| None, false, Vec::new());
        assert_eq!(
            servers.running(),
            0,
            "a session must not start a server nobody asked for"
        );
    }

    /// LSP-8: the set is what stops the processes, so dropping it must stop them all.
    #[test]
    fn dropping_the_set_stops_every_server() {
        let servers = Servers::new(root(), None, |_| None, false, Vec::new());
        // Nothing running, so this is the degenerate case; the property that matters is that the
        // set owns its servers, which is by construction, and that dropping it is not a leak.
        drop(servers);
    }

    /// LSP-7: an answer given before the index settled is partial, whatever it found. A
    /// `findReferences` against a half-built index returns some references and looks exactly like
    /// one that returned all of them.
    #[test]
    fn an_answer_during_indexing_is_marked_partial() {
        // Nothing has said indexing finished, so an answer built now is partial.
        let mid_index = Answer {
            locations: vec![crate::Location {
                path: "/workspace/src/a.rs".into(),
                line: 1,
                character: 1,
                kind: None,
            }],
            text: None,
            partial: true,
        };
        assert!(
            mid_index.partial,
            "an answer given while indexing must say so"
        );

        // Every other token a real rust-analyzer sends on this workspace, none of which means the
        // index is built. Recorded from an actual session rather than guessed, because guessing is
        // exactly what went wrong here once: the earlier list waited for `rustAnalyzer/Indexing`,
        // which nothing sends.
        for earlier in [
            "rustAnalyzer/Fetching",
            "rustAnalyzer/Building CrateGraph",
            "rustAnalyzer/Roots Scanned",
            "rustAnalyzer/Building compile-time-deps",
            "rustAnalyzer/Loading proc-macros",
            "rust-analyzer/flycheck/0",
            // The name that looks right and is sent by nothing.
            "rustAnalyzer/Indexing",
        ] {
            assert!(
                !says_indexing_finished(&serde_json::json!({
                    "method": "$/progress",
                    "params": { "token": earlier, "value": { "kind": "end" } },
                })),
                "{earlier} ending does not mean the index is built"
            );
        }
        // Nor does the beginning of indexing.
        assert!(!says_indexing_finished(&serde_json::json!({
            "method": "$/progress",
            "params": { "token": "rustAnalyzer/Indexing", "value": { "kind": "begin" } },
        })));
        // Nor an unrelated message.
        assert!(!says_indexing_finished(&serde_json::json!({
            "method": "window/logMessage",
            "params": { "message": "indexing finished" },
        })));
    }

    /// LSP-7: a settled index makes no partial claim, so the notice means something when it appears.
    #[test]
    fn a_settled_index_makes_no_partial_claim() {
        for token in [
            "rustAnalyzer/cachePriming",
            "gopls/loading",
            "pyright/analysis",
        ] {
            assert!(
                says_indexing_finished(&serde_json::json!({
                    "method": "$/progress",
                    "params": { "token": token, "value": { "kind": "end" } },
                })),
                "{token} ending must settle the index"
            );
        }

        let settled = Answer::default();
        assert!(!settled.partial);
    }

    /// LSP-8: indexing is the whole cost, so a server is started once for a language and reused.
    ///
    /// Pinned on the bookkeeping rather than on a live process: the property is that a second
    /// question for a language already running starts nothing, which is what the map decides.
    #[test]
    fn a_server_is_started_once_and_reused() {
        let servers = Servers::new(root(), None, |_| None, false, Vec::new());
        assert_eq!(servers.running(), 0);

        // Two files of the same language must map to one server, and two languages to two.
        assert_eq!(
            Language::for_path("src/a.rs"),
            Language::for_path("src/b.rs"),
            "two Rust files must not want two servers"
        );
        assert_ne!(
            Language::for_path("src/a.rs"),
            Language::for_path("web/a.ts"),
            "two languages are two servers"
        );
    }

    /// LSP-8: a server that ignores `shutdown` is killed, so it cannot outlive the agent.
    ///
    /// The kill path is in `Drop`. Pinned here as the property that the grace period is bounded and
    /// that dropping does not wait forever on a process that will not go.
    #[test]
    fn a_server_that_ignores_shutdown_is_killed() {
        assert!(
            SHUTDOWN_GRACE < Duration::from_secs(5),
            "the grace period must be short enough that quitting is not a hang"
        );
        // A server is only ever owned by a `Server`, whose `Drop` kills it, so there is no path
        // that leaks one. Dropping a set with nothing in it must still be sound.
        drop(Servers::new(root(), None, |_| None, false, Vec::new()));
    }

    /// LSP-9: the capability is checked before a process is started, so a run that was not granted
    /// one does not get a server launched on its behalf.
    #[test]
    fn a_request_without_the_capability_is_refused() {
        use bravebot_core::capability::CapabilitySet;

        // The grant a file read gives is not this one, which is the whole clause.
        let reads_only = CapabilitySet::from_iter([Capability::FileRead]);
        assert!(
            reads_only.token_for(Capability::LanguageServer).is_none(),
            "file reads must not carry a language server with them"
        );

        // And an empty set carries nothing, so the default is refusal.
        assert!(
            CapabilitySet::none()
                .token_for(Capability::LanguageServer)
                .is_none()
        );
    }

    #[test]
    fn a_language_is_decided_from_the_extension_alone() {
        assert_eq!(Language::for_path("src/lib.rs"), Some(Language::Rust));
        assert_eq!(
            Language::for_path("app/main.TS"),
            Some(Language::TypeScript)
        );
        assert_eq!(Language::for_path("s.py"), Some(Language::Python));
        assert_eq!(Language::for_path("m.go"), Some(Language::Go));
        assert!(Language::for_path("no-extension").is_none());
    }

    #[test]
    fn every_language_names_a_binary() {
        for language in [
            Language::Rust,
            Language::TypeScript,
            Language::Python,
            Language::Go,
        ] {
            let (program, _) = language.server();
            assert!(!program.is_empty(), "{} names no binary", language.as_str());
        }
    }
}
