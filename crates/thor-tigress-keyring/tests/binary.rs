//! Runs `thor-tigress-keyring` as a program: the whole life of a request, the
//! passphrase typed at the prompt, and the two ways it can end with a failure.

use std::{fmt, fs, io, io::Write, process::Command};

/// Why a test could not run the program.
#[derive(Debug)]
enum RunError {
    /// Starting the program or a file operation failed.
    Io(io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<io::Error> for RunError {
    fn from(error: io::Error) -> Self {
        RunError::Io(error)
    }
}

const PROGRAM: &str = env!("CARGO_BIN_EXE_thor-tigress-keyring");
const PASS: &str = "integration passphrase";

#[test]
fn records_approves_and_revokes_a_person() -> Result<(), RunError> {
    let folder = std::env::temp_dir().join(format!("thor-keyring-bin-{}", std::process::id()));
    fs::create_dir_all(&folder)?;
    let keyring = folder.join("keyring").display().to_string();
    let run = |args: &[&str]| {
        Command::new(PROGRAM)
            .args(["--keyring", &keyring])
            .args(args)
            .env("THOR_KEYRING_PASSPHRASE", PASS)
            .output()
    };

    let init = run(&["init"])?;
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let request = run(&["request", "Ada Lovelace", "ada@example.com"])?;
    assert!(request.status.success());
    assert!(String::from_utf8_lossy(&request.stdout).contains("recorded Ada Lovelace"));

    let approve = run(&["approve", "ada@example.com"])?;
    assert!(approve.status.success());
    let key = String::from_utf8_lossy(&approve.stdout)
        .lines()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    assert_eq!(key.len(), 48);

    let keys = run(&["keys"])?;
    assert!(String::from_utf8_lossy(&keys.stdout).contains("active"));

    let export = run(&["export"])?;
    assert_eq!(String::from_utf8_lossy(&export.stdout), format!("{key}\n"));

    let revoked = run(&["revoke", "ada@example.com"])?;
    assert!(revoked.status.success());

    let bad = run(&["nonsense"])?;
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).starts_with("thor-tigress-keyring: "));

    let missing = run(&["show", "nobody@example.com"])?;
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).starts_with("thor-tigress-keyring: "));

    fs::remove_dir_all(&folder)?;
    Ok(())
}

#[test]
fn the_passphrase_can_come_from_standard_input() -> Result<(), RunError> {
    let folder = std::env::temp_dir().join(format!("thor-keyring-stdin-{}", std::process::id()));
    fs::create_dir_all(&folder)?;
    let keyring = folder.join("keyring").display().to_string();
    let mut child = Command::new(PROGRAM)
        .args(["--keyring", &keyring, "init"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut input) = child.stdin.take() {
        input.write_all(b"typed at the prompt\n")?;
    }
    let out = child.wait_with_output()?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let asked = String::from_utf8_lossy(&out.stderr);
    assert!(asked.starts_with("passphrase for "), "{asked}");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("created "), "{said}");
    fs::remove_dir_all(&folder)?;
    Ok(())
}
