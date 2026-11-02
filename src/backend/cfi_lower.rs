//! Target-neutral lowering of `.cfi_*` directives into a DWARF `.eh_frame`
//! section, shared by the integrated assemblers (x86-64, i386, AArch64, RISC-V).
//!
//! GNU as turns the CFI directives of a translation unit into call frame
//! information: one CIE per distinct set of frame conventions and one FDE per
//! `.cfi_startproc` ... `.cfi_endproc` region. Without it no unwinder can walk
//! through the object's code: forced unwinding (`pthread_exit`,
//! `pthread_cancel`) stops at the first such frame so the cleanup handlers of
//! every caller are skipped, C++ exceptions through C callbacks call
//! `std::terminate`, and `backtrace()`/gdb/perf lose the call chain.
//!
//! The lowering runs on the parsed item stream of an assembler, before
//! layout, through the [`CfiStream`] adapter:
//!
//! * every CFI directive is replaced by a local label marking its location
//!   (consecutive directives with no bytes in between share one label), and
//!   the directive is recorded as a DWARF call frame instruction;
//! * after the last item the CIEs and FDEs are appended as ordinary data
//!   directives in `.eh_frame`. Every location-dependent field is a label
//!   difference (`.long end - start`, `.long start - .`), so the ELF writer
//!   resolves it after layout exactly like hand-written `.eh_frame` data.
//!
//! Two kinds of field need target help and become adapter items:
//!
//! * a location advance spans code whose size is final only after the
//!   assembler's branch relaxation (x86) or the linker's relaxation
//!   (RISC-V), so it is a [`CfiStream::advance`] item the ELF writer sizes;
//! * where GNU as relocates an FDE's initial location (and a pc-relative
//!   personality/LSDA word) with a dedicated PC-relative relocation instead
//!   of a plain `sym - .` difference -- `R_RISCV_32_PCREL`, whereas a RISC-V
//!   `sym - .` data word becomes an ADD32/SUB32 pair -- the target sets
//!   [`CfiTarget::pcrel_item`] and gets [`CfiStream::pcrel32`] items.
//!
//! Encoding choices match GNU as: augmentation `zR` plus `P`, `L`, `S` when
//! requested, `DW_EH_PE_pcrel|sdata4` FDE pointers, CIE/FDE padded with
//! `DW_CFA_nop` to 4 bytes (the last FDE to the address size), CIE version
//! and initial instructions per target. `.cfi_sections` selecting only
//! `.debug_frame` suppresses the output, as with GNU as when `.debug_frame`
//! is not requested; `.debug_frame` itself is not generated.

use crate::backend::asm_expr;
use crate::common::fx_hash::FxHashMap;
use std::fmt::Write as _;

/// Frame conventions of one target, as GNU as encodes them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CfiTarget {
    /// Address size: width of absolute encoded pointers and the alignment
    /// of the end of `.eh_frame`.
    pub ptr_size: i64,
    /// CIE code alignment factor (unit of location advances).
    pub code_align: u64,
    /// CIE data alignment factor (unit of factored register offsets).
    pub data_align: i64,
    /// Return-address column when no `.cfi_return_column` is given.
    pub default_ra_column: u64,
    /// DWARF number of the stack pointer.
    pub sp_column: u64,
    /// CFA offset from the stack pointer at function entry: x86 `call`
    /// pushed the return address (CIE: `def_cfa sp, slot; offset ra,
    /// -slot`); RISC-V keeps it in a register (CIE: `def_cfa sp, 0`).
    pub entry_cfa_offset: i64,
    /// The CIE records the return address saved at CFA - entry offset.
    pub ra_saved_at_entry: bool,
    /// CIE version regardless of the return column (RISC-V GNU as: 3);
    /// `None` = 1, or 3 when the column needs more than one byte.
    pub cie_version: Option<u8>,
    /// FDE initial locations and 4-byte pc-relative personality/LSDA words
    /// are [`CfiStream::pcrel32`] items rather than `.long sym - .`.
    pub pcrel_item: bool,
    /// DWARF register number of a register name, as CFI operands spell it.
    pub dwarf_reg: fn(&str) -> Option<u64>,
}

/// What the lowering needs to know about one assembler item.
pub(crate) enum CfiItemKind {
    Section(SectionKey),
    PushSection(SectionKey),
    PopSection,
    Previous,
    /// A CFI directive: name (`.cfi_offset`) and operand text.
    Cfi(String, String),
    /// Emits no bytes: a location label before it marks the same address
    /// as one after it.
    NoBytes,
    /// Everything else.
    Bytes,
}

/// An assembler's item stream as the CFI lowering sees it.
pub(crate) trait CfiStream: Sized {
    fn cfi_kind(&self) -> CfiItemKind;
    /// The label this item defines, if it is a label definition.
    fn as_label(&self) -> Option<&str>;
    fn label(name: String) -> Self;
    /// Parse assembler text (the rendered `.eh_frame`).
    fn parse(text: &str) -> Result<Vec<Self>, String>;
    /// A DWARF location advance from label `from` to label `to`, sized
    /// once the layout is known.
    fn advance(from: String, to: String) -> Self;
    /// A 4-byte PC-relative reference to `sym`; only requested when
    /// [`CfiTarget::pcrel_item`] is set.
    fn pcrel32(sym: String) -> Result<Self, String>;
}

/// Section identity for the "same section as `.cfi_startproc`" check.
pub(crate) type SectionKey = (String, Option<String>);

/// Personality / LSDA pointer: DW_EH_PE encoding plus symbol.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct EncodedPtr {
    enc: u8,
    sym: String,
}

/// Everything that decides which CIE an FDE refers to.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct CieKey {
    simple: bool,
    signal_frame: bool,
    ra_column: u64,
    personality: Option<EncodedPtr>,
    lsda_enc: Option<u8>,
}

struct Fde {
    start: String,
    end: String,
    cie: CieKey,
    lsda: Option<EncodedPtr>,
    /// (location label, encoded instruction)
    insns: Vec<(String, Vec<InsnPart>)>,
}

/// One piece of an encoded call frame instruction.
#[derive(Clone, Debug)]
enum InsnPart {
    Byte(u8),
    Uleb(u64),
    Sleb(i64),
}

struct OpenFde {
    start: String,
    section: SectionKey,
    simple: bool,
    signal_frame: bool,
    ra_column: u64,
    personality: Option<EncodedPtr>,
    lsda: Option<EncodedPtr>,
    insns: Vec<(String, Vec<InsnPart>)>,
    /// Tracked CFA offset (for `.cfi_adjust_cfa_offset`/`.cfi_rel_offset`).
    cfa_offset: i64,
    saved_cfa_offsets: Vec<i64>,
}

fn dwarf_reg(t: &CfiTarget, name: &str) -> Result<u64, String> {
    let n = name.trim();
    if let Ok(v) = n.parse::<u64>() {
        return Ok(v);
    }
    (t.dwarf_reg)(n).ok_or_else(|| format!("bad register expression `{}` in CFI directive", n))
}

fn parse_int(s: &str) -> Result<i64, String> {
    asm_expr::parse_integer_expr(s.trim())
        .map_err(|_| format!("bad expression `{}` in CFI directive", s.trim()))
}

fn split_args(args: &str) -> Vec<&str> {
    if args.trim().is_empty() {
        Vec::new()
    } else {
        args.split(',').map(str::trim).collect()
    }
}

fn factored(off: i64, align: i64, what: &str) -> Result<i64, String> {
    if off % align != 0 {
        return Err(format!(
            "{} offset {} is not a multiple of the data alignment {}",
            what, off, align
        ));
    }
    Ok(off / align)
}

fn encode_offset(reg: u64, off: i64, t: &CfiTarget) -> Result<Vec<InsnPart>, String> {
    let f = factored(off, t.data_align, ".cfi_offset")?;
    Ok(if f >= 0 && reg < 64 {
        vec![InsnPart::Byte(0x80 | reg as u8), InsnPart::Uleb(f as u64)]
    } else if f >= 0 {
        // DW_CFA_offset_extended
        vec![
            InsnPart::Byte(0x05),
            InsnPart::Uleb(reg),
            InsnPart::Uleb(f as u64),
        ]
    } else {
        // DW_CFA_offset_extended_sf
        vec![InsnPart::Byte(0x11), InsnPart::Uleb(reg), InsnPart::Sleb(f)]
    })
}

fn encode_def_cfa_offset(off: i64, t: &CfiTarget) -> Result<Vec<InsnPart>, String> {
    Ok(if off >= 0 {
        vec![InsnPart::Byte(0x0e), InsnPart::Uleb(off as u64)]
    } else {
        // DW_CFA_def_cfa_offset_sf takes a factored operand.
        let f = factored(off, t.data_align, ".cfi_def_cfa_offset")?;
        vec![InsnPart::Byte(0x13), InsnPart::Sleb(f)]
    })
}

fn parse_encoded_ptr(args: &str, what: &str) -> Result<Option<EncodedPtr>, String> {
    let parts = split_args(args);
    let enc = parts
        .first()
        .ok_or_else(|| format!("{} requires an encoding", what))
        .and_then(|e| parse_int(e))?;
    if enc == 0xff {
        return Ok(None); // DW_EH_PE_omit
    }
    let enc = u8::try_from(enc).map_err(|_| format!("invalid {} encoding {}", what, enc))?;
    // Only the pointer forms an unwinder decodes without extra context:
    // absptr/udata/sdata (2/4/8 bytes), absolute or pc-relative, optionally
    // indirect. GNU as rejects the same set it cannot express.
    let format_ok = matches!(enc & 0x0f, 0x00 | 0x02 | 0x03 | 0x04 | 0x0a | 0x0b | 0x0c);
    let app_ok = matches!(enc & 0x70, 0x00 | 0x10);
    if !format_ok || !app_ok {
        return Err(format!("unsupported {} encoding {:#x}", what, enc));
    }
    let sym = parts
        .get(1)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{} requires a symbol", what))?;
    Ok(Some(EncodedPtr {
        enc,
        sym: sym.to_string(),
    }))
}

fn encoded_ptr_size(enc: u8, t: &CfiTarget) -> i64 {
    match enc & 0x0f {
        0x00 => t.ptr_size,
        0x02 | 0x0a => 2,
        0x03 | 0x0b => 4,
        _ => 8,
    }
}

const ADVANCE_MARK: &str = ".L__lccc_cfi_adv";
const PCREL_MARK: &str = ".L__lccc_cfi_pcrel";

/// Rendering state: the text plus the adapter items its markers stand for.
struct Render<'t> {
    t: &'t CfiTarget,
    s: String,
    advances: Vec<(String, String)>,
    pcrels: Vec<String>,
}

impl Render<'_> {
    fn line(&mut self, l: std::fmt::Arguments<'_>) {
        let _ = self.s.write_fmt(l);
        self.s.push('\n');
    }

    /// `.long sym - .`, or a PC-relative adapter item.
    fn pcrel32(&mut self, sym: &str) {
        if self.t.pcrel_item {
            self.line(format_args!("{}{}:", PCREL_MARK, self.pcrels.len()));
            self.pcrels.push(sym.to_string());
        } else {
            self.line(format_args!(".long {} - .", sym));
        }
    }

    /// One encoded pointer as a data directive.
    fn encoded_ptr(&mut self, p: &EncodedPtr) {
        let size = encoded_ptr_size(p.enc, self.t);
        let pcrel = p.enc & 0x70 == 0x10;
        if pcrel && size == 4 {
            return self.pcrel32(&p.sym);
        }
        let dir = match size {
            2 => ".short",
            4 => ".long",
            _ => ".quad",
        };
        if pcrel {
            self.line(format_args!("{} {} - .", dir, p.sym));
        } else {
            self.line(format_args!("{} {}", dir, p.sym));
        }
    }

    fn insn(&mut self, parts: &[InsnPart]) {
        for p in parts {
            match p {
                InsnPart::Byte(b) => self.line(format_args!(".byte {:#x}", b)),
                InsnPart::Uleb(v) => self.line(format_args!(".uleb128 {}", v)),
                InsnPart::Sleb(v) => self.line(format_args!(".sleb128 {}", v)),
            }
        }
    }
}

/// Replace the CFI directives of `items` by location labels and append the
/// `.eh_frame` data they describe.
pub(crate) fn lower_cfi<S: CfiStream>(items: Vec<S>, t: &CfiTarget) -> Result<Vec<S>, String> {
    if !items
        .iter()
        .any(|i| matches!(i.cfi_kind(), CfiItemKind::Cfi(..)))
    {
        return Ok(items);
    }
    let mut out: Vec<S> = Vec::with_capacity(items.len() + 16);
    let mut fdes: Vec<Fde> = Vec::new();
    let mut open: Option<OpenFde> = None;
    let mut emit_eh_frame = true;
    let mut label_seq = 0usize;
    // Label naming the current location, valid while no byte-emitting item
    // has been seen since it was placed.
    let mut here: Option<String> = None;
    let mut cur_section: Option<SectionKey> = Some((".text".to_string(), None));
    let mut prev_section: Option<SectionKey> = None;
    let mut section_stack: Vec<(Option<SectionKey>, Option<SectionKey>)> = Vec::new();

    let new_label = |label_seq: &mut usize| {
        let l = format!(".L__lccc_cfi{}", *label_seq);
        *label_seq += 1;
        l
    };

    for item in items {
        let (name, args) = match item.cfi_kind() {
            CfiItemKind::Cfi(name, args) => (name, args),
            kind => {
                let no_bytes = matches!(kind, CfiItemKind::NoBytes);
                match kind {
                    CfiItemKind::Section(key) => {
                        prev_section = cur_section.take();
                        cur_section = Some(key);
                    }
                    CfiItemKind::PushSection(key) => {
                        section_stack.push((cur_section.clone(), prev_section.clone()));
                        prev_section = cur_section.take();
                        cur_section = Some(key);
                    }
                    CfiItemKind::PopSection => {
                        if let Some((c, p)) = section_stack.pop() {
                            cur_section = c;
                            prev_section = p;
                        }
                    }
                    CfiItemKind::Previous => std::mem::swap(&mut cur_section, &mut prev_section),
                    CfiItemKind::NoBytes | CfiItemKind::Bytes | CfiItemKind::Cfi(..) => {}
                }
                if !no_bytes {
                    here = None;
                }
                out.push(item);
                continue;
            }
        };

        // Location label for this directive.
        let loc = match &here {
            Some(l) => l.clone(),
            None => {
                let l = new_label(&mut label_seq);
                out.push(S::label(l.clone()));
                here = Some(l.clone());
                l
            }
        };
        let name = name.as_str();

        match name {
            ".cfi_sections" => {
                emit_eh_frame = split_args(&args).contains(&".eh_frame");
                continue;
            }
            ".cfi_startproc" => {
                if open.is_some() {
                    return Err(".cfi_startproc without matching .cfi_endproc".to_string());
                }
                let simple = match args.trim() {
                    "" => false,
                    "simple" => true,
                    other => return Err(format!("junk at end of .cfi_startproc: `{}`", other)),
                };
                let section = cur_section
                    .clone()
                    .ok_or_else(|| ".cfi_startproc outside any section".to_string())?;
                open = Some(OpenFde {
                    start: loc,
                    section,
                    simple,
                    signal_frame: false,
                    ra_column: t.default_ra_column,
                    personality: None,
                    lsda: None,
                    insns: Vec::new(),
                    cfa_offset: if simple { 0 } else { t.entry_cfa_offset },
                    saved_cfa_offsets: Vec::new(),
                });
                continue;
            }
            _ => {}
        }

        let Some(f) = open.as_mut() else {
            return Err(format!(
                "CFI instruction used without previous .cfi_startproc: {}",
                name
            ));
        };
        if cur_section.as_ref() != Some(&f.section) {
            return Err(format!(
                "{} in section {} but the enclosing .cfi_startproc is in {}",
                name,
                cur_section.as_ref().map_or("<none>", |s| s.0.as_str()),
                f.section.0
            ));
        }
        if name == ".cfi_endproc" {
            if !args.trim().is_empty() {
                return Err(format!("junk at end of .cfi_endproc: `{}`", args.trim()));
            }
            let Some(f) = open.take() else {
                return Err(format!("{name} without an open .cfi_startproc"));
            };
            fdes.push(Fde {
                start: f.start,
                end: loc,
                cie: CieKey {
                    simple: f.simple,
                    signal_frame: f.signal_frame,
                    ra_column: f.ra_column,
                    lsda_enc: f.lsda.as_ref().map(|l| l.enc),
                    personality: f.personality,
                },
                lsda: f.lsda,
                insns: f.insns,
            });
            continue;
        }
        match encode_cfi_insn(f, name, &args, t)? {
            CfiEffect::Insn(insn) => f.insns.push((loc, insn)),
            CfiEffect::StateOnly => {}
            CfiEffect::Label(l) => out.push(S::label(l)),
        }
    }

    if open.is_some() {
        return Err("open CFI at the end of file; missing .cfi_endproc directive".to_string());
    }
    if !emit_eh_frame || fdes.is_empty() {
        return Ok(out);
    }
    let r = render_eh_frame(&fdes, t);
    let parsed = S::parse(&r.s).map_err(|e| format!("internal .eh_frame lowering: {}", e))?;
    for item in parsed {
        if let Some(l) = item.as_label() {
            if let Some(i) = marker_index(l, ADVANCE_MARK, r.advances.len())? {
                let (from, to) = r.advances[i].clone();
                out.push(S::advance(from, to));
                continue;
            }
            if let Some(i) = marker_index(l, PCREL_MARK, r.pcrels.len())? {
                out.push(S::pcrel32(r.pcrels[i].clone())?);
                continue;
            }
        }
        out.push(item);
    }
    Ok(out)
}

/// What one CFI instruction directive contributes to its open FDE.
enum CfiEffect {
    /// A DWARF call frame instruction at the directive's location.
    Insn(Vec<InsnPart>),
    /// Only FDE/CIE state changed (signal frame, return column, personality,
    /// LSDA).
    StateOnly,
    /// `.cfi_label`: a label at the directive's location.
    Label(String),
}

/// Encode CFI directive `name` (every directive other than `.cfi_sections`,
/// `.cfi_startproc` and `.cfi_endproc`) for the open FDE `f`, tracking the
/// CFA offset that `.cfi_rel_offset`, `.cfi_adjust_cfa_offset` and the
/// remember/restore stack need.
fn encode_cfi_insn(
    f: &mut OpenFde,
    name: &str,
    args: &str,
    t: &CfiTarget,
) -> Result<CfiEffect, String> {
    let a = split_args(args);
    let need = |n: usize| -> Result<(), String> {
        if a.len() == n {
            Ok(())
        } else {
            Err(format!("{} expects {} operand(s), got `{}`", name, n, args))
        }
    };
    let insn: Vec<InsnPart> = match name {
        ".cfi_def_cfa" => {
            need(2)?;
            let reg = dwarf_reg(t, a[0])?;
            let off = parse_int(a[1])?;
            f.cfa_offset = off;
            if off >= 0 {
                vec![
                    InsnPart::Byte(0x0c),
                    InsnPart::Uleb(reg),
                    InsnPart::Uleb(off as u64),
                ]
            } else {
                let fo = factored(off, t.data_align, name)?;
                vec![
                    InsnPart::Byte(0x12),
                    InsnPart::Uleb(reg),
                    InsnPart::Sleb(fo),
                ]
            }
        }
        ".cfi_def_cfa_offset" => {
            need(1)?;
            let off = parse_int(a[0])?;
            f.cfa_offset = off;
            encode_def_cfa_offset(off, t)?
        }
        ".cfi_adjust_cfa_offset" => {
            need(1)?;
            f.cfa_offset += parse_int(a[0])?;
            encode_def_cfa_offset(f.cfa_offset, t)?
        }
        ".cfi_def_cfa_register" => {
            need(1)?;
            vec![InsnPart::Byte(0x0d), InsnPart::Uleb(dwarf_reg(t, a[0])?)]
        }
        ".cfi_offset" => {
            need(2)?;
            encode_offset(dwarf_reg(t, a[0])?, parse_int(a[1])?, t)?
        }
        ".cfi_rel_offset" => {
            need(2)?;
            let off = parse_int(a[1])? - f.cfa_offset;
            encode_offset(dwarf_reg(t, a[0])?, off, t)?
        }
        ".cfi_val_offset" => {
            need(2)?;
            let reg = dwarf_reg(t, a[0])?;
            let fo = factored(parse_int(a[1])?, t.data_align, name)?;
            if fo >= 0 {
                vec![
                    InsnPart::Byte(0x14),
                    InsnPart::Uleb(reg),
                    InsnPart::Uleb(fo as u64),
                ]
            } else {
                vec![
                    InsnPart::Byte(0x15),
                    InsnPart::Uleb(reg),
                    InsnPart::Sleb(fo),
                ]
            }
        }
        ".cfi_register" => {
            need(2)?;
            vec![
                InsnPart::Byte(0x09),
                InsnPart::Uleb(dwarf_reg(t, a[0])?),
                InsnPart::Uleb(dwarf_reg(t, a[1])?),
            ]
        }
        ".cfi_restore" => {
            if a.is_empty() {
                return Err(".cfi_restore expects register operand(s)".to_string());
            }
            let mut v = Vec::new();
            for r in &a {
                let reg = dwarf_reg(t, r)?;
                if reg < 64 {
                    v.push(InsnPart::Byte(0xc0 | reg as u8));
                } else {
                    v.extend([InsnPart::Byte(0x06), InsnPart::Uleb(reg)]);
                }
            }
            v
        }
        ".cfi_undefined" | ".cfi_same_value" => {
            if a.is_empty() {
                return Err(format!("{} expects register operand(s)", name));
            }
            let op = if name == ".cfi_undefined" { 0x07 } else { 0x08 };
            let mut v = Vec::new();
            for r in &a {
                v.extend([InsnPart::Byte(op), InsnPart::Uleb(dwarf_reg(t, r)?)]);
            }
            v
        }
        ".cfi_remember_state" => {
            need(0)?;
            f.saved_cfa_offsets.push(f.cfa_offset);
            vec![InsnPart::Byte(0x0a)]
        }
        ".cfi_restore_state" => {
            need(0)?;
            f.cfa_offset = f
                .saved_cfa_offsets
                .pop()
                .ok_or_else(|| ".cfi_restore_state without .cfi_remember_state".to_string())?;
            vec![InsnPart::Byte(0x0b)]
        }
        ".cfi_escape" => {
            if a.is_empty() {
                return Err(".cfi_escape expects byte operand(s)".to_string());
            }
            let mut v = Vec::new();
            for b in &a {
                let x = parse_int(b)?;
                let byte =
                    u8::try_from(x & 0xff).map_err(|_| format!("bad .cfi_escape byte `{}`", b))?;
                v.push(InsnPart::Byte(byte));
            }
            v
        }
        ".cfi_signal_frame" => {
            need(0)?;
            f.signal_frame = true;
            return Ok(CfiEffect::StateOnly);
        }
        ".cfi_return_column" => {
            need(1)?;
            f.ra_column = dwarf_reg(t, a[0])?;
            return Ok(CfiEffect::StateOnly);
        }
        ".cfi_personality" => {
            f.personality = parse_encoded_ptr(args, name)?;
            return Ok(CfiEffect::StateOnly);
        }
        ".cfi_lsda" => {
            f.lsda = parse_encoded_ptr(args, name)?;
            return Ok(CfiEffect::StateOnly);
        }
        ".cfi_label" => {
            need(1)?;
            return Ok(CfiEffect::Label(a[0].to_string()));
        }
        other => {
            return Err(format!("unsupported CFI directive {}", other));
        }
    };
    Ok(CfiEffect::Insn(insn))
}

/// Index of a rendering marker label (`<mark><n>`), checked against the
/// number of markers rendered.
fn marker_index(label: &str, mark: &str, count: usize) -> Result<Option<usize>, String> {
    let Some(n) = label.strip_prefix(mark) else {
        return Ok(None);
    };
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    match n.parse::<usize>() {
        Ok(i) if i < count => Ok(Some(i)),
        _ => Err(format!("internal .eh_frame lowering: bad marker {}", label)),
    }
}

fn render_eh_frame<'t>(fdes: &[Fde], t: &'t CfiTarget) -> Render<'t> {
    let mut r = Render {
        t,
        s: String::new(),
        advances: Vec::new(),
        pcrels: Vec::new(),
    };
    let last_align_log2 = t.ptr_size.trailing_zeros();
    r.line(format_args!(".section .eh_frame,\"a\",@progbits"));

    let mut cies: FxHashMap<CieKey, String> = FxHashMap::default();
    for (i, f) in fdes.iter().enumerate() {
        if !cies.contains_key(&f.cie) {
            let l = format!(".L__lccc_cie{}", cies.len());
            write_cie(&mut r, &l, &f.cie);
            cies.insert(f.cie.clone(), l);
        }
        let l = format!(".L__lccc_fde{}", i);
        r.line(format_args!("{}:", l));
        r.line(format_args!(".long {}_end - {}_body", l, l));
        r.line(format_args!("{}_body:", l));
        r.line(format_args!(".long {}_body - {}", l, cies[&f.cie]));
        r.pcrel32(&f.start);
        r.line(format_args!(".long {} - {}", f.end, f.start));
        match &f.lsda {
            Some(p) => {
                r.line(format_args!(".uleb128 {}", encoded_ptr_size(p.enc, t)));
                r.encoded_ptr(p);
            }
            None => r.line(format_args!(".uleb128 0")),
        }
        let mut prev = f.start.as_str();
        for (loc, parts) in &f.insns {
            if loc != prev {
                r.line(format_args!("{}{}:", ADVANCE_MARK, r.advances.len()));
                r.advances.push((prev.to_string(), loc.clone()));
                prev = loc;
            }
            r.insn(parts);
        }
        let align_log2 = if i + 1 == fdes.len() {
            last_align_log2
        } else {
            2
        };
        r.line(format_args!(".p2align {}", align_log2));
        r.line(format_args!("{}_end:", l));
    }
    r
}

fn write_cie(r: &mut Render<'_>, l: &str, key: &CieKey) {
    let t = r.t;
    let mut aug = String::from("z");
    let mut aug_len = 0i64;
    if let Some(p) = &key.personality {
        aug.push('P');
        aug_len += 1 + encoded_ptr_size(p.enc, t);
    }
    if key.lsda_enc.is_some() {
        aug.push('L');
        aug_len += 1;
    }
    aug.push('R');
    aug_len += 1;
    if key.signal_frame {
        aug.push('S');
    }
    r.line(format_args!("{}:", l));
    r.line(format_args!(".long {}_end - {}_body", l, l));
    r.line(format_args!("{}_body:", l));
    r.line(format_args!(".long 0"));
    // Version 1 stores the return column in one byte; version 3 (GNU as
    // for columns > 255, and always on RISC-V) uses ULEB128.
    let version = t
        .cie_version
        .unwrap_or(if key.ra_column > 255 { 3 } else { 1 });
    r.line(format_args!(".byte {}", version));
    r.line(format_args!(".asciz \"{}\"", aug));
    r.line(format_args!(".uleb128 {}", t.code_align));
    r.line(format_args!(".sleb128 {}", t.data_align));
    if version == 1 {
        r.line(format_args!(".byte {}", key.ra_column));
    } else {
        r.line(format_args!(".uleb128 {}", key.ra_column));
    }
    r.line(format_args!(".uleb128 {}", aug_len));
    if let Some(p) = &key.personality {
        r.line(format_args!(".byte {:#x}", p.enc));
        r.encoded_ptr(p);
    }
    if let Some(enc) = key.lsda_enc {
        r.line(format_args!(".byte {:#x}", enc));
    }
    r.line(format_args!(".byte 0x1b")); // FDE pointers: pcrel | sdata4
    if !key.simple {
        r.insn(&[
            InsnPart::Byte(0x0c),
            InsnPart::Uleb(t.sp_column),
            InsnPart::Uleb(t.entry_cfa_offset as u64),
        ]);
        if t.ra_saved_at_entry
            && let Ok(parts) = encode_offset(key.ra_column, -t.entry_cfa_offset, t)
        {
            r.insn(&parts);
        }
    }
    r.line(format_args!(".p2align 2"));
    r.line(format_args!("{}_end:", l));
}
