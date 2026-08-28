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
use bravebot_aichat::protocol::{Tool, ToolCall, Usage};
use bravebot_core::ask::{self, Choice, Question, Series};
use bravebot_core::event::{Role, Sink};
use bravebot_core::label::Label;
use bravebot_core::policy::{Destination, Policy};
use bravebot_core::slot::{SlotId, SlotStore};
use bravebot_core::todo::{self, Item, List, Status};
use bravebot_core::value::Labelled;
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
             from. Name the file with path, or with path_ref where a listing gave you a \
             reference instead of a name.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative path, e.g. src/main.rs. Give this \
                                        or path_ref, never both."
                    },
                    "path_ref": {
                        "type": "string",
                        "description": "A reference to a file whose name you were not shown, \
                                        e.g. \"ref:2\" from a listing. Only useful where that \
                                        file is one you may be shown: a reference to a \
                                        quarantined file already is the file, so reading it \
                                        returns nothing you do not have."
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
                "required": []
            }),
        ),
        Tool::function(
            "list_files",
            "List files in the workspace, recursively, under a directory. Give a glob \
             pattern to narrow the result rather than listing everything. In a directory you \
             may not read, the names are quarantined and you get one reference per file \
             instead: use those as path_ref to read a file, process it, and write it back, \
             without ever being told what it is called.",
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
            "Write a UTF-8 text file in the workspace. Name the destination with path, or with \
             path_ref to write back to a file a listing gave you a reference to. Give either \
             the contents or a reference to quarantined content that becomes the contents. The \
             user must approve each write before it happens, so explain what you are changing; \
             a write to a path_ref is always shown, since the user is the only one who sees \
             which file it is.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative path, e.g. src/main.rs. Give this \
                                        or path_ref, never both."
                    },
                    "path_ref": {
                        "type": "string",
                        "description": "A reference to the file to write, e.g. \"ref:2\". Use \
                                        the reference a listing gave you to write back to a \
                                        file whose name you were never shown."
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
                "required": []
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
                        "description": "Workspace-relative path, e.g. src/main.rs. Give this \
                                        or path_ref, never both."
                    },
                    "path_ref": {
                        "type": "string",
                        "description": "A reference to the file to edit, e.g. \"ref:2\". Only \
                                        useful where that file is trusted, since an edit \
                                        locates a passage and that means reading it."
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
                "required": ["old_text", "new_text"]
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
             change a file you cannot see: name the file's reference, say what has to be true \
             of it afterwards, then pass the reference that comes back to write_file as \
             contents_ref. You are not shown its output either, and nobody reads it before it \
             is written, so be exact about the shape of the answer: the complete document and \
             nothing else. What the change should be is its decision, not yours. It reads the \
             whole document, so it can work out what is wrong and where the fix goes, and leave \
             a file alone if it turns out not to be the one you are after.",
            json!({
                "type": "object",
                "properties": {
                    "reads": {
                        "type": "array",
                        "description": "The references to give it, e.g. [\"ref:0\", \"ref:1\"]. \
                                        At least one, and usually every reference that bears on \
                                        the task: the input names each block by its reference, \
                                        so one that can see the whole set can tell which file is \
                                        which. It still returns one document.",
                        "items": {"type": "string"}
                    },
                    "about": {
                        "type": "string",
                        "description": "Which of the references in reads this call is about. \
                                        The answer replaces that document and may be written to \
                                        no other file, and where nothing should change that \
                                        document stands as the answer, so the processor says so \
                                        in a word rather than reproducing a file it was told to \
                                        leave alone. Required when reads names more than one \
                                        file: one answer has one destination, and nothing else \
                                        can say which."
                    },
                    "instruction": {
                        "type": "string",
                        "description": "What to do with them and what to produce. Where the \
                                        result is going into a file, ask for the whole document \
                                        and nothing else: no explanation, no summary, no code \
                                        fence. Whatever comes back is what gets written. Give \
                                        the symptom the user reported and ask for the cause to \
                                        be found; naming a remedy you have not verified makes \
                                        it apply your guess rather than diagnose. Include the \
                                        file's name and language if that matters, because the \
                                        processor knows nothing but what you tell it and what \
                                        the references hold. May be conditional: say what the \
                                        document must look like if it is the one the task is \
                                        about, and to return it unchanged if it is not."
                    }
                },
                "required": ["reads", "instruction"]
            }),
        ),
        Tool::function(
            "load_skill",
            "Read one of the skills listed for you. A skill is instructions the user wrote for a              kind of task. Load one before doing that kind of work, and follow what it says.              Only the names you were listed exist; there is no path to give and nothing else to              browse.",
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The name of a skill exactly as it was listed for you,                                         e.g. commit-style."
                    }
                },
                "required": ["name"]
            }),
        ),
        Tool::function(
            "ask_user",
            "Ask the user up to four questions and wait for their answers. Only for what you \
             cannot find out yourself: which of two approaches to take, whether something is in \
             scope, which of two plausible files they meant. Never for a fact about this machine. \
             A path, a filename, whether a program is installed, what something is called: go and \
             look with list_files, search, read_file or run instead, and note that a quarantined \
             result does not stop you asking afterwards, so looking first costs you nothing. Ask \
             everything the plan turns on in one call rather than a question per turn; they are \
             put to the user one at a time. Offer concrete options where you can; the user may \
             also answer in their own words or skip a question, and a skipped question is an \
             answer to work with rather than a reason to ask again.",
            json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "description": "The questions to put, at most four. A limit rather than \
                                        a target: ask what the work turns on and no more.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "header": {
                                    "type": "string",
                                    "description": "Two or three words naming what this asks \
                                                    about, shown as a tag beside it, e.g. \
                                                    \"Cache layer\". It is how the user tells \
                                                    one question from the next."
                                },
                                "question": {
                                    "type": "string",
                                    "description": "The question, in one sentence."
                                },
                                "options": {
                                    "type": "array",
                                    "description": "The choices to offer. Give them here rather \
                                                    than listing them inside the question text: \
                                                    only these are shown as choices. Each may \
                                                    be a plain string, or an object with a \
                                                    'label'. The user can always answer in \
                                                    their own words instead, so there is no \
                                                    need to offer an 'other'. Omit only for a \
                                                    question that genuinely has no set answers.",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "label": {
                                                "type": "string",
                                                "description": "The option, in a few words."
                                            },
                                            "detail": {
                                                "type": "string",
                                                "description": "Optional line saying what \
                                                                choosing it means."
                                            }
                                        },
                                        "required": ["label"]
                                    }
                                },
                                "multiple": {
                                    "type": "boolean",
                                    "description": "Set true when this question asks for more \
                                                    than one answer, such as \"which of these\" \
                                                    or \"pick any that apply\". Left false the \
                                                    user can pick only one. Defaults to false."
                                }
                            },
                            "required": ["header", "question"]
                        }
                    }
                },
                "required": ["questions"]
            }),
        ),
        Tool::function(
            "run",
            "Run a program. Give a pipeline of stages, each a program name and a list of \
             arguments; each stage's output feeds the next. There is no shell, so there are no \
             pipes, no redirection, no && and no $(...): a character like ; or | inside an \
             argument is part of that argument and nothing splits it. Compose stages instead of \
             reaching for a pipe. The user approves the exact arguments before anything runs, so \
             say what you are running and why first. You will NOT be shown the output: it comes \
             back as a reference, like a file you may not read, and you can pass that reference \
             to spawn_processor or write it to a file with write_file. Do not run a program to \
             read something you could read with read_file or search.",
            json!({
                "type": "object",
                "properties": {
                    "pipeline": {
                        "type": "array",
                        "description": "The stages, in order. One entry runs one program; two \
                                        entries run the first and feed its output to the second.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "program": {
                                    "type": "string",
                                    "description": "The program to run, e.g. \"git\". A name is \
                                                    looked up on PATH; a path is taken relative \
                                                    to the workspace. Never a command line, and \
                                                    never a shell."
                                },
                                "args": {
                                    "type": "array",
                                    "description": "What comes AFTER the program, one argument \
                                                    per entry. Do not repeat the program name \
                                                    here: this is not an argv vector and there is \
                                                    no argv[0]. For `git log --oneline -50` the \
                                                    program is \"git\" and args are [\"log\", \
                                                    \"--oneline\", \"-50\"]. Split them the \
                                                    way a shell would have, since nothing here \
                                                    splits a string for you: never [\"log \
                                                    --oneline -50\"] as one entry.",
                                    "items": {"type": "string"}
                                }
                            },
                            "required": ["program"]
                        }
                    }
                },
                "required": ["pipeline"]
            }),
        ),
        Tool::function(
            "read_output",
            "Ask to be shown what a command printed. Give the reference a run handed back. The \
             user sees the output and decides; if they agree, it comes back to you as text you \
             can read. Use it whenever you ran something to find something out, which is most of \
             the time: a run's output is quarantined by default, so `which`, `find`, `uname` and \
             the like tell you nothing until you ask for the result. Ask for the errors too when \
             a run fails, or you will not know why it failed and must not claim it succeeded. \
             Only for output from run; a quarantined file is not readable this way.",
            json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "The reference a run gave you, e.g. \"ref:5\"."
                    }
                },
                "required": ["ref"]
            }),
        ),
    ]
}

/// A read a tool decided not to perform yet.
///
/// The planner asked for a file whose contents it may not see, so there is nothing to show it
/// and no reason to have the bytes in hand. What travels back is the path and the size, and the
/// slot the turn reserves from them holds the file until something needs it.
///
/// The path is the promoted one, still labelled, because the kernel checks it as routing again
/// when it reserves the slot.
#[derive(Debug, Clone)]
pub struct Deferral {
    pub path: Labelled<String>,
    /// What the planner is told the reference is of. The path itself where the planner named
    /// it, since it is its own words coming back.
    pub origin: String,
    pub bytes: usize,
}

/// A listing the planner may not read, reserved one reference per entry.
///
/// The names stay wrapped: this crate carries them from the lister to the kernel and never
/// looks. `count` is the number of them, which is released to the planner and the person alike,
/// because how many files a directory holds is shape rather than content.
#[derive(Debug)]
pub struct Entries {
    /// What to call each one to the planner, naming the directory and never the file.
    pub origin: String,
    pub paths: Labelled<Vec<String>>,
    pub count: usize,
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
    /// A file this call reserved rather than read. `text` is empty where this is set: there is
    /// nothing to present, and the turn reserves the slot instead.
    pub deferred: Option<Deferral>,
    /// The entries of a listing this call reserved, one reference each.
    pub entries: Option<Entries>,
    /// The slot this result stands for unchanged, where there is one.
    pub unchanged_from: Option<SlotId>,
    /// Which document a processor's answer is about, where it produced one.
    pub answers_for: Option<Option<SlotId>>,
    /// What an isolated processor said about what it did. For the person watching only.
    pub said: Option<Labelled<String>>,
    /// Whether the text is workspace content rather than the driver's own words about the call.
    pub content: bool,
    /// What the call spent at the model, where it called one.
    ///
    /// Zero for every tool but the processor. A turn that reported only its own rounds would
    /// understate what it cost by however much its processors wrote.
    pub usage: Usage,
    /// The command whose output this is, where a run produced it.
    ///
    /// Recorded on the slot by the turn loop, since only a slot minted from a command may be
    /// offered to the user for reading.
    pub printed_by: Option<String>,
}

/// Everything a tool works with that is not the policy.
///
/// Three things, and the second two are new because of the processor: the quarantine, so a
/// reference the planner names resolves to something, and the model, so an isolated processor
/// has somewhere to run. Bundled rather than passed one by one because dispatch would otherwise
/// take seven arguments to give two tools what they need.
pub struct Tools<'a> {
    pub workspace: &'a Workspace,
    /// The skills this turn found, which the planner selects from by name.
    pub skills: &'a crate::skills::Catalogue,
    /// Where quarantined content lives, by the names the planner was given for it.
    pub slots: &'a mut SlotStore,
    /// The model an isolated processor runs on.
    pub chat: Chat<'a>,
    /// The turn's stop token, so a slow program does not have to be waited out.
    pub cancel: &'a bravebot_core::cancel::Cancel,
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
    /// Set by a read that reserved a file instead of opening it.
    deferred: Option<Deferral>,
    /// Set by a listing the planner may not read.
    entries: Option<Entries>,
    /// The change a write made, for showing under the line it belongs to.
    changes: Vec<crate::diff::Change>,
    /// Whether those lines are content nobody vouched for.
    untrusted: bool,
    /// The slot this result stands for unchanged, where a processor said a document should not
    /// change. The turn records it against the slot it mints, so a write of it can be
    /// recognised as changing nothing.
    unchanged_from: Option<SlotId>,
    /// Which document a processor's answer is about, where it produced one.
    ///
    /// `Some(None)` is a processor that was given several documents and told which of them it
    /// was about by nobody: its answer is for no file in particular, and the turn records that
    /// so a write of it is refused rather than guessed at.
    answers_for: Option<Option<SlotId>>,
    /// What an isolated processor said about what it did, for the person watching.
    ///
    /// Never part of `text`, which is what the planner is told about: this half of a processor's
    /// answer reaches a screen and stops there.
    said: Option<Labelled<String>>,
    /// Whether `text` is workspace content rather than the driver's own words about the call.
    ///
    /// What the kernel does with a result is worth reporting only where the result is content:
    /// saying "the model has read it" about a sentence the driver wrote to explain a refusal is
    /// true, useless, and read by the person as a claim about their file.
    content: bool,
    /// What the tool spent at the model. Only a processor spends anything.
    usage: Usage,
    /// The command whose output this is, where a run produced it.
    ///
    /// Recorded on the slot by the turn loop, because only a slot minted from a command may be
    /// offered to the user for reading.
    printed_by: Option<String>,
}

impl Produced {
    /// A result the person watching is told about in the driver's own summary of it.
    fn new(text: Labelled<String>, origin: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            text,
            origin: origin.into(),
            note: note.into(),
            failed: false,
            deferred: None,
            entries: None,
            changes: Vec::new(),
            untrusted: false,
            unchanged_from: None,
            answers_for: None,
            said: None,
            content: false,
            usage: Usage::default(),
            printed_by: None,
        }
    }

    /// Say that what this produced is workspace content, not the driver's words about it.
    fn of_content(mut self) -> Self {
        self.content = true;
        self
    }

    /// A read that reserved a file rather than opening it.
    fn deferring(path: Labelled<String>, origin: String, bytes: usize) -> Self {
        Self::new(
            Labelled::trusted(String::new()),
            origin.clone(),
            format!("{bytes} bytes, quarantined"),
        )
        .with_deferral(Deferral {
            path,
            origin,
            bytes,
        })
    }

    fn with_deferral(mut self, deferral: Deferral) -> Self {
        self.deferred = Some(deferral);
        self
    }

    /// A listing whose entries were reserved rather than shown.
    fn with_entries(mut self, entries: Entries) -> Self {
        self.entries = Some(entries);
        self
    }

    fn with_changes(mut self, changes: Vec<crate::diff::Change>) -> Self {
        self.changes = changes;
        self
    }

    /// Say that the change came from content nobody vouched for.
    fn marked_untrusted(mut self, untrusted: bool) -> Self {
        self.untrusted = untrusted;
        self
    }

    fn costing(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self
    }
}

/// A tool name without the group some models put in front of it.
///
/// Only the one prefix, and only where something is left after it: this is for a name that means
/// one of ours, not a general invitation to guess.
fn strip_namespace(name: &str) -> &str {
    for prefix in ["functions.", "functions_"] {
        if let Some(rest) = name.strip_prefix(prefix)
            && !rest.is_empty()
        {
            return rest;
        }
    }
    name
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
        // Named for what it is rather than for what it does: every one of these is a model
        // with no tools, no memory and one round, and a person watching a line go by should not
        // have to remember which of the verbs meant that.
        "spawn_processor" => "Isolated processor",
        "load_skill" => "Skill",
        "ask_user" => "Ask",
        "run" => "Run",
        "read_output" => "Read output",
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
        "load_skill" => Some("name"),
        _ => None,
    }
}

/// What a call is about, for the line shown while it runs.
///
/// Which argument names the target depends on the tool, so the key is chosen from the tool's
/// own name. The value is the model's word for it, released to a screen and nowhere else: it
/// goes on a line a person reads and is never compared, matched, or routed anywhere.
fn target_of<S: Sink>(
    policy: &mut Policy<'_, S>,
    tool: &str,
    slots: &SlotStore,
    arguments: &Value,
) -> String {
    // A question is about nothing in the workspace, and its subject is the question itself,
    // which the person is about to read anyway. How many were asked is the useful thing on a
    // line that goes by while they answer, and a count is structure rather than content, so
    // nothing is released to say it.
    if tool == "ask_user" {
        return match arguments.get("questions").and_then(Value::as_array) {
            Some(asked) if asked.len() > 1 => tally(asked.len(), "question", "questions"),
            _ => String::new(),
        };
    }

    // A processor has no single argument naming a target: what it is working on is the set of
    // references it was given, which are names the driver handed out and can read back.
    let named = if tool == "spawn_processor" {
        let proof = policy.authorise_display_release("what a tool is working on");
        references_in(arguments).declassify(&proof)
    } else {
        let Some(key) = target_key(tool) else {
            return String::new();
        };

        // A call that named a reference instead of a path says so with the reference, which is
        // then resolved below: it is the planner that cannot know the name, not the person.
        let key = if arguments.get(key).is_none() && arguments.get("path_ref").is_some() {
            "path_ref"
        } else {
            key
        };

        match argument(arguments, key) {
            Some(value) => {
                let proof = policy.authorise_display_release("what a tool is working on");
                value.declassify(&proof)
            }
            None => return String::new(),
        }
    };

    // The line a person reads says which file, always. A reference means something to the
    // planner and nothing at all to the person watching their own workspace being worked on.
    if named.contains("ref:") {
        return name_references(&named, &policy.names_for_display(slots));
    }
    named
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
fn tally(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Unchanged lines kept around each run of changes, so a hunk can be read in context.
const DIFF_CONTEXT: usize = 3;

/// How many lines of a quarantined file are shown when offering to vouch for it.
///
/// Enough to tell what the file is, not so much that the prompt becomes a document nobody reads.
/// The decision being asked for is about the path, not about these lines.
const VOUCH_PREVIEW: usize = 20;

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
fn change_report(
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
    // Some models namespace a tool by the group it was offered in: "functions_todo_write" and
    // "functions.todo_write" both mean todo_write, and answering "no such tool" to those spends
    // a round on a typo of our own making. The prefix is stripped before the name is matched
    // against the table, which is the driver's own list of literals: nothing the model writes
    // reaches anything but that comparison.
    let name = strip_namespace(&call.function.name).to_string();
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
                deferred: produced.deferred,
                entries: produced.entries,
                unchanged_from: produced.unchanged_from,
                answers_for: produced.answers_for,
                said: produced.said,
                content: produced.content,
                usage: produced.usage,
                printed_by: produced.printed_by,
            };
        }
    };

    // Announced before the call runs, so a slow one is visible while it is slow. This is the
    // difference between a turn that looks stuck and one that is plainly working.
    let target = target_of(policy, &name, tools.slots, &arguments);
    reporter.tool_started(Activity::running(verb, target.clone()));

    let produced = match name.as_str() {
        "read_file" => read_file(policy, tools.workspace, tools.slots, confirmer, &arguments),
        "list_files" => list_files(policy, tools.workspace, &arguments),
        "search" => search(policy, tools.workspace, &arguments),
        "write_file" => write_file(policy, tools, confirmer, &arguments),
        "edit_file" => edit_file(policy, tools.workspace, tools.slots, confirmer, &arguments),
        "todo_write" => todo_write(policy, reporter, tools.slots, &arguments),
        "spawn_processor" => spawn_processor(policy, tools, &arguments),
        "load_skill" => load_skill(policy, tools.skills, &arguments),
        "ask_user" => ask_user(policy, confirmer, &arguments),
        "run" => run(policy, tools, confirmer, &arguments),
        "read_output" => read_output(policy, tools, confirmer, &arguments),
        other => problem(format!("error: no such tool '{other}'")),
    };

    let finished = Activity::running(verb, target)
        .with_changes(produced.changes)
        .marked_untrusted(produced.untrusted);
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
        deferred: produced.deferred,
        entries: produced.entries,
        unchanged_from: produced.unchanged_from,
        answers_for: produced.answers_for,
        said: produced.said,
        content: produced.content,
        usage: produced.usage,
        printed_by: produced.printed_by,
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
        deferred: None,
        entries: None,
        changes: Vec::new(),
        untrusted: false,
        unchanged_from: None,
        answers_for: None,
        said: None,
        content: false,
        usage: Usage::default(),
        printed_by: None,
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
        bravebot_core::label::Label::untrusted_public(),
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
    Labelled::new(
        names.join(", "),
        bravebot_core::label::Label::untrusted_public(),
    )
}

fn read_file<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    slots: &SlotStore,
    confirmer: &mut C,
    arguments: &Value,
) -> Produced {
    let found = match path_argument(policy, "read_file", Purpose::Read, slots, arguments) {
        Ok(found) => found,
        Err(refusal) => return problem(refusal),
    };
    // What the reference that comes back is said to be of. The planner's own path where it
    // typed one, and the reference's name where it did not: a read through a reference must not
    // hand back the filename the reference exists to hold.
    let (proposed, destination, shown_path) = (found.path, found.destination, found.shown);

    let path = match destination {
        // The promotion the model's own choice of file already gets: the read is confined to the
        // workspace and changes nothing in it.
        Destination::Named => match policy.promote_confined_read("read_file", "path", &proposed) {
            Ok(p) => p,
            Err(denial) => return problem(format!("refused: {denial}")),
        },
        // Already promoted, by the gate that took the name out of the reference. Promoting it
        // again would work and would record that the model proposed a path it never saw.
        Destination::Reference => proposed.clone(),
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

    // A file the planner may not see need not be opened yet. Whether it may see it is a
    // question about the trust map, keyed by the path it named, so nothing about any file's
    // contents reaches this branch. A page of a file is a different request: the offset and the
    // limit describe a slice, and there is nothing to slice until something reads it, so those
    // are still read now.
    let whole_file = arguments.get("offset").is_none() && arguments.get("limit").is_none();

    // The trust question, put where it bites rather than only at startup. A file is quarantined
    // because nobody vouched for it, and that is the user's decision to make: they are shown the
    // path and the first lines of it and can vouch on the spot, after which this read and every
    // later one sees the file. A yes writes the same rule `@` and the startup question write, so
    // nothing here is a second route to trusting content.
    //
    // Asked once per path per turn, and only for a path that is quarantined, so a planner retrying
    // a read does not put the same question up twice.
    if policy.should_offer_vouch(&proposed_path) {
        let (preview, truncated) = match workspace.peek_for_review(&proposed_path) {
            Some(body) => {
                let head: Vec<&str> = body.lines().take(VOUCH_PREVIEW).collect();
                let truncated = body.lines().count() > head.len();
                (head.join("\n"), truncated)
            }
            // Nothing to show means nothing to vouch about: a path that cannot be read is reported
            // by the read below, not turned into a question.
            None => (String::new(), false),
        };
        if !preview.is_empty() {
            let request = crate::confirm::VouchRequest {
                path: proposed_path.clone(),
                preview,
                truncated,
            };
            if confirmer.confirm_vouch(&request) == Decision::Approve {
                policy.vouch_for_named_path(&proposed_path);
            }
        }
    }

    // A reference to a file the planner may not read already is that file, so reading it has
    // nothing to hand back but another name for the same thing, which reads as the read having
    // failed. One planner went four references deep before giving up. Nothing to do but say so.
    if whole_file
        && destination == Destination::Reference
        && policy.read_is_quarantined(&proposed_path)
    {
        return confirmed(
            format!(
                "{shown_path} already names that file, and nothing will show you what is in \
                 it. Give {shown_path} to spawn_processor to work on, and name {shown_path} as \
                 path_ref to write what comes back to the same file. If the work would go better \
                 with you reading it yourself, say so in your reply: the user can vouch for the \
                 file, and then you will be shown it. They know which file {shown_path} is even \
                 though you do not."
            ),
            format!("nothing to read: {shown_path} already holds it"),
        );
    }

    if whole_file && policy.read_is_quarantined(&proposed_path) {
        return match workspace.survey(&proposed_path) {
            Ok(bytes) => Produced::deferring(path, shown_path, bytes).of_content(),
            // A path that names nothing is said so now, exactly as an eager read would have.
            Err(e) => problem(format!("error: {e}")),
        };
    }

    match workspace.read_page(policy, &path, offset, limit) {
        Ok(page) => {
            // Reshaped inside the kernel, so the driver never holds the text. Only
            // `Policy::present` decides whether the planner sees what comes out.
            let note = note_for(policy, "read_file", &page, |p| {
                tally(p.lines.len(), "line", "lines")
            });
            let rendered = policy.render_in_place("read_file", &page, |p| render_page(&p));
            Produced::new(rendered, shown_path, note).of_content()
        }
        Err(e) => problem(format!("error: {e}")),
    }
}

/// The text a slot holds for a file, read at the moment something needs it.
///
/// The same shaping an eager read would have applied, from the same two functions, so a
/// deferred read and an immediate one put the same bytes in the same slot. Deferring changes
/// when a file is read and nothing else about it.
pub(crate) fn read_into_slot(workspace: &Workspace, path: &str) -> Result<String, String> {
    workspace
        .page(path, 1, usize::MAX)
        .map(|page| {
            let mut text = render_page(&page);
            // A file that went through a slot used to come back a byte shorter than it went in,
            // because the lines are joined with newlines between them and none after. Every
            // processed file lost its last newline, which the next diff anybody reads calls
            // "no newline at end of file".
            if page.ends_with_newline && !text.ends_with('\n') {
                text.push('\n');
            }
            text
        })
        .map_err(|e| e.to_string())
}

/// Read the files any of these slots is still waiting on.
///
/// Called by every consumer of a slot before it asks for the bytes. Doing nothing where the
/// slots hold their contents already, so a consumer need not know whether an earlier one got
/// there first.
pub(crate) fn materialise<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    slots: &mut SlotStore,
    tool: &str,
    wanted: &[SlotId],
) -> Result<Vec<String>, String> {
    // The files this actually opened, for the line the person reads. A read deferred until a
    // processor needed it is still a read of their workspace, and until it was reported the only
    // reads on the screen were the planner's, which are the ones that read nothing.
    let mut opened = Vec::new();
    for slot in wanted {
        let was_unread = slots.deferred(slot).is_some();
        policy
            .materialise(tool, slot, slots, |path| read_into_slot(workspace, path))
            .map_err(|denial| format!("refused: {denial}"))?;
        if was_unread {
            opened.push(slot.clone());
        }
    }

    if opened.is_empty() {
        return Ok(Vec::new());
    }
    let named = policy.names_for_display(slots);
    Ok(opened
        .iter()
        .map(|slot| {
            named
                .iter()
                .find(|(id, _, _)| id == slot)
                .map(|(slot, label, path)| format!("{slot}{label}:{path}"))
                .unwrap_or_else(|| slot.to_string())
        })
        .collect())
}

/// The path a call is about, from `path` or from a reference to a file.
///
/// A planner working in a directory it may not read has no filename to type, so it names the
/// reference the listing gave it instead. What comes back is the same in both cases: the path,
/// and how it was arrived at, which is what decides whether an approval can be skipped.
fn path_argument<S: Sink>(
    policy: &mut Policy<'_, S>,
    tool: &'static str,
    purpose: Purpose,
    slots: &SlotStore,
    arguments: &Value,
) -> Result<PathArgument, String> {
    let named = argument(arguments, "path");
    let referenced = argument(arguments, "path_ref");

    match (named, referenced) {
        (Some(_), Some(_)) => Err(
            "error: give 'path' or 'path_ref', not both. Use path_ref alone for a file you \
             were never shown the name of."
                .to_string(),
        ),
        (None, None) => Err("error: one of 'path' or 'path_ref' is required".to_string()),
        (Some(path), None) => {
            let shown = path.clone().into_parts_for_decoding().0;
            Ok(PathArgument {
                path,
                destination: Destination::Named,
                shown,
            })
        }
        (None, Some(reference)) => {
            let slot = policy
                .accept_reference(tool, "path_ref", &reference)
                .map_err(|denial| format!("refused: {denial}"))?;
            // Which gate the name comes out of is decided by what this call will do with it: a
            // read may promote it, an effect may not and needs a person instead. Asking the
            // wrong one is not possible from here, because the caller says which it is.
            let path = match purpose {
                Purpose::Read => policy
                    .promote_reference_for_read(tool, "path_ref", &slot, slots)
                    .map_err(|denial| format!("refused: {denial}"))?,
                Purpose::Effect => {
                    let path = policy
                        .destination_from_reference(tool, "path_ref", &slot, slots)
                        .map_err(|denial| format!("refused: {denial}"))?;
                    // Untrusted and public, which is what a name out of a directory nobody
                    // vouched for is. The endorsement is what will authorise it, not its label.
                    Labelled::new(path, bravebot_core::label::Label::untrusted_public())
                }
            };
            Ok(PathArgument {
                path,
                destination: Destination::Reference,
                shown: slot.to_string(),
            })
        }
    }
}

/// What a call is going to do with the path it asked for.
///
/// The two take different routes out of a reference, and neither is reachable by asking for the
/// other: a read is promoted, and an effect is not promoted at all but endorsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Read,
    Effect,
}

/// A path a call is about, and what the planner may be told about it.
///
/// `shown` is the difference. A path the planner typed is its own words coming back, and a path
/// out of a reference is a name it has never seen: saying it in a result would hand over the
/// thing the reference exists to keep, so what goes back is the reference's own name. The person
/// watching is told the real path either way, on the line under it and in the approval.
struct PathArgument {
    path: Labelled<String>,
    destination: Destination,
    shown: String,
}

/// Put the file a reference names into a line a person is about to read.
///
/// Literal matching, not a pattern: the names are `ref:0`, `ref:1` and so on, the driver handed
/// them out itself, and a regular expression over text a model wrote is attack surface for no
/// gain. A name that has no file behind it, which is anything a processor produced, is left as
/// the model wrote it.
pub(crate) fn name_references(text: &str, named: &[(SlotId, Label, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(at) = rest.find("ref:") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "ref:".len()..];
        let digits = after.chars().take_while(|c| c.is_ascii_digit()).count();
        let name = format!("ref:{}", &after[..digits]);

        // A name with no file behind it, which is anything a processor produced, stays as the
        // planner wrote it: there is nothing truer to put in its place.
        // The reference, its label, and the file: all three, because the planner has only the
        // first of them. A bare filename on a line about a call the planner made reads as
        // though it knew the name, and the whole arrangement is that it does not.
        match named.iter().find(|(slot, _, _)| slot.as_str() == name) {
            Some((slot, label, path)) if digits > 0 => {
                out.push_str(&format!("{slot}{label}:{path}"))
            }
            _ => out.push_str(&name),
        }
        rest = &after[digits..];
    }

    out.push_str(rest);
    out
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
        Labelled::new(
            ".".to_string(),
            bravebot_core::label::Label::untrusted_public(),
        )
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

            // A listing the planner may not read is handed over one reference per entry rather
            // than as one document it can do nothing with. The names stay wrapped the whole way:
            // this reshapes the listing into a list of them inside the kernel and carries it
            // out, and the kernel is what turns each into a slot.
            //
            // The label decides, not the contents: whether the planner may see these names is
            // the same question `present` would ask a moment later.
            if !listing.label().is_trusted() {
                let count = {
                    let shaped = policy
                        .render_in_place("list_files", &listing, |listing| listing.files.len());
                    let proof = policy.authorise_display_release("how many entries a listing has");
                    shaped.declassify(&proof)
                };
                let paths =
                    policy.render_in_place("list_files", &listing, |listing| listing.files.clone());
                return Produced::new(Labelled::trusted(String::new()), proposed_dir.clone(), note)
                    .of_content()
                    .with_entries(Entries {
                        origin: format!("an entry in \"{proposed_dir}\""),
                        paths,
                        count,
                    });
            }

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
            Produced::new(rendered, proposed_dir, note).of_content()
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
    workspace: &Workspace,
    slots: &mut SlotStore,
    named: &Labelled<String>,
    path: &str,
) -> Result<(Labelled<String>, bool), String> {
    let slot = policy
        .accept_reference("write_file", "contents_ref", named)
        .map_err(|denial| format!("refused: {denial}"))?;

    // The bytes are needed now, so a slot still holding only a path reads its file here.
    materialise(
        policy,
        workspace,
        slots,
        "write_file",
        std::slice::from_ref(&slot),
    )?;

    // Where an answer belongs, decided when the processor was asked and not now.
    policy
        .write_belongs_here(path, &slot, slots)
        .map_err(|denial| format!("refused: {denial}"))?;

    // Asked before the bytes are taken, and answered from where the slot came from rather than
    // from what it holds.
    let changes = policy.write_would_change(path, &slot, slots);

    let content = policy
        .resolve("write_file", &slot, slots)
        .map_err(|denial| format!("refused: {denial}"))?;

    Ok((
        policy.declassify_into_workspace(&slot, path, content),
        changes,
    ))
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
    let found = match path_argument(
        policy,
        "write_file",
        Purpose::Effect,
        tools.slots,
        arguments,
    ) {
        Ok(found) => found,
        Err(refusal) => return problem(refusal),
    };
    let (path, destination, shown_path) = (found.path, found.destination, found.shown);

    // The path is routing, so naming a destination from it is not a content decision.
    let proposed_path = path.clone().into_parts_for_decoding().0;

    let written = argument(arguments, "contents");
    let named = argument(arguments, "contents_ref");

    // Two sources would leave the driver deciding which one was meant, and they say different
    // things about what lands in the file. Neither is a decision taken from content: both
    // arguments are the planner's, and this only reports which of them are present.
    // A write of a document the kernel filled from this very file puts it back exactly as it
    // is. Set below, from the slot's provenance, never from comparing what it holds.
    let mut changes_anything = true;

    // What the planner called the body, for the account it is given afterwards. Its own words
    // either way: the reference it named, or its own text.
    let body_from = match &named {
        Some(reference) => {
            let proof = policy.authorise_display_release("which reference a write carried");
            reference.clone().declassify(&proof)
        }
        None => "the contents you gave".to_string(),
    };

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
            match quarantined_body(policy, workspace, tools.slots, &reference, &proposed_path) {
                Ok((body, would_change)) => {
                    changes_anything = would_change;
                    body
                }
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

    // Nothing to do, and nothing to ask about. A processor told to leave a document alone hands
    // back the document, and writing it puts the file back exactly as it is: a diff with nothing
    // in it, put to a person once per file that turned out not to need changing. Approvals that
    // say nothing are how the ones that say something get waved through.
    //
    // The planner is told what it would have been told anyway. Which files a processor decided
    // to leave alone is a fact about their contents, and those do not go into its context.
    if !changes_anything {
        return confirmed(
            format!("{shown_path} holds what {body_from} holds. Nothing further to do for it."),
            "unchanged, nothing written",
        );
    }

    if policy.write_needs_approval(&proposed_path, body_label, destination) {
        let request = WriteRequest {
            intent,
            existing: existing.clone(),
            path: proposed_path.clone(),
            contents: shown.clone(),
            // The reviewer is the only one who will read this. Say what they are reading.
            untrusted: !body_label.is_trusted(),
        };

        if confirmer.confirm_write(&request) == Decision::Reject {
            return problem(format!(
                "refused: the user did not approve writing {shown_path}. Do not retry \
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
            //
            // A write through a reference says the same in the only terms the planner has. It
            // used to read "replaced ref:1, which was already there", which says a reference was
            // replaced rather than a file, and does not say the work is done: one planner read
            // that, could not tell whether anything had happened, and wrote both files a second
            // time. So this says what landed where, and that there is nothing left to do.
            let done = match (intent, destination) {
                (Intent::Create, Destination::Named) => format!("created {shown_path}"),
                (_, Destination::Named) => {
                    format!("replaced {shown_path}, which was already there")
                }
                (Intent::Create, Destination::Reference) => format!(
                    "created the file {shown_path} names, from {}. It is written; do not write \
                     {shown_path} again.",
                    body_from
                ),
                (_, Destination::Reference) => format!(
                    "replaced the file {shown_path} names, which was already there, from {}. It \
                     is written; do not write {shown_path} again.",
                    body_from
                ),
            };
            confirmed(done, note)
                .with_changes(changes)
                .marked_untrusted(!body_label.is_trusted())
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
    slots: &SlotStore,
    confirmer: &mut C,
    arguments: &Value,
) -> Produced {
    let found = match path_argument(policy, "edit_file", Purpose::Effect, slots, arguments) {
        Ok(found) => found,
        Err(refusal) => return problem(refusal),
    };
    let (proposed, destination, shown_path) = (found.path, found.destination, found.shown);
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

    if policy.write_needs_approval(&proposed_path, body_label, destination) {
        let request = WriteRequest {
            path: proposed_path.clone(),
            contents: shown.clone(),
            existing: Some(current.clone()),
            intent: Intent::Edit,
            untrusted: !body_label.is_trusted(),
        };

        if confirmer.confirm_write(&request) == Decision::Reject {
            return problem(format!(
                "refused: the user did not approve editing {shown_path}. Do not retry the \
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
                format!("edited {shown_path}: {occurrences} replacement(s)"),
                note,
            )
            .with_changes(changes)
            .marked_untrusted(!body_label.is_trusted())
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
    slots: &SlotStore,
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
    let mut rows = rows.declassify(&proof);

    // The planner writes its list in the only terms it has, which are reference names. The
    // person reading the list has the opposite problem: "write ref:1 back to its file" says
    // nothing about their own workspace, and they are the only one entitled to know which file
    // that is. So the names go in here, on the way to the screen and nowhere else.
    let named = policy.names_for_display(slots);
    for row in &mut rows {
        row.content = name_references(&row.content, &named);
    }
    reporter.todos(rows);

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
/// Show a command's output to the user, and give it to the planner if they agree.
///
/// The one place bytes cross out of quarantine into the planner's context on nothing but a
/// person's say-so, and the order is what makes that defensible: the output is released for
/// display, put in front of the person in full, and only then, if they agree, does an endorsement
/// exist for the kernel to consume.
///
/// The driver never reads it. The text goes from the slot to the screen and, on approval, from the
/// kernel to the planner; nothing here branches on a byte of it.
fn read_output<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    tools: &mut Tools<'_>,
    confirmer: &mut C,
    arguments: &Value,
) -> Produced {
    let Some(named) = argument(arguments, "ref") else {
        return problem("error: 'ref' is required and must be a reference name, e.g. \"ref:5\"");
    };

    let slot = match policy.accept_reference("read_output", "ref", &named) {
        Ok(slot) => slot,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    // Refused here as well as in the kernel, so a planner naming a file is told what to do about
    // it rather than being told a gate said no.
    if !tools.slots.is_from_command(&slot) {
        return problem(format!(
            "refused: {slot} is not something a program printed, so there is nothing to show. \
             Only a reference that came back from run can be read this way."
        ));
    }

    // Released for the person to read, which is the whole of what this call is for. A display
    // release cannot feed an effect, and this one feeds a screen.
    let shown = {
        let content = match policy.resolve("read_output", &slot, tools.slots) {
            Ok(content) => content,
            Err(denial) => return problem(format!("refused: {denial}")),
        };
        let proof = policy.authorise_display_release("command output the planner asked to read");
        content.declassify(&proof)
    };

    let request = crate::confirm::OutputRequest {
        command: tools
            .slots
            .command_of(&slot)
            .unwrap_or("a command")
            .to_string(),
        output: shown,
        reference: slot.to_string(),
    };

    if confirmer.confirm_read_output(&request) == Decision::Reject {
        return problem(format!(
            "refused: the user did not let you read {slot}. Do not ask for it again. Work with \
             what you have, or say in your reply what you needed from it."
        ));
    }

    // The approval is what makes these bytes readable, and it is bound to this exact reference.
    policy.issue_grant("read_output", "ref", slot.to_string());

    match policy.read_output(&slot, tools.slots) {
        Ok(text) => {
            let lines = tally(request.lines(), "line", "lines");
            Produced::new(text, format!("what {slot} held"), format!("{lines}, read")).of_content()
        }
        Err(denial) => problem(format!("refused: {denial}")),
    }
}

/// Run a program, after a person approves the exact arguments.
///
/// The order is the whole of the safety argument, and it is the same order a write goes through:
///
/// 1. The pipeline is assembled from the planner's arguments, which are untrusted.
/// 2. Every program name is resolved **once**, to an absolute path.
/// 3. The person is shown that exact argv and that exact binary, and answers.
/// 4. The approval mints an endorsement bound to that exact pipeline.
/// 5. `before_run` consumes it, and only then does anything execute, by the resolved path.
///
/// Nothing here branches on untrusted content. The argv is released for display, which is what a
/// person reading it is; what comes back from the program is never read by the driver or the
/// planner, and goes into a slot at the label the kernel fixed before it ran.
fn run<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    tools: &mut Tools<'_>,
    confirmer: &mut C,
    arguments: &Value,
) -> Produced {
    let Some(entries) = arguments.get("pipeline").and_then(Value::as_array) else {
        return problem(
            "error: 'pipeline' is required and must be an array of stages, e.g. \
             [{\"program\": \"git\", \"args\": [\"log\", \"--oneline\"]}]",
        );
    };
    if entries.is_empty() {
        return problem("error: 'pipeline' needs at least one stage");
    }

    // Assembled from the planner's own words, which are untrusted. Every field is wrapped as it is
    // taken and released through one witness, so the audit trail records that a command line was
    // released rather than leaving it to happen implicitly. A person reading argv is the
    // legitimate destination for it: their reading it is what an approval is.
    let proof = policy.authorise_display_release("a proposed command line");
    let mut stages = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(named) = entry.get("program").and_then(Value::as_str) else {
            return problem(
                "error: every stage needs a 'program', which must be a string naming one program",
            );
        };
        let program = Labelled::new(
            named.to_string(),
            bravebot_core::label::Label::untrusted_public(),
        )
        .declassify(&proof);

        let mut args = Vec::new();
        match entry.get("args") {
            None => {}
            Some(Value::Array(given)) => {
                for arg in given {
                    let Some(text) = arg.as_str() else {
                        return problem("error: every entry in a stage's 'args' must be a string");
                    };
                    args.push(
                        Labelled::new(
                            text.to_string(),
                            bravebot_core::label::Label::untrusted_public(),
                        )
                        .declassify(&proof),
                    );
                }
            }
            Some(_) => {
                return problem("error: a stage's 'args' must be an array of strings");
            }
        }
        stages.push(bravebot_core::Stage::new(program, args));
    }
    let pipeline = bravebot_core::Pipeline::new(stages);

    // Resolved once, before anyone is asked. What the person is shown, what the trusted list
    // records, and what executes are then the same value, so `$PATH` changing afterwards cannot
    // put a different binary behind an approval.
    let directory = tools.workspace.root().to_path_buf();
    let mut resolved = Vec::with_capacity(pipeline.len());
    for stage in &pipeline.stages {
        match crate::programs::resolve(&stage.program, &directory) {
            Some(path) => resolved.push(path),
            // Whether a name is a program is decided by looking for it, never by its shape. A
            // guess from the shape refused every path with a space in it, which on macOS is most
            // of /Applications: a planner naming the Brave binary correctly was told four times
            // that it had written a command line, and concluded that spaces were unsupported.
            //
            // Whitespace only picks the wording once the lookup has already failed, which is the
            // one point where a command line and a mistyped path are worth telling apart.
            None if stage.program.contains(char::is_whitespace) => {
                return problem(format!(
                    "error: '{}' was not found, and it contains a space. If that was a command \
                     line, put the program in 'program' and each argument in its own entry of \
                     'args'; there is no shell here to split it. If it is genuinely a path with a \
                     space in it, check the spelling: a path with spaces is fine.",
                    stage.program
                ));
            }
            None => {
                return problem(format!(
                    "error: '{}' was not found. It may not be installed, or may not be on PATH.",
                    stage.program
                ));
            }
        }
    }
    let shown: Vec<String> = resolved
        .iter()
        .map(|path| path.display().to_string())
        .collect();

    if policy.run_needs_approval(&pipeline, &shown) {
        let request = crate::confirm::RunRequest {
            pipeline: pipeline.clone(),
            resolved: shown.clone(),
            directory: directory.display().to_string(),
        };
        let answer = confirmer.confirm_run(&request);
        if !answer.approved() {
            return problem(
                "refused: the user did not approve running this. Do not retry the same \
                 pipeline; ask what they would prefer."
                    .to_string(),
            );
        }
        // Recorded before the run, so a repeat of the same command later in this turn is not
        // asked about again. The policy carries it out of the turn and the session records it.
        if answer.remember {
            for command in request.would_vouch_for() {
                policy.remember_command(command);
            }
        }
    }

    // The approval is what makes this argv trustworthy, and it is bound to this exact pipeline.
    policy.endorse_run(&pipeline);

    let label = match policy.before_run(&pipeline, &shown) {
        Ok(label) => label,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    let displayed = pipeline.display();
    match crate::exec::run(&pipeline, &resolved, &directory, tools.cancel) {
        Ok(ran) => {
            // stdout and stderr together, because a program that failed usually explains itself
            // on stderr and a result that dropped the explanation would be the least useful thing
            // to hand back. Both carry the same label: the kernel fixed it before anything ran and
            // nothing about what was printed changes it.
            let mut text = ran.stdout.clone();
            if !ran.stderr.is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&ran.stderr);
            }

            // Said in the driver's own words, from the exit codes, which are structure rather than
            // content: nothing here reads a byte of what the program printed.
            let outcome = if ran.succeeded() {
                "succeeded".to_string()
            } else {
                let failed: Vec<String> = ran
                    .failures()
                    .iter()
                    .map(|(at, code)| match code {
                        Some(code) => format!("stage {at} exited {code}"),
                        None => format!("stage {at} was killed"),
                    })
                    .collect();
                failed.join(", ")
            };
            let lines = text.lines().count();
            let note = format!("{outcome}, {}", tally(lines, "line", "lines"));

            let mut produced = Produced::new(
                Labelled::new(text, label),
                format!("what `{displayed}` printed"),
                note,
            )
            .of_content();
            // Marks the block the person is shown as content nobody vouched for, which is what a
            // program's output is: it may include bytes an earlier stage read out of a file an
            // attacker wrote.
            produced.untrusted = !label.is_trusted();
            // What the slot will be told it came from, so the user can be asked to read it later
            // and can see which command they are reading.
            produced.printed_by = Some(displayed.clone());
            produced
        }
        // A run that produced nothing still says what happened. The argv is safe to repeat back:
        // a person endorsed it, so it is not something an attacker chose.
        Err(error) => problem(format!("error: `{displayed}` did not run: {error}")),
    }
}

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
        let named = Labelled::new(
            name.to_string(),
            bravebot_core::label::Label::untrusted_public(),
        );
        match policy.accept_reference("spawn_processor", "reads", &named) {
            Ok(slot) => reads.push(slot),
            Err(denial) => return problem(format!("refused: {denial}")),
        }
    }

    // Named for the audit trail from the slots it reads, which are the driver's own names for
    // things. Two processors reading the same references in one turn share a name, and that is
    // the honest description of them.
    //
    // "Isolated" is said every time because it is true every time: the request carries no tool
    // list, no history and no second round, and nothing about a call can change that. The word
    // is not "sandboxed": there is no operating-system boundary here, and there is no untrusted
    // *code* for one to hold. What confines a processor is that it has no capabilities at all.
    // Reference names, not filenames: this is the planner's copy, and it is the one thing it
    // may not have. The person's copy of the same line is resolved where it is drawn.
    let origin = {
        let proof = policy.authorise_display_release("which references a processor was given");
        format!(
            "an isolated processor over {}",
            references_in(arguments).declassify(&proof)
        )
    };

    // Before the spec, not after: `before_processor` computes the output's label by taint over
    // the inputs, and a slot that reads its file here may come back untrusted where the trust
    // map fell after the slot was reserved. A spec built first would carry the label the inputs
    // used to have.
    let opened = match materialise(
        policy,
        tools.workspace,
        tools.slots,
        "spawn_processor",
        &reads,
    ) {
        Ok(opened) => opened,
        Err(refusal) => return problem(refusal),
    };

    // Which document the call is about: the answer replaces that one, and where nothing should
    // change it stands as the answer. The planner's own choice, out of the references it named,
    // fixed before the processor exists.
    let about = match argument(arguments, "about") {
        Some(named) => match policy.accept_reference("spawn_processor", "about", &named) {
            Ok(slot) => Some(slot),
            Err(denial) => return problem(format!("refused: {denial}")),
        },
        None => None,
    };

    let spec = match policy.before_processor(&origin, &reads, &instruction, about, tools.slots) {
        Ok(spec) => spec,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    match processor::run(policy, &mut tools.chat, tools.slots, &spec) {
        Ok(done) => {
            // Nothing to keep. A slot is written once and read by whatever the planner points
            // at it, and a slot holding a copy of a document that is already in a slot has
            // nothing for anyone to point at: the file it stands for is the file it came from,
            // and that file needs no writing. So none is minted, and the planner is told there
            // is nothing to write rather than handed a name for a copy.
            if let Some(from) = &done.unchanged_from {
                let mut produced = confirmed(
                    format!(
                        "{from} needs no change, so there is nothing to write for it. Do not \
                         write it, and do not process it again."
                    ),
                    "left it alone",
                )
                .costing(done.usage);
                produced.said = done.note;
                return produced;
            }

            // Nothing to write, and nothing minted for it. An answer that never said which part
            // of itself was a file cannot become one: everything a processor writes is for a
            // person to read unless it declares where the document begins, and this one
            // declared nothing.
            let Some(document) = done.document else {
                let mut produced = confirmed(
                    "that answer named no document, so there is nothing to write. What it said \
                     is on the screen. Ask again, and say that the whole file must follow the \
                     line that marks where the document begins."
                        .to_string(),
                    "said something, produced no document",
                )
                .costing(done.usage);
                produced.said = done.note;
                return produced;
            };

            let wrote = note_for(policy, "spawn_processor", &document, |text: String| {
                tally(text.lines().count(), "line", "lines")
            });
            // Says who did what. The planner's own reads read nothing, since a reference to a
            // file already is the file; the processor is what opens them, and until this said
            // so the only reads on the screen were the ones that did not happen.
            let note = if opened.is_empty() {
                format!("an isolated processor wrote {wrote}")
            } else {
                format!(
                    "an isolated processor read {} and wrote {wrote}",
                    opened.join(", ")
                )
            };
            let mut produced = Produced::new(document, origin, note)
                .costing(done.usage)
                .of_content();
            produced.unchanged_from = done.unchanged_from;
            produced.answers_for = Some(spec.about().cloned());
            produced.said = done.note;
            produced
        }
        Err(error) => problem(format!("error: {error}")),
    }
}

/// Read a skill the planner was listed.
///
/// The name arrives as model output, so it is promoted the way a read path is: the operation
/// changes nothing and is confined to a boundary the user established. It is in fact more
/// confined than a read. The promoted name never becomes a path component; it only **selects**
/// from the set the driver enumerated before the turn began, so a name naming a traversal, an
/// absolute path, or anything else at all matches nothing and the call is refused. There is no
/// filesystem lookup for it to reach.
///
/// The body keeps the label it was read with and goes back like any other tool result, so
/// `Policy::present` is what decides whether the planner sees it.
fn load_skill<S: Sink>(
    policy: &mut Policy<'_, S>,
    skills: &crate::skills::Catalogue,
    arguments: &Value,
) -> Produced {
    let Some(proposed) = argument(arguments, "name") else {
        return problem("error: 'name' is required and must be a string");
    };

    let name = match policy.promote_confined_read("load_skill", "name", &proposed) {
        Ok(name) => name,
        Err(denial) => return problem(format!("refused: {denial}")),
    };
    // Safe to read: promotion just proved this is (T,pub), and comparing trusted text decides
    // nothing an attacker steers.
    let Ok(name) = name.into_trusted() else {
        return problem("error: the skill name was not usable");
    };

    let Some(skill) = skills.get(&name) else {
        // The names are listed in the system prompt, so this is a mistake worth naming rather
        // than a refusal worth explaining.
        return problem(format!(
            "error: no skill named '{name}'. The skills available to you are listed for you; \
             there are no others."
        ));
    };

    let note = note_for(policy, "load_skill", skill.body(), |text: String| {
        tally(text.lines().count(), "line", "lines")
    });
    Produced::new(skill.body().clone(), skill.origin.clone(), note)
}

/// Read one option the model offered.
///
/// Two shapes, because a model that writes a bare string meant an option with no explanation and
/// refusing it would cost the person a choice over punctuation.
fn choice_from(option: &Value) -> Option<Choice> {
    if let Value::String(label) = option {
        return Some(Choice::new(label.clone(), None));
    }
    let label = option.get("label")?.as_str()?.to_string();
    let detail = option
        .get("detail")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(Choice::new(label, detail))
}

/// Read one question the model asked.
///
/// Total in the way `choice_from` is: it yields `None` for a question there is nothing to draw,
/// and the count check at the call site turns that into a refusal of the whole call. Nothing
/// here decides which questions exist, only whether the call as a whole is answerable.
fn question_from(entry: &Value) -> Option<Question> {
    let header = entry.get("header")?.as_str()?.to_string();
    let prompt = entry.get("question")?.as_str()?.to_string();

    let offered = match entry.get("options") {
        None | Some(Value::Null) => None,
        Some(Value::Array(options)) => Some(options),
        // Present but not a list. Falling through to a bare text field here would throw away
        // options the model meant to offer and leave the user staring at a question that names
        // choices it does not show.
        Some(_) => return None,
    };

    let choices: Vec<Choice> = offered
        .map(|options| options.iter().filter_map(choice_from).collect())
        .unwrap_or_default();
    // An option with nothing to draw is the one thing skipped, and this is what says so rather
    // than asking a question with a hole in it.
    if choices.len() != offered.map_or(0, Vec::len) {
        return None;
    }

    // Never silently falls back to one answer. A model that asked for several and was quietly
    // given a single-answer picker leaves the user unable to say what they were asked for, with
    // nothing on screen to suggest anything went wrong.
    let multiple = match entry.get("multiple") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        // Tool arguments arrive as JSON text, so the quoted spelling is common enough to accept.
        Some(Value::String(text)) if text.eq_ignore_ascii_case("true") => true,
        Some(Value::String(text)) if text.eq_ignore_ascii_case("false") => false,
        Some(_) => return None,
    };

    Some(Question::new(header, prompt, choices, multiple))
}

/// Put the planner's questions to the person and hand back what they said.
///
/// The only tool whose result comes from a person rather than from the workspace, and the only
/// one with no effect at all. It still has a destination, the user's screen, and that is what
/// makes the questions and their options **routing**: they decide what the person is shown and
/// therefore what they can answer. The routing field here is approved by being read, since what
/// is drawn is exactly the bytes the gate checked and nothing re-parses them afterwards.
///
/// The gate runs once for the whole series. A series is asked whole or refused whole, because
/// asking some of it would mean deciding which of the questions the person sees, and that
/// decision would be taken from what is in them.
///
/// Note there is no hand-written check on the context here. The refusal is the ordinary routing
/// gate doing its job, which is the point: relocating that decision into the driver would be the
/// violation, not the safeguard.
fn ask_user<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    confirmer: &mut C,
    arguments: &Value,
) -> Produced {
    let Some(entries) = arguments.get("questions").and_then(Value::as_array) else {
        return problem(
            "error: 'questions' is required and must be an array of one to four questions",
        );
    };

    if entries.is_empty() {
        return problem("error: 'questions' must hold at least one question.");
    }
    // Refused rather than trimmed. A question dropped here is one the model is told the person
    // was asked and the person never saw, which is worse than being made to send the call again.
    if entries.len() > ask::MOST_AT_ONCE {
        return problem(format!(
            "error: at most {} questions can be asked at once; nobody was asked anything. Send \
             the ones the work turns on.",
            ask::MOST_AT_ONCE
        ));
    }

    let asked: Vec<Question> = entries.iter().filter_map(question_from).collect();
    if asked.len() != entries.len() {
        return problem(
            "error: every question needs a 'header' tag and a 'question' sentence, and every \
             option needs a label; nobody was asked anything. Send the whole set again.",
        );
    }

    let series = policy.label_model_output("ask_user", Series::new(asked));

    // One string standing for every question, so the gate checks everything the person will be
    // shown rather than the first question or the sentences alone.
    let canonical = policy.render_in_place("ask_user", &series, |s| ask::canonical_series(&s));
    if let Err(denial) = policy.before_action("ask_user", "questions", Role::Routing, &canonical) {
        return problem(format!(
            "refused: {denial}. Questions can only be put to the user before anything untrusted \
             has reached your context. Continue without an answer, or say in your reply what you \
             need to know."
        ));
    }

    // Shaped inside the kernel for the same reason a task list is: laying out options means
    // reading them. Every question yields a prompt and every choice a row, so nothing in the
    // text decides what the person is shown the existence of.
    let shaped = policy.render_in_place("ask_user", &series, |s| ask::asking(&s));
    let proof = policy.authorise_display_release("questions for the user");
    let answers = confirmer.ask_user(&shaped.declassify(&proof));

    // The kernel puts the replies into words and lines them up against the questions, so nothing
    // here branches on what the person said or counts what they answered.
    match policy.record_answers("ask_user", &series, &answers) {
        Ok(text) => Produced::new(text, "", tally(entries.len(), "answer", "answers")),
        Err(denial) => problem(format!("refused: {denial}")),
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
        Labelled::new(
            ".".to_string(),
            bravebot_core::label::Label::untrusted_public(),
        )
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
            Produced::new(rendered, proposed_where, note).of_content()
        }
        Err(e) => problem(format!("error: {e}")),
    }
}

#[cfg(test)]
mod tests {

    /// A model that namespaces a tool by the group it was offered in means the tool. Answering
    /// "no such tool" to that spends a round on a difference in spelling.
    #[test]
    fn a_namespaced_tool_name_means_the_tool() {
        assert_eq!(strip_namespace("functions_todo_write"), "todo_write");
        assert_eq!(strip_namespace("functions.write_file"), "write_file");
        assert_eq!(strip_namespace("todo_write"), "todo_write");
        // Not an invitation to guess at anything else.
        assert_eq!(strip_namespace("tools.todo_write"), "tools.todo_write");
        assert_eq!(strip_namespace("functions."), "functions.");
    }

    /// A task list is written by the planner, which has only reference names, and read by the
    /// person whose directory it is, who has only filenames. The line has to carry both, and the
    /// filename has to be the half that reaches the screen.
    #[test]
    fn a_task_list_names_the_file_a_reference_stands_for() {
        let untrusted = bravebot_core::label::Label::untrusted_private();
        let named = vec![
            (SlotId::new("ref:1"), untrusted, "src/game.js".to_string()),
            (SlotId::new("ref:10"), untrusted, "server.py".to_string()),
        ];

        // The reference and its label come with the name: a bare filename would read as
        // something the planner knows, and it does not.
        assert_eq!(
            name_references("Write the fixed ref:1 back to its file", &named),
            "Write the fixed ref:1(U,priv):src/game.js back to its file"
        );
        // A longer name is not the shorter one with something after it.
        assert_eq!(
            name_references("ref:10 and ref:1.", &named),
            "ref:10(U,priv):server.py and ref:1(U,priv):src/game.js."
        );
        // A reference with no file behind it is left as the planner wrote it: a processor's
        // output is content, and there is nothing truer to put in its place.
        assert_eq!(
            name_references("what ref:4 produced", &named),
            "what ref:4 produced"
        );
        assert_eq!(name_references("nothing to do", &named), "nothing to do");
    }
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
                "spawn_processor",
                "load_skill",
                "ask_user",
                "run",
                "read_output"
            ]
        );
    }

    /// A **shell** stays absent, and this is the distinction the whole tool turns on. A shell
    /// string is destination and payload at once, so there is no separable routing field a person
    /// could approve, and a parser that tried to recover one would be racing a shell it does not
    /// control. An argv vector has no such problem, which is why `run` exists and this does not.
    ///
    /// The test that used to stand here banned every tool whose name contained "run". It predated
    /// the argv design by a day and would have blocked it, which is the failure mode worth
    /// remembering: a test pinning the old reason for a rule outlives the reason.
    #[test]
    fn no_shell_is_offered() {
        for tool in available() {
            let name = tool.function.name;
            assert!(!name.contains("shell"), "{name} takes a shell string");
            assert!(!name.contains("exec"), "{name} takes a shell string");
        }
    }

    /// `run` takes a pipeline of argv stages and nothing else. A single string field would be a
    /// command line by another name, and everything the design rests on would go with it.
    #[test]
    fn run_takes_argv_and_never_a_command_line() {
        let tool = available()
            .into_iter()
            .find(|t| t.function.name == "run")
            .expect("run is offered");
        let properties = tool.function.parameters["properties"]
            .as_object()
            .expect("run has parameters");
        assert_eq!(
            properties.keys().collect::<Vec<_>>(),
            vec!["pipeline"],
            "run gained a field that is not the pipeline"
        );

        let stage = &properties["pipeline"]["items"]["properties"];
        assert!(stage.get("program").is_some(), "a stage names its program");
        assert_eq!(
            stage["args"]["type"], "array",
            "arguments must be a list, never a string for something to split"
        );
        for absent in ["command", "cmd", "shell", "script", "argv_string"] {
            assert!(
                stage.get(absent).is_none(),
                "a stage gained a '{absent}' field, which would be a command line"
            );
        }
    }

    /// A planner that asks the user for a path, a filename, or whether something is installed is
    /// asking a person to do a lookup, and they answer less precisely than the filesystem does.
    /// One session opened by asking where Brave was installed rather than looking.
    #[test]
    fn asking_is_described_as_a_last_resort_after_looking() {
        let tool = available()
            .into_iter()
            .find(|t| t.function.name == "ask_user")
            .expect("ask_user is offered");
        let described = tool.function.description.to_lowercase();
        assert!(
            described.contains("cannot find out yourself"),
            "ask_user does not say to look first: {described}"
        );
        assert!(
            described.contains("never for a fact about this machine"),
            "ask_user does not rule out asking for discoverable facts: {described}"
        );
    }

    /// The reason looking first is safe, which the planner has no way to know otherwise. It used
    /// to be told the opposite, that a question was refused once anything had been read, which is
    /// what made front-loading questions look obligatory.
    #[test]
    fn ask_user_says_that_looking_first_does_not_forfeit_the_question() {
        let tool = available()
            .into_iter()
            .find(|t| t.function.name == "ask_user")
            .expect("ask_user is offered");
        assert!(
            tool.function
                .description
                .contains("does not stop you asking afterwards"),
            "ask_user still implies reading forfeits the question: {}",
            tool.function.description
        );
    }

    /// `args` is what follows the program, not an argv vector. A planner reading it as argv puts
    /// the program name in twice, and `open open -a ...` ran instead of `open -a ...`: the
    /// browser never started, the error was quarantined, and the turn reported success.
    #[test]
    fn run_says_the_arguments_exclude_the_program_name() {
        let tool = available()
            .into_iter()
            .find(|t| t.function.name == "run")
            .expect("run is offered");
        let described = tool.function.parameters["properties"]["pipeline"]["items"]["properties"]
            ["args"]["description"]
            .as_str()
            .expect("args is described");
        assert!(
            described.contains("Do not repeat the program name"),
            "args does not rule out argv[0]: {described}"
        );
        assert!(
            described.contains("no argv[0]"),
            "args does not name the convention it is not: {described}"
        );
    }

    /// The tool must tell the planner it will not see the output, or it spends rounds running
    /// things to read results that never come back to it.
    #[test]
    fn run_says_its_output_does_not_come_back_to_the_planner() {
        let tool = available()
            .into_iter()
            .find(|t| t.function.name == "run")
            .expect("run is offered");
        let described = tool.function.description.to_lowercase();
        assert!(
            described.contains("not be shown") || described.contains("reference"),
            "run does not say the output is quarantined: {described}"
        );
        assert!(
            described.contains("approve"),
            "run does not say the user approves it first"
        );
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
        assert_eq!(
            value.label(),
            bravebot_core::label::Label::untrusted_public()
        );
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

    mod questions {
        use super::*;
        use crate::confirm::{ApproveWrites, ChoosesFirst, Unattended};
        use bravebot_core::ask::{Answer, Asking};
        use bravebot_core::capability::{Capability, CapabilitySet};
        use bravebot_core::event::RecordingSink;
        use bravebot_core::label::{Integrity, Label};
        use bravebot_core::policy::{ReleasePlan, Routing};
        use bravebot_core::trust::TrustStore;

        fn routing() -> Routing {
            let mut r = Routing::new();
            r.insert_trusted("task", "plan some work");
            r
        }

        /// Records what it was shown, so a test can assert the person saw the whole series.
        #[derive(Default)]
        struct Watching {
            seen: Vec<Asking>,
            reply: Vec<Answer>,
        }

        impl Confirmer for Watching {
            fn confirm_write(&mut self, _request: &WriteRequest) -> Decision {
                Decision::Reject
            }

            fn confirm_run(
                &mut self,
                _request: &crate::confirm::RunRequest,
            ) -> crate::confirm::RunDecision {
                crate::confirm::RunDecision::reject()
            }

            fn confirm_read_output(
                &mut self,
                _request: &crate::confirm::OutputRequest,
            ) -> Decision {
                Decision::Reject
            }

            fn confirm_vouch(&mut self, _request: &crate::confirm::VouchRequest) -> Decision {
                Decision::Reject
            }

            fn ask_user(&mut self, asking: &Asking) -> Vec<Answer> {
                self.seen.push(asking.clone());
                self.reply.clone()
            }
        }

        /// Run the tool against a fresh policy in a workspace the user vouched for.
        fn call<C: Confirmer>(confirmer: &mut C, arguments: Value) -> Labelled<String> {
            let mut sink = RecordingSink::new();
            let mut trust = TrustStore::new();
            trust.trust(".");
            let mut policy = Policy::begin(
                routing(),
                ReleasePlan::new(),
                CapabilitySet::from_iter([Capability::FileRead]),
                &mut sink,
            )
            .expect("policy")
            .with_trust(trust);
            let produced = ask_user(&mut policy, confirmer, &arguments);
            // A question has no source in the workspace, so there is nothing for an origin to
            // name.
            assert!(
                produced.origin.is_empty(),
                "a question named an origin: {}",
                produced.origin
            );
            produced.text
        }

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

        fn one_question() -> Value {
            json!({"questions": [{
                "header": "Cache layer",
                "question": "Which cache layer?",
                "options": [{"label": "HTTP", "detail": "in front of the handler"},
                            {"label": "Query"}]
            }]})
        }

        fn three_questions() -> Value {
            json!({"questions": [
                {"header": "Cache", "question": "Which cache layer?",
                 "options": [{"label": "HTTP"}, {"label": "Query"}]},
                {"header": "Scope", "question": "Is the migration in scope?",
                 "options": [{"label": "Yes"}, {"label": "No"}]},
                {"header": "Branch", "question": "Which branch?",
                 "options": [{"label": "main"}]}
            ]})
        }

        /// The point of a series: one call settles everything the plan turns on, and the person
        /// is shown all of it rather than one question per turn.
        #[test]
        fn the_person_is_shown_every_question_in_the_call() {
            let mut confirmer = Watching::default();
            call(&mut confirmer, three_questions());
            let shown = confirmer.seen.first().expect("the user was asked");
            assert_eq!(shown.prompts.len(), 3);
            assert_eq!(shown.prompts[0].question, "Which cache layer?");
            assert_eq!(shown.prompts[1].question, "Is the migration in scope?");
            assert_eq!(shown.prompts[2].question, "Which branch?");
        }

        /// The tag reaches the screen, since it is the thing that tells one question from the
        /// next when three arrive together.
        #[test]
        fn every_question_carries_its_tag_to_the_person() {
            let mut confirmer = Watching::default();
            call(&mut confirmer, three_questions());
            let shown = confirmer.seen.first().expect("asked");
            let tags: Vec<&str> = shown.prompts.iter().map(|p| p.header.as_str()).collect();
            assert_eq!(tags, vec!["Cache", "Scope", "Branch"]);
        }

        /// A lone question is a series of one, so there is one path through the tool rather than
        /// two that could drift apart.
        #[test]
        fn a_single_question_is_asked_as_a_series_of_one() {
            let mut confirmer = Watching::default();
            call(&mut confirmer, one_question());
            assert_eq!(confirmer.seen.first().expect("asked").prompts.len(), 1);
        }

        /// The planner has to be able to read the reply, or asking was pointless.
        #[test]
        fn every_answer_reaches_the_planner_in_the_clear() {
            let mut confirmer = ChoosesFirst;
            let text = call(&mut confirmer, three_questions());
            assert_eq!(text.label(), Label::trusted_public());
            let told = released(&text);
            assert!(told.contains("The user chose: HTTP"), "{told}");
            assert!(told.contains("The user chose: Yes"), "{told}");
            assert!(told.contains("The user chose: main"), "{told}");
        }

        /// And each answer has to say which question it settled, or the planner is guessing.
        #[test]
        fn each_answer_is_reported_under_the_question_it_answers() {
            let mut confirmer = ChoosesFirst;
            let told = released(&call(&mut confirmer, three_questions()));
            assert!(
                told.contains("Which cache layer?\nThe user chose: HTTP"),
                "{told}"
            );
        }

        /// Skipping one question must not cost the person the answers they did give.
        #[test]
        fn a_skipped_question_is_reported_beside_its_answered_siblings() {
            let mut confirmer = Watching {
                reply: vec![
                    Answer::Chosen(vec![0]),
                    Answer::Declined,
                    Answer::Chosen(vec![0]),
                ],
                ..Default::default()
            };
            let told = released(&call(&mut confirmer, three_questions()));
            assert!(told.contains("The user chose: HTTP"), "{told}");
            assert!(told.contains("declined"), "{told}");
            assert!(told.contains("The user chose: main"), "{told}");
        }

        /// An interface that answered nothing answered nothing, and every question is reported
        /// as skipped rather than one answer sliding onto the wrong question.
        #[test]
        fn an_answer_the_interface_never_gave_is_reported_as_a_decline() {
            let mut confirmer = Unattended;
            let told = released(&call(&mut confirmer, three_questions()));
            assert_eq!(told.matches("declined").count(), 3, "{told}");
        }

        #[test]
        fn an_unattended_run_declines_rather_than_choosing() {
            let mut confirmer = Unattended;
            assert!(released(&call(&mut confirmer, one_question())).contains("declined"));
        }

        #[test]
        fn approving_writes_does_not_answer_a_question() {
            let mut confirmer = ApproveWrites;
            assert!(released(&call(&mut confirmer, one_question())).contains("declined"));
        }

        /// The property the whole tool rests on. Once the context has met untrusted content the
        /// questions may have been shaped by it, and a person picking among strings an attacker
        /// wrote does not make those strings trusted.
        #[test]
        fn a_series_is_refused_once_the_context_has_met_something_untrusted() {
            let mut sink = RecordingSink::new();
            let mut policy = Policy::begin(
                routing(),
                ReleasePlan::new(),
                CapabilitySet::from_iter([Capability::FileRead]),
                &mut sink,
            )
            .expect("policy")
            .resuming(Integrity::Untrusted);

            let mut confirmer = Watching::default();
            let text = ask_user(&mut policy, &mut confirmer, &three_questions()).text;

            assert!(
                confirmer.seen.is_empty(),
                "the user was asked questions derived from untrusted content"
            );
            let told = released(&text);
            assert!(told.starts_with("refused:"), "{told}");
            assert!(
                told.contains("before anything untrusted has reached your context"),
                "the refusal does not say when asking is possible: {told}"
            );
        }

        /// And the refusal is text, not a failed turn: the model can carry on without an answer.
        #[test]
        fn a_refused_series_is_reported_rather_than_failing_the_turn() {
            let mut sink = RecordingSink::new();
            let mut policy = Policy::begin(
                routing(),
                ReleasePlan::new(),
                CapabilitySet::from_iter([Capability::FileRead]),
                &mut sink,
            )
            .expect("policy")
            .resuming(Integrity::Untrusted);

            let mut confirmer = Unattended;
            let text = ask_user(&mut policy, &mut confirmer, &one_question()).text;
            assert_eq!(text.label(), Label::trusted_public());
        }

        /// Trimming would tell the model the person was asked something they never saw.
        #[test]
        fn more_than_four_questions_are_refused_rather_than_trimmed() {
            let mut confirmer = Watching::default();
            let many: Vec<Value> = (0..5)
                .map(|i| json!({"header": format!("T{i}"), "question": format!("Q{i}?")}))
                .collect();
            let told = released(&call(&mut confirmer, json!({"questions": many})));
            assert!(
                confirmer.seen.is_empty(),
                "the user was asked a trimmed set of questions"
            );
            // The message has to name the limit, not merely be an error. Trimming the list and
            // then failing some later check would also produce an error, and would tell the
            // model its questions were malformed when what was wrong was how many it asked.
            assert!(
                told.contains(&ask::MOST_AT_ONCE.to_string()),
                "the refusal does not say what the limit is: {told}"
            );
        }

        #[test]
        fn an_empty_list_of_questions_is_an_error() {
            let mut confirmer = Watching::default();
            let told = released(&call(&mut confirmer, json!({"questions": []})));
            assert!(told.starts_with("error:"), "{told}");
            assert!(confirmer.seen.is_empty());
        }

        #[test]
        fn a_missing_question_list_is_an_error() {
            let mut confirmer = Watching::default();
            let told = released(&call(&mut confirmer, json!({"question": "Which?"})));
            assert!(told.starts_with("error:"), "{told}");
        }

        /// The whole call fails rather than one question quietly going missing, because which
        /// questions exist must not be decided by what the model wrote in them.
        #[test]
        fn a_question_with_no_tag_fails_the_whole_call() {
            let mut confirmer = Watching::default();
            let told = released(&call(
                &mut confirmer,
                json!({"questions": [
                    {"header": "Cache", "question": "Which cache layer?"},
                    {"question": "Which branch?"}
                ]}),
            ));
            assert!(told.starts_with("error:"), "{told}");
            assert!(
                confirmer.seen.is_empty(),
                "the user was asked the questions that parsed"
            );
        }

        #[test]
        fn a_question_with_no_sentence_fails_the_whole_call() {
            let mut confirmer = Watching::default();
            let told = released(&call(
                &mut confirmer,
                json!({"questions": [{"header": "Cache"}]}),
            ));
            assert!(told.starts_with("error:"), "{told}");
            assert!(confirmer.seen.is_empty());
        }

        #[test]
        fn an_option_with_no_label_fails_the_whole_call() {
            let mut confirmer = Watching::default();
            let told = released(&call(
                &mut confirmer,
                json!({"questions": [{
                    "header": "Cache", "question": "Which?",
                    "options": [{"label": "HTTP"}, {"detail": "no label"}]
                }]}),
            ));
            assert!(told.starts_with("error:"), "{told}");
            assert!(confirmer.seen.is_empty());
        }

        /// A question the model could not supply options for is still worth asking: the person
        /// answers in their own words.
        #[test]
        fn a_question_with_no_options_still_reaches_the_person() {
            let mut confirmer = Watching::default();
            call(
                &mut confirmer,
                json!({"questions": [{"header": "Branch", "question": "Which branch?"}]}),
            );
            assert!(
                confirmer.seen.first().expect("asked").prompts[0]
                    .rows
                    .is_empty()
            );
        }

        #[test]
        fn options_given_as_plain_strings_are_offered() {
            let mut confirmer = Watching::default();
            call(
                &mut confirmer,
                json!({"questions": [{
                    "header": "Cache", "question": "Which?", "options": ["HTTP", "Query"]
                }]}),
            );
            let rows = &confirmer.seen.first().expect("asked").prompts[0].rows;
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].label, "HTTP");
        }

        #[test]
        fn a_multiple_choice_question_says_so_to_the_person() {
            let mut confirmer = Watching::default();
            call(
                &mut confirmer,
                json!({"questions": [{
                    "header": "Platforms", "question": "Which?",
                    "options": ["Linux"], "multiple": true
                }]}),
            );
            assert!(confirmer.seen.first().expect("asked").prompts[0].multiple);
        }

        /// Tool arguments arrive as JSON text, so a quoted boolean is common. Reading it as
        /// false would hand the user a one-answer picker for a question that asked for several.
        #[test]
        fn a_quoted_boolean_still_asks_for_several_answers() {
            for spelling in [json!("true"), json!("True"), json!("TRUE")] {
                let mut confirmer = Watching::default();
                call(
                    &mut confirmer,
                    json!({"questions": [{
                        "header": "Platforms", "question": "Which?",
                        "options": ["Linux"], "multiple": spelling
                    }]}),
                );
                assert!(confirmer.seen.first().expect("asked").prompts[0].multiple);
            }
        }

        /// Anything else is refused rather than read as one answer, for the same reason: a
        /// silent downgrade is invisible to everyone who could have noticed it.
        #[test]
        fn an_unreadable_multiple_fails_the_call_rather_than_asking_for_one() {
            let mut confirmer = Watching::default();
            let told = released(&call(
                &mut confirmer,
                json!({"questions": [{
                    "header": "Platforms", "question": "Which?",
                    "options": ["Linux"], "multiple": "yes"
                }]}),
            ));
            assert!(told.starts_with("error:"), "{told}");
            assert!(confirmer.seen.is_empty());
        }

        /// Options that are not a list would leave the person staring at a question naming
        /// choices it does not show.
        #[test]
        fn options_that_are_not_a_list_fail_the_call() {
            let mut confirmer = Watching::default();
            let told = released(&call(
                &mut confirmer,
                json!({"questions": [{
                    "header": "Cache", "question": "Which?", "options": "HTTP or Query"
                }]}),
            ));
            assert!(told.starts_with("error:"), "{told}");
        }
    }

    mod todos {
        use super::*;
        use crate::report::RecordingReporter;
        use bravebot_core::capability::{Capability, CapabilitySet};
        use bravebot_core::event::RecordingSink;
        use bravebot_core::label::Integrity;
        use bravebot_core::policy::{ReleasePlan, Routing};

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
            let produced = todo_write(&mut policy, &mut reporter, &SlotStore::new(), &arguments);
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
                &SlotStore::new(),
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
                &SlotStore::new(),
                &list(&[("a task", "in_progress")]),
            );

            assert_eq!(reporter.updates.len(), 1);
            assert!(policy.finish(), "a gate refused something");
        }
    }
}
