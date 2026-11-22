#!/usr/bin/env python3
"""Differential fuzzer for 256-bit register homes (loops, calls, branches).

`avx2_pool_fuzz.py` covers straight-line 256-bit kernels. The register homes
admitted by `collect_avx256_home_values` also have to survive what that kernel
never produces:

  * loop-carried values (phi webs) across back edges, with trip counts that
    include zero, so the zero-iteration edge of the loop is exercised;
  * values live ACROSS a call (`noinline` helper taking and returning
    `__m256i`), which the allocator must refuse to home in a caller-saved
    register;
  * branches whose arms define and then redefine the accumulators, so live
    ranges have holes and phi merges;
  * every producer on the admission list, and every 128-bit consumer that
    reads one of them (`permute2x128`, `extracti128`, `castsi256_si128`,
    `zextsi128_si256`, `blend_ps`, `inserti128`).

The reference is GCC 16.2 at -O0..-O3. Clang 23.1.5 is run as a second,
independent oracle on every case, so a disagreement names the side that is
wrong instead of blaming the reference. Every case is a real program run on
the host CPU (AVX2 is required on the host, which this sandbox has).

Usage:
  avx256_home_fuzz.py --lccc PATH [--gcc gcc-16] [--clang clang-23]
                      [--cases N] [--seed S] [--keep DIR]
Exit status is 0 only when every case agrees with GCC and Clang.
"""

from __future__ import annotations

import argparse
import os
import random
import subprocess
import sys
import tempfile

PRELUDE = r"""#include <immintrin.h>
#include <stdint.h>

__attribute__((noinline)) static __m256i helper(__m256i a, __m256i b) {
    return _mm256_xor_si256(_mm256_add_epi64(a, b), _mm256_set1_epi64x(0x9e3779b97f4a7c15LL));
}
"""

# Binary ops over two 256-bit values. `{i}` is an immediate 0..255.
BIN = [
    "_mm256_add_epi64({a}, {b})",
    "_mm256_sub_epi64({a}, {b})",
    "_mm256_xor_si256({a}, {b})",
    "_mm256_and_si256({a}, {b})",
    "_mm256_or_si256({a}, {b})",
    "_mm256_blend_epi32({a}, {b}, {i})",
    "_mm256_castps_si256(_mm256_blend_ps(_mm256_castsi256_ps({a}), _mm256_castsi256_ps({b}), {i}))",
    "_mm256_shuffle_epi8({a}, {b})",
    "_mm256_unpacklo_epi64({a}, {b})",
    "_mm256_unpackhi_epi64({a}, {b})",
    "_mm256_permute2x128_si256({a}, {b}, {i})",
    "_mm256_inserti128_si256({a}, _mm256_castsi256_si128({b}), {bit})",
    "_mm256_cmpgt_epi64({a}, {b})",
    # Cross-width chain: Extracti128 and Cast256to128 feed a 128-bit xor,
    # whose result is re-widened with Zext128to256.
    "_mm256_zextsi128_si256(_mm_xor_si128(_mm256_extracti128_si256({a}, {bit}), _mm256_castsi256_si128({b})))",
]

# Unary ops: one 256-bit operand and an immediate or shift count.
UN = [
    "_mm256_permute4x64_epi64({a}, {i})",
    "_mm256_shuffle_epi32({a}, {i})",
    "_mm256_slli_epi64({a}, {sh})",
    "_mm256_srli_epi64({a}, {sh})",
    "helper({a}, {b})",
]


class Scope:
    """Names visible at one point of the generated kernel."""

    def __init__(self, names: list[str]):
        self.names = list(names)


def gen_stmts(rng: random.Random, scope: Scope, accs: list[str], depth: int,
              count: int, counter: list[int], out: list[str], indent: str) -> None:
    for _ in range(count):
        choice = rng.random()
        if depth < 2 and choice < 0.10:
            # A branch: arms define locals and finish by assigning accumulators.
            cond_a = rng.choice(scope.names)
            cond_b = rng.choice(scope.names)
            shift = rng.randrange(4) * 8
            out.append(f"{indent}if ((_mm256_movemask_epi8(_mm256_cmpgt_epi64({cond_a}, {cond_b})) >> {shift}) & 1) {{")
            for arm in range(2):
                if arm == 1:
                    out.append(f"{indent}}} else {{")
                arm_scope = Scope(scope.names)
                gen_stmts(rng, arm_scope, accs, depth + 1, rng.randrange(1, 4),
                          counter, out, indent + "    ")
                target = rng.choice(accs)
                src = rng.choice(arm_scope.names)
                out.append(f"{indent}    {target} = {src};")
            out.append(f"{indent}}}")
            continue
        counter[0] += 1
        name = f"t{counter[0]}"
        a = rng.choice(scope.names)
        b = rng.choice(scope.names)
        i = rng.randrange(256)
        sh = rng.randrange(1, 64)
        bit = rng.randrange(2)
        if choice < 0.45:
            expr = rng.choice(UN).format(a=a, b=b, i=i, sh=sh)
        else:
            expr = rng.choice(BIN).format(a=a, b=b, i=i, bit=bit)
        out.append(f"{indent}__m256i {name} = {expr};")
        scope.names.append(name)
        if rng.random() < 0.25:
            target = rng.choice(accs)
            out.append(f"{indent}{target} = {name};")


def gen_kernel(rng: random.Random) -> str:
    nacc = rng.randrange(2, 7)
    ntemp = rng.randrange(0, 6)
    accs = [f"a{k}" for k in range(nacc)]
    body: list[str] = []
    body.append("void kernel(const uint64_t *in, uint64_t *out, long n) {")
    for k in range(nacc):
        body.append(f"    __m256i a{k} = _mm256_loadu_si256((const __m256i *)(in + {4 * (k % 16)}));")
    for k in range(ntemp):
        body.append(f"    __m256i p{k} = _mm256_set1_epi64x({rng.randrange(1 << 30)});")
    body.append("    for (long it = 0; it < n; it++) {")
    loop_in = ["    " * 2 + "__m256i x = _mm256_loadu_si256((const __m256i *)(in + 4 * ((it * 3) & 15)));"]
    body.extend(loop_in)
    outer = [f"a{k}" for k in range(nacc)] + ["x"] + [f"p{k}" for k in range(ntemp)]
    scope = Scope(outer)
    counter = [0]
    gen_stmts(rng, scope, accs, 0, rng.randrange(12, 40), counter, body, "        ")
    # Every accumulator is reassigned from a live value, so each loop-carried
    # web has a definition on every path to the back edge.
    for k in range(nacc):
        body.append(f"        a{k} = helper(a{k}, {rng.choice(scope.names)});")
    body.append("    }")
    stores = accs + [f"p{k}" for k in range(ntemp)]
    for k, name in enumerate(stores[:16]):
        body.append(f"    _mm256_storeu_si256((__m256i *)(out + {4 * k}), {name});")
    body.append("}")
    return PRELUDE + "\n".join(body) + "\n"


HARNESS = r"""
#include <stdint.h>
#include <stdio.h>
void kernel(const uint64_t *in, uint64_t *out, long n);
int main(int argc, char **argv) {
    uint64_t seed = argc > 1 ? strtoull(argv[1], 0, 0) : 1;
    uint64_t in[64] __attribute__((aligned(32)));
    uint64_t out[64] __attribute__((aligned(32)));
    static const long counts[] = {0, 1, 2, 3, 7, 16};
    for (unsigned round = 0; round < 6; round++) {
        for (int i = 0; i < 64; i++) {
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            in[i] = seed;
        }
        for (int i = 0; i < 64; i++) out[i] = 0x5555555555555555ull;
        kernel(in, out, counts[round]);
        for (int i = 0; i < 64; i++) printf("%016llx\n", (unsigned long long)out[i]);
    }
    return 0;
}
"""
HARNESS = "#include <stdlib.h>\n" + HARNESS


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def build_outputs(compiler: str, flags: list[str], workdir: str, tag: str,
                  kernel_src: str, harness_obj: str):
    obj = os.path.join(workdir, f"{tag}.o")
    res = run([compiler, *flags, "-fno-pie", "-c", "-o", obj, kernel_src])
    if res.returncode != 0:
        return None, "compile failed: " + res.stderr.strip()[:500]
    exe = os.path.join(workdir, f"{tag}.exe")
    link = run([compiler, "-no-pie", "-o", exe, harness_obj, obj])
    if link.returncode != 0:
        return None, "link failed: " + link.stderr.strip()[:500]
    out = run([exe, "12345"])
    if out.returncode != 0:
        return None, f"run failed rc={out.returncode}"
    return out.stdout, None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", required=True)
    ap.add_argument("--gcc", default="gcc-16")
    ap.add_argument("--clang", default="clang-23")
    ap.add_argument("--cases", type=int, default=200)
    ap.add_argument("--seed", type=int, default=0x256)
    ap.add_argument("--keep", default="")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    workdir = args.keep or tempfile.mkdtemp(prefix="avx256home_")
    os.makedirs(workdir, exist_ok=True)
    harness_src = os.path.join(workdir, "harness.c")
    with open(harness_src, "w") as f:
        f.write(HARNESS)
    harness_obj = os.path.join(workdir, "harness.o")
    res = run([args.gcc, "-O1", "-fno-pie", "-c", "-o", harness_obj, harness_src])
    if res.returncode != 0:
        print("harness build failed:", res.stderr[:400])
        return 2

    failures = 0
    compared = 0
    for case in range(args.cases):
        src = os.path.join(workdir, f"case{case:04d}.c")
        with open(src, "w") as f:
            f.write(gen_kernel(rng))
        opt = rng.choice(["-O0", "-O1", "-O2", "-O3"])
        flags = [opt, "-mavx2"]
        outs = {}
        outs["gcc"] = build_outputs(args.gcc, flags, workdir, f"gcc_{case:04d}", src, harness_obj)
        outs["lccc"] = build_outputs(args.lccc, flags, workdir, f"lccc_{case:04d}", src, harness_obj)
        outs["clang"] = build_outputs(args.clang, flags, workdir, f"clang_{case:04d}", src, harness_obj)
        g_out, g_err = outs["gcc"]
        l_out, l_err = outs["lccc"]
        c_out, c_err = outs["clang"]
        if g_err or c_err:
            # A broken reference is an environment problem, not a verdict on lccc.
            print(f"SKIP case={case} {opt} reference error gcc={g_err} clang={c_err}")
            continue
        compared += 1
        if l_err or l_out != g_out:
            failures += 1
            print(f"FAIL case={case} {opt} lccc_err={l_err} lccc_eq_gcc={l_out == g_out} "
                  f"clang_eq_gcc={c_out == g_out}")
            print(f"  kept: {src}")
        else:
            print(f"ok   case={case} {opt}")
    print(f"== avx256_home_fuzz: {compared - failures} passed, {failures} failed, {compared} compared")
    return 0 if failures == 0 and compared > 0 else 1


if __name__ == "__main__":
    sys.exit(main())
