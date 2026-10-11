//! Checks a teacher conversation before it becomes training data.
//!
//! Every Rust block of every assistant turn must build on its own with
//! `clippy-driver -D warnings -W clippy::pedantic`, its tests must pass, and
//! spark must find no broken rule. Blocks in other languages are built, and run
//! when they check something, by [`crate::teacher::languages`]. The prose must have no
//! slop and no claim the code contradicts. Rust blocks use only the standard
//! library, so no Cargo project is needed.
//!
//! An entry written from a real section of the corpus may show what the source
//! teaches, such as `unwrap()` in a chapter about `unwrap()`. For those, a
//! broken rule is kept as a note instead of a problem.

use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use thor_spark_safety_eval::answer;

use crate::{
    error::DataError,
    teacher::{languages, Conversation},
};

/// The edition every block is built with.
const EDITION: &str = "2024";

/// How long one test binary or program may run.
pub(crate) const TEST_LIMIT: Duration = Duration::from_secs(30);

/// How often a running test binary is polled.
const POLL: Duration = Duration::from_millis(20);

/// Lines of compiler or test output kept in a problem.
const OUTPUT_LINES: usize = 12;

/// The flags every build gets: warnings, clippy's included, are errors.
const LINT_FLAGS: [&str; 4] = ["-D", "warnings", "-W", "clippy::pedantic"];

/// What one check found wrong with one assistant turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A block does not build, or clippy warns.
    Build {
        /// The turn, counting from 1.
        turn: usize,
        /// The block's language.
        language: &'static str,
        /// The first lines of the compiler's output.
        output: String,
    },
    /// A block's tests fail, or the program exits with an error.
    Tests {
        /// The turn, counting from 1.
        turn: usize,
        /// The block's language.
        language: &'static str,
        /// The first lines of the test output.
        output: String,
    },
    /// A block's tests ran longer than the limit.
    TimedOut {
        /// The turn, counting from 1.
        turn: usize,
    },
    /// spark found a broken rule.
    Rule {
        /// The turn, counting from 1.
        turn: usize,
        /// The rule and what broke it.
        detail: String,
    },
    /// spark found slop in the prose.
    Slop {
        /// The turn, counting from 1.
        turn: usize,
        /// The phrases found.
        phrases: Vec<String>,
    },
    /// The prose claims a rule the code breaks.
    FalseClaim {
        /// The turn, counting from 1.
        turn: usize,
        /// The words of the claim.
        text: String,
    },
}

impl Problem {
    /// A short name for the kind of problem.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Problem::Build { .. } => "build",
            Problem::Tests { .. } => "tests",
            Problem::TimedOut { .. } => "timed out",
            Problem::Rule { .. } => "rule",
            Problem::Slop { .. } => "slop",
            Problem::FalseClaim { .. } => "false claim",
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Build {
                turn,
                language,
                output,
            } => write!(
                f,
                "turn {turn}: {language} does not build cleanly\n{output}"
            ),
            Problem::Tests {
                turn,
                language,
                output,
            } => write!(f, "turn {turn}: {language} tests or program fail\n{output}"),
            Problem::TimedOut { turn } => {
                write!(f, "turn {turn}: tests ran over {} s", TEST_LIMIT.as_secs())
            }
            Problem::Rule { turn, detail } => write!(f, "turn {turn}: {detail}"),
            Problem::Slop { turn, phrases } => {
                write!(f, "turn {turn}: slop: {}", phrases.join(", "))
            }
            Problem::FalseClaim { turn, text } => write!(f, "turn {turn}: false claim: {text}"),
        }
    }
}

/// What checking one conversation found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checked {
    /// Blocks built, by language.
    pub blocks: BTreeMap<&'static str, usize>,
    /// Rust tests that ran and passed.
    pub tests: usize,
    /// Programs in other languages that ran and exited cleanly.
    pub runs: usize,
    /// Blocks whose fence says `ignore`, counted but not built.
    pub ignored: usize,
    /// Everything wrong; empty when the conversation can be used.
    pub problems: Vec<Problem>,
    /// Broken rules in an entry written from a real section, kept for a reviewer.
    pub notes: Vec<String>,
}

impl Checked {
    /// Blocks built in every language.
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.blocks.values().sum()
    }

    fn record(&mut self, turn: usize, language: &'static str, built: Built) {
        if built != Built::Ignored {
            *self.blocks.entry(language).or_insert(0) += 1;
        }

        match built {
            Built::Clean { tests, ran } => {
                self.tests += tests;
                self.runs += usize::from(ran && language != "rust");
            }
            Built::BuildFailed(output) => self.problems.push(Problem::Build {
                turn,
                language,
                output,
            }),
            Built::TestsFailed(output) => self.problems.push(Problem::Tests {
                turn,
                language,
                output,
            }),
            Built::TimedOut => self.problems.push(Problem::TimedOut { turn }),
            Built::Ignored => self.ignored += 1,
        }
    }
}

/// What spark says about an answer that should lose: the reasons it would be
/// caught without a person.
#[must_use]
pub fn spark_objections(text: &str) -> Vec<String> {
    let score = answer::score(text);
    let rules = score
        .blocks
        .iter()
        .flat_map(|block| &block.report.violations)
        .map(|violation| violation.rule.label().to_string());
    let slop = score
        .slop
        .hits
        .iter()
        .map(|hit| format!("slop: {}", hit.text));
    let claims = score
        .false_claims
        .iter()
        .map(|claim| format!("false claim: {}", claim.text));
    let mut objections: Vec<String> = rules.chain(slop).chain(claims).collect();
    objections.dedup();

    objections
}

/// Checks every assistant turn of `conversation`, building in `scratch`.
///
/// # Errors
///
/// `DataError::Io` when the scratch folder can't be written or the compiler
/// can't be started.
pub fn check(conversation: &Conversation, scratch: &Path) -> Result<Checked, DataError> {
    let mut checked = Checked::default();
    let grounded = conversation.section.is_some();

    for (turn, answer_turn) in conversation.answers() {
        for problem in spark_problems(turn, &answer_turn.content) {
            match problem {
                Problem::Rule { .. } if grounded => checked.notes.push(problem.to_string()),
                other => checked.problems.push(other),
            }
        }

        for block in languages::fenced(&answer_turn.content)
            .into_iter()
            .filter(|block| matches!(block.tag.as_str(), "rust" | "rs"))
        {
            let built = if block.ignored {
                Built::Ignored
            } else {
                build_and_test(&block.code, scratch)?
            };

            checked.record(turn, "rust", built);
        }

        for block in languages::blocks(&answer_turn.content) {
            let built = languages::check(&block, scratch)?;
            checked.record(turn, block.language.name(), built);
        }
    }

    Ok(checked)
}

fn spark_problems(turn: usize, text: &str) -> Vec<Problem> {
    let score = answer::score(text);
    let rules = score.blocks.iter().flat_map(|block| {
        let parse_error = block
            .report
            .parse_error
            .iter()
            .map(move |error| Problem::Rule {
                turn,
                detail: format!("does not parse: {error}"),
            });
        let violations = block
            .report
            .violations
            .iter()
            .map(move |violation| Problem::Rule {
                turn,
                detail: format!(
                    "{} at block line {}: {}",
                    violation.rule.label(),
                    violation.line,
                    violation.detail
                ),
            });

        parse_error.chain(violations)
    });
    let slop = (!score.slop.clean()).then(|| Problem::Slop {
        turn,
        phrases: score
            .slop
            .hits
            .iter()
            .map(|hit| hit.text.clone())
            .chain(
                (score.slop.em_dashes > 1).then(|| format!("{} em dashes", score.slop.em_dashes)),
            )
            .collect(),
    });
    let claims = score.false_claims.iter().map(|claim| Problem::FalseClaim {
        turn,
        text: claim.text.clone(),
    });

    rules.chain(slop).chain(claims).collect()
}

/// How one block fared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Built {
    /// It built; `tests` Rust tests passed and `ran` says whether a program ran cleanly.
    Clean {
        /// Rust tests that passed.
        tests: usize,
        /// True when the block was run and exited cleanly.
        ran: bool,
    },
    /// It did not build; the first lines of the compiler's output.
    BuildFailed(String),
    /// Its tests or the program failed; the first lines of the output.
    TestsFailed(String),
    /// Its tests or the program ran longer than the limit.
    TimedOut,
    /// Its fence says `ignore`, so it was not built.
    Ignored,
}

impl Built {
    /// How a block fared once it built and was run: the Rust tests counted in
    /// `output` pass, or the run failed or ran over time.
    #[must_use]
    pub fn after_run(finished: Finished) -> Built {
        match finished {
            Finished::Passed(output) => Built::Clean {
                tests: passed_tests(&output),
                ran: true,
            },
            Finished::Failed(output) => Built::TestsFailed(output),
            Finished::OverTime => Built::TimedOut,
        }
    }
}

/// Whether a block is a program or a library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrateType {
    Bin,
    Lib,
}

impl CrateType {
    fn of(code: &str) -> CrateType {
        if code
            .lines()
            .any(|line| line.trim_start().starts_with("fn main("))
        {
            CrateType::Bin
        } else {
            CrateType::Lib
        }
    }

    fn flag(self) -> &'static str {
        match self {
            CrateType::Bin => "bin",
            CrateType::Lib => "lib",
        }
    }
}

fn build_and_test(code: &str, scratch: &Path) -> Result<Built, DataError> {
    fs::create_dir_all(scratch).map_err(DataError::io(scratch))?;

    let source = scratch.join("example.rs");
    fs::write(&source, code).map_err(DataError::io(&source))?;

    let crate_type = CrateType::of(code).flag();
    let flags = ["--crate-type", crate_type, "--emit=metadata", "--out-dir"];
    let build = clippy(&source, scratch, &flags, scratch)?;

    if let Finished::Failed(output) = build {
        return Ok(Built::BuildFailed(output));
    }

    if !code.contains("#[test]") {
        return Ok(Built::Clean {
            tests: 0,
            ran: false,
        });
    }

    let test_binary = scratch.join("example-tests");
    let test_flags = ["--test", "-A", "dead_code", "-o"];
    let test_build = clippy(&source, scratch, &test_flags, &test_binary)?;

    if let Finished::Failed(output) = test_build {
        return Ok(Built::BuildFailed(output));
    }

    let mut run = Command::new(&test_binary);
    run.arg("--test-threads=1");

    Ok(Built::after_run(run_limited(run, scratch, TEST_LIMIT)?))
}

fn clippy(
    source: &Path,
    scratch: &Path,
    mode: &[&str],
    target: &Path,
) -> Result<Finished, DataError> {
    let mut command = Command::new("clippy-driver");

    command
        .args(["--edition", EDITION, "--crate-name", "example"])
        .args(LINT_FLAGS)
        .args(mode)
        .arg(target)
        .arg(source);

    run_limited(command, scratch, TEST_LIMIT * 4)
}

/// How a command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finished {
    /// It exited with success; everything it printed.
    Passed(String),
    /// It exited with failure; the first lines it printed.
    Failed(String),
    /// It ran longer than its limit and was killed.
    OverTime,
}

pub(crate) fn run_limited(
    mut command: Command,
    scratch: &Path,
    limit: Duration,
) -> Result<Finished, DataError> {
    let log: PathBuf = scratch.join("output.log");
    let file = fs::File::create(&log).map_err(DataError::io(&log))?;
    let copy = file.try_clone().map_err(DataError::io(&log))?;
    let program = PathBuf::from(command.get_program());
    let mut child = command
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(copy)
        .spawn()
        .map_err(DataError::io(&program))?;
    let started = Instant::now();

    let status = loop {
        if let Some(status) = child.try_wait().map_err(DataError::io(&program))? {
            break Some(status);
        }

        if started.elapsed() > limit {
            child.kill().map_err(DataError::io(&program))?;
            child.wait().map_err(DataError::io(&program))?;

            break None;
        }

        thread::sleep(POLL);
    };

    let output = fs::read_to_string(&log).map_err(DataError::io(&log))?;

    Ok(match status {
        Some(status) if status.success() => Finished::Passed(output),
        Some(_) => Finished::Failed(first_lines(&output)),
        None => Finished::OverTime,
    })
}

fn first_lines(output: &str) -> String {
    output
        .lines()
        .take(OUTPUT_LINES)
        .collect::<Vec<_>>()
        .join("\n")
}

fn passed_tests(output: &str) -> usize {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("test result: ok. "))
        .filter_map(|rest| rest.split_whitespace().next())
        .filter_map(|count| count.parse::<usize>().ok())
        .sum()
}

#[cfg(test)]
mod tests;
