#!/usr/bin/env python3
"""Compatibility wrapper forwarding to scripts/perf_ab.py --preset vecreg_ops."""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))
import perf_ab

if __name__ == "__main__":
    sys.argv = [sys.argv[0], "--preset", "vecreg_ops", *sys.argv[1:]]
    sys.exit(perf_ab.main())
