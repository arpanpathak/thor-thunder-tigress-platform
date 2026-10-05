//! thor-lasso-distiller: uses a teacher model, served by `trtllm-serve`, to
//! turn training data into conversations.
//!
//! [`conversations`] builds multi-turn conversations from book passages: the
//! model writes the questions, the book's own text is every answer.
//! [`client`] is the HTTP client for the server.

pub mod client;
pub mod conversations;
pub mod error;
