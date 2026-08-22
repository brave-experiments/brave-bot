//! Running programs, as a pipeline of separately gated stages.
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
//! # Why programs are not enumerated
//!
//! There is no allowlist of permitted programs, and adding one would buy less than it appears to.
//! What bounds a stage is not its name but the confinement it runs under, which the OS enforces:
//! a stage that asked for no writes cannot write, whatever binary it is. So `sed`, `awk`, `jq`,
//! `rg` and anything else installed all work without being named anywhere, and a stage that
//! misrepresents itself fails rather than escaping.
//!
//! # Why a declared reach may be trusted to select a gate
//!
//! [`Reach`] arrives from the model, so it is untrusted, and the driver does branch on it to
//! decide which gate applies. That is admissible here for one specific reason: the branch is
//! monotone in the safe direction. Declaring [`Reach::Confined`] asks for *less* privilege, not
//! more, and it is the OS that then holds the stage to it. A stage that wanted to write and said
//! otherwise to avoid being shown to a person does not gain a silent write; it gains a denied
//! one.
//!
//! Anything beyond [`Reach::Confined`] is an effect, and effects need a human. There is no
//! declaration that reaches further without being seen.
//!
//! # What goes in, and what comes out
//!
//! Output is **always** untrusted and private, for every stage, with no exception and no
//! declaration that changes it. A program can emit anything, including bytes an earlier stage read
//! from a file an attacker wrote, so untrusted is the only label that holds without knowing what
//! ran. Whether some narrower class could ever be trusted is deliberately left open and tracked as
//! an issue; it is not settled here.
//!
//! Input splits the way every other effect in this repository splits:
//!
//! - **argv is routing.** The program and its arguments decide what happens and where it lands, so
//!   they must be `(T,pub)`: promoted for a confined stage, endorsed by a person for an effect.
//! - **stdin is content.** It may be untrusted, because it is carried rather than consulted. The
//!   planner names a quarantined slot and the kernel supplies the bytes, so untrusted content is
//!   fed to `sed` or `awk` without the planner or the driver ever reading it.
//!
//! That split is what lets both trusted and untrusted data reach a command line. It also explains
//! why untrusted text does not belong in argv: an argument is a destination, and a destination
//! derived from attacker-influenced bytes is the injection this design exists to prevent. Content
//! has its own channel.
//!
//! # Private input asks
//!
//! Untrusted is about integrity; private is about confidentiality, and they gate differently.
//! Workspace content is private as a matter of course, and handing it to a subprocess releases it
//! to somewhere the policy no longer governs: a stage with network could send it, and a stage with
//! writes could put it anywhere it can write. So private stdin is a release, and a release needs a
//! person. [`Pipeline::releases_private`] is what a caller checks to know it must ask.

use std::fmt;

/// What a stage needs beyond reading inside the workspace.
///
/// Ordered by privilege, least first. A stage gets exactly what it declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reach {
    /// Read the workspace, and nothing else. No writes, no network, no children.
    ///
    /// The default, and the case that needs no human: a stage that cannot change anything and
    /// cannot speak to anyone is confined the same way [`crate::policy::Policy::promote_confined_read`]
    /// describes, so a wrong choice costs a step rather than causing harm.
    #[default]
    Confined,
    /// Write inside the workspace, as well as read it.
    Writes,
    /// Reach the network, as well as read the workspace.
    Network,
    /// Both write and reach the network.
    WritesAndNetwork,
}

impl Reach {
    /// Whether this reach changes anything or speaks to anyone, and so needs a person.
    pub fn is_effect(self) -> bool {
        !matches!(self, Self::Confined)
    }

    pub fn writes(self) -> bool {
        matches!(self, Self::Writes | Self::WritesAndNetwork)
    }

    pub fn network(self) -> bool {
        matches!(self, Self::Network | Self::WritesAndNetwork)
    }

    /// Read a reach from what the model asked for.
    ///
    /// Total, and deliberately biased: anything unrecognised is [`Reach::Confined`], the least
    /// privileged reading. An unparseable request must not be able to acquire more than it named,
    /// and a stage that genuinely needed more will fail visibly under confinement rather than
    /// quietly succeeding.
    pub fn parse(writes: bool, network: bool) -> Self {
        match (writes, network) {
            (false, false) => Self::Confined,
            (true, false) => Self::Writes,
            (false, true) => Self::Network,
            (true, true) => Self::WritesAndNetwork,
        }
    }
}

impl fmt::Display for Reach {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confined => f.write_str("confined"),
            Self::Writes => f.write_str("writes"),
            Self::Network => f.write_str("network"),
            Self::WritesAndNetwork => f.write_str("writes and network"),
        }
    }
}

/// One program in a pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    pub program: String,
    pub args: Vec<String>,
    pub reach: Reach,
}

impl Stage {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            reach: Reach::Confined,
        }
    }

    pub fn with_reach(mut self, reach: Reach) -> Self {
        self.reach = reach;
        self
    }

    /// The stage as a person should read it before approving.
    ///
    /// Each argument is shown separately, quoted when it contains a space, so a reviewer can see
    /// where one argument ends and the next begins. That boundary is the whole point: it is what a
    /// shell would have decided and what this does not leave to anything.
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
/// a pipe character, so `git log` into `sed -n 1,10p` does its filtering inside confinement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    pub stages: Vec<Stage>,
    /// The label of anything fed to the first stage's stdin.
    ///
    /// `None` when nothing is. Held on the pipeline rather than the stage because only the first
    /// stage has a stdin of its own; the rest are fed by the stage before them.
    ///
    /// This is the label of *content*, so untrusted is permitted: the bytes are carried into the
    /// process and never consulted by the driver or shown to the planner. Private is what asks a
    /// person, since a subprocess is somewhere the policy stops governing.
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

    /// Whether running this would put private data somewhere the policy no longer governs.
    ///
    /// True when private content is fed in, whatever the stages do with it. Deliberately not
    /// conditioned on reach: a stage with no network and no writes still had the bytes, and
    /// treating "it probably could not get out" as equivalent to "it cannot" is the kind of
    /// reasoning this design avoids. A person decides instead.
    pub fn releases_private(&self) -> bool {
        self.stdin.is_some_and(|label| !label.is_public())
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stages.len()
    }

    /// The most privileged reach any stage asked for.
    ///
    /// A pipeline is as consequential as its most consequential stage, so this is what decides
    /// whether the whole thing needs a person.
    pub fn reach(&self) -> Reach {
        let writes = self.stages.iter().any(|s| s.reach.writes());
        let network = self.stages.iter().any(|s| s.reach.network());
        Reach::parse(writes, network)
    }

    /// Whether any stage would change anything or speak to anyone.
    pub fn is_effect(&self) -> bool {
        self.reach().is_effect()
    }

    /// Whether a person has to approve this before it runs.
    ///
    /// Either because it changes something, or because it releases private data to a process the
    /// policy stops governing. Both are the user's call, for different reasons.
    pub fn needs_approval(&self) -> bool {
        self.is_effect() || self.releases_private()
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

    #[test]
    fn a_confined_stage_is_not_an_effect() {
        assert!(!Reach::Confined.is_effect());
        assert!(!Reach::Confined.writes());
        assert!(!Reach::Confined.network());
    }

    /// Anything beyond reading is an effect and must reach a person.
    #[test]
    fn every_reach_beyond_confined_is_an_effect() {
        for reach in [Reach::Writes, Reach::Network, Reach::WritesAndNetwork] {
            assert!(reach.is_effect(), "{reach} was not treated as an effect");
        }
    }

    /// The default is the least privileged reading, so a stage that says nothing gets nothing.
    #[test]
    fn the_default_reach_is_confined() {
        assert_eq!(Reach::default(), Reach::Confined);
        assert_eq!(Stage::new("sed", vec![]).reach, Reach::Confined);
    }

    #[test]
    fn a_reach_reports_what_it_covers() {
        assert!(Reach::Writes.writes() && !Reach::Writes.network());
        assert!(Reach::Network.network() && !Reach::Network.writes());
        assert!(Reach::WritesAndNetwork.writes() && Reach::WritesAndNetwork.network());
    }

    /// A pipeline is as consequential as its most consequential stage: a read-only stage feeding a
    /// writing one is a writing pipeline.
    #[test]
    fn a_pipeline_takes_the_most_privileged_reach_in_it() {
        let pipeline = Pipeline::new(vec![
            Stage::new("git", vec!["log".into()]),
            Stage::new("tee", vec!["out.txt".into()]).with_reach(Reach::Writes),
        ]);
        assert_eq!(pipeline.reach(), Reach::Writes);
        assert!(pipeline.is_effect());
    }

    #[test]
    fn a_wholly_confined_pipeline_is_not_an_effect() {
        let pipeline = Pipeline::new(vec![
            Stage::new("git", vec!["log".into(), "--oneline".into()]),
            Stage::new("sed", vec!["-n".into(), "1,10p".into()]),
        ]);
        assert_eq!(pipeline.reach(), Reach::Confined);
        assert!(!pipeline.is_effect());
    }

    /// Reach combines across stages rather than being taken from any one of them.
    #[test]
    fn separate_stages_contribute_separate_privileges() {
        let pipeline = Pipeline::new(vec![
            Stage::new("curl", vec!["https://example.com".into()]).with_reach(Reach::Network),
            Stage::new("tee", vec!["out.txt".into()]).with_reach(Reach::Writes),
        ]);
        assert_eq!(pipeline.reach(), Reach::WritesAndNetwork);
    }

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
        let stage = Stage::new("prog", vec![String::new()]);
        assert_eq!(stage.display(), "prog ''");
    }

    /// A shell metacharacter is data here, so it is shown as the ordinary argument it is rather
    /// than being escaped as though it had meaning.
    #[test]
    fn a_metacharacter_is_shown_as_the_argument_it_is() {
        let stage = Stage::new("git", vec!["commit".into(), "-m".into(), "fix; ok".into()]);
        assert_eq!(stage.display(), "git commit -m 'fix; ok'");
    }

    mod input {
        use super::*;
        use crate::label::Label;

        fn confined() -> Pipeline {
            Pipeline::new(vec![Stage::new("sed", vec!["-n".into(), "1,5p".into()])])
        }

        /// Untrusted content may be fed to a command. It is carried into the process, never
        /// consulted, so there is nothing for it to steer: that is what makes a command line usable
        /// on data an attacker wrote.
        #[test]
        fn untrusted_content_may_be_fed_in_without_asking() {
            let pipeline = confined().with_stdin(Label::untrusted_public());
            assert!(!pipeline.releases_private());
            assert!(
                !pipeline.needs_approval(),
                "carrying untrusted content should not require a person"
            );
        }

        /// Private content is a release to somewhere the policy stops governing, so it asks even
        /// though nothing is being changed.
        #[test]
        fn private_content_asks_even_when_the_pipeline_is_confined() {
            let pipeline = confined().with_stdin(Label::untrusted_private());
            assert!(!pipeline.is_effect(), "this pipeline changes nothing");
            assert!(pipeline.releases_private());
            assert!(
                pipeline.needs_approval(),
                "private data reached a subprocess without a person approving"
            );
        }

        /// Trusted but private is still private: integrity and confidentiality gate separately, and
        /// vouching for a file's contents is not consenting to send them somewhere.
        #[test]
        fn trusted_private_content_still_asks() {
            let pipeline = confined().with_stdin(Label::trusted_private());
            assert!(pipeline.releases_private());
            assert!(pipeline.needs_approval());
        }

        /// A confined stage still had the bytes. Deciding it "probably could not leak them" is the
        /// reasoning this design refuses, so confinement does not excuse the release.
        #[test]
        fn confinement_does_not_excuse_releasing_private_data() {
            let confined_pipeline = confined().with_stdin(Label::untrusted_private());
            let networked = Pipeline::new(vec![
                Stage::new("curl", vec!["https://example.com".into()]).with_reach(Reach::Network),
            ])
            .with_stdin(Label::untrusted_private());

            assert!(confined_pipeline.releases_private());
            assert!(networked.releases_private());
        }

        /// Feeding nothing in releases nothing, so a plain confined pipeline runs unattended.
        #[test]
        fn a_pipeline_with_no_input_releases_nothing() {
            assert!(!confined().releases_private());
            assert!(!confined().needs_approval());
        }

        /// An effect asks regardless of what was fed in, since the two reasons are independent.
        #[test]
        fn an_effect_asks_whether_or_not_anything_was_fed_in() {
            let writing = Pipeline::new(vec![
                Stage::new("tee", vec!["out.txt".into()]).with_reach(Reach::Writes),
            ]);
            assert!(writing.needs_approval());
            assert!(
                writing.with_stdin(Label::untrusted_public()).needs_approval(),
                "public input should not have excused the write"
            );
        }
    }

    #[test]
    fn a_pipeline_shows_its_stages_in_order() {
        let pipeline = Pipeline::new(vec![
            Stage::new("git", vec!["log".into()]),
            Stage::new("head", vec!["-3".into()]),
        ]);
        assert_eq!(pipeline.display(), "git log | head -3");
    }
}
