//! Exact intra-function liveness of XMM registers **and frame slots** for the
//! scalar-FP peephole passes.
//!
//! # Why this exists
//!
//! [`super::liveness::FileLiveness`] answers "is GP register F read on any
//! path from here?" with a real backward dataflow over the function's CFG.
//! The scalar-FP folds (`fold_fp_register_loads`, `fold_fma_memory_operand`,
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
//! One backward dataflow per function over a combined 128-bit fact vector:
//!
//! * bits `0..16`  — `%xmm0`..`%xmm15` (a `%ymmN` or `%zmmN` mention aliases
//!   `%xmmN`: all three spellings name the same physical register's low 128
//!   bits, and an EVEX writemask makes the destination a partial write, so it
//!   stays a read);
//! * bits `16..64` — frame slots, interned per function as `(base, offset,
//!   byte-size)` triples.  A slot is *read* by any instruction whose text
//!   contains a memory operand overlapping it, unless that instruction is a
//!   plain store that *covers* it (a covering store is a kill).  Indexed
//!   frame operands (`8(%rsp,%rax)`) and any address escape (`leaq
//!   N(%rbp)`, raw `%rsp`/`%rbp` copies) make every slot of the function
//!   permanently live (fail closed).
//!
//! * bits `64..80` and `80..96` — bits 128..256 and 256..512 of each
//!   vector register. Legacy XMM definitions preserve both regions. VEX/EVEX
//!   full definitions and upper-zeroing effects kill them independently.
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
//! * implicit blend/SHA inputs and vector-state saves are reads; FXSAVE-format
//!   restores kill only low XMM state, while XRSTOR-family writes depend on
//!   the runtime component mask and therefore prove no unconditional kill;
//!   unknown vector operations read all facts locally and prove no kills;
//! * inline asm reads everything and proves no kills;
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

#[cfg(test)]
thread_local! {
    static REFRESH_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn take_refresh_count() -> usize {
    REFRESH_COUNT.with(|count| count.replace(0))
}

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
    live_out: Vec<u128>,
    /// Every slot is pinned live (address escape / indexed frame access /
    /// mid-body %rsp shift).  Register facts stay exact.
    slots_opaque: bool,
    known: bool,
}

/// File-wide XMM + frame-slot liveness.
pub(super) struct FpLiveness {
    funcs: Vec<FnFacts>,
}

const XMM_ALL: u128 = 0xFFFF;
// Keep slot bits 16..64 unchanged; track architectural upper regions separately.
const YMM_HIGH_SHIFT: u32 = 64;
const ZMM_HIGH_SHIFT: u32 = 80;
const UPPER_ALL: u128 = (XMM_ALL << YMM_HIGH_SHIFT) | (XMM_ALL << ZMM_HIGH_SHIFT);
const VECTOR_ALL: u128 = XMM_ALL | UPPER_ALL;

fn upper_regions(low: u128) -> u128 {
    (low << YMM_HIGH_SHIFT) | (low << ZMM_HIGH_SHIFT)
}
/// `%xmm0..%xmm7`: SysV FP argument registers a `call` may read implicitly.
const CALL_READS: u128 = 0x00FF;
/// `%xmm0`, `%xmm1`: FP return registers live at `ret` (a two-double
/// aggregate or `double _Complex` returns in both).
const RET_READS: u128 = 0x0003;
/// `%xmm0` only: the ret-live set of a function whose every `ret` tail
/// block materialises `%xmm0` and never touches `%xmm1` — the signature of
/// a scalar FP return.  Mirrors `FileLiveness::returns_in_rax_only`.
const RET_READS_XMM0_ONLY: u128 = 0x0001;
const MAX_SLOTS: usize = 48; // bits 16..64; upper vector regions occupy 64..96

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
            // DIRECTIVE-LAW: selecting -- this loop *reads* directive lines to find
            // `.cfi_startproc`; it never passes one over, so the opacity rule for
            // windows does not apply here.
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
                // DIRECTIVE-LAW: selecting -- the scan is looking for the
                // function's closing `.cfi_endproc`/`.size`, not stepping over it.
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
        Some(f.live_out[idx - f.start] & (1u128 << n) != 0)
    }

    /// A VEX/EVEX XMM move writes both the low region and architectural
    /// upper zeros. Low-XMM death alone cannot justify removing that write.
    /// Unknown control flow refuses deletion rather than guessing about upper
    /// observations. Legacy moves keep the established low-region query.
    pub(super) fn reg_move_dead_after(
        &self,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
        n: u32,
    ) -> bool {
        let t = infos[idx].trimmed(store.get(idx));
        if !t.starts_with('v') {
            return self.xmm_dead_after(store, infos, idx, n, &[idx]);
        }
        if n >= 16 {
            return false;
        }
        let Some(f) = self.func_of(idx).filter(|f| f.known) else {
            return false;
        };
        let low = 1u128 << n;
        f.live_out[idx - f.start] & (low | upper_regions(low)) == 0
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
            if b == base && ranges_overlap(offset, size, o, s) && bits & (1u128 << (16 + k)) != 0 {
                return Some(true);
            }
        }
        Some(false)
    }

    /// Whole write-region proof for a deletion/retargeting window. A legacy
    /// write changes at most low XMM; any VEX/EVEX write in the owned window
    /// to the queried register can also change architectural upper zeros.
    /// Conservatively require all upper regions dead in that case. This is intentionally stronger than
    /// a low-128 query, including for scalar VEX producers being retargeted.
    /// `owned` names every instruction whose write is removed or redirected;
    /// `idx` is the last instruction of that transformation window.
    pub(super) fn deleted_write_regions_dead_after(
        &self,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
        n: u32,
        owned: &[usize],
    ) -> bool {
        if n >= 16 {
            return false;
        }
        let Some(f) = self.func_of(idx).filter(|f| f.known) else {
            return false;
        };
        let low = 1u128 << n;
        let has_upper_write = owned.iter().any(|&i| {
            let text = infos[i].trimmed(store.get(i));
            let (mn, operands) = split_mnemonic(text);
            has_implicit_vector_effects_line(text)
                || mn.starts_with('v')
                    && dest_operand(operands)
                        .and_then(|dst| xmm_index(dst.split('{').next().unwrap_or(dst).trim_end()))
                        == Some(n)
        });
        let footprint = low
            | if has_upper_write {
                upper_regions(low)
            } else {
                0
            };
        f.live_out[idx - f.start] & footprint == 0
    }

    /// Low-128 query only: not a whole-instruction deletion proof. Keep this
    /// private so consumers cannot accidentally ignore architectural zeros.
    /// Unknown CFG cannot prove deadness: textual register absence misses
    /// implicit architectural readers (state saves, SHA/blends, opaque ops).
    fn xmm_dead_after(
        &self,
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
        n: u32,
        _owned: &[usize],
    ) -> bool {
        let ans = self.xmm_live_after(idx, n);
        if ans == Some(true) && std::env::var_os("CCC_DEBUG_FP_LIVENESS").is_some() {
            self.explain_live(store, infos, idx, n);
        }
        ans == Some(false)
    }

    /// `CCC_DEBUG_FP_LIVENESS=1`: say why `%xmmN` is live after `idx` — the
    /// first later line on a live path that reads it, or, when no such
    /// textual reader exists (back edge / call / ret), the live ranges of
    /// the register inside the function.
    fn explain_live(&self, store: &LineStore, infos: &[LineInfo], idx: usize, n: u32) {
        let Some(f) = self.func_of(idx) else { return };
        let bit = 1u128 << n;
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
        #[cfg(test)]
        REFRESH_COUNT.with(|count| count.set(count.get() + 1));
        self.funcs[pos - 1] = analyse(store, infos, start, end);
    }
}

/// Parse `%xmmN` / `%ymmN` / `%zmmN` (N < 16) at the START of `op`; returns N.
///
/// All three spellings name the same physical register's low 128 bits, so all
/// three must report the same N: an AVX-512 `vaddpd %zmm3, %zmm4, %zmm5` reads
/// `%xmm3` and `%xmm4` and writes `%xmm5` exactly as the `%ymm` spelling does.
/// Missing the `%zmm` spelling made this oracle answer "dead" for a register a
/// later EVEX instruction still read, and every consumer of that answer
/// (`eliminate_fp_xmm_roundtrips`, the dead-write and store-forwarding folds)
/// deletes code on the strength of it.
///
/// N >= 16 returns `None` on purpose: `%xmm16`..`%xmm31` exist only under
/// AVX-512, this model tracks 16 registers, and an untracked register is a
/// distinct physical one — ignoring it cannot make a tracked register look dead.
fn xmm_index(op: &str) -> Option<u32> {
    let rest = op
        .strip_prefix("%xmm")
        .or_else(|| op.strip_prefix("%ymm"))
        .or_else(|| op.strip_prefix("%zmm"))?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 2 || rest.len() != digits {
        return None;
    }
    let n: u32 = rest.parse().ok()?;
    (n < 16).then_some(n)
}

/// Bitmask of every `%xmmN`/`%ymmN`/`%zmmN` (N < 16) mentioned anywhere in `t`.
///
/// The `%zmm` spelling is included for the aliasing reason given on
/// [`xmm_index`].  A mention inside an EVEX writemask suffix (`%zmm3{%k1}`)
/// counts as a mention too, which is what keeps masked writes conservative.
fn xmm_mentions(t: &str) -> u128 {
    let b = t.as_bytes();
    let mut mask = 0u128;
    let mut i = 0;
    while i + 4 < b.len() {
        if b[i] == b'%'
            && (b[i + 1] == b'x' || b[i + 1] == b'y' || b[i + 1] == b'z')
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
                mask |= 1u128 << n;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    mask
}

/// True when `t` writes its destination XMM register WITHOUT reading it
/// (see `is_full_xmm_def`). Exported for `vector_copy::propagate_vector_copies`:
/// before renaming reads at a destination write it must distinguish true full
/// defs (`vmulsd %xmmA, %xmmB, %xmmD` — safe to rename through) from
/// destination-reading writes (`shufps $0x00, %x, %x` — renaming hides the
/// implicit destination read and later passes mis-kill its feeder).
pub(super) fn is_full_xmm_def_line(t: &str) -> bool {
    let (mn, ops) = split_mnemonic(t);
    is_full_xmm_def(mn, ops)
}

/// True when textual source/trailing-destination roles miss register effects.
/// Copy propagation, bracket coalescing and private-upper-lane proofs must
/// stop at these effects too; preserving a feeder in DCE alone is insufficient.
pub(super) fn has_implicit_vector_effects_line(t: &str) -> bool {
    let (mn, operands) = split_mnemonic(t);
    let base = mn.strip_prefix('v').unwrap_or(mn);
    state_image_register_reads(mn).is_some()
        // AVX2 gather masks are explicit but nontrailing read/write outputs.
        // Treat them as local barriers until consumers model both outputs;
        // merely keeping the mask read live in DCE does not protect equality
        // facts in copy propagation and register-backed spill forwarding.
        // The EVEX spellings may use k masks instead; refusing them here is
        // conservative and avoids inferring encoding from a mnemonic alone.
        || matches!(
            base,
            "pcmpistrm" | "pcmpestrm" | "zeroupper" | "zeroall"
                | "gatherdps" | "gatherdpd" | "gatherqps" | "gatherqpd"
                | "pgatherdd" | "pgatherdq" | "pgatherqd" | "pgatherqq"
        )
        || implicit_fp_reads(mn, operands, xmm_mentions(t)) & VECTOR_ALL != 0
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
        // Immediate-operand shuffles read their SOURCE plus an immediate and
        // write the whole destination (`pshufd $0x0E, %xmm15, %xmm0` builds
        // xmm0 from xmm15 alone) — full defs, unlike the register-mask
        // interleave family (`unpckl*`, `pshufb`, `palignr`) which folds
        // into the destructive 2-operand form and READS the destination.
        // `shufps`/`shufpd` do NOT belong here: their legacy 2-operand form
        // takes result lanes 0..1 from the DESTINATION's old value
        // (`shufps $0x00, %xmm1, %xmm0` is [xmm0[0], xmm0[0], xmm1[0],
        // xmm1[0]]) — a hidden dst read every bit as real as `unpcklps`'.
        // Misclassifying them as full defs let dead-FP-move elimination drop
        // the `vmovss` reset feeding a splat and turn it into
        // [s, s, t, t] (bb_slp_v8 C2 miscompile). The VEX 3-operand
        // spellings are true full defs and classify above.
        "pshufd" | "pshufhw" | "pshuflw" => true,
        // Packed conversions define the whole destination.
        "cvtdq2pd" | "cvtdq2ps" | "cvtpd2ps" | "cvtps2pd" | "cvttpd2dq" | "cvttps2dq"
        | "cvtpd2dq" | "cvtps2dq" | "vcvtdq2pd" | "vcvtdq2ps" | "vcvtpd2ps" | "vcvtps2pd"
        | "vcvttpd2dq" | "vcvttps2dq" | "vcvtpd2dq" | "vcvtps2dq" => true,
        // Scalar int→fp conversions merge into the upper lanes: READ.
        "cvtsi2sd" | "cvtsi2ss" | "cvtsi2sdq" | "cvtsi2ssq" | "cvtsi2sdl" | "cvtsi2ssl"
        | "cvtsd2ss" | "cvtss2sd" | "sqrtsd" | "sqrtss" | "roundsd" | "roundss" | "rcpss"
        | "rsqrtss" => false,
        // Admit source-only VEX definitions positively. A leading `v` is NOT
        // an effect certificate: VPCMP{E,I}STR{I,M} reads both explicit vector
        // operands and writes implicit outputs, and future instructions may
        // have similar hidden dependencies. Unknown forms prove no kill.
        _ => mn.strip_prefix('v').is_some_and(source_only_vex_opcode),
    }
}

/// Exact source-only VEX definitions. The same operand families have no
/// hidden vector inputs in legacy SSE spelling (but may read their explicit
/// destination there); keep admission and kill proofs separate.
fn source_only_vex_opcode(base: &str) -> bool {
    matches!(
        base,
        "addpd"
            | "addps"
            | "addsd"
            | "addss"
            | "subpd"
            | "subps"
            | "subsd"
            | "subss"
            | "mulpd"
            | "mulps"
            | "mulsd"
            | "mulss"
            | "divpd"
            | "divps"
            | "divsd"
            | "divss"
            | "minpd"
            | "minps"
            | "minsd"
            | "minss"
            | "maxpd"
            | "maxps"
            | "maxsd"
            | "maxss"
            | "andpd"
            | "andps"
            | "andnpd"
            | "andnps"
            | "orpd"
            | "orps"
            | "xorpd"
            | "xorps"
            | "sqrtpd"
            | "sqrtps"
            | "sqrtsd"
            | "sqrtss"
            | "rcpps"
            | "rcpss"
            | "rsqrtps"
            | "rsqrtss"
            | "cmppd"
            | "cmpps"
            | "cmpsd"
            | "cmpss"
            | "roundpd"
            | "roundps"
            | "roundsd"
            | "roundss"
            | "haddpd"
            | "haddps"
            | "hsubpd"
            | "hsubps"
            | "addsubpd"
            | "addsubps"
            | "cvtsi2sd"
            | "cvtsi2sdq"
            | "cvtsi2sdl"
            | "cvtsi2ss"
            | "cvtsi2ssq"
            | "cvtsi2ssl"
            | "cvtss2sd"
            | "cvtsd2ss"
            | "cvtph2ps"
            | "cvtps2ph"
            | "movlhps"
            | "movhlps"
            | "movdqa32"
            | "movdqu32"
            | "pshufd"
            | "pshufhw"
            | "pshuflw"
            | "pshufb"
            | "shufps"
            | "shufpd"
            | "unpckhpd"
            | "unpckhps"
            | "unpcklpd"
            | "unpcklps"
            | "punpckhbw"
            | "punpckhwd"
            | "punpckhdq"
            | "punpckhqdq"
            | "punpcklbw"
            | "punpcklwd"
            | "punpckldq"
            | "punpcklqdq"
            | "blendpd"
            | "blendps"
            | "blendvpd"
            | "blendvps"
            | "pblendw"
            | "pblendvb"
            | "pblendd"
            | "permpd"
            | "permps"
            | "permd"
            | "permq"
            | "permw"
            | "permb"
            | "perm2f128"
            | "perm2i128"
            | "permilpd"
            | "permilps"
            | "extractf128"
            | "extracti128"
            | "pand"
            | "pandn"
            | "por"
            | "pxor"
            | "pandd"
            | "pandq"
            | "pandnd"
            | "pandnq"
            | "pord"
            | "porq"
            | "pxord"
            | "pxorq"
            | "paddb"
            | "paddw"
            | "paddd"
            | "paddq"
            | "paddsb"
            | "paddsw"
            | "paddusb"
            | "paddusw"
            | "psubb"
            | "psubw"
            | "psubd"
            | "psubq"
            | "psubsb"
            | "psubsw"
            | "psubusb"
            | "psubusw"
            | "pmullw"
            | "pmulld"
            | "pmulhw"
            | "pmulhuw"
            | "pmulhrsw"
            | "pmuldq"
            | "pmuludq"
            | "pmaddwd"
            | "pmaddubsw"
            | "psadbw"
            | "mpsadbw"
            | "pcmpeqb"
            | "pcmpeqw"
            | "pcmpeqd"
            | "pcmpeqq"
            | "pcmpgtb"
            | "pcmpgtw"
            | "pcmpgtd"
            | "pcmpgtq"
            | "psllw"
            | "pslld"
            | "psllq"
            | "pslldq"
            | "psllvd"
            | "psllvq"
            | "psrlw"
            | "psrld"
            | "psrlq"
            | "psrldq"
            | "psrlvd"
            | "psrlvq"
            | "psraw"
            | "psrad"
            | "psravd"
            | "pminsb"
            | "pminsw"
            | "pminsd"
            | "pminub"
            | "pminuw"
            | "pminud"
            | "pmaxsb"
            | "pmaxsw"
            | "pmaxsd"
            | "pmaxub"
            | "pmaxuw"
            | "pmaxud"
            | "pabsb"
            | "pabsw"
            | "pabsd"
            | "pavgb"
            | "pavgw"
            | "packsswb"
            | "packssdw"
            | "packuswb"
            | "packusdw"
            | "phaddw"
            | "phaddd"
            | "phaddsw"
            | "phsubw"
            | "phsubd"
            | "phsubsw"
            | "palignr"
            | "psignb"
            | "psignw"
            | "psignd"
            | "pmovsxbw"
            | "pmovsxbd"
            | "pmovsxbq"
            | "pmovsxwd"
            | "pmovsxwq"
            | "pmovsxdq"
            | "pmovzxbw"
            | "pmovzxbd"
            | "pmovzxbq"
            | "pmovzxwd"
            | "pmovzxwq"
            | "pmovzxdq"
            | "pclmulqdq"
            | "aesenc"
            | "aesenclast"
            | "aesdec"
            | "aesdeclast"
            | "aesimc"
            | "aeskeygenassist"
    )
}

/// FMA3 reads its explicit destination. An exact family/order/type spelling
/// admits explicit inputs, never a full-definition kill.
fn fma3_opcode(base: &str) -> bool {
    ["fmaddsub", "fmsubadd", "fmadd", "fmsub", "fnmadd", "fnmsub"]
        .iter()
        .find_map(|prefix| base.strip_prefix(prefix))
        .is_some_and(|tail| {
            matches!(
                tail,
                "132pd"
                    | "132ps"
                    | "132sd"
                    | "132ss"
                    | "213pd"
                    | "213ps"
                    | "213sd"
                    | "213ss"
                    | "231pd"
                    | "231ps"
                    | "231sd"
                    | "231ss"
            )
        })
}

/// Known explicit-input forms which do not necessarily define a whole
/// destination. Unknown mnemonics cannot certify the absence of hidden inputs.
pub(super) fn known_explicit_vector_inputs(mn: &str, operands: &str) -> bool {
    let base = mn.strip_prefix('v').unwrap_or(mn);
    is_full_xmm_def(mn, operands)
        || source_only_vex_opcode(base)
        || fma3_opcode(base)
        || matches!(
            base,
            "movsd"
                | "movss"
                | "movntdq"
                | "movntdqa"
                | "movntps"
                | "movntpd"
                | "pextrb"
                | "pextrw"
                | "pextrd"
                | "pextrq"
                | "extractps"
                | "cvtsi2sd"
                | "cvtsi2sdq"
                | "cvtsi2sdl"
                | "cvtsi2ss"
                | "cvtsi2ssq"
                | "cvtsi2ssl"
                | "cvtsd2ss"
                | "cvtss2sd"
                | "cvtsd2si"
                | "cvtsd2siq"
                | "cvtsd2sil"
                | "cvttsd2si"
                | "cvttsd2siq"
                | "cvttsd2sil"
                | "cvtss2si"
                | "cvtss2siq"
                | "cvtss2sil"
                | "cvttss2si"
                | "cvttss2siq"
                | "cvttss2sil"
                | "ucomisd"
                | "ucomiss"
                | "comisd"
                | "comiss"
                | "ptest"
                | "testpd"
                | "testps"
                | "pcmpistri"
                | "pcmpistrm"
                | "pcmpestri"
                | "pcmpestrm"
                | "insertps"
                | "pinsrb"
                | "pinsrw"
                | "pinsrd"
                | "pinsrq"
                | "insertf128"
                | "inserti128"
                | "gatherdpd"
                | "gatherdps"
                | "gatherqpd"
                | "gatherqps"
                | "pgatherdd"
                | "pgatherdq"
                | "pgatherqd"
                | "pgatherqq"
                | "movhpd"
                | "movhps"
                | "movlpd"
                | "movlps"
                | "sha1msg1"
                | "sha1msg2"
                | "sha1nexte"
                | "sha1rnds4"
                | "sha256msg1"
                | "sha256msg2"
                | "zeroupper"
                | "zeroall"
        )
}

/// Reads of vector registers by architectural state-image instructions.
/// Even restores with no register inputs have non-textual vector writes.
fn state_image_register_reads(mn: &str) -> Option<u128> {
    match mn {
        "fxsave" | "fxsave64" | "fxsaveq" => Some(XMM_ALL),
        "xsave" | "xsave64" | "xsaveq" | "xsaveopt" | "xsaveopt64" | "xsaveoptq" | "xsavec"
        | "xsavec64" | "xsaves" | "xsaves64" => Some(VECTOR_ALL),
        "fxrstor" | "fxrstor64" | "fxrstorq" | "xrstor" | "xrstor64" | "xrstorq" | "xrstors"
        | "xrstors64" => Some(0),
        _ => None,
    }
}

/// Definite vector-register writes by architectural state-image restores.
/// FXSAVE-format state restores the full low 128 bits of XMM registers but
/// does not restore YMM/ZMM upper components. XRSTOR/XRSTORS writes depend on
/// XCR0, XSTATE_BV, and the runtime EDX:EAX request mask; without proving that
/// mask at the instruction, treating any component as unconditionally killed
/// would be unsound.
fn state_image_register_writes(mn: &str) -> u128 {
    match mn {
        "fxrstor" | "fxrstor64" | "fxrstorq" => XMM_ALL,
        _ => 0,
    }
}

/// Architectural implicit reads, plus a local fail-closed effect for vector
/// instructions not admitted above. Do not turn unknown operations into kills
/// or veto the whole function: later known definitions still retire facts.
fn implicit_fp_reads(mn: &str, operands: &str, mentioned: u128) -> u128 {
    const SLOT_ALL: u128 = ((1u128 << 64) - 1) & !XMM_ALL;
    if matches!(mn, "blendvpd" | "blendvps" | "pblendvb" | "sha256rnds2") {
        return 1;
    }
    if let Some(mut reads) = state_image_register_reads(mn) {
        // State images are not scalar accesses: FXSAVE has a 512-byte layout;
        // XSAVE's footprint also depends on enabled features and its mask.
        // The scalar-slot model cannot prove byte-precise kills. A frame image
        // conservatively reads all slot facts HERE, not throughout the function.
        if operands.contains("(%rsp") || operands.contains("(%rbp") {
            reads |= SLOT_ALL;
        }
        return reads;
    }
    if (mn.starts_with('v') || mentioned != 0) && !known_explicit_vector_inputs(mn, operands) {
        return u128::MAX;
    }
    0
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

/// Conservative READ extent. None means unbounded/unsupported, never zero.
/// This superset is NOT proof that an instruction overwrites all these bytes.
pub(super) fn read_extent(mn: &str, operands: &str) -> Option<i32> {
    // CMOV's condition can end in l (signed less-than). The register,
    // not that character, determines the width of an unsuffixed CMOVL.
    if mn.starts_with("cmov") {
        let dst = dest_operand(operands)?;
        let family = register_family(dst)?;
        for size in [MoveSize::Q, MoveSize::L, MoveSize::W] {
            if dst == reg_id_to_name(family, size) {
                return Some(size.byte_size());
            }
        }
        return None;
    }
    // Narrowing packed conversions read more than their result width.
    // In EVEX a YMM result can consume a ZMM-sized memory source; an XMM
    // result can consume 128 or 256 source bits. Use the conservative read
    // bound, never as an overwrite certificate (these aren't plain stores).
    if matches!(mn, "vcvtpd2ps" | "vcvtpd2dq" | "vcvttpd2dq") {
        return if operands.contains("%ymm") {
            Some(64)
        } else if operands.contains("%xmm") {
            Some(32)
        } else {
            None
        };
    }
    if let Some(sz) = match mn {
        // FXSAVE-format restore images have a fixed 512-byte footprint.
        // XRSTOR-family footprints depend on enabled XSAVE components and
        // remain unknown below, never capped at the largest vector width.
        "fxrstor" | "fxrstor64" | "fxrstorq" => Some(512),
        "movsd" | "vmovsd" | "addsd" | "subsd" | "mulsd" | "divsd" | "minsd" | "maxsd"
        | "sqrtsd" | "ucomisd" | "comisd" | "vaddsd" | "vsubsd" | "vmulsd" | "vdivsd"
        | "vminsd" | "vmaxsd" | "vsqrtsd" | "vucomisd" | "vcomisd" | "cvtsd2ss" | "vcvtsd2ss"
        | "cvttsd2si" | "cvtsd2si" | "vcvttsd2si" | "vcvtsd2si" | "movq" | "movabsq" | "vmovq"
        | "cvtsi2sdq" | "cvtsi2ssq" | "pushq" | "popq" | "cmpq" | "testq" | "addq" | "subq"
        | "andq" | "orq" | "xorq" | "imulq" | "leaq" | "incq" | "decq" | "negq" | "notq"
        | "shlq" | "shrq" | "sarq" => Some(8),
        "ldmxcsr" | "stmxcsr" | "vldmxcsr" | "vstmxcsr" | "movss" | "vmovss" | "addss"
        | "subss" | "mulss" | "divss" | "minss" | "maxss" | "sqrtss" | "ucomiss" | "comiss"
        | "vaddss" | "vsubss" | "vmulss" | "vdivss" | "vminss" | "vmaxss" | "vsqrtss"
        | "vucomiss" | "vcomiss" | "cvtss2sd" | "vcvtss2sd" | "cvttss2si" | "cvtss2si" | "movd"
        | "vmovd" | "movl" | "cvtsi2sd" | "cvtsi2ss" | "cvtsi2sdl" | "cvtsi2ssl" | "movslq"
        | "cmpl" | "testl" | "addl" | "subl" | "andl" | "orl" | "xorl" | "imull" | "incl"
        | "decl" | "negl" | "notl" | "shll" | "shrl" | "sarl" => Some(4),
        "movw" | "cmpw" | "testw" | "addw" | "subw" | "andw" | "orw" | "xorw" | "movzwl"
        | "movswl" | "movzwq" | "movswq" => Some(2),
        "movb" | "cmpb" | "testb" | "addb" | "subb" | "andb" | "orb" | "xorb" | "movzbl"
        | "movsbl" | "movzbq" | "movsbq" | "movzbw" | "movsbw" => Some(1),
        _ => None,
    } {
        return Some(sz);
    }
    if mn.starts_with("set") {
        return Some(1);
    }
    // State images, gathers/scatters and other unmodelled footprints make
    // slots opaque. Vector spelling alone is not an instruction contract.
    if !known_explicit_vector_inputs(mn, operands)
        || mn.contains("gather")
        || mn.contains("scatter")
    {
        return None;
    }
    // Narrowing EVEX conversions produce a YMM result from a full ZMM-sized
    // memory source. The destination spelling alone would undercount the
    // read by 32 bytes and could delete a live high-half frame store.
    if matches!(mn, "vcvtpd2ps" | "vcvtpd2dq" | "vcvttpd2dq") && operands.contains("%ymm") {
        return Some(64);
    }
    if operands.contains("%zmm") {
        Some(64)
    } else if operands.contains("%ymm") {
        Some(32)
    } else if operands.contains("%xmm") {
        Some(16)
    } else {
        None
    }
}

/// Guaranteed contiguous overwrite, deliberately separate from read extent.
/// A mask can preserve every byte; it therefore proves no covering overwrite.
fn overwrite_extent(mn: &str, operands: &str) -> Option<i32> {
    if !is_plain_store(mn) || operands.contains(['{', '}']) {
        return None;
    }
    read_extent(mn, operands)
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
            | "vmovdqa32"
            | "vmovdqa64"
            | "vmovdqu8"
            | "vmovdqu16"
            | "vmovdqu32"
            | "vmovdqu64"
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
///
/// ── THE ORDERING HOLE (PR #584) ──
///
/// The census is only "free to keep" at the ABI level; the GP dead-write
/// pass is ENTITLED to delete `movb $N, %al` before a non-variadic callee
/// (the accumulator is provably dead there, and deleting it is a real
/// instruction-count win).  Once it does, this oracle's only source for
/// the SSE-argument set is the staged-move window below — and that window
/// walk used to BREAK on the first line that was not itself a staging
/// move: the hazard-area release `addq $K, %rsp`, the stack-argument
/// `pushq %rax`, and the GP half of every rax relay (`movq %xmmN, %rax`)
/// all terminated the scan, so a call that staged its arguments across
/// any of those was modeled as reading ZERO xmm registers — and
/// `eliminate_dead_vector_copies` deleted the live argument staging (a
/// 10-double call compiled with 7 of its 8 register arguments reading
/// garbage).  Two fixes, layered:
///
/// 1. `# LCCC_CALL_FP <n>` (Phase 0 below): the codegen's authoritative
///    count, published after the call text exactly like `# LCCC_CALL_ARGS`
///    — a comment, immune to every value-shape pass.  Exact for every
///    call that went through `emit_call_reg_args_impl`.
/// 2. The window walk (Phase 2) now SKIPS register-file-transparent lines
///    (rsp adjustments, pushes/pops) and the GP half of relay pairs, with
///    a step budget that covers the full 8-register staging window.  This
///    keeps the hand-emitted libcall sites sound too.
fn call_xmm_reads(store: &LineStore, infos: &[LineInfo], start: usize, n: usize) -> u128 {
    // Phase 0: the authority marker. `# LCCC_CALL_FP <n>` is published by
    // `emit_call_instruction_impl` within a few lines after the call text
    // (same discharge contract as `# LCCC_CALL_ARGS`); it survives every
    // peephole pass because it is a comment. n = the exact number of
    // leading SSE argument registers the callee reads (0..=8).
    for k in n + 1..(n + 5).min(infos.len()) {
        if infos[k].is_nop() {
            continue;
        }
        let mk = infos[k].trimmed(store.get(k));
        if let Some(rest) = mk.strip_prefix("# LCCC_CALL_FP ") {
            if let Ok(v) = rest.trim().parse::<u32>() {
                if v <= 8 {
                    return (1u128 << v) - 1;
                }
            }
            break; // illegible marker: fall through to the heuristics
        }
        if mk.starts_with('#') {
            continue; // a sibling marker (LCCC_CALL_ARGS / VA / CHAIN)
        }
        break; // real code: no marker published for this call
    }
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
    let mut census: Option<u128> = None;
    while k > start && steps < 8 {
        k -= 1;
        if infos[k].kind == LineKind::Empty
            || super::helpers::steps_over_line(&infos[k], infos[k].trimmed(store.get(k)))
        {
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
            census = Some((1u128 << cnt) - 1);
        }
        break; // first %rax-family line decides
    }
    if let Some(c) = census {
        return c;
    }
    // Phase 2: no census.  Indirect calls without one (`__builtin_apply`'s
    // `call *%r11`, whose eight `movups` loads sit behind a label) stay
    // conservative.  A direct call without a census is either GP-only or a
    // site whose FP operands are staged by plain XMM moves in the window
    // before it (`movq %rax, %xmm0` / `movd %eax, %xmm0` /
    // `movdqu (%rax), %xmm0`): collect the argument registers those moves
    // define and stop at the first line that is neither such a move nor
    // REGISTER-FILE-TRANSPARENT.
    //
    // Transparent lines (skipped, consuming a step): rsp adjustments
    // (`addq/subq $K, %rsp` — the hazard-area release, the stack-argument
    // cleanup, alignment), `pushq`/`popq` (no xmm definition; `pushq %rax`
    // is the stack-argument staging for an FP value read via rax), and the
    // GP half of a rax relay (`movq/vmovq/movd/vmovd %xmmN, %rax` — reads
    // an xmm, DEFINES a GP register, and is immediately followed below by
    // its xmm-writing partner).  Skipping any of these can only EXTEND the
    // walk and collect MORE registers — conservative-safe: the failure
    // mode of the old break-on-first-foreign-line was the opposite
    // (under-collection), and under-collection is what deleted live
    // argument staging (PR #584).
    //
    // Step budget: 24 = the full SysV staging window (8 SSE argument
    // registers, up to two lines per relay-staged argument) plus the
    // transparent adjustments. Anything real that is not staging still
    // breaks the walk — the contiguous-window soundness guard stands.
    let mut written = 0u128;
    let mut k = n;
    let mut steps = 0;
    while k > start && steps < 24 {
        k -= 1;
        if infos[k].kind == LineKind::Empty
            || super::helpers::steps_over_line(&infos[k], infos[k].trimmed(store.get(k)))
        {
            continue;
        }
        // Register-file-transparent lines are checked BEFORE the barrier
        // test: `pushq`/`popq` ARE barriers (they adjust %rsp, which the
        // slot-tracking passes must not see through) but they define and
        // read no XMM register, so the call-reads walk crosses them.  A
        // stack-argument `pushq %rax` sits between every >=9-FP-argument
        // call's register staging and its `call`.
        let tk = infos[k].trimmed(store.get(k));
        let (mn, operands) = split_mnemonic(tk);
        if mn == "pushq" || mn == "popq" || mn == "pushfq" || mn == "popfq" {
            steps += 1;
            continue;
        }
        // rsp adjustment with an immediate (`addq $16, %rsp` /
        // `subq $8, %rsp`) — the hazard-area release, the stack-argument
        // cleanup, alignment padding.  Also an %rsp-family barrier shape
        // for the slot passes; XMM-transparent here.
        if mn == "addq" || mn == "subq" {
            let ops = operand_list(operands);
            if ops.len() == 2
                && ops[0].starts_with('$')
                && (ops[1] == "%rsp" || ops[1] == "%rsp)" || ops[1] == "%esp")
            {
                steps += 1;
                continue;
            }
        }
        if infos[k].is_barrier() {
            break;
        }
        steps += 1;
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
            Some(d) if staging => written |= 1u128 << d,
            _ => {
                // GP half of a relay pair: `movq %xmmN, %rax` — an xmm
                // SOURCE with a GP destination. It defines no xmm register
                // (nothing to collect) and its partner below was already
                // collected; skipping it extends the walk to arguments
                // staged further up. Only the exact xmm→GP move forms;
                // anything else is real code and stops the scan.
                let ops = operand_list(operands);
                let relay_half = matches!(mn, "movq" | "vmovq" | "movd" | "vmovd")
                    && dest_operand(operands).is_some_and(|d| xmm_index(d).is_none())
                    && ops[..ops.len().saturating_sub(1)]
                        .iter()
                        .any(|s| s.starts_with("%xmm") || s.starts_with("%ymm"));
                if relay_half {
                    continue;
                }
                break;
            }
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
fn ret_xmm_reads(store: &LineStore, infos: &[LineInfo], start: usize, end: usize) -> u128 {
    // Compiler-emitted functions publish the exact set in the prologue
    // (`prologue.rs::ret_xmm_mask`); the marker is authoritative.
    for n in start..end {
        if infos[n].is_nop() {
            continue;
        }
        let t = infos[n].trimmed(store.get(n));
        if let Some(m) = t.strip_prefix("# LCCC_RET_XMM ") {
            if let Ok(mask) = m.trim().parse::<u128>() {
                return mask & RET_READS;
            }
        }
        if infos[n].kind == LineKind::Ret {
            break; // marker precedes the body; no point scanning further
        }
    }
    let mut union = 0u128;
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
            if super::helpers::steps_over_line(&infos[k], infos[k].trimmed(store.get(k))) {
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
    reads: u128,
    writes: u128,
}

/// Upper-region observations from explicit operands. EVEX masks do not
/// hide the register identity; conservatively retain every masked lane.
fn upper_mentions(text: &str) -> u128 {
    let mut bits = 0;
    for (prefix, regions) in [("%ymm", 1u8), ("%zmm", 3u8)] {
        for tail in text.split(prefix).skip(1) {
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            if let Ok(n) = tail[..digits].parse::<u32>()
                && n < 16
            {
                bits |= 1u128 << (YMM_HIGH_SHIFT + n);
                if regions == 3 {
                    bits |= 1u128 << (ZMM_HIGH_SHIFT + n);
                }
            }
        }
    }
    bits
}

/// The upper-state projection of the instruction effect. A legacy XMM
/// overwrite NEVER kills these regions. Known VEX/EVEX full definitions
/// overwrite their active width and zero all higher regions; unknown or
/// destructive operations retain explicit reads and prove no upper kills.
fn upper_effect(t: &str) -> Effect {
    let (mn, operands) = split_mnemonic(t);
    if mn == "vzeroupper" || mn == "vzeroall" {
        return Effect {
            reads: 0,
            writes: UPPER_ALL,
        };
    }
    let mut reads = upper_mentions(operands);
    let mut writes = 0;
    let Some(dst) = dest_operand(operands) else {
        return Effect { reads, writes };
    };
    // GAS permits whitespace before/inside EVEX decorations. Strip the
    // decoration and trailing whitespace, but retain its partial-write flag.
    let bare_dst = dst.split('{').next().unwrap_or(dst).trim_end();
    let Some(n) = xmm_index(bare_dst) else {
        return Effect { reads, writes };
    };
    let low = 1u128 << n;
    let srcs = last_top_level_comma(operands.as_bytes())
        .map(|c| &operands[..c])
        .unwrap_or("");
    let zero_idiom = matches!(
        mn,
        "vxorpd" | "vxorps" | "vpxor" | "vpxord" | "vpxorq" | "vpcmpeqd" | "vpcmpeqb"
    ) && operand_list(operands)
        .iter()
        .all(|op| xmm_index(op) == Some(n));
    if mn.starts_with('v') && (is_full_xmm_def(mn, operands) || zero_idiom) {
        let all_upper = upper_regions(low);
        if dst.contains('{') {
            // Masked active lanes are not a full def. Bits above VL still
            // become zero, even for a merge-masked destination.
            writes = if bare_dst.starts_with("%xmm") {
                all_upper
            } else if bare_dst.starts_with("%ymm") {
                low << ZMM_HIGH_SHIFT
            } else {
                0
            };
        } else {
            writes = all_upper;
            reads = (reads & !all_upper) | upper_mentions(srcs);
            if zero_idiom {
                reads &= !all_upper;
            }
        }
    }
    Effect { reads, writes }
}

/// Upper-register projection shared with final VEX promotion. A source's
/// implicit XSAVE observation is just as real as an explicit YMM/ZMM read;
/// a flag-only VEX instruction never kills its last textual operand.
pub(super) fn upper_vector_access_line(t: &str) -> (u32, u32) {
    let (mn, operands) = split_mnemonic(t);
    let effect = upper_effect(t);
    let reads = effect.reads | implicit_fp_reads(mn, operands, xmm_mentions(t));
    let read_mask = (reads >> YMM_HIGH_SHIFT) | (reads >> ZMM_HIGH_SHIFT);
    // Promotion can change BOTH regions, so only a full kill absorbs a path.
    let write_mask = (effect.writes >> YMM_HIGH_SHIFT) & (effect.writes >> ZMM_HIGH_SHIFT);
    ((read_mask & XMM_ALL) as u32, (write_mask & XMM_ALL) as u32)
}

/// Analyse one function.
fn analyse(store: &LineStore, infos: &[LineInfo], start: usize, end: usize) -> FnFacts {
    let n_lines = end - start;
    let mut facts = FnFacts {
        start,
        end,
        slots: Vec::new(),
        live_out: vec![u128::MAX; n_lines],
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
                if super::helpers::steps_over_line(&infos[k], infos[k].trimmed(store.get(k))) {
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
        if infos[n].kind == LineKind::Empty
            || super::helpers::steps_over_line(&infos[n], infos[n].trimmed(store.get(n)))
        {
            continue;
        }
        lines.push(n);
    }
    let mut effects: Vec<Effect> = Vec::with_capacity(lines.len());
    let mut succs: Vec<Vec<usize>> = Vec::with_capacity(lines.len());
    // Deferred slot reads/writes per line: (slot index, is_write).
    let mut slot_ops: Vec<Vec<(usize, bool)>> = Vec::with_capacity(lines.len());

    let intern =
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
            // Inert metadata was removed above. Remaining directives can
            // encode arbitrary instructions or control flow: fail closed.
            LineKind::Directive => return facts,
            LineKind::InlineAsm => (
                Effect {
                    reads: u128::MAX,
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
                    reads: ret_reads | upper_regions(ret_reads),
                    writes: 0,
                },
                Vec::new(),
            ),
            LineKind::Call => (
                Effect {
                    reads: call_xmm_reads(store, infos, start, n)
                        | xmm_mentions(t)
                        | upper_regions(CALL_READS)
                        | upper_mentions(t),
                    writes: VECTOR_ALL,
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
                let mut writes = 0u128;
                let dest = dest_operand(operands);
                // An EVEX writemask (`%zmm3{%k1}`, with or without `{z}`) makes
                // the destination write PARTIAL: merge-masking leaves the
                // inactive lanes at their old value. Zero-masking instead zeros
                // inactive lanes and is a full definition, but this low-region
                // model conservatively retains that destination too.  `xmm_index` already declines
                // the masked spelling; this guard states the reason and keeps
                // the answer correct if that parser ever learns to strip masks.
                let masked_dest = dest.is_some_and(|d| d.contains('{'));
                if let Some(d) = dest.filter(|_| !masked_dest) {
                    if let Some(dn) = xmm_index(d) {
                        let bit = 1u128 << dn;
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
                                let is_dest = k + 1 == n_ops && n_ops >= 2;
                                let overwrite =
                                    is_dest.then(|| overwrite_extent(mn, operands)).flatten();
                                let Some(width) = overwrite.or_else(|| read_extent(mn, operands))
                                else {
                                    opaque = true;
                                    break;
                                };
                                let is_write = overwrite.is_some();
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
                if mn == "vzeroall" {
                    writes |= XMM_ALL;
                    reads &= !XMM_ALL;
                }
                let upper = upper_effect(t);
                (
                    Effect {
                        reads: reads | upper.reads | implicit_fp_reads(mn, operands, mentioned),
                        writes: writes | upper.writes | state_image_register_writes(mn),
                    },
                    fall,
                )
            }
        };
        effects.push(eff);
        succs.push(edges);
        slot_ops.push(ops);
    }
    // Without a proven frame delta, RBP and RSP coordinates are not disjoint
    // address spaces. Keep register precision but refuse slot deletion when
    // both bases are used: e.g. -8(%rbp) aliases 8(%rsp) after subq $16,%rsp.
    opaque |= facts.slots.iter().any(|s| s.0 == 4) && facts.slots.iter().any(|s| s.0 == 5);
    facts.slots_opaque = opaque;

    // Fold slot ops into effects now that every slot is interned (needed so
    // that a READ of slot A also keeps any OVERLAPPING slot B live, and a
    // covering write of B kills A).  A `call` reads every %rsp-based slot:
    // the outgoing-argument area lives there and the callee reads it without
    // any textual trace in this function.
    if !opaque {
        let slots = facts.slots.clone();
        let rsp_slot_mask: u128 = slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.0 == 4)
            .fold(0u128, |m, (k, _)| m | (1u128 << (16 + k)));
        for (pos, ops) in slot_ops.iter().enumerate() {
            let mut r = 0u128;
            let mut w = 0u128;
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
                        if o <= o2 && i64::from(o) + i64::from(s) >= i64::from(o2) + i64::from(s2) {
                            w |= 1u128 << (16 + k); // covering store kills
                        } else {
                            r |= 1u128 << (16 + k); // partial write: old bytes live
                        }
                    } else {
                        r |= 1u128 << (16 + k);
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
    let mut live_in = vec![0u128; lines.len()];
    let mut live_out = vec![0u128; lines.len()];
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 64 {
        changed = false;
        rounds += 1;
        for pos in (0..lines.len()).rev() {
            let mut out = 0u128;
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
                        None => out |= ret_reads | upper_regions(ret_reads),
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
    let mut carry = 0u128;
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
    fn decorated_destination_spacing_keeps_whole_write_obligations() {
        for suffix in ["{%k1}", " {%k1}", "\t{%k1}", " { %k1 }"] {
            let producer = format!("vaddpd %xmm4, %xmm4, %xmm2{suffix}");
            let (store, infos, lv) = build(&format!(
                "    {producer}\n    movaps %xmm5, %xmm2\n    vextractf128 $1, %ymm2, %xmm0\n"
            ));
            let at = line_of(&store, &producer);
            assert!(
                !lv.deleted_write_regions_dead_after(&store, &infos, at, 2, &[at]),
                "EVEX XMM upper-zero write remains observable: {suffix:?}"
            );
        }
    }

    #[test]
    fn decorated_destination_spacing_keeps_merge_masked_upper_inputs() {
        for suffix in ["{%k1}", " {%k1}", "\t{%k1}", " { %k1 }"] {
            let (store, infos, lv) = build(&format!(
                "    vmovaps %xmm6, %xmm2\n    movaps %xmm5, %xmm2\n    vaddpd %ymm4, %ymm4, %ymm2{suffix}\n    movaps %xmm5, %xmm2\n    vextractf128 $1, %ymm2, %xmm0\n"
            ));
            let at = line_of(&store, "vmovaps %xmm6, %xmm2");
            assert!(
                !lv.reg_move_dead_after(&store, &infos, at, 2),
                "mask may be zero: old upper lanes are still inputs: {suffix:?}"
            );
        }
    }

    #[test]
    fn frame_read_across_i32_endpoint_keeps_store_live() {
        let (store, _, lv) = build(
            "    movsd %xmm7, 2147483644(%rbp)\n    vmovupd 2147483640(%rbp), %ymm2\n    movl $1, %eax\n",
        );
        let def = line_of(&store, "movsd %xmm7, 2147483644");
        assert_eq!(lv.slot_live_after(def, 5, 2147483644, 8), Some(true));
        // A disjoint negative displacement must not become a false alias.
        assert_eq!(lv.slot_live_after(def, 5, i32::MIN, 8), Some(false));
    }

    #[test]
    fn architectural_implicit_vector_inputs_remain_live() {
        // Legacy variable blends already model XMM0: keep positive controls
        // alongside SHA256RNDS2, whose message input is implicit too.
        for op in [
            "blendvpd %xmm5, %xmm7",
            "blendvps %xmm5, %xmm7",
            "pblendvb %xmm5, %xmm7",
            "sha256rnds2 %xmm5, %xmm7",
        ] {
            let (store, _, lv) = build(&format!(
                "    movaps %xmm6, %xmm0\n    {op}\n    movaps %xmm5, %xmm0\n    movl $1, %eax\n"
            ));
            let def = line_of(&store, "movaps %xmm6, %xmm0");
            assert_eq!(
                lv.xmm_live_after(def, 0),
                Some(true),
                "implicit XMM0 input: {op}"
            );
        }
    }

    #[test]
    fn pr824_external_redteam_unknown_cfg_keeps_implicit_inputs() {
        for op in [
            "fxsave64 (%rdi)",
            "xsaveopt64 (%rdi)",
            "blendvpd %xmm5, %xmm7",
            "sha256rnds2 %xmm5, %xmm7",
        ] {
            let n = if op.starts_with("fxsave") || op.starts_with("xsave") {
                3
            } else {
                0
            };
            let (store, infos, lv) = build(&format!(
                "    movapd %xmm2, %xmm{n}\n    {op}\n    jmp *%rax\n"
            ));
            let def = line_of(&store, &format!("movapd %xmm2, %xmm{n}"));
            assert!(!lv.xmm_dead_after(&store, &infos, def, n, &[def]), "{op}");
        }
    }

    #[test]
    fn pr824_external_redteam_zmm_read_and_masked_store_footprints() {
        for (op, offset) in [
            ("vmovdqu32 0(%rsp), %zmm7", 48),
            ("vmovupd %ymm1, 0(%rsp){%k1}", 16),
        ] {
            let (store, _, lv) = build(&format!(
                "    movsd %xmm6, {offset}(%rsp)\n    {op}\n    movsd {offset}(%rsp), %xmm0\n"
            ));
            let def = line_of(&store, &format!("movsd %xmm6, {offset}(%rsp)"));
            assert_eq!(lv.slot_live_after(def, 4, offset, 8), Some(true), "{op}");
        }
        let (store, _, lv) = build(
            "    movsd %xmm6, 48(%rsp)\n    vmovdqu32 0(%rsp), %zmm7\n    vmovupd %zmm7, (%rdi)\n",
        );
        let def = line_of(&store, "movsd %xmm6, 48(%rsp)");
        assert_eq!(lv.slot_live_after(def, 4, 48, 8), Some(true));
    }

    #[test]
    fn pattern_e_keeps_store_overlapped_by_zmm_read() {
        let (mut store, mut infos, _) = build(
            "    movq %rax, -64(%rbp)\n    movq %rax, %xmm0\n    vmovdqu32 -112(%rbp), %zmm7\n    vmovupd %zmm7, (%rdi)\n",
        );
        let i = line_of(&store, "movq %rax, -64(%rbp)");
        super::super::local_patterns::eliminate_fp_xmm_roundtrips(&mut store, &mut infos);
        assert!(!infos[i].is_nop(), "byte 48 of the ZMM footprint is read");
        let (mut store, mut infos, _) = build(
            "    movq %rax, -64(%rbp)\n    movq %rax, %xmm0\n    vmovupd -112(%rbp), %ymm7\n    vmovupd %ymm7, (%rdi)\n",
        );
        let i = line_of(&store, "movq %rax, -64(%rbp)");
        super::super::local_patterns::eliminate_fp_xmm_roundtrips(&mut store, &mut infos);
        assert!(
            infos[i].is_nop(),
            "byte 48 is outside the YMM read: useful deletion remains"
        );
    }

    #[test]
    fn read_extent_uses_source_width_not_result_width_or_condition_suffix() {
        for (mn, operands, extent) in [
            ("cmovl", "0(%rsp), %rax", 8),
            ("cmovl", "0(%rsp), %r9d", 4),
            ("cmovl", "0(%rsp), %r9w", 2),
            ("vcvttpd2dq", "0(%rsp), %ymm7", 64),
            ("vcvtpd2dq", "0(%rsp), %ymm7", 64),
            ("vcvtpd2ps", "0(%rsp), %ymm7", 64),
            ("vcvttpd2dq", "0(%rsp), %xmm7", 32),
        ] {
            assert_eq!(read_extent(mn, operands), Some(extent));
            let offset = extent - 1;
            let (store, _, lv) = build(&format!(
                "    movb $1, {offset}(%rsp)\n    {mn} {operands}\n    vmovupd %ymm7, (%rdi)\n"
            ));
            let def = line_of(&store, "movb $1");
            assert_eq!(
                lv.slot_live_after(def, 4, offset, 1),
                Some(true),
                "{mn} {operands}"
            );
        }
    }

    #[test]
    fn read_boundaries_and_guaranteed_overwrites_are_distinct() {
        for (reg, width) in [("xmm", 16), ("ymm", 32), ("zmm", 64)] {
            for offset in [15, 16, 31, 32, 47, 48, 63, 64] {
                let (store, _, lv) = build(&format!(
                    "    movb $1, {offset}(%rsp)\n    vmovupd 0(%rsp), %{reg}7\n    vmovupd %{reg}7, (%rdi)\n"
                ));
                let i = line_of(&store, "movb $1");
                assert_eq!(
                    lv.slot_live_after(i, 4, offset, 1),
                    Some(offset < width),
                    "read {reg} at {offset}"
                );
                for mask in ["", "{%k1}"] {
                    let (store, _, lv) = build(&format!(
                        "    movb $1, {offset}(%rsp)\n    vmovupd %{reg}7, 0(%rsp){mask}\n    movzbl {offset}(%rsp), %eax\n"
                    ));
                    let i = line_of(&store, "movb $1");
                    assert_eq!(
                        lv.slot_live_after(i, 4, offset, 1),
                        Some(!mask.is_empty() || offset >= width),
                        "write {reg}{mask} at {offset}"
                    );
                }
            }
        }
        for op in [
            "fxrstor64 0(%rsp)",
            "xrstor64 0(%rsp)",
            "vfutureload 0(%rsp), %xmm7",
        ] {
            let (store, _, lv) = build(&format!("    movsd %xmm6, 512(%rsp)\n    {op}\n"));
            let i = line_of(&store, "movsd %xmm6");
            assert_eq!(
                lv.slot_live_after(i, 4, 512, 8),
                Some(true),
                "unknown extent: {op}"
            );
        }
    }

    #[test]
    fn architectural_state_saves_observe_vector_inputs() {
        for mn in [
            "fxsave",
            "fxsave64",
            "fxsaveq",
            "xsave",
            "xsave64",
            "xsaveq",
            "xsaveopt",
            "xsaveopt64",
            "xsaveoptq",
            "xsavec",
            "xsavec64",
            "xsaves",
            "xsaves64",
        ] {
            let (store, _, lv) = build(&format!(
                "    movaps %xmm6, %xmm7\n    {mn} (%rdi)\n    movaps %xmm5, %xmm7\n    movl $1, %eax\n"
            ));
            let def = line_of(&store, "movaps %xmm6, %xmm7");
            assert_eq!(
                lv.xmm_live_after(def, 7),
                Some(true),
                "implicit state save: {mn}"
            );
            let (store, infos, lv) = build(&format!(
                "    vmovaps %xmm6, %xmm7\n    movaps %xmm5, %xmm7\n    {mn} (%rdi)\n    vzeroupper\n    movl $1, %eax\n"
            ));
            let def = line_of(&store, "vmovaps %xmm6, %xmm7");
            assert_eq!(
                lv.reg_move_dead_after(&store, &infos, def, 7),
                mn.starts_with("fxsave"),
                "XSAVE observes the upper zeros too; FXSAVE only the overwritten low region: {mn}"
            );
        }
    }

    #[test]
    fn fxrstor_kills_all_low_xmm_state_but_preserves_upper_vector_state() {
        for mn in ["fxrstor", "fxrstor64", "fxrstorq"] {
            // Cover the low endpoint, the legacy XMM7 boundary, and XMM15
            // (which only exists in 64-bit mode). The later read must observe
            // the restored value, not the value copied into the register here.
            for reg in [0u32, 7, 15] {
                let def_text = format!("movaps %xmm6, %xmm{reg}");
                let (store, mut infos, lv) = build(&format!(
                    "    {def_text}\n    {mn} (%rdi)\n    movaps %xmm{reg}, %xmm0\n    movl $1, %eax\n"
                ));
                let def = line_of(&store, &def_text);
                assert_eq!(
                    lv.xmm_live_after(def, reg),
                    Some(false),
                    "{mn} restores and overwrites low XMM{reg}"
                );
                assert!(
                    lv.reg_move_dead_after(&store, &infos, def, reg),
                    "the legacy low-XMM move is dead before {mn} for XMM{reg}"
                );
                assert!(
                    super::super::dead_code::eliminate_dead_fp_moves(&store, &mut infos),
                    "the dead legacy move is removed by the production pass for {mn}/XMM{reg}"
                );
                assert!(
                    infos[def].is_nop(),
                    "the move is eliminated for {mn}/XMM{reg}"
                );
            }

            // Use an XMM-width VEX producer: eliminate_dead_fp_moves parses
            // XMM moves, not YMM/ZMM moves. The later wider read makes the
            // producer's upper-zeroing write observable across FXRSTOR.
            for reg in [7u32, 15] {
                for (width, region_shift) in [("ymm", YMM_HIGH_SHIFT), ("zmm", ZMM_HIGH_SHIFT)] {
                    let def_text = format!("vmovaps %xmm6, %xmm{reg}");
                    let use_text = format!("vmovaps %{width}{reg}, %{width}0");
                    let (store, mut infos, lv) = build(&format!(
                        "    {def_text}\n    {mn} (%rdi)\n    {use_text}\n    movl $1, %eax\n"
                    ));
                    let def = line_of(&store, &def_text);
                    assert_eq!(
                        lv.xmm_live_after(def, reg),
                        Some(false),
                        "{mn} kills low XMM{reg} before the {width} read"
                    );
                    let facts = lv.func_of(def).expect("function facts");
                    assert_ne!(
                        facts.live_out[def - facts.start] & (1u128 << (region_shift + reg)),
                        0,
                        "{width}-high state for register {reg} survives {mn} and is read afterwards"
                    );
                    assert!(
                        !lv.reg_move_dead_after(&store, &infos, def, reg),
                        "deleting the XMM-width VEX move would erase live {width}-high state"
                    );
                    super::super::dead_code::eliminate_dead_fp_moves(&store, &mut infos);
                    assert!(
                        !infos[def].is_nop(),
                        "the production pass preserves the XMM VEX move carrying live {width}-high state"
                    );
                }
            }
        }
    }

    #[test]
    fn xrstor_family_does_not_unconditionally_kill_masked_vector_state() {
        for mn in ["xrstor", "xrstor64", "xrstorq", "xrstors", "xrstors64"] {
            for reg in [7u32, 15] {
                let def_text = format!("movaps %xmm6, %xmm{reg}");
                let (store, _, lv) = build(&format!(
                    "    {def_text}\n    {mn} (%rdi)\n    movaps %xmm{reg}, %xmm0\n    movl $1, %eax\n"
                ));
                let def = line_of(&store, &def_text);
                assert_eq!(
                    lv.xmm_live_after(def, reg),
                    Some(true),
                    "without a proven request mask, {mn} cannot kill old XMM{reg}"
                );
            }
        }
    }

    #[test]
    fn unadmitted_vector_effects_fail_closed_locally() {
        // Synthetic mnemonics test the admission contract, not an executable ISA.
        for op in ["vfutureop %xmm5, %xmm7", "futureop %xmm5, %xmm7"] {
            let (store, _, lv) = build(&format!(
                "    movaps %xmm6, %xmm0\n    {op}\n    movaps %xmm5, %xmm0\n    movl $1, %eax\n"
            ));
            let def = line_of(&store, "movaps %xmm6, %xmm0");
            assert_eq!(
                lv.xmm_live_after(def, 0),
                Some(true),
                "unknown implicit inputs: {op}"
            );
            assert!(!is_full_xmm_def_line(op));
        }
        // No global function veto: a later ordinary overwrite still kills a
        // value defined AFTER the opaque instruction.
        let (store, _, lv) = build(
            "    vfutureop %xmm5, %xmm7\n    movaps %xmm6, %xmm0\n    movaps %xmm5, %xmm0\n    movl $1, %eax\n",
        );
        let def = line_of(&store, "movaps %xmm6, %xmm0");
        assert_eq!(lv.xmm_live_after(def, 0), Some(false));
    }

    #[test]
    fn architectural_state_images_have_non_scalar_frame_footprints() {
        for mn in [
            "fxsave",
            "fxsave64",
            "fxsaveq",
            "fxrstor",
            "fxrstor64",
            "fxrstorq",
            "xsave",
            "xsave64",
            "xsaveq",
            "xsaveopt",
            "xsaveopt64",
            "xsaveoptq",
            "xsavec",
            "xsavec64",
            "xsaves",
            "xsaves64",
            "xrstor",
            "xrstor64",
            "xrstorq",
            "xrstors",
            "xrstors64",
        ] {
            let (store, _, lv) = build(&format!(
                "    movsd %xmm7, -32(%rbp)\n    {mn} -512(%rbp)\n    movl $1, %eax\n"
            ));
            let def = line_of(&store, "movsd %xmm7, -32(%rbp)");
            assert!(
                !lv.slot_dead_after(def, 5, -32, 8),
                "variable-size frame state image: {mn}"
            );
        }
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
    fn zmm_mentions_alias_the_same_xmm() {
        // The aliasing hole this pins: an EVEX instruction reads and writes the
        // low 128 bits of the same physical registers the SSE spelling does.
        assert_eq!(
            xmm_mentions("vaddpd %zmm3, %zmm4, %zmm5"),
            (1 << 3) | (1 << 4) | (1 << 5)
        );
        assert_eq!(xmm_index("%zmm7"), Some(7));
        assert_eq!(xmm_index("%zmm0"), Some(0));
        // Untracked registers are distinct physical ones: reporting None keeps
        // them out of the model without making a tracked register look dead.
        assert_eq!(xmm_index("%zmm16"), None);
        assert_eq!(xmm_index("%zmm20"), None);
        assert_eq!(xmm_mentions("vaddpd %zmm20, %zmm21, %zmm22"), 0);
        // A masked destination still counts as a mention of its register.
        assert_eq!(xmm_mentions("vaddpd %zmm1, %zmm2, %zmm3{%k1}"), 0b1110);
        assert_eq!(xmm_index("%zmm3{%k1}"), None);
    }

    #[test]
    fn a_zmm_read_keeps_the_aliased_xmm_live() {
        // Before the aliasing fix this answered "dead": the scan saw no `%xmm1`
        // spelling, so a fold was free to delete the value `%zmm1` still reads.
        let (store, infos, lv) =
            build("    vmovsd %xmm0, %xmm1\n    vaddpd %zmm1, %zmm2, %zmm3\n    vzeroupper\n");
        let i = line_of(&store, "vmovsd %xmm0, %xmm1");
        assert_eq!(
            lv.xmm_live_after(i, 1),
            Some(true),
            "the %zmm1 read must keep %xmm1 live"
        );
        assert!(!lv.xmm_dead_after(&store, &infos, i, 1, &[i]));
    }

    #[test]
    fn a_masked_evex_write_is_not_a_full_definition() {
        // The discriminating question is what the instruction does to the value
        // ALREADY in the destination.  Merge-masking reads the inactive lanes
        // from it and zero-masking zeroes only the active ones, so under either
        // mask the predecessor's definition stays live into the instruction.
        // Unmasked, the same mnemonic is a full definition and kills it.
        let masked = [
            "vaddpd %zmm1, %zmm1, %zmm2{%k1}",
            "vaddpd %zmm1, %zmm1, %zmm2{%k1}{z}",
        ];
        for op in masked {
            let (store, _infos, lv) = build(&format!(
                "    vmovapd %xmm4, %xmm2\n    {op}\n    vmovsd %xmm2, %xmm0\n"
            ));
            let def = line_of(&store, "vmovapd %xmm4, %xmm2");
            assert_eq!(
                lv.xmm_live_after(def, 2),
                Some(true),
                "a masked destination still reads its incoming value: {op}"
            );
        }
        let (store, _infos, lv) = build(
            "    vmovapd %xmm4, %xmm2\n    vaddpd %zmm1, %zmm1, %zmm2\n    vmovsd %xmm2, %xmm0\n",
        );
        let def = line_of(&store, "vmovapd %xmm4, %xmm2");
        assert_eq!(
            lv.xmm_live_after(def, 2),
            Some(false),
            "unmasked, the EVEX write is a full definition and kills the old value"
        );
        // And the unmasked full-width %zmm spelling still defines %xmm2, so the
        // aliasing fix must not make the oracle conservative to the point of
        // losing that answer: nothing after this line reads it.
        let (store, infos, lv) = build("    vmovapd %zmm4, %zmm2\n    vzeroupper\n");
        let i = line_of(&store, "vmovapd");
        assert_eq!(lv.xmm_live_after(i, 2), Some(false));
        assert!(lv.xmm_dead_after(&store, &infos, i, 2, &[i]));
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

    /// ── The PR #584 bug class ──
    ///
    /// `eliminate_dead_pure_writes` legitimately deletes the `movb $N, %al`
    /// census before a non-variadic callee (%al is dead there).  Once the
    /// census is gone, the fallback window walk is the only source for the
    /// SSE-argument set.  It used to BREAK on the first non-staging line —
    /// the hazard-area release `addq $K, %rsp`, the stack-argument
    /// `pushq %rax`, and the GP half of every rax relay all terminated the
    /// scan — so the call was modeled as reading ZERO xmm registers and
    /// `eliminate_dead_vector_copies` deleted the live argument staging
    /// (a 10-double call lost 7 of its 8 register arguments to garbage).
    /// The `# LCCC_CALL_FP <n>` authority marker fixes the general case;
    /// the tests below pin every layer.

    /// The authority marker is exact and immune to the census deletion:
    /// no census anywhere, marker says 8 → all eight argument registers
    /// read across the hazard-area release.
    #[test]
    fn call_fp_marker_is_authoritative_without_census() {
        let (store, _i, lv) = build(
            "    movsd %xmm8, %xmm1\n\
             \x20   movsd %xmm9, %xmm2\n\
             \x20   movsd %xmm10, %xmm3\n\
             \x20   movsd %xmm11, %xmm4\n\
             \x20   movsd %xmm12, %xmm5\n\
             \x20   movsd %xmm13, %xmm6\n\
             \x20   movsd %xmm14, %xmm7\n\
             \x20   movq %xmm7, %rax\n\
             \x20   movq %rax, %xmm0\n\
             \x20   addq $16, %rsp\n\
             \x20   call f10@PLT\n\
             \x20   # LCCC_CALL_ARGS 0\n\
             \x20   # LCCC_CALL_FP 8\n",
        );
        for (probe, reg) in [
            ("movsd %xmm8, %xmm1", 1),
            ("movsd %xmm10, %xmm3", 3),
            ("movsd %xmm14, %xmm7", 7),
        ] {
            let n = line_of(&store, probe);
            assert_eq!(
                lv.xmm_live_after(n, reg),
                Some(true),
                "{probe}: staged argument read (marker says 8)"
            );
        }
    }

    /// The marker sits among sibling markers (`VA`, `CALL_ARGS`, `CHAIN`)
    /// and is skipped by their parsers; it must parse through them too.
    /// The order matches `emit_call_instruction_impl`: CHAIN keeps its
    /// tight-window position (the GP oracle scans n+1..n+3 for it) and
    /// CALL_FP is emitted LAST — reordering them regresses nested
    /// functions (the static-chain staging gets deleted; found via
    /// nested_functions in the corpus).
    #[test]
    fn call_fp_marker_parses_through_sibling_markers() {
        let (store, _i, lv) = build(
            "    movsd %xmm3, %xmm1\n\
             \x20   call g@PLT\n\
             \x20   # LCCC_VA_CALL\n\
             \x20   # LCCC_CALL_ARGS 2\n\
             \x20   # LCCC_CHAIN_CALL\n\
             \x20   # LCCC_CALL_FP 2\n",
        );
        let n = line_of(&store, "movsd %xmm3, %xmm1");
        assert_eq!(lv.xmm_live_after(n, 1), Some(true));
        assert_eq!(lv.xmm_live_after(n, 2), Some(false));
        assert_eq!(lv.xmm_live_after(n, 3), Some(false));
        assert_eq!(lv.xmm_live_after(n, 7), Some(false));
    }

    /// A zero-count marker models a GP-only call exactly (Phase 0 wins
    /// over the census-less window heuristic even when xmm moves happen
    /// to sit in the window for other reasons).
    #[test]
    fn call_fp_zero_marker_is_exact() {
        let (store, _i, lv) = build(
            "    movsd %xmm3, %xmm1\n\
             \x20   call g@PLT\n\
             \x20   # LCCC_CALL_ARGS 1\n\
             \x20   # LCCC_CALL_FP 0\n",
        );
        let n = line_of(&store, "movsd %xmm3, %xmm1");
        assert_eq!(lv.xmm_live_after(n, 1), Some(false));
    }

    /// The hardened window walk (no marker, no census — the hand-emitted
    /// libcall shape): rsp adjustments, pushes and relay GP-halves are
    /// register-file-transparent; the staging collected through them all.
    #[test]
    fn window_walk_skips_transparent_lines_and_relay_halves() {
        let (store, _i, lv) = build(
            "    movsd %xmm8, %xmm1\n\
             \x20   movq %xmm9, %rax\n\
             \x20   movq %rax, %xmm2\n\
             \x20   movq %xmm2, %rax\n\
             \x20   pushq %rax\n\
             \x20   subq $8, %rsp\n\
             \x20   movq %xmm15, %rax\n\
             \x20   movq %rax, %xmm0\n\
             \x20   addq $16, %rsp\n\
             \x20   call raw_helper@PLT\n",
        );
        let n = line_of(&store, "movsd %xmm8, %xmm1");
        assert_eq!(
            lv.xmm_live_after(n, 1),
            Some(true),
            "staging survives the push/addq/window"
        );
        assert_eq!(
            lv.xmm_live_after(n, 9),
            Some(true),
            "the relay half below reads %xmm9"
        );
        let m = line_of(&store, "movq %rax, %xmm2");
        assert_eq!(
            lv.xmm_live_after(m, 2),
            Some(true),
            "relay-staged argument read through the relay GP half"
        );
        let k = line_of(&store, "movq %xmm9, %rax");
        assert_eq!(
            lv.xmm_live_after(k, 9),
            Some(false),
            "the relay half consumes its source"
        );
    }

    /// The window walk still STOPS at real code: a computation between
    /// the staging and the call ends the collection, so the staging is
    /// not modeled as a call argument.
    #[test]
    fn window_walk_still_stops_at_real_code() {
        let (store, _i, lv) = build(
            "    movsd %xmm5, %xmm1\n\
             \x20   vaddsd %xmm4, %xmm3, %xmm3\n\
             \x20   call raw_helper@PLT\n",
        );
        let n = line_of(&store, "movsd %xmm5, %xmm1");
        // The staging is separated from the call by a real instruction:
        // the collection stopped at the vaddsd, so the call's read set is
        // empty and the staged xmm1 is dead past its own definition.
        assert_eq!(
            lv.xmm_live_after(n, 1),
            Some(false),
            "staging behind real code is not a call argument"
        );
        assert_eq!(lv.xmm_live_after(n, 5), Some(false));
    }

    /// Full-depth relay chain: eight arguments staged as rax relays with
    /// a hazard-area release before the call — the exact f10 shape from
    /// the PR #584 repro, without any marker or census.  Every staged
    /// destination must be read by the call, and every relay source must
    /// stay live until its own relay half.
    #[test]
    fn window_walk_covers_the_full_eight_argument_relay_chain() {
        let mut body = String::new();
        for d in 0..8u32 {
            body.push_str(&format!(
                "    movq %xmm{}, %rax\n    movq %rax, %xmm{}\n",
                d + 7,
                d
            ));
        }
        body.push_str("    addq $16, %rsp\n    call f10@PLT\n");
        let (store, _i, lv) = build(&body);
        // (a) every staged destination is a call argument: live-out of its
        // own staging line (the anti-deletion property the bug broke).
        for d in 0..8u32 {
            let probe = format!("movq %rax, %xmm{d}");
            let n = line_of(&store, &probe);
            assert_eq!(
                lv.xmm_live_after(n, d),
                Some(true),
                "xmm{d} staged argument is read by the call"
            );
        }
        // (b) chain integrity: each relay source stays live out of the
        // PREVIOUS pair's staging line (its own relay half is next) and
        // dies at the relay half itself.
        for d in 1..8u32 {
            let src = d + 7;
            let staging_below = format!("movq %rax, %xmm{}", d - 1);
            let n = line_of(&store, &staging_below);
            assert_eq!(
                lv.xmm_live_after(n, src),
                Some(true),
                "%xmm{src} live until its relay half"
            );
            let half = format!("movq %xmm{src}, %rax");
            let h = line_of(&store, &half);
            assert_eq!(
                lv.xmm_live_after(h, src),
                Some(false),
                "the relay half consumes %xmm{src}"
            );
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
    fn mixed_frame_coordinates_cannot_prove_a_slot_dead() {
        let asm = "f:\n.cfi_startproc\npushq %rbp\nmovq %rsp, %rbp\nsubq $16, %rsp\nmovq %rax, -8(%rbp)\nmovq 8(%rsp), %rax\nleave\nret\n.cfi_endproc\n";
        let store = LineStore::new(asm.to_string());
        let infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let lv = FpLiveness::new(&store, &infos);
        let n = line_of(&store, "movq %rax, -8(%rbp)");
        assert_eq!(lv.slot_live_after(n, 5, -8, 8), Some(true));
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
    #[test]
    fn legacy_xmm_def_preserves_live_upper_lanes() {
        for wide in ["ymm", "zmm"] {
            for overwrite in [
                "pshufd $0, %xmm3, %xmm7",
                "pshufhw $0, %xmm3, %xmm7",
                "pshuflw $0, %xmm3, %xmm7",
                "movaps %xmm3, %xmm7",
                "pxor %xmm7, %xmm7",
            ] {
                let body = format!(
                    "    vmovapd %xmm2, %xmm7\n    {overwrite}\n    vmovupd %{wide}7, (%rdi)\n"
                );
                let (store, infos, lv) = build(&body);
                let i = line_of(&store, "vmovapd");
                assert_eq!(
                    lv.xmm_live_after(i, 7),
                    Some(false),
                    "low XMM region is dead: {body}"
                );
                assert!(!lv.reg_move_dead_after(&store, &infos, i, 7), "{body}");
            }
        }
        // VEX writes zero the upper lanes; a full VEX overwrite DOES kill.
        let (store, _, lv) = build(
            "    vmovapd %xmm2, %xmm7\n    vpshufd $0, %xmm3, %xmm7\n    vmovupd %ymm7, (%rdi)\n",
        );
        assert_eq!(
            lv.xmm_live_after(line_of(&store, "vmovapd"), 7),
            Some(false)
        );
        // Legacy-only scalar/128-bit functions retain the profitable kill.
        let (store, _, lv) = build(
            "    movapd %xmm2, %xmm7\n    pshufd $0, %xmm3, %xmm7\n    movupd %xmm7, (%rdi)\n",
        );
        assert_eq!(lv.xmm_live_after(line_of(&store, "movapd"), 7), Some(false));
    }

    #[test]
    fn raw_bytes_cannot_prove_fp_register_dead() {
        // movupd %xmm7,(%rdi), hidden in .byte: no textual %xmm7 use exists.
        for opaque in [".byte 0x66,0x0f,0x11,0x3f", ".long 0", ".p2align 4,0xcc"] {
            let (store, infos, lv) = build(&format!("    movapd %xmm2, %xmm7\n    {opaque}\n"));
            let i = line_of(&store, "movapd");
            assert!(!lv.xmm_dead_after(&store, &infos, i, 7, &[i]), "{opaque}");
        }
    }
}

// Independent PR824 audit tests. Append to fp_liveness.rs in a disposable tree.
#[cfg(test)]
mod pr824_external_redteam {
    use super::super::{dead_code, local_patterns, vector_copy};
    use super::*;

    fn build(asm: &str) -> (LineStore, Vec<LineInfo>) {
        let store = LineStore::new(asm.to_owned());
        let infos = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        (store, infos)
    }

    #[test]
    fn fxsave_direct_cfg_control_retains_feeder() {
        let (store, mut infos) = build(
            ".type f, @function\nf:\n movapd %xmm2, %xmm3\n fxsave64 (%rdi)\n ret\n.size f, .-f\n",
        );
        dead_code::eliminate_dead_fp_moves(&store, &mut infos);
        assert!(!infos[2].is_nop(), "FXSAVE observes XMM3");
    }

    #[test]
    fn fxsave_unknown_cfg_must_retain_feeder() {
        let (store, mut infos) = build(
            ".type f, @function\nf:\n movapd %xmm2, %xmm3\n fxsave64 (%rdi)\n jmp *%rax\n.size f, .-f\n",
        );
        let facts = FpLiveness::new(&store, &infos);
        assert_eq!(
            facts.xmm_live_after(2, 3),
            None,
            "exercise the unknown-CFG path"
        );
        dead_code::eliminate_dead_fp_moves(&store, &mut infos);
        assert!(
            !infos[2].is_nop(),
            "incorrectly deleted XMM3 feeder observed by FXSAVE"
        );
    }

    #[test]
    fn ymm_slot_control_retains_overlapped_store() {
        let (mut store, mut infos) = build(
            ".type f, @function\nf:\n movq %rax, 24(%rsp)\n movq %rax, %xmm0\n vmovdqu 0(%rsp), %ymm3\n vmovdqu %ymm3, (%rdi)\n ret\n.size f, .-f\n",
        );
        let facts = FpLiveness::new(&store, &infos);
        assert_eq!(facts.slot_live_after(3, 4, 24, 8), Some(true));
        local_patterns::eliminate_fp_xmm_roundtrips(&mut store, &mut infos);
        assert!(!infos[2].is_nop(), "YMM load observes the store");
    }

    #[test]
    fn zmm_slot_must_retain_overlapped_store() {
        let (mut store, mut infos) = build(
            ".type f, @function\nf:\n movq %rax, 48(%rsp)\n movq %rax, %xmm0\n vmovdqu32 0(%rsp), %zmm3\n vmovdqu32 %zmm3, (%rdi)\n ret\n.size f, .-f\n",
        );
        let facts = FpLiveness::new(&store, &infos);
        // Bytes 48..55 overlap the 64-byte ZMM load from 0(%rsp): live.
        assert_eq!(facts.slot_live_after(3, 4, 48, 8), Some(true));
        local_patterns::eliminate_fp_xmm_roundtrips(&mut store, &mut infos);
        assert!(
            !infos[2].is_nop(),
            "incorrectly deleted store read by bytes 48..55 of the ZMM load"
        );
    }
    #[test]
    fn new_dead_fp_pass_upper_zeroing_positive_control() {
        let (store, mut infos) = build(
            ".type f, @function\nf:\n vmovapd %xmm6, %xmm7\n pshufd $27, %xmm5, %xmm7\n vmovupd %ymm7, (%rdi)\n ret\n.size f, .-f\n",
        );
        dead_code::eliminate_dead_fp_moves(&store, &mut infos);
        assert!(
            !infos[2].is_nop(),
            "new consumer correctly checks upper regions"
        );
    }

    #[test]
    fn vector_copy_consumer_must_preserve_vex_upper_zeroing() {
        let (store, mut infos) = build(
            ".type f, @function\nf:\n vmovapd %xmm6, %xmm7\n pshufd $27, %xmm5, %xmm7\n vmovupd %ymm7, (%rdi)\n ret\n.size f, .-f\n",
        );
        vector_copy::eliminate_dead_vector_copies(&store, &mut infos);
        assert!(
            !infos[2].is_nop(),
            "old deletion consumer must also check the newly separate upper regions"
        );
    }

    #[test]
    fn vector_copy_consumer_full_vex_overwrite_control() {
        let (store, mut infos) = build(
            ".type f, @function\nf:\n vmovapd %xmm6, %xmm7\n vpshufd $27, %xmm5, %xmm7\n vmovupd %ymm7, (%rdi)\n ret\n.size f, .-f\n",
        );
        vector_copy::eliminate_dead_vector_copies(&store, &mut infos);
        assert!(
            infos[2].is_nop(),
            "real full-width/upper-zero overwrite permits deletion"
        );
    }

    #[test]
    fn whole_pipeline_must_preserve_upper_zeroing() {
        let asm = ".type f, @function\nf:\n vmovapd %xmm6, %xmm7\n pshufd $27, %xmm5, %xmm7\n vmovupd %ymm7, (%rdi)\n ret\n.size f, .-f\n";
        let out = super::super::peephole_optimize(asm.to_owned());
        assert!(
            out.contains("vmovapd") || out.contains("vpshufd"),
            "neither original nor replacement upper-zeroing operation remains"
        );
    }
}

#[cfg(test)]
mod competing_effect_redteam {
    use super::super::{dead_code, local_patterns, vector_copy};
    use super::*;
    fn build(asm: &str) -> (LineStore, Vec<LineInfo>) {
        let store = LineStore::new(asm.to_owned());
        let infos = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        (store, infos)
    }
    #[test]
    fn masked_vector_copy_is_not_an_unconditional_identity() {
        for suffix in ["{%k1}", "{%k1}{z}"] {
            let asm = format!(
                ".type f, @function\nf:\n vmovapd %xmm2, %xmm3{suffix}\n vmovupd %xmm3, (%rdi)\n ret\n.size f, .-f\n"
            );
            let (mut store, mut infos) = build(&asm);
            vector_copy::propagate_vector_copies(&mut store, &mut infos);
            assert!(
                store.get(3).contains("%xmm3"),
                "masked copy propagated as whole identity: {suffix}, {}",
                store.get(3)
            );
        }
    }
    #[test]
    fn masked_memory_store_is_not_a_guaranteed_overwrite() {
        let (mut store, mut infos) = build(
            ".type f, @function\nf:\n movq %rax, 8(%rsp)\n movq %rax, %xmm0\n vmovupd %xmm2, 0(%rsp){%k1}\n movsd 8(%rsp), %xmm1\n movsd %xmm1, (%rdi)\n ret\n.size f, .-f\n",
        );
        local_patterns::eliminate_fp_xmm_roundtrips(&mut store, &mut infos);
        assert!(
            !infos[2].is_nop(),
            "masked store cannot kill preceding value when the lane is disabled"
        );
    }
    #[test]
    fn opaque_tailcall_preserves_fp_argument_staging() {
        let (store, mut infos) = build(
            ".type f, @function\nf:\n movapd %xmm2, %xmm3\n jmp foreign_callee\n.size f, .-f\n",
        );
        dead_code::eliminate_dead_fp_moves(&store, &mut infos);
        assert!(
            !infos[2].is_nop(),
            "tail callee implicitly reads XMM3 argument"
        );
    }
    #[test]
    fn unknown_memory_footprint_cannot_certify_slot_dead() {
        let (mut store, mut infos) = build(
            ".type f, @function\nf:\n movq %rax, 48(%rsp)\n movq %rax, %xmm0\n vmystery 0(%rsp), %xmm3\n ret\n.size f, .-f\n",
        );
        local_patterns::eliminate_fp_xmm_roundtrips(&mut store, &mut infos);
        assert!(
            !infos[2].is_nop(),
            "unknown footprint must not certify dead frame bytes"
        );
    }
    #[test]
    fn complete_unmasked_store_retires_prior_value_control() {
        let (mut store, mut infos) = build(
            ".type f, @function\nf:\n movq %rax, 8(%rsp)\n movq %rax, %xmm0\n vmovupd %xmm2, 0(%rsp)\n movsd 8(%rsp), %xmm1\n movsd %xmm1, (%rdi)\n ret\n.size f, .-f\n",
        );
        local_patterns::eliminate_fp_xmm_roundtrips(&mut store, &mut infos);
        assert!(
            infos[2].is_nop(),
            "real covering overwrite should allow the optimization"
        );
    }
}

#[cfg(test)]
mod frame_coverage_redteam {
    use super::*;

    fn facts(text: String) -> FpLiveness {
        let store = LineStore::new(text);
        let infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        FpLiveness::new(&store, &infos)
    }

    #[test]
    fn every_boundary_of_xmm_ymm_zmm_reads_is_covered() {
        for (reg, width) in [("xmm3", 16), ("ymm3", 32), ("zmm3", 64)] {
            for off in -8..=72 {
                let lv = facts(format!(
                    ".type f, @function\nf:\n movq %rax, {off}(%rsp)\n movq %rax, %xmm0\n vmovdqu32 0(%rsp), %{reg}\n vmovdqu32 %{reg}, (%rdi)\n ret\n.size f, .-f\n"
                ));
                let overlap = off < width && off + 8 > 0;
                assert_eq!(
                    lv.slot_live_after(3, 4, off, 8),
                    Some(overlap),
                    "{reg}, offset={off}"
                );
            }
        }
    }

    #[test]
    fn overwrite_requires_full_byte_coverage_and_no_mask() {
        for (reg, width) in [("xmm2", 16), ("ymm2", 32), ("zmm2", 64)] {
            for mask in ["", "{%k1}"] {
                for off in -8..=72 {
                    let lv = facts(format!(
                        ".type f, @function\nf:\n movq %rax, {off}(%rsp)\n movq %rax, %xmm0\n vmovdqu32 %{reg}, 0(%rsp){mask}\n movq {off}(%rsp), %rax\n ret\n.size f, .-f\n"
                    ));
                    let covered = mask.is_empty() && off >= 0 && off + 8 <= width;
                    assert_eq!(
                        lv.slot_live_after(3, 4, off, 8),
                        Some(!covered),
                        "{reg}, mask={mask}, offset={off}"
                    );
                }
            }
        }
    }

    #[test]
    fn unmodelled_nonvector_memory_extent_is_not_sixteen_bytes() {
        let lv = facts(".type f, @function\nf:\n movq %rax, 24(%rsp)\n movq %rax, %xmm0\n fldenv 0(%rsp)\n ret\n.size f, .-f\n".into());
        assert_eq!(lv.slot_live_after(3, 4, 24, 8), Some(true));
    }
}

#[cfg(test)]
mod narrowing_conversion_redteam {
    use super::*;

    #[test]
    fn evex_narrowing_memory_sources_cover_all_sixty_four_bytes() {
        for mnemonic in ["vcvtpd2ps", "vcvtpd2dq", "vcvttpd2dq"] {
            for mask in ["", "{%k1}", "{%k1}{z}"] {
                for offset in -8..=72 {
                    let store = LineStore::new(format!(
                        ".type f, @function\nf:\n movq %rax, {offset}(%rsp)\n movq %rax, %xmm0\n {mnemonic} 0(%rsp), %ymm2{mask}\n vmovupd %ymm2, (%rdi)\n ret\n.size f, .-f\n"
                    ));
                    let infos: Vec<_> = (0..store.len())
                        .map(|i| classify_line(store.get(i)))
                        .collect();
                    let facts = FpLiveness::new(&store, &infos);
                    let overlap = offset < 64 && offset + 8 > 0;
                    assert_eq!(
                        facts.slot_live_after(3, 4, offset, 8),
                        Some(overlap),
                        "{mnemonic}, {mask}, offset={offset}"
                    );
                }
            }
        }
    }
}
