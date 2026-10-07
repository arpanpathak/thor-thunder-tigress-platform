# Worklog

What was done, what broke, and what is still open, newest first. Read this
before changing anything on the Thor. Commit hashes point to the details.

## Machines

| Machine | Role |
|---|---|
| yahboom (Orin NX 16 GB, JetPack 6) | development; `thor-sync` mirrors edits to the Thor within seconds |
| Thor (`ssh thor`, AGX Thor 128 GB) | `thor-chat` (llama-server, Nemotron 3 Nano Q8_0, :8079), `thor-tigress-agent` (page, API, search, :8080), SearXNG (:8888) |

`thor-sync` copies `.git` too but excludes `/book/`, so `git status` on the
Thor lists every book file as deleted. Nothing is lost; the book lives in the
repository and on GitHub Pages. It never deletes on the Thor either, so
folders removed here (`jetson-thor/landing/`, `jetson-thor/site/`) linger
there as untracked.

## 2026-10-06

### Later the same evening

- **Thinking panel, second fix.** The first fix (87793db) still moved the
  panel to a new message node on every frame. Chrome and Safari drop a
  click whose press landed on a node that was detached, even briefly. Now
  the panel never leaves the page; only the parts around it are replaced.
  Tested with real pointer input over WebDriver BiDi in Firefox on the Thor
  (press, 150 ms hold, release): before any fix it didn't open; with either
  fix it opened. No Chrome or Safari on either machine, so those weren't
  tested directly. The page is sent with `Cache-Control: no-store`.
- **Lightning switched off** at the user's request (disappointing answers):
  unloaded, then `LIGHTNING=none` added to `~/.config/thor-chat/env` on the
  Thor and `./serve.sh reload`, so it is off the list too. The Nano stayed
  loaded. Memory available: 60.2 GB. The file stays in `~/models/gguf/`.
- **Chat page code:** `updateLast` now patches the streaming message by
  named parts (`data-part`); only the thinking panel is patched in place,
  the rest is replaced. The picker code lost its fake `"model"` entry
  (`showServer`, `showModels`, `chosenModel`). Tested with real pointer
  input, with one model and with two.
- **serve.sh moved** to `jetson-thor/model-serving/serve.sh`. The Thor's two
  unit files were repointed with `sed` + `daemon-reload`, without a restart.
  The old copy on the Thor was deleted by hand (thor-sync never deletes).
- **New commands:** `models`, `memory`, `load` (guarded: one-token warm-up,
  undone below `MIN_FREE_GB`), `unload`, `reload`, plus
  `~/.config/thor-chat/models.local.ini` for models being tried. All were
  tested on the Thor.
- **Book:** chapter "Model serving" (`ch21-model-serving.md`), placed right
  after "Access and syncing"; figures renumbered in the chapters after it.
  The `/slots` commands in ch09 and ch20 now pass `?model=nemotron`, which
  router mode requires.
- **Open question from the user:** why llama.cpp and not TensorRT-LLM or
  TensorRT Edge-LLM. The Thor has TensorRT 10.16.2 (`pip`, `libnvinfer`),
  but no TensorRT-LLM and no container. Support for these models on sm_110
  hasn't been checked.

### Nemotron 3.5 Lightning as a second model

- The model exists: NVIDIA, released 2026-08-11, 30B total / 3B active,
  Mamba-2 + MoE + attention, 1M context, OpenMDW-1.1 licence.
- On the Thor:
  `~/models/gguf/Nemotron-3.5-Lightning-30B-A3B/NVIDIA-Nemotron-3.5-Lightning-30B-A3B-Q8_0.gguf`
  (unsloth GGUF, 35,004,643,392 bytes, size matches Hugging Face). The Thor's
  llama.cpp (8216c84, 2026-10-05) runs it.
- `serve.sh run` now starts llama-server in router mode
  (`--models-preset ~/.config/thor-chat/models.ini --models-max 2`). The Nano
  is unchanged (4 × 1M, aliases `nemotron`, `nemotron-think`, its GGUF path).
  Lightning gets 1 × 256K and the alias `lightning`. The user chose this
  split.
- Chat page: the model chip is a picker filled from `/v1/models`. The choice
  is remembered and sent as `model`. Tested in headless Firefox on the Thor.
- Measured with both loaded: Nano 53.5 tok/s, Lightning 52.5 tok/s (same Rust
  prompt, thinking off, 600 tokens). Both loaded 32 s after a restart; 26 GB
  of memory still available.
- **Behaviour change:** a request without `model`, or with an unknown name,
  now gets 400. Before, the name was ignored. Lasso's default `--model teacher`
  would be refused if pointed at :8079 or :8080.
- **Incident, 20:49–20:52 PDT:** I first loaded Lightning on a test port at
  4 × 1M next to the live Nano. Memory ran out on the first generation and
  the OOM killer took `thor-chat` down. systemd restarted it on the
  router-mode `serve.sh` that thor-sync had already copied (both models at
  4 × 1M), which crash-looped. Fixed by restoring the committed `serve.sh`
  and restarting. Later tests ran in a 48 GB systemd scope with a watchdog,
  and the deploy had an automatic rollback.

### Chat page: thinking panel (87793db)

Clicking "thinking…" while the answer streamed did nothing. The page
rebuilt the last message on every animation frame, so the mouse went down
on one element and came up on its replacement, and the browser dropped the
click. The fix keeps the same panel and updates only its text. Reproduced
and verified in headless Firefox on the Thor with a real press and release
(a scripted `click()` did not show the bug). Deployed by copying the page;
the agent reads it from disk on every request.

### Coverage (2103674)

100.00% line coverage in every crate, 317 tests, CI green. What it took:
`cargo llvm-cov` counts each function by its best single compiled copy, so
generic code and closures that never run show up as misses. They were
replaced with trait objects (`&mut dyn Write`), channels instead of
`join().map_err(...)`, `serve() -> Outcome<Infallible>`, and tests for the
real error paths.

### Live chat prompt and temperature, reverted (3aaf81a, 70aecbd, 7c4aacf, 0e6fe2e)

I gave chat requests a default system prompt and temperature 0.3. Its "at
most three short sentences" line cut every answer short. Reverted at the
user's request and redeployed. Rule since then: no change to how the live
chat answers without the user approving that exact change.

The "Nemotron regressed" report had no server-side cause. It was sampling
at temperature 1.0 with long histories. The rules plus temperature 0.3
improved style but not correctness. The options offered (compile-and-retry,
a stronger coding model, the fine-tune) are still undecided.

### Teacher set (1ad650b, d67fcd8, 361817c)

- `train/teacher/01-10*.md`: 60 hand-written conversations. Keep them; the
  user asked that they never be deleted.
- `train/teacher/grounded/*.md`: conversations grounded in real sections of
  books, docs and open-source code, one file per source, with source,
  section and licence on every entry.
- `teacher pick N` queues real sections; `teacher check` builds, tests and
  rule-checks every fence.
- Totals at the last check: 106 entries pass, 46 of them grounded.
- Not used: the gpu-accelerated-kubernetes book (user's call),
  cpp-core-guidelines (personal-use licence), and gobyexample (licence only
  stated in its README; awaiting the user's call).
- Open courseware is not fetched yet. Check each licence first; NC licences
  are refused.

### Review page (2676c49, b06944b)

`reinforcer` serves train, teacher and conversations on one port
(127.0.0.1:8787), with a dark page, Good (`a`) / Slop (`f`) and categories
on a selection (keys 1–8). Hammer's old review binary is gone.

### Book

ch13 §7.7 (the running-median case study, gaps G1–G6) and §7.8 (the teacher
set), with three figures.

## 2026-10-04 to 2026-10-05

Platform, Stage 0 checker (spark), web chat on the Thor with web search,
the book on GitHub Pages, voltforge.tech forwarding, OpenCode and Claude Code
pointed at the Thor, lasso. See `git log` from 72423d3 to 627358f.

## Rules learned the hard way

- Do only what was asked; no adjacent edits, commits or pushes.
- Never change the live chat's prompts, sampling or served model without
  the user's approval of that exact change.
- No long GPU jobs on the Thor unless asked, and only in a time window the
  user gave.
- Draft changes to `jetson-thor/model-serving/serve.sh` outside the repo: thor-sync
  puts them on the Thor at once, and the next restart runs them.
- No Cloudflare. No AI attribution in commits.
