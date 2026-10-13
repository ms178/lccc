#!/usr/bin/env python3
"""Strict Clippy wrapper plus the crate-policy audit — CI gate `clippy`.

`src/lib.rs` deliberately keeps a *load-bearing* list of the Clippy categories
this tree violates. Every name in that list must resolve, or `-D warnings` turns
the resulting `unknown_lints` warning into a hard error and the crate cannot be
built with the toolchain that does not know the name yet. A lint list pinned to
the newest Clippy is therefore incompatible with a declared `rust-version`
(`Cargo.toml`) older than that Clippy — which is exactly the situation the
crate is in: `clippy::chunks_exact_to_as_chunks` did not exist before Clippy
1.98, and the declared MSRV is 1.97.0.

The crate root therefore carries `#![allow(unknown_lints)]`, scoped to this
crate and documented at the site. That allow has one cost: a *typo* in the
policy list would also be silenced — a crate-level `allow` cannot be overridden
by a command-line `-D unknown_lints`, and the warning itself is what a command
line would have to match on. This wrapper buys the property back with a
toolchain-accurate probe instead of an output scrape:

  * it runs Clippy exactly as CI does (`--all-targets --profile fastbuild
    --locked -j N -- -D warnings`) and propagates its status;
  * it re-declares the crate-root policy list in a generated probe compiled by
    `clippy-driver` under `#![deny(unknown_lints)]`. The compiler — not a
    regex over somebody's stderr wording — says which names this toolchain does
    not know; and
  * every such name must be a dated entry in `scripts/clippy_msrv_lints.txt`.

An unlisted unknown lint — a typo (`clippy::manual_contans`), a lint removed
upstream, or a name this toolchain spells differently — fails the gate. A
listed name is a deliberate compatibility entry, and on a toolchain that knows
it the probe reports nothing, so hosted CI stays green.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ALLOW_FILE = REPO / "scripts" / "clippy_msrv_lints.txt"
LIB_RS = REPO / "src" / "lib.rs"
UNKNOWN_LINT = re.compile(r"unknown lint: `clippy::([A-Za-z0-9_]+)`")

CLIPPY_ARGS = [
    "clippy",
    "--all-targets",
    "--profile",
    "fastbuild",
    "--locked",
    "-j",
    os.environ.get("CARGO_BUILD_JOBS", "2"),
    "--",
    "-D",
    "warnings",
]


def load_allowed(path: Path) -> dict[str, str]:
    """`name -> reason` for every compatibility entry (comments stripped)."""
    allowed: dict[str, str] = {}
    if not path.is_file():
        return allowed
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        fields = line.split(None, 1)
        allowed[fields[0]] = fields[1] if len(fields) > 1 else ""
    return allowed


def crate_policy_lints() -> list[str]:
    """Every `clippy::` name in the crate-root `#![allow(...)]` policy block."""
    text = LIB_RS.read_text(encoding="utf-8")
    start = text.find("#![allow(\n    clippy::")
    if start < 0:
        raise SystemExit("check_clippy_policy: crate-root clippy policy block not found")
    end = text.find(")]", start)
    if end < 0:
        raise SystemExit("check_clippy_policy: unterminated crate-root clippy policy block")
    block = text[start:end]
    names = re.findall(r"clippy::([A-Za-z0-9_]+)", block)
    # The block is the crate's lint policy; duplicates would be a merge artifact.
    return sorted(set(names))


def unknown_on_this_toolchain(names: list[str]) -> list[str] | None:
    """Names `clippy-driver` rejects under `#![deny(unknown_lints)]`.

    Returns None when no `clippy-driver` is available (the probe then cannot
    run; `cargo clippy` strict is still enforced by the caller).
    """
    driver = shutil.which("clippy-driver")
    if driver is None:
        return None
    probe = REPO / "target" / "clippy-policy-probe.rs"
    probe.parent.mkdir(parents=True, exist_ok=True)
    allows = ", ".join(f"clippy::{n}" for n in names)
    probe.write_text(
        f"#![deny(unknown_lints)]\n#![allow({allows})]\npub fn probe() {{}}\n",
        encoding="utf-8",
    )
    proc = subprocess.run(
        [
            driver,
            "--edition",
            "2021",
            "--crate-type",
            "lib",
            "--emit=metadata",
            "-o",
            str(probe.with_suffix(".rmeta")),
            str(probe),
        ],
        cwd=REPO,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    if proc.returncode == 0:
        return []
    unknown = sorted(set(UNKNOWN_LINT.findall(proc.stdout)))
    if not unknown:
        print(proc.stdout, file=sys.stderr)
        raise SystemExit(
            "check_clippy_policy: probe failed without an unknown-lint diagnostic"
        )
    return unknown


def main(argv: list[str]) -> int:
    if shutil.which("cargo") is None:
        print("check_clippy_policy: cargo is not on PATH", file=sys.stderr)
        return 127

    proc = subprocess.run(
        ["cargo", *CLIPPY_ARGS, *argv],
        cwd=REPO,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    output = proc.stdout
    sys.stdout.write(output)

    allowed = load_allowed(ALLOW_FILE)
    if proc.returncode != 0:
        print(
            f"check_clippy_policy: FAIL: cargo clippy exited {proc.returncode}",
            file=sys.stderr,
        )
        return proc.returncode

    names = crate_policy_lints()
    unknown = unknown_on_this_toolchain(names)
    if unknown is None:
        print(
            "check_clippy_policy: clippy-driver not found; lint-name probe skipped "
            "(cargo clippy strict still enforced)"
        )
        unknown = []
    for name in unknown:
        state = "documented compatibility entry" if name in allowed else "UNLISTED"
        print(f"check_clippy_policy: {name}: unknown on this toolchain ({state})")
    unexpected = [name for name in unknown if name not in allowed]
    if unexpected:
        print(
            "check_clippy_policy: FAIL: unknown Clippy lint(s) not listed in "
            f"{ALLOW_FILE.relative_to(REPO)}: "
            + ", ".join(unexpected)
            + "\n  A typo in the crate-root policy list would be silenced by "
            "`#![allow(unknown_lints)]`;\n  list a genuine newer-than-MSRV "
            "lint there (name + reason) instead of fixing the spelling.",
            file=sys.stderr,
        )
        return 1

    print(
        "check_clippy_policy: PASS (clippy strict clean; "
        f"{len(names)} policy lint name(s) probed, "
        f"{len(unknown)} compatibility name(s) unknown on this toolchain)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
