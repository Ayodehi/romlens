//! Pseudo-C for one routine at a time (`docs/18-decompiler.md`).
//!
//! The stages, each in its own module: find the function's instructions
//! (`function`), split them into blocks and find the graph's dominators and
//! loops (`cfg`), lift each instruction to IR (`ir`, `lift`), then print it
//! (`emit`) against the declarations in `snes.h` (`header`).

pub mod canon;
pub mod cfg;
pub mod dataflow;
pub mod emit;
pub mod function;
pub mod header;
pub mod ir;
pub mod lift;
pub mod loops;
pub mod signature;
pub mod structure;

pub use cfg::{Block, BlockId, Cfg, Loop, Term};
pub use emit::{CToken, CTokenKind, Stats};
pub use function::{
    Callee, Dest, Function, FunctionError, Step, Transfer, containing, discover, entries,
};
pub use header::snes_h;
pub use signature::{Program, Summary};

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::project::Project;
use crate::rom::image::RomImage;

/// How far the output goes; each level is valid C.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// Every instruction as statements over the CPU's registers and flags,
    /// with a goto for every edge.
    Lift,
    /// After data flow: dead flags gone, values carried into their uses.
    Clean,
    /// After structuring and signatures.
    #[default]
    Full,
}

impl Level {
    pub const fn name(self) -> &'static str {
        match self {
            Level::Lift => "lift",
            Level::Clean => "clean",
            Level::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Option<Level> {
        match s {
            "lift" => Some(Level::Lift),
            "clean" => Some(Level::Clean),
            "full" => Some(Level::Full),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DecompileOptions {
    pub level: Level,
    /// Use the project's names for memory. Off, every access is `MEM8(…)`,
    /// which is what a test runtime backs with an array.
    pub names: bool,
    /// The direct page where the analysis does not know it.
    pub assume_dp: Option<u16>,
}

impl Default for DecompileOptions {
    fn default() -> Self {
        Self {
            level: Level::Full,
            names: true,
            assume_dp: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Decompiled {
    pub name: String,
    /// Canonical.
    pub entry: SnesAddress,
    /// A translation unit: `#include "snes.h"`, declarations, the function.
    pub text: String,
    pub tokens: Vec<CToken>,
    /// For each line of `text`, the instructions it came from.
    pub lines: Vec<Vec<FileOffset>>,
    pub warnings: Vec<String>,
    pub stats: Stats,
    /// Every routine (`false`) and call table (`true`) the text names.
    pub callees: Vec<(String, SnesAddress, bool)>,
}

impl Decompiled {
    /// The lines that came from the instruction at `off`.
    pub fn lines_for(&self, off: FileOffset) -> Vec<usize> {
        (0..self.lines.len())
            .filter(|&i| self.lines[i].contains(&off))
            .collect()
    }
}

/// Decompile the routine entered at `at`.
pub fn decompile(
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    at: SnesAddress,
    opts: &DecompileOptions,
) -> Result<Decompiled, FunctionError> {
    let program = Program::build(
        rom,
        snap,
        lift::LiftOptions {
            assume_dp: opts.assume_dp,
        },
    );
    let f = discover(rom, snap, &program.entries, at)?;
    Ok(render_with(rom, project, snap, &f, opts, Some(&program)))
}

/// Every routine's summary, for `render_with`.
pub fn program(rom: &RomImage, snap: &AnalysisSnapshot, opts: &DecompileOptions) -> Program {
    Program::build(
        rom,
        snap,
        lift::LiftOptions {
            assume_dp: opts.assume_dp,
        },
    )
}

/// Print a function already found, assuming every call reads and writes
/// everything.
pub fn render(
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    f: &Function,
    opts: &DecompileOptions,
) -> Decompiled {
    render_with(rom, project, snap, f, opts, None)
}

/// Print a function already found, with what every routine reads and
/// returns when `program` is given.
pub fn render_with(
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    f: &Function,
    opts: &DecompileOptions,
    program: Option<&Program>,
) -> Decompiled {
    let cfg = Cfg::build(f);
    let lifted = lift::lift(
        f,
        &cfg,
        lift::LiftOptions {
            assume_dp: opts.assume_dp,
        },
    );
    let mut lifted = lifted;
    // At `full` the registers become each routine's own variables.
    let canonical = opts.level == Level::Full && program.is_some();
    if opts.level != Level::Lift {
        let conv = match program {
            Some(p) if canonical => p.canonical_conventions(f.entry),
            Some(p) => p.conventions(f.entry),
            None => Default::default(),
        };
        dataflow::clean(rom, f, &cfg, &mut lifted, &conv);
        if canonical && let Some(p) = program {
            canon::canonicalize(rom, f, &cfg, &mut lifted, p, &conv);
        }
    }
    let mut warnings = lifted.warnings.clone();
    if f.truncated {
        warnings.push(format!(
            "the routine was cut short at {} instructions",
            function::MAX_INSNS
        ));
    }
    if let Some(log) = &project.exec_log {
        let mixed = f
            .steps
            .iter()
            .filter(|s| {
                log.insn(s.insn.address.as_u24())
                    .is_some_and(|i| i.mixed_widths())
            })
            .count();
        if mixed > 0 {
            warnings.push(format!(
                "{mixed} instructions ran with more than one register width; this shows the widths the analysis decoded"
            ));
        }
    }
    let asm = emit::asm_text(rom, project, snap, f);

    let mut names = emit::Namer::new(rom, project, snap, opts.names);
    names.summaries = program.map(|p| &p.summaries);
    if canonical {
        names.abis = program.map(|p| &p.abis);
    }
    names.plan_placeholders(&memory_uses(&lifted));
    let name = names.own(f.entry);
    // The body first, so every name it uses is known for the declarations.
    let mut body = emit::Emitter::new(&mut names);
    body.vars = lifted.vars.clone();
    body.modern = canonical;
    body.w.indent = 1;
    body.stats.instructions = f.steps.len() as u32;
    // A temporary nothing reads is a value kept only for its read.
    let read = read_temps(&lifted);
    for b in &mut lifted.blocks {
        for l in &mut b.lines {
            if let ir::Stmt::Assign {
                dst: ir::Place::Temp(t),
                value,
            } = &l.stmt
                && !read.contains(t)
            {
                l.stmt = ir::Stmt::Eval(value.clone());
            }
        }
    }
    let used = used_temps(&lifted);
    let renames: std::collections::BTreeMap<u32, u32> = used
        .iter()
        .enumerate()
        .map(|(i, &t)| (t, i as u32 + 1))
        .collect();
    dataflow::rename_temps(&mut lifted, &renames);
    let used: Vec<u32> = (1..=renames.len() as u32).collect();
    // Structured first at `full`: the counted loops take their counters
    // out of the declarations.
    let structured = (opts.level == Level::Full).then(|| {
        let (mut tree, gotos) = structure::Structurer::new(&cfg, &lifted.blocks).run();
        if canonical {
            loops::counted(&mut tree, &cfg, &mut lifted);
            canon::mark_unused(&mut lifted);
        }
        (tree, gotos)
    });
    body.vars = lifted.vars.clone();
    let mut declared = false;
    // The variables, by type, then the temporaries.
    for ty in [ir::CType::U16, ir::CType::U8, ir::CType::Bool] {
        let of: Vec<&str> = lifted
            .vars
            .iter()
            .filter(|v| v.ty == ty && !v.param && !v.unused)
            .map(|v| v.name.as_str())
            .collect();
        if !of.is_empty() {
            body.w.tok(ty.name(), CTokenKind::Type, None);
            body.w.w(" ");
            body.w.w(&of.join(", "));
            body.w.w(";");
            body.w.end(&[]);
            declared = true;
        }
    }
    if !used.is_empty() {
        body.w.tok("u32", CTokenKind::Type, None);
        let temps: Vec<String> = used.iter().map(|t| format!("t{t}")).collect();
        body.w.w(" ");
        body.w.w(&temps.join(", "));
        body.w.w(";");
        body.w.end(&[]);
        declared = true;
    }
    if declared {
        body.w.blank();
    }
    if let Some((tree, gotos)) = structured {
        emit::TreeLayout {
            f,
            cfg: &cfg,
            blocks: &lifted.blocks,
            gotos: &gotos,
        }
        .print(&mut body, &tree, &asm);
    } else {
        emit::GotoLayout {
            f,
            cfg: &cfg,
            blocks: &lifted.blocks,
        }
        .print(&mut body, &asm);
    }
    let stats = body.stats;
    let callees = body.names.callees();
    let body = std::mem::take(&mut body.w);

    let mut w = emit::Writer::default();
    w.comment_line(
        &format!(
            "{name} at {}: {} instructions, {} level. From Romlens; for reading, not for rebuilding the ROM.",
            f.entry,
            f.steps.len(),
            opts.level.name()
        ),
        &[],
    );
    for warning in &warnings {
        w.comment_line(&format!("note: {warning}"), &[]);
    }
    w.tok("#include", CTokenKind::Keyword, None);
    w.w(" \"snes.h\"");
    w.end(&[]);
    let decls = names.declarations();
    if !decls.is_empty() {
        w.blank();
        for d in decls {
            w.w(d);
            w.end(&[]);
        }
    }
    w.blank();
    if let Some(s) = program.and_then(|p| p.summaries.get(&f.entry)) {
        w.comment_line(&signature::describe(s), &[]);
    }
    match &lifted.abi {
        Some(abi) => {
            let ret = abi.ret.map(|(_, t)| t.name()).unwrap_or("void");
            w.tok(ret, CTokenKind::Type, None);
            w.w(" ");
            w.tok(&name, CTokenKind::Function, Some(f.entry));
            w.w("(");
            let mut first = true;
            for (k, t) in &abi.params {
                if !first {
                    w.w(", ");
                }
                first = false;
                w.tok(t.name(), CTokenKind::Type, None);
                w.w(" ");
                let v = lifted
                    .vars
                    .iter()
                    .find(|v| v.param && v.slot == *k)
                    .map(|v| v.name.clone())
                    .unwrap_or_else(|| k.name().to_owned());
                w.tok(&v, CTokenKind::Local, None);
            }
            for (k, t) in &abi.outs {
                if !first {
                    w.w(", ");
                }
                first = false;
                w.tok(t.name(), CTokenKind::Type, None);
                w.w(" *");
                w.tok(&format!("{}_out", k.name()), CTokenKind::Local, None);
            }
            if first {
                w.tok("void", CTokenKind::Type, None);
            }
            w.w(")");
        }
        None => {
            w.tok("void", CTokenKind::Type, None);
            w.w(" ");
            w.tok(&name, CTokenKind::Function, Some(f.entry));
            w.w("(");
            w.tok("void", CTokenKind::Type, None);
            w.w(")");
        }
    }
    w.end(&[0]);
    w.w("{");
    w.end(&[]);
    let base = w.text.len() as u32;
    w.text.push_str(&body.text);
    w.tokens.extend(body.tokens.into_iter().map(|mut t| {
        t.start += base;
        t
    }));
    w.lines.extend(body.lines);
    w.w("}");
    w.end(&[]);

    let lines = w
        .lines
        .iter()
        .map(|steps| {
            let mut offs: Vec<FileOffset> =
                steps.iter().map(|&i| f.steps[i].insn.file_offset).collect();
            offs.sort_unstable();
            offs.dedup();
            offs
        })
        .collect();
    Decompiled {
        name,
        entry: f.entry,
        text: w.text,
        tokens: w.tokens,
        lines,
        warnings,
        stats,
        callees,
    }
}

/// Every statement the printed code has, and every expression outside
/// one (conditions, switch indexes, returned values).
fn visit(lifted: &lift::Lifted, stmt: &mut dyn FnMut(&ir::Stmt), expr: &mut dyn FnMut(&ir::Expr)) {
    for b in &lifted.blocks {
        for l in &b.lines {
            stmt(&l.stmt);
        }
        if let Some(c) = &b.cond {
            expr(c);
        }
        if let Some((s, _)) = &b.switch {
            expr(s);
        }
        if let Some(x) = &b.exit {
            for s in x.before.iter().chain(&x.after) {
                stmt(s);
            }
            for (_, e) in &x.outs {
                expr(e);
            }
            if let Some(e) = &x.ret {
                expr(e);
            }
        }
    }
}

/// The expressions a statement reads.
fn stmt_exprs(s: &ir::Stmt) -> Vec<&ir::Expr> {
    use ir::{Place, Stmt};
    match s {
        Stmt::Assign { dst, value } => {
            let mut v = vec![value];
            if let Place::Mem { addr, .. } = dst {
                v.push(addr);
            }
            v
        }
        Stmt::Effect(_, args) => args.iter().collect(),
        Stmt::Call(ir::CallTarget::Table { index, .. }) => vec![index],
        Stmt::Eval(e) => vec![e],
        Stmt::Invoke { args, .. } => args.iter().collect(),
        _ => vec![],
    }
}

fn temps_in(e: &ir::Expr, out: &mut std::collections::BTreeSet<u32>) {
    e.walk(&mut |x| {
        if let ir::Expr::Temp(t) = x {
            out.insert(*t);
        }
    })
}

/// The temporaries the code reads.
fn read_temps(lifted: &lift::Lifted) -> std::collections::BTreeSet<u32> {
    let mut reads = std::collections::BTreeSet::new();
    let reads_cell = std::cell::RefCell::new(&mut reads);
    visit(
        lifted,
        &mut |s| {
            for e in stmt_exprs(s) {
                temps_in(e, &mut reads_cell.borrow_mut());
            }
        },
        &mut |e| temps_in(e, &mut reads_cell.borrow_mut()),
    );
    reads
}

/// The temporaries the printed code still reads or writes.
fn used_temps(lifted: &lift::Lifted) -> std::collections::BTreeSet<u32> {
    let mut out = std::collections::BTreeSet::new();
    let cell = std::cell::RefCell::new(&mut out);
    visit(
        lifted,
        &mut |s| {
            match s {
                ir::Stmt::Assign {
                    dst: ir::Place::Temp(t),
                    ..
                }
                | ir::Stmt::Invoke {
                    ret: Some(ir::Place::Temp(t)),
                    ..
                } => {
                    cell.borrow_mut().insert(*t);
                }
                _ => {}
            }
            for e in stmt_exprs(s) {
                temps_in(e, &mut cell.borrow_mut());
            }
        },
        &mut |e| temps_in(e, &mut cell.borrow_mut()),
    );
    out
}

/// Every memory access at a constant address (or a constant base plus an
/// index): the address, the width, and whether it is indexed.
fn memory_uses(lifted: &lift::Lifted) -> Vec<(u32, ir::Width, bool)> {
    use ir::{BinOp, Expr, Place, Stmt};
    fn at(addr: &Expr, w: ir::Width, out: &mut Vec<(u32, ir::Width, bool)>) {
        match addr {
            Expr::Const(a) => out.push((*a, w, false)),
            Expr::Bin(BinOp::Add, x, y) => {
                if let (_, Expr::Const(a)) | (Expr::Const(a), _) = (&**x, &**y) {
                    out.push((*a, w, true));
                }
            }
            _ => {}
        }
    }
    fn expr(e: &Expr, out: &mut Vec<(u32, ir::Width, bool)>) {
        e.walk(&mut |x| {
            if let Expr::Mem { addr, width } = x {
                at(addr, *width, out);
            }
        })
    }
    let mut out = Vec::new();
    let cell = std::cell::RefCell::new(&mut out);
    visit(
        lifted,
        &mut |s| {
            if let Stmt::Assign {
                dst: Place::Mem { addr, width },
                ..
            } = s
            {
                at(addr, *width, &mut cell.borrow_mut());
            }
            for e in stmt_exprs(s) {
                expr(e, &mut cell.borrow_mut());
            }
        },
        &mut |e| expr(e, &mut cell.borrow_mut()),
    );
    out
}
