#!/usr/bin/env bash
# Serves Nemotron and the chat page from the Jetson Thor, on localhost only.
# Tailscale (see README.md) is what makes it reachable from other machines.
#
#   ./serve.sh run          run in this terminal
#   ./serve.sh install      run as a service, started at boot
#   ./serve.sh uninstall    stop and remove the service
#   ./serve.sh logs         follow the service log
#   ./serve.sh key          create an access key; the page asks for it once
#
# Settings (environment, or ~/.config/thor-chat/env):
#   MODEL     GGUF file          default: Nemotron 3 Nano 30B-A3B Q8_0
#   USERS     people at once     default: 4
#   CONTEXT   tokens per person  default: 1048576 (the model's full context)
#   PORT      local port         default: 8080
set -euo pipefail

CONFIG="${HOME}/.config/thor-chat"
[ -f "${CONFIG}/env" ] && . "${CONFIG}/env"

MODEL="${MODEL:-${HOME}/models/gguf/Nemotron-3-Nano-30B-A3B/NVIDIA-Nemotron-3-Nano-30B-A3B-Q8_0.gguf}"
USERS="${USERS:-4}"
CONTEXT="${CONTEXT:-1048576}"
PORT="${PORT:-8080}"
SERVER="${SERVER:-${HOME}/.local/src/llama.cpp/build/bin/llama-server}"
WEB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVICE="thor-chat"
UNIT="${HOME}/.config/systemd/user/${SERVICE}.service"

run() {
  local key_args=()
  [ -s "${CONFIG}/api-key" ] && key_args=(--api-key-file "${CONFIG}/api-key")
  exec "$SERVER" \
    --model "$MODEL" \
    --host 127.0.0.1 --port "$PORT" \
    --path "$WEB" \
    --n-gpu-layers 999 \
    --flash-attn on \
    --parallel "$USERS" \
    --ctx-size "$((CONTEXT * USERS))" \
    --jinja \
    "${key_args[@]}"
}

case "${1:-}" in
  run)
    run
    ;;
  install)
    mkdir -p "$(dirname "$UNIT")"
    cat > "$UNIT" <<UNITFILE
[Unit]
Description=Nemotron chat (llama-server) on 127.0.0.1:${PORT}
After=network-online.target

[Service]
ExecStart=${WEB}/serve.sh run
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target
UNITFILE
    systemctl --user daemon-reload
    systemctl --user enable --now "$SERVICE"
    loginctl enable-linger "$USER" 2>/dev/null || true
    echo "running; it starts at boot. Logs: $0 logs"
    ;;
  uninstall)
    systemctl --user disable --now "$SERVICE" 2>/dev/null || true
    rm -f "$UNIT"
    systemctl --user daemon-reload
    echo "stopped and removed"
    ;;
  logs)
    journalctl --user -u "$SERVICE" -f
    ;;
  key)
    mkdir -p "$CONFIG"
    head -c 24 /dev/urandom | base64 | tr -d '/+=' > "${CONFIG}/api-key"
    chmod 600 "${CONFIG}/api-key"
    echo "access key: $(cat "${CONFIG}/api-key")"
    echo "restart to apply: systemctl --user restart ${SERVICE}"
    ;;
  *)
    sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
    ;;
esac
