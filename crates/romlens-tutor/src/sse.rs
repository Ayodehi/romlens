//! Server-sent events, read as the bytes arrive. All three protocols stream
//! this way: Anthropic names each event (`event: content_block_delta`),
//! OpenAI's Responses repeats the name in the data's `type`, and Chat
//! Completions sends bare `data:` lines ending in `data: [DONE]`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub event: Option<String>,
    pub data: String,
}

/// Feeds bytes in any chunks and hands back each event once its blank line
/// has arrived.
#[derive(Default)]
pub struct Reader {
    pending: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl Reader {
    pub fn new() -> Reader {
        Reader::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(n) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=n).collect();
            let mut line = &line[..line.len() - 1];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            self.line(&String::from_utf8_lossy(line), &mut out);
        }
        out
    }

    /// The stream has ended. An event whose lines all arrived counts even
    /// without its blank line; a line cut off part way is dropped, with its
    /// event, so a stream cut short reads as cut short.
    pub fn finish(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        if !self.pending.is_empty() {
            self.pending.clear();
            self.event = None;
            self.data.clear();
        }
        self.line("", &mut out);
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<Event>) {
        if line.is_empty() {
            if !self.data.is_empty() || self.event.is_some() {
                out.push(Event {
                    event: self.event.take(),
                    data: std::mem::take(&mut self.data).join("\n"),
                });
            }
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.find(':') {
            Some(i) => {
                let v = &line[i + 1..];
                (&line[..i], v.strip_prefix(' ').unwrap_or(v))
            }
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_events_split_anywhere() {
        let text =
            "event: a\ndata: {\"x\":1}\n\n: ping\r\ndata: one\r\ndata: two\r\n\r\ndata: [DONE]\n";
        for split in 0..text.len() {
            let mut r = Reader::new();
            let mut got = r.feed(&text.as_bytes()[..split]);
            got.extend(r.feed(&text.as_bytes()[split..]));
            got.extend(r.finish());
            assert_eq!(
                got,
                vec![
                    Event {
                        event: Some("a".into()),
                        data: "{\"x\":1}".into()
                    },
                    Event {
                        event: None,
                        data: "one\ntwo".into()
                    },
                    Event {
                        event: None,
                        data: "[DONE]".into()
                    },
                ],
                "split at {split}"
            );
        }
    }

    #[test]
    fn a_line_cut_off_is_dropped() {
        let mut r = Reader::new();
        assert!(r.feed(b"data: one\n\ndata: {\"half").len() == 1);
        assert!(r.finish().is_empty());
    }
}
