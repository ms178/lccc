#!/usr/bin/env python3
"""Check GCC-compatible warning groups and suppression for frontend warnings."""

from __future__ import annotations

import argparse
import re
import shlex
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = r'''
extern int printf(const char *, ...) __attribute__((format(printf, 1, 2)));
static int unused_file_scope;

int warning_probe(int signed_value, unsigned int unsigned_value) {
    int unused_local = 0;
    int shadowed = 1;
    [[maybe_unused]] int deliberately_suppressed = 1;
    {
        int shadowed = 2;
        (void)shadowed;
        int used_inner_shadow = 3;
        (void)used_inner_shadow;
    }
    return (signed_value < unsigned_value) + printf("%d", 1L);
}
'''

FEATURES = {
    "unused": "unused variable",
    "format": "format '%d'",
    "sign": "different signedness",
}
CASES = {
    "default": ((), set()),
    "wall": (("-Wall",), {"unused", "format"}),
    "wextra": (("-Wextra",), {"sign"}),
    "wall-extra": (("-Wall", "-Wextra"), {"unused", "format", "sign"}),
    "individual-unused": (("-Wunused-variable",), {"unused"}),
    "individual-format": (("-Wformat",), {"format"}),
    "individual-sign": (("-Wsign-compare",), {"sign"}),
    "disable-unused": (("-Wall", "-Wno-unused-variable"), {"format"}),
    "disable-format": (("-Wall", "-Wno-format"), {"unused"}),
    "disable-sign": (("-Wextra", "-Wno-sign-compare"), set()),
    "disable-sign-wall-extra": (("-Wall", "-Wextra", "-Wno-sign-compare"), {"unused", "format"}),
}


def compiler_version(command: list[str]) -> str:
    proc = subprocess.run(command + ["-dumpfullversion"], text=True, capture_output=True)
    if proc.returncode:
        raise RuntimeError(f"cannot query GCC version: {proc.stderr.strip()}")
    return proc.stdout.strip()


def observed_features(stderr: str) -> set[str]:
    return {feature for feature, phrase in FEATURES.items() if phrase in stderr}


def run_case(label: str, command: list[str], flags: tuple[str, ...], source: Path) -> set[str]:
    proc = subprocess.run(
        command + ["-std=c23", "-fsyntax-only", *flags, str(source)],
        cwd=ROOT,
        text=True,
        capture_output=True,
        timeout=30,
    )
    if proc.returncode:
        raise RuntimeError(
            f"{label} {flags} rejected the warning fixture (exit {proc.returncode}):\n"
            f"{proc.stderr[-4000:]}"
        )
    found = observed_features(proc.stderr)
    # The C23 attribute and the referenced inner object must remain silent.
    if "deliberately_suppressed" in proc.stderr or "used_inner_shadow" in proc.stderr:
        raise RuntimeError(f"{label} warned about a suppressed/used object under {flags}:\n{proc.stderr}")
    if "unused" in found and (
        "unused_local" not in proc.stderr or "shadowed" not in proc.stderr
    ):
        raise RuntimeError(f"{label} missed an unused local or the shadowed outer object under {flags}:\n{proc.stderr}")
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        assert CASES["wextra"][1] == {"sign"}
        assert CASES["wall"][1] == {"unused", "format"}
        assert CASES["wall-extra"][1] == {"unused", "format", "sign"}
        print("warning compatibility manifest self-test: PASS")
        return 0

    gcc = shlex.split(args.gcc)
    lccc = shlex.split(args.lccc)
    version = compiler_version(gcc)
    if not re.match(r"^16\.2(?:\.|$)", version):
        raise RuntimeError(f"expected GCC 16.2, found {version!r}")
    if lccc and not Path(lccc[0]).is_absolute() and "/" in lccc[0]:
        lccc[0] = str((ROOT / lccc[0]).resolve())

    with tempfile.TemporaryDirectory(prefix="lccc-warning-compat-") as temp:
        source = Path(temp) / "warnings.c"
        source.write_text(SOURCE, encoding="utf-8")
        for label, (flags, expected) in CASES.items():
            gcc_found = run_case("GCC 16.2", gcc, flags, source)
            lccc_found = run_case("LCCC", lccc, flags, source)
            if gcc_found != expected:
                raise RuntimeError(
                    f"GCC 16.2 {label} {flags}: expected {sorted(expected)}, got {sorted(gcc_found)}"
                )
            if lccc_found != gcc_found:
                raise RuntimeError(
                    f"LCCC {label} {flags}: expected GCC warnings {sorted(gcc_found)}, "
                    f"got {sorted(lccc_found)}"
                )
            print(f"warning compatibility {label}: PASS ({', '.join(sorted(lccc_found)) or 'silent'})")

    print(f"warning compatibility gate: PASS (GCC {version})")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        raise SystemExit(f"warning compatibility: ERROR: {exc}")
