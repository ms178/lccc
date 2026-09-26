//! Lowering of `.cfi_*` directives into a DWARF `.eh_frame` section.
//!
//! GNU as turns the CFI directives of a translation unit into call frame
//! information: one CIE per distinct set of frame conventions and one FDE per
//! `.cfi_startproc` ... `.cfi_endproc` region. Without it no unwinder can walk
//! through the object's code: forced unwinding (`pthread_exit`,
//! `pthread_cancel`) stops at the first such frame so the cleanup handlers of
//! every caller are skipped, C++ exceptions through C callbacks call
//! `std::terminate`, and `backtrace()`/gdb/perf lose the call chain.
//!
//! The lowering runs on the parsed item stream, before layout:
//!
//! * every CFI directive is replaced by a local label marking its location
//!   (consecutive directives with no bytes in between share one label), and
//!   the directive is recorded as a DWARF call frame instruction;
//! * after the last item the CIEs and FDEs are appended as ordinary data
//!   directives in `.eh_frame`. Every location-dependent field is a label
//!   difference (`.long end - start`, `.long start - .`), so the ELF writer
//!   resolves it after branch relaxation exactly like hand-written
//!   `.eh_frame` data (glibc's signal trampolines), emitting the one
//!   `R_X86_64_PC32`/`R_386_PC32` per FDE that GNU as emits.
//!
//! Encoding choices match GNU as: version 1 CIE, augmentation `zR` plus `P`,
//! `L`, `S` when requested, `DW_EH_PE_pcrel|sdata4` FDE pointers, CIE/FDE
//! padded with `DW_CFA_nop` to the pointer size, and each location advance
//! in its smallest form (`DW_CFA_advance_loc`, `_loc1`, `_loc2`, `_loc4`).
//! The distance between two code labels is only known after relaxation, so
//! an advance is an `AsmItem::CfaAdvance` that the ELF writer sizes once the
//! code layout is final.
//!
//! `.cfi_sections` selecting only `.debug_frame` (the Linux kernel outside
//! the vDSO) suppresses the output, as with GNU as when `.debug_frame` is
//! not requested; `.debug_frame` itself is not generated.

use super::parser::{AsmItem, CfiDirective, SectionDirective, parse_asm};
use crate::backend::asm_expr;
use crate::common::fx_hash::FxHashMap;
use std::fmt::Write as _;

/// Frame conventions of the target the object is assembled for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CfiArch {
    X86_64,
    I386,
}

impl CfiArch {
    fn ptr_size(self) -> i64 {
        match self {
            CfiArch::X86_64 => 8,
            CfiArch::I386 => 4,
        }
    }
    fn data_align(self) -> i64 {
        -self.ptr_size()
    }
    fn default_ra_column(self) -> u64 {
        match self {
            CfiArch::X86_64 => 16,
            CfiArch::I386 => 8,
        }
    }
    fn sp_column(self) -> u64 {
        match self {
            CfiArch::X86_64 => 7,
            CfiArch::I386 => 4,
        }
    }

    /// DWARF register number of an assembler register name (with or without
    /// `%`) or a plain register number, as GNU as accepts in CFI operands.
    fn dwarf_reg(self, name: &str) -> Result<u64, String> {
        let n = name.trim().trim_start_matches('%').to_ascii_lowercase();
        if let Ok(v) = n.parse::<u64>() {
            return Ok(v);
        }
        let num = |prefix: &str, base: u64, count: u64| -> Option<u64> {
            let idx: u64 = n.strip_prefix(prefix)?.parse().ok()?;
            (idx < count).then_some(base + idx)
        };
        let r = match self {
            CfiArch::X86_64 => match n.as_str() {
                "rax" | "eax" => Some(0),
                "rdx" | "edx" => Some(1),
                "rcx" | "ecx" => Some(2),
                "rbx" | "ebx" => Some(3),
                "rsi" | "esi" => Some(4),
                "rdi" | "edi" => Some(5),
                "rbp" | "ebp" => Some(6),
                "rsp" | "esp" => Some(7),
                "rip" => Some(16),
                "rflags" | "eflags" => Some(49),
                "es" => Some(50),
                "cs" => Some(51),
                "ss" => Some(52),
                "ds" => Some(53),
                "fs" => Some(54),
                "gs" => Some(55),
                "fs.base" => Some(58),
                "gs.base" => Some(59),
                _ => {
                    // r8..r15 (and their d/w/b forms), xmm/st/mm banks.
                    let gpr = n
                        .strip_prefix('r')
                        .map(|s| s.trim_end_matches(['d', 'w', 'b']))
                        .and_then(|s| s.parse::<u64>().ok())
                        .filter(|v| (8..=15).contains(v));
                    gpr.or_else(|| num("xmm", 17, 16))
                        .or_else(|| num("st", 33, 8))
                        .or_else(|| num("mm", 41, 8))
                }
            },
            CfiArch::I386 => match n.as_str() {
                "eax" => Some(0),
                "ecx" => Some(1),
                "edx" => Some(2),
                "ebx" => Some(3),
                "esp" => Some(4),
                "ebp" => Some(5),
                "esi" => Some(6),
                "edi" => Some(7),
                "eip" => Some(8),
                "eflags" => Some(9),
                _ => num("st", 11, 8)
                    .or_else(|| num("xmm", 21, 8))
                    .or_else(|| num("mm", 29, 8)),
            },
        };
        let r = r.or_else(|| {
            // `%st(3)` spelling.
            let inner = n.strip_prefix("st(")?.strip_suffix(')')?;
            let idx: u64 = inner.parse().ok()?;
            let base = if self == CfiArch::X86_64 { 33 } else { 11 };
            (idx < 8).then_some(base + idx)
        });
        r.ok_or_else(|| format!("bad register expression `{}` in CFI directive", name.trim()))
    }
}

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
    /// (location label, encoded instruction bytes as `.byte` operands)
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

type SectionKey = (String, Option<String>);

fn section_key(dir: &SectionDirective) -> SectionKey {
    (dir.name.clone(), dir.comdat_group.clone())
}

/// Items that never emit bytes: a CFI location label placed before them
/// still marks the same address as one placed after them.
fn emits_no_bytes(item: &AsmItem) -> bool {
    matches!(
        item,
        AsmItem::Label(_)
            | AsmItem::Global(_)
            | AsmItem::Weak(_)
            | AsmItem::Hidden(_)
            | AsmItem::Protected(_)
            | AsmItem::Internal(_)
            | AsmItem::SymbolType(_, _)
            | AsmItem::Size(_, _)
            | AsmItem::Cfi(_)
            | AsmItem::File(_, _)
            | AsmItem::Loc(_, _, _)
            | AsmItem::OptionDirective(_)
            | AsmItem::Symver(_, _, _)
            | AsmItem::Set(_, _)
            | AsmItem::Empty
    )
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

fn encode_offset(reg: u64, off: i64, arch: CfiArch) -> Result<Vec<InsnPart>, String> {
    let f = factored(off, arch.data_align(), ".cfi_offset")?;
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

fn encode_def_cfa_offset(off: i64, arch: CfiArch) -> Result<Vec<InsnPart>, String> {
    Ok(if off >= 0 {
        vec![InsnPart::Byte(0x0e), InsnPart::Uleb(off as u64)]
    } else {
        // DW_CFA_def_cfa_offset_sf takes a factored operand.
        let f = factored(off, arch.data_align(), ".cfi_def_cfa_offset")?;
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

fn encoded_ptr_size(enc: u8, arch: CfiArch) -> i64 {
    match enc & 0x0f {
        0x00 => arch.ptr_size(),
        0x02 | 0x0a => 2,
        0x03 | 0x0b => 4,
        _ => 8,
    }
}

/// Emit one encoded pointer as a data directive.
fn write_encoded_ptr(out: &mut String, p: &EncodedPtr, arch: CfiArch) {
    let dir = match encoded_ptr_size(p.enc, arch) {
        2 => ".short",
        4 => ".long",
        _ => ".quad",
    };
    if p.enc & 0x70 == 0x10 {
        let _ = writeln!(out, "{} {} - .", dir, p.sym);
    } else {
        let _ = writeln!(out, "{} {}", dir, p.sym);
    }
}

fn write_insn(out: &mut String, parts: &[InsnPart]) {
    for p in parts {
        let _ = match p {
            InsnPart::Byte(b) => writeln!(out, ".byte {:#x}", b),
            InsnPart::Uleb(v) => writeln!(out, ".uleb128 {}", v),
            InsnPart::Sleb(v) => writeln!(out, ".sleb128 {}", v),
        };
    }
}

/// Replace the CFI directives of `items` by location labels and append the
/// `.eh_frame` data they describe.
pub fn lower_cfi(items: Vec<AsmItem>, arch: CfiArch) -> Result<Vec<AsmItem>, String> {
    if !items.iter().any(|i| matches!(i, AsmItem::Cfi(_))) {
        return Ok(items);
    }
    let mut out: Vec<AsmItem> = Vec::with_capacity(items.len() + 16);
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
        match &item {
            AsmItem::Section(dir) => {
                prev_section = cur_section.take();
                cur_section = Some(section_key(dir));
            }
            AsmItem::PushSection(dir) => {
                section_stack.push((cur_section.clone(), prev_section.clone()));
                prev_section = cur_section.take();
                cur_section = Some(section_key(dir));
            }
            AsmItem::PopSection => {
                if let Some((c, p)) = section_stack.pop() {
                    cur_section = c;
                    prev_section = p;
                }
            }
            AsmItem::Previous => std::mem::swap(&mut cur_section, &mut prev_section),
            _ => {}
        }
        let AsmItem::Cfi(dir) = item else {
            if !emits_no_bytes(&item) {
                here = None;
            }
            out.push(item);
            continue;
        };

        // Location label for this directive.
        let loc = match &here {
            Some(l) => l.clone(),
            None => {
                let l = new_label(&mut label_seq);
                out.push(AsmItem::Label(l.clone()));
                here = Some(l.clone());
                l
            }
        };

        let (name, args): (String, String) = match &dir {
            CfiDirective::StartProc => (".cfi_startproc".into(), String::new()),
            CfiDirective::EndProc => (".cfi_endproc".into(), String::new()),
            CfiDirective::DefCfaOffset(v) => (".cfi_def_cfa_offset".into(), v.to_string()),
            CfiDirective::DefCfaRegister(r) => (".cfi_def_cfa_register".into(), r.clone()),
            CfiDirective::Offset(r, v) => (".cfi_offset".into(), format!("{}, {}", r, v)),
            CfiDirective::Other(line) => {
                let t = line.trim();
                match t.find(char::is_whitespace) {
                    Some(i) => (t[..i].to_string(), t[i..].trim().to_string()),
                    None => (t.to_string(), String::new()),
                }
            }
        };
        let name = name.as_str();

        match name {
            ".cfi_sections" => {
                emit_eh_frame = split_args(&args).iter().any(|s| *s == ".eh_frame");
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
                    ra_column: arch.default_ra_column(),
                    personality: None,
                    lsda: None,
                    insns: Vec::new(),
                    cfa_offset: if simple { 0 } else { arch.ptr_size() },
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
            let f = open.take().expect("checked above");
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
        let a = split_args(&args);
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
                let reg = arch.dwarf_reg(a[0])?;
                let off = parse_int(a[1])?;
                f.cfa_offset = off;
                if off >= 0 {
                    vec![
                        InsnPart::Byte(0x0c),
                        InsnPart::Uleb(reg),
                        InsnPart::Uleb(off as u64),
                    ]
                } else {
                    let fo = factored(off, arch.data_align(), name)?;
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
                encode_def_cfa_offset(off, arch)?
            }
            ".cfi_adjust_cfa_offset" => {
                need(1)?;
                f.cfa_offset += parse_int(a[0])?;
                encode_def_cfa_offset(f.cfa_offset, arch)?
            }
            ".cfi_def_cfa_register" => {
                need(1)?;
                vec![InsnPart::Byte(0x0d), InsnPart::Uleb(arch.dwarf_reg(a[0])?)]
            }
            ".cfi_offset" => {
                need(2)?;
                encode_offset(arch.dwarf_reg(a[0])?, parse_int(a[1])?, arch)?
            }
            ".cfi_rel_offset" => {
                // Offset relative to the CFA register's current value, which
                // sits `cfa_offset` bytes below the CFA.
                need(2)?;
                let off = parse_int(a[1])? - f.cfa_offset;
                encode_offset(arch.dwarf_reg(a[0])?, off, arch)?
            }
            ".cfi_val_offset" => {
                need(2)?;
                let reg = arch.dwarf_reg(a[0])?;
                let fo = factored(parse_int(a[1])?, arch.data_align(), name)?;
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
                    InsnPart::Uleb(arch.dwarf_reg(a[0])?),
                    InsnPart::Uleb(arch.dwarf_reg(a[1])?),
                ]
            }
            ".cfi_restore" => {
                if a.is_empty() {
                    return Err(".cfi_restore expects register operand(s)".to_string());
                }
                let mut v = Vec::new();
                for r in &a {
                    let reg = arch.dwarf_reg(r)?;
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
                    v.extend([InsnPart::Byte(op), InsnPart::Uleb(arch.dwarf_reg(r)?)]);
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
                    let byte = u8::try_from(x & 0xff)
                        .map_err(|_| format!("bad .cfi_escape byte `{}`", b))?;
                    v.push(InsnPart::Byte(byte));
                }
                v
            }
            ".cfi_signal_frame" => {
                need(0)?;
                f.signal_frame = true;
                continue;
            }
            ".cfi_return_column" => {
                need(1)?;
                f.ra_column = arch.dwarf_reg(a[0])?;
                continue;
            }
            ".cfi_personality" => {
                f.personality = parse_encoded_ptr(&args, name)?;
                continue;
            }
            ".cfi_lsda" => {
                f.lsda = parse_encoded_ptr(&args, name)?;
                continue;
            }
            ".cfi_label" => {
                need(1)?;
                out.push(AsmItem::Label(a[0].to_string()));
                continue;
            }
            other => {
                // Non-x86 or never-emitted forms (.cfi_window_save,
                // .cfi_val_encoded_addr, .cfi_personality_id, ...): refuse
                // loudly instead of writing unwind info that lies.
                return Err(format!("unsupported CFI directive {}", other));
            }
        };
        f.insns.push((loc, insn));
    }

    if open.is_some() {
        return Err("open CFI at the end of file; missing .cfi_endproc directive".to_string());
    }
    if !emit_eh_frame || fdes.is_empty() {
        return Ok(out);
    }
    let (text, advances) = render_eh_frame(&fdes, arch);
    let parsed = parse_asm(&text).map_err(|e| format!("internal .eh_frame lowering: {}", e))?;
    for item in parsed {
        match item {
            AsmItem::Label(l) if l.starts_with(ADVANCE_MARK) => {
                let (from, to) = advances[l[ADVANCE_MARK.len()..]
                    .parse::<usize>()
                    .map_err(|_| format!("internal .eh_frame lowering: bad marker {}", l))?]
                .clone();
                out.push(AsmItem::CfaAdvance { from, to });
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

/// Stands for the location advance with the given index in the rendered
/// `.eh_frame` text; replaced by `AsmItem::CfaAdvance` after parsing. In the
/// lowering's own reserved `.L__lccc_cfi*` label namespace.
const ADVANCE_MARK: &str = ".L__lccc_cfi_adv";

/// Render the `.eh_frame` section as assembler text, with each location
/// advance as an `ADVANCE_MARK<n>:` label; `n` indexes the returned
/// `(from, to)` label pairs.
///
/// Layout as GNU as's `cfi_finish`: each CIE right before the first FDE
/// that uses it, CIEs and FDEs padded with `DW_CFA_nop` to 4 bytes (the
/// size of every field they hold), and only the last FDE to the pointer
/// size, which also gives the section its alignment. Nothing is padded in
/// front of the first entry: hand-written `.eh_frame` data earlier in the
/// file (glibc's signal-restorer asm) must not be followed by zero bytes,
/// which an unwinder reads as the terminator.
fn render_eh_frame(fdes: &[Fde], arch: CfiArch) -> (String, Vec<(String, String)>) {
    let mut advances: Vec<(String, String)> = Vec::new();
    let last_align_log2 = if arch == CfiArch::X86_64 { 3 } else { 2 };
    let mut s = String::new();
    let _ = writeln!(s, ".section .eh_frame,\"a\",@progbits");

    let mut cies: FxHashMap<CieKey, String> = FxHashMap::default();
    for (i, f) in fdes.iter().enumerate() {
        if !cies.contains_key(&f.cie) {
            let l = format!(".L__lccc_cie{}", cies.len());
            write_cie(&mut s, &l, &f.cie, arch);
            cies.insert(f.cie.clone(), l);
        }
        let l = format!(".L__lccc_fde{}", i);
        let _ = writeln!(s, "{}:", l);
        let _ = writeln!(s, ".long {}_end - {}_body", l, l);
        let _ = writeln!(s, "{}_body:", l);
        // CIE pointer: distance from this field back to the CIE's start.
        let _ = writeln!(s, ".long {}_body - {}", l, cies[&f.cie]);
        let _ = writeln!(s, ".long {} - .", f.start);
        let _ = writeln!(s, ".long {} - {}", f.end, f.start);
        match &f.lsda {
            Some(p) => {
                let _ = writeln!(s, ".uleb128 {}", encoded_ptr_size(p.enc, arch));
                write_encoded_ptr(&mut s, p, arch);
            }
            None => {
                let _ = writeln!(s, ".uleb128 0");
            }
        }
        let mut prev = f.start.as_str();
        for (loc, parts) in &f.insns {
            if loc != prev {
                let _ = writeln!(s, "{}{}:", ADVANCE_MARK, advances.len());
                advances.push((prev.to_string(), loc.clone()));
                prev = loc;
            }
            write_insn(&mut s, parts);
        }
        let align_log2 = if i + 1 == fdes.len() {
            last_align_log2
        } else {
            2
        };
        let _ = writeln!(s, ".p2align {}", align_log2);
        let _ = writeln!(s, "{}_end:", l);
    }
    (s, advances)
}

fn write_cie(s: &mut String, l: &str, key: &CieKey, arch: CfiArch) {
    let mut aug = String::from("z");
    let mut aug_len = 0i64;
    if let Some(p) = &key.personality {
        aug.push('P');
        aug_len += 1 + encoded_ptr_size(p.enc, arch);
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
    let _ = writeln!(s, "{}:", l);
    let _ = writeln!(s, ".long {}_end - {}_body", l, l);
    let _ = writeln!(s, "{}_body:", l);
    let _ = writeln!(s, ".long 0");
    // Version 1 stores the return column in one byte; version 3 (GNU as
    // behaviour for columns > 255) uses ULEB128.
    let version = if key.ra_column > 255 { 3 } else { 1 };
    let _ = writeln!(s, ".byte {}", version);
    let _ = writeln!(s, ".asciz \"{}\"", aug);
    let _ = writeln!(s, ".uleb128 1");
    let _ = writeln!(s, ".sleb128 {}", arch.data_align());
    if version == 1 {
        let _ = writeln!(s, ".byte {}", key.ra_column);
    } else {
        let _ = writeln!(s, ".uleb128 {}", key.ra_column);
    }
    let _ = writeln!(s, ".uleb128 {}", aug_len);
    if let Some(p) = &key.personality {
        let _ = writeln!(s, ".byte {:#x}", p.enc);
        write_encoded_ptr(s, p, arch);
    }
    if let Some(enc) = key.lsda_enc {
        let _ = writeln!(s, ".byte {:#x}", enc);
    }
    let _ = writeln!(s, ".byte 0x1b"); // FDE pointers: pcrel | sdata4
    if !key.simple {
        // CFA = sp + ptr_size; return address saved at CFA - ptr_size.
        write_insn(
            s,
            &[
                InsnPart::Byte(0x0c),
                InsnPart::Uleb(arch.sp_column()),
                InsnPart::Uleb(arch.ptr_size() as u64),
            ],
        );
        if let Ok(parts) = encode_offset(key.ra_column, arch.data_align(), arch) {
            write_insn(s, &parts);
        }
    }
    let _ = writeln!(s, ".p2align 2");
    let _ = writeln!(s, "{}_end:", l);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assemble `asm` with the integrated assembler for `arch` and return
    /// the raw bytes of its `.eh_frame` section (None when absent).
    fn eh_frame_of(asm: &str, arch: CfiArch) -> Option<Vec<u8>> {
        let path = std::env::temp_dir().join(format!(
            "lccc-cfi-test-{}-{:?}-{}.o",
            std::process::id(),
            arch,
            asm.len()
        ));
        let p = path.to_str().unwrap();
        match arch {
            CfiArch::X86_64 => crate::backend::x86::assembler::assemble(asm, p),
            CfiArch::I386 => crate::backend::i686::assembler::assemble(asm, p),
        }
        .expect("assembles");
        let elf = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        section_bytes(&elf, ".eh_frame")
    }

    /// Minimal ELF32/ELF64 little-endian section lookup by name.
    fn section_bytes(elf: &[u8], want: &str) -> Option<Vec<u8>> {
        let u16_at = |o: usize| u16::from_le_bytes([elf[o], elf[o + 1]]) as usize;
        let u32_at = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap()) as usize;
        let u64_at = |o: usize| u64::from_le_bytes(elf[o..o + 8].try_into().unwrap()) as usize;
        let is64 = elf[4] == 2;
        let (shoff, shentsize, shnum, shstrndx) = if is64 {
            (u64_at(0x28), u16_at(0x3a), u16_at(0x3c), u16_at(0x3e))
        } else {
            (u32_at(0x20), u16_at(0x2e), u16_at(0x30), u16_at(0x32))
        };
        let hdr = |i: usize| -> (usize, usize, usize) {
            let b = shoff + i * shentsize;
            if is64 {
                (u32_at(b), u64_at(b + 0x18), u64_at(b + 0x20))
            } else {
                (u32_at(b), u32_at(b + 0x10), u32_at(b + 0x14))
            }
        };
        let (_, stroff, _) = hdr(shstrndx);
        (0..shnum).find_map(|i| {
            let (name, off, size) = hdr(i);
            let n = &elf[stroff + name..];
            let n = &n[..n.iter().position(|&c| c == 0).unwrap()];
            (n == want.as_bytes()).then(|| elf[off..off + size].to_vec())
        })
    }

    const X64_FN: &str = "\
.text
.globl f
.type f, @function
f:
.cfi_startproc
pushq %rbx
.cfi_def_cfa_offset 16
.cfi_offset %rbx, -16
popq %rbx
.cfi_def_cfa_offset 8
ret
.cfi_endproc
.size f, .-f
";

    #[test]
    fn x86_64_cie_and_fde_match_gnu_as_layout() {
        let eh = eh_frame_of(X64_FN, CfiArch::X86_64).expect(".eh_frame emitted");
        // GNU as 2.47 (`as --64`), byte for byte: CIE "zR" (data align -8,
        // RA column 16, def_cfa rsp+8, rip at cfa-8, nop-padded to 4), then
        // the FDE: pc_range 3, advance_loc 1; def_cfa_offset 16; rbx(3) at
        // cfa-16; advance_loc 1; def_cfa_offset 8; padded to 8 as the last.
        let gnu_as: &[u8] = &[
            0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x00, 0x01, 0x78,
            0x10, 0x01, 0x1b, 0x0c, 0x07, 0x08, 0x90, 0x01, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x00,
            0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x41,
            0x0e, 0x10, 0x83, 0x02, 0x41, 0x0e, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(eh, gnu_as);
    }

    #[test]
    fn i386_uses_its_own_register_numbers_and_alignment() {
        let asm = "\
.text
f:
.cfi_startproc
pushl %ebx
.cfi_def_cfa_offset 8
.cfi_offset %ebx, -8
pushl %ebp
.cfi_def_cfa_offset 12
.cfi_offset %ebp, -12
movl %esp, %ebp
.cfi_def_cfa_register %ebp
leave
popl %ebx
ret
.cfi_endproc
";
        let eh = eh_frame_of(asm, CfiArch::I386).expect(".eh_frame emitted");
        // GNU as 2.47 (`as --32`), byte for byte: data align -4, RA column
        // 8 (eip), def_cfa esp(4)+4, eip at cfa-4; ebx(3) at cfa-8, ebp(5)
        // at cfa-12, CFA register ebp(5); everything padded to 4.
        let gnu_as: &[u8] = &[
            0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x00, 0x01, 0x7c,
            0x08, 0x01, 0x1b, 0x0c, 0x04, 0x04, 0x88, 0x01, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x00,
            0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x41,
            0x0e, 0x08, 0x83, 0x02, 0x41, 0x0e, 0x0c, 0x85, 0x03, 0x42, 0x0d, 0x05, 0x00, 0x00,
        ];
        assert_eq!(eh, gnu_as);
    }

    /// Location advances at every encoding boundary (63/64, 255/256,
    /// 65535/65536 bytes and zero), a second CIE emitted at its first use
    /// between FDEs, and only the last FDE padded to the pointer size.
    const ADVANCES: &str = "\
.text
f:
.cfi_startproc
.skip 0x3f
.cfi_def_cfa_offset 16
.skip 0x40
.cfi_def_cfa_offset 24
.skip 0xff
.cfi_def_cfa_offset 32
.skip 0x100
.cfi_def_cfa_offset 40
.skip 0xffff
.cfi_def_cfa_offset 48
.skip 0x10000
.cfi_def_cfa_offset 56
.cfi_def_cfa_offset 64
ret
.cfi_endproc
g:
.cfi_startproc
.cfi_signal_frame
nop
.cfi_def_cfa_offset 16
ret
.cfi_endproc
h:
.cfi_startproc
ret
.cfi_endproc
";

    #[test]
    fn advances_take_the_smallest_form_like_gnu_as() {
        // GNU as 2.47 output for ADVANCES, byte for byte. Advances:
        // 0x7f (loc 63), 02 40, 02 ff, 03 00 01, 03 ff ff, 04 00 00 01 00,
        // none for the zero distance; CIE "zRS" right before g's FDE.
        let gnu_as_64: &[u8] = &[
            0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x00, 0x01, 0x78,
            0x10, 0x01, 0x1b, 0x0c, 0x07, 0x08, 0x90, 0x01, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00,
            0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7e, 0x02, 0x02, 0x00, 0x00, 0x7f,
            0x0e, 0x10, 0x02, 0x40, 0x0e, 0x18, 0x02, 0xff, 0x0e, 0x20, 0x03, 0x00, 0x01, 0x0e,
            0x28, 0x03, 0xff, 0xff, 0x0e, 0x30, 0x04, 0x00, 0x00, 0x01, 0x00, 0x0e, 0x38, 0x0e,
            0x40, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x53,
            0x00, 0x01, 0x78, 0x10, 0x01, 0x1b, 0x0c, 0x07, 0x08, 0x90, 0x01, 0x00, 0x10, 0x00,
            0x00, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
            0x00, 0x41, 0x0e, 0x10, 0x10, 0x00, 0x00, 0x00, 0x78, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let gnu_as_32: &[u8] = &[
            0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x00, 0x01, 0x7c,
            0x08, 0x01, 0x1b, 0x0c, 0x04, 0x04, 0x88, 0x01, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00,
            0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7e, 0x02, 0x02, 0x00, 0x00, 0x7f,
            0x0e, 0x10, 0x02, 0x40, 0x0e, 0x18, 0x02, 0xff, 0x0e, 0x20, 0x03, 0x00, 0x01, 0x0e,
            0x28, 0x03, 0xff, 0xff, 0x0e, 0x30, 0x04, 0x00, 0x00, 0x01, 0x00, 0x0e, 0x38, 0x0e,
            0x40, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x53,
            0x00, 0x01, 0x7c, 0x08, 0x01, 0x1b, 0x0c, 0x04, 0x04, 0x88, 0x01, 0x00, 0x10, 0x00,
            0x00, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x7e, 0x02, 0x02, 0x00, 0x02, 0x00, 0x00, 0x00,
            0x00, 0x41, 0x0e, 0x10, 0x10, 0x00, 0x00, 0x00, 0x78, 0x00, 0x00, 0x00, 0x80, 0x02,
            0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(eh_frame_of(ADVANCES, CfiArch::X86_64).unwrap(), gnu_as_64);
        assert_eq!(eh_frame_of(ADVANCES, CfiArch::I386).unwrap(), gnu_as_32);
    }

    #[test]
    fn no_cfi_means_no_eh_frame_and_untouched_items() {
        let asm = ".text\nf:\nret\n";
        assert!(eh_frame_of(asm, CfiArch::X86_64).is_none());
        let items = parse_asm(asm).unwrap();
        let n = items.len();
        assert_eq!(lower_cfi(items, CfiArch::X86_64).unwrap().len(), n);
    }

    #[test]
    fn debug_frame_only_sections_suppress_eh_frame() {
        let asm = format!(".cfi_sections .debug_frame\n{}", X64_FN);
        assert!(eh_frame_of(&asm, CfiArch::X86_64).is_none());
    }

    #[test]
    fn personality_and_lsda_select_zplr_cie() {
        let asm = "\
.text
f:
.cfi_startproc
.cfi_personality 0x9b,DW.ref.__gcc_personality_v0
.cfi_lsda 0x1b,.LLSDA0
ret
.cfi_endproc
.section .gcc_except_table,\"a\",@progbits
.LLSDA0:
.byte 0xff
.data
DW.ref.__gcc_personality_v0:
.quad __gcc_personality_v0
";
        let eh = eh_frame_of(asm, CfiArch::X86_64).expect(".eh_frame emitted");
        assert_eq!(&eh[8..14], b"\x01zPLR\0");
        // aug data length: P (1 + 4) + L (1) + R (1) = 7
        assert_eq!(eh[17], 7);
        assert_eq!(eh[18], 0x9b);
        assert_eq!(eh[23], 0x1b, "LSDA encoding");
        assert_eq!(eh[24], 0x1b, "FDE encoding");
    }

    #[test]
    fn malformed_regions_are_rejected() {
        let lower = |s: &str| lower_cfi(parse_asm(s).unwrap(), CfiArch::X86_64);
        assert!(
            lower("f:\n.cfi_startproc\nret\n")
                .unwrap_err()
                .contains("missing .cfi_endproc")
        );
        assert!(lower("f:\n.cfi_startproc\n.cfi_startproc\n").is_err());
        assert!(
            lower("f:\n.cfi_startproc\n.cfi_window_save\n.cfi_endproc\n")
                .unwrap_err()
                .contains("unsupported CFI directive")
        );
    }
}
