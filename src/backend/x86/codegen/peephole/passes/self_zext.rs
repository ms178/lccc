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
///   * labels / unconditional control transfer → CLEAR ALL (join points)
///   * conditional jump → preserve facts on the fallthrough; its target label
///     clears them independently before any joined path is processed
///   * call → CLEAR caller-saved families only (callee-saved keep the
///     property: the callee must preserve their full 64-bit value)
///   * anything unrecognised → CLEAR ALL (an unknown writer may be 64-bit)
///
/// The unknown→CLEAR-ALL rule is what makes the pass safe against any future
/// instruction: a missed opportunity is cheap, a false claim is a miscompile.
pub(super) fn eliminate_redundant_self_zext(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let preserve_fallthrough = std::env::var_os("CCC_NO_SELF_ZEXT_FALLTHROUGH").is_none();
    eliminate_redundant_self_zext_with_policy(store, infos, preserve_fallthrough)
}

fn eliminate_redundant_self_zext_with_policy(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    preserve_fallthrough: bool,
) -> bool {
    let len = store.len();
    let mut changed = false;
    // (self-zext line, destination family) deferred to the rewrite phase.
    let mut self_zext_candidates: Vec<(usize, u8)> = Vec::new();
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
            LineKind::Label | LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret => {
                zero_upper = [false; 16];
                i += 1;
                continue;
            }
            LineKind::CondJmp => {
                // A conditional branch writes no GPR. Only its fallthrough is
                // scanned here; any target (including a backward edge) starts
                // at a label and clears facts before the joined block.
                if !preserve_fallthrough {
                    zero_upper = [false; 16];
                }
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
            LineKind::Directive if !info.inert_dir => {
                // Data/unknown directives can encode instructions or hide an
                // entry point; only classified inert metadata is transparent.
                zero_upper = [false; 16];
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
                if let Some(families) = implicit_32_bit_write(mnem, t) {
                    // Widening multiply/divide: %eax and %edx each receive a
                    // 32-bit result and nothing else is written, so both upper
                    // halves are zero and the self-zext the emitter adds to
                    // widen such a result is a no-op.  Decided from the
                    // mnemonic and the operand SHAPE: the classifier reports
                    // %rax for the `div`/`mul` spellings but REG_NONE for the
                    // single-operand `imull`, and the explicit-destination
                    // width test below would read the divisor `%edi` as a
                    // destination, mismatch `%eax` and clear the fact this arm
                    // just established.
                    clear_families(&mut zero_upper, implicit_write_refs(t.as_bytes()));
                    for fam in families {
                        zero_upper[fam] = true;
                    }
                } else if writes_two_explicit_operands(mnem) {
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
                    i += 1;
                    continue;
                }
                // A self-zext that survives the redundancy test may still
                // have a one-instruction form: the note is recorded here and
                // rewritten after the scan, once no line borrow is outstanding
                // (the `t`/`mnem` slices above borrow the store).
                if t.starts_with("movl ") && dest_reg <= 15 {
                    self_zext_candidates.push((i, dest_reg));
                }
                i += 1;
            }
            _ => {
                zero_upper = [false; 16];
                i += 1;
            }
        }
    }
    // Rewrite phase: no line borrow is live here, so the store can be edited.
    // A `movd` establishes the same fact a 32-bit destination write does, so
    // the next fixpoint iteration of this pass sees it and continues from
    // there; nothing here depends on `zero_upper` being updated in place.
    for (i, dest_reg) in self_zext_candidates {
        if infos[i].is_nop() {
            continue;
        }
        if let Some(replacement) = xmm_bitcast_self_zext(store, infos, i, dest_reg) {
            replace_line(store, &mut infos[i - 1], i - 1, replacement);
            mark_nop(&mut infos[i]);
            changed = true;
        }
    }

    changed
}

/// The `movd`/`vmovd` that replaces `movq %xmmN, %rX` when the line at `i` is
/// the self-zext `movl %eX, %eX` of that same `%rX`.
///
/// Returns `None` unless the previous *live* line is exactly that `movq`, the
/// two lines are adjacent, both are editable, and the destination families
/// agree. The VEX spelling is preserved: rewriting a `vmovq` to a legacy
/// `movd` inside VEX-encoded code would pay the SSE/AVX transition penalty the
/// emitter avoided, and `vmovd` does the same zero-extending GPR write.
fn xmm_bitcast_self_zext(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    dest_reg: u8,
) -> Option<String> {
    if i == 0 || infos[i].pinned || infos[i - 1].pinned {
        return None;
    }
    let prev = infos[i - 1];
    if prev.is_nop() || prev.kind == LineKind::Empty {
        return None;
    }
    let text = prev.trimmed(store.get(i - 1));
    // `vmovq` is written by the AVX path; `movd`/`vmovd` already copy only the
    // low half, so there is nothing to fuse.
    let (mnem, rest, fused) = if let Some(r) = text.strip_prefix("vmovq ") {
        ("vmovq", r, "vmovd")
    } else if let Some(r) = text.strip_prefix("movq ") {
        ("movq", r, "movd")
    } else {
        return None;
    };
    let (src, dst) = rest.split_once(',')?;
    let (src, dst) = (src.trim(), dst.trim());
    if !src.starts_with("%xmm") || !dst.starts_with("%r") {
        return None;
    }
    // The GPR must be the one the self-zext widens, and the self-zext must
    // name the same family on both sides (otherwise it is a real move).
    if register_family_fast(dst) != dest_reg {
        return None;
    }
    let self_zext = infos[i].trimmed(store.get(i));
    let (a, b) = self_zext["movl ".len()..].split_once(',')?;
    if register_family_fast(a.trim()) != register_family_fast(b.trim()) {
        return None;
    }
    let _ = mnem;
    Some(format!(
        "    {fused} {src}, {}",
        REG_NAMES[1][dest_reg as usize]
    ))
}

/// Register families whose implicit write by `mnem` is exactly 32 bits wide,
/// so the write leaves the family's upper 32 bits zero.
///
/// Restricted to the spellings the emitter actually produces, and only to the
/// `l` (32-bit) forms: `divq %r` writes a 64-bit %rax, so the extension after
/// it is real, and the suffix-less `div`/`mul` aliases are ambiguous without
/// the operand.  `cltq`/`cdqe` are excluded on purpose -- they sign-extend into
/// a full 64-bit %rax, the exact opposite of this property.
///
/// `imull` is the one mnemonic here that also has explicit-destination forms;
/// only the single-operand (widening) form writes %rax:%rdx implicitly.
fn implicit_32_bit_write(mnem: &str, t: &str) -> Option<[usize; 2]> {
    const RAX: usize = 0;
    const RDX: usize = 2;
    match mnem {
        // Widening multiply and divide: %edx:%eax both receive 32 bits.
        "mull" | "divl" | "idivl" => Some([RAX, RDX]),
        "imull" if !t.contains(',') => Some([RAX, RDX]),
        _ => None,
    }
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
    use super::{eliminate_redundant_self_zext, eliminate_redundant_self_zext_with_policy};
    use crate::backend::peephole_common::LineStore;

    /// Runs the pass; returns how many `movl %R, %R` lines survive.
    fn survivors(asm: &str, self_zext: &str) -> usize {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        eliminate_redundant_self_zext(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop() && store.get(i).trim() == self_zext)
            .count()
    }

    fn survivors_with_policy(asm: &str, self_zext: &str, preserve_fallthrough: bool) -> usize {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        eliminate_redundant_self_zext_with_policy(&mut store, &mut infos, preserve_fallthrough);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop() && store.get(i).trim() == self_zext)
            .count()
    }

    #[test]
    fn zero_extension_survives_a_conditional_fallthrough_but_not_a_join() {
        // A conditional store in a hash-insertion loop: the branch does not
        // change the index, and its fallthrough has no other predecessor.
        let loop_arm = "    shrl $17, %edi\n    movzwl (%rsi,%rdi,2), %r12d\n    cmpl %r14d, %r12d\n    je .Lnext\n    movzwl %r10w, %r11d\n    andl $16383, %r11d\n    movw %r12w, (%rdx,%r11,2)\n    movl %edi, %edi\n    movw %r10w, (%rsi,%rdi,2)\n.Lnext:\n";
        assert_eq!(survivors_with_policy(loop_arm, "movl %edi, %edi", true), 0);
        assert_eq!(survivors_with_policy(loop_arm, "movl %edi, %edi", false), 1);

        // The taken edge and other predecessors may carry different upper
        // bits. Labels, unconditional jumps, and real 64-bit writes remain
        // barriers even when fallthrough propagation is enabled.
        for asm in [
            "    movl %esi, %edi\n    je .Ljoin\n.Ljoin:\n    movl %edi, %edi\n",
            "    movl %esi, %edi\n    je .Lother\n    movq %rax, %rdi\n    movl %edi, %edi\n.Lother:\n",
            "    movl %esi, %edi\n    jmp .Lother\n    movl %edi, %edi\n.Lother:\n",
            "    movl %esi, %edi\n    je .Lother\n    .byte 0x90\n    movl %edi, %edi\n.Lother:\n",
        ] {
            assert_eq!(survivors_with_policy(asm, "movl %edi, %edi", true), 1);
        }
    }

    #[test]
    fn unknown_directive_clears_zero_extension_facts() {
        let asm = "    movl %esi, %edi\n    .byte 0x90\n    movl %edi, %edi\n";
        assert_eq!(survivors(asm, "movl %edi, %edi"), 1);
    }

    /// A 32-bit implicit write establishes the fact. `divl`/`mull`/`idivl` and
    /// the single-operand `imull` leave %eax and %edx each written 32 bits
    /// wide, so the self-zext the emitter adds to widen such a result for a
    /// 64-bit consumer is a no-op.
    #[test]
    fn implicit_32_bit_writes_establish_the_fact() {
        for (writer, self_zext) in [
            ("divl %edi", "movl %eax, %eax"),
            ("divl %edi", "movl %edx, %edx"),
            ("idivl %edi", "movl %eax, %eax"),
            ("idivl %edi", "movl %edx, %edx"),
            ("mull %edi", "movl %eax, %eax"),
            ("mull %edi", "movl %edx, %edx"),
            ("imull %edi", "movl %eax, %eax"),
            ("imull %edi", "movl %edx, %edx"),
        ] {
            let asm = format!("    {writer}\n    {self_zext}\n");
            assert_eq!(survivors(&asm, self_zext), 0, "after `{writer}`");
        }
    }

    /// The forms that must KEEP the extension.  A 64-bit `%rax` write really
    /// does need it, `cltq` sign-extends into a full 64-bit `%rax`, and the
    /// explicit-destination `imull` establishes only its destination.
    #[test]
    fn implicit_64_bit_writes_and_explicit_forms_keep_the_extension() {
        for (seed, writer, self_zext) in [
            ("movl %esi, %eax", "divq %rdi", "movl %eax, %eax"),
            ("movl %esi, %edx", "divq %rdi", "movl %edx, %edx"),
            ("movl %esi, %eax", "mulq %rdi", "movl %eax, %eax"),
            ("movl %esi, %edx", "mulq %rdi", "movl %edx, %edx"),
            // sign-extends: the self-zext is load-bearing
            ("movl %esi, %eax", "cltq", "movl %eax, %eax"),
        ] {
            let asm = format!("    {seed}\n    {writer}\n    {self_zext}\n");
            assert_eq!(survivors(&asm, self_zext), 1, "after `{writer}`");
        }
        // `imull %edi, %eax` writes %eax 32 bits wide -- the fact holds for
        // %eax -- and says nothing at all about %edx.
        assert_eq!(
            survivors(
                "    imull %edi, %eax\n    movl %eax, %eax\n",
                "movl %eax, %eax"
            ),
            0
        );
        assert_eq!(
            survivors(
                "    imull %edi, %eax\n    movl %edx, %edx\n",
                "movl %edx, %edx"
            ),
            1
        );
    }

    /// `movq %xmmN, %rX` + `movl %eX, %eX` is the float-bit-cast widening
    /// pair; `movd %xmmN, %eX` is the same architectural state in one
    /// instruction. Returns the surviving text of both lines.
    fn fuse(asm: &str) -> Vec<String> {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        eliminate_redundant_self_zext(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop() && !store.get(i).trim().is_empty())
            .map(|i| store.get(i).trim().to_string())
            .filter(|l| !l.starts_with('.'))
            .collect()
    }

    #[test]
    fn xmm_bitcast_self_zext_fuses_to_movd() {
        assert_eq!(
            fuse(
                "    movss (%rbx), %xmm9\n    movq %xmm9, %rax\n    movl %eax, %eax\n    xorq %rax, %r11\n"
            ),
            vec!["movss (%rbx), %xmm9", "movd %xmm9, %eax", "xorq %rax, %r11"]
        );
        // The VEX spelling is preserved: a legacy `movd` inside VEX code pays
        // the SSE/AVX transition penalty the emitter avoided.
        assert_eq!(
            fuse("    vmovq %xmm3, %rcx\n    movl %ecx, %ecx\n"),
            vec!["vmovd %xmm3, %ecx"]
        );
    }

    /// Every refusal the fusion must make. Each case leaves BOTH lines intact
    /// (the pair is never half-applied) and the surviving text equals the
    /// input's live lines.
    #[test]
    fn xmm_bitcast_fusion_refuses_what_it_cannot_prove() {
        let refused = [
            // not adjacent
            "    movq %xmm9, %rax\n    xorl %r10d, %r10d\n    movl %eax, %eax\n",
            // the self-zext widens a DIFFERENT register
            "    movq %xmm9, %rax\n    movl %ecx, %ecx\n",
            // a real move, not a self-zext
            "    movq %xmm9, %rax\n    movl %eax, %ecx\n",
            // the GPR source is not an XMM register
            "    movq %rdx, %rax\n    movl %eax, %eax\n",
        ];
        for asm in refused {
            let got = fuse(asm);
            let want: Vec<String> = asm
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            assert_eq!(got, want, "must refuse: {asm:?}");
        }
    }

    /// A pinned line is never rewritten: the parameter pre-store the pin marks
    /// is a live ABI contract, not an artifact.
    #[test]
    fn xmm_bitcast_fusion_respects_pins() {
        let mut store = LineStore::new("    movq %xmm9, %rax\n    movl %eax, %eax\n".to_string());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        infos[0].pinned = true;
        eliminate_redundant_self_zext(&mut store, &mut infos);
        assert_eq!(store.get(0).trim(), "movq %xmm9, %rax");
        assert_eq!(store.get(1).trim(), "movl %eax, %eax");
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
