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

use std::path::PathBuf;

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

/// How two parts of a plan are joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joiner {
    /// `&&`: the right side runs only if the left side succeeded.
    And,
    /// `||`: the right side runs only if the left side failed.
    Or,
    /// `;`: the right side runs either way.
    Then,
}

/// Where one of a step's streams goes.
///
/// The paths are absolute and are not tidied. What is recorded is what will be opened, and
/// rewriting a `..` away would make the two differ wherever a symlink is involved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Stdout {
        path: PathBuf,
        append: bool,
    },
    Stdin {
        path: PathBuf,
    },
    Stderr {
        path: PathBuf,
        append: bool,
    },
    Both {
        path: PathBuf,
    },
    /// Standard error joins standard output, touching no file.
    StderrToStdout,
}

/// One program in a plan, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The name the line used, which is what a reader recognises.
    pub program: String,
    /// The file that name resolved to, absolute.
    ///
    /// Resolved before anybody is asked, so that what a person endorsed and what executes are the
    /// same value. This is what an endorsement is keyed on, never the name: `$PATH` decides what a
    /// bare name means, so an approval recorded against the string would follow the name onto
    /// whatever it later pointed at.
    pub resolved: PathBuf,
    /// The argument vector, literal and final.
    pub args: Vec<String>,
    /// `NAME=value` written in front of this step's own program.
    pub environment: Vec<(String, String)>,
    /// Where this step's streams go.
    pub routes: Vec<Route>,
}

impl Step {
    /// The step as a person should read it before approving.
    ///
    /// The resolved binary rather than the name, because that is what will run, and every argument
    /// quoted so that no two argument lists can render alike.
    pub fn display(&self) -> String {
        let mut out = String::new();
        for (name, value) in &self.environment {
            out.push_str(&format!("{name}={} ", quoted(value)));
        }
        out.push_str(&self.resolved.to_string_lossy());
        for arg in &self.args {
            out.push(' ');
            out.push_str(&quoted(arg));
        }
        for route in &self.routes {
            out.push(' ');
            out.push_str(&match route {
                Route::Stdout { path, append } => format!(
                    "{} {}",
                    if *append { ">>" } else { ">" },
                    quoted(&path.to_string_lossy())
                ),
                Route::Stdin { path } => format!("< {}", quoted(&path.to_string_lossy())),
                Route::Stderr { path, append } => format!(
                    "{} {}",
                    if *append { "2>>" } else { "2>" },
                    quoted(&path.to_string_lossy())
                ),
                Route::Both { path } => format!("&> {}", quoted(&path.to_string_lossy())),
                Route::StderrToStdout => "2>&1".to_string(),
            });
        }
        out
    }

    /// The command an endorsement for this step would be recorded against.
    pub fn command(&self) -> crate::programs::Command {
        crate::programs::Command::new(
            self.resolved.to_string_lossy().to_string(),
            self.args.clone(),
        )
    }
}

/// What a plan runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Steps {
    /// Steps feeding one another, and a single step where the line had no pipe.
    Pipeline(Vec<Step>),
    /// Two of these joined by `&&`, `||` or `;`.
    Join {
        left: Box<Steps>,
        joiner: Joiner,
        right: Box<Steps>,
    },
    /// `( … )`, which groups and starts nothing of its own.
    Group(Box<Steps>),
}

impl Steps {
    /// Every step that could run, in the order the line writes them.
    ///
    /// Every one of them, including those a branch may not reach. A step that does not run is not
    /// an effect, but it was still endorsed, and that is the conservative direction.
    pub fn steps(&self) -> Vec<&Step> {
        let mut out = Vec::new();
        self.gather(&mut out);
        out
    }

    fn gather<'a>(&'a self, out: &mut Vec<&'a Step>) {
        match self {
            Self::Pipeline(steps) => out.extend(steps.iter()),
            Self::Join { left, right, .. } => {
                left.gather(out);
                right.gather(out);
            }
            Self::Group(inner) => inner.gather(out),
        }
    }

    /// The shape as a person should read it before approving.
    pub fn display(&self) -> String {
        match self {
            Self::Pipeline(steps) => steps
                .iter()
                .map(Step::display)
                .collect::<Vec<_>>()
                .join(" | "),
            Self::Join {
                left,
                joiner,
                right,
            } => {
                let joiner = match joiner {
                    Joiner::And => "&&",
                    Joiner::Or => "||",
                    Joiner::Then => ";",
                };
                format!("{} {joiner} {}", left.display(), right.display())
            }
            Self::Group(inner) => format!("( {} )", inner.display()),
        }
    }
}

/// A command line, compiled.
///
/// This is the routing field a raw string does not have: what will run, with which binary and
/// which literal arguments, every file it may write, every file it reads by name, and the
/// directory it runs in. A person endorses this rather than the text the planner sent, because
/// after compilation this is what decides where an effect lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The line as the planner spelled it.
    ///
    /// Context for a reader, and it binds nothing: two spellings that compile alike are one plan,
    /// and one spelling that compiles two ways on two occasions is two.
    pub line: String,
    /// The directory every step runs in.
    pub directory: PathBuf,
    /// What runs, and how the parts are joined.
    pub steps: Steps,
    /// Every file the plan may write, in the order the line names them.
    pub writes: Vec<PathBuf>,
    /// Every file the plan reads by naming it as a source for a stream.
    pub reads: Vec<PathBuf>,
    /// The label of anything fed to the first step's standard input.
    pub stdin: Option<crate::label::Label>,
}

impl Plan {
    /// Every step that could run.
    pub fn steps(&self) -> Vec<&Step> {
        self.steps.steps()
    }

    /// Whether running this would put the user's private data into a program.
    pub fn releases_private(&self) -> bool {
        self.stdin.is_some_and(|label| !label.is_public())
    }

    /// The plan as a person should read it before approving.
    ///
    /// Not the line. The line is shown beside this as context, and an endorsement binds here.
    pub fn display(&self) -> String {
        self.steps.display()
    }

    /// The exact value an endorsement for this plan is bound to.
    ///
    /// Length-prefixed rather than delimited, so no argument can contain whatever a delimiter
    /// would have been, and prefix-coded over the shape, so `a && b` and `a ; b` cannot encode
    /// alike. Two plans encode the same way only if they are the same plan, which is what a
    /// single-use grant needs: a grant left unconsumed by a failed run must not be satisfiable by
    /// a second plan the planner shapes to collide with the first.
    ///
    /// Keyed on each step's resolved file rather than on the name the line used, for the reason a
    /// vouched-for command is: `$PATH` decides what a bare name means, and an endorsement must not
    /// follow a name onto a different binary. The line itself is not part of this, because two
    /// spellings of one plan are one plan.
    ///
    /// Not for a person to read. [`Plan::display`] is that, and the two exist separately because a
    /// rendering has to be legible while this has to be injective.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        length_prefixed(&mut out, &self.directory.to_string_lossy());
        encode(&mut out, &self.steps);
        out
    }
}

/// Append `text` so that no text can forge the boundary after it.
fn length_prefixed(out: &mut String, text: &str) {
    out.push_str(&format!("{}:{text}", text.len()));
}

fn encode(out: &mut String, steps: &Steps) {
    match steps {
        Steps::Pipeline(list) => {
            out.push_str(&format!("P{}|", list.len()));
            for step in list {
                encode_step(out, step);
            }
        }
        Steps::Join {
            left,
            joiner,
            right,
        } => {
            out.push_str(match joiner {
                Joiner::And => "A|",
                Joiner::Or => "O|",
                Joiner::Then => "T|",
            });
            encode(out, left);
            encode(out, right);
        }
        Steps::Group(inner) => {
            out.push_str("G|");
            encode(out, inner);
        }
    }
}

fn encode_step(out: &mut String, step: &Step) {
    length_prefixed(out, &step.resolved.to_string_lossy());
    out.push_str(&format!("a{}|", step.args.len()));
    for arg in &step.args {
        length_prefixed(out, arg);
    }
    out.push_str(&format!("e{}|", step.environment.len()));
    for (name, value) in &step.environment {
        length_prefixed(out, name);
        length_prefixed(out, value);
    }
    out.push_str(&format!("r{}|", step.routes.len()));
    for route in &step.routes {
        let (tag, path) = match route {
            Route::Stdout { path, append } => (if *append { ">>" } else { ">" }, Some(path)),
            Route::Stdin { path } => ("<", Some(path)),
            Route::Stderr { path, append } => (if *append { "2>>" } else { "2>" }, Some(path)),
            Route::Both { path } => ("&>", Some(path)),
            Route::StderrToStdout => ("2>&1", None),
        };
        out.push_str(tag);
        out.push('|');
        if let Some(path) = path {
            length_prefixed(out, &path.to_string_lossy());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::Label;

    fn step(program: &str, args: &[&str]) -> Step {
        Step {
            program: program.to_string(),
            resolved: PathBuf::from(format!("/usr/bin/{program}")),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            environment: Vec::new(),
            routes: Vec::new(),
        }
    }

    fn plan(steps: Steps) -> Plan {
        Plan {
            line: String::new(),
            directory: PathBuf::from("/work"),
            steps,
            writes: Vec::new(),
            reads: Vec::new(),
            stdin: None,
        }
    }

    /// An endorsement is bound to this value, so two plans that would do different things must
    /// never produce the same one. The shape counts: `a && b` runs `b` only on success and
    /// `a ; b` runs it either way, so a grant for one must not satisfy the other.
    #[test]
    fn two_plans_never_encode_alike() {
        let pair = || (step("a", &[]), step("b", &[]));
        let (one, two) = pair();
        let sequenced = plan(Steps::Join {
            left: Box::new(Steps::Pipeline(vec![one])),
            joiner: Joiner::Then,
            right: Box::new(Steps::Pipeline(vec![two])),
        });
        let (one, two) = pair();
        let conditional = plan(Steps::Join {
            left: Box::new(Steps::Pipeline(vec![one])),
            joiner: Joiner::And,
            right: Box::new(Steps::Pipeline(vec![two])),
        });
        let (one, two) = pair();
        let piped = plan(Steps::Pipeline(vec![one, two]));

        assert_ne!(sequenced.canonical(), conditional.canonical());
        assert_ne!(sequenced.canonical(), piped.canonical());
        assert_ne!(conditional.canonical(), piped.canonical());
    }

    /// A step boundary is part of what is endorsed, so splitting one command into two must not
    /// encode the way one command with more arguments does.
    #[test]
    fn a_step_boundary_is_part_of_what_is_endorsed() {
        let split = plan(Steps::Pipeline(vec![step("a", &["b"]), step("c", &[])]));
        let joined = plan(Steps::Pipeline(vec![step("a", &["b", "c"])]));
        assert_ne!(split.canonical(), joined.canonical());
    }

    /// An argument holding the encoding's own punctuation is data like any other, so it cannot
    /// forge a boundary and make one plan encode as another.
    #[test]
    fn an_argument_cannot_forge_a_plans_encoding_boundary() {
        let one = plan(Steps::Pipeline(vec![step("prog", &["a1|1:x"])]));
        let other = plan(Steps::Pipeline(vec![step("prog", &["a1", "x"])]));
        assert_ne!(one.canonical(), other.canonical());
    }

    /// Where the bytes go is what a person endorsed, so the same steps writing somewhere else are
    /// a different plan.
    #[test]
    fn where_a_plan_writes_is_part_of_what_it_encodes() {
        let with_route = |path: &str| {
            let mut only = step("prog", &[]);
            only.routes = vec![Route::Stdout {
                path: PathBuf::from(path),
                append: false,
            }];
            plan(Steps::Pipeline(vec![only]))
        };
        assert_ne!(
            with_route("/work/a.txt").canonical(),
            with_route("/work/b.txt").canonical()
        );

        let mut appending = step("prog", &[]);
        appending.routes = vec![Route::Stdout {
            path: PathBuf::from("/work/a.txt"),
            append: true,
        }];
        assert_ne!(
            with_route("/work/a.txt").canonical(),
            plan(Steps::Pipeline(vec![appending])).canonical(),
            "truncating and appending are different things to endorse"
        );
    }

    /// The directory decides what a relative path in the plan means, so it is part of the plan.
    #[test]
    fn the_directory_a_plan_runs_in_is_part_of_what_it_encodes() {
        let here = plan(Steps::Pipeline(vec![step("prog", &[])]));
        let mut elsewhere = plan(Steps::Pipeline(vec![step("prog", &[])]));
        elsewhere.directory = PathBuf::from("/somewhere-else");
        assert_ne!(here.canonical(), elsewhere.canonical());
    }

    /// Keyed on the file, never on the name. `$PATH` decides what a bare name means, so an
    /// endorsement recorded against the string would follow the name onto a different binary.
    #[test]
    fn a_plan_is_keyed_on_the_file_a_name_resolved_to() {
        let mut written_short = step("grep", &["x"]);
        written_short.program = "grep".to_string();
        let mut written_long = step("grep", &["x"]);
        written_long.program = "/usr/bin/grep".to_string();
        assert_eq!(
            plan(Steps::Pipeline(vec![written_short])).canonical(),
            plan(Steps::Pipeline(vec![written_long])).canonical(),
            "two spellings of one plan are one plan"
        );
    }

    /// The line is context for a reader and binds nothing, so two spellings that compile alike
    /// are one endorsement.
    #[test]
    fn the_line_a_plan_came_from_is_not_part_of_what_it_encodes() {
        let mut one = plan(Steps::Pipeline(vec![step("prog", &["a b"])]));
        one.line = "prog 'a b'".to_string();
        let mut other = plan(Steps::Pipeline(vec![step("prog", &["a b"])]));
        other.line = "prog \"a b\"".to_string();
        assert_eq!(one.canonical(), other.canonical());
    }

    #[test]
    fn the_same_plan_encodes_the_same_way() {
        let build = || plan(Steps::Pipeline(vec![step("git", &["log"])]));
        assert_eq!(build().canonical(), build().canonical());
    }

    /// A person reads the binary that will run and where each argument ends, because those are
    /// exactly what no shell is deciding here.
    #[test]
    fn a_plan_shows_its_resolved_binaries_and_argument_boundaries() {
        let shown = plan(Steps::Pipeline(vec![
            step("git", &["commit", "-m", "two words"]),
            step("head", &["-3"]),
        ]))
        .display();
        assert_eq!(
            shown,
            "/usr/bin/git commit -m 'two words' | /usr/bin/head -3"
        );
    }

    #[test]
    fn a_plan_shows_where_its_streams_go_and_how_its_parts_are_joined() {
        let mut writing = step("prog", &[]);
        writing.routes = vec![
            Route::Stdout {
                path: PathBuf::from("/work/out.txt"),
                append: true,
            },
            Route::StderrToStdout,
        ];
        let shown = plan(Steps::Join {
            left: Box::new(Steps::Group(Box::new(Steps::Pipeline(vec![step(
                "a",
                &[],
            )])))),
            joiner: Joiner::Or,
            right: Box::new(Steps::Pipeline(vec![writing])),
        })
        .display();
        assert_eq!(
            shown,
            "( /usr/bin/a ) || /usr/bin/prog >> /work/out.txt 2>&1"
        );
    }

    /// Every step that could run is listed, including one a branch may not reach: it was still
    /// endorsed, and that is the conservative direction.
    #[test]
    fn a_plan_lists_every_step_that_could_run() {
        let shape = Steps::Join {
            left: Box::new(Steps::Pipeline(vec![step("a", &[]), step("b", &[])])),
            joiner: Joiner::And,
            right: Box::new(Steps::Group(Box::new(Steps::Pipeline(vec![step(
                "c",
                &[],
            )])))),
        };
        let whole = plan(shape);
        let listed: Vec<&str> = whole
            .steps()
            .iter()
            .map(|step| step.program.as_str())
            .collect();
        assert_eq!(listed, ["a", "b", "c"]);
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
