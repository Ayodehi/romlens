//! `graph`: a routine's control-flow graph, or its callers and callees
//! (docs/19), as text, Graphviz DOT or JSON with a layout.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};
use romlens_core::graph::layout::{EdgeSpec, NodeSize};
use romlens_core::graph::{
    BlockExit, CallHow, CallLink, CallNeighbourhood, EdgeKind, Layout, LayoutOptions, RoutineGraph,
    call_neighbourhood, layout, routine_graph,
};
use romlens_core::{AddressExpr, AddressStyle, SnesAddress, TextOptions, format_lines_text};

use crate::commands::rom::json_str;
use crate::commands::session::{self, Session, any_address};

pub struct GraphArgs<'a> {
    pub rom: &'a Path,
    pub expr: &'a str,
    pub project: Option<&'a Path>,
    pub calls: bool,
    pub dot: bool,
    pub json: bool,
}

pub fn run(args: GraphArgs) -> Result<()> {
    let s = session::open(args.rom, args.project, false)?;
    let addr = match any_address(args.expr)? {
        AddressExpr::Snes(a) => a,
        AddressExpr::File(off) => s
            .rom
            .snes_address_for(off)
            .with_context(|| format!("{off} has no SNES address"))?,
    };
    if args.calls {
        let n = call_neighbourhood(&s.rom, &s.project, &s.snap, addr)?;
        print!(
            "{}",
            if args.dot {
                calls_dot(&n)
            } else if args.json {
                calls_json(&n)
            } else {
                calls_text(&n)
            }
        );
    } else {
        let g = routine_graph(&s.rom, &s.project, &s.snap, addr)?;
        let lines = block_lines(&s, &g);
        print!(
            "{}",
            if args.dot {
                blocks_dot(&g, &lines)
            } else if args.json {
                blocks_json(&g, &lines)
            } else {
                blocks_text(&g)
            }
        );
    }
    Ok(())
}

/// Each block's lines as the listing prints them, SNES addresses only.
fn block_lines(s: &Session, g: &RoutineGraph) -> Vec<Vec<String>> {
    g.blocks
        .iter()
        .map(|b| match b.lines(&s.idx) {
            Some(r) => format_lines_text(
                &s.rom,
                &s.snap,
                &s.project,
                &s.idx,
                r.start as u32,
                r.len() as u32,
                TextOptions {
                    style: AddressStyle::Snes,
                    verbose: false,
                },
            )
            .lines()
            .map(|l| l.trim_end().to_owned())
            .collect(),
            None => vec![stub_text(&b.exit)],
        })
        .collect()
}

fn stub_text(exit: &BlockExit) -> String {
    match exit {
        BlockExit::Tail { name, target } => format!("→ {name} ({target})"),
        BlockExit::Unknown(why) => format!("→ ? {why}"),
        _ => String::new(),
    }
}

fn kind_name(k: EdgeKind) -> &'static str {
    match k {
        EdgeKind::Taken => "taken",
        EdgeKind::NotTaken => "not taken",
        EdgeKind::Jump => "jump",
        EdgeKind::Fall => "fall",
        EdgeKind::Case => "case",
    }
}

fn exit_name(e: &BlockExit) -> String {
    match e {
        BlockExit::Next => "runs on".into(),
        BlockExit::Branch => "branches".into(),
        BlockExit::Switch { table } => format!("switch on {table}"),
        BlockExit::Return => "returns".into(),
        BlockExit::Halt => "halts".into(),
        BlockExit::Tail { name, .. } => format!("continues in {name}"),
        BlockExit::Unknown(why) => format!("goes where the analysis cannot follow: {why}"),
    }
}

fn how_name(h: CallHow) -> &'static str {
    match h {
        CallHow::Call => "call",
        CallHow::Table => "table",
        CallHow::Tail => "tail",
        CallHow::Observed => "seen",
    }
}

fn edge_label(e: &romlens_core::graph::GraphEdge) -> String {
    let mut s = String::from(kind_name(e.kind));
    if e.kind == EdgeKind::Case {
        let c: Vec<String> = e.cases.iter().map(u32::to_string).collect();
        s = format!("case {}", c.join(", "));
    }
    if e.back {
        s.push_str(", back");
    }
    if let Some(c) = e.count {
        let _ = write!(s, ", {c}×");
    }
    s
}

fn blocks_text(g: &RoutineGraph) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} at {}: {}, {}, {}{}{}",
        g.name,
        g.entry,
        plural(g.blocks.len(), "block"),
        plural(g.edges.len(), "edge"),
        plural(g.loops.len(), "loop"),
        if g.irreducible { ", irreducible" } else { "" },
        if g.truncated { ", truncated" } else { "" },
    );
    for (i, b) in g.blocks.iter().enumerate() {
        let at = b
            .address
            .map(|a| a.to_string())
            .unwrap_or_else(|| "--:----".into());
        let mut line = format!("block {i:<3} {at}  ");
        if b.offsets.is_empty() {
            line.push_str(&stub_text(&b.exit));
        } else {
            let n = b.offsets.len();
            let _ = write!(
                line,
                "{n} instruction{}, {}",
                if n == 1 { "" } else { "s" },
                exit_name(&b.exit)
            );
        }
        if b.loop_header {
            line.push_str(", loop header");
        }
        if b.loop_depth > 0 {
            let _ = write!(line, ", loop depth {}", b.loop_depth);
        }
        if let Some(r) = b.runs {
            let _ = write!(line, ", ran {r}×");
        }
        let _ = writeln!(out, "{line}");
        for e in g.edges.iter().filter(|e| e.from == i) {
            let _ = writeln!(out, "    → block {} ({})", e.to, edge_label(e));
        }
    }
    out
}

fn plural(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

fn dot_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn blocks_dot(g: &RoutineGraph, lines: &[Vec<String>]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "digraph \"{}\" {{", dot_escape(&g.name));
    let _ = writeln!(out, "  node [shape=box, fontname=\"Menlo\", fontsize=10];");
    for (i, b) in g.blocks.iter().enumerate() {
        let mut label: String = lines[i]
            .iter()
            .map(|l| format!("{}\\l", dot_escape(l)))
            .collect();
        if let Some(r) = b.runs {
            let _ = write!(label, "ran {r}×\\l");
        }
        let mut attrs = format!("label=\"{label}\"");
        if b.offsets.is_empty() {
            attrs.push_str(", style=dashed");
        } else if b.loop_depth > 0 {
            attrs.push_str(", style=filled, fillcolor=\"#e8f0ff\"");
        }
        let _ = writeln!(out, "  b{i} [{attrs}];");
    }
    for e in &g.edges {
        let color = match e.kind {
            _ if e.back => "blue",
            EdgeKind::Taken => "darkgreen",
            EdgeKind::NotTaken => "red",
            EdgeKind::Jump | EdgeKind::Fall | EdgeKind::Case => "gray40",
        };
        let mut attrs = format!("color={color}");
        if e.kind == EdgeKind::Case || e.count.is_some() {
            let _ = write!(attrs, ", label=\"{}\"", dot_escape(&edge_label(e)));
        }
        let _ = writeln!(out, "  b{} -> b{} [{attrs}];", e.from, e.to);
    }
    out.push_str("}\n");
    out
}

/// The layout in character cells: each line one unit high, each
/// character one wide.
fn cell_layout(sizes: &[NodeSize], edges: &[EdgeSpec]) -> Layout {
    layout(
        sizes,
        edges,
        &LayoutOptions {
            node_gap: 4.0,
            layer_gap: 3.0,
            edge_gap: 2.0,
            ..LayoutOptions::default()
        },
    )
}

fn json_points(l: &Layout, i: usize) -> String {
    let p: Vec<String> = l.edges[i]
        .points
        .iter()
        .map(|p| format!("[{}, {}]", num(p.x), num(p.y)))
        .collect();
    format!("[{}]", p.join(", "))
}

/// Short and stable: at most two decimals.
fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn opt(v: Option<u64>) -> String {
    v.map(|c| c.to_string()).unwrap_or_else(|| "null".into())
}

fn blocks_json(g: &RoutineGraph, lines: &[Vec<String>]) -> String {
    let sizes: Vec<NodeSize> = lines
        .iter()
        .map(|l| NodeSize {
            width: l.iter().map(|s| s.chars().count()).max().unwrap_or(1) as f64 + 2.0,
            height: l.len().max(1) as f64,
        })
        .collect();
    let specs: Vec<EdgeSpec> = g
        .edges
        .iter()
        .map(|e| EdgeSpec {
            from: e.from,
            to: e.to,
            back: e.back,
        })
        .collect();
    let l = cell_layout(&sizes, &specs);
    let mut out = String::new();
    let _ = writeln!(out, "{{");
    let _ = writeln!(out, "  \"name\": {},", json_str(&g.name));
    let _ = writeln!(out, "  \"entry\": \"{}\",", g.entry);
    let _ = writeln!(out, "  \"counted\": {},", g.counted);
    let _ = writeln!(out, "  \"blocks\": [");
    for (i, b) in g.blocks.iter().enumerate() {
        let text: Vec<String> = lines[i].iter().map(|s| json_str(s)).collect();
        let _ = writeln!(
            out,
            "    {{\"address\": {}, \"exit\": {}, \"loopDepth\": {}, \"loopHeader\": {}, \"runs\": {}, \"x\": {}, \"y\": {}, \"width\": {}, \"height\": {}, \"lines\": [{}]}}{}",
            b.address
                .map(|a| format!("\"{a}\""))
                .unwrap_or_else(|| "null".into()),
            json_str(&exit_name(&b.exit)),
            b.loop_depth,
            b.loop_header,
            opt(b.runs),
            num(l.nodes[i].x),
            num(l.nodes[i].y),
            num(sizes[i].width),
            num(sizes[i].height),
            text.join(", "),
            if i + 1 < g.blocks.len() { "," } else { "" }
        );
    }
    let _ = writeln!(out, "  ],");
    let _ = writeln!(out, "  \"edges\": [");
    for (i, e) in g.edges.iter().enumerate() {
        let cases: Vec<String> = e.cases.iter().map(u32::to_string).collect();
        let _ = writeln!(
            out,
            "    {{\"from\": {}, \"to\": {}, \"kind\": {}, \"cases\": [{}], \"back\": {}, \"count\": {}, \"points\": {}}}{}",
            e.from,
            e.to,
            json_str(kind_name(e.kind)),
            cases.join(", "),
            e.back,
            opt(e.count),
            json_points(&l, i),
            if i + 1 < g.edges.len() { "," } else { "" }
        );
    }
    let _ = writeln!(out, "  ],");
    let _ = writeln!(out, "  \"width\": {},", num(l.width));
    let _ = writeln!(out, "  \"height\": {}", num(l.height));
    let _ = writeln!(out, "}}");
    out
}

fn link_text(c: &CallLink) -> String {
    let sites: Vec<String> = c
        .sites
        .iter()
        .map(|s| {
            let mut t = format!("{} {}", s.address, how_name(s.how));
            if let Some(n) = s.count {
                let _ = write!(t, " {n}×");
            }
            t
        })
        .collect();
    format!(
        "  {} {}  {} site{}: {}",
        c.name,
        c.entry,
        c.sites.len(),
        if c.sites.len() == 1 { "" } else { "s" },
        sites.join(", ")
    )
}

fn calls_text(n: &CallNeighbourhood) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} at {}", n.name, n.entry);
    let _ = writeln!(out, "callers:");
    if n.callers.is_empty() {
        let _ = writeln!(out, "  (none the analysis found)");
    }
    for c in &n.callers {
        let _ = writeln!(out, "{}", link_text(c));
    }
    let _ = writeln!(out, "callees:");
    if n.callees.is_empty() {
        let _ = writeln!(out, "  (none)");
    }
    for c in &n.callees {
        let _ = writeln!(out, "{}", link_text(c));
    }
    out
}

fn calls_dot(n: &CallNeighbourhood) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "digraph \"{} calls\" {{", dot_escape(&n.name));
    let _ = writeln!(out, "  node [shape=box, fontname=\"Menlo\", fontsize=10];");
    let node = |id: &str, name: &str, at: SnesAddress, extra: &str| {
        format!("  {id} [label=\"{}\\n{at}\"{extra}];\n", dot_escape(name))
    };
    out.push_str(&node("c", &n.name, n.entry, ", style=bold"));
    for (i, c) in n.callers.iter().enumerate() {
        out.push_str(&node(&format!("in{i}"), &c.name, c.entry, ""));
        let _ = writeln!(out, "  in{i} -> c [label=\"{}\"];", sites_label(c));
    }
    for (i, c) in n.callees.iter().enumerate() {
        out.push_str(&node(&format!("out{i}"), &c.name, c.entry, ""));
        let _ = writeln!(out, "  c -> out{i} [label=\"{}\"];", sites_label(c));
    }
    out.push_str("}\n");
    out
}

fn sites_label(c: &CallLink) -> String {
    let mut hows: Vec<&str> = c.sites.iter().map(|s| how_name(s.how)).collect();
    hows.dedup();
    let mut s = format!("{}× {}", c.sites.len(), hows.join("/"));
    if let Some(n) = c.count() {
        let _ = write!(s, ", ran {n}×");
    }
    s
}

fn calls_json(n: &CallNeighbourhood) -> String {
    let link = |c: &CallLink| {
        let sites: Vec<String> = c
            .sites
            .iter()
            .map(|s| {
                format!(
                    "{{\"address\": \"{}\", \"how\": {}, \"count\": {}}}",
                    s.address,
                    json_str(how_name(s.how)),
                    opt(s.count)
                )
            })
            .collect();
        format!(
            "{{\"name\": {}, \"entry\": \"{}\", \"sites\": [{}]}}",
            json_str(&c.name),
            c.entry,
            sites.join(", ")
        )
    };
    let list = |v: &[CallLink]| {
        if v.is_empty() {
            "[]".to_owned()
        } else {
            let items: Vec<String> = v.iter().map(link).collect();
            format!("[\n    {}\n  ]", items.join(",\n    "))
        }
    };
    let mut out = String::new();
    let _ = writeln!(out, "{{");
    let _ = writeln!(out, "  \"name\": {},", json_str(&n.name));
    let _ = writeln!(out, "  \"entry\": \"{}\",", n.entry);
    let _ = writeln!(out, "  \"counted\": {},", n.counted);
    let _ = writeln!(out, "  \"callers\": {},", list(&n.callers));
    let _ = writeln!(out, "  \"callees\": {}", list(&n.callees));
    let _ = writeln!(out, "}}");
    out
}
