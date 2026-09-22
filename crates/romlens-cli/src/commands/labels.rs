//! `labels`: every visible label, filtered.

use std::path::Path;

use anyhow::Result;
use romlens_core::model::{LabelSource, Symbols};
use romlens_core::{AddressExpr, SnesAddress};

use crate::commands::session::{self, any_address};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SourceFilter {
    Auto,
    User,
    Imported,
    All,
}

pub struct LabelsArgs<'a> {
    pub rom: &'a Path,
    pub project: Option<&'a Path>,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub source: SourceFilter,
    pub count: Option<u32>,
}

fn address_of(rom: &romlens_core::RomImage, expr: &str) -> Result<SnesAddress> {
    Ok(match any_address(expr)? {
        AddressExpr::Snes(a) => a,
        AddressExpr::File(off) => rom
            .resolve(&format!("0x{:X}", off.0))?
            .snes_address
            .unwrap_or(SnesAddress::from_u24(0)),
    })
}

pub fn run(args: LabelsArgs<'_>) -> Result<()> {
    let s = session::open(args.rom, args.project, false)?;
    let symbols = Symbols::new(&s.rom, &s.project, &s.snap.auto_labels);
    let from = args.from.map(|e| address_of(&s.rom, e)).transpose()?;
    let to = args.to.map(|e| address_of(&s.rom, e)).transpose()?;
    let mut n = 0;
    for l in symbols.all_labels() {
        if from.is_some_and(|f| l.address < f) || to.is_some_and(|t| l.address > t) {
            continue;
        }
        let keep = matches!(
            (&l.source, args.source),
            (_, SourceFilter::All)
                | (LabelSource::Auto, SourceFilter::Auto)
                | (LabelSource::User, SourceFilter::User)
                | (LabelSource::Imported(_), SourceFilter::Imported)
        );
        if !keep {
            continue;
        }
        let source = match &l.source {
            LabelSource::Auto => "auto".to_owned(),
            LabelSource::User => "user".to_owned(),
            LabelSource::Imported(s) => format!("imported:{s}"),
            LabelSource::Builtin => "builtin".to_owned(),
        };
        let off = s
            .rom
            .file_offset_for(l.address)
            .map_or("        ".to_owned(), |o| o.to_string());
        println!("{}  {}  {:<24} {}", off, l.address, l.name, source);
        n += 1;
        if args.count.is_some_and(|c| n >= c) {
            break;
        }
    }
    Ok(())
}
