#!/usr/bin/env python3
"""Byte-precise type-punning differential fuzzer for LCCC (x86-64, little-endian).

Store-to-load forwarding, redundant-load elimination, SROA and the byte-load
reassembly pass all have to agree with the *representation* of an object: a
store of one scalar type followed by a load of another type at an overlapping
offset must observe exactly the bytes that were written.  The existing
differential fuzzers never generate that shape (no `union`, no `memcpy`
reinterpretation), so a forwarding rule that is wrong only for mixed-width
punning would pass them.

Reference semantics
-------------------
The expected output is computed by an independent Python model of the object
as a flat byte array, in the style of Vellvm's byte-level memory model
(`src/rocq/Semantics/MemoryBytes.v`: `dvalue_to_memory_bytes` /
`memory_bytes_to_dvalue`): a store serialises the value into its
little-endian representation, a load deserialises whatever bytes are present
at the address, regardless of the type that wrote them.  No Vellvm code is
copied; only the byte-serialisation discipline is reused.

The generated C only uses *defined* punning forms:
  * union member access (`c->u16[i]`) through a union whose members are
    arrays of `unsigned char`-compatible representations, and
  * `memcpy` of the object representation at arbitrary byte offsets,
  * `unsigned char` reads/writes of individual bytes (character types may
    alias anything, C11 6.5p7).
No signed overflow, no out-of-bounds index, no invalid shifts, no unspecified
evaluation order.  Signed-to-narrow conversions are modular (C11 6.3.1.3p3 /
GCC and LCCC behaviour).

Self-check
----------
`--ccc gcc` runs the same corpus through GCC: a disagreement there is a bug in
the *model* (or in the generator's definedness argument), not in a compiler.
Run that first; only then trust the LCCC verdicts.

Coverage (T7 widening): integer types 8..64 bits, `float` and `double` (stored
and reloaded as bit patterns, with the exponent forced finite so no NaN payload
semantics are exercised), and `__int128` / `unsigned __int128` (16 bytes, hashed
as two 64-bit words).  Lanes: `base` (the default target) and `v3`
(`-march=x86-64-v3`, the AVX2/BMI2 tuning the CPU model selects on modern hosts).
Not covered yet: bit-fields and struct padding (see the T7 follow-up record).

Usage:
  punning_fuzz.py --ccc gcc --seeds 0:200 --levels O0,O2 --jobs 2 --out /tmp/p  # model self-check
  punning_fuzz.py --ccc ./target/fastbuild/lccc --seeds 0:500 --levels O0,O1,O2,O3,Os --lanes base,v3 --out /tmp/p
"""
from __future__ import annotations

import argparse
import concurrent.futures as futures
import json
import random
import subprocess
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

MASK64 = (1 << 64) - 1
LCG_A = 6364136223846793005
LCG_C = 1442695040888963407
MIX_PRIME = 0x100000001B3

# name -> (size in bytes, signed, C spelling).  The name is also the union
# member name, so `c->{name}[i]` is the typed access in emitted code.
TYPES = {
    "u8": (1, False, "uint8_t"),
    "i8": (1, True, "int8_t"),
    "u16": (2, False, "uint16_t"),
    "i16": (2, True, "int16_t"),
    "u32": (4, False, "uint32_t"),
    "i32": (4, True, "int32_t"),
    "u64": (8, False, "uint64_t"),
    "i64": (8, True, "int64_t"),
    "f32": (4, False, "float"),
    "f64": (8, False, "double"),
    "u128": (16, False, "unsigned __int128"),
    "i128": (16, True, "__int128"),
}
FLOAT_TYPES = {"f32": 4, "f64": 8}  # name -> size; values are bit patterns
OBJECT_SIZE = 16  # bytes in `union cell` (large enough for the 128-bit members)
SETTERS = ("uset", "mset", "rawset")
MASK128 = (1 << 128) - 1
LANES = {"base": [], "v3": ["-march=x86-64-v3"]}

UNION_DEF = """\
union cell {
  unsigned char raw[16];
  uint8_t u8[16];   int8_t i8[16];
  uint16_t u16[8];  int16_t i16[8];
  uint32_t u32[4];  int32_t i32[4];
  uint64_t u64[2];  int64_t i64[2];
  float f32[4];     double f64[2];
  unsigned __int128 u128[1]; __int128 i128[1];
};
"""

HELPERS = """\
static uint64_t mix(uint64_t h, uint64_t x) {
  h = (h ^ x) * UINT64_C(0x100000001b3);
  return h ^ (h >> 29);
}
/* Mirror of finite_bits() in the Python model: clear the top exponent bit when
 * the exponent is all ones, so stored floats are never inf or NaN. */
static uint32_t fin32(uint64_t s) {
  uint32_t b = (uint32_t)s;
  if (((b >> 23) & 0xFFu) == 0xFFu) b &= ~(UINT32_C(1) << 30);
  return b;
}
static uint64_t fin64(uint64_t s) {
  uint64_t b = s;
  if (((b >> 52) & 0x7FFu) == 0x7FFu) b &= ~(UINT64_C(1) << 62);
  return b;
}
"""


# --------------------------------------------------------------------------
# Reference model
# --------------------------------------------------------------------------

def trunc(value: int, ty: str) -> int:
    """Modular conversion of a Python int to type `ty` (two's complement)."""
    size, signed, _ = TYPES[ty]
    bits = 8 * size
    v = value & ((1 << bits) - 1)
    if signed and v >> (bits - 1):
        v -= 1 << bits
    return v


def encode(value: int, size: int) -> bytes:
    return (value & ((1 << (8 * size)) - 1)).to_bytes(size, "little")


def decode(data: bytes, ty: str) -> int:
    return trunc(int.from_bytes(data, "little"), ty)


def finite_bits(bits: int, ty: str) -> int:
    """Clear the top exponent bit when the exponent is all ones (inf/NaN).

    Mirrored in C by `fin32` / `fin64` in HELPERS.  Only the exponent-all-ones
    case is changed, so every stored value is finite and the hash is over bit
    patterns, not over floating-point values.
    """
    if ty == "f32":
        b = bits & 0xFFFFFFFF
        if (b >> 23) & 0xFF == 0xFF:
            b &= ~(1 << 30)
        return b
    b = bits & MASK64
    if (b >> 52) & 0x7FF == 0x7FF:
        b &= ~(1 << 62)
    return b


def store_bits(x: int, ty: str) -> int:
    """The value a typed store writes: `(ty)x`, finitised for float types."""
    v = trunc(x, ty)
    return finite_bits(v, ty) if ty in FLOAT_TYPES else v


def load_words(data: bytes, ty: str) -> list[int]:
    """64-bit words a typed load hashes, matching the C side.

    One word for <= 64-bit types (sign-extended like `(uint64_t)` of the signed
    value; the bit pattern for floats), two words (low, high) for 128-bit types.
    """
    if TYPES[ty][0] == 16:
        return [int.from_bytes(data[:8], "little"), int.from_bytes(data[8:], "little")]
    if ty in FLOAT_TYPES:
        return [int.from_bytes(data, "little")]
    return [decode(data, ty) & MASK64]


@dataclass(frozen=True)
class Op:
    kind: str  # uset | uget | mset | mget | rawset | rawget
    ty: str    # value type ("u8" for raw ops)
    off: int   # byte offset inside the object
    shift: int  # value = trunc(s >> shift)


@dataclass
class Case:
    seed: int
    bodies: list[list[Op]]
    rounds: int
    s0: int
    h0: int


def gen_case(seed: int) -> Case:
    r = random.Random(seed)
    bodies: list[list[Op]] = []
    for _ in range(2):
        ops: list[Op] = []
        for _ in range(r.randint(8, 24)):
            kind = r.choice(["uset", "uget", "mset", "mget", "rawset", "rawget"])
            if kind == "rawset" or kind == "rawget":
                ty = "u8"
                off = r.randrange(OBJECT_SIZE)
            else:
                ty = r.choice(list(TYPES))
                size = TYPES[ty][0]
                if kind in ("uset", "uget"):
                    # union arrays are naturally aligned
                    off = size * r.randrange(OBJECT_SIZE // size)
                else:
                    off = r.randrange(OBJECT_SIZE - size + 1)
            ops.append(Op(kind, ty, off, r.randrange(0, 41)))
        bodies.append(ops)
    return Case(seed=seed, bodies=bodies, rounds=r.randint(2, 4),
                s0=r.getrandbits(64), h0=r.getrandbits(64))


def mix(h: int, x: int) -> int:
    h = ((h ^ (x & MASK64)) * MIX_PRIME) & MASK64
    return h ^ (h >> 29)


def run_model(case: Case) -> str:
    """Expected stdout of the generated program."""
    mems = {"local": bytearray(OBJECT_SIZE), "global": bytearray(OBJECT_SIZE)}
    s, h = case.s0, case.h0
    for _ in range(case.rounds):
        for mem_name in ("local", "global"):
            for ops in case.bodies:
                mem = mems[mem_name]
                for op in ops:
                    size = TYPES[op.ty][0]
                    if op.kind in SETTERS:
                        # Only stores advance the value stream (see emit_op).
                        s = (s * LCG_A + LCG_C) & MASK64
                    if op.kind in ("uset", "mset"):
                        val = store_bits(s >> op.shift, op.ty)
                        mem[op.off:op.off + size] = encode(val, size)
                    elif op.kind == "rawset":
                        mem[op.off] = (s >> op.shift) & 0xFF
                    elif op.kind in ("uget", "mget"):
                        for w in load_words(bytes(mem[op.off:op.off + size]), op.ty):
                            h = mix(h, w)
                    elif op.kind == "rawget":
                        h = mix(h, mem[op.off])
    return f"{h:016x} {s:016x}\n"


# --------------------------------------------------------------------------
# C emission
# --------------------------------------------------------------------------

def emit_op(op: Op) -> str:
    ctype = TYPES[op.ty][2]
    size = TYPES[op.ty][0]
    is_float = op.ty in FLOAT_TYPES
    head = "  s = s * UINT64_C(6364136223846793005) + UINT64_C(1442695040888963407);"
    if is_float:
        fin = "fin32" if size == 4 else "fin64"
        bits_t = "uint32_t" if size == 4 else "uint64_t"
        if op.kind == "uset":
            # Store the finitised bit pattern as a float/double value.
            return (f"{head}\n  {{ {bits_t} b = {fin}(s >> {op.shift}); {ctype} f;"
                    f" memcpy(&f, &b, {size}); c->{op.ty}[{op.off // size}] = f; }}")
        if op.kind == "uget":
            return (f"  {{ {ctype} f = c->{op.ty}[{op.off // size}]; {bits_t} t;"
                    f" memcpy(&t, &f, {size}); h = mix(h, t); }}")
        if op.kind == "mset":
            return (f"{head}\n  {{ {bits_t} t = {fin}(s >> {op.shift});"
                    f" memcpy(&c->raw[{op.off}], &t, {size}); }}")
        if op.kind == "mget":
            return (f"  {{ {bits_t} t; memcpy(&t, &c->raw[{op.off}], {size});"
                    f" h = mix(h, t); }}")
    val = f"({ctype})(s >> {op.shift})"
    if size == 16:
        if op.kind == "uset":
            return f"{head}\n  c->{op.ty}[0] = {val};"
        if op.kind == "uget":
            return (f"  h = mix(h, (uint64_t)c->{op.ty}[0]);"
                    f" h = mix(h, (uint64_t)((unsigned __int128)c->{op.ty}[0] >> 64));")
        if op.kind == "mset":
            return (f"{head}\n  {{ {ctype} t = {val};"
                    f" memcpy(&c->raw[{op.off}], &t, 16); }}")
        if op.kind == "mget":
            return (f"  {{ {ctype} t; memcpy(&t, &c->raw[{op.off}], 16);"
                    f" h = mix(h, (uint64_t)t);"
                    f" h = mix(h, (uint64_t)((unsigned __int128)t >> 64)); }}")
    if op.kind == "uset":
        return f"{head}\n  c->{op.ty}[{op.off // size}] = {val};"
    if op.kind == "uget":
        return f"  h = mix(h, (uint64_t)c->{op.ty}[{op.off // size}]);"
    if op.kind == "mset":
        return (f"{head}\n  {{ {ctype} t = {val};"
                f" memcpy(&c->raw[{op.off}], &t, {size}); }}")
    if op.kind == "mget":
        return (f"  {{ {ctype} t; memcpy(&t, &c->raw[{op.off}], {size});"
                f" h = mix(h, (uint64_t)t); }}")
    if op.kind == "rawset":
        return f"{head}\n  c->raw[{op.off}] = (unsigned char)(s >> {op.shift});"
    if op.kind == "rawget":
        return f"  h = mix(h, c->raw[{op.off}]);"
    raise ValueError(op.kind)


def generate(case: Case) -> str:
    out = [
        "#include <inttypes.h>",
        "#include <stdint.h>",
        "#include <stdio.h>",
        "#include <string.h>",
        "",
        UNION_DEF,
        HELPERS,
    ]
    for i, ops in enumerate(case.bodies):
        out.append(f"__attribute__((noinline)) static uint64_t body{i}("
                   "union cell *c, uint64_t *sp, uint64_t h) {")
        out.append("  uint64_t s = *sp;")
        out.extend(emit_op(op) for op in ops)
        out.append("  *sp = s;")
        out.append("  return h;")
        out.append("}")
        out.append("")
    out += [
        "static union cell global_cell;",
        "",
        "int main(void) {",
        "  union cell local_cell;",
        "  memset(&local_cell, 0, sizeof local_cell);",
        f"  uint64_t s = UINT64_C(0x{case.s0:016x});",
        f"  uint64_t h = UINT64_C(0x{case.h0:016x});",
        f"  for (int r = 0; r < {case.rounds}; r++) {{",
    ]
    for mem in ("&local_cell", "&global_cell"):
        for i in range(len(case.bodies)):
            out.append(f"    h = body{i}({mem}, &s, h);")
    out += [
        "  }",
        '  printf("%016" PRIx64 " %016" PRIx64 "\\n", h, s);',
        "  return 0;",
        "}",
        "",
    ]
    return "\n".join(out)


# --------------------------------------------------------------------------
# Runner
# --------------------------------------------------------------------------

@dataclass
class Result:
    seed: int
    level: str
    status: str  # pass | mismatch | compile-fail | run-fail | timeout
    detail: str = ""


def invoke(cmd: list[str], timeout: int) -> tuple[int | str, str, str]:
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return "timeout", "", ""


@dataclass(frozen=True)
class Job:
    seed: int
    ccc: str
    level: str   # without the dash, e.g. "O2"
    lane: str    # key of LANES
    out_dir: Path
    timeout: int


def one_case(job: Job) -> Result:
    seed, level, lane = job.seed, job.level, job.lane
    label = f"{level}/{lane}"
    case = gen_case(seed)
    expected = run_model(case)
    # Per-job names: jobs for one seed run concurrently at different levels/lanes.
    stem = f"punning_{seed:06d}_{level}_{lane}"
    src = job.out_dir / f"{stem}.c"
    exe = job.out_dir / f"{stem}.bin"
    src.write_text(generate(case))
    cmd = [job.ccc, f"-{level}", *LANES[lane], "-w", str(src), "-o", str(exe)]
    rc, _, err = invoke(cmd, job.timeout)
    if rc == "timeout":
        return Result(seed, label, "timeout", "compile")
    if rc != 0:
        return Result(seed, label, "compile-fail", err.strip()[-400:])
    rc, stdout, _ = invoke([str(exe)], job.timeout)
    exe.unlink(missing_ok=True)
    if rc == "timeout":
        return Result(seed, label, "timeout", "run")
    if rc != 0:
        return Result(seed, label, "run-fail", f"exit {rc}")
    if stdout != expected:
        return Result(seed, label, "mismatch",
                      f"expected {expected.strip()!r} got {stdout.strip()!r}")
    src.unlink(missing_ok=True)
    return Result(seed, label, "pass")


def parse_seeds(spec: str) -> range:
    lo, _, hi = spec.partition(":")
    return range(int(lo), int(hi))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--ccc", default=None,
                    help="compiler under test; `gcc` runs the model self-check")
    ap.add_argument("--seeds", default="0:100", help="half-open range lo:hi")
    ap.add_argument("--levels", default="O0,O1,O2,O3,Os")
    ap.add_argument("--lanes", default="base",
                    help=f"comma list from {sorted(LANES)}; base = default target")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--out", type=Path, default=Path("/tmp/punning-fuzz"))
    ap.add_argument("--emit-only", type=int, metavar="SEED",
                    help="print the generated C and expected output for SEED")
    args = ap.parse_args()

    if args.emit_only is not None:
        case = gen_case(args.emit_only)
        print(generate(case))
        print(f"/* expected: {run_model(case).strip()} */")
        return 0

    if args.ccc is None:
        ap.error("--ccc is required unless --emit-only is given")
    args.out.mkdir(parents=True, exist_ok=True)
    levels = [lv.strip() for lv in args.levels.split(",") if lv.strip()]
    lanes = [ln.strip() for ln in args.lanes.split(",") if ln.strip()]
    bad = [ln for ln in lanes if ln not in LANES]
    if bad:
        ap.error(f"unknown lane(s) {bad}; choose from {sorted(LANES)}")
    jobs = [Job(s, args.ccc, lv, ln, args.out, args.timeout)
            for s in parse_seeds(args.seeds) for lv in levels for ln in lanes]
    results: list[Result] = []
    with futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as ex:
        for res in ex.map(one_case, jobs):
            results.append(res)
            if res.status != "pass":
                print(f"FAIL seed={res.seed} -{res.level}: {res.status} {res.detail}",
                      file=sys.stderr)

    counts: dict[str, int] = {}
    for res in results:
        counts[res.status] = counts.get(res.status, 0) + 1
    summary = {"compiler": args.ccc, "cases": len(results), "counts": counts,
               "failures": [asdict(r) for r in results if r.status != "pass"]}
    (args.out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({"compiler": args.ccc, "cases": len(results), "counts": counts}))
    return 0 if set(counts) <= {"pass"} else 1


if __name__ == "__main__":
    sys.exit(main())
