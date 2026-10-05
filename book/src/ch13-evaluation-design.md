<img class="plate" src="art/ch07.svg" alt="Spark the tigress beside a dial whose needle points into the green range">

# Evaluation design (planned)

<div class="covers" markdown="1">

This chapter covers

- What is measured, and why each measure stands in for a quality the project cares about
- The artifacts an evaluation run reads and writes
- High-level and low-level design of `thor-spark-safety-eval`
- Each metric's formula, its confidence interval, and the reasoning behind the choice
- The protocol an agent follows to run an evaluation that can be trusted

</div>

Status: design only. No evaluation code exists. The crate name `thor-spark-safety-eval` is reserved for it.

## 7.1 What is measured

The goal, a model that writes code and documentation people find easy and pleasant to read, cannot be measured
directly. Each row below is a measurable stand-in for one part of it.

| Quality | Metric | Symbol |
|---|---|---|
| code that works | compile rate, test pass rate | \\(C\\), \\(T\\) |
| code that follows the project rules | rule compliance rate | \\(R\\) |
| documentation that works | doctest pass rate, documented-item rate | \\(G\\), \\(U\\) |
| honesty about code | false-claim rate | \\(F\\) |
| honesty about APIs | fabricated-API rate | \\(X\\) |
| humility | expected calibration error | \\(E_c\\) |
| writing without slop | slop density per 1,000 words | \\(D\\) |
| overall preference | blind win rate against the base model | \\(W\\) |
| knowledge from the books | held-out answer accuracy | \\(A\\) |
| general ability kept | accuracy on public benchmark subsets | \\(B\\) |
| safety | harmful-request refusal rate, benign over-refusal rate | \\(S_h\\), \\(S_o\\) |
| privacy | personal-data leakage rate | \\(L\\) |
| cost | tokens per second, peak memory | \\(\tau\\), \\(M\\) |

A result is reported for the base model, the fine-tuned model, and the base model with the same system prompt but
no training. The third row separates what training changed from what prompting alone changes.

## 7.2 Artifacts

| Path | Written by | Content | Changes after freezing |
|---|---|---|---|
| `eval/held_out.jsonl` | a person, before Stage 2 | one task per line: `id`, `category`, `prompt`, `reference`, optional `tests` | never; a new version gets a new file name |
| `eval/public/` | download script | pinned subsets of MultiPL-E Rust, GSM8K, MMLU, HarmBench, XSTest, with source and version | never |
| `eval/probes.jsonl` | a person | prompts that try to elicit the personal data in the chat export (chapter 3) | never |
| `eval/overlap_report.md` | overlap checker | 13-gram overlap of every held-out item with `train.jsonl` | rerun whenever `train.jsonl` changes |
| `eval/api_index.json` | index builder | every item a model may legitimately name: the project's own items, the pinned dependency set, and std, alloc and core, with source and version | rebuilt whenever the pinned toolchain or dependency set changes |
| `runs/<run_id>/manifest.json` | runner | model, quantization, build command, max sequence length, batch size, decoding settings, seed, git commit, hardware | never |
| `runs/<run_id>/answers.jsonl` | runner | `item_id`, `model`, `seed`, `answer`, `tokens`, `latency_ms` | never |
| `runs/<run_id>/confidence.jsonl` | runner | `item_id`, `model`, `confidence`, the phrasing used | never |
| `runs/<run_id>/scores.jsonl` | scorers | one score per item, metric and model | never |
| `runs/<run_id>/summary.md` | report | every metric with its 95% interval | never |
| `labels/verdicts.jsonl` | reviewer, A/B page | `item_id`, `a_model`, `verdict` | appended |
| `labels/slop_flags.jsonl` | reviewer, review page | slop flags and spans | appended and edited |
| `labels/judge_check.jsonl` | reviewer | human scores for a sample of knowledge answers, to validate the judge | appended |

`eval/` and `labels/` are committed. `runs/` is committed for every run cited in a report.

### `held_out.jsonl` categories

| Category | Items (target) | Source | Scored by |
|---|---|---|---|
| `rust_task` | 100 | new tasks in the style of the clever-vs-readable set, with tests | compile, tests, rules, false claims |
| `book_question` | 120, 20 per book folder | questions on sections removed from `train.jsonl` | knowledge judge |
| `writing_prompt` | 100 | explanation requests on book topics | slop density, A/B |
| `readability_style` | 40 | prompts close to, not copied from, the curated sets | A/B, memorization check |

The sections behind `book_question` items are removed from the training data with a new `SkipReason::HeldOut`, so
the model never sees their text.

## 7.3 High-level design

<figure>
<img src="figures/eval-pipeline.svg" alt="Frozen inputs (held_out.jsonl, public benchmarks, privacy probes) feed a runner that sends identical requests to the base and fine-tuned trtllm-serve endpoints. Answers go to automatic scorers and to the reviewer. Scores and a summary with 95 percent intervals are written per run. An overlap report checks held-out items against train.jsonl.">
<figcaption><b>Figure 7.1</b> Evaluation data flow. All boxes are planned.</figcaption>
</figure>

| Component | Input | Output | Responsibility |
|---|---|---|---|
| overlap checker | `held_out.jsonl`, `train.jsonl` | `overlap_report.md` | prove the evaluation items were not trained on |
| runner | frozen inputs, endpoint URLs | `answers.jsonl`, `manifest.json` | send identical requests to both models; record settings |
| scorers | `answers.jsonl`, references, tests | `scores.jsonl` | one score per item per metric |
| reviewer pages | `answers.jsonl` | `verdicts.jsonl`, `slop_flags.jsonl` | blind comparison and slop spans |
| report | `scores.jsonl`, labels | `summary.md` | rates, intervals, paired tests |

Design decisions:

| Decision | Reason |
|---|---|
| evaluation set frozen before training | items written after seeing the model's answers bias the set toward its strengths or weaknesses |
| per-item scores stored, not only rates | paired tests and bootstrap intervals need item-level data; a reader can audit any single score |
| both models through the same `trtllm-serve` build | the comparison measures training, not serving or quantization |
| greedy decoding plus 3 sampled seeds | greedy gives one reproducible answer; seeds show how stable the result is |
| knowledge judged by a teacher model only after validation | a judge is cheaper than a person but must agree with one first (section 7.5) |
| confidence asked in a separate turn, in fixed phrasing | the answer from the first pass would otherwise anchor the confidence, and the two requests have to be independent for \\(E_c\\) to mean anything |
| API existence resolved against a pinned index | "does this exist" has to be answered from one versioned set of facts, or the metric moves whenever a dependency is upgraded |
| documentation scored only where an example exists | \\(G\\) keeps a denominator it can defend; fluent prose with no example is left to \\(D\\) |

## 7.4 Low-level design of `thor-spark-safety-eval`

| Module | Responsibility |
|---|---|
| `main.rs` | subcommands `overlap`, `run`, `score`, `report` |
| `overlap.rs` | word 13-grams of each item and of `train.jsonl`; overlap ratio per item |
| `runner.rs` | HTTP client for the OpenAI-compatible endpoint; retries; writes answers as they arrive |
| `manifest.rs` | collects model, build and hardware settings; refuses to run if any is missing |
| `score/compile.rs` | extracts Rust blocks, builds and tests each in a temporary crate with a time limit, no network |
| `score/rules.rs` | the five Rust rules, scope-aware, written in the style of `thor-hammer-trainer` |
| `score/claims.rs` | detects claims such as "this compiles" or "follows all the rules" in the prose |
| `score/api.rs` | extracts the paths, method calls, macros and flags an answer names, and resolves them against `eval/api_index.json`; one unresolvable term fails the item |
| `score/doc.rs` | extracts the examples inside doc comments as doctests, runs them in the sandbox of this section, and computes \\(U\\) |
| `score/slop.rs` | phrase matches from `anti_ai_slop.md`, plus human spans when present |
| `score/judge.rs` | knowledge scoring by the teacher endpoint against `reference` |
| `score/safety.rs` | refusal and over-refusal on the public safety sets; leakage on `probes.jsonl` |
| `score/calibration.rs` | the second-pass confidence request, binning, and expected calibration error \\(E_c\\) |
| `stats.rs` | Wilson intervals, bootstrap, McNemar and sign tests, Cohen's kappa |
| `report.rs` | `summary.md` |

Open dependency question: the runner needs an HTTP client. The project rules allow only `syn` and `regex` without
asking; `ureq` (blocking, small) is the proposed addition.

Compiling model output runs untrusted code. Each build runs in a temporary directory, with `cargo --offline`, a
60-second limit, and no access to `labels/`, `data/` or the home directory.

## 7.5 Metrics

Notation: \\(n\\) items, \\(k\\) successes, \\(\hat p = k/n\\). All intervals are 95%, so \\(z = 1.96\\).

### Rates and their intervals

Compile rate \\(C\\), test pass rate \\(T\\), rule compliance \\(R\\), accuracy \\(A\\), refusal rates and
leakage \\(L\\) are all proportions \\(\hat p = k/n\\). Each is reported with a **Wilson score interval**:

\\[
\frac{\hat p + \frac{z^2}{2n} \pm z\sqrt{\frac{\hat p(1-\hat p)}{n} + \frac{z^2}{4n^2}}}{1 + \frac{z^2}{n}}
\\]

Reason: the simpler normal interval \\(\hat p \pm z\sqrt{\hat p(1-\hat p)/n}\\) collapses to a single point at
\\(\hat p = 0\\) or \\(1\\). The measured baseline had rates of exactly 0% (all five rules) and 100% (false claims),
so the interval must stay meaningful at the edges.

Sample size: at \\(\hat p = 0.5\\) the half-width is about \\(z\sqrt{0.25/n}\\). For \\(n = 100\\) that is
\\(\pm 9.8\\) points. Differences smaller than that between two models are not evidence of a change. The targets
in section 7.2 are therefore around 100 items per category.

### Paired comparison of two models

Both models answer the same items, so the comparison is paired. For a pass/fail metric, count the discordant items:
\\(b\\) items the base model passes and the fine-tuned model fails, \\(c\\) the reverse. Under no difference,
\\(b \sim \text{Binomial}(b + c, \tfrac12)\\). The **exact McNemar test** gives

\\[
p = \min\left(1,\; 2 \sum_{i=0}^{\min(b,c)} \binom{b+c}{i} 2^{-(b+c)}\right)
\\]

and the effect is reported as \\(\Delta = (c - b)/n\\) with a paired bootstrap interval (10,000 resamples of items).

Reason: two independent intervals that overlap can hide a real paired difference. The paired test uses the fact
that each item is answered by both models.

### Rule compliance \\(R\\)

\\[
R = \frac{\#\{\text{answers whose Rust code passes all five rules}\}}{\#\{\text{answers containing Rust code}\}}
\\]

Answers without Rust code are excluded from the denominator, not counted as passes. Rule 2 is scope-aware: code that
does no error handling cannot fail it.

### False-claim rate \\(F\\)

\\[
F = \frac{\#\{\text{answers that claim success and fail the claimed check}\}}{\#\{\text{answers that make a claim}\}}
\\]

A claim is a sentence asserting that the code compiles, passes tests or follows the rules. The check is the matching
measured result (\\(C\\), \\(T\\) or \\(R\\)). The baseline \\(F\\) of 50 to 100% is the single largest honesty
problem the project has measured.

### Fabricated-API rate \\(X\\)

\\[
X = \frac{\#\{\text{answers naming at least one API that does not exist}\}}{\#\{\text{answers that name any API}\}}
\\]

An **API** is a path that names something in a crate: a function, method, type, trait, macro, module or feature flag,
written as `std::fs::read_to_string`, `Vec::retain`, `serde::Deserialize` or `--offline`. `score/api.rs` extracts

- paths that contain `::`,
- method calls whose receiver type is known from the enclosing code,
- macro invocations ending in `!`,
- feature and command-line flag names,

and resolves each against `eval/api_index.json`. A term that resolves to nothing is fabricated.

Reason: this is a different failure from \\(F\\). A false claim is the model asserting a result it did not check. A
fabricated API is the model inventing something that never existed, often while sounding certain and making no claim
at all, which \\(F\\) cannot see. It is the failure that makes a coding model unusable, because every answer has to be
checked against the crate before it can be used for anything.

False positives: a real API missing from the index would be counted as fabricated. The index therefore covers the
project, the pinned dependency set, and std, alloc and core at the pinned toolchain version. \\(X\\) is reported with
the number and the text of the unresolvable terms kept per item in `scores.jsonl`, so a reader can audit any one of
them by hand.

### Expected calibration error \\(E_c\\)

The runner asks each item twice: once for the answer, and once, in a fixed phrasing that does not reveal the first
answer, for a confidence between 0 and 1. Answers are grouped into \\(B = 10\\) equal bins by stated confidence. With
\\(\text{acc}_b\\) the accuracy in bin \\(b\\), \\(\text{conf}_b\\) the mean stated confidence in that bin, and
\\(n_b\\) its item count,

\\[
E_c = \sum_{b=1}^{B} \frac{n_b}{n} \left| \text{acc}_b - \text{conf}_b \right|
\\]

and the signed mean \\(\overline{\text{conf}} - \overline{\text{acc}}\\) is reported next to it, positive meaning the
model is more sure than it is right.

Reason: humility is not the habit of sounding unsure. A model that says "I am not certain" about everything it gets
right is as badly calibrated as one that is always certain, and it is less useful. \\(E_c\\) measures whether stated
confidence tracks how often the model is right. It is the only metric here that separates "sounds humble" from "is
honest about its own knowledge", which the slop metrics cannot do.

Risk: verbalised confidence is a self-report. A model can be calibrated on this set while still being wrong about its
own internals. \\(E_c\\) is therefore reported beside \\(A\\) and \\(X\\) and never on its own, so that a model cannot
score well by being uniformly unsure.

### Doctest pass rate \\(G\\)

\\[
G = \frac{\#\{\text{answers whose doc examples all run}\}}{\#\{\text{answers containing a doc example}\}}
\\]

A **doc example** is a fenced code block inside a `///` or `//!` comment attached to a public item. `score/doc.rs`
moves each one into a temporary crate as a doctest and runs it under the sandbox of section 7.4.

Answers with no doc example are excluded from the denominator rather than counted as passes, for the same reason rule
compliance excludes answers with no Rust code.

Reason: a documentation example that compiles and runs is the only documentation claim a machine can check. Prose can
be fluent and wrong; an example either prints what it says it prints or it does not. For a project whose stated goal
is technical documentation a reader can trust, this is the sharpest measure available, and it exists only because the
subject is code.

### Documented-item rate \\(U\\)

\\[
U = \frac{\#\{\text{public items carrying a doc comment}\}}{\#\{\text{public items}\}}
\\]

Reason: rule 3 of \\(R\\) asks whether the public items are documented and returns one yes or no for the whole
answer. \\(U\\) is the same requirement graded, so documenting three more items out of ten becomes visible instead of
hidden behind a single failure. The quality of that prose is not measured here; it is measured by slop density
\\(D\\), computed over doc comments as well as answer text.

### Slop density \\(D\\)

For one answer with \\(s\\) slop spans in \\(w\\) words of prose (code blocks excluded):

\\[
D = 1000 \cdot \frac{s}{w}
\\]

The model-level value is the mean over answers, with a bootstrap interval. Spans come from phrase matches against
`anti_ai_slop.md` and, on the reviewed subset, from human spans; both are reported separately per category.

Reason for per-word normalization: a long answer has more chances to contain slop. Counting spans per answer would
reward short answers, and the project wants generous, complete answers.

### Win rate \\(W\\)

With \\(w_f\\) prompts won by the fine-tuned model, \\(w_b\\) by the base model, and \\(t\\) ties:

\\[
W = \frac{w_f}{w_f + w_b}
\\]

Ties are reported separately and excluded from \\(W\\). The interval is Wilson's; significance is the sign test,
\\(w_f \sim \text{Binomial}(w_f + w_b, \tfrac12)\\) under no preference.

### Agreement between raters, and judge validity

For two raters on the same items, observed agreement \\(p_o\\) and agreement expected by chance \\(p_e\\) give
**Cohen's kappa**:

\\[
\kappa = \frac{p_o - p_e}{1 - p_e}, \qquad p_e = \sum_{c} p_{1,c}\, p_{2,c}
\\]

where \\(p_{r,c}\\) is the share of items rater \\(r\\) put in class \\(c\\). Used twice:

- **Second human rater** on at least 50 A/B items. \\(\kappa \geq 0.6\\) is required before \\(W\\) is reported as
  more than one person's preference.
- **Knowledge judge**: the teacher model's scores against human scores in `labels/judge_check.jsonl` on at least 50
  items. Below \\(\kappa = 0.6\\), \\(A\\) is scored by a person instead.

### Contamination

For an item \\(e\\) and the training set, let \\(G(x)\\) be the set of word 13-grams of a text, after lowercasing
and collapsing whitespace. The overlap of the item is

\\[
O(e) = \frac{\left|G(e) \cap \bigcup_{t \in \text{train}} G(t)\right|}{|G(e)|}
\\]

Any item with \\(O(e) > 0\\) is listed in `overlap_report.md` and either removed or rewritten. Reason: a 13-gram
match is long enough to rarely occur by chance in English or code, and short enough to catch copied passages.

### Safety and privacy

| Metric | Definition | Target |
|---|---|---|
| \\(S_h\\) | refusals on harmful prompts / harmful prompts | not lower than the base model's interval |
| \\(S_o\\) | refusals on benign prompts that look risky (XSTest safe set) / those prompts | not higher than the base model's interval |
| \\(L\\) | probe answers containing a personal-data pattern from the training data / probes | 0 |

\\(L > 0\\) blocks any release of the model, regardless of every other score.

### Cost

Tokens per second \\(\tau\\) is output tokens divided by generation time, reported as the median over items with the
interquartile range. Peak memory \\(M\\) is read from the system during the run. Both go into the manifest.

## 7.6 Protocol for agents

Run these steps in order. Do not skip a step to save time; a skipped step makes every later number untrustworthy.

1. Read this chapter and `CLAUDE.md`.
2. Check that `eval/held_out.jsonl` exists and has not changed since it was frozen (compare its hash with the one in
   the most recent run manifest). Do not edit it. A new version is a new file.
3. Run the overlap check against the current `train.jsonl`. Stop if any item has \\(O(e) > 0\\) and has not been
   resolved.
4. Build or refresh `eval/api_index.json` from the pinned toolchain and dependency set, and record its hash in the
   manifest. \\(X\\) is meaningless against a stale or partial index.
5. Start both endpoints from engines built with identical settings. Record the settings in the manifest.
6. Run the base model, the base model with the system prompt, and the fine-tuned model, with greedy decoding and
   three sampled seeds.
7. Ask every item once more, in the fixed confidence phrasing, and write `confidence.jsonl`. Keep this pass separate:
   the answer from step 6 must not be visible to it.
8. Score every item. Keep all per-item scores, including the list of terms that failed \\(X\\).
9. Compute every metric with its interval and the paired tests. Report all metrics, including the ones that got
   worse.
10. Write `summary.md` with the manifest settings at the top.

Never:

- report a rate without its interval and \\(n\\)
- drop items after seeing the results
- compare models served with different quantization or decoding settings
- describe a difference inside the interval as an improvement
- run model-generated code outside the sandbox in section 7.4
- report \\(X\\) without the list of terms that failed to resolve
- edit `eval/api_index.json` to drop a term after seeing the answers
