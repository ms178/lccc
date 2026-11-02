#!/usr/bin/env python3
"""Red-team the addr_value generator without invoking lccc.

The generator must be deterministic, must actually exercise both defect
families, and must emit programs that a known-good compiler passes: a case the
reference cannot compile, or one whose checks fail under the reference, is a
bug in this generator (the first draft applied *different* cast chains to the
two sides of its comparison and failed for exactly that reason), not in the
compiler under test.
"""
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import addr_value_fuzz

REPO_ROOT = Path(__file__).resolve().parents[2]

# One marker per generated shape, so a shape silently dropping out of the
# generator fails this suite instead of shrinking coverage unnoticed.
ADDR_SHAPE_MARKERS = {
    "cast_chain": "(uintptr_t)&s",
    "scalar_assign": "= (%s)a;" % "int",
    "array_base": "uintptr_t b = (uintptr_t)v;",
    "runtime_index": "uintptr_t e = (uintptr_t)&v[i];",
    "struct_union_memcpy": "bx.raw = (uintptr_t)&s;",
    "noinline_arg": "id_u((uintptr_t)&s)",
    "ternary": "? (int)(uintptr_t)&s :",
    "loop": "for (int k = 0; k <",
    "pointer_diff": "char *base = (char *)&buf[0];",
    "padded_member": "char pad[24];",
    "vector_local": "vector_size(16)",
}
CONV_SHAPE_MARKERS = {
    "big_path": "x ^ (1ULL << 63)",
    "plain_unsigned": "\tuint64_t v = x;",
    "negative_signed": "(int64_t)(x | (1ULL << 63))",
    "narrow_unsigned": "(uint32_t)x | 0x80000000u",
    "narrow_signed": "(int32_t)(x | 0x80000000u)",
}
CONV_CHECK_MARKERS = {
    "repeat_conversion": ("= (double)v, q = (double)v", "= (float)v, q = (float)v"),
    "source_survives": ("\treturn w == v;",),
    "two_routes": ("\treturn p == e;",),
    "before_after": ("before == after && after == v;",),
    "interleaved_int_use": ("m == v + 1;",),
    "exact_round_trip": ("(v & 0x7fffffffffffffULL)",),
}


def _corpus(n: int = 60) -> str:
    return "\n".join(addr_value_fuzz.gen_c(seed) for seed in range(n))


class GeneratorShapeTests(unittest.TestCase):
    def test_deterministic_per_seed(self):
        self.assertEqual(addr_value_fuzz.gen_c(11), addr_value_fuzz.gen_c(11))

    def test_seed_changes_the_case_body(self):
        bodies = {addr_value_fuzz.gen_c(s).split("\n", 12)[-1] for s in range(8)}
        self.assertGreater(len(bodies), 1,
                           "the generator ignores its seed")

    def test_every_shape_is_reachable(self):
        corpus = _corpus()
        missing = [f"address shape {name}" for name, marker in ADDR_SHAPE_MARKERS.items()
                   if marker not in corpus]
        missing += [f"conversion source {name}" for name, marker in CONV_SHAPE_MARKERS.items()
                    if marker not in corpus]
        missing += [f"conversion check {name}"
                    for name, markers in CONV_CHECK_MARKERS.items()
                    if not any(m in corpus for m in markers)]
        # Report names only: dumping 60 generated translation units into an
        # assertion message buries the actual failure.
        self.assertEqual(missing, [], "generated corpus lost these shapes")

    def test_every_case_rejects_address_equals_value(self):
        # The anti-vacuity invariant: a compiler that lowers `&s` to `s` makes
        # both sides of a self-comparison agree, so each case must also assert
        # that the address is not the value it addresses.
        for seed in range(20):
            src = addr_value_fuzz.gen_c(seed)
            self.assertIn("if ((uintptr_t)s == a) return 0;", src,
                          f"seed {seed} lacks the anti-vacuity check")

    def test_case_is_self_checking(self):
        src = addr_value_fuzz.gen_c(3)
        self.assertIn('printf(bad ? "FAILURES: %d\\n" : "OK\\n", bad)', src)
        self.assertIn("return bad != 0;", src)
        self.assertIn("int main(void)", src)

    def test_no_absolute_address_escapes_its_own_function(self):
        # Every uintptr_t comparison must involve an object declared in the
        # same function: a callee's stack slot and a caller's stack slot are
        # different addresses, so such a comparison would prove nothing (and
        # fail).  The generator therefore never takes the address of anything
        # but a local declared in the function doing the check.
        for seed in range(20):
            src = addr_value_fuzz.gen_c(seed)
            for line in src.splitlines():
                if "uintptr_t" in line and "extern" in line:
                    self.fail(f"seed {seed} takes a global's address: {line}")
            body = src.split("int main(void)")[0]
            for fn in body.split("static int __attribute__((noinline))")[1:]:
                for line in fn.splitlines():
                    if "&" in line and "uintptr_t" in line:
                        self.assertIsNone(
                            __import__("re").search(r"&\s*(g_|extern_)", line),
                            f"seed {seed}: {line}")

    def test_conversion_sources_use_defined_casts(self):
        # `(uintptr_t)` of a floating value is only defined because every
        # generated initialiser is a small exactly-representable constant; a
        # negative or huge float here would be undefined behaviour and could
        # make the reference and lccc diverge for reasons that are not bugs.
        for seed in range(60):
            for line in addr_value_fuzz.gen_c(seed).splitlines():
                if " s = " in line and line.strip().startswith(("float", "double")):
                    value = line.split("=")[1].strip().rstrip(";")
                    self.assertIn(value, {"1", "2", "7", "10", "1.5", "2.5"},
                                  f"seed {seed}: non-representable FP initialiser")


class ReferenceAgreementTests(unittest.TestCase):
    """The generated corpus must pass under a known-good compiler."""

    def test_generated_cases_pass_under_host_cc(self):
        cc = shutil.which("gcc") or shutil.which("cc")
        if cc is None:
            self.skipTest("no host C compiler")
        for seed in range(6):
            with self.subTest(seed=seed), tempfile.TemporaryDirectory() as tmp:
                src = Path(tmp) / "case.c"
                src.write_text(addr_value_fuzz.gen_c(seed))
                exe = Path(tmp) / "case"
                for level in ("-O0", "-O2"):
                    build = subprocess.run([cc, level, "-w", str(src), "-o", str(exe)],
                                           capture_output=True, text=True)
                    self.assertEqual(build.returncode, 0,
                                     "generator emitted uncompilable C for seed "
                                     f"{seed}: {build.stderr[-400:]}")
                    run = subprocess.run([str(exe)], capture_output=True, text=True)
                    self.assertEqual(run.stdout.strip().splitlines()[-1], "OK",
                                     f"seed {seed} {level}: the checks fail under "
                                     f"the reference compiler: {run.stdout[-400:]}")

    def test_reference_rejection_is_reported_as_skip(self):
        # A case the reference cannot compile must not be scored as a
        # miscompile: the CLI reports it as `skip` with kind
        # `reference_rejected`.  A compiler that always fails stands in for a
        # rejected case here (the CLI invokes it only as the reference).
        with tempfile.TemporaryDirectory() as tmp:
            fake_cc = Path(tmp) / "fake-cc"
            fake_cc.write_text("#!/bin/sh\nexit 1\n")
            fake_cc.chmod(0o755)
            out = Path(tmp) / "out"
            proc = subprocess.run(
                [sys.executable, str(Path(addr_value_fuzz.__file__)),
                 "--ccc", str(fake_cc), "--gcc", str(fake_cc),
                 "--seeds", "0:2", "--levels", "O2", "--out", str(out)],
                capture_output=True, text=True)
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn('"skip": 2', proc.stdout)
            self.assertIn('"mismatch": 0', proc.stdout)
            self.assertIn('"reference_rejected"', (out / "summary.json").read_text())


class HarnessRegistrationTests(unittest.TestCase):
    def test_engine_is_registered_in_the_differential_harness(self):
        harness = (REPO_ROOT / "scripts" / "fuzz_diff.py").read_text()
        self.assertIn('"addr_value": {"script": "addr_value_fuzz.py"', harness)
        self.assertIn('"addr_value": functools.partial(run_standalone_engine',
                      harness)
        # No m32 compile-only fallback entry: that path compiles with the
        # -m32 flag matrix, which this hosted engine must never take.
        spec = harness.split('"addr_value": {"script"')[1].split("}")[0]
        self.assertNotIn('"gen"', spec)

    def test_engine_is_not_gated_by_the_m32_host_probe(self):
        # The reference compiler is the oracle here; the ELF32/int80 probe
        # exists for the -m32 engines and must not SKIP or degrade this one
        # on hosts without 32-bit support.
        harness = (REPO_ROOT / "scripts" / "fuzz_diff.py").read_text()
        spec = harness.split('"addr_value": {"script"')[1].split("}")[0]
        self.assertIn('"oracle": "reference"', spec)
        runner = harness.split("def run_standalone_engine")[1].split(
            "\n# Per-engine historical defaults")[0]
        self.assertIn('spec.get("oracle") == "reference"', runner)

    def test_engine_keeps_a_bounded_deterministic_default(self):
        harness = (REPO_ROOT / "scripts" / "fuzz_diff.py").read_text()
        block = harness.split("_LEGACY_ENGINE_DEFAULTS: dict[str, dict[str, Any]] = {")[1]
        block = block.split("\n}")[0]  # the whole table, not the first dict
        entry = re.search(r'"addr_value": \{([^}]*)\}', block)
        self.assertIsNotNone(entry, "no historical-defaults entry for addr_value")
        self.assertIn('"seed": 0', entry.group(1))
        self.assertIn("-O0,-O1", entry.group(1))
        self.assertIn('"count": 60', entry.group(1))

    def test_ci_runs_the_engine_and_the_selftest(self):
        ci = (REPO_ROOT / ".github" / "workflows" / "ci.yml").read_text()
        self.assertIn("--engine addr_value", ci)
        self.assertIn("unittest discover -s tests/fuzz -p 'test_*.py'", ci)


if __name__ == "__main__":
    unittest.main()
