//! What rule 2 needs to know about a file: the error types its functions
//! return, the enums and structs it defines, and what they implement.

use std::collections::{BTreeMap, BTreeSet};

use syn::Type;

use super::{Rule, Violation};

/// The error type of a `Result` the code returns, sorted by what rule 2 makes
/// of it.
#[derive(Debug, Clone)]
pub(super) enum ErrorType {
    Text(String),
    Boxed,
    Named(String),
}

/// Something found on a line of the checked code.
pub(super) struct At<T> {
    pub(super) line: usize,
    pub(super) value: T,
}

/// `impl Trait for Type`, by name.
pub(super) struct Implementation {
    pub(super) line: usize,
    pub(super) trait_name: String,
    pub(super) type_name: String,
}

/// What rule 2 needs to know, gathered over the whole file.
#[derive(Default)]
pub(super) struct ErrorFacts {
    /// True when the code uses an error crate, such as `anyhow`.
    pub(super) uses_error_crate: bool,
    /// The error types of returned `Result`s.
    pub(super) returned: Vec<At<ErrorType>>,
    pub(super) enums: BTreeSet<String>,
    pub(super) structs: BTreeSet<String>,
    pub(super) impls: Vec<Implementation>,
    /// `use a::B as C`: from `C` back to `B`.
    pub(super) renames: BTreeMap<String, String>,
    /// `type Result<T> = …`: the error type each alias stands for.
    pub(super) aliases: BTreeMap<String, ErrorType>,
}

pub(super) const ERROR_CRATES: [&str; 5] = ["anyhow", "thiserror", "eyre", "color_eyre", "snafu"];

/// Library error types an answer returns instead of writing its own enum. An
/// error type that is neither local nor listed here is taken to be defined in
/// another file of the same crate, and is not judged.
pub(super) const LIBRARY_ERRORS: [&str; 9] = [
    "io::Error",
    "fmt::Error",
    "ParseIntError",
    "ParseFloatError",
    "ParseBoolError",
    "Utf8Error",
    "FromUtf8Error",
    "TryFromIntError",
    "VarError",
];

impl ErrorFacts {
    pub(super) fn handles_errors(&self) -> bool {
        self.uses_error_crate
            || !self.returned.is_empty()
            || self.implementors("Error").next().is_some()
    }

    /// The `impl`s of `trait_name`, following `use … as …` renames.
    pub(super) fn implementors<'a>(
        &'a self,
        trait_name: &'a str,
    ) -> impl Iterator<Item = &'a Implementation> {
        self.impls.iter().filter(move |implementation| {
            self.renames
                .get(&implementation.trait_name)
                .unwrap_or(&implementation.trait_name)
                == trait_name
        })
    }

    pub(super) fn implements(&self, trait_name: &str, type_name: &str) -> bool {
        self.implementors(trait_name)
            .any(|implementation| implementation.type_name == type_name)
    }

    pub(super) fn local(&self, name: &str) -> bool {
        self.enums.contains(name) || self.structs.contains(name)
    }

    pub(super) fn violations(&self) -> Vec<Violation> {
        let violation = |line: usize, detail: String| Violation {
            rule: Rule::ErrorEnum,
            line,
            detail,
        };
        let mut found = Vec::new();

        for implementation in self.implementors("Error") {
            if self.structs.contains(&implementation.type_name) {
                found.push(violation(
                    implementation.line,
                    format!("{} is a struct, not an enum", implementation.type_name),
                ));
            }
        }

        let mut reported = BTreeSet::new();

        for returned in &self.returned {
            if let Some(detail) = self.problem(&returned.value)
                && reported.insert(detail.clone())
            {
                found.push(violation(returned.line, detail));
            }
        }

        found
    }

    /// What is wrong with returning `error`, after following type aliases.
    pub(super) fn problem(&self, error: &ErrorType) -> Option<String> {
        let resolved = match error {
            ErrorType::Named(name) => self.aliases.get(name).unwrap_or(error),
            ErrorType::Text(_) | ErrorType::Boxed => error,
        };

        match resolved {
            ErrorType::Text(text) => Some(format!("returns Result<_, {text}>")),
            ErrorType::Boxed => Some("returns Result<_, Box<dyn Error>>".to_string()),
            ErrorType::Named(name) if self.enums.contains(name) => self.missing_traits(name),
            ErrorType::Named(name) if self.structs.contains(name) => {
                Some(format!("{name} is a struct, not an enum"))
            }
            ErrorType::Named(name)
                if !self.local(name) && LIBRARY_ERRORS.contains(&name.as_str()) =>
            {
                Some(format!(
                    "returns the library error {name}, not a custom enum"
                ))
            }
            ErrorType::Named(_) => None,
        }
    }

    /// Which of `Display` and `Error` the local enum `name` lacks, if any.
    pub(super) fn missing_traits(&self, name: &str) -> Option<String> {
        let missing: Vec<&str> = ["Display", "Error"]
            .into_iter()
            .filter(|trait_name| !self.implements(trait_name, name))
            .collect();

        (!missing.is_empty()).then(|| format!("{name} does not implement {}", missing.join(" or ")))
    }
}

/// The last path segment of a type, such as `ConfigError` for
/// `crate::ConfigError`.
pub(super) fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

/// The last two path segments of a type when the last is the bare `Error`,
/// so `std::io::Error` reads `io::Error`; otherwise the same as [`type_name`].
pub(super) fn error_name(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else {
        return None;
    };

    let names: Vec<String> = path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let (last, before) = names.split_last()?;

    match before.last() {
        Some(module) if last == "Error" => Some(format!("{module}::{last}")),
        Some(_) | None => Some(last.clone()),
    }
}

/// The types among the arguments of a generic type, without its lifetimes
/// and constants.
fn type_arguments(arguments: &syn::AngleBracketedGenericArguments) -> Vec<&Type> {
    arguments
        .args
        .iter()
        .filter_map(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .collect()
}

/// The error type of `Result<T, E>`. A one-argument `Result<T>` is looked up
/// as a local alias; `io::Result<T>` stands for `io::Error`. `fmt::Result`
/// belongs to `Display` impls and is not error handling.
pub(super) fn result_error(ty: &Type) -> Option<ErrorType> {
    let Type::Path(path) = ty else {
        return None;
    };

    let last = path.path.segments.last()?;

    if last.ident != "Result" {
        return None;
    }

    let syn::PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return None;
    };

    let types = type_arguments(arguments);
    let module = path
        .path
        .segments
        .iter()
        .rev()
        .nth(1)
        .map(|segment| segment.ident.to_string());

    match (types.as_slice(), module.as_deref()) {
        ([_, error], _) => Some(classify(error)),
        ([_], None) => Some(ErrorType::Named("Result".to_string())),
        ([_], Some("fmt")) => None,
        ([_], Some(module)) => Some(ErrorType::Named(format!("{module}::Error"))),
        _ => None,
    }
}

pub(super) fn classify(error: &Type) -> ErrorType {
    match error {
        Type::Reference(reference) => match type_name(&reference.elem) {
            Some(name) if name == "str" => ErrorType::Text("&str".to_string()),
            Some(name) => ErrorType::Named(name),
            None => ErrorType::Named("reference".to_string()),
        },
        other => match error_name(other) {
            Some(name) if name == "String" => ErrorType::Text("String".to_string()),
            Some(name) if name == "Box" => ErrorType::Boxed,
            Some(name) => ErrorType::Named(name),
            None => ErrorType::Named("unnamed type".to_string()),
        },
    }
}
