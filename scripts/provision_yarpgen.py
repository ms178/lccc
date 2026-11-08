#!/usr/bin/env python3
"""Provision the pinned YARPGen 2.0 binary used by tests/simd/check_yarpgen.py.

Only github.com is contacted (an allowed host in this build environment).
The generator is pinned by commit SHA so every fuzz seed is reproducible and
no "latest" drift can silently change the differential oracle.  CMake is not
required: the upstream CMakeLists.txt builds exactly the fourteen non-test
translation units listed below with plain g++ -std=c++17.

    python3 scripts/provision_yarpgen.py [--dest /tmp/yarpgen] [--force]

The script also (re)writes engineering/toolchains/yarpgen-provenance.json
with the resolved commit, the source-tree hash and the resulting binary hash.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

REPO = "https://github.com/intel/yarpgen.git"
PINNED_COMMIT = "1adb290f453505838f3aa33dc571f292b0c1810f"
# Upstream CMakeLists.txt: all .cpp except the *_test.cpp unit-test drivers.
SOURCES = (
    "context.cpp", "data.cpp", "emit_policy.cpp", "expr.cpp",
    "gen_policy.cpp", "hash.cpp", "ir_value.cpp", "main.cpp",
    "options.cpp", "program.cpp", "statistics.cpp", "stmt.cpp",
    "type.cpp", "utils.cpp",
)
ROOT = Path(__file__).resolve().parent.parent
PROVENANCE = ROOT / "engineering" / "toolchains" / "yarpgen-provenance.json"


def run(cmd: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True,
                          check=True)


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def source_tree_hash(src: Path) -> str:
    """Hash the exact bytes of every translation unit we compiled."""
    h = hashlib.sha256()
    for name in SOURCES + ("enums.h", "options.h", "type.h"):
        p = src / name
        h.update(name.encode())
        h.update(p.read_bytes())
    return h.hexdigest()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dest", type=Path, default=Path("/tmp/yarpgen"))
    ap.add_argument("--force", action="store_true",
                    help="re-clone even if the pinned checkout already exists")
    args = ap.parse_args(argv)

    gxx = shutil.which("g++") or ""
    if not gxx:
        print("provision_yarpgen: g++ not found", file=sys.stderr)
        return 1
    dest: Path = args.dest
    git = dest / ".git"
    if args.force and dest.exists():
        shutil.rmtree(dest)
    if not git.is_dir():
        dest.parent.mkdir(parents=True, exist_ok=True)
        print(f"cloning {REPO} -> {dest}", flush=True)
        run(["git", "clone", "--filter=blob:none", "--no-checkout", REPO,
             str(dest)])
        run(["git", "fetch", "--depth", "1", "origin", PINNED_COMMIT], dest)
        run(["git", "checkout", "--detach", "FETCH_HEAD"], dest)
    head = run(["git", "rev-parse", "HEAD"], dest).stdout.strip()
    if head != PINNED_COMMIT:
        print(f"provision_yarpgen: commit mismatch: {head} != {PINNED_COMMIT}",
              file=sys.stderr)
        return 1

    src = dest / "src"
    binary = dest / "yarpgen"
    cmd = [gxx, "-std=c++17", "-O2", "-Wall", "-Wpedantic", "-Isrc",
           f'-DBUILD_VERSION="intel-main"', '-DBUILD_DATE="2026-11-08"',
           '-DYARPGEN_VERSION_MAJOR="2"', '-DYARPGEN_VERSION_MINOR="0"',
           *[f"src/{n}" for n in SOURCES], "-o", str(binary)]
    print("building yarpgen (14 TUs, -O2)...", flush=True)
    run(cmd, dest)
    version = run([str(binary), "--version"]).stdout.strip()
    if "yarpgen version 2.0" not in version:
        print(f"provision_yarpgen: unexpected version: {version!r}",
              file=sys.stderr)
        return 1

    record = {
        "schema": 1,
        "tool": "YARPGen 2.0 (PLDI'23 loop-oriented generator)",
        "repository": REPO,
        "commit": PINNED_COMMIT,
        "resolved_head": head,
        "build": {
            "compiler": run([gxx, "--version"]).stdout.splitlines()[0],
            "standard": "c++17", "optimization": "-O2",
            "warnings": ["-Wall", "-Wpedantic"],
            "translation_units": list(SOURCES),
            "cmake_required": False,
        },
        "source_tree_sha256": source_tree_hash(src),
        "binary_sha256": sha256(binary),
        "version_string": version,
        "invocation": "yarpgen -s <seed> --std=c --max-array-dims=3 -o <dir>",
        "notes": [
            "Seeded, UB-free C programs; the decimal global hash is the oracle.",
            "--max-array-dims=3 keeps .bss inside the supported small code "
            "model (2 GiB); larger images make 32-bit PC-relative "
            "relocations overflow for GCC, Clang and lccc alike.",
            "The upstream 'v1' branch is the older OOPSLA'20 generator and "
            "is NOT used here.",
        ],
    }
    PROVENANCE.parent.mkdir(parents=True, exist_ok=True)
    PROVENANCE.write_text(json.dumps(record, indent=2) + "\n")
    print(f"OK  {version}")
    print(f"    binary:     {binary} ({binary.stat().st_size} bytes)")
    print(f"    provenance: {PROVENANCE}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
