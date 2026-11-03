#!/usr/bin/env python3
"""Differential sweep of the host-clock macros against the zone database.

`__TIMESTAMP__`, `__DATE__` and `__TIME__` are the only predefined macros whose
value comes from outside the translation unit, and the local-time path goes
through `localtime_r`. This sweep therefore checks the *whole* rendering chain --
zone lookup, DST decision, civil conversion, field padding -- against an
independent implementation of the same question, over many instants and several
zones:

* seven zones, chosen for the cases that break naive offset arithmetic:
  `Asia/Kolkata` (+05:30, no DST), `Pacific/Chatham` (+12:45/+13:45),
  `Australia/Lord_Howe` (a 30-minute DST shift rather than an hour),
  `America/New_York` and `Europe/Berlin` (ordinary DST), `America/Santa_Isabel`
  (a legacy alias that may be absent -- then both sides fall back to UTC and
  still agree), and `UTC` as the control;
* instants that are boundary cases on their own: the epoch, one second either
  side of it, the last and first second of a day, the 2038 rollover, a leap day,
  and a spread of pseudo-random instants before and after the epoch (a negative
  `time_t` must floor to the second and then to the day, never truncate toward
  zero);
* the `SOURCE_DATE_EPOCH` path, where all three macros must render the same
  instant in UTC whatever `TZ` says.

The expectation comes from Python's `time` module, which formats through a
different API than the compiler uses, so agreement is evidence about the
rendering rather than one library agreeing with itself. The seed is fixed: a
failure is reproducible by re-running the script.
"""
from __future__ import annotations

import os
import random
import subprocess
import sys
import tempfile
import time

ZONES = [
    "UTC",
    "Asia/Kolkata",
    "Pacific/Chatham",
    "Australia/Lord_Howe",
    "America/New_York",
    "Europe/Berlin",
    "America/Santa_Isabel",
]

# Instants every zone must agree on, before any random sampling.
FIXED = [
    0,
    -1,
    1,
    -86_400,
    86_399,
    86_400,
    951_782_400,      # 2000-02-29, the leap day of a divisible-by-400 year
    981_173_106,      # 2001-02-03T04:05:06Z, a Saturday
    946_684_798,      # 1999-12-31T23:59:58Z
    1_234_567_890,    # 2009-02-13T23:31:30Z
    2**31 - 1,        # the last second a signed 32-bit time_t can hold
    2**31,            # and the first it cannot
]

SEED = 20261102

# The largest instant both reference compilers accept: 9999-12-31T23:59:59Z.
MAX_SOURCE_DATE_EPOCH = 253_402_300_799


def observations(seed: int) -> list[int]:
    rng = random.Random(seed)
    stamps = list(FIXED)
    # Ordinary instants across the whole epoch, then a tight cluster around it so
    # that day/month/year rollover at the boundary is sampled densely.
    stamps += [rng.randint(-2_000_000_000, 2_000_000_000) for _ in range(24)]
    stamps += [rng.randint(-50_000, 50_000) for _ in range(6)]
    return stamps


def run(compiler: str, source: str, env: dict[str, str]) -> dict[str, str]:
    proc = subprocess.run(
        [compiler, "-E", "-P", source], capture_output=True, text=True, env=env
    )
    if proc.returncode != 0:
        raise SystemExit(f"lccc failed to preprocess the probe: {proc.stderr[:400]}")
    fields: dict[str, str] = {}
    for line in proc.stdout.splitlines():
        if '= "' not in line:
            continue
        # `const char *t = "...";` -> key "t".
        key = line.split("=")[0].strip().split()[-1].lstrip("*")
        fields[key] = line.split('"')[1]
    return fields


def main(argv: list[str]) -> int:
    compiler = argv[1] if len(argv) > 1 else "target/fastbuild/lccc"
    if not os.access(compiler, os.X_OK):
        print(f"error: no such compiler: {compiler}", file=sys.stderr)
        return 2

    work = tempfile.mkdtemp(prefix="lccc-host-time-")
    source = os.path.join(work, "probe.c")
    with open(source, "w") as handle:
        handle.write(
            'const char *t = __TIMESTAMP__;\n'
            'const char *d = __DATE__;\n'
            'const char *c = __TIME__;\n'
        )

    stamps = observations(SEED)
    failures = 0
    checked = 0

    for zone in ZONES:
        os.environ["TZ"] = zone
        time.tzset()
        for stamp in stamps:
            env = dict(os.environ)
            env.pop("SOURCE_DATE_EPOCH", None)
            os.utime(source, (stamp, stamp))
            got = run(compiler, source, env).get("t")
            want = time.strftime("%a %b %e %H:%M:%S %Y", time.localtime(stamp))
            checked += 1
            if got != want:
                failures += 1
                if failures <= 12:
                    print(f"FAIL TZ={zone} @{stamp}: lccc={got!r} want={want!r}")

    # SOURCE_DATE_EPOCH: reproducible, UTC, and shared by all three macros.
    # Only instants both reference compilers accept are candidates: a negative or
    # five-digit-year value is *rejected* by GCC 16.2 and Clang 23.1.4 (and by
    # lccc, which diagnoses it), so it cannot carry a reproducible value.
    for stamp in [s for s in stamps if 0 <= s <= MAX_SOURCE_DATE_EPOCH]:
        env = dict(os.environ)
        env.pop("TZ", None)
        env["SOURCE_DATE_EPOCH"] = str(stamp)
        fields = run(compiler, source, env)
        for key, want in (
            ("t", time.strftime("%a %b %e %H:%M:%S %Y", time.gmtime(stamp))),
            ("d", time.strftime("%b %e %Y", time.gmtime(stamp))),
            ("c", time.strftime("%H:%M:%S", time.gmtime(stamp))),
        ):
            checked += 1
            if fields.get(key) != want:
                failures += 1
                if failures <= 24:
                    print(f"FAIL SOURCE_DATE_EPOCH @{stamp} {key}: "
                          f"lccc={fields.get(key)!r} want={want!r}")

    os.environ.pop("TZ", None)
    time.tzset()
    print(f"host-time-zones: {checked} observation(s), {failures} failure(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
