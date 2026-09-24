//! Automatic labels: a prefix that says what the analyzer believed plus the
//! six-digit canonical address.
//! Priority vector > SUB > CODE > PTR > JTBL > DATA.
//!
//! Code only branches reach is named for its shape: `LOOP_` where a branch
//! comes back to it from below (the top of a loop), `SKIP_` where every
//! branch to it goes forward (the code a test skips to). Code a `JMP`
//! reaches, and nothing branches back to, stays `CODE_`.

use std::collections::BTreeMap;

use crate::memory::address::SnesAddress;
use crate::model::label::{Label, LabelSource};
use crate::model::xref::{XRef, XRefKind};

/// Vector names in priority order.
pub const VECTOR_PRIORITY: [&str; 6] = ["RESET", "NMI", "IRQ", "COP", "BRK", "ABORT"];

fn rank(prefix: &str) -> u8 {
    match prefix {
        "RESET" => 0,
        "NMI" => 1,
        "IRQ" => 2,
        "COP" => 3,
        "BRK" => 4,
        "ABORT" => 5,
        "SUB" => 6,
        "CODE" | "LOOP" | "SKIP" => 7,
        "PTR" => 8,
        // Below PTR: a single `JMP (abs)` slot names its target more precisely
        // than a table base does. Above DATA: a dispatcher reading the base is
        // stronger evidence than an anonymous load.
        "JTBL" => 9,
        "DATA" => 10,
        _ => 11,
    }
}

/// Build the auto labels. `vectors` are `(name, canonical target)`; `xrefs`
/// must carry canonical targets; only in-ROM targets are labelled.
pub fn build(
    vectors: &[(&'static str, SnesAddress)],
    xrefs: &[XRef],
    labelable: impl Fn(&XRef) -> bool,
) -> BTreeMap<SnesAddress, Label> {
    let mut best: BTreeMap<SnesAddress, &'static str> = BTreeMap::new();
    // For code targets: whether a branch comes back to it, and whether
    // anything but a forward branch reaches it.
    let mut shape: BTreeMap<SnesAddress, (bool, bool)> = BTreeMap::new();
    let mut offer = |addr: SnesAddress, prefix: &'static str| {
        let e = best.entry(addr).or_insert(prefix);
        if rank(prefix) < rank(e) {
            *e = prefix;
        }
    };
    for (name, addr) in vectors {
        offer(*addr, name);
    }
    for x in xrefs {
        if x.to_offset.is_none() || !labelable(x) {
            continue;
        }
        let prefix = match x.kind {
            XRefKind::Call => "SUB",
            XRefKind::Jump | XRefKind::Branch => "CODE",
            XRefKind::Pointer => "PTR",
            XRefKind::JumpTable => "JTBL",
            XRefKind::Read | XRefKind::Write | XRefKind::ReadWrite => "DATA",
            XRefKind::Vector => continue,
        };
        if prefix == "CODE" {
            let back = x.to_offset.is_some_and(|to| x.from.0 >= to.0);
            let s = shape.entry(x.to).or_default();
            if x.kind == XRefKind::Branch && back {
                s.0 = true;
            } else if x.kind != XRefKind::Branch || back {
                s.1 = true;
            }
        }
        offer(x.to, prefix);
    }
    best.into_iter()
        .map(|(addr, prefix)| {
            let prefix = match (prefix, shape.get(&addr)) {
                ("CODE", Some((true, _))) => "LOOP",
                ("CODE", Some((false, false))) => "SKIP",
                (p, _) => p,
            };
            (
                addr,
                Label {
                    address: addr,
                    name: format!("{prefix}_{:06X}", addr.as_u24()),
                    source: LabelSource::Auto,
                },
            )
        })
        .collect()
}
