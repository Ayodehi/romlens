//! `romlens tutor` (docs/24): the tutor from the command line, with keys
//! from the environment (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or
//! `ROMLENS_KEY_<ENDPOINT>` for one of your own).

use std::io::Write;
use std::path::Path;

use anyhow::{Result, anyhow};
use romlens_ffi::tutor::{digest::digest, tools::RomTools};
use romlens_ffi::workbench::read_project_package;
use romlens_ffi::{Rom, Workbench};
use romlens_tutor::agent::{
    Approver, Credentials, Decision, Deps, EnvCredentials, Event, Mode, Proposal, Session,
};
use romlens_tutor::http::{Cancel, UreqTransport, list_models};
use romlens_tutor::lesson::LessonStore;
use romlens_tutor::models;
use romlens_tutor::prompt;
use romlens_tutor::provider::{Delta, Endpoint, Protocol};
use romlens_tutor::transcript::{Block, ImageRef, Turn};

/// Which endpoint: `anthropic`, `openai`, or a name of your own with its
/// base URL and protocol.
pub struct Where<'a> {
    pub provider: &'a str,
    pub base_url: Option<&'a str>,
    pub responses: bool,
}

pub fn endpoint(w: &Where) -> Result<Endpoint> {
    let mut e = match (w.provider, w.base_url) {
        ("anthropic", None) => Endpoint::anthropic(),
        ("openai", None) => Endpoint::openai(),
        (id, Some(url)) => Endpoint {
            id: id.into(),
            protocol: if w.responses {
                Protocol::Responses
            } else {
                Protocol::Chat
            },
            base_url: url.into(),
            tool_choice: false,
            strict: false,
            vision: true,
        },
        (id, None) => {
            return Err(anyhow!(
                "`{id}` needs --base-url (for example http://localhost:11434/v1)"
            ));
        }
    };
    if let Some(url) = w.base_url
        && matches!(w.provider, "anthropic" | "openai")
    {
        e.base_url = url.into();
    }
    Ok(e)
}

pub fn models_list(w: &Where) -> Result<()> {
    let e = endpoint(w)?;
    let key = EnvCredentials.key(&e.id);
    let ids = list_models(&UreqTransport::new(), &e, key.as_deref())?;
    for id in ids {
        match models::model(&id) {
            Some(m) => println!(
                "{id}  {}  context {}  ${}/${} per M tokens",
                m.name, m.context, m.price.input, m.price.output
            ),
            None => println!("{id}"),
        }
    }
    Ok(())
}

pub struct Ask<'a> {
    pub rom: &'a Path,
    pub project: Option<&'a Path>,
    /// What the question is about: an address, sent with a window of the
    /// listing.
    pub at: Option<&'a str>,
    pub question: &'a str,
    /// A recording of the ROM, for the frame and sound tools.
    pub rec: Option<&'a Path>,
    /// Pictures to send with the question (PNG, JPEG, GIF or WebP).
    pub attach: &'a [std::path::PathBuf],
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub cap: Option<f64>,
    /// `read-only`, `ask` (each edit asked on the terminal) or `accept`.
    pub mode: &'a str,
    /// Answer with a lesson (docs/25).
    pub explain: bool,
    /// Leave the lessons it finishes unchecked.
    pub no_check: bool,
}

/// Asks about each edit on the terminal.
struct Terminal;

impl Approver for Terminal {
    fn decide(&self, p: &Proposal) -> Decision {
        let mut err = std::io::stderr();
        let _ = writeln!(err, "\n┌ The tutor wants to: {}", p.summary);
        if !p.reason.is_empty() {
            let _ = writeln!(err, "│ because {}", p.reason);
        }
        if let Some(b) = &p.before {
            let _ = writeln!(err, "│ now:   {b}");
        }
        if let Some(a) = &p.after {
            let _ = writeln!(err, "│ after: {a}");
        }
        let _ = write!(err, "└ Accept? [y/N] ");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if line.trim().eq_ignore_ascii_case("y") || line.trim().eq_ignore_ascii_case("yes") {
            Decision::Accept
        } else {
            Decision::Reject { why: None }
        }
    }
}

pub fn workbench(rom: &Path, project: Option<&Path>) -> Result<std::sync::Arc<Workbench>> {
    let rom = Rom::open(rom.to_string_lossy().into_owned())?;
    let wb = match project {
        Some(p) => Workbench::with_project_files(
            rom,
            read_project_package(p.to_string_lossy().into_owned())?,
        )?,
        None => Workbench::new(rom),
    };
    wb.analyze_blocking()?;
    Ok(wb)
}

/// The selection as the app sends it: where, and the listing there.
fn selection(wb: &Workbench, at: &str) -> Result<String> {
    let r = wb.resolve_any(at.to_owned())?;
    let mut s = romlens_ffi::tutor::tools::addr(r.snes_address);
    if let Some(l) = wb.label_at(r.snes_address) {
        s.push_str(&format!(" ({})", l.name));
    }
    if let Some(line) = r.file_offset.and_then(|o| wb.line_for_offset(o)) {
        let from = line.saturating_sub(4);
        s.push('\n');
        s.push_str(&wb.asm_lines_text(from, 16, romlens_ffi::records::AddressStyle::Snes));
    }
    Ok(s)
}

pub fn ask(w: &Where, a: &Ask) -> Result<()> {
    let wb = workbench(a.rom, a.project)?;
    let e = endpoint(w)?;
    let model = match a.model {
        Some(m) => m.to_owned(),
        None => models::default_model(e.protocol)
            .ok_or_else(|| anyhow!("name the model with --model"))?
            .to_owned(),
    };
    let mut s = Session::new(&format!("cli-{}", wb.rom_identity().sha256), e, &model);
    s.effort = a.effort.map(str::to_owned);
    s.cost_cap = a.cap;
    s.mode = match a.mode {
        "accept" => Mode::AcceptEdits,
        "ask" => Mode::AskBeforeEdits,
        "read-only" => Mode::ReadOnly,
        other => return Err(anyhow!("--mode is read-only, ask or accept, not {other}")),
    };
    if s.mode != Mode::ReadOnly && a.project.is_none() {
        return Err(anyhow!("edits need --project, where they are saved"));
    }
    s.system = prompt::system();
    let lessons = romlens_tutor::review::default_root().map(|r| LessonStore::new(&r));
    let learner = lessons
        .as_ref()
        .map(|l| l.learner().summary())
        .unwrap_or_default();
    s.digest = digest(&wb) + &prompt::learner_section(&learner);
    s.explain = a.explain;
    let sel = a.at.map(|at| selection(&wb, at)).transpose()?;
    let explain = a.explain.then_some((true, learner.as_str()));
    let mut text = prompt::context(Some(s.mode), explain, sel.as_deref()).unwrap_or_default();
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(a.question);
    let tools = RomTools::new(wb.clone());
    tools.lessons.set_store(lessons);
    if let Some(r) = a.rec {
        let rec = romlens_ffi::RecordingSession::open(r.to_string_lossy().into_owned(), false)?;
        rec.check_rom(wb.rom())?;
        tools.set_recording(Some(rec));
    }
    let mut blocks = Vec::new();
    for (i, p) in a.attach.iter().enumerate() {
        let media_type = match p
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            _ => {
                return Err(anyhow!(
                    "{}: attach a PNG, JPEG, GIF or WebP picture",
                    p.display()
                ));
            }
        };
        let id = format!("attached-{i}");
        s.pictures.insert(id.clone(), std::fs::read(p)?);
        blocks.push(Block::Image {
            image: ImageRef {
                id,
                media_type: media_type.into(),
            },
        });
    }
    blocks.push(Block::Text { text: text.clone() });
    let on = |ev: Event| {
        let mut err = std::io::stderr();
        match ev {
            Event::Delta(Delta::Text(t)) => {
                print!("{t}");
                let _ = std::io::stdout().flush();
            }
            Event::Delta(Delta::Reasoning(t)) => {
                let _ = write!(err, "{t}");
            }
            Event::ToolStarted { name, input, .. } => {
                let _ = writeln!(err, "\n→ {name} {input}");
            }
            Event::ToolFinished {
                summary, is_error, ..
            } => {
                let _ = writeln!(err, "  {} {summary}", if is_error { "✗" } else { "←" });
            }
            Event::Retrying { wait, why, .. } => {
                let _ = writeln!(err, "\n(retrying in {}s: {why})", wait.as_secs());
            }
            Event::Discarded => {
                let _ = writeln!(err, "\n(that reply was cut off; asking again)");
            }
            Event::Cost { total, priced, .. } if priced => {
                let _ = writeln!(err, "\n(${total:.4} so far)");
            }
            _ => {}
        }
    };
    let cancel = Cancel::new();
    let stop = s.ask(
        Turn::user(blocks),
        &Deps {
            transport: &UreqTransport::new(),
            credentials: &EnvCredentials,
            tools: &tools,
            approver: &Terminal,
            events: &on,
            cancel: &cancel,
        },
    )?;
    println!();
    eprintln!("({model}, stopped: {stop:?})");
    // The lessons it finished, checked as the app checks them (docs/25).
    let reviews = tools.lessons.take_reviews();
    if !a.no_check
        && let Some(store) = tools.lessons.store()
    {
        let quiet = |_: Event| {};
        let d = Deps {
            transport: &UreqTransport::new(),
            credentials: &EnvCredentials,
            tools: &tools,
            approver: &Terminal,
            events: &quiet,
            cancel: &cancel,
        };
        for id in reviews {
            eprintln!("(checking lesson {id}…)");
            if let Some((changed, cost, note)) =
                romlens_ffi::tutor::session::check_lesson(&s, &id, &store, &tools, &wb, &d)
            {
                eprintln!("(checked {id}: {changed} step(s) rewritten, ${cost:.4}: {note})");
            }
        }
    }
    if let Some(p) = a.project
        && wb.is_dirty()
    {
        wb.save_project(p.to_string_lossy().into_owned())?;
        eprintln!("(the tutor's edits are saved in {})", p.display());
    }
    Ok(())
}

fn root(dir: Option<&Path>) -> Result<std::path::PathBuf> {
    dir.map(Path::to_path_buf)
        .or_else(romlens_tutor::review::default_root)
        .ok_or_else(|| anyhow!("no folder of conversations; name one with --dir"))
}

/// `romlens tutor sessions`.
pub fn sessions(dir: Option<&Path>) -> Result<()> {
    let root = root(dir)?;
    let all = romlens_tutor::review::all(&root);
    if all.is_empty() {
        println!("no conversations under {}", root.display());
    }
    for s in &all {
        println!("{}", romlens_tutor::review::line(s));
    }
    Ok(())
}

/// `romlens tutor lessons`.
pub fn lessons(dir: Option<&Path>) -> Result<()> {
    let store = LessonStore::new(&root(dir)?);
    let all = store.list();
    print!("{}", store.learner().summary());
    println!();
    for l in &all {
        println!("{}", romlens_tutor::review::lesson_line(l));
    }
    Ok(())
}

/// `romlens tutor lesson`.
pub fn lesson(which: &str, dir: Option<&Path>) -> Result<()> {
    let store = LessonStore::new(&root(dir)?);
    let l = romlens_tutor::review::find_lesson(&store, which).map_err(|e| anyhow!(e))?;
    println!("{}", l.describe());
    Ok(())
}

/// `romlens tutor show`.
pub fn show(which: &str, dir: Option<&Path>, full: bool, system: bool) -> Result<()> {
    use romlens_tutor::review;
    let root = root(dir)?;
    let s = review::find(&root, which).map_err(|e| anyhow!(e))?;
    let turns = review::load(&root, &s)?;
    let show = review::Show {
        result_lines: (!full).then_some(40),
        system,
    };
    print!("{}", review::render(&s, &turns, show));
    Ok(())
}
