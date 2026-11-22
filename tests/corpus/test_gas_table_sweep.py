"""Hold scripts/gas_table_sweep.py's own logic to account, without an assembler.

The sweep is a gate, and these are the parts of it a green run never
exercises: how the table is read, that what is sampled is the same on every
Python, which statement an assembler's line number means, and that the
expectation logic fails on every difference from the record -- in both
directions, a fix as much as a regression.

Where GNU as is present the gate goes further and plants a defect of every
kind in an assembler before it trusts a sweep (`--self-test`, which
`--check` runs first); these tests need no tool at all.
"""
from __future__ import annotations

import contextlib
import io
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from scripts import gas_table_sweep as sweep  # noqa: E402

TABLE = """
// A comment, and a template with two instances.
<cc:opc, o:0, no:1>

set<cc>, 0xf9<cc:opc>/0, i386, Modrm|No_wSuf|No_lSuf, { Reg8|Unspecified|BaseIndex }
add, 0x0, 0, D|W|Modrm, { Reg8|Reg16|Reg32|Reg64, +
                          Reg8|Reg16|Reg32|Reg64|Unspecified|BaseIndex }
nop, 0x90, 0, NoSuf, {}
shl, 0xd2/4 << 1, 0, W|Modrm, { ShiftCount, Reg8|Reg16 }
addps, 0x0f58, SSE, Modrm|SSE2AVX, { RegXMM, RegXMM }
undefined<nosuch>, 0x1, 0, NoSuf, {}
"""


def report(findings=None, **counts) -> sweep.Report:
    numbers = {"rows": 10, "statements": 40, "assembled": 30, "same": 25}
    numbers.update(counts)
    return sweep.Report(findings=findings or {}, **numbers)


class Table(unittest.TestCase):
    def test_rows_are_read_with_templates_expanded(self):
        rows = list(sweep.expand_rows(TABLE))
        self.assertEqual([row[0] for row in rows],
                         ["seto", "setno", "add", "nop", "shl", "vaddps"])
        # The template's field goes into the opcode; `<<` is no template.
        self.assertEqual([row[1] for row in rows[:2]], ["0xf90/0", "0xf91/0"])
        self.assertEqual(rows[4][1], "0xd2/4 << 1")
        self.assertEqual(rows[0][3], {"Modrm", "No_wSuf", "No_lSuf"})
        self.assertEqual(rows[0][4], [{"Reg8", "Unspecified", "BaseIndex"}])
        # A row continued with `+` is one row; operands stay in AT&T order.
        self.assertEqual(rows[2][4], [{"Reg8", "Reg16", "Reg32", "Reg64"},
                                      {"Reg8", "Reg16", "Reg32", "Reg64",
                                       "Unspecified", "BaseIndex"}])
        self.assertEqual(rows[3][4], [])

    def test_a_row_with_a_direction_bit_is_swept_in_both_directions(self):
        rows = {row[0]: row for row in sweep.expand_rows(
            "mov, 0x88, 0, D|W|Modrm, { Reg32, Reg32|Unspecified|BaseIndex }\n"
            "lea, 0x8d, 0, Modrm, { Unspecified|BaseIndex, Reg32 }\n"
            "fma, 0x68, FMA4, D|Modrm, { RegXMM, RegXMM|Unspecified|BaseIndex, RegXMM, RegXMM }\n")}
        orders = lambda name: sweep.orders_of(rows[name][3], rows[name][4])  # noqa: E731
        self.assertEqual(len(orders("mov")), 2)
        self.assertEqual(orders("mov")[1], list(reversed(rows["mov"][4])))
        self.assertEqual(orders("lea"), [rows["lea"][4]])
        # More than two operands: the first with the second and with the
        # last, where that is another order.
        self.assertEqual(len(orders("fma")), 2)
        self.assertIn("BaseIndex", orders("fma")[1][0])
        made, _ = sweep.statements(list(rows.values()), sweep.MODES["64"], 400, set(), 1)
        self.assertIn("mov %ecx, (%rdx)", made)
        self.assertIn("mov (%rdx), %ecx", made)
        self.assertIn("lea (%rdx), %ecx", made)
        self.assertNotIn("lea %ecx, (%rdx)", made)

    def test_a_gather_is_swept_with_a_vector_index(self):
        rows = list(sweep.expand_rows(
            "vgatherdps, 0x6692, AVX2, Modrm|Vex256|NoSuf|VecSIB256, "
            "{ RegYMM, Dword|Unspecified|BaseIndex, RegYMM }\n"))
        for name in ("64", "32"):
            made, unknown = sweep.statements(rows, sweep.MODES[name], 400, set(), 1)
            self.assertFalse(unknown)
            base = "%rdx" if name == "64" else "%edx"
            self.assertIn(f"vgatherdps %ymm2, ({base},%ymm7,2), %ymm2", made)
            self.assertTrue(all("%ymm7," in statement for statement in made), made)

    def test_commas_inside_brackets_do_not_split(self):
        self.assertEqual(sweep.split_top("a, <b:c, d>, { e, f }, g(h, i)"),
                         ["a", " <b:c, d>", " { e, f }", " g(h, i)"])
        self.assertEqual(sweep.split_top("1 << 2, 3"), ["1 << 2", " 3"])

    def test_the_mode_decides_which_rows_are_swept(self):
        rows = [("a", "0", "x64", set(), []), ("b", "0", "No64", set(), []),
                ("c", "0", "APX_F", set(), []), ("d", "0", "i386&SSE", set(), [])]
        names = lambda mode, only=frozenset(): [  # noqa: E731
            row[0] for row in sweep.rows_of(rows, sweep.MODES[mode], only)]
        self.assertEqual(names("64"), ["a", "c", "d"])
        for mode in ("32", "16", "16gcc"):
            self.assertEqual(names(mode), ["b", "d"])
        self.assertEqual(names("64", {"d"}), ["d"])


class Sampling(unittest.TestCase):
    def test_a_draw_is_the_same_number_everywhere(self):
        # SHA-256 of the seed and the parts: no generator whose sequence a
        # Python release may change. The expectation files mean these
        # statements.
        self.assertEqual(sweep.draw(1, "foreign", "rdrand %xmm1"), 12633246570150434517)
        self.assertEqual(sweep.draw(1, "add", "0x0", "%cl, %dl"), 1916326785247342553)
        self.assertEqual(sweep.draw(7, "x"), 9646028769132121576)
        # The parts are kept apart: ("ab", "c") is not ("a", "bc").
        self.assertNotEqual(sweep.draw(1, "ab", "c"), sweep.draw(1, "a", "bc"))

    def test_a_digest_names_a_set_of_statements(self):
        self.assertEqual(sweep.digest_of(["cmc"]), "81e5538a")
        self.assertEqual(sweep.digest_of(["b", "a"]), sweep.digest_of(["a", "b"]))
        self.assertNotEqual(sweep.digest_of(["a"]), sweep.digest_of(["b"]))

    def test_foreign_registers_go_where_no_row_takes_their_kind(self):
        rows = list(sweep.expand_rows(TABLE))
        mode = sweep.MODES["64"]
        made = sweep.foreign_statements(rows, mode, {"seto"}, 1, share=1)
        # `seto` takes a byte register or memory: every other kind is tried,
        # and a byte register is not.
        self.assertIn("seto %xmm1", made)
        self.assertIn("seto %ecx", made)
        self.assertIn("seto %fs", made)
        self.assertNotIn("seto %cl", made)
        self.assertEqual(len(made), len(sweep.FOREIGN) - 1)
        # 32-bit code has no 64-bit register to try.
        made32 = sweep.foreign_statements(rows, sweep.MODES["32"], {"seto"}, 1, share=1)
        self.assertNotIn("seto %rcx", made32)
        self.assertEqual(len(made32), len(sweep.FOREIGN) - 2)
        # A row with vector operands: vector registers of every width in
        # every combination, a register of its own in each position.
        vector = sweep.foreign_statements(rows, mode, {"vaddps"}, 1, share=1)
        for statement in ("vaddps %xmm1, %xmm2", "vaddps %ymm1, %xmm2", "vaddps %zmm1, %zmm2",
                          "vaddps %xmm1, %zmm2"):
            self.assertIn(statement, vector)
        self.assertEqual(sum(1 for statement in vector
                             if re.fullmatch(r"vaddps %[xyz]mm1, %[xyz]mm2", statement)), 9)
        # One in `share` of them, the same ones every time.
        some = sweep.foreign_statements(rows, mode, {"seto", "add"}, 1, share=3)
        every = sweep.foreign_statements(rows, mode, {"seto", "add"}, 1, share=1)
        self.assertTrue(set(some) < set(every))
        self.assertEqual(some, sweep.foreign_statements(rows, mode, {"seto", "add"}, 1, share=3))


class Batches(unittest.TestCase):
    def test_a_line_number_names_its_statement(self):
        batch = ["nop", "cmc", "stc"]
        for prelude in ("", ".code16"):
            lines = sweep.batch_source(batch, prelude).splitlines()
            for index, statement in enumerate(batch):
                label = lines.index(f"{sweep.TAG}{index}:") + 1
                self.assertEqual(lines[label], f"1:\t{statement}")
                for line in (label, label + 1):
                    self.assertEqual(sweep.statement_of_line(line, prelude), index,
                                     f"{prelude!r} line {line}")
            # The end of the last statement has a label of its own.
            self.assertIn(f"{sweep.TAG}{len(batch)}:", lines)

    def test_every_difference_between_two_encodings_has_a_verdict(self):
        same = (b"\x90", ())
        self.assertEqual(sweep.verdict_of(same, same), ("SAME", ""))
        self.assertEqual(sweep.verdict_of((b"\x90\x90", ()), same)[0], "LONGER")
        self.assertEqual(sweep.verdict_of((b"", ()), same)[0], "SHORTER")
        self.assertEqual(sweep.verdict_of((b"\xf8", ()), same)[0], "DIFF")
        self.assertEqual(sweep.verdict_of((b"\x90", ((0, 2, "ext", None),)), same)[0], "RELOC")


def system_as() -> str | None:
    """An assembler of this machine that assembles 64-bit x86 code, if any."""
    found = shutil.which("as")
    if not found or platform.machine() not in ("x86_64", "AMD64"):
        return None
    probe = subprocess.run([found, "--64", "-o", os.devnull, "-"], input="\tnop\n",
                           text=True, capture_output=True)
    return found if probe.returncode == 0 else None


@unittest.skipUnless(system_as(), "an x86-64 `as` to stand behind the planted drivers")
class EachStatement(unittest.TestCase):
    """`assemble_each`: the statements GNU as refuses, each a file of its own.

    A driver that goes on behind a file it refuses is handed many files in
    one run, any other one file at a time, and what is found is the same.
    The drivers are the self-test's: an assembler behind lccc's command
    line, with a defect planted in one statement each.
    """
    STATEMENTS = ["nop", "nosuchinsn", "clc", "sti", "bswap %xmm0", "nop %xmm0", "cmc", "stc"]
    # Planted: `clc` is assembled as `stc`, `stc` as nothing, a statement
    # with a register that the assembler refuses as `nop`; `sti` and
    # `nop %xmm0` end the run with status 3; `cmc` and what the assembler
    # refuses are refused.
    ACCEPTED = {0: "lccc 90", 2: "lccc f9", 4: "lccc 90", 7: "lccc "}
    CRASHED = {3: "exit status 3", 5: "exit status 3"}

    def driver(self, directory: Path, plant: bool, stops: bool) -> tuple[str, Path]:
        """A planted driver that counts its runs, and the file it counts in."""
        planted = directory / f"planted-{plant}-{stops}"
        planted.write_text(sweep.PLANTED.format(python=sys.executable, plant=plant,
                                                stops=stops, gas=system_as()))
        planted.chmod(0o755)
        runs = directory / f"runs-{plant}-{stops}"
        counting = directory / f"driver-{plant}-{stops}"
        counting.write_text(f"#!/bin/sh\necho run >> '{runs}'\nexec '{planted}' \"$@\"\n")
        counting.chmod(0o755)
        return str(counting), runs

    def each(self, plant: bool, stops: bool, statements: list[str]):
        mode = sweep.MODES["64"]
        with tempfile.TemporaryDirectory() as directory:
            tmp = Path(directory)
            driver, runs = self.driver(tmp, plant, stops)
            work = tmp / "work"
            work.mkdir()
            found = sweep.assemble_each(statements, list(range(len(statements))), driver,
                                        mode, work)
            return found, len(runs.read_text().splitlines())

    def test_a_driver_that_goes_on_is_told_apart_from_one_that_stops(self):
        mode = sweep.MODES["64"]
        with tempfile.TemporaryDirectory() as directory:
            tmp = Path(directory)
            for stops in (False, True):
                driver, _ = self.driver(tmp, False, stops)
                self.assertEqual(sweep.goes_on(driver, mode, tmp), not stops)
            # No driver at all, and one that assembles nothing.
            self.assertFalse(sweep.goes_on(str(tmp / "missing"), mode, tmp))
            self.assertFalse(sweep.goes_on("/bin/false", mode, tmp))

    def test_many_files_in_one_run_find_what_one_file_a_run_finds(self):
        for stops in (False, True):
            (accepted, crashed), _ = self.each(True, stops, self.STATEMENTS)
            self.assertEqual(accepted, self.ACCEPTED, f"stops={stops}")
            self.assertEqual(crashed, self.CRASHED, f"stops={stops}")

    def test_a_driver_that_goes_on_takes_all_of_them_in_one_run(self):
        statements = ["nop", "nosuchinsn", "clc", "cmc %eax", "stc", "cld"] * 20
        expected = {index: {"nop": "lccc 90", "clc": "lccc f8", "stc": "lccc f9",
                            "cld": "lccc fc"}[statement]
                    for index, statement in enumerate(statements)
                    if statement in ("nop", "clc", "stc", "cld")}
        (accepted, crashed), runs = self.each(False, False, statements)
        self.assertEqual((accepted, crashed), (expected, {}))
        # The question whether it goes on, and the statements.
        self.assertEqual(runs, 2)
        (accepted, crashed), runs = self.each(False, True, statements)
        self.assertEqual((accepted, crashed), (expected, {}))
        self.assertEqual(runs, 1 + len(statements))

    def test_a_run_that_dies_says_nothing_about_the_files_behind_it(self):
        # `sti` ends the run: the statements behind it get a run each, and
        # the ones in front of it are not asked again.
        statements = ["nop", "clc", "sti", "stc", "nosuchinsn", "cld"]
        (accepted, crashed), runs = self.each(True, False, statements)
        self.assertEqual(accepted, {0: "lccc 90", 1: "lccc f9", 3: "lccc ", 5: "lccc fc90"})
        self.assertEqual(crashed, {2: "exit status 3"})
        # One question, one run of all, and one each for `sti` and the
        # three behind it.
        self.assertEqual(runs, 2 + 4)


class Expectation(unittest.TestCase):
    FOUND = {
        "SHORTER": [("vaddps %xmm8, %xmm1, %xmm2", "lccc c5 gas c4")],
        "REJECT": [("ccmpb $1, %al", "error"), ("ccmpl $1, %eax", "error"),
                   ("ctestq %rax, %rcx", "error")],
        "ACCEPT": [("vpermd %xmm1, %ymm2, %ymm2", "lccc c4")],
    }

    def found(self, **changes) -> sweep.Report:
        return report({**self.FOUND, **changes}, foreign=20, refused=19)

    def test_a_record_reads_back_as_written(self):
        text = sweep.expectation_text(self.found())
        counts, listed, rejected = sweep.parse_expectation(text)
        self.assertEqual(counts, {"rows": 10, "statements": 40, "assembled": 30, "same": 25,
                                  "foreign": 20, "refused": 19})
        self.assertEqual(listed, {"shorter": {"vaddps %xmm8, %xmm1, %xmm2"},
                                  "accept": {"vpermd %xmm1, %ymm2, %ymm2"}})
        self.assertEqual(rejected, {
            "ccmpb": (1, sweep.digest_of(["ccmpb $1, %al"])),
            "ccmpl": (1, sweep.digest_of(["ccmpl $1, %eax"])),
            "ctestq": (1, sweep.digest_of(["ctestq %rax, %rcx"])),
        })
        self.assertEqual(sweep.failures_against(self.found(), text), [])
        # A sweep without the other direction records nothing about it.
        plain = sweep.expectation_text(report(self.FOUND))
        self.assertNotIn("foreign", sweep.parse_expectation(plain)[0])

    def failures(self, now: sweep.Report, recorded: sweep.Report) -> list[str]:
        return sweep.failures_against(now, sweep.expectation_text(recorded))

    def assert_one(self, failures: list[str], start: str):
        self.assertEqual(len(failures), 1, failures)
        self.assertTrue(failures[0].startswith(start), failures[0])

    def test_every_new_finding_fails(self):
        base = self.found()
        for verdict in sweep.DEFECTS:
            self.assert_one(self.failures(self.found(**{verdict: [("cmc", "x")]}), base),
                            f"{verdict}: cmc")
        more = dict(self.FOUND)
        more["SHORTER"] = more["SHORTER"] + [("vandps %xmm9, %xmm1, %xmm2", "x")]
        self.assert_one(self.failures(report(more, foreign=20, refused=19), base),
                        "SHORTER (not audited): vandps")
        more = dict(self.FOUND)
        more["ACCEPT"] = more["ACCEPT"] + [("rdrand %xmm1", "lccc 0f c7 f1")]
        self.assert_one(self.failures(report(more, foreign=20, refused=19), base),
                        "ACCEPT: rdrand %xmm1")
        more = dict(self.FOUND)
        more["REJECT"] = more["REJECT"] + [("ccmpb $2, %cl", "error")]
        self.assert_one(self.failures(report(more, foreign=20, refused=19), base),
                        "REJECT: `ccmpb`: 2 statements, 1 recorded")
        more = dict(self.FOUND)
        more["REJECT"] = more["REJECT"] + [("add %eax, %ecx", "error")]
        self.assert_one(self.failures(report(more, foreign=20, refused=19), base),
                        "REJECT: `add`: 1 statements, 0 recorded")

    def test_every_fix_fails_until_it_is_recorded(self):
        base = self.found()
        for verdict, start in (("SHORTER", "STALE: `shorter vaddps"),
                               ("ACCEPT", "STALE: `accept vpermd"),
                               ("REJECT", None)):
            fewer = {v: items for v, items in self.FOUND.items() if v != verdict}
            failures = self.failures(report(fewer, foreign=20, refused=19), base)
            if start:
                self.assert_one(failures, start)
            else:
                self.assertEqual(sorted(f[:22] for f in failures),
                                 ["STALE: `reject ccmpb`:", "STALE: `reject ccmpl`:",
                                  "STALE: `reject ctestq`"])

    def test_as_many_refusals_of_other_statements_fail(self):
        # One statement of the mnemonic assembles now and another is
        # refused that was not: the count is the one recorded.
        moved = dict(self.FOUND)
        moved["REJECT"] = [("ccmpb $9, %bl", "error")] + moved["REJECT"][1:]
        self.assert_one(self.failures(report(moved, foreign=20, refused=19), self.found()),
                        "REJECT: `ccmpb`: 1 statements as recorded, but other ones")

    def test_how_much_was_swept_is_part_of_the_record(self):
        base = self.found()
        for key in ("rows", "statements", "assembled", "same", "foreign", "refused"):
            now = self.found()
            setattr(now, key, getattr(now, key) - 1)
            self.assert_one(self.failures(now, base), f"COUNT: {key} is")
        # The other direction recorded and not swept, and the reverse.
        without = report({v: items for v, items in self.FOUND.items() if v != "ACCEPT"})
        self.assertTrue(all(f.startswith(("COUNT: `foreign` is recorded",
                                          "COUNT: `refused` is recorded",
                                          "STALE: `accept"))
                            for f in self.failures(without, base)))
        self.assertEqual(sorted(f[:26] for f in self.failures(base, without)),
                         ["ACCEPT: vpermd %xmm1, %ymm", "COUNT: `foreign` is not re",
                          "COUNT: `refused` is not re"])
        # A record with a count missing is not a record of that count.
        text = "\n".join(line for line in sweep.expectation_text(base).splitlines()
                         if not line.startswith("same "))
        self.assert_one(sweep.failures_against(base, text), "COUNT: `same` is not recorded")

    def test_a_line_the_record_cannot_mean_is_an_error(self):
        good = sweep.expectation_text(self.found())
        for line in ("frobnicate 3", "same many", "reject ccmpb", "reject ccmpb 1 XYZ",
                     "reject ccmpb one 81e5538a", "shorter", "same 25",
                     "accept vpermd %xmm1, %ymm2, %ymm2",
                     "reject ccmpb 1 " + sweep.digest_of(["ccmpb $1, %al"])):
            with self.assertRaises(ValueError, msg=line):
                sweep.parse_expectation(good + line + "\n")
        # Comments and blank lines are not lines of the record.
        sweep.parse_expectation(good + "\n# a note\n")


class CommandLine(unittest.TestCase):
    def refuses(self, *argv: str) -> str:
        errors = io.StringIO()
        with contextlib.redirect_stderr(errors), self.assertRaises(SystemExit) as raised:
            sweep.main(list(argv))
        self.assertEqual(raised.exception.code, 2, argv)
        return errors.getvalue()

    def test_a_record_is_about_the_whole_default_sweep(self):
        for part in (["--only", "add"], ["--no-foreign"], ["--per-row", "2"], ["--seed", "9"]):
            for recording in ("--check", "--update"):
                self.refuses(recording, "--lccc", "lccc", *part)

    def test_modes_exclude_each_other_and_lccc_is_required(self):
        self.refuses("--32", "--16", "--lccc", "lccc")
        self.assertIn("--lccc", self.refuses("--check"))
        self.assertEqual({mode.name for mode in sweep.MODES.values()},
                         {"64", "32", "16", "16gcc"})
        # The other direction: every code in which a size prefix is read
        # its own way.
        self.assertEqual([name for name, mode in sweep.MODES.items() if mode.foreign],
                         ["64", "32", "16"])


if __name__ == "__main__":
    unittest.main()
