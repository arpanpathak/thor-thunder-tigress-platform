//! Runs `thor-tigress-keyring` as a program: the whole life of a request, the
//! passphrase typed at the prompt, and the two ways it can end with a failure.

use std::{
    io::Write,
    path::Path,
    process::{Command, Output},
};

const PROGRAM: &str = env!("CARGO_BIN_EXE_thor-tigress-keyring");
const PASS: &str = "integration passphrase";

/// Runs the program on `keyring` with the passphrase in the environment.
fn run(keyring: &Path, arguments: &[&str]) -> anyhow::Result<Output> {
    let output = Command::new(PROGRAM)
        .arg("--keyring")
        .arg(keyring)
        .args(arguments)
        .env("THOR_KEYRING_PASSPHRASE", PASS)
        .output()?;

    Ok(output)
}

fn printed(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn complained(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn records_approves_and_revokes_a_person() -> anyhow::Result<()> {
    let folder = tempfile::tempdir()?;
    let keyring = folder.path().join("keyring");
    let init = run(&keyring, &["init"])?;
    let request = run(&keyring, &["request", "Ada Lovelace", "ada@example.com"])?;
    let approve = run(&keyring, &["approve", "ada@example.com"])?;
    let key = printed(&approve)
        .lines()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    let keys = run(&keyring, &["keys"])?;
    let export = run(&keyring, &["export"])?;
    let revoked = run(&keyring, &["revoke", "ada@example.com"])?;

    assert!(init.status.success(), "{}", complained(&init));
    assert!(request.status.success());
    assert!(printed(&request).contains("recorded Ada Lovelace"));
    assert!(approve.status.success());
    assert_eq!(key.len(), 48);
    assert!(printed(&keys).contains("active"));
    assert_eq!(printed(&export), format!("{key}\n"));
    assert!(revoked.status.success());

    Ok(())
}

#[test]
fn a_usage_error_exits_with_two_and_a_failed_command_with_one() -> anyhow::Result<()> {
    let folder = tempfile::tempdir()?;
    let keyring = folder.path().join("keyring");

    run(&keyring, &["init"])?;

    let bad = run(&keyring, &["nonsense"])?;
    let missing = run(&keyring, &["show", "nobody@example.com"])?;

    assert_eq!(bad.status.code(), Some(2));
    assert!(complained(&bad).starts_with("thor-tigress-keyring: "));
    assert_eq!(missing.status.code(), Some(1));
    assert!(complained(&missing).starts_with("thor-tigress-keyring: "));

    Ok(())
}

#[test]
fn the_passphrase_can_come_from_standard_input() -> anyhow::Result<()> {
    let folder = tempfile::tempdir()?;
    let keyring = folder.path().join("keyring").display().to_string();
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

    Ok(())
}
