#!/usr/bin/env python3
"""GP-18: `__has_builtin` differential over the GNU x86 builtin namespace.

GCC 16.2 conditions 110 of the 246 registered `__builtin_ia32_*` names on
a target flag (measured sweep, 2026-10-28). This gate re-runs that sweep
against both compilers — default plus twelve feature flags — and fails on
any answer divergence. The namespace is compiled in chunks because LCCC's
`#warning` diagnostics are capped per translation unit.

Two row kinds:

* answers   — every (name, flag) pair must match GCC exactly;
* macros    — the gating predefined macros (`__SSSE3__` ... `__AVX2__`)
              must be defined by the same flags in both compilers, which
              is what makes the macro table a sound availability proxy.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FLAGS = (
    (),
    ("-msse3",),
    ("-mssse3",),
    ("-msse4.1",),
    ("-msse4.2",),
    ("-mavx",),
    ("-mbmi",),
    ("-mbmi2",),
    ("-mavx2",),
    ("-mfma",),
    ("-mf16c",),
    ("-maes",),
    ("-mpclmul",),
    ("-mavx512f",),
)
GATING_MACROS = (
    "__SSE3__",
    "__SSSE3__",
    "__SSE4_1__",
    "__SSE4_2__",
    "__AVX__",
    "__AVX2__",
    "__BMI__",
    "__BMI2__",
    "__FMA__",
    "__F16C__",
    "__AES__",
    "__PCLMUL__",
    "__AVX512F__",
)
ANSWER = re.compile(r"\b(YES|NO)\s+(\d+)\s+__builtin")
CHUNK = 60


def load_names() -> list[str]:
    text = (ROOT / "src/common/x86_builtin_types.rs").read_text(encoding="utf-8")
    names = sorted(set(re.findall(r'"(__builtin_ia32_[a-z0-9_]+)"', text)))
    if len(names) < 200:
        raise RuntimeError(f"builtin namespace shrank unexpectedly: {len(names)} names")
    return names


def sweep(compiler: list[str], flag_args: tuple[str, ...], names: list[str]) -> dict[str, int]:
    answers: dict[str, int] = {}
    for start in range(0, len(names), CHUNK):
        chunk = names[start : start + CHUNK]
        lines: list[str] = []
        for index, name in enumerate(chunk):
            lines.append(
                f"#if __has_builtin({name})\n#warning YES {index} {name}\n"
                f"#else\n#warning NO {index} {name}\n#endif"
            )
        source = ROOT / "tests" / "regression" / "_has_builtin_sweep.c"
        source.write_text("\n".join(lines) + "\nint has_builtin_sweep;\n", encoding="utf-8")
        proc = subprocess.run(
            [*compiler, *flag_args, "-fsyntax-only", str(source)],
            cwd=ROOT,
            text=True,
            capture_output=True,
            timeout=60,
        )
        found = {
            chunk[int(m.group(2))]: 1 if m.group(1) == "YES" else 0
            for m in ANSWER.finditer(proc.stderr)
            if int(m.group(2)) < len(chunk)
        }
        missing = [n for n in chunk if n not in found]
        if missing:
            raise RuntimeError(
                f"{' '.join(compiler)} lost {len(missing)} answers under "
                f"{' '.join(flag_args) or 'default'} (diagnostic cap?): {missing[:3]}"
            )
        answers.update(found)
    return answers


def defined_macros(compiler: list[str], flag_args: tuple[str, ...]) -> set[str]:
    proc = subprocess.run(
        [*compiler, *flag_args, "-dM", "-E", "-x", "c", str(ROOT / "tests/regression/_has_builtin_sweep.c")],
        cwd=ROOT,
        text=True,
        capture_output=True,
        timeout=60,
    )
    if proc.returncode:
        raise RuntimeError(f"-dM failed for {compiler}: {proc.stderr[:200]}")
    return {line.split()[1] for line in proc.stdout.splitlines() if line.startswith("#define ")} - {""}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    args = parser.parse_args()
    lccc = [args.lccc]
    gcc = [args.gcc]

    names = load_names()
    failures: list[str] = []

    for flag_args in FLAGS:
        label = " ".join(flag_args) or "default"
        g = sweep(gcc, flag_args, names)
        lc = sweep(lccc, flag_args, names)
        diverging = [n for n in names if g.get(n) != lc.get(n)]
        if diverging:
            for name in diverging[:10]:
                failures.append(
                    f"answer[{label}] {name}: gcc={g.get(name)} lccc={lc.get(name)}"
                )
            failures.append(f"answer[{label}]: {len(diverging)} divergence(s)")
        else:
            yes = sum(g.values())
            print(
                f"has_builtin answers [{label}]: PASS "
                f"({len(names)} names, {yes} known to GCC)"
            )

        # The macro proxy itself: same flags must define the same gating
        # macros in both compilers (implication chains included).
        g_macros = defined_macros(gcc, flag_args) & set(GATING_MACROS)
        l_macros = defined_macros(lccc, flag_args) & set(GATING_MACROS)
        if g_macros != l_macros:
            failures.append(
                f"macros[{label}]: gcc={sorted(g_macros)} lccc={sorted(l_macros)}"
            )

    source = ROOT / "tests/regression/_has_builtin_sweep.c"
    if source.exists():
        source.unlink()

    if failures:
        for line in failures:
            print(f"ERROR: {line}", file=sys.stderr)
        return 1
    print(f"has_builtin feature gating: PASS ({len(names)} names x {len(FLAGS)} flag sets)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
