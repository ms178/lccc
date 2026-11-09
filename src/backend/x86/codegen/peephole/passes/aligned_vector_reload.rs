//! Full-width vector store→reload forwarding through an over-aligned frame
//! address.
//!
//! # The shape
//!
//! An over-aligned local (`_Alignas(32)`, a `__m256` home, an aligned array)
//! cannot be reached with a fixed `off(%rbp)`: the frame base is only 16-byte
//! aligned at entry, so `emit_alloca_addr_to`
//! (`src/backend/x86/codegen/memory.rs`) materialises the address at EVERY
//! use as `leaq K(%rbp), %r; andq $-A, %r` and then dereferences `(%r)`.
//! A vector copy-out/copy-in therefore emits
//!
//! ```text
//!     leaq -193(%rbp), %rax
//!     andq $-32, %rax
//!     vmovdqu %ymm0, (%rax)     # spill the value
//!     leaq -193(%rbp), %rax     # SAME address, re-derived
//!     andq $-32, %rax
//!     vmovups (%rax), %ymm3     # reload it immediately
//! ```
//!
//! The reload is a pure round-trip: `%ymm0` still holds the bits.  It costs a
//! load, and — far worse — it puts a store-to-load forwarding stall (~5-6
//! cycles) on the loop-carried dependency chain.  Measured on an AVX-512 host
//! with `tests/simd/vector_home_call_loop.c`, rewriting the reload to
//! `vmovups %ymm0, %ymm3` takes the loop from 2 374 298 to 1 425 636 cycles per
//! 200 000 iterations (**-40.0%**); the existing copy-propagation and
//! dead-store passes then retire the now-dead spill entirely (-43.6%).
//! Removing only the duplicated `leaq`/`andq` measurably does NOT help
//! (+1.1%, i.e. noise): the address ALU work is hidden by the out-of-order
//! engine, the forwarding stall is not.  That measurement is why this pass
//! forwards the reload instead of CSE-ing the address.
//!
//! # Why the existing store-forwarding pass does not cover it
//!
//! `store_forwarding::global_store_forwarding` keys mappings by a literal
//! `off(%rbp)` slot offset and models GP families plus the *scalar-FP* XMM
//! sizes (`movsd|movss|movq|movd`).  This shape has neither property: the
//! address is indirect, and the operand is a whole 256-bit register.  Rather
//! than widen that pass's `MoveSize`/`RegHomes` invariants — which are woven
//! through ~2800 lines and assume a ≤64-bit value — this is a separate,
//! small, self-contained walk whose entire state is provable from the
//! instructions it has just seen.
//!
//! # Soundness
//!
//! The tracked address is a **canonical frame key** `(disp, align)`, not a
//! register identity.  `leaq K(%rbp), %r` establishes `(K, 1)` for `%r`;
//! an immediately following `andq $-A, %r` refines it to `(K, A)`.  Two
//! accesses with the same key denote the same bytes *for a fixed `%rbp`*,
//! which is exactly what lets the reload fire even though `%rax` was
//! overwritten in between.  `%rbp` itself needs no frame-pointer proof: any
//! instruction that could change it is not one of the recognised forms, so
//! it clears the state (and `leave`, which changes `%rbp` without naming it,
//! is rejected explicitly).
//!
//! A pending store survives ONLY across
//!
//! * the address idiom (touches no memory and no vector register),
//! * nops / inert directives ([`steps_over_line`]), and
//! * a reload it just forwarded whose destination is a DIFFERENT vector
//!   register (same bytes still in memory, same bytes still in the source).
//!
//! EVERY other line — a call, a label, a branch, a `ret`, an unknown store,
//! an arithmetic op, inline asm, a data-emitting directive — clears both the
//! pending store and all address keys.  This is deliberately blunt: it makes
//! "the stored register is still intact" and "nothing wrote that memory"
//! true by construction rather than by a clobber analysis that could be
//! wrong.  The peephole driver pins inline-asm and volatile regions before
//! phase 2 runs, so those arrive here as `pinned` and also clear the state.
//! Consequently the rewrite can only ever fire inside straight-line code
//! with no intervening call.
//!
//! The rewrite requires
//!
//! 1. store and load are both full-width moves from [`MOVE_MNEMONICS`], in
//!    the **same encoding class** (legacy SSE vs VEX/EVEX).  Class matters
//!    more than width: a legacy-SSE 128-bit load preserves the upper YMM bits
//!    while a VEX-128 load zeroes them, so mixing classes would change the
//!    destination's upper half.
//! 2. the load is **no wider** than the store.  A narrower load reads the low
//!    bytes of the stored value, which are exactly the low bytes of the
//!    stored register, so the source is re-viewed at the LOAD's width
//!    (`%ymm0` → `%xmm0`).  A WIDER load would read bytes the store never
//!    wrote and is refused.
//! 3. the load's base register carries the pending key, and the memory
//!    operand is a bare `(%r)` — no index, no scale, no displacement, so the
//!    load starts at offset 0 of the stored object and there is no address
//!    arithmetic left to disagree about.
//! 4. neither line is `pinned` or `volatile_access`.  A volatile read is an
//!    observable event and must survive even when its value is known.
//!
//! The emitted instruction reuses the **load's own mnemonic** in its
//! register/register form.  That keeps the transformation the smallest
//! possible semantic delta — same opcode, memory source replaced by the
//! register that provably holds those bytes — so it is auditable without a
//! mnemonic-equivalence table, and it needs exactly the ISA feature the load
//! already needed (every mnemonic in [`MOVE_MNEMONICS`] has a register form).
//! Within one encoding class and width the register form writes exactly the
//! bits the memory form wrote, including the VEX/EVEX upper-half zeroing.
//!
//! Only the LOAD is removed.  The store is left alone, so no escape analysis
//! is needed: the memory image is unchanged and any other observer still
//! sees the same bytes.
//!
//! All state is `Copy`: tracking a store allocates nothing, so the pass adds
//! no heap traffic to the compiler itself.

use super::super::types::*;
use super::helpers::*;

/// Full-width, unmasked moves whose register/register form moves exactly the
/// operand width.  Deliberately excludes:
///
/// * `movsd`/`movss`/`movd`/`movq` — partial or merging, handled by the
///   scalar-FP path in `store_forwarding`;
/// * `movnt*` — non-temporal stores have ordering semantics a register move
///   cannot reproduce;
/// * `vmovdqu8`/`vmovdqu16` — AVX-512BW granularities that exist for masked
///   forms;
/// * `movhl`/`movlh`/`movddup`/… — lane shuffles, not width-preserving copies.
const MOVE_MNEMONICS: &[&str] = &[
    "movaps",
    "movups",
    "movapd",
    "movupd",
    "movdqa",
    "movdqu",
    "vmovaps",
    "vmovups",
    "vmovapd",
    "vmovupd",
    "vmovdqa",
    "vmovdqu",
    "vmovdqa32",
    "vmovdqa64",
    "vmovdqu32",
    "vmovdqu64",
];

/// Vector register width class.  Comparison is by [`Width::bytes`], so
/// "no wider than" is an explicit byte-count test rather than an implicit
/// ordering that a future variant could silently reorder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Width {
    Xmm = 0,
    Ymm = 1,
    Zmm = 2,
}

impl Width {
    fn bytes(self) -> u8 {
        match self {
            Self::Xmm => 16,
            Self::Ymm => 32,
            Self::Zmm => 64,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Xmm => "xmm",
            Self::Ymm => "ymm",
            Self::Zmm => "zmm",
        }
    }
}

/// A vector register named by (width, index) instead of a heap `String`, so
/// tracking a store allocates nothing.  Re-viewing a stored register at a
/// narrower width is then just a different `width` with the same `index`,
/// which is exactly the architectural aliasing of `%xmmN`/`%ymmN`/`%zmmN`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct VecReg {
    width: Width,
    index: u8,
}

impl VecReg {
    /// Parse `%xmmN` / `%ymmN` / `%zmmN` with N < 32 and nothing else, so an
    /// opmask (`%k1`), a GP register or a decorated operand can never be
    /// mistaken for a vector register.
    fn parse(tok: &str) -> Option<Self> {
        let rest = tok.strip_prefix('%')?;
        let (width, digits) = if let Some(d) = rest.strip_prefix("xmm") {
            (Width::Xmm, d)
        } else if let Some(d) = rest.strip_prefix("ymm") {
            (Width::Ymm, d)
        } else {
            (Width::Zmm, rest.strip_prefix("zmm")?)
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let index = digits.parse::<u8>().ok()?;
        if index >= 32 {
            return None;
        }
        Some(Self { width, index })
    }
}

impl core::fmt::Display for VecReg {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "%{}{}", self.width.prefix(), self.index)
    }
}

/// The canonical frame address a GP register holds: `disp(%rbp)` rounded
/// down to `align`.  `align == 1` means "not rounded".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FrameKey {
    disp: i32,
    align: i64,
}

/// A vector store whose bytes are still live in `src` and still in memory at
/// `key`.
#[derive(Clone, Copy)]
struct PendingStore {
    key: FrameKey,
    src: VecReg,
    vex: bool,
}

/// Split a two-operand instruction into (mnemonic, first, second).
/// Returns `None` for any other operand count, so a three-operand or
/// zero-operand form can never be misread.
fn two_ops(t: &str) -> Option<(&str, &str, &str)> {
    let (mnem, rest) = t.split_once(|c: char| c.is_ascii_whitespace())?;
    let rest = rest.trim();
    // A memory operand may contain commas of its own (`(%rax,%rbx,4)`), and
    // it may be EITHER operand (`vmovdqu %ymm0, (%rax)` as well as
    // `vmovdqu (%rax), %ymm0`), so the split has to be paren-aware in both
    // directions rather than assuming a fixed order.
    let mut depth = 0i32;
    let mut split = None;
    for (k, &b) in rest.as_bytes().iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            b',' if depth == 0 => {
                if split.is_some() {
                    return None; // three or more operands
                }
                split = Some(k);
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    let k = split?;
    let (a, b) = (rest[..k].trim(), rest[k + 1..].trim());
    if a.is_empty() || b.is_empty() {
        return None;
    }
    Some((mnem, a, b))
}

/// The `disp` and the inside of the parentheses of a `DISP(%reg)` operand.
/// `DISP` may be absent (meaning 0).
fn split_mem(mem: &str) -> Option<(i32, &str)> {
    let open = mem.find('(')?;
    if !mem.ends_with(')') {
        return None;
    }
    let inner = &mem[open + 1..mem.len() - 1];
    let disp_txt = &mem[..open];
    let disp = if disp_txt.is_empty() {
        0
    } else {
        disp_txt.parse::<i32>().ok()?
    };
    Some((disp, inner))
}

/// `(%reg)` with no displacement, index or scale.
fn bare_base_fam(mem: &str) -> Option<RegId> {
    let (disp, inner) = split_mem(mem)?;
    if disp != 0 || inner.contains(',') || inner.contains('*') {
        return None;
    }
    let fam = register_family_fast(inner);
    // %rsp moves with push/pop and outgoing call setup, so a key based on it
    // would not be stable; %rbp-relative keys are the whole point.
    if fam == REG_NONE || fam > REG_GP_MAX || fam == REG_RSP {
        return None;
    }
    Some(fam)
}

/// A GP destination this pass may track: never `%rsp` (moves implicitly) and
/// never `%rbp` (it is the key's base, so writing it must clear state, not
/// establish a key).
fn trackable_gp(tok: &str) -> Option<RegId> {
    if !tok.starts_with('%') {
        return None;
    }
    let fam = register_family_fast(tok);
    if fam == REG_NONE || fam > REG_GP_MAX || fam == REG_RSP || fam == REG_RBP {
        return None;
    }
    Some(fam)
}

/// Recognise `leaq DISP(%rbp), %reg` (DISP optional) → the register's family
/// and the key `(DISP, 1)`.
fn parse_frame_lea(t: &str) -> Option<(RegId, FrameKey)> {
    let (mnem, mem, dst) = two_ops(t)?;
    if mnem != "leaq" {
        return None;
    }
    let (disp, inner) = split_mem(mem)?;
    if inner != "%rbp" {
        return None;
    }
    Some((trackable_gp(dst)?, FrameKey { disp, align: 1 }))
}

/// Recognise `andq $-A, %reg` with A a power of two ≥ 2.
fn parse_align_and(t: &str) -> Option<(RegId, i64)> {
    let (mnem, imm, dst) = two_ops(t)?;
    if mnem != "andq" {
        return None;
    }
    let mask = imm.strip_prefix('$')?.parse::<i64>().ok()?;
    if mask >= 0 {
        return None;
    }
    // mask == -A; A must be a power of two so the AND really is an alignment.
    // `checked_neg` keeps i64::MIN from overflowing under overflow-checks.
    let align = mask.checked_neg()?;
    if align < 2 || (align & (align - 1)) != 0 {
        return None;
    }
    Some((trackable_gp(dst)?, align))
}

/// A full-width vector move between a register and a bare `(%base)`.
///
/// The mnemonic is the interned `&'static str` from [`MOVE_MNEMONICS`], not a
/// slice of the input line: the caller rewrites that very line, so holding a
/// borrow of it across `LineStore::replace` would be a borrow error — and
/// copying it into a `String` would allocate on every vector move.
struct VecMemMove {
    mnem: &'static str,
    reg: VecReg,
    base: RegId,
    vex: bool,
    is_store: bool,
}

fn parse_vec_mem_move(t: &str) -> Option<VecMemMove> {
    let (mnem, a, b) = two_ops(t)?;
    let mnem = MOVE_MNEMONICS.iter().copied().find(|&m| m == mnem)?;
    let (reg, mem, is_store) = if a.starts_with('%') {
        (a, b, true)
    } else {
        (b, a, false)
    };
    Some(VecMemMove {
        mnem,
        reg: VecReg::parse(reg)?,
        base: bare_base_fam(mem)?,
        vex: mnem.starts_with('v'),
        is_store,
    })
}

/// Forward reloads of a just-stored vector through an over-aligned frame
/// address.  Returns whether anything changed.
pub(super) fn forward_aligned_vector_reload(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len().min(infos.len());
    let mut changed = false;
    // Indexed by GP family (0..=15); every writer of this array has already
    // refused REG_NONE, %rsp and %rbp, so the index is always in range.
    let mut addr: [Option<FrameKey>; 16] = [None; 16];
    let mut pending: Option<PendingStore> = None;
    for i in 0..len {
        let info = infos[i];
        if info.pinned || info.volatile_access {
            addr = [None; 16];
            pending = None;
            continue;
        }
        let t = info.trimmed(store.get(i));
        if steps_over_line(&info, t) {
            continue;
        }
        // `leave` rewrites %rbp and %rsp without naming either.
        if t == "leave" || t == "leaveq" {
            addr = [None; 16];
            pending = None;
            continue;
        }
        if let Some((fam, key)) = parse_frame_lea(t) {
            addr[fam as usize] = Some(key);
            continue;
        }
        if let Some((fam, align)) = parse_align_and(t) {
            // Only a direct refinement of a plain `leaq disp(%rbp)` is a key
            // we can name.  Anything else (a second AND, an AND on a register
            // of unknown provenance) retires the key.
            addr[fam as usize] = match addr[fam as usize] {
                Some(k) if k.align == 1 => Some(FrameKey {
                    disp: k.disp,
                    align,
                }),
                _ => None,
            };
            continue;
        }
        if let Some(mv) = parse_vec_mem_move(t) {
            let key = addr[mv.base as usize];
            if mv.is_store {
                pending = key.map(|k| PendingStore {
                    key: k,
                    src: mv.reg,
                    vex: mv.vex,
                });
                continue;
            }
            let Some(ps) = pending else {
                continue;
            };
            // The load writes a vector register.  If that is the pending
            // source (same index, any width) the source no longer holds the
            // stored bytes afterwards, so the mapping dies either way.
            pending = None;
            if key != Some(ps.key)
                || mv.reg.width.bytes() > ps.src.width.bytes()
                || mv.vex != ps.vex
            {
                continue;
            }
            // Re-view the source at the LOAD's width: `%xmm{N}` is the low
            // 128 bits of `%ymm{N}`/`%zmm{N}`, which is exactly what a
            // narrower load at offset 0 reads.
            let src = VecReg {
                width: mv.reg.width,
                index: ps.src.index,
            };
            let text = format!("    {} {}, {}", mv.mnem, src, mv.reg);
            infos[i] = classify_line(&text);
            store.replace(i, text);
            changed = true;
            if mv.reg.index != ps.src.index {
                // Different register: memory and source are both unchanged,
                // so a second reload of the same store can forward too.
                pending = Some(ps);
            }
            continue;
        }
        // Everything else may write memory, the stored register, or %rbp.
        addr = [None; 16];
        pending = None;
    }
    changed
}

#[cfg(test)]
mod aligned_vector_reload_tests {
    use super::*;
    use crate::backend::peephole_common::LineStore;

    fn build(asm: &str) -> (LineStore, Vec<LineInfo>) {
        let store = LineStore::new(asm.to_string());
        let infos = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        (store, infos)
    }

    fn run(asm: &str) -> String {
        let (mut store, mut infos) = build(asm);
        forward_aligned_vector_reload(&mut store, &mut infos);
        (0..store.len())
            .map(|i| store.get(i).to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    const STORE: &str = "    leaq -193(%rbp), %rax\n    andq $-32, %rax\n    vmovdqu %ymm0, (%rax)";
    const RELOAD: &str =
        "    leaq -193(%rbp), %rax\n    andq $-32, %rax\n    vmovups (%rax), %ymm3";

    #[test]
    fn the_measured_shape_is_forwarded_across_a_rederived_address() {
        let out = run(&format!("{STORE}\n{RELOAD}"));
        assert!(
            out.contains("vmovups %ymm0, %ymm3"),
            "reload must become a register copy:\n{out}"
        );
        assert!(
            !out.contains("vmovups (%rax), %ymm3"),
            "the memory reload must be gone:\n{out}"
        );
        // The store survives: this pass removes a read, never a write.
        assert!(out.contains("vmovdqu %ymm0, (%rax)"), "{out}");
    }

    #[test]
    fn a_plain_unaligned_frame_address_also_forwards() {
        let asm = "    leaq -64(%rbp), %rcx\n    vmovaps %xmm1, (%rcx)\n    vmovaps (%rcx), %xmm2";
        assert!(run(asm).contains("vmovaps %xmm1, %xmm2"));
    }

    #[test]
    fn a_zero_displacement_lea_is_a_key() {
        let asm = "    leaq (%rbp), %rdx\n    vmovdqu %ymm4, (%rdx)\n    vmovdqu (%rdx), %ymm5";
        assert!(run(asm).contains("vmovdqu %ymm4, %ymm5"));
    }

    #[test]
    fn two_registers_holding_the_same_key_forward() {
        let asm = "    leaq -64(%rbp), %rax\n    vmovdqu %ymm0, (%rax)\n    leaq -64(%rbp), %rcx\n    vmovdqu (%rcx), %ymm3";
        assert!(run(asm).contains("vmovdqu %ymm0, %ymm3"));
    }

    #[test]
    fn sse_and_vex_classes_are_never_mixed() {
        // Legacy SSE preserves the upper YMM bits, VEX-128 zeroes them.
        let asm = "    leaq -64(%rbp), %rax\n    movdqu %xmm0, (%rax)\n    vmovups (%rax), %xmm3";
        assert!(run(asm).contains("vmovups (%rax), %xmm3"));
        let asm = "    leaq -64(%rbp), %rax\n    vmovdqu %xmm0, (%rax)\n    movups (%rax), %xmm3";
        assert!(run(asm).contains("movups (%rax), %xmm3"));
    }

    #[test]
    fn legacy_sse_pair_forwards_with_an_sse_move() {
        let asm = "    leaq -64(%rbp), %rax\n    movups %xmm0, (%rax)\n    movaps (%rax), %xmm3";
        let out = run(asm);
        assert!(out.contains("movaps %xmm0, %xmm3"), "{out}");
        assert!(
            !out.contains("vmov"),
            "must not invent a VEX opcode:\n{out}"
        );
    }

    #[test]
    fn zmm_reuses_the_loads_evex_mnemonic() {
        let asm = "    leaq -128(%rbp), %rax\n    andq $-64, %rax\n    vmovdqu64 %zmm2, (%rax)\n    vmovdqa64 (%rax), %zmm5";
        assert!(run(asm).contains("vmovdqa64 %zmm2, %zmm5"));
    }

    #[test]
    fn a_narrower_load_reviews_the_source_at_its_own_width() {
        // 32-byte store, 16-byte load at offset 0: the low half.
        let asm = format!("{STORE}\n    vmovups (%rax), %xmm3");
        let out = run(&asm);
        assert!(out.contains("vmovups %xmm0, %xmm3"), "{out}");
        let asm = "    leaq -128(%rbp), %rax\n    andq $-64, %rax\n    vmovdqu64 %zmm2, (%rax)\n    vmovdqu (%rax), %ymm5";
        assert!(run(asm).contains("vmovdqu %ymm2, %ymm5"));
    }

    #[test]
    fn a_wider_load_is_refused() {
        for (st, ld) in [
            ("vmovdqu %xmm0, (%rax)", "vmovups (%rax), %ymm3"),
            ("vmovdqu %ymm0, (%rax)", "vmovdqu64 (%rax), %zmm3"),
            ("vmovdqu %xmm0, (%rax)", "vmovdqu64 (%rax), %zmm3"),
        ] {
            let asm = format!("    leaq -64(%rbp), %rax\n    {st}\n    {ld}");
            assert!(run(&asm).contains(ld), "{st} / {ld} must be refused");
        }
    }

    #[test]
    fn a_different_address_is_refused() {
        let asm = format!(
            "{STORE}\n    leaq -129(%rbp), %rax\n    andq $-32, %rax\n    vmovups (%rax), %ymm3"
        );
        assert!(run(&asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn a_different_alignment_is_refused() {
        let asm = format!(
            "{STORE}\n    leaq -193(%rbp), %rax\n    andq $-16, %rax\n    vmovups (%rax), %ymm3"
        );
        assert!(run(&asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn an_intervening_call_kills_the_mapping() {
        let asm = format!("{STORE}\n    call f\n    vmovups (%rax), %ymm3");
        assert!(run(&asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn an_intervening_write_to_the_source_register_kills_it() {
        let asm = format!("{STORE}\n    vpaddd %ymm1, %ymm2, %ymm0\n    vmovups (%rax), %ymm3");
        assert!(run(&asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn an_intervening_unknown_store_kills_it() {
        let asm = format!("{STORE}\n    movq %rdx, (%rdi)\n    vmovups (%rax), %ymm3");
        assert!(run(&asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn an_intervening_write_to_rbp_kills_it() {
        let asm = format!("{STORE}\n    movq %rdx, %rbp\n    vmovups (%rax), %ymm3");
        assert!(run(&asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn leave_kills_it_even_though_it_does_not_name_rbp() {
        let asm = format!("{STORE}\n    leave\n    vmovups (%rax), %ymm3");
        assert!(run(&asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn a_label_kills_the_mapping() {
        let asm = format!("{STORE}\n.L9:\n    vmovups (%rax), %ymm3");
        assert!(run(&asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn a_second_store_replaces_the_pending_value() {
        let asm = format!("{STORE}\n    vmovdqu %ymm7, (%rax)\n    vmovups (%rax), %ymm3");
        let out = run(&asm);
        assert!(out.contains("vmovups %ymm7, %ymm3"), "{out}");
        assert!(!out.contains("vmovups %ymm0, %ymm3"), "{out}");
    }

    #[test]
    fn two_reloads_of_one_store_both_forward() {
        let asm = format!("{STORE}\n    vmovups (%rax), %ymm3\n    vmovups (%rax), %ymm4");
        let out = run(&asm);
        assert!(out.contains("vmovups %ymm0, %ymm3"), "{out}");
        assert!(out.contains("vmovups %ymm0, %ymm4"), "{out}");
    }

    #[test]
    fn a_reload_into_the_source_register_ends_the_mapping() {
        // The first reload rewrites %ymm0, so the second must NOT forward.
        let asm = format!("{STORE}\n    vmovups (%rax), %ymm0\n    vmovups (%rax), %ymm4");
        let out = run(&asm);
        assert!(out.contains("vmovups %ymm0, %ymm0"), "{out}");
        assert!(out.contains("vmovups (%rax), %ymm4"), "{out}");
    }

    #[test]
    fn a_narrowing_reload_into_the_source_also_ends_the_mapping() {
        // VEX-128 into %xmm0 zeroes %ymm0's upper half: the 32 stored bytes
        // are no longer in the register.
        let asm = format!("{STORE}\n    vmovups (%rax), %xmm0\n    vmovups (%rax), %ymm4");
        let out = run(&asm);
        assert!(out.contains("vmovups %xmm0, %xmm0"), "{out}");
        assert!(out.contains("vmovups (%rax), %ymm4"), "{out}");
    }

    #[test]
    fn an_indexed_or_displaced_operand_is_refused() {
        for mem in ["(%rax,%rbx,4)", "8(%rax)", "(%rax,%rbx)"] {
            let asm = format!(
                "    leaq -64(%rbp), %rax\n    vmovdqu %ymm0, {mem}\n    vmovups {mem}, %ymm3"
            );
            assert!(
                run(&asm).contains(&format!("vmovups {mem}, %ymm3")),
                "{mem}"
            );
        }
    }

    #[test]
    fn rsp_is_never_used_as_a_key_base_or_destination() {
        let asm = "    leaq -64(%rsp), %rsp\n    vmovdqu %ymm0, (%rsp)\n    vmovups (%rsp), %ymm3";
        assert!(run(asm).contains("vmovups (%rsp), %ymm3"));
        let asm = "    leaq -64(%rbp), %rsp\n    vmovdqu %ymm0, (%rsp)\n    vmovups (%rsp), %ymm3";
        assert!(run(asm).contains("vmovups (%rsp), %ymm3"));
    }

    #[test]
    fn rbp_is_never_a_tracked_destination() {
        let asm = "    leaq -64(%rbp), %rbp\n    vmovdqu %ymm0, (%rbp)\n    vmovups (%rbp), %ymm3";
        assert!(run(asm).contains("vmovups (%rbp), %ymm3"));
    }

    #[test]
    fn partial_and_non_temporal_moves_are_ignored() {
        for mnem in ["movsd", "movss", "movntdq", "vmovntdq", "vmovdqu8"] {
            let asm = format!(
                "    leaq -64(%rbp), %rax\n    {mnem} %xmm0, (%rax)\n    {mnem} (%rax), %xmm3"
            );
            assert!(
                run(&asm).contains(&format!("{mnem} (%rax), %xmm3")),
                "{mnem} must be left alone"
            );
        }
    }

    #[test]
    fn a_volatile_or_pinned_load_is_not_rewritten() {
        let asm = "    leaq -64(%rbp), %rax\n    vmovdqu %ymm0, (%rax)\n    vmovups (%rax), %ymm3";
        let (mut store, mut infos) = build(asm);
        let last = infos.len() - 1;
        infos[last].volatile_access = true;
        infos[last].pinned = true;
        assert!(!forward_aligned_vector_reload(&mut store, &mut infos));
        assert!(store.get(last).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn a_pinned_store_does_not_establish_a_mapping() {
        let asm = "    leaq -64(%rbp), %rax\n    vmovdqu %ymm0, (%rax)\n    vmovups (%rax), %ymm3";
        let (mut store, mut infos) = build(asm);
        infos[1].pinned = true;
        assert!(!forward_aligned_vector_reload(&mut store, &mut infos));
    }

    #[test]
    fn a_self_reload_becomes_a_self_move_the_existing_pass_retires() {
        let asm = "    leaq -64(%rbp), %rax\n    vmovdqu %ymm0, (%rax)\n    vmovups (%rax), %ymm0";
        assert!(run(asm).contains("vmovups %ymm0, %ymm0"));
    }

    #[test]
    fn inert_directives_are_stepped_over_but_data_directives_are_not() {
        let asm = format!("{STORE}\n    .cfi_remember_state\n    vmovups (%rax), %ymm3");
        assert!(run(&asm).contains("vmovups %ymm0, %ymm3"));
        let asm = format!("{STORE}\n    .byte 0\n    vmovups (%rax), %ymm3");
        assert!(run(&asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn an_and_without_a_preceding_lea_is_not_a_key() {
        let asm = "    andq $-32, %rax\n    vmovdqu %ymm0, (%rax)\n    vmovups (%rax), %ymm3";
        assert!(run(asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn a_second_and_is_not_a_refinement() {
        let asm = "    leaq -64(%rbp), %rax\n    andq $-32, %rax\n    andq $-32, %rax\n    vmovdqu %ymm0, (%rax)\n    vmovups (%rax), %ymm3";
        assert!(run(asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn a_non_power_of_two_mask_is_not_an_alignment() {
        let asm = "    leaq -64(%rbp), %rax\n    andq $-48, %rax\n    vmovdqu %ymm0, (%rax)\n    vmovups (%rax), %ymm3";
        assert!(run(asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn a_positive_mask_is_not_an_alignment() {
        let asm = "    leaq -64(%rbp), %rax\n    andq $31, %rax\n    vmovdqu %ymm0, (%rax)\n    vmovups (%rax), %ymm3";
        assert!(run(asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn an_opmask_register_is_not_a_vector_operand() {
        let asm =
            "    leaq -64(%rbp), %rax\n    vmovdqa32 %zmm0, (%rax)\n    vmovdqa32 (%rax), %k1";
        assert!(run(asm).contains("vmovdqa32 (%rax), %k1"));
    }

    #[test]
    fn an_out_of_range_register_index_is_refused() {
        assert!(VecReg::parse("%ymm32").is_none());
        assert!(VecReg::parse("%ymm999").is_none());
        assert!(VecReg::parse("%ymm").is_none());
        assert!(VecReg::parse("%ymm0a").is_none());
        assert!(VecReg::parse("ymm0").is_none());
        assert!(VecReg::parse("%k1").is_none());
        assert!(VecReg::parse("%rax").is_none());
        assert_eq!(VecReg::parse("%zmm31").unwrap().to_string(), "%zmm31");
    }

    #[test]
    fn a_huge_displacement_is_refused_without_overflowing() {
        let asm = "    leaq 99999999999999999999(%rbp), %rax\n    vmovdqu %ymm0, (%rax)\n    vmovups (%rax), %ymm3";
        assert!(run(asm).contains("vmovups (%rax), %ymm3"));
    }

    #[test]
    fn the_extreme_mask_does_not_overflow() {
        // i64::MIN negates to 2^63, which is a power of two but not an
        // alignment any real frame uses; checked_neg must not panic.
        let asm = format!(
            "    leaq -64(%rbp), %rax\n    andq $-9223372036854775808, %rax\n{}",
            "    vmovdqu %ymm0, (%rax)\n    vmovups (%rax), %ymm3"
        );
        let _ = run(&asm);
    }

    #[test]
    fn three_operand_forms_are_not_moves() {
        assert!(two_ops("vmovdqu %ymm0, %ymm1, %ymm2").is_none());
        assert!(two_ops("vzeroupper").is_none());
        assert!(two_ops("vmovdqu %ymm0").is_none());
        assert!(two_ops("vmovdqu (%rax), (%rdx)").is_some());
        assert!(two_ops("vmovdqu %ymm0, (%rax,%rbx,4)").is_some());
    }

    #[test]
    fn unbalanced_parentheses_are_refused() {
        assert!(two_ops("vmovdqu %ymm0, (%rax").is_none());
        assert!(two_ops("vmovdqu %ymm0, )%rax(").is_none());
    }

    #[test]
    fn width_bytes_and_prefixes_are_consistent() {
        assert_eq!(
            [Width::Xmm.bytes(), Width::Ymm.bytes(), Width::Zmm.bytes()],
            [16, 32, 64]
        );
        assert_eq!(
            [
                Width::Xmm.prefix(),
                Width::Ymm.prefix(),
                Width::Zmm.prefix()
            ],
            ["xmm", "ymm", "zmm"]
        );
        for (tok, w) in [
            ("%xmm7", Width::Xmm),
            ("%ymm7", Width::Ymm),
            ("%zmm7", Width::Zmm),
        ] {
            let r = VecReg::parse(tok).unwrap();
            assert_eq!(r.width, w);
            assert_eq!(r.index, 7);
            assert_eq!(r.to_string(), tok);
        }
    }

    #[test]
    fn the_pass_is_idempotent() {
        let asm = format!("{STORE}\n{RELOAD}");
        let once = run(&asm);
        assert_eq!(run(&once), once, "a second run must change nothing");
    }

    #[test]
    fn empty_input_is_a_no_op() {
        let (mut store, mut infos) = build("");
        assert!(!forward_aligned_vector_reload(&mut store, &mut infos));
    }
}
