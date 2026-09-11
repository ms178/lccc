#!/usr/bin/env python3
"""Compatibility wrapper forwarding to scripts/fuzz_diff.py --engine csmith.

Historical flag spellings (--ccc, --clang, --gcc, --jobs, --tests,
--seed-start, --include, --out-dir, ...) are translated by
fuzz_diff.translate_legacy_args.  Like the pre-unification tester, the default
run is infinite (use --tests N / --count N for a bounded run).
"""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))
import fuzz_diff

if __name__ == "__main__":
    argv = fuzz_diff.translate_legacy_args(sys.argv[1:], default_count=0)
    sys.argv = [sys.argv[0], "--engine", "csmith", *argv]
    sys.exit(fuzz_diff.main())
