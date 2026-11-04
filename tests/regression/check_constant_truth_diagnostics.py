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
# Structured and unstructured statement transfers share the same semantic
# predicate. Each count is a language-level expectation, not just parity.
CASES.update({
    "while_break": ("while (1) { break; }", 1),
    "for_break": ("for (;;) { break; }", 1),
    "do_break": ("do { break; } while (1);", 1),
    "do_continue": ("do { continue; } while (0);", 1),
    "while_continue": ("while (1) { continue; }", 0),
    "for_continue": ("for (;;) { continue; }", 0),
    "do_continue_forever": ("do { continue; } while (1);", 0),
    "conditional_break": ("extern int unknown; while (1) { if (unknown) break; }", 1),
    "dead_break": ("while (1) { return 1; break; }", 0),
    "false_break": ("while (1) { if (0) break; }", 0),
    "dead_after_continue": ("while (1) { continue; break; }", 0),
    "inner_break": ("while (1) { while (1) { break; } }", 0),
    "inner_then_outer_break": ("while (1) { while (1) { break; } break; }", 1),
    "unreachable_outer_break": ("while (1) { while (1) {} break; }", 0),
    "switch_break_only": ("while (1) { switch (0) { default: break; } }", 0),
    "switch_continue": ("do { switch (0) { default: continue; } } while (0);", 1),
    "switch_break_then_outer_break": ("while (1) { switch (0) { default: break; } break; }", 1),
    "loop_in_switch": ("switch (0) { default: while (1) { break; } }", 1),
    "false_then_returns": ("if (0) {} else return 7;", 0),
    "true_then_returns": ("if (1) return 7; else {}", 0),
    "goto_out": ("while (1) { goto out; } return 1; out:;", 1),
    "goto_return": ("goto done; return 1; done: return 7;", 0),
    "goto_cycle": ("again: goto again;", 0),
    "goto_dead_break": ("while (1) { goto again; break; again:; }", 0),
    "goto_live_break": ("while (1) { goto done; continue; done: break; }", 1),
    "unreachable_label": ("return 7; dead:;", 0),
    "case_entry_after_return": ("switch (0) { return 1; default:; }", 1),
    "gnu_stmt_goto": ("({ goto out; }); return 1; out:;", 1),
    "gnu_return_expr_goto": ("return ({ goto out; 1; }); out:;", 1),
    "computed_goto_out": ("goto *&&out; return 7; out:;", 1),
    "computed_goto_cycle": ("again: goto *&&again; dead:;", 0),
    "condition_trap": ("if (({ __builtin_trap(); 1; })) {}", 0),
    "while_condition_trap": ("while (({ __builtin_trap(); 1; })) {}", 0),
    "do_condition_trap": ("do { continue; } while (({ __builtin_trap(); 1; }));", 0),
    "for_init_trap": ("for (({ __builtin_trap(); 0; }); 0;) {}", 0),
    "for_decl_trap": ("for (int i=({ __builtin_trap(); 0; }); 0;) {}", 0),
    "for_increment_trap": ("for (;; ({ __builtin_trap(); 0; })) { continue; }", 0),
    "switch_condition_trap": ("switch (({ __builtin_trap(); 0; })) { default: break; }", 0),
    "decl_trap": ("int i=({ __builtin_trap(); 0; });", 0),
    "and_trap": ("(void)(1 && ({ __builtin_trap(); 0; }));", 0),
    "and_short_circuit": ("(void)(0 && ({ __builtin_trap(); 0; }));", 1),
    "or_trap": ("(void)(0 || ({ __builtin_trap(); 0; }));", 0),
    "or_short_circuit": ("(void)(1 || ({ __builtin_trap(); 0; }));", 1),
    "conditional_trap": ("(void)(1 ? ({ __builtin_trap(); 0; }) : 0);", 0),
    "conditional_notrap": ("(void)(0 ? ({ __builtin_trap(); 0; }) : 0);", 1),
    "gnu_conditional_trap": ("(void)(0 ?: ({ __builtin_trap(); 0; }));", 0),
    "gnu_conditional_notrap": ("(void)(1 ?: ({ __builtin_trap(); 0; }));", 1),
    "asm_goto_exit": ('asm goto ("" : : : : out); return 1; out:;', 1),
    # Statement expressions nested in ordinary evaluated operands must retain
    # the enclosing return/goto context. _Generic evaluates only its selected
    # association, while sizeof and _Alignof operands remain unevaluated.
    "nested_call_argument_return": (
        "extern void consume(int); consume(({ return 7; 0; }));", 0
    ),
    "nested_subscript_index_return": (
        "int a[1]; (void)a[({ return 7; 0; })];", 0
    ),
    "nested_assignment_rhs_return": (
        "int x=0; x = ({ return 7; 0; });", 0
    ),
    "nested_call_argument_conditional_return": (
        "extern int unknown; extern void consume(int); "
        "consume(({ if (unknown) return 7; 0; }));", 1
    ),
    "generic_selected_stmt_expr_return": (
        "extern int value; (void)_Generic(value, int: ({ return 7; 0; }), "
        "default: 1);", 0
    ),
    "generic_unselected_stmt_expr_return": (
        "extern int value; (void)_Generic(value, int: 1, "
        "default: ({ return 7; 0; }));", 1
    ),
    "choose_selected_stmt_expr_return": (
        "(void)__builtin_choose_expr(1, ({ return 7; 0; }), 1);", 0
    ),
    "choose_unselected_stmt_expr_return": (
        "(void)__builtin_choose_expr(1, 1, ({ return 7; 0; }));", 1
    ),
    "constant_p_stmt_expr_is_unevaluated": (
        "(void)__builtin_constant_p(({ return 7; 0; }));", 1
    ),
    "classify_type_stmt_expr_is_unevaluated": (
        "(void)__builtin_classify_type(({ return 7; 0; }));", 1
    ),
})
# GCC also diagnoses the unreachable statement before the first case label.
# Require that exact extra diagnostic rather than disabling it or accepting
# arbitrary warnings. The semantic return-warning expectation stays unchanged.
ORACLE_AUXILIARY = {("gcc", "case_entry_after_return"): ["-Wswitch-unreachable"]}
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
                        auxiliary = ORACLE_AUXILIARY.get((family, name), [])
                        diagnostics = re.findall(r"\[(-W[^]\s]+)\]", result.stderr)
                        expected_diagnostics = ["-Wreturn-type"] * expected + auxiliary
                        passed = (result.returncode == 0
                                  and sorted(diagnostics) == sorted(expected_diagnostics)
                                  and len(re.findall(r"\bwarning:", result.stderr))
                                  == len(expected_diagnostics))
                        records.append(dict(target=target, opt=opt, case=name, compiler=family,
                                            expected=expected, expected_auxiliary=auxiliary,
                                            actual=count, diagnostics=diagnostics, rc=result.returncode,
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
                        "for (; (unsigned int)4294967296ULL;) {}",
                        "while (1) { break; }",
                        "for (;;) { break; }",
                        "do { continue; } while (0);",
                        "while (1) { switch (0) { default: break; } break; }",
                        "goto value; value:;",
                        "do { switch (0) { default: continue; } } while (0);"):
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
    auxiliary = sum(len(record.get("expected_auxiliary", [])) for record in records)
    print(f"typed constant truth: {len(records)} checks, {failures} failures, "
          f"{auxiliary} exact oracle-specific auxiliary diagnostics")
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
