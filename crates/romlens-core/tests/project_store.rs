//! Exact JSON goldens, round trip, version and format errors, the package
//! writer and the ROM locator.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use romlens_core::fixtures;
use romlens_core::io::{
    PROJECT_FILES, PROJECT_VERSION, from_files, locate_rom, read_identity, read_package, to_files,
    write_package,
};
use romlens_core::model::{
    Command, CommentKind, DataKind, FlagOverride, Label, LabelSource, OverrideKind, Project,
    RomIdentity,
};
use romlens_core::{AddressStyle, FileOffset, ProjectError, RomImage, SnesAddress};

fn rom() -> RomImage {
    RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap()
}

fn sample(rom: &RomImage) -> Project {
    let mut p = Project::new(rom);
    let a = |b: u8, o: u16| SnesAddress::new(b, o);
    p.apply(
        rom,
        Command::SetLabel {
            address: a(0x00, 0x8000),
            name: Some("Boot".into()),
        },
    )
    .unwrap();
    p.labels.insert(
        a(0x7E, 0x0A1C),
        Label {
            address: a(0x7E, 0x0A1C),
            name: "SamusPose".into(),
            source: LabelSource::Imported("pjboy".into()),
        },
    );
    p.apply(
        rom,
        Command::SetComment {
            address: a(0x00, 0x8000),
            kind: CommentKind::Line,
            text: Some("disable IRQ".into()),
        },
    )
    .unwrap();
    p.apply(
        rom,
        Command::SetComment {
            address: a(0x00, 0x8007),
            kind: CommentKind::Block,
            text: Some("Force blank\nthen spin".into()),
        },
    )
    .unwrap();
    p.apply(
        rom,
        Command::MarkRegion {
            start: FileOffset(0x1000),
            len: 512,
            kind: OverrideKind::Data(DataKind::Byte),
        },
    )
    .unwrap();
    p.apply(
        rom,
        Command::MarkRegion {
            start: FileOffset(0x1400),
            len: 64,
            kind: OverrideKind::Data(DataKind::Table { stride: 4 }),
        },
    )
    .unwrap();
    p.apply(
        rom,
        Command::MarkRegion {
            start: FileOffset(0x1800),
            len: 16,
            kind: OverrideKind::Data(DataKind::Graphics { bpp: 4 }),
        },
    )
    .unwrap();
    p.apply(
        rom,
        Command::MarkRegion {
            start: FileOffset(0x2000),
            len: 32,
            kind: OverrideKind::Unknown,
        },
    )
    .unwrap();
    p.apply(
        rom,
        Command::MarkRegion {
            start: FileOffset(0x3000),
            len: 1,
            kind: OverrideKind::Code,
        },
    )
    .unwrap();
    p.apply(
        rom,
        Command::SetFlagOverride {
            offset: FileOffset(0x1C41),
            flags: Some(FlagOverride {
                m: Some(true),
                ..Default::default()
            }),
        },
    )
    .unwrap();
    p.apply(
        rom,
        Command::SetFlagOverride {
            offset: FileOffset(0x2100),
            flags: Some(FlagOverride {
                m: Some(false),
                x: Some(false),
                e: Some(false),
                dbr: Some(0x80),
                dp: Some(0),
            }),
        },
    )
    .unwrap();
    p.settings.address_style = AddressStyle::Snes;
    p
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/project")
}

#[test]
fn files_match_the_goldens_and_round_trip() {
    let rom = rom();
    let project = sample(&rom);
    let files = to_files(&rom, &project);
    let mut expected_names: Vec<String> = PROJECT_FILES.iter().map(|s| s.to_string()).collect();
    expected_names.sort();
    assert_eq!(files.keys().cloned().collect::<Vec<_>>(), expected_names);
    std::fs::create_dir_all(golden_dir()).unwrap();
    for (name, bytes) in &files {
        let path = golden_dir().join(name);
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, bytes).unwrap();
            continue;
        }
        let expected = std::fs::read(&path)
            .unwrap_or_else(|_| panic!("missing golden {}; UPDATE_GOLDEN=1", path.display()));
        assert_eq!(
            String::from_utf8_lossy(bytes),
            String::from_utf8_lossy(&expected).replace("\r\n", "\n"),
            "{name} differs"
        );
    }
    let back = from_files(&rom, &files).unwrap();
    assert_eq!(back, project);
    let identity = read_identity(&files).unwrap();
    assert_eq!(identity, RomIdentity::of(&rom));
}

#[test]
fn errors() {
    let rom = rom();
    let mut files = to_files(&rom, &sample(&rom));
    let mut newer = files.clone();
    let text = String::from_utf8(files["project.json"].clone()).unwrap();
    let current = format!("\"version\": {PROJECT_VERSION}");
    assert!(
        text.contains(&current),
        "project.json stopped carrying a version"
    );
    newer.insert(
        "project.json".into(),
        text.replace(&current, "\"version\": 99").into_bytes(),
    );
    assert_eq!(
        from_files(&rom, &newer).unwrap_err(),
        ProjectError::NewerVersion(99)
    );
    let mut bad = files.clone();
    bad.insert(
        "project.json".into(),
        text.replace("romlens-project", "something-else")
            .into_bytes(),
    );
    assert!(matches!(
        from_files(&rom, &bad).unwrap_err(),
        ProjectError::BadFormat(_)
    ));
    let mut broken = files.clone();
    broken.insert("labels.json".into(), b"[{\"address\": 5}]".to_vec());
    assert!(matches!(
        from_files(&rom, &broken).unwrap_err(),
        ProjectError::Json { .. }
    ));
    files.remove("project.json");
    assert_eq!(
        from_files(&rom, &files).unwrap_err(),
        ProjectError::MissingFile("project.json".into())
    );
    // Unknown fields and files are ignored; missing optional files are empty.
    let mut minimal = BTreeMap::new();
    minimal.insert(
        "project.json".to_owned(),
        format!(
            "{{\"format\":\"romlens-project\",\"version\":1,\"future\":true,\"rom\":{{\"sha256\":\"{}\",\"size\":32768,\"mapping\":\"LoROM\",\"fastRom\":false,\"title\":\"X\"}}}}",
            rom.sha256_hex()
        )
        .into_bytes(),
    );
    minimal.insert("notes.txt".to_owned(), b"hello".to_vec());
    let p = from_files(&rom, &minimal).unwrap();
    assert!(p.labels.is_empty() && p.region_overrides.is_empty());
    assert_eq!(p.settings.address_style, AddressStyle::Both);
}

#[test]
fn package_write_read_and_locate() {
    let rom = rom();
    let base = std::env::temp_dir().join(format!("romlens-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let package = base.join("Test.romlens");
    let files = to_files(&rom, &sample(&rom));
    write_package(&package, &files).unwrap();
    // A second write replaces in place.
    write_package(&package, &files).unwrap();
    let read = read_package(&package).unwrap();
    assert_eq!(read, files);
    assert!(!package.join("project.json.tmp").exists());
    assert_eq!(from_files(&rom, &read).unwrap(), sample(&rom));

    let identity = RomIdentity::of(&rom);
    assert_eq!(
        locate_rom(&identity, &package, &[]),
        None,
        "nothing beside the package yet"
    );
    let wrong = base.join("other.sfc");
    std::fs::write(&wrong, fixtures::minimal_hirom()).unwrap();
    assert_eq!(
        locate_rom(&identity, &package, std::slice::from_ref(&wrong)),
        None,
        "hash mismatch is not accepted"
    );
    let right = base.join("test.SMC");
    std::fs::write(&right, fixtures::minimal_lorom()).unwrap();
    assert_eq!(locate_rom(&identity, &package, &[]), Some(right.clone()));
    let elsewhere = base.join("sub");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let hinted = elsewhere.join("hinted.sfc");
    std::fs::write(&hinted, fixtures::minimal_lorom()).unwrap();
    assert_eq!(
        locate_rom(&identity, &package, &[wrong, hinted.clone()]),
        Some(hinted)
    );
    assert!(matches!(
        read_package(&base.join("missing")).unwrap_err(),
        ProjectError::Io(_)
    ));
    let _ = std::fs::remove_dir_all(&base);
}

/// A package written by 0.2.0 still opens. The fixture is a byte-for-byte copy
/// of the v1 goldens, kept frozen: regenerating the goldens with
/// `UPDATE_GOLDEN=1` must never touch it, or the guarantee tests nothing.
#[test]
fn v1_package_still_opens() {
    let rom = rom();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/project-v1");
    let files = read_package(&dir).unwrap();
    let text = String::from_utf8(files["project.json"].clone()).unwrap();
    assert!(
        text.contains("\"version\": 1"),
        "the v1 fixture was regenerated and is no longer v1"
    );
    assert_eq!(from_files(&rom, &files).unwrap(), sample(&rom));
    assert_eq!(read_identity(&files).unwrap(), RomIdentity::of(&rom));
}

/// Traces and imports live in subdirectories of the package (2A.3), so the
/// writer creates them and the reader walks into them, keying files by a
/// `/`-separated path on every platform.
#[test]
fn packages_carry_subdirectories() {
    let rom = rom();
    let base = std::env::temp_dir().join(format!("romlens-subdirs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let package = base.join("Test.romlens");
    let mut files = to_files(&rom, &sample(&rom));
    files.insert("traces/play.cdl".to_owned(), vec![1, 2, 3, 4]);
    files.insert("imports/boot.sym".to_owned(), b"00:8000 Boot\n".to_vec());
    write_package(&package, &files).unwrap();
    assert!(package.join("traces").join("play.cdl").is_file());
    assert!(!package.join("traces").join("play.cdl.tmp").exists());
    assert_eq!(read_package(&package).unwrap(), files);
    // Rewriting replaces in place and leaves no stray directories behind.
    write_package(&package, &files).unwrap();
    assert_eq!(read_package(&package).unwrap(), files);
    // A name may not climb out of the package.
    let mut escaping = files.clone();
    escaping.insert("../escaped.json".to_owned(), b"no".to_vec());
    assert!(write_package(&package, &escaping).is_err());
    assert!(!base.join("escaped.json").exists());
    let _ = std::fs::remove_dir_all(&base);
}
