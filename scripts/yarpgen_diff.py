#!/usr/bin/env python3
"""Compatibility wrapper forwarding to scripts/fuzz_diff.py --engine yarpgen."""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))
import fuzz_diff

if __name__ == "__main__":
    sys.argv = [sys.argv[0], "--engine", "yarpgen", *sys.argv[1:]]
    sys.exit(fuzz_diff.main())
