#!/usr/bin/env python3
"""guarded folds: scalar/CFG contracts, guard pages, i686 and disabled controls.

The gf_range assertions also pin the interaction between the sound
range-gated same-width Cast fold and the guarded signed-range rewrite.  When
that Cast is elided, a U32 producer can reach an I32-typed Cmp.  The final IR
must still reduce to one unsigned compare, and the kill-switch / disabled-pass
controls must show exactly why.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile
from check_llvm_missed_zero_choice import (body, gcc_oracle_identity,
                                           report_gcc_oracle, run)

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/llvm_guarded_folds/driver.c"
HELPERS = SOURCE.with_name("helpers.c")
MASK64 = (1 << 64) - 1
ICX_VERSION = re.compile(r"\b2026\.2\.\d+\b")
RANGE_XS = (
    -2147483647 - 1, -2147483647, -256, -16, -15, -2, -1, 0, 1, 2,
    14, 15, 16, 255, 2147483646, 2147483647,
)
RANGE_YS = (
    0, 1, 2, 14, 15, 16, 255, 0x40000000, 0x7ffffff0,
    0x80000000, 0xfffffff0, 0xffffffff,
)


def icx_identity_ok(text):
    """Accept only the authenticated ICX 2026.2.x release family."""
    return bool(ICX_VERSION.search(text))


def run_ir(command, env=None):
    """Run a compiler command and return its final-IR stderr dump."""
    result = subprocess.run(command, env=env, capture_output=True, timeout=60)
    if result.returncode or re.search(rb"\bwarning:", result.stderr, re.I):
        raise RuntimeError(
            f"failed/warning-producing command {command!r}: "
            f"{result.stderr.decode(errors='replace')}"
        )
    return result.stderr.decode(errors="replace")


def ir_function(text, name):
    pattern = rf"^=== IR {re.escape(name)} ===\s*$(.*?)(?=^=== IR |\Z)"
    match = re.search(pattern, text, re.M | re.S)
    if not match:
        raise ValueError(f"missing final-IR function {name}")
    return match.group(1)


def check_ir_mechanisms(irs):
    on = ir_function(irs["on"], "gf_range")
    if "op: Ult" not in on:
        raise ValueError("gf_range final IR lost the unsigned compare")
    for forbidden in ("op: Sge", "op: Slt", "Select {"):
        if forbidden in on:
            raise ValueError(f"gf_range final IR still has {forbidden}: {on}")
    if "from_ty: U32, to_ty: I32" in on:
        raise ValueError("range-gated same-width Cast was not elided")
    if "op: And" not in on or "ty: U32" not in on:
        raise ValueError("mixed-signedness U32 mask witness is missing")

    off = ir_function(irs["off"], "gf_range")
    for required in ("op: Sge", "op: Slt", "Select {"):
        if required not in off:
            raise ValueError(f"disabled bit_idioms control lost {required}: {off}")
    if "op: Ult" in off:
        raise ValueError("disabled bit_idioms control unexpectedly folded the range")

    skip = ir_function(irs["skip"], "gf_range")
    if "from_ty: U32, to_ty: I32" not in skip:
        raise ValueError("cast_sign_range kill switch did not restore the Cast")
    if "op: Ult" not in skip:
        raise ValueError("cast_sign_range kill switch lost the unsigned compare")


def check_mechanisms(assemblies, irs, verbose=True):
    on = assemblies["on"]
    for name in ("gf_cmp32", "gf_cmp64", "gf_abs"):
        if any(re.match(r"set\w*\b", line) for line in body(on, name)):
            raise ValueError(f"Boolean roundtrip survives: {name}: {body(on, name)}")
    range_body = body(on, "gf_range")
    if any(line.startswith("cmov") for line in range_body):
        raise ValueError("signed lower range guard survives")
    if not any(line.startswith("setb") for line in range_body):
        raise ValueError("unsigned range compare mechanism missing")
    phi = "\n".join(body(on, "gf_phi"))
    if "gf_names" in phi or "gf_cleanup" not in phi or "gf_lock" not in phi:
        raise ValueError(f"Phi observation/call-effect mechanism missing: {phi}")
    shift = body(on, "gf_shift")
    if not any(line.startswith("test") for line in shift) or not shift[-2].startswith("cmov"):
        raise ValueError(f"shift flags or terminal select mechanism missing: {shift}")
    for label in assemblies.keys() - {"on", "skip"}:
        for name in ("gf_cmp32", "gf_cmp64", "gf_range", "gf_phi"):
            old, new = body(assemblies[label], name), body(on, name)
            if len(new) >= len(old):
                raise ValueError(f"{name}: no mechanism reduction against {label}: {len(old)} -> {len(new)}")
            if verbose:
                print(f"{name}: {len(old)} -> {len(new)} instructions versus {label}")
    skip_range = body(assemblies["skip"], "gf_range")
    if any(line.startswith("cmov") for line in skip_range):
        raise ValueError("cast-sign-range kill switch still has a signed guard")
    if not any(line.startswith("setb") for line in skip_range):
        raise ValueError("cast-sign-range kill switch lost the unsigned compare")
    if len(skip_range) != len(range_body):
        raise ValueError("cast-sign-range kill switch changed the range instruction count")
    check_ir_mechanisms(irs)


def range_oracle_checksum():
    """Independent 64-bit oracle for the generated runtime fixture."""
    s = 1469598103934665603
    for x in RANGE_XS:
        for y in RANGE_YS:
            bit = int(x >= 0 and x < (y & 15))
            value = bit + 0x9e3779b97f4a7c15 + ((s << 6) & MASK64) + (s >> 2)
            s ^= value & MASK64
    return f"{s:016x}\n".encode()


def range_oracle_source():
    xs = ", ".join(str(x) for x in RANGE_XS)
    ys = ", ".join(f"{y}u" for y in RANGE_YS)
    return f"""\
#include <stdint.h>
#include <stdio.h>

static __attribute__((noinline)) unsigned
gf_range(int x, unsigned y) {{
    return (long long)x >= 0 && (long long)x < (int)(y & 15);
}}

int main(void) {{
    static const int xs[] = {{{xs}}};
    static const unsigned ys[] = {{{ys}}};
    uint64_t sum = 1469598103934665603ULL;
    for (unsigned i = 0; i < sizeof xs / sizeof xs[0]; i++) {{
        for (unsigned j = 0; j < sizeof ys / sizeof ys[0]; j++) {{
            uint64_t bit = (uint64_t)gf_range(xs[i], ys[j]);
            sum ^= bit + 0x9e3779b97f4a7c15ULL + (sum << 6) + (sum >> 2);
        }}
    }}
    printf("%016llx\\n", (unsigned long long)sum);
    return 0;
}}
"""


def asm_function(name, lines):
    return f"{name}:\n" + "".join(f"  {line}\n" for line in lines) + f"  .size {name}, .-{name}\n"


def ir_function_fixture(lines):
    return "=== IR gf_range ===\n" + "\n".join(lines) + "\n=== IR end ===\n"


def synthetic_dataset():
    def repeats(prefix, n):
        return [f"{prefix}{i}" for i in range(n)]
    on = {
        "gf_cmp32": ["nop"] * 5 + ["ret"],
        "gf_cmp64": ["nop"] * 5 + ["ret"],
        "gf_abs": ["nop"] * 5 + ["ret"],
        "gf_range": ["andl $15, %esi", "xorl %eax, %eax", "cmpl %esi, %edi", "setb %al", "ret"],
        "gf_phi": ["call gf_cleanup", "movq gf_lock(%rip), %rax"] + ["nop"] * 10 + ["ret"],
        "gf_shift": ["testl %edi, %edi", "movl %esi, %eax", "cmovne %edx, %eax", "ret"],
    }
    off = {
        "gf_cmp32": ["nop"] * 8 + ["ret"],
        "gf_cmp64": ["nop"] * 8 + ["ret"],
        "gf_range": ["andl $15, %esi", "cmpl %esi, %edi", "setl %r8b", "movzbl %r8b, %r9d", "testl %edi, %edi", "movl $0, %eax", "cmovgeq %r9, %rax", "ret"],
        "gf_phi": ["call gf_cleanup", "movq gf_lock(%rip), %rax"] + ["nop"] * 18 + ["ret"],
    }
    skip = {"gf_range": list(on["gf_range"])}
    assemblies = {
        "on": "\n".join(asm_function(name, lines) for name, lines in on.items()),
        "off": "\n".join(asm_function(name, lines) for name, lines in off.items()),
        "skip": "\n".join(asm_function(name, lines) for name, lines in skip.items()),
    }
    irs = {
        "on": ir_function_fixture([
            "  block 0 (label 7):",
            "    ParamRef { dest: Value(2), param_idx: 0, ty: I32 }",
            "    BinOp { dest: Value(14), op: And, lhs: Value(Value(3)), rhs: Const(I64(15)), ty: U32 }",
            "    Cmp { dest: Value(22), op: Ult, lhs: Value(Value(2)), rhs: Value(Value(14)), ty: I32 }",
            "    term: Return(Some(Value(Value(20))))",
        ]),
        "off": ir_function_fixture([
            "    Cmp { dest: Value(7), op: Sge, lhs: Value(Value(2)), rhs: Const(I32(0)), ty: I32 }",
            "    Cmp { dest: Value(17), op: Slt, lhs: Value(Value(2)), rhs: Value(Value(14)), ty: I32 }",
            "    Select { dest: Value(21), cond: Value(Value(7)), true_val: Value(Value(18)), false_val: Const(I64(0)), ty: I64 }",
        ]),
        "skip": ir_function_fixture([
            "    Cast { dest: Value(15), src: Value(Value(14)), from_ty: U32, to_ty: I32 }",
            "    Cmp { dest: Value(22), op: Ult, lhs: Value(Value(2)), rhs: Value(Value(15)), ty: I32 }",
        ]),
    }
    return (on, off, skip, assemblies, irs)


def self_test():
    on, off, skip, assemblies, irs = synthetic_dataset()
    check_mechanisms(assemblies, irs, verbose=False)

    mutations = []

    def add(name, mutate):
        mutations.append((name, mutate))

    def on_line(name, index, line):
        def mutate(bodies, ir_bodies):
            bodies[0][name][index] = line
        return mutate

    add("cmp32 setcc", on_line("gf_cmp32", 0, "sete %al"))
    add("cmp64 setcc", on_line("gf_cmp64", 0, "setne %al"))
    add("abs setcc", on_line("gf_abs", 0, "setl %al"))
    add("range cmov", on_line("gf_range", 3, "cmovgeq %r9, %rax"))
    add("range no setb", on_line("gf_range", 3, "nop"))

    def phi_cleanup(bodies, ir_bodies):
        bodies[0]["gf_phi"][0] = "nop"
    add("phi missing cleanup", phi_cleanup)

    def phi_lock(bodies, ir_bodies):
        bodies[0]["gf_phi"][1] = "nop"
    add("phi missing lock", phi_lock)

    def phi_names(bodies, ir_bodies):
        bodies[0]["gf_phi"][2] = "movq gf_names(%rip), %rax"
    add("phi names leak", phi_names)
    add("shift no test", on_line("gf_shift", 0, "nop"))
    add("shift no terminal cmov", on_line("gf_shift", 2, "movl %edx, %eax"))

    def no_reduction(bodies, ir_bodies):
        bodies[1]["gf_cmp32"] = ["nop"] * 4 + ["ret"]
    add("no instruction reduction", no_reduction)

    def skip_cmov(bodies, ir_bodies):
        bodies[2]["gf_range"][3] = "cmovgeq %r9, %rax"
    add("skip has cmov", skip_cmov)

    def ir_replace(label, old, new):
        def mutate(bodies, ir_bodies):
            ir_bodies[label] = ir_bodies[label].replace(old, new)
        return mutate

    add("IR Ult->Slt", ir_replace("on", "op: Ult", "op: Slt"))
    add("IR Sge survives", ir_replace("on", "op: Ult", "op: Sge\n    op: Ult"))
    add("IR Select survives", ir_replace("on", "op: Ult", "Select {\n    op: Ult"))
    add("IR Cast survives", ir_replace("on", "op: Ult", "from_ty: U32, to_ty: I32\n    op: Ult"))
    add("kill switch Cast missing", ir_replace("skip", "from_ty: U32, to_ty: I32", "from_ty: U32, to_ty: U32"))
    add("disabled Sge missing", ir_replace("off", "op: Sge", "op: Eq"))
    add("disabled Ult leaked", ir_replace("off", "op: Slt", "op: Ult"))

    for name, mutate in mutations:
        bodies = ({k: list(v) for k, v in on.items()},
                  {k: list(v) for k, v in off.items()},
                  {k: list(v) for k, v in skip.items()})
        ir_bodies = dict(irs)
        mutate(bodies, ir_bodies)
        candidate = {
            "on": "\n".join(asm_function(n, l) for n, l in bodies[0].items()),
            "off": "\n".join(asm_function(n, l) for n, l in bodies[1].items()),
            "skip": "\n".join(asm_function(n, l) for n, l in bodies[2].items()),
        }
        try:
            check_mechanisms(candidate, ir_bodies, verbose=False)
        except ValueError:
            continue
        raise AssertionError(f"self-test mutation was accepted: {name}")

    # The authenticated vendor oracle is pinned by a pure predicate too: an
    # older ICX or a doctored version string must not silently turn the
    # four-compiler runtime comparison into a three-compiler one.
    if not icx_identity_ok(
        "Intel(R) oneAPI DPC++/C++ Compiler 2026.2.0 (2026.2.0.20261001)"
    ):
        raise AssertionError("ICX 2026.2 identity predicate rejected the pinned release")
    if icx_identity_ok("Intel(R) oneAPI DPC++/C++ Compiler 2025.2.0"):
        raise AssertionError("ICX identity predicate accepted an obsolete release")
    print(f"PASS guarded folds self-test: {len(mutations) + 2} verdict mutations rejected")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    ap.add_argument("--clang")
    ap.add_argument("--icx", help="ICX 2026.2 binary for the fourth runtime oracle")
    ap.add_argument("--require-icx", action="store_true",
                    help="fail closed unless the pinned ICX 2026.2 oracle ran")
    ap.add_argument("--baseline")
    ap.add_argument("--linker")
    ap.add_argument("--baseline-linker")
    ap.add_argument("--self-test", action="store_true",
                    help="run compiler-free verdict mutations and exit")
    ap.add_argument("--allow-gcc16-snapshot", action="store_true",
                    help="explicit supplemental snapshot oracle; never normative GCC 16.2 evidence")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return
    self_test()
    # The oracle only produces the expected program output; every mechanism
    # assertion below compares this compiler against itself (pass on vs
    # off), so a snapshot oracle cannot change any verdict -- but the
    # strict default stays, and CI opts in explicitly.
    gcc_version = run([args.gcc, "-dumpfullversion"]).decode().strip()
    report_gcc_oracle(args.gcc, gcc_version,
                      gcc_oracle_identity(gcc_version, args.allow_gcc16_snapshot))
    if args.clang and not re.search(r"\b23\.1\.5\b", run([args.clang, "--version"]).decode()):
        raise ValueError("Clang 23.1.5 required")
    if args.require_icx and not args.icx:
        raise ValueError("--require-icx but no --icx oracle was supplied")
    if args.icx:
        icx = subprocess.run([args.icx, "--version"], capture_output=True,
                             timeout=30, text=True, errors="replace")
        icx_text = (icx.stdout or "") + (icx.stderr or "")
        if icx.returncode or not icx_identity_ok(icx_text):
            raise ValueError("ICX 2026.2 required")
        version = ICX_VERSION.search(icx_text).group(0)
        print(f"oracle: icx: {version}")
    env = os.environ | {"CCC_VERIFY_IR": "abort", "CCC_VERIFY_REGALLOC": "1"}
    with tempfile.TemporaryDirectory() as tmp:
        p = Path(tmp)
        sources = [str(SOURCE), str(HELPERS)]
        run([args.gcc, "-O2", "-Wall", "-Wextra", "-Werror", *sources, "-o", str(p / "gcc")])
        expected = run([str(p / "gcc")])
        if args.clang:
            run([args.clang, "-O2", "-Wall", "-Wextra", "-Werror", *sources, "-o", str(p / "clang")])
            if run([str(p / "clang")]) != expected:
                raise ValueError("oracle disagreement")
        for flags in [[o] for o in ("-O0", "-O1", "-O2", "-O3", "-Os")] + [["-O3", "-g"], ["-O2", "-march=x86-64-v3"]]:
            run([args.compiler, *flags, "-Wall", "-Wextra", "-Werror", *sources, "-o", str(p / "native")], env=env)
            if run([str(p / "native")]) != expected:
                raise ValueError(f"output mismatch: {flags}")
        for disabled in ("bit_idioms", "peephole", ""):
            settings = env | ({"CCC_NO_PEEPHOLE": "1"} if disabled == "peephole" else {"CCC_DISABLE_PASSES": disabled})
            run([args.compiler, "-O2", *sources, "-o", str(p / "off")], env=settings)
            if run([str(p / "off")]) != expected:
                raise ValueError(f"disabled output mismatch: {disabled}")

        # Generated full-domain gf_range oracle: Python is the independent
        # authority; GCC 16.2, Clang 23.1.5 and LCCC ON/OFF/kill-switch must
        # all agree. This covers negative x, y&15 boundaries and U32 y values
        # with the sign bit set, where the old exact-type fact lookup regressed.
        range_src = p / "range_oracle.c"
        range_src.write_text(range_oracle_source())
        range_expected = range_oracle_checksum()
        range_compilers = [("gcc16", args.gcc, env)]
        if args.clang:
            range_compilers.append(("clang23", args.clang, env))
        if args.icx:
            range_compilers.append(("icx", args.icx, env))
        range_compilers.extend([
            ("lccc-on", args.compiler, env),
            ("lccc-bit-idioms-off", args.compiler, env | {"CCC_DISABLE_PASSES": "bit_idioms"}),
            ("lccc-cast-fold-off", args.compiler, env | {"CCC_SIMPLIFY_SKIP": "cast_sign_range"}),
        ])
        for label, compiler, settings in range_compilers:
            binary = p / f"range-{label}"
            run([compiler, "-O2", "-Wall", "-Wextra", "-Werror", str(range_src), "-o", str(binary)], env=settings)
            if run([str(binary)]) != range_expected:
                raise ValueError(f"gf_range runtime oracle mismatch: {label}")
        for level in ("-O0", "-O1", "-O2", "-O3", "-Os"):
            binary = p / f"range-lccc{level}"
            run([args.compiler, level, "-Wall", "-Wextra", "-Werror", str(range_src), "-o", str(binary)], env=env)
            if run([str(binary)]) != range_expected:
                raise ValueError(f"gf_range runtime oracle mismatch: LCCC {level}")

        assemblies = {}
        irs = {}
        specs = [
            ("on", args.compiler, env),
            ("off", args.compiler, env | {"CCC_DISABLE_PASSES": "bit_idioms"}),
            ("skip", args.compiler, env | {"CCC_SIMPLIFY_SKIP": "cast_sign_range"}),
        ] + ([("before", args.baseline, env)] if args.baseline else [])
        for label, compiler, settings in specs:
            asm_path = p / f"{label}.s"
            irs[label] = run_ir(
                [compiler, "-O2", "-march=x86-64-v3", "-S", str(SOURCE), "-o", str(asm_path)],
                env=settings | {"LCCC_DUMP_IR": "1"},
            )
            assemblies[label] = asm_path.read_text()
        check_mechanisms(assemblies, irs)

        start = p / "start.c"
        start.write_text('int main(void); int printf(const char*s,...){(void)s;return 0;}\n'
                         'void _start(void){int c=main();__asm__ volatile("movl %0, %%ebx; movl $1, %%eax; int $0x80"::"r"(c):"eax","ebx");__builtin_unreachable();}\n')
        # Execute real 32-bit code without requiring a 32-bit libc. The native
        # checksum is retained; only the guard-page setup becomes a stack array.
        for label, compiler, settings in [("gcc", args.gcc, env), ("on", args.compiler, env), ("off", args.compiler, env | {"CCC_DISABLE_PASSES": "bit_idioms"})]:
            binary = p / f"i686-{label}"
            run([compiler, "-m32", "-O2", "-DGF_FREESTANDING", "-fno-pie", "-fno-stack-protector", "-ffreestanding", "-nostdlib", "-static", *sources, str(start), "-o", str(binary)], env=settings)
            if binary.read_bytes()[4] != 1:
                raise ValueError("not an ELF32 executable")
            run([str(binary)])
    # Compiler-independent ELF metadata mechanism. The runtime linker suite
    # separately throws/catches both types, including an uncaught inner type.
    linker = args.linker or str(Path(args.compiler).parent / "lccc-ld")
    with tempfile.TemporaryDirectory() as tmp:
        p = Path(tmp)
        source = SOURCE.with_name("eh.cpp")
        def addresses(binary):
            result = {}
            for line in run(["nm", "-D", str(binary)]).decode().splitlines():
                fields = line.split()
                if len(fields) == 3 and fields[2] in ("guard_int", "guard_long"):
                    result[fields[2]] = int(fields[0], 16)
            if len(result) != 2:
                raise ValueError("missing ELF identity fixture symbols")
            return result
        for identical in (False, True):
            obj = p / "eh.o"
            run([args.gcc, "-x", "c++", "-O2", "-fPIC", "-fno-ipa-icf", "-fno-reorder-blocks-and-partition", "-ffunction-sections", *(["-DGF_IDENTICAL_CATCH"] if identical else []), "-c", str(source), "-o", str(obj)])
            run([linker, "-shared", "--icf=all", str(obj), "-o", str(p / "eh.so")])
            symbols = addresses(p / "eh.so")
            if (symbols["guard_int"] == symbols["guard_long"]) != identical:
                raise ValueError(f"unwind identity error: identical={identical}: {symbols}")
            if args.baseline_linker and not identical:
                run([args.baseline_linker, "-shared", "--icf=all", str(obj), "-o", str(p / "before.so")])
                symbols = addresses(p / "before.so")
                if symbols["guard_int"] != symbols["guard_long"]:
                    raise ValueError("text-only ICF baseline failure not reproduced")
    print("PASS ELF: identical unwind folds; different catch types remain distinct")
    print("PASS gf_range oracle: Python/GCC16/Clang23/LCCC ON/OFF/kill-switch, 192 boundary pairs")
    print("PASS guarded folds: native/i686/guard-page/disabled/IR/RA/mechanism controls")


if __name__ == "__main__":
    main()
