#!/usr/bin/env python3
"""Shared, dependency-free helpers for the autonomous engineering loop.

Portable across Windows and POSIX. No third-party imports.
"""
from __future__ import annotations

import contextlib
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

SHA_RE = re.compile(r"^[0-9a-f]{40}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
TASK_ID_RE = re.compile(r"^[A-Za-z0-9._-]+$")


class LoopError(Exception):
    """Raised for a guard or validation failure."""


def repo_root(repo: str) -> Path:
    out = subprocess.run(
        ["git", "-C", repo, "rev-parse", "--show-toplevel"],
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    return Path(out)


def git(repo: str, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", repo, *args], check=True, capture_output=True, text=True,
    ).stdout.strip()


def git_ok(repo: str, *args: str) -> bool:
    return subprocess.run(
        ["git", "-C", repo, *args],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    ).returncode == 0


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8", errors="replace")).hexdigest()


def sha256_file(path: str | Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def validate_sha(value: Any, field: str, *, allow_none: bool = False) -> None:
    if value is None and allow_none:
        return
    if not isinstance(value, str) or not SHA_RE.fullmatch(value):
        raise LoopError(f"{field} must be 40 lowercase hex" + (" or null" if allow_none else ""))


def validate_sha256(value: Any, field: str, *, allow_none: bool = False) -> None:
    if value is None and allow_none:
        return
    if not isinstance(value, str) or not SHA256_RE.fullmatch(value):
        raise LoopError(f"{field} must be 64 lowercase hex" + (" or null" if allow_none else ""))


def atomic_json_write(path: str | Path, data: dict[str, Any]) -> Path:
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=target.parent, prefix=".ae-", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as handle:
            json.dump(data, handle, indent=2, sort_keys=True)
            handle.write("\n")
        os.replace(tmp, target)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)
    return target


def read_json(path: str | Path) -> dict[str, Any]:
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise LoopError("JSON root must be an object")
    return data


def worktree_clean(worktree: str) -> bool:
    """Clean means no staged, unstaged or untracked changes in the worktree."""
    return git(worktree, "status", "--porcelain") == ""


def trusted_executable(name: str, forbidden_roots: list[str],
                       expected_sha256: str | None = None) -> str:
    """Resolve an executable, reject untrusted roots, and pin its identity.

    Returns the VERIFIED ABSOLUTE PATH that the caller must launch. When a
    reviewed digest is supplied, the file bytes must match it; a pathname alone
    is never treated as proof of binary identity.
    """
    resolved = shutil.which(name)
    if not resolved:
        raise LoopError(f"executable not found: {name}")
    path = Path(resolved).resolve()
    if not path.is_absolute():
        raise LoopError(f"executable did not resolve to an absolute path: {name}")
    for root in forbidden_roots:
        try:
            if path.is_relative_to(Path(root).resolve()):
                raise LoopError(f"executable resolves inside a controlled directory: {path}")
        except (OSError, ValueError):
            continue
    if expected_sha256 is not None:
        cm_digest = sha256_file(path)
        if cm_digest != expected_sha256:
            raise LoopError(f"executable identity mismatch for {name}: {cm_digest}")
    return str(path)


def _drain(stream: Any, sink: bytearray, cap: int, exceeded: threading.Event) -> None:
    while True:
        chunk = stream.read(4096)
        if not chunk:
            return
        if len(sink) + len(chunk) > cap:
            exceeded.set()
            return
        sink.extend(chunk)


def _terminate(proc: subprocess.Popen) -> str:
    """Terminate a timed-out or over-cap process. Full tree kill is POSIX-only."""
    if os.name == "posix":
        try:
            os.killpg(proc.pid, signal.SIGTERM)
            proc.wait(timeout=5)
            return "process_tree_terminated"
        except (ProcessLookupError, subprocess.TimeoutExpired, OSError):
            proc.kill()
            return "process_killed"
    try:
        proc.terminate()
        proc.wait(timeout=5)
        return "process_only_terminated"
    except (subprocess.TimeoutExpired, OSError):
        proc.kill()
        return "process_killed"


def run_bounded(argv: list[str], cwd: str, timeout: int, max_output: int,
                env: dict[str, str] | None = None) -> dict[str, Any]:
    """Run a command with output bounded WHILE it runs, not after exit."""
    started = time.monotonic()
    options: dict[str, Any] = {"stdout": subprocess.PIPE, "stderr": subprocess.PIPE,
                               "cwd": cwd, "env": env, "bufsize": 0}
    if os.name == "posix":
        options["start_new_session"] = True
    else:
        options["creationflags"] = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
    proc = subprocess.Popen(argv, **options)
    out = bytearray()
    err = bytearray()
    cap = threading.Event()
    threads = [
        threading.Thread(target=_drain, args=(proc.stdout, out, max_output, cap), daemon=True),
        threading.Thread(target=_drain, args=(proc.stderr, err, max_output, cap), daemon=True),
    ]
    for thread in threads:
        thread.start()
    reason = "none"
    deadline = time.monotonic() + timeout
    while proc.poll() is None:
        if cap.is_set():
            reason = "output_cap"
            _terminate(proc)
            break
        if time.monotonic() >= deadline:
            reason = "timeout"
            _terminate(proc)
            break
        time.sleep(0.02)
    if reason == "none" and cap.is_set():
        reason = "output_cap"
        _terminate(proc)
    for thread in threads:
        thread.join(timeout=5)
    duration = round(time.monotonic() - started, 3)
    code = proc.returncode
    return {
        "exit_code": code,
        "stdout": bytes(out).decode("utf-8", "replace"),
        "stderr": bytes(err).decode("utf-8", "replace"),
        "duration_seconds": duration,
        "termination": reason,
        "capped": cap.is_set(),
    }


@contextlib.contextmanager
def exclusive_lock(path: str | Path, *, timeout: float = 5.0):
    """Atomic, cross-process, cross-platform advisory lock (not a sandbox)."""
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(str(target), os.O_CREAT | os.O_RDWR, 0o600)
    try:
        if os.fstat(fd).st_size == 0:
            os.write(fd, b"0")
        os.lseek(fd, 0, os.SEEK_SET)
        deadline = time.monotonic() + timeout
        acquired = False
        try:
            import msvcrt  # type: ignore[import-not-found]

            while not acquired:
                try:
                    msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
                    acquired = True
                except OSError:
                    if time.monotonic() >= deadline:
                        raise LoopError("task is already claimed")
                    time.sleep(0.05)
            try:
                yield
            finally:
                os.lseek(fd, 0, os.SEEK_SET)
                msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)
        except ImportError:
            import fcntl

            while not acquired:
                try:
                    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    acquired = True
                except OSError:
                    if time.monotonic() >= deadline:
                        raise LoopError("task is already claimed")
                    time.sleep(0.05)
            try:
                yield
            finally:
                fcntl.flock(fd, fcntl.LOCK_UN)
    finally:
        os.close(fd)
