use super::*;

impl super::InstructionEncoder {
    // ---- Encoding helpers ----

    /// Build a REX prefix byte.
    pub(crate) fn rex(&self, w: bool, r: bool, x: bool, b: bool) -> u8 {
        let mut rex = 0x40u8;
        if w {
            rex |= 0x08;
        }
        if r {
            rex |= 0x04;
        }
        if x {
            rex |= 0x02;
        }
        if b {
            rex |= 0x01;
        }
        rex
    }

    /// Encode ModR/M byte.
    pub(crate) fn modrm(&self, mod_: u8, reg: u8, rm: u8) -> u8 {
        (mod_ << 6) | ((reg & 7) << 3) | (rm & 7)
    }

    /// Encode SIB byte.
    pub(crate) fn sib(&self, scale: u8, index: u8, base: u8) -> u8 {
        let scale_bits = match scale {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            _ => 0,
        };
        (scale_bits << 6) | ((index & 7) << 3) | (base & 7)
    }

    /// Emit REX prefix if needed for reg-reg operation.
    pub(crate) fn emit_rex_rr(&mut self, size: u8, reg: &str, rm: &str) {
        let w = size == 8;
        let r = needs_rex_ext(reg);
        let b = needs_rex_ext(rm);
        let need_rex = w || r || b || is_rex_required_8bit(reg) || is_rex_required_8bit(rm);
        if need_rex {
            self.bytes.push(self.rex(w, r, false, b));
        }
    }

    /// Emit segment override prefix (0x64 for %fs, 0x65 for %gs) if present.
    /// Must be emitted before any operand-size override, REX prefix, or opcode.
    // TODO: emit_segment_prefix is called in mov, ALU ops, push, and pop.
    // Other instruction families that accept memory operands should also call this.
    pub(crate) fn emit_segment_prefix(&mut self, mem: &MemoryOperand) -> Result<(), String> {
        if let Some(ref seg) = mem.segment {
            // All six segment override prefixes. In 64-bit mode only fs/gs
            // change the effective address, but cs/ds/es/ss overrides remain
            // legal encodings that appear in real code (the canonical long
            // NOPs carry a %cs prefix), so refusing them rejected valid input.
            let byte = match seg.as_str() {
                "es" => Some(0x26u8),
                "cs" => Some(0x2E),
                // %ds is the default segment for every addressing form in
                // 64-bit mode, so an explicit override is a pure no-op and GAS
                // drops it.  %ss is NOT dropped: even though it selects the
                // same flat segment, GAS still emits 0x36, and hardware treats
                // the prefix as significant for a few corner cases (it is also
                // the documented spelling of the CET no-track prefix).
                // Verified against GAS 2.47: `mov %ds:8(%rax),%rbx` -> 48 8b 58
                // 08, `mov %ss:8(%rax),%rbx` -> 36 48 8b 58 08.
                "ds" => None,
                "ss" => Some(0x36),
                "fs" => Some(0x64),
                "gs" => Some(0x65),
                _ => return Err(format!("unsupported segment override: %{}", seg)),
            };
            if let Some(b) = byte {
                // The segment override is the OUTERMOST legacy prefix: it must
                // precede an operand-size (0x66) or address-size (0x67) prefix
                // that an earlier stage may already have emitted.
                let mut at = self.bytes.len();
                while at > 0 && matches!(self.bytes[at - 1], 0x66 | 0x67) {
                    at -= 1;
                }
                self.bytes.insert(at, b);
            }
        }
        Ok(())
    }

    /// Emit REX prefix for a memory operand where 'reg' is the reg field.
    pub(crate) fn emit_rex_rm(&mut self, size: u8, reg: &str, mem: &MemoryOperand) {
        // 32-bit address-size override (0x67): required whenever the memory
        // operand's base or index is a 32-bit register (e.g.
        // `leal (%edi,%edi,2),%edi`). Emitted BEFORE the REX byte (prefix
        // order: segment, 66/67, REX, opcode). The old code silently dropped
        // it — the address was encoded as 64-bit, which GAS-oracle caught as
        // a one-byte divergence from GNU as (and is semantically wrong for
        // addresses >= 4 GiB).
        // Fold BEFORE computing REX: if the index moves into the base slot the
        // extension bit for r8-r15 must be REX.B, not REX.X.
        let folded = fold_scale1_index(mem);
        let mem = folded.as_ref().unwrap_or(mem);

        let addr32 = mem.base.as_ref().is_some_and(|b| is_reg32(&b.name))
            || mem.index.as_ref().is_some_and(|i| is_reg32(&i.name));
        if addr32 {
            self.bytes.push(0x67);
        }
        let w = size == 8;
        let r = needs_rex_ext(reg);
        let b = mem.base.as_ref().is_some_and(|b| needs_rex_ext(&b.name));
        let x = mem.index.as_ref().is_some_and(|i| needs_rex_ext(&i.name));
        let need_rex = w || r || b || x || is_rex_required_8bit(reg);
        if need_rex {
            self.bytes.push(self.rex(w, r, x, b));
        }
    }

    /// Emit REX prefix for unary operation on register.
    pub(crate) fn emit_rex_unary(&mut self, size: u8, rm: &str) {
        let w = size == 8;
        let b = needs_rex_ext(rm);
        let need_rex = w || b || is_rex_required_8bit(rm);
        if need_rex {
            self.bytes.push(self.rex(w, false, false, b));
        }
    }

    /// Encode ModR/M + SIB + displacement for a memory operand.
    /// Returns the bytes to append. `reg_field` is the /r value (3 bits).
    /// True when the instruction currently being encoded carries a REX prefix.
    ///
    /// `self.bytes` holds exactly one instruction, so the prefix area is the
    /// run of legacy prefixes at the front.  A REX byte (0x40-0x4F) is the last
    /// prefix before the opcode, so scanning past the legacy prefixes and
    /// testing the next byte identifies it unambiguously.
    pub(crate) fn has_rex_prefix(&self) -> bool {
        for &b in &self.bytes {
            match b {
                // Legacy prefixes: segment, operand/address size, lock/rep.
                0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0x66 | 0x67 | 0xF0 | 0xF2 | 0xF3 => {}
                0x40..=0x4F => return true,
                _ => return false,
            }
        }
        false
    }

    pub(crate) fn encode_modrm_mem(
        &mut self,
        reg_field: u8,
        mem: &MemoryOperand,
    ) -> Result<(), String> {
        let folded = fold_scale1_index(mem);
        let mem = folded.as_ref().unwrap_or(mem);

        let base = mem.base.as_ref();
        let index = mem.index.as_ref();

        // RIP-relative addressing
        if let Some(base_reg) = base {
            if base_reg.name == "rip" {
                // ModR/M: mod=00, rm=101 (RIP-relative)
                self.bytes.push(self.modrm(0, reg_field, 5));
                // 32-bit displacement (will be filled by relocation)
                match &mem.displacement {
                    Displacement::Symbol(sym) => {
                        self.add_relocation(sym, R_X86_64_PC32, -4);
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    }
                    Displacement::SymbolAddend(sym, addend) => {
                        self.add_relocation(sym, R_X86_64_PC32, *addend - 4);
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    }
                    Displacement::SymbolPlusOffset(sym, offset) => {
                        self.add_relocation(sym, R_X86_64_PC32, *offset - 4);
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    }
                    Displacement::SymbolMod(sym, modifier) => {
                        // Modifiers are case-insensitive in GNU as: glibc's
                        // multiarch sources emit lowercase `sym@gottpoff(%rip)`.
                        let reloc_type = match modifier.to_ascii_lowercase().as_str() {
                            // GAS never emits the plain, un-relaxable
                            // R_X86_64_GOTPCREL (9) for a RIP-relative GOT load.
                            // It emits the "X" variants, which tell the linker
                            // it may rewrite `mov sym@GOTPCREL(%rip),%reg` into
                            // a direct `lea` when the symbol turns out to be
                            // local -- removing a GOT entry and a load.  Which
                            // of the two applies is decided purely by whether
                            // the instruction carries a REX prefix.
                            "gotpcrel" => {
                                if self.has_rex_prefix() {
                                    R_X86_64_REX_GOTPCRELX
                                } else {
                                    R_X86_64_GOTPCRELX
                                }
                            }
                            "gotpcrelx" => R_X86_64_GOTPCRELX,
                            "rex_gotpcrelx" => R_X86_64_REX_GOTPCRELX,
                            "gottpoff" => R_X86_64_GOTTPOFF,
                            "tpoff" => R_X86_64_TPOFF32,
                            "plt" => R_X86_64_PLT32,
                            _ => R_X86_64_PC32,
                        };
                        self.add_relocation(sym, reloc_type, -4);
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    }
                    Displacement::Integer(val) => {
                        self.bytes.extend_from_slice(&(*val as i32).to_le_bytes());
                    }
                    Displacement::SymbolDiff(a, b) | Displacement::SymbolDiffAddend(a, b, _) => {
                        return Err(format!(
                            "symbol-difference displacement `{} - {}` is not valid with RIP-relative addressing", a, b));
                    }
                    Displacement::None => {
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    }
                }
                return Ok(());
            }
        }

        // Handle symbol displacements that need relocations.
        // We defer emitting the relocation until after the ModR/M and SIB bytes
        // so the relocation offset correctly points to the displacement bytes.
        let mut diff_sym: Option<String> = None;
        let (disp_val, has_symbol, deferred_reloc) = match &mem.displacement {
            Displacement::None => (0i64, false, None),
            Displacement::Integer(v) => (*v, false, None),
            Displacement::Symbol(sym) => (0i64, true, Some((sym.clone(), R_X86_64_32S, 0i64))),
            Displacement::SymbolAddend(sym, addend) => {
                (0i64, true, Some((sym.clone(), R_X86_64_32S, *addend)))
            }
            Displacement::SymbolPlusOffset(sym, offset) => {
                (0i64, true, Some((sym.clone(), R_X86_64_32S, *offset)))
            }
            Displacement::SymbolDiff(sym, diff) => {
                // head_64.S rva(): `((gdt) - startup_32)(%ebp)`. Recorded as
                // a diff relocation; same-section pairs fold to a constant
                // after layout, so no reloc reaches the object file.
                diff_sym = Some(diff.clone());
                (0i64, true, Some((sym.clone(), R_X86_64_32, 0i64)))
            }
            Displacement::SymbolDiffAddend(sym, diff, addend) => {
                diff_sym = Some(diff.clone());
                (0i64, true, Some((sym.clone(), R_X86_64_32, *addend)))
            }
            Displacement::SymbolMod(sym, modifier) => {
                let reloc_type = match modifier.to_ascii_lowercase().as_str() {
                    "tpoff" => R_X86_64_TPOFF32,
                    "gotpcrel" => R_X86_64_GOTPCREL,
                    "gottpoff" => R_X86_64_GOTTPOFF,
                    _ => R_X86_64_32S,
                };
                (0i64, true, Some((sym.clone(), reloc_type, 0i64)))
            }
        };

        // No base register - need SIB with no-base encoding
        if base.is_none() && index.is_none() {
            // Direct memory reference - mod=00, rm=100 (SIB), SIB: base=101 (no base)
            self.bytes.push(self.modrm(0, reg_field, 4));
            self.bytes.push(self.sib(1, 4, 5)); // index=100 (none), base=101 (disp32)
            if let Some((sym, reloc_type, addend)) = deferred_reloc {
                match &diff_sym {
                    Some(d) => self.add_diff_relocation(&sym, d, reloc_type, addend),
                    None => self.add_relocation(&sym, reloc_type, addend),
                }
            }
            self.bytes
                .extend_from_slice(&(disp_val as i32).to_le_bytes());
            return Ok(());
        }

        let base_reg = base.map(|r| &r.name as &str).unwrap_or("");
        let base_num = if !base_reg.is_empty() {
            reg_num(base_reg).unwrap_or(0)
        } else {
            5
        };

        // Determine if we need SIB
        let need_sib = index.is_some()
            || (base_num & 7) == 4  // RSP/R12 always need SIB
            || base.is_none();

        // Determine displacement size
        let (mod_bits, disp_size) = if has_symbol {
            (2, 4) // always use disp32 for symbols
        } else if disp_val == 0 && (base_num & 7) != 5 {
            // No displacement (RBP/R13 always need at least disp8)
            (0, 0)
        } else if (-128..=127).contains(&disp_val) {
            (1, 1) // disp8
        } else {
            (2, 4) // disp32
        };

        if need_sib {
            let idx = index.as_ref();
            let idx_num = idx.map(|r| reg_num(&r.name).unwrap_or(4)).unwrap_or(4); // 4 = no index
            let scale = mem.scale.unwrap_or(1);

            if base.is_none() {
                // No base - disp32 with SIB
                self.bytes.push(self.modrm(0, reg_field, 4));
                self.bytes.push(self.sib(scale, idx_num, 5));
                if let Some((sym, reloc_type, addend)) = deferred_reloc {
                    match &diff_sym {
                        Some(d) => self.add_diff_relocation(&sym, d, reloc_type, addend),
                        None => self.add_relocation(&sym, reloc_type, addend),
                    }
                }
                self.bytes
                    .extend_from_slice(&(disp_val as i32).to_le_bytes());
            } else {
                self.bytes.push(self.modrm(mod_bits, reg_field, 4));
                self.bytes.push(self.sib(scale, idx_num, base_num));
                if let Some((sym, reloc_type, addend)) = deferred_reloc {
                    match &diff_sym {
                        Some(d) => self.add_diff_relocation(&sym, d, reloc_type, addend),
                        None => self.add_relocation(&sym, reloc_type, addend),
                    }
                }
                match disp_size {
                    0 => {}
                    1 => self.bytes.push(disp_val as u8),
                    4 => self
                        .bytes
                        .extend_from_slice(&(disp_val as i32).to_le_bytes()),
                    _ => unreachable!(),
                }
            }
        } else {
            self.bytes.push(self.modrm(mod_bits, reg_field, base_num));
            if let Some((sym, reloc_type, addend)) = deferred_reloc {
                match &diff_sym {
                    Some(d) => self.add_diff_relocation(&sym, d, reloc_type, addend),
                    None => self.add_relocation(&sym, reloc_type, addend),
                }
            }
            match disp_size {
                0 => {}
                1 => self.bytes.push(disp_val as u8),
                4 => self
                    .bytes
                    .extend_from_slice(&(disp_val as i32).to_le_bytes()),
                _ => unreachable!(),
            }
        }

        Ok(())
    }

    /// Add a relocation relative to current position.
    pub(crate) fn add_relocation(&mut self, symbol: &str, reloc_type: u32, addend: i64) {
        // Strip @PLT suffix from symbol names - the suffix only affects relocation type,
        // not the symbol name in the ELF symbol table. Use PLT32 reloc when @PLT is present.
        let (sym, rtype) = if let Some(base) = symbol.strip_suffix("@PLT") {
            let plt_type = if reloc_type == R_X86_64_PC32 {
                R_X86_64_PLT32
            } else {
                reloc_type
            };
            (base, plt_type)
        } else {
            (symbol, reloc_type)
        };
        self.relocations.push(Relocation {
            offset: self.offset + self.bytes.len() as u64 - (self.offset), // adjusted in caller
            symbol: sym.to_string(),
            reloc_type: rtype,
            addend,
            diff_symbol: None,
        });
    }

    /// Add a symbol-difference relocation for an immediate (`$a - b`).
    /// Resolution happens after layout: same-section pairs patch the
    /// constant `a - b + addend`; otherwise the writer converts to a
    /// PC-relative reloc against `a` with the addend adjusted by `b`'s
    /// position (GAS semantics for `$sym - 0b`).
    pub(crate) fn add_diff_relocation(
        &mut self,
        symbol: &str,
        diff: &str,
        reloc_type: u32,
        addend: i64,
    ) {
        self.relocations.push(Relocation {
            offset: self.bytes.len() as u64,
            symbol: symbol.to_string(),
            reloc_type,
            addend,
            diff_symbol: Some(diff.to_string()),
        });
    }

    /// Adjust a RIP-relative relocation's addend to account for immediate bytes
    /// that follow the displacement field in the instruction encoding.
    ///
    /// In x86-64, RIP-relative addressing computes the effective address as
    /// RIP + disp32, where RIP points to the byte *after* the current instruction.
    /// The R_X86_64_PC32 relocation computes S + A - P, where P is the address of
    /// the disp32 field. So the addend A must equal -(bytes from disp32 to end of
    /// instruction). `encode_modrm_mem` always uses A = -4 (for the disp32 itself),
    /// but instructions with trailing immediate bytes need A = -(4 + trailing_bytes).
    ///
    /// `reloc_count_before` is the length of `self.relocations` before
    /// `encode_modrm_mem` was called. This ensures we only adjust the relocation
    /// that was emitted by `encode_modrm_mem`, not any subsequent ones.
    pub(crate) fn adjust_rip_reloc_addend(
        &mut self,
        reloc_count_before: usize,
        trailing_bytes: i64,
    ) {
        // Only adjust if encode_modrm_mem added a relocation
        if self.relocations.len() > reloc_count_before {
            let reloc = &mut self.relocations[reloc_count_before];
            match reloc.reloc_type {
                R_X86_64_PC32 | R_X86_64_PLT32 | R_X86_64_GOTPCREL | R_X86_64_GOTTPOFF => {
                    reloc.addend -= trailing_bytes;
                }
                _ => {}
            }
        }
    }
}

/// Fold an index-only, scale-1 address into a plain base address.
///
/// `-1(,%rdi,1)` and `-1(%rdi)` compute the same effective address, but the
/// first needs a SIB byte and -- because SIB with no base only supports
/// mod=00 + disp32 -- a full 4-byte displacement.  Moving the index into the
/// base slot removes the SIB byte and lets the displacement shrink to disp8,
/// turning 8 bytes into 4.  ICC performs this fold; GAS 2.47, clang 22.1,
/// gcc 16.2 and icx 2024.0 all emit the longer form.
///
/// Two register numbers can never be folded, because the base slot assigns
/// them a meaning the index slot does not have:
///   * reg 4 (%rsp/%r12) -- base==100 is the escape that selects a SIB byte,
///     so folding produced `(%rsp,%r12,1)`.
///   * reg 5 (%rbp/%r13) -- mod=00 with base==101 means "no base, disp32",
///     so folding produced `(%rbp)`.
///
/// Returning the rewritten operand (rather than mutating in place) lets the
/// REX emitter and the ModR/M emitter share one decision: the extension bit
/// for r8-r15 has to move from REX.X to REX.B along with the register, and
/// having two code paths decide independently is what made an earlier version
/// of this fold emit `rex.WX mov -0x1(%rdx)` for `mov -1(,%r10,1)`.
pub(crate) fn fold_scale1_index(mem: &MemoryOperand) -> Option<MemoryOperand> {
    if mem.base.is_some() || mem.scale.unwrap_or(1) != 1 {
        return None;
    }
    let idx = mem.index.as_ref()?;
    // %rsp can never be an index, so an index-only operand naming it is
    // invalid input; leave it for the validator to reject.
    if idx.name == "rsp" || idx.name == "esp" {
        return None;
    }
    // Every other register folds, including reg 4 (%r12) and reg 5
    // (%rbp/%r13). Those two need special ModR/M shapes in the base slot --
    // %r12 needs a SIB with index=none, and %rbp at mod=00 would mean "no
    // base", so it needs mod=01 with a zero disp8 -- but the general memory
    // encoder already produces exactly those forms for a plain base operand
    // (verified byte-identical to GAS for `lea (%r12)`, `lea 0(%rbp)` and
    // `mov -1(%r12)`), so rewriting the operand is sufficient and no special
    // case is needed here.
    //
    // This is what lets us reach ICC's encoding for the whole family:
    // `lea 0(,%r12,1),%rdx` becomes 49 8d 14 24 (4 bytes) instead of
    // 4a 8d 14 25 00000000 (8 bytes).
    Some(MemoryOperand {
        base: mem.index.clone(),
        index: None,
        scale: None,
        ..mem.clone()
    })
}
