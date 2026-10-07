//! ELF object file writer for AArch64.
//!
//! Takes parsed assembly statements and produces an ELF .o (relocatable) file
//! with proper sections, symbols, and relocations for AArch64/ELF64.
//!
//! Uses `ElfWriterBase` from `elf.rs` for shared section/symbol/relocation
//! management, directive processing, and ELF serialization. This file only
//! contains AArch64-specific logic: instruction encoding dispatch, branch
//! resolution (AArch64 relocation types), and symbol difference resolution.

// ELF writer helpers; some section/relocation utilities defined for completeness.
#![allow(dead_code)]

use super::encoder::{EncodeResult, RelocType, encode_instruction};
use super::parser::{AsmDirective, AsmStatement, DataValue, Operand, SizeExpr, SymbolKind};
use crate::backend::elf::{
    self, ELFCLASS64, EM_AARCH64, ElfWriterBase, ObjReloc, STT_FUNC, STT_NOTYPE, STT_OBJECT,
    STT_TLS, STV_HIDDEN, STV_INTERNAL, STV_PROTECTED,
};

/// AArch64 NOP instruction: `d503201f` in little-endian
const AARCH64_NOP: [u8; 4] = [0x1f, 0x20, 0x03, 0xd5];

/// The ELF writer for AArch64.
///
/// Composes with `ElfWriterBase` for shared infrastructure and adds
/// AArch64-specific branch resolution and symbol difference handling.
pub struct ElfWriter {
    /// Shared ELF writer state (sections, symbols, labels, directives)
    pub base: ElfWriterBase,
    /// Pending branch/local relocations to resolve after all labels are known.
    /// Includes all branch-type relocs (B, BL, B.cond, CBZ, TBZ, etc.) plus
    /// local .L-prefixed label references, so intra-section targets can be
    /// resolved at assembly time without emitting unnecessary relocations.
    pending_branch_relocs: Vec<PendingReloc>,
    /// Pending symbol differences to resolve after all labels are known
    pending_sym_diffs: Vec<PendingSymDiff>,
    /// Pending raw expressions to resolve after all labels are known
    pending_exprs: Vec<PendingExpr>,
    /// Pending instructions with symbolic offset expressions (resolved after labels are known)
    pending_instructions: Vec<PendingInstruction>,
}

struct PendingReloc {
    section: String,
    offset: u64,
    reloc_type: u32,
    symbol: String,
    addend: i64,
}

/// Returns true if the given ELF relocation type is a branch/jump relocation
/// that should be deferred for intra-section resolution at assembly time.
fn is_branch_reloc_type(elf_type: u32) -> bool {
    matches!(
        elf_type,
        279 |  // R_AARCH64_TSTBR14
        280 |  // R_AARCH64_CONDBR19
        282 |  // R_AARCH64_JUMP26
        283 // R_AARCH64_CALL26
    )
}

/// A pending raw expression to be resolved after all labels are known.
struct PendingExpr {
    section: String,
    offset: u64,
    expr: String,
    size: usize,
}

/// A pending symbol difference to be resolved after all labels are known.
struct PendingSymDiff {
    /// Section containing the data directive
    section: String,
    /// Offset within that section where the value should be written
    offset: u64,
    /// The positive symbol (A in A - B)
    sym_a: String,
    /// The negative symbol (B in A - B)
    sym_b: String,
    /// Extra addend
    extra_addend: i64,
    /// Size in bytes (1, 4, or 8)
    size: usize,
}

/// A pending instruction whose operands contain symbolic expressions.
/// A NOP placeholder is emitted at the instruction offset; after all labels
/// are known, the expression is evaluated, operands are updated, and the
/// instruction is re-encoded and patched in place.
struct PendingInstruction {
    section: String,
    offset: u64,
    mnemonic: String,
    operands: Vec<Operand>,
    raw_operands: String,
}

impl ElfWriter {
    pub fn new() -> Self {
        Self {
            base: ElfWriterBase::new(AARCH64_NOP, 4),
            pending_branch_relocs: Vec::new(),
            pending_sym_diffs: Vec::new(),
            pending_exprs: Vec::new(),
            pending_instructions: Vec::new(),
        }
    }

    /// Resolve pending symbol differences after all labels are known.
    fn resolve_sym_diffs(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.pending_sym_diffs);
        for diff in &pending {
            let sym_a_info = self.base.labels.get(&diff.sym_a).cloned();
            let sym_b_info = self.base.labels.get(&diff.sym_b).cloned();

            match (sym_a_info, sym_b_info) {
                (Some((sec_a, off_a)), Some((sec_b, off_b))) => {
                    if sec_a == sec_b {
                        // Same section: resolve at assembly time by patching the data
                        let value = (off_a as i64) - (off_b as i64) + diff.extra_addend;
                        if let Some(section) = self.base.sections.get_mut(&diff.section) {
                            let off = diff.offset as usize;
                            if diff.size == 1 && off < section.data.len() {
                                section.data[off] = value as u8;
                            } else if diff.size == 4 && off + 4 <= section.data.len() {
                                section.data[off..off + 4]
                                    .copy_from_slice(&(value as i32).to_le_bytes());
                            } else if diff.size == 8 && off + 8 <= section.data.len() {
                                section.data[off..off + 8].copy_from_slice(&value.to_le_bytes());
                            }
                        }
                    } else {
                        // Cross-section: emit R_AARCH64_PREL32 or PREL64 based on size
                        let addend =
                            off_a as i64 + diff.offset as i64 - off_b as i64 + diff.extra_addend;
                        let reloc_type = if diff.size == 8 {
                            RelocType::Prel64.elf_type()
                        } else {
                            RelocType::Prel32.elf_type()
                        };
                        if let Some(section) = self.base.sections.get_mut(&diff.section) {
                            section.relocs.push(ObjReloc {
                                offset: diff.offset,
                                reloc_type,
                                symbol_name: sec_a.clone(),
                                addend,
                            });
                        }
                    }
                }
                _ => {
                    // Forward-referenced or external symbols: emit PREL32 or PREL64 based on size
                    let reloc_type = if diff.size == 8 {
                        RelocType::Prel64.elf_type()
                    } else {
                        RelocType::Prel32.elf_type()
                    };
                    if let Some(section) = self.base.sections.get_mut(&diff.section) {
                        section.relocs.push(ObjReloc {
                            offset: diff.offset,
                            reloc_type,
                            symbol_name: diff.sym_a.clone(),
                            addend: diff.extra_addend,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolve pending raw expressions by substituting label offsets and evaluating.
    fn resolve_pending_exprs(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.pending_exprs);
        for pexpr in &pending {
            // Substitute label names with their offset values
            let mut expr = pexpr.expr.clone();

            // Collect all label names, sorted longest first to avoid partial replacements
            let mut label_names: Vec<&String> = self.base.labels.keys().collect();
            label_names.sort_by_key(|name| std::cmp::Reverse(name.len()));

            let mut all_resolved = true;
            for label_name in &label_names {
                if expr.contains(label_name.as_str()) {
                    if let Some((_section, offset)) = self.base.labels.get(*label_name) {
                        expr = expr.replace(label_name.as_str(), &offset.to_string());
                    } else {
                        all_resolved = false;
                    }
                }
            }

            if all_resolved {
                // Try to evaluate the expression
                if let Ok(val) = crate::backend::asm_expr::parse_integer_expr(&expr) {
                    if let Some(section) = self.base.sections.get_mut(&pexpr.section) {
                        let off = pexpr.offset as usize;
                        match pexpr.size {
                            1 if off < section.data.len() => {
                                section.data[off] = val as u8;
                            }
                            2 if off + 2 <= section.data.len() => {
                                section.data[off..off + 2]
                                    .copy_from_slice(&(val as i16).to_le_bytes());
                            }
                            4 if off + 4 <= section.data.len() => {
                                section.data[off..off + 4]
                                    .copy_from_slice(&(val as i32).to_le_bytes());
                            }
                            8 if off + 8 <= section.data.len() => {
                                section.data[off..off + 8].copy_from_slice(&val.to_le_bytes());
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Couldn't evaluate - emit as symbol reference
                    if let Some(section) = self.base.sections.get_mut(&pexpr.section) {
                        section.relocs.push(ObjReloc {
                            offset: pexpr.offset,
                            reloc_type: RelocType::Abs32.elf_type(),
                            symbol_name: pexpr.expr.clone(),
                            addend: 0,
                        });
                    }
                }
            } else {
                // Unresolved symbols - emit as relocation
                if let Some(section) = self.base.sections.get_mut(&pexpr.section) {
                    section.relocs.push(ObjReloc {
                        offset: pexpr.offset,
                        reloc_type: RelocType::Abs32.elf_type(),
                        symbol_name: pexpr.expr.clone(),
                        addend: 0,
                    });
                }
            }
        }
        Ok(())
    }

    /// Resolve pending instructions that contain symbolic offset expressions.
    /// After all labels are positioned, substitute label names with offsets,
    /// evaluate the expression, update operands, re-encode, and patch in place.
    fn resolve_pending_instructions(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.pending_instructions);
        for pinstr in &pending {
            // Resolve each operand that has a deferred expression
            let resolved_operands: Vec<Operand> = pinstr
                .operands
                .iter()
                .map(|op| match op {
                    Operand::MemExpr {
                        base,
                        expr,
                        writeback,
                    } => {
                        if let Some(val) = self.resolve_label_expr(expr) {
                            if *writeback {
                                Operand::MemPreIndex {
                                    base: base.clone(),
                                    offset: val,
                                }
                            } else {
                                Operand::Mem {
                                    base: base.clone(),
                                    offset: val,
                                }
                            }
                        } else {
                            op.clone()
                        }
                    }
                    Operand::Expr(expr) => {
                        if let Some(val) = self.resolve_label_expr(expr) {
                            Operand::Imm(val)
                        } else {
                            op.clone()
                        }
                    }
                    _ => op.clone(),
                })
                .collect();

            // Re-encode the instruction with resolved operands
            match encode_instruction(&pinstr.mnemonic, &resolved_operands, &pinstr.raw_operands) {
                Ok(EncodeResult::Word(word)) => {
                    if let Some(section) = self.base.sections.get_mut(&pinstr.section) {
                        let off = pinstr.offset as usize;
                        if off + 4 <= section.data.len() {
                            section.data[off..off + 4].copy_from_slice(&word.to_le_bytes());
                        }
                    }
                }
                Ok(EncodeResult::WordWithReloc { word, reloc }) => {
                    let elf_type = reloc.reloc_type.elf_type();
                    if let Some(section) = self.base.sections.get_mut(&pinstr.section) {
                        let off = pinstr.offset as usize;
                        if off + 4 <= section.data.len() {
                            section.data[off..off + 4].copy_from_slice(&word.to_le_bytes());
                        }
                        let is_local = reloc.symbol.starts_with(".L")
                            || reloc.symbol.starts_with(".l")
                            || reloc.symbol == ".";
                        if is_local || is_branch_reloc_type(elf_type) {
                            self.pending_branch_relocs.push(PendingReloc {
                                section: pinstr.section.clone(),
                                offset: pinstr.offset,
                                reloc_type: elf_type,
                                symbol: reloc.symbol.clone(),
                                addend: reloc.addend,
                            });
                        } else {
                            section.relocs.push(ObjReloc {
                                offset: pinstr.offset,
                                reloc_type: elf_type,
                                symbol_name: reloc.symbol,
                                addend: reloc.addend,
                            });
                        }
                    }
                }
                Ok(EncodeResult::Words(words)) => {
                    if let Some(section) = self.base.sections.get_mut(&pinstr.section) {
                        let off = pinstr.offset as usize;
                        for (j, word) in words.iter().enumerate() {
                            let wo = off + j * 4;
                            if wo + 4 <= section.data.len() {
                                section.data[wo..wo + 4].copy_from_slice(&word.to_le_bytes());
                            }
                        }
                    }
                }
                Ok(EncodeResult::Skip) => {}
                Err(e) => {
                    // This branch used to print a warning and continue, which
                    // left the NOP placeholder emitted at line 792 in the
                    // output: a file containing `str z15, [x9, #45, mul vl]`
                    // assembled to a bare `nop` and exited 0. An assembler
                    // that cannot encode an instruction must say so -- silently
                    // substituting a NOP turns an unsupported instruction into
                    // a program that runs and does the wrong thing.
                    return Err(format!(
                        "{}: failed to resolve deferred instruction `{} {}`: {}",
                        pinstr.section, pinstr.mnemonic, pinstr.raw_operands, e
                    ));
                }
            }
        }
        Ok(())
    }

    /// Substitute all known label names in an expression string with their byte
    /// offsets, then evaluate the resulting numeric expression.
    fn resolve_label_expr(&self, expr: &str) -> Option<i64> {
        let mut resolved = expr.to_string();

        // Collect all label names, sorted longest first to avoid partial replacements
        let mut label_names: Vec<&String> = self.base.labels.keys().collect();
        label_names.sort_by_key(|name| std::cmp::Reverse(name.len()));

        for label_name in &label_names {
            if resolved.contains(label_name.as_str()) {
                if let Some((_section, offset)) = self.base.labels.get(*label_name) {
                    resolved = resolved.replace(label_name.as_str(), &offset.to_string());
                }
            }
        }

        crate::backend::asm_expr::parse_integer_expr(&resolved).ok()
    }

    /// Process all parsed assembly statements.
    pub fn process_statements(&mut self, statements: &[AsmStatement]) -> Result<(), String> {
        for stmt in statements {
            self.process_statement(stmt)?;
        }
        // Merge subsections (e.g., .text.__subsection.1 → .text) before resolving
        let remap = self.base.merge_subsections();
        // Fix up pending references that pointed to now-merged subsection names
        if !remap.is_empty() {
            for reloc in &mut self.pending_branch_relocs {
                if let Some((parent, offset_adj)) = remap.get(&reloc.section) {
                    reloc.offset += offset_adj;
                    reloc.section = parent.clone();
                }
            }
            for diff in &mut self.pending_sym_diffs {
                if let Some((parent, offset_adj)) = remap.get(&diff.section) {
                    diff.offset += offset_adj;
                    diff.section = parent.clone();
                }
            }
            for expr in &mut self.pending_exprs {
                if let Some((parent, offset_adj)) = remap.get(&expr.section) {
                    expr.offset += offset_adj;
                    expr.section = parent.clone();
                }
            }
            for instr in &mut self.pending_instructions {
                if let Some((parent, offset_adj)) = remap.get(&instr.section) {
                    instr.offset += offset_adj;
                    instr.section = parent.clone();
                }
            }
        }
        // Resolve symbol differences first (needs all labels to be known)
        self.resolve_sym_diffs()?;
        self.resolve_pending_exprs()?;
        self.resolve_pending_instructions()?;
        self.resolve_local_branches()?;
        Ok(())
    }

    fn process_statement(&mut self, stmt: &AsmStatement) -> Result<(), String> {
        match stmt {
            AsmStatement::Empty => Ok(()),

            AsmStatement::Label(name) => {
                self.base.ensure_text_section();
                let section = self.base.current_section.clone();
                let offset = self.base.current_offset();
                self.base.labels.insert(name.clone(), (section, offset));
                Ok(())
            }

            AsmStatement::Directive(dir) => self.process_directive(dir),

            AsmStatement::Instruction {
                mnemonic,
                operands,
                raw_operands,
            } => self.process_instruction(mnemonic, operands, raw_operands),

            AsmStatement::LdrLiteralPool { .. } => {
                // Should have been expanded by expand_literal_pools() before reaching here
                Err("LdrLiteralPool should have been expanded before ELF writing".to_string())
            }
        }
    }

    fn process_directive(&mut self, dir: &AsmDirective) -> Result<(), String> {
        match dir {
            AsmDirective::Section(sec) => {
                self.base.process_section_directive(
                    &sec.name,
                    sec.flags.as_deref().unwrap_or(""),
                    sec.flags.is_some(), // flags are explicit when provided
                    sec.section_type.as_deref(),
                );
                Ok(())
            }

            AsmDirective::PushSection(sec) => {
                self.base.push_section(
                    &sec.name,
                    sec.flags.as_deref().unwrap_or(""),
                    sec.flags.is_some(),
                    sec.section_type.as_deref(),
                );
                Ok(())
            }

            AsmDirective::PopSection => {
                self.base.pop_section();
                Ok(())
            }

            AsmDirective::Previous => {
                self.base.restore_previous_section();
                Ok(())
            }

            AsmDirective::Subsection(n) => {
                self.base.set_subsection(*n);
                Ok(())
            }

            AsmDirective::Global(sym) => {
                for s in sym.split(',') {
                    let s = s.trim();
                    if !s.is_empty() {
                        self.base.set_global(s);
                    }
                }
                Ok(())
            }
            AsmDirective::Weak(sym) => {
                self.base.set_weak(sym);
                Ok(())
            }
            AsmDirective::Hidden(sym) => {
                self.base.set_visibility(sym, STV_HIDDEN);
                Ok(())
            }
            AsmDirective::Protected(sym) => {
                self.base.set_visibility(sym, STV_PROTECTED);
                Ok(())
            }
            AsmDirective::Internal(sym) => {
                self.base.set_visibility(sym, STV_INTERNAL);
                Ok(())
            }

            AsmDirective::SymbolType(sym, kind) => {
                let st = match kind {
                    SymbolKind::Function => STT_FUNC,
                    SymbolKind::Object => STT_OBJECT,
                    SymbolKind::TlsObject => STT_TLS,
                    SymbolKind::NoType => STT_NOTYPE,
                };
                self.base.set_symbol_type(sym, st);
                Ok(())
            }

            AsmDirective::Size(sym, expr) => {
                match expr {
                    SizeExpr::CurrentMinusSymbol(label) => {
                        self.base.set_symbol_size(sym, Some(label), None);
                    }
                    SizeExpr::Constant(size) => {
                        self.base.set_symbol_size(sym, None, Some(*size));
                    }
                }
                Ok(())
            }

            AsmDirective::Align {
                bytes,
                max_pad,
                fill,
            } => {
                self.base.align_to_capped_ex(*bytes, *max_pad, *fill)?;
                Ok(())
            }
            AsmDirective::Balign(bytes) => {
                self.base.align_to(*bytes)?;
                Ok(())
            }

            AsmDirective::Byte(vals) => self.emit_data_values(vals, 1),

            AsmDirective::Short(vals) => {
                for val in vals {
                    // RAW-value NOBITS rule: `.short 256` stores two zero
                    // bytes but GAS still rejects the non-zero value.
                    self.base.nobits_guard(*val != 0)?;
                    self.base.emit_bytes(&(*val as u16).to_le_bytes())?;
                }
                Ok(())
            }

            AsmDirective::Long(vals) => self.emit_data_values(vals, 4),
            AsmDirective::Quad(vals) => self.emit_data_values(vals, 8),

            AsmDirective::Zero(size, fill) => {
                self.base.emit_fill(*size, *fill)?;
                Ok(())
            }
            AsmDirective::Fill { total, elem, value } => {
                self.base.emit_fill_pattern(*total, *elem, *value)?;
                Ok(())
            }
            AsmDirective::Asciz(bytes) => {
                self.base.nobits_string_guard(bytes)?;
                self.base.emit_bytes(bytes)?;
                Ok(())
            }
            AsmDirective::Ascii(bytes) => {
                self.base.nobits_string_guard(bytes)?;
                self.base.emit_bytes(bytes)?;
                Ok(())
            }

            AsmDirective::Comm(sym, size, align) => {
                self.base.emit_comm(sym, *size, *align);
                Ok(())
            }

            AsmDirective::Local(_) => Ok(()),

            AsmDirective::Set(alias, target) => {
                self.base.set_alias(alias, target);
                Ok(())
            }

            AsmDirective::Incbin { path, skip, count } => {
                let data = std::fs::read(path)
                    .map_err(|e| format!(".incbin: failed to read '{}': {}", path, e))?;
                let skip = *skip as usize;
                let data = if skip < data.len() {
                    &data[skip..]
                } else {
                    &[]
                };
                let data = match count {
                    Some(c) => {
                        let c = *c as usize;
                        if c < data.len() { &data[..c] } else { data }
                    }
                    None => data,
                };
                self.base.emit_bytes(data)?;
                Ok(())
            }

            AsmDirective::RawBytes(bytes) => {
                self.base.emit_bytes(bytes)?;
                Ok(())
            }

            AsmDirective::Org { expr, fill } => self.process_org(expr, *fill),

            AsmDirective::Cfi | AsmDirective::Ignored | AsmDirective::Ltorg => Ok(()),
        }
    }

    /// `.org new-lc, fill` — pad the current section with `fill` up to byte
    /// offset `new-lc` from the section start (GAS semantics).
    ///
    /// Evaluated immediately: unlike `.word` expressions, padding changes
    /// every offset that follows it, so deferral is impossible. The
    /// expression may reference `.` (the current location counter) and
    /// labels that are already defined in the current section; forward
    /// references and cross-section symbols are errors, matching GAS.
    /// Backward targets error too ("attempt to move .org backwards") — a
    /// silent no-op would corrupt vector-table layouts.
    fn process_org(&mut self, expr: &str, fill: u8) -> Result<(), String> {
        let section = self.base.current_section.clone();
        let current = self
            .base
            .sections
            .get(&section)
            .ok_or_else(|| ".org: no active section".to_string())?
            .data
            .len() as i64;
        let target = self.eval_org_expr(expr, &section, current)?;
        if target < current {
            return Err(format!(
                ".org: attempt to move .org backwards (to byte {} from byte {})",
                target, current
            ));
        }
        let padding = (target - current) as usize;
        // `.org` is the same constant-source / unbounded-output amplifier
        // as `.zero`: refuse the materialization before touching memory
        // (GAS dies writing it too — nonzero exit either way).
        if padding > crate::backend::elf::MAX_DIRECTIVE_FILL {
            return Err(format!(
                ".org: {padding} bytes of fill exceed the {}-byte limit",
                crate::backend::elf::MAX_DIRECTIVE_FILL
            ));
        }
        self.base.ensure_default_section();
        self.base.charge_fill(padding)?;
        if padding > 0 {
            // `.org` pads with its fill byte (default ZERO), even in an
            // executable section — it is a positioning directive, not an
            // alignment one (GAS emits plain fill bytes too).
            let data = &mut self
                .base
                .sections
                .get_mut(&section)
                .ok_or_else(|| ".org: no active section".to_string())?
                .data;
            data.resize(data.len() + padding, fill);
        }
        Ok(())
    }

    /// Substitute the location counter and same-section labels in a `.org`
    /// expression, then evaluate the result as a constant expression.
    ///
    /// Scanning is token-boundary aware: an identifier is a maximal run of
    /// `[A-Za-z0-9_.$]`, so the `.` inside `.Lventry_start` is part of the
    /// label and only a *standalone* `.` token becomes the location counter.
    fn eval_org_expr(&self, expr: &str, section: &str, current: i64) -> Result<i64, String> {
        let bytes = expr.as_bytes();
        let mut rebuilt = String::with_capacity(expr.len() + 16);
        let mut i = 0usize;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'$' {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'_'
                        || bytes[i] == b'.'
                        || bytes[i] == b'$')
                {
                    i += 1;
                }
                let token = &expr[start..i];
                if token == "." {
                    rebuilt.push_str(&current.to_string());
                } else if let Some((sym_sec, off)) = self.base.labels.get(token) {
                    if sym_sec != section {
                        return Err(format!(
                            ".org: symbol '{}' is in section '{}', not the current section",
                            token, sym_sec
                        ));
                    }
                    rebuilt.push_str(&off.to_string());
                } else if crate::backend::asm_expr::parse_integer_expr(token).is_ok() {
                    // Numeric literal (decimal, hex 0x, binary 0b, ...).
                    // Checked after labels so that a label can never be
                    // shadowed, and before the error arm so hex/binary
                    // constants survive the scan. Numeric *label*
                    // references (`662b`) parse as neither and are
                    // rejected below — they are not supported in `.org`.
                    rebuilt.push_str(token);
                } else {
                    return Err(format!(
                        ".org: unknown or forward-referenced symbol '{}' (padding cannot be deferred, so forward references are unsupported)",
                        token
                    ));
                }
                continue;
            }
            rebuilt.push(c as char);
            i += 1;
        }
        crate::backend::asm_expr::parse_integer_expr(&rebuilt)
    }

    /// Emit typed data values (Long or Quad) with proper relocations.
    fn emit_data_values(&mut self, vals: &[DataValue], size: usize) -> Result<(), String> {
        for val in vals {
            match val {
                DataValue::Integer(v) => {
                    self.base.emit_data_integer(*v, size)?;
                }
                DataValue::Symbol(sym) => {
                    let reloc_type = if size == 4 {
                        RelocType::Abs32.elf_type()
                    } else {
                        RelocType::Abs64.elf_type()
                    };
                    self.base.emit_data_symbol_ref(sym, 0, size, reloc_type)?;
                }
                DataValue::SymbolOffset(sym, addend) => {
                    let reloc_type = if size == 4 {
                        RelocType::Abs32.elf_type()
                    } else {
                        RelocType::Abs64.elf_type()
                    };
                    self.base
                        .emit_data_symbol_ref(sym, *addend, size, reloc_type)?;
                }
                DataValue::SymbolDiff(sym_a, sym_b) => {
                    self.record_sym_diff(sym_a, sym_b, 0, size)?;
                }
                DataValue::SymbolDiffAddend(sym_a, sym_b, addend) => {
                    self.record_sym_diff(sym_a, sym_b, *addend, size)?;
                }
                DataValue::Expr(expr) => {
                    let section = self.base.current_section.clone();
                    let offset = self.base.current_offset();
                    self.pending_exprs.push(PendingExpr {
                        section,
                        offset,
                        expr: expr.clone(),
                        size,
                    });
                    self.base.emit_placeholder(size)?;
                }
            }
        }
        Ok(())
    }

    /// Record a pending symbol difference for deferred resolution.
    fn record_sym_diff(
        &mut self,
        sym_a: &str,
        sym_b: &str,
        extra_addend: i64,
        size: usize,
    ) -> Result<(), String> {
        let section = self.base.current_section.clone();
        let offset = self.base.current_offset();
        self.pending_sym_diffs.push(PendingSymDiff {
            section,
            offset,
            sym_a: sym_a.to_string(),
            sym_b: sym_b.to_string(),
            extra_addend,
            size,
        });
        self.base.emit_placeholder(size)?;
        Ok(())
    }

    /// Check if any operand contains a deferred symbolic expression.
    fn has_deferred_expr(operands: &[Operand]) -> bool {
        operands
            .iter()
            .any(|op| matches!(op, Operand::MemExpr { .. } | Operand::Expr(_)))
    }

    fn process_instruction(
        &mut self,
        mnemonic: &str,
        operands: &[Operand],
        raw_operands: &str,
    ) -> Result<(), String> {
        self.base.ensure_text_section();
        // GAS 2.47: instructions are what raise a fresh section's
        // sh_addralign (data-only `.text` stays 1 — measured).
        self.base.note_instruction();

        // movz/movk with :abs_g*: modifiers resolve here, at write time
        // (GAS parity): known local values encode inline; preemption-
        // eligible (global/weak) and unresolvable symbols become MOVW
        // relocations the linker fills in.
        if (mnemonic == "movz" || mnemonic == "movk") && operands.len() >= 2 {
            if let Some(Operand::Modifier { kind, symbol }) = operands.get(1) {
                let spec = super::encoder::abs_g_spec(kind)?;
                if let Some(spec) = spec {
                    return self.process_movw(mnemonic, operands, kind, spec, symbol);
                }
                // Not an abs_g modifier (e.g. :lo12:): fall through to the
                // normal encoder path.
            }
        }

        // If any operand has a symbolic expression that needs deferred resolution,
        // emit a NOP placeholder and queue for later resolution.
        if Self::has_deferred_expr(operands) {
            let section = self.base.current_section.clone();
            let offset = self.base.current_offset();
            self.pending_instructions.push(PendingInstruction {
                section,
                offset,
                mnemonic: mnemonic.to_string(),
                operands: operands.to_vec(),
                raw_operands: raw_operands.to_string(),
            });
            // Emit NOP placeholder (will be patched during resolution)
            self.base.emit_u32_le(u32::from_le_bytes(AARCH64_NOP))?;
            return Ok(());
        }

        match encode_instruction(mnemonic, operands, raw_operands) {
            Ok(EncodeResult::Word(word)) => {
                self.base.emit_u32_le(word)?;
                Ok(())
            }
            Ok(EncodeResult::WordWithReloc { word, reloc }) => {
                let elf_type = reloc.reloc_type.elf_type();
                let is_local = reloc.symbol.starts_with(".L")
                    || reloc.symbol.starts_with(".l")
                    || reloc.symbol == ".";

                if is_local || is_branch_reloc_type(elf_type) {
                    let offset = self.base.current_offset();
                    self.pending_branch_relocs.push(PendingReloc {
                        section: self.base.current_section.clone(),
                        offset,
                        reloc_type: elf_type,
                        symbol: reloc.symbol.clone(),
                        addend: reloc.addend,
                    });
                    self.base.emit_u32_le(word)?;
                } else {
                    self.base.add_reloc(elf_type, reloc.symbol, reloc.addend);
                    self.base.emit_u32_le(word)?;
                }
                Ok(())
            }
            Ok(EncodeResult::Words(words)) => {
                for word in words {
                    self.base.emit_u32_le(word)?;
                }
                Ok(())
            }
            Ok(EncodeResult::Skip) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// movz/movk with an `:abs_g*:` modifier (kernel `tramp_alias` & co.).
    ///
    /// Three outcomes, matching GAS:
    /// 1. Global/weak symbols — always a MOVW relocation (the linker may
    ///    preempt them; baking in a known value would break that).
    /// 2. Local symbols whose value is known (aliases via `.set` expanded
    ///    first, then labels) — encode the halfword inline, with overflow
    ///    checks for the non-`_nc` forms.
    /// 3. Anything else (forward references, externals) — a MOVW
    ///    relocation with a zeroed immediate field.
    fn process_movw(
        &mut self,
        mnemonic: &str,
        operands: &[Operand],
        kind: &str,
        spec: super::encoder::AbsGSpec,
        symbol: &str,
    ) -> Result<(), String> {
        let (rd, is_64) = super::encoder::get_reg(operands, 0)?;
        let hw = spec.shift / 16;
        // 32-bit movz/movk have only two halfword slots (hw 0..1).
        if !is_64 && hw > 1 {
            return Err(format!(
                "{}: :{}: selects halfword {}, but W registers only have halfwords 0..1",
                mnemonic, kind, hw
            ));
        }
        let is_movz = mnemonic == "movz";

        let is_global = self.base.global_symbols.contains_key(symbol)
            || self.base.weak_symbols.contains_key(symbol);

        if !is_global {
            // `.set`/`.equ` aliases may wrap the label arithmetic; expand
            // them, then substitute labels and evaluate (backward labels
            // only — that is exactly what GAS resolves inline too).
            let effective = self.base.resolve_expr_aliases(symbol);
            if let Some(value) = self.resolve_label_expr(&effective) {
                if !spec.nc {
                    // Overflow checks (GAS "relocation overflow" parity):
                    // non-_nc forms require the value to fit the field.
                    if spec.signed {
                        let chunk = value >> spec.shift;
                        if !(-(1i64 << 15)..=(1i64 << 15) - 1).contains(&chunk) {
                            return Err(format!(
                                "{}: :{}: relocation overflow for '{}' (value {})",
                                mnemonic, kind, symbol, value
                            ));
                        }
                    } else {
                        if value < 0 {
                            return Err(format!(
                                "{}: :{}: relocation overflow for '{}' (negative value {})",
                                mnemonic, kind, symbol, value
                            ));
                        }
                        let val = value as u64;
                        if spec.shift + 16 < 64 && (val >> (spec.shift + 16)) != 0 {
                            return Err(format!(
                                "{}: :{}: relocation overflow for '{}' (value {})",
                                mnemonic, kind, symbol, value
                            ));
                        }
                    }
                }
                let imm16 = ((value as u64) >> spec.shift) as u32 & 0xFFFF;
                let word = super::encoder::movw_word(is_movz, rd, is_64, hw, imm16);
                self.base.emit_u32_le(word)?;
                return Ok(());
            }
        }

        // Relocation path: zeroed immediate field, filled at link time.
        let reloc_type = match (spec.signed, spec.nc, hw) {
            (true, _, 0) => RelocType::MovwSabsG0,
            (true, _, 1) => RelocType::MovwSabsG1,
            (true, _, _) => RelocType::MovwSabsG2,
            (false, false, 0) => RelocType::MovwUabsG0,
            (false, false, 1) => RelocType::MovwUabsG1,
            (false, false, 2) => RelocType::MovwUabsG2,
            (false, false, _) => RelocType::MovwUabsG3,
            (false, true, 0) => RelocType::MovwUabsG0Nc,
            (false, true, 1) => RelocType::MovwUabsG1Nc,
            (false, true, _) => RelocType::MovwUabsG2Nc,
        };
        let word = super::encoder::movw_word(is_movz, rd, is_64, hw, 0);
        self.base
            .add_reloc(reloc_type.elf_type(), symbol.to_string(), 0);
        self.base.emit_u32_le(word)?;
        Ok(())
    }

    /// Resolve local branch labels to PC-relative offsets using AArch64 relocation types.
    /// For symbols defined in the same section AND with local binding, the PC-relative
    /// offset is computed and patched directly into the instruction (matching GAS
    /// behavior). For global/weak symbols (preemptible — always relocated), undefined
    /// symbols, and cross-section symbols, a relocation is emitted.
    fn resolve_local_branches(&mut self) -> Result<(), String> {
        for reloc in &self.pending_branch_relocs {
            // "." means current address (branch to self)
            let (target_section, target_offset) = if reloc.symbol == "." {
                (reloc.section.clone(), reloc.offset)
            } else if let Some(label_info) = self.base.labels.get(&reloc.symbol) {
                // GAS parity: branches to global/weak symbols always get a
                // relocation, even when the symbol is defined in this same
                // section — the linker may preempt them (shared objects,
                // interposition), and baking the PC-relative offset into
                // the instruction would make that impossible. The binding
                // is only known once every directive has been processed
                // (`.globl` may follow the branch), so the check happens
                // here at resolution time rather than at deferral time.
                if self.base.global_symbols.contains_key(&reloc.symbol)
                    || self.base.weak_symbols.contains_key(&reloc.symbol)
                {
                    if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                        section.relocs.push(ObjReloc {
                            offset: reloc.offset,
                            reloc_type: reloc.reloc_type,
                            symbol_name: reloc.symbol.clone(),
                            addend: reloc.addend,
                        });
                    }
                    continue;
                }
                label_info.clone()
            } else {
                // Symbol not defined locally - emit as external relocation
                if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                    section.relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type: reloc.reloc_type,
                        symbol_name: reloc.symbol.clone(),
                        addend: reloc.addend,
                    });
                }
                continue;
            };

            if target_section != reloc.section {
                // Cross-section reference - convert to section symbol + offset
                if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                    section.relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type: reloc.reloc_type,
                        symbol_name: target_section.clone(),
                        addend: target_offset as i64 + reloc.addend,
                    });
                }
                continue;
            }

            let pc_offset = (target_offset as i64) - (reloc.offset as i64) + reloc.addend;

            // GAS parity: an out-of-range PC-relative offset is a hard error.
            //
            // Every immediate field below is patched with a plain mask/shift,
            // so without this check a branch that cannot be encoded silently
            // WRAPS and jumps to an unrelated address. That is worse than a
            // normal miscompile: the relocation is *resolved* here, so no
            // diagnostic is emitted and no external relocation is left behind
            // for the linker to catch -- the corrupted branch is invisible in
            // the object file. (Upstream fork issue #121.)
            //
            // ADRP (275) is encoded as a page difference rather than a byte
            // offset, so it gets its own operand.
            let checked = if reloc.reloc_type == 275 {
                let pc_page = (reloc.offset as i64) & !0xFFF;
                let target_page = (target_offset as i64) & !0xFFF;
                target_page - pc_page
            } else {
                pc_offset
            };
            let (lo, hi) = match reloc.reloc_type {
                // R_AARCH64_JUMP26 / R_AARCH64_CALL26: imm26 << 2
                282 | 283 => (-(1i64 << 27), (1i64 << 27)),
                // R_AARCH64_CONDBR19 / R_AARCH64_LD_PREL_LO19: imm19 << 2
                280 | 273 => (-(1i64 << 20), (1i64 << 20)),
                // R_AARCH64_TSTBR14: imm14 << 2
                279 => (-(1i64 << 15), (1i64 << 15)),
                // R_AARCH64_ADR_PREL_LO21: 21-bit signed byte offset
                274 => (-(1i64 << 20), (1i64 << 20)),
                // R_AARCH64_ADR_PREL_PG_HI21: 21-bit signed *page* offset
                275 => (-(1i64 << 32), (1i64 << 32)),
                _ => (i64::MIN, i64::MAX),
            };
            if !(lo..hi).contains(&checked) {
                return Err(format!(
                    "branch out of range: `{}` at offset {:#x} needs a PC-relative \
                     offset of {:#x}, which is not encodable in relocation type {} \
                     (allowed {:#x}..{:#x}); it would silently branch to the wrong address",
                    reloc.symbol, reloc.offset, checked, reloc.reloc_type, lo, hi
                ));
            }
            // AArch64 instructions are 4-byte aligned, so every PC-relative
            // branch displacement must be a multiple of 4.
            if matches!(reloc.reloc_type, 282 | 283 | 280 | 279 | 273) && (pc_offset & 3) != 0 {
                return Err(format!(
                    "misaligned branch target: `{}` needs a PC-relative offset of \
                     {:#x}, which is not a multiple of 4",
                    reloc.symbol, pc_offset
                ));
            }

            if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                let instr_offset = reloc.offset as usize;
                if instr_offset + 4 > section.data.len() {
                    continue;
                }

                let mut word = u32::from_le_bytes([
                    section.data[instr_offset],
                    section.data[instr_offset + 1],
                    section.data[instr_offset + 2],
                    section.data[instr_offset + 3],
                ]);

                match reloc.reloc_type {
                    282 | 283 => {
                        // R_AARCH64_JUMP26 / R_AARCH64_CALL26
                        let imm26 = ((pc_offset >> 2) as u32) & 0x3FFFFFF;
                        word |= imm26;
                    }
                    280 => {
                        // R_AARCH64_CONDBR19
                        let imm19 = ((pc_offset >> 2) as u32) & 0x7FFFF;
                        word |= imm19 << 5;
                    }
                    279 => {
                        // R_AARCH64_TSTBR14
                        let imm14 = ((pc_offset >> 2) as u32) & 0x3FFF;
                        word |= imm14 << 5;
                    }
                    273 => {
                        // R_AARCH64_LD_PREL_LO19 - LDR literal
                        let imm19 = ((pc_offset >> 2) as u32) & 0x7FFFF;
                        word |= imm19 << 5;
                    }
                    274 => {
                        // R_AARCH64_ADR_PREL_LO21 - ADR instruction
                        let imm = pc_offset as i32;
                        let immlo = (imm as u32) & 0x3;
                        let immhi = ((imm as u32) >> 2) & 0x7FFFF;
                        word |= (immlo << 29) | (immhi << 5);
                    }
                    275 => {
                        // R_AARCH64_ADR_PREL_PG_HI21 - ADRP instruction (local resolution)
                        let pc_page = (reloc.offset as i64) & !0xFFF;
                        let target_page = (target_offset as i64) & !0xFFF;
                        let page_off = target_page - pc_page;
                        let imm = (page_off >> 12) as i32;
                        let immlo = (imm as u32) & 0x3;
                        let immhi = ((imm as u32) >> 2) & 0x7FFFF;
                        word |= (immlo << 29) | (immhi << 5);
                    }
                    _ => {
                        // Unknown reloc type for local branch - leave as external
                        section.relocs.push(ObjReloc {
                            offset: reloc.offset,
                            reloc_type: reloc.reloc_type,
                            symbol_name: reloc.symbol.clone(),
                            addend: reloc.addend,
                        });
                        continue;
                    }
                }

                section.data[instr_offset..instr_offset + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
        Ok(())
    }

    /// Write the final ELF object file.
    pub fn write_elf(&mut self, output_path: &str) -> Result<(), String> {
        let config = elf::ElfConfig {
            e_machine: EM_AARCH64,
            e_flags: 0,
            elf_class: ELFCLASS64,
            force_rela: false,
        };
        self.base.write_elf(output_path, &config, false)
    }
}

// ── Tests: .org directive + branch binding triage (GAS parity) ───────────
#[cfg(test)]
mod org_and_branch_tests {
    use super::super::parser::parse_asm;
    use super::*;

    fn assemble_to_writer(asm: &str) -> Result<ElfWriter, String> {
        let statements = parse_asm(asm)?;
        let mut writer = ElfWriter::new();
        writer.process_statements(&statements)?;
        Ok(writer)
    }

    fn text_of(writer: &ElfWriter) -> &Vec<u8> {
        &writer.base.sections[".text"].data
    }

    fn text_relocs(writer: &ElfWriter) -> &Vec<ObjReloc> {
        &writer.base.sections[".text"].relocs
    }

    #[test]
    fn org_pads_vector_table_entries() {
        // The Linux kernel vectors-table shape: every entry must be exactly
        // 128 bytes so that hardware-indexed dispatch lands on entry starts.
        let writer = assemble_to_writer(
            ".text\n\
             .globl vectors\n\
             vectors:\n\
             .Lventry_start:\n\
             nop\n\
             .org .Lventry_start + 128\n\
             .Lventry_2:\n\
             nop\n\
             .org .Lventry_2 + 128\n\
             .org . + 64, 0xcc\n",
        )
        .expect("assembles");
        let data = text_of(&writer);
        // two 128-byte entries + a 64-byte 0xcc tail = 320
        assert_eq!(data.len(), 320, "entries padded to 128 bytes each");
        assert_eq!(data[0], 0x1f_u8, "first nop at 0");
        assert_eq!(data[128], 0x1f_u8, "second nop at 128");
        assert!(data[256..320].iter().all(|&b| b == 0xcc), "fill byte tail");
    }

    #[test]
    fn org_accepts_absolute_and_arithmetic_exprs() {
        let writer =
            assemble_to_writer(".text\nnop\n.org 0x20\nnop\n.org . + 0x10\n").expect("assembles");
        // nop(4) -> .org 0x20 -> nop(36) -> .org . + 0x10 = 52.
        assert_eq!(text_of(&writer).len(), 0x34);
    }

    #[test]
    fn org_rejects_forward_reference_and_backward_move() {
        let fwd = assemble_to_writer(".text\nnop\n.org later + 8\nlater:\nret\n");
        assert!(fwd.is_err(), "forward reference must be rejected");
        let back = assemble_to_writer(".text\nnop\n.org 2\n");
        assert!(back.is_err(), "backwards move must be rejected");
    }

    // Regression for the arm global-branch-reloc fix (2026-08-28 round):
    // branches to GLOBAL/WEAK symbols must keep their relocations even when
    // the target lives in the same section (the linker may preempt them);
    // only genuinely local symbols may be resolved in place.
    #[test]
    fn global_branches_relocate_same_section_locals_bake() {
        let writer = assemble_to_writer(
            ".text\n\
             .globl global_target\n\
             global_target:\n\
             ret\n\
             bl global_target\n\
             bl .Llocal_fn\n\
             .Llocal_fn:\n\
             ret\n\
             .weak weak_target\n\
             weak_target:\n\
             ret\n\
             b weak_target\n",
        )
        .expect("assembles");

        let relocs = text_relocs(&writer);
        let global = relocs
            .iter()
            .find(|r| r.symbol_name == "global_target")
            .expect("CALL26 reloc for global target");
        assert_eq!(global.reloc_type, 283, "R_AARCH64_CALL26");
        assert_eq!(global.offset, 4, "bl sits at offset 4");
        assert_eq!(global.addend, 0);

        let weak = relocs
            .iter()
            .find(|r| r.symbol_name == "weak_target")
            .expect("JUMP26 reloc for weak target");
        assert_eq!(weak.reloc_type, 282, "R_AARCH64_JUMP26");
        assert_eq!(weak.offset, 20, "b sits at offset 20");

        // The .Llocal_fn branch is resolved in place: 0x94000001 = bl +1.
        let data = text_of(&writer);
        let local_bl = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        assert_eq!(local_bl, 0x9400_0001, "local branch baked in place");
        assert!(
            !relocs.iter().any(|r| r.symbol_name == ".Llocal_fn"),
            "local branch must not produce a relocation"
        );
    }

    #[test]
    fn caspal_assembles_through_pipeline() {
        // End-to-end guard for the CASP pair encoder (task
        // fix_arm_asm_caspal_instruction): the word must match the
        // Capstone-round-tripped constant from the encoder unit tests.
        let writer = assemble_to_writer("caspal x0, x1, x2, x3, [x11]\n").expect("assembles");
        let data = text_of(&writer);
        let word = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        assert_eq!(word, 0x4860_FD62, "caspal x0, x1, x2, x3, [x11]");
    }
}

// ── Tests: movz/movk :abs_g*: MOVW resolution (GAS parity) ───────────────
#[cfg(test)]
mod movw_tests {
    use super::super::parser::parse_asm;
    use super::*;

    fn assemble_to_writer(asm: &str) -> Result<ElfWriter, String> {
        let statements = parse_asm(asm)?;
        let mut writer = ElfWriter::new();
        writer.process_statements(&statements)?;
        Ok(writer)
    }

    fn text_of(writer: &ElfWriter) -> &Vec<u8> {
        &writer.base.sections[".text"].data
    }

    fn word_at(data: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
    }

    /// Regression for the arm MOVW symbolic-relocation fix (2026-08-28 round):
    /// kernel `tramp_alias` macro shape. A local `.set` alias of a label
    /// difference plus constant must resolve INLINE with the documented
    /// chunk values (no relocation, no "expected immediate" error).
    #[test]
    fn tramp_alias_shape_resolves_inline() {
        let writer = assemble_to_writer(
            ".text\n\
             tramp_sym:\n\
             ret\n\
             tramp_start:\n\
             ret\n\
             .set .Lalias1, -10526720 + tramp_sym - tramp_start\n\
             movz x0, :abs_g2_s:.Lalias1\n\
             movk x0, :abs_g1_nc:.Lalias1\n\
             movk x0, :abs_g0_nc:.Lalias1\n",
        )
        .expect("assembles");
        let data = text_of(&writer);
        let v = (-10526724i64) as u64; // 0 - 4 - 10526720
        // Base words: sf=1, opc=10/11, fixed "100101" at bits 28-23
        // (i.e. 0b1010_0101 << 23), then hw at 22-21 and imm16 at 20-5.
        let want = [
            0xD2800000u32 | (2 << 21) | (((v >> 32) & 0xFFFF) as u32) << 5,
            0xF2800000u32 | (1 << 21) | (((v >> 16) & 0xFFFF) as u32) << 5,
            0xF2800000u32 | ((v & 0xFFFF) as u32) << 5,
        ];
        assert_eq!(word_at(data, 8), want[0], "movz :abs_g2_s:");
        assert_eq!(word_at(data, 12), want[1], "movk :abs_g1_nc:");
        assert_eq!(word_at(data, 16), want[2], "movk :abs_g0_nc:");
        assert!(writer.base.sections[".text"].relocs.is_empty());
    }

    /// External symbols keep MOVW relocations with ABI type numbers.
    #[test]
    fn external_symbols_get_movw_relocs() {
        let writer = assemble_to_writer(
            "movz x1, :abs_g0_nc:external_sym\n\
             movk x1, :abs_g1_nc:external_sym\n",
        )
        .expect("assembles");
        let relocs = &writer.base.sections[".text"].relocs;
        assert_eq!(relocs.len(), 2, "two MOVW relocations");
        assert_eq!(relocs[0].reloc_type, 264, "R_AARCH64_MOVW_UABS_G0_NC");
        assert_eq!(relocs[1].reloc_type, 266, "R_AARCH64_MOVW_UABS_G1_NC");
        assert_eq!(relocs[0].symbol_name, "external_sym");
        // immediate fields are zero until the linker applies them
        let data = text_of(&writer);
        assert_eq!(word_at(data, 0) >> 5 & 0xFFFF, 0);
        assert_eq!(word_at(data, 4) >> 5 & 0xFFFF, 0);
    }

    /// Non-_nc forms overflow-check; movn rejects modifiers outright.
    #[test]
    fn movw_overflow_and_movn_rejections() {
        // :abs_g0: with a value wider than 16 bits must be rejected.
        let overflow =
            assemble_to_writer(".text\nval_set:\nret\n.set v, 70000\nmovz x2, :abs_g0:v\n");
        assert!(overflow.is_err(), "abs_g0 overflow must be rejected");
        // A value that fits must pass.
        let fits = assemble_to_writer(".text\n.set v, 70000\nmovz x2, :abs_g1:v\n").expect("fits");
        let w = word_at(text_of(&fits), 0);
        assert_eq!((w >> 21) & 3, 1, "hw=1 for abs_g1");
        assert_eq!((w >> 5) & 0xFFFF, 70000 >> 16, "chunk bits");
        // movn has no MOVW forms.
        let movn = assemble_to_writer(".text\nmovn x3, :abs_g0:v\n");
        assert!(movn.is_err(), "movn must reject :abs_g*: modifiers");
    }
}

// =============================================================================
// Branch-displacement range validation (upstream fork issue #121)
// =============================================================================
#[cfg(test)]
mod branch_range_tests {
    use super::super::parser::parse_asm;
    use super::*;

    fn assemble(asm: &str) -> Result<ElfWriter, String> {
        let statements = parse_asm(asm)?;
        let mut writer = ElfWriter::new();
        writer.process_statements(&statements)?;
        Ok(writer)
    }

    /// A conditional branch (`b.eq`) encodes a 19-bit displacement: +-1 MiB.
    /// Pushing the target 2 MiB away used to wrap silently into the immediate
    /// field and branch to an unrelated address, with no diagnostic and no
    /// external relocation left behind to reveal it.
    #[test]
    fn out_of_range_cond_branch_is_diagnosed_not_wrapped() {
        let err = match assemble(
            ".text\n\
             b.eq .Lfar\n\
             .org 0x200000\n\
             .Lfar:\n\
             nop\n",
        ) {
            Ok(_) => panic!("an unencodable +-1 MiB displacement must be diagnosed"),
            // `ElfWriter` is not `Debug`, so `expect_err` is unavailable here.
            Err(e) => e,
        };
        assert!(
            err.contains("branch out of range"),
            "expected a range diagnostic, got: {err}"
        );
    }

    /// `tbz` is tighter still: a 14-bit displacement, +-32 KiB.
    #[test]
    fn out_of_range_tbz_is_diagnosed_not_wrapped() {
        let err = match assemble(
            ".text\n\
             tbz x0, #3, .Lfar\n\
             .org 0x10000\n\
             .Lfar:\n\
             nop\n",
        ) {
            Ok(_) => panic!("an unencodable +-32 KiB displacement must be diagnosed"),
            Err(e) => e,
        };
        assert!(
            err.contains("branch out of range"),
            "expected a range diagnostic, got: {err}"
        );
    }

    /// The check must not fire on ordinary code: short forward and backward
    /// branches are the common case and must keep resolving inline.
    #[test]
    fn in_range_branches_still_resolve_inline() {
        let w = assemble(
            ".text\n\
             b.eq .Lfwd\n\
             nop\n\
             .Lfwd:\n\
             nop\n\
             tbz x0, #0, .Lfwd2\n\
             nop\n\
             .Lfwd2:\n\
             nop\n",
        )
        .expect("short branches must assemble");
        assert!(
            w.base.sections[".text"].relocs.is_empty(),
            "a resolved local branch leaves no relocation behind"
        );
    }

    /// A backward branch resolves to a negative displacement; the range check
    /// must be signed, not a magnitude test.
    #[test]
    fn backward_branch_resolves_with_negative_displacement() {
        let w = assemble(
            ".text\n\
             .Ltop:\n\
             nop\n\
             nop\n\
             b.eq .Ltop\n",
        )
        .expect("a short backward branch must assemble");
        assert!(w.base.sections[".text"].relocs.is_empty());
    }
}

// =============================================================================
// AArch64 encoder regression tests
// =============================================================================
//
// Every expected word in this module was produced by GNU Binutils 2.44
// (`aarch64-linux-gnu-as`) and pasted here, rather than derived from the
// encoder under test. A unit test that re-derives its expectations from the
// code it guards shares every wrong assumption that code has, and passes
// forever -- which is exactly how the four defects below survived.
//
// The pinned values are therefore "what GAS does", and the assertions are
// "lccc must match it", including the cases where GAS *rejects* the input.
#[cfg(test)]
mod nobits_laws_tests {
    use super::super::parser::parse_asm;
    use super::*;

    fn assemble(asm: &str) -> Result<ElfWriter, String> {
        let statements = parse_asm(asm)?;
        let mut writer = ElfWriter::new();
        writer.process_statements(&statements)?;
        Ok(writer)
    }

    /// GAS 2.47 NOBITS verdict law, aarch64 rows — same table as the x86
    /// `gas247_nobits_store_and_position_laws`: numeric stores use the RAW
    /// value before truncation, `.fill` uses the fill wording on the raw
    /// value, position directives with any fill byte are legal.  ARM and
    /// riscv share `ElfWriterBase`'s guards; these rows pin the aarch64
    /// wiring (Byte/Short inline guards, Ascii/Asciz string guards, Fill).
    #[test]
    fn gas247_nobits_store_and_position_laws() {
        for (src, needle) in [
            (".bss\n.byte 1\n", "non-zero value"),
            (".bss\n.short 256\n", "non-zero value"),
            (".bss\n.long 4294967296\n", "non-zero value"),
            (".bss\n.fill 4,1,7\n", "fill section"),
            (".bss\n.fill 4,1,256\n", "fill section"),
            (".bss\n.ascii \"ab\"\n", "non-empty string"),
            (".bss\n.asciz \"a\"\n", "non-empty string"),
        ] {
            // `ElfWriter` is not `Debug`, so `unwrap_err` is unavailable; match.
            let err = match assemble(src) {
                Ok(_) => panic!("{src:?}: expected {needle:?} error"),
                Err(e) => e,
            };
            assert!(
                err.contains(needle),
                "{src:?}: expected {needle:?} in error, got: {err}"
            );
        }
        for ok in [
            ".bss\n.byte 0\n.short 0\n.long 0\n.ascii \"\"\n.fill 4,1,0\n",
            ".bss\n.org 8,0xff\n.zero 4\n.space 4,1\n.p2align 4,0xff\n.byte 0\n",
            ".data\n.byte 1\n.short 256\n.fill 4,1,7\n",
        ] {
            assemble(ok).unwrap_or_else(|e| panic!("{ok:?} must assemble, got: {e}"));
        }
    }

    /// P1 bare-input + instruction-raise law (GAS 2.47 measured on
    /// aarch64): content before any section directive lands in `.text`;
    /// sections are CREATED at sh_addralign 1 (`.text;.byte 1` -> 1,
    /// `.section .foo,"ax";.byte 1` -> 1); the FIRST instruction raises
    /// the containing section to the instruction alignment (`.text;nop`
    /// -> 4, `.byte 1;nop` -> 4); an align directive raises as usual
    /// (bare `.p2align 4` -> `.text` algn 16, size 0).
    #[test]
    fn gas247_bare_input_default_section_and_align_law() {
        let w = assemble(".byte 1\n").unwrap();
        assert_eq!(w.base.sections[".text"].data, vec![1]);
        assert_eq!(w.base.sections[".text"].sh_addralign, 1);

        let w = assemble(".zero 4\n").unwrap();
        assert_eq!(w.base.sections[".text"].data, vec![0; 4]);
        assert_eq!(w.base.sections[".text"].sh_addralign, 1);

        let w = assemble(".p2align 4\n").unwrap();
        assert_eq!(w.base.sections[".text"].sh_addralign, 16);

        let w = assemble(".text\n.byte 1\n").unwrap();
        assert_eq!(w.base.sections[".text"].sh_addralign, 1);

        let w = assemble(".data\n.byte 1\n").unwrap();
        assert_eq!(w.base.sections[".data"].sh_addralign, 1);

        let w = assemble(".text\nnop\n").unwrap();
        assert_eq!(w.base.sections[".text"].sh_addralign, 4);

        let w = assemble(".byte 1\nnop\n").unwrap();
        assert_eq!(w.base.sections[".text"].sh_addralign, 4);

        let w = assemble(".text\n.p2align 4\n").unwrap();
        assert_eq!(w.base.sections[".text"].sh_addralign, 16);

        let w = assemble(".section .foo,\"ax\"\n.byte 1\n").unwrap();
        assert_eq!(w.base.sections[".foo"].sh_addralign, 1);
    }

    /// The assembly-wide directive-fill budget on the `ElfWriterBase`
    /// implementation: two maximum-size directives exactly fill
    /// `MAX_TOTAL_DIRECTIVE_FILL` (accepted), the next byte is refused
    /// BEFORE its allocation.  Materializes ~512 MiB of zeros — bounded
    /// by the standing 4 GiB swap policy.
    #[test]
    fn fill_budget_bounds_resident_fill() {
        let err = match assemble(".data\n.zero 268435456\n.zero 268435456\n.zero 1\n") {
            Ok(_) => panic!("budget must refuse the third directive"),
            Err(e) => e,
        };
        assert!(err.contains("budget"), "got: {err}");
    }

    /// Exactly at the budget: both maximum directives are accepted
    /// (charge == MAX_TOTAL_DIRECTIVE_FILL edge).  Separate from the
    /// refusal row so the successful object is built and dropped alone.
    #[test]
    fn fill_budget_exact_boundary_accepted() {
        assemble(".data\n.zero 268435456\n.zero 268435456\n")
            .unwrap_or_else(|e| panic!("exact-budget fill must assemble, got: {e}"));
    }
}

#[cfg(test)]
mod aarch64_encoder_tests {
    use super::super::parser::parse_asm;
    use super::*;

    fn assemble(asm: &str) -> Result<ElfWriter, String> {
        let statements = parse_asm(asm)?;
        let mut writer = ElfWriter::new();
        writer.process_statements(&statements)?;
        Ok(writer)
    }

    /// Assemble one instruction and return its single little-endian word.
    fn word_of(asm: &str) -> u32 {
        let w = assemble(asm).unwrap_or_else(|e| panic!("`{asm}` must assemble: {e}"));
        let data = &w.base.sections[".text"].data;
        assert_eq!(data.len(), 4, "`{asm}` must encode to exactly one word");
        u32::from_le_bytes([data[0], data[1], data[2], data[3]])
    }

    fn one_insn(insn: &str) -> String {
        format!(".text\n{insn}\n")
    }

    /// Defect 2: `str b9,[x10]` encoded with the *double* size field, so the
    /// B and D forms were indistinguishable. GAS: 3d000149 vs fd000149.
    #[test]
    fn fp_ldr_str_size_field_follows_the_register_width() {
        assert_eq!(
            word_of(&one_insn("str b9,[x10]")),
            0x3d00_0149,
            "`str b9,[x10]`"
        );
        assert_eq!(
            word_of(&one_insn("str d9,[x10]")),
            0xfd00_0149,
            "`str d9,[x10]`"
        );
        assert_eq!(
            word_of(&one_insn("str s9,[x10]")),
            0xbd00_0149,
            "`str s9,[x10]`"
        );
        assert_eq!(
            word_of(&one_insn("str q9,[x10]")),
            0x3d80_0149,
            "`str q9,[x10]`"
        );

        // The four forms must be mutually distinguishable -- a shared size
        // field is precisely the bug this guards.
        let forms = ["b9", "s9", "d9", "q9"].map(|r| word_of(&one_insn(&format!("str {r},[x10]"))));
        for i in 0..forms.len() {
            for j in (i + 1)..forms.len() {
                assert_ne!(
                    forms[i], forms[j],
                    "`str` size field collapsed two register widths together"
                );
            }
        }
    }

    /// Defect 1: FP `ldnp`/`stnp` hardcoded `V=0`, so `ldnp s24,s13,[x20,#80]`
    /// assembled as the GPR (`w`) form. GAS: 2c4a3698 vs 284a3698.
    #[test]
    fn fp_ldnp_stnp_sets_the_v_bit() {
        let fp = word_of(&one_insn("ldnp s24,s13,[x20,#80]"));
        let gpr = word_of(&one_insn("ldnp w24,w13,[x20,#80]"));
        assert_eq!(fp, 0x2c4a_3698, "`ldnp s24,s13,[x20,#80]`");
        assert_eq!(gpr, 0x284a_3698, "`ldnp w24,w13,[x20,#80]`");
        assert_ne!(fp, gpr, "the FP pair must not reuse the GPR encoding");
        assert_eq!(fp >> 26 & 1, 1, "V must be set for FP pairs");
        assert_eq!(gpr >> 26 & 1, 0, "V must be clear for GPR pairs");

        assert_eq!(word_of(&one_insn("stnp d2,d3,[x4,#-8]")), 0x6c3f_8c82);
    }

    /// Defect 4: half-precision scalar FP was encoded as single (`ftype` 00
    /// instead of 11) because the test was a two-way `starts_with('d')`.
    #[test]
    fn half_precision_scalar_fp_uses_ftype_11() {
        assert_eq!(word_of(&one_insn("fmadd h11,h3,h2,h12")), 0x1fc2_306b);
        assert_eq!(word_of(&one_insn("fmadd s11,s3,s2,s12")), 0x1f02_306b);
        assert_eq!(word_of(&one_insn("fmadd d4,d5,d6,d7")), 0x1f46_1ca4);
        assert_eq!(word_of(&one_insn("fadd h1,h2,h3")), 0x1ee3_2841);
        assert_eq!(word_of(&one_insn("fcvt s2,h3")), 0x1ee2_4062);
        assert_eq!(word_of(&one_insn("scvtf h1,w2")), 0x1ee2_0041);

        // The whole point: h and s must not share an encoding.
        assert_ne!(
            word_of(&one_insn("fmadd h11,h3,h2,h12")),
            word_of(&one_insn("fmadd s11,s3,s2,s12"))
        );
    }

    /// `fmov` between FP registers is a same-width move: it carries one `type`
    /// field, so mixed widths are invalid. The old code preferred whichever
    /// operand it happened to look at first and assembled all of them.
    #[test]
    fn fmov_rejects_mismatched_fp_widths() {
        for insn in [
            "fmov s0,d1",
            "fmov d0,s1",
            "fmov h0,s1",
            "fmov s0,h1",
            "fmov h0,d1",
        ] {
            let err = match assemble(&one_insn(insn)) {
                Ok(_) => panic!("`{insn}` is not a valid fmov (GAS rejects it) but assembled"),
                Err(e) => e,
            };
            assert!(
                err.contains("different floating-point widths"),
                "`{insn}`: expected a width diagnostic, got: {err}"
            );
        }
    }

    /// FMOV (general) covers H as well as S and D, and `sf` follows the GP
    /// register: H pairs with either width, but S only with W and D only
    /// with X. `fmov h0,w1` used to emit the *single* encoding (1e270020).
    #[test]
    fn fmov_general_covers_half_precision_and_pairs_widths() {
        assert_eq!(word_of(&one_insn("fmov h0,w1")), 0x1ee7_0020);
        assert_eq!(word_of(&one_insn("fmov h0,x1")), 0x9ee7_0020);
        assert_eq!(word_of(&one_insn("fmov w0,h1")), 0x1ee6_0020);
        assert_eq!(word_of(&one_insn("fmov x0,h1")), 0x9ee6_0020);
        assert_eq!(word_of(&one_insn("fmov s0,w1")), 0x1e27_0020);
        assert_eq!(word_of(&one_insn("fmov d0,x1")), 0x9e67_0020);
        assert_eq!(word_of(&one_insn("fmov w0,s1")), 0x1e26_0020);
        assert_eq!(word_of(&one_insn("fmov x0,d1")), 0x9e66_0020);
        assert_eq!(word_of(&one_insn("fmov h0,wzr")), 0x1ee7_03e0);
        assert_eq!(word_of(&one_insn("fmov h0,xzr")), 0x9ee7_03e0);

        // S only pairs with W; D only pairs with X.
        for insn in ["fmov s0,x1", "fmov d0,w1"] {
            let err = match assemble(&one_insn(insn)) {
                Ok(_) => panic!("`{insn}` is not a valid fmov (GAS rejects it) but assembled"),
                Err(e) => e,
            };
            assert!(
                err.contains("general-purpose register"),
                "`{insn}`: expected a GP-width diagnostic, got: {err}"
            );
        }
        // Q and B have no FMOV (general) form at all.
        for insn in ["fmov q0,x1", "fmov x0,q1", "fmov b0,w1"] {
            assert!(
                assemble(&one_insn(insn)).is_err(),
                "`{insn}` must be rejected"
            );
        }
    }

    /// Immediate offsets were masked into their fields (`& 0x1FF`, `& 0x7F`),
    /// so an out-of-range offset wrapped into a plausible-looking instruction
    /// instead of being diagnosed -- `ldr x0,[x1,#32768]` became `ldur x0,[x1]`.
    #[test]
    fn out_of_range_offsets_are_diagnosed_not_masked() {
        for insn in [
            "ldr x0,[x1,#32768]",
            "ldr x0,[x1,#-257]",
            "stp x0,x1,[x2,#8192]",
            "stp x0,x1,[x2,#-520]",
            "ldnp x0,x1,[x2,#8192]",
        ] {
            let err = match assemble(&one_insn(insn)) {
                Ok(_) => panic!("`{insn}` wraps its offset silently; GAS rejects it"),
                Err(e) => e,
            };
            assert!(
                err.contains("offset"),
                "`{insn}`: expected an offset diagnostic, got: {err}"
            );
        }
        // Misalignment is the same defect wearing a different hat: the pair
        // immediate is scaled, so a non-multiple would truncate away.
        let err = match assemble(&one_insn("stp x0,x1,[x2,#4]")) {
            Ok(_) => panic!("`stp x0,x1,[x2,#4]` is misaligned; GAS rejects it"),
            Err(e) => e,
        };
        assert!(err.contains("multiple of 8"), "got: {err}");
    }

    /// The range check must be inclusive *and* signed: the extreme legal
    /// offsets on both sides have to keep encoding.
    #[test]
    fn in_range_offsets_still_encode_at_their_boundaries() {
        assert_eq!(word_of(&one_insn("ldr x0,[x1,#255]")), 0xf84f_f020);
        assert_eq!(word_of(&one_insn("ldr x0,[x1,#-256]")), 0xf850_0020);
        assert_eq!(word_of(&one_insn("stp x0,x1,[x2,#504]")), 0xa91f_8440);
        assert_eq!(word_of(&one_insn("stp x0,x1,[x2,#-512]")), 0xa920_0440);
        assert_eq!(word_of(&one_insn("stp q0,q1,[x2,#1008]")), 0xad1f_8440);
    }

    /// The unscaled (LDUR/STUR) form *does* exist for 128-bit accesses --
    /// `size == 00` with `opc == 10` selects Q. Pinned because this is a
    /// tempting thing to "fix": 8 is not a multiple of the Q access width, so
    /// the scaled form correctly declines, but GAS still assembles the
    /// instruction as `stur q0,[x1,#8]` (3c808020).
    #[test]
    fn unscaled_form_exists_for_128bit_accesses() {
        assert_eq!(word_of(&one_insn("str q0,[x1,#8]")), 0x3c80_8020);
        assert_eq!(word_of(&one_insn("ldr q0,[x1,#8]")), 0x3cc0_8020);
        assert_eq!(word_of(&one_insn("str q0,[x1,#-256]")), 0x3c90_0020);
    }

    /// A load/store pair encodes one register class for both registers, so
    /// they must agree -- and `b`/`h` have no pair form at all.
    #[test]
    fn pair_registers_must_share_a_register_class() {
        for insn in ["stp s0,x1,[x0]", "ldp s0,x1,[x0]", "stp w0,x1,[x0]"] {
            let err = match assemble(&one_insn(insn)) {
                Ok(_) => panic!("`{insn}` mixes register classes; GAS rejects it"),
                Err(e) => e,
            };
            assert!(
                err.contains("different register classes"),
                "`{insn}`: expected a class diagnostic, got: {err}"
            );
        }
        for insn in ["stp b0,b1,[x0]", "stp h0,h1,[x0]", "ldnp b0,b1,[x0]"] {
            let err = match assemble(&one_insn(insn)) {
                Ok(_) => panic!("`{insn}` has no pair form; GAS rejects it"),
                Err(e) => e,
            };
            assert!(
                err.contains("unsupported register"),
                "`{insn}`: expected an unsupported-register diagnostic, got: {err}"
            );
        }
    }

    /// ADD/SUB, CLZ/REV/RBIT and SXTB/SXTH/UXTH/UXTB have no FP or SIMD
    /// encoding at all, but `parse_reg_num` resolves `h2` to `2`, so
    /// `add x0,x1,h2` assembled as `add x0,x1,x2`. GNU as rejects all three.
    #[test]
    fn gp_only_arith_and_bitfield_encoders_reject_fp_registers() {
        for insn in [
            "add x0,x1,h2",
            "add x0,x1,v2",
            "add x0,x1,s2",
            "sub x0,x1,s2",
            "clz x0,d1",
            "rev x0,d1",
            "rev16 x0,d1",
            "rev32 x0,d1",
            "rbit x0,d1",
            "sxtb x0,s1",
            "sxth x0,s1",
            "uxtb x0,s1",
            "uxth x0,s1",
        ] {
            assert!(
                assemble(&one_insn(insn)).is_err(),
                "`{insn}` names an FP/SIMD register in a GP-only encoding; \\
                 GNU as rejects it"
            );
        }
        // `add d0,d1,d2` is the legal scalar-FP 3-same form and stays legal;
        // only the GP dispatch path rejects FP operands.
        assert!(assemble(&one_insn("add d0,d1,d2")).is_ok());
        // The GP spellings still assemble, and SP is still legal in add/sub.
        assert_eq!(word_of(&one_insn("add x0,x1,x2")), 0x8b02_0020);
        assert_eq!(word_of(&one_insn("add sp,sp,#16")), 0x9100_43ff);
        assert_eq!(word_of(&one_insn("clz x0,x1")), 0xdac0_1020);
        assert_eq!(word_of(&one_insn("rev x0,x1")), 0xdac0_0c20);
    }

    /// The extend aliases are `SXTB <Xd>, <Wn>`: the *source* is always the
    /// 32-bit form. GNU as assembles `sxtb x0,w1` and rejects `sxtb x0,x1`.
    #[test]
    fn extend_aliases_require_a_32bit_source_register() {
        assert_eq!(word_of(&one_insn("sxtb x0,w1")), 0x9340_1c20);
        assert_eq!(word_of(&one_insn("sxtb w0,w1")), 0x1300_1c20);
        assert_eq!(word_of(&one_insn("sxth x0,w1")), 0x9340_3c20);
        assert_eq!(word_of(&one_insn("sxtw x0,w1")), 0x9340_7c20);
        for insn in [
            "sxtb x0,x1",
            "sxth x0,x1",
            "uxtb x0,x1",
            "uxth x0,x1",
            "sxtb w0,x1",
        ] {
            assert!(
                assemble(&one_insn(insn)).is_err(),
                "`{insn}` uses a 64-bit source with an extend alias; \\
                 GNU as rejects it"
            );
        }
    }

    /// The permissive operand readers resolve *any* register spelling to a
    /// bare 0-31 number, so an FP/SIMD register reached encoders that only
    /// have a GP encoding and was silently assembled as one. `mov x0, d1`,
    /// `mov d0, x1`, `mov d0, lr` and `and x0, x1, d2` were all accepted and
    /// produced words GNU as rejects outright.
    #[test]
    fn gp_only_encoders_reject_fp_and_simd_registers() {
        for insn in [
            "mov d0,x1",
            "mov x0,d1",
            "mov d0,lr",
            "mov d0,sp",
            "and x0,x1,d2",
            "orr x0,s1,x2",
            "and x0,x1,h2",
            "and x0,x1,v2",
        ] {
            assert!(
                assemble(&one_insn(insn)).is_err(),
                "`{insn}` mixes an FP/SIMD register into a GP-only encoding; \\
                 GNU as rejects it"
            );
        }
    }

    /// SP is legal as the destination of the non-flags-setting *immediate*
    /// logical forms and nowhere else. GNU as assembles `and sp,x1,#15` and
    /// rejects `and x0,sp,#15`, `and sp,x1,x2` and `ands sp,x1,#15`.
    #[test]
    fn logical_immediate_sp_legality_matches_gas() {
        assert_eq!(word_of(&one_insn("and sp,x1,#15")), 0x9240_0c3f);
        assert_eq!(word_of(&one_insn("orr sp,x1,#1")), 0xb240_003f);
        for insn in [
            "and x0,sp,#15",
            "and sp,x1,x2",
            "and x0,sp,x2",
            "ands sp,x1,#15",
        ] {
            assert!(
                assemble(&one_insn(insn)).is_err(),
                "`{insn}` uses SP where the architecture does not allow it; \\
                 GNU as rejects it"
            );
        }
        // The plain forms are untouched.
        assert_eq!(word_of(&one_insn("and x0,x1,x2")), 0x8a02_0020);
        assert_eq!(word_of(&one_insn("and x0,x1,#15")), 0x9240_0c20);
    }

    /// `mov` to or from SP lowers to ADD, and both the 64-bit `sp` and the
    /// 32-bit `wsp` spelling take that form. `wsp` was missed, so
    /// `mov w0,wsp` assembled as an ORR (0x2a1f03e0) instead of an ADD
    /// (0x110003e0).
    #[test]
    fn mov_to_or_from_sp_lowers_to_add_for_both_spellings() {
        assert_eq!(word_of(&one_insn("mov sp,x1")), 0x9100_003f);
        assert_eq!(word_of(&one_insn("mov x0,sp")), 0x9100_03e0);
        assert_eq!(word_of(&one_insn("mov sp,sp")), 0x9100_03ff);
        assert_eq!(word_of(&one_insn("mov w0,wsp")), 0x1100_03e0);
        assert_eq!(word_of(&one_insn("mov wsp,w1")), 0x1100_003f);
        // A `mov` to/from SP is an ADD, never an ORR: the two differ in bit 30.
        for insn in ["mov sp,x1", "mov x0,sp", "mov w0,wsp", "mov wsp,w1"] {
            assert_eq!(
                word_of(&one_insn(insn)) & (1 << 30),
                0,
                "`{insn}` must lower to ADD, which has bit 30 clear"
            );
        }
    }

    /// `lr` is an alias for `x30`, not a register of its own. GNU as assembles
    /// `fmov d0,lr` to exactly the word it assembles `fmov d0,x30` to
    /// (0x9e6703c0); lccc rejected it, because the width helper that decides
    /// whether the GP operand is 32- or 64-bit did not list `lr` even though
    /// the crate's own register parser did. The two must agree about what a
    /// register spelling means.
    ///
    /// `fp` (x29), `ip0` (x16) and `ip1` (x17) are the same class of omission
    /// but are deliberately NOT fixed here: they are unrecognised at the
    /// parser level, and GAS resolves them context-sensitively -- `ldr x0, fp`
    /// loads the *symbol* `fp` when one is defined, while `mov x1, fp` uses
    /// the register. Matching that needs parser work with its own analysis,
    /// not a one-line addition to a width table.
    #[test]
    fn lr_alias_is_a_64bit_general_purpose_register() {
        // Same words as the x30 spellings, from GNU as.
        assert_eq!(word_of(&one_insn("fmov d0,lr")), 0x9e67_03c0);
        assert_eq!(word_of(&one_insn("fmov d0,x30")), 0x9e67_03c0);
        assert_eq!(word_of(&one_insn("fmov d30,lr")), 0x9e67_03de);
        assert_eq!(word_of(&one_insn("fmov lr,d0")), 0x9e66_001e);
        assert_eq!(word_of(&one_insn("fmov x30,d0")), 0x9e66_001e);
        // H pairs with either GP width, and `lr` is 64-bit, so sf=1.
        assert_eq!(word_of(&one_insn("fmov h0,lr")), 0x9ee7_03c0);

        // The alias and the numbered spelling are the same register.
        assert_eq!(
            word_of(&one_insn("fmov d0,lr")),
            word_of(&one_insn("fmov d0,x30")),
            "`lr` must encode identically to `x30`"
        );

        // S only pairs with a 32-bit GP register, so `lr` is rejected -- the
        // same reason GAS gives, rather than "unknown register".
        let err = match assemble(&one_insn("fmov s0,lr")) {
            Ok(_) => panic!("`fmov s0,lr` must be rejected: GAS rejects it"),
            Err(e) => e,
        };
        assert!(
            err.contains("64-bit"),
            "`fmov s0,lr` should fail on width, not on spelling: {err}"
        );
    }

    /// The `#fbits` operand of the fixed-point conversions was parsed and then
    /// thrown away, so `fcvtzs x10,s30,#55` assembled as `fcvtzs x10,s30` --
    /// wrong by a factor of 2^55. Two fields are involved: bit 21 flips from 1
    /// to 0 (integer vs fixed-point form) and bits 15..10 carry
    /// `scale = 64 - fbits`.
    #[test]
    fn fixed_point_conversions_encode_fbits() {
        // Fixed-point form: bit 21 clear, scale = 64 - fbits.
        assert_eq!(word_of(&one_insn("fcvtzs x10,s30,#55")), 0x9e18_27ca);
        assert_eq!(word_of(&one_insn("fcvtzu x10,d4,#16")), 0x9e59_c08a);
        assert_eq!(word_of(&one_insn("fcvtzs w4,s17,#13")), 0x1e18_ce24);
        assert_eq!(word_of(&one_insn("fcvtzu w4,h17,#13")), 0x1ed9_ce24);
        assert_eq!(word_of(&one_insn("scvtf h23,x8,#48")), 0x9ec2_4117);
        assert_eq!(word_of(&one_insn("scvtf s1,w2,#1")), 0x1e02_fc41);
        assert_eq!(word_of(&one_insn("ucvtf s0,w1,#31")), 0x1e03_8420);
        assert_eq!(word_of(&one_insn("scvtf d1,x2,#64")), 0x9e42_0041);
        assert_eq!(word_of(&one_insn("scvtf h1,w2,#16")), 0x1ec2_c041);
        assert_eq!(word_of(&one_insn("ucvtf d0,x1,#40")), 0x9e43_6020);

        // Integer form: bit 21 set, scale 0. These must be unchanged by the
        // fix -- and must stay distinct from their fixed-point twins.
        assert_eq!(word_of(&one_insn("fcvtzs x0,s1")), 0x9e38_0020);
        assert_eq!(word_of(&one_insn("fcvtzs x0,s1,#64")), 0x9e18_0020);
        assert_ne!(
            word_of(&one_insn("fcvtzs x0,s1")),
            word_of(&one_insn("fcvtzs x0,s1,#64")),
            "fbits=64 must still differ from the integer form (scale 0, bit 21 clear)"
        );
        assert_eq!(word_of(&one_insn("scvtf s1,w2")), 0x1e22_0041);
        assert_eq!(word_of(&one_insn("ucvtf s0,w1")), 0x1e23_0020);
        assert_eq!(word_of(&one_insn("fcvtzu w0,d1")), 0x1e79_0020);

        // fbits is bounded by the width of the integer operand.
        for insn in [
            "scvtf s1,w2,#64",
            "fcvtzs w0,s1,#33",
            "fcvtzs x0,s1,#0",
            "scvtf s1,w2,#0",
        ] {
            assert!(
                assemble(&one_insn(insn)).is_err(),
                "`{insn}` is out of range; GAS rejects it"
            );
        }
    }

    /// The *vector* fixed-point conversions are a different encoding from the
    /// integer ones, not a variant of them: the integer form is
    /// `0 Q U 0 1 1 1 0 size 1 10000 opcode 10 Rn Rd`, while the fixed-point
    /// form is `0 Q U 0 1 1 1 1 0 immh:immb 111111 Rn Rd` for FCVTZS/U and
    /// `... 111001 ...` for SCVTF/UCVTF. lccc routed every vector conversion to
    /// the integer form, so `fcvtzs v20.2d,v12.2d,#13` assembled as
    /// `fcvtzs v20.2d,v12.2d` -- wrong by a factor of 2^13.
    ///
    /// `immh:immb` is `2 * esize - fbits`, which is why the constants below
    /// look like they count down as `fbits` counts up.
    #[test]
    fn vector_fixed_point_conversions_encode_immh_immb() {
        assert_eq!(word_of(&one_insn("fcvtzs v20.2d,v12.2d,#13")), 0x4f73_fd94);
        assert_eq!(word_of(&one_insn("scvtf v20.2d,v24.2d,#57")), 0x4f47_e714);
        assert_eq!(word_of(&one_insn("ucvtf v26.2d,v17.2d,#30")), 0x6f62_e63a);
        assert_eq!(word_of(&one_insn("fcvtzs v3.2d,v11.2d,#10")), 0x4f76_fd63);
        assert_eq!(word_of(&one_insn("fcvtzs v18.2d,v5.2d,#62")), 0x4f42_fcb2);
        assert_eq!(word_of(&one_insn("scvtf v22.4s,v11.4s,#20")), 0x4f2c_e576);

        // Boundaries of immh:immb for each element width, and the Q bit.
        assert_eq!(word_of(&one_insn("fcvtzs v1.8h,v2.8h,#16")), 0x4f10_fc41);
        assert_eq!(word_of(&one_insn("fcvtzs v1.8h,v2.8h,#1")), 0x4f1f_fc41);
        assert_eq!(word_of(&one_insn("fcvtzs v1.4h,v2.4h,#5")), 0x0f1b_fc41);
        assert_eq!(word_of(&one_insn("fcvtzs v1.4s,v2.4s,#32")), 0x4f20_fc41);
        assert_eq!(word_of(&one_insn("fcvtzs v1.4s,v2.4s,#1")), 0x4f3f_fc41);
        assert_eq!(word_of(&one_insn("fcvtzs v1.2s,v2.2s,#1")), 0x0f3f_fc41);
        assert_eq!(word_of(&one_insn("fcvtzs v1.2d,v2.2d,#1")), 0x4f7f_fc41);

        // U bit and the integer-to-FP opcode.
        assert_eq!(word_of(&one_insn("fcvtzu v1.8h,v2.8h,#7")), 0x6f19_fc41);
        assert_eq!(word_of(&one_insn("scvtf v1.8h,v2.8h,#4")), 0x4f1c_e441);
        assert_eq!(word_of(&one_insn("ucvtf v1.4s,v2.4s,#12")), 0x6f34_e441);
        assert_eq!(word_of(&one_insn("scvtf v1.2d,v2.2d,#1")), 0x4f7f_e441);
        assert_eq!(word_of(&one_insn("ucvtf v1.2s,v2.2s,#20")), 0x2f2c_e441);

        // The integer form must be untouched by the fixed-point path.
        assert_eq!(word_of(&one_insn("fcvtzs v20.2d,v12.2d")), 0x4ee1_b994);
        assert_ne!(
            word_of(&one_insn("fcvtzs v20.2d,v12.2d")),
            word_of(&one_insn("fcvtzs v20.2d,v12.2d,#13")),
            "the fixed-point form must not collapse onto the integer form"
        );

        // fbits is bounded by the element width.
        for insn in [
            "fcvtzs v1.8h,v2.8h,#17",
            "fcvtzs v1.4s,v2.4s,#33",
            "fcvtzs v1.2d,v2.2d,#65",
            "scvtf v1.4s,v2.4s,#0",
        ] {
            assert!(
                assemble(&one_insn(insn)).is_err(),
                "`{insn}` is out of range; GAS rejects it"
            );
        }
    }

    /// Only FCVTZS and FCVTZU round toward zero, and they are the only members
    /// of the family with a fixed-point form. The other eight rounding modes
    /// must reject `#fbits` rather than encode a rounding the hardware lacks.
    #[test]
    fn only_the_z_rounding_modes_take_fbits() {
        for insn in [
            "fcvtas w1,s2,#8",
            "fcvtau x1,d2,#8",
            "fcvtns x0,s1,#8",
            "fcvtnu x3,d4,#17",
            "fcvtms w1,s2,#8",
            "fcvtmu x1,d2,#8",
            "fcvtps w1,s2,#8",
            "fcvtpu x1,d2,#8",
        ] {
            assert!(
                assemble(&one_insn(insn)).is_err(),
                "`{insn}` has no fixed-point form; GAS rejects it"
            );
        }
        // ...and they keep working without it.
        assert_eq!(word_of(&one_insn("fcvtas w1,s2")), 0x1e24_0041);
        assert_eq!(word_of(&one_insn("fcvtns x0,s1")), 0x9e20_0020);
        assert_eq!(word_of(&one_insn("fcvtzs x0,s1,#8")), 0x9e18_e020);
        assert_eq!(word_of(&one_insn("fcvtzu x0,s1,#8")), 0x9e19_e020);
    }

    /// Advanced-SIMD by-element forms. Two independent errors lived here:
    /// `mla`/`mls` omitted bit 29 (the U field, which is 1 for them and 0 for
    /// `mul`/`sqdmulh`/`sqrdmulh`), and the FP by-element forms omitted bit 23
    /// entirely while `fmul` wrongly set bit 29. `mla v0.4s,v1.4s,v2.s[1]`
    /// assembled as 0x4fa20020 -- a MUL, which discards the accumulator -- so
    /// the program ran and computed the wrong thing.
    #[test]
    fn neon_by_element_encodes_u_and_prefix_bits() {
        // Integer by element: U = 1 for MLA/MLS, 0 for the rest.
        assert_eq!(word_of(&one_insn("mul v0.4s,v1.4s,v2.s[1]")), 0x4fa2_8020);
        assert_eq!(word_of(&one_insn("mla v0.4s,v1.4s,v2.s[1]")), 0x6fa2_0020);
        assert_eq!(word_of(&one_insn("mls v0.4s,v1.4s,v2.s[1]")), 0x6fa2_4020);
        assert_eq!(
            word_of(&one_insn("sqdmulh v0.4s,v1.4s,v2.s[1]")),
            0x4fa2_c020
        );
        assert_eq!(
            word_of(&one_insn("sqrdmulh v0.4s,v1.4s,v2.s[1]")),
            0x4fa2_d020
        );
        assert_eq!(word_of(&one_insn("mla v0.8h,v1.8h,v2.h[3]")), 0x6f72_0020);
        assert_eq!(word_of(&one_insn("mls v0.2s,v1.2s,v2.s[3]")), 0x2fa2_4820);

        // FP by element: bits 28..23 are the fixed prefix 011111, bit 29 is 0.
        assert_eq!(word_of(&one_insn("fmul v0.4s,v1.4s,v2.s[1]")), 0x4fa2_9020);
        assert_eq!(word_of(&one_insn("fmla v0.4s,v1.4s,v2.s[1]")), 0x4fa2_1020);
        assert_eq!(word_of(&one_insn("fmls v0.4s,v1.4s,v2.s[1]")), 0x4fa2_5020);
        assert_eq!(word_of(&one_insn("fmul v0.2d,v1.2d,v2.d[0]")), 0x4fc2_9020);
        assert_eq!(word_of(&one_insn("fmla v0.2d,v1.2d,v2.d[0]")), 0x4fc2_1020);
        assert_eq!(word_of(&one_insn("fmla v0.2s,v1.2s,v2.s[3]")), 0x0fa2_1820);
        assert_eq!(word_of(&one_insn("fmla v0.4s,v1.4s,v2.s[0]")), 0x4f82_1020);

        // An accumulator-based and a non-accumulator form must differ in the
        // U bit, and `fmul` must not carry the integer group's U=1 habit.
        let mla = word_of(&one_insn("mla v0.4s,v1.4s,v2.s[1]"));
        let mul = word_of(&one_insn("mul v0.4s,v1.4s,v2.s[1]"));
        assert_ne!(
            mla & (1 << 29),
            mul & (1 << 29),
            "MLA must set U where MUL clears it"
        );
        assert_eq!(
            word_of(&one_insn("fmul v0.4s,v1.4s,v2.s[1]")) & (1 << 29),
            0,
            "the FP by-element group has no U field"
        );
    }

    /// A symbolic-operand instruction that cannot be encoded used to leave the
    /// NOP placeholder in the output and print a warning, so an unsupported
    /// SVE store assembled to a bare `nop` and exited 0.
    #[test]
    fn unencodable_deferred_instruction_fails_closed() {
        let err = match assemble(".text\nstr z15,[x9,#45,mul vl]\n") {
            Ok(w) => panic!(
                "an unencodable deferred instruction must fail, but produced \
                 {bytes:02x?} (a NOP placeholder is 1f2003d5)",
                bytes = &w.base.sections[".text"].data
            ),
            Err(e) => e,
        };
        assert!(
            err.contains("failed to resolve deferred instruction"),
            "expected the deferred-resolution diagnostic, got: {err}"
        );
    }

    /// ...but the fail-closed path must not break the instructions that
    /// legitimately resolve through it: a literal-pool load is deferred too,
    /// and has to come out as a real LDR, not as the placeholder.
    #[test]
    fn supported_deferred_instruction_still_resolves() {
        let w =
            assemble(".text\n.Ltab:\nldr x0, .Ltab\n").expect("a literal-pool ldr must assemble");
        let data = &w.base.sections[".text"].data;
        assert!(!data.is_empty(), "the LDR must be emitted");
        assert_ne!(
            data,
            &AARCH64_NOP.to_vec(),
            "the deferred instruction left its NOP placeholder unresolved"
        );
        // An LDR literal is 0x?8 / 0x?c in its high byte (opc<1:0> == 00),
        // little-endian, so that byte is the *last* of the four.
        let opcode_byte = data[data.len() - 1];
        assert_eq!(
            opcode_byte & 0x3f,
            0x18,
            "expected an LDR-literal opcode, got {data:02x?}"
        );
    }
    /// The operand-legality matrix, asserted against *this crate's* encoder.
    ///
    /// `tests/aarch64/operand-legality.tsv` is generated from GNU as by
    /// `scripts/aarch64_operand_legality_matrix.py`, and `ci_local.sh` re-checks
    /// it against the cross assembler at gate time.  This test is the same table
    /// with no external tools, so the guarantee also holds on a dev box that has
    /// no aarch64 binutils -- and, unlike an `.is_err()` assertion, it pins the
    /// accepted encodings as well as the rejections.
    ///
    /// It exists because a register-*class* check cannot answer the question an
    /// encoder has to answer.  `clz x0, sp`, `mov sp, xzr`, `add x0, w1, w2`,
    /// `fmov h0, wsp` and `lsl x0, x1, #64` are all made of general-purpose
    /// registers, and every one of them used to assemble: the first four into a
    /// different instruction than the one written, the last into the corrupt
    /// word 0xfffffc20 from an unchecked `width - 1 - amount` underflow.
    #[test]
    fn operand_legality_matrix_matches_the_encoder() {
        let table = include_str!("../../../../tests/aarch64/operand-legality.tsv");
        let mut checked = 0usize;
        let mut accepted = 0usize;
        let mut rejected = 0usize;
        let mut per_group: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        let mut drift: Vec<String> = Vec::new();

        for line in table.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let mut fields = line.split('\t');
            let (Some(group), Some(insn), Some(expect)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if group == "group" {
                continue; // column header
            }
            checked += 1;
            *per_group.entry(group).or_insert(0) += 1;
            let result = assemble(&one_insn(insn));
            let encoded = result.as_ref().ok().map(|w| {
                w.base.sections[".text"]
                    .data
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            });
            match expect.strip_prefix("OK ") {
                Some(hex) => {
                    accepted += 1;
                    match encoded {
                        Some(got) if got == hex => {}
                        Some(got) => {
                            drift.push(format!("[{group}] {insn}: gas=OK {hex} encoder=OK {got}"))
                        }
                        None => {
                            // `ElfWriter` is not `Debug`, so take the message
                            // from the `Err` arm rather than `unwrap_err`.
                            let why = match &result {
                                Err(e) => e.as_str(),
                                Ok(_) => "encoder produced no section data",
                            };
                            drift.push(format!(
                                "[{group}] {insn}: gas=OK {hex} encoder=REJECT ({why})"
                            ));
                        }
                    }
                }
                None => {
                    rejected += 1;
                    if let Some(got) = encoded {
                        drift.push(format!("[{group}] {insn}: gas=REJECT encoder=OK {got}"));
                    }
                }
            }
        }

        // Ratchets.  The previous form (`checked >= 420`, `accepted >= 250`,
        // `rejected >= 170`) was written when the matrix had 423 rows and was
        // never raised as the table grew: at 10411 rows the same `checked >=
        // 420` permitted 96% of the coverage to disappear while staying green,
        // and it could not notice a *family* disappearing at all as long as
        // the survivors were numerous.  The counts below are exact, per group, and generated by
        // `scripts/aarch64_operand_legality_matrix.py --ratchets`; a group
        // that is renamed or dropped fails on the missing entry, and a group
        // that shrinks by one row fails on its count.  Regenerating the table
        // (a deliberate act: `--regenerate`) therefore also means updating
        // this list, which is the point.
        #[rustfmt::skip]
        const GROUP_RATCHETS: &[(&str, usize)] = &[
            ("", 10411),  // total rows
            ("add/sub explicit extend", 16),
            ("add/sub extended form reg31", 17),
            ("add/sub extended form widths", 9),
            ("add/sub immediate shift", 13),
            ("bic forms", 12),
            ("branch operands", 23),
            ("cmp/cmn/tst aliases", 40),
            ("dp bit test", 11),
            ("dp wide immediates", 23),
            ("extend aliases", 31),
            ("fmov gp", 32),
            ("gp only arithmetic", 8),
            ("gp only logical siblings", 19),
            ("immediate width overflow", 10),
            ("ldr/str byte forms", 61),
            ("ldst Rt class", 37),
            ("ldst exclusives", 39),
            ("ldst imm9 range", 20),
            ("ldst prfm", 18),
            ("mov immediate", 15),
            ("mov immediate wide forms", 21),
            ("movw halfword selector", 14),
            ("neon addv maxv minv", 22),
            ("neon by-element", 21),
            ("neon ext", 13),
            ("neon ld1r ld2r ld4r", 21),
            ("neon movi imm8", 17),
            ("neon movi shift", 24),
            ("neon mvni", 18),
            ("neon tbl tbx", 13),
            ("positive controls", 7),
            ("reg31 dp1src", 23),
            ("reg31 logical immediate", 17),
            ("reg31 logical register", 10),
            ("reg31 mov", 23),
            ("shift legality add/sub", 22),
            ("shift legality logical", 19),
            ("sweep atomic arity", 114),
            ("sweep load/store family", 464),
            ("sweep neon by-element", 172),
            ("sweep neon elem-gp", 183),
            ("sweep neon lane", 377),
            ("sweep neon ld lane", 136),
            ("sweep neon movi", 157),
            ("sweep neon shift", 807),
            ("sweep register class", 3773),
            ("sweep system sys", 40),
            ("sweep system sysreg", 3261),
            ("system Rt", 27),
            ("system immediates", 24),
            ("system pstate", 74),
            ("system sys", 13),
            ("width add/sub", 16),
            ("width extended add/sub", 7),
            ("width logical", 7),
        ];

        let (_, want_total) = GROUP_RATCHETS[0];
        assert_eq!(
            checked, want_total,
            "the matrix has {checked} rows, the ratchet pins {want_total}; \
             regenerate the ratchet with \
             `scripts/aarch64_operand_legality_matrix.py --ratchets` if the \
             table was deliberately regenerated"
        );
        assert!(
            accepted + rejected == checked,
            "row accounting is inconsistent: {accepted} accepted + {rejected} \
             rejected != {checked} rows"
        );
        for (group, want) in &GROUP_RATCHETS[1..] {
            let got = per_group.get(group).copied().unwrap_or(0);
            assert_eq!(
                got, *want,
                "group `{group}` has {got} rows, the ratchet pins {want}"
            );
        }
        let pinned: std::collections::HashSet<&str> =
            GROUP_RATCHETS[1..].iter().map(|(g, _)| *g).collect();
        let mut unpinned: Vec<&str> = per_group
            .keys()
            .copied()
            .filter(|g| !pinned.contains(g))
            .collect();
        unpinned.sort_unstable();
        assert!(
            unpinned.is_empty(),
            "the table has group(s) with no ratchet entry: {unpinned:?}"
        );
        assert!(accepted >= 265, "only {accepted} accepted rows remain");
        assert!(rejected >= 190, "only {rejected} rejected rows remain");
        assert!(
            drift.is_empty(),
            "{} matrix row(s) disagree with this encoder:\n{}",
            drift.len(),
            drift
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// Families that #762 left on the fail-open path, pinned to GNU as 2.47
    /// words (little-endian display as the architectural word).
    #[test]
    fn fail_closed_families_match_gas_pins() {
        fn must_reject(insn: &str) {
            assert!(
                assemble(&one_insn(insn)).is_err(),
                "`{insn}` must be rejected"
            );
        }

        // LD2R/LD4R used to emit unallocated words (R/S bits swapped).
        assert_eq!(word_of(&one_insn("ld2r {v0.16b,v1.16b},[x0]")), 0x4d60_c000);
        assert_eq!(
            word_of(&one_insn("ld4r {v0.16b,v1.16b,v2.16b,v3.16b},[x0]")),
            0x4d60_e000
        );
        assert_eq!(word_of(&one_insn("ld1r {v0.16b},[x0]")), 0x4d40_c000);
        assert_eq!(
            word_of(&one_insn("ld3r {v0.16b,v1.16b,v2.16b},[x0]")),
            0x4d40_e000
        );

        // MOVI/MVNI shift matrix: msl was dropped (parser) and .4h lsl #8
        // always encoded cmode=1000.
        assert_eq!(word_of(&one_insn("movi v0.4s,#1,msl #8")), 0x4f00_c420);
        assert_eq!(word_of(&one_insn("movi v0.4h,#1,lsl #8")), 0x0f00_a420);
        assert_eq!(word_of(&one_insn("movi v0.16b,#0")), 0x4f00_e400);
        assert_eq!(word_of(&one_insn("movi v0.2d,#0")), 0x6f00_e400);
        assert_eq!(word_of(&one_insn("mvni v0.4s,#1,msl #8")), 0x6f00_c420);
        must_reject("movi v0.16b,#256");
        must_reject("movi v0.2d,#0x0101010101010101");
        must_reject("movi v0.4s,#1,lsr #8");

        // EXT index is per-arrangement, not masked.
        assert!(assemble(&one_insn("ext v0.16b,v1.16b,v2.16b,#1")).is_ok());
        assert!(assemble(&one_insn("ext v0.16b,v1.16b,v2.16b,#15")).is_ok());
        must_reject("ext v0.16b,v1.16b,v2.16b,#16");
        must_reject("ext v0.8b,v1.8b,v2.8b,#8");
        must_reject("ext v0.4s,v1.4s,v2.4s,#1");

        // ADDV: .2d is unallocated.
        must_reject("addv d0,v0.2d");
        assert!(assemble(&one_insn("addv s0,v0.4s")).is_ok());

        // TBL table size 1..=4.
        must_reject("tbl v0.16b,{v0.16b,v1.16b,v2.16b,v3.16b,v4.16b},v5.16b");
        assert!(assemble(&one_insn("tbl v0.16b,{v0.16b},v1.16b")).is_ok());

        // PRFM register form opc bits.
        assert_eq!(word_of(&one_insn("prfm pldl1keep,[x0,x0]")), 0xf8a0_6800);

        // SP is never a transferred GPR of STR/LDR.
        must_reject("str sp,[x0]");
        must_reject("ldr sp,[x0]");
        assert!(assemble(&one_insn("str xzr,[x0]")).is_ok());
        assert!(assemble(&one_insn("str x0,[sp]")).is_ok());

        // Pair transferred registers and register-offset Rm: 31 is ZR, not SP.
        must_reject("ldp sp,x1,[x0]");
        must_reject("stp x0,sp,[x1]");
        must_reject("ldnp sp,x1,[x0]");
        must_reject("str x0,[x1,sp]");
        must_reject("prfm pldl1keep,[w0]");
        assert!(assemble(&one_insn("ldp xzr,x1,[x0]")).is_ok());
        assert!(assemble(&one_insn("str x0,[x1,xzr]")).is_ok());
        assert!(assemble(&one_insn("ldp x0,x1,[sp]")).is_ok());

        // LDRSW writes Xt.
        must_reject("ldrsw w0,[x0]");
        assert!(assemble(&one_insn("ldrsw x0,[x0]")).is_ok());

        // LDUR imm9 is signed 9-bit, not masked.
        must_reject("ldur x0,[x0,#256]");
        assert!(assemble(&one_insn("ldur x0,[x0,#255]")).is_ok());

        // Atomics drop no offsets.
        must_reject("cas x0,x1,[x2,#8]");
        must_reject("swp x0,x1,[x2,#8]");
        assert!(assemble(&one_insn("cas x0,x1,[x2]")).is_ok());

        // System immediates are range-checked, not masked.
        must_reject("svc #65536");
        must_reject("brk #65536");
        must_reject("msr daifset,#16");
        must_reject("msr spsel,#2");
        must_reject("mrs w0,nzcv");
        must_reject("mrs d0,fpcr");
        must_reject("hint #128");
        assert!(assemble(&one_insn("svc #0")).is_ok());
        assert!(assemble(&one_insn("msr daifset,#15")).is_ok());

        // MOVZ is Xd|XZR with a 16-bit immediate.
        must_reject("movz d0,#1");
        must_reject("movz w0,#0x10000");
        must_reject("movz sp,#1");
        assert_eq!(word_of(&one_insn("movz x0,#1")), 0xd280_0020);

        // GNU as encodes UXTW as MOV Wd,Wn (0x2a0103e0), not the ARM ARM
        // UBFM Xd,Xn,#0,#31 word (0xd3407c20) that llvm-mc emits. Both
        // destination spellings assemble to the same MOV; an X source is
        // rejected.  Measured against aarch64-linux-gnu-as 2.44, 2026-10-05.
        assert_eq!(word_of(&one_insn("uxtw x0,w1")), 0x2a01_03e0);
        assert_eq!(word_of(&one_insn("uxtw w0,w1")), 0x2a01_03e0);
        assert_eq!(word_of(&one_insn("mov w0,w1")), 0x2a01_03e0);
        must_reject("uxtw x0,x1");
        must_reject("uxtw w0,x1");
        assert_eq!(word_of(&one_insn("sxtw x0,w1")), 0x9340_7c20);
        must_reject("sxtw w0,w1");

        // Fail-closed: truncation, SIMD-as-GPR, illegal scale, bitmask.
        must_reject("mov w0,#0x100000000");
        must_reject("and w0,w1,#0x100000001");
        must_reject("casp d0,d1,d2,d3,[x4]");
        must_reject("ldr x0,[x1,x2,lsl #1]");
        assert!(assemble(&one_insn("ldr x0,[x1,x2,lsl #3]")).is_ok());
        assert!(
            assemble(".text\n.p2align foo\n").is_err(),
            "malformed .p2align exponent must be rejected"
        );
        must_reject("ext v0.16b,v1.8b,v2.16b,#1");
        must_reject("tbl v0.4s,{v0.16b},v1.16b");

        // .p2align N,,M skips padding that would exceed M.
        {
            // 4 bytes of code, then .p2align 4,,10: 12 bytes of padding would
            // be needed to reach 16, which exceeds 10, so the location
            // counter stays at 4. Without the cap, lccc used to emit 12
            // bytes of NOP.
            let asm = ".text\nmovz x0,#1\n.p2align 4,,10\nmovz x1,#2\n";
            let w = assemble(asm).expect("p2align program");
            let n = w.base.sections[".text"].data.len();
            assert_eq!(
                n, 8,
                "p2align 4,,10 at offset 4 must not pad (got {n} bytes)"
            );
            // GAS raises sh_addralign even when the cap suppresses padding.
            assert_eq!(
                w.base.sections[".text"].sh_addralign, 16,
                "skipped p2align still raises sh_addralign"
            );
        }
        {
            // At offset 4, .p2align 3,,8 needs 4 bytes (<= 8) so it pads.
            let asm = ".text\nmovz x0,#1\n.p2align 3,,8\nmovz x1,#2\n";
            let w = assemble(asm).expect("p2align program");
            let n = w.base.sections[".text"].data.len();
            assert_eq!(n, 12, "p2align 3,,8 at offset 4 must pad 4 bytes (got {n})");
            assert_eq!(
                w.base.sections[".text"].sh_addralign, 8,
                "skipped-or-applied cap still raises sh_addralign"
            );
        }
        {
            // GAS: max==0 is unlimited. `.byte 1; .p2align 4,,0; .byte 2`
            // at offset 1 needs 15 bytes of pad → 17 total, not a skip.
            let asm = ".data\n.byte 1\n.p2align 4,,0\n.byte 2\n";
            let w = assemble(asm).expect("p2align max=0");
            let d = &w.base.sections[".data"].data;
            assert_eq!(d.len(), 17, "p2align 4,,0 must pad (got {} bytes)", d.len());
            assert_eq!(d[0], 1);
            assert_eq!(d[16], 2);
            assert!(d[1..16].iter().all(|&b| b == 0));
            assert_eq!(w.base.sections[".data"].sh_addralign, 16);
            let asm = ".data\n.byte 1\n.p2align 4,,-1\n.byte 2\n";
            let w = assemble(asm).expect("p2align negative max is unlimited");
            assert_eq!(w.base.sections[".data"].data.len(), 17);
            let asm = ".data\n.byte 1\n.balign 16,,0\n.byte 2\n";
            let w = assemble(asm).expect("balign max=0");
            assert_eq!(w.base.sections[".data"].data.len(), 17);
            // GAS: fill is any integer, low byte stored (-1 == 0xff).
            let asm = ".data\n.byte 1\n.p2align 4,-1\n.byte 2\n";
            let w = assemble(asm).expect("p2align negative fill");
            let d = &w.base.sections[".data"].data;
            assert_eq!(d.len(), 17);
            assert!(d[1..16].iter().all(|&b| b == 0xff));
            // GAS: .balign 0 is a no-op; non-powers-of-two are errors.
            let asm = ".data\n.byte 1\n.balign 0\n.byte 2\n";
            let w = assemble(asm).expect("balign 0");
            assert_eq!(w.base.sections[".data"].data.len(), 2);
            for bad in [".data\n.balign 3\n", ".data\n.balign -8\n"] {
                assert!(
                    assemble(bad).is_err(),
                    "{bad} must be rejected like GAS (not a power of 2)"
                );
            }
        }
        {
            // Fill is honoured in .data AND .text (GAS 2.44:
            // `.byte 1; .p2align 3, 0xff; .byte 2` → 01ffffffffffffff02).
            let asm = ".data\n.byte 1\n.p2align 3, 0xff\n.byte 2\n";
            let w = assemble(asm).expect("p2align fill data");
            let d = &w.base.sections[".data"].data;
            assert_eq!(
                d.as_slice(),
                &[0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02],
                "data p2align fill"
            );
            let asm = ".text\n.byte 1\n.p2align 3, 0xff\n.byte 2\n";
            let w = assemble(asm).expect("p2align fill text");
            let d = &w.base.sections[".text"].data;
            assert_eq!(
                d.as_slice(),
                &[0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02],
                "text p2align fill"
            );
        }
    }

    /// PR768 M2: the GAS 2.47 `read.c::do_repeat` budget, as observed
    /// through the ARM driver (both pinned targets measured identical
    /// verdicts): negative counts wrap to `size_t` and report GAS's
    /// wording, astronomic counts fail in O(body) time instead of
    /// exhausting memory, an unresolvable count is an error instead of
    /// the old silent zero repetitions, and counts inside the GAS budget
    /// still hit the deterministic expansion ceilings before any loop
    /// runs.
    #[test]
    fn gas247_rept_budget_law_arm() {
        // Small counts assemble and repeat.
        let w = assemble(".data\n.rept 2\n.byte 7\n.endr\n").expect("rept 2");
        assert_eq!(w.base.sections[".data"].data, vec![7u8, 7u8]);
        // Negative count -> size_t wrap -> GAS's "excessive count".
        let err = match assemble(".data\n.rept -1\n.byte 0\n.endr\n") {
            Err(e) => e,
            Ok(_) => panic!("rept -1 must fail"),
        };
        assert!(
            err.contains("excessive count 18446744073709551615 for REPT - ignored"),
            "got: {err}"
        );
        // Astronomic count (the PR768 OOM repro) -> deterministic error.
        let err = match assemble(".data\n.rept 1099511627776\n.byte 0\n.endr\n") {
            Err(e) => e,
            Ok(_) => panic!("rept 2^40 must fail"),
        };
        assert!(
            err.contains("excessive count 1099511627776 for REPT - ignored"),
            "got: {err}"
        );
        // Unresolvable count: fail-closed (was `unwrap_or(0)` — accept).
        let err = match assemble(".data\n.rept garbage\n.byte 0\n.endr\n") {
            Err(e) => e,
            Ok(_) => panic!("rept garbage must fail"),
        };
        assert!(
            err.contains("bad count") || err.contains("cannot evaluate"),
            "got: {err}"
        );
        // Growth line ceiling for a count that passes the GAS budget.
        let err = match assemble(".data\n.rept 5000000\n.byte 0\n.endr\n") {
            Err(e) => e,
            Ok(_) => panic!("rept line-cap must fail"),
        };
        assert!(err.contains("line limit"), "got: {err}");
    }

    /// Every verdict here is pinned against GNU as 2.47 (red-team probe
    /// log): the signed-shift bucket (`p2align 63`, clamps `64`/`-1`),
    /// cap-skip BEFORE the bucket error, never-raise-in-bucket, and the
    /// deterministic `MAX_DIRECTIVE_FILL` ceiling for `.zero`/`.fill`/
    /// `.org` (beyond 256 MiB GAS's own answer is free-disk state).
    #[test]
    fn gas247_alignment_fill_org_matrix() {
        // ---- signed-shift bucket (align >= 1<<63), clamp family ----
        for exp in ["63", "64", "-1"] {
            // @0: accepted, nothing padded, alignment stays 1 (GAS).
            let w = assemble(&format!(".data\n.p2align {exp}\n.byte 3\n"))
                .unwrap_or_else(|e| panic!("p2align {exp} @0 must assemble: {e}"));
            assert_eq!(
                w.base.sections[".data"].sh_addralign, 1,
                "p2align {exp} @0 must not raise sh_addralign"
            );
            assert_eq!(
                w.base.sections[".data"].data,
                vec![3u8],
                "p2align {exp} @0 must not pad"
            );
            // @1, no cap: error (GAS "jump over nop padding out of range").
            assert!(
                assemble(&format!(".data\n.byte 1\n.p2align {exp}\n")).is_err(),
                "p2align {exp} @1 without cap must fail"
            );
            // @1, positive cap: SKIP — GAS assembles it with align 1.
            let w = assemble(&format!(".data\n.byte 1\n.p2align {exp},,5\n.byte 2\n"))
                .unwrap_or_else(|e| panic!("p2align {exp},,5 @1 must assemble (GAS skips): {e}"));
            assert_eq!(
                w.base.sections[".data"].data.len(),
                2,
                "p2align {exp},,5 @1 must skip the pad"
            );
            assert_eq!(
                w.base.sections[".data"].sh_addralign, 1,
                "the bucket never raises sh_addralign, even when skipped"
            );
        }

        // ---- section-kind axis (PR768 H1): the bucket verdict is
        // section-kind independent on GAS 2.47 — `.text` and custom
        // `ax`/`w` sections accept a positive-cap skip exactly like
        // `.data`, and reject the uncapped pad exactly like `.data`.
        // (The review's ".text rejects caps 0/1/5/100" was measured on
        // binutils 2.40, not the pinned oracle.)
        for sec in [".text", ".section .cax,\"ax\"", ".section .cw,\"w\""] {
            let sname = if let Some(rest) = sec.strip_prefix(".section ") {
                rest.split(',').next().unwrap().trim()
            } else {
                sec
            };
            let w = assemble(&format!("{sec}\n.byte 1\n.p2align 63,,5\n.byte 2\n"))
                .unwrap_or_else(|e| panic!("{sec} capped skip must assemble (GAS 2.47): {e}"));
            assert_eq!(
                w.base.sections[sname].data.len(),
                2,
                "{sec} p2align 63,,5 @1 must skip the pad"
            );
            assert_eq!(
                w.base.sections[sname].sh_addralign, 1,
                "{sec} bucket never raises"
            );
            assert!(
                assemble(&format!("{sec}\n.byte 1\n.p2align 63\n")).is_err(),
                "{sec} uncapped p2align 63 @1 must fail (deterministic stand-in)"
            );
            let w = assemble(&format!("{sec}\n.p2align 63,,5\n.byte 3\n"))
                .unwrap_or_else(|e| panic!("{sec} @0 no-op must assemble: {e}"));
            assert_eq!(w.base.sections[sname].data, vec![3u8], "{sec} @0 no pad");
            assert_eq!(w.base.sections[sname].sh_addralign, 1, "{sec} @0 no raise");
        }

        // ---- representable alignments: raise-on-skip, cap-skip, huge raise ----
        let w = assemble(".data\n.byte 1\n.p2align 40,,5\n.byte 2\n")
            .expect("p2align 40,,5 must skip at offset 1");
        assert_eq!(
            w.base.sections[".data"].data.len(),
            2,
            "cap 5 skips 2^40-1 pad"
        );
        assert_eq!(
            w.base.sections[".data"].sh_addralign,
            1u64 << 40,
            "GAS raises sh_addralign even when the cap skips the pad"
        );
        let w = assemble(".data\n.p2align 62\n.byte 3\n").expect("p2align 62 @0");
        assert_eq!(w.base.sections[".data"].sh_addralign, 1u64 << 62);
        assert_eq!(w.base.sections[".data"].data, vec![3u8], "@0 needs no pad");

        // ---- .balign byte-count form ----
        let w = assemble(".data\n.balign 1099511627776\n.byte 3\n")
            .expect("balign 2^40 @0 must assemble");
        assert_eq!(w.base.sections[".data"].sh_addralign, 1u64 << 40);
        assert!(
            assemble(".data\n.byte 1\n.balign 1099511627776\n").is_err(),
            "balign 2^40 @1 must fail (padding beyond the fill ceiling)"
        );
        for src in [".data\n.balign\n.byte 3\n", ".data\n.balign 0\n.byte 3\n"] {
            assemble(src).unwrap_or_else(|e| panic!("{src:?} must assemble (GAS no-ops): {e}"));
        }
        assert!(
            assemble(".data\n.balign 3\n").is_err(),
            "balign 3 is not a power of 2"
        );
        assert!(
            assemble(".data\n.balign -16\n").is_err(),
            "balign -16 is negative"
        );

        // ---- .zero / .space ----
        let w = assemble(".data\n.zero -1\n").expect(".zero -1 is a GAS no-op");
        assert_eq!(
            w.base.sections[".data"].data.len(),
            0,
            ".zero -1 emits nothing"
        );
        let w = assemble(".data\n.zero 64\n").expect(".zero 64");
        assert_eq!(w.base.sections[".data"].data.len(), 64);
        assert!(
            assemble(".data\n.zero 268435457\n").is_err(),
            "256 MiB + 1 must be refused"
        );
        let w = assemble(".data\n.space -5\n").expect(".space -5 is a no-op");
        assert_eq!(w.base.sections[".data"].data.len(), 0);

        // ---- .fill ----
        let w = assemble(".data\n.fill -1\n").expect(".fill -1 is a GAS no-op");
        assert_eq!(w.base.sections[".data"].data.len(), 0);
        let w = assemble(".data\n.fill 4611686018427387904,4\n")
            .expect("overflowing .fill is accepted and emits zero bytes");
        assert_eq!(
            w.base.sections[".data"].data.len(),
            0,
            ".fill 2^62,4 emits nothing"
        );
        let w = assemble(".data\n.fill 1,9\n").expect(".fill 1,9");
        assert_eq!(
            w.base.sections[".data"].data.len(),
            8,
            ".fill size clamps to 8 per copy"
        );
        // Out-of-i64-range literals: GAS reads them as their two's-complement
        // pattern (all 2^63..2^64-1 are negative counts -> zero bytes, and
        // 2^64 itself is an expression error).  Measured on 2.47.
        let w = assemble(".data\n.fill 18446744073709551615,1,5\n")
            .expect(".fill u64max,1,5 must assemble");
        assert_eq!(
            w.base.sections[".data"].data.len(),
            0,
            "u64max repeat is -1"
        );
        let w =
            assemble(".data\n.zero 18446744073709551615\n").expect(".zero u64max must assemble");
        assert_eq!(w.base.sections[".data"].data.len(), 0);
        let w = assemble(".data\n.zero 9223372036854775808\n").expect(".zero 2^63 must assemble");
        assert_eq!(w.base.sections[".data"].data.len(), 0);
        assert!(
            assemble(".data\n.fill 18446744073709551616,1,0\n").is_err(),
            "a literal above u64::MAX is a GAS expression error"
        );
        assert!(
            assemble(".data\n.fill 268435457,1,1\n").is_err(),
            "256 MiB + 1 must be refused"
        );

        // ---- .org (same constant-source amplifier) ----
        let w = assemble(".data\n.org 4\n").expect(".org 4");
        assert_eq!(w.base.sections[".data"].data.len(), 4);
        assert!(
            assemble(".data\n.org 268435457\n").is_err(),
            "org past the ceiling must fail"
        );
        assert!(
            assemble(".data\n.byte 1\n.org 0\n").is_err(),
            "backwards .org must fail"
        );
    }
}
