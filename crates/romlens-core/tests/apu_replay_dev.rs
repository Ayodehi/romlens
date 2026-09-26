//! Romlens's SPC700 run beside a real recording (docs/23, A7): point
//! `ROMLENS_SPC_REC` at a `.romrec` with a sound side (and optionally
//! `ROMLENS_SPC_REC_FRAMES=A..B`, the frames after the boot ROM has handed
//! over to the driver) and every frame's I/O writes must match Mesen's on
//! the cycle. Without it, this says so and passes.

use romlens_core::apu::replay::run_free;
use romlens_core::recording::{MachineStateSource, RomrecSource};

#[test]
fn a_free_run_keeps_step_with_the_recording() {
    let Some(path) = std::env::var_os("ROMLENS_SPC_REC") else {
        eprintln!("ROMLENS_SPC_REC is not set: the replay against a recording was not run");
        return;
    };
    let rec = RomrecSource::open(std::path::Path::new(&path)).unwrap();
    let last = rec.frame_count().unwrap() - 1;
    let (from, to) = match std::env::var("ROMLENS_SPC_REC_FRAMES") {
        Ok(r) => {
            let (a, b) = r.split_once("..").unwrap();
            (a.parse().unwrap(), b.parse().unwrap())
        }
        Err(_) => (0, last),
    };
    let checks = run_free(&rec, from, to).unwrap();
    let bad: Vec<_> = checks
        .iter()
        .filter(|c| !c.matches() && !c.differs_only_at_the_end())
        .collect();
    assert!(
        bad.is_empty(),
        "{} of {} frames differ, the first {:?}",
        bad.len(),
        checks.len(),
        bad[0]
    );
}
