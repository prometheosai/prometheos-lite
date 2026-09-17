#!/usr/bin/env python3
"""Cross-platform source-policy checks formerly embedded in hosted CI."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


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
    ROOT / "src" / "api",
    ("LlmClient::generate", "llm_client.generate"),
    set(),
)
if violations:
    raise SystemExit("repository policy violations:\n" + "\n".join(violations))
print("PASS: fallback confinement and API LLM boundary")
