# Web chat

Nemotron in the browser, shared through Tailscale.

```text
browser ─► https://<thor>.<tailnet>.ts.net ─► Tailscale ─► thor-tigress-agent :8080
                                                              ├─► llama-server :8079 (Nemotron)
                                                              └─► SearXNG :8888 (web search)
```

All three listen on the Thor only. `jetson-thor/web/` holds the page
(`index.html`, one file) and `serve.sh`. `thor-tigress-agent` (a crate in this
repository) serves the page, checks the access key and, when **web** is on,
lets the model search through SearXNG; the page lists the sources it used.

## Start it

On the Thor:

```bash
cd ~/Projects/thor-thunder-tigress-platform
cargo install --path crates/thor-tigress-agent
cd jetson-thor/web
./serve.sh install       # thor-chat and thor-tigress-agent services, start at boot
./serve.sh logs          # follow both logs
```

Settings go in `~/.config/thor-chat/env`, then `./serve.sh install` again:

| Setting | Default |
|---|---|
| `USERS` (people at once) | 4 |
| `CONTEXT` (tokens per person) | 1,048,576 |
| `MODEL` | Nemotron 3 Nano Q8_0 |

SearXNG runs as the `searxng` user service from `~/.local/src/searxng`
(settings in `~/.config/searxng/settings.yml`, JSON output on).

The page has themes (including Material Deep Ocean), a **think** toggle and a
**web** toggle. Search gives the model result snippets only; it does not yet
read the pages, so check its sources.

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
./serve.sh key && systemctl --user restart thor-chat thor-tigress-agent    # new key
cat ~/.config/thor-chat/api-key                         # show it
```

People enter it once in the page: ⚙ → Access key → Save.

## Own domain

Tailscale addresses end in `.ts.net`. To show your own domain (e.g.
`chat.voltforge.tech`) in the address bar, use Cloudflare Tunnel instead. A
Namecheap URL redirect only forwards visitors to the `.ts.net` address.
