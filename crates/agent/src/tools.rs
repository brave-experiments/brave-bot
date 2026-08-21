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
use bua_aichat::protocol::{Tool, ToolCall};
use bua_core::event::Sink;
use bua_core::policy::Policy;
use bua_core::value::Labelled;
use serde_json::{Value, json};

use crate::workspace::Workspace;

/// The tools the model may call.
pub fn available() -> Vec<Tool> {
    vec![
        Tool::function(
            "read_file",
            "Read a UTF-8 text file from the workspace. Returns its contents.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative path, e.g. src/main.rs"
                    }
                },
                "required": ["path"]
            }),
        ),
        Tool::function(
            "list_files",
            "List files in the workspace, recursively, under a directory.",
            json!({
                "type": "object",
                "properties": {
                    "directory": {
                        "type": "string",
                        "description": "Workspace-relative directory. Use \".\" for the root."
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
    /// Rendered result. Untrusted — it is workspace content — and released for the model
    /// to read, which is not an effect.
    pub text: String,
}

/// Run one tool call the model asked for.
///
/// Errors are returned as text rather than failing the turn: a model that asked for a
/// missing file should be told so and allowed to try again, exactly as it would be told
/// about a compile error.
pub fn dispatch<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    confirmer: &mut C,
    call: &ToolCall,
) -> Output {
    let name = call.function.name.clone();
    let arguments = match call.arguments() {
        Ok(value) => value,
        Err(e) => {
            return Output {
                call_id: call.id.clone(),
                tool: name,
                text: format!("error: the arguments were not valid JSON: {e}"),
            };
        }
    };

    let text = match name.as_str() {
        "read_file" => read_file(policy, workspace, &arguments),
        "list_files" => list_files(policy, workspace, &arguments),
        "search" => search(policy, workspace, &arguments),
        "write_file" => write_file(policy, workspace, confirmer, &arguments),
        "edit_file" => edit_file(policy, workspace, confirmer, &arguments),
        other => format!("error: no such tool '{other}'"),
    };

    Output {
        call_id: call.id.clone(),
        tool: name,
        text,
    }
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
) -> String {
    let Some(proposed) = argument(arguments, "path") else {
        return "error: 'path' is required and must be a string".to_string();
    };

    let path = match policy.promote_confined_read("read_file", "path", &proposed) {
        Ok(p) => p,
        Err(denial) => return format!("refused: {denial}"),
    };

    match workspace.read(policy, &path) {
        Ok(contents) => {
            let proof = policy.authorise_content_release("read_file", "contents");
            contents.declassify(&proof)
        }
        Err(e) => format!("error: {e}"),
    }
}

fn list_files<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    arguments: &Value,
) -> String {
    let proposed = argument(arguments, "directory").unwrap_or_else(|| {
        Labelled::new(".".to_string(), bua_core::label::Label::untrusted_public())
    });

    let directory = match policy.promote_confined_read("list_files", "directory", &proposed) {
        Ok(d) => d,
        Err(denial) => return format!("refused: {denial}"),
    };

    match workspace.list(policy, &directory) {
        Ok(listing) => {
            let proof = policy.authorise_content_release("list_files", "paths");
            let listing = listing.declassify(&proof);
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
        }
        Err(e) => format!("error: {e}"),
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
) -> String {
    let Some(path) = argument(arguments, "path") else {
        return "error: 'path' is required and must be a string".to_string();
    };
    let Some(contents) = argument(arguments, "contents") else {
        return "error: 'contents' is required and must be a string".to_string();
    };

    // Reading the proposed values to show a person is not a decision the agent makes on
    // the model's behalf, so it does not need a gate — but it must not be fed back into
    // the flow except through the approval below.
    let proposed_path = path.clone().into_parts_for_decoding().0;
    let proposed_body = contents.clone().into_parts_for_decoding().0;

    let existing = workspace.peek_for_review(&proposed_path);
    let request = WriteRequest {
        intent: if existing.is_some() {
            Intent::Overwrite
        } else {
            Intent::Create
        },
        existing,
        path: proposed_path.clone(),
        contents: proposed_body,
    };

    if confirmer.confirm_write(&request) == Decision::Reject {
        return format!(
            "refused: the user did not approve writing {proposed_path}. Do not retry \
             the same write; ask what they would prefer."
        );
    }

    // The approval is what makes the path trusted, and it is bound to this exact value.
    policy.issue_grant("file_write", "path", proposed_path.clone());

    match workspace.write_endorsed(policy, &path, &contents) {
        Ok(_) => format!("wrote {proposed_path}"),
        Err(e) => format!("error: {e}"),
    }
}

/// Replace an exact passage in a file, after a person approves the diff.
///
/// Same endorsement shape as [`write_file`] — the model never decides a write destination —
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
) -> String {
    let Some(proposed) = argument(arguments, "path") else {
        return "error: 'path' is required and must be a string".to_string();
    };
    let Some(old_text) = argument(arguments, "old_text") else {
        return "error: 'old_text' is required and must be a string".to_string();
    };
    let Some(new_text) = argument(arguments, "new_text") else {
        return "error: 'new_text' is required and must be a string".to_string();
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
        Err(denial) => return format!("refused: {denial}"),
    };

    let current = match workspace.read(policy, &path) {
        Ok(contents) => {
            let proof = policy.authorise_content_release("edit_file", "contents");
            contents.declassify(&proof)
        }
        Err(e) => return format!("error: {e}"),
    };

    let (old_text, _) = old_text.into_parts_for_decoding();
    let (new_text, _) = new_text.into_parts_for_decoding();

    let replaced = match crate::replace::replace(&current, &old_text, &new_text, replace_all) {
        Ok(r) => r,
        Err(e) => return format!("error: {e}"),
    };

    let (proposed_path, _) = proposed.into_parts_for_decoding();
    let request = WriteRequest {
        path: proposed_path.clone(),
        contents: replaced.contents.clone(),
        existing: Some(current.clone()),
        intent: Intent::Edit,
    };

    if confirmer.confirm_write(&request) == Decision::Reject {
        return format!(
            "refused: the user did not approve editing {proposed_path}. Do not retry the \
             same edit; ask what they would prefer."
        );
    }

    policy.issue_grant("file_write", "path", proposed_path.clone());

    let body = Labelled::new(
        replaced.contents,
        bua_core::label::Label::untrusted_public(),
    );
    match workspace.write_endorsed_if_unchanged(policy, &path, &body, &current) {
        Ok(_) => format!(
            "edited {proposed_path}: {} replacement(s)",
            replaced.occurrences
        ),
        Err(e) => format!("error: {e}"),
    }
}

fn search<S: Sink>(policy: &mut Policy<'_, S>, workspace: &Workspace, arguments: &Value) -> String {
    let Some(pattern) = argument(arguments, "pattern") else {
        return "error: 'pattern' is required and must be a string".to_string();
    };
    let proposed_dir = argument(arguments, "directory").unwrap_or_else(|| {
        Labelled::new(".".to_string(), bua_core::label::Label::untrusted_public())
    });

    let pattern = match policy.promote_confined_read("search", "pattern", &pattern) {
        Ok(p) => p,
        Err(denial) => return format!("refused: {denial}"),
    };
    let directory = match policy.promote_confined_read("search", "directory", &proposed_dir) {
        Ok(d) => d,
        Err(denial) => return format!("refused: {denial}"),
    };

    match workspace.grep(policy, &pattern, &directory) {
        Ok(found) => {
            let proof = policy.authorise_content_release("search", "matches");
            let found = found.declassify(&proof);
            if found.matches.is_empty() {
                "(no matches)".to_string()
            } else {
                let rendered = found
                    .matches
                    .iter()
                    .map(|m| format!("{}:{}: {}", m.path, m.line, m.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                if found.truncated {
                    // Without this a model that gets exactly the cap concludes it has
                    // every occurrence, which is how a rename misses call sites.
                    format!(
                        "{rendered}\n\n(this search stopped at {} matches and is \
                         incomplete; narrow the pattern or search a subdirectory)",
                        found.matches.len()
                    )
                } else {
                    rendered
                }
            }
        }
        Err(e) => format!("error: {e}"),
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
                "search"
            ]
        );
    }

    /// Command execution stays absent. Unlike a write, a command has no separable routing
    /// field to endorse — the string is destination and payload at once — so there is
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
}
