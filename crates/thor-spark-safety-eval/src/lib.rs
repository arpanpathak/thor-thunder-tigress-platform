//! thor-spark-safety-eval: Stage 0 of the platform.
//!
//! Scores an answer two ways. [`prose::slop`] finds stock slop phrases (filler,
//! fake importance, hedging) in its prose. [`code::rules`] checks its Rust code
//! against the five Rust rules listed in the README, each rule only where it
//! applies. [`prose::claims`] finds prose that says the rules are followed when
//! the code breaks them. [`answer`] puts the three together for one answer and
//! [`report`] adds answers up into the rates a run is compared on.
//!
//! It does not compile code or run tests; a block can pass every rule and
//! still not build. [`generate`] collects a run from a model server, and
//! [`cli`] holds the `spark` commands.

pub mod answer;
pub mod cli;
pub mod code;
pub mod error;
pub mod generate;
pub mod prose;
pub mod report;
