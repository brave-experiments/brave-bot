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
use std::process::Command;

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
                 unset CLAUDE_CODE_USE_BEDROCK to use the Brave backend",
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
/// `announce` is called before a browser is opened, because a window appearing unprompted with
/// nothing said about it is indistinguishable from something having gone wrong. It is only called
/// when a login is actually about to happen.
pub fn resolve(
    profile: Option<&str>,
    announce: impl FnOnce(),
) -> Result<Credentials, CredentialError> {
    match export(profile) {
        Ok(credentials) => Ok(credentials),
        // The common cause of a failed export is a session that has expired, and the remedy is a
        // browser. Attempted once: a second login would open a second window for whatever the first
        // one failed to fix.
        Err(CredentialError::Refused { detail }) => {
            announce();
            login(profile).map_err(|_| CredentialError::Refused { detail })?;
            export(profile)
        }
        Err(other) => Err(other),
    }
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

/// Open a browser and wait for the person to approve the session.
///
/// Inherits the terminal deliberately: the CLI prints the URL and a confirmation code, and on a
/// machine with no browser to open that output is the only way through.
fn login(profile: Option<&str>) -> Result<(), CredentialError> {
    let mut command = Command::new(AWS);
    command.args(["sso", "login"]);
    if let Some(profile) = profile {
        command.args(["--profile", profile]);
    }

    let status = command.status().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => CredentialError::NotInstalled,
        _ => CredentialError::Refused {
            detail: e.to_string(),
        },
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(CredentialError::Refused {
            detail: "the sign-in did not complete".to_string(),
        })
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
    })
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
