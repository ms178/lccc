//! Generic SIMD intrinsic family: `__lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}`.
//!
//! The instruction mnemonic is part of the builtin NAME; this module is the
//! single audit point mapping mnemonic -> (emission class, IntrinsicOp).
//! Headers call these builtins for every intrinsic that must compile to a
//! real instruction (no scalar fallback loops).
//!
//! Emission classes:
//! - Vec128/Vec256/Vec512: vector result in a fresh stack slot (pointer
//!   returned; backend keeps the value in XMM/YMM/ZMM registers).
//! - Scalar: scalar result in a GPR dest (mask compares, movemask, extracts...).
//! - PtrStore: first arg is the destination pointer (masked stores, stores).

use super::lower::Lowerer;
use crate::common::types::IrType;
use crate::frontend::parser::ast::Expr;
use crate::frontend::sema::builtins::BuiltinIntrinsic;
use crate::ir::reexports::{Instruction, IntrinsicOp, IrConst, Operand};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LcccSimdClass {
    Vec128,
    Vec256,
    Vec512,
    Scalar,
    PtrStore,
}

/// mnemonic -> (class, IntrinsicOp)
/// Is this operand an EXACT compile-time integer immediate?
///
/// `operand_to_imm_i64` in the x86 backend has a catch-all arm that returns
/// `0` for floats, decimal containers, `I128` and any SSA `Operand::Value`.
/// Accepting one of those as an immediate would silently emit `$0` instead of
/// a diagnostic -- a wrong-code bug with no warning.  GCC 16.2 says "the last
/// argument must be an 8-bit immediate" and Clang 23.1.5 says "must be a
/// constant integer"; this predicate is the exact, target-independent gate.
/// `I128` is admitted ONLY when it fits in `i64`; no truncation is implied.
fn exact_immediate_operand(op: &Operand) -> bool {
    match op {
        Operand::Const(c) => c.to_i128().and_then(|v| i64::try_from(v).ok()).is_some(),
        Operand::Value(_) => false,
    }
}

pub(crate) fn lccc_simd_lookup(
    mnemonic: &str,
) -> Option<(LcccSimdClass, IntrinsicOp, bool, usize)> {
    use IntrinsicOp::*;
    use LcccSimdClass::*;
    // (class, op, has_imm, nargs): nargs = real operands after the leading
    // dummy/imm argument. The immediate (if any) arrives as the FIRST call
    // argument; lowering re-appends it as the LAST intrinsic operand so all
    // emitters use a uniform "imm = args.last()" rule.
    let b = |c, o| Some((c, o, false, 2usize));
    let u = |c, o| Some((c, o, false, 1usize));
    let ui = |c, o| Some((c, o, true, 1usize));
    let t = |c, o| Some((c, o, true, 2usize));
    let t3 = |c, o| Some((c, o, true, 3usize));
    let s2 = |c, o| Some((c, o, true, 2usize));
    let s1 = |c, o| Some((c, o, false, 1usize));
    let s1i = |c, o| Some((c, o, true, 1usize));
    let m2 = |c, o| Some((c, o, false, 2usize));
    let m4 = |c, o| Some((c, o, false, 4usize));
    let m3 = |c, o| Some((c, o, false, 3usize));
    let m3i = |c, o| Some((c, o, true, 3usize));
    let m2i = |c, o| Some((c, o, true, 2usize));
    let ps = |c, o| Some((c, o, false, 2usize));
    let ps3 = |c, o| Some((c, o, false, 3usize));
    match mnemonic {
        // ---- 512-bit packed integer binary ----
        "vpaddb512" => b(Vec512, Paddb512),
        "vpaddw512" => b(Vec512, Paddw512),
        "vpaddd512" => b(Vec512, Paddd512),
        "vpaddq512" => b(Vec512, Paddq512),
        "vpsubb512" => b(Vec512, Psubb512),
        "vpsubw512" => b(Vec512, Psubw512),
        "vpsubd512" => b(Vec512, Psubd512),
        "vpsubq512" => b(Vec512, Psubq512),
        "vpaddsb512" => b(Vec512, Paddsb512),
        "vpaddsw512" => b(Vec512, Paddsw512),
        "vpaddusb512" => b(Vec512, Paddusb512),
        "vpaddusw512" => b(Vec512, Paddusw512),
        "vpsubsb512" => b(Vec512, Psubsb512),
        "vpsubsw512" => b(Vec512, Psubsw512),
        "vpsubusb512" => b(Vec512, Psubusb512),
        "vpsubusw512" => b(Vec512, Psubusw512),
        "vpavgb512" => b(Vec512, Pavgb512),
        "vpavgw512" => b(Vec512, Pavgw512),
        "vpmaxub512" => b(Vec512, Pmaxub512),
        "vpminub512" => b(Vec512, Pminub512),
        "vpmaxuw512" => b(Vec512, Pmaxuw512),
        "vpminuw512" => b(Vec512, Pminuw512),
        "vpmaxsb512" => b(Vec512, Pmaxsb512),
        "vpminsb512" => b(Vec512, Pminsb512),
        "vpmaxsw512" => b(Vec512, Pmaxsw512),
        "vpminsw512" => b(Vec512, Pminsw512),
        "vpmaxsd512" => b(Vec512, Pmaxsd512),
        "vpminsd512" => b(Vec512, Pminsd512),
        "vpmaxud512" => b(Vec512, Pmaxud512),
        "vpminud512" => b(Vec512, Pminud512),
        "vpmaxsq512" => b(Vec512, Pmaxsq512),
        "vpminsq512" => b(Vec512, Pminsq512),
        "vpmaxuq512" => b(Vec512, Pmaxuq512),
        "vpminuq512" => b(Vec512, Pminuq512),
        "vpcmpeqd512" => b(Vec512, Pcmpeqd512),
        "vpcmpeqq512" => b(Vec512, Pcmpeqq512),
        "vpcmpgtb512" => b(Vec512, Pcmpgtb512),
        "vpcmpgtw512" => b(Vec512, Pcmpgtw512),
        "vpcmpgtd512" => b(Vec512, Pcmpgtd512),
        "vpcmpgtq512" => b(Vec512, Pcmpgtq512),
        "vpsadbw512" => b(Vec512, Psadbw512),
        "vpmaddubsw512" => b(Vec512, Pmaddubsw512),
        "vpmaddwd512" => b(Vec512, Pmaddwd512),
        "vpmullw512" => b(Vec512, Pmullw512),
        "vpmulhw512" => b(Vec512, Pmulhw512),
        "vpmulhuw512" => b(Vec512, Pmulhuw512),
        "vpmulld512" => b(Vec512, Pmulld512),
        "vpmuludq512" => b(Vec512, Pmuludq512),
        "vpxorq512" => b(Vec512, Pxor512),
        "vpxord512" => b(Vec512, Pxor512),
        "vporq512" => b(Vec512, Por512),
        "vpord512" => b(Vec512, Por512),
        "vpandq512" => b(Vec512, Pand512),
        "vpandd512" => b(Vec512, Pand512),
        "vpandnq512" => b(Vec512, Pandn512),
        "vpandnd512" => b(Vec512, Pandn512),
        "vpshufb512" => b(Vec512, Pshufb512),
        "vpabsb512" => u(Vec512, Pabsb512),
        "vpabsw512" => u(Vec512, Pabsw512),
        "vpabsd512" => u(Vec512, Pabsd512),
        "vpabsq512" => u(Vec512, Pabsq512),
        "vpunpcklbw512" => b(Vec512, Punpcklbw512),
        "vpunpcklwd512" => b(Vec512, Punpcklwd512),
        "vpunpckldq512" => b(Vec512, Punpckldq512),
        "vpunpcklqdq512" => b(Vec512, Punpcklqdq512),
        "vpunpckhbw512" => b(Vec512, Punpckhbw512),
        "vpunpckhwd512" => b(Vec512, Punpckhwd512),
        "vpunpckhdq512" => b(Vec512, Punpckhdq512),
        "vpunpckhqdq512" => b(Vec512, Punpckhqdq512),
        "vpacksswb512" => b(Vec512, Packsswb512),
        "vpackuswb512" => b(Vec512, Packuswb512),
        "vpackssdw512" => b(Vec512, Packssdw512),
        "vpackusdw512" => b(Vec512, Packusdw512),
        // ---- shifts/shuffles with immediate ----
        "vpsllw512" => ui(Vec512, Psllwi512),
        "vpsrlw512" => ui(Vec512, Psrlwi512),
        "vpsraw512" => ui(Vec512, Psrawi512),
        "vpslld512" => ui(Vec512, Psllidi512),
        "vpsrld512" => ui(Vec512, Psrlidi512),
        "vpsrad512" => ui(Vec512, Psradi512),
        "vpsllq512" => ui(Vec512, Psllqi512),
        "vpsrlq512" => ui(Vec512, Psrlqi512),
        "vpsraq512" => ui(Vec512, Psraqi512),
        "vpshufd512" => ui(Vec512, Pshufd512),
        "vpshuflw512" => ui(Vec512, Pshuflw512),
        "vpshufhw512" => ui(Vec512, Pshufhw512),
        "vpalignr512" => t(Vec512, Palignr512),
        "vpclmulqdq512" => t(Vec512, Vpclmulqdq512),
        // ---- sign/zero extension (unary, LL from dest) ----
        "vpmovzxbw512" => u(Vec512, Pmovzxbw512),
        "vpmovzxbd512" => u(Vec512, Pmovzxbd512),
        "vpmovzxbq512" => u(Vec512, Pmovzxbq512),
        "vpmovzxwd512" => u(Vec512, Pmovzxwd512),
        "vpmovzxwq512" => u(Vec512, Pmovzxwq512),
        "vpmovzxdq512" => u(Vec512, Pmovzxdq512),
        "vpmovsxbw512" => u(Vec512, Pmovsxbw512),
        "vpmovsxbd512" => u(Vec512, Pmovsxbd512),
        "vpmovsxbq512" => u(Vec512, Pmovsxbq512),
        "vpmovsxwd512" => u(Vec512, Pmovsxwd512),
        "vpmovsxwq512" => u(Vec512, Pmovsxwq512),
        "vpmovsxdq512" => u(Vec512, Pmovsxdq512),
        // ---- popcount ----
        "vpopcntb512" => u(Vec512, Popcntb512),
        "vpopcntw512" => u(Vec512, Popcntw512),
        "vpopcntd512" => u(Vec512, Popcntd512),
        "vpopcntq512" => u(Vec512, Popcntq512),
        // ---- ternary logic ----
        "vpternlogd512" => t3(Vec512, TernaryLogic512),
        "vpternlogq512" => t3(Vec512, TernaryLogic512),
        "vpternlogd256" => t3(Vec256, TernaryLogic256),
        "vpternlogq256" => t3(Vec256, TernaryLogic256),
        "vpternlogd128" => t3(Vec128, TernaryLogic128),
        "vpternlogq128" => t3(Vec128, TernaryLogic128),
        // ---- insert/extract ----
        "vinserti32x4_512" => t(Vec512, InsertI32x4),
        "vinserti64x2_512" => t(Vec512, InsertI64x2),
        "vinserti32x8_512" => t(Vec512, InsertI32x8),
        "vinserti64x4_512" => t(Vec512, InsertI64x4),
        "vextracti32x4_512" => ui(Vec128, ExtractI32x4),
        "vextracti64x2_512" => ui(Vec128, ExtractI64x2),
        "vextracti32x8_512" => ui(Vec256, ExtractI32x8),
        "vextracti64x4_512" => ui(Vec256, ExtractI64x4),
        // ---- permutes ----
        "vpermd512" => b(Vec512, PermutexvarEp32),
        "vpermps512" => b(Vec512, PermutexvarEp32),
        "vpermq_var512" => b(Vec512, PermutexvarEp64),
        "vpermi2d512" => b(Vec512, PermutexvarEp32),
        // ---- broadcast ----
        "vbroadcasti32x4_512" => u(Vec512, BroadcastI32x4),
        "vbroadcasti64x2_512" => u(Vec512, BroadcastI64x2),
        "vbroadcasti32x8_512" => u(Vec512, BroadcastI32x8),
        "vbroadcasti64x4_512" => u(Vec512, BroadcastI64x4),
        "vpbroadcastb512" => u(Vec512, SetEpi8_512),
        "vpbroadcastw512" => u(Vec512, SetEpi16_512),
        "vpbroadcastd512" => u(Vec512, SetEpi32_512),
        "vpbroadcastq512" => u(Vec512, SetEpi64x512),
        // ---- casts ----
        "vzextsi128_512" => u(Vec512, Zext128to512),
        "vcastsi512_256" => u(Vec256, Cast512to256),
        "vcastsi256_512" => u(Vec512, Cast256to512),
        "vcastsi128_512" => u(Vec512, Cast128to512),
        // ---- loads/stores ----
        "vmovdqu64_load" => u(Vec512, Loadu512),
        "vmovdqu64_store" => ps(PtrStore, Storeu512),
        // ---- mask compares (scalar mask in GPR) ----
        "vpcmpub512" => s2(Scalar, CmpeqEpu8Mask512),
        "vpcmpb512" => s2(Scalar, CmpEpi8Mask512),
        "vpcmpuw512" => s2(Scalar, CmpeqEpu16Mask512),
        "vpcmpw512" => s2(Scalar, CmpEpi16Mask512),
        "vpcmpud512" => s2(Scalar, CmpeqEpu32Mask512),
        "vpcmpd512" => s2(Scalar, CmpEpi32Mask512),
        "vpcmpuq512" => s2(Scalar, CmpeqEpu64Mask512),
        "vpcmpq512" => s2(Scalar, CmpEpi64Mask512),
        "vpcmpub256" => s2(Scalar, CmpeqEpu8Mask256),
        "vpcmpb256" => s2(Scalar, CmpEpi8Mask256),
        "vpcmpub128" => s2(Scalar, CmpeqEpu8Mask128),
        "vpcmpb128" => s2(Scalar, CmpEpi8Mask128),
        // AVX-512VL 128/256-bit word/dword/qword mask compares.  The IR ops,
        // the x86 emitter and `simd_mask_facts::mask_compare_shape` already
        // implemented every one of these; only the builtin dispatch was
        // missing, so 26 GCC 16.2 public names were unreachable and their
        // constant-image folds could never fire from C.  The `cmpeq_*` and
        // `cmp_*` public spellings share ONE mnemonic: the predicate reaches
        // the emitter as `args[2]` through the leading `long` immediate.
        "vpcmpuw256" => s2(Scalar, CmpEpu16Mask256),
        "vpcmpw256" => s2(Scalar, CmpEpi16Mask256),
        "vpcmpud256" => s2(Scalar, CmpEpu32Mask256),
        "vpcmpd256" => s2(Scalar, CmpEpi32Mask256),
        "vpcmpuq256" => s2(Scalar, CmpEpu64Mask256),
        "vpcmpq256" => s2(Scalar, CmpEpi64Mask256),
        "vpcmpuw128" => s2(Scalar, CmpEpu16Mask128),
        "vpcmpw128" => s2(Scalar, CmpEpi16Mask128),
        "vpcmpud128" => s2(Scalar, CmpEpu32Mask128),
        "vpcmpd128" => s2(Scalar, CmpEpi32Mask128),
        "vpcmpuq128" => s2(Scalar, CmpEpu64Mask128),
        "vpcmpq128" => s2(Scalar, CmpEpi64Mask128),
        // ---- masked loads/stores ----
        "vmovdqu8_maskz_load512" => m2(Vec512, MaskzLoaduEpi8_512),
        "vmovdqu8_maskz_load256" => m2(Vec256, MaskzLoaduEpi8_256),
        "vmovdqu8_maskz_load128" => m2(Vec128, MaskzLoaduEpi8_128),
        "vmovdqu8_mask_load512" => m3(Vec512, MaskLoaduEpi8_512),
        "vmovdqu8_mask_load256" => m3(Vec256, MaskLoaduEpi8_256),
        "vmovdqu8_mask_load128" => m3(Vec128, MaskLoaduEpi8_128),
        "vmovdqu8_mask_store512" => ps3(PtrStore, MaskStoreuEpi8_512),
        "vmovdqu8_mask_store256" => ps3(PtrStore, MaskStoreuEpi8_256),
        "vmovdqu8_mask_store128" => ps3(PtrStore, MaskStoreuEpi8_128),
        // ---- masked arithmetic ----
        "vpmaddubsw_maskz512" => m3(Vec512, MaskzMaddubsEpi16_512),
        "vpbroadcastw_maskz512" => m2(Vec512, MaskzSet1Epi16_512),
        "vpbroadcastd_maskz512" => m2(Vec512, MaskzSet1Epi32_512),
        "vpbroadcastq_maskz512" => m2(Vec512, MaskzSet1Epi64x512),
        "vinserti64x2_maskz512" => m3i(Vec512, MaskzInsertI64x2),
        "vinserti32x4_maskz512" => m3i(Vec512, MaskzInsertI32x4),
        "vextracti32x4_maskz512" => m2i(Vec128, MaskzExtractI32x4),
        "vextracti64x4_maskz512" => m2i(Vec256, MaskzExtractI64x4),
        "vextracti64x2_maskz512" => m2i(Vec128, MaskzExtractI64x2),
        // Arity must match the x86 emitter and `masked_form`: zeroing
        // reads [mask, a, b] (3), merge reads [mask, src, a, b] (4).
        // The merged tree declared m2/m3, so these two builtins produced
        // short argument lists that no emitter could consume. They were
        // unreachable from C only because no header prototype existed;
        // stage 5 adds the prototypes and fixes the arity together.
        "vpshufb_maskz128" => m3(Vec128, MaskzShuffleEpi8_128),
        "vpshufb_mask128" => m4(Vec128, MaskShuffleEpi8_128),
        "vreduce_add_epu32_512" => s1(Scalar, ReduceAddEpu32_512),
        // ---- AVX-VNNI 512 ----
        "vpdpbusd512" => m3(Vec512, Vpdpbusd512),
        "vpdpbusds512" => m3(Vec512, Vpdpbusds512),
        // ---- 512-bit FP ----
        "vaddps512" => b(Vec512, AddPs512),
        "vsubps512" => b(Vec512, SubPs512),
        "vmulps512" => b(Vec512, MulPs512),
        "vdivps512" => b(Vec512, DivPs512),
        "vminps512" => b(Vec512, MinPs512),
        "vmaxps512" => b(Vec512, MaxPs512),
        "vaddpd512" => b(Vec512, AddPd512),
        "vsubpd512" => b(Vec512, SubPd512),
        "vmulpd512" => b(Vec512, MulPd512),
        "vdivpd512" => b(Vec512, DivPd512),
        "vminpd512" => b(Vec512, MinPd512),
        "vmaxpd512" => b(Vec512, MaxPd512),
        "vsqrtps512" => u(Vec512, SqrtPs512),
        "vsqrtpd512" => u(Vec512, SqrtPd512),
        "vcmpps512" => s2(Vec512, CmpPs512),
        "vcmppd512" => s2(Vec512, CmpPd512),
        "vcvtps2pd512" => u(Vec512, CvtPs2Pd512),
        "vcvtpd2ps512" => u(Vec256, CvtPd2Ps512),
        "vcvtdq2ps512" => u(Vec512, CvtEp32_2Ps512),
        "vcvtps2dq512" => u(Vec512, CvtPs2Ep32_512),
        "vcvttps2dq512" => u(Vec512, CvttPs2Ep32_512),
        "vcvtdq2pd512" => u(Vec512, CvtEp32_2Pd512),
        "vcvtpd2dq512" => u(Vec256, CvtPd2Ep32_512),
        "vcvttpd2dq512" => u(Vec256, CvttPd2Ep32_512),
        "vfmadd132ps512" => m3(Vec512, FmaPs132v512),
        "vfmadd213ps512" => m3(Vec512, FmaPs213v512),
        "vfmadd231ps512" => m3(Vec512, FmaPs231v512),
        "vfmadd132pd512" => m3(Vec512, FmaPd132v512),
        "vfmadd213pd512" => m3(Vec512, FmaPd213v512),
        "vfmadd231pd512" => m3(Vec512, FmaPd231v512),
        // ---- 128-bit FP (SSE) ----
        "divps128" => b(Vec128, DivPs128),
        "minps128" => b(Vec128, MinPs128),
        "maxps128" => b(Vec128, MaxPs128),
        "sqrtps128" => u(Vec128, SqrtPs128),
        "rcpps128" => u(Vec128, RcpPs128),
        "rsqrtps128" => u(Vec128, RsqrtPs128),
        "andps128" => b(Vec128, AndPs128),
        "andnps128" => b(Vec128, AndnPs128),
        "orps128" => b(Vec128, OrPs128),
        "xorps128" => b(Vec128, XorPs128),
        "addss128" => b(Vec128, AddSs128),
        "subss128" => b(Vec128, SubSs128),
        "mulss128" => b(Vec128, MulSs128),
        "divss128" => b(Vec128, DivSs128),
        "minss128" => b(Vec128, MinSs128),
        "maxss128" => b(Vec128, MaxSs128),
        "sqrtss128" => u(Vec128, SqrtSs128),
        "rcpss128" => u(Vec128, RcpSs128),
        "rsqrtss128" => u(Vec128, RsqrtSs128),
        "cmpps128" => s2(Vec128, CmpPs128),
        "shufps128" => s2(Vec128, ShufPs128),
        "unpcklps128" => b(Vec128, UnpcklPs128),
        "unpckhps128" => b(Vec128, UnpckhPs128),
        "movmskps128" => s1(Scalar, MovemaskPs128),
        // Portable 16-byte sign-bit extraction.  `Pmovmskb128` is one of the
        // few vector instructions with a REAL lowering on all four backends
        // (x86-64 `pmovmskb`, i686 `pmovmskb`, AArch64 NEON reduction,
        // RISC-V software reduction), so it is exposed on every target by
        // `portable_simd_op`.  On x86 it is still SSE2-gated like its
        // siblings; on AArch64/RISC-V it is a target-neutral LCCC helper
        // with exactly defined semantics (bit i = sign bit of byte i).
        "pmovmskb128" => s1(Scalar, Pmovmskb128),
        "cvtps2dq128" => u(Vec128, CvtPs2Ep32_128),
        "cvtdq2ps128" => u(Vec128, CvtEp32ToPs128),
        "cvttps2dq128" => u(Vec128, CvttPs2Ep32_128),
        "cvtps2pd128" => u(Vec128, CvtPs2Pd128),
        "cvtpd2ps128" => u(Vec128, CvtPd2Ps128),
        "divpd128" => b(Vec128, DivPd128),
        "minpd128" => b(Vec128, MinPd128),
        "maxpd128" => b(Vec128, MaxPd128),
        "sqrtpd128" => u(Vec128, SqrtPd128),
        "andpd128" => b(Vec128, AndPd128),
        "andnpd128" => b(Vec128, AndnPd128),
        "orpd128" => b(Vec128, OrPd128),
        "xorpd128" => b(Vec128, XorPd128),
        "addsd128" => b(Vec128, AddSd128),
        "subsd128" => b(Vec128, SubSd128),
        "mulsd128" => b(Vec128, MulSd128),
        "divsd128" => b(Vec128, DivSd128),
        "minsd128" => b(Vec128, MinSd128),
        "maxsd128" => b(Vec128, MaxSd128),
        "sqrtsd128" => b(Vec128, SqrtSd128),
        "cmppd128" => s2(Vec128, CmpPd128),
        "shufpd128" => s2(Vec128, ShufPd128),
        "unpcklpd128" => b(Vec128, UnpcklPd128),
        "unpckhpd128" => b(Vec128, UnpckhPd128),
        "movmskpd128" => s1(Scalar, MovemaskPd128),
        "cvtpd2dq128" => u(Vec128, CvtPd2Ep32_128),
        "cvtdq2pd128" => u(Vec128, CvtEp32ToPd128),
        "cvttpd2dq128" => u(Vec128, CvttPd2Ep32_128),
        "movss128" => b(Vec128, Movss128),
        "movsd128" => b(Vec128, Movsd128),
        "cvtsi2ss128" => b(Vec128, CvtSi2Ss128),
        "cvtsi2sd128" => b(Vec128, CvtSi2Sd128),
        "cvtsi2ss64_128" => b(Vec128, CvtSi2Ss64_128),
        "cvtsi2sd64_128" => b(Vec128, CvtSi2Sd64_128),
        "cvtss2si128" => s1(Scalar, CvtSs2Si128),
        "cvtsd2si128" => s1(Scalar, CvtSd2Si128),
        "cvttss2si128" => s1(Scalar, CvttSs2Si128),
        "cvttsd2si128" => s1(Scalar, CvttSd2Si128),
        "cvtss2sd128" => b(Vec128, CvtSs2Sd128),
        "cvtsd2ss128" => b(Vec128, CvtSd2Ss128),
        "haddps128" => b(Vec128, HaddPs128),
        "hsubps128" => b(Vec128, HsubPs128),
        "addsubps128" => b(Vec128, AddsubPs128),
        "haddpd128" => b(Vec128, HaddPd128),
        "hsubpd128" => b(Vec128, HsubPd128),
        "addsubpd128" => b(Vec128, AddsubPd128),
        "movddup128" => u(Vec128, Movddup128),
        "movsldup128" => u(Vec128, Movsldup128),
        "movshdup128" => u(Vec128, Movshdup128),
        "roundps128" => ui(Vec128, RoundPs128),
        "roundpd128" => ui(Vec128, RoundPd128),
        "blendps128" => s2(Vec128, BlendPs128),
        "blendpd128" => s2(Vec128, BlendPd128),
        "blendvps128" => m3(Vec128, BlendvPs128),
        "blendvpd128" => m3(Vec128, BlendvPd128),
        "dpps128" => s2(Vec128, DpPs128),
        "dppd128" => s2(Vec128, DpPd128),
        "insertps128" => s2(Vec128, InsertPs128),
        "extractps128" => s1i(Scalar, ExtractPs128),
        "vpermilps128" => ui(Vec128, VpermilPs128),
        "fmadd132ps128" => m3(Vec128, FmaPs132),
        "fmadd213ps128" => m3(Vec128, FmaPs213),
        "fmadd231ps128" => m3(Vec128, FmaPs231),
        "fmadd132pd128" => m3(Vec128, FmaPd132),
        "fmadd213pd128" => m3(Vec128, FmaPd213),
        "fmadd231pd128" => m3(Vec128, FmaPd231),
        // ---- 256-bit FP (AVX) ----
        "vdivps256" => b(Vec256, DivPs256),
        "vminps256" => b(Vec256, MinPs256),
        "vmaxps256" => b(Vec256, MaxPs256),
        "vsqrtps256" => u(Vec256, SqrtPs256),
        "vcmpps256" => s2(Vec256, CmpPs256),
        "vshufps256" => s2(Vec256, ShufPs256),
        "vunpcklps256" => b(Vec256, UnpcklPs256),
        "vunpckhps256" => b(Vec256, UnpckhPs256),
        "vmovmskps256" => s1(Scalar, MovemaskPs256),
        "vcvtps2dq256" => u(Vec256, CvtPs2Ep32_256),
        "vcvtdq2ps256" => u(Vec256, CvtEp32ToPs256),
        "vcvttps2dq256" => u(Vec256, CvttPs2Ep32_256),
        "vcvtps2pd256" => u(Vec256, CvtPs2Pd256),
        "vcvtpd2ps256" => u(Vec128, CvtPd2Ps256),
        "vdivpd256" => b(Vec256, DivPd256),
        "vminpd256" => b(Vec256, MinPd256),
        "vmaxpd256" => b(Vec256, MaxPd256),
        "vsqrtpd256" => u(Vec256, SqrtPd256),
        "vcmppd256" => s2(Vec256, CmpPd256),
        "vshufpd256" => s2(Vec256, ShufPd256),
        "vunpcklpd256" => b(Vec256, UnpcklPd256),
        "vunpckhpd256" => b(Vec256, UnpckhPd256),
        "vmovmskpd256" => s1(Scalar, MovemaskPd256),
        "vcvtpd2dq256" => u(Vec128, CvtPd2Ep32_256),
        "vcvtdq2pd256" => u(Vec256, CvtEp32ToPd256),
        "vcvttpd2dq256" => u(Vec128, CvttPd2Ep32_256),
        "vpermilpd256" => ui(Vec256, VpermilPd256),
        "vpermilps256" => ui(Vec256, VpermilPs256),
        "vpermilvarps256" => b(Vec256, VpermilvarPs256),
        "vpermilvarpd256" => b(Vec256, VpermilvarPd256),
        "vperm2f128" | "vperm2f128ps" | "vperm2f128pd" => s2(Vec256, Vperm2f128),
        "vinsertf128" | "vinsertf128ps" | "vinsertf128pd" => s2(Vec256, Vinsertf128),
        "vextractf128" | "vextractf128ps" | "vextractf128pd" => ui(Vec128, Vextractf128),
        "vbroadcastss" => u(Vec256, Vbroadcastss),
        "vbroadcastsd" => u(Vec256, Vbroadcastsd),
        "vtestps256" => b(Scalar, TestzPs256),
        "vroundps256" => ui(Vec256, RoundPs256),
        "vroundpd256" => ui(Vec256, RoundPd256),
        "vblendps256" => s2(Vec256, BlendPs256),
        "vblendpd256" => s2(Vec256, BlendPd256),
        "vblendvps256" => m3(Vec256, BlendvPs256),
        "vblendvpd256" => m3(Vec256, BlendvPd256),
        "vhaddps256" => b(Vec256, HaddPs256),
        "vhsubps256" => b(Vec256, HsubPs256),
        "vaddsubps256" => b(Vec256, AddsubPs256),
        "vfmadd132ps256" => m3(Vec256, FmaPs132v256),
        "vfmadd213ps256" => m3(Vec256, FmaPs213v256),
        "vfmadd231ps256" => m3(Vec256, FmaPs231v256),
        "vfmadd132pd256" => m3(Vec256, FmaPd132v256),
        "vfmadd213pd256" => m3(Vec256, FmaPd213v256),
        "vfmadd231pd256" => m3(Vec256, FmaPd231v256),
        _ => None,
    }
}

/// Instructions that are implemented on ALL FOUR LCCC backends and may
/// therefore be exposed as target-neutral helpers on AArch64/RISC-V.
///
/// This is an explicit allowlist, NOT a relaxation of the foreign-builtin
/// contract: `check_foreign_intrinsics.py` still rejects every registered
/// `BuiltinIntrinsic::X86*` name on a non-x86 target.  These ops carry a
/// portable, target-independent semantic definition (bit i of the result is
/// the sign bit of byte i of the 16-byte input) and each has a measured
/// lowering in `src/backend/{x86,i686,arm,riscv}/codegen/intrinsics.rs`.
/// Anything outside this set keeps the historical diagnostic, so no x86-only
/// instruction can silently compile to zeros on another ISA.
fn portable_simd_op(op: IntrinsicOp) -> bool {
    matches!(op, IntrinsicOp::Pmovmskb128)
}

/// Legacy native helper permissions follow the encoded instruction, not the
/// result carrier: AVX conversions can return 128 bits, and 128-bit VPERMIL
/// is still AVX. EVEX helpers use per-instruction subset requirements; a
/// ZMM-width result is not itself evidence that AVX-512F is the only feature.
type SimdFeatureRequirement = (crate::common::x86_features::X86Feature, &'static str);

fn legacy_simd_feature(mnemonic: &str) -> Option<SimdFeatureRequirement> {
    use crate::common::x86_features::X86Feature as F;
    if mnemonic.ends_with("512") {
        return None;
    }
    let fma_stem = mnemonic.strip_prefix('v').unwrap_or(mnemonic);
    let is_fma3 = ["fmadd", "fmsub", "fnmadd", "fnmsub", "fmaddsub", "fmsubadd"]
        .iter()
        .any(|stem| fma_stem.starts_with(stem));
    Some(if is_fma3 {
        (F::Fma, "-mfma")
    } else if mnemonic.starts_with('v') {
        (F::Avx, "-mavx")
    } else {
        match mnemonic {
            "divps128" | "minps128" | "maxps128" | "sqrtps128" | "rcpps128" | "rsqrtps128"
            | "andps128" | "andnps128" | "orps128" | "xorps128" | "addss128" | "subss128"
            | "mulss128" | "divss128" | "minss128" | "maxss128" | "sqrtss128" | "rcpss128"
            | "rsqrtss128" | "cmpps128" | "shufps128" | "unpcklps128" | "unpckhps128"
            | "movmskps128" | "movss128" | "cvtsi2ss128" | "cvtsi2ss64_128" | "cvtss2si128"
            | "cvttss2si128" => (F::Sse, "-msse"),
            "haddps128" | "hsubps128" | "addsubps128" | "haddpd128" | "hsubpd128"
            | "addsubpd128" | "movddup128" | "movsldup128" | "movshdup128" => (F::Sse3, "-msse3"),
            "roundps128" | "roundpd128" | "blendps128" | "blendpd128" | "blendvps128"
            | "blendvpd128" | "dpps128" | "dppd128" | "insertps128" | "extractps128" => {
                (F::Sse41, "-msse4.1")
            }
            _ => (F::Sse2, "-msse2"),
        }
    })
}

/// Required subsets for the EVEX instructions actually exposed by this
/// builtin family. Keep this list instruction-based: AVX-512F does not imply
/// BW/DQ, and the 128/256-bit EVEX forms additionally require AVX512VL.
fn simd_feature_requirements(mnemonic: &str) -> Vec<SimdFeatureRequirement> {
    use crate::common::x86_features::X86Feature as F;
    let is_512 = mnemonic.ends_with("512") || mnemonic.ends_with("_512");
    let mut req = Vec::new();

    // EVERY `vpcmp*128`/`vpcmp*256` helper in this family is an EVEX
    // mask-compare encoding.  Before the AVX-512VL completion only the byte
    // forms existed here, and the evex predicate named just `vpcmpb`/`vpcmpub`;
    // a newly added `vpcmpd128` would therefore have fallen through to
    // `legacy_simd_feature`, which answers `F::Avx` for any `v*` mnemonic --
    // emitting an AVX-512VL instruction on a plain AVX target.  Recognising
    // the whole 128/256 family by width is the only fail-closed rule: the
    // 512 forms are already covered by `is_512`, and no legacy (non-EVEX)
    // `vpcmp*128/256` helper exists in `lccc_simd_lookup`.
    let mask_cmp_low =
        mnemonic.starts_with("vpcmp") && (mnemonic.ends_with("128") || mnemonic.ends_with("256"));

    let evex = is_512
        || mask_cmp_low
        || mnemonic.starts_with("vpternlog")
        || mnemonic.starts_with("vmovdqu8_mask")
        || matches!(
            mnemonic,
            "vpshufb_maskz128" | "vpshufb_mask128" | "vcastsi512_256"
                // The 512-bit unaligned load/store helpers are generated with
                // digit-less mnemonics (scripts/gen_lcccsimd.py LOAD512 /
                // STORE512), so `is_512` does not recognise them.  Before this
                // clause they fell through to `legacy_simd_feature` and were
                // classified as plain AVX, letting emit_evex_load_512 emit a
                // zmm move under `-mavx` (SA-03).  A mechanical sweep over all
                // 354 `lccc_simd_lookup` keys found exactly these two.
                | "vmovdqu64_load" | "vmovdqu64_store"
        );
    if evex {
        push_feature(&mut req, F::Avx512F, "-mavx512f");
    }

    // Byte/word integer operations are AVX512BW, including the explicit
    // low-width EVEX forms. The list follows the helper's encoded mnemonic,
    // not its C carrier or signedness suffix.
    let bw = [
        "vpaddb",
        "vpaddw",
        "vpsubb",
        "vpsubw",
        "vpaddsb",
        "vpaddsw",
        "vpaddusb",
        "vpaddusw",
        "vpsubsb",
        "vpsubsw",
        "vpsubusb",
        "vpsubusw",
        "vpavgb",
        "vpavgw",
        "vpmaxub",
        "vpminub",
        "vpmaxuw",
        "vpminuw",
        "vpmaxsb",
        "vpminsb",
        "vpmaxsw",
        "vpminsw",
        "vpsadbw",
        "vpmaddubsw",
        "vpmaddwd",
        "vpmullw",
        "vpmulhw",
        "vpmulhuw",
        "vpshufb",
        "vpabsb",
        "vpabsw",
        "vpunpcklbw",
        "vpunpcklwd",
        "vpunpckhbw",
        "vpunpckhwd",
        "vpacksswb",
        "vpackuswb",
        "vpackssdw",
        "vpackusdw",
        "vpsllw",
        "vpsrlw",
        "vpsraw",
        "vpalignr",
        "vpmovzxbw",
        "vpmovzxbd",
        "vpmovzxbq",
        "vpmovzxwd",
        "vpmovzxwq",
        "vpmovsxbw",
        "vpmovsxbd",
        "vpmovsxbq",
        "vpmovsxwd",
        "vpmovsxwq",
        "vpbroadcastb",
        "vpbroadcastw",
        "vpcmpb",
        "vpcmpub",
        "vpcmpw",
        "vpcmpuw",
        "vpcmpeqb",
        "vpcmpeqw",
        "vpcmpgtb",
        "vpcmpgtw",
        "vmovdqu8_mask",
        "vpshufb_mask",
    ];
    // Exact names, not stems: `vpshuflw`/`vpshufhw` also exist as legacy
    // VEX 128/256 forms that need only AVX, so a prefix rule would
    // over-constrain them.  GCC 16.2 puts `_mm512_shufflelo_epi16` and
    // `_mm512_shufflehi_epi16` in avx512bwintrin.h (SA-04).
    const BW_EXACT: &[&str] = &["vpshuflw512", "vpshufhw512"];
    if bw.iter().any(|stem| mnemonic.starts_with(stem)) || BW_EXACT.contains(&mnemonic) {
        push_feature(&mut req, F::Avx512Bw, "-mavx512bw");
    }

    // The 256-bit insert/extract halves of a zmm are AVX512DQ encodings, not
    // plain F: GCC 16.2 declares them in avx512dqintrin.h.  A GCC-16.2 header
    // sweep over the generated intrinsic wrappers found exactly these six
    // missing DQ requirements; the names are pinned here rather than matched
    // by prefix so unrelated `vinsert*`/`vextract*` helpers cannot inherit a
    // feature they do not need.
    const DQ_EXACT: &[&str] = &[
        "vextracti32x8_512",
        "vextracti64x2_512",
        "vextracti64x2_maskz512",
        "vinserti32x8_512",
        "vinserti64x2_512",
        "vinserti64x2_maskz512",
    ];
    if DQ_EXACT.contains(&mnemonic) {
        push_feature(&mut req, F::Avx512Dq, "-mavx512dq");
    }

    // The qword absolute/min/max operations are AVX512F (the `q` describes
    // lane width, not the AVX512DQ subset); only the six insert/extract
    // helpers above actually carry the DQ bit.  Any further DQ encoding must
    // be added to DQ_EXACT with a GCC/SDM citation, not inferred from a name.

    if mnemonic.starts_with("vpopcntb") || mnemonic.starts_with("vpopcntw") {
        push_feature(&mut req, F::Avx512BitAlg, "-mavx512bitalg");
    }
    if mnemonic.starts_with("vpopcntd") || mnemonic.starts_with("vpopcntq") {
        push_feature(&mut req, F::Avx512Vpopcntdq, "-mavx512vpopcntdq");
    }
    if mnemonic.starts_with("vpdpbusd") {
        push_feature(&mut req, F::Avx512Vnni, "-mavx512vnni");
    }
    if mnemonic.starts_with("vpternlog")
        || mnemonic.starts_with("vpcmp")
        || mnemonic.starts_with("vmovdqu8_mask")
        || matches!(mnemonic, "vpshufb_maskz128" | "vpshufb_mask128")
    {
        if !is_512 {
            push_feature(&mut req, F::Avx512Vl, "-mavx512vl");
        }
    }

    // These EVEX encodings use the VPCLMULQDQ extension in addition to F.
    if mnemonic == "vpclmulqdq512" {
        push_feature(&mut req, F::Vpclmulqdq, "-mvpclmulqdq");
    }

    if evex {
        req
    } else {
        legacy_simd_feature(mnemonic).into_iter().collect()
    }
}

fn push_feature(
    req: &mut Vec<SimdFeatureRequirement>,
    feature: crate::common::x86_features::X86Feature,
    flag: &'static str,
) {
    if !req.iter().any(|(seen, _)| *seen == feature) {
        req.push((feature, flag));
    }
}

/// The register-form map op a 256-bit FMA3 mnemonic denotes, with the lane
/// type and register width.
///
/// The three encodings of one family (`132`/`213`/`231`) differ ONLY in which
/// register the destructive FMA3 form overwrites; the algebra is always
/// `±(a·b) ± c` over the builtin's `[a, b, c]` operand order (Intel SDM
/// Vol. 2B, VFMADD132PS/VFMADD213PS/VFMADD231PS — the digit names the
/// destination role).  The register-form emitter re-encodes any of the four
/// sign structures into whichever 132/213/231 spelling fits the operands'
/// allocated homes (`emit_avx_map_fma_inner`, `fma_form_reencode`), so the
/// digit never survives into code selection — only the sign algebra does.
///
/// `None` for every other mnemonic, including the `fmaddsub`/`fmsubadd`
/// alternating families: those are NOT `a·b ± c` and must never be routed
/// through this mapping (the strict `<digits><ps|pd>256` tail parse rejects
/// them).
fn native_fma_256(mnemonic: &str) -> Option<(IntrinsicOp, IrType, usize)> {
    use IntrinsicOp::*;
    let stem = mnemonic.strip_prefix('v')?;
    // Order matters: `fnmsub`/`fnmadd` share the `f`+`n` prefix with `fmsub`.
    let (negative_product, negative_addend, rest) = if let Some(r) = stem.strip_prefix("fnmsub") {
        (true, true, r)
    } else if let Some(r) = stem.strip_prefix("fnmadd") {
        (true, false, r)
    } else if let Some(r) = stem.strip_prefix("fmsub") {
        (false, true, r)
    } else if let Some(r) = stem.strip_prefix("fmadd") {
        (false, false, r)
    } else {
        return None;
    };
    // `rest` is `<132|213|231><ps|pd>256`; anything else (an `addsub` tail, a
    // 128/512-bit width, a scalar form) has no 256-bit register-form twin.
    let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let digits_len = rest.len() - digits.len();
    if !matches!(digits_len, 3) || !matches!(rest.get(..3), Some("132" | "213" | "231")) {
        return None;
    }
    match digits {
        "ps256" => Some((
            VecMaddF32x8Signed(negative_product, negative_addend),
            IrType::F32,
            32,
        )),
        "pd256" => Some((
            VecMaddF64x4Signed(negative_product, negative_addend),
            IrType::F64,
            32,
        )),
        _ => None,
    }
}

/// What a pointer-form packed map op needs beyond its vector operands.
///
/// `Binary` covers the families whose only arguments are the two register
/// images.  The other two shapes exist because the register twins are NOT
/// drop-in renames of the pointer forms:
///
/// * a shift twin takes the count as an IMMEDIATE operand with a per-lane
///   bound (`psllw` defines 1..=15, `pslld` 1..=31, `psllq` 1..=63) and
///   asserts on anything outside it, while the C-level `_mm_slli_epi32(a, 0)`
///   is a legal identity and `_mm_slli_epi16(a, 40)` is a legal zeroing shift;
/// * a compare twin takes a PREDICATE immediate in the vectorizer's canonical
///   encoding, where signed greater-than is not a predicate at all.
#[derive(Clone, Copy)]
enum NativeMapTail {
    Binary,
    /// `dst = a << k`: second argument is an immediate in `1..=max`.
    ShiftImm {
        max: i64,
    },
    /// `dst = a ? b`: the twin takes the predicate immediate; `gt` is the
    /// signed less-than predicate with the operands swapped, because no x86
    /// encoding has a greater-than form for every lane width.
    Compare {
        predicate: i64,
        swap: bool,
    },
}

/// `CCC_NO_NATIVE_PACKED_MAP`: revert every routed map op to its pointer
/// form.  Read once — the lowering consults it per intrinsic.
fn packed_map_disabled() -> bool {
    static DISABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DISABLED.get_or_init(|| std::env::var_os("CCC_NO_NATIVE_PACKED_MAP").is_some())
}

/// Pointer-form packed map op -> its register-valued twin.
///
/// This is the RH-03 audit table: one row per BUILTIN-VISIBLE pointer op, the
/// twin the backend already implements and the SLP vectorizer already emits,
/// the twin's lane type and the register width.  A row is only present when
/// the twin exists in `src/ir/intrinsics.rs` AND the x86 backend emits a
/// single instruction for it; the pointer form stays for everything else
/// (word-high multiplies, variable-count shifts, byte shifts, `pmuludq`,
/// shuffles, extracts, and the three-operand blends, whose twin takes a third
/// vector operand that these tails do not model).
///
/// The lane type is the TWIN's lane, and it is what makes the table
/// self-checking: `Paddb` adds BYTES, so a wrong lane here is a wrong
/// arithmetic width that no instruction count can see (byte add and qword add
/// are both one instruction).  The operand C types are deliberately NOT part
/// of the key: `__m128i` is declared `long long __vector(2)` in
/// `include/lcccsimd.h`, so `_mm_add_epi8`'s arguments are i64x2 at the C
/// level while the operation is per-byte.  Requiring the declared lane type to
/// match would reject every real call, and inferring the lane type from it
/// would compute a different function.  Bitwise identity of the register image
/// (16 or 32 bytes) plus the twin's own lane type is the whole contract.
fn native_packed_map(op: IntrinsicOp) -> Option<(IntrinsicOp, IrType, usize, NativeMapTail)> {
    use IntrinsicOp::*;
    use NativeMapTail::Binary;
    let row = match op {
        // ---- 128-bit register images -------------------------------------
        Paddb128 => (VecAddI8x16, IrType::I8, 16, Binary),
        Paddw128 => (VecAddI16x8, IrType::I16, 16, Binary),
        Paddd128 => (VecAddI32x4, IrType::I32, 16, Binary),
        Paddq128 => (VecAddI64x2, IrType::I64, 16, Binary),
        Psubb128 => (VecSubI8x16, IrType::I8, 16, Binary),
        Psubw128 => (VecSubI16x8, IrType::I16, 16, Binary),
        Psubd128 => (VecSubI32x4, IrType::I32, 16, Binary),
        Psubq128 => (VecSubI64x2, IrType::I64, 16, Binary),
        // Bitwise: any lane width emits the identical encoding, so the table
        // names the dword twin (and the byte twin is equally valid); what
        // matters is that `pand`/`por`/`pxor` never route through an
        // ARITHMETIC twin, where the lane width would change the answer.
        Pand128 => (VecAndI32x4, IrType::I32, 16, Binary),
        Por128 => (VecOrI32x4, IrType::I32, 16, Binary),
        Pxor128 => (VecXorI32x4, IrType::I32, 16, Binary),
        // The 128-bit float bitwise forms are the same instructions; without
        // this they lose a home per operand and a home for the result.
        AndPs128 => (VecAndI32x4, IrType::I32, 16, Binary),
        OrPs128 => (VecOrI32x4, IrType::I32, 16, Binary),
        XorPs128 => (VecXorI32x4, IrType::I32, 16, Binary),
        Pmullw128 => (VecMulI16x8, IrType::I16, 16, Binary),
        Pmulld128 => (VecMulI32x4, IrType::I32, 16, Binary),
        Pmaxub128 => (VecMaxU8x16, IrType::U8, 16, Binary),
        Pminub128 => (VecMinU8x16, IrType::U8, 16, Binary),
        Pminsw128 => (VecMinI16x8, IrType::I16, 16, Binary),
        Pmaxsw128 => (VecMaxI16x8, IrType::I16, 16, Binary),
        // SSE4.1 extrema.  The dword twins are the vectorizer's own
        // (`VecSminI32x4`/`VecSmaxI32x4` are `pminsd`/`pmaxsd`; the unsigned
        // pair is `VecMinU32x4`/`VecMaxU32x4`), the byte pair needed a twin
        // of its own.  Lane type is the TWIN's, which is why unsigned dword
        // is `U32` even though the C argument is an i64x2 `__m128i`.
        Pminud128 => (VecMinU32x4, IrType::U32, 16, Binary),
        Pmaxud128 => (VecMaxU32x4, IrType::U32, 16, Binary),
        Pminsd128 => (VecSminI32x4, IrType::I32, 16, Binary),
        Pmaxsd128 => (VecSmaxI32x4, IrType::I32, 16, Binary),
        Pminsb128 => (VecMinI8x16, IrType::I8, 16, Binary),
        Pmaxsb128 => (VecMaxI8x16, IrType::I8, 16, Binary),
        // The six SSE2 singles that had no twin at all: one row each turns a
        // spilled pointer form (5 instructions) into the encoding itself (2).
        Pandn128 => (VecAndNotI32x4, IrType::I32, 16, Binary),
        Pavgb128 => (VecAvgU8x16, IrType::U8, 16, Binary),
        Pmulhuw128 => (VecMulHiU16x8, IrType::U16, 16, Binary),
        Paddsb128 => (VecAddSatI8x16, IrType::I8, 16, Binary),
        Packsswb128 => (VecPacksswbI16x8, IrType::I16, 16, Binary),
        Punpcklbw128 => (VecUnpackLoI8x16, IrType::I8, 16, Binary),
        // Unsigned 16-bit saturating subtract — the only SATURATING map op in
        // the register set.  It is also what makes the missing `pminuw` /
        // `pmaxuw` encodings expressible without one:
        // `min(a,b) = a - subs_epu(a,b)` and `max(a,b) = b + subs_epu(a,b)`
        // are exact for every lane (see include/smmintrin.h and
        // include/avx2intrin.h).
        Psubusw128 => (VecSubSatU16x8, IrType::I16, 16, Binary),

        // ---- 256-bit register images -------------------------------------
        Paddb256 => (VecAddI8x32, IrType::I8, 32, Binary),
        Paddw256 => (VecAddI16x16, IrType::I16, 32, Binary),
        Paddd256 => (VecAddI32x8, IrType::I32, 32, Binary),
        Paddq256 => (VecAddI64x4, IrType::I64, 32, Binary),
        Psubb256 => (VecSubI8x32, IrType::I8, 32, Binary),
        Psubw256 => (VecSubI16x16, IrType::I16, 32, Binary),
        Psubd256 => (VecSubI32x8, IrType::I32, 32, Binary),
        Psubq256 => (VecSubI64x4, IrType::I64, 32, Binary),
        Pand256 => (VecAndI32x8, IrType::I32, 32, Binary),
        Por256 => (VecOrI32x8, IrType::I32, 32, Binary),
        Pxor256 => (VecXorI32x8, IrType::I32, 32, Binary),
        Pmullw256 => (VecMulI16x16, IrType::I16, 32, Binary),
        Pmulld256 => (VecMulI32x8, IrType::I32, 32, Binary),
        Pmaxub256 => (VecMaxU8x32, IrType::U8, 32, Binary),
        Pminub256 => (VecMinU8x32, IrType::U8, 32, Binary),
        Pminsd256 => (VecMinI32x8, IrType::I32, 32, Binary),
        Pmaxsd256 => (VecMaxI32x8, IrType::I32, 32, Binary),
        Pminud256 => (VecMinU32x8, IrType::U32, 32, Binary),
        Pmaxud256 => (VecMaxU32x8, IrType::U32, 32, Binary),
        Pminsb256 => (VecMinI8x32, IrType::I8, 32, Binary),
        Pmaxsb256 => (VecMaxI8x32, IrType::I8, 32, Binary),
        Pandn256 => (VecAndNotI32x8, IrType::I32, 32, Binary),
        Punpcklbw256 => (VecUnpackLoI8x32, IrType::I8, 32, Binary),
        Psubusw256 => (VecSubSatU16x16, IrType::I16, 32, Binary),

        _ => return None,
    };
    Some(row)
}

/// Pointer-form immediate-shift op -> (twin, lane, bytes, largest defined
/// shift).  Variable-count forms (`_mm_sll_epi32` and friends) are absent on
/// purpose: their twin would have to materialise a count vector, which is not
/// a single instruction for every lane width.
fn native_packed_shift(op: IntrinsicOp) -> Option<(IntrinsicOp, IrType, usize, i64)> {
    use IntrinsicOp::*;
    Some(match op {
        Psllwi128 => (VecShlI16x8, IrType::I16, 16, 15),
        Psrlwi128 => (VecLShrI16x8, IrType::I16, 16, 15),
        Psrawi128 => (VecAShrI16x8, IrType::I16, 16, 15),
        Pslldi128 => (VecShlI32x4, IrType::I32, 16, 31),
        Psrldi128 => (VecLShrI32x4, IrType::I32, 16, 31),
        Psradi128 => (VecAShrI32x4, IrType::I32, 16, 31),
        Psllqi128 => (VecShlI64x2, IrType::I64, 16, 63),
        Psrlqi128 => (VecLShrI64x2, IrType::I64, 16, 63),
        Psllwi256 => (VecShlI16x16, IrType::I16, 32, 15),
        Psrlwi256 => (VecLShrI16x16, IrType::I16, 32, 15),
        Psrawi256 => (VecAShrI16x16, IrType::I16, 32, 15),
        Psllidi256 => (VecShlI32x8, IrType::I32, 32, 31),
        Psrlidi256 => (VecLShrI32x8, IrType::I32, 32, 31),
        Psradi256 => (VecAShrI32x8, IrType::I32, 32, 31),
        Psllqi256 => (VecShlI64x4, IrType::I64, 32, 63),
        Psrlqi256 => (VecLShrI64x4, IrType::I64, 32, 63),
        _ => return None,
    })
}

/// Pointer-form compare op -> (twin, lane, bytes, predicate, swap).
///
/// The predicate encoding is the vectorizer's (`emit_int_cmp`): 0 = equal,
/// 1 = signed less-than, 2 = signed less-or-equal, 4 = not-equal, 5/6 = the
/// unsigned forms.  `pcmpgtX` is the SIGNED greater-than of the pointer op,
/// which is exactly `lt` with the operands exchanged.
fn native_packed_cmp(op: IntrinsicOp) -> Option<(IntrinsicOp, IrType, usize, i64, bool)> {
    use IntrinsicOp::*;
    Some(match op {
        Pcmpeqb128 => (VecCmpI8x16, IrType::I8, 16, 0, false),
        Pcmpeqw128 => (VecCmpI16x8, IrType::I16, 16, 0, false),
        Pcmpeqd128 => (VecCmpI32x4, IrType::I32, 16, 0, false),
        Pcmpgtb128 => (VecCmpI8x16, IrType::I8, 16, 1, true),
        Pcmpgtw128 => (VecCmpI16x8, IrType::I16, 16, 1, true),
        Pcmpgtd128 => (VecCmpI32x4, IrType::I32, 16, 1, true),
        Pcmpeqb256 => (VecCmpI8x32, IrType::I8, 32, 0, false),
        Pcmpeqd256 => (VecCmpI32x8, IrType::I32, 32, 0, false),
        Pcmpeqq256 => (VecCmpI64x4, IrType::I64, 32, 0, false),
        Pcmpgtb256 => (VecCmpI8x32, IrType::I8, 32, 1, true),
        Pcmpgtd256 => (VecCmpI32x8, IrType::I32, 32, 1, true),
        Pcmpgtq256 => (VecCmpI64x4, IrType::I64, 32, 1, true),
        _ => return None,
    })
}

impl Lowerer {
    /// Keep packed FP helpers in the same register-valued IR as native vector
    /// arithmetic. The legacy address result remains an addressable object;
    /// whole-vector stores/loads let vec_ssa eliminate it instead of hiding
    /// the producer behind the pointer-result instruction family.
    fn try_native_packed_fp_helper(
        &mut self,
        op: IntrinsicOp,
        mnemonic: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        use IntrinsicOp::*;
        // The FMA3 map families have no memory-form IntrinsicOp at all: their
        // only encoding is the destructive register form, and the `m3` table
        // entry (`FmaPs132v256` & co.) exists purely to describe the operand
        // list.  Keyed on the mnemonic because one opcode names four sign
        // structures, and the sign structure — not the op — selects the op.
        let fma = native_fma_256(mnemonic);
        let (native, element, bytes) = match fma {
            Some(triple) => triple,
            None => match op {
                DivPs128 => (VecDivF32x4, IrType::F32, 16),
                MinPs128 => (VecMinF32x4, IrType::F32, 16),
                MaxPs128 => (VecMaxF32x4, IrType::F32, 16),
                SqrtPs128 => (VecSqrtF32x4, IrType::F32, 16),
                DivPd128 => (VecDivF64x2, IrType::F64, 16),
                MinPd128 => (VecMinF64x2, IrType::F64, 16),
                MaxPd128 => (VecMaxF64x2, IrType::F64, 16),
                SqrtPd128 => (VecSqrtF64x2, IrType::F64, 16),
                DivPs256 => (VecDivF32x8, IrType::F32, 32),
                MinPs256 => (VecMinF32x8, IrType::F32, 32),
                MaxPs256 => (VecMaxF32x8, IrType::F32, 32),
                SqrtPs256 => (VecSqrtF32x8, IrType::F32, 32),
                DivPd256 => (VecDivF64x4, IrType::F64, 32),
                MinPd256 => (VecMinF64x4, IrType::F64, 32),
                MaxPd256 => (VecMaxF64x4, IrType::F64, 32),
                SqrtPd256 => (VecSqrtF64x4, IrType::F64, 32),
                _ => return None,
            },
        };
        if self.target != crate::backend::Target::X86_64 {
            return None;
        }
        // The 128-bit double wrapper still crosses an I128 carrier at inline
        // returns. Until that bridge is native, this route adds spills (the
        // ordered FP fixture measures 74 -> 89 provider instructions). Keep
        // its established pointer emitter; wide doubles have no I128 bridge.
        if bytes == 16 && element == IrType::F64 {
            return None;
        }
        // Resolve the complete plan before evaluating any operand. Volatile,
        // foreign address spaces and mismatched types retain the legacy path.
        let types: Vec<_> = args.iter().map(|arg| self.expr_ctype(arg)).collect();
        let shape = super::vector_simd::simd_shape(types.first()?)?;
        if shape.bytes != bytes || shape.elem_ir != element {
            return None;
        }
        for (arg, ty) in args.iter().zip(&types) {
            let other = super::vector_simd::simd_shape(ty)?;
            if other.bytes != bytes
                || other.elem_ir != element
                || self.expr_access_is_volatile(arg)
                || self.aggregate_lvalue_space(arg) != crate::common::types::AddressSpace::Default
            {
                return None;
            }
        }
        let mut operands = Vec::with_capacity(args.len());
        for (arg, ty) in args.iter().zip(&types) {
            let ptr = self.vector_operand_ptr(arg, ty);
            operands.push(Operand::Value(self.emit_vec_load(&shape, ptr)));
        }
        let value = self.fresh_value();
        self.emit(Instruction::Intrinsic {
            dest: Some(value),
            op: native,
            dest_ptr: None,
            args: operands,
        });
        let result = self.emit_entry_alloca(IrType::Ptr, bytes, bytes, false);
        self.emit_vec_store(&shape, value, result);
        Some(Operand::Value(result))
    }

    /// Is `arg` a whole 16/32-byte register image that this rewrite may copy
    /// as ONE packed instruction?
    ///
    /// `simd_shape` is the same predicate the vector-extension lowering uses:
    /// dense lanes, no bit-packed `_Bool` vector, 16 or 32 bytes, and available
    /// on this target's vector lowering ceiling.  It deliberately accepts a
    /// FLOAT element type — `_mm_add_epi8` may be handed a `__m128` through an
    /// implicit conversion, and the bytewise sum of those bytes is what the
    /// builtin computes.  Volatile objects and foreign address spaces are
    /// refused: the `Vec*` families carry no volatility and address no segment,
    /// so those keep the pointer path that the legacy emitter provides.
    fn packed_map_operand_ok(&self, arg: &Expr, bytes: usize) -> bool {
        let ty = self.expr_ctype(arg);
        super::vector_simd::simd_shape(&ty).is_some_and(|shape| shape.bytes == bytes)
            && !self.expr_access_is_volatile(arg)
            && self.aggregate_lvalue_space(arg) == crate::common::types::AddressSpace::Default
    }

    /// Route a pointer-form packed integer/bitwise op through its
    /// register-valued twin (RH-03).
    ///
    /// The pointer forms take one ADDRESS per operand and write their result
    /// through a fourth, so every operand is a real frame slot even when the
    /// value already lives in `%xmm`/`%ymm`:
    ///
    /// ```text
    ///   movdqu %xmm1, 32(%rsp)          ; operand home
    ///   vpaddb 32(%rsp), %xmm0, %xmm0   ; the operation, on memory
    /// ```
    ///
    /// The twin does the same arithmetic on register values, and the entry
    /// alloca this returns is the same address-result convention the pointer
    /// form uses, so `vector_temp_promotion` deletes it exactly as it does for
    /// the FP helpers -- the caller's `(A)(...)` deref becomes a whole-vector
    /// read of the forwarded source.
    ///
    /// Refusal is total and silent: every shape the table does not own, every
    /// volatile/foreign-space operand, every shift count outside the
    /// instruction's defined range and every non-constant shift count keeps
    /// the established pointer path, which is always correct.
    pub(super) fn try_native_packed_map(
        &mut self,
        op: IntrinsicOp,
        args: &[Expr],
    ) -> Option<Operand> {
        // Resolve the whole plan BEFORE lowering anything.  Lowering is not
        // idempotent: an argument that emits IR (a call, a compound
        // assignment) must not be evaluated once for a plan that is then
        // abandoned and a second time by the pointer path.
        let (twin, lane, bytes, tail) = match native_packed_map(op) {
            Some((twin, lane, bytes, tail)) => (twin, lane, bytes, tail),
            None => match native_packed_shift(op) {
                Some((twin, lane, bytes, max)) => {
                    (twin, lane, bytes, NativeMapTail::ShiftImm { max })
                }
                None => {
                    let (twin, lane, bytes, predicate, swap) = native_packed_cmp(op)?;
                    (
                        twin,
                        lane,
                        bytes,
                        NativeMapTail::Compare { predicate, swap },
                    )
                }
            },
        };
        // Kill switch, in the same spirit as `CCC_NO_VLFOLD`: it returns to
        // the pointer form, which is semantically identical and merely slower.
        // It exists so the gate can prove itself non-vacuous against a real
        // pre-fix lowering instead of against a hand-edited budget.
        if packed_map_disabled() {
            return None;
        }
        // x86-64 only.  The i686 backend implements a SUBSET of the packed
        // families in their value form: `emit_intrinsic` asserts "only the
        // dest_ptr form is supported here" for the rest (`VecMinU8x16` is one
        // of them), so routing i686 through this table would turn a working
        // program into an internal compiler error.  32-bit keeps the pointer
        // path until those emitters grow the value form.
        if self.target != crate::backend::Target::X86_64 {
            return None;
        }
        if args.len() != 2 {
            return None;
        }
        // Both register operands, or the one plus an immediate.  The
        // register operand predicate is checked for EVERY operand before any
        // of them is lowered.
        if !self.packed_map_operand_ok(&args[0], bytes) {
            return None;
        }
        let immediate = match tail {
            NativeMapTail::Binary | NativeMapTail::Compare { .. } => {
                if !self.packed_map_operand_ok(&args[1], bytes) {
                    return None;
                }
                None
            }
            NativeMapTail::ShiftImm { max } => {
                // A constant count in the instruction's DEFINED range only:
                // zero is the identity and an over-wide count is defined to
                // shift everything out, and the twin's emitter asserts on
                // both.  Both are rare and the pointer path is exact for
                // them, so they are refused rather than special-cased.
                let count = self.eval_const_expr(&args[1])?.to_i64()?;
                if !(1..=max).contains(&count) {
                    return None;
                }
                Some(Operand::Const(IrConst::I64(count)))
            }
        };
        let lanes = bytes / lane.size();
        let fam = crate::passes::slp_vectorizer::family_for(lane, lanes)?;
        let shape = super::vector_simd::SimdShape {
            fam,
            elem_ir: lane,
            lanes,
            bytes,
        };

        // From here on the answer is committed: operand homes are loaded as
        // whole registers of the TWIN's lane type, which is a bitwise copy.
        let mut operands = Vec::with_capacity(3);
        for arg in args {
            if !self.expr_ctype(arg).is_vector() {
                continue;
            }
            let ty = self.expr_ctype(arg);
            let ptr = self.vector_operand_ptr(arg, &ty);
            operands.push(Operand::Value(self.emit_vec_load(&shape, ptr)));
        }
        match tail {
            NativeMapTail::Binary => {}
            NativeMapTail::ShiftImm { .. } => {
                operands.push(immediate?);
            }
            NativeMapTail::Compare { predicate, swap } => {
                if operands.len() != 2 {
                    return None;
                }
                if swap {
                    operands.swap(0, 1);
                }
                operands.push(Operand::Const(IrConst::I64(predicate)));
            }
        }
        let value = self.fresh_value();
        self.emit(Instruction::Intrinsic {
            dest: Some(value),
            op: twin,
            dest_ptr: None,
            args: operands,
        });
        let result = self.emit_entry_alloca(IrType::Ptr, bytes, bytes, false);
        self.emit_vec_store(&shape, value, result);
        Some(Operand::Value(result))
    }

    /// Lower a call to the generic SIMD family.
    /// Name format: `__lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}`.
    pub(super) fn lower_lccc_simd(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        let rest = name.strip_prefix("__lccc_simd")?;
        let (width_s, rest) = rest.split_once('_')?;
        let (_class_s, mnemonic) = rest.split_once('_')?;
        // Table keys are "<insn><width>" (e.g. vpaddd512); some special forms
        // carry their own suffix (e.g. vinserti32x4_512). Try both.
        let suffixed;
        let (canonical, (class, op, has_imm, nargs)) =
            if let Some(spec) = lccc_simd_lookup(mnemonic) {
                (mnemonic, spec)
            } else {
                suffixed = format!("{}{}", mnemonic, width_s);
                let Some(spec) = lccc_simd_lookup(&suffixed) else {
                    return self.reject_builtin_call(
                        format!("unknown SIMD helper instruction '{mnemonic}' in '{name}'"),
                        args,
                        64,
                    );
                };
                (suffixed.as_str(), spec)
            };
        // The table, not a caller-supplied prototype, owns this contract.
        // Never silently truncate extra operands or hand too few to codegen.
        if args.len() != nargs + 1 {
            return self.reject_builtin_call(
                format!(
                    "'{name}' requires {} arguments including the leading control argument, but {} were provided",
                    nargs + 1,
                    args.len()
                ),
                args,
                64,
            );
        }
        let width = match class {
            LcccSimdClass::Vec128 => 16usize,
            LcccSimdClass::Vec256 => 32,
            LcccSimdClass::Vec512 => 64,
            _ => 0,
        };
        let x86_target = matches!(
            self.target,
            crate::backend::Target::X86_64 | crate::backend::Target::I686
        );
        if !x86_target && !portable_simd_op(op) {
            return self.reject_builtin_call(
                format!("'{name}' is not available on {}", self.target.triple()),
                args,
                width.max(16),
            );
        }
        // x86 ISA feature permissions are meaningless on AArch64/RISC-V: the
        // portable subset has no x86 encoding to gate.  Keeping the checks
        // x86-only also prevents `check_x86_simd_feature` from diagnosing a
        // missing `-msse2` on a target that has no SSE2 at all.
        if x86_target {
            let feature_requirements = simd_feature_requirements(canonical);
            let uses_avx512 = feature_requirements.iter().any(|(feature, _)| {
                matches!(
                    feature,
                    crate::common::x86_features::X86Feature::Avx512F
                        | crate::common::x86_features::X86Feature::Avx512Bw
                        | crate::common::x86_features::X86Feature::Avx512Dq
                        | crate::common::x86_features::X86Feature::Avx512Vl
                        | crate::common::x86_features::X86Feature::Avx512Vnni
                        | crate::common::x86_features::X86Feature::Avx512BitAlg
                        | crate::common::x86_features::X86Feature::Avx512Vpopcntdq
                        | crate::common::x86_features::X86Feature::Avx512Cd
                        | crate::common::x86_features::X86Feature::Avx512Ifma
                        | crate::common::x86_features::X86Feature::Avx512Vbmi
                        | crate::common::x86_features::X86Feature::Avx512Vbmi2
                        | crate::common::x86_features::X86Feature::Avx512Bf16
                        | crate::common::x86_features::X86Feature::Avx512Fp16
                        | crate::common::x86_features::X86Feature::Avx512Bmm
                        | crate::common::x86_features::X86Feature::Avx512Vp2intersect
                        | crate::common::x86_features::X86Feature::Avx512Er
                        | crate::common::x86_features::X86Feature::Avx512Pf
                )
            });
            if self.target == crate::backend::Target::I686 && uses_avx512 {
                return self.reject_builtin_call(
                    format!("'{name}' uses an AVX-512 encoding not implemented for i686"),
                    args,
                    width.max(16),
                );
            }
            for (feature, flag) in feature_requirements {
                self.check_x86_simd_feature(feature, flag, name, args);
            }
        }
        if let Some(result) = self.try_native_packed_fp_helper(op, canonical, &args[1..]) {
            return Some(result);
        }
        // A compile-time immediate is a HARD REQUIREMENT, not a preference.
        // GCC 16.2 says "the last argument must be an 8-bit immediate" and
        // Clang 23.1.5 says "argument ... must be a constant integer"; the
        // x86 backend's `operand_to_imm_i64` silently returns 0 for an SSA
        // `Operand::Value`, so a runtime `vpcmp` predicate used to compile to
        // an EQ compare with NO diagnostic -- a silent miscompile.  Validate
        // the immediate BEFORE evaluating any other operand, so a rejected
        // call cannot leave half-lowered expressions behind.  Constant
        // expressions and enum constants already lower to `Operand::Const`,
        // so only genuinely runtime values are refused.
        let imm_op = if has_imm {
            let op = self.lower_expr(&args[0]);
            // `Operand::Const` alone is not enough: a float, a decimal
            // container or an out-of-i64-range I128 also reaches
            // `operand_to_imm_i64`, whose catch-all arm would silently emit
            // `$0`.  Require an EXACT integer immediate in the i64 domain.
            if !exact_immediate_operand(&op) {
                return self.reject_builtin_call(
                    format!("the last argument to '{name}' must be a constant integer immediate"),
                    args,
                    width.max(16),
                );
            }
            Some(op)
        } else {
            None
        };
        // Drop the leading dummy/imm argument; keep exactly nargs real
        // operands; re-append the immediate (if any) as the LAST operand.
        let mut arg_ops: Vec<Operand> = args
            .iter()
            .enumerate()
            .skip(1)
            .take(nargs)
            .map(|(index, arg)| self.lower_typed_x86_builtin_argument(name, arg, index))
            .collect();
        if let Some(op) = imm_op {
            arg_ops.push(op);
        }
        match class {
            LcccSimdClass::Vec128 | LcccSimdClass::Vec256 | LcccSimdClass::Vec512 => {
                let result_alloca = self.fresh_value();
                // Natural alignment for the vector width (Raptor Lake prefers
                // aligned vmovdqa / store-forwarding on 16/32-byte slots).
                let align = match width {
                    64 => 64usize,
                    32 => 32usize,
                    _ => 16usize,
                };
                self.emit(Instruction::Alloca {
                    dest: result_alloca,
                    ty: IrType::Ptr,
                    size: width,
                    align,
                    volatile: false,
                    semantic_volatile: false,
                });
                let dest_val = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(dest_val),
                    op,
                    dest_ptr: Some(result_alloca),
                    args: arg_ops,
                });
                Some(Operand::Value(result_alloca))
            }
            LcccSimdClass::Scalar => {
                let dest_val = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(dest_val),
                    op,
                    dest_ptr: None,
                    args: arg_ops,
                });
                Some(Operand::Value(dest_val))
            }
            LcccSimdClass::PtrStore => {
                if arg_ops.is_empty() {
                    return Some(Operand::Const(IrConst::I64(0)));
                }
                let ptr_val = self.operand_to_value(arg_ops[0].clone());
                self.emit(Instruction::Intrinsic {
                    dest: None,
                    op,
                    dest_ptr: Some(ptr_val),
                    args: arg_ops[1..].to_vec(),
                });
                Some(Operand::Const(IrConst::I64(0)))
            }
        }
    }
}

// Keep the enum import used (documentation of classes).
#[allow(dead_code)]
fn _class_doc(c: LcccSimdClass) -> &'static str {
    match c {
        LcccSimdClass::Vec128 => "16-byte vector result",
        LcccSimdClass::Vec256 => "32-byte vector result",
        LcccSimdClass::Vec512 => "64-byte vector result",
        LcccSimdClass::Scalar => "scalar GPR result",
        LcccSimdClass::PtrStore => "store through dest pointer",
    }
}

#[cfg(test)]
mod legacy_permission_tests {
    use super::legacy_simd_feature;
    use crate::common::x86_features::X86Feature as F;

    #[test]
    fn every_generated_helper_prototype_agrees_with_operand_metadata() {
        let mut checked = 0;
        let mut unmapped = std::collections::BTreeSet::new();
        for line in include_str!("../../../include/lcccsimd.h").lines() {
            let line = line.trim();
            if !line.ends_with(");") || line.contains('{') {
                continue;
            }
            let Some((_, rest)) = line.split_once("__lccc_simd") else {
                continue;
            };
            let (name, parameters) = rest.split_once('(').expect("helper prototype");
            let mut fields = name.splitn(3, '_');
            let width = fields.next().expect("width");
            let _kind = fields.next().expect("kind");
            let mnemonic = fields.next().expect("mnemonic");
            let suffixed = format!("{mnemonic}{width}");
            let canonical = if super::lccc_simd_lookup(mnemonic).is_some() {
                mnemonic
            } else {
                suffixed.as_str()
            };
            let Some((_, _, _, operands)) = super::lccc_simd_lookup(canonical) else {
                unmapped.insert(name);
                continue;
            };
            let features = super::simd_feature_requirements(canonical);
            assert!(
                !features.is_empty(),
                "{name} has no ISA permission metadata"
            );
            assert_eq!(parameters.split(',').count(), operands + 1, "{name}");
            checked += 1;
        }
        assert!(checked > 250, "vacuous helper audit: {checked}");
        // Explicit ratchet, not a blanket skip: these historical vector-result
        // AVX512 declarations have no lowering. Valid calls now diagnose;
        // SIMD-TYPE-1 records their removal or supported-contract replacement.
        assert_eq!(
            unmapped,
            std::collections::BTreeSet::from(["512_i_vpcmpeqb512", "512_i_vpcmpeqw512",])
        );
    }

    #[test]
    fn avx512_permissions_are_subset_and_encoding_specific() {
        use crate::common::x86_features::X86Feature as F;
        let cases: &[(&str, &[F])] = &[
            ("vpcmpub128", &[F::Avx512F, F::Avx512Bw, F::Avx512Vl]),
            // AVX-512VL completion: word forms need BW, dword/qword forms do
            // NOT, and every 128/256 EVEX form needs VL. A regression that
            // fell back to `legacy_simd_feature` would answer only `F::Avx`.
            ("vpcmpuw128", &[F::Avx512F, F::Avx512Bw, F::Avx512Vl]),
            ("vpcmpw256", &[F::Avx512F, F::Avx512Bw, F::Avx512Vl]),
            ("vpcmpud128", &[F::Avx512F, F::Avx512Vl]),
            ("vpcmpd256", &[F::Avx512F, F::Avx512Vl]),
            ("vpcmpuq128", &[F::Avx512F, F::Avx512Vl]),
            ("vpcmpq256", &[F::Avx512F, F::Avx512Vl]),
            ("vpcmpud512", &[F::Avx512F]),
            ("vpternlogq256", &[F::Avx512F, F::Avx512Vl]),
            ("vpternlogd512", &[F::Avx512F]),
            (
                "vmovdqu8_maskz_load256",
                &[F::Avx512F, F::Avx512Bw, F::Avx512Vl],
            ),
            ("vmovdqu8_mask_store512", &[F::Avx512F, F::Avx512Bw]),
            ("vpmaxsq512", &[F::Avx512F]),
            ("vpabsq512", &[F::Avx512F]),
            ("vpcmpeqb512", &[F::Avx512F, F::Avx512Bw]),
            ("vpcmpeqw512", &[F::Avx512F, F::Avx512Bw]),
            ("vpcmpgtb512", &[F::Avx512F, F::Avx512Bw]),
            ("vpcmpgtw512", &[F::Avx512F, F::Avx512Bw]),
            ("vpopcntw512", &[F::Avx512F, F::Avx512BitAlg]),
            ("vpopcntq512", &[F::Avx512F, F::Avx512Vpopcntdq]),
            ("vpdpbusd512", &[F::Avx512F, F::Avx512Vnni]),
            ("vfmadd231ps512", &[F::Avx512F]),
            ("vcastsi512_256", &[F::Avx512F]),
            // SA-03: digit-less EVEX loads/stores must not answer F::Avx.
            ("vmovdqu64_load", &[F::Avx512F]),
            ("vmovdqu64_store", &[F::Avx512F]),
            // SA-04: GCC 16.2 avx512bwintrin.h owns the shuffle twins.
            ("vpshuflw512", &[F::Avx512F, F::Avx512Bw]),
            ("vpshufhw512", &[F::Avx512F, F::Avx512Bw]),
            // GCC 16.2 avx512dqintrin.h owns the zmm half insert/extract
            // forms; the mechanical header sweep found exactly six.
            ("vextracti32x8_512", &[F::Avx512F, F::Avx512Dq]),
            ("vextracti64x2_512", &[F::Avx512F, F::Avx512Dq]),
            ("vextracti64x2_maskz512", &[F::Avx512F, F::Avx512Dq]),
            ("vinserti32x8_512", &[F::Avx512F, F::Avx512Dq]),
            ("vinserti64x2_512", &[F::Avx512F, F::Avx512Dq]),
            ("vinserti64x2_maskz512", &[F::Avx512F, F::Avx512Dq]),
            ("vfmadd231ps256", &[F::Fma]),
            ("vfmsub231ps256", &[F::Fma]),
            ("vfnmadd231pd256", &[F::Fma]),
            ("vfnmsub231pd256", &[F::Fma]),
            ("vfmaddsub231ps256", &[F::Fma]),
            ("vfmsubadd231ps256", &[F::Fma]),
            ("vcvtpd2ps256", &[F::Avx]),
        ];
        for (mnemonic, want) in cases {
            let got: Vec<_> = super::simd_feature_requirements(mnemonic)
                .into_iter()
                .map(|(feature, _)| feature)
                .collect();
            assert_eq!(got.as_slice(), *want, "{mnemonic}");
        }
    }

    #[test]
    fn source_instruction_not_result_width_owns_permission() {
        for (mnemonic, feature) in [
            ("minps128", F::Sse),
            ("cvtpd2ps128", F::Sse2),
            ("haddps128", F::Sse3),
            ("roundps128", F::Sse41),
            ("vcvtpd2ps256", F::Avx),
            ("vextractf128", F::Avx),
            ("vpermilps128", F::Avx),
            ("fmadd132ps128", F::Fma),
        ] {
            assert_eq!(legacy_simd_feature(mnemonic).unwrap().0, feature);
        }
        assert!(legacy_simd_feature("vcvtpd2ps512").is_none());
    }
}

#[cfg(test)]
mod portable_subset_tests {
    use super::{LcccSimdClass, lccc_simd_lookup, portable_simd_op, simd_feature_requirements};
    use crate::ir::reexports::IntrinsicOp as O;

    /// The allowlist is ONE instruction wide by design.  Adding a second op
    /// requires a new proof that all four backends implement it; a silent
    /// widening of the gate would let an x86-only encoding through to
    /// AArch64/RISC-V as a no-op or a wrong answer.
    #[test]
    fn pmovmskb128_is_the_only_portable_helper_and_is_exactly_scalar() {
        assert_eq!(
            lccc_simd_lookup("pmovmskb128"),
            Some((LcccSimdClass::Scalar, O::Pmovmskb128, false, 1))
        );
        assert!(portable_simd_op(O::Pmovmskb128));
        // The x86 ISA permission is still required on x86-64/i686.
        assert!(!simd_feature_requirements("pmovmskb128").is_empty());
        // Near misses: same semantic family, different widths or encodings,
        // plus a memory-writing blend.  None has a four-ISA lowering.
        for op in [
            O::MovemaskPs128,
            O::MovemaskPd128,
            O::MovemaskPs256,
            O::Pmovmskb256,
            O::BlendvPs128,
            O::Setzero128,
            O::Pand128,
            O::VecCmpI8x16,
        ] {
            assert!(!portable_simd_op(op), "{op:?} must stay x86-only");
        }
    }

    /// `portable_simd_op` is a PURE predicate over the op, so the target gate
    /// cannot be widened by a target-dependent branch.  Probe a batch of
    /// unrelated ops to make the "one instruction wide" claim non-vacuous.
    #[test]
    fn portable_allowlist_is_fail_closed_for_unrelated_intrinsics() {
        for op in [
            O::Aesenc128,
            O::Crc32_64,
            O::SqrtF64,
            O::Pause,
            O::VecLoadI32x4,
            O::VecStoreI32x4,
            O::LoadF64x2,
            O::Storeldi128,
        ] {
            assert!(!portable_simd_op(op), "{op:?} must stay x86-only");
        }
    }
}

#[cfg(test)]
mod immediate_diagnostic_tests {
    use super::exact_immediate_operand;
    use crate::ir::reexports::{IrConst as C, Operand, Value};

    /// GCC 16.2 / Clang 23.1.5 hard-error on a runtime `vpcmp` predicate;
    /// LCCC used to emit `$0` with no diagnostic.  The gate must accept every
    /// EXACT integer immediate and refuse every value that the x86 backend's
    /// `operand_to_imm_i64` would silently turn into zero.
    #[test]
    fn exact_immediate_operand_accepts_only_exact_integers() {
        for c in [
            C::I8(-1),
            C::I8(7),
            C::I16(-128),
            C::I16(32767),
            C::I32(i32::MIN),
            C::I32(i32::MAX),
            C::I64(i64::MIN),
            C::I64(i64::MAX),
            C::I128(i64::MIN as i128),
            C::I128(i64::MAX as i128),
            C::Zero,
        ] {
            assert!(
                exact_immediate_operand(&Operand::Const(c)),
                "{c:?} is an exact i64 immediate"
            );
        }
        for c in [
            C::I128(i128::from(i64::MAX) + 1),
            C::I128(i128::from(i64::MIN) - 1),
            C::F32(0.0),
            C::F64(0.0),
            C::D32(0),
            C::D64(0),
            C::LongDouble(0.0, [0u8; 16]),
        ] {
            assert!(
                !exact_immediate_operand(&Operand::Const(c)),
                "{c:?} must NOT become a silent $0"
            );
        }
        assert!(!exact_immediate_operand(&Operand::Value(Value(0))));
        assert!(!exact_immediate_operand(&Operand::Value(Value(u32::MAX))));
    }

    /// Every lookup row that declares `has_imm` must have its leading control
    /// argument checked.  A row added later without the flag would bypass the
    /// diagnostic; pin the mask-compare predicate family explicitly.
    #[test]
    fn mask_compare_predicates_are_declared_as_immediates() {
        use super::lccc_simd_lookup;
        for mnemonic in [
            "vpcmpb128",
            "vpcmpub256",
            "vpcmpw128",
            "vpcmpuw256",
            "vpcmpd128",
            "vpcmpud256",
            "vpcmpq128",
            "vpcmpuq256",
            "vpcmpb512",
            "vpcmpub512",
            "vpcmpw512",
            "vpcmpuw512",
            "vpcmpd512",
            "vpcmpud512",
            "vpcmpq512",
            "vpcmpuq512",
        ] {
            let (_, _, has_imm, nargs) = lccc_simd_lookup(mnemonic)
                .unwrap_or_else(|| panic!("{mnemonic} must be in the dispatch table"));
            assert!(has_imm, "{mnemonic} predicate must be a checked immediate");
            assert_eq!(nargs, 2, "{mnemonic} takes exactly a and b");
        }
    }
}
