//! `tables`: the `JMP`/`JSR (abs,X)` dispatch tables the analyzer resolved,
//! and the dispatch sites it could not (checklist 2.1).

use std::path::Path;

use anyhow::Result;
use romlens_core::analysis::WarningKind;

use crate::commands::session;

pub fn run(
    rom: &Path,
    project: Option<&Path>,
    json: bool,
    entries: bool,
    unresolved: bool,
) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let tables = &s.snap.jump_tables;
    // A site that failed is still worth listing: the reason is what tells a
    // reader whether the table is unreadable or merely unmarked.
    let failed: Vec<_> = s
        .snap
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::ComputedJump)
        .collect();

    if json {
        println!("{{");
        println!("  \"tables\": [");
        let rows: Vec<String> = tables
            .iter()
            .map(|t| {
                format!(
                    "    {{\"base\": {}, \"baseAddress\": \"{}\", \"site\": {}, \
\"siteAddress\": \"{}\", \"call\": {}, \"entries\": {}, \"bytes\": {}, \
\"stop\": \"{}\", \"confidence\": {:.2}}}",
                    t.base,
                    t.base_address,
                    t.site,
                    t.site_address,
                    t.call,
                    t.targets.len(),
                    t.len(),
                    t.stop.name(),
                    t.confidence()
                )
            })
            .collect();
        println!("{}", rows.join(",\n"));
        println!("  ],");
        println!("  \"unresolved\": {}", failed.len());
        println!("}}");
        return Ok(());
    }

    println!(
        "{} jump {} resolved",
        tables.len(),
        if tables.len() == 1 { "table" } else { "tables" }
    );
    for t in tables {
        println!(
            "{}  {}  {:>3} entries  {:<4}  from {}  {:.2}  {}",
            romlens_core::FileOffset(t.base),
            t.base_address,
            t.targets.len(),
            if t.call { "JSR" } else { "JMP" },
            t.site_address,
            t.confidence(),
            t.stop.name()
        );
        if entries {
            for (i, (target, off)) in t.targets.iter().enumerate() {
                println!(
                    "    [{i:>3}]  {}  {}",
                    romlens_core::FileOffset(*off),
                    target
                );
            }
        }
    }
    if unresolved {
        println!();
        println!("{} dispatch {} unresolved", failed.len(), {
            if failed.len() == 1 { "site" } else { "sites" }
        });
        for w in failed {
            println!("{}  {}", w.offset, w.text);
        }
    }
    Ok(())
}
