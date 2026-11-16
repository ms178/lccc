#!/usr/bin/env python3
"""Codegen gate: an integer broadcast must not round-trip a live %ymm0 value.

`avx_fp_private` loads a 256-bit vector, patches one lane through
`_mm256_set1_epi32` + `_mm256_blend_ps`, and stores it back. The loaded base
is live in %ymm0 across the broadcast. Before the fix, the base was committed
to its frame slot before the broadcast and reloaded for the blend, a 32-byte
store and reload on the loop-carried chain. The fix stages the broadcast in
the other scratch bank so the base stays in %ymm0.

Checks, all against the real compiler output:

  (1) SHAPE   in the hot loop, between the base load into %ymm0 and the
              blend, there is no vector memory operation on the stack, and
              the blend reads the base straight from %ymm0.
  (2) VALUE   the benchmark prints the reference checksum (computed by GCC and
              Clang, 2088761911016512).
  (3) PATCH   the bit-exact regression test for patched lanes exits 0.

Usage: check_broadcast_pending_codegen.py [--lccc PATH] [--repo DIR]
       check_broadcast_pending_codegen.py --shape-only ASM [--expect-defect]
Exit status 0 on pass. The gate is also run against the pre-fix compiler as a
negative control: it must fail there.
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile

REFERENCE = "avx_fp_private checksum=2088761911016512"
BCAST = re.compile(r"vpbroadcastd %xmm\d+, %ymm\d+")
LOAD0 = re.compile(r"v(?:movups|movdqu|movdqa)\s+(?:-?\d*\(%[a-z0-9]+\)|\(%[a-z0-9]+\)),\s*%ymm0$")
BLEND0 = re.compile(r"vblendps \$\d+, %ymm\d+, %ymm0, %ymm0$")
STACK_VEC = re.compile(r"v(?:movups|movdqu|movdqa)\s+(?:%ymm\d+,\s*-?\d*\(%rsp\)|-?\d*\(%rsp\),\s*%ymm\d+)")


def check_shape(asm_lines):
    hits = [i for i, l in enumerate(asm_lines) if BCAST.search(l)]
    if not hits:
        return "no integer broadcast (`vpbroadcastd`) in the output"
    for b in hits:
        load = next((j for j in range(b - 1, max(-1, b - 24), -1) if LOAD0.search(asm_lines[j])), None)
        if load is None:
            return f"broadcast at line {b + 1}: no base load into %ymm0 before it"
        blend = next((j for j in range(b + 1, min(len(asm_lines), b + 12)) if BLEND0.search(asm_lines[j])), None)
        if blend is None:
            return f"broadcast at line {b + 1}: no blend reading the base from %ymm0 after it"
        for j in range(load, blend + 1):
            if STACK_VEC.search(asm_lines[j]):
                return f"stack round trip of the live base between lines {load + 1} and {blend + 1}: {asm_lines[j].strip()}"
    return None


# A 256-bit store that writes a vector to memory, in any of the spellings the
# backend emits: the source is a %ymm register and the destination is memory.
YMM_STORE = re.compile(r"^\s*v(?:movups|movdqu|movdqa|movaps|movapd)\s+%ymm\d+,\s*-?\d*\(%[a-z0-9]+\)\s*$")
CALL = re.compile(r"^\s*call[lq]?\s")


def check_blend_publish(asm_lines):
    """(1b) PUBLISH: a blend result is stored exactly once before the next call.

    The blend result is the new `input[0]`, and the callee reads it through
    memory, so one store that publishes it is required.  Any second vector
    store before the call is a copy of a value nothing reads: the frame home
    of the blend result, flushed because the publishing store was refused as
    its consumer.  Measured on avx_fp_private: the pre-fix loop stored the
    blend result to 751(%rsp) and then to input[0] before the call.
    """
    blends = [i for i, l in enumerate(asm_lines) if BLEND0.search(l)]
    if not blends:
        return "no blend into %ymm0 in the output: the check would be vacuous"
    for b in blends:
        stores = []
        for k in range(b + 1, min(len(asm_lines), b + 24)):
            if CALL.search(asm_lines[k]):
                break
            if YMM_STORE.search(asm_lines[k]):
                stores.append(k)
        if len(stores) != 1:
            where = ", ".join(str(k + 1) for k in stores) or "none"
            return (f"blend at line {b + 1} is followed by {len(stores)} vector stores "
                    f"before the call (lines {where}); exactly one (the publishing store) "
                    f"is required")
    return None


def run(cmd, **kw):
    return subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=180, **kw)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", default="target/fastbuild/lccc")
    ap.add_argument("--repo", default=".")
    ap.add_argument("--shape-only", metavar="ASM",
                    help="check only the SHAPE leg on an existing .s file")
    ap.add_argument("--expect-defect", action="store_true",
                    help="invert: succeed only if the check fails (non-vacuity)")
    a = ap.parse_args()
    if a.shape_only:
        with open(a.shape_only) as f:
            lines = f.read().splitlines()
        msg = check_shape(lines) or check_blend_publish(lines)
        if a.expect_defect:
            if msg is None:
                print("FAIL: expected the stack round trip to be detected, but the shape passed",
                      file=sys.stderr)
                return 1
            print("defect detected as expected:", msg)
            return 0
        if msg:
            print("FAIL (1) SHAPE:", msg, file=sys.stderr)
            return 1
        print("(1) SHAPE ok")
        return 0
    bench = os.path.join(a.repo, "tests/benchmark/programs/avx_fp_private.c")
    patch = os.path.join(a.repo, "tests/regression/avx_lane_broadcast_pending.c")
    failures = []
    with tempfile.TemporaryDirectory() as d:
        asm = os.path.join(d, "fp.s")
        r = run([a.lccc, "-O2", "-mavx2", "-std=gnu17", "-S", bench, "-o", asm])
        if r.returncode != 0:
            failures.append(f"(1) compile failed: {r.stderr.decode(errors='replace')}")
        else:
            with open(asm) as f:
                lines = f.read().splitlines()
            msg = check_shape(lines)
            if msg:
                failures.append(f"(1) SHAPE: {msg}")
            else:
                print("(1) SHAPE ok: no stack round trip of the live base around the broadcast")
            msg = check_blend_publish(lines)
            if msg:
                failures.append(f"(1b) PUBLISH: {msg}")
            else:
                print("(1b) PUBLISH ok: the blend result is stored once, before the call")

        exe = os.path.join(d, "fp")
        r = run([a.lccc, "-O2", "-mavx2", "-std=gnu17", bench, "-o", exe])
        if r.returncode != 0:
            failures.append("(2) VALUE: benchmark does not compile")
        else:
            out = run([exe]).stdout.decode(errors="replace").strip()
            if out != REFERENCE:
                failures.append(f"(2) VALUE: got {out!r}, want {REFERENCE!r}")
            else:
                print(f"(2) VALUE ok: {out}")

        pexe = os.path.join(d, "patch")
        r = run([a.lccc, "-O2", "-mavx2", "-std=gnu17", patch, "-o", pexe])
        if r.returncode != 0:
            failures.append("(3) PATCH: regression test does not compile")
        else:
            p = run([pexe])
            if p.returncode != 0:
                failures.append(f"(3) PATCH: exit {p.returncode}: {p.stderr.decode(errors='replace')}")
            else:
                print("(3) PATCH ok:", p.stdout.decode(errors="replace").strip())

    for f in failures:
        print("FAIL", f, file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
