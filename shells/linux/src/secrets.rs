//! Where the tutor's keys are kept (docs/24, decision 4): one secret per
//! endpoint in the desktop's Secret Service (GNOME Keyring, KWallet), never in
//! the settings file or a project. The macOS twin is `Keychain`.

use std::collections::HashMap;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

use romlens_ffi::tutor::session::CredentialStore;

pub trait KeyStore: Send + Sync {
    fn key(&self, endpoint: &str) -> Option<String>;
    /// Keep `key` for `endpoint`, or with `None` (or only blanks) remove it.
    fn set_key(&self, endpoint: &str, key: Option<&str>) -> Result<(), String>;
}

/// The Secret Service, through libsecret: the same schema name as the macOS
/// Keychain's service, one attribute naming the endpoint.
#[cfg_attr(test, allow(dead_code))]
pub struct SecretService;

#[cfg_attr(test, allow(dead_code))]
const SCHEMA: &str = "io.github.ayodehi.Romlens.tutor";

#[cfg_attr(test, allow(dead_code))]
fn schema() -> libsecret::Schema {
    libsecret::Schema::new(
        SCHEMA,
        libsecret::SchemaFlags::NONE,
        HashMap::from([("endpoint", libsecret::SchemaAttributeType::String)]),
    )
}

impl KeyStore for SecretService {
    fn key(&self, endpoint: &str) -> Option<String> {
        libsecret::password_lookup_sync(
            Some(&schema()),
            HashMap::from([("endpoint", endpoint)]),
            gtk::gio::Cancellable::NONE,
        )
        .ok()
        .flatten()
        .map(|k| k.to_string())
        .filter(|k| !k.is_empty())
    }

    fn set_key(&self, endpoint: &str, key: Option<&str>) -> Result<(), String> {
        let attributes = || HashMap::from([("endpoint", endpoint)]);
        libsecret::password_clear_sync(Some(&schema()), attributes(), gtk::gio::Cancellable::NONE)
            .map_err(|e| e.to_string())?;
        let Some(key) = key.map(str::trim).filter(|k| !k.is_empty()) else {
            return Ok(());
        };
        libsecret::password_store_sync(
            Some(&schema()),
            attributes(),
            Some(libsecret::COLLECTION_DEFAULT),
            &format!("Romlens tutor: {endpoint}"),
            key,
            gtk::gio::Cancellable::NONE,
        )
        .map_err(|e| e.to_string())
    }
}

/// Keys in memory, for tests.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryKeyStore(Mutex<HashMap<String, String>>);

#[cfg(test)]
impl KeyStore for MemoryKeyStore {
    fn key(&self, endpoint: &str) -> Option<String> {
        self.0.lock().unwrap().get(endpoint).cloned()
    }

    fn set_key(&self, endpoint: &str, key: Option<&str>) -> Result<(), String> {
        let mut map = self.0.lock().unwrap();
        match key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => map.insert(endpoint.to_owned(), k.to_owned()),
            None => map.remove(endpoint),
        };
        Ok(())
    }
}

/// What the core asks for a key through, from the turn's thread.
pub struct TutorCredentials(pub Arc<dyn KeyStore>);

impl CredentialStore for TutorCredentials {
    fn key(&self, endpoint: String) -> Option<String> {
        self.0.key(&endpoint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_kept_per_endpoint_trimmed_and_blank_removes_it() {
        let s = MemoryKeyStore::default();
        assert_eq!(s.key("anthropic"), None);
        s.set_key("anthropic", Some("  sk-test \n")).unwrap();
        s.set_key("openai", Some("other")).unwrap();
        assert_eq!(s.key("anthropic").as_deref(), Some("sk-test"));
        assert_eq!(s.key("openai").as_deref(), Some("other"));
        s.set_key("anthropic", Some("   ")).unwrap();
        assert_eq!(s.key("anthropic"), None, "blank removes");
        s.set_key("openai", None).unwrap();
        assert_eq!(s.key("openai"), None);
    }

    #[test]
    fn the_core_reads_keys_through_the_store() {
        let s = Arc::new(MemoryKeyStore::default());
        s.set_key("x", Some("k")).unwrap();
        let c = TutorCredentials(s);
        assert_eq!(c.key("x".into()).as_deref(), Some("k"));
        assert_eq!(c.key("y".into()), None);
    }
}
