#!/usr/bin/env python3
"""scalar facts: native/IR/RA checks, mechanism controls and an optional saved baseline."""

import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile
from check_llvm_missed_zero_choice import body, run

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
    parser.add_argument("--baseline")
    parser.add_argument("--clang", help="optional exact Clang 23.1.3 oracle")
    args = parser.parse_args()
    if not re.search(r"\b16\.2(?:\.0)?\b", run([args.gcc, "--version"]).decode()):
        raise ValueError("GCC 16.2 required")
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
        # The i686 leg executes real 32-bit code through `int $0x80`.  A host
        # whose kernel or seccomp policy refuses ia32 syscalls kills every
        # probe binary with SIGSYS (-31) before the first cohort
        # instruction, and a host without 32-bit headers cannot build it at
        # all; both are named environmental prerequisites (CI's runner has
        # them), so the leg skips loudly instead of dying in a traceback
        # that names neither the reason nor the fix.  The probe is compiled
        # with the reference GCC so it measures the environment, not this
        # build: a probe that fails under gcc-16 says nothing about lccc.
        probe = p / "probe32.c"
        probe.write_text(
            "void _start(void) {\n"
            '    __asm__ volatile("movl $0, %ebx; movl $1, %eax; int $0x80");\n'
            "    __builtin_unreachable();\n"
            "}\n"
        )
        # Probe with the reference GCC and report the refusal mode by name:
        # a compile failure means the host lacks 32-bit headers/toolchain; a
        # killed probe (SIGSYS, rc -31) means the kernel/seccomp policy
        # refuses ia32 syscalls.  Different prerequisites, different fixes,
        # and neither says anything about the compiler under test.
        probe_rc = subprocess.run(
            [
                args.gcc,
                "-m32",
                "-O2",
                "-nostdlib",
                "-static",
                "-Wl,-e,_start",
                str(probe),
                "-o",
                str(p / "probe32"),
            ],
            capture_output=True,
            timeout=60,
        ).returncode
        if probe_rc == 0:
            probe_rc = subprocess.run(
                [str(p / "probe32")], capture_output=True, timeout=60
            ).returncode
        if probe_rc != 0:
            if probe_rc < 0:
                reason = f"the probe binary was killed by signal {-probe_rc} (ia32 syscalls refused)"
            else:
                reason = (
                    "the reference GCC cannot build 32-bit code here "
                    f"(rc={probe_rc}: no 32-bit headers or toolchain)"
                )
            print(
                f"SKIP: i686 leg -- this host cannot run 32-bit freestanding "
                f"code: {reason}. CI's runner executes ia32 syscalls and runs "
                "the leg."
            )
        else:
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
        # Mechanism witness.  The steal only fires where a callee-saved home
        # is held ENTIRELY by cold, non-loop values: since
        # `37d6130` (protect loop-carried state from the values only read on a
        # cold path) a register co-holding another loop web is refused, so the
        # original witness (hot_web_callee_saved.c) no longer produces a steal
        # -- its six homes are all loop webs, which is the *protected* shape.
        # Asserting "asm differs" on it was therefore an assertion about a
        # mechanism that cannot fire there.  Two contracts replace it:
        #
        #   1. WITNESS: a shape where the steal does fire must change codegen
        #      when the knob is set -- the mechanism is wired, reachable and
        #      disableable.  `cmp_replay_far.c`'s `main` steals 1 web at -O2
        #      (vid 217 -> reg 1, evicting vid 196, cost 12 against a hot
        #      count of 310), measured on the from-source fastbuild of 5e7a31d2.
        #
        #      The previous witness, `cmp_replay_acc_nohome.c`, was retired
        #      DELIBERATELY, and the reason matters: its web is still stolen
        #      (vid 253 -> reg 1, evicting vid 9, cost 3 against a hot count of
        #      20), so the mechanism has not stopped firing on it.  What
        #      changed is the code: the allocator now reaches the same home
        #      without the steal, so the knob no longer changes that file's
        #      assembly.  A witness must change the CODE, not merely the
        #      decision, so it was replaced rather than re-pinned as
        #      "identical" (which would freeze an accident of the current
        #      allocator).
        #   2. PROTECTED: the loop-web fixture must be byte-identical with the
        #      knob off and on -- the policy refuses to evict a register that
        #      co-holds loop state.  If that flips, the protection regressed
        #      and this pin must be re-baselined deliberately, not silently.
        witness = ROOT / "tests/regression/cmp_replay_far.c"
        witness_fn = "main"
        for disabled in [False, True]:
            settings = env | ({"CCC_NO_HOT_WEB_STEAL": "1"} if disabled else {})
            for source, stem in ((hot, "hot"), (witness, "witness")):
                run(
                    [args.compiler, "-O2", "-S", str(source),
                     "-o", str(p / f"{stem}-{disabled}.s")],
                    env=settings,
                )
        if body((p / "witness-False.s").read_text(), witness_fn) == body(
            (p / "witness-True.s").read_text(), witness_fn
        ):
            raise ValueError(
                f"hot web: enable/disable does not change the witness "
                f"({witness.name}:{witness_fn}); the steal still needs a shape "
                f"whose assembly it actually changes -- re-baseline this pin "
                f"deliberately (see the comment above), do not delete it"
            )
        if (p / "hot-False.s").read_text() != (p / "hot-True.s").read_text():
            raise ValueError(
                "hot web: the loop-web protection fixture is no longer "
                "invariant under the knob (hot_web_callee_saved.c): a "
                "callee-saved home co-held by loop state was evicted"
            )
        #   3. OUTCOME: on the protected fixture, with the knob off and on, no
        #      callee-saved register is saved without a body use (a phantom
        #      home) and no hot state is stepped through a stack slot by a
        #      memory-destination ALU op (the slot-resident loop counter).
        #      Byte-identity (2) alone would freeze a BAD allocation as
        #      readily as a good one; this pins its quality.
        alias = {"rbx": "rbx|ebx|bx|bl", "rbp": "rbp|ebp|bp|bpl"}
        rmw = re.compile(
            r"^(?:add|sub|xor|or|and|inc|dec|ro[lr]|sh[lr]|sar)q? "
            r"(?:[^,]+, )?-?\d+\(%rsp\)$"
        )
        for disabled in (False, True):
            text = (p / f"hot-{disabled}.s").read_text()
            for fn in ("exercise", "nested_calls"):
                lines = body(text, fn)
                if not lines:
                    raise ValueError(
                        f"hot web: {fn} body missing (knob off={disabled})"
                    )
                pushed = re.findall(
                    r"^pushq %(rbx|rbp|r1[2-5])$", "\n".join(lines), re.M
                )
                for reg in pushed:
                    pat = re.compile(rf"%(?:{alias.get(reg, reg + '[dwb]?')})\b")
                    if not any(
                        pat.search(line)
                        and not re.match(r"(?:pushq|popq|\.cfi_)", line)
                        for line in lines
                    ):
                        raise ValueError(
                            f"hot web: {fn} saves %{reg} but never uses it "
                            f"(knob off={disabled})"
                        )
                bad = [line for line in lines if rmw.match(line)]
                if bad:
                    raise ValueError(
                        f"hot web: {fn} steps state in a stack slot "
                        f"(knob off={disabled}): {bad[0]}"
                    )
    print("scalar-facts: PASS (native, disabled controls, IR/RA verification and mechanisms)")


if __name__ == "__main__":
    main()
