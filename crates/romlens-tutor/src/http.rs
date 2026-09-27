//! Sending requests (docs/24, "The transport"). The real transport is ureq
//! over rustls, checking certificates with the platform's own verifier and
//! reading the stream on the caller's thread; the fake one replays recorded
//! streams, for tests. Retries, the key and cancelling are here, so the
//! protocols stay pure.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::provider::{Endpoint, HttpRequest, Protocol, anthropic};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HttpError {
    #[error("HTTP {status}: {message}")]
    Status {
        status: u16,
        message: String,
        retry_after: Option<u64>,
    },
    #[error("{0}")]
    Network(String),
    #[error("stopped")]
    Cancelled,
}

impl HttpError {
    /// Worth sending again: rate limits, overload, the server's own
    /// failures, and a connection that never got going.
    pub fn retryable(&self) -> bool {
        match self {
            HttpError::Status { status, .. } => {
                matches!(status, 408 | 409 | 429 | 500 | 502 | 503 | 504 | 529)
            }
            HttpError::Network(_) => true,
            HttpError::Cancelled => false,
        }
    }
}

/// Stops a request between chunks.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub fn reset(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub trait Transport: Send + Sync {
    /// Posts `req` and hands each chunk of the body to `sink` as it
    /// arrives. A status other than 2xx is an error carrying the body's
    /// message.
    fn post(
        &self,
        req: &HttpRequest,
        cancel: &Cancel,
        sink: &mut dyn FnMut(&[u8]),
    ) -> Result<(), HttpError>;

    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>, HttpError>;
}

/// The header that carries the key.
pub fn auth(protocol: Protocol, key: &str) -> (String, String) {
    match protocol {
        Protocol::Anthropic => ("x-api-key".into(), key.into()),
        Protocol::Responses | Protocol::Chat => ("authorization".into(), format!("Bearer {key}")),
    }
}

pub fn with_key(mut req: HttpRequest, protocol: Protocol, key: Option<&str>) -> HttpRequest {
    if let Some(k) = key.filter(|k| !k.is_empty()) {
        req.headers.push(auth(protocol, k));
    }
    req
}

/// The message in a provider's error body, or the body itself.
pub fn error_message(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        let e = &v["error"];
        if let Some(m) = e["message"]
            .as_str()
            .or(e.as_str())
            .or(v["message"].as_str())
        {
            return m.to_owned();
        }
    }
    text.chars().take(500).collect()
}

pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqTransport {
    pub fn new() -> UreqTransport {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(20)))
            // A long turn streams for minutes; only silence ends it.
            .timeout_recv_body(None)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .new_agent();
        UreqTransport { agent }
    }
}

fn status_error(status: u16, retry_after: Option<u64>, body: &[u8]) -> HttpError {
    HttpError::Status {
        status,
        message: error_message(body),
        retry_after,
    }
}

impl Transport for UreqTransport {
    fn post(
        &self,
        req: &HttpRequest,
        cancel: &Cancel,
        sink: &mut dyn FnMut(&[u8]),
    ) -> Result<(), HttpError> {
        let mut r = self.agent.post(&req.url);
        for (k, v) in &req.headers {
            r = r.header(k, v);
        }
        let resp = r
            .send(&req.body[..])
            .map_err(|e| HttpError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        let mut body = resp.into_body().into_reader();
        if !(200..300).contains(&status) {
            let mut b = Vec::new();
            let _ = body.take(64 * 1024).read_to_end(&mut b);
            return Err(status_error(status, retry_after, &b));
        }
        let mut buf = [0u8; 16 * 1024];
        loop {
            if cancel.is_cancelled() {
                return Err(HttpError::Cancelled);
            }
            let n = body
                .read(&mut buf)
                .map_err(|e| HttpError::Network(e.to_string()))?;
            if n == 0 {
                return Ok(());
            }
            sink(&buf[..n]);
        }
    }

    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>, HttpError> {
        let mut r = self.agent.get(url);
        for (k, v) in headers {
            r = r.header(k, v);
        }
        let resp = r.call().map_err(|e| HttpError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let mut b = Vec::new();
        resp.into_body()
            .into_reader()
            .take(8 * 1024 * 1024)
            .read_to_end(&mut b)
            .map_err(|e| HttpError::Network(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(status_error(status, None, &b));
        }
        Ok(b)
    }
}

/// Replays recorded responses in order, and keeps what was sent.
#[derive(Default)]
pub struct FakeTransport {
    replies: std::sync::Mutex<std::collections::VecDeque<Result<Vec<u8>, HttpError>>>,
    pub sent: std::sync::Mutex<Vec<HttpRequest>>,
    /// Bytes per chunk, to exercise the reader.
    pub chunk: usize,
}

impl FakeTransport {
    pub fn new(replies: Vec<Result<Vec<u8>, HttpError>>) -> FakeTransport {
        FakeTransport {
            replies: std::sync::Mutex::new(replies.into()),
            sent: Default::default(),
            chunk: 7,
        }
    }

    pub fn sent(&self) -> Vec<HttpRequest> {
        self.sent.lock().unwrap().clone()
    }
}

impl Transport for FakeTransport {
    fn post(
        &self,
        req: &HttpRequest,
        cancel: &Cancel,
        sink: &mut dyn FnMut(&[u8]),
    ) -> Result<(), HttpError> {
        self.sent.lock().unwrap().push(req.clone());
        let next = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(HttpError::Network("no more recorded replies".into())));
        let bytes = next?;
        for c in bytes.chunks(self.chunk.max(1)) {
            if cancel.is_cancelled() {
                return Err(HttpError::Cancelled);
            }
            sink(c);
        }
        Ok(())
    }

    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>, HttpError> {
        self.sent.lock().unwrap().push(HttpRequest {
            url: url.into(),
            headers: headers.to_vec(),
            body: Vec::new(),
        });
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(HttpError::Network("no more recorded replies".into())))
    }
}

/// The ids of the models an endpoint serves, newest first where it says.
pub fn list_models(
    t: &dyn Transport,
    endpoint: &Endpoint,
    key: Option<&str>,
) -> Result<Vec<String>, HttpError> {
    let mut headers = Vec::new();
    if endpoint.protocol == Protocol::Anthropic {
        headers.push((
            "anthropic-version".to_owned(),
            anthropic::VERSION.to_owned(),
        ));
    }
    if let Some(k) = key.filter(|k| !k.is_empty()) {
        headers.push(auth(endpoint.protocol, k));
    }
    let base = match endpoint.protocol {
        Protocol::Anthropic => endpoint.url("v1/models"),
        _ => endpoint.url("models"),
    };
    let mut ids = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let url = match (&after, endpoint.protocol) {
            (Some(a), Protocol::Anthropic) => format!("{base}?limit=1000&after_id={a}"),
            (None, Protocol::Anthropic) => format!("{base}?limit=1000"),
            _ => base.clone(),
        };
        let body = t.get(&url, &headers)?;
        let v: Value = serde_json::from_slice(&body)
            .map_err(|e| HttpError::Network(format!("the model list was not JSON: {e}")))?;
        for m in v["data"].as_array().into_iter().flatten() {
            if let Some(id) = m["id"].as_str() {
                ids.push(id.to_owned());
            }
        }
        match (v["has_more"].as_bool(), v["last_id"].as_str()) {
            (Some(true), Some(last)) if endpoint.protocol == Protocol::Anthropic => {
                after = Some(last.to_owned())
            }
            _ => return Ok(ids),
        }
    }
}

/// How long to wait before try `attempt` (1-based): the server's
/// `retry-after` when it gave one, else 1, 2, 4 units (a second each,
/// outside tests).
pub fn backoff(attempt: u32, e: Option<&HttpError>, unit: Duration) -> Duration {
    if let Some(HttpError::Status {
        retry_after: Some(s),
        ..
    }) = e
    {
        return Duration::from_secs((*s).min(60));
    }
    unit * (1 << (attempt.saturating_sub(1)).min(5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_what_the_provider_said() {
        assert_eq!(
            error_message(br#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#),
            "invalid x-api-key"
        );
        assert_eq!(
            error_message(br#"{"error":{"message":"Incorrect API key","code":"invalid_api_key"}}"#),
            "Incorrect API key"
        );
        assert_eq!(error_message(b"Bad Gateway"), "Bad Gateway");
        assert!(
            HttpError::Status {
                status: 529,
                message: String::new(),
                retry_after: None
            }
            .retryable()
        );
        assert!(
            !HttpError::Status {
                status: 401,
                message: String::new(),
                retry_after: None
            }
            .retryable()
        );
    }

    #[test]
    fn models_are_listed_page_by_page() {
        let t = FakeTransport::new(vec![
            Ok(br#"{"data":[{"id":"claude-opus-5"}],"has_more":true,"last_id":"claude-opus-5"}"#.to_vec()),
            Ok(br#"{"data":[{"id":"claude-haiku-4-5"}],"has_more":false,"last_id":"claude-haiku-4-5"}"#.to_vec()),
        ]);
        let ids = list_models(&t, &Endpoint::anthropic(), Some("k")).unwrap();
        assert_eq!(ids, ["claude-opus-5", "claude-haiku-4-5"]);
        let sent = t.sent();
        assert!(sent[1].url.ends_with("after_id=claude-opus-5"));
        assert!(sent[0].headers.contains(&("x-api-key".into(), "k".into())));

        let t = FakeTransport::new(vec![Ok(
            br#"{"object":"list","data":[{"id":"qwen3:32b","object":"model"}]}"#.to_vec(),
        )]);
        let local = Endpoint {
            id: "ollama".into(),
            protocol: Protocol::Chat,
            base_url: "http://localhost:11434/v1".into(),
            tool_choice: false,
            strict: false,
            vision: false,
        };
        assert_eq!(list_models(&t, &local, None).unwrap(), ["qwen3:32b"]);
        assert_eq!(t.sent()[0].url, "http://localhost:11434/v1/models");
        assert!(t.sent()[0].headers.is_empty());
    }
}
