# Fine-tuning Gemma 4 with LoRA

LoRA fine-tuning for Gemma 4 26B-A4B (mixture of experts) and Gemma 4 31B (dense), in bf16 on
the Thor or in 4-bit QLoRA on a Colab A100 or H100, from one script and one data format.

The goal is a model that works through openbatrangs's tools (read, edit, run the tests) before
it answers, writes simple and idiomatic code, checks with a tool or says it does not know
instead of guessing, and writes plain technical English without narrating its own steps.

| File | Purpose |
|---|---|
| `lora.py` | train an adapter, merge it into the base model, or try a model on one prompt |
| `run_thor.sh` | the overnight run on the Thor, with the free-memory check |
| `colab_train.ipynb` | the same training in Colab, with checkpoints on Google Drive |
| `requirements.txt` | pinned transformers, peft, TRL, accelerate, datasets, bitsandbytes |
| `mypy.ini` | `mypy --strict` settings for `lora.py` |
| `fetch_corpus.sh`, `teacher/` | inputs of `thor-hammer-trainer` (Chapter 2 of the book) |

## Building the data

Training data comes from three places, and `lora.py` takes any number of JSONL files at once.

**The existing set.** `thor-hammer-trainer` builds `data/train.jsonl` from the hand-written
pairs, the verified readable rewrites, and sections of books and docs:

```bash
bash train/fetch_corpus.sh
target/release/thor-hammer-trainer data
```

Its records have the fields `instruction`, `response`, `source` and `origin`. In `lora.py`, each
record becomes one user turn and one assistant turn. A book or doc section has no instruction,
so its user turn reads "Write the documentation section `origin`." until `lasso` writes a real
question for it. Records whose source is `chat` are left out, because that source is an export
of Claude conversations, and Claude output is not used as training data.

**Agent runs.** The `lasso agent` commands ([#38](https://github.com/arpanpathak/thor-thunder-tigress-platform/issues/38),
in progress) turn merged pull requests into coding tasks and record how a model solves them:

| Command | What it does |
|---|---|
| `lasso agent tasks` | builds tasks from merged PRs that close an issue and change test files: the issue text as the prompt, the base commit, the PR's tests and a check command; about 20% go to an eval split |
| `lasso agent run` | checks out the base commit, copies in the tests, runs `openbatrangs --workspace DIR --transcript-out FILE` on the prompt, runs the check, and appends one record per task |
| `lasso agent score` | pass rate, mean steps, step-limit and error counts |
| `lasso agent export` | passed runs as `{"tools","messages"}` training lines; runs flagged as slop are left out |

The teacher is DeepSeek V4 Pro (`deepseek-v4-pro`). The same commands score the student, Gemma
on the Thor through `openbatrangs --thor`, on the eval split.

**Review.** Before a run goes into training, a person reads it in the reinforcer at
`http://127.0.0.1:8787`. The agent runs tab ([#37](https://github.com/arpanpathak/thor-thunder-tigress-platform/issues/37),
in progress) shows the task, every tool call with its arguments and output, pass or fail, the
step count and the final answer. The `a` key marks a run Good and `f` marks it Slop. Slop flags
go to `labels/agent_run_flags.jsonl`, and `lasso agent export --labels` leaves those runs out.

## Data format

One JSON object per line, in the OpenAI chat format that `tokenizer.apply_chat_template(messages,
tools=tools)` accepts. Tool-call arguments may be a JSON string, as OpenAI sends them, or an
object. A short agent run, shown here on several lines:

```json
{"tools": [{"type": "function", "function": {"name": "run", "description": "Run a shell command.",
   "parameters": {"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}}}],
 "messages": [
  {"role": "system", "content": "You are a coding agent."},
  {"role": "user", "content": "parse(\"\") panics. Make it return an empty list."},
  {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function",
   "function": {"name": "run", "arguments": "{\"command\": \"cargo test parse_empty\"}"}}]},
  {"role": "tool", "tool_call_id": "call_1", "content": "test parse_empty ... FAILED"},
  {"role": "assistant", "content": "parse now returns an empty list for empty input, and cargo test passes."}]}
```

The loss counts only the tokens the model writes. These are its tool calls, its answers, and
the token that ends each of them (`<|tool_response>` after a tool call, `<turn|>` after an
answer). System
prompts, user turns, tool definitions and tool output are context. For the example above, the
trained text is `<|tool_call>call:run{command:<|"|>cargo test parse_empty<|"|>}<tool_call|><|tool_response>`
and then the final answer with its `<turn|>`.

The Gemma 4 template moves the text of a tool-calling assistant turn after the tool's output,
so it reads as part of the next answer. `lasso agent export` drops that text for this reason.

## LoRA targets

`lora.py` reads the model's config and reports which kind of model it loaded. In both models,
every language-model layer has attention (`q_proj`, `k_proj`, `v_proj`, `o_proj`) and a dense
MLP (`gate_proj`, `up_proj`, `down_proj`), and those get adapters. In 26B-A4B, each layer also
has 128 experts stored as two 3-D tensors beside that MLP, and a router. The experts and the
router stay frozen, so the adapter trains the parts every token passes through. The vision tower
also stays frozen.

At rank 16, the adapter of 26B-A4B has about 19 million parameters, under 0.1% of the base
model's 25 billion.

## Training on the Thor

The Thor trains in bf16, with the base weights frozen in the shared 128 GB memory. The
`~/finetune` venv has the Jetson build of torch 2.11 and the packages in `requirements.txt`
except bitsandbytes.

A run takes the GPU and most of the memory for hours, so it happens only inside a time window
the owner of the Thor has agreed to, usually overnight, with serving paused. `run_thor.sh` never
stops a service. When memory is short, it prints the command that pauses the running engines
and exits. When training ends, it has already printed the command that starts them again.

```bash
train/run_thor.sh 26b data/train.jsonl data/agent.jsonl
train/run_thor.sh 31b data/train.jsonl data/agent.jsonl -- --epochs 2
```

The adapter, checkpoints and `train.log` go to `runs/lora-26b` or `runs/lora-31b`. A checkpoint
is written every 50 steps and the script always resumes from the newest one, so a run cut short
continues in the next window with the same command.

## Training in Colab

Open `colab_train.ipynb` in Colab, choose an A100 or H100 runtime, and run the cells from top to
bottom. The notebook asks for the training JSONL once and stores it on Google Drive with the
checkpoints and the adapter. After a disconnect, running every cell again continues from the
last checkpoint. The base model loads in 4-bit NF4 (QLoRA) and the adapter trains in bf16.

Gemma models are gated on Hugging Face. Accept the licence on the model page and sign in when the
notebook asks, or store a token as the Colab secret `HF_TOKEN`.

## Merging

Serving needs the adapter merged into the base weights, then quantized back to NVFP4 for
TensorRT Edge-LLM. Merging runs on the Thor in bf16, from the local copy of the base model, so
that `processor_config.json` is copied along:

```bash
~/finetune/bin/python train/lora.py --merge --model ~/models/hf/gemma-4-26B-A4B-it \
    --adapter runs/lora-26b --output-dir runs/merged-26b
~/finetune/bin/python train/lora.py --generate --model runs/merged-26b
```

`--generate` prints the model's answer to one tool-calling prompt, with special tokens, so the
tool-call format is visible. It also takes `--adapter` to try an adapter without merging. In
the venv, set `LD_LIBRARY_PATH` as `run_thor.sh` does.

## Options

| Option | Default | Meaning |
|---|---|---|
| `--model` | required | local folder or Hugging Face id |
| `--data` | none | training JSONL files |
| `--precision` | `bf16` | `bf16` on the Thor, `4bit` (QLoRA) in Colab |
| `--max-length` | 4096 | longest example in tokens; longer ones are cut at the end |
| `--epochs` | 1 | passes over the data |
| `--max-steps` | -1 | stop after this many optimizer steps; -1 runs every epoch |
| `--learning-rate` | 1e-4 | peak of a cosine schedule with 10 warmup steps |
| `--rank` | 16 | LoRA rank; alpha is twice the rank, dropout 0.05 |
| `--gradient-accumulation` | 16 | examples per optimizer step, one example per forward pass |
| `--save-steps` | 50 | checkpoint interval in optimizer steps |
| `--resume` | off | continue from the newest checkpoint in `--output-dir` |
| `--output-dir` | `runs/lora` | adapter and checkpoints; merged model with `--merge` |
| `--merge`, `--generate`, `--adapter`, `--new-tokens` | | merge and try a model |

## Memory and time (estimates)

The numbers below are estimates, not measurements of a full run. Only the logits figure was
measured, on a 2-layer model with the real 262,144-token vocabulary. The logits of one
4,096-token example and their gradients take 16 GB, and 32 GB at 8,192 tokens, so the default
length is 4,096.

| Setup | Base weights | Peak memory | Time per epoch of the current data |
|---|---|---|---|
| Thor, 26B-A4B, bf16 | 49 GB | about 75 GB | 2 to 5 hours |
| Thor, 31B, bf16 | 59 GB | about 87 GB | 4 to 8 hours |
| Colab A100 80 GB or H100, 26B-A4B, 4-bit | about 15 GB | about 35 GB | 2 to 4 hours |
| Colab A100 80 GB or H100, 31B, 4-bit | about 18 GB | about 40 GB | 3 to 6 hours |

The data is the `thor-hammer-trainer` set without its chat source, about 11,000 records and
5 million tokens. With 16 examples per step, one epoch is about 690 optimizer steps. On a 40 GB A100,
use `--max-length 2048`; the notebook picks it from the GPU's memory.

## Not done yet

- The 4-bit path has not run yet, because bitsandbytes is not installed on the Thor. The first
  Colab run tests it.
- No run on the real 26B-A4B or 31B weights yet.
- Quantizing the merged model to NVFP4 and building the Edge-LLM engine are separate steps that
  this folder does not cover.
- The experts of 26B-A4B get no adapter. PEFT can wrap 3-D expert tensors (`target_parameters`),
  which would train the experts too at a higher memory cost.
- Gemma 4's generation prompt opens each answer with an empty thinking block
  (`<|channel>thought\n<channel|>`); the training examples have none.
- No per-source weighting: the curated sets are not repeated, as Chapter 5 of the book plans.
- No fused cross-entropy, so the logits take 16 GB at 4,096 tokens. Liger's fused linear
  cross-entropy computes the loss without storing the full logits.
- The DPO pass on `preferences.jsonl` is not written.
