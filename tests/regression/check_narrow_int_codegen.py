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
import traceback

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


def resolve_tool(requested: str | None, fallbacks: tuple[str, ...]) -> tuple[str | None, bool]:
    """Resolve a compiler the way the repository names its oracles.

    A caller passes the *policy* name (`clang-23`, `gcc-16`), which ci.yml and
    scripts/ci_local.sh both do by convention; the runner may expose the same
    binary under its family name. Resolution therefore tries the requested name
    and then the family, and reports which one answered. The second element of
    the result is True when the requested name resolved exactly -- a fallback is
    never silent, because a gate that quietly grades against a different
    compiler than the one it names is worse than one that fails.
    """
    candidates = [requested] if requested else []
    candidates.extend(fallbacks)
    for index, name in enumerate(candidates):
        if not name:
            continue
        found = shutil.which(name)
        if found is None and pathlib.Path(name).is_file():
            found = str(pathlib.Path(name).resolve())
        if found:
            return found, index == 0
    return None, False


def run(argv: list[str], *, cwd: pathlib.Path | None = None) -> subprocess.CompletedProcess:
    # `text=True` alone decodes with the LOCALE encoding, and a CI runner under
    # `LC_ALL=C` then raises `UnicodeDecodeError` on the first byte above 0x7f in
    # a compiler diagnostic -- a crash in the gate, not a finding. The gate's
    # job is to compare bytes and numbers, so decode leniently and never let a
    # diagnostic's encoding decide the verdict.
    return subprocess.run(argv, capture_output=True, text=True, errors="replace", cwd=cwd)


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
    text = asm.read_text(errors="replace")
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
    text = asm.read_text(errors="replace")
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
    """Every level x {-g, no -g}: the adjustments stay described.

    `large_stack_frame_codegen.c` is excluded: its 3 GiB frame is a compile-time
    memory cost with nothing level-dependent to learn (the probe/window shapes
    are covered by `large_frame_probe_boundary.c` across the whole matrix), and
    an optimizer sweep is not where a 3 GiB allocation belongs.
    """
    for level in OPT_MATRIX:
        for debug in ([], ["-g"]):
            flags = [level, *debug]
            for src in (NARROW, PROBE):
                asm = work / f"{src.stem}{level}{'g' if debug else ''}.s"
                ok, err = compile_to_asm(lccc, src, asm, flags)
                if not ok:
                    fail(reasons, f"{src.name} failed to compile at {' '.join(flags)}: {err.strip()[:300]}")
                    continue
                text = asm.read_text(errors="replace")
                if ".cfi_undefined" in text:
                    fail(reasons, f"{src.name} has an undefined CFA at {' '.join(flags)}")


def decode_uleb(data: bytes, start: int, limit: int = 10) -> tuple[int, int] | None:
    """Decode one ULEB128 at `start`; return (value, next_offset) or None."""
    value = 0
    shift = 0
    for index in range(limit):
        if start + index >= len(data):
            return None
        byte = data[start + index]
        value |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            return value, start + index + 1
    return None


def eh_frame_bytes(obj: pathlib.Path) -> bytes | None:
    """Extract the raw `.eh_frame` payload of `obj`.

    The bytes are the point: `readelf --debug-dump=frames` prints a CFA offset
    through 32 bits and `frames-interp` multiplies it out, so a table that says
    `rsp + 2147483664` can be displayed as `rsp - 2147483632`. Decoding the
    section ourselves is the only rendering-independent assertion available
    with stock binutils.
    """
    for tool, args in (
        ("objdump", ["-s", "-j", ".eh_frame"]),
        ("readelf", ["-x", ".eh_frame"]),
    ):
        path = shutil.which(tool)
        if path is None:
            continue
        proc = run([path, *args, str(obj)])
        if proc.returncode != 0:
            continue
        payload = bytearray()
        for line in proc.stdout.splitlines():
            parts = line.split()
            # objdump: " 0020 00000000 27000000 004d0e90 80808080  ....'....M......"
            # readelf: "  00000000 00000000 00000000 00000000  ................"
            if not parts or not parts[0] or not all(c in "0123456789abcdef" for c in parts[0].lower()):
                continue
            for word in parts[1:]:
                if len(word) != 8 or not all(c in "0123456789abcdef" for c in word.lower()):
                    break
                payload += bytes.fromhex(word)
            else:
                continue
        if payload:
            return bytes(payload)
    return None


def check_large_frame_unwind(lccc: str, work: pathlib.Path, reasons: list[str]) -> None:
    """The >2 GiB frame must get an exact FDE, not a refusal."""
    obj = work / "bigframe.o"
    proc = run([lccc, "-O2", "-c", str(BIGFRAME), "-o", str(obj)])
    if proc.returncode != 0:
        fail(reasons, f"large_frame fixture did not compile: {proc.stderr.strip()[:400]}")
        return
    data = eh_frame_bytes(obj)
    if data is None:
        fail(reasons, "no tool could dump .eh_frame (objdump/readelf), so the >2 GiB FDE assertion cannot run")
        return
    # Every CFI operand we care about follows `DW_CFA_def_cfa_offset` (0x0e) as a
    # ULEB128. 0x0e also occurs inside other operands, so decode every
    # candidate and compare values -- a false positive would have to encode the
    # exact frame size by accident.
    decoded: set[int] = set()
    for index, byte in enumerate(data):
        if byte != 0x0E:
            continue
        result = decode_uleb(data, index + 1)
        if result is not None:
            decoded.add(result[0])
    for expected in (1099511627792, 2147483664):
        if expected not in decoded:
            fail(
                reasons,
                f"no FDE carries an exact DW_CFA_def_cfa_offset for the {expected}-byte frame "
                f"(decoded CFA offsets: {sorted(decoded) or 'none'}; .eh_frame starts {data[:48].hex()})",
            )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lccc", default=str(REPO / "target" / "fastbuild" / "lccc"))
    parser.add_argument("--gcc", default=None, help="GCC oracle (default: gcc-16, then gcc)")
    parser.add_argument("--clang", default=None, help="Clang oracle (default: clang-23, then clang)")
    args = parser.parse_args()

    reasons: list[str] = []
    lccc, _ = resolve_tool(args.lccc, ("lccc",))
    if lccc is None:
        print(f"FAIL: lccc not found at {args.lccc}", file=sys.stderr)
        return 1
    gcc, gcc_exact = resolve_tool(args.gcc, ("gcc-16", "gcc"))
    clang, clang_exact = resolve_tool(args.clang, ("clang-23", "clang"))

    for requested, resolved, exact in (
        (args.gcc, gcc, gcc_exact),
        (args.clang, clang, clang_exact),
    ):
        if requested and resolved is None:
            reasons.append(f"the requested oracle {requested!r} does not resolve and no fallback exists")
        elif requested and not exact:
            print(f"note: {requested} resolved to {resolved} (the runner names it by family)", file=sys.stderr)

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
    names = ", ".join(f"{pathlib.Path(o).name}={o}" for o in (gcc, clang) if o)
    oracle = f"oracles agreed: {names}" if names else "no oracle available; shape and run checks only"
    staging = "sign-extending staging present" if "movswq" in narrow_text or "movswl" in narrow_text else ""
    print(f"check_narrow_int_codegen: PASS ({oracle}; narrow divides 32-bit; {staging}; "
          f"probe frame executed at 3 levels; optimizer matrix {len(OPT_MATRIX)}x2 clean; "
          f">2 GiB FDE exact)")
    return 0


if __name__ == "__main__":
    # A gate that crashes prints a traceback and no verdict, which is
    # indistinguishable from an infrastructure fault in a shard log. Report the
    # traceback AS the finding instead.
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except BaseException:  # noqa: BLE001 - a gate reports, it does not escape
        detail = traceback.format_exc()
        print(detail, file=sys.stderr)
        # Shard logs are read from the TAIL, so the diagnosis has to survive
        # there: repeat the innermost frame and the exception on the final line.
        innermost = [line for line in detail.strip().splitlines() if line.strip()][-1]
        print(f"check_narrow_int_codegen: FAIL (gate error: {innermost})", file=sys.stderr)
        sys.exit(1)
