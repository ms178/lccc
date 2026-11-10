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
//!   `vpalignr`, `vcmpps`, …) take `r/m` in AT&T operand 1.
//!
//! Packed (and scalar) arithmetic never requires an aligned memory operand —
//! the alignment trap belongs to `movaps`/`movdqa` only — so an unaligned
//! `movups`/`movdqu` load folds just as safely as an aligned one.
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
                slot == 1 && IMM_FIRST_BASES.binary_search(&base).is_ok()
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

/// Rejection census for `CCC_DEBUG_VLFUSE=1`: counts *why* candidates were
/// declined so a tuning session can tell "no opportunity here" apart from
/// "the gate is too strict".
#[derive(Default)]
struct Census {
    candidates: usize,
    no_consumer: usize,
    not_foldable: usize,
    temp_live: usize,
    upper_live: usize,
    alignment: usize,
    folded: usize,
}

static CENSUS: std::sync::Mutex<Census> = std::sync::Mutex::new(Census {
    candidates: 0,
    no_consumer: 0,
    not_foldable: 0,
    temp_live: 0,
    upper_live: 0,
    alignment: 0,
    folded: 0,
});

/// `CCC_DEBUG_VLFUSE=1` dumps the rejection census to stderr once per process.
pub(super) fn dump_census() {
    if std::env::var_os("CCC_DEBUG_VLFUSE").is_none() {
        return;
    }
    if let Ok(c) = CENSUS.lock() {
        crate::cli::stderr_line(format_args!(
            "[VLFUSE] candidates={} folded={} no_consumer={} not_foldable={} temp_live={} upper_live={} alignment={}",
            c.candidates,
            c.folded,
            c.no_consumer,
            c.not_foldable,
            c.temp_live,
            c.upper_live,
            c.alignment
        ));
    }
    dump_rejects();
}

/// Mnemonic histogram of the consumers `try_fold` declined, so a tuning
/// session can see which instruction families are missing from the
/// allow-lists instead of guessing.
static REJECTS: std::sync::Mutex<Option<std::collections::BTreeMap<String, usize>>> =
    std::sync::Mutex::new(None);

fn census_reject(mnemonic: &str) {
    census_bump("not_foldable");
    if std::env::var_os("CCC_DEBUG_VLFUSE").is_none() {
        return;
    }
    if let Ok(mut m) = REJECTS.lock() {
        *m.get_or_insert_with(Default::default)
            .entry(mnemonic.to_string())
            .or_default() += 1;
    }
}

/// Print the decline histogram.  Called from [`dump_census`].
fn dump_rejects() {
    if let Ok(m) = REJECTS.lock() {
        if let Some(map) = &*m {
            let mut rows: Vec<_> = map.iter().collect();
            rows.sort_by(|a, b| b.1.cmp(a.1));
            let top: Vec<String> = rows
                .iter()
                .take(24)
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            crate::cli::stderr_line(format_args!("[VLFUSE] declined: {}", top.join(" ")));
        }
    }
}

fn census_bump(kind: &str) {
    if std::env::var_os("CCC_DEBUG_VLFUSE").is_none() {
        return;
    }
    if let Ok(mut c) = CENSUS.lock() {
        match kind {
            "cand" => c.candidates += 1,
            "no_consumer" => c.no_consumer += 1,
            "not_foldable" => c.not_foldable += 1,
            "temp_live" => c.temp_live += 1,
            "upper_live" => c.upper_live += 1,
            "alignment" => c.alignment += 1,
            "folded" => c.folded += 1,
            _ => {}
        }
    }
}

/// Fuse `movups MEM, %xmmT … OP %xmmT, %xmmD` into `OP MEM, %xmmD`.
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
        let load_text = infos[i].trimmed(store.get(i)).to_string();
        let Some((mem, t_reg)) = parse_vector_load(&load_text) else {
            i += 1;
            continue;
        };
        census_bump("cand");
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
        let mem = mem.to_string();

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
            census_bump("no_consumer");
            i += 1;
            continue;
        };
        if infos[j].pinned {
            i += 1;
            continue;
        }
        let cons = infos[j].trimmed(store.get(j)).to_string();
        // Reject EVEX decorations: a masked or broadcasting operand is not the
        // plain `r/m` substitution this pass proves.
        if cons.contains('{') || cons.contains('}') || cons.contains("%k") {
            i += 1;
            continue;
        }
        let Some(new_line) = try_fold(&cons, &mem, t_reg) else {
            census_reject(
                cons.split_once(char::is_whitespace)
                    .map(|(m, _)| m)
                    .unwrap_or("?"),
            );
            i += 1;
            continue;
        };
        // Alignment gate (see [`fold_needs_aligned_load`]): a legacy SSE
        // consumer may only re-read the address if the retired load was the
        // selector's own proof that the object is 16-byte aligned.
        let cons_mnemonic = cons.split_once(char::is_whitespace).map(|(m, _)| m);
        if cons_mnemonic.is_some_and(fold_needs_aligned_load)
            && !ALIGNED_VEC_LOADS.contains(&load_mnemonic)
        {
            census_bump("alignment");
            i += 1;
            continue;
        }
        // `%xmmT` must die at the consumer: the load's definition disappears,
        // and with it (for a VEX load) the zeroing of the architectural upper
        // regions.
        let oracle = fp.get_or_insert_with(|| FpLiveness::new(store, infos));
        if oracle.xmm_live_after(j, t_num) != Some(false) {
            census_bump("temp_live");
            i += 1;
            continue;
        }
        if load_text.trim_start().starts_with('v') && !oracle.upper_regions_dead_after(j, t_num) {
            census_bump("upper_live");
            i += 1;
            continue;
        }
        census_bump("folded");
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
    if operands[spans[last].0..spans[last].1].trim() == t_reg {
        return None;
    }
    let slot = spans[..last]
        .iter()
        .position(|(lo, hi)| operands[*lo..*hi].trim() == t_reg)?;
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
