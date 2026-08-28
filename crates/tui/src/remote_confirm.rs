//! Talking to the main thread from a worker.
//!
//! A turn runs off the main thread so the indicator keeps animating while the model is slow.
//! But only the main thread owns the terminal, so the turn can neither draw an approval prompt
//! nor update a display itself. Both travel over one channel, because the main thread waits on
//! exactly one thing and `mpsc` has no way to select across two.
//!
//! The messages behave in two ways, and the difference is consent:
//!
//! - A write, a run, and a question **ask**. The worker blocks until an answer arrives, and every
//!   failure resolves to the negative one: a channel that cannot carry the question cannot carry
//!   consent either, and a reply that never came is not an answer to report as the user's.
//! - Progress **announces**. There is no reply to wait for and nothing to refuse, so a listener
//!   that has gone away is simply not drawing. Failing a turn because nobody was watching would
//!   let the display outrank the work.
//!
//! Replies come back over one channel too, tagged with what they answer. The worker asks one
//! thing at a time and blocks, so a reply of the wrong kind means the two ends have lost step
//! with each other; that resolves to the negative answer rather than to a retry, because a
//! decision taken against a question nobody matched is worse than no decision at all.

use bravebot_agent::confirm::{
    Confirmer, Decision, OutputRequest, RunDecision, RunRequest, VouchRequest, WriteRequest,
};
use bravebot_agent::report::{Activity, Landing, Phase, Reporter, Shown};
use bravebot_core::ask::{Answer, Asking};
use bravebot_core::todo::Row;
use std::sync::mpsc::{Receiver, Sender};

/// What a worker sends the main thread.
#[derive(Debug)]
pub enum ToMain {
    /// A write needs approval. The main thread must reply.
    Write(WriteRequest),
    /// A pipeline needs approval before it runs. The main thread must reply.
    Run(RunRequest),
    /// A command's output needs a person to read it before the planner may. The main thread
    /// must reply.
    ReadOutput(OutputRequest),
    /// A quarantined file the model would like to read. The main thread must reply.
    Vouch(VouchRequest),
    /// The task list changed. No reply.
    /// The planner is asking the user something. The main thread must reply.
    Ask(Asking),
    Todos(Vec<Row>),
    /// The model has written this many output tokens so far. No reply.
    Written(u64),
    /// The turn is waiting on the model again. No reply.
    Phase(Phase),
    /// The model said something between tool calls. No reply.
    Narration(String),
    /// A tool call has begun. No reply.
    Started(Activity),
    /// The tool call last announced has finished. No reply.
    Finished(Activity),
    /// Quarantined content, for the person watching to read. No reply.
    Quarantined(Shown),
    /// Where the result of the last call ended up. No reply.
    Landed(Landing),
}

/// What the main thread sends back, tagged with what it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Write(Decision),
    Run(RunDecision),
    ReadOutput(Decision),
    Vouch(Decision),
    Ask(Vec<Answer>),
}

/// The worker's end for questions: sends one, waits for the answer.
pub struct RemoteConfirmer {
    outbound: Sender<ToMain>,
    answers: Receiver<Reply>,
}

impl RemoteConfirmer {
    pub fn new(outbound: Sender<ToMain>, answers: Receiver<Reply>) -> Self {
        Self { outbound, answers }
    }

    /// Send a question and block for its reply.
    fn exchange(&mut self, message: ToMain) -> Option<Reply> {
        // A channel that cannot carry the question cannot carry consent either.
        self.outbound.send(message).ok()?;
        self.answers.recv().ok()
    }
}

impl Confirmer for RemoteConfirmer {
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision {
        match self.exchange(ToMain::Write(request.clone())) {
            Some(Reply::Write(decision)) => decision,
            // No reply, or a reply to something else. Neither is consent.
            _ => Decision::Reject,
        }
    }

    fn confirm_run(&mut self, request: &RunRequest) -> RunDecision {
        match self.exchange(ToMain::Run(request.clone())) {
            Some(Reply::Run(decision)) => decision,
            // A reply tagged as answering the write question is not an answer to this one, and
            // running a program on it would be acting on consent nobody gave.
            _ => RunDecision::reject(),
        }
    }

    fn confirm_read_output(&mut self, request: &OutputRequest) -> Decision {
        match self.exchange(ToMain::ReadOutput(request.clone())) {
            Some(Reply::ReadOutput(decision)) => decision,
            // A reply to a different question is not consent to put these bytes in the planner's
            // context.
            _ => Decision::Reject,
        }
    }

    fn confirm_vouch(&mut self, request: &VouchRequest) -> Decision {
        match self.exchange(ToMain::Vouch(request.clone())) {
            Some(Reply::Vouch(decision)) => decision,
            _ => Decision::Reject,
        }
    }

    fn ask_user(&mut self, asking: &Asking) -> Vec<Answer> {
        match self.exchange(ToMain::Ask(asking.clone())) {
            Some(Reply::Ask(answers)) => answers,
            _ => Vec::new(),
        }
    }
}

/// The worker's end for progress: sends and moves on.
///
/// A separate handle from [`RemoteConfirmer`] over a clone of the same sender, because a turn holds
/// both at once and one object cannot be borrowed mutably twice. They stay distinct types for a
/// better reason than that, though: nothing that only announces should be able to answer a
/// question about a write.
pub struct RemoteReporter {
    outbound: Sender<ToMain>,
}

impl RemoteReporter {
    pub fn new(outbound: Sender<ToMain>) -> Self {
        Self { outbound }
    }
}

impl Reporter for RemoteReporter {
    fn todos(&mut self, rows: Vec<Row>) {
        // Deliberately ignored. Unlike a write, there is no decision resting on this arriving,
        // so a closed channel means the display is gone, not that the turn should stop.
        let _ = self.outbound.send(ToMain::Todos(rows));
    }

    fn output_tokens(&mut self, written: u64) {
        let _ = self.outbound.send(ToMain::Written(written));
    }

    fn phase(&mut self, phase: Phase) {
        let _ = self.outbound.send(ToMain::Phase(phase));
    }

    fn narration(&mut self, text: String) {
        let _ = self.outbound.send(ToMain::Narration(text));
    }

    fn tool_started(&mut self, activity: Activity) {
        let _ = self.outbound.send(ToMain::Started(activity));
    }

    fn tool_finished(&mut self, activity: Activity) {
        let _ = self.outbound.send(ToMain::Finished(activity));
    }
    fn quarantined(&mut self, shown: Shown) {
        let _ = self.outbound.send(ToMain::Quarantined(shown));
    }

    fn landed(&mut self, landing: Landing) {
        let _ = self.outbound.send(ToMain::Landed(landing));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_agent::confirm::Intent;
    use bravebot_core::todo::{Item, List, Status, rows};
    use std::sync::mpsc::channel;
    use std::thread;

    fn request() -> WriteRequest {
        WriteRequest {
            path: "notes.md".into(),
            contents: "body\n".into(),
            existing: None,
            intent: Intent::Create,
            untrusted: false,
        }
    }

    /// The question reaches the other side and the answer comes back.
    #[test]
    fn an_answer_travels_back_to_the_worker() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel();

        let responder = thread::spawn(move || {
            match inbound.recv().expect("a message arrived") {
                ToMain::Write(asked) => assert_eq!(asked.path, "notes.md"),
                other => panic!("expected a write question, got {other:?}"),
            }
            answer_tx
                .send(Reply::Write(Decision::Approve))
                .expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Approve);
        responder.join().expect("responder finished");
    }

    fn a_run() -> RunRequest {
        RunRequest {
            pipeline: bravebot_core::Pipeline::new(vec![bravebot_core::Stage::new(
                "git",
                vec!["log".into()],
            )]),
            resolved: vec!["/usr/bin/git".into()],
            directory: "/tmp/project".into(),
        }
    }

    /// The run question reaches the other side and the answer comes back.
    #[test]
    fn a_run_answer_travels_back_to_the_worker() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel();

        let responder = thread::spawn(move || {
            match inbound.recv().expect("a message arrived") {
                ToMain::Run(asked) => assert_eq!(asked.pipeline.len(), 1),
                other => panic!("expected a run question, got {other:?}"),
            }
            answer_tx
                .send(Reply::Run(RunDecision::approve_always()))
                .expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        let answer = confirmer.confirm_run(&a_run());
        assert!(answer.approved());
        assert!(
            answer.remember,
            "the standing permission was lost in transit"
        );
        responder.join().expect("responder finished");
    }

    /// An approval for a write is not an approval to run a program. The two questions are
    /// different, so a reply tagged as answering one must not settle the other.
    #[test]
    fn an_approved_write_does_not_approve_a_run() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel();

        let responder = thread::spawn(move || {
            inbound.recv().expect("a message arrived");
            answer_tx
                .send(Reply::Write(Decision::Approve))
                .expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        let answer = confirmer.confirm_run(&a_run());
        assert!(
            !answer.approved(),
            "consent to a write was taken as consent to run a program"
        );
        assert!(!answer.remember);
        responder.join().expect("responder finished");
    }

    /// Nobody is there to ask, so nothing runs.
    #[test]
    fn a_closed_channel_refuses_a_run() {
        let (outbound, inbound) = channel::<ToMain>();
        let (_answer_tx, answer_rx) = channel::<Reply>();
        drop(inbound);

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        let answer = confirmer.confirm_run(&a_run());
        assert!(!answer.approved());
        assert!(
            !answer.remember,
            "a standing permission was inferred from a channel nobody answered"
        );
    }

    fn a_series() -> Asking {
        bravebot_core::ask::asking(&bravebot_core::ask::Series::new(vec![
            bravebot_core::ask::Question::new(
                "Cache",
                "Which cache layer?",
                vec![bravebot_core::ask::Choice::new("HTTP", None)],
                false,
            ),
        ]))
    }

    #[test]
    fn every_answer_in_a_series_travels_back_to_the_worker() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel();

        let responder = thread::spawn(move || {
            match inbound.recv().expect("a message arrived") {
                ToMain::Ask(asked) => assert_eq!(asked.prompts.len(), 1),
                other => panic!("expected a question, got {other:?}"),
            }
            answer_tx
                .send(Reply::Ask(vec![Answer::Chosen(vec![0]), Answer::Declined]))
                .expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert_eq!(
            confirmer.ask_user(&a_series()),
            vec![Answer::Chosen(vec![0]), Answer::Declined]
        );
        responder.join().expect("responder finished");
    }

    /// An interface that has gone away cannot be asked, so nothing is reported as its answer.
    #[test]
    fn a_closed_channel_answers_no_question() {
        let (outbound, inbound) = channel::<ToMain>();
        let (_answer_tx, answer_rx) = channel::<Reply>();
        drop(inbound);

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert!(confirmer.ask_user(&a_series()).is_empty());
    }

    /// And an answer channel that closes without replying answers nothing, rather than hanging.
    #[test]
    fn a_dropped_answer_channel_answers_no_question() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel::<Reply>();
        thread::spawn(move || {
            inbound.recv().expect("a message arrived");
            drop(answer_tx);
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert!(confirmer.ask_user(&a_series()).is_empty());
    }

    /// The two ends ask one thing at a time, so a reply of the wrong kind means they have lost
    /// step with each other. Taking it as the answer would report a decision against a question
    /// nobody matched.
    #[test]
    fn a_write_approval_is_not_taken_as_an_answer_to_a_question() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel::<Reply>();
        thread::spawn(move || {
            inbound.recv().expect("a message arrived");
            answer_tx
                .send(Reply::Write(Decision::Approve))
                .expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert!(confirmer.ask_user(&a_series()).is_empty());
    }

    /// And the other way round: an answer to a question is not consent to a write.
    #[test]
    fn an_answer_to_a_question_is_not_taken_as_consent_to_a_write() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel::<Reply>();
        thread::spawn(move || {
            inbound.recv().expect("a message arrived");
            answer_tx
                .send(Reply::Ask(vec![Answer::Chosen(vec![0])]))
                .expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
    }

    #[test]
    fn a_refusal_travels_back_too() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel();

        thread::spawn(move || {
            inbound.recv().expect("a message arrived");
            answer_tx
                .send(Reply::Write(Decision::Reject))
                .expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
    }

    /// An interface that has gone away cannot consent, so the write is refused rather than
    /// applied unseen.
    #[test]
    fn a_closed_channel_refuses_a_write() {
        let (outbound, inbound) = channel::<ToMain>();
        let (_answer_tx, answer_rx) = channel::<Reply>();
        drop(inbound);

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
    }

    /// And an answer channel that closes without replying is a refusal, not a hang.
    #[test]
    fn a_dropped_answer_channel_refuses() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel::<Reply>();

        thread::spawn(move || {
            inbound.recv().expect("a message arrived");
            // Goes away without answering, as it would if the interface exited.
            drop(answer_tx);
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
    }

    /// Several writes in one turn are answered independently and in order.
    #[test]
    fn each_write_gets_its_own_answer() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel();

        thread::spawn(move || {
            for decision in [Decision::Approve, Decision::Reject, Decision::Approve] {
                inbound.recv().expect("a message arrived");
                answer_tx.send(Reply::Write(decision)).expect("answered");
            }
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Approve);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Approve);
    }

    #[test]
    fn a_task_list_travels_without_an_answer() {
        let (outbound, inbound) = channel::<ToMain>();

        let mut reporter = RemoteReporter::new(outbound);
        reporter.todos(rows(&List::new(vec![Item::new("step", Status::Active)])));

        match inbound.recv().expect("a message arrived") {
            ToMain::Todos(rows) => assert_eq!(rows[0].content, "step"),
            other => panic!("expected a task list, got {other:?}"),
        }
    }

    /// The asymmetry that matters: nobody watching is not a failure. A write refuses when the
    /// channel is gone, but a report has no answer to withhold and must not block or panic.
    #[test]
    fn a_closed_channel_drops_a_task_list_without_failing() {
        let (outbound, inbound) = channel::<ToMain>();
        drop(inbound);

        let mut reporter = RemoteReporter::new(outbound);
        reporter.todos(rows(&List::new(vec![Item::new("step", Status::Done)])));
        reporter.phase(Phase::Thinking);
        reporter.narration("nobody is listening".into());
        reporter.tool_started(Activity::running("Read", "a.rs"));
        reporter.tool_finished(Activity::running("Read", "a.rs").done("1 line"));
    }

    /// Both handles share one channel, and a write still gets its answer with reports interleaved.
    #[test]
    fn questions_and_reports_share_the_channel() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel::<Reply>();

        let mut reporter = RemoteReporter::new(outbound.clone());
        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);

        let responder = thread::spawn(move || {
            let mut seen = Vec::new();
            while let Ok(message) = inbound.recv() {
                match message {
                    ToMain::Ask(_) => seen.push("ask"),
                    ToMain::Run(_) => seen.push("run"),
                    ToMain::ReadOutput(_) => seen.push("read_output"),
                    ToMain::Vouch(_) => seen.push("vouch"),
                    ToMain::Todos(_) => seen.push("todos"),
                    ToMain::Written(_) => seen.push("written"),
                    ToMain::Phase(_) => seen.push("phase"),
                    ToMain::Narration(_) => seen.push("narration"),
                    ToMain::Started(_) => seen.push("started"),
                    ToMain::Finished(_) => seen.push("finished"),
                    ToMain::Quarantined(_) => seen.push("quarantined"),
                    ToMain::Landed(_) => seen.push("landed"),
                    ToMain::Write(_) => {
                        seen.push("write");
                        answer_tx
                            .send(Reply::Write(Decision::Approve))
                            .expect("answered");
                        return seen;
                    }
                }
            }
            seen
        });

        reporter.todos(rows(&List::new(vec![Item::new("step", Status::Active)])));
        reporter.output_tokens(42);
        reporter.phase(Phase::Planning);
        reporter.narration("about to write".into());
        reporter.tool_started(Activity::running("Write", "notes.md"));
        reporter.tool_finished(Activity::running("Write", "notes.md").done("1 line"));
        assert_eq!(confirmer.confirm_write(&request()), Decision::Approve);
        assert_eq!(
            responder.join().expect("finished"),
            vec![
                "todos",
                "written",
                "phase",
                "narration",
                "started",
                "finished",
                "write"
            ]
        );
    }

    /// A count travels with no reply, like a task list: there is nothing to answer.
    #[test]
    fn a_written_count_travels_without_an_answer() {
        let (outbound, inbound) = channel::<ToMain>();

        let mut reporter = RemoteReporter::new(outbound);
        reporter.output_tokens(512);

        match inbound.recv().expect("a message arrived") {
            ToMain::Written(written) => assert_eq!(written, 512),
            other => panic!("expected a count, got {other:?}"),
        }
    }

    /// And a closed channel drops it rather than failing the turn: nobody watching is not an error.
    #[test]
    fn a_closed_channel_drops_a_written_count_without_failing() {
        let (outbound, inbound) = channel::<ToMain>();
        drop(inbound);

        let mut reporter = RemoteReporter::new(outbound);
        reporter.output_tokens(7);
    }
}
