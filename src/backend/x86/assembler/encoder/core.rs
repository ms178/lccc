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
        let (r, r4) = gp_ext_bits(reg);
        let (b, b4) = gp_ext_bits(rm);
        let extra = is_rex_required_8bit(reg) || is_rex_required_8bit(rm);
        self.emit_rex_or_rex2(w, r, false, b, r4, false, b4, extra);
    }

    /// REX2 payload: `0xD5 | M0 R4 X4 B4 W R3 X3 B3`. M0 is left 0 here;
    /// `fixup_rex2_map1` promotes a following `0x0F` escape into M0=1.
    pub(crate) fn emit_rex2(
        &mut self,
        w: bool,
        r: bool,
        x: bool,
        b: bool,
        r4: bool,
        x4: bool,
        b4: bool,
    ) {
        let mut p = 0u8;
        if r4 {
            p |= 1 << 6;
        }
        if x4 {
            p |= 1 << 5;
        }
        if b4 {
            p |= 1 << 4;
        }
        if w {
            p |= 1 << 3;
        }
        if r {
            p |= 1 << 2;
        }
        if x {
            p |= 1 << 1;
        }
        if b {
            p |= 1 << 0;
        }
        self.bytes.push(0xD5);
        self.bytes.push(p);
    }

    /// REX or REX2. REX2 wins whenever an EGPR is involved or `{rex2}` was
    /// requested: REX cannot address %r16–%r31.
    pub(crate) fn emit_rex_or_rex2(
        &mut self,
        w: bool,
        r: bool,
        x: bool,
        b: bool,
        r4: bool,
        x4: bool,
        b4: bool,
        extra_rex: bool,
    ) {
        if r4 || x4 || b4 || self.apx_rex2 {
            self.emit_rex2(w, r, x, b, r4, x4, b4);
            return;
        }
        if w || r || x || b || extra_rex {
            self.bytes.push(self.rex(w, r, x, b));
        }
    }

    /// APX EVEX prefix for promoted GP integer instructions.
    ///
    /// Verified against GNU as 2.47 (`build_apx_evex_prefix`):
    /// P0 = `~R3 ~X3 ~B3 ~R4 B4 mmm` (B4 is **not** inverted),
    /// P1 = `W ~vvvv ~X4 pp`,
    /// P2 = `0 00 ND ~v4 NF 00`.
    ///
    /// `mmm`: 4 = legacy map-0/1 promotions, 2 = 0F38 BMI, 3 = 0F3A (rorx).
    pub(crate) fn emit_evex_apx(
        &mut self,
        r: bool,
        x: bool,
        b: bool,
        r4: bool,
        x4: bool,
        b4: bool,
        w: bool,
        vvvv: u8,
        nd: bool,
        nf: bool,
        pp: u8,
    ) {
        self.emit_evex_apx_map(r, x, b, r4, x4, b4, w, vvvv, nd, nf, pp, 4);
    }

    pub(crate) fn emit_evex_apx_map(
        &mut self,
        r: bool,
        x: bool,
        b: bool,
        r4: bool,
        x4: bool,
        b4: bool,
        w: bool,
        vvvv: u8,
        nd: bool,
        nf: bool,
        pp: u8,
        mmm: u8,
    ) {
        let mut p0 = mmm & 7;
        if !r {
            p0 |= 0x80;
        }
        if !x {
            p0 |= 0x40;
        }
        if !b {
            p0 |= 0x20;
        }
        if !r4 {
            p0 |= 0x10; // R4 inverted
        }
        if b4 {
            p0 |= 0x08; // B4 NOT inverted
        }
        let v_lo = vvvv & 0x0F;
        let mut p1 = (u8::from(w) << 7) | ((!v_lo & 0x0F) << 3) | (pp & 3);
        if !x4 {
            p1 |= 0x04; // X4 inverted (U bit)
        }
        let v4 = (vvvv & 0x10) != 0;
        let mut p2 = 0u8;
        if nd {
            p2 |= 0x10;
        }
        if !v4 {
            p2 |= 0x08; // v4 inverted
        }
        if nf {
            p2 |= 0x04;
        }
        self.bytes.extend_from_slice(&[0x62, p0, p1, p2]);
    }

    /// CCMP/CTEST EVEX: P1.vvvv is the DFV bitmap **not inverted**; P2[3:0] is SCC.
    /// ND/NF/v4 are unused (must be 0).
    pub(crate) fn emit_evex_ccmp(
        &mut self,
        r: bool,
        x: bool,
        b: bool,
        r4: bool,
        x4: bool,
        b4: bool,
        w: bool,
        dfv: u8,
        scc: u8,
        pp: u8,
    ) {
        let mut p0 = 0x04u8;
        if !r {
            p0 |= 0x80;
        }
        if !x {
            p0 |= 0x40;
        }
        if !b {
            p0 |= 0x20;
        }
        if !r4 {
            p0 |= 0x10;
        }
        if b4 {
            p0 |= 0x08;
        }
        let mut p1 = (u8::from(w) << 7) | ((dfv & 0x0F) << 3) | (pp & 3);
        if !x4 {
            p1 |= 0x04;
        }
        let p2 = scc & 0x0F;
        self.bytes.extend_from_slice(&[0x62, p0, p1, p2]);
    }

    /// After a REX2 map-0 encoding, collapse `D5 pp 0F opc` into `D5 (pp|0x80) opc`
    /// (M0=1, omit the 0F escape). Map 2/3 (`0F 38` / `0F 3A`) cannot use REX2.
    pub(crate) fn fixup_rex2_map1(&mut self, start: usize) -> Result<(), String> {
        let mut i = start;
        while i < self.bytes.len() {
            match self.bytes[i] {
                0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0x66 | 0x67 | 0xF0 | 0xF2 | 0xF3 => {
                    i += 1;
                }
                0xD5 => {
                    if i + 2 >= self.bytes.len() {
                        return Ok(());
                    }
                    if self.bytes[i + 2] == 0x0F {
                        if i + 3 < self.bytes.len() && matches!(self.bytes[i + 3], 0x38 | 0x3A) {
                            return Err(
                                "EGPR with 0F38/0F3A opcode requires APX EVEX (map 4)".into()
                            );
                        }
                        self.bytes[i + 1] |= 0x80; // M0
                        self.bytes.remove(i + 2);
                        for r in &mut self.relocations {
                            if r.offset as usize > i + 2 {
                                r.offset -= 1;
                            }
                        }
                    }
                    return Ok(());
                }
                _ => return Ok(()),
            }
        }
        Ok(())
    }

    /// Operand-size pp field for APX EVEX (replaces a leading 0x66).
    pub(crate) fn apx_pp(size: u8) -> u8 {
        if size == 2 { 1 } else { 0 }
    }

    fn gp_id_or_err(name: &str) -> Result<u8, String> {
        gp_id(name).ok_or_else(|| format!("not a GP register: {}", name))
    }

    /// Emit APX EVEX for a GP dest-in-r/m form (reg = ModRM.reg, rm = r/m,
    /// optional NDD in vvvv).
    pub(crate) fn emit_apx_evex_rr(
        &mut self,
        size: u8,
        reg: &str,
        rm: &str,
        ndd: Option<&str>,
        nf: bool,
    ) -> Result<(), String> {
        self.emit_apx_evex_rr_pp(size == 8, reg, rm, ndd, nf, Self::apx_pp(size))
    }

    /// Like `emit_apx_evex_rr` with an explicit EVEX.W / pp (map-2 promotions
    /// such as `adcx`/`adox`/`crc32`/`movbe` do not follow the ALU size→pp rule).
    pub(crate) fn emit_apx_evex_rr_pp(
        &mut self,
        w: bool,
        reg: &str,
        rm: &str,
        ndd: Option<&str>,
        nf: bool,
        pp: u8,
    ) -> Result<(), String> {
        let (r, r4) = if reg.is_empty() {
            (false, false)
        } else {
            gp_ext_bits(reg)
        };
        let (b, b4) = gp_ext_bits(rm);
        let (nd, vvvv) = match ndd {
            Some(n) => (true, Self::gp_id_or_err(n)?),
            None => (false, 0),
        };
        self.emit_evex_apx(r, false, b, r4, false, b4, w, vvvv, nd, nf, pp);
        Ok(())
    }

    pub(crate) fn emit_apx_evex_rm(
        &mut self,
        size: u8,
        reg: &str,
        mem: &MemoryOperand,
        ndd: Option<&str>,
        nf: bool,
    ) -> Result<(), String> {
        self.emit_apx_evex_rm_pp(size == 8, reg, mem, ndd, nf, Self::apx_pp(size))
    }

    pub(crate) fn emit_apx_evex_rm_pp(
        &mut self,
        w: bool,
        reg: &str,
        mem: &MemoryOperand,
        ndd: Option<&str>,
        nf: bool,
        pp: u8,
    ) -> Result<(), String> {
        let folded = fold_scale1_index(mem);
        let mem = folded.as_ref().unwrap_or(mem);
        let addr32 = mem.base.as_ref().is_some_and(|b| is_reg32(&b.name))
            || mem.index.as_ref().is_some_and(|i| is_reg32(&i.name));
        if addr32 {
            self.bytes.push(0x67);
        }
        let (r, r4) = if reg.is_empty() {
            (false, false)
        } else {
            gp_ext_bits(reg)
        };
        let (b, b4) = mem
            .base
            .as_ref()
            .map(|b| gp_ext_bits(&b.name))
            .unwrap_or((false, false));
        let (x, x4) = mem
            .index
            .as_ref()
            .map(|i| gp_ext_bits(&i.name))
            .unwrap_or((false, false));
        let (nd, vvvv) = match ndd {
            Some(n) => (true, Self::gp_id_or_err(n)?),
            None => (false, 0),
        };
        self.emit_evex_apx(r, x, b, r4, x4, b4, w, vvvv, nd, nf, pp);
        Ok(())
    }

    /// APX EVEX with ND=1 and vvvv=0. Used by ZU forms (`imulzu`, `setzu`)
    /// that set the NDD bit without an extra destination register.
    pub(crate) fn emit_apx_evex_rr_nd1(
        &mut self,
        size: u8,
        reg: &str,
        rm: &str,
        nf: bool,
    ) -> Result<(), String> {
        self.emit_apx_evex_rr_nd1_pp(size == 8, reg, rm, nf, Self::apx_pp(size))
    }

    pub(crate) fn emit_apx_evex_rr_nd1_pp(
        &mut self,
        w: bool,
        reg: &str,
        rm: &str,
        nf: bool,
        pp: u8,
    ) -> Result<(), String> {
        let (r, r4) = if reg.is_empty() {
            (false, false)
        } else {
            gp_ext_bits(reg)
        };
        let (b, b4) = gp_ext_bits(rm);
        self.emit_evex_apx(r, false, b, r4, false, b4, w, 0, true, nf, pp);
        Ok(())
    }

    pub(crate) fn emit_apx_evex_rm_nd1(
        &mut self,
        size: u8,
        reg: &str,
        mem: &MemoryOperand,
        nf: bool,
    ) -> Result<(), String> {
        self.emit_apx_evex_rm_nd1_pp(size == 8, reg, mem, nf, Self::apx_pp(size))
    }

    pub(crate) fn emit_apx_evex_rm_nd1_pp(
        &mut self,
        w: bool,
        reg: &str,
        mem: &MemoryOperand,
        nf: bool,
        pp: u8,
    ) -> Result<(), String> {
        let folded = fold_scale1_index(mem);
        let mem = folded.as_ref().unwrap_or(mem);
        let addr32 = mem.base.as_ref().is_some_and(|b| is_reg32(&b.name))
            || mem.index.as_ref().is_some_and(|i| is_reg32(&i.name));
        if addr32 {
            self.bytes.push(0x67);
        }
        let (r, r4) = if reg.is_empty() {
            (false, false)
        } else {
            gp_ext_bits(reg)
        };
        let (b, b4) = mem
            .base
            .as_ref()
            .map(|b| gp_ext_bits(&b.name))
            .unwrap_or((false, false));
        let (x, x4) = mem
            .index
            .as_ref()
            .map(|i| gp_ext_bits(&i.name))
            .unwrap_or((false, false));
        self.emit_evex_apx(r, x, b, r4, x4, b4, w, 0, true, nf, pp);
        Ok(())
    }

    /// BMI/BMI2 EVEX: dest in ModRM.reg, vvvv is a source (ND=0), map 2 or 3.
    pub(crate) fn emit_apx_evex_vvvv_rr(
        &mut self,
        w: bool,
        reg: &str,
        rm: &str,
        vvvv: &str,
        nf: bool,
        pp: u8,
        mmm: u8,
    ) -> Result<(), String> {
        let (r, r4) = if reg.is_empty() {
            (false, false)
        } else {
            gp_ext_bits(reg)
        };
        let (b, b4) = gp_ext_bits(rm);
        let v = Self::gp_id_or_err(vvvv)?;
        self.emit_evex_apx_map(r, false, b, r4, false, b4, w, v, false, nf, pp, mmm);
        Ok(())
    }

    pub(crate) fn emit_apx_evex_vvvv_rm(
        &mut self,
        w: bool,
        reg: &str,
        mem: &MemoryOperand,
        vvvv: &str,
        nf: bool,
        pp: u8,
        mmm: u8,
    ) -> Result<(), String> {
        let folded = fold_scale1_index(mem);
        let mem = folded.as_ref().unwrap_or(mem);
        let addr32 = mem.base.as_ref().is_some_and(|b| is_reg32(&b.name))
            || mem.index.as_ref().is_some_and(|i| is_reg32(&i.name));
        if addr32 {
            self.bytes.push(0x67);
        }
        let (r, r4) = if reg.is_empty() {
            (false, false)
        } else {
            gp_ext_bits(reg)
        };
        let (b, b4) = mem
            .base
            .as_ref()
            .map(|b| gp_ext_bits(&b.name))
            .unwrap_or((false, false));
        let (x, x4) = mem
            .index
            .as_ref()
            .map(|i| gp_ext_bits(&i.name))
            .unwrap_or((false, false));
        let v = Self::gp_id_or_err(vvvv)?;
        self.emit_evex_apx_map(r, x, b, r4, x4, b4, w, v, false, nf, pp, mmm);
        Ok(())
    }

    pub(crate) fn emit_evex_ccmp_rr(
        &mut self,
        w: bool,
        reg: &str,
        rm: &str,
        dfv: u8,
        scc: u8,
        pp: u8,
    ) -> Result<(), String> {
        let (r, r4) = if reg.is_empty() {
            (false, false)
        } else {
            gp_ext_bits(reg)
        };
        let (b, b4) = gp_ext_bits(rm);
        self.emit_evex_ccmp(r, false, b, r4, false, b4, w, dfv, scc, pp);
        Ok(())
    }

    pub(crate) fn emit_evex_ccmp_rm(
        &mut self,
        w: bool,
        reg: &str,
        mem: &MemoryOperand,
        dfv: u8,
        scc: u8,
        pp: u8,
    ) -> Result<(), String> {
        let folded = fold_scale1_index(mem);
        let mem = folded.as_ref().unwrap_or(mem);
        let addr32 = mem.base.as_ref().is_some_and(|b| is_reg32(&b.name))
            || mem.index.as_ref().is_some_and(|i| is_reg32(&i.name));
        if addr32 {
            self.bytes.push(0x67);
        }
        let (r, r4) = if reg.is_empty() {
            (false, false)
        } else {
            gp_ext_bits(reg)
        };
        let (b, b4) = mem
            .base
            .as_ref()
            .map(|b| gp_ext_bits(&b.name))
            .unwrap_or((false, false));
        let (x, x4) = mem
            .index
            .as_ref()
            .map(|i| gp_ext_bits(&i.name))
            .unwrap_or((false, false));
        self.emit_evex_ccmp(r, x, b, r4, x4, b4, w, dfv, scc, pp);
        Ok(())
    }

    pub(crate) fn apx_wants_evex(&self) -> bool {
        self.apx_nf || self.apx_evex
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
        let (r, r4) = if reg.is_empty() {
            (false, false)
        } else {
            gp_ext_bits(reg)
        };
        let (b, b4) = mem
            .base
            .as_ref()
            .map(|b| gp_ext_bits(&b.name))
            .unwrap_or((false, false));
        let (x, x4) = mem
            .index
            .as_ref()
            .map(|i| gp_ext_bits(&i.name))
            .unwrap_or((false, false));
        self.emit_rex_or_rex2(w, r, x, b, r4, x4, b4, is_rex_required_8bit(reg));
    }

    /// Emit REX prefix for unary operation on register.
    pub(crate) fn emit_rex_unary(&mut self, size: u8, rm: &str) {
        let w = size == 8;
        let (b, b4) = gp_ext_bits(rm);
        self.emit_rex_or_rex2(
            w,
            false,
            false,
            b,
            false,
            false,
            b4,
            is_rex_required_8bit(rm),
        );
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

    /// Skip group-2/3/4 legacy prefixes; return the index of the first
    /// opcode/REX/VEX/EVEX byte of the instruction currently in `self.bytes`.
    fn prefix_start(&self) -> usize {
        let mut i = 0;
        while i < self.bytes.len() {
            match self.bytes[i] {
                0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0x66 | 0x67 | 0xF0 | 0xF2 | 0xF3 => {
                    i += 1
                }
                _ => break,
            }
        }
        i
    }

    /// APX EVEX of a *relaxable* legacy-map ALU (ADD/OR/ADC/SBB/AND/SUB/XOR/CMP,
    /// TEST, IMUL, MOV). GAS 2.47 emits `R_X86_64_CODE_6_GOTPCRELX` for these
    /// and plain `R_X86_64_GOTPCREL` for AVX-512 EVEX and APX map-4 BMI/ADX
    /// (`andn`, `crc32`, `adcx`, …) which the linker cannot rewrite.
    fn apx_evex_legacy_relaxable(&self) -> bool {
        let i = self.prefix_start();
        if self.bytes.len() < i + 5 || self.bytes[i] != 0x62 {
            return false;
        }
        // EVEX P0.mmm: 4 = APX-promoted legacy/map-4. AVX-512 uses 1/2/3.
        if self.bytes[i + 1] & 7 != 4 {
            return false;
        }
        matches!(
            self.bytes[i + 4],
            0x01 | 0x03
                | 0x09
                | 0x0B
                | 0x11
                | 0x13
                | 0x19
                | 0x1B
                | 0x21
                | 0x23
                | 0x29
                | 0x2B
                | 0x31
                | 0x33
                | 0x39
                | 0x3B
                | 0x85
                | 0x8B
                | 0xAF
        )
    }

    /// Relaxable GOTPCREL reloc for the instruction currently in `self.bytes`.
    ///
    /// REX (0x40-0x4F) → `R_X86_64_REX_GOTPCRELX` (42).
    /// REX2 (0xD5)     → `R_X86_64_CODE_4_GOTPCRELX` (43) — the displacement
    /// sits 4 bytes after the prefix start (`D5 pp opc modrm disp32`).
    /// APX EVEX ALU    → `R_X86_64_CODE_6_GOTPCRELX` (49) — disp sits 6 bytes
    /// after `0x62` (`62 p1 p2 p3 opc modrm disp32`).
    /// AVX-512 EVEX / APX map-4 BMI → `R_X86_64_GOTPCREL` (9), matching GAS.
    /// Anything else   → `R_X86_64_GOTPCRELX` (41).
    pub(crate) fn gotpcrel_x_type(&self) -> u32 {
        let i = self.prefix_start();
        match self.bytes.get(i).copied() {
            Some(0x40..=0x4F) => R_X86_64_REX_GOTPCRELX,
            Some(0xD5) => R_X86_64_CODE_4_GOTPCRELX,
            Some(0x62) if self.apx_evex_legacy_relaxable() => R_X86_64_CODE_6_GOTPCRELX,
            Some(0x62) => R_X86_64_GOTPCREL,
            _ => R_X86_64_GOTPCRELX,
        }
    }

    /// GOTTPOFF reloc class for the instruction currently in `self.bytes`.
    pub(crate) fn gottpoff_type(&self) -> u32 {
        let i = self.prefix_start();
        match self.bytes.get(i).copied() {
            Some(0xD5) => R_X86_64_CODE_4_GOTTPOFF,
            Some(0x62) if self.apx_evex_legacy_relaxable() => R_X86_64_CODE_6_GOTTPOFF,
            _ => R_X86_64_GOTTPOFF,
        }
    }

    /// TLSDESC reloc class for the instruction currently in `self.bytes`.
    pub(crate) fn tlsdesc_type(&self) -> u32 {
        let i = self.prefix_start();
        match self.bytes.get(i).copied() {
            Some(0xD5) => R_X86_64_CODE_4_GOTPC32_TLSDESC,
            Some(0x62) if self.apx_evex_legacy_relaxable() => R_X86_64_CODE_6_GOTPC32_TLSDESC,
            _ => R_X86_64_GOTPC32_TLSDESC,
        }
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
                            "gotpcrel" => self.gotpcrel_x_type(),
                            "gotpcrelx" => R_X86_64_GOTPCRELX,
                            "rex_gotpcrelx" => R_X86_64_REX_GOTPCRELX,
                            "code_4_gotpcrelx" => R_X86_64_CODE_4_GOTPCRELX,
                            "code_6_gotpcrelx" => R_X86_64_CODE_6_GOTPCRELX,
                            "gottpoff" => self.gottpoff_type(),
                            "tlsdesc" => self.tlsdesc_type(),
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
                            "symbol-difference displacement `{} - {}` is not valid with RIP-relative addressing",
                            a, b
                        ));
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
                // Non-RIP `sym@GOTPCREL(%reg)` is not relaxable: GAS 2.47
                // emits plain `R_X86_64_GOTPCREL` (9) even when the insn is
                // REX2 (`addq foo@GOTPCREL(%rax), %r16` → type 9, not 43).
                // GOTTPOFF/TLSDESC without %rip are rejected by GAS; keep
                // the classic types if they ever reach the encoder.
                let reloc_type = match modifier.to_ascii_lowercase().as_str() {
                    "tpoff" => R_X86_64_TPOFF32,
                    "gotpcrel" => R_X86_64_GOTPCREL,
                    "gottpoff" => R_X86_64_GOTTPOFF,
                    "tlsdesc" => R_X86_64_GOTPC32_TLSDESC,
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
                R_X86_64_PC32
                | R_X86_64_PLT32
                | R_X86_64_GOTPCREL
                | R_X86_64_GOTPCRELX
                | R_X86_64_REX_GOTPCRELX
                | R_X86_64_CODE_4_GOTPCRELX
                | R_X86_64_CODE_6_GOTPCRELX
                | R_X86_64_GOTTPOFF
                | R_X86_64_CODE_4_GOTTPOFF
                | R_X86_64_CODE_6_GOTTPOFF
                | R_X86_64_GOTPC32_TLSDESC
                | R_X86_64_CODE_4_GOTPC32_TLSDESC
                | R_X86_64_CODE_6_GOTPC32_TLSDESC => {
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
