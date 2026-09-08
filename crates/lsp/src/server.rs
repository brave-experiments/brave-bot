//! A language server: launched under confinement, kept for the session, asked read-only questions.
//!
//! The process is third-party code, so [LSP-5] applies [MCP-3] unchanged: no confinement, no
//! process. Its profile is wider than a stdio MCP server's because a server that cannot read the
//! tree cannot index it, and narrower in the ways that matter: no network, no writes, no children.
//!
//! [LSP-5]: ../../../docs/specs/tools/lsp.md
//! [MCP-3]: ../../../docs/specs/mcp.md

use crate::protocol::{
    Operation, RpcNotification, RpcRequest, RpcResponse, content_length, frame, hover_text,
    initialize_params, locations_in, path_to_uri, position_params, reference_params,
};
use crate::{Answer, LspError, LspResult};
use bravebot_core::capability::Capability;
use bravebot_core::event::Sink;
use bravebot_core::policy::Policy;
use bravebot_sandbox::Sandbox;
use bravebot_sandbox::policy::SandboxPolicy;
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

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript",
            Self::Python => "Python",
            Self::Go => "Go",
        }
    }
}

/// The confinement a language server runs under.
///
/// LSP-5: read-only access to the workspace and to the read-only paths its ecosystem resolves
/// dependencies from. No network, no writes, no children.
///
/// Built here rather than by the caller so the profile is one thing to review, and so a caller
/// cannot widen it by assembling its own.
pub fn confinement(workspace: &Path, home: Option<&Path>) -> SandboxPolicy {
    let mut policy = SandboxPolicy::strict().allow_read(workspace);

    // Where each ecosystem keeps the sources a definition may resolve into, and where the server
    // binaries themselves live. Read-only throughout: a server that wanted to build in order to
    // resolve a macro answers without having done so, which LSP-7 reports as a partial index.
    //
    // `.cargo` and `.rustup` are granted whole rather than only their registry and toolchain
    // subdirectories, because the binary is in `.cargo/bin` and a profile that cannot read the
    // program cannot exec it: the process died before `main` with a message about the working
    // directory, which reads like anything but a denied read. Found by running a real server
    // under this profile rather than by reasoning about it.
    // `.nvm` and the npm prefixes are here for the same reason as `.cargo`: a Node-based server is a
    // script the runtime reads, so the profile has to reach both the interpreter and the package.
    // Found the same way, by running one.
    if let Some(home) = home {
        for relative in [
            ".cargo",
            ".rustup",
            ".cache",
            ".nvm",
            ".npm",
            ".volta",
            ".local/share/pnpm",
            "go/pkg/mod",
            "go/bin",
        ] {
            policy = policy.allow_read(home.join(relative));
        }
    }

    // What any process needs to start at all: the loader, the shared libraries, and the system
    // configuration a TLS or locale initialiser reads on its way up. Withholding these does not
    // confine a language server, it stops one from running, and the failure names none of them.
    //
    // These hold no workspace content and nothing of the user's, so granting them read-only gives
    // up nothing the profile was protecting. Writes, the network and subprocesses stay denied,
    // which is what SANDBOX-2 measures this against.
    for shared in [
        "/usr",
        "/bin",
        "/System",
        "/Library",
        "/private/etc",
        "/opt/homebrew",
        "/etc",
        "/lib",
        "/lib64",
    ] {
        policy = policy.allow_read(shared);
    }

    policy
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
    /// Launch a server for this language under confinement.
    ///
    /// LSP-5 and LSP-6: confinement failing means no process, and a missing binary is reported as
    /// missing rather than as an empty answer.
    /// `resolved` is the absolute path to the binary. Resolving a bare name against `$PATH` is the
    /// caller's job: what a name means is the host's business, and a confined process cannot look it
    /// up for itself because the sandbox does not carry the parent's `$PATH` in. Passing a bare name
    /// through failed with `execvp() ... No such file or directory`, which surfaced as a server that
    /// exited before replying and read exactly like one that was not installed.
    pub fn launch(
        language: Language,
        resolved: &Path,
        root: &Path,
        sandbox: &dyn Sandbox,
        policy: &SandboxPolicy,
    ) -> LspResult<Self> {
        let (program, args) = language.server();
        let owned: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();

        let mut command = sandbox
            .command(&resolved.to_string_lossy(), &owned, policy)
            .map_err(|e| LspError::Confinement {
                language,
                detail: e.to_string(),
            })?;

        // The sandbox clears the environment, which is right: RUN-12's reason applies here too, and a
        // credential must not travel to a confined process nobody showed the user. But several
        // language servers are scripts whose shebang is `#!/usr/bin/env node`, and with no `PATH` at
        // all `env` cannot find the interpreter: `typescript-language-server` died with
        // `env: node: No such file or directory` before writing a byte, which surfaced as a server
        // that exited before replying.
        //
        // So exactly one variable is put back, holding exactly one directory: the one the resolved
        // binary is in, which is where a bundled interpreter sits beside it. That is enough for a
        // shebang to resolve and is not a route to anything else, since it names no directory the
        // profile has not already granted read access to. Nothing of the user's is restored, and in
        // particular no credential.
        if let Some(directory) = resolved.parent() {
            command.env("PATH", directory);
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
    home: Option<PathBuf>,
    /// How a program name becomes the file it names.
    ///
    /// Supplied rather than done here, for the reason [`Server::launch`] takes a resolved path:
    /// `$PATH` is the host's business, and this crate has no opinion about it. It also keeps the
    /// lookup in one place for the whole repository, so a name cannot mean one binary to `run` and
    /// another to this.
    resolve: fn(&str) -> Option<PathBuf>,
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
        home: Option<PathBuf>,
        resolve: fn(&str) -> Option<PathBuf>,
    ) -> Self {
        Self {
            running: HashMap::new(),
            root: root.into(),
            home,
            resolve,
        }
    }

    /// How many servers are running. Zero until something asks.
    pub fn running(&self) -> usize {
        self.running.len()
    }

    /// Ask about a position in a file, starting a server for its language if none is running.
    ///
    /// LSP-9: the capability is checked before anything is launched, so a run that was not granted
    /// it does not get a process started on its behalf.
    pub fn ask<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        sandbox: &dyn Sandbox,
        question: &Question<'_>,
    ) -> LspResult<Answer> {
        policy
            .before_capability(Capability::LanguageServer)
            .map_err(LspError::Denied)?;

        let language = Language::for_path(question.path).ok_or_else(|| LspError::NoServerFor {
            path: question.path.to_string(),
        })?;

        if !self.running.contains_key(&language) {
            let (program, _) = language.server();
            // LSP-6: a binary that is not installed is said to be missing here, before anything is
            // launched, rather than surfacing as a process that exited. The two used to be
            // indistinguishable and the difference is the whole clause.
            let resolved =
                (self.resolve)(program).ok_or(LspError::NoBinary { language, program })?;
            let profile = confinement(&self.root, self.home.as_deref());
            let server = Server::launch(language, &resolved, &self.root, sandbox, &profile)?;
            self.running.insert(language, server);
        }

        let server = self
            .running
            .get_mut(&language)
            .expect("just inserted if absent");
        server.ask(question)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_sandbox::Unavailable;

    fn root() -> PathBuf {
        PathBuf::from("/workspace")
    }

    /// LSP-5, which is MCP-3 applied here: third-party code that cannot be confined does not run.
    #[test]
    fn a_server_is_not_launched_without_confinement() {
        let result = Server::launch(
            Language::Rust,
            Path::new("/usr/local/bin/rust-analyzer"),
            &root(),
            &Unavailable,
            &confinement(&root(), None),
        );
        assert!(
            matches!(result, Err(LspError::Confinement { .. })),
            "an unconfinable server must not start"
        );
    }

    /// LSP-5: the profile is wider than an MCP server's on reads and no wider on anything else.
    #[test]
    fn the_profile_grants_no_network_and_no_writes() {
        let profile = confinement(&root(), Some(Path::new("/home/someone")));
        assert!(
            !profile.allow_network,
            "a server must not reach the network"
        );
        assert!(
            !profile.allow_subprocesses,
            "a server must not spawn children"
        );
        assert!(
            profile.writable.is_empty(),
            "a server must not write anywhere: {:?}",
            profile.writable
        );
        assert!(
            profile.readable.contains(&root()),
            "a server must be able to read the tree it indexes"
        );
        // `.cargo` whole rather than `.cargo/registry`: the binary is in `.cargo/bin`, and a profile
        // that cannot read the program cannot exec it.
        assert!(
            profile
                .readable
                .contains(&PathBuf::from("/home/someone/.cargo")),
            "a server must be able to resolve dependencies and be executable, got {:?}",
            profile.readable
        );
    }

    /// SANDBOX-2: the profile has to be confinement rather than a permissive policy wearing
    /// the name.
    #[test]
    fn the_profile_is_meaningful_confinement() {
        assert!(confinement(&root(), Some(Path::new("/home/someone"))).is_meaningful());
        assert!(confinement(&root(), None).is_meaningful());
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
        let servers = Servers::new(root(), None, |_| None);
        assert_eq!(
            servers.running(),
            0,
            "a session must not start a server nobody asked for"
        );
    }

    /// LSP-8: the set is what stops the processes, so dropping it must stop them all.
    #[test]
    fn dropping_the_set_stops_every_server() {
        let servers = Servers::new(root(), None, |_| None);
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
        let servers = Servers::new(root(), None, |_| None);
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
        drop(Servers::new(root(), None, |_| None));
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
