//! The inspector's "Screen at this point" (docs/21): what the PPU registers
//! hold when the selected instruction runs. It takes a walk back through the
//! routine, so it is worked out off the main thread and only while the
//! section is open. The macOS twin is `showScreen`, `screen` and
//! `refreshScreen` in `RomViewModel`.

use romlens_ffi::ScreenSetupInfo;

#[derive(Default)]
pub struct ScreenModel {
    /// The section is open.
    pub shown: bool,
    pub setup: Option<ScreenSetupInfo>,
    pub loading: bool,
    /// The instruction `setup` (or the run in flight) is for.
    for_offset: Option<u32>,
    ticket: u64,
}

impl ScreenModel {
    /// The instruction at the selection changed, the section opened or the
    /// analysis moved on. Returns the run to start, if one is needed: the
    /// instruction and a ticket to hand back to `finished`.
    pub fn want(&mut self, at: Option<u32>, force: bool) -> Option<(u32, u64)> {
        let Some(at) = at else {
            self.clear();
            return None;
        };
        if !self.shown || (!force && self.for_offset == Some(at)) {
            return None;
        }
        self.for_offset = Some(at);
        self.ticket += 1;
        self.loading = true;
        Some((at, self.ticket))
    }

    /// A run finished. False if it was for an instruction no longer wanted.
    pub fn finished(&mut self, ticket: u64, setup: Option<ScreenSetupInfo>) -> bool {
        if ticket != self.ticket {
            return false;
        }
        self.setup = setup;
        self.loading = false;
        true
    }

    /// Nothing is selected, or the selection is not an instruction.
    pub fn clear(&mut self) {
        self.ticket += 1;
        self.setup = None;
        self.loading = false;
        self.for_offset = None;
    }

    /// The analysis changed: what was worked out may no longer be true.
    pub fn invalidate(&mut self) {
        self.for_offset = None;
    }
}

/// The text a link's button carries.
pub fn link_title(link: romlens_ffi::ScreenLinkInfo) -> &'static str {
    match link {
        romlens_ffi::ScreenLinkInfo::Tiles { .. } => "Show Tiles",
        romlens_ffi::ScreenLinkInfo::Tilemap { .. } => "Show Tilemap",
        romlens_ffi::ScreenLinkInfo::Palette { .. } => "Show Palette",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> ScreenModel {
        ScreenModel {
            shown: true,
            ..ScreenModel::default()
        }
    }

    #[test]
    fn nothing_is_worked_out_while_the_section_is_closed() {
        let mut m = ScreenModel::default();
        assert_eq!(m.want(Some(4), false), None);
        assert!(!m.loading);
        m.shown = true;
        assert!(m.want(Some(4), false).is_some(), "opening it asks");
    }

    #[test]
    fn the_same_instruction_is_asked_once_unless_forced() {
        let mut m = open();
        let (at, ticket) = m.want(Some(4), false).unwrap();
        assert_eq!(at, 4);
        assert!(m.loading);
        assert_eq!(m.want(Some(4), false), None);
        assert!(m.want(Some(4), true).is_some(), "an analysis changed");
        assert!(!m.finished(ticket, None), "the older run is stale");
    }

    #[test]
    fn a_newer_selection_beats_an_older_run() {
        let mut m = open();
        let (_, first) = m.want(Some(4), false).unwrap();
        let (_, second) = m.want(Some(9), false).unwrap();
        assert!(!m.finished(first, None));
        assert!(m.loading);
        assert!(m.finished(second, None));
        assert!(!m.loading);
    }

    #[test]
    fn leaving_an_instruction_clears_the_result_and_drops_the_run() {
        let mut m = open();
        let (_, ticket) = m.want(Some(4), false).unwrap();
        assert_eq!(m.want(None, false), None);
        assert!(!m.loading && m.setup.is_none());
        assert!(!m.finished(ticket, None));
        // Coming back asks again.
        assert!(m.want(Some(4), false).is_some());
    }

    #[test]
    fn invalidate_makes_the_same_instruction_ask_again() {
        let mut m = open();
        let (_, t) = m.want(Some(4), false).unwrap();
        m.finished(t, None);
        m.invalidate();
        assert!(m.want(Some(4), false).is_some());
    }
}
