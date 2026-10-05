# thor-thunder-tigress-platform

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
| [openbatrangs](https://github.com/arpanpathak/openbatrangs) | Agentic coding CLI for local models through Ollama | `projects/openbatrangs` |
| [local-copilot-codebuddy](https://github.com/arpanpathak/local-copilot-codebuddy) | Terminal coding copilot on TensorRT-LLM or llama.cpp, no server | `projects/local-copilot-codebuddy` |
| [thor-sync](https://github.com/arpanpathak/thor-sync) | Keeps project folders copied to the Jetson over SSH or Tailscale | `tools/thor-sync` |

The other repositories are git submodules: a pointer to one commit, not a copy.
Clone with them, or fetch them later:

```bash
git clone --recurse-submodules https://github.com/arpanpathak/thor-thunder-tigress-platform
git submodule update --init          # in an existing clone
```

## Crates

| Crate | Binary | What it does |
|---|---|---|
| `thor-spark-safety-eval` | `spark` | Scores answers: slop phrases in prose, the five Rust rules in code, and claims of following the rules that the code contradicts |
| `thor-hammer-trainer` | `thor-hammer-trainer` | Builds `data/train.jsonl` from books, docs, a chat export and hand-written pairs, with licence checks, clean-up, deduplication and a per-source token cap |
| `thor-tigress-reinforcer-frontend` | `reinforcer` | Review page for the training set: syntax-highlighted code, rule-breaking lines tagged, slop phrases suggested, a phrase marked once found everywhere |
| `thor-lasso-distiller` | `lasso` | Asks a teacher model, served by `trtllm-serve`, for the question each book section answers, and builds conversations whose answers are the book text |

The five Rust rules, checked by `spark` and kept by every crate here:

1. No `unwrap()` or `expect()`, not even in tests.
2. Errors are a hand-written enum implementing `Display` and `std::error::Error`.
3. Every `pub` item has a `///` doc comment.
4. No comments inside function bodies.
5. No index loops like `for i in 0..n`.

`cargo test` runs `spark` on this repository's own source, so a crate that
breaks a rule fails the build.

## Quick start

```bash
cargo build --release
bash train/fetch_corpus.sh                       # clone the open books and docs (~2 GB)
target/release/thor-hammer-trainer data          # build data/train.jsonl and data/stats.md
target/release/reinforcer data/train.jsonl 8787  # review at http://127.0.0.1:8787
target/release/spark rs crates                   # check Rust code against the five rules
target/release/spark score RUN.jsonl --field text  # score a model's answers
target/release/lasso conversations --dry-run     # plan conversations, no model needed
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
