//! `romlens tutor` (docs/24): the tutor from the command line, with keys
//! from the environment (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or
//! `ROMLENS_KEY_<ENDPOINT>` for one of your own).

use std::io::Write;
use std::path::Path;

use anyhow::{Result, anyhow};
use romlens_tutor::agent::{Credentials, Deps, EnvCredentials, Event, NoTools, Session};
use romlens_tutor::http::{Cancel, UreqTransport, list_models};
use romlens_tutor::models;
use romlens_tutor::provider::{Delta, Endpoint, Protocol};
use romlens_tutor::transcript::Turn;

use super::session::load_rom;

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
    pub question: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub cap: Option<f64>,
}

pub fn ask(w: &Where, a: &Ask) -> Result<()> {
    let rom = load_rom(a.rom)?;
    let e = endpoint(w)?;
    let model = match a.model {
        Some(m) => m.to_owned(),
        None => models::default_model(e.protocol)
            .ok_or_else(|| anyhow!("name the model with --model"))?
            .to_owned(),
    };
    let mut s = Session::new(&format!("cli-{}", rom.sha256_hex()), e, &model);
    s.effort = a.effort.map(str::to_owned);
    s.cost_cap = a.cap;
    s.system =
        "You are Romlens's tutor, teaching SNES development from a ROM the student has open."
            .into();
    s.digest = format!("The ROM: {}", rom.sha256_hex());
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
        Turn::user_text(a.question),
        &Deps {
            transport: &UreqTransport::new(),
            credentials: &EnvCredentials,
            tools: &NoTools,
            events: &on,
            cancel: &cancel,
        },
    )?;
    println!();
    eprintln!("({model}, stopped: {stop:?})");
    Ok(())
}
