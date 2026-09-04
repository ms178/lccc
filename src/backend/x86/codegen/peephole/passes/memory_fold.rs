//! Memory operand folding pass.
//!
//! Folds a stack load followed by an ALU instruction that uses the loaded register
//! as a source operand into a single instruction with a memory source operand.
//!
//! Pattern:
//!   movq  -N(%rbp), %rcx       ; LoadRbp { reg: 1(rcx), offset: -N, size: Q }
//!   addq  %rcx, %rax           ; Other: rax = rax + rcx
//!
//! Transformed to:
//!   addq  -N(%rbp), %rax       ; rax = rax + mem[rbp-N]
//!
//! Supported ALU ops: add, sub, and, or, xor, cmp, test (with q/l suffixes).
//! The loaded register must be used as the first (source) operand in AT&T syntax.
//! We only fold when the loaded register is one of the scratch registers (rax=0,
//! rcx=1, rdx=2) to avoid breaking live register values.

use super::super::types::*;
use super::helpers::{
    get_dest_reg, implicit_read_reg_family, is_read_modify_write, is_rsp_shift_line,
};

/// True when a line transfers control or merges paths, so that textual line
/// order can diverge from execution order.  A load-value reader textually
/// AFTER a "killing" pure overwrite may still be reached from the load
/// without ever executing the overwrite when a branch lies in between; the
/// overwrite only dominates the code that follows it on the same straight-
/// line fall-through.  Calls are deliberately NOT listed here, but they
/// are not free passage either: a Call is an OPAQUE READER of %xmm0-%xmm7
/// (the SysV FP argument registers — when the loaded value is passed to
/// the call, the caller reads it implicitly, and the self-move-elided
/// argument staging leaves no textual trace).  Every fold that deletes a
/// load whose register can be read that way therefore carries its own
/// call veto for %xmm0-%xmm7.  A call cannot read %xmm8-%xmm15 (they are
/// callee-saved and never argument registers, and compiled code never
/// reads a callee-saved register before defining it), so those are
/// genuinely safe to scan across.
fn is_cf_transfer(kind: LineKind) -> bool {
    matches!(
        kind,
        LineKind::Label | LineKind::CondJmp | LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret
    )
}

/// Format a stack slot as an assembly memory operand string.
/// Uses (%rbp) or (%rsp) depending on the original instruction text.
fn format_stack_offset(offset: i32, original_line: &str) -> String {
    if original_line.contains("(%rsp)") {
        format!("{}(%rsp)", offset)
    } else {
        format!("{}(%rbp)", offset)
    }
}

/// Try to parse an ALU instruction of the form "OPsuffix %src, %dst"
/// where OP is add/sub/and/or/xor/cmp/test.
/// Returns (op_name_with_suffix, dst_reg_str, src_family, dst_family).
fn parse_alu_reg_reg(trimmed: &str) -> Option<(&str, &str, RegId, RegId)> {
    let b = trimmed.as_bytes();
    if b.len() < 6 {
        return None;
    }

    let op_len = if b.starts_with(b"add")
        || b.starts_with(b"sub")
        || b.starts_with(b"and")
        || b.starts_with(b"xor")
        || b.starts_with(b"cmp")
    {
        3
    } else if b.starts_with(b"test") || b.starts_with(b"imul") {
        4
    } else if b.starts_with(b"or")
        && b.len() > 2
        && (b[2] == b'q' || b[2] == b'l' || b[2] == b'w' || b[2] == b'b')
    {
        2
    } else {
        return None;
    };

    let suffix = b[op_len];
    if suffix != b'q' && suffix != b'l' && suffix != b'w' && suffix != b'b' {
        return None;
    }
    let op_with_suffix = &trimmed[..op_len + 1];

    let rest = trimmed[op_len + 1..].trim();
    let (src_str, dst_str) = rest.split_once(',')?;
    let src_str = src_str.trim();
    let dst_str = dst_str.trim();

    if !src_str.starts_with('%') || !dst_str.starts_with('%') {
        return None;
    }

    let src_fam = register_family_fast(src_str);
    let dst_fam = register_family_fast(dst_str);
    if src_fam == REG_NONE || dst_fam == REG_NONE {
        return None;
    }

    Some((op_with_suffix, dst_str, src_fam, dst_fam))
}

/// Fold movsd stack load into subsequent scalar FP binary op as memory operand.
///
/// Pattern (produced after eliminate_fp_xmm_roundtrips):
///   movsd -M(%rbp), %xmm1   ; LoadXmmRbp{offset: -M} (historically Other{dest_reg: 25})
///   OP %xmm1, %xmm0          ; OP ∈ {mulsd, addsd, subsd, divsd}
///
/// Transformed to:
///   OP -M(%rbp), %xmm0
///
/// This reduces 4 instructions per FP binop (after roundtrip elimination) to 3.
pub(super) fn fold_fp_memory_operands(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 1 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Candidate: an XMM slot LOAD (`movsd -M(%rbp), %xmmN`). The line
        // classifier routes these to LoadXmmRbp (with `rbp_offset` cached by
        // xmm_slot_line_info); when this pass was written they were classified
        // as `Other{dest_reg: 25}` and the stale pre-filter silently disabled
        // the whole fold (caught by test_fp_memory_fold_mulsd after the
        // classifier gained the dedicated XMM slot kinds). The dest-register
        // check stays textual below: LoadXmmRbp records offset/size, not the
        // register, and the pass folds only the exact `%xmm1` dest form.
        if let LineKind::LoadXmmRbp { .. } = infos[i].kind {
            let offset = infos[i].rbp_offset;
            if offset == RBP_OFFSET_NONE {
                i += 1;
                continue;
            }

            let line_i = infos[i].trimmed(store.get(i));
            // Verify it is a movsd load from stack (not another xmm1-writing insn)
            if !line_i.starts_with("movsd ") || !line_i.ends_with(", %xmm1") {
                i += 1;
                continue;
            }

            // Find next non-NOP (skip only NOPs, not other instructions)
            let mut j = i + 1;
            while j < len && j < i + 4 && infos[j].is_nop() {
                j += 1;
            }
            if j >= len {
                i += 1;
                continue;
            }

            let line_j = infos[j].trimmed(store.get(j));
            let mem_op = format_stack_offset(offset, line_i);
            let replacement = match line_j {
                "mulsd %xmm1, %xmm0" => Some(format!("    mulsd {}, %xmm0", mem_op)),
                "addsd %xmm1, %xmm0" => Some(format!("    addsd {}, %xmm0", mem_op)),
                "subsd %xmm1, %xmm0" => Some(format!("    subsd {}, %xmm0", mem_op)),
                "divsd %xmm1, %xmm0" => Some(format!("    divsd {}, %xmm0", mem_op)),
                _ => None,
            };
            if let Some(new_text) = replacement {
                mark_nop(&mut infos[i]);
                replace_line(store, &mut infos[j], j, new_text);
                changed = true;
                i = j + 1;
                continue;
            }
        }

        i += 1;
    }
    changed
}

/// Fold a single-use scalar-FP register load into an adjacent scalar-FP
/// arithmetic instruction (VEX 3-operand and legacy 2-operand forms).
///
/// ```text
/// movsd 8(%rsi), %xmm5
/// vsubsd %xmm5, %xmm4, %xmm4
///   ->
/// vsubsd 8(%rsi), %xmm4, %xmm4
///
/// movsd -40(%rbp), %xmm1
/// mulsd %xmm1, %xmm0
///   ->
/// mulsd -40(%rbp), %xmm0
/// ```
///
/// This deliberately uses a stronger-than-necessary liveness proof: the loaded
/// XMM register must not be READ again before the function's `.size` — the
/// first later mention may also be a full overwrite of the register (see
/// [`is_pure_xmm_overwrite`]), which kills the loaded value and unblocks the
/// fold when the allocator reuses the register later.  That still misses
/// registers read after an intervening redefinition, but keeps deleting the
/// defining load safe without teaching the text peephole full XMM dataflow.
/// The load and consumer must be adjacent, so no address register or memory
/// state can change between them.  Source==destination is rejected because the
/// removed load also supplies the destructive destination's old value.
pub(super) fn fold_fp_register_loads(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    fn mentions_xmm(line: &str, reg: &str) -> bool {
        line.match_indices(reg).any(|(at, _)| {
            line.as_bytes()
                .get(at + reg.len())
                .is_none_or(|b| !b.is_ascii_digit())
        })
    }

    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i + 1 < len {
        if infos[i].is_nop() || infos[i + 1].is_nop() {
            i += 1;
            continue;
        }
        let load = infos[i].trimmed(store.get(i));
        let Some((load_op, operands)) = load.split_once(' ') else {
            i += 1;
            continue;
        };
        if load_op != "movsd" && load_op != "movss" {
            i += 1;
            continue;
        }
        let Some((mem, src_reg)) = operands.rsplit_once(',') else {
            i += 1;
            continue;
        };
        let mem = mem.trim();
        let src_reg = src_reg.trim();
        if !src_reg.starts_with("%xmm") || !mem.contains('(') || mem.starts_with('%') {
            i += 1;
            continue;
        }

        let consumer = infos[i + 1].trimmed(store.get(i + 1));
        let Some((arith_op, arith_operands)) = consumer.split_once(' ') else {
            i += 1;
            continue;
        };
        let legal_op = match load_op {
            "movsd" => matches!(
                arith_op,
                "vaddsd" | "vsubsd" | "vmulsd" | "vdivsd" | "addsd" | "subsd" | "mulsd" | "divsd"
            ),
            "movss" => matches!(
                arith_op,
                "vaddss" | "vsubss" | "vmulss" | "vdivss" | "addss" | "subss" | "mulss" | "divss"
            ),
            _ => false,
        };
        if !legal_op {
            i += 1;
            continue;
        }
        let ops: Vec<&str> = arith_operands.split(',').map(str::trim).collect();
        // VEX 3-operand form (dst == src2 required, and dst may not be the
        // loaded register: the removed load also supplies the destructive
        // destination's old value).  Legacy 2-operand form (mulsd/addsd/
        // subsd/divsd and the *ss variants): src must be the loaded register
        // and dst must differ from it for the same reason.
        if arith_op.starts_with('v') {
            if ops.len() != 3 || ops[0] != src_reg || ops[1] != ops[2] || ops[1] == src_reg {
                i += 1;
                continue;
            }
        } else if ops.len() != 2 || ops[0] != src_reg || ops[1] == src_reg {
            i += 1;
            continue;
        }

        // The source value must have no later use. Stop at `.size`; crossing a
        // label is harmless for this intentionally whole-function proof.  One
        // refinement: when the next mention fully overwrites the register,
        // the loaded value is dead at that point — every later mention refers
        // to the new value — so the fold stays sound when the allocator
        // reuses the register for an unrelated value later on.  The overwrite
        // only PROVES the value dead for code reached after it: a branch
        // between the consumer and the overwrite lets a path skip the
        // overwrite, so a reader textually past it can still observe the
        // loaded value (see is_cf_transfer).  The kill is therefore accepted
        // only across a control-flow-free straight-line stretch.
        // A Call is an OPAQUE READER of %xmm0-%xmm7 (the SysV FP argument
        // registers): when the loaded value is itself an argument of the
        // call, the caller reads it implicitly and the read leaves no
        // textual trace ("call foo" mentions no register — the argument
        // move is a self-move the builder elided).  Deleting the load
        // would feed the call a stale register, so the kill proof stops
        // at a call for the argument registers.  %xmm8-%xmm15 are
        // callee-saved and never argument registers, and compiled code
        // never reads a callee-saved register before defining it, so the
        // scan keeps walking there.  (The sibling folds carry the same
        // veto; unreachable through the current per-use constant
        // materialization, pinned as defense in depth.)
        let src_is_arg_reg = src_reg
            .strip_prefix("%xmm")
            .and_then(|d| d.parse::<u32>().ok())
            .is_some_and(|n| n <= 7);
        let mut later_mention = false;
        for k in (i + 2)..len {
            if infos[k].is_nop() {
                continue;
            }
            let t = infos[k].trimmed(store.get(k));
            if t.starts_with(".size ") {
                break;
            }
            if src_is_arg_reg && infos[k].kind == LineKind::Call {
                later_mention = true;
                break;
            }
            if mentions_xmm(t, src_reg) {
                let straight =
                    !(i + 2..k).any(|m| !infos[m].is_nop() && is_cf_transfer(infos[m].kind));
                later_mention = !(is_pure_xmm_overwrite(t, src_reg) && straight);
                break;
            }
        }
        if later_mention {
            i += 1;
            continue;
        }

        let replacement = if arith_op.starts_with('v') {
            format!("    {} {}, {}, {}", arith_op, mem, ops[1], ops[2])
        } else {
            format!("    {} {}, {}", arith_op, mem, ops[1])
        };
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[i + 1], i + 1, replacement);
        changed = true;
        i += 2;
    }
    changed
}

/// Fold stack-load-to-scratch relay moves: eliminate the scratch register
/// as intermediary when loading from a stack slot to another register.
///
/// Pattern:
///   movq  -N(%rbp), %rax       ; LoadRbp { reg: 0(rax), offset: -N, size: Q }
///   movq  %rax, %r12           ; Other: copy rax to callee-saved/arg register
///
/// Transformed to:
///   movq  -N(%rbp), %r12       ; direct load to destination register
///
/// Safety: The scratch register (rax) must not be read between the load and
/// the copy. We only fold loads to rax (reg 0) since codegen guarantees rax
/// is a temporary. The destination register must be a different GP register.
/// We verify rax is dead after (not read before being overwritten) to ensure
/// we don't break code that uses rax after the copy.
pub(super) fn fold_load_relay(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 1 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Step 1: Find a load from stack to %rax (scratch register).
        if let LineKind::LoadRbp {
            reg: 0,
            offset,
            size,
        } = infos[i].kind
        {
            // Only fold Q and L loads (not sign-extending SLQ, which changes value).
            if size != MoveSize::Q && size != MoveSize::L {
                i += 1;
                continue;
            }

            // Step 2: Find next non-NOP instruction.
            let mut j = i + 1;
            while j < len && infos[j].is_nop() {
                j += 1;
            }
            if j >= len {
                i += 1;
                continue;
            }

            // Step 3: Check if it's "movq %rax, %DEST" or "movl %eax, %DESTd"
            // where DEST is a different GP register.
            let dest_reg = match infos[j].kind {
                LineKind::Other { dest_reg }
                    if dest_reg != REG_NONE && dest_reg != 0 && dest_reg <= REG_GP_MAX =>
                {
                    let line_j = infos[j].trimmed(store.get(j));
                    // Must be a simple register-to-register mov
                    let is_movq_rax = line_j.starts_with("movq %rax, %") && !line_j.contains('(');
                    let is_movl_eax = line_j.starts_with("movl %eax, %") && !line_j.contains('(');
                    if is_movq_rax || is_movl_eax {
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

            // Step 4: Verify no intervening store to the same offset.
            let mut intervening_store = false;
            for k in (i + 1)..j {
                if let LineKind::StoreRbp { offset: so, .. } = infos[k].kind {
                    if so == offset {
                        intervening_store = true;
                        break;
                    }
                }
            }
            if intervening_store {
                i += 1;
                continue;
            }

            // Step 5: Check rax liveness after the copy.
            if !is_rax_dead_after(store, infos, j + 1, len) {
                i += 1;
                continue;
            }

            // Step 6: Transform! Replace load target and eliminate the copy.
            let load_line = infos[i].trimmed(store.get(i));
            let mem_op = format_stack_offset(offset, load_line);
            let dest_name = REG_NAMES[if size == MoveSize::L { 1 } else { 0 }][dest_reg as usize];
            let mnemonic = size.mnemonic();
            let new_load = format!("    {} {}, {}", mnemonic, mem_op, dest_name);

            replace_line(store, &mut infos[i], i, new_load);
            mark_nop(&mut infos[j]);
            changed = true;
            i = j + 1;
            continue;
        }

        i += 1;
    }

    changed
}

/// Fold load+leaq+store relay: eliminate accumulator relay for address computation.
///
/// Pattern:
///   movq  -N(%rbp), %rax       ; load base pointer from stack
///   leaq  K(%rax), %rax        ; compute base + offset
///   movq  %rax, %r12           ; store result to dest register
///
/// Transformed to:
///   movq  -N(%rbp), %r12       ; load directly to dest
///   leaq  K(%r12), %r12        ; compute offset in-place
///
/// Saves 1 instruction per occurrence. Safe when %rax is dead after the copy.
pub(super) fn fold_leaq_relay(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 2 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Step 1: Load from stack to %rax.
        if let LineKind::LoadRbp {
            reg: 0,
            offset,
            size: MoveSize::Q,
        } = infos[i].kind
        {
            // Step 2: Next must be leaq K(%rax), %rax
            let mut j = i + 1;
            while j < len && infos[j].is_nop() {
                j += 1;
            }
            if j >= len {
                i += 1;
                continue;
            }

            let leaq_offset = {
                let lj = infos[j].trimmed(store.get(j));
                if !lj.starts_with("leaq ") || !lj.ends_with(", %rax") {
                    i += 1;
                    continue;
                }
                let inner = &lj[5..lj.len() - 6]; // between "leaq " and ", %rax"
                if !inner.ends_with("(%rax)") {
                    i += 1;
                    continue;
                }
                let num_str = &inner[..inner.len() - 6]; // before "(%rax)"
                match num_str.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        i += 1;
                        continue;
                    }
                }
            };

            // Step 3: Next must be movq %rax, %DEST
            let mut k = j + 1;
            while k < len && infos[k].is_nop() {
                k += 1;
            }
            if k >= len {
                i += 1;
                continue;
            }

            let dest_reg = match infos[k].kind {
                LineKind::Other { dest_reg }
                    if dest_reg != REG_NONE && dest_reg != 0 && dest_reg <= REG_GP_MAX =>
                {
                    let lk = infos[k].trimmed(store.get(k));
                    if lk.starts_with("movq %rax, %") && !lk.contains('(') {
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

            // Step 4: Check rax is dead after k.
            let rax_dead = is_rax_dead_after(store, infos, k + 1, len);
            if !rax_dead {
                i += 1;
                continue;
            }

            // Step 5: Transform.
            let load_line = infos[i].trimmed(store.get(i));
            let mem_op = format_stack_offset(offset, load_line);
            let dest_64 = REG_NAMES[0][dest_reg as usize];
            let new_load = format!("    movq {}, {}", mem_op, dest_64);
            let new_leaq = format!("    leaq {}({}), {}", leaq_offset, dest_64, dest_64);

            replace_line(store, &mut infos[i], i, new_load);
            replace_line(store, &mut infos[j], j, new_leaq);
            mark_nop(&mut infos[k]);
            changed = true;
            i = k + 1;
            continue;
        }
        i += 1;
    }
    changed
}

/// Fold load+cltq+store relay: eliminate accumulator relay for sign-extend load.
///
/// Pattern:
///   movq  -N(%rbp), %rax       ; load 64-bit value (only lower 32 used)
///   cltq                       ; sign-extend %eax → %rax
///   movq  %rax, %r12           ; store sign-extended result
///
/// Transformed to:
///   movslq -N(%rbp), %r12      ; sign-extending load directly to dest
///
/// Saves 2 instructions per occurrence.
pub(super) fn fold_cltq_relay(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 2 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Step 1: Load from stack to %rax (either movq or movl).
        if let LineKind::LoadRbp {
            reg: 0,
            offset,
            size,
        } = infos[i].kind
        {
            if size != MoveSize::Q && size != MoveSize::L {
                i += 1;
                continue;
            }

            // Step 2: Next must be cltq.
            let mut j = i + 1;
            while j < len && infos[j].is_nop() {
                j += 1;
            }
            if j >= len {
                i += 1;
                continue;
            }
            {
                let lj = infos[j].trimmed(store.get(j));
                if lj != "cltq" {
                    i += 1;
                    continue;
                }
            }

            // Step 3: Next must be movq %rax, %DEST.
            let mut k = j + 1;
            while k < len && infos[k].is_nop() {
                k += 1;
            }
            if k >= len {
                i += 1;
                continue;
            }

            let dest_reg = match infos[k].kind {
                LineKind::Other { dest_reg }
                    if dest_reg != REG_NONE && dest_reg != 0 && dest_reg <= REG_GP_MAX =>
                {
                    let lk = infos[k].trimmed(store.get(k));
                    if lk.starts_with("movq %rax, %") && !lk.contains('(') {
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

            // Step 4: Check rax is dead after k.
            let rax_dead = is_rax_dead_after(store, infos, k + 1, len);
            if !rax_dead {
                i += 1;
                continue;
            }

            // Step 5: Transform! Replace all 3 instructions with one movslq.
            let load_line = infos[i].trimmed(store.get(i));
            let mem_op = format_stack_offset(offset, load_line);
            let dest_64 = REG_NAMES[0][dest_reg as usize];
            let new_inst = format!("    movslq {}, {}", mem_op, dest_64);

            replace_line(store, &mut infos[i], i, new_inst);
            mark_nop(&mut infos[j]);
            mark_nop(&mut infos[k]);
            changed = true;
            i = k + 1;
            continue;
        }
        i += 1;
    }
    changed
}

/// Fold movzbq/movzwq/movsbq/movswq relay: eliminate rax as intermediary
/// for zero/sign-extend-then-copy patterns.
///
/// Pattern:
///   movzbq  %al, %rax          ; zero-extend byte result to 64-bit
///   movq    %rax, %r12         ; copy to dest register
///
/// Transformed to:
///   movzbl  %al, %r12d         ; zero-extend directly to dest (32-bit write, implicit 64-bit zext)
///
/// Also handles movzwq→movq, movsbq→movq, movswq→movq, and movslq→movq.
/// Saves 1 instruction per occurrence.
pub(super) fn fold_extend_relay(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 1 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Step 1: Look for extension instructions writing to %rax from %al/%ax/%eax.
        if let LineKind::Other { dest_reg: 0 } = infos[i].kind {
            let line_i = infos[i].trimmed(store.get(i));

            // Parse the extension type and source sub-register.
            let (new_op, src_sub_idx) = if line_i == "movzbq %al, %rax" {
                // movzbq %al, %rax → movzbl %al, %DESTd
                ("movzbl", 3usize) // 3 = B (byte) index in REG_NAMES
            } else if line_i == "movzwq %ax, %rax" {
                ("movzwl", 2) // W (word)
            } else if line_i == "movsbq %al, %rax" {
                ("movsbl", 3) // B
            } else if line_i == "movswq %ax, %rax" {
                ("movswl", 2) // W
            } else {
                i += 1;
                continue;
            };

            // Step 2: Next must be movq %rax, %DEST.
            let mut j = i + 1;
            while j < len && infos[j].is_nop() {
                j += 1;
            }
            if j >= len {
                i += 1;
                continue;
            }

            let dest_reg = match infos[j].kind {
                LineKind::Other { dest_reg }
                    if dest_reg != REG_NONE && dest_reg != 0 && dest_reg <= REG_GP_MAX =>
                {
                    let lj = infos[j].trimmed(store.get(j));
                    if lj.starts_with("movq %rax, %") && !lj.contains('(') {
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

            // Step 3: Check rax is dead after.
            if !is_rax_dead_after(store, infos, j + 1, len) {
                i += 1;
                continue;
            }

            // Step 4: Transform.
            // Use the same sub-register for the source (al/ax from rax family=0).
            let src_name = REG_NAMES[src_sub_idx][0]; // %al or %ax
                                                      // Unsigned extends may target the 32-bit destination because the
                                                      // architectural 32-bit write zero-extends to the full GPR.  Signed
                                                      // byte/word extends must target the 64-bit destination: `movsbl
                                                      // %al,%esi` would sign-extend only to 32 bits and then zero the
                                                      // upper half of %rsi, corrupting negative values that are live as
                                                      // I64/long (gcc.c-torture/execute/20030218-1.c: -256 became
                                                      // 4294967040 after `movswl %ax,%esi; movq %rsi,%rax`).
            let dest = if matches!(new_op, "movsbl" | "movswl") {
                REG_NAMES[0][dest_reg as usize]
            } else {
                REG_NAMES[1][dest_reg as usize]
            };
            let op = match new_op {
                "movsbl" => "movsbq",
                "movswl" => "movswq",
                _ => new_op,
            };
            let new_inst = format!("    {} {}, {}", op, src_name, dest);

            replace_line(store, &mut infos[i], i, new_inst);
            mark_nop(&mut infos[j]);
            changed = true;
            i = j + 1;
            continue;
        }
        i += 1;
    }
    changed
}

/// General accumulator relay fold: retarget instructions that write to %rax
/// when the result is immediately copied to another register.
///
/// Handles:
///   leaq   X, %rax  +  movq %rax, %REG  →  leaq X, %REG
///   movslq X, %rax   +  movq %rax, %REG  →  movslq X, %REG
///   xorl   %eax, %eax + movq %rax, %REG  →  xorl %REGd, %REGd
///   addq   X, %rax  +  movq %rax, %REG  →  (not safe: flags + read-modify-write)
///
/// Only applies to instructions that purely write %rax without reading it first,
/// and where %rax is dead after the copy.
pub(super) fn fold_general_relay(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 1 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Step 1: Instruction writes to %rax (dest_reg == 0).
        if let LineKind::Other { dest_reg: 0 } = infos[i].kind {
            let line_i = infos[i].trimmed(store.get(i));

            // Step 2: Next must be movq %rax, %DEST.
            let mut j = i + 1;
            while j < len && infos[j].is_nop() {
                j += 1;
            }
            if j >= len {
                i += 1;
                continue;
            }
            let dest_reg = match infos[j].kind {
                LineKind::Other { dest_reg }
                    if dest_reg != REG_NONE && dest_reg != 0 && dest_reg <= REG_GP_MAX =>
                {
                    let lj = infos[j].trimmed(store.get(j));
                    if lj.starts_with("movq %rax, %") && !lj.contains('(') {
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

            // Step 3: Check rax is dead after.
            if !is_rax_dead_after(store, infos, j + 1, len) {
                i += 1;
                continue;
            }

            let dest_64 = REG_NAMES[0][dest_reg as usize];
            let dest_32 = REG_NAMES[1][dest_reg as usize];

            // Step 4: Match specific retargetable patterns.
            let new_inst = if line_i.starts_with("leaq ") && line_i.ends_with(", %rax") {
                // leaq X, %rax → leaq X, %REG
                // Safe: leaq doesn't read %rax (it computes an address, doesn't deref).
                // But check the source doesn't reference rax!
                let src = &line_i[5..line_i.len() - 6]; // between "leaq " and ", %rax"
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1;
                    continue;
                }
                Some(format!("    leaq {}, {}", src, dest_64))
            } else if line_i.starts_with("movslq ") && line_i.ends_with(", %rax") {
                // movslq X, %rax → movslq X, %REG
                let src = &line_i[7..line_i.len() - 6];
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1;
                    continue;
                }
                Some(format!("    movslq {}, {}", src, dest_64))
            } else if line_i == "xorl %eax, %eax" {
                // xorl %eax, %eax → xorl %REGd, %REGd
                Some(format!("    xorl {}, {}", dest_32, dest_32))
            } else if line_i.starts_with("movq $") && line_i.ends_with(", %rax") {
                // movq $imm, %rax → movq $imm, %REG
                let imm = &line_i[5..line_i.len() - 6];
                Some(format!("    movq {}, {}", imm, dest_64))
            } else if line_i.starts_with("movl $") && line_i.ends_with(", %eax") {
                // movl $imm, %eax → movl $imm, %REGd
                let imm = &line_i[5..line_i.len() - 6];
                Some(format!("    movl {}, {}", imm, dest_32))
            } else if line_i.starts_with("movq ")
                && line_i.ends_with(", %rax")
                && line_i.contains('(')
            {
                // movq N(%reg), %rax → movq N(%reg), %REG (pointer dereference)
                // Safe: source is a memory operand, doesn't read %rax as a value.
                // But check the addressing mode doesn't use %rax as base/index!
                let src = &line_i[5..line_i.len() - 6]; // between "movq " and ", %rax"
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1;
                    continue;
                }
                Some(format!("    movq {}, {}", src, dest_64))
            } else if line_i.starts_with("movl ")
                && line_i.ends_with(", %eax")
                && line_i.contains('(')
            {
                // movl N(%reg), %eax → movl N(%reg), %REGd (32-bit pointer dereference)
                let src = &line_i[5..line_i.len() - 6];
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1;
                    continue;
                }
                Some(format!("    movl {}, {}", src, dest_32))
            } else if line_i.starts_with("movzbq ") && line_i.ends_with(", %rax") {
                // movzbq N(%reg), %rax → movzbl N(%reg), %REGd (byte load zero-extend)
                let src = &line_i[7..line_i.len() - 6];
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1;
                    continue;
                }
                Some(format!("    movzbl {}, {}", src, dest_32))
            } else if line_i.starts_with("movzwq ") && line_i.ends_with(", %rax") {
                // movzwq N(%reg), %rax → movzwl N(%reg), %REGd
                let src = &line_i[7..line_i.len() - 6];
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1;
                    continue;
                }
                Some(format!("    movzwl {}, {}", src, dest_32))
            } else {
                None
            };

            if let Some(new_text) = new_inst {
                replace_line(store, &mut infos[i], i, new_text);
                mark_nop(&mut infos[j]);
                changed = true;
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    changed
}

/// Fold store relay: `movq %reg, %rax; movq %rax, N(%rsp)` → `movq %reg, N(%rsp)`.
/// Eliminates the intermediate %rax relay for register-to-stack stores.
pub(super) fn fold_store_relay(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 1 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Step 1: movq %REG, %rax (or movl %REGd, %eax)
        let (src_reg, is_32bit) = match infos[i].kind {
            LineKind::Other { dest_reg: 0 } => {
                let line = infos[i].trimmed(store.get(i));
                if line.starts_with("movq %") && line.ends_with(", %rax") && !line.contains('(') {
                    let src = &line[6..line.len() - 6]; // between "movq %" and ", %rax"
                    if !src.contains('%') {
                        // simple register name
                        (src.to_string(), false)
                    } else {
                        i += 1;
                        continue;
                    }
                } else if line.starts_with("movl %")
                    && line.ends_with(", %eax")
                    && !line.contains('(')
                {
                    let src = &line[6..line.len() - 6];
                    if !src.contains('%') {
                        (src.to_string(), true)
                    } else {
                        i += 1;
                        continue;
                    }
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

        // Step 2: Next must be movq %rax, N(%rsp) or movl %eax, N(%rsp)
        let mut j = i + 1;
        while j < len && infos[j].is_nop() {
            j += 1;
        }
        if j >= len {
            i += 1;
            continue;
        }

        let stored = match infos[j].kind {
            LineKind::StoreRbp {
                reg: 0,
                offset,
                size,
            } => {
                // movq/movl %rax/%eax → stack
                Some((offset, size))
            }
            _ => None,
        };

        if let Some((offset, size)) = stored {
            // Check rax is dead after the store
            if is_rax_dead_after(store, infos, j + 1, len) {
                // SOUNDNESS FIX: the folded store must write the SAME number
                // of bytes as the ORIGINAL store, using the STORE's width — NOT
                // the source move's width. Previously the mnemonic came from the
                // source `movq %rX,%rax`/`movl %rXd,%eax`, so a `movq %rX,%rax;
                // movl %eax,N(%rsp)` was folded to `movq %rX,N(%rsp)`, writing 8
                // bytes where only 4 were written before and corrupting the
                // adjacent stack slot. Use the store's MoveSize and the matching
                // sub-register.
                let src_fam = register_family_fast(&format!("%{}", src_reg));
                if src_fam == REG_NONE || src_fam as usize >= REG_NAMES[0].len() {
                    i += 1;
                    continue;
                }
                // SOUNDNESS: a 32-bit source move (`movl %REGd, %eax`) ZERO-EXTENDS
                // to %rax, so a 64-bit store of %rax stores 0 in the upper 32 bits.
                // Folding that to `movq %REG, slot` would store %REG's RAW upper bits
                // (which may be non-zero) — a value change. Only fold a 64-bit store
                // when the source move was itself 64-bit.
                let store_is_64 = !matches!(size, MoveSize::L);
                if is_32bit && store_is_64 {
                    i += 1;
                    continue;
                }
                let (mnem, reg_name) = match size {
                    MoveSize::L => ("movl", REG_NAMES[1][src_fam as usize]),
                    _ => ("movq", REG_NAMES[0][src_fam as usize]),
                };
                let line = infos[j].trimmed(store.get(j));
                if let Some(comma) = line.rfind(',') {
                    let mem_part = line[comma + 1..].trim();
                    // REG_NAMES entries already include the leading '%' — do NOT add
                    // another one (the old double-'%' produced `movl %%ebp, ...`).
                    let new_inst = format!("    {} {}, {}", mnem, reg_name, mem_part);
                    replace_line(store, &mut infos[j], j, new_inst);
                    mark_nop(&mut infos[i]);
                    changed = true;
                    i = j + 1;
                    continue;
                }
            }
        }

        i += 1;
    }

    changed
}

/// Check if %rax is dead starting from instruction index `start`.
/// Returns true if rax is overwritten before being read within a 16-instruction window.
/// Returns true if the register `reg` (0=rax, 1=rcx, 2=rdx) is not read
/// again before the next write within a 16-instruction window starting at
/// `start`. Barriers (except calls) conservatively return false.
fn is_reg_dead_after(
    store: &LineStore,
    infos: &[LineInfo],
    start: usize,
    len: usize,
    reg: u8,
) -> bool {
    let scan_limit = (start + 64).min(len);
    let mask = 1u16 << reg;
    let (reg64, reg32, reg8) = reg_names(reg);
    let mut scan = start;
    while scan < scan_limit {
        if infos[scan].is_nop() {
            scan += 1;
            continue;
        }
        // A barrier means control flow splits or the function returns. Only a
        // function CALL genuinely clobbers caller-saved registers. A Ret may
        // use %rax as the return value, and a branch/label may have the
        // register LIVE on another edge — so we must NOT treat
        // Ret/branch/label as dead. This was the soundness bug in the relay
        // passes: they forwarded through barriers and produced wrong code on
        // paths that used the register.
        if infos[scan].is_barrier() {
            if infos[scan].kind == LineKind::Call {
                // A call clobbers caller-saved registers (their results
                // overwrite any prior value), so the register is dead after.
                return true;
            }
            return false;
        }
        if infos[scan].reg_refs & mask != 0 {
            match infos[scan].kind {
                LineKind::LoadRbp { reg: r, .. } if r == reg => return true,
                LineKind::Pop { reg: r } if r == reg => return true,
                LineKind::Other { dest_reg } if dest_reg == reg => {
                    let t = infos[scan].trimmed(store.get(scan));
                    if t == format!("xorl {}, {}", reg32, reg32) {
                        return true;
                    }
                    if t.ends_with(&format!(", %{}", reg64)) || t.ends_with(&format!(", {}", reg32))
                    {
                        // The instruction WRITES the register. It is only dead
                        // (free to retarget) if this write establishes a FRESH
                        // value that does NOT depend on the current value — a
                        // self-move or sign-extension FROM the register still
                        // depends on it, so it is NOT dead.
                        let src = t
                            .split_once(',')
                            .map(|(s, _)| {
                                let mut toks = s.splitn(2, char::is_whitespace);
                                let _mnem = toks.next();
                                toks.next().unwrap_or("")
                            })
                            .unwrap_or("");
                        let reads = src.contains(reg32)
                            || src.contains(&format!("%{}", reg64))
                            || src.contains(reg8);
                        if !reads && !is_read_modify_write(t) {
                            return true;
                        }
                    }
                    return false; // register read (or rmw write)
                }
                _ => return false, // register read
            }
        }
        scan += 1;
    }
    true // ran out of window = assume dead
}

/// Register names for family id 0=rax, 1=rcx, 2=rdx: (64-bit, 32-bit, 8-bit).
fn reg_names(reg: u8) -> (&'static str, &'static str, &'static str) {
    match reg {
        0 => ("rax", "%eax", "%al"),
        1 => ("rcx", "%ecx", "%cl"),
        _ => ("rdx", "%edx", "%dl"),
    }
}

fn is_rax_dead_after(store: &LineStore, infos: &[LineInfo], start: usize, len: usize) -> bool {
    is_reg_dead_after(store, infos, start, len, 0)
}

/// Fold stack loads into subsequent ALU instructions as memory operands.
///
/// Safety: We only fold when the loaded register (the one being eliminated) is
/// a scratch register (rax=0, rcx=1, rdx=2) because the codegen guarantees
/// these are temporary and overwritten before the next use. We also verify
/// the loaded register is not the *destination* of the ALU instruction to avoid
/// creating a memory-destination instruction (which would write to the stack slot).
pub(super) fn fold_memory_operands(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 1 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        if let LineKind::LoadRbp {
            reg: load_reg,
            offset,
            size: load_size,
        } = infos[i].kind
        {
            // Only fold loads into scratch registers (rax=0, rcx=1, rdx=2)
            if load_reg > 2 {
                i += 1;
                continue;
            }

            // Only fold Q and L loads (64-bit and 32-bit). SLQ (sign-extending)
            // loads have different semantics.
            if load_size != MoveSize::Q && load_size != MoveSize::L {
                i += 1;
                continue;
            }

            // Find the next non-NOP, non-empty instruction
            let mut j = i + 1;
            while j < len && (infos[j].is_nop() || infos[j].kind == LineKind::Empty) {
                j += 1;
            }
            if j >= len {
                i += 1;
                continue;
            }

            // SOUNDNESS: an RSP-shifting line between the load and the
            // fold target changes the load's effective slot offset (the fold
            // would substitute a memory operand at the WRONG address). Skip.
            let mut shifted = false;
            for k in (i + 1)..j {
                if is_rsp_shift_line(infos[k].trimmed(store.get(k))) {
                    shifted = true;
                    break;
                }
            }
            if shifted {
                i += 1;
                continue;
            }

            let is_foldable_target =
                matches!(infos[j].kind, LineKind::Other { .. } | LineKind::Cmp);
            if is_foldable_target {
                // SOUNDNESS: between the load and the fold target, the
                // loaded register must not be WRITTEN by anything — otherwise
                // the target instruction operates on a different value than
                // the one the load produced, and folding the memory operand
                // would test/combine the WRONG slot. (The old code only
                // checked for stores to the same stack offset, missing
                // register re-writes such as `movl mem2, %eax` between
                // `movl mem1, %eax` and `testl %eax, %eax`.) Checking
                // `get_dest_reg` catches every register-writing instruction.
                let mut reg_rewritten = false;
                for k in (i + 1)..j {
                    if get_dest_reg(&infos[k]) == load_reg {
                        reg_rewritten = true;
                        break;
                    }
                }
                if reg_rewritten {
                    i += 1;
                    continue;
                }
                let trimmed_j = infos[j].trimmed(store.get(j));

                // Special case: testq/testl %REG, %REG where REG is the loaded scratch reg.
                // Fold to cmpq/cmpl $0, -N(%rbp).
                //
                // SOUNDNESS: the folding width MUST come from the TEST
                // instruction itself, NOT from the load width. `testl %eax, %eax`
                // tests the 32-bit value (so SF reflects bit 31), whereas
                // `testq %rax, %rax` tests the 64-bit value (SF reflects bit 63).
                // If a `testl` follows a 64-bit `movq` load of a value whose upper
                // 32 bits are non-zero (e.g. a zero-extended unsigned that is
                // negative as 32-bit: 0x00000000_FFFFFFCD), folding to `cmpq $0, mem`
                // would test 64 bits and flip the sign flag — miscompiling the
                // branch. So `testl`→`cmpl`, `testq`→`cmpq`.
                let test_q = trimmed_j.starts_with("testq ");
                let test_l = trimmed_j.starts_with("testl ");
                if (test_q || test_l) && {
                    // confirm it's the loaded scratch reg self-test
                    let pat = match load_reg {
                        0 => ("%rax", "%eax"),
                        1 => ("%rcx", "%ecx"),
                        _ => ("%rdx", "%edx"),
                    };
                    trimmed_j == &format!("testq {}, {}", pat.0, pat.0)
                        || trimmed_j == &format!("testl {}, {}", pat.1, pat.1)
                } {
                    // SOUNDNESS: the fold deletes the load — the loaded
                    // register must not be read again before its next write,
                    // or the value is lost (the register then holds stale
                    // data). This check was missing in fold_memory_operands
                    // (the relay folds had it for %rax only); the improved
                    // emitter produces more multi-use scratch values, which
                    // exposed it.
                    if !is_reg_dead_after(store, infos, j + 1, len, load_reg) {
                        i += 1;
                        continue;
                    }
                    let load_line = infos[i].trimmed(store.get(i));
                    let mem_op = format_stack_offset(offset, load_line);
                    // WIDTH SOUNDNESS: the memory test must read exactly
                    // the bytes the load defined. A 32-bit load
                    // (`movl mem, %reg`) zero-extends into the register, so a
                    // following `testq %reg, %reg` only tests the low 32 bits
                    // — folding it to `cmpq $0, mem` would read 8 bytes from
                    // a 4-byte slot (stale upper half → wrong flags → wrong
                    // branch). Fuse width = the LOAD's width, capped by the
                    // test's width: L-load + testq → `cmpl` (matches the
                    // register's tested bits); L-load + testl → `cmpl`;
                    // Q-load + testl → `cmpl` (low 4 bytes of the 8-byte
                    // value are what testl sees); Q-load + testq → `cmpq`.
                    let cmp_suffix = if test_l || load_size == MoveSize::L {
                        "cmpl"
                    } else {
                        "cmpq"
                    };
                    let new_inst = format!("    {} $0, {}", cmp_suffix, mem_op);
                    mark_nop(&mut infos[i]);
                    replace_line(store, &mut infos[j], j, new_inst);
                    changed = true;
                    i = j + 1;
                    continue;
                }

                if let Some((op_suffix, dst_str, src_fam, dst_fam)) = parse_alu_reg_reg(trimmed_j) {
                    // WIDTH SOUNDNESS: a 32-bit load (`movl mem,%reg`) only
                    // defines the low 32 bits of the register (upper 32 are
                    // zeroed, so the load is 4 bytes). Folding it into a
                    // 64-bit op (`xorq mem,%reg`) would read 8 bytes from a
                    // 4-byte stack slot, pulling in stale upper bytes —
                    // miscompile. Q-load into a 32-bit op is fine (reads the
                    // low 4 bytes of an 8-byte slot), as are matching widths.
                    let op_is_64 = op_suffix.ends_with('q');
                    if load_size == MoveSize::L && op_is_64 {
                        i += 1;
                        continue;
                    }
                    if src_fam == load_reg && dst_fam != load_reg {
                        // Check for intervening store to the same offset
                        let mut intervening_store = false;
                        for k in (i + 1)..j {
                            if let LineKind::StoreRbp { offset: so, .. } = infos[k].kind {
                                if so == offset {
                                    intervening_store = true;
                                    break;
                                }
                            }
                        }
                        if intervening_store {
                            i += 1;
                            continue;
                        }

                        // SOUNDNESS: the fold deletes the load; the
                        // loaded register must not be read again before its
                        // next write, or the value is lost.
                        if !is_reg_dead_after(store, infos, j + 1, len, load_reg) {
                            i += 1;
                            continue;
                        }

                        let load_line = infos[i].trimmed(store.get(i));
                        let mem_op = format_stack_offset(offset, load_line);
                        let new_inst = format!("    {} {}, {}", op_suffix, mem_op, dst_str);

                        mark_nop(&mut infos[i]);
                        replace_line(store, &mut infos[j], j, new_inst);
                        changed = true;
                        i = j + 1;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }

    changed
}

/// Fuse a register-memory load with a following dead single-use register copy:
///
/// ```text
///     movzbl (%rcx,%r9), %esi      movzbl (%rcx,%r9), %r8d
///     movl   %esi, %r8d        =>  (copy removed)
/// ```
///
/// Dominant redundant-copy shape in hot byte loops (gzip longest_match).
/// Soundness: the copy must be the load's ONLY consumer — the loaded register
/// is proven dead before any read/redispatch; the rewrite never changes the
/// load's width or extension semantics (width-narrowing copies are refused).
/// Fold a single-use scalar FP load into the memory-src2 slot of an adjacent
/// FMA3 231-form instruction:
///
/// ```text
///   movsd 32(%rsi), %xmm11                vfmadd231sd 32(%rsi), %xmm10, %xmm2
///   vfmadd231sd %xmm11, %xmm10, %xmm2  =>
/// ```
///
/// The first AT&T operand (Intel src2) is the only FMA3 slot that may read
/// memory.  When the loaded register is never read or written again, the
/// staging `movsd` is pure overhead — the dominant shape in the second and
/// later iterations of unrolled dot products, where each `b` element is
/// loaded exactly once.
///
/// Liveness proof, deliberately stronger than the block-local scan used by
/// `fold_scalar_fp_memory_into_vex_op`: from the load line to the end of the
/// function the register must be mentioned in EXACTLY those two lines (token-
/// bounded, so `%xmm1` never matches inside `%xmm11`), and no `call` may
/// intervene — a call can read `%xmm0`-`%xmm7` implicitly as the FP
/// argument registers without naming them in the text.  Mentions BEFORE the load are irrelevant
/// (the load redefines the register), so a home reused across loop iterations
/// stays foldable for the later load.  Being function-wide, the proof is
/// immune to cross-block reads that a block-local scan cannot see.
pub(super) fn fold_fma_memory_src2(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    /// FMA3 231 forms: dest = dest OP (src2 * src1); src2 is the rm slot and
    /// therefore the only memory-legal operand position.
    const FMA231: &[&str] = &["vfmadd231", "vfmsub231", "vfnmadd231", "vfnmsub231"];

    /// Parse an exact `%xmmN` operand token into N.
    fn xmm_num(op: &str) -> Option<u32> {
        let d = op.strip_prefix("%xmm")?;
        if d.is_empty() || !d.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        d.parse().ok()
    }

    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i + 1 < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        // ── load side: [v]movs{d,s} <mem>, %xmmN ─────────────────────────
        let li = infos[i].trimmed(store.get(i));
        let (width, rest) = if let Some(r) = li.strip_prefix("movsd ") {
            ("sd", r)
        } else if let Some(r) = li.strip_prefix("movss ") {
            ("ss", r)
        } else if let Some(r) = li.strip_prefix("vmovsd ") {
            ("sd", r)
        } else if let Some(r) = li.strip_prefix("vmovss ") {
            ("ss", r)
        } else {
            i += 1;
            continue;
        };
        // Split on the LAST comma: SIB addresses contain commas of their own
        // (`32(%rdi,%rax,8)`), and only the final operand is the destination.
        let Some((addr, dst)) = rest.rsplit_once(',') else {
            i += 1;
            continue;
        };
        let addr = addr.trim();
        let Some(n) = xmm_num(dst.trim()) else {
            i += 1;
            continue;
        };
        // The source must be a plain memory reference (a register source has
        // no paren; an XMM-form source would make this a reg-reg move).
        if !addr.contains('(') || addr.contains("%xmm") {
            i += 1;
            continue;
        }

        // ── FMA side: adjacent (NOPs in between are dropped lines) ──────
        let j = next_non_nop(infos, i + 1, len);
        if j >= len || infos[j].pinned {
            i += 1;
            continue;
        }
        let lj = infos[j].trimmed(store.get(j));
        let Some((fop, body)) = FMA231.iter().find_map(|m| {
            lj.strip_prefix(&format!("{}{} ", m, width))
                .map(|b| (*m, b))
        }) else {
            i += 1;
            continue;
        };
        // The pre-fold line has three plain register operands; anything with
        // a memory operand already (or an immediate) is not our shape.
        let ops: Vec<&str> = body.split(',').map(|s| s.trim()).collect();
        if ops.len() != 3 || ops.iter().any(|o| xmm_num(o).is_none()) {
            i += 1;
            continue;
        }
        if xmm_num(ops[0]) != Some(n) {
            i += 1;
            continue;
        }

        // ── liveness: the load and the FMA consume exactly two mentions;
        //    the first mention past them must fully overwrite the register
        //    (killing the loaded value) or be absent.  No intervening call
        //    for xmm0-xmm7 (calls read %xmm0-%xmm7 implicitly as the FP
        //    argument registers). ──────────────────────────────────────────────────
        let reg_token = format!("%xmm{}", n);
        let mut total = 0;
        let mut vetoed = false;
        let mut k = i;
        while k < len {
            let t = infos[k].trimmed(store.get(k));
            if t == ".cfi_endproc" {
                break;
            }
            if !infos[k].is_nop() {
                if n <= 7 && k != i && k != j && infos[k].kind == LineKind::Call {
                    vetoed = true;
                    break;
                }
                let m = mentions_token(t, &reg_token);
                if m > 0 && total == 2 {
                    // First mention past load+consumer: sound only when it
                    // rewrites the whole register (everything after it then
                    // uses the new value) AND every path from the consumer
                    // reaches it — a control-flow transfer in between lets a
                    // branch skip the overwrite while a reader past the merge
                    // still observes the loaded value (is_cf_transfer).
                    let straight =
                        !(j + 1..k).any(|h| !infos[h].is_nop() && is_cf_transfer(infos[h].kind));
                    vetoed = !(is_pure_xmm_overwrite(t, &reg_token) && straight);
                    break;
                }
                total += m;
                if total > 2 {
                    break;
                }
            }
            k += 1;
        }
        if vetoed || total != 2 {
            i += 1;
            continue;
        }

        mark_nop(&mut infos[i]);
        replace_line(
            store,
            &mut infos[j],
            j,
            format!("    {}{} {}, {}, {}", fop, width, addr, ops[1], ops[2]),
        );
        changed = true;
        i = j + 1;
    }
    changed
}

/// Extract exact `SYM(%rip)` constant-pool operand tokens (`.LCFP_` family)
/// from one line.  Both operand boundaries are checked: the token must be
/// preceded by whitespace/comma/line start and the `(%rip)` must be followed
/// by comma/whitespace/line end.  Displaced forms (`8+.LCFP_0(%rip)`) have
/// non-identifier characters in the symbol range and are rejected.
fn fp_const_rip_tokens(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = line[from..].find(".LCFP_") {
        let at = from + rel;
        from = at + 6;
        let before_ok = at == 0 || matches!(bytes[at - 1], b' ' | b',' | b'\t');
        if !before_ok {
            continue;
        }
        let Some(paren_rel) = line[at..].find('(') else {
            continue;
        };
        let sym = &line[at..at + paren_rel];
        // Pure pool symbol: identifiers and dots only (no `+`, no whitespace).
        if sym.is_empty()
            || !sym
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
        {
            continue;
        }
        if !line[at + paren_rel..].starts_with("(%rip)") {
            continue;
        }
        let after = at + paren_rel + 6;
        if after < line.len() && !matches!(bytes[after], b',' | b' ' | b'\t') {
            continue;
        }
        out.push(sym.to_string());
    }
    out
}

/// Hoist a repeatedly-loaded RIP-relative FP constant-pool value into a
/// single register materialization:
///
/// ```text
///     movsd .LCFP_0(%rip), %xmm1   <- placed into a dead (NOP) slot
///     ...
///     vaddsd .LCFP_0(%rip), %xmm2, %xmm2     vaddsd %xmm1, %xmm2, %xmm2
///     vaddsd .LCFP_0(%rip), %xmm4, %xmm4  -> vaddsd %xmm1, %xmm4, %xmm4
/// ```
///
/// The mature emitter homes scalar FP constants in `.rodata` and re-reads
/// the pool at every use.  In leaf hot kernels that multiplies the load
/// count — dot8d's SSE2 kernel performed 12 loads per call (8 data plus 4
/// reads of the SAME 8-byte constant) where gcc/clang materialize the
/// constant once (gcc: a single `pxor`) — a measured ~18% runtime gap on
/// the dot8bench2 driver.  The rewritten value bits are identical: the
/// materialization copies the same pool bytes the folded operand read.
///
/// Soundness conditions (all function-wide and conservative):
/// * no `call` anywhere — every xmm is caller-saved, so a materialization
///   would be dead after any call;
/// * no inline-asm region — opaque code may touch any xmm register;
/// * no `blendv`-family line — those read `%xmm0` implicitly;
/// * the chosen register has ZERO textual mentions in the whole store,
///   NOP'd lines included, because later passes may revive a NOP slot with
///   its original text;
/// * the materialization lands in a NOP slot before the first rewritten use
///   with no label between slot and last use, so every use is reached by
///   fall-through from the materialization;
/// * only exact `SYM(%rip)` operands of known scalar-FP mnemonics are
///   rewritten; pinned lines and multi-line combined slots are skipped.
///
/// Needs a count of at least 2 rewritten uses of the same symbol: with one
/// use the materialization would just add an instruction.
pub(super) fn hoist_repeated_fp_constant_loads(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    const FP_OPS: &[&str] = &[
        "addsd",
        "addss",
        "subsd",
        "subss",
        "mulsd",
        "mulss",
        "divsd",
        "divss",
        "vaddsd",
        "vaddss",
        "vsubsd",
        "vsubss",
        "vmulsd",
        "vmulss",
        "vdivsd",
        "vdivss",
        "vfmadd132sd",
        "vfmadd132ss",
        "vfmadd213sd",
        "vfmadd213ss",
        "vfmadd231sd",
        "vfmadd231ss",
        "vfmsub132sd",
        "vfmsub132ss",
        "vfmsub213sd",
        "vfmsub213ss",
        "vfmsub231sd",
        "vfmsub231ss",
        "vfnmadd132sd",
        "vfnmadd132ss",
        "vfnmadd213sd",
        "vfnmadd213ss",
        "vfnmadd231sd",
        "vfnmadd231ss",
        "vfnmsub132sd",
        "vfnmsub132ss",
        "vfnmsub213sd",
        "vfnmsub213ss",
        "vfnmsub231sd",
        "vfnmsub231ss",
    ];

    let len = store.len();
    for k in 0..len {
        if infos[k].is_nop() {
            continue;
        }
        if infos[k].kind == LineKind::Call || infos[k].kind == LineKind::InlineAsm {
            return false;
        }
        if infos[k].trimmed(store.get(k)).contains("blendv") {
            return false;
        }
    }

    // Collect rewritten-candidate uses per pool symbol.
    let mut uses: Vec<(String, Vec<usize>)> = Vec::new();
    for k in 0..len {
        if infos[k].is_nop() || infos[k].pinned {
            continue;
        }
        let t = store.get(k);
        if t.contains('\n') {
            continue; // combined multi-line slot: leave alone
        }
        let tt = infos[k].trimmed(t);
        let Some((op, _)) = tt.split_once(' ') else {
            continue;
        };
        if !FP_OPS.contains(&op) {
            continue;
        }
        for sym in fp_const_rip_tokens(tt) {
            match uses.iter_mut().find(|(s, _)| *s == sym) {
                Some((_, sites)) => sites.push(k),
                None => uses.push((sym, vec![k])),
            }
        }
    }
    // A line whose token scanner reports the same symbol twice (rmw spell
    // variants) must count as ONE use: the "at least two rewritten uses"
    // threshold counts distinct lines.  Sites are pushed in ascending line
    // order, so adjacent duplicates dedup in place.
    for (_, sites) in uses.iter_mut() {
        sites.dedup();
    }

    let mut changed = false;
    for (sym, sites) in uses.iter().filter(|(_, s)| s.len() >= 2) {
        // Free-register scan over ALL lines (NOP'd text can be revived by
        // later passes, so its mentions count too).
        let mut mentioned = [false; 16];
        for k in 0..len {
            let t = store.get(k);
            for n in 0..16 {
                if !mentioned[n] && mentions_token(t, &format!("%xmm{}", n)) > 0 {
                    mentioned[n] = true;
                }
            }
        }
        let Some(free) = (0..16).find(|n| !mentioned[*n]) else {
            continue;
        };
        let reg = format!("%xmm{}", free);

        // Materialization slot: the latest NOP before the first use, with
        // no label between slot and last use (pure fall-through).
        let first = sites[0];
        let last = sites[sites.len() - 1];
        let Some(slot) = (0..first).rev().find(|&p| infos[p].is_nop()) else {
            continue;
        };
        if (slot + 1..=last).any(|k| infos[k].kind == LineKind::Label) {
            continue;
        }

        // Materialization width: `movss` for a symbol used only by
        // single-precision ops (an exact 4-byte pool read, zero-extending
        // into the register), `movsd` as soon as any double-precision site
        // needs the full 8 bytes.  Mnemonics are never touched by the
        // rewrite below, so reading them off the live store is exact.
        let any_double = sites.iter().any(|&k| {
            store
                .get(k)
                .trim()
                .split_once(' ')
                .is_some_and(|(op, _)| op.ends_with("sd"))
        });
        let load_op = if any_double { "movsd" } else { "movss" };

        replace_line(
            store,
            &mut infos[slot],
            slot,
            format!("    {} {}(%rip), {}", load_op, sym, reg),
        );
        let tok = format!("{}(%rip)", sym);
        for &k in sites {
            let t = store.get(k).to_string();
            let new = crate::backend::peephole_common::replace_whole_word(&t, &tok, &reg);
            replace_line(store, &mut infos[k], k, new);
        }
        changed = true;
    }
    changed
}

/// Count occurrences of a register token (e.g. `%xmm3`) in `text` with
/// operand-boundary checks on both sides: `%xmm1` must not match inside
/// `%xmm11`, and `%ax` must not match inside `%rax`.
fn mentions_token(text: &str, token: &str) -> usize {
    let mut count = 0;
    let mut from = 0;
    while let Some(rel) = text[from..].find(token) {
        let at = from + rel;
        let after = at + token.len();
        let ok_before = at == 0
            || !text[..at]
                .chars()
                .next_back()
                .map_or(false, |c| c.is_ascii_alphanumeric() || c == '_' || c == '%');
        let ok_after = !text[after..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
        if ok_before && ok_after {
            count += 1;
        }
        from = at + 1;
    }
    count
}

/// True for the canonical zero materializations of %xmm0.
fn is_xmm0_zeroing(t: &str) -> bool {
    let t = t.trim();
    t == "xorpd %xmm0, %xmm0"
        || t == "xorps %xmm0, %xmm0"
        || t == "pxor %xmm0, %xmm0"
        || t == "vxorpd %xmm0, %xmm0, %xmm0"
        || t == "vxorps %xmm0, %xmm0, %xmm0"
        || t == "vpxor %xmm0, %xmm0, %xmm0"
}

/// True when `t` unconditionally overwrites `reg` (its last operand) from
/// sources that do not include `reg`: the full-width mov families, the
/// zero-extending narrow movs, and the self-xor zeroings.  Anything else
/// (ALU/FMA/cvt forms) READS the register even when it also writes it, so
/// it does not redefine it.
///
/// MERGE-WRITE RULE (red-team fix, PR #359 follow-up): the scalar move
/// forms `movsd`/`movss`/`vmovsd`/`vmovss` with a REGISTER source write
/// only the low element and PRESERVE the remaining bits of the destination
/// (SDM: MOVSD/MOVSS 128-bit legacy and VEX forms merge).  Those preserved
/// bits may have been defined by the very load a fold deletes — a
/// memory-source scalar load zeroes bits above the element — so treating a
/// reg-reg scalar move as a full overwrite let the load-folding passes
/// delete a load whose zeroed high half was still observable through the
/// merge chain (`vmovsd %xmm12, %xmm11` followed by a packed
/// `vmovapd %xmm11, ...`).  Reg-reg scalar moves are therefore NOT
/// overwrites; only their memory-source forms are.  VEX-encoded xor keeps
/// the full self-xor spelling (`vxorpd %r, %r, %r`) as an overwrite.
fn is_pure_xmm_overwrite(t: &str, reg: &str) -> bool {
    let t = t.trim();
    for z in ["xorpd", "xorps", "pxor", "vxorpd", "vxorps", "vpxor"] {
        if let Some(rest) = t.strip_prefix(z) {
            let rest = rest.trim_start();
            if rest == format!("{}, {}", reg, reg) || rest == format!("{}, {}, {}", reg, reg, reg) {
                return true;
            }
        }
    }
    // Scalar merge forms: only the memory-source spelling defines the whole
    // register (upper bits zeroed); the register-source spelling merges.
    const MERGE_MOVS: &[&str] = &["vmovsd ", "vmovss ", "movsd ", "movss "];
    for m in MERGE_MOVS {
        if let Some(rest) = t.strip_prefix(m) {
            if let Some((srcs, dst)) = rest.trim().rsplit_once(',') {
                if dst.trim() == reg
                    && mentions_token(srcs, reg) == 0
                    && !srcs.trim_start().starts_with('%')
                {
                    return true;
                }
            }
        }
    }
    // Full-width packed and zero-extending narrow forms define the whole
    // register from any source.
    const FULL_MOVS: &[&str] = &[
        "vmovapd ", "vmovaps ", "movapd ", "movaps ", "vmovdqa ", "vmovdqu ", "movdqa ", "movdqu ",
        "vmovd ", "vmovq ", "movd ", "movq ",
    ];
    for m in FULL_MOVS {
        if let Some(rest) = t.strip_prefix(m) {
            if let Some((srcs, dst)) = rest.trim().rsplit_once(',') {
                if dst.trim() == reg && mentions_token(srcs, reg) == 0 {
                    return true;
                }
            }
        }
    }
    false
}

/// Fold a zero-addend 213-form FMA whose multiplier was JUST loaded into
/// the 132 form with the load folded into the memory SRC3 slot:
///
/// ```text
///   movsd 8(%rsi), %xmm3              xorpd %xmm0, %xmm0
///   xorpd %xmm0, %xmm0          ->    vfmadd132sd 8(%rsi), %xmm0, %xmm2
///   vfmadd213sd %xmm0, %xmm3, %xmm2
/// ```
///
/// 213 computes dst = vvvv*dst + src2 with src2 = the zero; 132 computes
/// dst = dst*mem + vvvv with the zero moved to vvvv and the multiplier
/// moved to the memory-legal rm slot.  FP multiplication is commutative
/// (IEEE 754 requires x*y == y*x bitwise), and both forms read the same
/// two values in the same roles, so the rewrite is value-identical — this
/// is the exact form GCC picks for constant-accumulator dot products.
///
/// Soundness: the loaded register must be mentioned exactly twice between
/// the load and the end of the function (the load itself and the FMA), the
/// same last-mention proof as [`fold_fma_memory_src2`]; the three lines
/// must be adjacent (NOPs aside) so no label makes the FMA reachable
/// without the zeroing; and neither the zero nor the destination may be
/// the loaded register.  The zeroing itself is preserved — %xmm0 is
/// general scratch, so its zero state is not provable here;
/// [`eliminate_redundant_xmm0_zeroing`] removes the repeats block-locally.
pub(super) fn fold_zero_addend_fma213_to_132(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    /// Parse `%xmmN` -> N.
    fn xmm_num(op: &str) -> Option<u32> {
        let d = op.strip_prefix("%xmm")?;
        if d.is_empty() || !d.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        d.parse().ok()
    }

    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i + 2 < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        // ── load side: [v]movs{d,s} <mem>, %xmmB ─────────────────────────
        let li = infos[i].trimmed(store.get(i));
        let (width, rest) = if let Some(r) = li.strip_prefix("movsd ") {
            ("sd", r)
        } else if let Some(r) = li.strip_prefix("movss ") {
            ("ss", r)
        } else if let Some(r) = li.strip_prefix("vmovsd ") {
            ("sd", r)
        } else if let Some(r) = li.strip_prefix("vmovss ") {
            ("ss", r)
        } else {
            i += 1;
            continue;
        };
        let Some((addr, dst)) = rest.rsplit_once(',') else {
            i += 1;
            continue;
        };
        let addr = addr.trim();
        let Some(b) = xmm_num(dst.trim()) else {
            i += 1;
            continue;
        };
        if b == 0 || !addr.contains('(') || addr.contains("%xmm") {
            i += 1;
            continue;
        }
        let b_tok = format!("%xmm{}", b);

        // ── zeroing line ──────────────────────────────────────────────────
        let j = next_non_nop(infos, i + 1, len);
        if j >= len || infos[j].pinned || !is_xmm0_zeroing(infos[j].trimmed(store.get(j))) {
            i += 1;
            continue;
        }

        // ── FMA side: vfmadd213s{d,s} %xmm0, %xmmB, %dstA ────────────────
        let k = next_non_nop(infos, j + 1, len);
        if k >= len || infos[k].pinned {
            i += 1;
            continue;
        }
        let lk = infos[k].trimmed(store.get(k));
        let Some(body) = lk.strip_prefix(&format!("vfmadd213{} ", width)) else {
            i += 1;
            continue;
        };
        let ops: Vec<&str> = body.split(',').map(|s| s.trim()).collect();
        let ok =
            ops.len() == 3 && ops[0] == "%xmm0" && ops[1] == b_tok && xmm_num(ops[2]) != Some(b);
        if !ok {
            i += 1;
            continue;
        }

        // ── liveness: the FMA must be the LAST reader of the loaded value.
        //    The load/FMA adjacency already proves nothing reads %xmmB in
        //    between; after the FMA, the first mention of %xmmB must be a
        //    pure full-width overwrite (the register reused for another
        //    value — the later definition is what any subsequent reader
        //    sees) or absent entirely.  Any read — including a read-
        //    modify-write whose destination merely happens to be %xmmB —
        //    vetoes the fold. ─────────────────────────────────────────────
        let mut ok_after = true;
        let mut m = k + 1;
        while m < len {
            let t = infos[m].trimmed(store.get(m));
            if t == ".cfi_endproc" {
                break;
            }
            // Same call-as-argument-reader contract as the other FP folds:
            // a Call implicitly reads %xmm1-%xmm7 when the loaded value is
            // passed to it (self-move-elided argument staging is invisible
            // to this textual scan).  %xmm0 is excluded by construction —
            // it is the zeroing register itself.
            if b <= 7 && !infos[m].is_nop() && infos[m].kind == LineKind::Call {
                ok_after = false;
                break;
            }
            if !infos[m].is_nop() && mentions_token(t, &b_tok) > 0 {
                // Pure redefinition kills the loaded value only for code it
                // dominates; a branch between the FMA and the redefinition
                // lets the other path read the loaded multiplier past the
                // merge (is_cf_transfer).
                let straight =
                    !(k + 1..m).any(|h| !infos[h].is_nop() && is_cf_transfer(infos[h].kind));
                ok_after = is_pure_xmm_overwrite(t, &b_tok) && straight;
                break;
            }
            m += 1;
        }
        if !ok_after {
            i += 1;
            continue;
        }

        mark_nop(&mut infos[i]);
        replace_line(
            store,
            &mut infos[k],
            k,
            format!("    vfmadd132{} {}, %xmm0, {}", width, addr, ops[2]),
        );
        changed = true;
        i = k + 1;
    }
    changed
}

/// Delete `xorpd %xmm0, %xmm0` (and pxor/vxorpd/vxorps forms) whose zero is
/// still live from an earlier zeroing in the same basic block.  Forward
/// state machine: the zero state is set by a zeroing, and cleared by any
/// line whose LAST operand is %xmm0 (the AT&T destination — writes only;
/// reads of %xmm0 such as `vfmadd213sd %xmm0, ...` preserve it), by any
/// call/label/branch/ret (control flow merges unknown states; calls
/// clobber), and by opaque inline asm.  This is what turns the per-
/// accumulator zero materialization of a constant-accumulator loop into
/// GCC's single resident zero.
pub(super) fn eliminate_redundant_xmm0_zeroing(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut zero_live = false;
    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        if is_xmm0_zeroing(t) {
            if zero_live && !infos[i].pinned {
                mark_nop(&mut infos[i]);
                changed = true;
            } else {
                zero_live = true;
            }
            continue;
        }
        match infos[i].kind {
            LineKind::Label
            | LineKind::Jmp
            | LineKind::JmpIndirect
            | LineKind::CondJmp
            | LineKind::Call
            | LineKind::Ret
            | LineKind::InlineAsm => zero_live = false,
            _ => {
                // Writes %xmm0 exactly when the last AT&T operand is %xmm0.
                if t.rsplit_once(',')
                    .map(|(_, last)| last.trim() == "%xmm0")
                    .unwrap_or(false)
                {
                    zero_live = false;
                }
            }
        }
    }
    changed
}

pub(super) fn fold_load_copy_relay(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    use super::super::types::register_family_fast;
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    let dbg = std::env::var("CCC_DEBUG_LCR").is_ok();
    let mut stats = [0u32; 6]; // loads, copy-ok, dead, fused, copy-bad, not-dead
    let mut cur_fn = String::new();

    // Parse "MNEM MEM, %reg" with a parenthesized MEM that has no segment
    // prefix, lock/rep prefix, or destination-in-source aliasing concerns.
    fn parse_mem_load(t: &str) -> Option<(&str, &str, &str)> {
        const LOADS: &[&str] = &[
            "movzbl", "movzbw", "movzwl", "movslq", "movswq", "movsbq", "movsbl", "movswl", "movl",
            "movq",
        ];
        let mnem = LOADS
            .iter()
            .find(|m| t.starts_with(*m) && t.as_bytes().get(m.len()) == Some(&b' '))?;
        let rest = &t[mnem.len() + 1..];
        let comma = rest.rfind(',')?;
        let mem = rest[..comma].trim();
        let dst = rest[comma + 1..].trim();
        if !mem.contains('(') || mem.starts_with('%') || mem.contains('%') && !mem.contains('(') {
            return None;
        }
        if !dst.starts_with('%') {
            return None;
        }
        // Reject segment-prefixed and prefixed forms (lock/rep/bnd).
        if t.contains('%') && mem.contains(':') {
            return None;
        }
        Some((mnem, mem, dst))
    }

    while i + 1 < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        if dbg && t.ends_with(':') && !t.starts_with('.') && !t.starts_with('%') {
            cur_fn = t.trim_end_matches(':').to_string();
        }
        let (mnem, mem, dst) = match parse_mem_load(t) {
            Some(x) => x,
            None => {
                i += 1;
                continue;
            }
        };
        stats[0] += 1;
        let load_fam = register_family_fast(dst);
        if load_fam == super::super::types::REG_NONE {
            i += 1;
            continue;
        }
        // The memory operand must be a pure address (no write side effects),
        // and we must be able to re-emit it unchanged.
        // Find the copy: the next non-NOP instruction must be a pure
        // register-to-register mov consuming the loaded register.
        let mut j = i + 1;
        while j < len && infos[j].is_nop() {
            j += 1;
        }
        if j >= len || infos[j].is_barrier() || infos[j].pinned {
            i += 1;
            continue;
        }
        let tc = infos[j].trimmed(store.get(j));
        let copy_parts: Vec<&str> = tc.split(", ").collect();
        let copy_ok = if copy_parts.len() == 2 {
            // copy_parts[0] still carries the mnemonic ("movl %r8d").
            let src = copy_parts[0].rsplit_once(' ').map(|(_, r)| r).unwrap_or("");
            let cdest = copy_parts[1];
            let is_movq = tc.starts_with("movq ");
            let is_movl = tc.starts_with("movl ");
            // Both copy operands must be GENERAL-PURPOSE registers: the
            // family mapper aliases %xmm/%ymm names onto GPR families, and a
            // GPR<->XMM move is a conversion, never a pure relay.
            let is_gp = |r: &str| r.starts_with("%r") || r.starts_with("%e");
            if !(is_movq || is_movl) {
                false
            } else if !is_gp(src) || !is_gp(cdest) || cdest.contains('(') {
                false
            } else {
                let src_fam = register_family_fast(src);
                let dest_fam = register_family_fast(cdest);
                if src_fam != load_fam
                    || dest_fam == super::super::types::REG_NONE
                    || src_fam == dest_fam
                {
                    false
                } else if mem.contains(cdest) {
                    false
                } else {
                    match (mnem, is_movl) {
                        // Zero-extending loads: 32-bit and 64-bit copies both
                        // reproduce the same full-register value.
                        ("movzbl", _) | ("movzbw", _) | ("movzwl", _) | ("movl", _) => true,
                        // Sign-extending loads: only a width-preserving 64-bit
                        // copy keeps the sign extension intact.
                        ("movslq", false) | ("movswq", false) | ("movsbq", false) => true,
                        ("movq", false) => true,
                        _ => false,
                    }
                }
            }
        } else {
            false
        };
        if !copy_ok {
            stats[4] += 1;
            if dbg && stats[4] < 30 {
                eprintln!("[LCR][{}] copy-bad: load=[{}] copy=[{}]", cur_fn, t, tc);
            }
            i += 1;
            continue;
        }
        stats[1] += 1;
        let cdest = copy_parts[1];
        // Prove the loaded register is dead after the copy within a bounded
        // window: no read before the next write; calls kill caller-saved regs.
        let load_is_callee_saved = matches!(load_fam, 3 | 5 | 12 | 13 | 14 | 15);
        let mut dead = false;
        let mut k = j + 1;
        let limit = (len).min(j + 16);
        while k < limit {
            if infos[k].is_nop() {
                k += 1;
                continue;
            }
            if infos[k].pinned {
                // Pinned lines are immovable; if one references the loaded
                // register we cannot prove deadness inside this window.
                if infos[k].reg_refs & (1u16 << load_fam) != 0 {
                    break;
                }
                k += 1;
                continue;
            }
            if infos[k].is_barrier() {
                // A call CONSUMES the loaded register if it is passed as an
                // argument — the fuse would retarget the load and the call
                // would read the un-loaded register. Only when the call does
                // not reference the register may the caller-saved clobber
                // count as deadness.
                if matches!(infos[k].kind, LineKind::Call)
                    && !load_is_callee_saved
                    && infos[k].reg_refs & (1u16 << load_fam) == 0
                {
                    dead = true;
                }
                // Labels/branches: conservatively keep the value (another path
                // may read it), unless we already saw a full overwrite.
                break;
            }
            match infos[k].kind {
                LineKind::LoadRbp { reg, .. } if reg == load_fam => {
                    dead = true;
                    break;
                }
                LineKind::Pop { reg } if reg == load_fam => {
                    dead = true;
                    break;
                }
                _ => {
                    // Instructions with implicit register usage (cltq/cqto/cdq/
                    // cqo read %eax, div/idiv/mul read %rax:%rdx, shld/shrd read
                    // %cl) can read the loaded register even when the text names
                    // no register of this family. They are never a pure write, so
                    // they must veto the fuse: retargeting the load would leave
                    // the implicit read seeing an un-loaded register.
                    // (Regression: gzip 1.14 pqdownheap/gen_codes SIGBUS/SIGSEGV
                    // when `cltq` after a relayed load read undefined %eax.)
                    let td = infos[k].trimmed(store.get(k));
                    if implicit_read_reg_family(td) == Some(load_fam) {
                        break;
                    }
                    if infos[k].reg_refs & (1u16 << load_fam) != 0 {
                        // A pure write to the register family (dest-only) kills it.
                        let writes = matches!(infos[k].kind, LineKind::Other { dest_reg } if dest_reg == load_fam);
                        let reads_src = reg_names_family(load_fam).iter().any(|n| td.contains(n));
                        if writes && !reads_src {
                            dead = true;
                        }
                        break;
                    }
                }
            }
            k += 1;
        }
        if !dead {
            stats[5] += 1;
            if dbg && stats[5] < 30 {
                eprintln!("[LCR][{}] not-dead: load=[{}] copy=[{}]", cur_fn, t, tc);
            }
            i = j + 1;
            continue;
        }
        stats[2] += 1;
        // Rewrite the load to target the copy's destination; drop the copy.
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static LCR_FUSED: AtomicU32 = AtomicU32::new(0);
            let lim = std::env::var("CCC_LCR_LIMIT")
                .ok()
                .and_then(|v| v.parse::<u32>().ok());
            if let Some(l) = lim {
                if LCR_FUSED.load(Ordering::Relaxed) >= l {
                    i = j + 1;
                    continue;
                }
                LCR_FUSED.fetch_add(1, Ordering::Relaxed);
            }
        }
        stats[3] += 1;
        if dbg {
            eprintln!(
                "[LCR][{}] FUSE #{}: [{}] + [{}] -> dest {}",
                cur_fn, stats[3], t, tc, cdest
            );
        }
        let new_load = format!("    {} {}, {}", mnem, mem, cdest);
        replace_line(store, &mut infos[i], i, new_load);
        mark_nop(&mut infos[j]);
        changed = true;
        i = j + 1;
    }
    if dbg && stats[0] > 0 {
        eprintln!(
            "[LCR] loads={} copy-ok={} dead={} fused={} copy-bad={} not-dead={}",
            stats[0], stats[1], stats[2], stats[3], stats[4], stats[5]
        );
    }
    changed
}

/// Every textual name of a register family (all widths): a read of ANY of
/// them after the relay would see a value the retargeted load no longer
/// provides, so all of them must veto the fuse.
fn reg_names_family(fam: u8) -> &'static [&'static str] {
    match fam {
        0 => &["%rax", "%eax", "%ax", "%al", "%ah"],
        1 => &["%rcx", "%ecx", "%cx", "%cl", "%ch"],
        2 => &["%rdx", "%edx", "%dx", "%dl", "%dh"],
        3 => &["%rbx", "%ebx", "%bx", "%bl", "%bh"],
        4 => &["%rsp", "%esp", "%sp", "%spl"],
        5 => &["%rbp", "%ebp", "%bp", "%bpl"],
        6 => &["%rsi", "%esi", "%si", "%sil"],
        7 => &["%rdi", "%edi", "%di", "%dil"],
        8 => &["%r8", "%r8d", "%r8w", "%r8b"],
        9 => &["%r9", "%r9d", "%r9w", "%r9b"],
        10 => &["%r10", "%r10d", "%r10w", "%r10b"],
        11 => &["%r11", "%r11d", "%r11w", "%r11b"],
        12 => &["%r12", "%r12d", "%r12w", "%r12b"],
        13 => &["%r13", "%r13d", "%r13w", "%r13b"],
        14 => &["%r14", "%r14d", "%r14w", "%r14b"],
        15 => &["%r15", "%r15d", "%r15w", "%r15b"],
        _ => &[],
    }
}

#[cfg(test)]
mod fold_load_copy_relay_tests {
    use super::super::super::types::classify_line;
    use super::fold_load_copy_relay;
    use crate::backend::peephole_common::LineStore;

    fn run(asm: &str) -> (bool, Vec<String>) {
        let mut store = LineStore::new(asm.to_string());
        let n = store.len();
        let mut infos: Vec<_> = (0..n).map(|i| classify_line(store.get(i))).collect();
        let changed = fold_load_copy_relay(&mut store, &mut infos);
        let out = (0..store.len()).map(|i| store.get(i).to_string()).collect();
        (changed, out)
    }

    /// Regression: gzip 1.14 pqdownheap prologue. After a redundant-copy
    /// elimination the sequence is:
    ///   movslq 40(%rsp), %rax ; movq %rax, %rbp ; cltq ; movq %rax, 16(%rsp)
    /// `cltq` implicitly READS %eax (the loaded value's low 32 bits) and then
    /// writes %rax. Treating it as a pure write to %rax let the pass retarget
    /// the load to %rbp, leaving `cltq` reading undefined %eax and storing
    /// garbage (SIGBUS in pqdownheap / SIGSEGV in gen_codes).
    #[test]
    fn refuses_fuse_when_cltq_reads_loaded_register() {
        let asm = "\
    movslq 40(%rsp), %rax\n\
    movq %rax, %rbp\n\
    cltq\n\
    movq %rax, 16(%rsp)\n\
    leaq heap(%rip), %r9\n\
";
        let (changed, _) = run(asm);
        assert!(!changed, "fuse must be refused: cltq reads %eax implicitly");
    }

    /// A plain dead copy with no implicit reader MAY be fused.
    #[test]
    fn fuses_when_loaded_register_is_truly_dead() {
        let asm = "\
    movslq 40(%rsp), %rax\n\
    movq %rax, %rbp\n\
    movslq 8(%rsp), %rax\n\
    movslq heap_len(%rip), %r10\n\
";
        let (changed, out) = run(asm);
        assert!(changed, "safe relay should fuse");
        assert!(
            out[0].contains("%rbp"),
            "load retargeted to copy dest: {out:?}"
        );
    }

    /// Shifts reading %cl must veto a relay of a %rcx-family load.
    #[test]
    fn refuses_fuse_when_shld_reads_cl() {
        let asm = "\
    movslq 8(%rsp), %rcx\n\
    movq %rcx, %rdx\n\
    shldq $1, %rdx, %rax\n\
";
        let (changed, _) = run(asm);
        assert!(!changed, "shld reads %cl implicitly; fuse must be refused");
    }
}

#[cfg(test)]
mod fma_mem_fold_tests {
    use super::super::super::types::classify_line;
    use super::fold_fma_memory_src2;
    use crate::backend::peephole_common::LineStore;

    fn run(asm: &str) -> (bool, Vec<String>) {
        let mut store = LineStore::new(asm.to_string());
        let n = store.len();
        let mut infos: Vec<_> = (0..n).map(|i| classify_line(store.get(i))).collect();
        let changed = fold_fma_memory_src2(&mut store, &mut infos);
        let out: Vec<String> = (0..store.len())
            .filter(|i| !infos[*i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect();
        (changed, out)
    }

    /// The canonical dot-product shape: single-use b-element load folded
    /// into the FMA's memory src2 slot.
    #[test]
    fn folds_single_use_load_into_fma_src2() {
        let (changed, out) = run("    movsd 32(%rsi), %xmm11\n\
             \x20   vfmadd231sd %xmm11, %xmm10, %xmm2\n");
        assert!(changed, "single-use load should fold");
        assert_eq!(out.len(), 1, "load must be gone: {out:?}");
        assert_eq!(out[0], "vfmadd231sd 32(%rsi), %xmm10, %xmm2");
    }

    /// A later mention of the loaded register vetoes the fold — the value
    /// is still needed after the FMA.
    #[test]
    fn refuses_when_register_mentioned_later() {
        let (changed, _) = run("    movsd 32(%rsi), %xmm11\n\
             \x20   vfmadd231sd %xmm11, %xmm10, %xmm2\n\
             \x20   vaddsd %xmm11, %xmm0, %xmm0\n");
        assert!(!changed, "later reader must veto the fold");
    }

    /// A later FULL overwrite of the loaded register kills the loaded
    /// value, so the fold is sound: everything past the overwrite uses
    /// the new value.  This is the register-reuse shape real allocators
    /// emit constantly (the dot8d SSE2 kernel reuses %xmm3 for the last
    /// a-element load).
    #[test]
    fn folds_when_next_mention_fully_overwrites() {
        let (changed, out) = run("    movsd 32(%rsi), %xmm11\n\
             \x20   vfmadd231sd %xmm11, %xmm10, %xmm2\n\
             \x20   movsd 40(%rdi), %xmm11\n");
        assert!(changed, "pure overwrite kills the loaded value");
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0], "vfmadd231sd 32(%rsi), %xmm10, %xmm2");
        assert_eq!(out[1], "movsd 40(%rdi), %xmm11");
    }

    /// SIB addressing has commas of its own; the destination check must
    /// look past them to the final operand.
    #[test]
    fn folds_when_overwrite_uses_sib_addressing() {
        let (changed, out) = run("    movsd 32(%rsi), %xmm11\n\
             \x20   vfmadd231sd %xmm11, %xmm10, %xmm2\n\
             \x20   vmovsd 32(%rdi,%rax,8), %xmm11\n");
        assert!(changed, "SIB-addressed overwrite still kills the value");
        assert_eq!(out[0], "vfmadd231sd 32(%rsi), %xmm10, %xmm2");
        assert_eq!(out.len(), 2, "{out:?}");
    }

    /// A self-zeroing xor is a full overwrite too.
    #[test]
    fn folds_when_next_mention_is_self_zeroing_xor() {
        let (changed, _) = run("    movsd 32(%rsi), %xmm11\n\
             \x20   vfmadd231sd %xmm11, %xmm10, %xmm2\n\
             \x20   pxor %xmm11, %xmm11\n");
        assert!(changed, "pxor zeroes the whole register");
    }

    /// A STORE mentioning the register reads it — not an overwrite.
    #[test]
    fn refuses_when_next_mention_is_a_store() {
        let (changed, _) = run("    movsd 32(%rsi), %xmm11\n\
             \x20   vfmadd231sd %xmm11, %xmm10, %xmm2\n\
             \x20   movsd %xmm11, (%rax)\n");
        assert!(!changed, "a store reads the loaded value");
    }

    /// A partial write (movhpd keeps the low half) does not kill the
    /// loaded value.
    #[test]
    fn refuses_when_next_mention_partially_writes() {
        let (changed, _) = run("    movsd 32(%rsi), %xmm11\n\
             \x20   vfmadd231sd %xmm11, %xmm10, %xmm2\n\
             \x20   movhpd (%rax), %xmm11\n");
        assert!(!changed, "movhpd leaves the low lane untouched");
    }

    /// RED-TEAM REGRESSION (PR #359 follow-up): the three-mention scan must
    /// apply the same merge-write rule as the two-mention proof — a reg-reg
    /// scalar `vmovsd` past load+consumer is NOT a full overwrite (it
    /// preserves the high bits the deleted load zeroed).
    #[test]
    fn refuses_reg_reg_scalar_movsd_merge_after_consumer() {
        let (changed, _) = run("    movapd %xmm9, %xmm11\n\
             \x20   movsd 32(%rsi), %xmm11\n\
             \x20   vfmadd231sd %xmm11, %xmm10, %xmm2\n\
             \x20   vmovsd %xmm12, %xmm11\n\
             \x20   vmovapd %xmm11, %xmm5\n");
        assert!(
            !changed,
            "merge write preserves high bits from the deleted load"
        );
    }

    /// dest == loaded register means the FMA line mentions it twice; the
    /// two-mention budget is exhausted, so the fold is refused (sound even
    /// though the rewrite would actually be valid).
    #[test]
    fn refuses_when_dest_equals_loaded_reg() {
        let (changed, _) = run("    movsd (%rsi), %xmm2\n\
             \x20   vfmadd231sd %xmm2, %xmm3, %xmm2\n");
        assert!(!changed, "dest==src2 shape must be refused");
    }

    /// Mentions BEFORE the load are irrelevant (the load redefines the
    /// register), so a home reused across iterations stays foldable.
    #[test]
    fn folds_reused_home_when_later_use_is_last() {
        let (changed, out) = run("    movsd (%rsi), %xmm5\n\
             \x20   vfmadd213sd %xmm0, %xmm5, %xmm2\n\
             \x20   movsd 56(%rsi), %xmm5\n\
             \x20   vfmadd231sd %xmm5, %xmm3, %xmm8\n\
             \x20   ret\n");
        assert!(changed, "second (last-def) load should fold");
        assert!(
            out.iter()
                .any(|l| l == "vfmadd231sd 56(%rsi), %xmm3, %xmm8"),
            "expected folded second pair: {out:?}"
        );
        assert!(
            out.iter().any(|l| l == "movsd (%rsi), %xmm5"),
            "first pair must stay untouched: {out:?}"
        );
    }

    /// A call after the pair can implicitly read %xmm0-%xmm7 (variadic FP
    /// arguments) — veto for registers in the argument range.
    #[test]
    fn refuses_when_call_intervenes_for_arg_reg() {
        let (changed, _) = run("    movsd (%rsi), %xmm5\n\
             \x20   vfmadd231sd %xmm5, %xmm3, %xmm8\n\
             \x20   call printf\n");
        assert!(!changed, "call may read %xmm5 as variadic arg");
    }

    /// Same shape with a register outside the argument range: xmm11 is
    /// never an implicit call operand, so the fold is safe.
    #[test]
    fn folds_past_call_for_non_arg_reg() {
        let (changed, out) = run("    movsd (%rsi), %xmm11\n\
             \x20   vfmadd231sd %xmm11, %xmm3, %xmm8\n\
             \x20   call printf\n");
        assert!(changed, "xmm11 cannot be a variadic arg");
        assert_eq!(out[0], "vfmadd231sd (%rsi), %xmm3, %xmm8");
    }

    /// Token-bounded matching: a later `%xmm11` mention is NOT a mention
    /// of `%xmm1` and must not veto the fold.
    #[test]
    fn token_boundary_distinguishes_xmm1_from_xmm11() {
        let (changed, out) = run("    movsd (%rsi), %xmm1\n\
             \x20   vfmadd231sd %xmm1, %xmm3, %xmm8\n\
             \x20   movsd (%rdi), %xmm11\n");
        assert!(changed, "%xmm11 must not count as %xmm1");
        assert_eq!(out[0], "vfmadd231sd (%rsi), %xmm3, %xmm8");
    }

    /// Width mismatch (movss load feeding an -sd FMA) is not our pattern.
    #[test]
    fn refuses_width_mismatch() {
        let (changed, _) = run("    movss (%rsi), %xmm11\n\
             \x20   vfmadd231sd %xmm11, %xmm3, %xmm8\n");
        assert!(!changed, "ss load must not fold into sd fma");
    }

    /// Only 231 forms have a memory-legal src2; a 213 form's first operand
    /// is the addend register, not the rm slot — must not fold.
    #[test]
    fn refuses_non_231_form() {
        let (changed, _) = run("    movsd (%rsi), %xmm11\n\
             \x20   vfmadd213sd %xmm11, %xmm3, %xmm8\n");
        assert!(!changed, "213 form has no memory src2 slot");
    }

    /// A label between the load and the FMA means the FMA is reachable
    /// without the load — adjacency (NOPs aside) is mandatory.
    #[test]
    fn refuses_when_label_between() {
        let (changed, _) = run("    movsd (%rsi), %xmm11\n\
             .L5:\n\
             \x20   vfmadd231sd %xmm11, %xmm3, %xmm8\n");
        assert!(!changed, "label breaks adjacency");
    }

    /// A BRANCH between the FMA and the "killing" overwrite breaks the
    /// whole-function textual proof: when the branch is taken, %xmm5 still
    /// holds the LOADED value at the merge, and the reader past `.Ldone`
    /// depends on it.  The overwrite only dominates the fall-through path,
    /// so the fold must be refused even though the first later mention is a
    /// pure full-width overwrite.
    #[test]
    fn refuses_when_branch_bypasses_the_killing_overwrite() {
        let (changed, out) = run("    movsd 32(%rsi), %xmm5\n\
             \x20   vfmadd231sd %xmm5, %xmm10, %xmm2\n\
             \x20   jne .Ldone\n\
             \x20   vmovsd 40(%rdi), %xmm5\n\
             .Ldone:\n\
             \x20   vaddsd %xmm5, %xmm3, %xmm3\n\
             \x20   ret\n");
        assert!(!changed, "taken path reads the loaded value: {out:?}");
    }
}

#[cfg(test)]
mod fma132_zero_tests {
    use super::super::super::types::classify_line;
    use super::{eliminate_redundant_xmm0_zeroing, fold_zero_addend_fma213_to_132};
    use crate::backend::peephole_common::LineStore;

    fn run(asm: &str) -> (bool, Vec<String>) {
        let mut store = LineStore::new(asm.to_string());
        let n = store.len();
        let mut infos: Vec<_> = (0..n).map(|i| classify_line(store.get(i))).collect();
        let mut changed = fold_zero_addend_fma213_to_132(&mut store, &mut infos);
        changed |= eliminate_redundant_xmm0_zeroing(&mut store, &mut infos);
        let out: Vec<String> = (0..store.len())
            .filter(|i| !infos[*i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect();
        (changed, out)
    }

    /// The canonical dot-product first-iteration shape: the b load folds
    /// into the 132-form memory slot, the zeroing stays for the FMA.
    #[test]
    fn folds_zero_addend_triple_to_132() {
        let (changed, out) = run("    movsd (%rsi), %xmm3\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm3, %xmm2\n");
        assert!(changed, "triple must fold");
        assert_eq!(out.len(), 2, "b load must be gone: {out:?}");
        assert_eq!(out[0], "xorpd %xmm0, %xmm0");
        assert_eq!(out[1], "vfmadd132sd (%rsi), %xmm0, %xmm2");
    }

    /// A later use of the loaded register vetoes the fold.
    #[test]
    fn refuses_when_multiplier_used_later() {
        let (changed, _) = run("    movsd (%rsi), %xmm3\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm3, %xmm2\n\
             \x20   vaddsd %xmm3, %xmm2, %xmm2\n");
        assert!(!changed, "later reader must veto");
    }

    /// Without the zeroing line between, the 132 rewrite would read an
    /// unproven %xmm0 — refuse.
    #[test]
    fn refuses_without_zeroing_line() {
        let (changed, _) = run("    movsd (%rsi), %xmm3\n\
             \x20   vfmadd213sd %xmm0, %xmm3, %xmm2\n");
        assert!(!changed, "missing zeroing must veto");
    }

    /// A label between load and zeroing breaks adjacency (the FMA could be
    /// entered without the load).
    #[test]
    fn refuses_when_label_between() {
        let (changed, _) = run("    movsd (%rsi), %xmm3\n\
             .L5:\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm3, %xmm2\n");
        assert!(!changed, "label breaks adjacency");
    }

    /// The dot8 first-iteration shape: four triples collapse to ONE zeroing
    /// plus four 132-form FMAs (the b loads all fold, the repeated xorpds
    /// die block-locally).
    #[test]
    fn dot8_first_iteration_collapses_to_one_zero() {
        let (changed, out) = run("    movsd (%rsi), %xmm3\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm3, %xmm2\n\
             \x20   movsd 8(%rsi), %xmm5\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm5, %xmm4\n\
             \x20   movsd 16(%rsi), %xmm7\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm7, %xmm6\n\
             \x20   movsd 24(%rsi), %xmm9\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm9, %xmm8\n");
        assert!(changed);
        let zeros = out.iter().filter(|l| *l == "xorpd %xmm0, %xmm0").count();
        assert_eq!(zeros, 1, "exactly one resident zero: {out:?}");
        let fma132 = out.iter().filter(|l| l.starts_with("vfmadd132sd")).count();
        assert_eq!(fma132, 4, "all four must be 132-form: {out:?}");
    }

    /// The zero state dies at a call: the zeroing after it must stay.
    #[test]
    fn zero_state_dies_at_call() {
        let (_, out) = run("    xorpd %xmm0, %xmm0\n\
             \x20   call foo\n\
             \x20   xorpd %xmm0, %xmm0\n");
        let zeros = out.iter().filter(|l| *l == "xorpd %xmm0, %xmm0").count();
        assert_eq!(zeros, 2, "call clobbers the zero: {out:?}");
    }

    /// Reads of %xmm0 (as an FMA addend) preserve the zero state; a write
    /// (last operand) kills it.
    #[test]
    fn zero_state_survives_reads_dies_on_writes() {
        let (_, out) = run("    xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm3, %xmm2\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vaddsd %xmm1, %xmm0\n\
             \x20   xorpd %xmm0, %xmm0\n");
        let zeros = out.iter().filter(|l| *l == "xorpd %xmm0, %xmm0").count();
        // first stays; second dies (still zero after the read); third stays
        // (the 2-operand vaddsd wrote %xmm0).
        assert_eq!(zeros, 2, "read preserves, write kills: {out:?}");
    }
}

#[cfg(test)]
mod fma132_reuse_tests {
    use super::super::super::types::classify_line;
    use super::fold_zero_addend_fma213_to_132;
    use crate::backend::peephole_common::LineStore;

    fn run(asm: &str) -> (bool, Vec<String>) {
        let mut store = LineStore::new(asm.to_string());
        let n = store.len();
        let mut infos: Vec<_> = (0..n).map(|i| classify_line(store.get(i))).collect();
        let changed = fold_zero_addend_fma213_to_132(&mut store, &mut infos);
        let out: Vec<String> = (0..store.len())
            .filter(|i| !infos[*i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect();
        (changed, out)
    }

    /// The dot8 iteration-2 register-reuse shape: %xmm3 is later redefined
    /// by a pure load, so the first-iteration fold is still sound.
    #[test]
    fn folds_when_multiplier_redefined_later() {
        let (changed, out) = run("    movsd (%rsi), %xmm3\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm3, %xmm2\n\
             \x20   movsd 56(%rdi), %xmm3\n\
             \x20   vfmadd231sd 56(%rsi), %xmm3, %xmm8\n");
        assert!(changed, "pure later redefinition must allow the fold");
        assert!(
            out.iter().any(|l| l == "vfmadd132sd (%rsi), %xmm0, %xmm2"),
            "expected 132 form: {out:?}"
        );
    }

    /// A later read-modify-write whose DEST is the multiplier still reads
    /// the loaded value — must veto.
    #[test]
    fn refuses_rmw_on_multiplier() {
        let (changed, _) = run("    movsd (%rsi), %xmm3\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm3, %xmm2\n\
             \x20   vaddsd %xmm1, %xmm3, %xmm3\n");
        assert!(!changed, "rmw reads the loaded value");
    }

    /// Same dominance gap as the other two folds: a branch between the FMA
    /// and the later redefinition means the branch-taken path reaches the
    /// merge reader with the LOADED multiplier intact.  The textual
    /// whole-function scan must refuse despite the pure overwrite being the
    /// first later mention.
    #[test]
    fn refuses_when_branch_between_fma_and_redefinition() {
        let (changed, out) = run("    movsd (%rsi), %xmm3\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm3, %xmm2\n\
             \x20   jne .Ldone\n\
             \x20   movsd 56(%rdi), %xmm3\n\
             .Ldone:\n\
             \x20   vaddsd %xmm3, %xmm0, %xmm0\n\
             \x20   ret\n");
        assert!(!changed, "taken path reads the loaded multiplier: {out:?}");
    }

    /// Same call-as-argument-reader gap as fold_fp_register_loads: the
    /// loaded multiplier %xmm1 survives the FMA (dest %xmm2), and a later
    /// call may read it implicitly as its second FP argument.  The first
    /// TEXTUAL mention being a pure overwrite does not make the value
    /// dead — the call in between reads it without mentioning it.
    #[test]
    fn refuses_when_call_reads_loaded_multiplier_register() {
        let (changed, out) = run("    movsd (%rsi), %xmm1\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm1, %xmm2\n\
             \x20   call foo\n\
             \x20   vmovapd %xmm3, %xmm1\n");
        assert!(!changed, "the call may read %xmm1 as its argument: {out:?}");
    }

    /// The veto is call-specific: a non-call line in the stretch leaves
    /// the fold enabled.
    #[test]
    fn still_folds_when_non_call_intervenes() {
        let (changed, out) = run("    movsd (%rsi), %xmm1\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm1, %xmm2\n\
             \x20   vaddsd %xmm4, %xmm3, %xmm3\n\
             \x20   vmovapd %xmm3, %xmm1\n");
        assert!(
            changed,
            "no implicit reader of %xmm1 in the stretch: {out:?}"
        );
    }

    /// A call is harmless when the multiplier lives in the callee-saved
    /// half: %xmm9 is never an implicit call operand.
    #[test]
    fn still_folds_when_call_does_not_read_callee_saved_reg() {
        let (changed, out) = run("    movsd (%rsi), %xmm9\n\
             \x20   xorpd %xmm0, %xmm0\n\
             \x20   vfmadd213sd %xmm0, %xmm9, %xmm2\n\
             \x20   call foo\n\
             \x20   vmovapd %xmm3, %xmm9\n");
        assert!(changed, "xmm9 is not an implicit call operand: {out:?}");
    }
}

#[cfg(test)]
mod fp_reg_load_tests {
    use super::super::super::types::classify_line;
    use super::fold_fp_register_loads;
    use crate::backend::peephole_common::LineStore;

    fn run(asm: &str) -> (bool, Vec<String>) {
        let mut store = LineStore::new(asm.to_string());
        let n = store.len();
        let mut infos: Vec<_> = (0..n).map(|i| classify_line(store.get(i))).collect();
        let changed = fold_fp_register_loads(&mut store, &mut infos);
        let out: Vec<String> = (0..store.len())
            .filter(|i| !infos[*i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect();
        (changed, out)
    }

    /// A Call is an OPAQUE reader of %xmm0-%xmm7 (the SysV FP argument
    /// registers): when the loaded value is passed to the call, the caller
    /// reads it implicitly and the read is invisible to a textual scan
    /// ("call foo" mentions no register — the argument move is a self-move
    /// the builder elided).  The later full overwrite therefore does NOT
    /// prove the loaded value dead: the call in between would read a stale
    /// register.  (Unreachable through the current per-use constant
    /// materialization; pinned as defense in depth so the contract holds
    /// if the isel ever shares one load across uses.)
    #[test]
    fn refuses_when_call_reads_loaded_arg_register() {
        let (changed, out) = run("    movsd (%rsi), %xmm0\n\
             \x20   vaddsd %xmm0, %xmm5, %xmm5\n\
             \x20   call foo\n\
             \x20   vmovapd %xmm1, %xmm0\n");
        assert!(!changed, "the call may read %xmm0 as its argument: {out:?}");
    }

    /// Same stretch with a non-call line in place of the call: the fold
    /// must stay enabled (the veto is call-specific, not a general
    /// straight-line restriction).
    #[test]
    fn still_folds_when_non_call_intervenes() {
        let (changed, out) = run("    movsd (%rsi), %xmm0\n\
             \x20   vaddsd %xmm0, %xmm5, %xmm5\n\
             \x20   vaddsd %xmm4, %xmm2, %xmm2\n\
             \x20   vmovapd %xmm1, %xmm0\n");
        assert!(
            changed,
            "no implicit reader between load and overwrite: {out:?}"
        );
    }

    /// Calls are harmless for the callee-saved half: %xmm8-%xmm15 are never
    /// argument registers and compiled code never reads one before
    /// defining it, so the kill proof holds across the call.
    #[test]
    fn still_folds_when_call_does_not_read_callee_saved_reg() {
        let (changed, out) = run("    movsd (%rsi), %xmm9\n\
             \x20   vaddsd %xmm9, %xmm5, %xmm5\n\
             \x20   call foo\n\
             \x20   vmovapd %xmm1, %xmm9\n");
        assert!(changed, "xmm9 is not an implicit call operand: {out:?}");
    }

    /// The dot8d SSE2 shape: %xmm3 carries the b0 load, the consumer is
    /// adjacent, and %xmm3 is later REUSED by a full-overwrite load of
    /// the last a element.  The reuse kills the loaded value, so the
    /// fold is sound.
    #[test]
    fn folds_when_register_reused_by_later_overwrite() {
        let (changed, out) = run("    movsd (%rsi), %xmm3\n\
             \x20   vmulsd %xmm3, %xmm2, %xmm2\n\
             \x20   vaddsd %xmm4, %xmm2, %xmm2\n\
             \x20   movsd 0x38(%rdi), %xmm3\n");
        assert!(changed, "later full overwrite kills the loaded value");
        assert_eq!(out[0], "vmulsd (%rsi), %xmm2, %xmm2");
        assert_eq!(out.len(), 3, "{out:?}");
    }

    /// A later reader vetoes the fold even with the refinement.
    #[test]
    fn refuses_when_later_line_reads_register() {
        let (changed, _) = run("    movsd (%rsi), %xmm3\n\
             \x20   vmulsd %xmm3, %xmm2, %xmm2\n\
             \x20   vaddsd %xmm3, %xmm0, %xmm0\n");
        assert!(!changed, "later reader must veto");
    }

    /// Register-token boundary: a later mention of %xmm11 is NOT a
    /// mention of %xmm1 — the scan must run past it and fold.
    #[test]
    fn folds_when_later_line_mentions_wider_numbered_register() {
        let (changed, out) = run("    movsd (%rsi), %xmm1\n\
             \x20   vmulsd %xmm1, %xmm2, %xmm2\n\
             \x20   movsd (%rdi), %xmm11\n");
        assert!(changed, "%xmm11 does not mention %xmm1");
        assert_eq!(out[0], "vmulsd (%rsi), %xmm2, %xmm2");
    }

    /// A read-modify-write on the loaded register reads the old value —
    /// not a pure overwrite.
    #[test]
    fn refuses_when_reuse_is_read_modify_write() {
        let (changed, _) = run("    movsd (%rsi), %xmm3\n\
             \x20   vmulsd %xmm3, %xmm2, %xmm2\n\
             \x20   vaddsd %xmm5, %xmm3, %xmm3\n");
        assert!(!changed, "rmw reads the loaded value before writing");
    }

    /// RED-TEAM REGRESSION (PR #359 follow-up): a reg-reg scalar `vmovsd`
    /// is a MERGE write — it preserves bits 64:127 of the destination.
    /// Here those preserved bits come from the load being deleted (a
    /// memory-source `movsd` zeroes them), and the final packed `vmovapd`
    /// observes them through the merge chain. Deleting the load therefore
    /// changes observable state; the fold must be refused.
    #[test]
    fn refuses_when_next_mention_is_reg_reg_scalar_movsd_merge() {
        let (changed, _) = run("    movapd %xmm9, %xmm11\n\
             \x20   movsd 32(%rsi), %xmm11\n\
             \x20   vmulsd %xmm11, %xmm2, %xmm2\n\
             \x20   vmovsd %xmm12, %xmm11\n\
             \x20   vmovapd %xmm11, %xmm5\n");
        assert!(
            !changed,
            "reg-reg vmovsd preserves the high half — not a full overwrite"
        );
    }

    /// Same shape, single-precision: a reg-reg `movss` merges bits 32:127.
    #[test]
    fn refuses_when_next_mention_is_reg_reg_scalar_movss_merge() {
        let (changed, _) = run("    movaps %xmm9, %xmm11\n\
             \x20   movss 32(%rsi), %xmm11\n\
             \x20   vmulss %xmm11, %xmm2, %xmm2\n\
             \x20   movss %xmm12, %xmm11\n\
             \x20   movaps %xmm11, %xmm5\n");
        assert!(
            !changed,
            "reg-reg movss merges bits 32:127 — not an overwrite"
        );
    }

    /// Memory-source scalar moves DO define the full register (the upper
    /// bits are zeroed), so the accept side of the refinement is preserved
    /// even with a packed read after the merge.
    #[test]
    fn folds_when_next_mention_is_memory_source_scalar_movsd() {
        let (changed, out) = run("    movapd %xmm9, %xmm11\n\
             \x20   movsd 32(%rsi), %xmm11\n\
             \x20   vmulsd %xmm11, %xmm2, %xmm2\n\
             \x20   movsd 40(%rdi), %xmm11\n\
             \x20   vmovapd %xmm11, %xmm5\n");
        assert!(
            changed,
            "memory-source movsd zeroes the high half: full overwrite"
        );
        assert_eq!(out[1], "vmulsd 32(%rsi), %xmm2, %xmm2", "{out:?}");
    }

    /// A BRANCH between the consumer and the pure overwrite invalidates the
    /// textual kill proof: the taken path reaches the reader past the merge
    /// with the LOADED value still in %xmm3, while the overwrite only runs
    /// on the fall-through path.  Deleting the load corrupts the taken path.
    #[test]
    fn refuses_when_branch_skips_the_killing_overwrite() {
        let (changed, out) = run("    movsd (%rsi), %xmm3\n\
             \x20   vmulsd %xmm3, %xmm2, %xmm2\n\
             \x20   jne .Ldone\n\
             \x20   vmovsd 40(%rdi), %xmm3\n\
             .Ldone:\n\
             \x20   vaddsd %xmm3, %xmm0, %xmm0\n\
             \x20   ret\n");
        assert!(!changed, "taken path reads the loaded value: {out:?}");
    }
}

#[cfg(test)]
mod fp_const_hoist_tests {
    use super::super::super::types::{classify_line, mark_nop, LineInfo, LineKind};
    use super::hoist_repeated_fp_constant_loads;
    use crate::backend::peephole_common::LineStore;

    /// Run the pass over `asm` after marking `nop_lines` dead and applying
    /// `mutate` to the classification vector (for InlineAsm injection).
    fn run_with(
        asm: &str,
        nop_lines: &[usize],
        mutate: impl FnOnce(&mut [LineInfo]),
    ) -> (bool, Vec<String>) {
        let mut store = LineStore::new(asm.to_string());
        let n = store.len();
        let mut infos: Vec<_> = (0..n).map(|i| classify_line(store.get(i))).collect();
        for &i in nop_lines {
            mark_nop(&mut infos[i]);
        }
        mutate(&mut infos);
        let changed = hoist_repeated_fp_constant_loads(&mut store, &mut infos);
        let out: Vec<String> = (0..store.len())
            .filter(|i| !infos[*i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect();
        (changed, out)
    }

    fn run(asm: &str, nop_lines: &[usize]) -> (bool, Vec<String>) {
        run_with(asm, nop_lines, |_| {})
    }

    /// The dot8d SSE2 shape: the constant zero is re-read at every use;
    /// a dead slot before the first use hosts the single materialization.
    #[test]
    fn hoists_repeated_constant_into_free_register() {
        let (changed, out) = run(
            "    movsd (%rdi), %xmm2\n\
             \x20   movsd (%rsi), %xmm3\n\
             \x20   vmulsd %xmm3, %xmm2, %xmm2\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm2, %xmm2\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm4, %xmm4\n",
            &[1],
        );
        assert!(changed, "two constant reads must hoist");
        assert_eq!(
            out[1], "movsd .LCFP_0(%rip), %xmm0",
            "slot hosts load: {out:?}"
        );
        assert_eq!(out[3], "vaddsd %xmm0, %xmm2, %xmm2");
        assert_eq!(out[4], "vaddsd %xmm0, %xmm4, %xmm4");
    }

    /// Without a dead slot there is nowhere to materialize.
    #[test]
    fn refuses_without_a_dead_slot() {
        let (changed, _) = run(
            "    vaddsd .LCFP_0(%rip), %xmm2, %xmm2\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm4, %xmm4\n",
            &[],
        );
        assert!(!changed, "no NOP slot, no hoist");
    }

    /// A label between slot and uses breaks fall-through dominance.
    #[test]
    fn refuses_when_a_label_separates_slot_and_uses() {
        let (changed, _) = run(
            "    movsd (%rsi), %xmm3\n\
             \x20L5:\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm2, %xmm2\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm4, %xmm4\n",
            &[0],
        );
        assert!(!changed, "jump could enter past the materialization");
    }

    /// Any call kills every caller-saved xmm after it.
    #[test]
    fn refuses_when_function_contains_a_call() {
        let (changed, _) = run(
            "    movsd (%rsi), %xmm3\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm2, %xmm2\n\
             \x20   call foo\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm4, %xmm4\n",
            &[0],
        );
        assert!(!changed, "calls clobber all xmm registers");
    }

    /// One use: the materialization would only add an instruction.
    #[test]
    fn refuses_when_only_one_use() {
        let (changed, _) = run(
            "    movsd (%rsi), %xmm3\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm2, %xmm2\n",
            &[0],
        );
        assert!(!changed, "single use must not hoist");
    }

    /// blendv-family instructions read %xmm0 implicitly.
    #[test]
    fn refuses_with_blendv_present() {
        let (changed, _) = run(
            "    movsd (%rsi), %xmm3\n\
             \x20   pblendvb %xmm7, %xmm6\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm2, %xmm2\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm4, %xmm4\n",
            &[0],
        );
        assert!(!changed, "implicit xmm0 reader vetoes");
    }

    /// Opaque inline asm may touch any register.
    #[test]
    fn refuses_with_inline_asm() {
        let (changed, _) = run_with(
            "    movsd (%rsi), %xmm3\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm2, %xmm2\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm4, %xmm4\n",
            &[0],
            |infos| infos[1].kind = LineKind::InlineAsm,
        );
        assert!(!changed, "inline asm vetoes");
    }

    /// Every xmm mentioned somewhere: no free materialization register.
    #[test]
    fn refuses_when_all_registers_mentioned() {
        let mut asm = String::from("    movsd (%rsi), %xmm3\n");
        for n in 0..16 {
            asm.push_str(&format!("    vmovsd %xmm{}, (%rsp)\n", n));
        }
        asm.push_str("    vaddsd .LCFP_0(%rip), %xmm2, %xmm2\n");
        asm.push_str("    vaddsd .LCFP_0(%rip), %xmm4, %xmm4\n");
        let (changed, _) = run(&asm, &[0]);
        assert!(!changed, "no free register, no hoist");
    }

    /// NOP'd lines' mentions still count: later passes may revive the slot
    /// text, so a register mentioned only in dead lines is NOT free.
    #[test]
    fn dead_line_mentions_still_block_a_register() {
        let (changed, out) = run(
            "    movsd (%rsi), %xmm3\n\
             \x20   movsd (%rax), %xmm0\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm2, %xmm2\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm4, %xmm4\n",
            &[0, 1],
        );
        assert!(changed, "still hoists with another register");
        assert_eq!(
            out[0], "movsd .LCFP_0(%rip), %xmm1",
            "xmm0 is claimed by the dead line: {out:?}"
        );
    }

    /// RED-TEAM REGRESSION (PR #359 follow-up): one rmw line mentions the
    /// token twice (`vaddsd .LCFP_0(%rip), %xmm2, %xmm2`); the site list
    /// must be deduplicated so a SINGLE rewritten line does not pass the
    /// "at least two rewritten uses" threshold — hoisting there keeps the
    /// static instruction count identical and adds a dynamic pool load.
    #[test]
    fn refuses_single_use_line_mentioned_twice() {
        let (changed, _) = run(
            "    movsd (%rdi), %xmm2\n\
             \x20   vaddsd .LCFP_0(%rip), %xmm2, %xmm2\n",
            &[0],
        );
        assert!(!changed, "one distinct use line is below the >=2 threshold");
    }

    /// Single-precision-only sites materialize with `movss` (an exact
    /// 4-byte pool read); any double-precision site keeps `movsd`.
    #[test]
    fn materializes_movss_for_single_precision_sites() {
        let (changed, out) = run(
            "    movss (%rdi), %xmm2\n\
             \x20   movss (%rsi), %xmm3\n\
             \x20   vmulss %xmm3, %xmm2, %xmm2\n\
             \x20   vaddss .LCFP_0(%rip), %xmm2, %xmm2\n\
             \x20   vaddss .LCFP_0(%rip), %xmm4, %xmm4\n",
            &[1],
        );
        assert!(changed, "two constant reads must hoist");
        assert_eq!(
            out[1], "movss .LCFP_0(%rip), %xmm0",
            "single-precision pool read: {out:?}"
        );
    }
}
