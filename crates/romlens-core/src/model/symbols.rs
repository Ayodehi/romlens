//! Name lookup in the agreed order: user > imported > (hardware register,
//! handled by the formatter) > auto.

use std::collections::BTreeMap;

use crate::cpu65816::format::{Symbol, SymbolLookup};
use crate::memory::address::SnesAddress;
use crate::model::label::{Label, LabelSource};
use crate::model::project::Project;
use crate::rom::image::RomImage;

pub struct Symbols<'a> {
    pub rom: &'a RomImage,
    pub project: &'a Project,
    pub auto: &'a BTreeMap<SnesAddress, Label>,
}

impl<'a> Symbols<'a> {
    pub fn new(
        rom: &'a RomImage,
        project: &'a Project,
        auto: &'a BTreeMap<SnesAddress, Label>,
    ) -> Self {
        Self { rom, project, auto }
    }

    /// The label at a (canonicalised) address in lookup order.
    pub fn label_at(&self, address: SnesAddress) -> Option<&'a Label> {
        let addr = Project::canonical(self.rom, address);
        self.project
            .labels
            .get(&addr)
            .or_else(|| self.auto.get(&addr))
    }

    /// Every visible label, ascending by address; user labels shadow auto ones.
    pub fn all_labels(&self) -> Vec<&'a Label> {
        let mut out: Vec<&'a Label> = self.project.labels.values().collect();
        out.extend(
            self.auto
                .iter()
                .filter(|(a, _)| !self.project.labels.contains_key(a))
                .map(|(_, l)| l),
        );
        out.sort_by_key(|l| l.address);
        out
    }
}

impl SymbolLookup for Symbols<'_> {
    fn name_for(&self, address: SnesAddress) -> Option<Symbol> {
        self.label_at(address).map(|l| Symbol {
            name: l.name.clone(),
            user: matches!(l.source, LabelSource::User | LabelSource::Imported(_)),
        })
    }
}
