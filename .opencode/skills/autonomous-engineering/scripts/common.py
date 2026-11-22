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
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any

SHA_RE = re.compile(r"^[0-9a-f]{40}$")
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


@contextlib.contextmanager
def exclusive_lock(path: str | Path, *, timeout: float = 5.0):
    """Atomic, cross-process, cross-platform advisory lock.

    Uses OS byte-range locking when available (msvcrt on Windows, fcntl on
    POSIX) and releases automatically when the process dies. This is an
    advisory lock for cooperative tasks, not a security sandbox.
    """
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
