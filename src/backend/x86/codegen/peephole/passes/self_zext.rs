use super::super::types::*;
use super::helpers::*;

// ── Redundant self-zext elimination ────────────────────────────────────────

/// A 32-bit write to a GPR zero-extends architecturally; `movl %R, %R` is the
/// emitter's I32→I64 zext placed in-place (same register). When the upper 32
/// bits are already known-zero, that self-move is a no-op and is deleted.
///
/// Straight-line dataflow over the 16 GP families, conservative by design:
///   * 32-bit write  → family known-zero (SET)
///   * 64-bit write  → family unknown (CLEAR)
///   * 8/16-bit write → no effect on the upper 32 bits (no change)
///   * flags-only lines (cmp/test/...) and flag reads → no change
///   * labels / any control transfer → CLEAR ALL (join points)
///   * call → CLEAR caller-saved families only (callee-saved keep the
///     property: the callee must preserve their full 64-bit value)
///   * anything unrecognised → CLEAR ALL (an unknown writer may be 64-bit)
///
/// The unknown→CLEAR-ALL rule is what makes the pass safe against any future
/// instruction: a missed opportunity is cheap, a false claim is a miscompile.
pub(super) fn eliminate_redundant_self_zext(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    // zero_upper[family] = upper 32 bits provably zero.
    let mut zero_upper = [false; 16];
    // Caller-saved GP families: rax,rcx,rdx,rsi,rdi,r8,r9,r10,r11.
    // Callee-saved (rbx,rbp,r12-r15) survive calls with their full value.
    const CALLER_SAVED: [usize; 9] = [0, 1, 2, 6, 7, 8, 9, 10, 11];
    let mut i = 0;
    while i < len {
        let info = infos[i];
        if info.is_nop() || info.kind == LineKind::Empty {
            i += 1;
            continue;
        }
        let t = info.trimmed(store.get(i));
        match info.kind {
            LineKind::Label
            | LineKind::CondJmp
            | LineKind::Jmp
            | LineKind::JmpIndirect
            | LineKind::Ret => {
                zero_upper = [false; 16];
                i += 1;
                continue;
            }
            LineKind::Call => {
                for &f in &CALLER_SAVED {
                    zero_upper[f] = false;
                }
                i += 1;
                continue;
            }
            LineKind::Pop { reg } => {
                if reg <= 15 {
                    zero_upper[reg as usize] = false;
                }
                i += 1;
                continue;
            }
            LineKind::Cmp => {
                // The classifier files every `cmp`-prefixed mnemonic here,
                // including the ones that WRITE registers: `cmpxchg` (%rax,
                // and an explicit register destination when it has one) and
                // the string compares (`cmps*` advance %rsi/%rdi).
                if t.starts_with("cmpxchg") {
                    zero_upper = [false; 16];
                } else {
                    clear_families(&mut zero_upper, implicit_write_refs(t.as_bytes()));
                }
                i += 1;
                continue;
            }
            LineKind::Push { .. }
            | LineKind::SetCC { .. }
            | LineKind::Directive
            | LineKind::StoreRbp { .. }
            | LineKind::StoreXmmRbp { .. } => {
                // no GP-64 write
                i += 1;
                continue;
            }
            LineKind::LoadRbp { reg, size, .. } => {
                if reg <= 15 {
                    match size {
                        MoveSize::Q | MoveSize::SLQ => zero_upper[reg as usize] = false,
                        MoveSize::L => zero_upper[reg as usize] = true,
                        _ => {} // 8/16-bit: upper 32 unchanged
                    }
                }
                i += 1;
                continue;
            }
            LineKind::LoadXmmRbp { .. } => {
                i += 1;
                continue;
            }
            LineKind::SelfMove => {
                // movq %R, %R — 64-bit write (value unchanged, but the write
                // is a full register write: property is preserved for the
                // same value, so clearing is conservative-correct).
                i += 1;
                continue;
            }
            LineKind::InlineAsm => {
                zero_upper = [false; 16];
                i += 1;
                continue;
            }
            LineKind::Other { dest_reg } => {
                // Identify the destination and the write width. Flags-only
                // operations never write a register.
                let mnem = t.split_whitespace().next().unwrap_or("");
                let flags_only = mnem.starts_with("cmp") && !mnem.starts_with("cmpxchg")
                    || mnem.starts_with("test")
                    || mnem.starts_with("ucomis")
                    || mnem.starts_with("vcomis")
                    || mnem.starts_with("comis")
                    || mnem == "bt"
                    || mnem.starts_with("sahf")
                    || mnem.starts_with("lahf") // writes %ah only: upper half unchanged
                    || mnem.starts_with("fcomi");
                if flags_only {
                    i += 1;
                    continue;
                }
                // Self-zext: `movl %R, %R` with the upper bits ALREADY zero
                // (state BEFORE this instruction's own 32-bit write — a
                // deletion is only sound if the zero-extension is redundant)
                // is a no-op.
                let is_redundant_self_zext = if t.starts_with("movl ") && dest_reg <= 15 {
                    if let Some((s1, s2)) = t["movl ".len()..].split_once(',') {
                        let a = register_family_fast(s1.trim());
                        let b = register_family_fast(s2.trim());
                        a == b && a <= 15 && zero_upper[a as usize]
                    } else {
                        false
                    }
                } else {
                    false
                };
                if writes_two_explicit_operands(mnem) {
                    // `xchg`/`xadd` write BOTH operands and `cmpxchg` writes
                    // %rax as well as its destination; the classified
                    // destination names only one of them.
                    zero_upper = [false; 16];
                } else if dest_reg <= 15 {
                    // Second destinations (`mulq` -> %rdx, `rep stos` ->
                    // %rcx/%rdi, ...) first: the implicit write set is exact
                    // for every mnemonic it lists, and clearing is the
                    // conservative answer for each family in it.
                    clear_families(&mut zero_upper, implicit_write_refs(t.as_bytes()));
                    zero_upper[dest_reg as usize] = if mnem == "cdq" || mnem == "cltd" {
                        // The classifier names the primary destination %rax,
                        // the INPUT; the real 32-bit write is %edx.
                        zero_upper[2] = true;
                        zero_upper[dest_reg as usize]
                    } else {
                        // The width is read from the destination OPERAND's
                        // spelling, never from the mnemonic's last letter:
                        // `cmovl`, `shl`, `sal`, `rol`, `rcl` and `imul` end in
                        // `l` without being 32-bit writes.  Only the 32-bit
                        // name proves the zero-extension; the 64-bit name and
                        // everything else (8/16-bit partials, implicit primary
                        // destinations such as `mulq`'s %rax) clear.
                        explicit_dest_token(t) == REG_NAMES[1][dest_reg as usize]
                    };
                } else if writes_memory_only(mnem, t) {
                    // A plain integer ALU/`mov` instruction whose destination
                    // is memory writes no GP register at all.
                } else {
                    // No identifiable GP destination (XMM, opaque or implicit
                    // writers): the write set is unknown — an unknown writer
                    // may be 64-bit -> clear ALL.
                    zero_upper = [false; 16];
                }
                if is_redundant_self_zext {
                    mark_nop(&mut infos[i]);
                    changed = true;
                }
                i += 1;
            }
            _ => {
                zero_upper = [false; 16];
                i += 1;
            }
        }
    }
    changed
}

/// The explicit AT&T destination: the operand after the last comma, or the
/// only operand of a one-operand instruction.
fn explicit_dest_token(t: &str) -> &str {
    match t.rsplit_once(',') {
        Some((_, last)) => last.trim(),
        None => t
            .split_once(char::is_whitespace)
            .map_or("", |(_, op)| op.trim()),
    }
}

/// Instructions that write a register the classified destination does not
/// name: both operands of `xchg`/`xadd`, and `cmpxchg`'s implicit %rax.
fn writes_two_explicit_operands(mnem: &str) -> bool {
    mnem.starts_with("xchg") || mnem.starts_with("xadd") || mnem.starts_with("cmpxchg")
}

fn clear_families(zero_upper: &mut [bool; 16], mask: u16) {
    for (fam, known) in zero_upper.iter_mut().enumerate() {
        if mask & (1 << fam) != 0 {
            *known = false;
        }
    }
}

/// A positive list, not a negative one: integer ALU and `mov` mnemonics (with
/// or without a `b/w/l/q` suffix) whose only architectural outputs, when the
/// destination is memory, are that memory and the flags.  Prefixed lines
/// (`lock ...`), string instructions, multiply/divide and every other
/// mnemonic stay on the clear-all path.
fn writes_memory_only(mnem: &str, t: &str) -> bool {
    const PLAIN: [&str; 22] = [
        "mov", "add", "sub", "and", "or", "xor", "adc", "sbb", "inc", "dec", "neg", "not", "shl",
        "sal", "shr", "sar", "rol", "ror", "rcl", "rcr", "bts", "btr",
    ];
    let base = match mnem.as_bytes().last() {
        Some(b'b' | b'w' | b'l' | b'q') if !PLAIN.contains(&mnem) => &mnem[..mnem.len() - 1],
        _ => mnem,
    };
    PLAIN.contains(&base) && explicit_dest_token(t).ends_with(')')
}

#[cfg(test)]
mod tests {
    use super::super::super::types::classify_line;
    use super::eliminate_redundant_self_zext;
    use crate::backend::peephole_common::LineStore;

    /// Runs the pass; returns how many `movl %R, %R` lines survive.
    fn survivors(asm: &str, self_zext: &str) -> usize {
        let store = LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        eliminate_redundant_self_zext(&store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop() && store.get(i).trim() == self_zext)
            .count()
    }

    /// A memory-destination ALU/`mov` writes no register: the index's
    /// zero-extension established before `C[i]++; D[i]++` still holds for the
    /// second table.
    #[test]
    fn memory_destination_keeps_the_fact() {
        for between in [
            "addl $1, (%rdi, %rdx, 4)",
            "movl %eax, (%rdi, %rdx, 4)",
            "incq (%rsi)",
            "subb $3, (%rsi)",
        ] {
            let asm = format!("    movl %edx, %edx\n    {between}\n    movl %edx, %edx\n");
            assert_eq!(survivors(&asm, "movl %edx, %edx"), 1, "after `{between}`");
        }
    }

    /// The fact is established by a 32-bit DESTINATION operand, not by a
    /// mnemonic that happens to end in `l`.
    #[test]
    fn a_32_bit_write_establishes_the_fact() {
        let asm = "    movl (%rdi), %eax\n    movl %eax, %eax\n";
        assert_eq!(survivors(asm, "movl %eax, %eax"), 0);
    }

    /// Each line below leaves %rax (or %rdx) with possibly non-zero upper bits
    /// after a 32-bit write had established the fact; the following self-zext
    /// is therefore NOT redundant.  Every case was mis-handled by the
    /// mnemonic-suffix width rule or the partial flags-only list.
    #[test]
    fn writes_that_clobber_the_upper_half_kill_the_fact() {
        let cases = [
            // suffix-less mnemonics ending in `l`, 64-bit destination
            ("movl %esi, %eax", "cmovl %rcx, %rax", "movl %eax, %eax"),
            ("movl %esi, %eax", "shl %cl, %rax", "movl %eax, %eax"),
            ("movl %esi, %eax", "imul %rcx, %rax", "movl %eax, %eax"),
            ("movl %esi, %eax", "rol $3, %rax", "movl %eax, %eax"),
            // a second, implicit destination
            ("movl %esi, %edx", "mulq %rcx", "movl %edx, %edx"),
            ("movl %esi, %edx", "divq %rcx", "movl %edx, %edx"),
            // both operands written; the classified destination names one
            ("movl %esi, %eax", "xchgq %rax, %rcx", "movl %eax, %eax"),
            ("movl %esi, %eax", "xaddq %rax, (%rdi)", "movl %eax, %eax"),
            (
                "movl %esi, %eax",
                "lock xaddq %rax, (%rdi)",
                "movl %eax, %eax",
            ),
            // `cmp` prefix, but %rax is written
            (
                "movl %esi, %eax",
                "cmpxchgq %rcx, (%rdi)",
                "movl %eax, %eax",
            ),
            // 64-bit destination of a suffixed mnemonic
            ("movl %esi, %eax", "addq %rcx, %rax", "movl %eax, %eax"),
            // partial write: provably nothing, conservatively unknown
            ("movl %esi, %eax", "movw %cx, %ax", "movl %eax, %eax"),
        ];
        for (def, between, zext) in cases {
            let asm = format!("    {def}\n    {between}\n    {zext}\n");
            assert_eq!(survivors(&asm, zext), 1, "`{between}` must kill the fact");
        }
    }

    /// Unlisted memory-destination instructions stay on the clear-all path.
    #[test]
    fn unlisted_memory_destination_clears_all() {
        let asm = "    movl %edx, %edx\n    vmovdqu %ymm0, (%rdi)\n    movl %edx, %edx\n";
        assert_eq!(survivors(asm, "movl %edx, %edx"), 2);
    }
}
