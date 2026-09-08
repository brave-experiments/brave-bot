//! Taking the host out of a URL, for the gates that decide about one.
//!
//! A rule and a prompt are both about who is being talked to, so both need the host and neither
//! needs the rest. Parsing is deliberately the smallest thing that answers that question: no
//! dependency, no percent-decoding, and no attempt to normalise anything a comparison does not
//! depend on.
//!
//! The one property that matters is that this cannot report a host the request will not go to. A
//! parser that read `https://example.com@evil.test/` as `example.com` would hand a person the
//! wrong name to approve, so userinfo is dropped rather than mistaken for a host: everything
//! before the last `@` in the authority belongs to whoever wrote the URL, not to the destination.

/// The host in `url`, lowercased, or `None` where it names none.
///
/// The port is left out. A rule naming a host means the host whichever port it answers on, and a
/// person approving one is approving who they are talking to rather than a socket.
pub fn host_of(url: &str) -> Option<String> {
    let after_scheme = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .trim_start();

    // The authority ends at the first of these. Whichever comes first wins, so a `/` before a `?`
    // does not let a query string contribute to the host.
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);

    // Userinfo is not the host. The last `@` rather than the first, since a password may hold one
    // and the destination is what follows the final separator.
    let host_and_port = match authority.rsplit_once('@') {
        Some((_userinfo, host)) => host,
        None => authority,
    };

    let host = if let Some(rest) = host_and_port.strip_prefix('[') {
        // A bracketed IPv6 literal, whose colons are part of the address.
        rest.split_once(']').map(|(inside, _)| inside)?
    } else {
        host_and_port
            .split_once(':')
            .map(|(host, _port)| host)
            .unwrap_or(host_and_port)
    };

    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_is_taken_from_an_ordinary_url() {
        assert_eq!(
            host_of("https://example.com/docs"),
            Some("example.com".into())
        );
        assert_eq!(host_of("http://example.com"), Some("example.com".into()));
        assert_eq!(
            host_of("https://docs.example.com/a/b?c=d#e"),
            Some("docs.example.com".into())
        );
    }

    #[test]
    fn a_port_is_not_part_of_the_host() {
        assert_eq!(
            host_of("http://127.0.0.1:8080/path"),
            Some("127.0.0.1".into())
        );
        assert_eq!(
            host_of("https://example.com:443"),
            Some("example.com".into())
        );
    }

    /// A host is compared against rules written in lower case, and case says nothing in a URL, so
    /// the comparison must not depend on it.
    #[test]
    fn the_host_is_lowercased() {
        assert_eq!(host_of("https://EXAMPLE.com/X"), Some("example.com".into()));
    }

    /// The one that matters. Read as `example.com`, this URL would put the wrong name in front of
    /// a person and check a rule against a host the request never reaches.
    #[test]
    fn userinfo_is_not_mistaken_for_the_host() {
        assert_eq!(
            host_of("https://example.com@evil.test/path"),
            Some("evil.test".into())
        );
        assert_eq!(
            host_of("https://user:pass@example.com/path"),
            Some("example.com".into())
        );
        // A `@` inside the password does not move the boundary.
        assert_eq!(
            host_of("https://user:p@ss@real.test/"),
            Some("real.test".into())
        );
    }

    #[test]
    fn a_bracketed_address_keeps_its_colons() {
        assert_eq!(host_of("http://[::1]:8080/path"), Some("::1".into()));
        assert_eq!(
            host_of("https://[2001:db8::1]/x"),
            Some("2001:db8::1".into())
        );
    }

    #[test]
    fn a_url_with_no_host_reports_none() {
        assert_eq!(host_of("https://"), None);
        assert_eq!(host_of(""), None);
        assert_eq!(host_of("https:///just/a/path"), None);
        assert_eq!(host_of("file:///etc/passwd"), None);
    }

    /// A query or fragment before any slash must not become part of the host.
    #[test]
    fn the_authority_ends_at_the_first_delimiter() {
        assert_eq!(
            host_of("https://example.com?q=/evil.test"),
            Some("example.com".into())
        );
        assert_eq!(
            host_of("https://example.com#/evil.test"),
            Some("example.com".into())
        );
    }
}
