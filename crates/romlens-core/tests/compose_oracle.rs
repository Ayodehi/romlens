//! Development aids for the compositor (docs/22, P1), never run by default
//! and never in CI: each reads a local recording named by `ORACLE_REC`
//! (from `scripts/oracle/run.sh`) and prints what it found, or writes a
//! composed frame as raw RGBA to a scratch path to look at beside the
//! emulator's own screen. Nothing leaves the developer's machine.

use romlens_core::graphics::compose::{Lines, compose};
use romlens_core::recording::MachineStateSource;
use romlens_core::recording::reader::RomrecSource;

#[test]
#[ignore]
fn write_composed_frame() {
    let (Ok(rec), Ok(frame), Ok(out)) = (
        std::env::var("ORACLE_REC"),
        std::env::var("ORACLE_FRAME"),
        std::env::var("ORACLE_OUT"),
    ) else {
        return;
    };
    let src = RomrecSource::open(std::path::Path::new(&rec)).unwrap();
    let s = src.state_at(frame.parse().unwrap()).unwrap();
    let ppu = s.ppu().unwrap();
    let f =
        match romlens_core::recording::lines::frame_replay(&src, frame.parse().unwrap()).unwrap() {
            Some(mut r) => romlens_core::graphics::compose::compose_lines(&mut r),
            None => compose(
                s.vram().unwrap(),
                s.cgram().unwrap(),
                s.oam().unwrap(),
                Lines::Frame(&ppu),
            ),
        };
    let mut bytes = Vec::new();
    bytes.extend((f.bitmap.width as u16).to_le_bytes());
    bytes.extend((f.bitmap.height as u16).to_le_bytes());
    for px in f.bitmap.rgba.chunks(4) {
        // As the Mesen dump: ARGB little-endian.
        let v = 0xFF00_0000u32 | (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        bytes.extend(v.to_le_bytes());
    }
    std::fs::write(out, bytes).unwrap();
}

/// The replayed registers at the last line against the frame's own
/// snapshot: where they differ, the replay is wrong.
#[test]
#[ignore]
fn replay_matches_the_snapshot() {
    let (Ok(rec), Ok(frame)) = (std::env::var("ORACLE_REC"), std::env::var("ORACLE_FRAME")) else {
        return;
    };
    let frame: u64 = frame.parse().unwrap();
    let src = RomrecSource::open(std::path::Path::new(&rec)).unwrap();
    let lines = romlens_core::recording::lines::frame_line_states(&src, frame, 224)
        .unwrap()
        .unwrap();
    let end = src.state_at(frame).unwrap().ppu().unwrap();
    let last = lines.last().unwrap();
    let writes = src.line_writes(frame).unwrap().unwrap();
    println!(
        "{} writes; mode {} / {}",
        writes.len(),
        last.bg_mode(),
        end.bg_mode()
    );
    for bg in 1..=4 {
        for v in [false, true] {
            println!(
                "scroll bg{bg} {v}: replay {:#06x} snapshot {:#06x}",
                last.scroll(bg, v),
                end.scroll(bg, v)
            );
        }
    }
    for i in 0..6 {
        println!(
            "m7[{i}]: replay {:#06x} snapshot {:#06x}",
            last.mode7(i),
            end.mode7(i)
        );
    }
    let first = &lines[0];
    println!(
        "line 0: m7 {:?} scroll1 {:#x},{:#x}",
        (0..6).map(|i| first.mode7(i)).collect::<Vec<_>>(),
        first.scroll(1, false),
        first.scroll(1, true)
    );
    for w in writes
        .iter()
        .filter(|w| {
            std::env::var("ORACLE_REGS").map_or(true, |r| {
                r.split(',')
                    .any(|x| u8::from_str_radix(x, 16).ok() == Some(w.reg))
            })
        })
        .take(80)
    {
        println!(
            "  {:>4} {:>3} ${:04X} = {:02X}",
            w.line,
            w.dot,
            0x2100 + w.reg as u16,
            w.value
        );
    }
}

/// The colour-math and window registers of one line.
#[test]
#[ignore]
fn line_registers() {
    let (Ok(rec), Ok(frame), Ok(line)) = (
        std::env::var("ORACLE_REC"),
        std::env::var("ORACLE_FRAME"),
        std::env::var("ORACLE_LINE"),
    ) else {
        return;
    };
    let src = RomrecSource::open(std::path::Path::new(&rec)).unwrap();
    let lines =
        romlens_core::recording::lines::frame_line_states(&src, frame.parse().unwrap(), 224)
            .unwrap()
            .unwrap();
    let p = &lines[line.parse::<usize>().unwrap()];
    for r in [
        0x2100u16, 0x2105, 0x2123, 0x2124, 0x2125, 0x2126, 0x2127, 0x2128, 0x2129, 0x212A, 0x212B,
        0x212C, 0x212D, 0x212E, 0x212F, 0x2130, 0x2131,
    ] {
        print!("${r:04X}={:02X} ", p.register(r));
    }
    println!("fixed={:#06x}", p.fixed_colour());
}

/// The sprites in range of one line, in OAM order.
#[test]
#[ignore]
fn line_sprites() {
    let (Ok(rec), Ok(frame), Ok(line)) = (
        std::env::var("ORACLE_REC"),
        std::env::var("ORACLE_FRAME"),
        std::env::var("ORACLE_LINE"),
    ) else {
        return;
    };
    let src = RomrecSource::open(std::path::Path::new(&rec)).unwrap();
    let s = src.state_at(frame.parse().unwrap()).unwrap();
    let lines =
        romlens_core::recording::lines::frame_line_states(&src, frame.parse().unwrap(), 224)
            .unwrap()
            .unwrap();
    let y: u32 = line.parse().unwrap();
    let p = &lines[y as usize];
    let obsel = p.obj_select();
    println!(
        "OBSEL {:02X} OAMADD {:02X}{:02X}",
        p.register(0x2101),
        p.register(0x2103),
        p.register(0x2102)
    );
    let mut tiles = 0;
    for e in romlens_core::graphics::oam::decode_oam(s.oam().unwrap()) {
        let (w, h) = obsel.size_of(e.large);
        if (y.wrapping_sub(e.y as u32) & 0xFF) < h as u32 {
            tiles += w as u32 / 8;
            println!(
                "  sprite {:3} x {:4} y {:3} {}x{} tile {:03X} pal {} pri {}",
                e.index, e.x, e.y, w, h, e.tile, e.palette, e.priority
            );
        }
    }
    println!("tiles {tiles}");
}

/// What drew each pixel that differs from a Mesen dump, tallied.
#[test]
#[ignore]
fn diff_winners() {
    use romlens_core::graphics::compose::Winner;
    let (Ok(rec), Ok(frame), Ok(shot)) = (
        std::env::var("ORACLE_REC"),
        std::env::var("ORACLE_FRAME"),
        std::env::var("ORACLE_SHOT"),
    ) else {
        return;
    };
    let frame: u64 = frame.parse().unwrap();
    let src = RomrecSource::open(std::path::Path::new(&rec)).unwrap();
    let f = romlens_core::graphics::compose::compose_lines(
        &mut romlens_core::recording::lines::frame_replay(&src, frame)
            .unwrap()
            .unwrap(),
    );
    let b = std::fs::read(shot).unwrap();
    let mut tally: std::collections::BTreeMap<String, (u32, (u32, u32))> = Default::default();
    for y in 0..224u32 {
        for x in 0..256u32 {
            let i = 4 + (((y + 7) * 256 + x) * 4) as usize;
            let v = u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
            let theirs = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
            let ours = f.bitmap.get(x, y);
            if ours[..3] != theirs[..] {
                let key = match f.winner(x, y).unwrap() {
                    Winner::Bg(p) => format!("BG{} pri {}", p.layer, p.entry.priority),
                    Winner::Sprite(p) => {
                        format!("sprite {} pri {} colour {}", p.sprite, p.priority, p.colour)
                    }
                    w => format!("{w:?}"),
                };
                let e = tally.entry(key).or_insert((0, (x, y)));
                e.0 += 1;
            }
        }
    }
    for (k, (n, at)) in tally {
        println!("{n:6}  {k}  first at {at:?}");
    }
}

/// Replaying a frame's writes over the previous frame's end must arrive at
/// the frame's own snapshot, memories included: every frame of a recording.
#[test]
#[ignore]
fn replays_arrive_at_the_snapshots() {
    let Ok(rec) = std::env::var("ORACLE_REC") else {
        return;
    };
    let src = RomrecSource::open(std::path::Path::new(&rec)).unwrap();
    let n = src.frame_count().unwrap();
    let mut bad = 0;
    for f in 1..n {
        let Some(r) = romlens_core::recording::lines::frame_replay(&src, f).unwrap() else {
            continue;
        };
        let (_, vram, cgram, oam) = r.finish();
        let s = src.state_at(f).unwrap();
        let diff = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x != y).count();
        let (dv, dc, doam) = (
            diff(&vram, s.vram().unwrap()),
            diff(&cgram, s.cgram().unwrap()),
            diff(&oam, s.oam().unwrap()),
        );
        if dv + dc + doam > 0 {
            bad += 1;
            if bad <= 10 {
                println!("frame {f}: VRAM {dv} CGRAM {dc} OAM {doam} bytes differ");
            }
        }
    }
    println!("{bad} of {} frames differ", n - 1);
}
