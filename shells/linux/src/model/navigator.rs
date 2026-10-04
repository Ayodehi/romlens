//! The sidebar's data: labels, variables, regions and banks, loaded off the
//! main thread and filtered with a short debounce. The macOS twin is
//! `NavigatorModel`.

use romlens_ffi::{LabelInfo, LabelSource, RegionInfo, RegionKind, Rom, VariableInfo, Workbench};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavTab {
    Labels,
    Variables,
    Regions,
    Banks,
}

impl NavTab {
    pub const ALL: [NavTab; 4] = [Self::Labels, Self::Variables, Self::Regions, Self::Banks];

    pub fn title(self) -> &'static str {
        match self {
            Self::Labels => "Labels",
            Self::Variables => "Variables",
            Self::Regions => "Regions",
            Self::Banks => "Banks",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Labels => "user-bookmarks-symbolic",
            Self::Variables => "accessories-text-editor-symbolic",
            Self::Regions => "view-paged-symbolic",
            Self::Banks => "view-grid-symbolic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bank {
    pub bank: u8,
    pub file_offset: u32,
    pub length: u32,
}

/// The largest of each kind, per kind. The navigator used to ask for every
/// region and filter shell-side, which was fine at a thousand and is not at
/// the forty thousand a trace import produces. The largest blocks are what a
/// person navigates by, and the list says when it is not showing everything.
pub const REGION_LIMIT: u32 = 500;

#[derive(Default, Clone)]
pub struct NavigatorData {
    pub labels: Vec<LabelInfo>,
    pub variables: Vec<VariableInfo>,
    pub regions: Vec<RegionInfo>,
    pub banks: Vec<Bank>,
    /// Whether the region list is the largest few rather than all of them.
    pub regions_truncated: bool,
}

impl NavigatorData {
    /// Read everything from the workbench. Slow on a big project, so callers
    /// run it off the main thread.
    pub fn load(workbench: &Workbench, rom: &Rom) -> Self {
        let mut regions = workbench.regions_of_kind(RegionKind::Code, REGION_LIMIT);
        regions.extend(workbench.regions_of_kind(RegionKind::Data, REGION_LIMIT));
        regions.sort_by_key(|r| r.start);
        Self {
            labels: workbench.labels(),
            variables: workbench.variables(),
            regions_truncated: regions.len() >= REGION_LIMIT as usize,
            regions,
            banks: banks(rom),
        }
    }
}

/// Banks computed by stepping the canonical address every 32 KB.
pub fn banks(rom: &Rom) -> Vec<Bank> {
    let len = rom.byte_len();
    let mut out: Vec<Bank> = Vec::new();
    let mut offset = 0u32;
    while offset < len {
        let step = 0x8000.min(len - offset);
        if let Some(a) = rom.snes_address_for(offset) {
            let bank = (a >> 16) as u8;
            match out.last_mut() {
                Some(last) if last.bank == bank => last.length += step,
                _ => out.push(Bank {
                    bank,
                    file_offset: offset,
                    length: step,
                }),
            }
        }
        offset += step;
    }
    out
}

fn is_named_by_a_person(l: &LabelInfo) -> bool {
    matches!(l.source, LabelSource::User | LabelSource::Imported)
}

/// Case-insensitive contains, or an address prefix when the query starts with
/// `$`; user labels first, then by address.
pub fn filter_labels(labels: &[LabelInfo], query: &str) -> Vec<LabelInfo> {
    let q = query.trim().to_lowercase();
    let mut matched: Vec<LabelInfo> = if q.is_empty() {
        labels.to_vec()
    } else if let Some(rest) = q.strip_prefix('$') {
        let needle = rest.replace(':', "");
        labels
            .iter()
            .filter(|l| format!("{:06x}", l.address).starts_with(&needle))
            .cloned()
            .collect()
    } else {
        labels
            .iter()
            .filter(|l| l.name.to_lowercase().contains(&q))
            .cloned()
            .collect()
    };
    matched.sort_by(|a, b| {
        is_named_by_a_person(b)
            .cmp(&is_named_by_a_person(a))
            .then(a.address.cmp(&b.address))
    });
    matched
}

#[derive(Default)]
pub struct Navigator {
    pub tab: Option<NavTab>,
    pub filter: String,
    pub data: NavigatorData,
    pub filtered_labels: Vec<LabelInfo>,
    pub filtered_variables: Vec<VariableInfo>,
    pub filtered_regions: Vec<RegionInfo>,
    pub loading: bool,
}

impl Navigator {
    pub fn tab(&self) -> NavTab {
        self.tab.unwrap_or(NavTab::Labels)
    }

    pub fn set_data(&mut self, data: NavigatorData) {
        self.data = data;
        self.loading = false;
        self.apply_filter();
    }

    pub fn apply_filter(&mut self) {
        self.filtered_labels = filter_labels(&self.data.labels, &self.filter);
        let q = self.filter.trim().to_lowercase();
        self.filtered_regions = if q.is_empty() {
            self.data.regions.clone()
        } else {
            self.data
                .regions
                .iter()
                .filter(|r| r.name.to_lowercase().contains(&q))
                .cloned()
                .collect()
        };
        self.filtered_variables = if q.is_empty() {
            self.data.variables.clone()
        } else {
            self.data
                .variables
                .iter()
                .filter(|v| v.name.to_lowercase().contains(&q))
                .cloned()
                .collect()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(address: u32, name: &str, source: LabelSource) -> LabelInfo {
        LabelInfo {
            address,
            name: name.into(),
            source,
            origin: String::new(),
            file_offset: None,
        }
    }

    fn labels() -> Vec<LabelInfo> {
        vec![
            label(0x80_9000, "SUB_809000", LabelSource::Auto),
            label(0x80_8000, "Boot", LabelSource::User),
            label(0x80_8100, "SUB_808100", LabelSource::Auto),
            label(0x80_9100, "Imported", LabelSource::Imported),
        ]
    }

    #[test]
    fn people_named_labels_come_first_then_by_address() {
        let out = filter_labels(&labels(), "");
        let names: Vec<_> = out.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Boot", "Imported", "SUB_808100", "SUB_809000"]);
    }

    #[test]
    fn text_filters_ignore_case() {
        let out = filter_labels(&labels(), "  sub_80 ");
        assert_eq!(out.len(), 2);
        assert!(
            filter_labels(&labels(), "BOOT")
                .iter()
                .all(|l| l.name == "Boot")
        );
        assert!(filter_labels(&labels(), "zzz").is_empty());
    }

    #[test]
    fn a_dollar_filters_by_address_prefix() {
        let by = |q: &str| filter_labels(&labels(), q).len();
        assert_eq!(by("$80"), 4);
        assert_eq!(by("$80:81"), 1);
        assert_eq!(by("$808"), 2);
        assert_eq!(by("$00"), 0);
    }

    #[test]
    fn banks_merge_consecutive_chunks() {
        let rom = crate::model::testing::test_rom();
        let banks = banks(&rom);
        assert_eq!(banks.len(), 1);
        assert_eq!(
            (banks[0].bank, banks[0].file_offset, banks[0].length),
            (0, 0, 0x8000)
        );
    }

    #[test]
    fn data_loads_and_filters_by_region_name() {
        let rom = crate::model::testing::test_rom();
        let wb = Workbench::new(rom.clone());
        wb.analyze_blocking().unwrap();
        let mut nav = Navigator {
            loading: true,
            ..Navigator::default()
        };
        nav.set_data(NavigatorData::load(&wb, &rom));
        assert!(!nav.loading);
        assert!(!nav.filtered_regions.is_empty());
        assert!(
            nav.filtered_regions
                .windows(2)
                .all(|w| w[0].start <= w[1].start)
        );
        nav.filter = "code".into();
        nav.apply_filter();
        assert!(
            nav.filtered_regions
                .iter()
                .all(|r| r.name.to_lowercase().contains("code"))
        );
        assert_eq!(nav.tab(), NavTab::Labels);
    }
}
