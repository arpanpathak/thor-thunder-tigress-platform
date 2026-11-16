//! `thor-tigress serve`: choose an engine, a model and a context, check that they fit, then
//! download, write the settings, restart the service and wait until it answers.
//!
//! Every question has a flag, so a script can answer them all and pass `--yes`.

use std::{fmt, iter};

use inquire::{Confirm, CustomType, Select, Text};

use crate::{
    catalog::{self, Candidate, Source},
    cli::{ContextChoice, ServeOptions},
    edge::BuildOutlook,
    engine::{Engine, Installed, engine_for},
    error::{Outcome, ThorError},
    hub::Hub,
    machine::{self, Machine},
    memory::{self, Budget, gigabytes, or_unknown, tokens},
    plan::{self, Plan},
    recipes,
    service::UnitState,
    start,
};

/// Rows shown at once in a menu.
const PAGE_SIZE: usize = 15;

/// What the columns of the model menu mean.
const MODEL_COLUMNS: &str = "name, weights, longest context, memory at that context, fit; ↑↓ to move, enter to choose, type to filter";

/// One row of a menu: the text shown and the value it stands for.
pub struct MenuItem<Value> {
    /// The text shown.
    pub label: String,
    /// What choosing the row gives back.
    pub value: Value,
}

impl<Value> fmt::Display for MenuItem<Value> {
    /// Writes the label.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

/// What a row of the model menu does.
enum ModelChoice {
    /// Serve this model.
    Model(Box<Candidate>),
    /// Ask for other words and search the Hub again.
    SearchAgain,
    /// Ask for a Hugging Face name or a folder.
    EnterName,
}

/// What every step of `thor-tigress serve` needs once the engine is chosen.
struct Session<'a> {
    /// The machine and its settings.
    machine: &'a Machine,
    /// The Hugging Face Hub.
    hub: &'a Hub,
    /// The chosen engine.
    engine: Engine,
    /// The memory there is for the model.
    budget: Budget,
    /// Replies the engine serves at once, each with its own KV cache.
    slots: u64,
    /// The command line.
    options: &'a ServeOptions,
}

/// Runs `thor-tigress serve`.
pub fn serve(machine: &Machine, options: &ServeOptions) -> Outcome<()> {
    let engine = choose_engine(machine, options.engine)?;
    check_installed(machine, engine)?;
    let hub = Hub::from_environment(&machine.home);
    let session = Session {
        machine,
        hub: &hub,
        engine,
        budget: machine.budget(engine),
        slots: engine.slots(&machine.settings),
        options,
    };

    let mut candidate = choose_model(&session)?;
    print_recipes(&hub, &candidate, options.docs);
    choose_files(&session, &mut candidate)?;

    let context = choose_context(&session, &candidate)?;
    check_window(&candidate, context, options.reply_tokens)?;
    check_fits(&session, &candidate, context)?;

    let plan = Plan::new(machine, &candidate, context, options.reply_tokens);
    plan.print(&candidate, &session.budget, session.slots);
    confirm(options.yes, "Go ahead?")?;

    start::carry_out(machine, &hub, &candidate, &plan, options)
}

/// The engine from `--engine`, or from a menu that says what each one is for.
fn choose_engine(machine: &Machine, wanted: Option<Engine>) -> Outcome<Engine> {
    if let Some(engine) = wanted {
        return Ok(engine);
    }

    let items = Engine::MENU
        .into_iter()
        .map(|engine| MenuItem {
            label: format!(
                "{:<10} {:<68} {}",
                engine.flag(),
                engine.summary(),
                engine
                    .installed(&machine.settings, &machine.search_path)
                    .short()
            ),
            value: engine,
        })
        .collect();

    Ok(Select::new("Engine:", items).prompt()?.value)
}

/// Checks that the engine's program is on this machine.
fn check_installed(machine: &Machine, engine: Engine) -> Outcome<()> {
    match engine.installed(&machine.settings, &machine.search_path) {
        Installed::Yes(_found) => Ok(()),
        Installed::No(missing) => Err(ThorError::Condition(format!(
            "{} is not installed here: {missing}. See thor-tigress engines.",
            engine.title()
        ))),
    }
}

/// The model from `--model`, or from the model menu.
fn choose_model(session: &Session<'_>) -> Outcome<Candidate> {
    match &session.options.model {
        Some(name) => resolve_name(session, name),
        None => model_menu(session, ""),
    }
}

/// The model menu: models on disk first, then a Hub search in the engine's format, then
/// "search for something else" and "enter a name or a folder".
fn model_menu(session: &Session<'_>, query: &str) -> Outcome<Candidate> {
    let items = model_items(session, query);
    let title = format!(
        "{} model ({} usable now):",
        session.engine.title(),
        gigabytes(session.budget.usable())
    );
    let chosen = Select::new(&title, items)
        .with_page_size(PAGE_SIZE)
        .with_help_message(MODEL_COLUMNS)
        .prompt()?;

    match chosen.value {
        ModelChoice::Model(candidate) => Ok(*candidate),
        ModelChoice::SearchAgain => {
            model_menu(session, &Text::new("Search Hugging Face for:").prompt()?)
        }
        ModelChoice::EnterName => resolve_name(
            session,
            &Text::new("Hugging Face name or local folder:").prompt()?,
        ),
    }
}

/// The rows of the model menu. A Hub result already on disk is shown once, as on disk.
fn model_items(session: &Session<'_>, query: &str) -> Vec<MenuItem<ModelChoice>> {
    let engine = session.engine;
    let wanted = query.to_lowercase();
    let on_disk: Vec<Candidate> = catalog::on_disk(engine, &session.machine.models_root(engine))
        .into_iter()
        .filter(|candidate| candidate.name().to_lowercase().contains(&wanted))
        .collect();
    let disk_names: Vec<String> = on_disk
        .iter()
        .map(|candidate| last_part(&candidate.name()))
        .collect();
    let from_hub = hub_candidates(session, query, &disk_names);

    on_disk
        .into_iter()
        .chain(from_hub)
        .map(|candidate| MenuItem {
            label: candidate_label(&candidate, &session.budget, session.slots),
            value: ModelChoice::Model(Box::new(candidate)),
        })
        .chain(extra_rows())
        .collect()
}

/// The Hub's results for a search, each read in full, leaving out the ones on disk.
fn hub_candidates(session: &Session<'_>, query: &str, disk_names: &[String]) -> Vec<Candidate> {
    let engine = session.engine;

    eprintln!(
        "Asking Hugging Face for {} models matching {query:?}…",
        engine.format_tag()
    );
    let found = session
        .hub
        .search(engine, query, session.options.limit)
        .unwrap_or_else(|error| {
            eprintln!("  the search failed: {error}");
            Vec::new()
        });

    found
        .into_iter()
        .filter(|model| !disk_names.contains(&last_part(&model.id)))
        .map(|model| catalog::from_hub(session.hub, engine, model))
        .collect()
}

/// The two rows every model menu ends with.
fn extra_rows() -> [MenuItem<ModelChoice>; 2] {
    [
        MenuItem {
            label: "search for something else".to_string(),
            value: ModelChoice::SearchAgain,
        },
        MenuItem {
            label: "enter a Hugging Face name or a local folder".to_string(),
            value: ModelChoice::EnterName,
        },
    ]
}

/// One model menu row: name, weights, longest context, cost at that context, fit, and
/// where it is.
pub fn candidate_label(candidate: &Candidate, budget: &Budget, slots: u64) -> String {
    let max_context = candidate.max_context();
    let cost = max_context.and_then(|context| candidate.cost(context, slots));
    let place = match candidate.source {
        Source::Disk(_) => "on disk".to_string(),
        Source::Hub => format!("{} downloads", candidate.downloads.unwrap_or(0)),
    };

    format!(
        "{:<44} {:>8} {:>9} ctx {:>8} at max  {:<21} {place}",
        candidate.name(),
        or_unknown(candidate.weights, gigabytes),
        or_unknown(max_context, tokens),
        or_unknown(cost, gigabytes),
        fit_word(budget, cost, candidate.weights),
    )
}

/// Whether a model fits: at its longest context, only with a smaller one, not at all, or
/// unknown. A truth table: each row is one answer.
pub fn fit_word(budget: &Budget, cost_at_max: Option<u64>, weights: Option<u64>) -> &'static str {
    let fits_at_max = cost_at_max.is_some_and(|bytes| budget.fits(bytes));
    let weights_fit = weights.is_some_and(|bytes| budget.fits(bytes));

    match (fits_at_max, weights_fit, weights.is_some()) {
        (true, _, _) => "fits at max",
        (false, true, _) => "fits, smaller context",
        (false, false, true) => "too big",
        (false, false, false) => "size unknown",
    }
}

/// The last part of a repository or path.
fn last_part(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_string()
}

/// A model named by hand: a folder or file on disk, else a Hugging Face repository.
/// A repository in a format the engine does not serve stops here, with the reason.
fn resolve_name(session: &Session<'_>, name: &str) -> Outcome<Candidate> {
    let path = machine::expand_home(name.trim(), &session.machine.home);

    if path.exists() {
        return Ok(catalog::from_disk(session.engine, &path, None));
    }

    eprintln!("Reading {name} from Hugging Face…");
    let model = session.hub.model(name.trim())?;
    let format = engine_for(&model.tags, &[]);

    if format.is_some_and(|format| !session.engine.can_serve(format)) {
        return Err(ThorError::Condition(format!(
            "{name} is not in {}'s format; its tags say {}",
            session.engine.title(),
            format.map_or_else(
                || "nothing".to_string(),
                |format| format!("--engine {format}")
            )
        )));
    }

    Ok(catalog::from_hub(session.hub, session.engine, model))
}

/// Prints the engine's recipes and documentation for the model.
/// Nothing is fetched with `--no-docs` or `HF_HUB_OFFLINE`.
fn print_recipes(hub: &Hub, candidate: &Candidate, wanted: bool) {
    let page = candidate.repo.as_deref().map(|repo| hub.page(repo));
    let lines = if wanted && !hub.offline {
        recipes::lines(candidate.engine, candidate, page)
    } else {
        vec!["not fetched (--no-docs or HF_HUB_OFFLINE)".to_string()]
    };

    println!(
        "\nDocumentation for {} on {}:\n  {}\n",
        candidate.name(),
        candidate.engine.title(),
        lines.join("\n  ")
    );
}

/// Narrows the files to download: one GGUF variant of a llama.cpp repository, and none for
/// vLLM, whose container downloads the model itself on first start.
fn choose_files(session: &Session<'_>, candidate: &mut Candidate) -> Outcome<()> {
    match (candidate.engine, &candidate.source) {
        (Engine::LlamaCpp, Source::Hub) => keep_variant(session, candidate),
        (Engine::Vllm, _) => {
            candidate.files.clear();
            Ok(())
        }
        (Engine::EdgeLlm, _) | (Engine::LlamaCpp, Source::Disk(_)) => Ok(()),
    }
}

/// Keeps the files of one GGUF variant and counts its size as the weights.
fn keep_variant(session: &Session<'_>, candidate: &mut Candidate) -> Outcome<()> {
    let variants = catalog::gguf_variants(&candidate.files);

    if variants.is_empty() {
        return Err(ThorError::Condition(format!(
            "{} has no GGUF files",
            candidate.name()
        )));
    }

    let variant = choose_variant(session, &variants)?;

    candidate
        .files
        .retain(|file| file.path.ends_with(".gguf") && catalog::model_id(&file.path) == variant);
    candidate.weights = Some(candidate.files.iter().map(|file| file.size).sum());

    Ok(())
}

/// The GGUF variant from `--file`, the largest that fits with `--yes`, or from a menu.
fn choose_variant(session: &Session<'_>, variants: &[catalog::GgufVariant<'_>]) -> Outcome<String> {
    let budget = &session.budget;
    let chosen = match (&session.options.file, session.options.yes) {
        (Some(wanted), _) => variants.iter().find(|variant| {
            variant
                .name
                .to_lowercase()
                .ends_with(&wanted.to_lowercase())
        }),
        (None, true) => variants
            .iter()
            .rev()
            .find(|variant| budget.fits(variant.bytes)),
        (None, false) => return variant_menu(variants, budget),
    };

    chosen
        .map(|variant| variant.name.to_string())
        .ok_or_else(|| {
            let names: Vec<&str> = variants
                .iter()
                .map(|variant| variant.name.as_str())
                .collect();

            ThorError::Condition(format!(
                "no variant fits or matches; the variants are {}",
                names.join(", ")
            ))
        })
}

/// The variant menu, smallest first, each with its size and fit.
fn variant_menu(variants: &[catalog::GgufVariant<'_>], budget: &Budget) -> Outcome<String> {
    let items = variants
        .iter()
        .map(|variant| {
            let fit = if budget.fits(variant.bytes) {
                "fits"
            } else {
                "too big"
            };

            MenuItem {
                label: format!(
                    "{:<50} {:>9}  {fit}",
                    variant.name,
                    gigabytes(variant.bytes)
                ),
                value: variant.name.to_string(),
            }
        })
        .collect();

    Ok(Select::new("Variant:", items)
        .with_page_size(PAGE_SIZE)
        .prompt()?
        .value)
}

/// The context from `--context`, or from a menu of the maximum and its halves, each with
/// its memory cost.
fn choose_context(session: &Session<'_>, candidate: &Candidate) -> Outcome<u64> {
    match session.options.context {
        Some(ContextChoice::Tokens(context)) => Ok(context),
        Some(ContextChoice::Max) => candidate.max_context().ok_or_else(|| {
            ThorError::Condition(format!(
                "{}'s config.json gives no maximum context; pass --context TOKENS",
                candidate.name()
            ))
        }),
        None => context_menu(session, candidate),
    }
}

/// The context menu. The first row is the model's maximum, so Enter keeps it.
fn context_menu(session: &Session<'_>, candidate: &Candidate) -> Outcome<u64> {
    let Some(max) = candidate.max_context() else {
        return Ok(
            CustomType::<u64>::new("Context in tokens (config.json gives no maximum):").prompt()?,
        );
    };

    let items: Vec<MenuItem<Option<u64>>> =
        memory::context_choices(max, session.options.reply_tokens * 2)
            .into_iter()
            .map(|context| MenuItem {
                label: context_label(session, candidate, context),
                value: Some(context),
            })
            .chain(iter::once(MenuItem {
                label: "another size".to_string(),
                value: None,
            }))
            .collect();
    let chosen = Select::new("Context:", items)
        .with_page_size(PAGE_SIZE)
        .prompt()?;

    match chosen.value {
        Some(context) => Ok(context),
        None => Ok(CustomType::<u64>::new("Context in tokens:").prompt()?),
    }
}

/// One context menu row: tokens, memory and fit. A context whose Edge-LLM build with a
/// draft model needs a larger workspace than any known to build here says so.
fn context_label(session: &Session<'_>, candidate: &Candidate, context: u64) -> String {
    let cost = candidate.cost(context, session.slots);
    let reply_tokens = session.options.reply_tokens;
    let note = plan::edge_build_note(session.machine, candidate, context, reply_tokens);
    let outlook = note.map_or(BuildOutlook::KnownToBuild, |note| note.outlook);
    let fits = cost.map(|bytes| session.budget.fits(bytes));
    let fit = match (fits, outlook) {
        (Some(true), BuildOutlook::KnownToBuild) => "fits",
        (Some(true), BuildOutlook::Untested) => {
            "fits in memory; drafter build untested at this size"
        }
        (Some(true), BuildOutlook::LikelyToFail) => {
            "fits in memory; the drafter build is likely to fail"
        }
        (Some(false), _) => "too big",
        (None, _) => "cost unknown",
    };

    format!(
        "{:>11} tokens  {:>9} of {} usable  {fit}",
        tokens(context),
        or_unknown(cost, gigabytes),
        gigabytes(session.budget.usable())
    )
}

/// Checks that the context is no larger than the model's maximum and leaves room for a
/// prompt after the reply tokens.
pub fn check_window(candidate: &Candidate, context: u64, reply_tokens: u64) -> Outcome<()> {
    if candidate.max_context().is_some_and(|max| context > max) {
        return Err(ThorError::Condition(format!(
            "{} tokens is more than {}'s maximum of {}",
            tokens(context),
            candidate.name(),
            or_unknown(candidate.max_context(), tokens)
        )));
    }

    if context <= reply_tokens {
        return Err(ThorError::Condition(format!(
            "a context of {} leaves no room for a prompt after {} reply tokens (--reply-tokens)",
            tokens(context),
            tokens(reply_tokens)
        )));
    }

    Ok(())
}

/// Checks that the model and context fit in memory. When they do not, the message says
/// which context would fit and which services hold memory. A model whose size is unknown is let through with a warning.
fn check_fits(session: &Session<'_>, candidate: &Candidate, context: u64) -> Outcome<()> {
    let Some(cost) = candidate.cost(context, session.slots) else {
        eprintln!(
            "The memory cost of {} is unknown (no config.json or no sizes); it is not checked.",
            candidate.name()
        );
        return Ok(());
    };

    if session.budget.fits(cost) {
        return Ok(());
    }

    let running = session
        .machine
        .services_holding_memory_except(candidate.engine.service());

    Err(ThorError::Condition(does_not_fit_message(
        candidate,
        context,
        session.slots,
        &session.budget,
        &running,
    )))
}

/// The message for a model that does not fit: how much it needs, how much there is,
/// the largest context that would fit, and the running services that could be stopped.
pub fn does_not_fit_message(
    candidate: &Candidate,
    context: u64,
    slots: u64,
    budget: &Budget,
    running: &[UnitState],
) -> String {
    let mut lines = vec![shortfall(candidate, context, slots, budget)];

    lines.extend(
        largest_fitting_context(candidate, slots, budget).map(|fit| {
            format!(
                "  The largest context that fits is {}: --context {fit}",
                tokens(fit)
            )
        }),
    );
    lines.extend(running.iter().map(|state| {
        format!(
            "  To make room, stop {} ({}): thor-tigress stop {}",
            state.name,
            or_unknown(state.held_memory(), gigabytes),
            state.name
        )
    }));

    lines.join("\n")
}

/// What the model needs against what can be used, and where the usable figure comes from.
fn shortfall(candidate: &Candidate, context: u64, slots: u64, budget: &Budget) -> String {
    let unit = candidate.engine.service();
    let cap = budget
        .service_limit
        .map(|limit| format!(", {unit} capped at {}", gigabytes(limit)))
        .unwrap_or_default();

    format!(
        "{} at {} tokens needs about {}; {} can be used.\n  \
         {} available, {} held by {unit} (freed when it restarts), {} kept free (MIN_FREE_GB){cap}.",
        candidate.name(),
        tokens(context),
        or_unknown(candidate.cost(context, slots), gigabytes),
        gigabytes(budget.usable()),
        gigabytes(budget.available),
        gigabytes(budget.freed_by_restart),
        gigabytes(budget.keep_free),
    )
}

/// The largest of the maximum context and its halves that fits.
pub fn largest_fitting_context(candidate: &Candidate, slots: u64, budget: &Budget) -> Option<u64> {
    let max = candidate.max_context()?;

    memory::context_choices(max, 1).into_iter().find(|context| {
        candidate
            .cost(*context, slots)
            .is_some_and(|bytes| budget.fits(bytes))
    })
}

/// Asks a yes-or-no question, unless `--yes` answered it. No stops with nothing changed.
pub fn confirm(yes: bool, question: &str) -> Outcome<()> {
    if yes {
        return Ok(());
    }

    if Confirm::new(question).with_default(false).prompt()? {
        return Ok(());
    }

    Err(ThorError::Condition(
        "Stopped; nothing was changed.".to_string(),
    ))
}
