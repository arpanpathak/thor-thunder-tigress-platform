# Web chat

Nemotron in the browser, shared through Tailscale.

```text
browser ─► https://<thor>.<tailnet>.ts.net ─► Tailscale ─► llama-server :8080 (Thor only)
```

`jetson-thor/web/` holds the page (`index.html`, one file) and `serve.sh`,
which runs llama.cpp's `llama-server` with Nemotron and serves the page.

## Start it

On the Thor:

```bash
cd ~/Projects/thor-thunder-tigress-platform/jetson-thor/web
./serve.sh install       # runs as a service, starts at boot
./serve.sh logs          # follow the log
```

Settings go in `~/.config/thor-chat/env`, then `systemctl --user restart thor-chat`:

| Setting | Default |
|---|---|
| `USERS` (people at once) | 4 |
| `CONTEXT` (tokens per person) | 1,048,576 |
| `MODEL` | Nemotron 3 Nano Q8_0 |

## Share it

Install Tailscale on the Thor once:

```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up --ssh
```

Then pick one:

| | Command | Who can open it |
|---|---|---|
| Private | `sudo tailscale serve --bg 8080` | devices on your tailnet; share the Thor from the Tailscale admin console |
| Public | `./serve.sh key`, then `sudo tailscale funnel --bg 8080` | anyone with the link and the access key |

A new public address can take a few minutes to resolve everywhere.

## Access key

```bash
./serve.sh key && systemctl --user restart thor-chat    # new key
cat ~/.config/thor-chat/api-key                         # show it
```

People enter it once in the page: ⚙ → Access key → Save.

## Own domain

Tailscale addresses end in `.ts.net`. To show your own domain (e.g.
`chat.voltforge.tech`) in the address bar, use Cloudflare Tunnel instead. A
Namecheap URL redirect only forwards visitors to the `.ts.net` address.
