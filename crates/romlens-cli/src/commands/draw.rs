//! `romlens draw` (docs/26): a diagram Romlens draws for the tutor, as SVG
//! or PNG, with what it shows; and `romlens draw check`, the checks the
//! tutor's own SVG must pass. A diagram is Romlens's drawing of the machine,
//! with no game graphics in it, so it may be written to a file.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use romlens_draw::kinds;

/// The spec: a file, `-` for stdin, or the JSON itself.
fn spec(arg: &str) -> Result<serde_json::Value> {
    let text = if arg == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else if arg.trim_start().starts_with('{') {
        arg.to_owned()
    } else {
        std::fs::read_to_string(arg)?
    };
    Ok(serde_json::from_str(&text)?)
}

fn write(out: &Path, svg: &str) -> Result<()> {
    match out.extension().and_then(|e| e.to_str()) {
        Some("svg") => std::fs::write(out, svg)?,
        Some("png") => {
            let p = romlens_draw::render(svg).map_err(|p| {
                anyhow!(
                    "the drawing fails its checks:\n{}",
                    romlens_draw::check::describe(&p)
                )
            })?;
            std::fs::write(out, p.png)?;
        }
        _ => return Err(anyhow!("--out is a .svg or a .png file")),
    }
    Ok(())
}

pub struct DrawArgs<'a> {
    pub rom: &'a Path,
    pub kind: &'a str,
    pub spec: &'a str,
    pub project: Option<&'a Path>,
    pub recording: Option<&'a Path>,
    pub out: Option<&'a Path>,
}

pub fn draw(a: DrawArgs) -> Result<()> {
    let wb = super::tutor::workbench(a.rom, a.project)?;
    let rec = match a.recording {
        Some(r) => {
            let rec = romlens_ffi::RecordingSession::open(r.to_string_lossy().into_owned(), false)?;
            rec.check_rom(wb.rom())?;
            Some(rec)
        }
        None => None,
    };
    // An address, or a label the project or the analysis has.
    let resolve = |t: &str| -> Result<u32, String> {
        let t = t.trim().trim_matches('`');
        if let Ok(r) = wb.resolve_any(t.to_owned()) {
            return Ok(r.snes_address);
        }
        wb.labels()
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(t))
            .map(|l| l.address)
            .ok_or_else(|| format!("`{t}` is neither an address nor a label"))
    };
    let source = romlens_ffi::tutor::draw::Draw {
        wb: &wb,
        rom: wb.rom(),
        rec,
        resolve: &resolve,
    };
    let d = kinds::draw(a.kind, &spec(a.spec)?, &source).map_err(|e| anyhow!(e))?;
    if let Err(p) = romlens_draw::render(&d.svg) {
        return Err(anyhow!(
            "the drawing fails its checks:\n{}",
            romlens_draw::check::describe(&p)
        ));
    }
    println!("{}", d.description);
    match a.out {
        Some(out) => write(out, &d.svg)?,
        None => print!("\n{}", d.svg),
    }
    Ok(())
}

pub fn check(svg: &PathBuf, out: Option<&Path>) -> Result<()> {
    let text = std::fs::read_to_string(svg)?;
    match romlens_draw::render(&text) {
        Ok(p) => {
            println!("passes: {} × {} pixels", p.width, p.height);
            if let Some(out) = out {
                write(out, &text)?;
            }
            Ok(())
        }
        Err(p) => Err(anyhow!("{}", romlens_draw::check::describe(&p))),
    }
}
