//! Reasoning a model wrote into the reply itself.
//!
//! A model with a channel of its own for its working keeps it out of the reply, and this crate
//! never sees it. A model without one opens the reply with `<think>` instead and closes the
//! block before answering, so the working is the first thing on the screen and the answer is
//! underneath it. Same text either way, and it is not what anybody asked for.
//!
//! Taking it off is a decision read out of the reply, which is why it belongs here. The turn
//! hands every byte to the interface without looking at it, because in a session that has
//! observed something untrusted those bytes are untrusted and the driver may not branch on
//! them. A screen is the one place allowed to ask what the text says.

/// The openings a reply may start with, and what closes each.
///
/// Written out rather than built from names, so the set a reader has to trust is on one screen,
/// and so nothing is allocated to look for one that is not there.
const BLOCKS: [(&str, &str); 3] = [
    ("<think>", "</think>"),
    ("<thinking>", "</thinking>"),
    ("<reasoning>", "</reasoning>"),
];

/// What a finished reply says, with a leading reasoning block taken off.
///
/// Leading only. A tag further down is something the reply is talking about, and an answer
/// explaining what `<think>` means is not thinking out loud.
///
/// An opening whose close never arrived is left alone and drawn whole. That is the shape a reply
/// cut off mid-thought has, and a truncated thought on the screen is better than a blank where
/// the answer should be.
pub fn spoken(text: &str) -> &str {
    match opened(text) {
        Some((rest, close)) => after(rest, close).unwrap_or(text),
        None => text,
    }
}

/// The same, for a reply still being written.
///
/// Only the unclosed case differs. While bytes are still arriving an open block means the model
/// is mid-thought rather than cut off, so nothing is drawn until the close lands and the
/// indicator is what says the turn is alive. A part-written opening is held back for the same
/// reason: `<thi` becomes `<think>` often enough that drawing it and taking it away again reads
/// worse than a frame of nothing.
pub fn spoken_so_far(text: &str) -> &str {
    match opened(text) {
        Some((rest, close)) => after(rest, close).unwrap_or_default(),
        None if opening(text) => "",
        None => text,
    }
}

/// What follows the opening `text` begins with, and the tag that closes it.
fn opened(text: &str) -> Option<(&str, &str)> {
    BLOCKS
        .iter()
        .find_map(|(open, close)| Some((text.trim_start().strip_prefix(open)?, *close)))
}

/// What follows the first `close` in `rest`, or `None` where the block is still open.
fn after<'a>(rest: &'a str, close: &str) -> Option<&'a str> {
    let at = rest.find(close)?;
    Some(rest[at + close.len()..].trim_start())
}

/// Whether `text` is the start of an opening and nothing more, so which it will be is not
/// settled yet.
fn opening(text: &str) -> bool {
    let text = text.trim_start();
    !text.is_empty() && BLOCKS.iter().any(|(open, _)| open.starts_with(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block is the model talking to itself on the way to an answer. Drawn, it buries the
    /// answer under a paragraph nobody asked for and leaves a person scrolling to find out what
    /// the turn concluded.
    #[test]
    fn a_leading_reasoning_block_is_not_drawn() {
        assert_eq!(
            spoken("<think>the user wants the config file</think>It is in ~/.bravebot."),
            "It is in ~/.bravebot."
        );
    }

    /// Three spellings are in circulation and a reply uses whichever its model was trained on.
    #[test]
    fn every_spelling_of_the_block_is_recognised() {
        for (open, close) in BLOCKS {
            assert_eq!(
                spoken(&format!("{open}working{close}\n\nthe answer")),
                "the answer"
            );
        }
    }

    /// The rule is leading only, and this is what it buys: a reply may talk about the tags, quote
    /// one, or show the code that writes them, and none of that is the model reasoning out loud.
    #[test]
    fn a_reply_that_mentions_a_tag_keeps_every_word() {
        let mentioned = "Some models open with <think> and close with </think> before answering.";
        assert_eq!(spoken(mentioned), mentioned);
    }

    /// A reply that ran out of budget mid-thought still has to reach the screen. Swallowing it
    /// would report the turn as having answered nothing, which is a different thing from having
    /// been cut off.
    #[test]
    fn a_thought_that_never_closed_is_drawn_whole() {
        assert_eq!(spoken("<think>the user wants"), "<think>the user wants");
    }

    /// The same text while it is still arriving means the opposite: the model is thinking, not
    /// truncated. Drawing it and taking it back the moment the close lands is the flicker this
    /// exists to avoid.
    #[test]
    fn a_thought_still_being_written_draws_nothing() {
        assert_eq!(spoken_so_far("<think>the user wants"), "");
    }

    /// And the answer appears the moment the block closes, rather than waiting for the round.
    #[test]
    fn the_answer_is_drawn_as_soon_as_the_thought_closes() {
        assert_eq!(
            spoken_so_far("<think>working</think>It is in ~/"),
            "It is in ~/"
        );
    }

    /// A tag arrives a few characters at a time like everything else, so the first frame of one
    /// is not yet a tag. Held back rather than drawn, or every reasoning reply would open with a
    /// stray bracket that vanishes.
    #[test]
    fn a_part_written_opening_is_held_back() {
        for so_far in ["<", "<t", "<thi", "<think", "<reason"] {
            assert_eq!(spoken_so_far(so_far), "", "{so_far} was drawn");
        }
    }

    /// What is held back is only what could still become a tag. A reply that opens with markup
    /// or a comparison is drawn as soon as it is no longer one of the three.
    #[test]
    fn text_that_cannot_become_a_tag_is_drawn_at_once() {
        assert_eq!(spoken_so_far("<html>"), "<html>");
        assert_eq!(spoken_so_far("<- that one"), "<- that one");
        assert_eq!(spoken_so_far("2 < 3"), "2 < 3");
    }

    /// Nothing to draw stays nothing to draw, so an empty tail is not mistaken for an opening.
    #[test]
    fn an_empty_reply_is_still_empty() {
        assert_eq!(spoken_so_far(""), "");
        assert_eq!(spoken(""), "");
    }
}
