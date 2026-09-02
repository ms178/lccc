//! VEX-encoded instruction support for the i686 integrated assembler.
//!
//! Ported from the x86-64 backend's `avx.rs` (the VEX half; AVX-512/EVEX is
//! deliberately NOT ported — 32-bit code has no zmm registers and the
//! regression corpus never reaches for it).  Everything is structurally
//! identical to the x86-64 encoder except:
//!   * `needs_vex_ext` is constant `false` (there are no r8-r15, so VEX.R/X/B
//!     are always "unused" = 1).  The branching is kept so the two encoders
//!     cannot drift.
//!   * no RIP-relative memory operands reach these paths from the i686
//!     intrinsic emitters, so the rip-reloc addend fixup is not needed.
//!
//! Soundness notes preserved from the x86-64 port:
//!   * `encode_avx_3op_commutative` swaps the two sources ONLY for
//!     commutative integer/bitwise ops (2-byte VEX win).  FP add/mul are NOT
//!     bit-commutative on x86 (NaN payload comes from SRC1), so FP mnemonics
//!     must not be registered with `commutative = true`.
//!   * vhaddps/vhsubps/vaddsubps use the F2 prefix (VEX pp=3), vhaddpd/vhsubpd
//!     66 (pp=1) — the historical pp=2 mixup encoded illegal instructions.
//!   * Two-operand 0F3A forms (vroundps) must pass vvvv=0 (the "unused" 1111
//!     after inversion), not the destination register.

use super::*;

/// VEX.R/X/B extension bits: i686 has no extended registers, so the bits are
/// never set.  Kept as a function so the port cannot drift from x86-64.
#[inline]
fn needs_vex_ext(_name: &str) -> bool {
    false
}

/// Is this an YMM register?
fn is_ymm(name: &str) -> bool {
    name.starts_with("ymm")
}

impl super::InstructionEncoder {
    /// Emit a 2-byte or 3-byte VEX prefix.
    /// pp: 0=none, 1=66, 2=F3, 3=F2; mm: 1=0F, 2=0F38, 3=0F3A
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

        // 2-byte VEX is only possible when the operand map is 0F and neither
        // VEX.W nor the X/B extension bits are needed.
        if mm == 1 && w == 0 && !x && !b {
            self.bytes.push(0xC5);
            let byte2 = (r_bit << 7) | (vvvv_inv << 3) | (l << 2) | pp;
            self.bytes.push(byte2);
        } else {
            self.bytes.push(0xC4);
            let byte1 = (r_bit << 7) | (x_bit << 6) | (b_bit << 5) | mm;
            let byte2 = (w << 7) | (vvvv_inv << 3) | (l << 2) | pp;
            self.bytes.push(byte1);
            self.bytes.push(byte2);
        }
    }

    /// VEX.L bit: 1 when any register operand is a YMM register.
    pub(crate) fn vex_l_from_ops(&self, ops: &[Operand]) -> u8 {
        for op in ops {
            match op {
                Operand::Register(r) if is_ymm(&r.name) => return 1,
                _ => {}
            }
        }
        0
    }

    /// vmovdqa (66) / vmovdqu (F3) load/store.
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
        let pp = if is_66 { 1 } else { 2 };
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst))
                if (is_xmm(&src.name) && is_xmm(&dst.name))
                    || (is_ymm(&src.name) && is_ymm(&dst.name)) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
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

    /// vmovaps/vmovapd/vmovups/vmovupd (no mandatory prefix / 66).
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

    /// Store-only VEX form (vmovntps/vmovntpd/vmovntdq).
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

    /// VEX.NDS 3-operand, no mandatory prefix (vaddps family).
    pub(crate) fn encode_avx_3op_np(
        &mut self,
        ops: &[Operand],
        opcode: u8,
    ) -> Result<(), String> {
        self.encode_avx_3op(ops, opcode, false)
    }

    /// VEX.NDS 3-operand: `op src, vvvv, dst` with a 0F map and optional 66.
    pub(crate) fn encode_avx_3op(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        self.encode_avx_3op_commutative(ops, opcode, has_66, false)
    }

    /// VEX.NDS 3-operand with explicit pp (vhaddps pp=3, vhaddpd pp=1).
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
            _ => Err("unsupported AVX 3-op operands".to_string()),
        }
    }

    /// VEX.NDS 3-operand, optionally exploiting commutativity for the 2-byte
    /// VEX prefix (see the x86-64 encoder's NaN-payload warning: only integer
    /// and bitwise ops may be registered commutative).
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
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
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

    /// VEX.NDS 3-operand in the 0F38 map.
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

    /// VEX.NDS 3-operand in the 0F38 map with explicit pp (VNNI family).
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

    /// VEX.NDS 3-operand in the 0F38 map with W=1 (double FMA).
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
                self.emit_vex(r, false, b, 2, 1, vvvv_enc, l, pp);
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
                self.emit_vex(r, x, b_ext, 2, 1, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
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

    /// 0F3A-map VEX with imm8 where AT&T operands are (imm, src, vvvv, dst).
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
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported AVX 3-op+imm8 operands".to_string()),
        }
    }

    /// 0F3A-map VEX 2-operand with imm8: (imm, src, dst), vvvv unused (0).
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
            Operand::Immediate(ImmediateValue::Integer(i)) => *i as u8,
            _ => return Err("AVX 3A imm must be an immediate".to_string()),
        };
        match (&ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
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

    /// 0F-map VEX 3-operand with imm8 (vshufps/vshufpd): (imm, src, vvvv, dst).
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
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported AVX 3-op+imm8 operands".to_string()),
        }
    }

    /// 0F38-map VEX 2-operand (vpabs*, vpmovzx*, vptest): (src, dst), vvvv=0.
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
                self.emit_vex(r, false, b, 2, 0, 0, l, pp);
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

    /// 0F-map VEX 2-operand (vmovddup, vsqrtps, vcvtps2pd, ...).
    /// pp: 0=NP, 1=66, 2=F3, 3=F2.
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
                self.emit_vex(r, false, b, 1, 0, 0, l, pp);
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

    /// Variable-count shifts and vpsllw/d/q reg forms: (count, src, dst) or
    /// ($imm, src, dst) with dst doubling as vvvv.
    pub(crate) fn encode_avx_shift(
        &mut self,
        ops: &[Operand],
        reg_op: u8,
        imm_ext: u8,
        imm_op: u8,
        has_66: bool,
    ) -> Result<(), String> {
        let pp = if has_66 { 1 } else { 0 };
        if ops.len() != 3 {
            return Err("AVX shift requires 3 operands".to_string());
        }
        match (&ops[0], &ops[1], &ops[2]) {
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
    }

    /// 0F3A-map shuffle with imm8 (vpshufd/vpermilps imm forms): (imm, src, dst).
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
                self.emit_vex(r, false, b, 3, 0, 0, l, pp);
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
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported AVX shuffle 3A operands".to_string()),
        }
    }

    /// vpermq/vpermpd (0F3A map with W=1): (imm, src, dst).
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
                self.emit_vex(r, false, b, 3, 1, 0, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported AVX permq operands".to_string()),
        }
    }

    /// vpshufd (66 0F 70): (imm, src, dst).
    pub(crate) fn encode_avx_shuffle(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        self.encode_avx_shuffle_3a(ops, opcode, has_66)
    }

    /// vextracti128/vextractf128: ($imm, %ymm_src, %xmm_or_mem_dst), L=1.
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
                self.emit_vex(r, false, b, 3, 0, 0, 1, pp);
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
                self.encode_modrm_mem(src_num, mem)?;
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported AVX extract operands".to_string()),
        }
    }

    /// xmm/ymm -> GPR extraction (vpmovmskb, vmovmskps/pd).
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

    /// vextractps: ($imm, %xmm, gpr_or_mem), 128-bit only.
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

    /// vblendvps/vblendvpd/vpblendvb — VEX /is4: AT&T (%mask, %src, %vvvv, %dst),
    /// mask register encoded in imm8[7:4].
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
                let mask_full = mask_num | (if needs_vex_ext(&mask.name) { 8 } else { 0 });
                self.bytes.push((mask_full & 0xF) << 4);
                Ok(())
            }
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

    /// vbroadcastss/vbroadcastsd/vpbroadcast* (register or memory source).
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

    /// vpbroadcast b/w/d/q from a GPR source (VEX.128/256.66.0F38 7A-7C /r).
    /// The destination's L bit picks 128 vs 256-bit form.
    pub(crate) fn encode_avx_broadcast_gpr_vex(
        &mut self,
        ops: &[Operand],
        opcode: u8,
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
                let l = if is_ymm(&dst.name) { 1 } else { 0 };
                // vvvv must be all-ones (unused): pass 0 to emit_vex.
                self.emit_vex(r, false, false, 2, 0, 0, l, 1);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            _ => Err("unsupported vpbroadcast-gpr operands".to_string()),
        }
    }

    /// vpbroadcast b/w/d/q: GPR source uses the 0F38 7A-7C VEX form, an
    /// xmm/mem source uses the broadcast form (0F38 78/79/58/59).
    pub(crate) fn encode_vpbroadcast(
        &mut self,
        ops: &[Operand],
        broadcast_op: u8,
        gpr_op: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("vpbroadcast requires 2 operands".to_string());
        }
        match &ops[0] {
            Operand::Register(r) if !is_xmm_or_ymm(&r.name) => {
                self.encode_avx_broadcast_gpr_vex(ops, gpr_op)
            }
            _ => self.encode_avx_broadcast(ops, &[broadcast_op]),
        }
    }

    /// vpslldq/vpsrldq — VEX.0F 73 /r ib with the shift kind in the reg
    /// field: (imm, src, dst), dst doubling as vvvv (same shape as the
    /// immediate AVX shift forms).
    pub(crate) fn encode_avx_3op_0f_imm8_shift(
        &mut self,
        ops: &[Operand],
        imm_op: u8,
        imm_ext: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX shift requires 3 operands".to_string());
        }
        match (&ops[0], &ops[1], &ops[2]) {
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
                self.emit_vex(false, false, b, 1, 0, vvvv_enc, l, 1);
                self.bytes.push(imm_op);
                self.bytes.push(self.modrm(3, imm_ext, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported AVX shift operands".to_string()),
        }
    }

    /// 0F3A-map VEX with explicit pp and imm8 where AT&T operands are
    /// (imm, src2, src1, dst) — vpclmulqdq. vvvv = src1, r/m = src2.
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
            Operand::Immediate(ImmediateValue::Integer(i)) => *i as u8,
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

    /// Scalar 3-operand VEX (vmulss/vaddss/vfmadd*ss/sd): pp 2=F3, 3=F2.
    pub(crate) fn encode_avx_scalar_3op(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        let ops: &[Operand] = if ops.len() == 2 {
            &[ops[0].clone(), ops[1].clone(), ops[1].clone()]
        } else {
            ops
        };
        if ops.len() != 3 {
            return Err("AVX scalar 3-op requires 2 or 3 operands".to_string());
        }
        let l = 0;

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

    /// Scalar load/store/merge (vmovss/vmovsd): pp 2=F3, 3=F2.
    pub(crate) fn encode_avx_scalar_mov(
        &mut self,
        ops: &[Operand],
        load_op: u8,
        store_op: u8,
        pp: u8,
    ) -> Result<(), String> {
        match ops.len() {
            2 => {
                match (&ops[0], &ops[1]) {
                    (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                        let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                        let r = needs_vex_ext(&dst.name);
                        let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                        let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                        self.emit_vex(r, x, b_ext, 1, 0, 0, 0, pp);
                        self.bytes.push(load_op);
                        self.encode_modrm_mem(dst_num, mem)
                    }
                    (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                        let src_num = reg_num(&src.name).ok_or("bad register")?;
                        let r = needs_vex_ext(&src.name);
                        let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                        let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                        self.emit_vex(r, x, b_ext, 1, 0, 0, 0, pp);
                        self.bytes.push(store_op);
                        self.encode_modrm_mem(src_num, mem)
                    }
                    (Operand::Register(src), Operand::Register(dst))
                        if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
                    {
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
            3 => self.encode_avx_scalar_3op(ops, load_op, pp),
            _ => Err("AVX scalar mov requires 2 or 3 operands".to_string()),
        }
    }

    /// vmovd (GPR <-> XMM, 32-bit on i686).
    pub(crate) fn encode_avx_movd(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("vmovd requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
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
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, 0, 1);
                self.bytes.push(0x6E);
                self.encode_modrm_mem(dst_num, mem)
            }
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

    /// Scalar GP<->XMM conversions (vcvtss2si/vcvtsi2sd, ...).
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
        let w = 0u8; // no 64-bit GPRs on i686
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

    /// GPR -> XMM scalar conversion with NDS merge (vcvtsi2ss/vcvtsi2sd).
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

    /// FMA3: all VEX forms share the regular 0F38 grid (pp is always 66).
    pub(crate) fn encode_fma3_vex(&mut self, ops: &[Operand], m: &str) -> Result<(), String> {
        let (opcode, w) = fma3_opcode(m).ok_or("bad FMA3 mnemonic")?;
        let l = self.vex_l_from_ops(ops);
        if ops.len() != 3 {
            return Err("FMA3 requires 3 operands".to_string());
        }
        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 2, w, vvvv_enc, l, 1);
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
                self.emit_vex(r, x, b_ext, 2, w, vvvv_enc, l, 1);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported FMA3 operands".to_string()),
        }
    }
}
