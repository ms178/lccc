"""Shared path defaults for the kernel measurement tools.

The shell side of the kernel harness resolves "which Linux tree" through
``scripts/kernel_version.sh`` (see its header for why a single resolver is
mandatory).  The Python tools need the same answer, and a second copy of the
resolution logic would drift from the first one the moment the archpkgbuilds
package bumps its stable point -- exactly the failure mode that made twelve
scripts point at a stale ``linux-6.18.55`` directory.

So the Python side asks the shell side.  ``kernel_version.sh`` is a pure
resolver (a few stat() calls plus one sed), so shelling out costs about one
millisecond and keeps one source of truth.

The returned path is only ever a *default*: every tool still honours an
explicit ``--kernel``/``--kernel-dir`` argument or ``KERNEL_DIR`` first.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent


def _resolver() -> Path:
    return HERE / "kernel_version.sh"


def kernel_version() -> str:
    """Return the kernel version the harness should agree on (e.g. ``6.18.58``).

    ``KVER`` wins when set.  Otherwise the shell resolver's answer is used.
    The literal fallback exists only for the pathological case of a tree that
    has neither the resolver script nor a PKGBUILD checkout; it must stay in
    step with ``LCCC_KERNEL_FALLBACK_KVER`` in ``kernel_version.sh``.
    """
    explicit = os.environ.get("KVER")
    if explicit:
        return explicit
    resolver = _resolver()
    if resolver.is_file():
        try:
            out = subprocess.run(
                ["bash", str(resolver), "--kver"],
                check=True,
                capture_output=True,
                text=True,
                timeout=30,
            ).stdout.strip()
        except (OSError, subprocess.SubprocessError):
            out = ""
        if out:
            return out
    return "6.18.58"


def kernel_default_dir() -> str:
    """Return the default kernel tree path, honouring ``KERNEL_DIR``."""
    explicit = os.environ.get("KERNEL_DIR")
    if explicit:
        return explicit
    resolver = _resolver()
    if resolver.is_file():
        try:
            out = subprocess.run(
                ["bash", str(resolver), "--default-dir"],
                check=True,
                capture_output=True,
                text=True,
                timeout=30,
            ).stdout.strip()
        except (OSError, subprocess.SubprocessError):
            out = ""
        if out:
            return out
    workdir = os.environ.get(
        "KERNEL_WORK",
        str(Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "lccc" / "kernel-work"),
    )
    return str(Path(workdir) / f"linux-{kernel_version()}")


if __name__ == "__main__":
    print(kernel_default_dir())
