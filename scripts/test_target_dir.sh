#!/usr/bin/env bash
# Path-resolution regressions for the shared smoke-script target-dir logic
# (scripts/lib/target_dir.sh). Runs as the first step of the smoke evidence
# chain so every smoke run proves the resolution handles all path forms:
# empty, relative, POSIX-absolute, Windows drive (forward + backslash), and
# UNC (forward + backslash) — the backslash-UNC form being the case that
# classify-before-normalize incorrectly prefixed with the repository root.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/lib/target_dir.sh"

fail() {
  echo "TARGET-DIR RESOLUTION TEST FAILED: $1"
  exit 1
}

REPO="/repo"

# check DESC INPUT EXPECTED
check() {
  local desc="$1" input="$2" expected="$3" got
  got="$(resolve_target_dir "$input" "$REPO")"
  if [ "$got" != "$expected" ]; then
    fail "$desc: input=[$input] expected=[$expected] got=[$got]"
  fi
  echo "PASS: $desc"
}

check "empty stays empty"              ""                   ""
check "relative anchored to repo"      "sub/target"        "$REPO/sub/target"
check "bare name anchored"             "target"            "$REPO/target"
check "posix absolute passthrough"     "/var/build"        "/var/build"
check "drive forward passthrough"      "D:/build"          "D:/build"
check "drive backslash normalized"     'D:\build'         "D:/build"
check "unc forward passthrough"        "//server/share"    "//server/share"
check "unc backslash not prefixed"     '\\server\share'    "//server/share"
check "unc backslash deep normalized" '\\srv\share\t\bin' "//srv/share/t/bin"

echo "PASS: all target-dir resolution regressions"
