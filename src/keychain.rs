//! Secrets kept in the system's own credential store: the macOS Keychain,
//! Windows Credential Manager, or the Secret Service on Linux. Where none
//! can be used, a secret is kept for this session only, in memory, and
//! the person is told so (`docs/TRAVEL.md`).
//!
//! Only a reference to each secret goes in the library, so a backup, an
//! export, or a copied library never carries it. Unlike `src/secret.rs`,
//! which seals map keys into the settings file, nothing here is written
//! to Picasso's own files.
//!
//! Publishing to a site uses it; until then only its tests do.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashMap;
use std::sync::Mutex;

/// The service the secrets are filed under in the system's store.
const SERVICE: &str = "Picasso WordPress";

/// Why a secret couldn't be read, kept, or removed.
#[derive(Clone, Debug, PartialEq)]
pub enum StoreError {
    /// No credential store is available, or it is locked and the person declined to unlock it.
    Unavailable,
    /// The store refused, with its own words.
    Failed(String),
}

/// A place secrets are kept, by reference.
pub trait Store: Send + Sync {
    fn save(&self, reference: &str, secret: &str) -> Result<(), StoreError>;
    /// `None` when there is no secret under `reference`.
    fn load(&self, reference: &str) -> Result<Option<String>, StoreError>;
    /// Removing a secret that isn't there succeeds.
    fn delete(&self, reference: &str) -> Result<(), StoreError>;
}

/// The system's credential store.
pub struct System;

impl System {
    fn entry(reference: &str) -> Result<keyring::Entry, StoreError> {
        keyring::Entry::new(SERVICE, reference).map_err(store_error)
    }
}

impl Store for System {
    fn save(&self, reference: &str, secret: &str) -> Result<(), StoreError> {
        System::entry(reference)?.set_password(secret).map_err(store_error)
    }

    fn load(&self, reference: &str) -> Result<Option<String>, StoreError> {
        match System::entry(reference)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(store_error(err)),
        }
    }

    fn delete(&self, reference: &str) -> Result<(), StoreError> {
        match System::entry(reference)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(store_error(err)),
        }
    }
}

fn store_error(err: keyring::Error) -> StoreError {
    match err {
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => StoreError::Unavailable,
        // The store's message names the problem, never the secret.
        other => StoreError::Failed(other.to_string()),
    }
}

/// Secrets for this session only, gone when Picasso quits.
#[derive(Default)]
pub struct Session {
    secrets: Mutex<HashMap<String, String>>,
}

impl Store for Session {
    fn save(&self, reference: &str, secret: &str) -> Result<(), StoreError> {
        self.secrets.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(reference.to_string(), secret.to_string());
        Ok(())
    }

    fn load(&self, reference: &str) -> Result<Option<String>, StoreError> {
        Ok(self.secrets.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(reference).cloned())
    }

    fn delete(&self, reference: &str) -> Result<(), StoreError> {
        self.secrets.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(reference);
        Ok(())
    }
}

/// Where a secret ended up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kept {
    /// In the system's store, for later sessions too.
    System,
    /// In memory, until Picasso quits, because the system's store couldn't be used.
    Session,
}

/// The system's store, with this session as the fallback. A secret read
/// from or saved to the system's store is also kept in memory until
/// Picasso quits, so macOS asks to unlock it at most once a session.
pub struct Vault {
    system: Box<dyn Store>,
    session: Session,
}

impl Default for Vault {
    fn default() -> Self {
        Vault::new(Box::new(System))
    }
}

impl Vault {
    pub fn new(system: Box<dyn Store>) -> Self {
        Vault { system, session: Session::default() }
    }

    /// Keeps `secret` in the system's store, or for this session when the
    /// store is unavailable. A store that refuses for another reason is
    /// an error to show, not a reason to keep the secret elsewhere.
    pub fn save(&self, reference: &str, secret: &str) -> Result<Kept, StoreError> {
        match self.system.save(reference, secret) {
            Ok(()) => {
                // Replaces an earlier fallback, and spares asking the store again.
                self.session.save(reference, secret)?;
                Ok(Kept::System)
            }
            Err(StoreError::Unavailable) => {
                self.session.save(reference, secret)?;
                Ok(Kept::Session)
            }
            Err(err) => Err(err),
        }
    }

    /// The secret under `reference`, from this session first; the system's
    /// store is asked only once.
    pub fn load(&self, reference: &str) -> Result<Option<String>, StoreError> {
        if let Some(secret) = self.session.load(reference)? {
            return Ok(Some(secret));
        }
        match self.system.load(reference) {
            Err(StoreError::Unavailable) => Ok(None),
            Ok(Some(secret)) => {
                self.session.save(reference, &secret)?;
                Ok(Some(secret))
            }
            other => other,
        }
    }

    /// Forgets the secret everywhere it is kept.
    pub fn delete(&self, reference: &str) -> Result<(), StoreError> {
        self.session.delete(reference)?;
        match self.system.delete(reference) {
            Err(StoreError::Unavailable) => Ok(()),
            other => other,
        }
    }
}

/// The reference a site's Application Password is kept under: its
/// origin and user name, never anything secret.
pub fn wordpress_reference(origin: &str, user: &str) -> String {
    format!("wordpress {user}@{origin}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A system store that is always unavailable, or refuses outright.
    struct Broken(StoreError);

    impl Store for Broken {
        fn save(&self, _: &str, _: &str) -> Result<(), StoreError> {
            Err(self.0.clone())
        }
        fn load(&self, _: &str) -> Result<Option<String>, StoreError> {
            Err(self.0.clone())
        }
        fn delete(&self, _: &str) -> Result<(), StoreError> {
            Err(self.0.clone())
        }
    }

    #[test]
    fn a_secret_goes_to_the_system_store_and_comes_back() {
        let vault = Vault::new(Box::new(Session::default()));
        assert_eq!(vault.save("site", "abcd efgh"), Ok(Kept::System));
        assert_eq!(vault.load("site"), Ok(Some("abcd efgh".to_string())));
        assert_eq!(vault.delete("site"), Ok(()));
        assert_eq!(vault.load("site"), Ok(None));
        assert_eq!(vault.delete("site"), Ok(()));
    }

    /// A system store that counts how often it is read.
    #[derive(Default)]
    struct Counted(Session, std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl Store for Counted {
        fn save(&self, reference: &str, secret: &str) -> Result<(), StoreError> {
            self.0.save(reference, secret)
        }
        fn load(&self, reference: &str) -> Result<Option<String>, StoreError> {
            self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.0.load(reference)
        }
        fn delete(&self, reference: &str) -> Result<(), StoreError> {
            self.0.delete(reference)
        }
    }

    #[test]
    fn the_system_store_is_asked_once_a_session() {
        let store = Counted::default();
        store.0.save("site", "abcd efgh").unwrap();
        let reads = store.1.clone();
        let vault = Vault::new(Box::new(store));
        for _ in 0..3 {
            assert_eq!(vault.load("site"), Ok(Some("abcd efgh".to_string())));
        }
        assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 1, "macOS asks to unlock it once");
        assert_eq!(vault.save("other", "ijkl"), Ok(Kept::System));
        assert_eq!(vault.load("other"), Ok(Some("ijkl".to_string())));
        assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 1, "a secret just saved isn't read back");
    }

    #[test]
    fn without_a_system_store_a_secret_lasts_the_session() {
        let vault = Vault::new(Box::new(Broken(StoreError::Unavailable)));
        assert_eq!(vault.save("site", "abcd efgh"), Ok(Kept::Session));
        assert_eq!(vault.load("site"), Ok(Some("abcd efgh".to_string())));
        assert_eq!(vault.delete("site"), Ok(()));
        assert_eq!(vault.load("site"), Ok(None));
    }

    #[test]
    fn a_store_that_refuses_is_an_error_not_a_fallback() {
        let vault = Vault::new(Box::new(Broken(StoreError::Failed("denied".into()))));
        assert_eq!(vault.save("site", "abcd efgh"), Err(StoreError::Failed("denied".into())));
        assert_eq!(vault.load("site"), Err(StoreError::Failed("denied".into())));
    }

    #[test]
    fn the_reference_names_the_account_and_site_only() {
        assert_eq!(wordpress_reference("https://example.com", "ana"), "wordpress ana@https://example.com");
    }

    #[test]
    fn store_errors_never_fall_back_for_a_bad_entry() {
        assert_eq!(store_error(keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("locked")))), StoreError::Unavailable);
        assert!(matches!(store_error(keyring::Error::TooLong("user".into(), 10)), StoreError::Failed(_)));
        let _ = System::entry("site");
    }
}
