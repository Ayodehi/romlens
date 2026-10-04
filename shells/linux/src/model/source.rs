//! The Source tab's state (docs/22, S2): the source files of the project's
//! imported `.dbg` files, the one shown, its text, and which of its lines made
//! bytes. The macOS twin is `SourceModel`.

use std::collections::HashMap;

use romlens_ffi::Workbench;
use romlens_ffi::source::{SourceFileInfo, SourceLineInfo};

#[derive(Default)]
pub struct SourceModel {
    pub files: Vec<SourceFileInfo>,
    /// An index into `files`.
    pub shown: Option<usize>,
    /// The shown file's lines, or `None` when it cannot be read.
    pub text: Option<Vec<String>>,
    /// Why `text` is `None`, or a warning about it.
    pub problem: Option<String>,
    /// The shown file's lines that made bytes, by line number (1-based).
    pub by_line: HashMap<u32, SourceLineInfo>,
    /// Bumped whenever the shown file or its text changes.
    pub generation: u64,
}

impl SourceModel {
    pub fn has_files(&self) -> bool {
        !self.files.is_empty()
    }

    pub fn shown_file(&self) -> Option<&SourceFileInfo> {
        self.shown.and_then(|i| self.files.get(i))
    }

    /// Read the files again, after an import or on opening a project.
    pub fn reload(&mut self, wb: &Workbench) {
        let fresh = wb.source_files();
        if fresh == self.files {
            return;
        }
        let keep = self.shown_file().map(|f| (f.map, f.file));
        self.files = fresh;
        let again =
            keep.and_then(|(m, f)| self.files.iter().position(|x| x.map == m && x.file == f));
        // The first file with lines of its own, usually the one with RESET.
        let first = self
            .files
            .iter()
            .position(|f| f.lines > 0)
            .or(if self.files.is_empty() { None } else { Some(0) });
        self.show(again.or(first), wb);
    }

    /// Show file `index`.
    pub fn show(&mut self, index: Option<usize>, wb: &Workbench) {
        self.shown = index;
        self.by_line.clear();
        self.text = None;
        self.problem = None;
        if let Some(f) = self.shown_file().cloned() {
            for l in wb.source_file_lines(f.map, f.file) {
                self.by_line.entry(l.line).or_insert(l);
            }
            self.load(&f);
        }
        self.generation += 1;
    }

    /// Show the file of `line`, if it is not already.
    pub fn show_file_of(&mut self, line: &SourceLineInfo, wb: &Workbench) {
        if self
            .shown_file()
            .is_some_and(|f| f.map == line.map && f.file == line.file)
        {
            return;
        }
        if let Some(i) = self
            .files
            .iter()
            .position(|f| f.map == line.map && f.file == line.file)
        {
            self.show(Some(i), wb);
        }
    }

    fn load(&mut self, f: &SourceFileInfo) {
        let data = match std::fs::read(&f.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.problem = Some(format!("{} is not at {}.", f.name, f.path));
                return;
            }
            Err(_) => {
                self.problem = Some(format!("{} could not be read.", f.name));
                return;
            }
        };
        // ca65 reads bytes; most sources are ASCII or UTF-8, older ones Latin-1.
        let string = String::from_utf8_lossy(&data);
        let mut lines: Vec<String> = string
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
            .collect();
        if lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        self.text = Some(lines);
        if f.size != 0 && data.len() != f.size as usize {
            self.problem = Some(format!(
                "{} has changed since it was assembled ({} bytes, was {}): lines may not match.",
                f.name,
                data.len(),
                f.size
            ));
        }
    }
}

/// A macro's line made bytes once per expansion: clicking it again goes to the
/// next. Returns the range to select given what is selected now.
pub fn next_range(
    ranges: &[romlens_ffi::ByteRange],
    current: Option<u32>,
) -> Option<romlens_ffi::ByteRange> {
    if ranges.is_empty() {
        return None;
    }
    let at = ranges
        .iter()
        .position(|r| current.is_some_and(|c| c >= r.start && c < r.start + r.len));
    Some(ranges[at.map_or(0, |i| (i + 1) % ranges.len())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use romlens_ffi::{ByteRange, Rom, make_ca65_test_program};

    fn project() -> (std::path::PathBuf, std::sync::Arc<Workbench>) {
        // The ca65 fixture: a ROM, its .dbg and the sources it names.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("romlens-src-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let files = make_ca65_test_program();
        for f in &files {
            std::fs::write(dir.join(&f.name), &f.bytes).unwrap();
        }
        let find = |n: &str| files.iter().find(|f| f.name == n).expect(n);
        let rom = Rom::from_bytes(find("fixture.sfc").bytes.clone(), "fixture.sfc".into()).unwrap();
        let wb = Workbench::new(rom);
        let text = String::from_utf8(find("fixture.dbg").bytes.clone()).unwrap();
        wb.import_dbg(
            "fixture.dbg".into(),
            dir.to_string_lossy().into_owned(),
            text,
        )
        .unwrap();
        (dir, wb)
    }

    #[test]
    fn imported_sources_are_listed_read_and_mapped_to_lines() {
        let (dir, wb) = project();
        let mut s = SourceModel::default();
        assert!(!s.has_files());
        s.reload(&wb);
        assert!(s.has_files());
        let f = s.shown_file().expect("a file is shown").clone();
        assert!(f.lines > 0, "the first file with lines of its own");
        let text = s.text.as_ref().expect("the source is read");
        assert!(!text.is_empty());
        assert!(!s.by_line.is_empty());
        let before = s.generation;
        // Nothing changed: reloading does not disturb what is shown.
        s.reload(&wb);
        assert_eq!(s.generation, before);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_file_says_where_it_looked() {
        let (dir, wb) = project();
        let mut s = SourceModel::default();
        s.reload(&wb);
        let path = s.shown_file().unwrap().path.clone();
        std::fs::remove_file(&path).unwrap();
        s.show(s.shown, &wb);
        assert!(s.text.is_none());
        assert!(
            s.problem.as_ref().unwrap().contains(&path),
            "{:?}",
            s.problem
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_changed_file_warns_that_lines_may_not_match() {
        let (dir, wb) = project();
        let mut s = SourceModel::default();
        s.reload(&wb);
        let path = s.shown_file().unwrap().path.clone();
        let mut data = std::fs::read(&path).unwrap();
        data.extend_from_slice(b"; added later\n");
        std::fs::write(&path, data).unwrap();
        s.show(s.shown, &wb);
        assert!(s.text.is_some());
        assert!(
            s.problem
                .as_ref()
                .unwrap()
                .contains("has changed since it was assembled")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clicking_a_macro_line_again_goes_to_the_next_expansion() {
        let r = |start, len| ByteRange { start, len };
        let ranges = vec![r(0x10, 3), r(0x40, 2), r(0x80, 4)];
        assert_eq!(next_range(&ranges, None), Some(r(0x10, 3)));
        assert_eq!(next_range(&ranges, Some(0x11)), Some(r(0x40, 2)));
        assert_eq!(next_range(&ranges, Some(0x41)), Some(r(0x80, 4)));
        assert_eq!(next_range(&ranges, Some(0x83)), Some(r(0x10, 3)), "wraps");
        assert_eq!(next_range(&ranges, Some(0x500)), Some(r(0x10, 3)));
        assert_eq!(next_range(&[], Some(0)), None);
    }
}
