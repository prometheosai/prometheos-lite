"""Resolve exactly one Cargo compiler-artifact for the focused lib test target.

Reads Cargo's `--message-format=json` output, selects the single expected
compiler-artifact by identity and test profile, verifies its executable is a
real file on disk, and prints its exact absolute path. Missing, ambiguous,
wrong-identity or malformed input fails closed.

This deliberately does NOT search the filesystem by name or modification time:
the hosted runner used a nonstandard Cargo output layout, so any hardcoded
`debug/deps` assumption is unsafe.
"""

import argparse
import json
import os
import sys

EXPECTED_TARGET_NAME = "prometheos_lite"
EXPECTED_TARGET_KIND = "lib"


def resolve(input_path: str) -> str:
    with open(input_path, encoding="utf-8-sig") as handle:
        raw_lines = handle.read().splitlines()

    candidates = []
    for line_number, line in enumerate(raw_lines, 1):
        if not line.strip():
            continue
        try:
            record = json.loads(line)
        except (ValueError, TypeError) as error:
            raise ValueError(f"malformed Cargo JSON on line {line_number}: {error}") from error
        if not isinstance(record, dict) or record.get("reason") != "compiler-artifact":
            continue
        target = record.get("target")
        profile = record.get("profile")
        if not isinstance(target, dict) or not isinstance(profile, dict):
            continue
        kinds = target.get("kind")
        if (
            target.get("name") == EXPECTED_TARGET_NAME
            and isinstance(kinds, list)
            and EXPECTED_TARGET_KIND in kinds
            and target.get("test") is True
            and profile.get("test") is True
        ):
            candidates.append(record)

    if not candidates:
        raise ValueError("no matching compiler-artifact for the focused lib test target")
    if len(candidates) != 1:
        raise ValueError(f"ambiguous compiler-artifact selection: {len(candidates)} candidates")

    executable = candidates[0].get("executable")
    if not isinstance(executable, str) or not executable:
        raise ValueError("compiler-artifact has no executable")
    absolute = os.path.abspath(executable)
    if not os.path.isfile(absolute):
        raise ValueError(f"compiler-artifact executable is not a real file: {absolute}")
    return absolute


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, help="path to Cargo --message-format=json output")
    args = parser.parse_args()
    try:
        print(resolve(args.input))
    except (OSError, ValueError) as error:
        print(f"cargo artifact resolution failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
