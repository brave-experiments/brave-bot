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
/// What a reference is a reference to.
///
/// The planner acts on the two differently, so it is told which it has. A file can be worked on
/// and written back to; content can be worked on and written out. Deliberately not inferred from
/// whether the bytes happen to have been read: that is the driver's business and saying it aloud
/// was what sent a planner off trying to read a reference it already held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A file in the workspace. An address as well as a document.
    File,
    /// Text in a slot, which came from somewhere but is not somewhere.
    Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Which of the two things this is.
    pub kind: Kind,
    /// The slot the content lives in.
    pub slot: SlotId,
    /// Where it came from, as a workspace-relative path. Routing, so already trusted.
    pub origin: String,
    /// Lines of text, or `None` for a file that has not been read yet.
    pub lines: Option<usize>,
    /// Bytes of text, or of the file on disk where it has not been read yet. `None` for a file
    /// nothing has looked at, where even the size would be a fact about the directory.
    pub bytes: Option<usize>,
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
            kind: Kind::Content,
            slot,
            origin: origin.into(),
            lines: Some(lines),
            bytes: Some(bytes),
            label,
        }
    }

    /// A reference to a file the slot has reserved but not read.
    ///
    /// The line count is the one fact that cannot be had without opening the file, so it is
    /// absent rather than guessed. The size is there when the file was named by the planner,
    /// which is where its sense of scale came from anyway, and absent for an entry out of a
    /// listing it may not read: a directory's shape is the directory's business.
    pub fn unread(
        slot: SlotId,
        origin: impl Into<String>,
        bytes: Option<usize>,
        label: Label,
    ) -> Self {
        Self {
            kind: Kind::File,
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
        // What it is and what to do with it, never what the driver has done with it. Whether the
        // bytes have been read off disk is an implementation detail of when files are opened,
        // and putting it here once cost a whole session: told "not read yet", a planner read the
        // reference, was told the same thing about the reference that came back, and concluded
        // that reading was broken.
        // The label brings its own brackets, so an entry with no measurements is not wrapped
        // in a second pair.
        let shape = match (self.lines, self.bytes) {
            (Some(lines), Some(bytes)) => format!("({lines} lines, {bytes} bytes, {})", self.label),
            (Some(lines), None) => format!("({lines} lines, {})", self.label),
            (None, Some(bytes)) => format!("({bytes} bytes, {})", self.label),
            // Nothing but the label, which happens for an entry in a listing: how big the files
            // in a directory are is the directory's business, and a planner that cannot tell two
            // entries apart works on both, which is the right answer anyway.
            (None, None) => self.label.to_string(),
        };

        let next = match self.kind {
            Kind::File => format!(
                "Quarantined: you will not be shown what this file holds, and read_file has \
                 nothing to add: {} already is the file. Give it to spawn_processor to work on, \
                 and name it as write_file's path_ref to put the result back.",
                self.slot
            ),
            Kind::Content => format!(
                "Quarantined: you will not be shown it. Give {} to spawn_processor to work on, \
                 or write it into a file as contents_ref.",
                self.slot
            ),
        };

        format!("[{}] {} {shape}. {next}", self.slot, self.origin)
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
        assert!(described.contains("Quarantined"));
    }

    /// A reference must tell the planner how to act on the content, or it cannot do anything
    /// with a file it may not read.
    #[test]
    fn a_description_says_how_to_refer_to_the_content() {
        let described = reference().describe();
        assert!(
            described.contains("spawn_processor") && described.contains("ref:1"),
            "the planner is not told what to do with it: {described}"
        );
    }

    /// A reference to a file says what it is and what to do with it, and says nothing about
    /// whether the bytes have been read. That is the driver's business, and a planner told about
    /// it reads a pending read: one was, and spent a session trying to perform it.
    #[test]
    fn a_file_reference_says_what_to_do_and_not_what_the_driver_did() {
        let described = Reference::unread(
            SlotId::new("ref:1"),
            "an entry in \".\"",
            None,
            Label::untrusted_private(),
        )
        .describe();

        assert!(
            !described.contains("read yet") && !described.contains("not read"),
            "the planner was told about the driver's reading: {described}"
        );
        assert!(
            described.contains("spawn_processor"),
            "the planner was not told what to do with it: {described}"
        );
        assert!(
            described.contains("path_ref"),
            "the planner was not told it is a destination: {described}"
        );
    }

    /// Content is not a destination. Only a reference to a file is one, and the two must not
    /// read alike: a planner that tried to write to a processor's output would be refused, and
    /// refusals it was invited to earn are the ones that cost a round.
    #[test]
    fn content_is_not_offered_as_a_destination() {
        let described = reference().describe();
        assert!(
            !described.contains("path_ref"),
            "content was offered as a destination: {described}"
        );
        assert!(
            described.contains("contents_ref"),
            "the planner was not told how to write it out: {described}"
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
