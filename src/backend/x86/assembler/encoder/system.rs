use super::*;

impl super::InstructionEncoder {
    /// `loop`, `loope`, `loopne`: a branch on the count register with an
    /// 8-bit displacement and no longer form. `narrow` counts in `%ecx`.
    pub(crate) fn encode_counter_branch(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        narrow: bool,
        name: &str,
    ) -> Result<(), String> {
        let [Operand::Label(label)] = ops else {
            return Err(format!("{name} requires a label"));
        };
        if narrow {
            self.bytes.push(0x67);
        }
        self.bytes.push(opcode);
        self.add_relocation(label, R_X86_64_PC8_INTERNAL, -1);
        self.bytes.push(0);
        Ok(())
    }

    pub(crate) fn encode_clflush(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("clflush requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
                self.encode_modrm_mem(7, mem)
            }
            _ => Err("clflush requires memory operand".to_string()),
        }
    }

    /// Encode MOV to/from control register: 0F 20 /r (read) or 0F 22 /r (write)
    pub(crate) fn encode_mov_cr(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mov cr requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(cr), Operand::Register(gp)) if is_control_reg(&cr.name) => {
                // movq %crN, %rax  =>  0F 20 ModR/M
                let cr_num = control_reg_num(&cr.name).ok_or("bad control register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                let (b, b4) = gp_ext_bits(&gp.name);
                self.emit_rex_or_rex2(false, cr_num >= 8, false, b, false, false, b4, false);
                self.bytes.extend_from_slice(&[0x0F, 0x20]);
                self.bytes.push(self.modrm(3, cr_num & 7, gp_num));
                Ok(())
            }
            (Operand::Register(gp), Operand::Register(cr)) if is_control_reg(&cr.name) => {
                // movq %rax, %crN  =>  0F 22 ModR/M
                let cr_num = control_reg_num(&cr.name).ok_or("bad control register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                let (b, b4) = gp_ext_bits(&gp.name);
                self.emit_rex_or_rex2(false, cr_num >= 8, false, b, false, false, b4, false);
                self.bytes.extend_from_slice(&[0x0F, 0x22]);
                self.bytes.push(self.modrm(3, cr_num & 7, gp_num));
                Ok(())
            }
            _ => Err("unsupported mov cr operands".to_string()),
        }
    }

    /// Encode MOV to/from debug register: 0F 21 /r (read) or 0F 23 /r (write)
    pub(crate) fn encode_mov_dr(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mov dr requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(dr), Operand::Register(gp)) if is_debug_reg(&dr.name) => {
                // movq %drN, %rax  =>  0F 21 ModR/M
                let dr_num = debug_reg_num(&dr.name).ok_or("bad debug register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                let mut rex = 0u8;
                if needs_rex_ext(&gp.name) {
                    rex |= 0x41;
                } // REX.B
                if rex != 0 {
                    self.bytes.push(rex);
                }
                self.bytes.extend_from_slice(&[0x0F, 0x21]);
                self.bytes.push(self.modrm(3, dr_num, gp_num));
                Ok(())
            }
            (Operand::Register(gp), Operand::Register(dr)) if is_debug_reg(&dr.name) => {
                // movq %rax, %drN  =>  0F 23 ModR/M
                let dr_num = debug_reg_num(&dr.name).ok_or("bad debug register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                let mut rex = 0u8;
                if needs_rex_ext(&gp.name) {
                    rex |= 0x41;
                } // REX.B
                if rex != 0 {
                    self.bytes.push(rex);
                }
                self.bytes.extend_from_slice(&[0x0F, 0x23]);
                self.bytes.push(self.modrm(3, dr_num, gp_num));
                Ok(())
            }
            _ => Err("unsupported mov dr operands".to_string()),
        }
    }

    /// Encode SGDT/SIDT/LGDT/LIDT: 0F 01 /N (memory operand)
    /// Handles suffixed forms (sgdtl, lgdtq, etc.) and label operands.
    pub(crate) fn encode_system_table(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err(format!("{} requires 1 operand", mnemonic));
        }
        // Strip size suffix (l/q) to get base mnemonic
        let base = mnemonic.trim_end_matches(['l', 'q']);
        let reg_ext = match base {
            "sgdt" => 0,
            "sidt" => 1,
            "lgdt" => 2,
            "lidt" => 3,
            _ => return Err(format!("unknown system table instruction: {}", mnemonic)),
        };
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(reg_ext, mem)
            }
            // Label operand: treat as RIP-relative memory reference
            Operand::Label(name) => {
                let mem = MemoryOperand {
                    segment: None,
                    displacement: Displacement::Symbol(name.clone()),
                    base: None,
                    index: None,
                    scale: None,
                    mask: None,
                    zeroing: false,
                    broadcast: None,
                };
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(reg_ext, &mem)
            }
            _ => Err(format!("{} requires memory operand", mnemonic)),
        }
    }

    /// Encode memory-only instructions (INVLPG, FXSAVE, FXRSTOR, etc.): opcode /ext
    pub(crate) fn encode_mem_only(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
        ext: u8,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("instruction requires 1 memory operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(opcode);
                self.encode_modrm_mem(ext, mem)
            }
            _ => Err("instruction requires memory operand".to_string()),
        }
    }

    /// Encode XSAVE-family: opcode /ext with optional forced REX.W (64-bit forms).
    /// GNU as encodings (verified against GNU as 2.47, re-probed S15):
    ///   xsave m       0F AE /4      xsave64 m     REX.W 0F AE /4
    ///   xrstor m      0F AE /5      xrstor64 m    REX.W 0F AE /5
    ///   xsaveopt m    0F AE /6      xsaveopt64 m  REX.W 0F AE /6
    ///   xsavec m      0F C7 /4      xsavec64 m    REX.W 0F C7 /4
    ///   xsaves m      0F C7 /5      xsaves64 m    REX.W 0F C7 /5
    ///   xrstors m     0F C7 /3      xrstors64 m   REX.W 0F C7 /3
    ///   fxrstor m     0F AE /1
    pub(crate) fn encode_xsave_family(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
        ext: u8,
        force_rex_w: bool,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("instruction requires 1 memory operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                if operands_have_egpr(ops) {
                    return Err(
                        "extended GPR cannot be used as base/index for `xsave' family".to_string(),
                    );
                }
                if force_rex_w {
                    self.emit_rex_rm(8, "", mem);
                } else {
                    self.emit_rex_rm(0, "", mem);
                }
                self.bytes.extend_from_slice(opcode);
                self.encode_modrm_mem(ext, mem)
            }
            _ => Err("instruction requires memory operand".to_string()),
        }
    }

    /// Encode FXSAVEQ: REX.W + 0F AE /0 (64-bit fxsave)
    pub(crate) fn encode_fxsaveq(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("fxsaveq requires 1 memory operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(8, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
                self.encode_modrm_mem(0, mem)
            }
            _ => Err("fxsaveq requires memory operand".to_string()),
        }
    }

    /// Encode FXRSTORQ: REX.W + 0F AE /1 (64-bit fxrstor)
    pub(crate) fn encode_fxrstorq(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("fxrstorq requires 1 memory operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(8, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
                self.encode_modrm_mem(1, mem)
            }
            _ => Err("fxrstorq requires memory operand".to_string()),
        }
    }

    /// Encode INVPCID: 66 0F 38 82 /r
    pub(crate) fn encode_invpcid(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("invpcid requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Memory(mem), Operand::Register(reg)) => {
                let reg_num = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.push(0x66);
                self.emit_rex_rm(0, &reg.name, mem);
                self.bytes.extend_from_slice(&[0x0F, 0x38, 0x82]);
                self.encode_modrm_mem(reg_num, mem)
            }
            _ => Err("invpcid requires memory and register operands".to_string()),
        }
    }

    /// Encode VMREAD (`NP 0F 78 /r`) and VMWRITE (`NP 0F 79 /r`).
    ///
    /// The VMCS field encoding is always the ModRM `reg` operand: the source
    /// of `vmread %field, r/m` and the destination of `vmwrite r/m, %field`.
    /// The operand size is fixed by the mode (64 bits in long mode), so there
    /// is no REX.W; REX carries only the register extensions. Bytes checked
    /// against GNU as 2.47 (`vmread %rdx,%rcx` = `0f 78 d1`, `vmwrite
    /// %r11,%rcx` = `41 0f 79 cb`). KVM's `vmcs_readl`/`vmcs_writel`
    /// (arch/x86/kvm/vmx/vmx_ops.h) are these two instructions; without them
    /// no object of arch/x86/kvm/vmx assembled.
    pub(crate) fn encode_vmread_vmwrite(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        let (opcode, field, rm) = match (mnemonic, ops) {
            ("vmread", [Operand::Register(field), rm]) => (0x78, field, rm),
            ("vmwrite", [rm, Operand::Register(field)]) => (0x79, field, rm),
            _ => {
                return Err(format!(
                    "{mnemonic} requires a register for the VMCS field and a register or memory operand"
                ));
            }
        };
        let field_num = reg_num(&field.name).ok_or("bad register")?;
        match rm {
            Operand::Register(reg) => {
                let rm_num = reg_num(&reg.name).ok_or("bad register")?;
                self.emit_rex_rr(0, &field.name, &reg.name);
                self.bytes.extend_from_slice(&[0x0F, opcode]);
                self.bytes.push(self.modrm(3, field_num, rm_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, &field.name, mem);
                self.bytes.extend_from_slice(&[0x0F, opcode]);
                self.encode_modrm_mem(field_num, mem)
            }
            _ => Err(format!("{mnemonic} requires a register or memory operand")),
        }
    }

    /// Encode the VMX instructions that take one 64-bit memory operand:
    /// VMPTRLD (`NP 0F C7 /6`), VMPTRST (`NP 0F C7 /7`), VMCLEAR (`66 0F C7
    /// /6`) and VMXON (`F3 0F C7 /6`). The mandatory prefix precedes REX.
    pub(crate) fn encode_vmx_pointer(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
        prefix: Option<u8>,
        ext: u8,
    ) -> Result<(), String> {
        let [Operand::Memory(mem)] = ops else {
            return Err(format!("{mnemonic} requires one memory operand"));
        };
        if let Some(prefix) = prefix {
            self.bytes.push(prefix);
        }
        self.emit_rex_rm(0, "", mem);
        self.bytes.extend_from_slice(&[0x0F, 0xC7]);
        self.encode_modrm_mem(ext, mem)
    }

    /// Encode INVEPT (`66 0F 38 80 /r`) and INVVPID (`66 0F 38 81 /r`): a
    /// 128-bit memory descriptor and the invalidation type in a register.
    pub(crate) fn encode_invept_invvpid(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
        opcode: u8,
    ) -> Result<(), String> {
        let [Operand::Memory(mem), Operand::Register(reg)] = ops else {
            return Err(format!("{mnemonic} requires memory and register operands"));
        };
        let reg_num = reg_num(&reg.name).ok_or("bad register")?;
        self.bytes.push(0x66);
        self.emit_rex_rm(0, &reg.name, mem);
        self.bytes.extend_from_slice(&[0x0F, 0x38, opcode]);
        self.encode_modrm_mem(reg_num, mem)
    }

    /// Encode RDFSBASE/RDGSBASE/WRFSBASE/WRGSBASE: F3 0F AE /N
    pub(crate) fn encode_fsgsbase(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err(format!("{} requires 1 register operand", mnemonic));
        }
        let ext = match mnemonic {
            "rdfsbase" => 0,
            "rdgsbase" => 1,
            "wrfsbase" => 2,
            "wrgsbase" => 3,
            _ => return Err(format!("unknown fsgsbase variant: {}", mnemonic)),
        };
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.push(0xF3);
                let size = infer_reg_size(&reg.name);
                self.emit_rex_unary(if size == 8 { 8 } else { 4 }, &reg.name);
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
                self.bytes.push(self.modrm(3, ext, num));
                Ok(())
            }
            _ => Err(format!("{} requires register operand", mnemonic)),
        }
    }

    /// `lcall`: see [`Self::encode_far_branch`].
    pub(crate) fn encode_lcall(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        self.encode_far_branch(ops, mnemonic, "lcall", 3)
    }

    /// `ljmp`: see [`Self::encode_far_branch`].
    pub(crate) fn encode_ljmp(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        self.encode_far_branch(ops, mnemonic, "ljmp", 5)
    }

    /// A far jump or call in 64-bit code: through a far pointer in memory,
    /// `FF /5` and `FF /3`.
    ///
    /// The pointer is `m16:32`, or `m16:16` under the `w` suffix, which
    /// takes the operand-size prefix (`ljmpw *xxx(,%edi,4)` = `67 66 ff 2c
    /// bd`; the `67` is spliced in front centrally). The kernel's EFI
    /// mixed-mode thunk drops into a 32-bit service with `lcalll
    /// *efi32_call(%rip)`. A bare symbol is the pointer's address, as it is
    /// for GNU as (which warns that the `*` is missing).
    ///
    /// What GNU as 2.47 refuses is refused here:
    ///
    /// * the direct forms `ljmp $segment, $offset` (`EA`, `9A`): the
    ///   opcodes do not exist in long mode. They used to be written all the
    ///   same -- an invalid-opcode fault where the statement stood;
    /// * the `q` suffix (`lcallq *(%rax)` used to come out with `REX.W`);
    /// * a register (`ljmp *%rax`): a far pointer does not fit in one.
    fn encode_far_branch(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
        name: &str,
        digit: u8,
    ) -> Result<(), String> {
        let suffix = mnemonic.strip_prefix(name).unwrap_or_default();
        if !matches!(suffix, "" | "w" | "l") {
            return Err(format!("invalid instruction suffix for `{name}'"));
        }
        let [target] = ops else {
            return Err(format!("unsupported instruction `{name}'"));
        };
        let mut pointer = match target {
            Operand::Indirect(inner) => inner.as_ref().clone(),
            other => other.clone(),
        };
        normalize_bare_label(&mut pointer);
        let Operand::Memory(mem) = &pointer else {
            return Err(format!("operand type mismatch for `{name}'"));
        };
        if suffix == "w" {
            self.bytes.push(0x66);
        }
        self.emit_rex_rm(0, "", mem);
        self.bytes.push(0xFF);
        self.encode_modrm_mem(digit, mem)
    }

    /// Encode LAR (Load Access Rights): 66 r/w 0F 02 /r — the operand size
    /// comes from the DESTINATION register (the source selector is r/m16);
    /// `lar %dx,%dx` needs 66, `lar (%edx),%dx` needs 67+66, and a 32/64-bit
    /// destination takes none/REX.W respectively (GAS 2.47 byte-verified).
    /// `lar` and `lsl` read a selector: a 16-bit register, or one as wide
    /// as the destination, of which they read 16 bits (`lar %ebp, %r9w`
    /// names neither).
    fn selector_width(mnemonic: &str, from: &Register, to: &Register) -> Result<(), String> {
        let (from, to) = (infer_reg_size(&from.name), infer_reg_size(&to.name));
        if from == 2 || from == to {
            Ok(())
        } else {
            Err(format!("register type mismatch for `{mnemonic}'"))
        }
    }

    pub(crate) fn encode_lar(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("lar requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                Self::selector_width("lar", src, dst)?;
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                // Infer size from destination register. LAR never takes
                // REX.W (GAS 2.47: `lar %rdx,%rdx` = 0f 02 d2 — the 32-bit
                // form zero-extends into the 64-bit destination), so the
                // width clamps at 4; only a 16-bit destination takes 66.
                let size = infer_reg_size(&dst.name).min(4);
                if size == 2 {
                    self.bytes.push(0x66); // operand-size prefix (before REX)
                }
                self.emit_rex_rr(size, &dst.name, &src.name);
                self.bytes.extend_from_slice(&[0x0F, 0x02]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let size = infer_reg_size(&dst.name).min(4);
                if size == 2 {
                    self.bytes.push(0x66); // operand-size prefix (before REX)
                }
                self.emit_rex_rm(size, &dst.name, mem);
                self.bytes.extend_from_slice(&[0x0F, 0x02]);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported lar operands".to_string()),
        }
    }

    /// Encode LSL (Load Segment Limit): 66 r/w 0F 03 /r — same destination
    /// width law as LAR (the selector source is always r/m16): the operand
    /// size is the DESTINATION register's width, 66 for r16 spellings,
    /// decided by the destination alone (GAS 2.47 byte-verified; the
    /// previous name-shape heuristic missed sp/bp and the whole memory
    /// form).
    pub(crate) fn encode_lsl(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("lsl requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                Self::selector_width("lsl", src, dst)?;
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let size = infer_reg_size(&dst.name).min(4); // no REX.W (GAS: lsl %rdx,%rdx = 0f 03 d2)
                if size == 2 {
                    self.bytes.push(0x66); // operand-size prefix (before REX)
                }
                self.emit_rex_rr(size, &dst.name, &src.name);
                self.bytes.extend_from_slice(&[0x0F, 0x03]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let size = infer_reg_size(&dst.name).min(4); // no REX.W
                if size == 2 {
                    self.bytes.push(0x66); // operand-size prefix (before REX)
                }
                self.emit_rex_rm(size, &dst.name, mem);
                self.bytes.extend_from_slice(&[0x0F, 0x03]);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported lsl operands".to_string()),
        }
    }
}

// ---- User-MSR / MSR-immediate family (APX, map-7 VEX/EVEX) ----
//
// Byte-exact from GAS 2.47's x86-64-user_msr.d / x86-64-msr_imm.d:
//
//   urdmsr  %s,%d   F2.0F38.F8 (r0-r15)   EVEX m4 pp3 F8 (any r16-r31)
//   uwrmsr  %s,%d   F3.0F38.F8 (r0-r15)   EVEX m4 pp2 F8
//   urdmsr  $i,%d   VEX m7 pp3 F8         EVEX m7 pp3 F8   (r16-r31)
//   uwrmsr  %s,$i   VEX m7 pp2 F8         EVEX m7 pp2 F8
//   rdmsr   $i,%d   VEX m7 pp3 F6         EVEX m7 pp3 F6
//   wrmsrns %s,$i   VEX m7 pp2 F6         EVEX m7 pp2 F6
//
// (m4/m7 = the 3-bit map field of P0; pp 3 = F2, 2 = F3.) Registers are
// 64-bit only — GAS rejects %r12d with "operand size mismatch". The
// immediate is UNSIGNED 32-bit: the invalid-testsuite rejects $-1 and
// anything above 0xffffffff. Legacy rdmsr/wrmsr (0F 32/30, no operands)
// are unchanged; only the $imm / 2-register spellings route here.
impl super::InstructionEncoder {
    fn msr_gp64<'a>(op: &'a Operand) -> Option<&'a str> {
        match op {
            Operand::Register(r) if is_reg64(&r.name) => Some(&r.name),
            _ => None,
        }
    }

    /// True when the register needs the r16-r31 EVEX form (bit 4 set).
    fn msr_is_egpr(name: &str) -> bool {
        gp_id(name).map(|n| n & 16 != 0).unwrap_or(false)
    }

    pub(crate) fn encode_user_msr(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        // The immediate forms of `urdmsr`/`uwrmsr` come back in here under
        // an inner name; a message names what was written.
        let written = mnemonic.strip_suffix("_imm").unwrap_or(mnemonic);
        match mnemonic {
            // ---- 2-register forms ----
            "urdmsr" | "uwrmsr" => {
                let [a, b] = ops else {
                    return Err(format!("number of operands mismatch for `{mnemonic}'"));
                };
                // The immediate spellings share the mnemonic:
                // `urdmsr $imm,%reg' reads, `uwrmsr %reg,$imm' writes.
                let imm_form = match (a, b) {
                    (Operand::Immediate(_), Operand::Register(_)) if mnemonic == "urdmsr" => {
                        Some("urdmsr_imm")
                    }
                    (Operand::Register(_), Operand::Immediate(_)) if mnemonic == "uwrmsr" => {
                        Some("uwrmsr_imm")
                    }
                    _ => None,
                };
                if let Some(inner) = imm_form {
                    return self.encode_user_msr(ops, inner);
                }
                let (Some(src), Some(dst)) = (Self::msr_gp64(a), Self::msr_gp64(b)) else {
                    return Err(format!("operand size mismatch for `{mnemonic}'"));
                };
                let pp_f2 = mnemonic == "urdmsr"; // F2 for read, F3 for write
                // Field law (GAS 2.47, tc-i386.c build_apx_evex_prefix +
                // print_register, cross-checked on the full register
                // matrix): the modrm REG field carries the FIRST urdmsr
                // operand and the SECOND uwrmsr operand; the RM field
                // carries the other one.  Extension bits follow their
                // FIELD OWNER, never a fixed operand position:
                //   reg  +8 -> REX.R / P0 bit7 (inverted), +16 -> REX2.R /
                //              P0 bit4 (inverted)
                //   rm   +8 -> REX.B / P0 bit5 (inverted), +16 -> REX2.B /
                //              P0 bit3 (SET, non-inverted)
                // EVEX iff any operand is an EGPR (r16+); r8-r15 use the
                // legacy REX form; a REX extending nothing is elided
                // (GAS: `urdmsr %rdx,%rax' = f2 0f 38 f8 d0, no 0x40).
                let (s, d) = (gp_id(src).unwrap(), gp_id(dst).unwrap());
                let (reg_id, rm_id) = if pp_f2 { (s, d) } else { (d, s) };
                if Self::msr_is_egpr(src) || Self::msr_is_egpr(dst) {
                    // EVEX m4, pp 3(read)/2(write), opcode F8, W=0,
                    // vvvv unused (P2 = 08: V' set, ND clear, aaa 0).
                    self.emit_evex(
                        reg_id & 8 != 0,
                        false,
                        rm_id & 8 != 0,
                        reg_id & 16 != 0,
                        4,
                        0,
                        0,
                        false,
                        if pp_f2 { 3 } else { 2 },
                        0,
                        false,
                        0,
                        false,
                        rm_id & 16 != 0,
                        false,
                    );
                    self.bytes.push(0xF8);
                    self.bytes
                        .push(self.modrm(3, (reg_id & 7) as u8, (rm_id & 7) as u8));
                } else {
                    if pp_f2 {
                        self.bytes.push(0xF2);
                    } else {
                        self.bytes.push(0xF3);
                    }
                    let rex = 0x40 | (u8::from(reg_id & 8 != 0) << 2) | u8::from(rm_id & 8 != 0);
                    if rex != 0x40 {
                        self.bytes.push(rex);
                    }
                    self.bytes.extend_from_slice(&[0x0F, 0x38, 0xF8]);
                    self.bytes
                        .push(self.modrm(3, (reg_id & 7) as u8, (rm_id & 7) as u8));
                }
                Ok(())
            }
            // ---- immediate forms ----
            "rdmsr" | "urdmsr_imm" | "wrmsrns" | "uwrmsr_imm" => {
                // Canonical shapes: ($imm, %reg) for the readers,
                // (%reg, $imm) for the writers.
                let (imm_op, reg_op, opcode, pp) = match mnemonic {
                    "rdmsr" => {
                        let [a, b] = ops else {
                            return Err("number of operands mismatch for `rdmsr'".to_string());
                        };
                        (a, b, 0xF6u8, 3u8)
                    }
                    "urdmsr_imm" => {
                        let [a, b] = ops else {
                            return Err("number of operands mismatch for `urdmsr'".to_string());
                        };
                        (a, b, 0xF8, 3)
                    }
                    "wrmsrns" | "uwrmsr_imm" => {
                        let [a, b] = ops else {
                            return Err(format!("number of operands mismatch for `{written}'"));
                        };
                        (b, a, 0xF6, 2)
                    }
                    _ => unreachable!(),
                };
                // wrmsrns/uwrmsr write: opcode F6 with pp2 for wrmsrns, F8
                // pp2 for uwrmsr.
                let opcode = if mnemonic == "uwrmsr_imm" {
                    0xF8
                } else {
                    opcode
                };
                let reg = Self::msr_gp64(reg_op)
                    .ok_or_else(|| format!("operand size mismatch for `{written}'"))?;
                // The MSR number is an unsigned 32-bit field: a number in
                // 0..=0xFFFFFFFF, or a symbol with `R_X86_64_32` (GNU as
                // 2.47: `urdmsr $msr_index, %rcx` is `c4 e7 7b f8 c1` and
                // four relocated bytes; it used to be refused).
                let (imm, symbol) = match imm_op {
                    Operand::Immediate(ImmediateValue::Integer(v))
                        if (0..=i64::from(u32::MAX)).contains(v) =>
                    {
                        (*v as u32, None)
                    }
                    Operand::Immediate(ImmediateValue::Symbol(sym)) => (0, Some((sym, 0))),
                    Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, addend)) => {
                        (0, Some((sym, *addend)))
                    }
                    _ => return Err(format!("operand type mismatch for `{written}'")),
                };
                let r = gp_id(reg).unwrap();
                if Self::msr_is_egpr(reg) {
                    // EVEX m7: reg field 0, rm = destination, B/B4 ext.
                    self.emit_evex(
                        false,
                        false,
                        r & 8 != 0,
                        false,
                        7,
                        0,
                        0,
                        false,
                        pp,
                        0,
                        false,
                        0,
                        false,
                        r & 16 != 0,
                        false,
                    );
                    self.bytes.push(opcode);
                    self.bytes.push(self.modrm(3, 0, (r & 7) as u8));
                } else {
                    // VEX m7: C4, R=1, X=1, B = !(r&8).
                    self.emit_vex(false, false, r & 8 != 0, 7, 0, 0, 0, pp);
                    self.bytes.push(opcode);
                    self.bytes.push(self.modrm(3, 0, (r & 7) as u8));
                }
                if let Some((sym, addend)) = symbol {
                    self.add_relocation(sym, R_X86_64_32, addend);
                }
                self.bytes.extend_from_slice(&imm.to_le_bytes());
                Ok(())
            }
            _ => Err(format!("not a user-msr mnemonic: {mnemonic}")),
        }
    }
}

#[cfg(test)]
mod user_msr_tests {
    use super::super::apx_tests::{fail_msg, hex, hex_relocs};
    use super::*;

    /// Bytes of GNU as 2.47 (`as --64`).
    #[test]
    fn register_and_number_forms() {
        assert_eq!(hex("urdmsr %rax, %rcx"), "f2 0f 38 f8 c1");
        assert_eq!(hex("uwrmsr %rcx, %rax"), "f3 0f 38 f8 c1");
        assert_eq!(hex("urdmsr $0x1234, %rcx"), "c4 e7 7b f8 c1 34 12 00 00");
        assert_eq!(hex("uwrmsr %rcx, $0x1234"), "c4 e7 7a f8 c1 34 12 00 00");
        assert_eq!(hex("wrmsrns %rcx, $0x1234"), "c4 e7 7a f6 c1 34 12 00 00");
        assert_eq!(hex("rdmsr $0x1234, %rcx"), "c4 e7 7b f6 c1 34 12 00 00");
        assert_eq!(
            hex("urdmsr $0xffffffff, %rcx"),
            "c4 e7 7b f8 c1 ff ff ff ff"
        );
    }

    /// The MSR number may be a symbol: an unsigned 32-bit field with
    /// `R_X86_64_32`, in the VEX and in the EVEX form (`%r16`..`%r31`).
    /// All of these were "operand type mismatch".
    #[test]
    fn msr_number_as_a_symbol() {
        for (line, bytes) in [
            ("urdmsr $ext, %rcx", "c4 e7 7b f8 c1 00 00 00 00"),
            ("uwrmsr %rcx, $ext", "c4 e7 7a f8 c1 00 00 00 00"),
            ("wrmsrns %rcx, $ext", "c4 e7 7a f6 c1 00 00 00 00"),
            ("rdmsr $ext, %rcx", "c4 e7 7b f6 c1 00 00 00 00"),
            ("urdmsr $ext+4, %r9", "c4 c7 7b f8 c1 00 00 00 00"),
            ("wrmsrns %r17, $ext", "62 ff 7e 08 f6 c1 00 00 00 00"),
            ("rdmsr $ext, %r31", "62 df 7f 08 f6 c7 00 00 00 00"),
        ] {
            assert_eq!(
                hex_relocs(line),
                (bytes.to_string(), vec![R_X86_64_32]),
                "{line}"
            );
        }
    }

    /// The relocation sits on the four bytes of the number and carries the
    /// addend that was written.
    #[test]
    fn msr_symbol_relocation_place_and_addend() {
        let items = parse_asm("urdmsr $ext+4, %r9").unwrap();
        let mut encoder = InstructionEncoder::new();
        for item in items {
            if let AsmItem::Instruction(instruction) = item {
                encoder.encode(&instruction).unwrap();
            }
        }
        let [relocation] = encoder.relocations.as_slice() else {
            panic!("one relocation expected, got {}", encoder.relocations.len());
        };
        assert_eq!(relocation.offset, 5);
        assert_eq!(relocation.addend, 4);
        assert_eq!(relocation.symbol, "ext");
    }

    /// A number outside 0..=0xFFFFFFFF is refused, and the message names
    /// the mnemonic that was written (it said `urdmsr_imm').
    #[test]
    fn refused_numbers_name_the_mnemonic_written() {
        for (line, mnemonic) in [
            ("urdmsr $-1, %rcx", "urdmsr"),
            ("urdmsr $0x100000000, %rcx", "urdmsr"),
            ("uwrmsr %rcx, $-1", "uwrmsr"),
            ("wrmsrns %rcx, $0x100000000", "wrmsrns"),
            ("rdmsr $-1, %rcx", "rdmsr"),
        ] {
            assert_eq!(
                fail_msg(line),
                format!("operand type mismatch for `{mnemonic}'"),
                "{line}"
            );
        }
    }
}
