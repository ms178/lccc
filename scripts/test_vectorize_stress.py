#!/usr/bin/env python3
"""Self-tests for scripts/vectorize_stress.py (its oracle must itself be sound).

Two properties are pinned:

  1. The packed-SIMD census is operand-aware.  A real XOR / vector copy must be
     counted, zero idioms / register copies / scalar FP / GPR moves must not.
  2. Every generated reference program is DEFINED C: the signed accumulating
     kernels (`cond_inc`, `cmp_sub`, `cmp_acc_map`) run on the `full` data
     profile (INT_MAX / INT_MIN injected) and used to overflow.  They are
     compiled with `gcc -fsanitize=undefined -fno-sanitize-recover=all` and run.
"""
from __future__ import annotations

import importlib.util
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("vectorize_stress", HERE / "vectorize_stress.py")
vs = importlib.util.module_from_spec(_spec)
sys.modules["vectorize_stress"] = vs
_spec.loader.exec_module(vs)


class CensusClassifier(unittest.TestCase):
    COUNTED = [
        # real vector work -- including the cases the old heuristic dropped
        ("vpxor", "%ymm1,%ymm2,%ymm3"),  # a kernel's actual XOR
        ("pxor", "(%rax),%xmm0"),
        ("pxor", "%xmm1,%xmm0"),
        ("xorps", "%xmm1,%xmm0"),
        ("vxorpd", "(%rsi,%rax,8),%ymm1,%ymm1"),
        ("vmovdqu", "(%rsi,%rax,1),%ymm0"),  # packed copy: load ...
        ("vmovdqu", "%ymm0,(%rdi,%rax,1)"),  # ... and store
        ("movdqu", "0x10(%rsi),%xmm0"),
        ("movups", "%xmm0,-0x20(%rsp)"),
        ("vmovdqu8", "(%rsi),%zmm0{%k1}{z}"),
        ("vpabsd", "%xmm1,%xmm0"),  # ends in "sd" but is packed
        ("vpmaxsd", "%xmm1,%xmm0,%xmm2"),
        ("vpaddd", "%xmm1,%xmm0,%xmm0"),
        ("vaddpd", "%ymm1,%ymm0,%ymm0"),
        ("vcvtdq2ps", "%ymm0,%ymm0"),
        ("vbroadcastss", "(%rax),%ymm0"),
        ("vextracti128", "$0x1,%ymm0,%xmm1"),
        ("vfmadd231ps", "%ymm2,%ymm1,%ymm0"),
        ("vpcmpeqd", "%ymm1,%ymm0,%ymm2"),
        ("vpsubd", "%xmm2,%xmm1,%xmm0"),
        ("andps", "%xmm1,%xmm0"),
        ("vpmovmskb", "%ymm0,%eax"),
    ]
    NOT_COUNTED = [
        ("pxor", "%xmm0,%xmm0"),  # zeroing idiom
        ("vpxor", "%xmm1,%xmm1,%xmm1"),
        ("vxorps", "%ymm2,%ymm2,%ymm2"),
        ("vpcmpeqd", "%ymm0,%ymm0,%ymm0"),  # all-ones idiom
        ("vpsubd", "%xmm1,%xmm1,%xmm1"),
        ("movaps", "%xmm1,%xmm0"),  # register copy
        ("vmovdqa", "%ymm1,%ymm0"),
        ("vmovd", "%xmm0,%eax"),
        ("vmovq", "%rax,%xmm0"),
        ("movd", "%eax,%xmm0"),
        ("movss", "(%rax),%xmm0"),
        ("vmovsd", "%xmm0,(%rax)"),
        ("vaddsd", "%xmm1,%xmm0,%xmm0"),
        ("mulss", "%xmm1,%xmm0"),
        ("vsqrtsd", "%xmm1,%xmm1,%xmm0"),
        ("vucomisd", "%xmm1,%xmm0"),
        ("vcmpltsd", "%xmm1,%xmm0,%xmm2"),
        ("cvtsi2sd", "%eax,%xmm0"),
        ("vcvttsd2si", "%xmm0,%eax"),
        ("vcvtss2sd", "%xmm1,%xmm1,%xmm0"),
        ("vfmadd231sd", "%xmm2,%xmm1,%xmm0"),
        ("vzeroupper", ""),
        ("add", "%rax,%rdx"),
        ("mov", "(%rsi),%eax"),
    ]

    def test_counted(self):
        for mn, ops in self.COUNTED:
            self.assertTrue(vs.is_packed_simd(mn, ops), f"{mn} {ops} must count as SIMD")

    def test_not_counted(self):
        for mn, ops in self.NOT_COUNTED:
            self.assertFalse(vs.is_packed_simd(mn, ops), f"{mn} {ops} must not count")

    def test_split_operands_respects_memory_operands(self):
        self.assertEqual(
            vs.split_operands("(%rsi,%rax,4),%ymm1,%ymm1 # comment"),
            ["(%rsi,%rax,4)", "%ymm1", "%ymm1"],
        )

    def test_vector_copy_listing_is_not_scalar(self):
        listing = """
  401000:\tvmovdqu (%rsi,%rax,1),%ymm0
  401005:\tvmovdqu %ymm0,(%rdi,%rax,1)
  40100a:\tadd    $0x20,%rax
  40100e:\tcmp    %rdx,%rax
  401011:\tjne    401000
  401013:\tvzeroupper
"""
        self.assertEqual(vs.count_simd_in_disassembly(listing), 2)

    def test_vector_xor_listing_counts_the_xor(self):
        listing = """
  401000:\tvmovdqu (%rsi,%rax,1),%ymm0
  401005:\tvpxor  (%rdx,%rax,1),%ymm0,%ymm0
  40100a:\tvmovdqu %ymm0,(%rdi,%rax,1)
  40100f:\tvpxor  %xmm1,%xmm1,%xmm1
"""
        self.assertEqual(vs.count_simd_in_disassembly(listing), 3)

    def test_scalar_loop_listing_is_zero(self):
        listing = """
  401000:\tmovzbl (%rsi,%rax,1),%ecx
  401004:\tadd    %ecx,%edx
  401006:\tmovss  (%rdi),%xmm0
  40100a:\tvmovd  %xmm0,%eax
  40100e:\tpxor   %xmm0,%xmm0
  401012:\tcvtsi2sd %eax,%xmm1
"""
        self.assertEqual(vs.count_simd_in_disassembly(listing), 0)


@unittest.skipUnless(shutil.which("gcc"), "needs gcc")
class ReferenceProgramsAreDefinedC(unittest.TestCase):
    """The GCC -O0 arm is only an oracle if the program has no undefined behaviour."""

    # Kernels that perform signed arithmetic on `full`-profile data.
    KERNELS = ["cond_inc", "cmp_sub", "cmp_acc_map", "cmp_add2", "cmp_neg", "cmp_shl"]

    def test_signed_accumulating_kernels_have_no_ub(self):
        by_name = {k.name: k for k in vs.KERNELS}
        with tempfile.TemporaryDirectory() as d:
            for name in self.KERNELS:
                for tag in ("i8", "i16", "i32", "i64", "u32"):
                    elem = vs.BY_TAG[tag]
                    kern = by_name[name]
                    if not kern.types(elem):
                        continue
                    src = Path(d) / f"{name}_{tag}.c"
                    exe = Path(d) / f"{name}_{tag}"
                    src.write_text(vs.gen_program(kern, elem, "lt", "int"))
                    cc = subprocess.run(
                        ["gcc", "-O1", "-w", "-fsanitize=undefined", "-fno-sanitize-recover=all",
                         str(src), "-o", str(exe)],
                        capture_output=True, text=True)
                    self.assertEqual(cc.returncode, 0, cc.stderr[-400:])
                    run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=120)
                    self.assertEqual(run.returncode, 0, f"{name}_{tag}: UB: {run.stderr[-300:]}")

    def test_unsigned_twin_placeholder(self):
        self.assertEqual(vs.BY_TAG["i32"].unsigned_c, "uint32_t")
        self.assertEqual(vs.BY_TAG["i8"].unsigned_c, "uint8_t")
        self.assertEqual(vs.BY_TAG["u16"].unsigned_c, "uint16_t")
        self.assertEqual(vs.BY_TAG["i64"].unsigned_c, "uint64_t")


if __name__ == "__main__":
    unittest.main()
