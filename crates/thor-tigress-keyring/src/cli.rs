//! The `thor-tigress-keyring` command line, as a library so it can be tested.
//!
//! ```text
//! thor-tigress-keyring init [--keyring FILE] [--passphrase-file FILE]
//! thor-tigress-keyring request NAME EMAIL
//! thor-tigress-keyring requests
//! thor-tigress-keyring approve EMAIL
//! thor-tigress-keyring keys
//! thor-tigress-keyring show EMAIL
//! thor-tigress-keyring revoke EMAIL|KEY
//! thor-tigress-keyring export
//! ```
//!
//! The passphrase is read, in order, from `--passphrase-file`, from the
//! `THOR_KEYRING_PASSPHRASE` environment variable, or from one line of
//! standard input. The environment variable is what the chat server uses, so
//! nobody has to type anything at boot. A command line that cannot be read is
//! refused before any keyring is opened, so a typo never asks for a passphrase.

use std::{
    fs,
    io::{BufRead, Write},
    path::PathBuf,
};

use zeroize::Zeroizing;

use crate::{
    error::{KeyringError, Outcome},
    store::{self, Keyring, Person, Status},
};

/// The keyring, relative to `$HOME`, unless `--keyring` says otherwise.
pub const DEFAULT_KEYRING: &str = ".config/thor-chat/keyring";

/// The help text.
#[must_use]
pub fn usage() -> String {
    "\
usage: thor-tigress-keyring COMMAND [--keyring FILE] [--passphrase-file FILE]
  init                    start a new keyring at --keyring
  request NAME EMAIL      record someone waiting for access
  requests                list who is waiting
  approve EMAIL           give that person a key and print it
  keys                    list everyone, with keys masked
  show EMAIL              print one person's key
  revoke EMAIL|KEY        take a key away
  revoke-all              take every active key away at once
  export                  print every active key, one per line
"
    .to_string()
}

/// The parsed command line.
struct Options {
    command: String,
    arguments: Vec<String>,
    keyring: PathBuf,
    passphrase_file: Option<PathBuf>,
}

impl Options {
    fn parse(arguments: &[String]) -> Outcome<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let mut options = Options {
            command: String::new(),
            arguments: Vec::new(),
            keyring: home.join(DEFAULT_KEYRING),
            passphrase_file: None,
        };
        let mut rest = arguments.iter();

        while let Some(argument) = rest.next() {
            match argument.as_str() {
                "--keyring" => options.keyring = value_of(argument, rest.next())?,
                "--passphrase-file" => {
                    options.passphrase_file = Some(value_of(argument, rest.next())?);
                }
                "--help" => options.command = argument.clone(),
                other if other.starts_with("--") => {
                    return Err(KeyringError::Usage(format!(
                        "unknown option {other}\n{}",
                        usage()
                    )));
                }
                other if options.command.is_empty() => options.command = other.to_string(),
                other => options.arguments.push(other.to_string()),
            }
        }

        Ok(options)
    }
}

/// The file an option names, or a usage error when the command line ends at
/// the option.
fn value_of(option: &str, value: Option<&String>) -> Outcome<PathBuf> {
    value
        .map(PathBuf::from)
        .ok_or_else(|| KeyringError::Usage(format!("{option} needs a value\n{}", usage())))
}

/// Runs the command in `arguments`, reading the passphrase from `env_passphrase`
/// or `input`, asking for it on `prompt`, and writing what happened to `out`.
///
/// # Errors
///
/// [`KeyringError::Usage`] for a command line that can't be read, and any error
/// of the command itself.
pub fn main_with(
    arguments: &[String],
    env_passphrase: Option<String>,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
    out: &mut impl Write,
) -> Result<(), KeyringError> {
    let options = Options::parse(arguments)?;

    if options.command.is_empty() || matches!(options.command.as_str(), "help" | "-h" | "--help") {
        return write_out(out, &usage());
    }

    let text = command(&options, env_passphrase, input, prompt)?;

    write_out(out, &text)
}

/// The text one command prints. The keyring is opened only by the arms that
/// need it, so a usage error costs nothing.
fn command(
    options: &Options,
    env_passphrase: Option<String>,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
) -> Outcome<String> {
    match (options.command.as_str(), options.arguments.as_slice()) {
        ("init", []) => {
            let passphrase = passphrase(options, env_passphrase, input, prompt)?;
            let keyring = Keyring::create(&options.keyring, &passphrase)?;

            Ok(format!(
                "created {}\nkeep the passphrase: without it, nothing in the file can be read\n",
                keyring.path().display()
            ))
        }
        ("request", [name, email]) => {
            let mut keyring = opened(options, env_passphrase, input, prompt)?;
            let person = keyring.request(name, email)?;

            Ok(format!(
                "recorded {} <{}> at {}; approve with: thor-tigress-keyring approve {}\n",
                person.name,
                person.email,
                store::timestamp(person.created),
                person.email
            ))
        }
        ("requests", []) => {
            let keyring = opened(options, env_passphrase, input, prompt)?;

            Ok(report_requests(keyring.records()))
        }
        ("approve", [email]) => {
            let mut keyring = opened(options, env_passphrase, input, prompt)?;
            let key = keyring.approve(email)?;
            let name = keyring
                .find(email)
                .map_or_else(|| email.clone(), |person| person.name.clone());

            Ok(format!(
                "key for {name} <{email}>:\n{key}\nsend it to them; their old key, if any, no longer works\n"
            ))
        }
        ("keys", []) => {
            let keyring = opened(options, env_passphrase, input, prompt)?;

            Ok(report_keys(keyring.records()))
        }
        ("show", [email]) => {
            let keyring = opened(options, env_passphrase, input, prompt)?;
            let person = keyring
                .records()
                .iter()
                .find(|person| person.email == *email)
                .ok_or_else(|| KeyringError::NotFound(email.clone()))?;

            match &person.key {
                Some(key) if person.status == Status::Active => Ok(format!("{key}\n")),
                _ => Err(KeyringError::Invalid(format!(
                    "{} <{}> has no key",
                    person.name, person.email
                ))),
            }
        }
        ("revoke", [who]) => {
            let mut keyring = opened(options, env_passphrase, input, prompt)?;
            let person = keyring.revoke(who)?;

            Ok(format!("revoked {} <{}>\n", person.name, person.email))
        }
        ("revoke-all", []) => {
            let mut keyring = opened(options, env_passphrase, input, prompt)?;
            let count = keyring.revoke_all()?;

            Ok(format!("revoked {count} active key(s); the records stay\n"))
        }
        ("export", []) => {
            let keyring = opened(options, env_passphrase, input, prompt)?;

            Ok(keyring
                .active_keys()
                .iter()
                .map(|key| format!("{key}\n"))
                .collect())
        }
        ("init" | "requests" | "keys" | "export" | "revoke-all", _) => Err(KeyringError::Usage(
            format!("{} takes no arguments\n{}", options.command, usage()),
        )),
        ("request", _) => Err(KeyringError::Usage(format!(
            "request needs a name and an email\n{}",
            usage()
        ))),
        ("approve" | "show" | "revoke", _) => Err(KeyringError::Usage(format!(
            "{} needs one email address or key\n{}",
            options.command,
            usage()
        ))),
        _ => Err(KeyringError::Usage(format!(
            "unknown command {}\n{}",
            options.command,
            usage()
        ))),
    }
}

/// Opens the keyring with the passphrase, whatever its source.
fn opened(
    options: &Options,
    env_passphrase: Option<String>,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
) -> Outcome<Keyring> {
    let passphrase = passphrase(options, env_passphrase, input, prompt)?;

    Keyring::open(&options.keyring, &passphrase)
}

/// The passphrase, from the file, the environment or one line of input. The
/// question is written to `prompt`, never to the results.
fn passphrase(
    options: &Options,
    env_passphrase: Option<String>,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
) -> Outcome<Zeroizing<String>> {
    if let Some(path) = &options.passphrase_file {
        let text = fs::read_to_string(path).map_err(KeyringError::io(path))?;

        return checked(text.lines().next().unwrap_or_default());
    }

    if let Some(value) = env_passphrase {
        return checked(&value);
    }

    let question = format!("passphrase for {}: ", options.keyring.display());
    write_out(prompt, &question)?;
    prompt.flush().map_err(KeyringError::io("standard error"))?;

    let mut line = String::new();

    input
        .read_line(&mut line)
        .map_err(KeyringError::io("standard input"))?;

    checked(line.trim_end_matches(['\n', '\r']))
}

/// The non-empty passphrase, without the newline that ended it.
fn checked(passphrase: &str) -> Outcome<Zeroizing<String>> {
    let passphrase = passphrase.trim_end_matches(['\n', '\r']);

    if passphrase.is_empty() {
        return Err(KeyringError::EmptyPassphrase);
    }

    Ok(Zeroizing::new(passphrase.to_string()))
}

/// How a key is shown in a list: enough to tell two apart, not enough to use.
fn masked(key: Option<&str>) -> String {
    match key {
        Some(key) if key.len() > 8 => format!("{}…", key.get(..8).unwrap_or(key)),
        Some(key) => key.to_string(),
        None => "—".to_string(),
    }
}

/// The waiting list.
fn report_requests(people: &[Person]) -> String {
    let waiting: Vec<&Person> = people
        .iter()
        .filter(|person| person.status == Status::Requested)
        .collect();

    if waiting.is_empty() {
        return "nobody is waiting\n".to_string();
    }

    let mut text = format!("{} waiting\n", waiting.len());

    for person in waiting {
        text.push_str(&format!(
            "{}  {}  <{}>\n",
            store::timestamp(person.created),
            person.name,
            person.email
        ));
    }

    text
}

/// Everyone, in the order they were recorded.
fn report_keys(people: &[Person]) -> String {
    if people.is_empty() {
        return "the keyring is empty\n".to_string();
    }

    let mut text = format!(
        "{:<20} {:<28} {:<10} {:<21} {}\n",
        "NAME", "EMAIL", "STATUS", "CREATED", "KEY"
    );

    for person in people {
        text.push_str(&format!(
            "{:<20} {:<28} {:<10} {:<21} {}\n",
            person.name,
            person.email,
            status_word(person.status),
            store::timestamp(person.created),
            masked(person.key.as_deref())
        ));
    }

    text
}

/// The lowercase word for a status.
#[must_use]
pub fn status_word(status: Status) -> &'static str {
    match status {
        Status::Requested => "requested",
        Status::Active => "active",
        Status::Revoked => "revoked",
    }
}

/// Writes `text` to `out`, naming standard output when it fails.
fn write_out(out: &mut impl Write, text: &str) -> Outcome<()> {
    out.write_all(text.as_bytes())
        .map_err(KeyringError::io("standard output"))
}

#[cfg(test)]
mod tests;
