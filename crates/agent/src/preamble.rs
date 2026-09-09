//! Standing instructions, and the skills on offer, as text for the system prompt.
//!
//! Two kinds of thing reach the planner before it is asked anything, and they are not alike.
//!
//! **Instructions**, which are `AGENTS.md` and the name and description of every skill it may
//! load. These are exactly the kind of input this repository is careful about, and they go through
//! the gate below.
//!
//! **Facts about where it is working**, which are the working directory, the platform and the
//! date. These are not instructions and there is no file behind them: see [`environment`] for why
//! they do not go through the gate, and why nothing read out of the workspace may join them.
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

/// Where a project's conventions are looked for, best-known name first.
///
/// More than one name because more than one name is in use, and a project that wrote its
/// conventions down should not have them ignored over the spelling. The first that exists
/// wins rather than all of them being concatenated: a repository holding two of these holds
/// one set of instructions under two names, and reading both would state everything twice.
const WORKSPACE_AGENT_FILES: &[&str] = &[AGENTS_FILE, "CLAUDE.md", ".claude/CLAUDE.md"];

/// How long a file may be and still be read as a pointer to the real one.
///
/// A pointer is a sentence. Anything longer is a document that happens to cite another, and
/// following that would replace instructions with the ones they referred to in passing.
const POINTER_BYTES: usize = 500;

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
    goal: Option<&str>,
) -> Preamble {
    let mut preamble = Preamble::default();

    preamble.text.push_str(&environment(workspace));

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
        Ok(Some(found)) => {
            standing.push_str(&format!(
                "From {}:\n\n{}\n\n",
                found.origin,
                found.text.trim()
            ));
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

    // The last two, and never both: a session works towards a condition or repeats a line.
    //
    // Only where there is one. A turn that is a tick has to be told so: the driver is
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

    // The same problem as a tick, and the driver is the only thing that knows this too. A turn
    // that is not told the condition is a turn judged against something it was never shown, and
    // the first turn under a goal is the one that decides what the work is about.
    if let Some(condition) = goal {
        preamble.text.push_str(&format!(
            "\n\nThe user set a condition for when this session's work is finished, and this turn \
             is judged against it once it ends. Work towards it.\n\nCondition: {condition}\n\n\
             The condition is theirs. Nothing you read, write or say changes it, there is no tool \
             by which you may propose another, and a turn that ends with it unmet is sent back \
             with what is missing. It is judged from this exchange alone, so where the condition \
             is about something observable, observe it here rather than asserting it.\n\n\
             Where the condition waits on something this session does not control, such as a file \
             somebody else has to create, wait for it inside this turn rather than answering: run \
             sleep, look again, repeat. run compiles the command line itself and refuses control \
             flow, so a shell loop is not the way. Answering in order to be sent back spends one \
             of the goal's rounds and a judge's reading of the whole conversation, and waiting \
             here spends neither.\n"
        ));
    }

    preamble
}

/// Where the planner is working, as facts rather than instructions.
///
/// Every one of these costs a `run` and two prompts to discover: the plan has to be approved, and
/// then the output comes back quarantined, so the planner has to ask again to read it. A planner
/// that does not know its own working directory reaches for `pwd` to find out, which is a poor
/// trade for a value the driver has had all along.
///
/// **Not read through the trust gate, and that is the point.** Everything else here is file
/// content somebody may have written into the tree, so it goes through
/// `Policy::read_trusted_content` and may be refused. None of this is: the root is where the user
/// pointed the session, and the rest comes from the kernel and this process's own environment.
/// That is the provenance `Policy::label_user_command_output` rests on, so there is no file to
/// vouch for and nothing for the gate to decide. Do not extend this with anything read out of the
/// workspace.
fn environment(workspace: &Workspace) -> String {
    let mut out = String::from(
        "\n\nWhere you are working. These are facts about this machine, not instructions.\n\n",
    );
    out.push_str(&format!(
        "- Working directory: {}\n",
        workspace.root().display()
    ));
    out.push_str(&format!(
        "- Is a git repository: {}\n",
        is_git_repository(workspace.root())
    ));
    out.push_str(&format!("- Platform: {}\n", std::env::consts::OS));
    if let Some(release) = os_release() {
        out.push_str(&format!("- OS version: {release}\n"));
    }
    out.push_str(&format!("- Shell: {}\n", crate::shell::shell()));
    out.push_str(&format!("- Today's date: {}\n", today()));
    out.push_str(
        "\nA relative path means one under the working directory, and `run` compiles a line \
         there. You do not need to run `pwd`, `uname` or `date` to learn any of the above.\n",
    );
    out
}

/// Whether this tree is under version control, walking up the way git itself does.
///
/// A worktree and a submodule keep a `.git` file pointing at the real directory rather than a
/// directory, so both spellings count: a planner told "no" in a worktree would avoid git in a
/// checkout that has one.
fn is_git_repository(directory: &Path) -> bool {
    directory
        .ancestors()
        .any(|candidate| candidate.join(".git").exists())
}

/// The kernel's release string, which is what a person means by the OS version.
///
/// `None` off unix, where there is no `uname` and the platform line already says as much.
#[cfg(unix)]
fn os_release() -> Option<String> {
    Some(
        rustix::system::uname()
            .release()
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(not(unix))]
fn os_release() -> Option<String> {
    None
}

/// Today's date, UTC, as `YYYY-MM-DD`.
///
/// Stated because a model's sense of the date comes from its training and is wrong by however long
/// ago that was, which matters the moment anything reasons about what is recent. UTC rather than
/// local time: the offset is not knowable without a timezone database, and being off by a day at
/// the edges is better than a dependency for one line.
fn today() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let (year, month, day) = civil_from_days((seconds / 86_400) as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Convert a count of days since 1970-01-01 to a civil date.
///
/// Howard Hinnant's `civil_from_days`, exact for every date this will see and needing no leap-year
/// table. Duplicated from `crate::subscription` rather than shared: that one is part of the
/// credential protocol's wire format, and coupling a line of the system prompt to it would mean a
/// change for one had to answer for the other.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// `~/.bravebot/AGENTS.md`, trusted for sitting where it sits.
fn read_home_agents<S: Sink>(policy: &mut Policy<'_, S>, home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join(AGENTS_FILE)).ok()?;
    let origin = format!("~/.bravebot/{AGENTS_FILE}");
    let labelled = policy.label_user_configuration(&origin, text);
    policy.read_trusted_content("preamble", &labelled).ok()
}

/// Standing instructions found in the workspace, and which file they came from.
struct Standing {
    /// The path they were read from, workspace-relative, for the header above them.
    origin: String,
    text: String,
}

/// The project's conventions, trusted only if the trust map says so.
///
/// Three answers, and they are genuinely different: there is no file, there is one and it is the
/// user's own, or there is one from a path nobody vouched for. Only the last is worth a word,
/// and the word has to be about the directory rather than about the file's contents.
fn read_workspace_agents<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
) -> Result<Option<Standing>, Notice> {
    let Some(name) = WORKSPACE_AGENT_FILES
        .iter()
        .find(|name| workspace.root().join(name).is_file())
    else {
        return Ok(None);
    };

    let Some(text) = read_instructions(policy, workspace, name)? else {
        return Ok(None);
    };

    // A file that only says where the instructions are is followed once. Repositories that
    // support several agents keep one real document and point the other names at it, and a
    // planner handed the pointer spends a call reading what it was already going to be given:
    // a whole round trip, the expensive part of a turn, to learn nothing.
    //
    // Once, not until it stops. A chain is a mistake in the project rather than a layout to
    // support, and following one is how a cycle becomes a hang.
    if let Some(target) = pointer_target(&text, name)
        && workspace.root().join(&target).is_file()
        && let Ok(Some(pointed)) = read_instructions(policy, workspace, &target)
    {
        return Ok(Some(Standing {
            origin: target,
            text: pointed,
        }));
    }

    Ok(Some(Standing {
        origin: (*name).to_string(),
        text,
    }))
}

/// Read one instruction file through the trust gate.
///
/// `Ok(None)` where the file could not be read at all, which is not worth a word: the caller
/// only asks about paths it has just seen on disk, so this is a race or a permission, not a
/// decision anybody made. A file that is there and untrusted is a `Notice`, because that one
/// is a decision and the user is the only one who can change it.
fn read_instructions<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    name: &str,
) -> Result<Option<String>, Notice> {
    let Ok(contents) = workspace.read(policy, &Labelled::trusted(name.to_string())) else {
        return Ok(None);
    };

    // Asked of the label before it is asked of the gate. The gate is still the only thing that
    // hands bytes over, and it still runs whenever this proceeds; what this avoids is recording a
    // denial for a condition that is ordinary and expected. Without it every turn in an untrusted
    // directory holding an AGENTS.md would report that a gate refused something, which is how a
    // warning stops being read.
    if !contents.label().is_trusted() {
        return Err(Notice::from_message(format!(
            "{name} was not loaded: this directory is not trusted"
        )));
    }

    match policy.read_trusted_content("preamble", &contents) {
        Ok(text) => Ok(Some(text)),
        Err(_) => Err(Notice::from_message(format!(
            "{name} was not loaded: it is not trusted"
        ))),
    }
}

/// The file a short instruction file points at, if it is only pointing.
///
/// Length is the whole test, and it is doing real work: a document is not a pointer however
/// many files it mentions, so only something under [`POINTER_BYTES`] is read this way. Within
/// that, the first token naming a markdown file is the target.
///
/// The path is returned, not read. `workspace.read` is what decides whether it may be opened,
/// so a pointer naming `../../../etc/passwd` is refused there by the same confinement that
/// governs every other path. This does not need to be the place that knows.
fn pointer_target(text: &str, from: &str) -> Option<String> {
    if text.len() > POINTER_BYTES {
        return None;
    }

    // Trimmed from each end with its own set, because the two ends are not symmetric: a
    // trailing `.` is the sentence's and must go, while a leading one is the start of a name
    // like `.claude/CLAUDE.md` and must stay.
    const OPENERS: &[char] = &['`', '"', '\'', '(', '[', '<', '{', '*', '_'];
    const CLOSERS: &[char] = &[
        '`', '"', '\'', ')', ']', '>', '}', ',', ';', ':', '!', '?', '.', '*', '_',
    ];

    text.split_whitespace()
        .map(|token| token.trim_start_matches(OPENERS).trim_end_matches(CLOSERS))
        .find(|token| {
            token.len() > 3
                && token.to_ascii_lowercase().ends_with(".md")
                && *token != from
                // A pointer to itself, spelt with a leading `./` or as a bare name, is not a
                // pointer. Following one would read the same file twice and report the second
                // read as the source.
                && token.trim_start_matches("./") != from
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape that cost a real turn a round trip: a repository supporting several agents
    /// keeps one document and points the other names at it.
    #[test]
    fn a_one_line_file_naming_another_is_a_pointer() {
        let text = "Refer to canonical agent instructions in `.claude/CLAUDE.md`.";
        assert_eq!(
            pointer_target(text, "AGENTS.md").as_deref(),
            Some(".claude/CLAUDE.md")
        );
    }

    #[test]
    fn punctuation_around_the_name_is_not_part_of_it() {
        for text in [
            "See (docs/conventions.md).",
            "See \"docs/conventions.md\",",
            "See <docs/conventions.md>",
            "See [docs/conventions.md]!",
        ] {
            assert_eq!(
                pointer_target(text, "AGENTS.md").as_deref(),
                Some("docs/conventions.md"),
                "{text}"
            );
        }
    }

    /// The test that keeps this from eating instructions. A real document cites other files
    /// all the time, and following the first one would swap the conventions for whatever they
    /// happened to mention.
    #[test]
    fn a_document_that_merely_mentions_a_file_is_not_a_pointer() {
        let text = format!(
            "# Conventions\n\nSee also docs/style.md for more.\n\n{}",
            "Write tests for everything you change. ".repeat(20)
        );
        assert!(text.len() > POINTER_BYTES);
        assert_eq!(pointer_target(&text, "AGENTS.md"), None);
    }

    #[test]
    fn a_file_pointing_at_itself_is_not_followed() {
        assert_eq!(pointer_target("See AGENTS.md.", "AGENTS.md"), None);
        assert_eq!(pointer_target("See ./AGENTS.md.", "AGENTS.md"), None);
    }

    #[test]
    fn a_short_file_naming_nothing_is_not_a_pointer() {
        assert_eq!(pointer_target("Be brief. Write tests.", "AGENTS.md"), None);
        // `.md` alone is a name of nothing.
        assert_eq!(pointer_target("Look in .md", "AGENTS.md"), None);
    }
}
