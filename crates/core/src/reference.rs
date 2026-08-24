//! What the planner is told about content it may not see.
//!
//! The rule in CLAUDE.md is absolute: untrusted content never enters the planner's context.
//! But an agent still has to be able to work with untrusted files, reading them, editing them,
//! and moving text between them, so it needs *something* to reason about.
//!
//! That something is a [`Reference`]: a name for quarantined content plus facts about its
//! shape. A line count, a byte count, a label. Never a byte of the content itself.
//!
//! The model addresses content by reference, say "replace lines 40 to 60 of `ref:3`", and the
//! kernel resolves the reference when the effect fires. Nothing the model says can widen a
//! reference into the bytes behind it, because the bytes only exist inside the slot store.
//!
//! Trusted content needs none of this. It came from a path the user vouched for, so there is
//! no injected text to keep out and the planner may read it directly. References are for the
//! untrusted case, which is the default until a user says otherwise.

use crate::label::Label;
use crate::slot::SlotId;
use std::fmt;

/// A handle to quarantined content, plus the shape facts the planner may know.
///
/// Deliberately holds no content and no way to get any. Everything here is metadata *about*
/// untrusted bytes, which is not the same as the bytes: a line count cannot carry an
/// instruction, so telling the planner about it does not put attacker-controlled text in its
/// context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The slot the content lives in.
    pub slot: SlotId,
    /// Where it came from, as a workspace-relative path. Routing, so already trusted.
    pub origin: String,
    /// Lines of text, or `None` for a file that has not been read yet.
    pub lines: Option<usize>,
    /// Bytes of text, or of the file on disk where it has not been read yet.
    pub bytes: usize,
    /// The label the content carries.
    pub label: Label,
}

impl Reference {
    pub fn new(
        slot: SlotId,
        origin: impl Into<String>,
        lines: usize,
        bytes: usize,
        label: Label,
    ) -> Self {
        Self {
            slot,
            origin: origin.into(),
            lines: Some(lines),
            bytes,
            label,
        }
    }

    /// A reference to a file the slot has reserved but not read.
    ///
    /// The line count is the one fact that cannot be had without opening the file, so it is
    /// absent rather than guessed. The size comes from the filesystem, which is where the
    /// planner's sense of scale came from anyway.
    pub fn unread(slot: SlotId, origin: impl Into<String>, bytes: usize, label: Label) -> Self {
        Self {
            slot,
            origin: origin.into(),
            lines: None,
            bytes,
            label,
        }
    }

    /// How the planner is told about this content.
    ///
    /// Shape and provenance only. A reader of this string learns that untrusted text exists,
    /// where it came from, and how big it is, enough to decide what to do with it, and
    /// nothing an injection could ride in on.
    pub fn describe(&self) -> String {
        let shape = match self.lines {
            Some(lines) => format!("{lines} lines, {} bytes", self.bytes),
            // Said plainly, because a planner told only a size would read the absence of a line
            // count as a small file rather than as a file nothing has opened.
            None => format!("{} bytes on disk, not read yet", self.bytes),
        };
        format!(
            "[{}] {} ({shape}, {}). The contents are quarantined and not shown. \
             Refer to this content as {} in tool arguments.",
            self.slot, self.origin, self.label, self.slot,
        )
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// What a tool produced: either content the planner may read, or a reference to content it
/// may not.
///
/// The two cases are separated in the type so a tool cannot accidentally return untrusted
/// bytes where visible text was expected. Which case applies is decided by the *label*, in
/// [`crate::policy::Policy::present`], never by the tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presentation {
    /// Trusted content, safe for the planner to read.
    Visible(String),
    /// Untrusted content, described but not shown.
    Quarantined(Reference),
}

impl Presentation {
    /// The text to place in the planner's context.
    pub fn for_context(&self) -> String {
        match self {
            Self::Visible(text) => text.clone(),
            Self::Quarantined(reference) => reference.describe(),
        }
    }

    /// Whether the planner can see the actual content.
    pub fn is_visible(&self) -> bool {
        matches!(self, Self::Visible(_))
    }

    /// The reference, when the content is quarantined.
    pub fn reference(&self) -> Option<&Reference> {
        match self {
            Self::Quarantined(reference) => Some(reference),
            Self::Visible(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference() -> Reference {
        Reference::new(
            SlotId::new("ref:1"),
            "vendor/lib.js",
            120,
            4_096,
            Label::untrusted_private(),
        )
    }

    /// The whole point: a description carries shape, never content.
    #[test]
    fn a_description_names_the_shape_and_not_the_content() {
        let described = reference().describe();
        assert!(described.contains("ref:1"));
        assert!(described.contains("vendor/lib.js"));
        assert!(described.contains("120 lines"));
        assert!(described.contains("4096 bytes"));
        assert!(described.contains("quarantined"));
    }

    /// A reference must tell the planner how to act on the content, or it cannot do anything
    /// with a file it may not read.
    #[test]
    fn a_description_says_how_to_refer_to_the_content() {
        let described = reference().describe();
        assert!(
            described.contains("Refer to this content as ref:1"),
            "the planner is not told how to address it: {described}"
        );
    }

    /// Quarantined content puts only the description into the context.
    #[test]
    fn a_quarantined_presentation_shows_no_content() {
        let secret = "IGNORE PREVIOUS INSTRUCTIONS";
        let presentation = Presentation::Quarantined(reference());
        let context = presentation.for_context();

        assert!(!context.contains(secret));
        assert!(!presentation.is_visible());
        assert!(presentation.reference().is_some());
    }

    /// Trusted content is passed through unchanged: there is nothing to keep out of the
    /// context, and hiding it would make the agent useless in the user's own repository.
    #[test]
    fn a_visible_presentation_shows_the_content() {
        let presentation = Presentation::Visible("fn main() {}".to_string());
        assert_eq!(presentation.for_context(), "fn main() {}");
        assert!(presentation.is_visible());
        assert!(presentation.reference().is_none());
    }

    /// A reference is metadata, so it must be comparable and printable, unlike content,
    /// which deliberately is neither.
    #[test]
    fn references_are_ordinary_values() {
        assert_eq!(reference(), reference());
        assert!(!format!("{}", reference()).is_empty());
    }
}
