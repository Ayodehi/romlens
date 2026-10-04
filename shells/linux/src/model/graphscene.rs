//! What the Graph canvas draws: boxes of lines and the edges between them, in
//! the canvas's coordinates (top-left origin), built from the core's graph,
//! the listing's own lines and the core's layout. No GTK: widths come from
//! the monospaced character width, so the scene is deterministic and tested.
//! The macOS twin is `GraphScene` and `GraphSceneBuilder`.

use romlens_ffi::{
    CallHowKind, CallLinkInfo, CallNeighbourhoodInfo, GraphEdgeKind, GraphEdgeSpec, GraphExitKind,
    GraphLayoutOptions, GraphNodeSize, GraphPoint, RoutineGraphInfo, layout_graph,
};

use crate::asm::{AsmLine, LineKind, TokenKind};

pub const PADDING: (f64, f64) = (8.0, 4.0);
pub const MARGIN: f64 = 12.0;
/// Room above the first box for a run-count badge.
pub const TOP: f64 = 14.0;

/// A run of text in one colour class.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub text: String,
    pub kind: Option<TokenKind>,
    /// Dim, bold or the listing's address column colour.
    pub style: SegmentStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentStyle {
    Plain,
    Dim,
    Faint,
    Bold,
    Comment,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub segments: Vec<Segment>,
    /// The instruction (or data) a click selects.
    pub offset: Option<u32>,
}

impl Line {
    fn chars(&self) -> usize {
        self.segments.iter().map(|s| s.text.chars().count()).sum()
    }

    pub fn text(&self) -> String {
        self.segments.iter().map(|s| s.text.as_str()).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }

    pub fn mid_x(&self) -> f64 {
        self.x + self.w / 2.0
    }

    pub fn mid_y(&self) -> f64 {
        self.y + self.h / 2.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Box_ {
    pub rect: Rect,
    pub lines: Vec<Line>,
    /// Loops it is in, for the tint.
    pub loop_depth: u32,
    pub loop_header: bool,
    /// A tail call or an unknown destination: no instructions.
    pub stub: bool,
    /// The routine in the middle of the Calls view, or the entry block.
    pub emphasis: bool,
    /// The recording never reached it.
    pub dim: bool,
    /// Above the box's top-right corner, as `768×`.
    pub badge: Option<String>,
    pub tooltip: Option<String>,
    /// Where a double-click goes (a SNES address).
    pub target: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeColor {
    Back,
    Taken,
    NotTaken,
    Case,
    Plain,
    Table,
    Tail,
    Observed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub points: Vec<(f64, f64)>,
    pub color: EdgeColor,
    pub width: f64,
    pub dashed: bool,
    pub label: Option<String>,
    pub dim: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub boxes: Vec<Box_>,
    pub edges: Vec<Edge>,
    pub size: (f64, f64),
    pub row_height: f64,
}

impl Scene {
    pub fn empty() -> Self {
        Self {
            boxes: Vec::new(),
            edges: Vec::new(),
            size: (0.0, 0.0),
            row_height: 16.0,
        }
    }

    /// The box and line holding the instruction at `offset`.
    pub fn line_for_offset(&self, offset: u32) -> Option<(usize, usize)> {
        self.boxes.iter().enumerate().find_map(|(b, bx)| {
            bx.lines
                .iter()
                .position(|l| l.offset == Some(offset))
                .map(|l| (b, l))
        })
    }

    pub fn rect_for_offset(&self, offset: u32) -> Option<Rect> {
        let (b, l) = self.line_for_offset(offset)?;
        Some(self.line_rect(b, l))
    }

    pub fn line_rect(&self, b: usize, l: usize) -> Rect {
        let r = self.boxes[b].rect;
        Rect {
            x: r.x,
            y: r.y + PADDING.1 + l as f64 * self.row_height,
            w: r.w,
            h: self.row_height,
        }
    }

    /// The box and line under a point.
    pub fn hit(&self, p: (f64, f64)) -> Option<(usize, Option<usize>)> {
        let b = self.boxes.iter().rposition(|bx| bx.rect.contains(p))?;
        let r = self.boxes[b].rect;
        let l = ((p.1 - r.y - PADDING.1) / self.row_height).floor();
        let line = (l >= 0.0 && (l as usize) < self.boxes[b].lines.len()).then_some(l as usize);
        Some((b, line))
    }
}

fn plural(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{out}×")
}

pub fn summary_blocks(g: &RoutineGraphInfo) -> String {
    let mut parts = vec![
        plural(g.blocks.len(), "block"),
        plural(g.edges.len(), "edge"),
        plural(g.loops.len(), "loop"),
    ];
    if g.counted {
        parts.push("counts from the execution log".into());
    }
    if g.irreducible {
        parts.push("a loop with more than one way in".into());
    }
    if g.truncated {
        parts.push("cut short: too long to follow".into());
    }
    parts.join(", ")
}

pub fn summary_calls(n: &CallNeighbourhoodInfo) -> String {
    format!(
        "{}, {}",
        plural(n.callers.len(), "caller"),
        plural(n.callees.len(), "callee")
    ) + if n.counted {
        ", counts from the execution log"
    } else {
        ""
    }
}

fn seg(text: impl Into<String>, kind: Option<TokenKind>, style: SegmentStyle) -> Segment {
    Segment {
        text: text.into(),
        kind,
        style,
    }
}

/// A listing line as the graph shows it: the SNES address and the text,
/// coloured as the listing colours it.
pub fn line_from_record(r: &AsmLine, offset: Option<u32>) -> Line {
    let mut segments = Vec::new();
    if r.kind.is_content() {
        let at = r.snes_address.map_or_else(
            || romlens_ffi::format_file_offset(r.file_offset),
            romlens_ffi::format_snes_address,
        );
        segments.push(seg(format!("{at}  "), None, SegmentStyle::Faint));
    }
    let dim = if r.kind == LineKind::Data {
        SegmentStyle::Dim
    } else {
        SegmentStyle::Plain
    };
    let bytes = r.text.as_bytes();
    let mut tokens = r.tokens.clone();
    tokens.sort_by_key(|t| t.start);
    let mut at = 0;
    for t in &tokens {
        // A token that overlaps the last, or runs past the text, is skipped.
        if t.start < at || t.start + t.len > bytes.len() {
            continue;
        }
        if t.start > at {
            segments.push(seg(String::from_utf8_lossy(&bytes[at..t.start]), None, dim));
        }
        segments.push(seg(
            String::from_utf8_lossy(&bytes[t.start..t.start + t.len]),
            Some(t.kind),
            dim,
        ));
        at = t.start + t.len;
    }
    if at < bytes.len() {
        let style = if r.kind == LineKind::Comment {
            SegmentStyle::Comment
        } else {
            dim
        };
        segments.push(seg(String::from_utf8_lossy(&bytes[at..]), None, style));
    }
    Line { segments, offset }
}

/// Character cells: what a monospaced font measures in.
#[derive(Debug, Clone, Copy)]
pub struct Cells {
    pub char_width: f64,
    pub row_height: f64,
}

fn size_of(lines: &[Line], cells: Cells) -> GraphNodeSize {
    let chars = lines.iter().map(Line::chars).max().unwrap_or(0);
    GraphNodeSize {
        width: chars as f64 * cells.char_width + PADDING.0 * 2.0,
        height: lines.len().max(1) as f64 * cells.row_height + PADDING.1 * 2.0,
    }
}

fn pt(p: &GraphPoint, dx: f64, dy: f64) -> (f64, f64) {
    (p.x + dx, p.y + dy)
}

/// Blocks mode: the routine's control flow, each block holding the listing's
/// own lines. `record` fetches the listing line `n`.
pub fn blocks(
    g: &RoutineGraphInfo,
    cells: Cells,
    record: impl Fn(u32) -> Option<AsmLine>,
) -> Scene {
    let mut contents: Vec<Vec<Line>> = Vec::new();
    for b in &g.blocks {
        let mut lines = Vec::new();
        if let Some(first) = b.first_line {
            for n in first..first + b.line_count {
                let Some(r) = record(n) else { continue };
                if matches!(r.kind, LineKind::Blank | LineKind::Section) {
                    continue;
                }
                let offset = if r.kind.is_content() {
                    Some(r.file_offset)
                } else {
                    b.offsets.first().copied()
                };
                lines.push(line_from_record(&r, offset));
            }
        } else {
            let name = if b.exit_text.is_empty() {
                "?".to_owned()
            } else if b.exit == GraphExitKind::Tail {
                b.exit_text.clone()
            } else {
                "?".to_owned()
            };
            lines.push(Line {
                segments: vec![seg(format!("→ {name}"), None, SegmentStyle::Dim)],
                offset: None,
            });
        }
        contents.push(lines);
    }
    let sizes: Vec<GraphNodeSize> = contents.iter().map(|l| size_of(l, cells)).collect();
    let specs: Vec<GraphEdgeSpec> = g
        .edges
        .iter()
        .map(|e| GraphEdgeSpec {
            from: e.from,
            to: e.to,
            back: e.back,
        })
        .collect();
    let layout = layout_graph(
        sizes.clone(),
        specs,
        GraphLayoutOptions {
            node_gap: 28.0,
            layer_gap: 44.0,
            edge_gap: 14.0,
        },
    );

    let mut boxes = Vec::new();
    for (i, b) in g.blocks.iter().enumerate() {
        let rect = Rect {
            x: layout.nodes[i].x + MARGIN,
            y: layout.nodes[i].y + MARGIN + TOP,
            w: sizes[i].width,
            h: sizes[i].height,
        };
        let stub = b.offsets.is_empty();
        let tooltip = match b.exit {
            GraphExitKind::Tail => Some(format!(
                "Continues in {}; double-click to go there",
                b.exit_text
            )),
            GraphExitKind::Unknown => Some(format!(
                "Goes where the analysis cannot follow: {}",
                b.exit_text
            )),
            _ => b.runs.map(|r| {
                format!(
                    "Its first instruction ran {} times in the execution log",
                    count(r).trim_end_matches('×')
                )
            }),
        };
        boxes.push(Box_ {
            rect,
            lines: std::mem::take(&mut contents[i]),
            loop_depth: b.loop_depth,
            loop_header: b.loop_header,
            stub,
            emphasis: i == 0,
            dim: g.counted && !stub && b.runs == Some(0),
            badge: if stub { None } else { b.runs.map(count) },
            tooltip,
            target: (b.exit == GraphExitKind::Tail)
                .then_some(b.exit_target)
                .flatten(),
        });
    }
    let mut edges = Vec::new();
    for (i, e) in g.edges.iter().enumerate() {
        let color = if e.back {
            EdgeColor::Back
        } else {
            match e.kind {
                GraphEdgeKind::Taken => EdgeColor::Taken,
                GraphEdgeKind::NotTaken => EdgeColor::NotTaken,
                GraphEdgeKind::Case => EdgeColor::Case,
                GraphEdgeKind::Jump | GraphEdgeKind::Fall => EdgeColor::Plain,
            }
        };
        let mut label = Vec::new();
        if e.kind == GraphEdgeKind::Case {
            label.push(
                e.cases
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        if let Some(c) = e.count {
            label.push(count(c));
        }
        edges.push(Edge {
            points: layout.edges[i]
                .points
                .iter()
                .map(|p| pt(p, MARGIN, MARGIN + TOP))
                .collect(),
            color,
            width: if e.back { 1.6 } else { 1.2 },
            dashed: false,
            label: (!label.is_empty()).then(|| label.join(" ")),
            dim: g.counted && e.count == Some(0),
        });
    }
    Scene {
        boxes,
        edges,
        size: (
            layout.width + MARGIN * 2.0,
            layout.height + MARGIN * 2.0 + TOP,
        ),
        row_height: cells.row_height,
    }
}

fn sites_text(c: &CallLinkInfo, counted: bool) -> String {
    let mut hows: Vec<&str> = Vec::new();
    for s in &c.sites {
        let h = match s.how {
            CallHowKind::Call => "call",
            CallHowKind::Table => "table",
            CallHowKind::Tail => "tail call",
            CallHowKind::Observed => "seen",
        };
        if !hows.contains(&h) {
            hows.push(h);
        }
    }
    let mut t = format!("{}: {}", plural(c.sites.len(), "site"), hows.join(", "));
    if counted {
        let total: u64 = c.sites.iter().filter_map(|s| s.count).sum();
        t.push_str(&format!(", ran {}×", count(total).trim_end_matches('×')));
    }
    t
}

/// Calls mode: the routine in the middle, its callers on the left and its
/// callees on the right.
pub fn calls(n: &CallNeighbourhoodInfo, cells: Cells) -> Scene {
    struct Node {
        lines: Vec<Line>,
        entry: u32,
        centre: bool,
    }
    let line = |t: String, style: SegmentStyle| Line {
        segments: vec![seg(t, None, style)],
        offset: None,
    };
    let mut nodes = vec![Node {
        lines: vec![
            line(n.name.clone(), SegmentStyle::Bold),
            line(romlens_ffi::format_snes_address(n.entry), SegmentStyle::Dim),
            line(
                format!(
                    "{}, {}",
                    plural(n.callers.len(), "caller"),
                    plural(n.callees.len(), "callee")
                ),
                SegmentStyle::Dim,
            ),
        ],
        entry: n.entry,
        centre: true,
    }];
    let mut specs: Vec<GraphEdgeSpec> = Vec::new();
    let mut hows: Vec<CallHowKind> = Vec::new();
    let mut add = |c: &CallLinkInfo, caller: bool, nodes: &mut Vec<Node>| {
        let how = c.sites.first().map_or(CallHowKind::Call, |s| s.how);
        if c.entry == n.entry {
            // Calls itself: once, as a loop on the middle box.
            if !caller {
                specs.push(GraphEdgeSpec {
                    from: 0,
                    to: 0,
                    back: true,
                });
                hows.push(how);
            }
            return;
        }
        nodes.push(Node {
            lines: vec![
                line(c.name.clone(), SegmentStyle::Bold),
                line(
                    format!(
                        "{}  {}",
                        romlens_ffi::format_snes_address(c.entry),
                        sites_text(c, n.counted)
                    ),
                    SegmentStyle::Dim,
                ),
            ],
            entry: c.entry,
            centre: false,
        });
        let i = (nodes.len() - 1) as u32;
        specs.push(if caller {
            GraphEdgeSpec {
                from: i,
                to: 0,
                back: false,
            }
        } else {
            GraphEdgeSpec {
                from: 0,
                to: i,
                back: false,
            }
        });
        hows.push(how);
    };
    for c in &n.callers {
        add(c, true, &mut nodes);
    }
    for c in &n.callees {
        add(c, false, &mut nodes);
    }
    let sizes: Vec<GraphNodeSize> = nodes.iter().map(|nd| size_of(&nd.lines, cells)).collect();
    // Sideways: callers in a column on the left, callees on the right, so many
    // of either stack down the page instead of across it. The core lays it out
    // top to bottom with the axes swapped.
    let swapped: Vec<GraphNodeSize> = sizes
        .iter()
        .map(|s| GraphNodeSize {
            width: s.height,
            height: s.width,
        })
        .collect();
    let layout = layout_graph(
        swapped,
        specs.clone(),
        GraphLayoutOptions {
            node_gap: 10.0,
            layer_gap: 96.0,
            edge_gap: 10.0,
        },
    );
    let boxes = nodes
        .into_iter()
        .enumerate()
        .map(|(i, nd)| Box_ {
            rect: Rect {
                x: layout.nodes[i].y + MARGIN,
                y: layout.nodes[i].x + MARGIN,
                w: sizes[i].width,
                h: sizes[i].height,
            },
            lines: nd.lines,
            loop_depth: 0,
            loop_header: false,
            stub: false,
            emphasis: nd.centre,
            dim: false,
            badge: None,
            tooltip: (!nd.centre)
                .then(|| "Double-click to put this routine in the middle".to_owned()),
            target: (!nd.centre).then_some(nd.entry),
        })
        .collect();
    let edges = (0..specs.len())
        .map(|i| Edge {
            points: layout.edges[i]
                .points
                .iter()
                .map(|p| (p.y + MARGIN, p.x + MARGIN))
                .collect(),
            color: match hows[i] {
                CallHowKind::Call => EdgeColor::Plain,
                CallHowKind::Table => EdgeColor::Table,
                CallHowKind::Tail => EdgeColor::Tail,
                CallHowKind::Observed => EdgeColor::Observed,
            },
            width: 1.2,
            dashed: hows[i] == CallHowKind::Observed,
            label: None,
            dim: false,
        })
        .collect();
    Scene {
        boxes,
        edges,
        size: (layout.height + MARGIN * 2.0, layout.width + MARGIN * 2.0),
        row_height: cells.row_height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::{AsmBatch, LINES_PER_BATCH};
    use romlens_ffi::{Rom, Workbench, make_routines_test_rom};

    const CELLS: Cells = Cells {
        char_width: 8.0,
        row_height: 18.0,
    };

    fn routines() -> std::sync::Arc<Workbench> {
        let rom = Rom::from_bytes(make_routines_test_rom(), "r.sfc".into()).unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        wb
    }

    fn record(wb: &Workbench) -> impl Fn(u32) -> Option<AsmLine> + '_ {
        move |n| {
            let start = n - n % LINES_PER_BATCH;
            let batch = AsmBatch::decode(start, &wb.asm_lines(start, LINES_PER_BATCH)).ok()?;
            batch.line(n).cloned()
        }
    }

    #[test]
    fn blocks_become_boxes_with_the_listings_own_lines() {
        let wb = routines();
        let g = wb.routine_graph_blocking(0x00_8020).unwrap();
        let scene = blocks(&g, CELLS, record(&wb));
        assert_eq!(scene.boxes.len(), g.blocks.len());
        assert_eq!(scene.edges.len(), g.edges.len());
        assert!(scene.boxes[0].emphasis);
        // The loop's store is in some box, on a line a click can select.
        let (b, l) = scene.line_for_offset(0x22).expect("the store is drawn");
        // (The block's label line carries its first offset too, and comes first.)
        assert!(
            scene.boxes[b]
                .lines
                .iter()
                .any(|ln| ln.offset == Some(0x22) && ln.text().contains("STZ")),
            "{:?}",
            scene.boxes[b]
                .lines
                .iter()
                .map(Line::text)
                .collect::<Vec<_>>()
        );
        // Boxes sit inside the scene, and a point inside one hits it.
        for bx in &scene.boxes {
            assert!(bx.rect.x >= MARGIN && bx.rect.x + bx.rect.w <= scene.size.0 + 0.001);
            assert!(bx.rect.y + bx.rect.h <= scene.size.1 + 0.001);
        }
        let r = scene.line_rect(b, l);
        assert_eq!(scene.hit((r.mid_x(), r.mid_y())), Some((b, Some(l))));
        assert_eq!(scene.hit((-5.0, -5.0)), None);
        assert!(summary_blocks(&g).contains("block"));
    }

    #[test]
    fn a_back_edge_is_blue_and_a_stub_has_no_instructions() {
        let wb = routines();
        let g = wb.routine_graph_blocking(0x00_8020).unwrap();
        let scene = blocks(&g, CELLS, record(&wb));
        assert_eq!(
            scene
                .edges
                .iter()
                .filter(|e| e.color == EdgeColor::Back)
                .count(),
            g.edges.iter().filter(|e| e.back).count()
        );
        for (bx, b) in scene.boxes.iter().zip(&g.blocks) {
            assert_eq!(bx.stub, b.offsets.is_empty());
        }
    }

    #[test]
    fn calls_put_the_routine_in_the_middle() {
        let wb = routines();
        let n = wb.call_neighbourhood_blocking(0x00_8030).unwrap();
        let scene = calls(&n, CELLS);
        assert!(scene.boxes[0].emphasis);
        assert!(scene.boxes[0].lines[0].text().contains("SUB_008030"));
        assert!(
            scene
                .boxes
                .iter()
                .skip(1)
                .all(|b| b.target.is_some() && b.tooltip.is_some())
        );
        assert_eq!(
            scene.edges.len(),
            n.callers.len() + n.callees.len() - self_calls(&n)
        );
        assert!(summary_calls(&n).contains("caller"));
    }

    fn self_calls(n: &CallNeighbourhoodInfo) -> usize {
        n.callers.iter().filter(|c| c.entry == n.entry).count()
    }

    #[test]
    fn counts_have_grouping_and_plurals_agree() {
        assert_eq!(count(768), "768×");
        assert_eq!(count(1_234_567), "1,234,567×");
        assert_eq!(count(0), "0×");
        assert_eq!(plural(1, "block"), "1 block");
        assert_eq!(plural(2, "caller"), "2 callers");
    }

    #[test]
    fn a_listing_record_keeps_its_tokens_and_dims_data() {
        let wb = routines();
        let rec = record(&wb);
        let line = (0..60)
            .filter_map(&rec)
            .find(|r| r.kind == LineKind::Instruction)
            .unwrap();
        let l = line_from_record(&line, Some(line.file_offset));
        assert_eq!(l.segments[0].style, SegmentStyle::Faint);
        assert!(
            l.segments
                .iter()
                .any(|s| s.kind == Some(TokenKind::Mnemonic))
        );
        assert!(l.text().starts_with("$00:"));
    }
}
