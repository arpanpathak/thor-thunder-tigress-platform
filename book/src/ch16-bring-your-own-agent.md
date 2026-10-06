# Bring your own agent

Nemotron on the Thor is an API anyone with a key can call. Point the coding
agent you already use at it: Claude Code, openbatrangs, OpenCode, or anything
that speaks the OpenAI or Anthropic API.

| | |
|---|---|
| Address | `https://arpanpathak.taildb9a39.ts.net` |
| Model | Nemotron 3 Nano 30B-A3B, Q8_0, 1M-token context |
| Speed | about 53 tokens/s per reply; thinks before answering by default |
| OpenAI API | `/v1/models`, `/v1/chat/completions` |
| Anthropic API | `/v1/messages` (Claude Code) |
| Key | `Authorization: Bearer <key>` or `x-api-key: <key>` |

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

Claude Code speaks the Anthropic API, which llama-server also serves. Point
it at the Thor with environment variables:

```bash
export ANTHROPIC_BASE_URL=https://arpanpathak.taildb9a39.ts.net
export ANTHROPIC_AUTH_TOKEN=$(cat ~/.config/thor-chat/api-key)
export ANTHROPIC_MODEL=nemotron
export ANTHROPIC_DEFAULT_OPUS_MODEL=nemotron
export ANTHROPIC_DEFAULT_SONNET_MODEL=nemotron
export ANTHROPIC_DEFAULT_HAIKU_MODEL=nemotron
export CLAUDE_CODE_MAX_CONTEXT_TOKENS=1000000
claude
```

- The server ignores the model name; `nemotron` is a label.
- `CLAUDE_CODE_MAX_CONTEXT_TOKENS` tells Claude Code the real window.
  Without it, Claude Code assumes 200K for an unknown model.
- Unset the variables, or open a new terminal, to go back to your normal
  Claude account.

Measured on 2026-10-05 (Claude Code 2.1.290, from yahboom): "create hello.rs
that prints hello, compile it with rustc and run it" took 29 s, and the
program printed `hello`.

## openbatrangs

[openbatrangs](https://github.com/arpanpathak/openbatrangs) is a terminal
coding agent that edits files and runs commands in the current folder. It
needs Rust (`curl https://sh.rustup.rs -sSf | sh`).

Linux and macOS:

```bash
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

```text
your agent ─► https://…ts.net ─► Tailscale Funnel ─► thor-tigress-agent :8080 ─► llama-server :8079
                                                     (checks the key)
```

`thor-tigress-agent` passes `/v1/chat/completions` and `/v1/models` to
llama-server, and `/v1/messages` unchanged, so Claude Code's requests reach
llama-server's Anthropic API. It accepts the key as `Authorization: Bearer`
or `x-api-key`. Everything else about exposure is in chapter "Security".
