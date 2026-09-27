//! The tutor's edits (docs/24, decision 5): the changes a student can make
//! to a project, made by the tutor under the permission mode. Read-only
//! refuses them; "ask before edits" shows each as a card and waits for the
//! student; "accept edits" makes them at once. Each one made goes on the
//! undo stack as the tutor's, with the conversation and turn, so `/rewind`
//! can take it back.

use romlens_core::SnesAddress;
use romlens_core::memory::address::FileOffset;
use romlens_core::model::c_notes::{Anchor, Author, CVersion};
use romlens_core::model::region::{BankRule, DataKind, OverrideKind};
use romlens_core::model::variable::{VarType, VarWidth};
use romlens_core::model::{Command, CommentKind, FlagOverride, Origin};
use romlens_tutor::agent::{Decision, Event, Mode, Proposal, ToolContext, ToolOutput};
use romlens_tutor::provider::ToolSpec;
use serde_json::{Value, json};

use super::tools::{addr, boolean, choice, integer, nullable, object, spec, string};
use crate::workbench::Workbench;

const ADDRESS: &str =
    "A CPU address such as $80:8000, a file offset such as 0x1234, or a label's name";
const ROUTINE: &str = "The routine's entry, or any address in it";
const REASON: &str = "Why, in a sentence the student will read on the card, citing the evidence";

pub const NAMES: &[&str] = &[
    "set_label",
    "set_comment",
    "define_variable",
    "mark_region",
    "set_flags",
    "rename_local",
    "set_routine_note",
    "set_c_comment",
    "write_c_version",
];

pub fn specs() -> Vec<ToolSpec> {
    let anchor = object(&[
        ("first", integer("The first line of the version, from 1")),
        ("last", integer("The last line")),
        (
            "start",
            string("The first instruction's address those lines stand for"),
        ),
        ("end", string("The last instruction's address")),
    ]);
    vec![
        spec(
            "set_label",
            "Name (or with null, unname) an address: a routine, a branch target, a variable, a table. The name shows in the listing, the C and everywhere the address is used.",
            &[
                ("address", string(ADDRESS)),
                (
                    "name",
                    nullable(string(
                        "A name in the project's style, letters, digits and _, or null to remove it",
                    )),
                ),
                ("reason", string(REASON)),
            ],
        ),
        spec(
            "set_comment",
            "A comment in the listing at an address: a line comment after the instruction, or a block comment above it. null removes it.",
            &[
                ("address", string(ADDRESS)),
                ("text", nullable(string("The comment, or null"))),
                (
                    "kind",
                    choice(&["line", "block"], "After the instruction, or above it"),
                ),
                ("reason", string(REASON)),
            ],
        ),
        spec(
            "define_variable",
            "Define a variable in RAM: a name and a type, so the listing and the C read it by name.",
            &[
                ("address", string("Its address, such as $7E:0AF6")),
                ("name", string("Its name")),
                (
                    "width",
                    choice(&["byte", "word", "long"], "One element's width"),
                ),
                (
                    "count",
                    integer("Elements: 1 for a single value, more for an array"),
                ),
                ("reason", string(REASON)),
            ],
        ),
        spec(
            "mark_region",
            "Tell the analysis what a range of the ROM is: code, a kind of data, or unknown. The analysis runs again.",
            &[
                ("address", string(ADDRESS)),
                ("length", integer("Bytes")),
                (
                    "kind",
                    choice(
                        &[
                            "code",
                            "unknown",
                            "byte",
                            "word",
                            "long",
                            "pointer",
                            "string",
                            "graphics_2bpp",
                            "graphics_4bpp",
                            "graphics_8bpp",
                            "tilemap",
                            "palette",
                            "compressed",
                            "sample",
                        ],
                        "What it is",
                    ),
                ),
                ("reason", string(REASON)),
            ],
        ),
        spec(
            "set_flags",
            "Tell the analysis the M, X or E flag at an instruction, where it guessed wrong (garbage after a call is the sign). null leaves a flag to the analysis. Test first with decode_as.",
            &[
                ("address", string(ADDRESS)),
                ("m", nullable(boolean("M=1: A 8-bit"))),
                ("x", nullable(boolean("X=1: X and Y 8-bit"))),
                ("e", nullable(boolean("E=1: emulation mode"))),
                ("reason", string(REASON)),
            ],
        ),
        spec(
            "rename_local",
            "Give one of a routine's locals or parameters in the C a name (a, x, a8, i, x_out…, as decompile shows them). null restores Romlens's name.",
            &[
                ("routine", string(ROUTINE)),
                ("local", string("The name decompile shows now")),
                ("name", nullable(string("A C name, or null"))),
                ("reason", string(REASON)),
            ],
        ),
        spec(
            "set_routine_note",
            "A note printed above the routine in the C: what it does and what it takes and returns, in plain words. null removes it.",
            &[
                ("routine", string(ROUTINE)),
                ("text", nullable(string("The note, or null"))),
                ("reason", string(REASON)),
            ],
        ),
        spec(
            "set_c_comment",
            "A comment in the C before the statement an instruction makes. null removes it.",
            &[
                ("address", string("The instruction's address")),
                ("text", nullable(string("The comment, or null"))),
                ("reason", string(REASON)),
            ],
        ),
        spec(
            "write_c_version",
            "Save a C version of a routine: your own rewrite of it that explains what it does better than the generated C, shown beside it and never compiled. Keep it faithful, say in it what you simplified, and anchor its lines to the instructions they stand for. An empty text removes the version.",
            &[
                ("routine", string(ROUTINE)),
                (
                    "name",
                    string("A short name for this version, such as \"Plain words\""),
                ),
                ("text", string("The C")),
                (
                    "anchors",
                    json!({"type": "array", "items": anchor, "description": "Which lines stand for which instructions"}),
                ),
                ("reason", string(REASON)),
            ],
        ),
    ]
}

/// An edit's commands, its card's line, and what is there now and after.
type Plan = (Vec<Command>, String, Option<String>, Option<String>);

pub struct Edits<'a> {
    pub wb: &'a Workbench,
    pub place: super::media::Place<'a>,
}

fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v[k].as_str().ok_or_else(|| format!("{k} must be text"))
}

fn opt(v: &Value, k: &str) -> Option<String> {
    v[k].as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

impl Edits<'_> {
    fn snes(&self, t: &str) -> Result<SnesAddress, String> {
        let r = self.wb.resolve_any(t.trim().to_owned());
        match r {
            Ok(r) => Ok(SnesAddress::from_u24(r.snes_address)),
            Err(_) => (self.place)(t).map(|(a, _)| SnesAddress::from_u24(a)),
        }
    }

    fn routine(&self, t: &str) -> Result<SnesAddress, String> {
        let (a, off) = (self.place)(t)?;
        Ok(SnesAddress::from_u24(
            self.wb.function_containing(off).unwrap_or(a),
        ))
    }

    /// The commands, a line for the card, and what is there now and after.
    fn plan(
        &self,
        name: &str,
        v: &Value,
    ) -> Result<Plan, String> {
        Ok(match name {
            "set_label" => {
                let a = self.snes(text(v, "address")?)?;
                let new = opt(v, "name");
                let old = self.wb.label_at(a.as_u24()).map(|l| l.name);
                let line = match &new {
                    Some(n) => format!("Name {} `{n}`", addr(a.as_u24())),
                    None => format!("Remove the name at {}", addr(a.as_u24())),
                };
                (
                    vec![Command::SetLabel {
                        address: a,
                        name: new.clone(),
                    }],
                    line,
                    old,
                    new,
                )
            }
            "set_comment" => {
                let a = self.snes(text(v, "address")?)?;
                let kind = if v["kind"] == "block" {
                    CommentKind::Block
                } else {
                    CommentKind::Line
                };
                let new = opt(v, "text");
                let old = self.wb.comment_at(a.as_u24(), kind.into()).map(|c| c.text);
                (
                    vec![Command::SetComment {
                        address: a,
                        kind,
                        text: new.clone(),
                    }],
                    format!("Comment at {}", addr(a.as_u24())),
                    old,
                    new,
                )
            }
            "define_variable" => {
                let a = self.snes(text(v, "address")?)?;
                let n = text(v, "name")?.trim().to_owned();
                let width =
                    VarWidth::parse(text(v, "width")?).ok_or("width is byte, word or long")?;
                let count = v["count"].as_u64().unwrap_or(1).clamp(1, 0xFFFF) as u16;
                let ty = VarType { width, count };
                (
                    vec![
                        Command::SetLabel {
                            address: a,
                            name: Some(n.clone()),
                        },
                        Command::SetVariable {
                            address: a,
                            ty: Some(ty),
                        },
                    ],
                    format!("Define {} `{n}` as {}", addr(a.as_u24()), ty.describe()),
                    None,
                    Some(format!("{} {n}", ty.describe())),
                )
            }
            "mark_region" => {
                let (a, off) = (self.place)(text(v, "address")?)?;
                let len = v["length"]
                    .as_u64()
                    .ok_or("length must be a whole number")?
                    .max(1) as u32;
                let k = text(v, "kind")?;
                let kind = match k {
                    "code" => OverrideKind::Code,
                    "unknown" => OverrideKind::Unknown,
                    "byte" => OverrideKind::Data(DataKind::Byte),
                    "word" => OverrideKind::Data(DataKind::Word),
                    "long" => OverrideKind::Data(DataKind::Long),
                    "pointer" => OverrideKind::Data(DataKind::Pointer {
                        bank: BankRule::SameBank,
                    }),
                    "string" => OverrideKind::Data(DataKind::String),
                    "graphics_2bpp" => OverrideKind::Data(DataKind::Graphics { bpp: 2 }),
                    "graphics_8bpp" => OverrideKind::Data(DataKind::Graphics { bpp: 8 }),
                    "graphics_4bpp" => OverrideKind::Data(DataKind::Graphics { bpp: 4 }),
                    "tilemap" => OverrideKind::Data(DataKind::Tilemap),
                    "palette" => OverrideKind::Data(DataKind::Palette),
                    "compressed" => OverrideKind::Data(DataKind::Compressed),
                    "sample" => OverrideKind::Data(DataKind::Sample),
                    other => return Err(format!("{other} is not a kind this tool marks")),
                };
                let old = self.wb.region_at(off).map(|r| r.name);
                (
                    vec![Command::MarkRegion {
                        start: FileOffset(off),
                        len,
                        kind,
                    }],
                    format!("Mark {len} bytes at {} as {k}", addr(a)),
                    old,
                    Some(k.to_owned()),
                )
            }
            "set_flags" => {
                let (a, off) = (self.place)(text(v, "address")?)?;
                let f = FlagOverride {
                    m: v["m"].as_bool(),
                    x: v["x"].as_bool(),
                    e: v["e"].as_bool(),
                    dbr: None,
                    dp: None,
                };
                let show = |m: Option<bool>, n: &str| m.map(|b| format!("{n}={}", b as u8));
                let after: Vec<String> = [show(f.m, "M"), show(f.x, "X"), show(f.e, "E")]
                    .into_iter()
                    .flatten()
                    .collect();
                (
                    vec![Command::SetFlagOverride {
                        offset: FileOffset(off),
                        flags: (!after.is_empty()).then_some(f),
                    }],
                    format!(
                        "Flags at {}: {}",
                        addr(a),
                        if after.is_empty() {
                            "the analysis's".into()
                        } else {
                            after.join(" ")
                        }
                    ),
                    self.wb.flag_override_at(off).map(|o| format!("{o:?}")),
                    Some(after.join(" ")),
                )
            }
            "rename_local" => {
                let r = self.routine(text(v, "routine")?)?;
                let local = text(v, "local")?.trim().to_owned();
                let new = opt(v, "name");
                (
                    vec![Command::SetLocalName {
                        routine: r,
                        local: local.clone(),
                        name: new.clone(),
                    }],
                    format!(
                        "In {}, call `{local}` `{}`",
                        addr(r.as_u24()),
                        new.clone().unwrap_or_else(|| local.clone())
                    ),
                    Some(local),
                    new,
                )
            }
            "set_routine_note" => {
                let r = self.routine(text(v, "routine")?)?;
                let new = opt(v, "text");
                (
                    vec![Command::SetRoutineNote {
                        routine: r,
                        text: new.clone(),
                    }],
                    format!("Note on the routine at {}", addr(r.as_u24())),
                    self.wb.routine_note(r.as_u24()),
                    new,
                )
            }
            "set_c_comment" => {
                let a = self.snes(text(v, "address")?)?;
                let new = opt(v, "text");
                let old = self
                    .wb
                    .c_comments()
                    .into_iter()
                    .find(|c| c.address == a.as_u24())
                    .map(|c| c.text);
                (
                    vec![Command::SetCComment {
                        address: a,
                        text: new.clone(),
                    }],
                    format!("C comment at {}", addr(a.as_u24())),
                    old,
                    new,
                )
            }
            "write_c_version" => {
                let r = self.routine(text(v, "routine")?)?;
                let name = text(v, "name")?.trim().to_owned();
                let body = text(v, "text")?.to_owned();
                let mut anchors = Vec::new();
                for a in v["anchors"].as_array().into_iter().flatten() {
                    anchors.push(Anchor {
                        first: a["first"].as_u64().unwrap_or(1) as u32,
                        last: a["last"].as_u64().unwrap_or(1) as u32,
                        start: self.snes(a["start"].as_str().unwrap_or_default())?,
                        end: self.snes(a["end"].as_str().unwrap_or_default())?,
                    });
                }
                let version = (!body.trim().is_empty()).then(|| CVersion {
                    text: body.clone(),
                    author: Author::Tutor,
                    anchors,
                });
                let old = self
                    .wb
                    .c_versions(Some(r.as_u24()))
                    .into_iter()
                    .find(|c| c.name == name)
                    .map(|c| c.version.text);
                (
                    vec![Command::SetCVersion {
                        routine: r,
                        name: name.clone(),
                        version,
                    }],
                    format!("C version \"{name}\" of {}", addr(r.as_u24())),
                    old,
                    (!body.trim().is_empty()).then_some(body),
                )
            }
            other => return Err(format!("there is no edit named {other}")),
        })
    }

    pub fn run(&self, id: &str, name: &str, v: &Value, cx: &ToolContext) -> ToolOutput {
        let (commands, line, before, after) = match self.plan(name, v) {
            Ok(p) => p,
            Err(e) => return ToolOutput::error(e),
        };
        if cx.mode == Mode::ReadOnly {
            return ToolOutput::error(format!(
                "Not changed: Romlens is in read-only mode. Suggest the change in words instead ({line}); the student can switch modes with Shift-Tab."
            ));
        }
        let proposal = Proposal {
            id: id.to_owned(),
            tool: name.to_owned(),
            summary: line.clone(),
            reason: v["reason"].as_str().unwrap_or_default().to_owned(),
            before,
            after,
        };
        if cx.mode == Mode::AskBeforeEdits {
            cx.events.event(Event::EditProposed(proposal.clone()));
            let d = cx.approver.decide(&proposal);
            if let Decision::Reject { why } = d {
                cx.events.event(Event::EditDecided {
                    id: id.to_owned(),
                    applied: false,
                });
                return ToolOutput::text(match why {
                    Some(w) if !w.trim().is_empty() => {
                        format!("The student declined: {line}. They said: {w}")
                    }
                    _ => format!("The student declined: {line}."),
                });
            }
        }
        let origin = Origin::Tutor {
            conversation: cx.conversation.to_owned(),
            turn: cx.turn as u32,
        };
        match self.wb.apply_commands(commands, origin) {
            Ok(()) => {
                cx.events.event(Event::EditDecided {
                    id: id.to_owned(),
                    applied: true,
                });
                ToolOutput::text(format!("Done: {line}. The student can undo it."))
            }
            Err(e) => {
                cx.events.event(Event::EditDecided {
                    id: id.to_owned(),
                    applied: false,
                });
                ToolOutput::error(format!("Romlens refused it: {e}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tutor::tools::RomTools;
    use romlens_tutor::agent::{AcceptAll, Approver, Tools};
    use romlens_tutor::http::Cancel;
    use romlens_tutor::transcript::Part;
    use std::sync::{Arc, Mutex};

    struct Says(Decision, Mutex<Vec<Proposal>>);
    impl Approver for Says {
        fn decide(&self, p: &Proposal) -> Decision {
            self.1.lock().unwrap().push(p.clone());
            self.0.clone()
        }
    }

    fn fixture() -> (Arc<Workbench>, RomTools) {
        let rom = crate::Rom::from_bytes(romlens_core::fixtures::explain_lorom(), "e.sfc".into())
            .unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        (wb.clone(), RomTools::new(wb))
    }

    fn call(
        t: &RomTools,
        mode: Mode,
        turn: usize,
        approver: &dyn Approver,
        name: &str,
        input: Value,
    ) -> (ToolOutput, Vec<Event>) {
        let seen = Mutex::new(Vec::new());
        let on = |e: Event| seen.lock().unwrap().push(e);
        let cancel = Cancel::new();
        let cx = ToolContext {
            mode,
            conversation: "c1",
            turn,
            events: &on,
            approver,
            cancel: &cancel,
        };
        let out = t.run("call_1", name, &input, &cx);
        (out, seen.into_inner().unwrap())
    }

    fn text(o: &ToolOutput) -> String {
        match &o.parts[0] {
            Part::Text { text } => text.clone(),
            _ => String::new(),
        }
    }

    fn name_reset(name: &str) -> Value {
        json!({"address": "$00:8000", "name": name, "reason": "the reset vector points here"})
    }

    #[test]
    fn the_mode_decides() {
        let (wb, t) = fixture();
        let (o, _) = call(
            &t,
            Mode::ReadOnly,
            1,
            &AcceptAll,
            "set_label",
            name_reset("Reset"),
        );
        assert!(o.is_error && text(&o).contains("read-only"), "{}", text(&o));
        assert_eq!(wb.label_at(0x8000).unwrap().name, "RESET_008000");

        let no = Says(
            Decision::Reject {
                why: Some("I prefer Boot".into()),
            },
            Mutex::new(Vec::new()),
        );
        let (o, ev) = call(
            &t,
            Mode::AskBeforeEdits,
            1,
            &no,
            "set_label",
            name_reset("Reset"),
        );
        assert!(
            !o.is_error && text(&o).contains("I prefer Boot"),
            "{}",
            text(&o)
        );
        let asked = no.1.lock().unwrap().clone();
        assert_eq!(asked[0].summary, "Name $00:8000 `Reset`");
        assert_eq!(asked[0].before.as_deref(), Some("RESET_008000"));
        assert!(
            matches!(&ev[0], Event::EditProposed(p) if p.reason == "the reset vector points here")
        );
        assert!(matches!(&ev[1], Event::EditDecided { applied: false, .. }));
        assert_eq!(wb.label_at(0x8000).unwrap().name, "RESET_008000");

        let yes = Says(Decision::Accept, Mutex::new(Vec::new()));
        let (o, _) = call(
            &t,
            Mode::AskBeforeEdits,
            1,
            &yes,
            "set_label",
            name_reset("Reset"),
        );
        assert!(text(&o).starts_with("Done"), "{}", text(&o));
        assert_eq!(wb.label_at(0x8000).unwrap().name, "Reset");
        assert_eq!(wb.undo_title().as_deref(), Some("Tutor: Rename Label"));

        // Accept edits asks no one.
        let (o, ev) = call(
            &t,
            Mode::AcceptEdits,
            3,
            &no,
            "set_routine_note",
            json!({"routine": "$00:8003", "text": "Starts the game.", "reason": "r"}),
        );
        assert!(!o.is_error, "{}", text(&o));
        assert!(!ev.iter().any(|e| matches!(e, Event::EditProposed(_))));
        assert_eq!(wb.routine_note(0x8000).as_deref(), Some("Starts the game."));
        let (o, _) = call(
            &t,
            Mode::AcceptEdits,
            3,
            &no,
            "rename_local",
            json!({"routine": "$00:8000", "local": "c", "name": "carry", "reason": "r"}),
        );
        assert!(!o.is_error, "{}", text(&o));
        let c = wb
            .decompile_blocking(0x8000, crate::records::DecompileLevel::Full)
            .unwrap();
        assert!(
            c.text.contains("/* Starts the game. */") && c.text.contains("carry"),
            "{}",
            c.text
        );

        // A bad name is refused by the project, and says so.
        let (o, _) = call(
            &t,
            Mode::AcceptEdits,
            3,
            &no,
            "rename_local",
            json!({"routine": "$00:8000", "local": "c", "name": "for", "reason": "r"}),
        );
        assert!(o.is_error && text(&o).contains("keyword"), "{}", text(&o));

        // /rewind to turn 2 takes back turn 3's edits and keeps turn 1's.
        let r = wb.rewind_tutor_edits("c1".into(), 2).unwrap();
        assert_eq!((r.undone, r.reverted), (2, 0));
        assert_eq!(wb.routine_note(0x8000), None);
        assert_eq!(wb.label_at(0x8000).unwrap().name, "Reset");
    }

    #[test]
    fn a_version_and_a_variable() {
        let (wb, t) = fixture();
        let (o, _) = call(
            &t,
            Mode::AcceptEdits,
            1,
            &AcceptAll,
            "write_c_version",
            json!({
                "routine": "$00:8000", "name": "Plain words", "reason": "r",
                "text": "void Reset(void)\n{\n    screen_off();\n}\n",
                "anchors": [{"first": 3, "last": 3, "start": "$00:8007", "end": "$00:800B"}],
            }),
        );
        assert!(!o.is_error, "{}", text(&o));
        let v = wb.c_versions(Some(0x8000));
        assert_eq!(v[0].version.author, crate::cnotes::CAuthor::Tutor);
        assert_eq!(v[0].version.anchors[0].start, 0x8007);
        let (o, _) = call(
            &t,
            Mode::AcceptEdits,
            1,
            &AcceptAll,
            "define_variable",
            json!({
                "address": "$7E:0010", "name": "Product", "width": "word", "count": 1, "reason": "r",
            }),
        );
        assert!(!o.is_error, "{}", text(&o));
        assert_eq!(wb.variables()[0].name, "Product");
        let (o, _) = call(
            &t,
            Mode::ReadOnly,
            1,
            &AcceptAll,
            "c_versions",
            json!({"routine": "$00:8000"}),
        );
        assert!(text(&o).contains("\"Plain words\""), "{}", text(&o));
    }
}
