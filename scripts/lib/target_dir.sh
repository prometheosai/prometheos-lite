#!/usr/bin/env bash
# Shared target-directory resolution for the smoke scripts.
#
# Smoke scripts must locate the built prometheos binary even when the build
# is routed to another volume via CARGO_TARGET_DIR. The value can arrive
# in any of these forms and must classify correctly on every host:
#
#   ""                    -> empty (caller falls back to cargo metadata)
#   sub/target            -> relative; anchored to the repository root
#   /var/build            -> POSIX absolute; passes through
#   //server/share        -> UNC (forward separators); passes through
#   \\server\share        -> UNC (backslash separators); must ALSO pass
#                            through — normalizing FIRST makes it match the
#                            absolute-POSIX pattern instead of being
#                            incorrectly prefixed with the repository root
#   D:/build, D:\build   -> Windows drive; passes through (normalized)
#
# This file is sourced, not executed. `resolve_target_dir VALUE REPO_ROOT`
# echoes the resolved path.

# Resolve a CARGO_TARGET_DIR value for bash use. Windows separators are
# normalized FIRST so backslash UNC paths classify as absolute; relative
# values are then anchored to the repository root (cargo treats them as
# relative to its working directory, which is not the script's CWD).
resolve_target_dir() {
  local value="$1" repo_root="$2"
  # Normalize backslashes to forward slashes before classification so
  # `\\server\share` becomes `//server/share` and matches the absolute
  # pattern, and so `[ -x ]` works regardless of MSYS env conversion.
  value="${value//\\//}"
  case "$value" in
    "" ) ;;
    /* ) ;;
    [A-Za-z]:* ) ;;
    *  ) value="$repo_root/$value" ;;
  esac
  printf '%s\n' "$value"
}
