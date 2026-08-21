//! The tool set offered to the model, and dispatch for the calls it makes.
//!
//! Only non-destructive tools are exposed. A model-proposed path may become routing for
//! a confined read (see `Policy::promote_confined_read`), which is what makes iteration
//! possible; nothing here can change the workspace, so a wrong choice costs a step
//! rather than causing harm.
//!
//! Writing is deliberately absent. A write destination chosen by the model would be
//! routing derived from whatever it just read, which is the attack this system exists to
//! prevent. Writes stay with the user, through the `--write` path.

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
pub fn dispatch<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
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
    fn the_tool_set_is_read_only() {
        let names: Vec<String> = available()
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        assert_eq!(names, vec!["read_file", "list_files", "search"]);
        // No write, exec, or fetch tool: a model-chosen destination for any of those
        // would be routing derived from untrusted content.
        for name in &names {
            assert!(!name.contains("write"));
            assert!(!name.contains("exec"));
            assert!(!name.contains("shell"));
        }
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
