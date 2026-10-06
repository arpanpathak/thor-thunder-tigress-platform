#!/usr/bin/env bash
# Builds the voltforge.tech site into OUT_DIR:
#   OUT_DIR/CNAME                         voltforge.tech, for GitHub Pages
#   OUT_DIR/index.html                    forwards to /thor-tigress-cub/
#   OUT_DIR/thor-tigress-cub/index.html   the chat page, calling the Thor's public address
#   OUT_DIR/thor-tigress-cub/about.html   what it is, for people arriving from a link
# Usage: ./build.sh OUT_DIR    (THOR_API overrides the Thor's address)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
out="${1:?usage: build.sh OUT_DIR}"
api="${THOR_API:-https://arpanpathak.taildb9a39.ts.net/}"

mkdir -p "$out/thor-tigress-cub"
sed "s|<meta name=\"thor-api\" content=\"\">|<meta name=\"thor-api\" content=\"${api}\">|" \
  "$here/../web/index.html" > "$out/thor-tigress-cub/index.html"
grep -q "content=\"${api}\"" "$out/thor-tigress-cub/index.html"
cp "$here/about.html" "$here/../web/cub.svg" "$here/cub.png" "$out/thor-tigress-cub/"
echo "voltforge.tech" > "$out/CNAME"
cat > "$out/index.html" <<'HTML'
<!doctype html>
<meta charset="utf-8">
<title>voltforge.tech</title>
<meta http-equiv="refresh" content="0; url=/thor-tigress-cub/">
<link rel="canonical" href="/thor-tigress-cub/">
<a href="/thor-tigress-cub/">Thor Tigress Cub</a>
HTML
echo "built $out (API ${api})"
