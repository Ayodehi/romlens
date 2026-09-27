use romlens_core::recording::RomrecSource;
#[test]
#[ignore]
fn ff6_lines() {
    let p = std::path::Path::new(
        "/private/tmp/claude-501/-Users-benjaminbarbour-Projects-snes-visualizer/64ef85fa-1b9f-40da-bb5a-04d346f83409/scratchpad/ff6/b.romrec",
    );
    let rec = RomrecSource::open(p).unwrap();
    let f: u64 = std::env::var("F")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1963);
    let states = romlens_core::recording::lines::frame_line_states(&rec, f, 224)
        .unwrap()
        .expect("line writes");
    let mut last = String::new();
    for (y, s) in states.iter().enumerate() {
        let now = format!(
            "TM {:02X} TS {:02X} INIDISP {:02X} CGWSEL {:02X} CGADSUB {:02X} COLDATA {:04X} W12SEL {:02X} WOBJSEL {:02X} WH0 {:02X} WH1 {:02X}",
            s.register(0x212C),
            s.register(0x212D),
            s.register(0x2100),
            s.register(0x2130),
            s.register(0x2131),
            s.fixed_colour(),
            s.register(0x2123),
            s.register(0x2125),
            s.register(0x2126),
            s.register(0x2127)
        );
        if now != last {
            eprintln!("line {y:3}: {now}");
            last = now;
        }
    }
}

#[test]
#[ignore]
fn ff6_gradient() {
    use romlens_core::graphics::compose::{ComposeOptions, compose_lines_with};
    let p = std::path::Path::new(
        "/private/tmp/claude-501/-Users-benjaminbarbour-Projects-snes-visualizer/64ef85fa-1b9f-40da-bb5a-04d346f83409/scratchpad/ff6/b.romrec",
    );
    let rec = RomrecSource::open(p).unwrap();
    for (name, o) in [
        ("plain", ComposeOptions::alone(1)),
        ("math", ComposeOptions::alone_with_colour_math(1)),
    ] {
        let mut r = romlens_core::recording::lines::frame_replay(&rec, 1963)
            .unwrap()
            .unwrap();
        let f = compose_lines_with(&mut r, o);
        let ys = [10u32, 25, 42, 60, 75];
        let px: Vec<_> = ys.iter().map(|&y| f.bitmap.get(128, y)).collect();
        eprintln!("GRAD {name}: {px:?}");
    }
}
