<img class="cub" src="art/cub.svg" alt="The Thor Tigress Cub">

# Bring your own domain

The Thor Tigress Cub runs on the Thor and is reachable at its Tailscale
address, `https://arpanpathak.taildb9a39.ts.net`. That address is long and
says nothing. This chapter makes **`voltforge.tech/thor-tigress-cub`**, a
domain bought from Namecheap, open the same chat, with a redirect at Namecheap
and nothing else: no proxy in between, no port opened at home, no copy of the
page anywhere else.

<div class="covers">

This chapter covers

- what happens between typing an address and seeing the chat
- the four ways to put a domain in front of the Thor, and why a redirect was chosen
- every IP address involved, where it comes from, and which ones must not be used
- the exact Namecheap steps, how to check each, and how to undo them

</div>

## From an address to the chat

Three things happen when someone types `voltforge.tech/thor-tigress-cub`:

1. **Lookup.** The browser asks DNS for the IP address of `voltforge.tech`.
   The answer comes from the domain's *nameservers*, the servers the registrar
   lists as responsible for it. Their *records* map names to addresses: an
   `A` record gives an IPv4 address, an `AAAA` record an IPv6 one.
2. **Connection.** The browser connects to that IP address and, for HTTPS,
   checks that the server holds a certificate for `voltforge.tech`.
3. **Answer.** The server returns a page, or a redirect: "this is now at
   another address", which the browser follows.

The Thor's chat is published by Tailscale Funnel. Funnel only answers for the
name it was given, `arpanpathak.taildb9a39.ts.net`: it holds a certificate for
that name and routes traffic by it. Pointing `voltforge.tech`'s records at
Funnel's addresses would reach Funnel, which would not know the name and
would refuse it. So a domain can't be attached to the Thor directly. Something
has to answer for `voltforge.tech` first.

## Four ways, and the choice

| Way | Who answers for `voltforge.tech` | Address bar | Home IP | Cost | What can go wrong |
|---|---|---|---|---|---|
| **Redirect at Namecheap** (chosen) | Namecheap's redirect service, which only says "go to the `.ts.net` address" | changes to `.ts.net` | hidden | none | Namecheap's redirect service is down: the short address stops working; the `.ts.net` one keeps working |
| Copy of the page on GitHub Pages | GitHub's servers, holding a copy of the page that calls the Thor | stays `voltforge.tech` | hidden | none | two copies of the page to keep in step; the UI no longer comes from the Thor |
| Port forward on the router, Caddy on the Thor | the Thor itself | stays `voltforge.tech` | **public** | none | every internet host can reach port 443 at home; needs a public IPv4, which shared (CGNAT) connections don't have |
| Rented server with Caddy, over Tailscale | the rented server, passing everything to the Thor | stays `voltforge.tech` | hidden | ~$4–6 a month | one more machine to patch; it sees all traffic in clear text |

Why the redirect:

- **Nothing stands between visitors and the Thor.** After the redirect, the
  browser talks to the Thor through Funnel exactly as it does today. Namecheap
  sees only the first request, which carries no key and no message.
- **Nothing is exposed at home.** No router port is opened; the home IP stays
  unpublished. Port forwarding would put the Thor's whole HTTPS stack in front
  of every scanner on the internet, and a home IP, once published, can't be
  taken back.
- **Nothing new to run or patch.** No proxy, no second server, no second
  copy of the UI.
- **Fully reversible.** Deleting one record at Namecheap undoes it.

The cost is the address bar: it shows `arpanpathak.taildb9a39.ts.net` after
the jump. People share and bookmark `voltforge.tech/thor-tigress-cub`, which
keeps working as long as the redirect does.

## Every IP address in this story

| Address | Owner (from its network registration) | What it is | Use it? |
|---|---|---|---|
| `172.67.173.34`, `104.21.72.4`, `2606:4700:3037::ac43:ad22`, `2606:4700:3036::6815:4804` | Cloudflare (AS13335) | left over from when Cloudflare ran the domain's DNS; they answered with "error 1033", a Cloudflare tunnel with nothing behind it | **no: delete these records** |
| `185.199.108.153` to `185.199.111.153`, `2606:50c0:8000::153` to `2606:50c0:8003::153` | Fastly (AS54113), which delivers GitHub Pages | GitHub Pages' published addresses, proposed for the GitHub Pages copy | **no: not used** |
| `192.64.119.155` | Namecheap (AS22612) | Namecheap's redirect server; Namecheap sets this `A` record itself when a URL Redirect record exists, so you never type it | yes, implicitly |
| `208.111.35.209`, `208.111.34.11`, `2607:f740:0:3f::3cc`, `2607:f740:0:3f::2f0` | NetActuate (AS36236), hosting Tailscale's Funnel relays | what `arpanpathak.taildb9a39.ts.net` resolves to; Funnel relays the connection to the Thor | yes; the redirect's destination |
| `100.84.254.65` | Tailscale (private range 100.64.0.0/10) | the Thor's address inside the tailnet | only on your own devices |
| `192.168.0.189` | none; private home range | the Thor on the home Ethernet | only at home |

The ownership column comes from public routing registrations, which anyone can
query:

```bash
curl -s https://ipinfo.io/172.67.173.34/org      # AS13335 Cloudflare, Inc.
curl -s https://ipinfo.io/185.199.108.153/org    # AS54113 Fastly, Inc.
curl -s https://ipinfo.io/208.111.35.209/org     # AS36236 NetActuate, Inc
curl -s https://ipinfo.io/192.64.119.155/org     # AS22612 Namecheap, Inc.
```

The home connection's public IP appears nowhere in this table, and nothing in
this setup publishes it.

## What the Thor serves

`thor-tigress-agent` answers the same page at several paths, so the redirect
works whether or not Namecheap keeps the path:

| Path | Answer |
|---|---|
| `/`, `/thor-tigress-cub`, `/thor-tigress-cub/` | the chat |
| `/about.html` (also under `/thor-tigress-cub/`) | the About page: what it is, the numbers, how to get access |
| `/cub.svg`, `/cub.png` | the art; the PNG is the preview when the link is shared |
| `/health` | `{"status":"ok"}` |
| anything else outside `/v1/` | `404`; only these files are served, never the rest of the folder |

## Step 1: Namecheap answers for the domain

Namecheap → **Domain List** → **Manage** next to `voltforge.tech` →
**Nameservers** → **Namecheap BasicDNS** → save. Done on 2026-10-05.

Check:

```bash
curl -s "https://dns.google/resolve?name=voltforge.tech&type=NS" | python3 -m json.tool | grep data
```

It lists `dns1.registrar-servers.com` and `dns2.registrar-servers.com`.

## Step 2: delete the old records

**Manage** → **Advanced DNS** → **Host Records**. Delete:

- every `A` record with `172.67.173.34` or `104.21.72.4`
- every `AAAA` record starting with `2606:4700:`
- any record for host `www` pointing at Cloudflare or a parking page

Keep the `TXT` record `v=spf1 include:spf.efwd.registrar-servers.com ~all`:
it belongs to Namecheap's email forwarding, not to the website.

## Step 3: add the redirect

**Advanced DNS** → **Add New Record** → **URL Redirect Record**:

| Field | Value | Why |
|---|---|---|
| Host | `@` | the bare domain, `voltforge.tech` |
| Value | `https://arpanpathak.taildb9a39.ts.net` | the Thor; it serves the chat at `/` and at `/thor-tigress-cub/` |
| Type | **Unmasked**, **302 (temporary)** | see below |

Add a second one with Host `www` and the same value, so `www.voltforge.tech`
works too.

- **Unmasked, not masked.** A masked redirect keeps `voltforge.tech` in the
  address bar by loading the Thor inside a frame of a Namecheap page. Browsers
  then treat the chat as a third-party frame: its saved key and conversations
  are kept apart or blocked, and visitors can't see which site is really
  asking for their key.
- **302, not 301.** Browsers remember a 301 ("moved permanently") and stop
  asking Namecheap. If the redirect ever changes, people who visited before
  keep going to the old place. A 302 is asked again each time; the cost is
  one quick request.

## Step 4: check it

DNS changes at Namecheap usually apply within 30 minutes.

```bash
curl -sI http://voltforge.tech/thor-tigress-cub  | grep -iE "^HTTP|^location"
curl -sI https://voltforge.tech/thor-tigress-cub | grep -iE "^HTTP|^location"
curl -s  https://arpanpathak.taildb9a39.ts.net/health
```

Measured on 2026-10-05, a few minutes after saving the records:

| Request | Result |
|---|---|
| `http://voltforge.tech/thor-tigress-cub` | `302 Found`, `Location: https://arpanpathak.taildb9a39.ts.net`, `Server: namecheap-nginx` |
| `http://voltforge.tech/` and `http://www.voltforge.tech/thor-tigress-cub` | the same |
| `https://voltforge.tech/thor-tigress-cub` | **no answer**: the connection to port 443 times out |
| a browser opening `voltforge.tech/thor-tigress-cub` | lands on `https://arpanpathak.taildb9a39.ts.net/`, the chat |

Two things to know from these results:

- **Namecheap drops the path.** Every address on the domain goes to the
  Thor's `/`, which serves the chat. That is why `/thor-tigress-cub` works
  without any extra rule.
- **Namecheap's free redirect only works over HTTP.** Its redirect server
  doesn't answer on port 443, so `https://voltforge.tech` fails. Share the
  address as `voltforge.tech/thor-tigress-cub` or with `http://`, never with
  `https://`. Only the first hop is plain HTTP, and it carries nothing but
  the address; the chat itself, the key and every message go over HTTPS to
  the `.ts.net` address. A browser in "HTTPS-only" mode will refuse the first
  hop; the `.ts.net` address works there.

## What changes for visitors

| Before | After |
|---|---|
| share `https://arpanpathak.taildb9a39.ts.net` | share `voltforge.tech/thor-tigress-cub` |
| | the address bar shows the `.ts.net` address once the chat opens |
| key and conversations saved for the `.ts.net` address | the same; they are saved for the address that serves the page |

## Undo or change it later

- **Undo:** delete the URL Redirect records. The `.ts.net` address keeps working.
- **Keep `voltforge.tech` in the address bar later:** replace the redirect
  with a rented server running Caddy over Tailscale (fourth row of the
  table). The Thor needs no change: it already serves the chat at
  `/thor-tigress-cub/`.

## When a step fails

| Symptom | Cause | Fix |
|---|---|---|
| NS check still shows other nameservers | change not saved, or not spread yet | re-check step 1; wait up to 48 hours |
| `voltforge.tech` shows "error 1033" | the old Cloudflare records are still there | step 2 |
| `voltforge.tech` shows a Namecheap parking page | parking records still there, or the redirect not added | steps 2 and 3 |
| `https://voltforge.tech` times out or fails | Namecheap's free redirect has no HTTPS (measured above) | share `voltforge.tech/…` or `http://voltforge.tech/…`; the chat itself is always HTTPS |
| lands on the Thor but shows "not found" | the Thor's server is older than the `/thor-tigress-cub` route | rebuild and restart `thor-tigress-agent` (chapter "Web chat: Thor Tigress Cub") |
| red dot, "server not reachable" | the Thor or Funnel is down | `curl https://arpanpathak.taildb9a39.ts.net/health` |
