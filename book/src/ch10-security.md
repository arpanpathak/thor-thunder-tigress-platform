# Security

What the public chat exposes, tested on the Thor:

| Reachable through the tunnel | Without the key |
|---|---|
| the page (`/`) and `/health` | open, nothing sensitive |
| chat, completion, Anthropic messages, tokenize, embedding, slots, props, metrics | refused (401) |
| anything else on the Thor (SSH, files, other ports) | not exposed |

## Keep it that way

- **Don't share the key in chats or repositories.** If it leaks, make a new
  one: `./serve.sh key && systemctl --user restart thor-chat thor-tigress-agent`.
- **Update llama.cpp now and then.** The remaining risk is a bug in
  `llama-server` itself, which runs as your user.
- **Prefer private (`tailscale serve`) for people you know.** Public
  (`funnel`) lets anyone with the link try.
- **Anyone with the key can keep the GPU busy;** there are no per-person limits.
