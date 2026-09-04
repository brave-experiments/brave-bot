//! Rules a person wrote down in advance about which actions to ask them about.
//!
//! The same three lists Claude Code keeps, with the same spellings, so a `permissions` block
//! copied out of `~/.claude/settings.json` governs this agent unedited. A rule is `Tool` or
//! `Tool(specifier)`, the lists are consulted deny, then ask, then allow, and the first match in
//! that order decides. See `docs/specs/permissions.md`.
//!
//! # What a rule may be about
//!
//! A specifier matches a **routing** field and nothing else: a path, or the argv of a stage.
//! Routing is `(T,pub)` before it reaches any gate, so matching on it is the driver deciding from
//! trusted input, which is what it is for. Nothing here is ever handed a file's contents, a
//! program's output, or anything else a turn observed. A rule that matched on those would be the
//! driver branching on untrusted bytes, whatever the rule said.
//!
//! # What an allow rule grants, and what it deliberately does not
//!
//! It answers a prompt, and that is all. It does **not** make a command's output trusted.
//! Pressing `a` at a run prompt grants those two things together, because a person looking at one
//! command can be asked to answer for both; a pattern like `curl *` covers commands nobody has
//! read, so it cannot carry the second claim. An allow rule that trusted output would let one
//! line in a settings file turn fetched bytes into routing, which is the whole thing the labels
//! exist to stop. So output keeps the label it would have had, and a rule only stops the asking.
//!
//! For the same reason an allow rule never answers the confidentiality question: a run that
//! releases the user's private data asks whatever the rules say, because vouching for a command
//! is not consenting to hand it that data.
//!
//! # Nothing here widens reach
//!
//! Rules decide what is asked about, never what is reachable. A path outside the workspace and
//! the directories the user named is refused because it is out of reach, and no allow rule brings
//! it back: `additionalDirectories` is what makes a directory reachable, and it is a separate
//! statement.

use std::fmt;

/// The family of tools a rule names.
///
/// Claude Code's own spellings, which are categories rather than tool names there too: its docs
/// have `Edit(...)` covering every tool that edits a file and reject a path rule written for
/// `Write` or `Glob`. So these are not this agent's tool names, and `Bash` in particular names no
/// shell: the planner has none, and a rule matches the argv of a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    /// Every tool that reads a file or lists one: `read_file`, `list_files`, `search`.
    Read,
    /// Every tool that changes a file: `write_file`, `edit_file`.
    Edit,
    /// Running a program: every stage of a `run` pipeline.
    Bash,
}

impl Subject {
    /// The spelling accepted in a settings file.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "Read",
            Self::Edit => "Edit",
            Self::Bash => "Bash",
        }
    }

    /// The name in a rule, or `None` for anything this agent has no family for.
    fn parse(name: &str) -> Option<Self> {
        match name {
            "Read" => Some(Self::Read),
            "Edit" => Some(Self::Edit),
            "Bash" => Some(Self::Bash),
            _ => None,
        }
    }

    /// Whether a specifier for this family is a path pattern rather than a command pattern.
    fn takes_a_path(self) -> bool {
        matches!(self, Self::Read | Self::Edit)
    }
}

impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which list a rule came from, which is what it does when it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ruling {
    /// The action is refused outright.
    Deny,
    /// A person is asked, whatever else would have answered.
    Ask,
    /// No prompt.
    Allow,
}

impl Ruling {
    /// The list's own name, for the audit trail.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Ask => "ask",
            Self::Allow => "allow",
        }
    }
}

impl fmt::Display for Ruling {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the rules had to say about one action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// A rule matched.
    Ruled(Ruling),
    /// Nothing matched, so the rules decide nothing and the ordinary gates do.
    Unmatched,
}

/// Where a relative pattern in a rule is anchored.
///
/// Supplied by the caller because resolving `~` and the directory a settings file sits in is I/O,
/// and this crate does none. Absent entries make the patterns that need them match nothing, which
/// is what a machine with no home directory should get.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Anchors {
    /// The user's home directory, for a `~/` pattern.
    pub home: Option<String>,
    /// The directory the settings file sits in, for a single-slash pattern.
    pub settings_dir: Option<String>,
}

impl Anchors {
    /// Anchors that resolve nothing, for a caller with no settings file.
    pub fn none() -> Self {
        Self::default()
    }
}

/// What a specifier matches against.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pattern {
    /// Every use of the family, from a bare `Bash` or a `Bash(*)`.
    Everything,
    /// A path pattern, gitignore-shaped, against a workspace-relative path.
    Relative(PathPattern),
    /// A path pattern against an absolute path.
    Absolute(PathPattern),
    /// A command pattern, matched against one stage's argv rendered as a line.
    Command(String),
}

/// A path pattern and whether it was written anchored.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PathPattern {
    /// Segments, `**` and `*` intact.
    segments: Vec<String>,
    /// Whether the pattern names a single leading segment and so may float in a deny or ask rule.
    ///
    /// Claude Code's asymmetry: `Read(secrets/**)` as a deny rule catches a `secrets` directory at
    /// any depth, and the same pattern as an allow rule grants only the one at the top. A rule
    /// that restricts should cover the nested copy; a rule that grants should cover what it names.
    floats_when_restricting: bool,
}

/// One rule: a family, and what of it the rule is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    subject: Subject,
    pattern: Pattern,
}

impl Rule {
    /// Read one rule, or say why it was not one.
    ///
    /// A rule nobody can act on is dropped rather than guessed at, and the text says which it was
    /// so `doctor` can name it. Guessing would be worse than ignoring: a misread deny rule reads
    /// as protection that is not there.
    pub fn parse(text: &str, anchors: &Anchors) -> Result<Self, Rejected> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Rejected::new(text, "is empty"));
        }

        let (name, specifier) = match text.split_once('(') {
            None => (text, None),
            Some((name, rest)) => match rest.strip_suffix(')') {
                None => return Err(Rejected::new(text, "is missing its closing bracket")),
                Some(specifier) => (name.trim_end(), Some(specifier.trim())),
            },
        };

        let Some(subject) = Subject::parse(name) else {
            return Err(Rejected::new(
                text,
                "names no family of tools this agent has; use Read, Edit or Bash",
            ));
        };

        let pattern = match specifier {
            // A bare family name, and `(*)`, are the same rule and cover every use of it.
            None | Some("*") => Pattern::Everything,
            Some("") => {
                return Err(Rejected::new(
                    text,
                    "has empty brackets; drop them to mean every use",
                ));
            }
            Some(specifier) if subject.takes_a_path() => path_pattern(specifier, anchors)
                .ok_or_else(|| {
                    Rejected::new(
                        text,
                        "needs a home directory or a settings directory to say where it points",
                    )
                })?,
            Some(specifier) => Pattern::Command(command_pattern(specifier)),
        };

        Ok(Self { subject, pattern })
    }

    pub fn subject(&self) -> Subject {
        self.subject
    }

    /// Whether this rule covers reading or editing `path`.
    ///
    /// `path` is workspace-relative or absolute, as the gates hold it. `restricting` selects the
    /// depth a single-segment pattern is matched at, which differs between the lists.
    fn covers_path(&self, path: &str, restricting: bool) -> bool {
        match &self.pattern {
            Pattern::Everything => true,
            Pattern::Command(_) => false,
            Pattern::Relative(pattern) => {
                !is_absolute(path) && pattern.matches(&segments_of(path), restricting)
            }
            Pattern::Absolute(pattern) => {
                is_absolute(path) && pattern.matches(&segments_of(path), restricting)
            }
        }
    }

    /// Whether this rule covers running `command`, one stage rendered as a line.
    fn covers_command(&self, command: &str) -> bool {
        match &self.pattern {
            Pattern::Everything => true,
            Pattern::Command(pattern) => command_matches(pattern, command),
            Pattern::Relative(_) | Pattern::Absolute(_) => false,
        }
    }
}

/// A rule that was not one, and what was wrong with it.
///
/// Carries the text so a report can name it. The text came from the user's own settings file, so
/// there is nothing untrusted in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub text: String,
    pub reason: &'static str,
}

impl Rejected {
    fn new(text: &str, reason: &'static str) -> Self {
        Self {
            text: text.to_string(),
            reason,
        }
    }
}

impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}' {}", self.text, self.reason)
    }
}

/// The three lists, and the directories a settings file made reachable.
///
/// Empty is the state every session starts in and means the rules decide nothing at all: every
/// gate behaves exactly as it did before a settings file existed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Permissions {
    deny: Vec<Rule>,
    ask: Vec<Rule>,
    allow: Vec<Rule>,
}

impl Permissions {
    /// No rules.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read three lists of rule text, keeping what parsed and reporting what did not.
    ///
    /// A file with one unreadable rule still gets the rest. Refusing the lot would mean a typo in
    /// an allow rule took away a deny rule's protection, which is the wrong way round.
    pub fn parse(
        deny: &[String],
        ask: &[String],
        allow: &[String],
        anchors: &Anchors,
    ) -> (Self, Vec<Rejected>) {
        let mut rejected = Vec::new();
        let mut read = |texts: &[String]| -> Vec<Rule> {
            texts
                .iter()
                .filter_map(|text| match Rule::parse(text, anchors) {
                    Ok(rule) => Some(rule),
                    Err(problem) => {
                        rejected.push(problem);
                        None
                    }
                })
                .collect()
        };
        let permissions = Self {
            deny: read(deny),
            ask: read(ask),
            allow: read(allow),
        };
        (permissions, rejected)
    }

    /// Whether any rule was written at all.
    pub fn is_empty(&self) -> bool {
        self.deny.is_empty() && self.ask.is_empty() && self.allow.is_empty()
    }

    /// How many rules are in force, for a report.
    pub fn len(&self) -> usize {
        self.deny.len() + self.ask.len() + self.allow.len()
    }

    /// What the rules say about reading or editing `path`.
    pub fn for_path(&self, subject: Subject, path: &str) -> Decision {
        self.decide(|rule, restricting| {
            rule.subject == subject && rule.covers_path(path, restricting)
        })
    }

    /// What the rules say about running one stage, rendered as a command line.
    pub fn for_command(&self, command: &str) -> Decision {
        self.decide(|rule, _| rule.subject == Subject::Bash && rule.covers_command(command))
    }

    /// What the rules say about running a whole pipeline.
    ///
    /// Each stage is judged on its own and the strictest answer wins, which is the same rule
    /// Claude Code applies to a command joined by `&&` or `|`: a rule must match every part for
    /// the whole to be allowed, and matching any part is enough to restrict it. A pipeline is that
    /// shape by construction here, so there is no command string to split and no chance of
    /// splitting it differently from the shell.
    ///
    /// An empty pipeline is unmatched: there is nothing to have an opinion about.
    pub fn for_pipeline(&self, commands: &[String]) -> Decision {
        if commands.is_empty() {
            return Decision::Unmatched;
        }
        let each: Vec<Decision> = commands
            .iter()
            .map(|command| self.for_command(command))
            .collect();

        // Restricting any stage restricts the pipeline: an unwanted program in the middle is
        // still an unwanted program.
        for ruling in [Ruling::Deny, Ruling::Ask] {
            if each.contains(&Decision::Ruled(ruling)) {
                return Decision::Ruled(ruling);
            }
        }
        // Granting needs every stage granted. One stage nobody wrote a rule for is a program
        // nobody has answered for, and it is what the next stage reads.
        if each.iter().all(|d| *d == Decision::Ruled(Ruling::Allow)) {
            return Decision::Ruled(Ruling::Allow);
        }
        Decision::Unmatched
    }

    /// Deny, then ask, then allow, first match wins.
    ///
    /// The order is the whole of the precedence, and specificity does not enter into it: a broad
    /// deny beats a narrow allow, so a deny rule cannot carry exceptions. That is what makes a
    /// deny rule readable as a statement about what will not happen.
    fn decide(&self, matches: impl Fn(&Rule, bool) -> bool) -> Decision {
        for (rules, ruling, restricting) in [
            (&self.deny, Ruling::Deny, true),
            (&self.ask, Ruling::Ask, true),
            (&self.allow, Ruling::Allow, false),
        ] {
            if rules.iter().any(|rule| matches(rule, restricting)) {
                return Decision::Ruled(ruling);
            }
        }
        Decision::Unmatched
    }
}

/// Read a path specifier, resolving the anchor it was written with.
///
/// The four shapes Claude Code has, which differ only in where they start from:
/// `//x` the filesystem root, `~/x` the home directory, `/x` the directory the settings file sits
/// in, and `x` or `./x` the workspace.
fn path_pattern(specifier: &str, anchors: &Anchors) -> Option<Pattern> {
    if let Some(rest) = specifier.strip_prefix("//") {
        return Some(Pattern::Absolute(PathPattern::rooted(rest)));
    }
    if let Some(rest) = specifier.strip_prefix("~/") {
        let home = anchors.home.as_deref()?;
        return Some(Pattern::Absolute(PathPattern::rooted(&join(home, rest))));
    }
    if let Some(rest) = specifier.strip_prefix('/') {
        let base = anchors.settings_dir.as_deref()?;
        return Some(Pattern::Absolute(PathPattern::rooted(&join(base, rest))));
    }
    let rest = specifier.strip_prefix("./").unwrap_or(specifier);
    Some(Pattern::Relative(PathPattern::relative(rest)))
}

/// Join two path pieces with a single slash, whatever slashes they came with.
fn join(base: &str, rest: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        rest.trim_start_matches('/')
    )
}

impl PathPattern {
    /// A pattern that starts where it says it starts, so it never floats.
    fn rooted(pattern: &str) -> Self {
        Self {
            segments: split(pattern),
            floats_when_restricting: false,
        }
    }

    /// A workspace-relative pattern.
    ///
    /// Two gitignore properties decide the depth. A pattern with no slash in it is a name and
    /// matches at any depth, in every list, so `Read(.env)` and `Read(**/.env)` are one rule. A
    /// pattern whose first segment is a plain name and which has more after it is the case Claude
    /// Code treats asymmetrically, and `floats_when_restricting` carries that.
    fn relative(pattern: &str) -> Self {
        let segments = split(pattern);
        let is_a_bare_name = segments.len() == 1;
        let starts_at_a_named_segment = segments
            .first()
            .is_some_and(|first| first != "**" && !first.contains('*'));
        Self {
            floats_when_restricting: !is_a_bare_name && starts_at_a_named_segment,
            segments: if is_a_bare_name {
                // A name matches at any depth, which is a leading `**` and nothing else.
                let mut floated = vec!["**".to_string()];
                floated.extend(segments);
                floated
            } else {
                segments
            },
        }
    }

    /// Whether this pattern covers `path`, already split into segments.
    fn matches(&self, path: &[&str], restricting: bool) -> bool {
        if segments_match(&self.segments, path) {
            return true;
        }
        // A restricting rule also catches the nested copy of a directory it named.
        if restricting && self.floats_when_restricting {
            let mut floated = vec!["**".to_string()];
            floated.extend(self.segments.iter().cloned());
            return segments_match(&floated, path);
        }
        false
    }
}

/// Split a path or pattern into segments, dropping the empties a leading or doubled slash leaves.
fn split(text: &str) -> Vec<String> {
    text.split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .map(str::to_string)
        .collect()
}

/// The same, borrowing, for the path being tested.
fn segments_of(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect()
}

fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
}

/// Whether a pattern's segments cover a path's, with `**` crossing directories and `*` not.
///
/// Iterative, with one remembered `**` to fall back to, so a pattern full of stars costs time in
/// the length of the path rather than exponentially in the number of them. A pattern arrives from
/// a settings file, which is the user's own, but a matcher that can be made to hang is worth not
/// writing whoever supplies the input.
///
/// A trailing `**` matches the directory it hangs off as well as everything under it, which is
/// what makes `Edit(src/**)` cover `src` itself.
fn segments_match(pattern: &[String], path: &[&str]) -> bool {
    let mut p = 0;
    let mut s = 0;
    // Where to resume from if a `**` turns out to have swallowed too little.
    let mut star: Option<(usize, usize)> = None;

    while s < path.len() {
        match pattern.get(p) {
            Some(segment) if segment == "**" => {
                star = Some((p, s));
                p += 1;
            }
            Some(segment) if wildcard_matches(segment, path[s]) => {
                p += 1;
                s += 1;
            }
            _ => match star {
                // Let the last `**` take one more segment and try again.
                Some((star_p, star_s)) => {
                    p = star_p + 1;
                    s = star_s + 1;
                    star = Some((star_p, s));
                }
                None => return false,
            },
        }
    }

    // Trailing `**`s match the nothing that is left, which is what makes `src/**` cover `src`.
    pattern[p..].iter().all(|segment| segment == "**")
}

/// Whether one pattern segment covers one path segment, `*` standing in for any text within it.
///
/// Two pointers with a single remembered star, for the same reason as [`segments_match`]: no
/// recursion, and no pattern that costs more than the product of the two lengths.
fn wildcard_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let mut p = 0;
    let mut t = 0;
    let mut star: Option<(usize, usize)> = None;

    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, t));
                p += 1;
            }
            Some(c) if *c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((star_p, star_t)) => {
                    p = star_p + 1;
                    t = star_t + 1;
                    star = Some((star_p, t));
                }
                None => return false,
            },
        }
    }

    pattern[p..].iter().all(|c| *c == '*')
}

/// Read a command specifier, normalising the one spelling that is a synonym.
///
/// `ls:*` is Claude Code's other way of writing a trailing wildcard, recognised only at the end:
/// a colon anywhere else is a literal, so `git:* push` matches a command with a colon in it and
/// not a git subcommand.
fn command_pattern(specifier: &str) -> String {
    match specifier.strip_suffix(":*") {
        Some(head) => format!("{head} *"),
        None => specifier.to_string(),
    }
}

/// Whether a command pattern covers one command line.
///
/// A trailing ` *` also matches the bare command, so `Bash(ls *)` covers `ls`. That holds only
/// when the trailing star is the pattern's only one, which is what separates `Bash(ls *)` from
/// `Bash(* --help *)`: the second says there is an argument, the first says there may be.
///
/// The space before a trailing star is part of the pattern. `Bash(ls *)` does not match `lsof`,
/// and `Bash(ls*)` does, which is the difference between naming a command and naming a prefix.
fn command_matches(pattern: &str, command: &str) -> bool {
    if let Some(head) = pattern.strip_suffix(" *")
        && !head.contains('*')
        && head == command
    {
        return true;
    }
    wildcard_matches(pattern, command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchors() -> Anchors {
        Anchors {
            home: Some("/home/someone".to_string()),
            settings_dir: Some("/home/someone/.bravebot".to_string()),
        }
    }

    fn rules(deny: &[&str], ask: &[&str], allow: &[&str]) -> Permissions {
        let owned = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (permissions, rejected) =
            Permissions::parse(&owned(deny), &owned(ask), &owned(allow), &anchors());
        assert_eq!(rejected, Vec::new(), "a rule in this test did not parse");
        permissions
    }

    /// The point of the whole module: the block in Claude Code's own documentation governs this
    /// agent without being rewritten first.
    #[test]
    fn a_block_copied_from_claude_code_is_read() {
        let permissions = rules(
            &["Read(./.env)", "Read(./.env.*)"],
            &["Bash(git push *)"],
            &["Bash(git diff *)", "Bash(npm test *)"],
        );
        assert_eq!(
            permissions.for_path(Subject::Read, ".env"),
            Decision::Ruled(Ruling::Deny)
        );
        assert_eq!(
            permissions.for_path(Subject::Read, ".env.local"),
            Decision::Ruled(Ruling::Deny)
        );
        assert_eq!(
            permissions.for_command("git push origin main"),
            Decision::Ruled(Ruling::Ask)
        );
        assert_eq!(
            permissions.for_command("git diff --stat"),
            Decision::Ruled(Ruling::Allow)
        );
        assert_eq!(permissions.for_command("rm -rf /"), Decision::Unmatched);
    }

    /// No rules must mean no change to anything, or adding the feature would have altered every
    /// session that never asked for it.
    #[test]
    fn no_rules_decide_nothing() {
        let permissions = Permissions::new();
        assert!(permissions.is_empty());
        assert_eq!(
            permissions.for_path(Subject::Read, "src/main.rs"),
            Decision::Unmatched
        );
        assert_eq!(permissions.for_command("ls"), Decision::Unmatched);
    }

    /// The precedence is the whole of the rule, and specificity is not part of it. A deny rule
    /// that could be narrowed by an allow rule would not be readable as a statement about what
    /// will not happen.
    #[test]
    fn deny_beats_ask_and_ask_beats_allow_however_specific_the_loser() {
        let permissions = rules(&["Bash(aws *)"], &[], &["Bash(aws s3 ls)"]);
        assert_eq!(
            permissions.for_command("aws s3 ls"),
            Decision::Ruled(Ruling::Deny)
        );

        let permissions = rules(&[], &["Bash(git push *)"], &["Bash(git push origin main)"]);
        assert_eq!(
            permissions.for_command("git push origin main"),
            Decision::Ruled(Ruling::Ask)
        );
    }

    /// A bare family name and `(*)` are the same rule, and both cover every use.
    #[test]
    fn a_bare_family_name_covers_every_use_of_it() {
        for text in ["Bash", "Bash(*)"] {
            let permissions = rules(&[text], &[], &[]);
            assert_eq!(
                permissions.for_command("anything at all"),
                Decision::Ruled(Ruling::Deny),
                "{text} did not cover every command"
            );
        }
        let permissions = rules(&["Read"], &[], &[]);
        assert_eq!(
            permissions.for_path(Subject::Read, "src/main.rs"),
            Decision::Ruled(Ruling::Deny)
        );
        // A family names one family. Denying reads says nothing about writes.
        assert_eq!(
            permissions.for_path(Subject::Edit, "src/main.rs"),
            Decision::Unmatched
        );
    }

    /// The table in Claude Code's documentation, which is the specification of where a `*` goes
    /// and what it stands in for.
    #[test]
    fn a_command_pattern_matches_where_the_documented_table_says() {
        let cases: &[(&str, &[&str], &[&str])] = &[
            (
                "Bash(npm run build)",
                &["npm run build"],
                &["npm run build --watch"],
            ),
            (
                "Bash(npm run *)",
                &["npm run build", "npm run test --watch", "npm run"],
                &["npm install"],
            ),
            (
                "Bash(git log * main)",
                &["git log --oneline main", "git log -5 main"],
                &["git log main", "git push origin main"],
            ),
            (
                "Bash(git * main)",
                &["git merge main", "git push origin main"],
                &["git log"],
            ),
            ("Bash(* --version)", &["node --version"], &["node -v"]),
            ("Bash(ls *)", &["ls -la", "ls"], &["lsof"]),
            ("Bash(ls*)", &["ls -la", "lsof"], &[]),
            ("Bash(* --help *)", &["npm --help x"], &["npm --help"]),
        ];
        for (rule, matching, not_matching) in cases {
            let permissions = rules(&[], &[], &[rule]);
            for command in *matching {
                assert_eq!(
                    permissions.for_command(command),
                    Decision::Ruled(Ruling::Allow),
                    "{rule} should match {command:?}"
                );
            }
            for command in *not_matching {
                assert_eq!(
                    permissions.for_command(command),
                    Decision::Unmatched,
                    "{rule} should not match {command:?}"
                );
            }
        }
    }

    /// The other spelling of a trailing wildcard, and the reason it is only read at the end: a
    /// colon in the middle is a character in a command, not a wildcard.
    #[test]
    fn a_trailing_colon_star_is_a_trailing_wildcard_and_a_colon_elsewhere_is_not() {
        let permissions = rules(&[], &[], &["Bash(ls:*)"]);
        for command in ["ls", "ls -la"] {
            assert_eq!(
                permissions.for_command(command),
                Decision::Ruled(Ruling::Allow),
                "{command} did not match"
            );
        }

        let permissions = rules(&[], &[], &["Bash(git:* push)"]);
        assert_eq!(permissions.for_command("git push"), Decision::Unmatched);
        assert_eq!(
            permissions.for_command("git:anything push"),
            Decision::Ruled(Ruling::Allow)
        );
    }

    /// Gitignore semantics: a specifier with no slash in it is a name and matches wherever it
    /// turns up, so the two spellings are one rule.
    #[test]
    fn a_bare_name_matches_at_any_depth_in_every_list() {
        for text in ["Read(.env)", "Read(**/.env)"] {
            let permissions = rules(&[text], &[], &[]);
            for path in [".env", "src/.env", "a/b/c/.env"] {
                assert_eq!(
                    permissions.for_path(Subject::Read, path),
                    Decision::Ruled(Ruling::Deny),
                    "{text} should cover {path}"
                );
            }
            assert_eq!(
                permissions.for_path(Subject::Read, "env"),
                Decision::Unmatched,
                "{text} should not cover a different name"
            );
        }
    }

    /// The documented asymmetry, and the reason for it: a rule that restricts should cover the
    /// nested copy of what it named, and a rule that grants should cover what it named.
    #[test]
    fn a_single_segment_directory_floats_when_it_restricts_and_not_when_it_grants() {
        let denying = rules(&["Edit(src/**)"], &[], &[]);
        assert_eq!(
            denying.for_path(Subject::Edit, "src/app.ts"),
            Decision::Ruled(Ruling::Deny)
        );
        assert_eq!(
            denying.for_path(Subject::Edit, "vendor/pkg/src/lib.js"),
            Decision::Ruled(Ruling::Deny)
        );

        let allowing = rules(&[], &[], &["Edit(src/**)"]);
        assert_eq!(
            allowing.for_path(Subject::Edit, "src/app.ts"),
            Decision::Ruled(Ruling::Allow)
        );
        assert_eq!(
            allowing.for_path(Subject::Edit, "vendor/pkg/src/lib.js"),
            Decision::Unmatched
        );
    }

    /// An anchored pattern means the place it names, in every list, which is how somebody pins a
    /// rule to one directory when the floating kind would have caught more.
    #[test]
    fn an_anchored_pattern_matches_only_where_it_is_anchored() {
        for list in 0..3 {
            let rule = "Edit(/src/**)";
            let permissions = match list {
                0 => rules(&[rule], &[], &[]),
                1 => rules(&[], &[rule], &[]),
                _ => rules(&[], &[], &[rule]),
            };
            // A single slash anchors at the settings file's own directory, which is not the
            // workspace: this is the trap Claude Code's documentation warns about.
            assert_eq!(
                permissions.for_path(Subject::Edit, "src/app.ts"),
                Decision::Unmatched,
                "list {list}: a settings-anchored rule matched a workspace path"
            );
            assert_eq!(
                permissions.for_path(Subject::Edit, "/home/someone/.bravebot/src/app.ts"),
                Decision::Ruled(match list {
                    0 => Ruling::Deny,
                    1 => Ruling::Ask,
                    _ => Ruling::Allow,
                }),
                "list {list}: a settings-anchored rule missed its own directory"
            );
        }
    }

    /// The four anchors, each pointing where its own leader says.
    #[test]
    fn each_anchor_points_where_its_leader_says() {
        let permissions = rules(
            &[
                "Read(//etc/shadow)",
                "Read(~/.ssh/**)",
                "Read(/kept/**)",
                "Read(./local/**)",
            ],
            &[],
            &[],
        );
        for path in [
            "/etc/shadow",
            "/home/someone/.ssh/id_rsa",
            "/home/someone/.bravebot/kept/thing",
            "local/thing",
        ] {
            assert_eq!(
                permissions.for_path(Subject::Read, path),
                Decision::Ruled(Ruling::Deny),
                "{path} was not covered"
            );
        }
        // A single leading slash is not the filesystem root, which is the documented trap.
        assert_eq!(
            permissions.for_path(Subject::Read, "/kept/thing"),
            Decision::Unmatched
        );
    }

    /// A relative rule and an absolute one are about different paths, and neither reaches into
    /// the other's namespace. The trust map keeps the same separation for the same reason.
    #[test]
    fn a_relative_rule_says_nothing_about_an_absolute_path() {
        let permissions = rules(&["Read(secrets/**)"], &[], &[]);
        assert_eq!(
            permissions.for_path(Subject::Read, "secrets/key"),
            Decision::Ruled(Ruling::Deny)
        );
        assert_eq!(
            permissions.for_path(Subject::Read, "/secrets/key"),
            Decision::Unmatched
        );
    }

    /// `*` stays inside a segment and `**` crosses them. Without that a rule about one directory
    /// would quietly cover the tree beneath it.
    #[test]
    fn one_star_stays_in_a_segment_and_two_cross_them() {
        let permissions = rules(&["Read(/*.pdf)"], &[], &[]);
        assert_eq!(
            permissions.for_path(Subject::Read, "/home/someone/.bravebot/notes.pdf"),
            Decision::Ruled(Ruling::Deny)
        );
        assert_eq!(
            permissions.for_path(Subject::Read, "/home/someone/.bravebot/deep/notes.pdf"),
            Decision::Unmatched
        );

        let permissions = rules(&["Read(/**/*.pdf)"], &[], &[]);
        assert_eq!(
            permissions.for_path(Subject::Read, "/home/someone/.bravebot/deep/notes.pdf"),
            Decision::Ruled(Ruling::Deny)
        );
    }

    /// A trailing `/**` covers the directory it hangs off, not only what is under it, so a rule
    /// about a tree includes the tree's own name.
    #[test]
    fn a_trailing_double_star_covers_the_directory_it_names() {
        let permissions = rules(&["Read(//tmp/scratch/**)"], &[], &[]);
        for path in ["/tmp/scratch", "/tmp/scratch/a", "/tmp/scratch/a/b"] {
            assert_eq!(
                permissions.for_path(Subject::Read, path),
                Decision::Ruled(Ruling::Deny),
                "{path} was not covered"
            );
        }
        // And not a sibling whose name merely starts the same way.
        assert_eq!(
            permissions.for_path(Subject::Read, "/tmp/scratchpad"),
            Decision::Unmatched
        );
    }

    /// A family names one family. Reads and edits are separate questions, and a rule about one
    /// must not answer the other.
    #[test]
    fn a_rule_for_one_family_does_not_decide_another() {
        let permissions = rules(&[], &[], &["Edit(src/**)"]);
        assert_eq!(
            permissions.for_path(Subject::Edit, "src/main.rs"),
            Decision::Ruled(Ruling::Allow)
        );
        assert_eq!(
            permissions.for_path(Subject::Read, "src/main.rs"),
            Decision::Unmatched
        );
        // And a path rule is not a command rule, whatever it looks like.
        assert_eq!(permissions.for_command("src/main.rs"), Decision::Unmatched);
    }

    /// A rule nobody can act on is dropped and named, never guessed at. A misread deny rule
    /// would read as protection that is not there.
    #[test]
    fn a_rule_that_cannot_be_read_is_dropped_and_reported() {
        let texts: Vec<String> = [
            "Bash(git diff",
            "WebFetch(domain:example.com)",
            "Write(src/**)",
            "Bash()",
            "",
            "   ",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (permissions, rejected) = Permissions::parse(&texts, &[], &[], &anchors());
        assert!(permissions.is_empty(), "an unreadable rule was kept");
        assert_eq!(rejected.len(), texts.len());
    }

    /// One bad rule must not take the others down with it, or a typo in an allow rule would
    /// quietly remove a deny rule's protection.
    #[test]
    fn one_unreadable_rule_does_not_discard_the_others() {
        let texts: Vec<String> = ["Read(.env)", "Nonsense(x)", "Read(.pem)"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (permissions, rejected) = Permissions::parse(&texts, &[], &[], &anchors());
        assert_eq!(permissions.len(), 2);
        assert_eq!(rejected.len(), 1);
        assert_eq!(
            permissions.for_path(Subject::Read, ".env"),
            Decision::Ruled(Ruling::Deny)
        );
    }

    /// A machine with no home directory cannot say where `~/` points, and a rule that silently
    /// matched nothing under those circumstances would be worse than one reported as unusable.
    #[test]
    fn a_pattern_whose_anchor_is_unknown_is_reported_rather_than_matching_nothing() {
        let (permissions, rejected) = Permissions::parse(
            &["Read(~/.ssh/**)".to_string(), "Read(/kept/**)".to_string()],
            &[],
            &[],
            &Anchors::none(),
        );
        assert!(permissions.is_empty());
        assert_eq!(rejected.len(), 2);
    }

    /// A rule must cover every stage for a pipeline to be allowed. An unvouched stage in the
    /// middle is a transformation nobody answered for, and its output is what the next stage
    /// reads.
    #[test]
    fn a_pipeline_is_allowed_only_when_every_stage_is() {
        let permissions = rules(&[], &[], &["Bash(git log *)", "Bash(sed *)"]);
        let allowed = ["git log --oneline".to_string(), "sed -n 1,10p".to_string()];
        assert_eq!(
            permissions.for_pipeline(&allowed),
            Decision::Ruled(Ruling::Allow)
        );

        let one_unruled = [
            "git log --oneline".to_string(),
            "curl example.com".to_string(),
        ];
        assert_eq!(permissions.for_pipeline(&one_unruled), Decision::Unmatched);
    }

    /// The other half: restricting one stage restricts the pipeline, since an unwanted program in
    /// the middle is still an unwanted program.
    #[test]
    fn restricting_one_stage_restricts_the_whole_pipeline() {
        let permissions = rules(&["Bash(curl *)"], &[], &["Bash(git log *)"]);
        let stages = [
            "git log --oneline".to_string(),
            "curl example.com".to_string(),
        ];
        assert_eq!(
            permissions.for_pipeline(&stages),
            Decision::Ruled(Ruling::Deny)
        );

        let permissions = rules(&[], &["Bash(curl *)"], &["Bash(git log *)"]);
        assert_eq!(
            permissions.for_pipeline(&stages),
            Decision::Ruled(Ruling::Ask)
        );
    }

    /// A pattern of nothing but wildcards must cost time in the length of what it is matched
    /// against, not exponentially in the number of stars. The matcher is iterative for this.
    #[test]
    fn a_pattern_full_of_wildcards_still_finishes() {
        let pattern = format!("Bash({}b)", "*a".repeat(24));
        let permissions = rules(&[], &[], &[&pattern]);
        let command = "a".repeat(2048);
        assert_eq!(permissions.for_command(&command), Decision::Unmatched);
    }
}
