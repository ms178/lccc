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
import sys
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import godbolt_oracle as G  # noqa: E402
# Imported second: `godbolt_oracle` puts the repo's `scripts/` on sys.path,
# which is where the shared cache module lives.
import godbolt_cache as GC  # noqa: E402


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
    def test_no_file_scope_objects(self):
        """Compiler Explorer's asm view strips `.comm`/`.local`, so a
        file-scope `static` array makes the oracle's code impossible to
        re-assemble locally.  The two programs that had one are the reason
        this check exists."""
        import glob
        import re
        for path in glob.glob(os.path.join(G.PROGRAMS, "*.c")):
            src = open(path).read()
            body = src[src.index("int main"):]        # helpers are above
            self.assertNotRegex(
                body, r"^\s*static\s+\w+[^;]*\[",                      # noqa
                f"{os.path.basename(path)} declares file-scope storage")

    def test_no_rotates_by_the_full_width(self):
        """`x << 32` and `x << 64` are undefined even for unsigned types.
        Clang 23 and GCC fold them differently, so the program has no
        defined answer and the oracle would report a phantom divergence."""
        import glob
        for path in glob.glob(os.path.join(G.PROGRAMS, "*.c")):
            src = open(path).read()
            # Only the helper needs the mask: `rotl(h, (i & 63) + 1)` at a
            # call site is safe precisely because the helper does `r &= 63`.
            if "rotl" in src:
                self.assertRegex(
                    src, r"r\s*&=\s*(31|63)",
                    f"{os.path.basename(path)} rotates without masking the count")

    def test_every_program_prints_something(self):
        import glob
        for path in glob.glob(os.path.join(G.PROGRAMS, "*.c")):
            src = open(path).read()
            self.assertIn("printf", src, os.path.basename(path))


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
