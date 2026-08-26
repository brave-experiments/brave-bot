//! Write-once quarantined storage.
//!
//! Untrusted content lands in a slot and stays there. Slots are written exactly
//! once, so content cannot be laundered by overwriting, and reads go through
//! capability-scoped handles that carry a label ceiling.
//!
//! Two structural guarantees, both enforced by the type system rather than by
//! runtime checks:
//!
//! - **Write-once**: [`SlotWriter::write`] consumes the writer, so a second write
//!   does not compile.
//! - **Label floor**: the writer's label is fixed when the writer is minted, by the
//!   policy layer, never chosen by the code doing the writing.
//!
//! A slot may also be **deferred**: it names a file it has not read, and the bytes arrive when
//! something finally needs them. What is deferred is only the reading. The slot is spoken for
//! from the moment it is deferred, its label is decided then rather than by whatever turns up,
//! and the reading is a single filling that cannot be repeated. Nothing may read a deferred
//! slot in the meantime: [`SlotStore::take_for_effect`] and [`SlotReader::read`] refuse it by
//! name rather than returning nothing, so a consumer that forgot to ask for the file is an
//! error and never an empty document.

use crate::label::Label;
use crate::value::Labelled;
use std::collections::HashMap;
use std::fmt;

/// Identifier for a slot. Trusted metadata: slot ids are chosen by the planner from
/// trusted input, never derived from content.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotId(String);

impl SlotId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SlotError {
    /// Something wanted the bytes of a slot whose file has not been read yet.
    Unread(SlotId),
    /// A slot was written twice. Reaching this at runtime means something bypassed
    /// the consuming [`SlotWriter`].
    AlreadyWritten(SlotId),
    /// Read of a slot that has not been written yet.
    NotWritten(SlotId),
    /// Read of a slot outside the reader's declared scope.
    OutOfScope(SlotId),
    /// A slot's label exceeds the reader's ceiling.
    CeilingExceeded {
        slot: SlotId,
        slot_label: Label,
        ceiling: Label,
    },
}

impl fmt::Display for SlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unread(s) => write!(
                f,
                "slot '{s}' names a file that has not been read yet, so it has no bytes to give"
            ),
            Self::AlreadyWritten(s) => write!(f, "slot '{s}' has already been written"),
            Self::NotWritten(s) => write!(f, "slot '{s}' has not been written yet"),
            Self::OutOfScope(s) => write!(f, "slot '{s}' is not in this reader's scope"),
            Self::CeilingExceeded {
                slot,
                slot_label,
                ceiling,
            } => write!(
                f,
                "label ceiling exceeded: slot '{slot}' is {slot_label} but the ceiling is {ceiling}"
            ),
        }
    }
}

impl std::error::Error for SlotError {}

/// A slot that names a file it has not read.
///
/// The path is routing: the planner named it and it was checked `(T,pub)` before the slot was
/// deferred, so holding it here decides nothing an attacker steers. The label is what the trust
/// map said when the promise was made, and it is a ceiling rather than a promise in its own
/// right: [`crate::policy::Policy::materialise`] takes the meet with what the map says when the
/// file is actually read, so a path that stopped being trusted in between cannot be read back
/// as though it had not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deferred {
    path: String,
    label: Label,
}

impl Deferred {
    /// The file this slot will hold.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The label recorded when the read was deferred.
    pub fn label(&self) -> Label {
        self.label
    }
}

/// What a slot holds: the bytes, or the promise of them.
///
/// A slot that came from a file keeps the path either way. That is what lets a reference be an
/// address as well as a document: the planner can name the slot as somewhere to read from or
/// write to without ever being told what the file is called, which is the only way to work in a
/// directory whose filenames are themselves untrusted.
#[derive(Debug, Clone)]
enum Entry {
    Read {
        value: Labelled<String>,
        /// The file it was read from, where it was read from one.
        path: Option<String>,
        /// The file these bytes *are*, byte for byte, where they are a file's own contents.
        ///
        /// Set when a slot is filled from disk, and carried over when a processor answers that a
        /// document should not change. It is what lets a write that would put a file back exactly
        /// as it is be recognised as changing nothing, from bookkeeping rather than by comparing
        /// bytes: the driver never reads either side.
        verbatim: Option<String>,
    },
    Unread(Deferred),
}

impl Entry {
    fn label(&self) -> Label {
        match self {
            Self::Read { value, .. } => value.label(),
            Self::Unread(deferred) => deferred.label,
        }
    }

    fn path(&self) -> Option<&str> {
        match self {
            Self::Read { path, .. } => path.as_deref(),
            Self::Unread(deferred) => Some(&deferred.path),
        }
    }

    fn verbatim(&self) -> Option<&str> {
        match self {
            Self::Read { verbatim, .. } => verbatim.as_deref(),
            // Nothing has been read, so nothing is a copy of anything.
            Self::Unread(_) => None,
        }
    }
}

/// Quarantined storage for one run.
///
/// Holds `Labelled<String>` because slot content is always opaque text as far as the
/// kernel is concerned; interpreting it is the job of whatever declassifies it.
#[derive(Debug, Default)]
pub struct SlotStore {
    slots: HashMap<SlotId, Entry>,
}

impl SlotStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_written(&self, id: &SlotId) -> bool {
        self.slots.contains_key(id)
    }

    pub fn label_of(&self, id: &SlotId) -> Option<Label> {
        self.slots.get(id).map(Entry::label)
    }

    /// The file a slot names, whether or not it has been read.
    ///
    /// The path itself, which for an entry out of a quarantined listing is untrusted content:
    /// only the policy layer may ask, and only through the gates that decide what a name may
    /// become. Kept after the bytes arrive, so processing a file does not lose the address it
    /// came from.
    pub(crate) fn path_of(&self, id: &SlotId) -> Option<&str> {
        self.slots.get(id).and_then(Entry::path)
    }

    /// The file a slot's bytes are a copy of, where they are one.
    ///
    /// Only the policy layer may ask: what it is for is recognising a write that would put a
    /// file back exactly as it is, and that is a decision.
    pub(crate) fn verbatim_of(&self, id: &SlotId) -> Option<&str> {
        self.slots.get(id).and_then(Entry::verbatim)
    }

    /// Record that one slot holds exactly what another does.
    ///
    /// For a processor that answered that a document should not change: what went into the new
    /// slot is the old one's bytes, so it is a copy of the same file.
    pub(crate) fn copied_from(&mut self, id: &SlotId, source: &SlotId) {
        let Some(origin) = self.verbatim_of(source).map(str::to_string) else {
            return;
        };
        if let Some(Entry::Read { verbatim, .. }) = self.slots.get_mut(id) {
            *verbatim = Some(origin);
        }
    }

    /// The file a slot is waiting on, where it is waiting on one.
    ///
    /// Metadata, like everything else a caller may ask a slot store: a path the planner chose
    /// and a label, never a byte of what the file holds.
    pub fn deferred(&self, id: &SlotId) -> Option<&Deferred> {
        match self.slots.get(id) {
            Some(Entry::Unread(deferred)) => Some(deferred),
            _ => None,
        }
    }

    /// Metadata for every slot: id, whether written, and label. Never contents, so
    /// this is safe to surface in an audit trail or a UI.
    pub fn inventory(&self) -> Vec<(SlotId, Label)> {
        let mut items: Vec<_> = self
            .slots
            .iter()
            .map(|(id, entry)| (id.clone(), entry.label()))
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        items
    }

    /// Hand a slot's content to an effect, still labelled.
    ///
    /// For the case where the planner named a reference and the kernel is carrying out what it
    /// asked. The value stays wrapped, so a caller can pass it to a write but not read it.
    ///
    /// Bypasses the reader ceiling deliberately: a ceiling limits what a *reader* may see, and
    /// this is not a read. Nobody sees these bytes; they travel from the slot to the effect.
    /// A slot still waiting on its file refuses by name. Returning nothing would read as an
    /// empty document, and an effect would carry out the emptiness.
    pub fn take_for_effect(&self, id: &SlotId) -> Result<Labelled<String>, SlotError> {
        match self.slots.get(id) {
            Some(Entry::Read { value, .. }) => Ok(value.clone()),
            Some(Entry::Unread(_)) => Err(SlotError::Unread(id.clone())),
            None => Err(SlotError::NotWritten(id.clone())),
        }
    }

    /// Record that a slot will hold `path`, without reading it.
    ///
    /// Only the policy layer defers, for the same reason only the policy layer mints a writer:
    /// the label is decided here and not by whatever eventually turns up in the file.
    pub(crate) fn defer(
        &mut self,
        id: SlotId,
        path: impl Into<String>,
        label: Label,
    ) -> Result<(), SlotError> {
        if self.is_written(&id) {
            return Err(SlotError::AlreadyWritten(id));
        }
        self.slots.insert(
            id,
            Entry::Unread(Deferred {
                path: path.into(),
                label,
            }),
        );
        Ok(())
    }

    /// Put the bytes into a slot that was waiting for them.
    ///
    /// The single write that a deferred slot gets: an entry already holding content is refused,
    /// so a file cannot be read twice into one slot and the second reading cannot be the one
    /// that counts.
    pub(crate) fn fill(
        &mut self,
        id: &SlotId,
        content: Labelled<String>,
    ) -> Result<Measured, SlotError> {
        let deferred = match self.slots.get(id) {
            Some(Entry::Unread(deferred)) => deferred.clone(),
            Some(Entry::Read { .. }) => return Err(SlotError::AlreadyWritten(id.clone())),
            None => return Err(SlotError::NotWritten(id.clone())),
        };

        // The recorded label is a ceiling, so a file read back out of a path that lost its
        // trust in the meantime lands untrusted rather than at the label it was promised.
        let label = crate::label::taint_all([deferred.label, content.label()]);

        // Reading to measure, not to decide, exactly as `write_measured` does.
        let proof = crate::value::Declassification::authorise("measured on the way into a slot");
        let text = content.declassify(&proof);
        let measured = Measured {
            lines: text.lines().count(),
            bytes: text.len(),
        };

        // The path stays with the slot now that the bytes are here. A reference that stopped
        // being an address the moment it was read could not be written back to.
        self.slots.insert(
            id.clone(),
            Entry::Read {
                value: Labelled::new(text, label),
                verbatim: Some(deferred.path.clone()),
                path: Some(deferred.path),
            },
        );
        Ok(measured)
    }

    /// Mint a single-use write capability with a fixed label.
    ///
    /// The label is supplied by the caller minting the writer, the policy layer,
    /// not by the code that later writes. `SlotWriter` cannot change it.
    pub fn writer_for(&mut self, id: SlotId, label: Label) -> Result<SlotWriter<'_>, SlotError> {
        if self.is_written(&id) {
            return Err(SlotError::AlreadyWritten(id));
        }
        Ok(SlotWriter {
            store: self,
            id,
            label,
        })
    }

    /// Mint a read capability scoped to exactly `ids`, with a label ceiling.
    ///
    /// The ceiling is checked here, at mint time, for every slot already written,
    /// so an over-privileged read fails when the capability is created rather than
    /// at some later read.
    pub fn reader_for(
        &self,
        ids: impl IntoIterator<Item = SlotId>,
        ceiling: Label,
    ) -> Result<SlotReader<'_>, SlotError> {
        let scope: Vec<SlotId> = ids.into_iter().collect();
        for id in &scope {
            // A slot with no label yet cannot be checked; the read itself will fail.
            // Written without a let-chain so older toolchains can build this.
            match self.label_of(id) {
                Some(slot_label) if !slot_label.flows_to(ceiling) => {
                    return Err(SlotError::CeilingExceeded {
                        slot: id.clone(),
                        slot_label,
                        ceiling,
                    });
                }
                _ => {}
            }
        }
        Ok(SlotReader {
            store: self,
            scope,
            ceiling,
        })
    }
}

/// A single-use write capability for one slot at one label.
///
/// [`SlotWriter::write`] takes `self` by value, so write-once is a compile-time
/// property: a second write has no writer left to call.
#[derive(Debug)]
pub struct SlotWriter<'a> {
    store: &'a mut SlotStore,
    id: SlotId,
    label: Label,
}

impl SlotWriter<'_> {
    /// The label this writer will apply. Fixed at mint time.
    pub fn label(&self) -> Label {
        self.label
    }

    pub fn slot_id(&self) -> &SlotId {
        &self.id
    }

    /// Write the slot, consuming the capability.
    pub fn write(self, content: impl Into<String>) -> Result<(), SlotError> {
        if self.store.is_written(&self.id) {
            return Err(SlotError::AlreadyWritten(self.id));
        }
        self.store.slots.insert(
            self.id,
            Entry::Read {
                value: Labelled::new(content.into(), self.label),
                path: None,
                verbatim: None,
            },
        );
        Ok(())
    }

    /// Write an already-labelled value, returning its measurements.
    ///
    /// The measuring happens here because the shape of untrusted content is the only thing
    /// anyone outside is allowed to learn about it. Measuring elsewhere would mean the caller
    /// holding the bytes, which is the thing quarantine exists to prevent.
    ///
    /// The value's own label is kept rather than the writer's when the value is *more*
    /// restricted, so quarantining cannot raise integrity or lower confidentiality by accident.
    pub fn write_measured(self, content: Labelled<String>) -> Result<Measured, SlotError> {
        if self.store.is_written(&self.id) {
            return Err(SlotError::AlreadyWritten(self.id));
        }

        let label = crate::label::taint_all([self.label, content.label()]);

        // Reading to measure, not to decide: nothing about the bytes influences control flow,
        // and the counts leave here as numbers.
        let proof = crate::value::Declassification::authorise("measured on the way into a slot");
        let text = content.declassify(&proof);
        let measured = Measured {
            lines: text.lines().count(),
            bytes: text.len(),
        };

        self.store.slots.insert(
            self.id,
            Entry::Read {
                value: Labelled::new(text, label),
                path: None,
                verbatim: None,
            },
        );
        Ok(measured)
    }
}

/// The shape of quarantined content: everything anyone outside the slot store may know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measured {
    pub lines: usize,
    pub bytes: usize,
}

/// A read capability scoped to a fixed set of slots, with a label ceiling.
#[derive(Debug)]
pub struct SlotReader<'a> {
    store: &'a SlotStore,
    scope: Vec<SlotId>,
    ceiling: Label,
}

impl SlotReader<'_> {
    pub fn ceiling(&self) -> Label {
        self.ceiling
    }

    pub fn scope(&self) -> &[SlotId] {
        &self.scope
    }

    /// Read a slot. Still labelled on the way out, so the caller gains no ability to
    /// inspect the content, only to carry it.
    ///
    /// The ceiling is re-checked here because a slot may have been written after this
    /// reader was minted, and that write could carry a higher label than the mint-time
    /// check saw.
    pub fn read(&self, id: &SlotId) -> Result<Labelled<String>, SlotError> {
        if !self.scope.contains(id) {
            return Err(SlotError::OutOfScope(id.clone()));
        }
        let value = match self.store.slots.get(id) {
            Some(Entry::Read { value, .. }) => value,
            Some(Entry::Unread(_)) => return Err(SlotError::Unread(id.clone())),
            None => return Err(SlotError::NotWritten(id.clone())),
        };
        if !value.label().flows_to(self.ceiling) {
            return Err(SlotError::CeilingExceeded {
                slot: id.clone(),
                slot_label: value.label(),
                ceiling: self.ceiling,
            });
        }
        Ok(value.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(s: &str) -> SlotId {
        SlotId::new(s)
    }

    /// A slot waiting on its file must say so rather than hand back nothing. An effect given
    /// nothing would carry out the emptiness: a write would truncate the file it was meant to
    /// change.
    #[test]
    fn a_slot_that_has_not_read_its_file_refuses_to_give_bytes() {
        let mut store = SlotStore::new();
        store
            .defer(sid("ref:0"), "notes.md", Label::untrusted_private())
            .unwrap();

        assert_eq!(
            store.take_for_effect(&sid("ref:0")).unwrap_err(),
            SlotError::Unread(sid("ref:0"))
        );

        let reader = store
            .reader_for([sid("ref:0")], Label::untrusted_private())
            .unwrap();
        assert_eq!(
            reader.read(&sid("ref:0")).unwrap_err(),
            SlotError::Unread(sid("ref:0"))
        );
    }

    /// Deferring reserves the slot. A second slot of the same name would mean two files behind
    /// one reference, and the reference the planner holds would name whichever won.
    #[test]
    fn a_reserved_slot_cannot_be_reserved_or_written_again() {
        let mut store = SlotStore::new();
        store
            .defer(sid("ref:0"), "notes.md", Label::untrusted_private())
            .unwrap();

        assert_eq!(
            store.defer(sid("ref:0"), "other.md", Label::untrusted_private()),
            Err(SlotError::AlreadyWritten(sid("ref:0")))
        );
        assert!(
            store
                .writer_for(sid("ref:0"), Label::untrusted_private())
                .is_err()
        );
    }

    /// Reading the file is the slot's single write, so a second reading has nowhere to go.
    #[test]
    fn a_file_is_read_into_its_slot_once() {
        let mut store = SlotStore::new();
        store
            .defer(sid("ref:0"), "notes.md", Label::untrusted_private())
            .unwrap();

        let measured = store
            .fill(
                &sid("ref:0"),
                Labelled::new("one\ntwo".to_string(), Label::untrusted_private()),
            )
            .expect("the first reading fills it");
        assert_eq!(measured.lines, 2);

        assert_eq!(
            store.fill(
                &sid("ref:0"),
                Labelled::new("something else".to_string(), Label::untrusted_private())
            ),
            Err(SlotError::AlreadyWritten(sid("ref:0")))
        );
    }

    /// The label recorded when the slot was reserved is a ceiling, never a promise: content that
    /// arrives worse than the reservation keeps its own label.
    #[test]
    fn filling_a_slot_cannot_raise_the_label_it_was_reserved_at() {
        let mut store = SlotStore::new();
        store
            .defer(sid("ref:0"), "notes.md", Label::untrusted_private())
            .unwrap();
        store
            .fill(
                &sid("ref:0"),
                Labelled::new("payload".to_string(), Label::trusted_public()),
            )
            .unwrap();

        let label = store.label_of(&sid("ref:0")).unwrap();
        assert!(
            !label.is_trusted(),
            "the reservation was untrusted: {label}"
        );
    }

    #[test]
    fn write_then_read_preserves_the_label() {
        let mut store = SlotStore::new();
        store
            .writer_for(sid("page"), Label::untrusted_public())
            .unwrap()
            .write("hello")
            .unwrap();

        let reader = store
            .reader_for([sid("page")], Label::untrusted_public())
            .unwrap();
        let value = reader.read(&sid("page")).unwrap();
        assert_eq!(value.label(), Label::untrusted_public());
    }

    /// Write-once is primarily a compile-time property, since `write` consumes the
    /// writer. This covers the runtime backstop: minting a second writer fails.
    #[test]
    fn a_slot_cannot_be_written_twice() {
        let mut store = SlotStore::new();
        store
            .writer_for(sid("page"), Label::untrusted_public())
            .unwrap()
            .write("first")
            .unwrap();

        let err = store
            .writer_for(sid("page"), Label::untrusted_public())
            .expect_err("second writer must be refused");
        assert_eq!(err, SlotError::AlreadyWritten(sid("page")));
    }

    /// The point of write-once: untrusted content cannot be replaced with a
    /// differently-labelled value.
    #[test]
    fn overwriting_cannot_launder_a_label() {
        let mut store = SlotStore::new();
        store
            .writer_for(sid("page"), Label::untrusted_public())
            .unwrap()
            .write("injected")
            .unwrap();

        assert!(
            store
                .writer_for(sid("page"), Label::trusted_public())
                .is_err()
        );
        assert_eq!(
            store.label_of(&sid("page")),
            Some(Label::untrusted_public())
        );
    }

    #[test]
    fn reading_outside_scope_is_refused() {
        let mut store = SlotStore::new();
        store
            .writer_for(sid("a"), Label::untrusted_public())
            .unwrap()
            .write("a")
            .unwrap();
        store
            .writer_for(sid("b"), Label::untrusted_public())
            .unwrap()
            .write("b")
            .unwrap();

        let reader = store
            .reader_for([sid("a")], Label::untrusted_public())
            .unwrap();
        assert_eq!(
            reader.read(&sid("b")).unwrap_err(),
            SlotError::OutOfScope(sid("b"))
        );
    }

    #[test]
    fn reading_an_unwritten_slot_is_refused() {
        let store = SlotStore::new();
        let reader = store
            .reader_for([sid("ghost")], Label::untrusted_public())
            .unwrap();
        assert_eq!(
            reader.read(&sid("ghost")).unwrap_err(),
            SlotError::NotWritten(sid("ghost"))
        );
    }

    /// A private slot cannot be pulled into a reader whose ceiling is public. This
    /// fails at mint time, not read time.
    #[test]
    fn ceiling_is_enforced_when_the_reader_is_minted() {
        let mut store = SlotStore::new();
        store
            .writer_for(sid("email"), Label::untrusted_private())
            .unwrap()
            .write("private body")
            .unwrap();

        let err = store
            .reader_for([sid("email")], Label::untrusted_public())
            .expect_err("private slot must not enter a public-ceiling reader");
        assert!(matches!(err, SlotError::CeilingExceeded { .. }));
    }

    /// A reader minted while a slot is still empty passes the mint-time ceiling check
    /// vacuously, so [`SlotReader::read`] re-checks. The write-after-mint sequence that
    /// would exploit the gap cannot be expressed in safe code: `SlotReader` holds an
    /// immutable borrow of the store and `writer_for` needs a mutable one, so this
    /// test drives `read`'s check directly rather than through that impossible
    /// interleaving. The runtime check stays as a backstop for any future interior
    /// mutability.
    #[test]
    fn read_refuses_a_slot_above_the_ceiling() {
        let mut store = SlotStore::new();
        store
            .writer_for(sid("email"), Label::untrusted_private())
            .unwrap()
            .write("private")
            .unwrap();

        // Ceiling high enough to mint, then read a slot that sits above a lower one.
        let reader = store
            .reader_for([sid("email")], Label::untrusted_private())
            .unwrap();
        assert!(reader.read(&sid("email")).is_ok());

        let public_reader = store.reader_for([sid("email")], Label::untrusted_public());
        assert!(matches!(
            public_reader.unwrap_err(),
            SlotError::CeilingExceeded { .. }
        ));
    }

    /// Minting a reader for a not-yet-written slot is allowed; the read fails instead.
    #[test]
    fn minting_for_an_unwritten_slot_defers_the_check() {
        let store = SlotStore::new();
        let reader = store
            .reader_for([sid("future")], Label::untrusted_public())
            .expect("unwritten slots cannot be ceiling-checked yet");
        assert_eq!(
            reader.read(&sid("future")).unwrap_err(),
            SlotError::NotWritten(sid("future"))
        );
    }

    #[test]
    fn a_writer_cannot_choose_its_own_label() {
        let mut store = SlotStore::new();
        let writer = store
            .writer_for(sid("s"), Label::untrusted_private())
            .unwrap();
        // The only label available is the one minted; there is no setter.
        assert_eq!(writer.label(), Label::untrusted_private());
        writer.write("x").unwrap();
        assert_eq!(store.label_of(&sid("s")), Some(Label::untrusted_private()));
    }

    #[test]
    fn inventory_reports_metadata_without_contents() {
        let mut store = SlotStore::new();
        store
            .writer_for(sid("b"), Label::untrusted_private())
            .unwrap()
            .write("secret-content")
            .unwrap();
        store
            .writer_for(sid("a"), Label::untrusted_public())
            .unwrap()
            .write("public-content")
            .unwrap();

        let inventory = store.inventory();
        assert_eq!(
            inventory,
            vec![
                (sid("a"), Label::untrusted_public()),
                (sid("b"), Label::untrusted_private()),
            ]
        );
        let rendered = format!("{inventory:?}");
        assert!(!rendered.contains("secret-content"));
    }
}
