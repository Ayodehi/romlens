//! `romlens diff`: two versions of a ROM compared (docs/22, D1).

use std::path::Path;

use anyhow::Result;
use romlens_core::FileOffset;
use romlens_core::diff::{Comparison, LineOp, Pairing, RunKind, Side, compare};

use crate::commands::rom::json_str;
use crate::commands::session::{self, Session};

pub struct DiffOptions<'a> {
    pub project_a: Option<&'a Path>,
    pub project_b: Option<&'a Path>,
    /// List every changed routine's instructions side by side.
    pub routines: bool,
    pub json: bool,
}

fn side(s: &Session) -> Side<'_> {
    Side {
        rom: &s.rom,
        project: &s.project,
        snap: &s.snap,
    }
}

pub fn run(a: &Path, b: &Path, options: DiffOptions<'_>) -> Result<()> {
    let sa = session::open(a, options.project_a, false)?;
    let sb = session::open(b, options.project_b, false)?;
    let c = compare(side(&sa), side(&sb));
    if options.json {
        print!("{}", json(&c));
    } else {
        print!("{}", text(&c, options.routines));
    }
    Ok(())
}

fn bytes(n: u64) -> String {
    format!("{n} byte{}", if n == 1 { "" } else { "s" })
}

/// A JSON array of pre-formatted rows, one to a line.
fn array(rows: &[String]) -> String {
    if rows.is_empty() {
        "[]".to_owned()
    } else {
        format!("[\n{}\n  ]", rows.join(",\n"))
    }
}

fn count(c: &Comparison, p: Pairing) -> usize {
    c.routines.iter().filter(|r| r.pairing == p).count()
}

fn text(c: &Comparison, routines: bool) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let b = &c.bytes;
    let _ = writeln!(
        out,
        "Bytes: {} the same, {} changed, {} inserted, {} deleted; {} moved blocks",
        b.count(RunKind::Same),
        b.count(RunKind::Changed),
        b.count(RunKind::Inserted),
        b.count(RunKind::Deleted),
        b.moves.len()
    );
    for r in b.runs.iter().filter(|r| r.kind != RunKind::Same) {
        let what = match r.kind {
            RunKind::Changed => "changed ",
            RunKind::Inserted => "inserted",
            RunKind::Deleted => "deleted ",
            RunKind::Same => unreachable!(),
        };
        let _ = writeln!(
            out,
            "  {what} {}..{} -> {}..{} ({})",
            FileOffset(r.a.start),
            FileOffset(r.a.end),
            FileOffset(r.b.start),
            FileOffset(r.b.end),
            bytes(r.a.len().max(r.b.len()) as u64)
        );
    }
    for m in &b.moves {
        let _ = writeln!(
            out,
            "  moved    {} -> {} ({})",
            FileOffset(m.a),
            FileOffset(m.b),
            bytes(m.len as u64)
        );
    }
    let _ = writeln!(
        out,
        "\nRoutines: {} the same, {} moved, {} changed, {} added, {} removed",
        count(c, Pairing::Same),
        count(c, Pairing::Moved),
        count(c, Pairing::Changed),
        count(c, Pairing::Added),
        count(c, Pairing::Removed)
    );
    for r in c.routines.iter().filter(|r| r.pairing != Pairing::Same) {
        let name = |i: &Option<romlens_core::diff::RoutineInfo>| {
            i.as_ref()
                .map_or_else(String::new, |i| format!("{} {}", i.entry, i.name))
        };
        let _ = match r.pairing {
            Pairing::Added => writeln!(out, "  added    {}", name(&r.b)),
            Pairing::Removed => writeln!(out, "  removed  {}", name(&r.a)),
            p => {
                let changed = r.lines.iter().filter(|l| l.op != LineOp::Same).count();
                let detail = if p == Pairing::Changed {
                    format!(" ({changed} of {} lines)", r.lines.len())
                } else {
                    String::new()
                };
                writeln!(
                    out,
                    "  {:<8} {} -> {}{detail}",
                    p.name(),
                    name(&r.a),
                    name(&r.b)
                )
            }
        };
        if routines && r.pairing == Pairing::Changed {
            for l in &r.lines {
                let mark = match l.op {
                    LineOp::Same => ' ',
                    LineOp::Changed => '~',
                    LineOp::Added => '+',
                    LineOp::Removed => '-',
                };
                let cell = |s: &Option<(u32, String)>| {
                    s.as_ref()
                        .map_or_else(String::new, |(o, t)| format!("{} {t}", FileOffset(*o)))
                };
                let _ = writeln!(out, "    {mark} {:<34} | {}", cell(&l.a), cell(&l.b));
            }
        }
    }
    let _ = writeln!(out, "\nData: {} regions with changed bytes", c.data.len());
    for d in &c.data {
        let _ = writeln!(
            out,
            "  {} {}..{} {:<10} {}{} changed",
            if d.in_b { "b" } else { "a" },
            FileOffset(d.start),
            FileOffset(d.start + d.len),
            d.kind.name(),
            d.name
                .as_ref()
                .map_or_else(String::new, |n| format!("{n}: ")),
            d.changed
        );
    }
    out
}

fn json(c: &Comparison) -> String {
    let routine = |i: &Option<romlens_core::diff::RoutineInfo>| {
        i.as_ref().map_or_else(
            || "null".to_owned(),
            |i| {
                format!(
                    "{{\"entry\": \"{}\", \"offset\": {}, \"name\": {}, \"instructions\": {}, \"bytes\": {}}}",
                    i.entry,
                    i.offset,
                    json_str(&i.name),
                    i.instructions,
                    i.bytes
                )
            },
        )
    };
    let cell = |s: &Option<(u32, String)>| {
        s.as_ref().map_or_else(
            || "null".to_owned(),
            |(o, t)| format!("{{\"offset\": {o}, \"text\": {}}}", json_str(t)),
        )
    };
    let runs: Vec<String> = c
        .bytes
        .runs
        .iter()
        .map(|r| {
            format!(
                "    {{\"kind\": \"{:?}\", \"a\": [{}, {}], \"b\": [{}, {}]}}",
                r.kind, r.a.start, r.a.end, r.b.start, r.b.end
            )
            .to_lowercase()
        })
        .collect();
    let moves: Vec<String> = c
        .bytes
        .moves
        .iter()
        .map(|m| format!("    {{\"a\": {}, \"b\": {}, \"len\": {}}}", m.a, m.b, m.len))
        .collect();
    let routines: Vec<String> = c
        .routines
        .iter()
        .filter(|r| r.pairing != Pairing::Same)
        .map(|r| {
            let lines: Vec<String> = r
                .lines
                .iter()
                .map(|l| {
                    format!(
                        "{{\"op\": \"{}\", \"a\": {}, \"b\": {}}}",
                        format!("{:?}", l.op).to_lowercase(),
                        cell(&l.a),
                        cell(&l.b)
                    )
                })
                .collect();
            format!(
                "    {{\"pairing\": \"{}\", \"a\": {}, \"b\": {}, \"lines\": [{}]}}",
                r.pairing.name(),
                routine(&r.a),
                routine(&r.b),
                lines.join(", ")
            )
        })
        .collect();
    let data: Vec<String> = c
        .data
        .iter()
        .map(|d| {
            format!(
                "    {{\"side\": \"{}\", \"start\": {}, \"len\": {}, \"kind\": \"{}\", \"name\": {}, \"changed\": {}}}",
                if d.in_b { "b" } else { "a" },
                d.start,
                d.len,
                d.kind.name(),
                d.name.as_deref().map_or_else(|| "null".to_owned(), json_str),
                d.changed
            )
        })
        .collect();
    format!(
        "{{\n  \"sameRoutines\": {},\n  \"runs\": {},\n  \"moves\": {},\n  \"routines\": {},\n  \"data\": {}\n}}\n",
        count(c, Pairing::Same),
        array(&runs),
        array(&moves),
        array(&routines),
        array(&data)
    )
}
