//! Redaction of secrets in metadata before it is stored in an
//! [`OpenedDocument`](crate::OpenedDocument).
//!
//! Rules (applied to every JSON metadata part: PBIX `Settings`, `Metadata`,
//! `DiagramLayout` and `Connections`, PBIP `.pbi/*.json`, `.pbids`, …):
//!
//! - The value of any object key whose lower-cased name contains
//!   `connectionstring`, `password`, `pwd`, `token`, `key`, `secret` or
//!   `credential` is replaced by [`REDACTED`] (strings, numbers, objects
//!   and arrays; `null` and booleans are kept).
//! - Any other string with `name=value` parameters (connection strings,
//!   URLs) has the value of secret parameters (`password`, `pwd`, `sig`,
//!   `accountkey`, `sharedaccesskey`, `token`, `client_secret`, …)
//!   replaced by [`REDACTED`], keeping the rest (`Data Source=…`) readable.
//!
//! Report `datasetReference.byConnection.connectionString` values get the
//! second rule ([`sanitize_connection_string`]), so the remote server and
//! model stay visible.

use serde_json::Value;

/// Replacement text for redacted values.
pub const REDACTED: &str = "<redacted>";

const SECRET_KEY_PARTS: &[&str] = &[
    "connectionstring",
    "password",
    "pwd",
    "token",
    "key",
    "secret",
    "credential",
];

const SECRET_PARAMS: &[&str] = &[
    "password",
    "pwd",
    "sig",
    "signature",
    "accountkey",
    "sharedaccesskey",
    "sharedaccesssignature",
    "token",
    "accesstoken",
    "refreshtoken",
    "clientsecret",
    "secret",
    "apikey",
    "key",
    "code",
];

/// `true` if a JSON key names a secret-bearing value.
#[must_use]
pub fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEY_PARTS.iter().any(|p| lower.contains(p))
}

fn is_secret_param(name: &str) -> bool {
    let norm: String = name
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect();
    SECRET_PARAMS.contains(&norm.as_str())
}

/// Masks the values of secret parameters in a connection string or URL
/// (`;`, `&` and `?` separate parameters): `Data Source=x;Password=y` →
/// `Data Source=x;Password=<redacted>`, `https://a/b?sv=1&sig=z` →
/// `https://a/b?sv=1&sig=<redacted>`.
#[must_use]
pub fn sanitize_connection_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let (segment, delimiter, tail) = match rest.find([';', '&', '?']) {
            Some(i) => {
                let (segment, tail) = rest.split_at(i);
                let mut chars = tail.chars();
                let delimiter = chars.next();
                (segment, delimiter, chars.as_str())
            }
            None => (rest, None, ""),
        };
        match segment.split_once('=') {
            Some((name, _)) if is_secret_param(name) => {
                out.push_str(name);
                out.push('=');
                out.push_str(REDACTED);
            }
            _ => out.push_str(segment),
        }
        match delimiter {
            Some(d) => {
                out.push(d);
                rest = tail;
            }
            None => break,
        }
    }
    out
}

/// Redacts `value` in place (see the [module docs](self)) and returns the
/// JSON pointers of the redacted values.
pub fn redact_json(value: &mut Value) -> Vec<String> {
    let mut redacted = Vec::new();
    walk(value, &mut String::new(), &mut redacted);
    redacted
}

fn walk(value: &mut Value, pointer: &mut String, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                let len = pointer.len();
                pointer.push('/');
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
                if is_secret_key(key) && !matches!(child, Value::Null | Value::Bool(_)) {
                    *child = Value::String(REDACTED.to_owned());
                    out.push(pointer.clone());
                } else {
                    walk(child, pointer, out);
                }
                pointer.truncate(len);
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter_mut().enumerate() {
                let len = pointer.len();
                pointer.push('/');
                pointer.push_str(&i.to_string());
                walk(child, pointer, out);
                pointer.truncate(len);
            }
        }
        Value::String(s) if s.contains('=') => {
            let clean = sanitize_connection_string(s);
            if clean != *s {
                *s = clean;
                out.push(pointer.clone());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    #[test]
    fn connection_strings_keep_the_server() {
        assert_eq!(
            sanitize_connection_string(
                "Data Source=srv;Initial Catalog=db;Password=p@ss;User ID=u"
            ),
            "Data Source=srv;Initial Catalog=db;Password=<redacted>;User ID=u"
        );
        assert_eq!(
            sanitize_connection_string(
                "https://acct.blob.core.windows.net/c?sv=2020-01-01&sig=abc%2F"
            ),
            "https://acct.blob.core.windows.net/c?sv=2020-01-01&sig=<redacted>"
        );
        assert_eq!(sanitize_connection_string("plain"), "plain");
    }

    #[test]
    fn secret_keys_and_values_are_redacted() {
        let mut v = json!({
            "Connections": [{
                "ConnectionString": "Data Source=x;pwd=y",
                "Name": "EntityDataSource",
                "PbiModelDatabaseName": "db"
            }],
            "AccessToken": null,
            "url": "https://h/p?sig=s",
            "IsKeyed": true
        });
        let pointers = redact_json(&mut v);
        assert_eq!(v["Connections"][0]["ConnectionString"], REDACTED);
        assert_eq!(v["Connections"][0]["Name"], "EntityDataSource");
        assert_eq!(v["AccessToken"], Value::Null);
        assert_eq!(v["url"], "https://h/p?sig=<redacted>");
        assert_eq!(v["IsKeyed"], true);
        assert_eq!(pointers, ["/Connections/0/ConnectionString", "/url"]);
    }
}
