<!-- source: clippy/book/src/development/defining_lints.md; section: 02e814a3ba3a7722; licence: Apache-2.0 -->
### User
I added a new Clippy lint by hand, the code compiles, but the lint never fires on my test file. What did I miss?

### Assistant
Probably the registration. A lint pass only runs if it is listed in one of two macro invocations in `clippy_lints/src/lib.rs`: `late_lint_methods!` for passes that need type information, or `early_lint_methods!` for passes that work on the syntax tree alone. Add an entry at the `// add late passes here` marker (or the early equivalent):

```rust,ignore
FooFunctions: foo_functions::FooFunctions = foo_functions::FooFunctions,
```

Then run `cargo dev update_lints`, which regenerates the lint lists and declarations that have to agree with your new lint.

Next time, `cargo dev new_lint` creates the lint, its test file and the registration in one step, so there is nothing to forget.

---

<!-- source: clippy/book/src/development/emitting_lints.md; section: 03bb6e0fc95a7403; licence: Apache-2.0 -->
### User
When my lint fires, should I add a note, a help message or a suggestion?

### Assistant
They answer different questions for the user.

A **note** explains *why* the lint fired. It is shown apart from the main message, and works best attached to the span it talks about:

```text
error: calls to `std::mem::forget` with a reference instead of an owned value. Forgetting a reference does nothing.
  --> tests/ui/drop_forget_ref.rs:10:5
   |
10 |     forget(&SomeStruct);
   |     ^^^^^^^^^^^^^^^^^^^
   |
note: argument has type &SomeStruct
  --> tests/ui/drop_forget_ref.rs:10:12
```

A **help** message tells the user what to do when you cannot write the exact code change:

```text
error: constant division of 0.0 with 0.0 will always result in NaN
  --> tests/ui/zero_div_zero.rs:6:25
   |
6  |     let other_f64_nan = 0.0f64 / 0.0;
   |                         ^^^^^^^^^^^^
   |
   = help: consider using `f64::NAN` if you would like a constant representing NaN
```

A **suggestion** is the replacement code itself. It is the most useful of the three, because tools like `rustfix` (and `cargo clippy --fix`) can apply it automatically:

```text
error: This `.fold` can be more succinctly expressed as `.any`
--> tests/ui/methods.rs:390:13
    |
390 |     let _ = (0..3).fold(false, |acc, x| acc || x > 2);
    |                   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ help: try: `.any(|x| x > 2)`
```

So: emit a suggestion whenever you can compute the fixed code, a help message when the fix needs a judgment the lint can't make, and a note for context the user needs to understand the problem.

---

<!-- source: clippy/book/src/development/proposals/syntax-tree-patterns.md; section: 155f0f6e6f48270f; licence: Apache-2.0 -->
### User
There was a proposal to write Clippy lints as declarative syntax-tree patterns instead of hand-written matching code. What were the downsides?

### Assistant
The proposal named two.

**Performance.** The pattern-matching code was not optimised, so it could be slower than hand-written matching. The design also works in two stages: match the coarse shape first, then check extra properties in ordinary code. Hand-written lints can instead reject early. For example, a lint about an `if` with no `else` whose `then` block contains only another `if` can check that the block does not start with a comment before descending into it, while the pattern version matches the whole shape first and filters afterwards. The author's view was that this has no conceptual limit and could be fixed by letting patterns filter early.

**Applicability.** Not every lint can be expressed as a pattern, so some would still need custom matching code. Clippy's codebase would then mix two styles of lint, which is a drawback for anyone reading or maintaining it.
