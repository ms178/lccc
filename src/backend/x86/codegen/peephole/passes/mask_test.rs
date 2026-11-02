//! In-place byte mask: a redundant zero test, and a needlessly long encoding.
//!
//! Two rewrites of one shape, `and{l,q} $imm, %R` with `1 <= imm <= 255`, i.e. a
//! mask that keeps exactly the low byte and provably clears everything above it.
//!
//! ## 1. The mask is only there to be tested
//!
//! `(uint8_t)x != 0` is a mask followed by a zero test, and the machine already
//! has one instruction for "are any of these bits set":
//!
//! ```text
//!     andl $255, %r10        andl $255, %r10
//!     testq %r10, %r10   ->      je .L1          (or even `testb %r10b, %r10b`)
//!     je .L1
//! ```
//!
//! Both lines collapse into `testb $imm, %r10b` (or `testb %r10b, %r10b` when
//! the mask is 0xFF, which is two bytes shorter). That is one instruction and
//! five to six bytes fewer per site, and — the part that matters — the result of
//! the mask never enters a register, so the dependency chain into the branch is
//! one instruction shorter.
//!
//! Proof, per flag: `and` and `test` both clear CF and OF, and both set ZF and PF
//! from the AND of the operands. ZF is the flag this shape is asking about, and
//! because `imm <= 255` the mask's own result has no bit above bit 7, so the
//! byte-wide test sees exactly the same zero-ness. SF is the one flag that
//! differs: `andl $255`'s result is at most 255, so its SF is always 0, while
//! `testb`'s SF is bit 7 of the masked byte. AF is undefined for both forms, so
//! it is not an observable the rewrite can change. Therefore the rewrite is exact
//! as long as no SF decision can reach the program, which is enforced twice:
//!
//!   * the reader itself must be a `jcc`/`setcc` whose condition does not consult
//!     SF (`js`/`jns`/`jl`/`jge`/… and their `set` forms are refused), and
//!   * every flag is dead after the reader ([`flags_dead_after`]), so no later
//!     instruction can read the SF this rewrite changed.
//!
//! Two more obligations come from deleting the mask:
//!
//!   * the register's *value* must be dead after the reader
//!     ([`FileLiveness::live_after`]); otherwise a later user would see the
//!     unmasked value;
//!   * nothing between the mask and the reader may touch the register. Adjacency
//!     is enforced by walking with [`next_non_debug`], which steps over deleted
//!     lines and debug attribution but not over `.cfi_*`, alignment or opaque
//!     directives — a `.byte 0x9e` in the window is `sahf` and would eat the very
//!     flags this proof is about.
//!
//! ## 2. The mask is the value (encoding only)
//!
//! `andl $255, %r10` is seven bytes: the immediate 255 does not fit the sign-
//! extended `imm8` form (`83 /4 ib`, whose immediate is sign-extended, so 255
//! would become -1), so the assembler has to pick `81 /4 id` with a full 32-bit
//! immediate. The same value and the same zero-extension is
//! `movzbl %r10b, %r10d`, four bytes: no immediate to encode, nothing to decode
//! but a register-to-register source. This is the shortening Intel's and AMD's
//! optimisation manuals describe for masking to a narrow width and then
//! zero-extending.
//!
//! `and` sets flags and `movzx` does not, so this rule is gated on the flags
//! being dead ([`flags_dead_after`]). No register-liveness question arises: the
//! value in `%R` after `movzbl %Rb, %R32` is bit-for-bit the value after
//! `and $255, %R` in both widths — the 32-bit destination write zero-extends
//! exactly as the mask does, which is also why this rule is restricted to the
//! masks `movzx` reproduces *exactly*, `0xFF` and `0xFFFF`: any narrower mask
//! would leave bits that `movzx` keeps. That restriction does not apply to
//! rule 1: `testb $imm, %Rb` tests precisely the bits a narrower mask would
//! have kept. `andl $0xFFFF, %R` is six bytes with a 32-bit immediate; the
//! `movzwl %Rw, %Rl` that computes the same value is three.
//!
//! Both rules are skip-switchable (`CCC_PEEPHOLE_SKIP=mask_test`) and both are
//! idempotent: neither output contains an `and{l,q} $imm, %R` line with a mask
//! in range.
//!
//! ## Relation to `fold_copy_and_mask_into_test`
//!
//! `flag_peepholes::fold_copy_and_mask_into_test` already rewrites
//! `mov %A, %D; and $imm, %D` — and the bare `and` it leaves behind — into a
//! `test` when the masked value is dead and a flag consumer follows
//! immediately. It does NOT reach either shape here, and deliberately so: its
//! consumer must be the very next line, and a `test` *writes* flags, so the
//! two-instruction `and; test %R,%R; jcc` form that `(uint8_t)x != 0` lowers to
//! stops its scan (the consumer it finds writes the flags it needs read) and
//! survives that pass untouched. Nor does it have a single-line encoding rule
//! for the `%D == %A` case, `andl $255, %R` with dead flags, which is the
//! seven-byte form this back end emits for a standalone `(uint8_t)` cast where
//! `fold_copy_and_mask_into_movz` needs a preceding copy to fire.
//!
//! The two passes therefore overlap only where they agree, and both are
//! idempotent, so their order does not matter for correctness; running this one
//! late lets it see the shapes the earlier fixed-point loop has stopped
//! changing.
//!
//! Cross-pollination note: rule 1 is the x86-64 recipient of the i686 back end's
//! `narrow_dead_and_to_testb` (`src/backend/i686/codegen/peephole.rs`), which the
//! 2026-11-01 SWOT listed as an i686-exclusive capability. The port is not a
//! transliteration: the flag-window proof there is a hand-written reader-class
//! scan, while this back end already owns the per-flag model
//! (`flags_written_mask`/`flags_dead_after`), so the same law is stated once
//! against that oracle instead of being re-derived here.

use super::super::types::*;
use super::flag_peepholes::flags_dead_after;
use super::liveness::FileLiveness;

/// The largest mask the *byte* forms can express: everything above bit 7 is
/// already zero, which is what makes the byte-wide test equivalent.
const MAX_BYTE_MASK: u64 = 0xFF;

/// The largest mask either rule can say anything about.  Anything wider has no
/// single narrower instruction that computes the same value, so both rules
/// leave it alone (fail closed in [`parse_inplace_mask`]).
const MAX_MASK: u64 = 0xFFFF;

/// The `and` masks `movzx` reproduces exactly, as `(mnemonic, REG_NAMES row of
/// the source register)`: a full byte (`movzbl`) and a full word (`movzwl`).
/// Both write a 32-bit destination, whose zero-extension into the upper half of
/// the register is exactly what the wider `and` also does.
fn movzx_form(imm: u64) -> Option<(&'static str, usize)> {
    match imm {
        MAX_BYTE_MASK => Some(("movzbl", 3)),
        0xFFFF => Some(("movzwl", 2)),
        _ => None,
    }
}

/// Parse `andl $imm, %reg` / `andq $imm, %reg` for a GP register, returning the
/// mask and the register family. Memory destinations, other widths and
/// unparsable immediates all answer `None` (fail closed).
fn parse_inplace_mask(t: &str) -> Option<(u64, RegId)> {
    let rest = t
        .strip_prefix("andl $")
        .or_else(|| t.strip_prefix("andq $"))?;
    let (imm, dst) = rest.split_once(',')?;
    let dst = dst.trim();
    // A memory destination (`andl $255, -8(%rbp)`) masks a slot; there is no
    // register whose value could be left unmasked, and the byte form would
    // change the memory traffic. Only a bare register is in scope.
    if !dst.starts_with('%') || dst.contains('(') {
        return None;
    }
    let fam = register_family_fast(dst);
    if fam > REG_GP_MAX {
        return None;
    }
    let imm = imm.trim();
    let imm = if let Some(hex) = imm.strip_prefix("0x").or_else(|| imm.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()?
    } else {
        imm.parse::<u64>().ok()?
    };
    (1..=MAX_MASK).contains(&imm).then_some((imm, fam))
}

/// A `setcc`/`jcc` whose condition does not consult `%sf`.
///
/// `and`/`test` agree on ZF, CF, OF and PF, so only a condition that reads SF
/// can distinguish them: the `s` pair, and the signed `l`/`g` families, which
/// combine SF with OF. The unsigned pair (`b`/`a` and friends), equality,
/// `o`/`no` (OF is zero in both forms) and `p`/`np` (PF is identical) are all
/// exact.
fn condition_ignores_sf(t: &str) -> bool {
    let Some(mnemonic) = t.split_whitespace().next() else {
        return false;
    };
    let Some(cond) = mnemonic
        .strip_prefix("set")
        .or_else(|| mnemonic.strip_prefix('j'))
    else {
        return false;
    };
    matches!(
        cond,
        "e"
            | "z"
            | "ne"
            | "nz"
            | "b"
            | "c"
            | "nae"
            | "ae"
            | "nb"
            | "nc"
            | "be"
            | "na"
            | "a"
            | "nbe"
            | "o"
            | "no"
            | "p"
            | "pe"
            | "np"
            | "po"
    )
}

/// Is `t` a zero test of family `fam`: `test{,l,q} %R, %R` or `cmp{,l,q} $0, %R`?
///
/// Both spellings set the same ZF from the same value, so either can be deleted
/// once the mask's own test replaces it.
fn is_zero_test_of(t: &str, fam: RegId) -> bool {
    let l = REG_NAMES[1][fam as usize];
    let q = REG_NAMES[0][fam as usize];
    t == format!("testl {l}, {l}")
        || t == format!("testq {q}, {q}")
        || t == format!("cmpl $0, {l}")
        || t == format!("cmpq $0, {q}")
}

/// Find the flag consumer of the mask at `idx`.
///
/// Two shapes are recognised, both straight-line:
///
/// ```text
///     andl $imm, %R ; test %R, %R ; jcc/setcc     -> Some((reader, Some(test)))
///     andl $imm, %R ; jcc/setcc                   -> Some((reader, None))
/// ```
///
/// Anything else — a label, a branch, an opaque directive, another instruction
/// in the window — answers `None`, and the caller falls back to the
/// encoding-only rule.
fn find_reader(
    store: &LineStore,
    infos: &[LineInfo],
    idx: usize,
    fam: RegId,
) -> Option<(usize, Option<usize>)> {
    let len = infos.len();
    // The only lines this walker crosses are deleted lines and debug
    // attribution: `next_non_debug` refuses `.cfi_*`, alignment and every opaque
    // directive, which is what a proof about flags requires.
    let first = next_non_debug(store, infos, idx + 1, len);
    if first >= len || infos[first].pinned {
        return None;
    }
    let (reader, mid) = if is_zero_test_of(infos[first].trimmed(store.get(first)), fam) {
        let reader = next_non_debug(store, infos, first + 1, len);
        if reader >= len || infos[reader].pinned {
            return None;
        }
        (reader, Some(first))
    } else {
        (first, None)
    };
    matches!(
        infos[reader].kind,
        LineKind::CondJmp | LineKind::SetCC { .. }
    )
    .then_some((reader, mid))
}

/// Replace the mask at `idx` with the byte test and delete the redundant zero
/// test, if there was one.
fn emit_byte_test(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    idx: usize,
    imm: u64,
    fam: RegId,
    mid: Option<usize>,
) {
    let byte = REG_NAMES[3][fam as usize];
    // `testb %Rb, %Rb` (0xFF mask) is the shortest form of "is the byte zero";
    // any other mask needs its immediate. Either way `testb` takes an `imm8`,
    // which every mask in range fits by construction.
    let text = if imm == MAX_BYTE_MASK {
        format!("    testb {byte}, {byte}")
    } else {
        format!("    testb ${imm}, {byte}")
    };
    replace_line(store, &mut infos[idx], idx, text);
    if let Some(k) = mid {
        mark_nop(&mut infos[k]);
    }
}

/// Fold in-place byte masks (see the module documentation).
pub(super) fn fold_inplace_mask_test(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    // Built lazily: most files contain no candidate at all.
    let mut liveness: Option<FileLiveness> = None;
    // A rule-1 rewrite deletes a write of `%R` and a read of it, so a solution
    // computed before that no longer describes the text. Liveness is a backwards
    // problem and deleting a definition can only *grow* a live set, so the cached
    // answer could under-report liveness; each register whose mask was rewritten
    // is therefore marked dirty and its next query re-solves the enclosing
    // function. Same discipline as the push/pop pass.
    let mut dirty = 0u16;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        let Some((imm, fam)) = parse_inplace_mask(t) else {
            i += 1;
            continue;
        };
        // The stack pointer is never a data mask: `andl $255, %rsp` would be a
        // deliberate stack realignment, and no rewrite here may touch it. The
        // frame register is refused with it -- from the text alone a mask of
        // `%rbp` is indistinguishable from a frame adjustment, and neither
        // rewrite is worth getting that wrong over.
        if fam == REG_RSP || fam == REG_RBP {
            i += 1;
            continue;
        }
        let reader = find_reader(store, infos, i, fam);
        if let Some((reader, mid)) = reader
            && imm <= MAX_BYTE_MASK
            && condition_ignores_sf(infos[reader].trimmed(store.get(reader)))
        {
            let bit = 1u16 << fam;
            if dirty & bit != 0 {
                dirty &= !bit;
                liveness = None;
            }
            let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
            // The value must be dead after the reader: nothing may observe the
            // unmasked register afterwards.
            if lv.same_function(i, reader)
                && lv.live_after(reader, fam) == Some(false)
                && flags_dead_after(store, infos, reader + 1)
            {
                emit_byte_test(store, infos, i, imm, fam, mid);
                dirty |= bit;
                changed = true;
                i += 1;
                continue;
            }
        }
        // Rule 2: the value is wanted, the flags are not. Same value, fewer
        // bytes -- and no immediate to decode. Restricted to the masks `movzx`
        // reproduces exactly (see the module documentation).
        let ext = if flags_dead_after(store, infos, i + 1) {
            movzx_form(imm)
        } else {
            None
        };
        if let Some((suffix, row)) = ext {
            let l = REG_NAMES[1][fam as usize];
            let src = REG_NAMES[row][fam as usize];
            replace_line(store, &mut infos[i], i, format!("    {suffix} {src}, {l}"));
            changed = true;
        }
        i += 1;
    }
    changed
}

#[cfg(test)]
mod mask_test_tests {
    use super::super::super::types::*;
    use super::fold_inplace_mask_test;

    fn body_asm(body: &str) -> String {
        format!("f:\n    .cfi_startproc\n{body}    .cfi_endproc\n")
    }

    /// The same body preceded by the prologue marker that says this function
    /// returns nothing in `%rax`.
    ///
    /// A fixture without the marker is conservatively read as "`ret` reads
    /// `%rax`" ([`FileLiveness::ret_rax_marker`] answers `None`), so `%rax` is
    /// live at every `ret` and cannot be the subject of a value-dead fold. The
    /// real emitter always publishes the marker; only a test needs to say so.
    fn void_body_asm(body: &str) -> String {
        format!("f:\n    .cfi_startproc\n    # LCCC_RET_RAX 0\n{body}    .cfi_endproc\n")
    }

    fn run_asm(asm: String) -> String {
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        fold_inplace_mask_test(&mut store, &mut infos);
        store.build_result(|i| infos[i].is_nop())
    }

    fn run(body: &str) -> String {
        run_asm(body_asm(body))
    }

    /// `run` for a function that returns nothing in `%rax`.
    fn run_void(body: &str) -> String {
        run_asm(void_body_asm(body))
    }

    /// The witness from the performance backlog: a self-mask followed by a
    /// zero test and a branch is one `testb`.
    #[test]
    fn mask_plus_zero_test_becomes_one_byte_test() {
        // The branch target is defined on purpose: a jump to an unknown label
        // leaves the whole function unanalysable (`classify` refuses it), and
        // then no liveness-gated rewrite of this pass can fire.
        let out = run(
            "    andl $255, %r10\n    testq %r10, %r10\n    je .L1\n    ret\n.L1:\n    ret\n",
        );
        assert!(out.contains("testb %r10b, %r10b"), "{out}");
        assert!(!out.contains("andl $255"), "{out}");
        assert!(!out.contains("testq"), "{out}");
        assert_eq!(out.matches("je .L1").count(), 1, "{out}");
    }

    /// Without the redundant test the mask still feeds only the condition.
    /// `%ecx` is not a return register, so its masked value is dead here; the
    /// same shape on `%eax` is refused below, because `%eax` *is* the return.
    #[test]
    fn mask_feeding_a_condition_needs_no_zero_test() {
        let out = run("    andl $15, %ecx\n    setne %al\n    ret\n");
        assert!(out.contains("testb $15, %cl"), "{out}");
        assert!(!out.contains("andl"), "{out}");
        let kept = run("    andl $15, %eax\n    jne .L2\n    ret\n.L2:\n    ret\n");
        assert!(kept.contains("andl $15, %eax"), "{kept}");
    }

    /// A `cmp $0` zero test is the same question as `test`.
    #[test]
    fn a_cmp_zero_test_is_deleted_too() {
        let out = run(
            "    andl $3, %r8d\n    cmpl $0, %r8d\n    jne .L3\n    ret\n.L3:\n    ret\n",
        );
        assert!(out.contains("testb $3, %r8b"), "{out}");
        assert!(!out.contains("cmpl $0"), "{out}");
    }

    /// `setcc` is a reader too: a boolean materialisation ends in one.
    #[test]
    fn mask_feeding_a_setcc_is_rewritten() {
        let out = run("    andl $1, %ecx\n    sete %al\n    ret\n");
        assert!(out.contains("testb $1, %cl"), "{out}");
        assert!(!out.contains("andl"), "{out}");
        assert!(out.contains("sete %al"), "{out}");
    }

    /// SF is the one flag the two forms disagree on: a sign test must refuse
    /// the `testb` fold, so the mask (and the zero test feeding the reader)
    /// stays.  Rule 2 may still shorten the encoding where the flags are dead
    /// -- `movzbl` computes the identical value and touches no flag, which the
    /// `testq` after it immediately redefines anyway.
    #[test]
    fn a_sign_reader_keeps_the_mask() {
        let with_test =
            run("    andl $255, %r10\n    testq %r10, %r10\n    js .L4\n    ret\n.L4:\n    ret\n");
        assert!(!with_test.contains("testb"), "{with_test}");
        assert!(with_test.contains("testq %r10, %r10"), "{with_test}");
        for body in [
            "    andl $255, %r10\n    jns .L4\n    ret\n.L4:\n    ret\n",
            "    andl $255, %r10\n    setl %al\n    ret\n",
        ] {
            let out = run(body);
            assert!(out.contains("andl $255, %r10"), "{body} -> {out}");
            assert!(!out.contains("testb"), "{body} -> {out}");
        }
    }

    /// A whole-flags reader after the branch would see the SF difference.
    #[test]
    fn a_later_sf_reader_keeps_the_mask() {
        let out = run("    andl $255, %r10\n    je .L5\n    js .L6\n.L5:\n    ret\n.L6:\n    ret\n");
        assert!(out.contains("andl $255, %r10"), "{out}");
    }

    /// The value must be dead after the reader.
    #[test]
    fn a_value_live_after_the_reader_keeps_the_mask() {
        // A bare `je` is the reader: it writes no flags, so rule 2 cannot
        // re-encode the mask and the only question left is value liveness.
        let out = run(
            "    andl $255, %r10\n    je .L7\n    addq %r10, %rax\n    ret\n.L7:\n    ret\n",
        );
        assert!(out.contains("andl $255, %r10"), "{out}");
        assert!(out.contains("addq %r10, %rax"), "{out}");
    }

    /// Rule 1 is byte-only: `testb $imm, %Rb` cannot express a wider mask, and
    /// the pass must not silently narrow one.  `0xFFFF` is left out here: it is
    /// a whole word, which rule 2 *can* express exactly (`movzwl`), and it is
    /// covered by the zero-extension tests below.
    #[test]
    fn a_word_mask_is_not_a_byte_test() {
        for body in [
            "    andl $256, %r10\n    testq %r10, %r10\n    je .L8\n    ret\n.L8:\n    ret\n",
            "    andl $0x10000, %eax\n    testl %eax, %eax\n    je .L8\n    ret\n.L8:\n    ret\n",
        ] {
            let out = run(body);
            assert!(out.contains("andl"), "{body} -> {out}");
            assert!(!out.contains("testb"), "{body} -> {out}");
        }
    }

    /// A mask with no `movzx` equivalent, or one whose immediate is not a
    /// mask at all, is left alone in both rules.
    #[test]
    fn an_unrepresentable_mask_is_left_alone() {
        for body in [
            "    andl $256, %r10\n    movq %r10, (%rdi)\n    ret\n",
            "    andl $-1, %eax\n    movl %eax, (%rdi)\n    ret\n",
            "    andl $0x10000, %eax\n    movl %eax, (%rdi)\n    ret\n",
        ] {
            let out = run(body);
            assert!(out.contains("andl"), "{body} -> {out}");
        }
    }

    /// The word form is the second mask `movzx` reproduces exactly: six bytes
    /// of `andl $65535` become three of `movzwl`.
    #[test]
    fn a_word_mask_becomes_a_word_zero_extension() {
        let out = run("    andl $65535, %r10\n    movq %r10, (%rdi)\n    ret\n");
        assert!(out.contains("movzwl %r10w, %r10d"), "{out}");
        assert!(!out.contains("andl"), "{out}");
        let wide = run("    andq $65535, %r9\n    movq %r9, (%rdi)\n    ret\n");
        assert!(wide.contains("movzwl %r9w, %r9d"), "{wide}");
    }

    /// A memory destination is a different instruction; leave it.
    #[test]
    fn a_memory_destination_is_left_alone() {
        let out = run("    andl $255, -8(%rbp)\n    je .L9\n    ret\n");
        assert!(out.contains("andl $255, -8(%rbp)"), "{out}");
    }

    /// A store between the mask and the branch consumes the register.
    #[test]
    fn an_intervening_reader_blocks_the_test_fold() {
        let out = run("    andl $255, %r10\n    movq %r10, (%rdi)\n    jne .La\n    ret\n");
        assert!(out.contains("andl $255, %r10"), "{out}");
    }

    /// Rule 2: the value is wanted but the flags are not.
    #[test]
    fn a_dead_flags_mask_becomes_a_zero_extension() {
        let out = run("    andl $255, %r10\n    movq %r10, (%rdi)\n    ret\n");
        assert!(out.contains("movzbl %r10b, %r10d"), "{out}");
        assert!(!out.contains("andl"), "{out}");
    }

    /// `andq` masks the same low byte, and the 32-bit destination write
    /// re-creates the zero-extension the 64-bit mask produced.
    #[test]
    fn a_64_bit_mask_works_too() {
        let out = run("    andq $255, %rax\n    movq %rax, (%rdi)\n    ret\n");
        assert!(out.contains("movzbl %al, %eax"), "{out}");
    }

    /// A narrower mask cannot be expressed by `movzx`, so rule 2 must not fire.
    #[test]
    fn a_narrow_mask_is_not_a_zero_extension() {
        let out = run("    andl $15, %eax\n    movl %eax, (%rdi)\n    ret\n");
        assert!(out.contains("andl $15, %eax"), "{out}");
    }

    /// Live flags also block rule 2: `movzx` writes none.
    #[test]
    fn live_flags_block_the_encoding_rule() {
        let out = run("    andl $255, %r10\n    je .Lb\n    movq %r10, (%rdi)\n    ret\n");
        assert!(out.contains("andl $255, %r10"), "{out}");
    }

    /// Running the pass on its own output changes nothing.
    #[test]
    fn the_rewrite_is_idempotent() {
        let once = run(
            "    andl $255, %r10\n    testq %r10, %r10\n    je .Lc\n    ret\n.Lc:\n    ret\n",
        );
        let mut store = LineStore::new(once.clone());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let again = fold_inplace_mask_test(&mut store, &mut infos);
        assert!(!again, "{once}");
        assert_eq!(once, store.build_result(|i| infos[i].is_nop()), "{once}");
    }

    /// Every GP family has a byte name, including the REX-only ones.
    #[test]
    fn every_family_is_reachable() {
        // `%rsp` and `%rbp` are the frame registers: a mask of either is not a
        // shape any emitter produces, and liveness correctly refuses to call
        // them dead, so they are out of scope rather than special-cased.
        //
        // The `pop` is what makes the masked value dead: `RET_LIVE` keeps the
        // callee-saved families live at a `ret` (the function must have
        // restored them), so a fold on a bare `ret` could only ever fire for
        // the caller-saved ones.  The void marker drops `%rax` for the same
        // reason -- a fixture without it is read as "this `ret` reads `%rax`".
        for fam in (0..16usize).filter(|f| *f != 4 && *f != 5) {
            let q = REG_NAMES[0][fam];
            let out = run_void(&format!(
                "    andq $255, {q}\n    je .Ld\n.Ld:\n    pop {q}\n    ret\n"
            ));
            assert!(
                out.contains(&format!("testb {}", REG_NAMES[3][fam])),
                "{q} -> {out}"
            );
        }
    }
}
