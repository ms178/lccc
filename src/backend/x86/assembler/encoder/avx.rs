use super::*;

/// Intel SDM Vol.2: `{er}` (embedded rounding, implies SAE) vs `{sae}`
/// (SAE without rounding). GAS 2.47 rejects the wrong decorator per mnemonic.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvexSae {
    None,
    Er,
    Sae,
}

/// Standalone-`{sae}` rejection from [`apply_evex_sae`]; the EVEX
/// dispatcher appends `for `<mnemonic>'` (GAS: `unsupported static
/// rounding/sae for `vpaddd'`). Match on this constant, not a literal.
pub(crate) const SAE_UNSUPPORTED: &str = "unsupported static rounding/sae";

/// GAS 2.47 rejects a leading `{sae}`/`{r*-sae}` on an imm8-first form with
/// a POSITIONAL message (the token must sit after the immediates), not the
/// per-mnemonic `unsupported static rounding/sae`. The EVEX imm8 helpers
/// return this constant and the dispatcher's `r()` wrapper attaches the
/// mnemonic, matching GAS verbatim:
/// `` `vshufpd': RC/SAE operand must follow immediate operands ``.
pub(crate) const SAE_AFTER_IMM: &str = "RC/SAE operand must follow immediate operands";

/// Mixed-width vector registers on a packed EVEX form (GAS 2.47:
/// `register type mismatch for `vaddps''`). Sentinel returned by the
/// shared binary path; the dispatcher's `r()` wrapper attaches the
/// mnemonic, mirroring the SAE_UNSUPPORTED mechanism.
pub(crate) const REG_TYPE_MISMATCH: &str = "register type mismatch";

/// Packed-convert width behavior: same-width (`vcvtps2dq`), 2:1
/// narrowing (`vcvtpd2ps`: VEX dest is always xmm, AT&T mem-src to xmm
/// is ambiguous) or 1:2 widening (`vcvtps2pd`: VEX src is always xmm).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum VcvtKind {
    Same,
    Narrow,
    Wide,
}

/// Per-mnemonic packed-convert parameters: VEX `(opcode, pp)` (None
/// for the EVEX-only AVX512DQ converts), EVEX `(map, pp, w, opcode)`,
/// width kind, and source element size (for broadcast N).
pub(crate) struct VcvtParams {
    pub vex: Option<(u8, u8)>,
    pub map: u8,
    pub pp: u8,
    pub w: u8,
    pub opcode: u8,
    pub kind: VcvtKind,
    pub src_elem: u32,
}

/// Packed-convert parameters, opcodes verified against GAS 2.47 bytes.
pub(crate) fn vcvt_params(mnemonic: &str) -> Option<VcvtParams> {
    let p = |vex: Option<(u8, u8)>,
             map: u8,
             pp: u8,
             w: u8,
             opcode: u8,
             kind: VcvtKind,
             src_elem: u32| {
        VcvtParams {
            vex,
            map,
            pp,
            w,
            opcode,
            kind,
            src_elem,
        }
    };
    match mnemonic {
        // Widening: VEX xmm->xmm/xmm->ymm; EVEX + xmm->ymm/ymm->zmm.
        "vcvtps2pd" => Some(p(Some((0x5A, 0)), 1, 0, 0, 0x5A, VcvtKind::Wide, 4)),
        "vcvtdq2pd" => Some(p(Some((0xE6, 2)), 1, 2, 0, 0xE6, VcvtKind::Wide, 4)),
        "vcvtps2qq" => Some(p(None, 1, 1, 0, 0x7B, VcvtKind::Wide, 4)),
        "vcvttps2qq" => Some(p(None, 1, 1, 0, 0x7A, VcvtKind::Wide, 4)),
        // Narrowing: VEX xmm->xmm/ymm->xmm; EVEX + zmm->ymm; mem->ymm
        // is m512 (EVEX-only); mem->xmm is ambiguous (type mismatch).
        "vcvtpd2ps" => Some(p(Some((0x5A, 1)), 1, 1, 1, 0x5A, VcvtKind::Narrow, 8)),
        "vcvtpd2dq" => Some(p(Some((0xE6, 3)), 1, 3, 1, 0xE6, VcvtKind::Narrow, 8)),
        "vcvttpd2dq" => Some(p(Some((0xE6, 1)), 1, 1, 1, 0xE6, VcvtKind::Narrow, 8)),
        "vcvtqq2ps" => Some(p(None, 1, 0, 1, 0x5B, VcvtKind::Narrow, 8)),
        // Same width: xmm->xmm/ymm->ymm/zmm->zmm; mem unambiguous.
        "vcvtps2dq" => Some(p(Some((0x5B, 1)), 1, 1, 0, 0x5B, VcvtKind::Same, 4)),
        "vcvttps2dq" => Some(p(Some((0x5B, 2)), 1, 2, 0, 0x5B, VcvtKind::Same, 4)),
        "vcvtdq2ps" => Some(p(Some((0x5B, 0)), 1, 0, 0, 0x5B, VcvtKind::Same, 4)),
        "vcvtpd2qq" => Some(p(None, 1, 1, 1, 0x7B, VcvtKind::Same, 8)),
        "vcvttpd2qq" => Some(p(None, 1, 1, 1, 0x7A, VcvtKind::Same, 8)),
        "vcvtqq2pd" => Some(p(None, 1, 2, 1, 0xE6, VcvtKind::Same, 8)),
        // Unsigned-integer converts (AVX512DQ/F + VL; EVEX-only). Same
        // width classes as their signed siblings, verified against GAS
        // 2.47 (`scripts/distill_evex_opcodes.py`):
        //   vcvtps2uqq %xmm,%xmm / %ymm,%zmm / %zmm->none: WIDENING.
        "vcvtps2uqq" => Some(p(None, 1, 1, 0, 0x79, VcvtKind::Wide, 4)),
        "vcvttps2uqq" => Some(p(None, 1, 1, 0, 0x78, VcvtKind::Wide, 4)),
        //   vcvtpd2uqq keeps the lane count: xmm->xmm / ymm->ymm / zmm->zmm.
        "vcvtpd2uqq" => Some(p(None, 1, 1, 1, 0x79, VcvtKind::Same, 8)),
        "vcvttpd2uqq" => Some(p(None, 1, 1, 1, 0x78, VcvtKind::Same, 8)),
        //   float32 -> uint32 keeps the lane count.
        "vcvtps2udq" => Some(p(None, 1, 0, 0, 0x79, VcvtKind::Same, 4)),
        "vcvttps2udq" => Some(p(None, 1, 0, 0, 0x78, VcvtKind::Same, 4)),
        //   float64 -> uint32 halves it: zmm/m512 -> ymm.
        "vcvtpd2udq" => Some(p(None, 1, 0, 1, 0x79, VcvtKind::Narrow, 8)),
        "vcvttpd2udq" => Some(p(None, 1, 0, 1, 0x78, VcvtKind::Narrow, 8)),
        //   uint32 -> float32 same width (F2 prefix).
        "vcvtudq2ps" => Some(p(None, 1, 3, 0, 0x7A, VcvtKind::Same, 4)),
        //   uint64 -> float32 halves it (F2 prefix).
        "vcvtuqq2ps" => Some(p(None, 1, 3, 1, 0x7A, VcvtKind::Narrow, 8)),
        //   uint32 -> float64 widens (F3 prefix).
        "vcvtudq2pd" => Some(p(None, 1, 2, 0, 0x7A, VcvtKind::Wide, 4)),
        //   uint64 -> float64 same width (F3 prefix).
        "vcvtuqq2pd" => Some(p(None, 1, 2, 1, 0x7A, VcvtKind::Same, 8)),
        // F16C half-float widen (EVEX.66.0F38.W0 13 — GAS emits 62 f2):
        // xmm->xmm/ymm, ymm->zmm; memory source tuple N=2 (the ph element).
        "vcvtph2ps" => Some(p(None, 2, 1, 0, 0x13, VcvtKind::Wide, 2)),
        _ => None,
    }
}

/// Vector width class: 0 = xmm, 1 = ymm, 2 = zmm.
fn vec_width(name: &str) -> Option<u8> {
    if is_xmm(name) {
        Some(0)
    } else if is_ymm(name) {
        Some(1)
    } else if is_zmm(name) {
        Some(2)
    } else {
        None
    }
}

/// Packed-convert shape validation, shared by the VEX and EVEX paths
/// (GAS reports the same texts either way). Rejects cross-width
/// registers (`register type mismatch' for same-width, `operand size
/// mismatch' for narrowing/widening), non-vector operands (`operand
/// type mismatch') and the ambiguous narrowing mem->xmm (`operand type
/// mismatch', even masked — GAS never reaches the EVEX m128 form).
pub(crate) fn check_vcvt_shape(mnemonic: &str, ops: &[Operand]) -> Result<(), String> {
    let params = vcvt_params(mnemonic).ok_or_else(|| format!("{mnemonic} requires 2 operands"))?;
    if ops.len() != 2 {
        return Err(format!("number of operands mismatch for `{mnemonic}'"));
    }
    let dst_w = match &ops[1] {
        Operand::Register(r) => vec_width(&r.name),
        _ => None,
    };
    let Some(dst_w) = dst_w else {
        return Err(format!("operand type mismatch for `{mnemonic}'"));
    };
    match &ops[0] {
        Operand::Register(r) => {
            let Some(src_w) = vec_width(&r.name) else {
                return Err(format!("operand type mismatch for `{mnemonic}'"));
            };
            let ok = match params.kind {
                VcvtKind::Same => src_w == dst_w,
                VcvtKind::Narrow => matches!((src_w, dst_w), (0, 0) | (1, 0) | (2, 1)),
                VcvtKind::Wide => matches!((src_w, dst_w), (0, 0) | (0, 1) | (1, 2)),
            };
            if !ok {
                if params.kind == VcvtKind::Same {
                    return Err(format!("register type mismatch for `{mnemonic}'"));
                }
                return Err(format!("operand size mismatch for `{mnemonic}'"));
            }
        }
        Operand::Memory(_) => {
            if params.kind == VcvtKind::Narrow {
                match dst_w {
                    0 => return Err(format!("operand type mismatch for `{mnemonic}'")),
                    1 => {}
                    _ => return Err(format!("operand size mismatch for `{mnemonic}'")),
                }
            }
        }
        _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
    }
    Ok(())
}

/// Broadcast source/destination validation, shared by the VEX and EVEX
/// paths (GAS reports the same texts either way): non-broadcastable
/// sources (`operand type mismatch') and wrong-width destinations
/// (`operand size mismatch'). GPR sources accept any width (the
/// per-size matrix is unverified against GAS; follow-up).
pub(crate) fn check_broadcast_shape(
    mnemonic: &str,
    ops: &[Operand],
    evex: bool,
) -> Result<(), String> {
    if ops.len() != 2 {
        return Err(format!("number of operands mismatch for `{mnemonic}'"));
    }
    let dst_w = match &ops[1] {
        Operand::Register(r) => vec_width(&r.name),
        _ => None,
    };
    let Some(dst_w) = dst_w else {
        return Err(format!("operand type mismatch for `{mnemonic}'"));
    };
    let src_ok = match &ops[0] {
        // All broadcasts take memory sources.
        Operand::Memory(_) => true,
        Operand::Register(r) => {
            let vec = vec_width(&r.name);
            let gpr = is_reg8(&r.name)
                || is_reg16(&r.name)
                || is_reg32(&r.name)
                || is_reg64(&r.name)
                || is_egpr(&r.name);
            match mnemonic {
                "vbroadcastss" | "vbroadcastsd" => vec == Some(0),
                "vbroadcastf128" | "vbroadcasti128" => false,
                "vbroadcastf32x4" | "vbroadcastf32x8" | "vbroadcastf64x2" | "vbroadcastf64x4"
                | "vbroadcasti32x4" | "vbroadcasti32x8" | "vbroadcasti64x2" | "vbroadcasti64x4" => {
                    false
                }
                _ => vec == Some(0) || gpr,
            }
        }
        _ => false,
    };
    if !src_ok {
        // A scalar broadcast from a wider vector (`vbroadcastss
        // %ymm1,%zmm2`) is a size error, not a kind error — but only
        // for EVEX; the VEX form reports `operand type mismatch'
        // instead (GAS 2.47).
        if evex && matches!(mnemonic, "vbroadcastss" | "vbroadcastsd") {
            if let Operand::Register(r) = &ops[0] {
                if matches!(vec_width(&r.name), Some(1) | Some(2)) {
                    return Err(format!("operand size mismatch for `{mnemonic}'"));
                }
            }
        }
        return Err(format!("operand type mismatch for `{mnemonic}'"));
    }
    let dst_ok = match mnemonic {
        "vbroadcastss" => evex || dst_w <= 1,
        "vbroadcastsd" => dst_w >= 1,
        "vbroadcastf128" | "vbroadcasti128" => dst_w == 1,
        "vbroadcastf32x4" | "vbroadcastf64x2" | "vbroadcasti32x4" | "vbroadcasti64x2" => dst_w >= 1,
        "vbroadcastf32x8" | "vbroadcastf64x4" | "vbroadcasti32x8" | "vbroadcasti64x4" => dst_w == 2,
        _ => evex || dst_w <= 1,
    };
    if !dst_ok {
        return Err(format!("operand size mismatch for `{mnemonic}'"));
    }
    Ok(())
}

/// Compress/expand shape validation: `vpcompressd/q` store (reg ->
/// reg/mem), `vpexpandd/q` load (reg/mem -> reg). Same-width registers
/// (`register type mismatch' on cross), vector-or-memory sides only
/// (`operand type mismatch').
pub(crate) fn check_compress_shape(
    mnemonic: &str,
    ops: &[Operand],
    compress: bool,
) -> Result<(), String> {
    if ops.len() != 2 {
        return Err(format!("number of operands mismatch for `{mnemonic}'"));
    }
    let src_is_mem = matches!(ops[0], Operand::Memory(_));
    let dst_is_mem = matches!(ops[1], Operand::Memory(_));
    let src_w = match &ops[0] {
        Operand::Register(r) => vec_width(&r.name),
        _ => None,
    };
    let dst_w = match &ops[1] {
        Operand::Register(r) => vec_width(&r.name),
        _ => None,
    };
    if compress {
        // Store direction: vector-register source, reg/mem destination.
        if src_w.is_none() {
            return Err(format!("operand type mismatch for `{mnemonic}'"));
        }
        if dst_w.is_none() && !dst_is_mem {
            return Err(format!("operand type mismatch for `{mnemonic}'"));
        }
    } else {
        // Load direction: reg/mem source, vector-register destination.
        if dst_w.is_none() {
            return Err(format!("operand type mismatch for `{mnemonic}'"));
        }
        if src_w.is_none() && !src_is_mem {
            return Err(format!("operand type mismatch for `{mnemonic}'"));
        }
    }
    if let (Some(a), Some(b)) = (src_w, dst_w) {
        if a != b {
            return Err(format!("register type mismatch for `{mnemonic}'"));
        }
    }
    Ok(())
}

impl super::InstructionEncoder {
    // ---- VEX encoding helpers for AVX ----

    /// Emit a 2-byte or 3-byte VEX prefix.
    /// pp: 0=none, 1=66, 2=F3, 3=F2
    /// mm: 1=0F, 2=0F38, 3=0F3A
    /// w: 0 or 1 (VEX.W)
    /// vvvv: complement of source register number (15 - reg_num, or 15 if none)
    /// l: 0=128, 1=256
    /// r, x, b: VEX extension bits (inverted from REX)
    pub(crate) fn emit_vex(
        &mut self,
        r: bool,
        x: bool,
        b: bool,
        mm: u8,
        w: u8,
        vvvv: u8,
        l: u8,
        pp: u8,
    ) {
        let r_bit = if r { 0 } else { 1 };
        let x_bit = if x { 0 } else { 1 };
        let b_bit = if b { 0 } else { 1 };
        let vvvv_inv = (!vvvv) & 0xF;

        // Use 2-byte VEX if possible: mm=1, w=0, x=0, b=0
        if mm == 1 && w == 0 && !x && !b {
            self.bytes.push(0xC5);
            let byte2 = (r_bit << 7) | (vvvv_inv << 3) | (l << 2) | pp;
            self.bytes.push(byte2);
        } else {
            // 3-byte VEX
            self.bytes.push(0xC4);
            let byte1 = (r_bit << 7) | (x_bit << 6) | (b_bit << 5) | mm;
            let byte2 = (w << 7) | (vvvv_inv << 3) | (l << 2) | pp;
            self.bytes.push(byte1);
            self.bytes.push(byte2);
        }
    }

    /// Emit EVEX 4-byte prefix.
    /// Parameters match VEX but with additional EVEX-specific fields.
    /// ll: 00=128, 01=256, 10=512
    /// aaa: opmask register number (0 = no masking), z: zeroing mask semantics.
    pub(crate) fn emit_evex(
        &mut self,
        r: bool,
        x: bool,
        b: bool,
        r_prime: bool,
        mm: u8,
        w: u8,
        vvvv: u8,
        v_prime: bool,
        pp: u8,
        ll: u8,
        z: bool,
        aaa: u8,
        bcst: bool,
    ) {
        let r_bit = if r { 0u8 } else { 1 };
        let x_bit = if x { 0u8 } else { 1 };
        let b_bit = if b { 0u8 } else { 1 };
        let r_prime_bit = if r_prime { 0u8 } else { 1 };
        let vvvv_inv = (!vvvv) & 0xF;
        let v_prime_bit = if v_prime { 0u8 } else { 1 };

        self.bytes.push(0x62); // EVEX prefix indicator

        // Byte 1: R X B R' 0 0 mm
        let byte1 = (r_bit << 7) | (x_bit << 6) | (b_bit << 5) | (r_prime_bit << 4) | mm;
        self.bytes.push(byte1);

        // Byte 2: W vvvv 1 pp
        let byte2 = (w << 7) | (vvvv_inv << 3) | (1 << 2) | pp;
        self.bytes.push(byte2);

        // Byte 3: z L'L b V' aaa
        let byte3 =
            ((z as u8) << 7) | (ll << 5) | ((bcst as u8) << 4) | (v_prime_bit << 3) | (aaa & 0x7);
        self.bytes.push(byte3);
    }

    /// Extract EVEX masking info from an operand (register or memory):
    /// returns (aaa, zeroing). Mask-less operands yield (0, false).
    fn evex_mask_info(op: &Operand) -> (u8, bool) {
        match op {
            Operand::Register(r) => {
                let aaa = r.mask.as_ref().and_then(|m| reg_num(m)).unwrap_or(0);
                (aaa, r.zeroing)
            }
            Operand::Memory(m) => {
                let aaa = m.mask.as_ref().and_then(|m| reg_num(m)).unwrap_or(0);
                (aaa, m.zeroing)
            }
            _ => (0, false),
        }
    }

    /// VAES rounds and VPS{LL,RL}DQ whole-vector byte shifts have EVEX forms
    /// for the wider registers, but the ISA gives them no opmask (k1/z) and
    /// no embedded broadcast ({1toN}). GNU as rejects those operands with
    /// "unsupported masking/broadcast for `<mnemonic>'"; match that instead
    /// of silently encoding EVEX mask/bcst bits into an invalid instruction.
    pub(crate) fn evex_forbid_mask_bcst(mnemonic: &str, ops: &[Operand]) -> Result<(), String> {
        for op in ops {
            let (masked, zeroing, bcst) = match op {
                Operand::Register(reg) => (reg.mask.is_some(), reg.zeroing, false),
                Operand::Memory(mem) => (mem.mask.is_some(), mem.zeroing, mem.broadcast.is_some()),
                _ => continue,
            };
            if masked || zeroing {
                return Err(format!("unsupported masking for `{mnemonic}'"));
            }
            if bcst {
                return Err(format!("unsupported broadcast for `{mnemonic}'"));
            }
        }
        Ok(())
    }

    /// Broadcast-only variant of [`evex_forbid_mask_bcst`]: EVEX shifts by
    /// count-in-memory take an opmask but no `{1toN}` (GAS 2.47: "unsupported
    /// broadcast for `vpsllw'").
    pub(crate) fn evex_forbid_broadcast(mnemonic: &str, ops: &[Operand]) -> Result<(), String> {
        for op in ops {
            if matches!(op, Operand::Memory(mem) if mem.broadcast.is_some()) {
                return Err(format!("unsupported broadcast for `{mnemonic}'"));
            }
        }
        Ok(())
    }

    /// Encode an EVEX memory operand with scale-aware disp8 (EVEX disp8 is
    /// multiplied by N = element-size × vector-length; using it for
    /// displacements not divisible by N silently computes the WRONG address).
    /// The R/X/B extension bits must already be set in the emitted P1 byte.
    fn encode_evex_rm(&mut self, reg_field: u8, mem: &MemoryOperand, ll: u8) -> Result<(), String> {
        let scale_n = [16u32, 32, 64][ll as usize];
        self.encode_evex_mem(reg_field, mem, scale_n)
    }

    /// 5-bit EVEX register id: vector 0–31, GPR 0–31 (`gp_id`), k 0–7.
    fn evex_id(name: &str) -> Result<u8, String> {
        if let Some(v) = vec_reg_id(name) {
            return Ok(v);
        }
        if let Some(g) = gp_id(name) {
            return Ok(g);
        }
        let n = reg_num(name).ok_or_else(|| format!("bad EVEX register: {}", name))?;
        Ok(n | if needs_rex_ext(name) { 8 } else { 0 })
    }

    fn evex_vvvv_bits(name: Option<&str>) -> Result<(u8, bool), String> {
        match name {
            Some(v) => {
                let id = Self::evex_id(v)?;
                Ok((id & 0xF, (id & 16) != 0))
            }
            None => Ok((0, false)),
        }
    }

    /// EVEX prefix for a register r/m operand (ModRM.mod = 11).
    /// `vvvv` is the NDS/NDD register, or `None` for unused (encoded 1111).
    pub(crate) fn emit_evex_mod3(
        &mut self,
        dst: &str,
        src: &str,
        vvvv: Option<&str>,
        map: u8,
        w: u8,
        pp: u8,
        ll: u8,
        z: bool,
        aaa: u8,
        bcst: bool,
    ) -> Result<(u8, u8), String> {
        let d = Self::evex_id(dst)?;
        let s = Self::evex_id(src)?;
        let (vvvv_enc, v_prime) = Self::evex_vvvv_bits(vvvv)?;
        // APX map-4 lives in apx.rs (NDD/NF/EGPR). AVX-512 uses X = r/m bit4.
        self.emit_evex(
            (d & 8) != 0,
            (s & 16) != 0,
            (s & 8) != 0,
            (d & 16) != 0,
            map,
            w,
            vvvv_enc,
            v_prime,
            pp,
            ll,
            z,
            aaa,
            bcst,
        );
        Ok((d & 7, s & 7))
    }

    /// EVEX prefix for a memory r/m operand. Returns the 3-bit ModRM.reg field.
    pub(crate) fn emit_evex_memop(
        &mut self,
        reg: &str,
        mem: &MemoryOperand,
        vvvv: Option<&str>,
        map: u8,
        w: u8,
        pp: u8,
        ll: u8,
        z: bool,
        aaa: u8,
        bcst: bool,
    ) -> Result<u8, String> {
        let d = Self::evex_id(reg)?;
        let (x3, b3, b4, x4) = Self::evex_addr_bits(mem);
        let (vvvv_enc, v_prime) = Self::evex_vvvv_bits(vvvv)?;
        self.emit_evex(
            (d & 8) != 0,
            x3,
            b3,
            (d & 16) != 0,
            map,
            w,
            vvvv_enc,
            v_prime,
            pp,
            ll,
            z,
            aaa,
            bcst,
        );
        self.apply_evex_apx_addr(b4, x4);
        Ok(d & 7)
    }

    /// APX EGPR addressing on AVX-512 EVEX (GAS 2.47):
    /// P0 bit3 = B4 of the GP base (not inverted); P1 bit2 = !X4 of the index.
    /// Classic EVEX required those bits 0 and 1 respectively (X4=B4=0).
    fn evex_addr_bits(mem: &MemoryOperand) -> (bool, bool, bool, bool) {
        let b_id = mem.base.as_ref().and_then(|b| gp_id(&b.name));
        let x_id = mem.index.as_ref().and_then(|i| gp_id(&i.name));
        let b3 = match b_id {
            Some(id) => (id & 8) != 0,
            None => mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name)),
        };
        let x3 = match x_id {
            Some(id) => (id & 8) != 0,
            None => mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name)),
        };
        (
            x3,
            b3,
            b_id.is_some_and(|id| id >= 16),
            x_id.is_some_and(|id| id >= 16),
        )
    }

    fn apply_evex_apx_addr(&mut self, b4: bool, x4: bool) {
        let n = self.bytes.len();
        if n < 4 || self.bytes[n - 4] != 0x62 {
            return;
        }
        if b4 {
            self.bytes[n - 3] |= 1 << 3;
        }
        if x4 {
            self.bytes[n - 2] &= !0x04;
        }
    }

    fn evex_sae_class(map: u8, opcode: u8) -> EvexSae {
        match (map, opcode) {
            (1, 0x58 | 0x59 | 0x5C | 0x5E | 0x51) => EvexSae::Er, // add/mul/sub/div/sqrt
            // NOTE: 0x5B stays None here: convert ER is mnemonic-gated in
            // `encode_evex_vcvt` (can-round rule), and no other table
            // consumer encodes map-1 opcode 0x5B. A blanket Er would
            // false-accept e.g. `vcvttps2dq {rz-sae}` (GAS rejects).
            (1, 0x5D | 0x5F | 0xC2) => EvexSae::Sae, // min/max/vcmp
            (2, 0x96..=0x9F) | (2, 0xA6..=0xAF) | (2, 0xB6..=0xBF) => EvexSae::Er, // FMA + fmaddsub
            // vscalef{ps,pd,ss,sd}: the only SAE-capable member of the
            // AVX512F/DQ scalar-control family that reaches the binary
            // path. Bare `{sae}` is rejected (GAS: `unsupported static
            // rounding/sae`), `{r*-sae}` accepted.
            (2, 0x2C | 0x2D) => EvexSae::Er,
            _ => EvexSae::None,
        }
    }

    pub(crate) fn apply_evex_sae(
        sae: Option<Option<u8>>,
        class: EvexSae,
        vl_ll: u8,
    ) -> Result<(bool, u8), String> {
        match sae {
            None => Ok((false, vl_ll)),
            Some(tok) => match class {
                EvexSae::None => Err(SAE_UNSUPPORTED.to_string()),
                EvexSae::Er => match tok {
                    Some(rc) => Ok((true, rc)),
                    None => Err(SAE_UNSUPPORTED.to_string()),
                },
                EvexSae::Sae => match tok {
                    None => Ok((true, 0)),
                    Some(_) => Err(SAE_UNSUPPORTED.to_string()),
                },
            },
        }
    }

    /// Validate a *suffixed* `{sae}`/`{r*-sae}` (on a vector register)
    /// against the emitted instruction's SAE class. Returns the
    /// offending token (`{sae}` / `{rn-sae}` / ...) when GAS would say
    /// `unknown vector operation`, `None` when absent or legitimate.
    /// (`sae` = bare suffix present; `rounding` = `{r*-sae}` rc if any.)
    pub(crate) fn check_evex_sae_token(
        map: u8,
        opcode: u8,
        sae: bool,
        rounding: Option<u8>,
    ) -> Option<String> {
        if !sae && rounding.is_none() {
            return None;
        }
        let tok = match rounding {
            None => "{sae}".to_string(),
            Some(0) => "{rn-sae}".to_string(),
            Some(1) => "{rd-sae}".to_string(),
            Some(2) => "{ru-sae}".to_string(),
            Some(3) => "{rz-sae}".to_string(),
            Some(rc) => format!("{{r{rc}-sae}}"),
        };
        let bad = match Self::evex_sae_class(map, opcode) {
            EvexSae::None => true,
            EvexSae::Er => rounding.is_none(),
            EvexSae::Sae => rounding.is_some(),
        };
        bad.then_some(tok)
    }

    /// EVEX vector length from operands: 00=128(xmm), 01=256(ymm), 10=512(zmm).
    /// EVEX LL from the WIDEST vector register in the operand list. Max, not
    /// first-found: `vinserti32x8 $imm, %ymm28, %zmm29, %zmm30` carries its
    /// 256-bit source before the 512-bit dest, and LL must be 0b10. No
    /// GAS-accepted mixed-width shape wants first-found (narrowing and
    /// extract read the zmm source either way).
    fn evex_ll(ops: &[Operand]) -> u8 {
        let mut ll = 0b00;
        for op in ops {
            if let Operand::Register(r) = op {
                let name = r.name.to_lowercase();
                if name.starts_with("zmm") {
                    return 0b10;
                }
                if name.starts_with("ymm") {
                    ll = 0b01;
                }
            }
        }
        ll
    }

    /// Strip a leading `{sae}` / `{r*-sae}` AT&T decorator operand.
    /// `None` = no decorator; `Some(None)` = bare `{sae}`; `Some(Some(rc))` = `{r*-sae}`.
    pub(crate) fn peel_evex_sae(ops: &[Operand]) -> (&[Operand], Option<Option<u8>>) {
        match ops.first() {
            Some(Operand::Label(s)) => {
                if let Some(tok) = evex_sae_rounding(s) {
                    return (&ops[1..], Some(tok));
                }
            }
            Some(Operand::Register(r))
                if (r.sae || r.rounding.is_some())
                    && !is_xmm(&r.name)
                    && !is_ymm(&r.name)
                    && !is_zmm(&r.name)
                    && !is_kreg(&r.name) =>
            {
                return (&ops[1..], Some(r.rounding));
            }
            _ => {}
        }
        (ops, None)
    }

    fn evex_vl_bytes(ll: u8) -> u32 {
        [16u32, 32, 64][ll.min(2) as usize]
    }

    fn evex_mem_scale(mem: &MemoryOperand, vl_ll: u8, tuple_div: u8) -> (bool, u32) {
        if let Some(count) = mem.broadcast {
            let vl = Self::evex_vl_bytes(vl_ll);
            let n = if count == 0 {
                vl
            } else {
                vl / u32::from(count)
            };
            (true, n.max(1))
        } else {
            let n = Self::evex_vl_bytes(vl_ll) / u32::from(tuple_div.max(1));
            (false, n.max(1))
        }
    }

    /// EVEX 2-source binary op, AT&T (src, vvvv, dst) with optional mask on dst.
    /// Encodings verified byte-for-byte against GNU as 2.42 (see LCCC_ENGINEERING
    /// PLAN_PART3 WP-A; reference bytes captured from `as` on 2026-08-12).
    /// Example: vpaddd %zmm2, %zmm1, %zmm0  ->  62 f1 75 48 fe c2
    ///          vpmaddubsw %zmm2,%zmm1,%zmm0{%k1}{z} -> 62 f2 75 c9 04 c2
    pub(crate) fn encode_evex_binary(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
    ) -> Result<(), String> {
        self.encode_evex_binary_impl(ops, map, pp, w, opcode, None)
    }

    /// Scalar EVEX binary (vgetexpss/sd): the memory tuple is ELEMENT-sized
    /// (N = 4 for ss, 8 for sd — read from W), not Full VL. Everything else
    /// matches the packed path.
    pub(crate) fn encode_evex_binary_scalar(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
    ) -> Result<(), String> {
        self.encode_evex_binary_impl(ops, map, pp, w, opcode, Some(if w == 1 { 8 } else { 4 }))
    }

    fn encode_evex_binary_impl(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
        scalar_tuple_n: Option<u32>,
    ) -> Result<(), String> {
        let (ops, sae) = Self::peel_evex_sae(ops);
        if ops.len() != 3 {
            return Err("EVEX binary op requires 3 operands".to_string());
        }
        // Same-width rule: every vector REGISTER operand must agree on
        // xmm/ymm/zmm (sentinel — the dispatcher attaches the mnemonic).
        // Without it a mixed spelling silently encodes with the widest
        // operand's L'L and a truncated register field.
        {
            let width = |name: &str| -> Option<u8> {
                if is_zmm(name) {
                    Some(2)
                } else if is_ymm(name) {
                    Some(1)
                } else if is_xmm(name) {
                    Some(0)
                } else {
                    None
                }
            };
            let mut widths: Vec<u8> = Vec::with_capacity(3);
            for op in ops {
                if let Operand::Register(r) = op
                    && let Some(vw) = width(&r.name)
                {
                    widths.push(vw);
                }
            }
            if widths.windows(2).any(|p| p[0] != p[1]) {
                return Err(REG_TYPE_MISMATCH.to_string());
            }
        }
        let vl_ll = Self::evex_ll(ops);
        let (sae_bcst, ll) = Self::apply_evex_sae(sae, Self::evex_sae_class(map, opcode), vl_ll)?;
        let mut bcst = sae_bcst;
        let (aaa, z) = Self::evex_mask_info(&ops[2]);
        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let (dst_num, src_num) = self.emit_evex_mod3(
                    &dst.name,
                    &src.name,
                    Some(&vvvv.name),
                    map,
                    w,
                    pp,
                    ll,
                    z,
                    aaa,
                    bcst,
                )?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let (mem_bcst, scale_n) = match scalar_tuple_n {
                    Some(n) if mem.broadcast.is_none() => (false, n),
                    _ => Self::evex_mem_scale(mem, vl_ll, 1),
                };
                bcst |= mem_bcst;
                let dst_num = self.emit_evex_memop(
                    &dst.name,
                    mem,
                    Some(&vvvv.name),
                    map,
                    w,
                    pp,
                    ll,
                    z,
                    aaa,
                    bcst,
                )?;
                self.bytes.push(opcode);
                self.encode_evex_mem(dst_num, mem, scale_n)
            }
            _ => Err("unsupported EVEX binary operands".to_string()),
        }
    }

    /// EVEX unary op, AT&T (src, dst) with optional mask on dst; vvvv = 1 (GAS).
    /// Used for vpabs*, vpopcnt*, vsqrt*, vpmovzx*/vpmovsx* (src may be narrower).
    /// VEX packed convert: shape-check, then encode — except narrowing
    /// mem->ymm (m512), which has no VEX form and redirects to EVEX.
    pub(crate) fn encode_vcvt(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        check_vcvt_shape(mnemonic, ops)?;
        let params =
            vcvt_params(mnemonic).ok_or_else(|| format!("{mnemonic} requires 2 operands"))?;
        if params.kind == VcvtKind::Narrow
            && matches!(ops[0], Operand::Memory(_))
            && matches!(&ops[1], Operand::Register(r) if is_ymm(&r.name))
        {
            return self.encode_evex_vcvt(ops, mnemonic);
        }
        let Some((opcode, pp)) = params.vex else {
            return Err(format!("{mnemonic}: no VEX form"));
        };
        self.encode_avx_2op_0f(ops, opcode, pp)
    }

    /// EVEX packed convert (all widths, masked, broadcast). Disp8 N is
    /// width-kind-aware: same-width N=VL, narrowing mem (m512->ymm)
    /// N=64, widening N=VL/2, broadcast N=source-element-size. Converts
    /// have no SAE: a standalone `{sae}` fails via `apply_evex_sae`
    /// (GAS text), a suffixed one is ignored here and reported by the
    /// central post-encode check (`unknown vector operation').
    pub(crate) fn encode_evex_vcvt(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        let (ops, sae) = Self::peel_evex_sae(ops);
        // Convert ER/SAE classes (GAS 2.47, probed per mnemonic):
        //  * truncating `vcvtt*` accept ONLY bare `{sae}` (suppress-all);
        //    `{r*-sae}` is `unsupported static rounding/sae`;
        //  * the exact integer->FP widenings (vcvtps2pd, vcvtdq2pd,
        //    vcvtudq2pd) accept neither — no rounding mode exists;
        //  * everything else (fp->int non-truncating, fp->fp, int->fp
        //    with rounding like vcvtqq2pd/vcvtuqq2pd) is Er-class:
        //    `{r*-sae}` accepted, bare `{sae}` rejected.
        let sae_class = if mnemonic.starts_with("vcvtt") {
            EvexSae::Sae
        } else if matches!(mnemonic, "vcvtps2pd" | "vcvtdq2pd" | "vcvtudq2pd") {
            EvexSae::None
        } else {
            EvexSae::Er
        };
        let (sae_bcst, sae_ll) = Self::apply_evex_sae(sae, sae_class, 0)?;
        check_vcvt_shape(mnemonic, ops)?;
        let params =
            vcvt_params(mnemonic).ok_or_else(|| format!("{mnemonic} requires 2 operands"))?;
        let (aaa, z) = Self::evex_mask_info(&ops[1]);
        // Narrowing LL is the SOURCE width (ymm->xmm is LL=01,
        // zmm/m512->ymm is LL=10); same/wide use the dest width.
        // ER repurposes LL as the rounding control; the width-derived LL
        // applies only without SAE.
        let ll = if sae.is_some() {
            sae_ll
        } else if params.kind == VcvtKind::Narrow {
            match &ops[0] {
                Operand::Register(r) if is_zmm(&r.name) => 0b10,
                Operand::Register(r) if is_ymm(&r.name) => 0b01,
                Operand::Memory(_) => 0b10,
                _ => 0b00,
            }
        } else {
            match &ops[1] {
                Operand::Register(r) if is_zmm(&r.name) => 0b10,
                Operand::Register(r) if is_ymm(&r.name) => 0b01,
                _ => 0b00,
            }
        };
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let (dst_num, src_num) = self.emit_evex_mod3(
                    &dst.name, &src.name, None, params.map, params.w, params.pp, ll, z, aaa,
                    sae_bcst,
                )?;
                self.bytes.push(params.opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                // ER with a memory source is rejected (GAS text via the
                // router); the b-bit cannot mean both rounding and broadcast.
                if sae.is_some() {
                    return Err(SAE_UNSUPPORTED.to_string());
                }
                let vl = Self::evex_vl_bytes(ll);
                let (bcst, scale_n) = if mem.broadcast.is_some() {
                    (true, params.src_elem)
                } else {
                    match params.kind {
                        VcvtKind::Same => (false, vl),
                        VcvtKind::Narrow => (false, 64),
                        VcvtKind::Wide => (false, (vl / 2).max(1)),
                    }
                };
                let dst_num = self.emit_evex_memop(
                    &dst.name, mem, None, params.map, params.w, params.pp, ll, z, aaa, bcst,
                )?;
                self.bytes.push(params.opcode);
                self.encode_evex_mem(dst_num, mem, scale_n)
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    /// EVEX compress (reg -> reg/mem, store direction) / expand
    /// (reg/mem -> reg, load direction). Disp8 N is the element size
    /// (dword 4 / qword 8): the transferred prefix is mask-defined, so
    /// no VL multiple applies. No SAE (standalone `{sae}` fails here
    /// with the GAS text); explicit `{1toN}` is forbidden by the caller.
    pub(crate) fn encode_evex_compress_expand(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
        opcode: u8,
        w: u8,
        elem_n: u32,
        compress: bool,
    ) -> Result<(), String> {
        let (ops, sae) = Self::peel_evex_sae(ops);
        Self::apply_evex_sae(sae, EvexSae::None, 0)?;
        check_compress_shape(mnemonic, ops, compress)?;
        let (aaa, z) = Self::evex_mask_info(&ops[1]);
        let ll = if compress {
            match &ops[0] {
                Operand::Register(r) if is_zmm(&r.name) => 0b10,
                Operand::Register(r) if is_ymm(&r.name) => 0b01,
                _ => 0b00,
            }
        } else {
            match &ops[1] {
                Operand::Register(r) if is_zmm(&r.name) => 0b10,
                Operand::Register(r) if is_ymm(&r.name) => 0b01,
                _ => 0b00,
            }
        };
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let (reg, rm) = if compress {
                    self.emit_evex_mod3(&src.name, &dst.name, None, 2, w, 1, ll, z, aaa, false)?
                } else {
                    self.emit_evex_mod3(&dst.name, &src.name, None, 2, w, 1, ll, z, aaa, false)?
                };
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, reg, rm));
                Ok(())
            }
            (Operand::Register(src), Operand::Memory(mem)) if compress => {
                let reg = self.emit_evex_memop(&src.name, mem, None, 2, w, 1, ll, z, aaa, false)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(reg, mem, elem_n)
            }
            (Operand::Memory(mem), Operand::Register(dst)) if !compress => {
                let reg = self.emit_evex_memop(&dst.name, mem, None, 2, w, 1, ll, z, aaa, false)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(reg, mem, elem_n)
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    pub(crate) fn encode_evex_unary(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
    ) -> Result<(), String> {
        let (ops, sae) = Self::peel_evex_sae(ops);
        if ops.len() != 2 {
            return Err("EVEX unary op requires 2 operands".to_string());
        }
        // Vector length comes from the DESTINATION: pmovzx*/pmovsx* widen
        // (ymm src -> zmm dst) and the EVEX.LL field must be 10.
        let vl_ll = match ops.last() {
            Some(Operand::Register(r)) if r.name.to_lowercase().starts_with("zmm") => 0b10,
            Some(Operand::Register(r)) if r.name.to_lowercase().starts_with("ymm") => 0b01,
            _ => 0b00,
        };
        let (sae_bcst, ll) = Self::apply_evex_sae(sae, Self::evex_sae_class(map, opcode), vl_ll)?;
        let mut bcst = sae_bcst;
        // Compressed disp8 N is a fraction of VL for the pmovzx/sx wideners:
        // Half (bw/wd/dq), Quarter (bd/wq), Eighth (bq). Everything else is Full.
        let tuple_div = match opcode {
            0x30 | 0x33 | 0x35 | 0x20 | 0x23 | 0x25 => 2,
            0x31 | 0x34 | 0x21 | 0x24 => 4,
            0x32 | 0x22 => 8,
            _ => 1,
        };
        let (aaa, z) = Self::evex_mask_info(&ops[1]);
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let (dst_num, src_num) =
                    self.emit_evex_mod3(&dst.name, &src.name, None, map, w, pp, ll, z, aaa, bcst)?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let (mem_bcst, scale_n) = Self::evex_mem_scale(mem, vl_ll, tuple_div);
                bcst |= mem_bcst;
                let dst_num =
                    self.emit_evex_memop(&dst.name, mem, None, map, w, pp, ll, z, aaa, bcst)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(dst_num, mem, scale_n)
            }
            _ => Err("unsupported EVEX unary operands".to_string()),
        }
    }

    /// EVEX scalar convert-to-GP, AT&T (src, dst): `vcvtsd2usi`/`vcvtss2usi`
    /// (0x79) and the truncating `vcvttsd2usi`/`vcvttss2usi` (0x78).
    /// EVEX.LIG.F2/F3.0F.W{0,1}; W from the GP destination width, vvvv = 1.
    /// The ISA gives these no masking/broadcast/SAE (GNU as rejects all
    /// three); the caller chains `evex_forbid_mask_bcst` for message parity.
    /// `mem_n` is the memory-source tuple (8 for sd, 4 for ss) for EVEX disp8.
    pub(crate) fn encode_evex_cvt_to_gp(
        &mut self,
        ops: &[Operand],
        pp: u8,
        opcode: u8,
        mem_n: u32,
    ) -> Result<(), String> {
        let (ops, sae) = Self::peel_evex_sae(ops);
        if ops.len() != 2 {
            return Err("EVEX cvt-to-GP requires 2 operands".to_string());
        }
        // Reject `{sae}` with the GAS message, not an arity error.
        Self::apply_evex_sae(sae, EvexSae::None, 0)?;
        let dst = match &ops[1] {
            Operand::Register(r) if !is_xmm_or_ymm(&r.name) => r,
            _ => return Err("EVEX cvt-to-GP requires a general-purpose destination".to_string()),
        };
        let w = u8::from(is_reg64(&dst.name));
        match &ops[0] {
            Operand::Register(src) if is_xmm(&src.name) => {
                let (dst_num, src_num) =
                    self.emit_evex_mod3(&dst.name, &src.name, None, 1, w, pp, 0, false, 0, false)?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                let dst_num =
                    self.emit_evex_memop(&dst.name, mem, None, 1, w, pp, 0, false, 0, false)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(dst_num, mem, mem_n)
            }
            _ => Err("unsupported EVEX cvt-to-GP operands".to_string()),
        }
    }

    /// EVEX scalar convert-from-GP, AT&T (src, nds, dst): `vcvtusi2sd`/
    /// `vcvtusi2ss` (0x7B). EVEX.NDS.LIG.F2/F3.0F.W{0,1}; W comes from the
    /// `l`/`q` suffix (a memory source carries no width of its own).
    /// Like the to-GP direction, no masking/broadcast/SAE (rejected by GAS);
    /// the caller chains `evex_forbid_mask_bcst` for message parity.
    pub(crate) fn encode_evex_cvt_from_gp(
        &mut self,
        ops: &[Operand],
        pp: u8,
        w: u8,
    ) -> Result<(), String> {
        let (ops, sae) = Self::peel_evex_sae(ops);
        if ops.len() != 3 {
            return Err("EVEX cvt-from-GP requires 3 operands".to_string());
        }
        Self::apply_evex_sae(sae, EvexSae::None, 0)?;
        let nds = match &ops[1] {
            Operand::Register(r) => r,
            _ => return Err("EVEX cvt-from-GP: second operand must be a register".to_string()),
        };
        let dst = match &ops[2] {
            Operand::Register(r) => r,
            _ => return Err("EVEX cvt-from-GP: destination must be a register".to_string()),
        };
        let mem_n = if w == 1 { 8 } else { 4 };
        match &ops[0] {
            Operand::Register(src) if !is_xmm_or_ymm(&src.name) => {
                let (dst_num, src_num) = self.emit_evex_mod3(
                    &dst.name,
                    &src.name,
                    Some(&nds.name),
                    1,
                    w,
                    pp,
                    0,
                    false,
                    0,
                    false,
                )?;
                self.bytes.push(0x7B);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                let dst_num = self.emit_evex_memop(
                    &dst.name,
                    mem,
                    Some(&nds.name),
                    1,
                    w,
                    pp,
                    0,
                    false,
                    0,
                    false,
                )?;
                self.bytes.push(0x7B);
                self.encode_evex_mem(dst_num, mem, mem_n)
            }
            _ => Err("unsupported EVEX cvt-from-GP operands".to_string()),
        }
    }

    /// EVEX 2-operand + imm8 shuffle, AT&T ($imm, src, dst); vvvv = 1 (GAS).
    /// vpshufd/vpshuflw/vpshufhw: 66/F3/F2.0F.0F3A..70 /r ib.
    pub(crate) fn encode_evex_imm2(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
    ) -> Result<(), String> {
        // Imm8-first forms reject a leading RC/SAE token positionally
        // (GAS 2.47: `'{m}': RC/SAE operand must follow immediate
        // operands`); there is no legal SAE placement on the 2-op+imm8
        // forms at all.
        let (ops, head_sae) = Self::peel_evex_sae(ops);
        if head_sae.is_some() {
            return Err(SAE_AFTER_IMM.to_string());
        }
        if ops.len() != 3 {
            return Err("EVEX imm2 op requires 3 operands (imm, src, dst)".to_string());
        }
        let ll = Self::evex_ll(ops);
        let (aaa, z) = Self::evex_mask_info(&ops[2]);
        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let (dst_num, src_num) =
                    self.emit_evex_mod3(&dst.name, &src.name, None, map, w, pp, ll, z, aaa, false)?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(dst),
            ) => {
                let (mem_bcst, scale_n) = Self::evex_mem_scale(mem, ll, 1);
                let dst_num =
                    self.emit_evex_memop(&dst.name, mem, None, map, w, pp, ll, z, aaa, mem_bcst)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(dst_num, mem, scale_n)?;
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported EVEX imm2 operands".to_string()),
        }
    }

    /// EVEX 2-operand + imm8 (vpermq/vpermpd). Dest is ModRM.reg, vvvv unused.
    /// The previous "NDD / vvvv=dest, ModRM.reg=0" spelling encoded every
    /// non-zmm0 destination as the wrong register (verified vs GAS 2.47).
    pub(crate) fn encode_evex_imm2_ndd(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
    ) -> Result<(), String> {
        self.encode_evex_imm2(ops, map, pp, w, opcode)
    }

    /// EVEX shift-by-immediate, AT&T ($imm, src, dst); vvvv = dest (NDD).
    /// vpsllw/d/q, vpsrlw/d/q, vpsraw/d/q: 66.0F.71/72/73 /2|/4|/6 ib.
    /// EVEX shift dispatcher: `$imm` first operand takes the imm8 group
    /// path (`ModRM.reg` = /ext), otherwise the count sits in an xmm/m128
    /// r/m (`ModRM.reg` = dst, vvvv = src) — mirrors `encode_avx_shift`.
    pub(crate) fn encode_evex_shift(
        &mut self,
        mnemonic: &str,
        ops: &[Operand],
        w: u8,
        count_op: u8,
        imm_op: u8,
        ext: u8,
    ) -> Result<(), String> {
        if matches!(ops.first(), Some(Operand::Immediate(_))) {
            return self.encode_evex_shift_imm(ops, w, imm_op, ext);
        }
        if ops.len() != 3 {
            return Err("EVEX shift requires 3 operands (count, src, dst)".to_string());
        }
        Self::evex_forbid_broadcast(mnemonic, ops)?;
        let ll = Self::evex_ll(ops);
        let (aaa, z) = Self::evex_mask_info(&ops[2]);
        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(count), Operand::Register(src), Operand::Register(dst))
                if is_xmm(&count.name) =>
            {
                let (dst_num, count_num) = self.emit_evex_mod3(
                    &dst.name,
                    &count.name,
                    Some(&src.name),
                    1,
                    w,
                    1,
                    ll,
                    z,
                    aaa,
                    false,
                )?;
                self.bytes.push(count_op);
                self.bytes.push(self.modrm(3, dst_num, count_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(src), Operand::Register(dst)) => {
                let dst_num = self.emit_evex_memop(
                    &dst.name,
                    mem,
                    Some(&src.name),
                    1,
                    w,
                    1,
                    ll,
                    z,
                    aaa,
                    false,
                )?;
                self.bytes.push(count_op);
                // The count is always a fixed 128-bit tuple (N=16), whatever
                // the destination vector length.
                self.encode_evex_mem(dst_num, mem, 16)
            }
            (Operand::Register(count), _, _) if !is_xmm(&count.name) => {
                Err(format!("operand type mismatch for `{mnemonic}'"))
            }
            _ => Err("unsupported EVEX shift operands".to_string()),
        }
    }

    pub(crate) fn encode_evex_shift_imm(
        &mut self,
        ops: &[Operand],
        w: u8,
        opcode: u8,
        ext: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("EVEX shift requires 3 operands (imm, src, dst)".to_string());
        }
        let ll = Self::evex_ll(ops);
        let (aaa, z) = Self::evex_mask_info(&ops[2]);
        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_id = Self::evex_id(&src.name)?;
                let dst_id = Self::evex_id(&dst.name)?;
                self.emit_evex(
                    false,
                    (src_id & 16) != 0,
                    (src_id & 8) != 0,
                    false,
                    1,
                    w,
                    dst_id & 0xF,
                    (dst_id & 16) != 0,
                    1,
                    ll,
                    z,
                    aaa,
                    false,
                );
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, ext, src_id & 7));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(dst),
            ) => {
                let dst_id = Self::evex_id(&dst.name)?;
                let (x3, b3, b4, x4) = Self::evex_addr_bits(mem);
                let (mem_bcst, scale_n) = Self::evex_mem_scale(mem, ll, 1);
                self.emit_evex(
                    false,
                    x3,
                    b3,
                    false,
                    1,
                    w,
                    dst_id & 0xF,
                    (dst_id & 16) != 0,
                    1,
                    ll,
                    z,
                    aaa,
                    mem_bcst,
                );
                self.apply_evex_apx_addr(b4, x4);
                self.bytes.push(opcode);
                self.encode_evex_mem(ext, mem, scale_n)?;
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported EVEX shift operands".to_string()),
        }
    }

    /// EVEX 3-source + imm8 (AT&T: imm, src2, src1, dst) — vpternlog,
    /// vpalignr, vpclmulqdq, vinsert*, vpshld/shrd, and the AVX512DQ/F
    /// scalar-control family (vgetmantss/sd, vfixupimm*, vrange*,
    /// vreducess/sd, vrndscaless/sd).
    ///
    /// Decorator placement for the imm8 forms is positional (GAS 2.47):
    /// the RC/SAE token sits AFTER the immediate, BEFORE src1 — a leading
    /// token is the positional error `'{m}': RC/SAE operand must follow
    /// immediate operands`, and the post-immediate token is SAE-only
    /// (bare `{sae}`; `{r*-sae}` there is `unsupported static
    /// rounding/sae`).
    ///
    /// Memory tuples: vector forms transfer Full VL; the scalar-control
    /// forms (opcodes 0x27/0x51/0x55/0x57/0x0A/0x0B) are element-sized
    /// (N=4 for ss, 8 for sd — read from W).
    pub(crate) fn encode_evex_3src_imm(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
        allow_mixed: bool,
    ) -> Result<(), String> {
        // Same-width rule: every vector REGISTER operand must agree on
        // xmm/ymm/zmm (GAS 2.47 byte-probed on vpternlogd: mixed →
        // `register type mismatch'); the sentinel is attached by the
        // dispatcher's r() wrapper.
        {
            let width = |name: &str| -> Option<u8> {
                if is_zmm(name) {
                    Some(2)
                } else if is_ymm(name) {
                    Some(1)
                } else if is_xmm(name) {
                    Some(0)
                } else {
                    None
                }
            };
            let mut widths: Vec<u8> = Vec::with_capacity(4);
            for op in ops {
                if let Operand::Register(r) = op
                    && let Some(vw) = width(&r.name)
                {
                    widths.push(vw);
                }
            }
            if !allow_mixed && widths.windows(2).any(|p| p[0] != p[1]) {
                return Err(REG_TYPE_MISMATCH.to_string());
            }
        }

        // Head-position decorator on an imm8-first form: GAS reports the
        // placement, not the decorator.
        let (ops, head_sae) = Self::peel_evex_sae(ops);
        if head_sae.is_some() {
            return Err(SAE_AFTER_IMM.to_string());
        }
        // Post-immediate `{sae}` (AT&T: `vgetmantsd $imm, {sae}, %s2, %s1,
        // %dst`): legal bare form on the scalar-control mnemonics; a
        // rounding token there is rejected by the class rule below. The
        // token sits at index 1, so the operand window is rebuilt around
        // it (a suffix slice would drop the immediate instead).
        let owned;
        let (ops, sae) = match ops.get(1) {
            Some(Operand::Label(s)) if evex_sae_rounding(s).is_some() && ops.len() == 5 => {
                let sae = evex_sae_rounding(s);
                owned = [
                    ops[0].clone(),
                    ops[2].clone(),
                    ops[3].clone(),
                    ops[4].clone(),
                ];
                (&owned[..], sae)
            }
            _ => (ops, None),
        };
        let (sae_bcst, _) = Self::apply_evex_sae(sae, EvexSae::Sae, 0)?;
        if ops.len() != 4 {
            return Err("EVEX 3src-imm op requires 4 operands (imm, src2, src1, dst)".to_string());
        }
        let ll = Self::evex_ll(ops);
        let (aaa, z) = Self::evex_mask_info(&ops[3]);
        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src2),
                Operand::Register(src1),
                Operand::Register(dst),
            ) => {
                let (dst_num, src2_num) = self.emit_evex_mod3(
                    &dst.name,
                    &src2.name,
                    Some(&src1.name),
                    map,
                    w,
                    pp,
                    ll,
                    z,
                    aaa,
                    sae_bcst,
                )?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src2_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(src1),
                Operand::Register(dst),
            ) => {
                // vinserti32x4/i64x2 (38) N=16; vinserti32x8/i64x4 (3A) N=32;
                // scalar-control forms (27/51/55/57/0A/0B) are element-sized
                // (ss N=4, sd N=8, read from W); everything else Full VL.
                let tuple_n = match opcode {
                    0x38 => 16u32,
                    0x3A => 32,
                    0x27 | 0x51 | 0x55 | 0x57 | 0x0A | 0x0B => {
                        if w == 1 {
                            8
                        } else {
                            4
                        }
                    }
                    _ => Self::evex_vl_bytes(ll),
                };
                let (mem_bcst, scale_n) = if mem.broadcast.is_some() {
                    Self::evex_mem_scale(mem, ll, 1)
                } else {
                    (sae_bcst, tuple_n)
                };
                let dst_num = self.emit_evex_memop(
                    &dst.name,
                    mem,
                    Some(&src1.name),
                    map,
                    w,
                    pp,
                    ll,
                    z,
                    aaa,
                    mem_bcst,
                )?;
                self.bytes.push(opcode);
                self.encode_evex_mem(dst_num, mem, scale_n)?;
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported EVEX 3src-imm operands".to_string()),
        }
    }

    /// EVEX compare-to-mask, AT&T ($imm, src2, src1, kdst).
    /// vpcmpb/ub (0F3A 3F/3E), vpcmpw/uw (3F/3E W1), vpcmpd/ud (1F/1E),
    /// vpcmpq/uq (1F/1E W1). ModRM.reg = k-dest, r/m = src2, vvvv = src1.
    pub(crate) fn encode_evex_cmp_mask(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
    ) -> Result<(), String> {
        // `{sae}` sits between the immediate and src2: `$0, {sae}, %zmm1, %zmm2, %k1`.
        let (head, rest) = ops.split_first().ok_or("EVEX cmp-mask: missing operands")?;
        let (rest, sae) = Self::peel_evex_sae(rest);
        if rest.len() != 3 {
            return Err("EVEX cmp-mask op requires 4 operands (imm, src2, src1, kdst)".to_string());
        }
        let vl_ll = Self::evex_ll(rest);
        let (sae_bcst, ll) = Self::apply_evex_sae(sae, Self::evex_sae_class(map, opcode), vl_ll)?;
        let mut bcst = sae_bcst;
        match (head, &rest[0], &rest[1], &rest[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src2),
                Operand::Register(src1),
                Operand::Register(kdst),
            ) => {
                let k_num = reg_num(&kdst.name).ok_or("bad k-dest register")?;
                // The k destination carries a merge mask (`%k5{%k7}`); GAS
                // rejects `{z}` here ("unsupported masking").
                let (aaa, z) = Self::evex_mask_info(&rest[2]);
                if z {
                    return Err("unsupported masking for compare-to-mask".to_string());
                }
                let (_, src2_num) = self.emit_evex_mod3(
                    &kdst.name,
                    &src2.name,
                    Some(&src1.name),
                    map,
                    w,
                    pp,
                    ll,
                    false,
                    aaa,
                    bcst,
                )?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, k_num, src2_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(src1),
                Operand::Register(kdst),
            ) => {
                let (mem_bcst, scale_n) = Self::evex_mem_scale(mem, vl_ll, 1);
                bcst |= mem_bcst;
                let (aaa, z) = Self::evex_mask_info(&rest[2]);
                if z {
                    return Err("unsupported masking for compare-to-mask".to_string());
                }
                let k_num = self.emit_evex_memop(
                    &kdst.name,
                    mem,
                    Some(&src1.name),
                    map,
                    w,
                    pp,
                    ll,
                    false,
                    aaa,
                    bcst,
                )?;
                self.bytes.push(opcode);
                self.encode_evex_mem(k_num, mem, scale_n)?;
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported EVEX cmp-mask operands".to_string()),
        }
    }

    /// EVEX compare-to-mask without imm8, AT&T (src2, src1, kdst).
    /// vpshufbitqmb: F3.0F38.W0 8F /r (ModRM.reg = k-dest).
    pub(crate) fn encode_evex_cmp_mask_nimm(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("EVEX cmp-mask-nimm op requires 3 operands (src2, src1, kdst)".to_string());
        }
        let ll = Self::evex_ll(ops);
        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src2), Operand::Register(src1), Operand::Register(kdst)) => {
                let k_num = reg_num(&kdst.name).ok_or("bad k-dest register")?;
                let (aaa, z) = Self::evex_mask_info(&ops[2]);
                if z {
                    return Err("unsupported masking for compare-to-mask".to_string());
                }
                let (_, src2_num) = self.emit_evex_mod3(
                    &kdst.name,
                    &src2.name,
                    Some(&src1.name),
                    map,
                    w,
                    pp,
                    ll,
                    false,
                    aaa,
                    false,
                )?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, k_num, src2_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(src1), Operand::Register(kdst)) => {
                let k_num = reg_num(&kdst.name).ok_or("bad k-dest register")?;
                let (bcst, scale_n) = Self::evex_mem_scale(mem, ll, 1);
                let (aaa, z) = Self::evex_mask_info(&ops[2]);
                if z {
                    return Err("unsupported masking for compare-to-mask".to_string());
                }
                let _ = self.emit_evex_memop(
                    &kdst.name,
                    mem,
                    Some(&src1.name),
                    map,
                    w,
                    pp,
                    ll,
                    false,
                    aaa,
                    bcst,
                )?;
                self.bytes.push(opcode);
                self.encode_evex_mem(k_num, mem, scale_n)
            }
            _ => Err("unsupported EVEX cmp-mask-nimm operands".to_string()),
        }
    }

    /// EVEX extract with imm8, AT&T ($imm, src, dst); GAS encodes ModRM.reg = src,
    /// r/m = dst (verified on CPU: lane extraction semantics correct). vvvv = 1.
    /// vextracti32x4/i64x2 (0F3A 39), vextracti64x4 (0F3A 3B).
    pub(crate) fn encode_evex_extract_imm(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("EVEX extract requires 3 operands (imm, src, dst)".to_string());
        }
        let ll = Self::evex_ll(ops);
        let (aaa, z) = Self::evex_mask_info(&ops[2]);
        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                // GAS: ModRM.reg = src, r/m = dst.
                let (src_num, dst_num) =
                    self.emit_evex_mod3(&src.name, &dst.name, None, map, w, pp, ll, z, aaa, false)?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Memory(mem),
            ) => {
                // Tuple: i32x4/i64x2 (39) N=16, i32x8/i64x4 (3B) N=32.
                let tuple_n = if opcode == 0x3B { 32u32 } else { 16 };
                let src_num =
                    self.emit_evex_memop(&src.name, mem, None, map, w, pp, ll, z, aaa, false)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(src_num, mem, tuple_n)?;
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported EVEX extract operands".to_string()),
        }
    }

    /// EVEX masked/unmasked vector move, AT&T (src, dst) with mask on dst.
    /// vmovdqu8 (F3.0F.W0 6F/7F), vmovdqu16 (F3 W1), vmovdqu32 (F2 W0), vmovdqu64 (F2 W1).
    pub(crate) fn encode_evex_vmov(
        &mut self,
        ops: &[Operand],
        pp: u8,
        w: u8,
        load_op: u8,
        store_op: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("EVEX vmov requires 2 operands".to_string());
        }
        let ll = Self::evex_ll(ops);
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let (aaa, z) = Self::evex_mask_info(&ops[1]);
                let (dst_num, src_num) =
                    self.emit_evex_mod3(&dst.name, &src.name, None, 1, w, pp, ll, z, aaa, false)?;
                self.bytes.push(load_op);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let (aaa, z) = Self::evex_mask_info(&ops[1]);
                let dst_num =
                    self.emit_evex_memop(&dst.name, mem, None, 1, w, pp, ll, z, aaa, false)?;
                self.bytes.push(load_op);
                self.encode_evex_rm(dst_num, mem, ll)
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let (aaa, z) = Self::evex_mask_info(&ops[1]);
                let src_num =
                    self.emit_evex_memop(&src.name, mem, None, 1, w, pp, ll, z, aaa, false)?;
                self.bytes.push(store_op);
                self.encode_evex_rm(src_num, mem, ll)
            }
            _ => Err("unsupported EVEX vmov operands".to_string()),
        }
    }

    /// EVEX broadcast from GPR, AT&T (gpr, dst): vpbroadcastb/w/d/q.
    /// 66.0F38 7A/7B/7C (W0/W0/W0/W1). vvvv = 1 (GAS).
    pub(crate) fn encode_evex_broadcast_gpr(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("EVEX gpr-broadcast requires 2 operands".to_string());
        }
        let ll = Self::evex_ll(ops);
        let (aaa, z) = Self::evex_mask_info(&ops[1]);
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let (dst_num, src_num) =
                    self.emit_evex_mod3(&dst.name, &src.name, None, 2, w, 1, ll, z, aaa, false)?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num =
                    self.emit_evex_memop(&dst.name, mem, None, 2, w, 1, ll, z, aaa, false)?;
                self.bytes.push(opcode);
                // Tuple1 scalar: N is the broadcast element size, not VL.
                let n = match opcode {
                    0x18 => 4u32,        // vbroadcastss
                    0x19 => 8,           // vbroadcastsd
                    0x7A | 0x78 => 1u32, // byte
                    0x7B | 0x79 => 2,    // word
                    0x7C if w == 0 => 4, // dword (GPR form)
                    0x7C if w == 1 => 8, // qword
                    0x58 => 4,           // vpbroadcastd from xmm/mem
                    0x59 => 8,           // vpbroadcastq from xmm/mem
                    _ => [16u32, 32, 64][ll as usize],
                };
                self.encode_evex_mem(dst_num, mem, n)
            }
            _ => Err("unsupported EVEX gpr-broadcast operands".to_string()),
        }
    }

    /// EVEX broadcast from 128/256-bit memory, AT&T (mem, dst):
    /// vbroadcasti32x4/i64x2 (0F38 5A), vbroadcasti32x8/i64x4 (0F38 5B).
    pub(crate) fn encode_evex_broadcast_mem(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("EVEX mem-broadcast requires 2 operands".to_string());
        }
        let ll = Self::evex_ll(ops);
        let (aaa, z) = Self::evex_mask_info(&ops[1]);
        match (&ops[0], &ops[1]) {
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num =
                    self.emit_evex_memop(&dst.name, mem, None, 2, w, 1, ll, z, aaa, false)?;
                self.bytes.push(opcode);
                // Tuple type is a 128/256-bit chunk, not the destination VL:
                // i32x4/i64x2 → N=16, i32x8/i64x4 → N=32.
                let n = match opcode {
                    0x5A | 0x1A => 16u32,
                    0x5B | 0x1B => 32,
                    // vbroadcasti32x2/vbroadcastf32x2 load one 64-bit pair.
                    0x59 => 8,
                    _ => [16u32, 32, 64][ll as usize],
                };
                self.encode_evex_mem(dst_num, mem, n)
            }
            _ => Err("unsupported EVEX mem-broadcast operands".to_string()),
        }
    }

    /// Determine EVEX L'L from operands: 00=128(xmm), 01=256(ymm), 10=512(zmm)
    pub(crate) fn evex_ll_from_ops(&self, ops: &[Operand]) -> u8 {
        Self::evex_ll(ops)
    }

    /// Encode EVEX 3-operand instruction (e.g., vpxord, vpandd, etc.)
    /// Operands in AT&T order: src, vvvv, dst
    pub(crate) fn encode_evex_3op(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
        w: u8,
    ) -> Result<(), String> {
        // Same layout as encode_evex_binary (map=1). Keep this wrapper so
        // vpxord/vpandd and friends stay on the high-reg / dest!=0 path.
        self.encode_evex_binary(ops, 1, pp, w, opcode)
    }

    /// Encode EVEX rotate-by-immediate instructions (vprold, vprolq, vprord, vprorq).
    /// AT&T syntax: `vprold $imm8, %src, %dst`
    ///   ops[0] = imm8 (rotation count)
    ///   ops[1] = src  (in ModRM r/m field)
    ///   ops[2] = dst  (in EVEX.vvvv field)
    /// Extension digit `ext` goes in ModRM reg field (/0 for ror, /1 for rol).
    pub(crate) fn encode_evex_rotate_imm(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        ext: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("EVEX rotate requires 3 operands (imm, src, dst)".to_string());
        }
        let ll = self.evex_ll_from_ops(ops);

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_id = Self::evex_id(&src.name)?;
                let dst_id = Self::evex_id(&dst.name)?;
                let (aaa, z) = Self::evex_mask_info(&ops[2]);
                // pp=1 (66), mm=1 (0F map); ModRM.reg is the /ext, dest is vvvv.
                self.emit_evex(
                    false,
                    (src_id & 16) != 0,
                    (src_id & 8) != 0,
                    false,
                    1,
                    w,
                    dst_id & 0xF,
                    (dst_id & 16) != 0,
                    1,
                    ll,
                    z,
                    aaa,
                    false,
                );
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, ext, src_id & 7));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(dst),
            ) => {
                let dst_id = Self::evex_id(&dst.name)?;
                let (x3, b3, b4, x4) = Self::evex_addr_bits(mem);
                let (bcst, scale_n) = Self::evex_mem_scale(mem, ll, 1);
                let (aaa, z) = Self::evex_mask_info(&ops[2]);
                self.emit_evex(
                    false,
                    x3,
                    b3,
                    false,
                    1,
                    w,
                    dst_id & 0xF,
                    (dst_id & 16) != 0,
                    1,
                    ll,
                    z,
                    aaa,
                    bcst,
                );
                self.apply_evex_apx_addr(b4, x4);
                self.bytes.push(opcode);
                self.encode_evex_mem(ext, mem, scale_n)?;
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported EVEX rotate operands".to_string()),
        }
    }

    /// Determine VEX L (vector length) from operand: 0=128(xmm), 1=256(ymm)
    pub(crate) fn vex_l_from_ops(&self, ops: &[Operand]) -> u8 {
        for op in ops {
            match op {
                Operand::Register(r) if is_ymm(&r.name) => return 1,
                _ => {}
            }
        }
        0
    }

    /// Encode AVX vmovdqa/vmovdqu (load/store with 66/F3 prefix)
    pub(crate) fn encode_avx_mov(
        &mut self,
        ops: &[Operand],
        load_op: u8,
        store_op: u8,
        is_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX mov requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if is_66 { 1 } else { 2 }; // 66 -> pp=1, F3 -> pp=2

        match (&ops[0], &ops[1]) {
            // load: mem/reg -> xmm/ymm
            (Operand::Register(src), Operand::Register(dst))
                if (is_xmm(&src.name) && is_xmm(&dst.name))
                    || (is_ymm(&src.name) && is_ymm(&dst.name)) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                // Register-to-register moves can be spelled either direction:
                // the load form (opcode 28/6F/10) with reg=dst,rm=src, or the
                // store form (29/7F/11) with reg=src,rm=dst.  They differ only
                // in which operand lands in VEX.R vs VEX.B.  VEX.B has no
                // 2-byte encoding, so when the source needs the extension bit
                // (xmm8-15) but the destination does not, the load form is
                // forced to a 3-byte VEX while the store form fits in two.
                // GAS picks the shorter one: `vmovaps %xmm9,%xmm0` -> c5 79 29
                // c8, not c4 c1 78 28 c1.  One byte saved on a common move.
                let src_ext = needs_vex_ext(&src.name);
                let dst_ext = needs_vex_ext(&dst.name);
                if src_ext && !dst_ext {
                    self.emit_vex(src_ext, false, dst_ext, 1, 0, 0, l, pp);
                    self.bytes.push(store_op);
                    self.bytes.push(self.modrm(3, src_num, dst_num));
                    return Ok(());
                }
                self.emit_vex(dst_ext, false, src_ext, 1, 0, 0, l, pp);
                self.bytes.push(load_op);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(load_op);
                self.encode_modrm_mem(dst_num, mem)
            }
            // store: xmm/ymm -> mem
            (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(store_op);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported AVX mov operands".to_string()),
        }
    }

    /// Encode AVX vmovaps/vmovapd/vmovups/vmovupd (no mandatory prefix, or 66 prefix)
    /// Store-only VEX form: `op %xmm/%ymm, mem`. Used by the non-temporal
    /// stores, which have no load direction at all.
    pub(crate) fn encode_avx_store(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        is_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX store requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if is_66 { 1 } else { 0 };
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err(
                "non-temporal store requires register source and memory destination".to_string(),
            ),
        }
    }

    pub(crate) fn encode_avx_mov_np(
        &mut self,
        ops: &[Operand],
        load_op: u8,
        store_op: u8,
        is_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX mov requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if is_66 { 1 } else { 0 };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst))
                if (is_xmm(&src.name) && is_xmm(&dst.name))
                    || (is_ymm(&src.name) && is_ymm(&dst.name)) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                // Prefer the store direction when it avoids a 3-byte VEX; see
                // the matching comment in `encode_avx_mov`.
                let src_ext = needs_vex_ext(&src.name);
                let dst_ext = needs_vex_ext(&dst.name);
                if src_ext && !dst_ext {
                    self.emit_vex(src_ext, false, dst_ext, 1, 0, 0, l, pp);
                    self.bytes.push(store_op);
                    self.bytes.push(self.modrm(3, src_num, dst_num));
                    return Ok(());
                }
                self.emit_vex(dst_ext, false, src_ext, 1, 0, 0, l, pp);
                self.bytes.push(load_op);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(load_op);
                self.encode_modrm_mem(dst_num, mem)
            }
            (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(store_op);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported AVX mov operands".to_string()),
        }
    }

    /// Encode AVX 3-operand instruction with 66 prefix (or no prefix): op src, vvvv, dst
    /// Format: VEX.NDS.128/256.66.0F opcode /r
    pub(crate) fn encode_avx_3op(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        self.encode_avx_3op_commutative(ops, opcode, has_66, false)
    }

    /// Encode a VEX 3-operand instruction, optionally exploiting commutativity
    /// to reach the shorter 2-byte VEX prefix.
    ///
    /// The 2-byte VEX (C5) can only encode REX.R; it has no bit for B or X.
    /// So `vpaddd %xmm9,%xmm2,%xmm3` -- where the r/m operand is xmm9 -- needs
    /// the 3-byte C4 form purely to express VEX.B, costing one byte.
    ///
    /// For a COMMUTATIVE operation the two source operands can be exchanged:
    /// putting xmm9 in vvvv (which has 4 bits of its own, no REX needed) and
    /// xmm2 in r/m clears B and lets the 2-byte form encode it. clang and icx
    /// both do this; GAS, GCC and ICC do not.
    ///
    /// `commutative` is opt-in per mnemonic and is deliberately restricted to
    /// INTEGER and BITWISE operations. Bitwise FP ops (vandps/vorps/vxorps and
    /// their pd forms) qualify: they have no NaN-propagation rule at all, and
    /// were measured bit-identical under exchange with two distinct NaN
    /// payloads. Floating-point add/mul are NOT
    /// bit-commutative on x86: when both sources are NaN the result takes
    /// SRC1's payload, so exchanging them changes the result bits. Measured on
    /// this host: `vaddps` with sources (0x7fc00001, 0x7fc00002) yields
    /// 0x7fc00001 one way and 0x7fc00002 the other. clang performs the swap
    /// for vaddps/vmulps anyway; we do not, because a one-byte saving is not
    /// worth changing an architecturally-defined result.
    pub(crate) fn encode_avx_3op_commutative(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
        commutative: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX 3-op requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            // src_reg, vvvv_reg, dst_reg
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                // Exchange the sources when doing so removes the only reason we
                // would need the 3-byte prefix.
                let (src, vvvv) =
                    if commutative && needs_vex_ext(&src.name) && !needs_vex_ext(&vvvv.name) {
                        (vvvv, src)
                    } else {
                        (src, vvvv)
                    };
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // mem, vvvv_reg, dst_reg
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX 3-op operands".to_string()),
        }
    }

    /// Encode AVX 3-operand with no 66 prefix
    pub(crate) fn encode_avx_3op_np(&mut self, ops: &[Operand], opcode: u8) -> Result<(), String> {
        self.encode_avx_3op(ops, opcode, false)
    }

    /// Encode AVX 3-operand in 0F38 map
    pub(crate) fn encode_avx_3op_38(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX 3-op requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 2, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 2, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX 3-op operands".to_string()),
        }
    }

    /// Encode AVX 3-operand in the 0F38 map with an explicit VEX pp prefix
    /// (for the VNNI family, whose members use 66/F2/F3/no-prefix forms).
    pub(crate) fn encode_avx_3op_38_pp(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX 3-op requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 2, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // Memory source (VNNI dot-product with a folded load). The VEX
            // encoding is legal here — GAS 2.47 accepts
            // `{vex} vpdpbusd 0x20(%rax), %ymm1, %ymm2` as
            // `c4 e2 75 50 50 20` — but GAS *defaults* to the 7-byte EVEX
            // form (`62 f2 75 28 50 50 01`). LCCC deliberately emits the
            // shorter VEX form (6 bytes), consistent with the project's
            // xmm/ymm-stays-VEX policy; the `betterok` asmdiff cases assert
            // the win while requiring identical disassembly.
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 2, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX 3-op operands".to_string()),
        }
    }

    /// Encode a 0F3A-map AVX instruction with an imm8 where AT&T operands are
    /// (imm, src2, src1, dst) — vpclmulqdq (W0) and the GFNI affine forms
    /// (W1: GAS 2.47 emits `c4 e3 d1 ce ...` for `vgf2p8affineqb`). vvvv =
    /// src1 (NDS first source), r/m = src2, modrm.reg = dst.
    pub(crate) fn encode_avx_3op_3a_pp_imm8(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX 3A imm8 op requires 4 operands (imm, src2, src1, dst)".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let imm = match &ops[0] {
            Operand::Immediate(v) => match v {
                crate::backend::x86::assembler::parser::ImmediateValue::Integer(i) => *i as u8,
                _ => return Err("AVX 3A imm must be an integer".to_string()),
            },
            _ => return Err("AVX 3A imm must be an immediate".to_string()),
        };
        match (&ops[1], &ops[2], &ops[3]) {
            (Operand::Register(src2), Operand::Register(src1), Operand::Register(dst)) => {
                let src2_num = reg_num(&src2.name).ok_or("bad register")?;
                let src1_num = reg_num(&src1.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src2.name);
                let vvvv_enc = src1_num | (if needs_vex_ext(&src1.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 3, w, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src2_num));
                self.bytes.push(imm);
                Ok(())
            }
            // Memory src2 (GAS 2.47: `vpclmulqdq $0, (%rax), %xmm1, %xmm2`
            // -> `c4 e3 71 44 10 00`).
            (Operand::Memory(mem), Operand::Register(src1), Operand::Register(dst)) => {
                let src1_num = reg_num(&src1.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = src1_num | (if needs_vex_ext(&src1.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 3, w, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(imm);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX 3A imm8 operands".to_string()),
        }
    }

    /// Encode a 0F3A-map AVX instruction with an imm8 where AT&T operands are
    /// (imm, src, dst) and dest doubles as the NDS source — gf2p8affineqb/invqb.
    /// vvvv = dst, r/m = src, modrm.reg = dst.
    pub(crate) fn encode_avx_2op_3a_pp_imm8(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX 3A 2op imm8 requires 3 operands (imm, src, dst)".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let imm = match &ops[0] {
            Operand::Immediate(v) => match v {
                crate::backend::x86::assembler::parser::ImmediateValue::Integer(i) => *i as u8,
                _ => return Err("AVX 3A imm must be an integer".to_string()),
            },
            _ => return Err("AVX 3A imm must be an immediate".to_string()),
        };
        match (&ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                // This is a two-operand instruction: there is no NDS source, so
                // VEX.vvvv must be left unused.  `emit_vex` inverts this field,
                // so the "unused" 1111 encoding is requested by passing 0.  Passing
                // dst_num here made `vroundps $3,%ymm1,%ymm2` come out as
                // c4 e3 6d ... instead of GAS's c4 e3 7d ...
                self.emit_vex(r, false, b, 3, 0, 0, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(imm);
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 0, 0, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(imm);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX 3A 2op imm8 operands".to_string()),
        }
    }

    /// Encode AVX/FMA3 3-operand in 0F38 map with W=1 (double-precision FMA)
    /// vfmadd231pd, vfmadd213pd, vfmadd132pd, etc.
    pub(crate) fn encode_avx_3op_38_w1(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("FMA3 3-op requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 2, 1, vvvv_enc, l, pp); // W=1 for F64
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 2, 1, vvvv_enc, l, pp); // W=1 for F64
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            // NOTE: no (reg, mem, dst) "middle-memory" arm. GAS itself
            // rejects that spelling ("operand size mismatch"), and the one
            // producer this tree ever had (the `emit_vec_fma_128` acc-fold,
            // spelled `vfmadd132 %S2, MEM_acc, %dst`) was dead code from
            // the day it was written — the v6 safety net materialised every
            // fold before that emitter ran — so its inverted
            // multiplier/addend roles were never executed. It now emits the
            // canonical 213 form (`MEM_acc` first). An operand-order
            // mistake in a future emitter must fail the assembly loudly,
            // not silently encode a different instruction than the source
            // spelled.
            _ => Err("unsupported FMA3 3-op operands".to_string()),
        }
    }

    /// Encode AVX 3-operand in the 0F38 map with W=1 and an explicit VEX pp
    /// prefix (for the F2-prefixed FMA3 scalar-double forms: vfmadd*132sd, …).
    pub(crate) fn encode_avx_3op_38_pp_w1(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("FMA3 3-op requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);

        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 2, 1, vvvv_enc, l, pp); // W=1 for F64
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 2, 1, vvvv_enc, l, pp); // W=1 for F64
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            // NOTE: no (reg, mem, dst) "middle-memory" arm. GAS itself
            // rejects that spelling ("operand size mismatch"), and the one
            // producer this tree ever had (the `emit_vec_fma_128` acc-fold,
            // spelled `vfmadd132 %S2, MEM_acc, %dst`) was dead code from
            // the day it was written — the v6 safety net materialised every
            // fold before that emitter ran — so its inverted
            // multiplier/addend roles were never executed. It now emits the
            // canonical 213 form (`MEM_acc` first). An operand-order
            // mistake in a future emitter must fail the assembly loudly,
            // not silently encode a different instruction than the source
            // spelled.
            _ => Err("unsupported FMA3 3-op operands".to_string()),
        }
    }

    /// True when a VCMP packed-FP imm8 selects an operand-symmetric predicate.
    ///
    /// Relations eq/unord/neq/ord (imm&7 in {0,3,4,7}) are symmetric in every
    /// _q/_s signaling flavor, and true/false accept any operand order, so an
    /// assembler may exchange the sources (clang/icx do, to reach the 2-byte
    /// VEX prefix). Ordered relations and their negations never qualify.
    /// The immediate is reduced mod 32 first: GAS accepts the full imm8 range
    /// (e.g. `$0xab` assembles) and the predicate lives in the low 5 bits.
    fn vcmp_pred_is_symmetric(imm: i64) -> bool {
        matches!((imm as u8 & 31) & 7, 0 | 3 | 4 | 7)
    }

    /// Encode AVX 3-operand in 0F map (mm=1) with imm8 (vshufps, vshufpd, etc.)
    pub(crate) fn encode_avx_3op_0f_imm8(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX 3-op+imm8 requires 4 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                // Symmetric-predicate source swap (2-byte VEX win): VCMP
                // with (imm&31)&7 in {eq,unord,neq,ord} is operand-symmetric
                // in every _q/_s flavor (true/false included), so exchanging
                // the sources to clear VEX.B is bit-safe. clang and icx both
                // do this; GAS, GCC and ICC keep the 3-byte form. Ordered
                // predicates (lt/le/nlt/nle/...) never swap: no oracle does,
                // and NaN quieting is order-sensitive. vshuf (opcode 0xC6,
                // lane-selecting imm) shares this helper and never swaps.
                let (src, vvvv) = if opcode == 0xC2
                    && Self::vcmp_pred_is_symmetric(*imm)
                    && needs_vex_ext(&src.name)
                    && !needs_vex_ext(&vvvv.name)
                {
                    (vvvv, src)
                } else {
                    (src, vvvv)
                };
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp); // mm=1 (0F map)
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX 3-op+imm8 operands".to_string()),
        }
    }

    /// Encode AVX 2-operand in 0F38 map (e.g., vpabsb src, dst with vvvv=0)
    pub(crate) fn encode_avx_2op_38(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        self.encode_avx_2op_38_pp(ops, opcode, if has_66 { 1 } else { 0 })
    }

    /// pp-generalized 0F38 2-op (pp: 0=NP, 1=66, 2=F3, 3=F2) — vcvtph2ps
    /// is F3-prefixed, which the bool form cannot express.
    pub(crate) fn encode_avx_2op_38_pp(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX 2-op requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 2, 0, 0, l, pp); // vvvv=0 for 2-operand
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 2, 0, 0, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX 2-op operands".to_string()),
        }
    }

    /// Encode AVX 2-operand in 0F map (e.g., vmovddup, vmovshdup, vmovsldup)
    /// pp: 0=NP, 1=66, 2=F3, 3=F2
    pub(crate) fn encode_avx_2op_0f(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX 2-op requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, l, pp); // mm=1 (0F), vvvv=0
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX 2-op operands".to_string()),
        }
    }

    /// Encode AVX scalar comparison (vcmpss/vcmpsd) with F3/F2 prefix
    /// pp: 2=F3 (vcmpss), 3=F2 (vcmpsd)
    pub(crate) fn encode_avx_cmp_scalar(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX scalar cmp requires 4 operands (imm8, src, vvvv, dst)".to_string());
        }
        let l = 0; // LIG, use 128-bit

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX scalar cmp operands".to_string()),
        }
    }

    /// Encode AVX scalar 3-operand instruction (e.g. vmulss, vaddss)
    /// pp: 2=F3 (single), 3=F2 (double)
    pub(crate) fn encode_avx_scalar_3op(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        // The VEX scalar arithmetic forms are three-operand ONLY (SDM: `VADDSD
        // xmm1, xmm2, xmm3/m64`); GAS rejects `vaddsd %a, %d` with "number of
        // operands mismatch".  A former leniency here rewrote that spelling to
        // `%a, %d, %d`, which let two real defects survive unseen: the FP
        // emitter's `vsqrtsd %d, %d` (encoded with vvvv=0, i.e. a hidden
        // dependency on %xmm0) and the inline-asm `%d` duplicate modifier
        // printing its operand once (glibc's `%vdivss %1, %d0`).  Both are
        // fixed at the source; the assembler is an oracle again.
        if ops.len() != 3 {
            return Err(format!(
                "AVX scalar op requires 3 operands, got {} (the VEX scalar forms have no two-operand spelling)",
                ops.len()
            ));
        }
        let l = 0; // LIG - always 128-bit for scalar

        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX scalar 3-op operands".to_string()),
        }
    }

    /// Encode AVX scalar move (vmovss/vmovsd) - handles both 2-op (load/store) and 3-op (merge) forms
    /// pp: 2=F3 (vmovss), 3=F2 (vmovsd)
    pub(crate) fn encode_avx_scalar_mov(
        &mut self,
        ops: &[Operand],
        load_op: u8,
        store_op: u8,
        pp: u8,
        mnemonic: &str,
    ) -> Result<(), String> {
        match ops.len() {
            2 => {
                // 2-operand load/store form (no vvvv merge)
                match (&ops[0], &ops[1]) {
                    (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                        let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                        let r = needs_vex_ext(&dst.name);
                        let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                        let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                        self.emit_vex(r, x, b_ext, 1, 0, 0, 0, pp); // vvvv=0, L=0
                        self.bytes.push(load_op);
                        self.encode_modrm_mem(dst_num, mem)
                    }
                    (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                        let src_num = reg_num(&src.name).ok_or("bad register")?;
                        let r = needs_vex_ext(&src.name);
                        let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                        let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                        self.emit_vex(r, x, b_ext, 1, 0, 0, 0, pp); // vvvv=0, L=0
                        self.bytes.push(store_op);
                        self.encode_modrm_mem(src_num, mem)
                    }
                    // GAS 2.47: vmovss/vmovsd have NO two-register
                    // spelling (`vmovss %xmm6, %xmm7` is `operand type
                    // mismatch` — a register-to-register scalar move must
                    // use the 3-operand merge form or vmovaps). Formerly
                    // accepted here with the load encoding — a false
                    // accept, now rejected verbatim.
                    (Operand::Register(_), Operand::Register(_)) => {
                        Err(format!("operand type mismatch for `{mnemonic}'"))
                    }
                    _ => Err("unsupported AVX scalar mov 2-op operands".to_string()),
                }
            }
            3 => {
                // 3-operand merge form (VEX.NDS)
                self.encode_avx_scalar_3op(ops, load_op, pp)
            }
            _ => Err("AVX scalar mov requires 2 or 3 operands".to_string()),
        }
    }

    /// Encode AVX shuffle in 0F3A map (e.g. vpermilps, vpermilpd with immediate)
    /// Format: VEX.128/256.66.0F3A opcode /r ib
    pub(crate) fn encode_avx_shuffle_3a(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX shuffle 3A requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 3, 0, 0, l, pp); // mm=3 (0F3A), vvvv=0
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(dst),
            ) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 0, 0, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX shuffle 3A operands".to_string()),
        }
    }

    /// Parse SSE comparison predicate from pseudo-op mnemonic.
    /// Returns (predicate, suffix) e.g. "cmpnleps" -> Some((6, "ps"))
    pub(crate) fn parse_sse_cmp_pseudo(mnemonic: &str) -> Option<(u8, &str)> {
        if !mnemonic.starts_with("cmp") {
            return None;
        }
        let rest = &mnemonic[3..];
        // Try to match a suffix (ps, pd, ss, sd)
        let suffixes = ["ps", "pd", "ss", "sd"];
        for suffix in &suffixes {
            if let Some(pred_str) = rest.strip_suffix(*suffix) {
                let pred = match pred_str {
                    "eq" => 0,
                    "lt" => 1,
                    "le" => 2,
                    "unord" => 3,
                    "neq" => 4,
                    "nlt" => 5,
                    "nle" => 6,
                    "ord" => 7,
                    _ => continue,
                };
                return Some((pred, suffix));
            }
        }
        None
    }

    /// Try to encode an SSE comparison pseudo-op (e.g. cmpnleps -> cmpps $6, src, dst)
    pub(crate) fn try_encode_sse_cmp_pseudo(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<Option<()>, String> {
        let (pred, suffix) = match Self::parse_sse_cmp_pseudo(mnemonic) {
            Some(v) => v,
            None => return Ok(None),
        };
        let opcode: &[u8] = match suffix {
            "ps" => &[0x0F, 0xC2],
            "pd" => &[0x66, 0x0F, 0xC2],
            "ss" => &[0xF3, 0x0F, 0xC2],
            "sd" => &[0xF2, 0x0F, 0xC2],
            _ => return Ok(None),
        };
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }
        // Encode as the base instruction with an implicit immediate predicate
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let prefix_len = opcode.iter().position(|&b| b == 0x0F).unwrap_or(0);
                for &b in &opcode[..prefix_len] {
                    self.bytes.push(b);
                }
                self.emit_rex_rr(0, &dst.name, &src.name);
                self.bytes.extend_from_slice(&opcode[prefix_len..]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(pred);
                Ok(Some(()))
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let prefix_len = opcode.iter().position(|&b| b == 0x0F).unwrap_or(0);
                for &b in &opcode[..prefix_len] {
                    self.bytes.push(b);
                }
                self.emit_rex_rm(0, &dst.name, mem);
                self.bytes.extend_from_slice(&opcode[prefix_len..]);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(pred);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(Some(()))
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    /// Parse AVX comparison predicate from pseudo-op mnemonic.
    /// Returns (predicate, suffix) e.g. "vcmpnleps" -> Some((6, "ps"))
    pub(crate) fn parse_avx_cmp_pseudo(mnemonic: &str) -> Option<(u8, &str)> {
        if !mnemonic.starts_with("vcmp") {
            return None;
        }
        let rest = &mnemonic[4..];
        let suffixes = ["ps", "pd", "ss", "sd"];
        for suffix in &suffixes {
            if let Some(pred_str) = rest.strip_suffix(*suffix) {
                let pred = match pred_str {
                    "eq" => 0,
                    "lt" => 1,
                    "le" => 2,
                    "unord" => 3,
                    "neq" => 4,
                    "nlt" => 5,
                    "nle" => 6,
                    "ord" => 7,
                    // AVX extended predicates (8-31)
                    "eq_uq" => 8,
                    "nge" => 9,
                    "ngt" => 10,
                    "false" => 11,
                    "neq_oq" => 12,
                    "ge" => 13,
                    "gt" => 14,
                    "true" => 15,
                    "eq_os" => 16,
                    "lt_oq" => 17,
                    "le_oq" => 18,
                    "unord_s" => 19,
                    "neq_us" => 20,
                    "nlt_uq" => 21,
                    "nle_uq" => 22,
                    "ord_s" => 23,
                    "eq_us" => 24,
                    "nge_uq" => 25,
                    "ngt_uq" => 26,
                    "false_os" => 27,
                    "neq_os" => 28,
                    "ge_oq" => 29,
                    "gt_oq" => 30,
                    "true_us" => 31,
                    _ => continue,
                };
                return Some((pred, suffix));
            }
        }
        None
    }

    /// Try to encode an AVX comparison pseudo-op (e.g. vcmpnleps -> vcmpps $6, src, vvvv, dst)
    pub(crate) fn try_encode_avx_cmp_pseudo(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<Option<()>, String> {
        let (pred, suffix) = match Self::parse_avx_cmp_pseudo(mnemonic) {
            Some(v) => v,
            None => return Ok(None),
        };
        // AVX pseudo-ops take 3 operands: src, vvvv, dst (no explicit immediate)
        if ops.len() != 3 {
            return Err(format!("{} requires 3 operands", mnemonic));
        }
        let (has_66, pp_scalar) = match suffix {
            "ps" => (false, None),
            "pd" => (true, None),
            "ss" => (false, Some(2u8)), // F3
            "sd" => (false, Some(3u8)), // F2
            _ => return Ok(None),
        };

        let l = self.vex_l_from_ops(ops);
        let pp = match pp_scalar {
            Some(p) => p,
            None => {
                if has_66 {
                    1
                } else {
                    0
                }
            }
        };

        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                // Symmetric-predicate source swap (2-byte VEX win), same rule
                // as the numbered path: packed symmetric predicates only.
                // Scalar (ss/sd) never swaps: the merge lane comes from src.
                let (src, vvvv) = if pp_scalar.is_none()
                    && Self::vcmp_pred_is_symmetric(pred as i64)
                    && needs_vex_ext(&src.name)
                    && !needs_vex_ext(&vvvv.name)
                {
                    (vvvv, src)
                } else {
                    (src, vvvv)
                };
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(0xC2);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(pred);
                Ok(Some(()))
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(0xC2);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(pred);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(Some(()))
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    /// Encode AVX 3-operand in 0F3A map with imm8
    pub(crate) fn encode_avx_3op_3a_imm8(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX 3-op+imm8 requires 4 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 3, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 3, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX 3-op+imm8 operands".to_string()),
        }
    }

    /// VPERMIL2PS/PD — the FMA4-era five-operand VEX form
    /// (VEX.NDS.66.0F3A 48/49 /r ib; opcode 0x48 = ps, 0x49 = pd — the
    /// W bit does NOT select ps/pd here).
    ///
    /// AT&T shape: `$sel, src1, src2, src3, dst`. Three fields carry
    /// three of the four register sources — ModRM.reg = dst, r/m =
    /// src2, vvvv = src3 — so one source rides the imm8: GAS 2.47
    /// packs `imm8 = (slotless_reg << 4) | sel` where slotless_reg is
    /// src1 for the register/memory-src2 forms, and src2 when src1 is
    /// the memory operand (then r/m = that memory and W = 1 — the same
    /// memory-in-first-source signal the XOP 4-op forms pin with V'=1).
    /// All byte patterns probed against GAS 2.47, including the ymm
    /// (L=1) forms and `constant doesn't fit in 4 bits` for sel > 15.
    pub(crate) fn encode_vpermil2(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() > 5 {
            return Err("spurious operands; (5 operands/instruction max)".to_string());
        }
        if ops.len() != 5 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        let sel = match &ops[0] {
            Operand::Immediate(ImmediateValue::Integer(v)) => {
                if !(0..=15).contains(v) {
                    return Err(format!("constant doesn't fit in 4 bits for `{mnemonic}'"));
                }
                *v as u8
            }
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        // Width agreement across every vector register operand; L = width.
        let width_of = |op: &Operand| -> Option<u8> {
            match op {
                Operand::Register(r) if is_xmm(&r.name) => Some(0),
                Operand::Register(r) if is_ymm(&r.name) => Some(1),
                _ => None,
            }
        };
        let widths: Vec<u8> = ops[1..].iter().filter_map(width_of).collect();
        if widths.iter().any(|w| *w != widths[0]) {
            return Err(format!("register type mismatch for `{mnemonic}'"));
        }
        let l = widths.first().copied().unwrap_or(0);
        let dst_name = match &ops[4] {
            Operand::Register(r) => &r.name,
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        let dst_num = reg_num(dst_name).ok_or("bad register")?;
        let r = needs_vex_ext(dst_name);
        let src3 = match &ops[3] {
            Operand::Register(v) => v,
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        let vvvv_num = reg_num(&src3.name).ok_or("bad register")?;
        let vvvv_enc = vvvv_num | (if needs_vex_ext(&src3.name) { 8 } else { 0 });
        match (&ops[1], &ops[2]) {
            (Operand::Register(src1), Operand::Register(src2)) => {
                // imm8[7:4] carries the FULL 4-bit register id
                // (reg_num is the low 3 bits; the extension bit is 8).
                let slotless = reg_num(&src1.name).ok_or("bad register")?
                    | (if needs_vex_ext(&src1.name) { 8 } else { 0 });
                let b = needs_vex_ext(&src2.name);
                self.emit_vex(r, false, b, 3, 0, vvvv_enc, l, 1);
                self.bytes.push(opcode);
                self.bytes
                    .push(self.modrm(3, dst_num, reg_num(&src2.name).ok_or("bad register")?));
                self.bytes.push((slotless << 4) | sel);
                Ok(())
            }
            (Operand::Register(src1), Operand::Memory(mem)) => {
                // imm8[7:4] carries the FULL 4-bit register id
                // (reg_num is the low 3 bits; the extension bit is 8).
                let slotless = reg_num(&src1.name).ok_or("bad register")?
                    | (if needs_vex_ext(&src1.name) { 8 } else { 0 });
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 0, vvvv_enc, l, 1);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push((slotless << 4) | sel);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(src2)) => {
                let slotless = reg_num(&src2.name).ok_or("bad register")?
                    | (if needs_vex_ext(&src2.name) { 8 } else { 0 });
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                // W = 1: memory in the FIRST source position (GAS 2.47).
                self.emit_vex(r, x, b_ext, 3, 1, vvvv_enc, l, 1);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push((slotless << 4) | sel);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    /// EVEX expand/compress (AVX512F: vexpandp*/vcompressp*, EVEX.66.0F
    /// W0/W1 88/8A). AT&T `(src, dst)` for BOTH directions, but the
    /// register always sits in ModRM.reg: expand puts the DESTINATION in
    /// reg (`vexpandps (%rax), %xmm2` — r/m = memory source), compress
    /// puts the SOURCE in reg (`vcompressps %xmm2, (%rax)` — r/m = the
    /// memory destination). Half-memory tuples (GAS 2.47 disp8 probed:
    /// `vexpandps 16(%rax), %xmm2` scales by N=8? — see tuple_div below),
    /// no {1toN} broadcast, {k}{z} allowed on the destination.
    pub(crate) fn encode_evex_expandcompress(
        &mut self,
        ops: &[Operand],
        w: u8,
        opcode: u8,
        is_compress: bool,
    ) -> Result<(), String> {
        let (ops, sae) = Self::peel_evex_sae(ops);
        if ops.len() != 2 {
            return Err("EVEX expand/compress requires 2 operands".to_string());
        }
        // Reject `{sae}`/RC with the per-family GAS message (none exists).
        Self::apply_evex_sae(sae, EvexSae::None, 0)?;
        let (aaa, z) = Self::evex_mask_info(&ops[1]);
        // EVEX.LL comes from the REGISTER operand: the destination for
        // expand, the source for compress (probed: `vcompresspd %ymm2,
        // (%rax)` encodes LL=ymm).
        let reg_i = if is_compress { 0 } else { 1 };
        let ll = Self::evex_ll(std::slice::from_ref(&ops[reg_i]));
        // COMPRESSED tuple (byte-probed vs GAS 2.47): N = the ELEMENT
        // size at every VL — `vcompressps %zmm30, -512(%rdx)` -> disp8
        // 0x80 (N=4), `vcompresspd %zmm30, -1024(%rdx)` -> 0x80 (N=8);
        // the old VL/2 rule emitted the wrong compressed address.
        let tuple_n: u32 = if w == 1 { 8 } else { 4 };
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let (a, b) = if is_compress {
                    self.emit_evex_mod3(&src.name, &dst.name, None, 2, w, 1, ll, z, aaa, false)?
                } else {
                    self.emit_evex_mod3(&dst.name, &src.name, None, 2, w, 1, ll, z, aaa, false)?
                };
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, a, b));
                Ok(())
            }
            (Operand::Register(src), Operand::Memory(mem)) if is_compress => {
                let src_num =
                    self.emit_evex_memop(&src.name, mem, None, 2, w, 1, ll, z, aaa, false)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(src_num, mem, tuple_n)
            }
            (Operand::Memory(mem), Operand::Register(dst)) if !is_compress => {
                let dst_num =
                    self.emit_evex_memop(&dst.name, mem, None, 2, w, 1, ll, z, aaa, false)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(dst_num, mem, tuple_n)
            }
            _ => Err("operand type mismatch".to_string()),
        }
    }

    /// EVEX k-register compare/test (AVX512F/BW vptestm*). AT&T
    /// `(src1, src2, dst-k)`: vvvv = src1, r/m = src2 (register or memory
    /// — {1toN} broadcast allowed on the memory), ModRM.reg = the K
    /// destination. The k-destination may carry an input mask (`%k3{%k5}`
    /// → EVEX.aaa=5); `{z}` there is rejected (byte-probed GAS REJECT).
    pub(crate) fn encode_evex_ktest(
        &mut self,
        ops: &[Operand],
        pp: u8,
        w: u8,
        opcode: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("EVEX ktest requires 3 operands".to_string());
        }
        let dst = match &ops[2] {
            Operand::Register(r) => r,
            _ => return Err("operand type mismatch".to_string()),
        };
        let base = dst.name.split('{').next().unwrap_or(&dst.name).to_string();
        if !base.starts_with('k') {
            return Err("operand type mismatch".to_string());
        }
        if reg_num(&base).is_none() {
            return Err("operand type mismatch".to_string());
        }
        let (aaa, z) = Self::evex_mask_info(&ops[2]);
        if z {
            // {z} on an opmask destination: GAS rejects the spelling.
            return Err("operand type mismatch".to_string());
        }
        let ll = Self::evex_ll(&[ops[0].clone(), ops[1].clone()]);
        // Slot assignment (GAS 2.47 byte-probed: `vptestmd %xmm1, %xmm2,
        // %k3` = `62 f2 6d 08 27 d9` — vvvv = xmm2, r/m = xmm1): the
        // FIRST-listed source rides r/m, the SECOND rides vvvv.
        match (&ops[0], &ops[1]) {
            (Operand::Register(src1), Operand::Register(src2)) => {
                let (dst_num, rm_num) = self.emit_evex_mod3(
                    &base,
                    &src1.name,
                    Some(&src2.name),
                    2,
                    w,
                    pp,
                    ll,
                    z,
                    aaa,
                    false,
                )?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, rm_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(src2)) => {
                // The memory source takes r/m (Intel src2 = AT&T
                // first-listed); vvvv = the second-listed register.
                // `vptestmd %xmm1, (%rax), %k3` is GAS-rejected.
                let (bcst, scale_n) = Self::evex_mem_scale(mem, ll, 1);
                let dst_num =
                    self.emit_evex_memop(&base, mem, Some(&src2.name), 2, w, pp, ll, z, aaa, bcst)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(dst_num, mem, scale_n)
            }
            (Operand::Register(_), Operand::Memory(_)) => Err("operand type mismatch".to_string()),
            _ => Err("operand type mismatch".to_string()),
        }
    }

    /// EVEX down-conversion (AVX512F/DQ/BW vpmov*): EVEX.F3.0F38.W0
    /// 10–35. AT&T `(src, dst reg|mem{k}{z})`: ModRM.reg = the wide
    /// SOURCE, r/m = the narrow destination, EVEX.L'L encodes the SOURCE
    /// width, memory tuple N = source-lanes × dst-elem (byte-verified:
    /// vpmovwb zmm → 32, ymm → 16; vpmovdb xmm → 4 — i.e. VL/lanes…
    /// constant div = 4/dst_elem at every VL).
    pub(crate) fn encode_evex_narrow(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        dst_elem: u32,
        src_is_q: bool,
    ) -> Result<(), String> {
        let (ops, sae) = Self::peel_evex_sae(ops);
        if ops.len() != 2 {
            return Err("EVEX narrow requires 2 operands".to_string());
        }
        Self::apply_evex_sae(sae, EvexSae::None, 0)?;
        let src = match &ops[0] {
            Operand::Register(r)
                if is_xmm(&r.name)
                    || is_ymm(&r.name)
                    || r.name.to_lowercase().starts_with("zmm") =>
            {
                r
            }
            _ => return Err("operand type mismatch".to_string()),
        };
        let src_ll = Self::evex_ll(&[ops[0].clone()]);
        let (aaa, z) = Self::evex_mask_info(&ops[1]);
        // Memory tuple N = the narrow RESULT byte count at every source VL:
        // vpmovqb zmm -> 8 (VL/8), vpmovqw zmm -> 16 (VL/4), vpmovqd zmm
        // -> 32 (VL/2), vpmovdb xmm -> 4 (VL/4), vpmovdw ymm -> 16 (VL/2),
        // vpmovwb zmm -> 32 (VL/2) — all byte-probed against GAS 2.47.
        // The divisors below are those VL ratios, doubling for the
        // qword-source rows (half the lanes of the dword/word rows).
        let tuple_div = match (dst_elem, src_is_q) {
            (1, false) => 4, // *db family
            (2, false) => 2, // *dw and *wb families
            (_, false) => 1, // (no widening rows here)
            (1, true) => 8,  // *qb family
            (2, true) => 4,  // *qw family
            (_, true) => 2,  // *qd family
        };
        // Destination-width rule (GAS 2.47 byte-probed): the narrow
        // result keeps the source lane count, so the destination register
        // class is fixed by the mnemonic pair — `vpmovqd %zmm1, %xmm2`
        // (8 dwords = 32 bytes) is `operand size mismatch`, as is any
        // other off-class spelling.
        if let Operand::Register(dst) = &ops[1] {
            let src_lanes: u32 = match (src_ll, src_is_q) {
                (0b00, false) => 4,
                (0b01, false) => 8,
                (0b10, false) => 16,
                (0b00, true) => 2,
                (0b01, true) => 4,
                _ => 8,
            };
            let dst_bytes = src_lanes * dst_elem;
            let dst_ok = if dst_bytes <= 16 {
                is_xmm(&dst.name)
            } else {
                is_ymm(&dst.name)
            };
            if !dst_ok {
                return Err("operand size mismatch".to_string());
            }
        }
        match &ops[1] {
            Operand::Register(dst) => {
                let (src_num, dst_num) = self
                    .emit_evex_mod3(&src.name, &dst.name, None, 2, 0, 2, src_ll, z, aaa, false)?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                let src_num =
                    self.emit_evex_memop(&src.name, mem, None, 2, 0, 2, src_ll, z, aaa, false)?;
                self.bytes.push(opcode);
                let (_, scale_n) = Self::evex_mem_scale(mem, src_ll, tuple_div);
                self.encode_evex_mem(src_num, mem, scale_n)
            }
            _ => Err("operand type mismatch".to_string()),
        }
    }

    /// EVEX vcvtps2ph (AVX512F: EVEX.66.0F3A.W0 1D /r ib) — the imm8-first
    /// convert: `$imm, src(zmm|ymm|xmm), dst(ymm|xmm|mem{k}{z})`. EVEX.L'L
    /// encodes the SOURCE width (narrowing); mem destination tuple
    /// N = source-lanes × 2 → constant tuple_div = 2 (byte-verified zmm
    /// → 32). A leading `{sae}` token is the positional imm8-first error.
    pub(crate) fn encode_evex_vcvtps2ph(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("number of operands mismatch for `vcvtps2ph'".to_string());
        }
        let (ops, sae) = Self::peel_evex_sae(ops);
        if sae.is_some() {
            return Err(SAE_AFTER_IMM.to_string());
        }
        let imm = match &ops[0] {
            Operand::Immediate(ImmediateValue::Integer(v)) => *v as u8,
            _ => return Err("operand type mismatch for `vcvtps2ph'".to_string()),
        };
        let src = match &ops[1] {
            Operand::Register(r) => r,
            _ => return Err("operand type mismatch for `vcvtps2ph'".to_string()),
        };
        let src_ll = Self::evex_ll(&[ops[1].clone()]);
        let (aaa, z) = Self::evex_mask_info(&ops[2]);
        // Slot placement (GAS 2.47, byte-probed): ModRM.reg = SOURCE,
        // r/m = destination for BOTH the register and memory forms.
        match &ops[2] {
            Operand::Register(dst) => {
                let (src_num, dst_num) = self
                    .emit_evex_mod3(&src.name, &dst.name, None, 3, 0, 1, src_ll, z, aaa, false)?;
                self.bytes.push(0x1D);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                self.bytes.push(imm);
                Ok(())
            }
            Operand::Memory(mem) => {
                let src_num =
                    self.emit_evex_memop(&src.name, mem, None, 3, 0, 1, src_ll, z, aaa, false)?;
                self.bytes.push(0x1D);
                let (_, scale_n) = Self::evex_mem_scale(mem, src_ll, 2);
                self.encode_evex_mem(src_num, mem, scale_n)?;
                self.bytes.push(imm);
                Ok(())
            }
            _ => Err("operand type mismatch for `vcvtps2ph'".to_string()),
        }
    }

    /// AVX512F 3src+imm shuffles (vshuff32x4/vshuff64x2/vshufi32x4/
    /// vshufi64x2): no xmm forms exist in AVX512F (the 128-bit spellings
    /// are AVX512VL and GAS 2.47 rejects them standalone with `operand
    /// size mismatch` — mirrored here for line parity), ymm imm8 caps at
    /// 3, zmm at 7 (GAS: same `operand size mismatch` wording on
    /// overflow — byte-probed).
    pub(crate) fn encode_evex_shuffle32x4(
        &mut self,
        ops: &[Operand],
        w: u8,
        opcode: u8,
        mnemonic: &str,
    ) -> Result<(), String> {
        let xmm = ops
            .iter()
            .any(|op| matches!(op, Operand::Register(r) if is_xmm(&r.name)));
        if xmm {
            return Err(format!("operand size mismatch for `{mnemonic}'"));
        }
        // imm8 takes the full byte range (GAS 2.47 accepts $-1..$255,
        // rejects $256 with `operand type mismatch` — byte-probed).
        if let Some(Operand::Immediate(ImmediateValue::Integer(v))) = ops.first() {
            if !(-128..=255).contains(v) {
                return Err(format!("operand type mismatch for `{mnemonic}'"));
            }
        }
        self.encode_evex_3src_imm(ops, 3, 1, w, opcode, false)
    }

    /// VEX vcvtps2ph (F16C: VEX.128/256.66.0F3A.W0 1D /r ib — GAS 2.47
    /// byte-verified pp=66, NOT F2): `$imm, src(xmm|ymm), dst(xmm|mem)`.
    /// The register source rides ModRM.reg for the memory-destination
    /// form (`vcvtps2ph $4, %xmm1, (%rax)` = `c4 e3 79 1d 08 04`); vvvv
    /// unused; L from the source; zmm has no VEX form (routes EVEX).
    pub(crate) fn encode_avx_vcvtps2ph(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("number of operands mismatch for `vcvtps2ph'".to_string());
        }
        let imm = match &ops[0] {
            Operand::Immediate(ImmediateValue::Integer(v)) => *v as u8,
            _ => return Err("operand type mismatch for `vcvtps2ph'".to_string()),
        };
        let src = match &ops[1] {
            Operand::Register(r) if is_xmm(&r.name) || is_ymm(&r.name) => r,
            _ => return Err("operand type mismatch for `vcvtps2ph'".to_string()),
        };
        let src_num = reg_num(&src.name).ok_or("bad register")?;
        let l = self.vex_l_from_ops(&ops[1..]);
        match &ops[2] {
            Operand::Register(dst) if is_xmm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 3, 0, 0, l, 1);
                self.bytes.push(0x1D);
                // Byte-probed: ModRM.reg = the SOURCE, r/m = destination.
                self.bytes.push(self.modrm(3, src_num, dst_num));
                self.bytes.push(imm);
                Ok(())
            }
            Operand::Memory(mem) => {
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 0, 0, l, 1);
                self.bytes.push(0x1D);
                let rc = self.relocations.len();
                self.encode_modrm_mem(src_num, mem)?;
                self.bytes.push(imm);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("operand type mismatch for `vcvtps2ph'".to_string()),
        }
    }

    /// EVEX scalar moves with opmask support (vmovss: EVEX.LIG.F3.0F.W0,
    /// vmovsd: EVEX.LIG.F2.0F.W1; opcodes 10 = load/merge, 11 = store).
    /// GAS 2.47 byte-verified shapes:
    ///  * merge `vmovss %xmm4, %xmm5, %xmm6{%k7}{z}` — standard binary
    ///    placement (r/m = 1st-listed, vvvv = 2nd, reg = dst);
    ///  * load `vmovss (%rax), %xmm6{%k7}` — unary, mask on dst;
    ///  * store `vmovss %xmm6, (%rax){%k7}` — REVERSED: the register
    ///    source rides ModRM.reg, the memory is r/m;
    ///  * two-register masked spelling `vmovss %xmm6, %xmm7{%k7}` is
    ///    `operand type mismatch`.
    pub(crate) fn encode_evex_scalarmov(
        &mut self,
        ops: &[Operand],
        pp: u8,
        w: u8,
    ) -> Result<(), String> {
        match ops.len() {
            3 => {
                let (src1, src2, dst) = match (&ops[0], &ops[1], &ops[2]) {
                    (Operand::Register(a), Operand::Register(b), Operand::Register(d)) => {
                        (&a.name, &b.name, &d.name)
                    }
                    _ => return Err("operand type mismatch".to_string()),
                };
                let (aaa, z) = Self::evex_mask_info(&ops[2]);
                let (dst_num, rm_num) =
                    self.emit_evex_mod3(dst, src1, Some(src2), 1, w, pp, 0, z, aaa, false)?;
                self.bytes.push(0x10);
                self.bytes.push(self.modrm(3, dst_num, rm_num));
                Ok(())
            }
            2 => match (&ops[0], &ops[1]) {
                (Operand::Memory(mem), Operand::Register(dst)) => {
                    let (aaa, z) = Self::evex_mask_info(&ops[1]);
                    let dst_num =
                        self.emit_evex_memop(&dst.name, mem, None, 1, w, pp, 0, z, aaa, false)?;
                    self.bytes.push(0x10);
                    // Element-sized compressed disp (N=4 ss / N=8 sd —
                    // `vmovsd %xmm30, -1024(%rdx){%k7}` -> disp8 0x80,
                    // GAS 2.47 byte-probed; the raw N=1 rule forced a
                    // disp32 fallback GAS never takes).
                    self.encode_evex_mem(dst_num, mem, if w == 1 { 8 } else { 4 })
                }
                (Operand::Register(src), Operand::Memory(mem)) => {
                    let (aaa, z) = Self::evex_mask_info(&ops[1]);
                    let src_num =
                        self.emit_evex_memop(&src.name, mem, None, 1, w, pp, 0, z, aaa, false)?;
                    self.bytes.push(0x11);
                    self.encode_evex_mem(src_num, mem, if w == 1 { 8 } else { 4 })
                }
                (Operand::Register(_), Operand::Register(_)) => {
                    Err("operand type mismatch".to_string())
                }
                _ => Err("operand type mismatch".to_string()),
            },
            _ => Err("operand type mismatch".to_string()),
        }
    }

    /// Encode AVX vbroadcastss/vbroadcastsd
    pub(crate) fn encode_avx_broadcast(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("vbroadcast requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);

        match (&ops[0], &ops[1]) {
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                // VEX.256.66.0F38 opcode /r
                self.emit_vex(r, x, b_ext, 2, 0, 0, l, 1);
                self.bytes.extend_from_slice(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 2, 0, 0, l, 1);
                self.bytes.extend_from_slice(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            _ => Err("unsupported vbroadcast operands".to_string()),
        }
    }

    /// Encode vpbroadcastw/d/q from a GPR source (VEX.256.66.0F38 7B/7C /r).
    pub(crate) fn encode_avx_broadcast_gpr(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("vpbroadcast-gpr requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst))
                if !is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                // VEX.256.66.0F38.W{0|1} 7B|7C /r (mm=2, pp=1 for 66 prefix)
                self.emit_vex(r, false, b, 2, w, 0, 1, 1);
                self.bytes.extend_from_slice(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            _ => Err("unsupported vpbroadcast-gpr operands".to_string()),
        }
    }

    /// Encode EVEX GPR-source vpbroadcastb/w/d/q (62 .. 7A/7B/7C /r).
    /// EVEX.256.66.0F38.W{0|1} 7A|7B|7C /r with a GPR r/m source — the single-
    /// uop (port-5) scalar->vector splat that has no VEX equivalent.
    pub(crate) fn encode_avx_broadcast_evex_gpr(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("evex vpbroadcast-gpr requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst))
                if !is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let ll = if dst.name.to_lowercase().starts_with("zmm") {
                    0b10
                } else if dst.name.to_lowercase().starts_with("ymm") {
                    0b01
                } else {
                    0
                };
                let (dst_num, src_num) =
                    self.emit_evex_mod3(&dst.name, &src.name, None, 2, w, 1, ll, false, 0, false)?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            _ => Err("unsupported evex vpbroadcast-gpr operands".to_string()),
        }
    }

    /// Encode AVX pshufd-like (imm8 + 2 register operands).
    ///
    /// `pp` is the raw VEX pp field (0 = none, 1 = 66, 2 = F3, 3 = F2):
    /// vpshufd is 66 (pp=1), vpshuflw is F2 (pp=3), vpshufhw is F3 (pp=2).
    /// All three share opcode 0x70 in the 0F map with an imm8 trailing the
    /// ModRM byte (verified byte-for-byte against GAS 2.47).
    pub(crate) fn encode_avx_shuffle(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX shuffle requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(dst),
            ) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX shuffle operands".to_string()),
        }
    }

    /// Encode AVX vpmovmskb-like (xmm->gp)
    pub(crate) fn encode_avx_extract_gp(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX extract requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            _ => Err("unsupported AVX extract operands".to_string()),
        }
    }

    /// Encode AVX vmovd
    pub(crate) fn encode_avx_movd(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("vmovd requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            // GP -> XMM: VEX.128.66.0F 6E /r (W=1 for an r64 source)
            (Operand::Register(src), Operand::Register(dst))
                if !is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let w = u8::from(is_reg64(&src.name));
                self.emit_vex(r, false, b, 1, w, 0, 0, 1);
                self.bytes.push(0x6E);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // XMM -> GP: VEX.128.66.0F 7E /r (W=1 for an r64 destination)
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && !is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b = needs_vex_ext(&dst.name);
                let w = u8::from(is_reg64(&dst.name));
                self.emit_vex(r, false, b, 1, w, 0, 0, 1);
                self.bytes.push(0x7E);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            // mem -> XMM: VEX.128.66.0F 6E /r
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, 0, 1);
                self.bytes.push(0x6E);
                self.encode_modrm_mem(dst_num, mem)
            }
            // XMM -> mem: VEX.128.66.0F 7E /r
            (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, 0, 1);
                self.bytes.push(0x7E);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported vmovd operands".to_string()),
        }
    }

    /// Encode AVX vmovq
    pub(crate) fn encode_avx_movq(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("vmovq requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            // GP64 -> XMM: VEX.128.66.0F.W1 6E /r
            (Operand::Register(src), Operand::Register(dst))
                if !is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 1, 0, 0, 1);
                self.bytes.push(0x6E);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // XMM -> GP64: VEX.128.66.0F.W1 7E /r
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && !is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b = needs_vex_ext(&dst.name);
                self.emit_vex(r, false, b, 1, 1, 0, 0, 1);
                self.bytes.push(0x7E);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            // XMM -> XMM: VEX.128.F3.0F 7E /r
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, 0, 2);
                self.bytes.push(0x7E);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // mem -> XMM: VEX.128.F3.0F 7E /r
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, 0, 2);
                self.bytes.push(0x7E);
                self.encode_modrm_mem(dst_num, mem)
            }
            // XMM -> mem: VEX.128.66.0F D6 /r
            (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, 0, 1);
                self.bytes.push(0xD6);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported vmovq operands".to_string()),
        }
    }

    /// Encode AVX shift instructions (imm8 form or xmm form)
    pub(crate) fn encode_avx_shift(
        &mut self,
        ops: &[Operand],
        reg_op: u8,
        imm_ext: u8,
        imm_op: u8,
        has_66: bool,
    ) -> Result<(), String> {
        let pp = if has_66 { 1 } else { 0 };
        if ops.len() == 3 {
            match (&ops[0], &ops[1], &ops[2]) {
                // $imm, %xmm_src, %xmm_dst  (immediate shift, dst = vvvv).
                // NOTE: the VEX immediate-shift forms (0F 71/72/73) are
                // REGISTER-ONLY — the r/m-as-memory form exists solely
                // under EVEX (AVX-512), which this target does not assume.
                // GNU as confirms: `vpslld $3,(%rdi),%xmm3` assembles to an
                // EVEX encoding. The VEX-looking memory encoding is INVALID
                // and faults (SIGILL) — the VLFOLD analysis therefore never
                // admits immediate-shift consumers (see
                // memfold_consumer_unary_imm_* removal notes).
                (
                    Operand::Immediate(ImmediateValue::Integer(imm)),
                    Operand::Register(src),
                    Operand::Register(dst),
                ) => {
                    let src_num = reg_num(&src.name).ok_or("bad register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    let l = if is_ymm(&src.name) || is_ymm(&dst.name) {
                        1
                    } else {
                        0
                    };
                    let b = needs_vex_ext(&src.name);
                    let vvvv_enc = dst_num | (if needs_vex_ext(&dst.name) { 8 } else { 0 });
                    self.emit_vex(false, false, b, 1, 0, vvvv_enc, l, pp);
                    self.bytes.push(imm_op);
                    self.bytes.push(self.modrm(3, imm_ext, src_num));
                    self.bytes.push(*imm as u8);
                    Ok(())
                }
                // %xmm_count, %xmm_src(vvvv), %xmm_dst
                (Operand::Register(count), Operand::Register(vvvv), Operand::Register(dst))
                    if is_xmm_or_ymm(&count.name) =>
                {
                    let count_num = reg_num(&count.name).ok_or("bad register")?;
                    let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    let l = if is_ymm(&vvvv.name) || is_ymm(&dst.name) {
                        1
                    } else {
                        0
                    };
                    let r = needs_vex_ext(&dst.name);
                    let b = needs_vex_ext(&count.name);
                    let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                    self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                    self.bytes.push(reg_op);
                    self.bytes.push(self.modrm(3, dst_num, count_num));
                    Ok(())
                }
                // mem_count, %xmm_src(vvvv), %xmm_dst (VEX.NDS: r/m is the
                // shift count, vvvv the data). GAS 2.47:
                // `vpslld (%rax), %xmm6, %xmm7` -> `c5 c9 f2 38`.
                // The vector length comes from the data/dest registers;
                // the memory operand carries no width.
                (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                    let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    let l = if is_ymm(&vvvv.name) || is_ymm(&dst.name) {
                        1
                    } else {
                        0
                    };
                    let r = needs_vex_ext(&dst.name);
                    let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                    let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                    let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                    self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                    self.bytes.push(reg_op);
                    self.encode_modrm_mem(dst_num, mem)
                }
                _ => Err("unsupported AVX shift operands".to_string()),
            }
        } else {
            Err("AVX shift requires 3 operands".to_string())
        }
    }

    /// Encode x87 register-register arithmetic (fadd, fmul, fsub, fdiv).
    ///
    /// In AT&T syntax:
    ///   fadd %st(i), %st    -> D8 (base_st0 + i)  -- st(0) = st(0) op st(i)
    ///   fadd %st, %st(i)    -> DC (base_sti + i)  -- st(i) = st(i) op st(0)
    ///   fadd %st(i)         -> D8 (base_st0 + i)  -- shorthand for fadd %st(i), %st
    ///
    /// `opcode_st0` = D8 (reg field in modrm for st(0) as dest)
    /// `opcode_sti` = DC (reg field in modrm for st(i) as dest)
    /// `base_modrm` = base for the modrm second byte (e.g., 0xC0 for fadd)
    pub(crate) fn encode_x87_arith_reg(
        &mut self,
        ops: &[Operand],
        opcode_st0: u8,
        opcode_sti: u8,
        base_modrm: u8,
    ) -> Result<(), String> {
        match ops.len() {
            0 => {
                // Default: fadd %st(1), %st (i.e., st(0) = st(0) op st(1))
                self.bytes.extend_from_slice(&[opcode_st0, base_modrm + 1]);
                Ok(())
            }
            1 => {
                // fadd %st(i) -> st(0) = st(0) op st(i)
                match &ops[0] {
                    Operand::Register(reg) => {
                        let n = parse_st_num(&reg.name)?;
                        self.bytes.extend_from_slice(&[opcode_st0, base_modrm + n]);
                        Ok(())
                    }
                    _ => Err("x87 arith requires st register operand".to_string()),
                }
            }
            2 => {
                // Two operands: fadd %st(i), %st or fadd %st, %st(i)
                match (&ops[0], &ops[1]) {
                    (Operand::Register(src), Operand::Register(dst)) => {
                        let src_n = parse_st_num(&src.name)?;
                        let dst_n = parse_st_num(&dst.name)?;
                        if dst_n == 0 {
                            // fadd %st(i), %st -> D8 (base + i)
                            self.bytes
                                .extend_from_slice(&[opcode_st0, base_modrm + src_n]);
                        } else if src_n == 0 {
                            // fadd %st, %st(i) -> DC (base + i)
                            // Note: for fsub/fdiv, the DC form uses reversed base
                            // DC E0+i for fsubr, DC E8+i for fsub (swapped!)
                            // But base_modrm is from the D8 encoding perspective.
                            // The DC form for reverse direction:
                            // fadd: DC C0+i, fmul: DC C8+i, fsub: DC E8+i, fdiv: DC F8+i
                            // (fsub/fdiv swap: D8 E0 = fsub st(i),st; DC E8 = fsub st,st(i))
                            // The DC base is the instruction's OWN base
                            // (verified vs GNU as 2.47): fsub st,st(i) = DC E0+i,
                            // fsubr = DC E8+i, fdiv = DC F0+i, fdivr = DC F8+i.
                            // NO swap — the old remap table encoded fsub as fsubr.
                            self.bytes
                                .extend_from_slice(&[opcode_sti, base_modrm + dst_n]);
                        } else {
                            return Err("x87 arith: one operand must be st(0)".to_string());
                        }
                        Ok(())
                    }
                    _ => Err("x87 arith requires st register operands".to_string()),
                }
            }
            _ => Err("x87 arith requires 0-2 operands".to_string()),
        }
    }

    /// Encode fxch (exchange st(0) with st(i)).
    pub(crate) fn encode_fxch(&mut self, ops: &[Operand]) -> Result<(), String> {
        let n = match ops.len() {
            0 => 1, // fxch defaults to st(1)
            1 => match &ops[0] {
                Operand::Register(reg) => parse_st_num(&reg.name)?,
                _ => return Err("fxch requires st register".to_string()),
            },
            _ => return Err("fxch requires 0 or 1 operand".to_string()),
        };
        self.bytes.extend_from_slice(&[0xD9, 0xC8 + n]);
        Ok(())
    }

    /// Infer BMI2 W bit (0=32-bit, 1=64-bit) from destination register.
    pub(crate) fn bmi2_infer_w(&self, ops: &[Operand]) -> u8 {
        // Check destination (last operand) for register size
        if let Some(Operand::Register(r)) = ops.last() {
            if is_reg64(&r.name) {
                return 1;
            }
        }
        // Check other register operands
        for op in ops {
            if let Operand::Register(r) = op {
                if is_reg64(&r.name) {
                    return 1;
                }
                if is_reg32(&r.name) {
                    return 0;
                }
            }
        }
        1 // default to 64-bit
    }

    /// Encode AVX extract with imm8 (vextracti128, vextractf128)
    /// Format: VEX.256.66.0F3A opcode /r ib
    /// AT&T: $imm8, %src_ymm, %dst_xmm/mem
    pub(crate) fn encode_avx_extract_imm8(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX extract requires 3 operands".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b = needs_vex_ext(&dst.name);
                self.emit_vex(r, false, b, 3, 0, 0, 1, pp); // L=1 (256-bit source)
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Memory(mem),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 0, 0, 1, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(src_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX extract operands".to_string()),
        }
    }

    /// Encode AVX shuffle in 0F3A map with W=1 (vpermq, vpermpd)
    /// Format: VEX.256.66.0F3A.W1 opcode /r ib
    pub(crate) fn encode_avx_shuffle_3a_w1(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX permq requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 3, 1, 0, l, pp); // W=1
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(dst),
            ) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 1, 0, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX permq operands".to_string()),
        }
    }

    /// Encode AVX 4-operand instruction in 0F3A map (vblendvps, vblendvpd, vpblendvb)
    /// AT&T: $imm/mask, src, vvvv, dst -> actually: src_mask, src, vvvv, dst
    /// Intel: dst, vvvv, src, mask_reg
    /// VEX.NDS.128/256.66.0F3A opcode /r /is4
    pub(crate) fn encode_avx_4op_3a(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX 4-op requires 4 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        // AT&T: %mask, %src, %vvvv, %dst
        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Register(mask),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let mask_num = reg_num(&mask.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 3, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                // is4: mask register encoded in imm8[7:4]
                let mask_full = mask_num | (if needs_vex_ext(&mask.name) { 8 } else { 0 });
                self.bytes.push((mask_full & 0xF) << 4);
                Ok(())
            }
            // AT&T: %mask, mem, %vvvv, %dst — the rm operand is a memory
            // reference (VEX /is4 allows m128/m256 in the ModRM.rm slot).
            // Kernel crc-pclmul-template.S:
            //   vpblendvb %xmm3, -16(BUF,LEN), %xmm1, %xmm1
            // blends the final partial vector straight from memory.
            (
                Operand::Register(mask),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let mask_num = reg_num(&mask.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 3, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                let mask_full = mask_num | (if needs_vex_ext(&mask.name) { 8 } else { 0 });
                self.bytes.push((mask_full & 0xF) << 4);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX 4-op operands".to_string()),
        }
    }

    /// Encode AVX insert from GP register (vpinsrb, vpinsrd) via 0F3A map
    /// AT&T: $imm8, %gp/%mem, %xmm_vvvv, %xmm_dst
    pub(crate) fn encode_avx_insert_gp(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX insert requires 4 operands".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 3, 0, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 3, 0, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX insert operands".to_string()),
        }
    }

    /// Encode AVX insert via 0F map (vpinsrw)
    pub(crate) fn encode_avx_insert_gp_0f(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX insert requires 4 operands".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            // mem16 source (GAS 2.47: `vpinsrw $1, (%rax), %xmm1, %xmm2`
            // -> `c5 f1 c4 10 01`).
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX insert operands".to_string()),
        }
    }

    /// Encode AVX insert with W=1 (vpinsrq)
    pub(crate) fn encode_avx_insert_gp_w1(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX insert requires 4 operands".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 3, 1, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            // mem64 source (GAS 2.47: `vpinsrq $7, (%rax), %xmm8, %xmm9`
            // -> `c4 e3 d9 22 31 07`).
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 3, 1, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX insert operands".to_string()),
        }
    }

    /// Encode AVX word extract (vpextrw) with the GAS-preferred split:
    /// a GPR destination takes the legacy map-0F form VEX.128.66.0F C5
    /// /r ib (2-byte VEX: `c5 f9 c5 c0 00`), which is register-only, so a
    /// memory destination takes VEX.128.66.0F3A 15 /r ib instead
    /// (`c4 e3 79 15 00 00`). Shapes are validated centrally.
    pub(crate) fn encode_avx_extract_w(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX extract requires 3 operands".to_string());
        }
        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, 0, 1);
                self.bytes.push(0xC5);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Memory(mem),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 0, 0, 0, 1);
                self.bytes.push(0x15);
                let rc = self.relocations.len();
                self.encode_modrm_mem(src_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX extract operands".to_string()),
        }
    }

    /// Encode AVX extract byte/dword (vpextrb, vpextrd) via 0F3A map
    pub(crate) fn encode_avx_extract_byte(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX extract requires 3 operands".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b = needs_vex_ext(&dst.name);
                self.emit_vex(r, false, b, 3, 0, 0, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Memory(mem),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 0, 0, 0, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(src_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX extract operands".to_string()),
        }
    }

    /// Encode BMI2 3-operand GPR instructions (shrxq, shlxq, sarxq, etc.).
    ///
    /// AT&T syntax: `shrxq %src_shift, %r/m, %dst`
    ///   ops[0] = shift count register → VEX.vvvv
    ///   ops[1] = source r/m → ModRM r/m
    ///   ops[2] = destination → ModRM reg
    ///
    /// All use 0F38 map (mm=2). pp selects prefix: 0=NP, 1=66, 2=F3, 3=F2.
    pub(crate) fn encode_bmi2_shift(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("BMI2 instruction requires 3 operands".to_string());
        }

        // Two different AT&T operand conventions share this encoder.
        //
        // The shift-style forms (shlx/shrx/sarx/bzhi/bextr) put the r/m source
        // first and the vvvv operand second:  `shlxl %ecx, %eax, %edx`
        //   ops[0]=vvvv, ops[1]=r/m, ops[2]=dst.
        //
        // The data-manipulation forms (mulx/pdep/pext) instead follow the andn
        // convention, with the r/m operand FIRST:
        //   `mulxl %eax, %ecx, %edx`  ->  ops[0]=r/m, ops[1]=vvvv, ops[2]=dst.
        // Treating those as shift-style swapped vvvv with r/m, which silently
        // produced a valid but wrong instruction (c4 e2 7b f6 d1 instead of
        // c4 e2 73 f6 d0), and rejected the memory form outright.
        //
        // Distinguish by position: only the r/m operand may be memory, so if
        // ops[0] is memory this is the andn-style order.  For the all-register
        // case the caller tells us via the opcode: F5/F6 with pp!=0 are
        // pext/pdep/mulx, while bzhi (F5, pp=0) and the F7 shifts are
        // shift-style.
        let andn_order =
            matches!(ops[0], Operand::Memory(_)) || (matches!(opcode, 0xF5 | 0xF6) && pp != 0);
        let (rm_op, vvvv_op) = if andn_order {
            (&ops[0], &ops[1])
        } else {
            (&ops[1], &ops[0])
        };

        let vvvv_reg = match vvvv_op {
            Operand::Register(r) => r,
            _ => return Err("BMI2: vvvv operand must be register".to_string()),
        };
        // NF is legal for bextr (F7/pp=0) and bzhi (F5/pp=0) only.
        let nf_ok = matches!((opcode, pp), (0xF7, 0) | (0xF5, 0));
        if self.apx_nf && !nf_ok {
            return Err("{nf} unsupported for this BMI instruction".to_string());
        }
        let use_evex = self.apx_nf || self.apx_evex || operands_have_egpr(ops);

        if use_evex {
            return match (rm_op, &ops[2]) {
                (Operand::Register(src), Operand::Register(dst)) => {
                    let src_num = reg_num(&src.name).ok_or("bad src register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                    self.emit_apx_evex_vvvv_rr(
                        w != 0,
                        &dst.name,
                        &src.name,
                        &vvvv_reg.name,
                        self.apx_nf,
                        pp,
                        2,
                    )?;
                    self.bytes.push(opcode);
                    self.bytes.push(self.modrm(3, dst_num, src_num));
                    Ok(())
                }
                (Operand::Memory(mem), Operand::Register(dst)) => {
                    let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                    self.emit_apx_evex_vvvv_rm(
                        w != 0,
                        &dst.name,
                        mem,
                        &vvvv_reg.name,
                        self.apx_nf,
                        pp,
                        2,
                    )?;
                    self.bytes.push(opcode);
                    self.encode_modrm_mem(dst_num, mem)
                }
                _ => Err("BMI2: unsupported operand combination".to_string()),
            };
        }

        let vvvv_num = reg_num(&vvvv_reg.name).ok_or("bad vvvv register")?;
        let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv_reg.name) { 8 } else { 0 });

        match (rm_op, &ops[2]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 2, w, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 2, w, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("BMI2: unsupported operand combination".to_string()),
        }
    }

    /// Encode the VEX scalar convert-to-GP forms (`vcvtsd2si`, `vcvttss2si`...).
    ///
    /// Two operands: an XMM source and a general-purpose destination.  VEX.W
    /// selects the destination width, which means the 64-bit spelling cannot
    /// use the 2-byte VEX prefix.  LIG, so L is left at 0.
    /// `pp`: 2 = F3 (single source), 3 = F2 (double source).
    pub(crate) fn encode_avx_cvt_to_gp(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("VEX cvt-to-GP requires 2 operands".to_string());
        }
        let dst = match &ops[1] {
            Operand::Register(r) if !is_xmm_or_ymm(&r.name) => r,
            _ => return Err("VEX cvt-to-GP requires a general-purpose destination".to_string()),
        };
        let dst_num = reg_num(&dst.name).ok_or("bad destination register")?;
        let w = u8::from(is_reg64(&dst.name));
        let r = needs_vex_ext(&dst.name);

        match &ops[0] {
            Operand::Register(src) if is_xmm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad source register")?;
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, w, 0, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, w, 0, 0, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported VEX cvt-to-GP operands".to_string()),
        }
    }

    /// Encode the VEX scalar convert-from-GP forms (`vcvtsi2sd`/`vcvtsi2ss`).
    ///
    /// Three operands: an integer source (register or memory), an NDS operand
    /// that supplies the upper bits of the result, and an XMM destination.
    /// VEX.W selects the SOURCE width, taken from the `l`/`q` suffix because a
    /// memory source carries no width of its own.
    pub(crate) fn encode_avx_cvt_from_gp(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("VEX cvt-from-GP requires 3 operands".to_string());
        }
        let nds = match &ops[1] {
            Operand::Register(r) => r,
            _ => return Err("VEX cvt-from-GP: second operand must be a register".to_string()),
        };
        let dst = match &ops[2] {
            Operand::Register(r) => r,
            _ => return Err("VEX cvt-from-GP: destination must be a register".to_string()),
        };
        let dst_num = reg_num(&dst.name).ok_or("bad destination register")?;
        let vvvv = reg_num(&nds.name).ok_or("bad NDS register")?
            | (if needs_vex_ext(&nds.name) { 8 } else { 0 });
        let r = needs_vex_ext(&dst.name);

        match &ops[0] {
            Operand::Register(src) => {
                let src_num = reg_num(&src.name).ok_or("bad source register")?;
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, w, vvvv, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, w, vvvv, 0, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported VEX cvt-from-GP operands".to_string()),
        }
    }

    /// Encode `vmaskmov{ps,pd}` in both directions.
    ///
    /// Load:  `vmaskmovps (%rdi), %ymm1, %ymm2`  -> 66.0F38.W0 2C, mem is r/m.
    /// Store: `vmaskmovps %ymm2, %ymm1, (%rdi)` -> 66.0F38.W0 2E, mem is r/m.
    /// In both spellings the middle operand is the mask and goes in VEX.vvvv.
    pub(crate) fn encode_avx_maskmov(
        &mut self,
        ops: &[Operand],
        load_op: u8,
        store_op: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("vmaskmov requires 3 operands".to_string());
        }
        let mask = match &ops[1] {
            Operand::Register(r) => r,
            _ => return Err("vmaskmov: mask must be a register".to_string()),
        };
        let vvvv = reg_num(&mask.name).ok_or("bad mask register")?
            | (if needs_vex_ext(&mask.name) { 8 } else { 0 });
        let l = u8::from(is_ymm(&mask.name));

        match (&ops[0], &ops[2]) {
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 2, w, vvvv, l, 1);
                self.bytes.push(load_op);
                self.encode_modrm_mem(dst_num, mem)
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 2, w, vvvv, l, 1);
                self.bytes.push(store_op);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported vmaskmov operands".to_string()),
        }
    }

    /// Encode the AVX2 gathers (`vgatherdps`, `vpgatherdd`, ...).
    ///
    /// `vgatherdps %ymm2, (%rdi,%ymm1,4), %ymm3` has three operands: the mask
    /// register (VEX.vvvv), a VSIB memory operand whose index is a VECTOR
    /// register, and the destination.  The VSIB index supplies VEX.X, exactly
    /// like a scalar index would, so the normal memory path already encodes it
    /// correctly -- the only special part is that the index is an xmm/ymm.
    pub(crate) fn encode_avx_gather(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("gather requires 3 operands".to_string());
        }
        let mask = match &ops[0] {
            Operand::Register(r) => r,
            _ => return Err("gather: mask must be a register".to_string()),
        };
        let mem = match &ops[1] {
            Operand::Memory(m) => m,
            _ => return Err("gather: second operand must be a VSIB memory operand".to_string()),
        };
        let dst = match &ops[2] {
            Operand::Register(r) => r,
            _ => return Err("gather: destination must be a register".to_string()),
        };
        if mem.index.is_none() {
            return Err("gather: VSIB memory operand requires a vector index".to_string());
        }
        // GAS validates the (mask, dst, index) width triple per mnemonic
        // (probed 32/32 on GAS 2.47: dst x/y x index x/y for all 8). The
        // mask must match the dst; the (dst, index) pair must satisfy the
        // mnemonic's class, derived from (opcode bit0, W):
        //   idx-size == data-size (dps/dd/qpd/qq) -> widths must match;
        //   dword index over qword data (dpd/dq)  -> index must be xmm;
        //   qword index over dword data (qps/qd)  -> dst must be xmm
        // (VEX cannot address the 8 qword indices a ymm dst would need).
        let idx_name = &mem.index.as_ref().unwrap().name;
        if is_ymm(&mask.name) != is_ymm(&dst.name) {
            return Err("gather: mask and destination widths must match".to_string());
        }
        let (dst_y, idx_y) = (is_ymm(&dst.name), is_ymm(idx_name));
        let pair_ok = match (opcode & 1, w) {
            (0, 0) | (1, 1) => dst_y == idx_y,
            (0, 1) => !idx_y,
            _ => !dst_y,
        };
        if !pair_ok {
            return Err("gather: operand size mismatch".to_string());
        }

        let dst_num = reg_num(&dst.name).ok_or("bad destination register")?;
        let vvvv = reg_num(&mask.name).ok_or("bad mask register")?
            | (if needs_vex_ext(&mask.name) { 8 } else { 0 });
        // VEX.L is set when the destination/mask OR the VSIB index is 256-bit:
        // `vgatherqps %xmm12,(%r13,%ymm14,2),%xmm11` (qword indices over a
        // 128-bit result) encodes L=1. Verified against GAS 2.47.
        let l = u8::from(
            is_ymm(&dst.name)
                || is_ymm(&mask.name)
                || mem.index.as_ref().is_some_and(|i| is_ymm(&i.name)),
        );
        let r = needs_vex_ext(&dst.name);
        let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
        let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));

        self.emit_vex(r, x, b_ext, 2, w, vvvv, l, 1);
        self.bytes.push(opcode);
        self.encode_modrm_mem(dst_num, mem)
    }

    /// EVEX VSIB gather, AT&T (vsib-mem, dst {%k1-7}); dst in ModRM.reg,
    /// vvvv unused, Disp8*N with N = the DATA element size (w ? 8 : 4).
    /// All 8 mnemonics share the (opcode, w) of their VEX form; map=0F38,
    /// pp=66. GAS requires an explicit nonzero mask, rejects `{z}`, a mask
    /// or broadcast on the memory operand, and a RIP base; the (dst, index)
    /// width pair must satisfy the mnemonic's class (full 72-cell matrix
    /// probed on GAS 2.47):
    ///   idx-size == data-size (dps/dd/qpd/qq) -> widths must match;
    ///   dword index over qword data (dpd/dq)  -> (x,x), (y,x) or (z,y);
    ///   qword index over dword data (qps/qd)  -> (x,x), (x,y) or (y,z)
    /// (a zmm dst over qword indices would need 16 indices = 1024 index
    /// bits, which no vector register holds, so every zmm-dst qps/qd form
    /// is rejected).
    pub(crate) fn encode_evex_gather(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("EVEX gather requires 2 operands (vsib-mem, dst)".to_string());
        }
        let (mem, dst) = match (&ops[0], &ops[1]) {
            (Operand::Memory(m), Operand::Register(d)) => (m, d),
            _ => return Err("EVEX gather requires (vsib-mem, dst) operands".to_string()),
        };
        let idx_name = match mem.index.as_ref() {
            Some(i) if is_xmm_or_ymm(&i.name) || is_zmm(&i.name) => &i.name,
            _ => return Err("EVEX gather requires a vector index".to_string()),
        };
        if !is_xmm_or_ymm(&dst.name) && !is_zmm(&dst.name) {
            return Err("EVEX gather destination must be xmm/ymm/zmm".to_string());
        }
        if mem.base.as_ref().is_some_and(|b| b.name == "rip") {
            return Err("EVEX gather cannot use a RIP-relative base".to_string());
        }
        if mem.broadcast.is_some() {
            return Err("EVEX gather does not support broadcast".to_string());
        }
        if mem.mask.is_some() {
            return Err("EVEX gather mask belongs on the destination".to_string());
        }
        // Width class from (opcode bit0, W), mirroring the VEX helper.
        let (dst_y, dst_z) = (is_ymm(&dst.name), is_zmm(&dst.name));
        let (idx_y, idx_z) = (is_ymm(idx_name), is_zmm(idx_name));
        let pair_ok = match (opcode & 1, w) {
            (0, 0) | (1, 1) => (dst_y, dst_z) == (idx_y, idx_z),
            (0, 1) => {
                (!dst_y && !dst_z && !idx_y && !idx_z)
                    || (dst_y && !idx_y && !idx_z)
                    || (dst_z && idx_y && !idx_z)
            }
            _ => (!dst_y && !dst_z && !idx_z) || (dst_y && idx_z && !dst_z),
        };
        if !pair_ok {
            return Err("EVEX gather: operand size mismatch".to_string());
        }
        let (aaa, z) = Self::evex_mask_info(&ops[1]);
        if aaa == 0 {
            return Err("EVEX gather requires an explicit nonzero mask".to_string());
        }
        if z {
            return Err("unsupported masking for EVEX gather".to_string());
        }
        let dst_id = Self::evex_id(&dst.name)?;
        let idx_id = Self::evex_id(idx_name)?;
        let b_id = mem.base.as_ref().and_then(|b| gp_id(&b.name));
        let b3 = b_id.is_some_and(|id| id & 8 != 0);
        let b4 = b_id.is_some_and(|id| id >= 16);
        // LL is the WIDER of dst and index: `vgatherqps (mem), %xmm3` over
        // a ymm index encodes LL=01 (and decodes its xmm dst from the
        // mnemonic + LL), while `vgatherdpd (mem), %zmm3` over a ymm index
        // encodes LL=10. Max fits all 24 GAS-accepted (dst, index) cells.
        let width = |y: bool, z: bool| if z { 0b10 } else { u8::from(y) };
        let ll = width(dst_y, dst_z).max(width(idx_y, idx_z));
        self.emit_evex(
            (dst_id & 8) != 0,
            (idx_id & 8) != 0,
            b3,
            (dst_id & 16) != 0,
            2,
            w,
            0,
            (idx_id & 16) != 0,
            1,
            ll,
            false,
            aaa,
            false,
        );
        self.apply_evex_apx_addr(b4, false);
        self.bytes.push(opcode);
        self.encode_evex_mem(dst_id & 7, mem, if w != 0 { 8 } else { 4 })
    }

    /// Encode the BMI1 single-source bit-manipulation forms blsi/blsr/blsmsk.
    ///
    /// `blsrl %eax, %ecx` reads a single r/m source and writes one destination,
    /// but the destination is encoded in VEX.vvvv rather than ModRM.reg -- the
    /// reg field is repurposed as an opcode extension selecting the operation
    /// (/1 blsr, /2 blsmsk, /3 blsi).  All three share opcode 0xF3 in the
    /// NP.0F38 map, with W selecting the 32- vs 64-bit form.
    /// AT&T operand order: ops[0] = r/m source, ops[1] = destination.
    pub(crate) fn encode_bmi_blsx(
        &mut self,
        ops: &[Operand],
        ext: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("BMI1 bls* requires 2 operands".to_string());
        }
        let dst = match &ops[1] {
            Operand::Register(r) => r,
            _ => return Err("BMI1 bls*: destination must be a register".to_string()),
        };
        let use_evex = self.apx_nf || self.apx_evex || operands_have_egpr(ops);
        if use_evex {
            return match &ops[0] {
                Operand::Register(src) => {
                    let src_num = reg_num(&src.name).ok_or("bad source register")?;
                    self.emit_apx_evex_vvvv_rr(
                        w != 0,
                        "",
                        &src.name,
                        &dst.name,
                        self.apx_nf,
                        0,
                        2,
                    )?;
                    self.bytes.push(0xF3);
                    self.bytes.push(self.modrm(3, ext, src_num));
                    Ok(())
                }
                Operand::Memory(mem) => {
                    self.emit_apx_evex_vvvv_rm(w != 0, "", mem, &dst.name, self.apx_nf, 0, 2)?;
                    self.bytes.push(0xF3);
                    self.encode_modrm_mem(ext, mem)
                }
                _ => Err("BMI1 bls*: unsupported source operand".to_string()),
            };
        }
        let vvvv = reg_num(&dst.name).ok_or("bad destination register")?
            | (if needs_vex_ext(&dst.name) { 8 } else { 0 });

        match &ops[0] {
            Operand::Register(src) => {
                let src_num = reg_num(&src.name).ok_or("bad source register")?;
                let b = needs_vex_ext(&src.name);
                // VEX.R is unused (the reg field is an opcode extension).
                self.emit_vex(false, false, b, 2, w, vvvv, 0, 0);
                self.bytes.push(0xF3);
                self.bytes.push(self.modrm(3, ext, src_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(false, x, b_ext, 2, w, vvvv, 0, 0);
                self.bytes.push(0xF3);
                self.encode_modrm_mem(ext, mem)
            }
            _ => Err("BMI1 bls*: unsupported source operand".to_string()),
        }
    }

    /// Encode BMI1 ANDN: andnl %src2, %src1, %dst → dst = ~src1 & src2
    /// AT&T operand order: ops[0]=src2(r/m), ops[1]=src1(vvvv), ops[2]=dst(reg)
    pub(crate) fn encode_bmi_andn(&mut self, ops: &[Operand], w: u8) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("andn requires 3 operands".to_string());
        }

        // ops[1] = src1 → VEX.vvvv
        let vvvv_reg = match &ops[1] {
            Operand::Register(r) => r,
            _ => return Err("andn: second operand must be register".to_string()),
        };
        let use_evex = self.apx_nf || self.apx_evex || operands_have_egpr(ops);
        if use_evex {
            return match (&ops[0], &ops[2]) {
                (Operand::Register(src), Operand::Register(dst)) => {
                    let src_num = reg_num(&src.name).ok_or("bad register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    self.emit_apx_evex_vvvv_rr(
                        w != 0,
                        &dst.name,
                        &src.name,
                        &vvvv_reg.name,
                        self.apx_nf,
                        0,
                        2,
                    )?;
                    self.bytes.push(0xF2);
                    self.bytes.push(self.modrm(3, dst_num, src_num));
                    Ok(())
                }
                (Operand::Memory(mem), Operand::Register(dst)) => {
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    self.emit_apx_evex_vvvv_rm(
                        w != 0,
                        &dst.name,
                        mem,
                        &vvvv_reg.name,
                        self.apx_nf,
                        0,
                        2,
                    )?;
                    self.bytes.push(0xF2);
                    self.encode_modrm_mem(dst_num, mem)
                }
                _ => Err("unsupported andn operands".to_string()),
            };
        }
        let vvvv_num = reg_num(&vvvv_reg.name).ok_or("bad vvvv register")?;
        let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv_reg.name) { 8 } else { 0 });

        match (&ops[0], &ops[2]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 2, w, vvvv_enc, 0, 0); // NP.0F38
                self.bytes.push(0xF2);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 2, w, vvvv_enc, 0, 0);
                self.bytes.push(0xF2);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported andn operands".to_string()),
        }
    }

    /// Encode BMI2 rorx (rotate right logical with immediate).
    /// AT&T syntax: `rorxq $imm8, %r/m, %dst`
    ///   ops[0] = immediate
    ///   ops[1] = source r/m
    ///   ops[2] = destination
    pub(crate) fn encode_bmi2_rorx(&mut self, ops: &[Operand], w: u8) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("rorx requires 3 operands".to_string());
        }

        let imm = match &ops[0] {
            Operand::Immediate(ImmediateValue::Integer(val)) => *val as u8,
            _ => return Err("rorx: first operand must be immediate".to_string()),
        };
        if self.apx_nf {
            return Err("{nf} unsupported for `rorx'".to_string());
        }
        let use_evex = self.apx_evex || operands_have_egpr(ops);
        if use_evex {
            return match (&ops[1], &ops[2]) {
                (Operand::Register(src), Operand::Register(dst)) => {
                    let src_num = reg_num(&src.name).ok_or("bad src register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                    self.emit_apx_evex_vvvv_rr(w != 0, &dst.name, &src.name, "rax", false, 3, 3)?;
                    self.bytes.push(0xF0);
                    self.bytes.push(self.modrm(3, dst_num, src_num));
                    self.bytes.push(imm);
                    Ok(())
                }
                (Operand::Memory(mem), Operand::Register(dst)) => {
                    let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                    self.emit_apx_evex_vvvv_rm(w != 0, &dst.name, mem, "rax", false, 3, 3)?;
                    self.bytes.push(0xF0);
                    let rc = self.relocations.len();
                    self.encode_modrm_mem(dst_num, mem)?;
                    self.bytes.push(imm);
                    self.adjust_rip_reloc_addend(rc, 1);
                    Ok(())
                }
                _ => Err("rorx: unsupported operand combination".to_string()),
            };
        }

        match (&ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                // VEX.LZ.F2.0F3A.Wx F0 /r imm8 (mm=3 for 0F3A, pp=3 for F2)
                self.emit_vex(r, false, b, 3, w, 0, 0, 3);
                self.bytes.push(0xF0);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(imm);
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, w, 0, 0, 3);
                self.bytes.push(0xF0);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(imm);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("rorx: unsupported operand combination".to_string()),
        }
    }

    /// AVX 3-op in the 0F map with an explicit pp (0=none,1=66,2=F3,3=F2).
    /// Used by vhaddps (F2), vhaddpd (66), vaddsubps (F2), vaddsubpd (66).
    pub(crate) fn encode_avx_3op_pp(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX 3-op requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad vvvv register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad vvvv register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX 3-op operands".to_string()),
        }
    }

    /// AVX extract-to-GPR with imm8, AT&T ($imm, src, dst); GAS encodes
    /// ModRM.reg = src (xmm), r/m = dst (r32/m32). vextractps: 66.0F3A 17 /r ib.
    pub(crate) fn encode_avx_extract_gpr_imm8(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX extract-gpr requires 3 operands (imm, src, dst)".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };
        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Immediate(ImmediateValue::Integer(imm)), Operand::Register(src), dst) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let r = needs_vex_ext(&src.name);
                let rc_outer = self.relocations.len();
                match dst {
                    Operand::Register(d) => {
                        let dst_num = reg_num(&d.name).ok_or("bad dst register")?;
                        let b = needs_vex_ext(&d.name);
                        self.emit_vex(r, false, b, 3, 0, 0, 0, pp);
                        self.bytes.push(opcode);
                        self.bytes.push(self.modrm(3, src_num, dst_num));
                    }
                    Operand::Memory(mem) => {
                        let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                        let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                        self.emit_vex(r, x, b_ext, 3, 0, 0, 0, pp);
                        self.bytes.push(opcode);
                        self.encode_modrm_mem(src_num, mem)?;
                    }
                    _ => return Err("unsupported extract-gpr destination".to_string()),
                }
                self.bytes.push(*imm as u8);
                // The reg-dst arm above emits no relocation, so capturing the
                // count here (after either arm) and adjusting is a no-op for
                // registers and fixes RIP-relative memory destinations, whose
                // disp32 is otherwise off by the trailing imm8 byte.
                self.adjust_rip_reloc_addend(rc_outer, 1);
                Ok(())
            }
            _ => Err("unsupported AVX extract-gpr operands".to_string()),
        }
    }
}
