#!/usr/bin/env bash
set -euo pipefail

echo "=== PrometheOS Lite install smoke test ==="
echo ""

echo "--- Step 1: cargo install --path . --force ---"
cargo install --path . --force 2>&1
echo ""

echo "--- Step 2: verify binary exists and version ---"
prometheos --version 2>&1
echo ""

echo "--- Step 3: verify help works ---"
prometheos --help 2>&1 | head -5
echo ""

echo "--- Step 4: first-value create (JSON) ---"
prometheos work create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review \
  --json 2>&1
echo ""

echo "=== Smoke test complete ==="
echo ""
echo "Note: This script installs prometheos-lite system-wide via cargo."
echo "Run individual commands instead if you prefer not to install."
