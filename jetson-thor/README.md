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

`web/` holds the chat page (`index.html`, one file with the cub art inline, no
outside scripts) and the art (`cub.svg`). `model-serving/thor-tigress-serve`
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
| Context per person | Nano 1,048,576 tokens (`CONTEXT`); Qwen 32,768 (`EDGE_CONTEXT`) |
| Measured | Nano 53 tok/s, first token 0.2 s; Qwen 78 tok/s |
| Model names | `nemotron` / `nemotron-think`, `Qwen3.6-35B-A3B-NVFP4`, or the full id; others refused |

Change settings in `~/.config/thor-chat/env` (e.g. `USERS=8`), then
`thor-tigress-serve install`. The page has Web and Think switches, a stop key (Esc),
highlighted code with copy, twelve themes and an optional system prompt in
Settings (none by default).

```bash
thor-tigress-serve list               # the models on the Thor, each with a key
thor-tigress-serve list-latest        # the newest chat models on Hugging Face that fit
thor-tigress-serve load KEY|NAME      # load a model from list
thor-tigress-serve unload KEY|NAME    # unload it
thor-tigress-serve download KEY|REPO  # download a model from list-latest
thor-tigress-serve logs               # follow both logs
thor-tigress-serve key                # new access key; the page asks for it once
thor-tigress-serve uninstall          # stop and remove both services
```

The first time, run it from the repository:
`jetson-thor/model-serving/thor-tigress-serve install`. That also puts
`thor-tigress-serve` in `~/.local/bin`. The memory checks, settings and
models worth trying are in the book, chapter "Model serving".

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
