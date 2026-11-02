"""Validate the committed harness capability catalog against its JSON Schema.

This is a core-suite check (see ``scripts/local_ci.py`` SUITE_SPEC). It is
fail-closed in both directions:

* the committed fixture must validate against
  ``schemas/HarnessCatalog.schema.json``; and
* deliberately malformed instances (malformed JSON, missing required
  fields, wrong types, invalid version/digest patterns) must be REJECTED.
  A negative case that unexpectedly validates is a failure, not a pass.

Exit status: 0 only when every assertion holds; 1 otherwise.
"""

from __future__ import annotations

import json
import os
import sys
import tempfile

SCHEMA_PATH = os.path.join("schemas", "HarnessCatalog.schema.json")
FIXTURE_PATH = os.path.join("tests", "fixtures", "harness_catalog_canonical.json")


def _fail(message: str) -> int:
    print(f"FAIL: {message}")
    return 1


def _load(path: str, *, binary: bytes | None = None):
    if binary is not None:
        return json.loads(binary.decode("utf-8"))
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def main() -> int:
    try:
        from jsonschema import ValidationError, validate
    except ImportError:
        return _fail("jsonschema dependency is not installed; cannot validate (fail-closed)")

    try:
        schema = _load(SCHEMA_PATH)
    except Exception as error:  # noqa: BLE001 - fail closed on any load problem
        return _fail(f"cannot load schema {SCHEMA_PATH}: {error}")

    try:
        catalog = _load(FIXTURE_PATH)
    except Exception as error:  # noqa: BLE001 - fail closed on any load problem
        return _fail(f"cannot load fixture {FIXTURE_PATH}: {error}")

    # --- Positive case: the committed artifact must validate. ---------------
    try:
        validate(instance=catalog, schema=schema)
    except ValidationError as error:
        return _fail(f"committed catalog does not validate against schema: {error.message}")
    print("PASS: committed harness catalog validates against HarnessCatalog.schema.json")

    # --- Negative cases: malformed instances MUST be rejected. --------------
    def expect_rejected(label: str, instance) -> bool:
        try:
            validate(instance=instance, schema=schema)
        except ValidationError as error:
            print(f"PASS: {label} rejected ({error.message})")
            return True
        _fail(f"{label} unexpectedly validated; schema is not fail-closed")
        return False

    missing_field = _ok = False
    for field in ("schemaVersion", "conformanceVersion", "entries", "contentDigest"):
        dropped = {k: v for k, v in catalog.items() if k != field}
        if expect_rejected(f"missing required field {field!r}", dropped):
            missing_field = True
        else:
            missing_field = False
            break
    if not missing_field:
        return 1

    wrong_type = dict(catalog, entries="not-an-array")
    if not expect_rejected("wrong type for 'entries'", wrong_type):
        return 1
    if not expect_rejected("wrong type for 'contentDigest'", dict(catalog, contentDigest=12345)):
        return 1

    if not expect_rejected(
        "invalid schemaVersion pattern", dict(catalog, schemaVersion="1.0")
    ):
        return 1
    if not expect_rejected(
        "invalid conformanceVersion pattern", dict(catalog, conformanceVersion="v1.0.0")
    ):
        return 1
    if not expect_rejected(
        "invalid contentDigest pattern", dict(catalog, contentDigest="z" * 64)
    ):
        return 1

    # Malformed JSON must be rejected by the JSON layer itself.
    with tempfile.TemporaryDirectory() as tmp:
        malformed_path = os.path.join(tmp, "malformed.json")
        with open(malformed_path, "wb") as handle:
            handle.write(b'{"schemaVersion": "1.0.0", "entries": [')
        try:
            _load(malformed_path, binary=open(malformed_path, "rb").read())
        except Exception:  # noqa: BLE001 - rejection is the expected outcome
            print("PASS: malformed JSON rejected (not parseable)")
        else:
            return _fail("malformed JSON unexpectedly parsed; validation is not fail-closed")

    print("PASS: harness catalog schema validation is fail-closed (all negative cases rejected)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
