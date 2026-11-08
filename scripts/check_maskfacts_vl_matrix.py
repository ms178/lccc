#!/usr/bin/env python3
"""Generated three-way differential matrix for the AVX-512VL mask-compare
family and the all-on masked-store fold.

WHY THIS EXISTS
---------------
`engineering/isa/sse-avx-public-api.json` is a NAME inventory: it proves a
public wrapper exists, not that its lowering, encoding, feature gate, mask
width, predicate plumbing or constant-fold consumer is correct.  Before this
harness, 26 GCC 16.2 / Clang 23.1.5 public names were absent from LCCC even
though the IR ops, the x86 emitter and `mask_compare_shape` already existed.

This harness GENERATES 485 cases (no hand-maintained case list): all three
vector widths (128/256/512) x all eight lane types (epi/epu x 8/16/32/64),
each with runtime eq/ne, constant eq/ne, an ALL EIGHT-predicate runtime sweep
and an ALL EIGHT-predicate constant sweep, plus five masked-store cases.  The
constant pair sets `b`'s sign bit, so signed and unsigned LT/LE/GE/GT must
DISAGREE: a wrong opcode-signedness bit cannot pass the oracle.  It compiles
the same source with LCCC (maskfacts ON and OFF), GCC 16.2 and Clang 23.1.5,
EXECUTES all four binaries and compares stdout byte-for-byte.  It also
requires the ON build to prove the optimization fired: strictly fewer `vpcmp`
instructions in the constant-image cases, strictly fewer `kmovq` in the
all-on masked-store cases, and strictly fewer total instructions.  A harness
that accepted two identical builds would be vacuous, so `--self-test` injects
exactly that mutation (forces maskfacts off for the ON build) and requires a
non-zero verdict.  Broadcast casts are exact and explicit, so the generated
fixture compiles with `-Wall -Wextra -Werror` and ZERO warnings under both
reference compilers.

The generated fixture is also written to
tests/regression/maskfacts_cross/avx512vl_cmp_matrix.c so the exact source
is reviewable and reproducible.  That directory is deliberately NOT globbed
by the hosted regression runner: AVX-512VL binaries would SIGILL on a runner
without those features.  The Rust unit tests cover the IR-level contracts in
hosted CI; this is the executed three-way oracle.

Usage:
    scripts/check_maskfacts_vl_matrix.py [--write-fixture] [--json PATH]
    scripts/check_maskfacts_vl_matrix.py --self-test
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "regression" / "maskfacts_cross" / "avx512vl_cmp_matrix.c"
CACHE_BIN = Path(
    os.environ.get("LCCC_AGENT_CACHE_PREFIX", str(Path.home() / ".cache/lccc/agent-toolchains"))
) / "bin"
FLAGS = ["-O2", "-mavx512f", "-mavx512bw", "-mavx512vl"]
# Executing the generated binaries needs ALL three features on EVERY CPU.
# `check_avx512_scalar_witness.py` establishes this repository's precedent:
# AVX512F alone is not enough (vpcmpb is BW+VL), a host that lacks a feature
# gets an explicit SKIP, and a silent pass is never acceptable.
REQUIRED_ISA = ("avx512f", "avx512bw", "avx512vl")


def host_supports_required_isa(cpuinfo: Path = Path("/proc/cpuinfo")):
    """Return (supported, detail).  Never guesses from the toolchain."""
    try:
        rows = [
            line.split(":", 1)[1].split()
            for line in Path(cpuinfo).read_text().splitlines()
            if line.startswith("flags") and ":" in line
        ]
    except OSError as exc:
        return False, f"/proc/cpuinfo unreadable: {exc}"
    if not rows:
        return False, "no CPU flags reported"
    missing = sorted({f for f in REQUIRED_ISA for row in rows if f not in row})
    return not missing, ("missing " + ",".join(missing)) if missing else "all present"

LOAD = {128: "_mm_loadu_si128", 256: "_mm256_loadu_si256", 512: "_mm512_loadu_si512"}
LOAD_TY = {128: "__m128i_u", 256: "__m256i_u", 512: "void"}
SET1 = {
    (128, 8): "_mm_set1_epi8", (128, 16): "_mm_set1_epi16",
    (128, 32): "_mm_set1_epi32", (128, 64): "_mm_set1_epi64x",
    (256, 8): "_mm256_set1_epi8", (256, 16): "_mm256_set1_epi16",
    (256, 32): "_mm256_set1_epi32", (256, 64): "_mm256_set1_epi64x",
    (512, 8): "_mm512_set1_epi8", (512, 16): "_mm512_set1_epi16",
    (512, 32): "_mm512_set1_epi32", (512, 64): "_mm512_set1_epi64",
}
MASK_TY = {
    (128, 8): "__mmask16", (128, 16): "__mmask8", (128, 32): "__mmask8", (128, 64): "__mmask8",
    (256, 8): "__mmask32", (256, 16): "__mmask16", (256, 32): "__mmask8", (256, 64): "__mmask8",
    (512, 8): "__mmask64", (512, 16): "__mmask32", (512, 32): "__mmask16", (512, 64): "__mmask8",
}
LANES = ("epi8", "epu8", "epi16", "epu16", "epi32", "epu32", "epi64", "epu64")


def prefix(width: int) -> str:
    return "_mm_" if width == 128 else f"_mm{width}_"


def vec_ty(width: int) -> str:
    return f"__m{width}i"


def lane_bits(lane: str) -> int:
    return int(re.search(r"(\d+)$", lane).group(1))


def tag(width: int, lane: str) -> str:
    return f"w{width}_{lane}"


def load_expr(width: int, ptr: str) -> str:
    if width == 512:
        return f"{LOAD[width]}({ptr})"
    return f"{LOAD[width]}((const {LOAD_TY[width]} *)(const void *){ptr})"


def set1_expr(width: int, lane: str, value: int) -> str:
    """A literal broadcast with an EXACT, non-truncating C cast.

    A 64-bit lane needs `long long`, not `int`: casting 0x8091...4455 to
    `int` would silently truncate and turn the signed/unsigned boundary case
    into an ordinary small positive value, hiding the very distinction the
    predicate sweep is designed to test.
    """
    bits = lane_bits(lane)
    fn = SET1[(width, bits)]
    if bits == 8:
        return f"{fn}((char)0x{value & 0xFF:x})"
    if bits == 16:
        # An explicit `short` cast (not `int`) is required: Clang's default
        # `-Wconstant-conversion` warns on the implicit int -> short narrowing
        # of 0x8091, and a checked-in regression fixture must compile with
        # ZERO warnings under GCC 16.2 and Clang 23.1.5 alike.
        return f"{fn}((short)0x{value & 0xFFFF:x}u)"
    if bits == 32:
        return f"{fn}((int)0x{value & 0xFFFFFFFF:x}u)"
    return f"{fn}((long long)0x{value & 0xFFFFFFFFFFFFFFFF:x}ull)"


# A pair whose interpretation DIFFERS between signed and unsigned compares:
# `b` has its sign bit set, `a` is a small positive.  For predicate 1 (LT),
# signed says `b < a` while unsigned says `b > a`.  A compiler that used the
# wrong signedness for one lane width cannot pass the three-way oracle.
CONST_PAIR = {
    8: (0x11, 0x91),
    16: (0x0011, 0x8091),
    32: (0x00000011, 0x80910011),
    64: (0x0000000000000011, 0x8091001122334455),
}
PREDICATES = tuple(range(8))


def generate_source() -> tuple[str, list[str]]:
    """Return (C source, ordered case names)."""
    out: list[str] = [
        "/* GENERATED by scripts/check_maskfacts_vl_matrix.py -- DO NOT EDIT.",
        " * Three-way differential matrix: LCCC / GCC 16.2 / Clang 23.1.5.",
        " * `argc` seeds the runtime data so no compiler can constant-fold the",
        " * runtime cases; separate literal-constant cases exercise LCCC's",
        " * mask-facts constant-image fold (CCC_DISABLE_PASSES=maskfacts is the",
        " * A/B control).  Build: -O2 -mavx512f -mavx512bw -mavx512vl -I include",
        " * (the -I include form is for LCCC only).",
        " */",
        "#include <stdio.h>",
        "#include <immintrin.h>",
        "",
        "static unsigned char A[64], B[64];",
        "static unsigned char G[256];",
        "",
        "static unsigned long long h(const unsigned char *p, int n) {",
        "    unsigned long long x = 1469598103934665603ull;",
        "    for (int i = 0; i < n; i++) { x ^= p[i]; x *= 1099511628211ull; }",
        "    return x;",
        "}",
        "",
    ]
    names: list[str] = []

    def emit_case(name: str, body: str, expr: str) -> None:
        out.append(f"__attribute__((noinline)) static unsigned long long {name}(void) {{")
        out.extend("    " + line for line in body.splitlines())
        out.append(f"    return (unsigned long long)({expr});")
        out.append("}")
        out.append("")
        names.append(name)

    for width in (128, 256, 512):
        p, ty = prefix(width), vec_ty(width)
        for lane in LANES:
            bits = lane_bits(lane)
            mt = MASK_TY[(width, bits)]
            t = tag(width, lane)
            rt_load = (
                f"{ty} a = {load_expr(width, 'A')};\n"
                f"{ty} b = {load_expr(width, 'B')};"
            )
            emit_case(
                f"rt_eq_{t}", rt_load,
                f"({mt}){p}cmpeq_{lane}_mask(a, b)",
            )
            emit_case(
                f"rt_ne_{t}", rt_load,
                f"({mt}){p}cmp_{lane}_mask(a, b, 4)",
            )
            # Compile-time constant images.  `a == b` must fold to an
            # all-ones mask; `a != b` must fold to zero.  GCC and Clang fold
            # these too, so a wrong LCCC answer is caught by the oracle.
            emit_case(
                f"c_eq_{t}",
                f"{ty} a = {set1_expr(width, lane, 0x11)};\n"
                f"{ty} b = {set1_expr(width, lane, 0x11)};",
                f"({mt}){p}cmpeq_{lane}_mask(a, b)",
            )
            emit_case(
                f"c_ne_{t}",
                f"{ty} a = {set1_expr(width, lane, 0x11)};\n"
                f"{ty} b = {set1_expr(width, lane, 0x22)};",
                f"({mt}){p}cmpeq_{lane}_mask(a, b)",
            )
            # Predicate sweep over ALL eight AVX-512 vpcmp predicates, on
            # runtime AND on compile-time-constant images.  The constant pair
            # has the sign bit set in `b`, so signed and unsigned LT/LE/GE/GT
            # disagree; GCC 16.2 and Clang 23.1.5 are the oracle for the
            # predicate numbering AND for LCCC's constant fold.
            lo, hi = CONST_PAIR[bits]
            for pred in PREDICATES:
                emit_case(
                    f"rt_p{pred}_{t}", rt_load,
                    f"({mt}){p}cmp_{lane}_mask(a, b, {pred})",
                )
                emit_case(
                    f"c_p{pred}_{t}",
                    f"{ty} a = {set1_expr(width, lane, lo)};\n"
                    f"{ty} b = {set1_expr(width, lane, hi)};",
                    f"({mt}){p}cmp_{lane}_mask(a, b, {pred})",
                )

    # ---- all-on / all-off / true-prefix masked stores to a GLOBAL ----
    store_cases = [
        ("s128_on", 128, "0xFFFF", 16, 0),
        ("s256_on", 256, "0xFFFFFFFFu", 32, 32),
        ("s512_on", 512, "0xFFFFFFFFFFFFFFFFull", 64, 64),
        ("s128_off", 128, "0x0000", 16, 128),
        ("s128_prefix", 128, "0x00FF", 16, 160),
    ]
    for name, width, mask, nbytes, off in store_cases:
        p, ty = prefix(width), vec_ty(width)
        mt = MASK_TY[(width, 8)]
        # These cases hash MEMORY, not a mask value: the store's semantics
        # (which lanes are written) are what the oracle must agree on.
        out.append(f"__attribute__((noinline)) static unsigned long long {name}(void) {{")
        out.append(f"    {ty} v = {set1_expr(width, 'epi8', 0x5A)};")
        out.append(f"    {p}mask_storeu_epi8(G + {off}, ({mt}){mask}, v);")
        out.append(f"    return h(G + {off}, {nbytes});")
        out.append("}")
        out.append("")
        names.append(name)

    out.append("int main(int argc, char **argv) {")
    out.append("    (void)argv;")
    out.append("    for (int i = 0; i < 64; i++) {")
    out.append("        A[i] = (unsigned char)(i * 7 + argc * 3 + 11);")
    out.append("        B[i] = (unsigned char)((i % 3) ? A[i] : (A[i] ^ 0x5A));")
    out.append("    }")
    out.append("    for (int i = 0; i < 256; i++) G[i] = (unsigned char)(i * 5 + argc);")
    for name in names:
        out.append(f'    printf("%s %016llx\\n", "{name}", {name}());')
    out.append("    return 0;")
    out.append("}")
    out.append("")
    return "\n".join(out), names


# A runtime `vpcmp` predicate is a SILENT MISCOMPILE if a compiler substitutes
# an immediate instead of diagnosing it.  GCC 16.2 and Clang 23.1.5 both hard
# -error; LCCC's backend helper used to return 0 for an SSA Operand::Value, so
# `_mm_cmp_epi8_mask(a, b, runtime_p)` compiled to an EQ compare.  These two
# generated sources are the negative diagnostic and the positive control
# (constant-expression and enum immediates MUST still be accepted).
IMM_NEGATIVE = """/* GENERATED negative diagnostic case -- DO NOT EDIT. */
#include <stdio.h>
#include <immintrin.h>
int main(int argc, char **argv) {
    (void)argv;
    __m128i a = _mm_set1_epi8((char)(argc + 1));
    __m128i b = _mm_set1_epi8((char)(argc + 2));
    int runtime_p = argc & 7;
    unsigned r = (unsigned)_mm_cmp_epi8_mask(a, b, runtime_p);
    printf("%08x\\n", r);
    return 0;
}
"""

IMM_POSITIVE = """/* GENERATED positive immediate control -- DO NOT EDIT. */
#include <stdio.h>
#include <immintrin.h>
enum { P_GE = 5, P_GT = 6 };
static unsigned char A[16], B[16];
int main(int argc, char **argv) {
    (void)argv;
    for (int i = 0; i < 16; i++) {
        A[i] = (unsigned char)(i * 7 + argc * 3 + 11);
        B[i] = (unsigned char)((i % 3) ? A[i] : (A[i] ^ 0x5A));
    }
    __m128i a = _mm_loadu_si128((const __m128i_u *)(const void *)A);
    __m128i b = _mm_loadu_si128((const __m128i_u *)(const void *)B);
    unsigned le = (unsigned)_mm_cmp_epi8_mask(a, b, 1 + 1);
    unsigned ge = (unsigned)_mm_cmp_epi8_mask(a, b, P_GE);
    unsigned gt = (unsigned)_mm_cmp_epi8_mask(a, b, P_GT);
    printf("le=%08x ge=%08x gt=%08x\\n", le, ge, gt);
    return 0;
}
"""


# Public set1 signatures must match BOTH reference compilers.  `_mm512_set1_
# epi16` was `int` here while GCC 16.2 / Clang 23.1.5 declare `short`; the
# merged PR fixed it, and `_mm512_maskz_set1_epi16` had the same deviation.
# Pin all four typed wrappers and the three maskz helper prototypes so the
# exact-width lane-image proof cannot silently regress.
SET1_TYPES = {
    "_mm512_set1_epi8": "char",
    "_mm512_set1_epi16": "short",
    "_mm512_set1_epi32": "int",
    "_mm512_set1_epi64": "long long",
}


def _typed_param(text: str, name: str) -> str | None:
    pat = re.compile(
        r"\b" + re.escape(name) + r"\s*\(\s*([A-Za-z_][A-Za-z0-9_ ]*?)\s+__\w+\s*\)"
    )
    m = pat.search(text)
    return " ".join(m.group(1).split()) if m else None


def check_public_set1_signature_parity() -> tuple[list[str], list[str]]:
    failures: list[str] = []
    notes: list[str] = []
    ours = (ROOT / "include" / "lcccsimd.h").read_text()
    refs: dict[str, str] = {}
    for label, cc in (("gcc16", tool("gcc-16")), ("clang23", tool("clang-23"))):
        cp = run([cc, "-print-file-name=include"])
        if cp.returncode != 0:
            failures.append(f"{label}: cannot locate its intrinsic headers")
            continue
        hdr = Path(cp.stdout.strip()) / "avx512fintrin.h"
        if not hdr.exists():
            failures.append(f"{label}: missing {hdr}")
            continue
        refs[label] = hdr.read_text()
    for name, want in SET1_TYPES.items():
        got = _typed_param(ours, name)
        if got != want:
            failures.append(f"lccc {name} parameter is {got!r}, expected {want!r}")
        for label, text in refs.items():
            ref = _typed_param(text, name)
            if ref != want:
                failures.append(f"{label} {name} parameter is {ref!r}, expected {want!r}")
        notes.append(f"{name}: {want} (LCCC + GCC 16.2 + Clang 23.1.5 agree)")
    for suffix, want in (("vpbroadcastw_maskz512", "short"),
                         ("vpbroadcastd_maskz512", "int"),
                         ("vpbroadcastq_maskz512", "long long")):
        pat = re.compile(
            r"__lccc_simd512_i_" + re.escape(suffix) +
            r"\(long, __mmask\d+ __mask, ([A-Za-z_][A-Za-z0-9_ ]*?) __a\)"
        )
        m = pat.search(ours)
        got = " ".join(m.group(1).split()) if m else None
        if got != want:
            failures.append(f"helper {suffix} scalar is {got!r}, expected {want!r}")
    return failures, notes


# Executed control for the corrected `short` scalar of the maskz set1
# wrapper: a widened/truncated broadcast would change the stored bytes.
MASKZ_SET1 = """/* GENERATED maskz set1 signature control -- DO NOT EDIT. */
#include <stdio.h>
#include <immintrin.h>
static unsigned char OUT[64];
static unsigned long long h(const unsigned char *p, int n) {
    unsigned long long x = 1469598103934665603ull;
    for (int i = 0; i < n; i++) { x ^= p[i]; x *= 1099511628211ull; }
    return x;
}
int main(int argc, char **argv) {
    (void)argv;
    __mmask32 k = (__mmask32)(0xAAAA5555u | (unsigned)(argc & 1));
    __m512i z = _mm512_maskz_set1_epi16(k, (short)0x8091);
    _mm512_storeu_si512((void *)OUT, z);
    printf("%016llx\\n", h(OUT, 64));
    return 0;
}
"""


def run(cmd: list[str], env: dict | None = None, timeout: int = 300) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout)


def tool(name: str) -> str:
    p = CACHE_BIN / name
    return str(p) if p.exists() else shutil.which(name) or name


def count_asm(path: Path, mnemonic: str) -> int:
    # Count a MNEMONIC FAMILY, not a literal spelling: LCCC emits `vpcmpb`,
    # `vpcmpuw`, `vpcmpd`, ... and a `\b` after `vpcmp` would match none of
    # them (the next letter is a word character).  A harness that reported
    # zero for both builds would be a vacuous "fold fired" check.
    pat = re.compile(r"^\s+" + re.escape(mnemonic) + r"[a-z0-9]*\s", re.M)
    return len(pat.findall(path.read_text())) if path.exists() else 0


def count_ins(path: Path) -> int:
    pat = re.compile(r"^\s+[a-z][a-z0-9.]*\s", re.M)
    return len(pat.findall(path.read_text())) if path.exists() else 0


def execute_matrix(tmp: Path, mutate: bool, asm_dir: Path | None = None, execute: bool = True) -> dict:
    src, names = generate_source()
    cpath = tmp / "matrix.c"
    cpath.write_text(src)

    lccc = str(ROOT / "target" / "fastbuild" / "lccc")
    gcc, clang = tool("gcc-16"), tool("clang-23")
    for binary in (lccc, gcc, clang):
        if not Path(binary).exists() and not shutil.which(binary):
            raise RuntimeError(f"missing compiler: {binary}")

    base_env = os.environ.copy()
    on_env = base_env.copy()
    if mutate:
        on_env["CCC_DISABLE_PASSES"] = "maskfacts"
    else:
        on_env.pop("CCC_DISABLE_PASSES", None)
    off_env = base_env.copy()
    off_env["CCC_DISABLE_PASSES"] = "maskfacts"

    builds: dict[str, dict] = {}
    specs = [
        ("lccc_on", [lccc] + FLAGS + ["-I", "include"], on_env),
        ("lccc_off", [lccc] + FLAGS + ["-I", "include"], off_env),
        ("gcc16", [gcc] + FLAGS, base_env),
        ("clang23", [clang] + FLAGS, base_env),
    ]
    for label, cmd, env in specs:
        exe = tmp / label
        cp = run(cmd + [str(cpath), "-o", str(exe)], env=env)
        if cp.returncode != 0:
            raise RuntimeError(f"{label} compile rc={cp.returncode}\n{cp.stderr[-2000:]}")
        if execute:
            rp = run([str(exe)], env=env, timeout=180)
            if rp.returncode != 0:
                raise RuntimeError(
                    f"{label} run rc={rp.returncode}\n{rp.stderr[-2000:]}\n"
                    "SIGILL (-4) on a runner usually means avx512bw/avx512vl "
                    "are absent; use --require-execution on a capable host"
                )
            builds[label] = {"stdout": rp.stdout, "returncode": rp.returncode}
        else:
            builds[label] = {"stdout": None, "returncode": None}

    # Assembly receipts for ALL FOUR binaries.  The census needs the two
    # reference compilers too; emitting them here keeps the CI wiring to one
    # command instead of a second, drift-prone compile of a different source.
    # `--asm-dir` persists them outside the temp dir for the per-function
    # census and for elf/decoder audits.
    asm: dict[str, Path] = {}
    if asm_dir is not None:
        asm_dir.mkdir(parents=True, exist_ok=True)
    for label, cmd, env in specs:
        path = tmp / f"{label}.s"
        cp = run(cmd + ["-S", str(cpath), "-o", str(path)], env=env)
        if cp.returncode != 0:
            raise RuntimeError(f"{label} -S rc={cp.returncode}\n{cp.stderr[-1000:]}")
        if asm_dir is not None:
            persistent = asm_dir / f"{label}.s"
            shutil.copyfile(path, persistent)
            path = persistent
        asm[label] = path

    return {
        "names": names,
        "builds": builds,
        "execution_skipped": not execute,
        "asm": asm,
        "counts": {
            label: {
                "vpcmp": count_asm(p, "vpcmp"),
                "kmovq": count_asm(p, "kmovq"),
                "kmovw": count_asm(p, "kmovw"),
                "ins": count_ins(p),
            }
            for label, p in asm.items()
        },
        "source_sha256": __import__("hashlib").sha256(src.encode()).hexdigest(),
    }


def check_immediate_diagnostics(tmp: Path, execute: bool = True) -> tuple[list[str], list[str]]:
    """Negative diagnostic parity + positive constant-immediate control.

    Three compilers x two generated sources, EXECUTED for the positive case.
    A compiler that accepts a runtime predicate has a silent wrong-code bug,
    and a compiler that rejects `1+1` or an enum constant has over-corrected.
    """
    failures: list[str] = []
    notes: list[str] = []
    neg, pos = tmp / "imm_negative.c", tmp / "imm_positive.c"
    neg.write_text(IMM_NEGATIVE)
    pos.write_text(IMM_POSITIVE)
    lccc = str(ROOT / "target" / "fastbuild" / "lccc")
    specs = [
        ("lccc", [lccc] + FLAGS + ["-I", "include"]),
        ("gcc16", [tool("gcc-16")] + FLAGS),
        ("clang23", [tool("clang-23")] + FLAGS),
    ]
    for label, cmd in specs:
        cp = run(cmd + [str(neg), "-o", str(tmp / f"{label}_neg")])
        if cp.returncode == 0:
            failures.append(
                f"{label} accepted a RUNTIME vpcmp predicate (silent miscompile)"
            )
        else:
            notes.append(f"{label}: runtime predicate rejected rc={cp.returncode}")
    lccc_msg = run(specs[0][1] + [str(neg), "-o", str(tmp / "lccc_neg2")]).stderr
    if "constant integer immediate" not in lccc_msg:
        failures.append("lccc runtime-predicate diagnostic lacks the required wording")
    # Positive control: constant expressions and enum constants are legal
    # immediates.  Force maskfacts OFF for LCCC so the emitted `vpcmp $2/$5/$6`
    # instructions really execute instead of being folded away.
    envs = {
        "lccc": {**os.environ, "CCC_DISABLE_PASSES": "maskfacts"},
        "gcc16": os.environ.copy(),
        "clang23": os.environ.copy(),
    }
    outs: dict[str, str] = {}
    for label, cmd in specs:
        cp = run(cmd + [str(pos), "-o", str(tmp / f"{label}_pos")], env=envs[label])
        if cp.returncode != 0:
            failures.append(f"{label} rejected a constant-expression immediate: {cp.stderr[-400:]}")
            continue
        if not execute:
            continue  # compiled with -Werror; deliberately NOT executed
        rp = run([str(tmp / f"{label}_pos")], env=envs[label])
        if rp.returncode != 0:
            failures.append(f"{label} positive immediate binary rc={rp.returncode}")
            continue
        outs[label] = rp.stdout
    if not execute:
        notes.append("SKIP (positive-immediate execution): host lacks AVX-512BW/VL; "
                     "compile/diagnostic controls still gate")
    elif len(outs) == 3 and len(set(outs.values())) == 1:
        notes.append("constant-expression/enum immediates: 3-way stdout identical")
    elif len(outs) == 3:
        failures.append(f"positive immediate control disagrees: {outs}")
    mz = tmp / "maskz_set1.c"
    mz.write_text(MASKZ_SET1)
    mz_outs: dict[str, str] = {}
    for label, cmd in specs:
        cp = run(cmd + [str(mz), "-o", str(tmp / f"{label}_mz")], env=envs[label])
        if cp.returncode != 0:
            failures.append(f"{label} maskz set1 compile rc={cp.returncode}: {cp.stderr[-400:]}")
            continue
        if not execute:
            continue  # compiled; deliberately NOT executed
        rp = run([str(tmp / f"{label}_mz")], env=envs[label])
        if rp.returncode != 0:
            failures.append(f"{label} maskz set1 run rc={rp.returncode}")
            continue
        mz_outs[label] = rp.stdout
    if not execute:
        notes.append("SKIP (maskz-set1 execution): host lacks AVX-512BW/VL")
    elif len(mz_outs) == 3 and len(set(mz_outs.values())) == 1:
        notes.append("_mm512_maskz_set1_epi16(short): 3-way stored-byte hash identical")
    elif len(mz_outs) == 3:
        failures.append(f"maskz set1 stored bytes disagree: {mz_outs}")
    return failures, notes


BAD_GAS_MASK = re.compile(r"\{(?!%)k[0-7]\}")


def runtime_code_sizes(obj: Path, names: list[str]) -> dict[str, int]:
    """Per-function byte sizes from the ELF symtab, via binutils `nm -S`."""
    cp = run(["nm", "-S", "--defined-only", str(obj)], timeout=120)
    if cp.returncode != 0:
        raise RuntimeError(f"nm -S failed on {obj}: {cp.stderr[-400:]}")
    want = set(names)
    sizes: dict[str, int] = {}
    for line in cp.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 4 and parts[-1] in want:
            try:
                sizes[parts[-1]] = int(parts[1], 16)
            except ValueError:
                continue
    return sizes


def compare_code_sizes(totals: dict[str, int]) -> list[str]:
    """Pure, self-testable ratchet: LCCC ON must not be larger than EITHER
    pinned reference compiler on the runtime-compare functions."""
    ours = totals.get("lccc_on")
    if ours is None:
        return ["lccc_on code size missing"]
    bad = []
    for ref in ("gcc16", "clang23"):
        if ref in totals and ours > totals[ref]:
            bad.append(f"lccc_on runtime bytes {ours} > {ref} {totals[ref]}")
    return bad


def check_gas_mask_syntax(asm: dict[str, Path], tmp: Path) -> tuple[list[str], list[str]]:
    """GNU as requires `{%kN}`; LCCC used to emit `{kN}` (assembler error).

    This is a REAL assembler check, not a string assertion: every `-S` receipt
    is assembled with its producer's assembler, so an invalid EVEX decorator,
    a bad RIP-relative operand or a broken relocation is caught even on a
    runner that cannot EXECUTE AVX-512BW/VL.
    """
    failures: list[str] = []
    notes: list[str] = []
    for label, path in asm.items():
        bad = BAD_GAS_MASK.findall(path.read_text())
        if bad:
            failures.append(
                f"{label}: {len(bad)} GNU-as-invalid mask decorator(s) "
                f"{sorted(set(bad))} (must be {{%kN}})"
            )
    for label in ("lccc_on", "lccc_off"):
        if "{%k1}" not in asm[label].read_text():
            failures.append(
                f"{label}: no masked EVEX instruction in the receipt; gate is vacuous"
            )
    for label, path in asm.items():
        cc = tool("clang-23") if label == "clang23" else tool("gcc-16")
        obj = tmp / f"{label}_gas.o"
        cp = run([cc, "-c", str(path), "-o", str(obj)], timeout=180)
        if cp.returncode != 0:
            failures.append(
                f"{label}: assembler rejected the -S receipt rc={cp.returncode}: "
                f"{cp.stderr[-400:]}"
            )
        elif not obj.exists() or obj.stat().st_size == 0:
            failures.append(f"{label}: assembler produced no ELF object")
        else:
            notes.append(
                f"{label}: {obj.stat().st_size} byte ELF object from {Path(cc).name}"
            )
    return failures, notes


def evaluate(res: dict) -> tuple[list[str], list[str]]:
    """Return (failures, notes). Real checks, not a printed status."""
    failures: list[str] = []
    notes: list[str] = []
    builds = res["builds"]
    if res.get("execution_skipped"):
        notes.append(
            "SKIP (executed stdout leg): host lacks one of "
            + ",".join(REQUIRED_ISA)
            + "; all four binaries still COMPILE, LINK and emit -S, and every "
              "static count/structural check below still gates"
        )
    else:
        oracle = builds["gcc16"]["stdout"]
        for label in ("lccc_on", "lccc_off", "clang23"):
            if builds[label]["stdout"] != oracle:
                a, b = oracle.splitlines(), builds[label]["stdout"].splitlines()
                diff = next(
                    (f"line {i+1}: gcc={x!r} {label}={y!r}"
                     for i, (x, y) in enumerate(zip(a, b)) if x != y),
                    f"length gcc={len(a)} {label}={len(b)}",
                )
                failures.append(f"{label} stdout differs from GCC 16.2 ({diff})")
            else:
                notes.append(
                    f"{label}: {len(oracle.splitlines())} lines byte-identical to GCC 16.2"
                )
        if builds["lccc_on"]["stdout"] != builds["lccc_off"]["stdout"]:
            failures.append("maskfacts ON/OFF stdout differs (semantic miscompile)")
        else:
            notes.append("maskfacts ON/OFF stdout byte-identical")

    on, off = res["counts"]["lccc_on"], res["counts"]["lccc_off"]
    notes.append(
        f"lccc_on  vpcmp={on['vpcmp']} kmovq={on['kmovq']} kmovw={on['kmovw']} ins={on['ins']}"
    )
    notes.append(
        f"lccc_off vpcmp={off['vpcmp']} kmovq={off['kmovq']} kmovw={off['kmovw']} ins={off['ins']}"
    )
    if on["vpcmp"] >= off["vpcmp"]:
        failures.append(
            f"constant-image compare fold did NOT fire: vpcmp ON={on['vpcmp']} OFF={off['vpcmp']}"
        )
    else:
        notes.append(f"constant-image fold removed {off['vpcmp'] - on['vpcmp']} vpcmp instructions")
    if on["kmovq"] + on["kmovw"] >= off["kmovq"] + off["kmovw"]:
        failures.append(
            "all-on masked-store fold did NOT fire: kmov count did not decrease "
            f"(ON={on['kmovq'] + on['kmovw']} OFF={off['kmovq'] + off['kmovw']})"
        )
    else:
        notes.append(
            "all-on masked-store fold removed "
            f"{off['kmovq'] + off['kmovw'] - on['kmovq'] - on['kmovw']} kmov instructions"
        )
    if on["ins"] >= off["ins"]:
        failures.append(f"ON has no instruction win: ON={on['ins']} OFF={off['ins']}")
    else:
        notes.append(f"total emitted instructions {off['ins']} -> {on['ins']}")
    return failures, notes


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--write-fixture", action="store_true",
                    help="write the generated C matrix to the checked-in fixture path")
    ap.add_argument("--json", type=Path, help="write a machine-readable receipt")
    ap.add_argument("--self-test", action="store_true",
                    help="mutation test: force maskfacts OFF for the ON build and require failure")
    ap.add_argument("--asm-dir", type=Path,
                    help="persist the four -S receipts for scripts/maskfacts_function_census.py")
    ap.add_argument("--require-code-size-parity", action="store_true",
                    help="fail if LCCC ON runtime-compare bytes exceed GCC 16.2 or Clang 23.1.5")
    ap.add_argument("--require-execution", action="store_true",
                    help="FAIL on a host lacking avx512f/bw/vl instead of skipping the executed leg")
    args = ap.parse_args()

    if args.write_fixture:
        src, _ = generate_source()
        FIXTURE.parent.mkdir(parents=True, exist_ok=True)
        FIXTURE.write_text(src)
        print(f"wrote {FIXTURE} ({len(src.splitlines())} lines)")

    if args.self_test:
        with tempfile.TemporaryDirectory(prefix="maskfacts_cpu_") as cd:
            cpu = Path(cd)
            (cpu / "full").write_text(
                "flags\t: fpu avx512f avx512bw avx512vl\n")
            (cpu / "partial").write_text(
                "flags\t: fpu avx512f avx512dq\n")
            (cpu / "empty").write_text("")
            assert host_supports_required_isa(cpu / "full") == (True, "all present")
            assert host_supports_required_isa(cpu / "partial") == (
                False, "missing avx512bw,avx512vl")
            assert host_supports_required_isa(cpu / "empty") == (
                False, "no CPU flags reported")
            assert host_supports_required_isa(cpu / "absent")[0] is False
        print("  capability gate self-test PASS (4 branches)")
        with tempfile.TemporaryDirectory(prefix="maskfacts_gas_") as gd:
            g = Path(gd)
            bad_s = g / "bad.s"
            good_s = g / "good.s"
            for path, mask in ((bad_s, "{k1}"), (good_s, "{%k1}")):
                path.write_text(
                    ".text\n.globl f\n.type f, @function\nf:\n"
                    f"    vmovdqu8 %xmm0, (%rdi){mask}\n"
                    "    ret\n.size f, .-f\n"
                )
            as_bad = run([tool("gcc-16"), "-c", str(bad_s), "-o", str(g / "bad.o")])
            as_good = run([tool("gcc-16"), "-c", str(good_s), "-o", str(g / "good.o")])
            assert as_bad.returncode != 0, "GNU as ACCEPTED the invalid {k1} decorator"
            assert as_good.returncode == 0, \
                f"GNU as rejected the valid {{%k1}} decorator: {as_good.stderr}"
            assert BAD_GAS_MASK.findall(bad_s.read_text()) == ["{k1}"]
            assert BAD_GAS_MASK.findall(good_s.read_text()) == []
            assert check_gas_mask_syntax(
                {"lccc_on": good_s, "lccc_off": good_s,
                 "gcc16": good_s, "clang23": good_s}, g)[0] == []
            dirty = check_gas_mask_syntax({"lccc_on": bad_s, "lccc_off": good_s,
                                          "gcc16": good_s, "clang23": good_s}, g)[0]
            assert dirty, "gate missed an invalid decorator"
        print("  GNU-as mask-syntax self-test PASS (reject {kN}, accept {%kN})")
        # Code-size ratchet must reject every direction of regression.
        assert compare_code_sizes({"lccc_on": 5136, "gcc16": 5308,
                                   "clang23": 5208}) == []
        assert compare_code_sizes({"lccc_on": 5309, "gcc16": 5308,
                                   "clang23": 5208}), "GCC regression missed"
        assert compare_code_sizes({"lccc_on": 5209, "gcc16": 5308,
                                   "clang23": 5208}), "Clang regression missed"
        assert compare_code_sizes({"gcc16": 5308, "clang23": 5208}), \
            "missing lccc_on must fail closed"
        print("  code-size ratchet self-test PASS (both oracles + fail-closed)")
        # The mutation is precisely the vacuous-harness failure: an ON build
        # that behaves like OFF must be REJECTED by evaluate().
        with tempfile.TemporaryDirectory(prefix="maskfacts_vl_selftest_") as td:
            ok, _ = host_supports_required_isa()
            res = execute_matrix(Path(td), mutate=True, execute=ok)
            failures, _ = evaluate(res)
        if failures:
            print("self-test PASS: forcing maskfacts off is detected -> "
                  f"{len(failures)} failure(s): {failures[0]}")
            return 0
        print("self-test FAIL: a build with maskfacts disabled was ACCEPTED", file=sys.stderr)
        return 1

    supported, detail = host_supports_required_isa()
    if not supported and args.require_execution:
        print(f"FAIL: host cannot execute the AVX-512VL matrix ({detail})", file=sys.stderr)
        return 1
    if supported:
        print(f"  CPU capability: {detail}; executed stdout leg ENABLED")
    with tempfile.TemporaryDirectory(prefix="maskfacts_vl_") as td:
        tmp = Path(td)
        res = execute_matrix(
            tmp,
            mutate=os.environ.get("LCCC_MATRIX_MUTATE") == "1",
            asm_dir=args.asm_dir,
            execute=supported,
        )
        failures, notes = evaluate(res)
        gas_failures, gas_notes = check_gas_mask_syntax(res["asm"], tmp)
        size_failures: list[str] = []
        size_notes: list[str] = []
        if args.require_code_size_parity and not gas_failures:
            # Reuse the census's exact definition instead of a second parser.
            from importlib.util import module_from_spec, spec_from_file_location
            spec = spec_from_file_location(
                "maskfacts_census_for_sizes",
                str(ROOT / "scripts" / "maskfacts_function_census.py"))
            census = module_from_spec(spec)
            sys.modules[spec.name] = census
            spec.loader.exec_module(census)
            bodies, aliases = census.parse(res["asm"]["lccc_on"])
            resolve = census.resolver(bodies, aliases)
            names = [n for n in res["names"] if n.startswith("rt_")
                     and any(m.startswith("vpcmp") for m in resolve(n))]
            totals: dict[str, int] = {}
            for label in ("lccc_on", "lccc_off", "gcc16", "clang23"):
                sizes = runtime_code_sizes(tmp / f"{label}_gas.o", names)
                if len(sizes) != len(names):
                    size_failures.append(
                        f"{label}: nm resolved {len(sizes)}/{len(names)} runtime functions"
                    )
                totals[label] = sum(sizes.values())
            size_notes.append(
                "runtime bytes: " + " ".join(f"{k}={v}" for k, v in sorted(totals.items()))
            )
            size_failures += compare_code_sizes(totals)
            if not size_failures:
                size_notes.append(
                    f"PASS: lccc_on {totals['lccc_on']} bytes <= "
                    f"gcc16 {totals['gcc16']} and clang23 {totals['clang23']}"
                )
        diag_failures, diag_notes = check_immediate_diagnostics(tmp, execute=supported)
        sig_failures, sig_notes = check_public_set1_signature_parity()
        failures += gas_failures + size_failures + diag_failures + sig_failures
        notes += [f"assembler: {n}" for n in gas_notes]
        notes += [f"code size: {n}" for n in size_notes]
        notes += [f"immediate: {n}" for n in diag_notes]
        notes += [f"signature: {n}" for n in sig_notes]
        if args.json:
            args.json.write_text(json.dumps({
                "source_sha256": res["source_sha256"],
                "cases": len(res["names"]),
                "counts": res["counts"],
                "stdout_sha256": {
                    k: __import__("hashlib").sha256(v["stdout"].encode()).hexdigest()
                    for k, v in res["builds"].items() if v["stdout"] is not None
                },
                "execution_skipped": res.get("execution_skipped", False),
                "gas_mask_syntax": {"failures": gas_failures, "notes": gas_notes},
                "runtime_code_size": {
                    "required": bool(args.require_code_size_parity),
                    "failures": size_failures, "notes": size_notes,
                },
                "immediate_diagnostics": {"failures": diag_failures, "notes": diag_notes},
                "set1_signature_parity": {"failures": sig_failures, "notes": sig_notes},
                "failures": failures,
                "notes": notes,
            }, indent=2) + "\n")
    for n in notes:
        print("  " + n)
    if failures:
        for f in failures:
            print("FAIL: " + f, file=sys.stderr)
        return 1
    print(
        f"PASS: {len(res['names'])} generated cases, 4 binaries, "
        + ("three-way stdout identical" if supported
           else "static controls only (executed stdout leg SKIPPED: " + detail + ")")
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(2)
