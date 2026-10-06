<img class="cub" src="art/cub.svg" alt="The Thor Tigress Cub">

# Bring your own agent

Nemotron on the Thor is an API anyone with a key can call. Point the coding
agent you already use at it: Claude Code, openbatrangs, OpenCode, or anything
that speaks the OpenAI or Anthropic API.

| | |
|---|---|
| Address | `https://arpanpathak.taildb9a39.ts.net` |
| Model | Nemotron 3 Nano 30B-A3B, Q8_0, 1M-token context |
| Speed | about 53 tokens/s per reply; thinks before answering unless told not to (below) |
| OpenAI API | `/v1/models`, `/v1/chat/completions` |
| Anthropic API | `/v1/messages` (Claude Code) |
| Key | `Authorization: Bearer <key>` or `x-api-key: <key>` |

## Thinking on or off, per tool

Nemotron can reason before it answers. Reasoning is generated at the same
~53 tokens/s as the answer, so it costs seconds per step: helpful on hard
steps, wasted on easy ones. Each tool switches it differently:

| Tool | Thinking off | Thinking on | Default |
|---|---|---|---|
| Web chat | **Think** switch off | **Think** switch on | off |
| openbatrangs | `--no-think` | (nothing) | on |
| Claude Code | `claude-thor` (sets `MAX_THINKING_TOKENS=0`) | `claude-thor-think` | Claude Code asks for it |
| OpenCode | the `nemotron` model | the `nemotron-think` model; plan mode uses it | as configured |
| OpenAI SDKs, curl | `"chat_template_kwargs": {"enable_thinking": false}` | leave it out | on |
| Anthropic API (`/v1/messages`) | leave `thinking` out, or `"type": "disabled"` | `"thinking": {"type": "enabled"}` or `"adaptive"` | off |

llama-server ignores the Anthropic `thinking` field, so `thor-tigress-agent`
translates it: a `/v1/messages` request that doesn't ask for thinking gets
`enable_thinking: false`. Through the OpenAI path, Nemotron's own default
(thinking on) applies unless the request says otherwise.

## Get a key

Keys are given by the author for a limited time. Ask
[arpanpathak on GitHub](https://github.com/arpanpathak) and say what you want
to use it for. A key can be changed or withdrawn at any time, after which
requests get `401`.

Save it where the tools below look for it:

```bash
mkdir -p ~/.config/thor-chat
(umask 077; printf '%s\n' 'PASTE-THE-KEY' > ~/.config/thor-chat/api-key)
```

Before you start:

- The Thor serves four replies at a time, shared by everyone, including the
  web chat. When all four are busy, your request waits.
- Prompts and code you send are processed on the author's machine. Don't send
  secrets.
- There is no uptime promise. It is a developer kit on a home network.

## Check it works

```bash
K=$(cat ~/.config/thor-chat/api-key)
curl -s https://arpanpathak.taildb9a39.ts.net/v1/models -H "Authorization: Bearer $K"
```

It lists one model. `401` means the key is wrong or no longer valid.

## Claude Code

Claude Code speaks the Anthropic API, which llama-server also serves. Two
commands switch it to the Thor only while they run; plain `claude` keeps using
your Claude account.

macOS (`~/.zshrc`), or any machine that reaches the Thor through its public
address:

```bash
THOR_CC='CLAUDE_CONFIG_DIR=$HOME/.claude-thor ANTHROPIC_BASE_URL=https://arpanpathak.taildb9a39.ts.net ANTHROPIC_AUTH_TOKEN=$(cat ~/.config/thor-chat/api-key) ANTHROPIC_MODEL=nemotron ANTHROPIC_DEFAULT_OPUS_MODEL=nemotron ANTHROPIC_DEFAULT_SONNET_MODEL=nemotron ANTHROPIC_DEFAULT_HAIKU_MODEL=nemotron CLAUDE_CODE_MAX_CONTEXT_TOKENS=1000000'
echo "alias claude-thor='MAX_THINKING_TOKENS=0 $THOR_CC claude'" >> ~/.zshrc
echo "alias claude-thor-think='$THOR_CC claude'" >> ~/.zshrc
source ~/.zshrc
```

On yahboom (`~/.bashrc`, installed there) the same aliases use
`ANTHROPIC_BASE_URL=http://127.0.0.1:8080`: `thor-model-tunnel` forwards
port 8080 to `thor-tigress-agent` on the Thor. It must be 8080, not 8079:
only `thor-tigress-agent` translates the thinking setting.

| Variable | Why |
|---|---|
| `CLAUDE_CONFIG_DIR=$HOME/.claude-thor` | its own settings, history and memory; without it both commands share `~/.claude`, so a setting changed in one applies to the other and `/resume` lists both |
| `ANTHROPIC_AUTH_TOKEN` | the key, sent as `Authorization: Bearer` |
| `ANTHROPIC_*_MODEL=nemotron` | every model slot goes to the Thor; the server ignores the name |
| `CLAUDE_CODE_MAX_CONTEXT_TOKENS` | the real window; Claude Code assumes 200K for a model it doesn't know |
| `MAX_THINKING_TOKENS=0` | Claude Code then sends no `thinking` field, so the Thor turns thinking off |

The first run in `~/.claude-thor` shows the welcome screen and asks about
folder trust again; it doesn't ask you to log in. Two warnings are expected:
claude.ai connectors are off, and `nemotron` is not in Claude Code's model
list.

Measured on 2026-10-05 (Claude Code 2.1.290, from yahboom through the public
address, same prompt: "create a cargo project named dsa with a stack module
with unit tests, then run cargo test"):

| | Time | Tokens generated | Result |
|---|---|---|---|
| `claude-thor-think` | 207 s | about 11,000 | created the crate and the stack module; ran the tests |
| `claude-thor` | 75 s | about 900 | ignored the task and wrote a CLAUDE.md template instead |

A smaller task ("create hello.rs, compile it with rustc and run it") worked
in 29 s with thinking on. The honest summary: Claude Code's instructions are
long and written for Claude; Nemotron needs its reasoning to follow them, and
is then slow. For agent work on the Thor, openbatrangs (below) is the better
fit: its prompt is short, and a larger version of the same task (five
modules with tests) took 14 steps and 293 s, with all 14 tests passing.

To check requests reach the Thor, watch the server while it works:
`ssh thor 'journalctl --user -fu thor-chat'`.

## openbatrangs

[openbatrangs](https://github.com/arpanpathak/openbatrangs) is a terminal
coding agent that edits files and runs commands in the current folder. It
needs Rust (`curl https://sh.rustup.rs -sSf | sh`).

Linux and macOS (Intel or Apple silicon):

```bash
xcode-select --install       # macOS only: C compiler for the TLS library
cargo install --git https://github.com/arpanpathak/openbatrangs --locked
cd ~/some-project
openbatrangs --openai-url https://arpanpathak.taildb9a39.ts.net/v1 \
             --api-key-file ~/.config/thor-chat/api-key
```

To avoid typing that each time:

```bash
echo 'alias obr="openbatrangs --openai-url https://arpanpathak.taildb9a39.ts.net/v1 --api-key-file ~/.config/thor-chat/api-key"' >> ~/.zshrc   # ~/.bashrc on Linux
```

`--no-think` turns thinking off for faster steps; `--max-steps` raises the
limit of 40. The GPU panel is empty on a Mac, since it reads `tegrastats` or
`nvidia-smi`. The macOS build is checked on Linux, except for the TLS
library's C code, which needs Apple's compiler; it has not yet been run on a
Mac. On the Thor itself and on yahboom, `openbatrangs --thor` is enough
(chapter "Local models").

## OpenCode and other OpenAI clients

Anything with an OpenAI-compatible setting takes the base URL
`https://arpanpathak.taildb9a39.ts.net/v1` and the key. For OpenCode, use
the config in chapter "Local models" with that `baseURL`. Keep
`"chat_template_kwargs": { "enable_thinking": false }` on the build model:
with thinking on, OpenCode got long reasoning and no tool calls.

Python:

```python
from pathlib import Path
from openai import OpenAI

client = OpenAI(
    base_url="https://arpanpathak.taildb9a39.ts.net/v1",
    api_key=Path("~/.config/thor-chat/api-key").expanduser().read_text().strip(),
)
reply = client.chat.completions.create(
    model="nemotron",
    messages=[{"role": "user", "content": "Write a Rust function that reverses a string."}],
    extra_body={"chat_template_kwargs": {"enable_thinking": False}},
)
print(reply.choices[0].message.content)
```

Leave out `extra_body` to let it think first.

## How it is wired

<figure>
<img src="figures/agent-wiring.svg" alt="Agents call the .ts.net address through Tailscale Funnel with the key in a header; thor-tigress-agent checks it and passes requests to llama-server. yahboom reaches the same server through an SSH tunnel.">
<figcaption><b>Figure 9.1</b> How agents reach the Thor.</figcaption>
</figure>

| Path | What `thor-tigress-agent` does |
|---|---|
| `/v1/models` | passes it to llama-server |
| `/v1/chat/completions` | streams it through; answers plain JSON when the request doesn't ask for streaming (the OpenAI SDKs' default) |
| `/v1/messages` | passes it to llama-server's Anthropic API, turning thinking off unless the request asks for it |
| `/v1/messages/count_tokens` | passes it through |

It accepts the key as `Authorization: Bearer` or `x-api-key`, and allows
calls from other sites (CORS), so a page hosted elsewhere can use the API with
a key. Everything else about exposure is in chapter "Security".
