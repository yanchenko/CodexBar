//! Blocking HTTPS client (rustls via attohttpc).
//!
//! Defaults: connect 10s, read/request 30s. No tokio. Used by provider strategies.
//! CI tests use `httpmock` — never hit real networks in unit tests.

use std::collections::HashMap;
use std::time::Duration;

/// Default TCP connect timeout.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Default per-read / request inactivity timeout.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Default max response body (8 MiB).
pub const DEFAULT_MAX_BODY: usize = 8 * 1024 * 1024;

/// HTTP client with timeouts and optional default headers.
#[derive(Clone, Debug)]
pub struct HttpClient {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub max_body: usize,
    pub user_agent: String,
    pub default_headers: HashMap<String, String>,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self {
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            read_timeout: DEFAULT_READ_TIMEOUT,
            max_body: DEFAULT_MAX_BODY,
            user_agent: format!("AgentBar/{}", env!("CARGO_PKG_VERSION")),
            default_headers: HashMap::new(),
        }
    }
}

/// Successful or failed HTTP response (status always set when transport OK).
#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn body_lossy(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Transport or protocol error.
#[derive(Debug)]
pub enum HttpError {
    Transport(String),
    BodyTooLarge { max: usize },
    Status { status: u16, body: Vec<u8> },
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Transport(s) => write!(f, "http transport: {s}"),
            HttpError::BodyTooLarge { max } => write!(f, "response body exceeds {max} bytes"),
            HttpError::Status { status, .. } => write!(f, "http status {status}"),
        }
    }
}

impl std::error::Error for HttpError {}

impl HttpClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// GET `url` with optional extra headers.
    pub fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        self.request("GET", url, headers, None)
    }

    /// POST `url` with body and optional extra headers.
    pub fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<HttpResponse, HttpError> {
        self.request("POST", url, headers, Some(body))
    }

    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<HttpResponse, HttpError> {
        let mut rb = match method {
            "GET" => attohttpc::get(url),
            "POST" => attohttpc::post(url),
            "PUT" => attohttpc::put(url),
            "DELETE" => attohttpc::delete(url),
            other => {
                return Err(HttpError::Transport(format!("unsupported method {other}")));
            }
        };
        rb = rb
            .connect_timeout(self.connect_timeout)
            .read_timeout(self.read_timeout)
            .header("User-Agent", self.user_agent.as_str());

        // Dynamic header names need owned HeaderName (IntoHeaderName is 'static for &str).
        for (k, v) in &self.default_headers {
            let name = http::header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| HttpError::Transport(e.to_string()))?;
            rb = rb.header(name, v.as_str());
        }
        for (k, v) in headers {
            let name = http::header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| HttpError::Transport(e.to_string()))?;
            rb = rb.header(name, *v);
        }

        let resp = if let Some(b) = body {
            rb.bytes(b)
                .send()
                .map_err(|e| HttpError::Transport(e.to_string()))?
        } else {
            rb.send().map_err(|e| HttpError::Transport(e.to_string()))?
        };

        let status = resp.status().as_u16();
        let mut hdrs = HashMap::new();
        for (name, value) in resp.headers() {
            if let Ok(v) = value.to_str() {
                hdrs.insert(name.as_str().to_string(), v.to_string());
            }
        }

        let bytes = resp
            .bytes()
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        if bytes.len() > self.max_body {
            return Err(HttpError::BodyTooLarge {
                max: self.max_body,
            });
        }

        Ok(HttpResponse {
            status,
            headers: hdrs,
            body: bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[test]
    fn get_json_from_mock() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/v1/usage");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"usedPercent":12.5}"#);
        });

        // httpmock is HTTP not HTTPS — attohttpc still works for http://
        let client = HttpClient {
            connect_timeout: Duration::from_secs(2),
            read_timeout: Duration::from_secs(2),
            ..HttpClient::default()
        };
        let url = server.url("/v1/usage");
        let resp = client.get(&url, &[]).expect("get");
        m.assert();
        assert!(resp.is_success());
        assert!(resp.body_lossy().contains("12.5"));
    }

    #[test]
    fn post_with_header() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST)
                .path("/rpc")
                .header("x-test", "1")
                .body(r#"{"ping":true}"#);
            then.status(201).body("ok");
        });

        let client = HttpClient::new();
        let url = server.url("/rpc");
        let resp = client
            .post(&url, &[("x-test", "1"), ("content-type", "application/json")], br#"{"ping":true}"#)
            .expect("post");
        m.assert();
        assert_eq!(resp.status, 201);
        assert_eq!(resp.body_lossy(), "ok");
    }

    #[test]
    fn not_found_still_returns_body() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/missing");
            then.status(404).body("nope");
        });
        let client = HttpClient::new();
        let resp = client.get(&server.url("/missing"), &[]).unwrap();
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body_lossy(), "nope");
        assert!(!resp.is_success());
    }
}
