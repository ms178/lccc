#!/usr/bin/env python3
"""Generate include/lcccsimd.h — the generic SIMD intrinsic layer.

Every entry is (gcc_name, mnemonic, kind, ...). The generator emits:
  1. `__lccc_simd*` builtin prototypes (exact arity per kind)
  2. `_mm*` wrapper functions calling the builtins (struct deref via
     __CCC_M*_FROM_BUILTIN)

Builtin arg convention: the FIRST argument is the instruction immediate
(0 for non-immediate ops); lowering drops it and re-appends it as the LAST
IR operand.

Regenerate: python3 scripts/gen_lcccsimd.py
"""
from pathlib import Path

REPO = Path(__file__).parent.parent
OUT = REPO / "include" / "lcccsimd.h"

# (name, mnemonic)
BIN3_512 = [
    ("dpbusd_epi32", "vpdpbusd512"), ("dpbusds_epi32", "vpdpbusds512"),
]

BIN512 = [
    ("add_epi32", "vpaddd512"), ("add_epi64", "vpaddq512"),
    ("sub_epi32", "vpsubd512"), ("sub_epi64", "vpsubq512"),
    ("add_epi8", "vpaddb512"), ("add_epi16", "vpaddw512"),
    ("sub_epi8", "vpsubb512"), ("sub_epi16", "vpsubw512"),
    ("sad_epu8", "vpsadbw512"),
    ("maddubs_epi16", "vpmaddubsw512"), ("madd_epi16", "vpmaddwd512"),
    ("mullo_epi16", "vpmullw512"), ("mulhi_epi16", "vpmulhw512"),
    ("mulhi_epu16", "vpmulhuw512"), ("mullo_epi32", "vpmulld512"),
    ("mul_epu32", "vpmuludq512"),
    ("max_epi8", "vpmaxsb512"), ("min_epi8", "vpminsb512"),
    ("max_epu8", "vpmaxub512"), ("min_epu8", "vpminub512"),
    ("max_epi16", "vpmaxsw512"), ("min_epi16", "vpminsw512"),
    ("max_epu16", "vpmaxuw512"), ("min_epu16", "vpminuw512"),
    ("max_epi32", "vpmaxsd512"), ("min_epi32", "vpminsd512"),
    ("max_epu32", "vpmaxud512"), ("min_epu32", "vpminud512"),
    ("max_epi64", "vpmaxsq512"), ("min_epi64", "vpminsq512"),
    ("max_epu64", "vpmaxuq512"), ("min_epu64", "vpminuq512"),
    ("cmpeq_epi8", "vpcmpeqb512"), ("cmpeq_epi16", "vpcmpeqw512"),
    ("cmpeq_epi32", "vpcmpeqd512"), ("cmpeq_epi64", "vpcmpeqq512"),
    ("cmpgt_epi8", "vpcmpgtb512"), ("cmpgt_epi16", "vpcmpgtw512"),
    ("cmpgt_epi32", "vpcmpgtd512"), ("cmpgt_epi64", "vpcmpgtq512"),
    ("xor_si512", "vpxorq512"), ("or_si512", "vporq512"),
    ("and_si512", "vpandq512"), ("andnot_si512", "vpandnq512"),
    ("shuffle_epi8", "vpshufb512"),
    ("unpacklo_epi8", "vpunpcklbw512"), ("unpackhi_epi8", "vpunpckhbw512"),
    ("unpacklo_epi16", "vpunpcklwd512"), ("unpackhi_epi16", "vpunpckhwd512"),
    ("unpacklo_epi32", "vpunpckldq512"), ("unpackhi_epi32", "vpunpckhdq512"),
    ("unpacklo_epi64", "vpunpcklqdq512"), ("unpackhi_epi64", "vpunpckhqdq512"),
    ("packs_epi16", "vpacksswb512"), ("packus_epi16", "vpackuswb512"),
    ("packs_epi32", "vpackssdw512"), ("packus_epi32", "vpackusdw512"),
    ("avg_epu8", "vpavgb512"), ("avg_epu16", "vpavgw512"),
    ("adds_epi8", "vpaddsb512"), ("adds_epi16", "vpaddsw512"),
    ("adds_epu8", "vpaddusb512"), ("adds_epu16", "vpaddusw512"),
    ("subs_epi8", "vpsubsb512"), ("subs_epi16", "vpsubsw512"),
    ("subs_epu8", "vpsubusb512"), ("subs_epu16", "vpsubusw512"),
    ("permutexvar_epi32", "vpermd512"),
    ("permutexvar_epi64", "vpermq_var512"),
]
UNI512 = [
    ("abs_epi8", "vpabsb512"), ("abs_epi16", "vpabsw512"),
    ("abs_epi32", "vpabsd512"), ("abs_epi64", "vpabsq512"),
    ("popcnt_epi8", "vpopcntb512"), ("popcnt_epi16", "vpopcntw512"),
    ("popcnt_epi32", "vpopcntd512"), ("popcnt_epi64", "vpopcntq512"),
    ("cvtepi8_epi16", "vpmovsxbw512"), ("cvtepi8_epi32", "vpmovsxbd512"),
    ("cvtepi8_epi64", "vpmovsxbq512"), ("cvtepi16_epi32", "vpmovsxwd512"),
    ("cvtepi16_epi64", "vpmovsxwq512"), ("cvtepi32_epi64", "vpmovsxdq512"),
    ("cvtepu8_epi16", "vpmovzxbw512"), ("cvtepu8_epi32", "vpmovzxbd512"),
    ("cvtepu8_epi64", "vpmovzxbq512"), ("cvtepu16_epi32", "vpmovzxwd512"),
    ("cvtepu16_epi64", "vpmovzxwq512"), ("cvtepu32_epi64", "vpmovzxdq512"),
]
BIN_IMM512 = [
    ("slli_epi16", "vpsllw512"), ("srli_epi16", "vpsrlw512"),
    ("srai_epi16", "vpsraw512"),
    ("slli_epi32", "vpslld512"), ("srli_epi32", "vpsrld512"),
    ("srai_epi32", "vpsrad512"),
    ("slli_epi64", "vpsllq512"), ("srli_epi64", "vpsrlq512"),
    ("srai_epi64", "vpsraq512"),
]
UNI_IMM512 = [
    ("shuffle_epi32", "vpshufd512"),
    ("shufflelo_epi16", "vpshuflw512"), ("shufflehi_epi16", "vpshufhw512"),
]
TRI_IMM512 = [
    ("ternarylogic_epi32", "vpternlogd512"),
    ("ternarylogic_epi64", "vpternlogq512"),
    ("alignr_epi8", "vpalignr512"),
    ("clmulepi64_epi128", "vpclmulqdq512"),
]
EXTRACT512 = [
    ("extracti32x4_epi32", "vextracti32x4_512", "__m128i", "M128I"),
    ("extracti64x2_epi64", "vextracti64x2_512", "__m128i", "M128I"),
    ("extracti32x8_epi32", "vextracti32x8_512", "__m256i", "M256I"),
    ("extracti64x4_epi64", "vextracti64x4_512", "__m256i", "M256I"),
]
INSERT512 = [
    ("inserti32x4", "vinserti32x4_512", "__m128i"),
    ("inserti64x2", "vinserti64x2_512", "__m128i"),
    ("inserti32x8", "vinserti32x8_512", "__m256i"),
    ("inserti64x4", "vinserti64x4_512", "__m256i"),
]
CMP_MASK = [
    # (name, mnemonic, mask type, has_imm)
    ("cmpeq_epu8_mask", "vpcmpub512", "__mmask64", False),
    ("cmp_epu8_mask", "vpcmpub512", "__mmask64", True),
    ("cmpeq_epi8_mask", "vpcmpb512", "__mmask64", False),
    ("cmp_epi8_mask", "vpcmpb512", "__mmask64", True),
    ("cmpeq_epu16_mask", "vpcmpuw512", "__mmask32", False),
    ("cmp_epu16_mask", "vpcmpuw512", "__mmask32", True),
    ("cmpeq_epi16_mask", "vpcmpw512", "__mmask32", False),
    ("cmp_epi16_mask", "vpcmpw512", "__mmask32", True),
    ("cmpeq_epu32_mask", "vpcmpud512", "__mmask16", False),
    ("cmp_epu32_mask", "vpcmpud512", "__mmask16", True),
    ("cmpeq_epi32_mask", "vpcmpd512", "__mmask16", False),
    ("cmp_epi32_mask", "vpcmpd512", "__mmask16", True),
    ("cmpeq_epu64_mask", "vpcmpuq512", "__mmask8", False),
    ("cmp_epu64_mask", "vpcmpuq512", "__mmask8", True),
    ("cmpeq_epi64_mask", "vpcmpq512", "__mmask8", False),
    ("cmp_epi64_mask", "vpcmpq512", "__mmask8", True),
    ("cmpeq_epu8_mask", "vpcmpub256", "__mmask32", False),
    ("cmp_epu8_mask", "vpcmpub256", "__mmask32", True),
    ("cmp_epi8_mask", "vpcmpb256", "__mmask32", True),
    ("cmpeq_epu8_mask", "vpcmpub128", "__mmask16", False),
    ("cmp_epu8_mask", "vpcmpub128", "__mmask16", True),
    ("cmp_epi8_mask", "vpcmpb128", "__mmask16", True),
]
MZ_LOAD = [
    ("maskz_loadu_epi8", "vmovdqu8_maskz_load512", "__m512i", "__mmask64", "M512I"),
    ("maskz_loadu_epi8", "vmovdqu8_maskz_load256", "__m256i", "__mmask32", "M256I"),
    ("maskz_loadu_epi8", "vmovdqu8_maskz_load128", "__m128i", "__mmask16", "M128I"),
]
M_LOAD = [
    ("mask_loadu_epi8", "vmovdqu8_mask_load512", "__m512i", "__mmask64", "M512I"),
    ("mask_loadu_epi8", "vmovdqu8_mask_load256", "__m256i", "__mmask32", "M256I"),
    ("mask_loadu_epi8", "vmovdqu8_mask_load128", "__m128i", "__mmask16", "M128I"),
]
M_STORE = [
    ("mask_storeu_epi8", "vmovdqu8_mask_store512", "__m512i", "__mmask64", "M512I"),
    ("mask_storeu_epi8", "vmovdqu8_mask_store256", "__m256i", "__mmask32", "M256I"),
    ("mask_storeu_epi8", "vmovdqu8_mask_store128", "__m128i", "__mmask16", "M128I"),
]
MZ_BIN = [
    ("maskz_maddubs_epi16", "vpmaddubsw_maskz512", "__m512i", "__mmask64", "M512I"),
]
MZ_SET1 = [
    ("maskz_set1_epi16", "vpbroadcastw_maskz512", "__m512i", "__mmask32", "int"),
    ("maskz_set1_epi32", "vpbroadcastd_maskz512", "__m512i", "__mmask16", "int"),
    ("maskz_set1_epi64", "vpbroadcastq_maskz512", "__m512i", "__mmask8", "long long"),
]
MZ_INSERT = [
    ("maskz_inserti64x2", "vinserti64x2_maskz512", "__m512i", "__mmask8", "__m128i"),
    ("maskz_inserti32x4", "vinserti32x4_maskz512", "__m512i", "__mmask16", "__m128i"),
]
MZ_EXTRACT = [
    ("maskz_extracti32x4_epi32", "vextracti32x4_maskz512", "__m128i", "__mmask16", "M128I"),
    ("maskz_extracti64x4_epi64", "vextracti64x4_maskz512", "__m256i", "__mmask8", "M256I"),
    ("maskz_extracti64x2_epi64", "vextracti64x2_maskz512", "__m128i", "__mmask8", "M128I"),
]
LOAD512 = [("loadu_si512", "vmovdqu64_load", "M512I")]
STORE512 = [("storeu_si512", "vmovdqu64_store", "M512I")]
SET1 = [
    ("set1_epi8", "vpbroadcastb512", "char"),
    ("set1_epi16", "vpbroadcastw512", "int"),
    ("set1_epi32", "vpbroadcastd512", "int"),
    ("set1_epi64", "vpbroadcastq512", "long long"),
]
CASTS = [
    ("castsi128_si512", "vcastsi128_512", "__m128i", "M512I"),
    ("zextsi128_si512", "vzextsi128_512", "__m128i", "M512I"),
    ("castsi512_si256", "vcastsi512_256", "__m256i", "M256I"),
    ("castsi256_si512", "vcastsi128_512", "__m256i", "M512I"),
]
REDUCE = [
    ("reduce_add_epu32", "vreduce_add_epu32_512", "unsigned int"),
    ("reduce_add_epi32", "vreduce_add_epu32_512", "int"),
    ("reduce_add_epu64", "vreduce_add_epu32_512", "unsigned long long"),
    ("reduce_add_epi64", "vreduce_add_epu32_512", "long long"),
]

# 128-bit FP: (name, mnemonic, kind) — kind b=bin u=unary s=scalar
FP128_BIN = [
    ("div_ps", "divps128"), ("min_ps", "minps128"), ("max_ps", "maxps128"),
    ("unpacklo_ps", "unpcklps128"), ("unpackhi_ps", "unpckhps128"),
    ("div_pd", "divpd128"), ("min_pd", "minpd128"), ("max_pd", "maxpd128"),
    ("unpacklo_pd", "unpcklpd128"), ("unpackhi_pd", "unpckhpd128"),
    ("hadd_ps", "haddps128"), ("hsub_ps", "hsubps128"), ("addsub_ps", "addsubps128"),
    ("hadd_pd", "haddpd128"), ("hsub_pd", "hsubpd128"), ("addsub_pd", "addsubpd128"),
]
FP128_UNI = [
    ("sqrt_ps", "sqrtps128"), ("rcp_ps", "rcpps128"), ("rsqrt_ps", "rsqrtps128"),
    ("sqrt_pd", "sqrtpd128"), ("movddup", "movddup128"),
    ("moveldup_ps", "movsldup128"), ("movehdup_ps", "movshdup128"),
    ("cvtps_epi32", "cvtps2dq128"), ("cvtepi32_ps", "cvtdq2ps128"),
    ("cvttps_epi32", "cvttps2dq128"), ("cvtps_pd", "cvtps2pd128"),
    ("cvtpd_ps", "cvtpd2ps128"), ("cvtpd_epi32", "cvtpd2dq128"),
    ("cvtepi32_pd", "cvtdq2pd128"), ("cvttpd_epi32", "cvttpd2dq128"),
]
FP128_SCALAR = [
    ("movemask_ps", "movmskps128", "int", "M128"),
    ("movemask_pd", "movmskpd128", "int", "M128D"),
    ("cvtss_si32", "cvtss2si128", "int", "M128"),
    ("cvtsd_si32", "cvtsd2si128", "int", "M128D"),
    ("cvttss_si32", "cvtss2si128", "int", "M128"),
    ("cvttsd_si32", "cvtsd2si128", "int", "M128D"),
]
FP128_UIMM = [
    ("round_ps", "roundps128", "__m128", "M128"),
    ("round_pd", "roundpd128", "__m128d", "M128D"),
]
FP128_BIMM = [
    ("blend_ps", "blendps128", "__m128", "M128"),
    ("blend_pd", "blendpd128", "__m128d", "M128D"),
    ("dp_ps", "dpps128", "__m128", "M128"),
    ("dp_pd", "dppd128", "__m128d", "M128D"),
    ("insert_ps", "insertps128", "__m128", "M128"),
]
FP128_CMP = [
    ("cmpeq_ps", "cmpps128", "__m128", "M128", 0), ("cmplt_ps", "cmpps128", "__m128", "M128", 1),
    ("cmple_ps", "cmpps128", "__m128", "M128", 2), ("cmpunord_ps", "cmpps128", "__m128", "M128", 3),
    ("cmpneq_ps", "cmpps128", "__m128", "M128", 4), ("cmpnlt_ps", "cmpps128", "__m128", "M128", 5),
    ("cmpnle_ps", "cmpps128", "__m128", "M128", 6), ("cmpord_ps", "cmpps128", "__m128", "M128", 7),
    ("cmpeq_pd", "cmppd128", "__m128d", "M128D", 0), ("cmplt_pd", "cmppd128", "__m128d", "M128D", 1),
    ("cmple_pd", "cmppd128", "__m128d", "M128D", 2), ("cmpunord_pd", "cmppd128", "__m128d", "M128D", 3),
    ("cmpneq_pd", "cmppd128", "__m128d", "M128D", 4), ("cmpnlt_pd", "cmppd128", "__m128d", "M128D", 5),
    ("cmpnle_pd", "cmppd128", "__m128d", "M128D", 6), ("cmpord_pd", "cmppd128", "__m128d", "M128D", 7),
]
FP128_SHUF = [
    ("shuffle_ps", "shufps128", "__m128", "M128"),
    ("shuffle_pd", "shufpd128", "__m128d", "M128D"),
]
FP128_BLENDV = [
    ("blendv_ps", "blendvps128", "__m128", "M128"),
    ("blendv_pd", "blendvpd128", "__m128d", "M128D"),
]
FP128_FMA = [
    ("fmadd_ps", "fmadd132ps128", "__m128", "M128"),
    ("fmadd_pd", "fmadd132pd128", "__m128d", "M128D"),
]
FP128_MOV = [
    ("move_ss", "movss128", "__m128", "M128"),
    ("move_sd", "movsd128", "__m128d", "M128D"),
]
FP128_CVT2 = [
    ("cvtss_sd", "cvtss2sd128", "__m128d", "M128D"),
    ("cvtsd_ss", "cvtsd2ss128", "__m128", "M128"),
]
FP128_SI2FP = [
    ("cvtsi32_ss", "cvtsi2ss128", "__m128", "M128"),
    ("cvtsi64_ss", "cvtsi2ss64_128", "__m128", "M128"),
    ("cvtsi32_sd", "cvtsi2sd128", "__m128d", "M128D"),
    ("cvtsi64_sd", "cvtsi2sd64_128", "__m128d", "M128D"),
]

FP256_BIN = [
    ("div_ps", "vdivps256"), ("min_ps", "vminps256"), ("max_ps", "vmaxps256"),
    ("unpacklo_ps", "vunpcklps256"), ("unpackhi_ps", "vunpckhps256"),
    ("div_pd", "vdivpd256"), ("min_pd", "vminpd256"), ("max_pd", "vmaxpd256"),
    ("unpacklo_pd", "vunpcklpd256"), ("unpackhi_pd", "vunpckhpd256"),
    ("hadd_ps", "vhaddps256"), ("hsub_ps", "vhsubps256"), ("addsub_ps", "vaddsubps256"),
]
FP256_UNI = [
    ("sqrt_ps", "vsqrtps256"), ("sqrt_pd", "vsqrtpd256"),
    ("cvtps_epi32", "vcvtps2dq256"), ("cvtepi32_ps", "vcvtdq2ps256"),
    ("cvttps_epi32", "vcvttps2dq256"), ("cvtps_pd", "vcvtps2pd256"),
    ("cvtpd_ps", "vcvtpd2ps256"), ("cvtpd_epi32", "vcvtpd2dq256"),
    ("cvtepi32_pd", "vcvtdq2pd256"), ("cvttpd_epi32", "vcvttpd2dq256"),
    ("permutevar_ps", "vpermilvarps256"),
    ("permutevar_pd", "vpermilvarpd256"),
]
FP256_SCALAR = [
    ("movemask_ps", "vmovmskps256", "int", "M256"),
    ("movemask_pd", "vmovmskpd256", "int", "M256D"),
    ("testz_ps", "vtestps256", "int", "M256"),
]
FP256_UIMM = [
    ("round_ps", "vroundps256", "__m256", "M256"),
    ("round_pd", "vroundpd256", "__m256d", "M256D"),
    ("permute_ps", "vpermilps256", "__m256", "M256"),
    ("permute_pd", "vpermilps256", "__m256d", "M256D"),
]
FP256_BIMM = [
    ("blend_ps", "vblendps256", "__m256", "M256"),
    ("blend_pd", "vblendpd256", "__m256d", "M256D"),
]
FP256_CMP = [
    ("cmpeq_ps", "vcmpps256", "__m256", "M256", 0), ("cmplt_ps", "vcmpps256", "__m256", "M256", 1),
    ("cmple_ps", "vcmpps256", "__m256", "M256", 2), ("cmpunord_ps", "vcmpps256", "__m256", "M256", 3),
    ("cmpneq_ps", "vcmpps256", "__m256", "M256", 4), ("cmpnlt_ps", "vcmpps256", "__m256", "M256", 5),
    ("cmpnle_ps", "vcmpps256", "__m256", "M256", 6), ("cmpord_ps", "vcmpps256", "__m256", "M256", 7),
    ("cmpeq_pd", "vcmppd256", "__m256d", "M256D", 0), ("cmplt_pd", "vcmppd256", "__m256d", "M256D", 1),
    ("cmple_pd", "vcmppd256", "__m256d", "M256D", 2), ("cmpunord_pd", "vcmppd256", "__m256d", "M256D", 3),
    ("cmpneq_pd", "vcmppd256", "__m256d", "M256D", 4), ("cmpnlt_pd", "vcmppd256", "__m256d", "M256D", 5),
    ("cmpnle_pd", "vcmppd256", "__m256d", "M256D", 6), ("cmpord_pd", "vcmppd256", "__m256d", "M256D", 7),
]
FP256_SHUF = [
    ("shuffle_ps", "vshufps256", "__m256", "M256"),
    ("shuffle_pd", "vshufpd256", "__m256d", "M256D"),
]
FP256_BLENDV = [
    ("blendv_ps", "vblendvps256", "__m256", "M256"),
    ("blendv_pd", "vblendvpd256", "__m256d", "M256D"),
]
FP256_FMA = [
    ("fmadd_ps", "vfmadd132ps256", "__m256", "M256"),
    ("fmadd_pd", "vfmadd132pd256", "__m256d", "M256D"),
]
FP256_MISC = [
    ("permute2f128_ps", "vperm2f128", "__m256", "M256"),
    ("permute2f128_pd", "vperm2f128", "__m256d", "M256D"),
    ("insertf128_ps", "vinsertf128", "__m256", "M256"),
    ("insertf128_pd", "vinsertf128", "__m256d", "M256D"),
    ("extractf128_ps", "vextractf128", "__m128", "M128"),
    ("extractf128_pd", "vextractf128", "__m128d", "M128D"),
]

# ------------------------------------------------------------- emit -------
L = []
w = L.append

w("/* Generated by scripts/gen_lcccsimd.py — DO NOT EDIT BY HAND. */")
w("#ifndef _LCCCSIMD_H_INCLUDED")
w("#define _LCCCSIMD_H_INCLUDED")
w("")
w("/* ---- vector types (single definition point for ALL widths) ---- */")
for align, elty, cnt, name in [
    (16, "float", 4, "__m128"), (16, "double", 2, "__m128d"),
    (16, "long long", 2, "__m128i"), (1, "long long", 2, "__m128i_u"),
    (1, "double", 2, "__m128d_u"),
    (32, "float", 8, "__m256"), (32, "double", 4, "__m256d"),
    (32, "long long", 4, "__m256i"), (1, "long long", 4, "__m256i_u"),
    (1, "double", 4, "__m256d_u"), (1, "float", 8, "__m256_u"),
    (64, "float", 16, "__m512"), (64, "double", 8, "__m512d"),
    (64, "long long", 8, "__m512i"), (1, "long long", 8, "__m512i_u"),
]:
    w(f"typedef struct __attribute__((__aligned__({align}))) {{")
    w(f"    {elty} __val[{cnt}];")
    w(f"}} {name};")
    w("")
w("/* AVX-512 opmask types */")
w("typedef unsigned char __mmask8;")
w("typedef unsigned short __mmask16;")
w("typedef unsigned int __mmask32;")
w("typedef unsigned long long __mmask64;")
w("")
w("/* GCC-compatible vector extension types used by system headers */")
for elty, cnt, name in [
    ("float", 4, "__v4sf"), ("double", 2, "__v2df"),
    ("long long", 2, "__v2di"), ("unsigned long long", 2, "__v2du"),
    ("int", 4, "__v4si"), ("unsigned int", 4, "__v4su"),
    ("short", 8, "__v8hi"), ("unsigned short", 8, "__v8hu"),
    ("char", 16, "__v16qi"), ("signed char", 16, "__v16qs"),
    ("unsigned char", 16, "__v16qu"),
    ("float", 8, "__v8sf"), ("double", 4, "__v4df"),
    ("long long", 4, "__v4di"), ("unsigned long long", 4, "__v4du"),
    ("int", 8, "__v8si"), ("unsigned int", 8, "__v8su"),
    ("short", 16, "__v16hi"), ("unsigned short", 16, "__v16hu"),
    ("char", 32, "__v32qi"), ("unsigned char", 32, "__v32qu"),
    ("float", 16, "__v16sf"), ("double", 8, "__v8df"),
    ("long long", 8, "__v8di"), ("unsigned long long", 8, "__v8du"),
    ("int", 16, "__v16si"), ("unsigned int", 16, "__v16su"),
    ("short", 32, "__v32hi"), ("unsigned short", 32, "__v32hu"),
    ("char", 64, "__v64qi"), ("unsigned char", 64, "__v64qu"),
]:
    w(f"typedef {elty} {name} __attribute__ ((__vector_size__ ({cnt * 4})));")
w("")
w("/*")
w(" * Generic SIMD builtin family: __lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}.")
w("/*")
w(" * Generic SIMD builtin family: __lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}.")
w(" * First builtin argument = instruction immediate (0 for non-immediate ops);")
w(" * the compiler drops it and re-appends it as the last IR operand.")
w(" *")
w(" * Values are struct-based (lccc model): wrappers dereference the builtin's")
w(" * result pointer. The backend keeps values in XMM/YMM/ZMM registers between")
w(" * consecutive intrinsics (vec_live_regs cache).")
w(" */")
w("")
w("#define __CCC_M128I_FROM_BUILTIN(e) (*(__m128i *)(e))")
w("#define __CCC_M128_FROM_BUILTIN(e)  (*(__m128 *)(e))")
w("#define __CCC_M128D_FROM_BUILTIN(e) (*(__m128d *)(e))")
w("#define __CCC_M256I_FROM_BUILTIN(e) (*(__m256i *)(e))")
w("#define __CCC_M256_FROM_BUILTIN(e)  (*(__m256 *)(e))")
w("#define __CCC_M256D_FROM_BUILTIN(e) (*(__m256d *)(e))")
w("#define __CCC_M512I_FROM_BUILTIN(e) (*(__m512i *)(e))")
w("#define __CCC_M512_FROM_BUILTIN(e)  (*(__m512 *)(e))")
w("#define __CCC_M512D_FROM_BUILTIN(e) (*(__m512d *)(e))")
w("")

ALWAYS = "__attribute__((__always_inline__, __artificial__))"

def proto(name, ret, params):
    rt = "void" if ret == "void" else f"__{ret}"
    w(f"{rt} __lccc_simd{name}({', '.join(params)});")

def wrap(decl, body):
    w(f"static __inline__ {decl} {ALWAYS}")
    w(body)
    w("")


def macro(name, body):
    """Emit a #define wrapper. Used for IMMEDIATE-taking intrinsics: macro
    parameters are substituted textually, so the immediate arrives at the
    builtin call as a literal (a function parameter would be a runtime value
    and the compiler could not fold it into the instruction)."""
    w(f"#define {name} {body}")
    w("")

def r(s):
    return s.replace("__lccc_simd512_i_", "lccc_simd512_i_") if False else s

# ---- 512-bit binary (2-arg) ----
for name, mn in BIN512:
    proto(f"512_i_{mn}", "m512i", ["long", "__m512i __a", "__m512i __b"])
for name, mn in BIN512:
    wrap(f"__m512i _mm512_{name}(__m512i __a, __m512i __b)",
         f"{{ return __CCC_M512I_FROM_BUILTIN(__lccc_simd512_i_{mn}(0, __a, __b)); }}")
# ---- 512-bit binary (3-arg: dpbusd family) ----
for name, mn in BIN3_512:
    proto(f"512_i_{mn}", "m512i", ["long", "__m512i __a", "__m512i __b", "__m512i __c"])
for name, mn in BIN3_512:
    wrap(f"__m512i _mm512_{name}(__m512i __a, __m512i __b, __m512i __c)",
         f"{{ return __CCC_M512I_FROM_BUILTIN(__lccc_simd512_i_{mn}(0, __a, __b, __c)); }}")

# ---- 512-bit unary ----
for name, mn in UNI512:
    proto(f"512_i_{mn}", "m512i", ["long", "__m512i __a"])
for name, mn in UNI512:
    wrap(f"__m512i _mm512_{name}(__m512i __a)",
         f"{{ return __CCC_M512I_FROM_BUILTIN(__lccc_simd512_i_{mn}(0, __a)); }}")
# ---- 512-bit binary+imm ----
for name, mn in BIN_IMM512:
    proto(f"512_i_{mn}", "m512i", ["long", "__m512i __a", "int __imm"])
for name, mn in BIN_IMM512:
    macro(f"_mm512_{name}(__a, __imm)",
          f"__CCC_M512I_FROM_BUILTIN(__lccc_simd512_i_{mn}(__imm, __a))")
# ---- 512-bit unary+imm ----
for name, mn in UNI_IMM512:
    proto(f"512_i_{mn}", "m512i", ["long", "__m512i __a", "int __imm"])
for name, mn in UNI_IMM512:
    macro(f"_mm512_{name}(__a, __imm)",
          f"__CCC_M512I_FROM_BUILTIN(__lccc_simd512_i_{mn}(__imm, __a))")
# ---- 128/256-bit ternary (AVX-512VL) ----
for wd, pfx, m in [(128, "", "M128I"), (256, "256", "M256I")]:
    for el in ["epi32", "epi64"]:
        mn = "vpternlogd" + str(wd)
        ty = "__m128i" if wd == 128 else "__m256i"
        proto(f"{wd}_i_{mn}", "m512i", ["long", ty + " __a", ty + " __b", ty + " __c", "int __imm"])
        macro(f"_mm{pfx}_ternarylogic_{el}(__a, __b, __c, __imm)",
              f"__CCC_{m}_FROM_BUILTIN(__lccc_simd{wd}_i_{mn}(__imm, __a, __b, __c))")

# ---- 512-bit ternary+imm ----
for name, mn in TRI_IMM512:
    is_tern = "ternarylogic" in name
    extra = ", __c" if is_tern else ""
    call = ", __a, __b, __c" if is_tern else ", __a, __b"
    proto(f"512_i_{mn}", "m512i", ["long", "__m512i __a", "__m512i __b"] +
          (["__m512i __c"] if is_tern else []) + ["int __imm"])
    macro(f"_mm512_{name}(__a, __b{extra}, __imm)",
          f"__CCC_M512I_FROM_BUILTIN(__lccc_simd512_i_{mn}(__imm{call}))")
# ---- extract ----
for name, mn, rty, m in EXTRACT512:
    proto(f"512_i_{mn}", "m512i", ["long", "__m512i __a", "int __imm"])
for name, mn, rty, m in EXTRACT512:
    macro(f"_mm512_{name}(__a, __imm)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd512_i_{mn}(__imm, __a))")
# ---- insert ----
for name, mn, srcty in INSERT512:
    proto(f"512_i_{mn}", "m512i", ["long", "__m512i __a", f"{srcty} __b", "int __imm"])
for name, mn, srcty in INSERT512:
    macro(f"_mm512_{name}(__a, __b, __imm)",
          f"__CCC_M512I_FROM_BUILTIN(__lccc_simd512_i_{mn}(__imm, __a, __b))")
# ---- mask compares ----
for name, mn, mty, has_imm in CMP_MASK:
    proto(f"512_i_{mn}", "m512i", ["long", "__m512i __a", "__m512i __b"])
for name, mn, mty, has_imm in CMP_MASK:
    pfx, vty = ("_mm512_", "__m512i") if mn.endswith("512") else (
        ("_mm256_", "__m256i") if mn.endswith("256") else ("_mm_", "__m128i"))
    if has_imm:
        macro(f"{pfx}{name}(__a, __b, __imm)",
              f"({mty})__lccc_simd512_i_{mn}(__imm, __a, __b)")
    else:
        wrap(f"{mty} {pfx}{name}({vty} __a, {vty} __b)",
             f"{{ return ({mty})__lccc_simd512_i_{mn}(0, __a, __b); }}")
# ---- masked loads/stores ----
for name, mn, vty, mty, m in MZ_LOAD:
    proto(f"512_i_{mn}", "m512i", ["long", mty + " __mask", "const void *__p"])
for name, mn, vty, mty, m in MZ_LOAD:
    pfx = "_mm512_" if mn.endswith("512") else ("_mm256_" if mn.endswith("256") else "_mm_")
    macro(f"{pfx}{name}(__mask, __p)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd512_i_{mn}(0, __mask, __p))")
for name, mn, vty, mty, m in M_LOAD:
    proto(f"512_i_{mn}", "m512i", ["long", mty + " __mask", "const void *__p", vty + " __old"])
for name, mn, vty, mty, m in M_LOAD:
    pfx = "_mm512_" if mn.endswith("512") else ("_mm256_" if mn.endswith("256") else "_mm_")
    macro(f"{pfx}{name}(__old, __mask, __p)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd512_i_{mn}(0, __mask, __p, __old))")
for name, mn, vty, mty, m in M_STORE:
    proto(f"512_i_{mn}", "void", ["long", "void *__p", mty + " __mask", vty + " __a"])
for name, mn, vty, mty, m in M_STORE:
    pfx = "_mm512_" if mn.endswith("512") else ("_mm256_" if mn.endswith("256") else "_mm_")
    macro(f"{pfx}{name}(__p, __mask, __a)",
          f"__lccc_simd512_i_{mn}(0, __p, __mask, __a)")
# ---- masked arithmetic ----
for name, mn, vty, mty, m in MZ_BIN:
    proto(f"512_i_{mn}", "m512i", ["long", mty + " __mask", "__m512i __a", "__m512i __b"])
for name, mn, vty, mty, m in MZ_BIN:
    macro(f"_mm512_{name}(__mask, __a, __b)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd512_i_{mn}(0, __mask, __a, __b))")
for name, mn, vty, mty, gty in MZ_SET1:
    proto(f"512_i_{mn}", "m512i", ["long", mty + " __mask", gty + " __a"])
for name, mn, vty, mty, gty in MZ_SET1:
    macro(f"_mm512_{name}(__mask, __a)",
          f"__CCC_M512I_FROM_BUILTIN(__lccc_simd512_i_{mn}(0, __mask, __a))")
for name, mn, vty, mty, srcty in MZ_INSERT:
    proto(f"512_i_{mn}", "m512i", ["long", mty + " __mask", "__m512i __a", srcty + " __b"])
for name, mn, vty, mty, srcty in MZ_INSERT:
    macro(f"_mm512_{name}(__mask, __a, __b, __imm)",
          f"__CCC_M512I_FROM_BUILTIN(__lccc_simd512_i_{mn}(__imm, __mask, __a, __b))")
for name, mn, rty, mty, m in MZ_EXTRACT:
    proto(f"512_i_{mn}", "m512i", ["long", mty + " __mask", "__m512i __a"])
for name, mn, rty, mty, m in MZ_EXTRACT:
    macro(f"_mm512_{name}(__mask, __a, __imm)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd512_i_{mn}(__imm, __mask, __a))")
# ---- loads/stores/set1/casts/reduce ----
for name, mn, m in LOAD512:
    proto(f"512_i_{mn}", "m512i", ["long", "const void *__p"])
for name, mn, m in LOAD512:
    wrap(f"__m512i _mm512_{name}(const void *__p)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd512_i_{mn}(0, __p)); }}")
for name, mn, m in STORE512:
    proto(f"512_i_{mn}", "void", ["long", "void *__p", "__m512i __a"])
for name, mn, m in STORE512:
    wrap(f"void _mm512_{name}(void *__p, __m512i __a)",
         f"{{ __lccc_simd512_i_{mn}(0, __p, __a); }}")
for name, mn, gty in SET1:
    proto(f"512_i_{mn}", "m512i", ["long", gty + " __a"])
for name, mn, gty in SET1:
    wrap(f"__m512i _mm512_{name}({gty} __a)",
         f"{{ return __CCC_M512I_FROM_BUILTIN(__lccc_simd512_i_{mn}(0, __a)); }}")
for name, mn, srcty, m in CASTS:
    proto(f"512_i_{mn}", "m512i", ["long", srcty + " __a"])
for name, mn, srcty, m in CASTS:
    wrap(f"__m512i _mm512_{name}({srcty} __a)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd512_i_{mn}(0, __a)); }}")
for name, mn, rty in REDUCE:
    proto(f"512_i_{mn}", "m512i", ["long", "__m512i __a"])
for name, mn, rty in REDUCE:
    wrap(f"{rty} _mm512_{name}(__m512i __a)",
         f"{{ return ({rty})__lccc_simd512_i_{mn}(0, __a); }}")

# ---- 128-bit FP ----
for name, mn in FP128_BIN:
    vty, m = ("__m128", "M128") if name.endswith("_ps") else ("__m128d", "M128D")
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __a", "__m128 __b"])
    wrap(f"{vty} _mm_{name}({vty} __a, {vty} __b)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}(0, __a, __b)); }}")
for name, mn in FP128_UNI:
    vty, m = ("__m128", "M128") if name.endswith("_ps") or name.startswith("rcp") or name.startswith("rsqrt") else ("__m128d", "M128D")
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __a"])
    wrap(f"{vty} _mm_{name}({vty} __a)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}(0, __a)); }}")
for name, mn, rty, m in FP128_SCALAR:
    vty = "__m128"
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __a"])
    wrap(f"{rty} _mm_{name}({vty} __a)",
         f"{{ return ({rty})__lccc_simd128_ps_{mn}(0, __a); }}")
for name, mn, rty, m in FP128_UIMM:
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __a", "int __imm"])
    macro(f"_mm_{name}(__a, __imm)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}(__imm, __a))")
for name, mn, rty, m in FP128_BIMM:
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __a", "__m128 __b", "int __imm"])
    macro(f"_mm_{name}(__a, __b, __imm)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}(__imm, __a, __b))")
# _mm_extract_ps(a, imm) -> int
proto("128_ps_extractps128", "m128", ["long", "__m128 __a", "int __imm"])
macro("_mm_extract_ps(__a, __imm)",
      "(int)__lccc_simd128_ps_extractps128(__imm, __a)")

for name, mn, rty, m, imm in FP128_CMP:
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __a", "__m128 __b"])
    macro(f"_mm_{name}(__a, __b)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}({imm}, __a, __b))")
# generic _mm_cmp_ps/_mm_cmp_pd(a, b, imm)
proto("128_ps_cmpps128", "m128", ["long", "__m128 __a", "__m128 __b"])
macro("_mm_cmp_ps(__a, __b, __imm)",
      "__CCC_M128_FROM_BUILTIN(__lccc_simd128_ps_cmpps128(__imm, __a, __b))")
macro("_mm_cmp_pd(__a, __b, __imm)",
      "__CCC_M128D_FROM_BUILTIN(__lccc_simd128_ps_cmppd128(__imm, __a, __b))")

for name, mn, rty, m in FP128_SHUF:
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __a", "__m128 __b", "int __imm"])
    macro(f"_mm_{name}(__a, __b, __imm)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}(__imm, __a, __b))")
for name, mn, rty, m in FP128_BLENDV:
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __mask", "__m128 __a", "__m128 __b"])
    wrap(f"{rty} _mm_{name}({rty} __a, {rty} __b, {rty} __mask)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}(0, __mask, __a, __b)); }}")
for name, mn, rty, m in FP128_FMA:
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __a", "__m128 __b", "__m128 __c"])
    wrap(f"{rty} _mm_{name}({rty} __a, {rty} __b, {rty} __c)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}(0, __a, __b, __c)); }}")
for name, mn, rty, m in FP128_MOV:
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __a", "__m128 __b"])
    wrap(f"{rty} _mm_{name}({rty} __a, {rty} __b)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}(0, __a, __b)); }}")
for name, mn, rty, m in FP128_CVT2:
    a_ty = "__m128" if name == "cvtss_sd" else "__m128d"
    b_ty = "__m128" if name == "cvtsd_ss" else "__m128"
    proto(f"128_ps_{mn}", "m128", ["long", a_ty + " __a", b_ty + " __b"])
    wrap(f"{rty} _mm_{name}({a_ty} __a, {b_ty} __b)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}(0, __a, __b)); }}")
for name, mn, rty, m in FP128_SI2FP:
    gty = "long long" if "64" in name else "int"
    proto(f"128_ps_{mn}", "m128", ["long", "__m128 __a", "int __i"])
    wrap(f"{rty} _mm_{name}({rty} __a, {gty} __i)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd128_ps_{mn}(0, __a, __i)); }}")

# ---- 256-bit FP ----
for name, mn in FP256_BIN:
    vty, m = ("__m256", "M256") if name.endswith("_ps") else ("__m256d", "M256D")
    proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a", "__m256 __b"])
    wrap(f"{vty} _mm256_{name}({vty} __a, {vty} __b)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(0, __a, __b)); }}")
FP256_VAR = [x for x in FP256_UNI if x[0].startswith("permutevar")]
FP256_UNI = [x for x in FP256_UNI if not x[0].startswith("permutevar")]
for name, mn in FP256_UNI:
    vty, m = ("__m256", "M256") if name.endswith("_ps") else ("__m256d", "M256D")
    proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a"])
    wrap(f"{vty} _mm256_{name}({vty} __a)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(0, __a)); }}")
for name, mn in FP256_VAR:
    vty, m = ("__m256", "M256") if name.endswith("_ps") else ("__m256d", "M256D")
    rty = "m256" if name.endswith("_ps") else "m256d"
    proto(f"256_ps_{mn}", rty, ["long", vty + " __a", "__m256i __b"])
    wrap(f"{vty} _mm256_{name}({vty} __a, __m256i __b)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(0, __a, __b)); }}")
for name, mn, rty, m in FP256_SCALAR:
    vty = "__m256"
    proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a"])
    wrap(f"{rty} _mm256_{name}({vty} __a)",
         f"{{ return ({rty})__lccc_simd256_ps_{mn}(0, __a); }}")
for name, mn, rty, m in FP256_UIMM:
    proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a", "int __imm"])
    macro(f"_mm256_{name}(__a, __imm)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(__imm, __a))")
for name, mn, rty, m in FP256_BIMM:
    proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a", "__m256 __b", "int __imm"])
    macro(f"_mm256_{name}(__a, __b, __imm)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(__imm, __a, __b))")
for name, mn, rty, m, imm in FP256_CMP:
    proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a", "__m256 __b"])
    macro(f"_mm256_{name}(__a, __b)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}({imm}, __a, __b))")
for name, mn, rty, m in FP256_SHUF:
    proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a", "__m256 __b", "int __imm"])
    macro(f"_mm256_{name}(__a, __b, __imm)",
          f"__CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(__imm, __a, __b))")
for name, mn, rty, m in FP256_BLENDV:
    proto(f"256_ps_{mn}", "m256", ["long", "__m256 __mask", "__m256 __a", "__m256 __b"])
    wrap(f"{rty} _mm256_{name}({rty} __a, {rty} __b, {rty} __mask)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(0, __mask, __a, __b)); }}")
for name, mn, rty, m in FP256_FMA:
    proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a", "__m256 __b", "__m256 __c"])
    wrap(f"{rty} _mm256_{name}({rty} __a, {rty} __b, {rty} __c)",
         f"{{ return __CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(0, __a, __b, __c)); }}")
for name, mn, rty, m in FP256_MISC:
    if name.startswith("extract"):
        proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a", "int __imm"])
        macro(f"_mm256_{name}(__a, __imm)",
              f"__CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(__imm, __a))")
    elif name.startswith("insert"):
        proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a", "__m128 __b", "int __imm"])
        macro(f"_mm256_{name}(__a, __b, __imm)",
              f"__CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(__imm, __a, __b))")
    else:
        proto(f"256_ps_{mn}", "m256", ["long", "__m256 __a", "__m256 __b", "int __imm"])
        macro(f"_mm256_{name}(__a, __b, __imm)",
              f"__CCC_{m}_FROM_BUILTIN(__lccc_simd256_ps_{mn}(__imm, __a, __b))")
# generic 256 cmp
proto("256_ps_vcmpps256", "m256", ["long", "__m256 __a", "__m256 __b"])
macro("_mm256_cmp_ps(__a, __b, __imm)",
      "__CCC_M256_FROM_BUILTIN(__lccc_simd256_ps_vcmpps256(__imm, __a, __b))")
macro("_mm256_cmp_pd(__a, __b, __imm)",
      "__CCC_M256D_FROM_BUILTIN(__lccc_simd256_ps_vcmppd256(__imm, __a, __b))")

w("#endif /* _LCCCSIMD_H_INCLUDED */")
OUT.write_text("\n".join(L) + "\n")
print(f"wrote {OUT} ({len(L)} lines)")
