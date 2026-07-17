//! AgentBar redacting structured logger.
//!
//! Never log raw `apiKey`, `cookieHeader`, bearer tokens, or similar secrets.
//! [`redact`] is pure and unit-tested; [`info`]/[`warn`]/[`error`] go through
//! the `log` facade after redaction.

use log::{Level, Record};
use std::sync::Once;

static INIT: Once = Once::new();

/// Install a simple stderr logger (idempotent). Safe to call from engine start.
pub fn init() {
    INIT.call_once(|| {
        let _ =
            log::set_logger(&RedactingLogger).map(|()| log::set_max_level(log::LevelFilter::Debug));
    });
}

struct RedactingLogger;

impl log::Log for RedactingLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let msg = format!("{}", record.args());
        let safe = redact(&msg);
        let target = record.target();
        eprintln!(
            "[{}] {} {}: {}",
            record.level(),
            chrono_lite_now(),
            target,
            safe
        );
    }

    fn flush(&self) {}
}

/// Minimal timestamp without extra deps (UTC-ish wall clock via SystemTime).
fn chrono_lite_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

/// Log at info with category `target` after redaction.
pub fn info(target: &str, msg: impl AsRef<str>) {
    init();
    let safe = redact(msg.as_ref());
    log::info!(target: target, "{safe}");
}

/// Log at warn with category after redaction.
pub fn warn(target: &str, msg: impl AsRef<str>) {
    init();
    let safe = redact(msg.as_ref());
    log::warn!(target: target, "{safe}");
}

/// Log at error with category after redaction.
pub fn error(target: &str, msg: impl AsRef<str>) {
    init();
    let safe = redact(msg.as_ref());
    log::error!(target: target, "{safe}");
}

/// Log at debug with category after redaction.
pub fn debug(target: &str, msg: impl AsRef<str>) {
    init();
    let safe = redact(msg.as_ref());
    log::debug!(target: target, "{safe}");
}

const REDACTED: &str = "***";

/// Secret-bearing key names (case-insensitive match on key side of `key=value` / JSON).
const SECRET_KEYS: &[&str] = &[
    "apikey",
    "api_key",
    "cookieheader",
    "cookie_header",
    "cookie",
    "authorization",
    "token",
    "access_token",
    "refresh_token",
    "secret",
    "password",
    "passwd",
    "session",
    "sessionkey",
    "session_key",
    "bearer",
    "credentials",
];

/// Redact secret-looking material in `input`. Pure; used by logger and tests.
pub fn redact(input: &str) -> String {
    let mut out = input.to_string();

    // JSON `"secretKey":"value"` / `"secretKey": "value"` first (quoted keys).
    out = redact_json_quoted_secrets(&out);

    // Bearer tokens
    out = redact_regex_like_bearer(&out);

    // key=value and key: value (unquoted keys)
    out = redact_key_values(&out);

    out
}

/// Redact `"apiKey":"…"`, `"cookieHeader": "…"`, etc.
fn redact_json_quoted_secrets(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            // Parse a JSON string token starting at i
            if let Some((key, key_end)) = parse_json_string(&chars, i) {
                let key_lower = key.to_ascii_lowercase();
                if SECRET_KEYS.iter().any(|k| *k == key_lower) {
                    // Copy `"key"`
                    for c in &chars[i..key_end] {
                        result.push(*c);
                    }
                    i = key_end;
                    // whitespace, colon, whitespace
                    while i < chars.len() && chars[i].is_whitespace() {
                        result.push(chars[i]);
                        i += 1;
                    }
                    if i < chars.len() && chars[i] == ':' {
                        result.push(':');
                        i += 1;
                        while i < chars.len() && chars[i].is_whitespace() {
                            result.push(chars[i]);
                            i += 1;
                        }
                        // value: string or bare
                        if i < chars.len() && chars[i] == '"' {
                            if let Some((_val, val_end)) = parse_json_string(&chars, i) {
                                result.push('"');
                                result.push_str(REDACTED);
                                result.push('"');
                                i = val_end;
                                continue;
                            }
                        } else if i < chars.len() {
                            while i < chars.len()
                                && !chars[i].is_whitespace()
                                && chars[i] != ','
                                && chars[i] != '}'
                            {
                                i += 1;
                            }
                            result.push_str(REDACTED);
                            continue;
                        }
                    }
                    // Malformed after secret key — fall through one char.
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Parse `"…"` starting at `i` (must be `"`). Returns (content, index after closing quote).
fn parse_json_string(chars: &[char], i: usize) -> Option<(String, usize)> {
    if i >= chars.len() || chars[i] != '"' {
        return None;
    }
    let mut j = i + 1;
    let mut content = String::new();
    while j < chars.len() {
        if chars[j] == '\\' && j + 1 < chars.len() {
            content.push(chars[j + 1]);
            j += 2;
            continue;
        }
        if chars[j] == '"' {
            return Some((content, j + 1));
        }
        content.push(chars[j]);
        j += 1;
    }
    None
}

fn redact_regex_like_bearer(s: &str) -> String {
    // Case-insensitive "Bearer <token>"
    let lower = s.to_ascii_lowercase();
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if lower[i..].starts_with("bearer ") {
            result.push_str(&s[i..i + 7]); // "Bearer " with original case
            i += 7;
            // skip token chars
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'"' {
                i += 1;
            }
            if i > start {
                result.push_str(REDACTED);
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

fn redact_key_values(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Try to match a secret key starting at i
        if let Some((key_end, sep_end)) = match_secret_key_at(&chars, i) {
            // copy key + separator
            for c in &chars[i..sep_end] {
                result.push(*c);
            }
            i = sep_end;
            // skip whitespace after sep
            while i < chars.len() && chars[i].is_whitespace() {
                result.push(chars[i]);
                i += 1;
            }
            // value may be quoted
            if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let quote = chars[i];
                result.push(quote);
                i += 1;
                while i < chars.len() && chars[i] != quote {
                    i += 1;
                }
                result.push_str(REDACTED);
                if i < chars.len() && chars[i] == quote {
                    result.push(quote);
                    i += 1;
                }
            } else {
                // unquoted value until whitespace, comma, brace, or end
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && chars[i] != ','
                    && chars[i] != '}'
                    && chars[i] != ']'
                {
                    i += 1;
                }
                result.push_str(REDACTED);
            }
            let _ = key_end;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// If `chars[i..]` starts with a secret key then `=` or `:`, return (key_end, sep_end).
fn match_secret_key_at(chars: &[char], i: usize) -> Option<(usize, usize)> {
    // Word boundary: start or previous is non-alphanumeric/_
    if i > 0 {
        let prev = chars[i - 1];
        if prev.is_ascii_alphanumeric() || prev == '_' {
            return None;
        }
    }
    let rest: String = chars[i..].iter().collect();
    let rest_lower = rest.to_ascii_lowercase();
    for key in SECRET_KEYS {
        if rest_lower.starts_with(key) {
            let key_end = i + key.len();
            if key_end < chars.len() {
                // next must be end of key (not more alnum)
                let next = chars[key_end];
                if next.is_ascii_alphanumeric() || next == '_' {
                    continue;
                }
                // skip whitespace
                let mut j = key_end;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() && (chars[j] == '=' || chars[j] == ':') {
                    return Some((key_end, j + 1));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_api_key_equals() {
        let s = redact("loading apiKey=sk-live-abc123xyz for codex");
        assert!(!s.contains("sk-live-abc123xyz"));
        assert!(s.contains("apiKey="));
        assert!(s.contains(REDACTED));
    }

    #[test]
    fn redacts_cookie_header_jsonish() {
        let s = redact(r#"provider cookieHeader: "session=abc; token=xyz""#);
        assert!(!s.contains("session=abc"));
        assert!(s.contains(REDACTED));
    }

    #[test]
    fn redacts_bearer_token() {
        // Standalone Bearer (no Authorization key) — token body redacted.
        let s = redact("header Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig ok");
        assert!(!s.contains("eyJhbGciOiJIUzI1NiJ9"));
        assert!(s.to_ascii_lowercase().contains("bearer"));
        assert!(s.contains(REDACTED));
        // Authorization key redacts entire value (including Bearer …).
        let s2 = redact("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig");
        assert!(!s2.contains("eyJhbGciOiJIUzI1NiJ9"));
        assert!(s2.contains(REDACTED));
    }

    #[test]
    fn leaves_benign_text() {
        let s = redact("refresh provider=codex ok usedPercent=42.5");
        assert_eq!(s, "refresh provider=codex ok usedPercent=42.5");
    }

    #[test]
    fn redacts_quoted_json_api_key() {
        let s = redact(r#"{"apiKey":"super-secret-value","id":"codex"}"#);
        assert!(!s.contains("super-secret-value"), "got: {s}");
        assert!(s.contains("codex"));
        assert!(s.contains(REDACTED));
    }
}
