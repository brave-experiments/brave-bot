//! Words for the working indicator.
//!
//! A turn takes as long as it takes, and a spinner that says "working" for ninety seconds
//! reads as a hang. A different odd word each turn makes it obvious the session is alive and
//! that time is passing, without pretending to report progress it cannot measure.
//!
//! The list is deliberately mild. These appear unbidden in a terminal in someone's workplace,
//! so nothing here is crude, cute to the point of grating, or a joke that wears out. Nothing
//! implies a capability the agent lacks either: "Deducing" would suggest reasoning it is not
//! doing, whereas "Percolating" claims nothing.
//!
//! # Why this is not in a catalog
//!
//! Every other word the interface says is a sentence with a meaning to carry across, and the
//! translation is the same sentence in another language. These are not. What has to survive is
//! the tone and the variety, and a word-for-word translation of "Percolating" would keep neither:
//! the joke is that the word is odd, and odd does not translate. A language wants its own list,
//! of its own length, written by somebody who knows which words in it are mild and which are
//! grating, which is a different job from translating a prompt.
//!
//! So a language adds a list here rather than a hundred entries to a catalog, and one that has
//! not is shown the English rather than a machine-translated set of words whose whole purpose is
//! that a person chose them.

/// Present participles shown while a turn runs, in English.
///
/// Kept in one place so the tone stays consistent as entries are added.
pub const VERBS: &[&str] = &[
    "Grooving",
    "Percolating",
    "Marinating",
    "Simmering",
    "Noodling",
    "Puttering",
    "Tinkering",
    "Pondering",
    "Mulling",
    "Ruminating",
    "Cogitating",
    "Wrangling",
    "Finagling",
    "Rummaging",
    "Foraging",
    "Burrowing",
    "Spelunking",
    "Excavating",
    "Prospecting",
    "Divining",
    "Untangling",
    "Unravelling",
    "Unknotting",
    "Threading",
    "Weaving",
    "Splicing",
    "Whittling",
    "Chiselling",
    "Sanding",
    "Polishing",
    "Buffing",
    "Kneading",
    "Folding",
    "Proofing",
    "Steeping",
    "Brewing",
    "Distilling",
    "Fermenting",
    "Reducing",
    "Whisking",
    "Churning",
    "Bubbling",
    "Frothing",
    "Fizzing",
    "Crackling",
    "Humming",
    "Buzzing",
    "Whirring",
    "Clanking",
    "Ratcheting",
    "Cranking",
    "Winding",
    "Cogwheeling",
    "Calibrating",
    "Recalibrating",
    "Tuning",
    "Aligning",
    "Squaring",
    "Plumbing",
    "Surveying",
    "Triangulating",
    "Charting",
    "Mapping",
    "Plotting",
    "Sketching",
    "Doodling",
    "Scribbling",
    "Drafting",
    "Blueprinting",
    "Assembling",
    "Fabricating",
    "Machining",
    "Welding",
    "Soldering",
    "Riveting",
    "Bolting",
    "Scaffolding",
    "Shoring",
    "Buttressing",
    "Trellising",
    "Grafting",
    "Pruning",
    "Weeding",
    "Composting",
    "Germinating",
    "Sprouting",
    "Blossoming",
    "Ripening",
    "Harvesting",
    "Winnowing",
    "Sifting",
    "Sieving",
    "Panning",
    "Dredging",
    "Trawling",
    "Beachcombing",
    "Birdwatching",
    "Stargazing",
    "Moonwalking",
    "Meandering",
    "Ambling",
    "Sauntering",
    "Wandering",
    "Rambling",
    "Traipsing",
    "Galumphing",
    "Sashaying",
    "Waltzing",
    "Jitterbugging",
    "Boogieing",
    "Jamming",
    "Riffing",
    "Improvising",
    "Freestyling",
    "Vamping",
    "Harmonising",
    "Orchestrating",
    "Conducting",
];

/// The list for a language, or the English one where that language has not written its own.
///
/// Keyed on the language rather than the full locale: the words are the same whichever region a
/// speaker is in, which is not true of much else in the interface but is true of these.
fn for_language(_language: &str) -> &'static [&'static str] {
    // English is the only list written so far. A language adds itself here, by matching its
    // subtag and returning its own words, rather than by translating the ones above.
    VERBS
}

/// The verb for a turn, chosen by its number.
///
/// Deterministic rather than random: the same turn always shows the same word, so a test can
/// assert on it and a bug report can be reproduced. Cycling by index also guarantees a user
/// sees the whole list before any repeat, which random choice would not.
pub fn for_turn(turn: usize) -> &'static str {
    let words = for_language(bravebot_i18n::locale().language());
    words[turn % words.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The point of the feature is variety, so a short list would defeat it.
    #[test]
    fn there_are_around_a_hundred_verbs() {
        assert!(
            VERBS.len() >= 100,
            "only {} verbs; the indicator would repeat too soon",
            VERBS.len()
        );
    }

    /// A duplicate would silently reduce the variety the list exists to provide.
    #[test]
    fn every_verb_is_distinct() {
        let unique: BTreeSet<_> = VERBS.iter().collect();
        assert_eq!(unique.len(), VERBS.len(), "the list contains a duplicate");
    }

    /// All of them are used in the same sentence position, so they must share a form.
    #[test]
    fn every_verb_is_a_capitalised_participle() {
        for verb in VERBS {
            assert!(
                verb.ends_with("ing"),
                "{verb} is not a present participle and will read oddly"
            );
            assert!(
                verb.chars().next().is_some_and(char::is_uppercase),
                "{verb} is not capitalised"
            );
            assert!(
                verb.chars().all(|c| c.is_ascii_alphabetic()),
                "{verb} has characters that may not render in every terminal"
            );
        }
    }

    /// Long words push the timing and token counts off a narrow terminal.
    #[test]
    fn no_verb_is_overly_long() {
        for verb in VERBS {
            assert!(
                verb.len() <= 14,
                "{verb} is {} characters, which crowds the status line",
                verb.len()
            );
        }
    }

    /// The same turn must always show the same word, so failures are reproducible.
    #[test]
    fn the_choice_is_deterministic() {
        assert_eq!(for_turn(0), for_turn(0));
        assert_eq!(for_turn(7), for_turn(7));
        assert_ne!(for_turn(0), for_turn(1));
    }

    /// Cycling means the whole list is seen before anything repeats.
    #[test]
    fn the_verbs_cycle_rather_than_repeat_early() {
        let first = for_turn(0);
        for turn in 1..VERBS.len() {
            assert_ne!(for_turn(turn), first, "repeated at turn {turn}");
        }
        assert_eq!(for_turn(VERBS.len()), first, "the cycle did not wrap");
    }
}
