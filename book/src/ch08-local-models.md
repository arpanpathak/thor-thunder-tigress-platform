<img class="plate" src="art/ch05.svg" alt="A tigress beside a GPU module marked sm_110 with a lightning bolt">

# Local models

## edgechat (local-copilot-codebuddy)

Terminal chat that runs the model inside the program with llama.cpp. On the
Thor:

```bash
edgechat            # pick a model from ~/models
```

| Model | File | Context | Speed (measured) |
|---|---|---|---|
| Nemotron 3 Nano 30B-A3B, Q8_0 | `~/models/gguf/Nemotron-3-Nano-30B-A3B/` | 1M tokens | 52 tok/s |
| Qwen3.6-27B, Q4_K_M | linked from Ollama's copy | 256K tokens | 12 tok/s |

- It uses each model's full context unless `--kv-cache-tokens` caps it.
- It sends no built-in prompt. Your rules file
  `~/.config/local-copilot-codebuddy/rules.md` goes into every chat as written.
- To add a model, put a ChatML `.gguf` file under `~/models/gguf/`.

## Coding agent: OpenCode

[OpenCode](https://opencode.ai) is a terminal coding agent: it reads and edits
files and runs commands on the machine you use, and calls Nemotron on the Thor
for the model. Your code stays on your machine; only model calls reach the
Thor.

```text
your machine: opencode ─► 127.0.0.1:8079 ─► SSH tunnel ─► thor: llama-server :8079 (Nemotron)
                     └──► 127.0.0.1:8888 ─► SSH tunnel ─► thor: SearXNG :8888 (search)
```

The tunnel uses your existing SSH access, so nothing new is opened on the
network.

OpenCode was set up on yahboom on 2026-10-05 and removed the same day. In the
sessions there, Nemotron's tool calls often came back as plain text instead of
calls, so files it reported as written did not exist, and with a project-level
`CLAUDE.md` in the folder it refused multi-file work. openbatrangs (below)
replaced it. The setup is kept here for anyone who wants to try it.

### Setup on Linux

```bash
curl -fsSL https://opencode.ai/install | bash        # installs to ~/.opencode/bin
mkdir -p ~/.config/thor-chat
(umask 077; ssh thor cat .config/thor-chat/api-key > ~/.config/thor-chat/api-key)
```

`~/.config/opencode/opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "model": "thor/nemotron",
  "provider": {
    "thor": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "Thor",
      "options": {
        "baseURL": "http://127.0.0.1:8079/v1",
        "apiKey": "{file:~/.config/thor-chat/api-key}"
      },
      "models": {
        "nemotron": {
          "name": "Nemotron 3 Nano (Thor)",
          "options": { "chat_template_kwargs": { "enable_thinking": false } }
        },
        "nemotron-think": { "name": "Nemotron 3 Nano (Thor, thinking)" }
      }
    }
  },
  "agent": { "plan": { "model": "thor/nemotron-think" } },
  "mcp": {
    "searxng": {
      "type": "local",
      "command": ["npx", "-y", "mcp-searxng@2.5.0"],
      "environment": { "SEARXNG_URL": "http://127.0.0.1:8888" },
      "enabled": true
    }
  }
}
```

- **Build mode** uses `nemotron` with thinking off, so it acts.
- **Plan mode** (Tab) uses `nemotron-think`, so it reasons before proposing.
- **Web search** comes from the SearXNG instance on the Thor through the
  `mcp-searxng` plugin (an npm package, run locally by `npx`): tools
  `searxng_web_search` and `web_url_read`, which reads a page as text.

`enable_thinking: false` matters: Nemotron thinks by default, and on a large
task it spent a whole turn reasoning (11,504 characters) and stopped without
calling a single tool. With thinking off it goes straight to reading, writing
and running. The web chat's think toggle is not affected.

The tunnel as a user service that starts at boot and reconnects,
`~/.config/systemd/user/thor-model-tunnel.service`:

```ini
[Unit]
Description=SSH tunnel to the Thor: Nemotron (8079), SearXNG (8888), front server (8080)
After=network-online.target

[Service]
ExecStart=/usr/bin/ssh -o ControlMaster=no -o ControlPath=none -o BatchMode=yes -o ServerAliveInterval=30 -o ExitOnForwardFailure=yes -N -L 127.0.0.1:8079:127.0.0.1:8079 -L 127.0.0.1:8888:127.0.0.1:8888 -L 127.0.0.1:8080:127.0.0.1:8080 thor
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
```

```bash
systemctl --user enable --now thor-model-tunnel
```

### Setup on macOS

```bash
curl -fsSL https://opencode.ai/install | bash
mkdir -p ~/.config/opencode ~/.config/thor-chat
scp <yahboom>:.config/opencode/opencode.json ~/.config/opencode/
(umask 077; ssh thor cat .config/thor-chat/api-key > ~/.config/thor-chat/api-key)
echo 'alias opencode="(nc -z 127.0.0.1 8079 || ssh -fN -L 8079:127.0.0.1:8079 -L 8888:127.0.0.1:8888 thor) && command opencode"' >> ~/.zshrc
```

The alias opens the tunnel when it is not already up.

### Use

```bash
cd ~/Projects/some-project
opencode                           # interactive
opencode run "add a unit test"     # one task, then exit
```

Checks: `systemctl --user status thor-model-tunnel` and
`curl -s 127.0.0.1:8079/health` (must print `{"status":"ok"}`).

Each running session uses one of the Thor's chat slots while it generates.
After a new access key (`./serve.sh key` on the Thor), copy it again with the
`ssh thor cat …` line above.

## Ollama

```bash
ollama run qwen3.6:27b --verbose    # "eval rate" is tokens/s
ollama ps                           # must show 100% GPU
ollama stop qwen3.6:27b             # free its memory
```

## Coding agent: openbatrangs with Nemotron

[openbatrangs](https://github.com/arpanpathak/openbatrangs) is a terminal
coding agent: it writes files and runs commands in the current folder, one
action per step, until the task is done. `--thor` points it at Nemotron's
llama-server instead of Ollama.

```bash
ssh thor
mkdir -p ~/some-project && cd ~/some-project
openbatrangs --thor                  # interactive; /models lists the served model
openbatrangs --thor "create a cargo project named dsa with a stack and a queue, with tests, then run cargo test"
```

| Option | Effect |
|---|---|
| `--thor` | llama-server at `127.0.0.1:8079/v1`, key from `~/.config/thor-chat/api-key` |
| `--openai-url URL --api-key-file PATH` | any other OpenAI-compatible server |
| `--no-think` | thinking off (on by default; reasoning is shown dimmed after 💭) |
| `--max-steps N` | step limit, default 40 |

The same command works on yahboom through `thor-model-tunnel` (above). Install
on either machine: `cargo install --path ~/Projects/openbatrangs --locked`.

Measured on 2026-10-05, from yahboom, thinking on, task: "a cargo project with
a stack, a queue, a singly linked list, binary search and quicksort, each with
unit tests, then run cargo test and fix anything that fails":

| Steps | Time | Result |
|---|---|---|
| 14 of 40 | 293 s | 5 modules; fixed two rounds of compile errors itself; 14 tests pass |

Each step is 10–20 s of generation at about 53 tok/s. A run takes one of the
four chat slots, so it competes with web chat users while it works.

Two bugs that made it look like it was overthinking:

- Commands ran with a relative `TMPDIR`, so `cd dsa && cargo test` failed in
  the doc-test step every time. The model saw a failing test and kept
  rewriting working code until it ran out of steps. Sandbox paths are now
  absolute.
- Through the SSH tunnel, a reused idle connection failed between steps
  ("connection closed before message completed"). The client no longer keeps
  idle connections to this server.

## Memory

<figure>
<img src="figures/thor-memory.svg" alt="128 GB of unified memory shared by the CPU and GPU, about 115 GB usable for training, with reported peak memory for QLoRA 27B, full fine-tune 4B and LoRA 9B, and 273 GB/s bandwidth">
<figcaption><b>Figure 5.1</b> The Thor's memory and what training methods need.</figcaption>
</figure>

The CPU and GPU share 128 GB. The web chat (chapter 6) keeps Nemotron loaded:
about 58 GB at 4 people × 1M context. Stop it before loading a second large
model or training: `systemctl --user stop thor-chat`.
