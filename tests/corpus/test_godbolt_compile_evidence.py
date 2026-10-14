"""A successful process is not evidence that its requested target was honored."""
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import godbolt
import godbolt_cache

SPEC = importlib.util.spec_from_file_location(
    "oracle_compile_contract", ROOT / "tools/oracle/godbolt_oracle.py")
oracle = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(oracle)

ASM = {"code": 0, "asm": [{"text": "    ret"}]}
IGNORED = "icc: command line warning #10148: option '-march=x86-64-v3' not supported"


class CompileEvidence(unittest.TestCase):
    def remote(self, response, execute=False):
        replies = [ASM, response] if execute else [response]
        with patch.object(oracle, "USE_CACHE", False), \
                patch.object(oracle, "_ce_call", side_effect=replies):
            return oracle.remote("cicc2021100", "int main(void){return 7;}", "-O2", execute, 5)

    def test_partial_assembly_does_not_excuse_failed_or_missing_exit(self):
        for code in (1, None, False, "0"):
            with self.subTest(code=code):
                self.assertFalse(self.remote({**ASM, "code": code})["ok"])

    def test_diagnostics_and_malformed_responses_never_raise(self):
        for response in (None, [], "bad", {"code": 1, "stderr": [{"text": "bad option"}]}):
            with self.subTest(response=response):
                self.assertFalse(self.remote(response)["ok"])

    def test_ignored_target_rejected_in_both_remote_compile_legs(self):
        for diagnostics in (IGNORED, [{"text": IGNORED}]):
            with self.subTest(diagnostics=diagnostics):
                build = {**ASM, "stderr": diagnostics}
                self.assertFalse(self.remote(build)["ok"])
                self.assertFalse(self.remote({"didExecute": True, "code": 0,
                                             "buildResult": build}, True)["ok"])

    def test_ordinary_warning_and_nonzero_program_exit_are_valid_evidence(self):
        build = {**ASM, "stderr": [{"text": "warning: unused variable 'x'"}]}
        self.assertTrue(self.remote(build)["ok"])
        result = self.remote({"didExecute": True, "code": 7, "buildResult": build}, True)
        self.assertTrue(result["ok"])
        self.assertEqual(result["exit"], 7)

    def test_incomplete_compile_and_malformed_build_fail_closed(self):
        for response in ({**ASM, "timedOut": True}, {**ASM, "truncated": True}):
            self.assertFalse(self.remote(response)["ok"])
        for build in ([], "bad", {"code": 0, "timedOut": True}):
            self.assertFalse(self.remote({"didExecute": True, "code": 0,
                                         "buildResult": build}, True)["ok"])

    def test_shared_static_helper_rejects_ignored_flags(self):
        with patch.object(godbolt, "resolve_compiler", return_value="cicc2021100"), \
                patch.object(godbolt, "_request_json", return_value={**ASM, "stderr": IGNORED}):
            self.assertIsNone(godbolt.compile_on_godbolt("icc", "int f(void){return 0;}", "-O2"))

    def test_structured_producer_keeps_the_actual_rejection(self):
        with patch.object(godbolt, "resolve_compiler", return_value="cicc2021100"), \
                patch.object(godbolt, "_request_json", return_value={**ASM, "stderr": IGNORED}):
            with self.assertRaisesRegex(godbolt.GodboltError, "10148.*not supported"):
                godbolt.compile_on_godbolt("icc", "int f(void){return 0;}", "-O2",
                                           raise_on_error=True)

    def test_structured_producer_keeps_transport_errors(self):
        with patch.object(godbolt, "resolve_compiler", return_value="g162"), \
                patch.object(godbolt, "_request_json", side_effect=godbolt.GodboltError("HTTP 503")):
            with self.assertRaisesRegex(godbolt.GodboltError, "HTTP 503"):
                godbolt.compile_on_godbolt("gcc16.2", "int f(void){return 0;}", "-O2",
                                           raise_on_error=True)

    def test_classic_icc_uses_an_explicit_common_cpu_dialect_only(self):
        flags = "-O2 -march=haswell -fwrapv -DN=128 -lm"
        actual = godbolt.compiler_flags("cicc2021100", flags)
        self.assertIn("-xCORE-AVX2", actual)
        self.assertIn("-fwrapv -DN=128 -lm", actual)
        self.assertNotIn("-march=", actual)
        for compiler in ("g162", "cclang2310", "icx-latest"):
            self.assertEqual(godbolt.compiler_flags(compiler, flags), flags)
        self.assertEqual(godbolt.compiler_flags("cicc2021100", "-O2 -march=x86-64-v3"),
                         "-O2 -march=x86-64-v3")
        self.assertEqual(godbolt.compiler_flags("cicc2021100", "-O2 -DN='-march=haswell'"),
                         "-O2 -DN='-march=haswell'")
        for extra in ("-march=skylake", "-xAVX", "-axCORE-AVX2"):
            with self.subTest(extra=extra), self.assertRaises(godbolt.GodboltError):
                godbolt.compiler_flags("cicc2021100", flags + " " + extra)

    def test_response_records_requested_and_effective_cpu_options(self):
        flags = "-O2 -march=haswell -fwrapv"
        with patch.object(godbolt, "resolve_compiler", return_value="cicc2021100"), \
                patch.object(godbolt, "_request_json", return_value={**ASM}) as request:
            result = godbolt.compile_on_godbolt("icc", "int f(void){return 0;}", flags)
            self.assertEqual(result["lcccOracleOptions"]["requested"], flags)
            self.assertEqual(request.call_args.kwargs["body"]["options"]["userArguments"],
                             "-O2 -xCORE-AVX2 -fwrapv")
            self.assertIn("-xCORE-AVX2", result["lcccOracleOptions"]["effective"])

    def test_old_unvalidated_cache_namespaces_are_not_reused(self):
        self.assertNotEqual(godbolt_cache.NS_ASM, "att-v3")
        self.assertNotEqual(godbolt_cache.NS_ORACLE, "oracle-v2")
        self.assertNotEqual(godbolt_cache.NS_ENCDIFF, "encdiff-v2")


if __name__ == "__main__":
    unittest.main()
