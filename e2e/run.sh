#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

WP=${WP:-$(cd .. && pwd)}
DATA_DIR=${WP_DATA_DIR:-$(mktemp -d /tmp/wp-e2e-data.XXXXXX)}
PORT=${WP_PORT:-$(python3 -c 'import socket;s=socket.socket();s.bind(("",0));print(s.getsockname()[1]);s.close()')}
SHARE_DIR=${WP_SHARE_DIR:-/tmp/hostshare}

echo "[run] building debug binary (embeds static/app.js)"
(cd "$WP" && cargo build >/dev/null 2>&1 || cargo build)

[ -d node_modules ] || npm install

cache="$HOME/.cache/ms-playwright"
if ! ls "$cache" 2>/dev/null | grep -q chromium_headless_shell; then
  echo "[run] installing chromium headless shell"
  npx playwright-core install chromium-headless-shell
fi

echo "[run] starting server on 0.0.0.0:$PORT data=$DATA_DIR"
old_pid=$(pgrep -f "wp --listen 0.0.0.0:$PORT" || true)
[ -n "$old_pid" ] && kill $old_pid 2>/dev/null || true
"$WP/target/debug/wp" --listen "0.0.0.0:$PORT" --data "$DATA_DIR" >"$DATA_DIR/server.log" 2>&1 &
SRV=$!
trap 'kill $SRV 2>/dev/null || true; rm -rf "$DATA_DIR"' EXIT

for i in $(seq 1 30); do
  code=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:$PORT/" || true)
  [ "$code" = "200" ] && break
  sleep 1
done
[ "$code" = "200" ] || { echo "server did not become ready"; cat "$DATA_DIR/server.log"; exit 1; }

echo "[setup] generating fixture media"
WP_SHARE_DIR="$SHARE_DIR" npm run setup

echo "[run] e2e suite"
WP_BASE="http://127.0.0.1:$PORT" WP_SHARE_DIR="$SHARE_DIR" node e2e.js