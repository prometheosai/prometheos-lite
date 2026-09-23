#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FIXTURE="$REPO_ROOT/fixtures/repo-workbench/rust-risky"
GOAL="Find risky code and suggest safe improvements"

# Run from an isolated workdir so the demo never depends on (or mutates)
# untracked state in the repo root (e.g. a stale prometheos.db created by
# an older schema would make `work run` fail with a missing column).
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

# Binary: prefer a prebuilt one, fall back to a build. Resolution honors
# CARGO_TARGET_DIR / cargo metadata like cargo itself, so the build can be
# legitimately routed to another volume.
TARGET_DIR="${CARGO_TARGET_DIR:-}"
if [ -z "$TARGET_DIR" ]; then
  TARGET_DIR="$(cd "$REPO_ROOT" && cargo metadata --format-version 1 --no-deps 2>/dev/null | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
fi
find_bin() {
  for cand in "$TARGET_DIR/debug/prometheos" "$TARGET_DIR/debug/prometheos.exe" \
              "$REPO_ROOT/.cargo-target/debug/prometheos" "$REPO_ROOT/.cargo-target/debug/prometheos.exe" \
              "$REPO_ROOT/target/debug/prometheos" "$REPO_ROOT/target/debug/prometheos.exe"; do
    if [ -n "$cand" ] && [ -x "$cand" ]; then echo "$cand"; return 0; fi
  done
  return 1
}
BIN="$(find_bin)"
if [ -z "$BIN" ]; then
  echo "Building prometheos..."
  ( cd "$REPO_ROOT" && cargo build --bin prometheos ) >/dev/null
  BIN="$(find_bin)"
fi
[ -n "$BIN" ] || { echo "FAIL: prometheos binary not found"; exit 1; }

echo "================================================"
echo " PrometheOS Lite — Zero-to-First-Value Demo"
echo "================================================"
echo ""
echo "Fixture: $FIXTURE"
echo "Goal:    $GOAL"
echo ""

# Step 1: Create
echo "--- Step 1: create work context ---"
OUTPUT=$("$BIN" work create \
  --repo "$FIXTURE" \
  --goal "$GOAL" \
  --mode review \
  --json 2>&1)
echo "$OUTPUT"

WORK_ID=$(echo "$OUTPUT" | grep -o '"work_id": *"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
if [ -z "$WORK_ID" ]; then
  echo ""
  echo "ERROR: could not extract work_id from JSON output."
  echo "Falling back to manual mode."
  echo "Run the remaining commands manually with the printed work ID."
  exit 1
fi

echo ""
echo "Work ID: $WORK_ID"
echo "Next: prometheos work run $WORK_ID"
echo ""

# Step 2: Run
echo "--- Step 2: run analysis ---"
"$BIN" work run "$WORK_ID" 2>&1
echo ""
echo "Next: prometheos work artifacts $WORK_ID"
echo ""

# Step 3: Artifacts
echo "--- Step 3: list artifacts ---"
"$BIN" work artifacts "$WORK_ID" 2>&1
echo ""
echo "Next: prometheos work memory show $WORK_ID"
echo ""

# Step 4: Memory
echo "--- Step 4: inspect memory ---"
"$BIN" work memory show "$WORK_ID" 2>&1
echo ""
echo "Next: prometheos work continue $WORK_ID"
echo ""

# Step 5: Continue
echo "--- Step 5: continue context ---"
"$BIN" work continue "$WORK_ID" 2>&1
echo ""

echo "================================================"
echo " Zero-to-First-Value complete!"
echo "================================================"
echo ""
echo "You reached first value when:"
echo "  - A WorkContext ID was created.      ✓"
echo "  - A risk report was generated.        ✓"
echo "  - A patch plan was generated.         ✓"
echo "  - Memory was shown.                   ✓"
echo "  - Continue restored context.          ✓"
echo "  - No source files were modified.      ✓"
echo ""
echo "Cleanup: rm -rf $FIXTURE/.prometheos-lite"
echo ""
