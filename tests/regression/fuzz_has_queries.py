#!/usr/bin/env python3
"""Deterministic robustness fuzz for the `__has_*` preprocessor scanners.

The resolvers walk raw byte slices with manual indices, so the failure modes
that matter are not wrong answers but *panics, hangs and index errors*:

* an index that runs past the slice (missing ')' at end of line);
* a recovery loop that makes no progress (a lone quote or backslash);
* nested-paren recovery that underflows its depth counter;
* a macro operand whose expansion re-enters the expander.

Malformed input is EXPECTED to produce a diagnostic and a non-zero exit; that is
a pass.  A crash, a signal, or a timeout is a failure.  The corpus is seeded, so
a failure is reproducible from the printed case index alone.
"""
from __future__ import annotations

import argparse
import random
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
OPS = (
    "__has_feature",
    "__has_extension",
    "__has_builtin",
    "__has_attribute",
    "__has_c_attribute",
)
ATOMS = [
    "c_atomic", "c_generic_selections", "tls", "cxx_binary_literals",
    "gnu_asm_goto_with_outputs", "__c_generic_selections__", "modules", "",
    "(", ")", "::", "gnu::deprecated", '"', "'", "\\", '")"', "'('",
    " ", "\t", "123", "0b1", "é", "\u00ff", "​", "x" * 300, "FEAT", "SELF",
    "NOTIDENT", "..", ",", ";", "\x00", "\r",
]
PRELUDE = (
    "#define FEAT c_generic_selections\n"
    "#define SELF __has_feature\n"
    "#define NOTIDENT c_generic_selections tls\n"
    "#define LOOP LOOP\n"
    "#define PAREN (\n"
)


def cases(seed: int, count: int) -> list[str]:
    rng = random.Random(seed)
    out = []
    # Hand-built boundary cases first: empty operand, EOL right after the
    # name/quote/paren, and recovery that must not leave a stray token.
    for op in OPS:
        for tail in ("", "(", "()", "(;", "(x", '( "', "(' ", "(\\", "((x))",
                     "(x y)", "(())", "(\"))", "('))", "(x]", "(()", "())))"):
            out.append(f"int v = {op}{tail}\nint after = 1;\n")
            out.append(f"#if {op}{tail}\nint x = 1;\n#else\nint x = 0;\n#endif\n")
    for _ in range(count):
        op = rng.choice(OPS)
        body = "".join(rng.choice(ATOMS) for _ in range(rng.randint(0, 6)))
        line = f"int v = {op}{body}\nint after = 1;\n"
        if rng.random() < 0.4:
            line = f"#if {op}{body}\nint x = 1;\n#else\nint x = 0;\n#endif\n"
        if rng.random() < 0.2:
            line = f"#define M {op}{body}\nint v = M;\n"
        # Exercise the lifetime states as well as the live operator: a
        # withdrawn operator is an ordinary name, a user macro takes over.
        r = rng.random()
        if r < 0.2:
            line = f"#undef {op}\n" + line
        elif r < 0.35:
            line = f"#define {op}(x) 7\n" + line
        out.append(line)
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    ap.add_argument("--seed", type=int, default=20261108)
    ap.add_argument("--count", type=int, default=250)
    ap.add_argument("--timeout", type=float, default=10.0)
    args = ap.parse_args(argv)

    stds = ["-std=gnu23", "-std=c99", "-std=gnu11"]
    failures = []
    total = 0
    with tempfile.TemporaryDirectory(prefix="lccc-hasfuzz.") as tmp:
        src = Path(tmp) / "case.c"
        for i, body in enumerate(cases(args.seed, args.count)):
            src.write_text(PRELUDE + body)
            std = stds[i % len(stds)]
            total += 1
            try:
                proc = subprocess.run(
                    [args.lccc, "-E", std, str(src)],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.PIPE,
                    timeout=args.timeout,
                )
            except subprocess.TimeoutExpired:
                failures.append(f"case {i} ({std}): TIMEOUT after {args.timeout}s\n{body}")
                continue
            err = proc.stderr.decode("utf-8", "replace")
            if proc.returncode < 0:
                failures.append(
                    f"case {i} ({std}): killed by signal {-proc.returncode}\n{body}"
                )
            elif "panicked" in err or "internal compiler error" in err or "RUST_BACKTRACE" in err:
                failures.append(f"case {i} ({std}): crash diagnostic\n{body}\n{err[:400]}")

    if failures:
        print(f"FAIL: __has_* robustness fuzz ({len(failures)}/{total})", file=sys.stderr)
        for f in failures[:20]:
            print(f"  {f}", file=sys.stderr)
        return 1
    print(f"has-query fuzz: ok ({total} cases, seed {args.seed}, no panic/hang/signal)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
