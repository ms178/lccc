//! EVEX-promoted SSE/SSE2/CET forms — the `{evex}` encoding hint.
//!
//! Every encoding in this module was byte-probed against GNU as 2.47
//! (`62 ...` rows recorded 2026-09-25 during the ms178-1 red-team audit).
//! The promoted instructions are xmm-only (EVEX.LIG: L'L fixed 00) and
//! take no opmask/zeroing/`{1toN}` decorators; the compressed-displacement
//! scale N follows the memory operand's element size exactly as GAS
//! computes it (`vpextrb` N=1, `vpextrw`/`vpinsrw` N=2, dword forms N=4,
//! qword forms N=8, everything 128-bit N=16).
//!
//! Shape families (AT&T operand order):
//!   * `vmovd`/`vmovq`      — gp/xmm/mem moves (6E into, 7E out of xmm)
//!   * `vextractps`/`vpextr*` — `$imm, %xmm, r/m` (xmm in ModRM.reg)
//!   * `vpinsr*`/`vinsertps`  — `$imm, r/m, %xmm2, %xmm3` (vvvv = src2)
//!   * `vmpsadbw`             — `$imm, %xmm2, %xmm3, %xmm1` (vvvv = src3)
//!   * `vcvt*s2si`            — `%xmm, r/m` (to GP; W from the GP width)
//!   * `vcvtsi2s*`            — `r/m, %xmm2, %xmm3` (from GP; W from the GP)
//!   * `wrss`/`wruss`/`invpcid` — APX map-4 promotions of the CET/system ops

use super::*;

impl super::InstructionEncoder {
    /// Shared promoted-row preamble: no masks, no broadcast, xmm-only.
    fn check_promoted_shape(
        mnemonic: &str,
        ops: &[Operand],
        xmm_slots: &[usize],
    ) -> Result<(), String> {
        Self::evex_forbid_mask_bcst(mnemonic, ops)?;
        for &i in xmm_slots {
            if let Some(Operand::Register(r)) = ops.get(i) {
                if !is_xmm(&r.name) {
                    return Err(format!("operand type mismatch for `{mnemonic}'"));
                }
            }
        }
        Ok(())
    }

    /// `vmovd` / `vmovq`: EVEX.66.0F.W0 (vmovd) / EVEX.F2.0F.W1 (vmovq),
    /// opcode 6E (into xmm) / 7E (out of xmm). The xmm→xmm spelling is the
    /// F3 7E row (pp=2) — same instruction as the legacy 66 0F 7E move.
    pub(crate) fn encode_evex_vmovd_vmovq(
        &mut self,
        ops: &[Operand],
        pp: u8,
        w: u8,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        Self::evex_forbid_mask_bcst(mnemonic, ops)?;
        fn vec_name(op: &Operand) -> Option<&str> {
            match op {
                Operand::Register(r) if is_xmm(&r.name) => Some(&r.name),
                _ => None,
            }
        }
        // AT&T order: (src, dst). Exactly one side must be an xmm.
        match (vec_name(&ops[0]), vec_name(&ops[1])) {
            (None, Some(dst)) => {
                // gp/mem -> xmm: opcode 6E, xmm in ModRM.reg.
                match &ops[0] {
                    Operand::Register(gp) => {
                        let (dst_num, rm_num) =
                            self.emit_evex_mod3(dst, &gp.name, None, 1, w, pp, 0, false, 0, false)?;
                        self.bytes.push(0x6E);
                        self.bytes.push(self.modrm(3, dst_num, rm_num));
                        Ok(())
                    }
                    Operand::Memory(mem) => {
                        let dst_num =
                            self.emit_evex_memop(dst, mem, None, 1, w, pp, 0, false, 0, false)?;
                        self.bytes.push(0x6E);
                        // N = element size: 4 for vmovd (m32), 8 for vmovq (m64).
                        self.encode_evex_mem(dst_num, mem, 4u32 << w)
                    }
                    _ => Err(format!("operand type mismatch for `{mnemonic}'")),
                }
            }
            (Some(src), None) => {
                // xmm -> gp/mem: opcode 7E, xmm in ModRM.reg.
                match &ops[1] {
                    Operand::Register(gp) => {
                        let (dst_num, rm_num) =
                            self.emit_evex_mod3(src, &gp.name, None, 1, w, pp, 0, false, 0, false)?;
                        self.bytes.push(0x7E);
                        self.bytes.push(self.modrm(3, dst_num, rm_num));
                        Ok(())
                    }
                    Operand::Memory(mem) => {
                        let dst_num =
                            self.emit_evex_memop(src, mem, None, 1, w, pp, 0, false, 0, false)?;
                        self.bytes.push(0x7E);
                        self.encode_evex_mem(dst_num, mem, 4u32 << w)
                    }
                    _ => Err(format!("operand type mismatch for `{mnemonic}'")),
                }
            }
            (Some(src), Some(dst)) => {
                // xmm -> xmm: pp flips to F3 (the vmovdqa-family row).
                let (dst_num, rm_num) =
                    self.emit_evex_mod3(dst, src, None, 1, w, 2, 0, false, 0, false)?;
                self.bytes.push(0x7E);
                self.bytes.push(self.modrm(3, dst_num, rm_num));
                Ok(())
            }
            (None, None) => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    /// `vextractps`/`vpextrb`/`vpextrd`/`vpextrq`: `$imm, %xmm, r/m`.
    /// xmm in ModRM.reg, r/m in rm, imm8 last. `vpextrw` has TWO rows:
    /// register destination = EVEX.66.0F.W0 C5, memory destination =
    /// EVEX.66.0F3A.W0 15 (the SSE4.1 PEXTRW row promoted) — GAS 2.47
    /// byte-probed both.
    pub(crate) fn encode_evex_promoted_extract(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
        mem_map: Option<u8>,
        mem_opcode: Option<u8>,
        mem_n: u32,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        Self::check_promoted_shape(mnemonic, ops, &[1])?;
        let imm = match &ops[0] {
            Operand::Immediate(ImmediateValue::Integer(v)) => {
                if !(0..=255).contains(v) {
                    return Err(format!("constant doesn't fit in 8 bits for `{mnemonic}'"));
                }
                *v as u8
            }
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        match (&ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let (dst_num, rm_num) = self
                    .emit_evex_mod3(&src.name, &dst.name, None, map, w, pp, 0, false, 0, false)?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, rm_num));
                self.bytes.push(imm);
                Ok(())
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let (map, opcode) = match (mem_map, mem_opcode) {
                    (Some(m), Some(o)) => (m, o),
                    _ => (map, opcode),
                };
                let dst_num =
                    self.emit_evex_memop(&src.name, mem, None, map, w, pp, 0, false, 0, false)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(dst_num, mem, mem_n)?;
                self.bytes.push(imm);
                Ok(())
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    /// `vpinsrb`/`vpinsrw`/`vpinsrd`/`vpinsrq`/`vinsertps`/`vmpsadbw`:
    /// `$imm, r/m, %xmm2, %xmm3` — dst in ModRM.reg, vvvv = src2 (the
    /// merge-back register), rm = src1 (GP or memory), imm8 last.
    pub(crate) fn encode_evex_promoted_insert(
        &mut self,
        ops: &[Operand],
        map: u8,
        pp: u8,
        w: u8,
        opcode: u8,
        mem_n: u32,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        Self::check_promoted_shape(mnemonic, ops, &[2, 3])?;
        let imm = match &ops[0] {
            Operand::Immediate(ImmediateValue::Integer(v)) => {
                if !(0..=255).contains(v) {
                    return Err(format!("constant doesn't fit in 8 bits for `{mnemonic}'"));
                }
                *v as u8
            }
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        let dst_name = match &ops[3] {
            Operand::Register(r) => &r.name,
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        let vvvv_name = match &ops[2] {
            Operand::Register(r) => &r.name,
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        match &ops[1] {
            Operand::Register(src) => {
                let (dst_num, rm_num) = self.emit_evex_mod3(
                    dst_name,
                    &src.name,
                    Some(vvvv_name),
                    map,
                    w,
                    pp,
                    0,
                    false,
                    0,
                    false,
                )?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, rm_num));
                self.bytes.push(imm);
                Ok(())
            }
            Operand::Memory(mem) => {
                let dst_num = self.emit_evex_memop(
                    dst_name,
                    mem,
                    Some(vvvv_name),
                    map,
                    w,
                    pp,
                    0,
                    false,
                    0,
                    false,
                )?;
                self.bytes.push(opcode);
                self.encode_evex_mem(dst_num, mem, mem_n)?;
                self.bytes.push(imm);
                Ok(())
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    /// `vcvtsd2si`/`vcvtss2si`/`vcvttsd2si`/`vcvttss2si`: `%xmm, r/m` with
    /// the destination a GP register — EVEX.0F.{F2|F3} W{0,1} {2C|2D}, W
    /// taken from the GP width (r32 = W0, r64 = W1), N = 8 (sd) / 4 (ss)
    /// for the memory source.
    pub(crate) fn encode_evex_promoted_cvt_to_gp(
        &mut self,
        ops: &[Operand],
        pp: u8,
        opcode: u8,
        sd: bool,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        Self::check_promoted_shape(mnemonic, ops, &[0])?;
        let w = match &ops[1] {
            Operand::Register(r) => u8::from(is_reg64(&r.name)),
            Operand::Memory(_) => 0,
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let (dst_num, rm_num) =
                    self.emit_evex_mod3(&src.name, &dst.name, None, 1, w, pp, 0, false, 0, false)?;
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, rm_num));
                Ok(())
            }
            (Operand::Register(src), Operand::Memory(mem)) => {
                let dst_num =
                    self.emit_evex_memop(&src.name, mem, None, 1, w, pp, 0, false, 0, false)?;
                self.bytes.push(opcode);
                self.encode_evex_mem(dst_num, mem, if sd { 8 } else { 4 })
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    /// `vcvtsi2sd`/`vcvtsi2ss`: `$gp/mem, %xmm2, %xmm3` — dst in
    /// ModRM.reg, vvvv = src2, rm = the GP source; W from the GP width
    /// (register r64 = W1, r32 or memory = W0), pp = F2 (sd) / F3 (ss).
    /// Memory tuple is element-sized: N = 8 (sd, m64) / 4 (ss, m32).
    pub(crate) fn encode_evex_promoted_cvt_from_gp(
        &mut self,
        ops: &[Operand],
        pp: u8,
        sd: bool,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        Self::check_promoted_shape(mnemonic, ops, &[1, 2])?;
        let w = match &ops[0] {
            Operand::Register(r) => u8::from(is_reg64(&r.name)),
            Operand::Memory(_) => 0,
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        let dst_name = match &ops[2] {
            Operand::Register(r) => &r.name,
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        let vvvv_name = match &ops[1] {
            Operand::Register(r) => &r.name,
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        match &ops[0] {
            Operand::Register(src) => {
                let (dst_num, rm_num) = self.emit_evex_mod3(
                    dst_name,
                    &src.name,
                    Some(vvvv_name),
                    1,
                    w,
                    pp,
                    0,
                    false,
                    0,
                    false,
                )?;
                self.bytes.push(0x2A);
                self.bytes.push(self.modrm(3, dst_num, rm_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                let dst_num = self.emit_evex_memop(
                    dst_name,
                    mem,
                    Some(vvvv_name),
                    1,
                    w,
                    pp,
                    0,
                    false,
                    0,
                    false,
                )?;
                self.bytes.push(0x2A);
                // Tuple1 scalar at N=4 for BOTH widths: GAS 2.47 scales
                // vcvtsi2sd's m64 source by 4 as well (`8(%rax)` -> disp8=2).
                self.encode_evex_mem(dst_num, mem, 4)
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    /// `wrssd`/`wrssq`/`wrussd`/`wrussq` under `{evex}`: the APX map-4
    /// promotion (`EVEX.{66,NP}.map4.W{0,1} {66|65}`) — same register
    /// mapping as the legacy encoder (source in ModRM.reg, shadow-stack
    /// memory in rm).
    pub(crate) fn encode_evex_wrss(
        &mut self,
        ops: &[Operand],
        w: bool,
        pp: u8,
        opcode: u8,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        Self::evex_forbid_mask_bcst(mnemonic, ops)?;
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Memory(mem)) => {
                self.emit_apx_evex_rm_pp(w, &src.name, mem, None, false, pp)?;
                self.bytes.push(opcode);
                self.encode_modrm_mem(reg_num(&src.name).ok_or("bad register")? & 7, mem)
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    /// `invpcid` under `{evex}`: EVEX.F3.map4.W0 F2 — (m, r) only, exactly
    /// like the legacy form (GAS 2.47 rejects the (r, m) shape with
    /// `operand type mismatch').
    pub(crate) fn encode_evex_invpcid(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        Self::evex_forbid_mask_bcst(mnemonic, ops)?;
        match (&ops[0], &ops[1]) {
            (Operand::Memory(mem), Operand::Register(dst)) => {
                self.emit_apx_evex_rm_pp(false, &dst.name, mem, None, false, 2)?;
                self.bytes.push(0xF2);
                self.encode_modrm_mem(reg_num(&dst.name).ok_or("bad register")? & 7, mem)
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }
}

/// The legacy mnemonics that have an APX map-4 promotion, matched against
/// the suffixed spellings the central suffix inference produces
/// (`"cmpl"` → `Some(("cmp", 4))`, `"cmp"` → `Some(("cmp", 0))`; size 0
/// means "infer from the operands"). `"mul"`/`"imul"`/`"shl"`/`"sal"` end
/// in a suffix-shaped letter, so a generic strip would mis-split them —
/// the stem set is explicit instead.
pub(crate) fn promoted_stem_size(m: &str) -> Option<(&'static str, u8)> {
    const STEMS: &[&str] = &[
        "cmp", "test", "add", "or", "adc", "sbb", "and", "sub", "xor", "neg", "not", "inc", "dec",
        "mul", "div", "idiv", "imul", "rol", "ror", "rcl", "rcr", "shl", "sal", "shr", "sar",
    ];
    for &stem in STEMS {
        if m == stem {
            return Some((stem, 0));
        }
        for (sfx, size) in [('q', 8u8), ('l', 4), ('w', 2), ('b', 1)] {
            if m.len() == stem.len() + 1 && m.starts_with(stem) && m.ends_with(sfx) {
                return Some((stem, size));
            }
        }
    }
    None
}
