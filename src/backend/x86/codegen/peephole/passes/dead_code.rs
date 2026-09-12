//! Dead code elimination passes.
//!
//! Three passes that eliminate dead instructions:
//! - `eliminate_dead_reg_moves`: removes reg-to-reg moves where the destination
//!   is overwritten before being read (local, forward scan within a window).
//! - `eliminate_dead_stores`: removes stores to stack slots that are overwritten
//!   before being read (local, 16-instruction window).
//! - `eliminate_never_read_stores`: removes stores to stack slots that are never
//!   read anywhere in the function (global, whole-function analysis).

use super::super::types::*;
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
        if dst_reg == 4 || dst_reg == 5 {
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

        // Scan forward within the same basic block.
        let mut j = i + 1;
        let scan_end = (i + DEAD_MOVE_WINDOW).min(len);
        while j < scan_end {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }

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

// ── Dead store elimination (local, windowed) ─────────────────────────────────

pub(super) fn eliminate_dead_stores(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();
    const WINDOW: usize = 64;

    let mut pattern_bytes = [0u8; 24];

    // Frame-pointer status: reset at each .cfi_startproc, set true when a
    // function establishes a frame pointer (`movq %rsp, %rbp`). When %rbp is NOT
    // the frame pointer, `offset(%rbp)` accesses are pointer dereferences and
    // must never be treated as stack slots.
    let mut rbp_is_frame = false;

    for i in 0..len {
        match infos[i].kind {
            LineKind::Directive => {
                let d = infos[i].trimmed(store.get(i));
                if d == ".cfi_startproc" {
                    rbp_is_frame = false;
                }
            }
            LineKind::Other { .. } => {
                let o = infos[i].trimmed(store.get(i));
                if o == "movq %rsp, %rbp" || o == "movl %esp, %ebp" {
                    rbp_is_frame = true;
                }
            }
            _ => {}
        }

        let (store_offset, store_size) = match infos[i].kind {
            LineKind::StoreRbp { offset, size, .. } => (offset, size),
            _ => continue,
        };

        // SOUNDNESS: if %rbp is not the frame pointer and this store uses an
        // `(%rbp)` memory operand, it writes ARBITRARY memory (pointer deref), not
        // a stack slot. Never delete it as a dead stack store.
        if !rbp_is_frame && uses_rbp_mem_operand(infos[i].trimmed(store.get(i))) {
            continue;
        }

        let store_bytes = store_size.byte_size();

        let end = std::cmp::min(i + WINDOW, len);
        let mut slot_read = false;
        let mut slot_overwritten = false;
        let mut pattern_len: usize = 0;

        for j in (i + 1)..end {
            if infos[j].is_nop() {
                // ms178 SOUNDNESS FIX: another pass (store_fwd / copy_prop /
                // dead_regs / phase-1 locals) may have NOP'd this line, but the
                // LineStore preserves its original text. If that line was a LOAD
                // or STORE of OUR slot, then our store's value was consumed (or
                // the slot was rewritten) — deleting our store would orphan the
                // forwarded value / reintroduce a live store. Check the
                // preserved text (trimmed: NOP'd lines may retain their original
                // indentation, which the line parsers do not tolerate): any
                // same-slot access on a nop'd line counts as a read
                // (conservative) or overwrite.
                let nt = infos[j].trimmed(store.get(j)).trim();
                if let Some((off_str, _reg, sz)) = parse_load_from_rbp_str(nt) {
                    let off = fast_parse_i32(off_str);
                    if ranges_overlap(store_offset, store_bytes, off, sz.byte_size()) {
                        slot_read = true;
                        break;
                    }
                }
                if let Some((_reg, off_str, sz)) = parse_store_to_rbp_str(nt) {
                    let off = fast_parse_i32(off_str);
                    let nb = sz.byte_size();
                    if off <= store_offset && off + nb >= store_offset + store_bytes {
                        slot_overwritten = true;
                        break;
                    }
                    if ranges_overlap(store_offset, store_bytes, off, nb) {
                        slot_read = true;
                        break;
                    }
                }
                continue;
            }

            if infos[j].is_barrier() {
                slot_read = true;
                break;
            }

            // SOUNDNESS: user inline asm may read this slot through an "m"
            // operand (`movq -16(%rbp), %rax` inside the template) that the
            // line parsers never see — InlineAsm lines carry no parsed
            // offset. Skipping the line would let the store be deleted as
            // "overwritten, never read" while the asm still reads it (and
            // vice versa: a later asm read keeps an earlier store alive).
            // Treat any inline-asm line in the window as a potential reader
            // of every slot.
            if infos[j].kind == LineKind::InlineAsm {
                slot_read = true;
                break;
            }

            // SOUNDNESS: an RSP-shifting line between the store and this
            // point makes the store's %rsp-relative address ambiguous (the
            // offsets inside the shifted window refer to different physical
            // slots). Treat as a read so the store is never deleted.
            if is_rsp_shift_line(infos[j].trimmed(store.get(j))) {
                slot_read = true;
                break;
            }

            // SOUNDNESS: a `(%rbp)` access when %rbp is not the frame pointer
            // is an opaque pointer dereference that may READ or WRITE the slot.
            // Conservatively treat it as a read.
            if !rbp_is_frame && uses_rbp_mem_operand(infos[j].trimmed(store.get(j))) {
                slot_read = true;
                break;
            }

            if let LineKind::LoadRbp {
                offset: load_off,
                size: load_sz,
                ..
            } = infos[j].kind
            {
                if ranges_overlap(store_offset, store_bytes, load_off, load_sz.byte_size()) {
                    slot_read = true;
                    break;
                }
            }

            if let LineKind::StoreRbp {
                offset: new_off,
                size: new_sz,
                ..
            } = infos[j].kind
            {
                let new_bytes = new_sz.byte_size();
                if new_off <= store_offset && new_off + new_bytes >= store_offset + store_bytes {
                    slot_overwritten = true;
                    break;
                }
                if ranges_overlap(store_offset, store_bytes, new_off, new_bytes) {
                    slot_read = true;
                    break;
                }
            }

            // Scalar-FP / SSE slot traffic participates in the same byte-range
            // model. A GP store that is reloaded into an XMM register through
            // the slot (`movq %rax, -72(%rbp)` / `movss -72(%rbp), %xmm0` —
            // the -O0 int<->float bit-transfer idiom) is a live store; an XMM
            // store that fully covers our bytes kills it exactly like a GP one.
            if let LineKind::LoadXmmRbp {
                offset: load_off,
                size: load_sz,
            } = infos[j].kind
            {
                if ranges_overlap(store_offset, store_bytes, load_off, load_sz.byte_size()) {
                    slot_read = true;
                    break;
                }
            }
            if let LineKind::StoreXmmRbp {
                offset: new_off,
                size: new_sz,
            } = infos[j].kind
            {
                let new_bytes = new_sz.byte_size();
                if new_off <= store_offset && new_off + new_bytes >= store_offset + store_bytes {
                    slot_overwritten = true;
                    break;
                }
                if ranges_overlap(store_offset, store_bytes, new_off, new_bytes) {
                    slot_read = true;
                    break;
                }
            }

            // Check Other and Cmp lines for rbp references. Cmp lines can
            // have memory operands after memory fold (e.g., cmpq -N(%rbp), %rax).
            if matches!(infos[j].kind, LineKind::Other { .. } | LineKind::Cmp) {
                if infos[j].has_indirect_mem {
                    slot_read = true;
                    break;
                }
                let rbp_off = infos[j].rbp_offset;
                if rbp_off != RBP_OFFSET_NONE {
                    // PACKED vector moves are RANGE accesses: movdqu et al.
                    // read/write [rbp_offset, rbp_offset + 16) (32 for %ymm),
                    // but the cached offset alone credits only the low 8
                    // bytes. A scalar store fully inside the upper half of a
                    // `movdqu` copy's read range was elided because the point
                    // model missed the read (the i128 parameter's high-half
                    // home store; the wide condition test then read stale
                    // frame bytes). Overlap of the vector extent with the
                    // store's range is a read. Same range-vs-point model
                    // store_forwarding applies (16-byte invalidate).
                    if let Some(ext) = vector_frame_move_extent(infos[j].trimmed(store.get(j))) {
                        if ranges_overlap(store_offset, store_bytes, rbp_off, ext) {
                            slot_read = true;
                            break;
                        }
                    }
                    if rbp_off >= store_offset && rbp_off < store_offset + store_bytes {
                        slot_read = true;
                        break;
                    }
                    if rbp_off < store_offset && rbp_off + 8 > store_offset {
                        slot_read = true;
                        break;
                    }
                    continue;
                }
                if pattern_len == 0 {
                    pattern_len = write_rbp_pattern(&mut pattern_bytes, store_offset);
                }
                let pattern = std::str::from_utf8(&pattern_bytes[..pattern_len])
                    .expect("rbp pattern produced non-UTF8");
                let line = infos[j].trimmed(store.get(j));
                if line.contains(pattern) {
                    slot_read = true;
                    break;
                }
                if store_bytes > 1 {
                    let mut sub_pattern_bytes = [0u8; 24];
                    for byte_off in 1..store_bytes {
                        let check_off = store_offset + byte_off;
                        let check_len = write_rbp_pattern(&mut sub_pattern_bytes, check_off);
                        let check_pattern = std::str::from_utf8(&sub_pattern_bytes[..check_len])
                            .expect("rbp pattern produced non-UTF8");
                        let line = infos[j].trimmed(store.get(j));
                        if line.contains(check_pattern) {
                            slot_read = true;
                            break;
                        }
                    }
                    if slot_read {
                        break;
                    }
                }
            }
        }

        if slot_overwritten && !slot_read {
            mark_nop(&mut infos[i]);
            changed = true;
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
) -> Option<(i32, i32)> {
    // Parse "disp(%base,%idx,scale)".
    let lp = line.find('(')?;
    let rp = line[lp..].find(')')? + lp;
    let inside = &line[lp + 1..rp];
    let mut fields = inside.split(',');
    let _base = fields.next()?;
    let idx = fields.next()?.trim();
    let scale: i32 = match fields.next().map(str::trim) {
        None | Some("") => 1,
        Some(s) => s.parse().ok()?,
    };
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
    let mut label_pos: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (li, info) in infos.iter().enumerate() {
        if matches!(info.kind, LineKind::Label) {
            let t = info.trimmed(store.get(li));
            label_pos.entry(t.trim_end_matches(':')).or_insert(li);
        }
    }

    /// Parse `$N` (decimal or 0x hex) into a positive i64.
    fn pos_imm(s: &str) -> Option<i64> {
        let s = s.trim();
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
                        if register_family_fast(dst.trim()) == family {
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
                    if register_family_fast(dst.trim()) == family {
                        let open = mem.find('(')?;
                        let close = mem.rfind(')')?;
                        let inside = &mem[open + 1..close];
                        let mut comps = inside.split(',');
                        let base = comps.next().unwrap_or("").trim();
                        let stride_idx = comps.next().map(str::trim);
                        let stride = if let Some(si) = stride_idx {
                            if !si.is_empty() && si != idx {
                                false
                            } else {
                                let sc = comps.next().map(str::trim).unwrap_or("1");
                                sc.parse::<i64>().map(|v| v >= 1).unwrap_or(false)
                                    && (si.is_empty() || si == idx)
                            }
                        } else {
                            true
                        };
                        let disp_txt = mem[..open].trim();
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
            if register_family_fast(cmp_reg.trim()) == family {
                let imm: i32 = imm_str.trim().parse().ok()?;
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
                p.trim()
                    .trim_start_matches("movq $")
                    .trim_start_matches("movl $")
                    .parse::<i32>()
                    .ok()
            })
        } else if t.starts_with("xorl ") || t.starts_with("xorq ") {
            let ps: Vec<&str> = t.split(',').map(str::trim).collect();
            (ps.len() == 2 && ps[0] == ps[1]).then_some(0)
        } else {
            None
        };
        if let Some(a) = a {
            lower = Some(lower.map_or(a, |l| l.min(a)));
        }
    }
    let a = lower?;

    let lo = fm.disp + a * scale;
    let hi = fm.disp + b * scale + width;
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
) -> Option<(u8, i32, i32, bool)> {
    let fm = parse_frame_mem(line)?;
    if !fm.sib {
        return None;
    }
    if let Some((off, len)) = sib_frame_read_range(store, infos, use_idx, line, fm) {
        return Some((fm.base, off, len, false));
    }
    if fm.disp >= 0 {
        Some((fm.base, fm.disp, 0, true))
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
    read_ranges: &[(u8, i32, i32)],
    read_floors: &[(u8, i32)],
) -> bool {
    read_ranges
        .iter()
        .any(|&(rb, ro, rs)| rb == base && ranges_overlap(offset, store_bytes, ro, rs))
        || read_floors
            .iter()
            .any(|&(rb, d)| rb == base && offset + store_bytes > d)
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
        let mut rbp_is_frame;

        if matches!(infos[i].kind, LineKind::Push { reg: 5 }) {
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
                if matches!(infos[k].kind, LineKind::Directive) {
                    let dl = infos[k].trimmed(store.get(k));
                    if dl.contains("cfi_startproc") || dl.contains("cfi_def_cfa_offset") {
                        found_cfi = true;
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
            // Skip .cfi_def_cfa_offset directive if present
            if j < len && matches!(infos[j].kind, LineKind::Directive) {
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
        // Pre-scan for escape events before collecting reads.
        let mut has_unparseable_indirect = false;
        // Base-tagged frame reads and unbounded-upward "floors" (escaped
        // addresses and SIB array accesses). Base: b'b' = %rbp, b's' = %rsp.
        let mut read_ranges: Vec<(u8, i32, i32)> = Vec::new();
        // A floor at displacement D conservatively observes every frame
        // byte from D upward: an escaped `leaq D(%base)` pointer or an SIB
        // operand `D(%base,%idx,scale)` names an array whose element
        // addressing only extends upward (a negative index would be an
        // out-of-bounds access of a DIFFERENT object — C UB — so codegen
        // never relies on downward reach). Stores ending at or below D stay
        // deletable; this is what distinguishes the home-slot spills at
        // 16/32(%rsp) from a frame buffer based at 40(%rsp).
        let mut read_floors: Vec<(u8, i32)> = Vec::new();
        for k in body_start..func_end {
            if infos[k].is_nop() {
                continue;
            }
            let t = infos[k].trimmed(store.get(k));
            // Only a RAW copy of the frame base value (movq %rsp, %reg with
            // no memory operand) escapes the ENTIRE frame. A frame memory
            // operand, plain or SIB, names a specific displacement and is
            // handled with ranges/floors in the read scan below — the old
            // text checks matched only "(%rsp)" and treated every SIB form
            // (`40(%rsp,%r11,4)`) as a raw-base escape, bailing on the
            // common "frame buffer + scalar home slots" layout (sha, chacha).
            let frame_mem = parse_frame_mem(t);
            if frame_mem.is_none() {
                // Raw %rsp value flowing into a register (movq %rsp, %reg etc.).
                // Frame adjustments (subq/addq $N,%rsp; pushq/popq) and the
                // FP prologue mov are not escapes.
                if t.contains("%rsp") && !is_rsp_shift_line(t) && t != "movq %rsp, %rbp" {
                    has_unparseable_indirect = true;
                    break;
                }
                // Same for %rbp when it is the frame pointer.
                if rbp_is_frame
                    && t.contains("%rbp")
                    && t != "movq %rsp, %rbp"
                    && !matches!(
                        infos[k].kind,
                        LineKind::Push { reg: 5 } | LineKind::Pop { reg: 5 }
                    )
                {
                    has_unparseable_indirect = true;
                    break;
                }
            }
        }

        for k in body_start..func_end {
            if infos[k].is_nop() {
                continue;
            }

            // SOUNDNESS: an RSP-shifting instruction (push/pop/subq/addq
            // on %rsp) changes the effective address of every %rsp-relative
            // slot after it (offsets are compared verbatim). Bailing on ANY
            // such line disabled this pass for essentially every frame-
            // pointer-less function, because their EPILOGUES contain
            // `addq $N,%rsp` + `popq` chains — dead parameter-copy stores
            // like `movq %r10, 24(%rsp)` survived in every no-FP loop.
            //
            // An epilogue chain is harmless: between the `addq` and the
            // `ret`/tail-`jmp` only pops (reading their own pushed slots,
            // which live BELOW the canonical frame) and directives occur, so
            // no frame-slot load can observe a shifted offset, and after the
            // terminator the next block re-enters at canonical depth (all
            // in-body jumps originate at canonical depth — any body-interior
            // shift still bails below). Accept exactly that shape; every
            // other RSP shift keeps the conservative bail.
            if is_rsp_shift_line(infos[k].trimmed(store.get(k))) {
                if !is_epilogue_rsp_shift(store, infos, k, func_end) {
                    has_unparseable_indirect = true;
                    break;
                }
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
                }
                LineKind::LoadRbp { offset, size, .. } => {
                    let line = infos[k].trimmed(store.get(k));
                    if !rbp_is_frame && uses_rbp_mem_operand(line) {
                        has_unparseable_indirect = true;
                        break;
                    }
                    let base = frame_base(line, b's');
                    read_ranges.push((base, offset, size.byte_size()));
                }
                LineKind::LoadXmmRbp { offset, size } => {
                    let line = infos[k].trimmed(store.get(k));
                    if !rbp_is_frame && uses_rbp_mem_operand(line) {
                        has_unparseable_indirect = true;
                        break;
                    }
                    let base = frame_base(line, b's');
                    read_ranges.push((base, offset, size.byte_size()));
                }
                LineKind::Other { .. } => {
                    let rbp_off = infos[k].rbp_offset;
                    let line = infos[k].trimmed(store.get(k));
                    if line.starts_with("leaq ") {
                        // Address taken at a frame slot: the derived pointer
                        // may reach an unbounded upward span (passed to a
                        // callee, indexed later) — record a floor, not a
                        // fixed-width range.
                        if let Some(fm) = parse_frame_mem(line) {
                            read_floors.push((fm.base, fm.disp));
                        } else if rbp_off != RBP_OFFSET_NONE {
                            let base = frame_base(line, b's');
                            read_floors.push((base, rbp_off));
                        }
                    } else if rbp_off != RBP_OFFSET_NONE {
                        let base = frame_base(line, b's');
                        read_ranges.push((base, rbp_off, 32));
                    } else if parse_frame_mem(line).is_some() {
                        // Frame operand that was neither a leaq, a cached
                        // plain access, nor a provably bounded SIB: fail
                        // closed (unbounded negative-reach SIB).
                        if let Some((base, off, len, is_floor)) =
                            sib_frame_observation(store, infos, k, line)
                        {
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
                    // User asm naming a fixed frame slot can read/write that
                    // location with an unknown mnemonic width. x86 memory
                    // operands only ever extend UPWARD from the effective
                    // address (max 64 bytes for AVX-512), so a 64-byte range
                    // at the displacement is exact-sound without disabling
                    // DSE for unrelated home slots (`vstmxcsr 48(%rsp)`
                    // must not pin a counter store at 8(%rsp)).
                    if let Some(fm) = parse_frame_mem(line) {
                        if fm.sib {
                            // Raw asm index values are not bound by C UB: a
                            // negative or arbitrary index can name every
                            // lower slot. Fail closed.
                            has_unparseable_indirect = true;
                            break;
                        }
                        read_ranges.push((fm.base, fm.disp, 64));
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
                        read_ranges.push((base, rbp_off, 16));
                    } else if parse_frame_mem(line).is_some() {
                        if let Some((base, off, len, is_floor)) =
                            sib_frame_observation(store, infos, k, line)
                        {
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
        for k in body_start..func_end {
            if infos[k].is_nop() {
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
        if infos[idx].is_nop()
            || matches!(
                infos[idx].kind,
                LineKind::Empty | LineKind::Directive | LineKind::Label
            )
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
        if infos[j].is_nop() || matches!(infos[j].kind, LineKind::Directive) {
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
