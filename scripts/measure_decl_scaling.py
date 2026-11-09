#!/usr/bin/env python3
"""Measure wide declaration scaling on a host, without claiming a root cause.

Runs one compiler at a time; alternate LCCC/GCC order between rounds. Records
wall time and LCCC phase timings, exact commands, binary and generated-input
hashes, object hashes and return codes. Every successful compile must produce a
fresh object with an x86-64 ELF relocatable header. A failed or timed-out run
is not a timing sample. This is a diagnostic
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


def annotation_payload(output: dict) -> str:
    """Keep the GitHub annotation below the check-run message limit.

    Full commands and per-object identities remain in the JSON artifact. If a
    custom campaign has too many samples for one annotation, say exactly how
    many were omitted rather than publishing a truncated JSON string.
    """
    samples = [[r["shape"], r["n"], r["round"], r["compiler"], r["wall_s"], r["sema_s"]]
               for r in output["results"]]
    def encode(count: int) -> str:
        summary = {"compiler_sha256": {k: v["sha256"] for k, v in output["compilers"].items()},
                   "flags": output["flags"], "samples": samples[:count], "total_samples": len(samples),
                   "omitted_samples": len(samples) - count, "full_report": "declaration-scaling.json"}
        return json.dumps(summary, separators=(",", ":"), ensure_ascii=True).replace("%", "%25")

    if len(encode(0).encode("utf-8")) > 3800:
        raise ValueError("measurement annotation metadata exceeds safe limit")
    low, high = 0, len(samples)
    while low < high:
        middle = (low + high + 1) // 2
        if len(encode(middle).encode("utf-8")) <= 3800:
            low = middle
        else:
            high = middle - 1
    return encode(low)


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
        # A failed retry must not leave an earlier success report looking current.
        # Never allow a report path to overwrite a compiler (including hardlinks).
        if args.json.exists() or args.json.is_symlink():
            if args.json.exists() and any(path.exists() and os.path.samefile(args.json, path)
                                          for path in (args.lccc, args.gcc)):
                raise ValueError("report path names a compiler binary")
            args.json.unlink()
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
                    input_sha256 = digest(inp)
                    for r in range(args.rounds):
                        for name in (("lccc", "gcc") if r % 2 == 0 else ("gcc", "lccc")):
                            obj = root / "out.o"
                            obj.unlink(missing_ok=True)
                            cmd = [str(compilers[name]), "-O0", "-w", "-c", str(inp), "-o", str(obj)]
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
                            if not obj.is_file():
                                raise RuntimeError(f"{kind}/{n}/{name} did not produce an object")
                            with obj.open("rb") as stream:
                                header = stream.read(20)
                            if (len(header) != 20 or header[:7] != b"\x7fELF\x02\x01\x01"
                                    or header[16:20] != b"\x01\x00\x3e\x00"):
                                raise RuntimeError(f"{kind}/{n}/{name} did not produce x86-64 ELF relocatable")
                            match = re.search(r"\[TIME\] sema: ([0-9.]+)s", proc.stderr)
                            if name == "lccc" and match is None:
                                raise RuntimeError(f"{kind}/{n}: LCCC phase timer missing")
                            results.append({"shape": kind, "n": n, "round": r, "compiler": name,
                                            "command": cmd, "returncode": proc.returncode,
                                            "input_sha256": input_sha256, "object_sha256": digest(obj),
                                            "wall_s": round(elapsed, 6),
                                            "sema_s": float(match.group(1)) if match else None})
        output = {"compilers": identities, "flags": ["-O0", "-w", "-c"], "results": results,
                  "scope": "one hosted machine; no contention control; timings are not a performance guarantee"}
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
        print(f"::notice title=Declaration scaling measurement::{annotation_payload(output)}")
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"declaration scaling failed: {exc}", file=__import__("sys").stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
