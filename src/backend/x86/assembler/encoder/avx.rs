use super::*;

/// Intel SDM Vol.2: `{er}` (embedded rounding, implies SAE) vs `{sae}`
/// (SAE without rounding). GAS 2.47 rejects the wrong decorator per mnemonic.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EvexSae {
    None,
    Er,
    Sae,
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
            (1, 0x58 | 0x59 | 0x5C | 0x5E | 0x51 | 0x5B) => EvexSae::Er, // add/mul/sub/div/sqrt/cvt
            (1, 0x5D | 0x5F | 0xC2) => EvexSae::Sae,                     // min/max/vcmp
            (2, 0x96..=0x9F) | (2, 0xA6..=0xAF) | (2, 0xB6..=0xBF) => EvexSae::Er, // FMA + fmaddsub
            _ => EvexSae::None,
        }
    }

    fn apply_evex_sae(
        sae: Option<Option<u8>>,
        class: EvexSae,
        vl_ll: u8,
    ) -> Result<(bool, u8), String> {
        match sae {
            None => Ok((false, vl_ll)),
            Some(tok) => match class {
                EvexSae::None => Err("unsupported static rounding/sae".to_string()),
                EvexSae::Er => match tok {
                    Some(rc) => Ok((true, rc)),
                    None => Err("unsupported static rounding/sae".to_string()),
                },
                EvexSae::Sae => match tok {
                    None => Ok((true, 0)),
                    Some(_) => Err("unsupported static rounding/sae".to_string()),
                },
            },
        }
    }

    /// EVEX vector length from operands: 00=128(xmm), 01=256(ymm), 10=512(zmm).
    fn evex_ll(ops: &[Operand]) -> u8 {
        for op in ops {
            if let Operand::Register(r) = op {
                let name = r.name.to_lowercase();
                if name.starts_with("zmm") {
                    return 0b10;
                }
                if name.starts_with("ymm") {
                    return 0b01;
                }
            }
        }
        0b00
    }

    /// Strip a leading `{sae}` / `{r*-sae}` AT&T decorator operand.
    /// `None` = no decorator; `Some(None)` = bare `{sae}`; `Some(Some(rc))` = `{r*-sae}`.
    fn peel_evex_sae(ops: &[Operand]) -> (&[Operand], Option<Option<u8>>) {
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
        let (ops, sae) = Self::peel_evex_sae(ops);
        if ops.len() != 3 {
            return Err("EVEX binary op requires 3 operands".to_string());
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
                let (mem_bcst, scale_n) = Self::evex_mem_scale(mem, vl_ll, 1);
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

    /// EVEX 3-source + imm8, AT&T ($imm, src2, src1, dst) with optional mask on dst.
    /// vpternlogd/q (0F3A 25), vpalignr (0F3A 0F), vpclmulqdq (0F3A 44),
    /// vinserti32x4/i64x2 (0F3A 38), vpshld*/vpshrd* (0F3A 70-73).
    /// GAS convention: ModRM.reg = dest, r/m = src2, vvvv = src1.
    pub(crate) fn encode_evex_3src_imm(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
    ) -> Result<(), String> {
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
                    false,
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
                // everything else is Full VL (or {1toN} broadcast).
                let tuple_n = match opcode {
                    0x38 => 16u32,
                    0x3A => 32,
                    _ => Self::evex_vl_bytes(ll),
                };
                let (mem_bcst, scale_n) = if mem.broadcast.is_some() {
                    Self::evex_mem_scale(mem, ll, 1)
                } else {
                    (false, tuple_n)
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
                let (_, src2_num) = self.emit_evex_mod3(
                    &kdst.name,
                    &src2.name,
                    Some(&src1.name),
                    map,
                    w,
                    pp,
                    ll,
                    false,
                    0,
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
                let k_num = self.emit_evex_memop(
                    &kdst.name,
                    mem,
                    Some(&src1.name),
                    map,
                    w,
                    pp,
                    ll,
                    false,
                    0,
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
                let (_, src2_num) = self.emit_evex_mod3(
                    &kdst.name,
                    &src2.name,
                    Some(&src1.name),
                    map,
                    w,
                    pp,
                    ll,
                    false,
                    0,
                    false,
                )?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, k_num, src2_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(src1), Operand::Register(kdst)) => {
                let k_num = reg_num(&kdst.name).ok_or("bad k-dest register")?;
                let (bcst, scale_n) = Self::evex_mem_scale(mem, ll, 1);
                let _ = self.emit_evex_memop(
                    &kdst.name,
                    mem,
                    Some(&src1.name),
                    map,
                    w,
                    pp,
                    ll,
                    false,
                    0,
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
                    0x5A => 16u32,
                    0x5B => 32,
                    _ => [16u32, 32, 64][ll as usize],
                };
                self.encode_evex_mem(dst_num, mem, n)
            }
            _ => Err("unsupported EVEX mem-broadcast operands".to_string()),
        }
    }

    /// Determine EVEX L'L from operands: 00=128(xmm), 01=256(ymm), 10=512(zmm)
    pub(crate) fn evex_ll_from_ops(&self, ops: &[Operand]) -> u8 {
        for op in ops {
            if let Operand::Register(r) = op {
                let name = r.name.to_lowercase();
                if name.starts_with("zmm") {
                    return 0b10;
                }
                if name.starts_with("ymm") {
                    return 0b01;
                }
            }
        }
        0b00 // default to 128-bit
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
                    false,
                    0,
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
                    false,
                    0,
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
            _ => Err("unsupported AVX 3-op operands".to_string()),
        }
    }

    /// Encode a 0F3A-map AVX instruction with an imm8 where AT&T operands are
    /// (imm, src2, src1, dst) — vpclmulqdq. vvvv = src1 (NDS first source),
    /// r/m = src2, modrm.reg = dst.
    pub(crate) fn encode_avx_3op_3a_pp_imm8(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
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
                self.emit_vex(r, false, b, 3, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src2_num));
                self.bytes.push(imm);
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
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(imm);
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
            // Gas-style AT&T sometimes writes `vfmadd231sd %xmm, mem, %xmm`
            // (src1, src2/mem, dest). Map to the canonical (mem, vvvv, dst) form
            // when the middle operand is memory.
            (Operand::Register(src1), Operand::Memory(mem), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&src1.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&src1.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 2, 1, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
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
            // Gas-style AT&T sometimes writes `vfmadd231sd %xmm, mem, %xmm`
            // (src1, src2/mem, dest). Map to the canonical (mem, vvvv, dst) form
            // when the middle operand is memory.
            (Operand::Register(src1), Operand::Memory(mem), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&src1.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&src1.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 2, 1, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported FMA3 3-op operands".to_string()),
        }
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
        if ops.len() != 2 {
            return Err("AVX 2-op requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

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
                    (Operand::Register(src), Operand::Register(dst))
                        if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
                    {
                        // reg-reg: use load form
                        let src_num = reg_num(&src.name).ok_or("bad register")?;
                        let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                        let r = needs_vex_ext(&dst.name);
                        let b = needs_vex_ext(&src.name);
                        self.emit_vex(r, false, b, 1, 0, 0, 0, pp);
                        self.bytes.push(load_op);
                        self.bytes.push(self.modrm(3, dst_num, src_num));
                        Ok(())
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

    /// Encode AVX pshufd-like (imm8 + 2 register operands)
    pub(crate) fn encode_avx_shuffle(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX shuffle requires 3 operands".to_string());
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
            // GP -> XMM: VEX.128.66.0F 6E /r
            (Operand::Register(src), Operand::Register(dst))
                if !is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, 0, 1);
                self.bytes.push(0x6E);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // XMM -> GP: VEX.128.66.0F 7E /r
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && !is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b = needs_vex_ext(&dst.name);
                self.emit_vex(r, false, b, 1, 0, 0, 0, 1);
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
                // $imm, %xmm_src, %xmm_dst  (immediate shift, dst = vvvv)
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
                self.encode_modrm_mem(dst_num, mem)?;
                let mask_full = mask_num | (if needs_vex_ext(&mask.name) { 8 } else { 0 });
                self.bytes.push((mask_full & 0xF) << 4);
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
            _ => Err("unsupported AVX insert operands".to_string()),
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

        let dst_num = reg_num(&dst.name).ok_or("bad destination register")?;
        let vvvv = reg_num(&mask.name).ok_or("bad mask register")?
            | (if needs_vex_ext(&mask.name) { 8 } else { 0 });
        let l = u8::from(is_ymm(&dst.name));
        let r = needs_vex_ext(&dst.name);
        let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
        let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));

        self.emit_vex(r, x, b_ext, 2, w, vvvv, l, 1);
        self.bytes.push(opcode);
        self.encode_modrm_mem(dst_num, mem)
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
                Ok(())
            }
            _ => Err("unsupported AVX extract-gpr operands".to_string()),
        }
    }
}
