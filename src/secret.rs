//! Secrets kept in the settings file, such as map API keys. They are sealed
//! with ChaCha20-Poly1305 under a key derived from this machine's ID, so a
//! settings file copied to another machine doesn't carry them along.

use std::sync::OnceLock;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};

const PREFIX: &str = "sealed1:";

/// `plain` sealed for this machine, or `None` if no random nonce was available.
pub fn seal(plain: &str) -> Option<String> {
    seal_for(machine_id(), plain)
}

/// The text `sealed` holds, or `None` when it was sealed on another machine
/// or changed since.
pub fn open(sealed: &str) -> Option<String> {
    open_for(machine_id(), sealed)
}

/// True for text made by [`seal`], as opposed to a value saved in the clear.
pub fn is_sealed(text: &str) -> bool {
    text.starts_with(PREFIX)
}

fn machine_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        machine_uid::get()
            .ok()
            .or_else(|| std::env::var("COMPUTERNAME").ok())
            .or_else(|| std::env::var("HOSTNAME").ok())
            .unwrap_or_default()
    })
}

fn key(machine: &str) -> LessSafeKey {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, b"picasso settings");
    let prk = salt.extract(machine.as_bytes());
    let okm = prk
        .expand(&[b"picasso secret v1"], &CHACHA20_POLY1305)
        .expect("one ChaCha20 key is within HKDF's output limit");
    LessSafeKey::new(UnboundKey::from(okm))
}

fn seal_for(machine: &str, plain: &str) -> Option<String> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new().fill(&mut nonce).ok()?;
    let mut data = plain.as_bytes().to_vec();
    key(machine)
        .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(PREFIX), &mut data)
        .ok()?;
    let mut bytes = nonce.to_vec();
    bytes.extend(data);
    Some(format!("{PREFIX}{}", STANDARD.encode(bytes)))
}

fn open_for(machine: &str, sealed: &str) -> Option<String> {
    let bytes = STANDARD.decode(sealed.strip_prefix(PREFIX)?).ok()?;
    if bytes.len() < NONCE_LEN {
        return None;
    }
    let (nonce, data) = bytes.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;
    let mut data = data.to_vec();
    let plain = key(machine).open_in_place(nonce, Aad::from(PREFIX), &mut data).ok()?;
    String::from_utf8(plain.to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_only_on_the_machine_that_sealed() {
        let sealed = seal_for("machine-a", "abc123").unwrap();
        assert!(is_sealed(&sealed));
        assert!(!sealed.contains("abc123"));
        assert_eq!(open_for("machine-a", &sealed).as_deref(), Some("abc123"));
        assert_eq!(open_for("machine-b", &sealed), None);
        assert_ne!(seal_for("machine-a", "abc123").unwrap(), sealed, "each seal uses a fresh nonce");
    }

    #[test]
    fn rejects_changed_or_plain_text() {
        let sealed = seal_for("machine-a", "abc123").unwrap();
        let mut bytes = STANDARD.decode(sealed.strip_prefix(PREFIX).unwrap()).unwrap();
        bytes[NONCE_LEN] ^= 1;
        let changed = format!("{PREFIX}{}", STANDARD.encode(bytes));
        assert_eq!(open_for("machine-a", &changed), None);
        assert_eq!(open_for("machine-a", "abc123"), None);
        assert!(!is_sealed("abc123"));
        assert_eq!(open(&seal("on this machine").unwrap()).as_deref(), Some("on this machine"));
    }
}
