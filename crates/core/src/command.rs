//! Running programs, as a pipeline of separately approved stages.
//!
//! This is the one place the repository admits command execution, and the reason it can is
//! narrow: **there is no command string.** A stage is a program name and a vector of arguments,
//! executed directly. No shell interprets it, so there is no parser to defeat and no
//! metacharacter to smuggle. An argument containing `; rm -rf /` is one argument and stays one
//! argument, because nothing ever splits it.
//!
//! That is what restores the routing/content distinction the exclusion in CLAUDE.md was about. A
//! shell string is destination and payload fused, with nothing a person could approve in
//! isolation. An argv vector is a destination a person can read, approve, and have executed
//! verbatim.
//!
//! # What is controlled, and what is not
//!
//! Not the programs. There is no allowlist, and spawned programs run with whatever access the
//! user's own shell would give them. That is deliberate: `git push` needs `~/.ssh`, `npm install`
//! reads `~/.npmrc` and writes `node_modules`, and the set of programs someone might reasonably
//! ask for cannot be enumerated in advance. A confinement profile narrow enough to be worth
//! having would break ordinary tools, so none is imposed.
//!
//! What is controlled is the boundary that holds *without* knowing what ran:
//!
//! - **argv is routing**, so it must be `(T,pub)` and endorsed by a person before anything
//!   executes. This is the real control point, and it is the same one a write goes through.
//! - **stdout and stderr are always `(U,priv)`.** Every stage, no exceptions, and nothing a
//!   caller or the model can declare changes it. A program may print anything, including bytes an
//!   earlier stage read out of a file an attacker wrote, so that is the only label that holds.
//! - **stdin is content**, so it may be untrusted. It is carried into the process and never
//!   consulted, which is what lets untrusted data reach `sed` or `awk` without the planner or the
//!   driver reading it.
//!
//! # Why every run asks
//!
//! There is no read-only category, because there is no way to establish one. `foo --bar` might
//! write to disk and nothing here can tell. An earlier draft had each stage declare whether it
//! wrote or reached the network and used that to skip the prompt; it was dropped because a
//! declaration is only worth something if it is honest, and an unprompted write from a stage that
//! claimed otherwise is worse than a prompt nobody wanted. So the answer to "does this change
//! anything" is always "assume so", and a person decides.
//!
//! Private stdin is a second, independent reason to ask. Untrusted is about integrity and is fine
//! here, since carrying bytes decides nothing. Private is about confidentiality, and handing the
//! user's data to a program releases it somewhere this policy no longer governs.

use std::fmt;

/// One program in a pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    pub program: String,
    pub args: Vec<String>,
}

impl Stage {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    /// The stage as a person should read it before approving.
    ///
    /// Each argument is shown separately, quoted when it contains whitespace or is empty, so a
    /// reviewer can see where one argument ends and the next begins. That boundary is the whole
    /// point: it is what a shell would have decided and what this leaves to nothing.
    pub fn display(&self) -> String {
        let mut out = String::from(&self.program);
        for arg in &self.args {
            out.push(' ');
            if arg.is_empty() || arg.contains(char::is_whitespace) {
                out.push('\'');
                out.push_str(arg);
                out.push('\'');
            } else {
                out.push_str(arg);
            }
        }
        out
    }
}

/// A sequence of stages, each feeding the next.
///
/// Composition is what makes this useful without a shell: narrowing output is a stage rather than
/// a pipe character, so `git log` into `sed -n 1,10p` filters before anything is labelled. It also
/// means untrusted content can be reshaped by real tools while never being read here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    pub stages: Vec<Stage>,
    /// The label of anything fed to the first stage's stdin.
    ///
    /// `None` when nothing is. Held on the pipeline rather than the stage because only the first
    /// stage has a stdin of its own; the rest are fed by the stage before them.
    pub stdin: Option<crate::label::Label>,
}

impl Pipeline {
    pub fn new(stages: Vec<Stage>) -> Self {
        Self {
            stages,
            stdin: None,
        }
    }

    /// Note that content of this label will be fed to the first stage.
    pub fn with_stdin(mut self, label: crate::label::Label) -> Self {
        self.stdin = Some(label);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stages.len()
    }

    /// Whether running this would put the user's private data into a program.
    ///
    /// Deliberately not conditioned on what the stages appear to do: a program that was handed the
    /// bytes had them, and reasoning that it probably could not have sent them anywhere is the
    /// kind of reasoning this design avoids.
    pub fn releases_private(&self) -> bool {
        self.stdin.is_some_and(|label| !label.is_public())
    }

    /// The pipeline as a person should read it before approving.
    pub fn display(&self) -> String {
        self.stages
            .iter()
            .map(Stage::display)
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::Label;

    /// Argument boundaries have to be visible, because that boundary is exactly what no shell is
    /// deciding here and what a reviewer is being asked to approve.
    #[test]
    fn a_stage_shows_its_argument_boundaries() {
        let stage = Stage::new(
            "git",
            vec!["commit".into(), "-m".into(), "two words".into()],
        );
        assert_eq!(stage.display(), "git commit -m 'two words'");
    }

    /// An empty argument is real and must not vanish from the display, or a reviewer would approve
    /// something other than what runs.
    #[test]
    fn an_empty_argument_is_still_shown() {
        assert_eq!(Stage::new("prog", vec![String::new()]).display(), "prog ''");
    }

    /// A shell metacharacter is data here, so it is shown as the ordinary argument it is rather
    /// than as syntax. Nothing will re-parse it.
    #[test]
    fn a_metacharacter_is_shown_as_the_argument_it_is() {
        let stage = Stage::new("git", vec!["commit".into(), "-m".into(), "fix; ok".into()]);
        assert_eq!(stage.display(), "git commit -m 'fix; ok'");
    }

    #[test]
    fn a_pipeline_shows_its_stages_in_order() {
        let pipeline = Pipeline::new(vec![
            Stage::new("git", vec!["log".into()]),
            Stage::new("head", vec!["-3".into()]),
        ]);
        assert_eq!(pipeline.display(), "git log | head -3");
    }

    fn plain() -> Pipeline {
        Pipeline::new(vec![Stage::new("sed", vec!["-n".into(), "1,5p".into()])])
    }

    /// Untrusted content may be fed to a command. It is carried into the process, never consulted,
    /// so there is nothing for it to steer: that is what makes a command line usable on data an
    /// attacker wrote.
    #[test]
    fn untrusted_content_may_be_fed_in_without_a_release() {
        let pipeline = plain().with_stdin(Label::untrusted_public());
        assert!(
            !pipeline.releases_private(),
            "carrying untrusted content is not a release"
        );
    }

    /// Private content going into a program is a release to somewhere this policy stops governing.
    #[test]
    fn private_content_is_a_release() {
        assert!(plain().with_stdin(Label::untrusted_private()).releases_private());
    }

    /// Integrity and confidentiality gate separately: vouching for what a file contains is not
    /// consenting to send it somewhere.
    #[test]
    fn trusted_private_content_is_still_a_release() {
        assert!(plain().with_stdin(Label::trusted_private()).releases_private());
    }

    /// Feeding nothing in releases nothing.
    #[test]
    fn a_pipeline_with_no_input_releases_nothing() {
        assert!(!plain().releases_private());
    }
}
