//! Dead code elimination passes.
//!
//! Three passes that eliminate dead instructions:
//! - `eliminate_dead_reg_moves`: removes reg-to-reg moves where the destination
//!   is overwritten before being read (local, forward scan within a window).
//! - `eliminate_dead_stores`: removes stores to stack slots that are overwritten
//!   before being read (local, bounded window).
//! - `eliminate_never_read_stores`: removes stores to stack slots that are never
//!   read anywhere in the function (global, whole-function analysis).

use super::super::types::*;
use super::fp_liveness::FpLiveness;
use super::helpers::*;

// SOUNDNESS: LCCC compiles with -fomit-frame-pointer, so %rbp is a general
// data register unless a function explicitly establishes a frame pointer
// (`pushq %rbp` + `movq %rsp, %rbp`). When %rbp is NOT the frame pointer, a
// memory operand such as `movl %eax, 28(%rbp)` is a POINTER DEREFERENCE through
// the value in %rbp (e.g. an array/struct access), NOT a stack-slot access.
// The StoreRbp/LoadRbp classification (which also accepts `(%rsp)`) would
// otherwise treat such an access as a stack slot and could wrongly delete a
// live store. These helpers let the dead-store passes stay frame-pointer aware.
//
/// Returns true if `trimmed` contains a memory operand that dereferences `%rbp`
/// (e.g. `movl %eax, 28(%rbp)` or `movl 28(%rbp), %eax` or `(%rbp, %rax, 4)`).
/// A bare register move such as `movq %rax, %rbp` has no parenthesized operand,
/// so it returns false.
fn uses_rbp_mem_operand(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let start = i;
            let mut depth = 1;
            let mut j = i + 1;
            while j < bytes.len() && depth > 0 {
                if bytes[j] == b'(' {
                    depth += 1;
                } else if bytes[j] == b')' {
                    depth -= 1;
                }
                j += 1;
            }
            if trimmed[start..j].contains("%rbp") {
                return true;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    false
}

// ── Dead register move elimination ──────────────────────────────────────────

/// Maximum forward scan window for dead register move detection.
/// Widened from 2 to 8 after fixing also_reads classification (defense-in-depth
/// cross-size register matching at line 112-114). Previously window=4/48 caused
/// miscompilations in sqlite3VdbeExec, but the root cause was missing cross-size
/// register detection in the source operand check.
const DEAD_MOVE_WINDOW: usize = 8;

pub(super) fn eliminate_dead_reg_moves(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();
    let mut i = 0;

    while i < len {
        if infos[i].is_nop() || infos[i].is_barrier() {
            i += 1;
            continue;
        }

        // Check if this is a reg-to-reg movq.
        let dst_reg = match infos[i].kind {
            LineKind::Other { dest_reg } => {
                let trimmed = infos[i].trimmed(store.get(i));
                if parse_reg_to_reg_movq(&infos[i], trimmed).is_some() {
                    dest_reg
                } else {
                    i += 1;
                    continue;
                }
            }
            _ => {
                i += 1;
                continue;
            }
        };

        if dst_reg == REG_NONE || dst_reg > REG_GP_MAX {
            i += 1;
            continue;
        }
        // Don't eliminate moves to %rsp or %rbp.
        if dst_reg == REG_RSP || dst_reg == REG_RBP {
            i += 1;
            continue;
        }
        // Don't eliminate moves to callee-saved registers in the prologue area.
        // The register allocator assigns param values to callee-saved regs (rbx,
        // r12-r15), and the pre-store in emit_store_params copies ABI arg regs
        // to them. The dead-move analysis can't see past function calls (which
        // are barriers), so it incorrectly considers these writes dead when the
        // next visible use is after a call. Callee-saved regs survive calls,
        // so their pre-call writes are never truly dead.
        // Register encoding: rbx=1 (encoded differently in peephole)
        // We check: if the instruction is in the first few lines (prologue area)
        // and writes to a callee-saved register, skip elimination.
        // A simpler and more robust check: if the destination is a callee-saved
        // register (rbx/r12/r13/r14/r15) AND source is an arg register (rdi/rsi/
        // rdx/rcx/r8/r9), this is likely a param pre-store — don't eliminate.
        // Protect param pre-stores: movq from ABI arg regs to callee-saved regs.
        // These save function parameters before calls clobber the arg registers.
        // The dead-move scanner can't see past calls (barriers), so it incorrectly
        // considers these writes dead. Callee-saved regs survive calls.
        // Arg regs: rdi=7, rsi=6, rdx=2, rcx=1, r8=8, r9=9
        // Callee-saved: rbx=3, r12=12, r13=13, r14=14, r15=15
        {
            let trimmed = infos[i].trimmed(store.get(i));
            if let Some((src_id, _dst_id)) = parse_reg_to_reg_movq(&infos[i], trimmed) {
                let src_is_arg = matches!(src_id, 1 | 2 | 6 | 7 | 8 | 9);
                let dst_is_callee = matches!(dst_reg, 3 | 12 | 13 | 14 | 15);
                if src_is_arg && dst_is_callee {
                    i += 1;
                    continue;
                }
            }
        }

        let dst_mask = 1u16 << dst_reg;
        let mut dead = false;

        // Scan forward within the same basic block.  The window counts
        // INSTRUCTIONS: a deleted line or a hidden `-g` row occupies an
        // index but not a place in the window, or the same move would be
        // dead without `-g` and live with it.
        let mut j = i + 1;
        let mut examined = 1usize;
        while j < len && examined < DEAD_MOVE_WINDOW {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }
            examined += 1;

            if infos[j].is_barrier() {
                // A return cannot observe non-result caller-saved registers.
                // This lets copy propagation finish folds such as
                // `movq %rax,%rcx; movsbl (%rcx),%eax` at a leaf return.
                // Keep RAX/RDX (scalar/aggregate results), all callee-saved
                // registers, and every other control-flow boundary conservative.
                if infos[j].kind == LineKind::Ret && matches!(dst_reg, 1 | 6 | 7 | 8 | 9 | 10 | 11)
                {
                    dead = true;
                }
                // At any other basic block boundary, we can't prove the
                // register dead without analyzing every successor path.
                break;
            }

            let trimmed_j = infos[j].trimmed(store.get(j));
            if has_implicit_reg_usage(trimmed_j) {
                break;
            }

            let refs_dst = infos[j].reg_refs & dst_mask != 0;
            // Redefinition includes architectural implicit writes: `cqto`
            // overwrites %rdx, `idivq` rewrites %rax:%rdx — the classified
            // destination alone answers only half of that.  A redefinition
            // that retires a definition must be a FULL-WIDTH write of the
            // family: `movb $1, %al` rewrites only the low byte, and the
            // upper bits of the moved value survive into later reads, so
            // the any-width `writes_family` used here deleted the move and
            // corrupted them (`movq %r9, %rax; movb $1, %al; movq %rax,
            // %rbx` shipped %rbx = garbage-high | 1).
            let writes_dst = writes_family_full(&infos[j], trimmed_j, dst_reg);

            if writes_dst {
                let also_reads = match infos[j].kind {
                    LineKind::LoadRbp { .. } => false,
                    LineKind::Pop { .. } => false,
                    LineKind::SetCC { .. } => false,
                    LineKind::Other { .. } => {
                        if !refs_dst {
                            false
                        } else {
                            let t = infos[j].trimmed(store.get(j));
                            if is_read_modify_write(t) {
                                true
                            } else {
                                // Defense-in-depth: is_read_modify_write uses exact
                                // string matching for LEA source checks, which misses
                                // cross-size register references (e.g., %eax in source
                                // vs %rax in dest). Use REG_NAMES to match all size
                                // variants of the dest register in the source operand.
                                if let Some(comma_pos) = t.rfind(',') {
                                    let src_part = &t[..comma_pos];
                                    REG_NAMES
                                        .iter()
                                        .any(|row| src_part.contains(row[dst_reg as usize]))
                                } else {
                                    // Single-operand instruction that both reads
                                    // and writes (e.g., negq %rax) - conservative
                                    true
                                }
                            }
                        }
                    }
                    _ => refs_dst,
                };

                if also_reads {
                    break;
                } else {
                    dead = true;
                    break;
                }
            }

            if refs_dst {
                break;
            }

            j += 1;
        }

        if dead {
            mark_nop(&mut infos[i]);
            changed = true;
        }

        i += 1;
    }

    changed
}

// ── Dead XMM reg-move elimination ────────────────────────────────────────────

/// Parse a pure XMM→XMM register move: every operand is `%xmmN`, 2 or 3 of
/// them, destination last (AT&T). Returns `(src_first, dst)`. Covers the
/// scalar (`movsd`/`movss`), packed (`movapd`/`movups`/`movdqa`/`movdqu`/
/// `movupd`) and VEX spellings the emitter produces, plus the 3-operand merge
/// forms (`vmovsd %xmmA, %xmmB, %xmmC`) that store forwarding emits. Anything
/// with a memory operand, an immediate, or a GP operand returns `None`.
fn parse_fp_reg_move(t: &str) -> Option<(u32, u32)> {
    let rest = t
        .strip_prefix("vmovsd ")
        .or_else(|| t.strip_prefix("vmovss "))
        .or_else(|| t.strip_prefix("vmovapd "))
        .or_else(|| t.strip_prefix("vmovaps "))
        .or_else(|| t.strip_prefix("vmovupd "))
        .or_else(|| t.strip_prefix("vmovups "))
        .or_else(|| t.strip_prefix("vmovdqa "))
        .or_else(|| t.strip_prefix("vmovdqu "))
        .or_else(|| t.strip_prefix("vmovq "))
        .or_else(|| t.strip_prefix("movsd "))
        .or_else(|| t.strip_prefix("movss "))
        .or_else(|| t.strip_prefix("movapd "))
        .or_else(|| t.strip_prefix("movaps "))
        .or_else(|| t.strip_prefix("movupd "))
        .or_else(|| t.strip_prefix("movups "))
        .or_else(|| t.strip_prefix("movdqa "))
        .or_else(|| t.strip_prefix("movdqu "))
        .or_else(|| t.strip_prefix("movq "))?;
    let mut idxs = [u32::MAX; 3];
    let mut n = 0usize;
    for op in rest.split(',') {
        let op = op.trim_ascii();
        let name = op.strip_prefix('%')?;
        let num = name.strip_prefix("xmm")?.parse::<u32>().ok()?;
        if n == 3 {
            return None;
        }
        idxs[n] = num;
        n += 1;
    }
    if n < 2 {
        return None;
    }
    // "movq %rax, %xmm0" is excluded by the %xmm parse above; "movq %xmm0,
    // %rax" likewise (dst would not parse). Self-moves are included — deleting
    // a copy into a dead destination is safe, self-move or not.
    Some((idxs[0], idxs[n - 1]))
}

/// Dead XMM→XMM move elimination — the FP counterpart of
/// [`eliminate_dead_reg_moves`] (which cannot serve: `reg_refs` is a 16-bit
/// GP mask and its scan is `movq`-shaped). A register-only FP move whose
/// destination is never read before it is next rewritten writes nothing
/// observable; deleting it saves the instruction and the rename pressure.
/// The extract-staging idiom of vector-lane lowering is the hot case:
/// `movapd %xmm15, %xmm0` immediately overwritten by `pshufd $0x0E,
/// %xmm15, %xmm0` — the lane-0 copy is dead the instant it is written, and
/// it sat in the nbody pair loop's inner loop until this pass existed.
pub(super) fn eliminate_dead_fp_moves(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();
    let mut fp: Option<FpLiveness> = None;
    for i in 0..len {
        if infos[i].is_nop() || infos[i].pinned || infos[i].is_barrier() {
            continue;
        }
        if !matches!(infos[i].kind, LineKind::Other { .. }) {
            continue;
        }
        let trimmed = infos[i].trimmed(store.get(i));
        let Some((_src, dst)) = parse_fp_reg_move(trimmed) else {
            continue;
        };
        if fp
            .get_or_insert_with(|| FpLiveness::new(store, infos))
            .reg_move_dead_after(store, infos, i, dst)
        {
            mark_nop(&mut infos[i]);
            changed = true;
        }
    }
    changed
}

/// Typed byte range of a direct stack move, shared by the scalar and vector
/// cases. The caller must still prove that the base names a frame slot.
fn windowed_slot_move(kind: LineKind) -> Option<(bool, i64, i64)> {
    let (is_store, offset, size) = match kind {
        LineKind::StoreRbp { offset, size, .. } | LineKind::StoreXmmRbp { offset, size } => {
            (true, offset, size)
        }
        LineKind::LoadRbp { offset, size, .. } | LineKind::LoadXmmRbp { offset, size } => {
            (false, offset, size)
        }
        _ => return None,
    };
    let start = i64::from(offset);
    Some((is_store, start, start + i64::from(size.byte_size())))
}

fn is_frame_boundary(text: &str) -> bool {
    matches!(
        text.split_ascii_whitespace().next(),
        Some(".cfi_startproc" | ".cfi_endproc")
    )
}

// ── Dead store elimination (local, windowed) ─────────────────────────────────

pub(super) fn eliminate_dead_stores(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    const WINDOW: usize = 64;
    const STACK_REFS: u16 = (1 << REG_RBP) | (1 << REG_RSP);
    let mut changed = false;
    let mut rbp_is_frame = false;
    for i in 0..infos.len() {
        let info = infos[i];
        if info.is_nop() {
            continue;
        }
        let text = info.trimmed(store.get(i));
        match info.kind {
            LineKind::InlineAsm => rbp_is_frame = false,
            LineKind::Directive if info.is_barrier() || is_frame_boundary(text) => {
                rbp_is_frame = false;
            }
            LineKind::Other { dest_reg: REG_RBP } if text == "movq %rsp, %rbp" => {
                rbp_is_frame = true;
            }
            _ if info.reg_refs & (1 << REG_RBP) != 0
                && line_writes_gp_family(&info, text, REG_RBP) =>
            {
                rbp_is_frame = false;
            }
            _ => {}
        }
        let Some((true, start, end)) = windowed_slot_move(info.kind) else {
            continue;
        };
        if info.pinned {
            continue;
        }
        let base = frame_base(text, 0);
        if base == 0 || (base == b'b' && !rbp_is_frame) {
            continue;
        }
        let mut live = 0;
        for j in (i + 1)..infos.len() {
            let next = infos[j];
            let text = next.trimmed(store.get(j));
            // DIRECTIVE-LAW: bails -- function boundaries reset frame identity.
            if next.kind == LineKind::Directive && is_frame_boundary(text) {
                break;
            }
            if next.is_steppable() {
                continue;
            }
            if live == WINDOW || next.is_barrier() || next.has_indirect_mem {
                break;
            }
            live += 1;
            if (next.reg_refs & STACK_REFS != 0
                && (line_writes_gp_family(&next, text, REG_RSP)
                    || (base == b'b' && line_writes_gp_family(&next, text, REG_RBP))))
                || text == "leave"
                || text == "leaveq"
                || text.starts_with("enter ")
            {
                break;
            }
            if let Some((is_store, other_start, other_end)) = windowed_slot_move(next.kind) {
                // Equal displacements are not equal addresses across the two
                // frame bases. Without a relation between the bases, even an
                // access at a different displacement may read this slot.
                if frame_base(text, 0) != base {
                    break;
                }
                if other_start >= end || other_end <= start {
                    continue;
                }
                if is_store && other_start <= start && end <= other_end {
                    mark_nop(&mut infos[i]);
                    changed = true;
                }
                break;
            }
            // Untyped memory accesses have unknown width (a vector ALU input
            // isn't necessarily a MOV). Segment/index/indirect accesses were
            // rejected above; do not infer disjointness from a register name.
            if next.rbp_offset != RBP_OFFSET_NONE
                || (next.reg_refs & STACK_REFS != 0
                    && (text.contains("(%rsp") || text.contains("(%rbp")))
            {
                break;
            }
        }
    }
    changed
}

// ── Global dead store elimination for never-read stack slots ─────────────────

/// Frame base register used by the frame operand in `line`: b'b' for `%rbp`,
/// b's' for `%rsp`. `default` when no frame operand is present.
fn frame_base(line: &str, default: u8) -> u8 {
    if line.contains("(%rbp)") || line.contains("(%rbp,") {
        b'b'
    } else if line.contains("(%rsp)") || line.contains("(%rsp,") {
        b's'
    } else {
        default
    }
}

/// Byte change to the canonical-to-current rsp offset caused by one
/// rsp-shifting instruction. The returned `delta` is the number of bytes the
/// current rsp lies BELOW its canonical (post-prologue) value: a `push` moves
/// rsp down 8 (delta +8), `pop` reverses it (delta -8), `subq $N,%rsp` adds
/// N and `addq $N,%rsp` subtracts N. `None` means the shift cannot be parsed
/// as a constant, so the caller must keep its historical fail-closed rule.
fn rsp_delta_step(t: &str) -> Option<i64> {
    let t = t.trim_ascii();
    if t.starts_with("push") {
        return Some(8);
    }
    if t.starts_with("pop") {
        return Some(-8);
    }
    if let Some(rest) = t.strip_prefix("subq $") {
        return rest
            .strip_suffix(", %rsp")?
            .parse::<i32>()
            .ok()
            .map(i64::from);
    }
    if let Some(rest) = t.strip_prefix("addq $") {
        let n = rest.strip_suffix(", %rsp")?.parse::<i32>().ok()?;
        return Some(-i64::from(n));
    }
    None
}

/// Translate a frame displacement observed while rsp sits `rsp_delta` bytes
/// below its canonical depth back to the canonical displacement that the
/// stored read ranges and Phase-2 store offsets already use. At delta D a
/// textual `disp(%rsp)` addresses canonical byte `disp - D` (e.g. after two
/// pushes, `24(%rsp)` reads canonical slot 8). `%rbp`-relative operands are
/// unaffected because the frame pointer never moves inside a function.
fn canonical_frame_off(base: u8, off: i64, rsp_delta: i64) -> i64 {
    if base == b's' { off - rsp_delta } else { off }
}

/// Effective byte range read by an SIB frame operand
/// `disp(%base,%idx,scale)` when `%idx` is a structurally-recognised loop
/// induction variable, or `None` when its bounds cannot be proven.
///
/// Shifted-displacement SIB operands are common in message-schedule /
/// sliding-window kernels: `movl -24(%rsp,%r10,4),%edx` with `%r10` counting
/// iterations 16..64 reads `[40+4*16-24, 40+4*64-24)` — the textual
/// displacement `-24` is NOT a frame address by itself. A naive
/// "from displacement upward" floor would either pin unrelated low home
/// slots or miss the true (downward) reach. We recognize the codegen shape:
///
/// ```text
/// preheader: movq $A,%rX           (xorl %rXd,%rXd == A=0)
/// header:    cmpq $B,%rX ; j<cc> out
/// body:      <SIB use>; ...; addq $step,%rX; cmpq $B,%rX; jl loop
/// ```
///
/// and return `[disp + A*scale, disp + B*scale + width)`. Any unrecognized
/// WRITE to the index family inside the loop body, or a missing constant
/// init / upper bound, returns `None`; the caller then fails closed.
fn sib_frame_read_range(
    store: &LineStore,
    infos: &[LineInfo],
    use_idx: usize,
    line: &str,
    fm: FrameMem,
) -> Option<(i64, i64)> {
    // Parse "disp(%base,%idx,scale)".
    let lp = line.find('(')?;
    let rp = line[lp..].find(')')? + lp;
    let inside = &line[lp + 1..rp];
    let mut fields = inside.split(',');
    fields.next()?;
    let idx = fields.next()?.trim_ascii();
    let scale: i32 = match fields.next().map(str::trim_ascii) {
        None | Some("") => 1,
        Some(s) => s.parse().ok()?,
    };
    if !matches!(scale, 1 | 2 | 4 | 8) {
        return None;
    }
    let family = register_family_fast(idx);
    if family as usize > 15 {
        return None;
    }
    // Defense in depth: the index register is identified by its POSITION
    // in the SIB operand — the `reg_refs` mention bitmask cannot tell the
    // index apart from the base or the destination, so the text parse
    // stays authoritative for roles. The mask CAN prove the family is
    // mentioned at all; a disagreement means either parser is wrong, and
    // analyzing the wrong family could under-protect a frame read and
    // make a store wrongly deletable.
    debug_assert!(
        infos[use_idx].reg_refs & (1u16 << family as u16) != 0,
        "SIB index {idx} (family {family}) absent from line reg_refs: {line}"
    );

    // Access width: an UPPER bound on the bytes read. An under-estimate here
    // could leave an aliased store deletable, so the suffix table only
    // accepts mnemonics whose suffix truthfully gives the memory width.
    let mnem = line.split_whitespace().next()?;
    // x87 suffixes do NOT follow the GP convention (`fmull` reads an 8-byte
    // double, not 4; `fstpt` a 10-byte tbyte) and bit-string ops reach
    // beyond the operand word (`bts $bit,m32` accesses m32 + bit/32 word);
    // neither is ever emitted into a frame-SIB operand — refuse.
    let mnem_body = mnem.trim_start_matches('v');
    if mnem_body.starts_with('f')
        // bt/bts/btr/btc (incl. their l/q suffix spellings) address a
        // bitstring that may extend into the next operand word; state
        // saves/restores write/read areas far wider than any suffix says.
        || mnem_body.starts_with("bt")
        || mnem_body.starts_with("xsave")
        || mnem_body.starts_with("xrstor")
    {
        return None;
    }
    let width: i32 = match mnem_body {
        m if m.ends_with("b") && !m.starts_with("bt") => 1,
        m if m.ends_with("w") => 2,
        m if m.ends_with("l") => 4,
        m if m.ends_with("q") => 8,
        // Packed vector forms: widen by the widest vector register named;
        // an over-estimate only enlarges the protected read range.
        _ if line.contains("%zmm") => 64,
        _ if line.contains("%ymm") => 32,
        _ if line.contains("%xmm") => 16,
        _ => return None,
    };

    // Enclosing basic block.
    let mut block_start = use_idx;
    while block_start > 0 && !matches!(infos[block_start - 1].kind, LineKind::Label) {
        block_start -= 1;
    }
    let mut block_end = use_idx + 1;
    while block_end < infos.len() && !matches!(infos[block_end].kind, LineKind::Label) {
        block_end += 1;
    }

    // Label line indices for deciding whether a conditional jump is a
    // back-edge (continue predicate) or a forward exit guard.
    // FxHashMap, not the std SipHash default: this map is built by a
    // whole-file scan on every probe and served 19.7M hashed lookups on the
    // profiling TU — Callgrind priced the RandomState hashing alone at ~3%
    // of total compile Ir.  Only `.entry()`/`.get()` touch it, so the hash
    // function cannot change any pass decision.
    let mut label_pos: crate::common::fx_hash::FxHashMap<&str, usize> =
        crate::common::fx_hash::FxHashMap::default();
    for (li, info) in infos.iter().enumerate() {
        if matches!(info.kind, LineKind::Label) {
            let t = info.trimmed(store.get(li));
            label_pos.entry(t.trim_end_matches(':')).or_insert(li);
        }
    }

    /// Parse `$N` (decimal or 0x hex) into a positive i64.
    fn pos_imm(s: &str) -> Option<i64> {
        let s = s.trim_ascii();
        let v = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            i64::from_str_radix(h, 16).ok()?
        } else {
            s.parse::<i64>().ok()?
        };
        (v >= 1).then_some(v)
    }

    let mut upper: Option<i32> = None;
    for k in block_start..block_end {
        if infos[k].is_nop() {
            continue;
        }
        let t = infos[k].trimmed(store.get(k));
        // A write to the index family inside the loop body: only a
        // monotonically POSITIVE CONSTANT induction step is legal. cmp/test
        // READ operands even though the compared register is the last
        // textual operand; anything else (mov from a variable, variable
        // stride, mul, …) makes the index range unprovable.
        let is_compare = t.starts_with("cmp") || t.starts_with("test");
        let dest = if is_compare {
            REG_NONE
        } else {
            parse_dest_reg_fast(t)
        };
        if dest == family {
            let mut legal_step = t.starts_with("incq") || t.starts_with("incl");
            if !legal_step {
                for pre in ["addq $", "addl $"] {
                    if let Some(rest) = t.strip_prefix(pre) {
                        let (imm_str, dst) = rest.split_once(',')?;
                        if register_family_fast(dst.trim_ascii()) == family {
                            legal_step = pos_imm(imm_str).is_some();
                        }
                        break;
                    }
                }
            }
            // A self-lea `leaq N(%rX),%rX` or the scale strides
            // `leaq (%rX,%rX,S),%rX` / `leaq N(%rX,%rX,S),%rX` with a
            // positive constant N (S defaults to 1 when omitted). Any other
            // index register makes the stride variable — refuse.
            if !legal_step && t.starts_with("leaq ") {
                let rest = t.strip_prefix("leaq ").unwrap_or("");
                if let Some((mem, dst)) = rest.split_once(',') {
                    if register_family_fast(dst.trim_ascii()) == family {
                        let open = mem.find('(')?;
                        let close = mem.rfind(')')?;
                        let inside = &mem[open + 1..close];
                        let mut comps = inside.split(',');
                        let base = comps.next().unwrap_or("").trim_ascii();
                        let stride_idx = comps.next().map(str::trim_ascii);
                        let stride = if let Some(si) = stride_idx {
                            if !si.is_empty() && si != idx {
                                false
                            } else {
                                let sc = comps.next().map(str::trim_ascii).unwrap_or("1");
                                sc.parse::<i64>().map(|v| v >= 1).unwrap_or(false)
                                    && (si.is_empty() || si == idx)
                            }
                        } else {
                            true
                        };
                        let disp_txt = mem[..open].trim_ascii();
                        let disp = if disp_txt.is_empty() {
                            Some(0i64)
                        } else {
                            if let Some(h) = disp_txt
                                .strip_prefix("0x")
                                .or_else(|| disp_txt.strip_prefix("0X"))
                            {
                                i64::from_str_radix(h, 16).ok()
                            } else {
                                disp_txt.parse::<i64>().ok()
                            }
                        };
                        legal_step = stride
                            && base == idx
                            && disp.is_some_and(|d| d >= 0)
                            && (stride_idx.is_some() || disp.unwrap() >= 1);
                    }
                }
            }
            if !legal_step {
                return None;
            }
        }
        // Loop test: cmpq $B,%rX / cmpl $B,%rXd, followed by a jcc.
        if let Some(rest) = t
            .strip_prefix("cmpq $")
            .or_else(|| t.strip_prefix("cmpl $"))
        {
            let (imm_str, cmp_reg) = rest.split_once(',')?;
            if register_family_fast(cmp_reg.trim_ascii()) == family {
                let imm: i32 = imm_str.trim_ascii().parse().ok()?;
                // Find the next non-nop conditional jump.
                let mut j = k + 1;
                while j < block_end && infos[j].is_nop() {
                    j += 1;
                }
                let jt = if j < block_end {
                    infos[j].trimmed(store.get(j))
                } else {
                    ""
                };
                let jcc = jt.split_whitespace().next().unwrap_or("");
                // Back-edge (target at or before this block): the condition
                // CONTINUES the loop. Forward guard: the condition EXITS.
                let target = jt.split_whitespace().nth(1).unwrap_or("");
                let target_pos = label_pos.get(target).copied();
                let backward = target_pos.is_some_and(|p| p <= k);
                // Exclusive upper B (in-loop values satisfy idx < B), or
                // inclusive (idx <= B, hence B+1).
                let b: Option<i32> = if backward {
                    match jcc {
                        "jl" | "jb" | "jne" | "jnz" => Some(imm),
                        "jle" | "jbe" => Some(imm + 1),
                        _ => None,
                    }
                } else {
                    match jcc {
                        "jge" | "jae" => Some(imm),
                        "jg" | "ja" => Some(imm + 1),
                        _ => None,
                    }
                };
                let Some(b) = b else { return None };
                upper = Some(upper.map_or(b, |u| u.max(b)));
            }
        }
    }
    let b = upper?;

    // Constant init in the (recent) preheader: min A over matching constant
    // inits within 80 lines before the block start (widest downward reach).
    let win_start = block_start.saturating_sub(80);
    let mut lower: Option<i32> = None;
    for k in (win_start..block_start).rev() {
        if infos[k].is_nop() {
            continue;
        }
        let t = infos[k].trimmed(store.get(k));
        if parse_dest_reg_fast(t) != family {
            continue;
        }
        let a = if t.starts_with("movq $") || t.starts_with("movl $") {
            t.split(',').next().and_then(|p| {
                p.trim_ascii()
                    .trim_start_matches("movq $")
                    .trim_start_matches("movl $")
                    .parse::<i32>()
                    .ok()
            })
        } else if t.starts_with("xorl ") || t.starts_with("xorq ") {
            let ps: Vec<&str> = t.split(',').map(str::trim_ascii).collect();
            (ps.len() == 2 && ps[0] == ps[1]).then_some(0)
        } else {
            None
        };
        if let Some(a) = a {
            lower = Some(lower.map_or(a, |l| l.min(a)));
        }
    }
    let a = lower?;

    let scaled = |index| i64::from(fm.disp) + i64::from(index) * i64::from(scale);
    let lo = scaled(a);
    let hi = scaled(b) + i64::from(width);
    if hi <= lo {
        return None;
    }
    Some((lo, hi - lo))
}

/// What an SIB frame reference conservatively observes.
/// Returns `(base, off, len, is_floor)`: `is_floor` marks the unbounded
/// `[off, +∞)` model (only safe for non-negative displacements — a negative
/// textual displacement with an unbounded index can reach below the frame's
/// home slots, and the caller must bail).
fn sib_frame_observation(
    store: &LineStore,
    infos: &[LineInfo],
    use_idx: usize,
    line: &str,
) -> Option<(u8, i64, i64, bool)> {
    let fm = parse_frame_mem(line)?;
    if !fm.sib {
        return None;
    }
    if let Some((off, len)) = sib_frame_read_range(store, infos, use_idx, line, fm) {
        return Some((fm.base, off, len, false));
    }
    if fm.disp >= 0 {
        Some((fm.base, i64::from(fm.disp), 0, true))
    } else {
        None
    }
}

/// True if a store to `[offset, offset+store_bytes)` on `base` is observed by
/// any collected read range or unbounded-upward read floor.
fn slot_is_ever_read(
    base: u8,
    offset: i32,
    store_bytes: i32,
    read_ranges: &[(u8, i64, i64)],
    read_floors: &[(u8, i64)],
) -> bool {
    let lo = i64::from(offset);
    let hi = lo + i64::from(store_bytes);
    // The analysis does not establish a fixed relationship between frame
    // bases. An observation on the other one cannot prove disjointness.
    read_ranges
        .iter()
        .any(|&(rb, ro, rs)| rb != base || (lo < ro + rs && ro < hi))
        || read_floors.iter().any(|&(rb, d)| rb != base || hi > d)
}

// Share the architectural read contract with FP slot liveness. A suffix or
// an arbitrary 16/32/64-byte ceiling is not a bound on state images or future
// instructions. Unknown extents make this function's slot analysis opaque.
fn frame_read_extent(line: &str) -> Option<i32> {
    let code = line.split('#').next()?.trim_ascii();
    let (mnemonic, operands) = code.split_once(char::is_whitespace)?;
    super::fp_liveness::read_extent(mnemonic, operands.trim_ascii())
}

/// Removes a full-width vector store to an `%rsp` frame slot when no path from
/// the store reads its bytes before they are overwritten or the function exits.
/// The store must lie inside the function's own frame (see below).
///
/// `eliminate_never_read_stores` is address-global: a store survives while any
/// read anywhere in the function covers its bytes. That is too coarse once
/// block-local vector temporaries share a slot (`vector_pool.rs`). The stores of
/// values that are consumed in their register are then kept alive by a reload of
/// a different value in the same slot. The exact per-slot liveness oracle sees
/// the control flow: a loop body that rewrites the slot before it reads it, and
/// a function exit, kill the store. Only `%rsp`-relative stores are considered,
/// which is the form the vector spill path emits; every other store keeps the
/// established rules.
///
/// Soundness: a store is removed only when `slot_dead_after` proves that no
/// read of its bytes is reachable. Removing such a store adds no read and
/// removes only a kill whose effect is already dead, so the facts computed
/// before the removal stay valid for later stores in this pass (the same
/// argument `eliminate_dead_fp_moves` relies on).
pub(super) fn eliminate_flow_dead_vector_stores(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    const RSP_FAMILY: u8 = 4;
    let mut changed = false;
    let mut fp: Option<FpLiveness> = None;
    // Bytes of this function's own frame, from the first `subq $N, %rsp` after
    // its `.cfi_startproc`. Offsets in `[0, N)` name the frame; anything else
    // (a caller's stack argument, a function with no frame) is not dead when
    // the function returns, so those stores keep their established treatment.
    let mut frame_bytes: Option<i64> = None;
    for i in 0..infos.len() {
        let info = infos[i];
        if info.is_nop() {
            continue;
        }
        let text = info.trimmed(store.get(i));
        if info.kind == LineKind::Directive && is_frame_boundary(text) {
            frame_bytes = None;
            continue;
        }
        if frame_bytes.is_none() {
            if let Some(rest) = text.strip_prefix("subq $") {
                frame_bytes = rest
                    .strip_suffix(", %rsp")
                    .and_then(|v| v.parse::<i64>().ok());
            }
            continue;
        }
        if info.pinned {
            continue;
        }
        let LineKind::StoreXmmRbp { offset, size } = info.kind else {
            continue;
        };
        // An indexed frame operand names no fixed slot; keep it.
        if frame_base(text, 0) != b's' || text.contains("(%rsp,") {
            continue;
        }
        let end = i64::from(offset) + i64::from(size.byte_size());
        if i64::from(offset) < 0 || Some(end) > frame_bytes {
            continue;
        }
        if fp
            .get_or_insert_with(|| FpLiveness::new(store, infos))
            .slot_dead_after(i, RSP_FAMILY, offset, size.byte_size())
        {
            mark_nop(&mut infos[i]);
            changed = true;
        }
    }
    changed
}

pub(super) fn eliminate_never_read_stores(store: &LineStore, infos: &mut [LineInfo]) {
    let len = store.len();
    if len == 0 {
        return;
    }

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Detect function prologue — two forms:
        // 1. Frame pointer: pushq %rbp + movq %rsp,%rbp + subq $N,%rsp
        // 2. Frame-pointer-less: subq $N,%rsp (without push %rbp)
        let body_start;

        // Frame-pointer status: Form 1 establishes a frame pointer (so
        // `(%rbp)` is a genuine stack slot); Form 2 does not (so %rbp is a free
        // data register and `(%rbp)` is a pointer dereference, never a stack slot).
        let rbp_is_frame;

        if matches!(infos[i].kind, LineKind::Push { reg: REG_RBP }) {
            rbp_is_frame = true;
            // Form 1: frame pointer prologue
            let mut j = next_non_nop(infos, i + 1, len);
            if j >= len {
                i = j;
                continue;
            }
            let mov_line = infos[j].trimmed(store.get(j));
            if mov_line != "movq %rsp, %rbp" {
                // Not a frame-pointer prologue: %rbp was pushed as an ordinary
                // callee-saved register (no-FP mode allocates rbp), and the
                // real prologue marker is the `subq $N,%rsp` that follows the
                // push chain.  Falling through to the form-2 detection lets
                // that line be examined instead of skipping it — the old
                // `i = j + 1` jumped PAST the subq and disabled dead-store
                // elimination for every six-push no-FP function.
                i += 1;
                continue;
            }
            j += 1;

            j = next_non_nop(infos, j, len);
            if j >= len {
                i = j;
                continue;
            }
            let subq_line = infos[j].trimmed(store.get(j));
            let is_subq = if let Some(rest) = subq_line.strip_prefix("subq $") {
                rest.strip_suffix(", %rsp")
                    .and_then(|v| v.parse::<i64>().ok())
                    .is_some()
            } else {
                false
            };
            if !is_subq {
                i = j + 1;
                continue;
            }
            j += 1;

            // Skip callee-saved register saves
            j = next_non_nop(infos, j, len);
            let mut callee_save_end = j;
            while callee_save_end < len {
                if infos[callee_save_end].is_nop() {
                    callee_save_end += 1;
                    continue;
                }
                if let LineKind::StoreRbp {
                    reg,
                    size: MoveSize::Q,
                    ..
                } = infos[callee_save_end].kind
                {
                    if is_callee_saved_reg(reg) {
                        callee_save_end += 1;
                        continue;
                    }
                }
                break;
            }
            body_start = callee_save_end;
        } else {
            // Form 2: frame-pointer-less prologue (subq $N, %rsp)
            rbp_is_frame = false;
            let line = infos[i].trimmed(store.get(i));
            let is_subq_rsp = if let Some(rest) = line.strip_prefix("subq $") {
                rest.strip_suffix(", %rsp")
                    .and_then(|v| v.parse::<i64>().ok())
                    .is_some()
            } else {
                false
            };
            if !is_subq_rsp {
                i += 1;
                continue;
            }
            // Must be preceded by .cfi_startproc (within 3 lines) to confirm this
            // is a function prologue and not just a random subq in the middle.
            // Callee-saved pushes (`pushq %rbx` … `pushq %rbp`) legitimately sit
            // between the .cfi directives and the `subq $N,%rsp`: a six-push
            // prologue moved the directive 7+ lines away and the old fixed
            // 3-line window missed it, silently skipping dead-store
            // elimination for the entire function (gzip CRC kernel: a
            // never-read `movq %rax, 8(%rsp)` survived forever).  Scan back
            // past up to 8 push-save lines for the directive; any other
            // non-nop line terminates the window (an interior `subq` must not
            // match a directive belonging to an earlier function).
            let mut found_cfi = false;
            let check_start = if i >= 16 { i - 16 } else { 0 };
            for k in (check_start..i).rev() {
                if infos[k].is_nop() || matches!(infos[k].kind, LineKind::Push { .. }) {
                    continue;
                }
                // DIRECTIVE-LAW: selecting -- the walk reads each directive's
                // text to locate the named prologue markers
                // (cfi_startproc/cfi_def_cfa_offset). Only the code-less
                // `.cfi_*` family (`.cfi_offset`/`.cfi_remember_state`), proven
                // by its content prefix below, is stepped over; any other
                // directive can emit data and ends the backward walk.
                if matches!(infos[k].kind, LineKind::Directive) {
                    let dl = infos[k].trimmed(store.get(k));
                    if dl.contains("cfi_startproc") || dl.contains("cfi_def_cfa_offset") {
                        found_cfi = true;
                        break;
                    }
                    if infos[k].cfi != CfiMark::None {
                        continue;
                    }
                    break;
                }
                break;
            }
            if !found_cfi {
                i += 1;
                continue;
            }

            // Skip callee-saved register saves after the subq
            let mut j = next_non_nop(infos, i + 1, len);
            // Skip .cfi_def_cfa_offset directive if present.  By *text*: matching the
            // class would step the prologue walk over any directive, including a
            // `.long` that emits instructions, and the callee-save scan below would
            // then start inside data.  // DIRECTIVE-LAW: selecting
            if j < len && infos[j].is_cfi_dir() {
                j = next_non_nop(infos, j + 1, len);
            }
            let mut callee_save_end = j;
            while callee_save_end < len {
                if infos[callee_save_end].is_nop() {
                    callee_save_end += 1;
                    continue;
                }
                if let LineKind::StoreRbp {
                    reg,
                    size: MoveSize::Q,
                    ..
                } = infos[callee_save_end].kind
                {
                    if is_callee_saved_reg(reg) {
                        callee_save_end += 1;
                        continue;
                    }
                }
                break;
            }
            body_start = callee_save_end;
        }

        // Find the end of this function
        let mut func_end = len;
        for k in body_start..len {
            if infos[k].is_nop() {
                continue;
            }
            let line = infos[k].trimmed(store.get(k));
            if line.starts_with(".size ") {
                func_end = k + 1;
                break;
            }
        }

        // Phase 1: Collect all "read" byte ranges.
        //
        // Escape discipline: a stack slot can be observed by an indirect
        // access (deref through a register, or by a callee) ONLY if a frame
        // address escaped first — `leaq N(%rsp|%rbp), %reg` or a raw copy of
        // %rsp/%rbp-as-value into a register. C pointer arguments always
        // point at caller-owned memory, never at this fresh frame. So:
        //   * no escape event  -> indirect derefs are irrelevant to frame
        //     slots and are IGNORED (the old blanket bail killed this pass
        //     for every pointer-walking loop);
        //   * any escape event -> bail for the whole function (a leaked
        //     address may reach any slot via arithmetic or a callee).
        //
        // Pre-scan for a raw frame-base escape before collecting reads.
        let mut has_unparseable_indirect =
            frame_base_escapes(store, infos, body_start, func_end, rbp_is_frame);
        // Base-tagged frame reads and unbounded-upward "floors" (escaped
        // addresses and SIB array accesses). Base: b'b' = %rbp, b's' = %rsp.
        let mut read_ranges: Vec<(u8, i64, i64)> = Vec::new();
        // A floor at displacement D conservatively observes every frame
        // byte from D upward: an escaped `leaq D(%base)` pointer or an SIB
        // operand `D(%base,%idx,scale)` names an array whose element
        // addressing only extends upward (a negative index would be an
        // out-of-bounds access of a DIFFERENT object — C UB — so codegen
        // never relies on downward reach). Stores ending at or below D stay
        // deletable; this is what distinguishes the home-slot spills at
        // 16/32(%rsp) from a frame buffer based at 40(%rsp).
        let mut read_floors: Vec<(u8, i64)> = Vec::new();
        // Stores proven to be the ONLY writers a plain slot load observes
        // (see `locally_covering_stores`): such a load is attributed to them
        // instead of being published as a function-wide read range.
        let mut locally_read: Vec<usize> = Vec::new();
        // Constant byte distance of the current rsp below its canonical
        // (post-prologue) value across balanced in-body push/pop/sub/add
        // regions (variadic call-arg staging). Zero everywhere except those
        // regions; a non-parseable shift or an unbalanced terminator bails.
        let mut rsp_delta: i64 = 0;
        // Set once the linear path ends in a `ret`/unconditional `jmp`: the
        // next label is a branch-only target entered at canonical rsp depth,
        // so the prior block's epilogue delta must not carry into it.
        let mut path_ended = false;

        for k in body_start..func_end {
            if infos[k].is_nop() {
                continue;
            }
            let t = infos[k].trimmed(store.get(k));

            // Track the linear rsp depth across balanced push/pop windows and
            // the path-end label reset, translating frame operands below back
            // to canonical offsets via `canonical_frame_off`. A `false` return
            // means this line makes the verbatim-offset model unsafe (memory
            // form push, unparseable shift, branch at non-canonical depth, …):
            // leave the whole function unoptimized rather than mis-attribute.
            if !step_rsp_depth(store, infos, k, func_end, &mut rsp_delta, &mut path_ended) {
                has_unparseable_indirect = true;
                break;
            }

            // Indirect memory accesses through non-frame registers cannot
            // alias frame slots when no frame address escaped (checked by
            // the pre-scan above), so they are NOT a bail condition here.

            match infos[k].kind {
                LineKind::StoreRbp { .. } => {
                    // SOUNDNESS: when %rbp is not the frame pointer, an
                    // `(%rbp)` store is a pointer write to arbitrary memory — it
                    // may read/alias any stack slot. Bail out of never-read
                    // elimination entirely.
                    if !rbp_is_frame && uses_rbp_mem_operand(infos[k].trimmed(store.get(k))) {
                        has_unparseable_indirect = true;
                        break;
                    }
                    // A frame store below canonical depth (inside a push/pop
                    // window) uses shifted addressing that Phase 2's
                    // canonical-offset comparison cannot attribute. Call-arg
                    // windows normally use `push` (LineKind::Push), so this is
                    // rare; fail closed rather than misattribute the write.
                    if rsp_delta != 0 && frame_base(t, b's') == b's' {
                        has_unparseable_indirect = true;
                        break;
                    }
                }
                // Scalar-FP / SSE slot stores: an `(%rsp)` store is always a
                // frame-slot write (the FP value lives in a register; only the
                // slot base can be rsp). An `(%rbp)` store with %rbp NOT the
                // frame pointer is a pointer write — bail, exactly like the
                // GP case above — because such a store could write a frame
                // slot through a data-register %rbp value.
                LineKind::StoreXmmRbp { .. } => {
                    if !rbp_is_frame && uses_rbp_mem_operand(infos[k].trimmed(store.get(k))) {
                        has_unparseable_indirect = true;
                        break;
                    }
                    if rsp_delta != 0 && frame_base(t, b's') == b's' {
                        has_unparseable_indirect = true;
                        break;
                    }
                }
                // GP and XMM slot loads read the same way: the exact extent
                // the instruction reads, attributed to the stores that cover
                // it earlier in the body when there are such stores.
                LineKind::LoadRbp { offset, .. } | LineKind::LoadXmmRbp { offset, .. } => {
                    let line = infos[k].trimmed(store.get(k));
                    if !rbp_is_frame && uses_rbp_mem_operand(line) {
                        has_unparseable_indirect = true;
                        break;
                    }
                    let base = frame_base(line, b's');
                    let Some(width) = frame_read_extent(line) else {
                        has_unparseable_indirect = true;
                        break;
                    };
                    // Map a shifted-depth operand back to its canonical slot so
                    // it is compared against the canonical store offsets.
                    let coff = canonical_frame_off(base, i64::from(offset), rsp_delta);
                    match locally_covering_stores(store, infos, k, body_start, base, coff, width) {
                        Some(writers) => locally_read.extend(writers),
                        None => read_ranges.push((base, coff, i64::from(width))),
                    }
                }
                LineKind::Other { .. } => {
                    let rbp_off = infos[k].rbp_offset;
                    let line = infos[k].trimmed(store.get(k));
                    let lea = is_lea_instruction(line);
                    // A LEA never reads memory, and on a data-register %rbp it
                    // names an address derived from a data value: no frame slot.
                    if lea && lea_on_data_register_rbp_touches_no_frame_slot(line, rbp_is_frame) {
                        continue;
                    }
                    if lea {
                        // Address taken at a frame slot: the derived pointer
                        // may reach an unbounded upward span (passed to a
                        // callee, indexed later) — record a floor, not a
                        // fixed-width range.
                        if let Some(fm) = parse_frame_mem(line) {
                            if fm.sib && rsp_delta != 0 {
                                has_unparseable_indirect = true;
                                break;
                            }
                            let off = canonical_frame_off(fm.base, i64::from(fm.disp), rsp_delta);
                            read_floors.push((fm.base, off));
                        } else if rbp_off != RBP_OFFSET_NONE {
                            let base = frame_base(line, b's');
                            let off = canonical_frame_off(base, i64::from(rbp_off), rsp_delta);
                            read_floors.push((base, off));
                        }
                    } else if rbp_off != RBP_OFFSET_NONE {
                        let base = frame_base(line, b's');
                        let Some(width) = frame_read_extent(line) else {
                            has_unparseable_indirect = true;
                            break;
                        };
                        let off = canonical_frame_off(base, i64::from(rbp_off), rsp_delta);
                        read_ranges.push((base, off, i64::from(width)));
                    } else if let Some(fm) = parse_frame_mem(line) {
                        // Frame operand that was neither a leaq, a cached
                        // plain access, nor a provably bounded SIB: fail
                        // closed (unbounded negative-reach SIB).
                        if fm.sib && rsp_delta != 0 {
                            has_unparseable_indirect = true;
                            break;
                        }
                        if let Some((base, off, len, is_floor)) =
                            sib_frame_observation(store, infos, k, line)
                        {
                            let off = canonical_frame_off(base, off, rsp_delta);
                            if is_floor {
                                read_floors.push((base, off));
                            } else {
                                read_ranges.push((base, off, len));
                            }
                        } else {
                            has_unparseable_indirect = true;
                            break;
                        }
                    }
                }
                LineKind::Nop
                | LineKind::Empty
                | LineKind::SelfMove
                | LineKind::Label
                | LineKind::Jmp
                | LineKind::CondJmp
                | LineKind::JmpIndirect
                | LineKind::Ret
                | LineKind::Directive => {}
                LineKind::InlineAsm => {
                    let line = infos[k].trimmed(store.get(k));
                    // A known direct operand observes its architectural extent.
                    // State images and unsupported mnemonics are NOT bounded by
                    // the widest vector register. Fail closed for the function.
                    if let Some(fm) = parse_frame_mem(line) {
                        if fm.sib {
                            // Raw asm index values are not bound by C UB: a
                            // negative or arbitrary index can name every
                            // lower slot. Fail closed.
                            has_unparseable_indirect = true;
                            break;
                        }
                        let Some(width) = frame_read_extent(line) else {
                            has_unparseable_indirect = true;
                            break;
                        };
                        let off = canonical_frame_off(fm.base, i64::from(fm.disp), rsp_delta);
                        read_ranges.push((fm.base, off, i64::from(width)));
                    }
                }
                _ => {
                    let line = infos[k].trimmed(store.get(k));
                    let rbp_off = parse_rbp_offset(line);
                    if rbp_off != RBP_OFFSET_NONE {
                        // Range, not point: x87 folded operands read past
                        // the named offset (fldt = 10 bytes, movdqu = 16).
                        // 8 bytes missed the tail of a long double slot.
                        let base = frame_base(line, b's');
                        let Some(width) = frame_read_extent(line) else {
                            has_unparseable_indirect = true;
                            break;
                        };
                        let off = canonical_frame_off(base, i64::from(rbp_off), rsp_delta);
                        read_ranges.push((base, off, i64::from(width)));
                    } else if let Some(fm) = parse_frame_mem(line) {
                        if fm.sib && rsp_delta != 0 {
                            has_unparseable_indirect = true;
                            break;
                        }
                        if let Some((base, off, len, is_floor)) =
                            sib_frame_observation(store, infos, k, line)
                        {
                            let off = canonical_frame_off(base, off, rsp_delta);
                            if is_floor {
                                read_floors.push((base, off));
                            } else {
                                read_ranges.push((base, off, len));
                            }
                        } else {
                            has_unparseable_indirect = true;
                            break;
                        }
                    }
                }
            }
        }

        if has_unparseable_indirect {
            i = func_end;
            continue;
        }

        // Phase 2: Eliminate stores to unread slots
        locally_read.sort_unstable();
        for k in body_start..func_end {
            if infos[k].is_nop() || locally_read.binary_search(&k).is_ok() {
                continue;
            }
            if let LineKind::StoreRbp { offset, size, .. } = infos[k].kind {
                let store_text = infos[k].trimmed(store.get(k));
                // A store to the current stack top is an architectural input
                // to an immediately following pop/ret.  Inline retpolines use
                // exactly `movq %target,(%rsp); ret` to replace the speculative
                // return address.  Treating only explicit loads as reads
                // deleted that store and trapped forever in the pause loop.
                if offset == 0
                    && store_text.ends_with("(%rsp)")
                    && stack_top_is_consumed_next(store, infos, k + 1, func_end)
                {
                    continue;
                }
                // SOUNDNESS: never delete an `(%rbp)` store when %rbp is a
                // data register (pointer store to arbitrary memory).
                if !rbp_is_frame && uses_rbp_mem_operand(store_text) {
                    continue;
                }
                let store_bytes = size.byte_size();
                let base = frame_base(store_text, b's');
                if !slot_is_ever_read(base, offset, store_bytes, &read_ranges, &read_floors) {
                    mark_nop(&mut infos[k]);
                }
            }
            // Scalar-FP / SSE slot stores: same never-read test. Escape
            // discipline was already enforced (no leaq/raw-%rsp, and the
            // %rbp-as-data-register bail above), so an unread FP slot store
            // is dead — this kills the union/slot roundtrips in FP kernels
            // (copysign: `movsd %xmm0, 16(%rsp)` + `movq %rdi, 16(%rsp)`
            // were never reloaded).
            if let LineKind::StoreXmmRbp { offset, size } = infos[k].kind {
                let store_text = infos[k].trimmed(store.get(k));
                // Same architectural stack-top exception as the GP arm:
                // a store to the current stack top may be an input to an
                // immediately following pop/ret.
                if offset == 0
                    && store_text.ends_with("(%rsp)")
                    && stack_top_is_consumed_next(store, infos, k + 1, func_end)
                {
                    continue;
                }
                if !rbp_is_frame && uses_rbp_mem_operand(store_text) {
                    continue;
                }
                let store_bytes = size.byte_size();
                let base = frame_base(store_text, b's');
                if !slot_is_ever_read(base, offset, store_bytes, &read_ranges, &read_floors) {
                    mark_nop(&mut infos[k]);
                }
            }
        }

        i = func_end;
    }
}

/// The stores a plain frame-slot load at `load` observes, when they can be
/// proven to be ALL of them: walking backward through the load's straight-line
/// run, every byte of `[offset, offset + bytes)` is written by a slot store on
/// the same base before any point where an unseen writer could intervene.
///
/// Returning `Some(writers)` lets the caller attribute the load to exactly
/// those stores (they stay alive) instead of publishing it as a function-wide
/// read range — the classic upward-exposed-use refinement. Without it, any
/// load overlapping a slot pinned EVERY store to that slot anywhere in the
/// function: `zlib_ng_adler32_combine` kept a never-read
/// `movl %eax, 68(%rsp)` inside its hot loop because an unrelated block did
/// `movq %rax, 64(%rsp); …; movq 64(%rsp), %rdi` with the 8-byte reuse of the
/// slot overlapping `[68, 72)`.
///
/// The walk answers `None` (the caller then keeps the old, conservative
/// whole-range read) at anything that could make the backward view
/// incomplete or the offsets incomparable:
/// * a label (another predecessor may reach the load), any jump, `ret`,
///   `call`, or the body start;
/// * inline or pinned assembly, or a data-emitting directive
///   ([`steps_over_line`] refuses those);
/// * any `%rsp` shift (push/pop/`subq`/`addq`), which moves every slot;
/// * any other line with a frame memory operand — an immediate store, a
///   read-modify-write, a string op on a slot — that may write the bytes;
/// * a slot store on the other frame base (comparable only through the
///   frame layout, which this walk does not model).
///
/// Indirect stores through non-frame registers are stepped over: the caller
/// runs only after its escape pre-scan proved no frame address reached a
/// register, so they cannot alias a slot. A load wider than 64 bytes (never
/// emitted for a slot) is refused rather than tracked.
fn locally_covering_stores(
    store: &LineStore,
    infos: &[LineInfo],
    load: usize,
    body_start: usize,
    base: u8,
    lo: i64,
    bytes: i32,
) -> Option<Vec<usize>> {
    if !(1..=64).contains(&bytes) {
        return None;
    }
    let hi = lo + i64::from(bytes);
    let mut uncovered: u64 = if bytes == 64 {
        u64::MAX
    } else {
        (1u64 << bytes) - 1
    };
    let mut writers = Vec::new();
    let mut n = load;
    while n > body_start {
        n -= 1;
        let t = infos[n].trimmed(store.get(n));
        if steps_over_line(&infos[n], t) {
            continue;
        }
        if infos[n].pinned {
            return None;
        }
        let (s_off, s_size) = match infos[n].kind {
            LineKind::StoreRbp { offset, size, .. } | LineKind::StoreXmmRbp { offset, size } => {
                (offset, size.byte_size())
            }
            LineKind::Label
            | LineKind::Jmp
            | LineKind::CondJmp
            | LineKind::JmpIndirect
            | LineKind::Ret
            | LineKind::Call
            | LineKind::InlineAsm
            | LineKind::Push { .. }
            | LineKind::Pop { .. }
            | LineKind::Directive => return None,
            _ => {
                if is_rsp_shift_line(t)
                    || infos[n].rbp_offset != RBP_OFFSET_NONE
                    || parse_frame_mem(t).is_some()
                    || t.contains("(%rsp")
                    || t.contains("(%rbp")
                {
                    return None;
                }
                continue;
            }
        };
        if frame_base(t, b's') != base {
            return None;
        }
        let s_lo = i64::from(s_off);
        let s_hi = s_lo + i64::from(s_size);
        let (o_lo, o_hi) = (lo.max(s_lo), hi.min(s_hi));
        if o_lo >= o_hi {
            continue;
        }
        let width = o_hi - o_lo;
        let run = if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        let mask = run << (o_lo - lo);
        if uncovered & mask != 0 {
            writers.push(n);
            uncovered &= !mask;
            if uncovered == 0 {
                return Some(writers);
            }
        }
    }
    None
}

/// Whether the next executable instruction consumes the current stack top.
/// Labels and unwind directives do not execute and therefore do not break the
/// producer/consumer relation.
fn stack_top_is_consumed_next(
    store: &LineStore,
    infos: &[LineInfo],
    mut idx: usize,
    end: usize,
) -> bool {
    while idx < end {
        if matches!(infos[idx].kind, LineKind::Empty | LineKind::Label)
            || super::helpers::steps_over_line(&infos[idx], infos[idx].trimmed(store.get(idx)))
        {
            idx += 1;
            continue;
        }
        return matches!(infos[idx].kind, LineKind::Pop { .. } | LineKind::Ret)
            || infos[idx].trimmed(store.get(idx)).starts_with("ret ");
    }
    false
}

/// Is the RSP-shift at `k` part of an epilogue chain?
///
/// Accepted shape from `k` forward (within a bounded window): zero or more of
/// { `addq $N,%rsp`, `popq` (callee-saved restore), nop, directive } ending in
/// `ret` or an unconditional `jmp` (tail call). `pushq`/`subq` in the window,
/// or any other instruction, rejects — those are genuine mid-body shifts.
fn is_epilogue_rsp_shift(store: &LineStore, infos: &[LineInfo], k: usize, func_end: usize) -> bool {
    let mut j = k;
    let limit = (k + 24).min(func_end);
    while j < limit {
        if super::helpers::steps_over_line(&infos[j], infos[j].trimmed(store.get(j))) {
            j += 1;
            continue;
        }
        match infos[j].kind {
            LineKind::Ret | LineKind::Jmp => return true,
            LineKind::Pop { .. } => {
                j += 1;
                continue;
            }
            _ => {
                let t = infos[j].trimmed(store.get(j));
                if let Some(rest) = t.strip_prefix("addq $") {
                    if rest
                        .strip_suffix(", %rsp")
                        .and_then(|v| v.parse::<i64>().ok())
                        .is_some()
                    {
                        j += 1;
                        continue;
                    }
                }
                return false;
            }
        }
    }
    false
}

/// True if the block an unconditional `jmp` at `k` lands in — the run from the
/// next label up to that block's terminator — names any frame memory operand.
/// Only consulted when the jmp is reached at non-canonical rsp depth: if the
/// target block touches a frame slot, its true entry depth cannot be
/// reconstructed linearly, so the caller fails closed; a pure pop/ret shared
/// epilogue names no frame operand and is safe to reset to canonical depth.
fn shifted_jmp_target_touches_frame(
    store: &LineStore,
    infos: &[LineInfo],
    k: usize,
    func_end: usize,
) -> bool {
    let mut q = k + 1;
    while q < func_end && infos[q].kind != LineKind::Label {
        q += 1;
    }
    while q < func_end
        && !matches!(
            infos[q].kind,
            LineKind::Ret | LineKind::Jmp | LineKind::CondJmp | LineKind::JmpIndirect
        )
    {
        if !infos[q].is_nop() && parse_frame_mem(infos[q].trimmed(store.get(q))).is_some() {
            return true;
        }
        q += 1;
    }
    false
}

/// Advance the linear rsp-depth model across one body line, updating the
/// signed `rsp_delta` (bytes rsp sits below its canonical post-prologue value)
/// and the `path_ended` latch (a `ret`/unconditional `jmp` makes the next
/// label a branch-only target that re-enters at canonical depth).
///
/// Returns `false` when the line makes the verbatim-offset model unsafe and
/// the whole function must be left unoptimized: a memory-form push/pop (which
/// shifts rsp and also reads/writes a frame slot), an unparseable
/// non-epilogue shift, a conditional/indirect branch at non-canonical depth,
/// or a shifted unconditional jmp whose target block accesses a frame slot.
/// A fall-through/loop-header label deliberately does not reset the depth:
/// loop back-edges legitimately target a label inside an open push/pop window.
fn step_rsp_depth(
    store: &LineStore,
    infos: &[LineInfo],
    k: usize,
    func_end: usize,
    rsp_delta: &mut i64,
    path_ended: &mut bool,
) -> bool {
    let t = infos[k].trimmed(store.get(k));
    if infos[k].kind == LineKind::Label {
        if *path_ended {
            *rsp_delta = 0;
        }
        *path_ended = false;
    } else if infos[k].kind == LineKind::Ret {
        *path_ended = true;
    } else if infos[k].kind == LineKind::Jmp {
        if *rsp_delta != 0 && shifted_jmp_target_touches_frame(store, infos, k, func_end) {
            return false;
        }
        *path_ended = true;
    }

    if is_rsp_shift_line(t) {
        // A push/pop with a *memory* operand (e.g. the variadic idiom
        // `movq %rax,0(%rsp); pushq 0(%rsp)`) shifts rsp AND reads or writes a
        // frame slot at the pre-shift depth; counting only the delta step would
        // drop that frame access. Register/immediate forms carry no frame
        // operand and are safe to model as a bare step.
        if (t.starts_with("push") || t.starts_with("pop")) && t.contains('(') {
            return false;
        }
        match rsp_delta_step(t) {
            Some(step) => {
                // The frame model uses disp32 operands. An unrepresentable
                // sequence must not wrap back onto an unrelated slot.
                let Some(delta) = rsp_delta
                    .checked_add(step)
                    .and_then(|n| i32::try_from(n).ok())
                else {
                    return false;
                };
                *rsp_delta = i64::from(delta);
            }
            None if is_epilogue_rsp_shift(store, infos, k, func_end) => {}
            None => return false,
        }
    }
    // A branch that leaves RSP displaced cannot be folded into a bare step.
    *rsp_delta == 0 || !matches!(infos[k].kind, LineKind::CondJmp | LineKind::JmpIndirect)
}

/// True when `line` is a LEA whose `(%rbp)` operand names a DATA register, not
/// the frame pointer, so it computes an address from a data value and touches
/// no frame slot at all.
///
/// Under `-fomit-frame-pointer` %rbp is an ordinary register. The old prefix
/// test only recognised the 64-bit spelling, so `leal -1(%rbp), %eax` fell
/// through to the generic rbp-offset arm, where `frame_read_extent` has no LEA
/// entry and bailed the whole function: one such line disabled never-read store
/// elimination everywhere (glibc_strstr kept two dead spill stores in its
/// hottest loop). Any *other* instruction on a data %rbp stays on the
/// conservative generic path below, unchanged.
fn lea_on_data_register_rbp_touches_no_frame_slot(line: &str, rbp_is_frame: bool) -> bool {
    !rbp_is_frame && line.contains("%rbp") && !line.contains("%rsp") && is_lea_instruction(line)
}

/// True for an address-computing LEA in any width (`leaq`/`leal`/`leaw`/`lea`).
/// A LEA never accesses memory, so a frame-base operand inside one names an
/// address, never a slot read; every spelling must be recognised, because a
/// missed spelling falls through to the generic frame-read arms.
fn is_lea_instruction(line: &str) -> bool {
    match line.split_once(char::is_whitespace) {
        Some((mnemonic, _)) => matches!(mnemonic, "lea" | "leaq" | "leal" | "leaw"),
        None => false,
    }
}

/// Pre-scan for a raw frame-base escape: a line with no frame memory operand
/// that copies the frame base into another register (`movq %rsp,%reg`, or
/// `%rbp` when it is the frame pointer). A leaked base may reach any frame
/// slot through arithmetic or a callee, so a hit bails the whole function.
/// Frame memory operands (plain or SIB) name a specific displacement and are
/// handled with ranges/floors by the read scan instead; frame adjustments and
/// the FP prologue `movq %rsp,%rbp` are not escapes.
fn frame_base_escapes(
    store: &LineStore,
    infos: &[LineInfo],
    body_start: usize,
    func_end: usize,
    rbp_is_frame: bool,
) -> bool {
    for k in body_start..func_end {
        if infos[k].is_nop() {
            continue;
        }
        let t = infos[k].trimmed(store.get(k));
        if parse_frame_mem(t).is_some() {
            continue;
        }
        if t.contains("%rsp") && !is_rsp_shift_line(t) && t != "movq %rsp, %rbp" {
            return true;
        }
        if rbp_is_frame
            && t.contains("%rbp")
            && t != "movq %rsp, %rbp"
            && !matches!(
                infos[k].kind,
                LineKind::Push { reg: REG_RBP } | LineKind::Pop { reg: REG_RBP }
            )
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::super::super::types::*;
    use super::*;

    fn build(asm: &str) -> (LineStore, Vec<LineInfo>) {
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        (store, infos)
    }

    /// Address-taken frame slots retain their StoreRbp classification but are
    /// pinned by the real pipeline. Reporting a deletion when mark_nop cannot
    /// remove one falsely prolongs the post-global fixpoint.
    #[test]
    fn windowed_dse_does_not_report_pinned_stores_as_deleted() {
        let asm = "f:\n.cfi_startproc\n    pushq %rbp\n    movq %rsp, %rbp\n    subq $32, %rsp\n    movl %edi, -24(%rbp)\n    movl %esi, -24(%rbp)\n    leaq -24(%rbp), %rcx\n    movq %rbp, %rsp\n    popq %rbp\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        super::super::pin_address_taken_stack_slots(&store, &mut infos);
        let stores: Vec<usize> = (0..store.len())
            .filter(|&i| matches!(infos[i].kind, LineKind::StoreRbp { offset: -24, .. }))
            .collect();
        assert_eq!(stores.len(), 2);
        assert!(stores.iter().all(|&i| infos[i].pinned));
        assert!(!eliminate_dead_stores(&store, &mut infos));
        assert!(stores.iter().all(|&i| !infos[i].is_nop()));
    }

    /// A pinned candidate must not disable real dead-store elimination on an
    /// unrelated, unpinned slot in the same function.
    #[test]
    fn windowed_dse_still_eliminates_unpinned_stores_beside_pinned_ones() {
        let asm = "f:\n.cfi_startproc\n    pushq %rbp\n    movq %rsp, %rbp\n    subq $32, %rsp\n    movl %edi, -24(%rbp)\n    movl %esi, -24(%rbp)\n    movl %edi, -28(%rbp)\n    movl %esi, -28(%rbp)\n    leaq -24(%rbp), %rcx\n    movq %rbp, %rsp\n    popq %rbp\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        super::super::pin_address_taken_stack_slots(&store, &mut infos);
        let stores: Vec<usize> = (0..store.len())
            .filter(|&i| matches!(infos[i].kind, LineKind::StoreRbp { .. }))
            .collect();
        assert_eq!(stores.len(), 4);
        assert!(infos[stores[0]].pinned && infos[stores[1]].pinned);
        assert!(!infos[stores[2]].pinned && !infos[stores[3]].pinned);
        assert!(eliminate_dead_stores(&store, &mut infos));
        assert!(!infos[stores[0]].is_nop() && !infos[stores[1]].is_nop());
        assert!(infos[stores[2]].is_nop() && !infos[stores[3]].is_nop());
        assert!(!eliminate_dead_stores(&store, &mut infos));
    }

    /// A vector spill overwritten by a later spill of the same bytes, with no
    /// load in between, is dead -- the case the candidate arm used to miss
    /// because it matched `StoreRbp` only while the scan below was already
    /// XMM-aware.
    #[test]
    fn windowed_dse_eliminates_dead_vector_slot_store() {
        let asm = "f:\n.cfi_startproc\n    subq $56, %rsp\n    movdqu %xmm0, 16(%rsp)\n    movdqu %xmm1, 16(%rsp)\n    movdqu 16(%rsp), %xmm2\n    addq $56, %rsp\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        let spills: Vec<usize> = (0..store.len())
            .filter(|&i| {
                matches!(
                    infos[i].kind,
                    LineKind::StoreXmmRbp {
                        offset: 16,
                        size: MoveSize::DQ
                    }
                )
            })
            .collect();
        assert_eq!(
            spills.len(),
            2,
            "both spills classify as vector slot stores"
        );
        assert!(spills.iter().all(|&i| !infos[i].pinned));
        assert!(eliminate_dead_stores(&store, &mut infos));
        assert!(infos[spills[0]].is_nop(), "the overwritten spill is dead");
        assert!(!infos[spills[1]].is_nop(), "the reloaded spill survives");
        assert!(!eliminate_dead_stores(&store, &mut infos), "fixpoint");
    }

    /// A general-purpose load of the same bytes keeps a vector store alive: the
    /// scan's reader arm is byte-range based, not family based, so `movq` out of
    /// a slot written by `movdqu` counts as a read of those bytes.
    #[test]
    fn windowed_dse_keeps_vector_store_a_gp_load_reads() {
        let asm = "f:\n.cfi_startproc\n    subq $56, %rsp\n    movdqu %xmm0, 16(%rsp)\n    movq 16(%rsp), %rax\n    addq $56, %rsp\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        let spill = (0..store.len())
            .find(|&i| matches!(infos[i].kind, LineKind::StoreXmmRbp { offset: 16, .. }))
            .expect("the vector spill is in the fixture");
        assert!(matches!(
            infos[spill + 1].kind,
            LineKind::LoadRbp { offset: 16, .. }
        ));
        assert!(!eliminate_dead_stores(&store, &mut infos));
        assert!(!infos[spill].is_nop(), "a GP load reads those bytes");
    }

    /// A call inside the window is a barrier, so a vector store before it is
    /// never retired: the callee may read the slot through a pointer.
    #[test]
    fn windowed_dse_keeps_vector_store_across_a_call() {
        let asm = "f:\n.cfi_startproc\n    subq $56, %rsp\n    movdqu %xmm0, 16(%rsp)\n    call g\n    movdqu 16(%rsp), %xmm2\n    addq $56, %rsp\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        let spill = (0..store.len())
            .find(|&i| matches!(infos[i].kind, LineKind::StoreXmmRbp { offset: 16, .. }))
            .expect("the vector spill is in the fixture");
        assert!(infos[spill + 1].is_barrier(), "call is a barrier");
        assert!(matches!(infos[spill + 1].kind, LineKind::Call));
        assert!(!eliminate_dead_stores(&store, &mut infos));
        assert!(!infos[spill].is_nop());
    }

    /// `%ymm` spills are modelled as QQ (32 bytes), so a 16-byte load of the
    /// same slot reads only the low half and must not retire the wide store.
    #[test]
    fn windowed_dse_models_a_ymm_slot_as_thirty_two_bytes() {
        let asm = "f:\n.cfi_startproc\n    subq $56, %rsp\n    vmovdqu %ymm0, 16(%rsp)\n    movdqu 16(%rsp), %xmm1\n    addq $56, %rsp\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        let spill = (0..store.len())
            .find(|&i| matches!(infos[i].kind, LineKind::StoreXmmRbp { offset: 16, .. }))
            .expect("the wide spill is in the fixture");
        assert_eq!(
            match infos[spill].kind {
                LineKind::StoreXmmRbp { size, .. } => size.byte_size(),
                _ => 0,
            },
            32,
            "a %ymm slot move is QQ, 32 bytes"
        );
        assert!(!eliminate_dead_stores(&store, &mut infos));
        assert!(!infos[spill].is_nop(), "the low-half load reads it");
    }

    /// Width is what makes the kill exact: a narrow store inside a wide one is
    /// dead only because the wide store covers all of its bytes. The trailing
    /// reload is a `LoadXmmRbp`, so it is not in the store list.
    #[test]
    fn windowed_dse_kills_a_vector_store_a_wider_one_covers() {
        let asm = "f:\n.cfi_startproc\n    subq $56, %rsp\n    movsd %xmm0, 16(%rsp)\n    movdqu %xmm1, 16(%rsp)\n    movdqu 16(%rsp), %xmm2\n    addq $56, %rsp\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        let narrow: Vec<usize> = (0..store.len())
            .filter(|&i| {
                matches!(
                    infos[i].kind,
                    LineKind::StoreXmmRbp {
                        offset: 16,
                        size: MoveSize::SD
                    }
                )
            })
            .collect();
        let wide: Vec<usize> = (0..store.len())
            .filter(|&i| {
                matches!(
                    infos[i].kind,
                    LineKind::StoreXmmRbp {
                        offset: 16,
                        size: MoveSize::DQ
                    }
                )
            })
            .collect();
        assert_eq!((narrow.len(), wide.len()), (1, 1));
        assert!(eliminate_dead_stores(&store, &mut infos));
        assert!(infos[narrow[0]].is_nop(), "the 8-byte store is covered");
        assert!(!infos[wide[0]].is_nop(), "the covering store is reloaded");
    }

    /// An address-taken vector slot is pinned by the real pipeline and must not
    /// be reported as deletable, exactly as the GP case above pins.
    /// `pin_address_taken_stack_slots` is textual and kind-agnostic, and
    /// `direct_stack_slot` checks `(%rsp)` before `(%rbp)`, so a `leaq` of an
    /// rsp-relative vector slot pins it.
    #[test]
    fn windowed_dse_does_not_report_pinned_vector_stores_as_deleted() {
        let asm = "f:\n.cfi_startproc\n    subq $56, %rsp\n    movdqu %xmm0, 16(%rsp)\n    movdqu %xmm1, 16(%rsp)\n    leaq 16(%rsp), %rcx\n    movdqu 16(%rsp), %xmm2\n    addq $56, %rsp\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        super::super::pin_address_taken_stack_slots(&store, &mut infos);
        let spills: Vec<usize> = (0..store.len())
            .filter(|&i| matches!(infos[i].kind, LineKind::StoreXmmRbp { offset: 16, .. }))
            .collect();
        assert_eq!(spills.len(), 2);
        assert!(
            spills.iter().all(|&i| infos[i].pinned),
            "an address-taken vector slot is pinned"
        );
        assert!(!eliminate_dead_stores(&store, &mut infos));
        assert!(spills.iter().all(|&i| !infos[i].is_nop()));
    }

    fn keep_first_store_before(middle: &str) {
        let asm = format!(
            "f:\n.cfi_startproc\n    pushq %rbp\n    movq %rsp, %rbp\n    subq $64, %rsp\n    movdqu %xmm0, 16(%rsp)\n{middle}\n    movdqu %xmm1, 16(%rsp)\n    ret\n.cfi_endproc\n"
        );
        let (store, mut infos) = build(&asm);
        let first = (0..store.len())
            .find(|&i| matches!(infos[i].kind, LineKind::StoreXmmRbp { .. }))
            .unwrap();
        eliminate_dead_stores(&store, &mut infos);
        assert!(!infos[first].is_nop(), "must keep store across {middle}");
    }

    #[test]
    fn windowed_dse_does_not_confuse_frame_bases() {
        keep_first_store_before("    movdqu %xmm1, 16(%rbp)\n    movdqu 16(%rsp), %xmm2");
        // Here `%rbp - 48` *is* `%rsp + 16`. Equal offsets are not required for aliasing.
        keep_first_store_before("    movdqu -48(%rbp), %xmm2");
    }

    #[test]
    fn windowed_dse_needs_a_stable_base_and_a_known_read_width() {
        keep_first_store_before("    leaq -16(%rsp), %rsp");
        keep_first_store_before("    xchgq %rsp, %rax");
        keep_first_store_before("    .cfi_endproc");
        keep_first_store_before("    vpaddd (%rsp), %ymm0, %ymm2");
        keep_first_store_before("    movdqu %gs:0x10(%rbp), %xmm2");
    }

    #[test]
    fn windowed_dse_does_not_infer_segment_provenance_from_lea() {
        let asm = "f:\n.cfi_startproc\n    leaq base(%rip), %rbp\n    movdqu %xmm0, 16(%rsp)\n    movdqu %gs:16(%rbp), %xmm2\n    movdqu %xmm1, 16(%rsp)\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        assert!(!eliminate_dead_stores(&store, &mut infos));
        assert!(matches!(infos[3].kind, LineKind::StoreXmmRbp { .. }));
    }

    #[test]
    fn windowed_dse_widens_displacement_endpoint_arithmetic() {
        let asm =
            "f:\n    movdqu %xmm0, 2147483647(%rsp)\n    movdqu %xmm1, 2147483647(%rsp)\n    ret\n";
        let (store, mut infos) = build(asm);
        assert!(eliminate_dead_stores(&store, &mut infos));
        assert!(infos[1].is_nop());
    }

    /// A segment-qualified store is not a frame-slot store, so it is never a
    /// dead-store candidate. Operand shape taken verbatim from
    /// tests/bench/k_seg_gs_copy.c at -O2 (`movdqu %xmm0, %gs:(%rbp)`, which
    /// writes through the pointer in %rbp via the %gs base).  Before
    /// `is_direct_frame_slot` guarded the slot parsers this line classified as
    /// `StoreXmmRbp { offset: 0, size: DQ }` -- `fast_parse_i32("%gs:")` answers
    /// 0 because its first byte is not a digit -- so a real slot store at 0
    /// "covered" it and this pass deleted a `__seg_gs` write.  The window
    /// compares offsets without bases, which is what made the two lines look
    /// like the same slot.
    #[test]
    fn windowed_dse_never_deletes_a_segment_qualified_store() {
        // Two vector spills at 32(%rsp), the first genuinely dead, so the pass
        // has real work to do: the assertion that matters is that it does that
        // work WITHOUT touching the segment store.
        let asm = "f:\n.cfi_startproc\n    subq $56, %rsp\n    movdqu %xmm0, %gs:(%rbp)\n    movdqu %xmm3, 32(%rsp)\n    movdqu %xmm4, 32(%rsp)\n    movdqu 32(%rsp), %xmm2\n    addq $56, %rsp\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        let seg = (0..store.len())
            .find(|&i| infos[i].trimmed(store.get(i)).contains("%gs:"))
            .expect("the segment store is in the fixture");
        assert!(
            !matches!(
                infos[seg].kind,
                LineKind::StoreXmmRbp { .. } | LineKind::StoreRbp { .. }
            ),
            "a segment-qualified operand must not classify as a slot store"
        );
        assert!(
            infos[seg].has_indirect_mem,
            "it must be marked opaque so every offset-tracking pass bails"
        );
        let spills: Vec<usize> = (0..store.len())
            .filter(|&i| matches!(infos[i].kind, LineKind::StoreXmmRbp { offset: 32, .. }))
            .collect();
        assert_eq!(spills.len(), 2, "only the plain spills are slot stores");
        assert!(eliminate_dead_stores(&store, &mut infos));
        assert!(
            !infos[seg].is_nop(),
            "the __seg_gs write must survive: it is not a stack slot"
        );
        assert!(infos[spills[0]].is_nop(), "the first spill is dead");
        assert!(!infos[spills[1]].is_nop(), "the reloaded spill survives");
    }

    /// The windowed DSE models NOP'd lines as absent: a forwarded (NOP'd)
    /// reload does not keep the store alive, and a NOP'd covering store does
    /// not kill an earlier store whose value a later load reads.
    #[test]
    fn windowed_dse_treats_nop_lines_as_absent() {
        let asm = "f:\n.cfi_startproc\n    subq $24, %rsp\n    movq %rdi, 8(%rsp)\n    movq 8(%rsp), %rax\n    movq %rdi, 8(%rsp)\n    movq 8(%rsp), %rcx\n    addq $24, %rsp\n    ret\n.cfi_endproc\n";
        let (store, mut infos) = build(asm);
        let line = |text: &str, nth: usize| {
            (0..store.len())
                .filter(|&i| store.get(i).trim_ascii() == text)
                .nth(nth)
                .unwrap()
        };
        let (first, reload, second, read) = (
            line("movq %rdi, 8(%rsp)", 0),
            line("movq 8(%rsp), %rax", 0),
            line("movq %rdi, 8(%rsp)", 1),
            line("movq 8(%rsp), %rcx", 0),
        );
        // Forwarded reload: the first store is dead (overwritten, unread).
        mark_nop(&mut infos[reload]);
        assert!(eliminate_dead_stores(&store, &mut infos));
        assert!(infos[first].is_nop() && !infos[second].is_nop());
        // A deleted second store is not an overwrite: the first store feeds
        // the final read and must stay.
        let (store, mut infos) = build(asm);
        mark_nop(&mut infos[second]);
        eliminate_dead_stores(&store, &mut infos);
        assert!(!infos[first].is_nop(), "the first store feeds {read}");
    }

    /// A 32-bit `leal` on a DATA-register `%rbp` (no frame pointer) computes an
    /// address and touches no frame slot: it must not be mistaken for a frame
    /// read. Before this test the generic `rbp_offset` arm asked
    /// `frame_read_extent("leal …")`, which has no LEA entry, and bailed the
    /// whole function — one `leal -1(%rbp), %eax` in glibc_strstr's search loop
    /// disabled never-read store elimination and left two dead spill stores in
    /// the hottest block.
    #[test]
    fn leal_on_a_data_register_rbp_does_not_bail_dead_store_elimination() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subq $40, %rsp\n",
            ".cfi_def_cfa_offset 48\n",
            "    leal -1(%rbp), %eax\n",
            "    movq %rax, 8(%rsp)\n",
            "    imull $3, %edi, %edi\n",
            "    addq $40, %rsp\n",
            ".cfi_def_cfa_offset 8\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, mut infos) = build(asm);
        eliminate_never_read_stores(&store, &mut infos);
        let kept: Vec<&str> = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim_ascii())
            .filter(|t| t.contains("(%rsp)"))
            .collect();
        assert!(
            !kept.iter().any(|t| t.starts_with("movq %rax")),
            "a data-register `leal` must not keep the never-read store alive: {kept:?}"
        );
    }

    /// The control for the test above: a genuine memory operand on a
    /// data-register `%rbp` may alias any frame slot, so the never-read store
    /// must stay.
    #[test]
    fn memory_operand_on_a_data_register_rbp_still_bails() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subq $40, %rsp\n",
            ".cfi_def_cfa_offset 48\n",
            "    movl -1(%rbp), %eax\n",
            "    movq %rax, 8(%rsp)\n",
            "    imull $3, %edi, %edi\n",
            "    addq $40, %rsp\n",
            ".cfi_def_cfa_offset 8\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, mut infos) = build(asm);
        eliminate_never_read_stores(&store, &mut infos);
        let kept: Vec<&str> = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim_ascii())
            .filter(|t| t.contains("(%rsp)"))
            .collect();
        assert!(
            kept.iter().any(|t| t.starts_with("movq %rax")),
            "a pointer access through a data-register %rbp can reach any slot: {kept:?}"
        );
    }

    #[test]
    fn never_read_dse_uses_architectural_memory_extents() {
        for (read, offset, kept) in [
            ("fxrstor64 (%rsp)", 272, true),
            ("fxrstorq (%rsp)", 272, true),
            ("fxrstor64 (%rsp)", 688, false),
            ("xrstor64 (%rsp)", 688, true),
            ("xrstors (%rsp)", 688, true),
            ("future_state_read (%rsp)", 688, true),
            ("vmovupd (%rsp), %zmm7", 48, true),
            ("vmovupd (%rsp), %ymm7", 48, false),
            ("ldmxcsr (%rsp)", 48, false),
        ] {
            let candidate = format!("movsd %xmm6, {offset}(%rsp)");
            let asm = format!(
                "f:\n.cfi_startproc\n    subq $1024, %rsp\n    {candidate}\n    {read}\n    addq $1024, %rsp\n    ret\n.cfi_endproc\n.size f, .-f\n"
            );
            let (store, mut infos) = build(&asm);
            let index = (0..store.len())
                .find(|&i| store.get(i).contains(&candidate))
                .unwrap();
            eliminate_never_read_stores(&store, &mut infos);
            assert_eq!(!infos[index].is_nop(), kept, "{read}: {candidate}");
        }
    }

    #[test]
    fn canonical_frame_offsets_and_endpoints_do_not_wrap() {
        assert_eq!(
            canonical_frame_off(b's', i64::from(i32::MIN), 16),
            -2147483664
        );
        assert_eq!(rsp_delta_step("addq $-2147483648, %rsp"), Some(2147483648));
        assert_eq!(rsp_delta_step("addq $-9223372036854775808, %rsp"), None);
        assert!(slot_is_ever_read(
            b's',
            i32::MAX - 3,
            8,
            &[],
            &[(b's', i64::from(i32::MAX))]
        ));
        assert!(!slot_is_ever_read(
            b's',
            i32::MAX - 3,
            8,
            &[],
            &[(b's', i64::from(i32::MAX) + 5)]
        ));
    }

    #[test]
    fn global_dse_does_not_infer_disjointness_from_different_bases() {
        let asm = "f:\n.cfi_startproc\n pushq %rbp\n movq %rsp, %rbp\n subq $32, %rsp\n movq %rdi, -16(%rbp)\n movq 16(%rsp), %rax\n addq $32, %rsp\n popq %rbp\n ret\n.cfi_endproc\n.size f, .-f\n";
        let (store, mut infos) = build(asm);
        let i = (0..store.len())
            .find(|&i| store.get(i).contains("movq %rdi"))
            .unwrap();
        eliminate_never_read_stores(&store, &mut infos);
        assert!(
            !infos[i].is_nop(),
            "the load on rsp can read the store on rbp"
        );
    }

    #[test]
    fn shufps_reads_its_destination_so_the_feeding_move_stays() {
        // `shufps $0x00, %xmm0, %xmm0` takes result lanes 0..1 from the
        // DESTINATION's old value: it is NOT a full def (that is `pshufd`).
        // The `vmovss` reset feeding the splat must survive, or the splat
        // degenerates to [s, s, t, t] (bb_slp_v8 C2 miscompile chain).
        let asm = concat!(
            "f:\n",
            "    vmovss %xmm1, %xmm1, %xmm0
",
            "    shufps $0x00, %xmm0, %xmm0
",
            "    movdqa %xmm0, %xmm14
",
            "    ret
",
        );
        let (store, mut infos) = build(asm);
        let changed = eliminate_dead_fp_moves(&store, &mut infos);
        let kept = (0..store.len())
            .any(|i| !infos[i].is_nop() && store.get(i).contains("vmovss %xmm1, %xmm1, %xmm0"));
        assert!(
            kept,
            "shufps reads its destination; the reset must stay (changed={changed})"
        );
    }

    #[test]
    fn pr814_vex_upper_zero_survives_legacy_xmm_overwrite() {
        for overwrite in [
            "pshufd $0x1b, %xmm5, %xmm7",
            "pshufhw $0x1b, %xmm5, %xmm7",
            "movapd %xmm5, %xmm7",
            "pxor %xmm7, %xmm7",
        ] {
            let asm = format!(
                "f:\n.cfi_startproc\n    vmovapd %xmm6, %xmm7\n    {overwrite}\n    vmovupd %ymm7, (%rdi)\n    ret\n.cfi_endproc\n"
            );
            let (store, mut infos) = build(&asm);
            eliminate_dead_fp_moves(&store, &mut infos);
            assert!(!infos[2].is_nop(), "VEX upper-zero lost across {overwrite}");
        }
    }

    #[test]
    fn dead_vex_moves_require_all_architectural_regions_dead() {
        // Observe each region, with both straight-line and CFG paths. A legacy
        // XMM overwrite kills only the low 128 bits, never the VEX zeroing.
        for width in ["ymm", "zmm"] {
            for path in [
                "    movapd %xmm5, %xmm7\n".to_string(),
                "    testl %eax, %eax\n    je .Luse\n    movapd %xmm5, %xmm7\n.Luse:\n".to_string(),
                "    jmp .Lbody\n.Luse:\n    vmovupd %ymm7, (%rdi)\n    ret\n.Lbody:\n    movapd %xmm5, %xmm7\n    jmp .Luse\n".to_string(),
                format!("    vmovupd %{width}5, %{width}7{{%k1}}\n"),
            ] {
                let asm = format!("f:\n.cfi_startproc\n    vmovapd %xmm6, %xmm7\n{path}    vmovupd %{width}7, (%rdi)\n    ret\n.cfi_endproc\n");
                let (store, mut infos) = build(&asm);
                eliminate_dead_fp_moves(&store, &mut infos);
                assert!(!infos[2].is_nop(), "lost upper {width} state across {path}");
            }
        }
        for kill in [
            "vmovapd %xmm5, %xmm7",
            "vmovupd %ymm5, %ymm7",
            "vmovupd %zmm5, %zmm7",
            "vpxor %ymm7, %ymm7, %ymm7",
            "vzeroall",
            "vzeroupper\n    movapd %xmm5, %xmm7",
        ] {
            let asm = format!(
                "f:\n.cfi_startproc\n    vmovapd %xmm6, %xmm7\n    {kill}\n    vmovupd %zmm7, (%rdi)\n    ret\n.cfi_endproc\n"
            );
            let (store, mut infos) = build(&asm);
            assert!(
                eliminate_dead_fp_moves(&store, &mut infos),
                "missed full kill {kill}"
            );
            assert!(infos[2].is_nop(), "dead move retained across {kill}");
        }
    }

    #[test]
    fn low_width_legacy_moves_do_not_define_upper_regions() {
        let (store, mut infos) = build(
            "f:\n.cfi_startproc\n    movapd %xmm6, %xmm7\n    movapd %xmm5, %xmm7\n    vmovupd %zmm7, (%rdi)\n    ret\n.cfi_endproc\n",
        );
        assert!(eliminate_dead_fp_moves(&store, &mut infos));
        assert!(
            infos[2].is_nop(),
            "upper observation must not block legacy low-only DCE"
        );
    }

    #[test]
    fn vex_flag_only_tests_read_both_vector_operands() {
        for test in [
            "vucomisd %xmm5, %xmm7",
            "vucomiss %xmm5, %xmm7",
            "vcomisd %xmm5, %xmm7",
            "vcomiss %xmm5, %xmm7",
            "vptest %ymm5, %ymm7",
            "vtestpd %ymm5, %ymm7",
            "vtestps %ymm5, %ymm7",
            "vpcmpistri $0, %xmm5, %xmm7",
            "vpcmpistrm $0, %xmm5, %xmm7",
            "vpcmpestri $0, %xmm5, %xmm7",
            "vpcmpestrm $0, %xmm5, %xmm7",
        ] {
            let asm = format!(
                "f:\n.cfi_startproc\n    vmovapd %xmm6, %xmm7\n    {test}\n    setne %al\n    movzbl %al, %eax\n    ret\n.cfi_endproc\n"
            );
            let (store, mut infos) = build(&asm);
            assert!(
                !eliminate_dead_fp_moves(&store, &mut infos),
                "flag input deleted before {test}"
            );
            assert!(!infos[2].is_nop(), "flag input deleted before {test}");
        }
    }

    #[test]
    fn pr814_pinned_dead_fp_move_is_not_changed() {
        let (store, mut infos) = build("f:\n    vmovsd %xmm6, %xmm6, %xmm7\n    ret\n");
        infos[1].pinned = true;
        assert!(!eliminate_dead_fp_moves(&store, &mut infos));
        assert!(infos[1].pinned && !infos[1].is_nop());
    }

    #[test]
    fn dead_fp_staging_copy_is_killed() {
        // The nbody extract-staging idiom: lane-0 copy written and never read
        // before the next full def of the same register.
        let asm = concat!(
            "f:\n",
            "    movapd %xmm15, %xmm0\n",
            "    pshufd $0x0E, %xmm15, %xmm0\n",
            "    movsd %xmm0, %xmm1\n",
            "    ret\n",
        );
        let (store, mut infos) = build(asm);
        assert!(eliminate_dead_fp_moves(&store, &mut infos));
        let at = |s: &str| {
            (0..store.len())
                .find(|&i| store.get(i).contains(s))
                .unwrap()
        };
        assert!(
            infos[at("movapd")].is_nop(),
            "the dead movapd staging copy must go"
        );
        assert!(!infos[at("pshufd")].is_nop(), "the live pshufd stays");
    }

    #[test]
    fn live_fp_move_is_kept() {
        let asm = concat!(
            "f:\n",
            "    movapd %xmm15, %xmm0\n",
            "    addsd %xmm0, %xmm1\n",
            "    ret\n",
        );
        let (store, mut infos) = build(asm);
        eliminate_dead_fp_moves(&store, &mut infos);
        let at = |s: &str| {
            (0..store.len())
                .find(|&i| store.get(i).contains(s))
                .unwrap()
        };
        assert!(
            !infos[at("movapd")].is_nop(),
            "the move feeds addsd — it is live"
        );
    }

    #[test]
    fn partial_fp_move_needs_full_death() {
        // `movsd` merges into the low 64 of the destination: the upper bits
        // survive, so a later read of ANY xmm0 bit keeps the move. Here
        // `movaps` reads all of xmm0 after the merge copy — the copy stays.
        let asm = concat!(
            "f:\n",
            "    movsd %xmm6, %xmm0\n",
            "    movaps %xmm0, %xmm2\n",
            "    ret\n",
        );
        let (store, mut infos) = build(asm);
        eliminate_dead_fp_moves(&store, &mut infos);
        let at = |s: &str| {
            (0..store.len())
                .find(|&i| store.get(i).contains(s))
                .unwrap()
        };
        assert!(
            !infos[at("movsd %xmm6")].is_nop(),
            "xmm0 is read before its next def"
        );
    }

    #[test]
    fn memory_operands_are_not_fp_reg_moves() {
        let asm = concat!(
            "f:\n",
            "    movsd %xmm6, 8(%rsp)\n",
            "    movsd 8(%rsp), %xmm0\n",
            "    ret\n",
        );
        let (store, mut infos) = build(asm);
        eliminate_dead_fp_moves(&store, &mut infos);
        let at = |s: &str| {
            (0..store.len())
                .find(|&i| store.get(i).contains(s))
                .unwrap()
        };
        assert!(
            !infos[at("movsd %xmm6")].is_nop() && !infos[at("movsd 8(%rsp)")].is_nop(),
            "slot traffic untouched"
        );
    }

    #[test]
    fn merge_copy_into_dead_dest_is_killed() {
        // store forwarding's 3-operand vmovsd form into a destination whose
        // bits are never observed.
        let asm = concat!("f:\n", "    vmovsd %xmm6, %xmm6, %xmm7\n", "    ret\n",);
        let (store, mut infos) = build(asm);
        assert!(eliminate_dead_fp_moves(&store, &mut infos));
        let at = |s: &str| {
            (0..store.len())
                .find(|&i| store.get(i).contains(s))
                .unwrap()
        };
        assert!(infos[at("vmovsd")].is_nop(), "dead merge copy must go");
    }

    /// Upward-exposed slot reads: a load fully covered by an earlier store
    /// in its own straight-line run observes only that store, so it no
    /// longer pins every other store overlapping the slot (the never-read
    /// `movl %eax, 68(%rsp)` in `zlib_ng_adler32_combine`'s hot loop). Each
    /// negative is one condition under which the backward view is
    /// incomplete; the probe store must then survive exactly as before.
    #[test]
    fn never_read_dse_attributes_locally_covered_loads() {
        let probe = "movl %eax, 68(%rsp)";
        for (name, reuse, deleted) in [
            (
                "full cover",
                "    movq %rax, 64(%rsp)\n    movl %r14d, %r10d\n    movq 64(%rsp), %rdi\n",
                true,
            ),
            (
                "two writers cover",
                "    movl %eax, 64(%rsp)\n    movl %ecx, 68(%rsp)\n    movq 64(%rsp), %rdi\n",
                true,
            ),
            (
                "indirect non-frame store is stepped over",
                "    movq %rax, 64(%rsp)\n    movq %rcx, (%rdx)\n    movq 64(%rsp), %rdi\n",
                true,
            ),
            (
                "partial cover",
                "    movl %eax, 64(%rsp)\n    movq 64(%rsp), %rdi\n",
                false,
            ),
            (
                "label splits the run",
                "    movq %rax, 64(%rsp)\n.Ljoin:\n    movq 64(%rsp), %rdi\n",
                false,
            ),
            (
                "call splits the run",
                "    movq %rax, 64(%rsp)\n    call g\n    movq 64(%rsp), %rdi\n",
                false,
            ),
            (
                "unmodelled frame writer",
                "    movq %rax, 64(%rsp)\n    movl $7, 68(%rsp)\n    movq 64(%rsp), %rdi\n",
                false,
            ),
            ("no covering store", "    movq 64(%rsp), %rdi\n", false),
        ] {
            let asm = format!(
                "f:\n.cfi_startproc\n    subq $128, %rsp\n.Lloop:\n    imull $65521, %eax, %eax\n    {probe}\n    subl %eax, %r15d\n    jne .Lloop\n.Lnext:\n{reuse}    addq $128, %rsp\n    ret\n.cfi_endproc\n.size f, .-f\n"
            );
            let (store, mut infos) = build(&asm);
            let at = |needle: &str| {
                (0..store.len())
                    .find(|&i| store.get(i).contains(needle))
                    .unwrap()
            };
            let p = at(probe);
            eliminate_never_read_stores(&store, &mut infos);
            assert_eq!(infos[p].is_nop(), deleted, "{name}: probe store");
            // The store a covered load reads is never deleted.
            if let Some(c) =
                (0..store.len()).find(|&i| store.get(i).contains("movq %rax, 64(%rsp)"))
            {
                assert!(!infos[c].is_nop(), "{name}: the covering store is read");
            }
            if name == "two writers cover" {
                assert!(
                    !infos[at("movl %eax, 64(%rsp)")].is_nop(),
                    "{name}: low writer read"
                );
                assert!(
                    !infos[at("movl %ecx, 68(%rsp)")].is_nop(),
                    "{name}: high writer read"
                );
            }
        }
    }

    /// Run never-read DSE and report whether the line matching `text` was
    /// NOP'd (exact trimmed match, first occurrence).
    fn dse_removed(asm: &str, text: &str) -> bool {
        let (store, mut infos) = build(asm);
        let idx = (0..store.len())
            .find(|&i| store.get(i).trim_ascii() == text)
            .expect("probe line present");
        eliminate_never_read_stores(&store, &mut infos);
        infos[idx].is_nop()
    }

    /// The hidden `%ymm` return-buffer / argument-home store of a vector
    /// round-trip caller (`v8 f(v8 a){ return ext8(a); }`): once the paired
    /// reload is forwarded away, the 32-byte store is never read and must
    /// be deleted — before `MoveSize::QQ`, the line classified `Other` and
    /// registered a conservative READ of its own slot, pinning itself.
    #[test]
    fn dse_removes_never_read_ymm_store() {
        let asm = "f:\n\
.cfi_startproc\n\
    subq $136, %rsp\n\
    vmovdqu %ymm0, 96(%rsp)\n\
    call ext8@PLT\n\
    addq $136, %rsp\n\
    ret\n\
.cfi_endproc\n";
        assert!(dse_removed(asm, "vmovdqu %ymm0, 96(%rsp)"));
        // Mutation 1: a 32-byte reload of the slot is a read — kept.
        let read = asm.replace(
            "call ext8@PLT\n",
            "call ext8@PLT\n    vmovdqu 96(%rsp), %ymm1\n",
        );
        assert!(!dse_removed(&read, "vmovdqu %ymm0, 96(%rsp)"));
        // Mutation 2: a SCALAR read inside the upper half of the 32-byte
        // range observes the store — byte-range, not point, semantics.
        let tail = asm.replace(
            "call ext8@PLT\n",
            "call ext8@PLT\n    movq 120(%rsp), %rax\n",
        );
        assert!(!dse_removed(&tail, "vmovdqu %ymm0, 96(%rsp)"));
        // Mutation 3: a frame-address escape bails the whole function.
        let escape = asm.replace(
            "call ext8@PLT\n",
            "leaq 96(%rsp), %rdi\n    call ext8@PLT\n",
        );
        assert!(!dse_removed(&escape, "vmovdqu %ymm0, 96(%rsp)"));
    }

    /// A `24(%rsp)` read inside a 16-byte push window (delta +16) addresses
    /// the same slot as the canonical `8(%rsp)` store: the store must be
    /// kept, while an unrelated never-read vector store is still removed.
    /// Mirrors the variadic `loop_patterns` region.
    #[test]
    fn dse_push_window_read_translates_to_canonical_slot() {
        let asm = "f:\n\
.cfi_startproc\n\
    pushq %rbp\n\
    subq $48, %rsp\n\
    movq %rax, 8(%rsp)\n\
    movdqu %xmm0, 16(%rsp)\n\
    pushq %rcx\n\
    pushq %rdx\n\
    movq 24(%rsp), %rdi\n\
    call printf@PLT\n\
    addq $16, %rsp\n\
    addq $48, %rsp\n\
    popq %rbp\n\
    ret\n\
.cfi_endproc\n";
        // The shifted read keeps the canonical store alive; the dead vector
        // store is removed.
        assert!(!dse_removed(asm, "movq %rax, 8(%rsp)"));
        assert!(dse_removed(asm, "movdqu %xmm0, 16(%rsp)"));
        // Mutation: point the shifted read at an unmapped canonical slot
        // (40-16=24); the 8(%rsp) store is now genuinely dead and is removed.
        let mutated = asm.replace("movq 24(%rsp), %rdi", "movq 40(%rsp), %rdi");
        assert!(dse_removed(&mutated, "movq %rax, 8(%rsp)"));
    }

    /// A conditional jump at non-canonical rsp depth means not every
    /// successor enters at canonical depth; DSE must bail and keep stores.
    /// The push that opens the delta must sit in the body (prologue shifts
    /// precede the DSE body window and are not re-counted).
    #[test]
    fn dse_bails_on_unbalanced_conditional_jump() {
        let asm = "g:\n\
.cfi_startproc\n\
    pushq %rbp\n\
    subq $32, %rsp\n\
    movl %eax, 8(%rsp)\n\
    pushq %rcx\n\
    je .L1\n\
.L1:\n\
    addq $8, %rsp\n\
    addq $32, %rsp\n\
    popq %rbp\n\
    ret\n\
.cfi_endproc\n";
        assert!(!dse_removed(asm, "movl %eax, 8(%rsp)"));
        // Mutation: balance the push window before the terminator so delta is
        // zero at the je; no bail, and the dead store is removed.
        let mutated = asm.replace(
            "pushq %rcx\nje .L1\n",
            "pushq %rcx\naddq $8, %rsp\nje .L1\n",
        );
        assert!(dse_removed(&mutated, "movl %eax, 8(%rsp)"));
    }

    /// An SIB-form frame access while rsp is shifted cannot be mapped to a
    /// canonical slot: DSE must bail rather than mis-alias.
    #[test]
    fn dse_bails_on_sib_frame_access_at_shifted_depth() {
        let asm = "h:\n\
.cfi_startproc\n\
    subq $32, %rsp\n\
    movl %eax, 8(%rsp)\n\
    pushq %rbx\n\
    movl 32(%rsp,%rax,4), %edi\n\
    addq $8, %rsp\n\
    addq $32, %rsp\n\
    ret\n\
.cfi_endproc\n";
        assert!(!dse_removed(asm, "movl %eax, 8(%rsp)"));
        // Mutation: perform the SIB read at canonical depth (before the
        // push). Its unbounded-upward floor starts at slot 32, so the dead
        // slot-8 store below the floor is still removed.
        let mutated = asm.replace(
            "pushq %rbx\nmovl 32(%rsp,%rax,4), %edi\n",
            "movl 32(%rsp,%rax,4), %edi\npushq %rbx\n",
        );
        assert!(dse_removed(&mutated, "movl %eax, 8(%rsp)"));
    }

    /// A label inside an open push/pop window (a loop back-edge target, as in
    /// cmpxchg retry loops) must NOT reset the linear rsp delta: every edge
    /// reaching the label shares the shifted depth, so a textual frame
    /// operand still translates by the carried delta. Resetting here would
    /// mis-attribute the post-pop read and delete a live store.
    #[test]
    fn dse_carries_rsp_delta_across_loop_label_in_push_window() {
        let asm = "k:\n\
.cfi_startproc\n\
    subq $32, %rsp\n\
    pushq %rbx\n\
.L1:\n\
    movl 16(%rsp), %edi\n\
    movl %eax, 8(%rsp)\n\
    addq $32, %rsp\n\
    ret\n\
.cfi_endproc\n";
        // The +8 delta carries across the loop label: the textual 16(%rsp)
        // read maps to canonical slot 8 and keeps the slot-8 store alive.
        assert!(!dse_removed(asm, "movl %eax, 8(%rsp)"));
        // Mutation: balance the push before the label so the read really
        // targets canonical slot 16; the slot-8 store is then dead and goes.
        let mutated = asm.replace("pushq %rbx\n.L1:\n", "pushq %rbx\naddq $8, %rsp\n.L1:\n");
        assert!(dse_removed(&mutated, "movl %eax, 8(%rsp)"));
    }

    /// A label reached only through a branch after the linear path ended in
    /// a `ret` starts a block that codegen enters at canonical rsp depth, even
    /// though the preceding block's epilogue teardown left the linear delta
    /// negative. The block's frame reads must map to their textual slots
    /// (cpuid-output spill shape: stores kept, read in the post-`ret` block).
    #[test]
    fn dse_resets_delta_at_label_after_ret_for_branch_target() {
        let asm = "f:\n\
.cfi_startproc\n\
    subq $32, %rsp\n\
    movl %eax, 16(%rsp)\n\
    jc .L2\n\
.L1:\n\
    addq $32, %rsp\n\
    ret\n\
.L2:\n\
    movl 16(%rsp), %edx\n\
    jmp .L1\n\
.cfi_endproc\n";
        // The read in the post-ret branch block keeps the slot-16 store once
        // the epilogue's -32 delta is reset at the .L2 label.
        assert!(!dse_removed(asm, "movl %eax, 16(%rsp)"));
        // Mutation: drop the `ret` so .L2 is a fall-through label that keeps
        // the -32 delta; the read mis-maps to slot 48 and the store is deleted.
        let mutated = asm.replace("ret\n.L2:\n", ".L2:\n");
        assert!(dse_removed(&mutated, "movl %eax, 16(%rsp)"));
    }

    /// A `pushq MEM(%rsp)` both shifts rsp and reads a frame slot at the
    /// pre-shift depth; DSE must fail closed instead of treating it as a
    /// bare +8 step and dropping the read (variadic push-home idiom).
    #[test]
    fn dse_bails_on_memory_form_push() {
        let asm = "g:\n\
.cfi_startproc\n\
    subq $32, %rsp\n\
    movq %rax, 0(%rsp)\n\
    pushq 0(%rsp)\n\
    popq %rdx\n\
    addq $32, %rsp\n\
    ret\n\
.cfi_endproc\n";
        assert!(!dse_removed(asm, "movq %rax, 0(%rsp)"));
        // Mutation: a register push carries no frame read, so the slot-0 store
        // is genuinely never observed and is removed.
        let mutated = asm.replace("pushq 0(%rsp)\n", "pushq %rax\n");
        assert!(dse_removed(&mutated, "movq %rax, 0(%rsp)"));
    }

    /// An unconditional `jmp` reached at non-canonical depth whose target block
    /// performs a frame-slot access is unanalyzable (the block's true entry
    /// depth cannot be reconstructed linearly): DSE must fail closed rather
    /// than reset the delta and mis-attribute the target block's reads.
    #[test]
    fn dse_bails_on_shifted_jmp_into_frame_accessing_block() {
        let asm = "h:\n\
.cfi_startproc\n\
    subq $32, %rsp\n\
    movl %eax, 16(%rsp)\n\
    pushq %rbx\n\
    jmp .L2\n\
.L2:\n\
    movl 16(%rsp), %edi\n\
    popq %rbx\n\
    addq $32, %rsp\n\
    ret\n\
.cfi_endproc\n";
        assert!(!dse_removed(asm, "movl %eax, 16(%rsp)"));
        // Mutation: turn the shifted jump into fall-through (drop the jmp); the
        // label no longer resets the delta, the read mis-maps and the store is
        // deleted — proving the guard is what keeps it.
        let mutated = asm.replace("jmp .L2\n", "");
        assert!(dse_removed(&mutated, "movl %eax, 16(%rsp)"));
    }

    /// A vector spill whose slot is rewritten at the loop head before it is
    /// read is dead on the back-edge. The store at the bottom of the loop
    /// survives the windowed pass (a `jl` ends the window) and the address-global
    /// pass (the slot is read at the head), so only the per-path oracle removes
    /// it. This is the shape `vector_pool.rs` sharing produces for
    /// classify-style loops (`test_classify_predicate_codegen`: 13 spills
    /// against a ceiling of 12).
    #[test]
    fn vector_store_overwritten_on_the_back_edge_is_removed() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subq $40, %rsp\n",
            ".cfi_def_cfa_offset 48\n",
            "    xorl %ecx, %ecx\n",
            ".LB:\n",
            "    movdqu %xmm0, 16(%rsp)\n",
            "    movdqu 16(%rsp), %xmm1\n",
            "    paddb %xmm1, %xmm2\n",
            "    movdqu %xmm2, 16(%rsp)\n",
            "    addl $1, %ecx\n",
            "    cmpl $4, %ecx\n",
            "    jl .LB\n",
            "    addq $40, %rsp\n",
            ".cfi_def_cfa_offset 8\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, mut infos) = build(asm);
        assert!(eliminate_flow_dead_vector_stores(&store, &mut infos));
        let kept: Vec<&str> = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim_ascii())
            .filter(|t| t.starts_with("movdqu") && t.contains("(%rsp)"))
            .collect();
        assert_eq!(
            kept,
            vec!["movdqu %xmm0, 16(%rsp)", "movdqu 16(%rsp), %xmm1"],
            "only the store that the back-edge overwrites may be removed"
        );
    }

    /// The control: the same store is live when the slot is read at the loop
    /// head, because the value written at the bottom flows around the back-edge
    /// into that read. The pass must keep it.
    #[test]
    fn vector_store_read_after_the_back_edge_is_kept() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subq $40, %rsp\n",
            ".cfi_def_cfa_offset 48\n",
            "    xorl %ecx, %ecx\n",
            ".LB:\n",
            "    movdqu 16(%rsp), %xmm1\n",
            "    paddb %xmm1, %xmm2\n",
            "    movdqu %xmm2, 16(%rsp)\n",
            "    addl $1, %ecx\n",
            "    cmpl $4, %ecx\n",
            "    jl .LB\n",
            "    addq $40, %rsp\n",
            ".cfi_def_cfa_offset 8\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (store, mut infos) = build(asm);
        assert!(!eliminate_flow_dead_vector_stores(&store, &mut infos));
        assert!(
            (0..store.len()).any(
                |i| !infos[i].is_nop() && store.get(i).trim_ascii() == "movdqu %xmm2, 16(%rsp)"
            ),
            "a store that reaches a read through the back-edge is live"
        );
    }
}
