//! Standing instructions, and the skills on offer, as text for the system prompt.
//!
//! Two things reach the planner before it is asked anything: `AGENTS.md`, which says how work is
//! done here, and the name and description of every skill it may load. Both are instructions, so
//! both are exactly the kind of input this repository is careful about.
//!
//! # One way in, and it refuses
//!
//! Every source passes `Policy::read_trusted_content`, which hands the bytes over when they are
//! trusted and refuses otherwise. A refusal means the file is left out and the user is told; it
//! is never quarantined into a reference, because a reference to standing instructions is no use
//! to anybody.
//!
//! Going through that gate rather than `Policy::present` is sound only because it refuses
//! everything `present` would have quarantined. For trusted content the two agree: `present`
//! returns it visible, and its absorb is a no-op at trusted integrity, so nothing about the
//! context is left unrecorded by taking this path.
//!
//! # Why the system prompt, and not a message
//!
//! The system prompt belongs to the build rather than to the conversation, so it is not stored
//! and is put in front of each request afresh. A persistent session therefore holds one copy of
//! AGENTS.md however many turns it runs, where a `Message::user` would accumulate one per turn.

use crate::skills::{Catalogue, Notice};
use crate::workspace::Workspace;
use bravebot_core::event::Sink;
use bravebot_core::policy::Policy;
use bravebot_core::value::Labelled;
use std::path::Path;

/// The file a project or a user states their conventions in.
const AGENTS_FILE: &str = "AGENTS.md";

/// What gets appended to the system prompt, and what to tell the user about it.
#[derive(Debug, Clone, Default)]
pub struct Preamble {
    /// The text itself, empty when there is nothing to say.
    pub text: String,
    /// Lines for the person watching: what loaded, and what did not and why.
    pub notices: Vec<Notice>,
}

/// Build the preamble for one turn.
///
/// `skills` has already been discovered, gated, and had the untrusted entries dropped, so this
/// only has to render it. `AGENTS.md` is read here, from the user's own directory first and the
/// workspace second, so the more specific one is the one the planner reads last.
pub fn compose<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    home: Option<&Path>,
    skills: &Catalogue,
    tick: Option<crate::turn::Tick>,
) -> Preamble {
    let mut preamble = Preamble::default();

    let mut standing = String::new();
    if let Some(home) = home
        && let Some(text) = read_home_agents(policy, home)
    {
        standing.push_str(&format!(
            "From ~/.bravebot/{AGENTS_FILE}:\n\n{}\n\n",
            text.trim()
        ));
    }
    match read_workspace_agents(policy, workspace) {
        Ok(Some(text)) => {
            standing.push_str(&format!("From {AGENTS_FILE}:\n\n{}\n\n", text.trim()));
        }
        Ok(None) => {}
        Err(notice) => preamble.notices.push(notice),
    }

    if !standing.is_empty() {
        preamble.text.push_str(
            "\n\nStanding instructions from the user. These apply to every task here, and the \
             later ones are the more specific.\n\n",
        );
        preamble.text.push_str(&standing);
    }

    if !skills.is_empty() {
        preamble.text.push_str(
            "\n\nSkills. Each is a set of instructions for a kind of task, most of them written \
             by the user. When a task matches one, call load_skill with its name before starting \
             that work and follow what it says. These names are the only ones that exist.\n\n",
        );
        preamble.text.push_str(&skills.describe_for_prompt());
    }

    // Last, and only where there is one. A turn that is a tick has to be told so: the driver is
    // the only thing that knows, and a planner that cannot tell answers as though somebody had
    // just typed the line for the first time. Which kind of loop it is matters as much, because
    // the tool for saying when to run again is offered to one of the two and a turn that does
    // not know that will look for a tool it was never given.
    if let Some(tick) = tick {
        preamble.text.push_str(&format!(
            "\n\nThis turn is tick {} of a loop the user started. Every tick sends the same line \
             they typed, so you are being asked this again about a world that may have moved; \
             what earlier ticks did is above, so read it rather than repeating it. Load the loop \
             skill before working.\n\n",
            tick.number
        ));
        preamble.text.push_str(if tick.self_paced {
            "Nobody gave an interval, so this loop runs for exactly as long as you keep pacing \
             it: call schedule_next once, at the end of this turn, or the loop ends.\n"
        } else {
            "The user gave the interval, so the timing is theirs. There is nothing here for you \
             to schedule and no tool for it: do this tick's work and answer.\n"
        });
    }

    preamble
}

/// `~/.bravebot/AGENTS.md`, trusted for sitting where it sits.
fn read_home_agents<S: Sink>(policy: &mut Policy<'_, S>, home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join(AGENTS_FILE)).ok()?;
    let origin = format!("~/.bravebot/{AGENTS_FILE}");
    let labelled = policy.label_user_configuration(&origin, text);
    policy.read_trusted_content("preamble", &labelled).ok()
}

/// `<workspace>/AGENTS.md`, trusted only if the trust map says so.
///
/// Three answers, and they are genuinely different: there is no file, there is one and it is the
/// user's own, or there is one from a path nobody vouched for. Only the last is worth a word,
/// and the word has to be about the directory rather than about the file's contents.
fn read_workspace_agents<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
) -> Result<Option<String>, Notice> {
    if !workspace.root().join(AGENTS_FILE).is_file() {
        return Ok(None);
    }

    let Ok(contents) = workspace.read(policy, &Labelled::trusted(AGENTS_FILE.to_string())) else {
        return Ok(None);
    };

    // Asked of the label before it is asked of the gate. The gate is still the only thing that
    // hands bytes over, and it still runs whenever this proceeds; what this avoids is recording a
    // denial for a condition that is ordinary and expected. Without it every turn in an untrusted
    // directory holding an AGENTS.md would report that a gate refused something, which is how a
    // warning stops being read.
    if !contents.label().is_trusted() {
        return Err(Notice::from_message(format!(
            "{AGENTS_FILE} was not loaded: this directory is not trusted"
        )));
    }

    match policy.read_trusted_content("preamble", &contents) {
        Ok(text) => Ok(Some(text)),
        Err(_) => Err(Notice::from_message(format!(
            "{AGENTS_FILE} was not loaded: it is not trusted"
        ))),
    }
}
