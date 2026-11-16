//! The questions `thor-tigress` asks: menus, typed answers and confirmations.
//!
//! Commands ask through [`Prompt`], so a test can answer from a script instead of a
//! terminal. [`Terminal`] asks a person with `inquire`.

use std::fmt::Display;

use inquire::{Confirm, CustomType, Select, Text};

use crate::error::Outcome;

/// Rows shown at once in a menu.
const PAGE_SIZE: usize = 15;

/// Something that answers `thor-tigress`'s questions.
pub trait Prompt {
    /// The row chosen from `rows`. `help`, when given, replaces the menu's help line.
    fn select<Row: Display>(
        &mut self,
        question: &str,
        help: Option<&str>,
        rows: Vec<Row>,
    ) -> Outcome<Row>;

    /// A line of text.
    fn text(&mut self, question: &str) -> Outcome<String>;

    /// A whole number.
    fn number(&mut self, question: &str) -> Outcome<u64>;

    /// Yes or no. `default` is the answer the Enter key gives.
    fn confirm(&mut self, question: &str, default: bool) -> Outcome<bool>;
}

/// Asks a person on the terminal.
pub struct Terminal;

impl Prompt for Terminal {
    /// Shows a menu of `rows`, filtered as the person types.
    fn select<Row: Display>(
        &mut self,
        question: &str,
        help: Option<&str>,
        rows: Vec<Row>,
    ) -> Outcome<Row> {
        let mut menu = Select::new(question, rows).with_page_size(PAGE_SIZE);
        menu.help_message = help.or(menu.help_message);

        Ok(menu.prompt()?)
    }

    /// Reads one line.
    fn text(&mut self, question: &str) -> Outcome<String> {
        Ok(Text::new(question).prompt()?)
    }

    /// Reads a whole number, asking again until the answer is one.
    fn number(&mut self, question: &str) -> Outcome<u64> {
        Ok(CustomType::<u64>::new(question).prompt()?)
    }

    /// Reads `y` or `n`.
    fn confirm(&mut self, question: &str, default: bool) -> Outcome<bool> {
        Ok(Confirm::new(question).with_default(default).prompt()?)
    }
}
