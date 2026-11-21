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

Every distinct failure signature keeps one reproducer in ``--out``.  The
exit status is 1 when any failure was found, so the script can gate a job.

Examples::

    python3 tests/fuzz/mutate_frontend.py --lccc target/fastbuild/lccc \
        --seeds 'tests/**/*.c' --iterations 900 --out /tmp/lccc-mutfuzz -- -O1

    # Re-check a saved reproducer (or any file) without mutating it:
    python3 tests/fuzz/mutate_frontend.py --lccc target/fastbuild/lccc \
        --replay /tmp/lccc-mutfuzz/timeout-8a41ade96f.c -- -O0

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
import os
import random
import re
import subprocess
import sys
import time
from pathlib import Path

MAX_SEED_BYTES = 6000

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
            capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return "timeout", "TIMEOUT"
    err = proc.stderr.decode("latin-1")
    if proc.returncode < 0:
        return "signal", f"SIG{-proc.returncode}"
    if "internal error" in err or "panicked at" in err:
        m = (re.search(r"panicked at ([^\n:]+:\d+)", err)
             or re.search(r"internal error: ([^\n]{0,90})", err))
        return "ice", m.group(1) if m else err[:80]
    return ("ok" if proc.returncode == 0 else "diagnostic"), ""


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
    flags = flags or ["-O1"]
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    found: dict[str, list[str]] = {}
    stats = {"total": 0, "ok": 0, "diagnostic": 0, "ice": 0, "signal": 0, "timeout": 0}
    t0 = time.time()

    def record(kind: str, sig: str, src: bytes, origin: str) -> None:
        stats[kind] += 1
        if kind in ("ok", "diagnostic"):
            return
        digest = hashlib.sha1(src).hexdigest()[:10]
        key = f"{kind}: {sig}"
        if key not in found:
            (out / f"{kind}-{digest}.c").write_bytes(src)
        found.setdefault(key, []).append(f"{digest} ({origin})")

    if args.replay:
        for name in args.replay:
            stats["total"] += 1
            kind, sig = run_one(args.lccc, Path(name), flags, args.timeout)
            record(kind, sig, Path(name).read_bytes(), name)
    else:
        seeds = [p for p in glob.glob(args.seeds, recursive=True)
                 if os.path.getsize(p) < MAX_SEED_BYTES]
        if not seeds:
            print(f"no seed files match {args.seeds!r}", file=sys.stderr)
            return 2
        seeds.sort()
        rng = random.Random(args.seed)
        scratch = out / "current.c"
        for _ in range(args.iterations):
            seed_path = rng.choice(seeds)
            src = mutate(Path(seed_path).read_bytes(), rng)
            scratch.write_bytes(src)
            stats["total"] += 1
            kind, sig = run_one(args.lccc, scratch, flags, args.timeout)
            record(kind, sig, src, seed_path)
        scratch.unlink(missing_ok=True)

    failures = stats["ice"] + stats["signal"] + stats["timeout"]
    if args.json:
        print(json.dumps({"stats": stats, "flags": flags, "seed": args.seed,
                          "signatures": {k: len(v) for k, v in found.items()}},
                         indent=2, sort_keys=True))
    else:
        print(f"{json.dumps(stats)} in {time.time() - t0:.0f}s, flags {' '.join(flags)}")
        for key, hits in sorted(found.items(), key=lambda kv: -len(kv[1])):
            print(f"{len(hits):5d}  {key}   e.g. {hits[0]}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
