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

if violations:
    raise SystemExit("repository policy violations:\n" + "\n".join(violations))
print("PASS: fallback confinement and API LLM boundary")
print("PASS: hosted-CI independence (no actions, no external CI configs, no LFS)")
