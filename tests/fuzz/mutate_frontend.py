#!/usr/bin/env python3
"""Mutation fuzzer for the lccc front end and middle end.

The differential fuzzers in this directory generate *valid* programs.  This one
does the opposite: it takes the repository's own C files, damages them (token
insertion, truncation, byte flips, duplication, NUL / 0xFF bytes, repeated
tokens) and checks that the compiler survives.  It looks for exactly three
failure classes and treats everything else (success or an ordinary diagnostic)
as fine:

* ``ice``      -- an internal error or a Rust panic message on stderr;
* ``signal``   -- the compiler was killed by a signal (abort, stack overflow);
* ``timeout``  -- no result within ``--timeout`` seconds (a hang).

Each failure bucket retains up to three distinct inputs in ``--out``.
Exit status: 0 for a clean run, 1 for findings, 2 for a harness error.

Examples::

    python3 tests/fuzz/mutate_frontend.py --lccc target/fastbuild/lccc \
        --seeds 'tests/**/*.c' --iterations 900 --out /tmp/lccc-mutfuzz -- -O1

    # Re-check a saved reproducer (or any file) without mutating it:
    python3 tests/fuzz/mutate_frontend.py --lccc target/fastbuild/lccc \
        --replay /tmp/lccc-mutfuzz/timeout-<sha256>.c -- -O0

Everything after ``--`` is passed to the compiler.  The default is ``-O1``.
The run is deterministic for a given ``--seed``; change it to explore new
mutants (runs with the same seed share their mutant sequence, whatever the
compiler flags are).
"""
from __future__ import annotations

import argparse
import glob
import hashlib
import json
import math
import os
import random
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

MAX_SEED_BYTES = 6000
MAX_SAMPLES = 3
ICE_HEADER = re.compile(
    r"^(?:ccc: internal error: (?P<ice>.*)|"
    r"thread '[^\n]*' panicked at (?P<panic>.*))$", re.MULTILINE)

TOKENS = [
    b"{", b"}", b"(", b")", b"[", b"]", b";", b",", b"*", b"&", b"#define ",
    b"#include ", b"#if ", b"#endif", b"__attribute__((", b"asm(",
    b"__asm__ volatile(", b"typeof(", b"_Generic(", b"sizeof", b"struct ",
    b"union ", b"enum ", b"...", b"\\\n", b"\x00", b"\xff\xfe", b"'", b'"',
    b"0x", b"1e", b"?:", b"::", b"->", b"<<=", b"__int128", b"_Complex",
    b"_Atomic", b"__vector", b"[0]", b"[-1]", b"[999999999999]",
    b"static_assert(", b"constexpr ", b"auto ", b"_BitInt(", b"asm goto(",
]


def mutate(data: bytes, rng: random.Random) -> bytes:
    """Apply 1-5 random edits to ``data``."""
    buf = bytearray(data)
    for _ in range(rng.choice([1, 1, 2, 3, 5])):
        if not buf:
            break
        kind = rng.random()
        i = rng.randrange(len(buf))
        if kind < 0.25:
            del buf[i:i + rng.randint(1, 12)]
        elif kind < 0.50:
            buf[i:i] = rng.choice(TOKENS)
        elif kind < 0.65:
            buf[i] = rng.randrange(256)
        elif kind < 0.80:
            j = rng.randrange(len(buf))
            buf[i:i] = buf[j:j + rng.randint(1, 40)]
        elif kind < 0.90:
            del buf[i:]
        else:
            buf[i:i] = rng.choice(TOKENS) * rng.randint(2, 200)
    return bytes(buf)


def run_one(lccc: str, path: Path, flags: list[str], timeout: float):
    """Compile ``path`` to assembly.

    Returns ``(kind, signature)`` where ``kind`` is one of ``ok`` (rc 0),
    ``diagnostic`` (ordinary error exit) or a failure class (``ice``,
    ``signal``, ``timeout``) whose signature groups identical failures.
    """
    try:
        proc = subprocess.run(
            [lccc, "-S", "-o", os.devnull, str(path), *flags],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=timeout)
    except subprocess.TimeoutExpired:
        return "timeout", "TIMEOUT"
    if proc.returncode < 0:
        return "signal", f"SIG{-proc.returncode}"
    match = ICE_HEADER.search(proc.stderr.decode("utf-8", errors="replace"))
    if match:
        return "ice", (match.group("ice") or match.group("panic") or "unknown")[:256]
    return ("ok" if proc.returncode == 0 else "diagnostic"), ""


def publish(out: Path, kind: str, src: bytes, digest: str) -> Path:
    """Publish complete, content-addressed inputs even with concurrent runs."""
    dest = out / f"{kind}-{digest}.c"
    with tempfile.TemporaryDirectory(dir=out) as staging:
        pending = Path(staging) / "input.c"
        pending.write_bytes(src)
        pending.replace(dest)
    return dest


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    flags: list[str] = []
    if "--" in argv:
        cut = argv.index("--")
        argv, flags = argv[:cut], argv[cut + 1:]
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--lccc", required=True, help="path to the lccc binary")
    ap.add_argument("--seeds", default="tests/**/*.c",
                    help="glob of seed C files (default: %(default)s)")
    ap.add_argument("--iterations", type=int, default=500)
    ap.add_argument("--seed", type=int, default=20261120,
                    help="RNG seed; the mutant sequence is a function of it")
    ap.add_argument("--timeout", type=float, default=30.0,
                    help="seconds before a compile counts as a hang")
    ap.add_argument("--out", default="/tmp/lccc-mutfuzz",
                    help="directory for reproducers")
    ap.add_argument("--replay", nargs="+", metavar="FILE",
                    help="compile these files as they are, without mutating")
    ap.add_argument("--json", action="store_true", help="machine-readable summary")
    args = ap.parse_args(argv)
    if args.iterations <= 0:
        ap.error("--iterations must be positive")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        ap.error("--timeout must be finite and positive")
    flags = flags or ["-O1"]
    try:
        return run(args, flags)
    except (OSError, OverflowError, ValueError) as exc:
        print(f"mutation harness: {exc}", file=sys.stderr)
        return 2


def run(args: argparse.Namespace, flags: list[str]) -> int:
    out = Path(args.out).resolve()
    seeds = [] if args.replay else sorted(
        p for p in map(Path, glob.iglob(args.seeds, recursive=True))
        if p.is_file() and 0 < p.stat().st_size < MAX_SEED_BYTES
    )
    if not args.replay and not seeds:
        raise ValueError(f"no usable seed files match {args.seeds!r}")
    buckets: dict[tuple[str, str], dict] = {}
    stats = dict.fromkeys(("total", "ok", "diagnostic", "ice", "signal", "timeout"), 0)
    started = time.monotonic()

    def record(src: bytes, origin: Path, path: Path) -> None:
        kind, signature = run_one(args.lccc, path, flags, args.timeout)
        stats["total"] += 1
        stats[kind] += 1
        if kind in ("ok", "diagnostic"):
            return
        key = (kind, signature)
        if key not in buckets:
            buckets[key] = {"kind": kind, "signature": signature, "count": 0,
                            "coarse": kind != "ice", "samples": []}
        bucket = buckets[key]
        bucket["count"] += 1
        samples = bucket["samples"]
        if len(samples) < MAX_SAMPLES:
            digest = hashlib.sha256(src).hexdigest()
            if not any(sample["sha256"] == digest for sample in samples):
                saved = publish(out, kind, src, digest)
                samples.append({"path": str(saved), "origin": str(origin), "sha256": digest,
                                "replay_argv": [sys.executable, str(Path(__file__).resolve()),
                                                "--lccc", args.lccc, "--timeout", str(args.timeout),
                                                "--out", str(out), "--replay", str(saved), "--", *flags]})

    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="run-", dir=out) as work:
        scratch = Path(work) / "current.c"
        scratch.write_bytes(b"int lccc_mutation_probe(void) { return 0; }\n")
        kind, signature = run_one(args.lccc, scratch, flags, args.timeout)
        if kind != "ok":
            raise ValueError(f"known-good input rejected ({kind}{': ' + signature if signature else ''}); "
                             "check the compiler, flags and timeout")
        if args.replay:
            for name in args.replay:
                path = Path(name).absolute()
                record(path.read_bytes(), path, path)
        else:
            rng = random.Random(args.seed)
            for _ in range(args.iterations):
                origin = rng.choice(seeds)
                src = mutate(origin.read_bytes(), rng)
                scratch.write_bytes(src)
                record(src, origin, scratch)

    report = {"stats": stats, "flags": flags, "seed": args.seed, "timeout": args.timeout,
              "cwd": str(Path.cwd()), "buckets": list(buckets.values())}
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(f"{json.dumps(stats)} in {time.monotonic() - started:.0f}s, flags {' '.join(flags)}")
        for bucket in sorted(buckets.values(), key=lambda item: -item["count"]):
            print(f"{bucket['count']:5d}  {bucket['kind']}: {bucket['signature']}"
                  f"   e.g. {bucket['samples'][0]['path']}")
    return int(bool(buckets))


if __name__ == "__main__":
    sys.exit(main())
