#!/usr/bin/env python3
"""Compatibility wrapper forwarding to scripts/x86_gcc_torture.py --arch=i686.

All runner flags (including --from-list for failure-focused partial re-runs)
are forwarded unchanged; see x86_gcc_torture.py --help for the full surface.
"""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))
import x86_gcc_torture

if __name__ == "__main__":
    sys.argv = [sys.argv[0], "--arch", "i686", *sys.argv[1:]]
    sys.exit(x86_gcc_torture.main())
