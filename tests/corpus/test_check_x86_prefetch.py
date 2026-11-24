"""Hold scripts/check_x86_prefetch.py's own logic to account, without a compiler.

The gate compares the prefetch instruction two compilers write for one
statement.  A green run exercises neither the reading of a function that has
no instruction, nor a difference, nor the shape of the matrix: these do.
"""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from scripts import check_x86_prefetch as gate  # noqa: E402

# What GCC and lccc write: a tab or four blanks in front of the mnemonic, a
# comment behind a label (Clang), local labels inside a function.
ASSEMBLY = """\t.file\t"prefetch.c"
\t.text
\t.globl\tb_default
\t.type\tb_default, @function
b_default:
.LFB0:
\t.cfi_startproc
\tprefetcht0\t(%rdi)
\tret
\t.cfi_endproc
b_write:                                # @b_write
    prefetchw (%rax)
    ret
b_none:
\tret
m_twice:
\tprefetcht0\t(%rdi)
.L3:
\tprefetchnta\t64(%rdi)
\tret
not_asked:
\tprefetcht2\t(%rdi)
\tret
m_last:
\tprefetchrst2\t(%rdi)
"""


class ReadingTheAssembly(unittest.TestCase):
    NAMES = ["b_default", "b_write", "b_none", "m_twice", "m_last", "m_absent"]

    def setUp(self):
        self.found = gate.prefetches_of(ASSEMBLY, self.NAMES)

    def test_each_function_has_the_instructions_between_its_label_and_the_next(self):
        self.assertEqual(self.found["b_default"], ["prefetcht0"])
        self.assertEqual(self.found["b_write"], ["prefetchw"])
        self.assertEqual(self.found["m_last"], ["prefetchrst2"])

    def test_a_function_without_a_prefetch_has_none(self):
        self.assertEqual(self.found["b_none"], [])

    def test_a_function_the_compiler_did_not_write_has_none(self):
        self.assertEqual(self.found["m_absent"], [])

    def test_a_local_label_does_not_end_the_function(self):
        self.assertEqual(self.found["m_twice"], ["prefetcht0", "prefetchnta"])

    def test_another_functions_instruction_is_not_the_one_asked_abouts(self):
        # `not_asked` stands between `m_twice` and `m_last`: its prefetcht2
        # belongs to neither.
        self.assertNotIn("prefetcht2", [m for found in self.found.values() for m in found])
        self.assertNotIn("not_asked", self.found)

    def test_a_mnemonic_is_read_whole(self):
        found = gate.prefetches_of("f:\n\tprefetchit0\tg(%rip)\n\tprefetch\t(%eax)\n", ["f"])
        self.assertEqual(found["f"], ["prefetchit0", "prefetch"])

    def test_the_word_in_a_comment_or_a_symbol_is_no_instruction(self):
        found = gate.prefetches_of("f:\n\tcall\tprefetch_helper\n\t# prefetcht0 here\n", ["f"])
        self.assertEqual(found["f"], [])


class Comparing(unittest.TestCase):
    def test_the_same_instructions_are_no_difference(self):
        same = {"a": ["prefetcht0"], "b": []}
        self.assertEqual(gate.differences(same, {"a": ["prefetcht0"], "b": []}), [])

    def test_another_instruction_is_a_difference(self):
        self.assertEqual(gate.differences({"a": ["prefetchw"]}, {"a": ["prefetcht0"]}),
                         ["a: GCC prefetchw, lccc prefetcht0"])

    def test_a_missing_instruction_is_a_difference(self):
        # What `_mm_prefetch` was: nothing where GCC writes the hint.
        self.assertEqual(gate.differences({"a": ["prefetcht0"]}, {"a": []}),
                         ["a: GCC prefetcht0, lccc nothing"])

    def test_an_instruction_gcc_does_not_write_is_a_difference(self):
        # A pre-SSE 32-bit CPU: the opcode is undefined there.
        self.assertEqual(gate.differences({"a": []}, {"a": ["prefetcht0"]}),
                         ["a: GCC nothing, lccc prefetcht0"])

    def test_one_instruction_twice_is_a_difference(self):
        self.assertEqual(len(gate.differences({"a": ["prefetcht0"]},
                                              {"a": ["prefetcht0", "prefetcht0"]})), 1)


class TheMatrix(unittest.TestCase):
    def test_every_cpu_of_the_table_is_asked_in_the_modes_it_exists_in(self):
        cpus = gate.cpus()
        self.assertGreater(len(cpus), 100)
        labels = {label for label, _, _ in gate.rows()}
        for name, wide in cpus:
            self.assertIn(f"-m32 -march={name}", labels)
            self.assertEqual(f"-m64 -march={name}" in labels, wide, name)
        # A CPU that is 32-bit only, and the two with MOVRS.
        self.assertIn(("i686", False), cpus)
        self.assertIn(("diamondrapids", True), cpus)
        self.assertIn(("novalake", True), cpus)

    def test_every_cpu_is_asked_plain_and_with_each_option_that_moves_a_prefetch(self):
        asked = {tuple(flags) for _, flags, narrow in gate.rows() if narrow}
        for extra in ([], ["-mno-sse"], ["-msse"], ["-mprfchw"], ["-mmovrs"],
                      ["-mno-sse", "-mprfchw"]):
            self.assertIn(("-march=pentium3", *extra), asked)

    def test_the_labels_are_the_command_lines(self):
        for label, flags, narrow in gate.rows():
            self.assertEqual(label, " ".join(["-m32" if narrow else "-m64", *flags]))

    def test_no_target_is_asked_twice(self):
        labels = [label for label, _, _ in gate.rows()]
        self.assertEqual(len(labels), len(set(labels)))

    def test_a_32_bit_cpu_is_not_asked_of_the_64_bit_compiler(self):
        for label, flags, narrow in gate.rows():
            if not narrow:
                for cpu, wide in gate.cpus():
                    if not wide:
                        self.assertNotIn(f"-march={cpu}", flags, label)


class TheFixture(unittest.TestCase):
    def setUp(self):
        self.text, self.names = gate.fixture()

    def test_every_function_is_one_statement_under_a_name_of_its_own(self):
        self.assertEqual(len(self.names), len(set(self.names)))
        for name in self.names:
            self.assertEqual(self.text.count(f"void {name}(const char *p) {{"), 1, name)

    def test_all_twelve_pairs_of_the_builtin_are_there(self):
        for rw in range(3):
            for locality in range(4):
                self.assertIn(f"__builtin_prefetch(p, {rw}, {locality});", self.text)

    def test_every_named_selector_and_every_raw_one_below_sixteen_is_there(self):
        for hint in gate.NAMED_HINTS:
            self.assertIn(f"_mm_prefetch(p, {hint});", self.text)
        for value in range(16):
            self.assertIn(f"_mm_prefetch(p, {value});", self.text)
        # GCC 16.2's `enum _mm_hint`, all of it.
        self.assertEqual(set(gate.NAMED_HINTS), {
            "_MM_HINT_T0", "_MM_HINT_T1", "_MM_HINT_T2", "_MM_HINT_NTA", "_MM_HINT_ET0",
            "_MM_HINT_RST2", "_MM_HINT_IT0", "_MM_HINT_IT1"})

    def test_the_fixture_reads_the_small_header(self):
        # <immintrin.h> costs GCC a third of a second per target.
        self.assertTrue(self.text.startswith("#include <xmmintrin.h>\n"))
        self.assertIn("_m_prefetchw", gate.WRITE_FIXTURE)
        self.assertNotIn("_m_prefetchw", self.text)


class NoCLibrary(unittest.TestCase):
    def test_the_gates_own_stdlib_stands_in_front_of_the_systems(self):
        source = Path("/somewhere/prefetch.c")
        self.assertEqual(gate.hermetic(source), ["-isystem", "/somewhere/include"])

    def test_it_declares_what_mm_malloc_uses(self):
        for name in ("size_t", "NULL", "malloc", "free"):
            self.assertIn(name, gate.STDLIB)


class TheFirstError(unittest.TestCase):
    def test_the_line_with_the_error_is_the_message(self):
        said = "In file included from a.c:1:\nx.h:3:1: error: unknown type name 'foo'\n  3 | foo\n"
        self.assertEqual(gate.first_error(said), "x.h:3:1: error: unknown type name 'foo'")

    def test_without_an_error_line_the_first_line_is(self):
        self.assertEqual(gate.first_error("collect2: ld returned 1\n"), "collect2: ld returned 1")
        self.assertEqual(gate.first_error(""), "no message")


if __name__ == "__main__":
    unittest.main()
