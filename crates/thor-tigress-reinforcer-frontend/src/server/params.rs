//! A request's query parameters, read leniently: a parameter the page sends
//! badly counts as not sent.

use std::collections::HashMap;

use serde::Deserialize;

use crate::{
    datasets::{app::App, workspace::Workspace},
    error::Outcome,
};

/// The query parameters of one request, decoded.
#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub(super) struct Params(HashMap<String, String>);

impl Params {
    /// The value of a parameter.
    pub(super) fn param(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// A parameter that counts only when it has text.
    pub(super) fn text(&self, name: &str) -> Option<String> {
        self.param(name)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_string)
    }

    /// A parameter as a number, or `fallback` when absent or unparsable.
    pub(super) fn number(&self, name: &str, fallback: usize) -> usize {
        self.param(name)
            .and_then(|value| value.parse().ok())
            .unwrap_or(fallback)
    }

    /// A parameter as a yes or no: `1`, `true`, `yes` or `0`, `false`, `no`.
    pub(super) fn boolean(&self, name: &str) -> Option<bool> {
        match self.param(name)? {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        }
    }

    /// The dataset named by `?dataset=`, the first one when not named.
    ///
    /// # Errors
    ///
    /// `ReviewError::NotFound` for a name no dataset has.
    pub(super) fn app<'a>(&self, workspace: &'a Workspace) -> Outcome<&'a App> {
        Ok(&workspace.dataset(self.param("dataset"))?.app)
    }
}
