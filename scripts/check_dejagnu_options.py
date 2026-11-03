#!/usr/bin/env python3
"""Prove the DejaGnu grammar widening is a strict superset of the old one.

The x86 and AArch64 torture runners used to accept ONLY quoted ``dg-options``.
``tools/corpus/dejagnu.py`` adds the Tcl brace-list spelling.  This checker
walks a provisioned GCC C-torture corpus and asserts, for every source:

1. the new parser returns every flag the old quote-only parser returned
   (no regression for the 666 quoted directives in GCC 17-git);
2. every *added* flag is attributable to a brace-list directive;
3. the witness ``930529-1.c`` yields ``-fwrapv`` -- the flag whose absence
   made the GCC reference die with SIGILL on both x86 targets.

It never treats a missing corpus as a pass.  Use ``--corpus`` to point at a
checkout; CI lanes that do not provision the corpus should not call this
script.  Exit 0 = proof, 1 = regression, 2 = unusable input.

Example::

    python3 scripts/check_dejagnu_options.py \
        --corpus ~/.cache/lccc-gcc17/gcc/testsuite/gcc.c-torture
"""
from __future__ import annotations

import argparse
import re
import shlex
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from tools.corpus import dejagnu  # noqa: E402

OLD = re.compile(r"\{\s*dg-(?:additional-)?options\s+\"([^\"]*)\"([^}]*)\}")
BRACE = re.compile(r"\{\s*dg-(?:additional-)?options\s+\{")


def old_flags(text: str) -> tuple[str, ...]:
    out: list[str] = []
    for match in OLD.finditer(text[:8192]):
        if "target" in match.group(2):
            continue
        try:
            out.extend(shlex.split(match.group(1)))
        except ValueError:
            pass
    return tuple(out)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path,
                        default=Path.home() / ".cache/lccc-gcc17/gcc/testsuite/gcc.c-torture")
    args = parser.parse_args(argv)
    root = args.corpus
    legs = (root / "execute", root / "compile")
    missing = [str(p) for p in legs if not p.is_dir()]
    if missing:
        print("error: corpus leg directories missing: " + ", ".join(missing), file=sys.stderr)
        return 2
    sources = sorted(p for leg in legs for p in leg.glob("*.c"))
    if not sources:
        print("error: corpus contains no C sources", file=sys.stderr)
        return 2
    problems: list[str] = []
    added_total: dict[str, int] = {}
    quoted = 0
    for source in sources:
        text = source.read_text(errors="replace")[:8192]
        before, after = old_flags(text), dejagnu.extract_option_flags(text)
        if OLD.search(text):
            quoted += 1
        if not set(before) <= set(after):
            problems.append(f"{source.name}: old flags {before!r} not preserved in {after!r}")
        extra = tuple(f for f in after if f not in before)
        if extra and not BRACE.search(text):
            problems.append(f"{source.name}: flags {extra!r} added without a brace-list directive")
        for flag in extra:
            added_total[flag] = added_total.get(flag, 0) + 1
    witness = Path(legs[0]) / "930529-1.c"
    if witness.is_file() and "-fwrapv" not in dejagnu.extract_option_flags(witness.read_text(errors="replace")):
        problems.append("930529-1.c: -fwrapv is still not extracted")
    print(f"sources={len(sources)} quoted-directive-sources={quoted} "
          f"added-flags={added_total or '{}'}")
    if problems:
        for problem in problems[:20]:
            print("REGRESSION: " + problem, file=sys.stderr)
        if len(problems) > 20:
            print(f"... and {len(problems) - 20} more", file=sys.stderr)
        return 1
    print("PASS: new DejaGnu grammar is a strict superset for this corpus")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
