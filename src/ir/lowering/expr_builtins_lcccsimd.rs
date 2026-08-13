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

use crate::frontend::parser::ast::Expr;
use crate::frontend::sema::builtins::BuiltinIntrinsic;
use crate::ir::reexports::{Instruction, IntrinsicOp, IrConst, Operand};
use crate::common::types::IrType;
use super::lower::Lowerer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LcccSimdClass {
    Vec128,
    Vec256,
    Vec512,
    Scalar,
    PtrStore,
}

/// mnemonic -> (class, IntrinsicOp)
pub(super) fn lccc_simd_lookup(
    mnemonic: &str,
) -> Option<(LcccSimdClass, IntrinsicOp, bool, usize)> {
    use LcccSimdClass::*;
    use IntrinsicOp::*;
    // (class, op, has_imm, nargs): nargs = real operands after the leading
    // dummy/imm argument. The immediate (if any) arrives as the FIRST call
    // argument; lowering re-appends it as the LAST intrinsic operand so all
    // emitters use a uniform "imm = args.last()" rule.
    let b = |c, o| Some((c, o, false, 2usize));
    let u = |c, o| Some((c, o, false, 1usize));
    let bi = |c, o| Some((c, o, true, 2usize));
    let ui = |c, o| Some((c, o, true, 1usize));
    let t = |c, o| Some((c, o, true, 2usize));
    let t3 = |c, o| Some((c, o, true, 3usize));
    let s2 = |c, o| Some((c, o, true, 2usize));
    let s1 = |c, o| Some((c, o, false, 1usize));
    let s1i = |c, o| Some((c, o, true, 1usize));
    let m2 = |c, o| Some((c, o, false, 2usize));
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
        "vpsllw512" => bi(Vec512, Psllwi512),
        "vpsrlw512" => bi(Vec512, Psrlwi512),
        "vpsraw512" => bi(Vec512, Psrawi512),
        "vpslld512" => bi(Vec512, Psllidi512),
        "vpsrld512" => bi(Vec512, Psrlidi512),
        "vpsrad512" => bi(Vec512, Psradi512),
        "vpsllq512" => bi(Vec512, Psllqi512),
        "vpsrlq512" => bi(Vec512, Psrlqi512),
        "vpsraq512" => bi(Vec512, Psraqi512),
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
        "vpshufb_maskz128" => m2(Vec128, MaskzShuffleEpi8_128),
        "vpshufb_mask128" => m3(Vec128, MaskShuffleEpi8_128),
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
        "cmpps128" => s2(Vec128, CmpPs128),
        "shufps128" => s2(Vec128, ShufPs128),
        "unpcklps128" => b(Vec128, UnpcklPs128),
        "unpckhps128" => b(Vec128, UnpckhPs128),
        "movmskps128" => s1(Scalar, MovemaskPs128),
        "cvtps2dq128" => u(Vec128, CvtPs2Ep32_128),
        "cvtdq2ps128" => u(Vec128, CvtEp32_2Ps_128),
        "cvttps2dq128" => u(Vec128, CvttPs2Ep32_128),
        "cvtps2pd128" => u(Vec128, CvtPs2Pd_128),
        "cvtpd2ps128" => u(Vec128, CvtPd2Ps_128),
        "divpd128" => b(Vec128, DivPd128),
        "minpd128" => b(Vec128, MinPd128),
        "maxpd128" => b(Vec128, MaxPd128),
        "sqrtpd128" => u(Vec128, SqrtPd128),
        "cmppd128" => s2(Vec128, CmpPd128),
        "shufpd128" => s2(Vec128, ShufPd128),
        "unpcklpd128" => b(Vec128, UnpcklPd128),
        "unpckhpd128" => b(Vec128, UnpckhPd128),
        "movmskpd128" => s1(Scalar, MovemaskPd128),
        "cvtpd2dq128" => u(Vec128, CvtPd2Ep32_128),
        "cvtdq2pd128" => u(Vec128, CvtEp32_2Pd_128),
        "cvttpd2dq128" => u(Vec128, CvttPd2Ep32_128),
        "movss128" => b(Vec128, Movss128),
        "movsd128" => b(Vec128, Movsd128),
        "cvtsi2ss128" => b(Vec128, CvtSi2Ss_128),
        "cvtsi2sd128" => b(Vec128, CvtSi2Sd_128),
        "cvtsi2ss64_128" => b(Vec128, CvtSi2Ss64_128),
        "cvtsi2sd64_128" => b(Vec128, CvtSi2Sd64_128),
        "cvtss2si128" => s1(Scalar, CvtSs2Si_128),
        "cvtsd2si128" => s1(Scalar, CvtSd2Si_128),
        "cvtss2sd128" => b(Vec128, CvtSs2Sd_128),
        "cvtsd2ss128" => b(Vec128, CvtSd2Ss_128),
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
        "vcvtdq2ps256" => u(Vec256, CvtEp32_2Ps_256),
        "vcvttps2dq256" => u(Vec256, CvttPs2Ep32_256),
        "vcvtps2pd256" => u(Vec256, CvtPs2Pd_256),
        "vcvtpd2ps256" => u(Vec128, CvtPd2Ps_256),
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
        "vcvtdq2pd256" => u(Vec256, CvtEp32_2Pd_256),
        "vcvttpd2dq256" => u(Vec128, CvttPd2Ep32_256),
        "vpermilps256" => ui(Vec256, VpermilPs256),
        "vpermilvarps256" => b(Vec256, VpermilvarPs256),
        "vpermilvarpd256" => b(Vec256, VpermilvarPd256),
        "vperm2f128" => s2(Vec256, Vperm2f128),
        "vinsertf128" => s2(Vec256, Vinsertf128),
        "vextractf128" => ui(Vec128, Vextractf128),
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

impl Lowerer {
    /// Lower a call to the generic SIMD family.
    /// Name format: `__lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}`.
    pub(super) fn lower_lccc_simd(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        let rest = name.strip_prefix("__lccc_simd")?;
        let (width_s, rest) = rest.split_once('_')?;
        let (_class_s, mnemonic) = rest.split_once('_')?;
        // Table keys are "<insn><width>" (e.g. vpaddd512); some special forms
        // carry their own suffix (e.g. vinserti32x4_512). Try both.
        let (class, op, has_imm, nargs) = lccc_simd_lookup(mnemonic)
            .or_else(|| lccc_simd_lookup(&format!("{}{}", mnemonic, width_s)))?;
        let width = match class {
            LcccSimdClass::Vec128 => 16usize,
            LcccSimdClass::Vec256 => 32,
            LcccSimdClass::Vec512 => 64,
            _ => 0,
        };
        // Drop the leading dummy/imm argument; keep exactly nargs real
        // operands; re-append the immediate (if any) as the LAST operand.
        let mut arg_ops: Vec<Operand> = args
            .iter()
            .skip(1)
            .take(nargs)
            .map(|a| self.lower_expr(a))
            .collect();
        if has_imm {
            if let Some(imm_expr) = args.first() {
                arg_ops.push(self.lower_expr(imm_expr));
            }
        }
        if std::env::var("LCCC_DEBUG_SIMD").is_ok() {
            eprintln!("[SIMD] {} -> {:?}: {:?}", name, op, arg_ops);
        }
        match class {
            LcccSimdClass::Vec128 | LcccSimdClass::Vec256 | LcccSimdClass::Vec512 => {
                let result_alloca = self.fresh_value();
                self.emit(Instruction::Alloca {
                    dest: result_alloca,
                    ty: IrType::Ptr,
                    size: width,
                    align: 0,
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
