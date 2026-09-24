//! Pseudo-C for one routine at a time (`docs/18-decompiler.md`).
//!
//! The stages, each in its own module: find the function's instructions
//! (`function`), split them into blocks and find the graph's dominators and
//! loops (`cfg`), lift each instruction to IR (`ir`, `lift`), then print it
//! (`emit`) against the declarations in `snes.h` (`header`).

pub mod cfg;
pub mod dataflow;
pub mod emit;
pub mod function;
pub mod header;
pub mod ir;
pub mod lift;
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
    if opts.level != Level::Lift {
        let conv = program.map(|p| p.conventions(f.entry)).unwrap_or_default();
        dataflow::clean(rom, f, &cfg, &mut lifted, &conv);
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
    let name = names.own(f.entry);
    // The body first, so every name it uses is known for the declarations.
    let mut body = emit::Emitter::new(&mut names);
    body.w.indent = 1;
    body.stats.instructions = f.steps.len() as u32;
    let used = used_temps(&lifted);
    let renames: std::collections::BTreeMap<u32, u32> = used
        .iter()
        .enumerate()
        .map(|(i, &t)| (t, i as u32 + 1))
        .collect();
    dataflow::rename_temps(&mut lifted, &renames);
    let used: Vec<u32> = (1..=renames.len() as u32).collect();
    if !used.is_empty() {
        body.w.tok("u32", CTokenKind::Type, None);
        let temps: Vec<String> = used.iter().map(|t| format!("t{t}")).collect();
        body.w.w(" ");
        body.w.w(&temps.join(", "));
        body.w.w(";");
        body.w.end(&[]);
        body.w.blank();
    }
    if opts.level == Level::Full {
        let (tree, gotos) = structure::Structurer::new(&cfg, &lifted.blocks).run();
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
    w.tok("void", CTokenKind::Type, None);
    w.w(" ");
    w.tok(&name, CTokenKind::Function, Some(f.entry));
    w.w("(");
    w.tok("void", CTokenKind::Type, None);
    w.w(")");
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

/// The temporaries the printed code still reads or writes.
fn used_temps(lifted: &lift::Lifted) -> std::collections::BTreeSet<u32> {
    use ir::{Expr, Place, Stmt};
    let mut out = std::collections::BTreeSet::new();
    let see = |e: &Expr, out: &mut std::collections::BTreeSet<u32>| {
        e.walk(&mut |x| {
            if let Expr::Temp(t) = x {
                out.insert(*t);
            }
        })
    };
    for b in &lifted.blocks {
        for l in &b.lines {
            match &l.stmt {
                Stmt::Assign { dst, value } => {
                    match dst {
                        Place::Temp(t) => {
                            out.insert(*t);
                        }
                        Place::Mem { addr, .. } => see(addr, &mut out),
                        _ => {}
                    }
                    see(value, &mut out);
                }
                Stmt::Effect(_, args) => args.iter().for_each(|a| see(a, &mut out)),
                Stmt::Call(ir::CallTarget::Table { index, .. }) => see(index, &mut out),
                _ => {}
            }
        }
        if let Some(c) = &b.cond {
            see(c, &mut out);
        }
        if let Some((s, _)) = &b.switch {
            see(s, &mut out);
        }
    }
    out
}
