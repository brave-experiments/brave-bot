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

use crate::confirm::{Confirmer, Decision, Intent, WriteRequest};
use crate::report::Reporter;
use bua_aichat::protocol::{Tool, ToolCall};
use bua_core::event::Sink;
use bua_core::policy::Policy;
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
            "Write a UTF-8 text file in the workspace. The user must approve each write \
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
                        "description": "The complete new contents of the file."
                    }
                },
                "required": ["path", "contents"]
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
}

/// Run one tool call the model asked for.
///
/// Errors are returned as text rather than failing the turn: a model that asked for a
/// missing file should be told so and allowed to try again, exactly as it would be told
/// about a compile error.
pub fn dispatch<S: Sink, C: Confirmer, R: Reporter>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    confirmer: &mut C,
    reporter: &mut R,
    call: &ToolCall,
) -> Output {
    let name = call.function.name.clone();
    let arguments = match call.arguments() {
        Ok(value) => value,
        Err(e) => {
            return Output {
                call_id: call.id.clone(),
                tool: name,
                text: Labelled::trusted(format!("error: the arguments were not valid JSON: {e}")),
                origin: String::new(),
            };
        }
    };

    let (text, origin) = match name.as_str() {
        "read_file" => read_file(policy, workspace, &arguments),
        "list_files" => list_files(policy, workspace, &arguments),
        "search" => search(policy, workspace, &arguments),
        "write_file" => write_file(policy, workspace, confirmer, &arguments),
        "edit_file" => edit_file(policy, workspace, confirmer, &arguments),
        "todo_write" => todo_write(policy, reporter, &arguments),
        other => (
            Labelled::trusted(format!("error: no such tool '{other}'")),
            String::new(),
        ),
    };

    Output {
        call_id: call.id.clone(),
        tool: name,
        text,
        origin,
    }
}

/// A tool's own words: an error, a refusal, a confirmation. The driver wrote them, so they are
/// trusted. Distinct from workspace content, which never is unless the trust map says so.
fn own_words(text: impl Into<String>) -> (Labelled<String>, String) {
    (Labelled::trusted(text.into()), String::new())
}

/// Extract a string argument the model supplied, labelled untrusted because it is.
fn argument(arguments: &Value, key: &str) -> Option<Labelled<String>> {
    let raw = arguments.get(key)?.as_str()?.to_string();
    Some(Labelled::new(
        raw,
        bua_core::label::Label::untrusted_public(),
    ))
}

fn read_file<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    arguments: &Value,
) -> (Labelled<String>, String) {
    let Some(proposed) = argument(arguments, "path") else {
        return own_words("error: 'path' is required and must be a string");
    };

    let path = match policy.promote_confined_read("read_file", "path", &proposed) {
        Ok(p) => p,
        Err(denial) => return own_words(format!("refused: {denial}")),
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
            let rendered = policy.render_in_place("read_file", &page, |p| render_page(&p));
            (rendered, proposed_path)
        }
        Err(e) => own_words(format!("error: {e}")),
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
) -> (Labelled<String>, String) {
    let proposed = argument(arguments, "directory").unwrap_or_else(|| {
        Labelled::new(".".to_string(), bua_core::label::Label::untrusted_public())
    });

    let directory = match policy.promote_confined_read("list_files", "directory", &proposed) {
        Ok(d) => d,
        Err(denial) => return own_words(format!("refused: {denial}")),
    };

    // A filter only narrows a confined, non-destructive read, so it is promotable on the
    // same terms as the directory itself.
    let pattern = match argument(arguments, "pattern") {
        Some(proposed) => match policy.promote_confined_read("list_files", "pattern", &proposed) {
            Ok(p) => Some(p),
            Err(denial) => return own_words(format!("refused: {denial}")),
        },
        None => None,
    };

    let (proposed_dir, _) = proposed.into_parts_for_decoding();

    match workspace.list(policy, &directory, pattern.as_ref()) {
        Ok(listing) => {
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
            (rendered, proposed_dir)
        }
        Err(e) => own_words(format!("error: {e}")),
    }
}

/// Write a file, after a person approves it.
///
/// The order matters: the user sees the exact path and body *before* any grant exists, and
/// the grant is issued only for what they saw. Issuing it earlier would mean approving a
/// value that could still change.
fn write_file<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    confirmer: &mut C,
    arguments: &Value,
) -> (Labelled<String>, String) {
    let Some(path) = argument(arguments, "path") else {
        return own_words("error: 'path' is required and must be a string");
    };
    let Some(contents) = argument(arguments, "contents") else {
        return own_words("error: 'contents' is required and must be a string");
    };

    // The path is routing, so naming a destination from it is not a content decision.
    let proposed_path = path.clone().into_parts_for_decoding().0;

    // The body is the model's words. Its integrity is that of the context the model was
    // working from, which the kernel tracked: nothing here upgrades anything.
    let (raw_body, _) = contents.into_parts_for_decoding();
    let body = policy.label_model_output("write_file", raw_body);
    let body_label = body.label();

    if policy.write_needs_approval(&proposed_path, body_label) {
        let existing = workspace.peek_for_review(&proposed_path);
        let request = WriteRequest {
            intent: if existing.is_some() {
                Intent::Overwrite
            } else {
                Intent::Create
            },
            existing,
            path: proposed_path.clone(),
            // Released for display only. Showing a person what is about to happen is the
            // point of asking, and a display release cannot feed an effect.
            contents: {
                let proof = policy.authorise_display_release("proposed write");
                body.clone().declassify(&proof)
            },
        };

        if confirmer.confirm_write(&request) == Decision::Reject {
            return own_words(format!(
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
            own_words(format!("wrote {proposed_path}"))
        }
        Err(e) => own_words(format!("error: {e}")),
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
) -> (Labelled<String>, String) {
    let Some(proposed) = argument(arguments, "path") else {
        return own_words("error: 'path' is required and must be a string");
    };
    let Some(old_text) = argument(arguments, "old_text") else {
        return own_words("error: 'old_text' is required and must be a string");
    };
    let Some(new_text) = argument(arguments, "new_text") else {
        return own_words("error: 'new_text' is required and must be a string");
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
        Err(denial) => return own_words(format!("refused: {denial}")),
    };

    let source = match workspace.read(policy, &path) {
        Ok(contents) => contents,
        Err(e) => return own_words(format!("error: {e}")),
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
        Err(denial) => return own_words(format!("refused: {denial}")),
    };

    let (old_text, _) = old_text.into_parts_for_decoding();
    let (new_text, _) = new_text.into_parts_for_decoding();

    let replaced = match crate::replace::replace(&current, &old_text, &new_text, replace_all) {
        Ok(r) => r,
        Err(e) => return own_words(format!("error: {e}")),
    };

    let (proposed_path, _) = proposed.into_parts_for_decoding();

    // The result is the model's edit applied to trusted text, so its integrity is that of the
    // context the model was working from.
    let body = policy.label_model_output("edit_file", replaced.contents);
    let body_label = body.label();

    if policy.write_needs_approval(&proposed_path, body_label) {
        let request = WriteRequest {
            path: proposed_path.clone(),
            contents: {
                let proof = policy.authorise_display_release("proposed edit");
                body.clone().declassify(&proof)
            },
            existing: Some(current.clone()),
            intent: Intent::Edit,
        };

        if confirmer.confirm_write(&request) == Decision::Reject {
            return own_words(format!(
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
            own_words(format!(
                "edited {proposed_path}: {occurrences} replacement(s)"
            ))
        }
        Err(e) => own_words(format!("error: {e}")),
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
) -> (Labelled<String>, String) {
    let Some(todos) = arguments.get("todos").and_then(Value::as_array) else {
        return own_words("error: 'todos' is required and must be an array");
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
        return own_words(
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

    (summary, String::new())
}

fn search<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    arguments: &Value,
) -> (Labelled<String>, String) {
    let Some(pattern) = argument(arguments, "pattern") else {
        return own_words("error: 'pattern' is required and must be a string");
    };
    let proposed_dir = argument(arguments, "directory").unwrap_or_else(|| {
        Labelled::new(".".to_string(), bua_core::label::Label::untrusted_public())
    });

    let pattern = match policy.promote_confined_read("search", "pattern", &pattern) {
        Ok(p) => p,
        Err(denial) => return own_words(format!("refused: {denial}")),
    };
    let directory = match policy.promote_confined_read("search", "directory", &proposed_dir) {
        Ok(d) => d,
        Err(denial) => return own_words(format!("refused: {denial}")),
    };

    let include = match argument(arguments, "include") {
        Some(proposed) => match policy.promote_confined_read("search", "include", &proposed) {
            Ok(p) => Some(p),
            Err(denial) => return own_words(format!("refused: {denial}")),
        },
        None => None,
    };

    let (proposed_where, _) = proposed_dir.into_parts_for_decoding();

    match workspace.grep(policy, &pattern, &directory, include.as_ref()) {
        Ok(found) => {
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
            (rendered, proposed_where)
        }
        Err(e) => own_words(format!("error: {e}")),
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
                "search"
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
            let (text, origin) = todo_write(&mut policy, &mut reporter, &arguments);
            // A task list has no destination, so there is nothing for an origin to name.
            assert!(origin.is_empty(), "a task list named an origin: {origin}");
            (reporter, text)
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
            .expect("policy");

            // An untrusted read drops the context, exactly as reading an unvouched file would.
            policy
                .observe_path(Capability::FileRead, "somewhere/nobody/vouched/for")
                .expect("observed");
            assert_eq!(policy.context_integrity(), Integrity::Untrusted);

            let mut reporter = RecordingReporter::default();
            let (text, _) = todo_write(
                &mut policy,
                &mut reporter,
                &list(&[("after reading something untrusted", "pending")]),
            );

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
