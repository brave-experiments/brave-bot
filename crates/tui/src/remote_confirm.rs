//! Asking about a write from a worker thread.
//!
//! A turn runs off the main thread so the indicator keeps animating while the model is slow.
//! But only the main thread owns the terminal, so the turn cannot draw the approval prompt
//! itself. This carries the question across: the worker sends a [`WriteRequest`] and blocks
//! until an answer comes back, and the main thread draws the prompt and replies.
//!
//! Blocking the worker is correct rather than a limitation. A write must not proceed until a
//! person has answered, so the turn genuinely has nothing to do until then. Meanwhile the main
//! thread is still redrawing, so the interface stays alive.
//!
//! Every failure resolves to refusal. A closed channel means the interface is gone, and a write
//! nobody can be asked about must not happen.

use bua_agent::confirm::{Confirmer, Decision, WriteRequest};
use std::sync::mpsc::{Receiver, Sender};

/// The worker's end: sends questions, waits for answers.
pub struct RemoteConfirmer {
    questions: Sender<WriteRequest>,
    answers: Receiver<Decision>,
}

impl RemoteConfirmer {
    pub fn new(questions: Sender<WriteRequest>, answers: Receiver<Decision>) -> Self {
        Self { questions, answers }
    }
}

impl Confirmer for RemoteConfirmer {
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision {
        // A channel that cannot carry the question cannot carry consent either.
        if self.questions.send(request.clone()).is_err() {
            return Decision::Reject;
        }
        // Blocks until the main thread answers, which is what a write must wait for.
        self.answers.recv().unwrap_or(Decision::Reject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bua_agent::confirm::Intent;
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
        let (question_tx, question_rx) = channel::<WriteRequest>();
        let (answer_tx, answer_rx) = channel();

        let responder = thread::spawn(move || {
            let asked = question_rx.recv().expect("a question arrived");
            assert_eq!(asked.path, "notes.md");
            answer_tx.send(Decision::Approve).expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(question_tx, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Approve);
        responder.join().expect("responder finished");
    }

    #[test]
    fn a_refusal_travels_back_too() {
        let (question_tx, question_rx) = channel::<WriteRequest>();
        let (answer_tx, answer_rx) = channel();

        thread::spawn(move || {
            question_rx.recv().expect("a question arrived");
            answer_tx.send(Decision::Reject).expect("answered");
        });

        let mut confirmer = RemoteConfirmer::new(question_tx, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
    }

    /// An interface that has gone away cannot consent, so the write is refused rather than
    /// applied unseen.
    #[test]
    fn a_closed_question_channel_refuses() {
        let (question_tx, question_rx) = channel::<WriteRequest>();
        let (_answer_tx, answer_rx) = channel::<Decision>();
        drop(question_rx);

        let mut confirmer = RemoteConfirmer::new(question_tx, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
    }

    /// And an answer channel that closes without replying is a refusal, not a hang.
    #[test]
    fn a_dropped_answer_channel_refuses() {
        let (question_tx, question_rx) = channel::<WriteRequest>();
        let (answer_tx, answer_rx) = channel::<Decision>();

        thread::spawn(move || {
            question_rx.recv().expect("a question arrived");
            // Goes away without answering, as it would if the interface exited.
            drop(answer_tx);
        });

        let mut confirmer = RemoteConfirmer::new(question_tx, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
    }

    /// Several writes in one turn are answered independently and in order.
    #[test]
    fn each_write_gets_its_own_answer() {
        let (question_tx, question_rx) = channel::<WriteRequest>();
        let (answer_tx, answer_rx) = channel();

        thread::spawn(move || {
            for decision in [Decision::Approve, Decision::Reject, Decision::Approve] {
                question_rx.recv().expect("a question arrived");
                answer_tx.send(decision).expect("answered");
            }
        });

        let mut confirmer = RemoteConfirmer::new(question_tx, answer_rx);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Approve);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Reject);
        assert_eq!(confirmer.confirm_write(&request()), Decision::Approve);
    }
}
