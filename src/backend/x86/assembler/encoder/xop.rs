//! XOP (AMD eXtended Operations) and LWP (Lightweight Profiling) encoders.
//!
//! Both instruction families share the `0x8F` escape byte with a VEX-like
//! layout — but with a 5-BIT map field and two quirks the VEX emitters do
//! not have:
//!
//! * byte1 = `R X B mmmmm` — the extension bits are inverted, exactly like
//!   VEX, but `mmmmmm` covers maps 8/A (XOP) and A (LWP);
//! * byte2 = `V' vvvv L pp` — same field layout as VEX byte 2;
//! * the four-operand forms (`vpcmov`, `vpperm`, `vpmac*`) have FOUR
//!   register sources but only THREE register slots. AMD's answer: the
//!   imm8's HIGH NIBBLE carries the fourth register number, so the
//!   register-selector spelling always ends with `imm8 = reg(op1) << 4`.
//!   When the FIRST operand is memory (`vpcmov (%rax), %xmm2, %xmm3,
//!   %xmm4`) the memory operand takes the r/m slot, the imm8 disappears,
//!   and GAS encodes the 3-register form (verified byte-for-byte — the
//!   testsuite pins both shapes).
//!
//! Every opcode below was distilled from GAS 2.47 with
//! `scripts/distill_evex_opcodes.py`; the XOP/LWP instruction space is
//! 128-bit only (vprot*/vph*/vpcmov reject ymm operands at the source —
//! `vpcmov %ymm1, %ymm2, %ymm3, %ymm4` is the single ymm-capable member,
//! with L = 1).

use super::*;

/// XOP map field values (byte1 bits 4:0).
const XOP_MAP_8: u8 = 0x08;
const XOP_MAP_9: u8 = 0x09;
/// LWP lives in its own map.
const XOP_MAP_A: u8 = 0x0A;

/// Emit the 3-byte XOP/LWP prefix (`8F` + map byte + opcode-adjacent byte).
///
/// `r`/`x`/`b` are the *inverted* extension bits in VEX style: pass
/// `needs_vex_ext(reg)` for whichever of dst-reg / mem-index / mem-rm the
/// instruction touches. `vvvv` is the raw (non-inverted) 4-bit register
/// number of the NDS source, or 0 when unused; `v_prime` covers register
/// ids 16–31 (never legal for XOP/LWP hardware, but the field exists).
impl InstructionEncoder {
    fn emit_xop_prefix(
        &mut self,
        r: bool,
        x: bool,
        b: bool,
        vvvv: u8,
        v_prime: bool,
        l: u8,
        map: u8,
    ) {
        let r_bit = if r { 0 } else { 1 };
        let x_bit = if x { 0 } else { 1 };
        let b_bit = if b { 0 } else { 1 };
        let vvvv_inv = (!vvvv) & 0xF;
        // NOTE: V' is a PLAIN extension bit (V'=1 ⇔ vvvv id ≥ 16), not an
        // inverted field like R/X/B.
        let v_prime_bit = if v_prime { 1 } else { 0 };
        self.bytes.push(0x8F);
        self.bytes.push((r_bit << 7) | (x_bit << 6) | (b_bit << 5) | map);
        self.bytes.push((v_prime_bit << 7) | (vvvv_inv << 3) | (l << 2));
    }
}

/// XOP register id: xmm/ymm only (0–15), extended via the V'-covered
/// 4-bit field. zmm/k/GP are not XOP registers.
fn xop_vec_id(name: &str) -> Option<u8> {
    if let Some(v) = vec_reg_id(name) {
        return Some(v);
    }
    None
}

fn xop_vvvv(name: &str) -> Result<(u8, bool), String> {
    match xop_vec_id(name) {
        Some(id) => Ok((id & 0xF, id & 16 != 0)),
        None => Err(format!("bad XOP register: {name}")),
    }
}

impl InstructionEncoder {
    /// Is this mnemonic an XOP/LWP instruction (0x8F escape)?
    pub(crate) fn is_xop_mnemonic(mnemonic: &str) -> bool {
        matches!(
            mnemonic,
            "vpcmov"
                | "vpperm"
                | "vprotb"
                | "vprotw"
                | "vprotd"
                | "vprotq"
                // XOP horizontal arithmetic (map 9, unary 2-op).
                | "vphaddubw"
                | "vphaddbw"
                | "vphaddubq"
                | "vphaddbq"
                | "vphaddubd"
                | "vphaddbd"
                | "vphadduwd"
                | "vphaddwd"
                | "vphadduwq"
                | "vphaddwq"
                | "vphaddudq"
                | "vphadddq"
                | "vphsubbw"
                | "vphsubwd"
                | "vphsubdq"
                // XOP vfrcz* (map 9, unary 2-op, xmm+ymm).
                | "vfrczps"
                | "vfrczpd"
                | "vfrczss"
                | "vfrczsd"
                // XOP shifts (map 9, binary 3-op, xmm only).
                | "vpshab"
                | "vpshaw"
                | "vpshad"
                | "vpshaq"
                | "vpshlb"
                | "vpshlw"
                | "vpshld"
                | "vpshlq"
                // XOP FMA4-integer (map 8, 4-op with imm8-nibble).
                | "vpmacssww"
                | "vpmacsww"
                | "vpmacsswd"
                | "vpmacswd"
                | "vpmacssdd"
                | "vpmacsdd"
                | "vpmacssdql"
                | "vpmacsdql"
                | "vpmacssdqh"
                | "vpmacsdqh"
                // LWP (map A).
                | "lwpins"
                | "lwpval"
        )
    }

    /// LWP `lwpins`/`lwpval`: `$imm32, r/m32, vvvv-reg`. The instruction
    /// class rides the ModRM.reg field (0 = lwpins, 1 = lwpval); the
    /// destination-ish register rides vvvv (inverted, V'-extended).
    fn encode_lwp(
        &mut self,
        ops: &[Operand],
        reg_field: u8,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err(format!(
                "number of operands mismatch for `{mnemonic}'"
            ));
        }
        let imm = match &ops[0] {
            Operand::Immediate(ImmediateValue::Integer(v)) => *v,
            _ => return Err(format!("LWP requires an immediate first operand")),
        };
        // GAS 2.47: vector registers in either register slot are
        // `operand type mismatch for `lwpval'' (byte-probed).
        if ops.iter().skip(1).any(
            |op| matches!(op, Operand::Register(r) if is_xmm(&r.name) || is_ymm(&r.name) || is_zmm(&r.name)),
        ) {
            return Err(format!("operand type mismatch for `{mnemonic}'"));
        }
        let (vvvv, v_prime) = match &ops[2] {
            Operand::Register(r) => {
                let id = gp_id(&r.name)
                    .ok_or_else(|| format!("bad LWP register: {}", r.name))?;
                // GAS 2.47 (byte-verified): V' is NOT a register-extension
                // bit here — it flags the 64-bit destination spelling
                // (`lwpins $imm, %ebx, %r10d` → V'=0; `lwpins $imm, %eax,
                // %r8` → V'=1 with the same 4-bit vvvv number).
                (id & 0xF, is_reg64(&r.name))
            }
            _ => {
                return Err(format!(
                    "operand type mismatch for `{mnemonic}'"
                ))
            }
        };
        let imm_le = (imm as u32).to_le_bytes();
        match &ops[1] {
            Operand::Register(src) => {
                let src_num =
                    reg_num(&src.name).ok_or_else(|| format!("bad register: {}", src.name))?;
                let b = needs_vex_ext(&src.name);
                self.emit_xop_prefix(false, false, b, vvvv, v_prime, 0, XOP_MAP_A);
                self.bytes.push(0x12);
                self.bytes.push(self.modrm(3, reg_field, src_num));
                self.bytes.extend_from_slice(&imm_le);
                Ok(())
            }
            Operand::Memory(mem) => {
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_xop_prefix(false, x, b_ext, vvvv, v_prime, 0, XOP_MAP_A);
                self.bytes.push(0x12);
                self.encode_modrm_mem(reg_field, mem)?;
                self.bytes.extend_from_slice(&imm_le);
                Ok(())
            }
            _ => Err("unsupported LWP operands".to_string()),
        }
    }

    /// Four-operand XOP (map 8): `vpcmov`/`vpperm` (opcode 0xA2/0xA3) and
    /// the `vpmac*` family. AT&T operand order: (src1|mem, src2, sel, dst).
    ///
    /// * All-register form: `reg` = dst, `r/m` = src2, `vvvv` = sel, and
    ///   the imm8's high nibble carries src1's register number
    ///   (`imm8 = reg(op1) << 4`).
    /// * Memory-first form: the memory operand takes r/m, the imm8
    ///   disappears (GAS encodes the 3-slot reading; verified byte-for-byte
    ///   against GAS 2.47 — the testsuite pins both shapes).
    /// * Memory in the src2 or selector slot: rejected, exactly like GAS
    ///   (`vpcmov %xmm1, %xmm2, (%rax), %xmm4` is not a form).
    fn encode_xop_4op(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        // Selector (op3) and destination (op4) are always registers.
        let (vvvv, v_prime) = match &ops[2] {
            Operand::Register(r) => xop_vvvv(&r.name)?,
            _ => {
                return Err(format!(
                    "unsupported {mnemonic} operands: memory in the selector slot"
                ))
            }
        };
        let dst_name = match &ops[3] {
            Operand::Register(r) => &r.name,
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        // Same-width rule. Byte-probed against GAS 2.47: vpcmov is the
        // ONLY ymm-capable 4-op member (mixed widths there are `register
        // type mismatch'), while vpperm and the vpmac* family are
        // xmm-only — any ymm operand is `operand size mismatch'.
        let ymm_capable = opcode == 0xA2; // vpcmov only
        let width_of = |op: &Operand| -> Option<u8> {
            match op {
                Operand::Register(r) => {
                    if is_xmm(&r.name) {
                        Some(0)
                    } else if is_ymm(&r.name) {
                        Some(1)
                    } else {
                        None
                    }
                }
                Operand::Memory(_) => None,
                _ => Some(2),
            }
        };
        let widths: Vec<Option<u8>> = ops.iter().map(width_of).collect();
        if !ymm_capable
            && widths
                .iter()
                .any(|w| *w == Some(1) || *w == Some(2))
        {
            return Err(format!("operand size mismatch for `{mnemonic}'"));
        }
        let reg_widths: Vec<u8> = widths.iter().filter_map(|w| *w).collect();
        if reg_widths.iter().any(|w| *w != reg_widths[0]) {
            return Err(format!("register type mismatch for `{mnemonic}'"));
        }
        let l = reg_widths.first().copied().unwrap_or(0);
        match &ops[0] {
            // All-register form: the imm8's high nibble carries src1.
            Operand::Register(src1) => {
                let src1_num = xop_vec_id(&src1.name)
                    .ok_or_else(|| format!("bad XOP register: {}", src1.name))?;
                match &ops[1] {
                    Operand::Register(r) => {
                        let src2_name = &r.name;
                        let src2_num = xop_vec_id(src2_name)
                            .ok_or_else(|| format!("bad XOP register: {src2_name}"))?;
                        let dst_num = xop_vec_id(dst_name)
                            .ok_or_else(|| format!("bad XOP register: {dst_name}"))?;
                        let r = needs_vex_ext(dst_name);
                        let b = needs_vex_ext(src2_name);
                        self.emit_xop_prefix(r, false, b, vvvv, v_prime, l, XOP_MAP_8);
                        self.bytes.push(opcode);
                        self.bytes.push(self.modrm(3, dst_num, src2_num));
                        self.bytes.push(src1_num << 4);
                        Ok(())
                    }
                    Operand::Memory(mem) => {
                        // Memory-src2 form (GAS 2.47, byte-verified for all
                        // three 4-op families — vpcmov/vpperm/vpmac*):
                        // `vpcmov %xmm0, (%rax), %xmm2, %xmm4` encodes as
                        // `8f e8 68 a2 20 00`. Same hardware layout as the
                        // all-register form — reg = dst, r/m = src2 (here
                        // the memory), vvvv = src3 (ops[2], the selector) —
                        // but the slot-less register is now src1 (ops[0]),
                        // whose number rides imm8[7:4]: `20` = xmm0 << 4.
                        // V' keeps its plain vvvv-extension meaning (0 for
                        // every byte-verified spelling, unlike the
                        // memory-first form's pinned V'=1 below). The
                        // vector-index diagnostic comes from the shared
                        // memory-operand validator, byte-matching GAS.
                        let dst_num = xop_vec_id(dst_name)
                            .ok_or_else(|| format!("bad XOP register: {dst_name}"))?;
                        let r = needs_vex_ext(dst_name);
                        let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                        let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                        self.emit_xop_prefix(r, x, b_ext, vvvv, v_prime, l, XOP_MAP_8);
                        self.bytes.push(opcode);
                        let rc = self.relocations.len();
                        self.encode_modrm_mem(dst_num, mem)?;
                        self.bytes.push(src1_num << 4);
                        self.adjust_rip_reloc_addend(rc, 0);
                        Ok(())
                    }
                    _ => Err(format!(
                        "unsupported {mnemonic} operands: mixed register/memory src2"
                    )),
                }
            }
            Operand::Memory(mem) => {
                // Memory-first form: the memory operand takes r/m, so the
                // slot-less register is op2 — its number rides imm8[7:4]
                // (same AMD imm8-register-specifier mechanism as the
                // all-register form, just naming op2 instead of op1).
                // Only vpcmov/vpperm accept a memory FIRST operand; the
                // vpmac* family is register-only (GAS rejects the rest).
                if opcode != 0xA2 && opcode != 0xA3 {
                    return Err(format!(
                        "unsupported {mnemonic} operands: memory first operand"
                    ));
                }
                let src2_name = match &ops[1] {
                    Operand::Register(r) => &r.name,
                    _ => {
                        return Err(format!(
                            "unsupported {mnemonic} operands: memory in the src2 slot"
                        ))
                    }
                };
                let src2_num = xop_vec_id(src2_name)
                    .ok_or_else(|| format!("bad XOP register: {src2_name}"))?;
                let dst_num = xop_vec_id(dst_name)
                    .ok_or_else(|| format!("bad XOP register: {dst_name}"))?;
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let r = needs_vex_ext(dst_name);
                // GAS 2.47 quirk (byte-verified): the memory-first 4-op form
                // always sets V'=1 regardless of the vvvv register id —
                // `vpcmov (%rax), %xmm10, %xmm3, %xmm4` encodes vvvv=xmm10
                // with V'=1. Mirrored verbatim.
                self.emit_xop_prefix(r, x, b_ext, vvvv, true, l, XOP_MAP_8);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(src2_num << 4);
                self.adjust_rip_reloc_addend(rc, 0);
                Ok(())
            }
            _ => Err(format!("unsupported {mnemonic} operands")),
        }
    }

    /// XOP unary horizontal arithmetic (map 9): `vphadd*`/`vphsub*`,
    /// AT&T (src, dst). vvvv unused (encoded 1111). The family is
    /// xmm-only: any ymm operand is `operand size mismatch` (GAS 2.47,
    /// byte-probed — no ymm forms exist in the XOP horizontal ISA).
    fn encode_xop_unary(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        if ops.iter().any(
            |op| matches!(op, Operand::Register(r) if is_ymm(&r.name)),
        ) {
            return Err(format!("operand size mismatch for `{mnemonic}'"));
        }
        let (src_name, dst_name) = match (&ops[0], &ops[1]) {
            (Operand::Register(r), Operand::Register(d)) => (&r.name, &d.name),
            (Operand::Memory(mem), Operand::Register(d)) => {
                let dst_num = xop_vec_id(&d.name).ok_or_else(|| {
                    format!("bad XOP register: {}", d.name)
                })?;
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let r = needs_vex_ext(&d.name);
                self.emit_xop_prefix(r, x, b_ext, 0, false, 0, XOP_MAP_9);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.adjust_rip_reloc_addend(rc, 0);
                return Ok(());
            }
            _ => return Err(format!("unsupported XOP horizontal operands for `{mnemonic}'")),
        };
        let src_num = xop_vec_id(src_name)
            .ok_or_else(|| format!("bad XOP register: {src_name}"))?;
        let dst_num = xop_vec_id(dst_name)
            .ok_or_else(|| format!("bad XOP register: {dst_name}"))?;
        let r = needs_vex_ext(dst_name);
        let b = needs_vex_ext(src_name);
        self.emit_xop_prefix(r, false, b, 0, false, 0, XOP_MAP_9);
        self.bytes.push(opcode);
        self.bytes.push(self.modrm(3, dst_num, src_num));
        Ok(())
    }

    /// XOP `vfrcz*` (map 9, opcodes 80–83): AT&T `(src, dst)`, register
    /// or memory source, and — unlike the horizontal family — full
    /// ymm support (L = register width). GAS 2.47 rejects mixed widths
    /// with `register type mismatch` and a memory destination with
    /// `operand type mismatch` (both byte-probed).
    fn encode_vfrcz(&mut self, ops: &[Operand], opcode: u8, mnemonic: &str) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        let width_of = |op: &Operand| -> Option<u8> {
            match op {
                Operand::Register(r) if is_xmm(&r.name) => Some(0),
                Operand::Register(r) if is_ymm(&r.name) => Some(1),
                _ => None,
            }
        };
        let widths: Vec<u8> = ops.iter().filter_map(width_of).collect();
        if widths.iter().any(|w| *w != widths[0]) {
            return Err(format!("register type mismatch for `{mnemonic}'"));
        }
        let l = widths.first().copied().unwrap_or(0);
        let dst_name = match &ops[1] {
            Operand::Register(r) => &r.name,
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        let dst_num =
            xop_vec_id(dst_name).ok_or_else(|| format!("bad XOP register: {dst_name}"))?;
        let r = needs_vex_ext(dst_name);
        match &ops[0] {
            Operand::Register(src) => {
                let src_num = xop_vec_id(&src.name)
                    .ok_or_else(|| format!("bad XOP register: {}", src.name))?;
                let b = needs_vex_ext(&src.name);
                self.emit_xop_prefix(r, false, b, 0, false, l, XOP_MAP_9);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_xop_prefix(r, x, b_ext, 0, false, l, XOP_MAP_9);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.adjust_rip_reloc_addend(rc, 0);
                Ok(())
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    /// XOP shifts `vpsha*`/`vpshl*` (map 9, opcodes 94–9B): AT&T
    /// `(src1, src2, dst)` with vvvv = src1, r/m = src2 (register or
    /// memory), ModRM.reg = dst. xmm-only ISA — any ymm operand is
    /// `operand size mismatch` (GAS 2.47, byte-probed: the family has
    /// no ymm forms, so ymm is a SIZE error, not a width-mixing one).
    fn encode_xop_shift(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        if ops.iter().any(
            |op| matches!(op, Operand::Register(r) if is_ymm(&r.name)),
        ) {
            return Err(format!("operand size mismatch for `{mnemonic}'"));
        }
        let (src1_name, dst_name) = match (&ops[0], &ops[2]) {
            (Operand::Register(s), Operand::Register(d)) => (&s.name, &d.name),
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        let (vvvv, v_prime) = xop_vvvv(src1_name)?;
        let dst_num =
            xop_vec_id(dst_name).ok_or_else(|| format!("bad XOP register: {dst_name}"))?;
        let r = needs_vex_ext(dst_name);
        match &ops[1] {
            Operand::Register(src2) => {
                let src2_num = xop_vec_id(&src2.name)
                    .ok_or_else(|| format!("bad XOP register: {}", src2.name))?;
                let b = needs_vex_ext(&src2.name);
                self.emit_xop_prefix(r, false, b, vvvv, v_prime, 0, XOP_MAP_9);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src2_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_xop_prefix(r, x, b_ext, vvvv, v_prime, 0, XOP_MAP_9);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.adjust_rip_reloc_addend(rc, 0);
                Ok(())
            }
            _ => Err(format!("operand type mismatch for `{mnemonic}'")),
        }
    }

    /// XOP `vprot{b,w,d,q}`:
    /// * imm form (map 8, opcodes C0–C3): `$imm, src, dst` — vvvv unused.
    /// * register-count form (map 9, opcodes 90–93): `count, src, dst` —
    ///   vvvv = count, ModRM = (dst, src).
    /// * memory-count form (map 9): `m128, src, dst` — vvvv = src,
    ///   ModRM = (dst, mem).
    fn encode_vprot(
        &mut self,
        ops: &[Operand],
        opcode_imm: u8,
        opcode_reg: u8,
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err(format!("number of operands mismatch for `{mnemonic}'"));
        }
        // xmm-only ISA: any ymm operand is `operand size mismatch for
        // `vprot'' (GAS 2.47, byte-probed on all three forms).
        if ops.iter().any(
            |op| matches!(op, Operand::Register(r) if is_ymm(&r.name)),
        ) {
            return Err(format!("operand size mismatch for `{mnemonic}'"));
        }
        let (dst_name, l) = match &ops[2] {
            Operand::Register(r) => (&r.name, 0),
            _ => return Err(format!("operand type mismatch for `{mnemonic}'")),
        };
        let dst_num =
            xop_vec_id(dst_name).ok_or_else(|| format!("bad XOP register: {dst_name}"))?;
        let r = needs_vex_ext(dst_name);
        match (&ops[0], &ops[1]) {
            (Operand::Immediate(ImmediateValue::Integer(imm)), Operand::Register(src)) => {
                let src_num = xop_vec_id(&src.name)
                    .ok_or_else(|| format!("bad XOP register: {}", src.name))?;
                let b = needs_vex_ext(&src.name);
                self.emit_xop_prefix(r, false, b, 0, false, l, XOP_MAP_8);
                self.bytes.push(opcode_imm);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (Operand::Register(count), Operand::Register(src)) => {
                let (vvvv, v_prime) = xop_vvvv(&count.name)?;
                let src_num = xop_vec_id(&src.name)
                    .ok_or_else(|| format!("bad XOP register: {}", src.name))?;
                let b = needs_vex_ext(&src.name);
                self.emit_xop_prefix(r, false, b, vvvv, v_prime, l, XOP_MAP_9);
                self.bytes.push(opcode_reg);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(src)) => {
                // Memory-count form: the register source rides vvvv, the
                // memory rides r/m (GAS 2.47: `8f e9 e8 90 18` for
                // `vprotb (%rax), %xmm2, %xmm3`). Like the vpcmov mem
                // form, GAS always sets V'=1 here regardless of the id.
                let (vvvv, _) = xop_vvvv(&src.name)?;
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_xop_prefix(r, x, b_ext, vvvv, true, l, XOP_MAP_9);
                self.bytes.push(opcode_reg);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.adjust_rip_reloc_addend(rc, 0);
                Ok(())
            }
            _ => Err(format!("unsupported {mnemonic} operands")),
        }
    }

    /// Top-level XOP/LWP dispatch. Returns None for non-XOP mnemonics so
    /// the caller falls through to the VEX/EVEX tables.
    pub(crate) fn try_encode_xop(
        &mut self,
        mnemonic: &str,
        ops: &[Operand],
    ) -> Option<Result<(), String>> {
        if !Self::is_xop_mnemonic(mnemonic) {
            return None;
        }
        let res = match mnemonic {
            "vpcmov" => self.encode_xop_4op(ops, 0xA2, mnemonic),
            "vpperm" => self.encode_xop_4op(ops, 0xA3, mnemonic),
            "vpmacssww" => self.encode_xop_4op(ops, 0x85, mnemonic),
            "vpmacsww" => self.encode_xop_4op(ops, 0x95, mnemonic),
            "vpmacsswd" => self.encode_xop_4op(ops, 0x86, mnemonic),
            "vpmacswd" => self.encode_xop_4op(ops, 0x96, mnemonic),
            "vpmacssdd" => self.encode_xop_4op(ops, 0x8E, mnemonic),
            "vpmacsdd" => self.encode_xop_4op(ops, 0x9E, mnemonic),
            "vpmacssdql" => self.encode_xop_4op(ops, 0x87, mnemonic),
            "vpmacsdql" => self.encode_xop_4op(ops, 0x97, mnemonic),
            "vpmacssdqh" => self.encode_xop_4op(ops, 0x8F, mnemonic),
            "vpmacsdqh" => self.encode_xop_4op(ops, 0x9F, mnemonic),
            "vprotb" => self.encode_vprot(ops, 0xC0, 0x90, mnemonic),
            "vprotw" => self.encode_vprot(ops, 0xC1, 0x91, mnemonic),
            "vprotd" => self.encode_vprot(ops, 0xC2, 0x92, mnemonic),
            "vprotq" => self.encode_vprot(ops, 0xC3, 0x93, mnemonic),
            "vphaddubw" => self.encode_xop_unary(ops, 0xD1, mnemonic),
            "vphaddbw" => self.encode_xop_unary(ops, 0xC1, mnemonic),
            "vphaddubq" => self.encode_xop_unary(ops, 0xD3, mnemonic),
            "vphaddbq" => self.encode_xop_unary(ops, 0xC3, mnemonic),
            "vphaddubd" => self.encode_xop_unary(ops, 0xD2, mnemonic),
            "vphaddbd" => self.encode_xop_unary(ops, 0xC2, mnemonic),
            "vphadduwd" => self.encode_xop_unary(ops, 0xD6, mnemonic),
            "vphaddwd" => self.encode_xop_unary(ops, 0xC6, mnemonic),
            "vphadduwq" => self.encode_xop_unary(ops, 0xD7, mnemonic),
            "vphaddwq" => self.encode_xop_unary(ops, 0xC7, mnemonic),
            "vphaddudq" => self.encode_xop_unary(ops, 0xDB, mnemonic),
            "vphadddq" => self.encode_xop_unary(ops, 0xCB, mnemonic),
            "vphsubbw" => self.encode_xop_unary(ops, 0xE1, mnemonic),
            "vphsubwd" => self.encode_xop_unary(ops, 0xE2, mnemonic),
            "vphsubdq" => self.encode_xop_unary(ops, 0xE3, mnemonic),
            "vfrczps" => self.encode_vfrcz(ops, 0x80, mnemonic),
            "vfrczpd" => self.encode_vfrcz(ops, 0x81, mnemonic),
            "vfrczss" => self.encode_vfrcz(ops, 0x82, mnemonic),
            "vfrczsd" => self.encode_vfrcz(ops, 0x83, mnemonic),
            "vpshab" => self.encode_xop_shift(ops, 0x98, mnemonic),
            "vpshaw" => self.encode_xop_shift(ops, 0x99, mnemonic),
            "vpshad" => self.encode_xop_shift(ops, 0x9A, mnemonic),
            "vpshaq" => self.encode_xop_shift(ops, 0x9B, mnemonic),
            "vpshlb" => self.encode_xop_shift(ops, 0x94, mnemonic),
            "vpshlw" => self.encode_xop_shift(ops, 0x95, mnemonic),
            "vpshld" => self.encode_xop_shift(ops, 0x96, mnemonic),
            "vpshlq" => self.encode_xop_shift(ops, 0x97, mnemonic),
            "lwpins" => self.encode_lwp(ops, 0, mnemonic),
            "lwpval" => self.encode_lwp(ops, 1, mnemonic),
            _ => return None,
        };
        Some(res)
    }
}