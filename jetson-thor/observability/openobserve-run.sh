#!/bin/sh
# Starts OpenObserve on localhost only. The root password is read from its 0600 file.
export ZO_ROOT_USER_EMAIL=admin@thor.local
export ZO_ROOT_USER_PASSWORD="$(cat "$HOME/.config/thor-chat/openobserve-password")"
export ZO_DATA_DIR="$HOME/.local/share/openobserve"
export ZO_HTTP_ADDR=127.0.0.1
export ZO_HTTP_PORT=5080
export ZO_GRPC_ADDR=127.0.0.1
export ZO_GRPC_PORT=5081
export ZO_TELEMETRY=false
export ZO_MEMORY_CACHE_MAX_SIZE=256
export ZO_COMPACT_DATA_RETENTION_DAYS=30
exec "$HOME/.local/opt/openobserve/v1.1.2/openobserve"
