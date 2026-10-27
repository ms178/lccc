//! ELF object file writer for RISC-V.
//!
//! Takes parsed assembly statements and produces an ELF .o (relocatable) file
//! with proper sections, symbols, and relocations for RISC-V ELF (32 or 64-bit).
//!
//! Uses `ElfWriterBase` from `elf.rs` for shared section/symbol/relocation
//! management, directive processing, and ELF serialization. This file only
//! contains RISC-V-specific logic: instruction encoding dispatch, pcrel_hi/lo
//! pairing, RV64C compression, GNU numeric labels, and branch resolution.

// ELF writer helpers; some section/relocation utilities defined for completeness.
#![allow(dead_code)]

use super::cfi;
use super::compress;
use super::encoder::{
    EncodeResult, RelocType, encode_got_pseudo, encode_insn_directive, encode_instruction,
    encode_li, li_rvc_aliases_match,
};
use super::parser::{
    AsmStatement, DataValue, Directive, Operand, SizeExpr, SymbolType, Visibility,
};
use crate::backend::cfi_lower;
use crate::backend::elf::{
    self, DataFixup, ELFCLASS32, ELFCLASS64, EM_RISCV, ElfWriterBase, ObjReloc, SHF_ALLOC,
    SHF_EXECINSTR, SHF_WRITE, SHT_NOBITS, SHT_PROGBITS, STT_FUNC, STT_GNU_IFUNC, STT_NOTYPE,
    STT_OBJECT, STT_TLS, STV_HIDDEN, STV_INTERNAL, STV_PROTECTED, SetValue,
};
use crate::backend::elf_writer_common::{uleb128_fixed, uleb128_len};
use crate::common::fx_hash::FxHashMap;

// ELF flags for RISC-V
pub(super) const EF_RISCV_RVC: u32 = 0x1;
const EF_RISCV_FLOAT_ABI_SOFT: u32 = 0x0;
pub(super) const EF_RISCV_FLOAT_ABI_SINGLE: u32 = 0x2;
pub(super) const EF_RISCV_FLOAT_ABI_DOUBLE: u32 = 0x4;
pub(super) const EF_RISCV_FLOAT_ABI_QUAD: u32 = 0x6;

/// RISC-V NOP instruction: `addi x0, x0, 0` = 0x00000013 in little-endian
const RISCV_NOP: [u8; 4] = [0x13, 0x00, 0x00, 0x00];

/// `c.nop` — the 2-byte RV32C/RV64C `addi` with both rs1 and rd zero. GAS
/// `2.47` uses it inside alignment padding on an RVC march (`nop`, `c.nop`,
/// then zeros, each at its own alignment), so filling the odd bytes with
/// plain zeros instead is a byte-level divergence from the oracle, not only a
/// cosmetic one.
const RISCV_C_NOP: [u8; 2] = [0x01, 0x00];

/// The ELF writer for RISC-V.
///
/// Composes with `ElfWriterBase` for shared infrastructure and adds
/// RISC-V-specific pcrel_hi/lo pairing, RV64C compression, GNU numeric
/// label resolution, and RISC-V branch/call relocation patching.
pub struct ElfWriter {
    /// Shared ELF writer state (sections, symbols, labels, directives)
    pub base: ElfWriterBase,
    /// Pending relocations that reference local labels (resolved after all labels are known)
    pending_branch_relocs: Vec<PendingReloc>,
    /// Counter for generating synthetic pcrel_hi labels
    pcrel_hi_counter: u32,
    /// GNU numeric labels: e.g., "1" -> [(section, offset), ...] in definition order.
    /// Deferred data expressions that reference forward labels (resolved after all statements).
    /// ELF e_flags to use (default: RVC + double-float ABI)
    elf_flags: u32,
    /// ELF class: ELFCLASS64 (default) or ELFCLASS32 for RV32 targets.
    elf_class: u8,
    /// When true, don't emit R_RISCV_RELAX relocations (set by `.option norelax`).
    no_relax: bool,
    /// `.option pic` state: `la` loads the address from the GOT (as `lga`).
    pic: bool,
    /// `.option rvc` / `norvc` (default on: the objects declare
    /// `EF_RISCV_RVC`, so a relaxing linker may shrink code in 2-byte steps).
    /// GNU as's `insn_alignment` for `R_RISCV_ALIGN` runs (2 with RVC, else 4).
    rvc: bool,
    /// Stack for `.option push`/`.option pop`: saved `(no_relax, pic, rvc)`.
    option_stack: Vec<(bool, bool, bool)>,
    /// `.uleb128` values resolved after layout, in source order.
    pending_lebs: Vec<PendingLeb>,
    /// Bytes reserved for the n-th pending `.uleb128` (1 when absent).
    leb_widths: Vec<usize>,
    /// Set when a pending `.uleb128` outgrew its reservation: the widths to
    /// assemble again with (see [`Self::process_statements`]).
    leb_retry: Option<Vec<usize>>,
    /// CFI location advances and PC-relative words resolved after layout.
    pending_cfi: Vec<PendingCfi>,
    /// Branch relaxation (GNU as's `RELAX_BRANCH` frags): every branch and
    /// jump to a symbol counts here, in source order.
    branch_count: usize,
    /// The ones assembled this pass, checked after layout.
    relax_branches: Vec<RelaxBranch>,
    /// Lengths a previous pass found necessary, by ordinal.  They only
    /// grow, so the passes terminate.
    branch_lengths: FxHashMap<usize, u8>,
    /// Set when this pass lengthened a branch (assemble again).
    branch_retry: bool,
}

const R_RISCV_RVC_BRANCH: u32 = 44;
const R_RISCV_RVC_JUMP: u32 = 45;

/// A branch or jump to a symbol (GNU as's `RELAX_BRANCH` frag), checked
/// after layout and assembled again longer if its target needs it.
struct RelaxBranch {
    ordinal: usize,
    section: String,
    offset: u64,
    symbol: String,
    addend: i64,
    /// `j`/`jal`: never longer than 4 bytes (GNU as assumes jumps reach).
    jump: bool,
    /// The spelling has a 2-byte form (`c.j`, `c.jal`, `c.beqz`, `c.bnez`).
    rvc: bool,
    /// The length assembled this pass: 2, 4, 6 or 8.
    length: u8,
}

/// A `.eh_frame` field produced by the CFI lowering, written after layout.
enum PendingCfi {
    /// `DW_CFA_advance_loc*` by `value` (= `to - from`), `width` bytes.
    Advance {
        section: String,
        offset: u64,
        width: usize,
        value: SetValue,
    },
    /// `R_RISCV_32_PCREL` to `value`.
    Pcrel32 {
        section: String,
        offset: u64,
        value: SetValue,
    },
}

/// A `.uleb128` whose value was not known where it stands.
struct PendingLeb {
    section: String,
    offset: u64,
    width: usize,
    value: SetValue,
}

struct PendingReloc {
    section: String,
    offset: u64,
    reloc_type: u32,
    symbol: String,
    addend: i64,
    /// For pcrel_lo12 relocations resolved locally: the offset of the
    /// corresponding auipc (pcrel_hi) instruction.
    pcrel_hi_offset: Option<u64>,
    /// Pair an R_RISCV_RELAX when the relocation is emitted (decided at the
    /// instruction, where `.option norelax` state is known).
    relax: bool,
}

/// Relocations GAS 2.47 pairs with R_RISCV_RELAX when they come from a
/// single explicit instruction (relaxation enabled): PCREL_HI20/LO12_I/LO12_S,
/// HI20/LO12_I/LO12_S and the four TPREL forms. Explicit GOT/TLS-GOT/TLS-GD
/// hi20 are not paired (measured); CALL_PLT is handled separately.
fn relax_paired(elf_type: u32) -> bool {
    matches!(elf_type, 23..=32)
}

/// Check if a label name is a GNU numeric label (e.g., "1", "42").
fn is_numeric_label(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_digit())
}

/// Check if a symbol reference is a GNU numeric label reference (e.g., "1b", "1f", "42b").
/// Returns Some((label_name, is_backward)) if it is, None otherwise.
fn parse_numeric_label_ref(symbol: &str) -> Option<(&str, bool)> {
    if symbol.len() < 2 {
        return None;
    }
    let last_char = symbol.as_bytes()[symbol.len() - 1];
    let is_backward = last_char == b'b' || last_char == b'B';
    let is_forward = last_char == b'f' || last_char == b'F';
    if !is_backward && !is_forward {
        return None;
    }
    let label_part = &symbol[..symbol.len() - 1];
    if label_part.is_empty() || !label_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((label_part, is_backward))
}

/// Pre-process assembly statements to resolve GNU numeric label references.
/// Numeric labels like `1:` can be defined multiple times. References like `1b`
/// (backward) and `1f` (forward) must resolve to the nearest matching definition.
fn resolve_numeric_label_refs(statements: &[AsmStatement]) -> Result<Vec<AsmStatement>, String> {
    // First pass: collect all numeric label definition positions
    let mut label_defs: FxHashMap<String, Vec<(usize, usize)>> = FxHashMap::default();
    let mut instance_counter: FxHashMap<String, usize> = FxHashMap::default();

    for (i, stmt) in statements.iter().enumerate() {
        if let AsmStatement::Label(name) = stmt {
            if is_numeric_label(name) {
                let instance = instance_counter.entry(name.clone()).or_insert(0);
                label_defs
                    .entry(name.clone())
                    .or_default()
                    .push((i, *instance));
                *instance += 1;
            }
        }
    }

    if label_defs.is_empty() {
        reject_unresolved_numeric_refs(statements)?;
        return Ok(statements.to_vec());
    }

    // Second pass: rewrite labels and references
    let mut result = Vec::with_capacity(statements.len());

    for (i, stmt) in statements.iter().enumerate() {
        match stmt {
            AsmStatement::Label(name) if is_numeric_label(name) => {
                if let Some(defs) = label_defs.get(name) {
                    for &(def_idx, inst_id) in defs {
                        if def_idx == i {
                            let new_name = format!(".Lnum_{}_{}", name, inst_id);
                            result.push(AsmStatement::Label(new_name));
                            break;
                        }
                    }
                } else {
                    result.push(stmt.clone());
                }
            }
            AsmStatement::Instruction {
                mnemonic,
                operands,
                raw_operands,
            } => {
                let new_operands: Vec<Operand> = operands
                    .iter()
                    .map(|op| rewrite_numeric_ref_in_operand(op, i, &label_defs))
                    .collect();
                result.push(AsmStatement::Instruction {
                    mnemonic: mnemonic.clone(),
                    operands: new_operands,
                    raw_operands: raw_operands.clone(),
                });
            }
            AsmStatement::Directive(dir) => {
                result.push(AsmStatement::Directive(rewrite_numeric_refs_in_directive(
                    dir,
                    i,
                    &label_defs,
                )));
            }
            _ => result.push(stmt.clone()),
        }
    }

    reject_unresolved_numeric_refs(&result)?;
    Ok(result)
}

/// A numeric reference still present after the rewrite names no definition
/// (`j 3f` with no later `3:`). Left alone it becomes an undefined symbol
/// literally called "3f" that only fails at link time, if at all; GAS 2.47
/// rejects it: "local label `"3" (instance number 1 of a fb label)' is not
/// defined".
fn reject_unresolved_numeric_refs(statements: &[AsmStatement]) -> Result<(), String> {
    fn numeric_ref(s: &str) -> Option<&str> {
        let inner = match (s.find('('), s.starts_with('%') && s.ends_with(')')) {
            (Some(open), true) => s[open + 1..s.len() - 1].trim(),
            _ => s,
        };
        parse_numeric_label_ref(inner).map(|_| inner)
    }
    for stmt in statements {
        let AsmStatement::Instruction { operands, .. } = stmt else {
            continue;
        };
        for op in operands {
            let name = match op {
                Operand::Symbol(s) | Operand::Label(s) | Operand::SymbolOffset(s, _) => s,
                Operand::MemSymbol { symbol, .. } => symbol,
                _ => continue,
            };
            if let Some(r) = numeric_ref(name) {
                let (label, _) = parse_numeric_label_ref(r).unwrap_or((r, false));
                return Err(format!(
                    "local label `\"{label}\" (instance number 1 of a fb label)' is not defined (`{r}`)"
                ));
            }
        }
    }
    Ok(())
}

/// Rewrite a numeric label reference in an operand to a synthetic label name.
fn rewrite_numeric_ref_in_operand(
    op: &Operand,
    stmt_idx: usize,
    label_defs: &FxHashMap<String, Vec<(usize, usize)>>,
) -> Operand {
    match op {
        Operand::Symbol(s) => {
            if let Some(new_name) = resolve_numeric_ref_name(s, stmt_idx, label_defs) {
                Operand::Symbol(new_name)
            } else if let Some(new_name) = rewrite_modifier_numeric_ref(s, stmt_idx, label_defs) {
                Operand::Symbol(new_name)
            } else {
                op.clone()
            }
        }
        Operand::MemSymbol {
            base,
            symbol,
            modifier,
        } => {
            let new_symbol = resolve_numeric_ref_name(symbol, stmt_idx, label_defs)
                .or_else(|| rewrite_modifier_numeric_ref(symbol, stmt_idx, label_defs));
            match new_symbol {
                Some(symbol) => Operand::MemSymbol {
                    base: base.clone(),
                    symbol,
                    modifier: modifier.clone(),
                },
                None => op.clone(),
            }
        }
        Operand::Label(s) => {
            if let Some(new_name) = resolve_numeric_ref_name(s, stmt_idx, label_defs) {
                Operand::Label(new_name)
            } else {
                op.clone()
            }
        }
        Operand::SymbolOffset(s, off) => {
            if let Some(new_name) = resolve_numeric_ref_name(s, stmt_idx, label_defs) {
                Operand::SymbolOffset(new_name, *off)
            } else {
                op.clone()
            }
        }
        _ => op.clone(),
    }
}

/// `%pcrel_lo(1b)`: the GNU idiom that names an `auipc` by a numeric label
/// (`1: auipc a0, %pcrel_hi(sym)` then `addi a0, a0, %pcrel_lo(1b)`). The
/// reference sits inside the modifier, so the bare-operand rewrite above does
/// not see it; left alone it becomes an undefined symbol called "1b".
fn rewrite_modifier_numeric_ref(
    s: &str,
    stmt_idx: usize,
    label_defs: &FxHashMap<String, Vec<(usize, usize)>>,
) -> Option<String> {
    let open = s.find('(')?;
    if !s.starts_with('%') || !s.ends_with(')') {
        return None;
    }
    let inner = s[open + 1..s.len() - 1].trim();
    let new_inner = resolve_numeric_ref_name(inner, stmt_idx, label_defs)?;
    Some(format!("{}({})", &s[..open], new_inner))
}

/// Resolve a numeric label reference like "1b" or "2f" to a synthetic label name.
fn resolve_numeric_ref_name(
    symbol: &str,
    stmt_idx: usize,
    label_defs: &FxHashMap<String, Vec<(usize, usize)>>,
) -> Option<String> {
    let (label_name, is_backward) = parse_numeric_label_ref(symbol)?;
    let defs = label_defs.get(label_name)?;

    if is_backward {
        let mut best: Option<usize> = None;
        for &(def_idx, inst_id) in defs {
            if def_idx < stmt_idx {
                best = Some(inst_id);
            }
        }
        best.map(|inst_id| format!(".Lnum_{}_{}", label_name, inst_id))
    } else {
        for &(def_idx, inst_id) in defs {
            if def_idx > stmt_idx {
                return Some(format!(".Lnum_{}_{}", label_name, inst_id));
            }
        }
        None
    }
}

/// Rewrite numeric label references in the directives that carry
/// expressions: data values, `.org` and `.set`. `.` is left alone: the
/// writer binds it to each field's own position (a synthetic label placed
/// before the directive gave every field of `.long 0, .` the first one's).
fn rewrite_numeric_refs_in_directive(
    dir: &Directive,
    stmt_idx: usize,
    label_defs: &FxHashMap<String, Vec<(usize, usize)>>,
) -> Directive {
    let expr = |e: &str| {
        crate::backend::asm_expr::rewrite_numeric_label_refs(e, |r| {
            resolve_numeric_ref_name(r, stmt_idx, label_defs)
        })
    };
    let vals = |vals: &[DataValue]| -> Vec<DataValue> {
        vals.iter()
            .map(|v| match v {
                DataValue::Expression(e) => DataValue::Expression(expr(e)),
                DataValue::Integer(_) => v.clone(),
            })
            .collect()
    };
    match dir {
        Directive::Byte(v) => Directive::Byte(vals(v)),
        Directive::Short(v) => Directive::Short(vals(v)),
        Directive::Long(v) => Directive::Long(vals(v)),
        Directive::Quad(v) => Directive::Quad(vals(v)),
        Directive::Leb128 { signed, values } => Directive::Leb128 {
            signed: *signed,
            values: vals(values),
        },
        Directive::Org { expr: e, fill } => Directive::Org {
            expr: expr(e),
            fill: *fill,
        },
        Directive::Set(name, e) => Directive::Set(name.clone(), expr(e)),
        _ => dir.clone(),
    }
}

impl ElfWriter {
    pub fn new() -> Self {
        let mut w = Self {
            // The padding vocabulary follows the target, not the march: measured
            // on GNU as 2.47, `-march=rv64im` pads `.p2align` exactly like
            // `rv64gc` — including the 2-byte `c.nop` — because the fill lands
            // where control flow never goes and GAS therefore does not gate it
            // on RVC. Coupling it to `EF_RISCV_RVC` instead was a guess that
            // the oracle disproves.
            base: ElfWriterBase::new(RISCV_NOP, 2)
                .with_small_nop(RISCV_C_NOP)
                .with_code_align_floor(2),
            pending_branch_relocs: Vec::new(),
            pcrel_hi_counter: 0,
            elf_flags: EF_RISCV_FLOAT_ABI_DOUBLE | EF_RISCV_RVC,
            elf_class: ELFCLASS64,
            no_relax: false,
            pic: false,
            rvc: true,
            option_stack: Vec::new(),
            pending_lebs: Vec::new(),
            leb_widths: Vec::new(),
            leb_retry: None,
            pending_cfi: Vec::new(),
            branch_count: 0,
            relax_branches: Vec::new(),
            branch_lengths: FxHashMap::default(),
            branch_retry: false,
        };
        // Data values capture code labels symbolically; whether they end up
        // folded or relocated is decided by the relax state at the end
        // (`process_statements`), as in GNU as.
        w.base.keep_code_labels = true;
        w
    }

    /// Set the ELF e_flags (e.g., to change float ABI from the default double-float).
    ///
    pub fn set_elf_flags(&mut self, flags: u32) {
        self.elf_flags = flags;
        // GNU as derives both from one `riscv_opts.rvc`.
        self.set_rvc(flags & EF_RISCV_RVC != 0);
    }

    /// `riscv_opts.rvc`: the step a relaxing linker may shrink code in, and
    /// with it the alignment directives record for code (`insn_alignment`).
    fn set_rvc(&mut self, rvc: bool) {
        self.rvc = rvc;
        // GNU as 2.47, measured: `.text; nop` records addralign 2 for rv64gc
        // and 4 for rv64imafd, and so does any alignment directive in code.
        let insn_alignment = if rvc { 2 } else { 4 };
        self.base.set_code_align_floor(insn_alignment);
        self.base.set_text_align(insn_alignment);
    }

    /// Set the ELF class (ELFCLASS32 or ELFCLASS64).
    pub fn set_elf_class(&mut self, class: u8) {
        self.elf_class = class;
    }

    // R_RISCV_RELAX relocations are emitted alongside CALL_PLT, BRANCH, and
    // JAL relocations so the linker can perform relaxation (shortening
    // auipc+jalr to jal, etc.). This is required because linker relaxation
    // changes code layout, which would invalidate any locally-resolved offsets.

    /// R_RISCV_RELAX ELF relocation type
    const R_RISCV_RELAX: u32 = 51;

    /// R_RISCV_ALIGN ELF relocation type - marks alignment padding that the
    /// linker may need to adjust during relaxation.
    const R_RISCV_ALIGN: u32 = 43;
    /// `.uleb128 A - B` across relaxable code: the linker sets the LEB128 at
    /// the place to S+A of the first, then subtracts S+A of the second.
    const R_RISCV_32_PCREL: u32 = 57;
    const R_RISCV_SET_ULEB128: u32 = 60;
    const R_RISCV_SUB_ULEB128: u32 = 61;

    /// Code alignment as GNU as does it: in a relaxable code section a
    /// worst-case NOP run under an `R_RISCV_ALIGN` the linker trims, else
    /// plain (capped) padding.
    fn emit_align_with_reloc(
        &mut self,
        align_bytes: u64,
        fill: Option<u8>,
        max_pad: Option<u64>,
    ) -> Result<(), String> {
        // No `align_bytes <= 1` short cut here: a RISC-V *code* section raises a
        // sub-instruction alignment to the instruction size before deciding it
        // has nothing to do, and that decision needs the section's flags, which
        // only the shared writer has. Short-cutting made `.byte 1;
        // .p2align 0,,3` emit no padding where GNU as 2.47 pads to 4.
        // Which cap applies is a property of the SECTION, not of the mnemonic.
        //
        // GAS's `s_align` calls `md_do_align`, and the RISC-V hook fires only for
        // a code section with no fill operand (`fill == NULL && len == 0`); there
        // it hands the exponent to `riscv_frag_align_code (int n)`, which has no
        // `max` argument at all -- and that function itself refuses to run in
        // three situations, each of which puts the directive back on the generic
        // `frag_align (n, *fill, max)` path that compares the cap against the
        // padding in BYTES and skips the whole directive if the padding exceeds
        // it:
        //
        //   1. `n == 0`                       (`.p2align 0`, align to 1 byte)
        //   2. `bytes <= insn_alignment`, where `insn_alignment = rvc ? 2 : 4`
        //   3. `!riscv_opts.relax`            (`.option push; .option norelax`)
        //
        // Conjunction 2 has an input that is easy to misread: measured on the
        // local as 2.44 with only the ISA string changed, `.text; .byte 1;
        // .p2align 2,,1; .byte 2` pads 0 bytes at `-march=rv64im` and 3 bytes
        // at `-march=rv64gc`.  `insn_alignment` is not "the narrowest
        // instruction this assembler emits" but the step in which the LINKER
        // may shrink code -- GNU ld relaxes to compressed forms (`c.j`,
        // `c.lui`, ...) exactly when the object's `EF_RISCV_RVC` is set, and
        // GNU as derives both that flag and `insn_alignment` from the same
        // `riscv_opts.rvc`.  These objects declare `EF_RISCV_RVC` (as GCC's
        // do), so the default is the rv64gc row; `.option norvc` selects the
        // rv64im one.  (This backend once fixed it at 4 because it emits no
        // compressed encodings itself -- which left a `.p2align 2` with no
        // relocation for a linker that then shrank code before it in 2-byte
        // steps.)
        //
        // Measured rows (as 2.44 here, and as 2.47.20260726 -- the oracle
        // `scripts/ensure_gas_247.sh` provisions -- for the rows marked 47; pad
        // bytes after `.byte 1`, with the anchor `.byte 2` after the directive;
        // under the hook the count is the fixed 1-byte step to the instruction
        // alignment plus the worst-case run):
        //   .text  .p2align 4,,2      -> 15   cap dropped by the hook (1 + 14)
        //   .text  .p2align 3,,1      -> 7    cap dropped by the hook (1 + 6)
        //   .text  .p2align 2,,1      -> 3    rv64gc: hook (1 + 2)
        //   .text  norvc .p2align 2,,1 -> 0   rv64im: bytes 4 <= 4, cap honoured
        //   .text  .p2align 2,,3      -> 3    pad fits the cap
        //   .text  .p2align 1,,1      -> 1    (and 1 for any cap: unobservable,
        //                                     see the note in the matrix test)
        //   .text  norelax .p2align 3,,1  -> 0    relax off: cap honoured
        //   .text  norelax .p2align 3,,15 -> 7
        //   .text  .balign 8,,4       -> 7  (47)     .text .balign 4,,1 -> 3 (gc)
        //   .text  .p2align 4,0x13,2  -> 0  (47)    fill routes around the hook
        //   .text  .p2align 4,0x13,99 -> 15x0x13 (47)
        //   .data  .p2align 4,,2      -> 0  (47)     .data .balign 8,,4 -> 0 (47)
        //   .data  .p2align 4,,15     -> 15 zeros (47)
        let section_flags_exec = self
            .base
            .sections
            .get(&self.base.current_section)
            .map_or(true, |s| s.sh_flags & SHF_EXECINSTR != 0);
        // GNU as's `insn_alignment`: the step in which a relaxing linker may
        // shrink code, 2 with RVC (`.option rvc`, the default: the objects
        // declare `EF_RISCV_RVC`) and 4 without.
        let insn_alignment: u64 = if self.rvc { 2 } else { 4 };
        let hook =
            fill.is_none() && section_flags_exec && align_bytes > insn_alignment && !self.no_relax;
        // `align_bytes > insn_alignment` already excludes `n == 0` and `n == 1`
        // (1 and 2 are both <= 2), so the `n != 0` conjunct is deliberately not
        // spelled out.
        if hook {
            // `riscv_frag_align_code`: the address after this directive is not
            // known until link time (relaxing an earlier `call` shrinks it), so
            // GNU as first brings the offset to the instruction alignment
            // (fixed padding), then writes the WORST case, `align -
            // insn_alignment` bytes of NOPs, under an `R_RISCV_ALIGN` whose
            // addend is that length.  Every linker derives the alignment from
            // the addend (the smallest power of two above it) and keeps only
            // the NOPs the final address needs; lccc-ld does the same
            // (`riscv/linker/align.rs`).  Padding only to the boundary -- what
            // this backend did -- names too small an alignment whenever the
            // pad is short (2 bytes of a `.p2align 4` read as "align 4") and
            // too few bytes whenever earlier code shrinks: GNU ld then refuses
            // the object ("12 bytes required for alignment to 16-byte
            // boundary, but only 8 present"), and an aligned site got no
            // relocation at all.
            self.base.align_code_step(insn_alignment)?;
            let at = self.base.current_offset();
            let worst = align_bytes - insn_alignment;
            self.base.emit_code_nops(worst, align_bytes)?;
            if let Some(s) = self.base.sections.get_mut(&self.base.current_section) {
                s.relocs.push(ObjReloc {
                    offset: at,
                    reloc_type: Self::R_RISCV_ALIGN,
                    symbol_name: String::new(),
                    addend: worst as i64,
                });
            }
            return Ok(());
        }
        // Everything else is GAS's generic `frag_align (n, *fill, max)`: the
        // padding is final (nothing before it can move, or the alignment is
        // within one instruction step), the cap is honoured, and no
        // relocation is needed.
        self.base.align_to_capped_ex(align_bytes, max_pad, fill)?;
        Ok(())
    }

    /// Process all parsed assembly statements.
    ///
    /// A forward-referencing `.uleb128` takes its width before its value is
    /// known.  GNU as relaxes such fragments to the canonical length; here the
    /// whole input is assembled again with the widths the previous pass
    /// measured, which reaches the same layout.  Widths only grow (capped at
    /// 10, a u64's LEB128), so this terminates; inputs without such a value
    /// assemble once.  Branches work the same way: they start at their
    /// shortest form and a pass that finds one too short assembles again with
    /// it longer (see [`Self::emit_relaxable_branch`]).
    pub fn process_statements(&mut self, statements: &[AsmStatement]) -> Result<(), String> {
        let (flags, class) = (self.elf_flags, self.elf_class);
        let ptr_size = if class == ELFCLASS32 { 4 } else { 8 };
        let statements = cfi_lower::lower_cfi(statements.to_vec(), &cfi::target(ptr_size))?;
        loop {
            self.process_statements_once(&statements)?;
            let widths = self.leb_retry.take();
            if widths.is_none() && !self.branch_retry {
                return Ok(());
            }
            let mut fresh = ElfWriter::new();
            fresh.set_elf_flags(flags);
            fresh.set_elf_class(class);
            fresh.leb_widths = widths.unwrap_or_else(|| std::mem::take(&mut self.leb_widths));
            fresh.branch_lengths = std::mem::take(&mut self.branch_lengths);
            *self = fresh;
        }
    }

    fn process_statements_once(&mut self, statements: &[AsmStatement]) -> Result<(), String> {
        let statements = resolve_numeric_label_refs(statements)?;
        for stmt in &statements {
            self.process_statement(stmt)?;
        }
        // Merge subsections (e.g., .text.__subsection.1 → .text) before resolving
        // relocations. This is critical for kernel ALTERNATIVE macros which use
        // .subsection 1 to place alternative code within the same section.
        let remap = self.base.merge_subsections()?;
        self.base.allocate_local_commons()?;
        // Fix up pending references that pointed to now-merged subsection names.
        // Deferred expressions and branch relocs created inside a subsection store the
        // subsection name as their section; after merging, that section no longer exists.
        // Remap them to the parent section with the correct offset adjustment.
        if !remap.is_empty() {
            for reloc in &mut self.pending_branch_relocs {
                if let Some((parent, offset_adj)) = remap.get(&reloc.section) {
                    reloc.offset += offset_adj;
                    reloc.section = parent.clone();
                }
            }
            for branch in &mut self.relax_branches {
                if let Some((parent, offset_adj)) = remap.get(&branch.section) {
                    branch.offset += offset_adj;
                    branch.section = parent.clone();
                }
            }
        }
        self.relax_branch_lengths();
        if self.branch_retry {
            return Ok(());
        }
        // GNU as (2.44, measured) relocates every data difference involving a
        // code label -- even one between labels of an `.option norelax`
        // stretch -- unless relaxation is off at the END of the input, and
        // folds them all otherwise: the decision is per object, made here.
        self.base.keep_code_labels = !self.no_relax;
        for (section, offset, size, fixup) in self.base.resolve_pending_data(&remap)? {
            self.emit_data_fixup(&section, offset, size, &fixup)?;
        }
        self.resolve_pending_lebs(&remap)?;
        self.resolve_pending_cfi(&remap)?;
        if self.leb_retry.is_some() {
            return Ok(());
        }
        self.resolve_local_branches()?;
        Ok(())
    }

    fn process_statement(&mut self, stmt: &AsmStatement) -> Result<(), String> {
        match stmt {
            AsmStatement::Empty => Ok(()),

            AsmStatement::Label(name) => {
                self.base.define_label(name)?;
                Ok(())
            }

            AsmStatement::Directive(directive) => self.process_directive(directive),

            AsmStatement::Instruction {
                mnemonic,
                operands,
                raw_operands,
            } => self.process_instruction(mnemonic, operands, raw_operands),
        }
    }

    fn process_directive(&mut self, directive: &Directive) -> Result<(), String> {
        match directive {
            Directive::PushSection(info) => {
                self.base.push_section(
                    &info.name,
                    &info.flags,
                    info.flags_explicit,
                    Some(info.sec_type.as_str()),
                );
                Ok(())
            }
            Directive::PopSection => {
                self.base.pop_section();
                Ok(())
            }
            Directive::Previous => {
                self.base.restore_previous_section();
                Ok(())
            }
            Directive::Section(info) => {
                self.base.process_section_directive(
                    &info.name,
                    &info.flags,
                    info.flags_explicit,
                    Some(info.sec_type.as_str()),
                );
                Ok(())
            }

            Directive::Text => {
                self.base.switch_to_standard_section(
                    ".text",
                    SHT_PROGBITS,
                    SHF_ALLOC | SHF_EXECINSTR,
                );
                Ok(())
            }
            Directive::Data => {
                self.base
                    .switch_to_standard_section(".data", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE);
                Ok(())
            }
            Directive::Bss => {
                self.base
                    .switch_to_standard_section(".bss", SHT_NOBITS, SHF_ALLOC | SHF_WRITE);
                Ok(())
            }
            Directive::Rodata => {
                self.base
                    .switch_to_standard_section(".rodata", SHT_PROGBITS, SHF_ALLOC);
                Ok(())
            }

            Directive::Globl(sym) => {
                self.base.set_global(sym);
                Ok(())
            }
            Directive::Weak(sym) => {
                self.base.set_weak(sym);
                Ok(())
            }

            Directive::SymVisibility(sym, vis) => {
                let v = match vis {
                    Visibility::Hidden => STV_HIDDEN,
                    Visibility::Protected => STV_PROTECTED,
                    Visibility::Internal => STV_INTERNAL,
                };
                self.base.set_visibility(sym, v);
                Ok(())
            }

            Directive::Type(sym, st) => {
                let elf_type = match st {
                    SymbolType::Function => STT_FUNC,
                    SymbolType::Object => STT_OBJECT,
                    SymbolType::TlsObject => STT_TLS,
                    SymbolType::GnuIndirectFunction => STT_GNU_IFUNC,
                    SymbolType::NoType => STT_NOTYPE,
                };
                self.base.set_symbol_type(sym, elf_type);
                Ok(())
            }

            Directive::Size(sym, size_expr) => {
                match size_expr {
                    SizeExpr::CurrentMinus(label) => {
                        self.base.set_symbol_size(sym, Some(label), None);
                    }
                    SizeExpr::Absolute(size) => {
                        self.base.set_symbol_size(sym, None, Some(*size));
                    }
                }
                Ok(())
            }

            Directive::Align { bytes, tail } => {
                self.emit_align_with_reloc(*bytes, tail.fill, tail.max_pad)?;
                Ok(())
            }

            Directive::Balign {
                bytes,
                fill,
                max_pad,
            } => {
                self.emit_align_with_reloc(*bytes, *fill, *max_pad)?;
                Ok(())
            }

            Directive::Byte(values) => {
                for dv in values {
                    self.emit_data_value(dv, 1)?;
                }
                Ok(())
            }

            Directive::Short(values) => {
                for dv in values {
                    self.emit_data_value(dv, 2)?;
                }
                Ok(())
            }

            Directive::Long(values) => {
                for dv in values {
                    self.emit_data_value(dv, 4)?;
                }
                Ok(())
            }

            Directive::Leb128 { signed, values } => {
                for dv in values {
                    self.emit_leb128(dv, *signed)?;
                }
                Ok(())
            }

            Directive::Quad(values) => {
                for dv in values {
                    self.emit_data_value(dv, 8)?;
                }
                Ok(())
            }

            Directive::Zero { size, fill } => {
                // emit_fill bounds the request (MAX_DIRECTIVE_FILL) and
                // writes with a single resize — the old
                // `emit_bytes(&vec![fill; size])` materialized the whole
                // run before any ceiling could apply.
                self.base.emit_fill(*size, *fill)?;
                Ok(())
            }

            Directive::Fill { total, elem, value } => {
                self.base.emit_fill_pattern(*total, *elem, *value)?;
                Ok(())
            }

            Directive::Asciz(s) => {
                self.base.nobits_string_guard(s)?;
                self.base.emit_bytes(s)?;
                self.base.emit_bytes(&[0])?;
                Ok(())
            }

            Directive::Ascii(s) => {
                self.base.nobits_string_guard(s)?;
                self.base.emit_bytes(s)?;
                Ok(())
            }

            Directive::Comm { sym, size, align } => self.base.declare_comm(sym, *size, *align),
            Directive::Lcomm { sym, size } => self.base.declare_lcomm(sym, *size),
            Directive::Local(syms) => {
                self.base.set_local(syms);
                Ok(())
            }

            Directive::Set(alias, target) => self.base.define_set(alias, target),

            Directive::ArchOption(opt) => {
                let opt = opt.trim();
                if opt == "norelax" {
                    self.no_relax = true;
                } else if opt == "relax" {
                    self.no_relax = false;
                } else if opt == "pic" {
                    self.pic = true;
                } else if opt == "nopic" {
                    self.pic = false;
                } else if opt == "rvc" {
                    self.set_rvc(true);
                } else if opt == "norvc" {
                    self.set_rvc(false);
                } else if opt == "push" {
                    self.option_stack.push((self.no_relax, self.pic, self.rvc));
                } else if opt == "pop" {
                    if let Some((no_relax, pic, rvc)) = self.option_stack.pop() {
                        self.no_relax = no_relax;
                        self.pic = pic;
                        self.set_rvc(rvc);
                    }
                }
                // `rvc` selects no encodings here (this backend emits none);
                // it is GNU as's `insn_alignment` input for code alignment,
                // and, as in GNU as (`riscv_set_rvc`), it marks the object.
                if self.rvc {
                    self.elf_flags |= EF_RISCV_RVC;
                }
                Ok(())
            }

            Directive::Attribute(_) => Ok(()),

            Directive::Org { expr, fill } => self.base.emit_org(expr, *fill),
            Directive::File(number, name) => {
                self.base.debug_file(*number, name.clone());
                Ok(())
            }
            Directive::Loc(file, line, column) => {
                self.base.debug_loc(*file, *line, *column);
                Ok(())
            }
            Directive::Ignored => Ok(()),
            Directive::Cfi(name, _) => Err(format!(
                "internal: {name} reached the ELF writer without CFI lowering"
            )),
            Directive::CfaAdvance { from, to } => self.emit_cfa_advance(from, to),
            Directive::Pcrel32(sym) => {
                let value = self.base.capture_data_symbol(sym)?;
                self.base.ensure_default_section();
                let (section, offset) = (
                    self.base.current_section.clone(),
                    self.base.current_offset(),
                );
                self.base.emit_placeholder_bytes(4)?;
                self.pending_cfi.push(PendingCfi::Pcrel32 {
                    section,
                    offset,
                    value,
                });
                Ok(())
            }

            Directive::Insn(args) => {
                self.base.ensure_text_section();
                // `.insn` is an instruction: it raises sh_addralign like
                // a statement-level insn (GAS 2.47 measured).
                self.base.note_instruction();
                match encode_insn_directive(args) {
                    Ok(EncodeResult::Word(word)) => {
                        self.base.emit_u32_le(word)?;
                        Ok(())
                    }
                    Ok(EncodeResult::Half(half)) => {
                        self.base.emit_u16_le(half)?;
                        Ok(())
                    }
                    Ok(_) => Ok(()),
                    Err(e) => Err(e),
                }
            }

            Directive::Incbin { path, skip, count } => {
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

            Directive::Subsection(n) => {
                self.base.set_subsection(*n);
                Ok(())
            }

            Directive::Unknown { name, args } => Err(format!(
                "unsupported RISC-V assembler directive: {} {}",
                name, args
            )),
        }
    }

    /// Emit a `size`-byte data value (`.byte`/`.2byte`/`.4byte`/`.8byte`):
    /// constants now, everything that names a symbol through the shared
    /// value model (`ElfWriterBase::classify`), turned into relocations by
    /// [`Self::emit_data_fixup`].  Symbol names may carry an addend spelled
    /// in (`key+144`, from inline-asm operand substitution); the evaluator
    /// reads that as the expression it is.
    fn emit_data_value(&mut self, dv: &DataValue, size: usize) -> Result<(), String> {
        let value = match dv {
            DataValue::Integer(v) => return self.base.emit_data_integer(*v, size),
            DataValue::Expression(expr) => self.base.capture_data_symbol(expr)?,
        };
        self.base.emit_data_value(value, size)
    }

    /// `.uleb128` / `.sleb128` (GNU as `s_riscv_leb128`): a value constant
    /// where it stands is encoded at once; otherwise only an unsigned one is
    /// accepted, and it is resolved after layout.
    fn emit_leb128(&mut self, dv: &DataValue, signed: bool) -> Result<(), String> {
        let value = match dv {
            DataValue::Integer(v) => return self.emit_leb128_constant(*v, signed),
            DataValue::Expression(expr) => self.base.capture_data_symbol(expr)?,
        };
        if let SetValue::Value(v) = &value
            && v.is_absolute()
        {
            return self.emit_leb128_constant(v.constant, signed);
        }
        if signed {
            return Err("non-constant .sleb128 is not supported".to_string());
        }
        let width = self
            .leb_widths
            .get(self.pending_lebs.len())
            .copied()
            .unwrap_or(1);
        self.base.ensure_default_section();
        let (section, offset) = (
            self.base.current_section.clone(),
            self.base.current_offset(),
        );
        self.base.emit_placeholder_bytes(width)?;
        self.pending_lebs.push(PendingLeb {
            section,
            offset,
            width,
            value,
        });
        Ok(())
    }

    /// A CFA advance from label `from` to `to`. The lowering appends
    /// `.eh_frame` after all code and no code changes size once emitted, so
    /// two labels of one (sub)section are already at their final distance:
    /// that fixes the form now. Labels of different subsections move apart
    /// when those merge; such an advance takes the 5-byte
    /// `DW_CFA_advance_loc4`, which holds any distance. The value (and its
    /// relocations) is written after layout ([`Self::resolve_pending_cfi`]).
    fn emit_cfa_advance(&mut self, from: &str, to: &str) -> Result<(), String> {
        let value = self.base.capture_data_symbol(&format!("{to} - {from}"))?;
        let width = match (self.base.labels.get(from), self.base.labels.get(to)) {
            (Some((sf, of)), Some((st, ot))) if sf == st && ot >= of => cfi::advance_width(ot - of),
            _ => 5,
        };
        self.base.ensure_default_section();
        let (section, offset) = (
            self.base.current_section.clone(),
            self.base.current_offset(),
        );
        self.base.emit_placeholder_bytes(width)?;
        self.pending_cfi.push(PendingCfi::Advance {
            section,
            offset,
            width,
            value,
        });
        Ok(())
    }

    /// Write the CFI fields recorded during the pass. An advance over code
    /// under relaxation keeps its SET/SUB pair (see [`cfi`]); otherwise it
    /// is a constant.
    fn resolve_pending_cfi(&mut self, remap: &elf::SubsectionRemap) -> Result<(), String> {
        for p in std::mem::take(&mut self.pending_cfi) {
            match p {
                PendingCfi::Advance {
                    section,
                    offset,
                    width,
                    value,
                } => {
                    let (section, offset) = ElfWriterBase::remap_place(remap, &section, offset);
                    let (fixup, constant) = self
                        .base
                        .resolve_value(&value, remap)
                        .map_err(|e| format!("CFA advance at {section}+{offset:#x}: {e}"))?;
                    let (delta, pair) = match (fixup, constant) {
                        (DataFixup::Constant(c), _) => (c, None),
                        (DataFixup::Diff { add, sub, addend }, Some(c)) => {
                            (c, Some((add, sub, addend)))
                        }
                        _ => {
                            return Err(format!(
                                "CFA advance at {section}+{offset:#x}: CFI locations in different sections"
                            ));
                        }
                    };
                    let delta = u64::try_from(delta).map_err(|_| {
                        format!("CFA advance at {section}+{offset:#x}: negative distance {delta}")
                    })?;
                    if cfi::advance_width(delta) > width {
                        return Err(format!(
                            "internal: CFA advance at {section}+{offset:#x} outgrew its {width}-byte form"
                        ));
                    }
                    let (bytes, field, set, sub_t) = cfi::encode_advance(delta, width)?;
                    let sec = self
                        .base
                        .sections
                        .get_mut(&section)
                        .ok_or_else(|| format!("CFA advance: unknown section {section}"))?;
                    let at = usize::try_from(offset).map_err(|e| e.to_string())?;
                    sec.data[at..at + width].copy_from_slice(&bytes);
                    if let Some((add, sub, addend)) = pair {
                        self.base
                            .add_reloc_at(&section, offset + field, set, &add, addend);
                        self.base
                            .add_reloc_at(&section, offset + field, sub_t, &sub, 0);
                    }
                }
                PendingCfi::Pcrel32 {
                    section,
                    offset,
                    value,
                } => {
                    let (section, offset) = ElfWriterBase::remap_place(remap, &section, offset);
                    let (fixup, _) = self
                        .base
                        .resolve_value(&value, remap)
                        .map_err(|e| format!("`.eh_frame' at {section}+{offset:#x}: {e}"))?;
                    let DataFixup::Abs { sym, addend } = fixup else {
                        return Err(format!(
                            "`.eh_frame' at {section}+{offset:#x}: a PC-relative CFI pointer must name a symbol"
                        ));
                    };
                    self.base
                        .add_reloc_at(&section, offset, Self::R_RISCV_32_PCREL, &sym, addend);
                }
            }
        }
        Ok(())
    }

    fn emit_leb128_constant(&mut self, v: i64, signed: bool) -> Result<(), String> {
        let mut enc = Vec::with_capacity(10);
        if signed {
            crate::backend::elf_writer_common::encode_sleb128(&mut enc, v);
        } else {
            crate::backend::elf_writer_common::encode_uleb128(&mut enc, v as u64);
        }
        self.base.emit_bytes(&enc)
    }

    /// Write every pending `.uleb128`, or note the widths a further pass
    /// needs.  A difference involving code labels under relaxation keeps an
    /// `R_RISCV_SET_ULEB128`/`R_RISCV_SUB_ULEB128` pair (GNU as
    /// `riscv_insert_uleb128_fixes`) over its assembly-time value; GNU as
    /// drops the expression's constant from the pair (it zeroes
    /// `X_add_number`), which the linker then loses -- here it rides on the
    /// SET relocation, where the linker adds it.  GNU as also pairs
    /// differences of data labels; those are exact at assembly time, so they
    /// are written as constants.
    fn resolve_pending_lebs(&mut self, remap: &elf::SubsectionRemap) -> Result<(), String> {
        let mut needed: Option<Vec<usize>> = None;
        for (i, p) in std::mem::take(&mut self.pending_lebs)
            .into_iter()
            .enumerate()
        {
            let (section, offset) = ElfWriterBase::remap_place(remap, &p.section, p.offset);
            let where_ = format!("`.uleb128' at {section}+{offset:#x}");
            let (fixup, constant) = self
                .base
                .resolve_value(&p.value, remap)
                .map_err(|e| format!("{where_}: {e}"))?;
            let value = match (&fixup, constant) {
                (DataFixup::Constant(c), _) => *c,
                (DataFixup::Diff { add, sub, addend }, Some(c)) => {
                    self.base.add_reloc_at(
                        &section,
                        offset,
                        Self::R_RISCV_SET_ULEB128,
                        add,
                        *addend,
                    );
                    self.base
                        .add_reloc_at(&section, offset, Self::R_RISCV_SUB_ULEB128, sub, 0);
                    c
                }
                (DataFixup::Diff { .. }, None) => {
                    return Err(format!(
                        "{where_}: a difference of symbols in different sections has no LEB128 relocation"
                    ));
                }
                (DataFixup::Abs { .. }, _) => {
                    return Err(format!("{where_}: non-constant .uleb128 is not supported"));
                }
            };
            match uleb128_fixed(value as u64, p.width) {
                Some(bytes) => {
                    let sec = self
                        .base
                        .sections
                        .get_mut(&section)
                        .ok_or_else(|| format!("{where_}: unknown section"))?;
                    let at = usize::try_from(offset).map_err(|e| e.to_string())?;
                    sec.data[at..at + p.width].copy_from_slice(&bytes);
                }
                None => {
                    let w = needed.get_or_insert_with(|| self.leb_widths.clone());
                    if w.len() <= i {
                        w.resize(i + 1, 1);
                    }
                    w[i] = w[i].max(uleb128_len(value as u64));
                }
            }
        }
        self.leb_retry = needed;
        Ok(())
    }

    /// The relocation(s) for one deferred data value.  GAS 2.44 (riscv64,
    /// -mno-relax): `sym + k` is R_RISCV_32/64 (there is none for 1 or 2
    /// bytes: refused), and EVERY difference not folded to a constant --
    /// `.`-relative ones included -- is an ADD/SUB pair of the value's
    /// width, so the linker computes it after relaxation.
    fn emit_data_fixup(
        &mut self,
        section: &str,
        offset: u64,
        size: usize,
        fixup: &DataFixup,
    ) -> Result<(), String> {
        let (add_t, sub_t) = match size {
            1 => (RelocType::Add8, RelocType::Sub8),
            2 => (RelocType::Add16, RelocType::Sub16),
            4 => (RelocType::Add32, RelocType::Sub32),
            _ => (RelocType::Add64, RelocType::Sub64),
        };
        match fixup {
            DataFixup::Constant(_) => {}
            DataFixup::Abs { sym, addend } => {
                let t = match size {
                    4 => RelocType::Abs32,
                    8 => RelocType::Abs64,
                    _ => {
                        return Err(format!(
                            "{section}+{offset:#x}: a {size}-byte data value cannot name a symbol on RISC-V (no such relocation)"
                        ));
                    }
                };
                self.base
                    .add_reloc_at(section, offset, t.elf_type(), sym, *addend);
            }
            DataFixup::Diff { add, sub, addend } => {
                self.base
                    .add_reloc_at(section, offset, add_t.elf_type(), add, *addend);
                self.base
                    .add_reloc_at(section, offset, sub_t.elf_type(), sub, 0);
            }
        }
        Ok(())
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

        let rv64 = self.elf_class == ELFCLASS64;
        let encoded = match mnemonic {
            "li" => encode_li(operands, rv64),
            "lga" | "la.tls.ie" | "la.tls.gd" => {
                encode_got_pseudo(mnemonic, operands, self.elf_class == ELFCLASS64)
            }
            "la" if self.pic => encode_got_pseudo(mnemonic, operands, self.elf_class == ELFCLASS64),
            _ => encode_instruction(mnemonic, operands, raw_operands),
        };
        match encoded {
            Ok(EncodeResult::Word(word)) => {
                let spelled_addi = mnemonic.eq_ignore_ascii_case("li") && word & 0x7F == 0b0010011;
                let mnemonic = if spelled_addi && !li_rvc_aliases_match(operands, rv64) {
                    "" // GNU as's `li d,j`: no 2-byte form
                } else {
                    mnemonic
                };
                self.emit_instruction_word(word, mnemonic, operands.len())
            }
            Ok(EncodeResult::Half(half)) => {
                self.base.emit_u16_le(half)?;
                Ok(())
            }
            Ok(EncodeResult::WordWithReloc { word, reloc }) => {
                let elf_type = reloc.reloc_type.elf_type();
                let is_pcrel_hi =
                    elf_type == 23 || elf_type == 20 || elf_type == 22 || elf_type == 21;

                if is_pcrel_hi {
                    let label = format!(".Lpcrel_hi{}", self.pcrel_hi_counter);
                    self.pcrel_hi_counter += 1;
                    let section = self.base.current_section.clone();
                    let offset = self.base.current_offset();
                    self.base.labels.insert(label, (section, offset));
                }

                // For BRANCH (16), JAL (17): always emit as external relocations
                // so the linker resolves offsets correctly after relaxation.
                // For CALL_PLT (19): emit with paired R_RISCV_RELAX.
                let is_branch_or_jal = elf_type == 16 || elf_type == 17;
                let is_call_plt = elf_type == 19;

                if is_call_plt {
                    self.base
                        .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                    if !self.no_relax {
                        self.base.add_reloc(Self::R_RISCV_RELAX, String::new(), 0);
                    }
                    self.base.emit_u32_le(word)?;
                } else if is_branch_or_jal {
                    self.emit_relaxable_branch(
                        word,
                        elf_type,
                        (mnemonic, operands.len()),
                        &reloc.symbol,
                        reloc.addend,
                    )?;
                } else {
                    let is_local = reloc.symbol.starts_with(".L") || reloc.symbol.starts_with(".l");

                    if is_local {
                        let offset = self.base.current_offset();
                        self.pending_branch_relocs.push(PendingReloc {
                            section: self.base.current_section.clone(),
                            offset,
                            reloc_type: elf_type,
                            symbol: reloc.symbol.clone(),
                            addend: reloc.addend,
                            pcrel_hi_offset: None,
                            relax: !self.no_relax && relax_paired(elf_type),
                        });
                        self.base.emit_u32_le(word)?;
                    } else {
                        self.base
                            .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                        if !self.no_relax && relax_paired(elf_type) {
                            self.base.add_reloc(Self::R_RISCV_RELAX, String::new(), 0);
                        }
                        self.base.emit_u32_le(word)?;
                    }
                }
                Ok(())
            }
            Ok(EncodeResult::Words(words)) => {
                // Only `li` expands to several relocation-free words, which
                // GNU as assembles (`md_assemblef`) one by one as written.
                for word in words {
                    self.emit_instruction_word(word, compress::li_word_mnemonic(word), 3)?;
                }
                Ok(())
            }
            Ok(EncodeResult::WordsWithRelocs(items)) => {
                let mut pcrel_hi_label: Option<String> = None;

                for (word, reloc_opt) in &items {
                    if let Some(reloc) = reloc_opt {
                        let elf_type = reloc.reloc_type.elf_type();
                        let is_pcrel_hi = elf_type == 23;
                        let is_got_hi = elf_type == 20;
                        let is_tls_gd_hi = elf_type == 22;
                        let is_tls_got_hi = elf_type == 21;

                        if is_pcrel_hi || is_got_hi || is_tls_gd_hi || is_tls_got_hi {
                            let label = format!(".Lpcrel_hi{}", self.pcrel_hi_counter);
                            self.pcrel_hi_counter += 1;
                            let section = self.base.current_section.clone();
                            let offset = self.base.current_offset();
                            self.base.labels.insert(label.clone(), (section, offset));
                            pcrel_hi_label = Some(label);

                            self.base
                                .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                            // GAS 2.47 pairs R_RISCV_RELAX with the hi part of
                            // `lla`/`la`/`lga`, never with a TLS GOT/GD hi20.
                            if !self.no_relax && !is_tls_gd_hi && !is_tls_got_hi {
                                self.base.add_reloc(Self::R_RISCV_RELAX, String::new(), 0);
                            }
                            self.base.emit_u32_le(*word)?;
                            continue;
                        }

                        let is_pcrel_lo12_i = elf_type == 24;
                        let is_pcrel_lo12_s = elf_type == 25;

                        if let Some(hi_label) = pcrel_hi_label
                            .as_ref()
                            .filter(|_| is_pcrel_lo12_i || is_pcrel_lo12_s)
                        {
                            let hi_label = hi_label.clone();
                            self.base.add_reloc(elf_type, hi_label, 0);
                            if !self.no_relax {
                                self.base.add_reloc(Self::R_RISCV_RELAX, String::new(), 0);
                            }
                            self.base.emit_u32_le(*word)?;
                            continue;
                        }

                        // For BRANCH (16), JAL (17): always emit as external
                        // relocations. For CALL_PLT (19): emit with R_RISCV_RELAX.
                        let is_branch_or_jal = elf_type == 16 || elf_type == 17;
                        let is_call_plt = elf_type == 19;
                        if is_call_plt {
                            self.base
                                .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                            if !self.no_relax {
                                self.base.add_reloc(Self::R_RISCV_RELAX, String::new(), 0);
                            }
                        } else if is_branch_or_jal {
                            self.base
                                .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                        } else {
                            let is_local =
                                reloc.symbol.starts_with(".L") || reloc.symbol.starts_with(".l");
                            if is_local {
                                let offset = self.base.current_offset();
                                self.pending_branch_relocs.push(PendingReloc {
                                    section: self.base.current_section.clone(),
                                    offset,
                                    reloc_type: elf_type,
                                    symbol: reloc.symbol.clone(),
                                    addend: reloc.addend,
                                    pcrel_hi_offset: None,
                                    relax: !self.no_relax && relax_paired(elf_type),
                                });
                            } else {
                                self.base
                                    .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                            }
                        }
                    }
                    self.base.emit_u32_le(*word)?;
                }
                Ok(())
            }
            Ok(EncodeResult::Skip) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// A relocation-free instruction, in its 2-byte form when RVC is on and
    /// the spelling has one, as GNU as does (see
    /// [`compress::spelling_allows`]).  Linker relaxation never does this: it
    /// only shrinks call, `lui`/`auipc` and alignment sequences.
    fn emit_instruction_word(
        &mut self,
        word: u32,
        mnemonic: &str,
        operand_count: usize,
    ) -> Result<(), String> {
        if !self.rvc {
            return self.base.emit_u32_le(word);
        }
        if mnemonic.eq_ignore_ascii_case("unimp") {
            return self.base.emit_u16_le(0); // c.unimp
        }
        match compress::try_compress(word, self.elf_class == ELFCLASS64)
            .filter(|&half| compress::spelling_allows(mnemonic, operand_count, half))
        {
            Some(half) => self.base.emit_u16_le(half),
            None => self.base.emit_u32_le(word),
        }
    }

    /// GNU as's branch relaxation (`relaxed_branch_length` and
    /// `md_convert_frag_branch`, gas/config/tc-riscv.c).  A branch or jump to
    /// a symbol starts at its shortest form -- 2 bytes when the spelling has
    /// an RVC alias (`beqz`/`bnez` and `beq`/`bne rs, x0` on x8..x15, `j`,
    /// RV32 `jal target`), else 4 -- and a pass that finds it too short
    /// assembles again with it longer:
    ///
    /// * 2: `c.beqz`/`c.bnez`/`c.j`/`c.jal` + `R_RISCV_RVC_BRANCH`/`_JUMP`;
    /// * 4: the instruction + `R_RISCV_BRANCH`/`R_RISCV_JAL`;
    /// * 6 / 8: a conditional branch beyond +-4 KiB, or whose target is
    ///   undefined, weak or in another section, is the inverted `c.b*` / `b*`
    ///   over a `jal x0, target` (+-1 MiB), so it links wherever the target
    ///   lands.  Jumps stop at 4: GNU as assumes they reach.
    ///
    /// Displacement fields under a relocation stay zero; the relocation
    /// carries the target.  Linker relaxation only deletes bytes, so a branch
    /// that reaches here still reaches after it.
    fn emit_relaxable_branch(
        &mut self,
        word: u32,
        elf_type: u32,
        (mnemonic, operand_count): (&str, usize),
        symbol: &str,
        addend: i64,
    ) -> Result<(), String> {
        const R_RISCV_JAL: u32 = 17;
        const JAL_X0: u32 = 0b1101111;
        let jump = elf_type == R_RISCV_JAL;
        let half = if self.rvc {
            self.rvc_branch_form(word, jump, mnemonic, operand_count)
        } else {
            None
        };
        let ordinal = self.branch_count;
        self.branch_count += 1;
        let shortest = if half.is_some() { 2 } else { 4 };
        let length = self
            .branch_lengths
            .get(&ordinal)
            .copied()
            .unwrap_or(shortest);
        self.relax_branches.push(RelaxBranch {
            ordinal,
            section: self.base.current_section.clone(),
            offset: self.base.current_offset(),
            symbol: symbol.to_string(),
            addend,
            jump,
            rvc: half.is_some(),
            length,
        });
        let symbol = symbol.to_string();
        match (length, half) {
            (2, Some(half)) => {
                let rvc_type = if jump {
                    R_RISCV_RVC_JUMP
                } else {
                    R_RISCV_RVC_BRANCH
                };
                self.base.add_reloc(rvc_type, symbol, addend);
                return self.base.emit_u16_le(half);
            }
            (4, _) => {
                self.base.add_reloc(elf_type, symbol, addend);
                return self.base.emit_u32_le(word);
            }
            // Inverted condition (funct3 bit 0 / c.beqz<->c.bnez), skipping
            // the jump: B-type offset 8, CB-type offset 6.
            (6, Some(half)) => self.base.emit_u16_le((half ^ 0x2000) | 0x18)?,
            (8, _) if !jump => self.base.emit_u32_le((word ^ 0x1000) | 0x400)?,
            _ => return Err(format!("internal: branch length {length} for `{mnemonic}`")),
        }
        self.base.add_reloc(R_RISCV_JAL, symbol, addend);
        self.base.emit_u32_le(JAL_X0)
    }

    /// The 2-byte form of a branch or jump whose spelling has an RVC alias
    /// in GNU as's opcode table, displacement zero.
    fn rvc_branch_form(
        &self,
        word: u32,
        jump: bool,
        mnemonic: &str,
        operand_count: usize,
    ) -> Option<u16> {
        let rd = (word >> 7) & 0x1F;
        let funct3 = (word >> 12) & 0x7;
        let rs1 = (word >> 15) & 0x1F;
        let rs2 = (word >> 20) & 0x1F;
        let mnemonic = mnemonic.to_ascii_lowercase();
        if jump {
            return match (mnemonic.as_str(), operand_count, rd) {
                ("j", 1, 0) => Some(0xA001),
                ("jal", 1, 1) if self.elf_class == ELFCLASS32 => Some(0x2001),
                _ => None,
            };
        }
        let spelled = matches!(
            (mnemonic.as_str(), operand_count),
            ("beqz" | "bnez", 2) | ("beq" | "bne", 3)
        );
        if !spelled || rs2 != 0 || funct3 > 1 || !(8..16).contains(&rs1) {
            return None;
        }
        let base: u16 = if funct3 == 0 { 0xC001 } else { 0xE001 };
        Some(base | (((rs1 - 8) as u16) << 7))
    }

    /// After layout: give every branch the length its target needs (GNU as's
    /// `relaxed_branch_length`), and ask for another pass if one grew.
    fn relax_branch_lengths(&mut self) {
        for branch in std::mem::take(&mut self.relax_branches) {
            let target = self
                .base
                .labels
                .get(&branch.symbol)
                .filter(|(section, _)| *section == branch.section)
                .filter(|_| !self.base.weak_symbols.contains_key(&branch.symbol));
            let needed = match target {
                None if branch.jump => 4,
                None => 8,
                Some((_, offset)) => {
                    let distance = *offset as i64 + branch.addend - branch.offset as i64;
                    let rvc_reach = if branch.jump { 2048 } else { 256 };
                    if branch.rvc && (-rvc_reach..rvc_reach).contains(&distance) {
                        2
                    } else if branch.jump || (-4096..4096).contains(&distance) {
                        4
                    } else if branch.rvc {
                        6
                    } else {
                        8
                    }
                }
            };
            if needed > branch.length {
                self.branch_lengths.insert(branch.ordinal, needed);
                self.branch_retry = true;
            }
        }
    }

    /// Resolve local branch labels to PC-relative offsets using RISC-V relocation types.
    fn resolve_local_branches(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.pending_branch_relocs);
        for reloc in &pending {
            // pcrel_lo12 relocations must always be emitted as external relocations
            // so the linker can pair them with their corresponding pcrel_hi20.
            let is_pcrel_lo = reloc.reloc_type == 24 || reloc.reloc_type == 25;
            if is_pcrel_lo {
                let symbol_name = reloc.symbol.clone();
                if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                    section.relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type: reloc.reloc_type,
                        symbol_name,
                        addend: reloc.addend,
                    });
                    if reloc.relax {
                        section.relocs.push(ObjReloc {
                            offset: reloc.offset,
                            reloc_type: Self::R_RISCV_RELAX,
                            symbol_name: String::new(),
                            addend: 0,
                        });
                    }
                }
                continue;
            }

            let resolved = self.base.labels.get(&reloc.symbol).cloned();

            let (target_section, target_offset) = match resolved {
                Some(v) => v,
                None => {
                    if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                        section.relocs.push(ObjReloc {
                            offset: reloc.offset,
                            reloc_type: reloc.reloc_type,
                            symbol_name: reloc.symbol.clone(),
                            addend: reloc.addend,
                        });
                        if reloc.relax {
                            section.relocs.push(ObjReloc {
                                offset: reloc.offset,
                                reloc_type: Self::R_RISCV_RELAX,
                                symbol_name: String::new(),
                                addend: 0,
                            });
                        }
                    }
                    continue;
                }
            };

            if target_section != reloc.section {
                if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                    section.relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type: reloc.reloc_type,
                        symbol_name: reloc.symbol.clone(),
                        addend: reloc.addend,
                    });
                    if reloc.relax {
                        section.relocs.push(ObjReloc {
                            offset: reloc.offset,
                            reloc_type: Self::R_RISCV_RELAX,
                            symbol_name: String::new(),
                            addend: 0,
                        });
                    }
                }
                continue;
            }

            let ref_offset = reloc.pcrel_hi_offset.unwrap_or(reloc.offset);
            let pc_offset = (target_offset as i64) - (ref_offset as i64) + reloc.addend;

            // GAS parity: validate the PC-relative offset against the
            // encoding's range before patching it in.
            //
            // Each immediate below is split across the instruction's fields
            // with plain shifts and masks, so an offset that does not fit
            // silently WRAPS and the branch lands somewhere else entirely.
            // Because the relocation is resolved right here, neither a
            // diagnostic nor an external relocation survives to reveal it.
            // (Upstream fork issue #121; same defect as the AArch64 writer.)
            let (lo, hi) = match reloc.reloc_type {
                // R_RISCV_BRANCH: B-type, 13-bit signed (bit 0 implicit)
                16 => (-(1i64 << 12), (1i64 << 12)),
                // R_RISCV_JAL: J-type, 21-bit signed (bit 0 implicit)
                17 => (-(1i64 << 20), (1i64 << 20)),
                // R_RISCV_CALL_PLT / R_RISCV_PCREL_HI20: 32-bit via AUIPC+JALR
                19 | 23 => (-(1i64 << 31), (1i64 << 31)),
                _ => (i64::MIN, i64::MAX),
            };
            if !(lo..hi).contains(&pc_offset) {
                return Err(format!(
                    "branch out of range: `{}` at offset {:#x} needs a PC-relative \
                     offset of {:#x}, which is not encodable in relocation type {} \
                     (allowed {:#x}..{:#x}); it would silently branch to the wrong address",
                    reloc.symbol, reloc.offset, pc_offset, reloc.reloc_type, lo, hi
                ));
            }

            if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                let instr_offset = reloc.offset as usize;

                match reloc.reloc_type {
                    16 => {
                        // R_RISCV_BRANCH (B-type, 12-bit)
                        if instr_offset + 4 > section.data.len() {
                            continue;
                        }
                        let mut word = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        let imm = pc_offset as u32;
                        let bit12 = (imm >> 12) & 1;
                        let bit11 = (imm >> 11) & 1;
                        let bits10_5 = (imm >> 5) & 0x3F;
                        let bits4_1 = (imm >> 1) & 0xF;
                        word &= 0x01FFF07F;
                        word |= (bit12 << 31) | (bits10_5 << 25) | (bits4_1 << 8) | (bit11 << 7);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&word.to_le_bytes());
                    }
                    17 => {
                        // R_RISCV_JAL (J-type, 20-bit)
                        if instr_offset + 4 > section.data.len() {
                            continue;
                        }
                        let mut word = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        let imm = pc_offset as u32;
                        let bit20 = (imm >> 20) & 1;
                        let bits10_1 = (imm >> 1) & 0x3FF;
                        let bit11 = (imm >> 11) & 1;
                        let bits19_12 = (imm >> 12) & 0xFF;
                        word &= 0x00000FFF;
                        word |=
                            (bit20 << 31) | (bits10_1 << 21) | (bit11 << 20) | (bits19_12 << 12);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&word.to_le_bytes());
                    }
                    19 => {
                        // R_RISCV_CALL_PLT (AUIPC + JALR pair, 8 bytes)
                        if instr_offset + 8 > section.data.len() {
                            continue;
                        }

                        let hi = ((pc_offset as i32 + 0x800) >> 12) as u32;
                        let lo = ((pc_offset as i32) << 20 >> 20) as u32;

                        let mut auipc = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        auipc = (auipc & 0xFFF) | (hi << 12);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&auipc.to_le_bytes());

                        let mut jalr = u32::from_le_bytes([
                            section.data[instr_offset + 4],
                            section.data[instr_offset + 5],
                            section.data[instr_offset + 6],
                            section.data[instr_offset + 7],
                        ]);
                        jalr = (jalr & 0xFFFFF) | ((lo & 0xFFF) << 20);
                        section.data[instr_offset + 4..instr_offset + 8]
                            .copy_from_slice(&jalr.to_le_bytes());
                    }
                    23 => {
                        // R_RISCV_PCREL_HI20 (AUIPC hi20)
                        if instr_offset + 4 > section.data.len() {
                            continue;
                        }
                        let hi = ((pc_offset as i32 + 0x800) >> 12) as u32;
                        let mut word = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        word = (word & 0xFFF) | (hi << 12);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&word.to_le_bytes());
                    }
                    24 => {
                        // R_RISCV_PCREL_LO12_I (ADDI/LD lo12 I-type)
                        if instr_offset + 4 > section.data.len() {
                            continue;
                        }
                        let lo = (pc_offset as i32) & 0xFFF;
                        let mut word = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        word = (word & 0xFFFFF) | (((lo as u32) & 0xFFF) << 20);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&word.to_le_bytes());
                    }
                    25 => {
                        // R_RISCV_PCREL_LO12_S (SW/SD lo12 S-type)
                        if instr_offset + 4 > section.data.len() {
                            continue;
                        }
                        let lo = (pc_offset as i32) & 0xFFF;
                        let mut word = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        let imm_lo = (lo as u32) & 0x1F;
                        let imm_hi = ((lo as u32) >> 5) & 0x7F;
                        word &= 0x01FFF07F;
                        word |= (imm_hi << 25) | (imm_lo << 7);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&word.to_le_bytes());
                    }
                    _ => {
                        section.relocs.push(ObjReloc {
                            offset: reloc.offset,
                            reloc_type: reloc.reloc_type,
                            symbol_name: reloc.symbol.clone(),
                            addend: reloc.addend,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Write the final ELF object file.
    pub fn write_elf(&mut self, output_path: &str) -> Result<(), String> {
        let config = elf::ElfConfig {
            e_machine: EM_RISCV,
            e_flags: self.elf_flags,
            elf_class: self.elf_class,
            // RISC-V always uses RELA relocations, even in 32-bit mode
            force_rela: true,
        };
        let (address_size, abs_addr) = if self.elf_class == ELFCLASS32 {
            (4, RelocType::Abs32)
        } else {
            (8, RelocType::Abs64)
        };
        self.base.finish_debug_sections(
            address_size,
            RelocType::Abs32.elf_type(),
            abs_addr.elf_type(),
        )?;
        // RISC-V needs include_referenced_locals=true for pcrel_hi synthetic labels
        self.base.write_elf(output_path, &config, true)
    }
}

// =============================================================================
// Branch-displacement range validation (upstream fork issue #121)
// =============================================================================
//
// Deliberately mirrors the AArch64 module of the same name: defect #121 was
// present in BOTH writers, and a fix verified on one target only is a fix
// verified on half the defect.
// =============================================================================
// Branch-displacement validation (upstream fork issue #121, and the bit-0
// truncation found while testing it)
// =============================================================================
//
// Deliberately mirrors the AArch64 module of the same name: defect #121 was
// present in BOTH writers, and a fix verified on one target only is a fix
// verified on half the defect.
//
// Ground truth established by experiment, and worth stating because it is not
// what the code's shape suggests:
//
//   * A branch whose target is a SYMBOL -- local or external, forward or
//     backward -- is always emitted as an `R_RISCV_BRANCH` / `R_RISCV_JAL`
//     relocation. It is never patched inline, because linker relaxation can
//     shrink the code after assembly and the offset is genuinely unknowable
//     here. `resolve_local_branches` therefore never sees reloc types 16/17.
//
//   * A branch whose target is a LITERAL immediate is encoded on the spot, and
//     *that* is where a bad offset can be silently mangled -- which is why the
//     validation lives in `encoder/base.rs`, not in the relocation resolver.
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

    /// P1 bare-input + instruction-raise law — same creation law as
    /// aarch64/GAS (sections start at sh_addralign 1, instructions raise
    /// to the target alignment: `ElfWriterBase::new(RISCV_NOP, 2)` so an
    /// instruction raises to 2; data-only input stays 1).
    #[test]
    fn gas247_bare_input_default_section_and_align_law() {
        let w = assemble(".byte 1\n").unwrap();
        assert_eq!(w.base.sections[".text"].data, vec![1]);
        assert_eq!(w.base.sections[".text"].sh_addralign, 1);

        let w = assemble(".zero 4\n").unwrap();
        assert_eq!(w.base.sections[".text"].sh_addralign, 1);

        let w = assemble(".p2align 4\n").unwrap();
        assert_eq!(w.base.sections[".text"].sh_addralign, 16);

        let w = assemble(".text\nnop\n").unwrap();
        assert_eq!(w.base.sections[".text"].sh_addralign, 2);
    }

    /// Same GAS 2.47 NOBITS rows as the x86/arm suites — riscv shares
    /// `ElfWriterBase`'s guards plus its own raw-value pre-truncation
    /// checks in the Byte/Short arms (`.short 256` stores zeros yet is
    /// still rejected).  No budget row: riscv charges through the exact
    /// same `ElfWriterBase::charge_fill` the ARM budget tests exercise.
    #[test]
    fn gas247_nobits_store_and_position_laws() {
        for (src, needle) in [
            (".bss\n.byte 1\n", "non-zero value"),
            (".bss\n.short 256\n", "non-zero value"),
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
}

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

    /// `ElfWriter` is not `Debug`, so `expect_err` is unavailable; match instead.
    fn expect_err(asm: &str, what: &str) -> String {
        match assemble(asm) {
            Ok(_) => panic!("{what}"),
            Err(e) => e,
        }
    }

    /// Bit 0 of a B-type displacement is implicit (`imm[4:1]` only), so an odd
    /// literal offset used to be silently truncated -- `beq x1, x2, 17`
    /// encoded to the identical word `0x00208863` as `beq x1, x2, 16`.
    #[test]
    fn odd_literal_branch_offset_is_diagnosed_not_truncated() {
        let err = expect_err(
            ".text\nbeq x1, x2, 17\n",
            "an odd literal branch offset must be diagnosed, not truncated",
        );
        assert!(
            err.contains("misaligned branch offset"),
            "expected a misalignment diagnostic, got: {err}"
        );
    }

    /// Same defect, J-type (`imm[10:1]` only): `jal x1, 4097` and
    /// `jal x1, 4096` both encoded to `0x000010ef`.
    #[test]
    fn odd_literal_jal_offset_is_diagnosed_not_truncated() {
        let err = expect_err(
            ".text\njal x1, 4097\n",
            "an odd literal jal offset must be diagnosed, not truncated",
        );
        assert!(
            err.contains("misaligned branch offset"),
            "expected a misalignment diagnostic, got: {err}"
        );
    }

    /// The even neighbours must still encode, and must encode to *different*
    /// words -- a check that only "something was rejected" would also pass for
    /// an encoder that rounded everything to zero.
    #[test]
    fn even_literal_offsets_encode_distinctly() {
        let mut words = vec![];
        for imm in [0i64, 16, 4094] {
            let asm = format!(".text\nbeq x1, x2, {imm}\n");
            let w =
                assemble(&asm).unwrap_or_else(|e| panic!("beq x1, x2, {imm} should encode: {e}"));
            let data = w.base.sections[".text"].data.clone();
            assert_eq!(
                data.len(),
                4,
                "one branch is 4 bytes (no compression in process_statements)"
            );
            words.push(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
        }
        assert_eq!(words.len(), 3);
        assert!(
            words[0] != words[1] && words[1] != words[2],
            "distinct even offsets must give distinct words, got {words:#x?}"
        );
    }

    /// B-type holds a 13-bit signed displacement, so 8192 is past the end.
    #[test]
    fn out_of_range_literal_branch_offset_is_diagnosed() {
        let err = expect_err(
            ".text\nbeq x1, x2, 8192\n",
            "a literal branch offset outside +-4 KiB must be diagnosed",
        );
        assert!(
            err.contains("branch out of range"),
            "expected a range diagnostic, got: {err}"
        );
    }

    /// J-type holds a 21-bit signed displacement.
    #[test]
    fn out_of_range_literal_jal_offset_is_diagnosed() {
        let err = expect_err(
            ".text\njal x1, 2097152\n",
            "a literal jal offset outside +-1 MiB must be diagnosed",
        );
        assert!(
            err.contains("branch out of range"),
            "expected a range diagnostic, got: {err}"
        );
    }

    /// Lock in the deliberate design: symbol targets stay as relocations for
    /// the linker, because relaxation may still shrink the code. If this ever
    /// starts resolving inline, branch distances silently stop being relaxed.
    #[test]
    fn branch_to_symbol_stays_a_relocation_for_the_linker() {
        for (asm, sym) in [
            (".text\nbeq x1, x2, .Lfwd\nnop\n.Lfwd:\nnop\n", ".Lfwd"),
            (".text\n.Lb:\nnop\nbeq x1, x2, .Lb\n", ".Lb"),
            (".text\nbeq x1, x2, extern_sym\n", "extern_sym"),
        ] {
            let w = assemble(asm).unwrap_or_else(|e| panic!("`{asm}` should assemble: {e}"));
            let relocs = &w.base.sections[".text"].relocs;
            assert_eq!(
                relocs.len(),
                1,
                "branch to `{sym}` must be left as exactly one relocation for the linker, got {}",
                relocs.len()
            );
            assert_eq!(relocs[0].symbol_name, sym);
        }
    }

    /// The implicit-`rd` form (`jal offset`) must be validated too, not only
    /// the two-operand form -- they are separate code paths in `encode_jal`.
    #[test]
    fn implicit_rd_jal_form_is_validated_too() {
        expect_err(
            ".text\njal 4097\n",
            "the one-operand jal form must validate its offset as well",
        );
        assemble(".text\njal 4096\n").expect("even, in-range offset must encode");
    }

    // ---- boundary matrices -------------------------------------------------

    /// B-type holds a signed 13-bit displacement with bit 0 implicit, so the
    /// encodable range is -4096 .. +4094 in steps of 2. This pins BOTH ends,
    /// including the negative one -- a range check written with an unsigned
    /// comparison or a `<=` instead of `<` passes every positive-only test.
    #[test]
    fn b_type_range_boundaries_are_exact() {
        for imm in [-4096i64, -4094, -2, 0, 2, 4094] {
            let asm = format!(".text\nbeq x1, x2, {imm}\n");
            assemble(&asm).unwrap_or_else(|e| panic!("beq x1, x2, {imm} must encode: {e}"));
        }
        for imm in [-4098i64, 4096] {
            let asm = format!(".text\nbeq x1, x2, {imm}\n");
            let err = expect_err(&asm, &format!("beq x1, x2, {imm} must be rejected"));
            assert!(
                err.contains("branch out of range"),
                "imm={imm}: expected a range diagnostic, got {err}"
            );
        }
    }

    /// J-type holds a signed 21-bit displacement: -1048576 .. +1048574.
    #[test]
    fn j_type_range_boundaries_are_exact() {
        for imm in [-1048576i64, -2, 0, 2, 1048574] {
            let asm = format!(".text\njal x1, {imm}\n");
            assemble(&asm).unwrap_or_else(|e| panic!("jal x1, {imm} must encode: {e}"));
        }
        for imm in [-1048578i64, 1048576] {
            let asm = format!(".text\njal x1, {imm}\n");
            let err = expect_err(&asm, &format!("jal x1, {imm} must be rejected"));
            assert!(
                err.contains("branch out of range"),
                "imm={imm}: expected a range diagnostic, got {err}"
            );
        }
    }

    /// Bit 0 is dropped by BOTH B- and J-type layouts, for positive and
    /// negative offsets alike, on every literal path: `beq`, `jal rd, imm`,
    /// `jal imm`, and the `j` pseudo-instruction (which is `jal x0, imm`).
    #[test]
    fn odd_offsets_rejected_on_every_literal_path() {
        for imm in [-4095i64, -3, -1, 1, 3, 17, 4095] {
            for asm in [
                format!(".text\nbeq x1, x2, {imm}\n"),
                format!(".text\njal x1, {imm}\n"),
                format!(".text\njal {imm}\n"),
                format!(".text\nj {imm}\n"),
            ] {
                let err = expect_err(
                    &asm,
                    &format!("odd offset {imm} in `{asm}` must be diagnosed"),
                );
                assert!(
                    err.contains("misaligned branch offset"),
                    "imm={imm} in `{asm}`: expected a misalignment diagnostic, got {err}"
                );
            }
        }
    }

    /// `j` encodes to `jal x0, offset` and must share the J-type range exactly.
    #[test]
    fn j_pseudo_shares_the_j_type_range() {
        for imm in [-1048576i64, -2, 0, 2, 1048574] {
            let asm = format!(".text\nj {imm}\n");
            assemble(&asm).unwrap_or_else(|e| panic!("j {imm} must encode: {e}"));
        }
        for imm in [-1048578i64, 1048576] {
            let asm = format!(".text\nj {imm}\n");
            expect_err(&asm, &format!("j {imm} must be rejected"));
        }
        // ...and it really is `jal x0, offset`: rd must be x0.
        let w = assemble(".text\nj 16\n").expect("j 16 must encode");
        assert_eq!(
            w.base.sections[".text"].data.len(),
            4,
            "`j` is a single instruction"
        );
    }

    /// Symbolic targets must keep their relocation type and addend so the
    /// linker can resolve and relax them. Branch is 16 (R_RISCV_BRANCH) and
    /// Jal is 17 (R_RISCV_JAL); the `j` pseudo maps to Jal.
    #[test]
    fn symbolic_targets_keep_relocation_type_and_addend() {
        const R_RISCV_BRANCH: u32 = 16;
        const R_RISCV_JAL: u32 = 17;

        // GNU as: a conditional branch to an undefined symbol is the inverted
        // branch over `jal x0, target`, whose one relocation is the JAL at +4;
        // in reach of a defined label it is the 4-byte branch.
        for (asm, sym, want_type, want_offset) in [
            (".text\nbeq x1, x2, target\n", "target", R_RISCV_JAL, 4),
            (".text\nbne x1, x2, target\n", "target", R_RISCV_JAL, 4),
            (
                ".text\nbeq x1, x2, target\ntarget:\n",
                "target",
                R_RISCV_BRANCH,
                0,
            ),
            (".text\njal x1, target\n", "target", R_RISCV_JAL, 0),
            (".text\njal target\n", "target", R_RISCV_JAL, 0),
            (".text\nj target\n", "target", R_RISCV_JAL, 0),
        ] {
            let w = assemble(asm).unwrap_or_else(|e| panic!("`{asm}` must assemble: {e}"));
            let relocs = &w.base.sections[".text"].relocs;
            assert_eq!(
                relocs.len(),
                1,
                "`{asm}` must leave exactly one relocation, got {}",
                relocs.len()
            );
            assert_eq!(relocs[0].symbol_name, sym, "wrong symbol for `{asm}`");
            assert_eq!(
                relocs[0].reloc_type, want_type,
                "wrong relocation type for `{asm}`: expected {want_type}, got {}",
                relocs[0].reloc_type
            );
            assert_eq!(relocs[0].addend, 0, "wrong addend for `{asm}`");
            assert_eq!(relocs[0].offset, want_offset, "wrong offset for `{asm}`");
        }
    }

    #[test]
    fn symbol_difference_trailing_minus_applies_to_the_difference() {
        // Both labels are in `.data`, so the difference is a constant (GAS
        // folds it too); `A-B-4` is (A-B)-4 = -8, never A-(B-4) = 0.
        let w = assemble(".data\nA:\n.byte 0,0,0,0\nB:\n.byte 0,0,0,0\n.long A-B-4\n")
            .expect("symbol difference must assemble");
        let data = &w.base.sections[".data"];
        assert!(
            data.relocs.is_empty(),
            "a same-section difference is a constant"
        );
        assert_eq!(
            data.data[8..12],
            (-8i32).to_le_bytes(),
            "the trailing -4 applies to A-B"
        );
    }

    /// A surplus operand used to be discarded silently, so a malformed branch
    /// assembled as a valid one. The three literal-bearing forms now bound
    /// their operand counts.
    #[test]
    fn surplus_operands_are_rejected_not_ignored() {
        for asm in [
            ".text\nbeq x1, x2, 16, x3\n",
            ".text\njal x1, 16, x2\n",
            ".text\nj 16, x1\n",
            ".text\nj\n",
        ] {
            let err = expect_err(asm, &format!("`{asm}` must be rejected"));
            assert!(
                !err.is_empty(),
                "`{asm}` was rejected but without a diagnostic"
            );
        }
    }
}

#[cfg(test)]
mod padding_law_tests {
    use super::super::parser::parse_asm;
    use super::*;

    fn text(asm: &str) -> Vec<u8> {
        let statements = parse_asm(asm).expect("must parse");
        let mut w = ElfWriter::new();
        w.process_statements(&statements).expect("must assemble");
        w.base.sections[".text"].data.clone()
    }

    /// Same, for a named section: the cap law is section-conditional, so a
    /// `.data` pin cannot go through a helper hard-wired to `.text`.
    fn section(name: &str, asm: &str) -> Vec<u8> {
        let statements = parse_asm(asm).expect("must parse");
        let mut w = ElfWriter::new();
        w.process_statements(&statements).expect("must assemble");
        w.base.sections[name].data.clone()
    }

    const NOP: [u8; 4] = [0x13, 0x00, 0x00, 0x00];
    const C_NOP: [u8; 2] = [0x01, 0x00];
    const DATA: [u8; 5] = [0xa5; 5];

    /// The pad content is GNU as 2.47's `riscv_make_nops` (tc-riscv.c:5410),
    /// measured on the oracle (`-march=rv64gc`): one zero byte at an odd
    /// offset, then one `c.nop` where the position is 2 mod 4, and 4-byte NOPs
    /// for the rest -- every NOP sits where a hart could fetch it.  Before this
    /// law was shared with aarch64 the writer emitted NOPs first and zeros
    /// last, putting a 4-byte NOP at an odd offset.  (In a relaxable code
    /// section the run is the worst case `align - 2` after the step to the
    /// instruction alignment, see `relaxable_pad_is_the_worst_case_run`.)
    #[test]
    fn pad_is_zero_prefix_then_compressed_nop_then_nops() {
        // Offset 5, `.p2align 3`: 00 | c.nop | nop  (as 2.47: 00 01 00 13 00 00 00).
        assert_eq!(
            text(".byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.p2align 3\n"),
            [DATA.as_slice(), &[0x00], &C_NOP, &NOP].concat()
        );
        // Offset 5, `.p2align 4`: 00 | c.nop | nop | nop | nop.
        assert_eq!(
            text(".byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.p2align 4\n"),
            [DATA.as_slice(), &[0x00], &C_NOP, &NOP, &NOP, &NOP].concat()
        );
        // Without relaxation the pad is final and stops at the boundary.
        assert_eq!(
            text(
                ".option norelax\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.p2align 4\n"
            ),
            [DATA.as_slice(), &[0x00], &C_NOP, &NOP, &NOP].concat()
        );
    }

    /// The `.text` relocations of `asm`, as `(offset, type, addend)`.
    fn text_relocs(asm: &str) -> Vec<(u64, u32, i64)> {
        let statements = parse_asm(asm).expect("must parse");
        let mut w = ElfWriter::new();
        w.process_statements(&statements).expect("must assemble");
        w.base.sections[".text"]
            .relocs
            .iter()
            .map(|r| (r.offset, r.reloc_type, r.addend))
            .collect()
    }

    /// In a relaxable code section the pad is GNU as's `riscv_frag_align_code`:
    /// a fixed step to the instruction alignment, then the WORST case `align -
    /// insn_alignment` bytes of NOPs under an `R_RISCV_ALIGN` naming that
    /// length, which the linker trims to what the final address needs (GNU ld,
    /// lld, and lccc-ld's `riscv/linker/align.rs`).  Every row is GNU as
    /// 2.47.20260726 `-march=rv64gc` (lccc's objects declare `EF_RISCV_RVC`).
    /// This backend used to pad only to the boundary with the pad length as the
    /// addend: a short pad then named too small an alignment (2 bytes of a
    /// `.p2align 4` read as "align to 4"), and code shrinking before it left too
    /// few bytes -- GNU ld refused such objects outright ("12 bytes required for
    /// alignment to 16-byte boundary, but only 8 present").
    #[test]
    fn relaxable_pad_is_the_worst_case_run() {
        const ALIGN: u32 = 43;
        let five = ".byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n";
        // Offset 5, `.p2align 5`: 00 (step to 6), then 30 bytes = c.nop + 7 NOPs.
        let got = text(&format!("{five}.p2align 5\n"));
        assert_eq!(got.len(), 5 + 1 + 30);
        assert_eq!(&got[5..8], &[0x00, 0x01, 0x00][..]);
        assert!(got[8..].chunks_exact(4).all(|c| c == NOP));
        assert_eq!(
            text_relocs(&format!("{five}.p2align 5\n")),
            [(6, ALIGN, 30)]
        );
        // `.p2align 2` is above the 2-byte RVC step, so it is relaxable too.
        assert_eq!(text(&format!("{five}.p2align 2\n")).len(), 8);
        assert_eq!(text_relocs(&format!("{five}.p2align 2\n")), [(6, ALIGN, 2)]);
        // `.option norvc`: the step is 4 (00 c.nop), the run 4 bytes.
        let norvc = format!(".option norvc\n{five}.p2align 3\n");
        assert_eq!(
            text(&norvc),
            [DATA.as_slice(), &[0x00], &C_NOP, &NOP].concat()
        );
        assert_eq!(text_relocs(&norvc), [(8, ALIGN, 4)]);
        // ... and `.p2align 2` is no longer above the step: final pad, no reloc.
        let norvc2 = format!(".option norvc\n{five}.p2align 2\n");
        assert_eq!(text(&norvc2).len(), 8);
        assert_eq!(text_relocs(&norvc2), []);
        // `.option norelax`: final pad to the boundary, no relocation.
        let norelax = format!(".option norelax\n{five}.p2align 5\n");
        assert_eq!(text(&norelax).len(), 32);
        assert_eq!(text_relocs(&norelax), []);
        // An object assembled for a march without C (no `EF_RISCV_RVC`) steps
        // to 4 from the start, like `.option norvc`.
        let statements = parse_asm(&format!("{five}.p2align 3\n")).expect("must parse");
        let mut w = ElfWriter::new();
        w.set_elf_flags(EF_RISCV_FLOAT_ABI_DOUBLE);
        w.process_statements(&statements).expect("must assemble");
        let relocs: Vec<_> = w.base.sections[".text"]
            .relocs
            .iter()
            .map(|r| (r.offset, r.reloc_type, r.addend))
            .collect();
        assert_eq!(relocs, [(8, ALIGN, 4)]);
    }

    /// The `fill` operand was parsed and thrown away by this backend, so
    /// `.p2align 4, 0xff` assembled as NOPs instead of the asked-for byte,
    /// moving every address after the directive once the fill made the run a
    /// different length in a data section.  (`.p2align 4,,3` in `.text` is *not*
    /// such a case: the RISC-V code hook ignores the cap, so padding is correct
    /// there -- see `cap_applies_outside_a_code_section`.)
    /// away from the oracle's. The fill now reaches the writer (measured on
    /// as 2.47: an explicit fill byte replaces the NOP vocabulary completely,
    /// in `.text` as in `.data`).
    #[test]
    fn explicit_fill_byte_is_honoured_in_text() {
        let got =
            text(".byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.p2align 4, 0xff\n");
        assert_eq!(got.len(), 16, "same size as the NOP fill");
        assert!(
            got[5..].iter().all(|&b| b == 0xff),
            "11 bytes of 0xff, no NOPs: {got:02x?}"
        );
        // `.byte 1; .p2align 1, 0xff` is `01 ff` on GNU as 2.47 -march=rv64gc:
        // the directive pads to what it asks for, and the code floor only
        // raises the section's alignment.  (`ff 01 00`, read once as the law,
        // is a march without C, where the tail GNU as pads a code section with
        // follows the 4-byte alignment.)
        let got = text(".byte 0xa5\n.p2align 1, 0xff\n");
        assert_eq!(got, [0xa5, 0xff], "one fill byte to the 2-byte boundary");
        // `.balign` shares the grammar, including the cap slot -- which is live
        // outside a code section (see `cap_applies_outside_a_code_section`; in
        // `.text` the hook ignores it, so `.balign 8,,2` still pads).
        let got = text(".byte 0xa5\n.byte 0xa5\n.byte 0xa5\n.balign 8, 0xff\n");
        assert_eq!(got, [vec![0xa5; 3], vec![0xff; 5]].concat());
    }

    /// RISC-V has no padding cap: `riscv_frag_align_code (int n)` receives the
    /// alignment alone, and the generic fall-back it shares with `.balign`
    /// never reads the operand. Measured identical for every cap in
    /// {0,1,2,3,4,5,8,11,12,20,40,2048,2049} — but only because the default
    /// section is code and no fill was given, which is the one combination where
    /// GAS's hook declines to look at the cap. (aarch64 is the target where the
    /// cap skips everywhere: see the ARM writer's tests.)
    #[test]
    fn padding_cap_is_validated_then_dropped_as_on_gas() {
        let plain = text(".byte 0xa5\n.p2align 4\n");
        let capped = text(".byte 0xa5\n.p2align 4,, 3\n");
        assert_eq!(
            plain, capped,
            "a cap GAS ignores must not change LCCC either"
        );
        assert_eq!(plain.len(), 16);
        // A cap operand that is not an integer is still a syntax error, and so
        // is a fourth operand — the tail grammar is the shared one.
        assert!(parse_asm(".byte 0xa5\n.p2align 4, junk\n").is_err());
        assert!(parse_asm(".byte 0xa5\n.p2align 4,, 2, 9\n").is_err());
        assert!(
            parse_asm(".byte 0xa5\n.balign 8,, junk\n").is_err(),
            "GAS parses .balign's cap slot with get_absolute_expression() before
             any section logic runs, so a non-integer one is an assembler error"
        );
    }

    /// The half of the law a parser test cannot see, and the reason the cap must
    /// survive parsing: which operand applies depends on the section the
    /// directive sits in.  Every row is the byte string GNU as 2.47.20260726
    /// (`riscv64-linux-gnu-as`, the prefix `scripts/ensure_gas_247.sh`
    /// provisions) put in the section, read back with `objdump -s`; the last
    /// three are the rows where a backend that ignored the cap would disagree
    /// with GAS in both directions: it would pad `.data` where GAS does not, and
    /// it would have no way to express `.text` padding that a cap cancels.
    #[test]
    fn cap_applies_outside_a_code_section() {
        // `.text`, no fill operand: riscv_frag_align_code(n) never sees `max`.
        let plain = text(".text\n.byte 1\n.p2align 4\n.byte 2\n");
        for cap in ["2", "0", "14", "99"] {
            assert_eq!(
                text(&format!(".text\n.byte 1\n.p2align 4,,{cap}\n.byte 2\n")),
                plain,
                "a cap must not change `.text` padding (max={cap})"
            );
        }
        assert_eq!(plain.len(), 17, "01 then 15 pad bytes then 02");
        // `.data`: the generic frag_align, cap in BYTES against the pad needed.
        let d = |align: &str| {
            section(
                ".data",
                &format!(".data\n.byte 1\n.p2align {align}\n.byte 2\n"),
            )
        };
        assert_eq!(
            d("4,,2"),
            vec![0x01, 0x02],
            "15 > 2: the directive is skipped"
        );
        assert_eq!(d("4,,14"), vec![0x01, 0x02], "15 > 14: skipped");
        assert_eq!(
            d("4,,15"),
            [vec![0x01], vec![0x00; 15], vec![0x02]].concat(),
            "15 <= 15: pad, and in `.data` the default fill is zero, not NOP"
        );
        assert_eq!(
            d("4,,0"),
            [vec![0x01], vec![0x00; 15], vec![0x02]].concat(),
            "`0` means unlimited, exactly as GAS's `if (max && ...)` reads"
        );
        // A fill byte routes `.text` around the hook, so the cap is live there.
        assert_eq!(
            text(".text\n.byte 1\n.p2align 4,0x13,2\n.byte 2\n"),
            vec![0x01, 0x02],
            "fill given -> generic frag_align -> 15 > 2 -> skipped"
        );
        assert_eq!(
            text(".text\n.byte 1\n.p2align 4,0x13,99\n.byte 2\n"),
            [vec![0x01], vec![0x13; 15], vec![0x02]].concat(),
            "and padded with the asked-for byte when the cap allows it"
        );
        // `.balign` obeys the same rule in both directions.
        assert_eq!(
            section(".data", ".data\n.byte 1\n.balign 8,,4\n.byte 2\n"),
            vec![0x01, 0x02],
            "7 > 4 in `.data`: skipped, where `.text` pads"
        );
        assert_eq!(
            text(".text\n.byte 1\n.balign 8,,4\n.byte 2\n").len(),
            9,
            "the same cap in `.text` pads 7 bytes"
        );
    }

    /// The whole cap law in one table, because each of its four conditions is a
    /// separate way to get this wrong and each row is a *measurement*, not a
    /// re-derivation: `pad` is what GNU as put in the section (as 2.44 locally and
    /// as 2.47.20260726 for the `.p2align 4,0x13,*`, `.balign 8,,4`, `.data` and
    /// `.p2align 4,,2` rows -- see the comment on `hook_drops_cap`), read back from
    /// the bytes between two anchor `.byte` directives.
    ///
    /// `.p2align 1` is swept over every cap on purpose. Padding to 2 bytes at an
    /// odd offset is 1 byte, and the cap vocabulary is `0 == unlimited` or a
    /// positive count, so *no* cap can cancel it: the `n != 0` condition of the
    /// hook is unobservable at `n == 1`, and `n == 0` pads nothing. That is worth
    /// pinning, because the tempting "completeness" fix is to add a conjunct that
    /// can never change an answer -- and a reader then has to work out, again,
    /// which of the conjuncts are load-bearing.
    #[test]
    fn riscv_align_cap_matrix_matches_the_hook() {
        // section, pre-directive options, directive, expected pad, expected fill
        let cases: &[(&str, &str, &str, usize, Option<u8>)] = &[
            // The hook drops the cap: code section, no fill, exponent above the
            // instruction width, relaxation on.
            (".text", "", ".p2align 4,,2", 15, None),
            (".text", "", ".p2align 4,,99", 15, None),
            (".text", "", ".p2align 3,,1", 7, None),
            (".text", "", ".p2align 3,,7", 7, None),
            (".text", "", ".balign 8,,1", 7, None),
            // With RVC (the default: these objects declare `EF_RISCV_RVC`) 4
            // bytes are above `insn_alignment`, so the hook drops the cap here
            // too: the 1-byte step plus a 2-byte run (as 2.47 -march=rv64gc).
            (".text", "", ".p2align 2,,1", 3, None),
            (".text", "", ".p2align 2,,3", 3, None),
            (".text", "", ".balign 4,,1", 3, None),
            (".text", "", ".balign 4,,3", 3, None),
            // `bytes <= insn_alignment` without RVC: the hook declines, so the
            // cap is honoured -- the row a cap-ignoring backend gets backwards.
            (
                ".text",
                ".option push\n.option norvc\n",
                ".p2align 2,,1",
                0,
                None,
            ),
            (
                ".text",
                ".option push\n.option norvc\n",
                ".balign 4,,1",
                0,
                None,
            ),
            (
                ".text",
                ".option push\n.option norvc\n",
                ".p2align 2,,3",
                3,
                None,
            ),
            (
                ".text",
                ".option push\n.option norvc\n",
                ".p2align 3,,1",
                7,
                None,
            ),
            // `!relax`: same section, same exponent, cap honoured again.
            (
                ".text",
                ".option push\n.option norelax\n",
                ".p2align 4,,2",
                0,
                None,
            ),
            (
                ".text",
                ".option push\n.option norelax\n",
                ".p2align 4,,15",
                15,
                None,
            ),
            (
                ".text",
                ".option push\n.option norelax\n",
                ".p2align 3,,3",
                0,
                None,
            ),
            // The cap boundary itself: `pad > max` skips, `pad == max` pads, and
            // that is why the row above is `,,3` and not `,,7` -- at `,,7` both
            // readings of the law agree, so it pins the comparison but not the
            // hook.  Measured: 7 with relaxation on *and* off.
            (
                ".text",
                ".option push\n.option norelax\n",
                ".p2align 3,,7",
                7,
                None,
            ),
            // A non-positive explicit cap means *no cap*, not "never pad". Measured
            // on riscv64-linux-gnu-as at offset 1 in `.text`: `.p2align 4,0x13,0`
            // and `.p2align 4,0x13,-1` both pad the full 15 bytes, while `,2` pads
            // none. `s_align`/`frag_align` consult `max` only when it is strictly
            // positive, so the `v <= 0` arm in `parse_align_tail` is the law and
            // not a dropped value -- reading it as "suppress the alignment" is the
            // more tempting and the wrong half of the divergence.
            (".text", "", ".p2align 4,0x13,0", 15, Some(0x13)),
            (".text", "", ".p2align 4,0x13,-1", 15, Some(0x13)),
            (".text", "", ".p2align 4,,0", 15, None),
            (".data", "", ".p2align 4,0x13,0", 15, Some(0x13)),
            // An explicit fill operand routes around the hook, whatever the section.
            (".text", "", ".p2align 4,0x13,2", 0, Some(0x13)),
            (".text", "", ".p2align 4,0x13,99", 15, Some(0x13)),
            (".text", "", ".p2align 2,0x13,1", 0, Some(0x13)),
            // Non-code sections never reach the hook.
            (".data", "", ".p2align 4,,2", 0, None),
            (".data", "", ".p2align 4,,15", 15, Some(0)),
            (".data", "", ".p2align 2,,1", 0, None),
            (".data", "", ".balign 8,,4", 0, None),
            (".data", "", ".p2align 4,0x13,99", 15, Some(0x13)),
            (
                ".data",
                ".option push\n.option norelax\n",
                ".p2align 4,,15",
                15,
                Some(0),
            ),
        ];
        for (sec, pre, dir, pad, fill) in cases {
            let post = if pre.is_empty() {
                String::new()
            } else {
                ".option pop\n".to_string()
            };
            let src = format!("{sec}\n.byte 1\n{pre}{dir}\n.byte 2\n{post}");
            let data = if *sec == ".text" {
                text(&src)
            } else {
                section(sec, &src)
            };
            assert_eq!(
                data.len(),
                2 + pad,
                "{src:?}: expected {pad} pad bytes, got {:?}",
                &data[1..data.len().saturating_sub(1)]
            );
            assert_eq!(
                data.last().copied(),
                Some(2),
                "{src:?}: the anchor after the directive must survive"
            );
            if let Some(byte) = fill {
                assert!(
                    data[1..1 + pad].iter().all(|&b| b == *byte),
                    "{src:?}: {pad} pad bytes must all be {byte:#04x}, got {:02x?}",
                    &data[1..1 + pad]
                );
            }
        }
        // The unobservable equivalence class, stated as an assertion: at `n == 1`
        // the most padding there can ever be is 1 byte, the cap vocabulary is
        // `0 == unlimited` or a positive count, so no cap can cancel it.  Swept in
        // `.data` rather than `.text` because a `.byte`-only text section carries a
        // separate open question (the mapping-state pre-align that GAS runs before
        // `do_align`, which this backend does not emit yet -- see the pending
        // creation-alignment law in `gas247_bare_input_default_section_and_align_law`),
        // and a matrix row must not silently depend on an unsettled neighbouring law.
        for cap in ["0", "1", "2", "3", "15"] {
            let src = format!(".data\n.byte 1\n.p2align 1,,{cap}\n.byte 2\n");
            assert_eq!(
                section(".data", &src).len(),
                3,
                "`{src}`: 1 byte of pad, and no cap can change that"
            );
        }
        // `.p2align 0` pads nothing at all, which is the hook's `n != 0` condition
        // showing up as a fact rather than as a clause in the predicate.
        assert_eq!(
            section(".data", ".data\n.byte 1\n.p2align 0,,1\n.byte 2\n").len(),
            2
        );
    }
    fn alternative(orig: usize, repl: usize) -> String {
        let words = |n: usize, w: &str| vec![format!("\t.word {w}\n"); n].concat();
        format!(
            ".text\n661:\n{}662:\n.pushsection .altinstr_replacement,\"a\"\n663:\n{}664:\n.popsection\n\
             .org . - (664b-663b) + (662b-661b)\n.org . - (662b-661b) + (664b-663b)\n.word 0x33333333\n",
            words(orig, "0x11111111"),
            words(repl, "0x22222222")
        )
    }

    fn assemble(asm: &str) -> Result<ElfWriter, String> {
        let statements = parse_asm(asm)?;
        let mut w = ElfWriter::new();
        w.process_statements(&statements)?;
        Ok(w)
    }

    /// `.org` used to be dropped on this target: `.byte 1; .org 8; .byte 2`
    /// assembled to `01 02` instead of GAS's `01 00*7 02`, and the kernel's
    /// ALTERNATIVE length check (`alternative-macros.h`) could never fail.
    #[test]
    fn org_is_honoured_not_dropped() {
        assert_eq!(
            section(".data", ".data\n.byte 1\n.org 8\n.byte 2\n"),
            [1, 0, 0, 0, 0, 0, 0, 0, 2]
        );
        assert_eq!(
            section(".data", ".data\n.byte 1\n.org . + 2, 9\n.byte 2\n"),
            [1, 9, 9, 2]
        );
        let w = assemble(&alternative(2, 2)).expect("equal lengths assemble");
        assert_eq!(w.base.sections[".text"].data.len(), 12);
        assert!(assemble(&alternative(3, 1)).is_err(), "shorter replacement");
        assert!(assemble(&alternative(1, 3)).is_err(), "longer replacement");
        assert!(assemble(".data\n.quad 1\n.org 4\n").is_err(), "backwards");
    }

    /// Gigabyte NOBITS runs (`.zero`, `.skip` with a non-zero fill, `.fill`,
    /// alignment, `.org`) are a size: no bytes, no ceiling, no budget.
    #[test]
    fn sparse_nobits_is_a_size() {
        let w = assemble(
            ".bss\n.byte 0\n.zero 0x40000000\n.skip 0x40000000, 1\n.fill 0x40000000, 4, 0\n\
             .p2align 30\n.org 0x1c0000000 + 0x40000000\n.long 0\n",
        )
        .expect("sparse NOBITS assembles");
        let bss = &w.base.sections[".bss"];
        assert!(bss.data.is_empty(), "NOBITS holds no bytes");
        assert_eq!(bss.size(), 0x2_0000_0004);
        assert!(
            assemble(".bss\n.fill 0x40000000, 1, 1\n").is_err(),
            "non-zero fill"
        );
        assert!(
            assemble(".data\n.zero 0x10000001\n").is_err(),
            "PROGBITS ceiling"
        );
    }
}

/// Relocation kinds and R_RISCV_RELAX pairing of the TLS / GOT / low-12
/// forms, row for row against GNU as 2.47 (`riscv64-linux-gnu-as`, read back
/// with `objdump -dr`, 2026-10-19).
#[cfg(test)]
mod reloc_kind_tests {
    use super::super::parser::parse_asm;
    use super::*;

    /// (offset, type, symbol) of every `.text` relocation, RELAX as type 51.
    fn relocs(asm: &str) -> Vec<(u64, u32, String)> {
        let statements = parse_asm(asm).expect("must parse");
        let mut w = ElfWriter::new();
        w.process_statements(&statements).expect("must assemble");
        let mut r: Vec<_> = w.base.sections[".text"]
            .relocs
            .iter()
            .map(|r| (r.offset, r.reloc_type, r.symbol_name.clone()))
            .collect();
        r.sort_by_key(|x| (x.0, x.1 == 51));
        r
    }
    fn types(asm: &str) -> Vec<(u64, u32)> {
        relocs(asm).into_iter().map(|(o, t, _)| (o, t)).collect()
    }

    /// Stores take the S-type low-12 relocations. An I-type one patches
    /// bits 31:20, which in a store hold `rs2`: the linker would rewrite the
    /// stored register field with address bits.
    #[test]
    fn stores_take_s_type_low12() {
        assert_eq!(
            types("sw a1, %lo(g)(a0)\nfsd fa0, %lo(g)(a0)\nsw a1, %tprel_lo(v)(t0)\n"),
            vec![(0, 28), (0, 51), (4, 28), (4, 51), (8, 31), (8, 51)]
        );
        assert_eq!(
            types("lw a1, %lo(g)(a0)\nfld fa0, %tprel_lo(v)(a0)\n"),
            vec![(0, 27), (0, 51), (4, 30), (4, 51)]
        );
    }

    /// The Local-Exec sequence: HI20, TPREL_ADD on the `add`, LO12, each with
    /// RELAX, so the linker can shrink it to `addi rd, tp, off`.
    #[test]
    fn local_exec_sequence_is_relaxable() {
        assert_eq!(
            types(
                "lui t0, %tprel_hi(v)\nadd t0, t0, tp, %tprel_add(v)\naddi t0, t0, %tprel_lo(v)\n"
            ),
            vec![(0, 29), (0, 51), (4, 32), (4, 51), (8, 30), (8, 51)]
        );
        assert!(
            parse_asm("add t0, t0, tp, t1\n")
                .ok()
                .and_then(|s| {
                    let mut w = ElfWriter::new();
                    w.process_statements(&s).ok()
                })
                .is_none()
        );
        // `.option norelax` drops every pairing.
        assert_eq!(
            types(".option norelax\nlui t0, %tprel_hi(v)\nadd t0, t0, tp, %tprel_add(v)\n"),
            vec![(0, 29), (4, 32)]
        );
    }

    /// GOT-slot pseudos: TLS hi20 never pairs RELAX, the GOT `lga` does, and
    /// `ld` (RV64) reads the slot. `la` is GOT-indirect only under
    /// `.option pic` (and `.option push/pop` restore the state).
    #[test]
    fn got_pseudos_match_gas() {
        assert_eq!(
            types("la.tls.ie a5, v\nla.tls.gd a0, v\nlga t0, g\n"),
            vec![
                (0, 21),
                (4, 24),
                (4, 51),
                (8, 22),
                (12, 24),
                (12, 51),
                (16, 20),
                (16, 51),
                (20, 24),
                (20, 51)
            ]
        );
        assert_eq!(
            types(".option pic\nla a0, g\n.option push\n.option nopic\nla a1, g\n.option pop\nla a2, g\n")
                .into_iter()
                .filter(|&(_, t)| t == 20 || t == 23)
                .collect::<Vec<_>>(),
            vec![(0, 20), (8, 23), (16, 20)]
        );
        let statements = parse_asm("la.tls.ie a5, v\n").unwrap();
        let mut w = ElfWriter::new();
        w.process_statements(&statements).unwrap();
        let text = &w.base.sections[".text"].data;
        // auipc a5,0 ; ld a5,0(a5) -- GAS bytes 00000797 0007b783.
        assert_eq!(text[..8], [0x97, 0x07, 0x00, 0x00, 0x83, 0xb7, 0x07, 0x00]);
    }

    /// `call foo@plt` is GAS's legacy spelling of `call foo`; `mv` is
    /// `c.mv`, or `addi rd, rs, 0` under `.option norvc` (GAS 2.47 words).
    #[test]
    fn call_plt_suffix_and_mv_match_gas() {
        assert_eq!(
            relocs("call foo@plt\ntail bar@plt\n"),
            vec![
                (0, 19, "foo".to_string()),
                (0, 51, String::new()),
                (8, 19, "bar".to_string()),
                (8, 51, String::new())
            ]
        );
        assert!(parse_asm("call foo@plt+4\n").ok().is_none_or(|s| {
            let mut w = ElfWriter::new();
            w.process_statements(&s).is_err()
        }));
        for (asm, want) in [
            ("mv a0, t0\n", &[0x16, 0x85][..]),
            (".option norvc\nmv a0, t0\n", &[0x13, 0x85, 0x02, 0x00][..]),
        ] {
            let statements = parse_asm(asm).unwrap();
            let mut w = ElfWriter::new();
            w.process_statements(&statements).unwrap();
            assert_eq!(w.base.sections[".text"].data, want, "{asm:?}");
        }
    }

    /// A numeric reference with no definition is an error (GAS 2.47), not
    /// an undefined symbol called "3f".
    #[test]
    fn undefined_numeric_label_is_an_error() {
        for asm in [
            "j 3f\n",
            "1:\tauipc a0, %pcrel_hi(g)\n\taddi a0, a0, %pcrel_lo(3b)\n",
            "2:\tnop\n\tlw a0, %pcrel_lo(2f)(a0)\n",
        ] {
            let statements = parse_asm(asm).unwrap();
            let mut w = ElfWriter::new();
            assert!(w.process_statements(&statements).is_err(), "{asm:?}");
        }
    }

    /// `%pcrel_lo(1b)` names its auipc through a GNU numeric label: the
    /// relocation must point at a local label on that auipc, never at an
    /// undefined symbol called "1b" (the Linux/glibc `1: auipc` idiom).
    #[test]
    fn numeric_label_pcrel_lo_targets_the_auipc() {
        let statements =
            parse_asm("nop\n1:\tauipc a0, %pcrel_hi(g)\n\taddi a3, a0, %pcrel_lo(1b)\n").unwrap();
        let mut w = ElfWriter::new();
        w.process_statements(&statements).unwrap();
        let lo = w.base.sections[".text"]
            .relocs
            .iter()
            .find(|r| r.reloc_type == 24)
            .expect("PCREL_LO12_I");
        assert!(lo.symbol_name.starts_with(".L"), "{}", lo.symbol_name);
        // After the 2-byte `nop` (`c.nop`), as GNU as rv64gc places it.
        assert_eq!(w.base.labels[&lo.symbol_name], (".text".to_string(), 2));
    }
}

/// Data values naming code that linker relaxation may move.  Rows are GNU as
/// 2.44 (local) and 2.47.20260726, which agree on every one of them.
#[cfg(test)]
mod relax_value_tests {
    use super::super::parser::parse_asm;
    use super::*;

    const ADD32: u32 = 35;
    const SUB32: u32 = 39;
    const R64: u32 = 2;
    const SET_ULEB128: u32 = 60;
    const SUB_ULEB128: u32 = 61;

    fn assemble(asm: &str) -> ElfWriter {
        let statements = parse_asm(asm).expect("must parse");
        let mut w = ElfWriter::new();
        w.process_statements(&statements).expect("must assemble");
        w
    }

    fn relocs(w: &ElfWriter, sec: &str) -> Vec<(u64, u32, String, i64)> {
        w.base.sections[sec]
            .relocs
            .iter()
            .map(|r| (r.offset, r.reloc_type, r.symbol_name.clone(), r.addend))
            .collect()
    }

    fn r(off: u64, ty: u32, sym: &str, addend: i64) -> (u64, u32, String, i64) {
        (off, ty, sym.to_string(), addend)
    }

    /// GNU as: `.word b - a` over code is an ADD32/SUB32 pair against the
    /// labels in any section, even between labels with nothing relaxable in
    /// between, and even when the difference was written after both labels.
    /// Folding it (what this backend did) froze the pre-relaxation distance:
    /// a jump table or a length past a relaxed `call` read 4 bytes too far.
    #[test]
    fn code_label_differences_stay_relocations() {
        let w = assemble(
            ".text\na: call foo\n.p2align 4\nb: ret\n.word b-a\n\
             .section .rodata\n.word b-a\n.text\nc: addi a0,a0,1\nd: ret\n\
             .section .rodata\n.word d-c\n",
        );
        assert_eq!(
            relocs(&w, ".rodata"),
            [
                r(0, ADD32, "b", 0),
                r(0, SUB32, "a", 0),
                r(4, ADD32, "d", 0),
                r(4, SUB32, "c", 0),
            ]
        );
        assert_eq!(w.base.sections[".rodata"].data, [0u8; 8]);
        let text: Vec<_> = relocs(&w, ".text")
            .into_iter()
            .filter(|(_, t, _, _)| *t == ADD32 || *t == SUB32)
            .collect();
        // GNU as rv64gc: `b` at 0x16, the 2-byte `ret` ends at 0x18.
        assert_eq!(text, [r(24, ADD32, "b", 0), r(24, SUB32, "a", 0)]);
        // A data label on the other side keeps its constant on the ADD.
        let w =
            assemble(".text\n.LBB1: ret\n.section .rodata\n.byte 0\n.Ljt: .word .LBB1 - .Ljt\n");
        let rel = relocs(&w, ".rodata");
        assert_eq!(rel[0].1, ADD32);
        assert_eq!(rel[0].2, ".LBB1");
        assert_eq!(rel[1].1, SUB32);
        assert_eq!(rel[0].3 - rel[1].3, -1, "the ADD carries -(.Ljt - .rodata)");
    }

    /// GNU as folds them all when relaxation is off at the end of the input,
    /// and folds `.set` (and `.org`) eagerly whatever the relax state.
    #[test]
    fn folding_follows_gnu_as() {
        let w = assemble(
            ".text\na: call foo\nb: ret\n.section .rodata\n.word b-a\n.text\n.option norelax\n",
        );
        assert!(relocs(&w, ".rodata").is_empty());
        assert_eq!(w.base.sections[".rodata"].data, 8u32.to_le_bytes());
        let w = assemble(".text\na: call foo\nb:\n.set L, b-a\n.section .rodata\n.word L\n");
        assert!(relocs(&w, ".rodata").is_empty());
        assert_eq!(w.base.sections[".rodata"].data, 8u32.to_le_bytes());
    }

    /// A lone local code label is relocated against by name
    /// (`tc_fix_adjustable` is 0 for code on RISC-V): `.text + 8` would not
    /// follow the label when the linker deletes bytes before it.
    #[test]
    fn a_code_label_is_not_rewritten_to_its_section() {
        let w = assemble(".text\ncall foo\n.L1: ret\n.data\n.quad .L1\n");
        assert_eq!(relocs(&w, ".data"), [r(0, R64, ".L1", 0)]);
        // Data labels still go through their section, as before.
        let w = assemble(".section .rodata\n.byte 1\n.L2: .byte 2\n.data\n.quad .L2\n");
        assert_eq!(relocs(&w, ".data"), [r(0, R64, ".rodata", 1)]);
    }

    /// `.` at a code location is a label of its own (GNU as's `.L0 `).
    #[test]
    fn dot_in_code_is_a_label() {
        let w = assemble(".text\n1: call foo\n.word 1b - .\n");
        let rel: Vec<_> = relocs(&w, ".text")
            .into_iter()
            .filter(|(_, t, _, _)| *t == ADD32 || *t == SUB32)
            .collect();
        assert_eq!(rel.len(), 2);
        assert_eq!((rel[0].0, rel[0].1), (8, ADD32));
        assert_eq!((rel[1].0, rel[1].1), (8, SUB32));
        assert!(rel[1].2.starts_with(".L0\u{1}"), "{:?}", rel[1].2);
        assert_eq!(w.base.labels[&rel[1].2], (".text".to_string(), 8));
        // In a data section `.` does not move: section-relative, as before.
        let w = assemble(".text\n1: call foo\n.section .rodata\n.word 0\n.word 1b - .\n");
        let rel = relocs(&w, ".rodata");
        assert_eq!(
            (rel[1].1, rel[1].2.as_str(), rel[0].3),
            (SUB32, ".rodata", -4)
        );
    }

    #[test]
    fn uleb128_constants_and_sleb128() {
        let w = assemble(
            ".data\n.uleb128 300\n.sleb128 -2\n.uleb128 0\n.L1: .byte 1\n.L2: .sleb128 .L1 - .L2\n",
        );
        assert_eq!(
            w.base.sections[".data"].data,
            [0xac, 0x02, 0x7e, 0x00, 0x01, 0x7f]
        );
        // GNU as: "non-constant .sleb128 is not supported" (`s_riscv_leb128`).
        for asm in [
            ".data\n.sleb128 .L1 - .L2\n.L2: .byte 1\n.L1:\n",
            ".text\na: call foo\nb: ret\n.data\n.sleb128 b - a\n",
        ] {
            let statements = parse_asm(asm).expect("must parse");
            let err = ElfWriter::new()
                .process_statements(&statements)
                .unwrap_err();
            assert!(
                err.contains("non-constant .sleb128 is not supported"),
                "{asm}: {err}"
            );
        }
    }

    /// `.uleb128 b - a` over code: R_RISCV_SET_ULEB128 + R_RISCV_SUB_ULEB128
    /// over the assembly-time value (GNU as: `16` with b 0x16 past a).
    #[test]
    fn uleb128_over_code_is_a_set_sub_pair() {
        let w =
            assemble(".text\na: call foo\n.p2align 4\nb: ret\n.section .rodata\n.uleb128 b-a\n");
        assert_eq!(w.base.sections[".rodata"].data, [0x16]);
        assert_eq!(
            relocs(&w, ".rodata"),
            [r(0, SET_ULEB128, "b", 0), r(0, SUB_ULEB128, "a", 0)]
        );
        // A forward reference to a code label: GNU as writes the assembly-time
        // value, -2 here (`ret` is `c.jr ra`), in its 10-byte LEB128
        // (`feffffff ffffffff ff01`).
        let w = assemble(".text\nb: ret\n.section .rodata\n.uleb128 b - a\n.text\na: call foo\n");
        assert_eq!(
            w.base.sections[".rodata"].data,
            [0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]
        );
        assert_eq!(
            relocs(&w, ".rodata"),
            [r(0, SET_ULEB128, "b", 0), r(0, SUB_ULEB128, "a", 0)]
        );
        // The constant rides on the SET (GNU as drops it from the pair).
        let w = assemble(".text\na: call foo\nb: ret\n.section .rodata\n.uleb128 b-a+3\n");
        assert_eq!(w.base.sections[".rodata"].data, [11]);
        assert_eq!(
            relocs(&w, ".rodata"),
            [r(0, SET_ULEB128, "b", 3), r(0, SUB_ULEB128, "a", 0)]
        );
    }

    /// Forward references take their canonical length, as GNU as relaxes them
    /// to (`.uleb128 .Lfw - .Lc2` over 200 bytes is `c8 01`); data labels are
    /// exact at assembly time, so no relocation is needed for them.
    #[test]
    fn forward_uleb128_takes_the_canonical_length() {
        let w = assemble(".data\n.uleb128 .Lfw - .Lc2\n.Lc2: .fill 200,1,0\n.Lfw:\n.byte 7\n");
        let d = &w.base.sections[".data"].data;
        assert_eq!(&d[..2], &[0xc8, 0x01]);
        assert_eq!(d.len(), 2 + 200 + 1);
        assert_eq!(d[202], 7, "everything after moved with the grown field");
        assert!(relocs(&w, ".data").is_empty());
        // Two growing fields, the second spanning the first's growth.
        let w = assemble(
            ".data\n.uleb128 .L2-.L1\n.uleb128 .L3-.L1\n.L1: .fill 130,1,0\n.L2: .fill 20000,1,0\n.L3:\n",
        );
        let d = &w.base.sections[".data"].data;
        assert_eq!(&d[..2], &[0x82, 0x01]);
        assert_eq!(&d[2..5], &[0xa2, 0x9d, 0x01]);
        assert_eq!(d.len(), 5 + 20130);
    }

    #[test]
    fn fixed_width_uleb128_encoding() {
        assert_eq!(uleb128_fixed(0x16, 1), Some(vec![0x16]));
        assert_eq!(uleb128_fixed(0x16, 3), Some(vec![0x96, 0x80, 0x00]));
        assert_eq!(uleb128_fixed(128, 1), None);
        assert_eq!(uleb128_fixed(u64::MAX, 10).map(|v| v.len()), Some(10));
        for v in [0u64, 1, 127, 128, 16383, 16384, u64::MAX] {
            let mut canon = Vec::new();
            crate::backend::elf_writer_common::encode_uleb128(&mut canon, v);
            assert_eq!(uleb128_len(v), canon.len(), "{v}");
            assert_eq!(uleb128_fixed(v, canon.len()), Some(canon));
        }
    }
}

#[cfg(test)]
mod debug_line_tests {
    use super::super::parser::parse_asm;
    use super::*;

    fn assembled(asm: &str) -> ElfWriter {
        let statements = parse_asm(asm).expect("must parse");
        let mut w = ElfWriter::new();
        w.process_statements(&statements).expect("must assemble");
        w.base
            .finish_debug_sections(8, 1, 2)
            .expect("debug sections");
        w
    }

    const SOURCE: &str = ".file 1 \"p.c\"\n.text\n.globl f\n.type f,@function\nf:\n\
                          .loc 1 3 0\ncall g\n.loc 1 4 0\nret\n.size f, .-f\n";

    /// Under linker relaxation (the `call` may become a `jal`) every code
    /// address of the line table and the units is a label relocation -- the
    /// linker moves labels with deleted bytes, never `.text`+offset.
    #[test]
    fn relaxable_code_is_addressed_through_labels() {
        let w = assembled(SOURCE);
        let line = &w.base.sections[".debug_line"];
        assert_eq!(line.relocs.len(), 3, "first row, second row, sequence end");
        assert!(
            line.relocs
                .iter()
                .all(|r| r.reloc_type == 2 && r.addend == 0 && r.symbol_name.starts_with(".Lloc")),
            "{:?}",
            line.relocs
                .iter()
                .map(|r| (r.reloc_type, r.symbol_name.as_str(), r.addend))
                .collect::<Vec<_>>()
        );
        let info = &w.base.sections[".debug_info"];
        let code: Vec<_> = info
            .relocs
            .iter()
            .filter(|r| r.reloc_type == 2)
            .map(|r| (r.symbol_name.as_str(), r.addend))
            .collect();
        assert_eq!(
            code.len(),
            4,
            "unit start and end, f's start and end: {code:?}"
        );
        assert_eq!(
            code[0],
            (".text", 0),
            "the unit starts where its section does"
        );
        assert!(
            code[1..]
                .iter()
                .all(|&(s, a)| s.starts_with(".Lloc") && a == 0),
            "{code:?}"
        );
        for r in line.relocs.iter().chain(&info.relocs) {
            if r.symbol_name.starts_with(".Lloc") {
                assert!(
                    w.base.labels.contains_key(&r.symbol_name),
                    "{}",
                    r.symbol_name
                );
            }
        }
    }

    /// `.option norelax` at the end of the input: nothing moves, so section
    /// offsets do, as for every other target.
    #[test]
    fn norelax_code_is_addressed_by_section_offsets() {
        let w = assembled(&format!("{SOURCE}.option norelax\n"));
        let line = &w.base.sections[".debug_line"];
        let targets: Vec<_> = line
            .relocs
            .iter()
            .map(|r| (r.symbol_name.as_str(), r.addend))
            .collect();
        assert_eq!(targets, [(".text", 0)]);
    }
}
