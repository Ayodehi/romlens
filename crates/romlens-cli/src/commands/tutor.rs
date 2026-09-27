//! `romlens tutor` (docs/24): the tutor from the command line, with keys
//! from the environment (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or
//! `ROMLENS_KEY_<ENDPOINT>` for one of your own).

use std::io::Write;
use std::path::Path;

use anyhow::{Result, anyhow};
use romlens_ffi::tutor::{digest::digest, tools::RomTools};
use romlens_ffi::workbench::read_project_package;
use romlens_ffi::{Rom, Workbench};
use romlens_tutor::agent::{Credentials, Deps, EnvCredentials, Event, Mode, Session};
use romlens_tutor::http::{Cancel, UreqTransport, list_models};
use romlens_tutor::models;
use romlens_tutor::prompt;
use romlens_tutor::provider::{Delta, Endpoint, Protocol};
use romlens_tutor::transcript::Turn;

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
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub cap: Option<f64>,
}

fn workbench(rom: &Path, project: Option<&Path>) -> Result<std::sync::Arc<Workbench>> {
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
    s.mode = Mode::ReadOnly;
    s.system = prompt::system();
    s.digest = digest(&wb);
    let sel = a.at.map(|at| selection(&wb, at)).transpose()?;
    let mut text = prompt::context(Some(s.mode), sel.as_deref()).unwrap_or_default();
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(a.question);
    let tools = RomTools::new(wb.clone());
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
        Turn::user_text(&text),
        &Deps {
            transport: &UreqTransport::new(),
            credentials: &EnvCredentials,
            tools: &tools,
            events: &on,
            cancel: &cancel,
        },
    )?;
    println!();
    eprintln!("({model}, stopped: {stop:?})");
    Ok(())
}
