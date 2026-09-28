//! `export asm --check` against the real asar, on every fixture ROM. Skipped
//! (with a note) where asar is not on the PATH, as in CI.

use std::process::Command;

fn romlens(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_romlens"))
        .args(args)
        .output()
        .expect("romlens runs")
}

#[test]
fn every_fixture_reassembles_byte_exact_under_asar() {
    if Command::new("asar").arg("--version").output().is_err() {
        eprintln!("asar is not on the PATH: skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("romlens-asar-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut roms: Vec<(String, Vec<&str>)> = ["lorom", "hirom", "exhirom"]
        .iter()
        .map(|m| (format!("minimal-{m}"), vec!["--mapping", m]))
        .collect();
    for f in [
        "all-opcodes",
        "dispatch",
        "mixed-data",
        "graphics",
        "routines",
        "explain",
        "sound",
    ] {
        roms.push((f.to_owned(), vec!["--fixture", f]));
    }
    for (name, how) in roms {
        let rom = dir.join(format!("{name}.sfc"));
        let rom = rom.to_str().unwrap();
        let made = romlens(&[&["testrom", "--out", rom][..], &how].concat());
        assert!(
            made.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&made.stderr)
        );
        let asm = dir.join(format!("{name}.asm"));
        let out = romlens(&[
            "export",
            "asm",
            rom,
            "--out",
            asm.to_str().unwrap(),
            "--check",
        ]);
        let said = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && said.contains("byte-exact"),
            "{name}: {said}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
