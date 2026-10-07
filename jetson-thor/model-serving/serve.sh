#!/usr/bin/env bash
# Serves Nemotron and the chat page from the Jetson Thor, on localhost only.
# Tailscale (see the book, chapter "Web chat") makes it reachable elsewhere.
#
#   browser ─► :8080 thor-tigress-agent ─► :8079 llama-server router ─┬─ Nemotron 3 Nano
#                         │                                           └─ Nemotron 3.5 Lightning
#                         └──────────────► :8888 SearXNG (web search)
#
# The router picks the model by the request's "model" field. The Nano also
# answers to "nemotron", "nemotron-think" and its file path; Lightning to
# "lightning". Any other name is refused.
#
#   ./serve.sh install      run both as services, started at boot
#   ./serve.sh uninstall    stop and remove both services
#   ./serve.sh logs         follow both logs
#   ./serve.sh key          create an access key; the page asks for it once
#   ./serve.sh run          run llama-server in this terminal
#   ./serve.sh agent        run thor-tigress-agent in this terminal
#
#   ./serve.sh models       list every model: id, state, aliases
#   ./serve.sh load NAME    load a model (id or alias), send it one token to
#                           commit its memory, and unload it again if free
#                           memory falls below MIN_FREE_GB on the way
#   ./serve.sh unload NAME  free a model's memory; it reloads on its next request
#   ./serve.sh reload       re-read the presets, including models.local.ini;
#                           new models are listed, not loaded
#   ./serve.sh memory       memory available now, and which models are loaded
#
# Models to try go in ~/.config/thor-chat/models.local.ini, one section each
# (book, chapter "Model serving"). serve.sh appends that file to the presets
# it writes, so it survives restarts.
#
# Settings (environment, or ~/.config/thor-chat/env):
#   MODEL       first GGUF file      default: Nemotron 3 Nano 30B-A3B Q8_0
#   LIGHTNING   second GGUF file     default: Nemotron 3.5 Lightning 30B-A3B Q8_0;
#               listed but not loaded at start; "none" (or a missing file)
#               leaves it out of the presets
#   USERS       replies at once      default: 4
#   CONTEXT     tokens per reply     default: 1048576 (the model's full context)
#   LIGHTNING_USERS    Lightning's replies at once   default: 1
#   LIGHTNING_CONTEXT  Lightning's tokens per reply  default: 262144
#   Both at 4 x 1M do not fit in the Thor's memory; at these defaults about
#   24 GB stays free (measured 2026-10-06).
#   MIN_FREE_GB   ./serve.sh load undoes a load below this   default: 8
#   MODELS_MAX  models in memory at once  default: 2; loading one more
#               first unloads the least recently used, which may be the Nano
#   PORT        page and API port    default: 8080
#   MODEL_PORT  llama-server port    default: 8079
#   SEARCH_PORT SearXNG port         default: 8888
set -euo pipefail

CONFIG="${HOME}/.config/thor-chat"
[ -f "${CONFIG}/env" ] && . "${CONFIG}/env"

MODEL="${MODEL:-${HOME}/models/gguf/Nemotron-3-Nano-30B-A3B/NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0.gguf}"
LIGHTNING="${LIGHTNING:-${HOME}/models/gguf/Nemotron-3.5-Lightning-30B-A3B/NVIDIA-Nemotron-3.5-Lightning-30B-A3B-Q8_0.gguf}"
USERS="${USERS:-4}"
CONTEXT="${CONTEXT:-1048576}"
LIGHTNING_USERS="${LIGHTNING_USERS:-1}"
LIGHTNING_CONTEXT="${LIGHTNING_CONTEXT:-262144}"
MODELS_MAX="${MODELS_MAX:-2}"
MIN_FREE_GB="${MIN_FREE_GB:-8}"
PORT="${PORT:-8080}"
MODEL_PORT="${MODEL_PORT:-8079}"
SEARCH_PORT="${SEARCH_PORT:-8888}"
SERVER="${SERVER:-${HOME}/.local/src/llama.cpp/build/bin/llama-server}"
AGENT="${AGENT:-${HOME}/.cargo/bin/thor-tigress-agent}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WEB="$(cd "${HERE}/../web" && pwd)"
UNITS="${HOME}/.config/systemd/user"

# One preset section per model; the section name is the id the page lists.
presets() {
  cat <<PRESETS
version = 1

[*]
n-gpu-layers = 999
flash-attn = on
jinja = true
load-on-startup = true

[$(basename "$MODEL" .gguf)]
model = ${MODEL}
alias = nemotron,nemotron-think,${MODEL}
parallel = ${USERS}
ctx-size = $((CONTEXT * USERS))
PRESETS
  if [ -f "$LIGHTNING" ]; then
    cat <<PRESETS

[$(basename "$LIGHTNING" .gguf)]
model = ${LIGHTNING}
alias = lightning
load-on-startup = false
parallel = ${LIGHTNING_USERS}
ctx-size = $((LIGHTNING_CONTEXT * LIGHTNING_USERS))
PRESETS
  fi
  if [ -f "${CONFIG}/models.local.ini" ]; then
    echo
    cat "${CONFIG}/models.local.ini"
  fi
}

# Calls the router on localhost with the access key, if there is one.
router() {
  local method="$1" path="$2" body="${3:-}"
  local auth=()
  [ -s "${CONFIG}/api-key" ] && auth=(-H "Authorization: Bearer $(cat "${CONFIG}/api-key")")
  if [ -n "$body" ]; then
    curl -sS -X "$method" "http://127.0.0.1:${MODEL_PORT}${path}" "${auth[@]}" -H "Content-Type: application/json" -d "$body"
  else
    curl -sS -X "$method" "http://127.0.0.1:${MODEL_PORT}${path}" "${auth[@]}"
  fi
  echo
}

models() {
  router GET "/models${1:-}" | python3 -c '
import json, sys
for model in json.load(sys.stdin)["data"]:
    status = model["status"]
    state = status["value"]
    if status.get("failed"):
        state += " (failed, exit %s)" % status.get("exit_code")
    print("%-48s %-24s %s" % (model["id"], state, ", ".join(model.get("aliases", []))))'
}

# The state of one model, found by id or alias: loaded, loading, unloaded,
# "failed", or "unknown".
state() {
  router GET /models | python3 -c '
import json, sys
name = sys.argv[1]
for model in json.load(sys.stdin)["data"]:
    if name == model["id"] or name in model.get("aliases", []):
        print("failed" if model["status"].get("failed") else model["status"]["value"])
        break
else:
    print("unknown")' "$1"
}

free_gb() {
  awk '/MemAvailable/ {print int($2 / 1048576)}' /proc/meminfo
}

# Loads a model and commits its memory with a one-token reply. Memory on the
# Thor is committed on first use, so a load alone can look fine and the first
# reply can still run out; both are watched.
load() {
  local name="$1" warmup start=$SECONDS
  [ "$(state "$name")" = unknown ] && { echo "no model called $name; see $0 models" >&2; return 2; }
  router POST /models/load "{\"model\": \"$name\"}" >/dev/null
  until [ "$(state "$name")" = loaded ]; do
    case "$(state "$name")" in
      failed) echo "$name failed to load; see journalctl --user -u thor-chat" >&2; return 1 ;;
    esac
    if [ "$(free_gb)" -lt "$MIN_FREE_GB" ]; then
      router POST /models/unload "{\"model\": \"$name\"}" >/dev/null
      echo "free memory fell below ${MIN_FREE_GB} GB while loading; $name unloaded" >&2
      return 1
    fi
    sleep 1
  done
  echo "$name loaded in $((SECONDS - start)) s; committing its memory with one token"
  router POST /v1/chat/completions "{\"model\": \"$name\", \"max_tokens\": 1, \"messages\": [{\"role\": \"user\", \"content\": \"hi\"}]}" >/dev/null &
  warmup=$!
  while kill -0 "$warmup" 2>/dev/null; do
    if [ "$(free_gb)" -lt "$MIN_FREE_GB" ]; then
      router POST /models/unload "{\"model\": \"$name\"}" >/dev/null
      echo "free memory fell below ${MIN_FREE_GB} GB on the first reply; $name unloaded" >&2
      return 1
    fi
    sleep 0.5
  done
  memory
}

# Per-process numbers miss the GPU buffers on the Thor's unified memory, so
# the honest measure is MemAvailable before and after a load or unload.
memory() {
  awk '/MemTotal|MemAvailable/ {printf "%-14s %6.1f GB\n", $1, $2 / 1048576}' /proc/meminfo
  models | awk '$2 == "loaded" {print "loaded:        " $1}'
}

run() {
  local key_args=()
  [ -s "${CONFIG}/api-key" ] && key_args=(--api-key-file "${CONFIG}/api-key")
  mkdir -p "$CONFIG"
  presets > "${CONFIG}/models.ini"
  exec "$SERVER" \
    --models-preset "${CONFIG}/models.ini" \
    --models-max "$MODELS_MAX" \
    --host 127.0.0.1 --port "$MODEL_PORT" \
    "${key_args[@]}"
}

agent() {
  exec "$AGENT" \
    --listen "127.0.0.1:${PORT}" \
    --model "127.0.0.1:${MODEL_PORT}" \
    --search "127.0.0.1:${SEARCH_PORT}" \
    --web "$WEB" \
    --key-file "${CONFIG}/api-key"
}

unit() {
  local name="$1" description="$2" command="$3" after="$4"
  cat > "${UNITS}/${name}.service" <<UNITFILE
[Unit]
Description=${description}
After=network-online.target ${after}

[Service]
ExecStart=${HERE}/serve.sh ${command}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNITFILE
}

case "${1:-}" in
  run) run ;;
  agent) agent ;;
  models) models ;;
  load)
    [ -n "${2:-}" ] || { echo "usage: $0 load NAME (see $0 models)" >&2; exit 2; }
    load "$2"
    ;;
  unload)
    [ -n "${2:-}" ] || { echo "usage: $0 unload NAME (see $0 models)" >&2; exit 2; }
    router POST /models/unload "{\"model\": \"$2\"}"
    ;;
  reload)
    presets > "${CONFIG}/models.ini"
    models "?reload=1"
    ;;
  memory) memory ;;
  install)
    mkdir -p "$UNITS"
    unit thor-chat "Nemotron (llama-server) on 127.0.0.1:${MODEL_PORT}" run ""
    unit thor-tigress-agent "Chat page and web search on 127.0.0.1:${PORT}" agent "thor-chat.service"
    systemctl --user daemon-reload
    systemctl --user enable thor-chat thor-tigress-agent >/dev/null 2>&1
    systemctl --user restart thor-chat thor-tigress-agent
    loginctl enable-linger "$USER" 2>/dev/null || true
    echo "running; both start at boot. Logs: $0 logs"
    ;;
  uninstall)
    systemctl --user disable --now thor-tigress-agent thor-chat 2>/dev/null || true
    rm -f "${UNITS}/thor-chat.service" "${UNITS}/thor-tigress-agent.service"
    systemctl --user daemon-reload
    echo "stopped and removed"
    ;;
  logs)
    journalctl --user -u thor-chat -u thor-tigress-agent -f
    ;;
  key)
    mkdir -p "$CONFIG"
    head -c 24 /dev/urandom | base64 | tr -d '/+=' > "${CONFIG}/api-key"
    chmod 600 "${CONFIG}/api-key"
    echo "new key written to ${CONFIG}/api-key"
    echo "apply it: systemctl --user restart thor-chat thor-tigress-agent"
    ;;
  *)
    sed -n '2,/^set -euo/{/^#/p}' "$0" | sed 's/^# \{0,1\}//'
    ;;
esac
