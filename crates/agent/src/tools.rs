//! The tool set offered to the model, and dispatch for the calls it makes.
//!
//! Only non-destructive tools are exposed. A model-proposed path may become routing for
//! a confined read (see `Policy::promote_confined_read`), which is what makes iteration
//! possible; nothing here can change the workspace, so a wrong choice costs a step
//! rather than causing harm.
//!
//! Writing is present but gated differently. A write destination chosen by the model is
//! routing derived from whatever it just read, so it is never promoted on its own: the
//! user is shown the change and must approve, and that approval mints a single-use
//! endorsement bound to the exact path. A refusal, or a context where nobody can be asked,
//! means no write.
//!
//! `edit_file` exists because that approval has to be readable. A whole-file body cannot be
//! reviewed on a terminal, so an edit names a passage instead and the user approves a diff.
//! Locating the passage is an ordinary confined read; only the write that follows needs the
//! endorsement. Where the passage is ambiguous the edit is refused rather than guessed,
//! since a guess would mutate bytes that were never shown to anyone.
//!
//! `spawn_processor` is how work happens in a file the planner may not read. It hands
//! quarantined content to a model that holds nothing: no tools, no memory, no second round,
//! and one quarantined output. What comes back is a reference like any other, and
//! `write_file`'s `contents_ref` is what puts it in a file. Between them, the planner can
//! change a file it never saw and the driver can write bytes it never opened.

use crate::confirm::{Confirmer, Decision, Intent, WriteRequest};
use crate::diff::Diff;
use crate::processor::{self, Chat};
use crate::report::{Activity, Reporter};
use bua_aichat::protocol::{Tool, ToolCall, Usage};
use bua_core::event::Sink;
use bua_core::policy::Policy;
use bua_core::slot::SlotStore;
use bua_core::todo::{self, Item, List, Status};
use bua_core::value::Labelled;
use serde_json::{Value, json};

use crate::workspace::{Page, Workspace};

/// The statuses the schema advertises, taken from the kernel so the two cannot drift.
const TODO_STATUSES: [&str; 3] = Status::NAMES;

/// The tools the model may call.
pub fn available() -> Vec<Tool> {
    vec![
        Tool::function(
            "read_file",
            "Read a UTF-8 text file from the workspace. Returns its lines. Long files come \
             back one page at a time; the result says so and gives the offset to continue \
             from.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative path, e.g. src/main.rs"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "1-based line to start at. Defaults to the start of \
                                        the file."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum lines to return. Capped so one read cannot \
                                        fill the conversation."
                    }
                },
                "required": ["path"]
            }),
        ),
        Tool::function(
            "list_files",
            "List files in the workspace, recursively, under a directory. Give a glob \
             pattern to narrow the result rather than listing everything.",
            json!({
                "type": "object",
                "properties": {
                    "directory": {
                        "type": "string",
                        "description": "Workspace-relative directory. Use \".\" for the root."
                    },
                    "pattern": {
                        "type": "string",
                        "description": "Optional glob, e.g. \"*.rs\" for Rust files at any \
                                        depth, or \"src/**/*.rs\" to anchor it. Supports \
                                        *, ? and **; brace groups are not supported."
                    }
                },
                "required": ["directory"]
            }),
        ),
        Tool::function(
            "write_file",
            "Write a UTF-8 text file in the workspace. Give either the contents or a reference \
             to quarantined content that becomes the contents. The user must approve each write \
             before it happens, so explain what you are changing.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative path, e.g. src/main.rs"
                    },
                    "contents": {
                        "type": "string",
                        "description": "The complete new contents of the file. Give this or \
                                        contents_ref, never both."
                    },
                    "contents_ref": {
                        "type": "string",
                        "description": "A reference whose quarantined content becomes the whole \
                                        file, e.g. \"ref:2\". This is how to write out something \
                                        you were never shown, such as what spawn_processor \
                                        produced."
                    }
                },
                "required": ["path"]
            }),
        ),
        Tool::function(
            "edit_file",
            "Replace an exact passage of text in an existing workspace file. Prefer this to \
             write_file when changing part of a file: the user approves a diff, which is \
             easier to review than a whole body. The user must approve each edit before it \
             happens, so explain what you are changing.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative path, e.g. src/main.rs"
                    },
                    "old_text": {
                        "type": "string",
                        "description": "The exact text to replace, matched byte for byte \
                                        including whitespace and indentation. Must occur \
                                        exactly once unless replace_all is true. Include \
                                        enough surrounding lines to be unique."
                    },
                    "new_text": {
                        "type": "string",
                        "description": "The text to put in its place."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace every occurrence instead of requiring \
                                        exactly one. Defaults to false."
                    }
                },
                "required": ["path", "old_text", "new_text"]
            }),
        ),
        Tool::function(
            "todo_write",
            "Record the task list for what you are doing, and keep it current. Send the whole \
             list every time: it replaces the previous one, so include finished tasks with \
             status completed rather than dropping them. Use it when the work takes several \
             steps, and update it as you go so the user can see progress. Skip it for a single \
             step or a question.",
            json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "description": "The complete list, in the order the work will happen.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {
                                    "type": "string",
                                    "description": "The task, in a few words."
                                },
                                "status": {
                                    "type": "string",
                                    "enum": TODO_STATUSES,
                                    "description": "Mark exactly one task in_progress while \
                                                    work remains on it."
                                }
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["todos"]
            }),
        ),
        Tool::function(
            "search",
            "Find a literal substring in workspace files. Returns matching lines.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Literal text to find. Not a regular expression."
                    },
                    "directory": {
                        "type": "string",
                        "description": "Workspace-relative directory to search. Defaults to \".\"."
                    },
                    "include": {
                        "type": "string",
                        "description": "Optional glob limiting which files are searched, \
                                        e.g. \"*.rs\". Supports *, ? and **; brace groups \
                                        are not supported."
                    }
                },
                "required": ["pattern"]
            }),
        ),
        Tool::function(
            "spawn_processor",
            "Transform quarantined content you were not shown. Spawns an isolated model with no \
             tools, no memory and nothing to read but the references you name; it follows your \
             instruction and its output is quarantined as a new reference. This is how to \
             change a file you cannot see: read the file, process the reference it gave you \
             into the contents you want, then pass the new reference to write_file as \
             contents_ref. You are not shown its output either, and nobody reads it before \
             it is written, so say exactly what it must be.",
            json!({
                "type": "object",
                "properties": {
                    "reads": {
                        "type": "array",
                        "description": "The references to give it, e.g. [\"ref:0\"]. At least one.",
                        "items": {"type": "string"}
                    },
                    "instruction": {
                        "type": "string",
                        "description": "What to do with them and what to produce. Say that you \
                                        want the whole document back where you do, since \
                                        anything shorter is what gets written out. Include the \
                                        file's name and language if that matters, because the \
                                        processor knows nothing but what you tell it and what \
                                        the references hold."
                    }
                },
                "required": ["reads", "instruction"]
            }),
        ),
    ]
}

/// What a dispatched call produced, ready to send back as a tool message.
#[derive(Debug)]
pub struct Output {
    pub call_id: Option<String>,
    pub tool: String,
    /// The rendered result, still labelled.
    ///
    /// Deliberately not a plain `String`: whether the planner may see this is the kernel's
    /// decision, made from the label in `Policy::present`. A tool that could hand back bare
    /// text would be a tool that decided for itself, and untrusted workspace content would
    /// reach the planner's context by whichever tool forgot.
    pub text: Labelled<String>,
    /// Where the content came from, for the reference the planner is shown instead.
    pub origin: String,
    /// What the call spent at the model, where it called one.
    ///
    /// Zero for every tool but the processor. A turn that reported only its own rounds would
    /// understate what it cost by however much its processors wrote.
    pub usage: Usage,
}

/// Everything a tool works with that is not the policy.
///
/// Three things, and the second two are new because of the processor: the quarantine, so a
/// reference the planner names resolves to something, and the model, so an isolated processor
/// has somewhere to run. Bundled rather than passed one by one because dispatch would otherwise
/// take seven arguments to give two tools what they need.
pub struct Tools<'a> {
    pub workspace: &'a Workspace,
    /// Where quarantined content lives, by the names the planner was given for it.
    pub slots: &'a mut SlotStore,
    /// The model an isolated processor runs on.
    pub chat: Chat<'a>,
}

/// What one tool produced, before dispatch wraps it up.
///
/// Two audiences, kept apart. `text` goes to the planner and stays labelled, because the kernel
/// decides whether the planner may see it. `note` and `changes` go to a screen and are already
/// released, because a person is allowed to read what a planner is not.
struct Produced {
    text: Labelled<String>,
    origin: String,
    /// What to tell the person watching. A few words, never the result itself.
    note: String,
    failed: bool,
    /// The change a write made, for showing under the line it belongs to.
    changes: Vec<crate::diff::Change>,
    /// What the tool spent at the model. Only a processor spends anything.
    usage: Usage,
}

impl Produced {
    /// A result the person watching is told about in the driver's own summary of it.
    fn new(text: Labelled<String>, origin: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            text,
            origin: origin.into(),
            note: note.into(),
            failed: false,
            changes: Vec::new(),
            usage: Usage::default(),
        }
    }

    fn with_changes(mut self, changes: Vec<crate::diff::Change>) -> Self {
        self.changes = changes;
        self
    }

    fn costing(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self
    }
}

/// The driver's word for what a tool does.
///
/// Chosen from the tool's name, which dispatch already matches on, so this decides nothing new.
/// A literal rather than the raw name because the line is read by a person: "Read(src/main.rs)"
/// says what happened and "read_file" says what was typed.
fn verb_for(tool: &str) -> &'static str {
    match tool {
        "read_file" => "Read",
        "list_files" => "List",
        "search" => "Search",
        "write_file" => "Write",
        "edit_file" => "Update",
        "todo_write" => "Plan",
        "spawn_processor" => "Process",
        _ => "Tool",
    }
}

/// Which argument names what a call is about.
///
/// Chosen from the tool's own name, which dispatch already matches on, so this decides nothing
/// new. `None` for a tool with no single argument naming a target.
fn target_key(tool: &str) -> Option<&'static str> {
    match tool {
        "read_file" | "write_file" | "edit_file" => Some("path"),
        "list_files" => Some("directory"),
        "search" => Some("pattern"),
        _ => None,
    }
}

/// What a call is about, for the line shown while it runs.
///
/// Which argument names the target depends on the tool, so the key is chosen from the tool's
/// own name. The value is the model's word for it, released to a screen and nowhere else: it
/// goes on a line a person reads and is never compared, matched, or routed anywhere.
fn target_of<S: Sink>(policy: &mut Policy<'_, S>, tool: &str, arguments: &Value) -> String {
    // A processor has no single argument naming a target: what it is working on is the set of
    // references it was given, which are names the driver handed out and can read back.
    if tool == "spawn_processor" {
        let proof = policy.authorise_display_release("what a tool is working on");
        return references_in(arguments).declassify(&proof);
    }

    let Some(key) = target_key(tool) else {
        return String::new();
    };

    match argument(arguments, key) {
        Some(value) => {
            let proof = policy.authorise_display_release("what a tool is working on");
            value.declassify(&proof)
        }
        None => String::new(),
    }
}

/// How a call reads in the transcript of a session read back off disk.
///
/// The same words a live call is announced with, from the same two functions, so a resumed
/// transcript and a running one describe the same call the same way.
///
/// No policy, and none to be had: a stored conversation is plain messages, whose labels went
/// when it was written down. Nothing here is being released that was not released already. The
/// call is in the record because the planner was allowed to hold it, and this puts the same
/// words on a screen that the person watching saw the first time round. It reaches a transcript
/// line and nothing else.
pub fn describe_stored_call(tool: &str, arguments: &str) -> String {
    let parsed: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);

    let target = if tool == "spawn_processor" {
        let names: Vec<&str> = parsed
            .get("reads")
            .and_then(Value::as_array)
            .map(|entries| entries.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        names.join(", ")
    } else {
        target_key(tool)
            .and_then(|key| parsed.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };

    Activity::running(verb_for(tool), target).line()
}

/// Shape a count out of a labelled result and release it to the person watching.
///
/// The reshape happens inside the kernel and only the shaped line comes out, so the driver
/// counts nothing it is not allowed to hold. A screen is one of the destinations a display
/// release exists for, and a count cannot feed an effect.
fn note_for<S: Sink, T: Clone>(
    policy: &mut Policy<'_, S>,
    tool: &'static str,
    content: &Labelled<T>,
    shape: impl FnOnce(T) -> String,
) -> String {
    let shaped = policy.render_in_place(tool, content, shape);
    let proof = policy.authorise_display_release("what a tool produced");
    shaped.declassify(&proof)
}

/// A count with the right noun, so a line does not read "1 lines".
pub(crate) fn tally(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Unchanged lines kept around each run of changes, so a hunk can be read in context.
const DIFF_CONTEXT: usize = 3;

/// What a write did, for the person watching: a line saying how much changed, and the hunks
/// themselves where showing them is worth the room.
///
/// Both sides are strings already released to a screen, so this reasons about no labels, the
/// same footing [`crate::diff`] has always been on.
///
/// A new file says it is new and shows what it now holds. It used to show a line count and
/// nothing else, on the grounds that every line of it is an addition and the body would fill the
/// screen. The display trims what it draws, so it does not; and in a directory the user has
/// vouched for a create is never reviewed either, so that count was the only thing they were
/// ever going to be told about a file that had just appeared in their workspace.
pub(crate) fn change_report(
    intent: Intent,
    existing: Option<&str>,
    written: &str,
    replaced_age: Option<std::time::Duration>,
) -> (String, Vec<crate::diff::Change>) {
    if intent == Intent::Create {
        return (
            format!(
                "new file, {}",
                tally(written.lines().count(), "line", "lines")
            ),
            written
                .lines()
                .map(|line| crate::diff::Change::Added(line.to_string()))
                .collect(),
        );
    }

    let diff = Diff::compute(existing.unwrap_or(""), written);
    let changed = format!(
        "added {}, removed {}",
        tally(diff.added(), "line", "lines"),
        tally(diff.removed(), "line", "lines")
    );

    // An overwrite says so, and says how old what it replaced was. "Replaced the file" was not
    // enough on its own: a user watching a session write a file for the first time reads it as
    // this session's own work being rewritten, and asks why the agent never said it created
    // anything. The answer is usually that the file was there before the session started, which
    // is a thing the age says and the count of lines does not.
    //
    // An edit needs no such word, since naming a passage to replace is what it is.
    let note = match (intent, replaced_age) {
        (Intent::Overwrite, Some(age)) => format!(
            "replaced a file written {}, {changed}",
            crate::report::how_long_ago(age)
        ),
        (Intent::Overwrite, None) => format!("replaced the file, {changed}"),
        _ => changed,
    };
    (note, diff.condensed(DIFF_CONTEXT))
}

/// Run one tool call the model asked for.
///
/// Errors are returned as text rather than failing the turn: a model that asked for a
/// missing file should be told so and allowed to try again, exactly as it would be told
/// about a compile error.
pub fn dispatch<S: Sink, C: Confirmer, R: Reporter>(
    policy: &mut Policy<'_, S>,
    tools: &mut Tools<'_>,
    confirmer: &mut C,
    reporter: &mut R,
    call: &ToolCall,
) -> Output {
    let name = call.function.name.clone();
    let verb = verb_for(&name);

    let arguments = match call.arguments() {
        Ok(value) => value,
        Err(e) => {
            let produced = problem(format!("error: the arguments were not valid JSON: {e}"));
            // Announced and closed in one breath, because there was never a call to watch.
            reporter.tool_started(Activity::running(verb, ""));
            reporter.tool_finished(Activity::running(verb, "").failed(produced.note.clone()));
            return Output {
                call_id: call.id.clone(),
                tool: name,
                text: produced.text,
                origin: produced.origin,
                usage: produced.usage,
            };
        }
    };

    // Announced before the call runs, so a slow one is visible while it is slow. This is the
    // difference between a turn that looks stuck and one that is plainly working.
    let target = target_of(policy, &name, &arguments);
    reporter.tool_started(Activity::running(verb, target.clone()));

    let produced = match name.as_str() {
        "read_file" => read_file(policy, tools.workspace, &arguments),
        "list_files" => list_files(policy, tools.workspace, &arguments),
        "search" => search(policy, tools.workspace, &arguments),
        "write_file" => write_file(policy, tools, confirmer, &arguments),
        "edit_file" => edit_file(policy, tools.workspace, confirmer, &arguments),
        "todo_write" => todo_write(policy, reporter, &arguments),
        "spawn_processor" => spawn_processor(policy, tools, &arguments),
        other => problem(format!("error: no such tool '{other}'")),
    };

    let finished = Activity::running(verb, target).with_changes(produced.changes);
    reporter.tool_finished(if produced.failed {
        finished.failed(produced.note)
    } else {
        finished.done(produced.note)
    });

    Output {
        call_id: call.id.clone(),
        tool: name,
        text: produced.text,
        origin: produced.origin,
        usage: produced.usage,
    }
}

/// A tool's own words about something that did not happen: an error, or a refusal. The driver
/// wrote them, so they are trusted, and they double as the line the person watching sees.
///
/// Distinct from workspace content, which is never trusted unless the trust map says so.
fn problem(text: impl Into<String>) -> Produced {
    let text = text.into();
    Produced {
        text: Labelled::trusted(text.clone()),
        origin: String::new(),
        note: text,
        failed: true,
        changes: Vec::new(),
        usage: Usage::default(),
    }
}

/// A tool's own words about something that did happen, with what to show for it.
fn confirmed(text: impl Into<String>, note: impl Into<String>) -> Produced {
    Produced::new(Labelled::trusted(text.into()), "", note)
}

/// Extract a string argument the model supplied, labelled untrusted because it is.
fn argument(arguments: &Value, key: &str) -> Option<Labelled<String>> {
    let raw = arguments.get(key)?.as_str()?.to_string();
    Some(Labelled::new(
        raw,
        bua_core::label::Label::untrusted_public(),
    ))
}

/// The reference names in a `reads` argument, as one line.
///
/// Wrapped like every other argument: what the planner asked for is model output, and the
/// driver reads it only where a gate says so. Entries that are not strings are left out here
/// and refused where the call is actually made.
fn references_in(arguments: &Value) -> Labelled<String> {
    let names: Vec<&str> = arguments
        .get("reads")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    Labelled::new(names.join(", "), bua_core::label::Label::untrusted_public())
}

fn read_file<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    arguments: &Value,
) -> Produced {
    let Some(proposed) = argument(arguments, "path") else {
        return problem("error: 'path' is required and must be a string");
    };

    let path = match policy.promote_confined_read("read_file", "path", &proposed) {
        Ok(p) => p,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    // A model that omits these gets the head of the file, which is the useful default.
    let offset = arguments
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX)
        .min(usize::MAX as u64) as usize;

    let (proposed_path, _) = proposed.into_parts_for_decoding();

    match workspace.read_page(policy, &path, offset, limit) {
        Ok(page) => {
            // Reshaped inside the kernel, so the driver never holds the text. Only
            // `Policy::present` decides whether the planner sees what comes out.
            let note = note_for(policy, "read_file", &page, |p| {
                tally(p.lines.len(), "line", "lines")
            });
            let rendered = policy.render_in_place("read_file", &page, |p| render_page(&p));
            Produced::new(rendered, proposed_path, note)
        }
        Err(e) => problem(format!("error: {e}")),
    }
}

/// Render a page, saying what was left out.
///
/// The counts matter more than they look: a model handed a silent window of a large file
/// will answer as though it read the whole thing.
fn render_page(page: &Page) -> String {
    if page.lines.is_empty() {
        return if page.total_lines == 0 {
            "(the file is empty)".to_string()
        } else {
            format!(
                "(no lines at that offset; the file has {} lines)",
                page.total_lines
            )
        };
    }

    let body = page.lines.join("\n");
    let mut notes = Vec::new();

    if page.first_line > 1 || page.next_line().is_some() {
        notes.push(format!(
            "showing lines {}-{} of {}",
            page.first_line,
            page.first_line + page.lines.len() - 1,
            page.total_lines
        ));
    }
    if let Some(next) = page.next_line() {
        notes.push(format!("continue with offset {next}"));
    }
    if page.long_lines > 0 {
        notes.push(format!("{} long line(s) were shortened", page.long_lines));
    }

    if notes.is_empty() {
        body
    } else {
        format!("{body}\n\n({})", notes.join("; "))
    }
}

fn list_files<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    arguments: &Value,
) -> Produced {
    let proposed = argument(arguments, "directory").unwrap_or_else(|| {
        Labelled::new(".".to_string(), bua_core::label::Label::untrusted_public())
    });

    let directory = match policy.promote_confined_read("list_files", "directory", &proposed) {
        Ok(d) => d,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    // A filter only narrows a confined, non-destructive read, so it is promotable on the
    // same terms as the directory itself.
    let pattern = match argument(arguments, "pattern") {
        Some(proposed) => match policy.promote_confined_read("list_files", "pattern", &proposed) {
            Ok(p) => Some(p),
            Err(denial) => return problem(format!("refused: {denial}")),
        },
        None => None,
    };

    let (proposed_dir, _) = proposed.into_parts_for_decoding();

    match workspace.list(policy, &directory, pattern.as_ref()) {
        Ok(listing) => {
            let note = note_for(policy, "list_files", &listing, |listing| {
                tally(listing.files.len(), "file", "files")
            });
            let rendered = policy.render_in_place("list_files", &listing, |listing| {
                if listing.files.is_empty() {
                    "(no files)".to_string()
                } else if listing.truncated {
                    // Said plainly, because a model given a silently capped listing will treat
                    // it as the whole tree and conclude a file does not exist.
                    format!(
                        "{}\n\n(this listing stopped at {} files and is incomplete; \
                         list a subdirectory to see more)",
                        listing.files.join("\n"),
                        listing.files.len()
                    )
                } else {
                    listing.files.join("\n")
                }
            });
            Produced::new(rendered, proposed_dir, note)
        }
        Err(e) => problem(format!("error: {e}")),
    }
}

/// Resolve a reference into the bytes a write will carry.
///
/// Three steps, each one a gate. The name is accepted as a reference rather than read as
/// content; the kernel resolves it, refusing a name that points at nothing; and what comes back
/// is released for a write that stays inside the workspace. The bytes are wrapped throughout, so
/// nothing here can look at what it is about to write.
fn quarantined_body<S: Sink>(
    policy: &mut Policy<'_, S>,
    slots: &SlotStore,
    named: &Labelled<String>,
    path: &str,
) -> Result<Labelled<String>, String> {
    let slot = policy
        .accept_reference("write_file", "contents_ref", named)
        .map_err(|denial| format!("refused: {denial}"))?;

    let content = policy
        .resolve("write_file", &slot, slots)
        .map_err(|denial| format!("refused: {denial}"))?;

    Ok(policy.declassify_into_workspace(&slot, path, content))
}

/// Write a file, after a person approves it.
///
/// The order matters: the user sees the exact path and body *before* any grant exists, and
/// the grant is issued only for what they saw. Issuing it earlier would mean approving a
/// value that could still change.
fn write_file<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    tools: &mut Tools<'_>,
    confirmer: &mut C,
    arguments: &Value,
) -> Produced {
    let workspace = tools.workspace;
    let Some(path) = argument(arguments, "path") else {
        return problem("error: 'path' is required and must be a string");
    };

    // The path is routing, so naming a destination from it is not a content decision.
    let proposed_path = path.clone().into_parts_for_decoding().0;

    let written = argument(arguments, "contents");
    let named = argument(arguments, "contents_ref");

    // Two sources would leave the driver deciding which one was meant, and they say different
    // things about what lands in the file. Neither is a decision taken from content: both
    // arguments are the planner's, and this only reports which of them are present.
    let body = match (written, named) {
        (Some(_), Some(_)) => {
            return problem(
                "error: give 'contents' or 'contents_ref', not both. Use contents_ref alone \
                 when the file is to hold quarantined content.",
            );
        }
        (None, None) => {
            return problem("error: one of 'contents' or 'contents_ref' is required");
        }
        // The body is the model's words. Its integrity is that of the context the model was
        // working from, which the kernel tracked: nothing here upgrades anything.
        (Some(contents), None) => {
            let (raw_body, _) = contents.into_parts_for_decoding();
            policy.label_model_output("write_file", raw_body)
        }
        // Quarantined content, going where the planner said without the planner or the driver
        // having read a byte of it. The user still sees it, which is what an approval is.
        (None, Some(reference)) => {
            match quarantined_body(policy, tools.slots, &reference, &proposed_path) {
                Ok(body) => body,
                Err(refusal) => return problem(refusal),
            }
        }
    };
    let body_label = body.label();

    // Released for display only, and released whether or not anyone is asked: the reviewer sees
    // it before approving, and the same text is what the finished line reports having written.
    // A display release cannot feed an effect.
    let shown = {
        let proof = policy.authorise_display_release("proposed write");
        body.clone().declassify(&proof)
    };
    let existing = workspace.peek_for_review(&proposed_path);
    // Read before the write, since afterwards the age is the age of this write.
    let replaced_age = workspace.age_of(&proposed_path);
    let intent = if existing.is_some() {
        Intent::Overwrite
    } else {
        Intent::Create
    };

    if policy.write_needs_approval(&proposed_path, body_label) {
        let request = WriteRequest {
            intent,
            existing: existing.clone(),
            path: proposed_path.clone(),
            contents: shown.clone(),
        };

        if confirmer.confirm_write(&request) == Decision::Reject {
            return problem(format!(
                "refused: the user did not approve writing {proposed_path}. Do not retry \
                 the same write; ask what they would prefer."
            ));
        }
    }

    // The approval is what makes the path trusted, and it is bound to this exact value.
    policy.issue_grant("file_write", "path", proposed_path.clone());

    match workspace.write_endorsed(policy, &path, &body) {
        Ok(_) => {
            // The file now holds this data, so the map must say what the path means.
            policy.reconcile_after_write(&proposed_path, body_label);
            let (note, changes) = change_report(intent, existing.as_deref(), &shown, replaced_age);
            // What the model is told, which is what its own account of the turn will repeat. It
            // used to be told "wrote" either way, and would go on to say it had created a file
            // it had in fact replaced, which is the opposite of what the user needed to hear.
            let done = match intent {
                Intent::Create => format!("created {proposed_path}"),
                _ => format!("replaced {proposed_path}, which was already there"),
            };
            confirmed(done, note).with_changes(changes)
        }
        Err(e) => problem(format!("error: {e}")),
    }
}

/// Replace an exact passage in a file, after a person approves the diff.
///
/// Same endorsement shape as [`write_file`], since the model never decides a write destination,
/// but the reviewer is shown a diff of a located passage rather than a whole body, which is
/// the point of having this tool at all.
///
/// The file is read through the gates rather than peeked at, so the read is recorded and
/// the contents carry their label. The replacement then happens on released bytes, and the
/// result is written back only if the file still matches what was read.
fn edit_file<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    confirmer: &mut C,
    arguments: &Value,
) -> Produced {
    let Some(proposed) = argument(arguments, "path") else {
        return problem("error: 'path' is required and must be a string");
    };
    let Some(old_text) = argument(arguments, "old_text") else {
        return problem("error: 'old_text' is required and must be a string");
    };
    let Some(new_text) = argument(arguments, "new_text") else {
        return problem("error: 'new_text' is required and must be a string");
    };
    // Absent or non-boolean means the strict single-match behaviour, which is the safe
    // reading of an ambiguous argument.
    let replace_all = arguments
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // Reading to locate the passage is non-destructive and confined, so the path may be
    // promoted here exactly as it is for read_file. The write below is what needs a person.
    let path = match policy.promote_confined_read("edit_file", "path", &proposed) {
        Ok(p) => p,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    let source = match workspace.read(policy, &path) {
        Ok(contents) => contents,
        Err(e) => return problem(format!("error: {e}")),
    };

    // Locating the passage means comparing text, which is a decision. It is only permissible
    // on trusted content: doing it on untrusted bytes would let file content decide whether an
    // effect happens, which is the one thing this design forbids. An untrusted file is refused
    // rather than edited blind. The user can vouch for the path if they want edits there.
    //
    // Confidentiality is not the question here. Workspace content is private, and staying
    // inside the process to locate a passage releases nothing; only integrity decides whether
    // this comparison is safe to make.
    let current = match policy.read_trusted_content("edit_file", &source) {
        Ok(text) => text,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    let (old_text, _) = old_text.into_parts_for_decoding();
    let (new_text, _) = new_text.into_parts_for_decoding();

    let replaced = match crate::replace::replace(&current, &old_text, &new_text, replace_all) {
        Ok(r) => r,
        Err(e) => return problem(format!("error: {e}")),
    };

    let (proposed_path, _) = proposed.into_parts_for_decoding();

    // The result is the model's edit applied to trusted text, so its integrity is that of the
    // context the model was working from.
    let body = policy.label_model_output("edit_file", replaced.contents);
    let body_label = body.label();

    let shown = {
        let proof = policy.authorise_display_release("proposed edit");
        body.clone().declassify(&proof)
    };

    if policy.write_needs_approval(&proposed_path, body_label) {
        let request = WriteRequest {
            path: proposed_path.clone(),
            contents: shown.clone(),
            existing: Some(current.clone()),
            intent: Intent::Edit,
        };

        if confirmer.confirm_write(&request) == Decision::Reject {
            return problem(format!(
                "refused: the user did not approve editing {proposed_path}. Do not retry the \
                 same edit; ask what they would prefer."
            ));
        }
    }

    policy.issue_grant("file_write", "path", proposed_path.clone());

    let occurrences = replaced.occurrences;
    match workspace.write_endorsed_if_unchanged(policy, &path, &body, &current) {
        Ok(_) => {
            policy.reconcile_after_write(&proposed_path, body_label);
            let (note, changes) = change_report(Intent::Edit, Some(&current), &shown, None);
            confirmed(
                format!("edited {proposed_path}: {occurrences} replacement(s)"),
                note,
            )
            .with_changes(changes)
        }
        Err(e) => problem(format!("error: {e}")),
    }
}

/// Record the task list and show it.
///
/// The one tool here with no workspace effect at all: nothing is read, nothing is written, and
/// there is no path to endorse. It is the planner's own note to itself, carried to a screen.
///
/// Two things follow from that. The list is model output, so its integrity is the context's, and
/// it is never upgraded on the way through. And the whole list arrives every time, because
/// amending a single entry would mean locating it by model-authored text, which is a comparison
/// on untrusted content. Replacing the list wholesale compares nothing.
fn todo_write<S: Sink, R: Reporter>(
    policy: &mut Policy<'_, S>,
    reporter: &mut R,
    arguments: &Value,
) -> Produced {
    let Some(todos) = arguments.get("todos").and_then(Value::as_array) else {
        return problem("error: 'todos' is required and must be an array");
    };

    // Parsing is not a decision about what happens: every entry becomes an item, and an
    // unreadable status becomes outstanding work rather than being rejected. A malformed entry
    // is skipped only because there is nothing to show for it.
    let items: Vec<Item> = todos
        .iter()
        .filter_map(|entry| {
            let content = entry.get("content")?.as_str()?.to_string();
            let status = entry
                .get("status")
                .and_then(Value::as_str)
                .map(Status::parse)
                .unwrap_or(Status::Pending);
            Some(Item::new(content, status))
        })
        .collect();

    if items.len() != todos.len() {
        return problem(
            "error: every todo needs a 'content' string; the list was not changed. Send the \
             whole list again.",
        );
    }

    // The model's words, at the integrity of the context they came from.
    let list = policy.label_model_output("todo_write", List::new(items));

    // Shaped inside the kernel, because choosing a glyph means reading the statuses and the
    // driver may not hold them. Every item yields a row, so nothing in the content decides
    // what the user is shown the existence of.
    let rows = policy.render_in_place("todo_write", &list, |list| todo::rows(&list));

    // Showing a person what the model is doing is a release to a screen, which is one of the
    // destinations a witness exists for. It cannot feed an effect.
    let proof = policy.authorise_display_release("task list");
    reporter.todos(rows.declassify(&proof));

    // The model gets its own list back as the tool result, which is how it knows what is next:
    // the turn keeps no state, so the echo in the conversation *is* the memory. Rendered through
    // the kernel like everything else, then presented under the usual gate by the caller.
    let summary = policy.render_in_place("todo_write", &list, |list| {
        if list.is_empty() {
            return "the task list is now empty".to_string();
        }
        let lines = list
            .items
            .iter()
            .map(|item| format!("[{}] {}", item.status, item.content))
            .collect::<Vec<_>>()
            .join("\n");
        format!("{} of {} done\n{lines}", list.done(), list.len())
    });

    let note = note_for(policy, "todo_write", &list, |list| {
        format!("{} of {} done", list.done(), list.len())
    });

    Produced::new(summary, "", note)
}

/// Hand quarantined content to an isolated model and quarantine what comes back.
///
/// The tool that makes an untrusted workspace workable. Everything the planner cannot read, a
/// processor can, and everything a processor produces the planner still cannot read: what comes
/// back is a reference, exactly as a read of an untrusted file is.
///
/// Nothing here decides anything from content. The references are names the driver handed out,
/// the instruction is the planner's, and the label on the result is computed by the kernel from
/// the inputs before the processor runs.
fn spawn_processor<S: Sink>(
    policy: &mut Policy<'_, S>,
    tools: &mut Tools<'_>,
    arguments: &Value,
) -> Produced {
    let Some(instruction) = argument(arguments, "instruction") else {
        return problem("error: 'instruction' is required and must be a string");
    };
    let Some(entries) = arguments.get("reads").and_then(Value::as_array) else {
        return problem(
            "error: 'reads' is required and must be an array of reference names, e.g. \
             [\"ref:0\"]",
        );
    };

    let mut reads = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(name) = entry.as_str() else {
            return problem(
                "error: every entry in 'reads' must be a reference name, e.g. \"ref:0\"",
            );
        };
        let named = Labelled::new(name.to_string(), bua_core::label::Label::untrusted_public());
        match policy.accept_reference("spawn_processor", "reads", &named) {
            Ok(slot) => reads.push(slot),
            Err(denial) => return problem(format!("refused: {denial}")),
        }
    }

    // Named for the audit trail from the slots it reads, which are the driver's own names for
    // things. Two processors reading the same references in one turn share a name, and that is
    // the honest description of them.
    let origin = {
        let proof = policy.authorise_display_release("which references a processor was given");
        format!(
            "processor over {}",
            references_in(arguments).declassify(&proof)
        )
    };

    let spec = match policy.before_processor(&origin, &reads, &instruction, tools.slots) {
        Ok(spec) => spec,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    match processor::run(policy, &mut tools.chat, tools.slots, &spec) {
        Ok(done) => {
            let note = note_for(policy, "spawn_processor", &done.text, |text: String| {
                tally(text.lines().count(), "line", "lines")
            });
            Produced::new(done.text, origin, note).costing(done.usage)
        }
        Err(error) => problem(format!("error: {error}")),
    }
}

fn search<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    arguments: &Value,
) -> Produced {
    let Some(pattern) = argument(arguments, "pattern") else {
        return problem("error: 'pattern' is required and must be a string");
    };
    let proposed_dir = argument(arguments, "directory").unwrap_or_else(|| {
        Labelled::new(".".to_string(), bua_core::label::Label::untrusted_public())
    });

    let pattern = match policy.promote_confined_read("search", "pattern", &pattern) {
        Ok(p) => p,
        Err(denial) => return problem(format!("refused: {denial}")),
    };
    let directory = match policy.promote_confined_read("search", "directory", &proposed_dir) {
        Ok(d) => d,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    let include = match argument(arguments, "include") {
        Some(proposed) => match policy.promote_confined_read("search", "include", &proposed) {
            Ok(p) => Some(p),
            Err(denial) => return problem(format!("refused: {denial}")),
        },
        None => None,
    };

    let (proposed_where, _) = proposed_dir.into_parts_for_decoding();

    match workspace.grep(policy, &pattern, &directory, include.as_ref()) {
        Ok(found) => {
            let note = note_for(policy, "search", &found, |found| {
                tally(found.matches.len(), "match", "matches")
            });
            let rendered = policy.render_in_place("search", &found, |found| {
                if found.matches.is_empty() {
                    return "(no matches)".to_string();
                }
                let lines = found
                    .matches
                    .iter()
                    .map(|m| format!("{}:{}: {}", m.path, m.line, m.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                if found.truncated {
                    // Without this a model that gets exactly the cap concludes it has
                    // every occurrence, which is how a rename misses call sites.
                    format!(
                        "{lines}\n\n(this search stopped at {} matches and is \
                         incomplete; narrow the pattern or search a subdirectory)",
                        found.matches.len()
                    )
                } else {
                    lines
                }
            });
            Produced::new(rendered, proposed_where, note)
        }
        Err(e) => problem(format!("error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_set_is_reads_plus_gated_writes() {
        let names: Vec<String> = available()
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        assert_eq!(
            names,
            vec![
                "read_file",
                "list_files",
                "write_file",
                "edit_file",
                "todo_write",
                "search",
                "spawn_processor"
            ]
        );
    }

    /// Command execution stays absent. Unlike a write, a command has no separable routing
    /// field to endorse, because the string is destination and payload at once, so there is
    /// nothing a user could meaningfully approve.
    #[test]
    fn no_command_execution_is_offered() {
        for tool in available() {
            let name = tool.function.name;
            assert!(!name.contains("exec"), "{name} executes commands");
            assert!(!name.contains("shell"), "{name} executes commands");
            assert!(!name.contains("run"), "{name} executes commands");
        }
    }

    /// Every mutating tool must advertise that approval is required, so the model explains
    /// a change before proposing it.
    #[test]
    fn the_mutating_tools_state_that_approval_is_required() {
        for name in ["write_file", "edit_file"] {
            let tool = available()
                .into_iter()
                .find(|t| t.function.name == name)
                .unwrap_or_else(|| panic!("{name} is offered"));
            assert!(
                tool.function.description.contains("approve"),
                "{name} does not mention approval: {}",
                tool.function.description
            );
        }
    }

    /// The edit tool must state that matching is exact, since a model that assumes fuzzy
    /// matching will propose passages that are refused.
    #[test]
    fn the_edit_tool_states_that_matching_is_exact() {
        let edit = available()
            .into_iter()
            .find(|t| t.function.name == "edit_file")
            .expect("edit_file is offered");
        let old_text = edit.function.parameters["properties"]["old_text"]["description"]
            .as_str()
            .expect("old_text is described");
        assert!(
            old_text.contains("exact") && old_text.contains("whitespace"),
            "the description does not require an exact match: {old_text}"
        );
    }

    #[test]
    fn every_tool_declares_a_schema() {
        for tool in available() {
            assert_eq!(tool.kind, "function");
            assert_eq!(tool.function.parameters["type"], "object");
            assert!(!tool.function.description.is_empty());
        }
    }

    #[test]
    fn string_arguments_are_labelled_untrusted() {
        let arguments = json!({"path": "src/main.rs"});
        let value = argument(&arguments, "path").expect("present");
        assert_eq!(value.label(), bua_core::label::Label::untrusted_public());
    }

    #[test]
    fn a_missing_argument_is_none() {
        assert!(argument(&json!({}), "path").is_none());
        // A non-string is treated as absent rather than coerced.
        assert!(argument(&json!({"path": 42}), "path").is_none());
    }

    /// The advertised statuses come from the kernel, so the instruction cannot describe a
    /// vocabulary the parser does not read.
    #[test]
    fn the_todo_schema_advertises_the_statuses_the_kernel_parses() {
        let tool = available()
            .into_iter()
            .find(|t| t.function.name == "todo_write")
            .expect("todo_write is offered");
        let advertised = tool.function.parameters["properties"]["todos"]["items"]["properties"]
            ["status"]["enum"]
            .as_array()
            .expect("the statuses are enumerated");

        for value in advertised {
            let name = value.as_str().expect("a string");
            assert_eq!(
                Status::parse(name).to_string(),
                name,
                "'{name}' is advertised but does not round-trip"
            );
        }
    }

    /// The model has to be told the list is replaced wholesale, or it will send only what changed
    /// and the finished tasks will vanish from the display.
    #[test]
    fn the_todo_tool_states_that_the_whole_list_is_required() {
        let tool = available()
            .into_iter()
            .find(|t| t.function.name == "todo_write")
            .expect("todo_write is offered");
        assert!(
            tool.function.description.contains("whole list"),
            "the description does not ask for the whole list: {}",
            tool.function.description
        );
    }

    mod activity {
        use super::*;

        /// Every tool the model is offered needs a word of its own. Without this a new tool
        /// shows up in the transcript as the fallback, which tells the user nothing.
        #[test]
        fn every_offered_tool_has_its_own_verb() {
            for tool in available() {
                let name = &tool.function.name;
                assert_ne!(
                    verb_for(name),
                    verb_for("something nobody wrote"),
                    "{name} has no verb of its own"
                );
            }
        }

        #[test]
        fn counts_read_naturally_in_both_numbers() {
            assert_eq!(tally(0, "line", "lines"), "0 lines");
            assert_eq!(tally(1, "line", "lines"), "1 line");
            assert_eq!(tally(2, "match", "matches"), "2 matches");
        }

        /// A file appearing in someone's workspace should say so and show what is in it. Told
        /// only that three lines were written, the user has no idea what was created, and in a
        /// directory they have vouched for nothing else will tell them either.
        #[test]
        fn a_new_file_says_it_is_new_and_shows_what_it_holds() {
            let (note, changes) = change_report(Intent::Create, None, "one\ntwo\nthree\n", None);
            assert_eq!(note, "new file, 3 lines");
            assert_eq!(
                changes,
                vec![
                    crate::diff::Change::Added("one".to_string()),
                    crate::diff::Change::Added("two".to_string()),
                    crate::diff::Change::Added("three".to_string()),
                ]
            );
        }

        /// The three have to be distinguishable at a glance. A file that did not exist a
        /// moment ago, a file that did and no longer holds what it held, and a passage
        /// replaced inside one are different things to have done to somebody's workspace, and
        /// a diff on its own does not tell them apart: a whole-file rewrite whose diff is two
        /// lines looks exactly like a two-line edit.
        #[test]
        fn an_overwrite_says_it_replaced_a_file_and_an_edit_does_not() {
            let (overwritten, _) = change_report(Intent::Overwrite, Some("old\n"), "new\n", None);
            assert_eq!(
                overwritten,
                "replaced the file, added 1 line, removed 1 line"
            );

            let (edited, _) = change_report(Intent::Edit, Some("old\n"), "new\n", None);
            assert_eq!(edited, "added 1 line, removed 1 line");

            let (created, _) = change_report(Intent::Create, None, "new\n", None);
            assert_eq!(created, "new file, 1 line");
        }

        /// The line that was missing. A file being replaced for the first time in a session
        /// looks like the session's own work being rewritten, and the user asks why nothing
        /// ever said it was created. Its age is the answer: it was there before any of this.
        #[test]
        fn an_overwrite_says_how_old_the_file_it_replaced_was() {
            let (note, _) = change_report(
                Intent::Overwrite,
                Some("old\n"),
                "new\n",
                Some(std::time::Duration::from_secs(12 * 60)),
            );
            assert_eq!(
                note,
                "replaced a file written 12 minutes ago, added 1 line, removed 1 line"
            );
        }

        /// An edit is reported by what it changed, and carries the hunks so the user can see
        /// the change rather than take the counts on trust.
        #[test]
        fn an_edit_is_reported_by_what_it_changed() {
            let (note, changes) = change_report(
                Intent::Edit,
                Some("keep\nold\n"),
                "keep\nnew\nextra\n",
                None,
            );
            assert_eq!(note, "added 2 lines, removed 1 line");
            assert!(
                changes.contains(&crate::diff::Change::Added("new".to_string())),
                "the hunks do not show the change: {changes:?}"
            );
        }

        /// A write that changes nothing must say so rather than reporting a size, which would
        /// read as though the whole file had been rewritten.
        #[test]
        fn an_edit_that_changes_nothing_says_nothing_changed() {
            let (note, _) = change_report(Intent::Edit, Some("same\n"), "same\n", None);
            assert_eq!(note, "added 0 lines, removed 0 lines");
        }
    }

    mod todos {
        use super::*;
        use crate::report::RecordingReporter;
        use bua_core::capability::{Capability, CapabilitySet};
        use bua_core::event::RecordingSink;
        use bua_core::label::Integrity;
        use bua_core::policy::{ReleasePlan, Routing};

        fn routing() -> Routing {
            let mut r = Routing::new();
            r.insert_trusted("task", "do some work");
            r
        }

        /// Run the tool against a fresh policy, returning what the reporter saw and what the model
        /// was told.
        fn call(arguments: Value) -> (RecordingReporter, Labelled<String>) {
            let mut sink = RecordingSink::new();
            let mut policy = Policy::begin(
                routing(),
                ReleasePlan::new(),
                CapabilitySet::from_iter([Capability::FileRead]),
                &mut sink,
            )
            .expect("policy");
            let mut reporter = RecordingReporter::default();
            let produced = todo_write(&mut policy, &mut reporter, &arguments);
            // A task list has no destination, so there is nothing for an origin to name.
            assert!(
                produced.origin.is_empty(),
                "a task list named an origin: {}",
                produced.origin
            );
            (reporter, produced.text)
        }

        /// Read what the model was told, through the display gate rather than by minting a
        /// witness: only the policy layer can mint one, which is the point.
        fn released(text: &Labelled<String>) -> String {
            let mut sink = RecordingSink::new();
            let mut policy = Policy::begin(
                routing(),
                ReleasePlan::new(),
                CapabilitySet::from_iter([Capability::FileRead]),
                &mut sink,
            )
            .expect("policy");
            let proof = policy.authorise_display_release("test inspects the tool result");
            text.clone().declassify(&proof)
        }

        fn list(entries: &[(&str, &str)]) -> Value {
            json!({
                "todos": entries
                    .iter()
                    .map(|(content, status)| json!({"content": content, "status": status}))
                    .collect::<Vec<_>>()
            })
        }

        #[test]
        fn a_list_reaches_the_display_shaped_for_it() {
            let (reporter, _) = call(list(&[
                ("Read the file", "completed"),
                ("Make the change", "in_progress"),
                ("Run the tests", "pending"),
            ]));

            let rows = reporter.updates.last().expect("the display was told");
            assert_eq!(rows.len(), 3);
            assert!(rows[0].struck(), "the finished task is not struck through");
            assert!(!rows[1].struck());
            assert_eq!(rows[1].content, "Make the change");
        }

        /// The tool result is how the model knows what is next: the turn keeps no state, so the
        /// echo in the conversation is the only memory of the list.
        #[test]
        fn the_model_is_told_the_list_back() {
            let (_, text) = call(list(&[
                ("Read the file", "completed"),
                ("Make the change", "in_progress"),
            ]));

            let shown = released(&text);
            assert!(shown.contains("Make the change"), "the list is not echoed");
            assert!(shown.contains("1 of 2"), "progress is not reported");
        }

        /// The list is model output, so it can only ever be as trusted as the context it came
        /// from, and never more.
        #[test]
        fn the_list_is_labelled_from_the_context_not_upgraded() {
            let mut sink = RecordingSink::new();
            let mut policy = Policy::begin(
                routing(),
                ReleasePlan::new(),
                CapabilitySet::from_iter([Capability::FileRead]),
                &mut sink,
            )
            .expect("policy")
            // A conversation that had already been shown something untrusted, which is the only
            // way a context is untrusted: a read the planner was never shown does not do it.
            .resuming(Integrity::Untrusted);
            assert_eq!(policy.context_integrity(), Integrity::Untrusted);

            let mut reporter = RecordingReporter::default();
            let text = todo_write(
                &mut policy,
                &mut reporter,
                &list(&[("after reading something untrusted", "pending")]),
            )
            .text;

            assert_eq!(
                text.label().integrity,
                Integrity::Untrusted,
                "a list written after an untrusted read was labelled trusted"
            );
            assert!(
                text.into_trusted().is_err(),
                "the list came back as bare text"
            );
        }

        /// An unreadable status is outstanding work. Treating it as done would let a typo mark
        /// work finished that never was.
        #[test]
        fn an_unrecognised_status_shows_as_outstanding() {
            let (reporter, _) = call(list(&[("something", "nearly done")]));
            let rows = reporter.updates.last().expect("told");
            assert!(!rows[0].struck());
        }

        /// A list with no items is a list the model cleared, and the display must follow rather
        /// than keeping the previous one on screen.
        #[test]
        fn an_empty_list_is_reported_as_empty() {
            let (reporter, text) = call(json!({"todos": []}));
            assert_eq!(reporter.updates.last().expect("told").len(), 0);

            assert!(released(&text).contains("empty"));
        }

        /// A malformed list changes nothing. Showing a partial list would be worse than showing
        /// none, since the user could not tell which tasks were dropped.
        #[test]
        fn a_malformed_entry_leaves_the_list_alone() {
            let (reporter, text) = call(json!({"todos": [
                {"content": "fine", "status": "pending"},
                {"status": "pending"},
            ]}));

            assert!(
                reporter.updates.is_empty(),
                "a partial list reached the display"
            );
            assert!(released(&text).starts_with("error:"));
        }

        #[test]
        fn a_missing_list_is_an_error() {
            let (reporter, text) = call(json!({}));
            assert!(reporter.updates.is_empty());
            assert!(released(&text).starts_with("error:"));
        }

        /// Nothing about a task list is routing: it lands nowhere, so no gate should have been
        /// asked to endorse a destination.
        #[test]
        fn recording_a_list_needs_no_endorsement() {
            let mut sink = RecordingSink::new();
            let mut policy = Policy::begin(
                routing(),
                ReleasePlan::new(),
                // No write capability at all, so a tool that tried to route anywhere would fail.
                CapabilitySet::from_iter([Capability::FileRead]),
                &mut sink,
            )
            .expect("policy");

            let mut reporter = RecordingReporter::default();
            todo_write(
                &mut policy,
                &mut reporter,
                &list(&[("a task", "in_progress")]),
            );

            assert_eq!(reporter.updates.len(), 1);
            assert!(policy.finish(), "a gate refused something");
        }
    }
}
