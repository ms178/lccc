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
//! ## 3. The mask moved registers on its way to the test
//!
//! When the masked value's home differs from the masked register, the back end
//! spells the byte mask as a zero-extension into the home and tests THAT:
//!
//! ```text
//!     movzbl %al, %r11d
//!     testq %r11, %r11    ->      testb %al, %al
//!     je .L1                      je .L1
//! ```
//!
//! This is `if ((h & 255) == 0)` on a call result whose `And` was allocated
//! away from `%rax` — hot_web_loop's rare-arm guard, one `movzbl` per loop
//! iteration on the hot path. The zero-extended value and the source byte are
//! zero together, so ZF is exact; PF is set from the LOW BYTE of the result in
//! both forms, which is the same byte; CF and OF are cleared by both. SF is
//! the one flag that differs — the wide test's SF is always 0, `testb`'s is
//! bit 7 of the byte — so the rewrite carries rule 1's exact obligations: an
//! SF-blind reader ([`condition_ignores_sf`]), dead flags after it
//! ([`flags_dead_after`]), and a dead destination value afterwards
//! ([`FileLiveness::live_after`]). Deleting the `movzx` also moves the
//! source's last read DOWN to the test line, so both families take the dirty-
//! liveness discipline below, and the straight-line window between the three
//! lines is enforced by the same [`next_non_debug`] walk as rule 1 — the test
//! must be present (a `movzx` writes no flags, so a bare `jcc` after one
//! consumes some OLDER computation's flags and is no candidate), and nothing
//! else may intervene. A memory-sourced `movzbl (%mem), %R` is a load, not a
//! mask, and stays with `load_mask_test`; a non-canonical high-byte spelling
//! (`%ah`) fails the exact-spelling parse closed.
//!
//! All three rules are skip-switchable (`CCC_PEEPHOLE_SKIP=mask_test`) and all
//! are idempotent: no output contains an `and{l,q} $imm, %R` line with a mask
//! in range, nor a register-to-register `movzx` whose destination's only duty
//! was a zero test.
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
use super::flag_peepholes::{flags_dead_after, is_byteless_comment};
use super::helpers::{extract_jump_target, steps_over_line};
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
    let dst = dst.trim_ascii();
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
    let imm = imm.trim_ascii();
    let imm = if let Some(hex) = imm.strip_prefix("0x").or_else(|| imm.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()?
    } else {
        imm.parse::<u64>().ok()?
    };
    (1..=MAX_MASK).contains(&imm).then_some((imm, fam))
}

/// Parse `movzbl %Sb, %D32` / `movzwl %Sw, %D32` with plain, canonically
/// spelled GP registers, returning `(source REG_NAMES row, source family,
/// destination family)`.  A memory source is a load (out of scope here, see
/// `load_mask_test`), a clipped or high-byte spelling never round-trips
/// through the exact `REG_NAMES` comparison, and anything else fails closed.
fn parse_zext_pair(t: &str) -> Option<(usize, RegId, RegId)> {
    let (row, rest) = if let Some(r) = t.strip_prefix("movzbl ") {
        (3usize, r)
    } else if let Some(r) = t.strip_prefix("movzwl ") {
        (2usize, r)
    } else {
        return None;
    };
    let (src, dst) = rest.split_once(',')?;
    let (src, dst) = (src.trim_ascii(), dst.trim_ascii());
    if !src.starts_with('%') || src.contains('(') || !dst.starts_with('%') || dst.contains('(') {
        return None;
    }
    let s_fam = register_family_fast(src);
    let d_fam = register_family_fast(dst);
    if s_fam > REG_GP_MAX || d_fam > REG_GP_MAX {
        return None;
    }
    // Exact-spelling law: the source must be the canonical low-byte/word name
    // and the destination the 32-bit name — `movzbl %ah, %r11d` masks a
    // DIFFERENT byte than `%al`, and only the canonical rows prove identity.
    (src == REG_NAMES[row][s_fam as usize] && dst == REG_NAMES[1][d_fam as usize])
        .then_some((row, s_fam, d_fam))
}

/// A `setcc`/`jcc`/`cmovcc` whose condition does not consult `%sf`.
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
    let cond = if let Some(c) = mnemonic.strip_prefix("set") {
        c
    } else if let Some(rest) = mnemonic.strip_prefix("cmov") {
        rest.strip_suffix(['w', 'l', 'q']).unwrap_or(rest)
    } else if let Some(c) = mnemonic.strip_prefix('j') {
        c
    } else {
        return false;
    };
    matches!(
        cond,
        "e" | "z"
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
    let is_cmov = infos[reader]
        .trimmed(store.get(reader))
        .split_whitespace()
        .next()
        .is_some_and(|m| m.starts_with("cmov"));
    if is_cmov && (infos[reader].reg_refs & (1u16 << fam) != 0) {
        return None;
    }
    (matches!(
        infos[reader].kind,
        LineKind::CondJmp | LineKind::SetCC { .. }
    ) || is_cmov)
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

/// Rule 3 (see the module documentation): `movzx %S, %D; test %D, %D; jcc`
/// becomes `test{b,w} %S, %S` when the condition is SF-blind, the flags die at
/// the reader, and `%D`'s value dies with them.  Returns `true` on a rewrite.
fn fold_zext_test(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i: usize,
    liveness: &mut Option<FileLiveness>,
    dirty: &mut u16,
) -> bool {
    let Some((row, s_fam, d_fam)) = parse_zext_pair(infos[i].trimmed(store.get(i))) else {
        return false;
    };
    // Never the stack or frame family, in either role — same refusal and same
    // reasoning as rule 1.
    if s_fam == REG_RSP || s_fam == REG_RBP || d_fam == REG_RSP || d_fam == REG_RBP {
        return false;
    }
    // The explicit zero test MUST be present: a `movzx` writes no flags, so a
    // bare `jcc` directly after one reads some older computation's flags and
    // has nothing to do with this value.
    let Some((reader, Some(mid))) = find_reader(store, infos, i, d_fam) else {
        return false;
    };
    if !condition_ignores_sf(infos[reader].trimmed(store.get(reader))) {
        return false;
    }
    // Dirty-liveness discipline (see the pass body): a prior rewrite of either
    // family invalidates the cached solution before it is consulted.
    for fam in [s_fam, d_fam] {
        let bit = 1u16 << fam;
        if *dirty & bit != 0 {
            *dirty &= !bit;
            *liveness = None;
        }
    }
    let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
    if !lv.same_function(i, reader)
        || lv.live_after(reader, d_fam) != Some(false)
        || !flags_dead_after(store, infos, reader + 1)
    {
        return false;
    }
    let s = REG_NAMES[row][s_fam as usize];
    let mnem = if row == 3 { "testb" } else { "testw" };
    replace_line(store, &mut infos[mid], mid, format!("    {mnem} {s}, {s}"));
    mark_nop(&mut infos[i]);
    // The movzx (a write of %D) is gone and the source's last read moved DOWN
    // to the test: both changes can only grow a live set, so both families'
    // next queries must re-solve.
    *dirty |= (1u16 << s_fam) | (1u16 << d_fam);
    true
}

fn parse_reg_pair_row(t: &str, row: usize) -> Option<(RegId, RegId)> {
    let (_, rest) = t.split_once(char::is_whitespace)?;
    let (src, dst) = rest.split_once(',')?;
    let (src, dst) = (src.trim_ascii(), dst.trim_ascii());
    let s_fam = register_family_fast(src);
    let d_fam = register_family_fast(dst);
    if s_fam > REG_GP_MAX || d_fam > REG_GP_MAX {
        return None;
    }
    (src == REG_NAMES[row][s_fam as usize] && dst == REG_NAMES[row][d_fam as usize])
        .then_some((s_fam, d_fam))
}

fn parse_movl_reg_reg(t: &str) -> Option<(RegId, RegId)> {
    t.starts_with("movl ")
        .then(|| parse_reg_pair_row(t, 1))
        .flatten()
        .filter(|&(s, d)| s != REG_RSP && s != REG_RBP && d != REG_RSP && d != REG_RBP && s != d)
}

fn parse_movq_reg_reg(t: &str) -> Option<(RegId, RegId)> {
    t.starts_with("movq ")
        .then(|| parse_reg_pair_row(t, 0))
        .flatten()
        .filter(|&(s, d)| s != REG_RSP && s != REG_RBP && d != REG_RSP && d != REG_RBP && s != d)
}

fn parse_shrl_one_reg32(t: &str) -> Option<RegId> {
    let rest = t.strip_prefix("shrl ")?;
    let (imm, dst) = rest.split_once(',')?;
    if imm.trim_ascii() != "$1" {
        return None;
    }
    let dst = dst.trim_ascii();
    let fam = register_family_fast(dst);
    (fam <= REG_GP_MAX && fam != REG_RSP && fam != REG_RBP && dst == REG_NAMES[1][fam as usize])
        .then_some(fam)
}

fn parse_negl_reg32(t: &str) -> Option<RegId> {
    let dst = t.strip_prefix("negl ")?.trim_ascii();
    let fam = register_family_fast(dst);
    (fam <= REG_GP_MAX && fam != REG_RSP && fam != REG_RBP && dst == REG_NAMES[1][fam as usize])
        .then_some(fam)
}

fn parse_xorl_imm_reg32(t: &str) -> Option<(&str, RegId)> {
    let rest = t.strip_prefix("xorl ")?;
    let (imm, dst) = rest.split_once(',')?;
    let (imm, dst) = (imm.trim_ascii(), dst.trim_ascii());
    if !imm.starts_with('$') {
        return None;
    }
    let imm_body = &imm[1..];
    if imm_body.is_empty()
        || !imm_body
            .bytes()
            .enumerate()
            .all(|(k, b)| b.is_ascii_digit() || (k == 0 && b == b'-'))
    {
        return None;
    }
    let fam = register_family_fast(dst);
    (fam <= REG_GP_MAX && fam != REG_RSP && fam != REG_RBP && dst == REG_NAMES[1][fam as usize])
        .then_some((imm, fam))
}

fn parse_cmovne_reg32(t: &str) -> Option<(RegId, RegId)> {
    let mnem = t.split_whitespace().next()?;
    if !matches!(mnem, "cmovnel" | "cmovnzl") {
        return None;
    }
    parse_reg_pair_row(t, 1)
        .filter(|&(s, d)| s != REG_RSP && s != REG_RBP && d != REG_RSP && d != REG_RBP && s != d)
}

fn parse_mov_reg_reg_allow_rbp(t: &str) -> Option<(RegId, RegId)> {
    if t.starts_with("movl ") {
        parse_reg_pair_row(t, 1)
    } else if t.starts_with("movq ") {
        parse_reg_pair_row(t, 0)
    } else {
        None
    }
    .filter(|&(s, d)| s != REG_RSP && d != REG_RSP && s != d)
}

fn parse_cmovne_reg32_allow_rbp(t: &str) -> Option<(RegId, RegId)> {
    let mnem = t.split_whitespace().next()?;
    if !matches!(mnem, "cmovnel" | "cmovnzl") {
        return None;
    }
    parse_reg_pair_row(t, 1).filter(|&(s, d)| s != REG_RSP && d != REG_RSP && s != d)
}

fn is_neutral_non_flag_op(t: &str, info: &LineInfo, s_fam: RegId, t_fam: RegId) -> bool {
    let LineKind::Other { dest_reg: d } = info.kind else {
        return false;
    };
    if d > REG_GP_MAX || d == s_fam || d == t_fam || d == REG_RSP || d == REG_RBP {
        return false;
    }
    if info.reg_refs & (1u16 << t_fam) != 0 {
        return false;
    }
    if t.split(',')
        .next_back()
        .is_some_and(|dst| dst.contains('('))
    {
        return false;
    }
    let Some(mnem) = t.split_whitespace().next() else {
        return false;
    };
    matches!(
        mnem,
        "movb"
            | "movw"
            | "movl"
            | "movq"
            | "movzbl"
            | "movzwl"
            | "movsbl"
            | "movswl"
            | "movslq"
            | "lea"
            | "leal"
            | "leaq"
            | "addb"
            | "addw"
            | "addl"
            | "addq"
            | "subb"
            | "subw"
            | "subl"
            | "subq"
            | "xorl"
            | "xorq"
            | "orl"
            | "orq"
            | "andl"
            | "andq"
            | "shll"
            | "shlq"
            | "shrl"
            | "shrq"
            | "sarl"
            | "sarq"
            | "negl"
            | "negq"
            | "notl"
            | "notq"
    ) || (matches!(mnem, "imul" | "imull" | "imulq") && t.contains(','))
}

fn ensure_liveness<'a>(
    liveness: &'a mut Option<FileLiveness>,
    dirty: &mut u16,
    store: &LineStore,
    infos: &[LineInfo],
    mask: u16,
) -> &'a FileLiveness {
    if *dirty & mask != 0 {
        *dirty &= !mask;
        *liveness = None;
    }
    liveness.get_or_insert_with(|| FileLiveness::new(store, infos))
}

/// True when flags after `from - 1` are provably dead either directly via
/// `flags_dead_after` or across an immediate unconditional backward `jmp .Lhead`
/// (e.g. a loop latch) whose loop header redefines all flags before any reader.
fn crc_flags_dead_after(store: &LineStore, infos: &[LineInfo], from: usize) -> bool {
    if flags_dead_after(store, infos, from) {
        return true;
    }
    let len = store.len();
    let mut n = from;
    while n < len {
        let info = &infos[n];
        if info.is_nop() {
            n += 1;
            continue;
        }
        let t = info.trimmed(store.get(n));
        if steps_over_line(info, t) || is_byteless_comment(t) {
            n += 1;
            continue;
        }
        if info.kind == LineKind::Jmp
            && let Some(target) = extract_jump_target(t)
        {
            let lo = n.saturating_sub(256);
            for k in (lo..n).rev() {
                if infos[k].is_nop() {
                    continue;
                }
                let kt = infos[k].trimmed(store.get(k));
                if infos[k].is_cfi_start() || infos[k].is_cfi_end() {
                    break;
                }
                if infos[k].kind == LineKind::Label {
                    let lbl = kt.strip_suffix(':').map(str::trim_ascii);
                    if lbl == Some(target) {
                        return flags_dead_after(store, infos, k + 1);
                    }
                    if lbl.is_some_and(|l| !l.starts_with(".L")) {
                        break;
                    }
                }
            }
        }
        return false;
    }
    false
}

/// Fold `movl %S, %D; shrl $1, %D; andl $1, %S; [movq %S, %T;] negl %T` into
/// `movl %S, %D; shrl $1, %D; sbbl %T, %T` when flags after `negl` are dead
/// (and when `movq` is present, `%S` is dead after `movq`).
fn fold_crc_shift_and_neg(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i0: usize,
    liveness: &mut Option<FileLiveness>,
    dirty: &mut u16,
) -> bool {
    let len = store.len();
    let Some((s_fam, d_fam)) = parse_movl_reg_reg(infos[i0].trimmed(store.get(i0))) else {
        return false;
    };
    let i1 = next_non_debug(store, infos, i0 + 1, len);
    let i2 = next_non_debug(store, infos, i1 + 1, len);
    let i3 = next_non_debug(store, infos, i2 + 1, len);
    if i3 >= len || !all_editable(infos, &[i0, i1, i2, i3]) {
        return false;
    }
    if parse_shrl_one_reg32(infos[i1].trimmed(store.get(i1))) != Some(d_fam) {
        return false;
    }
    if parse_inplace_mask(infos[i2].trimmed(store.get(i2))) != Some((1, s_fam)) {
        return false;
    }
    let (opt_mov, neg_idx, t_fam) =
        if let Some((ms, md)) = parse_movq_reg_reg(infos[i3].trimmed(store.get(i3))) {
            if ms != s_fam || md == d_fam {
                return false;
            }
            let i4 = next_non_debug(store, infos, i3 + 1, len);
            if i4 >= len || infos[i4].pinned || infos[i4].is_nop() {
                return false;
            }
            (Some(i3), i4, md)
        } else {
            (None, i3, s_fam)
        };
    if parse_negl_reg32(infos[neg_idx].trimmed(store.get(neg_idx))) != Some(t_fam) {
        return false;
    }
    if !crc_flags_dead_after(store, infos, neg_idx + 1) {
        return false;
    }
    let query_mask = (1u16 << s_fam) | (1u16 << t_fam);
    let lv = ensure_liveness(liveness, dirty, store, infos, query_mask);
    if !lv.same_function(i0, neg_idx) {
        return false;
    }
    if let Some(mov_idx) = opt_mov
        && lv.live_after(mov_idx, s_fam) != Some(false)
    {
        return false;
    }
    let t32 = REG_NAMES[1][t_fam as usize];
    replace_line(
        store,
        &mut infos[neg_idx],
        neg_idx,
        format!("    sbbl {t32}, {t32}"),
    );
    mark_nop(&mut infos[i2]);
    if let Some(mov_idx) = opt_mov {
        mark_nop(&mut infos[mov_idx]);
    }
    *dirty |= query_mask | (1u16 << d_fam);
    true
}

/// Fold the 7-instruction CRC branchless poly step:
/// ```text
///   movl %S, %T; andl $1, %T; shrl $1, %S; movl %S, %U; xorl $K, %U;
///   testl %T, %T; cmovnel %U, %S
/// ```
/// into:
/// ```text
///   shrl $1, %S; sbbl %T, %T; andl $K, %T; xorl %T, %S
/// ```
/// when `%T`, `%U`, and flags are dead after `cmovnel`.
fn fold_crc_shift_xor_cmov(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i0: usize,
    liveness: &mut Option<FileLiveness>,
    dirty: &mut u16,
) -> bool {
    let len = store.len();
    let Some((s_fam, t_fam)) = parse_movl_reg_reg(infos[i0].trimmed(store.get(i0))) else {
        return false;
    };
    let i1 = next_non_debug(store, infos, i0 + 1, len);
    let i2 = next_non_debug(store, infos, i1 + 1, len);
    let i3 = next_non_debug(store, infos, i2 + 1, len);
    let i4 = next_non_debug(store, infos, i3 + 1, len);
    let i5 = next_non_debug(store, infos, i4 + 1, len);
    let i6 = next_non_debug(store, infos, i5 + 1, len);
    if i6 >= len || !all_editable(infos, &[i0, i1, i2, i3, i4, i5, i6]) {
        return false;
    }
    if parse_inplace_mask(infos[i1].trimmed(store.get(i1))) != Some((1, t_fam))
        || parse_shrl_one_reg32(infos[i2].trimmed(store.get(i2))) != Some(s_fam)
    {
        return false;
    }
    let Some((s3, u_fam)) = parse_movl_reg_reg(infos[i3].trimmed(store.get(i3))) else {
        return false;
    };
    if s3 != s_fam || u_fam == t_fam {
        return false;
    }
    let Some((imm_slice, x_fam)) = parse_xorl_imm_reg32(infos[i4].trimmed(store.get(i4))) else {
        return false;
    };
    if x_fam != u_fam
        || !is_zero_test_of(infos[i5].trimmed(store.get(i5)), t_fam)
        || parse_cmovne_reg32(infos[i6].trimmed(store.get(i6))) != Some((u_fam, s_fam))
    {
        return false;
    }
    if !crc_flags_dead_after(store, infos, i6 + 1) {
        return false;
    }
    let imm_owned = imm_slice.to_string();
    let mask = (1u16 << s_fam) | (1u16 << t_fam) | (1u16 << u_fam);
    let lv = ensure_liveness(liveness, dirty, store, infos, mask);
    if !lv.same_function(i0, i6)
        || lv.live_after(i6, t_fam) != Some(false)
        || lv.live_after(i6, u_fam) != Some(false)
    {
        return false;
    }
    let t32 = REG_NAMES[1][t_fam as usize];
    let s32 = REG_NAMES[1][s_fam as usize];
    replace_line(store, &mut infos[i3], i3, format!("    sbbl {t32}, {t32}"));
    replace_line(
        store,
        &mut infos[i4],
        i4,
        format!("    andl {imm_owned}, {t32}"),
    );
    replace_line(store, &mut infos[i5], i5, format!("    xorl {t32}, {s32}"));
    mark_nop(&mut infos[i0]);
    mark_nop(&mut infos[i1]);
    mark_nop(&mut infos[i6]);
    *dirty |= mask;
    true
}

/// Fold the 9-instruction split-register CRC branchless poly step (e.g.
/// `multmodp` / `zng_crc32_combine`):
/// ```text
///   mov %S, %T; andl $1, %T;
///   mov %S, %W; shrl $1, %W;
///   mov %W, %U; xorl $K, %U;
///   testl %T, %T; mov %W, %D; cmovnel %U, %D
/// ```
/// (including swapped `(T, W)` pairs, and `%D == %S`, e.g. `%ebp`) into:
/// ```text
///   [movl %S, %D;] shrl $1, %D; sbbl %T, %T; andl $K, %T; xorl %T, %D
/// ```
/// when `%T`, `%W`, `%U`, and flags are dead after `cmovnel`.
fn fold_crc_split_shift_xor_cmov(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i0: usize,
    liveness: &mut Option<FileLiveness>,
    dirty: &mut u16,
) -> bool {
    let len = store.len();
    let Some((s0, d0)) = parse_mov_reg_reg_allow_rbp(infos[i0].trimmed(store.get(i0))) else {
        return false;
    };
    let i1 = next_non_debug(store, infos, i0 + 1, len);
    let i2 = next_non_debug(store, infos, i1 + 1, len);
    let i3 = next_non_debug(store, infos, i2 + 1, len);
    let i4 = next_non_debug(store, infos, i3 + 1, len);
    let i5 = next_non_debug(store, infos, i4 + 1, len);
    let i6 = next_non_debug(store, infos, i5 + 1, len);
    let i7 = next_non_debug(store, infos, i6 + 1, len);
    let i8 = next_non_debug(store, infos, i7 + 1, len);
    if i8 >= len || !all_editable(infos, &[i0, i1, i2, i3, i4, i5, i6, i7, i8]) {
        return false;
    }
    let Some((s2, d2)) = parse_mov_reg_reg_allow_rbp(infos[i2].trimmed(store.get(i2))) else {
        return false;
    };
    if s0 != s2 {
        return false;
    }
    let (t_fam, w_fam) = if parse_inplace_mask(infos[i1].trimmed(store.get(i1))) == Some((1, d0))
        && parse_shrl_one_reg32(infos[i3].trimmed(store.get(i3))) == Some(d2)
    {
        (d0, d2)
    } else if parse_shrl_one_reg32(infos[i1].trimmed(store.get(i1))) == Some(d0)
        && parse_inplace_mask(infos[i3].trimmed(store.get(i3))) == Some((1, d2))
    {
        (d2, d0)
    } else {
        return false;
    };
    let s_fam = s0;
    if t_fam == REG_RBP || w_fam == REG_RBP || t_fam == w_fam || t_fam == s_fam || w_fam == s_fam {
        return false;
    }
    let Some((s4, u_fam)) = parse_movl_reg_reg(infos[i4].trimmed(store.get(i4))) else {
        return false;
    };
    if s4 != w_fam || u_fam == t_fam || u_fam == s_fam {
        return false;
    }
    let Some((imm_slice, x_fam)) = parse_xorl_imm_reg32(infos[i5].trimmed(store.get(i5))) else {
        return false;
    };
    if x_fam != u_fam || !is_zero_test_of(infos[i6].trimmed(store.get(i6)), t_fam) {
        return false;
    }
    let Some((s7, dst_fam)) = parse_mov_reg_reg_allow_rbp(infos[i7].trimmed(store.get(i7))) else {
        return false;
    };
    if s7 != w_fam || dst_fam == t_fam || dst_fam == u_fam {
        return false;
    }
    if parse_cmovne_reg32_allow_rbp(infos[i8].trimmed(store.get(i8))) != Some((u_fam, dst_fam)) {
        return false;
    }
    if !crc_flags_dead_after(store, infos, i8 + 1) {
        return false;
    }
    let imm_owned = imm_slice.to_string();
    let mask =
        (1u16 << s_fam) | (1u16 << t_fam) | (1u16 << w_fam) | (1u16 << u_fam) | (1u16 << dst_fam);
    let lv = ensure_liveness(liveness, dirty, store, infos, mask);
    if !lv.same_function(i0, i8)
        || lv.live_after(i8, t_fam) != Some(false)
        || lv.live_after(i8, w_fam) != Some(false)
        || lv.live_after(i8, u_fam) != Some(false)
    {
        return false;
    }
    let s32 = REG_NAMES[1][s_fam as usize];
    let t32 = REG_NAMES[1][t_fam as usize];
    let dst32 = REG_NAMES[1][dst_fam as usize];
    if dst_fam == s_fam {
        mark_nop(&mut infos[i0]);
        mark_nop(&mut infos[i1]);
        replace_line(store, &mut infos[i2], i2, format!("    shrl $1, {dst32}"));
        replace_line(store, &mut infos[i3], i3, format!("    sbbl {t32}, {t32}"));
        replace_line(
            store,
            &mut infos[i4],
            i4,
            format!("    andl {imm_owned}, {t32}"),
        );
        replace_line(
            store,
            &mut infos[i5],
            i5,
            format!("    xorl {t32}, {dst32}"),
        );
        mark_nop(&mut infos[i6]);
        mark_nop(&mut infos[i7]);
        mark_nop(&mut infos[i8]);
    } else {
        replace_line(
            store,
            &mut infos[i0],
            i0,
            format!("    movl {s32}, {dst32}"),
        );
        replace_line(store, &mut infos[i1], i1, format!("    shrl $1, {dst32}"));
        replace_line(store, &mut infos[i2], i2, format!("    sbbl {t32}, {t32}"));
        replace_line(
            store,
            &mut infos[i3],
            i3,
            format!("    andl {imm_owned}, {t32}"),
        );
        replace_line(
            store,
            &mut infos[i4],
            i4,
            format!("    xorl {t32}, {dst32}"),
        );
        mark_nop(&mut infos[i5]);
        mark_nop(&mut infos[i6]);
        mark_nop(&mut infos[i7]);
        mark_nop(&mut infos[i8]);
    }
    *dirty |= mask;
    true
}

/// Fold `movl %S, %T; andl $imm, %T; <1..=4 neutral ALU/mov ops>; test %T, %T; reader`
/// into `<neutral ops>; testb $imm, %Sb; reader` when `%T` and flags die at `reader`.
fn fold_copy_and_mask_across_neutral(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i0: usize,
    liveness: &mut Option<FileLiveness>,
    dirty: &mut u16,
) -> bool {
    let len = store.len();
    let Some((s_fam, t_fam)) = parse_movl_reg_reg(infos[i0].trimmed(store.get(i0))) else {
        return false;
    };
    let i1 = next_non_debug(store, infos, i0 + 1, len);
    if i1 >= len || !all_editable(infos, &[i0, i1]) {
        return false;
    }
    let Some((imm, m_fam)) = parse_inplace_mask(infos[i1].trimmed(store.get(i1))) else {
        return false;
    };
    if m_fam != t_fam || imm > MAX_BYTE_MASK {
        return false;
    }
    let mut cur = next_non_debug(store, infos, i1 + 1, len);
    let mut neutral_count = 0usize;
    while cur < len && !infos[cur].pinned && neutral_count < 4 {
        let trimmed = infos[cur].trimmed(store.get(cur));
        if is_zero_test_of(trimmed, t_fam) {
            break;
        }
        if !is_neutral_non_flag_op(trimmed, &infos[cur], s_fam, t_fam) {
            return false;
        }
        neutral_count += 1;
        cur = next_non_debug(store, infos, cur + 1, len);
    }
    if neutral_count == 0 || cur >= len || infos[cur].pinned {
        return false;
    }
    let test_idx = cur;
    if !is_zero_test_of(infos[test_idx].trimmed(store.get(test_idx)), t_fam) {
        return false;
    }
    let reader = next_non_debug(store, infos, test_idx + 1, len);
    if reader >= len || infos[reader].pinned {
        return false;
    }
    let reader_trimmed = infos[reader].trimmed(store.get(reader));
    if !condition_ignores_sf(reader_trimmed) {
        return false;
    }
    let is_cmov = reader_trimmed
        .split_whitespace()
        .next()
        .is_some_and(|m| m.starts_with("cmov"));
    let reader_ok = matches!(
        infos[reader].kind,
        LineKind::CondJmp | LineKind::SetCC { .. }
    ) || is_cmov;
    if !reader_ok || (is_cmov && (infos[reader].reg_refs & (1u16 << t_fam) != 0)) {
        return false;
    }
    if !flags_dead_after(store, infos, reader + 1) {
        return false;
    }
    let mask = (1u16 << s_fam) | (1u16 << t_fam);
    let lv = ensure_liveness(liveness, dirty, store, infos, mask);
    if !lv.same_function(i0, reader) || lv.live_after(reader, t_fam) != Some(false) {
        return false;
    }
    emit_byte_test(store, infos, test_idx, imm, s_fam, None);
    mark_nop(&mut infos[i0]);
    mark_nop(&mut infos[i1]);
    *dirty |= mask;
    true
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
            // Rule 3: the mask spelled as a register-to-register `movzx`
            // whose destination exists only to be zero-tested.
            if fold_zext_test(store, infos, i, &mut liveness, &mut dirty)
                || fold_crc_shift_xor_cmov(store, infos, i, &mut liveness, &mut dirty)
                || fold_crc_split_shift_xor_cmov(store, infos, i, &mut liveness, &mut dirty)
                || fold_crc_shift_and_neg(store, infos, i, &mut liveness, &mut dirty)
                || fold_copy_and_mask_across_neutral(store, infos, i, &mut liveness, &mut dirty)
            {
                changed = true;
            }
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
        let out =
            run("    andl $255, %r10\n    testq %r10, %r10\n    je .L1\n    ret\n.L1:\n    ret\n");
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
        let out =
            run("    andl $3, %r8d\n    cmpl $0, %r8d\n    jne .L3\n    ret\n.L3:\n    ret\n");
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
        let out =
            run("    andl $255, %r10\n    je .L5\n    js .L6\n.L5:\n    ret\n.L6:\n    ret\n");
        assert!(out.contains("andl $255, %r10"), "{out}");
    }

    /// The value must be dead after the reader.
    #[test]
    fn a_value_live_after_the_reader_keeps_the_mask() {
        // A bare `je` is the reader: it writes no flags, so rule 2 cannot
        // re-encode the mask and the only question left is value liveness.
        let out =
            run("    andl $255, %r10\n    je .L7\n    addq %r10, %rax\n    ret\n.L7:\n    ret\n");
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
        let once =
            run("    andl $255, %r10\n    testq %r10, %r10\n    je .Lc\n    ret\n.Lc:\n    ret\n");
        let mut store = LineStore::new(once.clone());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let again = fold_inplace_mask_test(&mut store, &mut infos);
        assert!(!again, "{once}");
        assert_eq!(once, store.build_result(|i| infos[i].is_nop()), "{once}");
    }

    /// Rule 3's witness, verbatim from hot_web_loop's hot path: the call
    /// result's low byte is zero-tested through a zero-extension into the
    /// comparison's own home.
    #[test]
    fn moved_byte_mask_tests_the_source_byte_directly() {
        let out = run(
            "    movzbl %al, %r11d\n    testq %r11, %r11\n    je .L1\n    ret\n.L1:\n    ret\n",
        );
        assert!(out.contains("testb %al, %al"), "{out}");
        assert!(!out.contains("movzbl"), "{out}");
        assert!(!out.contains("testq"), "{out}");
    }

    /// The word-width sibling.
    #[test]
    fn moved_word_mask_tests_the_source_word() {
        let out = run(
            "    movzwl %di, %esi\n    testl %esi, %esi\n    jne .L1\n    ret\n.L1:\n    ret\n",
        );
        assert!(out.contains("testw %di, %di"), "{out}");
        assert!(!out.contains("movzwl"), "{out}");
    }

    /// SF is the flag the byte test changes, so an SF-consulting reader
    /// keeps the wide shape.
    #[test]
    fn sf_reader_keeps_the_moved_mask() {
        let out = run(
            "    movzbl %al, %r11d\n    testq %r11, %r11\n    js .L1\n    ret\n.L1:\n    ret\n",
        );
        assert!(out.contains("movzbl %al, %r11d"), "{out}");
        assert!(out.contains("testq %r11, %r11"), "{out}");
    }

    /// A destination that outlives the reader is a value, not just a test
    /// input; the extension must stay.
    #[test]
    fn live_destination_keeps_the_moved_mask() {
        let out = run(concat!(
            "    movzbl %al, %r11d\n",
            "    testq %r11, %r11\n",
            "    je .L1\n",
            "    movq %r11, %rdx\n",
            "    ret\n",
            ".L1:\n",
            "    ret\n",
        ));
        assert!(out.contains("movzbl %al, %r11d"), "{out}");
        assert!(out.contains("movq %r11, %rdx"), "{out}");
    }

    /// A memory-sourced `movzbl` is a load; that shape belongs to
    /// `load_mask_test` and is refused here.
    #[test]
    fn memory_sourced_zext_is_not_a_moved_mask() {
        let out = run(
            "    movzbl (%rdi), %r11d\n    testq %r11, %r11\n    je .L1\n    ret\n.L1:\n    ret\n",
        );
        assert!(out.contains("movzbl (%rdi), %r11d"), "{out}");
    }

    /// A `movzx` writes no flags: a bare `jcc` directly after one is reading
    /// some OLDER computation's flags and is no candidate.
    #[test]
    fn zext_without_a_test_is_left_alone() {
        let out = run(concat!(
            "    cmpq %rsi, %rdx\n",
            "    movzbl %al, %r11d\n",
            "    je .L1\n",
            "    ret\n",
            ".L1:\n",
            "    ret\n",
        ));
        assert!(out.contains("movzbl %al, %r11d"), "{out}");
        assert!(out.contains("cmpq %rsi, %rdx"), "{out}");
    }

    /// Every GP family has a byte name, including the REX-only ones.
    #[test]
    fn every_family_is_reachable() {
        // `%rsp` and `%rbp` are the frame registers: a mask of either is not a
        // shape any emitter produces, and liveness correctly refuses to call
        // them dead, so they are out of scope rather than special-cased.
        //
        // The trailing write is what makes the masked value dead: `RET_LIVE`
        // keeps the callee-saved families live at a `ret` (the function must
        // have restored them), so a fold on a bare `ret` could only ever fire
        // for the caller-saved ones.  The void marker drops `%rax` for the same
        // reason -- a fixture without it is read as "this `ret` reads `%rax`".
        for fam in (0..16usize).filter(|f| *f != 4 && *f != 5) {
            let q = REG_NAMES[0][fam];
            let out = run_void(&format!(
                "    andq $255, {q}\n    je .Ld\n.Ld:\n    movq $0, {q}\n    ret\n"
            ));
            assert!(
                out.contains(&format!("testb {}", REG_NAMES[3][fam])),
                "{q} -> {out}"
            );
        }
    }

    #[test]
    fn cmov_ignores_sf_and_cross_neutral_copy_mask_folds() {
        let out = run(
            "    movl %edi, %r9d\n    andl $1, %r9d\n    movl %edx, %r10d\n    xorl %esi, %r10d\n    testl %r9d, %r9d\n    cmovnel %r10d, %edx\n    movl %edx, %eax\n    ret\n",
        );
        assert!(out.contains("testb $1, %dil"), "{out}");
        assert!(!out.contains("movl %edi, %r9d"), "{out}");
        assert!(!out.contains("andl $1, %r9d"), "{out}");
        assert!(!out.contains("testl %r9d, %r9d"), "{out}");

        // Live %r9 after cmovnel must NOT fold.
        let kept = run(
            "    movl %edi, %r9d\n    andl $1, %r9d\n    movl %edx, %r10d\n    xorl %esi, %r10d\n    testl %r9d, %r9d\n    cmovnel %r10d, %edx\n    movl %r9d, %eax\n    ret\n",
        );
        assert!(kept.contains("andl $1, %r9d"), "{kept}");
    }

    #[test]
    fn crc_shift_xor_cmov_and_shift_neg_fold_to_sbbl() {
        let poly = run(
            "    movl %esi, %r9d\n    andl $1, %r9d\n    shrl $1, %esi\n    movl %esi, %r10d\n    xorl $-306674912, %r10d\n    testl %r9d, %r9d\n    cmovnel %r10d, %esi\n    movl %esi, %eax\n    ret\n",
        );
        assert!(poly.contains("shrl $1, %esi"), "{poly}");
        assert!(poly.contains("sbbl %r9d, %r9d"), "{poly}");
        assert!(poly.contains("andl $-306674912, %r9d"), "{poly}");
        assert!(poly.contains("xorl %r9d, %esi"), "{poly}");
        assert!(!poly.contains("cmovnel"), "{poly}");

        let byte = run(
            "    movl %r10d, %r11d\n    shrl $1, %r11d\n    andl $1, %r10d\n    negl %r10d\n    andl $-306674912, %r10d\n    xorl %r10d, %r11d\n    movl %r11d, %eax\n    ret\n",
        );
        assert!(byte.contains("shrl $1, %r11d"), "{byte}");
        assert!(byte.contains("sbbl %r10d, %r10d"), "{byte}");
        assert!(!byte.contains("negl"), "{byte}");
        assert!(!byte.contains("andl $1"), "{byte}");

        let byte_movq = run(
            "    movl %r8d, %r10d\n    shrl $1, %r10d\n    andl $1, %r8d\n    movq %r8, %rsi\n    negl %esi\n    andl $-306674912, %esi\n    xorl %esi, %r10d\n    movl %r10d, %eax\n    ret\n",
        );
        assert!(byte_movq.contains("sbbl %esi, %esi"), "{byte_movq}");
        assert!(!byte_movq.contains("movq %r8, %rsi"), "{byte_movq}");

        // Exact 9-instruction shape emitted inside zng_crc32_combine / multmodp
        // (including %ebp as loop-carried b and movq %rdx, %rbp).
        let split_crc = run(concat!(
            "    # LCCC_RET_RDX 0\n",
            "    movl %ebp, %r9d\n",
            "    andl $1, %r9d\n",
            "    movl %ebp, %edx\n",
            "    shrl $1, %edx\n",
            "    movl %edx, %r11d\n",
            "    xorl $-306674912, %r11d\n",
            "    testl %r9d, %r9d\n",
            "    movq %rdx, %rbp\n",
            "    cmovnel %r11d, %ebp\n",
            "    movl %ebp, %eax\n",
            "    ret\n",
        ));
        assert!(split_crc.contains("shrl $1, %ebp"), "{split_crc}");
        assert!(split_crc.contains("sbbl %r9d, %r9d"), "{split_crc}");
        assert!(split_crc.contains("andl $-306674912, %r9d"), "{split_crc}");
        assert!(split_crc.contains("xorl %r9d, %ebp"), "{split_crc}");
        assert!(!split_crc.contains("cmovnel"), "{split_crc}");

        // Loop-latch backward-jmp shape: flags are killed at .Lhead (testl)
        // across `jmp .Lhead`.
        let split_loop = run(concat!(
            "    # LCCC_RET_RDX 0\n",
            ".Lhead:\n",
            "    testl %edi, %esi\n",
            "    je .Lexit\n",
            "    movl %ebp, %r9d\n",
            "    andl $1, %r9d\n",
            "    movl %ebp, %edx\n",
            "    shrl $1, %edx\n",
            "    movl %edx, %r11d\n",
            "    xorl $-306674912, %r11d\n",
            "    testl %r9d, %r9d\n",
            "    movq %rdx, %rbp\n",
            "    cmovnel %r11d, %ebp\n",
            "    jmp .Lhead\n",
            ".Lexit:\n",
            "    movl %ebp, %eax\n",
            "    ret\n",
        ));
        assert!(split_loop.contains("shrl $1, %ebp"), "{split_loop}");
        assert!(split_loop.contains("sbbl %r9d, %r9d"), "{split_loop}");
        assert!(!split_loop.contains("cmovnel"), "{split_loop}");

        // If %edx is live after cmovnel, keep intact (fail closed).
        let split_live = run(concat!(
            "    # LCCC_RET_RDX 0\n",
            "    movl %ebp, %r9d\n",
            "    andl $1, %r9d\n",
            "    movl %ebp, %edx\n",
            "    shrl $1, %edx\n",
            "    movl %edx, %r11d\n",
            "    xorl $-306674912, %r11d\n",
            "    testl %r9d, %r9d\n",
            "    movq %rdx, %rbp\n",
            "    cmovnel %r11d, %ebp\n",
            "    movl %edx, %eax\n",
            "    ret\n",
        ));
        assert!(split_live.contains("cmovnel %r11d, %ebp"), "{split_live}");
    }
}
