//! Command-line arguments of a `lasso agent` subcommand: `--name value`
//! options, positional values, and the arguments after `--`, which are passed
//! to another program unchanged.

use std::collections::HashMap;

use crate::error::DistillError;

/// The parsed arguments after the subcommand's name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Arguments {
    /// Each `--name value` option, by name without the dashes.
    pub options: HashMap<String, String>,
    /// The values that follow no option name.
    pub positional: Vec<String>,
    /// Everything after a lone `--`.
    pub passed_on: Vec<String>,
}

impl Arguments {
    /// Parses `arguments`. Every option takes a value; a later option with
    /// the same name replaces an earlier one.
    ///
    /// # Errors
    ///
    /// [`DistillError::Usage`] for an option without a value.
    pub fn parse(arguments: &[String]) -> Result<Self, DistillError> {
        let split = arguments.iter().position(|argument| argument == "--");
        let (own, passed_on) = arguments.split_at(split.unwrap_or(arguments.len()));
        let mut parsed = Arguments {
            passed_on: passed_on.iter().skip(1).cloned().collect(),
            ..Arguments::default()
        };
        let mut rest = own.iter();

        while let Some(argument) = rest.next() {
            parsed.take(argument, &mut rest)?;
        }

        Ok(parsed)
    }

    /// Adds `argument` as an option, with its value taken from `rest`, or as
    /// a positional value.
    fn take<'a>(
        &mut self,
        argument: &str,
        rest: &mut impl Iterator<Item = &'a String>,
    ) -> Result<(), DistillError> {
        match argument.strip_prefix("--") {
            Some(name) => {
                let value = rest.next().ok_or_else(missing_value(argument))?;
                self.options.insert(name.to_string(), value.clone());
            }
            None => self.positional.push(argument.to_string()),
        }

        Ok(())
    }

    /// Returns `Ok` when every option's name is in `known`.
    ///
    /// # Errors
    ///
    /// [`DistillError::Usage`] naming the first unknown option.
    pub fn allow_only(&self, known: &[&str]) -> Result<(), DistillError> {
        let unknown = self
            .options
            .keys()
            .find(|name| !known.contains(&name.as_str()));

        match unknown {
            Some(name) => Err(DistillError::Usage(format!(
                "unknown option --{name}\n{}",
                super::usage()
            ))),
            None => Ok(()),
        }
    }

    /// Returns the value of option `name`.
    ///
    /// # Errors
    ///
    /// [`DistillError::Usage`] when the option is missing.
    pub fn required(&self, name: &str) -> Result<String, DistillError> {
        self.options
            .get(name)
            .cloned()
            .ok_or_else(|| DistillError::Usage(format!("--{name} is required")))
    }

    /// Returns the value of option `name`, or `default` when it is missing.
    #[must_use]
    pub fn or(&self, name: &str, default: &str) -> String {
        self.options
            .get(name)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }

    /// Returns the value of option `name` as a number, or `default` when it
    /// is missing.
    ///
    /// # Errors
    ///
    /// [`DistillError::Usage`] when the value is not a whole number.
    pub fn number(&self, name: &str, default: u64) -> Result<u64, DistillError> {
        let text = self.or(name, &default.to_string());

        text.parse()
            .map_err(|_| DistillError::Usage(format!("--{name} needs a number, not {text}")))
    }

    /// Returns the only positional value, such as the runs file of `score`.
    ///
    /// # Errors
    ///
    /// [`DistillError::Usage`] when there is not exactly one.
    pub fn single(&self, what: &str) -> Result<String, DistillError> {
        match self.positional.as_slice() {
            [value] => Ok(value.clone()),
            _ => Err(DistillError::Usage(format!(
                "needs one {what}\n{}",
                super::usage()
            ))),
        }
    }
}

/// Returns a closure that builds the usage error for option `argument`
/// without a value.
fn missing_value(argument: &str) -> impl FnOnce() -> DistillError {
    let message = format!("{argument} needs a value");

    move || DistillError::Usage(message)
}
