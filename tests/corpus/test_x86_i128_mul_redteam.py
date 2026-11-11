#!/usr/bin/env python3
"""Red-team contracts for the x86-64 generic 128-bit multiply.

The historical emitter used ``pushq %rdx; pushq %rax; ...; popq`` as a register
move.  ``emit_i128_prep_binop_impl`` already documents why a transient RSP
shift is hazardous to a text peephole that models ``%rsp``-relative slots;
the current dead-store/store-forwarding passes track those shifts, but the
generic multiply still paid a stack round-trip on every dynamic 128x128
product and kept a needless dependency on that text model.

This module pins the 6-instruction register-only schoolbook sequence, checks
the emitted machine code with ELF/Capstone tooling when available, runs a
64-case differential against GCC 16.2 / Clang 23.1.5 (and ICX when present),
and includes source-level negative controls for the historical sequence and
for the other three backends' distinct stack contracts.

The sequence forms both cross products in place and adds the one through the
LEFT operand's high half last.  That order is a latency contract, not a
style choice: the term added last reaches the result through one ``addq``,
and the left operand is the loop-carried one in ``acc *= x``.  An
8-instruction variant that first copied the operands into fresh registers
(``movq %rcx, %rdi; movq %rax, %r8; movq %rdx, %rcx; ...``) was measured and
rejected: on the slot-reloading shape this backend emits it was no faster
than the historical 13 instructions (1.219 vs 1.226 ns per multiply on Ice
Lake-SP) while this one takes 0.972 ns, and it clobbered ``%rcx`` and ``%r8``
on top.  ``engineering/REDTEAM-2026-11-11-I128-TYPEOF.md`` has the data.
"""

from __future__ import annotations

import importlib.util
import os
import platform
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
LCCC = Path(os.environ.get("LCCC", REPO / "target/fastbuild/lccc"))
ARM = Path(os.environ.get("CCC_ARM", REPO / "target/fastbuild/lccc-arm"))
RISCV = Path(os.environ.get("CCC_RISCV", REPO / "target/fastbuild/lccc-riscv"))
I686 = Path(os.environ.get("CCC_I686", REPO / "target/fastbuild/lccc-i686"))
X86_MUL = REPO / "src/backend/x86/codegen/i128_ops.rs"
X86_EMIT = REPO / "src/backend/x86/codegen/emit.rs"
I686_MUL = REPO / "src/backend/i686/codegen/i128_ops.rs"
ARM_MUL = REPO / "src/backend/arm/codegen/i128_ops.rs"
RISCV_MUL = REPO / "src/backend/riscv/codegen/i128_ops.rs"
GCC = os.environ.get("GCC16") or shutil.which("gcc-16") or shutil.which("gcc")
CLANG = os.environ.get("CLANG") or shutil.which("clang-23")
ICX = os.environ.get("ICX") or shutil.which("icx")

EXPECTED_X86 = (
    "imulq %rcx, %rdx",
    "imulq %rax, %rsi",
    "movq %rdx, %rdi",
    "mulq %rcx",
    "addq %rsi, %rdx",
    "addq %rdi, %rdx",
)

# Registers the historical sequence clobbered that the generic multiply no
# longer touches.  A reappearance means the scratch set grew back.
RELEASED_SCRATCH = ("%r8", "%r9")

OLD_SEQUENCE = """    pushq %rdx
    pushq %rax
    movq %rcx, %r8
    movq %rsi, %r9
    popq %rax
    popq %rdi
    movq %rdi, %rcx
    imulq %r8, %rcx
    movq %rax, %rsi
    imulq %r9, %rsi
    mulq %r8
    addq %rcx, %rdx
    addq %rsi, %rdx
"""

MASK64 = (1 << 64) - 1


def deterministic_cases(count: int = 64) -> list[tuple[int, int, int, int]]:
    """Six hard edges plus a deterministic LCG sweep, all within uint64_t."""
    edges = [
        (0, 0, 0, 0),
        (1, 0, 1, 0),
        (MASK64, 0, MASK64, 0),
        (0x9E3779B97F4A7C15, 0x123456789ABCDEF0,
         0xC2B2AE3D27D4EB4F, 0xFEDCBA9876543210),
        (0x0123456789ABCDEF, MASK64, 0xFEDCBA9876543210, MASK64),
        (1 << 63, 1 << 63, 1 << 63, 1 << 63),
    ]
    state = 0x243F6A8885A308D3
    cases = list(edges)
    while len(cases) < count:
        def next_u64() -> int:
            nonlocal state
            state = (6364136223846793005 * state + 1442695040888963407) & MASK64
            return state
        cases.append(tuple(next_u64() for _ in range(4)))  # type: ignore[arg-type]
    return cases


def runtime_source() -> str:
    rows = ",\n".join(
        "    { " + ", ".join(f"0x{v:016x}ULL" for v in case) + " }"
        for case in deterministic_cases()
    )
    return f'''#include <stdio.h>
#include <stdint.h>
typedef unsigned __int128 u128;
volatile uint64_t lo_a, hi_a, lo_b, hi_b;
static const uint64_t cases[64][4] = {{
{rows}
}};
__attribute__((noinline)) static u128 wide_mul(void) {{
    u128 a = ((u128)hi_a << 64) | lo_a;
    u128 b = ((u128)hi_b << 64) | lo_b;
    return a * b;
}}
int main(void) {{
    for (int i = 0; i < 64; i++) {{
        lo_a = cases[i][0]; hi_a = cases[i][1];
        lo_b = cases[i][2]; hi_b = cases[i][3];
        u128 r = wide_mul();
        printf("%016llx %016llx\\n",
               (unsigned long long)r,
               (unsigned long long)(r >> 64));
    }}
    return 0;
}}
'''


def function_source(text: str, marker: str) -> str:
    start = text.index(marker)
    end = text.index("\n    }", start)
    return text[start:end]


def has_pyelf_and_capstone() -> bool:
    return all(
        importlib.util.find_spec(name) is not None
        for name in ("elftools", "capstone")
    )


def elf_machine(path: Path) -> str | None:
    """Return the ELF e_machine string via pyelftools, else readelf, else None."""
    if importlib.util.find_spec("elftools") is not None:
        from elftools.elf.elffile import ELFFile
        with path.open("rb") as stream:
            return ELFFile(stream).header["e_machine"]
    readelf = shutil.which("readelf")
    if not readelf:
        return None
    proc = subprocess.run(
        [readelf, "-h", str(path)], capture_output=True, text=True, timeout=30
    )
    for line in proc.stdout.splitlines():
        if "Machine:" in line:
            return line.split(":", 1)[1].strip()
    return None


def decode_x86_function(path: Path, symbol: str = "wide_mul") -> list[str]:
    """Decode one function's machine bytes; return lower-case mnemonics."""
    from capstone import CS_ARCH_X86, CS_MODE_64, Cs
    from elftools.elf.elffile import ELFFile

    with path.open("rb") as stream:
        elf = ELFFile(stream)
        symtab = elf.get_section_by_name(".symtab")
        text = elf.get_section_by_name(".text")
        if symtab is None or text is None:
            return []
        entry = next(
            (s for s in symtab.iter_symbols() if s.name == symbol), None
        )
        if entry is None:
            return []
        offset = entry["st_value"]
        size = entry["st_size"]
        code = text.data()[offset:offset + size]
    md = Cs(CS_ARCH_X86, CS_MODE_64)
    return [insn.mnemonic.lower() for insn in md.disasm(code, offset)]


class X86I128MulRedteam(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.x86 = X86_MUL.read_text()
        cls.x86_emit = X86_EMIT.read_text()
        cls.i686 = I686_MUL.read_text()
        cls.arm = ARM_MUL.read_text()
        cls.riscv = RISCV_MUL.read_text()

    def test_x86_generic_multiply_is_six_register_only_instructions(self) -> None:
        body = function_source(self.x86, "pub(super) fn emit_i128_mul_impl(&mut self) {")
        self.assertNotIn("pushq", body)
        self.assertNotIn("popq", body)
        self.assertNotIn("rsp", body)
        emitted = [
            line.strip().removeprefix('self.state.emit("').removesuffix('");')
            .removeprefix("    ")
            for line in body.splitlines()
            if "self.state.emit(" in line
        ]
        self.assertEqual(len(emitted), 6, emitted)
        self.assertEqual(tuple(emitted), EXPECTED_X86, emitted)
        for register in RELEASED_SCRATCH:
            self.assertNotIn(register, body)
        # The latency contract: the cross term through the LEFT high half
        # (parked in %rdi) is the last add, after the one through %rsi.
        self.assertLess(
            emitted.index("addq %rsi, %rdx"), emitted.index("addq %rdi, %rdx")
        )

    def test_historical_push_pop_sequence_is_rejected(self) -> None:
        self.assertIn("pushq %rdx", OLD_SEQUENCE)
        self.assertIn("popq %rdi", OLD_SEQUENCE)
        self.assertIn("mulq %r8", OLD_SEQUENCE)
        self.assertFalse(
            "pushq" not in OLD_SEQUENCE and "popq" not in OLD_SEQUENCE
        )
        self.assertNotIn("mulq %rcx", OLD_SEQUENCE)
        for register in RELEASED_SCRATCH:
            self.assertIn(register, OLD_SEQUENCE)

    def test_dead_push_pop_prep_helper_is_gone(self) -> None:
        # x86's live prep helper lives in i128_ops.rs and is deliberately
        # register-only.  The stale emit.rs twin was unreachable but still
        # carried the old push/pop shift; leaving it invited a future call
        # site to resurrect the hazard.
        self.assertNotIn("fn prep_i128_binop(", self.x86_emit)

    def test_other_backends_keep_their_own_stack_contracts(self) -> None:
        self.assertIn("esp_adjust", self.i686)
        self.assertIn('pushl %edx', self.i686)
        self.assertIn('addl $8, %esp', self.i686)
        for source, needle in ((self.arm, "umulh"), (self.riscv, "mulhu")):
            body = function_source(source, "pub(super) fn emit_i128_mul_impl(&mut self) {")
            self.assertIn(needle, body)
            self.assertNotIn("push", body)
            self.assertNotIn("pop", body)

    @unittest.skipUnless(
        platform.machine() in ("x86_64", "amd64") and LCCC.is_file() and GCC,
        "x86-64 host, fastbuild LCCC and GCC 16.2 required",
    )
    def test_differential_products_and_raw_assembly(self) -> None:
        references = [str(GCC)]
        if CLANG:
            references.append(str(CLANG))
        if ICX:
            references.append(str(ICX))
        with tempfile.TemporaryDirectory(prefix="i128-mul-") as td:
            root = Path(td)
            source = root / "probe.c"
            source.write_text(runtime_source())
            expected: str | None = None
            for reference in references:
                with self.subTest(reference=reference):
                    exe = root / f"ref-{Path(reference).name}.out"
                    proc = subprocess.run(
                        [reference, "-O2", "-fomit-frame-pointer", str(source), "-o", str(exe)],
                        capture_output=True, text=True, timeout=120,
                    )
                    self.assertEqual(proc.returncode, 0, proc.stderr)
                    run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=60)
                    self.assertEqual(run.returncode, 0, run.stderr)
                    self.assertEqual(len(run.stdout.splitlines()), 64)
                    if expected is None:
                        expected = run.stdout
                    self.assertEqual(run.stdout, expected)

            for opt in ("-O2", "-O3"):
                for env_name, env_value in (
                    ("default", None),
                    ("raw", "LCCC_NO_PEEPHOLE"),
                    ("pass-disabled", "CCC_NO_PEEPHOLE"),
                ):
                    with self.subTest(opt=opt, peephole=env_name):
                        env = os.environ.copy()
                        if env_value:
                            env[env_value] = "1"
                        asm = root / f"lccc-{opt}-{env_name}.s"
                        exe = root / f"lccc-{opt}-{env_name}.out"
                        obj = root / f"lccc-{opt}-{env_name}.o"
                        compile_asm = subprocess.run(
                            [str(LCCC), opt, "-fomit-frame-pointer", "-S", str(source), "-o", str(asm)],
                            capture_output=True, text=True, timeout=120, env=env,
                        )
                        self.assertEqual(compile_asm.returncode, 0, compile_asm.stderr)
                        text = asm.read_text()
                        body = text.split("wide_mul:", 1)[-1].split(".size", 1)[0]
                        self.assertNotIn("pushq %rdx", body, body)
                        self.assertNotIn("popq %rdi", body, body)
                        for register in RELEASED_SCRATCH:
                            self.assertNotIn(register, body, body)
                        insns = [
                            line.strip() for line in body.splitlines()
                            if line.strip() and not line.strip().startswith((".", "#"))
                        ]
                        if env_value:
                            # The emitter's own text: the six instructions,
                            # in order, with nothing between them.
                            start = insns.index(EXPECTED_X86[0])
                            self.assertEqual(
                                tuple(insns[start:start + len(EXPECTED_X86)]), EXPECTED_X86, body
                            )
                        else:
                            # The peepholes may forward a copy into an
                            # operand (`imulq %r10, %rdx` for `%rcx`), so the
                            # register names are theirs to choose.  What must
                            # survive is the arithmetic: one widening
                            # multiply, two cross products, both added into
                            # the high word.
                            widening = [l for l in insns if l.split()[0] == "mulq"]
                            self.assertEqual(len(widening), 1, body)
                            self.assertNotIn(",", widening[0], body)
                            cross = [l for l in insns if l.split()[0] == "imulq"]
                            self.assertEqual(len(cross), 2, body)
                            into_high = [
                                l for l in insns
                                if l.startswith("addq ") and l.endswith(", %rdx")
                            ]
                            self.assertEqual(len(into_high), 2, body)
                        compile_obj = subprocess.run(
                            [str(LCCC), opt, "-fomit-frame-pointer", "-c", str(source), "-o", str(obj)],
                            capture_output=True, text=True, timeout=120, env=env,
                        )
                        self.assertEqual(compile_obj.returncode, 0, compile_obj.stderr)
                        if has_pyelf_and_capstone():
                            mnemonics = decode_x86_function(obj)
                            self.assertTrue(mnemonics, "wide_mul symbol not decoded")
                            self.assertFalse(
                                any(m.startswith("push") or m.startswith("pop") for m in mnemonics),
                                mnemonics,
                            )
                            self.assertTrue(any(m == "mul" for m in mnemonics), mnemonics)
                        compile_exe = subprocess.run(
                            [str(LCCC), opt, "-fomit-frame-pointer", str(source), "-o", str(exe)],
                            capture_output=True, text=True, timeout=120, env=env,
                        )
                        self.assertEqual(compile_exe.returncode, 0, compile_exe.stderr)
                        run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=60)
                        self.assertEqual(run.returncode, 0, run.stderr)
                        self.assertEqual(run.stdout, expected)

    @unittest.skipUnless(
        ARM.is_file() and RISCV.is_file() and I686.is_file(),
        "all four fastbuild target wrappers required",
    )
    def test_cross_isa_sequences_are_unchanged_and_assemble(self) -> None:
        with tempfile.TemporaryDirectory(prefix="i128-cross-") as td:
            root = Path(td)
            source = root / "probe.c"
            source.write_text(runtime_source())
            env = os.environ.copy()
            env["LCCC_NO_PEEPHOLE"] = "1"
            for wrapper, machine_tokens, needle in (
                (ARM, ("AARCH64", "AArch64"), "umulh"),
                (RISCV, ("RISCV", "RISC-V"), "mulhu"),
            ):
                with self.subTest(wrapper=wrapper.name):
                    asm = root / f"{wrapper.name}.s"
                    obj = root / f"{wrapper.name}.o"
                    proc = subprocess.run(
                        [str(wrapper), "-O2", "-S", str(source), "-o", str(asm)],
                        capture_output=True, text=True, timeout=120, env=env,
                    )
                    self.assertEqual(proc.returncode, 0, proc.stderr)
                    text = asm.read_text()
                    self.assertIn(needle, text)
                    self.assertNotIn(
                        "push",
                        text.split("wide_mul:", 1)[-1].split(".size", 1)[0],
                    )
                    obj_proc = subprocess.run(
                        [str(wrapper), "-O2", "-c", str(source), "-o", str(obj)],
                        capture_output=True, text=True, timeout=120, env=env,
                    )
                    self.assertEqual(obj_proc.returncode, 0, obj_proc.stderr)
                    actual = elf_machine(obj)
                    if actual:
                        self.assertTrue(
                            any(token in actual for token in machine_tokens),
                            actual,
                        )
            # i686 has no C `__int128`; compile the existing 16-byte struct
            # return carrier to prove the distinct esp_adjust path still works.
            carrier = REPO / "tests/regression/two_reg_return_carrier.c"
            i686_obj = root / "i686.o"
            proc = subprocess.run(
                [str(I686), "-O2", "-c", str(carrier), "-o", str(i686_obj)],
                capture_output=True, text=True, timeout=180, env=env,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            actual = elf_machine(i686_obj)
            if actual:
                self.assertTrue(
                    "386" in actual or "i386" in actual.lower(), actual
                )


if __name__ == "__main__":
    unittest.main()
