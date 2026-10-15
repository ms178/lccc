"""Behavioral failure context: stale prior-iteration controls must not leak."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class MemoryScriptContext(unittest.TestCase):
    def test_next_reference_failure_reports_its_own_configuration(self):
        with tempfile.TemporaryDirectory() as td:
            compiler = Path(td) / "compiler"
            compiler.write_text('''#!/usr/bin/env python3
from pathlib import Path
import sys
args = sys.argv[1:]
# This fixture only tests diagnostic context, never compiler correctness.
if "-dumpfullversion" in args:
    print("16.2.0")
    raise SystemExit(0)
if "-print-file-name=include" in args:
    print("/unused/include")
    raise SystemExit(0)
if any(x.endswith("adjacent_membership.c") for x in args):
    raise SystemExit(37)
p = Path(args[args.index("-o") + 1])
p.write_text("#!/bin/sh\\nprintf 'checked\\\\n'\\n")
p.chmod(0o755)
''')
            compiler.chmod(0o755)
            result = subprocess.run(
                ["bash", str(ROOT / "tests/regression/check_memory_value_contracts.sh")],
                env={**os.environ, "CCC": str(compiler), "CC_REF": str(compiler), "GCC": str(compiler)},
                capture_output=True, text=True, timeout=30,
            )
            self.assertEqual(result.returncode, 37, result.stderr)
            self.assertIn("test=adjacent_membership opt=-O2 isa=x86-64 disabled=none phase=reference", result.stderr)


if __name__ == "__main__":
    unittest.main()
