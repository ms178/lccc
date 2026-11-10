#!/usr/bin/env python3
"""The verification environment must reuse an active exactly-4-GiB swap file.

A comment-only promise previously exported a second fixed path unconditionally.
On a 21-GiB sandbox that consumed another 4 GiB and could exhaust the disk.
This gate executes the real environment script with a sandbox-local /proc/swaps
fixture and a fake `stat`, then checks both the active-file and fallback paths.
It also pins the private mktemp diagnostic used by ensure_swap.sh: a fixed
/root-owned /tmp file made the calling user's redirection fail before swapon.
"""
from __future__ import annotations
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
GIB = 1024 ** 3


def run_case(directory: Path, active: Path | None, size: int) -> str:
    prefix = directory / "prefix"
    prefix.mkdir(exist_ok=True)
    (prefix / "env.sh").write_text("# test fixture\n")
    swaps = directory / "swaps"
    swaps.write_text("Filename Type Size Used Priority\n" +
                     (f"{active} file 4194300 0 -2\n" if active else ""))
    bindir = directory / "bin"
    bindir.mkdir(exist_ok=True)
    stat = bindir / "stat"
    stat.write_text(f"#!/bin/sh\nprintf '{size}\\n'\n")
    stat.chmod(0o755)
    env = os.environ.copy()
    env.update(LCCC_AGENT_CACHE_PREFIX=str(prefix), PATH=str(bindir) + ":" + env["PATH"],
               HOME=str(directory), LCCC_SWAPFILE="")
    # Substitute only /proc/swaps in a copied script; all selection logic is
    # otherwise the production text. This avoids requiring root in CI.
    source = (ROOT / "engineering/verification/agent-env.sh").read_text()
    source = source.replace("/proc/swaps", str(swaps))
    script = directory / "agent-env-copy.sh"
    script.write_text(source)
    result = subprocess.run(["bash", "-c", f'source "$1"; printf %s "$LCCC_SWAPFILE"',
                             "swap-test", str(script)], env=env, capture_output=True, text=True)
    if result.returncode:
        raise AssertionError(result.stderr)
    return result.stdout


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="lccc-swap-reuse-") as td:
        d = Path(td)
        active = d / "active.swap"
        active.write_bytes(b"")
        assert run_case(d, active, 4 * GIB) == str(active)
        assert run_case(d, active, 8 * GIB) == str(d / ".cache/lccc-swap.img")
        assert run_case(d, None, 4 * GIB) == str(d / ".cache/lccc-swap.img")
    swap = (ROOT / "scripts/ensure_swap.sh").read_text()
    assert 'mktemp "${TMPDIR:-/tmp}/lccc-swapon.XXXXXX"' in swap
    assert '2>"$swapon_err"' in swap
    assert "2>/tmp/lccc-swapon.err" not in swap
    print("agent swap reuse: PASS (active 4 GiB reused; wrong-size/no-swap fallback; private diagnostic)")


if __name__ == "__main__":
    main()
