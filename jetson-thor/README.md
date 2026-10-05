# Jetson Thor

The book has the full guide: https://arpanpathak.github.io/thor-thunder-tigress-platform/ (chapters 4 to 7).

The Thor is `192.168.0.189`, user `arpanpathak`. On yahboom it is just `thor`.

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

`thor-sync` (in `~/.local/bin` on yahboom) copies folders to the same place
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
chat export (edit `~/.config/thor-sync/exclude`). Sync goes one way, yahboom
to Thor: commit on yahboom, because a commit made on the Thor is overwritten.

## openBatarangs on the Thor

```bash
ssh thor
cd ~/Projects/some-project
openbatrangs                                  # interactive TUI
openbatrangs -m qwen3.6:27b --max-ctx 32768 "fix the failing test"
openbatrangs --read-only "explain this repo"  # no file writes or commands
openbatrangs doctor                           # check Ollama and the model
```

After changing openBatarangs on yahboom, rebuild it on the Thor:

```bash
ssh thor 'cd ~/Projects/openbatrangs && ~/.cargo/bin/cargo install --path .'
```

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

To rebuild after changing edgechat on yahboom:

```bash
ssh thor 'bash -lc "cd ~/Projects/edgechat && cargo install --path ."'
```

## Nemotron chat in the browser

`web/` holds the chat page (`index.html`, one file, no outside scripts) and
`serve.sh`, which runs llama.cpp's `llama-server` with Nemotron on
`127.0.0.1:8080`, serving the page and an OpenAI-compatible API. It is
installed as the `thor-chat` user service and starts at boot.

| Setting | Value |
|---|---|
| Model | Nemotron 3 Nano 30B-A3B Q8_0 |
| People at once | 4 (`USERS`) |
| Context per person | 1,048,576 tokens (`CONTEXT`) |
| Measured | 53 tok/s, first token 0.2 s |

Change settings in `~/.config/thor-chat/env` (e.g. `USERS=8`), then
`./serve.sh install`. The page has a think toggle, a stop
button, copyable highlighted code and an optional system prompt in settings
(none by default).

```bash
./serve.sh logs          # follow the server log
./serve.sh key           # require an access key; the page asks for it once
./serve.sh uninstall     # stop and remove the service
```

### Access key

With a key set, the page loads for anyone but the chat API refuses requests
without the key. It is on now, because the chat is public through Funnel.

```bash
cd ~/Projects/thor-thunder-tigress-platform/jetson-thor/web
./serve.sh key && systemctl --user restart thor-chat thor-tigress-agent   # new key; the old one stops working
cat ~/.config/thor-chat/api-key                        # show the current key
rm ~/.config/thor-chat/api-key && systemctl --user restart thor-chat thor-tigress-agent   # no key: open to anyone
```

The key is 24 random bytes from `/dev/urandom`, kept in
`~/.config/thor-chat/api-key` (readable only by you). People enter it once in
the page: ⚙ → Access key → Save. Their browser remembers it.

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
  `serve`, and run `./serve.sh key` first so only people you give the key to
  can chat. Turn it off with `sudo tailscale funnel --bg 8080 off`.

### chat.voltforge.tech

Tailscale addresses end in `.ts.net` and cannot be renamed. To share
`chat.voltforge.tech`: Namecheap → Domain List → voltforge.tech → Advanced
DNS → Add New Record → URL Redirect Record, host `chat`, value
`https://<thor-name>.<tailnet>.ts.net`, Permanent (301). The browser then
shows the `.ts.net` address. Keeping `voltforge.tech` in the address bar
needs a reverse proxy such as Cloudflare Tunnel instead.

## Platform tools on the Thor

`spark`, `thor-hammer-trainer`, `reinforcer` and `lasso` are installed in
`~/.cargo/bin`. `data/` is synced (see `~/.config/thor-sync/include`).
Reach the review page from yahboom with
`ssh -L 8787:localhost:8787 thor 'bash -lc "cd ~/Projects/thor-thunder-tigress-platform && reinforcer data/train.jsonl 8787"'`
and open http://localhost:8787.

`thor-hammer-trainer` needs its inputs on the Thor first:
`~/Projects/edgechat/convo_datastore`, `~/Projects/nvidia-cloud-software-engineer-interview`
and `corpus/` (`bash train/fetch_corpus.sh`).

## Coding agent: OpenCode with Nemotron

On yahboom, `opencode` in any project folder starts a terminal coding agent
that uses Nemotron on the Thor. Set up: OpenCode in `~/.opencode/bin`, the
config `~/.config/opencode/opencode.json` (provider `thor`, default model
`thor/nemotron`, thinking off for agent work, base URL `http://127.0.0.1:8079/v1`, key read from
`~/.config/thor-chat/api-key`), and the `thor-model-tunnel` user service that
keeps `127.0.0.1:8079` (llama-server) and `127.0.0.1:8888` (SearXNG) tunnelled to the Thor. Plan mode uses Nemotron with thinking on; web search and page reading come from the `mcp-searxng` plugin.

```bash
cd ~/Projects/some-project && opencode
systemctl --user status thor-model-tunnel      # tunnel up?
curl -s 127.0.0.1:8079/health                  # model reachable?
```

Full setup, including macOS: the book, chapter "Local models".

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
| 2026-10-05 | codebuddy, Nemotron 3 Nano 30B-A3B Q8_0 (llama.cpp), 1,046,528 context | 51.8 tok/s, first token 0.5 s |
| 2026-10-05 | web chat (llama-server), Nemotron Q8_0, 4 × 1,048,576 context, thinking off | 53 tok/s, first token 0.2 s |
