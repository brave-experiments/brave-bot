//! The tool set offered to the model, and dispatch for the calls it makes.
//!
//! Only non-destructive tools are exposed. A model-proposed path may become routing for
//! a confined read (see `Policy::promote_confined_read`), which is what makes iteration
//! possible; nothing here can change the workspace, so a wrong choice costs a step
//! rather than causing harm.
//!
//! Writing is present but gated differently. A write destination chosen by the model is
//! routing derived from whatever it just read, so it is never promoted on its own: the
//! user is shown the path and body and must approve, and that approval mints a single-use
//! endorsement bound to the exact path. A refusal, or a context where nobody can be asked,
//! means no write.

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
        Ok(files) => {
            let proof = policy.authorise_content_release("list_files", "paths");
            let files = files.declassify(&proof);
            if files.is_empty() {
                "(no files)".to_string()
            } else {
                files.join("\n")
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
        Ok(matches) => {
            let proof = policy.authorise_content_release("search", "matches");
            let matches = matches.declassify(&proof);
            if matches.is_empty() {
                "(no matches)".to_string()
            } else {
                matches
                    .iter()
                    .map(|m| format!("{}:{}: {}", m.path, m.line, m.text))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        Err(e) => format!("error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_set_is_reads_plus_a_gated_write() {
        let names: Vec<String> = available()
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        assert_eq!(
            names,
            vec!["read_file", "list_files", "write_file", "search"]
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

    /// The write tool must advertise that approval is required, so the model explains a
    /// change before proposing it.
    #[test]
    fn the_write_tool_states_that_approval_is_required() {
        let write = available()
            .into_iter()
            .find(|t| t.function.name == "write_file")
            .expect("write_file is offered");
        assert!(
            write.function.description.contains("approve"),
            "the description does not mention approval: {}",
            write.function.description
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
