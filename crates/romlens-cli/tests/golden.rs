//! CLI output pinned per fixture. Line endings are normalised so the same
//! files pass on Windows. `UPDATE_GOLDEN=1` rewrites them.

use std::path::{Path, PathBuf};
use std::process::Command;

use romlens_core::{MappingMode, fixtures};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("romlens-golden-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fixture(dir: &Path, mode: MappingMode) -> PathBuf {
    let path = dir.join(fixtures::file_name(mode));
    std::fs::write(&path, fixtures::for_mapping(mode)).unwrap();
    path
}

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
    }
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
}
