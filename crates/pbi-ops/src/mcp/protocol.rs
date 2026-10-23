//! MCP protocol constants, versions, JSON-RPC message helpers, log levels
//! and the progress mapping.

use serde_json::{Value, json};

/// Handshake-based ("legacy") revisions served after `initialize`, newest
/// first.
pub const LEGACY_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Stateless ("modern") revisions: the version travels in every request's
/// `_meta` (`io.modelcontextprotocol/protocolVersion`).
pub const MODERN_VERSIONS: [&str; 1] = ["2026-07-28"];

/// The revision `initialize` answers when the client asks for one we do not
/// serve.
pub const LATEST_LEGACY_VERSION: &str = LEGACY_VERSIONS[0];

/// `_meta` key of the per-request protocol version (modern revisions).
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// `_meta` key of the per-request log level (modern revisions).
pub const META_LOG_LEVEL: &str = "io.modelcontextprotocol/logLevel";
/// `_meta` key of the server identity in modern results.
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// JSON-RPC: the line is not JSON.
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC: not a valid request object.
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC: unknown method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC: invalid method parameters (also unknown tools).
pub const INVALID_PARAMS: i64 = -32602;
/// MCP 2026-07-28 `UnsupportedProtocolVersionError`.
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// The version `initialize` replies with: the client's when we serve it,
/// else [`LATEST_LEGACY_VERSION`].
#[must_use]
pub fn negotiate(requested: &str) -> &'static str {
    LEGACY_VERSIONS
        .iter()
        .find(|v| **v == requested)
        .copied()
        .unwrap_or(LATEST_LEGACY_VERSION)
}

/// Every revision served, newest first.
#[must_use]
pub fn supported_versions() -> Vec<&'static str> {
    MODERN_VERSIONS
        .iter()
        .chain(LEGACY_VERSIONS.iter())
        .copied()
        .collect()
}

/// The served revision equal to `v`, if any.
#[must_use]
pub fn known_version(v: &str) -> Option<&'static str> {
    MODERN_VERSIONS
        .iter()
        .chain(LEGACY_VERSIONS.iter())
        .find(|k| **k == v)
        .copied()
}

/// `true` for stateless revisions (per-request `_meta`, `resultType`).
#[must_use]
pub fn is_modern(version: &str) -> bool {
    MODERN_VERSIONS.contains(&version)
}

/// `true` when `version` is `since` or later (revisions are dates).
#[must_use]
pub fn at_least(version: &str, since: &str) -> bool {
    version >= since
}

pub(crate) fn result(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

pub(crate) fn error(id: &Value, code: i64, message: &str, data: Option<Value>) -> String {
    let mut error = json!({ "code": code, "message": message });
    if let (Some(data), Value::Object(map)) = (data, &mut error) {
        map.insert("data".into(), data);
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error }).to_string()
}

pub(crate) fn notification(method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}

/// Log levels of `logging/setLevel` and `notifications/message` (RFC 5424
/// severities, least severe first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Critical,
    Alert,
    Emergency,
}

impl LogLevel {
    const ALL: [LogLevel; 8] = [
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Notice,
        LogLevel::Warning,
        LogLevel::Error,
        LogLevel::Critical,
        LogLevel::Alert,
        LogLevel::Emergency,
    ];

    /// `debug`, `info`, …
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Notice => "notice",
            LogLevel::Warning => "warning",
            LogLevel::Error => "error",
            LogLevel::Critical => "critical",
            LogLevel::Alert => "alert",
            LogLevel::Emergency => "emergency",
        }
    }

    /// The level named `s`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == s)
    }
}

/// Turns command progress (a fraction, when known) into MCP progress
/// values: `0..=100` (`total: 100`, two decimals), strictly increasing as
/// the protocol requires. Events without a usable fraction advance by 1% of
/// what is left; once closed (the response is on its way) nothing more is
/// sent.
#[derive(Debug, Default)]
pub(crate) struct ProgressMeter {
    last: Option<f64>,
    closed: bool,
}

impl ProgressMeter {
    /// The value of the next notification, or `None` to skip it.
    pub(crate) fn next(&mut self, fraction: Option<f64>) -> Option<f64> {
        if self.closed {
            return None;
        }
        let target = fraction
            .filter(|f| f.is_finite())
            .map(|f| f.clamp(0.0, 1.0) * 100.0);
        let round = |v: f64| (v * 100.0).round() / 100.0;
        let raw = match (self.last, target) {
            (None, Some(t)) => t,
            (None, None) => 0.0,
            (Some(last), Some(t)) if t > last => t,
            (Some(last), _) => last + (100.0 - last) / 100.0,
        };
        let mut value = round(raw).min(100.0);
        if let Some(last) = self.last
            && value <= last
        {
            value = round(last + 0.01);
            if value <= last || value > 100.0 {
                return None;
            }
        }
        self.last = Some(value);
        Some(value)
    }

    /// No more notifications.
    pub(crate) fn close(&mut self) {
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_negotiate_to_a_served_legacy_revision() {
        assert_eq!(negotiate("2025-06-18"), "2025-06-18");
        assert_eq!(negotiate("2025-03-26"), "2025-03-26");
        assert_eq!(negotiate("2024-11-05"), "2024-11-05");
        assert_eq!(negotiate("1999-01-01"), LATEST_LEGACY_VERSION);
        assert_eq!(negotiate("2026-07-28"), LATEST_LEGACY_VERSION);
        assert!(at_least("2025-11-25", "2025-06-18"));
        assert!(!at_least("2025-03-26", "2025-06-18"));
        assert!(is_modern("2026-07-28") && !is_modern("2025-11-25"));
        assert_eq!(supported_versions().first(), Some(&"2026-07-28"));
    }

    #[test]
    fn progress_strictly_increases_and_stays_in_range() {
        let mut m = ProgressMeter::default();
        let inputs = [
            None,
            Some(0.0),
            Some(0.5),
            Some(0.25),
            None,
            Some(0.99),
            Some(1.0),
            Some(f64::NAN),
            Some(1.0),
        ];
        let mut out = Vec::new();
        for f in inputs {
            if let Some(v) = m.next(f) {
                out.push(v);
            }
        }
        assert!(out.windows(2).all(|w| w[1] > w[0]), "{out:?}");
        assert!(out.iter().all(|v| (0.0..=100.0).contains(v)), "{out:?}");
        assert_eq!(out.last().copied(), Some(100.0));
        m.close();
        assert_eq!(ProgressMeter::default().next(Some(0.5)), Some(50.0));
        assert_eq!(m.next(None), None);
    }

    #[test]
    fn log_levels_parse_and_order() {
        assert_eq!(LogLevel::parse("warning"), Some(LogLevel::Warning));
        assert_eq!(LogLevel::parse("verbose"), None);
        assert!(LogLevel::Debug < LogLevel::Error);
    }
}
