//! Pictures made by an image model for the tutor's `generate_image` tool
//! (docs/24): OpenAI's Images API (`POST /v1/images/generations`) or an
//! endpoint that speaks it (LiteLLM, a local server). It is sent a text
//! prompt only, never a picture from the game (`12-content-policy.md`
//! rule 8).

use serde_json::{Value, json};

use crate::http::{Cancel, HttpError, Transport, auth};
use crate::provider::{Endpoint, HttpRequest};

/// The sizes every image model takes.
pub const SIZES: &[&str] = &["1024x1024", "1536x1024", "1024x1536"];

pub fn request(
    e: &Endpoint,
    model: &str,
    prompt: &str,
    size: &str,
    key: Option<&str>,
) -> HttpRequest {
    let size = if SIZES.contains(&size) {
        size
    } else {
        SIZES[0]
    };
    let body = json!({"model": model, "prompt": prompt, "size": size, "n": 1});
    let mut headers = vec![("content-type".to_owned(), "application/json".to_owned())];
    if let Some(k) = key.filter(|k| !k.is_empty()) {
        headers.push(auth(e.protocol, k));
    }
    HttpRequest {
        url: e.url("images/generations"),
        headers,
        body: serde_json::to_vec(&body).expect("JSON serialises"),
    }
}

/// A picture, and the prompt the model says it drew from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generated {
    pub png: Vec<u8>,
    pub revised_prompt: Option<String>,
}

pub fn parse(body: &[u8]) -> Result<Generated, String> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| format!("the image service answered something else: {e}"))?;
    let d = &v["data"][0];
    let b64 = d["b64_json"]
        .as_str()
        .ok_or("the image service sent no picture (a URL-only service is not supported)")?;
    Ok(Generated {
        png: base64_decode(b64).ok_or("the picture did not decode")?,
        revised_prompt: d["revised_prompt"].as_str().map(str::to_owned),
    })
}

pub fn generate(
    t: &dyn Transport,
    e: &Endpoint,
    model: &str,
    prompt: &str,
    size: &str,
    key: Option<&str>,
    cancel: &Cancel,
) -> Result<Generated, String> {
    let req = request(e, model, prompt, size, key);
    let mut body = Vec::new();
    t.post(&req, cancel, &mut |b| body.extend_from_slice(b))
        .map_err(|e| match e {
            HttpError::Cancelled => "stopped".to_owned(),
            other => other.to_string(),
        })?;
    parse(&body)
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
        if chunk.len() == 1 {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::FakeTransport;
    use crate::provider::base64;

    #[test]
    fn base64_round_trips() {
        for n in 0..40u8 {
            let b: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(base64_decode(&base64(&b)).unwrap(), b);
        }
        assert!(base64_decode("A").is_none());
        assert!(base64_decode("@@@@").is_none());
    }

    #[test]
    fn a_picture_is_asked_for_and_read() {
        let png = b"\x89PNG fake".to_vec();
        let answer = format!(
            r#"{{"created":1,"data":[{{"b64_json":"{}","revised_prompt":"a diagram"}}]}}"#,
            base64(&png)
        );
        let t = FakeTransport::new(vec![Ok(answer.into_bytes())]);
        let g = generate(
            &t,
            &Endpoint::openai(),
            "gpt-image-1",
            "a diagram of the SNES memory map",
            "big",
            Some("sk"),
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(g.png, png);
        assert_eq!(g.revised_prompt.as_deref(), Some("a diagram"));
        let sent = &t.sent()[0];
        assert_eq!(sent.url, "https://api.openai.com/v1/images/generations");
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(body["size"], "1024x1024", "an unknown size falls back");
        assert_eq!(body["prompt"], "a diagram of the SNES memory map");
        assert!(
            sent.headers
                .contains(&("authorization".into(), "Bearer sk".into()))
        );
        assert!(
            parse(br#"{"data":[{"url":"https://x"}]}"#)
                .unwrap_err()
                .contains("URL")
        );
    }
}
