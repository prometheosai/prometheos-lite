#!/usr/bin/env bash
# Phase 2 full-stack compatibility smoke.
#
# Builds the prometheos binary, starts `prometheos serve`, and verifies the
# frontend-reachable routes over real HTTP with curl. This exercises the same
# project CRUD contract covered by the in-memory Phase 1 tests, but through the
# actual binary + server bootstrap (build, config load, port bind, route
# registration).
#
# No model/flow execution is involved: health and project CRUD are data-only.
# No dependencies beyond curl + cargo + shell built-ins.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${PORT:-3100}"
PROJECT_NAME="smoke-test-project-$(date +%s)"
WORK="$(mktemp -d)"
SERVER_PID=""

cleanup() {
  if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK" 2>/dev/null || true
}
trap cleanup EXIT

fail() {
  echo "FULLSTACK SMOKE FAILED: $1"
  exit 1
}

# Build the binary (honors CARGO_TARGET_DIR like every cargo invocation).
echo "Building prometheos binary..."
( cd "$REPO_ROOT" && cargo build --bin prometheos ) || fail "cargo build failed"

# Resolve the target dir the way cargo itself does: CARGO_TARGET_DIR env,
# else the configured target dir via cargo metadata, else the conventional
# fallbacks. The binary can legitimately live outside the repo when the
# build is routed to another volume. A relative CARGO_TARGET_DIR is
# anchored to the repository root.
TARGET_DIR="${CARGO_TARGET_DIR:-}"
if [ -z "$TARGET_DIR" ]; then
  TARGET_DIR="$(cd "$REPO_ROOT" && cargo metadata --format-version 1 --no-deps 2>/dev/null | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
fi
case "$TARGET_DIR" in
  "" ) ;;
  /* ) ;;
  [A-Za-z]:* ) ;;
  //* ) ;;
  *  ) TARGET_DIR="$REPO_ROOT/$TARGET_DIR" ;;
esac
# Normalize Windows separators so `[ -x ]` works regardless of whether
# MSYS performed its env-var path conversion (see demo script note).
TARGET_DIR="${TARGET_DIR//\\//}"
BIN=""
for cand in \
  "$TARGET_DIR/debug/prometheos" \
  "$TARGET_DIR/release/prometheos" \
  "$REPO_ROOT/target/debug/prometheos" \
  "$REPO_ROOT/.cargo-target/debug/prometheos"; do
  if [ -n "$cand" ] && [ -x "$cand" ]; then BIN="$cand"; break; fi
done
[ -n "$BIN" ] || fail "prometheos binary not found after build"

# Run the server in an isolated dir with a copied config so the repo tree
# (prometheos.db, prometheos-memory.db) stays clean.
cp "$REPO_ROOT/prometheos.config.json" "$WORK/prometheos.config.json" || fail "could not copy config"
cd "$WORK" || fail "could not enter work dir"

echo "Starting server on port $PORT..."
"$BIN" serve --port "$PORT" > "$WORK/server.log" 2>&1 &
SERVER_PID=$!
[ -n "$SERVER_PID" ] || fail "server did not start"

# Wait for readiness.
echo "Waiting for /health..."
for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    fail "server exited early; log:\n$(cat "$WORK/server.log")"
  fi
  sleep 1
done
curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 || fail "server did not become ready (timeout)"

# Health body.
HEALTH=$(curl -s "http://127.0.0.1:$PORT/health")
echo "health: $HEALTH"
echo "$HEALTH" | grep -q '"status":"ok"' || fail "health body missing status ok"

# Project CRUD cycle.
echo "POST /projects"
RESP=$(curl -s -w "\n%{http_code}" -X POST "http://127.0.0.1:$PORT/projects" \
  -H "Content-Type: application/json" \
  -d "{\"name\":\"$PROJECT_NAME\"}")
CODE=$(printf '%s' "$RESP" | tail -1)
BODY=$(printf '%s' "$RESP" | sed '$d')
[ "$CODE" = "201" ] || fail "POST /projects expected 201, got $CODE"
ID=$(printf '%s' "$BODY" | grep -o '"id":"[^"]*"' | head -1 | sed 's/"id":"//;s/"$//')
[ -n "$ID" ] || fail "could not parse project id from: $BODY"
echo "created project id=$ID"

echo "GET /projects"
LIST=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$PORT/projects")
LCODE=$(printf '%s' "$LIST" | tail -1)
[ "$LCODE" = "200" ] || fail "GET /projects expected 200, got $LCODE"
printf '%s' "$LIST" | grep -q "$PROJECT_NAME" || fail "GET /projects missing created project"

echo "GET /projects/$ID"
ONE=$(curl -s -w "\n%{http_code}" "http://127.0.0.1:$PORT/projects/$ID")
OCODE=$(printf '%s' "$ONE" | tail -1)
[ "$OCODE" = "200" ] || fail "GET /projects/:id expected 200, got $OCODE"
printf '%s' "$ONE" | grep -q "$PROJECT_NAME" || fail "GET /projects/:id missing project name"

echo "FULLSTACK SMOKE PASSED"
exit 0
