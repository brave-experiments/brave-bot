//! A question the planner puts to the person.
//!
//! A planning step often turns on something only the user knows: which of two approaches, which
//! of three candidate files, whether a migration is in scope. Without a way to ask, the model
//! either guesses or spends its final reply asking, and the turn ends before the answer can be
//! used.
//!
//! The question is **routing**, not content. It decides what the person is shown and therefore
//! what they can answer, so it must be `(T,pub)` like any other address. What makes that
//! workable is that the routing field here is approved by being read: the bytes gated are
//! exactly the bytes drawn, and nothing re-parses them afterwards. There is no effect to endorse
//! beyond the display itself.
//!
//! The consequence is that a planner which has been shown something untrusted cannot ask. That
//! is the correct reading rather than a limitation to route around: a person choosing among
//! strings an attacker wrote does not make those strings trusted, and treating a selection as
//! though it did would launder untrusted bytes into the planner's context through a keypress.
//! A quarantined read is not such a showing, since the planner met a reference and not the
//! bytes, and it leaves the planner free to ask.
//!
//! Rendering lives here rather than in the interface for the same reason it does in
//! [`crate::todo`]: shaping a question means looking at it, and the driver may not. Formatting
//! the answer lives here too, so the driver never has to branch on what the person said.

/// One option the person may pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The option itself, as the model wrote it.
    pub label: String,
    /// An optional line of explanation shown beneath it.
    pub detail: Option<String>,
}

impl Choice {
    pub fn new(label: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            label: label.into(),
            detail,
        }
    }
}

/// A question, as the model asked it.
///
/// A question with no choices is not an error: it is a question that can only be answered in the
/// person's own words. Rejecting one would be a decision made from content, and the reading that
/// hides nothing is to ask it anyway.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Question {
    /// A few words naming what this asks about, drawn as a tag beside the question.
    ///
    /// Its job is to be recognisable at a glance when several questions arrive together: the
    /// sentence says what is being asked, the tag says which of the pending decisions it is.
    pub header: String,
    pub prompt: String,
    pub choices: Vec<Choice>,
    /// Whether more than one choice may be picked.
    pub multiple: bool,
}

impl Question {
    pub fn new(
        header: impl Into<String>,
        prompt: impl Into<String>,
        choices: Vec<Choice>,
        multiple: bool,
    ) -> Self {
        Self {
            header: header.into(),
            prompt: prompt.into(),
            choices,
            multiple,
        }
    }
}

/// What the person did.
///
/// Not a label and not a gate, just the shape of a reply. Declining is a first-class answer
/// rather than an error: a question nobody wants to answer is still answered, and the turn
/// continues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Indices into the question's choices, in the order the person confirmed them.
    Chosen(Vec<usize>),
    /// Something the person typed instead.
    Typed(String),
    /// No answer. The default wherever nobody can be asked.
    Declined,
}

/// One option, shaped for a screen.
///
/// Carries the index as data so the interface can report a selection back without matching on
/// the label text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub index: usize,
    pub label: String,
    pub detail: Option<String>,
}

/// A whole question, released for display.
///
/// One value rather than a question plus loose rows, so the interface receives everything it
/// needs to draw in a single declassification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prompt {
    pub header: String,
    pub question: String,
    pub rows: Vec<Row>,
    pub multiple: bool,
    /// [`canonical`] for this question, carried so the interface can tell two questions apart
    /// without reimplementing the rule for what makes them different.
    pub key: String,
}

/// Shape a question for display.
///
/// Meant to be called inside [`crate::policy::Policy::render_in_place`], which is what lets the
/// question be looked at at all. Every choice produces exactly one row, in order: nothing is
/// filtered, reordered, or truncated, so what the model wrote cannot change which options exist,
/// only how they look.
pub fn prompt(question: &Question) -> Prompt {
    Prompt {
        header: question.header.clone(),
        question: question.prompt.clone(),
        rows: question
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| Row {
                index,
                label: choice.label.clone(),
                detail: choice.detail.clone(),
            })
            .collect(),
        multiple: question.multiple,
        key: canonical(question),
    }
}

/// One stable string standing for the whole question.
///
/// Used as the value the routing gate checks and, through [`Prompt::key`], as the key for
/// remembering an answer. Includes every field that changes what the person is asked, the tag
/// among them, so two questions that differ anywhere are different keys and the second is put to
/// them rather than answered from memory.
pub fn canonical(question: &Question) -> String {
    let mut out = String::new();
    out.push_str(if question.multiple {
        "pick any: "
    } else {
        "pick one: "
    });
    out.push_str(&question.header);
    out.push_str(": ");
    out.push_str(&question.prompt);
    for choice in &question.choices {
        out.push_str("\n- ");
        out.push_str(&choice.label);
        if let Some(detail) = &choice.detail {
            out.push_str(": ");
            out.push_str(detail);
        }
    }
    out
}

/// Put a reply into words for the planner.
///
/// Lives here so the driver never branches on what the person said. An index that names no
/// choice is dropped rather than guessed at: the alternative is reporting an answer nobody gave.
/// A selection that ends up empty reads as a decline, since that is what it is.
pub fn describe(question: &Question, answer: &Answer) -> String {
    match answer {
        Answer::Chosen(indices) => {
            let picked: Vec<&str> = indices
                .iter()
                .filter_map(|i| question.choices.get(*i))
                .map(|c| c.label.as_str())
                .collect();
            match picked.len() {
                0 => declined(),
                1 => format!("The user chose: {}", picked[0]),
                _ => format!(
                    "The user chose:\n{}",
                    picked
                        .iter()
                        .map(|label| format!("- {label}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            }
        }
        Answer::Typed(text) => format!("The user answered: {text}"),
        Answer::Declined => declined(),
    }
}

fn declined() -> String {
    "The user declined to answer. Proceed without it, or say what you need to continue.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question() -> Question {
        Question::new(
            "Cache layer",
            "Which cache layer?",
            vec![
                Choice::new("HTTP", Some("in front of the handler".into())),
                Choice::new("Query", None),
            ],
            false,
        )
    }

    /// A question the model could not supply options for is still worth asking: the person can
    /// answer in their own words. Refusing it would be a decision made from content.
    #[test]
    fn a_question_with_no_options_is_still_a_question() {
        let bare = Question::new("Branch", "Which branch?", Vec::new(), false);
        let shaped = prompt(&bare);
        assert_eq!(shaped.question, "Which branch?");
        assert!(shaped.rows.is_empty());
    }

    /// Rendering must not be able to hide an option. If it could, what the model wrote would be
    /// deciding what the person is allowed to see, which is exactly the decision this design
    /// keeps out of content.
    #[test]
    fn rendering_never_drops_an_option() {
        let many = Question::new(
            "Many",
            "?",
            (0..50).map(|i| Choice::new(format!("{i}"), None)).collect(),
            true,
        );
        let shaped = prompt(&many);
        assert_eq!(shaped.rows.len(), 50);
        assert!(shaped.rows.iter().enumerate().all(|(i, r)| r.index == i));
    }

    /// The tag is drawn, so it has to survive shaping. A question shaped without it would be
    /// drawn without the one thing that tells it apart from its siblings.
    #[test]
    fn a_shaped_question_carries_the_tag_it_is_shown_under() {
        assert_eq!(prompt(&question()).header, "Cache layer");
    }

    #[test]
    fn a_single_choice_is_reported_by_its_label() {
        assert_eq!(
            describe(&question(), &Answer::Chosen(vec![0])),
            "The user chose: HTTP"
        );
    }

    #[test]
    fn several_choices_are_listed() {
        let text = describe(&question(), &Answer::Chosen(vec![1, 0]));
        assert_eq!(text, "The user chose:\n- Query\n- HTTP");
    }

    /// An index naming no option cannot be resolved, and inventing one would report an answer
    /// nobody gave.
    #[test]
    fn a_choice_outside_the_option_list_is_dropped_rather_than_guessed() {
        let text = describe(&question(), &Answer::Chosen(vec![0, 7]));
        assert_eq!(text, "The user chose: HTTP");
    }

    /// And if dropping leaves nothing, the honest report is that no answer was given.
    #[test]
    fn a_selection_that_resolves_to_nothing_reads_as_a_decline() {
        assert_eq!(
            describe(&question(), &Answer::Chosen(vec![9])),
            describe(&question(), &Answer::Declined)
        );
    }

    #[test]
    fn typed_text_is_reported_as_the_users_own_words() {
        assert_eq!(
            describe(&question(), &Answer::Typed("neither".into())),
            "The user answered: neither"
        );
    }

    /// A decline has to leave the planner somewhere to go, or the model will simply ask again.
    #[test]
    fn declining_says_what_to_do_next() {
        assert!(describe(&question(), &Answer::Declined).contains("Proceed without it"));
    }

    /// The key has to separate questions that read the same but ask something different, or the
    /// second would be answered from memory with the first person's answer.
    #[test]
    fn the_key_distinguishes_questions_that_differ_anywhere() {
        let one = question();
        let mut other = question();
        other.multiple = true;
        assert_ne!(canonical(&one), canonical(&other));

        let mut relabelled = question();
        relabelled.choices[0].label = "HTTPS".into();
        assert_ne!(canonical(&one), canonical(&relabelled));

        let mut detailed = question();
        detailed.choices[1].detail = Some("at the database".into());
        assert_ne!(canonical(&one), canonical(&detailed));
    }

    /// The tag is part of what the person is shown, so two questions that differ only there are
    /// still two questions and the second must be put to them.
    #[test]
    fn the_key_distinguishes_questions_that_differ_in_their_tag() {
        let one = question();
        let mut retagged = question();
        retagged.header = "Caching".into();
        assert_ne!(canonical(&one), canonical(&retagged));
    }

    #[test]
    fn the_key_is_stable_for_the_same_question() {
        assert_eq!(canonical(&question()), canonical(&question()));
    }

    /// The interface tells questions apart by the key it was handed, so it has to be the same
    /// rule the gate checked rather than a second one that could drift from it.
    #[test]
    fn a_shaped_question_carries_its_own_key() {
        assert_eq!(prompt(&question()).key, canonical(&question()));
    }
}
