//! thor-lasso-distiller: uses a teacher model, served by `trtllm-serve`, to
//! turn training data into conversations.
//!
//! [`conversations`] builds multi-turn conversations from book passages: the
//! model writes the questions, the book's own text is every answer.
//! [`client`] is the HTTP client for the server, and [`cli`] the command line.
//!
//! [`agent`] builds coding tasks from merged pull requests, runs openbatrangs
//! on them, scores the runs, and exports the ones that passed as training data.

pub mod agent;
pub mod cli;
pub mod client;
pub mod conversations;
pub mod error;
#[cfg(test)]
pub mod testing;
