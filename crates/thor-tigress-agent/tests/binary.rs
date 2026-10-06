//! Runs the agent as a program for the cases that end it: arguments it can't
//! read, and an address it can't listen on.

use std::{fmt, io, net::TcpListener, process::Command};

/// Why a test could not run the program.
#[derive(Debug)]
enum RunError {
    /// Starting the program or binding a port failed.
    Io(io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<io::Error> for RunError {
    fn from(error: io::Error) -> Self {
        RunError::Io(error)
    }
}

#[test]
fn exits_with_failure_and_a_reason() -> Result<(), RunError> {
    let program = env!("CARGO_BIN_EXE_thor-tigress-agent");
    let unknown = Command::new(program).arg("--colour").output()?;
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).starts_with("thor-tigress-agent: "));
    let taken = TcpListener::bind("127.0.0.1:0")?;
    let address = taken.local_addr()?.to_string();
    let busy = Command::new(program).args(["--listen", &address, "--key-file", "/nonexistent"]).output()?;
    assert!(!busy.status.success());
    assert!(String::from_utf8_lossy(&busy.stderr).starts_with("thor-tigress-agent: "));
    assert_eq!(RunError::Io(io::Error::other("x")).to_string(), "x");
    Ok(())
}
