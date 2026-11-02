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
import re
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
PROBE_LOOP = HERE / "probe_loop_codegen.c"

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
    """The stack-clash probe is a FLAG, and it is a LOOP when asked for.

    Both halves are asserted, because either one alone can regress silently:

      * With `-fstack-clash-protection` a frame larger than a page must walk
        down it a page at a time (`orl $0, (%rsp)` per page) and close the walk
        with the register-form `subq %r11, %rsp` -- a frame above 2 GiB has no
        imm32 sub, so the loop's last step is the only encoding available.
      * WITHOUT the flag (the default, and what GCC 16 does for the same
        source) it must allocate in ONE step. The prologue used to probe
        unconditionally, so a 3 GiB frame paid 786 432 page faults before its
        first statement ran and could not start at all on a machine without
        that much stack; that is what the negative half pins.
    """
    asm = work / "probe.s"
    ok, err = compile_to_asm(
        lccc, PROBE, asm, ["-O2", "-fno-omit-frame-pointer", "-fstack-clash-protection"]
    )
    if not ok:
        fail(reasons, f"probe fixture did not compile: {err.strip()[:400]}")
        return
    text = asm.read_text(errors="replace")
    if "orl $0, (%rsp)" not in text:
        fail(reasons, "probe fixture has no stack-clash probe store: the >page frame was allocated in one step")
    # Below the unroll boundary the probe is a straight line of page steps: no
    # loop label, no branch, no scratch register. That is a code-size and
    # latency decision, so it is pinned here rather than left to drift.
    if "stack_probe" in text:
        fail(reasons, "a 3-page frame took the probe LOOP: below 64 KiB the unrolled form is cheaper")
    if "subq %r11, %rsp" in text:
        fail(reasons, "a 3-page frame closed its probe with the register form, which it does not need")
    # The probed frame keeps its CFA in %rbp, so the description is a
    # register rule rather than an offset; either form is a described frame.
    if ".cfi_undefined" not in text and ".cfi_def_cfa" not in text:
        fail(reasons, "probe fixture has neither a described CFA nor an explicit refusal")

    # Above the boundary the probe is a loop, and its tail is the register-form
    # `subq %r11, %rsp` -- the only encoding that can reach the bottom of a
    # frame past imm32, and the shape the unwind model has to describe.
    loop_asm = work / "probe_loop.s"
    ok, err = compile_to_asm(
        lccc, PROBE_LOOP, loop_asm, ["-O2", "-fno-omit-frame-pointer", "-fstack-clash-protection"]
    )
    if not ok:
        fail(reasons, f"probe-loop fixture did not compile: {err.strip()[:400]}")
        return
    loop_text = loop_asm.read_text(errors="replace")
    if "stack_probe" not in loop_text:
        fail(reasons, "a 64-page frame did not take the probe loop: the unrolled form is unbounded")
    if "subq %r11, %rsp" not in loop_text:
        fail(reasons, "the probe loop does not close with the register-form subq %r11, %rsp")
    for match in re.finditer(r"^\s*subq \$(\d+), %rsp$", loop_text, re.MULTILINE):
        if int(match.group(1)) > 4096:
            fail(
                reasons,
                f"the probe loop subtracts {match.group(1)} bytes in one step: a probe step is one page",
            )
            break
    # The loop moves %rsp by an amount that is only known at run time, so the
    # CFA has to come from %rbp. A frame that spelled a %rsp offset here would
    # be lying to the unwinder about every intermediate PC.
    if ".cfi_def_cfa_register %rbp" not in loop_text:
        fail(reasons, "the probe loop does not hand the CFA to %rbp, so its %rsp is undescribed mid-loop")
    if ".cfi_undefined" in loop_text:
        fail(reasons, "the probe-loop fixture has an undefined CFA")

    unprobed = work / "probe_default.s"
    ok, err = compile_to_asm(lccc, PROBE, unprobed, ["-O2", "-fno-omit-frame-pointer"])
    if not ok:
        fail(reasons, f"probe fixture did not compile without the flag: {err.strip()[:400]}")
        return
    plain = unprobed.read_text(errors="replace")
    if "orl $0, (%rsp)" in plain:
        fail(
            reasons,
            "probe fixture probes without -fstack-clash-protection: the probe ignores its flag",
        )
    # ... and it still allocates the whole frame, in one step: declining the
    # probe must not turn a >page frame into an unallocated one.
    steps = [int(m.group(1)) for m in re.finditer(r"^\s*subq \$(\d+), %rsp$", plain, re.MULTILINE)]
    if not any(step > 4096 for step in steps):
        fail(
            reasons,
            "the unprobed fixture did not allocate its >page frame in one step "
            f"(subq steps seen: {steps or 'none'})",
        )


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


# Object sizes the >2 GiB fixtures declare. Only the SIZE is stated here; the
# frame the prologue allocates is read back from the emitted asm, because the
# contract this gate enforces is asm <-> FDE: a hard-coded frame size would have
# to be updated in two places and would quietly stop testing the boundary it was
# written for the first time the layout rounded differently.
OBJECT_SIZES = {
    "large_frame_past_i32": 0x80000008,
    "large_frame_at_i32": 0x7FFFFFF8,
    "large_frame_past_u32": 0x10000000000,
}


def prologue_frames(asm: str) -> dict[str, tuple[int, str]]:
    """Per function: (bytes the prologue takes off %rsp, which encoding).

    Two encodings exist and the difference is the point of the fixtures: an
    `imm32` `subq $F, %rsp`, and the register form `movq $F, %r11` +
    `subq %r11, %rsp` that a delta past imm32 needs.
    """
    frames: dict[str, tuple[int, str]] = {}
    current: str | None = None
    pending: int | None = None
    for line in asm.splitlines():
        if line and not line[0].isspace() and line.endswith(":"):
            current = None if line.startswith(".") else line[:-1]
            pending = None
            continue
        if current is None:
            continue
        match = re.fullmatch(r"\s*subq \$(\d+), %rsp", line)
        if match is not None:
            frames.setdefault(current, (int(match.group(1)), "imm32"))
            continue
        match = re.fullmatch(r"\s*movq \$(\d+), %r11", line)
        if match is not None:
            pending = int(match.group(1))
            continue
        if pending is not None and re.fullmatch(r"\s*subq %r11, %rsp", line):
            frames.setdefault(current, (pending, "register"))
            pending = None
    return frames


# `readelf --debug-dump=frames` renders a `DW_CFA_def_cfa_offset` operand that
# does not fit a signed 32-bit value differently depending on its version: binutils
# 2.47 prints `2147483648` (measured), binutils 2.40 prints `-2147483648`
# (measured, same bytes, same object) because it decodes the ULEB into a signed
# long. The bytes are correct in both cases -- lccc's `.eh_frame` for these frames
# is byte-identical to GNU as 2.47's, `cmp`-verified -- so the gate normalizes the
# rendering instead of trusting it, and the raw-byte decode below stays the
# authoritative reading.
def readelf_int(text: str) -> int:
    """An unsigned DWARF operand from a possibly signed rendering.

    A negative rendering is sign extension, not a negative offset: the operand
    is a ULEB128 by definition. 2.40 widens through a 32-bit signed value, so
    the 32-bit repair is tried first; a value too large for that is widened from
    64 bits, and one that neither fits stays negative and fails the assertion
    (which is the right answer -- a truncated rendering cannot be trusted).
    """
    value = int(text)
    if value >= 0:
        return value
    for width in (32, 64):
        candidate = value + (1 << width)
        if 0 <= candidate < (1 << width):
            return candidate
    return value


def fde_blocks(obj: pathlib.Path) -> dict[int, list[str]] | None:
    """Map FDE pc-start to its rendered rule lines, via `readelf -wf`.

    The rendered form is the point: `decode_uleb` over the raw section finds
    every candidate operand but cannot say WHICH function it belongs to, and the
    contract below is per function.
    """
    readelf = shutil.which("readelf")
    if readelf is None:
        return None
    proc = run([readelf, "--debug-dump=frames", str(obj)])
    if proc.returncode != 0:
        return None
    blocks: dict[int, list[str]] = {}
    current: list[str] | None = None
    for line in proc.stdout.splitlines():
        if re.search(r"\bFDE\b", line):
            match = re.search(r"pc=([0-9a-f]+)\.\.", line)
            if match is None:
                current = None
                continue
            current = []
            blocks[int(match.group(1), 16)] = current
        elif re.search(r"\bCIE\b", line):
            current = None
        elif current is not None:
            current.append(line)
    return blocks


def function_addresses(obj: pathlib.Path) -> dict[str, int]:
    readelf = shutil.which("readelf")
    if readelf is None:
        return {}
    proc = run([readelf, "-sW", str(obj)])
    if proc.returncode != 0:
        return {}
    symbols: dict[str, int] = {}
    for line in proc.stdout.splitlines():
        parts = line.split()
        if len(parts) < 8 or parts[3] != "FUNC":
            continue
        symbols[parts[7]] = int(parts[1], 16)
    return symbols


def check_large_frame_unwind(lccc: str, work: pathlib.Path, reasons: list[str]) -> None:
    """A >2 GiB frame must be described EXACTLY, in either legitimate spelling.

    There are two ways to describe such a frame and both are exact, so the
    contract is stated over the descriptions rather than over one encoding:

      * `DW_CFA_def_cfa_offset <frame + 8>` with %rsp as the CFA base. The
        immediate does not fit in 32 bits for the largest fixtures, which is
        exactly what makes this the spelling that proves the ULEB128 path is not
        truncated. `at_i32` -- a 2 GiB object, the largest frame that still has
        no frame pointer and whose delta still has an `imm32` encoding -- keeps
        this form, and its offset is 2^31 exactly: the smallest value that needs
        a five-byte ULEB128 and cannot be confused with a 32-bit field.
      * `DW_CFA_def_cfa_register %rbp` once the prologue has established the
        frame pointer. The frame pointer is what makes a >2 GiB object
        addressable near both ends -- slots near the frame base are `%rbp`
        relative, slots near its bottom `%rsp` relative -- so the two largest
        fixtures keep it, and their CFA is `%rbp + 16` at every PC after the
        prologue. That is exact too; it just is not an offset.

    What must never happen is an UNDESCRIBED frame or an offset that does not
    match the frame the prologue actually allocates, so both are checked against
    the asm rather than against a table of remembered numbers.
    """
    obj = work / "bigframe.o"
    asm = work / "bigframe.s"
    proc = run([lccc, "-O2", "-c", str(BIGFRAME), "-o", str(obj)])
    if proc.returncode != 0:
        fail(reasons, f"large_frame fixture did not compile: {proc.stderr.strip()[:400]}")
        return
    proc = run([lccc, "-O2", "-S", str(BIGFRAME), "-o", str(asm)])
    if proc.returncode != 0:
        fail(reasons, f"large_frame fixture did not compile to asm: {proc.stderr.strip()[:400]}")
        return
    asm_text = asm.read_text(errors="replace")
    if ".cfi_undefined" in asm_text:
        fail(reasons, "large_frame fixture emits .cfi_undefined: the >2 GiB frame lost its CFA")
    frames = prologue_frames(asm_text)
    blocks = fde_blocks(obj)
    if blocks is None:
        fail(reasons, "readelf could not dump frames, so the >2 GiB FDE assertion cannot run")
        return
    addresses = function_addresses(obj)
    exact: set[int] = set()
    for name, size in OBJECT_SIZES.items():
        measured = frames.get(name)
        if measured is None:
            fail(reasons, f"no prologue stack adjustment found for {name} in the emitted asm")
            continue
        frame, _encoding = measured
        if frame < size:
            fail(reasons, f"{name} allocated {frame} bytes for a {size}-byte object")
        start = addresses.get(name)
        if start is None:
            fail(reasons, f"symbol {name} is missing from the object, so its FDE cannot be checked")
            continue
        rules = blocks.get(start)
        if rules is None:
            fail(
                reasons,
                f"no FDE covers {name} at pc 0x{start:x}: the >2 GiB frame has no unwind description",
            )
            continue
        offsets = {
            readelf_int(match.group(1))
            for rule in rules
            if (match := re.search(r"DW_CFA_def_cfa_offset:\s*(-?\d+)", rule)) is not None
        }
        exact |= offsets
        frame_pointer = any(
            re.search(r"DW_CFA_def_cfa_register:\s*r6 \(rbp\)", rule) for rule in rules
        )
        # The CFA is 8 bytes above the frame (%rsp points at its bottom after the
        # prologue), so an offset spelling has to be exactly frame + 8.
        if frame + 8 not in offsets and not frame_pointer:
            fail(
                reasons,
                f"{name} allocates {frame} bytes but is described by neither "
                f"DW_CFA_def_cfa_offset {frame + 8} nor a %rbp register rule "
                f"(offsets seen: {sorted(offsets) or 'none'})",
            )
    # `at_i32` exists to keep the immediate encoding alive: the day every frame
    # is routed through %r11 the register form still works, so nothing else would
    # notice that the short form had been lost.
    at_i32_form = frames.get("large_frame_at_i32", (0, ""))[1]
    if at_i32_form != "imm32":
        fail(
            reasons,
            f"large_frame_at_i32 used the {at_i32_form or 'unknown'} encoding: a delta that fits "
            "imm32 must keep the immediate form",
        )
    # The >2^31 exact-offset path has to be exercised by at least one function,
    # or the ULEB128 truncation this fixture set exists to catch cannot show up
    # in any FDE. `at_i32` is the one that spells it: 2^31.
    huge = {value for value in exact if value > 0x7FFFFFFF}
    if not huge:
        fail(
            reasons,
            "no FDE spells an exact CFA offset above 2^31, so the ULEB128 path is not covered "
            f"(offsets seen: {sorted(exact) or 'none'})",
        )
    # Second, independent reading of the same fact: decode the raw `.eh_frame`
    # operands ourselves, so a `readelf` that rendered the value through 32 bits
    # cannot make the assertion above pass. Every `DW_CFA_def_cfa_offset` (0x0e)
    # operand is decoded; 0x0e also occurs inside other operands, so a false
    # positive would have to encode the exact frame size by accident.
    data = eh_frame_bytes(obj)
    if data is None:
        fail(reasons, "no tool could dump the raw .eh_frame bytes for the ULEB cross-check")
        return
    decoded: set[int] = set()
    for index, byte in enumerate(data):
        if byte != 0x0E:
            continue
        result = decode_uleb(data, index + 1)
        if result is not None:
            decoded.add(result[0])
    for value in sorted(huge):
        if value not in decoded:
            fail(
                reasons,
                f"the raw .eh_frame carries no exact {value}-byte CFA offset "
                f"(decoded: {sorted(decoded) or 'none'})",
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
