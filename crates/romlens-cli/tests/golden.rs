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
            "{}{}",
            run(&["registers", "$420D"]),
            run(&["registers", "$2145"])
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
