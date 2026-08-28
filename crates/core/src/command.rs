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
    /// Each argument is shown separately, so a reviewer can see where one argument ends and the
    /// next begins. That boundary is the whole point: it is what a shell would have decided and
    /// what this leaves to nothing.
    ///
    /// The quoting has to be **unambiguous**, not merely readable, because an approval is bound to
    /// what the person saw. Quoting only on whitespace was not: `["a b"]` and `["'a", "b'"]` both
    /// came out as `prog 'a b'`, so a reviewer reading one could have been approving the other. So
    /// a quote or a backslash forces quoting too, and both are escaped inside it.
    pub fn display(&self) -> String {
        let mut out = String::from(&self.program);
        for arg in &self.args {
            out.push(' ');
            out.push_str(&quoted(arg));
        }
        out
    }
}

/// One argument, quoted so that no two arguments can render alike.
///
/// A bare token is one with nothing in it that quoting is for. Anything else is wrapped, with a
/// backslash and a quote escaped inside the wrapping, which is what makes the rendering reversible
/// and therefore safe to bind an approval to.
fn quoted(arg: &str) -> String {
    let plain =
        !arg.is_empty() && !arg.contains(|c: char| c.is_whitespace() || c == '\'' || c == '\\');
    if plain {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('\'');
    for c in arg.chars() {
        if c == '\'' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
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

    /// The exact value an endorsement for this pipeline is bound to.
    ///
    /// Length-prefixed rather than delimited, so no argument can contain whatever a delimiter
    /// would have been. Two pipelines encode alike only if they are the same pipeline, which is
    /// what a single-use grant needs: a grant left unconsumed by a failed run must not be
    /// satisfiable by a second pipeline the planner shapes to collide with the first.
    ///
    /// Not for a person to read. [`Pipeline::display`] is that, and the two exist separately
    /// because a rendering has to be legible while this has to be injective.
    pub fn canonical(&self) -> String {
        let mut out = format!("{}|", self.stages.len());
        for stage in &self.stages {
            out.push_str(&format!("{}|", stage.args.len() + 1));
            for token in std::iter::once(&stage.program).chain(&stage.args) {
                out.push_str(&format!("{}:{token}", token.len()));
            }
        }
        out
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

    /// Two different argument lists must never render alike. An approval is bound to what the
    /// person read, so a rendering that collides lets a reviewer approve one argv while another
    /// runs. Quoting on whitespace alone collided here: both of these came out as `prog 'a b'`.
    #[test]
    fn two_argument_lists_cannot_look_alike() {
        let one = Stage::new("prog", vec!["a b".into()]);
        let other = Stage::new("prog", vec!["'a".into(), "b'".into()]);
        assert_ne!(
            one.display(),
            other.display(),
            "two argument lists rendered identically, so an approval cannot name either"
        );
    }

    /// A quote in an argument is data, so it survives the rendering rather than being read as the
    /// end of a quoted run.
    #[test]
    fn a_quote_in_an_argument_is_escaped() {
        let stage = Stage::new("prog", vec!["it's".into()]);
        assert_eq!(stage.display(), r"prog 'it\'s'");
    }

    /// A backslash is what does the escaping, so an argument containing one has to escape it too
    /// or a trailing backslash would escape the closing quote.
    #[test]
    fn a_backslash_in_an_argument_is_escaped() {
        let stage = Stage::new("prog", vec![r"c:\path".into()]);
        assert_eq!(stage.display(), r"prog 'c:\\path'");
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

    /// A grant is bound to this value, so two different pipelines must never produce the same
    /// one. Delimiting would not have held: an argument may contain any byte a delimiter could be.
    #[test]
    fn two_pipelines_never_encode_alike() {
        let one = Pipeline::new(vec![Stage::new("prog", vec!["a".into(), "b".into()])]);
        let other = Pipeline::new(vec![Stage::new("prog", vec!["a b".into()])]);
        assert_ne!(one.canonical(), other.canonical());

        let split = Pipeline::new(vec![
            Stage::new("a", vec!["b".into()]),
            Stage::new("c", Vec::new()),
        ]);
        let joined = Pipeline::new(vec![Stage::new("a", vec!["b".into(), "c".into()])]);
        assert_ne!(
            split.canonical(),
            joined.canonical(),
            "a stage boundary must be part of what is endorsed"
        );
    }

    /// An argument holding the encoding's own punctuation is data like any other, so it cannot
    /// forge a boundary.
    #[test]
    fn an_argument_cannot_forge_an_encoding_boundary() {
        let one = Pipeline::new(vec![Stage::new("prog", vec!["1|1:x".into()])]);
        let other = Pipeline::new(vec![Stage::new("prog", vec!["1".into(), "x".into()])]);
        assert_ne!(one.canonical(), other.canonical());
    }

    /// The same pipeline must encode the same way every time, or an endorsement issued for one
    /// would not match the run it was issued for.
    #[test]
    fn the_same_pipeline_encodes_the_same_way() {
        let build = || Pipeline::new(vec![Stage::new("git", vec!["log".into()])]);
        assert_eq!(build().canonical(), build().canonical());
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
        assert!(
            plain()
                .with_stdin(Label::untrusted_private())
                .releases_private()
        );
    }

    /// Integrity and confidentiality gate separately: vouching for what a file contains is not
    /// consenting to send it somewhere.
    #[test]
    fn trusted_private_content_is_still_a_release() {
        assert!(
            plain()
                .with_stdin(Label::trusted_private())
                .releases_private()
        );
    }

    /// Feeding nothing in releases nothing.
    #[test]
    fn a_pipeline_with_no_input_releases_nothing() {
        assert!(!plain().releases_private());
    }
}
