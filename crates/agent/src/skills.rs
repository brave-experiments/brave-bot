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
/// on a line and nothing else, so there is no parser to surprise us and nothing to backtrack.
/// Keys other than `name` and `description` are ignored rather than refused, which leaves room
/// for a file written for another agent to work here too.
///
/// `None` means "not a skill", and every caller drops the file on that answer. A half-declared
/// skill is included in that: a name with no description is one the planner cannot choose
/// between, and advertising it would be worse than leaving it out.
pub fn parse_frontmatter(text: &str) -> Option<Frontmatter> {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some(MARKER) {
        return None;
    }

    let mut name = None;
    let mut description = None;
    let mut closed = false;

    for line in lines {
        if line.trim_end() == MARKER {
            closed = true;
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match key.trim() {
            "name" => name = Some(value),
            "description" => description = Some(value),
            _ => {}
        }
    }

    // An unterminated block is not frontmatter that happens to run long: it is a file whose whole
    // contents would otherwise be read as declarations.
    if !closed {
        return None;
    }

    let name = name.filter(|n| !n.is_empty())?;
    let description = description.filter(|d| !d.is_empty())?;
    Some(Frontmatter { name, description })
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
