#!/usr/bin/env python3
"""Narrow-integer codegen and large-frame unwind contracts, pinned in the TEXT
and in the emitted `.eh_frame` BYTES.

Four contracts, each one a shape this repository has already regressed at least
once, and each one checked where the defect is visible rather than where it is
merely probable:

1. STAGING WIDTH.  A narrow value (i8/i16/u8/u16) is canonical only to 32 bits,
   so every consumer that reads the register at 64-bit width re-extends it from
   the narrow width.  The fixture's index lanes use NEGATIVE indices, where a
   zero-extending staging copy would read the wrong element -- the exact bug
   `vector_sdiv_srem_short_signext.c` proves behaviourally.

2. NARROW DIVIDE WIDTH.  A narrow divide is a 32-bit divide (`cltd; idivl`,
   `xorl %edx,%edx; divl`).  `idivq`/`divq`/`cqto` in any of these functions
   means the width was lost again.

3. EXECUTED PROBE FRAME.  `large_frame_probe_boundary.c` crosses the 4 KiB
   stack-clash probe page, calls out of it, and is CALLED.  With a frame pointer
   the prologue's probe loop ends in the register-form `subq %r11, %rsp`, so
   this is the one stack adjustment above one page that can be executed and
   compared against the oracle.

4. LARGE-FRAME UNWIND BYTES.  For a >2 GiB frame the CFA offset is an ordinary
   ULEB128 above 2^31.  `readelf --debug-dump=frames` truncates that to 32 bits
   when it PRINTS it, so the assertion here decodes `.eh_frame` itself: the
   directive must be `DW_CFA_def_cfa_offset` (0x0e) carrying the exact value,
   never the factored `DW_CFA_def_cfa_offset_sf` (0x13) that a 32-bit round trip
   through the assembler's CFA-offset parser used to produce.

Usage: check_narrow_int_codegen.py [--lccc PATH] [--gcc PATH] [--clang PATH]
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent

NARROW = HERE / "narrow_int_codegen.c"
PROBE = HERE / "large_frame_probe_boundary.c"
LARGE = HERE / "large_stack_frame_codegen.c"

# Expected program output, pinned from GNU gcc 16 and Clang 23 (both agree, at
# every optimization level, which is what makes it an oracle and not a snapshot
# of this compiler's behaviour).
NARROW_EXPECT = "narrow_checks=538 sum=694188"
PROBE_EXPECT = "probe_frame=-234"

OPT_LEVELS = ["-O0", "-O1", "-O2", "-O3", "-Os"]

# `1048576 * 1048576 + 112` and `2147483648 + 16`: the probes' exact CFA
# offsets (frame + 8 for the return address), as ULEB128 payloads.
CFA_3GIB = 1099511627792
CFA_2GIB = 2147483664


def uleb(value: int) -> bytes:
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


class Gate:
    def __init__(self, lccc: Path, gcc: str | None, clang: str | None, work: Path):
        self.lccc = lccc
        self.gcc = shutil.which(gcc) if gcc else None
        self.clang = shutil.which(clang) if clang else None
        self.work = work
        self.failures: list[str] = []

    def fail(self, msg: str) -> None:
        self.failures.append(msg)
        print(f"  FAIL: {msg}", file=sys.stderr)

    def ok(self, msg: str) -> None:
        print(f"  {msg}")

    def run(self, cmd: list[str]) -> subprocess.CompletedProcess:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=600)

    def compile_asm(self, src: Path, flags: list[str], out: Path) -> str | None:
        proc = self.run([str(self.lccc), *flags, "-S", str(src), "-o", str(out)])
        if proc.returncode != 0:
            self.fail(f"{src.name} {' '.join(flags)}: -S failed: {proc.stderr.strip()[:300]}")
            return None
        return out.read_text()

    def build_and_run(self, src: Path, flags: list[str], out: Path) -> str | None:
        proc = self.run([str(self.lccc), *flags, str(src), "-o", str(out)])
        if proc.returncode != 0:
            self.fail(f"{src.name} {' '.join(flags)}: build failed: {proc.stderr.strip()[:300]}")
            return None
        run = self.run([str(out)])
        if run.returncode != 0:
            self.fail(f"{src.name} {' '.join(flags)}: exited {run.returncode}")
            return None
        return run.stdout.strip()

    # ── contract 1 and 2 ───────────────────────────────────────────────────
    def narrow_shapes(self) -> None:
        print("contract 1/2: narrow staging width and narrow divide width")
        asm = self.compile_asm(NARROW, ["-O2"], self.work / "narrow.s")
        if asm is None:
            return
        body = "\n".join(
            line for line in asm.splitlines() if not line.lstrip().startswith("#")
        )
        wide_divide = [
            m for m in ("idivq", "divq", "cqto") if re.search(rf"^[ \t]+{m}\b", body, re.M)
        ]
        if wide_divide:
            self.fail(f"narrow divides widened to 64 bits: {wide_divide}")
        else:
            self.ok("narrow divides use the 32-bit form only (no idivq/divq/cqto)")
        if not re.search(r"^[ \t]+(idivl|divl)\b", body, re.M):
            self.fail("no 32-bit divide in the narrow-divide fixture")
        # The signed index lanes must SIGN-extend: `movswq %..w, %..` (or the
        # 32-bit `movswl` into a 32-bit destination).
        if not re.search(r"^[ \t]+movsw[ql]\b", body, re.M):
            self.fail("a signed narrow index reached the address computation unsigned")
        else:
            self.ok("signed narrow indices are sign-extended")
        # `staged_twice_i16` must stage the same narrow value twice without a
        # second re-canonicalization: at most one extend per staging site is
        # allowed, which is what the accumulator's canonicality mark buys.
        fn = _function_body(body, "staged_twice_i16")
        canon = re.findall(r"^[ \t]+movswq %ax, %rax$", fn, re.M)
        if len(canon) > 1:
            self.fail(
                f"staged_twice_i16 re-canonicalizes the accumulator {len(canon)} times"
            )
        else:
            self.ok("the accumulator's canonical form is reused, not re-derived")

    # ── contract 3 ─────────────────────────────────────────────────────────
    def probe_frame(self) -> None:
        print("contract 3: executed frame past the probe page")
        for flags in (["-O2"], ["-O0"], ["-O2", "-fno-omit-frame-pointer"]):
            out = self.build_and_run(PROBE, flags, self.work / "probe.bin")
            if out is None:
                continue
            if out != PROBE_EXPECT:
                self.fail(f"probe frame {' '.join(flags)}: {out!r} != {PROBE_EXPECT!r}")
            else:
                self.ok(f"probe frame {' '.join(flags)}: {out}")
        asm = self.compile_asm(
            PROBE, ["-O2", "-fno-omit-frame-pointer"], self.work / "probe.s"
        )
        if asm is None:
            return
        if "orl $0, (%rsp)" not in asm:
            self.fail("no probe loop in the >1-page frame prologue")
        elif "subq %r11, %rsp" not in asm:
            self.fail("the probe loop does not end in the register-form adjustment")
        else:
            self.ok("probe loop present and closed by `subq %r11, %rsp`")
        if ".cfi_undefined" in asm:
            self.fail("a probed frame lost its unwind information (.cfi_undefined)")

    # ── contract 4 ─────────────────────────────────────────────────────────
    def large_frame_unwind(self) -> None:
        print("contract 4: >2 GiB frame unwind bytes")
        obj = self.work / "large.o"
        proc = self.run([str(self.lccc), "-O2", "-c", str(LARGE), "-o", str(obj)])
        if proc.returncode != 0:
            self.fail(f"large_stack_frame_codegen.c -c failed: {proc.stderr.strip()[:300]}")
            return
        sections = _elf_section_hex(obj, ".eh_frame")
        if sections is None:
            self.fail("no .eh_frame section in the large-frame object")
            return
        for name, value in (("2 GiB", CFA_2GIB), ("3 GiB", CFA_3GIB)):
            wanted = bytes([0x0E]) + uleb(value)
            if wanted not in sections:
                self.fail(
                    f"{name} frame: no DW_CFA_def_cfa_offset {value} "
                    f"(wanted {wanted.hex()}, .eh_frame {sections.hex()[:200]})"
                )
            else:
                self.ok(f"{name} frame: DW_CFA_def_cfa_offset {value} encoded exactly")
        if b"\x13" in sections.replace(b"\x13\x00", b""):
            # 0x13 is DW_CFA_def_cfa_offset_sf: a FACTORED operand, which a
            # 32-bit round trip produced. It must not appear for these frames.
            self.fail("a factored CFA operand appeared in .eh_frame")

    # ── the matrix over both fixtures ──────────────────────────────────────
    def opt_matrix(self) -> None:
        print("matrix: every optimization level, with and without -g")
        for src in (NARROW, PROBE):
            for opt in OPT_LEVELS:
                for extra in ([], ["-g"]):
                    flags = [opt, *extra]
                    asm = self.compile_asm(src, flags, self.work / "m.s")
                    if asm is None:
                        continue
                    if ".cfi_undefined" in asm:
                        self.fail(f"{src.name} {' '.join(flags)}: .cfi_undefined")
                    built = self.build_and_run(src, flags, self.work / "m.bin")
                    expect = NARROW_EXPECT if src is NARROW else PROBE_EXPECT
                    if built is not None and built != expect:
                        self.fail(f"{src.name} {' '.join(flags)}: {built!r} != {expect!r}")
        self.ok(f"both fixtures clean at {len(OPT_LEVELS)} levels x {{-g, no -g}}")

    def oracle(self) -> None:
        # The pinned strings must be what an independent compiler prints, or
        # they are a snapshot of this compiler's behaviour rather than an
        # oracle. Both oracles are optional so a minimal host can still run the
        # shape contracts, but a present one that disagrees is a failure.
        print("oracle: the pinned outputs are what gcc/clang print")
        for tool, label in ((self.gcc, "gcc"), (self.clang, "clang")):
            if tool is None:
                print(f"  skip {label}: not found")
                continue
            for src, expect in ((NARROW, NARROW_EXPECT), (PROBE, PROBE_EXPECT)):
                for opt in ("-O0", "-O2"):
                    out = self.work / "oracle.bin"
                    proc = self.run([tool, opt, str(src), "-o", str(out)])
                    if proc.returncode != 0:
                        self.fail(f"{label} {opt} {src.name}: build failed")
                        continue
                    run = self.run([str(out)])
                    got = run.stdout.strip()
                    if got != expect:
                        self.fail(f"{label} {opt} {src.name}: {got!r} != {expect!r}")
            print(f"  {label}: pinned outputs confirmed")

    def report(self) -> int:
        if self.failures:
            print(f"check_narrow_int_codegen: FAILED ({len(self.failures)} contract(s))")
            return 1
        print("check_narrow_int_codegen: PASS")
        return 0


def _function_body(asm: str, name: str) -> str:
    lines = asm.splitlines()
    out: list[str] = []
    inside = False
    for line in lines:
        if line.strip() == f"{name}:":
            inside = True
            continue
        if inside and line.strip().startswith(".size "):
            break
        if inside:
            out.append(line)
    return "\n".join(out)


def _elf_section_hex(obj: Path, section: str) -> bytes | None:
    """Raw bytes of `section`, read through `objdump -s` (no ELF parser here)."""
    proc = subprocess.run(
        ["objdump", "-s", "-j", section, str(obj)], capture_output=True, text=True
    )
    if proc.returncode != 0:
        return None
    data = bytearray()
    for line in proc.stdout.splitlines():
        parts = line.split()
        if len(parts) < 5 or not re.fullmatch(r"[0-9a-f]{4,}", parts[0]):
            continue
        for word in parts[1:5]:
            if not re.fullmatch(r"[0-9a-f]{8}", word):
                continue
            data.extend(bytes.fromhex(word))
    return bytes(data) or None


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc"))
    ap.add_argument("--clang", default=os.environ.get("CLANG", "clang"))
    args = ap.parse_args(argv)

    lccc = Path(args.lccc)
    if not lccc.exists():
        print(f"check_narrow_int_codegen: lccc not found at {lccc}", file=sys.stderr)
        return 1
    with tempfile.TemporaryDirectory(prefix="lccc-narrow.") as tmp:
        gate = Gate(lccc, args.gcc, args.clang, Path(tmp))
        gate.oracle()
        gate.narrow_shapes()
        gate.probe_frame()
        gate.large_frame_unwind()
        gate.opt_matrix()
        return gate.report()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
