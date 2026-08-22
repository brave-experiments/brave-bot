//! The audit trail, as a file rather than a screen.
//!
//! The same events Ctrl-T shows, written down so they can be read after the fact: which gate
//! checked what, the label every value carried, and what was released. One JSON object per line,
//! because a session's trail is appended a turn at a time and read with whatever is to hand.
//!
//! The two axes are written out in words rather than as the compact form the screen uses.
//! `(U,priv)` is right for a line of a terminal, where the reader has the legend in front of
//! them; `"integrity": "untrusted", "confidentiality": "private"` is right for a file being read
//! six months later by someone answering a question about what happened.
//!
//! Shaping events here rather than deriving it in the kernel is deliberate. `bua-core` has no
//! dependencies at all, and a serialisation format is a presentation concern: the same events
//! already become terminal lines a few modules away.
//!
//! Nothing in here is content. Every field is a gate name, a capability, a label, a path or a
//! slot id, which is the same reason the trail can be shown on a screen without a release.

use bua_core::event::{Event, Role};
use bua_core::label::{Confidentiality, Integrity, Label};
use serde_json::{Value, json};

/// One event, as it is written down.
pub fn as_json(event: &Event) -> Value {
    match event {
        Event::GatePassed { gate, detail } => json!({
            "kind": "gate_passed",
            "gate": gate,
            "detail": detail,
        }),
        Event::GateBlocked {
            gate,
            detail,
            reason,
        } => json!({
            "kind": "gate_blocked",
            "gate": gate,
            "detail": detail,
            "reason": reason,
        }),
        Event::SlotWritten { slot, label } => json!({
            "kind": "slot_written",
            "slot": slot.as_str(),
            "label": label_json(*label),
        }),
        Event::Observed { capability, label } => json!({
            "kind": "observed",
            "capability": capability.to_string(),
            "label": label_json(*label),
        }),
        Event::Declassified {
            slot,
            from,
            to,
            reason,
        } => json!({
            "kind": "declassified",
            "slot": slot.as_str(),
            "from": label_json(*from),
            "to": label_json(*to),
            "reason": reason,
        }),
        Event::ActionField {
            tool,
            field,
            role,
            label,
            allowed,
        } => json!({
            "kind": "action_field",
            "tool": tool,
            "field": field,
            "role": match role {
                Role::Routing => "routing",
                Role::Content => "content",
            },
            "label": label_json(*label),
            "allowed": allowed,
        }),
    }
}

/// A label, with both axes named.
fn label_json(label: Label) -> Value {
    json!({
        "integrity": match label.integrity {
            Integrity::Trusted => "trusted",
            Integrity::Untrusted => "untrusted",
        },
        "confidentiality": match label.confidentiality {
            Confidentiality::Public => "public",
            Confidentiality::Private => "private",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bua_core::capability::Capability;
    use bua_core::slot::SlotId;

    /// The four words the whole model is stated in have to appear in the file, or an audit is a
    /// list of gate names with no answer to the question it exists for.
    #[test]
    fn both_axes_are_written_out_in_words() {
        let written = as_json(&Event::Observed {
            capability: Capability::FileRead,
            label: Label::untrusted_private(),
        });
        assert_eq!(written["label"]["integrity"], "untrusted");
        assert_eq!(written["label"]["confidentiality"], "private");

        let written = as_json(&Event::Observed {
            capability: Capability::FileRead,
            label: Label::trusted_public(),
        });
        assert_eq!(written["label"]["integrity"], "trusted");
        assert_eq!(written["label"]["confidentiality"], "public");
    }

    /// A refusal is the line an audit is read for, and it has to say what was refused and why.
    #[test]
    fn a_refusal_records_what_it_refused_and_why() {
        let written = as_json(&Event::GateBlocked {
            gate: "trusted-read",
            detail: "edit_file".to_string(),
            reason: "content is untrusted".to_string(),
        });
        assert_eq!(written["kind"], "gate_blocked");
        assert_eq!(written["gate"], "trusted-read");
        assert_eq!(written["reason"], "content is untrusted");
    }

    /// A release is the other one: it is the moment content left, and the audit says where from
    /// and to what.
    #[test]
    fn a_release_records_both_ends_of_it() {
        let written = as_json(&Event::Declassified {
            slot: SlotId::new("ref:3"),
            from: Label::untrusted_private(),
            to: Label::untrusted_public(),
            reason: "shown to the user",
        });
        assert_eq!(written["slot"], "ref:3");
        assert_eq!(written["from"]["confidentiality"], "private");
        assert_eq!(written["to"]["confidentiality"], "public");
    }

    #[test]
    fn a_field_check_records_the_role_it_was_checked_as() {
        let written = as_json(&Event::ActionField {
            tool: "write_file".to_string(),
            field: "path".to_string(),
            role: Role::Routing,
            label: Label::trusted_public(),
            allowed: true,
        });
        assert_eq!(written["role"], "routing");
        assert_eq!(written["allowed"], true);
    }

    /// One line per event, so a file can be read with ordinary tools.
    #[test]
    fn an_event_fits_on_one_line() {
        let written = as_json(&Event::GatePassed {
            gate: "capability",
            detail: "file_read granted".to_string(),
        })
        .to_string();
        assert!(!written.contains('\n'));
    }
}
