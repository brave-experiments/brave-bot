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

/// The most questions one call may put to a person.
///
/// A limit rather than a target. Past a handful the person is being interviewed rather than
/// consulted, and a planner that needs more than this has not finished thinking.
pub const MOST_AT_ONCE: usize = 4;

/// The questions, as the model asked them.
///
/// A series rather than a loose vector so there is one value to gate, one value to shape, and
/// one value to report against. Nothing may ask half of it: see [`canonical_series`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Series {
    pub questions: Vec<Question>,
}

impl Series {
    pub fn new(questions: Vec<Question>) -> Self {
        Self { questions }
    }

    pub fn len(&self) -> usize {
        self.questions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }
}

/// A whole series, released for display.
///
/// One value rather than loose prompts, for the reason [`Prompt`] is one value: the interface
/// receives everything it needs to draw in a single declassification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Asking {
    pub prompts: Vec<Prompt>,
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

/// One field, written so its own bytes cannot be read as the structure around them.
///
/// The length is the whole of what makes the encoding injective. A reader takes exactly that
/// many bytes, so a `: ` inside a label is text rather than a boundary, and no arrangement of
/// fields can spell the string another arrangement spells. Written in bytes rather than
/// characters because bytes are what a reader would count.
fn field(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push(' ');
    out.push_str(&value.len().to_string());
    out.push(':');
    out.push_str(value);
}

/// One stable string standing for the whole question.
///
/// Used as the value the routing gate checks and, through [`Prompt::key`], as the key for
/// remembering an answer. Includes every field that changes what the person is asked, the tag
/// among them, so two questions that differ anywhere are different keys and the second is put to
/// them rather than answered from memory.
///
/// Every field is length-prefixed, and the choices are counted before they are listed, so the
/// question can be read back out of the string and only one question can produce it. A plain
/// concatenation could not: a tag of `Deploy` with a sentence of `Region: which one?` spelled
/// exactly what a tag of `Deploy: Region` with a sentence of `which one?` spelled, and the
/// second question was then answered from the first's memory without ever being shown. The
/// text of each field is still there in the clear, which is what keeps the gated value legible
/// in the trail.
pub fn canonical(question: &Question) -> String {
    let mut out = String::new();
    out.push_str(if question.multiple {
        "pick any"
    } else {
        "pick one"
    });
    out.push('\n');
    field(&mut out, "tag", &question.header);
    out.push('\n');
    field(&mut out, "ask", &question.prompt);
    out.push_str(&format!("\nchoices {}", question.choices.len()));
    for choice in &question.choices {
        out.push('\n');
        field(&mut out, "choice", &choice.label);
        // Absent and empty are different answers to "is there a detail", and a key that
        // confused them would be two questions again.
        if let Some(detail) = &choice.detail {
            out.push(' ');
            field(&mut out, "detail", detail);
        }
    }
    out
}

/// Shape a whole series for display.
///
/// Every question produces exactly one prompt, in order, for the same reason every choice
/// produces exactly one row: what the model wrote must not be able to decide which questions
/// the person is shown the existence of.
pub fn asking(series: &Series) -> Asking {
    Asking {
        prompts: series.questions.iter().map(prompt).collect(),
    }
}

/// One stable string standing for the whole series.
///
/// This is the value the routing gate checks, and it covers every question, because the gate
/// runs once for the call. Gating question by question would mean deciding, per question,
/// whether that one is put to the person, and which half of a series survives is exactly the
/// sort of decision that must not be derived from what is in it. A series is asked whole or
/// refused whole.
///
/// The count is part of the string, so two different ways of splitting the same questions are
/// two different series and neither can be answered from the other's memory. The position is
/// there to make the gated value legible in the audit trail and does no such work: the questions
/// are already concatenated in order, so a reordering changes the string with or without it.
pub fn canonical_series(series: &Series) -> String {
    let mut out = format!("asking {} questions", series.len());
    for (at, question) in series.questions.iter().enumerate() {
        out.push_str(&format!("\nquestion {} of {}\n", at + 1, series.len()));
        out.push_str(&canonical(question));
    }
    out
}

/// Put the replies to a whole series into words for the planner.
///
/// Walks the questions, never the answers, so the report has one paragraph per question the
/// model asked and the interface cannot add one. Pairing is by position, and it is total:
///
/// - a question with no answer at its index reads as a decline, which is the honest report of
///   an interface that did not answer it
/// - an answer past the last question is dropped, since there is no question it could be about
///   and attributing it to one would report the person as having said something about a
///   question nobody asked
///
/// With more than one answer in one result the planner cannot tell which question each settled,
/// so each is named. A series of one needs no such heading and gets none.
pub fn describe_series(series: &Series, answers: &[Answer]) -> String {
    let lone = series.len() == 1;
    series
        .questions
        .iter()
        .enumerate()
        .map(|(at, question)| {
            let answer = answers.get(at).unwrap_or(&Answer::Declined);
            let said = describe(question, answer);
            if lone {
                said
            } else {
                format!("{}\n{said}", question.prompt)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Put a reply into words for the planner.
///
/// Lives here so the driver never branches on what the person said. An index that names no
/// choice is dropped rather than guessed at: the alternative is reporting an answer nobody gave.
/// A selection that ends up empty reads as a decline, since that is what it is.
fn describe(question: &Question, answer: &Answer) -> String {
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

    /// The bug the length prefixes exist for. Two questions that differ in their tag and their
    /// sentence used to spell the same key, and the second was then filtered out of the
    /// outstanding set and reported to the planner as answered with the first's answer, without
    /// the person ever seeing it. Nothing a question can contain may spell another question.
    #[test]
    fn no_question_can_spell_the_key_of_another() {
        let split_early = Question::new(
            "Deploy",
            "Region: which one?",
            vec![Choice::new("us-east", None)],
            false,
        );
        let split_late = Question::new(
            "Deploy: Region",
            "which one?",
            vec![Choice::new("us-east", None)],
            false,
        );
        assert_ne!(
            canonical(&split_early),
            canonical(&split_late),
            "the tag and the sentence ran together"
        );

        let separator_in_label = Question::new(
            "Deploy",
            "Which?",
            vec![Choice::new("us-east: the eastern one", None)],
            false,
        );
        let label_and_detail = Question::new(
            "Deploy",
            "Which?",
            vec![Choice::new("us-east", Some("the eastern one".into()))],
            false,
        );
        assert_ne!(
            canonical(&separator_in_label),
            canonical(&label_and_detail),
            "a label carrying a separator read as a label plus a detail"
        );

        let newline_in_label = Question::new(
            "Deploy",
            "Which?",
            vec![Choice::new("us-east\n- us-west", None)],
            false,
        );
        let two_choices = Question::new(
            "Deploy",
            "Which?",
            vec![Choice::new("us-east", None), Choice::new("us-west", None)],
            false,
        );
        assert_ne!(
            canonical(&newline_in_label),
            canonical(&two_choices),
            "one label read as two choices"
        );
    }

    /// A detail nobody wrote and a detail written empty are different questions, and the one
    /// place an encoding of an optional field usually loses the difference.
    #[test]
    fn a_choice_with_no_detail_differs_from_one_with_an_empty_detail() {
        let bare = Question::new(
            "Deploy",
            "Which?",
            vec![Choice::new("us-east", None)],
            false,
        );
        let empty = Question::new(
            "Deploy",
            "Which?",
            vec![Choice::new("us-east", Some(String::new()))],
            false,
        );
        assert_ne!(canonical(&bare), canonical(&empty));
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

    fn platforms() -> Question {
        Question::new(
            "Platforms",
            "Which platforms?",
            vec![Choice::new("Linux", None), Choice::new("macOS", None)],
            true,
        )
    }

    fn series() -> Series {
        Series::new(vec![question(), platforms()])
    }

    /// The same rule as an option list, one level up. A series that could lose a question on the
    /// way to the screen would let what the model wrote decide what the person is asked.
    #[test]
    fn shaping_a_series_never_drops_a_question() {
        let many = Series::new(
            (0..20)
                .map(|i| Question::new(format!("{i}"), format!("question {i}?"), Vec::new(), false))
                .collect(),
        );
        let shaped = asking(&many);
        assert_eq!(shaped.prompts.len(), 20);
        assert!(
            shaped
                .prompts
                .iter()
                .enumerate()
                .all(|(i, p)| p.question == format!("question {i}?")),
            "the questions did not keep their order"
        );
    }

    /// Each prompt keeps its own key beside the series key, because the two answer different
    /// questions: the series key is what the gate checks, and the per-question key is what an
    /// interface remembers an answer under.
    #[test]
    fn every_shaped_question_keeps_its_own_key() {
        let shaped = asking(&series());
        assert_eq!(shaped.prompts[0].key, canonical(&question()));
        assert_eq!(shaped.prompts[1].key, canonical(&platforms()));
    }

    /// The gate runs once, so the value it checks has to cover everything the person will be
    /// shown. A key that missed a question would let that question through ungated.
    #[test]
    fn a_series_key_covers_every_question_in_it() {
        let key = canonical_series(&series());
        assert!(key.contains("Which cache layer?"), "{key}");
        assert!(key.contains("Which platforms?"), "{key}");
        assert!(key.contains("macOS"), "{key}");
    }

    /// Order is part of what the person is asked, since they answer one question at a time and
    /// each answer is given knowing the ones before it.
    #[test]
    fn two_series_holding_the_same_questions_in_a_different_order_have_different_keys() {
        let forwards = Series::new(vec![question(), platforms()]);
        let backwards = Series::new(vec![platforms(), question()]);
        assert_ne!(
            canonical_series(&forwards),
            canonical_series(&backwards),
            "reordering a series left it looking like the same series"
        );
    }

    /// Splitting three questions into two calls must not produce a key one of them already has,
    /// or the second would be answered from the first's memory.
    #[test]
    fn a_series_key_says_how_many_questions_are_in_it() {
        let alone = Series::new(vec![question()]);
        assert!(!canonical_series(&alone).contains(&canonical_series(&series())));
        assert_ne!(canonical_series(&alone), canonical(&question()));
    }

    /// With several answers in one result, an unlabelled reply leaves the planner to guess which
    /// question it settled.
    #[test]
    fn a_series_is_reported_question_by_question() {
        let text = describe_series(
            &series(),
            &[Answer::Chosen(vec![1]), Answer::Chosen(vec![0, 1])],
        );
        assert_eq!(
            text,
            "Which cache layer?\nThe user chose: Query\n\n\
             Which platforms?\nThe user chose:\n- Linux\n- macOS"
        );
    }

    /// A lone question cannot be confused with another, so naming it is noise.
    #[test]
    fn a_lone_question_is_reported_without_repeating_itself() {
        let alone = Series::new(vec![question()]);
        assert_eq!(
            describe_series(&alone, &[Answer::Chosen(vec![0])]),
            "The user chose: HTTP"
        );
    }

    /// The property skipping rests on. A question the person passed over must not cost them the
    /// answers they did give.
    #[test]
    fn a_skipped_question_is_reported_as_declined_beside_its_answered_siblings() {
        let text = describe_series(&series(), &[Answer::Declined, Answer::Chosen(vec![0])]);
        assert!(
            text.contains("Which cache layer?\nThe user declined"),
            "{text}"
        );
        assert!(
            text.contains("Which platforms?\nThe user chose: Linux"),
            "{text}"
        );
    }

    /// An interface that answered fewer questions than were asked did not answer the rest, and
    /// saying so is better than shifting the answers it did give onto the wrong questions.
    #[test]
    fn a_question_with_no_answer_at_all_reads_as_a_decline() {
        let text = describe_series(&series(), &[Answer::Chosen(vec![0])]);
        assert!(
            text.contains("Which cache layer?\nThe user chose: HTTP"),
            "{text}"
        );
        assert!(
            text.contains("Which platforms?\nThe user declined"),
            "{text}"
        );
    }

    /// And an answer past the end names no question, so reporting it would attribute words to
    /// the person about something nobody asked.
    #[test]
    fn an_answer_with_no_question_is_dropped_rather_than_attributed_to_one() {
        let alone = Series::new(vec![question()]);
        assert_eq!(
            describe_series(
                &alone,
                &[Answer::Chosen(vec![0]), Answer::Typed("hello".into())]
            ),
            "The user chose: HTTP"
        );
    }
}
