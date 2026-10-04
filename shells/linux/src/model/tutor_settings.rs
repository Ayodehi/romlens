//! The tutor's settings (docs/24, "Settings"): its endpoints, defaults and
//! switches. Everything but the keys is in
//! `$XDG_CONFIG_HOME/romlens/tutor.json`; the keys are in the Secret Service
//! (`secrets.rs`). The macOS twin is `TutorSettings`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use romlens_ffi::tutor::session::{
    TutorEndpointInfo, TutorMode, TutorModelInfo, TutorProtocol, tutor_default_endpoints,
    tutor_default_model, tutor_models,
};
use serde::{Deserialize, Serialize};

use crate::secrets::KeyStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Anthropic,
    Responses,
    Chat,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::Anthropic => "Anthropic Messages",
            Kind::Responses => "OpenAI Responses",
            Kind::Chat => "Chat Completions",
        }
    }

    pub fn protocol(self) -> TutorProtocol {
        match self {
            Kind::Anthropic => TutorProtocol::Anthropic,
            Kind::Responses => TutorProtocol::Responses,
            Kind::Chat => TutorProtocol::Chat,
        }
    }
}

/// An endpoint the tutor can talk to: Anthropic's and OpenAI's, which are
/// built in, and any the person adds (Ollama, LM Studio, LiteLLM...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub base_url: String,
    pub tool_choice: bool,
    pub strict: bool,
    pub vision: bool,
    /// Whether it needs a key (the two providers do; a local server may not).
    pub needs_key: bool,
    pub built_in: bool,
}

impl Endpoint {
    pub fn info(&self) -> TutorEndpointInfo {
        TutorEndpointInfo {
            id: self.id.clone(),
            protocol: self.kind.protocol(),
            base_url: self.base_url.clone(),
            tool_choice: self.tool_choice,
            strict: self.strict,
            vision: self.vision,
        }
    }

    pub fn built_ins() -> Vec<Endpoint> {
        tutor_default_endpoints()
            .into_iter()
            .map(|e| Endpoint {
                name: if e.id == "anthropic" {
                    "Anthropic"
                } else {
                    "OpenAI"
                }
                .to_owned(),
                kind: if e.protocol == TutorProtocol::Anthropic {
                    Kind::Anthropic
                } else {
                    Kind::Responses
                },
                id: e.id,
                base_url: e.base_url,
                tool_choice: e.tool_choice,
                strict: e.strict,
                vision: e.vision,
                needs_key: true,
                built_in: true,
            })
            .collect()
    }

    /// A new endpoint of the person's, with an id nothing else has.
    pub fn local(name: &str, base_url: &str, kind: Kind, taken: &[String]) -> Endpoint {
        let mut slug: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        slug = slug.trim_matches('-').to_owned();
        if slug.is_empty() || slug == "anthropic" || slug == "openai" {
            slug = "local".to_owned();
        }
        let mut id = slug.clone();
        let mut n = 2;
        while taken.contains(&id) {
            id = format!("{slug}-{n}");
            n += 1;
        }
        Endpoint {
            id,
            name: name.trim().to_owned(),
            kind,
            base_url: base_url.trim().to_owned(),
            tool_choice: false,
            strict: false,
            vision: false,
            needs_key: false,
            built_in: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModePreference {
    ReadOnly,
    AskBeforeEdits,
    AcceptEdits,
}

impl ModePreference {
    pub const ALL: [ModePreference; 3] = [
        ModePreference::ReadOnly,
        ModePreference::AskBeforeEdits,
        ModePreference::AcceptEdits,
    ];

    pub fn title(self) -> &'static str {
        match self {
            ModePreference::ReadOnly => "Read-only",
            ModePreference::AskBeforeEdits => "Ask before edits",
            ModePreference::AcceptEdits => "Accept edits",
        }
    }

    pub fn mode(self) -> TutorMode {
        match self {
            ModePreference::ReadOnly => TutorMode::ReadOnly,
            ModePreference::AskBeforeEdits => TutorMode::AskBeforeEdits,
            ModePreference::AcceptEdits => TutorMode::AcceptEdits,
        }
    }

    pub fn from_mode(m: TutorMode) -> Self {
        match m {
            TutorMode::ReadOnly => ModePreference::ReadOnly,
            TutorMode::AskBeforeEdits => ModePreference::AskBeforeEdits,
            TutorMode::AcceptEdits => ModePreference::AcceptEdits,
        }
    }

    /// Shift+Tab: the next mode, round.
    pub fn next(self) -> Self {
        match self {
            ModePreference::ReadOnly => ModePreference::AskBeforeEdits,
            ModePreference::AskBeforeEdits => ModePreference::AcceptEdits,
            ModePreference::AcceptEdits => ModePreference::ReadOnly,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    pub custom: Vec<Endpoint>,
    pub endpoint: String,
    /// The model last chosen for each endpoint.
    pub models: std::collections::BTreeMap<String, String>,
    pub efforts: std::collections::BTreeMap<String, String>,
    pub mode: ModePreference,
    pub cost_cap: Option<f64>,
    /// The thinking and the tool calls under each answer; off, only the
    /// answers, pictures and edit cards show.
    pub show_work: bool,
    /// Each finished lesson checked and corrected in the background.
    pub check_lessons: bool,
    /// A quiz gets up to two of the tutor's questions about the game, each
    /// checked by Romlens.
    pub tutor_quiz_questions: bool,
    /// Points, the rank, achievements and their banners. Off, the map still
    /// shows what is proven and due.
    pub show_progress: bool,
    /// Where `generate_image` draws: an endpoint that speaks OpenAI's Images
    /// API, or none.
    pub image_endpoint: Option<String>,
    pub image_model: String,
}

impl Default for Stored {
    fn default() -> Self {
        Self {
            custom: Vec::new(),
            endpoint: "anthropic".into(),
            models: Default::default(),
            efforts: Default::default(),
            mode: ModePreference::AskBeforeEdits,
            cost_cap: Some(5.0),
            show_work: false,
            check_lessons: true,
            tutor_quiz_questions: true,
            show_progress: true,
            image_endpoint: None,
            image_model: "gpt-image-1".into(),
        }
    }
}

pub struct TutorSettings {
    path: PathBuf,
    pub keys: Arc<dyn KeyStore>,
    stored: Stored,
}

impl TutorSettings {
    pub fn path() -> PathBuf {
        crate::config::config_dir().join("tutor.json")
    }

    pub fn load(keys: Arc<dyn KeyStore>) -> Self {
        Self::load_from(&Self::path(), keys)
    }

    /// Missing or unreadable files give the defaults: a setting is never
    /// worth refusing to start for.
    pub fn load_from(path: &Path, keys: Arc<dyn KeyStore>) -> Self {
        let stored = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            path: path.to_path_buf(),
            keys,
            stored,
        }
    }

    pub fn stored(&self) -> &Stored {
        &self.stored
    }

    fn save(&self) {
        // Failing to remember a switch is not worth an error dialog.
        let _ = (|| -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&self.stored)?)?;
            std::fs::rename(tmp, &self.path)
        })();
    }

    /// Change a switch and keep it.
    pub fn edit(&mut self, f: impl FnOnce(&mut Stored)) {
        f(&mut self.stored);
        self.save();
    }

    pub fn endpoints(&self) -> Vec<Endpoint> {
        let mut all = Endpoint::built_ins();
        all.extend(self.stored.custom.iter().cloned());
        all
    }

    pub fn endpoint(&self, id: &str) -> Option<Endpoint> {
        self.endpoints().into_iter().find(|e| e.id == id)
    }

    pub fn default_endpoint(&self) -> Endpoint {
        self.endpoint(&self.stored.endpoint)
            .unwrap_or_else(|| Endpoint::built_ins().remove(0))
    }

    /// The model for an endpoint: the one last chosen, or the table's default
    /// for its protocol.
    pub fn model(&self, e: &Endpoint) -> Option<String> {
        self.stored
            .models
            .get(&e.id)
            .cloned()
            .or_else(|| tutor_default_model(e.kind.protocol()))
    }

    pub fn effort(&self, e: &Endpoint) -> Option<String> {
        self.stored.efforts.get(&e.id).cloned()
    }

    pub fn add(&mut self, e: Endpoint) {
        self.edit(|s| s.custom.push(e));
    }

    pub fn update(&mut self, e: Endpoint) {
        self.edit(|s| {
            if let Some(slot) = s.custom.iter_mut().find(|c| c.id == e.id) {
                *slot = e;
            }
        });
    }

    pub fn remove(&mut self, id: &str) {
        let _ = self.keys.set_key(id, None);
        self.edit(|s| {
            s.custom.retain(|c| c.id != id);
            if s.endpoint == id {
                s.endpoint = "anthropic".into();
            }
            if s.image_endpoint.as_deref() == Some(id) {
                s.image_endpoint = None;
            }
        });
    }

    pub fn has_key(&self, e: &Endpoint) -> bool {
        self.keys.key(&e.id).is_some_and(|k| !k.is_empty())
    }

    /// Ready to ask: a key where one is needed.
    pub fn ready(&self, e: &Endpoint) -> bool {
        !e.needs_key || self.has_key(e)
    }

    /// The table's models for an endpoint's protocol; a local server's come
    /// from asking it.
    pub fn table_models(&self, e: &Endpoint) -> Vec<TutorModelInfo> {
        tutor_models()
            .into_iter()
            .filter(|m| m.protocol == e.kind.protocol())
            .collect()
    }

    /// Endpoints that can draw: OpenAI's, and the person's own that speak
    /// OpenAI's protocols (Anthropic's API makes no pictures).
    pub fn image_endpoints(&self) -> Vec<Endpoint> {
        self.endpoints()
            .into_iter()
            .filter(|e| e.kind != Kind::Anthropic)
            .collect()
    }

    pub fn image_endpoint(&self) -> Option<Endpoint> {
        self.stored
            .image_endpoint
            .as_deref()
            .and_then(|id| self.endpoint(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryKeyStore;

    fn settings(name: &str) -> (TutorSettings, PathBuf) {
        let dir = std::env::temp_dir().join(format!("romlens-tutor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tutor.json");
        (
            TutorSettings::load_from(&path, Arc::new(MemoryKeyStore::default())),
            path,
        )
    }

    #[test]
    fn the_two_providers_are_built_in_and_the_persons_come_after() {
        let (mut s, _) = settings("builtin");
        let ids: Vec<_> = s.endpoints().iter().map(|e| e.id.clone()).collect();
        assert_eq!(ids, ["anthropic", "openai"]);
        assert!(s.endpoints().iter().all(|e| e.built_in && e.needs_key));
        s.add(Endpoint::local(
            "Ollama",
            "http://localhost:11434/v1",
            Kind::Chat,
            &ids,
        ));
        assert_eq!(s.endpoints().last().unwrap().id, "ollama");
        assert!(
            !s.endpoint("ollama").unwrap().needs_key,
            "a local server needs no key"
        );
    }

    #[test]
    fn an_endpoint_id_is_a_slug_nothing_else_has() {
        let taken = [
            "anthropic".to_owned(),
            "openai".to_owned(),
            "ollama".to_owned(),
        ];
        let id = |n: &str| Endpoint::local(n, "http://x", Kind::Chat, &taken).id;
        assert_eq!(id("My GPU Box!"), "my-gpu-box");
        assert_eq!(id("Ollama"), "ollama-2", "taken: numbered");
        assert_eq!(id("OpenAI"), "local", "never a built-in's id");
        assert_eq!(id("???"), "local");
    }

    #[test]
    fn settings_round_trip_and_unknown_or_missing_keys_take_defaults() {
        let (mut s, path) = settings("round");
        assert_eq!(s.stored(), &Stored::default());
        s.edit(|s| {
            s.show_work = true;
            s.cost_cap = None;
            s.mode = ModePreference::AcceptEdits;
            s.models.insert("openai".into(), "gpt-x".into());
        });
        let again = TutorSettings::load_from(&path, Arc::new(MemoryKeyStore::default()));
        assert!(again.stored().show_work);
        assert_eq!(again.stored().cost_cap, None);
        assert_eq!(again.stored().mode, ModePreference::AcceptEdits);
        std::fs::write(&path, br#"{"show_work":true,"from_the_future":1}"#).unwrap();
        let sparse = TutorSettings::load_from(&path, Arc::new(MemoryKeyStore::default()));
        assert!(sparse.stored().show_work);
        assert!(
            sparse.stored().check_lessons,
            "a missing switch keeps its default"
        );
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(
            TutorSettings::load_from(&path, Arc::new(MemoryKeyStore::default())).stored(),
            &Stored::default()
        );
    }

    #[test]
    fn a_key_is_never_in_the_settings_file() {
        let (mut s, path) = settings("keys");
        let e = s.default_endpoint();
        assert!(!s.has_key(&e) && !s.ready(&e));
        s.keys.set_key(&e.id, Some("sk-secret-value")).unwrap();
        assert!(s.has_key(&e) && s.ready(&e));
        s.edit(|s| s.show_work = true);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("sk-secret-value"));
    }

    #[test]
    fn removing_an_endpoint_forgets_its_key_and_what_pointed_at_it() {
        let (mut s, _) = settings("remove");
        let e = Endpoint::local("Box", "http://b/v1", Kind::Responses, &[]);
        s.add(e.clone());
        s.keys.set_key(&e.id, Some("k")).unwrap();
        s.edit(|st| {
            st.endpoint = e.id.clone();
            st.image_endpoint = Some(e.id.clone());
        });
        assert_eq!(s.default_endpoint().id, "box");
        s.remove("box");
        assert!(s.endpoint("box").is_none());
        assert_eq!(s.keys.key("box"), None);
        assert_eq!(s.default_endpoint().id, "anthropic");
        assert!(s.image_endpoint().is_none());
    }

    #[test]
    fn the_model_is_the_last_chosen_or_the_tables_default_and_images_never_use_anthropic() {
        let (mut s, _) = settings("models");
        let a = s.endpoint("anthropic").unwrap();
        assert!(s.model(&a).is_some(), "the table has a default");
        s.edit(|st| {
            st.models.insert("anthropic".into(), "chosen".into());
        });
        assert_eq!(s.model(&a).as_deref(), Some("chosen"));
        assert!(!s.table_models(&a).is_empty());
        assert!(
            s.image_endpoints()
                .iter()
                .all(|e| e.kind != Kind::Anthropic)
        );
    }

    #[test]
    fn shift_tab_cycles_the_three_modes() {
        let m = ModePreference::ReadOnly;
        assert_eq!(m.next(), ModePreference::AskBeforeEdits);
        assert_eq!(m.next().next(), ModePreference::AcceptEdits);
        assert_eq!(m.next().next().next(), m);
        for m in ModePreference::ALL {
            assert_eq!(ModePreference::from_mode(m.mode()), m);
        }
    }
}
