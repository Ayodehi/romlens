//! Folding the C's blocks the way an IDE does: `for (…) {…}` on one line. A
//! `{ … }` block that spans lines can be hidden between its braces. Pure
//! offsets (in chars), so every line, token and highlight the pane keeps stays
//! good whether a block is folded or not. The macOS twin is `CFold` and
//! `CFolder`.

use std::collections::HashSet;
use std::ops::Range;

/// A `{ … }` block of the C that spans lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    /// Char offsets of the two braces.
    pub open: usize,
    pub close: usize,
}

impl Fold {
    /// What folding hides: everything between the braces. Its first character
    /// is where the `…` is drawn.
    pub fn hidden(&self) -> Range<usize> {
        self.open + 1..self.close
    }

    pub fn contains(&self, index: usize) -> bool {
        self.open < index && index <= self.close
    }
}

/// The blocks in C text that span lines, in the order they open. Braces in
/// comments, strings and character literals are not blocks.
pub fn find(text: &str) -> Vec<Fold> {
    let u: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    // Each open brace, with how many lines had ended before it.
    let mut opens: Vec<(usize, usize)> = Vec::new();
    let mut lines = 0;
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        let next = u.get(i + 1).copied().unwrap_or('\0');
        if c == '/' && next == '*' {
            i += 2;
            while i < u.len() && !(u[i] == '*' && u.get(i + 1) == Some(&'/')) {
                if u[i] == '\n' {
                    lines += 1;
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '/' && next == '/' {
            while i < u.len() && u[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '"' || c == '\'' {
            i += 1;
            while i < u.len() && u[i] != c && u[i] != '\n' {
                i += if u[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            continue;
        }
        match c {
            '\n' => lines += 1,
            '{' => opens.push((i, lines)),
            '}' => {
                if let Some((at, opened)) = opens.pop()
                    && opened < lines
                {
                    out.push(Fold { open: at, close: i });
                }
            }
            _ => {}
        }
        i += 1;
    }
    out.sort_by_key(|f| f.open);
    out
}

/// Which of the C's blocks are folded.
#[derive(Default)]
pub struct Folder {
    folds: Vec<Fold>,
    folded: HashSet<usize>,
}

impl Folder {
    /// New text is coming: nothing is folded until `set` says so.
    pub fn clear(&mut self) {
        self.folds.clear();
        self.folded.clear();
    }

    /// The text's blocks, and which of them start folded.
    pub fn set(&mut self, folds: Vec<Fold>, folded: HashSet<usize>) {
        self.folded = folded
            .intersection(&folds.iter().map(|f| f.open).collect())
            .copied()
            .collect();
        self.folds = folds;
    }

    pub fn folds(&self) -> &[Fold] {
        &self.folds
    }

    pub fn folded_opens(&self) -> &HashSet<usize> {
        &self.folded
    }

    pub fn is_folded(&self, f: &Fold) -> bool {
        self.folded.contains(&f.open)
    }

    /// What is hidden: the outermost folded blocks, in order.
    pub fn hidden(&self) -> Vec<Range<usize>> {
        let mut out: Vec<Range<usize>> = Vec::new();
        for f in &self.folds {
            if !self.folded.contains(&f.open) {
                continue;
            }
            if out.last().is_some_and(|last| last.contains(&f.open)) {
                continue;
            }
            out.push(f.hidden());
        }
        out
    }

    /// Whether `index` is hidden inside a folded block.
    pub fn is_hidden(&self, index: usize) -> bool {
        self.hidden().iter().any(|r| r.contains(&index))
    }

    pub fn toggle(&mut self, f: &Fold) {
        if !self.folded.remove(&f.open) {
            self.folded.insert(f.open);
        }
    }

    /// Fold the block opened on the line `line`, else the innermost open one
    /// around `index`.
    pub fn fold(&mut self, index: usize, line: Range<usize>) -> bool {
        let pick = self
            .folds
            .iter()
            .rev()
            .find(|f| line.contains(&f.open) && !self.is_folded(f))
            .or_else(|| {
                self.folds
                    .iter()
                    .rev()
                    .find(|f| f.contains(index) && !self.is_folded(f))
            })
            .copied();
        match pick {
            Some(f) => {
                self.folded.insert(f.open);
                true
            }
            None => false,
        }
    }

    /// Open the folded blocks on the line `line` and around `index`.
    pub fn unfold(&mut self, index: usize, line: Range<usize>) -> bool {
        let opened: Vec<usize> = self
            .folds
            .iter()
            .filter(|f| self.is_folded(f) && (line.contains(&f.open) || f.contains(index)))
            .map(|f| f.open)
            .collect();
        for o in &opened {
            self.folded.remove(o);
        }
        !opened.is_empty()
    }

    /// Fold every block inside the routine, so its body reads as its outline
    /// and each block opens one level at a time.
    pub fn fold_all(&mut self) {
        let outermost: HashSet<usize> = self
            .folds
            .iter()
            .filter(|f| !self.folds.iter().any(|o| o != *f && o.contains(f.open)))
            .map(|f| f.open)
            .collect();
        self.folded = self
            .folds
            .iter()
            .map(|f| f.open)
            .filter(|o| !outermost.contains(o))
            .collect();
    }

    pub fn unfold_all(&mut self) {
        self.folded.clear();
    }

    /// Open whatever hides any of `range`: the selection moved there.
    pub fn reveal(&mut self, range: Range<usize>) -> bool {
        let opened: Vec<usize> = self
            .folds
            .iter()
            .filter(|f| {
                let h = f.hidden();
                self.is_folded(f) && h.start < range.end && range.start < h.end
            })
            .map(|f| f.open)
            .collect();
        for o in &opened {
            self.folded.remove(o);
        }
        !opened.is_empty()
    }

    /// The folded block whose `…` is the character at `index`.
    pub fn placeholder_at(&self, index: usize) -> Option<Fold> {
        self.folds
            .iter()
            .find(|f| self.is_folded(f) && f.hidden().start == index && !self.is_hidden(f.open))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "void f(void)
{
    /* { not a block } */
    if (a) {
        s = \"}{\";
        c = '{';
    } else {
        x = 1; // }
    }
    { y = 2; }
}";

    fn line_of(text: &str, at: usize) -> usize {
        text.chars().take(at).filter(|c| *c == '\n').count()
    }

    fn line_range(text: &str, line: usize) -> Range<usize> {
        let mut start = 0;
        for (i, l) in text.split('\n').enumerate() {
            let end = start + l.chars().count() + 1;
            if i == line {
                return start..end;
            }
            start = end;
        }
        0..0
    }

    #[test]
    fn blocks_are_the_braces_that_span_lines() {
        let folds = find(TEXT);
        let lines: Vec<_> = folds
            .iter()
            .map(|f| [line_of(TEXT, f.open), line_of(TEXT, f.close)])
            .collect();
        // Comments, strings, character literals and one-line blocks are not blocks.
        assert_eq!(lines, [[1, 10], [3, 6], [6, 8]]);
    }

    #[test]
    fn folding_hides_the_outermost_blocks_only() {
        let mut f = Folder::default();
        f.set(find(TEXT), HashSet::new());
        assert!(f.hidden().is_empty());
        f.fold_all();
        // Every block but the routine's own, so the body reads as its outline.
        let outer = f.folds()[0];
        assert!(!f.is_folded(&outer));
        assert_eq!(f.hidden().len(), 2);
        f.toggle(&outer);
        // Folding the outer one hides everything inside it as one range.
        assert_eq!(f.hidden(), vec![outer.hidden()]);
        assert!(f.is_hidden(outer.open + 5));
        f.unfold_all();
        assert!(f.hidden().is_empty());
    }

    #[test]
    fn fold_and_unfold_work_from_the_line_or_the_caret() {
        let mut f = Folder::default();
        f.set(find(TEXT), HashSet::new());
        let ifline = line_range(TEXT, 3);
        assert!(f.fold(ifline.start, ifline.clone()));
        let blk = f.folds()[1];
        assert!(f.is_folded(&blk));
        // Nothing more opens on that line, so the same key folds the block
        // around the caret: the routine's own.
        assert!(f.fold(ifline.start, ifline.clone()));
        assert!(f.is_folded(&f.folds()[0]));
        f.toggle(&f.folds()[0].clone());
        assert!(f.unfold(blk.open + 2, 0..0));
        assert!(!f.is_folded(&blk));
        assert!(!f.unfold(0, 0..0), "nothing is folded");
    }

    #[test]
    fn a_selection_inside_a_folded_block_opens_it() {
        let mut f = Folder::default();
        f.set(find(TEXT), HashSet::new());
        let blk = f.folds()[1];
        f.toggle(&blk);
        assert!(f.reveal(blk.open + 3..blk.open + 4));
        assert!(!f.is_folded(&blk));
        assert!(!f.reveal(0..1));
    }

    #[test]
    fn the_ellipsis_belongs_to_the_outermost_folded_block() {
        let mut f = Folder::default();
        f.set(find(TEXT), HashSet::new());
        let (outer, inner) = (f.folds()[0], f.folds()[1]);
        f.toggle(&inner);
        assert_eq!(f.placeholder_at(inner.hidden().start), Some(inner));
        f.toggle(&outer);
        assert_eq!(
            f.placeholder_at(inner.hidden().start),
            None,
            "hidden inside the outer block"
        );
        assert_eq!(f.placeholder_at(outer.hidden().start), Some(outer));
        // An open block has no ellipsis.
        f.unfold_all();
        assert_eq!(f.placeholder_at(outer.hidden().start), None);
    }

    #[test]
    fn what_was_folded_stays_folded_by_open_brace_across_a_refresh() {
        let mut f = Folder::default();
        f.set(find(TEXT), HashSet::new());
        let inner = f.folds()[1];
        f.toggle(&inner);
        let keep = f.folded_opens().clone();
        f.clear();
        f.set(find(TEXT), keep);
        assert!(f.is_folded(&inner));
        // An offset that is no longer a block is dropped.
        f.set(find(TEXT), HashSet::from([9999]));
        assert!(f.folded_opens().is_empty());
    }
}
