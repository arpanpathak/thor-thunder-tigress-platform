//! Child processes: `gh` and `git` run to completion; the agent and the check
//! run under a time limit.

use std::{
    os::unix::process::CommandExt,
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

use crate::error::DistillError;

/// The interval between checks on a program with a time limit.
const POLL: Duration = Duration::from_millis(100);

/// The end state of a program run with a time limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// The program exited; the flag is true for a success status.
    Exited(bool),
    /// The program was running at the limit and was killed.
    TimedOut,
}

/// Runs `command` to completion and returns its standard output.
///
/// # Errors
///
/// [`DistillError::Io`] when it can't be started, and
/// [`DistillError::Command`] with its standard error when it fails.
pub fn output(command: &mut Command) -> Result<Vec<u8>, DistillError> {
    let program = command.get_program().to_string_lossy().into_owned();
    let result = command.output().map_err(DistillError::io(&program))?;

    if result.status.success() {
        return Ok(result.stdout);
    }

    Err(DistillError::Command(format!(
        "{program} failed: {}",
        String::from_utf8_lossy(&result.stderr).trim()
    )))
}

/// Runs `command` in a new process group and returns how it ended. When it
/// runs longer than `limit`, the whole group receives `SIGKILL`, which also
/// stops its children, such as `cargo` started by `sh`.
///
/// # Errors
///
/// [`DistillError::Io`] when it can't be started or waited for.
pub fn finish_within(command: &mut Command, limit: Duration) -> Result<Finish, DistillError> {
    let program = command.get_program().to_string_lossy().into_owned();
    let child = command
        .process_group(0)
        .spawn()
        .map_err(DistillError::io(&program))?;
    let deadline = Instant::now() + limit;
    let mut running = Running {
        child,
        program,
        deadline,
    };

    std::iter::repeat_with(|| running.poll())
        .find_map(Result::transpose)
        .unwrap_or(Ok(Finish::TimedOut))
}

/// A program started with a time limit.
struct Running {
    child: Child,
    program: String,
    deadline: Instant,
}

impl Running {
    /// Returns how the program ended, or `None` after sleeping for [`POLL`]
    /// when it is running and the deadline has not passed.
    fn poll(&mut self) -> Result<Option<Finish>, DistillError> {
        let status = self
            .child
            .try_wait()
            .map_err(DistillError::io(&self.program))?;

        match status {
            Some(status) => Ok(Some(Finish::Exited(status.success()))),
            None => self.stop_after_deadline(),
        }
    }

    /// Returns `None` after sleeping for [`POLL`] before the deadline; after
    /// it, kills the process group and returns [`Finish::TimedOut`].
    fn stop_after_deadline(&mut self) -> Result<Option<Finish>, DistillError> {
        if Instant::now() < self.deadline {
            thread::sleep(POLL);

            return Ok(None);
        }

        let group = format!("kill -KILL -{}", self.child.id());
        output(Command::new("sh").args(["-c", &group]))?;
        self.child.wait().map_err(DistillError::io(&self.program))?;

        Ok(Some(Finish::TimedOut))
    }
}
