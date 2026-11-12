//! What can go wrong while the shipper starts or runs a round.

use thiserror::Error;

/// Every failure the shipper reports.
#[derive(Debug, Error)]
pub enum TelemetryError {
    /// An option was unknown, missing its value, or not a number.
    #[error("{0}")]
    Usage(String),
    /// A file could not be read or opened.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The events could not be written as JSON.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// OpenObserve could not be reached or refused the events.
    #[error("openobserve: {0}")]
    Send(String),
}

/// The result of anything that can fail with a [`TelemetryError`].
pub type Outcome<T = ()> = Result<T, TelemetryError>;
