//! Where the AWS credentials that sign a request come from.
//!
//! The `aws` CLI is asked, rather than the credential files being read here. `~/.aws/config` is an
//! INI file describing SSO sessions, assumed roles, credential processes and chains between them,
//! and the tool that already resolves all of that correctly is installed on any machine configured
//! to reach Bedrock. Reimplementing it would mean an INI parser, the OIDC device-authorization flow,
//! and a cache format that is AWS's to change.
//!
//! # The first-use flow
//!
//! An SSO session expires, and the first request after that fails. The remedy is a browser: `aws sso
//! login` opens one, the person approves the session, and it closes. So a failure to resolve
//! credentials is not final here. The login is run once, and the export retried once, which turns
//! an expired session into a pause rather than an error.
//!
//! # What is trusted, and what is not
//!
//! The subprocess is fixed argv: the program is `aws`, the subcommands are constants in this file,
//! and the only value that varies is the profile name, which comes from the user's own settings and
//! arrives as a separate argument rather than inside a string a shell would split. No model output
//! and no workspace content reaches any of it, and there is no shell.
//!
//! What comes back is a credential, and it is treated as one: [`Secret`] keeps it out of a `Debug`
//! render, and nothing logs it. It is not workspace content, so it carries no label; it never enters
//! a turn, and the only thing it is ever used for is computing a signature.

use bravebot_config::Secret;
use std::io::BufRead;
use std::process::{Command, Stdio};
use std::sync::mpsc;

/// The program asked for credentials.
///
/// A bare name, resolved through `PATH` like any other command a person runs. An absolute path would
/// be wrong on the several platforms that install it somewhere different.
const AWS: &str = "aws";

/// Credentials for signing, as the CLI reported them.
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: Secret,
    /// Present for temporary credentials, which is what an SSO or assumed-role session gives.
    pub session_token: Option<Secret>,
    /// Seconds since the epoch at which these stop working, where the CLI said.
    ///
    /// Absent for a long-lived access key, which does not expire, and absent when the field is
    /// there but unreadable: both mean "nothing here says when to ask again".
    pub expires_at: Option<u64>,
}

/// Redacting rather than derived: two of these three fields are live credentials, and printing a
/// request while working out why a signature was rejected is the obvious first move.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key_id", &"<redacted>")
            .field("secret_access_key", &self.secret_access_key)
            .field("session_token", &self.session_token)
            .finish()
    }
}

#[derive(Debug)]
pub enum CredentialError {
    /// The `aws` CLI is not installed, or not on `PATH`.
    ///
    /// Separate from a failed run because the remedy is different and the message has to say so:
    /// no amount of signing in fixes a missing program.
    NotInstalled,
    /// The CLI ran and refused, after a login attempt if one was possible.
    Refused { detail: String },
    /// The CLI answered with something that was not a set of credentials.
    Undecodable { detail: String },
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => f.write_str(
                "the AWS CLI is not installed, and Bedrock credentials come from it. Install it, or \
                 unset BRAVEBOT_USE_BEDROCK to use the Brave backend",
            ),
            Self::Refused { detail } => write!(
                f,
                "AWS credentials could not be resolved: {detail}. Run `aws sso login` and try again"
            ),
            Self::Undecodable { detail } => write!(f, "unexpected credentials from the AWS CLI: {detail}"),
        }
    }
}

impl std::error::Error for CredentialError {}

/// Resolve credentials for a profile, signing in first if the session has expired.
///
/// The fallback is a last resort rather than the way a person is meant to sign in: what the sign-in
/// prints goes nowhere, because by the time this runs the caller is usually a worker thread with
/// nothing to draw on. Callers that can ask beforehand call [`sign_in_if_needed`], which reports
/// those lines to somebody, and reach here with nothing left to fix.
pub fn resolve(profile: Option<&str>) -> Result<Credentials, CredentialError> {
    match export(profile) {
        Ok(credentials) => Ok(credentials),
        // The common cause of a failed export is a session that has expired, and the remedy is a
        // browser. Attempted once: a second login would open a second window for whatever the first
        // one failed to fix.
        Err(CredentialError::Refused { detail }) => {
            login(profile, |_| {}).map_err(|_| CredentialError::Refused { detail })?;
            export(profile)
        }
        Err(other) => Err(other),
    }
}

/// Whether a session is already good, without touching a browser or a terminal.
///
/// For a caller deciding whether it is about to need one. Any failure reads as "a sign-in would
/// help", because the alternative is inspecting the CLI's wording to tell an expired session from a
/// misconfigured profile, and being wrong that way costs a sign-in that explains itself while being
/// wrong the other way leaves somebody staring at a stalled turn.
///
/// Answered from the last successful export while that credential is still good for long enough to
/// sign what follows, because the CLI takes most of a second to run and a caller asks this before
/// every turn: paid on each one it is a visible pause between pressing Enter and seeing the line
/// land. Only a good answer is kept, so a session needing a sign-in is never told it has one.
pub fn is_signed_in(profile: Option<&str>) -> bool {
    if known_good().holds(profile, now()) {
        return true;
    }
    match export(profile) {
        Ok(credentials) => {
            known_good().keep(profile, credentials.expires_at);
            true
        }
        Err(_) => false,
    }
}

/// The sessions an export has already shown to be good, and until when.
///
/// Per profile, because a session is signed in to one and not another, and a single slot would have
/// one profile's answer stand for the next one asked about.
#[derive(Default)]
struct KnownGood(std::sync::Mutex<std::collections::HashMap<Option<String>, u64>>);

impl KnownGood {
    /// Whether this profile was shown to be good and has not run out.
    ///
    /// A poisoned lock reads as "nothing is known", which costs a CLI run rather than trusting a
    /// map another thread was part way through.
    fn holds(&self, profile: Option<&str>, now: u64) -> bool {
        let Ok(known) = self.0.lock() else {
            return false;
        };
        known
            .get(&profile.map(str::to_string))
            .is_some_and(|&until| now < until)
    }

    /// Keep a good export, so the next caller need not run the CLI to hear the same thing.
    ///
    /// An export that did not say when it expires is not kept at all, and is re-asked every time.
    /// That is the long-lived-key case, where the CLI answers from a file rather than a network, so
    /// it is the cheap one, and a lifetime invented for it would be a guess with nothing behind it.
    fn keep(&self, profile: Option<&str>, expires_at: Option<u64>) {
        let Some(expires_at) = expires_at else { return };
        if let Ok(mut known) = self.0.lock() {
            // Held back from the stated expiry, because this decides whether to sign in before a
            // turn that then has to run: told yes in the credential's last second, the request that
            // follows is signed with one that has expired by the time it arrives.
            known.insert(
                profile.map(str::to_string),
                expires_at.saturating_sub(MARGIN),
            );
        }
    }
}

/// How long before a stated expiry a credential stops counting as usable.
///
/// Long enough for the turn that follows the check to be signed and sent.
const MARGIN: u64 = 300;

fn known_good() -> &'static KnownGood {
    static KNOWN: std::sync::OnceLock<KnownGood> = std::sync::OnceLock::new();
    KNOWN.get_or_init(KnownGood::default)
}

/// Seconds since the Unix epoch.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// Sign in for a profile, if that is what is missing, reporting what the sign-in says.
///
/// Separated from [`resolve`] so an interface can do the interactive part at a moment of its own
/// choosing, and put the URL and the code where a person is already looking. `say` is called per
/// line as the CLI writes it, while the command is still waiting: that is the whole of the flow, so
/// a caller that discards them leaves somebody with a browser open and no code to type into it.
///
/// Quiet when the session is already good: `say` is never called and no browser opens, so this is
/// safe to call before every turn.
pub fn sign_in_if_needed(
    profile: Option<&str>,
    say: impl FnMut(String),
) -> Result<(), CredentialError> {
    if is_signed_in(profile) {
        return Ok(());
    }
    login(profile, say)
}

/// Ask the CLI for credentials, without trying to fix anything.
fn export(profile: Option<&str>) -> Result<Credentials, CredentialError> {
    // `--format process` is the documented, stable shape for exactly this: a program asking another
    // program for credentials. The alternative, `--format env`, returns shell assignments that would
    // have to be parsed as such.
    let mut command = Command::new(AWS);
    command.args(["configure", "export-credentials", "--format", "process"]);
    if let Some(profile) = profile {
        command.args(["--profile", profile]);
    }

    let output = command.output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => CredentialError::NotInstalled,
        _ => CredentialError::Refused {
            detail: e.to_string(),
        },
    })?;

    if !output.status.success() {
        return Err(CredentialError::Refused {
            detail: first_line(&output.stderr),
        });
    }

    decode(&output.stdout)
}

/// Open a browser and wait for the person to approve the session, reporting what the CLI says.
///
/// `say` is called once per line as it arrives, because those lines are the sign-in: a URL and a
/// confirmation code, printed while the command waits for them to be used. Output is piped rather
/// than inherited so a caller drawing its own display can put them where a person is already
/// looking, instead of underneath it. The CLI flushes them immediately when piped, which is what
/// makes that safe: buffered until exit, a code would arrive after it had stopped being useful.
///
/// The browser is still opened. `--no-browser` would suppress the one step that needs no typing at
/// all, and the URL is printed either way for a machine that cannot open one.
///
/// Nothing here is labelled. The lines are a program's own prompt to the person at the keyboard, not
/// workspace content and not model output, and they reach a screen rather than a planner.
fn login(profile: Option<&str>, mut say: impl FnMut(String)) -> Result<(), CredentialError> {
    let mut command = Command::new(AWS);
    command.args(["sso", "login"]);
    if let Some(profile) = profile {
        command.args(["--profile", profile]);
    }

    let mut child = command
        // Both streams, because which one carries the code is the CLI's business and a person who
        // cannot see it is stuck either way.
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Nothing is typed at this: the code goes into a browser, and the terminal belongs to
        // whoever called. A program left able to read stdin would fight for the keys.
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => CredentialError::NotInstalled,
            _ => CredentialError::Refused {
                detail: e.to_string(),
            },
        })?;

    // Drained on a thread apiece and merged over one channel, so neither stream blocks against a
    // pipe nobody is reading and the lines arrive in the order they were written.
    let (lines, arriving) = mpsc::channel::<String>();
    let readers: Vec<_> = [
        child.stdout.take().map(Reading::Out),
        child.stderr.take().map(Reading::Err),
    ]
    .into_iter()
    .flatten()
    .map(|stream| {
        let lines = lines.clone();
        std::thread::spawn(move || stream.read_lines_into(&lines))
    })
    .collect();
    drop(lines);

    // Every line is passed on as it arrives. The loop ends when both readers have finished, which
    // is when the command has closed its streams.
    for line in arriving {
        say(line);
    }
    for reader in readers {
        let _ = reader.join();
    }

    let status = child.wait().map_err(|e| CredentialError::Refused {
        detail: e.to_string(),
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(CredentialError::Refused {
            detail: "the sign-in did not complete".to_string(),
        })
    }
}

/// One of the child's two streams, so both can be read by the same code.
enum Reading {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

impl Reading {
    /// Send every line this stream produces, dropping a trailing newline.
    ///
    /// Lines rather than chunks, because a caller is putting them into a transcript one entry at a
    /// time and a chunk boundary is not a place a person would break a sentence.
    fn read_lines_into(self, lines: &mpsc::Sender<String>) {
        let reader: Box<dyn std::io::Read> = match self {
            Self::Out(stream) => Box::new(stream),
            Self::Err(stream) => Box::new(stream),
        };
        for line in std::io::BufReader::new(reader).lines() {
            // A stream that stopped mid-line is a command that has finished or died, and either way
            // there is nothing further to report from it.
            let Ok(line) = line else { return };
            if lines.send(line).is_err() {
                return;
            }
        }
    }
}

/// Read the credential JSON the CLI's process format emits.
pub fn decode(bytes: &[u8]) -> Result<Credentials, CredentialError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| CredentialError::Undecodable {
            detail: e.to_string(),
        })?;

    let field = |name: &str| -> Option<String> {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .filter(|found| !found.is_empty())
    };

    let (Some(access_key_id), Some(secret_access_key)) =
        (field("AccessKeyId"), field("SecretAccessKey"))
    else {
        // Deliberately does not quote the body: on the success path it holds a live secret, and an
        // error message is the most likely thing to be pasted somewhere public.
        return Err(CredentialError::Undecodable {
            detail: "no access key in the response".to_string(),
        });
    };

    Ok(Credentials {
        access_key_id,
        secret_access_key: Secret::new(secret_access_key),
        session_token: field("SessionToken").map(Secret::new),
        expires_at: field("Expiration").as_deref().and_then(expiry_seconds),
    })
}

/// The epoch seconds in an `Expiration`, or `None` if it is not the shape this reads.
///
/// The CLI emits RFC 3339 in UTC, and only the fixed-width prefix through the seconds is read:
/// `YYYY-MM-DDTHH:MM:SS`. Anything after it is a zone suffix or a fraction, and neither changes
/// which second this is to a useful precision. Parsed by hand for the reason the signing timestamp
/// is formatted by hand, and an unparsable value is not an error anywhere: it means a caller
/// re-asks the CLI rather than trusting a date it could not read.
fn expiry_seconds(text: &str) -> Option<u64> {
    let digits = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();

    // Rejected rather than skipped over: a value whose separators are somewhere else is not the
    // format this claims to read, and guessing at it would produce a confidently wrong date.
    if [4, 7]
        .iter()
        .any(|&at| text.as_bytes().get(at) != Some(&b'-'))
        || text.as_bytes().get(10) != Some(&b'T')
        || [13, 16]
            .iter()
            .any(|&at| text.as_bytes().get(at) != Some(&b':'))
    {
        return None;
    }

    let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let days = days_from_civil(year, month, day);
    u64::try_from(days * 86_400 + hour * 3600 + minute * 60 + second).ok()
}

/// Days since 1970-01-01 for a civil date.
///
/// Howard Hinnant's `days_from_civil`, the inverse of the conversion the signing crate uses to
/// format a timestamp: the epoch shifts to 0000-03-01 so leap days fall at the end of the year and
/// the arithmetic needs no table.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The first line of a program's stderr, bounded.
///
/// One line because the CLI's failures start with the useful sentence and continue with a stack of
/// context, and bounded because this ends up in a message on someone's screen.
fn first_line(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("the AWS CLI gave no reason");
    line.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the CLI's `--format process` actually emits, which is what this has to read.
    #[test]
    fn credentials_are_read_from_the_process_format() {
        let credentials = decode(
            br#"{"Version":1,"AccessKeyId":"AKIA","SecretAccessKey":"secret","SessionToken":"token"}"#,
        )
        .expect("decoded");
        assert_eq!(credentials.access_key_id, "AKIA");
        assert_eq!(credentials.secret_access_key.expose(), "secret");
        assert_eq!(
            credentials.session_token.as_ref().map(Secret::expose),
            Some("token")
        );
    }

    /// Long-lived keys have no session token, and that is not a failure: it is what a plain access
    /// key in a credentials file looks like.
    #[test]
    fn long_lived_credentials_have_no_session_token() {
        let credentials =
            decode(br#"{"AccessKeyId":"AKIA","SecretAccessKey":"secret"}"#).expect("decoded");
        assert!(credentials.session_token.is_none());
    }

    /// The expiry is what lets a caller answer "is this still good" without running the CLI again,
    /// so it has to survive the decode rather than being dropped with the rest of the envelope.
    #[test]
    fn the_expiry_the_cli_reports_is_read_from_the_process_format() {
        let credentials = decode(
            br#"{"AccessKeyId":"AKIA","SecretAccessKey":"secret","Expiration":"2026-09-05T03:04:02+00:00"}"#,
        )
        .expect("decoded");
        assert_eq!(credentials.expires_at, Some(1_788_577_442));
    }

    /// A credential with no stated expiry is the long-lived case, and saying so is not the same as
    /// saying it expired at the epoch: a caller reads `None` as "nothing here says when to re-ask".
    #[test]
    fn credentials_with_no_stated_expiry_report_none() {
        let credentials =
            decode(br#"{"AccessKeyId":"AKIA","SecretAccessKey":"secret"}"#).expect("decoded");
        assert!(credentials.expires_at.is_none());
    }

    /// Checked against the dates the arithmetic gets wrong: the epoch itself, a leap day in a century
    /// year that is one, a leap day in an ordinary leap year, a year end, and the day after February
    /// in a century year that is not a leap year. A conversion error here hands back a credential
    /// after it stopped working, or re-runs the CLI every turn, and neither surfaces near this code.
    #[test]
    fn an_expiry_is_converted_to_the_instant_it_names() {
        for (text, seconds) in [
            ("1970-01-01T00:00:00Z", 0),
            ("2000-02-29T12:00:00Z", 951_825_600),
            ("2024-02-29T00:00:00Z", 1_709_164_800),
            ("2026-09-05T03:04:02+00:00", 1_788_577_442),
            ("2026-12-31T23:59:59Z", 1_798_761_599),
            ("2100-03-01T23:59:59Z", 4_107_628_799),
        ] {
            assert_eq!(expiry_seconds(text), Some(seconds), "{text}");
        }
    }

    /// The point of keeping the answer: a caller asking before every turn pays for the CLI once,
    /// rather than waiting most of a second between pressing Enter and seeing the line land.
    #[test]
    fn a_session_already_shown_to_be_good_is_not_asked_about_again() {
        let known = KnownGood::default();
        known.keep(Some("work"), Some(10_000));
        assert!(known.holds(Some("work"), 5_000));
    }

    /// The whole risk of keeping it. A session that has run out must send the next caller back to the
    /// CLI, or a turn is signed with a credential that expired and fails at the far end.
    #[test]
    fn a_session_that_has_run_out_is_asked_about_again() {
        let known = KnownGood::default();
        known.keep(Some("work"), Some(10_000));
        assert!(!known.holds(Some("work"), 10_000));
        assert!(!known.holds(Some("work"), 20_000));
    }

    /// Credentials handed over in their last moment are signed with after this returns, so the answer
    /// has to stop being yes before the expiry rather than at it.
    #[test]
    fn a_session_about_to_run_out_is_treated_as_already_gone() {
        let known = KnownGood::default();
        known.keep(Some("work"), Some(10_000));
        assert!(!known.holds(Some("work"), 10_000 - MARGIN));
        assert!(known.holds(Some("work"), 10_000 - MARGIN - 1));
    }

    /// One profile's session says nothing about another's. Answered from a single slot, signing in to
    /// one account would report every other one as good and skip the sign-in they need.
    #[test]
    fn one_profile_being_good_says_nothing_about_another() {
        let known = KnownGood::default();
        known.keep(Some("work"), Some(10_000));
        assert!(!known.holds(Some("personal"), 5_000));
        assert!(!known.holds(None, 5_000));
    }

    /// A default profile is a profile, and gets its own answer rather than sharing a named one's.
    #[test]
    fn the_default_profile_is_remembered_like_any_other() {
        let known = KnownGood::default();
        known.keep(None, Some(10_000));
        assert!(known.holds(None, 5_000));
        assert!(!known.holds(Some("work"), 5_000));
    }

    /// Nothing is kept for an export that did not say when it expires, so those callers re-ask. A
    /// stand-in lifetime here would be a guess reported as the credential's own word.
    #[test]
    fn a_session_with_no_stated_expiry_is_not_kept() {
        let known = KnownGood::default();
        known.keep(Some("work"), None);
        assert!(!known.holds(Some("work"), 0));
    }

    /// An unreadable expiry must not become a confident date. Read as one, a garbled value either
    /// pins a session as good long after it expired or forces the CLI on every turn.
    #[test]
    fn an_expiry_that_is_not_the_expected_shape_is_not_guessed_at() {
        for text in [
            "",
            "not a date",
            "2026-09-05",
            "2026/09/05T03:04:02Z",
            "2026-09-05 03:04:02Z",
            "2026-13-05T03:04:02Z",
            "2026-09-32T03:04:02Z",
            "2026-09-05T24:04:02Z",
            "2026-09-05T03:60:02Z",
            "xxxx-09-05T03:04:02Z",
        ] {
            assert_eq!(expiry_seconds(text), None, "{text} was read as a date");
        }
    }

    /// Signing with half a credential produces a signature that is rejected far from the cause, so
    /// an incomplete answer has to fail here instead.
    #[test]
    fn an_incomplete_answer_is_refused() {
        for body in [
            &br#"{}"#[..],
            &br#"{"AccessKeyId":"AKIA"}"#[..],
            &br#"{"SecretAccessKey":"secret"}"#[..],
            &br#"{"AccessKeyId":"","SecretAccessKey":"secret"}"#[..],
            &br#"{"AccessKeyId":"AKIA","SecretAccessKey":""}"#[..],
        ] {
            assert!(
                matches!(decode(body), Err(CredentialError::Undecodable { .. })),
                "{} was read as credentials",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn output_that_is_not_json_is_refused() {
        assert!(matches!(
            decode(b"could not connect to the endpoint"),
            Err(CredentialError::Undecodable { .. })
        ));
    }

    /// The decode error is the one most likely to be pasted into an issue, and on the success path
    /// the same bytes hold a live secret. It must not quote what it was given.
    #[test]
    fn a_decode_failure_does_not_quote_the_credentials_it_was_given() {
        let body = br#"{"AccessKeyId":"AKIA","SecretAccessKey":"a-live-secret","Oops":}"#;
        let message = match decode(body) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("should not have decoded"),
        };
        assert!(!message.contains("a-live-secret"), "leaked: {message}");
    }

    /// Printing a resolved credential is the reflex when a signature is rejected, and these are
    /// live keys.
    #[test]
    fn debugging_resolved_credentials_does_not_leak_them() {
        let credentials = decode(
            br#"{"AccessKeyId":"AKIAREAL","SecretAccessKey":"a-live-secret","SessionToken":"a-live-token"}"#,
        )
        .expect("decoded");
        let shown = format!("{credentials:?}");
        assert!(!shown.contains("a-live-secret"), "leaked: {shown}");
        assert!(!shown.contains("a-live-token"), "leaked: {shown}");
        assert!(!shown.contains("AKIAREAL"), "leaked: {shown}");
    }

    /// A missing CLI cannot be fixed by signing in, so it says something different. Reporting it as
    /// an expired session would send someone to a browser that cannot help.
    #[test]
    fn a_missing_cli_is_reported_as_a_missing_cli() {
        let message = CredentialError::NotInstalled.to_string();
        assert!(message.contains("not installed"), "{message}");
        assert!(!message.contains("sso login"), "{message}");
    }

    /// The remedy for an expired session is a browser, and the message is where someone finds that
    /// out.
    #[test]
    fn a_refusal_names_the_command_that_fixes_it() {
        let message = CredentialError::Refused {
            detail: "the SSO session has expired".to_string(),
        }
        .to_string();
        assert!(message.contains("aws sso login"), "{message}");
    }

    /// Failures arrive with the useful sentence first and a stack of context after it, and this
    /// ends up on someone's screen.
    #[test]
    fn only_the_first_line_of_a_failure_is_reported() {
        let reported = first_line(b"the session has expired\nplus a long trace\nand more");
        assert_eq!(reported, "the session has expired");
    }

    #[test]
    fn a_silent_failure_still_says_something() {
        assert!(!first_line(b"").is_empty());
        assert!(!first_line(b"   \n  \n").is_empty());
    }

    /// A single line of runaway output must not become the whole message.
    #[test]
    fn a_very_long_failure_is_bounded() {
        assert!(first_line("x".repeat(10_000).as_bytes()).chars().count() <= 200);
    }
}
