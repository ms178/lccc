#!/usr/bin/env python3
"""Does a call of each x86 intrinsic have the type its header gives it?

Why
---
`x86_intrinsic_census.py` counts the intrinsics a program can call under
lccc, and `x86_intrinsic_values.py` runs them: `__m128i r = NAME(a, b);`
and the bytes of `r` have to be GCC's.  Neither asks what *type* the call
has, and for 48 intrinsics it had none.  lccc lowers some public names
itself (`_mm_sll_epi16` is the instruction, no header function), and a name
that no header declares and no builtin table types is a call of an
undeclared function: `int`.  Its value is the address of a 16- or 32-byte
result.  Initializing an `__m128i` from it happens to copy the right bytes,
so every test that stores the result passed.  Everything that reads the
call by its type was wrong code:

    __m128i f(__m128i a, __m128i n) { return _mm_sll_epi16(a, n); }

returned a stack address in `%xmm0`; `sizeof(_mm_sll_epi16(a, n))` was 8;
the call as the argument of another function passed a pointer.

How
---
For every intrinsic of GCC's headers that GCC lets a program call (the
census's reader and callers, with every extension enabled: a type needs no
processor), the caller is written with one assertion in front of it:

    __m128i t7(__m128i a0, __m128i a1)
    { _Static_assert(__builtin_types_compatible_p(
          __typeof__(_mm_sll_epi16(a0, a1)), __m128i), "_mm_sll_epi16");
      return _mm_sll_epi16(a0, a1); }

GCC goes first: an assertion GCC itself refuses is this tool's mistake and
is dropped.  Then lccc compiles the callers without the assertion (what it
lacks is the census's finding, not this tool's) and the rest with it.  A
caller lccc refuses only with the assertion is a call of another type than
the header's, and is listed.

    scripts/x86_intrinsic_types.py --lccc target/fastbuild/lccc --check
    scripts/x86_intrinsic_types.py --lccc target/fastbuild/lccc --only emmintrin.h

`--check` fails when any intrinsic is listed.  There is no record to
update: the number is zero.

GCC must be 16.2 (`--gcc`, `GCC`, default `gcc-16`): the headers are the
list.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from scripts import x86_intrinsic_census as census  # noqa: E402

RETURN = re.compile(r"\{ return (.*); \}$")


def asserted(line: str, name: str, returns: str) -> str | None:
    """The caller `line` with the assertion on the type of its call, or
    `None` for a call that has no value."""
    match = RETURN.search(line)
    if returns == "void" or not match:
        return None
    plain = re.sub(r"\bconst\b", "", returns).strip()
    check = (f'_Static_assert(__builtin_types_compatible_p(__typeof__({match.group(1)}), '
             f'{plain}), "{name}"); ')
    return line[:match.start()] + "{ " + check + line[match.start() + 2:]


def examine(header: Path, lccc: str, gcc: str) -> tuple[int, list[tuple[str, str]]]:
    """(how many intrinsics of `header` were checked, [(name, header type)]
    of those whose call has another type)."""
    found = census.functions_of(header.read_text(errors="replace"))
    names = sorted(found)
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory)
        plain = [census.caller(i, name, *found[name]) for i, name in enumerate(names)]
        checked = [asserted(plain[i], name, found[name][0]) for i, name in enumerate(names)]
        alive = [i for i in range(len(names)) if checked[i] is not None]
        refused = census.refused([gcc, *census.GCC_FLAGS, "-O1"], [checked[i] for i in alive], tmp)
        alive = [alive[i] for i in range(len(alive)) if i not in refused]
        lacking = census.refused([lccc, *census.LCCC_FLAGS, "-O1"], [plain[i] for i in alive],
                                 tmp, census.OBJECT)
        alive = [alive[i] for i in range(len(alive)) if i not in lacking]
        wrong = census.refused([lccc, *census.LCCC_FLAGS, "-O1"], [checked[i] for i in alive], tmp)
        return len(alive), [(names[alive[i]], found[names[alive[i]]][0]) for i in sorted(wrong)]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--lccc", default=str(ROOT / "target" / "release" / "lccc"))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"),
                        help="GCC 16.2: its headers are the list")
    parser.add_argument("--only", metavar="HEADER", action="append", default=[])
    parser.add_argument("--check", action="store_true",
                        help="fail when a call has another type than the header's")
    args = parser.parse_args(argv)

    if not Path(args.lccc).exists():
        print(f"x86_intrinsic_types: no lccc at {args.lccc} (--lccc)", file=sys.stderr)
        return 2
    # The callers are compiled in a directory of their own.
    args.lccc = str(Path(args.lccc).resolve())
    version = census.gcc_version_of(args.gcc)
    if version != census.GCC_VERSION:
        print(f"x86_intrinsic_types: the headers are GCC {census.GCC_VERSION}'s; {args.gcc} is "
              f"{version or 'not runnable'} (--gcc, or GCC=)", file=sys.stderr)
        return 2
    include = Path(subprocess.run([args.gcc, "-print-file-name=include"], capture_output=True,
                                  text=True).stdout.strip())
    headers = [header for header in census.headers_of(include)
               if not args.only or header.name in args.only]
    total, wrong = 0, []
    with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 1) as pool:
        for header, (count, bad) in zip(headers, pool.map(
                lambda header: examine(header, args.lccc, args.gcc), headers)):
            total += count
            wrong += [(header.name, name, returns) for name, returns in bad]
    for header, name, returns in wrong:
        print(f"{header}: a call of {name} is not of type {returns}")
    print(f"x86_intrinsic_types: {total} intrinsics, {len(wrong)} whose call has another type"
          f" than GCC {census.GCC_VERSION}'s header gives it")
    return 1 if args.check and wrong else 0


if __name__ == "__main__":
    sys.exit(main())
