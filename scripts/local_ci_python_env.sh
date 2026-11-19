#!/usr/bin/env bash
# Trusted preflight for the local CI Python environment.
#
# Creates an ISOLATED venv under the gitignored `.local-ci/` directory and
# installs the hash-pinned toolchain into it with `--require-hashes`.
# It never touches the system interpreter, never runs implicitly from any
# other script, and never fetches anything beyond the pinned wheel hashes
# recorded in `requirements-local-ci.txt`.
#
# Exit status: 0 on success; non-zero otherwise. Nothing is installed when
# any hash fails to match.
#
# Usage:
#   scripts/local_ci_python_env.sh
#
# Then run the core suite with the venv interpreter explicitly, e.g. on Windows:
#   .local-ci/python-env/Scripts/python.exe scripts/local_ci.py run --suite core

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENV="$REPO_ROOT/.local-ci/python-env"

python -m venv "$VENV"

if [ -x "$VENV/Scripts/python.exe" ]; then
    PY="$VENV/Scripts/python.exe"
else
    PY="$VENV/bin/python"
fi

"$PY" -m pip install --require-hashes -r "$REPO_ROOT/requirements-local-ci.txt"

echo "Local CI Python environment ready at $VENV"
