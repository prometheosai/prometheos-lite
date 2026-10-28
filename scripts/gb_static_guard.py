#!/usr/bin/env python3
"""G-B static guard: MG1 / MG2(orphan CLI) / MG3 authority-surface detectors.

Landed implementation of the reviewed G-B static-guard design:
PROPOSAL-GB-0001 v0.3.3 (design baseline SHA-256
e3be38a92e437b5f9698ce68659f7426c66ad3f8994f34c10f9dabc35547e112),
authorized as a bounded PR-1 implementation candidate by Control-Gate
Response #23. Read-only with respect to the scanned tree. stdlib-only.
Deterministic.

Enforcement record (machine-readable counterpart: scripts/gate_completion_registry.json):
  MG1  usage guards: authority re-exports outside reviewed files, new
        authority re-exports inside reviewed files, authority-namespace
        glob imports, unconfined test modules, #[path]/include! vectors,
        macro-embedded authority tokens, unreviewed authority
        construction/call positions, unreviewed test citations.
  MG2  orphan CLI surface: unmanifested command modules and verbs
        (INITIAL DETECTION ONLY - the exception approval/expiry registry
        remains DESIGNED-ONLY and is NOT enforced by this guard).
  MG3  authority-home public-surface freeze: baseline additions,
        removals, visibility widening and other visibility changes;
        dispatch-signature type and in-home dispatch-trait baselines;
        name-family hits are WARNINGS ONLY (advisory, never a gate).

Reviewed data (fail closed if absent, corrupt, malformed or unpinned):
  scripts/reviewed_authorization_surfaces.json      authority surface definitions
  scripts/authority_pub_surface_baseline.json       authority-home public-surface freeze
  scripts/reviewed_authority_baselines.json         Option-A composite baselines
  scripts/authority_resolver_coverage_manifest.json declared scan scope + coverage limits
  scripts/gate_completion_registry.json             enforced / designed-only gate record

Detector semantics carried over UNCHANGED from the verified prototype:
  R1 conservative cfg model: a module is test-only ONLY when a syntactic
     predicate PROVES it cannot compile outside test builds (a SYNTACTIC
     model, not Rust cfg evaluation).
  R2 relative-path resolution for crate::/self::/super::/bare use paths.
  R3 position rules: parameter, return, tuple-variant and receiver-call
     positions are expression positions.
  R4 nested-module test-confinement inheritance.
  R5 MG2 registry/approval semantics remain DESIGNED-ONLY (not executed).
  R6 orphan CLI surface + new public constructor controls are executable.
  V3-A..V3-C and the v0.3.3 repairs (F-01 structural baseline-entry split,
     F-02 whitespace-separated visibility qualifiers, F-03 whitespace
     canonicalization, F-04 declaration-position attribution, F-05a/b/c
     union / raw identifiers / aliased re-exports, F-13 bidirectional
     removal detection) are implemented exactly as independently verified.

Fail-closed principle: unresolved constructs produce typed findings and
are never treated as safe; a missing, corrupt, malformed or unpinned
reviewed-data file is itself a blocking finding. Warnings never fail the
gate.

TRUST BOUNDARY: this guard DETECTS unreviewed authority-surface change; it
cannot AUTHORIZE change. Digest pinning makes change review-visible; the
human review gate is the trust anchor, and a successful re-pin is not
authorization.
"""
from __future__ import annotations

import contextlib
import hashlib
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# Reviewed data lives beside this module (scripts/); tests may inject an
# alternative directory. Data is loaded STRICTLY: a missing, unreadable or
# malformed reviewed-data file raises BaselineUnavailable, which the guard
# reports as a blocking finding - never as an empty default.
SURFACES_FILE = "reviewed_authorization_surfaces.json"
PUB_SURFACE_FILE = "authority_pub_surface_baseline.json"
COMPOSITE_FILE = "reviewed_authority_baselines.json"
MANIFEST_FILE = "authority_resolver_coverage_manifest.json"
REGISTRY_FILE = "gate_completion_registry.json"
PIN_FILE = "reviewed_config_digests.json"

DATA_FILE_NAMES = (SURFACES_FILE, PUB_SURFACE_FILE, COMPOSITE_FILE,
                   MANIFEST_FILE, REGISTRY_FILE)

_REQUIRED_SURFACE_KEYS = (
    "authority_home_prefixes", "authority_modules", "reviewed_symbols",
    "permitted_sites", "reviewed_reexport_files", "test_citation_files",
    "name_family_pattern",
)
_REQUIRED_COMPOSITE_KEYS = (
    "dispatch_type_allowlist", "reexport_baseline",
    "dispatch_trait_impl_baseline", "cli_surface_baseline",
)


class BaselineUnavailable(Exception):
    """A reviewed-data file is missing, unreadable, or malformed."""


def _require_json(path, expect=None):
    if not os.path.exists(path):
        raise BaselineUnavailable(f"reviewed data file is missing: {os.path.basename(path)}")
    try:
        with open(path, "r", encoding="utf-8") as fh:
            data = json.load(fh)
    except (OSError, ValueError) as exc:
        raise BaselineUnavailable(f"reviewed data file is unreadable: {os.path.basename(path)} ({exc})")
    if expect is not None and not isinstance(data, expect):
        raise BaselineUnavailable(f"reviewed data file is malformed: {os.path.basename(path)}")
    return data


def load_data(data_dir):
    """Load and validate the reviewed data set from data_dir (fail closed)."""
    data_dir = os.path.abspath(data_dir)
    surfaces = _require_json(os.path.join(data_dir, SURFACES_FILE), dict)
    pub_surface = _require_json(os.path.join(data_dir, PUB_SURFACE_FILE), dict)
    composite = _require_json(os.path.join(data_dir, COMPOSITE_FILE), dict)
    _require_json(os.path.join(data_dir, MANIFEST_FILE), dict)
    _require_json(os.path.join(data_dir, REGISTRY_FILE), dict)
    for key in _REQUIRED_SURFACE_KEYS:
        if key not in surfaces:
            raise BaselineUnavailable(f"reviewed surfaces file lacks '{key}'")
    for key in _REQUIRED_COMPOSITE_KEYS:
        if key not in composite:
            raise BaselineUnavailable(f"composite baseline file lacks '{key}'")
    return {
        "AUTHORITY_HOME_PREFIXES": list(surfaces["authority_home_prefixes"]),
        "AUTH_MODULES": set(surfaces["authority_modules"]),
        "REVIEWED_SYMBOLS": {
            name: {"modules": set(entry["modules"]), "methods": set(entry["methods"])}
            for name, entry in surfaces["reviewed_symbols"].items()
        },
        "PERMITTED_SITES": dict(surfaces["permitted_sites"]),
        "REVIEWED_REEXPORT_FILES": list(surfaces["reviewed_reexport_files"]),
        "TEST_CITATION_FILES": {k: set(v) for k, v in surfaces["test_citation_files"].items()},
        "NAME_FAMILY_RE": re.compile(surfaces["name_family_pattern"]),
        "PUB_SURFACE_BASELINE": pub_surface,
        "DISPATCH_TYPE_ALLOWLIST": set(composite["dispatch_type_allowlist"]),
        "CLI_SURFACE_BASELINE": composite["cli_surface_baseline"],
        "REEXPORT_BASELINE": composite["reexport_baseline"],
        "DISPATCH_TRAIT_BASELINE": composite["dispatch_trait_impl_baseline"],
    }


_DATA_KEYS = (
    "AUTHORITY_HOME_PREFIXES", "AUTH_MODULES", "REVIEWED_SYMBOLS",
    "PERMITTED_SITES", "REVIEWED_REEXPORT_FILES", "TEST_CITATION_FILES",
    "NAME_FAMILY_RE", "PUB_SURFACE_BASELINE", "DISPATCH_TYPE_ALLOWLIST",
    "CLI_SURFACE_BASELINE", "REEXPORT_BASELINE", "DISPATCH_TRAIT_BASELINE",
)


@contextlib.contextmanager
def use_data(data_dir):
    """Swap the reviewed data into the module globals, restoring after."""
    loaded = load_data(data_dir)
    saved = {k: globals()[k] for k in _DATA_KEYS}
    try:
        for key, value in loaded.items():
            globals()[key] = value
        yield loaded
    finally:
        for key, value in saved.items():
            globals()[key] = value


# Module-level view of the reviewed data. Inert until use_data() swaps the
# loaded set in; the guard never scans with an unloaded data set (an inert
# default can only fail loud, because an empty surface set makes every
# authority-shaped item an unreviewed addition).
AUTHORITY_HOME_PREFIXES = []
AUTH_MODULES = set()
REVIEWED_SYMBOLS = {}
PERMITTED_SITES = {}
REVIEWED_REEXPORT_FILES = []
TEST_CITATION_FILES = {}
NAME_FAMILY_RE = re.compile(r"^$")
PUB_SURFACE_BASELINE = {}
DISPATCH_TYPE_ALLOWLIST = set()
CLI_SURFACE_BASELINE = {"modules": [], "verbs": []}
REEXPORT_BASELINE = {}
DISPATCH_TRAIT_BASELINE = {}

RAW_PREFIX_RE = re.compile(r'(?:br|r)(#*)"')
IDENT_RE = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\b")
PATH_TAIL_RE = re.compile(r"((?:[A-Za-z_][A-Za-z0-9_]*::)+)$")
PUB_ITEM_RE = re.compile(
    r"\b(pub\s*(?:\([^)]*\))?)\s+"
    r"((?:(?:async|unsafe|extern(?:\s*\"[^\"]*\")?(?:\s*\([^)]*\))?|const)\s+)*)"
    r"(fn|struct|enum|trait|const|static(?:\s+(?:ref|mut))?|union|type|mod)\s+"
    r"((?:r#)?[A-Za-z_][A-Za-z0-9_]*)"
)
VISIBILITY_PREFIX_RE = re.compile(r"\bpub\s*(?:\([^)]*\))?\s*$")


def _canon_vis(vis: str) -> str:
    """Canonicalize a raw visibility fragment (v0.3.3 F-02/F-03):
    'pub', 'pub(crate)', 'pub (crate)', 'pub\\n(crate)', 'pub( crate )' and
    'pub(in a::b::c)' all canonicalize to 'pub', 'pub(crate)' and
    'pub(in a::b::c)' so a baseline entry matches any lexical spelling of the
    same effective visibility."""
    s = vis.strip()
    if not s.startswith("pub"):
        return " ".join(s.split())
    rest = s[3:].strip()
    if rest.startswith("(") and rest.endswith(")"):
        return "pub(" + " ".join(rest[1:-1].split()) + ")"
    return "pub" if not rest else "pub " + " ".join(rest.split())


def _norm_ws(s: str) -> str:
    """Collapse intra-fragment whitespace runs (F-03): 'static   mut' and
    'static\\nmut' both become 'static mut'."""
    return " ".join(s.split())


def parse_visibility_before(text: str, pos: int):
    """Statement-aware visibility parser (V3-C, extended v0.3.3 F-02):
    return the qualifier immediately preceding pos ('pub', 'pub(crate)',
    'pub(super)', 'pub(in a::b::c)') or None. Handles arbitrary path lengths,
    nested parens, attribute/newline/comment separation and WHITESPACE
    BETWEEN `pub` AND ITS PARENTHESIS GROUP (`pub (crate) use ...`, a
    compiler-valid spelling) - structurally, with no fixed-width lookback.
    The returned qualifier is canonical (whitespace-normalized)."""
    i = pos - 1
    while i >= 0 and text[i] in " \t\r\n":
        i -= 1
    if i < 0:
        return None
    if text[i] != ")":
        j = i
        while j >= 0 and (text[j].isalnum() or text[j] == "_"):
            j -= 1
        word = text[j + 1:i + 1]
        return word if word == "pub" else None
    depth, j = 1, i - 1
    while j >= 0 and depth > 0:
        if text[j] == ")":
            depth += 1
        elif text[j] == "(":
            depth -= 1
        j -= 1
    if depth > 0:
        return None
    # F-02: whitespace may separate `pub` from its parenthesis group.
    k = j
    while k >= 0 and text[k] in " \t\r\n":
        k -= 1
    while k >= 0 and (text[k].isalnum() or text[k] == "_"):
        k -= 1
    word = text[k + 1:j + 1].rstrip()
    if word != "pub":
        return None
    return _canon_vis("pub" + text[j + 1:i + 1])

# Types that are never "unreviewed" in dispatch signatures (primitives/std/self).
STDLIKE_TYPES = {
    "Self", "self", "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize",
    "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64", "String", "str",
    "Vec", "Option", "Result", "Box", "Arc", "Rc", "HashMap", "HashSet", "BTreeMap",
    "BTreeSet", "Cow", "Path", "PathBuf", "Duration", "Instant", "Error", "ErrorKind",
    "Value", "Map", "Number", "Uuid", "Url", "PathBuf", "Path",
}


class UnresolvedConstruct(Exception):
    pass




# --------------------------------------------------------------------------
# S1: lexical strip (records string spans with contents)
# --------------------------------------------------------------------------
def strip_noncode(text: str):
    out = []
    spans = []
    stats = {"line_comment": 0, "block_comment": 0, "string": 0,
             "raw_string": 0, "char": 0}
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        two = text[i:i + 2]
        prev_ident = i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_")
        if two == "//":
            j = text.find("\n", i)
            j = n if j == -1 else j
            out.append(" " * (j - i))
            stats["line_comment"] += 1
            i = j
            continue
        if two == "/*":
            depth, j = 1, i + 2
            while j < n and depth > 0:
                if text[j:j + 2] == "/*":
                    depth += 1
                    j += 2
                elif text[j:j + 2] == "*/":
                    depth -= 1
                    j += 2
                else:
                    j += 1
            if depth > 0:
                raise UnresolvedConstruct("unterminated block comment")
            out.append(" " * (j - i))
            stats["block_comment"] += 1
            i = j
            continue
        if c in "rb" and not prev_ident:
            m = RAW_PREFIX_RE.match(text, i)
            if m:
                close = '"' + m.group(1)
                end = text.find(close, m.end())
                if end == -1:
                    raise UnresolvedConstruct("unterminated raw string")
                end += len(close)
                spans.append((i, end, text[m.end():end - len(close)]))
                out.append(" " * (end - i))
                stats["raw_string"] += 1
                i = end
                continue
        if c == "'" and _is_char_literal(text, i):
            j = i + 1
            if text[j] == "\\":
                j += 2
            else:
                j += 1
            j = text.find("'", j)
            if j == -1:
                raise UnresolvedConstruct("unterminated char literal")
            out.append(" " * (j + 1 - i))
            stats["char"] += 1
            i = j + 1
        elif c == '"':
            j = i + 1
            closed = False
            while j < n:
                if text[j] == "\\":
                    j += 2
                elif text[j] == '"':
                    j += 1
                    closed = True
                    break
                else:
                    j += 1
            if not closed:
                raise UnresolvedConstruct("unterminated string")
            spans.append((i, j, text[i + 1:j - 1]))
            out.append(" " * (j - i))
            stats["string"] += 1
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out), stats, spans


def _is_char_literal(text: str, i: int) -> bool:
    if i + 2 >= len(text):
        return False
    if text[i + 1] == "\\":
        return True
    return text[i + 2] == "'"


# --------------------------------------------------------------------------
# R1: conservative cfg model (syntactic; NOT Rust cfg evaluation)
# --------------------------------------------------------------------------
def _balanced_arg(text: str, start: int):
    """Return (content, end_index) for the parenthesized group at start."""
    depth, i = 1, start + 1
    while i < len(text):
        if text[i] == "(":
            depth += 1
        elif text[i] == ")":
            depth -= 1
            if depth == 0:
                return text[start + 1:i], i + 1
        i += 1
    raise UnresolvedConstruct("unbalanced cfg predicate")


def _extract_cfg_predicates(attrs: str):
    preds = []
    for m in re.finditer(r"(?<![A-Za-z_])cfg\s*\(", attrs):
        content, _end = _balanced_arg(attrs, m.end() - 1)
        preds.append(content)
    return preds


def _split_top(s: str):
    """Split at top-level COMMAS tracking PAREN depth (cfg predicates)."""
    parts, depth, cur = [], 0, []
    for ch in s:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    if cur:
        parts.append("".join(cur))
    return parts


def _split_top_braces(s: str):
    """Split at top-level COMMAS tracking BRACE depth (use groups)."""
    parts, depth, cur = [], 0, []
    for ch in s:
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    if cur:
        parts.append("".join(cur))
    return parts


def _split_fn_args(s: str):
    """Split fn-signature parameters at top-level commas, tracking paren,
    angle-bracket and bracket depth (generics like HashMap<A, B> must not
    split)."""
    parts, depth, cur = [], 0, []
    for ch in s:
        if ch in "(<[":
            depth += 1
        elif ch in ")>]":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    if cur:
        parts.append("".join(cur))
    return parts


def _prod_value(pred: str):
    """Conservative 'can this predicate be true in a non-test build?' model.

    leaf `test` -> False (test is false outside test builds);
    any other leaf (feature/unknown) -> True (assume it may hold);
    all -> AND, any -> OR, not -> NOT.
    """
    pred = pred.strip()
    if pred.startswith("all("):
        inner, _ = _balanced_arg(pred, pred.index("("))
        return all(_prod_value(p) for p in _split_top(inner))
    if pred.startswith("any("):
        inner, _ = _balanced_arg(pred, pred.index("("))
        return any(_prod_value(p) for p in _split_top(inner))
    if pred.startswith("not("):
        inner, _ = _balanced_arg(pred, pred.index("("))
        return not _prod_value(inner)
    return pred.strip() != "test"


def attrs_prove_test_only(attrs: str) -> bool:
    """True only when the attributes PROVE the item cannot compile outside
    test builds. Unknown/uncertain predicates do NOT prove it (scan + flag)."""
    if "cfg_attr" in attrs:
        return False  # uncertain: cfg_attr does not gate compilation
    preds = _extract_cfg_predicates(attrs)
    if not preds:
        return False  # no proof available -> scan
    return not any(_prod_value(p) for p in preds)


# --------------------------------------------------------------------------
# R2: module paths and use-path resolution
# --------------------------------------------------------------------------
def current_module(rel: str) -> str:
    p = rel[:-3] if rel.endswith(".rs") else rel
    if p.startswith("src/"):
        p = p[4:]
    parts = [x for x in p.split("/") if x]
    if parts and parts[-1] == "mod":
        parts = parts[:-1]
    return "crate::" + "::".join(parts) if parts else "crate"


def resolve_use_path(rel: str, path: str) -> str:
    path = path.strip()
    if path.startswith("crate::"):
        return path
    if path.startswith("self::"):
        return current_module(rel) + "::" + path[5:]
    if path.startswith("super::"):
        base = current_module(rel)
        rest = path
        while rest.startswith("super::"):
            base = base.rsplit("::", 1)[0]
            rest = rest[7:]
        return base + "::" + rest
    # bare relative path: resolves against the CURRENT module (Rust 2018
    # uniform paths) - a child of the current module.
    return current_module(rel) + "::" + path


def _split_use(stmt: str):
    stmt = stmt.strip()
    m = re.match(r"^(.*?)::\{(.*)\}$", stmt, re.S)
    if m and "{" not in m.group(1):
        base, group = m.group(1).strip(), m.group(2)
        items = []
        for part in _split_top_braces(group):
            part = part.strip()
            if part:
                items.extend(_split_use(base + "::" + part))
        return items
    return [stmt]


def _classify_use_item(item: str, line: int):
    m = re.match(r"^([A-Za-z0-9_:]+)$", item)
    if m:
        return ("path", item, None)
    m = re.match(r"^([A-Za-z0-9_:]+)\s+as\s+([A-Za-z0-9_]+)$", item)
    if m:
        return ("alias", m.group(1), m.group(2))
    m = re.match(r"^([A-Za-z0-9_:]+)::\*$", item)
    if m:
        return ("glob", m.group(1), None)
    raise UnresolvedConstruct("unsupported use item at line %d: %r" % (line, item[:60]))


def parse_imports(stripped: str, rel: str):
    bindings, globs, reexports, aliases = {}, [], [], {}
    for m in re.finditer(r"\buse\b", stripped):
        start = m.start()
        is_pub = parse_visibility_before(stripped, start) is not None
        line = stripped.count("\n", 0, start) + 1
        semi = stripped.find(";", start)
        if semi == -1:
            continue
        stmt = stripped[start + 3:semi]
        if "{" in stmt and "}" not in stmt:
            raise UnresolvedConstruct("unbalanced use group at line %d" % line)
        for item in _split_use(stmt):
            kind, path, alias = _classify_use_item(item, line)
            resolved = resolve_use_path(rel, path)
            if kind == "glob":
                globs.append((resolved, line, path))
            elif kind == "alias":
                bindings[alias] = resolved
                aliases[alias] = resolved
                if is_pub:
                    # F-05c: an aliased re-export still carries its SOURCE
                    # authority symbol; `pub use ...::TrustRegistry as Tr`
                    # must be checked as TrustRegistry, not as Tr.
                    reexports.append((path.split("::")[-1], line))
            else:
                name = path.split("::")[-1]
                bindings[name] = resolved
                if is_pub:
                    reexports.append((name, line))
    return bindings, globs, reexports, aliases


# --------------------------------------------------------------------------
# Mods and attributes
# --------------------------------------------------------------------------
def parse_mods(stripped: str, spans):
    mods = []
    for m in re.finditer(r"\bmod\s+([A-Za-z0-9_]+)", stripped):
        name = m.group(1)
        line = stripped.count("\n", 0, m.start()) + 1
        after = stripped[m.end():m.end() + 200]
        inline = after.lstrip().startswith("{")
        attrs = []
        head = stripped[:m.start()].rstrip()
        while True:
            am = re.search(r"(\#\[[^\[\]]*\])\s*$", head)
            if not am:
                break
            attrs.insert(0, am.group(1))
            head = head[:am.start()].rstrip()
        joined = " ".join(attrs)
        span = None
        if inline:
            bopen = stripped.find("{", m.end())
            span = (m.start(), _find_close(stripped, bopen, "{", "}"))
        mods.append({
            "name": name, "line": line, "inline": inline, "attrs": joined,
            "test_only": attrs_prove_test_only(joined),
            "path_attr_text": _path_attr_for(stripped, spans, m.start()),
            "span": span, "start": m.start(),
        })
    return mods


def _path_attr_for(stripped, spans, mod_pos):
    for s, e, content in spans:
        if s < mod_pos and mod_pos - s < 300:
            if re.search(r"#\[\s*path\s*=\s*$", stripped[max(0, s - 40):s]):
                return content
    return None


def _string_content_after(stripped, spans, pos, window=25):
    best = None
    for s, e, content in spans:
        if pos <= s <= pos + window and (best is None or s < best[0]):
            best = (s, content)
    return best[1] if best else None


def _find_close(s, start, opener, closer):
    depth, i = 1, start + 1
    while i < len(s):
        if s[i] == opener:
            depth += 1
        elif s[i] == closer:
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return len(s)


# --------------------------------------------------------------------------
# Scanning
# --------------------------------------------------------------------------
class F:
    def __init__(self, diag, file, line, detail, severity):
        self.diag, self.file, self.line, self.detail, self.severity = (
            diag, file, line, detail, severity)

    def to_dict(self):
        return {"diagnostic": self.diag, "file": self.file, "line": self.line,
                "detail": self.detail, "severity": self.severity}


def _permitted(rel, name):
    return any(rel == s or rel.startswith(s) for s in PERMITTED_SITES.get(name, []))


def _bound_to_authority(bindings, name):
    full = bindings.get(name)
    if not full:
        return False
    module = full.rsplit("::", 1)[0] if "::" in full else full
    return module in REVIEWED_SYMBOLS[name]["modules"]


def _qualified_path(pre_stmt: str, rel: str, name: str):
    m = PATH_TAIL_RE.search(pre_stmt)
    if not m:
        return False
    raw = m.group(1)[:-2]
    resolved = resolve_use_path(rel, raw)
    # the resolved path IS the module of the symbol occurrence (the symbol
    # token itself was stripped by PATH_TAIL_RE) - no further rsplit.
    return resolved in REVIEWED_SYMBOLS[name]["modules"]


def _authority_types_in(text: str, bindings, rel: str):
    """Reviewed symbols appearing in a type-position text."""
    hits = []
    for s, sym in REVIEWED_SYMBOLS.items():
        if not re.search(r"\b" + s + r"\b", text):
            continue
        if _bound_to_authority(bindings, s):
            hits.append(s)
            continue
        # qualified: crate::auth::Symbol present in the type text
        if any(("crate::" + mod.split("::", 1)[1]) in text for mod in sym["modules"] if "::" in mod):
            hits.append(s)
            continue
    return sorted(set(hits))


def scan_file(rel: str, text: str, known_modules=None):
    rel = rel.replace("\\", "/")
    findings = []
    try:
        stripped, strip_stats, string_spans = strip_noncode(text)
    except UnresolvedConstruct as e:
        return [F("mg1_unresolved_construct", rel, 1,
                  "lexical strip failed (fail-closed): " + str(e), "blocking").to_dict()], \
            {"line_comment": 0, "block_comment": 0, "string": 0, "raw_string": 0, "char": 0}

    if stripped.count("{") != stripped.count("}"):
        return [F("mg1_unresolved_construct", rel, 1,
                  "unbalanced braces after strip; file requires manual review", "blocking").to_dict()], strip_stats

    try:
        bindings, globs, reexports, aliases = parse_imports(stripped, rel)
    except UnresolvedConstruct as e:
        return [F("mg1_unresolved_construct", rel, 1,
                  "use-statement parse failed (fail-closed): " + str(e), "blocking").to_dict()], strip_stats

    in_reviewed_home = any(rel.startswith(p) for p in AUTHORITY_HOME_PREFIXES)
    is_test_file = (os.path.basename(rel) == "tests.rs" or rel.startswith("tests/")
                    or "/tests/" in rel)

    mods = parse_mods(stripped, string_spans)
    test_only_spans = [m["span"] for m in mods if m["test_only"] and m["span"]]

    def in_test_only(pos):
        return any(a <= pos <= b for a, b in test_only_spans)

    # --- S4: aliases / globs / reexports ---------------------------------
    for alias, resolved in aliases.items():
        module = resolved.rsplit("::", 1)[0] if "::" in resolved else resolved
        if module in AUTH_MODULES and not _permitted(rel, alias):
            line = stripped.count("\n", 0, stripped.find(alias)) + 1
            findings.append(F("mg1_authority_alias_outside_site", rel, line,
                              f"alias {alias} -> {resolved} outside permitted site", "blocking"))
    for resolved, gline, raw in globs:
        bare = not raw.startswith(("crate::", "self::", "super::"))
        candidates = {resolved}
        if bare:
            # Rust-2018 uniform path: a bare first segment binds against the
            # current module's items OR the crate root.
            candidates.add("crate::" + raw)
        hit = candidates & AUTH_MODULES
        if hit:
            findings.append(F("mg1_glob_import_of_authority_namespace", rel, gline,
                              f"glob import {raw}::* resolves to authority namespace "
                              f"{sorted(hit)[0]}; cannot be enumerated lexically",
                              "blocking"))
            continue
        if bare:
            # distinguish known-internal from extern-crate globs
            if known_modules is None:
                continue  # single-file scan: authority check above is the conservative gate
            internal_known = resolved in known_modules or ("crate::" + raw) in known_modules
            continue  # extern/non-authority glob: outside this crate's authority surface (R11)
        if resolved.startswith("crate::") and (known_modules is None or resolved not in known_modules):
            findings.append(F("mg1_unresolved_glob_import", rel, gline,
                              f"internal glob import {raw}::* resolves to {resolved}, which is not a known module",
                              "blocking"))
    for name, rline in reexports:
        if name in REVIEWED_SYMBOLS:
            if rel not in REVIEWED_REEXPORT_FILES:
                findings.append(F("mg1_authority_reexport_outside_site", rel, rline,
                                  f"visibility-qualified or plain re-export of {name} outside reviewed re-export files",
                                  "blocking"))
            elif name not in REEXPORT_BASELINE.get(rel, []):
                # a reviewed re-export LOCATION is not perpetual approval of
                # every future symbol placed there (Response #17 item 4)
                findings.append(F("mg1_new_authority_reexport", rel, rline,
                                  f"authority symbol {name} newly re-exported; not in the reviewed "
                                  "re-export baseline for this file", "blocking"))

    # --- mod confinement (R1 + R4) -----------------------------------------
    for mod in mods:
        testish = mod["name"] == "tests" or mod["name"].startswith("test_")
        inherited = in_test_only(mod["start"])
        if not is_test_file and testish and not mod["test_only"] and not inherited:
            findings.append(F("mg1_unconfined_test_module", rel, mod["line"],
                              f"mod {mod['name']} is not provably test-only (attributes: "
                              f"{mod['attrs'] or 'none'}) -> scanned as production code", "blocking"))
        if not is_test_file and mod["path_attr_text"] is not None:
            target = mod["path_attr_text"].lower()
            kind = ("mg1_test_compiled_into_prod" if ("test" in target or testish)
                    else "mg1_include_directive")
            findings.append(F(kind, rel, mod["line"],
                              f"mod {mod['name']} #[path=\"{mod['path_attr_text']}\"] requires reviewed annotation",
                              "blocking"))

    # --- include! (production paths only) ----------------------------------
    if not is_test_file:
        for m in re.finditer(r"\binclude!\s*\(", stripped):
            line = stripped.count("\n", 0, m.start()) + 1
            content = _string_content_after(stripped, string_spans, m.end())
            if content is None:
                findings.append(F("mg1_unresolved_construct", rel, line,
                                  "include! without a resolvable string literal (fail-closed)", "blocking"))
                continue
            kind = "mg1_test_compiled_into_prod" if "test" in content.lower() else "mg1_include_directive"
            findings.append(F(kind, rel, line, f"include!(\"{content}\")", "blocking"))

    # --- S5: macro spans ----------------------------------------------------
    macro_spans = []
    for m in re.finditer(r"\b([A-Za-z_][A-Za-z0-9_]*)!\s*[\(\{\[]", stripped):
        opener = m.group(0)[-1]
        closer = {"(": ")", "{": "}", "[": "]"}[opener]
        macro_spans.append((m.start(), _find_close(stripped, m.end() - 1, opener, closer)))
    for m in re.finditer(r"\bmacro_rules!\s*([A-Za-z0-9_]+)\s*\{", stripped):
        macro_spans.append((m.start(), _find_close(stripped, m.end() - 1, "{", "}")))

    def in_macro(pos):
        return any(a <= pos <= b for a, b in macro_spans)

    for a, b in macro_spans:
        if in_test_only(a):
            continue
        body = stripped[a:b]
        hit = sorted({s for s in REVIEWED_SYMBOLS if re.search(r"\b" + s + r"\b", body)})
        if hit:
            line = stripped.count("\n", 0, a) + 1
            findings.append(F("mg1_macro_unresolved", rel, line,
                              f"authority token(s) {hit} inside macro construct - not statically resolvable; "
                              "requires reviewed annotation", "blocking"))

    # --- S2/S3: expression-position symbol uses (R3) ------------------------
    for m in IDENT_RE.finditer(stripped):
        name = m.group(1)
        if name not in REVIEWED_SYMBOLS or in_test_only(m.start()) or in_macro(m.start()):
            continue
        pre = stripped[max(0, m.start() - 80):m.start()]
        post = stripped[m.end():m.end() + 40]
        if re.search(r"\buse\b\s[^;]*$", pre):
            continue
        if re.search(r"\btype\s+[A-Za-z0-9_]+\s*=[^;]*$", pre):
            continue
        if re.search(r"\bimpl(<[^>]*>)?[^;{]*$", pre):
            continue
        bound = _bound_to_authority(bindings, name)
        pre_stmt = re.split(r"[;{}]", pre)[-1]
        qualified = _qualified_path(pre_stmt, rel, name)
        if not (bound or qualified):
            continue
        expr_position = (
            re.match(r"\s*::", post) is not None
            or re.match(r"\s*\(", post) is not None
            or re.search(r"::<\s*$", pre) is not None
            or re.search(r":\s*$", pre) is not None
        )
        if not expr_position:
            continue
        line = stripped.count("\n", 0, m.start()) + 1
        if is_test_file:
            if name in TEST_CITATION_FILES.get(rel, set()):
                continue
            findings.append(F("mg1_unreviewed_test_citation", rel, line,
                              f"test file cites {name} without a reviewed test-citation entry", "blocking"))
            continue
        if _permitted(rel, name):
            continue
        findings.append(F("mg1_unreviewed_authority_call", rel, line,
                          f"{name} used at expression position outside permitted site", "blocking"))

    # --- R3: signature positions (parameter / return / tuple-struct) ---------
    for m in re.finditer(r"\bfn\s+([A-Za-z0-9_]+)\s*(?:<[^>{}]*>)?\s*\(", stripped):
        if in_test_only(m.start()) or in_macro(m.start()):
            continue
        open_paren = stripped.index("(", m.end() - 1)
        close_paren = _find_close(stripped, open_paren, "(", ")")
        params_text = stripped[open_paren + 1:close_paren]
        ret_match = re.match(r"\s*->\s*([^\{;]+)", stripped[close_paren + 1:close_paren + 200])
        ret_text = ret_match.group(1).strip() if ret_match else ""
        authority_params = []
        for part in _split_fn_args(params_text):
            part = part.strip()
            if not part:
                continue
            pm = re.match(r"(?:&\s*(?:mut\s+)?|mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(.+)$", part)
            pname = pm.group(1) if pm else None
            ptype = pm.group(2) if pm else part
            hits = _authority_types_in(ptype, bindings, rel)
            for h in hits:
                if _permitted(rel, h):
                    continue
                line = stripped.count("\n", 0, m.start()) + 1
                findings.append(F("mg1_unreviewed_authority_call", rel, line,
                                  f"{h} used in parameter position of fn {m.group(1)}", "blocking"))
                if pname:
                    authority_params.append((pname, h))
        if ret_text:
            for h in _authority_types_in(ret_text, bindings, rel):
                if _permitted(rel, h):
                    continue
                line = stripped.count("\n", 0, m.start()) + 1
                findings.append(F("mg1_unreviewed_authority_call", rel, line,
                                  f"{h} used in return position of fn {m.group(1)}", "blocking"))
        # receiver-method binding on authority-typed parameters
        for pname, h in authority_params:
            for mm in re.finditer(r"\b" + re.escape(pname) + r"\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(", stripped):
                method = mm.group(1)
                if method in REVIEWED_SYMBOLS[h]["methods"]:
                    line = stripped.count("\n", 0, mm.start()) + 1
                    findings.append(F("mg1_unreviewed_authority_call", rel, line,
                                      f"receiver method {pname}.{method}() on authority type {h}", "blocking"))

    for m in re.finditer(r"\bstruct\s+([A-Za-z0-9_]+)\s*\(", stripped):
        if in_test_only(m.start()) or in_macro(m.start()):
            continue
        open_paren = stripped.index("(", m.end() - 1)
        close_paren = _find_close(stripped, open_paren, "(", ")")
        fields = stripped[open_paren + 1:close_paren]
        for part in _split_fn_args(fields):
            part = part.strip()
            if not part:
                continue
            ptype = part.split(":", 1)[1] if ":" in part else part
            for h in _authority_types_in(ptype, bindings, rel):
                if _permitted(rel, h):
                    continue
                line = stripped.count("\n", 0, m.start()) + 1
                findings.append(F("mg1_unreviewed_authority_call", rel, line,
                                  f"{h} used in tuple-struct field position of struct {m.group(1)}", "blocking"))

    # --- R-item 5: enum tuple-variant authority wrappers --------------------
    if not is_test_file:
        for m in re.finditer(r"\benum\s+([A-Za-z0-9_]+)\s*(?:<[^>{}]*>)?\s*\{", stripped):
            if in_test_only(m.start()) or in_macro(m.start()):
                continue
            bopen = stripped.index("{", m.end() - 1)
            bclose = _find_close(stripped, bopen, "{", "}")
            body = stripped[bopen + 1:bclose]
            for vm in re.finditer(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(([^()]*)\)", body):
                payload = vm.group(2)
                for h in _authority_types_in(payload, bindings, rel):
                    if _permitted(rel, h):
                        continue
                    line = stripped.count("\n", 0, bopen + vm.start()) + 1
                    findings.append(F("mg1_unreviewed_authority_call", rel, line,
                                      f"{h} used in tuple-variant field position of enum "
                                      f"{m.group(1)}::{vm.group(1)}", "blocking"))

    # --- MG3 3b: type aliases / trait impls (binding-aware) -----------------
    if not in_reviewed_home:
        for m in re.finditer(r"\btype\s+([A-Za-z0-9_]+)\s*=\s*([^;]+);", stripped):
            if in_test_only(m.start()) or in_macro(m.start()):
                continue
            hits = _authority_types_in(m.group(2), bindings, rel)
            if hits:
                line = stripped.count("\n", 0, m.start()) + 1
                findings.append(F("mg3_authority_shaped_consumption", rel, line,
                                  f"type {m.group(1)} aliases authority symbol(s) {hits}", "blocking"))
        for m in re.finditer(r"\bimpl(<[^>]*>)?\s*([\s\S]{0,200}?)\s*\{", stripped):
            if in_test_only(m.start()) or in_macro(m.start()):
                continue
            head = m.group(2)
            parts = head.split(" for ", 1)
            trait_part = parts[0] if len(parts) == 2 else None
            type_part = parts[1] if len(parts) == 2 else head
            hits = []
            for part in (trait_part, type_part):
                if part is None:
                    continue
                hits.extend(_authority_types_in(part, bindings, rel))
            if hits:
                line = stripped.count("\n", 0, m.start()) + 1
                findings.append(F("mg3_authority_shaped_consumption", rel, line,
                                  f"authority-shaped impl site {sorted(set(hits))}: {head.strip()[:80]}", "blocking"))

    # --- MG3 3a: public-surface baseline diff + visibility checks ------------
    if in_reviewed_home:
        # baseline entries: "<vis> <kind> <name>" (vis = effective visibility)
        baseline = PUB_SURFACE_BASELINE.get(rel, [])
        # F-01: split entries STRUCTURALLY - visibility may itself contain
        # spaces ("pub(in a::b::c) fn deep"), so a space split is wrong.
        # base_by_ident maps identity -> the baseline ENTRY (not just vis) so
        # a visibility-changed item also marks its entry as seen (F-13).
        base_by_ident = {}
        for b in baseline:
            split = _split_baseline_entry(b)
            if split is not None:
                base_by_ident[split[1]] = b
        seen = set()
        for vis, kind, name, name_pos in _pub_items(stripped):
            ident = f"{kind} {name}"
            full = f"{vis} {ident}"
            seen.add(full)
            # F-04: attribute the finding to the item's DECLARATION position,
            # not to the first occurrence of its name anywhere in the file.
            line = stripped.count("\n", 0, name_pos) + 1
            if full in baseline:
                continue
            if ident in base_by_ident:
                entry = base_by_ident[ident]
                seen.add(entry)
                base_vis = _split_baseline_entry(entry)[0]
                base_is_restricted = base_vis.startswith("pub(")
                cur_is_restricted = vis.startswith("pub(")
                if base_is_restricted and not cur_is_restricted:
                    findings.append(F("mg3_public_visibility_widening", rel, line,
                                      f"visibility widening: '{ident}' baselined as {base_vis} is now {vis}",
                                      "blocking"))
                else:
                    findings.append(F("mg3_public_visibility_changed", rel, line,
                                      f"visibility change: '{ident}' baselined as {base_vis} is now {vis}",
                                      "blocking"))
                continue
            findings.append(F("mg3_public_surface_addition", rel, line,
                              f"public item '{full}' is not in the reviewed public-surface baseline "
                              "(requires authority|ordinary classification)", "blocking"))
        # F-13: removals are reviewed-surface changes too. A baselined public
        # item that disappears shrinks the authority surface; the diff was
        # one-directional before this revision.
        for b in baseline:
            if b not in seen:
                findings.append(F("mg3_public_surface_removal", rel, 1,
                                  f"baselined public item '{b}' is absent from the reviewed file "
                                  "(requires authority|ordinary classification)", "blocking"))
        # V2-F: in-home impl-of-trait for an authority type. A trait impl can
        # change dispatch semantics without any new pub item, so the trait
        # name is baseline-diffed (dispatch-trait baseline).
        for m in re.finditer(r"\bimpl(?:<[^>{}]*>)?\s+([\s\S]{0,200}?)\s*\{", stripped):
            if in_test_only(m.start()) or in_macro(m.start()):
                continue
            head = m.group(1)
            if " for " not in head:
                continue
            trait_part, type_part = head.split(" for ", 1)
            if not _authority_types_in(type_part, bindings, rel):
                continue
            tm = re.search(r"([A-Za-z_][A-Za-z0-9_]*)\s*$", trait_part.strip())
            if not tm:
                continue
            trait_name = tm.group(1)
            if trait_name not in DISPATCH_TRAIT_BASELINE.get(rel, []):
                line = stripped.count("\n", 0, m.start()) + 1
                findings.append(F("mg3_dispatch_trait_impl_addition", rel, line,
                                  f"impl of trait '{trait_name}' for an authority type is not in the "
                                  "reviewed dispatch-trait baseline for this file", "blocking"))

    # --- MG3 3b dispatch-signature half ------------------------------------
    if in_reviewed_home:
        for m in re.finditer(r"\bimpl(?:<[^>{}]*>)?\s+(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\s*\{", stripped):
            impl_type = m.group(1)
            if impl_type not in REVIEWED_SYMBOLS:
                continue
            bopen = stripped.index("{", m.end() - 1)
            bclose = _find_close(stripped, bopen, "{", "}")
            body = stripped[bopen + 1:bclose]
            for fm in re.finditer(r"\bfn\s+([A-Za-z0-9_]+)\s*(?:<[^>{}]*>)?\s*\(", body):
                open_paren = body.index("(", fm.end() - 1)
                close_paren = _find_close(body, open_paren, "(", ")")
                params_text = body[open_paren + 1:close_paren]
                ret_match = re.match(r"\s*->\s*([^\{;]+)", body[close_paren + 1:close_paren + 200])
                types = []
                for part in _split_fn_args(params_text):
                    part = part.strip()
                    if not part:
                        continue
                    ptype = part.split(":", 1)[1] if ":" in part else part
                    types.extend(_type_identifiers(ptype))
                if ret_match:
                    types.extend(_type_identifiers(ret_match.group(1)))
                for tname in sorted(set(types)):
                    if tname in STDLIKE_TYPES or tname in REVIEWED_SYMBOLS:
                        continue
                    if tname in DISPATCH_TYPE_ALLOWLIST:
                        continue
                    line = stripped.count("\n", 0, bopen + fm.start()) + 1
                    findings.append(F("mg3_dispatch_signature_unreviewed_type", rel, line,
                                      f"type '{tname}' appears in the signature of {impl_type}::{fm.group(1)} "
                                      "without a reviewed dispatch-type entry", "blocking"))

    # --- MG2: orphan CLI surface (R6 + V2-G) ---------------------------------
    if rel.startswith("src/cli/commands/"):
        if rel == "src/cli/commands/mod.rs":
            stem = None  # the container module itself, not a command module
        elif rel.endswith("/mod.rs"):
            stem = rel.split("/")[-2]  # directory-form module (V2-G)
        else:
            stem = os.path.basename(rel)[:-3]
        if stem is not None and stem != "mod" and stem not in CLI_SURFACE_BASELINE.get("modules", []):
            findings.append(F("mg2_unmanifested_entry_point", rel, 1,
                              f"CLI command module '{stem}' is not in the reviewed CLI surface baseline",
                              "blocking"))
    if rel.startswith("src/cli/"):
        # clap-derive Subcommand enum variants ARE the verb surface
        for m in re.finditer(r"#\[derive\([^\)]*Subcommand[^\)]*\)\]\s*(?:pub\s+)?enum\s+([A-Za-z0-9_]+)\s*\{",
                             stripped):
            bopen = stripped.index("{", m.end() - 1)
            bclose = _find_close(stripped, bopen, "{", "}")
            body = stripped[bopen + 1:bclose]
            for vm in re.finditer(r"^\s*([A-Z][A-Za-z0-9_]*)\s*[\{\(,]", body, re.M):
                verb = vm.group(1)
                if verb not in CLI_SURFACE_BASELINE.get("verbs", []):
                    line = stripped.count("\n", 0, bopen + vm.start()) + 1
                    findings.append(F("mg2_unmanifested_entry_point", rel, line,
                                      f"CLI verb variant '{verb}' is not in the reviewed CLI surface baseline",
                                      "blocking"))
    for m in re.finditer(r"Command::new\s*\(\s*\"([^\"]+)\"", stripped):
        verb = m.group(1)
        if verb not in CLI_SURFACE_BASELINE.get("verbs", []):
            line = stripped.count("\n", 0, m.start()) + 1
            findings.append(F("mg2_unmanifested_entry_point", rel, line,
                              f"CLI verb '{verb}' is not in the reviewed CLI surface baseline", "blocking"))

    # --- MG3 3c: name-family warnings (non-blocking) -------------------------
    for m in IDENT_RE.finditer(stripped):
        name = m.group(1)
        if name in REVIEWED_SYMBOLS or in_test_only(m.start()) or in_macro(m.start()):
            continue
        if NAME_FAMILY_RE.match(name) and len(name) > 6:
            line = stripped.count("\n", 0, m.start()) + 1
            findings.append(F("mg3_name_family_warning", rel, line,
                              f"name-family heuristic hit: {name} (warning only, never a security guarantee)",
                              "warning"))
    return [f.to_dict() for f in findings], strip_stats


def _split_baseline_entry(entry: str):
    """Structurally split a pub-surface baseline entry '<vis> <ident>' where
    vis may be 'pub', 'pub(crate)', 'pub(super)', 'pub(in a::b::c)'
    (v0.3.3 F-01: visibility can contain spaces, so a space split is wrong).
    Returns (vis, ident) or None when the entry cannot be parsed."""
    if not entry.startswith("pub"):
        return None
    i, n = 3, len(entry)
    if i < n and entry[i] == "(":
        depth = 0
        while i < n:
            if entry[i] == "(":
                depth += 1
            elif entry[i] == ")":
                depth -= 1
                if depth == 0:
                    i += 1
                    break
            i += 1
        else:
            return None
    while i < n and entry[i] in " \t":
        i += 1
    if i >= n:
        return None
    return entry[:i].rstrip(), entry[i:]


def _pub_items(stripped: str):
    """Enumerate ALL public item forms with DISTINCT identities, the
    CANONICAL effective visibility and the declaration position of the item
    name (V3-A/V3-B/V3-C + v0.3.3 F-02/F-03/F-04/F-05a/F-05b):
    plain pub, pub(...)-qualified (arbitrary path lengths AND the
    whitespace-separated spelling `pub (crate)`), keyword combinations
    (async/unsafe/extern/const), static variants (`pub static X`,
    `pub static ref X`, `pub static mut X`, lazy_static-style bodies),
    `pub union` (F-05a) and raw identifiers `pub fn r#name` (F-05b) - with
    real identifiers, never collapsed. Returns (vis, kind, name, name_pos)."""
    items = []
    for m in PUB_ITEM_RE.finditer(stripped):
        vis = _canon_vis(m.group(1))
        mods = _norm_ws(m.group(2))
        kind_full = _norm_ws(mods + " " + m.group(3))
        items.append((vis, kind_full, m.group(4), m.start(4)))
    return items


def _type_identifiers(text: str):
    out = []
    for m in re.finditer(r"\b([A-Z][A-Za-z0-9_]*)\b", text):
        out.append(m.group(1))
    return out


def scan_tree(root):
    findings, files, stats = [], 0, {}
    rs_rels = []
    for dirpath, dirs, files_ in os.walk(root):
        # Scope: repository SOURCE only. Hidden dirs (.git, .cargo-target -
        # vendored cargo build output) and target/node_modules trees are
        # generated artifacts, not product source; scanning them would
        # attribute third-party generated code to this repository
        # (residual R3: generated/build-script code out of scan scope).
        dirs[:] = sorted(d for d in dirs
                         if not d.startswith(".") and d not in ("target", "node_modules"))
        for fn in sorted(files_):
            if fn.endswith(".rs"):
                rs_rels.append(os.path.relpath(os.path.join(dirpath, fn), root).replace("\\", "/"))
    known_modules = {current_module(r) for r in rs_rels}
    for rel in sorted(rs_rels):
        full = os.path.join(root, rel)
        try:
            with open(full, "r", encoding="utf-8", errors="replace") as fh:
                text = fh.read()
        except OSError as e:
            findings.append({"diagnostic": "mg1_unresolved_construct", "file": rel,
                             "line": 0, "detail": f"unreadable: {e}", "severity": "blocking"})
            continue
        files += 1
        fs, st = scan_file(rel, text, known_modules=known_modules)
        for k, v in st.items():
            stats[k] = stats.get(k, 0) + v
        findings.extend(fs)
    return findings, files, stats


def _normalized_digest(path):
    """LF-normalized digest, matching scripts/local_ci_policy.py."""
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read().replace(b"\r\n", b"\n")).hexdigest()


def run_guard(root, data_dir=None):
    """Scan root with the reviewed data from data_dir (default: beside this
    module). Returns (findings, files, stats). A BaselineUnavailable
    condition is reported as a blocking gb_baseline_unavailable finding
    (fail closed), never as an empty scan."""
    if data_dir is None:
        data_dir = HERE
    try:
        with use_data(data_dir):
            findings, files, stats = scan_tree(root)
    except BaselineUnavailable as exc:
        return ([{"diagnostic": "gb_baseline_unavailable", "file": str(data_dir),
                  "line": 0, "detail": str(exc), "severity": "blocking"}], 0, {})
    return findings, files, stats


def pin_violations(root, data_dir=None):
    """Every consumed reviewed-data file must be registered in the curated
    pin set with a matching LF-normalized digest. The pin set is not
    self-pinned, so this coverage check (consumed data IS pinned) is the
    guard's own defense against an unpinned or re-pointed baseline path."""
    if data_dir is None:
        data_dir = HERE
    root = os.path.abspath(root)
    data_dir = os.path.abspath(data_dir)
    violations = []
    pin_path = os.path.join(root, "scripts", PIN_FILE)
    try:
        pins = _require_json(pin_path, dict).get("files", {})
        if not isinstance(pins, dict):
            raise BaselineUnavailable("pin file must contain a 'files' object")
    except BaselineUnavailable as exc:
        return [{"diagnostic": "gb_pin_set_unavailable", "file": pin_path, "line": 0,
                 "detail": str(exc), "severity": "blocking"}]
    for name in DATA_FILE_NAMES:
        pinned_key = "scripts/" + name
        target = os.path.join(data_dir, name)
        if pinned_key not in pins:
            violations.append({
                "diagnostic": "gb_unreviewed_baseline_pin", "file": pinned_key, "line": 0,
                "detail": "consumed reviewed-data file is not registered in the curated pin set "
                          "(every new baseline path must enter the pin set in its landing PR)",
                "severity": "blocking"})
            continue
        try:
            actual = _normalized_digest(target)
        except OSError as exc:
            violations.append({
                "diagnostic": "gb_unreviewed_baseline_pin", "file": pinned_key, "line": 0,
                "detail": f"unreadable ({exc})", "severity": "blocking"})
            continue
        if actual != pins[pinned_key]:
            violations.append({
                "diagnostic": "gb_unreviewed_baseline_pin", "file": pinned_key, "line": 0,
                "detail": f"digest changed without a reviewed pin update "
                          f"(pinned {str(pins[pinned_key])[:12]}, found {actual[:12]})",
                "severity": "blocking"})
    return violations


def policy_violations(root, data_dir=None):
    """(formatted lines, findings, files, stats) for the repository policy
    gate: tree findings plus reviewed-data pin coverage (fail closed)."""
    findings, files, stats = run_guard(root, data_dir)
    lines = [f"G-B guard: {f['diagnostic']} {f['file']}:{f['line']} :: {f['detail']}"
             for f in findings if f["severity"] == "blocking"]
    for v in pin_violations(root, data_dir):
        lines.append(f"G-B guard: {v['diagnostic']} {v['file']} :: {v['detail']}")
    return lines, findings, files, stats


def main():
    if len(sys.argv) < 2:
        print("usage: gb_static_guard.py <tree> [--data DIR] [--json out.json]")
        return 2
    target = sys.argv[1]
    data_dir = os.path.abspath(sys.argv[sys.argv.index("--data") + 1]) if "--data" in sys.argv else HERE
    json_out = sys.argv[sys.argv.index("--json") + 1] if "--json" in sys.argv else None
    lines, findings, files, stats = policy_violations(target, data_dir)
    blocking = [f for f in findings if f["severity"] == "blocking"]
    warnings = [f for f in findings if f["severity"] == "warning"]
    print(f"scanned_files={files} blocking={len(blocking)} warnings={len(warnings)}")
    print(f"strip_stats={json.dumps(stats)}")
    for line in lines:
        print(f"BLOCK {line}")
    for f in warnings[:40]:
        print(f"warning  {f['diagnostic']} {f['file']}:{f['line']} :: {f['detail']}")
    if len(warnings) > 40:
        print(f"... {len(warnings) - 40} more warnings suppressed")
    if json_out:
        with open(json_out, "w", encoding="utf-8") as fh:
            json.dump({"files": files, "stats": stats, "findings": findings,
                       "violations": lines}, fh, indent=2)
    return 1 if lines else 0


if __name__ == "__main__":
    sys.exit(main())
