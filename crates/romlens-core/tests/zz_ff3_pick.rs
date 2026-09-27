use romlens_core::RomImage;
use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::cpu65816::AddressingMode;
use romlens_core::decompile::{self, DecompileOptions, discover, entries};
use romlens_core::model::project::Project;
#[test]
#[ignore]
fn pick() {
    let rom = RomImage::load(std::path::Path::new(
        "/Users/benjaminbarbour/Documents/SD2SNES/owned/FinalFantasy3.5F32.sfc",
    ))
    .unwrap();
    let project = Project::new(&rom);
    let snap = analyze(&rom, &project, &AnalysisControl::silent()).unwrap();
    let opts = DecompileOptions::default();
    let program = decompile::program(&rom, &project, &snap, &opts);
    let all = entries(&snap);
    for &e in &all {
        let Ok(f) = discover(&rom, &snap, &all, e) else {
            continue;
        };
        if !f.steps.iter().any(|s| {
            matches!(
                s.insn.mode,
                AddressingMode::StackRelative | AddressingMode::StackRelativeIndirectIndexed
            )
        }) {
            continue;
        }
        let d = decompile::render_with(&rom, &project, &snap, &f, &opts, Some(&program));
        let clean = !d.text.contains("STACK");
        let args = d
            .text
            .lines()
            .any(|l| l.contains(&format!("{}(", d.name)) && l.contains("arg"));
        if clean || args {
            eprintln!(
                "PICK {e} {} insns={} clean={clean} args={args}",
                d.name,
                f.steps.len()
            );
        }
    }
}
