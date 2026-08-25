//! Laying out what loaded, what did not, and what it costs.
//!
//! One renderer for both interfaces. The terminal session shows these lines in the transcript
//! and the one-shot command prints them, and they say the same thing because they are the same
//! lines: two layouts would eventually disagree about the thing a user consults them to settle.
//!
//! # Control characters are stripped here
//!
//! A skill's description reaches this from a file. In a trusted path that file is the user's
//! own, which is not the same as saying it holds nothing surprising: an escape sequence pasted
//! into a description would be interpreted by a terminal that receives it as bytes. The session
//! is safe on its own, since ratatui writes into a cell buffer rather than to the terminal, but
//! `println!` is not, so the stripping happens once here and covers both.

use bua_agent::skills::Inventory;

/// The column the second field starts at.
///
/// A terminal is often 80 wide and the transcript indents, so anything past this soft-wraps and
/// the layout stops being a layout. Fields are truncated to fit rather than left to wrap.
const FIELD: usize = 34;

/// The widest line worth emitting.
const WIDTH: usize = 76;

/// The lines describing an inventory, ready to print or to put in the transcript.
pub fn lines(inventory: &Inventory) -> Vec<String> {
    let mut out = vec!["Skills".to_string()];

    if inventory.loaded.is_empty() {
        out.push(String::new());
        out.push("  nothing loaded".to_string());
    } else {
        out.push(String::new());
        out.push("  loaded".to_string());
        for skill in &inventory.loaded {
            out.push(format!(
                "    {}{}",
                pad(&clean(&skill.name), FIELD - 4),
                clean(&skill.origin)
            ));
            out.push(format!(
                "      {}",
                truncate(&clean(&skill.description), WIDTH - 6)
            ));
        }
    }

    if !inventory.shadowed.is_empty() {
        out.push(String::new());
        out.push("  shadowed".to_string());
        for entry in &inventory.shadowed {
            out.push(format!(
                "    {}{}",
                pad(&clean(&entry.name), FIELD - 4),
                clean(&entry.hidden)
            ));
            out.push(format!("      replaced by {}", clean(&entry.winner)));
        }
    }

    if !inventory.skipped.is_empty() {
        out.push(String::new());
        out.push("  not loaded".to_string());
        for entry in &inventory.skipped {
            // A preamble refusal has no origin of its own: the reason is the whole sentence.
            if entry.origin.is_empty() {
                out.push(format!(
                    "    {}",
                    truncate(&clean(&entry.reason), WIDTH - 4)
                ));
                continue;
            }
            out.push(format!("    {}", clean(&entry.origin)));
            let detail = match entry.shape {
                Some(shape) => format!(
                    "{} lines, {} bytes, {}",
                    shape.lines,
                    shape.bytes,
                    reason_tail(&entry.reason)
                ),
                None => reason_tail(&entry.reason).to_string(),
            };
            out.push(format!("      {}", truncate(&clean(&detail), WIDTH - 6)));
        }
    }

    out.push(String::new());
    out.push(format!(
        "  {}{}",
        pad("project", FIELD - 2),
        clean(&inventory.project.root)
    ));
    out.push(format!("      {}", project_state(&inventory.project)));

    if !inventory.agents.is_empty() {
        out.push(format!(
            "  {}{}",
            pad("standing instructions", FIELD - 2),
            inventory
                .agents
                .iter()
                .map(|a| clean(a))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push(format!(
        "  {}{} bytes, roughly {} tokens per request",
        pad("preamble", FIELD - 2),
        inventory.preamble_bytes,
        inventory.approximate_tokens()
    ));

    out
}

/// What the project's line says about itself.
///
/// Always said, even when there is nothing to report, because an empty listing is otherwise two
/// different situations wearing the same face: a directory with no skills, and a directory whose
/// skills are not being shown. Telling those apart is the question this command answers.
fn project_state(project: &bua_agent::skills::Project) -> &'static str {
    match (project.trusted, project.has_skills_directory) {
        (true, true) => "trusted",
        (true, false) => "trusted, and no .bua/skills directory",
        (false, true) => "not trusted, so nothing here was loaded",
        (false, false) => "not trusted, and no .bua/skills directory",
    }
}

/// The part of a reason worth showing beside a path, without repeating "was not loaded".
///
/// The notice sentence reads well on its own mid-session; in a column under a heading that
/// already says these were not loaded, the prefix is noise.
fn reason_tail(reason: &str) -> &str {
    reason
        .strip_prefix("was not loaded: ")
        .or_else(|| reason.strip_prefix("was skipped: "))
        .unwrap_or(reason)
}

/// Drop control characters.
///
/// Everything here came from a file, and a file can hold an escape sequence whether or not
/// anyone meant it to. Dropping rather than escaping keeps the column widths honest.
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Pad to a column, or leave a single space when the field is already too wide to pad.
fn pad(text: &str, width: usize) -> String {
    let shown = truncate(text, width.saturating_sub(1));
    let used = crate::wrap::display_width(&shown);
    let spaces = width.saturating_sub(used).max(1);
    format!("{shown}{}", " ".repeat(spaces))
}

/// Cut to a display width, marking that something was cut.
fn truncate(text: &str, width: usize) -> String {
    if crate::wrap::display_width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        if crate::wrap::display_width(&out) + 1 >= width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bua_agent::skills::{Inventory, Loaded, Project, Shadowed, Shape, Skipped};

    fn rendered(inventory: &Inventory) -> String {
        lines(inventory).join("\n")
    }

    /// A description comes out of a file, and a file can hold an escape sequence whether or not
    /// anyone meant it to. The one-shot command prints these straight to a terminal.
    #[test]
    fn control_characters_never_reach_the_output() {
        let inventory = Inventory {
            loaded: vec![Loaded {
                name: "evil".to_string(),
                description: "before\u{1b}[2Jafter".to_string(),
                origin: "~/.bua/skills/evil/SKILL.md".to_string(),
                bytes: 10,
            }],
            ..Default::default()
        };

        let out = rendered(&inventory);
        assert!(!out.contains('\u{1b}'), "an escape survived: {out:?}");
        // The escape byte goes and the rest stays visible. Dropping the whole sequence would
        // hide that anything odd was in the file, which is the opposite of what a listing
        // consulted for debugging should do.
        assert!(
            out.contains("before[2Jafter"),
            "the text itself was lost: {out}"
        );
    }

    /// A rejected file needs to be distinguishable from one that was never there, and its size
    /// is what says so without quoting a line of it.
    #[test]
    fn a_rejected_skill_shows_its_shape_and_not_its_contents() {
        let inventory = Inventory {
            skipped: vec![Skipped {
                origin: ".bua/skills/hostile/SKILL.md".to_string(),
                reason: "was not loaded: it is not trusted".to_string(),
                shape: Some(Shape {
                    lines: 14,
                    bytes: 480,
                }),
            }],
            ..Default::default()
        };

        let out = rendered(&inventory);
        assert!(out.contains("not loaded"));
        assert!(out.contains(".bua/skills/hostile/SKILL.md"));
        assert!(out.contains("14 lines, 480 bytes"), "{out}");
        assert!(out.contains("not trusted"));
    }

    /// The heading already says these were not loaded, so repeating it in every row is noise.
    #[test]
    fn a_reason_is_not_repeated_under_the_heading_that_states_it() {
        assert_eq!(
            reason_tail("was not loaded: it is not trusted"),
            "it is not trusted"
        );
        assert_eq!(
            reason_tail("was skipped: it needs a name"),
            "it needs a name"
        );
        assert_eq!(reason_tail("something else"), "something else");
    }

    /// Shadowing is invisible in the running agent, so the listing is the only place it can be
    /// seen. Both sides have to be named or it does not explain anything.
    #[test]
    fn a_shadowed_skill_names_both_sides() {
        let inventory = Inventory {
            shadowed: vec![Shadowed {
                name: "commit-style".to_string(),
                hidden: "~/.bua/skills/commit-style/SKILL.md".to_string(),
                winner: ".bua/skills/commit-style/SKILL.md".to_string(),
            }],
            ..Default::default()
        };

        let out = rendered(&inventory);
        assert!(out.contains("shadowed"));
        assert!(out.contains("~/.bua/skills/commit-style/SKILL.md"));
        assert!(
            out.contains("replaced by .bua/skills/commit-style/SKILL.md"),
            "{out}"
        );
    }

    /// Nothing installed is the first run, and it should read as an answer rather than as a
    /// broken screen.
    #[test]
    fn an_empty_inventory_says_so() {
        let out = rendered(&Inventory::default());
        assert!(out.contains("nothing loaded"), "{out}");
        assert!(out.contains("preamble"), "{out}");
    }

    /// The four states have to read differently. A directory with no skills and a directory whose
    /// skills are being withheld look identical without this, and telling them apart is the whole
    /// question: someone who declined trust and sees an empty listing cannot otherwise know
    /// whether the command found nothing or is showing nothing.
    #[test]
    fn every_state_a_project_can_be_in_reads_differently() {
        let states = [(true, true), (true, false), (false, true), (false, false)];
        let mut seen: Vec<&str> = states
            .iter()
            .map(|&(trusted, has)| {
                project_state(&Project {
                    root: "~/work".to_string(),
                    trusted,
                    has_skills_directory: has,
                })
            })
            .collect();

        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            before,
            seen.len(),
            "two states say the same thing: {seen:?}"
        );
    }

    /// An empty project section is only unambiguous if the listing says where it looked.
    #[test]
    fn the_project_is_named_even_when_it_holds_nothing() {
        let inventory = Inventory {
            project: Project {
                root: "~/repos/thing".to_string(),
                trusted: false,
                has_skills_directory: false,
            },
            ..Default::default()
        };

        let out = rendered(&inventory);
        assert!(out.contains("~/repos/thing"), "{out}");
        assert!(out.contains("not trusted"), "{out}");
        assert!(out.contains("no .bua/skills directory"), "{out}");
    }

    /// The case that prompted this: declining trust in a project that does hold skills has to say
    /// so, or the listing looks the same as one with nothing to show.
    #[test]
    fn a_declined_project_with_skills_says_they_were_withheld() {
        let inventory = Inventory {
            project: Project {
                root: "~/repos/thing".to_string(),
                trusted: false,
                has_skills_directory: true,
            },
            ..Default::default()
        };

        assert!(
            rendered(&inventory).contains("nothing here was loaded"),
            "{}",
            rendered(&inventory)
        );
    }

    /// The layout stops being a layout the moment a line soft-wraps, and the transcript indents
    /// before this is drawn.
    #[test]
    fn no_line_is_wider_than_the_budget() {
        let inventory = Inventory {
            loaded: vec![Loaded {
                name: "a-very-long-skill-name-that-keeps-going-and-going".to_string(),
                description: "a description that runs on well past anything a terminal of \
                              ordinary width could hope to show on one line without wrapping"
                    .to_string(),
                origin: "~/.bua/skills/a-very-long-skill-name-that-keeps-going-and-going/SKILL.md"
                    .to_string(),
                bytes: 1,
            }],
            preamble_bytes: 1234,
            ..Default::default()
        };

        for line in lines(&inventory) {
            assert!(
                crate::wrap::display_width(&line) <= WIDTH + FIELD,
                "line is {} wide: {line:?}",
                crate::wrap::display_width(&line)
            );
        }
    }

    /// The number is an estimate and says so elsewhere; here it only has to be derived from the
    /// measurement rather than invented.
    #[test]
    fn the_cost_line_reports_both_the_measurement_and_the_estimate() {
        let inventory = Inventory {
            preamble_bytes: 1240,
            ..Default::default()
        };
        let out = rendered(&inventory);
        assert!(out.contains("1240 bytes"), "{out}");
        assert!(out.contains("roughly 310 tokens"), "{out}");
    }
}
