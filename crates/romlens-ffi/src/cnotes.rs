//! The C's annotations over the FFI (docs/24, U4): local names, routine
//! notes, C comments and C versions, and colouring C text nobody generated.

use romlens_core::SnesAddress;
use romlens_core::decompile::lex;
use romlens_core::model::c_notes::{Anchor, Author, CVersion};

use crate::records::{CTokenInfo, Command};
use crate::workbench::Workbench;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CAuthor {
    User,
    Tutor,
}

/// Lines `first..=last` (from 1) stand for the instructions `start..=end`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CAnchorInfo {
    pub first: u32,
    pub last: u32,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CVersionInfo {
    pub text: String,
    pub author: CAuthor,
    pub anchors: Vec<CAnchorInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NamedCVersionInfo {
    pub routine: u32,
    pub name: String,
    pub version: CVersionInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LocalNameInfo {
    pub local: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CCommentInfo {
    pub address: u32,
    pub text: String,
}

/// What `/rewind` did to the tutor's edits.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RewoundInfo {
    /// Edits undone from the top of the undo stack.
    pub undone: u32,
    /// Older edits taken back as one new undoable step.
    pub reverted: u32,
    /// Edits left because the student changed the same thing afterwards.
    pub kept: Vec<String>,
}

/// Who a batch of edits is from.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum EditOrigin {
    User,
    /// The tutor, in a conversation's turn.
    Tutor {
        conversation: String,
        turn: u32,
    },
}

impl From<CVersionInfo> for CVersion {
    fn from(v: CVersionInfo) -> Self {
        CVersion {
            text: v.text,
            author: match v.author {
                CAuthor::User => Author::User,
                CAuthor::Tutor => Author::Tutor,
            },
            anchors: v
                .anchors
                .into_iter()
                .map(|a| Anchor {
                    first: a.first,
                    last: a.last,
                    start: SnesAddress::from_u24(a.start),
                    end: SnesAddress::from_u24(a.end),
                })
                .collect(),
        }
    }
}

impl From<&CVersion> for CVersionInfo {
    fn from(v: &CVersion) -> Self {
        CVersionInfo {
            text: v.text.clone(),
            author: match v.author {
                Author::User => CAuthor::User,
                Author::Tutor => CAuthor::Tutor,
            },
            anchors: v
                .anchors
                .iter()
                .map(|a| CAnchorInfo {
                    first: a.first,
                    last: a.last,
                    start: a.start.as_u24(),
                    end: a.end.as_u24(),
                })
                .collect(),
        }
    }
}

/// Byte offsets of `text` to UTF-16 offsets, as Swift counts them.
pub(crate) fn utf16_tokens(
    text: &str,
    tokens: &[romlens_core::decompile::CToken],
) -> Vec<CTokenInfo> {
    let mut at = Vec::with_capacity(text.len() + 1);
    let mut n = 0u32;
    for c in text.chars() {
        for _ in 0..c.len_utf8() {
            at.push(n);
        }
        n += c.len_utf16() as u32;
    }
    at.push(n);
    tokens
        .iter()
        .map(|t| {
            let start = at[t.start as usize];
            let end = at[(t.start + t.len) as usize];
            CTokenInfo {
                start,
                len: end - start,
                kind: t.kind.into(),
                address: t.address.map(|a| a.as_u24()),
            }
        })
        .collect()
}

#[uniffi::export]
impl Workbench {
    /// Several edits as one undoable step, from the user or the tutor.
    pub fn execute_batch(
        &self,
        commands: Vec<Command>,
        origin: EditOrigin,
    ) -> Result<(), crate::RomlensError> {
        let origin = match origin {
            EditOrigin::User => romlens_core::model::Origin::User,
            EditOrigin::Tutor { conversation, turn } => {
                romlens_core::model::Origin::Tutor { conversation, turn }
            }
        };
        self.apply_commands(commands.into_iter().map(Into::into).collect(), origin)
    }

    /// Take back the edits the tutor made in `conversation` from turn
    /// `from_turn` on (docs/24, `/rewind`).
    pub fn rewind_tutor_edits(
        &self,
        conversation: String,
        from_turn: u32,
    ) -> Result<RewoundInfo, crate::RomlensError> {
        let r = self.rewind_edits(&|o| {
            matches!(o, romlens_core::model::Origin::Tutor { conversation: c, turn }
                if *c == conversation && *turn >= from_turn)
        })?;
        Ok(RewoundInfo {
            undone: r.undone,
            reverted: r.reverted,
            kept: r.kept,
        })
    }

    pub fn local_names(&self, routine: u32) -> Vec<LocalNameInfo> {
        let r = self.canonical_address(routine);
        self.with_project(|p| {
            p.local_names
                .iter()
                .filter(|((a, _), _)| a.as_u24() == r)
                .map(|((_, l), n)| LocalNameInfo {
                    local: l.clone(),
                    name: n.clone(),
                })
                .collect()
        })
    }

    pub fn routine_note(&self, routine: u32) -> Option<String> {
        let r = SnesAddress::from_u24(self.canonical_address(routine));
        self.with_project(|p| p.routine_notes.get(&r).cloned())
    }

    pub fn c_comments(&self) -> Vec<CCommentInfo> {
        self.with_project(|p| {
            p.c_comments
                .iter()
                .map(|(a, t)| CCommentInfo {
                    address: a.as_u24(),
                    text: t.clone(),
                })
                .collect()
        })
    }

    /// The C versions of a routine, or of every routine with `None`.
    pub fn c_versions(&self, routine: Option<u32>) -> Vec<NamedCVersionInfo> {
        let r = routine.map(|r| self.canonical_address(r));
        self.with_project(|p| {
            p.c_versions
                .iter()
                .filter(|((a, _), _)| r.is_none_or(|r| a.as_u24() == r))
                .map(|((a, n), v)| NamedCVersionInfo {
                    routine: a.as_u24(),
                    name: n.clone(),
                    version: v.into(),
                })
                .collect()
        })
    }

    /// Tokens for C text nobody generated (a C version, C in the tutor's
    /// answer), with the project's labels.
    pub fn lex_c(&self, text: String) -> Vec<CTokenInfo> {
        let labels = self.labels();
        let find = |n: &str| {
            labels
                .iter()
                .find(|l| l.name == n)
                .map(|l| SnesAddress::from_u24(l.address))
        };
        utf16_tokens(&text, &lex::lex(&text, &find))
    }
}
