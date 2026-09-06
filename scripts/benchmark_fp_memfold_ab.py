#!/usr/bin/env python3
"""Compatibility wrapper forwarding to scripts/perf_ab.py --preset fp_memfold."""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))
import perf_ab

if __name__ == "__main__":
    sys.argv = [sys.argv[0], "--preset", "fp_memfold", *sys.argv[1:]]
    sys.exit(perf_ab.main())
