<img class="cub" src="art/cub.svg" alt="The Thor Tigress Cub">

# Model serving

The Thor serves Nemotron 3 Nano, with Nemotron 3.5 Lightning listed as a
second option that is loaded only when someone asks for it. This chapter shows how to see what is loaded, take a model out of
memory, put it back, try a model nobody has run here before, and make a new
model the default. It also lists the models worth trying as of October 2026,
with their sizes and whether they fit.

Everything here runs on the Thor, from the folder `serve.sh` is in:

```bash
ssh thor
cd ~/Projects/thor-thunder-tigress-platform/jetson-thor/model-serving
```

`jetson-thor/model-serving/serve.sh` runs both services: `serve.sh run` is
the model server, `serve.sh agent` the page and API, which it serves from
`jetson-thor/web/`.

<div class="covers">

This chapter covers

- what runs: one llama-server router and one process per loaded model
- the five commands: `models`, `memory`, `load`, `unload`, `reload`
- trying a new model safely, step by step, and removing it again
- making a model the default, or leaving one out for good
- the memory budget, measured, and how to estimate a model's cost and speed
- models to try, with real file sizes and where each fits
- what goes wrong and how to tell

</div>

## What runs

<figure>
<img src="figures/model-router.svg" alt="Clients send requests with a model name to thor-tigress-agent on port 8080, which passes them to the llama-server router on 8079. The router sends each request to the model instance with that name or alias.">
<figcaption><b>Figure 7.1</b> How a request reaches a model.</figcaption>
</figure>

`thor-chat` runs `serve.sh run`, which starts llama-server in **router
mode**. The router holds no model itself. For each model it loads, it starts
a separate llama-server process on a private port and forwards requests to
it by the request's `model` field.

| Part | Where | What it does |
|---|---|---|
| `thor-chat` | systemd user service → `serve.sh run` | the router on `127.0.0.1:8079` |
| one llama-server per loaded model | children of the router, private ports | the model's weights, context and slots |
| `~/.config/thor-chat/env` | optional | settings: `MODEL`, `LIGHTNING`, `USERS`, `CONTEXT`, `MIN_FREE_GB`, `MODELS_MAX`, … |
| `~/.config/thor-chat/models.ini` | written by `serve.sh` at every start and `reload` | the presets the router reads; don't edit it, it is overwritten |
| `~/.config/thor-chat/models.local.ini` | yours, optional | models you are trying; appended to `models.ini` |
| `~/models/gguf/` | model files | one folder per model |
| `thor-tigress-agent` | `:8080` | passes `model` through unchanged; the web chat, Claude Code and OpenCode all go through it |

The router's rules, read from llama.cpp's source (`tools/server/server-models.cpp`,
commit `8216c84`):

- **Names.** A request must name a model by its id or one of its aliases.
  No name gives `400 model name is missing from the request`. An unknown
  name gives `400 model 'x' not found`.
- **Autoload.** A request for a model that isn't loaded loads it first. The
  request waits, about 12 to 32 seconds here.
- **At most `MODELS_MAX` loaded** (default 2). Loading one more first
  unloads the **least recently used** model. That can be the Nano, which is
  everyone's default.
- **Reload.** `GET /models?reload=1` re-reads `models.ini`. New sections are
  listed but not loaded. Removed sections are unloaded and dropped. A
  running model whose section changed is **unloaded**. An unchanged one
  keeps running.

The models listed today:

| Id | Aliases | At start | Slots × context | Memory when loaded |
|---|---|---|---|---|
| `NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0` | `nemotron`, `nemotron-think`, its file path | loaded | 4 × 1,048,576 | 57.8 GB |
| `NVIDIA-Nemotron-3.5-Lightning-30B-A3B-Q8_0` | `lightning` | not loaded (`load-on-startup = false`) | 1 × 262,144 | 35.4 GB |

Lightning was taken out of memory on 2026-10-06 because its answers were
disappointing. It stays in the list: picking it in the web chat, or
`./serve.sh load lightning`, loads it in about 13 seconds.

## The five commands

### See what is there: `models`

```text
$ ./serve.sh models
NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0              loaded                   /home/arpanpathak/models/gguf/Nemotron-3-Nano-30B-A3B/NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0.gguf, nemotron, nemotron-think
NVIDIA-Nemotron-3.5-Lightning-30B-A3B-Q8_0       unloaded                 lightning
```

The states are `loaded`, `loading`, `unloaded`, `sleeping`, and
`unloaded (failed, exit N)` when the model's process died. The cause of a
failure is in `journalctl --user -u thor-chat`.

### See the memory: `memory`

```text
$ ./serve.sh memory
MemTotal:        122.8 GB
MemAvailable:     60.3 GB
loaded:        NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0
```

The Thor's CPU and GPU share one memory. A process's own size (RSS) leaves
out its GPU buffers: the Nano's process shows 2.3 GB while it really costs
57.8 GB. The honest measure of what a model costs is how much
`MemAvailable` changes when it loads or unloads.

### Take a model out of memory: `unload`

```text
$ ./serve.sh unload lightning
{"success":true}
$ ./serve.sh memory
MemTotal:        122.8 GB
MemAvailable:     60.2 GB
loaded:        NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0
```

Measured on 2026-10-06: available memory went from 24.8 to 60.2 GB, so
Lightning costs 35.4 GB. Any reply Lightning was writing is cut off.

`unload` doesn't keep a model out. The next request that names it loads it
again: someone picking it in the web chat, or a tool sending `lightning`.
To keep it out until you say otherwise, see "Leave a model out" below.

### Put it back: `load`

```text
$ ./serve.sh load lightning
lightning loaded in 13 s; committing its memory with one token
MemTotal:        122.8 GB
MemAvailable:     25.3 GB
loaded:        NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0
loaded:        NVIDIA-Nemotron-3.5-Lightning-30B-A3B-Q8_0
```

`load` does more than ask the router:

1. It refuses a name the router doesn't know (exit code 2).
2. It asks the router to load the model and waits for `loaded`.
3. It sends the model a one-token request. On the Thor, memory is committed
   when it is first used, so a model can look loaded with plenty free and
   still run out on its first reply. That is what took the chat down on
   2026-10-06 (see "What went wrong once").
4. Throughout, if `MemAvailable` falls below `MIN_FREE_GB` (default 8), it
   unloads the model again and exits with code 1.

The guard was tested by forcing it with an impossible floor:

```text
$ MIN_FREE_GB=200 ./serve.sh load lightning
free memory fell below 200 GB while loading; lightning unloaded
$ ./serve.sh models | grep Lightning
NVIDIA-Nemotron-3.5-Lightning-30B-A3B-Q8_0       unloaded (failed, exit 1) lightning
```

"failed" there only records that the process was stopped mid-load. The next
`load` clears it.

The guard checks once a second. It can't stop a load that eats the last
8 GB in under a second, and it doesn't watch later replies, which use more
context than the warm-up. The budget below is what keeps you safe; the
guard is the second line.

### Pick up new presets: `reload`

```text
$ ./serve.sh reload
NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0              loaded                   /home/…/NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0.gguf, nemotron, nemotron-think
NVIDIA-Nemotron-3.5-Lightning-30B-A3B-Q8_0       loaded                   lightning
my-model                                         unloaded                 mine
```

(From a test on 2026-10-06 with a test section in `models.local.ini`. The
test used a different model; its id and alias are shown here as the
placeholders the steps below use.)

`reload` rewrites `models.ini` from the settings and `models.local.ini`,
then asks the router to re-read it. No restart, no reply cut off, unless you
changed the section of a model that is running: that model is unloaded.

## Try a new model, step by step

The steps use `my-model` as the id and `mine` as the alias. Put in the
model you want to try.

### 1. Check that it fits

Find the file size first (see "Models to try" for many already looked up),
then compare it with the budget:

| What stays loaded | Available | Room for a new model (keeping 8 GB free) |
|---|---|---|
| Nano only (today) | 60.3 GB measured | about 52 GB |
| Nano and Lightning | 24.8 GB measured | about 16 GB |
| nothing (unload both; the chat is down for anyone not using the new model) | about 118 GB, computed: 60.2 + 57.8 | about 110 GB |

A model needs its file size, plus its context, plus about 2 GB of working
buffers. How much context costs depends on the architecture (next section),
so **start with one slot and a small context**, measure, then grow.

### 2. Check that llama.cpp knows the architecture

The architecture is in the GGUF's metadata, and Hugging Face shows it:

```bash
REPO=owner/Some-Model-GGUF          # the Hugging Face repository
curl -s https://huggingface.co/api/models/$REPO |
  python3 -c "import sys,json; print(json.load(sys.stdin)['gguf']['architecture'])"
# for example: nemotron_h_moe
grep -c '"nemotron_h_moe"' ~/.local/src/llama.cpp/src/llama-arch.cpp
# 1: known. 0: rebuild llama.cpp first (below).
```

Every model in the table below was checked this way against the Thor's
llama.cpp (commit `8216c84`, 2026-10-05).

### 3. Download it

List the files in a repository with their sizes:

```bash
curl -s https://huggingface.co/api/models/$REPO/tree/main |
  python3 -c "import sys,json; [print(f['path'], round(f.get('size',0)/1e9,1), 'GB') for f in json.load(sys.stdin)]"
```

Then download into its own folder. `-C -` resumes a broken download:

```bash
mkdir -p ~/models/gguf/my-model && cd ~/models/gguf/my-model
curl -L -C - -O https://huggingface.co/$REPO/resolve/main/FILE.gguf
```

The Thor downloads at about 69 MB/s (measured), so 35 GB takes about 9
minutes. Big models come in parts (`…-00001-of-00003.gguf`, in a folder named
after the quant). Download every part into one folder; the preset names the
first part.

Check the size against the listing before using the file: `ls -l`.

### 4. Add it to `models.local.ini`

```ini
[my-model]
model = /home/arpanpathak/models/gguf/my-model/FILE.gguf
alias = mine
parallel = 1
ctx-size = 65536
load-on-startup = false
```

| Key | Meaning |
|---|---|
| `[name]` | the id: what `models` prints, what the web chat's picker shows, what requests send |
| `model` | absolute path to the GGUF (the first part, if split) |
| `alias` | other names, comma-separated; must not clash with any other id or alias |
| `parallel` | slots: replies at once |
| `ctx-size` | tokens for **all** slots together; each slot gets `ctx-size / parallel` |
| `load-on-startup` | `false` while trying it: a restart of `thor-chat` won't load it |

Any llama-server option works as a key, without the dashes:
`cache-type-k = q8_0`, `n-gpu-layers = 999`, `chat-template-file = …`.
`n-gpu-layers = 999`, `flash-attn = on` and `jinja = true` already come from
the `[*]` section.

### 5. Make it known: `reload`

```bash
./serve.sh reload
```

It appears as `unloaded`. **It also appears in the web chat's model picker
for everyone, at once.** Anyone who picks it loads it, and with
`MODELS_MAX=2` that unloads whichever loaded model was used least recently.
Try models when nobody else is chatting, or keep the test short.

### 6. Make room, then load it

With Lightning unloaded there are about 52 GB of room. If Lightning is
loaded and the budget says the new model doesn't fit next to it, unload it
first:

```bash
./serve.sh unload lightning
./serve.sh load mine
```

`load` prints the time it took and the memory after one token. The memory
the model really costs is the drop in `MemAvailable`.

### 7. Measure it

The same prompt used for the Nano and Lightning:

```bash
K=$(cat ~/.config/thor-chat/api-key)
curl -s 127.0.0.1:8079/v1/chat/completions -H "Authorization: Bearer $K" \
  -H "Content-Type: application/json" \
  -d '{"model":"mine","messages":[{"role":"user","content":"Write a Rust function that returns the median of a slice of f64, with a test."}],"chat_template_kwargs":{"enable_thinking":false},"max_tokens":600}' |
  python3 -c "import sys,json; t=json.load(sys.stdin)['timings']; print(round(t['predicted_per_second'],1),'tok/s')"
```

| Model | tok/s, measured 2026-10-06 |
|---|---|
| Nemotron 3 Nano, Q8_0 | 53.5 |
| Nemotron 3.5 Lightning, Q8_0 | 52.5 |

Write your result in the table in `WORKLOG.md` with the date, quant, slots
and context. A speed without those settings can't be compared.

### 8. Take it away again

```bash
./serve.sh unload mine
# delete its section from ~/.config/thor-chat/models.local.ini, then:
./serve.sh reload
rm -r ~/models/gguf/my-model      # only if you won't try it again
```

Steps 4, 5 and 8 were run on 2026-10-06 with a test section. The model was
listed, then gone after the second reload. The models already loaded stayed
loaded throughout.

## Change what is served for good

### Make a model the default or the second model

`MODEL` is the model listed first, which the page picks for people who never
chose. `LIGHTNING` is the second. Point either at another file in
`~/.config/thor-chat/env`:

```bash
MODEL=/home/arpanpathak/models/gguf/my-model/FILE.gguf
USERS=2
CONTEXT=131072
```

Then restart. A restart cuts off every reply being written, so wait until
no slot is busy:

```bash
K=$(cat ~/.config/thor-chat/api-key)
until [ "$(curl -s "127.0.0.1:8079/slots?model=nemotron" -H "Authorization: Bearer $K" |
           python3 -c "import sys,json;print(sum(s['is_processing'] for s in json.load(sys.stdin)))")" = 0 ]; do sleep 2; done
systemctl --user restart thor-chat
./serve.sh memory
```

The aliases `nemotron` and `nemotron-think` follow `MODEL`, so Claude Code
and OpenCode keep working when the default changes. They will then get the
new model under the old name.

### Leave a model out

```bash
echo 'LIGHTNING=none' >> ~/.config/thor-chat/env
./serve.sh reload        # unloads Lightning and drops it from the list
```

A missing file has the same effect. To bring it back, delete the line and
`reload`, then `./serve.sh load lightning`.

### Load Lightning at start again

Lightning's section says `load-on-startup = false`. To load it at every
start, change that line in `serve.sh` (the `presets` function) to `true`.

### More than two at once

`MODELS_MAX=3` in the env file, then restart `thor-chat`. With the Nano and
Lightning loaded, a third model has about 16 GB. Only small models fit (see
the table).

### Free memory when idle

llama-server can unload a model after a quiet period and reload it on the
next request (`--sleep-idle-seconds N`, or `sleep-idle-seconds = N` in a
preset section). It isn't used here: the first message after a quiet
period would wait 12 to 32 seconds.

### Stop everything

```bash
systemctl --user stop thor-chat     # all models out of memory; the chat answers 502
systemctl --user start thor-chat    # the Nano back in about 15 s
```

## The memory budget

<figure>
<img src="figures/model-memory.svg" alt="A bar of the Thor's 122.8 GB: Nemotron 3 Nano 57.8 GB, Lightning 35.4 GB, OS and other programs 4.8 GB, 24.8 GB available.">
<figcaption><b>Figure 7.2</b> Where the memory goes with both models loaded (measured before Lightning was unloaded).</figcaption>
</figure>

| Part | Size | How we know |
|---|---|---|
| Nemotron 3 Nano, 4 × 1,048,576 | 57.8 GB | llama-server's own report at start (chapter "Memory, context and slots") |
| Nemotron 3.5 Lightning, 1 × 262,144 | 35.4 GB | `MemAvailable` before and after `unload`, 2026-10-06 |
| OS, agent, SearXNG | 4.8 GB | the remainder |
| available | 24.8 GB | `./serve.sh memory` |

### Estimate a model before downloading it

```text
memory ≈ file size + context + about 2 GB
```

Context costs very different amounts per token depending on the
architecture:

- **Hybrid Mamba models** (Nemotron 3 Nano, Lightning, Nemotron 3 Super)
  keep a full key/value cache for a few attention layers only. A million
  tokens costs a few GB per slot, which is why the Nano holds 4 × 1M.
- **Models with mostly sliding-window or linear attention** (gpt-oss, the
  `qwen35` architecture of Qwen3.6-27B and Qwen3.8-27B, whose llama.cpp code
  loads gated delta net layers; Gemma 4) are cheap per token on most layers.
- **Dense models with full attention on every layer** (Devstral Small 2)
  can cost more for the context than for the weights at long context.

The safe way is the one in "Try a new model": one slot, 64K tokens, load,
read the drop in `MemAvailable`, then grow `ctx-size` and `parallel` while
watching `./serve.sh memory`.

### Estimate its speed

Generating one token reads every **active** weight once, and the Thor reads
memory at 273 GB/s. The Nano reads about 3.4 GB per token (3.2B active ×
about 1.06 bytes at Q8_0) and makes 53.5 tok/s, about two thirds of the
273 / 3.4 ≈ 80 limit. So:

```text
tok/s ≈ 180 / (active parameters in billions × bytes per parameter)
bytes per parameter: Q8_0 ≈ 1.06, Q6_K ≈ 0.82, Q4_K_M ≈ 0.60, MXFP4 ≈ 0.53, IQ2 ≈ 0.30
```

This is an estimate, calibrated on one model. MoE routing, attention over a
long context, and several people chatting at once all lower it. A dense
27B model at Q8_0 reads 29 GB per token, so about 6 tok/s. That's why the
fast models in the table are mixtures of experts with few active parameters.

## Models to try

As of 2026-10-06. Sizes come from the Hugging Face file listings of the
GGUF repositories named (unsloth unless noted). Architectures come from
the GGUF metadata, and every one was found in the Thor's llama.cpp.
**Speeds are estimates** from the formula above; only the two Nemotrons have
been measured. "Fits" uses the budget in step 1: **A** next to the Nano and
Lightning, **B** next to the Nano alone (today's state), **C** alone.

### Same class as Lightning: about 30B total, about 3B active

| Model | Total / active | Context | Licence | Quant, size | Fits | Est. tok/s |
|---|---|---|---|---|---|---|
| Nemotron 3 Nano (served) | 30B / 3B | 1M | NVIDIA Nemotron Open Model License | Q8_0, 33.6 GB | served | 53.5 measured |
| Nemotron 3.5 Lightning (listed, unloaded) | 30B / 3B | 1M | OpenMDW-1.1 | Q8_0, 35.0 GB | B | 52.5 measured |
| Qwen3.6-35B-A3B | 34.7B / 3B | 256K | Apache-2.0 | Q8_0, 36.9 GB; Q4_K_XL, 22.4 GB | B | 55 (Q8_0) |
| gpt-oss-20b (OpenAI; ggml-org GGUF) | 20.9B / 3.6B | 128K | Apache-2.0 | MXFP4, 12.1 GB | **A** | 90 |

### Dense, 12B to 27B: strong per parameter, slow here

| Model | Params | Context | Licence | Quant, size | Fits | Est. tok/s |
|---|---|---|---|---|---|---|
| Qwen3.8-27B | 27.3B | 256K | Apache-2.0 | Q8_0, 29.0 GB; Q4_K_XL, 17.6 GB; Q4_K_M, 16.5 GB | B (Q4_K_M: A only with a tiny context) | 6 (Q8_0), 11 (Q4_K_M) |
| Devstral Small 2 (Mistral) | 23.6B | 384K | Apache-2.0 | Q8_0, 25.1 GB; Q4_K_M, 14.3 GB | B; Q4_K_M: A | 7 (Q8_0), 13 (Q4_K_M) |
| Gemma 4 12B | 11.9B | 256K | Apache-2.0 | Q8_0, 13.1 GB | **A** | 14 |

### Large mixtures of experts: only alone, or instead of Lightning at low quants

| Model | Total / active | Context | Licence | Quant, size | Fits | Est. tok/s |
|---|---|---|---|---|---|---|
| Qwen3-Coder-Next | 80B / 3B | 256K | Apache-2.0 | Q4_K_M, 48.5 GB; Q8_0, 84.8 GB | Q4_K_M: B, barely; Q8_0: C | 100 (Q4_K_M), 55 (Q8_0) |
| gpt-oss-120b (OpenAI; ggml-org GGUF) | 116.8B / 5.1B | 128K | Apache-2.0 | MXFP4, 63.4 GB | C | 65 |
| Mistral Small 4 | 119B / 6B | 1M | Apache-2.0 | Q4_K_M, 73.8 GB | C | 50 |
| Nemotron 3 Super | 120.7B / 12B | 1M | NVIDIA Nemotron Open Model License | Q4_K_M, 82.5 GB; IQ2_XXS, 52.7 GB | C | 25 (Q4_K_M) |
| Qwen3.8-Flash-Next | 125B / 6B (+51B n-gram embedding) | 256K | Qwen Community 1.0 | UD-Q2_K_XL, 78.9 GB | C | 65; 2-bit loses quality |
| DeepSeek-V4-Flash-0731 | 284B / not stated in the GGUF card | 1M | MIT | UD-IQ1_M, 86.9 GB | C | not estimated; 1-bit loses a lot |
| GLM-5.3-Flash | 320B / 18B | 1M | MIT | UD-IQ1_M, 97.6 GB | C, barely, tiny context | 33; 1-bit loses a lot |

### Too big for 128 GB at any quant

| Model | Total | Smallest GGUF |
|---|---|---|
| GLM-5.3 | 754B | UD-IQ1_M, 228.5 GB |
| Kimi K3 | 2.78T | UD-IQ1_M, 648.9 GB |

Rankings and benchmark claims come from the model makers and from roundups
such as [Thunder Compute's October 2026 list](https://www.thundercompute.com/blog/best-open-source-llms)
and [vdf.ai's local coding comparison](https://vdf.ai/blog/best-local-llm-for-coding/).
Nothing in this table was measured on our tasks. The way to know is step 7
plus spark on the answers.

## Update llama.cpp for a new architecture

If step 2 printed 0, the model needs a newer llama.cpp. Not run while writing
this chapter; it rebuilds the binary every model uses:

```bash
cd ~/.local/src/llama.cpp
git pull
cmake --build build --config Release -j 14 --target llama-server
```

Then restart `thor-chat` with the wait-for-idle loop above, and update the
commit in chapter "Operations".

## What goes wrong

| You see | Cause | Do |
|---|---|---|
| `400 model name is missing from the request` | a client sends no `model` | send `nemotron`, `lightning` or an id |
| `400 model 'x' not found` | a name that is no id or alias | `./serve.sh models` |
| `400 model is not loaded` | sent with autoload off | `./serve.sh load NAME` |
| `unloaded (failed, exit 1)` | the process died: bad path, unknown architecture, out of memory, or stopped by `load`'s guard | `journalctl --user -u thor-chat -n 50` |
| a loaded model unloaded by itself | a third model was loaded and `MODELS_MAX` evicted the least recently used; or `reload` after its section changed | `./serve.sh models`; `./serve.sh load NAME` |
| `thor-chat … oom-kill` in the journal | memory ran out; the whole router restarts | `./serve.sh memory`; lower `ctx-size`/`parallel`, or unload something |
| the first message to a model takes 15 to 30 s | it was unloaded and is being loaded | normal; `./serve.sh load NAME` beforehand |
| a model you are trying shows in everyone's picker | every section in `models.ini` is listed | expected; remove the section and `reload` when done |

## What went wrong once

On 2026-10-06, Lightning was first tried with the Nano's settings (4 × 1M)
on a test port, next to the live Nano. Both loaded, and memory still looked
fine. On the first reply memory ran out, and the kernel's OOM killer stopped
`thor-chat`. systemd restarted it on a `serve.sh` that tried to load both at
4 × 1M, which ran out again and kept failing. The chat was down from 20:49
to 20:52 until the old script was put back.

Three things came out of it:

- Lightning got one slot of 256K.
- `load` sends a token and watches memory.
- The rule: **work out the budget before loading anything, and start small.**
