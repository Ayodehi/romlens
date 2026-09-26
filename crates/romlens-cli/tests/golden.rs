//! CLI output pinned per fixture. Line endings and the separator after a
//! redacted temporary path are normalised so the same files pass on
//! Windows. `UPDATE_GOLDEN=1` rewrites them.

use std::path::{Path, PathBuf};
use std::process::Command;

use romlens_core::{MappingMode, fixtures};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("romlens-golden-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fixture(dir: &Path, mode: MappingMode) -> PathBuf {
    let path = dir.join(fixtures::file_name(mode));
    std::fs::write(&path, fixtures::for_mapping(mode)).unwrap();
    path
}

/// stdout, plus `[exit code N]` and stderr on failure. stderr is otherwise
/// dropped so timings never enter a golden.
fn run(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_romlens"))
        .args(args)
        .output()
        .expect("romlens runs");
    let stdout = String::from_utf8(out.stdout).unwrap().replace("\r\n", "\n");
    let stderr = String::from_utf8(out.stderr).unwrap().replace("\r\n", "\n");
    format!(
        "{stdout}{}",
        if out.status.success() {
            String::new()
        } else {
            format!("[exit code {}]\n{stderr}", out.status.code().unwrap_or(-1))
        }
    )
}

/// `run`, with the arguments in two parts.
///
/// A command long enough to need a wrapped array literal is formatted
/// differently by different rustfmt versions, and a test harness is not worth
/// a toolchain argument. Two short slices are unambiguous to all of them.
fn run_with(head: &[&str], tail: &[&str]) -> String {
    run(&[head, tail].concat())
}

fn check(name: &str, actual: &str) {
    let path = golden_dir().join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| {
            panic!(
                "missing golden {}; run with UPDATE_GOLDEN=1",
                path.display()
            )
        })
        .replace("\r\n", "\n");
    assert_eq!(
        actual, expected,
        "golden {name} differs (UPDATE_GOLDEN=1 to accept)"
    );
}

/// Replaces the scratch directory with `<tmp>` and normalises the separator
/// that follows it, so Windows' `<tmp>\\Test.romlens` matches a golden written
/// on macOS. Only the separator directly after the marker is rewritten; the
/// rest of the output is left exactly as the CLI printed it.
fn redact_tmp(dir: &Path, text: &str) -> String {
    text.replace(&*dir.to_string_lossy(), "<tmp>")
        .replace("<tmp>\\", "<tmp>/")
}

fn slug(mode: MappingMode) -> &'static str {
    match mode {
        MappingMode::LoRom => "lorom",
        MappingMode::HiRom => "hirom",
        MappingMode::ExHiRom => "exhirom",
    }
}

#[test]
fn fixtures_match_goldens() {
    let dir = temp_dir("fixtures");
    for mode in MappingMode::all() {
        let rom = write_fixture(&dir, mode);
        let rom = rom.to_str().unwrap();
        check(&format!("info-{}", slug(mode)), &run(&["info", rom]));
        check(
            &format!("info-json-{}", slug(mode)),
            &run(&["info", rom, "--json"]),
        );
        let header = mode.header_offset().to_string();
        check(
            &format!("hex-{}", slug(mode)),
            &format!(
                "{}{}{}",
                run(&["hex", rom, "--rows", "2"]),
                run(&["hex", rom, "--from", &header, "--rows", "4"]),
                run(&[
                    "hex",
                    rom,
                    "--from",
                    "$00:8000",
                    "--rows",
                    "1",
                    "--address",
                    "snes"
                ])
            ),
        );
        check(
            &format!("resolve-{}", slug(mode)),
            &format!(
                "{}{}{}{}",
                run(&["resolve", rom, "$00:8000"]),
                run(&["resolve", rom, &header]),
                run(&["resolve", rom, "$7E:0000"]),
                run(&["resolve", rom, "0xFFFFFFF"])
            ),
        );
        check(
            &format!("disasm-{}", slug(mode)),
            &run(&[
                "disasm",
                rom,
                "--from",
                "$00:8000",
                "--count",
                "16",
                "--verbose",
            ]),
        );
        check(
            &format!("analyze-{}", slug(mode)),
            &format!(
                "{}{}",
                run(&["analyze", rom, "--stats"]),
                run(&["analyze", rom, "--json"])
            ),
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lorom_analysis_commands() {
    let dir = temp_dir("lorom");
    let rom = write_fixture(&dir, MappingMode::LoRom);
    let rom = rom.to_str().unwrap();
    // Dispatch tables, on a fixture so the golden needs no commercial ROM.
    let dispatch = dir.join("dispatch.sfc");
    std::fs::write(&dispatch, fixtures::dispatch_lorom()).unwrap();
    let dispatch = dispatch.to_str().unwrap();
    check(
        "tables-dispatch",
        &format!(
            "{}{}{}",
            run(&["tables", dispatch, "--entries", "--unresolved"]),
            run(&["tables", dispatch, "--json"]),
            run(&["analyze", dispatch, "--warnings"]),
        ),
    );
    check(
        "tables-none",
        &run(&["tables", rom, "--entries", "--unresolved"]),
    );
    // Scored guesses, likewise on a fixture.
    let mixed = dir.join("mixed.sfc");
    std::fs::write(&mixed, fixtures::mixed_data_lorom()).unwrap();
    let mixed = mixed.to_str().unwrap();
    check(
        "heuristics-mixed",
        &format!(
            "{}{}{}",
            run(&["heuristics", mixed]),
            run(&["heuristics", mixed, "--kind", "palette", "--json"]),
            run(&["analyze", mixed, "--stats"]),
        ),
    );
    check(
        "heuristics-mixed-off",
        &run(&["analyze", mixed, "--stats", "--no-heuristics"]),
    );
    // Accuracy against the truth the fixture builder states, which is the one
    // ground truth CI can hold (`12-content-policy.md`).
    check(
        "map-mixed",
        &format!(
            "{}{}",
            run(&["map", mixed, "--buckets", "64", "--width", "32"]),
            run(&["map", mixed, "--buckets", "8", "--json"]),
        ),
    );
    check(
        "accuracy-fixtures",
        &format!(
            "{}{}{}",
            run(&["accuracy", rom, "--fixture"]),
            run(&["accuracy", mixed, "--fixture"]),
            run(&["accuracy", dispatch, "--fixture", "--json"]),
        ),
    );
    check("labels-lorom", &run(&["labels", rom]));
    check(
        "xrefs-lorom",
        &format!(
            "{}{}",
            run(&["xrefs", rom, "$00:800E"]),
            run(&["xrefs", rom, "0x7"])
        ),
    );
    check(
        "export-asm-lorom",
        &format!(
            "{}{}",
            run(&[
                "export",
                "asm",
                rom,
                "--out",
                "-",
                "--range",
                "$00:8000..$00:8020"
            ]),
            run(&[
                "export",
                "asm",
                rom,
                "--out",
                "-",
                "--range",
                "0x7FC0..0x8000"
            ])
        ),
    );
    check(
        "export-sym-lorom",
        &run(&["export", "sym", rom, "--out", "-", "--include-auto"]),
    );
    check(
        "inspect-lorom",
        &format!(
            "{}\n{}",
            run(&["inspect", rom, "$00:8007"]),
            run(&["inspect", rom, "$00:2100"])
        ),
    );
    check(
        "search-lorom",
        &format!(
            "{}{}{}{}",
            run(&["search", rom, "78 18 FB"]),
            run_with(
                &["search", rom, "8D ?? 21"],
                &["--from", "$00:8000", "--to", "$00:8010"],
            ),
            // Text, and text in either case: the fixture's title is upper
            // case, so only the second finds it.
            run(&["search", rom, "romlens", "--text"]),
            run(&["search", rom, "romlens", "--ignore-case"]),
        ),
    );
    check(
        "disasm-flags-lorom",
        &run(&[
            "disasm",
            rom,
            "--from",
            "$00:8000",
            "--count",
            "8",
            "--flags",
            "m0x0e0",
            "--address",
            "snes",
        ]),
    );
    check(
        "registers",
        &format!(
            "{}{}{}{}{}",
            run(&["registers", "$420D"]),
            run(&["registers", "$2145"]),
            run(&["registers", "$4200", "--value", "$81"]),
            run(&["registers", "$4300", "--value", "0x1801"]),
            run(&["registers", "$2107"])
        ),
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn all_opcodes_disassemble() {
    let dir = temp_dir("opcodes");
    let rom = dir.join("ops.sfc");
    let wrote = run(&["testrom", "--out", rom.to_str().unwrap(), "--all-opcodes"]);
    assert!(wrote.contains("all-opcodes"), "{wrote}");
    assert_eq!(std::fs::read(&rom).unwrap(), fixtures::all_opcodes_lorom());
    check(
        "disasm-allopcodes",
        &run(&[
            "disasm",
            rom.to_str().unwrap(),
            "--count",
            "256",
            "--flags",
            "m0x0e0",
            "--address",
            "file",
        ]),
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A scripted editing session: init, label, comment, mark, flags, list,
/// and the listing that results; then remove each and check the package is
/// empty again.
#[test]
fn project_scenario() {
    let dir = temp_dir("project");
    let rom = write_fixture(&dir, MappingMode::LoRom);
    let rom = rom.to_str().unwrap();
    let pkg = dir.join("Test.romlens");
    let pkg = pkg.to_str().unwrap();
    let mut log = String::new();
    log += &redact_tmp(&dir, &run(&["project", pkg, "init", "--rom", rom]));
    log += &redact_tmp(&dir, &run(&["project", pkg, "init", "--rom", rom]));
    log += &run(&["project", pkg, "label", "$00:8000", "Boot"]);
    log += &run(&["project", pkg, "label", "$80:800E", "Idle"]);
    log += &run(&["project", pkg, "label", "$7E:0A1C", "SamusPose"]);
    log += &run(&["project", pkg, "label", "$00:8007", "bad name"]);
    log += &run(&[
        "project",
        pkg,
        "comment",
        "$00:8000",
        "disable IRQ",
        "--line",
    ]);
    log += &run(&[
        "project",
        pkg,
        "comment",
        "$00:8007",
        "Force blank\nthen spin",
        "--block",
    ]);
    log += &run(&["project", pkg, "mark", "0x20", "6", "word"]);
    // The vector table is a table of code pointers, so typing it that way
    // renders it as the routines it names.
    log += &run_with(
        &["project", pkg, "mark", "0x7FFC", "4", "table"],
        &["--stride", "2", "--elem", "code", "--bank", "same"],
    );
    log += &run(&["project", pkg, "mark", "0x22", "2", "code"]);
    log += &run(&["project", pkg, "mark", "0x100", "0", "byte"]);
    log += &run(&["project", pkg, "flags", "0x5", "--m", "0", "--dbr", "$7E"]);
    log += &redact_tmp(&dir, &run(&["project", pkg, "history"]));
    log += &run(&["disasm", rom, "--project", pkg, "--count", "24"]);
    log += &run_with(
        &["disasm", rom, "--project", pkg],
        &["--from", "0x7FFC", "--count", "3"],
    );
    log += &run(&["labels", rom, "--project", pkg, "--source", "user"]);
    log += &run(&["export", "sym", rom, "--project", pkg, "--out", "-"]);
    log += &run(&["project", pkg, "label", "$00:8000", "-"]);
    log += &run(&["project", pkg, "label", "$00:800E", "-"]);
    log += &run(&["project", pkg, "label", "$7E:0A1C", "-"]);
    log += &run(&["project", pkg, "comment", "$00:8000", "-", "--line"]);
    log += &run(&["project", pkg, "comment", "$00:8007", "-", "--block"]);
    log += &run(&["project", pkg, "clear", "0x20", "6"]);
    log += &run(&["project", pkg, "clear", "0x7FFC", "4"]);
    log += &run(&["project", pkg, "flags", "0x5", "--remove"]);
    log += &redact_tmp(&dir, &run(&["project", pkg, "history"]));
    check("project-lorom", &log);

    // Trace import, on a CDL this test writes: the format is one byte per ROM
    // byte in file order, so a fixture needs no emulator (`16-phase2-plan.md`).
    let mut cdl = b"CDLv2".to_vec();
    cdl.extend_from_slice(&0u32.to_le_bytes());
    let mut payload = vec![0u8; fixtures::minimal_lorom().len()];
    // The two unreachable NOPs ran; a subroutine at 0x10 the walk never finds;
    // sixteen bytes read as data.
    payload[0x0C] = 0x01 | 0x04 | 0x20 | 0x10; // code, jump target, 8-bit M and X
    payload[0x0D] = 0x01 | 0x20 | 0x10;
    for b in payload.iter_mut().skip(0x10).take(3) {
        *b = 0x01 | 0x20 | 0x10;
    }
    payload[0x10] |= 0x08; // subroutine entry
    for b in payload.iter_mut().skip(0x200).take(16) {
        *b = 0x02;
    }
    cdl.extend_from_slice(&payload);
    let cdl_path = dir.join("play.cdl");
    std::fs::write(&cdl_path, &cdl).unwrap();
    let cdl_arg = cdl_path.to_str().unwrap();
    let traced = dir.join("Traced.romlens");
    let traced = traced.to_str().unwrap();
    let truth_path = dir.join("play.tsv");
    let mut trace_log = redact_tmp(&dir, &run(&["project", traced, "init", "--rom", rom]));
    trace_log += &redact_tmp(
        &dir,
        &run(&["import", "trace", traced, "--rom", rom, cdl_arg]),
    );
    trace_log += &run(&["analyze", rom, "--project", traced, "--stats"]);
    trace_log += &run(&["labels", rom, "--project", traced]);
    trace_log += &redact_tmp(
        &dir,
        &run(&[
            "truth",
            "from-cdl",
            rom,
            cdl_arg,
            "--out",
            truth_path.to_str().unwrap(),
        ]),
    );
    trace_log += &run_with(
        &["accuracy", rom, "--project", traced],
        &["--truth", truth_path.to_str().unwrap()],
    );
    check("trace-lorom", &trace_log);

    // Symbol import: a user name survives, others are rewritten and reported,
    // and the licence notice travels with what it covers.
    let sym_path = dir.join("theirs.sym");
    std::fs::write(
        &sym_path,
        "; Symbols for the Romlens test ROM\n\
; Licence: 0BSD\n\
\n\
[labels]\n\
00:8000 Reset::Entry\n\
00:800A main.loop\n\
00:800E NMI@handler\n\
00:800a main@loop\n\
nonsense\n\
[comments]\n\
00:800A the spin\n",
    )
    .unwrap();
    let sym_arg = sym_path.to_str().unwrap();
    let imported = dir.join("Imported.romlens");
    let imported = imported.to_str().unwrap();
    let mut sym_log = redact_tmp(&dir, &run(&["project", imported, "init", "--rom", rom]));
    sym_log += &run(&["project", imported, "label", "$00:8000", "MyOwnBoot"]);
    sym_log += &redact_tmp(
        &dir,
        &run(&["import", "symbols", imported, "--rom", rom, sym_arg]),
    );
    sym_log += &run(&["labels", rom, "--project", imported]);
    // Straight back out through our own exporter, which is the round trip the
    // WLA reader exists to get for free.
    sym_log += &run(&["export", "sym", rom, "--project", imported, "--out", "-"]);
    check("symbols-lorom", &sym_log);
    // The package is the same five files an empty project writes.
    let empty = romlens_core::io::to_files(
        &romlens_core::RomImage::load(rom).unwrap(),
        &romlens_core::model::Project::new(&romlens_core::RomImage::load(rom).unwrap()),
    );
    let back = romlens_core::io::read_package(Path::new(pkg)).unwrap();
    assert_eq!(back, empty);
    // A different ROM is refused.
    let other = write_fixture(&dir, MappingMode::HiRom);
    let refused = run(&[
        "project",
        pkg,
        "label",
        "$00:8000",
        "X",
        "--rom",
        other.to_str().unwrap(),
    ]);
    assert!(refused.contains("different ROM"), "{refused}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn graphics_commands() {
    let dir = temp_dir("graphics");
    let rom = dir.join("romlens-graphics.sfc");
    std::fs::write(&rom, fixtures::graphics_lorom()).unwrap();
    let rom = rom.to_str().unwrap();
    // One tile with its planes, then the whole 4 bpp block as a grid, the
    // 2 bpp letters, and the sheet in a real palette as characters and as a
    // digest.
    check(
        "tiles-graphics",
        &[
            run(&["tiles", rom, "--from", "0x10A0"]),
            run_with(
                &["tiles", rom, "--from", "0x1000"],
                &["--count", "32", "--columns", "8"],
            ),
            run_with(
                &["tiles", rom, "--from", "0x1400"],
                &["--bpp", "2", "--count", "8"],
            ),
            run_with(
                &["tiles", rom, "--from", "0x1000", "--count", "32"],
                &["--ascii"],
            ),
            run_with(
                &["tiles", rom, "--from", "0x1000", "--count", "32"],
                &["--digest", "--palette", "0x1820"],
            ),
            run_with(&["tiles", rom, "--from", "0x1000"], &["--bpp", "3"]),
        ]
        .concat(),
    );
    check(
        "tiles-json-graphics",
        &run_with(
            &["tiles", rom, "--from", "0x1000"],
            &["--count", "2", "--json"],
        ),
    );
    check(
        "palette-graphics",
        &[
            run_with(&["palette", rom, "--from", "0x1800"], &["--count", "32"]),
            run_with(
                &["palette", rom, "--from", "0x1800"],
                &["--count", "2", "--json"],
            ),
        ]
        .concat(),
    );
    check(
        "oam-graphics",
        &[
            run(&["oam", rom, "--from", "0x1A00", "--visible"]),
            run_with(
                &["oam", rom, "--from", "0x1A00"],
                &["--obsel", "$60", "--sort", "priority", "--visible"],
            ),
            run_with(&["oam", rom, "--from", "0x1A00"], &["--sort", "screen"]),
        ]
        .concat(),
    );
    check(
        "tilemap-graphics",
        &[
            run(&["tilemap", rom, "--from", "0x2000"]),
            run_with(&["tilemap", rom, "--from", "0x2000"], &["--size", "65x65"]),
        ]
        .concat(),
    );
    let out = dir.join("tiles.bin");
    let out = out.to_str().unwrap();
    check(
        "decompress-graphics",
        &redact_tmp(
            &dir,
            &[
                run_with(&["decompress", rom, "--from", "0x3000"], &["--stats"]),
                run_with(&["decompress", rom, "--from", "0x3000"], &["--out", out]),
                run_with(&["decompress", rom, "--from", "0x1000"], &[]),
            ]
            .concat(),
        ),
    );
    assert_eq!(
        std::fs::read(out).unwrap(),
        &fixtures::graphics_lorom()[0x1000..0x1400]
    );
    // Typed ranges preview in `inspect` (checklist 2.25).
    let pkg = dir.join("g.romlens");
    let pkg = pkg.to_str().unwrap();
    let mut text = run(&["project", pkg, "init", "--rom", rom]);
    for (at, len, kind) in [
        ("0x1000", "1024", "graphics"),
        ("0x1800", "512", "palette"),
        ("0x2000", "2048", "tilemap"),
        ("0x3000", "433", "compressed"),
    ] {
        text += &run_with(&["project", pkg, "mark", at, len, kind], &["--rom", rom]);
    }
    text += &run_with(
        &["project", pkg, "preview", "0x2000", "--tiles", "0x1000"],
        &["--palette", "0x1800", "--rom", rom],
    );
    text += &run_with(&["project", pkg, "preview", "0x2001"], &["--rom", rom]);
    text += &run_with(
        &["project", pkg, "preview", "0x1000", "--columns", "8"],
        &["--palette", "0x1820", "--rom", rom],
    );
    for at in ["0x1000", "0x1800", "0x2000", "0x3000"] {
        let out = run(&["inspect", rom, at, "--project", pkg]);
        // The preview line and the image line under it.
        let lines: Vec<&str> = out.lines().collect();
        if let Some(i) = lines.iter().position(|l| l.starts_with("Preview")) {
            for line in &lines[i..(i + 2).min(lines.len())] {
                text += line;
                text += "\n";
            }
        }
    }
    check("preview-graphics", &redact_tmp(&dir, &text));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recording_commands() {
    let dir = temp_dir("recording");
    let rom = dir.join("romlens-graphics.sfc");
    std::fs::write(&rom, fixtures::graphics_lorom()).unwrap();
    let rom = rom.to_str().unwrap();
    let rec = dir.join("t.romrec");
    let rec = rec.to_str().unwrap();
    let other = write_fixture(&dir, MappingMode::LoRom);
    let other = other.to_str().unwrap();
    let mut out = run(&["testrec", "--out", rec, "--frames", "90"]);
    out += &run(&["rec", "validate", rec, "--rom", rom, "--sample", "8"]);
    out += &run(&["rec", "validate", rec, "--rom", other]);
    let cut = dir.join("cut.romrec");
    let cut = cut.to_str().unwrap();
    let whole = std::fs::read(rec).unwrap();
    std::fs::write(cut, &whole[..whole.len() * 2 / 3]).unwrap();
    out += &run(&["rec", "validate", cut]);
    out += &run(&["rec", "validate", cut, "--recover", "--strict"]);
    let window = dir.join("window.romrec");
    let window = window.to_str().unwrap();
    out += &redact_tmp(
        &dir,
        &run_with(
            &["rec", "convert", rec, "--from", "25"],
            &["--to", "35", "--out", window],
        ),
    );
    out += &redact_tmp(&dir, &run(&["rec", "validate", window, "--rom", rom]));
    out += &run_with(
        &["rec", "when", window, "--region", "vram"],
        &["--offset", "0xA0"],
    );
    out += &redact_tmp(
        &dir,
        &run_with(
            &["rec", "convert", rec, "--from", "80"],
            &["--to", "95", "--out", window],
        ),
    );
    out += &run(&["rec", "info", rec, "--rom", rom]);
    out += &run(&["rec", "info", rec, "--rom", other]);
    // Frame 30 rewrites tile 5 in VRAM ($A0-$BF); CGRAM entry 17 cycles.
    out += &redact_tmp(&dir, &run(&["rec", "index", rec]));
    out += &run_with(
        &["rec", "when", rec, "--region", "vram"],
        &["--offset", "0xA0", "--len", "32"],
    );
    out += &run_with(
        &["rec", "when", rec, "--region", "vram"],
        &["--offset", "0xA0", "--after", "30"],
    );
    out += &run_with(
        &["rec", "when", rec, "--region", "vram"],
        &["--offset", "0xA0", "--after", "50", "--backward"],
    );
    out += &run_with(
        &["rec", "when", rec, "--region", "cgram"],
        &["--offset", "34", "--len", "2", "--after", "3"],
    );
    out += &run_with(
        &["rec", "when", rec, "--region", "vram"],
        &["--offset", "0x10000"],
    );
    out += &run_with(
        &["rec", "changes", rec, "--region", "vram"],
        &["--from", "29", "--to", "30"],
    );
    out += &run_with(
        &["rec", "extract", rec],
        &["--frame", "40", "--region", "oam", "--hex"],
    );
    out += &run_with(
        &["rec", "extract", rec],
        &["--frame", "90", "--region", "oam", "--hex"],
    );
    out += &run_with(
        &["rec", "extract", rec],
        &["--frame", "0", "--region", "sram"],
    );
    out += &run_with(&["render", "bg", "--rec", rec], &["--bg", "1"]);
    out += &run_with(
        &["render", "bg", "--rec", rec],
        &["--bg", "1", "--frame", "30"],
    );
    out += &run_with(&["render", "bg", "--rec", rec], &["--bg", "3"]);
    out += &run_with(&["render", "bg", "--rec", rec], &["--bg", "4"]);
    out += &run_with(
        &["render", "sprite", "--rec", rec],
        &["--index", "0", "--ascii"],
    );
    out += &run_with(
        &["render", "sprite", "--rec", rec],
        &["--index", "3", "--frame", "8"],
    );
    // The whole screen, and what drew two of its pixels: a sprite and BG1.
    out += &run_with(&["render", "frame", "--rec", rec], &["--frame", "8"]);
    out += &run_with(
        &["render", "frame", "--rec", rec],
        &["--frame", "8", "--at", "150,55"],
    );
    out += &run_with(
        &["render", "frame", "--rec", rec],
        &["--frame", "8", "--at", "68,52"],
    );
    // Dump frame 0, import the dumps as a one-frame recording, and draw the
    // same picture from it.
    for region in ["vram", "cgram", "oam", "ppu"] {
        let f = dir.join(format!("{region}.bin"));
        out += &run_with(
            &["rec", "extract", rec, "--frame", "0", "--region", region],
            &["--out", f.to_str().unwrap()],
        );
    }
    let raw = dir.join("raw.romrec");
    let raw = raw.to_str().unwrap();
    let arg = |r: &str| dir.join(format!("{r}.bin")).to_string_lossy().into_owned();
    let (vram, cgram, oam, ppu) = (arg("vram"), arg("cgram"), arg("oam"), arg("ppu"));
    out += &run_with(
        &[
            "rec",
            "import-raw",
            "--rom",
            rom,
            "--vram",
            &vram,
            "--cgram",
            &cgram,
        ],
        &["--oam", &oam, "--ppu", &ppu, "--out", raw],
    );
    out += &run(&["rec", "info", raw]);
    out += &run_with(&["render", "bg", "--rec", raw], &["--bg", "1"]);
    // Mode 7: BGMODE 7 in the same registers, and VRAM whose map cell
    // (c, r) is tile (c + r) & $FF with tile t's pixels all t.
    let mut ppu7 = std::fs::read(&ppu).unwrap();
    ppu7[5] = 7;
    let mut vram7 = vec![0u8; 0x10000];
    for w in 0..0x4000usize {
        vram7[w * 2] = ((w % 128 + w / 128) & 0xFF) as u8;
        vram7[w * 2 + 1] = (w / 64) as u8;
    }
    let (ppu7_path, vram7_path, rec7) = (arg("ppu7"), arg("vram7"), arg("mode7"));
    std::fs::write(&ppu7_path, ppu7).unwrap();
    std::fs::write(&vram7_path, vram7).unwrap();
    out += &run_with(
        &["rec", "import-raw", "--rom", rom, "--vram", &vram7_path],
        &["--cgram", &cgram, "--ppu", &ppu7_path, "--out", &rec7],
    );
    out += &run_with(&["render", "bg", "--rec", &rec7], &["--bg", "1"]);
    out += &run_with(&["render", "bg", "--rec", &rec7], &["--bg", "2"]);
    check("rec-graphics", &redact_tmp(&dir, &out));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mesen_recorder_commands() {
    use romlens_core::recording::mesen::{RECORDER_SCRIPT, stream::encode};
    let dir = temp_dir("mesen");
    let rom = write_fixture(&dir, MappingMode::LoRom);
    let bytes = std::fs::read(&rom).unwrap();
    let rom = rom.to_str().unwrap();
    let other = write_fixture(&dir, MappingMode::HiRom);
    let other = other.to_str().unwrap();
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let (clean, cut, rec) = (
        path("clean.rlstream"),
        path("cut.rlstream"),
        path("r.romrec"),
    );
    std::fs::write(&clean, encode::fixture(&bytes, 12, true)).unwrap();
    let sound = path("sound.rlstream");
    std::fs::write(&sound, encode::fixture_with_audio(&bytes, 5)).unwrap();
    let mut short = encode::fixture(&bytes, 12, false);
    short.truncate(short.len() - 100);
    std::fs::write(&cut, short).unwrap();

    let mut out = run_with(&["rec", "pack", &clean, "--rom", rom], &["--out", &rec]);
    out += &run(&["rec", "info", &rec, "--rom", rom]);
    out += &run_with(&["rec", "pack", &sound, "--rom", rom], &["--out", &rec]);
    out += &run(&["rec", "info", &rec]);
    out += &run_with(&["rec", "pack", &clean, "--rom", rom], &["--out", &rec]);
    out += &run_with(
        &["rec", "extract", &rec, "--frame", "11", "--region", "cpu"],
        &["--hex"],
    );
    out += &run_with(
        &["rec", "pack", &cut, "--rom", rom, "--out", &rec],
        &["--wram", "off"],
    );
    out += &run(&["rec", "info", &rec]);
    out += &run_with(&["rec", "pack", &clean, "--rom", other], &["--out", &rec]);
    // Refer to it from a project, and see that it is noticed when replaced.
    let pkg = path("P.romlens");
    out += &redact_tmp(&dir, &run(&["project", &pkg, "init", "--rom", rom]));
    out += &redact_tmp(
        &dir,
        &run(&["project", &pkg, "recordings", "add", &rec, "--rom", rom]),
    );
    out += &redact_tmp(&dir, &run(&["project", &pkg, "recordings", "--rom", rom]));
    out += &run(&[
        "rec", "pack", &clean, "--rom", rom, "--out", &rec, "--wram", "full",
    ])
    .replace(&*dir.to_string_lossy(), "<tmp>");
    out += &redact_tmp(
        &dir,
        &run(&["project", &pkg, "recordings", "list", "--rom", rom]),
    );
    out += &redact_tmp(
        &dir,
        &run(&["project", &pkg, "recordings", "remove", &rec, "--rom", rom]),
    );
    out += &redact_tmp(
        &dir,
        &run(&["project", &pkg, "recordings", "remove", &rec, "--rom", rom]),
    );
    let script = path("mesen_recorder.lua");
    out += &run(&["rec", "script", "--out", &script]);
    assert_eq!(std::fs::read_to_string(&script).unwrap(), RECORDER_SCRIPT);
    check("rec-mesen", &redact_tmp(&dir, &out));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn testrom_round_trips() {
    let dir = temp_dir("testrom");
    let out = dir.join("t.sfc");
    let out = out.to_str().unwrap();
    let wrote = run(&["testrom", "--out", out, "--mapping", "hirom"]);
    assert!(wrote.ends_with("HiROM test ROM\n"), "{wrote:?}");
    assert_eq!(std::fs::read(out).unwrap(), fixtures::minimal_hirom());
    let info = run(&["info", out]);
    assert!(info.contains("Mapping:         HiROM"), "{info}");
    assert!(
        info.contains("File:            t.sfc"),
        "output names files, never paths"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn version_prints_crate_and_api() {
    let v = run(&["--version"]);
    assert!(v.contains("(core API "), "{v:?}");
}

#[test]
fn dev_rom_golden() {
    let Some(dir) = std::env::var_os("ROMLENS_ROM_DIR") else {
        eprintln!("skipped: ROMLENS_ROM_DIR unset");
        return;
    };
    let rom = PathBuf::from(dir).join("SuperMetroid.F8DF.sfc");
    if !rom.exists() {
        eprintln!("skipped: development ROM not present");
        return;
    }
    let rom = rom.to_str().unwrap();
    check("info-supermetroid", &run(&["info", rom]));
    check(
        "hex-supermetroid",
        &format!(
            "{}{}",
            run(&["hex", rom, "--from", "$80:841C", "--rows", "4"]),
            run(&["hex", rom, "--from", "0x7FC0", "--rows", "4"])
        ),
    );
    check("resolve-supermetroid", &run(&["resolve", rom, "$80:841C"]));
    // The docs/04 boot listing.
    check(
        "disasm-supermetroid",
        &run(&[
            "disasm",
            rom,
            "--from",
            "$80:841C",
            "--count",
            "34",
            "--address",
            "snes",
            "--verbose",
        ]),
    );
    check("analyze-supermetroid", &run(&["analyze", rom, "--stats"]));
    // Without table resolution, to attribute the gain (`16-phase2-plan.md`).
    check(
        "analyze-supermetroid-no-tables",
        &run(&["analyze", rom, "--stats", "--no-tables"]),
    );
    check(
        "tables-supermetroid",
        &run(&["tables", rom, "--unresolved"]),
    );
    check(
        "warnings-supermetroid",
        &run(&["analyze", rom, "--warnings"]),
    );
    check(
        "labels-supermetroid",
        &run(&["labels", rom, "--count", "40"]),
    );
    check("xrefs-supermetroid", &run(&["xrefs", rom, "$80:8573"]));
    check("inspect-supermetroid", &run(&["inspect", rom, "$80:8427"]));
}

/// The routines fixture's graphs (docs/19): blocks and calls, as text, DOT
/// and JSON with a layout.
#[test]
fn graph_commands() {
    let dir = temp_dir("graph");
    let rom = dir.join("routines.sfc");
    std::fs::write(&rom, fixtures::routines_lorom()).unwrap();
    let rom = rom.to_str().unwrap();
    let mut text = String::new();
    for at in ["$00:8000", "$00:8020", "$00:8030", "$00:8040", "$00:8050"] {
        text.push_str(&run(&["graph", rom, at]));
        text.push_str(&run(&["graph", rom, at, "--calls"]));
    }
    check("graph-routines", &text);
    // The Atlas's queries (docs/22, A1): a window of the map, the calls
    // between its columns, and its items.
    check(
        "map-routines-window",
        &format!(
            "{}{}",
            run(&[
                "map",
                rom,
                "--from",
                "$00:8000",
                "--len",
                "0x80",
                "--buckets",
                "16",
                "--width",
                "16",
                "--arcs",
                "--items",
                "8"
            ]),
            run(&[
                "map",
                rom,
                "--from",
                "0x0",
                "--len",
                "0x60",
                "--buckets",
                "4",
                "--arcs",
                "--items",
                "3",
                "--json"
            ]),
        ),
    );
    check(
        "graph-routines-dot",
        &format!(
            "{}{}",
            run(&["graph", rom, "$00:8020", "--dot"]),
            run(&["graph", rom, "$00:8000", "--calls", "--dot"]),
        ),
    );
    check(
        "graph-routines-json",
        &format!(
            "{}{}",
            run(&["graph", rom, "$00:8040", "--json"]),
            run(&["graph", rom, "$00:8020", "--calls", "--json"]),
        ),
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The explain fixture's hardware stores (docs/20): one explained, the
/// routine's all, and as JSON.
#[test]
fn explain_commands() {
    let dir = temp_dir("explain");
    let rom = dir.join("explain.sfc");
    std::fs::write(&rom, fixtures::explain_lorom()).unwrap();
    let rom = rom.to_str().unwrap();
    check(
        "explain",
        &format!(
            "{}{}{}{}{}",
            run(&["explain", rom, "$00:8053"]),
            run(&["explain", rom, "$00:8014"]),
            run(&["explain", rom, "$00:804E"]),
            run(&["explain", rom, "$00:8000"]),
            run(&["explain", rom, "--stats"]),
        ),
    );
    check(
        "explain-routine",
        &run(&["explain", rom, "$00:8000", "--routine"]),
    );
    check(
        "explain-json",
        &run(&["explain", rom, "$00:8009", "--json"]),
    );
    check("screen", &run(&["screen", rom, "$00:8056"]));
    check(
        "decompile-numbers",
        &format!(
            "{}{}{}",
            run(&["decompile", rom, "$00:8000", "--numbers", "hex"]),
            run(&["decompile", rom, "$00:8000", "--numbers", "decimal"]),
            run(&["decompile", rom, "$00:8000", "--numbers", "binary"]),
        ),
    );
    check(
        "explain-disasm",
        &run(&[
            "disasm",
            rom,
            "--from",
            "$00:8000",
            "--count",
            "90",
            "--address",
            "snes",
            "--explain",
        ]),
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Pseudo-C for the routines fixture (docs/18), at each level, and the
/// header every result includes.
#[test]
fn decompile_commands() {
    let dir = temp_dir("decompile");
    let rom = dir.join("routines.sfc");
    std::fs::write(&rom, fixtures::routines_lorom()).unwrap();
    let rom = rom.to_str().unwrap();
    for level in ["lift", "clean", "full"] {
        let mut all = String::new();
        for at in ["$00:8000", "$00:8020", "$00:8030", "$00:8040", "$00:8050"] {
            all.push_str(&run(&["decompile", rom, at, "--level", level]));
        }
        check(&format!("decompile-routines-{level}"), &all);
    }
    check(
        "decompile-routines-json",
        &run(&["decompile", rom, "$00:8040", "--level", "lift", "--json"]),
    );
    check(
        "decompile-errors",
        &format!(
            "{}{}",
            run(&["decompile", rom, "$00:8021"]),
            run(&["decompile", rom, "$7E:0000"]),
        ),
    );
    let header = dir.join("snes.h");
    let wrote = run(&["decompile", "--header", header.to_str().unwrap()]);
    assert!(wrote.starts_with("wrote "), "{wrote}");
    check(
        "decompile-snes-h",
        &std::fs::read_to_string(&header).unwrap(),
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Two versions of a ROM compared (docs/22, D1), on the fixture pair.
#[test]
fn diff_commands() {
    let dir = temp_dir("diff");
    let (a, b) = fixtures::diff_pair();
    let (pa, pb) = (dir.join("a.sfc"), dir.join("b.sfc"));
    std::fs::write(&pa, a).unwrap();
    std::fs::write(&pb, b).unwrap();
    let (pa, pb) = (pa.to_str().unwrap(), pb.to_str().unwrap());
    check(
        "diff-pair",
        &format!(
            "{}{}",
            run(&["diff", pa, pb, "--routines"]),
            run(&["diff", pa, pb, "--json"]),
        ),
    );
}

/// ca65's `.dbg`: the labels and the source lines, on the fixture cc65
/// built (`romlens-core/tests/data/ca65`), copied so its sources sit beside
/// it.
#[test]
fn dbg_import() {
    let dir = temp_dir("dbg");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../romlens-core/tests/data/ca65");
    for name in [
        "fixture.sfc",
        "fixture.dbg",
        "main.s",
        "macros.inc",
        "palette.s",
    ] {
        std::fs::copy(src.join(name), dir.join(name)).unwrap();
    }
    let rom = dir.join("fixture.sfc");
    let rom = rom.to_str().unwrap();
    let dbg = dir.join("fixture.dbg");
    let dbg = dbg.to_str().unwrap();
    let pkg = dir.join("Fixture.romlens");
    let pkg = pkg.to_str().unwrap();
    let mut log = redact_tmp(&dir, &run(&["project", pkg, "init", "--rom", rom]));
    log += &run(&["import", "dbg", pkg, "--rom", rom, dbg]);
    log += &redact_tmp(&dir, &run(&["source", rom, "--project", pkg]));
    log += &run(&["source", rom, "--project", pkg, "$00:800C"]);
    log += &run(&["source", rom, "--project", pkg, "--line", "palette.s:15"]);
    log += &run(&["source", rom, "--project", pkg, "--line", "main.s:15"]);
    log += &run(&["labels", rom, "--project", pkg]);
    check("dbg-import", &log);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn spc_disasm() {
    let dir = temp_dir("spc");
    let program = romlens_core::spc700::assemble(
        "
        .org $0200
        start:  MOV X,#$EF
                MOV SP,X
                MOV T0DIV,#$50
                MOV CONTROL,#$01    ; timer 0 on
                MOV DSPADDR,#$5D
                MOV DSPDATA,#$3C
                CALL !keyon
        main:   MOV A,CPUIO0
                CBNE CPUIO0,main
                MOV CPUIO0,A
        tick:   MOV Y,T0OUT
                BEQ tick
                MOV X,A
                JMP [!table+X]
        keyon:  MOV A,#$4C
                MOV Y,#$01
                MOVW DSPADDR,YA
                RET
        table:  .dw main, keyon
        ",
    )
    .unwrap();
    let (at, bytes) = &program.chunks[0];
    assert_eq!(*at, 0x0200);
    let path = dir.join("driver.bin");
    std::fs::write(&path, bytes).unwrap();
    let path = path.to_str().unwrap();
    let mut log = run(&["spc", "disasm", path, "--base", "$0200", "--count", "6"]);
    log += &run(&["spc", "disasm", path, "--base", "$0200", "--walk"]);
    log += &run(&["spc", "disasm", path, "--base", "$FFF0"]);
    check("spc-disasm", &log);
    let mut log = run(&["registers", "--dsp"]);
    log += &run(&["registers", "--dsp", "kon", "--value", "$81"]);
    log += &run(&["registers", "--dsp", "$25", "--value", "$8F"]);
    log += &run(&["registers", "--dsp", "$6C", "--value", "$E0"]);
    log += &run(&["registers", "--spc"]);
    log += &run(&["registers", "--spc", "CONTROL", "--value", "$B0"]);
    log += &run(&["registers", "--spc", "$F2", "--value", "$4C"]);
    log += &run(&["registers", "--dsp", "$80"]);
    check("registers-sound", &log);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn brr() {
    let dir = temp_dir("brr");
    let path = dir.join("sound.sfc");
    std::fs::write(&path, fixtures::sound_lorom()).unwrap();
    let rom = path.to_str().unwrap();
    let at = format!("0x{:X}", fixtures::sound::SAMPLE_OFFSET);
    let lp = format!(
        "0x{:X}",
        fixtures::sound::SAMPLE_OFFSET + fixtures::sound::SAMPLE_LOOP
    );
    let mut log = run(&["brr", rom, &at, "--loop", &lp, "--ascii"]);
    log += &run(&["brr", rom, &at, "--blocks", "--max", "2"]);
    check("brr", &log);
    let _ = std::fs::remove_dir_all(dir);
}
