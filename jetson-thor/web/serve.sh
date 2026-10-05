#!/usr/bin/env bash
# Serves Nemotron and the chat page from the Jetson Thor, on localhost only.
# Tailscale (see the book, chapter "Web chat") makes it reachable elsewhere.
#
#   browser ─► :8080 thor-tigress-agent ─► :8079 llama-server (Nemotron)
#                         └──────────────► :8888 SearXNG (web search)
#
#   ./serve.sh install      run both as services, started at boot
#   ./serve.sh uninstall    stop and remove both services
#   ./serve.sh logs         follow both logs
#   ./serve.sh key          create an access key; the page asks for it once
#   ./serve.sh run          run llama-server in this terminal
#   ./serve.sh agent        run thor-tigress-agent in this terminal
#
# Settings (environment, or ~/.config/thor-chat/env):
#   MODEL       GGUF file            default: Nemotron 3 Nano 30B-A3B Q8_0
#   USERS       replies at once      default: 4
#   CONTEXT     tokens per reply     default: 1048576 (the model's full context)
#   PORT        page and API port    default: 8080
#   MODEL_PORT  llama-server port    default: 8079
#   SEARCH_PORT SearXNG port         default: 8888
set -euo pipefail

CONFIG="${HOME}/.config/thor-chat"
[ -f "${CONFIG}/env" ] && . "${CONFIG}/env"

MODEL="${MODEL:-${HOME}/models/gguf/Nemotron-3-Nano-30B-A3B/NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0.gguf}"
USERS="${USERS:-4}"
CONTEXT="${CONTEXT:-1048576}"
PORT="${PORT:-8080}"
MODEL_PORT="${MODEL_PORT:-8079}"
SEARCH_PORT="${SEARCH_PORT:-8888}"
SERVER="${SERVER:-${HOME}/.local/src/llama.cpp/build/bin/llama-server}"
AGENT="${AGENT:-${HOME}/.cargo/bin/thor-tigress-agent}"
WEB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNITS="${HOME}/.config/systemd/user"

run() {
  local key_args=()
  [ -s "${CONFIG}/api-key" ] && key_args=(--api-key-file "${CONFIG}/api-key")
  exec "$SERVER" \
    --model "$MODEL" \
    --host 127.0.0.1 --port "$MODEL_PORT" \
    --n-gpu-layers 999 \
    --flash-attn on \
    --parallel "$USERS" \
    --ctx-size "$((CONTEXT * USERS))" \
    --jinja \
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
ExecStart=${WEB}/serve.sh ${command}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNITFILE
}

case "${1:-}" in
  run) run ;;
  agent) agent ;;
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
    sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'
    ;;
esac
