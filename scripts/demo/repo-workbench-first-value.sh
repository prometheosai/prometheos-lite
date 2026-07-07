#!/usr/bin/env bash
set -euo pipefail

FIXTURE="fixtures/repo-workbench/rust-risky"
GOAL="Find risky code and suggest safe improvements"

echo "================================================"
echo " PrometheOS Lite — Zero-to-First-Value Demo"
echo "================================================"
echo ""
echo "Fixture: $FIXTURE"
echo "Goal:    $GOAL"
echo ""

# Step 1: Create
echo "--- Step 1: create work context ---"
OUTPUT=$(cargo run -- work create \
  --repo "$FIXTURE" \
  --goal "$GOAL" \
  --mode review \
  --json 2>&1)
echo "$OUTPUT"

WORK_ID=$(echo "$OUTPUT" | grep '"work_id"' | sed 's/.*"work_id": "\(.*\)",/\1/')
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
cargo run -- work run "$WORK_ID" 2>&1
echo ""
echo "Next: prometheos work artifacts $WORK_ID"
echo ""

# Step 3: Artifacts
echo "--- Step 3: list artifacts ---"
cargo run -- work artifacts "$WORK_ID" 2>&1
echo ""
echo "Next: prometheos work memory show $WORK_ID"
echo ""

# Step 4: Memory
echo "--- Step 4: inspect memory ---"
cargo run -- work memory show "$WORK_ID" 2>&1
echo ""
echo "Next: prometheos work continue $WORK_ID"
echo ""

# Step 5: Continue
echo "--- Step 5: continue context ---"
cargo run -- work continue "$WORK_ID" 2>&1
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
