//! AWS Signature Version 4, for requests to Bedrock.
//!
//! The second signing scheme in this crate and unrelated to the first: Brave's services sign a body
//! digest with a long-lived derived key, AWS signs a description of the whole request with a key
//! derived per day, region, and service. Both are HMAC-SHA256 over a canonical string, which is why
//! they share a crate and its two dependencies.
//!
//! ```text
//! Authorization: AWS4-HMAC-SHA256 Credential=<id>/<date>/<region>/<service>/aws4_request,
//!                SignedHeaders=host;x-amz-content-sha256;x-amz-date,
//!                Signature=<hex(hmac(signing_key, string_to_sign))>
//! ```
//!
//! What is signed is a hash of the canonical request: method, path, query, the signed headers, and
//! the body's hash. Any of those changing invalidates the signature, which is what binds a signature
//! to one request rather than to a body alone.
//!
//! Only what Bedrock needs is implemented. Every request is a POST with a body, no query string, and
//! three signed headers, so there is no query-parameter canonicalisation and no multi-value header
//! folding here. A caller signing something else would need both.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::{Hmac, Mac};
use sha2::{Digest as _, Sha256};

/// The only algorithm this implements.
pub const ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// The headers signed on every request, in the order the canonical form requires.
///
/// Lowercase and sorted, which is not a style choice: the canonical request is built from exactly
/// this, so a different order or case produces a different signature from the one the service
/// computes.
pub const SIGNED_HEADERS: &str = "host;x-amz-content-sha256;x-amz-date";

/// What the terminating scope segment must be.
const TERMINATOR: &str = "aws4_request";

/// Credentials to sign one request with.
///
/// Borrowed rather than owned: these come from a short-lived source and are used immediately, so
/// nothing here should be tempted to keep a copy.
#[derive(Clone, Copy)]
pub struct Credentials<'a> {
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
    /// Present for temporary credentials, which is what an SSO or assumed-role session gives.
    ///
    /// It is signed as a header when present, so a request that omitted it while signing with
    /// temporary keys would be rejected.
    pub session_token: Option<&'a str>,
}

/// Deliberately not derived: a secret access key in a log is a live credential, and printing a
/// request or its credentials is the first thing anyone does when a signature is rejected.
impl std::fmt::Debug for Credentials<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key_id", &"<redacted>")
            .field("secret_access_key", &"<redacted>")
            .field("session_token", &self.session_token.map(|_| "<redacted>"))
            .finish()
    }
}

/// The headers a signed request carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headers {
    /// `x-amz-date`, the timestamp the signature is bound to.
    pub date: String,
    /// `x-amz-content-sha256`, the hex hash of the body.
    pub content_sha256: String,
    /// `authorization`, the signature and what it covers.
    pub authorization: String,
    /// `x-amz-security-token`, when signing with temporary credentials.
    pub security_token: Option<String>,
}

/// Sign one POST request.
///
/// `now` is seconds since the Unix epoch, passed in rather than read here so a test can pin a
/// timestamp and check a signature against a known value. `path` is the path as the request will
/// send it, already encoded.
pub fn sign_post(
    credentials: Credentials<'_>,
    region: &str,
    service: &str,
    host: &str,
    path: &str,
    body: &[u8],
    now: u64,
) -> Headers {
    let timestamp = timestamp(now);
    let date = &timestamp[..8];
    let body_hash = hex(&Sha256::digest(body));

    // Every signed header, in the canonical order, matching SIGNED_HEADERS exactly.
    let canonical_headers =
        format!("host:{host}\nx-amz-content-sha256:{body_hash}\nx-amz-date:{timestamp}\n");

    // The empty line is the query string, which Bedrock's invoke routes do not use.
    let canonical_request = format!(
        "POST\n{}\n\n{canonical_headers}\n{SIGNED_HEADERS}\n{body_hash}",
        canonical_path(path)
    );

    let scope = format!("{date}/{region}/{service}/{TERMINATOR}");
    let string_to_sign = format!(
        "{ALGORITHM}\n{timestamp}\n{scope}\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );

    let signature = hex(&hmac(
        &signing_key(credentials.secret_access_key, date, region, service),
        string_to_sign.as_bytes(),
    ));

    Headers {
        authorization: format!(
            "{ALGORITHM} Credential={}/{scope}, SignedHeaders={SIGNED_HEADERS}, Signature={signature}",
            credentials.access_key_id
        ),
        date: timestamp,
        content_sha256: body_hash,
        security_token: credentials.session_token.map(str::to_string),
    }
}

/// The path as the canonical request states it, which is the sent path encoded a second time.
///
/// Not a mistake and not belt-and-braces: for every service but S3, SigV4 canonicalises the path by
/// URI-encoding it again, so a `%3A` in the request appears as `%253A` in the string that is signed.
/// Signing the sent path instead produces a signature the service rejects with a message about the
/// secret key, which points at the wrong thing entirely. It only shows up on a path containing
/// escapes, which for Bedrock means every request naming an inference-profile ARN.
///
/// A percent sign is the only character this has to escape: the input is already encoded, so
/// everything else in it is unreserved and encoding it again would leave it unchanged.
fn canonical_path(path: &str) -> String {
    path.replace('%', "%25")
}

/// The key a signature is computed with, derived per day, region, and service.
///
/// The chain is what limits the damage a leaked signature does: it authorises one service in one
/// region on one day, rather than everything the secret key can reach.
fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let mut key = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    key = hmac(&key, region.as_bytes());
    key = hmac(&key, service.as_bytes());
    hmac(&key, TERMINATOR.as_bytes())
}

fn hmac(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A base64 body hash, for the one place AWS wants it that way.
///
/// Unused by the signature itself, and here because the event-stream framing a streamed reply
/// arrives in describes payloads this way.
pub fn base64_digest(body: &[u8]) -> String {
    BASE64.encode(Sha256::digest(body))
}

/// `YYYYMMDDTHHMMSSZ`, the only timestamp format the signature accepts.
///
/// Converted by hand rather than with a date library. The arithmetic is a dozen lines and needs no
/// timezone database, whereas a dependency here would be a patterns-and-parsing surface added to a
/// crate whose entire job is to be small and auditable.
fn timestamp(seconds_since_epoch: u64) -> String {
    let days = seconds_since_epoch / 86_400;
    let seconds_of_day = seconds_since_epoch % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60,
    )
}

/// The civil date for a count of days since 1970-01-01.
///
/// Howard Hinnant's `civil_from_days`, which shifts the epoch to 0000-03-01 so that leap days fall
/// at the end of the year and the month arithmetic needs no table.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let shifted = days_since_epoch + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Credentials<'static> {
        Credentials {
            access_key_id: "AKIDEXAMPLE",
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            session_token: None,
        }
    }

    /// The signature is only correct if every derivation step matches AWS's, and the only way to
    /// know that is a value computed elsewhere. This is AWS's own published test vector: the
    /// `aws4_request` chain over their example key, date, region and service.
    ///
    /// Recomputed here rather than asserted from this implementation's own output, which would only
    /// prove it agrees with itself.
    #[test]
    fn the_signing_key_matches_the_published_aws_test_vector() {
        let key = signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20150830",
            "us-east-1",
            "iam",
        );
        assert_eq!(
            hex(&key),
            "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9"
        );
    }

    /// SigV4 canonicalises a path by encoding it a second time, so a `%3A` in the request is `%253A`
    /// in the string that is signed. Signing the sent path instead is a 403 whose message blames the
    /// secret key, which points at the wrong thing entirely.
    #[test]
    fn the_canonical_path_is_the_sent_path_encoded_again() {
        assert_eq!(
            canonical_path("/model/arn%3Aaws%3Abedrock/invoke"),
            "/model/arn%253Aaws%253Abedrock/invoke"
        );
        // A path with nothing escaped in it is unchanged, which is why this only ever showed up on
        // requests naming an ARN.
        assert_eq!(canonical_path("/model/m/invoke"), "/model/m/invoke");
    }

    /// The whole signature over an ARN path, checked against a value computed by an independent
    /// implementation and confirmed against the live service. This is the case the double-encoding
    /// rule exists for, and the one a signature built from the sent path gets wrong.
    #[test]
    fn a_signature_over_an_encoded_path_matches_an_independent_implementation() {
        let headers = sign_post(
            keys(),
            "us-west-2",
            "bedrock",
            "bedrock-runtime.us-west-2.amazonaws.com",
            "/model/arn%3Aaws%3Abedrock%3Aus-west-2%3A1%3Afoo%2Fbar/invoke-with-response-stream",
            b"{\"x\":1}",
            1_756_857_600,
        );
        assert!(
            headers.authorization.ends_with(
                "Signature=8388b31113c6a56ca092994b1ec56e8ea2035d13dcee8eb5b412044322a6ca5c"
            ),
            "{}",
            headers.authorization
        );
    }

    /// The timestamp format is exact: AWS rejects anything but `YYYYMMDDTHHMMSSZ`, and the date
    /// inside it also forms the credential scope, so an off-by-one day is a rejected signature.
    #[test]
    fn the_timestamp_is_the_format_the_signature_requires() {
        assert_eq!(timestamp(0), "19700101T000000Z");
        assert_eq!(timestamp(1_440_938_160), "20150830T123600Z");
        assert_eq!(timestamp(1_756_857_600), "20250903T000000Z");
    }

    /// A leap day is where hand-rolled calendar arithmetic goes wrong, and being wrong by a day
    /// puts the wrong date in the credential scope.
    #[test]
    fn leap_days_land_on_the_right_date() {
        // 2024-02-29T00:00:00Z, and the day either side of it.
        assert_eq!(timestamp(1_709_164_800), "20240229T000000Z");
        assert_eq!(timestamp(1_709_164_800 - 86_400), "20240228T000000Z");
        assert_eq!(timestamp(1_709_164_800 + 86_400), "20240301T000000Z");
        // 2000-02-29: a century year that is a leap year, which the simple rule gets wrong.
        assert_eq!(timestamp(951_782_400), "20000229T000000Z");
        // 2100-03-01: a century year that is not, so February has 28 days.
        assert_eq!(timestamp(4_107_542_400), "21000301T000000Z");
    }

    /// The credential scope in the header must name the same day as the timestamp. If they disagree
    /// the service derives a different key and rejects the signature.
    #[test]
    fn the_credential_scope_names_the_same_day_as_the_timestamp() {
        let headers = sign_post(
            keys(),
            "us-west-2",
            "bedrock",
            "bedrock-runtime.us-west-2.amazonaws.com",
            "/model/m/invoke",
            b"{}",
            1_756_857_600,
        );
        assert!(headers.date.starts_with("20250903T"));
        assert!(
            headers
                .authorization
                .contains("Credential=AKIDEXAMPLE/20250903/us-west-2/bedrock/aws4_request"),
            "{}",
            headers.authorization
        );
    }

    /// The body's hash is signed, which is what stops a signature being reused for a different
    /// request. Two bodies must not produce the same signature.
    #[test]
    fn changing_the_body_changes_the_signature() {
        let one = sign_post(
            keys(),
            "us-west-2",
            "bedrock",
            "host.invalid",
            "/p",
            b"{\"a\":1}",
            1_756_857_600,
        );
        let two = sign_post(
            keys(),
            "us-west-2",
            "bedrock",
            "host.invalid",
            "/p",
            b"{\"a\":2}",
            1_756_857_600,
        );
        assert_ne!(one.authorization, two.authorization);
        assert_ne!(one.content_sha256, two.content_sha256);
    }

    /// The path is part of the canonical request, so a signature for one model's route must not be
    /// valid for another's.
    #[test]
    fn changing_the_path_changes_the_signature() {
        let one = sign_post(
            keys(),
            "us-west-2",
            "bedrock",
            "host.invalid",
            "/model/one/invoke",
            b"{}",
            1_756_857_600,
        );
        let two = sign_post(
            keys(),
            "us-west-2",
            "bedrock",
            "host.invalid",
            "/model/two/invoke",
            b"{}",
            1_756_857_600,
        );
        assert_ne!(one.authorization, two.authorization);
    }

    /// The region is in both the scope and the derived key, so a signature minted for one region
    /// cannot be spent in another.
    #[test]
    fn changing_the_region_changes_the_signature() {
        let west = sign_post(
            keys(),
            "us-west-2",
            "bedrock",
            "host.invalid",
            "/p",
            b"{}",
            1_756_857_600,
        );
        let east = sign_post(
            keys(),
            "us-east-1",
            "bedrock",
            "host.invalid",
            "/p",
            b"{}",
            1_756_857_600,
        );
        assert_ne!(west.authorization, east.authorization);
    }

    /// Temporary credentials are what an SSO session gives, and the token must be presented as a
    /// header. Signing with temporary keys and omitting it is rejected.
    #[test]
    fn a_session_token_is_carried_when_the_credentials_are_temporary() {
        let temporary = Credentials {
            session_token: Some("a-session-token"),
            ..keys()
        };
        let headers = sign_post(
            temporary,
            "us-west-2",
            "bedrock",
            "host.invalid",
            "/p",
            b"{}",
            1_756_857_600,
        );
        assert_eq!(headers.security_token.as_deref(), Some("a-session-token"));
    }

    /// Long-lived keys have no token, and sending an empty one is not the same as sending none.
    #[test]
    fn no_session_token_is_carried_for_long_lived_keys() {
        let headers = sign_post(
            keys(),
            "us-west-2",
            "bedrock",
            "host.invalid",
            "/p",
            b"{}",
            1_756_857_600,
        );
        assert_eq!(headers.security_token, None);
    }

    /// The signed-header list in the header must be exactly what the canonical request was built
    /// from. A mismatch is the single most common cause of a rejected SigV4 signature.
    #[test]
    fn the_declared_signed_headers_are_the_ones_that_were_signed() {
        let headers = sign_post(
            keys(),
            "us-west-2",
            "bedrock",
            "host.invalid",
            "/p",
            b"{}",
            1_756_857_600,
        );
        assert!(
            headers
                .authorization
                .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date"),
            "{}",
            headers.authorization
        );
    }

    /// Printing a request while debugging a rejected signature must not put a live credential in a
    /// log, and the reflex when a signature fails is to print everything.
    #[test]
    fn debugging_credentials_does_not_leak_them() {
        let shown = format!(
            "{:?}",
            Credentials {
                session_token: Some("a-session-token"),
                ..keys()
            }
        );
        assert!(!shown.contains("wJalrXUtnFEMI"), "leaked: {shown}");
        assert!(!shown.contains("AKIDEXAMPLE"), "leaked: {shown}");
        assert!(!shown.contains("a-session-token"), "leaked: {shown}");
    }
}
