//! What the model is told (docs/24, "What the model is told"): the system
//! prompt with its rules, then the primer, both fixed for every
//! conversation so they cache; and the few words each question carries
//! about the mode and the selection.

use crate::agent::Mode;

pub const SYSTEM: &str = include_str!("prompt/system.md");
pub const PRIMER: &str = include_str!("prompt/primer.md");

/// The system prompt a conversation starts with: the rules, the primer, and
/// the ladder and concept map lessons are taught by (docs/25).
pub fn system() -> String {
    format!("{SYSTEM}\n{PRIMER}\n{}", crate::lesson::concept_map_text())
}

/// What the student has learned, for the prefix a conversation starts with.
pub fn learner_section(summary: &str) -> String {
    format!("\n\n## The student's learning so far\n\n{summary}")
}

/// What goes before the student's words in a user turn: the mode when it
/// changed, Explain mode when it changed (with the learner record as it is
/// now, when it came on), and the selection when there is one. It is part
/// of the turn, so the prefix already sent is never rewritten.
pub fn context(
    mode_changed: Option<Mode>,
    explain_changed: Option<(bool, &str)>,
    selection: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(m) = mode_changed {
        parts.push(format!("[The mode is now: {}.]", m.name()));
    }
    match explain_changed {
        Some((true, learner)) => parts.push(format!(
            "[Explain mode is on: answer with a lesson.]\n[The student's learning so far:]\n{learner}"
        )),
        Some((false, _)) => parts.push("[Explain mode is off: answer as usual.]".into()),
        None => {}
    }
    if let Some(s) = selection.filter(|s| !s.trim().is_empty()) {
        parts.push(format!("[The student's selection in Romlens:]\n{s}"));
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_is_fixed_and_says_how_to_cite() {
        let s = system();
        assert_eq!(s, system());
        assert!(s.contains("`$BB:AAAA`"));
        assert!(s.contains("# The SNES, briefly"));
        assert!(s.contains("- `oam` OAM:"), "the concept map is in it");
        assert!(context(None, None, Some("  ")).is_none());
        let c = context(Some(Mode::ReadOnly), None, Some("$00:8000 SEI")).unwrap();
        assert!(c.starts_with("[The mode is now: read-only.]"));
        let c = context(None, Some((true, "- sprites 2")), None).unwrap();
        assert!(
            c.starts_with("[Explain mode is on: answer with a lesson.]"),
            "{c}"
        );
        assert!(c.ends_with("- sprites 2"), "{c}");
    }
}
