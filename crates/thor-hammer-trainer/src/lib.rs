//! thor-hammer-trainer: builds the instruction/response training set.
//!
//! The data pipeline lives in [`build`]. `book`, `chat`, `readability` and
//! `code` read the individual corpora; `example` and `report` hold the shared
//! types, and `slop_flags` scores what the result looks like.

pub mod book;
pub mod build;
pub mod chat;
pub mod clever_vs_readable;
pub mod code;
pub mod corpus;
pub mod error;
pub mod example;
pub mod readability;
pub mod report;
pub mod slop_flags;
