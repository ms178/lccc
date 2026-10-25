#!/usr/bin/env python3
"""guarded folds: scalar/CFG contracts, guard pages, i686 and disabled controls."""
import argparse
import os
from pathlib import Path
import re
import tempfile
from check_llvm_missed_zero_choice import (body, gcc_oracle_identity,
                                           report_gcc_oracle, run)

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/llvm_guarded_folds/driver.c"
HELPERS = SOURCE.with_name("helpers.c")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    ap.add_argument("--clang")
    ap.add_argument("--baseline")
    ap.add_argument("--linker")
    ap.add_argument("--baseline-linker")
    ap.add_argument("--allow-gcc16-snapshot", action="store_true",
                    help="explicit supplemental snapshot oracle; never normative GCC 16.2 evidence")
    args = ap.parse_args()
    # The oracle only produces the expected program output; every mechanism
    # assertion below compares this compiler against itself (pass on vs
    # off), so a snapshot oracle cannot change any verdict -- but the
    # strict default stays, and CI opts in explicitly.
    gcc_version = run([args.gcc, "-dumpfullversion"]).decode().strip()
    report_gcc_oracle(args.gcc, gcc_version,
                      gcc_oracle_identity(gcc_version, args.allow_gcc16_snapshot))
    if args.clang and not re.search(r"\b23\.1\.3\b", run([args.clang, "--version"]).decode()):
        raise ValueError("Clang 23.1.4 required")
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
        assemblies = {}
        for label, compiler, settings in [("on", args.compiler, env), ("off", args.compiler, env | {"CCC_DISABLE_PASSES": "bit_idioms"})] + ([("before", args.baseline, env)] if args.baseline else []):
            run([compiler, "-O2", "-march=x86-64-v3", "-S", str(SOURCE), "-o", str(p / f"{label}.s")], env=settings)
            assemblies[label] = (p / f"{label}.s").read_text()
        on = assemblies["on"]
        for name in ("gf_cmp32", "gf_cmp64", "gf_abs"):
            if any(re.match(r"set\w*\b", line) for line in body(on, name)):
                raise ValueError(f"Boolean roundtrip survives: {name}: {body(on, name)}")
        if any(line.startswith("cmov") for line in body(on, "gf_range")):
            raise ValueError("signed lower range guard survives")
        phi = "\n".join(body(on, "gf_phi"))
        if "gf_names" in phi or "gf_cleanup" not in phi or "gf_lock" not in phi:
            raise ValueError(f"Phi observation/call-effect mechanism missing: {phi}")
        shift = body(on, "gf_shift")
        if not any(line.startswith("test") for line in shift) or not shift[-2].startswith("cmov"):
            raise ValueError(f"shift flags or terminal select mechanism missing: {shift}")
        for label in assemblies.keys() - {"on"}:
            for name in ("gf_cmp32", "gf_cmp64", "gf_range", "gf_phi"):
                old, new = body(assemblies[label], name), body(on, name)
                if len(new) >= len(old):
                    raise ValueError(f"{name}: no mechanism reduction against {label}: {len(old)} -> {len(new)}")
                print(f"{name}: {len(old)} -> {len(new)} instructions versus {label}")
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
    print("PASS guarded folds: native/i686/guard-page/disabled/IR/RA/mechanism controls")


if __name__ == "__main__":
    main()
