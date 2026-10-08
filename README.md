# thor-thunder-tigress-platform

<p>
  <a href="https://github.com/arpanpathak/thor-thunder-tigress-platform/actions/workflows/ci.yml"><img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/ci.svg" alt="CI" height="28"></a>
  <a href="https://github.com/arpanpathak/thor-thunder-tigress-platform/blob/badges/history.csv"><img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/coverage.svg" alt="Line coverage" height="28"></a>
  <a href="https://github.com/arpanpathak/thor-thunder-tigress-platform/blob/badges/summary.json"><img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/tests.svg" alt="Tests" height="28"></a>
  <a href="#crates"><img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/rules.svg" alt="The five Rust rules" height="28"></a>
  <a href="rust-toolchain.toml"><img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/rust.svg" alt="Rust version" height="28"></a>
  <a href="LICENSE"><img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/license.svg" alt="License" height="28"></a>
</p>

Measured by CI on every push to `main`: each crate's tests run under
`cargo llvm-cov`, and `spark` checks the whole repository against the five
rules. The numbers are kept over time in
[`history.csv`](https://github.com/arpanpathak/thor-thunder-tigress-platform/blob/badges/history.csv) on the `badges` branch.

Fine-tune a local language model, on an NVIDIA Jetson, so it stops writing AI
slop: in prose (filler, fake importance, hedging) and in Rust (unwrap, comments
inside function bodies, index loops, errors that are strings).

This repository is the training and evaluation platform, and the umbrella for
the other projects that run on the same device.

**Book:** https://arpanpathak.github.io/thor-thunder-tigress-platform/ (or `mdbook serve book` locally).

## The projects

| Repository | What it is | Here |
|---|---|---|
| **thor-thunder-tigress-platform** (this one) | Data pipeline, slop and rule checker, review page, teacher-question generator | `crates/` |
| [openbatrangs](https://github.com/arpanpathak/openbatrangs) | Agentic coding CLI for Ollama or any OpenAI-compatible server, such as Nemotron on the Thor | `projects/openbatrangs` |
| [local-copilot-codebuddy](https://github.com/arpanpathak/local-copilot-codebuddy) | Terminal coding copilot on TensorRT-LLM or llama.cpp, no server | `projects/local-copilot-codebuddy` |
| [thor-sync](https://github.com/arpanpathak/thor-sync) | Keeps project folders copied to the Jetson over SSH or Tailscale | `tools/thor-sync` |

The other repositories are git submodules: a pointer to one commit, not a copy.
Clone with them, or fetch them later:

```bash
git clone --recurse-submodules https://github.com/arpanpathak/thor-thunder-tigress-platform
git submodule update --init          # in an existing clone
```

## Crates

| Crate | Binary | Health | What it does |
|---|---|---|---|
| `thor-spark-safety-eval` | `spark` | <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-spark-safety-eval-coverage.svg" alt="coverage" height="24"> <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-spark-safety-eval-tests.svg" alt="tests" height="24"> | Scores answers: slop phrases in prose, the five Rust rules in code, and claims of following the rules that the code contradicts |
| `thor-hammer-trainer` | `thor-hammer-trainer` | <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-hammer-trainer-coverage.svg" alt="coverage" height="24"> <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-hammer-trainer-tests.svg" alt="tests" height="24"> | Builds `data/train.jsonl` from books, docs, a chat export and hand-written pairs, with licence checks, clean-up, deduplication and a per-source token cap |
| `thor-tigress-reinforcer-frontend` | `reinforcer` | <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-tigress-reinforcer-frontend-coverage.svg" alt="coverage" height="24"> <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-tigress-reinforcer-frontend-tests.svg" alt="tests" height="24"> | Review page for the training set: syntax-highlighted code, rule-breaking lines tagged, slop phrases suggested, a phrase marked once found everywhere |
| `thor-lasso-distiller` | `lasso` | <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-lasso-distiller-coverage.svg" alt="coverage" height="24"> <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-lasso-distiller-tests.svg" alt="tests" height="24"> | Asks a teacher model, served by `trtllm-serve`, for the question each book section answers, and builds conversations whose answers are the book text |
| `thor-tigress-agent` | `thor-tigress-agent` | <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-tigress-agent-coverage.svg" alt="coverage" height="24"> <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-tigress-agent-tests.svg" alt="tests" height="24"> | The Thor Tigress Cub's server: the chat page, the personal keys in the keyring, the registration form, web search with recency, page reading, and the API, in front of llama-server |
| `thor-tigress-keyring` | `thor-tigress-keyring` | <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-tigress-keyring-coverage.svg" alt="coverage" height="24"> <img src="https://raw.githubusercontent.com/arpanpathak/thor-thunder-tigress-platform/badges/thor-tigress-keyring-tests.svg" alt="tests" height="24"> | The encrypted registry of who may use the chat and the key each was given: a name, an email, a status and a key per person, sealed with a passphrase (Argon2id, then XChaCha20-Poly1305) and queried over SSH |

The five Rust rules, checked by `spark` and kept by every crate here:

1. No `unwrap()` or `expect()`, not even in tests.
2. Errors are a hand-written enum implementing `Display` and `std::error::Error`.
3. Every `pub` item has a `///` doc comment.
4. No comments inside function bodies.
5. No index loops like `for i in 0..n`.

`cargo test` runs `spark` on this repository's own source, so a crate that
breaks a rule fails the build.

## Web search, and reading what it finds

With the chat's **Web** switch on, `thor-tigress-agent` runs a tool loop for the
chosen model:

- **`web_search`** asks SearXNG for titles, addresses, dates and snippets, with
  an optional `time_range` (`day`, `week`, `month`, `year`) for jobs and other
  recent postings. One call can fan out over several sub-questions (`queries`),
  and `kind: "jobs"` widens a search towards postings while `kind: "people"`
  looks for the recruiter and the hiring manager behind one. Every source gets
  one number to cite, and an answer is budgeted at 16 searches and 12 pages. An
  engine that was suspended or showed a CAPTCHA is named in the result, so a thin
  answer can say why.
- **`fetch_page_content_recursive`** opens one address a search returned, or one
  the user wrote, follows that page's own links up to two hops, and returns the
  text: six pages at most, 12,000 characters a page, 24,000 in all.

The first round of a Web-on answer carries `tool_choice: "required"`, so both
Nemotron and Qwen look before they answer; later rounds are the model's choice.
Eight tool rounds run at most, a repeat of a call already made is not run again,
and up to three rounds without tools at the end make the model write the answer.
The search hint is taken back out of the system line before those rounds, and no
call runs in them: a call written there is dropped and the model is asked again,
because running it is how a model that keeps calling tools never writes a word.
Text written beside a call is the answer. A call the model writes as text
(`<tool_call><function=web_search>…`) is filtered out of the stream and run
like a structured one on a tool round. When no round writes any text at all, the
answer names the sources the search found rather than ending on an excuse.

The fetch runs behind the rules of the book's
["Tool calling"](book/src/ch19-tool-calling.md) chapter: `https` and public
addresses only, one lookup and one connection, at most three redirects, no
cookies, `Authorization` or referrer, and page text labelled as untrusted.

## Quick start

```bash
cargo build --release
bash train/fetch_corpus.sh                       # clone the open books and docs (~2 GB)
target/release/thor-hammer-trainer data          # build data/train.jsonl and data/stats.md
target/release/reinforcer data/train.jsonl 8787  # review at http://127.0.0.1:8787
target/release/spark rs crates                   # check Rust code against the five rules
target/release/spark score RUN.jsonl --field text  # score a model's answers
target/release/lasso conversations --dry-run     # plan conversations, no model needed
target/release/thor-tigress-keyring init         # the encrypted registry of invited people
```

`fetch_corpus.sh` reads every licence file of every source; the trainer
refuses anything that is not MIT, Apache-2.0, BSD, CC BY or CC0.

The data pipeline reads personal inputs (a chat export, hand-written pairs)
from paths set in `crates/thor-hammer-trainer/src/build.rs`. They are not part
of this repository; `data/` and `corpus/` are git-ignored.

## Status

| Stage | State |
|---|---|
| 0. Slop and rule checker (`spark`) | Done. Scores the two baseline models in `runs/` |
| 1. Training data | Done: 13,114 examples, about 6.5M tokens, 35 sources |
| 2. QLoRA on the Thor, merge, quantize, serve, score before and after | Not started |
| 3. Distillation: teacher answers that pass the checker and compile | Question generator written and tested against a fake server; waits for `trtllm-serve` on the Thor |
| 4. Small Rust/Candle trainer | Only if it earns its place |

What `spark` does not do yet: compile code or run tests. A block can pass all
five rules and still not build.

## Hardware

Built on a Jetson Orin NX 16 GB; training targets a Jetson AGX Thor 128 GB
(Blackwell, sm_110, JetPack 7). Device setup, SSH and syncing are in
[`jetson-thor/README.md`](jetson-thor/README.md); syncing uses
[thor-sync](https://github.com/arpanpathak/thor-sync).

Models on the Thor are served by llama.cpp's llama-server in router mode
and managed with
[`thor-tigress-serve`](jetson-thor/model-serving/thor-tigress-serve):
`list`, `list-latest` (the newest models on Hugging Face), `load`, `unload`
and `download`, each taking a key from a list or a model name. Nemotron 3 Nano is loaded; Nemotron 3.5 Lightning is on disk. The
book's chapter "Model serving" has the memory checks and models worth
trying.
