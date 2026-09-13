//! Dead pure-write elimination and redundant-load reuse.
//!
//! Two block-level cleanups that need the deadness proofs from
//! [`super::relay_and_lea`] but are not tied to a particular instruction pair:
//!
//! * [`eliminate_dead_pure_writes`] deletes an instruction whose only effect is
//!   writing a register that is never read again. `dead_code::eliminate_dead_reg_moves`
//!   only recognises register-to-register `movq` inside an 8-instruction window;
//!   the address computations the newer folds leave behind (`leaq 0(,%rbx,4), %rdi`
//!   with the array access folded into a SIB operand) and widened copies
//!   (`movzbl`, `movslq`, `setCC`) survive it.
//! * [`reuse_redundant_loads`] rewrites a second load of the same memory operand
//!   into a register copy. The scan crosses forward conditional branches — the
//!   fall-through of a conditional jump is dominated by the branch — but stops
//!   at labels (join points), at anything that writes memory, and at any write
//!   to a register the address depends on.

use super::super::types::*;
use super::helpers::{has_implicit_reg_usage, writes_family};
use super::liveness::FileLiveness;
use super::relay_and_lea::{
    is_relayable_family, plain_gp_operand, provably_dead_lv, split_two_operands,
};

/// Instructions whose ONLY effect is the register they write: no flags, no
/// memory write, no implicit operand. `cmov` is included (it reads flags but
/// does not write them); `setCC` likewise.
const PURE_WRITE_PREFIXES: &[&str] = &[
    "movl ", "movq ", "movb ", "movw ", "movabsq ", "movzbl ", "movzbq ", "movzwl ", "movzwq ",
    "movsbl ", "movsbq ", "movswl ", "movswq ", "movslq ", "leal ", "leaq ", "cmov", "set",
];

/// True when the operand text is a register (no memory reference).
fn is_register_operand(text: &str) -> bool {
    text.starts_with('%') && !text.contains('(')
}

/// True when `src` is safe to drop: a register, an immediate, or an address
/// computation (`lea` never dereferences). A memory SOURCE is deliberately not
/// accepted — deleting a load can remove a fault the program relied on
/// (volatile / MMIO / a trapping access the surrounding code expects).
fn droppable_source(op: &str, src: &str) -> bool {
    if op.starts_with("lea") {
        return true;
    }
    is_register_operand(src) || src.starts_with('$')
}

/// Delete instructions that only write a register nobody reads afterwards.
pub(super) fn eliminate_dead_pure_writes(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        let Some(op) = PURE_WRITE_PREFIXES.iter().find(|p| t.starts_with(**p)) else {
            i += 1;
            continue;
        };
        // `setCC %al` has a single operand; everything else is `SRC, DST`.
        let (src, dst) = match split_two_operands(&t[op.len()..]) {
            Some((s, d)) => (s, d),
            None if op.starts_with("set") => ("", t[op.len()..].trim()),
            None => {
                i += 1;
                continue;
            }
        };
        if !is_register_operand(dst) || (!src.is_empty() && !droppable_source(op, src)) {
            i += 1;
            continue;
        }
        let Some(fam) = plain_gp_operand(dst) else {
            i += 1;
            continue;
        };
        if !is_relayable_family(fam) {
            i += 1;
            continue;
        }
        // A self-move is handled elsewhere; a write that also READS the family
        // (`leaq 1(%rax), %rax`) is not dead just because the family dies —
        // it is dead exactly when the family dies, which the proof covers, but
        // the proof scans from the NEXT line, so the self-read is already past.
        if provably_dead_lv(&lv, store, infos, i, fam, &[i]) {
            mark_nop(&mut infos[i]);
            lv.refresh_at(store, infos, i);
            changed = true;
        }
        i += 1;
    }
    changed
}

/// True when `label_idx` (a label line) has no incoming branch: the only way
/// to reach it is to fall through the preceding instruction. Such a label
/// starts a block dominated by its predecessor, so a value cached before it is
/// still valid after it.
#[expect(clippy::needless_range_loop)]
pub(super) fn label_is_fallthrough_only(
    store: &LineStore,
    infos: &[LineInfo],
    label_idx: usize,
) -> bool {
    let t = infos[label_idx].trimmed(store.get(label_idx));
    let Some(name) = t.strip_suffix(':').map(str::trim) else {
        return false;
    };
    // A label has a non-fallthrough predecessor when ANY other line names
    // it. Direct `jmp`/`jcc` are the common case, but switch lowering reaches
    // case blocks through an indirect `jmpq *%rdx` whose targets are spelled
    // only in jump-table data (`.long .LBB4 - .Ljt_0`, `.quad .LBBn`):
    // scanning jump mnemonics alone mislabels every jump-table target as
    // fallthrough-only and would let folds/cascades cross into a block that
    // is entered with different register state. Tokenise on the assembler
    // symbol alphabet so `.L1` cannot match `.L10`; over-matching only ever
    // fails closed (label treated as targeted, fold refused).
    for n in 0..store.len() {
        if infos[n].is_nop() || n == label_idx {
            continue;
        }
        let tn = infos[n].trimmed(store.get(n));
        if tn
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '_' && c != '$')
            .any(|tok| tok == name)
        {
            return false;
        }
    }
    true
}

/// Memory operand of a simple load `mov* MEM, %reg`, plus the register families
/// the address reads.
fn load_operand(t: &str) -> Option<(&'static str, &str, &str, Vec<RegId>)> {
    const LOADS: &[&str] = &[
        "movl ", "movq ", "movb ", "movw ", "movzbl ", "movzbq ", "movzwl ", "movzwq ", "movsbl ",
        "movsbq ", "movswl ", "movswq ", "movslq ",
    ];
    let op = LOADS.iter().find(|p| t.starts_with(**p))?;
    let (src, dst) = split_two_operands(&t[op.len()..])?;
    if !src.contains('(') || !is_register_operand(dst) {
        return None;
    }
    // Only plain `DISP(%base[,%index[,scale]])` operands; no rip-relative
    // (a symbol may be written through another alias) and no segment override.
    if src.contains("%rip") || src.contains(':') {
        return None;
    }
    let open = src.find('(')?;
    let close = src.rfind(')')?;
    if close + 1 != src.len() {
        return None;
    }
    let mut fams = Vec::new();
    for f in src[open + 1..close].split(',') {
        let f = f.trim();
        if f.is_empty() || matches!(f, "1" | "2" | "4" | "8") {
            continue;
        }
        let fam = register_family_fast(f);
        if fam == REG_NONE || fam > REG_GP_MAX {
            return None;
        }
        fams.push(fam);
    }
    Some((op, src, dst, fams))
}

/// Replace a repeated load of the same address with a register copy.
pub(super) fn reuse_redundant_loads(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    // Bisect switches for the two precisions added here, read once per
    // invocation. Both default ON; each can be disabled independently so a
    // miscompile can be attributed to exactly one of them.
    let policy = ReusePolicy {
        same_dst_reload: std::env::var("CCC_NO_SAME_DST_RELOAD").is_err(),
        frame_slot_aliasing: std::env::var("CCC_NO_FRAME_SLOT_ALIASING").is_err(),
    };
    reuse_redundant_loads_with(store, infos, policy)
}

/// The two precisions this pass can apply, as explicit policy so both arms are
/// unit-testable without mutating the process environment (env mutation in
/// tests is racy: every test in the binary shares one environment).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct ReusePolicy {
    /// Delete a reload of the same operand into the same destination.
    pub(super) same_dst_reload: bool,
    /// Let a store to a different, non-overlapping frame slot survive the scan.
    pub(super) frame_slot_aliasing: bool,
}

impl ReusePolicy {
    /// Both precisions on: the shipping configuration.
    pub(super) const fn full() -> Self {
        Self {
            same_dst_reload: true,
            frame_slot_aliasing: true,
        }
    }
    /// Both off: the historical behaviour, kept testable as a baseline.
    pub(super) const fn legacy() -> Self {
        Self {
            same_dst_reload: false,
            frame_slot_aliasing: false,
        }
    }
}

fn reuse_redundant_loads_with(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    policy: ReusePolicy,
) -> bool {
    let redundant_same_dst_reload = policy.same_dst_reload;
    let frame_slot_aliasing = policy.frame_slot_aliasing;
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let first = infos[i].trimmed(store.get(i)).to_string();
        let Some((op, mem, dst, addr_fams)) = load_operand(&first) else {
            i += 1;
            continue;
        };
        let Some(dst_fam) = plain_gp_operand(dst) else {
            i += 1;
            continue;
        };
        let mut addr_mask = 0u16;
        for f in &addr_fams {
            addr_mask |= 1u16 << f;
        }
        // A load whose destination is one of its OWN address registers
        // destroys its address operand: `movq (%r15), %r15` leaves %r15
        // holding the loaded value, not the address. A later load with the
        // same textual memory operand therefore reads a DIFFERENT location
        // (the value loaded here), so this load can never be a reuse source
        // for it. The per-line `addr_mask` invalidation below cannot catch
        // this: it only inspects lines AFTER the load, and the clobber is
        // the load itself.
        //
        // Found by scripts/gen_gep_chain_stress.py (seed 12, -O2): a hash
        // bucket walk emitted
        //     leaq 8(%r11), %r15     # &b->chain
        //     movq (%r15), %r15      # chain   <- dst == addr
        //     movq (%r15), %r12      # chain[0]
        // and the third instruction was rewritten to `movq %r15, %r12`,
        // returning the `char **` instead of the string it points at.
        if addr_fams.contains(&dst_fam) {
            i += 1;
            continue;
        }
        let dst_mask = 1u16 << dst_fam;
        let mem_owned = mem.to_string();
        let dst_owned = dst.to_string();
        // Frame-slot precision for the aliasing test below (see
        // `store_cannot_alias_slot`). `None` for anything that is not a plain
        // `D(%rsp)` / `D(%rbp)` operand, which restores the historical
        // "every store invalidates" behaviour verbatim.
        let cached_slot = if frame_slot_aliasing {
            parse_frame_slot(&mem_owned, mnemonic_mem_width(op))
        } else {
            None
        };

        let mut j = i + 1;
        while j < len {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }
            let t = infos[j].trimmed(store.get(j));
            // i686: `classify_line` only recognizes 64-bit `pushq`/`popq`,
            // so 32-bit pushes classify as `Other` and their implicit
            // `%esp` motion is invisible both here and in `writes_family`
            // (no effects-table row either). Break textually when the
            // cached slot is `%esp`-relative; `%ebp`-relative scans may
            // continue across pushes (ebp is stable). Scoped to this pass
            // on purpose: classifying 32-bit pushes pipeline-wide would
            // hand `pushl` to push/pop consumers that may assume 8-byte
            // frames.
            {
                let mnemonic = t.split_whitespace().next().unwrap_or("");
                if addr_fams.contains(&4)
                    && matches!(
                        mnemonic,
                        "pushl"
                            | "pushw"
                            | "push"
                            | "popl"
                            | "popw"
                            | "pop"
                            | "pushfl"
                            | "pushfw"
                            | "popfl"
                            | "popfw"
                    )
                {
                    break;
                }
            }
            match infos[j].kind {
                // A label is normally a join point: another predecessor may
                // have stored to this address. A label nobody branches to is
                // reached only by falling through, so the cached value holds.
                LineKind::Label => {
                    if !label_is_fallthrough_only(store, infos, j) {
                        break;
                    }
                    j += 1;
                    continue;
                }
                LineKind::Call | LineKind::Ret | LineKind::JmpIndirect => break,
                // An unconditional jump ends the straight-line region; the
                // fall-through of a CONDITIONAL jump is dominated by it, so the
                // scan may continue there.
                LineKind::Jmp => break,
                LineKind::Push { .. } | LineKind::Pop { .. } => break,
                _ => {}
            }
            if infos[j].pinned || has_implicit_reg_usage(t) {
                break;
            }
            // Any write to memory invalidates the cached value: without alias
            // analysis every store is assumed to hit this address. The one
            // exception is a store to a DIFFERENT, non-overlapping frame slot,
            // which is not an aliasing claim but a fact about the static frame
            // layout (see `store_cannot_alias_slot`).
            if line_writes_memory(t) && !store_cannot_alias_slot(cached_slot, t) {
                break;
            }
            // A write to an address register invalidates the operand. The
            // mentions gate keeps the common case O(1); `writes_family`
            // supplies the exact answer for the lines it admits — including
            // implicit writes such as `cqto` overwriting an %rdx address.
            if infos[j].reg_refs & addr_mask != 0
                && addr_fams.iter().any(|&f| writes_family(&infos[j], t, f))
            {
                break;
            }
            // A reload of the SAME operand into the SAME destination is wholly
            // redundant: the address registers are unchanged, no aliasing store
            // intervened, and it writes the destination the value it already
            // holds. Delete it rather than rewriting it. This must be tested
            // BEFORE the destination-write break below, because that break is
            // exactly what otherwise ends the scan on this line -- which is why
            // same-register reloads survived here while different-register ones
            // were already being folded into copies.
            if redundant_same_dst_reload {
                if let Some((op2, mem2, dst2, _)) = load_operand(t) {
                    if op2 == op && mem2 == mem_owned && dst2 == dst_owned {
                        // Keep a reload that the stack-load→ALU memory-operand
                        // fold is about to consume (`movq slot,%rcx; addq
                        // %rcx,%rax` → `addq slot,%rax`). Deleting it here
                        // makes a later consumer read the surviving earlier
                        // load instead, and the fold's register-deadness
                        // proof fails for the EARLIER load, so every member
                        // of a reload chain stays materialized (one extra
                        // instruction per chain). Preserving the redundant
                        // copy costs nothing: the fold deletes it in the
                        // very next phase. See load_would_memfold.
                        if super::memory_fold::load_would_memfold(store, infos, j) {
                            j += 1;
                            continue;
                        }
                        mark_nop(&mut infos[j]);
                        changed = true;
                        j += 1;
                        continue;
                    }
                }
            }
            // The first load's destination must still hold the value.
            if infos[j].reg_refs & dst_mask != 0 && writes_family(&infos[j], t, dst_fam) {
                break;
            }
            if let Some((op2, mem2, dst2, _)) = load_operand(t) {
                if op2 == op && mem2 == mem_owned && dst2 != dst_owned {
                    let dst2_fam = register_family_fast(dst2);
                    if dst2_fam == REG_NONE || dst2_fam > REG_GP_MAX {
                        j += 1;
                        continue;
                    }
                    // Pick the copy width by the load's WIDTH CLASS so the
                    // substitution preserves every observable bit of the
                    // second load's result.
                    //
                    // * `movq MEM, %rX` writes the full 64-bit family. Use
                    //   `movq` to copy all 64 bits.
                    // * Sign-extending-to-64-bit loads (`movsbq`/`movswq`/
                    //   `movslq`) write the full 64-bit family with sign-
                    //   extension. The first load's destination ALREADY
                    //   holds the sign-extended 64-bit value, so a `movq`
                    //   faithfully reproduces it. A `movl` here would zero-
                    //   extend the upper 32 bits and DESTROY the sign
                    //   extension — that was the v6 miscompile.
                    // * 32-bit-class loads (`movl`, `movzbl`, `movzwl`,
                    //   `movsbl`, `movswl`, and the zero-extending-to-64-bit
                    //   `movzbq`/`movzwq` whose result is the zero-extended
                    //   value) all IMPLICITLY zero-extend the upper 32 bits
                    //   of the family. A `movl` copy reproduces that exactly.
                    // * `movb`/`movw` PRESERVE the upper bits of the family
                    //   rather than zero/sign-extending them. A `movl`/`movq`
                    //   copy would clobber those bits, which the second load
                    //   would have kept. REFUSE the substitution.
                    let Some((mov, src_name, dst_name)) = (|| {
                        let wide64 = REG_NAMES[0][dst_fam as usize];
                        let wide64_dst = REG_NAMES[0][dst2_fam as usize];
                        let n32_src = REG_NAMES[1][dst_fam as usize];
                        let n32_dst = REG_NAMES[1][dst2_fam as usize];
                        if op.starts_with("movq")
                            || op.starts_with("movsbq")
                            || op.starts_with("movswq")
                            || op.starts_with("movslq")
                        {
                            Some(("movq", wide64, wide64_dst))
                        } else if op.starts_with("movl")
                            || op.starts_with("movzbl")
                            || op.starts_with("movzwl")
                            || op.starts_with("movzbq")
                            || op.starts_with("movzwq")
                            || op.starts_with("movsbl")
                            || op.starts_with("movswl")
                        {
                            Some(("movl", n32_src, n32_dst))
                        } else {
                            None // movb/movw: refuse, see comment above
                        }
                    })() else {
                        j += 1;
                        continue;
                    };
                    let new_line = format!("    {} {}, {}", mov, src_name, dst_name);
                    replace_line(store, &mut infos[j], j, new_line);
                    changed = true;
                }
            }
            j += 1;
        }
        i += 1;
    }
    changed
}

/// A plain frame-relative operand `D(%rsp)` / `D(%rbp)` and the byte range it
/// accesses. Indexed forms (`D(%rsp,%rax,4)`) and rip-relative forms are
/// rejected: their effective address is not a fixed range, so nothing can be
/// proven disjoint from them.
#[derive(Clone, Copy)]
pub(super) struct FrameSlot {
    /// Register family of the base (`%rsp` = 4, `%rbp` = 5).
    base: RegId,
    disp: i64,
    width: i64,
}

pub(super) fn parse_frame_slot(mem: &str, width: i64) -> Option<FrameSlot> {
    let open = mem.find('(')?;
    if !mem.ends_with(')') {
        return None;
    }
    let inside = &mem[open + 1..mem.len() - 1];
    // Reject SIB (base,index[,scale]) and anything that is not exactly the
    // stack pointer or the frame pointer.
    if inside.contains(',') {
        return None;
    }
    // Reject a segment override (`%fs:360(%rsp)`) EXPLICITLY. It happens to be
    // rejected anyway, because the displacement text then parses as "%fs:360"
    // and `parse::<i64>` fails - but soundness should not rest on a parse
    // failure in a different field. A segmented stack access is a different
    // address space as far as this proof is concerned.
    if mem.contains(':') {
        return None;
    }
    // i686: `%esp`/`%ebp` are the same families 4/5 in `scan_register_refs`
    // (types.rs, the `b'e'` arm), so the "base register cannot move inside
    // the scan" barrier transfers verbatim: `subl $N, %esp` writes family 4
    // and ends the scan exactly like `subq $N, %rsp`. Push/pop/call/ret/Jmp
    // breaks are arch-independent `LineKind`s. The proof never uses the
    // x86-64 red zone (disjoint displacements under a fixed base are disjoint
    // bytes with or without one), so i686's lack of it is irrelevant; and
    // call-arg staging windows always adjust `%esp` first, which breaks the
    // scan before any staged store can be misjudged. Validated by executing
    // every differing TU natively (32-bit ELF runs on the x86-64 host).
    let base = match inside {
        "%rsp" | "%esp" => 4,
        "%rbp" | "%ebp" => 5,
        _ => return None,
    };
    let disp_txt = mem[..open].trim();
    let disp = if disp_txt.is_empty() {
        0
    } else {
        disp_txt.parse::<i64>().ok()?
    };
    Some(FrameSlot { base, disp, width })
}

/// Bytes an instruction touches in memory.
///
/// Two independent estimates, combined with `max` so the answer is never an
/// underestimate — an underestimate here is a miscompile, because it can make
/// two overlapping accesses look disjoint:
///
/// 1. The FIRST width suffix letter of the mnemonic, which in AT&T names the
///    memory operand's width for both plain (`movl`, `addq`, `incl`) and
///    extending (`movzbl`, `movslq`) forms. Unrecognized mnemonics (string ops,
///    x87 `fstpt`, no suffix) get 16, the widest scalar access.
/// 2. The width of any vector register mentioned on the line. This is the half
///    the suffix cannot supply: `vmovdqu %ymm0, 24(%rsp)` has a `d` in the
///    suffix position, so rule 1 charges it 16 bytes while it really writes 32.
///    Charging only 16 would report a cached load at slot 48 as disjoint from
///    it. `%zmm` (AVX-512) writes 64.
///
/// The register scan is deliberately textual and over-broad: a `%xmm` appearing
/// anywhere on the line forces at least 16 bytes even if it is only a source.
/// Over-estimating merely loses an optimization.
fn mnemonic_mem_width(t: &str) -> i64 {
    let start = if t.starts_with("lock ") { 5 } else { 0 };
    let suffix = match t.as_bytes().get(start + 3) {
        Some(b'b') => 1,
        Some(b'w') => 2,
        Some(b'l') => 4,
        Some(b'q') => 8,
        _ => 16,
    };
    let vector = if t.contains("%zmm") {
        64
    } else if t.contains("%ymm") {
        32
    } else if t.contains("%xmm") {
        16
    } else {
        0
    };
    suffix.max(vector)
}

/// The memory operand a store writes to, or `None`. Mirrors
/// [`line_writes_memory`]'s destination-finding rules (which must use
/// [`last_top_level_comma`], not `rfind`, because an AT&T memory operand
/// carries its own commas) but returns the operand instead of a bool.
fn store_destination(t: &str) -> Option<&str> {
    if t.starts_with("lea") {
        return None;
    }
    if let Some(comma) = last_top_level_comma(t.as_bytes()) {
        let dst = t[comma + 1..].trim();
        return if dst.contains('(') { Some(dst) } else { None };
    }
    // Single-operand read-modify-write form: `incl 24(%rsp)`, `negq (%rax)`.
    let sp = t.find(' ')?;
    let only = t[sp + 1..].trim();
    if !t.starts_with('j') && only.contains('(') {
        Some(only)
    } else {
        None
    }
}

/// True when a store on this line provably cannot touch the byte range of
/// `cached`, so it must not invalidate a load cached from that frame slot.
///
/// This is the precision [`reuse_redundant_loads`] lacked: it broke the scan on
/// *every* memory write, on the (sound but blunt) principle that without alias
/// analysis any store may hit the cached address. For a FRAME-SLOT operand that
/// principle gives away too much, because the frame layout is static and known:
/// two distinct, non-overlapping `D(%rsp)` ranges are different bytes, full
/// stop. Spill-heavy code interleaves slot accesses constantly, so this is what
/// lets a cached reload survive the neighbouring spills around it.
///
/// Everything not provably disjoint stays conservative:
///
/// * a store through a register (`movl %eax, (%r8)`) has no parseable frame
///   destination and returns false — no points-to claim is attempted;
/// * a store to a different base (`(%rbp)` vs the cached `(%rsp)`) returns
///   false, since the two bases are not provably a fixed distance apart here;
/// * an indexed or rip-relative destination returns false;
/// * an unrecognized mnemonic returns `None` from the width query (its
///   footprint cannot be bounded from the mnemonic alone), so the store
///   is treated as clashing with everything.
///
/// The base register itself cannot move inside the scan: `%rsp`/`%rbp` are
/// members of the cached load's `addr_fams`, and a write to any address family
/// already ends the scan above. So the displacements compared here are stable.
/// Exact upper bound on the bytes a STORE writes, or `None` when the write
/// footprint cannot be bounded from the mnemonic alone.
///
/// The disjointness proof in [`store_cannot_alias_slot`] must NEVER use an
/// under-estimate: a store charged 16 bytes that really writes 512 makes a
/// live cached load look disjoint and deletes it (miscompile). The
/// over-broad fallback in [`mnemonic_mem_width`] (16 for anything unknown)
/// is fine on the READ/cached side — it only loses optimization — but on
/// the store side it meets a whitelist:
///
/// * ordinary GP/extending/RMW mnemonics with a `b/w/l/q` suffix write
///   exactly 1/2/4/8 bytes, apart from the handful whose suffix lies about
///   their footprint (`fbstpb` stores 18 bytes of packed decimal,
///   `cmpxchg8b` writes 8);
/// * vector stores take the widest vector register named on the line
///   (over-broad is sound because it only enlarges the store's range);
/// * special bounded multi-byte stores are listed explicitly
///   (`cmpxchg8b` 8, `cmpxchg16b`/`movdir64b`/`enqcmd(s)` 16/64/64);
/// * state-save and environment instructions write large or
///   implementation-variable areas and always return `None`
///   (`fxsave[64]` 512, `xsave[c|opt|s][64]` up to 4 KiB+, `fnsave` 108,
///   `fstenv` 28/32, `stmxcsr`/`sgdt`/`sidt`/`sldt`/`smsw` unrecognized
///   suffix anyway, `enter`'s stacked frame).
fn bounded_store_mem_width(t: &str) -> Option<i64> {
    let mut s = t.trim();
    if let Some(rest) = s.strip_prefix("lock ") {
        s = rest;
    }
    if let Some(rest) = s.strip_prefix('v') {
        // VEX: vector width dominates; strip the v so the suffix checks
        // below see the base op (vmovntdq etc. are always vector stores).
        let vw = if rest.contains("%zmm") {
            64
        } else if rest.contains("%ymm") {
            32
        } else if rest.contains("%xmm") {
            16
        } else {
            // VEX op with no vector reg (e.g. vpinsrw always names xmm) —
            // cannot decide cheaply; fail closed.
            return None;
        };
        // State/string-ish VEX forms with oversized footprints are not
        // known to exist as frame stores; keep the vector bound.
        return Some(vw);
    }
    // Special bounded multi-byte stores (whole-token mnemonic match).
    let mnem = s.split_whitespace().next().unwrap_or("");
    let specials: &[(&str, i64)] = &[
        ("cmpxchg8b", 8),
        ("cmpxchg16b", 16),
        ("movdir64b", 64),
        ("enqcmd", 64),
        ("enqcmds", 64),
        // x87 memory stores — their last letter does NOT follow the GP
        // suffix convention: fstpl is an 8-byte double (not 4), fistpll an
        // 8-byte long-long and fstpt a 10-byte tbyte (charged 16, an
        // over-estimate is safe on the store side).
        ("fstps", 4),
        ("fsts", 4),
        ("fstpl", 8),
        ("fstl", 8),
        ("fstpt", 16),
        ("fistps", 2),
        ("fists", 2),
        ("fistpl", 4),
        ("fistl", 4),
        ("fistpll", 8),
        ("fisttps", 2),
        ("fisttpl", 4),
        ("fisttpll", 8),
        ("fstcw", 2),
        ("fnstcw", 2),
        ("fstsw", 2),
        ("fnstsw", 2),
    ];
    if let Some((_, w)) = specials.iter().find(|(m, _)| *m == mnem) {
        return Some(*w);
    }
    // Unbounded / implementation-variable state saves: never prove
    // disjointness through them. Prefix-matched so 32/64-bit spellings and
    // the `fn` non-waiting variants are all covered (`fxsave`/`fxsave64`,
    // `xsave[c|opt|s][64]`, `fnsave`, `fnstenv`, `fbstpb`=18 bytes …).
    const BULK_PREFIXES: &[&str] = &[
        "fxsave", "xsave", "fsave", "fnsave", "stenv", "fbstp", "sgdt", "sidt", "sldt", "smsw",
    ];
    if BULK_PREFIXES.iter().any(|p| mnem.starts_with(p)) {
        return None;
    }
    // A %xmm/%ymm/%zmm store without VEX prefix (movntdq, movdqa, maskmov…):
    // take the widest register, over-broad on purpose.
    if s.contains("%zmm") {
        return Some(64);
    }
    if s.contains("%ymm") {
        return Some(32);
    }
    if s.contains("%xmm") {
        return Some(16);
    }
    // MMX stores write 8 bytes; maskmovq writes through an indirect
    // destination and never parses as a plain frame slot anyway.
    if s.contains("%mm") && mnem.starts_with("mov") {
        return Some(8);
    }
    // Suffix-exact GP store / RMW.
    match mnem.bytes().last() {
        Some(b'b') | Some(b'w') | Some(b'l') | Some(b'q') => {
            Some(match mnem.bytes().last().unwrap() {
                b'b' => 1,
                b'w' => 2,
                b'l' => 4,
                _ => 8,
            })
        }
        // Anything else (fstps/fstpt, stmxcsr, sgdt, movnti, rep forms,
        // x87 pops, …): the written width is not the suffix width — fail
        // closed.
        _ => None,
    }
}

pub(super) fn store_cannot_alias_slot(cached: Option<FrameSlot>, t: &str) -> bool {
    let Some(slot) = cached else {
        return false;
    };
    let Some(dst) = store_destination(t) else {
        return false;
    };
    // The store side needs an EXACT upper bound; an unrecognizable store
    // footprint invalidates the cache instead of being guessed at.
    let Some(width) = bounded_store_mem_width(t) else {
        return false;
    };
    let Some(other) = parse_frame_slot(dst, width) else {
        return false;
    };
    if other.base != slot.base {
        return false;
    }
    other.disp + other.width <= slot.disp || slot.disp + slot.width <= other.disp
}

/// True when frame slot `slot` (byte range from [`parse_frame_slot`]) is
/// STABLE over `infos[lo..=hi]` and its base register cannot move there:
/// no instruction in the range may write an overlapping byte, and nothing
/// may write the base (`%rsp`/`%rbp`).
///
/// Shared by the loop-hoisting passes, whose hand-rolled writer scans each
/// missed a clobber family and miscompiled loop-carried struct copies:
/// `hoist_loop_invariant_gpr_load` only recognised `movq/movl/movb/movw`
/// stores, so a `movdqu %xmm0, 136(%rsp)` inside the loop clobbered the
/// hoisted `movq 136(%rsp), %r11` (torture execute/20030613-1.c at -O1);
/// `promote_loop_invariant_fp_load` only recognised `movsd` stores, so a
/// GPR or XMM store to the same slot broke the promoted `%xmm2` the same
/// way. Both now call this instead of pattern-matching stores by hand.
///
/// Rules, all fail-closed:
/// * any memory write ([`line_writes_memory`]: plain/XMM/RMW stores, string
///   ops, atomics, `xsave`, ...) that cannot prove itself disjoint from the
///   slot ([`store_cannot_alias_slot`]: same base, non-overlapping byte
///   range) clashes — including stores through non-frame pointers, indexed
///   and rip-relative destinations, and unrecognized mnemonics;
/// * any write to the base ([`writes_family`]: `sub`/`add`/`mov`/`leave`,
///   loads into the base, `setCC %spl/%bpl`) clashes — base motion
///   re-points every base-relative address between the hoist point and the
///   load. Push/pop move `%rsp` implicitly (invisible to `writes_family`,
///   which sees only the classified destination plus the tabled implicits)
///   and are refused explicitly for `%rsp`-addressed slots;
/// * an unparseable slot (`None`) clashes against every memory write;
/// * opaque lines (inline asm — [`writes_family`] answers "may write" for
///   them, so the base-motion check below clashes) and
///   unknown-destination instructions (`Other { REG_NONE }`: the
///   implicit-operand table is exact-only, so a MAY model must fall back
///   explicitly) clash;
/// * a call clashes for a red-zone (`disp < 0`) `%rsp` slot — the callee
///   owns those bytes. Ordinary frame slots survive calls (the callee
///   cannot address the caller's frame except through an escaped pointer,
///   which is the CALLER's problem: both current callers exclude calls
///   from the range, GPR directly and FP via its `%xmm2` reservation).
///
/// Control flow needs no check: the scan covers every line of the range,
/// hence every path through it.
pub(super) fn frame_slot_stable_in_range(
    store: &LineStore,
    infos: &[LineInfo],
    lo: usize,
    hi: usize,
    base: RegId,
    slot: Option<FrameSlot>,
) -> bool {
    for chk in lo..=hi {
        if infos[chk].is_nop() {
            continue;
        }
        let t = infos[chk].trimmed(store.get(chk));
        // Unknown-destination instruction: the implicit-operand table yields
        // `(0, 0)` for unknown mnemonics, so the MAY model falls back here.
        if matches!(infos[chk].kind, LineKind::Other { dest_reg } if dest_reg == REG_NONE) {
            return false;
        }
        // Push/pop move %rsp implicitly (invisible to `writes_family`).
        if base == 4
            && matches!(
                infos[chk].kind,
                LineKind::Push { .. } | LineKind::Pop { .. }
            )
        {
            return false;
        }
        if writes_family(&infos[chk], t, base) {
            return false;
        }
        // The callee owns the red zone; ordinary frame slots are call-safe
        // (escaped pointers are the caller's contract — see above).
        if base == 4
            && matches!(infos[chk].kind, LineKind::Call)
            && slot.map_or(true, |s| s.disp < 0)
        {
            return false;
        }
        if line_writes_memory(t) && !store_cannot_alias_slot(slot, t) {
            return false;
        }
    }
    true
}

/// Conservative "this instruction writes memory" test: any instruction whose
/// LAST operand is a memory reference, plus the string/atomic families.
///
/// The destination operand MUST be located with [`last_top_level_comma`], not
/// `rfind(',')`. An AT&T memory operand carries its own commas inside balanced
/// parentheses (`DISP(%base,%index,scale)`), so the last comma of a store to an
/// indexed address is the one separating the SIB base from its index — and the
/// text after it (`%r12)`) contains no `(`. The naive split therefore reported
/// "does not write memory" for EVERY store through an indexed address.
///
/// That is a miscompile, not a lost optimization: this predicate is the guard
/// that stops [`reuse_redundant_loads`] from replacing a load with a copy of an
/// older load of the same address. With the guard blind to indexed stores, the
/// reload after a swap was rewritten to reuse the pre-store value:
///
/// ```asm
///     movzbl (%r10,%r12), %edx      # t = gh.s[i]
///     movzbl (%r10,%r13), %eax      # v = gh.s[j]
///     movb   %al, (%r10,%r12)       # gh.s[i] = v      <- invisible to rfind
///     movb   %dl, (%r10,%r13)       # gh.s[j] = t
///     movzbl (%r10,%r12), %r8d      # reload gh.s[i]   -> became `movl %edx,%r8d`
///     addl   %ebp, %r8d             # t += <stale t> instead of the swapped byte
/// ```
///
/// The bug stayed dormant while every such address was materialized
/// rip-relative (`gh+2(%rip)`), because [`load_operand`] rejects `%rip`
/// operands and so the rewrite never became eligible. Hoisting a shared
/// `GlobalAddr` base into a register — a legitimate, profitable mid-end
/// transform — exposed it. Regression: `pic_indexed_store_static_global.c`.
pub(super) fn line_writes_memory(t: &str) -> bool {
    if t.starts_with("lock") || t.starts_with("rep") || t.starts_with("movs") && !t.contains('%') {
        return true;
    }
    let Some(comma) = last_top_level_comma(t.as_bytes()) else {
        // Single-operand forms that write memory (`incl (%rax)`, `negq (%rax)`).
        return t.contains('(') && !t.starts_with("lea") && !t.starts_with("j");
    };
    let dst = t[comma + 1..].trim();
    dst.contains('(') && !t.starts_with("lea")
}

// ── load + self-test → compare against memory ────────────────────────────────

/// `movsbq (%rbx), %rsi; testq %rsi, %rsi; je` → `cmpb $0, (%rbx); je`.
///
/// The loaded value is only used to be compared against zero, so the load can
/// disappear into the compare. Flag-for-flag identical: `cmp $0, mem` and
/// `test %r, %r` over the same value both clear CF/OF and set ZF/SF/PF from
/// it, and the sign of a sign-extended byte is the sign of the byte.
///
/// This needs the register to be dead after the test, ACROSS the branch that
/// follows — which is exactly what the block-local scans could never prove and
/// why the transform was rejected in an earlier session. [`FileLiveness`]
/// answers it directly.
pub(super) fn fold_load_test_into_cmp(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    const SIGNED: &[(&str, &str)] = &[
        ("movsbq ", "cmpb"),
        ("movsbl ", "cmpb"),
        ("movswq ", "cmpw"),
        ("movswl ", "cmpw"),
        ("movslq ", "cmpl"),
    ];
    const UNSIGNED: &[(&str, &str)] = &[
        ("movzbq ", "cmpb"),
        ("movzbl ", "cmpb"),
        ("movzwq ", "cmpw"),
        ("movzwl ", "cmpw"),
    ];
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let load = infos[i].trimmed(store.get(i)).to_string();
        let entry = SIGNED
            .iter()
            .chain(UNSIGNED.iter())
            .find(|(p, _)| load.starts_with(*p));
        let Some((prefix, cmp_mnemonic)) = entry else {
            i += 1;
            continue;
        };
        let Some((mem, dst)) = split_two_operands(&load[prefix.len()..]) else {
            i += 1;
            continue;
        };
        if !mem.contains('(') || mem.contains("%rip") || !is_register_operand(dst) {
            i += 1;
            continue;
        }
        let Some(fam) = plain_gp_operand(dst) else {
            i += 1;
            continue;
        };
        // The very next instruction must be the self-test of that register.
        let mut j = i + 1;
        while j < len && (infos[j].is_nop() || infos[j].kind == LineKind::Directive) {
            j += 1;
        }
        if j >= len || infos[j].pinned {
            i += 1;
            continue;
        }
        let test = infos[j].trimmed(store.get(j)).to_string();
        let Some(args) = test
            .strip_prefix("testq ")
            .or_else(|| test.strip_prefix("testl "))
        else {
            i += 1;
            continue;
        };
        let Some((a, b)) = split_two_operands(args) else {
            i += 1;
            continue;
        };
        if a != b || register_family_fast(a) != fam {
            i += 1;
            continue;
        }
        // SF of `cmpb`/`cmpw` is the memory operand's sign bit. That matches
        // `testq`/`testl` of a sign-extended-to-64 load (`movsbq`/`movswq`/
        // `movslq`) and `testl` of a sign-extended-to-32 load (`movsbl`/
        // `movswl`). It does NOT match:
        //   * any zero-extending load (byte 0x80: test SF=0, cmpb SF=1);
        //   * `movsbl`/`movswl` + `testq` (the 32-bit write zero-extends,
        //     so testq SF is bit 63 = 0 for a negative byte).
        // ZF, PF, CF and OF are identical in every case and both forms leave AF
        // undefined, so SF is the ONLY flag this rewrite can change. The
        // consumers that matter are therefore exactly the ones that read SF --
        // a narrower question than "is every consumer ZF-only", which keeps the
        // fold alive under a downstream `jc`, `jo`, `jp` or `adc`. The walk
        // covers the taken edges of conditional jumps too, where a linear scan
        // would stop at the first writer on the fall-through path.
        let signed_64 = matches!(*prefix, "movsbq " | "movswq " | "movslq ");
        let signed_32 = matches!(*prefix, "movsbl " | "movswl ");
        let test_is_q = test.starts_with("testq ");
        if !(signed_64 || (signed_32 && !test_is_q))
            && super::flag_peepholes::flags_reach_an_sf_consumer(store, infos, j + 1)
        {
            i += 1;
            continue;
        }
        // The loaded value must be dead after the test — including along the
        // branch that consumes the flags.
        if lv.live_after(j, fam) != Some(false) {
            i += 1;
            continue;
        }
        let new_line = format!("    {} $0, {}", cmp_mnemonic, mem);
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[j], j, new_line);
        lv.refresh_at(store, infos, j);
        changed = true;
        i = j + 1;
    }
    changed
}

// ── accumulator round-trip elimination ───────────────────────────────────────

/// Read-modify-write ops whose destination is both source and result.
const RMW_OPS: &[&str] = &[
    "addq ", "addl ", "subq ", "subl ", "andq ", "andl ", "orq ", "orl ", "xorq ", "xorl ",
    "imulq ", "imull ", "shlq ", "shll ", "shrq ", "shrl ", "sarq ", "sarl ",
];

/// ```text
///     movq %r8, %rax          xorq %r10, %r8
///     xorq %r10, %rax    ->
///     movq %rax, %r8
/// ```
///
/// The accumulator is copied into a scratch register, updated, and copied back.
/// Applying the operation in place removes both copies. Sound when the scratch
/// register is dead after the copy-back, the accumulator is not read between
/// the copies, and no source operand of the RMW op mentions the accumulator
/// (`xorq %r8, %rax` with `%r8` as the accumulator would become `xorq %r8,
/// %r8`). Source operands that mention the SCRATCH register — including
/// registers embedded in compound sources like `imulq $imm, %rax` — are
/// rewritten to the accumulator, which carries the same value once the
/// staging copy is gone.
pub(super) fn fold_accumulator_roundtrip(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let first = infos[i].trimmed(store.get(i)).to_string();
        let Some(rest) = first.strip_prefix("movq ") else {
            i += 1;
            continue;
        };
        let Some((acc, tmp)) = split_two_operands(rest) else {
            i += 1;
            continue;
        };
        let (Some(acc_fam), Some(tmp_fam)) = (plain_gp_operand(acc), plain_gp_operand(tmp)) else {
            i += 1;
            continue;
        };
        if acc_fam == tmp_fam || !is_relayable_family(acc_fam) || !is_relayable_family(tmp_fam) {
            i += 1;
            continue;
        }
        let next_real = |from: usize| -> Option<usize> {
            let mut k = from;
            while k < len {
                if !infos[k].is_nop() && infos[k].kind != LineKind::Directive {
                    return Some(k);
                }
                k += 1;
            }
            None
        };
        let (Some(j), _) = (next_real(i + 1), ()) else {
            i += 1;
            continue;
        };
        let op_line = infos[j].trimmed(store.get(j)).to_string();
        let Some(op) = RMW_OPS.iter().find(|p| op_line.starts_with(**p)) else {
            i += 1;
            continue;
        };
        let Some((op_src, op_dst)) = split_two_operands(&op_line[op.len()..]) else {
            i += 1;
            continue;
        };
        if op_dst != tmp || op_src.contains('(') {
            i += 1;
            continue;
        }
        // Scan EVERY operand token on the op's source side. Compound sources
        // such as the 3-operand `imulq $imm, %rax` embed a register after the
        // immediate; parsing only the whole source string as a bare register
        // let those forms slip past the alias check, and the fold then
        // deleted the staging `movq` while the op still read the scratch
        // register — a stale-register miscompile (found via the dot8
        // magic-division loop: `imulq $magic, %rax, %rsi` reading a dead
        // `%rax` instead of the accumulator `%rsi`).
        //
        // * Source mentions the ACCUMULATOR anywhere -> reject. The in-place
        //   form would alias source and destination (historical behaviour
        //   for the bare-register case, kept conservatively).
        // * Source mentions the SCRATCH register -> rewrite that token to
        //   the accumulator: after the fold the staging copy is gone and the
        //   accumulator carries exactly the value the scratch held. Only the
        //   exact scratch text is rewritten (width-exact, mirroring the
        //   move-relay pass); any width-variant alias rejects instead.
        let mut src_tokens: Vec<&str> = op_src.split(',').map(str::trim).collect();
        let mut reject = false;
        for tok in src_tokens.iter_mut() {
            if let Some(src_fam) = plain_gp_operand(tok) {
                if src_fam == acc_fam {
                    reject = true;
                    break;
                }
                if src_fam == tmp_fam {
                    if *tok != tmp {
                        reject = true; // width-variant alias: not text-safe
                        break;
                    }
                    *tok = acc; // scratch carried the accumulator's value
                }
            }
        }
        if reject {
            i += 1;
            continue;
        }
        let Some(k) = next_real(j + 1) else {
            i += 1;
            continue;
        };
        let back = infos[k].trimmed(store.get(k)).to_string();
        if infos[j].pinned || infos[k].pinned {
            i += 1;
            continue;
        }
        let Some(brest) = back.strip_prefix("movq ") else {
            i += 1;
            continue;
        };
        if split_two_operands(brest) != Some((tmp, acc)) {
            i += 1;
            continue;
        }
        // The scratch register must die at the copy-back, and nothing may read
        // the accumulator in between (the op sees the copied value only).
        if lv.live_after(k, tmp_fam) != Some(false) {
            i += 1;
            continue;
        }
        let new_src = src_tokens.join(", ");
        let new_line = format!("    {}{}, {}", op, new_src, acc);
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[j], j, new_line);
        mark_nop(&mut infos[k]);
        lv.refresh_at(store, infos, j);
        changed = true;
        i = k + 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::super::super::peephole_optimize;
    use super::*;

    fn run(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    /// Run ONLY `reuse_redundant_loads` and return the surviving lines, so an
    /// assertion is about this pass rather than about whatever the rest of the
    /// pipeline would do to the same input.
    fn run_reuse(asm: &str, policy: ReusePolicy) -> String {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        reuse_redundant_loads_with(&mut store, &mut infos, policy);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn frame(body: &str) -> String {
        format!("f:\n.cfi_startproc\n{body}    ret\n.cfi_endproc\n")
    }

    fn slot_refs(out: &str, slot: &str) -> usize {
        out.lines().filter(|l| l.contains(slot)).count()
    }

    /// A reload of the same operand into the SAME register is pure waste: it
    /// writes the destination the value it already holds. This is the shape the
    /// pass used to miss, because the destination-write break below fired on
    /// exactly that line before the match was ever attempted.
    #[test]
    fn same_destination_reload_is_deleted() {
        let out = run_reuse(
            &frame("    movq 360(%rsp), %rax\n    movl %edx, %esi\n    movq 360(%rsp), %rax\n"),
            ReusePolicy::full(),
        );
        assert_eq!(
            slot_refs(&out, "360(%rsp)"),
            1,
            "second same-dst reload must go:\n{out}"
        );
    }

    /// Same input under the legacy policy: the reload must survive, which is
    /// what proves the deletion above is the new precision and not something
    /// the pass already did.
    #[test]
    fn same_destination_reload_survives_under_legacy_policy() {
        let out = run_reuse(
            &frame("    movq 360(%rsp), %rax\n    movl %edx, %esi\n    movq 360(%rsp), %rax\n"),
            ReusePolicy::legacy(),
        );
        assert_eq!(
            slot_refs(&out, "360(%rsp)"),
            2,
            "legacy arm must keep both:\n{out}"
        );
    }

    /// The headline precision: a spill to a DIFFERENT frame slot is not an
    /// aliasing hazard for a cached reload. Slot ranges in a static frame are
    /// a fact, not a points-to guess.
    #[test]
    fn store_to_a_different_frame_slot_does_not_break_reuse() {
        let body = "    movq 360(%rsp), %rax\n    movl %edx, 24(%rsp)\n    movq 360(%rsp), %rcx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert!(
            out.contains("movq %rax, %rcx"),
            "must forward from the register:\n{out}"
        );
        assert_eq!(
            slot_refs(&out, "360(%rsp)"),
            1,
            "only one slot read:\n{out}"
        );
    }

    /// The same input under the legacy policy breaks on the intervening store.
    #[test]
    fn store_to_a_different_frame_slot_breaks_reuse_under_legacy_policy() {
        let body = "    movq 360(%rsp), %rax\n    movl %edx, 24(%rsp)\n    movq 360(%rsp), %rcx\n";
        let out = run_reuse(&frame(body), ReusePolicy::legacy());
        assert!(
            !out.contains("movq %rax, %rcx"),
            "legacy arm must not forward:\n{out}"
        );
        assert_eq!(
            slot_refs(&out, "360(%rsp)"),
            2,
            "legacy arm keeps both reads:\n{out}"
        );
    }

    /// A dword store at 364 overlaps the cached qword at [360,368).
    #[test]
    fn overlapping_frame_slot_store_breaks_reuse() {
        let body = "    movq 360(%rsp), %rax\n    movl %edx, 364(%rsp)\n    movq 360(%rsp), %rcx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "360(%rsp)"),
            2,
            "overlapping store must break reuse:\n{out}"
        );
    }

    /// State-save instructions write hundreds of implementation-sized bytes
    /// at their frame operand (`fxsave64` = 512, `xsave64` up to 4 KiB).
    /// Charging them the 16-byte "unknown mnemonic" width would falsely
    /// prove them disjoint from nearby cached slots — a miscompile.
    #[test]
    fn bulk_state_saves_always_break_reuse() {
        for (op, dst) in [
            ("fxsave64", "32(%rsp)"),
            ("fxsave", "32(%rsp)"),
            ("xsave64", "(%rsp)"),
            ("xsaveopt64", "(%rsp)"),
            ("xrstor64", "(%rsp)"), // covered structurally like the saves
            ("fnsave", "32(%rsp)"),
            ("fnstenv", "32(%rsp)"),
            ("fbstpb", "32(%rsp)"),
        ] {
            let body =
                format!("    movq 48(%rsp), %rax\n    {op} {dst}\n    movq 48(%rsp), %rax\n");
            let out = run_reuse(&frame(&body), ReusePolicy::full());
            assert_eq!(
                slot_refs(&out, "48(%rsp)"),
                2,
                "{op} must invalidate a cached load it may overwrite:\n{out}"
            );
        }
    }

    /// `movdir64b` writes 64 direct-store bytes and `cmpxchg16b` 16; their
    /// suffix letter must not be read as a width (…b means byte elsewhere).
    #[test]
    fn oversized_special_stores_break_reuse_within_their_footprint() {
        for (op, dst, cached) in [
            ("movdir64b", "16(%rsp)", "48(%rsp)"),  // reaches [16,80)
            ("cmpxchg16b", "32(%rsp)", "40(%rsp)"), // reaches [32,48)
        ] {
            let body =
                format!("    movq {cached}, %rax\n    {op} {dst}\n    movq {cached}, %rax\n");
            let out = run_reuse(&frame(&body), ReusePolicy::full());
            assert_eq!(
                slot_refs(&out, cached),
                2,
                "{op} must invalidate a cached load inside its footprint:\n{out}"
            );
        }
        // A cache strictly beyond movdir64b's 64 bytes stays reusable.
        let body = "    movq 88(%rsp), %rax\n    movdir64b 16(%rsp)\n    movq 88(%rsp), %rax\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "88(%rsp)"),
            1,
            "a store proven out of reach must not invalidate:\n{out}"
        );
    }

    /// A same-destination reload that the stack-load→ALU memory-operand fold
    /// is about to consume (`movq slot,%rcx; addq %rcx,%rax` ⇒ `addq
    /// slot,%rax`) must survive the reuse pass. Deleting it makes the later
    /// consumer read the surviving earlier load, and the fold's register
    /// deadness proof then fails for that earlier load — so the whole reload
    /// chain stays materialized (one extra instruction per chain, the
    /// -O0 loop-carried reload regression).
    #[test]
    fn reload_feeding_memfold_consumer_is_preserved() {
        let body = "    movq -16(%rbp), %rcx\n    addq %rcx, %rax\n    movq -16(%rbp), %rcx\n    addq %rcx, %rax\n    movq -8(%rbp), %rcx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "-16(%rbp)"),
            2,
            "both fold-feeding reloads must survive reuse:\n{out}"
        );
    }

    /// Near-miss control: a reload whose following instruction is NOT a
    /// foldable ALU consumer is still deleted as before.
    #[test]
    fn reload_without_memfold_consumer_still_deleted() {
        let body = "    movq -16(%rbp), %rcx\n    movl %edx, %esi\n    movq -16(%rbp), %rcx\n    movq %rcx, %rdi\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "-16(%rbp)"),
            1,
            "only the first load must remain:\n{out}"
        );
    }

    /// x87 store suffixes lie relative to the GP convention: `fstpl` writes
    /// an 8-byte double despite the final `l`, `fistpll` an 8-byte
    /// long-long. Verify the adjacent-slot cache is invalidated.
    #[test]
    fn x87_store_widths_are_charged_correctly() {
        for (op, dst, cached, kept) in [
            ("fstpl", "32(%rsp)", "40(%rsp)", 1), // [32,40) vs [40,48): disjoint
            ("fstpl", "32(%rsp)", "36(%rsp)", 2),
            ("fstpt", "32(%rsp)", "40(%rsp)", 2), // 10 bytes [32,42)
            ("fistpll", "32(%rsp)", "36(%rsp)", 2),
            ("fstpl", "40(%rsp)", "32(%rsp)", 1), // disjoint, cache survives
        ] {
            let body =
                format!("    movq {cached}, %rax\n    {op} {dst}\n    movq {cached}, %rax\n");
            let out = run_reuse(&frame(&body), ReusePolicy::full());
            assert_eq!(
                slot_refs(&out, cached),
                kept,
                "{op} {dst} vs cached {cached}:\n{out}"
            );
        }
    }

    /// A store through a register makes no claim at all: without points-to
    /// analysis it may write anywhere, including the cached slot.
    #[test]
    fn indirect_store_breaks_reuse() {
        let body = "    movq 360(%rsp), %rax\n    movl %edx, (%rbx)\n    movq 360(%rsp), %rcx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "360(%rsp)"),
            2,
            "indirect store must break reuse:\n{out}"
        );
    }

    /// `%rsp` and `%rbp` are not provably a fixed distance apart in this scan,
    /// so a store to an `%rbp` slot must not be reasoned about.
    #[test]
    fn store_through_a_different_frame_base_breaks_reuse() {
        let body = "    movq 360(%rsp), %rax\n    movl %edx, 24(%rbp)\n    movq 360(%rsp), %rcx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "360(%rsp)"),
            2,
            "different frame base must break reuse:\n{out}"
        );
    }

    /// Moving the stack pointer renames every displacement, so the offsets
    /// compared by the disjointness test would no longer name the same bytes.
    #[test]
    fn stack_pointer_movement_breaks_reuse() {
        let body = "    movq 360(%rsp), %rax\n    subq $8, %rsp\n    movq 360(%rsp), %rcx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "360(%rsp)"),
            2,
            "an %rsp write must break reuse:\n{out}"
        );
    }

    /// `vmovdqu %ymm0, 24(%rsp)` writes 32 bytes, i.e. [24,56), which reaches
    /// the cached qword at 48. The mnemonic's suffix position holds a `v`-class
    /// letter, so the suffix rule alone charges 16 bytes and would report the
    /// two as disjoint. This test is what keeps the vector-register half of
    /// `mnemonic_mem_width` from being "simplified" away.
    #[test]
    fn wide_vector_store_reaches_a_distant_slot() {
        let body =
            "    movq 48(%rsp), %rax\n    vmovdqu %ymm0, 24(%rsp)\n    movq 48(%rsp), %rcx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "48(%rsp)"),
            2,
            "a 32-byte store must break reuse of slot 48:\n{out}"
        );
    }

    /// AVX-512: `%zmm` reaches 64 bytes.
    #[test]
    fn zmm_store_reaches_even_further() {
        let body =
            "    movq 80(%rsp), %rax\n    vmovdqu64 %zmm0, 24(%rsp)\n    movq 80(%rsp), %rcx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "80(%rsp)"),
            2,
            "a 64-byte store must break reuse of slot 80:\n{out}"
        );
    }

    /// A narrow store genuinely outside the cached range still permits reuse,
    /// so the vector rule is not simply "any vector register blocks everything".
    #[test]
    fn narrow_vector_store_clear_of_the_slot_permits_reuse() {
        let body =
            "    movq 360(%rsp), %rax\n    movdqu %xmm0, 24(%rsp)\n    movq 360(%rsp), %rcx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert!(
            out.contains("movq %rax, %rcx"),
            "a 16-byte store at 24 cannot touch slot 360:\n{out}"
        );
    }

    #[test]
    fn load_and_self_test_become_a_memory_compare() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            ".LBB1:\n",
            "    movsbq (%rbx), %rsi\n",
            "    testq %rsi, %rsi\n",
            "    je .LBB3\n",
            "    leaq 1(%rbx), %rbx\n",
            "    jmp .LBB1\n",
            ".LBB3:\n",
            "    movq %rbx, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("cmpb $0, (%rbx)"), "{out}");
        assert!(!out.contains("movsbq"), "{out}");
    }

    #[test]
    fn load_and_self_test_are_kept_when_the_value_is_used() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movsbq (%rbx), %rsi\n",
            "    testq %rsi, %rsi\n",
            "    je .LBB3\n",
            "    movq %rsi, %rax\n",
            "    ret\n",
            ".LBB3:\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movsbq (%rbx), %rsi"), "{out}");
    }

    #[test]
    fn unsigned_load_test_is_kept_for_a_sign_consumer() {
        // Byte 0x80: `testq` after `movzbl` has SF=0 (zero-extended);
        // `cmpb $0` has SF=1. A signed consumer must keep the load+test.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl (%rbx), %esi\n",
            "    testq %rsi, %rsi\n",
            "    js .LBB3\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB3:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movzbl (%rbx), %esi"), "{out}");
        assert!(out.contains("testq %rsi, %rsi"), "{out}");
        assert!(!out.contains("cmpb $0"), "{out}");
    }

    #[test]
    fn unsigned_load_test_still_folds_for_a_zero_consumer() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl (%rbx), %esi\n",
            "    testq %rsi, %rsi\n",
            "    je .LBB3\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB3:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("cmpb $0, (%rbx)"), "{out}");
        assert!(!out.contains("movzbl"), "{out}");
    }

    #[test]
    fn unsigned_load_test_folds_for_a_carry_consumer() {
        // SF is the only flag this rewrite can change, so a CF-only consumer is
        // not a reason to refuse: `testq` and `cmpb $0` both clear CF. The
        // older guard asked for ZF-only consumers and lost this fold.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl (%rbx), %esi\n",
            "    testq %rsi, %rsi\n",
            "    jc .LBB3\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB3:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("cmpb $0, (%rbx)"), "{out}");
        assert!(!out.contains("movzbl"), "{out}");
    }

    #[test]
    fn unsigned_load_test_is_kept_when_the_sign_consumer_is_on_the_taken_edge() {
        // The fall-through path kills the flags at `addl`, so a linear scan
        // stopped there having seen only the ZF-only `je` and folded. But the
        // TAKEN edge of that `je` carries the very same flags into .LBB3, where
        // `js` reads SF -- bit 7 of the byte for `cmpb $0`, bit 63 (always 0) of
        // the zero-extended register for `testq`. Walking the edge refuses it.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl (%rbx), %esi\n",
            "    testq %rsi, %rsi\n",
            "    je .LBB3\n",
            "    addl $1, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
            ".LBB3:\n",
            "    js .LBB4\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB4:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movzbl (%rbx), %esi"), "{out}");
        assert!(!out.contains("cmpb $0"), "{out}");
    }

    #[test]
    fn accumulator_roundtrip_is_applied_in_place() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            ".LBB2:\n",
            "    movq %r8, %rax\n",
            "    xorq %r10, %rax\n",
            "    movq %rax, %r8\n",
            "    addl $1, %ebx\n",
            "    jmp .LBB2\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("xorq %r10, %r8"), "{out}");
        assert!(!out.contains("movq %rax, %r8"), "{out}");
    }

    #[test]
    fn accumulator_roundtrip_is_kept_when_the_scratch_survives() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r8, %rax\n",
            "    xorq %r10, %rax\n",
            "    movq %rax, %r8\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        // %rax is the return value: the round-trip must stay.
        assert!(out.contains("%rax"), "{out}");
    }

    #[test]
    fn sib_destination_is_memory_not_a_fake_index_register() {
        // Reduced from gcc.c-torture/execute/20090207-1.c after base-index
        // folding. A raw last-comma split saw `%r8)` as the destination and
        // deleted this observable store as a dead pure register write.
        //
        // The base-index LEA fold now rewrites the pair first into the
        // canonical SIB store `movl $2, 8(%rsp, %r8)` — the SAME address
        // (rsp + r8 + 8) with the LEA folded away — so accept either the
        // folded or the pre-fold form: the invariant under guard is that the
        // observable SIB-destination store SURVIVES the dead-write pass.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 8(%rsp), %rcx\n",
            "    movl $2, (%rcx, %r8)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        let store_survives =
            out.contains("movl $2, 8(%rsp, %r8)") || out.contains("movl $2, (%rcx, %r8)");
        assert!(store_survives, "observable SIB store deleted: {out}");
    }

    #[test]
    fn leftover_staging_movq_before_popcnt_is_deleted() {
        // Acc-path unary used to emit `movq %rdi, %rax; popcntl %edi, %eax`.
        // popcntl writes eax without reading it, so the staging movq is dead.
        let out = run(concat!(
            "popcount32:\n",
            ".cfi_startproc\n",
            "    movq %rdi, %rax\n",
            "    popcntl %edi, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movq %rdi, %rax"), "{out}");
        assert!(out.contains("popcntl %edi, %eax"), "{out}");
    }

    #[test]
    fn leftover_staging_movq_before_lzcnt_is_deleted() {
        let out = run(concat!(
            "clz32:\n",
            ".cfi_startproc\n",
            "    movq %rdi, %rax\n",
            "    lzcntl %edi, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movq %rdi, %rax"), "{out}");
        assert!(out.contains("lzcntl %edi, %eax"), "{out}");
    }

    #[test]
    fn dead_scaled_lea_is_deleted() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            ".LBB2:\n",
            "    leaq 0(,%rbx,4), %rdi\n",
            "    movl (%rsi,%rbx,4), %eax\n",
            "    bswapl %eax\n",
            "    movl %eax, (%rsi,%rbx,4)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("leaq 0(,%rbx,4)"), "{out}");
        assert!(out.contains("bswapl %eax"), "{out}");
    }

    #[test]
    fn live_lea_is_kept() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 0(,%rbx,4), %rdi\n",
            "    movl (%rsi,%rdi), %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 0(,%rbx,4), %rdi"), "{out}");
    }

    #[test]
    fn dead_load_is_kept_because_it_may_fault() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl (%rsi), %eax\n",
            "    movl $7, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl (%rsi), %eax"), "{out}");
    }

    #[test]
    fn repeated_load_becomes_a_copy() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl (%rsi), %edx\n",
            "    movl (%rdi), %r8d\n",
            "    cmpl %r8d, %edx\n",
            "    jle .LBB2\n",
            "    movl (%rsi), %eax\n",
            "    movl %eax, (%rdi)\n",
            "    ret\n",
            ".LBB2:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl (%rsi), %eax"), "{out}");
    }

    fn fallthrough_predicate(asm: &str, label: &str) -> bool {
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let idx = (0..store.len())
            .position(|i| infos[i].trimmed(store.get(i)) == label)
            .expect("label present");
        label_is_fallthrough_only(&store, &infos, idx)
    }

    /// A plain guard label nobody branches to is fallthrough-only.
    #[test]
    fn fallthrough_label_without_predecessors_is_untargeted() {
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq 200(%rsp), %rax\n",
            "    cmpq %rax, %rdi\n",
            ".Lg:\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        assert!(fallthrough_predicate(asm, ".Lg:"));
    }

    /// Jump-table case labels are reached through an indirect jump whose
    /// targets exist only as `.long/.quad` data references: the predicate
    /// must see those edges (regression: only jump mnemonics were scanned).
    #[test]
    fn jump_table_data_references_make_label_targeted() {
        // PC-relative .long entries (LCCC's switch lowering).
        let asm = concat!(
            "    jmpq *%rdx\n",
            ".section .rodata\n",
            ".Ljt_0:\n",
            "    .long .Lc4 - .Ljt_0\n",
            "    .long .Lc5 - .Ljt_0\n",
            ".section .text,\"ax\",@progbits\n",
            ".Lc4:\n",
            "    ret\n",
            ".Lc5:\n",
            "    ret\n",
        );
        assert!(!fallthrough_predicate(asm, ".Lc4:"));
        assert!(!fallthrough_predicate(asm, ".Lc5:"));

        // Absolute .quad entries.
        let asm2 = concat!(
            "    jmpq *%rdx\n",
            "    .quad .Ld7\n",
            ".Ld7:\n",
            "    ret\n",
        );
        assert!(!fallthrough_predicate(asm2, ".Ld7:"));

        // Token boundaries: a reference to .Lc40 must not flag .Lc4.
        let asm3 = concat!(
            "    jmp .Lc40\n",
            ".Lc4:\n",
            "    ret\n",
            ".Lc40:\n",
            "    ret\n",
        );
        assert!(fallthrough_predicate(asm3, ".Lc4:"));
        assert!(!fallthrough_predicate(asm3, ".Lc40:"));
    }

    #[test]
    fn repeated_load_crosses_a_fallthrough_only_label() {
        let out = run(concat!(
            "swapmax:\n",
            ".cfi_startproc\n",
            "    movl (%rsi), %edx\n",
            "    movl (%rdi), %r8d\n",
            "    cmpl %r8d, %edx\n",
            "    jle .LBB2\n",
            ".LBB1:\n",
            "    movl (%rsi), %eax\n",
            "    movl %eax, (%rdi)\n",
            "    movl %r8d, (%rsi)\n",
            ".LBB2:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movl (%rsi), %eax"), "{out}");
    }

    #[test]
    fn repeated_load_after_a_store_is_kept() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl (%rsi), %edx\n",
            "    movl %ecx, (%rdi)\n",
            "    movl (%rsi), %eax\n",
            "    addl %edx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl (%rsi), %eax"), "{out}");
    }

    #[test]
    fn repeated_load_across_a_join_label_is_kept() {
        // `.LBB7` is a real join point (something branches to it), so a store
        // on the other path may have changed the memory.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl (%rsi), %edx\n",
            "    cmpl $3, %edx\n",
            "    jne .LBB7\n",
            "    movl $0, (%rsi)\n",
            ".LBB7:\n",
            "    movl (%rsi), %eax\n",
            "    addl %edx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl (%rsi), %eax"), "{out}");
    }

    #[test]
    fn repeated_sign_extending_load_uses_movq_to_preserve_upper_bits() {
        // Regression for the v6 miscompile: `movsbq (%rbx), %rdx` followed
        // by `movsbq (%rbx), %rax` was rewritten to `movl %edx, %eax`, which
        // zero-extends the upper 32 bits and destroys the sign extension.
        // For a byte 0x80 the original produced %rax = 0xFFFFFFFFFFFFFF80
        // and the rewrite produced %rax = 0x00000000FFFFFF80 — a miscompile
        // whenever the second load's 64-bit value was consumed.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movsbq (%rbx), %rdx\n",
            "    addq %rdx, %r9\n",
            "    movsbq (%rbx), %rax\n",
            "    addq %rax, %r9\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        // Either the second load is kept (safest), or the substitution
        // uses `movq` to copy the full 64-bit sign-extended value.
        // It MUST NOT become `movl %edx, %eax` (the v6 bug).
        assert!(
            out.contains("movsbq (%rbx), %rax") || out.contains("movq %rdx, %rax"),
            "{out}"
        );
        assert!(!out.contains("movl %edx, %eax"), "{out}");
    }

    #[test]
    fn repeated_movslq_load_uses_movq() {
        // Same shape for the 32-bit -> 64-bit sign extension.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movslq (%rbx), %rdx\n",
            "    addq %rdx, %r9\n",
            "    movslq (%rbx), %rax\n",
            "    addq %rax, %r9\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movslq (%rbx), %rax") || out.contains("movq %rdx, %rax"),
            "{out}"
        );
        assert!(!out.contains("movl %edx, %eax"), "{out}");
    }

    #[test]
    fn repeated_movzbl_load_still_uses_movl() {
        // Zero-extending loads to 32 bits keep using `movl` — the upper
        // 32 bits of the family are zero in both the load and the copy.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl (%rbx), %edx\n",
            "    addl %edx, %r9d\n",
            "    movzbl (%rbx), %eax\n",
            "    addl %eax, %r9d\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movl %edx, %eax"), "{out}");
    }

    // ── accumulator round-trip: compound-source alias guards ──────────────
    //
    // Regression for the dot8 magic-division miscompile: an earlier pass
    // folded the staging copy into the imul source (`imulq $imm, %rax, %rax`
    // after `movq %rsi, %rax`), then the round-trip fold deleted the staging
    // copy while rewriting only the DESTINATION — emitting
    // `imulq $imm, %rax, %rsi` reading the now-stale scratch register.

    #[test]
    fn acc_roundtrip_rewrites_compound_src_mentioning_scratch() {
        // Direct drive of the pass (end-to-end runs may legitimately
        // pre-rewrite the staging copy elsewhere): the 3-operand imul names
        // the scratch register in its compound source; the fold must move
        // that mention onto the accumulator together with the destination.
        let asm = concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movslq %edi, %rsi\n",
            "    movq %rsi, %r11\n",
            "    imulq $1717986919, %r11, %r11\n",
            "    movq %r11, %rsi\n",
            "    sarq $33, %rsi\n",
            "    movq %rsi, (%rdx)\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(fold_accumulator_roundtrip(&mut store, &mut infos));
        let out = store.build_result(|i| infos[i].is_nop());
        assert!(
            out.contains("imulq $1717986919, %rsi, %rsi"),
            "expected in-place fold with rewritten src: {out}"
        );
        assert!(
            !out.contains("imulq $1717986919, %r11, %rsi"),
            "stale-scratch miscompile: {out}"
        );
        // The staging copies are gone.
        assert!(!out.contains("movq %rsi, %r11"), "{out}");
        assert!(!out.contains("movq %r11, %rsi"), "{out}");
    }

    #[test]
    fn acc_roundtrip_never_reads_stale_scratch_when_src_mentions_acc() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movslq %edi, %rsi\n",
            "    movq %rsi, %rax\n",
            "    imulq $1717986919, %rsi, %rax\n",
            "    movq %rax, %rsi\n",
            "    sarq $33, %rsi\n",
            "    movq %rsi, (%rdx)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        // Whatever combination of folds fires, the imul must never end up
        // reading %rax as its source with %rsi as destination: %rax does not
        // hold the multiplicand once the staging copy is deleted.
        assert!(
            !out.contains("imulq $1717986919, %rax, %rsi"),
            "stale-scratch miscompile: {out}"
        );
    }

    // ── indexed-address store detection ─────────────────────────────────────
    //
    // `rfind(',')` locates the SIB comma inside `(%base,%index)`, so the
    // "does this line write memory?" guard answered FALSE for every store
    // through an indexed address. That let `reuse_redundant_loads` replace a
    // post-store reload with a copy of the pre-store value — the miscompile
    // behind tests/regression/pic_indexed_store_static_global.c.

    #[test]
    fn line_writes_memory_detects_indexed_stores() {
        // The regression itself: the LAST comma is inside the SIB address.
        assert!(line_writes_memory("movb %al, (%r10,%r12)"));
        assert!(line_writes_memory("movq %rax, (%r10,%r12,8)"));
        assert!(line_writes_memory("movl %eax, 16(%rbx,%rcx,4)"));
        assert!(line_writes_memory("movw %ax, -4(%rdi,%rsi)"));
        // Read-modify-write through an indexed address.
        assert!(line_writes_memory("addl %eax, (%rbx,%rcx)"));
        assert!(line_writes_memory("incl (%rbx,%rcx,4)"));
        // Forms that were already classified correctly must stay correct.
        assert!(line_writes_memory("movq %rax, -8(%rbp)"));
        assert!(line_writes_memory("movb %al, (%rdi)"));
        assert!(line_writes_memory("incl (%rax)"));
        assert!(line_writes_memory("lock cmpxchgq %rcx, (%rdx,%r8)"));
        // Loads READ memory: their destination is a register, so the last
        // top-level comma is followed by that register, not by an address.
        assert!(!line_writes_memory("movzbl (%r10,%r12), %edx"));
        assert!(!line_writes_memory("movq 8(%rbx,%rcx,2), %rax"));
        assert!(!line_writes_memory("leaq (%rax,%rcx), %rdx"));
        assert!(!line_writes_memory("movl %eax, %ebx"));
        assert!(!line_writes_memory("cmpq $32, %r8"));
    }

    #[test]
    fn reload_after_indexed_store_is_not_replaced_by_the_stale_load() {
        // Exact shape of the miscompiled s-box swap loop:
        //     t = s[i]; v = s[j]; s[i] = v; s[j] = t; t += s[i];
        // The trailing load of (%r13,%rsi) must survive — the two stores wrote
        // that address, so the pre-store value in %r9d is stale.
        let out = run(concat!(
            "hash_final:\n",
            ".cfi_startproc\n",
            ".LBB1:\n",
            "    movzbl (%r13,%rsi), %r9d\n",
            "    movl %r9d, %ebp\n",
            "    movzbl (%r13,%rdx), %eax\n",
            "    movb %al, (%r13,%rsi)\n",
            "    movb %r9b, (%r13,%rdx)\n",
            "    movzbl (%r13,%rsi), %r10d\n",
            "    movl %ebp, %r8d\n",
            "    addl %r10d, %r8d\n",
            "    addq $1, %r15\n",
            "    cmpq $32, %r15\n",
            "    jb .LBB1\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movzbl (%r13,%rsi), %r10d"),
            "post-store reload was replaced by the stale value: {out}"
        );
        assert!(
            !out.contains("movl %r9d, %r10d"),
            "stale-value copy substituted for the reload: {out}"
        );
    }

    #[test]
    fn reuse_still_fires_when_no_store_intervenes() {
        // The optimization must survive the fix: two loads of the same indexed
        // address with nothing writing memory in between are still folded away
        // (into a register copy, which copy-propagation may then absorb).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movzbl (%r13,%rsi), %r9d\n",
            "    movl %r9d, %ebp\n",
            "    movzbl (%r13,%rsi), %r10d\n",
            "    addl %r10d, %ebp\n",
            "    movl %ebp, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            !out.contains("movzbl (%r13,%rsi), %r10d"),
            "redundant load was not eliminated: {out}"
        );
    }

    /// i686: same-destination reload through `%esp` is deleted, mirroring
    /// `same_destination_reload_is_deleted`.
    #[test]
    fn i686_same_destination_reload_through_esp_is_deleted() {
        let out = run_reuse(
            &frame("    movl 12(%esp), %eax\n    movl %edx, %esi\n    movl 12(%esp), %eax\n"),
            ReusePolicy::full(),
        );
        assert_eq!(
            slot_refs(&out, "12(%esp)"),
            1,
            "second same-dst reload must go:\n{out}"
        );
    }

    /// i686: a store to a disjoint `%esp` slot does not break reuse.
    #[test]
    fn i686_store_to_a_different_esp_slot_does_not_break_reuse() {
        let body = "    movl 12(%esp), %eax\n    movl %edx, 24(%esp)\n    movl 12(%esp), %ecx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert!(
            out.contains("movl %eax, %ecx"),
            "must forward from the register:\n{out}"
        );
        assert_eq!(slot_refs(&out, "12(%esp)"), 1, "only one slot read:\n{out}");
    }

    /// i686: a dword store at 14 overlaps the cached dword at [12,16).
    #[test]
    fn i686_overlapping_esp_store_breaks_reuse() {
        let body = "    movl 12(%esp), %eax\n    movl %edx, 14(%esp)\n    movl 12(%esp), %ecx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "12(%esp)"),
            2,
            "overlap must keep both reads:\n{out}"
        );
    }

    /// i686: `%esp` movement ends the scan. This is the barrier staging
    /// windows rely on: `subl $N, %esp` writes family 4, so no cached slot
    /// survives it.
    #[test]
    fn i686_esp_movement_breaks_reuse() {
        let body = "    movl 12(%esp), %eax\n    subl $16, %esp\n    movl 12(%esp), %ecx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "12(%esp)"),
            2,
            "esp moved: both reads must survive:\n{out}"
        );
    }

    /// i686: push/pop end the scan like any other `%esp` motion.
    #[test]
    fn i686_push_breaks_reuse() {
        let body = "    movl 12(%esp), %eax\n    pushl %ebx\n    movl 12(%esp), %ecx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "12(%esp)"),
            2,
            "push moved esp: both reads must survive:\n{out}"
        );
    }

    /// i686: `%esp` vs `%ebp` are different bases (families 4/5), not a
    /// provable fixed distance apart here — stays conservative.
    #[test]
    fn i686_esp_vs_ebp_bases_stay_conservative() {
        let body = "    movl 12(%esp), %eax\n    movl %edx, 12(%ebp)\n    movl 12(%esp), %ecx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "12(%esp)"),
            2,
            "different base: both reads must survive:\n{out}"
        );
    }

    /// i686: disjoint `%ebp` slots forward, mirroring the `%rsp` case.
    #[test]
    fn i686_store_to_a_different_ebp_slot_does_not_break_reuse() {
        let body = "    movl 12(%ebp), %eax\n    movl %edx, 24(%ebp)\n    movl 12(%ebp), %ecx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert!(
            out.contains("movl %eax, %ecx"),
            "must forward from the register:\n{out}"
        );
        assert_eq!(slot_refs(&out, "12(%ebp)"), 1, "only one slot read:\n{out}");
    }

    /// i686: pushes do NOT disturb `%ebp`-relative scans (ebp is stable
    /// across them) — the break above is scoped to `%esp`-cached slots.
    #[test]
    fn i686_push_does_not_break_ebp_cached_reuse() {
        let body = "    movl 12(%ebp), %eax\n    pushl %ebx\n    movl %edx, 24(%ebp)\n    movl 12(%ebp), %ecx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert!(
            out.contains("movl %eax, %ecx"),
            "ebp scan must survive the push:\n{out}"
        );
    }

    /// i686: `leave` moves both `%esp` and `%ebp` (effects table), so it
    /// ends any frame-slot scan.
    #[test]
    fn i686_leave_breaks_reuse() {
        let body = "    movl 12(%ebp), %eax\n    leave\n    movl 12(%ebp), %ecx\n";
        let out = run_reuse(&frame(body), ReusePolicy::full());
        assert_eq!(
            slot_refs(&out, "12(%ebp)"),
            2,
            "leave moved the base: both reads must survive:\n{out}"
        );
    }

    /// i686 end-to-end through the FULL pipeline: an `%ebp`-cached reload
    /// separated by a disjoint-slot store survives a `pushl` (ebp is stable
    /// across pushes) and only one slot read remains. (The consumer is a
    /// store on purpose: an arithmetic consumer would let a downstream
    /// load-fold reintroduce the memory operand while deleting an
    /// instruction — a legitimate pre-existing pipeline interaction, but
    /// one that hides this pass's firing.) Guards the integration, not
    /// just the pass in isolation.
    #[test]
    fn i686_ebp_reuse_across_push_end_to_end() {
        let out = run(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebp\n",
            "    movl %esp, %ebp\n",
            "    movl 12(%ebp), %eax\n",
            "    pushl %ebx\n",
            "    movl %edx, 24(%ebp)\n",
            "    movl 12(%ebp), %ecx\n",
            "    movl %ecx, 4(%esp)\n",
            "    popl %ebx\n",
            "    popl %ebp\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert_eq!(
            slot_refs(&out, "12(%ebp)"),
            1,
            "full pipeline must eliminate the second slot read:\n{out}"
        );
    }
}
