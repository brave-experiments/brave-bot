//! Skills: instructions the user keeps for a kind of task.
//!
//! A skill is one `SKILL.md` in a directory of its own, opening with frontmatter that names it
//! and says when it applies:
//!
//! ```text
//! ---
//! name: commit-style
//! description: How commit messages are written here. Use before writing one.
//! ---
//!
//! the body
//! ```
//!
//! Only the name and the description reach the planner up front. The body is read when the
//! planner asks for it by name, which keeps a directory of long skills from filling a context
//! that has room for the task instead.
//!
//! # What this module may and may not read
//!
//! Everything here parses **trusted** text. A caller reaches this only after
//! `Policy::read_trusted_content` has handed the bytes over, which it does for a file in the
//! user's own directory and refuses for anything else. That refusal is the whole design: a
//! skill's name and description go into the system prompt verbatim, so a skill nobody vouched
//! for is dropped entirely rather than quarantined. A reference in place of a name would be no
//! use to anyone, and a name from a file an attacker wrote would be untrusted content in the
//! planner's context.

use crate::workspace::Workspace;
use bravebot_core::capability::Capability;
use bravebot_core::event::Sink;
use bravebot_core::policy::Policy;
use bravebot_core::value::Labelled;
use std::path::Path;

/// What a `SKILL.md` declares about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    /// What the planner calls the skill when it asks for the body.
    pub name: String,
    /// When to use it. This is what the planner decides from, so it says when rather than what.
    pub description: String,
}

/// The line that opens and closes a frontmatter block.
const MARKER: &str = "---";

/// Read the frontmatter at the top of a `SKILL.md`, if it has one.
///
/// Hand-written rather than a YAML dependency, per the conventions: this recognises `key: value`
/// on a line, and a value continued on the lines indented beneath it, and nothing else. There is
/// no parser to surprise us and nothing to backtrack. Keys other than `name` and `description`
/// are ignored rather than refused, which leaves room for a file written for another agent to
/// work here too.
///
/// `None` means "not a skill", and every caller drops the file on that answer. A half-declared
/// skill is included in that: a name with no description is one the planner cannot choose
/// between, and advertising it would be worse than leaving it out.
pub fn parse_frontmatter(text: &str) -> Option<Frontmatter> {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some(MARKER) {
        return None;
    }

    let mut block = Vec::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == MARKER {
            closed = true;
            break;
        }
        block.push(line);
    }

    // An unterminated block is not frontmatter that happens to run long: it is a file whose whole
    // contents would otherwise be read as declarations.
    if !closed {
        return None;
    }

    let mut name = None;
    let mut description = None;

    let mut at = 0;
    while at < block.len() {
        let line = block[at];
        at += 1;

        // Taken whether or not this line declares anything we want. A continuation belongs to
        // the key above it however that key is spelled, and leaving one unconsumed is how a
        // wrapped sentence holding a colon becomes a key of its own.
        let opened_at = indent_of(line);
        let mut wrapped = Vec::new();
        while let Some(next) = block.get(at) {
            if !next.trim().is_empty() && indent_of(next) <= opened_at {
                break;
            }
            wrapped.push(*next);
            at += 1;
        }

        let Some((key, first)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "name" => name = Some(value_of(first, &wrapped)),
            "description" => description = Some(value_of(first, &wrapped)),
            _ => {}
        }
    }

    let name = name.filter(|n| !n.is_empty())?;
    let description = description.filter(|d| !d.is_empty())?;
    Some(Frontmatter { name, description })
}

/// How many columns a line is indented by, which is what says whether it continues the one above.
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// One value, from what followed the colon and the lines indented under it.
///
/// A description is a sentence long enough that people wrap it, and every way of wrapping one
/// ends up here: a folded or literal block introduced by `>` or `|`, a quoted scalar carried over
/// several lines, or plain text simply continued. Folded because that is what wrapping means:
/// the line breaks were the file's, not the sentence's, so they become spaces. A literal block
/// asked for its newlines and keeps them.
fn value_of(first: &str, wrapped: &[&str]) -> String {
    let first = first.trim();
    let (joiner, first) = match first {
        "|" | "|-" | "|+" => ("\n", ""),
        ">" | ">-" | ">+" => (" ", ""),
        _ => (" ", first),
    };

    let mut parts = Vec::new();
    if !first.is_empty() {
        parts.push(first);
    }
    parts.extend(wrapped.iter().map(|l| l.trim()).filter(|l| !l.is_empty()));

    unquoted(&parts.join(joiner))
}

/// A quoted scalar without its quotes, and anything else unchanged.
///
/// The quotes are YAML's, put there so a value may open with a character that would otherwise
/// mean something. They are not part of what the planner is choosing from, and leaving them in
/// puts a stray apostrophe at each end of every description on the screen.
fn unquoted(value: &str) -> String {
    let opens = value.chars().next();
    let closes = value.chars().last();
    match (opens, closes) {
        (Some('\''), Some('\'')) if value.len() >= 2 => {
            value[1..value.len() - 1].replace("''", "'")
        }
        (Some('"'), Some('"')) if value.len() >= 2 => {
            value[1..value.len() - 1].replace("\\\"", "\"")
        }
        _ => value.to_string(),
    }
}

/// Everything after the frontmatter block, which is what the planner is given when it asks.
///
/// The frontmatter itself is left out because the planner has already been told the name and the
/// description; sending them again spends context on what it used to make the call.
pub fn body_after_frontmatter(text: &str) -> &str {
    let mut offset = 0;
    let mut lines = text.split_inclusive('\n');

    let Some(first) = lines.next() else {
        return "";
    };
    if first.trim_end() != MARKER {
        return text;
    }
    offset += first.len();

    for line in lines {
        offset += line.len();
        if line.trim_end() == MARKER {
            return text[offset..].trim_start_matches('\n');
        }
    }
    // No closing marker, so there was no frontmatter to strip.
    text
}

/// A skill the planner may ask for.
///
/// Deliberately not comparable: it holds a `Labelled`, which has no `PartialEq` precisely so
/// that content cannot be decided from by comparing it.
#[derive(Debug, Clone)]
pub struct Skill {
    /// What the planner names to load it.
    pub name: String,
    /// When to use it, which is what the planner decides from.
    pub description: String,
    /// Where it came from, for the audit trail and for what the user is told.
    pub origin: String,
    /// The instructions themselves, still carrying the label they were read with.
    ///
    /// Kept labelled rather than as bare text so the planner is shown them through
    /// `Policy::present` like any other content. Nothing here re-labels: the value is the one
    /// the source produced, reshaped in the kernel to drop the frontmatter.
    body: Labelled<String>,
}

impl Skill {
    /// The instructions, which reach the planner only when it asks for them by name.
    pub fn body(&self) -> &Labelled<String> {
        &self.body
    }
}

/// Something the user should be told about discovery, in words a person reads.
///
/// A skill that was skipped is worth a line: silence would read as "you have no skills" to
/// someone who just wrote one, and the reason is usually a typo in the frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub message: String,
}

impl Notice {
    /// Build a notice from words the driver wrote.
    ///
    /// Public so the preamble can report a refusal of its own. The message is always the
    /// driver's own text, never content, which is what makes it safe to put on a screen.
    pub fn from_message(message: impl Into<String>) -> Self {
        Self::new(message)
    }

    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// The skills available this turn, in the order they are offered to the planner.
#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    entries: Vec<Skill>,
}

impl Catalogue {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Skill> {
        self.entries.iter()
    }

    /// The skill by that name, or `None`.
    ///
    /// The planner selects from this set rather than naming a path, so whatever it asks for
    /// either matches something the driver enumerated or matches nothing at all. That is what
    /// keeps a proposed name from reaching the filesystem.
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.entries.iter().find(|s| s.name == name)
    }

    /// Add a skill, replacing one of the same name.
    ///
    /// Later wins, and discovery visits the home directory before the workspace, so a project's
    /// own skill shadows a global one. That is the same "most specific wins" the trust map uses.
    fn insert(&mut self, skill: Skill) {
        match self.entries.iter_mut().find(|s| s.name == skill.name) {
            Some(existing) => *existing = skill,
            None => self.entries.push(skill),
        }
    }

    /// The lines that go in the system prompt: one per skill, name and when to use it.
    ///
    /// Never the body. A directory of long skills would otherwise fill a context that has room
    /// for the task instead, which is the whole reason the body waits to be asked for.
    pub fn describe_for_prompt(&self) -> String {
        let mut out = String::new();
        for skill in &self.entries {
            out.push_str(&format!("- {}: {}\n", skill.name, skill.description));
        }
        out
    }
}

/// A skill written into this program rather than found on a disk.
///
/// The name is a string literal here, which is what makes it different in kind from every other
/// skill. A name read out of a directory is content: it comes from whoever wrote the directory,
/// it may not be trusted, and it is why an untrusted skill is counted rather than named. A name
/// written here comes from this repository, cannot be added to or changed by anything a turn
/// does, and is the only kind that may also be a word the interface itself claims.
struct BuiltIn {
    name: &'static str,
    description: &'static str,
    body: &'static str,
}

/// The skills every session has, whatever is on the disk.
const BUILT_IN: [BuiltIn; 1] = [BuiltIn {
    name: "loop",
    description: "How a repeating turn works: what one tick is, what to do in it, and how to \
                  say when the next is due. Load it when this turn is a tick of a loop, and when \
                  the user asks for something to be watched or repeated.",
    body: LOOP,
}];

/// What the planner is told about being inside a loop.
const LOOP: &str = "\
A loop repeats one prompt: the line the user typed when they started it. Every tick sends that \
same line again, unchanged, and nothing you do alters it. You are being asked the same question \
about a world that may have moved since the last time.

Each tick is a turn of its own in the same conversation, so what earlier ticks did is still \
there to read. Read it. A tick that repeats work the last one did has spent a request to learn \
nothing, and a tick that answers as though the question were new is the commonest way to waste \
one.

Do this tick's work, and only this tick's. A loop is not a way to ask for the whole job at once: \
if the line says to watch something, look at it as it is now and say what changed.

Keep the answer short. A tick that found nothing says so in a line rather than restating the \
situation, and somebody reading twenty of these should be able to see at a glance which one \
mattered.

There are two kinds of loop, and the turn you are in says which this is.

Where the user gave an interval, the timing is theirs. There is nothing for you to decide about \
it and no tool for it, so do not go looking for one and do not tell the user a tool is missing: \
the interval is already keeping time. Work the tick and answer.

Where they gave none, you are offered schedule_next, and the loop runs for exactly as long as \
you keep calling it. Call it once, at the end of the turn, after the work is done:

- delay_seconds from what you are actually waiting on rather than from a round number. Something \
  that takes ten minutes to change is not worth looking at in sixty seconds, and something that \
  changes hourly is not worth looking at in five minutes. The wait is held to between a minute \
  and an hour.
- noop true where this tick found nothing to do and changed nothing, false where something \
  happened worth keeping: an edit, a message, a finding. Runs of quiet ticks are counted and \
  shown to the user as a single line, so an honest noop is what keeps a long watch readable.
- reason in a few words, saying what you are waiting on. The user reads it.

Not calling it ends the loop, and that is the right answer once there is nothing left to watch. \
Say so in your answer rather than scheduling a tick to say it again.

Either kind stops when the user stops it. You never need to ask them to.
";

/// The directory holding skills, inside the user's own directory and inside a project.
const SKILLS: &str = "skills";
const WORKSPACE_SKILLS: &str = ".bravebot/skills";

/// The one file that makes a directory a skill.
const SKILL_FILE: &str = "SKILL.md";

/// Find the skills available to this turn.
///
/// Two sources, visited least specific first so the more specific shadows it: the user's own
/// directory, whose contents are trusted for being the user's own, and the project, whose
/// contents are trusted only if the trust map says so.
///
/// A skill from a path nobody vouched for is **dropped, not quarantined**. Its name and its
/// description would go into the system prompt verbatim, so offering a reference in their place
/// would be no use to the planner, and offering the strings themselves would be untrusted
/// content in the planner's context. There is no third option, and dropping it is the one that
/// holds the rule.
pub fn discover<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    home: Option<&Path>,
) -> (Catalogue, Vec<Notice>) {
    let mut catalogue = Catalogue::default();
    let mut notices = Vec::new();

    // First, so that a skill of the user's own with the same name shadows one of these, which is
    // the same "most specific wins" every other source follows. There is no gate to pass and
    // nothing to vouch for: this is the program's own text, not a source somebody supplied, so it
    // can neither be refused nor be missing, and there is never a notice about it.
    for built_in in BUILT_IN {
        catalogue.insert(Skill {
            name: built_in.name.to_string(),
            description: built_in.description.to_string(),
            // A string literal in this file, derived from nothing that arrived from anywhere. It
            // is trusted for being this program's own words, which is what the label says.
            body: Labelled::trusted(built_in.body.to_string()),
            origin: "built-in".to_string(),
        });
    }

    if let Some(home) = home {
        discover_home(policy, &home.join(SKILLS), &mut catalogue, &mut notices);
    }
    discover_workspace(policy, workspace, &mut catalogue, &mut notices);

    (catalogue, notices)
}

/// Skills from `~/.bravebot/skills`, labelled from where they sit.
fn discover_home<S: Sink>(
    policy: &mut Policy<'_, S>,
    root: &Path,
    catalogue: &mut Catalogue,
    notices: &mut Vec<Notice>,
) {
    if policy.before_capability(Capability::FileRead).is_err() {
        return;
    }

    for name in skill_directories(root) {
        let file = root.join(&name).join(SKILL_FILE);
        let origin = format!("~/.bravebot/{SKILLS}/{name}/{SKILL_FILE}");

        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };

        // Labelled here and gated below rather than used directly. The gate is the same one a
        // workspace skill passes through, so there is one way into the system prompt and it
        // refuses, and the trail records both halves.
        let labelled = policy.label_user_configuration(&origin, text);
        let Ok(text) = policy.read_trusted_content("skills", &labelled) else {
            continue;
        };

        match parse_frontmatter(&text) {
            Some(front) => {
                let body = policy.render_in_place("skills", &labelled, |whole| {
                    body_after_frontmatter(&whole).to_string()
                });
                catalogue.insert(Skill {
                    name: front.name,
                    description: front.description,
                    body,
                    origin,
                });
            }
            None => notices.push(Notice::new(format!(
                "{origin} was skipped: it needs a name and a description in its frontmatter"
            ))),
        }
    }
}

/// Skills from `<workspace>/.bravebot/skills`, labelled by the trust map.
fn discover_workspace<S: Sink>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    catalogue: &mut Catalogue,
    notices: &mut Vec<Notice>,
) {
    let root = workspace.root().join(WORKSPACE_SKILLS);
    let names = skill_directories(&root);
    if names.is_empty() {
        return;
    }

    // Checked before anything is enumerated, and checked on the label rather than on any
    // content. A directory name is content too: a skill directory in a project nobody vouched
    // for could be named to read like an instruction, and it would reach the user's screen in a
    // notice even if it never reached the prompt.
    if !policy.trust().is_trusted(WORKSPACE_SKILLS) {
        let (count, verb) = counted(names.len());
        notices.push(Notice::new(format!(
            "{count} in {WORKSPACE_SKILLS} {verb} not loaded: this directory is not trusted"
        )));
        return;
    }

    for name in names {
        let relative = format!("{WORKSPACE_SKILLS}/{name}/{SKILL_FILE}");

        let Ok(contents) = workspace.read(policy, &Labelled::trusted(relative.clone())) else {
            continue;
        };

        // Asked of the label before it is asked of the gate. The gate is still the only thing
        // that hands bytes over, and it still runs whenever this proceeds; what this avoids is
        // recording a denial for a condition that is ordinary and expected, which would mark
        // every turn in an untrusted directory as one where something was refused and teach the
        // user to ignore the times it means something.
        if !contents.label().is_trusted() {
            notices.push(Notice::new(format!(
                "{relative} was not loaded: it is not trusted"
            )));
            continue;
        }
        let Ok(text) = policy.read_trusted_content("skills", &contents) else {
            continue;
        };

        match parse_frontmatter(&text) {
            Some(front) => {
                let body = policy.render_in_place("skills", &contents, |whole| {
                    body_after_frontmatter(&whole).to_string()
                });
                catalogue.insert(Skill {
                    name: front.name,
                    description: front.description,
                    body,
                    origin: relative,
                });
            }
            None => notices.push(Notice::new(format!(
                "{relative} was skipped: it needs a name and a description in its frontmatter"
            ))),
        }
    }
}

/// A count of skills, and the verb that agrees with it, so a line does not read "1 skills were"
/// or "1 skill were".
fn counted(n: usize) -> (String, &'static str) {
    if n == 1 {
        ("1 skill".to_string(), "was")
    } else {
        (format!("{n} skills"), "were")
    }
}

/// The names of the directories under a skills root, sorted.
///
/// Sorted so a turn offers the same skills in the same order every time. An order that came from
/// the filesystem would vary by machine, which would make the prompt vary with it.
fn skill_directories(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| root.join(name).join(SKILL_FILE).is_file())
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The planner chooses a skill from its name and description alone, so a file declaring only
    /// one of them offers nothing to choose on and must not be advertised at all.
    #[test]
    fn frontmatter_without_a_name_or_description_is_skipped() {
        assert_eq!(parse_frontmatter("---\nname: only\n---\nbody\n"), None);
        assert_eq!(
            parse_frontmatter("---\ndescription: only\n---\nbody\n"),
            None
        );
        assert_eq!(parse_frontmatter("---\nname:\ndescription: x\n---\n"), None);
    }

    /// An ordinary markdown file in a skills directory is not a skill. Treating one as a skill
    /// would put a heading in the system prompt as though the user had declared it.
    #[test]
    fn a_file_with_no_frontmatter_is_not_a_skill() {
        assert_eq!(parse_frontmatter("# notes\n\nsome prose\n"), None);
        assert_eq!(parse_frontmatter(""), None);
    }

    /// Without a closing marker there is no bounded block, and every line of the file would be
    /// read as a declaration. Refusing is the safe reading of an ambiguous file.
    #[test]
    fn an_unterminated_frontmatter_block_is_skipped_rather_than_swallowing_the_body() {
        let text = "---\nname: runaway\ndescription: never closed\n\nbody: text\n";
        assert_eq!(parse_frontmatter(text), None);
    }

    /// A file written for another agent may carry keys this does not know. Ignoring them is what
    /// lets one skill directory serve more than one tool.
    #[test]
    fn keys_other_than_name_and_description_are_ignored() {
        let text = "---\nname: shared\nlicense: MPL-2.0\ndescription: works anyway\n---\nbody\n";
        assert_eq!(
            parse_frontmatter(text),
            Some(Frontmatter {
                name: "shared".to_string(),
                description: "works anyway".to_string(),
            })
        );
    }

    /// A description is a sentence, and sentences contain colons. Splitting on the last one, or
    /// refusing the line, would mangle exactly the text the planner decides from.
    #[test]
    fn a_value_may_contain_the_separator() {
        let text = "---\nname: n\ndescription: use this: always\n---\n";
        let parsed = parse_frontmatter(text).expect("parses");
        assert_eq!(parsed.description, "use this: always");
    }

    /// A description says when to use a skill, so it runs to a sentence or two and people wrap
    /// it. Every real skill file in this repository does, and reading only the first line of one
    /// left the value empty and the skill silently dropped.
    #[test]
    fn a_value_wrapped_over_several_lines_is_one_value() {
        let text = "---\nname: n\ndescription:\n  'Check the specs, clause by clause. Runs the\n  \
                    mechanical pass. Triggers on: check spec, spec drift.'\nargument-hint: '[x]'\n---\n";
        let parsed = parse_frontmatter(text).expect("parses");
        assert_eq!(
            parsed.description,
            "Check the specs, clause by clause. Runs the mechanical pass. Triggers on: check \
             spec, spec drift."
        );
    }

    /// A wrapped sentence contains colons, and a continuation line is not a declaration. Reading
    /// one as a key ends the value early and puts half a sentence in the prompt.
    #[test]
    fn a_continuation_line_holding_a_colon_does_not_start_a_new_key() {
        let text = "---\nname: n\ndescription: use it when\n  this holds: always\n---\n";
        let parsed = parse_frontmatter(text).expect("parses");
        assert_eq!(parsed.description, "use it when this holds: always");
    }

    /// The quotes are YAML's, put there so a value may open with a character that would otherwise
    /// mean something. Leaving them in shows the planner an apostrophe at each end.
    #[test]
    fn the_quotes_around_a_scalar_are_not_part_of_it() {
        let text = "---\nname: 'n'\ndescription: \"say when\"\n---\n";
        let parsed = parse_frontmatter(text).expect("parses");
        assert_eq!(parsed.name, "n");
        assert_eq!(parsed.description, "say when");
    }

    /// `>` folds and `|` keeps its newlines, which is the whole difference between the two and
    /// the only reason a file would choose one.
    #[test]
    fn a_folded_block_becomes_one_line_and_a_literal_block_keeps_its_own() {
        let folded = parse_frontmatter("---\nname: n\ndescription: >\n  one\n  two\n---\n");
        assert_eq!(folded.expect("parses").description, "one two");
        let literal = parse_frontmatter("---\nname: n\ndescription: |\n  one\n  two\n---\n");
        assert_eq!(literal.expect("parses").description, "one\ntwo");
    }

    /// The planner already has the name and the description. Sending them again spends context
    /// on what it used to make the call.
    #[test]
    fn the_body_is_everything_after_the_closing_marker() {
        let text = "---\nname: n\ndescription: d\n---\n\nline one\nline two\n";
        assert_eq!(body_after_frontmatter(text), "line one\nline two\n");
    }

    /// A body may itself contain a horizontal rule. Stripping at the last marker rather than the
    /// first would swallow the part of the skill above it.
    #[test]
    fn a_marker_inside_the_body_is_left_alone() {
        let text = "---\nname: n\ndescription: d\n---\nabove\n\n---\n\nbelow\n";
        assert_eq!(body_after_frontmatter(text), "above\n\n---\n\nbelow\n");
    }

    /// A file with no frontmatter is not a skill, but asking for its body must still return the
    /// file rather than nothing, since a caller uses this to decide what to show.
    #[test]
    fn a_file_without_frontmatter_is_all_body() {
        assert_eq!(body_after_frontmatter("just prose\n"), "just prose\n");
        assert_eq!(body_after_frontmatter(""), "");
    }
}
