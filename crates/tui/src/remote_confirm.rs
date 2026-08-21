//! Talking to the main thread from a worker.
//!
//! A turn runs off the main thread so the indicator keeps animating while the model is slow.
//! But only the main thread owns the terminal, so the turn can neither draw an approval prompt
//! nor update a display itself. Both travel over one channel, because the main thread waits on
//! exactly one thing and `mpsc` has no way to select across two.
//!
//! The two messages behave oppositely, and the difference is consent:
//!
//! - A write **asks**. The worker blocks until an answer arrives, which is what a write must do
//!   anyway, and every failure resolves to refusal: a channel that cannot carry the question
//!   cannot carry consent either.
//! - Progress **announces**. There is no reply to wait for and nothing to refuse, so a listener
//!   that has gone away is simply not drawing. Failing a turn because nobody was watching would
//!   let the display outrank the work.

use bua_agent::confirm::{Confirmer, Decision, WriteRequest};
use bua_agent::report::Reporter;
use bua_core::todo::Row;
use std::sync::mpsc::{Receiver, Sender};

/// What a worker sends the main thread.
#[derive(Debug)]
pub enum ToMain {
    /// A write needs approval. The main thread must reply.
    Write(WriteRequest),
    /// The task list changed. No reply.
    Todos(Vec<Row>),
}

/// The worker's end for questions: sends a write, waits for the answer.
pub struct RemoteConfirmer {
    outbound: Sender<ToMain>,
    answers: Receiver<Decision>,
}

impl RemoteConfirmer {
    pub fn new(outbound: Sender<ToMain>, answers: Receiver<Decision>) -> Self {
        Self { outbound, answers }
    }
}

impl Confirmer for RemoteConfirmer {
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision {
        // A channel that cannot carry the question cannot carry consent either.
        if self.outbound.send(ToMain::Write(request.clone())).is_err() {
            return Decision::Reject;
        }
        // Blocks until the main thread answers, which is what a write must wait for.
        self.answers.recv().unwrap_or(Decision::Reject)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use bua_agent::confirm::Intent;
    use bua_core::todo::{Item, List, Status, rows};
    use std::sync::mpsc::channel;
    use std::thread;

    fn request() -> WriteRequest {
        WriteRequest {
            path: "notes.md".into(),
            contents: "body\n".into(),
            existing: None,
            intent: Intent::Create,
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
            answer_tx.send(Decision::Approve).expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Approve);
        responder.join().expect("responder finished");
    }

    #[test]
    fn a_refusal_travels_back_too() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel();

        thread::spawn(move || {
            inbound.recv().expect("a message arrived");
            answer_tx.send(Decision::Reject).expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
    }

    /// An interface that has gone away cannot consent, so the write is refused rather than
    /// applied unseen.
    #[test]
    fn a_closed_channel_refuses_a_write() {
        let (outbound, inbound) = channel::<ToMain>();
        let (_answer_tx, answer_rx) = channel::<Decision>();
        drop(inbound);

        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
    }

    /// And an answer channel that closes without replying is a refusal, not a hang.
    #[test]
    fn a_dropped_answer_channel_refuses() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel::<Decision>();

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
                answer_tx.send(decision).expect("answered");
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
    }

    /// Both handles share one channel, and a write still gets its answer with reports interleaved.
    #[test]
    fn questions_and_reports_share_the_channel() {
        let (outbound, inbound) = channel::<ToMain>();
        let (answer_tx, answer_rx) = channel::<Decision>();

        let mut reporter = RemoteReporter::new(outbound.clone());
        let mut confirmer = RemoteConfirmer::new(outbound, answer_rx);

        let responder = thread::spawn(move || {
            let mut seen = Vec::new();
            while let Ok(message) = inbound.recv() {
                match message {
                    ToMain::Todos(_) => seen.push("todos"),
                    ToMain::Write(_) => {
                        seen.push("write");
                        answer_tx.send(Decision::Approve).expect("answered");
                        return seen;
                    }
                }
            }
            seen
        });

        reporter.todos(rows(&List::new(vec![Item::new("step", Status::Active)])));
        assert_eq!(confirmer.confirm_write(&request()), Decision::Approve);
        assert_eq!(responder.join().expect("finished"), vec!["todos", "write"]);
    }
}
