#!/usr/bin/env python3
"""Cross-platform source-policy checks formerly embedded in hosted CI.

Integrity invariants enforced here. Every check fails closed and reports
what it found, so a failure is actionable without reading this file.

1. Fallback confinement / API LLM boundary (unchanged).
2. Hosted-CI and LFS independence — the repository owns its verification.
3. Executable-configuration review pins — a pinned execution surface may
   only change when a human re-pins it deliberately. This is a CHANGE
   REVIEW requirement, not a payload signature: it does not look for any
   particular malicious content, it requires that build configuration
   cannot change without an explicit, reviewed act.
4. Duplicate JSON keys — a document that two parsers read differently is
   a supply-chain hazard, not a style choice.
5. Vendored byte fidelity — upstream-locked vendored bytes are LF-only,
   so a checkout cannot silently rewrite them (CRLF churn is a
   provenance violation).
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Directories that hold generated, vendored, local runtime state, or
# third-party trees which are not part of the repository's own source
# surface.
SKIPPED_TREES = (
    ".cargo-target",
    ".git",
    ".local-ci",
    ".next",
    ".prometheos",
    "node_modules",
    "prometheos-output",
    "target",
)
# Test fixtures intentionally contain malformed and duplicate-key
# documents; they are the SUBJECT of conformance tests, not repository
# configuration. The upstream SOMA bundle ships the same kind of
# deliberately-invalid corpus under fixtures/invalid/.
SKIPPED_PREFIXES = ("tests/fixtures/", "vendored/soma/v1.1/fixtures/invalid/", "vendored/soma/v1.2/fixtures/invalid/")


def matches(root: Path, needles: tuple[str, ...], excluded: set[Path]) -> list[str]:
    findings: list[str] = []
    for path in root.rglob("*.rs"):
        if path in excluded:
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if any(needle in line for needle in needles):
                findings.append(f"{path.relative_to(ROOT)}:{number}:{line.strip()}")
    return findings


violations = matches(
    ROOT / "src",
    ("with_fallback_allowed",),
    {ROOT / "src" / "tools" / "repo.rs"},
)
violations += matches(
    ROOT / "api" if (ROOT / "api").exists() else ROOT / "src" / "api",
    ("LlmClient::generate", "llm_client.generate"),
    set(),
)

# Hosted-CI independence: the repository owns its verification
# (scripts/local_ci.py); no hosted workflow, external CI service, or
# LFS dependence may appear. Fail closed on the first marker found.
hosted_ci_markers: dict[str, Path] = {
    "GitHub Actions (.github/workflows/)": ROOT / ".github" / "workflows",
    "GitLab CI (.gitlab-ci.yml)": ROOT / ".gitlab-ci.yml",
    "Travis CI (.travis.yml)": ROOT / ".travis.yml",
    "CircleCI (.circleci/)": ROOT / ".circleci",
    "Azure Pipelines (azure-pipelines.yml)": ROOT / "azure-pipelines.yml",
    "Jenkins (Jenkinsfile)": ROOT / "Jenkinsfile",
    "Drone CI (.drone.yml)": ROOT / ".drone.yml",
    "AppVeyor (appveyor.yml)": ROOT / "appveyor.yml",
    "AWS CodeBuild (buildspec.yml)": ROOT / "buildspec.yml",
}
hosted = [name for name, path in hosted_ci_markers.items() if path.exists()]
lfs: list[str] = []
if (ROOT / ".lfsconfig").exists():
    lfs.append(".lfsconfig")
gitattributes = ROOT / ".gitattributes"
if gitattributes.exists():
    for number, line in enumerate(gitattributes.read_text(encoding="utf-8").splitlines(), 1):
        if "filter=lfs" in line:
            lfs.append(f".gitattributes:{number} (filter=lfs)")
if hosted or lfs:
    raise SystemExit(
        "repository policy violations (hosted-CI/LFS independence):\n"
        + "\n".join(f"  {marker}" for marker in hosted + lfs)
    )


# ---------------------------------------------------------------------------
# 3. Executable-configuration review pins
# ---------------------------------------------------------------------------
#
# A pinned file is an EXECUTION surface: something a build, a bundler, a
# toolchain, or the verification runner itself will read and execute. Its
# digest is recorded in scripts/reviewed_config_digests.json. A change to
# a pinned file fails the suite until a human updates the pin deliberately
# and reviews the change.
#
# Digests are computed over LF-normalized bytes so that a platform
# checkout (CRLF working tree) does not change the verdict; the vendored
# byte-fidelity check below covers the case where line endings themselves
# are the change.

PIN_FILE = ROOT / "scripts" / "reviewed_config_digests.json"


def normalized_digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


pin_violations: list[str] = []
if not PIN_FILE.exists():
    pin_violations.append(
        f"{PIN_FILE.relative_to(ROOT)}: missing — executable configuration cannot be reviewed without it"
    )
else:
    try:
        pins = json.loads(PIN_FILE.read_text(encoding="utf-8"))
        pinned = pins["files"]
    except (ValueError, KeyError) as exc:
        pin_violations.append(f"{PIN_FILE.relative_to(ROOT)}: unreadable pin file ({exc})")
        pinned = {}
    for relative, expected in sorted(pins.get("files", {}).items()):
        target = ROOT / relative
        if not target.exists():
            pin_violations.append(f"{relative}: pinned executable configuration is missing")
            continue
        actual = normalized_digest(target)
        if actual != expected:
            pin_violations.append(
                f"{relative}: executable configuration changed without a reviewed pin update "
                f"(pinned {expected[:12]}, found {actual[:12]})"
            )

# ---------------------------------------------------------------------------
# 4. Duplicate JSON keys
# ---------------------------------------------------------------------------


def _duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    seen: set[str] = set()
    for key, _ in pairs:
        if key in seen:
            raise ValueError(f"duplicate key {key!r}")
        seen.add(key)
    return dict(pairs)


def repository_json_files() -> list[Path]:
    """The repository's own JSON documents.

    Tracked files are the precise definition: a JSON file that is not
    tracked is not repository content (it is local runtime state, a build
    output, or a third-party tree). When git is unavailable, fall back to
    a filesystem walk with the same exclusions.
    """
    tracked: list[Path] = []
    try:
        import subprocess

        listing = subprocess.run(
            ["git", "-C", str(ROOT), "ls-files", "-z", "--", "*.json"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        for entry in listing.split("\0"):
            if not entry:
                continue
            relative = entry.replace("\\", "/")
            if any(relative.startswith(prefix) for prefix in SKIPPED_PREFIXES):
                continue
            path = ROOT / entry
            if any(part in SKIPPED_TREES for part in path.relative_to(ROOT).parts):
                continue
            tracked.append(path)
        if tracked:
            return sorted(tracked)
    except (OSError, ValueError, subprocess.SubprocessError):
        pass

    found: list[Path] = []
    for path in ROOT.rglob("*.json"):
        relative = path.relative_to(ROOT).as_posix()
        if any(part in SKIPPED_TREES for part in path.relative_to(ROOT).parts):
            continue
        if any(relative.startswith(prefix) for prefix in SKIPPED_PREFIXES):
            continue
        found.append(path)
    return sorted(found)


json_violations: list[str] = []
for json_path in repository_json_files():
    relative = json_path.relative_to(ROOT).as_posix()
    try:
        json.loads(json_path.read_text(encoding="utf-8"), object_pairs_hook=_duplicate_keys)
    except ValueError as exc:
        json_violations.append(f"{relative}: {exc}")
    except (OSError, UnicodeDecodeError) as exc:
        json_violations.append(f"{relative}: unreadable ({exc})")

# ---------------------------------------------------------------------------
# 5. Vendored byte fidelity (LF-only upstream-locked bytes)
# ---------------------------------------------------------------------------

eol_violations: list[str] = []
vendored = ROOT / "vendored"
if vendored.exists():
    for path in sorted(vendored.rglob("*")):
        if not path.is_file():
            continue
        if b"\r" in path.read_bytes():
            eol_violations.append(
                f"{path.relative_to(ROOT).as_posix()}: carries CR bytes — vendored bytes are "
                "upstream-locked (-text) and must not be rewritten"
            )

# ---------------------------------------------------------------------------

if violations:
    raise SystemExit("repository policy violations:\n" + "\n".join(violations))
if pin_violations:
    raise SystemExit(
        "repository policy violations (executable configuration review pins):\n"
        + "\n".join(f"  {item}" for item in pin_violations)
    )
if json_violations:
    raise SystemExit(
        "repository policy violations (duplicate JSON keys):\n"
        + "\n".join(f"  {item}" for item in json_violations)
    )
if eol_violations:
    raise SystemExit(
        "repository policy violations (vendored byte fidelity):\n"
        + "\n".join(f"  {item}" for item in eol_violations)
    )
print("PASS: fallback confinement and API LLM boundary")
print("PASS: hosted-CI independence (no actions, no external CI configs, no LFS)")
print("PASS: executable configuration review pins (build surfaces are reviewed changes)")
print("PASS: no duplicate JSON keys in repository documents")
print("PASS: vendored byte fidelity (LF-only upstream-locked bytes)")
