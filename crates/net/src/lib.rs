//! The single network egress path.
//!
//! Every outbound request in the process goes through [`Egress::fetch`]. The HTTP
//! client is private to this module and no other crate depends on `ureq`, so there is
//! no second path that could skip the policy gate. That is deliberate: in the design
//! this replaces, two of three fetchers bypassed the redirect check because using the
//! hardened helper was optional.
//!
//! Redirects are followed manually and **revalidated on every hop**. Otherwise a
//! permitted host could redirect to a denied one and the gate would only ever have
//! seen the first URL.
//!
//! Timeouts bound each phase of a request separately rather than the call as a whole, so a
//! reply that takes a long time to write is not confused with a connection that has died. See
//! [`Timeouts`].
//!
//! Responses are size-capped and content-type filtered. That is resource hygiene, not
//! content inspection: the bytes are never parsed to decide anything, they are handed
//! back for the caller to label.

use bravebot_core::event::Sink;
use bravebot_core::label::Label;
use bravebot_core::policy::{Denial, Policy};
use bravebot_core::value::Labelled;
use std::fmt;
use std::time::Duration;

/// Response bodies are truncated past this size.
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Redirect hops followed before giving up.
pub const MAX_REDIRECTS: usize = 5;

/// How long each part of a request may take.
///
/// Deliberately not one end-to-end bound. A single global timeout counts the time the model
/// spends writing its reply, so a long answer is indistinguishable from a stalled connection and
/// is cut off for being long. Split up, the two questions can be asked separately: a reply is
/// allowed to take a while, and a connection that has stopped delivering it is not.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// Resolving the host name.
    pub resolve: Duration,
    /// Opening the socket, including the TLS handshake.
    pub connect: Duration,
    /// Sending the request and its body.
    pub send: Duration,
    /// The whole reply, from the request going out to the last byte of the body.
    ///
    /// Generous, because this is where a model thinking and then writing a long answer spends
    /// its time, and none of that is a fault. It is also the only bound on the wait before the
    /// reply starts, since nothing has arrived yet for a gap to be measured between.
    pub reply: Duration,
    /// The longest gap between two pieces of a reply that is still arriving.
    ///
    /// This is the one that catches a dead connection, which is what a closed laptop lid leaves
    /// behind: the socket is gone and nothing on it says so, so a read would otherwise wait for
    /// bytes that are never coming.
    pub idle: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            resolve: Duration::from_secs(15),
            connect: Duration::from_secs(30),
            send: Duration::from_secs(60),
            reply: Duration::from_secs(600),
            idle: Duration::from_secs(120),
        }
    }
}

#[derive(Debug)]
pub enum EgressError {
    /// The policy refused this request.
    Denied(Denial),
    /// A redirect chain exceeded [`MAX_REDIRECTS`].
    TooManyRedirects { url: String },
    /// A redirect response carried no usable target.
    MissingLocation { url: String },
    /// The URL could not be parsed, or was not http(s).
    InvalidUrl { url: String, detail: String },
    /// Transport failure.
    Transport {
        url: String,
        detail: String,
        /// Whether sending the same request again could get past it.
        ///
        /// Decided from what the transport reported, never from anything a server sent: a
        /// timeout, a reset, or a name that did not resolve are facts about the connection.
        transient: bool,
    },
    /// The server returned a non-success status.
    Status { url: String, status: u16 },
}

impl fmt::Display for EgressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied(d) => write!(f, "{d}"),
            Self::TooManyRedirects { url } => {
                write!(f, "too many redirects starting from {url}")
            }
            Self::MissingLocation { url } => {
                write!(f, "{url} returned a redirect with no location")
            }
            Self::InvalidUrl { url, detail } => write!(f, "invalid url {url}: {detail}"),
            Self::Transport { url, detail, .. } => {
                write!(f, "request to {url} failed: {detail}")
            }
            Self::Status { url, status } => write!(f, "{url} returned HTTP {status}"),
        }
    }
}

impl EgressError {
    /// Whether sending the same request again is worth doing.
    ///
    /// A connection that died and an overloaded server are both temporary, and a request that
    /// never arrived has changed nothing, so sending it again is the same request rather than a
    /// second one. Everything else is a decision that will be made the same way twice.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Transport { transient, .. } => *transient,
            Self::Status { status, .. } => RETRYABLE_STATUSES.contains(status),
            Self::Denied(_)
            | Self::TooManyRedirects { .. }
            | Self::MissingLocation { .. }
            | Self::InvalidUrl { .. } => false,
        }
    }
}

/// Statuses a server uses to say "not now" rather than "no".
///
/// 408 and 504 are timeouts, 429 is a rate limit, and 500, 502 and 503 are a server that is
/// unwell rather than a request that is wrong.
const RETRYABLE_STATUSES: [u16; 6] = [408, 429, 500, 502, 503, 504];

impl std::error::Error for EgressError {}

impl From<Denial> for EgressError {
    fn from(value: Denial) -> Self {
        Self::Denied(value)
    }
}

/// A fetched response. The body is labelled, so a caller receives untrusted bytes it
/// cannot inspect without going through the policy.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Labelled<Vec<u8>>,
    /// Whether the body hit [`MAX_RESPONSE_BYTES`].
    pub truncated: bool,
    /// The URL the body actually came from, after any redirects.
    pub final_url: String,
}

/// A response whose body is read in pieces as it arrives.
///
/// Same gates as [`Response`], and the same cap: what changes is when the caller sees the bytes,
/// not whether anything checked them. The label is fixed before the first byte is read, so a
/// stream cannot acquire a better one partway through.
pub struct Streamed<'r> {
    pub status: u16,
    pub content_type: Option<String>,
    pub final_url: String,
    label: Label,
    reader: Box<dyn std::io::Read + 'r>,
    read: usize,
}

impl Streamed<'_> {
    /// The label every piece of this body carries.
    pub fn label(&self) -> Label {
        self.label
    }

    /// Read the next piece, or `None` at the end of the body.
    ///
    /// Each piece comes back labelled, exactly as a whole body would: a caller that wants to look
    /// at one still has to go through the policy. The cap is enforced across the whole stream, so
    /// an endless response is cut off rather than read forever.
    pub fn next_chunk(&mut self) -> Result<Option<Labelled<Vec<u8>>>, EgressError> {
        if self.read >= MAX_RESPONSE_BYTES {
            return Ok(None);
        }

        let mut buffer = vec![0u8; STREAM_CHUNK_BYTES.min(MAX_RESPONSE_BYTES - self.read)];
        match self.reader.read(&mut buffer) {
            Ok(0) => Ok(None),
            Ok(n) => {
                self.read += n;
                buffer.truncate(n);
                Ok(Some(Labelled::new(buffer, self.label)))
            }
            Err(e) => Err(EgressError::Transport {
                url: self.final_url.clone(),
                detail: e.to_string(),
                transient: is_transient_io(&e),
            }),
        }
    }

    /// Whether the cap stopped the read before the body ended.
    pub fn truncated(&self) -> bool {
        self.read >= MAX_RESPONSE_BYTES
    }
}

impl fmt::Debug for Streamed<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // No body: it has not all arrived, and printing what has would expose labelled bytes.
        f.debug_struct("Streamed")
            .field("status", &self.status)
            .field("final_url", &self.final_url)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// How much is read from a streaming body at a time.
///
/// Small enough that a reply appears to arrive as it is written rather than in visible jumps.
const STREAM_CHUNK_BYTES: usize = 1024;

/// A request to send.
#[derive(Debug)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    pub fn post(url: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: Vec::new(),
            body: Some(body),
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// The one way out of the process.
pub struct Egress {
    agent: ureq::Agent,
}

impl Default for Egress {
    fn default() -> Self {
        Self::new()
    }
}

impl Egress {
    pub fn new() -> Self {
        Self::with_timeouts(Timeouts::default())
    }

    /// As [`Egress::new`], with different bounds on how long a request may take.
    ///
    /// Exists so the bounds can be exercised in a test at a scale a test can wait for. Nothing in
    /// the product changes them: the defaults are the product's.
    pub fn with_timeouts(timeouts: Timeouts) -> Self {
        let config = ureq::Agent::config_builder()
            // Redirects are handled here so each hop can be revalidated; letting the
            // client follow them silently would defeat the gate.
            .max_redirects(0)
            .timeout_resolve(Some(timeouts.resolve))
            .timeout_connect(Some(timeouts.connect))
            .timeout_send_request(Some(timeouts.send))
            .timeout_send_body(Some(timeouts.send))
            // ureq keeps checking this one while the body arrives, so it bounds the whole
            // reply rather than only its headers.
            .timeout_recv_response(Some(timeouts.reply))
            // Recomputed on every read, which is what makes it a gap rather than a total.
            .timeout_recv_body(Some(timeouts.idle))
            .build();
        Self {
            agent: config.into(),
        }
    }

    /// Send a request, checking the policy before the initial URL and before every
    /// redirect hop.
    ///
    /// `label` is the label the response body carries. It comes from the caller's
    /// capability, not from anything the server says.
    pub fn fetch<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        request: Request,
        label: Label,
    ) -> Result<Response, EgressError> {
        let (status, content_type, url, reader) = self.fetch_checked(policy, &request)?;
        let (body, truncated) = read_capped(reader).map_err(|e| EgressError::Transport {
            url: url.clone(),
            detail: e.to_string(),
            transient: is_transient_io(&e),
        })?;

        Ok(Response {
            status,
            content_type,
            body: Labelled::new(body, label),
            truncated,
            final_url: url,
        })
    }

    /// As [`Egress::fetch`], but handing back the body to read as it arrives.
    ///
    /// Every gate is the same and runs at the same point: the policy is checked before the initial
    /// URL and before every redirect hop, before any body exists. What differs is only that the
    /// caller reads the body in pieces, so a long reply can be shown while it is still being
    /// written. The label is fixed here, from the caller's capability, so no piece of the stream
    /// can arrive better labelled than the whole would have been.
    pub fn fetch_streaming<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        request: Request,
        label: Label,
    ) -> Result<Streamed<'static>, EgressError> {
        let (status, content_type, url, reader) = self.fetch_checked(policy, &request)?;

        Ok(Streamed {
            status,
            content_type,
            final_url: url,
            label,
            reader,
            read: 0,
        })
    }

    /// Send, following and revalidating redirects, and return the body reader unread.
    ///
    /// The single place the gate is applied, so a streamed request cannot take a different path
    /// through the checks than a buffered one.
    #[allow(clippy::type_complexity)]
    fn fetch_checked<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        request: &Request,
    ) -> Result<(u16, Option<String>, String, Box<dyn std::io::Read>), EgressError> {
        let mut url = request.url.clone();
        let mut hops = 0;

        loop {
            require_http_scheme(&url)?;
            policy.before_network(&url)?;

            let response = self.send_once(request, &url)?;
            let status = response.0;

            if is_redirect(status) {
                if hops >= MAX_REDIRECTS {
                    return Err(EgressError::TooManyRedirects {
                        url: request.url.clone(),
                    });
                }
                let location = response
                    .1
                    .ok_or_else(|| EgressError::MissingLocation { url: url.clone() })?;
                // Resolved against the current URL so a relative Location is checked
                // as the absolute URL it will actually resolve to.
                url = resolve(&url, &location)?;
                hops += 1;
                continue;
            }

            if !(200..300).contains(&status) {
                return Err(EgressError::Status { url, status });
            }

            return Ok((status, response.2, url, response.3));
        }
    }

    /// One hop. Returns status, location, content-type, and the body reader.
    #[allow(clippy::type_complexity)]
    fn send_once(
        &self,
        request: &Request,
        url: &str,
    ) -> Result<(u16, Option<String>, Option<String>, Box<dyn std::io::Read>), EgressError> {
        // GET and POST builders have different types in ureq, so the header loop is
        // repeated rather than abstracted over them.
        let result = match request.method {
            Method::Get => {
                let mut builder = self.agent.get(url);
                for (name, value) in &request.headers {
                    builder = builder.header(name, value);
                }
                builder.call()
            }
            Method::Post => {
                let mut builder = self.agent.post(url);
                for (name, value) in &request.headers {
                    builder = builder.header(name, value);
                }
                match &request.body {
                    Some(bytes) => builder.send(&bytes[..]),
                    None => builder.send_empty(),
                }
            }
        };

        let response = match result {
            Ok(r) => r,
            // A redirect with max_redirects(0) is returned as a response, not an
            // error, so anything here is a genuine transport failure.
            Err(e) => {
                return Err(EgressError::Transport {
                    url: url.to_string(),
                    detail: e.to_string(),
                    transient: is_transient_call(&e),
                });
            }
        };

        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let location = header("location");
        let content_type = header("content-type");

        Ok((
            status,
            location,
            content_type,
            Box::new(response.into_body().into_reader()),
        ))
    }
}

/// Whether a failure to send or to read the reply is worth another attempt.
///
/// Everything here is the connection giving out: a timeout, a socket that went away, a name
/// that did not resolve because the machine's network was not up yet. A protocol error or a
/// malformed URL is the request being wrong, and sending it again would be wrong again.
fn is_transient_call(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => true,
        ureq::Error::Io(e) => is_transient_io(e),
        _ => false,
    }
}

fn is_transient_io(error: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(
        error.kind(),
        TimedOut
            | Interrupted
            | UnexpectedEof
            | ConnectionReset
            | ConnectionAborted
            | BrokenPipe
            | NotConnected
            | WouldBlock
    )
}

fn is_redirect(status: u16) -> bool {
    (300..400).contains(&status)
}

/// Reject anything that is not http(s) before it reaches the client, so a `file://` or
/// similar cannot be used to read local data through the network path.
fn require_http_scheme(url: &str) -> Result<(), EgressError> {
    if url.starts_with("https://") || url.starts_with("http://") {
        return Ok(());
    }
    Err(EgressError::InvalidUrl {
        url: url.to_string(),
        detail: "only http and https are permitted".into(),
    })
}

/// Resolve a `Location` value against the URL it came from.
///
/// Handles absolute, scheme-relative, path-absolute, and relative forms, because a
/// server can use any of them and each must be checked as the absolute URL it becomes.
fn resolve(base: &str, location: &str) -> Result<String, EgressError> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.to_string());
    }

    let separator = base.find("://").ok_or_else(|| EgressError::InvalidUrl {
        url: base.to_string(),
        detail: "no scheme".into(),
    })?;
    // Scheme without its "://", then everything after it.
    let scheme = &base[..separator];
    let rest = &base[separator + 3..];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];

    // "//host/path" keeps the current scheme but replaces the authority.
    if let Some(host_and_path) = location.strip_prefix("//") {
        return Ok(format!("{scheme}://{host_and_path}"));
    }

    if location.starts_with('/') {
        return Ok(format!("{scheme}://{authority}{location}"));
    }

    let path = &rest[authority_end..];
    let parent = match path.rfind('/') {
        Some(index) => &path[..=index],
        None => "/",
    };
    Ok(format!("{scheme}://{authority}{parent}{location}"))
}

/// Read at most [`MAX_RESPONSE_BYTES`], reporting whether the cap was hit.
///
/// Truncation is size hygiene, not filtering: nothing is inspected, and the caller
/// still receives the bytes labelled.
///
/// A read that fails is an error rather than a short body. The two are indistinguishable once
/// the bytes are handed back, and treating a cut-off reply as a complete one turns a transport
/// failure into whatever the truncated bytes happen to parse as.
fn read_capped(mut reader: Box<dyn std::io::Read>) -> Result<(Vec<u8>, bool), std::io::Error> {
    use std::io::Read;
    let mut buffer = Vec::new();
    let mut limited = reader.by_ref().take(MAX_RESPONSE_BYTES as u64 + 1);
    limited.read_to_end(&mut buffer)?;

    if buffer.len() > MAX_RESPONSE_BYTES {
        buffer.truncate(MAX_RESPONSE_BYTES);
        return Ok((buffer, true));
    }
    Ok((buffer, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The classification a retry rests on. Getting it wrong in one direction repeats a request
    /// that will fail identically, and in the other abandons one that would have worked.
    #[test]
    fn a_connection_that_gave_out_is_worth_another_attempt_and_a_refusal_is_not() {
        let dead = EgressError::Transport {
            url: "https://example.com".into(),
            detail: "timeout".into(),
            transient: true,
        };
        assert!(dead.is_transient());

        let wrong = EgressError::Transport {
            url: "https://example.com".into(),
            detail: "malformed http".into(),
            transient: false,
        };
        assert!(!wrong.is_transient());

        assert!(
            !EgressError::InvalidUrl {
                url: "gopher://example.com".into(),
                detail: "scheme".into(),
            }
            .is_transient()
        );
    }

    /// A server saying "not now" is temporary; a server saying "no" is not.
    #[test]
    fn only_the_statuses_that_mean_not_now_are_worth_another_attempt() {
        let at = |status| {
            EgressError::Status {
                url: "https://example.com".into(),
                status,
            }
            .is_transient()
        };

        assert!(at(429));
        assert!(at(503));
        assert!(at(504));
        assert!(!at(400));
        assert!(!at(401));
        assert!(!at(404));
    }

    #[test]
    fn only_http_schemes_are_permitted() {
        assert!(require_http_scheme("https://example.com").is_ok());
        assert!(require_http_scheme("http://example.com").is_ok());
        assert!(require_http_scheme("file:///etc/passwd").is_err());
        assert!(require_http_scheme("ftp://example.com").is_err());
        assert!(require_http_scheme("gopher://example.com").is_err());
    }

    #[test]
    fn absolute_redirects_are_used_as_given() {
        assert_eq!(
            resolve("https://a.example/x", "https://b.example/y").unwrap(),
            "https://b.example/y"
        );
    }

    #[test]
    fn path_absolute_redirects_keep_the_authority() {
        assert_eq!(
            resolve("https://a.example/x/y", "/z").unwrap(),
            "https://a.example/z"
        );
    }

    #[test]
    fn relative_redirects_resolve_against_the_parent_path() {
        assert_eq!(
            resolve("https://a.example/x/y", "z").unwrap(),
            "https://a.example/x/z"
        );
    }

    #[test]
    fn scheme_relative_redirects_keep_the_scheme() {
        assert_eq!(
            resolve("https://a.example/x", "//b.example/y").unwrap(),
            "https://b.example/y"
        );
    }

    #[test]
    fn redirect_status_codes_are_recognised() {
        assert!(is_redirect(301));
        assert!(is_redirect(302));
        assert!(is_redirect(307));
        assert!(!is_redirect(200));
        assert!(!is_redirect(404));
    }

    #[test]
    fn bodies_are_capped() {
        let oversized = vec![b'x'; MAX_RESPONSE_BYTES + 100];
        let (body, truncated) =
            read_capped(Box::new(std::io::Cursor::new(oversized))).expect("the read succeeds");
        assert_eq!(body.len(), MAX_RESPONSE_BYTES);
        assert!(truncated);
    }

    #[test]
    fn small_bodies_are_not_reported_as_truncated() {
        let (body, truncated) = read_capped(Box::new(std::io::Cursor::new(b"hello".to_vec())))
            .expect("the read succeeds");
        assert_eq!(body, b"hello");
        assert!(!truncated);
    }

    /// The distinction that matters to a caller: a body that stopped early is not a short body,
    /// and handing back what arrived would leave nothing to tell them apart by.
    #[test]
    fn a_failed_read_is_not_a_short_body() {
        struct Interrupted;
        impl std::io::Read for Interrupted {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "gone"))
            }
        }

        assert!(read_capped(Box::new(Interrupted)).is_err());
    }
}
