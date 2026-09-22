//! Automatic labels: a prefix that says what the analyzer believed plus the
//! six-digit canonical address. Priority vector > SUB > CODE > PTR > DATA.

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
        "CODE" => 7,
        "PTR" => 8,
        "DATA" => 9,
        _ => 10,
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
            XRefKind::Read | XRefKind::Write | XRefKind::ReadWrite => "DATA",
            XRefKind::Vector => continue,
        };
        offer(x.to, prefix);
    }
    best.into_iter()
        .map(|(addr, prefix)| {
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
