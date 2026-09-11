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

use crate::lsp::LanguageServers;
use crate::workspace::{Listing, Page, Workspace};

/// The statuses the schema advertises, taken from the kernel so the two cannot drift.
const TODO_STATUSES: [&str; 3] = Status::NAMES;

/// The tools the model may call.
///
/// `self_paced` says this turn is a tick of a loop nobody gave an interval for, which is the one
/// turn that may say when the next one happens. Offered rather than always present because a tool
/// that does nothing outside one situation is a tool the planner has to be told to ignore, and a
/// planner that calls it anyway would have been told it worked.
pub fn available(self_paced: bool) -> Vec<Tool> {
    let mut tools = vec![
        Tool::function(
            "read_file",
            "Read a file from the workspace. Returns its lines. Ask for the whole \
             file: long ones come back one page at a time, and the result says so and gives the \
             offset to continue from. Name the file with path, or with path_ref where a listing \
             gave you a reference instead of a name. \
             \
             A picture or a PDF (.png, .jpg, .gif, .webp, .pdf) comes back as a reference rather \
             than as anything you can look at, whoever vouched for the directory it is in. Give \
             that reference to spawn_processor with a question about it and the answer comes back \
             the way any processor's does. That is how to check a screenshot or read a scanned \
             page.",
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
                        "description": "Maximum lines to return, capped so one read cannot \
                                        fill the conversation. Omit it and you get the file \
                                        up to that cap, which is what you want almost always: \
                                        this is for stepping through something long, not for \
                                        sampling something short, and several windows of one \
                                        file cost more than the file did."
                    }
                },
                "required": []
            }),
        ),
        Tool::function(
            "list_files",
            "List files in the workspace under a directory, recursively unless you give a \
             depth. Give a glob pattern or a depth to narrow the result rather than listing \
             everything: depth 1 is the one directory itself, the way ls reads it, and is what \
             to use when you want to know what a project holds rather than every file it \
             contains. In a directory you may not read, the names are quarantined and you get \
             one reference per file instead: use those as path_ref to read a file, process it, \
             and write it back, without ever being told what it is called.",
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
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Optional number of directory levels to descend. 1 \
                                        lists this directory and no deeper, naming each \
                                        subdirectory with a trailing / so you can see where \
                                        the tree continues. Omit it to walk the whole tree, \
                                        which in a large project is thousands of paths.",
                        "minimum": 1
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
            "Find regular expressions in workspace files. Returns matching lines. Give every \
             spelling you would otherwise search for one at a time: a list of patterns costs \
             one call and matches a line matching any of them.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "description": "Regular expression to find. Supports ., *, +, ?, |, \
                                        (), [], \\d, \\w, \\s, ^, $ and \\b. Counted \
                                        repetition such as a{2,3} is not supported and { is \
                                        an ordinary character. Escape a metacharacter with a \
                                        backslash to match it literally. May be a list, in \
                                        which case a line matches if it matches any of them: \
                                        prefer one call with every spelling you want over one \
                                        call each.",
                        "anyOf": [
                            {"type": "string"},
                            {"type": "array", "items": {"type": "string"}}
                        ]
                    },
                    "directory": {
                        "type": "string",
                        "description": "Workspace-relative directory to search. Defaults to \".\"."
                    },
                    "include": {
                        "type": "string",
                        "description": "Optional glob limiting which files are searched, \
                                        e.g. \"*.rs\" or \"**/*.{cc,h,mm}\". Supports *, ?, \
                                        ** and brace groups. Character classes and extended \
                                        globs are not supported."
                    },
                    "case_sensitive": {
                        "type": "boolean",
                        "description": "Whether case matters. Defaults to true. Set false \
                                        rather than shortening the pattern to dodge a capital."
                    }
                },
                "required": ["pattern"]
            }),
        ),
        Tool::function(
            "lsp",
            "Ask a language server about a symbol: where it is defined, what refers to it, what \
             implements it, what calls it. Use this instead of search when the question is about \
             a symbol rather than about text, because a search for a name finds every comment \
             and string that mentions it while this finds the declaration the compiler agrees \
             on. Give the position of the symbol you are asking about, taken from a line you \
             were already shown by a search or a read; nothing here works out where a name is \
             for you. Answers with the places it found, one per line. Where a definition is in \
             a dependency rather than in this project the line says so, and read_file will not \
             open it. hover is the one operation that answers with prose, and that prose always \
             comes back as a reference you cannot read rather than as text, because nothing in a \
             server's answer says which file wrote it.",
            json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "description": "Which question to ask.",
                        "enum": [
                            "goToDefinition",
                            "findReferences",
                            "hover",
                            "documentSymbol",
                            "workspaceSymbol",
                            "goToImplementation",
                            "incomingCalls",
                            "outgoingCalls"
                        ]
                    },
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative path to the file holding the symbol, \
                                        e.g. src/main.rs. Required for every operation but \
                                        workspaceSymbol."
                    },
                    "line": {
                        "type": "integer",
                        "description": "1-based line of the symbol, as a search or a read \
                                        reported it. A line that no longer holds the symbol \
                                        answers with nothing rather than failing.",
                        "minimum": 1
                    },
                    "character": {
                        "type": "integer",
                        "description": "1-based column of the symbol on that line. Point at the \
                                        name itself rather than at the start of the line.",
                        "minimum": 1
                    },
                    "query": {
                        "type": "string",
                        "description": "The name to look for. Only for workspaceSymbol, which \
                                        searches a language server's index rather than starting \
                                        from a position, and so needs a server already running: \
                                        ask about a symbol in a file of that language first."
                    }
                },
                "required": ["operation"]
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
            "Run a command line. Write it the way you would type it: `grep -rn thing src/ | \
             head -30`. Pipes, &&, ||, ;, ( ), quoting, redirection (>, >>, <, 2>, 2>&1, &>), \
             globs and {a,b} all work. There is no shell: the line is compiled here into the \
             programs and arguments it names, and anything that cannot be worked out from the \
             line itself is refused rather than guessed at. $(...), backticks, $VAR and ${...} \
             are refused for that reason: write the value out. Quoting settles all of them, so \
             '$HOME' is six characters and reaches the program as one argument. The user \
             approves the compiled plan before anything runs, so say what you are running and \
             why first. \
             \
             Narrowing the result inside the line is the cheapest thing you can do, and usually \
             the difference between a few lines and a few thousand tokens: pipe to `head -n`, \
             ask grep for `-l` to get names only or `-c` for a count, take a line range with \
             `sed -n 40,80p` rather than reading a whole file. Use whichever tool returns less, \
             and a command that filters usually returns less. \
             \
             Output comes back as text you can read where the user has vouched for every \
             command in the line; otherwise it comes back as a reference, like a file you may \
             not read: pass it to read_output to ask the user to show it to you, hand it to \
             spawn_processor, or write it to a file with write_file. Do use it to compile and \
             test what you changed. \
             \
             A program meant to keep running, such as a server or a watcher, needs \
             background: true. Without it the line is waited on and killed after five minutes, \
             so there is no moment at which it is up and you can do anything with it.",
            json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "One command line. Programs are looked up on PATH, or \
                                        taken as paths relative to the workspace. A newline is not \
                                        accepted, since this is one line and not a script."
                    },
                    "background": {
                        "type": "boolean",
                        "description": "Leave it running instead of waiting for it, and hand \
                                        back a job name. For a program meant to keep going: a \
                                        server, a watcher, a log follower. Use it when you need \
                                        the program still up while you do something else, such \
                                        as starting a server and then fetching a page from it. \
                                        Call job_output with the name to see what it has \
                                        printed. Must be one pipeline with no redirection, and \
                                        it is killed when this turn ends. Defaults to false, \
                                        which waits and hands back the output."
                    }
                },
                "required": ["command"]
            }),
        ),
        Tool::function(
            "job_output",
            "See what a background job has printed, and whether it has ended. Give the job name \
             run handed back. Each call reports what is new since the last one, so calling it \
             again after doing something else shows what happened in between rather than \
             repeating what you have seen. Output is quarantined exactly as a run's is: it comes \
             back as text where the user vouched for every command in the line, and otherwise as \
             a reference to pass to read_output, spawn_processor or write_file.",
            json!({
                "type": "object",
                "properties": {
                    "job": {
                        "type": "string",
                        "description": "The job name run gave you, e.g. \"job:1\"."
                    },
                    "kill": {
                        "type": "boolean",
                        "description": "Stop the job after reading what it printed. Use it once \
                                        you are done with a server you started. Defaults to \
                                        false, which leaves it running."
                    }
                },
                "required": ["job"]
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
        Tool::function(
            "spawn_agent",
            "Hand a whole sub-task to a second agent and get back one report. It has its own              context, so everything it reads stays with it and only what it says at the end              reaches you. That is what this is for: running a build reads the whole log, and              asking a delegate to run the build tells you what failed. Use it when finding              something out would cost you more context than the answer is worth, or when a              sub-task is separable enough to describe in a paragraph. It cannot ask the user              anything and it cannot spawn one of its own, so give it everything it needs in the              task. Its writes and its runs are still shown to the user for approval, so this              saves you context and never an approval. Do not use it for something one read would              answer: it is a whole second agent and costs like one.",
            json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "description": "Which kind of agent, narrowest first. \"reader\" reads, \
                                        lists, searches and runs processors: use it to find \
                                        something out. \"checker\" also runs programs, so it \
                                        can build, test and lint, but writes nothing: use it to \
                                        find out whether something works. \"worker\" also \
                                        writes files: use it to finish a sub-task. Pick the \
                                        narrowest one that can do the job.",
                        "enum": ["reader", "checker", "worker"]
                    },
                    "task": {
                        "type": "string",
                        "description": "What it has to do, in your own words, as the whole of                                         what it will know. It cannot see this conversation, your                                         references, the user's prompt or anything you have read,                                         and it cannot come back for more, so name the paths, the                                         commands, the symptom and what a finished answer has to                                         contain. Say what you want reported, not just what you                                         want done."
                    },
                    "each": {
                        "type": "array",
                        "description": "Optional. Start one delegate per entry instead of one \
                                        in total, each told the task followed by its own entry. \
                                        Put what they share in task and only the differing part \
                                        here, e.g. one path per entry. Writing the same \
                                        paragraph out four times costs you the seconds it takes \
                                        to say it, and the delegates cannot start until you \
                                        have.",
                        "items": {"type": "string"}
                    }
                },
                "required": ["kind", "task"]
            }),
        ),
        Tool::function(
            "fetch_url",
            "Fetch an http or https URL. What comes back is quarantined, like a file nobody \
             vouched for: you get a reference rather than the text, and you cannot read it or be \
             told what it says. Hand the reference to spawn_processor to have a question answered \
             about it, or to write_file as contents_ref to save it. The user is asked before \
             anything leaves the machine.",
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The absolute http or https URL to fetch."
                    }
                },
                "required": ["url"]
            }),
        ),
    ];

    if self_paced {
        tools.push(Tool::function(
            "schedule_next",
            "Say when this loop should run again. The user started a loop with no interval, so \
             each turn sets the pace for the next one. Call this once, at the end of the turn, \
             after the work is done. You are choosing only the moment: the prompt is the user's \
             own line and it is sent again unchanged, so there is nothing here to say what the \
             next turn asks. Pick the wait from what you are actually waiting on, not from a \
             round number: something that takes ten minutes to change is not worth looking at in \
             one. Not calling it ends the loop, which is the right answer once there is nothing \
             left to watch.",
            json!({
                "type": "object",
                "properties": {
                    "delay_seconds": {
                        "type": "integer",
                        "description": "How long to wait before the next run. Held to between \
                                        60 and 3600 seconds, so anything outside that becomes \
                                        the nearest of the two."
                    },
                    "reason": {
                        "type": "string",
                        "description": "What you are waiting on, in a few words, e.g. \"watching \
                                        the release build\". The user reads this; it is not sent \
                                        anywhere and nothing acts on it."
                    },
                    "noop": {
                        "type": "boolean",
                        "description": "True when this run found nothing to do and changed \
                                        nothing, false when something happened worth keeping: an \
                                        edit, a message, a finding. Runs of quiet ticks are \
                                        counted, so the user can see the loop is healthy without \
                                        reading every one."
                    }
                },
                "required": ["delay_seconds", "noop"]
            }),
        ));
    }

    tools
}

/// The tools a delegate of one kind is offered.
///
/// Filtered from the one table rather than written again, so a schema improved for the planner is
/// improved for a delegate on the same line. What a capability reaches is the rule, because a tool
/// offered to a run whose gates refuse it on every call is a tool the model has to be told to
/// ignore.
///
/// Five are left out by name, each for its own reason:
///
/// - `spawn_agent`, because a delegate cannot delegate. The bound on a tree of them is the
///   product of the bounds, which is a number nobody chose, and a person approving a write at
///   the third level has no way to see which task it belongs to.
/// - `ask_user`, because a delegate's task came from a planner rather than from the person, so a
///   question about it asks somebody to arbitrate something they never set up.
/// - `todo_write`, because the list on the screen belongs to the turn the person is watching, and
///   a delegate writing to it would replace what they were reading with the steps of a sub-task
///   they did not ask about.
/// - `schedule_next`, because only a tick of a self-paced loop may say when the next is due, and
///   a delegate is not one.
/// - `fetch_url`, because every kind holds the capability for reaching the network so the driver
///   can make its model call, and that is the whole of what it buys. A delegate pointing a
///   request at a host of its own would be egress nobody approved for this sub-task, and the
///   person shown the host would be answering for a task they never set. The capability being
///   held is what makes this one a name and not a capability check: no gate would refuse it.
pub fn for_delegate(capabilities: &bravebot_core::capability::CapabilitySet) -> Vec<Tool> {
    use bravebot_core::capability::Capability;

    available(false)
        .into_iter()
        .filter(|tool| match tool.function.name.as_str() {
            "spawn_agent" | "ask_user" | "todo_write" | "schedule_next" | "fetch_url" => false,
            "write_file" | "edit_file" => capabilities.contains(Capability::FileWrite),
            "run" | "read_output" | "job_output" => capabilities.contains(Capability::ShellExec),
            // LSP-9: asking a server is its own grant, so a delegate holding file reads has not
            // thereby been given one. Named rather than left to the catch-all below, which would
            // hand it over with `FileRead`.
            "lsp" => capabilities.contains(Capability::LanguageServer),
            _ => capabilities.contains(Capability::FileRead),
        })
        .collect()
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
    /// Whether a cap cut the result short, so the turn can say so beside the reference it hands
    /// over. Text inside a quarantined result reaches nobody who could act on it.
    pub incomplete: bool,
    /// What a run printed, whole, where the cap cut down what the planner is shown.
    ///
    /// Quarantined by the turn loop, which is what mints slots, and the reference goes back beside
    /// the sample. The cap is on what enters the conversation, not on what the command printed, so
    /// the middle stays reachable: a planner that needs it hands the reference to a processor or
    /// writes it to a file rather than running the command again.
    pub whole: Option<Labelled<String>>,
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
    /// How long the call spent waiting on the model, where it called one.
    ///
    /// Travels beside [`Output::usage`] and for the same reason. A processor is a request like any
    /// other, and the turn was waiting on the endpoint for it: left out, the seconds would be
    /// charged to tool execution, and a turn that did most of its work in processors would read as
    /// one that ran a very slow subprocess.
    pub inference: std::time::Duration,
    /// The command whose output this is and how it ended, where a run produced it.
    ///
    /// Recorded on the slot by the turn loop, since only a slot minted from a command may be
    /// offered to the user for reading.
    pub printed_by: Option<crate::report::Command>,
    /// The media type, where what this produced is a picture.
    ///
    /// Recorded on the slot by the turn loop, and what makes a picture reach a processor as a part
    /// rather than as a body. Its presence is also what forces a reference: a picture is never put
    /// in the planner's context, whatever the label on the file it came from says.
    pub picture: Option<String>,
    /// When the planner asked for the next tick of a self-paced loop.
    ///
    /// Travels back to whoever started the loop, which is the only thing that knows there is one.
    /// A turn that says it twice is taken at its last word, since that is the one it ended on.
    pub wakeup: Option<crate::turn::Wakeup>,
    /// The delegates the kernel approved, for the turn to start.
    ///
    /// Started by the turn because a delegate outlives the call that asked for one: the call
    /// answers at once and the delegate goes on working, so what starts it has to be the thing
    /// still there when it finishes.
    ///
    /// A list because one call may fan a task out over several. Each is gated, numbered and
    /// seeded on its own, so what the turn starts is the same thing whether the planner asked
    /// for them one call at a time or all at once.
    pub delegate: Vec<(crate::report::DelegateId, crate::delegate::Seeded)>,
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
    /// Whether this turn is a tick of a self-paced loop, and so may say when the next is due.
    ///
    /// Read by dispatch as well as by the tool table, so a call to a tool this turn was not
    /// offered is answered the way any other unknown name is rather than quietly working.
    pub self_paced: bool,
    /// The user's own directory, where their standing instructions and skills live.
    ///
    /// Carried so a delegate discovers the same ones this turn did. Without it a delegate would
    /// find only what the workspace holds, and a user whose conventions live in their home
    /// directory would have them apply to the turn and not to the work it handed on.
    pub home: Option<&'a std::path::Path>,
    /// Whether this turn is itself a delegate's, and so may not spawn one, ask a person, write
    /// the task list on their screen, or reach a host.
    ///
    /// Read by dispatch as well as by the tool table, for the reason `self_paced` is: a delegate
    /// is offered none of those four, and a call it makes anyway has to be answered the way any
    /// other unknown name is rather than quietly working. Two refusals rather than one, because a
    /// rule resting on the tool list alone rests on the model reading it, and a model naming a
    /// tool it was never offered is ordinary.
    pub delegated: bool,
    /// The language servers this session has started, or `None` where the host offers none.
    ///
    /// Held across calls rather than per call because indexing is the whole cost of a server, and
    /// paying it per question would make this slower than the search it replaces. `None` for a
    /// host that cannot confine a subprocess: LSP-5 is MCP-3 applied here, so no confinement means
    /// no process, and the tool answers by saying so.
    pub servers: Option<&'a mut LanguageServers>,
    /// How many delegates this turn has spawned, which is what numbers the next one.
    ///
    /// Held by the turn rather than counted here, because a delegate is numbered once for the
    /// whole turn and a call is one round of it. The number is the driver's own and nothing a
    /// model wrote reaches it, which is what makes it usable for saying whose reports are whose.
    pub spawned: &'a mut u32,
    /// The pipelines this turn left running, by the reference each was given.
    ///
    /// Held by the turn so they end with it: a background job outliving the turn that started one
    /// would be an effect nobody is watching and nobody can stop.
    pub jobs: &'a mut Jobs,
    /// The standing answer this turn runs under, for the one mode that refuses rather than answers.
    ///
    /// Read by dispatch, for the reason `self_paced` and `delegated` are: a mode that refuses
    /// writes has to refuse them here, because the confirmer that carries the same mode is
    /// consulted only where something wanted to prompt. A path the trust map already covers, and a
    /// path a rule in the settings file allows, raise no prompt at all, so a refusal that waited
    /// for one would let exactly those writes through.
    pub permission_mode: crate::PermissionMode,
}

/// The background pipelines a turn has started.
///
/// Dropping this kills whatever is still running, so the turn ending is the end of them.
///
/// A job's name is not a reference and holds no content. It is a label the driver minted, which
/// makes it trusted and public and so usable as routing: the planner names one to say which
/// pipeline it means, exactly as it names a program. What a job *printed* is content, and that
/// comes back through the ordinary quarantine like anything else a program printed.
#[derive(Debug, Default)]
pub struct Jobs {
    running: std::collections::BTreeMap<String, Job>,
    /// How many have been started, so each gets a name of its own.
    ///
    /// Never reused within a turn, so a name cannot come to mean a second pipeline after the
    /// planner has been told what it means.
    started: usize,
}

/// One background pipeline, and what it was started as.
#[derive(Debug)]
struct Job {
    running: crate::exec::Background,
    /// The line as the person approved it, for the account given afterwards.
    line: String,
    /// The label its output carries, as the kernel fixed it before anything started.
    ///
    /// Kept rather than worked out again when the output is read. The label belongs to the plan a
    /// person answered for, and deriving it a second time later would be a second answer waiting
    /// to disagree: what a person vouched for can change during a turn, and a pipeline started
    /// before that must not have its output relabelled because of it.
    label: bravebot_core::label::Label,
    /// How many bytes have already been handed over, so a later look reports what is new.
    ///
    /// A count of bytes, never a comparison of them: nothing here reads what was printed.
    read: usize,
}

impl Jobs {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many are held, running or finished.
    pub fn len(&self) -> usize {
        self.running.len()
    }

    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }

    /// Take a background pipeline and hand back the name the planner will call it by.
    fn keep(
        &mut self,
        running: crate::exec::Background,
        line: String,
        label: bravebot_core::label::Label,
    ) -> String {
        self.started += 1;
        let name = format!("job:{}", self.started);
        self.running.insert(
            name.clone(),
            Job {
                running,
                line,
                label,
                read: 0,
            },
        );
        name
    }
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
    /// Whether the result is a sample rather than the whole answer, because a cap was reached.
    ///
    /// A fact about the result rather than text inside it, so it survives quarantine. The notice
    /// written into `text` reaches the planner only when the planner may read `text` at all,
    /// which by default it may not.
    incomplete: bool,
    /// What a run printed, whole, where a cap cut down what the planner is shown.
    ///
    /// The cap is on what enters the conversation, not on what the command printed, so the whole
    /// of it is quarantined by the turn and the planner is handed the reference beside the sample.
    /// Without it the one case where the cap bites would be the one case with no way back to the
    /// middle short of running the command again.
    whole: Option<Labelled<String>>,
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
    /// How long the tool waited on the model. Only a processor waits.
    inference: std::time::Duration,
    /// The command whose output this is and how it ended, where a run produced it.
    ///
    /// Recorded on the slot by the turn loop, because only a slot minted from a command may be
    /// offered to the user for reading.
    printed_by: Option<crate::report::Command>,
    /// When the planner asked for the next tick of a self-paced loop.
    wakeup: Option<crate::turn::Wakeup>,
    /// The media type, where what this produced is a picture.
    ///
    /// Recorded on the slot by the turn loop, and what makes a picture reach a processor as a part
    /// rather than as a body. The driver's own, from a table of extensions.
    picture: Option<String>,
    /// The name of a pipeline this call left running, where it started one.
    ///
    /// Reported so a person watching sees that something was started rather than run, which is a
    /// different thing to have agreed to.
    background: Option<String>,
    /// The delegates the kernel has approved and nobody has started yet.
    ///
    /// Started by the turn rather than here, because a delegate outlives the call that asked for
    /// one: the call answers immediately and the delegate goes on working. The turn is what is
    /// still there when it finishes.
    ///
    /// A list because one call may fan a task out over several.
    delegate: Vec<(crate::report::DelegateId, crate::delegate::Seeded)>,
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
            incomplete: false,
            whole: None,
            changes: Vec::new(),
            untrusted: false,
            unchanged_from: None,
            answers_for: None,
            said: None,
            content: false,
            usage: Usage::default(),
            inference: std::time::Duration::ZERO,
            printed_by: None,
            picture: None,
            wakeup: None,
            background: None,
            delegate: Vec::new(),
        }
    }

    /// A delegate the kernel approved, for the turn to start.
    fn delegating(
        mut self,
        id: crate::report::DelegateId,
        seeded: crate::delegate::Seeded,
    ) -> Self {
        self.delegate.push((id, seeded));
        self
    }

    /// Say that what this produced is workspace content, not the driver's words about it.
    fn of_content(mut self) -> Self {
        self.content = true;
        self
    }

    /// Say what this produced is a picture, and the media type it is to be sent under.
    ///
    /// Its slot is marked, which is what lets a processor be given it as a part rather than as a
    /// body. The planner is never shown it, whatever the label says.
    fn of_a_picture(mut self, media: &str) -> Self {
        self.picture = Some(media.to_string());
        self
    }

    /// Say a pipeline was left running under this name.
    ///
    /// The name is the driver's own, so the planner is told it as text rather than being handed a
    /// reference: there is nothing quarantined about it, and nothing has been printed yet.
    fn started_in_the_background(mut self, job: String) -> Self {
        self.text = Labelled::trusted(format!(
            "started in the background as {job}. Nothing has been read from it yet: call \
             job_output with \"{job}\" to see what it has printed, and again later for what is \
             new. It is killed when this turn ends."
        ));
        self.background = Some(job);
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
    /// Say the result was cut short by a cap, whoever ends up being allowed to read it.
    fn capped(mut self, incomplete: bool) -> Self {
        self.incomplete = incomplete;
        self
    }

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

    /// Say how long the tool waited on the model for this.
    fn waiting(mut self, inference: std::time::Duration) -> Self {
        self.inference = inference;
        self
    }

    /// Say when the planner asked for the next tick of a self-paced loop.
    fn scheduling(mut self, wakeup: crate::turn::Wakeup) -> Self {
        self.wakeup = Some(wakeup);
        self
    }
}

/// Whether a call by this name changes a file.
///
/// The two write tools named in one place, so the driver can ask the question without knowing
/// which they are. Asked of a name the planner sent, so the namespace some models put in front
/// comes off first, exactly as it does before the call is dispatched.
pub(crate) fn writes_a_file(name: &str) -> bool {
    matches!(strip_namespace(name), "write_file" | "edit_file")
}

/// Whether a call by this name runs a program.
pub(crate) fn runs_a_program(name: &str) -> bool {
    strip_namespace(name) == "run"
}

/// Whether this set of tools can run a program at all.
pub(crate) fn offer_runs(offered: &[Tool]) -> bool {
    offered
        .iter()
        .any(|tool| runs_a_program(&tool.function.name))
}

/// Whether this set of tools can change a file at all.
///
/// A run offered no write tool cannot be told to write something: a reader delegate is doing
/// exactly what it was built to do by never writing.
pub(crate) fn offer_writes(offered: &[Tool]) -> bool {
    offered
        .iter()
        .any(|tool| writes_a_file(&tool.function.name))
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

/// Which argument names what a call is about.
///
/// Chosen from the tool's own name, which dispatch already matches on, so this decides nothing
/// new. `None` for a tool with no single argument naming a target.
fn target_key(tool: &str) -> Option<&'static str> {
    match tool {
        "read_file" | "write_file" | "edit_file" => Some("path"),
        "list_files" => Some("directory"),
        "search" => Some("pattern"),
        "lsp" => Some("path"),
        "load_skill" => Some("name"),
        "fetch_url" => Some("url"),
        "job_output" => Some("job"),
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

        match target_text(arguments, key) {
            Some(text) => {
                let value = Labelled::new(text, bravebot_core::label::Label::untrusted_public());
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
            .and_then(|key| target_text(&parsed, key))
            .unwrap_or_default()
    };

    Activity::running(crate::report::verb_for(tool), target).line()
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
    // Some models namespace a tool by the group it was offered in: "functions_todo_write" and
    // "functions.todo_write" both mean todo_write, and answering "no such tool" to those spends
    // a round on a typo of our own making. The prefix is stripped before the name is matched
    // against the table, which is the driver's own list of literals: nothing the model writes
    // reaches anything but that comparison.
    let name = strip_namespace(&call.function.name).to_string();
    let verb = crate::report::verb_for(&name);

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
                incomplete: produced.incomplete,
                whole: produced.whole,
                unchanged_from: produced.unchanged_from,
                answers_for: produced.answers_for,
                said: produced.said,
                content: produced.content,
                usage: produced.usage,
                inference: produced.inference,
                printed_by: produced.printed_by,
                picture: produced.picture,
                wakeup: produced.wakeup,
                delegate: produced.delegate,
            };
        }
    };

    // Announced before the call runs, so a slow one is visible while it is slow. This is the
    // difference between a turn that looks stuck and one that is plainly working.
    let target = target_of(policy, &name, tools.slots, &arguments);
    reporter.tool_started(Activity::running(verb, target.clone()));

    let produced = match name.as_str() {
        // A mode that refuses writes refuses them whether or not anybody would have been asked,
        // which is what makes it a statement about the turn rather than an answer given on the
        // person's behalf. Checked before the tool runs, so nothing is read and no path resolved.
        writing if tools.permission_mode.refuses_writes() && writes_a_file(writing) => problem(
            "refused: this turn is in plan mode, so writing is refused however the user would \
             have answered. Do not retry; say what you would change and why.",
        ),
        "read_file" => read_file(policy, tools.workspace, tools.slots, confirmer, &arguments),
        "list_files" => list_files(policy, tools.workspace, &arguments),
        "search" => search(policy, tools.workspace, &arguments),
        "lsp" => lsp(policy, tools, confirmer, &arguments),
        "write_file" => write_file(policy, tools, confirmer, &arguments),
        "edit_file" => edit_file(policy, tools.workspace, tools.slots, confirmer, &arguments),
        // The list on the screen belongs to the turn the person is watching, so a delegate that
        // names this is answered the way any other unknown name is rather than replacing what
        // they were reading with the steps of a sub-task they did not ask about.
        "todo_write" if !tools.delegated => todo_write(policy, reporter, tools.slots, &arguments),
        "spawn_processor" => spawn_processor(policy, tools, &arguments),
        // A delegate is never offered this, so a call to it from one is answered the way any
        // other unknown name is rather than quietly starting a second level.
        "spawn_agent" if !tools.delegated => spawn_agent(policy, tools, reporter, &arguments),
        "load_skill" => load_skill(policy, tools.skills, &arguments),
        // A delegate's task came from a planner, so the question would ask the person to
        // arbitrate something they never set up. Refused here as well as absent from the list.
        "ask_user" if !tools.delegated => ask_user(policy, confirmer, &arguments),
        "run" => run(policy, tools, confirmer, &arguments),
        "read_output" => read_output(policy, tools, confirmer, &arguments),
        // A kind's network capability buys the driver's own model call and nothing a delegate can
        // point somewhere, so this one is refused here too: the gate would pass it, since the
        // capability really is held.
        "fetch_url" if !tools.delegated => fetch_url(policy, tools, confirmer, &arguments),
        "job_output" => job_output(policy, tools, &arguments),
        "schedule_next" if tools.self_paced => schedule_next(policy, &arguments),
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
        incomplete: produced.incomplete,
        whole: produced.whole,
        unchanged_from: produced.unchanged_from,
        answers_for: produced.answers_for,
        said: produced.said,
        content: produced.content,
        usage: produced.usage,
        inference: produced.inference,
        printed_by: produced.printed_by,
        picture: produced.picture,
        wakeup: produced.wakeup,
        delegate: produced.delegate,
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
        incomplete: false,
        whole: None,
        changes: Vec::new(),
        untrusted: false,
        unchanged_from: None,
        answers_for: None,
        said: None,
        wakeup: None,
        content: false,
        usage: Usage::default(),
        inference: std::time::Duration::ZERO,
        printed_by: None,
        picture: None,
        background: None,
        delegate: Vec::new(),
    }
}

/// A tool's own words about something that did happen, with what to show for it.
fn confirmed(text: impl Into<String>, note: impl Into<String>) -> Produced {
    Produced::new(Labelled::trusted(text.into()), "", note)
}

/// Extract a string argument the model supplied, labelled untrusted because it is.
/// A target argument as one line, whether it was given as a string or a list.
///
/// `search` takes either, and a call that named three patterns must not announce itself with a
/// blank where the subject goes. Joining is enough: this reaches a transcript line and a
/// progress line, and nothing compares it or routes on it.
fn target_text(arguments: &Value, key: &str) -> Option<String> {
    match arguments.get(key)? {
        Value::String(one) => Some(one.clone()),
        Value::Array(many) => {
            let parts: Vec<&str> = many.iter().filter_map(Value::as_str).collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }
        _ => None,
    }
}

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
    let (proposed, destination, shown_path, proposed_path) =
        (found.path, found.destination, found.shown, found.released);

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
    if destination == Destination::Reference && policy.read_is_quarantined(&proposed_path) {
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

    // A picture, decided from the extension: the driver's own table, so nothing read chooses this.
    //
    // Handed back as a reference whatever the trust map says, unlike text. A processor may look at
    // it and the planner may not, for the reason PASTE-2 gives: a picture in a planner's context
    // may only be one a person put there themselves, and a screenshot with words in it is exactly
    // the content this arrangement exists to keep out of it. So a vouched-for directory does not
    // make a picture readable, and there is no trust question here to ask.
    if let Some(media) = crate::workspace::media_for(&proposed_path) {
        return match workspace.read_attachment(policy, &path, media) {
            Ok(encoded) => {
                let weight = workspace.survey(&proposed_path).unwrap_or(0);
                Produced::new(
                    encoded,
                    shown_path,
                    format!("{media}, {} KiB", weight.div_ceil(1024)),
                )
                .of_content()
                .of_a_picture(media)
            }
            Err(e) => problem(format!("error: {e}")),
        };
    }

    // A file the planner may not see need not be opened yet. Whether it may see it is a question
    // about the trust map, keyed by the path it named, so nothing about any file's contents
    // reaches this branch.
    //
    // The offset and the limit do not enter it either. They describe a slice of what the planner
    // would have read, and a quarantined read hands back a reference rather than text, so there
    // is nothing for them to be a slice of: honouring them would mean opening the file at the
    // moment the planner asked, which is the one thing this branch exists to avoid. The reference
    // is of the file, and what is read from it is read when something needs the bytes.
    if policy.read_is_quarantined(&proposed_path) {
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
            let shown = policy
                .read_planner_argument(tool, "path", &path)
                .map_err(|denial| format!("refused: {denial}"))?;
            refuse_denied_path(policy, purpose, &shown)?;
            Ok(PathArgument {
                path,
                destination: Destination::Named,
                released: shown.clone(),
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
            //
            // Both hand the name back as well as the value, because this name is not the
            // planner's words and must not be read as though it were: it came out of a
            // directory nobody vouched for, which is the whole reason the reference exists.
            // The gate that released it is the one above, which is why nothing below asks a
            // second one for the same string.
            let (path, resolved) = match purpose {
                Purpose::Read => {
                    let promoted = policy
                        .promote_reference_for_read(tool, "path_ref", &slot, slots)
                        .map_err(|denial| format!("refused: {denial}"))?;
                    let resolved = promoted
                        .clone()
                        .into_trusted()
                        .map_err(|_| "error: the reference was not promoted".to_string())?;
                    (promoted, resolved)
                }
                Purpose::Effect => {
                    let named = policy
                        .destination_from_reference(tool, "path_ref", &slot, slots)
                        .map_err(|denial| format!("refused: {denial}"))?;
                    // Untrusted and public, which is what a name out of a directory nobody
                    // vouched for is. The endorsement is what will authorise it, not its label.
                    let path = Labelled::new(
                        named.clone(),
                        bravebot_core::label::Label::untrusted_public(),
                    );
                    (path, named)
                }
            };
            // A rule covers the file, not the spelling of it. A path that arrived through a
            // reference is the same file as one the planner typed, so the rule that would have
            // refused the second refuses the first. The path itself stays out of the refusal:
            // what goes back to the planner names the reference, as it does everywhere else.
            refuse_denied_path(policy, purpose, &resolved)
                .map_err(|_| denied_by_rule(&slot.to_string()))?;
            Ok(PathArgument {
                path,
                destination: Destination::Reference,
                shown: slot.to_string(),
                released: resolved,
            })
        }
    }
}

/// Refuse a path a `deny` rule covers, before the file is opened.
///
/// Both what the call will do and what it may see are asked about: a read consults the `Read`
/// rules, and an effect consults `Edit` and `Read` both, because a file nothing may read is not
/// protected if it can be overwritten.
fn refuse_denied_path<S: Sink>(
    policy: &mut Policy<'_, S>,
    purpose: Purpose,
    path: &str,
) -> Result<(), String> {
    let refused = match purpose {
        Purpose::Read => policy.before_read(path),
        Purpose::Effect => policy.before_write(path),
    };
    refused.map_err(|_| denied_by_rule(path))
}

/// What the planner is told when a rule refused. Says the rule is the reason and that retrying is
/// not the answer, because a planner told only "refused" tries the same call again.
fn denied_by_rule(shown: &str) -> String {
    format!(
        "refused: a deny rule in the user's settings covers {shown}, so nothing here can \
         reach it. Do not retry, and do not look for another way to the same file: work \
         without it, or say in your reply what you needed it for."
    )
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
    /// The same path as plain text, released once by whichever gate this came out of.
    ///
    /// Carried rather than read again by each caller. Reading it twice would put two identical
    /// lines in the trail for one path and read as the driver having looked at it twice, and for
    /// a reference it is not even the same string: `shown` is the reference's name.
    released: String,
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

    // The directory a listing would walk. A rule that keeps a tree from being read keeps it from
    // being enumerated too: the names in a directory are what is in it.
    let proposed_dir = match policy.read_planner_argument("list_files", "directory", &proposed) {
        Ok(directory) => directory,
        Err(denial) => return problem(format!("refused: {denial}")),
    };
    if let Err(refusal) = refuse_denied_path(policy, Purpose::Read, &proposed_dir) {
        return problem(refusal);
    }

    // A filter only narrows a confined, non-destructive read, so it is promotable on the
    // same terms as the directory itself.
    let pattern = match argument(arguments, "pattern") {
        Some(proposed) => match policy.promote_confined_read("list_files", "pattern", &proposed) {
            Ok(p) => Some(p),
            Err(denial) => return problem(format!("refused: {denial}")),
        },
        None => None,
    };

    // Read as a plain number, like the offset and limit on a read: it narrows a confined read
    // and cannot name anything, so there is no destination for it to decide.
    let depth = arguments
        .get("depth")
        .and_then(Value::as_u64)
        .map(|depth| depth.max(1).min(usize::MAX as u64) as usize);

    match workspace.list(policy, &directory, pattern.as_ref(), depth) {
        Ok(listing) => {
            let note = note_for(policy, "list_files", &listing, |listing| {
                tally(
                    listing.files.len() + listing.directories.len(),
                    "entry",
                    "entries",
                )
            });

            // A listing the planner may not read is handed over one reference per entry rather
            // than as one document it can do nothing with. The names stay wrapped the whole way:
            // this reshapes the listing into a list of them inside the kernel and carries it
            // out, and the kernel is what turns each into a slot.
            //
            // The label decides, not the contents: whether the planner may see these names is
            // the same question `present` would ask a moment later.
            // Shape, not content, and released for the same reason as the count below: a planner
            // handed exactly the cap with nothing said about it reads a sample as the whole tree.
            let truncated = {
                let shaped =
                    policy.render_in_place("list_files", &listing, |listing| listing.truncated);
                let proof = policy.authorise_display_release("whether a listing hit its cap");
                shaped.declassify(&proof)
            };

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
                    .capped(truncated)
                    .with_entries(Entries {
                        origin: format!("an entry in \"{proposed_dir}\""),
                        paths,
                        count,
                    });
            }

            let rendered = policy.render_in_place("list_files", &listing, |listing| {
                let mut entries: Vec<String> = listing.files.clone();
                entries.extend(listing.directories.iter().map(|name| format!("{name}/")));
                entries.sort();
                let listing = Listing {
                    files: entries,
                    directories: Vec::new(),
                    truncated: listing.truncated,
                };
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
                .of_content()
                .capped(truncated)
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
    // The path is routing, so naming a destination from it is not a content decision, and the
    // gate that released it ran where the argument was read.
    let (path, destination, shown_path, proposed_path) =
        (found.path, found.destination, found.shown, found.released);

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
        (Some(contents), None) => match policy.adopt_model_output("write_file", contents) {
            Ok(body) => body,
            Err(denial) => return problem(format!("refused: {denial}")),
        },
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
    let (proposed, destination, shown_path, proposed_path) =
        (found.path, found.destination, found.shown, found.released);
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

    // Both are the planner's own words, and locating a passage by comparing them against the
    // file is a decision taken from them. The gate is what says the planner's words may be read
    // at all: it refuses once this context has met anything untrusted, which is the moment they
    // stop being the planner's own.
    //
    // Asked before the file is opened. A refusal here means no edit is going to happen, and a
    // refusal that has already spent a read capability and put an observation in the trail is a
    // refusal that did something.
    let old_text = match policy.read_planner_argument("edit_file", "old_text", &old_text) {
        Ok(text) => text,
        Err(denial) => return problem(format!("refused: {denial}")),
    };
    let new_text = match policy.read_planner_argument("edit_file", "new_text", &new_text) {
        Ok(text) => text,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

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

    let replaced = match crate::replace::replace(&current, &old_text, &new_text, replace_all) {
        Ok(r) => r,
        Err(e) => return problem(format!("error: {e}")),
    };

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
            let headline = format!("edited {shown_path}: {occurrences} replacement(s)");

            // What the edit produced, not only that it produced something. A count of
            // replacements is not a result a planner can check: one wrote eighteen files in a
            // session on nothing but these lines, never saw a single one of them afterwards, and
            // never compiled any of it either. The lines around the change answer the question it
            // would otherwise have to spend a round asking.
            //
            // Shaped inside the kernel and handed over labelled, exactly as a read is, so
            // `Policy::present` decides whether the planner sees it. Nothing is declassified here
            // and no label is built by hand: the excerpt carries the label the edited body
            // carries, which is the integrity of the context this edit was worked out in.
            //
            // Only where that label is trusted. A quarantined excerpt would take the confirmation
            // down with it, and a planner that cannot be told its edit landed is worse off than
            // one that is told only that. The file itself is always trusted by this point:
            // `read_trusted_content` above refuses to locate a passage in anything else.
            if body_label.is_trusted() {
                let told = policy.render_in_place("edit_file", &body, |contents| {
                    match crate::replace::changed_region(&current, &contents) {
                        Some(excerpt) => format!("{headline}\n\n{excerpt}"),
                        None => headline.clone(),
                    }
                });
                Produced::new(told, "", note)
                    .with_changes(changes)
                    .marked_untrusted(false)
            } else {
                confirmed(headline, note)
                    .with_changes(changes)
                    .marked_untrusted(true)
            }
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

/// Say when the next tick of a self-paced loop is due.
///
/// The narrowest tool here. Nothing is read, nothing is written, and there is no path to endorse:
/// the whole of what it decides is how long the caller waits before sending the same prompt
/// again. What that prompt says is not on this surface at all, which is what makes the decision
/// one a person could have approved on its own. "Ask me that again in twenty minutes" is
/// readable without knowing what "that" is, and a field for the next prompt would have made it
/// unreadable.
///
/// The wait is held to its bounds where it is turned into a [`crate::turn::Wakeup`], so what the
/// planner is told back is what will actually happen rather than what it asked for.
fn schedule_next<S: Sink>(policy: &mut Policy<'_, S>, arguments: &Value) -> Produced {
    let Some(seconds) = arguments.get("delay_seconds").and_then(Value::as_u64) else {
        return problem("error: 'delay_seconds' is required, as a whole number of seconds");
    };
    let Some(quiet) = arguments.get("noop").and_then(Value::as_bool) else {
        return problem(
            "error: 'noop' is required: true where this run found nothing to do, false where \
             something happened",
        );
    };

    let wakeup = crate::turn::Wakeup::asked(seconds, quiet);
    let held = wakeup.after.as_secs();

    // The planner's own words about what it is waiting on, at the integrity of the context they
    // came from. They reach a screen and stop there: nothing waits on them, and no later turn
    // reads them back.
    let reason = policy.label_model_output(
        "schedule_next",
        arguments
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    );
    let note = note_for(policy, "schedule_next", &reason, move |reason| {
        let reason = reason.trim().to_string();
        if reason.is_empty() {
            format!("next in {held}s")
        } else {
            format!("next in {held}s: {reason}")
        }
    });

    Produced::new(
        Labelled::trusted(format!(
            "scheduled: this loop runs again in {held} seconds, sending the user's own prompt \
             unchanged"
        )),
        "",
        note,
    )
    .scheduling(wakeup)
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
    let Some(line) = argument(arguments, "command") else {
        return problem(
            "error: 'command' is required and must be a string holding one command line, \
             e.g. \"git log --oneline -50\"",
        );
    };

    // Assembled from the planner's own words, which are untrusted. Released through one witness,
    // so the trail records that a command line was released rather than leaving it to happen
    // implicitly. A person reading it is the legitimate destination: their reading it is what an
    // approval is.
    let proof = policy.authorise_display_release("a proposed command line");
    let line = line.declassify(&proof);

    let directory = tools.workspace.root().to_path_buf();
    let plan = match crate::cmdline::compile(&line, &directory, tools.home) {
        Ok(plan) => plan,
        // The refusal names the span that caused it, so the planner can rewrite that part rather
        // than guessing at the whole line. There is no degraded mode to fall back to.
        Err(refused) => return problem(format!("error: {refused}")),
    };

    // A redirection names a file the run opens itself, so the confinement every other write goes
    // through is applied here to the path.
    for path in &plan.writes {
        if let Err(escape) = tools.workspace.confines(path) {
            return problem(format!("refused: {escape}"));
        }
    }

    // Before the person is asked. A rule refusing something is a statement that it does not run,
    // and there is nothing to show or approve once it has been made.
    if let Err(denial) = policy.before_plan_rules(&plan) {
        return problem(format!(
            "refused: {denial}. Do not retry, and do not look for another program that would \
             do the same thing: say in your reply what you needed it for."
        ));
    }

    if policy.plan_needs_approval(&plan) {
        let request = crate::confirm::RunRequest { plan: plan.clone() };
        let answer = confirmer.confirm_run(&request);
        if !answer.approved() {
            return problem(
                "refused: the user did not approve running this. Do not retry the same \
                 line; ask what they would prefer."
                    .to_string(),
            );
        }
        // Recorded before the run, so a repeat of the same command later in this turn is not
        // asked about again. The policy carries it out of the turn and the session records it.
        //
        // Not for a line that feeds a file to a program. The prompt does not offer `a` for those,
        // and this is the same refusal at the layer that would act on it: what an entry records
        // is a program and its exact argv, and a `<` redirection is in neither, so an entry made
        // while one file was redirected in would cover the same program fed any other file. A
        // front end answering `always` anyway must not be able to widen the list that way.
        if answer.remember && !plan.releases_private() {
            for command in request.would_vouch_for() {
                policy.remember_command(command);
            }
        }
    }

    // The approval is what makes this plan trustworthy, and it is bound to this exact plan.
    policy.endorse_plan(&plan);

    let label = match policy.before_plan(&plan) {
        Ok(label) => label,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    let displayed = plan.display();

    // Absent or non-boolean means the foreground, which is the reading that waits for the program
    // and hands back what it printed.
    let in_the_background = arguments
        .get("background")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if in_the_background {
        // One pipeline, because that is the whole of what a long-lived program is. A line with
        // joins waits on its own parts to decide where to go next, and nothing waits here; a
        // redirection is a destination the background has no reader for.
        let steps = match &plan.steps {
            bravebot_core::command::Steps::Pipeline(steps)
                if plan.writes.is_empty() && plan.reads.is_empty() =>
            {
                steps
            }
            _ => {
                return problem(
                    "error: a background command must be one pipeline with no redirection. \
                     Run the parts separately, or run this one in the foreground.",
                );
            }
        };

        return match crate::exec::start_steps(steps, &plan.directory) {
            Ok(running) => {
                let name = tools.jobs.keep(running, displayed.clone(), label);
                Produced::new(
                    // Nothing has been printed yet, and the label is the one the kernel fixed
                    // before anything started: leaving it running does not make it trustworthier.
                    Labelled::new(String::new(), label),
                    format!("`{displayed}` started in the background"),
                    format!("started as {name}"),
                )
                .started_in_the_background(name)
            }
            Err(error) => problem(format!("error: `{displayed}` did not start: {error}")),
        };
    }

    // A redirection is a write, so the map has to say what its destination holds once the line
    // has run: untrusted bytes landing in a vouched-for tree must mark that path untrusted, or a
    // later read hands them back to the planner as trusted.
    //
    // What the run reports opening, never the plan's write set. The set names every branch, so a
    // destination a line decided against is in it, and a rule about a file nothing wrote would
    // quarantine a file the planner can read today. Spelled the way a read of the file is
    // spelled, because relative and absolute rules are separate namespaces in the map and a rule
    // in the wrong one decides nothing.
    let mut opened: Vec<std::path::PathBuf> = Vec::new();
    let ran = crate::exec::run_plan(&plan, tools.cancel, crate::exec::LIMIT, &mut opened);
    let written: Vec<String> = opened
        .iter()
        .map(|path| tools.workspace.relative_display(path))
        .collect();
    policy.reconcile_after_run(&written, label);

    match ran {
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

            // Capped only where the planner may read it. Output it may not read is quarantined
            // whole, so nothing of it enters the conversation and there is nothing to bound.
            let sample = if label.is_trusted() {
                bounded(&text)
            } else {
                None
            };
            let (text, whole) = match sample {
                // The cap bounds the conversation, not the run. What was printed is kept whole
                // beside the sample, so the middle is still there to hand to a processor or write
                // to a file, and nothing has to be run twice to see it.
                Some(sample) => (sample, Some(Labelled::new(text, label))),
                None => (text, None),
            };

            // Said in the driver's own words, from the exit codes and the clock, which are
            // structure rather than content: nothing here reads a byte of what the program
            // printed.
            let outcome = if let Some(after) = ran.stopped {
                crate::report::Outcome::Stopped(after)
            } else if ran.ended_well {
                crate::report::Outcome::Succeeded
            } else {
                let failed: Vec<String> = ran
                    .failures()
                    .iter()
                    .map(|(at, code)| match code {
                        Some(code) => format!("step {at} exited {code}"),
                        None => format!("step {at} was killed"),
                    })
                    .collect();
                crate::report::Outcome::Failed(failed.join(", "))
            };
            let lines = text.lines().count();
            let note = format!("{}, {}", outcome.summary(), tally(lines, "line", "lines"));

            let mut produced = Produced::new(
                Labelled::new(text, label),
                format!("what `{displayed}` printed"),
                note,
            )
            .of_content()
            .capped(whole.is_some());
            produced.whole = whole;
            // Marks the block the person is shown as content nobody vouched for, which is what a
            // program's output is: it may include bytes an earlier step read out of a file an
            // attacker wrote.
            produced.untrusted = !label.is_trusted();
            // What the slot will be told it came from, so the user can be asked to read it later
            // and can see which command they are reading, beside how it went.
            produced.printed_by = Some(crate::report::Command {
                line: displayed.clone(),
                outcome,
            });
            produced
        }
        // A run that produced nothing still says what happened. The plan is safe to repeat back:
        // a person endorsed it, so it is not something an attacker chose.
        Err(error) => problem(format!("error: `{displayed}` did not run: {error}")),
    }
}

fn fetch_url<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    tools: &mut Tools<'_>,
    confirmer: &mut C,
    arguments: &Value,
) -> Produced {
    let Some(proposed) = argument(arguments, "url") else {
        return problem(
            "error: 'url' is required and must be a string holding an http or https URL",
        );
    };

    // The planner's own words, so untrusted. Released through one witness, because the legitimate
    // destination is a person reading it: their reading it is what the approval is.
    let proof = policy.authorise_display_release("a proposed url");
    let url = proposed.declassify(&proof);

    // Worked out here rather than in the prompt, so what a person is asked about is the host the
    // request will reach and not whatever the string looks like it names.
    let Some(host) = bravebot_core::url::host_of(&url) else {
        return problem(format!(
            "error: '{url}' names no host to fetch from; give an absolute http or https URL"
        ));
    };

    if policy.fetch_needs_approval(&url) {
        let request = crate::confirm::FetchRequest {
            url: url.clone(),
            host: host.clone(),
        };
        if confirmer.confirm_fetch(&request) == Decision::Reject {
            return problem(
                "refused: the user did not approve fetching this. Do not retry the same URL; \
                 ask what they would prefer."
                    .to_string(),
            );
        }
    }

    // Bound to this exact URL, so an approval cannot be spent on another.
    policy.endorse_fetch(&url);
    let label = match policy.before_fetch(&url) {
        Ok(label) => label,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    let request = bravebot_net::Request::get(&url);
    let fetched = tools
        .chat
        .egress
        .fetch_watching(policy, request, label, Some(tools.cancel));
    // Before anything returns, so a failure does not leave the rest of the turn's egress being
    // checked against the host this one call was approved for.
    policy.fetch_finished();

    match fetched {
        Ok(response) => {
            // Decoded lossily rather than refused for not being text. What a server sends is
            // untrusted either way, and a page with one bad byte is still the page that was asked
            // for: nothing here reads it, so there is nothing for a decoding failure to protect.
            let label = response.body.label();
            let (bytes, body_label) = policy
                .decode_transport("fetch_url", label)
                .decode(response.body);
            let text = String::from_utf8_lossy(&bytes).into_owned();

            let note = format!(
                "{}, {}",
                response.status,
                tally(text.lines().count(), "line", "lines")
            );

            // The URL as it was requested, not `final_url`. A redirect chain ends somewhere the
            // person approving never saw, and naming that in the transcript would present a host
            // nobody agreed to as though they had.
            let mut produced = Produced::new(
                Labelled::new(text, body_label),
                format!("what {url} returned"),
                note,
            )
            .of_content()
            .capped(response.truncated);
            // Always. A fetched body is never trusted, so the block a person is shown is always
            // marked as content nobody vouched for.
            produced.untrusted = true;
            produced
        }
        // The URL is safe to repeat: a person approved it, so it is not something an attacker
        // chose. Nothing of the response is, and none of it is read to build this.
        Err(error) => problem(format!("error: fetching {url} failed: {error}")),
    }
}

/// What a background pipeline has printed since it was last looked at.
///
/// The job name is routing, and it is the driver's own: a name this module minted and looked up in
/// its own map, so nothing the planner writes reaches anything but that lookup. The output is
/// content and carries the label the kernel fixed before the pipeline started.
fn job_output<S: Sink>(
    policy: &mut Policy<'_, S>,
    tools: &mut Tools<'_>,
    arguments: &Value,
) -> Produced {
    let Some(named) = argument(arguments, "job") else {
        return problem("error: 'job' is required and must be a job name, e.g. \"job:1\"");
    };

    // Released for a lookup against names the driver handed out, which is the same treatment a
    // reference gets. Nothing is decided from it beyond whether it is one of ours.
    let proof = policy.authorise_display_release("a job name the planner asked about");
    let name = named.declassify(&proof);

    let kill = arguments
        .get("kill")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let Some(job) = tools.jobs.running.get_mut(&name) else {
        return problem(format!(
            "error: there is no background job called '{name}'. Only a job name run handed back \
             in this turn can be read, and they do not outlive the turn."
        ));
    };

    let ended = job.running.ended();
    let printed = job.running.printed();
    // Bytes, so a partial line is not counted as read. The count is the driver's own bookkeeping
    // and nothing here compares what was printed.
    let fresh = printed.get(job.read..).unwrap_or_default().to_string();
    job.read = printed.len();

    let ran_for = job.running.ran_for();
    let line = job.line.clone();
    let label = job.label;

    if kill {
        job.running.kill();
    }

    // Said from the clock and the exit codes, which are structure: nothing here reads a byte of
    // what the pipeline printed.
    let outcome = if kill {
        "killed".to_string()
    } else if ended {
        let failed: Vec<String> = job
            .running
            .codes()
            .iter()
            .enumerate()
            .filter(|(_, code)| **code != Some(0))
            .map(|(at, code)| match code {
                Some(code) => format!("step {} exited {code}", at + 1),
                None => format!("step {} was killed", at + 1),
            })
            .collect();
        if failed.is_empty() {
            "ended, every step succeeded".to_string()
        } else {
            format!("ended: {}", failed.join(", "))
        }
    } else {
        format!("still running after {} seconds", ran_for.as_secs())
    };

    let note = format!(
        "{outcome}, {}",
        tally(fresh.lines().count(), "new line", "new lines")
    );

    // Capped only where the planner may read it, exactly as a foreground run is: output it may not
    // read is quarantined whole, and there is nothing of it in the conversation to bound.
    let sample = if label.is_trusted() {
        bounded(&fresh)
    } else {
        None
    };
    let (fresh, whole) = match sample {
        Some(sample) => (sample, Some(Labelled::new(fresh, label))),
        None => (fresh, None),
    };

    let mut produced = Produced::new(
        Labelled::new(fresh, label),
        format!("what `{line}` has printed"),
        note,
    )
    .of_content()
    .capped(whole.is_some());
    produced.whole = whole;
    produced.untrusted = !label.is_trusted();
    // So a person can be asked to read it later, and can see which command they are reading.
    produced.printed_by = Some(crate::report::Command {
        line,
        outcome: if ended || kill {
            crate::report::Outcome::Succeeded
        } else {
            crate::report::Outcome::Stopped(ran_for)
        },
    });
    produced
}

/// How much of a command's output may enter the conversation.
///
/// A context-budget decision and not a safety one: a single tool result must never be able to
/// spend a large fraction of a conversation, however useful what it printed was.
const OUTPUT_CAP: usize = 16 * 1024;

/// `text` cut to [`OUTPUT_CAP`], keeping the head and the tail, or `None` where it fits.
///
/// Head and tail rather than head alone, because a build log's verdict is at the end and its first
/// error is near the beginning: keeping only the front of one answers neither question a reader
/// has. What went is said in between, in the driver's own words, so a planner knows it is looking
/// at a sample rather than at a short result. Where the rest of it went is said by the turn,
/// which is the only thing that knows the slot it went to.
fn bounded(text: &str) -> Option<String> {
    if text.len() <= OUTPUT_CAP {
        return None;
    }
    let half = OUTPUT_CAP / 2;
    // Cut on a character boundary, or a multi-byte character straddling the cut would panic the
    // turn on output nobody chose.
    let head_end = text
        .char_indices()
        .map(|(at, _)| at)
        .take_while(|at| *at <= half)
        .last()
        .unwrap_or(0);
    let tail_start = text
        .char_indices()
        .map(|(at, _)| at)
        .find(|at| *at >= text.len() - half)
        .unwrap_or(text.len());

    let dropped_bytes = tail_start - head_end;
    let dropped_lines = text[head_end..tail_start].lines().count();
    Some(format!(
        "{}\n\n(the middle of this output was dropped: {dropped_bytes} bytes, \
         about {dropped_lines} lines.)\n\n{}",
        &text[..head_end],
        &text[tail_start..]
    ))
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

    let asked_at = std::time::Instant::now();
    let answer = processor::run(policy, &mut tools.chat, tools.slots, &spec);
    // Taken around the call rather than inside it, so a processor that failed still reports the
    // time it spent failing: a request that errored kept the turn waiting just as long.
    let waited = asked_at.elapsed();
    match answer {
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
                .costing(done.usage)
                .waiting(waited);
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
                .costing(done.usage)
                .waiting(waited);
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
                .waiting(waited)
                .of_content();
            produced.unchanged_from = done.unchanged_from;
            produced.answers_for = Some(spec.about().cloned());
            produced.said = done.note;
            produced
        }
        Err(error) => problem(format!("error: {error}")).waiting(waited),
    }
}

/// Hand a sub-task to a second planner and bring back one report.
///
/// Everything about the delegate is fixed by the kernel before it exists: which capabilities it
/// holds, which prompt it reads, and how many rounds it may take. What arrives here is a name out
/// of an enumerated set and a paragraph, and neither can widen anything.
///
/// The report comes back as a labelled value and is presented like any other tool result, so the
/// kernel decides whether the planner is shown the words or a reference to them. Nothing here
/// reads it, and nothing here could: a delegate's answer is model output, and which of the two
/// shapes it takes is settled by the label its own context gave it.
fn spawn_agent<S: Sink, R: Reporter>(
    policy: &mut Policy<'_, S>,
    tools: &mut Tools<'_>,
    reporter: &mut R,
    arguments: &Value,
) -> Produced {
    let Some(kind) = argument(arguments, "kind") else {
        return problem(format!(
            "error: 'kind' is required and must be one of {}",
            bravebot_core::delegate::Kind::NAMES.join(", ")
        ));
    };
    let tasks = match tasks_in(arguments) {
        Ok(tasks) => tasks,
        Err(refusal) => return problem(refusal),
    };

    let mut produced = Produced::new(
        Labelled::trusted(String::new()),
        String::new(),
        String::new(),
    );
    let mut started = Vec::new();
    let mut kind_name = "";

    for task in &tasks {
        // The trail's name for it is the driver's own word, not the task: a task is a paragraph,
        // and it would be in every line of the trail that mentions this run.
        //
        // Gated once per delegate rather than once per call. A fan-out is several runs, and a
        // gate that saw one of them would be approving the others on the strength of a sibling.
        let spec = match policy.before_delegate("delegate", &kind, task) {
            Ok(spec) => spec,
            Err(denial) => return problem(format!("refused: {denial}")),
        };

        // Numbered by the driver, in the order this turn spawned them. Everything reported about
        // this delegate carries the number, which is the only thing saying whose a line is: the
        // alternative is reading the line, which is prose a model wrote.
        //
        // The task goes with it, released for a screen the way the target of any other call is. A
        // person watching several delegates has nothing else to tell them apart by, and a fan-out
        // is exactly where several of them read alike.
        *tools.spawned += 1;
        let id = crate::report::DelegateId::nth(*tools.spawned);
        let asked = {
            let proof = policy.authorise_display_release("what a delegate was asked to do");
            task.clone().declassify(&proof)
        };
        reporter.delegate_started(crate::report::Delegation {
            id,
            kind: spec.kind().as_str(),
            task: asked,
        });

        kind_name = spec.kind().as_str();
        started.push(id.to_string());

        // Everything the kernel settled, taken off the policy here on the turn's own thread. From
        // this point the delegate needs nothing further from the run that spawned it, which is
        // what lets the two run at the same time.
        let seeded = crate::delegate::seed(policy, spec);
        produced = produced.delegating(id, seeded);
    }

    // Started rather than finished. The turn hears about the report when there is one, and in the
    // meantime the planner has its round back: a turn that had to sit still until a delegate
    // answered could only ever have one working.
    let named = started.join(", ");
    let body = if started.len() == 1 {
        format!(
            "the {kind_name} delegate {named} has started. Its report will reach you when it is \
             ready, and you do not have to wait for it: carry on, or spawn another. You will be \
             told what it said before you are asked to answer."
        )
    } else {
        format!(
            "{} {kind_name} delegates have started: {named}. Each reports on its own and you do \
             not have to wait for any of them: carry on, or spawn another. You will be told what \
             they said before you are asked to answer.",
            started.len()
        )
    };
    let note = if started.len() == 1 {
        format!("a {kind_name} delegate started")
    } else {
        format!("{} {kind_name} delegates started", started.len())
    };

    produced.text = Labelled::trusted(body);
    produced.origin = format!("a {kind_name} delegate");
    produced.note = note;
    produced
}

/// The most delegates one call may fan a task out over.
///
/// A bound on futility rather than on authority: the planner could always start this many with
/// this many calls, and each is gated on its own either way. What changes is the cost of asking,
/// and a field that turns one sentence into forty runs is a field worth a ceiling.
const MAX_FANOUT: usize = 8;

/// The tasks one `spawn_agent` call asks for, one per delegate.
///
/// Without `each` that is the task itself. With it, the task is what they share and each entry is
/// what one of them is additionally told, composed here before anything is labelled: both halves
/// are the same model output out of the same call, so joining them settles nothing about either.
fn tasks_in(arguments: &Value) -> Result<Vec<Labelled<String>>, String> {
    let Some(task) = arguments.get("task").and_then(Value::as_str) else {
        return Err(
            "error: 'task' is required and must be a string saying what the delegate has to do"
                .to_string(),
        );
    };

    let Some(each) = arguments.get("each") else {
        return Ok(vec![Labelled::new(
            task.to_string(),
            bravebot_core::label::Label::untrusted_public(),
        )]);
    };

    let Some(entries) = each.as_array() else {
        return Err("error: 'each' must be an array of strings, one per delegate".to_string());
    };
    if entries.is_empty() {
        return Err(
            "error: 'each' was empty, so it named no delegates; leave it out to start one"
                .to_string(),
        );
    }
    if entries.len() > MAX_FANOUT {
        return Err(format!(
            "error: 'each' named {} delegates and at most {MAX_FANOUT} may be started by one \
             call; split the work or narrow it",
            entries.len()
        ));
    }

    entries
        .iter()
        .map(|entry| {
            let Some(entry) = entry.as_str() else {
                return Err(
                    "error: every entry in 'each' must be a string saying what that one \
                     delegate is additionally told"
                        .to_string(),
                );
            };
            Ok(Labelled::new(
                format!("{task}\n\n{entry}"),
                bravebot_core::label::Label::untrusted_public(),
            ))
        })
        .collect()
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

/// Whether an `include` glob leans on syntax the matcher does not have.
///
/// A pattern the matcher cannot read selects no files, and a search over no files reports no
/// matches, which is the same sentence a search that read the whole tree and found nothing
/// prints. A real turn took that for proof and answered the question wrong: the glob it wanted
/// was `**/*.{cc,h,mm}`, brace groups were not supported at the time, and it retreated to
/// `**/*.cc`, dropping the two extensions the answer was actually in.
///
/// Braces are supported now. This is for what is still missing, and for the next thing to be
/// added: the point is that an unreadable pattern must never come back looking like a fact
/// about the tree.
fn reads_as_an_unsupported_glob(pattern: &str) -> Option<&'static str> {
    // Extended globs. Checked before the bracket test, which their contents would otherwise
    // trip with a less useful message.
    for tell in ["?(", "*(", "+(", "@(", "!("] {
        if pattern.contains(tell) {
            return Some(
                "extended globs like !(…) and +(…) are not supported; name the files with * \
                 and ** instead",
            );
        }
    }
    if pattern.starts_with('!') {
        return Some(
            "a leading ! does not negate a pattern here; search without an include instead",
        );
    }
    // A character class. Both halves are required, in order, so a filename holding a stray
    // bracket is not lectured at.
    if let Some(open) = pattern.find('[')
        && pattern[open..].contains(']')
    {
        return Some(
            "character classes like [abc] are not supported; write the alternatives as a \
             brace group, e.g. {a,b,c}",
        );
    }
    None
}

/// The patterns a search was asked for.
///
/// One string or a list of them, because the model writes both and the difference is not
/// worth a failed call. A list entry that is not a string is dropped here and the emptiness
/// is refused below, which is the same shape `references_in` uses.
fn patterns_in(arguments: &Value) -> Vec<Labelled<String>> {
    let label = bravebot_core::label::Label::untrusted_public();
    match arguments.get("pattern") {
        Some(Value::Array(entries)) => entries
            .iter()
            .filter_map(Value::as_str)
            .map(|p| Labelled::new(p.to_string(), label))
            .collect(),
        Some(Value::String(one)) => vec![Labelled::new(one.clone(), label)],
        _ => Vec::new(),
    }
}

/// Ask a language server about a symbol.
///
/// The two halves of the answer take different roads out of here, which is [LSP-3]:
///
/// - The **locations** are written into a line by the driver, from a path and two integers it read
///   off the server's index. That line is the driver's own words, so it is trusted, the same footing
///   a line count or an exit status reaches the planner on. Nothing a file wrote is in it.
/// - The **text**, where an operation reports any, is bytes a file chose, and no answer says which
///   file chose them: a hover response carries a position and no file. So it is untrusted and comes
///   back as a reference, in a vouched-for tree as much as in `vendor/`, with the locations listed
///   either way.
///
/// [LSP-3]: ../../../docs/specs/tools/lsp.md
fn lsp<S: Sink, C: Confirmer + ?Sized>(
    policy: &mut Policy<'_, S>,
    tools: &mut Tools<'_>,
    confirmer: &mut C,
    arguments: &Value,
) -> Produced {
    let Some(named) = argument(arguments, "operation") else {
        return problem("error: 'operation' is required");
    };

    // The operation is routing: it decides what the server is asked. Promoted like any other
    // proposal, then matched against the closed set, so a name off the list is refused here rather
    // than forwarded on the chance a server understands it.
    let operation = match policy.promote_confined_read("lsp", "operation", &named) {
        Ok(promoted) => match promoted.clone().into_trusted() {
            Ok(name) => match bravebot_lsp::Operation::parse(&name) {
                Some(operation) => operation,
                None => {
                    return problem(format!(
                        "refused: {}",
                        bravebot_lsp::LspError::UnknownOperation { named: name }
                    ));
                }
            },
            Err(_) => return problem("refused: the operation was not trusted"),
        },
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    // A position is two integers the planner states. Nothing reads the file to work one out, which
    // is what keeps this tool clear of LABEL-5: a position derived by scanning bytes would be a
    // decision taken from content.
    let line = arguments
        .get("line")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let character = arguments
        .get("character")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;

    // `workspaceSymbol` ranges over the tree instead of starting from a position, so it is the one
    // operation with no path to promote.
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .map(str::to_string);

    let relative = if operation.needs_position() {
        let Some(proposed) = argument(arguments, "path") else {
            return problem(format!(
                "error: 'path' is required for {}",
                operation.as_str()
            ));
        };
        let promoted = match policy.promote_confined_read("lsp", "path", &proposed) {
            Ok(promoted) => promoted,
            Err(denial) => return problem(format!("refused: {denial}")),
        };
        let named = match promoted.clone().into_trusted() {
            Ok(named) => named,
            Err(_) => return problem("refused: the path was not trusted"),
        };
        // A deny rule covering the file covers asking a server about it too: the answer quotes
        // where things are in it, so this is a read.
        if let Err(refusal) = refuse_denied_path(policy, Purpose::Read, &named) {
            return problem(refusal);
        }
        named
    } else {
        String::new()
    };

    let Some(servers) = tools.servers.as_deref_mut() else {
        // LSP-5, and it is not a fault: a host that cannot confine a subprocess does not get to run
        // one, and saying so is better than an answer nobody could trust.
        return problem(
            "refused: no language server can be started here, because this platform offers no \
             way to confine one. Use search and read_file instead.",
        );
    };

    // The server is given the path it can open, which is the resolved one. What the planner is
    // shown is rendered back against the root by the lsp module.
    let root = servers.root().to_path_buf();
    let absolute = if relative.is_empty() {
        String::new()
    } else {
        root.join(&relative).to_string_lossy().into_owned()
    };

    let answer = match servers.ask(
        policy,
        confirmer,
        &bravebot_lsp::Question {
            operation,
            path: &absolute,
            line,
            character,
            query: query.as_deref(),
        },
    ) {
        Ok(answer) => answer,
        // LSP-6: every one of these says which failure it was, and none of them reads as a
        // statement about the code. A planner told "no references" deletes a function.
        Err(error) => return problem(format!("refused: {error}")),
    };

    let described = crate::lsp::describe(operation, &answer, &root);
    let found = answer.locations.len();

    // The text is the half that is content, and nothing here labels it by the path this call
    // named: a doc comment is written where the symbol is defined, which is a different file. The
    // kernel decides whether the planner sees it. Where there is no text, the result is the
    // driver's own words about where things are, which is trusted.
    let text = match crate::lsp::text_of(policy, &answer) {
        Some(Ok(text)) => Some(text),
        Some(Err(denial)) => return problem(format!("refused: {denial}")),
        None => None,
    };

    let note = format!(
        "{}{}",
        tally(found, "location", "locations"),
        if answer.partial {
            ", still indexing"
        } else {
            ""
        }
    );

    match text {
        // Hover text exists, so the result carries it and is labelled by its file. The locations go
        // in front of it: they are structure, and a reader who cannot see the text can still act on
        // them.
        Some(text) => {
            let untrusted = !text.label().is_trusted();
            let combined =
                policy.render_in_place("lsp", &text, |text| format!("{described}\n\n{text}"));
            let mut produced = Produced::new(combined, relative, note);
            produced.content = true;
            produced.untrusted = untrusted;
            produced.incomplete = answer.partial;
            produced
        }
        // Locations only, which is every operation but hover. The line is the driver's, composed
        // from structure, so there is nothing here to quarantine.
        None => {
            let mut produced = Produced::new(Labelled::trusted(described), relative, note);
            produced.incomplete = answer.partial;
            produced
        }
    }
}

fn search<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    arguments: &Value,
) -> Produced {
    let proposed_patterns = patterns_in(arguments);
    if proposed_patterns.is_empty() {
        return problem("error: 'pattern' is required and must be a string or a list of strings");
    }

    let mut patterns = Vec::with_capacity(proposed_patterns.len());
    for proposed in &proposed_patterns {
        match policy.promote_confined_read("search", "pattern", proposed) {
            Ok(p) => patterns.push(p),
            Err(denial) => return problem(format!("refused: {denial}")),
        }
    }

    let proposed_dir = argument(arguments, "directory").unwrap_or_else(|| {
        Labelled::new(
            ".".to_string(),
            bravebot_core::label::Label::untrusted_public(),
        )
    });

    let directory = match policy.promote_confined_read("search", "directory", &proposed_dir) {
        Ok(d) => d,
        Err(denial) => return problem(format!("refused: {denial}")),
    };

    // The directory a search would walk, on the same footing as a listing: a match quotes the
    // line it was found on, so searching a tree is reading it.
    let proposed_where = match policy.read_planner_argument("search", "directory", &proposed_dir) {
        Ok(directory) => directory,
        Err(denial) => return problem(format!("refused: {denial}")),
    };
    if let Err(refusal) = refuse_denied_path(policy, Purpose::Read, &proposed_where) {
        return problem(refusal);
    }

    let include = match argument(arguments, "include") {
        Some(proposed) => match policy.promote_confined_read("search", "include", &proposed) {
            Ok(p) => Some(p),
            Err(denial) => return problem(format!("refused: {denial}")),
        },
        None => None,
    };

    // Case sensitivity is a property of the call, not of anything read, so it is taken from
    // the arguments directly. Absent means sensitive: a search that quietly widened itself
    // would report matches the caller cannot see the reason for.
    let case_sensitive = arguments
        .get("case_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    match workspace.grep(
        policy,
        &patterns,
        &directory,
        include.as_ref(),
        case_sensitive,
    ) {
        Ok(found) => {
            let note = note_for(policy, "search", &found, |found| {
                format!(
                    "{} in {}",
                    tally(found.matches.len(), "match", "matches"),
                    tally(found.searched, "file", "files")
                )
            });

            // Either cap leaves the answer partial, and the distinction between them matters to
            // whoever reads the body. To the turn it does not: a sample is a sample.
            let incomplete = {
                let shaped = policy.render_in_place("search", &found, |found| {
                    found.truncated || found.unvisited || found.timed_out
                });
                let proof = policy.authorise_display_release("whether a search hit a cap");
                shaped.declassify(&proof)
            };
            // Asked of the glob alone, and only where the promote gate left it trusted, so this
            // reads no content and needs no witness. Nothing here looks at the result: whether to
            // say it is decided below, inside the render gate, from whether anything was found.
            let bad_glob = include.as_ref().and_then(|include| {
                include
                    .clone()
                    .into_trusted()
                    .ok()
                    .and_then(|g| reads_as_an_unsupported_glob(&g))
            });
            let had_include = include.is_some();

            let rendered = policy.render_in_place("search", &found, |found| {
                let mut body = if found.matches.is_empty() {
                    // The three ways a search comes back empty, which used to print the same
                    // sentence. Files were read and the needle was not in them, which is an
                    // answer. Or the include glob selected nothing, so nothing was read and
                    // the tree was never asked. Or a rule covers what was selected, which no
                    // query can get around. Told apart here because a reader who cannot tell
                    // them apart takes a broken query for proof of absence, and rewrites a glob
                    // that was never the problem.
                    if found.withheld && found.considered == 0 {
                        "(a deny rule in the user's settings covers what this search would \
                         have read, so nothing was searched. Do not retry with another \
                         pattern or glob: work without it, or say in your reply what you \
                         needed it for)"
                            .to_string()
                    } else if had_include && found.considered == 0 {
                        let mut text = "(the include glob matched no files, so nothing was \
                                        searched; this says nothing about whether the pattern \
                                        is in the tree)"
                            .to_string();
                        if let Some(advice) = bad_glob {
                            text.push_str(&format!("\n\n({advice})"));
                        }
                        text
                    } else {
                        "(no matches)".to_string()
                    }
                } else {
                    found
                        .matches
                        .iter()
                        .map(|m| format!("{}:{}: {}", m.path, m.line, m.text))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                // The empty result is the one that most needs this. A search that stopped before
                // it reached the file holding the needle reports nothing, and nothing reads as an
                // answer: the model concludes the string does not occur in the tree.
                if found.unvisited {
                    // The count the walk actually stopped at, not the constant it usually is.
                    // A host may have lowered the cap, and a notice naming a number the search
                    // did not reach is worse than one naming none.
                    body.push_str(&format!(
                        "\n\n(this search stopped after {} files and did not reach the whole \
                         tree; search a subdirectory to cover the rest)",
                        found.considered
                    ));
                }
                if found.timed_out {
                    body.push_str(&format!(
                        "\n\n(this search ran out of time after {} files and did not read the \
                         rest; narrow it with a directory or an include glob)",
                        found.searched
                    ));
                }
                if found.truncated {
                    // Without this a model that gets exactly the cap concludes it has
                    // every occurrence, which is how a rename misses call sites.
                    body.push_str(&format!(
                        "\n\n(this search stopped at {} matches and is incomplete; narrow the \
                         pattern or search a subdirectory)",
                        found.matches.len()
                    ));
                }
                body
            });
            Produced::new(rendered, proposed_where, note)
                .of_content()
                .capped(incomplete)
        }
        Err(e) => problem(format!("error: {e}")),
    }
}

#[cfg(test)]
mod tests {

    /// A glob the matcher cannot read selects no files, and a search over no files reports no
    /// matches, which is the sentence a search that read the whole tree and found nothing
    /// prints. Saying which syntax was the problem is what keeps the two apart.
    #[test]
    fn a_glob_leaning_on_missing_syntax_is_named() {
        for pattern in [
            "src/[abc]*.rs",
            "**/*.[ch]",
            "!(vendor)/**",
            "**/+(a|b).rs",
            "!vendor/**",
        ] {
            assert!(
                reads_as_an_unsupported_glob(pattern).is_some(),
                "{pattern} was not recognised"
            );
        }
    }

    /// The advice is free on a result that found nothing anyway, but a pattern that works must
    /// never be lectured at. Brace groups are supported, so they are the first thing that must
    /// not fire here.
    #[test]
    fn a_glob_the_matcher_can_read_is_left_alone() {
        for pattern in [
            "*.rs",
            "**/*.{cc,h,mm}",
            "{src,tests}/**/*.rs",
            "crates/**/tools.rs",
            "a?.rs",
            // A lone bracket in a filename is not a character class.
            "notes[draft.md",
        ] {
            assert!(
                reads_as_an_unsupported_glob(pattern).is_none(),
                "{pattern} was flagged"
            );
        }
    }

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
        let names: Vec<String> = available(false)
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
                "lsp",
                "spawn_processor",
                "load_skill",
                "ask_user",
                "run",
                "job_output",
                "read_output",
                "spawn_agent",
                "fetch_url"
            ]
        );
    }

    /// The depth is what bounds a whole tree of delegates, and a bound resting on the tool list
    /// alone rests on the model reading it. This pins half of it; the other half is dispatch,
    /// which answers the call as an unknown name.
    #[test]
    fn a_delegate_is_never_offered_a_way_to_delegate() {
        for name in bravebot_core::delegate::Kind::NAMES {
            let kind = bravebot_core::delegate::Kind::from_name(name).expect("enumerated");
            let offered: Vec<String> = for_delegate(&kind.capabilities())
                .iter()
                .map(|t| t.function.name.clone())
                .collect();
            assert!(
                !offered.iter().any(|t| t == "spawn_agent"),
                "a {name} was offered a way to delegate"
            );
        }
    }

    /// A tool a run's gates would refuse on every call is a tool the model has to be told to
    /// ignore, so the set follows the capabilities rather than being listed per kind.
    #[test]
    fn a_delegate_is_offered_only_the_tools_its_capabilities_reach() {
        use bravebot_core::delegate::Kind;

        let names = |kind: Kind| -> Vec<String> {
            for_delegate(&kind.capabilities())
                .iter()
                .map(|t| t.function.name.clone())
                .collect()
        };

        let reader = names(Kind::Reader);
        assert!(reader.contains(&"read_file".to_string()));
        assert!(reader.contains(&"spawn_processor".to_string()));
        assert!(!reader.contains(&"run".to_string()));
        assert!(!reader.contains(&"read_output".to_string()));
        assert!(!reader.contains(&"write_file".to_string()));
        assert!(!reader.contains(&"edit_file".to_string()));

        let checker = names(Kind::Checker);
        assert!(checker.contains(&"run".to_string()));
        assert!(checker.contains(&"read_output".to_string()));
        assert!(!checker.contains(&"write_file".to_string()));
        assert!(!checker.contains(&"edit_file".to_string()));

        let worker = names(Kind::Worker);
        assert!(worker.contains(&"write_file".to_string()));
        assert!(worker.contains(&"edit_file".to_string()));
        assert!(worker.contains(&"run".to_string()));
    }

    /// Neither tool has an audience from inside a delegate. The question would put a task the
    /// person never set to them, and the list would replace the one they are watching with the
    /// steps of a sub-task they did not ask about.
    #[test]
    fn a_delegate_is_offered_no_task_list_and_no_way_to_ask() {
        for name in bravebot_core::delegate::Kind::NAMES {
            let kind = bravebot_core::delegate::Kind::from_name(name).expect("enumerated");
            let offered: Vec<String> = for_delegate(&kind.capabilities())
                .iter()
                .map(|t| t.function.name.clone())
                .collect();
            assert!(
                !offered.iter().any(|t| t == "ask_user"),
                "a {name} was offered a question to put to somebody"
            );
            assert!(
                !offered.iter().any(|t| t == "todo_write"),
                "a {name} was offered the task list a person is watching"
            );
            assert!(
                !offered.iter().any(|t| t == "schedule_next"),
                "a {name} was offered a way to pace a loop it is not a tick of"
            );
        }
    }

    /// A kind holds the network capability so the driver can make its model call, and that is
    /// the whole of what it buys: a delegate pointing a request at a host of its own would be
    /// egress nobody approved for this sub-task, and the person asked about the host would be
    /// arbitrating a task they never set. The one tool whose capability is held and whose name is
    /// still withheld, so it is worth a test of its own.
    #[test]
    fn no_kind_is_offered_a_tool_that_reaches_the_network() {
        for name in bravebot_core::delegate::Kind::NAMES {
            let kind = bravebot_core::delegate::Kind::from_name(name).expect("enumerated");
            let capabilities = kind.capabilities();
            assert!(
                capabilities.contains(bravebot_core::capability::Capability::WebFetch),
                "a {name} could not have made its own requests"
            );
            let offered: Vec<String> = for_delegate(&capabilities)
                .iter()
                .map(|t| t.function.name.clone())
                .collect();
            assert!(
                !offered.iter().any(|t| t == "fetch_url"),
                "a {name} was offered a way to reach a host of its own"
            );
        }
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
        for tool in available(false) {
            let name = tool.function.name;
            assert!(!name.contains("shell"), "{name} takes a shell string");
            assert!(!name.contains("exec"), "{name} takes a shell string");
        }
    }

    /// `run` has exactly one field saying what to run. The line is compiled here rather than handed
    /// anywhere, so a second way to say what to run would be a second thing to keep honest.
    /// `background` says what to do with the line rather than what it is, and is the only other
    /// field.
    #[test]
    fn run_takes_one_command_line_and_nothing_else() {
        let tool = available(false)
            .into_iter()
            .find(|t| t.function.name == "run")
            .expect("run is offered");
        let properties = tool.function.parameters["properties"]
            .as_object()
            .expect("run has parameters");
        assert_eq!(
            properties.keys().collect::<Vec<_>>(),
            vec!["background", "command"],
            "run gained a field beside the command line and whether to wait for it"
        );
        assert_eq!(properties["command"]["type"], "string");
        assert_eq!(properties["background"]["type"], "boolean");
        assert_eq!(
            tool.function.parameters["required"]
                .as_array()
                .expect("run says what is required"),
            &[serde_json::json!("command")],
            "the command line is the only thing a run must be given"
        );
    }

    /// A tool's description is the only instruction the planner reliably reads, so wording that
    /// changes behaviour is behaviour. This one has to say that narrowing inside the line is the
    /// cheapest thing available, and give the shapes rather than gesture at them.
    #[test]
    fn the_run_description_tells_the_planner_to_filter_at_the_source() {
        let described = run_description();
        for shape in ["head -n", "-l", "-c", "sed -n"] {
            assert!(
                described.contains(shape),
                "the description does not give `{shape}` as a way to narrow: {described}"
            );
        }
        assert!(
            described.contains("returns less"),
            "the description does not say to prefer whichever returns less: {described}"
        );
    }

    /// The instruction that measurably steered a session into the expensive path. A capped
    /// structured search returns thousands of tokens of truncated matches where `grep -rl` returns
    /// a dozen lines, and a planner obeying that sentence pays the difference every round.
    #[test]
    fn the_run_description_does_not_send_the_planner_to_the_other_tools_instead() {
        let described = run_description().to_lowercase();
        for steer in [
            "do not use it to read something read_file",
            "prefer read_file",
            "prefer search",
        ] {
            assert!(
                !described.contains(steer),
                "the description steers away from a command line: {described}"
            );
        }
    }

    fn run_description() -> String {
        available(false)
            .into_iter()
            .find(|t| t.function.name == "run")
            .expect("run is offered")
            .function
            .description
    }

    /// Blind output is the default, not the rule, and describing it as the rule is what made
    /// compiling look pointless: a planner told it will never see what a program printed has no
    /// reason to run a build. Vouching is the way out and the description has to say so.
    #[test]
    fn run_says_a_vouched_command_comes_back_readable() {
        let tool = available(false)
            .into_iter()
            .find(|t| t.function.name == "run")
            .expect("run is offered");
        let description = &tool.function.description;

        assert!(
            description.contains("vouched"),
            "run's description does not mention vouching, so blind output reads as permanent"
        );
        assert!(
            description.contains("compile and test"),
            "run's description does not say to use it for building and testing"
        );
        // The old wording promised the output would never be shown, which is false for a vouched
        // command and is the sentence a planner reasoned from.
        assert!(
            !description.contains("You will NOT be shown the output"),
            "run's description still claims output is never shown"
        );
    }

    /// The same ban across every tool rather than only `run`, because the tool a shell arrives in
    /// will not be the one anybody is watching. Shell mode gave the *user* a real shell, and the
    /// whole justification for that is that the planner has none: a field like this appearing
    /// anywhere in the tool list would end the distinction quietly.
    #[test]
    fn only_run_takes_a_command_line() {
        // Names a shell string is plausibly called, none of which is the compiled surface.
        const FORBIDDEN: [&str; 6] = [
            "shell",
            "script",
            "cmd",
            "command_line",
            "argv_string",
            "sh",
        ];

        for tool in available(false) {
            let name = tool.function.name;
            let Some(properties) = tool.function.parameters["properties"].as_object() else {
                continue;
            };
            for field in properties.keys() {
                assert!(
                    !FORBIDDEN.contains(&field.as_str()),
                    "{name} gained a '{field}' field, which is a shell line by another name"
                );
                // One tool takes a line, and it is the one whose whole job is compiling one.
                if field == "command" {
                    assert_eq!(
                        name, "run",
                        "{name} gained a 'command' field; only run compiles a line"
                    );
                }
            }
        }
    }

    /// A planner that asks the user for a path, a filename, or whether something is installed is
    /// asking a person to do a lookup, and they answer less precisely than the filesystem does.
    /// One session opened by asking where Brave was installed rather than looking.
    #[test]
    fn asking_is_described_as_a_last_resort_after_looking() {
        let tool = available(false)
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
        let tool = available(false)
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

    /// The tool must tell the planner it will not see the output, or it spends rounds running
    /// things to read results that never come back to it.
    #[test]
    fn run_says_its_output_does_not_come_back_to_the_planner() {
        let tool = available(false)
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
            let tool = available(false)
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
        let edit = available(false)
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
        for tool in available(false) {
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
        let tool = available(false)
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
        let tool = available(false)
            .into_iter()
            .find(|t| t.function.name == "todo_write")
            .expect("todo_write is offered");
        assert!(
            tool.function.description.contains("whole list"),
            "the description does not ask for the whole list: {}",
            tool.function.description
        );
    }

    /// A build log's verdict is at the end and its first error is near the beginning, so a
    /// sample that kept only the front would answer neither question a reader of one has.
    #[test]
    fn a_capped_output_keeps_its_head_and_its_tail() {
        let mut log = String::new();
        while log.len() <= OUTPUT_CAP * 2 {
            log.push_str("a line in the middle of a long build log\n");
        }
        let log = format!("the first line\n{log}the last line\n");

        let sample = bounded(&log).expect("an output twice the cap is capped");

        assert!(sample.len() < log.len(), "nothing was dropped");
        assert!(
            sample.starts_with("the first line\n"),
            "the head went: {}",
            &sample[..40]
        );
        assert!(sample.ends_with("the last line\n"), "the tail went");
        // In the driver's own words, so a planner knows it is reading a sample rather than a
        // short result. How many bytes went, because that is what says how much is missing.
        assert!(
            sample.contains("the middle of this output was dropped"),
            "the sample does not say that it is one"
        );
    }

    /// The cap is a bound on a long result, not a transformation every result goes through: a
    /// planner told that a two-line answer was cut short would narrow a command that answered it.
    #[test]
    fn an_output_inside_the_cap_is_left_alone() {
        assert!(bounded("two\nlines\n").is_none());
        assert!(bounded(&"x".repeat(OUTPUT_CAP)).is_none());
    }

    mod activity {
        use super::*;

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

            fn confirm_fetch(&mut self, _request: &crate::confirm::FetchRequest) -> Decision {
                Decision::Reject
            }

            fn confirm_vouch(&mut self, _request: &crate::confirm::VouchRequest) -> Decision {
                Decision::Reject
            }

            /// Refuses. A test double is not a person agreeing to start a process.
            fn confirm_server(&mut self, _request: &crate::confirm::ServerRequest) -> Decision {
                Decision::Reject
            }

            fn ask_user(&mut self, asking: &Asking) -> Vec<Answer> {
                self.seen.push(asking.clone());
                self.reply.clone()
            }

            /// Nobody is typing: no interface, and no queue to type into.
            fn interjection(&mut self) -> Option<String> {
                None
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

    mod scheduling {
        use super::*;
        use bravebot_core::capability::{Capability, CapabilitySet};
        use bravebot_core::event::RecordingSink;
        use bravebot_core::policy::{ReleasePlan, Routing};

        fn call(arguments: Value) -> Produced {
            let mut sink = RecordingSink::new();
            let mut routing = Routing::new();
            routing.insert_trusted("task", "watch the build");
            let mut policy = Policy::begin(
                routing,
                ReleasePlan::new(),
                CapabilitySet::from_iter([Capability::FileRead]),
                &mut sink,
            )
            .expect("policy");
            schedule_next(&mut policy, &arguments)
        }

        /// The whole reason this tool can exist. A turn may choose the moment and nothing else,
        /// so the field a person would have to read the loop's prompt to approve is not here to
        /// be filled in.
        #[test]
        fn nothing_on_this_tool_says_what_the_next_turn_asks() {
            let tool = available(true)
                .into_iter()
                .find(|t| t.function.name == "schedule_next")
                .expect("schedule_next is offered to a self-paced tick");
            let properties = tool.function.parameters["properties"]
                .as_object()
                .expect("properties");
            let mut fields: Vec<&str> = properties.keys().map(String::as_str).collect();
            fields.sort_unstable();
            assert_eq!(fields, ["delay_seconds", "noop", "reason"]);
        }

        /// A tool that does nothing outside one situation is a tool the planner has to be told to
        /// ignore, and one it calls anyway is one it was told worked.
        #[test]
        fn the_tool_that_paces_a_loop_is_offered_only_to_a_tick_of_one() {
            assert!(
                !available(false)
                    .iter()
                    .any(|t| t.function.name == "schedule_next"),
                "an ordinary turn was offered a way to schedule the next one"
            );
        }

        /// The planner has to be told the wait it is getting, not the wait it asked for, or its
        /// next answer describes a schedule that is not happening.
        #[test]
        fn a_wait_outside_the_bounds_is_reported_as_the_one_that_will_happen() {
            for (asked, held) in [(1u64, 60u64), (86_400, 3_600)] {
                let produced = call(json!({"delay_seconds": asked, "noop": false}));
                assert_eq!(
                    produced.wakeup.expect("a wakeup").after,
                    std::time::Duration::from_secs(held)
                );
                assert!(
                    produced.note.contains(&format!("{held}s")),
                    "asked for {asked}: {}",
                    produced.note
                );
            }
        }

        #[test]
        fn what_the_turn_is_waiting_on_reaches_the_person_watching() {
            let produced = call(json!({
                "delay_seconds": 300,
                "noop": true,
                "reason": "watching the release build"
            }));
            assert!(
                produced.note.contains("watching the release build"),
                "{}",
                produced.note
            );
            assert!(produced.wakeup.expect("a wakeup").quiet);
        }

        /// Whether a tick found anything is what the count of quiet ones is built from, so a turn
        /// that leaves it out is asking for a number to be invented.
        #[test]
        fn a_schedule_missing_what_it_needs_is_refused() {
            for arguments in [
                json!({"noop": false}),
                json!({"delay_seconds": 300}),
                json!({"delay_seconds": "soon", "noop": false}),
            ] {
                let produced = call(arguments.clone());
                assert!(produced.failed, "{arguments} was accepted");
                assert!(produced.wakeup.is_none(), "{arguments} still scheduled one");
            }
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

    /// Editing compares the planner's `old_text` against the file to find the passage to
    /// replace, and that comparison decides whether the write happens at all.
    mod editing {
        use super::*;
        use bravebot_core::capability::{Capability, CapabilitySet};
        use bravebot_core::event::{Event, RecordingSink};
        use bravebot_core::label::Integrity;
        use bravebot_core::policy::{ReleasePlan, Routing};
        use bravebot_core::trust::TrustStore;

        /// A directory that removes itself, so a test leaves nothing behind.
        struct Scratch {
            path: std::path::PathBuf,
        }

        impl Scratch {
            fn new(name: &str) -> Self {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|since| since.as_nanos())
                    .unwrap_or(0);
                // The name carries the pid and a nanosecond stamp, and `create_dir` below
                // refuses a name already taken rather than reusing it, which is the secure
                // creation this rule asks for. A fixed name would also collide with another
                // run of the suite on the same machine, which is not hypothetical here.
                // nosemgrep: rust.lang.security.temp-dir.temp-dir
                let path = std::env::temp_dir().join(format!(
                    "bravebot-editing-{name}-{}-{stamp}",
                    std::process::id()
                ));
                std::fs::create_dir(&path).expect("create scratch");
                Self { path }
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }

        fn routing() -> Routing {
            let mut r = Routing::new();
            r.insert_trusted("task", "edit a file");
            r
        }

        /// A policy that vouches for every relative path, so the file's own contents are
        /// trusted and locating a passage in them is permitted. The question each test asks is
        /// about the *arguments*, so the file must not be what refuses.
        ///
        /// The rule is `"."` rather than the scratch directory because a workspace read asks
        /// about the path relative to its root, and a trust rule only ever covers a path of
        /// the same kind: an absolute rule would be dead here and the tests would be passing
        /// for a reason nobody wrote down.
        fn policy_vouching(sink: &mut RecordingSink) -> Policy<'_, RecordingSink> {
            let mut store = TrustStore::new();
            store.trust(".");
            Policy::begin(
                routing(),
                ReleasePlan::new(),
                CapabilitySet::from_iter([Capability::FileRead, Capability::FileWrite]),
                sink,
            )
            .expect("policy")
            .with_trust(store)
        }

        fn edit(policy: &mut Policy<'_, RecordingSink>, workspace: &Workspace) -> String {
            let produced = edit_file(
                policy,
                workspace,
                &SlotStore::new(),
                &mut crate::confirm::ApproveWrites,
                &json!({"path": "a.txt", "old_text": "old", "new_text": "new"}),
            );
            let proof = policy.authorise_display_release("test inspects the tool result");
            produced.text.declassify(&proof)
        }

        /// The baseline: with the file vouched for and a context that has met nothing
        /// untrusted, the edit lands. Without this the refusal below would prove nothing.
        #[test]
        fn an_edit_from_a_trusted_context_replaces_the_passage() {
            let scratch = Scratch::new("trusted");
            std::fs::write(scratch.path.join("a.txt"), "keep\nold\ntail\n").unwrap();
            let workspace = Workspace::new(&scratch.path).expect("workspace");

            let mut sink = RecordingSink::new();
            let mut policy = policy_vouching(&mut sink);
            let told = edit(&mut policy, &workspace);

            assert!(told.starts_with("edited a.txt"), "{told}");
            assert_eq!(
                std::fs::read_to_string(scratch.path.join("a.txt")).unwrap(),
                "keep\nnew\ntail\n"
            );
        }

        /// The property the gate exists for. `old_text` is the planner's words, and a planner
        /// whose context has met untrusted content is writing words an attacker may have
        /// steered. Comparing them against the file decides whether a write happens, so the
        /// read is refused and the file is left alone.
        #[test]
        fn an_edit_is_refused_once_the_context_has_met_something_untrusted() {
            let scratch = Scratch::new("fallen");
            std::fs::write(scratch.path.join("a.txt"), "keep\nold\ntail\n").unwrap();
            let workspace = Workspace::new(&scratch.path).expect("workspace");

            let mut sink = RecordingSink::new();
            let mut policy = policy_vouching(&mut sink).resuming(Integrity::Untrusted);
            let told = edit(&mut policy, &workspace);

            assert!(told.starts_with("refused:"), "{told}");
            assert!(
                told.contains("must not decide anything"),
                "the refusal does not say why: {told}"
            );
            assert_eq!(
                std::fs::read_to_string(scratch.path.join("a.txt")).unwrap(),
                "keep\nold\ntail\n",
                "the file was edited from a context that had met untrusted content"
            );
            // And the refusal did nothing on the way to refusing. An edit that cannot happen
            // must not have opened the file first: that spends the read capability and puts an
            // observation in the trail for a turn in which nothing was read.
            assert!(
                !sink
                    .events()
                    .iter()
                    .any(|e| matches!(e, Event::Observed { .. })),
                "the file was read before the refusal: {:?}",
                sink.events()
            );
        }
    }
}
