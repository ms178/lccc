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
pub(super) fn eliminate_redundant_self_zext(store: &LineStore, infos: &mut [LineInfo]) {
    let len = store.len();
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
            LineKind::Label | LineKind::CondJmp | LineKind::Jmp | LineKind::JmpIndirect
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
            LineKind::Push { .. } | LineKind::SetCC { .. } | LineKind::Directive
            | LineKind::Cmp | LineKind::StoreRbp { .. } | LineKind::StoreXmmRbp { .. } => {
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
                // Identify the destination and the write width from the
                // mnemonic. Flags-only operations never write a register.
                let mnem = t.split_whitespace().next().unwrap_or("");
                let flags_only = mnem.starts_with("cmp")
                    || mnem.starts_with("test")
                    || mnem.starts_with("ucomis")
                    || mnem.starts_with("vcomis")
                    || mnem.starts_with("comis")
                    || mnem == "bt"
                    || mnem.starts_with("sahf")
                    || mnem.starts_with("lahf")
                    || mnem.starts_with("fcomi")
                    || mnem == "xchg";
                if flags_only {
                    if mnem == "xchg" {
                        // Full-width write to BOTH operands.
                        zero_upper = [false; 16];
                    }
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
                if dest_reg <= 15 {
                    if mnem.ends_with('l') {
                        // 32-bit write: zero-extends.
                        zero_upper[dest_reg as usize] = true;
                    } else if mnem.ends_with('q') {
                        // 64-bit write.
                        zero_upper[dest_reg as usize] = false;
                    } else if mnem == "cdq" || mnem == "cltd" {
                        zero_upper[2] = true; // 32-bit edx write
                    } else if mnem == "cqto" || mnem == "cqo" {
                        zero_upper[2] = false; // 64-bit rdx write
                    } else {
                        // Unknown width: this register is no longer provable.
                        zero_upper[dest_reg as usize] = false;
                    }
                } else {
                    // No identifiable GP destination (memory dest, XMM,
                    // implicit writers): the write set is unknown — an
                    // unknown writer may be 64-bit -> clear ALL.
                    zero_upper = [false; 16];
                }
                if is_redundant_self_zext {
                    mark_nop(&mut infos[i]);
                }
                i += 1;
            }
            _ => {
                zero_upper = [false; 16];
                i += 1;
            }
        }
    }
}
