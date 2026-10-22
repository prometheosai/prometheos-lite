#!/usr/bin/env python3
"""Regenerate the executable-configuration review pins.

Running this script IS the deliberate re-pin act: it re-records the
current digest of every pinned execution surface in
``scripts/reviewed_config_digests.json``.

Workflow for a legitimate change to a pinned surface:

1. Make the change.
2. Run ``python scripts/review_config_digests.py``.
3. Review the diff — the ONLY expected change is the digest for the file
   you deliberately changed.
4. Commit both together.

The policy gate (``scripts/local_ci_policy.py``) fails closed when a
pinned file's bytes differ from its pin, so an unreviewed change to build
configuration cannot reach a merge unnoticed.

To pin a NEW surface, add its path to the ``files`` object in the pin
file before running this script — the set of pinned surfaces is curated,
not auto-discovered.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PIN_FILE = ROOT / "scripts" / "reviewed_config_digests.json"
NOTE = (
    "Digests of executable configuration surfaces (build, bundler, toolchain and "
    "verification-runner configuration). A change to a pinned file fails the "
    "repository policy gate until a human re-pins it deliberately with "
    "scripts/review_config_digests.py. Digests are over LF-normalized bytes so a "
    "platform checkout does not change the verdict."
)


def normalized_digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def main() -> int:
    pins = json.loads(PIN_FILE.read_text(encoding="utf-8"))
    files = pins.get("files", {})
    if not files:
        raise SystemExit("pin file lists no surfaces; add them deliberately before re-pinning")

    missing = [relative for relative in files if not (ROOT / relative).exists()]
    if missing:
        raise SystemExit(
            "pinned surfaces are missing from the working tree:\n"
            + "\n".join(f"  {item}" for item in missing)
        )

    refreshed = {relative: normalized_digest(ROOT / relative) for relative in sorted(files)}
    pins["schemaVersion"] = "1.0.0"
    pins["note"] = NOTE
    pins["files"] = refreshed
    PIN_FILE.write_text(json.dumps(pins, indent=2, sort_keys=False) + "\n", encoding="utf-8")
    print(f"re-pinned {len(refreshed)} executable configuration surfaces")
    for relative, digest in refreshed.items():
        print(f"  {relative} {digest[:12]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
