//! [`Endpoint`]: a provider base URL, parsed without a URL crate.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::error::AiError;

/// A parsed `http(s)://host[:port][/path]` base URL (no user info, query or
/// fragment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    scheme: String,
    host: String,
    port: Option<u16>,
    path: String,
}

impl Endpoint {
    /// Parses a base URL.
    ///
    /// # Errors
    /// [`AiError::InvalidSettings`] for anything but a plain `http`/`https`
    /// (or internal `mock`) URL.
    pub fn parse(url: &str) -> Result<Self, AiError> {
        let bad = |why: &str| AiError::InvalidSettings(format!("invalid endpoint '{url}': {why}"));
        let url = url.trim();
        let (scheme, rest) = url
            .split_once("://")
            .ok_or_else(|| bad("expected http://… or https://…"))?;
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https" | "mock") {
            return Err(bad("the scheme must be http or https"));
        }
        if rest.contains(['?', '#']) {
            return Err(bad("no query string or fragment"));
        }
        if rest.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(bad("no spaces or control characters"));
        }
        let (authority, path) = match rest.find('/') {
            Some(i) => (
                rest.get(..i).unwrap_or_default(),
                rest.get(i..).unwrap_or(""),
            ),
            None => (rest, ""),
        };
        if authority.contains('@') {
            return Err(bad(
                "credentials in the URL are not allowed (keys go in the OS keychain)",
            ));
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, tail) = v6
                .split_once(']')
                .ok_or_else(|| bad("unterminated IPv6 address"))?;
            let port = match tail {
                "" => None,
                p => Some(p.strip_prefix(':').ok_or_else(|| bad("bad port"))?),
            };
            h.parse::<Ipv6Addr>().map_err(|_| bad("bad IPv6 address"))?;
            (format!("[{h}]"), port)
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_owned(), Some(p)),
                None => (authority.to_owned(), None),
            }
        };
        let host = host.to_ascii_lowercase();
        if host.is_empty()
            || !host.starts_with('[')
                && !host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            return Err(bad("bad host name"));
        }
        let port = port
            .map(|p| p.parse::<u16>().map_err(|_| bad("bad port")))
            .transpose()?;
        Ok(Self {
            scheme,
            host,
            port,
            path: path.trim_end_matches('/').to_owned(),
        })
    }

    /// `mock://<id>`: the endpoint of the mock provider.
    #[must_use]
    pub fn mock(id: &str) -> Self {
        Self {
            scheme: "mock".into(),
            host: "mock".into(),
            port: None,
            path: format!("/{id}"),
        }
    }

    /// `http`, `https` or `mock`.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// Host name (lower case; IPv6 in brackets).
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// `host[:port]`: what a consent is bound to.
    #[must_use]
    pub fn host_port(&self) -> String {
        match self.port {
            Some(p) => format!("{}:{p}", self.host),
            None => self.host.clone(),
        }
    }

    /// The host is this machine (`localhost`, `127.0.0.0/8`, `::1`).
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        let h = self.host.trim_start_matches('[').trim_end_matches(']');
        h == "localhost"
            || h.ends_with(".localhost")
            || h.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
            || h.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
    }

    /// Requests stay on this machine (loopback or the mock provider).
    #[must_use]
    pub fn is_local(&self) -> bool {
        self.scheme == "mock" || self.is_loopback()
    }

    /// The base URL without a trailing slash.
    #[must_use]
    pub fn base(&self) -> String {
        format!("{}://{}{}", self.scheme, self.host_port(), self.path)
    }

    /// `base` + `/` + `path`.
    #[must_use]
    pub fn join(&self, path: &str) -> String {
        format!("{}/{}", self.base(), path.trim_start_matches('/'))
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_base_urls() {
        let e =
            Endpoint::parse("https://API.openai.com/v1/").unwrap_or_else(|_| Endpoint::mock(""));
        assert_eq!(e.host_port(), "api.openai.com");
        assert_eq!(
            e.join("chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert!(!e.is_local());
        let e = Endpoint::parse("http://127.0.0.1:11434").unwrap_or_else(|_| Endpoint::mock(""));
        assert_eq!(e.host_port(), "127.0.0.1:11434");
        assert!(e.is_loopback());
        assert_eq!(e.join("/api/chat"), "http://127.0.0.1:11434/api/chat");
        let e = Endpoint::parse("http://[::1]:8000/v1").unwrap_or_else(|_| Endpoint::mock(""));
        assert_eq!(e.host_port(), "[::1]:8000");
        assert!(e.is_loopback());
        assert!(Endpoint::parse("http://localhost:1234/v1").is_ok_and(|e| e.is_loopback()));
    }

    #[test]
    fn refuses_odd_urls() {
        for bad in [
            "api.openai.com",
            "ftp://x",
            "https://user:pw@host/v1",
            "https://host/v1?key=abc",
            "https://host:99999",
            "https://ho st",
            "https://",
            "https://[::1",
        ] {
            assert!(Endpoint::parse(bad).is_err(), "{bad}");
        }
    }
}
