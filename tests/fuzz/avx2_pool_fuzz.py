#!/usr/bin/env python3
"""Differential fuzzer for single-block 256-bit vector slot sharing.

Generates random straight-line AVX2 kernels that keep many __m256i temporaries
live at once (so the stack layout must place them in slots), builds each one
with lccc and with GCC 16, runs both on identical random inputs, and compares
an FNV-1a hash of the outputs. Any mismatch is a miscompile.

Usage: avx2_pool_fuzz.py --lccc <binary> [--gcc gcc-16] [--cases N]
                         [--seed S] [--keep DIR]
Exit status is 0 only when every case matches and no build fails.
"""

import argparse
import os
import random
import subprocess
import sys
import tempfile

OPS2 = [
    "_mm256_add_epi64({a}, {b})",
    "_mm256_sub_epi64({a}, {b})",
    "_mm256_xor_si256({a}, {b})",
    "_mm256_and_si256({a}, {b})",
    "_mm256_or_si256({a}, {b})",
    "_mm256_blend_epi32({a}, {b}, {imm8})",
    "_mm256_shuffle_epi8({a}, {b})",
]
OPS1 = [
    "_mm256_permute4x64_epi64({a}, {imm8})",
    "_mm256_shuffle_epi32({a}, {imm8})",
    "_mm256_slli_epi64({a}, {sh})",
    "_mm256_srli_epi64({a}, {sh})",
]


def gen_kernel(rng: random.Random, temps: int, steps: int) -> str:
    """One noinline kernel: `temps` loaded vectors, `steps` random ops that
    mix them, then every value is stored, so no temporary is dead code."""
    lines = []
    names = []
    for i in range(temps):
        lines.append(
            f"    __m256i t{i} = _mm256_loadu_si256((const __m256i *)(in + {4 * (i % 16)}));"
        )
        names.append(f"t{i}")
    for s in range(steps):
        dst = f"u{s}"
        # Operands are drawn from the whole live set, including early names,
        # so live ranges vary from very short to whole-function.
        a = rng.choice(names)
        if rng.random() < 0.35:
            op = rng.choice(OPS1)
            expr = op.format(
                a=a, imm8=rng.randrange(256), sh=rng.randrange(1, 64)
            )
        else:
            b = rng.choice(names)
            op = rng.choice(OPS2)
            expr = op.format(a=a, b=b, imm8=rng.randrange(256))
        lines.append(f"    __m256i {dst} = {expr};")
        names.append(dst)
    # Store the last `temps` values; earlier ones are still exercised as
    # operands above.
    for k, name in enumerate(names[-temps:]):
        lines.append(
            f"    _mm256_storeu_si256((__m256i *)(out + {4 * (k % 16)}), {name});"
            if k < 16
            else f"    _mm256_storeu_si256((__m256i *)(out + {4 * (k % 16)}), _mm256_xor_si256({name}, _mm256_set1_epi64x({k})));"
        )
    body = "\n".join(lines)
    return (
        "#include <immintrin.h>\n#include <stdint.h>\n"
        "__attribute__((noinline)) void kernel(const uint64_t *in, uint64_t *out) {\n"
        + body
        + "\n}\n"
    )


HARNESS = r"""
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
void kernel(const uint64_t *in, uint64_t *out);
int main(int argc, char **argv) {
    uint64_t seed = argc > 1 ? strtoull(argv[1], 0, 0) : 1;
    uint64_t in[64] __attribute__((aligned(32)));
    uint64_t out[64] __attribute__((aligned(32)));
    for (int round = 0; round < 8; round++) {
        for (int i = 0; i < 64; i++) {
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            in[i] = seed;
            out[i] = 0x5555555555555555ull;
        }
        kernel(in, out);
        for (int i = 0; i < 64; i++) printf("%016llx\n", (unsigned long long)out[i]);
    }
    return 0;
}
"""


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def build_and_run(compiler, src_dir, src_name, flags, extra_objs=()):
    exe = os.path.join(src_dir, os.path.basename(src_name) + "." + os.path.basename(compiler) + ".exe")
    cmd = [compiler, *flags, "-o", exe, os.path.join(src_dir, src_name), *extra_objs]
    res = run(cmd)
    if res.returncode != 0:
        return None, "build failed: " + res.stderr.strip()[:600]
    out = run([exe, "12345"])
    if out.returncode != 0:
        return None, f"run failed rc={out.returncode}"
    return out.stdout, None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", required=True)
    ap.add_argument("--gcc", default="gcc-16")
    ap.add_argument("--cases", type=int, default=200)
    ap.add_argument("--seed", type=int, default=0x5EED)
    ap.add_argument("--keep", default="")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    workdir = args.keep or tempfile.mkdtemp(prefix="avx2pool_")
    os.makedirs(workdir, exist_ok=True)
    harness = os.path.join(workdir, "harness.c")
    with open(harness, "w") as f:
        f.write(HARNESS)
    # Harness object is compiled once per compiler so that only the kernel
    # differs between the two sides.
    failures = 0
    for case in range(args.cases):
        temps = rng.randrange(8, 48)
        steps = rng.randrange(temps, 4 * temps)
        src_name = f"case{case:04d}.c"
        with open(os.path.join(workdir, src_name), "w") as f:
            f.write(gen_kernel(rng, temps, steps))
        opt = rng.choice(["-O0", "-O1", "-O2"])
        flags = [opt, "-mavx2"]
        # Harness is shared text; compile it separately per compiler inside
        # build_and_run via a tiny wrapper translation unit.
        kernel_src = os.path.join(workdir, src_name)
        results = {}
        for label, comp in (("gcc", args.gcc), ("lccc", args.lccc)):
            obj = os.path.join(workdir, f"{label}_{case:04d}.o")
            res = run([comp, *flags, "-c", "-o", obj, kernel_src])
            if res.returncode != 0:
                results[label] = (None, "compile failed: " + res.stderr.strip()[:400])
                continue
            hobj = os.path.join(workdir, f"harness_{label}.o")
            if not os.path.exists(hobj):
                run([comp, "-O1", "-c", "-o", hobj, harness])
            exe = os.path.join(workdir, f"{label}_{case:04d}.exe")
            link = run([comp, "-o", exe, hobj, obj])
            if link.returncode != 0:
                results[label] = (None, "link failed: " + link.stderr.strip()[:400])
                continue
            out = run([exe, str(args.seed + case)])
            results[label] = (out.stdout, None if out.returncode == 0 else f"rc={out.returncode}")
        g_out, g_err = results["gcc"]
        l_out, l_err = results["lccc"]
        if g_err or l_err or g_out != l_out:
            failures += 1
            print(f"FAIL case={case} {opt} temps={temps} steps={steps} "
                  f"gcc_err={g_err} lccc_err={l_err} "
                  f"outputs_equal={g_out == l_out}")
            print(f"  kept: {kernel_src}")
        else:
            print(f"ok   case={case} {opt} temps={temps} steps={steps}")
    print(f"== avx2_pool_fuzz: {args.cases - failures} passed, {failures} failed, {args.cases} total")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
