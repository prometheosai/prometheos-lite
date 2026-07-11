#!/usr/bin/env bash
#
# Governed provider-proposal smoke (#78).
#
# Builds the binary, creates a throwaway git repo, then drives the full safe path
# through the provider-backed `generate` command using the deterministic offline
# mock provider (no network, no API key):
#
#   generate (mock) -> dry-run -> approve -> apply -> validate -> report
#
# and proves:
#   - the fixture is unchanged before approval
#   - the generated patch is approved against its exact hash
#   - provider output with a '..' traversal is rejected
#   - provenance (no secrets) is present in the report
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
TARGET_DIR="${TARGET_DIR:-$ROOT/target}"
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) BIN="$TARGET_DIR/debug/prometheos.exe" ;;
  *) BIN="$TARGET_DIR/debug/prometheos" ;;
esac
[ -x "$BIN" ] || BIN="$TARGET_DIR/debug/prometheos"

echo "Building prometheos..."
cargo build --bin prometheos

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

REPO="$WORK/repo"
mkdir -p "$REPO/src"
cd "$REPO"
git init -q
git config user.email "smoke@prometheos.local"
git config user.name "smoke"

cat > src/main.rs <<'EOF'
pub fn main() {}
EOF
git add -A
git commit -qm "initial"

echo "== 1. generate via deterministic mock provider (offline) =="
OUT="$("$BIN" workflow generate --repo "$REPO" --goal "add generated file" --authority assist --allowed 'src/**' --provider mock)"
echo "$OUT"
ID="$(printf '%s' "$OUT" | head -n1 | tr -d '\r')"
PHASH="$(printf '%s' "$OUT" | sed -n 's/^patch_hash=\(.*\)$/\1/p' | tr -d '\r')"
[ -n "$ID" ] || { echo "FAIL: generate returned no id"; exit 1; }
[ -n "$PHASH" ] || { echo "FAIL: generate returned no patch hash"; exit 1; }
echo "   id=$ID patch_hash=$PHASH"

echo "== 2. fixture unchanged before approval =="
[ -f src/generated_patch.rs ] && { echo "FAIL: patch applied before approval"; exit 1; }
grep -q 'pub fn main()' src/main.rs || { echo "FAIL: fixture source modified before approval"; exit 1; }
echo "   fixture intact (good)"

echo "== 3. rejected malicious provider output (path traversal) =="
if PROMETHEOS_MOCK_MODE=traversal "$BIN" workflow generate --repo "$REPO" --goal g --authority assist --allowed 'src/**' --provider mock >/dev/null 2>&1; then
  echo "FAIL: traversal provider output was accepted"; exit 1
fi
echo "   traversal rejected (good)"

echo "== 4. dry-run passes in isolated worktree =="
if ! "$BIN" workflow dry-run --repo "$REPO" "$ID" --validate "grep -q generated src/generated_patch.rs"; then
  echo "FAIL: dry-run should pass"; exit 1
fi

echo "== 5. approve exact generated patch hash =="
"$BIN" workflow approve --repo "$REPO" "$ID" --patch-hash "$PHASH"

echo "== 6. apply + validate =="
"$BIN" workflow apply --repo "$REPO" "$ID" --patch-hash "$PHASH" --validate "grep -q generated src/generated_patch.rs"
grep -q generated src/generated_patch.rs || { echo "FAIL: tree not patched"; exit 1; }
echo "   tree patched (good)"

echo "== 7. report exposes provenance without secrets =="
REPORT="$("$BIN" workflow report --repo "$REPO" "$ID")"
echo "$REPORT" | grep -q '"implementation": "mock"' || { echo "FAIL: provenance missing"; exit 1; }
echo "$REPORT" | grep -qi 'sk-\|authorization\|bearer' && { echo "FAIL: secret leaked into report"; exit 1; }
echo "   provenance present, no secrets (good)"

echo "GOVERNED PROVIDER-PROPOSAL SMOKE PASSED"
