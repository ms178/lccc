#!/usr/bin/env python3
"""scalar facts: native/IR/RA checks, mechanism controls and an optional saved baseline."""

import argparse
import os
from pathlib import Path
import re
import tempfile
from check_llvm_missed_zero_choice import (body, gcc_oracle_identity,
                                           report_gcc_oracle, run)

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/llvm_scalar_facts.c"
NEW = (
    "sf_bits",
    "sf_sum",
    "sf_range",
    "sf_shared",
    "sf_masked",
    "sf_select",
    "sf_trip",
    "sf_guard",
    "sf_lowone",
    "sf_boolor",
    "sf_sub",
    "sf_even",
    "sf_signed_select",
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc"))
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--allow-gcc16-snapshot", action="store_true",
                        help="explicit supplemental snapshot oracle; never normative GCC 16.2 evidence")
    parser.add_argument("--baseline")
    parser.add_argument("--clang", help="optional exact Clang 23.1.3 oracle")
    args = parser.parse_args()
    # The oracle only produces the expected program output; every mechanism
    # assertion below compares this compiler against itself (pass on vs
    # off), so a snapshot oracle cannot change any verdict -- but the
    # strict default stays, and CI opts in explicitly.
    gcc_version = run([args.gcc, "-dumpfullversion"]).decode().strip()
    report_gcc_oracle(args.gcc, gcc_version,
                      gcc_oracle_identity(gcc_version, args.allow_gcc16_snapshot))
    if args.clang and not re.search(
        r"\b23\.1\.3\b", run([args.clang, "--version"]).decode()
    ):
        raise ValueError("Clang 23.1.3 required")
    env = os.environ | {"CCC_VERIFY_IR": "abort", "CCC_VERIFY_REGALLOC": "1"}
    with tempfile.TemporaryDirectory() as directory:
        p = Path(directory)
        run(
            [
                args.gcc,
                "-O2",
                "-Wall",
                "-Wextra",
                "-Werror",
                str(SOURCE),
                "-o",
                str(p / "gcc"),
            ]
        )
        expected = run([str(p / "gcc")])
        if args.clang:
            run(
                [
                    args.clang,
                    "-O2",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(SOURCE),
                    "-o",
                    str(p / "clang"),
                ]
            )
            if run([str(p / "clang")]) != expected:
                raise ValueError("independent oracle disagreement")
        for flags in [[o] for o in ("-O0", "-O1", "-O2", "-O3", "-Os")] + [
            ["-O3", "-g"],
            ["-O2", "-fwrapv"],
        ]:
            run(
                [
                    args.compiler,
                    *flags,
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(SOURCE),
                    "-o",
                    str(p / "native"),
                ],
                env=env,
            )
            if run([str(p / "native")]) != expected:
                raise ValueError(f"output mismatch: {flags}")
        run(
            [args.compiler, "-O2", str(SOURCE), "-o", str(p / "off")],
            env=env | {"CCC_DISABLE_PASSES": "bit_idioms"},
        )
        if run([str(p / "off")]) != expected:
            raise ValueError("disabled output mismatch")
        for label, compiler, settings in [
            ("on", args.compiler, env),
            ("off", args.compiler, env | {"CCC_DISABLE_PASSES": "bit_idioms"}),
        ] + ([("before", args.baseline, env)] if args.baseline else []):
            run(
                [compiler, "-O2", "-S", str(SOURCE), "-o", str(p / f"{label}.s")],
                env=settings,
            )
        on = (p / "on.s").read_text()
        # Each check names the eliminated computation, not a register spelling
        # or a globally fixed instruction count.
        forbidden = {
            "sf_bits": r"or[lq]?\b",
            "sf_sum": r"(?:add|sub)[lq]?\b",
            "sf_range": r"and[lq]?\b",
            "sf_masked": r"shr[lq]?\b",
            "sf_select": r"cmov\w*\b",
            "sf_trip": r"add[lq]?\b",
            "sf_guard": r"and[lq]?\b",
            "sf_lowone": r"(?:neg|sub|blsi)[lq]?\b",
            "sf_boolor": r"or[lq]?\b",
            "sf_sub": r"(?:add|sub)[lq]?\b",
            "sf_even": r"and[lq]?\b",
            "sf_signed_select": r"cmov\w*\b",
        }
        for name, pattern in forbidden.items():
            if any(re.match(pattern, line) for line in body(on, name)):
                raise ValueError(
                    f"{name}: eliminated computation survives: {body(on, name)}"
                )
        for label in ("off", "before") if args.baseline else ("off",):
            other = (p / f"{label}.s").read_text()
            for name in NEW:
                yes, no = body(on, name), body(other, name)
                if name == "sf_shared":
                    # Keeping x live for the independent result can cost moves:
                    # total instruction count ties on the current allocator.
                    # Require elimination of the quotient-dependent multiply,
                    # not an unjustified total-count reduction.
                    scaled_quotient = r"imul[lq]?\s+\$12\b"
                    if any(re.match(scaled_quotient, line) for line in yes) or not any(
                        re.match(scaled_quotient, line) for line in no
                    ):
                        raise ValueError(
                            "shared remainder: quotient-scale mechanism missing"
                        )
                elif len(yes) >= len(no):
                    raise ValueError(
                        f"{name}: no structural improvement versus {label}: {len(yes)} >= {len(no)}"
                    )
                print(f"{name}: {len(no)} -> {len(yes)} instructions versus {label}")
        # Exercise the same scalar contracts on real i686 machine code without
        # depending on an installed 32-bit libc or conflating ABI entry setup.
        freestanding = (
            SOURCE.read_text()
            .replace(
                "#include <stdint.h>",
                "typedef unsigned long long uint64_t;\n"
                "#define UINT64_C(x) x##ULL\n#define UINT64_MAX (~(uint64_t)0)",
            )
            .replace("int main(void)", "int cohort_main(void)")
        )
        freestanding += (
            "\nvoid _start(void) { int code = cohort_main(); "
            '__asm__ volatile("movl %0, %%ebx; movl $1, %%eax; int $0x80" '
            ':: "r"(code) : "eax", "ebx"); __builtin_unreachable(); }\n'
        )
        narrow = p / "i686.c"
        narrow.write_text(freestanding)
        for compiler, settings in [
            (args.gcc, env),
            (args.compiler, env),
            (args.compiler, env | {"CCC_DISABLE_PASSES": "bit_idioms"}),
        ] + ([(args.baseline, env)] if args.baseline else []):
            run(
                [
                    compiler,
                    "-m32",
                    "-O2",
                    "-nostdlib",
                    "-static",
                    "-Wl,-e,_start",
                    str(narrow),
                    "-o",
                    str(p / "i686"),
                ],
                env=settings,
            )
            run([str(p / "i686")])
        hot = ROOT / "tests/regression/hot_web_callee_saved.c"
        for compiler in [args.gcc] + ([args.clang] if args.clang else []):
            run(
                [
                    compiler,
                    "-O2",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(hot),
                    "-o",
                    str(p / "hot-ref"),
                ]
            )
            run([str(p / "hot-ref")])
        for flags in [[o] for o in ("-O0", "-O1", "-O2", "-O3", "-Os")] + [
            ["-O3", "-g"],
            ["-O2", "-fwrapv"],
        ]:
            for disabled in [False, True]:
                settings = env | ({"CCC_NO_HOT_WEB_STEAL": "1"} if disabled else {})
                run(
                    [args.compiler, *flags, str(hot), "-o", str(p / "hot")],
                    env=settings,
                )
                run([str(p / "hot")])
        for cap in ["0", "1", "2", "3"]:
            run(
                [args.compiler, "-O2", str(hot), "-o", str(p / "hot")],
                env=env | {"CCC_HOT_WEB_STEAL": cap},
            )
            run([str(p / "hot")])
        # The knob must stay wired to a live mechanism: an A/B control that
        # cannot change any output is a knob that silently stopped meaning
        # anything.  This used to compare the hot_web fixture's `exercise`
        # body, but the two-wave Phase-1 allocation (mandatory call-spanning
        # before optional hot-loop) now homes this shape's webs before the
        # steal is consulted, so the bodies are identical for a good reason
        # -- verified: both settings keep the callee-saved homes, and the
        # mechanism outcome for this shape is pinned by
        # check_hot_loop_register_homes.sh.  The discrimination witness
        # therefore moved to a shape where the steal is still the deciding
        # mechanism (measured: it is the one file in the 67-program
        # benchmark corpus whose -O2 codegen the knob changes).
        witness = ROOT / "tests/benchmark/programs/zlib_ng_adler32_combine.c"
        for disabled in [False, True]:
            run(
                [
                    args.compiler,
                    "-O2",
                    "-S",
                    str(witness),
                    "-o",
                    str(p / f"hot-{disabled}.s"),
                ],
                env=env | ({"CCC_NO_HOT_WEB_STEAL": "1"} if disabled else {}),
            )
        if (p / "hot-False.s").read_text() == (p / "hot-True.s").read_text():
            raise ValueError(
                "hot web: enable/disable does not change the pressure witness"
            )
    print("scalar-facts: PASS (native, disabled controls, IR/RA verification and mechanisms)")


if __name__ == "__main__":
    main()
