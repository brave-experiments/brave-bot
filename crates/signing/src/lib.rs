//! Request signing for the backends this agent talks to.
//!
//! Two schemes, one per backend: Brave's services below, and AWS Signature Version 4 in
//! [`sigv4`] for Bedrock. They share this crate because they share their whole toolkit,
//! HMAC-SHA256 over a canonical string, and because signing is all either of them does:
//! neither carries workspace content or model output.
//!
//! The services key is an HMAC signing key, **not** a bearer token: it is never
//! transmitted. Each request carries a `Digest` header over the body and an
//! `Authorization` header holding an HMAC-SHA256 signature.
//!
//! ```text
//! Digest:        SHA-256=<base64(sha256(body))>
//! Authorization: Signature keyId="<id>",algorithm="hs2019",headers="digest",
//!                signature="<base64(hmac_sha256(key, signing_string))>"
//! ```
//!
//! The signing string is exactly one line, `digest: SHA-256=...`, because only the
//! digest header is signed. The server requires `headers` to be exactly `"digest"`
//! and rejects anything else, so signing additional headers would fail rather than
//! add protection.
//!
//! Signing the digest is what binds the signature to the body: altering the body
//! changes the digest, which changes the signing string, which invalidates the
//! signature.

pub mod sigv4;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::{Hmac, Mac};
use sha2::{Digest as _, Sha256};

/// The only algorithm the server accepts.
pub const ALGORITHM: &str = "hs2019";

/// The only signed-header set the server accepts.
pub const SIGNED_HEADERS: &str = "digest";

/// Value for the `Digest` header: `SHA-256=<base64>`.
pub fn digest_header(body: &[u8]) -> String {
    format!("SHA-256={}", BASE64.encode(Sha256::digest(body)))
}

/// The string that gets signed, derived from the digest header value.
fn signing_string(digest_header_value: &str) -> String {
    format!("digest: {digest_header_value}")
}

/// Value for the `Authorization` header.
///
/// `key_id` identifies which derived key the server should verify against; the server
/// derives its copy from a master seed plus this id, so `signing_key` and `key_id`
/// must be a matched pair.
pub fn authorization_header(signing_key: &str, key_id: &str, digest_header_value: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(signing_key.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(signing_string(digest_header_value).as_bytes());
    let signature = BASE64.encode(mac.finalize().into_bytes());

    format!(
        "Signature keyId=\"{key_id}\",algorithm=\"{ALGORITHM}\",headers=\"{SIGNED_HEADERS}\",signature=\"{signature}\""
    )
}

/// Both headers a signed request needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedHeaders {
    pub digest: String,
    pub authorization: String,
}

/// Sign a request body.
pub fn sign(signing_key: &str, key_id: &str, body: &[u8]) -> SignedHeaders {
    let digest = digest_header(body);
    let authorization = authorization_header(signing_key, key_id, &digest);
    SignedHeaders {
        digest,
        authorization,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test vector from the IETF HTTP Digest Headers draft, which the server's own
    /// test suite also uses. Verifying against it means our digest matches the
    /// server's byte for byte.
    #[test]
    fn digest_matches_the_ietf_test_vector() {
        let body = br#"{"hello": "world"}"#;
        assert_eq!(
            digest_header(body),
            "SHA-256=X48E9qOokqqrvdts8nOJRJN3OWDUoyWxBf7kbu9DBPE="
        );
    }

    #[test]
    fn digest_of_an_empty_body_is_the_sha256_of_nothing() {
        assert_eq!(
            digest_header(b""),
            "SHA-256=47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
    }

    #[test]
    fn the_signing_string_is_only_the_digest_line() {
        assert_eq!(signing_string("SHA-256=abc"), "digest: SHA-256=abc");
    }

    #[test]
    fn authorization_header_has_the_expected_shape() {
        let header = authorization_header("key", "my-id", "SHA-256=abc");
        assert!(header.starts_with("Signature keyId=\"my-id\","));
        assert!(header.contains("algorithm=\"hs2019\""));
        assert!(header.contains("headers=\"digest\""));
        assert!(header.contains("signature=\""));
        assert!(header.ends_with('"'));
    }

    /// The server rejects any signed-header set other than "digest", so this must not
    /// drift.
    #[test]
    fn only_the_digest_header_is_signed() {
        assert_eq!(SIGNED_HEADERS, "digest");
        let header = authorization_header("key", "id", "SHA-256=abc");
        assert!(!header.contains("(request-target)"));
    }

    /// Independently computed with Python:
    ///   hmac.new(b"test-key", b"digest: SHA-256=abc", sha256)
    /// so this pins our output against a second implementation rather than itself.
    #[test]
    fn signature_matches_an_independent_hmac() {
        let header = authorization_header("test-key", "id", "SHA-256=abc");
        assert!(
            header.contains("signature=\"IKKflRlwqN1asb9TGW7V2d39RR6m+xzVgPEZRxMh1cE=\""),
            "unexpected signature in {header}"
        );
    }

    /// Changing the body changes the signature: this is what stops a proxy from
    /// altering a request in flight.
    #[test]
    fn a_different_body_produces_a_different_signature() {
        let a = sign("key", "id", b"body one");
        let b = sign("key", "id", b"body two");
        assert_ne!(a.digest, b.digest);
        assert_ne!(a.authorization, b.authorization);
    }

    #[test]
    fn a_different_key_produces_a_different_signature() {
        let a = sign("key-one", "id", b"body");
        let b = sign("key-two", "id", b"body");
        assert_eq!(a.digest, b.digest, "digest does not depend on the key");
        assert_ne!(a.authorization, b.authorization);
    }

    #[test]
    fn signing_is_deterministic() {
        assert_eq!(sign("key", "id", b"body"), sign("key", "id", b"body"));
    }

    /// The key must never appear in the headers: it signs, it is not sent.
    #[test]
    fn the_signing_key_is_never_transmitted() {
        let key = "super-secret-signing-key";
        let headers = sign(key, "id", b"body");
        assert!(!headers.authorization.contains(key));
        assert!(!headers.digest.contains(key));
    }
}
