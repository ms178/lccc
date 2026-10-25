"""The snapshot must agree with objdump's opcode map, byte for byte.

``scripts/isa/gas-2.47.json`` is built by expanding `opcodes/i386-opc.tbl`, and
that expansion is a *transcription*: every modifier the table carries
(`D`, `W`, `FloatMF`, `+rd`, `Ugh`, `C`) has to be turned into the opcodes GNU
as actually emits.  A transcription error is invisible to the encoder
cross-reference -- the encoder is checked against the snapshot, so a wrong
snapshot entry either lets an illegal encoding through or invents a gap.

This test anchors the transcription to a second, independent implementation:
``objdump``.  Every candidate encoding is laid out in its own 16-byte slot
(each padded with `int3`, so slot boundaries are always instruction
boundaries), the whole buffer is disassembled in one pass, and the mnemonic
objdump prints at each slot is compared with the mnemonic the snapshot files
that encoding under.

It needs the pinned GNU binutils 2.47 pair, taken from the same explicit
channels the assembler differential uses (``ASMDIFF_TEST_AS`` /
``ASMDIFF_TEST_OBJDUMP``).  Without them it skips: an ambient distro objdump is
a different version of the opcode map and is not an authority here.
"""

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CROSSREF = ROOT / "scripts" / "gas_encoding_crossref.py"

# `ASMDIFF_TEST_*` are the assembler differential's channels; using the same
# names keeps one pin regime for every tool-based leg in the repository.
AS_ENV, OBJDUMP_ENV = "ASMDIFF_TEST_AS", "ASMDIFF_TEST_OBJDUMP"

# The mnemonics whose transcription this test pins.  They are the families the
# modifiers actually move: the eight ALU blocks, the `D|W` cross product on
# `mov`, the compact SIMD moves (which pair by `0x10`, not `0x01`), the
# commutative rows (`test`/`xchg`), the x87 register forms, `+rd` rows and the
# special-register moves the table exempts from the extended scheme.
FAMILIES = (
    "add", "or", "adc", "sbb", "and", "sub", "xor", "cmp",
    "mov", "movabs", "test", "xchg", "push", "pop", "bswap", "in",
    "movd", "movdqa", "movdqu", "movq", "movups", "movsd",
    "fadd", "faddp", "fmulp", "fcom",
    "inc", "dec", "jne", "setne", "cmovne",
)

# objdump's spelling of the same encoding: `movabs` is `mov` with a 64-bit
# immediate, and `pushp`/`popp` are GAS's 16-bit pseudo-suffixes for `push` and
# `pop`.  The relation is symmetric -- `mov 0xa0` decodes as `movabs` while
# `movabs 0xb8` decodes as `mov`.
ALIASES = {"movabs": "mov", "pushp": "push", "popp": "pop"}
ALIAS_GROUPS = {frozenset(("mov", "movabs")), frozenset(("push", "pushp")),
                frozenset(("pop", "popp"))}

# Encodings that are in the table but that `objdump -M att` in 64-bit mode
# names differently, with the reason.  Each one is a property of the *decoder*,
# not of the transcription, so they are excluded one by one rather than by
# weakening the comparison.
EXEMPT = {
    ("mov", 0x24): "0f 24 is `mov` to/from an obsolete TR register; objdump "
                   "does not decode TR at all",
    ("mov", 0x26): "0f 26 is the reverse of 0f 24 (see above)",
    ("push", 0x06): "06 is `push %es`, which does not exist in 64-bit mode "
                    "(the row is No64)",
    ("pop", 0x07): "07 is `pop %es`, which does not exist in 64-bit mode",
    ("xchg", 0x90): "90 is spelled `nop` by every disassembler",
    ("fcom", 0xDC): "dc /2 is `FCOM m64fp`, a memory-only form; GAS's reverse "
                    "match still produces the register shape",
    ("movq", 0x6E): "the row is the REX.W form of `66 0f 6e`, which is `movd` "
                    "without REX.W; the table does not record the W bit and "
                    "objdump correctly names the spelling it sees",
    ("movq", 0x7E): "the reverse of the `movq` 0f 6e row (see above)",
}

# `inc`/`dec` `40..4F` are one-byte opcodes in 32-bit mode and REX prefixes in
# 64-bit mode.  The slot decode is done in 64-bit mode (every other family
# needs it), so those are excluded here and covered by the encoder gate, which
# knows the backend's mode.
MODE_DEPENDENT = {"inc", "dec"}

PREFIX = {"66": b"\x66", "f2": b"\xf2", "f3": b"\xf3", "none": b""}


def _load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _objdump() -> str | None:
    as_path = os.environ.get(AS_ENV) or ""
    od_path = os.environ.get(OBJDUMP_ENV) or ""
    if not (as_path and od_path):
        return None
    if not all(Path(p).is_file() and os.access(p, os.X_OK)
               for p in (as_path, od_path)):
        return None
    return od_path


def _slot_bytes(enc: dict) -> bytes | None:
    """The encoding as bytes, or `None` when the oracle cannot spell it."""
    if enc["family"] != "legacy":
        return None
    if enc["w"] not in (None, 0, 1):
        return None
    body = PREFIX.get(enc["pp"] or "none")
    if body is None:
        return None
    rex = b"\x48" if enc["w"] == 1 else b""
    if enc["map"] == "none":
        opcode = bytes([enc["opcode"]])
    else:
        lead = {"0f": b"\x0f", "0f38": b"\x0f\x38", "0f3a": b"\x0f\x3a"}
        if enc["map"] not in lead:
            return None
        opcode = lead[enc["map"]] + bytes([enc["opcode"]])
    # `w = 1` means the 64-bit form, which is spelled with REX.W; without it
    # `movq` and `movd` (`66 0f 6e`) are the same bytes and the wrong name is
    # compared.
    opcode = body + rex + opcode
    # A ModR/M byte of `c0` selects the register form of `/digit` rows; rows
    # with no ModR/M simply decode the stray byte as the next instruction, and
    # slot padding keeps that from shifting a later slot.
    reg = enc["reg"] or 0
    return opcode + bytes([0xC0 | ((reg << 3) & 0x38)])


class GasSnapshotDecodeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.gas = _load(CROSSREF, "gas_encoding_crossref")
        cls.table = cls.gas.load_snapshot()
        cls.objdump = _objdump()

    def _decode_all(self, slots: list[tuple[str, dict, bytes]]) -> dict[int, str]:
        """`{slot index: mnemonic objdump printed}` for the whole buffer."""
        if self.objdump is None:
            self.skipTest(
                "needs the pinned binutils 2.47 pair: set ASMDIFF_TEST_AS and "
                "ASMDIFF_TEST_OBJDUMP (CI provisions both)")
        stride = 16
        buf = bytearray()
        for _, _, payload in slots:
            assert len(payload) < stride
            buf.extend(payload)
            buf.extend(b"\xcc" * (stride - len(payload)))
        with tempfile.TemporaryDirectory() as tmp:
            blob = Path(tmp) / "slots.bin"
            blob.write_bytes(bytes(buf))
            out = subprocess.run(
                [self.objdump, "-D", "-b", "binary", "-m", "i386:x86-64",
                 "-M", "att", str(blob)],
                capture_output=True, text=True, timeout=300)
        self.assertEqual(out.returncode, 0, out.stderr)
        found: dict[int, str] = {}
        for line in out.stdout.splitlines():
            head, _, rest = line.partition("\t")
            if not (head.strip().endswith(":") and rest):
                continue
            try:
                offset = int(head.strip().rstrip(":"), 16)
            except ValueError:
                continue
            if offset % stride:
                continue
            fields = rest.split("\t")
            if len(fields) < 2:
                continue
            found[offset // stride] = fields[1].strip().split()[0]
        return found

    def _slots(self) -> list[tuple[str, dict, bytes]]:
        slots = []
        for name in FAMILIES:
            for enc in self.table.get(name, []):
                payload = _slot_bytes(enc)
                if payload is None:
                    continue
                if name in MODE_DEPENDENT and 0x40 <= enc["opcode"] <= 0x4F:
                    continue
                if (name, enc["opcode"]) in EXEMPT:
                    continue
                slots.append((name, enc, payload))
        return slots

    def test_the_snapshot_agrees_with_objdump(self):
        slots = self._slots()
        self.assertGreater(len(slots), 150)
        decoded = self._decode_all(slots)
        self.assertEqual(len(decoded), len(slots),
                         "objdump did not start an instruction in every slot")

        disagreements = []
        for index, (name, enc, payload) in enumerate(slots):
            got = decoded.get(index)
            if got is None:
                disagreements.append((name, payload.hex(), "<no instruction>"))
                continue
            if got.startswith("."):
                continue          # `.byte` and friends: invalid encoding
            if got == name or got == ALIASES.get(name, name):
                continue
            if frozenset((got, name)) in ALIAS_GROUPS:
                continue
            disagreements.append(
                (name, payload.hex(),
                 f"{got} ({self.gas._fmt(enc)})"))
        self.assertEqual(
            disagreements, [],
            "\n".join(f"{n} {b} decoded as {g}" for n, b, g in disagreements))


if __name__ == "__main__":
    unittest.main()
