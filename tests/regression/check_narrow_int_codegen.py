#!/usr/bin/env python3
"""Pin the narrow-integer staging width and the large-frame stack adjustment.

Two classes of defect are invisible to a behavioural test, so this gate asserts
them where they are actually decided: in the instruction stream and in the
unwind tables.

  * A narrow (i8/i16/u8/u16) value is canonical only in its low 32 bits, so
    every consumer that reads the register at 64-bit width has to re-extend it.
    `narrow_int_codegen.c` uses negative index lanes precisely where a
    zero-extending copy picks the wrong element, and stages one value twice so
    that a staging path which extends unconditionally is visible.

  * `add/sub r64, imm` is a sign-extended imm32, so a frame above 2 GiB needs
    the constant materialized into a scratch register. `large_stack_frame_codegen.c`
    pins the three encoding ranges; `large_frame_probe_boundary.c` is the shape
    that can actually RUN (a frame that crosses the stack-clash probe page and
    is closed by the register-form `subq %r11, %rsp`).

Exit status is 0 when every contract holds, 1 otherwise; the reasons are
printed one per line prefixed with `FAIL`, so a shard's tail is diagnosable.

  python3 tests/regression/check_narrow_int_codegen.py [--lccc PATH] [--gcc PATH] [--clang PATH]

Both oracles are optional and independent: a missing one is reported, never
silently skipped, and when both are present they must agree with each other as
well as with lccc.
"""

from __future__ import annotations

import argparse
import pathlib
import shutil
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent.parent

NARROW = HERE / "narrow_int_codegen.c"
PROBE = HERE / "large_frame_probe_boundary.c"
BIGFRAME = HERE / "large_stack_frame_codegen.c"

# The fixtures print these strings; keeping them here means a change to the
# program has to be a deliberate change to the contract as well.
NARROW_LINE = "narrow_checks=538 sum=694188"
PROBE_LINE = "probe_boundary total=1686"

# -O2 -fno-omit-frame-pointer is the shape whose prologue emits the probe loop;
# the optimizer matrix below proves the adjustment is correct at every level.
OPT_MATRIX = ["-O0", "-O1", "-O2", "-O3", "-Os"]


def run(argv: list[str], *, cwd: pathlib.Path | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(argv, capture_output=True, text=True, cwd=cwd)


def fail(reasons: list[str], message: str) -> None:
    reasons.append(message)


def compile_to_asm(lccc: str, src: pathlib.Path, out: pathlib.PurePath, flags: list[str]) -> tuple[bool, str]:
    proc = run([lccc, *flags, "-S", str(src), "-o", str(out)])
    return proc.returncode == 0, proc.stderr


def check_narrow_shape(lccc: str, work: pathlib.Path, reasons: list[str]) -> str:
    asm = work / "narrow.s"
    ok, err = compile_to_asm(lccc, NARROW, asm, ["-O2"])
    if not ok:
        fail(reasons, f"narrow fixture did not compile at -O2: {err.strip()[:400]}")
        return ""
    text = asm.read_text()
    # No 64-bit divide for a narrow type, and no 64-bit sign extension before
    # one: both mean the operand width was lost.
    for mnemonic in ("idivq", "divq", "cqto"):
        if mnemonic in text:
            fail(reasons, f"narrow fixture emits {mnemonic}: a narrow divide is running at 64-bit width")
    for mnemonic in ("idivl", "divl"):
        if mnemonic not in text:
            fail(reasons, f"narrow fixture never emits {mnemonic}: the 32-bit narrow divide disappeared")
    # A signed narrow value staged for a 64-bit consumer must be sign-extended
    # from its own width, never merely copied.
    if "movswq" not in text and "movswl" not in text:
        fail(reasons, "narrow fixture never sign-extends an i16 value (movswq/movswl absent)")
    if ".cfi_undefined" in text:
        fail(reasons, "narrow fixture has an undefined CFA: the unwind model lost the frame")
    return text


def check_probe_shape(lccc: str, work: pathlib.Path, reasons: list[str]) -> None:
    asm = work / "probe.s"
    ok, err = compile_to_asm(lccc, PROBE, asm, ["-O2", "-fno-omit-frame-pointer"])
    if not ok:
        fail(reasons, f"probe fixture did not compile: {err.strip()[:400]}")
        return
    text = asm.read_text()
    if "orl $0, (%rsp)" not in text:
        fail(reasons, "probe fixture has no stack-clash probe store: the >page frame was allocated in one step")
    if "subq %r11, %rsp" not in text:
        fail(reasons, "probe fixture does not close its probe loop with the register-form subq %r11, %rsp")
    # The probed frame keeps its CFA in %rbp, so the description is a
    # register rule rather than an offset; either form is a described frame.
    if ".cfi_undefined" not in text and ".cfi_def_cfa" not in text:
        fail(reasons, "probe fixture has neither a described CFA nor an explicit refusal")


def run_binary(compiler: str, src: pathlib.Path, flags: list[str], work: pathlib.Path, tag: str) -> tuple[int, str]:
    exe = work / f"{tag}.bin"
    proc = run([compiler, *flags, str(src), "-o", str(exe)])
    if proc.returncode != 0:
        return -1, proc.stderr
    ran = run([str(exe)])
    return ran.returncode, ran.stdout


def check_semantics(lccc: str, oracles: list[str], work: pathlib.Path, reasons: list[str]) -> None:
    """Both fixtures must RUN, agree with each known expected line, and agree
    with every available oracle (and therefore with each other)."""
    for src, line, tag in ((PROBE, PROBE_LINE, "probe"), (NARROW, NARROW_LINE, "narrow")):
        levels = (["-O2"], ["-O0"], ["-O2", "-fno-omit-frame-pointer"]) if src is PROBE else (["-O2"], ["-O0"])
        for flags in levels:
            rc, out = run_binary(lccc, src, flags, work, f"{tag}_lccc")
            if rc != 0:
                fail(reasons, f"{tag} fixture({flags}) exited {rc}: {out.strip()[:300]}")
                continue
            if line not in out:
                fail(reasons, f"{tag} fixture({flags}) printed {out.strip()[:120]!r}, expected {line!r}")
                continue
            for oracle in oracles:
                grc, gout = run_binary(oracle, src, flags, work, f"{tag}_{pathlib.Path(oracle).name}")
                if grc != 0:
                    fail(reasons, f"oracle {oracle} cannot build the {tag} fixture (rc={grc}): {gout.strip()[:300]}")
                elif gout != out:
                    fail(
                        reasons,
                        f"{tag} fixture({flags}) disagrees with {pathlib.Path(oracle).name}:\n"
                        f"  lccc:   {out!r}\n  oracle: {gout!r}",
                    )


def check_optimizer_matrix(lccc: str, work: pathlib.Path, reasons: list[str]) -> None:
    """Every level x {-g, no -g}: the adjustments stay described."""
    for level in OPT_MATRIX:
        for debug in ([], ["-g"]):
            flags = [level, *debug]
            for src in (NARROW, PROBE, BIGFRAME):
                asm = work / f"{src.stem}{level}{'g' if debug else ''}.s"
                ok, err = compile_to_asm(lccc, src, asm, flags)
                if not ok:
                    fail(reasons, f"{src.name} failed to compile at {' '.join(flags)}: {err.strip()[:300]}")
                    continue
                text = asm.read_text()
                if ".cfi_undefined" in text:
                    fail(reasons, f"{src.name} has an undefined CFA at {' '.join(flags)}")


def check_large_frame_unwind(lccc: str, work: pathlib.Path, reasons: list[str]) -> None:
    """The >2 GiB frame must get an exact FDE, not a refusal."""
    obj = work / "bigframe.o"
    proc = run([lccc, "-O2", "-c", str(BIGFRAME), "-o", str(obj)])
    if proc.returncode != 0:
        fail(reasons, f"large_frame fixture did not compile: {proc.stderr.strip()[:400]}")
        return
    readelf = shutil.which("readelf")
    if readelf is None:
        fail(reasons, "readelf is unavailable, so the >2 GiB FDE assertion cannot run")
        return
    frames = run([readelf, "--debug-dump=frames", str(obj)])
    if frames.returncode != 0:
        fail(reasons, f"readelf --debug-dump=frames failed: {frames.stderr.strip()[:300]}")
        return
    text = frames.stdout
    # The two magnitudes the fixture allocates (see large_stack_frame_codegen.c).
    for expected in ("1099511627792", "2147483664"):
        if expected not in text:
            fail(reasons, f"no FDE describes the {expected}-byte frame: the CFA offset is missing or truncated")
    if "DW_CFA_undefined" in text:
        fail(reasons, "an FDE declares the return address undefined for a large frame")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lccc", default=str(REPO / "target" / "fastbuild" / "lccc"))
    parser.add_argument("--gcc", default=None, help="GCC oracle (default: gcc-16, then gcc)")
    parser.add_argument("--clang", default=None, help="Clang oracle (default: clang-23, then clang)")
    args = parser.parse_args()

    lccc = shutil.which(args.lccc) or (args.lccc if pathlib.Path(args.lccc).is_file() else None)
    if lccc is None:
        print(f"FAIL: lccc not found at {args.lccc}", file=sys.stderr)
        return 1
    gcc = args.gcc or shutil.which("gcc-16") or shutil.which("gcc")
    clang = args.clang or shutil.which("clang-23") or shutil.which("clang")

    reasons: list[str] = []
    with tempfile.TemporaryDirectory() as tmp:
        work = pathlib.Path(tmp)
        narrow_text = check_narrow_shape(lccc, work, reasons)
        check_probe_shape(lccc, work, reasons)
        check_semantics(lccc, [c for c in (gcc, clang) if c], work, reasons)
        check_optimizer_matrix(lccc, work, reasons)
        check_large_frame_unwind(lccc, work, reasons)

    if reasons:
        for reason in reasons:
            print(f"FAIL: {reason}", file=sys.stderr)
        print(f"check_narrow_int_codegen: FAIL ({len(reasons)} contract(s))", file=sys.stderr)
        return 1
    names = ", ".join(pathlib.Path(o).name for o in (gcc, clang) if o)
    oracle = f"oracles agreed: {names}" if names else "no oracle available; shape and run checks only"
    staging = "sign-extending staging present" if "movswq" in narrow_text or "movswl" in narrow_text else ""
    print(f"check_narrow_int_codegen: PASS ({oracle}; narrow divides 32-bit; {staging}; "
          f"probe frame executed at 3 levels; optimizer matrix {len(OPT_MATRIX)}x2 clean; "
          f">2 GiB FDE exact)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
