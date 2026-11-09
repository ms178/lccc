#!/usr/bin/env python3
"""GCC-16.2 lifetime contract for the special `__has_*` query operators.

The operators are compiler-owned names, not ordinary MacroDef entries.  GCC
and Clang both let `#undef` withdraw them, `#define` replace them, and
`#pragma push_macro/pop_macro` save/restore that state.  LCCC previously
ignored `#undef` for them (the operator stayed live in `#ifdef`, `#if` and
code), and its `#if` path evaluated the builtin before a user macro could
expand.  This gate pins the corrected lifetime for every operator and the
unclassified directive-warning controls measured on gcc-16.2.

`__has_warning` and `__building_module` are LCCC's Clang-compatible
extensions: they have the same lifetime, but GCC does not define them and
emits no builtin directive warnings for them, so their warning expectations
differ. `__building_module` always answers 0 (LCCC has no Clang modules); it
exists so the Clang resource headers' `!__building_module(_Builtin_stddef)`
textual-fallback guards preprocess instead of dying on a stray argument list.
"""
from __future__ import annotations

import argparse
import shutil
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
    "__building_module",
}
ALL_OPS = dict(GCC_OPS, __has_warning='"-Wcpp"', __building_module="_Builtin_stddef")
# Expected live answer for the operand above (GCC 16.2 / LCCC registry).
LIVE_VALUE = {
    "__has_feature": "1", "__has_extension": "1", "__has_builtin": "1",
    "__has_attribute": "1", "__has_c_attribute": "202311", "__has_warning": "1",
    "__building_module": "0",
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

        # 10. Malformed operands are GCC-worded errors, never silent zeros.
        malformed = [
            ("__has_builtin()", "macro '__has_builtin' requires an identifier"),
            ("__has_builtin(123)", "macro '__has_builtin' requires an identifier"),
            ("__has_builtin(memcpy strlen)", "expected ')' after 'memcpy'"),
            ("__has_builtin memcpy", "missing '(' after '__has_builtin'"),
            ("__has_attribute()", "macro '__has_attribute' requires an identifier"),
            ("__has_attribute(packed aligned)", "missing ')' after '__has_attribute'"),
            ("__has_attribute(gnu::)", "attribute identifier required after scope"),
            ("__has_attribute packed", "missing '(' after '__has_attribute'"),
            ("__has_c_attribute()", "macro '__has_attribute' requires an identifier"),
            ("__has_c_attribute(deprecated fallback)", "missing ')' after '__has_attribute'"),
            ("__has_feature tls", "missing '(' after '__has_feature'"),
            ("__has_extension tls", "missing '(' after '__has_extension'"),
        ]
        for expr, msg in malformed:
            for ctx, text in [
                ("code", f"int x={expr};\n"),
                ("if", f"#if {expr}\nint x=1;\n#endif\n"),
            ]:
                src_path = Path(tmp) / "malformed.c"
                src_path.write_text(text)
                r = run(args.lccc, src_path)
                check(f"malformed/{expr}/{ctx}", r.returncode != 0 and msg in r.stderr,
                      f"rc={r.returncode} {r.stderr}")
        # A macro operand expanding to several tokens quotes the first token.
        src.write_text("#define B memcpy strlen\nint x=__has_builtin(B);\n")
        r = run(args.lccc, src)
        check("malformed/macro-junk", r.returncode != 0 and "expected ')' after 'memcpy'" in r.stderr,
              r.stderr)

        # Directive-only operators are rejected in code (GCC still answers),
        # and malformed header operands are errors in #if.
        for op, operand in [
            ("__has_include", "<stddef.h>"),
            ("__has_include_next", "<stddef.h>"),
            ("__has_embed", '"nonexistent.bin"'),
        ]:
            src.write_text(f"int x={op}({operand});\n")
            r = run(args.lccc, src)
            check(f"directive-only/{op}/code", r.returncode != 0
                  and f"'{op}' used outside of preprocessing directive" in r.stderr, r.stderr)
        for expr, msg in [
            ("__has_include()", "operator '__has_include' requires a header-name"),
            ("__has_include stddef.h", "missing '(' before '__has_include' operand"),
            ("__has_include_next()", "operator '__has_include_next' requires a header-name"),
        ]:
            src.write_text(f"#if {expr}\nint x=1;\n#endif\n")
            r = run(args.lccc, src)
            check(f"directive-only/{expr}", r.returncode != 0 and msg in r.stderr, r.stderr)

        # 11. GCC expands attribute scope/name identifiers and whole scoped tokens.
        for src_text, want in [
            ("#define SCOPE gnu\nint x=__has_attribute(SCOPE::deprecated);\n", "int x=1;"),
            ("#define NAME deprecated\nint x=__has_attribute(gnu::NAME);\n", "int x=1;"),
            ("#define SCOPED gnu::deprecated\nint x=__has_attribute(SCOPED);\n", "int x=1;"),
            ("#define SCOPED gnu::deprecated\n#if __has_attribute(SCOPED)\nint x=1;\n#else\nint x=0;\n#endif\n", "int x=1;"),
            ("#define SCOPE gnu\n#if __has_attribute(SCOPE::deprecated)\nint x=1;\n#else\nint x=0;\n#endif\n", "int x=1;"),
        ]:
            src.write_text(src_text)
            r = run(args.lccc, src)
            check(f"attr-macro/{want}", r.returncode == 0 and want in r.stdout
                  and "int x=0;" not in r.stdout, r.stdout + r.stderr)

        # 12. Clang resource headers must preprocess: their textual-fallback
        #     guards spell `!__building_module(_Builtin_stddef)`, and the strict
        #     trailing-token #if check rejects an unknown function-like name.
        #     This is the exact shape that broke the AArch64/RV64 cross gate.
        clang = shutil.which("clang-23")
        if clang:
            res = subprocess.run([clang, "-print-resource-dir"], capture_output=True,
                                 text=True, timeout=20)
            if res.returncode == 0:
                inc = Path(res.stdout.strip()) / "include"
                hdr = Path(tmp) / "clang_resource.c"
                hdr.write_text(
                    "#include <stddef.h>\n"
                    "size_t x;\n"
                    "int m=__building_module(_Builtin_stddef);\n"
                    "#if __has_feature(modules) && !__building_module(_Builtin_stddef)\n"
                    "int textual=1;\n#endif\n"
                )
                for driver in ("lccc", "lccc-i686", "lccc-arm", "lccc-riscv"):
                    cc = Path(args.lccc).parent / driver
                    if not cc.exists():
                        continue
                    r = subprocess.run(
                        [str(cc), "-isystem", str(inc), "-fsyntax-only", str(hdr)],
                        capture_output=True, text=True, timeout=30)
                    check(f"clang-resource/{driver}", r.returncode == 0, r.stderr)

        # 12b. Macro expansion must not destroy the operator-specific operand
        #      rule. Clang's __building_module / __has_warning take the RAW
        #      operand; GCC's feature/builtin/attribute queries expand it in the
        #      resolver. The old expand-whole-line-first pass expanded M before
        #      the resolver saw it (`__building_module(M)` -> stray 'bar' error,
        #      `__has_warning(W)` -> silent 1 where Clang requires a literal).
        raw_operand_cases = [
            ("bm-direct", "#define M foo bar\nint x=__building_module(M);\n",
             0, "int x=0;", ""),
            ("bm-alias", "#define M foo bar\n#define BM __building_module\nint x=BM(M);\n",
             0, "int x=0;", ""),
            ("bm-prescan", "#define M foo bar\n#define ID(x) x\nint x=ID(__building_module(M));\n",
             0, "int x=0;", ""),
            ("bm-missing-paren", "#define M foo bar\nint x=__building_module M;\n",
             1, "int x=0;", "missing '(' after '__building_module'"),
            ("hw-macro-operand", '#define W "-Wcpp"\nint x=__has_warning(W);\n',
             1, "int x=0;", "expected string literal in '__has_warning'"),
            ("hw-alias-macro-operand", '#define W "-Wcpp"\n#define HW __has_warning\nint x=HW(W);\n',
             1, "int x=0;", "expected string literal in '__has_warning'"),
            ("hw-literal", 'int x=__has_warning("-Wcpp");\n', 0, "int x=1;", ""),
            ("hw-alias-literal", '#define HW __has_warning\nint x=HW("-Wcpp");\n',
             0, "int x=1;", ""),
            ("bm-stringify", "#define M foo bar\n#define STR(x) #x\nconst char *s=STR(__building_module(M));\n",
             0, '"__building_module(M)"', ""),
            ("unused-malformed", "#define IGN(x) 7\nint x=IGN(__has_feature(bad junk));\n",
             0, "int x=7;", ""),
            ("feature-operand-expands", "#define FEAT tls\nint x=__has_feature(FEAT);\n",
             0, "int x=1;", ""),
            ("builtin-operand-expands", "#define B memcpy\nint x=__has_builtin(B);\n",
             0, "int x=1;", ""),
            ("attribute-operand-expands", "#define NAME deprecated\nint x=__has_attribute(gnu::NAME);\n",
             0, "int x=1;", ""),
        ]
        for name, text, want_rc, want_out, want_diag in raw_operand_cases:
            src.write_text(text)
            r = run(args.lccc, src)
            diag_ok = (want_diag in r.stderr) if want_diag else (r.stderr.strip() == "")
            check(f"raw-operand/{name}",
                  r.returncode == want_rc and want_out in r.stdout and diag_ok,
                  r.stdout + r.stderr)
        if clang:
            for name, text, want_rc, want_out, want_diag in raw_operand_cases:
                if not any(op in text for op in ("__building_module", "__has_warning")):
                    continue
                src.write_text(text)
                c = run(clang, src)
                l = run(args.lccc, src)
                check(f"raw-operand/clang-parity/{name}",
                      c.returncode == l.returncode == want_rc
                      and want_out in c.stdout and want_out in l.stdout
                      and ((want_diag in c.stderr) if want_diag else True),
                      f"clang={c.stdout + c.stderr}\nlccc={l.stdout + l.stderr}")

        # 13. Extra tokens after #undef are diagnosed, repeated #undef still warns.
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
