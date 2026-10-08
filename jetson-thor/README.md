# Jetson Thor

The book has the full guide: https://arpanpathak.github.io/thor-thunder-tigress-platform/ (chapters 4 to 7).

The Thor is `192.168.0.189`, user `arpanpathak`. On any machine set up as
below it is just `thor`. "Your machine" in this file means whichever Linux or
macOS machine you work from; this setup was built and tested from a Jetson
Orin NX named yahboom.

## Log in

```bash
ssh thor
```

No IP and no password. This comes from a block in `~/.ssh/config` and the key
`~/.ssh/id_ed25519`. To set the same up on another machine:

```bash
ssh-keygen -t ed25519 -N "" -f ~/.ssh/id_ed25519      # skip if the key exists
ssh-copy-id arpanpathak@192.168.0.189                  # asks for the password once
cat >> ~/.ssh/config <<'EOF'

Host thor
    HostName 192.168.0.189
    User arpanpathak
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 30
    ControlMaster auto
    ControlPath ~/.ssh/cm-%r@%h:%p
    ControlPersist 10m
EOF
```

`ControlMaster` keeps one connection open for 10 minutes, so later `ssh thor`
and `scp` calls start instantly.

Run one command without logging in: `ssh thor 'ollama ps'`.
Copy a file: `scp notes.md thor:` or `scp thor:Projects/x.log .`

## Keep projects in sync

`thor-sync` (installed in `~/.local/bin` on your machine) copies folders to the same place
under the Thor's home, e.g. `~/Projects/openbatrangs` → `~/Projects/openbatrangs`.

```bash
thor-sync              # sync everything on the list
thor-sync add          # add the folder you are in
thor-sync rm           # take it off the list
thor-sync ls           # show the list
thor-sync on / off     # background sync, every change within a few seconds
```

On the list now: `thor-thunder-tigress-platform`, `openbatrangs`, `edgechat`.
Background sync is on. Git history is included, so `git` works on the Thor.
Skipped: anything `.gitignore` skips, plus `target/`, model files and the
chat export (edit `~/.config/thor-sync/exclude`). Sync goes one way, your
machine to the Thor: commit on your machine, because a commit made on the Thor
is overwritten.

## openBatarangs on the Thor

```bash
ssh thor
cd ~/Projects/some-project
openbatrangs --thor                           # interactive TUI on Nemotron
openbatrangs --thor "fix the failing test"    # one task (thinking on; --no-think to turn off)
openbatrangs -m qwen3.6:27b --max-ctx 32768 "fix the failing test"   # Ollama instead
openbatrangs --read-only "explain this repo"  # no file writes or commands
openbatrangs doctor                           # check Ollama and the model
```

After changing openBatarangs on your machine, rebuild it on the Thor:

```bash
ssh thor 'cd ~/Projects/openbatrangs && ~/.cargo/bin/cargo install --path .'
```

On a machine with the SSH tunnel (book, chapter "Local models") the same
`openbatrangs --thor` works.
From any other machine, including macOS: `--openai-url
https://arpanpathak.taildb9a39.ts.net/v1 --api-key-file
~/.config/thor-chat/api-key` (book, chapter "Bring your own agent").

## Claude Code with Nemotron

`claude-thor` (thinking off) and `claude-thor-think` (thinking on) are shell
aliases that run Claude Code against Nemotron, through the SSH tunnel to
`thor-tigress-agent` on port 8080 or through the public address, with their own settings, history and memory
in `~/.claude-thor`. Plain `claude` is unchanged. Measured: thinking on works
but is slow (207 s for a small crate); thinking off is fast but ignored the
task. openbatrangs is the better agent here.

```bash
mkdir -p ~/cc-thor-test && cd ~/cc-thor-test && claude-thor
```

The alias, the macOS version and what each variable does: book, chapter
"Bring your own agent". The public address passes `/v1/messages` (Anthropic
API) to llama-server and accepts the key as `x-api-key` or bearer.

## local-copilot-codebuddy with Nemotron on the Thor

Built with llama.cpp for the Thor's GPU (sm_110) and no TensorRT-LLM:

```bash
ssh thor
local-copilot-codebuddy ~/models/gguf/Nemotron-3-Nano-30B-A3B/NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0.gguf
```

With no argument it lists the models in `~/models`. It uses the model's full
context (1,046,528 tokens for Nemotron 3 Nano) unless `--kv-cache-tokens` caps
it. Your coding rules are in `~/.config/local-copilot-codebuddy/rules.md` and
go into every conversation.

To rebuild after changing edgechat on your machine:

```bash
ssh thor 'bash -lc "cd ~/Projects/edgechat && cargo install --path ."'
```

## Thor Tigress Cub: chat in the browser

`web/` holds the chat page (`index.html` with the cub art inline, `chat.css` for
the theme, and `chat.js` for the markdown, links and tables; no outside scripts)
and the art (`cub.svg`). `model-serving/thor-tigress-serve`
serves the models and the page. `thor-tigress-serve install`
creates two user services that start at boot: `thor-chat` (llama.cpp's
`llama-server` with Nemotron on `127.0.0.1:8079`) and `thor-tigress-agent`
(the page, the key check, web search and the API on `127.0.0.1:8080`).
`about.html` and the art are served next to the chat. Full guide: the book,
chapter "Web chat: Thor Tigress Cub".

| Setting | Value |
|---|---|
| Models | Nemotron 3 Nano 30B-A3B Q8_0 (llama.cpp, the default); Qwen3.6-35B-A3B NVFP4 (TensorRT Edge-LLM, 78 tok/s); Nemotron 3.5 Lightning on disk, not loaded |
| People at once | Nano 4 (`USERS`); Qwen 1 |
| Context per person | Nano 131,072 tokens (`CONTEXT`); Qwen 32,768 (`EDGE_CONTEXT`) |
| KV cache | Nano 3.0 GB at 4 x 128K, held while loaded. It was 24 GB at 4 x 1M until 2026-10-08 |
| Measured | Nano 53 tok/s, first token 0.2 s; Qwen 78 tok/s |
| Model names | `nemotron` / `nemotron-think`, `Qwen3.6-35B-A3B-NVFP4`, or the full id; others refused |

Change settings in `~/.config/thor-chat/env` (e.g. `USERS=8`), then
`thor-tigress-serve install`. For the context on its own,
`thor-tigress-serve context TOKENS [USERS]` is quicker: it reloads the models
through the router and never restarts a service. The page has Web and Think switches, a stop key (Esc),
highlighted code with copy, twelve themes and an optional system prompt in
Settings (none by default).

```bash
thor-tigress-serve list               # the models on the Thor, each with a key and what it costs
thor-tigress-serve list-latest        # the newest chat models on Hugging Face that fit
thor-tigress-serve load KEY|NAME      # load a model from list
thor-tigress-serve unload KEY|NAME    # unload it
thor-tigress-serve download KEY|REPO  # download a model from list-latest
thor-tigress-serve context            # what the context setting costs in memory
thor-tigress-serve context TOKENS     # change it and reload; no service restart
thor-tigress-serve logs               # follow both logs
thor-tigress-serve key                # new access key; the page asks for it once
thor-tigress-serve uninstall          # stop and remove both services
```

The first time, run it from the repository:
`jetson-thor/model-serving/thor-tigress-serve install`. That also puts
`thor-tigress-serve` in `~/.local/bin`. The memory checks, settings and
models worth trying are in the book, chapter "Model serving".

### Memory, context, and swapping models

A loaded model costs two things: its weights, and a **KV cache that is reserved
for the whole of `CONTEXT x USERS` the moment it loads** — not as chats arrive,
and not released while it stays loaded. That second number is what surprises
people, and at a big context it dwarfs the weights.

Nemotron 3 Nano is unusually cheap in KV: it is a hybrid, and only 6 of its 52
blocks are attention layers (each with 2 KV heads of 128), so it needs 6 KiB per
token. The other 46 blocks are Mamba, whose state does not grow with the
context. Even so, `CONTEXT=1048576` with `USERS=4` reserved 24 GB:

| | Weights | KV cache | To load |
|---|---|---|---|
| 4 x 1,048,576 (the old default) | 33.6 GB | 24.0 GB | ~60 GB |
| 4 x 131,072 (since 2026-10-08) | 33.6 GB | 3.0 GB | ~39 GB |

On 2026-10-08 the Thor was holding 96 GB of its 122.8 GB with just two models
loaded, and 24 GB of that was Nano's KV cache. Cutting the context to 128K freed
33 GB the same day, without restarting a service:

```bash
thor-tigress-serve context                    # what the current setting costs
thor-tigress-serve context 131072             # 128K per reply, keeping USERS=4
thor-tigress-serve context 131072 2           # 128K per reply, 2 replies at once
```

`context` writes `~/.config/thor-chat/env`, regenerates `models.ini`, and
unloads and reloads the models that are loaded through the router. `thor-chat`
itself does not restart, `thor-tigress-agent` does not restart, and the
TensorRT Edge-LLM service is not touched — so the other model keeps answering
while one reloads.

Nothing here is hard-coded per model. `list` reads the GGUF header of every model
on disk (`block_count`, `attention.head_count_kv`, `attention.key_length`, …)
and works out the KV per token, so a new model gets an honest `loads` figure the
first time it appears. `load` also refuses a model that would not fit, counting
its KV cache rather than only its file size — the check that was missing when
Nano was given 4 x 1M on a 122.8 GB machine with another model already resident.

To swap one model for another:

```bash
thor-tigress-serve list          # keys, states, and the cost of each
thor-tigress-serve unload 2      # by key, or by name
thor-tigress-serve load 2
```

### Choosing the engine in the chat page

The picker at the top of the page is filled from the agent's `/v1/models`, which
merges llama-server's list with every TensorRT Edge-LLM engine the agent was
told about. Choosing a row sends that model's name in the request, and the agent
routes by name: a model served by an engine goes to that engine, everything else
to llama-server. Nothing else in the page changes.

As of 2026-10-08 the picker offers three rows, and picking is how you test one
engine against the other on the same prompt:

| Row | Served by | Port |
|---|---|---|
| Nemotron 3 Nano 30B A3B · Q8_0 (llama.cpp) | llama-server | 8079 |
| Qwen3.6 35B A3B NVFP4 (TensorRT) | TensorRT Edge-LLM | 8081 |
| Nemotron 3 Nano 30B A3B NVFP4 (TensorRT) | TensorRT Edge-LLM | 8082 |

The engine is named in brackets only when more than one engine answers, so a
single-engine Thor still reads as before. A row for an engine that is down is
simply left out, so one stopping never hides the others.

Think works on every row. The think budget in Settings does not: it is a
llama-server body field (`reasoning_budget_tokens` and `reasoning_budget_message`),
and TensorRT Edge-LLM refuses a field it does not know with "Extra inputs are not
permitted" and a 400. The page therefore sends those two fields to llama-server
only, so Think on an engine reasoning works but its thinking has no token cap
from that setting; the round is still bounded by the page's own `max_tokens`.
Anything else driving these servers has to make the same split — the engines
accept an OpenAI request plus `chat_template_kwargs`, and nothing else.

An Edge-LLM model has no key to load. It is one line in
`~/.config/thor-chat/env` naming the checkpoint folder and its port, and then
`install`:

```bash
echo 'EDGE_MODELS=Nemotron-3-Nano-30B-A3B-NVFP4=8082' >> ~/.config/thor-chat/env
thor-tigress-serve install
```

The folder must sit beside `EDGE_MODEL`'s, under `~/models/edge-llm`, because
the folder's name is the model id the page shows and the request must name.
`install` writes `thor-edge-llm-<name>.service` next to `thor-edge-llm.service`
and reloads the agent, which is what makes the row appear. The first start
builds the engine (minutes) and later starts reuse it from
`~/models/edge-llm/cache`, where each model gets its own directory keyed by a
hash, so the engines never collide. `EDGE_CONTEXT` caps their context; it is not
part of the KV arithmetic above, because Edge-LLM pages its KV cache far more
tightly than llama.cpp reserves one.

`thor-tigress-serve load`/`unload` start and stop these services too, so
"unload 3" then "load 3" is how to free and reclaim an engine's memory from the
same list. Running `install` restarts every service, so use it when adding a
model rather than for a routine change.

### Access key

With a key set, the page loads for anyone but the chat API refuses requests
without the key. It is on now, because the chat is public through Funnel.

```bash
thor-tigress-serve key && systemctl --user restart thor-chat thor-tigress-agent   # new key; the old one stops working
cat ~/.config/thor-chat/api-key                        # show the current key
rm ~/.config/thor-chat/api-key && systemctl --user restart thor-chat thor-tigress-agent   # no key: open to anyone
```

The key is 24 random bytes from `/dev/urandom`, kept in
`~/.config/thor-chat/api-key` (readable only by you). People paste it once on
the page's invite screen; their browser remembers it (Settings can change it).

### Reach it over Tailscale

The server only listens on the Thor itself. Tailscale gives it an HTTPS
address. On the Thor (needs your sudo password and a Tailscale login):

```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up --ssh                # open the printed URL and log in
sudo tailscale serve --bg 8080         # https://<thor-name>.<tailnet>.ts.net
tailscale serve status                 # shows the address
```

If `serve` asks to enable HTTPS certificates, follow the link it prints
(Tailscale admin console, DNS page).

Who can open it:

- Invited people (private): in the Tailscale admin console, Machines → the
  Thor → Share, and invite them by email. They install Tailscale, accept, and
  open the address. Nothing is on the open internet.
- Anyone with the link (public): `sudo tailscale funnel --bg 8080` instead of
  `serve`, and run `thor-tigress-serve key` first so only people you give the key to
  can chat. Turn it off with `sudo tailscale funnel --bg 8080 off`.

### voltforge.tech/thor-tigress-cub

`https://voltforge.tech/thor-tigress-cub` is a one-file forwarding page on
GitHub Pages (repository `arpanpathak/voltforge.tech`) that sends the browser
to `https://arpanpathak.taildb9a39.ts.net/`. HTTPS on both hops, no proxy, no
port forward, no copy of the UI. Namecheap's own URL redirect was tried first
and dropped: it only works over plain HTTP. Reasoning, every IP address and
the steps: the book, chapter "Bring your own domain".

## Platform tools on the Thor

`spark`, `thor-hammer-trainer`, `reinforcer` and `lasso` are installed in
`~/.cargo/bin`. `data/` is synced (see `~/.config/thor-sync/include`).
Reach the review page from your machine with
`ssh -L 8787:localhost:8787 thor 'bash -lc "cd ~/Projects/thor-thunder-tigress-platform && reinforcer data/train.jsonl 8787"'`
and open http://localhost:8787.

`thor-hammer-trainer` needs its inputs on the Thor first:
`~/Projects/edgechat/convo_datastore`, `~/Projects/nvidia-cloud-software-engineer-interview`
and `corpus/` (`bash train/fetch_corpus.sh`).

## Coding agent: OpenCode (removed)

OpenCode was tried on the Jetson Orin NX dev machine on 2026-10-05 and removed the same day; its tool
calls with Nemotron often came back as text. The config is kept in the book,
chapter "Local models". Use openbatrangs (`openbatrangs --thor`) instead.

## Ollama

```bash
ollama list                          # downloaded models
ollama ps                            # loaded models; must say 100% GPU
ollama run qwen3.6:27b --verbose     # plain chat; "eval rate" is tokens/s
ollama stop qwen3.6:27b              # unload to free memory
```

## Measured

| Date | What | Result |
|---|---|---|
| 2026-10-04 | openBatarangs build on the Thor (release, 14 cores) | 19.5 s |
| 2026-10-04 | openBatarangs, qwen3.6:27b, read-only "list the crates" task | 6 steps, 21 s, correct |
| 2026-10-05 | openBatarangs, qwen3.6:27b, 262,144 context, "create, compile and run hello.rs" | 3 steps, 9 s, correct |
| 2026-10-05 | openBatarangs `--thor` (Nemotron, thinking on), 5-module DSA crate with tests | 14 steps, 293 s, 14 tests pass |
| 2026-10-05 | codebuddy, Nemotron 3 Nano 30B-A3B Q8_0 (llama.cpp), 1,046,528 context | 51.8 tok/s, first token 0.5 s |
| 2026-10-05 | web chat (llama-server), Nemotron Q8_0, 4 × 1,048,576 context, thinking off | 53 tok/s, first token 0.2 s |
| 2026-10-08 | Nemotron 3 Nano 30B-A3B, llama.cpp Q8_0 vs TensorRT Edge-LLM NVFP4, same 20 Rust tasks (`compare.py`), thinking off | 53.3 vs 58.9 tok/s; 15/20 vs 16/20 compile; 14/20 vs 16/20 tests; 35.4 GB vs 21.0 GB of GPU |
| 2026-10-08 | Thor memory with two models loaded, before the context cut | 96 GB used, and 24 GB of it was Nano's 4 x 1M KV cache. `context 131072` freed 33 GB with no service restart |
