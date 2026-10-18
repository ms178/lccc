"""Pins the GCC reference link policy of run_regression.py.

GCC's -m32 reference executables must link -no-pie on this host: GNU ld 2.44's
default-PIE i386 output jumps to 0x1 before main.  The x86-64 reference links
must not change, and library inputs must keep following the source file.
"""
import importlib.util
import pathlib
import sys
import unittest

_HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("run_regression", _HERE / "run_regression.py")
rr = importlib.util.module_from_spec(_spec)
sys.modules["run_regression"] = rr
_spec.loader.exec_module(rr)


class GccReferenceLinkMode(unittest.TestCase):
    def test_m32_reference_is_non_pie(self):
        self.assertIn("-no-pie", rr.split_flags_for_gcc("-m32 -O2", "t.c"))

    def test_x86_64_reference_is_unchanged(self):
        self.assertEqual(rr.split_flags_for_gcc("-O2", "t.c"), ["-O2", "t.c"])

    def test_no_duplicate_no_pie(self):
        self.assertEqual(rr.split_flags_for_gcc("-m32 -no-pie", "t.c").count("-no-pie"), 1)

    def test_libraries_follow_the_source(self):
        out = rr.split_flags_for_gcc("-m32 -lm -O1", "t.c")
        self.assertEqual(out[out.index("t.c") + 1:], ["-lm"])


if __name__ == "__main__":
    unittest.main()
