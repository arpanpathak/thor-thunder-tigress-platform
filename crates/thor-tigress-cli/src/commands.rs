//! `thor engines`, `thor list`, `thor stop` and `thor logs`.

use std::{iter, process::Command};

use inquire::Select;

use crate::{
    catalog::{self, Candidate},
    engine::Engine,
    error::{Outcome, ThorError},
    docker,
    machine::Machine,
    memory::{self, gigabytes, or_unknown, tokens},
    serve::confirm,
    service::{self, UnitState},
};

/// The width explanations are wrapped to.
const TEXT_WIDTH: usize = 88;

/// Prints each engine: what it is for, whether it is installed, where its files live, and
/// a command that serves a model on it.
pub fn engines(machine: &Machine) {
    for engine in Engine::MENU {
        let heading = format!(
            "{} (--engine {})\n  {}",
            engine.title(),
            engine.flag(),
            engine.installed(&machine.settings, &machine.search_path)
        );
        let details: Vec<String> = wrap(engine.explanation(), TEXT_WIDTH)
            .into_iter()
            .chain(storage(machine, engine))
            .chain(iter::once(format!("example: {}", engine.example())))
            .collect();

        println!("{heading}\n  {}\n", details.join("\n  "));
    }
}

/// Where an engine keeps its files, and the service that runs it.
fn storage(machine: &Machine, engine: Engine) -> [String; 2] {
    let root = machine.models_root(engine).display().to_string();
    let files = match engine {
        Engine::EdgeLlm => {
            format!(
                "checkpoints: {root}/FOLDER; drafters: {root}/drafters/FOLDER; engine cache: {root}/cache"
            )
        }
        Engine::LlamaCpp => format!("GGUF files: {root}/FOLDER/FILE.gguf"),
        Engine::Vllm => format!("downloads: the Hugging Face cache, {root}"),
    };
    let service = format!(
        "service: {} on port {}",
        engine.service(),
        machine.settings.value(engine.port_key())
    );

    [files, service]
}

/// Splits text into lines of about `width` characters, at spaces.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();

    for word in text.split_whitespace() {
        if line.len() + word.len() > width {
            lines.push(line.trim_end().to_string());
            line.clear();
        }

        line.push_str(word);
        line.push(' ');
    }

    lines.push(line.trim_end().to_string());

    lines
}

/// Prints memory, the `thor-*` services, and every model on disk with its cost.
pub fn list(machine: &Machine) {
    let meminfo = machine.meminfo();

    println!(
        "Memory: {} available of {}\n\nServices",
        or_unknown(memory::available_bytes(&meminfo), gigabytes),
        or_unknown(memory::total_bytes(&meminfo), gigabytes)
    );
    print_services(machine);

    println!("\nOn disk (cost = weights + KV cache at the longest context)");

    let rows = Engine::MENU.into_iter().flat_map(|engine| {
        catalog::on_disk(engine, &machine.models_root(engine))
            .into_iter()
            .map(|candidate| disk_row(machine, &candidate))
    });

    for row in rows {
        println!("{row}");
    }
}

/// Prints one row per `thor-*` service.
fn print_services(machine: &Machine) {
    let services = machine.thor_services();

    if services.is_empty() {
        println!("  none found by systemctl --user list-units 'thor-*'");
    }

    for state in &services {
        println!("{}", service_row(machine, state));
    }
}

/// One service row: name, state, memory, and the models its server lists.
fn service_row(machine: &Machine, state: &UnitState) -> String {
    let engine = Engine::MENU
        .into_iter()
        .find(|engine| engine.service() == state.name);
    let serving = engine
        .filter(|_engine| state.is_running())
        .map(|engine| service::served_models(&machine.settings.value(engine.port_key())).join(", "))
        .unwrap_or_default();

    let container = if state.container.is_some() {
        "(Docker container)"
    } else {
        ""
    };

    format!(
        "  {:<34} {:<11} {:>9} {container} {serving}",
        state.name,
        state.active_state,
        or_unknown(state.held_memory(), gigabytes)
    )
}

/// One model row: engine, name, weights, longest context, cost, and whether the settings
/// name it.
fn disk_row(machine: &Machine, candidate: &Candidate) -> String {
    let slots = candidate.engine.slots(&machine.settings);
    let max_context = candidate.max_context();
    let cost = max_context.and_then(|context| candidate.cost(context, slots));
    let key = candidate.engine.model_key();
    let note = if candidate.is_named_by(&machine.settings.value(key)) {
        format!("in the settings ({key})")
    } else {
        String::new()
    };

    format!(
        "  {:<10} {:<46} {:>9} {:>9} ctx {:>9}  {note}",
        candidate.engine.flag(),
        candidate.name(),
        or_unknown(candidate.weights, gigabytes),
        or_unknown(max_context, tokens),
        or_unknown(cost, gigabytes),
    )
}

/// Stops a service and, for vLLM, its Docker container, then reports the memory freed.
/// A service named on the command line is stopped without a question, so scripts and
/// `ssh thor thor stop NAME` work without a terminal; a service chosen from the menu is
/// confirmed first.
pub fn stop(machine: &Machine, name: Option<String>, yes: bool) -> Outcome<()> {
    let named = name.is_some();
    let units = service::thor_units()?;
    let unit = choose_unit(units, name, "Stop which service?")?;

    confirm(
        yes || named,
        &format!("Stop {unit}? Everything it serves stops answering."),
    )?;

    let before = machine.available();
    service::systemctl(&["stop", &unit])?;
    stop_container(machine, &unit)?;
    let after = machine.available();

    println!(
        "Stopped {unit}. Freed {}; {} available now.",
        gigabytes(after.saturating_sub(before)),
        gigabytes(after)
    );

    Ok(())
}

/// Stops the vLLM service's container, which outlives the service: the service runs
/// `docker run`, and stopping it ends only that client.
fn stop_container(machine: &Machine, unit: &str) -> Outcome<()> {
    if unit != Engine::Vllm.service() {
        return Ok(());
    }

    let container = machine.settings.value("VLLM_CONTAINER");
    let stopped = docker::stop_container(&container)?;
    let note = if stopped {
        "stopped"
    } else {
        "was not running"
    };

    println!("Docker container {container} {note}.");

    Ok(())
}

/// Follows a service's log, starting with its last 100 lines.
pub fn logs(name: Option<String>) -> Outcome<()> {
    let units = service::thor_units()?;
    let unit = choose_unit(units, name, "Whose log?")?;

    Command::new("journalctl")
        .args(["--user", "-u", &unit, "-n", "100", "-f"])
        .status()
        .map_err(|source| ThorError::Program {
            program: "journalctl".to_string(),
            source,
        })?;

    Ok(())
}

/// The service a name gives, or one chosen from a menu when there is no name.
fn choose_unit(units: Vec<String>, name: Option<String>, question: &str) -> Outcome<String> {
    match name {
        Some(name) => find_unit(&units, &name),
        None => Ok(Select::new(question, units).prompt()?),
    }
}

/// The service a name means: the name itself, `thor-NAME`, or the service of the engine
/// called NAME (`edge-llm` means `thor-edge-llm`).
pub fn find_unit(units: &[String], name: &str) -> Outcome<String> {
    let prefixed = format!("thor-{name}");
    let engine_service = Engine::from_flag(name)
        .map(Engine::service)
        .unwrap_or_default();

    units
        .iter()
        .find(|unit| [name, prefixed.as_str(), engine_service].contains(&unit.as_str()))
        .map(String::to_string)
        .ok_or_else(|| {
            ThorError::Condition(format!(
                "no service {name:?}; the services are: {}",
                units.join(", ")
            ))
        })
}
