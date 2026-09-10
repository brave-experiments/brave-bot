//! Whether a newer bravebot has been published, and the command that takes it.
//!
//! No model, no turn, and no token: a version number, asked of the registry this copy was
//! installed from and compared against the one compiled in.
//!
//! # Startup waits for none of it
//!
//! The line shown at startup comes out of `~/.bravebot`, where an earlier launch wrote the answer
//! down. Asking again happens on a thread nothing joins, at most once a day, and what it learns is
//! for the next launch. Somebody opening a session on a train waits for nothing and, with no
//! answer on disk yet, is told nothing.
//!
//! # Only an installation there is a command for
//!
//! Two ways of installing bravebot have an update command this program can name: the npm package,
//! whose launcher says which it is, and the install script, which writes down where it put the
//! binary. Everything else, a build from source above all, is left alone. Telling somebody who
//! compiled this that a release exists, without a command that would install it, is noise.
//!
//! # What the fetched number decides
//!
//! Whether one line is printed, and nothing else. It reaches no request field, no turn and no
//! planner's context. The version in that line is composed from the three numbers parsed out of
//! the answer rather than from the answer's own bytes, so nothing a registry says is quoted back
//! onto somebody's screen.

use bravebot_core::capability::{Capability, CapabilitySet};
use bravebot_core::label::Label;
use bravebot_core::policy::{Policy, ReleasePlan, Routing};
use bravebot_i18n::t;
use bravebot_net::{Egress, Request};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::audit::Trail;

/// How this copy was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Install {
    /// The npm package. Its launcher runs this binary and says so.
    Npm,
    /// The install script, which recorded where it put the binary.
    Script,
}

/// What the npm launcher sets [`bravebot_config::env_var::INSTALLED_VIA`] to.
const NPM: &str = "npm";

/// The registry entry for the npm package, which answers with the published version.
const NPM_URL: &str = "https://registry.npmjs.org/@brave/bravebot/latest";

/// The newest release of this repository, which answers with the tag it was published under.
const RELEASES_URL: &str =
    "https://api.github.com/repos/brave-experiments/bravebot/releases/latest";

/// What updates an npm install.
const NPM_COMMAND: &str = "npm install -g @brave/bravebot@latest";

/// What updates a script install: the same line that installed it, which takes the newest release
/// and puts it where this one already is.
const SCRIPT_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/brave-experiments/bravebot/main/install.sh | sh";

/// The file the install script writes the installed path into.
const INSTALLED_BY_FILE: &str = "installed-by";

/// Where the answer to the last ask is kept.
const CACHE_FILE: &str = "update-check";

/// Written here and renamed, so an interrupted write cannot leave half a line to be read back.
const CACHE_TEMPORARY: &str = "update-check.tmp";

/// How long an answer stands before it is worth asking again.
///
/// Releases happen on the order of days, and the cost of a stale answer is being told about an
/// update one launch later than it existed. The cost of asking on every launch is a request per
/// launch to somebody else's registry, for a number that will not have changed.
const GOOD_FOR: u64 = 24 * 60 * 60;

impl Install {
    /// The command that updates a copy installed this way.
    ///
    /// A literal per installation, never composed: this is a line somebody pastes into a shell,
    /// so no part of it comes from a file, an environment variable or a response.
    pub fn update_command(self) -> &'static str {
        match self {
            Self::Npm => NPM_COMMAND,
            Self::Script => SCRIPT_COMMAND,
        }
    }

    /// Who is asked what the newest version is.
    fn url(self) -> &'static str {
        match self {
            Self::Npm => NPM_URL,
            Self::Script => RELEASES_URL,
        }
    }

    /// The field of that answer holding the version.
    fn field(self) -> &'static str {
        match self {
            Self::Npm => "version",
            Self::Script => "tag_name",
        }
    }

    /// What a stored answer records about where it came from.
    ///
    /// Kept in the file so an answer from one registry is not read as an answer from the other on
    /// a machine where both installations have existed.
    fn source(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Script => "releases",
        }
    }
}

/// A published version, as three numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// An answer as it was stored: what was newest, and when that was true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Answer {
    at: u64,
    latest: Version,
}

/// The line to say at startup, having started the ask that answers the next launch.
///
/// Nothing waits on that ask. Whatever is said here was already on disk when the process started.
pub fn at_startup() -> Option<String> {
    let install = installed_how()?;
    let stored = stored_answer(install);
    refresh(install, stored.map(|answer| answer.at));
    line(install, running_version()?, stored?.latest)
}

/// What to say about a version that is out, or nothing when this copy is not behind it.
///
/// Separated from the files and the clock so what is said can be tested without either.
fn line(install: Install, running: Version, latest: Version) -> Option<String> {
    (latest > running).then(|| {
        t!(
            update_available,
            version = latest,
            running = running,
            command = install.update_command()
        )
    })
}

/// The version this binary was built as.
fn running_version() -> Option<Version> {
    parse_version(env!("CARGO_PKG_VERSION"))
}

/// How this copy was installed, or `None` for one nothing here can offer a command for.
fn installed_how() -> Option<Install> {
    method(
        std::env::var(bravebot_config::env_var::INSTALLED_VIA)
            .ok()
            .as_deref(),
        // Compared against the known install locations to report how bravebot was installed.
        // A wrong answer downgrades to "unknown"; nothing is granted on the strength of it.
        // nosemgrep: rust.lang.security.current-exe.current-exe
        std::env::current_exe().ok().as_deref(),
        recorded_install().as_deref(),
    )
}

/// Decide from the launcher's word and the recorded path, so neither the environment nor the
/// filesystem is needed to test it.
///
/// The recorded path has to be the binary that is actually running. A checkout built from source,
/// on a machine where the script installed a copy as well, is not a script install: it is a build
/// nothing here knows how to update, and telling its user to curl over the top of it would replace
/// somebody else's copy rather than theirs.
fn method(via: Option<&str>, running: Option<&Path>, recorded: Option<&Path>) -> Option<Install> {
    if via == Some(NPM) {
        return Some(Install::Npm);
    }
    match (running, recorded) {
        (Some(running), Some(recorded)) if same_file(running, recorded) => Some(Install::Script),
        _ => None,
    }
}

/// Whether two paths name the same binary, following symlinks where they resolve.
///
/// A directory on `PATH` is often a link, so the path a process was started by and the path the
/// script wrote down can spell the same file differently.
fn same_file(left: &Path, right: &Path) -> bool {
    let resolved = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    left == right || resolved(left) == resolved(right)
}

/// Where the install script says it put the binary.
fn recorded_install() -> Option<PathBuf> {
    let path = bravebot_agent::home::directory()?.join(INSTALLED_BY_FILE);
    let contents = std::fs::read_to_string(path).ok()?;
    let recorded = contents.lines().next()?.trim();
    (!recorded.is_empty()).then(|| PathBuf::from(recorded))
}

/// The answer an earlier launch wrote down, where there is one for this installation.
fn stored_answer(install: Install) -> Option<Answer> {
    let path = bravebot_agent::home::directory()?.join(CACHE_FILE);
    parse_answer(&std::fs::read_to_string(path).ok()?, install)
}

/// Read a stored answer out of the file's contents.
///
/// Separate from the I/O so the rules are testable, and there are three: the line is the shape
/// this program writes, it came from the registry this installation would ask, and the version on
/// it is three numbers. Anything else is no answer, which is silence rather than an error.
fn parse_answer(contents: &str, install: Install) -> Option<Answer> {
    let mut fields = contents.lines().next()?.splitn(3, '\t');
    let (Some(at), Some(source), Some(latest)) = (fields.next(), fields.next(), fields.next())
    else {
        return None;
    };
    if source != install.source() {
        return None;
    }
    Some(Answer {
        at: at.parse().ok()?,
        latest: parse_version(latest)?,
    })
}

/// One answer as the line it is stored as, without its newline.
fn encode_answer(install: Install, answer: Answer) -> String {
    format!("{}\t{}\t{}", answer.at, install.source(), answer.latest)
}

/// A version from `major.minor.patch`, with the `v` a tag carries allowed in front.
///
/// Three numbers and nothing else. A prerelease, a build suffix, or anything else that is not a
/// number in one of the three places is no answer at all, so a `0.6.0-rc.1` on the registry leaves
/// everybody where they were rather than sending them to a release candidate.
fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let number = |part: &str| {
        part.chars()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    Some(Version {
        major: number(major)?,
        minor: number(minor)?,
        patch: number(patch)?,
    })
}

/// Ask again, on a thread nothing joins, where the answer on disk is old enough to be worth it.
///
/// Nothing waits on the thread and nothing reads what it learns: it writes the answer down, and
/// the next launch is what says anything about it.
fn refresh(install: Install, asked_at: Option<u64>) {
    if !worth_asking(bravebot_agent::home::writable().is_some(), now(), asked_at) {
        return;
    }
    std::thread::spawn(move || {
        if let Some(latest) = ask(install) {
            store(install, Answer { at: now(), latest });
        }
    });
}

/// Whether to ask the registry again.
///
/// A session that may not write to `~/.bravebot` does not ask at all. An incognito session is that
/// case, and a request whose answer is thrown away is one this program made for nothing.
///
/// A stamp in the future is asked again rather than waited out: a clock that moved, or a file
/// somebody else wrote, must not leave this silent until the date catches up with the file.
fn worth_asking(recordable: bool, now: u64, asked_at: Option<u64>) -> bool {
    recordable
        && match asked_at {
            None => true,
            Some(at) => now.checked_sub(at).is_none_or(|age| age >= GOOD_FOR),
        }
}

/// Ask the registry this installation came from what the newest version is.
///
/// Through the egress gate like everything else, since that gate is the only way out. The
/// destination is one of two literals in this file, fixed before the request is made, so nothing
/// fetched has any say in where this goes.
fn ask(install: Install) -> Option<Version> {
    let mut sink = Trail::new();
    let egress = Egress::new();

    let mut routing = Routing::new();
    routing.insert_trusted("update", install.url());

    let mut policy = Policy::begin(
        routing,
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .ok()?;

    let request = Request::get(install.url())
        .header("accept", "application/json")
        // GitHub answers a request without one with 403, and a name is politer than a library's
        // default to a registry serving this for free.
        .header("user-agent", "bravebot");

    let response = egress
        .fetch(&mut policy, request, Label::untrusted_public())
        .ok()?;
    let (bytes, _) = response.body.into_parts_for_decoding();
    version_in(install, &bytes)
}

/// The version an answer states, or `None` for anything that is not the shape expected.
///
/// Separate from the request so both shapes can be read without a server.
fn version_in(install: Install, bytes: &[u8]) -> Option<Version> {
    let answer: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    parse_version(answer.get(install.field())?.as_str()?)
}

/// Write the answer down for the next launch.
///
/// Best effort throughout: a machine with no home, a read-only disk and an incognito session all
/// mean the next launch asks again, which is the same thing that happens on a first run.
fn store(install: Install, answer: Answer) {
    let Some(directory) = bravebot_agent::home::writable() else {
        return;
    };
    if bravebot_agent::home::create_directory(&directory).is_err() {
        return;
    }
    let temporary = directory.join(CACHE_TEMPORARY);
    let line = format!("{}\n", encode_answer(install, answer));
    if bravebot_agent::home::write_file(&temporary, line.as_bytes()).is_ok() {
        let _ = std::fs::rename(&temporary, directory.join(CACHE_FILE));
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    /// The whole point of the line is the command, and a person reading it should not have to work
    /// out which of the two installations they have.
    #[test]
    fn a_newer_version_is_named_along_with_the_command_that_installs_it() {
        let said = line(Install::Npm, version(0, 5, 0), version(0, 6, 1)).expect("a newer version");
        assert!(
            said.contains("0.6.1"),
            "the newer version is missing: {said}"
        );
        assert!(
            said.contains("0.5.0"),
            "the running version is missing: {said}"
        );
        assert!(
            said.contains("npm install -g @brave/bravebot@latest"),
            "the command is missing: {said}"
        );
    }

    /// A script install is told to run the script again, never the npm command, which would leave
    /// two copies on the machine and update the one that is not running.
    #[test]
    fn a_script_install_is_given_the_script_again() {
        let said =
            line(Install::Script, version(0, 5, 0), version(1, 0, 0)).expect("a newer version");
        assert!(
            said.contains("install.sh"),
            "the install script is missing: {said}"
        );
        assert!(!said.contains("npm install"), "the wrong command: {said}");
    }

    /// Saying nothing is the ordinary case: most launches are of the newest version, and a line
    /// about it every time is what makes people stop reading the startup screen.
    #[test]
    fn the_version_in_hand_is_not_an_update() {
        assert_eq!(line(Install::Npm, version(0, 5, 0), version(0, 5, 0)), None);
    }

    /// A registry that answers with something older, which is what a yanked release looks like,
    /// must not send anybody backwards.
    #[test]
    fn an_older_published_version_says_nothing() {
        assert_eq!(line(Install::Npm, version(0, 6, 0), version(0, 5, 9)), None);
    }

    /// Each part is compared as a number. Compared as text, 10 sorts before 9 and a release would
    /// go unannounced for as long as the numbering stayed in that decade.
    #[test]
    fn versions_are_ordered_by_number_rather_than_by_spelling() {
        assert!(version(0, 10, 0) > version(0, 9, 0));
        assert!(version(0, 5, 10) > version(0, 5, 9));
        assert!(version(1, 0, 0) > version(0, 999, 999));
    }

    #[test]
    fn a_tag_is_read_with_or_without_the_v_it_is_published_under() {
        assert_eq!(parse_version("v1.2.3"), Some(version(1, 2, 3)));
        assert_eq!(parse_version("1.2.3"), Some(version(1, 2, 3)));
    }

    /// Nobody is sent to a release candidate by a notice at startup, so a version that is not
    /// three plain numbers is not an answer.
    #[test]
    fn a_prerelease_is_not_a_version_to_offer() {
        assert_eq!(parse_version("0.6.0-rc.1"), None);
        assert_eq!(parse_version("0.6.0+build.7"), None);
    }

    #[test]
    fn a_version_that_is_not_three_numbers_is_no_answer() {
        for text in ["", "1", "1.2", "1.2.3.4", "one.two.three", "1.-2.3", " "] {
            assert_eq!(parse_version(text), None, "{text} was read as a version");
        }
    }

    /// The two registries answer in their own shapes, and each installation reads its own.
    #[test]
    fn the_registry_states_a_version_and_the_release_listing_states_a_tag() {
        let registry = br#"{"name":"@brave/bravebot","version":"0.7.2"}"#;
        let listing = br#"{"tag_name":"v0.7.2","name":"0.7.2"}"#;
        assert_eq!(
            version_in(Install::Npm, registry),
            Some(version(0, 7, 2)),
            "the registry answer was not read"
        );
        assert_eq!(
            version_in(Install::Script, listing),
            Some(version(0, 7, 2)),
            "the release listing was not read"
        );
    }

    /// The line is composed from the three numbers that were read, so nothing a registry sends is
    /// quoted back onto somebody's screen.
    #[test]
    fn a_version_is_said_as_the_numbers_read_rather_than_as_the_text_that_arrived() {
        let latest = version_in(Install::Npm, br#"{"version":"v0.06.0"}"#).expect("a version");
        let said = line(Install::Npm, version(0, 5, 0), latest).expect("a newer version");
        assert!(said.contains("0.6.0"), "{said}");
        assert!(
            !said.contains("0.06.0"),
            "the answer was quoted back: {said}"
        );
    }

    /// An answer of the wrong shape is silence. A startup notice is the last place an error from a
    /// third party's registry should surface.
    #[test]
    fn an_answer_that_is_not_the_shape_expected_is_no_version() {
        for bytes in [
            &b"not json at all"[..],
            &b"{}"[..],
            &br#"{"version":null}"#[..],
            &br#"{"version":"latest"}"#[..],
            &br#"{"tag_name":"v0.7.2"}"#[..],
        ] {
            assert_eq!(version_in(Install::Npm, bytes), None);
        }
    }

    #[test]
    fn a_stored_answer_is_read_back_as_it_was_written() {
        let answer = Answer {
            at: 1_700_000_000,
            latest: version(0, 6, 0),
        };
        let line = encode_answer(Install::Npm, answer);
        assert_eq!(parse_answer(&line, Install::Npm), Some(answer));
    }

    /// One machine can have had both installations. An answer from the release listing says
    /// nothing about what npm has published, so it is not read as though it did.
    #[test]
    fn an_answer_from_the_other_registry_is_not_read() {
        let stored = encode_answer(
            Install::Script,
            Answer {
                at: 1_700_000_000,
                latest: version(9, 9, 9),
            },
        );
        assert_eq!(parse_answer(&stored, Install::Npm), None);
    }

    #[test]
    fn a_file_that_is_not_the_shape_written_here_is_no_answer() {
        for contents in ["", "nonsense", "1700000000\tnpm", "later\tnpm\t0.6.0"] {
            assert_eq!(parse_answer(contents, Install::Npm), None, "{contents}");
        }
    }

    /// Asking on every launch would be a request to somebody else's registry per session, for a
    /// number that changes on the order of days.
    #[test]
    fn a_fresh_answer_is_not_asked_for_again() {
        assert!(!worth_asking(
            true,
            1_000_000,
            Some(1_000_000 - GOOD_FOR + 1)
        ));
        assert!(worth_asking(true, 1_000_000, Some(1_000_000 - GOOD_FOR)));
        assert!(worth_asking(true, 1_000_000, None));
    }

    /// A clock that moved, or a stamp somebody else wrote, must not leave this silent until the
    /// date catches up with the file.
    #[test]
    fn an_answer_stamped_in_the_future_is_asked_again() {
        assert!(worth_asking(true, 1_000_000, Some(2_000_000)));
    }

    /// A session that keeps nothing has nowhere to put an answer, so asking for one would be a
    /// request made for something that is thrown away on arrival.
    #[test]
    fn a_session_that_records_nothing_asks_nothing() {
        assert!(!worth_asking(false, 1_000_000, None));
        assert!(!worth_asking(false, 1_000_000, Some(1)));
    }

    /// The launcher is the one thing that knows it is the npm package, since the binary it starts
    /// is an ordinary file wherever npm happened to unpack it.
    #[test]
    fn the_launcher_saying_npm_is_what_makes_it_an_npm_install() {
        assert_eq!(
            method(Some("npm"), None, None),
            Some(Install::Npm),
            "the launcher was not believed"
        );
        assert_eq!(
            method(Some("something else"), None, None),
            None,
            "a word nothing sets was taken for an installation"
        );
    }

    #[test]
    fn the_binary_the_script_recorded_is_a_script_install() {
        assert_eq!(
            method(
                None,
                Some(Path::new("/usr/local/bin/bravebot")),
                Some(Path::new("/usr/local/bin/bravebot"))
            ),
            Some(Install::Script)
        );
    }

    /// A checkout built from source on a machine that also has a script install is a build nothing
    /// here can update, and a command that replaced the other copy would be worse than silence.
    #[test]
    fn a_binary_other_than_the_recorded_one_is_not_a_script_install() {
        assert_eq!(
            method(
                None,
                Some(Path::new("/home/someone/bravebot/target/debug/bravebot")),
                Some(Path::new("/usr/local/bin/bravebot"))
            ),
            None
        );
    }

    /// A build from source, which is neither installation, is told nothing at all.
    #[test]
    fn an_installation_nothing_recorded_is_left_alone() {
        assert_eq!(method(None, Some(Path::new("/tmp/bravebot")), None), None);
    }

    /// The version compiled in has to be readable as three numbers, or nothing is ever compared
    /// against it and the notice silently never appears.
    #[test]
    fn the_version_this_was_built_as_is_three_numbers() {
        assert!(running_version().is_some(), "{}", env!("CARGO_PKG_VERSION"));
    }
}
