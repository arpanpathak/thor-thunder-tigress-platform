//! thor-hammer-trainer: builds the instruction/response training set.
//!
//! The data pipeline lives in [`build`]. [`sources`] reads the individual
//! corpora, one module each; [`example`] and [`report`] hold the shared types,
//! and [`slop_flags`] scores what the result looks like. [`teacher`] reads the
//! conversations the teacher wrote: [`teacher::verify`] builds and checks
//! them, with [`teacher::languages`] for code that is not Rust,
//! [`teacher::pick`] queues real sections to write from, and
//! [`teacher::build`] is the `teacher` command.

pub mod build;
pub mod error;
pub mod example;
pub mod report;
pub mod slop_flags;
pub mod sources;
pub mod teacher;
