"""Resolve the two LCCC A/B legs for the research tools in ``engineering/``.

Every differential tool must name its legs explicitly.  A hardcoded
``.../fastbuild-baselineN`` path silently measured against a stale (and
eventually deleted) compiler when the baseline was rebuilt under a new number;
both the four-way runtime oracle and the host-CPU benchmark returned a hard
"MISSING: lccc_old" instead of a result.  Fail-closed resolution removes that
class of reproducibility bug: a missing leg is an error, never a fallback to
whatever happens to be installed.

Legs may be given as a fastbuild directory (``.../fastbuild``) or as a path to
the ``lccc`` binary itself.  Defaults come from ``LCCC_AB_OLD`` /
``LCCC_AB_NEW`` or the current pinned layout.
"""
from __future__ import annotations

import argparse
import os
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_OLD = ROOT / "target" / "fastbuild-baseline3" / "fastbuild"
DEFAULT_NEW = ROOT / "target" / "fastbuild"


def add_leg_arguments(ap: argparse.ArgumentParser) -> None:
    """Add ``--old`` / ``--new`` to a research tool's parser."""
    ap.add_argument("--old", type=pathlib.Path,
                    default=pathlib.Path(os.environ.get("LCCC_AB_OLD", DEFAULT_OLD)),
                    help="pristine-main fastbuild directory or lccc binary")
    ap.add_argument("--new", type=pathlib.Path,
                    default=pathlib.Path(os.environ.get("LCCC_AB_NEW", DEFAULT_NEW)),
                    help="PR-tree fastbuild directory or lccc binary")


def resolve(old: pathlib.Path, new: pathlib.Path) -> tuple[str, str]:
    """Return the two compiler paths, failing closed if either is missing."""
    out: list[str] = []
    for label, p in (("old", old), ("new", new)):
        binary = p / "lccc" if p.is_dir() else p
        if not binary.is_file():
            raise SystemExit(
                f"error: {label} leg {p} is not a built LCCC (missing {binary}); "
                "rebuild with scripts/build_lccc_fast.sh or pass --old/--new"
            )
        out.append(str(binary))
    return out[0], out[1]
