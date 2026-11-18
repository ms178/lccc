#!/usr/bin/env python3
"""Every regression fixture that uses an ISA-gated x86 intrinsic must ship a
NAME.flags file that enables that ISA.

run_regression.py falls back to DEFAULT_FLAGS (-O2, no ISA) when a .flags file
is absent. A fixture that calls an AVX intrinsic under that default fails with
"requires the avx instruction-set extension", which is what broke the SSA
corpus lane on avx1_fp256_logic_only.c. This check makes that a static failure
instead of a CI-only one.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent

# Intrinsic-name prefix -> the -m flag that enables it. Only the families whose
# absence the compiler rejects; SSE2 is the x86-64 baseline and needs no flag.
REQUIRES = (
    (re.compile(r"\b_mm256_\w+"), "-mavx"),
    (re.compile(r"\b_mm512_\w+"), "-mavx512f"),
)


# A flag enables every ISA it implies, so -mavx2 satisfies -mavx. Only the
# implications the x86 driver actually has are listed; each is a superset.
IMPLIES = {
    "-mavx2": {"-mavx"},
    "-mavx512f": {"-mavx", "-mavx2"},
    "-mavx512bw": {"-mavx512f", "-mavx", "-mavx2"},
    "-mavx512vl": {"-mavx512f", "-mavx", "-mavx2"},
}


def flags_for(src: pathlib.Path) -> set[str]:
    f = src.with_suffix(".flags")
    words = f.read_text().split() if f.exists() else ["-O2"]
    enabled = set(words)
    for word in words:
        enabled |= IMPLIES.get(word, set())
    return enabled


# Preprocessor guards that remove an intrinsic when its ISA is absent. A use
# inside such a region is legitimate without the flag: the fixture is written to
# exercise the non-ISA path. Only unguarded uses are required to have the flag.
_GUARD = re.compile(
    r"^[ \t]*#[ \t]*if(?:def)?[ \t]*\(?[ \t]*(?:defined[ \t]*\(?[ \t]*)?__AVX",
    re.M,
)


def strip_isa_guarded(text: str) -> str:
    """Return text with every `#if ...__AVX...` region (to its matching #endif) blanked."""
    out, depth, guarded = [], 0, False
    for line in text.splitlines(keepends=True):
        stripped = line.strip()
        if stripped.startswith("#"):
            directive = stripped[1:].lstrip()
            if directive.startswith(("if", "ifdef", "ifndef")):
                if guarded:
                    depth += 1
                elif _GUARD.match(line):
                    guarded, depth = True, 1
                    out.append("\n")
                    continue
            elif guarded and directive.startswith("endif"):
                depth -= 1
                if depth == 0:
                    guarded = False
                out.append("\n")
                continue
        out.append("\n" if guarded else line)
    return "".join(out)


# Fixtures whose ISA flags are supplied by a dedicated driver, not by a
# NAME.flags file that run_regression.py reads. Each entry names the driver that
# owns the ISA configuration, so the exemption is auditable and cannot grow
# silently: a new unguarded AVX fixture still fails this check.
DRIVER_OWNED = {
    "legacy_sse_native_chain.c": "check_legacy_sse_native.py (-mavx/-mavx2 -DUSE_AVX)",
    "native_packed_fp_helpers.c": "check_legacy_sse_native.py (-mavx/-mavx2 -DUSE_AVX)",
}


def check() -> list[str]:
    problems = []
    for src in sorted(ROOT.glob("*.c")):
        if src.name in DRIVER_OWNED:
            continue
        text = strip_isa_guarded(src.read_text(errors="replace"))
        have = flags_for(src)
        for pattern, flag in REQUIRES:
            if pattern.search(text) and flag not in have:
                problems.append(f"{src.name}: uses {pattern.pattern!r} but flags "
                                f"{sorted(have)} lack {flag}")
    return problems


def main() -> int:
    problems = check()
    for p in problems:
        print(f"ISA-FLAGS FAIL: {p}", file=sys.stderr)
    if problems:
        return 1
    print("ISA-FLAGS OK: every fixture using an ISA-gated intrinsic enables its ISA")
    return 0


if __name__ == "__main__":
    sys.exit(main())
