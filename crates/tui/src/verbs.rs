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

/// Present participles shown while a turn runs.
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

/// The verb for a turn, chosen by its number.
///
/// Deterministic rather than random: the same turn always shows the same word, so a test can
/// assert on it and a bug report can be reproduced. Cycling by index also guarantees a user
/// sees the whole list before any repeat, which random choice would not.
pub fn for_turn(turn: usize) -> &'static str {
    VERBS[turn % VERBS.len()]
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
