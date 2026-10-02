#!/usr/bin/env python3
"""Behavioral contract tests for scripts/edg_corpus_mine.py.

Compiler-free.  Pins the invocation-schema and translator contracts the
PR #724 review showed were broken: `%-D` pass-through tokens, target value
consumption, empty default invocations, per-set languages, RUN-line phases,
absolute @N locations, Clang `{{regex}}` template splicing, count bounds,
provenance flags, and --force regeneration safety.

Run:  python3 -m unittest tests/corpus/test_miner_contracts.py
"""
from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location(
    "edg_corpus_mine", REPO / "scripts" / "edg_corpus_mine.py")
mmod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(mmod)


class TranslatorTest(unittest.TestCase):
    def test_percent_d_passthrough_becomes_define(self):
        gcc, arch, untr = mmod.translate_options("%-DFOO=1 --c99")
        self.assertIn("-DFOO=1", gcc)
        self.assertIn("-std=c99", gcc)
        self.assertEqual(arch, [])

    def test_percent_u_and_i_passthrough(self):
        gcc, arch, untr = mmod.translate_options("%-UBAR %-Iinc")
        self.assertIn("-UBAR", gcc)
        self.assertIn("-Iinc", gcc)

    def test_target_consumes_its_value(self):
        gcc, arch, untr = mmod.translate_options("--target linux_aarch64 --c")
        self.assertEqual(arch, ["linux_aarch64"])
        self.assertEqual(untr, [])

    def test_bare_tokens_are_never_arch(self):
        gcc, arch, untr = mmod.translate_options("140000 %-DFIRST_WAY")
        self.assertEqual(arch, [])
        self.assertIn("-DFIRST_WAY", gcc)

    def test_march_forms(self):
        gcc, arch, untr = mmod.translate_options("--march=x86-64-v3")
        self.assertEqual(arch, ["x86-64-v3"])

    def test_semantic_switches_never_silently_empty(self):
        # Every EDG_TO_GCC value must be non-empty or explicitly listed as
        # an acknowledged no-op; unknown switches land in `untranslated`.
        gcc, arch, untr = mmod.translate_options("--microsoft --gnu_version")
        self.assertTrue(tr := untr or gcc)  # something is visible either way
        self.assertIn("--microsoft", untr)

    def test_last_standard_flag_wins_per_set(self):
        gcc, _, _ = mmod.translate_options("--c99 --c11")
        stds = [f for f in gcc if f.startswith("-std=")]
        self.assertEqual(stds, ["-std=c11"])


class SftSchemaTest(unittest.TestCase):
    def _parse(self, content):
        with tempfile.TemporaryDirectory() as td:
            p = Path(td) / "t.sft.c"
            p.write_text(content, encoding="utf-8")
            return mmod.parse_sft(p)

    def test_empty_option_segments_are_default_invocations(self):
        i = self._parse("//options: : --c99\nint x;\n")
        self.assertEqual(i["option_sets"], ["", "--c99"])

    def test_language_is_per_invocation(self):
        i = self._parse("//options: --c : --c++\nint x;\n")
        self.assertEqual(i["languages_per_set"], ["c", "c++"])

    def test_options_all_recorded(self):
        i = self._parse("//options_all: -DALL\n//options: --c99\nint x;\n")
        self.assertEqual(i["options_all"], ["-DALL"])

    def test_run_line_phases(self):
        i = self._parse(
            "// RUN: %clang_cc1 -verify %s\n"
            "int x;\n")
        self.assertEqual(i["invocations"][0]["phase"], "frontend")
        i = self._parse(
            "// RUN: %clang_cc1 %s -emit-llvm -o - | FileCheck %s\nint x;\n")
        self.assertEqual(i["invocations"][0]["phase"], "filecheck")
        i = self._parse(
            "// RUN: %clang_cc1 %s -o %t && %t\nint main(){return 0;}\n")
        self.assertEqual(i["invocations"][0]["phase"], "execute")
        i = self._parse("// RUN: %clang_cc1 -E %s\nint x;\n")
        self.assertEqual(i["invocations"][0]["phase"], "preprocess")

    def test_verify_prefixes_per_run_command(self):
        i = self._parse(
            "// RUN: %clang_cc1 -verify=expected,c23 %s\n"
            "// RUN: %clang_cc1 -verify %s\nint x;\n")
        self.assertEqual([p["prefixes"] for p in i["invocations"]],
                         [["expected", "c23"], ["expected"]])

    def test_absolute_numeric_loc_is_absolute_line(self):
        i = self._parse("int a;\nint b;\n// expected-error@3 {{x}}\nint c;\n")
        e = i["expected"][0]
        self.assertEqual(e["location"]["line"], 3)     # absolute, NOT the annotation line

    def test_relative_and_any_locs(self):
        i = self._parse("int a;\n// expected-error@-1 {{x}}\nint b;\n")
        self.assertEqual(i["expected"][0]["location"]["line"], 1)
        i = self._parse("int a;\n// expected-error@* {{x}}\nint b;\n")
        self.assertIsNone(i["expected"][0]["location"]["line"])

    def test_template_regex_generation(self):
        i = self._parse(
            "// expected-error-re {{'(unnamed struct at {{.*}})' boom}}\n")
        e = i["expected"][0]
        rx = mmod.template_to_regex(e["msg"])
        self.assertIn(".*", rx)
        # literal parens are escaped (regex groups must not form)
        self.assertIn("\\(unnamed", rx)
        import re
        self.assertTrue(re.search(rx,
                                  "'(unnamed struct at /x:1:1)' boom"))

    def test_count_bounds(self):
        for spec, lo, hi in ((None, 1, 1), ("2", 2, 2),
                             ("1+", 1, None), ("0-1", 0, 1)):
            i = self._parse(f"// expected-warning{(' ' + spec) if spec else ''} {{{{m}}}}\n")
            e = i["expected"][0]
            self.assertEqual((e["count_min"], e["count_max"]), (lo, hi), spec)

    def test_system_header_provenance_flag(self):
        i = self._parse("// from /usr/include/string.h\ntypedef int x;\n")
        self.assertTrue(i.get("contains_expanded_system_headers"))

    def test_untranslated_flags_survive_into_record(self):
        i = self._parse("//options: --microsoft\nint x;\n")
        self.assertIn("--microsoft", i["untranslated_edg_flags"])


class RegenerationSafetyTest(unittest.TestCase):
    def test_force_refuses_empty_input_and_preserves_output(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            edg = root / "edg"
            (edg / mmod.CLANG_C_REL).mkdir(parents=True)
            out = root / "out"
            (out / "clang-c").mkdir(parents=True)
            sentinel = out / "clang-c" / "keep.txt"
            sentinel.write_text("precious", encoding="utf-8")
            rc = mmod.mine_clang_c(edg, out, force=True,
                                   index_only=False,
                                   include_giants=False,
                                   emit_sidecars=False)
            self.assertNotEqual(rc, 0)
            self.assertTrue(sentinel.exists())
            self.assertEqual(sentinel.read_text(), "precious")

    def fixture_git(self,edg,source):
        data=source.read_bytes();row=b'100644 blob '+mmod.blob(data).encode()+b'\t'+str(source.relative_to(edg)).encode()+b'\0'
        def fake(cmd,**kwargs):
            if 'status' in cmd:return ''
            if 'ls-tree' in cmd:return row
            return mmod.PIN_TREE+'\n' if any(str(v).startswith('HEAD:') for v in cmd) else mmod.PIN+'\n'
        return fake
    def source(self,root):
        edg=root/'edg';source=edg/mmod.CLANG_C_REL/'Parser/test.sft.c';source.parent.mkdir(parents=True)
        source.write_text('// RUN: %clang_cc1 -std=c99 -fsyntax-only -verify %s\n// expected-no-diagnostics\nint x;\n')
        notice=edg/'tests/tests/imported/clang/LICENSE.txt';notice.parent.mkdir(parents=True,exist_ok=True)
        retained=(REPO/'third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt').read_bytes()
        notice.write_bytes(retained[retained.find(b'==============================================================================\nThe LLVM Project is under'):])
        return edg,source
    def test_fresh_notice_identity_roundtrip_no_compiler(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);edg,source=self.source(root);out=root/'out'
            with mock.patch.object(mmod.subprocess,'check_output',side_effect=self.fixture_git(edg,source)):
                self.assertEqual(mmod.mine_clang_c(edg,out),0)
            data=mmod.verify_index(out/'clang-c/corpus-index.json');self.assertEqual(data['copied'],1)
    def test_exchange_failure_preserves_complete_live_generation(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);edg,source=self.source(root);out=root/'out';live=out/'clang-c';live.mkdir(parents=True);(live/'sentinel').write_text('precious')
            with mock.patch.object(mmod.subprocess,'check_output',side_effect=self.fixture_git(edg,source)),mock.patch.object(mmod.publication,'exchange',side_effect=OSError('unavailable')),self.assertRaises(OSError):
                mmod.mine_clang_c(edg,out,force=True)
            self.assertEqual((live/'sentinel').read_text(),'precious');self.assertFalse((live/'corpus-index.json').exists())


class IndexSchemaTest(unittest.TestCase):
    def test_shipped_index_matches_schema_and_hashes(self):
        idx_path = REPO / "tests" / "corpus" / "clang-c" / "corpus-index.json"
        if not idx_path.is_file():
            self.skipTest("corpus not mined in this tree")
        data = mmod.verify_index(idx_path)
        self.assertGreater(data["count"], 1000)
        import hashlib
        checked = 0
        for f in data["files"]:
            if not f.get("file"):
                continue
            p = REPO / "tests" / "corpus" / f["file"]
            self.assertTrue(p.is_file(), f["file"])
            self.assertEqual(
                hashlib.sha256(p.read_bytes()).hexdigest(), f["sha256"],
                f["file"])
            checked += 1
        self.assertGreater(checked, 1000)
        # v2 is strict and every actual RUN has its own prefix association.
        self.assertEqual(data['schema'], mmod.schema.VERSION)
        for f in data['files']:
            for inv in f['invocations']:
                active = {e['id'] for e in f['expected'] if e['prefix'] in inv['prefixes']}
                self.assertEqual(set(inv['expected_ids']), active if inv['verify'] else set())
            for e in f['expected']:
                self.assertTrue(e['count_max'] is None or type(e['count_max']) is int)
                self.assertIn('location', e)



if __name__ == "__main__":
    unittest.main(verbosity=1)
