//! The seal around the registry: a passphrase becomes a key, and the key
//! seals the bytes of the file.
//!
//! Argon2id stretches the passphrase, so guessing it is slow and needs memory.
//! XChaCha20-Poly1305 then encrypts and authenticates the file in one step, so
//! a changed byte is refused rather than read.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use zeroize::Zeroizing;

use crate::error::{KeyringError, Outcome};

/// Bytes of salt in the file, mixed into the key.
pub const SALT_LEN: usize = 16;

/// Bytes of nonce written with every seal; it must never repeat under one key.
pub const NONCE_LEN: usize = 24;

/// Bytes of key the passphrase is stretched into.
pub const KEY_LEN: usize = 32;

/// Argon2id memory, in 1 KiB blocks.
pub const MEMORY_KIB: u32 = 19 * 1024;

/// Argon2id passes over that memory.
pub const PASSES: u32 = 2;

/// A key, wiped when it is dropped.
pub type Key = Zeroizing<[u8; KEY_LEN]>;

/// Fills `bytes` from the operating system's random source.
///
/// # Errors
///
/// [`KeyringError::Random`] when the system has no random bytes to give.
pub fn random(bytes: &mut [u8]) -> Outcome<()> {
    getrandom::fill(bytes).map_err(KeyringError::Random)
}

/// Stretches `passphrase` with `salt` into the key the registry is sealed with.
///
/// # Errors
///
/// [`KeyringError::Kdf`] when Argon2id refuses the parameters or runs out of
/// memory.
pub fn derive_key(passphrase: &[u8], salt: &[u8]) -> Outcome<Key> {
    let params = Params::new(MEMORY_KIB, PASSES, 1, Some(KEY_LEN)).map_err(KeyringError::Kdf)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(passphrase, salt, key.as_mut())
        .map_err(KeyringError::Kdf)?;
    Ok(key)
}

/// The cipher for a 32-byte `key`.
///
/// # Errors
///
/// [`KeyringError::KeyLength`] when `key` is not [`KEY_LEN`] bytes.
pub fn cipher(key: &[u8]) -> Outcome<XChaCha20Poly1305> {
    XChaCha20Poly1305::new_from_slice(key).map_err(|_| KeyringError::KeyLength)
}

/// The 24-byte nonce in `bytes`.
///
/// # Errors
///
/// [`KeyringError::NonceLength`] when `bytes` is not [`NONCE_LEN`] long.
pub fn nonce(bytes: &[u8]) -> Outcome<XNonce> {
    XNonce::try_from(bytes).map_err(|_| KeyringError::NonceLength)
}

/// Seals `plaintext`, binding `aad` to it without encrypting it.
///
/// # Errors
///
/// [`KeyringError::KeyLength`] or [`KeyringError::NonceLength`] for a wrong
/// size, and [`KeyringError::Invalid`] when the cipher refuses the message.
pub fn seal(key: &[u8], nonce_bytes: &[u8], aad: &[u8], plaintext: &[u8]) -> Outcome<Vec<u8>> {
    let sealed = cipher(key)?.encrypt(
        &nonce(nonce_bytes)?,
        Payload {
            msg: plaintext,
            aad,
        },
    );
    sealed.map_err(|error| KeyringError::Invalid(error.to_string()))
}

/// Opens `ciphertext` that was sealed with `key` and `aad`.
///
/// # Errors
///
/// [`KeyringError::Sealed`] when the passphrase is wrong or the file was
/// changed, and the other errors of [`seal`].
pub fn open(
    key: &[u8],
    nonce_bytes: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Outcome<Zeroizing<Vec<u8>>> {
    let opened = cipher(key)?.decrypt(
        &nonce(nonce_bytes)?,
        Payload {
            msg: ciphertext,
            aad,
        },
    );
    opened.map(Zeroizing::new).map_err(|_| KeyringError::Sealed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Vec<u8> {
        vec![7u8; KEY_LEN]
    }

    #[test]
    fn random_fills_the_whole_buffer() -> Outcome<()> {
        let mut bytes = [0u8; 64];
        random(&mut bytes)?;
        assert!(bytes.iter().any(|byte| *byte != 0));
        Ok(())
    }

    #[test]
    fn a_passphrase_and_a_salt_always_give_the_same_key() -> Outcome<()> {
        let salt = [3u8; SALT_LEN];
        let once = derive_key(b"correct horse", &salt)?;
        let twice = derive_key(b"correct horse", &salt)?;
        let other = derive_key(b"correct horse", &[4u8; SALT_LEN])?;
        assert_eq!(once.as_ref(), twice.as_ref());
        assert_ne!(once.as_ref(), other.as_ref());
        Ok(())
    }

    #[test]
    fn the_cipher_and_the_nonce_are_the_size_they_must_be() {
        assert!(cipher(&key()).is_ok());
        assert!(matches!(cipher(b"short"), Err(KeyringError::KeyLength)));
        assert!(nonce(&[0u8; NONCE_LEN]).is_ok());
        assert!(matches!(nonce(b"short"), Err(KeyringError::NonceLength)));
    }

    #[test]
    fn what_is_sealed_comes_back_only_with_the_same_key_and_aad() -> Outcome<()> {
        let nonce_bytes = [9u8; NONCE_LEN];
        let sealed = seal(&key(), &nonce_bytes, b"thor", b"ada@example.com")?;
        assert_ne!(sealed, b"ada@example.com");
        let opened = open(&key(), &nonce_bytes, b"thor", &sealed)?;
        assert_eq!(opened.as_slice(), b"ada@example.com");
        assert!(matches!(
            open(&[8u8; KEY_LEN], &nonce_bytes, b"thor", &sealed),
            Err(KeyringError::Sealed)
        ));
        assert!(matches!(
            open(&key(), &nonce_bytes, b"other", &sealed),
            Err(KeyringError::Sealed)
        ));
        let mut changed = sealed.clone();
        changed.truncate(changed.len() - 1);
        assert!(matches!(
            open(&key(), &nonce_bytes, b"thor", &changed),
            Err(KeyringError::Sealed)
        ));
        Ok(())
    }

    #[test]
    fn a_seal_refuses_a_wrong_nonce_size() {
        assert!(matches!(
            seal(&key(), b"short", b"", b"x"),
            Err(KeyringError::NonceLength)
        ));
    }
}
