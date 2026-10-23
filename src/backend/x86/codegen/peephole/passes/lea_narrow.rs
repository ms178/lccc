//! Narrow a `leaq` whose result is truncated into a 32-bit `leal`.
//!
//! The emitter materialises an address in a 64-bit register and then truncates
//! it where the C expression asks for the low 32 bits:
//!
//! ```text
//!     leaq   8(%rdi), %rdx        # a = (uint32_t)(i + 8)
//!     ...
//!     movl   %edx, %r13d
//! ```
//!
//! `leal 8(%rdi), %r13d` computes the same low 32 bits in one instruction and
//! zero-extends into the destination exactly as the `movl` did, so the pair
//! costs one instruction too many.  The shape is what every
//! `(uint32_t)(p + k)` / `(unsigned)(i + k)` cast of an address-scale value
//! compiles to, and it repeats once per unrolled iteration in pointer-hash and
//! record-decoding loops.
//!
//! Measured on `tests/benchmark/programs/byte_load_reassembly.c` — the corpus's
//! worst ratio against GCC at `-O2` — the 32-byte decode step carries two of
//! these pairs, so the loop drops from 18 to 16 instructions and the function
//! from 163 to 161.  Across the 49 benchmark programs the fold fires on exactly
//! two (`byte_load_reassembly`, `glibc_strstr` 173 -> 172).
//!
//! Paired on the repository runner (`--compilers lccc --candidate base=...`,
//! 9 reps, 2 warm-ups) that is `byte_load_reassembly` 5.515 ms -> 5.339 ms
//! (-3.2%); the arms that cannot change shape bound the noise floor of the
//! measurement host at +-1..2%, so the shrink is directional evidence, not a
//! corpus-level speed claim.
//!
//! # Correctness
//!
//! The 32-bit `lea` is exact: x86 address arithmetic wraps modulo 2^64, and the
//! low 32 bits of a sum depend only on the low 32 bits of its inputs, so the
//! truncated result is identical.  `lea` writes no flags either way.
//!
//! Deleting the `movl` moves the destination's write *earlier* (to the `leaq`)
//! and widens the source's use to 32 bits, so:
//!
//! 1. The `movl` is found inside a short forward window from the `leaq`.
//!    A label, branch, call, return, opaque directive or inline-asm line ends
//!    the window — a jump target between the pair (or into the `movl`) would
//!    make deleting the second line change what a branch executes.
//! 2. No line inside the window mentions the `leaq` destination before the
//!    truncating move: the first mention *is* the move, which is also what
//!    proves the 64-bit value is not read in between.
//! 3. No line inside the window mentions the `movl` destination, textually or
//!    implicitly.  Otherwise that line observed the pre-`movl` value (a read)
//!    or would clobber the value moved up (a write).
//! 4. Neither line is pinned ([`all_editable`]): a parameter pre-store or an
//!    inline-asm argument is not ours to rewrite.
//! 5. The 64-bit destination is dead after the `movl` unless it *is* the
//!    `movl` destination.  `FileLiveness` answers this over the whole function
//!    (including implicit reads at calls and returns); an unknown answer
//!    (`None`) refuses, because a wrong "dead" claim resurrects a stale value.
//! 6. The destination is neither `%rsp` nor `%rbp`: those are the frame.
//! 7. The address is pure arithmetic: no `%rip`, no segment prefix (`%fs:`), no
//!    relocation suffix (`@TPOFF`) and no symbol.  Narrowing the destination of
//!    a relocation-carrying `lea` changes the relocation the assembler emits
//!    (`R_X86_64_PC32`/`R_X86_64_TPOFF64` on a 64-bit destination versus their
//!    32-bit forms), and the relocation shape is the linker's contract, not a
//!    peephole's.  See [`plain_address`].

use super::super::types::*;
use super::helpers::*;
use super::liveness::FileLiveness;
use super::relay_and_lea::{plain_gp_operand, split_two_operands};

/// How far after the `leaq` the truncating `movl` may sit.  The emitter keeps
/// the cast adjacent, but intervening independent work interleaves it:
/// `byte_load_reassembly`'s decode step puts five emitted lines between the
/// two, so a narrower window silently disables the fold on the very loop it
/// was written for.  Every line the scan crosses is checked (rules 2 and 3),
/// so the width trades compile time, not soundness.
const WINDOW: usize = 8;

/// True when a line inside the window neither transfers control nor hides
/// register traffic (`call`/`syscall` read and write caller-saved registers
/// without naming them).
fn window_opaque(info: LineInfo, line: &str) -> bool {
    info.pinned
        || matches!(
            info.kind,
            LineKind::Label
                | LineKind::Jmp
                | LineKind::JmpIndirect
                | LineKind::CondJmp
                | LineKind::Call
                | LineKind::Ret
                | LineKind::InlineAsm
        )
        || (info.kind == LineKind::Directive && !info.inert_dir)
        || line.contains("syscall")
}

/// True when the address text is registers, displacements and scale only.
///
/// A symbol, a relocation suffix (`sym@TPOFF`) or a segment override (`%fs:`) is
/// rejected: the assembler picks the relocation kind from the `lea`'s operand
/// size, so narrowing the destination silently changes the object file's
/// contract with the linker.
fn plain_address(addr: &str) -> bool {
    if addr.contains('@') || addr.contains("%rip") || addr.contains("%fs") || addr.contains("%gs") {
        return false;
    }
    let mut rest = addr;
    while let Some(pct) = rest.find('%') {
        if pct != 0 {
            // Everything between registers must be displacement punctuation.
            let between = &rest[..pct];
            if !between.chars().all(|c| {
                c.is_ascii_digit()
                    || matches!(c, 'x' | '+' | '-' | '(' | ')' | ',' | '*' | ' ' | '\t')
            }) {
                return false;
            }
        }
        let after = &rest[pct + 1..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric()))
            .unwrap_or(after.len());
        if end == 0 {
            return false;
        }
        rest = &after[end..];
    }
    rest.chars().all(|c| {
        c.is_ascii_digit() || matches!(c, 'x' | '+' | '-' | '(' | ')' | ',' | '*' | ' ' | '\t')
    })
}

/// `movl %<t>d, %<d>d` → `d`, with both operands plain 32-bit GPR spellings.
fn truncating_move(line: &str, t_fam: RegId) -> Option<RegId> {
    let rest = line.strip_prefix("movl ")?;
    let (src, dst) = split_two_operands(rest)?;
    if src != REG_NAMES[1][t_fam as usize] {
        return None;
    }
    let d = plain_gp_operand(dst)?;
    (dst == REG_NAMES[1][d as usize]).then_some(d)
}

/// `leaq ADDR, %T` + (window) + `movl %Td, %Dd` → `leal ADDR, %Dd`.
pub(super) fn narrow_lea_truncations(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i + 1 < store.len() {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let lea = infos[i].trimmed(store.get(i));
        let Some(rest) = lea.strip_prefix("leaq ") else {
            i += 1;
            continue;
        };
        // Rules 6, 7: frame destinations and relocation-free addresses.
        let Some((addr_text, lea_dst)) = split_two_operands(rest) else {
            i += 1;
            continue;
        };
        let Some(t_fam) = plain_gp_operand(lea_dst) else {
            i += 1;
            continue;
        };
        if lea_dst != REG_NAMES[0][t_fam as usize] || !plain_address(addr_text) {
            i += 1;
            continue;
        }
        if t_fam == 4 || t_fam == 5 {
            i += 1;
            continue;
        }
        // Rule 1/2: first mention of the destination inside the window must be
        // the truncating move itself.
        let t_mask = 1u16 << t_fam;
        let mut cand = None;
        let mut seen = 0;
        for j in i + 1..store.len() {
            if infos[j].is_nop() {
                continue;
            }
            // Count *emitted* lines, not indices: deleted lines cost nothing.
            seen += 1;
            if seen > WINDOW {
                break;
            }
            let t = infos[j].trimmed(store.get(j));
            if window_opaque(infos[j], t) {
                break;
            }
            if infos[j].reg_refs & t_mask != 0 || implicit_reg_refs(t.as_bytes()) & t_mask != 0 {
                if let Some(d) = truncating_move(t, t_fam) {
                    cand = Some((j, d));
                }
                break;
            }
        }
        let Some((j, d_fam)) = cand else {
            i += 1;
            continue;
        };
        if d_fam == 4 || d_fam == 5 {
            i += 1;
            continue;
        }
        // Rule 3: the moved-up write must not be observed or clobbered first.
        let d_mask = 1u16 << d_fam;
        let mut blocked = false;
        for k in i + 1..j {
            if infos[k].is_nop() {
                continue;
            }
            let t = infos[k].trimmed(store.get(k));
            if window_opaque(infos[k], t)
                || infos[k].reg_refs & d_mask != 0
                || implicit_reg_refs(t.as_bytes()) & d_mask != 0
            {
                blocked = true;
                break;
            }
        }
        if blocked {
            i += 1;
            continue;
        }
        // Rule 5: the 64-bit result must be dead once the 32-bit one exists.
        if t_fam != d_fam && lv.live_after(j, t_fam) != Some(false) {
            i += 1;
            continue;
        }
        // Rule 4: commit gate for the pair, before the first edit.
        if !all_editable(infos, &[i, j]) {
            i += 1;
            continue;
        }
        let new_line = format!("    leal {}, {}", addr_text, REG_NAMES[1][d_fam as usize]);
        replace_line(store, &mut infos[i], i, new_line);
        mark_nop(&mut infos[j]);
        lv.refresh_at(store, infos, i);
        changed = true;
        i = j + 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the pass over one function body.
    ///
    /// The `.cfi_startproc`/`.cfi_endproc` pair is not decoration: `FileLiveness`
    /// only solves functions it can delimit (see its `new`), so a snippet
    /// without them has no liveness answer at all and every query would be
    /// `None` -- which is exactly the refusal path, not the fold.
    fn run(body: &str) -> Vec<String> {
        let asm = format!(".text\nf:\n.cfi_startproc\n{body}\n.cfi_endproc\n");
        let mut store = LineStore::new(asm);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        narrow_lea_truncations(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).to_string())
            .collect()
    }

    fn joined(body: &str) -> String {
        run(body).join("\n")
    }

    #[test]
    fn truncation_of_a_fresh_lea_narrows() {
        let out = joined("    leaq 8(%rdi), %rdx\n    movl %edx, %r13d\n    ret");
        assert!(out.contains("leal 8(%rdi), %r13d"), "{out}");
        assert!(!out.contains("movl %edx"), "{out}");
    }

    #[test]
    fn self_truncation_narrows_in_place() {
        // The unrolled `(uint32_t)(i + 16)` case: the allocator put the lea and
        // the truncation in the same register, which is a no-op movl.
        let out = joined(
            "    leaq 16(%rdi), %r9\n    movl %r9d, %r9d\n    xorl (%r8,%rdi), %r9d\n    ret",
        );
        assert!(out.contains("leal 16(%rdi), %r9d"), "{out}");
        assert!(!out.contains("movl %r9d, %r9d"), "{out}");
        assert!(out.contains("xorl (%r8,%rdi), %r9d"), "{out}");
    }

    #[test]
    fn independent_work_in_the_window_does_not_block_the_fold() {
        // `byte_load_reassembly`'s decode step: unrelated instructions sit
        // between the `leaq` and the truncation.
        let out = joined(
            "    leaq 8(%rdi), %rdx\n    movl (%r8,%rdi), %r15d\n    xorl %edi, %r15d\n    \
             movl %edx, %r13d\n    xorl 8(%r8,%rdi), %r13d\n    ret",
        );
        assert!(out.contains("leal 8(%rdi), %r13d"), "{out}");
        assert!(!out.contains("movl %edx"), "{out}");
        assert!(out.contains("xorl 8(%r8,%rdi), %r13d"), "{out}");
    }

    #[test]
    fn a_live_wide_result_is_preserved() {
        // %rdx is read after the truncation, so the 64-bit value is still
        // needed and the pair must stay exactly as emitted.
        let out =
            joined("    leaq 8(%rdi), %rdx\n    movl %edx, %r13d\n    addq %rdx, %rax\n    ret");
        assert!(out.contains("leaq 8(%rdi), %rdx"), "{out}");
        assert!(out.contains("movl %edx, %r13d"), "{out}");
    }

    #[test]
    fn a_label_between_the_pair_blocks_the_fold() {
        // Deleting a jump target's instruction would change what the branch
        // executes.
        let out = joined("    leaq 8(%rdi), %rdx\n.L3:\n    movl %edx, %r13d\n    ret");
        assert!(out.contains("movl %edx, %r13d"), "{out}");
        assert!(!out.contains("leal 8(%rdi), %r13d"), "{out}");
    }

    #[test]
    fn a_call_in_the_window_blocks_the_fold() {
        let out = joined("    leaq 8(%rdi), %rdx\n    call g\n    movl %edx, %r13d\n    ret");
        assert!(out.contains("movl %edx, %r13d"), "{out}");
    }

    #[test]
    fn an_earlier_read_of_the_destination_blocks_the_fold() {
        // %r13 is read before the `movl` moves its definition up.
        let out =
            joined("    leaq 8(%rdi), %rdx\n    addq %r13, %rax\n    movl %edx, %r13d\n    ret");
        assert!(out.contains("movl %edx, %r13d"), "{out}");
    }

    #[test]
    fn a_second_use_of_the_wide_value_blocks_the_fold() {
        // The first mention of %rdx is a use, not the truncating move.
        let out =
            joined("    leaq 8(%rdi), %rdx\n    addq %rdx, %rax\n    movl %edx, %r13d\n    ret");
        assert!(out.contains("leaq 8(%rdi), %rdx"), "{out}");
        assert!(out.contains("movl %edx, %r13d"), "{out}");
    }

    #[test]
    fn relocation_carrying_addresses_keep_their_shape() {
        for addr in [
            "buf(%rip)",
            "tls_slots@TPOFF(%rax)",
            "table(%rax)",
            "%fs:0(%rax)",
        ] {
            let body = format!("    leaq {addr}, %rdx\n    movl %edx, %r13d\n    ret");
            let out = joined(&body);
            assert!(out.contains(&format!("leaq {addr}, %rdx")), "{addr}: {out}");
        }
        assert!(!plain_address("buf(%rip)"));
        assert!(!plain_address("tls_slots@TPOFF(%rax)"));
        assert!(!plain_address("table(%rax)"));
        assert!(!plain_address("%fs:0(%rax)"));
    }

    #[test]
    fn frame_registers_are_never_narrowed() {
        let out = joined("    leaq 8(%rdi), %rdx\n    movl %edx, %ebp\n    ret");
        assert!(out.contains("movl %edx, %ebp"), "{out}");
    }

    #[test]
    fn a_64_bit_move_is_not_this_pattern() {
        let out = joined("    leaq 8(%rdi), %rdx\n    movq %rdx, %r13\n    ret");
        assert!(out.contains("leaq 8(%rdi), %rdx"), "{out}");
    }

    #[test]
    fn plain_addresses_are_accepted() {
        for addr in [
            "8(%rdi)",
            "(%r15,%r9)",
            "16(%rax,%rbx,4)",
            "0x10(%rcx)",
            "-8(%rdx)",
        ] {
            assert!(plain_address(addr), "{addr}");
            // The address parser alone is not the fold: every accepted address
            // must also survive the pass's own refusal list.
            let body = format!("    leaq {addr}, %rdx\n    movl %edx, %r13d\n    ret");
            let out = joined(&body);
            assert!(
                out.contains(&format!("leal {addr}, %r13d")),
                "{addr}: {out}"
            );
        }
    }
}
