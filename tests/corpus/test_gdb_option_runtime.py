#!/usr/bin/env python3
"""GDB 18.1 differential runtime checks for the driver option contract.

The executable ledger classifies `-Ofast` and `-Og` as implemented and
`-fno-omit-frame-pointer` as an observable codegen change. A macro/assembly
delta proves the *compiler* changed something; it does not prove the resulting
binary still computes the right answer on the host CPU, that a debugger can
unwind it, or that the frame-pointer option really produces a walkable chain.
This module asks a real debugger, on the host CPU, comparing LCCC against
GCC 16.2 and Clang 23.1.5.

Measured with GNU gdb (GDB) 18.1 on x86-64 (2026-11-05):

* all three compilers print ``46`` for input 5 at -O2/-Ofast/-Og;
* GDB's backtrace at ``leaf`` is ``leaf -> middle -> top -> main`` for all
  three, so LCCC's unwind info is on par with the oracles;
* with `-fno-omit-frame-pointer`, LCCC's `info frame` reports a saved ``rbp``;
  at default -O2 it reports only ``rip``;
* LCCC emits `DW_TAG_formal_parameter` DIEs without `DW_AT_location`, so GDB
  reports ``a=<optimized out>, b=<optimized out>`` while GCC/Clang resolve
  ``a=5, b=7``. That is a tracked feature gap (see the DIE census gate), not a
  silent defect. The `parameter_location_is_never_wrong` test is a forward
  ratchet: if a location is ever emitted, GDB must print the right value.

Resolution uses the repository's single pinned-debugger policy
(`tests/regression/lib/gdb_oracle.sh`): exact banner 18.1, or an explicit skip.
An unpinned debugger never substitutes for the oracle. Dynamic tests skip when
the pinned toolchain is absent; the documentation and harness-integrity tests
always run.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GDB_LIB = ROOT / "tests/regression/lib/gdb_oracle.sh"
CENSUS = ROOT / "tests/regression/check_dwarf_die_census.py"

# Explicit concatenation: a `"""\` leading backslash would become a C line
# splice on line 1 and silently shift every line-table row by one.
SOURCE = (
    "#include <stdio.h>\n"
    "#include <stdlib.h>\n"
    "__attribute__((noinline)) int leaf(int a, int b) { volatile int s = a * 3 + b; return s; }\n"
    "__attribute__((noinline)) int middle(int x) { volatile int y = leaf(x, 7); return y + 1; }\n"
    "__attribute__((noinline)) int top(int n) { volatile int z = middle(n); return z * 2; }\n"
    "int main(int argc, char **argv) { (void)argc; printf(\"%d\\n\", top(atoi(argv[1]))); return 0; }\n"
)
# Derive the line number from the fixture: a hand-written constant drifts the
# moment a line is added above `leaf`, and a stale `break file:N` silently
# becomes "no compiled code, run to exit" -- a green test that asked nothing.
LEAF_LINE = next(
    i for i, line in enumerate(SOURCE.splitlines(), 1) if "int leaf(" in line
)

OPTIONS = ("-O2", "-Ofast", "-Og")
INPUTS = (5, -3, 0, 12345)
FRAME_ORDER = ("leaf", "middle", "top", "main")
# GDB prints `#0  leaf (...)` for the innermost frame but prefixes outer
# frames with the return address: `#1  0x... in middle (...)`. Both shapes are
# one frame; a regex that only matches `in <name>` silently drops frame 0.
FRAME_LINE = re.compile(r"^#\d+\s+(?:0x[0-9a-f]+ in )?(\w+) \(", re.MULTILINE)


def frames_of(gdb_output: str) -> list[str]:
    return FRAME_LINE.findall(gdb_output)


def optional(command: str) -> str | None:
    found = shutil.which(command)
    if found:
        return found
    path = ROOT / command
    return str(path.resolve()) if path.exists() else None


def gdb_oracle() -> tuple[str | None, int]:
    """(absolute path, status): 0 resolved, 3 explicit skip, 1 hard failure."""
    proc = subprocess.run(
        ["bash", str(GDB_LIB), "--print", "--gate", "test_gdb_option_runtime",
         "--allow-missing-debugger"],
        capture_output=True, text=True, timeout=30,
    )
    return (proc.stdout.strip() or None), proc.returncode


class GdbOptionRuntime(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.gdb, cls.gdb_status = gdb_oracle()
        cls.lccc = optional(os.environ.get("CCC", "target/fastbuild/lccc"))
        cls.gcc = optional(os.environ.get("GCC16", "gcc-16"))
        cls.clang = optional(os.environ.get("CLANG23", "clang-23"))
        cls.readelf = optional(os.environ.get("READELF", "readelf"))
        cls.tmp = tempfile.TemporaryDirectory(prefix="lccc-gdb-runtime-")
        cls.addClassCleanup(cls.tmp.cleanup)
        cls.work = Path(cls.tmp.name)
        cls.source = cls.work / "frames.c"
        cls.source.write_text(SOURCE, encoding="utf-8")
        cls._compiled: dict[tuple[str, str], Path | None] = {}

    def require_tools(self, *tools: str | None) -> None:
        if self.gdb_status == 1:
            self.fail("the pinned GDB 18.1 oracle is missing or mismatched")
        if self.gdb_status == 3 or not self.gdb:
            self.skipTest("pinned GNU gdb (GDB) 18.1 not available (explicit skip)")
        if not all(tools):
            self.skipTest("a comparison compiler or readelf is unavailable")

    def compile(self, compiler: str, opt: str, *, extra: tuple[str, ...] = ()) -> Path | None:
        key = (compiler, opt + "".join(extra))
        if key not in self._compiled:
            out = self.work / (f"b{len(self._compiled)}")
            proc = subprocess.run(
                [compiler, "-g", opt, *extra, str(self.source), "-o", str(out)],
                capture_output=True, text=True, timeout=120,
            )
            self._compiled[key] = out if proc.returncode == 0 and out.exists() else None
            if self._compiled[key] is None:
                return None
        return self._compiled[key]

    def run_gdb(self, binary: Path, *commands: str) -> str:
        argv = [self.gdb, "-q", "-batch", "-nx", "-ex", "set confirm off"]
        for command in commands:
            argv += ["-ex", command]
        argv.append(str(binary))
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=120)
        return proc.stdout + proc.stderr

    def test_known_parameter_location_gap_is_documented(self) -> None:
        """Always runs: a measured GDB limitation must never be silent."""
        text = CENSUS.read_text()
        self.assertIn("<optimized out>", text)
        self.assertIn("DW_AT_location", text)
        self.assertIn("feature", text)

    def test_gdb_harness_rejects_an_unresolvable_breakpoint(self) -> None:
        """Negative control: if a binary/debugger pair cannot resolve a symbol,
        the harness must notice instead of reporting a vacuous pass."""
        self.require_tools(self.lccc)
        binary = self.compile(self.lccc, "-O2")
        self.assertIsNotNone(binary, "lccc -g -O2 failed to build the fixture")
        out = self.run_gdb(binary, "break definitely_not_a_symbol_in_this_program",
                           "run 5", "bt 5")
        self.assertIn('Function "definitely_not_a_symbol_in_this_program" not defined', out)
        self.assertNotIn("Breakpoint 1, definitely_not_a_symbol_in_this_program", out)

    def test_runtime_parity_with_gcc_and_clang(self) -> None:
        self.require_tools(self.lccc, self.gcc, self.clang)
        compilers = {"lccc": self.lccc, "gcc": self.gcc, "clang": self.clang}
        results: dict[tuple[str, str, int], str] = {}
        for name, compiler in compilers.items():
            for opt in OPTIONS:
                binary = self.compile(compiler, opt)
                self.assertIsNotNone(binary, f"{name} {opt} did not build")
                for value in INPUTS:
                    proc = subprocess.run([str(binary), str(value)],
                                          capture_output=True, text=True, timeout=60)
                    self.assertEqual(proc.returncode, 0, f"{name} {opt} {value}: {proc.stderr}")
                    results[(name, opt, value)] = proc.stdout.strip()
        for value in INPUTS:
            expected = str(6 * value + 16)
            for name in compilers:
                for opt in OPTIONS:
                    with self.subTest(name=name, opt=opt, value=value):
                        self.assertEqual(results[(name, opt, value)], expected)

    def test_backtrace_parity_at_every_optimisation_level(self) -> None:
        self.require_tools(self.lccc, self.gcc, self.clang)
        for name, compiler in (("lccc", self.lccc), ("gcc", self.gcc),
                               ("clang", self.clang)):
            for opt in OPTIONS:
                with self.subTest(name=name, opt=opt):
                    binary = self.compile(compiler, opt)
                    self.assertIsNotNone(binary)
                    out = self.run_gdb(binary, "break leaf", "run 5", "bt 6")
                    self.assertEqual(frames_of(out), list(FRAME_ORDER),
                                     f"{name} {opt} backtrace:\n{out}")
                    self.assertIn("Breakpoint 1, leaf", out)

    def test_frame_pointer_option_keeps_a_walkable_rbp_chain(self) -> None:
        self.require_tools(self.lccc)
        binary = self.compile(self.lccc, "-O2",
                              extra=("-fno-omit-frame-pointer",))
        self.assertIsNotNone(binary)
        out = self.run_gdb(binary, "break leaf", "run 5", "info frame", "bt 5")
        self.assertRegex(out, r"Saved registers:[\s\S]*?rbp at 0x[0-9a-f]+",
                         "the frame-pointer option did not produce a saved rbp")
        self.assertEqual(frames_of(out), list(FRAME_ORDER))

    def test_line_breakpoint_resolves_for_every_compiler(self) -> None:
        self.require_tools(self.lccc, self.gcc, self.clang)
        # GDB matches the file name recorded in DWARF, which is the absolute
        # path we compiled; a bare basename is not guaranteed to resolve.
        spec = f"{self.source}:{LEAF_LINE}"
        for name, compiler in (("lccc", self.lccc), ("gcc", self.gcc),
                               ("clang", self.clang)):
            with self.subTest(name=name):
                binary = self.compile(compiler, "-O2")
                self.assertIsNotNone(binary)
                out = self.run_gdb(binary, f"break {spec}", "run 5", "bt 5")
                self.assertRegex(out, r"Breakpoint 1 at 0x[0-9a-f]+")
                self.assertIn("Breakpoint 1, leaf", out)

    def test_parameter_location_is_never_wrong(self) -> None:
        """Forward ratchet: LCCC may either omit a parameter location (the
        measured, documented gap) or emit one that GDB resolves to the true
        value. A location that prints a wrong value -- the classic half-correct
        DWARF implementation -- fails here.

        The oracles are checked in the same run, so a setup in which readelf or
        GDB cannot see locations at all cannot pass vacuously.
        """
        self.require_tools(self.lccc, self.gcc, self.readelf)
        oracle = self.compile(self.gcc, "-O2")
        self.assertIsNotNone(oracle)
        oracle_dwarf = subprocess.run(
            [self.readelf, "--debug-dump=info", str(oracle)],
            capture_output=True, text=True, timeout=60,
        ).stdout
        self.assertIn("DW_AT_location", oracle_dwarf)

        binary = self.compile(self.lccc, "-O2")
        self.assertIsNotNone(binary)
        dwarf = subprocess.run(
            [self.readelf, "--debug-dump=info", str(binary)],
            capture_output=True, text=True, timeout=60,
        ).stdout
        # Walk the DIE stream by tag, not by a character window: a nearby
        # global variable's DW_AT_location must not be mistaken for a
        # parameter location (that would demand values LCCC never claimed).
        die = re.compile(r"^\s*<\d+><[0-9a-f]+>:.*\((DW_TAG_\w+)\)")
        attr = re.compile(r"^\s*<[0-9a-f]+>\s+(DW_AT_\w+)\s*:")
        current: str | None = None
        has_location = False
        for line in dwarf.splitlines():
            tag = die.match(line)
            if tag:
                current = tag.group(1)
                continue
            attribute = attr.match(line)
            if attribute and current == "DW_TAG_formal_parameter" \
                    and attribute.group(1) == "DW_AT_location":
                has_location = True
        out = self.run_gdb(binary, "break leaf", "run 5", "info args", "p a+b")
        if has_location:
            self.assertIn("a = 5", out)
            self.assertIn("b = 7", out)
            self.assertRegex(out, r"\$1 = 12")
        else:
            self.assertIn("<optimized out>", out)
            self.assertIn("value has been optimized out", out)


if __name__ == "__main__":
    unittest.main()
