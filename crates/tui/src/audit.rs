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

/// One event as the transcript shows it: the words, and whether it is a refusal.
///
/// The transcript holds these rather than the events themselves, because half of what it shows
/// did not happen in this process. A trail read back off disk is a *record* of an event and not
/// the event: an [`Event`] names its gate with a `&'static str` supplied by the code that emitted
/// it, and a name read out of a file only looks like one. Keeping the distinction means a resumed
/// trail cannot be mistaken for something a gate in this process decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrailLine {
    pub text: String,
    /// Whether this is something a gate refused, which is drawn differently.
    pub blocked: bool,
}

/// One event, as the transcript shows it.
///
/// The wording lives here rather than in the renderer so that [`recalled`] can produce the same
/// words for the same event. Two spellings of one line would drift the moment either changed.
pub fn as_line(event: &Event) -> TrailLine {
    match event {
        Event::GatePassed { gate, detail } => TrailLine::passed(format!("{gate}: {detail}")),
        Event::GateBlocked { gate, reason, .. } => TrailLine::blocked(format!("{gate}: {reason}")),
        Event::Observed { capability, label } => {
            TrailLine::passed(format!("{capability} produced {label}"))
        }
        Event::SlotWritten { slot, label } => TrailLine::passed(format!("slot {slot} at {label}")),
        Event::SlotDeferred {
            slot,
            label,
            origin,
        } => TrailLine::passed(format!("slot {slot} holds {origin}, unread, at {label}")),
        Event::Declassified { slot, from, to, .. } => {
            TrailLine::passed(format!("released {slot} {from} → {to}"))
        }
        Event::ActionField {
            tool,
            field,
            role,
            label,
            allowed,
        } => {
            let text = format!("{tool}.{field} [{}] {label}", role_word(*role));
            if *allowed {
                TrailLine::passed(text)
            } else {
                TrailLine::blocked(text)
            }
        }
    }
}

/// One event read back out of the audit file.
///
/// `None` for a line this build does not recognise, which is left out rather than shown as a
/// mangled one: an audit that cannot be read faithfully should say less, not say it wrong.
pub fn recalled(event: &Value) -> Option<TrailLine> {
    let text = match event["kind"].as_str()? {
        "gate_passed" => format!("{}: {}", event["gate"].as_str()?, event["detail"].as_str()?),
        "gate_blocked" => {
            return Some(TrailLine::blocked(format!(
                "{}: {}",
                event["gate"].as_str()?,
                event["reason"].as_str()?
            )));
        }
        "observed" => format!(
            "{} produced {}",
            event["capability"].as_str()?,
            label_text(&event["label"])
        ),
        "slot_written" => format!(
            "slot {} at {}",
            event["slot"].as_str()?,
            label_text(&event["label"])
        ),
        "slot_deferred" => format!(
            "slot {} holds {}, unread, at {}",
            event["slot"].as_str()?,
            event["origin"].as_str()?,
            label_text(&event["label"])
        ),
        "declassified" => format!(
            "released {} {} → {}",
            event["slot"].as_str()?,
            label_text(&event["from"]),
            label_text(&event["to"])
        ),
        "action_field" => {
            let text = format!(
                "{}.{} [{}] {}",
                event["tool"].as_str()?,
                event["field"].as_str()?,
                event["role"].as_str()?,
                label_text(&event["label"])
            );
            return Some(match event["allowed"].as_bool() {
                Some(true) => TrailLine::passed(text),
                // A missing or unreadable verdict is shown as a refusal, since a check whose
                // answer nobody can read is not one to draw as though it passed.
                _ => TrailLine::blocked(text),
            });
        }
        _ => return None,
    };
    Some(TrailLine::passed(text))
}

impl TrailLine {
    fn passed(text: String) -> Self {
        Self {
            text,
            blocked: false,
        }
    }

    fn blocked(text: String) -> Self {
        Self {
            text,
            blocked: true,
        }
    }
}

fn role_word(role: Role) -> &'static str {
    match role {
        Role::Routing => "routing",
        Role::Content => "content",
    }
}

/// A stored label in the compact form a terminal shows.
///
/// Unrecognised words read as the more restrictive axis, matching how everything else in this
/// tree degrades. Nothing turns on it here, since this is a line on a screen, but a label drawn
/// as better than it was would be the wrong thing to be relaxed about.
fn label_text(label: &Value) -> String {
    let integrity = if label["integrity"] == "trusted" {
        "T"
    } else {
        "U"
    };
    let confidentiality = if label["confidentiality"] == "public" {
        "pub"
    } else {
        "priv"
    };
    format!("({integrity},{confidentiality})")
}

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
        Event::SlotDeferred {
            slot,
            label,
            origin,
        } => json!({
            "kind": "slot_deferred",
            "slot": slot.as_str(),
            "origin": origin,
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
            "role": role_word(*role),
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

    /// Every kind, so the round trip below covers the whole enum rather than the easy half.
    fn every_kind() -> Vec<Event> {
        vec![
            Event::GatePassed {
                gate: "capability",
                detail: "file_read granted".to_string(),
            },
            Event::GateBlocked {
                gate: "trusted-read",
                detail: "edit_file".to_string(),
                reason: "content is untrusted".to_string(),
            },
            Event::Observed {
                capability: Capability::FileRead,
                label: Label::untrusted_private(),
            },
            Event::SlotWritten {
                slot: SlotId::new("ref:2"),
                label: Label::untrusted_private(),
            },
            Event::Declassified {
                slot: SlotId::new("ref:3"),
                from: Label::untrusted_private(),
                to: Label::untrusted_public(),
                reason: "shown to the user",
            },
            Event::ActionField {
                tool: "write_file".to_string(),
                field: "path".to_string(),
                role: Role::Routing,
                label: Label::trusted_public(),
                allowed: true,
            },
            Event::ActionField {
                tool: "fetch".to_string(),
                field: "url".to_string(),
                role: Role::Routing,
                label: Label::untrusted_public(),
                allowed: false,
            },
        ]
    }

    /// A trail read back off disk must read exactly as it did when it happened. Two spellings of
    /// one line is the failure this guards: the file and the screen would drift apart the moment
    /// either was reworded, and nobody would notice until they compared a resumed session with a
    /// live one.
    #[test]
    fn a_stored_event_reads_back_as_the_line_it_was() {
        for event in every_kind() {
            assert_eq!(
                recalled(&as_json(&event)),
                Some(as_line(&event)),
                "{event:?} did not come back as the line it was shown as"
            );
        }
    }

    /// A refusal must still be a refusal after the round trip, since that is what colours it red
    /// and a refusal drawn as an ordinary line is the one thing the trail exists to make loud.
    #[test]
    fn a_refusal_is_still_a_refusal_when_it_is_read_back() {
        let blocked = as_json(&Event::GateBlocked {
            gate: "action",
            detail: String::new(),
            reason: "injection blocked".to_string(),
        });
        let line = recalled(&blocked).expect("a refusal reads back");
        assert!(line.blocked);
        assert!(line.text.contains("injection blocked"));

        let refused_field = as_json(&Event::ActionField {
            tool: "fetch".to_string(),
            field: "url".to_string(),
            role: Role::Routing,
            label: Label::untrusted_public(),
            allowed: false,
        });
        assert!(
            recalled(&refused_field)
                .expect("a field reads back")
                .blocked
        );
    }

    /// A line from a newer build is left out rather than drawn as a mangled one: an audit that
    /// cannot be read faithfully should say less, not say it wrong.
    #[test]
    fn an_event_this_build_does_not_know_is_left_out() {
        assert_eq!(recalled(&json!({"kind": "invented_later"})), None);
        assert_eq!(recalled(&json!({"gate": "capability"})), None);
        assert_eq!(recalled(&Value::Null), None);
    }

    /// A truncated line must not become a line claiming something happened that did not. The last
    /// line of an audit is exactly the one a killed session leaves half-written.
    #[test]
    fn a_half_written_event_is_left_out() {
        assert_eq!(recalled(&json!({"kind": "gate_passed"})), None);
        assert_eq!(
            recalled(&json!({"kind": "gate_blocked", "gate": "action"})),
            None
        );
    }

    /// A verdict nobody can read is drawn as a refusal. A check whose answer is missing is not
    /// one to show as though it passed.
    #[test]
    fn a_field_check_with_no_verdict_reads_as_refused() {
        let line = recalled(&json!({
            "kind": "action_field",
            "tool": "write_file",
            "field": "path",
            "role": "routing",
            "label": {"integrity": "trusted", "confidentiality": "public"},
        }))
        .expect("a field with no verdict still reads back");
        assert!(line.blocked);
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
