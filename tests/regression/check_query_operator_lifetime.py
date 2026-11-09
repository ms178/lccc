#!/usr/bin/env python3
"""GCC-16.2 lifetime contract for the special `__has_*` query operators.

The operators are compiler-owned names, not ordinary MacroDef entries.  GCC
and Clang both let `#undef` withdraw them, `#define` replace them, and
`#pragma push_macro/pop_macro` save/restore that state.  LCCC previously
ignored `#undef` for them (the operator stayed live in `#ifdef`, `#if` and
code), and its `#if` path evaluated the builtin before a user macro could
expand.  This gate pins the corrected lifetime for every operator and the
unclassified directive-warning controls measured on gcc-16.2.

`__has_warning` is LCCC's Clang-compatible extension: it has the same
lifetime, but GCC does not define it and emits no builtin directive warnings
for it, so its warning expectations differ.
"""
from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent

# operator -> a well-formed operand for #if probes
GCC_OPS = {
    "__has_feature": "tls",
    "__has_extension": "tls",
    "__has_builtin": "memcpy",
    "__has_attribute": "packed",
    "__has_c_attribute": "deprecated",
    "__has_include": "<stddef.h>",
    "__has_include_next": "<stddef.h>",
    "__has_embed": '"nonexistent.bin"',
}
# Operators LCCC also resolves in ordinary code lines (include/embed are
# directive-only; their code-line spelling is not a query).
CODE_OPS = {
    "__has_feature", "__has_extension", "__has_builtin",
    "__has_attribute", "__has_c_attribute", "__has_warning",
}
ALL_OPS = dict(GCC_OPS, __has_warning='"-Wcpp"')
# Expected live answer for the operand above (GCC 16.2 / LCCC registry).
LIVE_VALUE = {
    "__has_feature": "1", "__has_extension": "1", "__has_builtin": "1",
    "__has_attribute": "1", "__has_c_attribute": "202311", "__has_warning": "1",
}


def run(cc: str, src: Path, *flags: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [cc, "-std=gnu23", "-E", "-P", *flags, str(src)],
        capture_output=True, text=True, timeout=20,
    )


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    args = ap.parse_args(argv)
    failures: list[str] = []
    checks = 0

    def check(name: str, cond: bool, detail: str = "") -> None:
        nonlocal checks
        checks += 1
        if not cond:
            failures.append(f"{name}: {detail}")

    with tempfile.TemporaryDirectory(prefix="lccc-query-life.") as tmp:
        src = Path(tmp) / "case.c"
        for op, operand in ALL_OPS.items():
            gcc_op = op in GCC_OPS
            # 1. Initially defined, unless it is the Clang-only extension in
            #    a GCC persona (LCCC still defines __has_warning).
            src.write_text(f"#ifdef {op}\nint yes=1;\n#else\nint no=0;\n#endif\n")
            r = run(args.lccc, src)
            check(f"{op}/initial", r.returncode == 0 and "int yes=1;" in r.stdout, r.stderr)

            # 2. #undef withdraws it from #ifdef and defined().
            src.write_text(
                f"#undef {op}\n"
                f"#ifdef {op}\nint yes=1;\n#else\nint no=0;\n#endif\n"
                f"#if defined({op})\nint dyes=1;\n#else\nint dno=0;\n#endif\n"
            )
            r = run(args.lccc, src)
            check(f"{op}/undef-defined", r.returncode == 0 and "int no=0;" in r.stdout
                  and "int dno=0;" in r.stdout and "int yes=1;" not in r.stdout, r.stderr)
            if gcc_op:
                check(f"{op}/undef-warning", f"undefining '{op}'" in r.stderr, r.stderr)

            # 3. A withdrawn operator is an ordinary name in code and an error
            #    in #if (stray argument list), never a silent builtin answer.
            if op in CODE_OPS:
                src.write_text(f"#undef {op}\nint code={op}({operand});\n")
                r = run(args.lccc, src)
                check(f"{op}/undef-code", f"int code={op}({operand});" in r.stdout, r.stdout)
            src.write_text(
                f"#undef {op}\n#if {op}({operand})\nint yes=1;\n#else\nint no=0;\n#endif\n"
            )
            r = run(args.lccc, src)
            check(f"{op}/undef-if", r.returncode != 0 and "missing binary operator" in r.stderr,
                  f"rc={r.returncode} {r.stderr}")

            # 4. A user #define takes over in both #if and code.
            src.write_text(
                f"#define {op}(x) 7\n"
                f"#if {op}({operand}) == 7\nint yes=1;\n#else\nint no=0;\n#endif\n"
                + (f"int code={op}({operand});\n" if op in CODE_OPS else "")
            )
            r = run(args.lccc, src)
            check(f"{op}/redef-if", r.returncode == 0 and "int yes=1;" in r.stdout
                  and "int no=0;" not in r.stdout, r.stderr)
            if op in CODE_OPS:
                check(f"{op}/redef-code", "int code=7;" in r.stdout, r.stdout)
            if gcc_op:
                check(f"{op}/redef-warning", f"'{op}' redefined" in r.stderr, r.stderr)

            # 5. push_macro/pop_macro restores the live operator after #undef.
            src.write_text(
                f'#pragma push_macro("{op}")\n#undef {op}\n'
                f'#pragma pop_macro("{op}")\n'
                f"#ifdef {op}\nint yes=1;\n#else\nint no=0;\n#endif\n"
            )
            r = run(args.lccc, src)
            check(f"{op}/push-pop", r.returncode == 0 and "int yes=1;" in r.stdout, r.stderr)

            # 6. An #undef inside an inactive block has no effect.
            src.write_text(
                f"#if 0\n#undef {op}\n#endif\n"
                f"#ifdef {op}\nint yes=1;\n#else\nint no=0;\n#endif\n"
            )
            r = run(args.lccc, src)
            check(f"{op}/skipped-undef", r.returncode == 0 and "int yes=1;" in r.stdout, r.stderr)

            # 7. An alias expands to the operator only while it is live.
            src.write_text(
                f"#define ALIAS {op}\nint live=ALIAS({operand});\n"
                f"#undef {op}\nint dead=ALIAS({operand});\n"
                if op in CODE_OPS else
                f"#define ALIAS {op}\n#if ALIAS({operand})\nint live=1;\n#endif\n"
                f"#undef {op}\n#if ALIAS({operand})\nint dead=1;\n#else\nint dead=0;\n#endif\n"
            )
            r = run(args.lccc, src)
            if op in CODE_OPS:
                check(f"{op}/alias-live", f"int live={LIVE_VALUE[op]};" in r.stdout, r.stdout)
                check(f"{op}/alias-dead", f"int dead=ALIAS({operand});" in r.stdout
                      or f"int dead={op}({operand});" in r.stdout, r.stdout)
            else:
                check(f"{op}/alias-dead-if", r.returncode != 0 and "int dead=0;" in r.stdout,
                      f"rc={r.returncode} {r.stdout} {r.stderr}")

            if not gcc_op:
                continue
            # 8. Unclassified warning controls (GCC 16.2 measurements).
            src.write_text(f"#undef {op}\n")
            for flags, want in [
                (["-w"], False),
                (["-Wno-cpp"], True),
                (["-Wno-builtin-macro-redefined"], True),
            ]:
                r = run(args.lccc, src, *flags)
                check(f"{op}/warn{flags}", r.returncode == 0
                      and (f"undefining '{op}'" in r.stderr) == want, r.stderr)
            r = run(args.lccc, src, "-Werror")
            check(f"{op}/werror", r.returncode != 0 and "undefining" in r.stderr, r.stderr)

            # 9. -pedantic-errors makes taking over the live operator an error,
            #    but defining after #undef is only the weaker 'defined' warning.
            src.write_text(f"#define {op}(x) 7\n")
            r = run(args.lccc, src, "-pedantic-errors")
            check(f"{op}/pedantic-redef", r.returncode != 0 and "redefined" in r.stderr, r.stderr)
            r = run(args.lccc, src, "-pedantic-errors", "-w")
            check(f"{op}/pedantic-w", r.returncode == 0 and r.stderr == "", r.stderr)
            src.write_text(f"#undef {op}\n#define {op}(x) 7\n")
            r = run(args.lccc, src, "-pedantic-errors")
            check(f"{op}/pedantic-after-undef", r.returncode == 0
                  and f"'{op}' defined" in r.stderr, r.stderr)

        # 10. Extra tokens after #undef are diagnosed, repeated #undef still warns.
        src.write_text("#undef __has_feature junk\n#undef __has_feature\n")
        r = run(args.lccc, src)
        check("undef/extra-tokens", "extra tokens at end of '#undef' directive" in r.stderr, r.stderr)
        check("undef/repeated", r.stderr.count("undefining '__has_feature'") == 2, r.stderr)

    if failures:
        print(f"FAIL: query-operator lifetime ({len(failures)}/{checks})", file=sys.stderr)
        for f in failures[:30]:
            print(f"  {f}", file=sys.stderr)
        return 1
    print(f"query-operator lifetime: ok ({checks} checks, {len(ALL_OPS)} operators)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
