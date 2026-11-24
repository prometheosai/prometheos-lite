"""Helpers to inspect the workflow through a real YAML 1.2 parser.

Uses the repository's already-installed frontend `js-yaml` dependency rather
than adding a new runtime dependency. No GitHub Actions dispatch is performed.
"""

import hashlib
import json
import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "pr341-windows-acceptance.yml"
RESOLVER_SOURCE = ROOT / "scripts" / "resolve_cargo_test_artifact.py"
BUILD_STEP = "Build focused Windows owner-registry test binary"
RESOLVER_SHA256 = "fa3a0b71bd623f8e1a5d3078f0cb845a0df93a7974fac5cdd755cc2a9247aaa4"

_NODE_SCRIPT = """
const yaml = require(process.argv[1] || 'js-yaml');
const fs = require('fs');
const doc = yaml.load(fs.readFileSync(process.argv[2], 'utf8'));
process.stdout.write(JSON.stringify(doc));
"""


def js_yaml_module():
    """Return an explicit module path, or empty string to use NODE_PATH."""
    configured = os.environ.get("PROMETHEOS_JS_YAML")
    if configured:
        return configured
    candidates = [ROOT / "frontend" / "node_modules" / "js-yaml"]
    parent = ROOT.parent
    for _ in range(4):
        candidates.append(parent / "frontend" / "node_modules" / "js-yaml")
        parent = parent.parent
    for candidate in candidates:
        if candidate.is_dir():
            return str(candidate)
    if os.environ.get("NODE_PATH"):
        return ""
    raise RuntimeError("js-yaml not found; set NODE_PATH or PROMETHEOS_JS_YAML")


def parse_workflow(path=WORKFLOW):
    result = subprocess.run(
        ["node", "-e", _NODE_SCRIPT, js_yaml_module(), str(path)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise ValueError(result.stderr.strip() or "YAML parse failed")
    return json.loads(result.stdout)


def build_step_run(doc, job="windows-owner-registry", step_name=BUILD_STEP):
    steps = doc["jobs"][job]["steps"]
    matches = [step for step in steps if step.get("name") == step_name]
    if len(matches) != 1:
        raise ValueError(f"expected one {step_name!r} step, found {len(matches)}")
    return matches[0]["run"]


def embedded_resolver_source(script):
    start_marker = "$resolverSource = @'\n"
    if start_marker not in script:
        raise ValueError("PowerShell here-string opening is absent from the effective run scalar")
    start = script.index(start_marker) + len(start_marker)
    end = script.index("\n'@\n", start)
    source = script[start:end].replace("\r\n", "\n")
    return source if source.endswith("\n") else source + "\n"


def materialization_script(script):
    """Extract the effective PowerShell materialization code from the run scalar."""
    lines = script.splitlines()
    start = next(i for i, line in enumerate(lines) if "$resolverSource = @'" in line)
    end = next(i for i, line in enumerate(lines) if "WriteAllText($resolver" in line)
    return "\n".join(lines[start:end + 1]) + "\n"


def resolver_sha256(source):
    return hashlib.sha256(source.encode("utf-8")).hexdigest()


def reviewed_resolver_source():
    return RESOLVER_SOURCE.read_text(encoding="utf-8").replace("\r\n", "\n")
