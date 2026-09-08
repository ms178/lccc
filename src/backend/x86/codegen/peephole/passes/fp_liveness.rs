//! Exact intra-function liveness of XMM registers **and frame slots** for the
//! scalar-FP peephole passes.
//!
//! # Why this exists
//!
//! [`super::liveness::FileLiveness`] answers "is GP register F read on any
//! path from here?" with a real backward dataflow over the function's CFG.
//! The scalar-FP folds (`fold_fp_register_loads`, `fold_fma_memory_src2`,
//! `fold_scalar_fp_memory_into_vex_op`, `eliminate_fp_spill_around_load`,
//! `fold_ptr_deref_through_stack`, `promote_loop_invariant_fp_load`, the
//! `eliminate_fp_xmm_roundtrips` patterns …) had no such oracle: every one of
//! them proved "`%xmmN` is dead" or "slot `-O(%rbp)` is dead" with a *textual
//! forward scan* — "no later line in this block / this function mentions the
//! token".  Such a scan is blind to
//!
//! * **loop back-edges** — the reader sits textually *above* the fold, at the
//!   loop header, and is reached through `jmp .LBBn` (miscompiles `lc.c`,
//!   `lc2.c`, `patE.c`, `patE2.c`: a loop-carried FP value's home was folded
//!   away because its only textual mention after the fold was the adjacent
//!   consumer);
//! * **calls** — every XMM register is caller-saved, so a value that is
//!   *promoted into* `%xmm2` for a loop whose body calls `sin` is destroyed
//!   on every iteration (`hoistH.c`);
//! * **non-mov slot readers** — `mulsd -112(%rbp), %xmm0` reads the slot but
//!   is not a `LoadXmmRbp`; a same-offset `movq %rax, -112(%rbp)` was taken
//!   as "slot dead" although the `mulsd` between them still read the old
//!   value (`t2.c`).
//!
//! Patching each scan individually leaves the same hole in the next pass.
//! This module is the single oracle every FP fold consults instead.
//!
//! # Model
//!
//! One backward dataflow per function over a combined fact vector:
//!
//! * bits `0..16`  — `%xmm0`..`%xmm15` (a `%ymmN` mention aliases `%xmmN`);
//! * bits `16..`   — frame slots, interned per function as `(base, offset,
//!   byte-size)` triples.  A slot is *read* by any instruction whose text
//!   contains a memory operand overlapping it, unless that instruction is a
//!   plain store that *covers* it (a covering store is a kill).  Indexed
//!   frame operands (`8(%rsp,%rax)`) and any address escape (`leaq
//!   N(%rbp)`, raw `%rsp`/`%rbp` copies) make every slot of the function
//!   permanently live (fail closed).
//!
//! Instruction effects:
//!
//! * a line's **register writes** are its AT&T destination operand when the
//!   mnemonic is a full-width definition (`is_full_xmm_def`); every other
//!   mention is a read.  Merge-forms (`movsd %xmmA, %xmmB`, the legacy
//!   destructive ALU ops, `cvtsi2sd`, blends…) read their destination too;
//! * `call` reads `%xmm0`..`%xmm7` (FP argument registers; the argument
//!   moves may be self-move-elided and therefore invisible) and kills all
//!   sixteen (caller-saved, SysV);
//! * `ret` reads `%xmm0` and `%xmm1` (FP return registers) and nothing else;
//! * inline asm reads and writes everything;
//! * an unresolvable transfer (indirect jump, jump table, tail call to a
//!   symbol) makes the function unanalysable — every query then answers
//!   `None`, and callers must fall back to *not folding*.
//!
//! Frame-pointer status is per function: `(%rbp)` operands are frame slots
//! only after `movq %rsp, %rbp`; without it `%rbp` is a data register and a
//! `(%rbp)` access is an opaque pointer dereference (treated as an escape).
//! `%rsp`-relative slots are only tracked when no mid-body `%rsp` shift
//! exists (`push`/`pop`/`subq $N,%rsp` outside prologue/epilogue chains).
//!
//! # Cost
//!
//! Built once per phase-1 iteration for the whole file (the FP passes run in
//! sequence and each refreshes the function it rewrote via [`refresh_at`]).
//! Everything is linear in the function length except the fixpoint, which
//! converges in a handful of rounds for structured code (bounded at 64).

use super::super::types::*;
use super::helpers::is_rsp_shift_line;
use crate::backend::peephole_common::{LineStore, last_top_level_comma};

/// Per-line liveness facts for one function.  `live_out[n]` is the fact
/// vector live immediately after line `n`.
struct FnFacts {
    start: usize,
    end: usize,
    /// Interned frame slots: (base family 4=%rsp / 5=%rbp, offset, size).
    slots: Vec<(u8, i32, i32)>,
    /// `Some(bits)` per line inside `[start, end)` when analysable.
    live_out: Vec<u64>,
    /// Every slot is pinned live (address escape / indexed frame access /
    /// mid-body %rsp shift).  Register facts stay exact.
    slots_opaque: bool,
    known: bool,
}

/// File-wide XMM + frame-slot liveness.
pub(super) struct FpLiveness {
    funcs: Vec<FnFacts>,
}

const XMM_ALL: u64 = 0xFFFF;
/// `%xmm0..%xmm7`: SysV FP argument registers a `call` may read implicitly.
const CALL_READS: u64 = 0x00FF;
/// `%xmm0`, `%xmm1`: FP return registers live at `ret` (a two-double
/// aggregate or `double _Complex` returns in both).
const RET_READS: u64 = 0x0003;
/// `%xmm0` only: the ret-live set of a function whose every `ret` tail
/// block materialises `%xmm0` and never touches `%xmm1` — the signature of
/// a scalar FP return.  Mirrors `FileLiveness::returns_in_rax_only`.
const RET_READS_XMM0_ONLY: u64 = 0x0001;
const MAX_SLOTS: usize = 48; // 64 - 16 register bits

impl FpLiveness {
    /// Analyse every function in the file.  A function starts at
    /// `.type NAME, @function` (present with or without unwind tables — the
    /// kernel builds with `-fno-asynchronous-unwind-tables` and emits no CFI
    /// at all) or, for fragments that carry CFI only, at `.cfi_startproc`;
    /// it ends at the first `.cfi_endproc` or `.size` that follows.
    pub(super) fn new(store: &LineStore, infos: &[LineInfo]) -> Self {
        let mut funcs = Vec::new();
        let len = store.len();
        let mut i = 0;
        while i < len {
            if infos[i].is_nop() || infos[i].kind != LineKind::Directive {
                i += 1;
                continue;
            }
            let t = infos[i].trimmed(store.get(i));
            let is_start = t.starts_with(".cfi_startproc")
                || t.starts_with(".type ") && t.trim_end().ends_with("@function");
            if !is_start {
                i += 1;
                continue;
            }
            let mut end = len;
            for n in i + 1..len {
                if infos[n].is_nop() || infos[n].kind != LineKind::Directive {
                    continue;
                }
                let tn = infos[n].trimmed(store.get(n));
                if tn.starts_with(".cfi_endproc") || tn.starts_with(".size ") {
                    end = n;
                    break;
                }
            }
            funcs.push(analyse(store, infos, i, end));
            i = end.max(i + 1);
        }
        // No delimiter anywhere (a peephole unit-test fragment, a hand-
        // written stub): analyse the whole text as one function.  Sound
        // because falling off the end keeps EVERY fact live — only a `ret`
        // kills, and a `ret` is the end of the program the fragment
        // describes.  Files with at least one delimited function keep the
        // strict rule: code outside any function answers `None`.
        if funcs.is_empty() && len > 0 {
            funcs.push(analyse(store, infos, 0, len));
        }
        FpLiveness { funcs }
    }

    fn func_of(&self, idx: usize) -> Option<&FnFacts> {
        // Functions are in ascending order; binary search on `start`.
        let pos = self.funcs.partition_point(|f| f.start <= idx);
        if pos == 0 {
            return None;
        }
        let f = &self.funcs[pos - 1];
        (idx < f.end).then_some(f)
    }

    /// `Some(true)` when `%xmm<n>` may be read on some path after line `idx`
    /// before being fully redefined, `Some(false)` when provably dead,
    /// `None` when the enclosing function could not be analysed (or `idx`
    /// lies outside any delimited function).
    pub(super) fn xmm_live_after(&self, idx: usize, n: u32) -> Option<bool> {
        if n >= 16 {
            return None;
        }
        let f = self.func_of(idx)?;
        if !f.known {
            return None;
        }
        Some(f.live_out[idx - f.start] & (1u64 << n) != 0)
    }

    /// Same question for the `size`-byte frame slot at `offset(%base)`
    /// (`base` is the family id: 4 = `%rsp`, 5 = `%rbp`).  Any overlapping
    /// tracked slot that is live makes the answer `Some(true)`; a slot that
    /// was never interned (no instruction of the function names it) is
    /// dead unless the function's slots are opaque.
    pub(super) fn slot_live_after(
        &self,
        idx: usize,
        base: u8,
        offset: i32,
        size: i32,
    ) -> Option<bool> {
        let f = self.func_of(idx)?;
        if !f.known {
            return None;
        }
        if f.slots_opaque {
            return Some(true);
        }
        let bits = f.live_out[idx - f.start];
        for (k, &(b, o, s)) in f.slots.iter().enumerate() {
            if b == base && ranges_overlap(offset, size, o, s) && bits & (1u64 << (16 + k)) != 0 {
                return Some(true);
            }
        }
        Some(false)
    }

    /// The question every load-folding pass asks, with the fail-closed
    /// fallback built in: may `%xmm<n>` be observed after line `idx`?
    ///
    /// * Analysable function → the dataflow answer.
    /// * Otherwise (`None`: unresolvable control flow, or no function
    ///   delimiter) → the value is dead only when NO line of the enclosing
    ///   text other than `owned` mentions the register at all (a proof that
    ///   holds under any CFG), and no implicit reader can observe it: a
    ///   `call` reads `%xmm0`–`%xmm7`, a `ret` reads `%xmm0`/`%xmm1`.
    pub(super) fn xmm_dead_after(
        &self,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
        n: u32,
        owned: &[usize],
    ) -> bool {
        let ans = self.xmm_live_after(idx, n);
        if ans == Some(true) && std::env::var_os("CCC_DEBUG_FP_LIVENESS").is_some() {
            self.explain_live(store, infos, idx, n);
        }
        match ans {
            Some(live) => !live,
            None => {
                if n >= 16 {
                    return false;
                }
                let (lo, hi) = match self.func_of(idx) {
                    Some(f) => (f.start, f.end),
                    None => (0, store.len()),
                };
                let bit = 1u64 << n;
                for k in lo..hi {
                    if infos[k].is_nop() || owned.contains(&k) {
                        continue;
                    }
                    match infos[k].kind {
                        LineKind::Call if n <= 7 => return false,
                        LineKind::Ret if n <= 1 => return false,
                        LineKind::InlineAsm => return false,
                        _ => {}
                    }
                    if xmm_mentions(infos[k].trimmed(store.get(k))) & bit != 0 {
                        return false;
                    }
                }
                true
            }
        }
    }

    /// `CCC_DEBUG_FP_LIVENESS=1`: say why `%xmmN` is live after `idx` — the
    /// first later line on a live path that reads it, or, when no such
    /// textual reader exists (back edge / call / ret), the live ranges of
    /// the register inside the function.
    fn explain_live(&self, store: &LineStore, infos: &[LineInfo], idx: usize, n: u32) {
        let Some(f) = self.func_of(idx) else { return };
        let bit = 1u64 << n;
        let reader = (idx + 1..f.end).find(|&k| {
            !infos[k].is_nop()
                && xmm_mentions(infos[k].trimmed(store.get(k))) & bit != 0
                && f.live_out[k - f.start] & bit != 0
        });
        let why = match reader {
            Some(k) => format!("line {} `{}`", k, store.get(k).trim()),
            None => {
                let mut ranges = String::new();
                let mut k = f.start;
                while k < f.end {
                    if f.live_out[k - f.start] & bit == 0 {
                        k += 1;
                        continue;
                    }
                    let s0 = k;
                    while k < f.end && f.live_out[k - f.start] & bit != 0 {
                        k += 1;
                    }
                    ranges.push_str(&format!(
                        " [{}..{}] ends at `{}`",
                        s0,
                        k - 1,
                        store.get(k.min(f.end - 1)).trim()
                    ));
                }
                format!("no textual reader below; live ranges:{ranges}")
            }
        };
        eprintln!(
            "fp_liveness: xmm{} live after line {} `{}` - {}",
            n,
            idx,
            store.get(idx).trim(),
            why
        );
    }

    /// Slot counterpart of [`Self::xmm_dead_after`].  There is no textual
    /// fallback for slots: without a CFG a store's readers cannot be bounded.
    pub(super) fn slot_dead_after(&self, idx: usize, base: u8, offset: i32, size: i32) -> bool {
        self.slot_live_after(idx, base, offset, size) == Some(false)
    }

    /// Re-analyse the function containing `idx` after a pass rewrote lines
    /// inside it.  Mandatory before the next query in the same pass: a fold
    /// can extend or shorten live ranges of the registers it touches.
    pub(super) fn refresh_at(&mut self, store: &LineStore, infos: &[LineInfo], idx: usize) {
        let pos = self.funcs.partition_point(|f| f.start <= idx);
        if pos == 0 {
            return;
        }
        let (start, end) = (self.funcs[pos - 1].start, self.funcs[pos - 1].end);
        if idx >= end {
            return;
        }
        self.funcs[pos - 1] = analyse(store, infos, start, end);
    }
}

/// Parse `%xmmN` / `%ymmN` (N < 16) at the START of `op`; returns N.
fn xmm_index(op: &str) -> Option<u32> {
    let rest = op
        .strip_prefix("%xmm")
        .or_else(|| op.strip_prefix("%ymm"))?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 2 || rest.len() != digits {
        return None;
    }
    let n: u32 = rest.parse().ok()?;
    (n < 16).then_some(n)
}

/// Bitmask of every `%xmmN`/`%ymmN` mentioned anywhere in `t`.
fn xmm_mentions(t: &str) -> u64 {
    let b = t.as_bytes();
    let mut mask = 0u64;
    let mut i = 0;
    while i + 4 < b.len() {
        if b[i] == b'%'
            && (b[i + 1] == b'x' || b[i + 1] == b'y')
            && b[i + 2] == b'm'
            && b[i + 3] == b'm'
        {
            let mut j = i + 4;
            let mut n = 0u32;
            let mut nd = 0;
            while j < b.len() && b[j].is_ascii_digit() && nd < 2 {
                n = n * 10 + u32::from(b[j] - b'0');
                j += 1;
                nd += 1;
            }
            if nd > 0 && n < 16 && !(j < b.len() && b[j].is_ascii_digit()) {
                mask |= 1u64 << n;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    mask
}

/// Split `t` into (mnemonic, operand text).
fn split_mnemonic(t: &str) -> (&str, &str) {
    match t.find(|c: char| c.is_ascii_whitespace()) {
        Some(p) => (&t[..p], t[p..].trim()),
        None => (t, ""),
    }
}

/// True when `mn` writes its AT&T destination register **in full** without
/// reading it — the definition kills the previous value of the register.
///
/// Legacy SSE ALU ops are destructive (`addsd %xmm1, %xmm0` reads `%xmm0`).
/// Scalar moves with a REGISTER source merge into the destination's upper
/// bits (SDM MOVSD/MOVSS: the legacy and VEX reg-reg forms preserve bits
/// 127:64 / 127:32) and therefore READ it; only their memory-source spelling
/// defines the whole register.  `cvtsi2sd`/`cvtss2sd` & co. merge as well.
/// VEX three-operand ALU forms write the destination from the sources only.
/// Conversions to a *new* full width (`cvtsd2ss` merges — excluded;
/// `cvtdq2pd`, `cvttsd2si` to GP — write fully / write no xmm).
fn is_full_xmm_def(mn: &str, operands: &str) -> bool {
    // Zero idioms: `xorpd %x, %x` / `vxorpd %x, %x, %x` define %x (checked
    // by the caller: sources == destination is a full write, not a read).
    match mn {
        "movsd" | "movss" | "vmovsd" | "vmovss" => {
            // Memory source → full definition.  Legacy register source →
            // merge into the destination's upper lanes (reads it).  The VEX
            // three-operand register form `vmovss %a, %b, %d` writes D from
            // A (upper lanes) and B (low lane) only — a full definition
            // whether or not D is among the sources.
            let src = match last_top_level_comma(operands.as_bytes()) {
                Some(c) => operands[..c].trim(),
                None => return false,
            };
            if !src.starts_with('%') {
                return true;
            }
            mn.starts_with('v') && src.contains(',')
        }
        // Full-width moves and zero-extending narrow moves.
        "movapd" | "movaps" | "movupd" | "movups" | "movdqa" | "movdqu" | "movq" | "movd"
        | "vmovapd" | "vmovaps" | "vmovupd" | "vmovups" | "vmovdqa" | "vmovdqu" | "vmovq"
        | "vmovd" | "lddqu" | "vlddqu" | "movddup" | "vmovddup" | "vbroadcastsd"
        | "vbroadcastss" | "vpbroadcastd" | "vpbroadcastq" | "vpbroadcastb" | "vpbroadcastw"
        | "movshdup" | "movsldup" | "vmovshdup" | "vmovsldup" | "vmovdqa64" | "vmovdqu64" => true,
        // Packed conversions define the whole destination.
        "cvtdq2pd" | "cvtdq2ps" | "cvtpd2ps" | "cvtps2pd" | "cvttpd2dq" | "cvttps2dq"
        | "cvtpd2dq" | "cvtps2dq" | "vcvtdq2pd" | "vcvtdq2ps" | "vcvtpd2ps" | "vcvtps2pd"
        | "vcvttpd2dq" | "vcvttps2dq" | "vcvtpd2dq" | "vcvtps2dq" => true,
        // Scalar int→fp conversions merge into the upper lanes: READ.
        "cvtsi2sd" | "cvtsi2ss" | "cvtsi2sdq" | "cvtsi2ssq" | "cvtsi2sdl" | "cvtsi2ssl"
        | "cvtsd2ss" | "cvtss2sd" | "sqrtsd" | "sqrtss" | "roundsd" | "roundss" | "rcpss"
        | "rsqrtss" => false,
        // Legacy destructive ALU (add/sub/mul/div/min/max/and/or/xor/…): READ.
        _ => {
            if let Some(v) = mn.strip_prefix('v') {
                // VEX three-operand (and the VEX scalar conversions, which
                // take an explicit merge source operand — the destination
                // is still defined from the operands only).  Everything
                // VEX that names a destination writes it fully, EXCEPT
                // the masked-merge / blend-into-dest families that read it:
                // `vblendv*` reads only sources; `vfmadd*` reads the dest
                // as an operand (213/231/132 all name it among the
                // multiplicands or the addend).
                let is_fma = v.starts_with("fmadd")
                    || v.starts_with("fmsub")
                    || v.starts_with("fnmadd")
                    || v.starts_with("fnmsub");
                let is_gather = v.contains("gather");
                let is_insert = v.starts_with("insert") || v.starts_with("pinsr");
                !(is_fma || is_gather || is_insert)
            } else {
                false
            }
        }
    }
}

/// Destination operand of `t` (the last top-level operand), if any.
fn dest_operand(operands: &str) -> Option<&str> {
    if operands.is_empty() {
        return None;
    }
    Some(match last_top_level_comma(operands.as_bytes()) {
        Some(c) => operands[c + 1..].trim(),
        None => operands.trim(),
    })
}

/// Every top-level operand of `operands`.
fn operand_list(operands: &str) -> Vec<&str> {
    let mut out = Vec::with_capacity(3);
    let mut depth = 0u32;
    let mut start = 0usize;
    for (i, b) in operands.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                out.push(operands[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = operands[start..].trim();
    if !last.is_empty() {
        out.push(last);
    }
    out
}

/// A direct frame operand `OFF(%rsp)` / `OFF(%rbp)` → (base family, offset).
/// Indexed or scaled frame operands return `Err(())` (opaque); non-frame
/// operands return `Ok(None)`.
fn frame_operand(op: &str) -> Result<Option<(u8, i32)>, ()> {
    let Some(p) = op.find('(') else {
        return Ok(None);
    };
    let inner = &op[p + 1..];
    let Some(close) = inner.find(')') else {
        return Ok(None);
    };
    let inner = &inner[..close];
    let base = match inner {
        "%rsp" => 4u8,
        "%rbp" => 5u8,
        _ => {
            // `8(%rsp,%rax,8)` etc. — frame base with an index: opaque.
            if inner.starts_with("%rsp,") || inner.starts_with("%rbp,") {
                return Err(());
            }
            return Ok(None);
        }
    };
    let disp = op[..p].trim();
    if disp.is_empty() {
        return Ok(Some((base, 0)));
    }
    // Segment overrides / symbol displacements: opaque.
    match disp.parse::<i32>() {
        Ok(v) => Ok(Some((base, v))),
        Err(_) => Err(()),
    }
}

/// Byte width of the memory access an instruction performs on its frame
/// operand.  Precise for the mnemonics the FP passes create; conservative
/// (16) for anything vector-shaped, 8 for unknown GP-width forms.
fn access_width(mn: &str, operands: &str, op: &str) -> i32 {
    if let Some(sz) = match mn {
        "movsd" | "vmovsd" | "addsd" | "subsd" | "mulsd" | "divsd" | "minsd" | "maxsd"
        | "sqrtsd" | "ucomisd" | "comisd" | "vaddsd" | "vsubsd" | "vmulsd" | "vdivsd"
        | "vminsd" | "vmaxsd" | "vsqrtsd" | "vucomisd" | "vcomisd" | "cvtsd2ss" | "vcvtsd2ss"
        | "cvttsd2si" | "cvtsd2si" | "vcvttsd2si" | "vcvtsd2si" | "movq" | "vmovq"
        | "cvtsi2sdq" | "cvtsi2ssq" | "pushq" | "popq" | "cmpq" | "testq" | "addq" | "subq"
        | "andq" | "orq" | "xorq" | "imulq" | "leaq" | "incq" | "decq" | "negq" | "notq"
        | "shlq" | "shrq" | "sarq" | "cmovq" => Some(8),
        "movss" | "vmovss" | "addss" | "subss" | "mulss" | "divss" | "minss" | "maxss"
        | "sqrtss" | "ucomiss" | "comiss" | "vaddss" | "vsubss" | "vmulss" | "vdivss"
        | "vminss" | "vmaxss" | "vsqrtss" | "vucomiss" | "vcomiss" | "cvtss2sd" | "vcvtss2sd"
        | "cvttss2si" | "cvtss2si" | "movd" | "vmovd" | "movl" | "cvtsi2sd" | "cvtsi2ss"
        | "cvtsi2sdl" | "cvtsi2ssl" | "movslq" | "cmpl" | "testl" | "addl" | "subl" | "andl"
        | "orl" | "xorl" | "imull" | "incl" | "decl" | "negl" | "notl" | "shll" | "shrl"
        | "sarl" | "cmovl" => Some(4),
        "movw" | "cmpw" | "testw" | "addw" | "subw" | "andw" | "orw" | "xorw" | "movzwl"
        | "movswl" | "movzwq" | "movswq" => Some(2),
        "movb" | "cmpb" | "testb" | "addb" | "subb" | "andb" | "orb" | "xorb" | "movzbl"
        | "movsbl" | "movzbq" | "movsbq" | "movzbw" | "movsbw" => Some(1),
        _ => None,
    } {
        return sz;
    }
    if mn.starts_with("set") {
        return 1;
    }
    let _ = op;
    // Unknown mnemonic: over-approximate the extent.  A wider READ can only
    // keep more slots live (conservative); kills come from listed plain
    // stores only, so no unknown form ever produces a false kill.
    if operands.contains("%ymm") { 32 } else { 16 }
}

/// A plain store mnemonic: memory destination is written and NOT read.
fn is_plain_store(mn: &str) -> bool {
    matches!(
        mn,
        "movq"
            | "movl"
            | "movw"
            | "movb"
            | "movsd"
            | "movss"
            | "movd"
            | "vmovsd"
            | "vmovss"
            | "vmovq"
            | "vmovd"
            | "movapd"
            | "movaps"
            | "movupd"
            | "movups"
            | "movdqa"
            | "movdqu"
            | "vmovapd"
            | "vmovaps"
            | "vmovupd"
            | "vmovups"
            | "vmovdqa"
            | "vmovdqu"
            | "movabsq"
    )
}

/// XMM registers a `call` at line `n` may read.
///
/// lccc's call sequence (`calls.rs::emit_call_reg_args_impl`) publishes the
/// number of SSE argument registers it loaded as `movb $N, %al` — for EVERY
/// call with an FP argument, prototyped or not (SysV 3.5.7 only requires it
/// for variadic callees; `%al` is dead on entry to a prototyped one, so the
/// byte is a free, exact census) — or as `xorl %eax, %eax` for a variadic
/// callee without FP arguments.  Only the hazard-area release
/// (`addq $K, %rsp`) and directives may sit between the census and the
/// `call`.  Hence:
///
/// * `movb $N, %al`   → the callee reads `%xmm0..%xmm(N-1)`;
/// * `xorl %eax, %eax` → no XMM argument;
/// * no census        → either a GP-only prototyped call (the common case:
///   no XMM argument at all) or one of the raw libcall sites that stage
///   their FP operand themselves (`i128_ops.rs`: `movq %rax, %xmm0; call
///   __fixdfti@PLT`; `__builtin_apply`: eight `movups` then `call *%r11`).
///   Those stage the argument in the same straight-line window as the
///   call, so the reads are the argument registers WRITTEN in the window
///   between the previous barrier and the call.  The typed MachInst call
///   (`CallTyped`) admits integer arguments only.
///
/// Function-pointer calls through the retpoline thunks and PLT calls all
/// pass through the same census emitter.
fn call_xmm_reads(store: &LineStore, infos: &[LineInfo], start: usize, n: usize) -> u64 {
    // Calls whose census (if any) is NOT in the adjacent window: the inline
    // retpoline (`call .Lrpl_inner_N` behind a `jmp`/label pair, census
    // emitted before the `jmp`), the external thunks, and any indirect
    // `call *…` — all conservative.
    let t = infos[n].trimmed(store.get(n));
    let target = t.split_whitespace().nth(1).unwrap_or("");
    if target.starts_with('*') || target.starts_with(".L") || target.contains("indirect_thunk") {
        return CALL_READS;
    }
    // Phase 1: the census, if any, within the few lines before the call.
    let mut k = n;
    let mut steps = 0;
    let mut census: Option<u64> = None;
    while k > start && steps < 8 {
        k -= 1;
        if infos[k].is_nop() || matches!(infos[k].kind, LineKind::Directive | LineKind::Empty) {
            continue;
        }
        if infos[k].is_barrier() {
            break;
        }
        steps += 1;
        if infos[k].reg_refs & 1 == 0 {
            continue; // no %rax family: the hazard-area release etc.
        }
        let t = infos[k].trimmed(store.get(k));
        if t == "xorl %eax, %eax" {
            census = Some(0);
        } else if let Some(cnt) = t
            .strip_prefix("movb $")
            .and_then(|r| r.strip_suffix(", %al"))
            .or_else(|| {
                t.strip_prefix("movl $")
                    .and_then(|r| r.strip_suffix(", %eax"))
            })
            .and_then(|num| num.parse::<u32>().ok())
            .filter(|&c| c <= 8)
        {
            census = Some((1u64 << cnt) - 1);
        }
        break; // first %rax-family line decides
    }
    if let Some(c) = census {
        return c;
    }
    // Phase 2: no census.  Indirect calls without one (`__builtin_apply`'s
    // `call *%r11`, whose eight `movups` loads sit behind a label) stay
    // conservative.  A direct call without a census is either GP-only or a
    // raw libcall site whose FP operand is staged by the plain XMM moves
    // IMMEDIATELY before it (`movq %rax, %xmm0` / `movd %eax, %xmm0` /
    // `movdqu (%rax), %xmm0`, at most two): collect the argument registers
    // those moves define and stop at the first line that is not such a move.
    let mut written = 0u64;
    let mut k = n;
    let mut steps = 0;
    while k > start && steps < 4 {
        k -= 1;
        if infos[k].is_nop() || matches!(infos[k].kind, LineKind::Directive | LineKind::Empty) {
            continue;
        }
        if infos[k].is_barrier() {
            break;
        }
        steps += 1;
        let tk = infos[k].trimmed(store.get(k));
        let (mn, operands) = split_mnemonic(tk);
        let staging = matches!(
            mn,
            "movq"
                | "movd"
                | "movdqu"
                | "movdqa"
                | "movups"
                | "movaps"
                | "movupd"
                | "movapd"
                | "movsd"
                | "movss"
                | "vmovq"
                | "vmovd"
                | "vmovdqu"
                | "vmovups"
                | "vmovaps"
                | "vmovupd"
                | "vmovapd"
                | "vmovsd"
                | "vmovss"
                | "pxor"
                | "xorps"
                | "xorpd"
        );
        match dest_operand(operands).and_then(xmm_index) {
            Some(d) if staging => written |= 1u64 << d,
            _ => break,
        }
    }
    written & CALL_READS
}

/// XMM registers live at the function's `ret`s.  Compiler output carries
/// the exact set as a `# LCCC_RET_XMM <mask>` marker emitted by the
/// prologue from the function's return classification.  Without a marker
/// (hand-written fragments, unit tests) the set is derived from what each
/// `ret`'s tail block (back to the nearest label / jump / call) materialises
/// — the same evidence [`super::liveness::FileLiveness::returns_in_rax_only`]
/// uses for `%rdx`:
///
/// * tail writes `%xmm0` and never names `%xmm1` → `{xmm0}` (scalar FP,
///   `_Complex float`, all-SSE single-eightbyte aggregates);
/// * tail writes the accumulator and neither FP return register → `{}`
///   (integer / pointer / x87 / sret returns);
/// * anything else (names `%xmm1`, materialises nothing — a shared epilogue
///   reached by `jmp`, a bare `call; ret`) → `{xmm0, xmm1}`.
///
/// The per-`ret` classes are unioned; a single unclassifiable `ret` makes
/// the whole function conservative.
fn ret_xmm_reads(store: &LineStore, infos: &[LineInfo], start: usize, end: usize) -> u64 {
    // Compiler-emitted functions publish the exact set in the prologue
    // (`prologue.rs::ret_xmm_mask`); the marker is authoritative.
    for n in start..end {
        if infos[n].is_nop() {
            continue;
        }
        let t = infos[n].trimmed(store.get(n));
        if let Some(m) = t.strip_prefix("# LCCC_RET_XMM ") {
            if let Ok(mask) = m.trim().parse::<u64>() {
                return mask & RET_READS;
            }
        }
        if infos[n].kind == LineKind::Ret {
            break; // marker precedes the body; no point scanning further
        }
    }
    let mut union = 0u64;
    let mut saw_ret = false;
    for n in start..end {
        if infos[n].is_nop() || infos[n].kind != LineKind::Ret {
            continue;
        }
        saw_ret = true;
        let mut writes_xmm0 = false;
        let mut writes_rax = false;
        let mut mentions_xmm1 = false;
        let mut k = n;
        while k > start {
            k -= 1;
            if infos[k].is_nop() || infos[k].kind == LineKind::Directive {
                continue;
            }
            if matches!(
                infos[k].kind,
                LineKind::Label
                    | LineKind::Jmp
                    | LineKind::CondJmp
                    | LineKind::JmpIndirect
                    | LineKind::Call
                    | LineKind::InlineAsm
            ) {
                break;
            }
            let t = infos[k].trimmed(store.get(k));
            let m = xmm_mentions(t);
            if m & 0b10 != 0 {
                mentions_xmm1 = true;
            }
            let (_, operands) = split_mnemonic(t);
            if m & 0b01 != 0 && dest_operand(operands).and_then(xmm_index) == Some(0) {
                writes_xmm0 = true;
            }
            if super::helpers::writes_family(&infos[k], t, 0) {
                writes_rax = true;
            }
        }
        union |= if writes_xmm0 && !mentions_xmm1 {
            RET_READS_XMM0_ONLY
        } else if writes_rax && !writes_xmm0 && !mentions_xmm1 {
            // Integer return: the accumulator is materialised, neither FP
            // return register is.  (`%xmm0` traffic that does not write it
            // — a compare, a store — does not make it a return value.)
            0
        } else {
            RET_READS
        };
    }
    if saw_ret { union } else { RET_READS }
}

/// `CCC_DEBUG_FP_LIVENESS=1`: report why a function stayed unanalysable.
fn debug_unknown(store: &LineStore, n: usize, why: &str) {
    if std::env::var_os("CCC_DEBUG_FP_LIVENESS").is_some() {
        eprintln!(
            "fp_liveness: line {} unanalysable ({}): {}",
            n,
            why,
            store.get(n).trim()
        );
    }
}

#[derive(Clone, Copy)]
struct Effect {
    reads: u64,
    writes: u64,
}

/// Analyse one function.
fn analyse(store: &LineStore, infos: &[LineInfo], start: usize, end: usize) -> FnFacts {
    let n_lines = end - start;
    let mut facts = FnFacts {
        start,
        end,
        slots: Vec::new(),
        live_out: vec![u64::MAX; n_lines],
        slots_opaque: false,
        known: false,
    };

    // ── pass 0: frame-pointer status, escapes, labels ─────────────────────
    //
    // %rsp-shift discipline (mirrors dead_code::eliminate_never_read_stores):
    // the prologue chain (`pushq`s, `movq %rsp,%rbp`, one `subq $N,%rsp`)
    // and epilogue chains (`addq $N,%rsp` / `movq %rbp,%rsp` / `leave` /
    // `popq`s ending in `ret` or a tail `jmp`) keep %rsp-relative offsets
    // canonical; any OTHER shift (outgoing-argument pushes around a call,
    // F128 scratch windows) makes %rsp-relative slots ambiguous, so they are
    // not tracked in such a function.  %rbp slots are unaffected.
    let mut rbp_is_frame = false;
    let mut labels: Vec<(&str, usize)> = Vec::new();
    let mut rsp_shift_in_body = false;
    let mut opaque = false;
    let mut in_prologue = true;
    for n in start..end {
        if infos[n].is_nop() {
            continue;
        }
        let t = infos[n].trimmed(store.get(n));
        match infos[n].kind {
            LineKind::Label => {
                if let Some(name) = t.strip_suffix(':') {
                    labels.push((name, n));
                }
                continue;
            }
            LineKind::Directive | LineKind::Empty => continue,
            LineKind::InlineAsm => {
                opaque = true; // may read/write any slot through "m" operands
                in_prologue = false;
                continue;
            }
            _ => {}
        }
        let is_frame_setup = t == "movq %rsp, %rbp";
        let is_frame_teardown = t == "movq %rbp, %rsp" || t == "leave";
        if is_frame_setup {
            rbp_is_frame = true;
        }
        let (mn, operands) = split_mnemonic(t);
        let shift = is_rsp_shift_line(t);
        if in_prologue {
            let prologue_line = matches!(infos[n].kind, LineKind::Push { .. })
                || is_frame_setup
                || t.starts_with("subq $") && t.ends_with("%rsp");
            if !prologue_line {
                in_prologue = false;
            }
        }
        if shift && !in_prologue && !rsp_shift_in_body {
            // Epilogue chain?  Only {addq $N,%rsp | popq | directives | nops}
            // may follow until a ret / unconditional jmp.
            let mut k = n;
            let mut epilogue = false;
            while k < end {
                if infos[k].is_nop() || matches!(infos[k].kind, LineKind::Directive) {
                    k += 1;
                    continue;
                }
                match infos[k].kind {
                    LineKind::Ret | LineKind::Jmp => {
                        epilogue = true;
                        break;
                    }
                    LineKind::Pop { .. } => {}
                    _ => {
                        let tk = infos[k].trimmed(store.get(k));
                        let ok = tk == "leave"
                            || tk == "movq %rbp, %rsp"
                            || tk.starts_with("addq $") && tk.ends_with("%rsp");
                        if !ok {
                            break;
                        }
                    }
                }
                k += 1;
            }
            if !epilogue {
                rsp_shift_in_body = true;
            }
        }
        // Address escapes: `leaq OFF(%rsp|%rbp), %reg`, raw `%rsp`/`%rbp`
        // value copies (frame set-up/tear-down and push/pop chains excluded).
        if mn.starts_with("lea")
            && (operands.contains("(%rsp") || rbp_is_frame && operands.contains("(%rbp"))
        {
            opaque = true;
        }
        if operands.contains("%rsp")
            && !operands.contains("(%rsp")
            && !shift
            && !is_frame_setup
            && !is_frame_teardown
        {
            opaque = true;
        }
        if rbp_is_frame
            && operands.contains("%rbp")
            && !operands.contains("(%rbp")
            && !is_frame_setup
            && !is_frame_teardown
            && !matches!(
                infos[n].kind,
                LineKind::Push { reg: 5 } | LineKind::Pop { reg: 5 }
            )
        {
            opaque = true;
        }
        if !rbp_is_frame && operands.contains("(%rbp") {
            // %rbp is a data register here: a `(%rbp)` access is a pointer
            // dereference of unknown target — may alias any %rsp slot.
            opaque = true;
        }
    }
    let track_rsp = !rsp_shift_in_body;

    // Per-function return set (see `ret_xmm_reads`).
    let ret_reads = ret_xmm_reads(store, infos, start, end);

    // ── pass 1: intern slots, compute effects and successors ──────────────
    let resolve = |name: &str| -> Option<usize> {
        labels.iter().find(|(l, _)| *l == name).map(|&(_, idx)| idx)
    };
    let mut lines: Vec<usize> = Vec::with_capacity(n_lines);
    for n in start..end {
        if infos[n].is_nop() || matches!(infos[n].kind, LineKind::Directive | LineKind::Empty) {
            continue;
        }
        lines.push(n);
    }
    let mut effects: Vec<Effect> = Vec::with_capacity(lines.len());
    let mut succs: Vec<Vec<usize>> = Vec::with_capacity(lines.len());
    // Deferred slot reads/writes per line: (slot index, is_write).
    let mut slot_ops: Vec<Vec<(usize, bool)>> = Vec::with_capacity(lines.len());

    let mut intern =
        |slots: &mut Vec<(u8, i32, i32)>, base: u8, off: i32, size: i32| -> Option<usize> {
            if let Some(k) = slots
                .iter()
                .position(|&(b, o, s)| b == base && o == off && s == size)
            {
                return Some(k);
            }
            if slots.len() >= MAX_SLOTS {
                return None;
            }
            slots.push((base, off, size));
            Some(slots.len() - 1)
        };

    for (pos, &n) in lines.iter().enumerate() {
        let t = infos[n].trimmed(store.get(n));
        let next = lines.get(pos + 1).copied();
        let fall: Vec<usize> = next.into_iter().collect();
        let mut ops: Vec<(usize, bool)> = Vec::new();

        let (eff, edges) = match infos[n].kind {
            LineKind::InlineAsm => (
                Effect {
                    reads: u64::MAX,
                    writes: 0,
                },
                fall,
            ),
            LineKind::Label => (
                Effect {
                    reads: 0,
                    writes: 0,
                },
                fall,
            ),
            LineKind::Ret => (
                Effect {
                    reads: ret_reads,
                    writes: 0,
                },
                Vec::new(),
            ),
            LineKind::Call => (
                Effect {
                    reads: call_xmm_reads(store, infos, start, n) | xmm_mentions(t),
                    writes: XMM_ALL,
                },
                fall,
            ),
            LineKind::JmpIndirect => {
                return facts; // unknown target: unanalysable
            }
            LineKind::Jmp => {
                let target = t.split_whitespace().nth(1).unwrap_or("");
                if target.starts_with('*') {
                    return facts;
                }
                let Some(idx) = resolve(target) else {
                    return facts; // tail call / foreign label
                };
                (
                    Effect {
                        reads: 0,
                        writes: 0,
                    },
                    vec![idx],
                )
            }
            LineKind::CondJmp => {
                let target = t.split_whitespace().nth(1).unwrap_or("");
                let Some(idx) = resolve(target) else {
                    return facts;
                };
                let mut edges = fall;
                edges.push(idx);
                (
                    Effect {
                        reads: 0,
                        writes: 0,
                    },
                    edges,
                )
            }
            _ => {
                let (mn, operands) = split_mnemonic(t);
                let mentioned = xmm_mentions(t);
                let mut reads = mentioned;
                let mut writes = 0u64;
                let dest = dest_operand(operands);
                if let Some(d) = dest {
                    if let Some(dn) = xmm_index(d) {
                        let bit = 1u64 << dn;
                        let srcs = match last_top_level_comma(operands.as_bytes()) {
                            Some(c) => &operands[..c],
                            None => "",
                        };
                        let src_mask = xmm_mentions(srcs);
                        // Zero idiom `xorpd %x,%x` / `vxorpd %x,%x,%x` /
                        // `pxor %x,%x` / `psubd %x,%x`: the sources ARE the
                        // destination but the result is independent of it.
                        let zero_idiom = matches!(
                            mn,
                            "xorpd"
                                | "xorps"
                                | "pxor"
                                | "vxorpd"
                                | "vxorps"
                                | "vpxor"
                                | "pcmpeqd"
                                | "vpcmpeqd"
                                | "pcmpeqb"
                                | "vpcmpeqb"
                        ) && src_mask == bit
                            && operand_list(operands)
                                .iter()
                                .all(|o| xmm_index(o) == Some(dn));
                        if zero_idiom {
                            writes |= bit;
                            reads &= !bit;
                        } else if is_full_xmm_def(mn, operands) {
                            writes |= bit;
                            if src_mask & bit == 0 {
                                reads &= !bit;
                            }
                        }
                        // else: merge / destructive form — dest stays read.
                    }
                }
                // `blendv*` (legacy) reads %xmm0 implicitly.
                if mn == "blendvpd" || mn == "blendvps" || mn == "pblendvb" {
                    reads |= 1;
                }
                // Frame slot traffic.
                if !opaque {
                    let list = operand_list(operands);
                    let n_ops = list.len();
                    for (k, op) in list.iter().enumerate() {
                        match frame_operand(op) {
                            Err(()) => {
                                opaque = true;
                                break;
                            }
                            Ok(None) => {}
                            Ok(Some((base, off))) => {
                                if base == 5 && !rbp_is_frame || base == 4 && !track_rsp {
                                    opaque = true;
                                    break;
                                }
                                if mn.starts_with("lea") {
                                    opaque = true;
                                    break;
                                }
                                let width = access_width(mn, operands, op);
                                let is_dest = k + 1 == n_ops && n_ops >= 2;
                                let is_write = is_dest && is_plain_store(mn);
                                match intern(&mut facts.slots, base, off, width) {
                                    Some(si) => ops.push((si, is_write)),
                                    None => {
                                        opaque = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                (Effect { reads, writes }, fall)
            }
        };
        effects.push(eff);
        succs.push(edges);
        slot_ops.push(ops);
    }
    facts.slots_opaque = opaque;

    // Fold slot ops into effects now that every slot is interned (needed so
    // that a READ of slot A also keeps any OVERLAPPING slot B live, and a
    // covering write of B kills A).  A `call` reads every %rsp-based slot:
    // the outgoing-argument area lives there and the callee reads it without
    // any textual trace in this function.
    if !opaque {
        let slots = facts.slots.clone();
        let rsp_slot_mask: u64 = slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.0 == 4)
            .fold(0u64, |m, (k, _)| m | (1u64 << (16 + k)));
        for (pos, ops) in slot_ops.iter().enumerate() {
            let mut r = 0u64;
            let mut w = 0u64;
            if infos[lines[pos]].kind == LineKind::Call {
                r |= rsp_slot_mask;
            }
            for &(si, is_write) in ops {
                let (b, o, s) = slots[si];
                for (k, &(b2, o2, s2)) in slots.iter().enumerate() {
                    if b2 != b || !ranges_overlap(o, s, o2, s2) {
                        continue;
                    }
                    if is_write {
                        if o <= o2 && o + s >= o2 + s2 {
                            w |= 1u64 << (16 + k); // covering store kills
                        } else {
                            r |= 1u64 << (16 + k); // partial write: old bytes live
                        }
                    } else {
                        r |= 1u64 << (16 + k);
                    }
                }
            }
            effects[pos].reads |= r;
            effects[pos].writes |= w & !r;
        }
    }

    // ── backward dataflow ─────────────────────────────────────────────────
    let idx_of = |n: usize| -> usize { n - start };
    let mut line_pos = vec![usize::MAX; n_lines];
    for (pos, &n) in lines.iter().enumerate() {
        line_pos[idx_of(n)] = pos;
    }
    let mut live_in = vec![0u64; lines.len()];
    let mut live_out = vec![0u64; lines.len()];
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 64 {
        changed = false;
        rounds += 1;
        for pos in (0..lines.len()).rev() {
            let mut out = 0u64;
            for &s in &succs[pos] {
                // A successor line index may be a label (in `lines`) — map it.
                let sp = line_pos[idx_of(s)];
                if sp == usize::MAX {
                    // Target is a nop'd/directive line: walk to the next real one.
                    let mut k = s;
                    let mut found = None;
                    while k < end {
                        let p = line_pos[idx_of(k)];
                        if p != usize::MAX {
                            found = Some(p);
                            break;
                        }
                        k += 1;
                    }
                    match found {
                        Some(p) => out |= live_in[p],
                        None => out |= ret_reads,
                    }
                } else {
                    out |= live_in[sp];
                }
            }
            if succs[pos].is_empty() && !matches!(infos[lines[pos]].kind, LineKind::Ret) {
                // Fell off the end of the function text.  Compiled code
                // never executes past its last line: a function ends in
                // `ret`, a tail `jmp`, or a noreturn `call`/`ud2`/`hlt`,
                // all of which are modelled explicitly (a `ret` reads the
                // return registers; an unresolvable `jmp` makes the
                // function unknown; a noreturn call has no live-out).  The
                // only texts that reach here are fragments (unit tests,
                // hand-written stubs) whose end is the end of observation:
                // nothing is live.
            }
            let inn = effects[pos].reads | (out & !effects[pos].writes);
            if out != live_out[pos] || inn != live_in[pos] {
                live_out[pos] = out;
                live_in[pos] = inn;
                changed = true;
            }
        }
    }
    if changed {
        debug_unknown(store, start, "no convergence");
        return facts; // no convergence: stay unknown
    }
    // Materialise per-line facts; directive/nop lines inherit the facts of
    // the next real line so a query "after line k" is well-defined for any k.
    let mut carry = 0u64;
    for n in (start..end).rev() {
        let p = line_pos[idx_of(n)];
        if p != usize::MAX {
            facts.live_out[idx_of(n)] = live_out[p];
            carry = live_in[p];
        } else {
            facts.live_out[idx_of(n)] = carry;
        }
    }
    facts.known = true;
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRO: &str =
        "f:\n.cfi_startproc\n    pushq %rbp\n    movq %rsp, %rbp\n    subq $64, %rsp\n";
    const EPI: &str = "    movq %rbp, %rsp\n    popq %rbp\n    ret\n.cfi_endproc\n";

    fn build(body: &str) -> (LineStore, Vec<LineInfo>, FpLiveness) {
        let asm = format!("{PRO}{body}{EPI}");
        let store = LineStore::new(asm);
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let lv = FpLiveness::new(&store, &infos);
        (store, infos, lv)
    }

    fn line_of(store: &LineStore, needle: &str) -> usize {
        (0..store.len())
            .find(|&i| store.get(i).contains(needle))
            .expect("line")
    }

    #[test]
    fn mentions_and_index_parse_xmm_and_ymm() {
        assert_eq!(
            xmm_mentions("vaddsd %xmm12, %xmm3, %xmm3"),
            (1 << 12) | (1 << 3)
        );
        assert_eq!(xmm_mentions("vmovupd (%rdi), %ymm1"), 1 << 1);
        assert_eq!(xmm_mentions("movq %rax, -8(%rbp)"), 0);
        assert_eq!(xmm_index("%xmm15"), Some(15));
        assert_eq!(xmm_index("%xmm16"), None);
        assert_eq!(xmm_index("%ymm0"), Some(0));
        assert_eq!(xmm_index("(%rax)"), None);
    }

    #[test]
    fn full_def_classification() {
        assert!(is_full_xmm_def("movsd", "(%rax), %xmm1"));
        assert!(
            !is_full_xmm_def("movsd", "%xmm2, %xmm1"),
            "reg-reg movsd merges"
        );
        assert!(
            is_full_xmm_def("vmovss", "%xmm3, %xmm3, %xmm1"),
            "VEX 3-operand reg form defines D from its two sources"
        );
        assert!(
            !is_full_xmm_def("vmovss", "%xmm3, %xmm1"),
            "VEX 2-operand reg form is a merge"
        );
        assert!(is_full_xmm_def("movapd", "%xmm2, %xmm1"));
        assert!(is_full_xmm_def("vaddsd", "%xmm2, %xmm1, %xmm0"));
        assert!(
            !is_full_xmm_def("addsd", "%xmm2, %xmm1"),
            "legacy ALU is destructive"
        );
        assert!(
            !is_full_xmm_def("cvtsi2sdq", "%rax, %xmm1"),
            "cvtsi2sd merges"
        );
        assert!(!is_full_xmm_def("vfmadd231sd", "%xmm2, %xmm1, %xmm0"));
    }

    /// The `lc.c` shape: the loaded register's only textual mention after
    /// the fold is the adjacent consumer, but the loop header reads it
    /// through the back edge.
    #[test]
    fn back_edge_reader_keeps_xmm_live() {
        let (store, _i, lv) = build(
            "    xorpd %xmm4, %xmm4\n\
             .LBB1:\n\
             \x20   cmpl %esi, %ecx\n\
             \x20   jge .LBB3\n\
             \x20   vmulsd .LC0(%rip), %xmm4, %xmm5\n\
             \x20   vaddsd %xmm5, %xmm0, %xmm0\n\
             \x20   movsd (%rdi,%r8), %xmm4\n\
             \x20   vsubsd %xmm4, %xmm3, %xmm3\n\
             \x20   addl $1, %ecx\n\
             \x20   jmp .LBB1\n\
             .LBB3:\n",
        );
        let n = line_of(&store, "vsubsd %xmm4");
        assert_eq!(
            lv.xmm_live_after(n, 4),
            Some(true),
            "%xmm4 is read at the loop top"
        );
        assert_eq!(
            lv.xmm_live_after(n, 5),
            Some(false),
            "%xmm5 is redefined before use"
        );
    }

    #[test]
    fn straight_line_value_is_dead_after_last_use() {
        let (store, _i, lv) = build(
            "    movsd (%rdi), %xmm5\n\
             \x20   vaddsd %xmm5, %xmm4, %xmm4\n\
             \x20   movapd %xmm4, %xmm0\n",
        );
        let n = line_of(&store, "vaddsd %xmm5");
        assert_eq!(lv.xmm_live_after(n, 5), Some(false));
        assert_eq!(lv.xmm_live_after(n, 4), Some(true));
        let m = line_of(&store, "movapd %xmm4, %xmm0");
        assert_eq!(
            lv.xmm_live_after(m, 0),
            Some(true),
            "return register live at ret"
        );
        assert_eq!(lv.xmm_live_after(m, 4), Some(false));
        assert_eq!(
            lv.xmm_live_after(m, 1),
            Some(false),
            "scalar return: %xmm1 not returned"
        );
    }

    /// `double _Complex` / two-double aggregate return: the tail block
    /// materialises %xmm1, so it is live at `ret`.
    #[test]
    fn two_register_fp_return_keeps_xmm1_live() {
        let (store, _i, lv) = build(
            "    movsd (%rdi), %xmm1\n\
             \x20   vaddsd %xmm1, %xmm4, %xmm4\n\
             \x20   movapd %xmm4, %xmm0\n",
        );
        let n = line_of(&store, "vaddsd %xmm1");
        assert_eq!(lv.xmm_live_after(n, 1), Some(true));
    }

    /// An integer return (tail block materialises %eax, names no XMM) reads
    /// no XMM register at `ret`.
    #[test]
    fn integer_return_reads_no_xmm() {
        let (store, _i, lv) = build("    movsd (%rdi), %xmm5\n    movl $1, %eax\n");
        let n = line_of(&store, "movl $1, %eax");

        assert_eq!(lv.xmm_live_after(n, 0), Some(false));
        assert_eq!(lv.xmm_live_after(n, 1), Some(false));
        assert_eq!(lv.xmm_live_after(n, 5), Some(false));
    }

    /// A tail block that materialises nothing (shared epilogue reached by
    /// `jmp`, value produced in an earlier block) stays conservative.
    #[test]
    fn unmaterialised_return_is_conservative() {
        let (store, _i, lv) = build(
            "    movsd (%rdi), %xmm0\n\
             \x20   jmp .Lepi\n\
             .Lepi:\n\
             \x20   nop\n",
        );
        let n = line_of(&store, "nop");
        assert_eq!(lv.xmm_live_after(n, 0), Some(true));
        assert_eq!(lv.xmm_live_after(n, 1), Some(true));
    }

    /// The prologue marker overrides tail-block inference: a shared
    /// epilogue reached by `jmp` materialises nothing, yet the marker says
    /// the function returns an integer.
    #[test]
    fn ret_marker_is_authoritative() {
        let (store, _i, lv) = build(
            "    # LCCC_RET_XMM 0\n\
             \x20   movsd (%rdi), %xmm0\n\
             \x20   jmp .Lepi\n\
             .Lepi:\n\
             \x20   nop\n",
        );
        let n = line_of(&store, "nop");
        assert_eq!(lv.xmm_live_after(n, 0), Some(false));
        assert_eq!(lv.xmm_live_after(n, 1), Some(false));
        let (store, _i, lv) = build(
            "    # LCCC_RET_XMM 3\n\
             \x20   movsd (%rdi), %xmm0\n\
             \x20   movsd 8(%rdi), %xmm1\n\
             \x20   movl $1, %eax\n",
        );
        let n = line_of(&store, "movl $1, %eax");
        assert_eq!(lv.xmm_live_after(n, 0), Some(true));
        assert_eq!(lv.xmm_live_after(n, 1), Some(true));
    }

    /// Mixed INTEGER+SSE aggregate: `movq %rax, %xmm0; movq %rdx, %rax` —
    /// %xmm0 live, %xmm1 not.
    #[test]
    fn mixed_aggregate_return_keeps_xmm0_only() {
        let (store, _i, lv) = build("    movq %rax, %xmm0\n    movq %rdx, %rax\n");
        let n = line_of(&store, "movq %rdx, %rax");
        assert_eq!(lv.xmm_live_after(n, 0), Some(true));
        assert_eq!(lv.xmm_live_after(n, 1), Some(false));
    }

    #[test]
    fn call_reads_argument_registers_and_kills_all() {
        let (store, _i, lv) = build(
            "    movsd (%rdi), %xmm1\n\
             \x20   movapd %xmm1, %xmm9\n\
             \x20   call g@PLT\n\
             \x20   vaddsd %xmm9, %xmm0, %xmm0\n",
        );
        let n = line_of(&store, "movapd %xmm1, %xmm9");
        assert_eq!(
            lv.xmm_live_after(n, 1),
            Some(true),
            "%xmm1 is an argument register"
        );
        assert_eq!(
            lv.xmm_live_after(n, 9),
            Some(false),
            "call clobbers %xmm9: value dead"
        );
        assert_eq!(lv.xmm_live_after(n, 11), Some(false));
    }

    /// The `movb $N, %al` census bounds the call's implicit XMM reads:
    /// `%xmm3` is not an argument of a one-FP-argument call.
    #[test]
    fn call_census_bounds_argument_reads() {
        let (store, _i, lv) = build(
            "    movsd (%rdi), %xmm3\n\
             \x20   vmulsd %xmm3, %xmm2, %xmm2\n\
             \x20   movapd %xmm2, %xmm0\n\
             \x20   movb $1, %al\n\
             \x20   addq $16, %rsp\n\
             \x20   call printf@PLT\n\
             \x20   xorpd %xmm0, %xmm0\n",
        );
        let n = line_of(&store, "vmulsd %xmm3");
        assert_eq!(
            lv.xmm_live_after(n, 3),
            Some(false),
            "census says one FP arg"
        );
        assert_eq!(lv.xmm_live_after(n, 2), Some(true), "feeds %xmm0 = arg 0");
        let m = line_of(&store, "movapd %xmm2, %xmm0");
        assert_eq!(lv.xmm_live_after(m, 0), Some(true));
        assert_eq!(lv.xmm_live_after(m, 1), Some(false));
    }

    #[test]
    fn variadic_zero_census_reads_no_xmm() {
        let (store, _i, lv) = build(
            "    movsd (%rdi), %xmm0\n\
             \x20   vmulsd %xmm0, %xmm2, %xmm2\n\
             \x20   xorl %eax, %eax\n\
             \x20   call printf@PLT\n\
             \x20   xorpd %xmm0, %xmm0\n",
        );
        let n = line_of(&store, "vmulsd %xmm0");
        assert_eq!(lv.xmm_live_after(n, 0), Some(false));
    }

    /// Unmarked calls read the argument registers staged in their own
    /// window (the i128 libcall shape) and nothing else.
    #[test]
    fn unmarked_call_reads_registers_staged_in_its_window() {
        let (store, _i, lv) = build(
            "    movsd (%rdi), %xmm5\n\
             \x20   vmulsd %xmm5, %xmm2, %xmm2\n\
             \x20   movq %rax, %xmm0\n\
             \x20   call __fixdfti@PLT\n",
        );
        let n = line_of(&store, "vmulsd %xmm5");
        assert_eq!(
            lv.xmm_live_after(n, 5),
            Some(false),
            "not staged for the call"
        );
        assert_eq!(
            lv.xmm_live_after(n, 0),
            Some(false),
            "redefined by the staging move"
        );
        let m = line_of(&store, "movq %rax, %xmm0");
        assert_eq!(
            lv.xmm_live_after(m, 0),
            Some(true),
            "staged argument is read"
        );
    }

    /// GP-only prototyped call: no census, no XMM staged → no XMM read.
    #[test]
    fn gp_only_call_reads_no_xmm() {
        let (store, _i, lv) = build(
            "    movsd (%rdi), %xmm4\n\
             \x20   vaddss %xmm4, %xmm3, %xmm3\n\
             \x20   movq %r8, %rdi\n\
             \x20   call puts@PLT\n",
        );
        let n = line_of(&store, "vaddss %xmm4");
        assert_eq!(lv.xmm_live_after(n, 4), Some(false));
        assert_eq!(lv.xmm_live_after(n, 0), Some(false));
    }

    /// Indirect call without a census (`__builtin_apply`), external
    /// retpoline thunk and inline-retpoline local-label calls are
    /// conservative.
    #[test]
    fn calls_without_adjacent_census_read_all_argument_registers() {
        for call in [
            "call *%r11",
            "call __x86_indirect_thunk_r10",
            "call .Lrpl_inner_3",
        ] {
            let (store, _i, lv) = build(&format!(
                "    movsd (%rdi), %xmm5\n    vmulsd %xmm5, %xmm2, %xmm2\n    {call}\n"
            ));
            let n = line_of(&store, "vmulsd %xmm5");
            assert_eq!(lv.xmm_live_after(n, 5), Some(true), "{call}");
            assert_eq!(lv.xmm_live_after(n, 9), Some(false), "{call}");
        }
    }

    #[test]
    fn reg_reg_movsd_merge_keeps_destination_live() {
        let (store, _i, lv) = build(
            "    movsd (%rdi), %xmm2\n\
             \x20   movsd %xmm3, %xmm2\n\
             \x20   movapd %xmm2, %xmm0\n",
        );
        let n = line_of(&store, "movsd (%rdi), %xmm2");
        assert_eq!(
            lv.xmm_live_after(n, 2),
            Some(true),
            "upper bits observed via merge"
        );
    }

    /// `t2.c`: `mulsd -112(%rbp)` READS the slot although it is not a load
    /// mnemonic; a same-offset store after it must not make it dead.
    #[test]
    fn alu_memory_operand_reads_slot() {
        let (store, _i, lv) = build(
            "    movsd %xmm0, -112(%rbp)\n\
             \x20   movsd (%rax), %xmm0\n\
             \x20   mulsd -112(%rbp), %xmm0\n\
             \x20   movq %rax, -112(%rbp)\n",
        );
        let n = line_of(&store, "movsd %xmm0, -112(%rbp)");
        assert_eq!(lv.slot_live_after(n, 5, -112, 8), Some(true));
        let m = line_of(&store, "mulsd -112(%rbp)");
        assert_eq!(
            lv.slot_live_after(m, 5, -112, 8),
            Some(false),
            "covering store kills it"
        );
    }

    /// `patE.c` / `patE2.c`: slot read at the loop header via the back edge.
    #[test]
    fn slot_read_through_back_edge_is_live() {
        let (store, _i, lv) = build(
            ".LBB1:\n\
             \x20   cmpl %esi, %ecx\n\
             \x20   jge .LBB3\n\
             \x20   movsd -80(%rbp), %xmm0\n\
             \x20   movq (%rax), %rax\n\
             \x20   movq %rax, -80(%rbp)\n\
             \x20   movq %rax, %xmm1\n\
             \x20   addsd %xmm1, %xmm0\n\
             \x20   jmp .LBB1\n\
             .LBB3:\n",
        );
        let n = line_of(&store, "movq %rax, -80(%rbp)");
        assert_eq!(lv.slot_live_after(n, 5, -80, 8), Some(true));
    }

    #[test]
    fn partial_overlap_is_a_read_covering_store_is_a_kill() {
        let (store, _i, lv) = build(
            "    movq %rax, -16(%rbp)\n\
             \x20   movl %ecx, -12(%rbp)\n\
             \x20   movq %rdx, -16(%rbp)\n\
             \x20   movsd -16(%rbp), %xmm0\n",
        );
        let a = line_of(&store, "movq %rax, -16(%rbp)");
        assert_eq!(
            lv.slot_live_after(a, 5, -16, 8),
            Some(true),
            "partial write keeps low bytes live"
        );
        let b = line_of(&store, "movl %ecx, -12(%rbp)");
        assert_eq!(
            lv.slot_live_after(b, 5, -16, 8),
            Some(false),
            "covering movq kills"
        );
    }

    #[test]
    fn address_escape_makes_slots_opaque_but_registers_exact() {
        let (store, _i, lv) = build(
            "    leaq -16(%rbp), %rdi\n\
             \x20   movq %rax, -16(%rbp)\n\
             \x20   movsd (%rsi), %xmm3\n\
             \x20   vaddsd %xmm3, %xmm0, %xmm0\n",
        );
        let n = line_of(&store, "movq %rax, -16(%rbp)");
        assert_eq!(lv.slot_live_after(n, 5, -16, 8), Some(true));
        let m = line_of(&store, "vaddsd %xmm3");
        assert_eq!(lv.xmm_live_after(m, 3), Some(false));
    }

    #[test]
    fn indirect_jump_makes_function_unknown() {
        let (store, _i, lv) = build("    movsd (%rdi), %xmm5\n    jmpq *%rax\n");
        let n = line_of(&store, "movsd (%rdi), %xmm5");
        assert_eq!(lv.xmm_live_after(n, 5), None);
        assert_eq!(lv.slot_live_after(n, 5, -8, 8), None);
    }

    /// A fragment without any function delimiter is analysed as one
    /// function; a fragment that ends without `ret` behaves as if it
    /// returned (return registers live, everything else dead).
    #[test]
    fn undelimited_fragment_is_one_function() {
        let store = LineStore::new(
            "    movsd (%rdi), %xmm5\n    vaddsd %xmm5, %xmm4, %xmm4\n    movsd 8(%rdi), %xmm5\n"
                .to_string(),
        );
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let lv = FpLiveness::new(&store, &infos);
        assert_eq!(
            lv.xmm_live_after(1, 5),
            Some(false),
            "redefined by the next load"
        );
        assert_eq!(
            lv.xmm_live_after(2, 5),
            Some(false),
            "fell off the end: nothing live"
        );
        assert_eq!(
            lv.xmm_live_after(2, 0),
            Some(false),
            "no ret → no return-register read"
        );
    }

    /// Code outside every delimited function is not analysed.
    #[test]
    fn code_outside_delimited_functions_answers_none() {
        let asm = "    movsd (%rdi), %xmm5\nf:\n.cfi_startproc\n    ret\n.cfi_endproc\n";
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let lv = FpLiveness::new(&store, &infos);
        assert_eq!(lv.xmm_live_after(0, 5), None);
    }

    #[test]
    fn frame_pointer_less_rsp_slots_are_tracked() {
        let asm = "g:\n.cfi_startproc\n    subq $24, %rsp\n    movsd %xmm0, 8(%rsp)\n    movsd (%rdi), %xmm0\n    addsd 8(%rsp), %xmm0\n    addq $24, %rsp\n    ret\n.cfi_endproc\n";
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let lv = FpLiveness::new(&store, &infos);
        let n = line_of(&store, "movsd %xmm0, 8(%rsp)");
        assert_eq!(lv.slot_live_after(n, 4, 8, 8), Some(true));
        let m = line_of(&store, "addsd 8(%rsp)");
        assert_eq!(lv.slot_live_after(m, 4, 8, 8), Some(false));
    }

    #[test]
    fn refresh_tracks_a_rewrite() {
        let (mut store, mut infos, mut lv) = build(
            "    movsd (%rdi), %xmm5\n\
             \x20   vaddsd %xmm5, %xmm4, %xmm4\n\
             \x20   movapd %xmm4, %xmm0\n",
        );
        let n = line_of(&store, "vaddsd %xmm5");
        assert_eq!(lv.xmm_live_after(n, 5), Some(false));
        replace_line(
            &mut store,
            &mut infos[n + 1],
            n + 1,
            "    vaddsd %xmm5, %xmm4, %xmm0".to_string(),
        );
        lv.refresh_at(&store, &infos, n + 1);
        assert_eq!(lv.xmm_live_after(n, 5), Some(true));
    }
}
