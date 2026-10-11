//! Checks Rust code against the five Rust rules listed in the README.
//!
//! 1. No `unwrap()` or `expect()`, tests included.
//! 2. Errors are an enum implementing `Display` and `Error`, written by hand or
//!    derived with `thiserror`; `anyhow` is allowed. No `String` or boxed errors.
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

use std::collections::BTreeMap;

use proc_macro2::{LineColumn, Span};
use serde::Serialize;
use syn::{
    Attribute, Expr, ExprForLoop, ExprMethodCall, File, ImplItemFn, Item, ItemFn, ItemImpl,
    ItemType, ItemUse, Macro, Pat, ReturnType, Signature, Token, UseTree, Visibility,
    punctuated::Punctuated,
    spanned::Spanned,
    visit::{self, Visit},
};

mod error_facts;

use self::error_facts::{At, ERROR_CRATES, ErrorFacts, Implementation, result_error, type_name};
use crate::code::comments::{self, Comment};
/// One of the five rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// Rule 1: no `unwrap()` or `expect()`.
    NoUnwrap,
    /// Rule 2: errors are an enum with `Display` and `Error`, by hand or by
    /// `thiserror`, or `anyhow`; never a `String` or a boxed error.
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
    #[must_use]
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
    #[must_use]
    pub fn all_pass(&self) -> bool {
        self.parsed
            && self
                .verdicts
                .values()
                .all(|verdict| *verdict != Verdict::Fail)
    }
}

/// How code is wrapped before parsing, since answers are often fragments.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wrapping {
    /// As it is: a whole file.
    File,
    /// As the body of a function: statements.
    FunctionBody,
    /// As the body of an `impl`: methods.
    ImplBody,
}

impl Wrapping {
    /// The order fragments are tried in.
    const ALL: [Wrapping; 3] = [Wrapping::File, Wrapping::FunctionBody, Wrapping::ImplBody];

    fn prefix(self) -> &'static str {
        match self {
            Wrapping::File => "",
            Wrapping::FunctionBody => "fn __spark_snippet() {\n",
            Wrapping::ImplBody => "impl __SparkSnippet {\n",
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Wrapping::File => "",
            Wrapping::FunctionBody | Wrapping::ImplBody => "\n}",
        }
    }

    /// Lines the prefix adds, so violation lines can point at the original.
    fn offset(self) -> usize {
        self.prefix().lines().count()
    }
}

/// Checks `source` against the five rules.
#[must_use]
pub fn check(source: &str) -> CodeReport {
    let mut first_error = None;

    for wrapping in Wrapping::ALL {
        let wrapped = format!("{}{source}{}", wrapping.prefix(), wrapping.suffix());

        match syn::parse_file(&wrapped) {
            Ok(file) => return judge(&file, source, wrapping),
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

/// What a rule gets: it fails when broken, passes when it applies and holds,
/// and otherwise has nothing to judge.
fn verdict(applies: bool, broken: bool) -> Verdict {
    match (applies, broken) {
        (_, true) => Verdict::Fail,
        (true, false) => Verdict::Pass,
        (false, false) => Verdict::NotApplicable,
    }
}

fn judge(file: &File, source: &str, wrapping: Wrapping) -> CodeReport {
    let mut scan = Scan {
        offset: wrapping.offset(),
        ..Scan::default()
    };
    scan.visit_file(file);

    let whole_file = wrapping == Wrapping::File;
    let mut violations = scan.violations.clone();

    if whole_file {
        violations.extend(scan.comments_in_bodies(&comments::find(source)));
    }

    violations.extend(scan.errors.violations());
    violations.sort_by_key(|violation| (violation.line, violation.rule));

    let verdicts = Rule::ALL
        .iter()
        .map(|rule| {
            let broken = violations.iter().any(|violation| violation.rule == *rule);

            (*rule, verdict(scan.applies(*rule, whole_file), broken))
        })
        .collect();

    CodeReport {
        parsed: true,
        parse_error: None,
        verdicts,
        violations,
    }
}

/// The most characters of a comment a violation quotes.
const DETAIL_CHARS: usize = 60;

/// Where a function body's braces are.
#[derive(Clone, Copy)]
struct Body {
    open: LineColumn,
    close: LineColumn,
}

impl Body {
    /// True when `comment` starts between the braces.
    fn contains(self, comment: &Comment) -> bool {
        let at = (comment.line, comment.column);

        (self.open.line, self.open.column) < at && at < (self.close.line, self.close.column)
    }
}

/// Walks the syntax tree once and records everything the rules need.
#[derive(Default)]
struct Scan {
    offset: usize,
    in_trait_impl: bool,
    violations: Vec<Violation>,
    bodies: Vec<Body>,
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

    fn check_docs(
        &mut self,
        visibility: &Visibility,
        attributes: &[Attribute],
        name: &str,
        span: Span,
    ) {
        if !matches!(visibility, Visibility::Public(_)) {
            return;
        }

        self.pub_items += 1;

        let documented = attributes
            .iter()
            .any(|attribute| attribute.path().is_ident("doc"));

        if !documented {
            self.report(
                Rule::PubDocs,
                span,
                format!("pub {name} has no doc comment"),
            );
        }
    }

    fn body(&mut self, block: &syn::Block) {
        let open = block.brace_token.span.open().start();
        let close = block.brace_token.span.close().start();
        let shift = |position: LineColumn| LineColumn {
            line: position.line.saturating_sub(self.offset),
            column: position.column,
        };

        self.bodies.push(Body {
            open: shift(open),
            close: shift(close),
        });
    }

    fn comments_in_bodies(&self, comments: &[Comment]) -> Vec<Violation> {
        comments
            .iter()
            .filter(|comment| self.bodies.iter().any(|body| body.contains(comment)))
            .map(|comment| Violation {
                rule: Rule::NoBodyComments,
                line: comment.line,
                detail: shorten(comment.text.trim(), DETAIL_CHARS),
            })
            .collect()
    }

    fn signature(&mut self, signature: &Signature) {
        let ReturnType::Type(_, returned) = &signature.output else {
            return;
        };

        if let Some(error) = result_error(returned) {
            let line = self.line(signature.span());
            self.errors.returned.push(At { line, value: error });
        }
    }

    /// Whether `rule` has anything to judge in what was scanned.
    fn applies(&self, rule: Rule, whole_file: bool) -> bool {
        match rule {
            Rule::NoUnwrap | Rule::NoIndexLoops => true,
            Rule::ErrorEnum => self.errors.handles_errors(),
            Rule::PubDocs => self.pub_items > 0,
            Rule::NoBodyComments => whole_file && !self.bodies.is_empty(),
        }
    }

    /// Records `#[derive(Error)]` on the type `name` as its `Display` and
    /// `Error` impls, which is what `thiserror` writes.
    fn derives_error(&mut self, attributes: &[Attribute], name: &str) {
        let derives = attributes
            .iter()
            .filter(|attribute| attribute.path().is_ident("derive"))
            .filter_map(|attribute| {
                attribute
                    .parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
                    .ok()
            })
            .flatten()
            .any(|derived| {
                derived
                    .segments
                    .last()
                    .is_some_and(|last| last.ident == "Error")
            });

        if derives {
            let line = attributes
                .first()
                .map_or(1, |attribute| self.line(attribute.span()));

            self.errors.uses_error_crate = true;

            self.errors
                .impls
                .extend(["Display", "Error"].map(|trait_name| Implementation {
                    line,
                    trait_name: trait_name.to_string(),
                    type_name: name.to_string(),
                }));
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

        self.check_docs(visibility, attributes, &name, item.span());

        match item {
            Item::Struct(item) => {
                self.errors.structs.insert(item.ident.to_string());
                self.derives_error(&item.attrs, &item.ident.to_string());
            }
            Item::Enum(item) => {
                self.errors.enums.insert(item.ident.to_string());
                self.derives_error(&item.attrs, &item.ident.to_string());
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
        self.check_docs(
            &item.vis,
            &item.attrs,
            &format!("fn {}", item.sig.ident),
            item.span(),
        );

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

            self.errors.impls.push(Implementation {
                line,
                trait_name,
                type_name: self_name,
            });
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
        self.errors.uses_error_crate |=
            use_root(&item.tree).is_some_and(|root| ERROR_CRATES.contains(&root.as_str()));

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

        self.errors.uses_error_crate |= last.is_some_and(|name| name == "anyhow" || name == "bail");

        for argument in &macro_arguments(mac) {
            self.visit_expr(argument);
        }

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
        for argument in &macro_arguments(mac) {
            self.visit_expr(argument);
        }
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
mod tests;
