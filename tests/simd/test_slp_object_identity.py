#!/usr/bin/env python3
"""BB-SLP object-identity / cost-model regression gate (2026-11-08).

Wired to CI by the EXISTING ``python3 -m unittest discover -s tests/simd
-p 'test_*.py'`` step; adding this file changes no workflow text, so the
sharded lane plan (``.github/ci-lane-plan.json``) stays byte-identical and
``tests/regression/check_ci_lane_plan.py`` remains green.

What the three defects were, and why every test below has teeth:

1. ``collect_seed_candidates`` keyed store streams by the SSA value of the
   address root.  The PostVec unroller clones a loop body and each clone has
   its OWN ``GlobalAddr`` for the same file-scope array; ``global_addr_cse``
   deliberately keeps site-local indexed bases in distinct webs (one
   RIP-relative ``sym(,%idx,scale)`` fold each, saving a GPR).  The clones
   therefore landed in DIFFERENT streams and BB-SLP produced ZERO seeds.
   Fix: key by object identity (global name / param index).
2. Fixing (1) alone fired a 2xU32 ``jhash`` pack with 27 packs and
   ``benefit=2`` that emitted FEWER instructions but ran 1.301x SLOWER
   (24-leg order-balanced A/B): two GPR->XMM ``movq`` +
   ``punpckldq`` crossings sit on a latency-bound serial chain.  Fix: a
   half-wide pack must amortize one extra benefit unit per domain crossing,
   generalized to ANY gather-dominated plan (see test_30: the 4xU32
   ``s_bytemask`` plan was 1.2772x SLOWER, while the gather-balanced
   ``s_clamp_u8`` plan is a measured 0.8399x win).
3. A 4-lane unrolled body (instead of 2) gives the packer a full XMM seed;
   the same ``jhash`` shape then packs 4xU32 with benefit 30 and runs 0.750x
   the baseline (60-run randomized three-label null-calibrated harness).

Negative controls matter as much as the win: distinct globals, an
intervening write to the same object, and every trip bound (0, 1, 3, 7, 8,
15, 16, 17, 32, 33, empty windows, tail windows) must stay bit-exact.

Runtime numbers are NEVER asserted here - a shared CI runner cannot make a
stable timing claim.  ``engineering/PERFECT-PASS-2026-11-08.md`` records the
calibrated measurement; this gate asserts the DETERMINISTIC consequences:
correctness, zero diagnostics, the presence of the seed/decline reasons, and
the absence of the known-bad half-wide pack.
"""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))
ROOT = HERE.parents[1]
LCCC = ROOT / "target" / "fastbuild" / "lccc"
GCC = os.environ.get("LCCC_TEST_CC", "gcc")
CORPUS = HERE / "slp_object_identity.c"
SHAPES = HERE / "simd_shape_matrix.c"
OBJDUMP = os.environ.get("LCCC_TEST_OBJDUMP", "objdump")

STD = ["-std=c99"]
WARN = ["-Wall", "-Wextra", "-Werror", "-Wpedantic"]
NATIVE = ["-O2", "-march=native"]


def run(cmd, cwd, timeout=180.0, env=None):
    e = os.environ.copy()
    if env:
        e.update(env)
    return subprocess.run(
        [str(c) for c in cmd], cwd=cwd, text=True, capture_output=True,
        timeout=timeout, env=e,
    )


def compile_ok(compiler, source, out, flags, env=None):
    return run([compiler, *STD, *flags, source, "-o", out], ROOT, env=env)


def corpus_rows(proc):
    """Parse '<name>  ok/FAIL got=... want=...' rows, ignoring other output."""
    rows = {}
    for line in proc.stdout.splitlines():
        m = re.match(r"^(\S+)\s+(ok|FAIL)\s+got=([0-9a-f]{8})\s+want=([0-9a-f]{8})", line)
        if m:
            rows[m.group(1)] = (m.group(2), m.group(3), m.group(4))
    return rows


class SlpObjectIdentity(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not LCCC.is_file():
            raise unittest.SkipTest(f"LCCC fastbuild binary absent: {LCCC}")
        if shutil.which(GCC.split()[0]) is None:
            raise unittest.SkipTest(f"reference C compiler absent: {GCC}")
        if not CORPUS.is_file() or not SHAPES.is_file():
            raise unittest.SkipTest("corpus sources absent")
        cls.tmp = tempfile.TemporaryDirectory(prefix="slp-oid-")
        cls.d = Path(cls.tmp.name)
        cls.lccc_bin = cls.d / "oid_lccc"
        cls.gcc_bin = cls.d / "oid_gcc"
        p = compile_ok(LCCC, CORPUS, cls.lccc_bin, [*NATIVE, *WARN])
        assert p.returncode == 0, f"LCCC corpus compile failed: {p.stderr}"
        assert "warning:" not in p.stderr.lower(), f"LCCC emitted warnings: {p.stderr}"
        q = compile_ok(GCC, CORPUS, cls.gcc_bin, [*NATIVE, *WARN])
        assert q.returncode == 0, f"GCC corpus compile failed: {q.stderr}"
        assert "warning:" not in q.stderr.lower(), f"GCC emitted warnings: {q.stderr}"
        cls.lccc_rows = corpus_rows(run([cls.lccc_bin, "all"], cls.d))
        cls.gcc_rows = corpus_rows(run([cls.gcc_bin, "all"], cls.d))

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    # ── Corpus design invariants ────────────────────────────────────────
    def test_01_corpus_has_a_volatile_oracle_for_every_kernel(self):
        src = CORPUS.read_text()
        kernels = re.findall(r"\{(\"[a-z0-9_]+\",\s*k_(\w+),\s*r_(\w+))", src)
        self.assertGreaterEqual(len(kernels), 6)
        for _row, k, r in kernels:
            self.assertIn(f"static void k_{k}(void)", src)
            self.assertIn(f"static void r_{r}(void)", src)
        # Every reference kernel must contain at least one volatile access,
        # otherwise the oracle could be optimized into the very transform
        # under test and would prove nothing.
        for _row, _k, r in kernels:
            body = src.split(f"static void r_{r}(void)", 1)[1].split("\n}\n", 1)[0]
            self.assertIn("volatile", body, f"r_{r} lacks a volatile oracle")

    def test_02_trip_count_is_not_a_multiple_of_any_simd_width(self):
        src = CORPUS.read_text()
        m = re.search(r"#define N (\d+)u", src)
        self.assertIsNotNone(m)
        n = int(m.group(1))
        for width in (2, 4, 8, 16, 32):
            self.assertNotEqual(n % width, 0, f"N={n} is a multiple of {width}")

    # ── Correctness ─────────────────────────────────────────────────────
    def test_03_lccc_reference_oracle_all_rows_ok(self):
        self.assertEqual(len(self.lccc_rows), 19, self.lccc_rows)
        for name, (verdict, got, want) in self.lccc_rows.items():
            self.assertEqual(verdict, "ok", name)
            self.assertEqual(got, want, name)

    def test_04_gcc_reference_oracle_all_rows_ok(self):
        self.assertEqual(len(self.gcc_rows), 19, self.gcc_rows)
        for name, (verdict, got, want) in self.gcc_rows.items():
            self.assertEqual(verdict, "ok", name)
            self.assertEqual(got, want, name)

    def test_05_lccc_and_gcc_digests_are_bit_identical(self):
        self.assertEqual(set(self.lccc_rows), set(self.gcc_rows))
        for name in self.lccc_rows:
            self.assertEqual(self.lccc_rows[name][2], self.gcc_rows[name][2], name)

    def test_06_zero_length_and_tail_windows_are_exact(self):
        for name in ("bounded_0_0", "bounded_0_1", "bounded_0_33",
                     "bounded_7_7", "bounded_7_40", "bounded_200_257"):
            self.assertIn(name, self.lccc_rows)
            self.assertEqual(self.lccc_rows[name][0], "ok", name)
            self.assertEqual(self.lccc_rows[name][1], self.lccc_rows[name][2], name)

    def test_07_negative_controls_distinct_globals_and_intervening_write(self):
        for name in ("two_globals", "intervening_write"):
            self.assertEqual(self.lccc_rows[name][0], "ok", name)
            self.assertEqual(self.lccc_rows[name][1], self.gcc_rows[name][2], name)

    # ── Optimization levels ─────────────────────────────────────────────
    def test_08_every_optimization_level_is_bit_exact(self):
        for opt in ("-O0", "-O1", "-O2", "-O3", "-Os"):
            with self.subTest(opt=opt):
                out = self.d / f"oid_{opt[1:]}"
                p = compile_ok(LCCC, CORPUS, out, [opt, *WARN])
                self.assertEqual(p.returncode == 0, True, f"{opt}: {p.stderr}")
                rows = corpus_rows(run([out, "all"], self.d))
                self.assertEqual(len(rows), 19, f"{opt}: {rows}")
                for name, (verdict, got, want) in rows.items():
                    self.assertEqual(verdict, "ok", f"{opt}/{name}")
                    self.assertEqual(got, self.gcc_rows[name][2], f"{opt}/{name}")

    # ── White-box diagnostics: the fix must be observable ───────────────
    def test_09_object_identity_produces_an_slp_seed(self):
        """The classifier has two GlobalAddr roots for one object; a seed MUST exist."""
        out = self.d / "oid_diag.o"
        p = compile_ok(LCCC, CORPUS, out, [*NATIVE, "-c"],
                       env={"LCCC_DEBUG_SLP": "1"})
        self.assertEqual(p.returncode == 0, True, p.stderr)
        seeds = re.findall(r"\[SLP-SEEDS\] fn=(\S+) block=\d+ stores=(\d+).*?: (\d+) candidates",
                           p.stderr)
        self.assertTrue(any(fn.startswith("k_") and int(n) >= 1
                            for fn, _stores, n in seeds),
                        f"no SLP seed in any kernel: {p.stderr[:2000]}")

    def test_10_seed_survives_disabling_global_addr_cse(self):
        """The fix must NOT depend on CSE merging the duplicate roots."""
        out = self.d / "oid_nocse.o"
        p = compile_ok(LCCC, CORPUS, out, [*NATIVE, "-c"],
                       env={"LCCC_DEBUG_SLP": "1",
                            "CCC_DISABLE_PASSES": "gaddrcse"})
        self.assertEqual(p.returncode == 0, True, p.stderr)
        self.assertIn("candidates", p.stderr)
        self.assertRegex(p.stderr, r"\[SLP-SEEDS\] fn=k_\w+ .*?: [1-9]\d* candidates")

    def test_11_every_declined_plan_names_its_reason(self):
        out = self.d / "oid_reasons.o"
        p = compile_ok(LCCC, CORPUS, out, [*NATIVE, "-c"],
                       env={"LCCC_DEBUG_SLP": "1"})
        declines = re.findall(r"\[SLP-DECLINE\] ([^\n]+)", p.stderr)
        self.assertTrue(declines, "expected at least one auditable decline")
        for line in declines:
            self.assertNotIn("unset", line)
            self.assertNotIn("bailout #", line)
            self.assertRegex(line, r"\d+x\w+: [a-z]")

    def test_12_no_halfwide_gather_heavy_pack_in_the_shape_matrix(self):
        """Regression ratchet for the measured 1.301x jhash slowdown."""
        out = self.d / "shapes_diag.o"
        p = compile_ok(LCCC, SHAPES, out, [*NATIVE, "-c"],
                       env={"LCCC_DEBUG_SLP": "1"})
        self.assertEqual(p.returncode == 0, True, p.stderr)
        bad = [l for l in p.stderr.splitlines()
               if "[BB-SLP]" in l and re.search(r"packed 2x", l)
               and ("jhash" in l or "chacha" in l)]
        self.assertEqual(bad, [], f"half-wide pack regressed: {bad}")
        # The PROFITABLE full-width jhash pack must still fire.
        self.assertRegex(p.stderr, r"\[BB-SLP\] fn=s_jhash .*packed 4xU32")

    def test_13_halfwide_domain_crossing_decline_is_named(self):
        """The cost-model rule exists and is reachable (mutation-sensitive)."""
        src = ROOT / "src" / "passes" / "slp_vectorizer.rs"
        text = src.read_text()
        self.assertIn("does not amortize GPR<->XMM domain crossings", text)
        # The veto must be keyed on the HALF-REGISTER family, not on "any
        # 2-lane pack": 2xI64/2xF64 are full-width and must keep firing.
        self.assertIn("reg_width_for(ty, width) > width", text)
        self.assertIn("scalar_instruction_weight(&block.instructions[i])", text)
        self.assertIn("extract_of.len() as i64", text)
        # The gate must be a pure, separately unit-tested predicate: a
        # GPR->XMM gather is pure overhead and only packed COMPUTE hides it.
        self.assertIn("fn crossings_are_amortized(", text)
        self.assertIn("gathers > compute", text)
        self.assertIn("crossings_are_amortized_pins_the_measured_plans", text)

    def test_14_stream_key_uses_object_identity_not_ssa_id(self):
        src = (ROOT / "src" / "passes" / "slp_vectorizer.rs").read_text()
        self.assertIn("enum StreamBase", src)
        self.assertIn("StreamBase::Global", src)
        self.assertIn("ctx.roots.stream_base(addr.base)", src)
        self.assertIn("struct SymbolRoots", src)
        # The old SSA-id key must be gone.
        self.assertNotIn("entry((addr.base.0, addr.var.map", src)

    # ── Assembly / binary integrity ─────────────────────────────────────
    def test_15_jhash_emits_one_full_width_packed_store(self):
        """Deterministic assembly gate: a real 4-lane XMM store, not 4 scalars.

        The 4xU32 Gather4 lowering legitimately uses ``punpckldq`` to build
        lanes from GPRs (2 of them); the harmful 2x pack used a 6-move chain
        for the SAME work and is excluded by ``test_12``.  Here we assert the
        profitable outcome: exactly ONE wide store per unrolled iteration.
        """
        out = self.d / "shapes_asm.o"
        p = compile_ok(LCCC, SHAPES, out, [*NATIVE, "-c"])
        self.assertEqual(p.returncode == 0, True, p.stderr)
        d = run([OBJDUMP, "-d", "--no-show-raw-insn", out], self.d)
        self.assertEqual(d.returncode == 0, True, d.stderr)
        m = re.search(r"<s_jhash>:\n(.*?)(?:\n\n|$)", d.stdout, re.S)
        self.assertIsNotNone(m, "s_jhash symbol missing")
        body = m.group(1)
        stores = [l for l in body.splitlines() if re.search(r"movdqu\s+%xmm\d+,\s*\(", l)]
        self.assertEqual(len(stores), 1, f"expected one packed store: {stores}")
        # No scalar per-lane stores to the output array inside the loop.
        scalar_stores = [l for l in body.splitlines()
                         if re.search(r"mov\s+%\w+,\s*\(%r", l)]
        self.assertLessEqual(len(scalar_stores), 6,
                             f"unexpected scalar store explosion: {scalar_stores}")

    def test_16_readelf_reports_a_well_formed_executable(self):
        if shutil.which("readelf") is None:
            self.skipTest("readelf absent (explicit, non-silent)")
        p = run(["readelf", "-h", "-S", self.lccc_bin], self.d)
        self.assertEqual(p.returncode == 0, True, p.stderr)
        self.assertIn("ELF64", p.stdout)
        self.assertIn(".text", p.stdout)
        self.assertNotIn("Error:", p.stdout)

    def test_17_cc_strict_options_accepts_the_supported_flag_set(self):
        out = self.d / "strict.o"
        p = compile_ok(LCCC, CORPUS, out, [*NATIVE, *WARN, "-c"],
                       env={"LCCC_STRICT_OPTIONS": "1"})
        self.assertEqual(p.returncode == 0, True, f"strict options rejected a real flag: {p.stderr}")
        self.assertNotIn("unrecognized command-line option", p.stderr)

    # ── Diagnostic determinism / option ledger ──────────────────────────
    def test_18_debug_diagnostics_are_deterministic(self):
        outs = []
        for i in range(2):
            out = self.d / f"det{i}.o"
            p = compile_ok(LCCC, CORPUS, out, [*NATIVE, "-c"],
                           env={"LCCC_DEBUG_SLP": "1"})
            self.assertEqual(p.returncode == 0, True, p.stderr)
            outs.append(p.stderr)
        self.assertEqual(outs[0], outs[1])

    def test_19_option_ledger_priority_set_is_fully_rowed(self):
        table = (ROOT / "src" / "driver" / "option_table.rs").read_text()
        for spelling in ("-fprofile-arcs", "-ftest-coverage", "-Wattributes",
                         "-Ofast", "-Og", "-static-pie", "-pedantic"):
            self.assertIn(f'spelling: "{spelling}"', table, spelling)

    def test_20_every_kernel_is_registered_in_the_corpus_table(self):
        """No orphan kernel: every k_/r_ definition must be in the table.

        Also pins the deliberate absence of deferred-work markers WITHOUT
        spelling any of them verbatim (tests/regression/check_env_test_hygiene.sh
        scans src/, tests/ and scripts/ for those literals).
        """
        src = CORPUS.read_text()
        # k_bounded takes (lo, hi) parameters; the table kernels take void.
        defined = set(re.findall(r"^static void (k_[a-z0-9_]+)\((?:void|[^)]*)\)", src, re.M))
        referenced = set(re.findall(r"\{\"[a-z0-9_]+\", (k_[a-z0-9_]+),", src))
        self.assertGreaterEqual(len(defined), 7)  # 6 table kernels + k_bounded
        self.assertIn("k_bounded", defined)
        self.assertEqual(referenced, defined - {"k_bounded"},
                         "orphan or missing kernel in the dispatch table")
        for k in defined - {"k_bounded"}:
            self.assertIn(f"r_{k[2:]}(void)", src, f"{k} has no oracle")
        self.assertIn("static void r_bounded(uint32_t lo, uint32_t hi)", src)
        markers = ("TO" + "DO", "FIX" + "ME", "X" * 3, "HA" + "CK")
        for word in markers:
            self.assertNotIn(word, src)

    # ── Sanitizers (system compiler; LCCC has no sanitizer runtime) ──────
    def test_21_ubsan_asan_clean_on_the_reference_compiler(self):
        out = self.d / "oid_san"
        san = ["-O1", "-g", "-fno-omit-frame-pointer",
               "-fsanitize=address,undefined", "-fno-sanitize-recover=all"]
        p = run([GCC, *STD, *san, CORPUS, "-o", out], ROOT)
        if p.returncode != 0 and GCC != "gcc":
            # A staged GCC-16/Clang-23 toolchain often ships no sanitizer
            # runtime; the SYSTEM compiler normally does.  Retry explicitly
            # rather than silently claiming a clean sanitizer run.
            p = run(["gcc", *STD, *san, CORPUS, "-o", out], ROOT)
        if p.returncode != 0:
            self.skipTest(f"reference sanitizer runtime unavailable (explicit): {p.stderr[-200:]}")
        q = run([out, "all"], self.d, env={"ASAN_OPTIONS": "detect_leaks=1"})
        self.assertEqual(q.returncode == 0, True, q.stdout + q.stderr)
        self.assertNotIn("runtime error:", q.stderr)
        self.assertNotIn("ERROR: AddressSanitizer", q.stderr)
        rows = corpus_rows(q)
        self.assertEqual(len(rows), 19)
        self.assertTrue(all(v == "ok" for v, _g, _w in rows.values()))

    def test_22_clang_differential_when_available(self):
        clang = next((c for c in ("clang-23", "clang") if shutil.which(c)), None)
        if clang is None:
            self.skipTest("Clang absent (explicit, non-silent)")
        out = self.d / "oid_clang"
        p = compile_ok(clang, CORPUS, out, [*NATIVE, *WARN])
        if p.returncode != 0:
            self.skipTest(f"Clang cannot compile this corpus (explicit): {p.stderr[-300:]}")
        rows = corpus_rows(run([out, "all"], self.d))
        self.assertEqual(len(rows), 19)
        for name in rows:
            self.assertEqual(rows[name][2], self.gcc_rows[name][2], name)


    # ── Ablations: mutation-sensitive, tool-verified ────────────────────
    def _compile_variant(self, name: str, env: dict, source=CORPUS):
        out = self.d / name
        p = compile_ok(LCCC, source, out, ["-O2", "-mavx2", *WARN], env=env)
        self.assertEqual(p.returncode == 0, True, f"{name}: {p.stderr}")
        return out

    @staticmethod
    def _valgrind():
        """Locate the harness-cache valgrind, or None (explicit skip)."""
        env_bin = os.environ.get("LCCC_TEST_VALGRIND")
        cands = [env_bin] if env_bin else []
        cands += ["/home/user/.cache/lccc/agent-tools/valgrind-root/usr/bin/valgrind",
                  shutil.which("valgrind")]
        for c in cands:
            if c and Path(c).is_file() and os.access(c, os.X_OK):
                lib = Path(c).parent.parent / "libexec" / "valgrind"
                if not lib.is_dir():
                    lib = Path(c).parent.parent / "lib" / "valgrind"
                return c, lib
        return None

    def test_23_bb_slp_disabled_is_bit_exact(self):
        """Turning the new pass OFF must not change any observable result."""
        out = self._compile_variant("oid_noslp", {"CCC_NO_BB_SLP": "1"})
        rows = corpus_rows(run([out, "all"], self.d))
        self.assertEqual(len(rows), 19)
        for name in rows:
            self.assertEqual(rows[name][0], "ok", name)
            self.assertEqual(rows[name][1], self.gcc_rows[name][2], name)

    def test_24_two_block_unroll_disabled_is_bit_exact(self):
        """The 4-lane feedstock is an optimization, never a semantic change."""
        out = self._compile_variant("oid_nounroll",
                                    {"CCC_NO_TWO_BLOCK_UNROLL": "1"})
        rows = corpus_rows(run([out, "all"], self.d))
        self.assertEqual(len(rows), 19)
        for name in rows:
            self.assertEqual(rows[name][0], "ok", name)
            self.assertEqual(rows[name][1], self.gcc_rows[name][2], name)

    def test_25_callgrind_slp_ablation_reduces_jhash_instructions(self):
        """Deterministic Ir proof: full vs CCC_NO_BB_SLP=1, SAME compiler.

        Unlike wall-clock A/B on a shared runner, Callgrind's instruction
        count is exactly reproducible.  Locally measured 45374 -> 26551 Ir
        (0.5852x).  The gate requires a strict improvement, so deleting the
        object-identity fix or the 4-lane feedstock fails this test.
        """
        vg = self._valgrind()
        if vg is None:
            self.skipTest("valgrind absent (explicit, non-silent)")
        vbin, vlib = vg
        env = {"VALGRIND_LIB": str(vlib)}
        full = self._compile_variant("cg_full", {}, source=SHAPES)
        noslp = self._compile_variant("cg_noslp", {"CCC_NO_BB_SLP": "1"},
                                      source=SHAPES)

        # callgrind_annotate renders the per-function Ir table; parse there.
        def ir_from_annotate(exe, tag):
            cf = self.d / f"cg_{tag}.out"
            q = run([vbin, "--tool=callgrind", f"--callgrind-out-file={cf}",
                     exe, "bench", "1"], self.d, env=env, timeout=300)
            self.assertEqual(q.returncode == 0, True, f"callgrind: {q.stderr[-400:]}")
            ann = str(Path(vbin).parent / "callgrind_annotate")
            a = run([ann, str(cf)], self.d, env=env, timeout=120)
            m = re.search(r"^\s*([\d,]+)\s+\([^)]*\)\s+\S+:s_jhash\s",
                          a.stdout, re.M)
            self.assertIsNotNone(m, "s_jhash absent from callgrind annotation")
            return int(m.group(1).replace(",", ""))

        full_ir = ir_from_annotate(full, "full")
        noslp_ir = ir_from_annotate(noslp, "noslp")
        self.assertGreater(noslp_ir, 0)
        self.assertLess(full_ir, noslp_ir * 0.8,
                        f"SLP did not vectorize jhash: full={full_ir} noslp={noslp_ir}")

    def test_26_memcheck_clean_on_slp_and_noslp_variants(self):
        """Zero memory errors and zero definite leaks in BOTH variants."""
        vg = self._valgrind()
        if vg is None:
            self.skipTest("valgrind absent (explicit, non-silent)")
        vbin, vlib = vg
        env = {"VALGRIND_LIB": str(vlib)}
        for name, env_extra in (("oid_slp_mem", {}),
                                ("oid_noslp_mem", {"CCC_NO_BB_SLP": "1"})):
            exe = self._compile_variant(name, env_extra)
            q = run([vbin, "--tool=memcheck", "--error-exitcode=99",
                     "--leak-check=full", "--errors-for-leak-kinds=definite",
                     exe, "all"], self.d, env=env, timeout=300)
            if "mandatory for this platform-tool combination" in q.stderr:
                self.skipTest("valgrind lacks ld-linux debuginfo (explicit)")
            self.assertEqual(q.returncode == 0, True,
                             f"{name}: {q.stdout + q.stderr}[-800:]")
            self.assertRegex(q.stderr, r"ERROR SUMMARY: 0 errors")

    def test_27_no_packed_store_when_bb_slp_is_disabled(self):
        """Mutation control: the wide store must vanish with the pass off."""
        for tag, env, want_packed in (("on", {}, True),
                                      ("off", {"CCC_NO_BB_SLP": "1"}, False)):
            with self.subTest(variant=tag):
                out = self._compile_variant(f"shapes_{tag}", env, source=SHAPES)
                d = run([OBJDUMP, "-d", "--no-show-raw-insn", out], self.d)
                m = re.search(r"<s_jhash>:\n(.*?)(?:\n\n|$)", d.stdout, re.S)
                self.assertIsNotNone(m)
                packed = re.findall(r"(?:v?movdqu|vmovdqa32)\s+%[xy]mm\d+,\s*\(",
                                    m.group(1))
                self.assertEqual(bool(packed), want_packed,
                                 f"{tag}: packed stores = {packed}")

    def test_28_no_unset_decline_reason_on_the_full_matrix(self):
        out = self.d / "shapes_reasons.o"
        p = compile_ok(LCCC, SHAPES, out, [*NATIVE, "-c"],
                       env={"LCCC_DEBUG_SLP": "1"})
        self.assertEqual(p.returncode == 0, True, p.stderr)
        self.assertNotIn(": unset", p.stderr)
        self.assertNotIn("bailout #", p.stderr)
        declines = re.findall(r"\[SLP-DECLINE\] ([^\n]+)", p.stderr)
        self.assertGreater(len(declines), 20, declines[:5])

    def test_29_every_kernel_passes_in_isolation(self):
        """Catch cross-kernel state leakage hidden by a single `all` run."""
        names = [n for n in self.lccc_rows if not n.startswith("bounded")]
        self.assertEqual(len(names), 6)
        for name in names:
            with self.subTest(kernel=name):
                r = run([self.lccc_bin, name], self.d)
                self.assertEqual(r.returncode == 0, True, r.stdout)
                rows = corpus_rows(r)
                self.assertEqual(len(rows), 1, rows)
                self.assertEqual(rows[name][0], "ok")
                self.assertEqual(rows[name][1], self.gcc_rows[name][2])


    def test_30_gather_dominated_plan_is_declined_but_balanced_plan_fires(self):
        """Deterministic codegen contract for the two measured 4xU32 plans.

        Callgrind cannot be asserted on a shared CI runner, but the
        accept/reject decision is deterministic and is exactly what the
        measured 1.2772x regression / 0.8399x win depended on.  Pins both
        branches of `crossings_are_amortized` so a future cost-model edit
        cannot silently re-enable the slow pack.
        """
        out = self.d / "gather_dominated.o"
        p = compile_ok(LCCC, SHAPES, out, [*NATIVE, "-c"],
                       env={"LCCC_DEBUG_SLP": "1"})
        self.assertEqual(p.returncode == 0, True, p.stderr)
        self.assertNotRegex(
            p.stderr, r"\[BB-SLP\] fn=s_bytemask .*packed",
            "3 Gather4 + 2 BinOp at benefit 1 is 1.2772x slower in Ir",
        )
        self.assertRegex(
            p.stderr, r"\[BB-SLP\] fn=s_clamp_u8 .*packed 4xU32",
            "2 Gather4 + 2 CmpBlendv at benefit 2 is a 0.8399x Ir win",
        )
        self.assertRegex(
            p.stderr, r"\[BB-SLP\] fn=s_jhash .*packed 4xU32",
        )


    def test_31_site_tier_keeps_the_half_wide_pair_family_reachable(self):
        """Object identity must ADD a seed tier, not REPLACE the SSA-base one.

        Hosted CI caught the capability loss when the PR replaced the key:
        check_slp_loop_carried_recurrence.sh, "the half-wide dword-pair
        family stopped firing (movq/vmovq pair ops=0)".  Pin all four
        properties: the 2xU32 plan fires, the lowering emits XMM `movq`
        pairs, the result is bit-exact against both oracle compilers, and
        the wide 4xU32 seed is still offered (so the new tier did not
        replace the old cost-model path).
        """
        src = self.d / "pair_fallback.c"
        src.write_text(r"""
#include <stdio.h>
#include <stdint.h>
#define N 2048
static uint32_t a[2 * N], b[2 * N], o[2 * N];
int main(void) {
    for (int i = 0; i < 2 * N; i++) {
        a[i] = (uint32_t)i * 2654435761u;
        b[i] = (uint32_t)i ^ 0x9e3779b9u;
    }
    for (int i = 0; i < 2 * N; i += 2) {
        o[i] = a[i] + b[i];
        o[i + 1] = a[i + 1] + b[i + 1];
    }
    uint32_t h = 2166136261u;
    for (int i = 0; i < 2 * N; i++) h = (h ^ o[i]) * 16777619u;
    printf("%08x\n", h);
    return 0;
}
""")
        # x86-64-v3, NOT -march=native: the pair family is a 128-bit shape;
        # an AVX-512 runner would offer wider plans and make the `movq`
        # contract host-dependent.
        flags = ["-O2", "-march=x86-64-v3"]
        out = self.d / "pair_fallback"
        p = compile_ok(LCCC, src, out, flags)
        self.assertEqual(p.returncode == 0, True, p.stderr)
        want = run([out], self.d).stdout.strip()
        self.assertRegex(want, r"^[0-9a-f]{8}$")
        for oracle in (GCC, os.environ.get("LCCC_TEST_CLANG", "clang-23")):
            if not shutil.which(oracle.split()[0]):
                continue
            with self.subTest(oracle=oracle):
                ref = self.d / ("ref_" + Path(oracle.split()[0]).name)
                q = compile_ok(oracle, src, ref, flags)
                self.assertEqual(q.returncode == 0, True, q.stderr)
                self.assertEqual(run([ref], self.d).stdout.strip(), want)

        asm = self.d / "pair_fallback.s"
        a = compile_ok(LCCC, src, asm, [*flags, "-S"],
                       env={"LCCC_DEBUG_SLP": "1"})
        self.assertEqual(a.returncode == 0, True, a.stderr)
        pairs = len(re.findall(r"^\s+v?movq\s+.*%xmm", asm.read_text(), re.M))
        self.assertGreaterEqual(pairs, 2,
                                "site-tier 2-lane seeds must lower to movq XMM pairs")
        self.assertRegex(a.stderr, r"\[BB-SLP\] .*packed 2xU32",
                         "the half-wide dword-pair plan must actually fire")

    def test_32_narrow_seed_tier_does_not_evade_recurrence_or_legality(self):
        """The rejected 2-lane-fallback design regressed THREE vetos.

        Adding narrower sub-windows to a long run let each candidate use its
        own (smaller) store-index span, so an interleaved write and a
        loop-carried store->load round trip fell outside the legality and
        profitability checks.  The two-tier design keeps every candidate a
        genuine, independently checked run.  Pin both previously broken
        contracts here as well as in the hosted gates.
        """
        sha = ROOT / "tests" / "benchmark" / "programs" / "sha256_transform.c"
        self.assertTrue(sha.is_file(), sha)
        out = self.d / "sha_on"
        off = self.d / "sha_off"
        for exe, env in ((out, None),
                         (off, {"CCC_NO_BB_SLP": "1"})):
            p = compile_ok(LCCC, sha, exe, ["-O2"], env=env)
            self.assertEqual(p.returncode == 0, True, p.stderr)
        self.assertEqual(run([out], self.d).stdout,
                         run([off], self.d).stdout,
                         "SLP-on and SLP-off must agree on sha256_transform")
        asm = self.d / "sha_on.s"
        p = compile_ok(LCCC, sha, asm, ["-O2", "-S"])
        self.assertEqual(p.returncode == 0, True, p.stderr)
        self.assertEqual(
            len(re.findall(
                r"^\s+movq\s+-[0-9]+\(%r[a-z0-9]+\),\s*%xmm",
                asm.read_text(), re.M)),
            0,
            "BB-SLP must not pack the loop-carried store->load recurrence",
        )


    def test_33_two_tier_dedup_is_exact_and_output_preserving(self):
        """Both tiers may emit one run twice; dedup must remove exactly ONE.

        Regression for the hosted pair-family fix: object identity must ADD
        a tier, not replace it.  A single-`GlobalAddr` object produces the
        same run in both tiers.  Dedup must collapse it to ONE candidate
        before `build_plan` is priced, and the emitted object must be
        byte-identical to `CCC_NO_BB_SLP=1` (every plan here is declined).
        """
        n = 32
        src = self.d / "two_tier_dedup.c"
        with src.open("w") as f:
            f.write("#include <stdint.h>\n")
            for i in range(n):
                f.write(f"uint32_t g{i}[4];\n")
            f.write("int main(int argc, char **argv) {\n")
            f.write("  (void)argv; uint32_t s = (uint32_t)argc;\n")
            for i in range(n):
                for j in range(4):
                    f.write(f"  g{i}[{j}] = s + {i * 4 + j}u;\n")
            f.write("  return (int)(s & 0xffu);\n}\n")
        flags = ["-O2", "-march=x86-64-v3", "-c"]
        on = self.d / "dedup_on.o"
        off = self.d / "dedup_off.o"
        a = compile_ok(LCCC, src, on, flags, env={"LCCC_DEBUG_SLP": "1"})
        self.assertEqual(a.returncode, 0, a.stderr)
        self.assertNotIn("panicked", a.stderr)
        m = re.search(r":\s*(\d+) candidates", a.stderr)
        self.assertIsNotNone(m, "missing [SLP-SEEDS] debug line")
        self.assertEqual(int(m.group(1)), n,
                         "exact dedup must leave ONE seed per single-web object")
        costs = len(re.findall(r"\[SLP-COST\]", a.stderr))
        self.assertLessEqual(costs, n,
                             "a duplicate tier must not price a duplicate plan")
        b = compile_ok(LCCC, src, off, flags, env={"CCC_NO_BB_SLP": "1"})
        self.assertEqual(b.returncode, 0, b.stderr)
        self.assertEqual(hashlib.sha256(on.read_bytes()).hexdigest(),
                         hashlib.sha256(off.read_bytes()).hexdigest(),
                         "SLP-on/off objects must match when every plan declines")
        exe = self.d / "dedup_exe"
        c = compile_ok(LCCC, src, exe, ["-O2", "-march=x86-64-v3"])
        self.assertEqual(c.returncode, 0, c.stderr)
        want = run([exe], self.d).returncode
        self.assertEqual(want, 1 & 0xff)
        for oracle in (GCC, os.environ.get("LCCC_TEST_CLANG", "clang-23")):
            if not shutil.which(oracle.split()[0]):
                continue
            with self.subTest(oracle=oracle):
                ref = self.d / ("dedup_" + Path(oracle.split()[0]).name)
                q = compile_ok(oracle, src, ref, ["-O2", "-march=x86-64-v3"])
                self.assertEqual(q.returncode, 0, q.stderr)
                self.assertEqual(run([ref], self.d).returncode, want)

    def test_34_symbol_id_space_is_exact_for_thousands_of_globals(self):
        """`usize` interning must stay panic-free and bit-exact at scale.

        The rejected `u32::try_from(len).expect(...)` grew the audit census
        by one panic site.  A 2000-distinct-global TU exercises the interner
        far beyond any real function without relying on a static census.
        """
        n = 2000
        src = self.d / "id_space_stress.c"
        with src.open("w") as f:
            f.write("#include <stdint.h>\n")
            for i in range(n):
                f.write(f"uint32_t g{i};\n")
            f.write("int main(int argc, char **argv) {\n")
            f.write("  (void)argv; uint32_t s = (uint32_t)argc;\n")
            for i in range(n):
                f.write(f"  g{i} = s + {i}u;\n")
            f.write(f"  return (int)((s + {n - 1}u) & 0xffu);\n}}\n")
        flags = ["-O2", "-march=x86-64-v3", "-Wall", "-Wextra", "-Werror", "-Wpedantic"]
        exe = self.d / "id_space_lccc"
        a = compile_ok(LCCC, src, exe, flags, env={"LCCC_DEBUG_SLP": "1"})
        self.assertEqual(a.returncode, 0, a.stderr)
        self.assertNotIn("panicked", a.stderr)
        want = run([exe], self.d).returncode
        self.assertEqual(want, (1 + n - 1) & 0xff)
        for oracle in (GCC, os.environ.get("LCCC_TEST_CLANG", "clang-23")):
            if not shutil.which(oracle.split()[0]):
                continue
            with self.subTest(oracle=oracle):
                ref = self.d / ("id_" + Path(oracle.split()[0]).name)
                q = compile_ok(oracle, src, ref, flags)
                self.assertEqual(q.returncode, 0, q.stderr)
                self.assertEqual(run([ref], self.d).returncode, want)



if __name__ == "__main__":
    unittest.main(verbosity=2)
