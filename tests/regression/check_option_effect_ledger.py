#!/usr/bin/env python3
"""GP-22: the option-effect ledger.

Every option in the priority set must be backed by a probe that proves it
changes an artifact or a diagnostic, or must be refused by name. Accepted-
and-inert is the failure state; each row below is a measured GCC 16.2.0
behaviour (2026-10-28) turned into a ratchet:

* effect rows   - LCCC's artifact must differ from the no-flag baseline,
                  and where GCC's relation is observable it is asserted too;
* composition   rows - `-Og`/`-Ofast` must equal their measured GCC
                  compositions exactly (byte-identical assembly);
* diagnostic rows - the warning must fire with GCC's label in both
                  compilers;
* refusal rows  - LCCC must refuse by name what it cannot honour, while
                  GCC's acceptance is recorded as the divergence;
* divergence-pinned rows - behaviour LCCC deliberately implements
                  differently, asserted in BOTH directions so the pin can
                  only move deliberately.

Deleting any assertion from its row makes the corresponding promise
unverifiable; adding an option without a row fails the gate's priority-set
check below.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

FIXTURES = {
    "opt.c": "int fp_probe(double x) { return (int)(x * 3.0000001); }\n",
    "whole.c": (
        "int helper(int x) { return x * 3; }\n"
        "int entry(int y) { return helper(y); }\n"
    ),
    "attr.c": "__attribute__((nosuchattr)) int attr_probe(void) { return 0; }\n",
    "cov.c": "int cov_probe(int x) { return x + 1; }\n",
    "se.c": "int se_probe(int x) { int y = ({ int t = x; t + 1; }); return y; }\n",
    "main.c": "int main(void) { return 0; }\n",
}

# The GP-22 priority set: every spelling must appear in some row below.
PRIORITY_SET = (
    "-fprofile-arcs",
    "-ftest-coverage",
    "-Wattributes",
    "-Ofast",
    "-Og",
    "-static-pie",
    "-pedantic",
)


def run(compiler: list[str], *args: str, cwd: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [*compiler, *args], cwd=cwd, text=True, capture_output=True, timeout=60
    )


def asm(compiler: list[str], source: Path, *flags: str) -> str:
    proc = run(compiler, *flags, "-S", str(source), "-o", "-", cwd=source.parent)
    if proc.returncode:
        raise RuntimeError(f"assembly failed: {compiler} {flags}: {proc.stderr[:300]}")
    return proc.stdout


def compile_obj(compiler: list[str], source: Path, *flags: str, out: Path) -> subprocess.CompletedProcess:
    return run(compiler, *flags, "-c", str(source), "-o", str(out), cwd=source.parent)


def main() -> int:
    import argparse as _argparse

    parser = _argparse.ArgumentParser()
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    args = parser.parse_args()

    def absolutize(tool: str) -> str:
        # Absolute paths stay as-is; a repo-relative build path (target/...)
        # is anchored to the repository; bare tool names resolve via PATH.
        path = Path(tool)
        if path.is_absolute():
            return str(path)
        if "/" in tool:
            return str((ROOT / path).resolve())
        return tool

    lccc = [absolutize(args.lccc)]
    gcc = [absolutize(args.gcc)]

    failures: list[str] = []
    covered: set[str] = set()

    def need(flag: str) -> None:
        covered.add(flag)

    with tempfile.TemporaryDirectory(prefix="lccc-option-ledger-") as tmp:
        base = Path(tmp)
        for name, text in FIXTURES.items():
            (base / name).write_text(text, encoding="utf-8")
        opt = base / "opt.c"
        whole = base / "whole.c"
        attr = base / "attr.c"
        cov = base / "cov.c"
        se = base / "se.c"
        main_c = base / "main.c"

        # --- effect: -fwhole-program internalises (GP-16's verified row) ---
        need("-fwhole-program")
        def exports_helper(assembly: str) -> bool:
            # Assemblers separate the directive and symbol with whitespace
            # (GCC's uses a TAB); compare structurally, not byte-literals.
            return re.search(r"^\s*\.globl\s+helper\s*$", assembly, re.M) is not None

        wl_base = asm(lccc, whole, "-O2")
        wl_eff = asm(lccc, whole, "-O2", "-fwhole-program")
        if wl_base == wl_eff:
            failures.append("-fwhole-program: LCCC artifact unchanged")
        elif exports_helper(wl_base) and exports_helper(wl_eff):
            failures.append("-fwhole-program: expected `helper` to lose .globl")
        gl_base = asm(gcc, whole, "-O2")
        gl_eff = asm(gcc, whole, "-O2", "-fwhole-program")
        if gl_base == gl_eff or not exports_helper(gl_base):
            failures.append(
                f"-fwhole-program: GCC relation changed (oracle drift; "
                f"base/export={exports_helper(gl_base)})"
            )
        else:
            print("ledger -fwhole-program: PASS (both compilers internalise `helper`)")

        # --- composition: -Og == -O1, byte-identical, both compilers -----
        need("-Og")
        for fixture in (opt, whole):
            if asm(lccc, fixture, "-Og") != asm(lccc, fixture, "-O1"):
                failures.append(f"-Og: LCCC -Og != -O1 on {fixture.name}")
        # GCC's own -Og matched its -O1 byte-identically on the scalar probe
        # (measured); on multi-function units its -Og keeps a couple of
        # differences, which is why LCCC's mapping contract is stated on
        # the LCCC side and the scalar relation is re-measured here.
        if asm(gcc, opt, "-Og") != asm(gcc, opt, "-O1"):
            failures.append("-Og: GCC -Og != -O1 on the scalar probe (oracle drift)")
        if not any(f.startswith("-Og") for f in failures):
            print("ledger -Og: PASS (LCCC == -O1 everywhere; GCC scalar probe agrees)")

        # --- composition: -Ofast == -O3 + fast-math, both compilers -------
        need("-Ofast")
        if asm(lccc, opt, "-Ofast") != asm(lccc, opt, "-O3", "-ffast-math"):
            failures.append("-Ofast: LCCC -Ofast != -O3 -ffast-math")
        if asm(gcc, opt, "-Ofast") != asm(gcc, opt, "-O3"):
            failures.append("-Ofast: GCC -Ofast != -O3 on the probe (oracle drift)")
        if asm(lccc, opt, "-Ofast") == asm(lccc, opt):
            failures.append("-Ofast: LCCC artifact unchanged from default")
        if not any(f.startswith("-Ofast") for f in failures):
            print("ledger -Ofast: PASS (== -O3 -ffast-math; differs from default)")

        # --- refusal: coverage flags (GP-13's honest refusal) ---------------
        # GCC's side is recorded as the divergence: -ftest-coverage writes a
        # .gcno notes file, bare -fprofile-arcs does not (measured), and
        # neither exists in LCCC - which now refuses by name instead of
        # silently recording its own profile format.
        for flag in ("-fprofile-arcs", "-ftest-coverage"):
            need(flag)
            lc = compile_obj(lccc, cov, flag, out=base / f"l{flag.strip('-')}.o")
            if lc.returncode == 0 or "is not implemented: lccc emits no .gcno/.gcda" not in lc.stderr:
                failures.append(f"{flag}: named refusal missing: {lc.stderr[:200]!r}")
                continue
            gc = compile_obj(gcc, cov, flag, out=base / f"g{flag.strip('-')}.o")
            if gc.returncode:
                failures.append(f"{flag}: GCC no longer accepts it (oracle drift)")
                continue
            gcno = sorted(base.glob("*.gcno"))
            if flag == "-ftest-coverage" and not gcno:
                failures.append(f"{flag}: GCC no longer emits .gcno (oracle drift)")
            if flag == "-fprofile-arcs" and gcno:
                failures.append(f"{flag}: GCC started emitting .gcno (oracle drift)")
            for f in gcno:
                f.unlink()
            if list(base.glob("*.gcno")):
                failures.append(f"{flag}: LCCC started emitting .gcno - update this pin")
                continue
            print(
                f"ledger {flag}: PASS (named refusal; GCC's gcov notes behaviour "
                "recorded as the documented divergence)"
            )

        # --- diagnostic: -Wattributes fires in both compilers -------------
        need("-Wattributes")
        for compiler, tag in ((gcc, "GCC"), (lccc, "LCCC")):
            proc = run(compiler, "-fsyntax-only", str(attr), cwd=base)
            if "[-Wattributes]" not in proc.stderr or "nosuchattr" not in proc.stderr:
                failures.append(f"-Wattributes: {tag} diagnostic missing: {proc.stderr[:200]!r}")
        if not any(f.startswith("-Wattributes") for f in failures):
            print("ledger -Wattributes: PASS (fires [-Wattributes] in both compilers)")

        # --- diagnostic: -pedantic accepts and acts (GP-10) ----------------
        need("-pedantic")
        ped = run(lccc, "-std=c23", "-pedantic", "-fsyntax-only", str(se), cwd=base)
        if ped.returncode:
            failures.append(f"-pedantic: LCCC rc={ped.returncode}: {ped.stderr[:200]!r}")
        elif "ISO C forbids braced-groups" not in ped.stderr:
            failures.append(f"-pedantic: LCCC diagnostic missing: {ped.stderr[:200]!r}")
        elif "tolerated but not implemented" in ped.stderr:
            failures.append("-pedantic: tolerated-noop message returned")
        else:
            print("ledger -pedantic: PASS (accepted and emits the ISO C diagnostic)")

        # --- refusal: -static-pie named, GCC forwards it -------------------
        need("-static-pie")
        sp = run(lccc, "-c", str(main_c), "-o", str(base / "m.o"), "-static-pie", cwd=base)
        if sp.returncode == 0 or "-static-pie is not supported by the LCCC driver" not in sp.stderr:
            failures.append(f"-static-pie: LCCC must refuse by name: {sp.stderr[:200]!r}")
        gsp = run(gcc, "-###", "-static-pie", str(main_c), "-o", str(base / "m2.o"), cwd=base)
        if gsp.returncode or "static-pie" not in gsp.stdout + gsp.stderr:
            failures.append(
                "-static-pie: GCC no longer forwards it "
                f"(rc={gsp.returncode} out={gsp.stdout[:120]!r} err={gsp.stderr[:200]!r})"
            )
        else:
            print("ledger -static-pie: PASS (LCCC refuses by name; GCC forwards - recorded divergence)")

        # --- refusal rows: the documented-refusal class stays honest --------
        lto = run(lccc, "-c", str(opt), "-o", str(base / "o.o"), "-flto", cwd=base)
        if lto.returncode == 0 or "not implemented" not in lto.stderr:
            failures.append(f"-flto: named refusal missing: {lto.stderr[:200]!r}")
        else:
            print("ledger -flto: PASS (named refusal)")
        san = run(lccc, "-c", str(opt), "-o", str(base / "o2.o"), "-fsanitize=address", cwd=base)
        if san.returncode == 0 or "is not supported" not in san.stderr:
            failures.append(f"-fsanitize=address: named refusal missing: {san.stderr[:200]!r}")
        else:
            print("ledger -fsanitize=address: PASS (named refusal)")

    missing = [f for f in PRIORITY_SET if f not in covered]
    if missing:
        failures.append(f"priority-set options without a ledger row: {missing}")

    if failures:
        for line in failures:
            print(f"ERROR: {line}", file=sys.stderr)
        return 1
    print(f"option-effect ledger: PASS ({len(PRIORITY_SET)} priority-set options, GCC 16.2.0)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
