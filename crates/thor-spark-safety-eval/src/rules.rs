//! Checks Rust code against the five Rust rules listed in the README.
//!
//! 1. No `unwrap()` or `expect()`, tests included.
//! 2. Errors are a hand-written enum implementing `Display` and `Error`.
//! 3. Every `pub` item has a `///` doc comment.
//! 4. No comments inside function bodies.
//! 5. No index loops such as `for i in 0..n`.
//!
//! Each rule is scope-aware: a rule only counts for code it can apply to. A
//! fragment that handles no errors is `NotApplicable` for rule 2, not a pass and
//! not a failure, otherwise every short snippet fails it and the rate means
//! nothing. Rule 4 is judged only on code that parses as a whole file: a
//! statement fragment such as `for x in &v { } // same as v.iter()` is a
//! teaching snippet, and its comments annotate it rather than sit in a body. Answers are often fragments, so code that does not parse as a file
//! is tried again as the body of a function and as the body of an `impl`.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{LineColumn, Span};
use serde::Serialize;
use syn::{
    Attribute, Expr, ExprForLoop, ExprMethodCall, File, ImplItemFn, Item, ItemFn, ItemImpl,
    ItemType, ItemUse, Macro, Pat, ReturnType, Signature, Token, Type, UseTree, Visibility,
    punctuated::Punctuated,
    spanned::Spanned,
    visit::{self, Visit},
};

use crate::comments::{self, Comment};

/// One of the five rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// Rule 1: no `unwrap()` or `expect()`.
    NoUnwrap,
    /// Rule 2: errors are a hand-written enum with `Display` and `Error`.
    ErrorEnum,
    /// Rule 3: every `pub` item has a doc comment.
    PubDocs,
    /// Rule 4: no comments inside function bodies.
    NoBodyComments,
    /// Rule 5: no index loops.
    NoIndexLoops,
}

impl Rule {
    /// The five rules, in the order the README lists them.
    pub const ALL: [Rule; 5] = [
        Rule::NoUnwrap,
        Rule::ErrorEnum,
        Rule::PubDocs,
        Rule::NoBodyComments,
        Rule::NoIndexLoops,
    ];

    /// A short label for tables, such as `R1 no unwrap`.
    pub fn label(self) -> &'static str {
        match self {
            Rule::NoUnwrap => "R1 no unwrap",
            Rule::ErrorEnum => "R2 error enum",
            Rule::PubDocs => "R3 pub docs",
            Rule::NoBodyComments => "R4 no body comments",
            Rule::NoIndexLoops => "R5 iterators",
        }
    }
}

/// What one rule says about one piece of code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The rule applies and holds.
    Pass,
    /// The rule applies and is broken at least once.
    Fail,
    /// The code contains nothing the rule is about.
    NotApplicable,
}

/// One place a rule is broken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    /// The rule broken.
    pub rule: Rule,
    /// The 1-based line in the code that was checked.
    pub line: usize,
    /// What was found, in a few words.
    pub detail: String,
}

/// The result of checking one piece of code.
#[derive(Debug, Clone, Serialize)]
pub struct CodeReport {
    /// False when the code parsed neither as a file, a function body nor an
    /// `impl` body. No rule is judged then.
    pub parsed: bool,
    /// Why parsing failed, when it did.
    pub parse_error: Option<String>,
    /// One verdict per rule.
    pub verdicts: BTreeMap<Rule, Verdict>,
    /// Every place a rule is broken, in line order.
    pub violations: Vec<Violation>,
}

impl CodeReport {
    /// True when the code parsed and no rule failed.
    pub fn all_pass(&self) -> bool {
        self.parsed && self.verdicts.values().all(|verdict| *verdict != Verdict::Fail)
    }
}

/// Checks `source` against the five rules.
pub fn check(source: &str) -> CodeReport {
    let wrappers = [
        (String::new(), String::new()),
        ("fn __spark_snippet() {\n".to_string(), "\n}".to_string()),
        ("impl __SparkSnippet {\n".to_string(), "\n}".to_string()),
    ];
    let mut first_error = None;
    for (prefix, suffix) in &wrappers {
        let wrapped = format!("{prefix}{source}{suffix}");
        let offset = prefix.lines().count();
        match syn::parse_file(&wrapped) {
            Ok(file) => return judge(&file, source, offset),
            Err(error) if first_error.is_none() => {
                let start = error.span().start();
                first_error = Some(format!("line {}: {error}", start.line));
            }
            Err(_) => {}
        }
    }
    CodeReport {
        parsed: false,
        parse_error: first_error,
        verdicts: Rule::ALL
            .iter()
            .map(|rule| (*rule, Verdict::NotApplicable))
            .collect(),
        violations: Vec::new(),
    }
}

fn judge(file: &File, source: &str, offset: usize) -> CodeReport {
    let mut scan = Scan {
        offset,
        ..Scan::default()
    };
    scan.visit_file(file);
    let comments = comments::find(source);
    let whole_file = offset == 0;
    let body_comments = match whole_file {
        true => scan.comments_in_bodies(&comments),
        false => Vec::new(),
    };
    let mut violations = scan.violations.clone();
    violations.extend(body_comments);
    violations.extend(scan.errors.violations());
    violations.sort_by_key(|violation| (violation.line, violation.rule));
    let applies = |rule: Rule| match rule {
        Rule::NoUnwrap | Rule::NoIndexLoops => true,
        Rule::ErrorEnum => scan.errors.handles_errors(),
        Rule::PubDocs => scan.pub_items > 0,
        Rule::NoBodyComments => whole_file && !scan.bodies.is_empty(),
    };
    let verdicts = Rule::ALL
        .iter()
        .map(|rule| {
            let broken = violations.iter().any(|violation| violation.rule == *rule);
            let verdict = match (applies(*rule), broken) {
                (_, true) => Verdict::Fail,
                (true, false) => Verdict::Pass,
                (false, false) => Verdict::NotApplicable,
            };
            (*rule, verdict)
        })
        .collect();
    CodeReport {
        parsed: true,
        parse_error: None,
        verdicts,
        violations,
    }
}

/// The error type of a `Result` the code returns, sorted by what rule 2 makes
/// of it.
#[derive(Debug, Clone)]
enum ErrorType {
    Text(String),
    Boxed,
    Named(String),
}

/// What rule 2 needs to know, gathered over the whole file.
#[derive(Debug, Default, Clone)]
struct ErrorFacts {
    crates: Vec<(usize, String)>,
    returned: Vec<(usize, ErrorType)>,
    enums: BTreeSet<String>,
    structs: BTreeSet<String>,
    impls: Vec<(usize, String, String)>,
    renames: BTreeMap<String, String>,
    aliases: BTreeMap<String, ErrorType>,
}

const ERROR_CRATES: [&str; 5] = ["anyhow", "thiserror", "eyre", "color_eyre", "snafu"];

/// Library error types an answer returns instead of writing its own enum. An
/// error type that is neither local nor listed here is taken to be defined in
/// another file of the same crate, and is not judged.
const LIBRARY_ERRORS: [&str; 9] = [
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
    fn handles_errors(&self) -> bool {
        !self.crates.is_empty() || !self.returned.is_empty() || self.implementors("Error").next().is_some()
    }

    fn implementors<'a>(&'a self, trait_name: &'a str) -> impl Iterator<Item = (usize, &'a str)> {
        self.impls
            .iter()
            .filter(move |(_, implemented, _)| {
                self.renames.get(implemented).unwrap_or(implemented) == trait_name
            })
            .map(|(line, _, self_name)| (*line, self_name.as_str()))
    }

    fn implements(&self, trait_name: &str, type_name: &str) -> bool {
        self.implementors(trait_name)
            .any(|(_, implementor)| implementor == type_name)
    }

    fn local(&self, name: &str) -> bool {
        self.enums.contains(name) || self.structs.contains(name)
    }

    fn violations(&self) -> Vec<Violation> {
        let violation = |line: usize, detail: String| Violation {
            rule: Rule::ErrorEnum,
            line,
            detail,
        };
        let mut found: Vec<Violation> = self
            .crates
            .iter()
            .map(|(line, name)| violation(*line, format!("uses the {name} crate")))
            .collect();
        for (line, name) in self.implementors("Error") {
            if self.structs.contains(name) {
                found.push(violation(line, format!("{name} is a struct, not an enum")));
            }
        }
        let mut reported = BTreeSet::new();
        for (line, error) in &self.returned {
            let resolved = match error {
                ErrorType::Named(name) => self.aliases.get(name).unwrap_or(error),
                other => other,
            };
            let problem = match resolved {
                ErrorType::Text(text) => Some(format!("returns Result<_, {text}>")),
                ErrorType::Boxed => Some("returns Result<_, Box<dyn Error>>".to_string()),
                ErrorType::Named(name) if self.enums.contains(name) => {
                    let missing: Vec<&str> = [
                        ("Display", self.implements("Display", name)),
                        ("Error", self.implements("Error", name)),
                    ]
                    .iter()
                    .filter(|(_, present)| !present)
                    .map(|(trait_name, _)| *trait_name)
                    .collect();
                    (!missing.is_empty())
                        .then(|| format!("{name} does not implement {}", missing.join(" or ")))
                }
                ErrorType::Named(name) if self.structs.contains(name) => {
                    Some(format!("{name} is a struct, not an enum"))
                }
                ErrorType::Named(name) if !self.local(name) && LIBRARY_ERRORS.contains(&name.as_str()) => {
                    Some(format!("returns the library error {name}, not a custom enum"))
                }
                ErrorType::Named(_) => None,
            };
            if let Some(detail) = problem
                && reported.insert(detail.clone())
            {
                found.push(violation(*line, detail));
            }
        }
        found
    }
}

/// Walks the syntax tree once and records everything the rules need.
#[derive(Default)]
struct Scan {
    offset: usize,
    in_trait_impl: bool,
    violations: Vec<Violation>,
    bodies: Vec<(LineColumn, LineColumn)>,
    pub_items: usize,
    errors: ErrorFacts,
}

impl Scan {
    fn line(&self, span: Span) -> usize {
        span.start().line.saturating_sub(self.offset).max(1)
    }

    fn report(&mut self, rule: Rule, span: Span, detail: String) {
        let line = self.line(span);
        self.violations.push(Violation { rule, line, detail });
    }

    fn check_docs(&mut self, visibility: &Visibility, attributes: &[Attribute], name: String, span: Span) {
        if !matches!(visibility, Visibility::Public(_)) {
            return;
        }
        self.pub_items += 1;
        let documented = attributes
            .iter()
            .any(|attribute| attribute.path().is_ident("doc"));
        if !documented {
            self.report(Rule::PubDocs, span, format!("pub {name} has no doc comment"));
        }
    }

    fn body(&mut self, block: &syn::Block) {
        let open = block.brace_token.span.open().start();
        let close = block.brace_token.span.close().start();
        let shift = |position: LineColumn| LineColumn {
            line: position.line.saturating_sub(self.offset),
            column: position.column,
        };
        self.bodies.push((shift(open), shift(close)));
    }

    fn comments_in_bodies(&self, comments: &[Comment]) -> Vec<Violation> {
        comments
            .iter()
            .filter(|comment| {
                let at = (comment.line, comment.column);
                self.bodies.iter().any(|(open, close)| {
                    (open.line, open.column) < at && at < (close.line, close.column)
                })
            })
            .map(|comment| Violation {
                rule: Rule::NoBodyComments,
                line: comment.line,
                detail: shorten(comment.text.trim(), 60),
            })
            .collect()
    }

    fn signature(&mut self, signature: &Signature) {
        let ReturnType::Type(_, returned) = &signature.output else {
            return;
        };
        if let Some(error) = result_error(returned) {
            let line = self.line(signature.span());
            self.errors.returned.push((line, error));
        }
    }

    fn derives_error(&mut self, attributes: &[Attribute]) {
        let derives = attributes
            .iter()
            .filter(|attribute| attribute.path().is_ident("derive"))
            .any(|attribute| {
                let mut found = false;
                let _ = attribute.parse_nested_meta(|meta| {
                    found |= meta.path.segments.last().is_some_and(|last| last.ident == "Error");
                    Ok(())
                });
                found
            });
        if derives {
            let line = attributes
                .first()
                .map(|attribute| self.line(attribute.span()))
                .unwrap_or(1);
            self.errors.crates.push((line, "thiserror (derive(Error))".to_string()));
        }
    }
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_item(&mut self, item: &'ast Item) {
        let (visibility, attributes, name) = match item {
            Item::Fn(item) => (&item.vis, &item.attrs, format!("fn {}", item.sig.ident)),
            Item::Struct(item) => (&item.vis, &item.attrs, format!("struct {}", item.ident)),
            Item::Enum(item) => (&item.vis, &item.attrs, format!("enum {}", item.ident)),
            Item::Trait(item) => (&item.vis, &item.attrs, format!("trait {}", item.ident)),
            Item::Type(item) => (&item.vis, &item.attrs, format!("type {}", item.ident)),
            Item::Const(item) => (&item.vis, &item.attrs, format!("const {}", item.ident)),
            Item::Static(item) => (&item.vis, &item.attrs, format!("static {}", item.ident)),
            Item::Mod(module) if module.content.is_none() => return visit::visit_item(self, item),
            Item::Mod(item) => (&item.vis, &item.attrs, format!("mod {}", item.ident)),
            Item::Union(item) => (&item.vis, &item.attrs, format!("union {}", item.ident)),
            _ => return visit::visit_item(self, item),
        };
        self.check_docs(visibility, attributes, name, item.span());
        match item {
            Item::Struct(item) => {
                self.errors.structs.insert(item.ident.to_string());
                self.derives_error(&item.attrs);
            }
            Item::Enum(item) => {
                self.errors.enums.insert(item.ident.to_string());
                self.derives_error(&item.attrs);
            }
            _ => {}
        }
        visit::visit_item(self, item);
    }

    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        self.signature(&item.sig);
        self.body(&item.block);
        visit::visit_item_fn(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        self.check_docs(&item.vis, &item.attrs, format!("fn {}", item.sig.ident), item.span());
        if !self.in_trait_impl {
            self.signature(&item.sig);
        }
        self.body(&item.block);
        visit::visit_impl_item_fn(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.signature(&item.sig);
        if let Some(block) = &item.default {
            self.body(block);
        }
        visit::visit_trait_item_fn(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        let trait_name = item
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .map(|segment| segment.ident.to_string());
        if let (Some(trait_name), Some(self_name)) = (trait_name, type_name(&item.self_ty)) {
            let line = self.line(item.span());
            self.errors.impls.push((line, trait_name, self_name));
        }
        let outer = self.in_trait_impl;
        self.in_trait_impl = item.trait_.is_some();
        visit::visit_item_impl(self, item);
        self.in_trait_impl = outer;
    }

    fn visit_item_type(&mut self, item: &'ast ItemType) {
        if let Some(error) = result_error(&item.ty) {
            self.errors.aliases.insert(item.ident.to_string(), error);
        }
        visit::visit_item_type(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        if let Some(root) = use_root(&item.tree)
            && ERROR_CRATES.contains(&root.as_str())
        {
            let line = self.line(item.span());
            self.errors.crates.push((line, root));
        }
        visit::visit_item_use(self, item);
    }

    fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
        self.errors
            .renames
            .insert(rename.rename.to_string(), rename.ident.to_string());
        visit::visit_use_rename(self, rename);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        let name = call.method.to_string();
        let banned = (name == "unwrap" && call.args.is_empty())
            || (name == "expect" && call.args.len() == 1);
        if banned {
            self.report(Rule::NoUnwrap, call.method.span(), format!(".{name}()"));
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast ExprForLoop) {
        if let Some(range) = range_of(&node.expr) {
            let variable = match &*node.pat {
                Pat::Ident(binding) => Some(binding.ident.to_string()),
                _ => None,
            };
            let to_len = range
                .end
                .as_deref()
                .is_some_and(|end| matches!(end, Expr::MethodCall(call) if call.method == "len"));
            let indexes = variable.as_deref().is_some_and(|name| {
                let mut finder = IndexFinder { name, found: false };
                finder.visit_block(&node.body);
                finder.found
            });
            if to_len || indexes {
                let pattern = variable.unwrap_or_else(|| "_".to_string());
                self.report(
                    Rule::NoIndexLoops,
                    node.for_token.span,
                    format!("for {pattern} in a range, used as an index"),
                );
            }
        }
        visit::visit_expr_for_loop(self, node);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        let last = mac
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string());
        if let Some(name) = last.filter(|name| name == "anyhow" || name == "bail") {
            let line = self.line(mac.span());
            self.errors.crates.push((line, format!("anyhow ({name}!)")));
        }
        macro_arguments(mac)
            .iter()
            .for_each(|argument| self.visit_expr(argument));
        visit::visit_macro(self, mac);
    }
}

/// Finds `name` used inside an indexing expression such as `items[name]`.
struct IndexFinder<'a> {
    name: &'a str,
    found: bool,
}

impl<'ast> Visit<'ast> for IndexFinder<'_> {
    fn visit_expr_index(&mut self, node: &'ast syn::ExprIndex) {
        let mut mentions = Mentions {
            name: self.name,
            found: false,
        };
        mentions.visit_expr(&node.index);
        self.found |= mentions.found;
        visit::visit_expr_index(self, node);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        macro_arguments(mac)
            .iter()
            .for_each(|argument| self.visit_expr(argument));
    }
}

/// Finds a path that is exactly `name`.
struct Mentions<'a> {
    name: &'a str,
    found: bool,
}

impl<'ast> Visit<'ast> for Mentions<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.found |= path.is_ident(self.name);
        visit::visit_path(self, path);
    }
}

/// The arguments of a macro call that reads like a function call, such as
/// `assert_eq!(a, b)` or `println!("{}", x)`. `syn` does not look inside macro
/// calls, so without this an `unwrap()` inside `assert_eq!` goes unseen.
fn macro_arguments(mac: &Macro) -> Vec<Expr> {
    mac.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
        .map(|arguments| arguments.into_iter().collect())
        .unwrap_or_default()
}

/// The range a `for` loop walks, looking through parentheses and adapters
/// such as `(0..n).rev()`.
fn range_of(expr: &Expr) -> Option<&syn::ExprRange> {
    match expr {
        Expr::Range(range) => Some(range),
        Expr::Paren(inner) => range_of(&inner.expr),
        Expr::MethodCall(call) => range_of(&call.receiver),
        _ => None,
    }
}

/// The last path segment of a type, such as `ConfigError` for
/// `crate::ConfigError`.
fn type_name(ty: &Type) -> Option<String> {
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
fn error_name(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else {
        return None;
    };
    let names: Vec<String> = path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    match names.as_slice() {
        [.., module, last] if last == "Error" => Some(format!("{module}::{last}")),
        [.., last] => Some(last.clone()),
        [] => None,
    }
}

/// The error type of `Result<T, E>`. A one-argument `Result<T>` is looked up
/// as a local alias; `io::Result<T>` stands for `io::Error`. `fmt::Result`
/// belongs to `Display` impls and is not error handling.
fn result_error(ty: &Type) -> Option<ErrorType> {
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
    let types: Vec<&Type> = arguments
        .args
        .iter()
        .filter_map(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .collect();
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

fn classify(error: &Type) -> ErrorType {
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

/// The first segment of a `use` path, such as `anyhow` for `use anyhow::Result`.
fn use_root(tree: &UseTree) -> Option<String> {
    match tree {
        UseTree::Path(path) => Some(path.ident.to_string()),
        UseTree::Name(name) => Some(name.ident.to_string()),
        UseTree::Rename(rename) => Some(rename.ident.to_string()),
        _ => None,
    }
}

fn shorten(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(source: &str, rule: Rule) -> Verdict {
        check(source)
            .verdicts
            .get(&rule)
            .copied()
            .unwrap_or(Verdict::NotApplicable)
    }

    #[test]
    fn finds_unwrap_and_expect_including_inside_macros() {
        let report = check("fn a() { let x = b().unwrap(); assert_eq!(c().expect(\"c\"), 1); }");
        let found: Vec<&str> = report
            .violations
            .iter()
            .filter(|violation| violation.rule == Rule::NoUnwrap)
            .map(|violation| violation.detail.as_str())
            .collect();
        assert_eq!(found, vec![".unwrap()", ".expect()"]);
    }

    #[test]
    fn unwrap_or_is_allowed() {
        assert_eq!(verdict("fn a() { b().unwrap_or(0); }", Rule::NoUnwrap), Verdict::Pass);
    }

    #[test]
    fn rule_two_does_not_apply_to_code_without_error_handling() {
        assert_eq!(verdict("fn add(a: i32, b: i32) -> i32 { a + b }", Rule::ErrorEnum), Verdict::NotApplicable);
    }

    #[test]
    fn rule_two_fails_string_errors_and_boxed_errors() {
        assert_eq!(verdict("fn a() -> Result<(), String> { Ok(()) }", Rule::ErrorEnum), Verdict::Fail);
        assert_eq!(
            verdict("fn main() -> Result<(), Box<dyn std::error::Error>> { Ok(()) }", Rule::ErrorEnum),
            Verdict::Fail
        );
        assert_eq!(verdict("use anyhow::Result;", Rule::ErrorEnum), Verdict::Fail);
    }

    #[test]
    fn rule_two_passes_a_hand_written_enum() {
        let source = "
            #[derive(Debug)] enum E { Bad }
            impl std::fmt::Display for E { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(\"bad\") } }
            impl std::error::Error for E {}
            fn a() -> Result<(), E> { Err(E::Bad) }
        ";
        assert_eq!(verdict(source, Rule::ErrorEnum), Verdict::Pass);
    }

    #[test]
    fn rule_two_does_not_judge_a_signature_a_trait_imposes() {
        let source = "struct R; impl std::io::Read for R { fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> { Ok(0) } }";
        assert_eq!(verdict(source, Rule::ErrorEnum), Verdict::NotApplicable);
    }

    #[test]
    fn rule_two_follows_a_renamed_error_trait() {
        let source = "
            use std::error::Error as StdError;
            enum E { Bad }
            impl std::fmt::Display for E { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(\"bad\") } }
            impl StdError for E {}
            fn a() -> Result<(), E> { Err(E::Bad) }
        ";
        assert_eq!(verdict(source, Rule::ErrorEnum), Verdict::Pass);
    }

    #[test]
    fn rule_two_fails_an_enum_without_error_impl() {
        let source = "enum E { Bad } fn a() -> Result<(), E> { Err(E::Bad) }";
        assert_eq!(verdict(source, Rule::ErrorEnum), Verdict::Fail);
    }

    #[test]
    fn rule_two_follows_a_result_alias() {
        let source = "type Result<T> = std::result::Result<T, String>; fn a() -> Result<()> { Ok(()) }";
        assert_eq!(verdict(source, Rule::ErrorEnum), Verdict::Fail);
    }

    #[test]
    fn rule_two_fails_a_library_error_and_trusts_types_from_other_files() {
        assert_eq!(
            verdict("fn a() -> Result<(), std::io::Error> { Ok(()) }", Rule::ErrorEnum),
            Verdict::Fail
        );
        assert_eq!(verdict("fn a() -> Result<(), DataError> { Ok(()) }", Rule::ErrorEnum), Verdict::Pass);
    }

    #[test]
    fn rule_two_reads_io_result_as_io_error_and_ignores_fmt_result() {
        assert_eq!(verdict("fn main() -> std::io::Result<()> { Ok(()) }", Rule::ErrorEnum), Verdict::Fail);
        let display = "struct S; impl std::fmt::Display for S { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) } }";
        assert_eq!(verdict(display, Rule::ErrorEnum), Verdict::NotApplicable);
    }

    #[test]
    fn a_module_declared_by_file_needs_no_doc_at_the_declaration() {
        assert_eq!(verdict("pub mod rules;", Rule::PubDocs), Verdict::NotApplicable);
        assert_eq!(verdict("pub mod inline {}", Rule::PubDocs), Verdict::Fail);
    }

    #[test]
    fn undocumented_pub_items_fail_and_private_ones_do_not_count() {
        assert_eq!(verdict("pub fn a() {}", Rule::PubDocs), Verdict::Fail);
        assert_eq!(verdict("/// Adds.\npub fn a() {}", Rule::PubDocs), Verdict::Pass);
        assert_eq!(verdict("fn a() {}", Rule::PubDocs), Verdict::NotApplicable);
    }

    #[test]
    fn comments_inside_bodies_fail_and_outside_do_not() {
        assert_eq!(verdict("// fine\nfn a() {\n    1;\n}", Rule::NoBodyComments), Verdict::Pass);
        assert_eq!(verdict("fn a() {\n    // not fine\n    1;\n}", Rule::NoBodyComments), Verdict::Fail);
    }

    #[test]
    fn statement_fragments_are_checked_but_their_comments_are_annotations() {
        let report = check("let x = 1; // note\nlet y = x.unwrap();");
        assert!(report.parsed);
        let lines: Vec<(Rule, usize)> = report
            .violations
            .iter()
            .map(|violation| (violation.rule, violation.line))
            .collect();
        assert_eq!(lines, vec![(Rule::NoUnwrap, 2)]);
        assert_eq!(report.verdicts.get(&Rule::NoBodyComments), Some(&Verdict::NotApplicable));
    }

    #[test]
    fn index_loops_fail_and_counted_repeats_do_not() {
        assert_eq!(verdict("fn a(v: &[i32]) { for i in 0..v.len() { let _ = i; } }", Rule::NoIndexLoops), Verdict::Fail);
        assert_eq!(verdict("fn a(v: &[i32], n: usize) { for i in 0..n { f(v[i]); } }", Rule::NoIndexLoops), Verdict::Fail);
        assert_eq!(verdict("fn a() { for _ in 0..3 { f(); } }", Rule::NoIndexLoops), Verdict::Pass);
    }

    #[test]
    fn unparsable_code_is_not_judged() {
        let report = check("fn a( {");
        assert!(!report.parsed);
        assert!(report.parse_error.is_some());
        assert!(!report.all_pass());
    }
}
