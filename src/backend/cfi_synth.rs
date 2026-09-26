//! Call frame information synthesized from the final x86 instruction stream.
//!
//! The x86-64 and i686 codegens emit only the `.cfi_startproc`/`.cfi_endproc`
//! delimiters; every other CFI directive is derived here, AFTER the text
//! peephole, by abstract interpretation of the stack pointer over the
//! function's final instructions (the approach of GCC's dwarf2cfi, which
//! derives CFI from the final insn stream rather than from the prologue
//! generator).
//!
//! Why after the peephole: the peephole deletes callee-saved pushes,
//! shrinks or removes frames and drops `movq %rsp, %rbp` pairs. CFI written
//! by the prologue generator either had to be patched by every such pass or
//! (the previous state) blocked those passes whenever a directive was in the
//! way; the x86-64 prologue also never described its callee-saved pushes at
//! all, so an unwinder restored the callee's values into the caller's
//! `%rbx`/`%r12`-`%r15`. Deriving the CFI from the final code makes it
//! exact by construction and makes the peephole's input identical with and
//! without unwind tables.
//!
//! Model: `sp_off` = CFA - %sp and `fp_off` = CFA - %fp when known; the CFA
//! rule is `sp + sp_off` or `fp + fp_off`. Saved registers are the pushes
//! (and `mov` stores) of callee-saved registers in the prologue region --
//! the leading run of frame-setup instructions, before any instruction can
//! have written such a register -- recorded at their CFA-relative slot. The
//! rule stays valid through the epilogue: a `pop` does not clobber the slot,
//! which is also how GCC describes x86 epilogues.
//!
//! Calls: the callee pops its return address. What some callees pop on
//! top of it (i686 `ret $N`: the SysV hidden struct-return pointer, the
//! stack arguments of fastcall) is invisible in the call's text, so the
//! codegen reports it (`CalleePops`); the pop is folded into the caller's
//! cleanup, and without it the CFA would stay N bytes too high.
//!
//! Control flow: the state at a label is derived, as in dwarf2cfi, only
//! from states that are themselves derived from the entry along direct
//! edges -- fallthrough, `jmp`/`jcc`, a label named in user asm (asm goto),
//! `call .Llocal` (one slot deeper) -- to a fixed point; the first such
//! state fixes the label (codegen frames are path-independent at block
//! boundaries). What stays unreached is entered only indirectly: labels
//! whose address is taken (jump tables, computed goto, `@GOTOFF`) take the
//! frame of the function's indirect jumps into its body (`jmp *`, a `ret`
//! with the frame still set up as in the inline retpoline) when those
//! agree, anything else the post-prologue state. At each label whose state
//! differs from the state the directives emitted so far describe, explicit
//! directives re-establish it (`.cfi_def_cfa*`), the equivalent of GCC's
//! remember/restore pairs.
//!
//! User inline asm (`#APP`/`#NO_APP`) is not interpreted, as with GCC: its
//! own CFI directives are kept verbatim and applied to the model. If the CFA
//! becomes %sp-relative with an unknown offset (an unmodelled %sp write) the
//! function's remaining range until a known state is marked with
//! `.cfi_undefined` of the return-address column, which ends unwinding there
//! rather than describing a wrong frame; the corpus has no such function
//! (`LCCC_CFI_DEBUG=1` reports any).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::fmt::Write as _;

// One definition of the target's frame conventions, shared with the
// assembler's CFI lowering (DWARF register numbers, data alignment), so
// the synthesizer reads DWARF with the same tables the assembler writes it.
pub(crate) use crate::backend::x86::assembler::cfi::CfiArch;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CfaReg {
    Sp,
    Fp,
}

/// Frame state at a program point.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Frame {
    /// CFA - %sp, when known.
    sp_off: Option<i64>,
    /// CFA - %fp, when %fp holds a frame address.
    fp_off: Option<i64>,
    cfa: CfaReg,
}

impl Frame {
    fn entry(arch: CfiArch) -> Self {
        Frame {
            sp_off: Some(arch.slot()),
            fp_off: None,
            cfa: CfaReg::Sp,
        }
    }
    /// The CFA rule, or None when it cannot be expressed.
    fn rule(&self) -> Option<(CfaReg, i64)> {
        match self.cfa {
            CfaReg::Sp => self.sp_off.map(|o| (CfaReg::Sp, o)),
            CfaReg::Fp => self.fp_off.map(|o| (CfaReg::Fp, o)),
        }
    }
}

/// The CFA rule the directives emitted so far describe.
type Described = Option<(CfaReg, i64)>;

fn parse_imm(s: &str) -> Option<i64> {
    let s = s.trim().strip_prefix('$')?;
    if let Some(h) = s.strip_prefix("0x") {
        i64::from_str_radix(h, 16).ok()
    } else if let Some(h) = s.strip_prefix("-0x") {
        i64::from_str_radix(h, 16).ok().map(|v| -v)
    } else {
        s.parse().ok()
    }
}

/// `N(%reg)` with a plain integer displacement.
fn parse_disp_reg(s: &str) -> Option<(i64, &str)> {
    let s = s.trim();
    let open = s.find('(')?;
    let inner = s[open + 1..].strip_suffix(')')?;
    if inner.contains(',') {
        return None;
    }
    let reg = inner.strip_prefix('%')?;
    let d = s[..open].trim();
    let disp = if d.is_empty() { 0 } else { d.parse().ok()? };
    Some((disp, reg))
}

fn split_operands(ops: &str) -> Vec<&str> {
    // AT&T operands: split on commas outside parentheses.
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in ops.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(ops[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = ops[start..].trim();
    if !last.is_empty() {
        out.push(last);
    }
    out
}

/// Does `op` name `reg` (any width alias is NOT considered: only the full
/// register is a stack/frame pointer write we model)?
fn is_reg(op: &str, reg: &str) -> bool {
    op.strip_prefix('%') == Some(reg)
}

/// Mentions %sp as a register operand (any width) -- used to detect
/// unmodelled stack pointer writes.
fn writes_reg_family(dst: &str, arch: CfiArch, full: &str) -> bool {
    let Some(r) = dst.strip_prefix('%') else {
        return false;
    };
    match arch {
        CfiArch::X86_64 => {
            r == full
                || r == &full.replacen('r', "e", 1)
                || (full == "rsp" && (r == "sp" || r == "spl"))
                || (full == "rbp" && (r == "bp" || r == "bpl"))
        }
        CfiArch::I386 => r == full || r == &full[1..],
    }
}

enum Flow {
    Next,
    /// No fallthrough (`ret`, `jmp`, `ud2`).
    Stop,
}

struct Insn<'a> {
    mnem: &'a str,
    ops: Vec<&'a str>,
}

fn parse_insn(t: &str) -> Option<Insn<'_>> {
    if t.is_empty() || t.starts_with('.') || t.starts_with('#') || t.ends_with(':') {
        return None;
    }
    let t = t.split('#').next().unwrap_or("").trim();
    fn split(t: &str) -> (&str, &str) {
        match t.find(|c: char| c.is_whitespace() || c == ';') {
            Some(i) => (&t[..i], t[i..].trim_start_matches(';').trim()),
            None => (t, ""),
        }
    }
    let (mut mnem, mut rest) = split(t);
    // Classify the instruction BEHIND semantic-neutral prefixes: `rep ret` /
    // `repz ret` (the AMD K8/K10 return idiom) is a return, `notrack jmp *%rax`
    // (-fcf-protection jump tables) and `bnd jmp` indirect jumps, and
    // `cs jmp __x86_indirect_thunk_rax` (-mindirect-branch-cs-prefix) a tail
    // call. Left as the mnemonic, each fell through as a no-op and pushed the
    // post-return frame into the next block. Only prefixes that change neither
    // the stack effect nor the control flow qualify: `data16` is excluded
    // (`data16 push` pushes two bytes, not a slot).
    while !rest.is_empty()
        && matches!(
            mnem,
            "rep"
                | "repe"
                | "repz"
                | "repne"
                | "repnz"
                | "bnd"
                | "notrack"
                | "lock"
                | "xacquire"
                | "xrelease"
                | "cs"
                | "ds"
                | "es"
                | "ss"
                | "fs"
                | "gs"
        )
    {
        (mnem, rest) = split(rest);
    }
    Some(Insn {
        mnem,
        ops: split_operands(rest),
    })
}

/// Apply one instruction to the frame model. Returns the control-flow
/// effect and, for direct branches, the target label.
fn step<'a>(
    f: &mut Frame,
    insn: &Insn<'a>,
    arch: CfiArch,
    call_pop: Option<i64>,
    unknown: &mut bool,
) -> (Flow, Option<&'a str>) {
    let slot = arch.slot();
    let sfx = arch.suffix();
    let m = insn.mnem;
    let ops = &insn.ops;
    let sp = arch.sp();
    let fp = arch.fp();
    let bump = |f: &mut Frame, d: i64| {
        f.sp_off = f.sp_off.map(|o| o + d);
    };

    // Control flow.
    if m == "ret" || m == "retq" || m == "retl" || m == "ud2" || m == "hlt" {
        return (Flow::Stop, None);
    }
    if m == "jmp" || m == "jmpq" || m == "jmpl" {
        let target = ops.first().copied().filter(|o| !o.starts_with('*'));
        return (Flow::Stop, target);
    }
    if m.starts_with('j') && ops.len() == 1 && !ops[0].starts_with('*') {
        // jcc / jrcxz / loop forms all start with 'j' except loop*.
        return (Flow::Next, Some(ops[0]));
    }

    let push_like = m == format!("push{}", sfx) || m == "push";
    let pop_like = m == format!("pop{}", sfx) || m == "pop";
    if push_like || m == format!("pushf{}", sfx) || m == "pushf" {
        bump(f, slot);
        return (Flow::Next, None);
    }
    if pop_like {
        let dst = ops.first().copied().unwrap_or("");
        bump(f, -slot);
        if is_reg(dst, fp) {
            // %fp reloaded from the stack: no longer a frame address.
            f.fp_off = None;
            if f.cfa == CfaReg::Fp {
                f.cfa = CfaReg::Sp;
            }
        } else if is_reg(dst, sp) {
            f.sp_off = None;
        }
        return (Flow::Next, None);
    }
    if m == format!("popf{}", sfx) || m == "popf" {
        bump(f, -slot);
        return (Flow::Next, None);
    }
    if arch == CfiArch::I386 && (m == "pushal" || m == "pusha") {
        bump(f, 32);
        return (Flow::Next, None);
    }
    if arch == CfiArch::I386 && (m == "popal" || m == "popa") {
        bump(f, -32);
        return (Flow::Next, None);
    }
    if m == "leave" || m == format!("leave{}", sfx) {
        // %sp = %fp; pop %fp.
        f.sp_off = f.fp_off.map(|o| o - slot);
        f.fp_off = None;
        f.cfa = CfaReg::Sp;
        return (Flow::Next, None);
    }
    if m == "call" || m == format!("call{}", sfx) {
        // The callee pops its return address, plus whatever its convention
        // makes it pop (`ret $N`); `None` when that is not known.
        match call_pop {
            Some(n) => bump(f, -n),
            None => {
                f.sp_off = None;
                if f.cfa == CfaReg::Sp {
                    *unknown = true;
                }
            }
        }
        return (Flow::Next, None);
    }

    let Some(&dst) = ops.last() else {
        return (Flow::Next, None);
    };
    let add = format!("add{}", sfx);
    let sub = format!("sub{}", sfx);
    let mov = format!("mov{}", sfx);
    let lea = format!("lea{}", sfx);
    if is_reg(dst, sp) && ops.len() == 2 {
        let src = ops[0];
        if m == add || m == sub {
            if let Some(n) = parse_imm(src) {
                bump(f, if m == sub { n } else { -n });
                return (Flow::Next, None);
            }
        } else if m == lea {
            if let Some((d, base)) = parse_disp_reg(src) {
                if base == sp {
                    bump(f, -d);
                    return (Flow::Next, None);
                }
                if base == fp {
                    f.sp_off = f.fp_off.map(|o| o - d);
                    return (Flow::Next, None);
                }
            }
        } else if m == mov && is_reg(src, fp) {
            f.sp_off = f.fp_off;
            return (Flow::Next, None);
        }
        // Any other write to %sp: offset unknown from here on.
        f.sp_off = None;
        if f.cfa == CfaReg::Sp {
            *unknown = true;
        }
        return (Flow::Next, None);
    }
    if writes_reg_family(dst, arch, sp) && !is_cmp_like(m) {
        f.sp_off = None;
        if f.cfa == CfaReg::Sp {
            *unknown = true;
        }
        return (Flow::Next, None);
    }
    if is_reg(dst, fp) && ops.len() == 2 && m == mov && is_reg(ops[0], sp) {
        // Frame pointer established: CFA moves to %fp.
        f.fp_off = f.sp_off;
        if f.cfa == CfaReg::Sp && f.fp_off.is_some() {
            f.cfa = CfaReg::Fp;
        }
        return (Flow::Next, None);
    }
    if writes_reg_family(dst, arch, fp) && !is_cmp_like(m) {
        f.fp_off = None;
        if f.cfa == CfaReg::Fp {
            *unknown = true;
        }
    }
    (Flow::Next, None)
}

/// Instructions whose last operand is read, not written.
fn is_cmp_like(m: &str) -> bool {
    const READ_ONLY: &[&str] = &[
        "cmp", "test", "push", "ucomis", "comis", "vucomis", "vcomis", "ptest", "vptest",
    ];
    let bit_test = m.starts_with("bt")
        && !m.starts_with("bts")
        && !m.starts_with("btr")
        && !m.starts_with("btc");
    bit_test || READ_ONLY.iter().any(|p| m.starts_with(p))
}

/// The symbol-like tokens of an assembly line.
fn tokens(t: &str) -> impl Iterator<Item = &str> {
    t.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$'))
}

/// The tokens of `t` that name one of `labels`.
fn names_label<'a>(t: &'a str, labels: &FxHashSet<&str>) -> Vec<&'a str> {
    tokens(t).filter(|tok| labels.contains(tok)).collect()
}

/// The register an i386 PIC-setup instruction writes: `call
/// __x86.get_pc_thunk.R` loads %eR; `addl $_GLOBAL_OFFSET_TABLE_, %eR`
/// then completes the GOT pointer in a register a thunk loaded.
fn pic_setup_reg(insn: &Insn<'_>, arch: CfiArch, thunked: &[&str]) -> Option<&'static str> {
    if arch != CfiArch::I386 || insn.ops.is_empty() {
        return None;
    }
    let reg32 = |r: &str| -> Option<&'static str> {
        Some(match r {
            "ax" | "eax" => "eax",
            "bx" | "ebx" => "ebx",
            "cx" | "ecx" => "ecx",
            "dx" | "edx" => "edx",
            "si" | "esi" => "esi",
            "di" | "edi" => "edi",
            "bp" | "ebp" => "ebp",
            _ => return None,
        })
    };
    if insn.mnem == "call" || insn.mnem == "calll" {
        return insn.ops[0]
            .strip_prefix("__x86.get_pc_thunk.")
            .and_then(reg32);
    }
    if insn.mnem == "addl" && insn.ops.len() == 2 && insn.ops[0] == "$_GLOBAL_OFFSET_TABLE_" {
        return insn.ops[1]
            .strip_prefix('%')
            .and_then(reg32)
            .filter(|r| thunked.contains(r));
    }
    None
}

/// Prologue-region instructions: frame setup that cannot write a
/// callee-saved register other than establishing %fp from %sp.
fn is_prologue_insn(insn: &Insn<'_>, arch: CfiArch) -> bool {
    let sfx = arch.suffix();
    let m = insn.mnem;
    if m == "endbr64" || m == "endbr32" || m == "nop" || m.starts_with("nop") {
        return true;
    }
    if m == format!("push{}", sfx) {
        return insn.ops.len() == 1 && insn.ops[0].starts_with('%');
    }
    if insn.ops.len() == 2 && is_reg(insn.ops[1], arch.sp()) {
        let sub = format!("sub{}", sfx);
        return m == sub && parse_imm(insn.ops[0]).is_some();
    }
    if m == format!("mov{}", sfx) && insn.ops.len() == 2 {
        // %sp -> %fp, or a callee-saved register stored to a frame slot.
        if is_reg(insn.ops[0], arch.sp()) && is_reg(insn.ops[1], arch.fp()) {
            return true;
        }
        if let Some(r) = insn.ops[0].strip_prefix('%') {
            if arch.is_callee_saved(r) {
                if let Some((_, base)) = parse_disp_reg(insn.ops[1]) {
                    return base == arch.sp() || base == arch.fp();
                }
            }
        }
    }
    false
}

fn reg_name(r: CfaReg, arch: CfiArch) -> &'static str {
    match r {
        CfaReg::Sp => arch.sp(),
        CfaReg::Fp => arch.fp(),
    }
}

/// Directives turning the described rule `from` into `to`.
fn transition(out: &mut String, from: Described, to: Described, arch: CfiArch) {
    if from == to {
        return;
    }
    match (from, to) {
        (_, None) => {
            let _ = writeln!(out, "    .cfi_undefined %{}", arch.ra());
        }
        (Some((fr, _)), Some((tr, to_off))) if fr == tr => {
            let _ = writeln!(out, "    .cfi_def_cfa_offset {}", to_off);
        }
        (Some((_, fo)), Some((tr, to_off))) if fo == to_off => {
            let _ = writeln!(out, "    .cfi_def_cfa_register %{}", reg_name(tr, arch));
        }
        (Some(_), Some((tr, to_off))) => {
            let _ = writeln!(out, "    .cfi_def_cfa %{}, {}", reg_name(tr, arch), to_off);
        }
        (None, Some((tr, to_off))) => {
            let _ = writeln!(out, "    .cfi_def_cfa %{}, {}", reg_name(tr, arch), to_off);
            let _ = writeln!(out, "    .cfi_offset %{}, {}", arch.ra(), -arch.slot());
        }
    }
}

/// What the emitted CFI currently says, as far as the model can express it.
///
/// `rule` is the register+offset rule the stream describes. `opaque` marks a
/// user rule the model has no form for -- a CFA in another register (GCC's
/// `.cfi_def_cfa %r10, 0` stack realignment), a `DW_CFA_def_cfa_expression`,
/// or escape bytes that cannot be decoded. After an opaque rule the next
/// transition must restate the whole rule: a relative `.cfi_def_cfa_offset`
/// is only defined when the current rule is register+offset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Stated {
    rule: Described,
    opaque: bool,
}

/// User CFI state inside one function: the stated rule plus the
/// `.cfi_remember_state` stack the user's directives push onto.
struct UserCfi {
    stated: Stated,
    remembered: Vec<Stated>,
}

impl UserCfi {
    fn new(rule: Described) -> Self {
        UserCfi {
            stated: Stated {
                rule,
                opaque: false,
            },
            remembered: Vec::new(),
        }
    }

    /// Emit the directives that make the stream describe `want`.
    fn describe(&mut self, out: &mut String, want: Described, arch: CfiArch) {
        if self.stated.opaque {
            match want {
                Some((r, o)) => {
                    let _ = writeln!(out, "    .cfi_def_cfa %{}, {}", reg_name(r, arch), o);
                }
                None => {
                    let _ = writeln!(out, "    .cfi_undefined %{}", arch.ra());
                }
            }
        } else {
            transition(out, self.stated.rule, want, arch);
        }
        self.stated = Stated {
            rule: want,
            opaque: false,
        };
    }

    fn opaque(&mut self) {
        self.stated.opaque = true;
    }

    /// `DW_CFA_def_cfa` / `.cfi_def_cfa` with a DWARF register number.
    fn def_cfa(&mut self, reg: Option<CfaReg>, off: i64) {
        match reg {
            Some(r) => {
                self.stated = Stated {
                    rule: Some((r, off)),
                    opaque: false,
                }
            }
            None => self.opaque(),
        }
    }

    fn def_cfa_register(&mut self, reg: Option<CfaReg>) {
        match (reg, self.stated) {
            (
                Some(r),
                Stated {
                    rule: Some((_, o)),
                    opaque: false,
                },
            ) => self.def_cfa(Some(r), o),
            _ => self.opaque(),
        }
    }

    fn def_cfa_offset(&mut self, off: i64) {
        match self.stated {
            Stated {
                rule: Some((r, _)),
                opaque: false,
            } => self.def_cfa(Some(r), off),
            _ => self.opaque(),
        }
    }

    fn remember(&mut self) {
        self.remembered.push(self.stated);
    }

    fn restore(&mut self) {
        // A state remembered outside the user's block is not ours to know.
        match self.remembered.pop() {
            Some(st) => self.stated = st,
            None => self.opaque(),
        }
    }

    /// Decode `.cfi_escape` bytes: the CFA-rule opcodes are applied, the
    /// register-rule opcodes skipped (the model describes only the CFA), and
    /// anything else -- an expression CFA, a location change inside the
    /// escape, an unknown or truncated opcode -- makes the rule opaque.
    fn escape(&mut self, bytes: &[u8], arch: CfiArch) {
        let dwarf_reg = |n: u64| -> Option<CfaReg> {
            if arch.dwarf_reg(arch.sp()).ok() == Some(n) {
                Some(CfaReg::Sp)
            } else if arch.dwarf_reg(arch.fp()).ok() == Some(n) {
                Some(CfaReg::Fp)
            } else {
                None
            }
        };
        let mut i = 0;
        let uleb = |i: &mut usize| -> Option<u64> {
            let (mut v, mut shift) = (0u64, 0u32);
            loop {
                let b = *bytes.get(*i)?;
                *i += 1;
                v |= u64::from(b & 0x7f).checked_shl(shift)?;
                if b & 0x80 == 0 {
                    return Some(v);
                }
                shift += 7;
            }
        };
        let sleb = |i: &mut usize| -> Option<i64> {
            let (mut v, mut shift) = (0i64, 0u32);
            loop {
                let b = *bytes.get(*i)?;
                *i += 1;
                v |= i64::from(b & 0x7f).checked_shl(shift)?;
                shift += 7;
                if b & 0x80 == 0 {
                    if shift < 64 && b & 0x40 != 0 {
                        v |= -1i64 << shift;
                    }
                    return Some(v);
                }
            }
        };
        let daf = arch.data_align();
        while i < bytes.len() {
            let op = bytes[i];
            i += 1;
            let ok = match op {
                0x00 => Some(()),
                0x0a => {
                    self.remember();
                    Some(())
                }
                0x0b => {
                    self.restore();
                    Some(())
                }
                0x0c => uleb(&mut i).and_then(|r| {
                    let o = uleb(&mut i)?;
                    self.def_cfa(dwarf_reg(r), i64::try_from(o).ok()?);
                    Some(())
                }),
                0x0d => uleb(&mut i).map(|r| self.def_cfa_register(dwarf_reg(r))),
                0x0e => uleb(&mut i).and_then(|o| {
                    self.def_cfa_offset(i64::try_from(o).ok()?);
                    Some(())
                }),
                0x12 => uleb(&mut i).and_then(|r| {
                    let o = sleb(&mut i)?.checked_mul(daf)?;
                    self.def_cfa(dwarf_reg(r), o);
                    Some(())
                }),
                0x13 => sleb(&mut i).and_then(|o| {
                    self.def_cfa_offset(o.checked_mul(daf)?);
                    Some(())
                }),
                // Register rules: operands skipped, CFA unchanged.
                0x06..=0x08 | 0x2e | 0x80..=0xbf => uleb(&mut i).map(|_| ()),
                0x05 | 0x09 | 0x14 | 0x2f => uleb(&mut i).and_then(|_| uleb(&mut i)).map(|_| ()),
                0x11 | 0x15 => uleb(&mut i).and_then(|_| sleb(&mut i)).map(|_| ()),
                0x10 | 0x16 => uleb(&mut i).and_then(|_| uleb(&mut i)).and_then(|n| {
                    i = i.checked_add(usize::try_from(n).ok()?)?;
                    (i <= bytes.len()).then_some(())
                }),
                0xc0..=0xff => Some(()),
                // 0x0f def_cfa_expression, location changes (0x01..=0x04,
                // 0x40..=0x7f) and unknown opcodes.
                _ => None,
            };
            if ok.is_none() {
                self.opaque();
                return;
            }
        }
    }
}

/// Apply a user CFI directive (inside `#APP`) to the stated rule, and align
/// the model with what the user established: their description is
/// authoritative inside their asm, so the directives after the block
/// describe changes relative to it.
fn apply_user_cfi(t: &str, f: &mut Frame, user: &mut UserCfi, arch: CfiArch) {
    let (name, args) = match t.find(char::is_whitespace) {
        Some(i) => (&t[..i], t[i..].trim()),
        None => (t, ""),
    };
    let parse = |s: &str| crate::backend::asm_expr::parse_integer_expr(s.trim()).ok();
    // `Err` = not a register the model tracks; the rule becomes opaque.
    let reg_of = |s: &str| -> Option<CfaReg> {
        let n = arch.dwarf_reg(s).ok()?;
        if arch.dwarf_reg(arch.sp()).ok() == Some(n) {
            Some(CfaReg::Sp)
        } else if arch.dwarf_reg(arch.fp()).ok() == Some(n) {
            Some(CfaReg::Fp)
        } else {
            None
        }
    };
    match name {
        ".cfi_def_cfa_offset" => match parse(args) {
            Some(o) => user.def_cfa_offset(o),
            None => user.opaque(),
        },
        ".cfi_adjust_cfa_offset" => match (parse(args), user.stated) {
            (
                Some(d),
                Stated {
                    rule: Some((_, o)),
                    opaque: false,
                },
            ) => user.def_cfa_offset(o + d),
            _ => user.opaque(),
        },
        ".cfi_def_cfa_register" => user.def_cfa_register(reg_of(args)),
        ".cfi_def_cfa" => {
            let mut it = args.splitn(2, ',');
            match (it.next(), it.next().and_then(parse)) {
                (Some(r), Some(o)) => user.def_cfa(reg_of(r), o),
                _ => user.opaque(),
            }
        }
        ".cfi_remember_state" => user.remember(),
        ".cfi_restore_state" => user.restore(),
        ".cfi_escape" => {
            let bytes: Option<Vec<u8>> = args
                .split(',')
                .map(|b| parse(b).and_then(|v| u8::try_from(v).ok()))
                .collect();
            match bytes {
                Some(b) => user.escape(&b, arch),
                None => user.opaque(),
            }
        }
        // Register save/restore rules, `.cfi_signal_frame`, ...: the model
        // describes only the CFA.
        _ => return,
    }
    if let Stated {
        rule: Some((r, o)),
        opaque: false,
    } = user.stated
    {
        f.cfa = r;
        match r {
            CfaReg::Sp => f.sp_off = Some(o),
            CfaReg::Fp => f.fp_off = Some(o),
        }
    }
}

/// Section tracker mirroring the assembler's `.section`/`.previous`/
/// `.pushsection`/`.popsection` semantics by section name.
#[derive(Clone)]
struct Sections {
    cur: String,
    prev: String,
    stack: Vec<(String, String)>,
}

impl Sections {
    fn new() -> Self {
        Sections {
            cur: ".text".to_string(),
            prev: ".text".to_string(),
            stack: Vec::new(),
        }
    }
    /// Update for directive line `t`; returns true if it was a section
    /// directive.
    fn apply(&mut self, t: &str) -> bool {
        if t == ".previous" {
            std::mem::swap(&mut self.cur, &mut self.prev);
            return true;
        }
        if t == ".popsection" {
            if let Some((c, p)) = self.stack.pop() {
                self.cur = c;
                self.prev = p;
            }
            return true;
        }
        let named = if matches!(t, ".text" | ".data" | ".bss" | ".rodata") {
            Some(t)
        } else {
            t.strip_prefix(".section")
                .or_else(|| t.strip_prefix(".pushsection"))
                .filter(|r| r.starts_with([' ', '\t']))
                .map(|r| r.split(',').next().unwrap_or("").trim())
        };
        let Some(name) = named else {
            return false;
        };
        if t.starts_with(".pushsection") {
            self.stack.push((self.cur.clone(), self.prev.clone()));
        }
        self.prev = std::mem::replace(&mut self.cur, name.to_string());
        true
    }
}

/// Synthesize CFI for every function in `asm` delimited by bare
/// `.cfi_startproc`/`.cfi_endproc` markers (`.cfi_startproc` directly after
/// the function label, `.cfi_endproc` directly before its `.size`). Code
/// outside such functions, including hand-written functions in top-level
/// asm, is copied unchanged.
/// What callees pop beyond their return address (i686 `ret $N`: the SysV
/// hidden struct-return pointer, fastcall stack arguments). The call text
/// does not carry it, so the codegen reports it; see
/// `CodegenState::callee_pops_direct`. Empty on x86-64.
#[derive(Clone, Copy)]
pub(crate) struct CalleePops<'a> {
    /// By callee symbol (`None`: the sites disagree; unknown).
    pub direct: &'a FxHashMap<String, Option<u32>>,
    /// By caller: each indirect call's pop in emission order (`None`:
    /// unknown).
    pub indirect: &'a FxHashMap<String, Vec<Option<u32>>>,
}

pub(crate) fn synthesize(
    asm: &str,
    arch: CfiArch,
    marked: &FxHashSet<String>,
    pops: CalleePops<'_>,
) -> String {
    let lines: Vec<&str> = asm.lines().collect();
    let mut out = String::with_capacity(asm.len() + asm.len() / 8);
    let debug = std::env::var_os("LCCC_CFI_DEBUG").is_some();
    // Local labels whose address is taken anywhere in the module (jump
    // tables, `&&label` tables of computed goto -- often emitted far from
    // the function --, `@GOTOFF` loads): the possible targets of indirect
    // jumps. Direct branches (`j*`) and label definitions do not count.
    let mut addr_taken: FxHashSet<&str> = FxHashSet::default();
    for l in &lines {
        let t = l.trim();
        if !t.contains(".L") || t.ends_with(':') || t.starts_with('j') || t.starts_with(".cfi_") {
            continue;
        }
        addr_taken.extend(tokens(t).filter(|tok| tok.starts_with(".L")));
    }
    let mut sections = Sections::new();
    let mut app = false;
    let mut i = 0;
    let mut prev_label: Option<&str> = None;
    while i < lines.len() {
        let t = lines[i].trim();
        if t == "#APP" {
            app = true;
        } else if t == "#NO_APP" {
            app = false;
        }
        if !app && t == ".cfi_startproc" {
            if let Some(name) = prev_label.filter(|n| marked.contains(*n)) {
                let want = format!(".size {}, .-{}", name, name);
                let mut end = None;
                for k in i + 1..lines.len() {
                    if lines[k].trim() != ".cfi_endproc" {
                        continue;
                    }
                    let next = lines[k + 1..]
                        .iter()
                        .map(|s| s.trim())
                        .find(|s| !s.is_empty());
                    if next == Some(want.as_str()) {
                        end = Some(k);
                        break;
                    }
                }
                if let Some(end) = end {
                    out.push_str(lines[i]);
                    out.push('\n');
                    let mut fn_sections = sections.clone();
                    synthesize_function(
                        &lines[i + 1..end],
                        arch,
                        debug,
                        name,
                        pops,
                        &addr_taken,
                        &mut fn_sections,
                        &mut out,
                    );
                    sections = fn_sections;
                    i = end;
                    prev_label = None;
                    continue;
                }
            }
        }
        sections.apply(t);
        if let Some(l) = t.strip_suffix(':') {
            prev_label = Some(l);
        } else if !t.is_empty() {
            prev_label = None;
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    if !asm.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

struct LineCtx {
    /// In the function's own section (jump tables etc. live elsewhere).
    in_fn: bool,
    /// Inside user inline asm.
    app: bool,
}

/// Bytes popped by the callee of each call in `body` (per line; `Some(0)`
/// for every other line). Direct calls are looked up by symbol. Indirect
/// calls take the codegen's per-function list in order, which is only
/// trusted when the final code still has exactly as many indirect calls
/// (the peephole may turn `movl $f, %eax; call *%eax` into `call f`); on a
/// mismatch every indirect call of a function whose list contains a
/// non-zero or unknown pop is treated as unknown -- the CFA is then
/// undefined after it rather than described wrongly.
fn call_pops(
    body: &[&str],
    ctx: &[LineCtx],
    arch: CfiArch,
    name: &str,
    pops: CalleePops<'_>,
    debug: bool,
) -> Vec<Option<i64>> {
    let sfx = arch.suffix();
    let mut out = vec![Some(0); body.len()];
    let mut indirect_lines = Vec::new();
    for (k, l) in body.iter().enumerate() {
        if !ctx[k].in_fn || ctx[k].app {
            continue;
        }
        let Some(insn) = parse_insn(l.trim()) else {
            continue;
        };
        if insn.mnem != "call" && insn.mnem != format!("call{}", sfx) {
            continue;
        }
        let Some(&target) = insn.ops.first() else {
            continue;
        };
        if target.starts_with('*') {
            indirect_lines.push(k);
        } else {
            let sym = target.split('@').next().unwrap_or(target);
            if let Some(&p) = pops.direct.get(sym) {
                out[k] = p.map(|n| n as i64);
            }
        }
    }
    if let Some(list) = pops.indirect.get(name) {
        if list.len() == indirect_lines.len() {
            for (&k, p) in indirect_lines.iter().zip(list) {
                out[k] = p.map(|n| n as i64);
            }
        } else if list.iter().any(|p| *p != Some(0)) {
            if debug {
                eprintln!(
                    "lccc: cfi: {}: {} indirect calls in the final code, {} recorded",
                    name,
                    indirect_lines.len(),
                    list.len()
                );
            }
            for &k in &indirect_lines {
                out[k] = None;
            }
        }
    }
    out
}

fn synthesize_function<'b>(
    body: &[&'b str],
    arch: CfiArch,
    debug: bool,
    name: &str,
    pops: CalleePops<'_>,
    addr_taken: &FxHashSet<&str>,
    sections: &mut Sections,
    out: &mut String,
) {
    let fn_section = sections.cur.clone();
    let mut ctx = Vec::with_capacity(body.len());
    {
        let mut app = false;
        for l in body {
            let t = l.trim();
            if t == "#APP" {
                app = true;
            } else if t == "#NO_APP" {
                app = false;
            } else {
                sections.apply(t);
            }
            ctx.push(LineCtx {
                in_fn: sections.cur == fn_section,
                app,
            });
        }
    }
    let call_pop = call_pops(body, &ctx, arch, name, pops, debug);
    let sfx = arch.suffix();
    let push_m = format!("push{}", sfx);
    let mov_m = format!("mov{}", sfx);

    // Pass 0: prologue region -> saved registers and the body state.
    let mut saves: Vec<(&str, i64)> = Vec::new();
    let mut body_state = Frame::entry(arch);
    {
        let mut f = Frame::entry(arch);
        let mut unknown = false;
        // Registers the i386 PIC setup (`call __x86.get_pc_thunk.R`;
        // `addl $_GLOBAL_OFFSET_TABLE_, %R`) has overwritten: a later store
        // of one of them spills the GOT pointer, it saves nothing.
        let mut clobbered: Vec<&'static str> = Vec::new();
        for (k, l) in body.iter().enumerate() {
            if !ctx[k].in_fn || ctx[k].app {
                break;
            }
            let t = l.trim();
            if t.ends_with(':') {
                break;
            }
            let Some(insn) = parse_insn(t) else {
                continue;
            };
            if let Some(r) = pic_setup_reg(&insn, arch, &clobbered) {
                // Part of the prologue: stopping here would leave the
                // frame allocation after it out of `body_state`.
                clobbered.push(r);
                let _ = step(&mut f, &insn, arch, call_pop[k], &mut unknown);
                body_state = f;
                continue;
            }
            if !is_prologue_insn(&insn, arch) {
                break;
            }
            if insn.mnem == push_m {
                let r = insn.ops[0].trim_start_matches('%');
                if arch.is_callee_saved(r)
                    && !clobbered.contains(&r)
                    && !saves.iter().any(|(s, _)| *s == r)
                {
                    if let Some(o) = f.sp_off {
                        saves.push((r, -(o + arch.slot())));
                    }
                }
            } else if insn.mnem == mov_m {
                if let (Some(r), Some((d, base))) =
                    (insn.ops[0].strip_prefix('%'), parse_disp_reg(insn.ops[1]))
                {
                    if arch.is_callee_saved(r)
                        && !clobbered.contains(&r)
                        && !saves.iter().any(|(s, _)| *s == r)
                    {
                        let off = if base == arch.sp() {
                            f.sp_off.map(|o| d - o)
                        } else if base == arch.fp() {
                            f.fp_off.map(|o| d - o)
                        } else {
                            None
                        };
                        if let Some(off) = off {
                            saves.push((r, off));
                        }
                    }
                }
            }
            let _ = step(&mut f, &insn, arch, call_pop[k], &mut unknown);
            body_state = f;
        }
    }
    // User asm's own labels are not frame-state points: #APP text is not
    // interpreted (below, labels it NAMES are edges, as in asm goto).
    let labels: Vec<&str> = body
        .iter()
        .enumerate()
        .filter(|(k, _)| ctx[*k].in_fn && !ctx[*k].app)
        .filter_map(|(_, l)| l.trim().strip_suffix(':'))
        .collect();
    let label_set: FxHashSet<&str> = labels.iter().copied().collect();

    // Pass 1: the frame state at every label, GCC dwarf2cfi style: only
    // states derived from the entry along direct edges (fallthrough,
    // jmp/jcc, labels named by user asm -- asm goto) are propagated, to a
    // fixed point. The first state to reach a label fixes it: codegen
    // frames are path-independent at block boundaries.
    let entry = Frame::entry(arch);
    let mut at_label: FxHashMap<&'b str, Frame> = FxHashMap::default();
    let propagate = |at_label: &mut FxHashMap<&'b str, Frame>| -> Vec<Frame> {
        let mut indirect_jumps = Vec::new();
        loop {
            let mut changed = false;
            indirect_jumps.clear();
            let mut cur: Option<Frame> = Some(entry);
            for (k, l) in body.iter().enumerate() {
                if !ctx[k].in_fn {
                    continue;
                }
                let t = l.trim();
                if ctx[k].app {
                    if let Some(f) = cur {
                        for lbl in names_label(t, &label_set) {
                            if let std::collections::hash_map::Entry::Vacant(e) =
                                at_label.entry(lbl)
                            {
                                e.insert(f);
                                changed = true;
                            }
                        }
                    }
                    continue;
                }
                if let Some(lbl) = t.strip_suffix(':') {
                    match (cur, at_label.get(lbl)) {
                        (_, Some(&s)) => {
                            // Only a disagreement in the described rule is a
                            // conflict: an untracked %sp under an %fp-based
                            // CFA (e.g. a `__builtin_setjmp` resume point)
                            // describes the same frame.
                            if debug && cur.is_some_and(|c| c.rule() != s.rule()) {
                                eprintln!(
                                    "lccc: cfi: {}: {} reached with two frame states",
                                    name, lbl
                                );
                            }
                            cur = Some(s);
                        }
                        (Some(c), None) => {
                            at_label.insert(lbl, c);
                            changed = true;
                        }
                        (None, None) => {}
                    }
                    continue;
                }
                let (Some(insn), Some(mut f)) = (parse_insn(t), cur) else {
                    continue;
                };
                let mut unknown = false;
                let (flow, target) = step(&mut f, &insn, arch, call_pop[k], &mut unknown);
                // `call .Llocal` (the inline retpoline) is a push and a
                // jump: the target runs with one more slot on the stack.
                let local_call = (insn.mnem == "call" || insn.mnem == format!("call{}", sfx))
                    .then(|| insn.ops.first().copied())
                    .flatten()
                    .filter(|tg| label_set.contains(tg))
                    .map(|tg| {
                        let mut g = f;
                        g.sp_off = g.sp_off.map(|o| o + arch.slot());
                        (tg, g)
                    });
                if let Some((tg, g)) = target
                    .filter(|tg| label_set.contains(tg))
                    .map(|tg| (tg, f))
                    .or(local_call)
                {
                    if let std::collections::hash_map::Entry::Vacant(e) = at_label.entry(tg) {
                        e.insert(g);
                        changed = true;
                    }
                }
                if insn.mnem.starts_with("jmp")
                    && insn.ops.first().is_some_and(|o| o.starts_with('*'))
                {
                    indirect_jumps.push(f);
                } else if matches!(insn.mnem, "ret" | "retq" | "retl")
                    && cur.is_some_and(|c| c.rule() != entry.rule())
                {
                    // A `ret` with the frame still set up pops an address
                    // pushed in this body (the inline retpoline): an
                    // indirect jump, landing with the frame it leaves.
                    let mut g = f;
                    g.sp_off = g.sp_off.map(|o| o - arch.slot());
                    indirect_jumps.push(g);
                }
                cur = match flow {
                    Flow::Next => Some(f),
                    Flow::Stop => None,
                };
            }
            if !changed {
                return indirect_jumps;
            }
        }
    };
    let indirect_jumps = propagate(&mut at_label);
    if at_label.len() < label_set.len() {
        // What is left is reached only through an indirect jump (jump
        // table, computed goto) or not at all. Address-taken labels take
        // the frame of the function's indirect jumps into its body. A
        // `jmp *` with the frame fully popped is a sibling call and one
        // after an unmodelled %sp write a non-local jump (the inline
        // `__builtin_longjmp`): neither dispatches into this body. Without
        // one unambiguous dispatch rule, or for dead code, the state after
        // the prologue.
        let mut dispatch = indirect_jumps
            .iter()
            .filter(|s| s.rule().is_some() && s.rule() != entry.rule());
        let fallback = match dispatch.next() {
            Some(&s) if dispatch.all(|d| d.rule() == s.rule()) => s,
            _ => body_state,
        };
        for &lbl in &labels {
            if addr_taken.contains(lbl) {
                at_label.entry(lbl).or_insert(fallback);
            }
        }
        propagate(&mut at_label);
        for &lbl in &labels {
            at_label.entry(lbl).or_insert(body_state);
        }
    }

    // Pass 2: emit.
    let mut user = UserCfi::new(Some((CfaReg::Sp, arch.slot())));
    let mut cur: Option<Frame> = Some(Frame::entry(arch));
    let mut n_saves_emitted = 0usize;
    let mut reported_unknown = false;
    for (k, l) in body.iter().enumerate() {
        let t = l.trim();
        let LineCtx { in_fn, app } = ctx[k];
        // Any CFI outside user asm is dropped: this function's CFI is
        // derived here in full.
        if in_fn && !app && t.starts_with(".cfi_") {
            continue;
        }
        out.push_str(l);
        out.push('\n');
        if !in_fn {
            continue;
        }
        if app {
            if t.starts_with(".cfi_") {
                if let Some(mut f) = cur {
                    apply_user_cfi(t, &mut f, &mut user, arch);
                    cur = Some(f);
                }
            }
            continue;
        }
        if let Some(lbl) = t.strip_suffix(':') {
            let state = at_label.get(lbl).copied().or(cur).unwrap_or(body_state);
            cur = Some(state);
            user.describe(out, state.rule(), arch);
            continue;
        }
        let (Some(insn), Some(mut f)) = (parse_insn(t), cur) else {
            continue;
        };
        let mut unknown = false;
        let (flow, _) = step(&mut f, &insn, arch, call_pop[k], &mut unknown);
        if unknown && debug && !reported_unknown {
            eprintln!(
                "lccc: cfi: {}: unmodelled stack pointer write `{}`",
                name, t
            );
            reported_unknown = true;
        }
        user.describe(out, f.rule(), arch);
        // Announce each prologue save right after the instruction that
        // performed it (saves are recorded in program order).
        if let Some(&(r, off)) = saves.get(n_saves_emitted) {
            let performs = (insn.mnem == push_m || insn.mnem == mov_m)
                && insn.ops.first().and_then(|o| o.strip_prefix('%')) == Some(r);
            if performs {
                let _ = writeln!(out, "    .cfi_offset %{}, {}", r, off);
                n_saves_emitted += 1;
            }
        }
        cur = match flow {
            Flow::Next => Some(f),
            Flow::Stop => None,
        };
    }
    if debug && n_saves_emitted != saves.len() {
        eprintln!(
            "lccc: cfi: {}: {} of {} prologue saves announced",
            name,
            n_saves_emitted,
            saves.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth(body: &str) -> String {
        let asm = format!(
            ".text\n.type f, @function\nf:\n.cfi_startproc\n{}.cfi_endproc\n.size f, .-f\n",
            body
        );
        let marked: FxHashSet<String> = std::iter::once("f".to_string()).collect();
        let none = FxHashMap::default();
        let none_ind = FxHashMap::default();
        synthesize(
            &asm,
            CfiArch::X86_64,
            &marked,
            CalleePops {
                direct: &none,
                indirect: &none_ind,
            },
        )
    }

    fn synth_i386(body: &str, direct: &[(&str, u32)], indirect: &[Option<u32>]) -> String {
        let asm = format!(
            ".text\n.type f, @function\nf:\n.cfi_startproc\n{}.cfi_endproc\n.size f, .-f\n",
            body
        );
        let marked: FxHashSet<String> = std::iter::once("f".to_string()).collect();
        let direct: FxHashMap<String, Option<u32>> = direct
            .iter()
            .map(|&(n, p)| (n.to_string(), Some(p)))
            .collect();
        let mut ind = FxHashMap::default();
        if !indirect.is_empty() {
            ind.insert("f".to_string(), indirect.to_vec());
        }
        synthesize(
            &asm,
            CfiArch::I386,
            &marked,
            CalleePops {
                direct: &direct,
                indirect: &ind,
            },
        )
    }

    /// i386 SysV: a struct-returning callee pops the hidden pointer
    /// (`ret $4`) and the codegen folds that into its cleanup, so the CFA
    /// after such a call is 4 lower than the push/sub arithmetic says.
    #[test]
    fn i386_sret_callee_pop_is_tracked() {
        let body = "    pushl %ebx\n    subl $24, %esp\n    subl $16, %esp\n    call make@PLT\n    addl $12, %esp\n    addl $24, %esp\n    popl %ebx\n    ret\n";
        let out = synth_i386(body, &[("make", 4)], &[]);
        assert!(
            out.contains("    call make@PLT\n    .cfi_def_cfa_offset 44\n    addl $12, %esp\n    .cfi_def_cfa_offset 32\n    addl $24, %esp\n    .cfi_def_cfa_offset 8\n    popl %ebx\n    .cfi_def_cfa_offset 4\n"),
            "{out}"
        );
        // An indirect call takes its pop from the per-function list...
        let body = "    subl $12, %esp\n    call *%eax\n    addl $8, %esp\n    ret\n";
        let out = synth_i386(body, &[], &[Some(4)]);
        assert!(
            out.contains("    addl $8, %esp\n    .cfi_def_cfa_offset 4\n"),
            "{out}"
        );
        // ...and when the final code disagrees with the list, the frame is
        // declared unknown instead of described wrongly.
        let body =
            "    subl $12, %esp\n    call *%eax\n    call *%ecx\n    addl $8, %esp\n    ret\n";
        let out = synth_i386(body, &[], &[Some(4)]);
        assert!(out.contains("    .cfi_undefined %eip\n"), "{out}");
    }

    /// A prefixed terminator ends its block. `rep ret` used to fall through
    /// as a no-op, and label states are seeded first-edge-wins in program
    /// order: `.L2` -- reached only from the later `jne` with the frame still
    /// set up (CFA = %rsp + 16) -- took the torn-down frame (CFA = %rsp + 8)
    /// of the bogus fallthrough, and its epilogue was described as reaching
    /// offset 0. The same holds for the CET and retpoline forms.
    #[test]
    fn prefixed_terminators_stop_the_block() {
        for ret in [
            "rep ret",
            "repz ret",
            "rep; ret",
            "repz retq",
            "bnd ret",
            "notrack jmp *%rax",
            "cs jmp __x86_indirect_thunk_rax",
            "bnd jmp *%rcx",
        ] {
            let body = format!(
                "    pushq %rbx\n    testq %rdi, %rdi\n    je .L3\n    popq %rbx\n    {ret}\n.L2:\n    movq %rdi, %rbx\n    popq %rbx\n    ret\n.L3:\n    decq %rdi\n    jne .L2\n    popq %rbx\n    ret\n"
            );
            let out = synth(&body);
            assert!(
                out.contains(".L2:\n    .cfi_def_cfa_offset 16\n    movq %rdi, %rbx\n    popq %rbx\n    .cfi_def_cfa_offset 8\n"),
                "{ret}: {out}"
            );
            assert!(!out.contains("offset 0\n"), "{ret}: {out}");
            assert!(!out.contains(".cfi_undefined"), "{ret}: {out}");
        }
    }

    /// `data16` is not semantic-neutral (`data16 push` moves %rsp by two),
    /// so it must not be peeled like `rep`.
    #[test]
    fn parse_insn_peels_only_neutral_prefixes() {
        let m = |t| parse_insn(t).map(|i| (i.mnem, i.ops.len()));
        assert_eq!(m("rep ret"), Some(("ret", 0)));
        assert_eq!(m("repz ret # K8"), Some(("ret", 0)));
        assert_eq!(m("rep; ret"), Some(("ret", 0)));
        assert_eq!(m("notrack jmp *%rax"), Some(("jmp", 1)));
        assert_eq!(m("lock addq $1, (%rsp)"), Some(("addq", 2)));
        assert_eq!(m("rep movsb"), Some(("movsb", 0)));
        assert_eq!(m("rep"), Some(("rep", 0)));
        assert_eq!(m("data16 push %ax").map(|x| x.0), Some("data16"));
    }

    /// A label laid out before any branch to it used to be seeded with the
    /// post-prologue guess, and that guess then fixed the states of the
    /// labels it branched to. Here the guess (8: the frame is allocated
    /// after the first body instruction) reached `.L2` and `.L3` instead of
    /// 40, and the epilogue went to -24.
    #[test]
    fn i386_pic_label_states_come_from_real_edges() {
        let body = "    pushl %ebx\n    call __x86.get_pc_thunk.bx\n    addl $_GLOBAL_OFFSET_TABLE_, %ebx\n    movl 8(%esp), %eax\n    subl $32, %esp\n    jmp .L1\n.L2:\n    decl %eax\n    jne .L3\n.L1:\n    testl %eax, %eax\n    jne .L2\n.L3:\n    addl $32, %esp\n    popl %ebx\n    ret\n";
        let out = synth_i386(body, &[], &[]);
        assert!(!out.contains("offset -"), "{out}");
        assert!(!out.contains(".L2:\n    .cfi_def_cfa_offset"), "{out}");
        assert!(
            out.contains("    addl $32, %esp\n    .cfi_def_cfa_offset 8\n    popl %ebx\n    .cfi_def_cfa_offset 4\n    ret\n"),
            "{out}"
        );
        // The GOT pointer spilled after the PIC setup is not a save of the
        // caller's %ebx.
        let body = "    call __x86.get_pc_thunk.bx\n    addl $_GLOBAL_OFFSET_TABLE_, %ebx\n    subl $12, %esp\n    movl %ebx, 4(%esp)\n    addl $12, %esp\n    ret\n";
        let out = synth_i386(body, &[], &[]);
        assert!(!out.contains(".cfi_offset"), "{out}");
    }

    /// The post-prologue state includes the frame allocation after the
    /// i386 PIC setup. A computed-goto target whose `&&label` table is
    /// emitted outside the function takes the dispatch frame; a label no
    /// edge or address reaches (dead code) the post-prologue one.
    #[test]
    fn i386_indirect_targets_and_dead_code_after_pic_setup() {
        let asm = ".text\n.type f, @function\nf:\n.cfi_startproc\n    pushl %ebx\n    call __x86.get_pc_thunk.bx\n    addl $_GLOBAL_OFFSET_TABLE_, %ebx\n    subl $24, %esp\n    movl %eax, %ecx\n    pushl %eax\n    jmp *%ecx\n.L7:\n    popl %eax\n    addl $24, %esp\n    popl %ebx\n    ret\n.L9:\n    addl $24, %esp\n    popl %ebx\n    ret\n.cfi_endproc\n.size f, .-f\n.section .data.rel.ro.local\ntbl:\n    .long .L7@GOTOFF\n";
        let marked: FxHashSet<String> = std::iter::once("f".to_string()).collect();
        let (d, i) = (FxHashMap::default(), FxHashMap::default());
        let out = synthesize(
            asm,
            CfiArch::I386,
            &marked,
            CalleePops {
                direct: &d,
                indirect: &i,
            },
        );
        assert!(
            out.contains(".L7:\n    popl %eax\n    .cfi_def_cfa_offset 32\n"),
            "{out}"
        );
        assert!(
            out.contains(
                ".L9:\n    .cfi_def_cfa_offset 32\n    addl $24, %esp\n    .cfi_def_cfa_offset 8\n"
            ),
            "{out}"
        );
    }

    /// `-mindirect-branch=thunk-inline`: `call .Lset` pushes the address
    /// the `ret` at `.Lset` pops after overwriting it with the jump-table
    /// target. `.Lset` runs one slot deeper; the table targets land with
    /// the frame the `ret` leaves.
    #[test]
    fn inline_retpoline_is_a_push_and_an_indirect_jump() {
        let out = synth(
            "    pushq %rbx\n    movq %rdi, %rbx\n    leaq .Ljt(%rip), %rcx\n    movslq (%rcx,%rax,4), %rdx\n    addq %rcx, %rdx\n    call .Lset\n.Lspec:\n    pause\n    lfence\n    jmp .Lspec\n.Lset:\n    movq %rdx, (%rsp)\n    ret\n.section .rodata\n.Ljt:\n    .long .L2 - .Ljt\n.section .text,\"ax\",@progbits\n.L2:\n    popq %rbx\n    ret\n",
        );
        assert!(
            out.contains(".Lset:\n    .cfi_def_cfa_offset 24\n"),
            "{out}"
        );
        assert!(
            out.contains(
                ".L2:\n    .cfi_def_cfa_offset 16\n    popq %rbx\n    .cfi_def_cfa_offset 8\n"
            ),
            "{out}"
        );
    }

    /// Jump-table targets are reached by no direct edge: they take the
    /// frame at the dispatching `jmp *`, not the post-prologue guess.
    #[test]
    fn jump_table_targets_take_the_dispatch_frame() {
        let out = synth(
            "    pushq %rbx\n    subq $16, %rsp\n    movq %rsi, %rax\n    pushq %rax\n    jmp *.Ljt(,%rdi,8)\n.section .rodata\n.Ljt:\n    .quad .L2\n    .quad .L3\n.section .text,\"ax\",@progbits\n.L3:\n    popq %rax\n    jmp .L4\n.L2:\n    popq %rax\n.L4:\n    addq $16, %rsp\n    popq %rbx\n    ret\n",
        );
        assert!(
            out.contains(
                ".L3:\n    popq %rax\n    .cfi_def_cfa_offset 32\n    jmp .L4\n.L2:\n    .cfi_def_cfa_offset 40\n"
            ),
            "{out}"
        );
        assert!(!out.contains("offset -"), "{out}");
    }

    #[test]
    fn fpo_pushes_and_frame() {
        let out = synth(
            "    pushq %rbx\n    pushq %r12\n    subq $16, %rsp\n    call g\n    addq $16, %rsp\n    popq %r12\n    popq %rbx\n    ret\n",
        );
        let want = ".text\n.type f, @function\nf:\n.cfi_startproc\n    pushq %rbx\n    .cfi_def_cfa_offset 16\n    .cfi_offset %rbx, -16\n    pushq %r12\n    .cfi_def_cfa_offset 24\n    .cfi_offset %r12, -24\n    subq $16, %rsp\n    .cfi_def_cfa_offset 40\n    call g\n    addq $16, %rsp\n    .cfi_def_cfa_offset 24\n    popq %r12\n    .cfi_def_cfa_offset 16\n    popq %rbx\n    .cfi_def_cfa_offset 8\n    ret\n.cfi_endproc\n.size f, .-f\n";
        assert_eq!(out, want);
    }

    #[test]
    fn frame_pointer_with_early_return() {
        let out = synth(
            "    pushq %rbp\n    movq %rsp, %rbp\n    pushq %rbx\n    subq $24, %rsp\n    testq %rdi, %rdi\n    je .L1\n    leaq -8(%rbp), %rsp\n    popq %rbx\n    popq %rbp\n    ret\n.L1:\n    call g\n    leave\n    ret\n",
        );
        assert!(out.contains("    pushq %rbp\n    .cfi_def_cfa_offset 16\n    .cfi_offset %rbp, -16\n    movq %rsp, %rbp\n    .cfi_def_cfa_register %rbp\n    pushq %rbx\n    .cfi_offset %rbx, -24\n"), "{out}");
        assert!(out.contains("    popq %rbp\n    .cfi_def_cfa %rsp, 8\n    ret\n.L1:\n    .cfi_def_cfa %rbp, 16\n"), "{out}");
        assert!(
            out.contains("    leave\n    .cfi_def_cfa %rsp, 8\n"),
            "{out}"
        );
    }

    #[test]
    fn jump_table_in_rodata_is_skipped() {
        let out = synth(
            "    pushq %rbx\n    jmp *.Ljt(,%rdi,8)\n.section .rodata\n.Ljt:\n    .quad .L2\n.section .text,\"ax\",@progbits\n.L2:\n    popq %rbx\n    ret\n",
        );
        assert!(out.contains(".Ljt:\n    .quad .L2\n"), "{out}");
        assert!(
            out.contains(".L2:\n    popq %rbx\n    .cfi_def_cfa_offset 8\n"),
            "{out}"
        );
    }

    /// `.cfi_escape` CFA opcodes inside user asm are decoded like the
    /// directives they spell. Ignored, the model kept CFA = %rsp + 16 across
    /// a block that left it at 24, and the compiler's `addq $8, %rsp` after
    /// it was described as reaching 8.
    #[test]
    fn user_cfi_escape_cfa_opcodes_are_decoded() {
        for esc in [
            ".cfi_escape 0x0e, 0x18",       // DW_CFA_def_cfa_offset 24
            ".cfi_escape 0x13, 0x7d",       // DW_CFA_def_cfa_offset_sf -3 * -8
            ".cfi_escape 0x0c, 0x07, 0x18", // DW_CFA_def_cfa %rsp(7), 24
            ".cfi_escape 0x12, 0x07, 0x7d", // DW_CFA_def_cfa_sf %rsp, -3 * -8
            // A register rule (DW_CFA_expression %rbx, 2-byte block) is
            // skipped without losing the CFA opcode after it.
            ".cfi_escape 0x10, 0x03, 0x02, 0x77, 0x00, 0x0e, 0x18",
            // remember / change / restore / change, all inside one escape.
            ".cfi_escape 0x0a, 0x0e, 0x40, 0x0b, 0x0e, 0x18",
        ] {
            let out = synth(&format!(
                "    pushq %rbx\n#APP\n    pushq %rax\n    {esc}\n#NO_APP\n    addq $8, %rsp\n    popq %rbx\n    ret\n"
            ));
            assert!(
                out.contains("    addq $8, %rsp\n    .cfi_def_cfa_offset 16\n    popq %rbx\n    .cfi_def_cfa_offset 8\n"),
                "{esc}: {out}"
            );
        }
    }

    /// A CFA the model cannot express -- an expression, or a register it
    /// does not track (GCC's `%r10` stack realignment) -- must be followed by
    /// a FULL rule: `DW_CFA_def_cfa_offset` is undefined unless the current
    /// rule is register+offset, and was emitted here.
    #[test]
    fn opaque_user_cfa_is_restated_in_full() {
        for esc in [
            ".cfi_escape 0x0f, 0x03, 0x77, 0x10, 0x06", // DW_CFA_def_cfa_expression
            ".cfi_def_cfa %r10, 0",
            ".cfi_def_cfa_register %r10",
            ".cfi_escape 0x41",       // DW_CFA_advance_loc inside the escape
            ".cfi_escape 0x0c, 0x07", // truncated
        ] {
            let out = synth(&format!(
                "    pushq %rbx\n#APP\n    {esc}\n#NO_APP\n    popq %rbx\n    ret\n"
            ));
            assert!(
                out.contains("    popq %rbx\n    .cfi_def_cfa %rsp, 8\n"),
                "{esc}: {out}"
            );
            assert!(!out.contains(".cfi_def_cfa_offset 8"), "{esc}: {out}");
        }
    }

    /// `.cfi_remember_state`/`.cfi_restore_state` in user asm restore the
    /// user's CFA rule; ignored, the block's temporary 32 leaked out.
    #[test]
    fn user_cfi_remember_restore_state() {
        let out = synth(
            "    pushq %rbx\n#APP\n    .cfi_remember_state\n    .cfi_def_cfa_offset 32\n    .cfi_restore_state\n#NO_APP\n    popq %rbx\n    ret\n",
        );
        assert!(
            out.contains("    popq %rbx\n    .cfi_def_cfa_offset 8\n"),
            "{out}"
        );
        // A restore with nothing remembered in the block is not knowable.
        let out = synth(
            "    pushq %rbx\n#APP\n    .cfi_restore_state\n#NO_APP\n    popq %rbx\n    ret\n",
        );
        assert!(
            out.contains("    popq %rbx\n    .cfi_def_cfa %rsp, 8\n"),
            "{out}"
        );
    }

    #[test]
    fn user_asm_cfi_is_kept() {
        let out = synth(
            "#APP\n    pushq %rax\n    .cfi_adjust_cfa_offset 8\n    popq %rax\n    .cfi_adjust_cfa_offset -8\n#NO_APP\n    ret\n",
        );
        assert!(out.contains("    .cfi_adjust_cfa_offset 8\n"), "{out}");
        assert!(!out.contains(".cfi_def_cfa_offset"), "{out}");
    }
}
