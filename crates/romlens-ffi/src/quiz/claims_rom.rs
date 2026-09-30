//! The claims about the open ROM and its analysis (docs/28, Q4).

use romlens_tutor::quiz::Claim;

use super::World;

pub fn check(_w: &World, c: &Claim) -> Result<(), String> {
    Err(format!(
        "Romlens can't check {} claims yet",
        serde_json::to_value(c)
            .ok()
            .and_then(|v| v["kind"].as_str().map(str::to_owned))
            .unwrap_or_default()
    ))
}
