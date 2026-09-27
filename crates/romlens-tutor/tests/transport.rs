//! The real transport against a server on the loopback: the bytes arrive
//! as a stream, a status comes back with the provider's message, and Esc
//! stops a stream part way.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

use romlens_tutor::http::{Cancel, HttpError, Transport, UreqTransport};
use romlens_tutor::provider::HttpRequest;

/// Serves one request with `status` and `body` in `chunks` pieces, and
/// hands back what was asked.
fn serve(
    status: &'static str,
    body: Vec<u8>,
    chunks: usize,
) -> (String, thread::JoinHandle<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/chat/completions", l.local_addr().unwrap());
    let h = thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut head = String::new();
        let mut len = 0usize;
        loop {
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
            head.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut b = vec![0; len];
        r.read_exact(&mut b).unwrap();
        head.push_str(&String::from_utf8_lossy(&b));
        let mut s = s;
        write!(s, "HTTP/1.1 {status}\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nretry-after: 3\r\n\r\n").unwrap();
        let n = body.len().div_ceil(chunks.max(1)).max(1);
        for c in body.chunks(n) {
            write!(s, "{:x}\r\n", c.len()).unwrap();
            s.write_all(c).unwrap();
            s.write_all(b"\r\n").unwrap();
            s.flush().unwrap();
            thread::sleep(std::time::Duration::from_millis(5));
        }
        s.write_all(b"0\r\n\r\n").unwrap();
        head
    });
    (url, h)
}

fn req(url: &str) -> HttpRequest {
    HttpRequest {
        url: url.into(),
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("authorization".into(), "Bearer k".into()),
        ],
        body: br#"{"model":"m"}"#.to_vec(),
    }
}

#[test]
fn a_stream_arrives_in_pieces() {
    let body = std::fs::read(format!(
        "{}/tests/streams/chat_ollama.sse",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let (url, h) = serve("200 OK", body.clone(), 6);
    let mut got = Vec::new();
    let mut pieces = 0;
    UreqTransport::new()
        .post(&req(&url), &Cancel::new(), &mut |b| {
            pieces += 1;
            got.extend_from_slice(b);
        })
        .unwrap();
    assert_eq!(got, body);
    assert!(pieces > 1);
    let asked = h.join().unwrap();
    assert!(asked.starts_with("POST /v1/chat/completions"));
    assert!(
        asked
            .to_ascii_lowercase()
            .contains("authorization: bearer k")
    );
    assert!(asked.ends_with(r#"{"model":"m"}"#));
}

#[test]
fn a_status_says_why_and_when() {
    let (url, h) = serve(
        "429 Too Many Requests",
        br#"{"error":{"message":"slow down"}}"#.to_vec(),
        1,
    );
    let e = UreqTransport::new()
        .post(&req(&url), &Cancel::new(), &mut |_| {})
        .unwrap_err();
    assert_eq!(
        e,
        HttpError::Status {
            status: 429,
            message: "slow down".into(),
            retry_after: Some(3)
        }
    );
    h.join().unwrap();
}

#[test]
fn esc_stops_a_stream() {
    let (url, h) = serve("200 OK", vec![b'x'; 64 * 1024], 64);
    let cancel = Cancel::new();
    let mut seen = 0;
    let e = UreqTransport::new()
        .post(&req(&url), &cancel, &mut |b| {
            seen += b.len();
            cancel.cancel();
        })
        .unwrap_err();
    assert_eq!(e, HttpError::Cancelled);
    assert!(seen < 64 * 1024);
    let _ = h.join();
}
