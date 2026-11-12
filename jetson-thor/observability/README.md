# Observability

Request telemetry and per-key rate limits for the model servers, with dashboards in
[OpenObserve](https://github.com/openobserve/openobserve) v1.1.2.

```
thor-tigress-agent ──► requests.jsonl ──► thor-tigress-telemetry ──► OpenObserve :5080
  one line per          tail                + agent/engine health      streams: requests, health
  model request                             + memory, load             dashboard: Thor traffic
```

- `thor-tigress-agent` writes one JSON line per `/v1/chat/completions` and `/v1/messages`
  request to `~/.local/state/thor-tigress/requests.jsonl`: person, model, status, time to
  first byte, total time, bytes, input and output tokens, and whether a rate limit refused it.
- Rate limits per key: 30 requests a minute, 4 at once (`--limit-per-minute`,
  `--limit-at-once`). The operator key is not limited. A refusal is a 429 with `Retry-After`.
- `thor-tigress-telemetry` ships new log lines and a health check of the agent and each
  engine every 5 s. Requests OpenObserve does not accept are kept and sent next round.
- OpenObserve listens on 127.0.0.1 only. Its own telemetry is off; data is kept 30 days.

## Install on the Thor

```bash
V=v1.1.2; D=~/.local/opt/openobserve/$V; B=https://downloads.openobserve.ai/releases/openobserve/$V
mkdir -p $D && cd $D
curl -fsSLO $B/openobserve-$V-linux-arm64.tar.gz
curl -fsSLO $B/openobserve-$V-linux-arm64.tar.gz.sha256sum
echo "$(cut -d' ' -f1 openobserve-$V-linux-arm64.tar.gz.sha256sum)  openobserve-$V-linux-arm64.tar.gz" | sha256sum -c -
tar -xzf openobserve-$V-linux-arm64.tar.gz
cd -

# OpenObserve refuses a password without an upper, a lower, a digit and a special character.
(umask 077; printf '%s-Aa1' "$(head -c 18 /dev/urandom | base64 | tr -d '/+=')" > ~/.config/thor-chat/openobserve-password)

cp jetson-thor/observability/openobserve-run.sh ~/.local/opt/openobserve/run.sh
cp jetson-thor/observability/thor-*.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now thor-openobserve thor-tigress-telemetry
```

`thor-tigress-telemetry` is built from `crates/thor-tigress-telemetry` and installed in
`~/.cargo/bin`.

## Dashboard

```bash
P=$(cat ~/.config/thor-chat/openobserve-password)
curl -u "admin@thor.local:$P" -H 'Content-Type: application/json' \
  --data @jetson-thor/observability/thor-traffic.dashboard.json \
  127.0.0.1:5080/api/default/dashboards
```

From another machine:

```bash
ssh -L 5080:127.0.0.1:5080 thor     # then http://127.0.0.1:5080, user admin@thor.local
```

| Panel | Shows |
|---|---|
| Requests, answer tokens, errors, rate-limited | totals over the selected range |
| Requests per minute, by model | `count(*)` per minute |
| Output tokens per second, by model | output tokens over first-byte-to-last-byte time; streamed answers only |
| p95 request time, p50 time to first byte | `approx_percentile_cont` per minute |
| Use per person | requests, refusals and output tokens per key holder |
| Answers by status | count per HTTP status |
| Servers | share of health checks answered, slowest check |
| Memory available | lowest `MemAvailable` per minute |

## Known gaps

- OpenAI-format streams that do not set `stream_options.include_usage` carry no token
  counts and are logged with 0 tokens.
- A non-streamed answer arrives in one piece, so its time to first byte equals its total time.
