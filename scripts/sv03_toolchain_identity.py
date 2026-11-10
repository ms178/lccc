#!/usr/bin/env python3
"""Pinned compiler identity for the SV-03 differential harnesses.

Hosted runners expose the verified Clang 23.1.5 oracle as ``$CLANG`` (an
absolute path), not necessarily as a ``clang-23`` executable on ``PATH``.
The first hosted run exposed exactly this mismatch: a harness that resolved
``tool("clang-23")`` failed while its own ``--clang "$CLANG"`` argument was
verified and unused.  This module is the single boundary that turns an
explicit path or a pinned executable name into a *version-checked* path.

A differential gate MUST fail rather than silently use a runner-image GCC or
Clang of a different major/minor version.  The checks are cheap (one
``-dumpfullversion`` / ``-dumpversion`` / ``--version`` process per oracle)
and the pure predicates are mutation-tested without any toolchain.
"""
from __future__ import annotations

import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from typing import Iterable

EXPECTED_VERSIONS: dict[str, str] = {
    "gcc16": "16.2.0",
    "clang23": "23.1.5",
    "icx2026": "2026.2.0",
}
VERSION_ARGS: dict[str, tuple[str, ...]] = {
    "gcc16": ("-dumpfullversion",),
    "clang23": ("-dumpversion",),
    "icx2026": ("--version",),
}
ICX_VERSION = re.compile(r"\b(2026\.2\.\d+)\b")


class ToolchainIdentityError(RuntimeError):
    """A pinned compiler is missing, not executable, or the wrong version."""


def resolve_executable(value: str, *, search_paths: Iterable[Path] = ()) -> str:
    """Resolve ``value`` without silently falling back to an unpinned name.

    Absolute/containing-slash values are used verbatim (this is what hosted
    ``$CLANG`` needs).  Bare names are looked up in explicit cache prefixes
    first, then on ``PATH``.  No ``clang``/``cc`` fallback exists: a missing
    exact oracle is a hard error.
    """
    if not value:
        raise ToolchainIdentityError("empty compiler path")
    path = Path(value)
    if path.is_absolute() or len(path.parts) > 1:
        if path.is_file() and os.access(path, os.X_OK):
            return str(path)
        raise ToolchainIdentityError(f"explicit compiler {value!r} is not executable")
    for directory in search_paths:
        candidate = Path(directory) / value
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    found = shutil.which(value)
    if found:
        return found
    raise ToolchainIdentityError(f"pinned compiler {value!r} is unavailable")


def extract_version(name: str, text: str) -> str:
    """Extract the comparable version token from a compiler's version output."""
    if name == "icx2026":
        match = ICX_VERSION.search(text)
        return match.group(1) if match else ""
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    return lines[0] if lines else ""


def version_matches(name: str, token: str) -> bool:
    """Pure predicate: exact GCC/Clang release, any 2026.2.x ICX patch."""
    expected = EXPECTED_VERSIONS.get(name)
    if expected is None:
        return False
    if name == "icx2026":
        return bool(re.fullmatch(r"2026\.2\.\d+", token))
    return token == expected


def version_text(name: str, compiler: str) -> str:
    args = VERSION_ARGS.get(name)
    if args is None:
        raise ToolchainIdentityError(f"unknown pinned compiler family {name!r}")
    try:
        result = subprocess.run(
            [compiler, *args],
            capture_output=True,
            text=True,
            errors="replace",
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise ToolchainIdentityError(f"cannot query {name} version: {exc}") from exc
    if result.returncode != 0:
        detail = (result.stdout + result.stderr).strip()[-300:]
        raise ToolchainIdentityError(f"cannot query {name} version: {detail}")
    return result.stdout + result.stderr


def require_version(
    name: str, value: str, *, search_paths: Iterable[Path] = ()
) -> dict[str, str]:
    """Return the resolved, version-checked compiler identity."""
    compiler = resolve_executable(value, search_paths=search_paths)
    token = extract_version(name, version_text(name, compiler))
    if not version_matches(name, token):
        raise ToolchainIdentityError(
            f"{name} must be {EXPECTED_VERSIONS[name]}, got {token!r} from {compiler}"
        )
    return {"name": name, "path": compiler, "version": token}


def require_compiler_versions(
    compilers: dict[str, str], *, search_paths: Iterable[Path] = ()
) -> dict[str, dict[str, str]]:
    """Check every pinned oracle; ``lccc`` and absent optional ICX are skipped."""
    identities: dict[str, dict[str, str]] = {}
    for name, value in compilers.items():
        if name == "lccc" or not value:
            continue
        identities[name] = require_version(name, value, search_paths=search_paths)
    return identities


def self_test() -> list[str]:
    """Mutation-test the pure identity predicates and explicit-path resolver."""
    failures: list[str] = []
    for name, expected in EXPECTED_VERSIONS.items():
        if not version_matches(name, expected):
            failures.append(f"{name} rejected its pinned version")
        wrong = "16.1.0" if name == "gcc16" else "23.0.0" if name == "clang23" else "2025.2.0"
        if version_matches(name, wrong):
            failures.append(f"{name} accepted wrong version {wrong}")
    if version_matches("system-cc", "16.2.0"):
        failures.append("unknown compiler family accepted")
    sample = "Intel(R) oneAPI DPC++/C++ Compiler 2026.2.0 (2026.2.0.20261001)"
    if extract_version("icx2026", sample) != "2026.2.0":
        failures.append("ICX version extraction missed the pinned release")
    if extract_version("icx2026", "Intel(R) oneAPI 2025.2.0") != "":
        failures.append("ICX version extraction accepted a wrong release")
    with tempfile.TemporaryDirectory(prefix="sv03-tool-identity-") as tmp:
        explicit = Path(tmp) / "pinned-clang-23"
        explicit.write_text("#!/bin/sh\nexit 0\n")
        explicit.chmod(0o755)
        try:
            resolved = resolve_executable(str(explicit), search_paths=())
            if resolved != str(explicit):
                failures.append("explicit absolute compiler path was not preserved")
        except ToolchainIdentityError as exc:
            failures.append(f"explicit absolute compiler path rejected: {exc}")
        non_executable = Path(tmp) / "not-executable"
        non_executable.write_text("not a compiler\n")
        try:
            resolve_executable(str(non_executable), search_paths=())
            failures.append("non-executable explicit path accepted")
        except ToolchainIdentityError:
            pass
    try:
        resolve_executable("sv03-definitely-missing-compiler", search_paths=())
        failures.append("missing pinned compiler name accepted")
    except ToolchainIdentityError:
        pass
    return failures
