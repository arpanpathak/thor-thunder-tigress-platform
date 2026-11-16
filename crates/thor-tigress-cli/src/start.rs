//! Carrying out a confirmed plan: download, write the settings, restart the service, and
//! wait until the server answers or fails.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Child,
};

use inquire::Confirm;

use crate::{
    catalog::Candidate,
    cli::ServeOptions,
    error::{Outcome, ThorError, file_error},
    hub::Hub,
    machine::{self, Machine, tilde},
    memory::{self, gigabytes},
    plan::{AGENT_SERVICE, Plan},
    service::{self, Readiness},
    settings::Settings,
};

/// The most error lines printed from a failed start.
const ERROR_LINES_SHOWN: usize = 20;

/// Downloads what is missing, writes the settings, restarts the service, and reports.
pub fn carry_out(
    machine: &Machine,
    hub: &Hub,
    candidate: &Candidate,
    plan: &Plan,
    options: &ServeOptions,
) -> Outcome<()> {
    download(hub, candidate, plan)?;

    let backup = write_settings(&machine.settings, plan)?;
    let unit = plan.engine.service();

    if service::unit_state(unit)?.load_state != "loaded" {
        println!(
            "{unit} is not installed as a service yet. The settings are written; create and start it with:"
        );
        println!("  thor-tigress-serve install");
        return Ok(());
    }

    let started = machine::seconds_now();
    let readiness = restart_and_wait(plan, started, options)?;

    match readiness {
        Readiness::Ready => announce_ready(machine, plan),
        Readiness::Failed | Readiness::TimedOut => {
            explain_failure(plan, readiness, started, backup.as_deref(), options.yes)
        }
    }
}

/// Downloads the model's files into the plan's folder, and saves the `config.json` that
/// was read for the memory estimate when the download has none (a GGUF repository's base
/// model's), so `thor list` can estimate it later.
fn download(hub: &Hub, candidate: &Candidate, plan: &Plan) -> Outcome<()> {
    let (Some(folder), Some(repo)) = (&plan.download_folder, &candidate.repo) else {
        return Ok(());
    };

    println!("Downloading {repo} into {}", tilde(folder, &plan.home));

    for file in &candidate.files {
        hub.download(repo, file, folder)?;
    }

    save_config(candidate, folder)
}

/// Writes `config.json` into the folder when it is missing and the candidate has one.
fn save_config(candidate: &Candidate, folder: &Path) -> Outcome<()> {
    let path = folder.join("config.json");
    let Some(config) = candidate.config.as_ref().filter(|_config| !path.exists()) else {
        return Ok(());
    };

    let text = serde_json::to_string_pretty(config).map_err(|source| ThorError::Json {
        what: path.display().to_string(),
        source,
    })?;

    fs::write(&path, text).map_err(file_error(&path))
}

/// Backs up the settings file, then writes the plan's settings into it.
fn write_settings(settings: &Settings, plan: &Plan) -> Outcome<Option<PathBuf>> {
    let mut updated = Settings::load(&settings.path, &settings.home)?;

    for (key, value) in &plan.changes {
        updated.set(key, value);
    }

    let backup = updated.save_with_backup(machine::seconds_now())?;

    match &backup {
        Some(backup) => println!(
            "Settings written to {}; the old file is {}",
            tilde(&settings.path, &settings.home),
            tilde(backup, &settings.home)
        ),
        None => println!(
            "Settings written to {}",
            tilde(&settings.path, &settings.home)
        ),
    }

    Ok(backup)
}

/// Restarts the engine's service with its log on the terminal, and waits.
fn restart_and_wait(plan: &Plan, started: u64, options: &ServeOptions) -> Outcome<Readiness> {
    let unit = plan.engine.service();
    let minutes = options.wait.as_secs() / 60;

    println!(
        "Restarting {unit}. Its log follows; waiting up to {minutes} minutes for {} on port {}.",
        plan.served_name, plan.port
    );

    let log = service::follow_log(unit, started)?;
    let readiness = service::systemctl(&["restart", unit]).and_then(|_output| {
        service::wait_until_ready(unit, &plan.port, &plan.served_name, options.wait)
    });
    stop_following(log);

    readiness
}

/// Stops the `journalctl -f` that shows the log.
fn stop_following(mut log: Child) {
    let stopped = log.kill().and_then(|()| log.wait());

    if let Err(error) = stopped {
        eprintln!("could not stop following the log: {error}");
    }
}

/// Restarts the chat page's service when it must learn the new name, then says so.
fn announce_ready(machine: &Machine, plan: &Plan) -> Outcome<()> {
    if plan.restart_agent {
        service::systemctl(&["restart", AGENT_SERVICE])?;
    }

    let available = memory::available_bytes(&machine.meminfo()).unwrap_or(0);

    println!(
        "\nReady: {} answers on 127.0.0.1:{}. {} of memory is available now.",
        plan.served_name,
        plan.port,
        gigabytes(available)
    );

    Ok(())
}

/// Prints the log's error lines and how to go back, and offers to go back now.
fn explain_failure(
    plan: &Plan,
    readiness: Readiness,
    started: u64,
    backup: Option<&Path>,
    yes: bool,
) -> Outcome<()> {
    let unit = plan.engine.service();
    let log = service::log_since(unit, started).unwrap_or_default();
    let errors = service::error_lines(&log);
    let shown = &errors[errors.len().saturating_sub(ERROR_LINES_SHOWN)..];

    println!(
        "\n{unit} {readiness}. The lines in its log that report errors:\n  {}\n\n{}",
        shown.join("\n  "),
        going_back(plan, backup)
    );

    if yes || !Confirm::new("Go back now?").with_default(true).prompt()? {
        return Err(ThorError::Condition(format!(
            "{} is not serving",
            plan.served_name
        )));
    }

    go_back(plan, backup)
}

/// The commands that undo the change.
pub fn going_back(plan: &Plan, backup: Option<&Path>) -> String {
    let unit = plan.engine.service();
    let settings = tilde(&plan.settings_path, &plan.home);

    match backup {
        Some(backup) => format!(
            "To go back:\n  cp {} {settings}\n  systemctl --user restart {unit}\nTo stop it from restarting: thor stop {unit}",
            tilde(backup, &plan.home)
        ),
        None => format!(
            "To go back: remove the lines above from {settings} (there was no file before), then\n  \
             systemctl --user restart {unit}\nTo stop it from restarting: thor stop {unit}"
        ),
    }
}

/// Puts the backup back and restarts the service.
fn go_back(plan: &Plan, backup: Option<&Path>) -> Outcome<()> {
    let unit = plan.engine.service();
    let Some(backup) = backup else {
        return Err(ThorError::Condition(format!(
            "there is no backup to restore; stop the service with: thor stop {unit}"
        )));
    };

    fs::copy(backup, &plan.settings_path).map_err(file_error(&plan.settings_path))?;
    service::systemctl(&["restart", unit])?;
    println!(
        "Restored {} and restarted {unit}.",
        tilde(&plan.settings_path, &plan.home)
    );

    Ok(())
}
