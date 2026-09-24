//! The graphs the Graph tab draws (docs/19): a routine's blocks and edges,
//! its callers and callees, and their layout.

mod common;

use std::sync::Arc;
use std::time::Instant;

use romlens_core::analysis::{AnalysisControl, AnalysisSnapshot, analyze};
use romlens_core::decompile;
use romlens_core::fixtures;
use romlens_core::graph::layout::{EdgeSpec, NodeSize};
use romlens_core::graph::{
    BlockExit, CallHow, EdgeKind, LayoutOptions, call_neighbourhood, layout, routine_graph,
};
use romlens_core::model::Project;
use romlens_core::model::exec_log::{ExecInsn, ExecLog, Flow, FlowKind, MemKind};
use romlens_core::{RomImage, SnesAddress};

fn setup(bytes: Vec<u8>) -> (RomImage, Project, AnalysisSnapshot) {
    let rom = RomImage::from_bytes(bytes, "t.sfc").unwrap();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    (rom, project, snap)
}

fn at(a: u32) -> SnesAddress {
    SnesAddress::from_u24(a)
}

fn edge_kinds(g: &romlens_core::graph::RoutineGraph) -> Vec<(usize, usize, EdgeKind, bool)> {
    g.edges
        .iter()
        .map(|e| (e.from, e.to, e.kind, e.back))
        .collect()
}

#[test]
fn a_counted_loop_is_three_blocks_with_a_way_back() {
    let (rom, project, snap) = setup(fixtures::routines_lorom());
    let g = routine_graph(&rom, &project, &snap, at(0x008020)).unwrap();
    assert_eq!(g.name, "SUB_008020");
    let starts: Vec<u32> = g
        .blocks
        .iter()
        .map(|b| b.address.unwrap().as_u24())
        .collect();
    assert_eq!(starts, vec![0x008020, 0x008022, 0x008028]);
    assert_eq!(g.blocks[1].offsets.len(), 3, "STZ, DEX, BPL");
    assert_eq!(
        edge_kinds(&g),
        vec![
            (0, 1, EdgeKind::Fall, false),
            (1, 1, EdgeKind::Taken, true),
            (1, 2, EdgeKind::NotTaken, false),
        ]
    );
    assert_eq!(g.loops.len(), 1);
    assert_eq!((g.loops[0].header, g.loops[0].body.clone()), (1, vec![1]));
    assert!(g.blocks[1].loop_header && g.blocks[1].loop_depth == 1);
    assert_eq!(g.blocks[2].exit, BlockExit::Return);
    assert!(!g.counted && g.blocks.iter().all(|b| b.runs.is_none()));
}

#[test]
fn an_if_joins_where_both_ways_meet() {
    let (rom, project, snap) = setup(fixtures::routines_lorom());
    let g = routine_graph(&rom, &project, &snap, at(0x008040)).unwrap();
    let starts: Vec<u32> = g
        .blocks
        .iter()
        .map(|b| b.address.unwrap().as_u24())
        .collect();
    assert_eq!(starts, vec![0x008040, 0x008046, 0x008048]);
    assert_eq!(
        edge_kinds(&g),
        vec![
            (0, 2, EdgeKind::Taken, false),
            (0, 1, EdgeKind::NotTaken, false),
            (1, 2, EdgeKind::Fall, false),
        ]
    );
    assert!(g.loops.is_empty());
}

#[test]
fn a_spin_is_a_block_that_jumps_to_itself() {
    let (rom, project, snap) = setup(fixtures::routines_lorom());
    let g = routine_graph(&rom, &project, &snap, at(0x008000)).unwrap();
    let spin = g
        .blocks
        .iter()
        .position(|b| b.address == Some(at(0x008016)))
        .unwrap();
    assert!(
        g.edges
            .iter()
            .any(|e| e.from == spin && e.to == spin && e.kind == EdgeKind::Jump && e.back)
    );
}

#[test]
fn tables_and_tail_calls_leave_through_stubs() {
    let (rom, project, snap) = setup(fixtures::dispatch_lorom());
    let g = routine_graph(&rom, &project, &snap, at(0x008000)).unwrap();
    let mut tails: Vec<u32> = g
        .blocks
        .iter()
        .filter_map(|b| match &b.exit {
            BlockExit::Tail { target, .. } => Some(target.as_u24()),
            _ => None,
        })
        .collect();
    tails.sort_unstable();
    assert_eq!(
        tails,
        vec![0x008030, 0x008038],
        "JMP ($8060,X) into two routines"
    );
    let cases: Vec<Vec<u32>> = g
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Case)
        .map(|e| e.cases.clone())
        .collect();
    let mut cases = cases;
    cases.sort();
    assert_eq!(cases, vec![vec![0], vec![1]]);
    assert!(g.blocks.iter().all(|b| b.offsets.is_empty()
        == matches!(b.exit, BlockExit::Tail { .. } | BlockExit::Unknown(_))));
}

#[test]
fn callers_and_callees_agree() {
    let (rom, project, snap) = setup(fixtures::routines_lorom());
    let reset = call_neighbourhood(&rom, &project, &snap, at(0x008000)).unwrap();
    let callees: Vec<(u32, Vec<CallHow>)> = reset
        .callees
        .iter()
        .map(|c| (c.entry.as_u24(), c.sites.iter().map(|s| s.how).collect()))
        .collect();
    assert_eq!(
        callees,
        vec![
            (0x008020, vec![CallHow::Call]),
            (0x008030, vec![CallHow::Call]),
            (0x008040, vec![CallHow::Call]),
            (0x008050, vec![CallHow::Call]),
        ]
    );
    let loop_ = call_neighbourhood(&rom, &project, &snap, at(0x008020)).unwrap();
    assert_eq!(loop_.callers.len(), 1);
    assert_eq!(loop_.callers[0].entry, at(0x008000));
    assert_eq!(loop_.callers[0].sites[0].address, at(0x008005));
    assert!(loop_.callees.is_empty());
}

#[test]
fn a_table_call_and_a_tail_call_both_count_as_callers() {
    let (rom, project, snap) = setup(fixtures::dispatch_lorom());
    let n = call_neighbourhood(&rom, &project, &snap, at(0x008030)).unwrap();
    assert_eq!(n.callers.len(), 1);
    let how: Vec<CallHow> = n.callers[0].sites.iter().map(|s| s.how).collect();
    assert_eq!(how, vec![CallHow::Table, CallHow::Tail]);
    let reset = call_neighbourhood(&rom, &project, &snap, at(0x008000)).unwrap();
    let table: Vec<u32> = reset
        .callees
        .iter()
        .filter(|c| c.sites.iter().any(|s| s.how == CallHow::Table))
        .map(|c| c.entry.as_u24())
        .collect();
    assert_eq!(table, vec![0x008030, 0x008038, 0x008040, 0x008048]);
}

/// The loop at `$00:8020` run once: sixteen times round, the branch back
/// taken fifteen times.
fn loop_log() -> ExecLog {
    let insn = |pc: u32, count: u32| ExecInsn {
        pc,
        abs: (pc & 0x7FFF) as i32,
        kind: MemKind::PrgRom,
        states: 1 | 2,
        count,
    };
    let flow = |from: u32, to: u32, kind: FlowKind, count: u32| Flow {
        from,
        to,
        kind,
        count,
    };
    let mut log = ExecLog {
        rom_crc32: 0,
        rom_size: 0x8000,
        insns: vec![
            insn(0x008020, 1),
            insn(0x008022, 10),
            insn(0x008025, 16),
            insn(0x008026, 16),
            insn(0x008028, 1),
            // The same instruction through a mirror bank adds up.
            insn(0x808022, 6),
        ],
        accesses: Vec::new(),
        flows: vec![
            flow(0x008026, 0x008022, FlowKind::Branch, 15),
            flow(0x008026, 0x008028, FlowKind::BranchNotTaken, 1),
        ],
        dma: Vec::new(),
    };
    log.normalize();
    log
}

#[test]
fn a_log_counts_blocks_and_edges() {
    let (rom, mut project, snap) = setup(fixtures::routines_lorom());
    project.exec_log = Some(Arc::new(loop_log()));
    let g = routine_graph(&rom, &project, &snap, at(0x008020)).unwrap();
    assert!(g.counted);
    let runs: Vec<Option<u64>> = g.blocks.iter().map(|b| b.runs).collect();
    assert_eq!(runs, vec![Some(1), Some(16), Some(1)]);
    let counts: Vec<Option<u64>> = g.edges.iter().map(|e| e.count).collect();
    assert_eq!(counts, vec![Some(1), Some(15), Some(1)]);
    // A routine the log never saw: every block ran zero times.
    let other = routine_graph(&rom, &project, &snap, at(0x008040)).unwrap();
    assert!(other.blocks.iter().all(|b| b.runs == Some(0)));
    let n = call_neighbourhood(&rom, &project, &snap, at(0x008020)).unwrap();
    assert_eq!(
        n.callers[0].sites[0].count,
        Some(0),
        "the JSR itself was not in the log"
    );
}

fn sizes_for(g: &romlens_core::graph::RoutineGraph) -> Vec<NodeSize> {
    g.blocks
        .iter()
        .map(|b| NodeSize {
            width: 160.0,
            height: 16.0 * b.offsets.len().max(1) as f64 + 8.0,
        })
        .collect()
}

fn specs(g: &romlens_core::graph::RoutineGraph) -> Vec<EdgeSpec> {
    g.edges
        .iter()
        .map(|e| EdgeSpec {
            from: e.from,
            to: e.to,
            back: e.back,
        })
        .collect()
}

#[test]
fn every_fixture_routine_lays_out_top_down() {
    let (rom, project, snap) = setup(fixtures::routines_lorom());
    for e in decompile::entries(&snap) {
        let g = routine_graph(&rom, &project, &snap, e).unwrap();
        let sizes = sizes_for(&g);
        let l = layout(&sizes, &specs(&g), &LayoutOptions::default());
        assert_eq!(l.rows[0], 0, "the entry is at the top");
        for (i, edge) in g.edges.iter().enumerate() {
            if !edge.back {
                assert!(
                    l.rows[edge.to] > l.rows[edge.from],
                    "{} edge {i} falls",
                    g.name
                );
            }
            assert!(l.edges[i].points.len() >= 2);
        }
    }
}

/// Every routine on the development ROM: a graph and a layout with no box
/// on another, and how long it takes.
#[test]
fn every_routine_on_the_development_rom_lays_out() {
    let Some(rom) = common::dev_rom() else { return };
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let started = Instant::now();
    let (mut routines, mut blocks, mut edges, mut biggest) = (0, 0, 0, 0);
    let mut slowest = (0u128, String::new());
    for e in decompile::entries(&snap) {
        let Ok(g) = routine_graph(&rom, &project, &snap, e) else {
            continue;
        };
        let t = Instant::now();
        let sizes = sizes_for(&g);
        let l = layout(&sizes, &specs(&g), &LayoutOptions::default());
        let us = t.elapsed().as_micros();
        if us > slowest.0 {
            slowest = (us, g.name.clone());
        }
        routines += 1;
        blocks += g.blocks.len();
        edges += g.edges.len();
        biggest = biggest.max(g.blocks.len());
        for a in 0..sizes.len() {
            for b in a + 1..sizes.len() {
                let (p, q) = (l.nodes[a], l.nodes[b]);
                let x = p.x + 1e-6 < q.x + sizes[b].width && q.x + 1e-6 < p.x + sizes[a].width;
                let y = p.y + 1e-6 < q.y + sizes[b].height && q.y + 1e-6 < p.y + sizes[a].height;
                assert!(!(x && y), "{}: blocks {a} and {b} overlap", g.name);
            }
        }
        let n = call_neighbourhood(&rom, &project, &snap, e).unwrap();
        assert!(
            n.callers
                .iter()
                .chain(&n.callees)
                .all(|c| !c.sites.is_empty())
        );
    }
    eprintln!(
        "{routines} routines, {blocks} blocks, {edges} edges, largest {biggest} blocks, {:?} in all; slowest layout {} µs ({})",
        started.elapsed(),
        slowest.0,
        slowest.1
    );
}
