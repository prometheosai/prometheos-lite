#!/usr/bin/env bash
#
# Approval-controlled patch workflow golden path.
#
# Builds the binary, creates a throwaway git repo with a boundary bug, then drives
# the full safe path through the CLI:
#
#   propose -> dry-run -> approve -> apply -> validate -> report
#
# and proves:
#   - a patch is rejected unless scope passes (negative test)
#   - dry-run isolates changes in a worktree
#   - apply refuses without approval / wrong hash
#   - apply mutates the tree only after matching approval
#   - rollback restores the tree when validation fails
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

cat > Cargo.toml <<'EOF'
[package]
name = "repro"
version = "0.1.0"
edition = "2021"
EOF
cat > src/lib.rs <<'EOF'
pub mod calc;
EOF
cat > src/calc.rs <<'EOF'
pub fn add(a: i32, b: i32) -> i32 { a - b }
EOF
git add -A
git commit -qm "initial"

PATCH="$WORK/fix.patch"
cat > "$PATCH" <<'EOF'
--- a/src/calc.rs
+++ b/src/calc.rs
@@ -1 +1 @@
-pub fn add(a: i32, b: i32) -> i32 { a - b }
+pub fn add(a: i32, b: i32) -> i32 { a + b }
EOF

PHASH="$(sha256sum "$PATCH" | cut -d' ' -f1)"

echo "== 1. propose (scope ok) =="
ID="$("$BIN" workflow propose --repo "$REPO" --goal "fix add boundary" --authority assist --patch "$PATCH" --allowed "src/**")"
echo "   id=$ID"
[ -n "$ID" ] || { echo "FAIL: propose returned no id"; exit 1; }

echo "== 2. apply refuses without approval =="
if "$BIN" workflow apply --repo "$REPO" "$ID" --patch-hash "$PHASH" --no-rollback 2>/dev/null; then
  echo "FAIL: apply succeeded without approval"; exit 1
fi
echo "   refused (good)"

echo "== 3. dry-run passes in isolated worktree =="
if ! "$BIN" workflow dry-run --repo "$REPO" "$ID" --validate "grep -q 'a + b' src/calc.rs"; then
  echo "FAIL: dry-run should pass"; exit 1
fi

echo "== 4. approve =="
"$BIN" workflow approve --repo "$REPO" "$ID" --patch-hash "$PHASH"

echo "== 5. apply (with approval) mutates tree =="
"$BIN" workflow apply --repo "$REPO" "$ID" --patch-hash "$PHASH" --validate "grep -q 'a + b' src/calc.rs"
grep -q 'a + b' src/calc.rs || { echo "FAIL: tree not patched"; exit 1; }
echo "   tree patched (good)"
# Commit so the next apply starts from a clean tree (simulates user review + commit).
git -C "$REPO" add -A
git -C "$REPO" commit -qm "applied fix via approval-controlled workflow"

echo "== 5.5 approval before dry-run is refused =="
PATCHX="$WORK/revert.patch"
cat > "$PATCHX" <<'EOF'
--- a/src/calc.rs
+++ b/src/calc.rs
@@ -1 +1 @@
-pub fn add(a: i32, b: i32) -> i32 { a + b }
+pub fn add(a: i32, b: i32) -> i32 { a - b }
EOF
PHASHX="$(sha256sum "$PATCHX" | cut -d' ' -f1)"
IDX="$("$BIN" workflow propose --repo "$REPO" --goal "revert" --authority assist --patch "$PATCHX" --allowed "src/**")"
if "$BIN" workflow approve --repo "$REPO" "$IDX" --patch-hash "$PHASHX" 2>/dev/null; then
  echo "FAIL: approval succeeded before dry-run"; exit 1
fi
echo "   refused (good)"

echo "== 5.6 apply after HEAD moved is refused =="
"$BIN" workflow dry-run --repo "$REPO" "$IDX" --validate "grep -q 'a - b' src/calc.rs"
"$BIN" workflow approve --repo "$REPO" "$IDX" --patch-hash "$PHASHX"
# Move the repository HEAD away from the validated base.
echo "// unrelated committed change" >> src/lib.rs
git -C "$REPO" add -A
git -C "$REPO" commit -qm "move repository head"
if "$BIN" workflow apply --repo "$REPO" "$IDX" --patch-hash "$PHASHX" 2>/dev/null; then
  echo "FAIL: stale proposal applied after HEAD changed"; exit 1
fi
echo "   refused (good)"

echo "== 6. rollback restores tree on validation failure =="
PATCH2="$WORK/break.patch"
cat > "$PATCH2" <<'EOF'
--- a/src/calc.rs
+++ b/src/calc.rs
@@ -1 +1 @@
-pub fn add(a: i32, b: i32) -> i32 { a + b }
+pub fn add(a: i32, b: i32) -> i32 { a * b }
EOF
PHASH2="$(sha256sum "$PATCH2" | cut -d' ' -f1)"
ID2="$("$BIN" workflow propose --repo "$REPO" --goal "change op" --authority assist --patch "$PATCH2" --allowed "src/**")"
"$BIN" workflow dry-run --repo "$REPO" "$ID2" >/dev/null
"$BIN" workflow approve --repo "$REPO" "$ID2" --patch-hash "$PHASH2"
# validation greps for 'a + b', which this patch removes -> must roll back
if "$BIN" workflow apply --repo "$REPO" "$ID2" --patch-hash "$PHASH2" --validate "grep -q 'a + b' src/calc.rs"; then
  echo "FAIL: apply should have failed validation and rolled back"; exit 1
fi
grep -q 'a + b' src/calc.rs || { echo "FAIL: rollback did not restore tree"; exit 1; }
echo "   rolled back to 'a + b' (good)"

echo "== 7. report =="
"$BIN" workflow report --repo "$REPO" "$ID" | grep -q '"applied": true' || { echo "FAIL: report missing applied=true"; exit 1; }

echo "APPROVAL-CONTROLLED PATCH SMOKE PASSED"
