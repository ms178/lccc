"""Hold scripts/x86_intrinsic_values.py's own logic to account, without a compiler.

The tool is a gate, and a green run exercises none of the parts that make
it one: how a refused test is found in a file of hundreds, how a test that
kills the program is stepped over, how a crash is told from a diagnostic,
and what counts as a failure. A fake compiler (a shell script that refuses
by rule) and fake programs stand in for the real ones.
"""
from __future__ import annotations

import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from scripts import x86_intrinsic_values as values  # noqa: E402


def script(directory: Path, name: str, body: str) -> str:
    path = directory / name
    path.write_text("#!/bin/sh\n" + body)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return str(path)


def shape(*parameters: str) -> list[tuple[str, str]]:
    return values.split_parameters(list(parameters))


class TheTestWritten(unittest.TestCase):
    """One line of C per test: the compiler's line number names the test."""

    def test_a_test_is_one_line(self):
        test = values.Test("_mm_add_ps", "__m128", shape("__m128 __A", "__m128 __B"), {})
        line = test.line(7)
        self.assertNotIn("\n", line)
        self.assertTrue(line.startswith("void v7(void) {"))
        self.assertIn('lv_begin("_mm_add_ps", "_mm_add_ps")', line)
        self.assertIn("__m128 r = _mm_add_ps(a0, a1); lv_mix(&r, sizeof r);", line)

    def test_floats_are_filled_as_floats_and_integers_as_integers(self):
        self.assertEqual(values.fill_kind("__m128"), 1)
        self.assertEqual(values.fill_kind("__m256d"), 2)
        self.assertEqual(values.fill_kind("const double *"), 2)
        self.assertEqual(values.fill_kind("__m512i"), 0)
        self.assertEqual(values.fill_kind("unsigned long long"), 0)
        line = values.Test("_mm_cvtps_pd", "__m128d", shape("__m128 __A"), {}).line(0)
        self.assertIn("lv_fill(&a0, sizeof a0, 1);", line)

    def test_what_a_pointer_to_non_const_points_at_is_hashed(self):
        store = values.Test("_mm_storeu_ps", "void", shape("float *__P", "__m128 __A"), {})
        line = store.line(0)
        self.assertIn("lv_mix(b0, 256);", line)
        self.assertIn("(float *)(void *)(b0 + 1)", line, "an unaligned store gets an odd address")
        load = values.Test("_mm_loadu_ps", "__m128", shape("float const *__P"), {})
        self.assertNotIn("lv_mix(b0", load.line(0), "what is only read is not a result")

    def test_an_aligned_access_gets_an_aligned_address(self):
        line = values.Test("_mm_load_ps", "__m128", shape("float const *__P"), {}).line(0)
        self.assertIn("(b0 + 0)", line)

    def test_the_undefined_half_of_a_cast_is_not_hashed(self):
        line = values.Test("_mm256_castps128_ps256", "__m256", shape("__m128 __A"), {}).line(0)
        self.assertIn("lv_mix(&r, 16);", line)

    def test_a_bit_scan_is_not_given_zero(self):
        line = values.Test("_bit_scan_forward", "int", shape("int __A"), {}).line(0)
        self.assertIn("a0 |= 1;", line)

    def test_a_control_word_is_given_only_values_that_do_not_fault(self):
        line = values.Test("_mm_setcsr", "void", shape("unsigned int __I"), {}).line(0)
        self.assertIn(values.ARGUMENT_RULES["_mm_setcsr"], line)
        self.assertIn("a0 = 0;", values.Test("_xgetbv", "long long",
                                             shape("unsigned int __A"), {}).line(0))

    def test_what_reads_the_clock_is_run_and_not_hashed(self):
        self.assertTrue(values.RUN_ONLY.match("__rdtsc"))
        line = values.Test("__rdtsc", "unsigned long long", [], {}).line(0)
        self.assertIn("(void)r;", line)
        self.assertNotIn("lv_mix", line)

    def test_what_changes_the_state_of_the_program_is_not_run(self):
        for name in ("_xsetbv", "_mm_free", "_writefsbase_u64", "_xrstor", "_tile_zero"):
            self.assertTrue(values.NOT_RUN.match(name), name)
        for name in ("_xgetbv", "_mm_add_ps", "_mm_getcsr"):
            self.assertFalse(values.NOT_RUN.match(name), name)


class TheImmediates(unittest.TestCase):
    SHAPE = shape("__m128i __A", "const int __mask")

    def test_a_constant_is_passed_as_written_and_named_in_the_label(self):
        test = values.Test("_mm_shuffle_epi32", "__m128i", self.SHAPE, {1: 27})
        self.assertEqual(test.label, "_mm_shuffle_epi32#27")
        self.assertIn("_mm_shuffle_epi32(a0, 27)", test.line(0))

    def test_the_expression_twin_has_the_value_and_the_arguments_of_the_literal(self):
        literal = values.Test("_mm_shuffle_epi32", "__m128i", self.SHAPE, {1: 27})
        twin = values.Test("_mm_shuffle_epi32", "__m128i", self.SHAPE, {1: 27}, "~")
        self.assertEqual(twin.label, "_mm_shuffle_epi32#27~")
        self.assertIn("_mm_shuffle_epi32(a0, -(-27))", twin.line(0))
        # The same seed: the two tests draw the same arguments, so that the
        # same hash is the same result.
        self.assertEqual(twin.seed, literal.seed)
        self.assertIn('lv_begin("_mm_shuffle_epi32#27~", "_mm_shuffle_epi32#27")', twin.line(0))

    def test_the_probe_reads_the_value_from_an_object_of_the_running_program(self):
        probe = values.Test("_mm_shuffle_epi32", "__m128i", self.SHAPE, {1: 27}, "?")
        line = probe.line(3)
        self.assertEqual(probe.label, "_mm_shuffle_epi32#27?")
        self.assertIn("static volatile int c1 = 27;", line)
        self.assertIn("_mm_shuffle_epi32(a0, c1)", line)
        self.assertEqual(probe.seed, "_mm_shuffle_epi32#27")

    def test_every_candidate_is_a_test_and_some_are_twinned(self):
        tests = values.variants_of("_mm_shuffle_epi32", "__m128i", self.SHAPE, [1])
        literals = [t for t in tests if not t.spelling]
        twins = [t for t in tests if t.spelling == "~"]
        self.assertEqual([t.constants[1] for t in literals], values.IMMEDIATES)
        self.assertEqual(len(twins), values.EXPRESSION_TWINS)
        self.assertEqual([t.constants for t in twins],
                         [t.constants for t in literals[:values.EXPRESSION_TWINS]])

    def test_a_rounding_parameter_gets_rounding_modes_and_a_scale_gets_scales(self):
        self.assertEqual(values.candidates_for("_mm512_add_round_pd", "__R"), values.ROUNDINGS)
        self.assertEqual(values.candidates_for("_mm_i32gather_epi32", "__scale"), values.SCALES)
        self.assertEqual(values.candidates_for("_mm_shuffle_epi32", "__mask"), values.IMMEDIATES)

    def test_many_immediates_are_a_spread_of_their_product_and_the_same_every_time(self):
        wide = shape("__m128i __A", "const int __M", "const int __N", "const int __O")
        first = values.variants_of("_x", "__m128i", wide, [1, 2, 3])
        again = values.variants_of("_x", "__m128i", wide, [1, 2, 3])
        literals = [t for t in first if not t.spelling]
        self.assertEqual(len(literals), values.MOST_VARIANTS)
        self.assertEqual([t.label for t in first], [t.label for t in again])
        self.assertGreater(len({t.constants[1] for t in literals}), 10,
                           "the spread reaches across the first immediate")

    def test_no_immediate_is_one_test(self):
        tests = values.variants_of("_mm_add_ps", "__m128", shape("__m128 __A", "__m128 __B"), [])
        self.assertEqual([t.label for t in tests], ["_mm_add_ps"])

    def test_only_integer_types_may_be_immediates(self):
        self.assertTrue(values.may_be_immediate("const int"))
        self.assertTrue(values.may_be_immediate("unsigned int"))
        self.assertFalse(values.may_be_immediate("__m128i"))
        self.assertFalse(values.may_be_immediate("float"))


class FindingTheRefused(unittest.TestCase):
    """`implicated`: the tests a compiler refuses, out of a file of many."""

    LINES = [f"void v{i}(void) {{ {'BAD' if i in (2, 5) else 'ok'}; }}" for i in range(8)]

    def refused(self, body: str) -> set[int]:
        with tempfile.TemporaryDirectory() as directory:
            tmp = Path(directory)
            compiler = script(tmp, "cc", body)
            return values.implicated([compiler], self.LINES, tmp, head="/* one line */\n")

    def test_an_error_names_its_line(self):
        self.assertEqual(self.refused(
            'f=$(for a; do :; done; echo "$a")\n'
            'grep -n BAD "$f" | while IFS=: read n rest; do echo "c.c:$n:9: error: bad" >&2; done\n'
            'grep -q BAD "$f" && exit 1; exit 0\n'), {2, 5})

    def test_a_note_names_the_test_an_error_in_a_header_is_in(self):
        # GCC reports a bad immediate in its header and the caller in a note.
        self.assertEqual(self.refused(
            'f=$(for a; do :; done; echo "$a")\n'
            'grep -n BAD "$f" | while IFS=: read n rest; do\n'
            '  echo "emmintrin.h:1397:20: error: the last argument must be an 8-bit immediate" >&2\n'
            '  echo "c.c:$n:44: note: called from here" >&2; done\n'
            'grep -q BAD "$f" && exit 1; exit 0\n'), {2, 5})

    def test_an_error_without_a_line_is_found_by_halves(self):
        # An assembler's message, or a crash: nothing names the test.
        self.assertEqual(self.refused(
            'f=$(for a; do :; done; echo "$a")\n'
            'grep -q BAD "$f" && { echo "ccc: error: operand size mismatch" >&2; exit 1; }\n'
            'exit 0\n'), {2, 5})

    def test_one_error_at_a_time_is_followed_to_the_last(self):
        # A compiler that stops at its first error, as lccc's lowering does.
        self.assertEqual(self.refused(
            'f=$(for a; do :; done; echo "$a")\n'
            'n=$(grep -n BAD "$f" | head -1 | cut -d: -f1)\n'
            '[ -n "$n" ] && { echo "c.c:$n:1: error: bad" >&2; exit 1; }\n'
            'exit 0\n'), {2, 5})

    def test_a_compiler_that_takes_everything_refuses_nothing(self):
        self.assertEqual(self.refused("exit 0\n"), set())

    def test_a_mention_of_a_line_outside_the_tests_is_not_a_test(self):
        # Line 1 is the head: an error there implicates no test by name, and
        # the halving finds that every test "fails".
        self.assertEqual(self.refused('echo "c.c:1:1: error: head" >&2; exit 1\n'),
                         set(range(8)))


class RunningThePrograms(unittest.TestCase):
    def run_program(self, body: str, count: int):
        with tempfile.TemporaryDirectory() as directory:
            tmp = Path(directory)
            return values.run(Path(script(tmp, "program", body)), count, tmp)

    def test_every_test_prints_its_hash(self):
        hashes, died = self.run_program(
            'echo "a 00000000000000aa"; echo "b 00000000000000bb"; echo "c 00000000000000cc"\n', 3)
        self.assertEqual(hashes, ["00000000000000aa", "00000000000000bb", "00000000000000cc"])
        self.assertEqual(died, {})

    def test_a_test_that_kills_the_program_is_stepped_over_and_the_rest_run(self):
        # The program dies in test 1 unless the skip file names it.
        body = ('echo "a 00000000000000aa"\n'
                'if grep -qx 1 "$1"; then echo "-"; else kill -SEGV $$; fi\n'
                'echo "c 00000000000000cc"\n')
        hashes, died = self.run_program(body, 3)
        self.assertEqual(hashes, ["00000000000000aa", None, "00000000000000cc"])
        self.assertEqual(list(died), [1])
        self.assertNotEqual(died[1], 0)

    def test_two_tests_that_die_are_both_found(self):
        body = ('for i in 0 1 2 3; do\n'
                '  if [ $i = 1 ] || [ $i = 3 ]; then\n'
                '    if grep -qx $i "$1"; then echo "-"; else exit 9; fi\n'
                '  else echo "t 000000000000000$i"; fi\n'
                'done\n')
        hashes, died = self.run_program(body, 4)
        self.assertEqual(hashes, ["0000000000000000", None, "0000000000000002", None])
        self.assertEqual(died, {1: 9, 3: 9})

    def test_a_failure_behind_the_last_test_is_an_error_of_the_tool(self):
        with self.assertRaises(RuntimeError):
            self.run_program('echo "a 00000000000000aa"; exit 3\n', 1)


class ACrashIsNotADiagnostic(unittest.TestCase):
    TEST = values.Test("_mm_shuffle_epi32", "__m128i", TheImmediates.SHAPE, {1: 27}, "?")

    def verdict(self, body: str, tag: str = "m"):
        with tempfile.TemporaryDirectory() as directory:
            tmp = Path(directory)
            return values.verdict_of([script(tmp, "cc", body)], self.TEST, tmp, tag)

    def test_a_diagnostic_is_a_refusal(self):
        verdict = self.verdict(
            'echo "m.c:3:9: error: the last argument must be an 8-bit immediate" >&2; exit 1\n')
        self.assertEqual(verdict.message, "error: the last argument must be an 8-bit immediate")
        self.assertFalse(verdict.crashed)
        self.assertFalse(verdict.accepted)
        self.assertTrue(verdict.placed)

    def test_an_object_file_is_an_acceptance(self):
        verdict = self.verdict("exit 0\n")
        self.assertTrue(verdict.accepted)
        self.assertFalse(verdict.crashed)

    def test_an_error_of_a_later_stage_names_no_line_of_the_program(self):
        # The assembler's: it has the operand and no source line to name.
        verdict = self.verdict(
            'echo "ccc: error: operand type mismatch for \\`vblendps\'" >&2; exit 1\n')
        self.assertFalse(verdict.accepted)
        self.assertFalse(verdict.placed)
        self.assertIn("operand type mismatch", verdict.message)

    def test_the_place_is_the_one_in_the_file_that_was_asked_about(self):
        # The files of one verdict are named by its tag: another's place is none.
        body = 'echo "r7.c:3:9: error: the last argument must be a 4-bit immediate" >&2; exit 1\n'
        self.assertTrue(self.verdict(body, "r7").placed)
        self.assertFalse(self.verdict(body, "r8").placed)

    def test_an_internal_error_is_a_crash(self):
        verdict = self.verdict(
            "echo \"thread '<unnamed>' panicked at src/backend/x86_common.rs:964:34:\" >&2\n"
            'echo "ccc: internal error: x86 immediate operand is not a compile-time constant" >&2\n'
            'exit 101\n')
        self.assertTrue(verdict.crashed)
        self.assertIn("panicked at", verdict.message)

    def test_an_end_by_a_signal_is_a_crash(self):
        self.assertTrue(self.verdict("kill -SEGV $$\n").crashed)

    def test_gccs_internal_compiler_error_is_a_crash_too(self):
        self.assertTrue(self.verdict(
            'echo "m.c:3:1: internal compiler error: in expand_expr" >&2; exit 4\n').crashed)


class TheConstantsGccRefuses(unittest.TestCase):
    """Which of the refused choices lccc is asked for."""

    def choices(self, name: str, constants: list[dict[int, int]], spelling: str = ""):
        return [values.Test(name, "__m128", TheImmediates.SHAPE, chosen, spelling)
                for chosen in constants]

    def labels(self, tests, invalid):
        return [test.label for test in values.refused_constants(tests, set(invalid))]

    def test_the_one_next_to_the_range_and_the_far_ones(self):
        # A 4-bit immediate: 0..15 compile, the rest is refused.
        offered = [0, 1, 8, 15, 16, 0x1B, 0x20, 0xFF, -1, 256, 0x1E4]
        tests = self.choices("_mm_blend_ps", [{1: value} for value in offered])
        invalid = [i for i, value in enumerate(offered) if not 0 <= value <= 15]
        self.assertEqual(self.labels(tests, invalid),
                         ["_mm_blend_ps#16", "_mm_blend_ps#256", "_mm_blend_ps#-1"])

    def test_a_range_that_ends_at_the_byte_has_two_refused_constants(self):
        offered = [0, 255, -1, 256, 0x1E4]
        tests = self.choices("_mm_dp_ps", [{1: value} for value in offered])
        self.assertEqual(self.labels(tests, [2, 3, 4]), ["_mm_dp_ps#256", "_mm_dp_ps#-1"])

    def test_an_intrinsic_gcc_compiles_no_choice_of_is_not_asked_about(self):
        # Not for this processor: every choice is refused, and for another reason.
        tests = self.choices("_mm_roundscale_ps", [{1: 0}, {1: 256}])
        self.assertEqual(self.labels(tests, [0, 1]), [])

    def test_an_intrinsic_that_takes_every_constant_is_not_asked_about(self):
        tests = self.choices("_mm_shuffle_ps", [{1: 0}, {1: 256}, {1: -1}])
        self.assertEqual(self.labels(tests, []), [])

    def test_the_expression_twins_are_not_asked_about(self):
        tests = (self.choices("_mm_blend_ps", [{1: 0}, {1: 16}])
                 + self.choices("_mm_blend_ps", [{1: 16}], "~"))
        self.assertEqual(self.labels(tests, [1, 2]), ["_mm_blend_ps#16"])

    def test_two_immediates_are_ordered_by_what_they_add_up_to(self):
        tests = self.choices("_mm_x", [{1: 0, 2: 0}, {1: 8, 2: 4}, {1: 0, 2: 4}, {1: 256, 2: 0}])
        self.assertEqual(self.labels(tests, [1, 2, 3]), ["_mm_x#0,4", "_mm_x#256,0"])


class WhatFails(unittest.TestCase):
    def test_a_clean_outcome_passes(self):
        outcome = values.Outcome()
        outcome.tested["_mm_add_ps"] = 2
        outcome.wants_a_constant.add("_mm_shuffle_epi32")
        outcome.takes_a_value.add("_mm_slli_epi32")
        outcome.not_here.add("_mm256_dpbssd_epi32")
        outcome.not_executable.add("_x")
        self.assertFalse(outcome.failed())

    def test_each_kind_of_finding_fails_on_its_own(self):
        for field in ("differ", "dies", "refused", "not_assembly", "crashes", "accepts",
                      "late"):
            outcome = values.Outcome()
            getattr(outcome, field)["_mm_add_ps"] = ["_mm_add_ps -O2"]
            self.assertTrue(outcome.failed(), field)

    def test_merging_keeps_every_finding(self):
        total, part = values.Outcome(), values.Outcome()
        part.differ["_a"] = ["_a -O0"]
        part.crashes["_b"] = ["_b#1? -O2: panicked"]
        part.tested["_a"] = 2
        part.takes_a_value.add("_c")
        part.wants_a_constant.add("_d")
        part.not_here.add("_e")
        total.merge(part)
        self.assertEqual(total.differ, {"_a": ["_a -O0"]})
        self.assertEqual(total.crashes, {"_b": ["_b#1? -O2: panicked"]})
        self.assertEqual(total.tested, {"_a": 2})
        self.assertEqual((total.takes_a_value, total.wants_a_constant, total.not_here),
                         ({"_c"}, {"_d"}, {"_e"}))
        self.assertTrue(total.failed())


class TheCommandLine(unittest.TestCase):
    def test_a_missing_compiler_is_an_error_and_no_pass(self):
        with open(os.devnull, "w") as quiet:
            stderr, sys.stderr = sys.stderr, quiet
            try:
                self.assertEqual(values.main(["--lccc", "/nonexistent/lccc", "--check"]), 2)
            finally:
                sys.stderr = stderr

    def test_another_gcc_than_the_one_whose_headers_are_the_list_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            tmp = Path(directory)
            lccc = script(tmp, "lccc", "exit 0\n")
            old = script(tmp, "gcc-old", 'echo 12.2.0\n')
            with open(os.devnull, "w") as quiet:
                stderr, sys.stderr = sys.stderr, quiet
                try:
                    self.assertEqual(values.main(["--lccc", lccc, "--gcc", old, "--check"]), 2)
                finally:
                    sys.stderr = stderr


if __name__ == "__main__":
    unittest.main()
