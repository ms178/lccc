#!/usr/bin/env python3
"""Typed constant truth must agree with the return-path diagnostic.

Non-kernel fixtures only. Explicit expected counts prevent a compiler with all
warnings disabled from passing. Both reference compiler versions are checked.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
CASES = {
    "narrow_if": ("if ((unsigned char)256) return 7;", 1),
    "narrow_while": ("while ((unsigned char)256) {}", 1),
    "narrow_for": ("for (; (unsigned int)4294967296ULL;) {}", 1),
    "narrow_do": ("do {} while ((unsigned char)256);", 1),
    "nested_cast": ("while ((int)(unsigned char)256) {}", 1),
    "negated_cast": ("while (!((unsigned char)256)) {}", 0),
    "nonzero_if": ("if ((unsigned char)257) return 7;", 0),
    "nonzero_while": ("while ((unsigned char)257) {}", 0),
    "boolean_cast": ("while ((_Bool)256) {}", 0),
    "wide_integer": ("while ((unsigned long long)4294967296ULL) {}", 0),
    "zero": ("while ((unsigned char)0) {}", 1),
    "enum_true": ("enum { FOREVER=1 }; while (FOREVER) {}", 0),
    "shadowed_enum": ("enum { FOREVER=1 }; {int FOREVER=0; while (FOREVER) {} }", 1),
    "shadowed_global_enum": ("int GLOBAL_FOREVER=0; while (GLOBAL_FOREVER) {}", 1),
    "local_typedef_zero": ("typedef unsigned char T; while ((T)256) {}", 1),
    "local_typedef_true": ("typedef unsigned char T; while ((T)257) {}", 0),
    "gnu_nested": ("enum { FOREVER=1 }; if (FOREVER) { "
                   "int inner(void) { return 1; } return inner(); }", 0),
}
TARGETS = {
    "x86_64": ("lccc", [], ["--target=x86_64-linux-gnu"]),
    "i686": ("lccc-i686", ["-m32"], ["--target=i686-linux-gnu"]),
    "aarch64": ("lccc-arm", None, ["--target=aarch64-linux-gnu"]),
    "riscv64": ("lccc-riscv", None, ["--target=riscv64-linux-gnu"]),
}


def run(command):
    return subprocess.run(command, text=True, capture_output=True, timeout=30)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", type=Path, default=ROOT / "target/fastbuild/lccc")
    parser.add_argument("--gcc", default=os.environ.get("GCC16", "gcc-16"))
    parser.add_argument("--clang", default=os.environ.get("CLANG23", "clang-23"))
    parser.add_argument("--icx", default=os.environ.get("ICX"))
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    for compiler, pattern in [(args.gcc, r"16\.2\."), (args.clang, r"23\.1\.4\b")]:
        version = run([compiler, "--version"])
        if version.returncode or not re.search(pattern, version.stdout.splitlines()[0]):
            parser.error(f"wrong or unavailable oracle: {compiler}: {version.stdout} {version.stderr}")
    if args.icx:
        version = run([args.icx, "--version"])
        if version.returncode or not re.search(r"2026\.2\b", version.stdout):
            parser.error("ICX must be authenticated 2026.2 (provision with ensure_icx_2026.sh)")
    records = []
    with tempfile.TemporaryDirectory(prefix="lccc-typed-truth-") as temporary:
        source = Path(temporary) / "case.c"
        for target, (binary, gcc_flags, clang_flags) in TARGETS.items():
            compilers = [("lccc", str(args.lccc.parent / binary), []),
                         ("clang", args.clang, clang_flags)]
            if gcc_flags is not None:
                compilers.append(("gcc", args.gcc, gcc_flags))
            if args.icx and target == "x86_64":
                compilers.append(("icx", args.icx, []))
            for opt in ("-O0", "-O2"):
                for name, (body, expected) in CASES.items():
                    source.write_text(f"enum {{ GLOBAL_FOREVER=1 }}; int subject(void) {{ {body} }}\n")
                    for family, compiler, flags in compilers:
                        # Clang/ICX reject GNU nested definitions. GCC and LCCC
                        # exercise the parent's fact lifetime across that walk.
                        if name == "gnu_nested" and family in ("clang", "icx"):
                            continue
                        result = run([compiler, *flags, opt, "-std=gnu11", "-Wreturn-type",
                                      "-S", str(source), "-o", os.devnull])
                        count = len(re.findall(r"\[-Wreturn-type\]", result.stderr))
                        passed = (result.returncode == 0 and count == expected
                                  and len(re.findall(r"\bwarning:", result.stderr)) == count)
                        records.append(dict(target=target, opt=opt, case=name, compiler=family,
                                            expected=expected, actual=count, rc=result.returncode,
                                            passed=passed))
                        if not passed:
                            print(f"FAIL {family}/{target}/{opt}/{name}: rc={result.returncode}, "
                                  f"warnings={count}, expected={expected}\n{result.stderr}")
        # These statement expressions must retain their reachable final value.
        # A false proof of divergence can affect typing as well as warnings.
        runtime_compilers = [("lccc", str(args.lccc)), ("gcc", args.gcc), ("clang", args.clang)]
        if args.icx:
            runtime_compilers.append(("icx", args.icx))
        for control in ("if ((unsigned char)256) __builtin_unreachable();",
                        "while ((unsigned char)256) {}",
                        "for (; (unsigned int)4294967296ULL;) {}"):
            source.write_text("int subject(void) { return ({ " + control +
                              " 7; }); } int main(void) { return subject() != 7; }\n")
            for opt in ("-O0", "-O2"):
                for family, compiler in runtime_compilers:
                    binary = Path(temporary) / "runtime"
                    built = run([compiler, opt, "-Werror", "-Wreturn-type", str(source),
                                 "-o", str(binary)])
                    executed = run([str(binary)]) if built.returncode == 0 else None
                    passed = (executed is not None and executed.returncode == 0
                              and not executed.stdout and not executed.stderr)
                    records.append(dict(target="x86_64", opt=opt, case=control,
                                        compiler=family, passed=passed,
                                        build_rc=built.returncode,
                                        run_rc=executed.returncode if executed else None))
                    if not passed:
                        print(f"FAIL runtime {family}/{opt}/{control}: {built.stderr}")
    if args.json:
        args.json.write_text(json.dumps(records, indent=2) + "\n")
    failures = sum(not record["passed"] for record in records)
    print(f"typed constant truth: {len(records)} checks, {failures} failures")
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
