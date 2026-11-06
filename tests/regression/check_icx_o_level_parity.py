#!/usr/bin/env python3
"""Four-compiler parity for numeric ``-O<n>`` and ``-Ofast`` composition.

This is the *measurement* counterpart of ``tests/corpus/test_numeric_o_levels.py``
and ``tests/corpus/test_ofast_composition.py``. Those modules pin GCC 16.2.0 and
Clang 23.1.5 (both available in the sandbox); the authenticated ``icx-oracle``
lane in ``.github/workflows/ci.yml`` is the only place where an ICX 2026.2 binary
exists, so the third comparison oracle is measured here instead of guessed.

Invariants asserted for **every** compiler, including ICX:

* a numeric level >= 3 clamps to that compiler's own ``-O3`` (byte-identical
  assembly, no diagnostic);
* a later plain ``-O*`` withdraws ``-Ofast``'s *codegen* effect, so
  ``-Ofast -O4`` is byte-identical to that compiler's own ``-O4``.

For LCCC, GCC 16.2.0 and Clang 23.1.5 the gate additionally asserts that the
``__FAST_MATH__`` macro is withdrawn too, and that the reduction probe is
non-vacuous. **ICX 2026.2 measured differently on both** (first failing run of
the authenticated lane, 2026-11-06):

* ICX's ``-Ofast`` emits the same assembly as its ``-O3`` on this packed FP
  reduction, so the probe is vacuous *for ICX only*;
* ICX keeps ``__FAST_MATH__`` defined after ``-Ofast -O4`` / ``-O9`` / ``-O2``,
  while GCC 16.2, Clang 23.1.5 and LCCC withdraw it.

ICX 2026.2 is built on an older LLVM than Clang 23.1.5, which explains the
macro divergence. LCCC follows GCC/Clang: a stale ``__FAST_MATH__`` after the
compiler has withdrawn fast-math codegen is a correctness hazard for source
that branches on the macro. These two ICX values are therefore **reported and
pinned as observations**, never asserted as universal invariants -- a future
ICX that adopts the GCC/Clang behaviour must not turn CI red. Malformed
spellings are likewise reported, not asserted: Clang accepts ``-O-1`` and
``-Ofast=3``, GCC wraps ``2^32`` to ``-O0``, LCCC deliberately fails closed.

Locally: ``python3 tests/regression/check_icx_o_level_parity.py`` without
``--icx`` runs GCC 16.2.0 / Clang 23.1.5 / LCCC and reports that ICX was not
available. ``--require-icx`` makes a missing ICX a hard failure; the hosted
lane always passes it.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# A packed FP reduction: strict FP cannot vectorize it, -ffast-math can. This
# is the same fixture the corpus modules use, so a "no codegen delta" result is
# visible instead of being hidden behind an algebraically trivial function.
FIXTURE = (
    "double fp_reduce(const double *p, int n) {\n"
    "    double s = 0.0;\n"
    "    for (int i = 0; i < n; i++) s += p[i];\n"
    "    return s;\n"
    "}\n"
)

LEVELS = ("-O3", "-O4", "-O9", "-O2147483647", "-O0000000000000000000000004")
# Universal semantic invariant: a numeric level above 3 maps to the compiler's
# own highest level. Values near INT_MAX and 25-digit leading-zero spellings are
# *parser limits*, not semantics: different LLVM versions reject/accept them
# differently, so they are measured and reported for every compiler but only
# asserted for LCCC (whose parser this patch owns). Keeps an oracle difference
# from being mistaken for a defect in the design being validated.
CORE_LEVELS = ("-O4", "-O9")
WITHDRAWALS = ("-Ofast -O4", "-Ofast -O9", "-Ofast -O2")
DIVERGENT = ("-O-1", "-Ofast=3", "-O2x", "-O2147483648", "-O4294967296")


def resolve(name: str) -> str | None:
    """Like the corpus ``optional``: an absolute, cwd-independent compiler path."""
    found = shutil.which(name)
    if found:
        return str(Path(found).resolve())
    path = ROOT / name
    return str(path.resolve()) if path.is_file() else None


def run(compiler: str, source: Path, *flags: str) -> subprocess.CompletedProcess:
    out = source.parent / "probe.s"
    out.unlink(missing_ok=True)  # never hash a stale artifact from a failed compile
    words = " ".join(flags).split()
    proc = subprocess.run(
        [compiler, *words, "-S", str(source), "-o", str(out)],
        cwd=source.parent, capture_output=True, text=True, timeout=90,
    )
    digest = hashlib.sha256(out.read_bytes()).hexdigest()[:12] if out.exists() else ""
    return subprocess.CompletedProcess(proc.args, proc.returncode, digest, proc.stderr)


def macro(compiler: str, source: Path, *flags: str) -> int:
    words = " ".join(flags).split()
    proc = subprocess.run(
        [compiler, *words, "-dM", "-E", str(source)],
        cwd=source.parent, capture_output=True, text=True, timeout=90,
    )
    return int(proc.returncode == 0 and "__FAST_MATH__" in proc.stdout)


def measure(compiler: str, source: Path) -> dict:
    reference = run(compiler, source, "-O3")
    ofast = run(compiler, source, "-Ofast")
    return {
        "o3": reference.stdout,
        "o3_rc": reference.returncode,
        "ofast": ofast.stdout,
        "ofast_macro": macro(compiler, source, "-Ofast"),
        "levels": {f: run(compiler, source, f).stdout for f in LEVELS},
        # The reference is the *last* -O spelling alone. `-Ofast -O2` must
        # equal this compiler's -O2, while `-Ofast -O4`/`-O9` equal its own
        # clamped -O3 -- comparing every withdrawal against -O3 is wrong for
        # compilers whose -O2 and -O3 codegen differ (GCC 16.2 does).
        "withdrawals": {
            f: (
                run(compiler, source, f).stdout,
                run(compiler, source, f.split()[-1]).stdout,
                macro(compiler, source, f),
            )
            for f in WITHDRAWALS
        },
        "divergent": {
            f: run(compiler, source, f).returncode for f in DIVERGENT
        },
        "composition": run(compiler, source, "-O3 -ffast-math").stdout,
    }


def check(compilers: dict[str, str], source: Path) -> tuple[list[str], dict]:
    data = {name: measure(path, source) for name, path in compilers.items()}
    problems: list[str] = []
    for name, m in data.items():
        if m["o3_rc"] != 0 or not m["o3"]:
            problems.append(f"{name}: -O3 did not produce assembly")
            continue
        # ICX 2026.2 genuinely does not vectorize this reduction at -Ofast:
        # measured in the authenticated lane. Report it, do not assert it.
        if m["ofast"] == m["o3"] and name != "icx":
            problems.append(f"{name}: probe is vacuous, -Ofast == -O3")
        asserted = LEVELS if name == "lccc" else CORE_LEVELS
        for flag in asserted:
            if m["levels"][flag] != m["o3"]:
                problems.append(f"{name}: {flag} did not clamp to its own -O3")
        for flags, (digest, expected, has_macro) in m["withdrawals"].items():
            if not expected:
                problems.append(f"{name}: {flags} reference did not compile")
            elif digest != expected:
                problems.append(f"{name}: {flags} did not withdraw implicit fast-math")
            # ICX 2026.2 keeps the macro after withdrawal (older LLVM base).
            # GCC 16.2 / Clang 23.1.5 / LCCC withdraw it -- that is the policy
            # this PR implements, and it is asserted for those three only.
            if has_macro and name != "icx":
                problems.append(f"{name}: {flags} still defines __FAST_MATH__")
        if m["ofast"] != m["composition"]:
            # Reported, not asserted: Clang/ICX may own extra -Ofast defaults.
            pass
    return problems, data


def report(data: dict) -> None:
    print("compiler  -O3        -Ofast      macro  Ofast==composed  withdrawn")
    for name, m in data.items():
        print(
            f"{name:<9} {m['o3']:<11} {m['ofast']:<11} "
            f"{m['ofast_macro']:<6} {str(m['ofast'] == m['composition']):<16} "
            + ",".join(
                str(int(d == expected)) for d, expected, _ in m["withdrawals"].values()
            )
        )
    print("\nNumeric levels equal to each compiler's own -O3:")
    print("compiler  " + "  ".join(f"{f:<28}" for f in LEVELS))
    for name, m in data.items():
        print(f"{name:<9} "
              + "  ".join(f"{str(m['levels'][f] == m['o3']):<28}" for f in LEVELS))
    if "icx" in data:
        icx = data["icx"]
        macros_after = ",".join(
            str(mac) for _d, _e, mac in icx["withdrawals"].values()
        )
        print(
            "\nICX 2026.2 measured divergence: -Ofast==-O3 on this fixture = "
            f"{str(icx['ofast'] == icx['o3'])}, __FAST_MATH__ after "
            f"-Ofast -O4/-O9/-O2 = [{macros_after}] "
            "(GCC 16.2 / Clang 23.1.5 / LCCC = [0,0,0]; LCCC follows them "
            "because a stale fast-math macro after withdrawal is unsafe)."
        )
    print("\nMalformed spellings (exit status; 0 = accepted):")
    print("compiler  " + "  ".join(f"{f:<16}" for f in DIVERGENT))
    for name, m in data.items():
        print(f"{name:<9} " + "  ".join(f"{m['divergent'][f]:<16}" for f in DIVERGENT))


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", default="target/fastbuild/lccc")
    ap.add_argument("--gcc", default="gcc-16")
    ap.add_argument("--clang", default="clang-23")
    ap.add_argument("--icx", default=None)
    ap.add_argument("--require-icx", action="store_true")
    ap.add_argument("--json", default=None)
    args = ap.parse_args(argv)

    compilers: dict[str, str] = {}
    for name, value in (("lccc", args.lccc), ("gcc", args.gcc),
                        ("clang", args.clang), ("icx", args.icx)):
        if value is None:
            continue
        path = resolve(value)
        if path is None:
            if name == "icx" and not args.require_icx:
                continue
            print(f"FAIL: {name} compiler {value!r} not found", file=sys.stderr)
            return 2
        compilers[name] = path
    if args.require_icx and "icx" not in compilers:
        print("FAIL: --require-icx was given but no ICX binary was resolved",
              file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory(prefix="lccc-icx-o-levels-") as tmp:
        source = Path(tmp) / "reduce.c"
        source.write_text(FIXTURE, encoding="utf-8")
        problems, data = check(compilers, source)
    report(data)
    if args.json:
        Path(args.json).write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    if "icx" not in compilers:
        print("ICX 2026.2 not available on this host: authenticated icx-oracle "
              "lane supplies it (--require-icx there).")
    for problem in problems:
        print(f"FAIL: {problem}", file=sys.stderr)
    print(f"o-level-parity: {len(compilers)} compiler(s), "
          f"{len(problems)} problem(s)")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
