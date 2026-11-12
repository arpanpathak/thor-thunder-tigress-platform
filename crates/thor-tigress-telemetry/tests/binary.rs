//! Runs the shipper as a program: arguments it cannot read end it with a
//! reason, and one round against an OpenObserve that is not there still exits.

use std::process::Command;

#[test]
fn exits_with_a_reason_or_after_one_round() -> anyhow::Result<()> {
    let program = env!("CARGO_BIN_EXE_thor-tigress-telemetry");
    let folder = tempfile::tempdir()?;
    let password = folder.path().join("password");

    std::fs::write(&password, "p4ss")?;

    let unknown = Command::new(program).args(["--colour", "red"]).output()?;
    let once = Command::new(program)
        .args([
            "--log",
            "/nonexistent",
            "--password-file",
            &password.display().to_string(),
        ])
        .args([
            "--openobserve",
            "http://127.0.0.1:1",
            "--agent",
            "127.0.0.1:1",
            "--rounds",
            "1",
        ])
        .output()?;

    assert!(!unknown.status.success());
    assert!(
        String::from_utf8_lossy(&unknown.stderr)
            .starts_with("thor-tigress-telemetry: unknown option")
    );
    assert!(once.status.success());
    assert!(String::from_utf8_lossy(&once.stderr).contains("openobserve:"));

    Ok(())
}
