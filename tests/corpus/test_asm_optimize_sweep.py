"""Hold scripts/asm_optimize_sweep.py's own logic to account, without an assembler.

The sweep is a gate, and these are the parts of it a green run never
exercises: which statements it makes on top of the table sweep's, which of
them it leaves out, that the record fails on every difference in both
directions, how a statement's address is read for the run, and -- where a C
compiler for this machine is at hand -- that the run tells a wrong
replacement from a right one.
"""
from __future__ import annotations

import platform
import re
import shutil
import sys
import unittest
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from scripts import asm_optimize_sweep as sweep  # noqa: E402
from scripts import gas_table_sweep as table  # noqa: E402

TABLE = """
add, 0x0, 0, D|W|Modrm, { Reg8|Reg16|Reg32|Reg64, +
                          Reg8|Reg16|Reg32|Reg64|Unspecified|BaseIndex }
add, 0x83/0, 0, Modrm, { Imm8S, Reg16|Reg32|Reg64|Unspecified|BaseIndex }
add, 0x80/0, 0, W|Modrm, { Imm8|Imm16|Imm32|Imm32S, +
                           Reg8|Reg16|Reg32|Reg64|Unspecified|BaseIndex }
sub, 0x28, APX_F, D|W|Modrm|DstVVVV|EVexMap4|NF, { Reg8|Reg16|Reg32|Reg64, +
                           Reg8|Reg16|Reg32|Reg64, Reg8|Reg16|Reg32|Reg64 }
lea, 0x8d, 0, Modrm|No_bSuf|No_sSuf, { Unspecified|BaseIndex, Reg16|Reg32|Reg64 }
nop, 0x90, 0, NoSuf, {}
"""

MODE = table.MODES["64"]


def made(mode=MODE):
    rows = list(table.expand_rows(TABLE))
    return sweep.statements(rows, mode, sweep.PER_ROW, sweep.SEED, set())


# Sixty statements both assemble: forty of `add`, twenty flagless `sub`s.
ASSEMBLED = ([f"add ${n}, %ecx" for n in range(40)]
             + [f"{{nf}} sub ${n}, %ecx, %edx" for n in range(20)])


def report(**levels) -> sweep.Report:
    blank = {"rewritten": [], "equal": 0, "longer": [], "shorter": [], "other": [],
             "kinds": Counter()}
    return sweep.Report(statements=100, assembled=ASSEMBLED,
                        levels={level: {**blank, **levels.get(level, {})}
                                for level in sweep.LEVELS},
                        lost=[], written=[])


class Statements(unittest.TestCase):
    def assertMade(self, statement, statements):
        self.assertTrue(statement in statements, f"`{statement}` is not made")

    def test_what_an_optimizer_looks_at_is_put_in(self):
        statements = made()
        # An immediate at the edges of a width, into a register of each.
        for statement in ("add $0x7f, %cl", "add $0x80, %ecx", "add $0x7fffffff, %ecx",
                          "add $0x80000000, %ecx", "add $0x7fffffff, %rcx", "add $-1, %r9d",
                          "add $0xffff, %cx", "add $0x8000, %r9w", "add $0xff, %sil"):
            self.assertMade(statement, statements)
        # One register in every position.
        for statement in ("add %ecx, %ecx", "add %rcx, %rcx", "sub %ecx, %ecx, %ecx",
                          "sub %r9, %r9, %r9"):
            self.assertMade(statement, statements)
        # Every form of address, for `lea`.
        for statement in ("lea (,%rcx,2), %cx", "lea (%r13,%rcx), %cx", "lea %fs:(%rdx), %cx",
                          "lea (%ecx), %cx", "lea sym, %cx"):
            self.assertMade(statement, statements)
        # `{nf}` in front of the statements of a row that takes it, and of no other.
        self.assertMade("{nf} sub %ecx, %ecx, %ecx", statements)
        self.assertFalse(any(statement.startswith("{nf} add") for statement in statements))
        self.assertFalse(any(statement.startswith("{nf} lea") for statement in statements))

    def test_two_registers_of_each_width(self):
        choices = ["%cl", "%dh", "%sil", "%r9b", "%cx", "%r9w", "%ecx", "%ebp", "%r9d", "%rcx",
                   "%r13", "(%rdx)", "$5", "%xmm1", "%xmm9", "%xmm2"]
        self.assertEqual(sweep.of_each_width(choices),
                         ["%cl", "%sil", "%cx", "%r9w", "%ecx", "%r9d", "%rcx", "%r13", "%xmm1",
                          "%xmm9"])
        self.assertEqual(sweep.of_each_width(["(%rdx)", "sym", "$1"]), [])

    def test_thirty_two_bit_code_has_no_long_registers_or_apx(self):
        statements = made(table.MODES["32"])
        self.assertTrue(statements)
        self.assertFalse([statement for statement in statements
                          if re.search(r"%r[a-z0-9]+", statement)
                          or statement.startswith(("sub ", "{nf}"))])
        self.assertMade("lea (%bx,%si), %cx", statements)

    def test_a_number_that_does_not_fit_is_left_out(self):
        statements = made()
        for statement in ("add $0x7fffffff, %cl", "add $0x8000, %sil", "add $0x80000000, %cx",
                          "addb $0x8000, (%rdx)"):
            self.assertFalse(statement in statements, statement)

    def test_fits_goes_by_suffix_then_register_then_32_bits(self):
        for statement, suffix, fits in (
            ("add $0xff, %cl", "", True),
            ("add $-0x80, %cl", "", True),
            ("add $0x100, %cl", "", False),
            ("add $0xffff, %cx", "", True),
            ("add $0x10000, %cx", "", False),
            ("add $0xffffffff, %ecx", "", True),
            ("add $0x100000000, %ecx", "", False),
            ("add $0x100000000, %rcx", "", True),
            ("addb $0x100, (%rdx)", "b", False),
            ("addw $0xffff, (%rdx)", "w", True),
            ("addq $0x100000000, (%rdx)", "q", True),
            # No suffix and no register: 32 bits.
            ("add $0xffffffff, (%rdx)", "", True),
            ("add $0x100000000, (%rdx)", "", False),
            # The registers of an address say nothing about the operand.
            ("add $0x100, (%rcx)", "b", False),
            ("add $0x7fffffff, 8(%rcx,%rdx)", "", True),
        ):
            self.assertEqual(sweep.fits(statement, suffix), fits, statement)

    def test_the_statements_are_the_same_on_every_run(self):
        self.assertEqual(sorted(made()), sorted(made()))


class Record(unittest.TestCase):
    def test_a_report_is_its_own_expectation(self):
        found = report(O2={"rewritten": ["movq $1, %rax = b801000000"], "equal": 50,
                           "longer": ["vcmpeqss %xmm9, %xmm1, %xmm1"],
                           "shorter": ["add $1, (,%rcx,1)"], "other": ["andnps %xmm1, %xmm1"],
                           "kinds": Counter({("mov", "mov"): 1})})
        text = sweep.expectation_text(found)
        self.assertEqual(sweep.failures_of(found, text), [])
        for line in ("statements 100", "assembled 60", "O2 equal 50",
                     "O2 longer vcmpeqss %xmm9, %xmm1, %xmm1", "O2 kind mov mov 1"):
            self.assertIn(line, text.splitlines())
        # What both assemble, by mnemonic, the pseudo prefix part of the name.
        swept = [line.split()[:3] for line in text.splitlines() if line.startswith("swept ")]
        self.assertEqual(swept, [["swept", "add", "40"], ["swept", "{nf}sub", "20"]])

    def test_every_difference_fails_in_both_directions(self):
        base = report(O2={"equal": 50, "longer": ["vcmpeqss %xmm9, %xmm1, %xmm1"]})
        text = sweep.expectation_text(base)
        changed = (
            # A new longer encoding, and one that is gone.
            report(O2={"equal": 50, "longer": ["vcmpeqss %xmm9, %xmm1, %xmm1",
                                               "movbe %ecx, %ecx"]}),
            report(O2={"equal": 50}),
            # Fewer encodings as GNU as's, and more.
            report(O2={"equal": 49, "longer": ["vcmpeqss %xmm9, %xmm1, %xmm1"]}),
            report(O2={"equal": 51, "longer": ["vcmpeqss %xmm9, %xmm1, %xmm1"]}),
            # Another statement rewritten, or the same one to other bytes.
            report(O2={"equal": 50, "longer": ["vcmpeqss %xmm9, %xmm1, %xmm1"],
                       "rewritten": ["movq $1, %rax = b801000000"]}),
            # The other level.
            report(O2={"equal": 50, "longer": ["vcmpeqss %xmm9, %xmm1, %xmm1"]},
                   Os={"other": ["andnps %xmm1, %xmm1"]}),
        )
        for other in changed:
            self.assertTrue(sweep.failures_of(other, text))
            self.assertTrue(sweep.failures_of(base, sweep.expectation_text(other)))
        rewritten = report(O2={"rewritten": ["movq $1, %rax = b801000000"]})
        again = report(O2={"rewritten": ["movq $1, %rax = 48c7c001000000"]})
        self.assertTrue(sweep.failures_of(again, sweep.expectation_text(rewritten)))

    def test_a_statement_one_of_the_two_stops_assembling_is_named(self):
        """The counts of the levels can stay as they are when a statement leaves
        the sweep and another joins; the line of its mnemonic cannot."""
        base = report()
        text = sweep.expectation_text(base)
        fewer = report()
        fewer.assembled = ASSEMBLED[1:]
        failures = sweep.failures_of(fewer, text)
        self.assertTrue(any(line.startswith("expected, not found:  swept add 40 ")
                            for line in failures), failures)
        self.assertTrue(any(line.startswith("found, not expected:  swept add 39 ")
                            for line in failures), failures)
        self.assertFalse(any("{nf}sub" in line for line in failures), failures)
        # As many, but another one.
        other = report()
        other.assembled = ASSEMBLED[1:] + ["add $99, %ecx"]
        failures = sweep.failures_of(other, text)
        self.assertEqual(sorted(line.split()[3:6] for line in failures),
                         [["swept", "add", "40"], ["swept", "add", "40"]])
        # A mnemonic that is new to the sweep, and one that left it whole.
        more = report()
        more.assembled = ASSEMBLED + ["lea (%rdx), %ecx"]
        self.assertTrue(any("swept lea 1 " in line for line in sweep.failures_of(more, text)))
        gone = report()
        gone.assembled = ASSEMBLED[:40]
        self.assertTrue(any(line.startswith("expected, not found:  swept {nf}sub 20 ")
                            for line in sweep.failures_of(gone, text)))

    def test_a_lost_statement_fails_whatever_the_record_says(self):
        found = report()
        found.lost = ["-O2: movq $1, %rax: ccc: error: nope"]
        self.assertTrue(sweep.failures_of(found, sweep.expectation_text(found)))
        self.assertTrue(sweep.failures_of(found, None))
        found = report()
        found.written = ["movq $1, %rax: lccc 00 gas 01"]
        self.assertTrue(sweep.failures_of(found, None))

    def test_size_is_held_to_the_shorter_of_gnu_as_s_two(self):
        swept = sweep.Sweep(
            ["a", "b"],
            {"O0": {0: (b"\x01\x02\x03", ()), 1: (b"\x01\x02\x03", ())},
             "O2": {0: (b"\x01\x02", ()), 1: (b"\x01\x02", ())},
             "Os": {0: (b"\x01", ()), 1: (b"\x01\x02\x03", ())}},
            {}, {}, {})
        self.assertEqual(sweep.reference(swept, 0, "O2")[0], b"\x01\x02")
        self.assertEqual(sweep.reference(swept, 0, "Os")[0], b"\x01")
        # -Os longer than -O2: the -O2 one.
        self.assertEqual(sweep.reference(swept, 1, "Os")[0], b"\x01\x02")

    def test_a_statement_is_named_with_its_pseudo_prefix(self):
        self.assertEqual(sweep.name_of("movq $1, %rax"), "movq")
        self.assertEqual(sweep.name_of("{nf} add $1, %ecx"), "{nf} add")
        self.assertEqual(sweep.name_of("cltq"), "cltq")


class Run(unittest.TestCase):
    def test_the_address_of_a_statement(self):
        terms, displacement, bits, accessed = sweep.address_of("movl 8(%rdx,%rcx,4), %eax", True)
        self.assertEqual((dict(terms), displacement, bits, accessed), ({2: 1, 1: 4}, 8, 64, True))
        terms, displacement, bits, accessed = sweep.address_of("movl (,%rax,2), %ecx", True)
        self.assertEqual((dict(terms), displacement, bits, accessed), ({0: 2}, 0, 64, True))
        terms, _, bits, accessed = sweep.address_of("leal -0x10(%ecx), %edx", True)
        self.assertEqual((dict(terms), bits, accessed), ({1: 1}, 32, False))
        # One register twice is one term.
        terms, *_ = sweep.address_of("movl (%r13,%r13), %ecx", True)
        self.assertEqual(dict(terms), {13: 2})
        for statement in ("movl %ecx, %eax", "fadd %st(1), %st", "ret"):
            self.assertIsNone(sweep.address_of(statement, True))

    def test_what_can_be_run(self):
        plain, relocated = (b"\x90", ()), (b"\x90", ((0, 1, "sym", 0),))
        self.assertTrue(sweep.runnable("movq $1, %rax", plain, plain))
        self.assertFalse(sweep.runnable("movq $sym, %rax", relocated, plain))
        self.assertFalse(sweep.runnable("movl %fs:(%rdx), %eax", plain, plain))
        # `lea` reads nothing through its segment.
        self.assertTrue(sweep.runnable("leaq %fs:(%rdx), %rax", plain, plain))

    def test_each_replacement_is_run_once(self):
        written, replaced = (b"\x48\xc7\xc0\x01\x00\x00\x00", ()), (b"\xb8\x01\x00\x00\x00", ())
        swept = sweep.Sweep(
            ["mov $1, %rax", "movq $1, %rax", "ret"],
            {},
            {"O0": {0: written, 1: written, 2: (b"\xc3", ())},
             "O2": {0: replaced, 1: replaced, 2: (b"\xc3", ())},
             "Os": {0: replaced, 1: replaced, 2: (b"\xc3", ())}},
            {"O2": {}, "Os": {}}, {})
        self.assertEqual(sweep.pairs_of(swept), [("mov $1, %rax", written[0], replaced[0])])

    def test_what_lccc_writes_otherwise_as_written_is_run_against_gnu_as(self):
        folded, plain = (b"\x83\x11\xfd", ()), (b"\x83\x14\x0d\x00\x00\x00\x00\xfd", ())
        same = (b"\x90", ())
        swept = sweep.Sweep(
            ["adc $-3, (,%rcx,1)", "adcl $-3, (,%rcx,1)", "nop", "mov sym, %eax"],
            {"O0": {0: plain, 1: plain, 2: same, 3: (b"\x8b\x04\x25\0\0\0\0", ((3, 11, "sym", 0),))}},
            {"O0": {0: folded, 1: folded, 2: same, 3: (b"\xa1\0\0\0\0\0\0\0\0", ((1, 1, "sym", 0),))}},
            {}, {})
        self.assertEqual(sweep.written_pairs_of(swept),
                         [("adc $-3, (,%rcx,1)", plain[0], folded[0])])

    def test_the_flags_an_instruction_leaves_undefined(self):
        self.assertEqual(sweep.undefined_flags("andq %rcx, %rcx"), sweep.AF)
        self.assertEqual(sweep.undefined_flags("shll $1, %edx"), sweep.AF)
        self.assertEqual(sweep.undefined_flags("btq $5, %rcx"),
                         sweep.OF | sweep.SF | sweep.AF | sweep.PF)
        for statement in ("subq %r9, %r9", "addl %ecx, %ecx", "movq $1, %rax", "andnps %xmm1, %xmm1"):
            self.assertEqual(sweep.undefined_flags(statement), 0, statement)

    @unittest.skipUnless(platform.machine() in ("x86_64", "AMD64")
                         and (shutil.which("cc") or shutil.which("gcc")),
                         "needs an x86-64 machine and a C compiler")
    def test_a_wrong_replacement_is_told_from_a_right_one(self):
        compiler = sweep.compiler_for(MODE, None)
        self.assertEqual(sweep.self_test(compiler), [])
        # And the program is one per call: nothing is left of the last.
        same = [("movq $1, %rax", bytes.fromhex("48c7c001000000"), bytes.fromhex("b801000000"))]
        ran, skipped, different = sweep.execute(same, MODE, compiler)
        self.assertEqual((ran, skipped, different), (1, 0, []))
        wrong = [("movq $1, %rax", bytes.fromhex("48c7c001000000"), bytes.fromhex("b802000000"))]
        ran, skipped, different = sweep.execute(wrong, MODE, compiler)
        self.assertEqual((ran, skipped, len(different)), (0, 0, 1))


if __name__ == "__main__":
    unittest.main()
