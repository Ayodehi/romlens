use romlens_core::RomImage;
use romlens_core::analysis::{AnalysisControl, analyze};
use romlens_core::cpu65816::AddressingMode;
use romlens_core::decompile::{self, DecompileOptions, discover, entries};
use romlens_core::model::project::Project;

#[test]
#[ignore]
fn survey() {
    let root = "/Users/benjaminbarbour/Documents/SD2SNES";
    let mut names = Vec::new();
    for d in ["owned", "Ben's Games", "games", "homebrew"] {
        for e in std::fs::read_dir(format!("{root}/{d}")).unwrap().flatten() {
            let p = e.path();
            if p.extension()
                .is_some_and(|x| ["sfc", "smc", "fig"].contains(&x.to_str().unwrap_or("")))
            {
                names.push(p);
            }
        }
    }
    names.sort();
    let (mut gt, mut gw, mut gl) = (0, 0, 0);
    let mut reasons: std::collections::BTreeMap<String, u32> = Default::default();
    for p in names {
        let label = format!(
            "{}/{}",
            p.parent().unwrap().file_name().unwrap().to_string_lossy(),
            p.file_name().unwrap().to_string_lossy()
        );
        let Ok(rom) = RomImage::load(&p) else {
            eprintln!("{label:60} could not load");
            continue;
        };
        let project = Project::new(&rom);
        let Ok(snap) = analyze(&rom, &project, &AnalysisControl::silent()) else {
            continue;
        };
        let all = entries(&snap);
        let opts = DecompileOptions::default();
        let program = decompile::program(&rom, &project, &snap, &opts);
        let mut left = 0;
        let mut left_ex = Vec::new();
        let (mut total, mut with, mut ind) = (0, 0, 0);
        let mut ex = Vec::new();
        for &e in &all {
            let Ok(f) = discover(&rom, &snap, &all, e) else {
                continue;
            };
            total += 1;
            let sr: Vec<_> = f
                .steps
                .iter()
                .filter(|s| {
                    matches!(
                        s.insn.mode,
                        AddressingMode::StackRelative
                            | AddressingMode::StackRelativeIndirectIndexed
                    )
                })
                .collect();
            if !sr.is_empty() {
                with += 1;
                if sr
                    .iter()
                    .any(|s| s.insn.mode == AddressingMode::StackRelativeIndirectIndexed)
                {
                    ind += 1;
                }
                if ex.len() < 3 {
                    ex.push(format!("{e}"));
                }
                let d = decompile::render_with(&rom, &project, &snap, &f, &opts, Some(&program));
                let t = &d.text;
                if t.contains("STACK8(") || t.contains("STACK16(") {
                    left += 1;
                    let why = d
                        .warnings
                        .iter()
                        .find(|w| w.starts_with("the stack is left"))
                        .cloned()
                        .unwrap_or_else(|| "(no reason given)".into());
                    *reasons.entry(why).or_insert(0) += 1;
                    if left_ex.len() < 4 {
                        left_ex.push(format!("{e}"));
                    }
                }
            }
        }
        gt += total;
        gw += with;
        eprintln!(
            "SURVEY {label:60} {total:5} routines {with:4} use d,S ({ind:3} (d,S),Y), {left:3} still read the stack as memory  {}",
            left_ex.join(" ")
        );
        gl += left;
    }
    for (r, n) in &reasons {
        eprintln!("SURVEY {n:4} {r}");
    }
    eprintln!("SURVEY total {gt} routines, {gw} use d,S, {gl} still read the stack as memory");
}
