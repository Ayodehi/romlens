//! Reading saved conversations back as text (`romlens tutor sessions` and
//! `romlens tutor show`): what the student asked, what went with it, what
//! the model thought and called, what the tools answered, and what it cost,
//! turn by turn. For working out why the tutor answered as it did.

use std::path::{Path, PathBuf};

use crate::store::{Meta, Store, StoreError};
use crate::transcript::{Block, Part, Role, Turn};

/// Where the app keeps conversations: `ROMLENS_TUTOR_DIR`, else the
/// sandboxed app's Application Support folder, else the unsandboxed one.
pub fn default_root() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("ROMLENS_TUTOR_DIR") {
        return Some(PathBuf::from(d));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let sandboxed = home.join(
        "Library/Containers/io.github.Ayodehi.Romlens/Data/Library/Application Support/Tutor",
    );
    let plain = home.join("Library/Application Support/Tutor");
    Some(if sandboxed.is_dir() || !plain.is_dir() {
        sandboxed
    } else {
        plain
    })
}

/// A saved conversation and the ROM it is about.
#[derive(Debug, Clone)]
pub struct Saved {
    /// The ROM's SHA-256, the folder it is in.
    pub rom: String,
    pub meta: Meta,
}

/// Every conversation under `root`, the latest first.
pub fn all(root: &Path) -> Vec<Saved> {
    let mut out: Vec<Saved> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .flat_map(|e| {
            let rom = e.file_name().to_string_lossy().into_owned();
            Store::new(root, &rom)
                .list()
                .into_iter()
                .map(move |meta| Saved {
                    rom: rom.clone(),
                    meta,
                })
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.meta.updated));
    out
}

/// The conversation `which` names: `latest`, an id, or the start of one.
pub fn find(root: &Path, which: &str) -> Result<Saved, String> {
    let every = all(root);
    if which == "latest" {
        return every
            .into_iter()
            .next()
            .ok_or_else(|| format!("no conversations under {}", root.display()));
    }
    let mut hits: Vec<Saved> = every
        .into_iter()
        .filter(|s| s.meta.id.starts_with(which))
        .collect();
    match hits.len() {
        0 => Err(format!(
            "no conversation `{which}` under {}",
            root.display()
        )),
        1 => Ok(hits.remove(0)),
        n => Err(format!("`{which}` is the start of {n} conversations")),
    }
}

pub fn load(root: &Path, s: &Saved) -> Result<Vec<Turn>, StoreError> {
    Ok(Store::new(root, &s.rom).load(&s.meta.id)?.0.turns)
}

/// One line for a conversation, for a list.
pub fn line(s: &Saved) -> String {
    let m = &s.meta;
    format!(
        "{}  {}  {:>3} turns  ${:.4}  {} on {}  {}  [{}]",
        m.id,
        when(m.updated),
        m.turns,
        m.cost,
        m.model,
        m.endpoint.id,
        m.title,
        &s.rom[..s.rom.len().min(8)]
    )
}

/// How much of each tool result and each question to print.
#[derive(Debug, Clone, Copy)]
pub struct Show {
    /// Lines of a tool result before it is cut; `None` for all of it.
    pub result_lines: Option<usize>,
    /// The fixed system prompt too.
    pub system: bool,
}

/// A conversation as text.
pub fn render(s: &Saved, turns: &[Turn], show: Show) -> String {
    let m = &s.meta;
    let mut o = String::new();
    o.push_str(&format!("# {}\n\n", m.title));
    o.push_str(&format!(
        "id {}  ROM {}\n{} on {} ({}), effort {}, mode {}\ncreated {}, updated {}; {} turns, ${:.4}{}\n",
        m.id,
        s.rom,
        m.model,
        m.endpoint.id,
        m.endpoint.base_url,
        m.effort.as_deref().unwrap_or("default"),
        m.mode.name(),
        when(m.created),
        when(m.updated),
        m.turns,
        m.cost,
        m.cost_cap
            .map(|c| format!(" of a ${c:.2} cap"))
            .unwrap_or_default()
    ));
    if show.system {
        o.push_str("\n## System prompt\n\n");
        o.push_str(&m.system);
        o.push_str("\n\n## ROM digest\n\n");
        o.push_str(&m.digest);
        o.push('\n');
    }
    for (i, t) in turns.iter().enumerate() {
        let who = match t.role {
            Role::User
                if t.blocks
                    .iter()
                    .all(|b| matches!(b, Block::ToolResult { .. })) =>
            {
                "tool results"
            }
            Role::User => "student",
            Role::Assistant => "tutor",
        };
        o.push_str(&format!("\n## {} · {who}", i + 1));
        if t.role == Role::Assistant {
            let model = t.native.as_ref().map(|n| n.model.as_str()).unwrap_or("?");
            let u = &t.usage;
            o.push_str(&format!(
                "  ({model}; in {} + cache read {} + cache write {}, out {}; ${:.4})",
                u.input, u.cache_read, u.cache_write, u.output, t.cost
            ));
        }
        if !t.sent {
            o.push_str("  [not sent: rewound or compacted]");
        }
        o.push('\n');
        for b in &t.blocks {
            match b {
                Block::Text { text } => {
                    o.push('\n');
                    o.push_str(text.trim_end());
                    o.push('\n');
                }
                Block::Image { image } => {
                    o.push_str(&format!("\n[picture {} {}]\n", image.id, image.media_type));
                }
                Block::Reasoning { summary } => {
                    o.push_str("\n> thinking: ");
                    o.push_str(&summary.trim().replace('\n', "\n> "));
                    o.push('\n');
                }
                Block::ToolCall { id, name, input } => {
                    o.push_str(&format!("\n→ {name} {input}  ({})\n", short(id)));
                }
                Block::ToolResult {
                    id,
                    parts,
                    is_error,
                } => {
                    o.push_str(&format!(
                        "\n← {}{}\n",
                        short(id),
                        if *is_error { " ERROR" } else { "" }
                    ));
                    for p in parts {
                        match p {
                            Part::Text { text } => o.push_str(&indent(text, show.result_lines)),
                            Part::Image { image } => {
                                o.push_str(&format!("    [picture {}]\n", image.id))
                            }
                        }
                    }
                }
                Block::Note { text } => o.push_str(&format!("\n— {text}\n")),
            }
        }
    }
    o
}

fn short(id: &str) -> &str {
    &id[id.len().saturating_sub(8)..]
}

fn indent(text: &str, limit: Option<usize>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let keep = limit.unwrap_or(usize::MAX).min(lines.len());
    let mut o: String = lines[..keep].iter().map(|l| format!("    {l}\n")).collect();
    if keep < lines.len() {
        o.push_str(&format!(
            "    … {} more lines (--full shows them)\n",
            lines.len() - keep
        ));
    }
    o
}

/// Seconds since 1970 as a UTC date and time.
fn when(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!(
        "{y:04}-{mo:02}-{d:02} {:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Session;
    use crate::provider::Endpoint;
    use crate::transcript::Usage;
    use serde_json::json;

    #[test]
    fn a_date_reads_as_a_date() {
        assert_eq!(when(0), "1970-01-01 00:00Z");
        assert_eq!(when(1_790_545_983), "2026-09-27 21:53Z");
    }

    #[test]
    fn a_saved_conversation_reads_back() {
        let root = std::env::temp_dir().join(format!("romlens-review-{}", std::process::id()));
        let store = Store::new(&root, "abc123");
        let mut s = Session::new("c1-aa", Endpoint::anthropic(), "claude-opus-5");
        s.turns = vec![
            Turn::user_text("What is ADDR_7F8000?"),
            Turn {
                role: Role::Assistant,
                blocks: vec![
                    Block::Reasoning {
                        summary: "Look at the C.".into(),
                    },
                    Block::ToolCall {
                        id: "toolu_1".into(),
                        name: "decompile".into(),
                        input: json!({"address": "$00:8000", "level": "full"}),
                    },
                ],
                native: None,
                usage: Usage {
                    input: 4,
                    output: 10,
                    cache_read: 0,
                    cache_write: 100,
                },
                cost: 0.01,
                sent: true,
            },
            Turn {
                sent: false,
                ..Turn::user(vec![Block::ToolResult {
                    id: "toolu_1".into(),
                    parts: vec![Part::Text {
                        text: (0..50).map(|i| format!("line {i}\n")).collect(),
                    }],
                    is_error: false,
                }])
            },
        ];
        store.save(&s, "What is ADDR_7F8000?", 1).unwrap();
        let found = find(&root, "c1").unwrap();
        assert_eq!(find(&root, "latest").unwrap().meta.id, "c1-aa");
        assert!(find(&root, "zz").is_err());
        assert!(line(&found).contains("What is ADDR_7F8000?"));
        let text = render(
            &found,
            &load(&root, &found).unwrap(),
            Show {
                result_lines: Some(5),
                system: false,
            },
        );
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            text.contains("## 1 · student\n\nWhat is ADDR_7F8000?"),
            "{text}"
        );
        assert!(text.contains("> thinking: Look at the C."), "{text}");
        assert!(
            text.contains(r#"→ decompile {"address":"$00:8000","level":"full"}"#),
            "{text}"
        );
        assert!(text.contains("## 3 · tool results  [not sent"), "{text}");
        assert!(text.contains("    line 4\n    … 45 more lines"), "{text}");
        assert!(text.contains("cache write 100, out 10; $0.0100"), "{text}");
    }
}
