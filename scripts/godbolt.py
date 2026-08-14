#!/usr/bin/env python3
"""Compiler Explorer (godbolt.org) oracle client for LCCC.

LCCC's competitive targets are GCC, Clang, ICC and ICX (project goal §1), but
ICC and ICX cannot be installed in most development or CI environments. This
client fetches their code generation over the public Compiler Explorer API so
those compilers can serve as *reference oracles* for both correctness spot
checks and codegen comparisons, without a local Intel toolchain.

Two modes:

  asm      Print one compiler's assembly for a source file.
  compare  Compile the same source with several compilers and report, per
           function, the instruction count and a normalized instruction-mix
           summary — the quantitative form used in the competition matrix.

Examples
--------
    # ICX's code for a kernel
    scripts/godbolt.py asm --compiler icx --flags "-O3 -march=raptorlake" k.c

    # Head-to-head against the reference set
    scripts/godbolt.py compare k.c --flags "-O3 -march=x86-64-v3"

    # Compare LCCC's own output against the reference set
    scripts/godbolt.py compare k.c --local ./target/release/lccc

Compiler selection accepts either a short alias (`gcc`, `clang`, `icc`, `icx`)
which resolves to the newest x86-64 build Compiler Explorer offers, or an
explicit Compiler Explorer id (`cg162`, `cclang2210`, `cicc2021100`, ...).
Results are cached under `.godbolt-cache/` so repeated comparisons during a
tuning loop do not re-hit the network.

Network access is required for anything but `--list`. Every failure is
reported explicitly; the script never silently substitutes a different
compiler or silently returns partial assembly.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

API = "https://godbolt.org/api"
CACHE = Path(os.environ.get("GODBOLT_CACHE", ".godbolt-cache"))
TIMEOUT = int(os.environ.get("GODBOLT_TIMEOUT", "120"))

# Short aliases -> (name prefix used by Compiler Explorer, id prefix).
# The newest matching entry by semantic version wins, so these keep working
# as Compiler Explorer adds releases.
ALIASES = {
    "gcc": ("x86-64 gcc", "cg"),
    "clang": ("x86-64 clang", "cclang"),
    "icc": ("x86-64 icc", "cicc"),
    "icx": ("x86-64 icx", "cicx"),
}

# Default reference set for `compare`.
DEFAULT_SET = ["gcc", "clang", "icc", "icx"]


def _get_json(url: str):
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        return json.load(resp)


_COMPILER_LIST: list[dict] | None = None


def compiler_list() -> list[dict]:
    global _COMPILER_LIST
    if _COMPILER_LIST is None:
        _COMPILER_LIST = _get_json(f"{API}/compilers/c?fields=id,name,semver")
    return _COMPILER_LIST


def _semver_key(c: dict) -> tuple:
    s = c.get("semver") or ""
    parts = []
    for piece in s.split("."):
        m = re.match(r"\d+", piece)
        parts.append(int(m.group()) if m else 0)
    return tuple(parts) if parts else (0,)


def resolve(spec: str) -> tuple[str, str]:
    """Resolve an alias or explicit id to (compiler_id, display_name)."""
    if spec in ALIASES:
        name_prefix, id_prefix = ALIASES[spec]
        cands = [
            c for c in compiler_list()
            if c["name"].startswith(name_prefix)
            and c["id"].startswith(id_prefix)
            # Exclude nightly/trunk builds: an oracle must be reproducible.
            and "trunk" not in c["id"]
            and "assertions" not in c["id"]
        ]
        if not cands:
            raise SystemExit(f"error: no Compiler Explorer entry for alias {spec!r}")
        best = max(cands, key=_semver_key)
        return best["id"], best["name"]
    for c in compiler_list():
        if c["id"] == spec:
            return c["id"], c["name"]
    raise SystemExit(f"error: unknown compiler id {spec!r} (try --list)")


def compile_remote(compiler_id: str, source: str, flags: str) -> list[str]:
    """Return the assembly lines Compiler Explorer produced, or raise."""
    body = {
        "source": source,
        "options": {
            "userArguments": flags,
            "filters": {
                "binary": False,
                "execute": False,
                # AT&T output matches LCCC's own emission, so diffs are direct.
                "intel": False,
                "demangle": True,
                "directives": True,
                "labels": True,
                "commentOnly": True,
                "trim": True,
            },
            "compilerOptions": {"executorRequest": False},
        },
        "lang": "c",
        "allowStoreCodeDebug": False,
    }
    key = hashlib.sha256(
        f"{compiler_id}\0{flags}\0{source}".encode()
    ).hexdigest()[:32]
    cached = CACHE / f"{key}.json"
    if cached.exists():
        return json.loads(cached.read_text())

    req = urllib.request.Request(
        f"{API}/compiler/{compiler_id}/compile",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Accept": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            data = json.load(resp)
    except urllib.error.URLError as exc:
        raise SystemExit(f"error: Compiler Explorer request failed: {exc}") from exc

    if data.get("code") != 0:
        msg = "\n".join(x.get("text", "") for x in data.get("stderr", []))
        raise SystemExit(
            f"error: {compiler_id} failed to compile:\n{msg.strip()[:2000]}"
        )
    lines = [b.get("text", "") for b in data.get("asm", [])]
    CACHE.mkdir(parents=True, exist_ok=True)
    cached.write_text(json.dumps(lines))
    return lines


def compile_local(binary: str, source_path: Path, flags: str) -> list[str]:
    """Assembly from a locally built compiler (normally LCCC itself)."""
    out = source_path.with_suffix(".local.s")
    cmd = [binary, *flags.split(), "-S", str(source_path), "-o", str(out)]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    if r.returncode != 0:
        raise SystemExit(
            f"error: {binary} failed:\n{(r.stderr or r.stdout).strip()[:2000]}"
        )
    return out.read_text().splitlines()


# ─── Assembly analysis ────────────────────────────────────────────────────

DIRECTIVE = re.compile(r"^\s*\.")
LABEL = re.compile(r"^\s*[.\w$]+:\s*$")

# Coarse instruction classes for the mix summary. The point is not a cycle
# model — it is to make a codegen difference legible at a glance: "ICX used 4
# fewer loads and one more FMA" is actionable, "ICX is 3 instructions shorter"
# is not.
CLASSES: list[tuple[str, re.Pattern]] = [
    ("branch", re.compile(r"^(j\w+|call|ret|loop\w*)$")),
    ("vector", re.compile(r"^v?(p?(add|sub|mul|div|min|max|and|or|xor|cmp|"
                          r"shuf|perm|blend|pack|unpck|broadcast|movdq|movap|"
                          r"movup|insert|extract|gather|fmadd|fmsub|fnmadd|"
                          r"fnmsub|sqrt|rcp|rsqrt|dp|test|abs|sad|madd|avg)"
                          r"\w*)$")),
    ("fp", re.compile(r"^(add|sub|mul|div|sqrt|min|max|ucomi|comi|cvt)"
                      r"(ss|sd|ps|pd)$")),
    ("load", re.compile(r"^(mov\w*|lea)$")),  # refined below by operand shape
    ("alu", re.compile(r"^(add|sub|and|or|xor|not|neg|inc|dec|imul|mul|idiv|"
                       r"div|shl|shr|sar|rol|ror|adc|sbb|cmp|test|set\w+|"
                       r"cmov\w+|bt\w*|bs\w|popcnt|lzcnt|tzcnt|movz\w+|"
                       r"movs\w+|cdq\w*|cqo|bswap|xchg)\w*$")),
    ("stack", re.compile(r"^(push|pop|leave|enter)\w*$")),
    ("nop", re.compile(r"^(nop\w*|pause|endbr\d+)$")),
]


def analyze(lines: list[str], func: str | None = None) -> dict:
    """Instruction count and mix, optionally restricted to one function."""
    in_func = func is None
    depth_ok = True
    counts: dict[str, int] = {}
    total = 0
    mem_ops = 0
    for raw in lines:
        line = raw.split("#")[0].split("//")[0].rstrip()
        if not line.strip():
            continue
        if func is not None:
            if re.match(rf"^\s*{re.escape(func)}:\s*$", line):
                in_func = True
                continue
            if in_func and re.match(r"^\s*\.(size|cfi_endproc)\b", line):
                # `.size f, .-f` ends the function body.
                if ".size" in line and func not in line:
                    continue
                in_func = False
                continue
        if not in_func or not depth_ok:
            continue
        if LABEL.match(line) or DIRECTIVE.match(line):
            continue
        mnemonic = line.strip().split()[0].lower()
        # Strip a lock/rep prefix so the following opcode is classified.
        if mnemonic in ("lock", "rep", "repe", "repz", "repne", "repnz"):
            parts = line.strip().split()
            if len(parts) < 2:
                continue
            mnemonic = parts[1].lower()
        total += 1
        if "(" in line and not mnemonic.startswith("lea"):
            mem_ops += 1
        for name, rx in CLASSES:
            if rx.match(mnemonic):
                counts[name] = counts.get(name, 0) + 1
                break
        else:
            counts["other"] = counts.get("other", 0) + 1
    counts["memory"] = mem_ops
    return {"instructions": total, "mix": counts}


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Compiler Explorer oracle for LCCC codegen comparison.")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p_list = sub.add_parser("list", help="show resolvable compilers")
    p_list.add_argument("--filter", default="")

    p_asm = sub.add_parser("asm", help="print one compiler's assembly")
    p_asm.add_argument("source", type=Path)
    p_asm.add_argument("--compiler", default="gcc")
    p_asm.add_argument("--flags", default="-O2")
    p_asm.add_argument("--func", default=None)

    p_cmp = sub.add_parser("compare", help="compare several compilers")
    p_cmp.add_argument("source", type=Path)
    p_cmp.add_argument("--compilers", default=",".join(DEFAULT_SET))
    p_cmp.add_argument("--flags", default="-O2")
    p_cmp.add_argument("--func", default=None)
    p_cmp.add_argument("--local", default=None,
                       help="path to a locally built compiler (e.g. LCCC)")
    p_cmp.add_argument("--local-name", default="lccc")
    p_cmp.add_argument("--local-flags", default=None,
                       help="flags for --local (defaults to --flags)")

    args = ap.parse_args()

    if args.cmd == "list":
        for c in sorted(compiler_list(), key=lambda c: c["id"]):
            if args.filter and args.filter not in c["id"] \
                    and args.filter.lower() not in c["name"].lower():
                continue
            print(f"{c['id']:<24} {c['name']}")
        for alias in ALIASES:
            cid, name = resolve(alias)
            print(f"{alias:<24} -> {cid} ({name})")
        return 0

    source = args.source.read_text()

    if args.cmd == "asm":
        cid, name = resolve(args.compiler)
        print(f"# {name} [{cid}]  flags: {args.flags}", file=sys.stderr)
        for line in compile_remote(cid, source, args.flags):
            print(line)
        return 0

    # compare
    rows: list[tuple[str, dict]] = []
    if args.local:
        lines = compile_local(args.local, args.source,
                              args.local_flags or args.flags)
        rows.append((args.local_name, analyze(lines, args.func)))
    for spec in [s for s in args.compilers.split(",") if s.strip()]:
        cid, name = resolve(spec.strip())
        lines = compile_remote(cid, source, args.flags)
        rows.append((name, analyze(lines, args.func)))

    keys = ["instructions", "branch", "vector", "fp", "alu", "load",
            "stack", "memory", "nop", "other"]
    width = max(len(n) for n, _ in rows) + 2
    print(f"{'compiler':<{width}}" + "".join(f"{k:>13}" for k in keys))
    print("-" * (width + 13 * len(keys)))
    for name, res in rows:
        cells = []
        for k in keys:
            v = res["instructions"] if k == "instructions" else res["mix"].get(k, 0)
            cells.append(f"{v:>13}")
        print(f"{name:<{width}}" + "".join(cells))

    best = min(rows, key=lambda r: r[1]["instructions"])
    print(f"\nfewest instructions: {best[0]} ({best[1]['instructions']})")
    print("NOTE: instruction count is a proxy, not a verdict. Confirm any "
          "conclusion with a timed run on real hardware (project goal §12).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
