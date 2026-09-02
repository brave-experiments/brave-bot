//! The execution state a bounded run carries instead of its history.
//!
//! A turn re-sends the whole conversation every round, so the request grows with the length of
//! the run. [`crate::policy::Policy::adopt_summary`] shortens it after the fact by replacing the
//! older part with a paraphrase. This is the other answer: the history is never sent at all, and
//! what stands in its place is a structure the model maintains on purpose.
//!
//! Each step the model is shown the task, this state, and the newest observation. It answers with
//! a patch and an action. The patch is merged, the reasoning that produced it is dropped, and the
//! next step sees the result. Nothing accumulates, so the request has a size rather than a growth
//! rate.
//!
//! # Why this is a kernel type
//!
//! Two reasons, and neither is that the state is secret.
//!
//! The **merge** decides what the model will be shown next, so it decides what the model decides
//! from. A merge that lost a key would make a run forget something it had recorded, and a merge
//! that let one grow without bound would give back the unbounded request this exists to avoid.
//! That is a rule about information flow, so it lives where the rules do.
//!
//! The **rendering** is the other half of the same point. What goes into the request is this
//! structure written out, and writing it out is where a key holding a quote mark could otherwise
//! close the string it was in and put structure of its own into the prompt. So the kernel owns
//! the escaping, and there is no second renderer anywhere for a caller to reach for instead.
//!
//! # What it may hold
//!
//! Whatever the model may already say out loud, and nothing else. A patch is model output and is
//! adopted through a gate that refuses once the context has gone untrusted, exactly as a summary
//! is: see [`crate::policy::Policy::adopt_state_patch`]. So the state can hold the *name* of a
//! quarantined thing, `ref:3`, and can never hold a byte of what is behind it. That is the same
//! bargain the rest of the system runs on, and here it is what makes carrying the state forward
//! carry no untrusted bytes forward.

use std::collections::BTreeMap;
use std::fmt;

/// How deep a state may nest.
///
/// Deep enough for the shape a real schema wants, which is a few named groups of named fields, and
/// shallow enough that the rendered form cannot become a tree nobody can read. A run that wants
/// more nesting than this wants a flatter schema.
pub const MAX_DEPTH: usize = 6;

/// How many entries one map may hold.
///
/// A bound per map rather than only on the whole, so a single runaway key cannot spend the entire
/// budget and leave a state that is technically small and practically one enormous list.
pub const MAX_ENTRIES: usize = 256;

/// How large the rendered state may be, in bytes.
///
/// **This is the whole point of the mode**, so it is a refusal and not a nudge. The bound on the
/// request is this number plus the task and one observation, whatever the length of the run, and a
/// state allowed to creep past it would give back the growth the mode exists to remove. Sized for
/// the shape the paper measured, a handful of named fields holding identifiers and short strings,
/// with room to spare for a schema that wants more.
pub const MAX_BYTES: usize = 8 * 1024;

/// One value in a state.
///
/// A closed set on purpose. The model proposes these, so the set of things it can propose is the
/// set of things something has to be able to check, and "any JSON" is not a set anybody bounds.
///
/// There is no null. In a patch, absent-with-intent is how a key is deleted, and that is
/// [`Patch`]'s business rather than a value anything can hold: a state that could store null would
/// have two ways to say a key is not set, and code that treated them differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    /// A whole number. Signed, because a count that can go down is a normal thing for a state to
    /// hold and a run should not have to encode one as text to keep it.
    Number(i64),
    Bool(bool),
    /// An ordered sequence. Replaced wholesale by a patch rather than merged element by element:
    /// there is no key to merge on, and a list that merged by position would rewrite the wrong
    /// element the moment anything was inserted.
    List(Vec<Value>),
    /// A named group, merged key by key. See [`State::merged`].
    Map(BTreeMap<String, Value>),
}

impl Value {
    /// How deep this value nests, counting itself.
    fn depth(&self) -> usize {
        match self {
            Self::Text(_) | Self::Number(_) | Self::Bool(_) => 1,
            Self::List(values) => 1 + values.iter().map(Self::depth).max().unwrap_or(0),
            Self::Map(entries) => 1 + entries.values().map(Self::depth).max().unwrap_or(0),
        }
    }

    /// The widest map anywhere inside this value, counting itself.
    fn widest(&self) -> usize {
        match self {
            Self::Text(_) | Self::Number(_) | Self::Bool(_) => 0,
            Self::List(values) => values.iter().map(Self::widest).max().unwrap_or(0),
            Self::Map(entries) => entries
                .len()
                .max(entries.values().map(Self::widest).max().unwrap_or(0)),
        }
    }

    /// Write this value as JSON, compactly and deterministically.
    fn render_into(&self, out: &mut String) {
        match self {
            Self::Text(text) => render_string(text, out),
            Self::Number(number) => out.push_str(&number.to_string()),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::List(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    value.render_into(out);
                }
                out.push(']');
            }
            Self::Map(entries) => {
                out.push('{');
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    render_string(key, out);
                    out.push(':');
                    value.render_into(out);
                }
                out.push('}');
            }
        }
    }
}

/// Write a JSON string, escaping what has to be escaped.
///
/// The reason the kernel owns rendering. A key or a value holding a quote mark, a backslash or a
/// newline, written out as it stands, would close the string it was in and put structure into the
/// prompt that no state ever held. Escaped here, the rendered form is always exactly the state
/// that was checked.
///
/// Control characters go out as `\u00XX` rather than being dropped, because dropping one would
/// make two different states render identically and a run unable to tell them apart.
fn render_string(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// What a patch does to one key.
///
/// Three cases, because the paper's JSON has three: a value replaces, a null deletes, and an
/// object merges into whatever object is already there. Spelling them as separate variants rather
/// than as an `Option<Value>` is what makes nested deletion expressible at all: `{"inventory":
/// {"shelf_42": null}}` is a [`Change::Merge`] holding a [`Change::Delete`], and there is no way
/// to write that with a value type that has no null in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Replace whatever is at this key.
    Set(Value),
    /// Remove this key and whatever is under it.
    Delete,
    /// Merge into the group at this key, recursively.
    Merge(Patch),
}

/// What one step proposes doing to the state.
///
/// **A key the patch does not mention is left exactly as it was.** That is the single most
/// important thing about this type. The paper measured its most common failure on smaller models
/// as the state being overwritten rather than merged, at 68% of all errors, and the shape of a
/// patch is what makes that unrepresentable here: there is no way to express "and the rest is
/// gone", so forgetting to repeat a key cannot lose it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Patch {
    entries: BTreeMap<String, Change>,
}

impl Patch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace whatever is at this key.
    pub fn set(mut self, key: impl Into<String>, value: Value) -> Self {
        self.entries.insert(key.into(), Change::Set(value));
        self
    }

    /// Remove this key.
    pub fn delete(mut self, key: impl Into<String>) -> Self {
        self.entries.insert(key.into(), Change::Delete);
        self
    }

    /// Merge into the group at this key.
    pub fn merge(mut self, key: impl Into<String>, nested: Patch) -> Self {
        self.entries.insert(key.into(), Change::Merge(nested));
        self
    }

    /// Record a change spelled however the caller decoded it.
    pub fn with(mut self, key: impl Into<String>, change: Change) -> Self {
        self.entries.insert(key.into(), change);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// What this patch does, in a line, for the person watching and for the audit trail.
    ///
    /// Keys only, never values. A value can be long, and this goes on a screen beside everything
    /// else the step did. A nested change is named by its path, `inventory.shelf_42`, because
    /// "set inventory" for a patch that touched one shelf of five hundred says almost nothing.
    pub fn describe(&self) -> String {
        let mut set = Vec::new();
        let mut cleared = Vec::new();
        self.paths(&mut String::new(), &mut set, &mut cleared);

        let mut parts = Vec::new();
        if !set.is_empty() {
            parts.push(format!("set {}", set.join(", ")));
        }
        if !cleared.is_empty() {
            parts.push(format!("cleared {}", cleared.join(", ")));
        }
        if parts.is_empty() {
            "left the state alone".to_string()
        } else {
            parts.join("; ")
        }
    }

    /// Collect the dotted paths this patch sets and clears.
    fn paths(&self, prefix: &mut String, set: &mut Vec<String>, cleared: &mut Vec<String>) {
        for (key, change) in &self.entries {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match change {
                Change::Set(_) => set.push(path),
                Change::Delete => cleared.push(path),
                Change::Merge(nested) => {
                    let mut deeper = path;
                    nested.paths(&mut deeper, set, cleared);
                }
            }
        }
    }
}

/// Why a patch was refused.
///
/// Every one of these names something the model can do differently, because the answer to a
/// refused patch is to tell the model what was wrong and let it try again. A reason a model cannot
/// act on would be a reason to end the run instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// The state would nest deeper than [`MAX_DEPTH`].
    TooDeep { depth: usize },
    /// Some map would hold more than [`MAX_ENTRIES`] keys.
    TooWide { entries: usize },
    /// The state would render larger than [`MAX_BYTES`].
    TooLarge { bytes: usize },
    /// A key was empty or was nothing but whitespace.
    ///
    /// Refused rather than accepted because it cannot be referred to: a state with a key nobody
    /// can name holds something no later patch can update or delete.
    EmptyKey,
    /// A patch tried to merge a map into something that is not one, or the reverse.
    ///
    /// Named separately from the caps because it is the one refusal that says the model has
    /// misunderstood its own schema rather than merely overrun a bound.
    Conflict { key: String },
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooDeep { depth } => write!(
                f,
                "the state would nest {depth} levels deep and the limit is {MAX_DEPTH}; flatten it"
            ),
            Self::TooWide { entries } => write!(
                f,
                "one group would hold {entries} keys and the limit is {MAX_ENTRIES}; summarise \
                 rather than listing every one"
            ),
            Self::TooLarge { bytes } => write!(
                f,
                "the state would come to {bytes} bytes and the limit is {MAX_BYTES}; drop what is \
                 finished with and keep what the rest of the work needs"
            ),
            Self::EmptyKey => write!(
                f,
                "a key was empty, and a key nothing can name is not usable"
            ),
            Self::Conflict { key } => write!(
                f,
                "'{key}' is a group in one and a single value in the other; set it outright if the \
                 shape was meant to change"
            ),
        }
    }
}

impl std::error::Error for StateError {}

/// The state one bounded run carries.
///
/// Constructed empty and only ever changed by [`State::merged`], which returns a new one rather
/// than editing this: a patch that fails must leave the run with the state it already had, and the
/// easiest way to guarantee that is to have nothing to undo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    entries: BTreeMap<String, Value>,
}

impl State {
    /// The state a run begins with, which is empty.
    ///
    /// Empty rather than seeded from the task. The task is sent beside the state every step, so a
    /// copy of it here would be the one thing in the request that was sent twice.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key)
    }

    /// This state with the patch applied, or why it was refused.
    ///
    /// Pure and total, in the sense the manifest validator is: it looks at a state and a patch and
    /// nothing else, it decides the same way every time, and it either produces a whole new state
    /// or changes nothing at all. There is no half-merged state, and no key repaired to make a
    /// patch fit.
    ///
    /// **Maps merge, everything else replaces.** Setting a key whose value is a map merges into
    /// that map key by key, recursively, so a patch touching one shelf leaves the other four
    /// hundred alone. Setting a key whose value is a list, a number, a string or a boolean
    /// replaces it, because there is no key to merge those on. Deleting removes the key and
    /// whatever was under it.
    ///
    /// The caps are checked on the **result**, not on the patch. A small patch can produce an
    /// oversized state, and it is the state that is going into the next request.
    pub fn merged(&self, patch: &Patch) -> Result<Self, StateError> {
        let mut entries = self.entries.clone();
        merge_into(&mut entries, &patch.entries)?;
        let candidate = Self { entries };

        let depth = candidate.depth();
        if depth > MAX_DEPTH {
            return Err(StateError::TooDeep { depth });
        }
        let widest = candidate.widest();
        if widest > MAX_ENTRIES {
            return Err(StateError::TooWide { entries: widest });
        }
        let bytes = candidate.render().len();
        if bytes > MAX_BYTES {
            return Err(StateError::TooLarge { bytes });
        }

        Ok(candidate)
    }

    fn depth(&self) -> usize {
        self.entries.values().map(Value::depth).max().unwrap_or(0)
    }

    fn widest(&self) -> usize {
        self.entries
            .len()
            .max(self.entries.values().map(Value::widest).max().unwrap_or(0))
    }

    /// The state as compact JSON, which is the form that goes into the request.
    ///
    /// Deterministic: keys come out in one order because the maps are ordered, so the same state
    /// renders to the same bytes every time. That is what makes a run reproducible, and what lets
    /// a size be checked here and relied on there.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push('{');
        for (index, (key, value)) in self.entries.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            render_string(key, &mut out);
            out.push(':');
            value.render_into(&mut out);
        }
        out.push('}');
        out
    }

    /// How much of the byte budget the state is using, as a percentage.
    ///
    /// For the person watching. A run whose state is filling up is a run about to start refusing
    /// patches, and that is worth seeing before it happens rather than after.
    pub fn occupancy(&self) -> u8 {
        let used = self.render().len().min(MAX_BYTES);
        ((used * 100) / MAX_BYTES) as u8
    }
}

/// Whether anywhere inside this value there is a key nothing can name.
///
/// A key is how a later patch reaches what is under it, so one that is empty or all whitespace
/// holds something the run can never update or delete. Checked at every depth and through lists,
/// because a group arriving whole brings its own keys with it.
fn names_something_unnameable(value: &Value) -> bool {
    match value {
        Value::Text(_) | Value::Number(_) | Value::Bool(_) => false,
        Value::List(values) => values.iter().any(names_something_unnameable),
        Value::Map(entries) => entries
            .iter()
            .any(|(key, value)| key.trim().is_empty() || names_something_unnameable(value)),
    }
}

/// Merge patch entries into state entries, recursively.
///
/// Split out so the recursion has one implementation. The rule is stated on [`State::merged`].
fn merge_into(
    entries: &mut BTreeMap<String, Value>,
    patch: &BTreeMap<String, Change>,
) -> Result<(), StateError> {
    for (key, proposed) in patch {
        if key.trim().is_empty() {
            return Err(StateError::EmptyKey);
        }

        match proposed {
            Change::Delete => {
                entries.remove(key);
            }
            Change::Merge(nested) => match entries.get_mut(key) {
                // A group into a group merges, which is what keeps a patch about one field from
                // dropping every sibling of it.
                Some(Value::Map(existing)) => merge_into(existing, &nested.entries)?,
                // Nothing there yet. The group is built from the patch, which lets a run create a
                // group and fill its first key in one step.
                None => {
                    let mut fresh = BTreeMap::new();
                    merge_into(&mut fresh, &nested.entries)?;
                    entries.insert(key.clone(), Value::Map(fresh));
                }
                // A group arriving where a single value lives. The model has changed its mind
                // about the shape of its own schema, which is worth saying rather than silently
                // resolving: whichever side is discarded, something the run recorded is gone.
                Some(_) => return Err(StateError::Conflict { key: key.clone() }),
            },
            // Replaces whatever was there, group included. A decoder turns a JSON object into
            // `Merge` and never into this, so a `Set` carrying a group is a caller saying
            // "replace it outright", which is a thing a run is allowed to want: a group whose
            // work is finished is cheaper to overwrite than to empty key by key.
            //
            // The value is checked as well as the key. A group arriving whole can carry keys of
            // its own, at any depth and inside a list, and one of those being unnameable is the
            // same defect as a top-level one: it holds something no later patch can reach. The
            // caps are recursive, so this has to be too.
            Change::Set(value) => {
                if names_something_unnameable(value) {
                    return Err(StateError::EmptyKey);
                }
                entries.insert(key.clone(), value.clone());
            }
        }
    }

    // Nested maps can be deleted down to nothing, and an empty group left behind would be a key
    // the model has to keep reading and can no longer learn anything from. Pruned here rather than
    // at the deletion, so one pass covers however deep the patch reached.
    prune(entries);
    Ok(())
}

/// Drop groups that no longer hold anything.
fn prune(entries: &mut BTreeMap<String, Value>) {
    let empty: Vec<String> = entries
        .iter_mut()
        .filter_map(|(key, value)| match value {
            Value::Map(nested) => {
                prune(nested);
                nested.is_empty().then(|| key.clone())
            }
            _ => None,
        })
        .collect();
    for key in empty {
        entries.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Value {
        Value::Text(value.to_string())
    }

    fn map(pairs: &[(&str, Value)]) -> Value {
        Value::Map(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    /// A run begins with nothing, and nothing renders as an empty object rather than as nothing at
    /// all: the model is shown a state every step, and a blank where one was promised reads as a
    /// bug in the harness.
    #[test]
    fn a_new_state_is_empty_and_renders_as_an_object() {
        let state = State::new();
        assert!(state.is_empty());
        assert_eq!(state.render(), "{}");
    }

    #[test]
    fn setting_a_key_puts_it_in_the_state() {
        let state = State::new()
            .merged(&Patch::new().set("working_dir", text("/tmp")))
            .expect("a first patch");
        assert_eq!(state.get("working_dir"), Some(&text("/tmp")));
    }

    /// The property the whole type exists for, and the one the paper measured as the most common
    /// failure of every runtime that did not have it: a patch that mentions one key must not lose
    /// the others. 68% of the errors on smaller models were exactly this.
    #[test]
    fn a_key_the_patch_does_not_mention_survives_it() {
        let state = State::new()
            .merged(
                &Patch::new()
                    .set("working_dir", text("/tmp"))
                    .set("flags_found", Value::Number(2)),
            )
            .expect("a first patch")
            .merged(&Patch::new().set("working_dir", text("/srv")))
            .expect("a second patch");

        assert_eq!(state.get("working_dir"), Some(&text("/srv")));
        assert_eq!(
            state.get("flags_found"),
            Some(&Value::Number(2)),
            "a key the patch said nothing about was lost"
        );
    }

    /// The same property one level down, which is where it actually gets tested in anger: the
    /// paper's warehouse holds five hundred shelves in one group and a step touches one of them.
    #[test]
    fn a_patch_into_a_group_leaves_the_rest_of_the_group_alone() {
        let state = State::new()
            .merged(&Patch::new().set(
                "inventory",
                map(&[
                    ("shelf_1", text("item_a")),
                    ("shelf_2", text("item_b")),
                    ("shelf_3", text("item_c")),
                ]),
            ))
            .expect("a first patch")
            .merged(&Patch::new().merge("inventory", Patch::new().set("shelf_2", text("item_z"))))
            .expect("a second patch");

        let Some(Value::Map(inventory)) = state.get("inventory") else {
            panic!("the group is gone");
        };
        assert_eq!(inventory.get("shelf_1"), Some(&text("item_a")));
        assert_eq!(inventory.get("shelf_2"), Some(&text("item_z")));
        assert_eq!(inventory.get("shelf_3"), Some(&text("item_c")));
    }

    /// The paper's own worked example: a customer orders `item_12`, the agent ships it from
    /// `shelf_42`, and the patch is `{"inventory": {"shelf_42": null}}`. The shelf empties and its
    /// neighbour is untouched.
    #[test]
    fn deleting_a_key_inside_a_group_removes_only_that_key() {
        let state = State::new()
            .merged(&Patch::new().set(
                "inventory",
                map(&[("shelf_41", text("item_11")), ("shelf_42", text("item_12"))]),
            ))
            .expect("a first patch")
            .merged(&Patch::new().merge("inventory", Patch::new().delete("shelf_42")))
            .expect("a nested delete");

        let Some(Value::Map(inventory)) = state.get("inventory") else {
            panic!("the group is gone");
        };
        assert_eq!(inventory.get("shelf_42"), None, "the shelf did not empty");
        assert_eq!(
            inventory.get("shelf_41"),
            Some(&text("item_11")),
            "a neighbouring shelf was emptied too"
        );
    }

    /// A group can be created and filled in one step, so a run does not need a patch whose only
    /// purpose is to declare an empty group before anything can go in it.
    #[test]
    fn merging_into_a_group_that_does_not_exist_yet_creates_it() {
        let state = State::new()
            .merged(&Patch::new().merge("inventory", Patch::new().set("shelf_1", text("item_a"))))
            .expect("a patch");

        let Some(Value::Map(inventory)) = state.get("inventory") else {
            panic!("the group was not created");
        };
        assert_eq!(inventory.get("shelf_1"), Some(&text("item_a")));
    }

    #[test]
    fn deleting_a_key_removes_it() {
        let state = State::new()
            .merged(&Patch::new().set("scratch", text("something")))
            .expect("a first patch")
            .merged(&Patch::new().delete("scratch"))
            .expect("a delete");
        assert_eq!(state.get("scratch"), None);
    }

    /// Deleting something that was never there is not an error. The model is working from a state
    /// it can see, but a patch that tidies up defensively should not fail the step over it.
    #[test]
    fn deleting_a_key_that_is_not_there_is_not_an_error() {
        assert!(
            State::new()
                .merged(&Patch::new().delete("never-set"))
                .is_ok()
        );
    }

    /// A group emptied by deletions goes with them. Left behind it would be a key the model reads
    /// every step and can learn nothing from.
    #[test]
    fn a_group_emptied_of_everything_is_pruned() {
        let state = State::new()
            .merged(&Patch::new().set("group", map(&[("only", text("value"))])))
            .expect("a first patch");
        assert!(state.get("group").is_some());

        // Deleting the one key inside it, rather than the group itself, is what leaves an empty
        // group behind for pruning to find.
        let state = state
            .merged(&Patch::new().merge("group", Patch::new().delete("only")))
            .expect("a nested delete");
        assert_eq!(state.get("group"), None);
    }

    /// A list is replaced rather than merged: there is no key to merge on, and merging by position
    /// would rewrite the wrong element as soon as anything was inserted.
    #[test]
    fn a_list_is_replaced_rather_than_merged() {
        let state = State::new()
            .merged(&Patch::new().set(
                "tested",
                Value::List(vec![text("one"), text("two"), text("three")]),
            ))
            .expect("a first patch")
            .merged(&Patch::new().set("tested", Value::List(vec![text("four")])))
            .expect("a second patch");

        assert_eq!(state.get("tested"), Some(&Value::List(vec![text("four")])));
    }

    /// Merging into something that is not a group. The model has written `{"field": {...}}` where
    /// `field` holds a single value, so it has changed its mind about the shape of its own schema.
    /// Resolving it silently would discard one side or the other, and whichever went, something the
    /// run had recorded would be gone with no mention of it.
    #[test]
    fn merging_a_group_into_a_single_value_is_refused() {
        let scalar = State::new()
            .merged(&Patch::new().set("field", text("a string")))
            .expect("a first patch");

        assert_eq!(
            scalar.merged(&Patch::new().merge("field", Patch::new().set("nested", text("x")))),
            Err(StateError::Conflict {
                key: "field".to_string()
            })
        );
    }

    /// Replacing outright is always allowed, whatever was there. A group whose work is finished is
    /// cheaper to overwrite than to empty one key at a time, and a run that could not do it would
    /// carry the leftovers in every request for the rest of its life.
    #[test]
    fn setting_a_key_outright_replaces_whatever_shape_was_there() {
        let group = State::new()
            .merged(&Patch::new().set("field", map(&[("nested", text("x"))])))
            .expect("a first patch")
            .merged(&Patch::new().set("field", text("a string")))
            .expect("replacing a group with a value");
        assert_eq!(group.get("field"), Some(&text("a string")));

        let scalar = State::new()
            .merged(&Patch::new().set("field", text("a string")))
            .expect("a first patch")
            .merged(&Patch::new().set("field", map(&[("nested", text("x"))])))
            .expect("replacing a value with a group");
        assert_eq!(scalar.get("field"), Some(&map(&[("nested", text("x"))])));
    }

    /// A refused patch must leave the run with the state it already had. Nothing is half applied,
    /// which is what lets the driver simply tell the model what was wrong and ask again.
    #[test]
    fn a_refused_patch_changes_nothing() {
        let before = State::new()
            .merged(&Patch::new().set("kept", text("value")))
            .expect("a first patch");

        // A patch whose first key is fine and whose second conflicts. Applied left to right
        // without this property, the first would have landed.
        let refused = before.merged(
            &Patch::new()
                .set("also-fine", text("value"))
                .merge("kept", Patch::new().set("nested", text("x"))),
        );
        assert!(refused.is_err(), "{refused:?}");

        assert_eq!(before.get("also-fine"), None, "half of a patch was applied");
        assert_eq!(before.get("kept"), Some(&text("value")));
    }

    /// A key nothing can name holds something no later patch can update or delete.
    #[test]
    fn an_empty_key_is_refused() {
        for key in ["", " ", "\t", "\n"] {
            assert_eq!(
                State::new().merged(&Patch::new().set(key, text("value"))),
                Err(StateError::EmptyKey),
                "{key:?} was accepted"
            );
        }
    }

    /// At any depth, and through a list. A group arriving whole brings its own keys with it, and one
    /// of those being unnameable is the same defect as a top-level one: the caps are recursive, so
    /// this has to be. Found by review: only the top level was checked.
    #[test]
    fn an_empty_key_inside_a_value_is_refused_at_any_depth() {
        let buried = [
            // Directly inside a group being set outright.
            Patch::new().set("group", map(&[("", text("x"))])),
            // Inside a group nested in a group.
            Patch::new().set("outer", map(&[("inner", map(&[(" ", text("x"))]))])),
            // Inside a group inside a list, which is the shape a decoder builds from JSON and the
            // one that got through.
            Patch::new().set("items", Value::List(vec![map(&[("", text("x"))])])),
            // And through a merge, whose nested keys go the other route.
            Patch::new().merge("group", Patch::new().set("", text("x"))),
        ];

        for patch in buried {
            assert_eq!(
                State::new().merged(&patch),
                Err(StateError::EmptyKey),
                "an unnameable key was accepted: {patch:?}"
            );
        }
    }

    /// The bound the mode exists for. A state allowed to creep past it would give back the
    /// growing request that this whole design is here to remove.
    #[test]
    fn a_state_over_the_byte_budget_is_refused() {
        let huge = "x".repeat(MAX_BYTES + 1);
        let refused = State::new().merged(&Patch::new().set("field", text(&huge)));
        assert!(
            matches!(refused, Err(StateError::TooLarge { .. })),
            "{refused:?}"
        );
    }

    /// And the refusal says what to do about it, because the answer to an oversized state is for
    /// the model to drop what it is finished with and try again.
    #[test]
    fn an_oversized_state_says_what_would_fix_it() {
        let huge = "x".repeat(MAX_BYTES + 1);
        let refused = State::new()
            .merged(&Patch::new().set("field", text(&huge)))
            .expect_err("too large");
        let said = refused.to_string();
        assert!(said.contains("drop what is finished with"), "{said}");
    }

    #[test]
    fn a_state_nested_deeper_than_the_limit_is_refused() {
        let mut value = text("bottom");
        for _ in 0..MAX_DEPTH + 2 {
            value = map(&[("down", value)]);
        }
        let refused = State::new().merged(&Patch::new().set("top", value));
        assert!(
            matches!(refused, Err(StateError::TooDeep { .. })),
            "{refused:?}"
        );
    }

    #[test]
    fn a_group_wider_than_the_limit_is_refused() {
        let mut wide = BTreeMap::new();
        for index in 0..MAX_ENTRIES + 1 {
            wide.insert(format!("k{index}"), Value::Number(index as i64));
        }
        let refused = State::new().merged(&Patch::new().set("group", Value::Map(wide)));
        assert!(
            matches!(refused, Err(StateError::TooWide { .. })),
            "{refused:?}"
        );
    }

    /// The caps describe the result rather than the patch, because it is the result that goes into
    /// the next request. A small patch onto a nearly full state is how the budget actually gets
    /// spent.
    #[test]
    fn a_small_patch_that_would_overfill_the_state_is_refused() {
        let nearly = "x".repeat(MAX_BYTES - 64);
        let state = State::new()
            .merged(&Patch::new().set("bulk", text(&nearly)))
            .expect("a state just under the cap");

        let refused = state.merged(&Patch::new().set("one_more", text(&"y".repeat(200))));
        assert!(
            matches!(refused, Err(StateError::TooLarge { .. })),
            "{refused:?}"
        );
    }

    /// The same state must render to the same bytes every time, or a run is not reproducible and
    /// a size checked here means nothing there.
    #[test]
    fn rendering_is_deterministic_whatever_order_keys_arrived_in() {
        let one = State::new()
            .merged(
                &Patch::new()
                    .set("zebra", text("z"))
                    .set("alpha", text("a"))
                    .set("middle", text("m")),
            )
            .expect("a patch");
        let other = State::new()
            .merged(
                &Patch::new()
                    .set("middle", text("m"))
                    .set("zebra", text("z"))
                    .set("alpha", text("a")),
            )
            .expect("a patch");

        assert_eq!(one.render(), other.render());
        assert_eq!(one.render(), r#"{"alpha":"a","middle":"m","zebra":"z"}"#);
    }

    /// The reason rendering is the kernel's. A key holding a quote mark, written out as it stands,
    /// would close the string it was in and put structure into the prompt that no state held.
    #[test]
    fn a_key_or_value_holding_json_punctuation_cannot_add_structure() {
        let state = State::new()
            .merged(
                &Patch::new()
                    .set(r#"evil":{"injected"#, text("x"))
                    .set("value", text(r#"","also":"injected"#)),
            )
            .expect("a patch");

        let rendered = state.render();
        // Every quote that is part of the content is escaped, so the only unescaped ones are the
        // ones this renderer put there to delimit the four strings and the two keys.
        assert!(
            rendered.contains(r#"\"[]"#) || rendered.contains(r#"\""#),
            "{rendered}"
        );
        assert!(!rendered.contains(r#""evil":{"injected""#), "{rendered}");
        assert!(!rendered.contains(r#""also":"injected""#), "{rendered}");
    }

    #[test]
    fn a_newline_in_a_value_does_not_break_the_line() {
        let state = State::new()
            .merged(&Patch::new().set("note", text("first\nsecond")))
            .expect("a patch");
        let rendered = state.render();
        assert!(!rendered.contains('\n'), "{rendered}");
        assert!(rendered.contains("\\n"), "{rendered}");
    }

    /// A patch says what it did in keys, never in values: this goes on a screen beside everything
    /// else the step did, and a value can be the length of a file.
    #[test]
    fn a_patch_describes_itself_by_key_and_never_by_value() {
        let described = Patch::new()
            .set(
                "found",
                text("a very long secret value nobody wants on screen"),
            )
            .delete("stale")
            .describe();

        assert!(described.contains("found"), "{described}");
        assert!(described.contains("stale"), "{described}");
        assert!(!described.contains("secret"), "{described}");
    }

    #[test]
    fn a_patch_that_does_nothing_says_so() {
        assert_eq!(Patch::new().describe(), "left the state alone");
    }

    /// For the person watching. A run whose state is filling up is about to start refusing
    /// patches, and that is worth seeing before it happens.
    #[test]
    fn occupancy_grows_with_the_state_and_never_passes_a_hundred() {
        assert_eq!(State::new().occupancy(), 0);

        let half = State::new()
            .merged(&Patch::new().set("bulk", text(&"x".repeat(MAX_BYTES / 2))))
            .expect("a state around half the cap");
        assert!(
            (40..=60).contains(&half.occupancy()),
            "{}",
            half.occupancy()
        );
    }
}
