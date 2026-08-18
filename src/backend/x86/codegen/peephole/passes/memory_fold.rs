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
use super::helpers::{get_dest_reg, is_read_modify_write, is_rsp_shift_line, implicit_read_reg_family};

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
    if b.len() < 6 { return None; }

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
///   movsd -M(%rbp), %xmm1   ; Other{dest_reg: 25 (xmm1), rbp_offset: -M}
///   OP %xmm1, %xmm0          ; Other{dest_reg: 24 (xmm0)}, OP ∈ {mulsd, addsd, subsd, divsd}
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
        if infos[i].is_nop() { i += 1; continue; }

        // Look for Other{dest_reg: 25} = writes to %xmm1 (family 24+1=25)
        if let LineKind::Other { dest_reg: 25 } = infos[i].kind {
            let offset = infos[i].rbp_offset;
            if offset == RBP_OFFSET_NONE { i += 1; continue; }

            let line_i = infos[i].trimmed(store.get(i));
            // Verify it is a movsd load from stack (not another xmm1-writing insn)
            if !line_i.starts_with("movsd ") || !line_i.ends_with(", %xmm1") {
                i += 1; continue;
            }

            // Find next non-NOP (skip only NOPs, not other instructions)
            let mut j = i + 1;
            while j < len && j < i + 4 && infos[j].is_nop() { j += 1; }
            if j >= len { i += 1; continue; }

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

/// Fold a single-use scalar-FP register load into an adjacent VEX arithmetic
/// instruction.
///
///     movsd 8(%rsi), %xmm5
///     vsubsd %xmm5, %xmm4, %xmm4
///       ->
///     vsubsd 8(%rsi), %xmm4, %xmm4
///
/// This deliberately uses a stronger-than-necessary liveness proof: the loaded
/// XMM register must not be mentioned again before the function's `.size`.
/// That misses registers which are overwritten later, but makes deleting the
/// defining load safe without teaching the text peephole full XMM dataflow.
/// The load and consumer must be adjacent, so no address register or memory
/// state can change between them.  Source==destination is rejected because the
/// removed load also supplies the destructive destination's old value.
pub(super) fn fold_fp_register_loads(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    fn mentions_xmm(line: &str, reg: &str) -> bool {
        line.match_indices(reg).any(|(at, _)| {
            line.as_bytes().get(at + reg.len()).is_none_or(|b| !b.is_ascii_digit())
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
            "movsd" => matches!(arith_op, "vaddsd" | "vsubsd" | "vmulsd" | "vdivsd"),
            "movss" => matches!(arith_op, "vaddss" | "vsubss" | "vmulss" | "vdivss"),
            _ => false,
        };
        if !legal_op {
            i += 1;
            continue;
        }
        let ops: Vec<&str> = arith_operands.split(',').map(str::trim).collect();
        if ops.len() != 3 || ops[0] != src_reg || ops[1] != ops[2] || ops[1] == src_reg {
            i += 1;
            continue;
        }

        // The source value must have no later use. Stop at `.size`; crossing a
        // label is harmless for this intentionally whole-function proof.
        let mut later_mention = false;
        for k in (i + 2)..len {
            if infos[k].is_nop() {
                continue;
            }
            let t = infos[k].trimmed(store.get(k));
            if t.starts_with(".size ") {
                break;
            }
            if mentions_xmm(t, src_reg) {
                later_mention = true;
                break;
            }
        }
        if later_mention {
            i += 1;
            continue;
        }

        let replacement = format!("    {} {}, {}, {}", arith_op, mem, ops[1], ops[2]);
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
        if infos[i].is_nop() { i += 1; continue; }

        // Step 1: Find a load from stack to %rax (scratch register).
        if let LineKind::LoadRbp { reg: 0, offset, size } = infos[i].kind {
            // Only fold Q and L loads (not sign-extending SLQ, which changes value).
            if size != MoveSize::Q && size != MoveSize::L {
                i += 1; continue;
            }

            // Step 2: Find next non-NOP instruction.
            let mut j = i + 1;
            while j < len && infos[j].is_nop() { j += 1; }
            if j >= len { i += 1; continue; }

            // Step 3: Check if it's "movq %rax, %DEST" or "movl %eax, %DESTd"
            // where DEST is a different GP register.
            let dest_reg = match infos[j].kind {
                LineKind::Other { dest_reg } if dest_reg != REG_NONE && dest_reg != 0 && dest_reg <= REG_GP_MAX => {
                    let line_j = infos[j].trimmed(store.get(j));
                    // Must be a simple register-to-register mov
                    let is_movq_rax = line_j.starts_with("movq %rax, %") && !line_j.contains('(');
                    let is_movl_eax = line_j.starts_with("movl %eax, %") && !line_j.contains('(');
                    if is_movq_rax || is_movl_eax {
                        dest_reg
                    } else {
                        i += 1; continue;
                    }
                }
                _ => { i += 1; continue; }
            };

            // Step 4: Verify no intervening store to the same offset.
            let mut intervening_store = false;
            for k in (i + 1)..j {
                if let LineKind::StoreRbp { offset: so, .. } = infos[k].kind {
                    if so == offset { intervening_store = true; break; }
                }
            }
            if intervening_store { i += 1; continue; }

            // Step 5: Check rax liveness after the copy.
            if !is_rax_dead_after(store, infos, j + 1, len) { i += 1; continue; }

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
        if infos[i].is_nop() { i += 1; continue; }

        // Step 1: Load from stack to %rax.
        if let LineKind::LoadRbp { reg: 0, offset, size: MoveSize::Q } = infos[i].kind {
            // Step 2: Next must be leaq K(%rax), %rax
            let mut j = i + 1;
            while j < len && infos[j].is_nop() { j += 1; }
            if j >= len { i += 1; continue; }

            let leaq_offset = {
                let lj = infos[j].trimmed(store.get(j));
                if !lj.starts_with("leaq ") || !lj.ends_with(", %rax") { i += 1; continue; }
                let inner = &lj[5..lj.len() - 6]; // between "leaq " and ", %rax"
                if !inner.ends_with("(%rax)") { i += 1; continue; }
                let num_str = &inner[..inner.len() - 6]; // before "(%rax)"
                match num_str.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => { i += 1; continue; }
                }
            };

            // Step 3: Next must be movq %rax, %DEST
            let mut k = j + 1;
            while k < len && infos[k].is_nop() { k += 1; }
            if k >= len { i += 1; continue; }

            let dest_reg = match infos[k].kind {
                LineKind::Other { dest_reg } if dest_reg != REG_NONE && dest_reg != 0 && dest_reg <= REG_GP_MAX => {
                    let lk = infos[k].trimmed(store.get(k));
                    if lk.starts_with("movq %rax, %") && !lk.contains('(') {
                        dest_reg
                    } else { i += 1; continue; }
                }
                _ => { i += 1; continue; }
            };

            // Step 4: Check rax is dead after k.
            let rax_dead = is_rax_dead_after(store, infos, k + 1, len);
            if !rax_dead { i += 1; continue; }

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
        if infos[i].is_nop() { i += 1; continue; }

        // Step 1: Load from stack to %rax (either movq or movl).
        if let LineKind::LoadRbp { reg: 0, offset, size } = infos[i].kind {
            if size != MoveSize::Q && size != MoveSize::L { i += 1; continue; }

            // Step 2: Next must be cltq.
            let mut j = i + 1;
            while j < len && infos[j].is_nop() { j += 1; }
            if j >= len { i += 1; continue; }
            {
                let lj = infos[j].trimmed(store.get(j));
                if lj != "cltq" { i += 1; continue; }
            }

            // Step 3: Next must be movq %rax, %DEST.
            let mut k = j + 1;
            while k < len && infos[k].is_nop() { k += 1; }
            if k >= len { i += 1; continue; }

            let dest_reg = match infos[k].kind {
                LineKind::Other { dest_reg } if dest_reg != REG_NONE && dest_reg != 0 && dest_reg <= REG_GP_MAX => {
                    let lk = infos[k].trimmed(store.get(k));
                    if lk.starts_with("movq %rax, %") && !lk.contains('(') {
                        dest_reg
                    } else { i += 1; continue; }
                }
                _ => { i += 1; continue; }
            };

            // Step 4: Check rax is dead after k.
            let rax_dead = is_rax_dead_after(store, infos, k + 1, len);
            if !rax_dead { i += 1; continue; }

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
        if infos[i].is_nop() { i += 1; continue; }

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
                i += 1; continue;
            };

            // Step 2: Next must be movq %rax, %DEST.
            let mut j = i + 1;
            while j < len && infos[j].is_nop() { j += 1; }
            if j >= len { i += 1; continue; }

            let dest_reg = match infos[j].kind {
                LineKind::Other { dest_reg } if dest_reg != REG_NONE && dest_reg != 0 && dest_reg <= REG_GP_MAX => {
                    let lj = infos[j].trimmed(store.get(j));
                    if lj.starts_with("movq %rax, %") && !lj.contains('(') {
                        dest_reg
                    } else { i += 1; continue; }
                }
                _ => { i += 1; continue; }
            };

            // Step 3: Check rax is dead after.
            if !is_rax_dead_after(store, infos, j + 1, len) { i += 1; continue; }

            // Step 4: Transform.
            // Use the same sub-register for the source (al/ax from rax family=0).
            let src_name = REG_NAMES[src_sub_idx][0]; // %al or %ax
            let dest_32 = REG_NAMES[1][dest_reg as usize]; // %r12d etc.
            let new_inst = format!("    {} {}, {}", new_op, src_name, dest_32);

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
        if infos[i].is_nop() { i += 1; continue; }

        // Step 1: Instruction writes to %rax (dest_reg == 0).
        if let LineKind::Other { dest_reg: 0 } = infos[i].kind {
            let line_i = infos[i].trimmed(store.get(i));

            // Step 2: Next must be movq %rax, %DEST.
            let mut j = i + 1;
            while j < len && infos[j].is_nop() { j += 1; }
            if j >= len { i += 1; continue; }
            let dest_reg = match infos[j].kind {
                LineKind::Other { dest_reg } if dest_reg != REG_NONE && dest_reg != 0 && dest_reg <= REG_GP_MAX => {
                    let lj = infos[j].trimmed(store.get(j));
                    if lj.starts_with("movq %rax, %") && !lj.contains('(') {
                        dest_reg
                    } else { i += 1; continue; }
                }
                _ => { i += 1; continue; }
            };

            // Step 3: Check rax is dead after.
            if !is_rax_dead_after(store, infos, j + 1, len) { i += 1; continue; }

            let dest_64 = REG_NAMES[0][dest_reg as usize];
            let dest_32 = REG_NAMES[1][dest_reg as usize];

            // Step 4: Match specific retargetable patterns.
            let new_inst = if line_i.starts_with("leaq ") && line_i.ends_with(", %rax") {
                // leaq X, %rax → leaq X, %REG
                // Safe: leaq doesn't read %rax (it computes an address, doesn't deref).
                // But check the source doesn't reference rax!
                let src = &line_i[5..line_i.len() - 6]; // between "leaq " and ", %rax"
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1; continue;
                }
                Some(format!("    leaq {}, {}", src, dest_64))
            } else if line_i.starts_with("movslq ") && line_i.ends_with(", %rax") {
                // movslq X, %rax → movslq X, %REG
                let src = &line_i[7..line_i.len() - 6];
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1; continue;
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
            } else if line_i.starts_with("movq ") && line_i.ends_with(", %rax") && line_i.contains('(') {
                // movq N(%reg), %rax → movq N(%reg), %REG (pointer dereference)
                // Safe: source is a memory operand, doesn't read %rax as a value.
                // But check the addressing mode doesn't use %rax as base/index!
                let src = &line_i[5..line_i.len() - 6]; // between "movq " and ", %rax"
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1; continue;
                }
                Some(format!("    movq {}, {}", src, dest_64))
            } else if line_i.starts_with("movl ") && line_i.ends_with(", %eax") && line_i.contains('(') {
                // movl N(%reg), %eax → movl N(%reg), %REGd (32-bit pointer dereference)
                let src = &line_i[5..line_i.len() - 6];
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1; continue;
                }
                Some(format!("    movl {}, {}", src, dest_32))
            } else if line_i.starts_with("movzbq ") && line_i.ends_with(", %rax") {
                // movzbq N(%reg), %rax → movzbl N(%reg), %REGd (byte load zero-extend)
                let src = &line_i[7..line_i.len() - 6];
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1; continue;
                }
                Some(format!("    movzbl {}, {}", src, dest_32))
            } else if line_i.starts_with("movzwq ") && line_i.ends_with(", %rax") {
                // movzwq N(%reg), %rax → movzwl N(%reg), %REGd
                let src = &line_i[7..line_i.len() - 6];
                if src.contains("%rax") || src.contains("%eax") {
                    i += 1; continue;
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
        if infos[i].is_nop() { i += 1; continue; }

        // Step 1: movq %REG, %rax (or movl %REGd, %eax)
        let (src_reg, is_32bit) = match infos[i].kind {
            LineKind::Other { dest_reg: 0 } => {
                let line = infos[i].trimmed(store.get(i));
                if line.starts_with("movq %") && line.ends_with(", %rax") && !line.contains('(') {
                    let src = &line[6..line.len() - 6]; // between "movq %" and ", %rax"
                    if !src.contains('%') { // simple register name
                        (src.to_string(), false)
                    } else { i += 1; continue; }
                } else if line.starts_with("movl %") && line.ends_with(", %eax") && !line.contains('(') {
                    let src = &line[6..line.len() - 6];
                    if !src.contains('%') {
                        (src.to_string(), true)
                    } else { i += 1; continue; }
                } else { i += 1; continue; }
            }
            _ => { i += 1; continue; }
        };

        // Step 2: Next must be movq %rax, N(%rsp) or movl %eax, N(%rsp)
        let mut j = i + 1;
        while j < len && infos[j].is_nop() { j += 1; }
        if j >= len { i += 1; continue; }

        let stored = match infos[j].kind {
            LineKind::StoreRbp { reg: 0, offset, size } => {
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
                    i += 1; continue;
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
fn is_reg_dead_after(store: &LineStore, infos: &[LineInfo], start: usize, len: usize, reg: u8) -> bool {
    let scan_limit = (start + 64).min(len);
    let mask = 1u16 << reg;
    let (reg64, reg32, reg8) = reg_names(reg);
    let mut scan = start;
    while scan < scan_limit {
        if infos[scan].is_nop() { scan += 1; continue; }
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
                    if t == format!("xorl {}, {}", reg32, reg32) { return true; }
                    if t.ends_with(&format!(", %{}", reg64)) || t.ends_with(&format!(", {}", reg32)) {
                        // The instruction WRITES the register. It is only dead
                        // (free to retarget) if this write establishes a FRESH
                        // value that does NOT depend on the current value — a
                        // self-move or sign-extension FROM the register still
                        // depends on it, so it is NOT dead.
                        let src = t.split_once(',').map(|(s, _)| {
                            let mut toks = s.splitn(2, char::is_whitespace);
                            let _mnem = toks.next();
                            toks.next().unwrap_or("")
                        }).unwrap_or("");
                        let reads = src.contains(reg32) || src.contains(&format!("%{}", reg64))
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

        if let LineKind::LoadRbp { reg: load_reg, offset, size: load_size } = infos[i].kind {
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

            let is_foldable_target = matches!(infos[j].kind,
                LineKind::Other { .. } | LineKind::Cmp);
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
                    let pat = match load_reg { 0 => ("%rax", "%eax"), 1 => ("%rcx", "%ecx"), _ => ("%rdx", "%edx") };
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
                    let cmp_suffix = if test_l || load_size == MoveSize::L { "cmpl" } else { "cmpq" };
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
            "movzbl", "movzbw", "movzwl", "movslq", "movswq", "movsbq",
            "movsbl", "movswl", "movl", "movq",
        ];
        let mnem = LOADS.iter().find(|m| t.starts_with(*m) && t.as_bytes().get(m.len()) == Some(&b' '))?;
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
            let is_gp = |r: &str| {
                r.starts_with("%r") || r.starts_with("%e")
            };
            if !(is_movq || is_movl) {
                false
            } else if !is_gp(src) || !is_gp(cdest) || cdest.contains('(') {
                false
            } else {
                let src_fam = register_family_fast(src);
                let dest_fam = register_family_fast(cdest);
                if src_fam != load_fam || dest_fam == super::super::types::REG_NONE || src_fam == dest_fam {
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
                        let reads_src = reg_names_family(load_fam)
                            .iter().any(|n| td.contains(n));
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
            let lim = std::env::var("CCC_LCR_LIMIT").ok().and_then(|v| v.parse::<u32>().ok());
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
            eprintln!("[LCR][{}] FUSE #{}: [{}] + [{}] -> dest {}", cur_fn, stats[3], t, tc, cdest);
        }
        let new_load = format!("    {} {}, {}", mnem, mem, cdest);
        replace_line(store, &mut infos[i], i, new_load);
        mark_nop(&mut infos[j]);
        changed = true;
        i = j + 1;
    }
    if dbg && stats[0] > 0 {
        eprintln!("[LCR] loads={} copy-ok={} dead={} fused={} copy-bad={} not-dead={}",
            stats[0], stats[1], stats[2], stats[3], stats[4], stats[5]);
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
    use crate::backend::peephole_common::LineStore;
    use super::fold_load_copy_relay;

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
        assert!(out[0].contains("%rbp"), "load retargeted to copy dest: {out:?}");
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
