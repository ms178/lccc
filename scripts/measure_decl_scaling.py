#!/usr/bin/env python3
"""Measure wide declaration scaling on a host, without claiming a root cause.

Runs one compiler at a time; alternate LCCC/GCC order between rounds. Records
wall time and LCCC phase timings, exact commands, binary hashes and return
codes. A failed or timed-out run is not a timing sample. This is a diagnostic
measurement, not a noisy wall-clock CI threshold or a proof of asymptotics.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time


def source(kind: str, n: int) -> str:
    if kind == "block":
        return "void f(void) {\n" + "".join(f"int v{i};\n" for i in range(n)) + "}\n"
    if kind == "external":
        return "".join(f"extern int v{i};\n" for i in range(n))
    raise ValueError(kind)


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--lccc", type=Path, required=True)
    p.add_argument("--gcc", type=Path, required=True)
    p.add_argument("--json", type=Path, required=True)
    p.add_argument("--sizes", default="10000,20000,40000")
    p.add_argument("--rounds", type=int, default=2)
    p.add_argument("--timeout", type=float, default=180)
    args = p.parse_args()
    try:
        sizes = [int(n) for n in args.sizes.split(",")]
        if not sizes or any(n < 1 or n > 100000 for n in sizes) or len(set(sizes)) != len(sizes):
            raise ValueError("sizes must be distinct integers in 1..100000")
        if not 1 <= args.rounds <= 10 or not 0 < args.timeout <= 600:
            raise ValueError("invalid rounds or timeout")
        compilers = {"lccc": args.lccc.resolve(strict=True), "gcc": args.gcc.resolve(strict=True)}
        identities = {name: {"path": str(path), "sha256": digest(path)} for name, path in compilers.items()}
        results = []
        with tempfile.TemporaryDirectory(prefix="decl-scaling-") as tmp:
            root = Path(tmp)
            for kind in ("block", "external"):
                for n in sizes:
                    inp = root / f"{kind}-{n}.c"
                    inp.write_text(source(kind, n), encoding="utf-8")
                    for r in range(args.rounds):
                        for name in (("lccc", "gcc") if r % 2 == 0 else ("gcc", "lccc")):
                            cmd = [str(compilers[name]), "-O0", "-w", "-c", str(inp), "-o", str(root / "out.o")]
                            env = {k: v for k, v in os.environ.items()
                                   if not k.startswith(("CCC_", "LCCC_"))}
                            if name == "lccc":
                                env["CCC_TIME_PHASES"] = "1"
                            start = time.monotonic()
                            proc = subprocess.run(cmd, capture_output=True, text=True, env=env,
                                                  timeout=args.timeout, check=False)
                            elapsed = time.monotonic() - start
                            if proc.returncode:
                                raise RuntimeError(f"{kind}/{n}/{name} exited {proc.returncode}: {proc.stderr[-2000:]}")
                            match = re.search(r"\[TIME\] sema: ([0-9.]+)s", proc.stderr)
                            if name == "lccc" and match is None:
                                raise RuntimeError(f"{kind}/{n}: LCCC phase timer missing")
                            results.append({"shape": kind, "n": n, "round": r, "compiler": name,
                                            "wall_s": round(elapsed, 6),
                                            "sema_s": float(match.group(1)) if match else None})
        output = {"compilers": identities, "flags": ["-O0", "-w", "-c"], "results": results,
                  "scope": "one hosted machine; no contention control; timings are not a performance guarantee"}
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
        payload = json.dumps(output, separators=(",", ":"), ensure_ascii=True).replace("%", "%25")
        print(f"::notice title=Declaration scaling measurement::{payload}")
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"declaration scaling failed: {exc}", file=__import__("sys").stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
