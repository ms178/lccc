//! Vector load → vector-ALU memory-operand fusion.
//!
//! Every SSE/AVX arithmetic instruction can read one of its sources straight
//! out of memory.  LCCC's instruction selector instead materialises each
//! operand with a stand-alone move and only then computes:
//!
//! ```text
//!     movups (%rdx,%r9,1), %xmm15      mulps  (%rdx,%r9,1), %xmm14
//!     movups (%rsi,%r9,1), %xmm14  ->
//!     mulps  %xmm15, %xmm14
//! ```
//!
//! which is a whole extra µop per operand, an extra rename slot and an extra
//! schedule slot for every vectorised kernel in the tree.  GCC 16 and Clang 23
//! both emit the folded form; on `tests/benchmark/programs/f32_sse1_map.c`
//! (the `d[i] = x[i]*y[i] + z[i]` map) the inner loop is 15 instructions per
//! eight elements for LCCC against 11 for Clang, and every one of the four
//! extra instructions is a `movups` this pass removes.
//!
//! [`super::load_op_fuse`] does exactly this for the *general-purpose*
//! registers; its register model is a `u16` bitmask with no room for the
//! vector families (24..39), so the vector half needs its own pass.
//!
//! ## Semantics
//!
//! In AT&T syntax every operand of a packed x86 arithmetic instruction that is
//! not the trailing destination is an `r/m` slot, so *substituting the memory
//! operand for the register operand* — never reordering operands — reproduces
//! the original value exactly, commutative or not:
//!
//! * legacy two-operand `OP src, dst` computes `dst = dst OP src`;
//! * VEX three-operand `vOP src2, src1, dst` computes `dst = src1 OP src2`
//!   (Intel order), so AT&T operand 0 is the right-hand source and operand 1
//!   the left-hand one — both are `r/m`;
//! * the immediate-first forms (`vpshufd $imm, src, dst`, `vroundps`,
//!   `vpalignr`, `vcmpps`, …) take `r/m` in AT&T operand 1 — with one carved
//!   exception, the VEX *shift-by-immediate* group, documented at
//!   [`VEX_REG_ONLY_IMM_SRCS`]: `vpslld $3, MEM, %ymm3` is not a VEX
//!   instruction, and an assembler that accepts the text emits AVX-512VL.
//!
//! Packed (and scalar) arithmetic never requires an aligned memory operand —
//! the alignment trap belongs to `movaps`/`movdqa` only — so an unaligned
//! `movups`/`movdqu` load folds just as safely as an aligned one.
//!
//! A bare address carries no size, so the folded operand's width has to be
//! recoverable from the instruction that remains, and it has to be the width
//! the retired load actually produced — see [`uniform_width_bytes`] and the
//! register-class rule in [`try_fold`].  `vmovsd (%rax), %xmm1` writes 64 bits
//! and `vaddpd %xmm1, %xmm2, %xmm3` reads 128, so folding them would silently
//! read bytes 8..15 the original never touched.
//!
//! ## Soundness argument
//!
//! A load at `i` with destination `%xmmT` may be retired and its address
//! re-read at a consumer `j > i` when:
//!
//! 1. **The window is straight-line.**  Every line strictly between `i` and
//!    `j` is a plain `Other`/`Cmp`/`SetCC` instruction: no label, branch,
//!    call, `ret`, push/pop or inline assembly, so textual order is execution
//!    order and the consumer is reached exactly once per execution of the
//!    load.
//! 2. **No line in the window mentions `%xmmT`.**  The scan stops at the first
//!    mention, so nothing between can read or write the temporary; the
//!    consumer is that first mention.
//! 3. **No line in the window has an implicit vector effect**
//!    ([`super::fp_liveness::has_implicit_vector_effects_line`]), so no masked
//!    write, gather, `vzeroupper` or string-comparison side channel can
//!    redefine `%xmmT` without naming it.
//! 4. **No line in the window writes memory.**  A store between the load and
//!    the consumer could alias the loaded address; a *load* cannot change any
//!    byte, so read-only accesses are allowed to cross — that is the whole
//!    point, because the selector interleaves the operand loads of one
//!    expression (`movups …%xmm15; movups …%xmm14; mulps %xmm15,%xmm14`).
//!    Anything not positively recognised as a pure load is refused.
//! 5. **No line in the window writes a GP register the address names.**  The
//!    fused instruction re-reads the address at the consumer, so the base and
//!    index registers must still hold the same values.
//! 6. **`%xmmT` is dead after the consumer** — asked of [`FpLiveness`], not a
//!    textual scan.  For a VEX load the architectural upper regions die with
//!    it (a `vmovups` into `%xmmN` zeroes bits 255:128 of `%ymmN`), so they
//!    are checked too; deleting the load must not drop an observable zeroing.
//! 7. **The consumer's destination is not `%xmmT`.**  A legacy two-operand op
//!    reads its destination, so `movups MEM,%xmmT; mulps %xmmT,%xmmT` computes
//!    `MEM*MEM` and the folded `mulps MEM,%xmmT` would read the stale
//!    register instead.
//! 8. **Neither line is pinned.**  Volatile accesses stay exactly where the
//!    frontend put them; this pass never moves one.
//! 9. **The encoding survives the substitution.**  Two shapes are declined
//!    even though the *semantics* would be preserved, because no legal
//!    encoding exists at the ISA level the caller selected:
//!    a VEX shift-by-immediate (its source is register-only, so the text
//!    would silently become AVX-512VL — see [`VEX_REG_ONLY_IMM_SRCS`]), and
//!    any consumer whose destination is narrower than the temporary (the
//!    operand size would be undeterminable).
//!
//! The replacement performs exactly one read of the original width at the
//! original address, so volatile/MMIO semantics are untouched (and pinned
//! lines are skipped outright), and no flags behaviour changes: the deleted
//! `movups` never wrote flags and the consumer writes the same flags from the
//! same value.
//!
//! ## Liveness monotonicity (why the oracle is never refreshed)
//!
//! Both rewrites only ever *remove* register effects: the retired load loses
//! its definition of `%xmmT`, and the fused consumer loses its read of
//! `%xmmT` (it reads memory instead, which the bitmask liveness model does
//! not track).  Removing a definition and a use can only shrink the live sets
//! of the transformed program, so a snapshot taken before the rewrite is an
//! over-approximation of the liveness after it.  `dead` is a negative answer,
//! and `x ∉ over-approximation ⟹ x ∉ exact`, so every "dead" verdict the
//! stale oracle gives stays valid.
//!
//! Switchable with `CCC_PEEPHOLE_SKIP=vector_load_fuse`.

use super::super::types::*;
use super::fp_liveness::{FpLiveness, has_implicit_vector_effects_line};
use super::load_op_fuse::{
    accesses_memory, is_plain_memory_operand, is_window_kind, may_write_families,
    memory_operand_families, next_insn,
};

/// Longest straight-line gap between a vector load and the consumer it is
/// fused into.  The selector emits the operand loads of one expression
/// back-to-back, so a short window catches them all; a longer one only adds
/// scan cost and keeps the address registers live further.
const MAX_GAP: usize = 8;

/// Every GP family — `may_write_families`' "I do not know" answer.
const ALL_GPR: u16 = 0xFFFF;

/// Packed and scalar moves that copy memory into a vector register without
/// touching anything else.  Scalar `movss`/`movsd` are included: the merge
/// into the destination's upper lanes is the definition of the load, and the
/// fold only ever *removes* that definition, which the deadness proof below
/// already accounts for.
const VEC_LOADS: &[&str] = &[
    "lddqu",
    "movapd",
    "movaps",
    "movdqa",
    "movdqu",
    "movsd",
    "movss",
    "movupd",
    "movups",
    "vlddqu",
    "vmovapd",
    "vmovaps",
    "vmovdqa",
    "vmovdqa32",
    "vmovdqa64",
    "vmovdqu",
    "vmovdqu32",
    "vmovdqu64",
    "vmovsd",
    "vmovss",
    "vmovupd",
    "vmovups",
];

/// The subset of [`VEC_LOADS`] whose spelling *asserts* natural alignment of
/// the addressed object: `movaps`/`movapd`/`movdqa` (and their VEX forms)
/// fault on an unaligned address, so the selector only ever emits one where
/// it has proved 16-byte alignment.  That proof is what licenses the fold
/// below — see [`fold_needs_aligned_load`].
const ALIGNED_VEC_LOADS: &[&str] = &[
    "movapd",
    "movaps",
    "movdqa",
    "vmovapd",
    "vmovaps",
    "vmovdqa",
    "vmovdqa32",
    "vmovdqa64",
];

/// Mnemonic bases whose `r/m` operand width is **not** the uniform rule
/// ([`uniform_width_bytes`]).  Sorted for `binary_search`.
///
/// Measured, not assumed — each entry is a GAS `objdump -M intel` operand size
/// that disagrees with the rule:
///
/// ```text
///   vcvtps2pd (%rax), %xmm0        -> QWORD    (8,  not 16: widening)
///   vcvtpd2dq (%rax), %xmm0        -> rejected (source is 256-bit)
///   vpmaxsd (%rax), %xmm1, %xmm0   -> XMMWORD  (16, not 8: PACKED, despite `sd`)
///   vpmovzxbd (%rax), %xmm0        -> DWORD    (4; 8 with a %ymm destination)
///   vinsertps $0, (%rax), %xmm1, %xmm0 -> DWORD
/// ```
///
/// `pmaxsd`/`pminsd` are the trap: the uniform rule reads the `sd` suffix as
/// "scalar double, 8 bytes", but they are packed dword comparisons reading 16.
/// Trusting the suffix there would let a 64-bit `movsd` load fold into a
/// 128-bit read.  The `pmovsx`/`pmovzx` family is genuinely destination-width
/// dependent, so no single answer exists for it either.
///
/// Every entry is declined rather than modelled: these are widening, narrowing
/// or shuffling forms, and a fold that has to *prove* a width the instruction
/// text does not state is not worth the one load it saves.
const NON_UNIFORM_WIDTH_BASES: &[&str] = &[
    "cvtdq2pd",
    "cvtpd2dq",
    "cvtpd2ps",
    "cvtps2pd",
    "cvttpd2dq",
    "insertps",
    "pmovsxbd",
    "pmovsxbq",
    "pmovsxbw",
    "pmovsxdq",
    "pmovsxwd",
    "pmovsxwq",
    "pmovzxbd",
    "pmovzxbq",
    "pmovzxbw",
    "pmovzxdq",
    "pmovzxwd",
    "pmovzxwq",
];

/// Packed compares whose suffix *lies*: `pmaxsd`/`pminsd` read the whole
/// register (measured: `vpmaxsd (%rax), %xmm1, %xmm0` is an `XMMWORD`
/// operand), not the 8 bytes the `sd` ending suggests.  Trusting the suffix
/// would fold a 64-bit `movsd` load into a 128-bit read, so they are named
/// here and answered "full width" before the suffix rule runs.  Sorted.
const FULL_WIDTH_DESPITE_SUFFIX: &[&str] = &["pmaxsd", "pminsd"];

/// Bytes the `r/m` operand of `mnemonic` reads or writes in a register of class
/// `reg_class` (`1` = `%xmm`, `2` = `%ymm`, `3` = `%zmm`), or `None` when the
/// width is not the uniform rule and the fold must be declined.
///
/// The uniform rule, verified mnemonic-by-mnemonic against GAS: a `ss` suffix
/// means 4 bytes, a `sd` suffix means 8, and anything else reads the whole
/// register.  [`NON_UNIFORM_WIDTH_BASES`] names the exceptions.
fn uniform_width_bytes(mnemonic: &str, reg_class: u8) -> Option<u32> {
    let base = mnemonic.strip_prefix('v').unwrap_or(mnemonic);
    if NON_UNIFORM_WIDTH_BASES.binary_search(&base).is_ok() {
        return None;
    }
    // Checked before the suffix rule: these are packed despite ending in `sd`.
    if FULL_WIDTH_DESPITE_SUFFIX.binary_search(&base).is_ok() {
        return Some(128 * u32::from(reg_class));
    }
    if base.ends_with("ss") {
        Some(4)
    } else if base.ends_with("sd") {
        Some(8)
    } else {
        Some(128 * u32::from(reg_class))
    }
}

/// Whether a consumer spelled `mnemonic` may read the folded address when the
/// only alignment evidence is an *unaligned* load spelling.
///
/// Architecturally no packed arithmetic instruction requires an aligned memory
/// operand — the requirement belongs to `movaps`/`movdqa` alone — so a VEX
/// consumer is always safe (verified: `vaddpd (%rax), %ymm15, %ymm15` survives
/// every misalignment, and GCC's own `-mavx` output relies on it).
///
/// A **legacy** SSE consumer is a different story.  Hosts exist on which an
/// unaligned 16-byte memory operand on a legacy SSE arithmetic instruction
/// traps — this tree's own test host is one (see the measurements in
/// `engineering/`; `movapd`/`addpd`/`paddd` at `addr % 16 != 0` raise
/// `SIGSEGV`/`SI_KERNEL` there, while `movupd`, `addsd` and every VEX form do
/// not).  Shipping a fold that traps on the host it is developed on is not
/// acceptable, so the legacy case demands the selector's own alignment proof:
/// the load must be spelled `movaps`/`movapd`/`movdqa`.
///
/// The restriction costs nothing against the state of the art — it is exactly
/// what Clang does.  `a16`/`a1` below differ only in the alignment attribute
/// of the vector pointer, and Clang folds the first but not the second:
///
/// ```text
///   a16: movapd (%rdi), %xmm0      a1:  movupd (%rdi), %xmm1
///        addpd  (%rsi), %xmm0           movupd (%rsi), %xmm0
///                                       addpd  %xmm1, %xmm0
/// ```
///
/// GCC never folds a vector load into an arithmetic memory operand at all.
fn fold_needs_aligned_load(mnemonic: &str) -> bool {
    !mnemonic.starts_with('v')
}

/// Mnemonic bases (a leading `v` is stripped) whose two-operand form
/// `OP src, dst` and whose VEX form `vOP src2, src1, dst` accept `r/m` in the
/// leading source slots.  Sorted for `binary_search`.
const FOLDABLE_BASES: &[&str] = &[
    "addpd",
    "addps",
    "addsd",
    "addss",
    "addsubpd",
    "addsubps",
    "andnpd",
    "andnps",
    "andpd",
    "andps",
    "comisd",
    "comiss",
    "cvtdq2pd",
    "cvtdq2ps",
    "cvtpd2dq",
    "cvtpd2ps",
    "cvtps2dq",
    "cvtps2pd",
    "cvttpd2dq",
    "cvttps2dq",
    "divpd",
    "divps",
    "divsd",
    "divss",
    "fmadd132pd",
    "fmadd132ps",
    "fmadd132sd",
    "fmadd132ss",
    "fmadd213pd",
    "fmadd213ps",
    "fmadd213sd",
    "fmadd213ss",
    "fmadd231pd",
    "fmadd231ps",
    "fmadd231sd",
    "fmadd231ss",
    "fmaddsub132pd",
    "fmaddsub132ps",
    "fmaddsub213pd",
    "fmaddsub213ps",
    "fmaddsub231pd",
    "fmaddsub231ps",
    "fmsub132pd",
    "fmsub132ps",
    "fmsub132sd",
    "fmsub132ss",
    "fmsub213pd",
    "fmsub213ps",
    "fmsub213sd",
    "fmsub213ss",
    "fmsub231pd",
    "fmsub231ps",
    "fmsub231sd",
    "fmsub231ss",
    "fmsubadd132pd",
    "fmsubadd132ps",
    "fmsubadd213pd",
    "fmsubadd213ps",
    "fmsubadd231pd",
    "fmsubadd231ps",
    "fnmadd132pd",
    "fnmadd132ps",
    "fnmadd132sd",
    "fnmadd132ss",
    "fnmadd213pd",
    "fnmadd213ps",
    "fnmadd213sd",
    "fnmadd213ss",
    "fnmadd231pd",
    "fnmadd231ps",
    "fnmadd231sd",
    "fnmadd231ss",
    "fnmsub132pd",
    "fnmsub132ps",
    "fnmsub132sd",
    "fnmsub132ss",
    "fnmsub213pd",
    "fnmsub213ps",
    "fnmsub213sd",
    "fnmsub213ss",
    "fnmsub231pd",
    "fnmsub231ps",
    "fnmsub231sd",
    "fnmsub231ss",
    "haddpd",
    "haddps",
    "hsubpd",
    "hsubps",
    "maxpd",
    "maxps",
    "maxsd",
    "maxss",
    "minpd",
    "minps",
    "minsd",
    "minss",
    "mulpd",
    "mulps",
    "mulsd",
    "mulss",
    "orpd",
    "orps",
    "packssdw",
    "packsswb",
    "packusdw",
    "packuswb",
    "paddb",
    "paddd",
    "paddq",
    "paddsb",
    "paddsw",
    "paddusb",
    "paddusw",
    "paddw",
    "pand",
    "pandn",
    "pavgb",
    "pavgw",
    "pcmpeqb",
    "pcmpeqd",
    "pcmpeqq",
    "pcmpeqw",
    "pcmpgtb",
    "pcmpgtd",
    "pcmpgtq",
    "pcmpgtw",
    "phaddd",
    "phaddsw",
    "phaddw",
    "phsubd",
    "phsubsw",
    "phsubw",
    "pmaddubsw",
    "pmaddwd",
    "pmaxsb",
    "pmaxsd",
    "pmaxsw",
    "pmaxub",
    "pmaxud",
    "pmaxuw",
    "pminsb",
    "pminsd",
    "pminsw",
    "pminub",
    "pminud",
    "pminuw",
    "pmovsxbd",
    "pmovsxbq",
    "pmovsxbw",
    "pmovsxdq",
    "pmovsxwd",
    "pmovsxwq",
    "pmovzxbd",
    "pmovzxbq",
    "pmovzxbw",
    "pmovzxdq",
    "pmovzxwd",
    "pmovzxwq",
    "pmuldq",
    "pmulhrsw",
    "pmulhuw",
    "pmulhw",
    "pmulld",
    "pmullw",
    "pmuludq",
    "por",
    "psadbw",
    "pslld",
    "pslldq",
    "psllq",
    "psllw",
    "psrad",
    "psraw",
    "psrld",
    "psrldq",
    "psrlq",
    "psrlw",
    "psubb",
    "psubd",
    "psubq",
    "psubsb",
    "psubsw",
    "psubusb",
    "psubusw",
    "psubw",
    "punpckhbw",
    "punpckhdq",
    "punpckhqdq",
    "punpckhwd",
    "punpcklbw",
    "punpckldq",
    "punpcklqdq",
    "punpcklwd",
    "pxor",
    "subpd",
    "subps",
    "subsd",
    "subss",
    "ucomisd",
    "ucomiss",
    "xorpd",
    "xorps",
];

/// Mnemonic bases whose immediate-first form `OP $imm, src, dst` takes `r/m`
/// in AT&T operand 1.  Kept separate from [`FOLDABLE_BASES`] on purpose: the
/// extract family (`pextrd $imm, %xmm, r/m32`) and the insert-with-register
/// forms spell the *destination* in that slot, so an "any immediate-first
/// instruction" rule would fold a load into a store.
const IMM_FIRST_BASES: &[&str] = &[
    "blendpd",
    "blendps",
    "cmppd",
    "cmpps",
    "dppd",
    "dpps",
    "insertps",
    "mpsadbw",
    "palignr",
    "pblendd",
    "pblendw",
    "pshufd",
    "pshufhw",
    "pshuflw",
    "pslld",
    "pslldq",
    "psllq",
    "psllw",
    "psrad",
    "psraw",
    "psrld",
    "psrldq",
    "psrlq",
    "psrlw",
    "pternlogd",
    "pternlogq",
    "roundpd",
    "roundps",
    "roundsd",
    "roundss",
];

/// The VEX **shift-by-immediate** group — opcode `0F 71` (`psllw`/`psrlw`/
/// `psraw`), `0F 72` (`pslld`/`psrld`/`psrad`) and `0F 73` (`psllq`/`psrlq`/
/// `pslldq`/`psrldq`).  Sorted for `binary_search`.
///
/// In the VEX encoding this group's source is **register-only**: `VPSLLD
/// xmm1, xmm2, imm8` has no `xmm2/m128` form, so `vpslld $3, MEM, %ymm3` is
/// not a VEX instruction at all.  GAS does not reject it — it silently
/// promotes the text to the EVEX encoding, which requires AVX-512VL:
///
/// ```text
///   vmovdqu (%rdi), %ymm3        c5 fe 6f 1f        vmovdqu (%rdi),%ymm3
///   vpslld  $3, %ymm3, %ymm3     c5 e5 72 f3 03     vpslld $0x3,%ymm3,%ymm3
///                           ->
///   vpslld  $3, (%rdi), %ymm3    62 f1 65 28 72 37 03
/// ```
///
/// `c5`/`c4` is VEX, `62` is EVEX.  Emitting the folded form into a function
/// compiled for `-march=x86-64-v3` — which guarantees AVX2 and *not*
/// AVX-512VL — turns working code into `SIGILL` on the target it was built
/// for.  Measured on this tree's own `tests/regression/bb_slp_v7.c`
/// (`v7_mf_shift`).  The legacy two-operand form has no such problem
/// (`pslld MEM, %xmm0` is a real SSE2 encoding), which is why the gate below
/// is scoped to the `v`-prefixed spelling.
const VEX_REG_ONLY_IMM_SRCS: &[&str] = &[
    "pslld", "pslldq", "psllq", "psllw", "psrad", "psraw", "psrld", "psrldq", "psrlq", "psrlw",
];

/// Split an operand list at its top-level commas, returning the byte spans of
/// the operands so a caller can splice one back in without disturbing the
/// surrounding separators or whitespace.
fn operand_spans(operands: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::with_capacity(4);
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in operands.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                spans.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
    }
    spans.push((start, operands.len()));
    // Trim the surrounding whitespace out of every span so a caller splicing
    // one operand back in keeps the separators that neighbour it.
    for (lo, hi) in &mut spans {
        let bytes = operands.as_bytes();
        while *lo < *hi && bytes[*lo].is_ascii_whitespace() {
            *lo += 1;
        }
        while *hi > *lo && bytes[*hi - 1].is_ascii_whitespace() {
            *hi -= 1;
        }
    }
    spans
}

/// The vector register number named by `op` (`%xmmN`, `%ymmN`, `%zmmN`).
fn vec_reg_index(op: &str) -> Option<u32> {
    let rest = op
        .strip_prefix("%xmm")
        .or_else(|| op.strip_prefix("%ymm"))
        .or_else(|| op.strip_prefix("%zmm"))?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = rest.parse().ok()?;
    (n < 16).then_some(n)
}

/// The 128/256/512-bit register class of `%xmmN`/`%ymmN`/`%zmmN`, as a rank
/// that orders by width.  `None` for anything that is not a vector register.
fn vec_reg_class(op: &str) -> Option<u8> {
    if op.starts_with("%xmm") {
        Some(1)
    } else if op.starts_with("%ymm") {
        Some(2)
    } else if op.starts_with("%zmm") {
        Some(3)
    } else {
        None
    }
}

/// `movups MEM, %xmmN` → `(mem, "%xmmN")`.  Only a plain parenthesised (or
/// validated segment-qualified) address is accepted, never a register or
/// immediate source.
fn parse_vector_load(t: &str) -> Option<(&str, &str)> {
    let (mnemonic, rest) = t.split_once(char::is_whitespace)?;
    if !VEC_LOADS.contains(&mnemonic) {
        return None;
    }
    let rest = rest.trim();
    let comma = crate::backend::peephole_common::last_top_level_comma(rest.as_bytes())?;
    let src = rest[..comma].trim();
    let dst = rest[comma + 1..].trim();
    if !is_plain_memory_operand(src) {
        return None;
    }
    vec_reg_index(dst)?;
    Some((src, dst))
}

/// Whether `t` reads memory without writing it.  A positive list: `lea`
/// computes an address, `prefetch*` only touches the cache hierarchy, and a
/// two-operand move / broadcast whose destination is a plain register and
/// whose source is memory is a load.  Everything else — stores, `lock`
/// read-modify-write, string primitives, `xchg`, unknown mnemonics — is
/// answered "no", which makes the caller refuse.
fn is_pure_load(t: &str) -> bool {
    let t = t.trim();
    // A `lock` prefix turns any read-modify-write into an observable store;
    // no locked form is a pure load.
    if t.starts_with("lock ") {
        return false;
    }
    let Some((mnemonic, rest)) = t.split_once(char::is_whitespace) else {
        return false;
    };
    if mnemonic.starts_with("lea") || mnemonic.starts_with("prefetch") {
        return true;
    }
    let ops = operand_spans(rest.trim());
    if ops.len() != 2 {
        return false;
    }
    let src = rest.trim()[ops[0].0..ops[0].1].trim();
    let dst = rest.trim()[ops[1].0..ops[1].1].trim();
    // Destination must be a bare register (a memory destination is a store),
    // source must be memory (neither `%reg` nor `$imm`).
    if !dst.starts_with('%') || dst.contains('(') {
        return false;
    }
    if src.starts_with('%') || src.starts_with('$') {
        return false;
    }
    let base = mnemonic.strip_prefix('v').unwrap_or(mnemonic);
    base.starts_with("mov")
        || base.starts_with("broadcast")
        || base.starts_with("pbroadcast")
        || base.starts_with("lddqu")
}

/// Whether the straight-line instruction `t` at `info` may sit between a
/// vector load and the consumer that re-reads its address.
///
/// `addr_gp` is the set of GP families the address names; `t_fam` is the
/// loaded register's family (the caller has already established that this
/// line does not mention it).
fn window_line_ok(t: &str, info: &LineInfo, addr_gp: u16) -> bool {
    if !is_window_kind(info.kind) || info.pinned || info.volatile_access {
        return false;
    }
    // EVEX writemasks, gathers, string compares and `vzeroupper` all redefine
    // or observe vector state the operand list does not spell out.
    if has_implicit_vector_effects_line(t) {
        return false;
    }
    if !accesses_memory(t, info) {
        // A pure register instruction: only the address registers matter.
        let wr = may_write_families(t, info);
        return wr != ALL_GPR && wr & addr_gp == 0;
    }
    // Reads do not change the loaded bytes; writes might.
    if !is_pure_load(t) {
        return false;
    }
    // A load writes only its own destination register, which is a vector or a
    // GP family the caller has already excluded from `addr_gp` membership
    // changes — but an unrecognised mnemonic is not given the benefit of the
    // doubt.
    let wr = may_write_families(t, info);
    wr != ALL_GPR && wr & addr_gp == 0
}

/// Whether a consumer may take the loaded value as a memory operand in
/// operand slot `slot`.
///
/// The trailing AT&T operand is the destination and is never foldable: with
/// only two operands the destination is also a source (a legacy SSE op is
/// destructive), so slot 0 is the one `r/m` slot; with the VEX three-operand
/// encoding both leading slots are `r/m` sources; and with an immediate in
/// slot 0 the single source is slot 1 for the mnemonics listed in
/// [`IMM_FIRST_BASES`] — never for all of them, because the extract family
/// (`vpextrd $1, %xmm3, %eax`) spells its *destination* there.
fn slot_foldable(mnemonic: &str, operands: &str, spans: &[(usize, usize)], slot: usize) -> bool {
    if slot + 1 >= spans.len() {
        // The trailing operand is the destination.
        return false;
    }
    let base = mnemonic.strip_prefix('v').unwrap_or(mnemonic);
    let first = operands[spans[0].0..spans[0].1].trim();
    match spans.len() {
        2 => slot == 0 && FOLDABLE_BASES.binary_search(&base).is_ok(),
        n if n >= 3 => {
            if first.starts_with('$') {
                // A VEX shift-by-immediate has no memory source at all; see
                // [`VEX_REG_ONLY_IMM_SRCS`].  The legacy spelling keeps its
                // SSE2 memory form, so only the `v` prefix is refused.
                slot == 1
                    && IMM_FIRST_BASES.binary_search(&base).is_ok()
                    && !(mnemonic.starts_with('v')
                        && VEX_REG_ONLY_IMM_SRCS.binary_search(&base).is_ok())
            } else {
                // VEX/EVEX three-operand: Intel `VOP dst, src1, src2` is
                // spelled `vop src2, src1, dst` in AT&T, and `src1` lives in
                // the VEX.vvvv field — a register-only slot.  Only `src2`,
                // the ModRM.r/m operand, may be memory.  Folding slot 1 would
                // emit `vfmadd231ps %ymm1, MEM, %ymm2`, which no encoder
                // accepts (and which GAS rejects outright).
                slot == 0
                    && mnemonic.starts_with('v')
                    && FOLDABLE_BASES.binary_search(&base).is_ok()
            }
        }
        _ => false,
    }
}

/// Fuse `movups MEM, %xmmT … OP %xmmT, %xmmD` into `OP MEM, %xmmD`.
///
pub(super) fn fuse_vector_load_into_op(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut fp: Option<FpLiveness> = None;
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].pinned
            || !matches!(
                infos[i].kind,
                LineKind::Other { .. } | LineKind::LoadXmmRbp { .. }
            )
        {
            i += 1;
            continue;
        }
        // Reject on borrowed text: `trimmed` hands back a slice of the line,
        // so the overwhelming majority of candidates — every scalar
        // instruction and every directive — are turned away here without a
        // single heap allocation.
        let load_text = infos[i].trimmed(store.get(i));
        let Some((mem, t_reg)) = parse_vector_load(load_text) else {
            i += 1;
            continue;
        };
        let Some(t_num) = vec_reg_index(t_reg) else {
            i += 1;
            continue;
        };
        // `parse_vector_load` has already restricted the mnemonic to
        // [`VEC_LOADS`]; keep the spelling for the alignment gate below.
        let load_mnemonic = load_text
            .split_once(char::is_whitespace)
            .map(|(m, _)| m)
            .unwrap_or("");
        let t_fam = VEC_FAMILY_BASE + t_num as RegId;
        let addr_gp = memory_operand_families(mem);

        // Walk forward to the first line that mentions the temporary.
        let mut j = next_insn(store, infos, i + 1, len);
        let mut consumer = None;
        let mut gap = 0usize;
        while j < len {
            let info = &infos[j];
            if !is_window_kind(info.kind) || info.pinned {
                break;
            }
            let line = info.trimmed(store.get(j));
            if mentions_family(line.as_bytes(), t_fam) {
                consumer = Some(j);
                break;
            }
            if !window_line_ok(line, info, addr_gp) {
                break;
            }
            gap += 1;
            if gap > MAX_GAP {
                break;
            }
            j = next_insn(store, infos, j + 1, len);
        }
        let Some(j) = consumer else {
            i += 1;
            continue;
        };
        if infos[j].pinned {
            i += 1;
            continue;
        }
        // Borrowed too: the EVEX filter below rejects most consumers, and
        // only a fold that survives every gate reaches `try_fold`, which is
        // the first thing in this pass that allocates.
        let cons = infos[j].trimmed(store.get(j));
        // Reject EVEX decorations: a masked or broadcasting operand is not the
        // plain `r/m` substitution this pass proves.
        if cons.contains('{') || cons.contains('}') || cons.contains("%k") {
            i += 1;
            continue;
        }
        let cons_mnemonic = cons.split_once(char::is_whitespace).map(|(m, _)| m);
        // Element-width discipline.  The retired load and the fused consumer
        // must move the SAME number of bytes, because the fold turns a
        // register operand of a known width into a bare address of an inferred
        // one.  `vmovsd (%rax), %xmm1` writes 64 bits and
        // `vaddpd %xmm1, %xmm2, %xmm3` reads 128, so the folded
        // `vaddpd (%rax), %xmm2, %xmm3` would read bytes 8..15 the original
        // never touched — a silent wrong answer, not a trap.  Same in reverse
        // for `vmovss` into a packed consumer.
        let t_class = vec_reg_class(t_reg).unwrap_or(1);
        if cons_mnemonic.and_then(|m| uniform_width_bytes(m, t_class))
            != uniform_width_bytes(load_mnemonic, t_class)
        {
            i += 1;
            continue;
        }
        let Some(new_line) = try_fold(&cons, &mem, t_reg) else {
            i += 1;
            continue;
        };
        // Alignment gate (see [`fold_needs_aligned_load`]): a legacy SSE
        // consumer may only re-read the address if the retired load was the
        // selector's own proof that the object is 16-byte aligned.
        if cons_mnemonic.is_some_and(fold_needs_aligned_load)
            && !ALIGNED_VEC_LOADS.contains(&load_mnemonic)
        {
            i += 1;
            continue;
        }
        // `%xmmT` must die at the consumer: the load's definition disappears,
        // and with it (for a VEX load) the zeroing of the architectural upper
        // regions.
        let oracle = fp.get_or_insert_with(|| FpLiveness::new(store, infos));
        if oracle.xmm_live_after(j, t_num) != Some(false) {
            i += 1;
            continue;
        }
        if load_text.trim_start().starts_with('v') && !oracle.upper_regions_dead_after(j, t_num) {
            i += 1;
            continue;
        }
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[j], j, new_line);
        changed = true;
        i = j + 1;
    }
    changed
}

/// Build the fused consumer text, or `None` when the consumer cannot take the
/// memory operand in the slot the loaded register occupies.
fn try_fold(cons: &str, mem: &str, t_reg: &str) -> Option<String> {
    let (mnemonic, operands) = cons.split_once(char::is_whitespace)?;
    let operands = operands.trim();
    let spans = operand_spans(operands);
    // The destination is the trailing operand and must not be the temporary:
    // a legacy two-operand op reads it, and a VEX destination that aliases the
    // source is a shape this pass declines rather than models.
    let last = spans.len() - 1;
    let dst = operands[spans[last].0..spans[last].1].trim();
    if dst == t_reg {
        return None;
    }
    // Width discipline.  The folded operand is a bare address, so the
    // assembler has to derive its size from the mnemonic plus the *other*
    // operands, and the destination is what pins it down.  That only works
    // while the temporary is no wider than the destination:
    // `vcvtpd2dq %ymm3, %xmm0` reads a 256-bit source into a 128-bit
    // destination, and the folded `vcvtpd2dq MEM, %xmm0` names no size GAS can
    // pick (it rejects the line outright).  Widening conversions are the
    // sound direction — `vcvtdq2pd %xmm3, %ymm0` folds to a `XMMWORD` operand
    // and `vpmovzxbd %xmm3, %ymm0` to a `QWORD` one, both exactly what the
    // register form read.  A destination that is not a vector register names
    // no width at all and is declined.
    if vec_reg_class(t_reg)? > vec_reg_class(dst).unwrap_or(0) {
        return None;
    }
    let slot = spans[..last]
        .iter()
        .position(|(lo, hi)| operands[*lo..*hi].trim() == t_reg)?;
    // `%xmmT` must appear in exactly one source slot.  A second occurrence
    // stays behind when the load retires and would then read whatever the
    // register held *before* the load instead of the loaded value — the
    // retired load is the only thing that made the two agree.  The shape is
    // not exotic: AT&T `vfmadd231sd %xmm1, %xmm1, %xmm12` is Intel
    // `VFMADD231SD xmm12, xmm1, xmm1`, where `%xmm1` is both the foldable
    // `r/m` operand and the VEX.vvvv addend.  Refusing costs one load in a
    // fused-multiply-add; getting it wrong corrupts the addend.
    if spans[..last]
        .iter()
        .enumerate()
        .any(|(k, (lo, hi))| k != slot && operands[*lo..*hi].trim() == t_reg)
    {
        return None;
    }
    if !slot_foldable(mnemonic, operands, &spans, slot) {
        return None;
    }
    let (lo, hi) = spans[slot];
    // Every operand after the folded slot must be a plain register: a second
    // memory operand would be an illegal encoding, and it would also mean the
    // instruction already reads memory the window rules never proved
    // reorderable.
    for &(a, b) in &spans[slot + 1..] {
        if !operands[a..b].trim().starts_with('%') {
            return None;
        }
    }
    Some(format!(
        "    {mnemonic} {}{mem}{}",
        &operands[..lo],
        &operands[hi..]
    ))
}

#[cfg(test)]
mod tests {
    use super::super::super::types::classify_line;
    use super::*;
    use crate::backend::peephole_common::LineStore;

    fn run(asm: &str) -> String {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        fuse_vector_load_into_op(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A whole function, so `FpLiveness` can delimit and analyse it.
    fn frame(body: &str) -> String {
        format!(".type f, @function\nf:\n{body}\n.size f, .-f\n")
    }

    fn body_of(asm: &str) -> String {
        run(&frame(asm))
            .lines()
            .filter(|l| !l.starts_with(".type ") && !l.starts_with(".size ") && l != &"f:")
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The same normalisation applied to an *expected* body written with the
    /// indentation of the surrounding source.
    fn norm(asm: &str) -> String {
        asm.lines().map(str::trim).collect::<Vec<_>>().join("\n")
    }

    /// The pass must leave `asm` byte-identical (modulo indentation).
    fn assert_unchanged(asm: &str) {
        assert_eq!(body_of(asm), norm(asm), "pass rewrote a refused shape");
    }

    #[test]
    fn folds_adjacent_packed_load() {
        let out = body_of(
            "    movaps (%rdx,%r9,1), %xmm15\n    mulps %xmm15, %xmm14\n    movups %xmm14, (%rdi)\n    ret",
        );
        assert!(out.contains("mulps (%rdx,%r9,1), %xmm14"), "{out}");
        assert!(!out.contains("movaps (%rdx,%r9,1), %xmm15"), "{out}");
    }

    #[test]
    fn folds_across_an_intervening_load() {
        // The exact shape the selector emits for `x[i]*y[i]`: the two operand
        // loads are adjacent and the second one defines the destination.
        let out = body_of(
            "    movaps (%rdx,%r9,1), %xmm15\n    movups (%rsi,%r9,1), %xmm14\n    mulps %xmm15, %xmm14\n    movups %xmm14, (%rdi)\n    ret",
        );
        assert!(out.contains("mulps (%rdx,%r9,1), %xmm14"), "{out}");
        assert!(out.contains("movups (%rsi,%r9,1), %xmm14"), "{out}");
    }

    #[test]
    fn refuses_an_unaligned_load_with_a_legacy_consumer() {
        // `movups` asserts nothing about alignment, and a legacy SSE
        // arithmetic instruction with a misaligned memory operand traps on
        // hosts that enforce alignment there (this tree's own test host).
        // Clang makes the same distinction: it folds `movapd` loads and
        // leaves `movupd` loads alone.
        assert_unchanged(
            "    movups (%rdx,%r9,1), %xmm15\n    mulps %xmm15, %xmm14\n    movups %xmm14, (%rdi)\n    ret",
        );
        // Same shape, one aligned spelling: the selector's own proof.
        let out = body_of(
            "    movdqa (%rdx,%r9,1), %xmm15\n    pmulld %xmm15, %xmm14\n    movdqu %xmm14, (%rdi)\n    ret",
        );
        assert!(out.contains("pmulld (%rdx,%r9,1), %xmm14"), "{out}");
    }

    #[test]
    fn folds_an_unaligned_load_with_a_vex_consumer() {
        // VEX arithmetic operands carry no alignment requirement at all, so
        // an unaligned load spelling is fine there.
        let out = body_of(
            "    vmovups (%rdx,%r9,1), %ymm3\n    vmulps %ymm3, %ymm4, %ymm5\n    vmovups %ymm5, (%rdi)\n    ret",
        );
        assert!(out.contains("vmulps (%rdx,%r9,1), %ymm4, %ymm5"), "{out}");
    }

    #[test]
    fn refuses_when_the_temporary_stays_live() {
        let asm =
            "    movups (%rdx), %xmm1\n    mulps %xmm1, %xmm2\n    movaps %xmm1, %xmm0\n    ret";
        assert_unchanged(asm);
    }

    #[test]
    fn refuses_a_legacy_self_operand() {
        // `movups MEM,%xmm3; mulps %xmm3,%xmm3` is MEM*MEM; folding would
        // compute stale(%xmm3)*MEM.
        let asm = "    movups (%rdx), %xmm3\n    mulps %xmm3, %xmm3\n    ret";
        assert_unchanged(asm);
    }

    #[test]
    fn refuses_across_a_store() {
        let asm =
            "    movups (%rdx), %xmm1\n    movq %rax, (%rdx)\n    mulps %xmm1, %xmm2\n    ret";
        assert_unchanged(asm);
    }

    #[test]
    fn refuses_across_an_address_register_write() {
        let asm = "    movups (%rdx), %xmm1\n    addq $16, %rdx\n    mulps %xmm1, %xmm2\n    ret";
        assert_unchanged(asm);
    }

    #[test]
    fn refuses_across_a_label() {
        let asm = "    movups (%rdx), %xmm1\n.L1:\n    mulps %xmm1, %xmm2\n    ret";
        assert_unchanged(asm);
    }

    #[test]
    fn folds_the_vex_three_operand_form() {
        let out = body_of(
            "    vmovups (%rdx,%r9,1), %ymm3\n    vmulps %ymm3, %ymm4, %ymm5\n    vmovups %ymm5, (%rdi)\n    ret",
        );
        assert!(out.contains("vmulps (%rdx,%r9,1), %ymm4, %ymm5"), "{out}");
    }

    #[test]
    fn folds_the_vex_three_operand_slot() {
        // `vsubps src2, src1, dst` is dst = src1 - src2; folding must not
        // swap the operands.
        let out = body_of(
            "    vmovups (%rdx), %ymm3\n    vsubps %ymm3, %ymm4, %ymm5\n    vmovups %ymm5, (%rdi)\n    ret",
        );
        assert!(out.contains("vsubps (%rdx), %ymm4, %ymm5"), "{out}");
    }

    #[test]
    fn refuses_when_the_temporary_is_also_the_vex_vvvv_addend() {
        // AT&T `vfmadd231sd %xmm1, %xmm1, %xmm12` is Intel
        // `VFMADD231SD xmm12, xmm1, xmm1`: `%xmm1` is both the foldable `r/m`
        // operand and the VEX.vvvv addend.  Retiring the load would leave the
        // addend reading `%xmm1` as it was *before* the load, which is a
        // miscompile.  This is the exact shape that broke
        // tests/regression/fma_slot_operand_staging.c.
        assert_unchanged(
            "    movsd 48(%rsp), %xmm1\n    vfmadd231sd %xmm1, %xmm1, %xmm12\n    ret",
        );
    }

    #[test]
    fn refuses_when_the_temporary_is_both_scalar_sources() {
        // Same hazard one mnemonic family over: `%xmm1` is `r/m` and the
        // `vvvv` addend of a `vaddsd` with a distinct destination.
        assert_unchanged("    movsd 8(%rsp), %xmm1\n    vaddsd %xmm1, %xmm1, %xmm2\n    ret");
    }

    #[test]
    fn declines_when_the_temporary_is_also_the_destination() {
        // A VEX destination is a pure write, so this shape would be sound,
        // but the pass declines every destination-aliasing form rather than
        // model the difference, and a legacy two-operand destination is a
        // genuine read.  Pinned so the conservative choice is deliberate.
        assert_unchanged(
            "    vmovups (%rdx), %ymm1\n    vaddps %ymm1, %ymm4, %ymm1\n    vmovups %ymm1, (%rdi)\n    ret",
        );
    }

    #[test]
    fn refuses_a_scalar_load_into_a_packed_consumer() {
        // The width rule.  `vmovsd` writes 64 bits and zeroes the rest;
        // `vaddpd` then reads 128.  Folding would produce
        // `vaddpd (%rax), %xmm2, %xmm3`, which reads bytes 8..15 the original
        // never touched.  A silent wrong answer, not a fault.
        let tail = "    vmovupd (%rsi), %xmm1\n    vmovupd %xmm1, (%rdx)\n    ret";
        assert_unchanged(&format!(
            "    vmovsd (%rax), %xmm1\n    vaddpd %xmm1, %xmm2, %xmm3\n    vmovupd %xmm3, (%rdi)\n{tail}"
        ));
        assert_unchanged(&format!(
            "    vmovss (%rax), %xmm1\n    vmulps %xmm1, %xmm2, %xmm3\n    vmovups %xmm3, (%rdi)\n{tail}"
        ));
        assert_unchanged(&format!(
            "    vmovsd (%rax), %xmm1\n    vaddps %xmm1, %xmm2, %xmm3\n    vmovaps %xmm3, (%rdi)\n{tail}"
        ));
    }

    #[test]
    fn still_folds_a_scalar_load_into_a_matching_scalar_consumer() {
        // Same width on both sides is sound and is where the scalar wins are.
        let tail = "    vmovupd (%rsi), %xmm1\n    vmovupd %xmm1, (%rdx)\n    ret";
        let out = body_of(&format!(
            "    vmovsd (%rax), %xmm1\n    vaddsd %xmm1, %xmm2, %xmm3\n    vmovsd %xmm3, (%rdi)\n{tail}"
        ));
        assert!(out.contains("vaddsd (%rax), %xmm2, %xmm3"), "{out}");
        let out = body_of(&format!(
            "    vmovss (%rax), %xmm1\n    vaddss %xmm1, %xmm2, %xmm3\n    vmovss %xmm3, (%rdi)\n{tail}"
        ));
        assert!(out.contains("vaddss (%rax), %xmm2, %xmm3"), "{out}");
    }

    #[test]
    fn refuses_a_scalar_load_feeding_a_wrong_width_scalar_consumer() {
        // 32-bit load, 64-bit read: `vmovss` writes 4 bytes, `vaddsd` reads 8.
        let tail = "    vmovupd (%rsi), %xmm1\n    vmovupd %xmm1, (%rdx)\n    ret";
        assert_unchanged(&format!(
            "    vmovss (%rax), %xmm1\n    vaddsd %xmm1, %xmm2, %xmm3\n    vmovsd %xmm3, (%rdi)\n{tail}"
        ));
    }

    #[test]
    fn refuses_pmaxsd_whose_sd_suffix_lies() {
        // `pmaxsd` is a PACKED dword compare: GAS reads 16 bytes from its `r/m`
        // operand despite the `sd` suffix.  Trusting the suffix would let a
        // 64-bit `vmovsd` load fold into a 128-bit read.
        let tail = "    vmovupd (%rsi), %xmm1\n    vmovupd %xmm1, (%rdx)\n    ret";
        assert_unchanged(&format!(
            "    vmovsd (%rax), %xmm1\n    vpmaxsd %xmm1, %xmm2, %xmm3\n    vmovdqu %xmm3, (%rdi)\n{tail}"
        ));
        // ...while a *packed* load, which really is 16 bytes, still folds.
        let out = body_of(&format!(
            "    vmovdqu (%rax), %xmm1\n    vpmaxsd %xmm1, %xmm2, %xmm3\n    vmovdqu %xmm3, (%rdi)\n{tail}"
        ));
        assert!(out.contains("vpmaxsd (%rax), %xmm2, %xmm3"), "{out}");
    }

    #[test]
    fn refuses_the_destination_dependent_widening_forms() {
        // `pmovzxbd` reads 4 bytes with an %xmm destination and 8 with a %ymm
        // one; `cvtps2pd` reads 8.  Neither width is stated by the text.
        let tail = "    vmovupd (%rsi), %xmm1\n    vmovupd %xmm1, (%rdx)\n    ret";
        assert_unchanged(&format!(
            "    vmovdqu (%rax), %xmm1\n    vpmovzxbd %xmm1, %xmm2\n    vmovdqu %xmm2, (%rdi)\n{tail}"
        ));
        assert_unchanged(&format!(
            "    vmovdqu (%rax), %xmm1\n    vcvtps2pd %xmm1, %xmm2\n    vmovupd %xmm2, (%rdi)\n{tail}"
        ));
    }

    #[test]
    fn refuses_a_vex_shift_by_immediate() {
        // The whole point of [`VEX_REG_ONLY_IMM_SRCS`]: this text assembles,
        // but only as the EVEX encoding `62 f1 65 28 72 37 03`, which needs
        // AVX-512VL and traps SIGILL on the AVX2-only targets
        // `-march=x86-64-v3` promises.  Caught by
        // tests/regression/check_bb_slp_v7_codegen.sh (v7_mf_shift).
        assert_unchanged(
            "    vmovdqu (%rdi), %ymm3\n    vpslld $3, %ymm3, %ymm3\n    vmovdqu %ymm3, (%rsi)\n    ret",
        );
        assert_unchanged(
            "    vmovdqu (%rdi), %ymm3\n    vpsrld $3, %ymm3, %ymm1\n    vmovdqu %ymm1, (%rsi)\n    ret",
        );
        assert_unchanged(
            "    vmovdqu (%rdi), %ymm3\n    vpslldq $3, %ymm3, %ymm1\n    vmovdqu %ymm1, (%rsi)\n    ret",
        );
    }

    #[test]
    fn still_folds_a_vex_shuffle_by_immediate() {
        // `vpshufd $imm, src, dst` really does take `r/m` in AT&T operand 1,
        // so refusing the shift group must not take the shuffles with it.
        let out = body_of(
            "    vmovdqu (%rdi), %ymm3\n    vpshufd $0x1b, %ymm3, %ymm1\n    vmovdqu %ymm1, (%rsi)\n    ret",
        );
        assert!(out.contains("vpshufd $0x1b, (%rdi), %ymm1"), "{out}");
    }

    #[test]
    fn refuses_when_the_temporary_is_wider_than_the_destination() {
        // `vcvtpd2dq %ymm3, %xmm0` reads 256 bits and writes 128.  The folded
        // `vcvtpd2dq MEM, %xmm0` leaves the operand size undeterminable and
        // GAS rejects the line, so the pass must never emit it.
        assert_unchanged(
            "    vmovupd (%rdi), %ymm3\n    vcvtpd2dq %ymm3, %xmm0\n    vmovdqu %xmm0, (%rsi)\n    ret",
        );
        assert_unchanged(
            "    vmovupd (%rdi), %ymm3\n    vcvttpd2dq %ymm3, %xmm0\n    vmovdqu %xmm0, (%rsi)\n    ret",
        );
    }

    #[test]
    fn refuses_a_conversion_whose_operand_width_follows_the_destination() {
        // `vcvtdq2pd %xmm3, %ymm0` reads 128 bits but `vcvtdq2pd %xmm3, %xmm0`
        // reads 64 — the operand size follows the *destination*, not the
        // source, so a 128-bit `%xmm` load is only safe for one of the two.
        // The pass declines the family rather than model that (see
        // [`NON_UNIFORM_WIDTH_BASES`]).
        let tail = "    vmovupd (%rsi), %xmm3\n    vmovupd %xmm3, (%rdx)\n    ret";
        assert_unchanged(&format!(
            "    vmovdqu (%rdi), %xmm3\n    vcvtdq2pd %xmm3, %ymm0\n    vmovupd %ymm0, (%rsi)\n{tail}"
        ));
    }

    #[test]
    fn refuses_a_non_vector_destination() {
        // A GP destination names no vector width for the assembler to infer.
        assert_unchanged("    vmovdqu (%rdi), %xmm3\n    vpextrd $1, %xmm3, %eax\n    ret");
    }

    #[test]
    fn refuses_the_vex_vvvv_slot() {
        // AT&T operand 1 of a VEX three-operand instruction is the VEX.vvvv
        // source, which is register-only: `vsubps %ymm4, MEM, %ymm5` has no
        // encoding (lccc's assembler rejects it, and so does GAS).
        assert_unchanged(
            "    vmovups (%rdx), %ymm3\n    vsubps %ymm4, %ymm3, %ymm5\n    vmovups %ymm5, (%rdi)\n    ret",
        );
    }

    #[test]
    fn refuses_the_fma_vvvv_slot() {
        // The same constraint for FMA3: `vfmadd231ps %ymm4, MEM, %ymm5` is
        // not encodable, only `vfmadd231ps MEM, %ymm4, %ymm5` is.
        assert_unchanged(
            "    vmovups (%rdx), %ymm3\n    vfmadd231ps %ymm4, %ymm3, %ymm5\n    vmovups %ymm5, (%rdi)\n    ret",
        );
    }

    #[test]
    fn folds_an_immediate_first_consumer() {
        let out = body_of(
            "    vmovups (%rdx), %ymm3\n    vpshufd $0x1b, %ymm3, %ymm4\n    vmovups %ymm4, (%rdi)\n    ret",
        );
        assert!(out.contains("vpshufd $0x1b, (%rdx), %ymm4"), "{out}");
    }

    #[test]
    fn refuses_an_extract_that_spells_a_memory_destination() {
        // `vpextrd $1, %xmm3, %eax` — slot 1 is the *destination*, and the
        // mnemonic is not in the immediate-first allow list.
        let asm = "    vmovups (%rdx), %xmm3\n    vpextrd $1, %xmm3, %eax\n    ret";
        assert_unchanged(asm);
    }

    #[test]
    fn refuses_a_pinned_volatile_load() {
        let mut store = LineStore::new(frame(
            "    movups (%rdx), %xmm1\n    mulps %xmm1, %xmm2\n    ret",
        ));
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let load = (0..store.len())
            .find(|&i| store.get(i).trim().starts_with("movups"))
            .unwrap();
        infos[load].pinned = true;
        infos[load].volatile_access = true;
        assert!(!fuse_vector_load_into_op(&mut store, &mut infos));
    }

    #[test]
    fn refuses_an_unknown_consumer_mnemonic() {
        let asm = "    movups (%rdx), %xmm1\n    aesenc %xmm1, %xmm2\n    ret";
        assert_unchanged(asm);
    }

    #[test]
    fn refuses_when_the_upper_regions_stay_live() {
        // `vmovups` into `%xmm3` zeroes bits 255:128 of `%ymm3`; the shuffle
        // reads them back, so the load cannot be retired.
        let asm = "    vmovups (%rdx), %xmm3\n    vaddps %xmm3, %xmm4, %xmm5\n    vmovups %ymm3, (%rdi)\n    ret";
        assert_unchanged(asm);
    }
}
