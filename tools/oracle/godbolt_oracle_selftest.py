#!/usr/bin/env python3
"""Offline self-test for tools/oracle/godbolt_oracle.py.

The harness talks to Compiler Explorer, so its own logic is easy to get
wrong in ways that produce a CONFIDENT wrong answer rather than an error.
Every case below is a bug that was actually observed while building the
harness, each of which silently reported success:

  * joining Compiler Explorer's `stdout` chunks with "" fused `a` and `b`
    into `ab`, which is indistinguishable from a miscompilation;
  * joining `asm` chunks with "" collapsed the whole body to one line, the
    instruction count became 0, and the size table printed a perfect 0.000
    ratio for every compiler -- a table that looked like a triumph;
  * asking for `asm` and execution in one request yields execution with no
    assembly, i.e. zero size data with no error.

No network: the stream shapes are the ones the API actually returns, and the
cases are checked directly.  Run: tools/oracle/godbolt_oracle_selftest.py
"""

import json
import os
import re
import sys
import unittest
from unittest import mock
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import godbolt_oracle as G  # noqa: E402
# Imported second: `godbolt_oracle` puts the repo's `scripts/` on sys.path,
# which is where the shared cache module lives.
import godbolt_cache as GC  # noqa: E402

# A file-scope ARRAY definition (`static unsigned char tbl[256];`) becomes a
# `.comm`/`.local` symbol, which Compiler Explorer's asm view strips -- the
# oracle's code then cannot be re-assembled locally.  MULTILINE is load-
# bearing: the body handed to this pattern starts at `int main`, so without it
# `^` can match only at offset 0 and the assertion never fires.
FILE_SCOPE_STORAGE_RE = re.compile(r"(?m)^\s*static\s+\w+[^;]*\[")


class TestStreamJoin(unittest.TestCase):
    def test_stdout_chunks_rejoin_with_newline(self):
        # Exactly the shape the API returns: one chunk per line, and the
        # newline belongs to NEITHER chunk.
        chunks = [{"text": "bitops 6c2706411b3f28d9"},
                  {"text": "bitops crc 4905c741"}]
        self.assertEqual(G._join(chunks, "\n"),
                         "bitops 6c2706411b3f28d9\nbitops crc 4905c741")

    def test_joining_stdout_with_empty_fuses_lines(self):
        # The regression itself: "" is the wrong separator here.
        chunks = [{"text": "a"}, {"text": "b"}]
        self.assertNotEqual(G._join(chunks, ""), "a\nb")
        self.assertEqual(G._join(chunks, ""), "ab")

    def test_asm_chunks_count_instructions(self):
        # The real shape: many small chunks, each a line WITHOUT a trailing
        # newline.  That is what made the whole body collapse to one line.
        asm = [{"text": t} for t in
               ("main:", "\tpush    rbp", "\tmov\trbp, rsp",
                "\tsub\trsp, 8", "\tret")]
        text = G._join(asm, "\n")
        # push / mov / sub / ret -- `main:` is a label and must not count.
        self.assertEqual(sum(1 for l in text.splitlines() if G.INSN.match(l)), 4)
        # The regression: one line, zero instructions, and a size table that
        # then reports a perfect 0.000 ratio for every compiler.
        self.assertEqual(
            sum(1 for l in G._join(asm, "").splitlines() if G.INSN.match(l)), 0)

    def test_labels_directives_and_comments_are_not_instructions(self):
        text = ('main:\n\t.cfi_startproc\n\tpush %rbp\n.L2:\n'
                '\t# a comment\n\t.size main, .-main\n\tret\n')
        self.assertEqual(sum(1 for l in text.splitlines() if G.INSN.match(l)), 2)

    def test_normalise_crlf_and_trailing_space(self):
        self.assertEqual(G._norm("a  \r\nb \r\n"), "a\nb")
        self.assertIsNone(G._norm(None))


class TestExecuteResult(unittest.TestCase):
    """`_join` is applied to whichever stream CE returned; both may be empty
    and neither may be a plain string."""

    def test_empty_and_string_forms(self):
        self.assertEqual(G._join(None, "\n"), "")
        self.assertEqual(G._join([], "\n"), "")
        self.assertEqual(G._join("already joined", "\n"), "already joined")


class TestExecutionProtocol(unittest.TestCase):
    def remote(self, response):
        with mock.patch.object(G, 'USE_CACHE', False), mock.patch.object(G, '_ce_call',
                side_effect=[{'code': 0, 'asm': [{'text': '\tret'}]}, response]):
            return G.remote('cg162', 'int main(void){return 7;}', '-O2', True, 5)

    def test_flattened_nonzero_exit_not_build_success(self):
        result = self.remote({'didExecute': True, 'code': 7, 'buildResult': {'code': 0},
                              'stdout': [{'text': 'program'}]})
        self.assertTrue(result['ok']); self.assertEqual(result['exit'], 7)
        self.assertEqual(result['stdout'], 'program')

    def test_nested_nonzero_exit_and_empty_output_not_build_output(self):
        result = self.remote({'didExecute': True, 'code': 0, 'buildResult': {'code': 0, 'stdout': [{'text': 'compiler banner'}]},
                              'stdout': [], 'execResult': {'code': -6, 'stdout': []}})
        self.assertTrue(result['ok']); self.assertEqual(result['exit'], -6)
        self.assertEqual(result['stdout'], '')

    def test_missing_boolean_or_resource_exit_evidence_is_not_green(self):
        for response in ({'didExecute': True, 'buildResult': {'code': 0}},
                         {'didExecute': True, 'code': False, 'buildResult': {'code': 0}},
                         {'didExecute': True, 'code': 0, 'buildResult': {'code': 0}, 'timedOut': True}):
            self.assertFalse(self.remote(response)['ok'])

    def test_att_filter_shape_is_boolean_not_nested_dictionary(self):
        with mock.patch.object(G, '_post', return_value={}) as post:
            G._ce_call('cg162', 'int f(void){return 1;}', '-O2', False, 5)
        options = post.call_args.args[1]['options']
        self.assertIs(options['filters']['intel'], False)
        self.assertIs(options['filters']['directives'], False)

    def test_old_cache_namespace_is_not_reused(self):
        self.assertNotEqual(GC.NS_ORACLE, 'oracle-v1')


class TestOracleTable(unittest.TestCase):
    def test_presets_name_three_independent_vendors(self):
        ids = G.ORACLE_SET["default"]
        self.assertEqual(len(ids), 3, "GNU, LLVM and Intel, one each")
        vendors = {G.VENDOR.get(i) for i in ids}
        self.assertEqual(vendors, {"GNU", "LLVM", "Intel"},
                         "an oracle set of one vendor is not a cross-vendor oracle")

    def test_every_default_oracle_is_classified(self):
        for cid in G.ORACLE_SET["default"]:
            self.assertIn(G.VENDOR.get(cid), {"GNU", "LLVM", "Intel"})


class TestProgramsAreLinkable(unittest.TestCase):
    """The linkability contract applies to the programs this harness RUNS.

    `tests/oracle/programs/` also holds delta kernels claimed by
    `tests/oracle/delta_corpus.json`; those are measured by function name and
    never linked, so `main`/`printf`/file-scope rules do not describe them.
    `TestDirectoryPartition` below pins what does, and pins that the two
    classes are disjoint and total -- so narrowing these three checks cannot
    be used to smuggle a program out of them.
    """

    def test_no_file_scope_objects(self):
        """Compiler Explorer's asm view strips `.comm`/`.local`, so a
        file-scope `static` array makes the oracle's code impossible to
        re-assemble locally.  The two programs that had one are the reason
        this check exists."""
        for prog in G.standalone_programs():
            src = Path(G.PROGRAMS, prog).read_text()
            body = src[src.index("int main"):]        # helpers are above
            self.assertNotRegex(
                body, FILE_SCOPE_STORAGE_RE,
                f"{prog} declares file-scope storage")

    def test_file_scope_pattern_actually_fires(self):
        """Positive control.  This assertion was written with `^` and no
        MULTILINE, so against a body that starts at `int main` it could only
        ever match at offset 0 -- it never fired, and the check that two real
        programs motivated was a no-op for as long as it existed.  A pattern
        that cannot match is not a gate, so the pattern is now pinned against
        the shape it exists to reject."""
        bad = "int main(void)\n{\n\treturn 0;\n}\nstatic unsigned char tbl[256];\n"
        self.assertRegex(bad, FILE_SCOPE_STORAGE_RE)
        good = "int main(void)\n{\n\tstatic int scalar = 0;\n\treturn scalar;\n}\n"
        self.assertNotRegex(good, FILE_SCOPE_STORAGE_RE)

    def test_no_rotates_by_the_full_width(self):
        """`x << 32` and `x << 64` are undefined even for unsigned types.
        Clang 23 and GCC fold them differently, so the program has no
        defined answer and the oracle would report a phantom divergence.

        Deliberately checked over BOTH classes: a kernel is compared against
        oracles too, so an undefined shift poisons its delta exactly as it
        poisons a program's semantics diff."""
        for name in G._all_sources():
            src = Path(G.PROGRAMS, name).read_text()
            # Only the helper needs the mask: `rotl(h, (i & 63) + 1)` at a
            # call site is safe precisely because the helper does `r &= 63`.
            if "rotl" in src:
                self.assertRegex(
                    src, r"r\s*&=\s*(31|63)",
                    f"{name} rotates without masking the count")

    def test_every_program_prints_something(self):
        for prog in G.standalone_programs():
            src = Path(G.PROGRAMS, prog).read_text()
            self.assertIn("printf", src, prog)


class TestDirectoryPartition(unittest.TestCase):
    """Every `.c` under `tests/oracle/programs/` belongs to exactly one class.

    The sweep used to take "everything in the directory" as its definition of
    "a program", which was sound while the directory held only programs.  A
    partition that is not total is worse than no partition: a file in neither
    class is measured by nobody and checked by nothing.
    """

    def test_partition_is_total_and_disjoint(self):
        everything = set(G._all_sources())
        standalone = set(G.standalone_programs())
        kernels = set(G.delta_kernels())
        self.assertEqual(standalone & kernels, set(),
                         "a file cannot be both a runnable program and a kernel")
        self.assertEqual(standalone | kernels, everything,
                         "every .c in tests/oracle/programs is classified")
        self.assertTrue(standalone, "the runnable-program corpus went empty")

    def test_kernels_never_reach_the_sweep(self):
        """The regression this partition exists for: a kernel handed to
        `check()` links, fails on the missing `main`, and is reported as
        `lccc did not build` -- a red row that is a category error."""
        standalone = set(G.standalone_programs())
        for kernel in G.delta_kernels():
            self.assertNotIn(kernel, standalone)
            src = Path(G.PROGRAMS, kernel).read_text()
            self.assertNotIn("int main", src,
                             f"{kernel} is corpus-claimed but has a main; "
                             f"it is a program, not a kernel")

    def test_standalone_programs_are_still_runnable(self):
        for prog in G.standalone_programs():
            src = Path(G.PROGRAMS, prog).read_text()
            self.assertIn("int main", src, f"{prog} has no entry point")

    def test_kernels_export_their_measured_function(self):
        """A `static` measured function can be inlined away or dropped
        entirely, which yields a zero-instruction row that reads as an
        optimal result -- the same silent lie `_label_is_function` blocks on
        the oracle side."""
        with open(G.DELTA_CORPUS) as fh:
            corpus = json.load(fh)
        claimed = 0
        for entry in corpus["entries"]:
            path = os.path.join(G.REPO, entry["source"])
            self.assertTrue(os.path.isfile(path),
                            f"{entry['id']}: corpus claims a missing source {path}")
            src = Path(path).read_text()
            fn = re.escape(entry["function"])
            # Both house styles: `unsigned global_match_probe(` on one line,
            # and the return type on its own line above a column-0 name.
            self.assertRegex(
                src, rf"(?m)^(?:[A-Za-z_][A-Za-z0-9_]*[ \t*]+)?{fn}[ \t]*\(",
                f"{entry['id']}: {entry['function']} is not defined at file scope")
            self.assertNotRegex(
                src, rf"(?m)^\s*static\b[^;{{}}]*\b{fn}\s*\(",
                f"{entry['id']}: measured function {entry['function']} is static")
            if os.path.dirname(path) == G.PROGRAMS:
                claimed += 1
        self.assertGreater(claimed, 0, "no kernel lives in tests/oracle/programs")

    def test_unreadable_corpus_degrades_to_the_old_sweep(self):
        """A JSON typo must not silently empty the sweep, and must not crash
        it either: with no readable corpus nothing is claimed, so every file
        is treated as a program and the linkability checks see all of them.

        Only `G.DELTA_CORPUS` is repointed, at a throwaway path -- the real
        corpus is a committed gate input and this test must not be able to
        edit it.
        """
        import tempfile
        saved = G.DELTA_CORPUS
        try:
            with tempfile.TemporaryDirectory() as d:
                G.DELTA_CORPUS = os.path.join(d, "missing.json")
                self.assertEqual(G.corpus_kernel_sources(), set())
                self.assertEqual(G.standalone_programs(), G._all_sources())

                G.DELTA_CORPUS = os.path.join(d, "empty.json")
                open(G.DELTA_CORPUS, "w").close()          # malformed JSON
                self.assertEqual(G.corpus_kernel_sources(), set())

                # A claim is authoritative by design, so a corpus entry that
                # names a RUNNABLE program removes it from the sweep.  Pinned
                # here so that is a decision rather than a surprise;
                # test_kernels_never_reach_the_sweep rejects it for real files.
                G.DELTA_CORPUS = os.path.join(d, "claims-a-program.json")
                with open(G.DELTA_CORPUS, "w") as fh:
                    json.dump({"entries": [
                        {"id": "x", "source": "tests/oracle/programs/bitops.c",
                         "function": "main"}]}, fh)
                self.assertNotIn("bitops.c", G.standalone_programs())
                self.assertIn("bitops.c", G.delta_kernels())
        finally:
            G.DELTA_CORPUS = saved
        # ...and the real corpus is untouched and still authoritative.
        self.assertEqual(G.DELTA_CORPUS, saved)
        self.assertTrue(os.path.isfile(saved))
        self.assertTrue(G.delta_kernels(), "the corpus claims no kernel")


class TestRevalidateDropsStaleRecords(unittest.TestCase):
    """`--revalidate` must drop exactly the records that no longer describe
    the compiler CE is serving, and keep everything else.

    Offline: `live_semvers` is stubbed, so this needs no network and no rate
    limit.  Every case here is silent when wrong -- a kept drifted record
    publishes one compiler's numbers under another's name, and a dropped good
    record only costs a re-fetch, so neither shows up in a sweep's output.
    """

    LIVE = {"cg162": "16.2", "cclang2310": "23.1.0", "cicxlatest": "(latest)"}

    def setUp(self):
        import tempfile
        import time
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self._saved_cache = GC.CACHE
        GC.CACHE = Path(self._tmp.name)
        self.addCleanup(lambda: setattr(GC, "CACHE", self._saved_cache))
        self._saved_probe = G.live_semvers
        G.live_semvers = lambda cids, timeout=60: dict(self.LIVE)
        self.addCleanup(lambda: setattr(G, "live_semvers", self._saved_probe))
        self.now = time.time()

    def _put(self, cid, semver, age_days, ident):
        """Store one record; `ident` is carried in `reason` so survivors are
        identifiable without depending on the cache key."""
        GC.store_json(GC.NS_ORACLE,
                      {"id": cid, "ok": True, "asm_insns": 1, "reason": ident,
                       "ce_semver": semver,
                       "cached_at": self.now - age_days * 86400.0},
                      cid, "-O2", ident, False)

    def _survivors(self):
        return {json.loads(path.read_text())["reason"]
                for path, _key in GC.iter_records(GC.NS_ORACLE)}

    def test_drift_age_and_provenance(self):
        # pinned and still matching: kept, however old (a pinned channel that
        # has not moved is exactly the case the cache exists for)
        self._put("cg162", "16.2", 200, "pinned-current-old")
        # pinned but CE re-pointed the id: dropped, age irrelevant
        self._put("cg162", "15.1", 1, "pinned-drifted")
        # moving channel, fresh: kept (no version to compare, under the limit)
        self._put("cicxlatest", "(latest)", 3, "moving-fresh")
        # moving channel, old: dropped (age is the only available evidence)
        self._put("cicxlatest", "(latest)", 90, "moving-aged")
        # written before provenance existed: dropped, origin unknown
        GC.store_json(GC.NS_ORACLE, {"id": "cg162", "ok": True, "asm_insns": 1},
                      "cg162", "-O2", "legacy-no-provenance", False)
        # a compiler this run did not ask about: untouched
        self._put("crv64gtrunk", "(trunk)", 400, "other-compiler")

        dropped = G.revalidate({"cg162", "cicxlatest"}, 28.0)
        self.assertEqual(dropped, 3)
        self.assertEqual(self._survivors(),
                         {"pinned-current-old", "moving-fresh", "other-compiler"})

    def test_a_failed_probe_keeps_records(self):
        """Not knowing the current version is a reason to KEEP, never to
        delete: a dropped record costs a re-fetch against a hard rate limit,
        and a probe failure says nothing about the record."""
        G.live_semvers = lambda cids, timeout=60: {}
        self._put("cg162", "16.2", 1, "pinned")
        self._put("cicxlatest", "(latest)", 1, "moving")
        self.assertEqual(G.revalidate({"cg162", "cicxlatest"}, 28.0), 0)
        self.assertEqual(self._survivors(), {"pinned", "moving"})

    def test_zero_max_age_drops_everything_unversioned(self):
        """`--max-age-days 0` means "re-measure the moving channels now"."""
        self._put("cicxlatest", "(latest)", 0.0, "moving")
        self._put("cg162", "16.2", 0.0, "pinned")
        self.assertEqual(G.revalidate({"cicxlatest", "cg162"}, 0.0), 1)
        self.assertEqual(self._survivors(), {"pinned"})


if __name__ == "__main__":
    unittest.main(verbosity=2)
