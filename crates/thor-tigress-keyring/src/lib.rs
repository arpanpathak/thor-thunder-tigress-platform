//! thor-tigress-keyring: the encrypted registry of the people allowed to use
//! the Thor Tigress Cub, and the keys they were given.
//!
//! The registry is one file, [`store::Keyring`], encrypted with a passphrase:
//! Argon2id turns the passphrase into a key, and XChaCha20-Poly1305 seals the
//! JSON the file is made of. Nothing about a person or their key is stored in
//! the clear.
//!
//! [`store`] holds the records and the file format, [`crypto`] the passphrase
//! and the seal, and [`cli`] the command line.

pub mod cli;
pub mod crypto;
pub mod error;
pub mod store;
