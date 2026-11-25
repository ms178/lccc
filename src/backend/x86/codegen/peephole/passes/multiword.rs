//! Multi-word arithmetic: recover `adc`/`sbb` from a materialised carry, and
//! both halves of a product from one `mul`.
//!
//! Multi-word arithmetic reaches the backend as single-word operations --
//! whether the source wrote it that way (portable bignum limbs) or the
//! 128-bit split produced it (`passes::i128_split`).  The carry is spelled
//! as a comparison, `s = a + b; hi += (s < a)`, and the two halves of a
//! 64x64 product as two operations, `UMulHigh(a, b)` and `Mul(a, b)`.
//! Lowered one instruction at a time the first is
//!
//! ```text
//!     addq  %rdx, %rsi        # s = a + b
//!     cmpq  %rdx, %rsi        # s <u b ?
//!     setb  %al
//!     movzbl %al, %eax
//!     addq  %rax, %rbx        # hi += carry
//! ```
//!
//! five instructions with a four-deep dependency chain from `s` to `hi`,
//! where the machine has a carry flag for exactly this: `addq; adcq $0`.
//! GCC 16.2 and Clang 23.1.5 both emit the two-instruction form for this
//! source, and one `mulq` for the product.  Five independent rewrites:
//!
//! 1. [`fold_carry_compare`] -- the carry *producer*.  A `cmp` that recomputes
//!    the carry of the addition (or the borrow of the subtraction) just
//!    before it is redundant when only CF is read from it; an addition
//!    spelled `lea` is turned back into the `add` that sets the flag.
//! 2. [`fold_carry_into_adc`] -- the carry *consumer*.  `setb b; movzb b, r`
//!    followed by an add or subtract of `r` is an add or subtract with
//!    carry.  It reads nothing but CF, so it does not care what set the
//!    flags.
//! 3. [`fold_carry_addend`] -- `adc $0, h; add x, h` is `adc x, h`.
//! 4. [`fold_product_pair`] -- `movq a, %rax; mulq b` leaves the low half of
//!    the product in `%rax`; a following `imulq b, a` recomputes it.
//! 5. [`forward_carry_flag`] -- the carry *between* two links of a chain
//!    (`passes::carry_chain`).  `setc b; movzb b, r; ...; bt $0, r` puts
//!    back into CF what is still there when nothing in between wrote the
//!    flags: `add; adc; adc` with no boolean in between.
//! 6. [`widen_carry_in_place`] -- the carry that does leave the flag (round
//!    a loop, into a result).  `setc b; movzb b, r` becomes `mov $0, r;
//!    setc r`: the same two instructions with one of them, not two, between
//!    the flag and the register.  Late, once, behind every fold above.
//!
//! # Soundness
//!
//! Every rewrite preserves the value of each register and memory cell that
//! is live afterwards, and changes the flags only where they are dead:
//!
//! * the producer fold needs CF of the earlier `add`/`sub` to equal CF of
//!   the `cmp` (the identities are listed at each case) and every reader of
//!   the `cmp` to read CF alone: the next flag user is an `adc`/`sbb`, which
//!   redefines all six flags, or a carry `setcc` after which
//!   [`flags_dead_after`] proves the flags dead;
//! * the consumer fold needs CF at the rewritten instruction to be CF at the
//!   `setcc` -- only the zero extension and register loads lie between, and
//!   none of them writes flags -- the boolean register and its extension
//!   dead afterwards, and every flag dead after the original add (an `adc`
//!   leaves different flags than the `add` did, and an `lea` left the old
//!   ones);
//! * the addend fold needs the flags dead after the pair: one `adc` of both
//!   produces the same sum and different flags;
//! * the product fold needs `%rax` and both factors unchanged between the
//!   `mul` and the `imul` it replaces, and the `imul`'s flags dead;
//! * the forwarding needs CF at the `bt` to be the bit the `bt` tests: no
//!   line between the `setc` and the `bt` writes a flag, the byte or its
//!   extension, and none is a label, a branch or a call.  A `bt` leaves ZF
//!   as it was and the four other flags undefined, so that nothing can
//!   tell a deleted one from one that ran.
//! * the widening in place needs CF at the extension to be CF at the `setc`
//!   and the register untouched from the `setc` on (only flag-neutral lines
//!   that do not name it lie between), and the scratch byte, which is no
//!   longer written, dead behind the extension.  A `mov` of an immediate
//!   writes no flag.
//!
//! No rewrite deletes or duplicates a memory access the program made, and
//! none touches a pinned line or one flagged as a volatile access.  The
//! tests execute every accepted shape, before and after, on a register,
//! memory and carry model.

use super::super::types::*;
use super::flag_peepholes::{FlagsEffect, flags_dead_after, flags_effect};
use super::helpers::writes_family;
use super::liveness::FileLiveness;
use super::relay_and_lea::{
    line_refs_family, plain_gp_operand, provably_dead_lv, split_two_operands,
};

const RAX: RegId = 0;
const RDX: RegId = 2;
const RSP: RegId = 4;

/// Operand width of the instructions this module rewrites.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Width {
    L,
    Q,
}

impl Width {
    fn suffix(self) -> char {
        match self {
            Width::L => 'l',
            Width::Q => 'q',
        }
    }

    fn name(self, fam: RegId) -> &'static str {
        REG_NAMES[usize::from(self == Width::L)][fam as usize]
    }

    /// The family of `text` when it is a plain register spelled at this width.
    fn register(self, text: &str) -> Option<RegId> {
        let fam = plain_gp_operand(text)?;
        (self.name(fam) == text).then_some(fam)
    }
}

/// Split a line into mnemonic and operand text at the first blank.
fn mnemonic(t: &str) -> Option<(&str, &str)> {
    let at = t.find([' ', '\t'])?;
    Some((&t[..at], t[at..].trim_ascii()))
}

/// `OPl A, B` / `OPq A, B` for the given stem.
fn sized<'a>(t: &'a str, stem: &str) -> Option<(Width, &'a str, &'a str)> {
    let (mnem, args) = mnemonic(t)?;
    let width = match mnem.strip_prefix(stem)? {
        "l" => Width::L,
        "q" => Width::Q,
        _ => return None,
    };
    let (src, dst) = split_two_operands(args)?;
    Some((width, src, dst))
}

fn next_real(store: &LineStore, infos: &[LineInfo], i: usize) -> Option<usize> {
    let k = next_non_debug(store, infos, i + 1, store.len());
    (k < store.len()).then_some(k)
}

fn prev_real(store: &LineStore, infos: &[LineInfo], i: usize) -> Option<usize> {
    let mut k = i;
    while k > 0 {
        k -= 1;
        if !debug_transparent(store, infos[k], k) {
            return Some(k);
        }
    }
    None
}

/// True when every line in `lines` may be rewritten or deleted.
fn rewritable(infos: &[LineInfo], lines: &[usize]) -> bool {
    all_editable(infos, lines) && lines.iter().all(|&l| !infos[l].volatile_access)
}

/// `$<decimal>` as a value.
fn immediate(text: &str) -> Option<i64> {
    text.strip_prefix('$')?.parse::<i64>().ok()
}

/// An operand an addition may take besides its register destination, safe
/// to re-read or to hoist over the lines this module deletes: an immediate,
/// a plain register, or memory addressed without any of `clobbered`.
fn stable_operand(text: &str, width: Width, clobbered: &[RegId]) -> bool {
    if immediate(text).is_some() {
        return true;
    }
    if let Some(fam) = width.register(text) {
        return !clobbered.contains(&fam);
    }
    // Memory: a parenthesised address, no segment or symbol games.
    text.contains('(')
        && !text.starts_with('%')
        && !text.starts_with('$')
        && !text.contains(':')
        && !clobbered.iter().any(|&fam| line_refs_family(text, fam))
}

/// `setb`/`setae` and their synonyms: `Some(true)` when the byte is CF.
fn carry_setcc(mnem: &str) -> Option<bool> {
    match mnem {
        "setb" | "setc" | "setnae" => Some(true),
        "setae" | "setnb" | "setnc" => Some(false),
        _ => None,
    }
}

/// How the instruction after `setcc; movzb` consumes the widened carry.
struct Tail<'a> {
    width: Width,
    /// Register that ends up holding `value +/- carry`.
    dest: RegId,
    /// Operand to copy into `dest` first; `None` when `dest` holds it.
    preload: Option<&'a str>,
    subtract: bool,
}

/// `(%A,%B)` / `(%A,%B,1)`: a two-register sum with no displacement.
fn lea_sum(address: &str) -> Option<(RegId, RegId)> {
    let inner = address.strip_prefix('(')?.strip_suffix(')')?;
    let mut parts = inner.split(',').map(str::trim_ascii);
    let (base, index) = (parts.next()?, parts.next()?);
    match parts.next() {
        None | Some("1") => {}
        Some(_) => return None,
    }
    if parts.next().is_some() {
        return None;
    }
    Some((Width::Q.register(base)?, Width::Q.register(index)?))
}

/// `K(%A)`: one register plus a non-zero displacement.
fn lea_offset(address: &str) -> Option<(i64, RegId)> {
    let open = address.find('(')?;
    let offset = address[..open].trim_ascii().parse::<i64>().ok()?;
    let base = address[open..]
        .strip_prefix('(')?
        .strip_suffix(')')?
        .trim_ascii();
    (offset != 0 && i32::try_from(offset).is_ok()).then_some(())?;
    Some((offset, Width::Q.register(base)?))
}

fn parse_tail<'a>(t: &'a str, carry: RegId, byte: RegId) -> Option<Tail<'a>> {
    let carriers = [carry, byte];
    for (stem, subtract) in [("add", false), ("sub", true)] {
        let Some((width, src, dst)) = sized(t, stem) else {
            continue;
        };
        let dest = width.register(dst)?;
        // `%rbp` is an ordinary destination: the instruction being replaced
        // already writes it, so this is not a function that holds its frame
        // there.  Excluding it left the second limb of an unrolled bignum
        // row on `setb; movzbl; leaq (%rdx,%r14), %rbp`.
        if dest == RSP {
            return None;
        }
        // `value +/- carry`, in place.
        if width.register(src) == Some(carry) {
            return (!carriers.contains(&dest)).then_some(Tail {
                width,
                dest,
                preload: None,
                subtract,
            });
        }
        // `carry + value`, into the carry register.
        if !subtract && dest == carry && stable_operand(src, width, &carriers) {
            return Some(Tail {
                width,
                dest,
                preload: Some(src),
                subtract,
            });
        }
        return None;
    }
    // `leaq (%value,%carry), %dest`: the three-operand add.
    let (width, address, dst) = sized(t, "lea")?;
    if width != Width::Q {
        return None;
    }
    let dest = width.register(dst)?;
    let (a, b) = lea_sum(address)?;
    let value = match (a == carry, b == carry) {
        (true, false) => b,
        (false, true) => a,
        _ => return None,
    };
    if carriers.contains(&value) || dest == RSP || value == RSP {
        return None;
    }
    Some(Tail {
        width,
        dest,
        preload: (value != dest).then(|| width.name(value)),
        subtract: false,
    })
}

/// A register load that may sit between the widened carry and the
/// instruction that adds it: `mov SRC, %D`, mentioning neither carrier.
/// Returns the family of `%D`.  A `mov` writes no flag, so the carry flag
/// the `setcc` read is still the carry flag after it.
fn staged_load(t: &str, carriers: [RegId; 2]) -> Option<RegId> {
    let (_, _, dst) = sized(t, "mov")?;
    let dest = plain_gp_operand(dst)?;
    (!carriers.contains(&dest) && !carriers.iter().any(|&fam| line_refs_family(t, fam)))
        .then_some(dest)
}

/// Fold `setb b; movzb b, r; add r, x` into `adc $0, x` -- and the `setae`,
/// subtract, reversed-operand and `lea` forms listed at [`parse_tail`].  A
/// load of `x` may sit between the extension and the add
/// (`movq 16(%rsp), %x; addq %r, %x`): it stays where it is.
///
/// With `c` the carry flag at the `setcc`:
///
/// ```text
/// setb,  add:  x + c          = adc $0, x
/// setb,  sub:  x - c          = sbb $0, x
/// setae, add:  x + (1 - c)    = x - (-1) - c    = sbb $-1, x
/// setae, sub:  x - (1 - c)    = x + (-1) + c    = adc $-1, x
/// ```
///
/// including modular wrap, and at 32 bits with the same zero extension the
/// `addl`/`subl` had.
pub(super) fn fold_carry_into_adc(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let mut liveness = None;
    for j in 0..store.len() {
        if infos[j].is_nop() || infos[j].pinned {
            continue;
        }
        let Some((set_mnem, byte_reg)) = mnemonic(infos[j].trimmed(store.get(j))) else {
            continue;
        };
        let Some(carry_set) = carry_setcc(set_mnem) else {
            continue;
        };
        let Some(byte) = plain_gp_operand(byte_reg) else {
            continue;
        };
        if byte_reg != REG_NAMES[3][byte as usize] {
            continue;
        }
        let Some(k) = next_real(store, infos, j) else {
            continue;
        };
        let Some((zx, zx_args)) = mnemonic(infos[k].trimmed(store.get(k))) else {
            continue;
        };
        if !matches!(zx, "movzbl" | "movzbq") {
            continue;
        }
        let Some((zx_src, zx_dst)) = split_two_operands(zx_args) else {
            continue;
        };
        let Some(carry) = plain_gp_operand(zx_dst) else {
            continue;
        };
        let wide_name = if zx == "movzbq" { Width::Q } else { Width::L };
        if zx_src != byte_reg || wide_name.name(carry) != zx_dst {
            continue;
        }
        let Some(mut l) = next_real(store, infos, k) else {
            continue;
        };
        let mut tail = parse_tail(infos[l].trimmed(store.get(l)), carry, byte);
        if tail.is_none() {
            // `mov SRC, %x` staged between the extension and `add %r, %x`.
            let Some(staged) = staged_load(infos[l].trimmed(store.get(l)), [carry, byte]) else {
                continue;
            };
            let Some(after) = next_real(store, infos, l) else {
                continue;
            };
            tail = parse_tail(infos[after].trimmed(store.get(after)), carry, byte)
                .filter(|t| t.preload.is_none() && t.dest == staged);
            l = after;
        }
        let Some(tail) = tail else {
            continue;
        };
        if !rewritable(infos, &[j, k, l]) || !flags_dead_after(store, infos, l + 1) {
            continue;
        }
        // The boolean and its extension must not be read again -- unless one
        // of them is the register the result lands in.
        let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
        let owned = [j, k, l];
        let mut retired = 0u16;
        for fam in [carry, byte] {
            if fam != tail.dest {
                retired |= 1u16 << fam;
            }
        }
        let cheap = [carry, byte].into_iter().all(|fam| {
            retired & (1u16 << fam) == 0 || provably_dead_lv(lv, store, infos, l, fam, &owned)
        });
        // Around a back edge the dataflow keeps the byte register live
        // through the sequence's own partial write; ask about the program
        // without these lines' mentions (see `fold_setcc_increment`).
        if !cheap
            && !matches!(
                lv.live_after_dropping_region(store, infos, l, j, l, retired),
                Some(live) if live & retired == 0
            )
        {
            continue;
        }

        let w = tail.width.suffix();
        let dest = tail.width.name(tail.dest);
        let update = match (carry_set, tail.subtract) {
            (true, false) => format!("    adc{w} $0, {dest}"),
            (true, true) => format!("    sbb{w} $0, {dest}"),
            (false, false) => format!("    sbb{w} $-1, {dest}"),
            (false, true) => format!("    adc{w} $-1, {dest}"),
        };
        let preload = tail.preload.map(|src| format!("    mov{w} {src}, {dest}"));
        mark_nop(&mut infos[j]);
        match preload {
            Some(copy) => replace_line(store, &mut infos[k], k, copy),
            None => mark_nop(&mut infos[k]),
        }
        replace_line(store, &mut infos[l], l, update);
        lv.refresh_at(store, infos, l);
        changed = true;
    }
    changed
}

/// The lines `(setc, extension)` that put the carry flag into bit 0 of
/// `reg`, when the flag still holds it at line `at`: straight-line code
/// from the `setc` on, with no write of a flag, of the byte, or of `reg`
/// besides the extension itself.
fn carry_still_in_flag(
    store: &LineStore,
    infos: &[LineInfo],
    at: usize,
    reg: RegId,
) -> Option<(usize, usize, RegId)> {
    // What may stand between: a line that neither is control flow nor
    // touches the flags.
    let neutral = |i: usize| {
        let info = infos[i];
        !info.is_barrier()
            && !info.pinned
            && flags_effect(info.trimmed(store.get(i))) == FlagsEffect::Neutral
    };
    // The nearest write of `reg` above: the extension of a byte.
    let mut k = at;
    let byte = loop {
        k = prev_real(store, infos, k)?;
        if !neutral(k) {
            return None;
        }
        let t = infos[k].trimmed(store.get(k));
        if !writes_family(&infos[k], t, reg) {
            continue;
        }
        let (mnem, args) = mnemonic(t)?;
        let (src, dst) = split_two_operands(args)?;
        let width = match mnem {
            "movzbl" => Width::L,
            "movzbq" => Width::Q,
            _ => return None,
        };
        let byte = plain_gp_operand(src)?;
        if width.register(dst) != Some(reg) || src != REG_NAMES[3][byte as usize] {
            return None;
        }
        break byte;
    };
    // The nearest write of the byte above that: `setc`.
    let mut j = k;
    loop {
        j = prev_real(store, infos, j)?;
        let info = infos[j];
        let t = info.trimmed(store.get(j));
        if info.is_barrier() || info.pinned {
            return None;
        }
        if writes_family(&info, t, byte) {
            let (mnem, operand) = mnemonic(t)?;
            return (carry_setcc(mnem) == Some(true) && operand == REG_NAMES[3][byte as usize])
                .then_some((j, k, byte));
        }
        if flags_effect(t) != FlagsEffect::Neutral {
            return None;
        }
    }
}

/// Drop the `bt` that puts a carry back into the flag it never left:
///
/// ```text
/// adcq  y, %rax                     adcq  y, %rax
/// setc  %cl
/// movzbl %cl, %r10d
/// movq  %rax, 8(%rdi)        ->     movq  %rax, 8(%rdi)
/// movq  16(%rsi), %rax              movq  16(%rsi), %rax
/// btl   $0, %r10d
/// adcq  16(%rdx), %rax              adcq  16(%rdx), %rax
/// ```
///
/// The full adder (`IntrinsicOp::Carry`) takes its carry in from a
/// register, with `bt`, because that is where a value lives; between two
/// links of a chain the flag itself has survived, since only moves were
/// emitted in between.  With the `bt` gone the boolean usually has no
/// reader left, and the `setc` and its extension go with it.
pub(super) fn forward_carry_flag(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let mut liveness = None;
    for b in 0..store.len() {
        if infos[b].is_nop() || !rewritable(infos, &[b]) {
            continue;
        }
        let Some((width, bit, tested)) = sized(infos[b].trimmed(store.get(b)), "bt") else {
            continue;
        };
        let Some(reg) = width.register(tested) else {
            continue;
        };
        if immediate(bit) != Some(0) {
            continue;
        }
        let Some((j, k, byte)) = carry_still_in_flag(store, infos, b, reg) else {
            continue;
        };
        let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
        mark_nop(&mut infos[b]);
        lv.refresh_at(store, infos, b);
        changed = true;
        // The boolean, if that was its last reader: the extension first,
        // then the byte it extended.
        if !rewritable(infos, &[j, k]) || !provably_dead_lv(lv, store, infos, k, reg, &[k]) {
            continue;
        }
        mark_nop(&mut infos[k]);
        lv.refresh_at(store, infos, k);
        if byte == reg
            || byte_set_again_unread(store, infos, j, byte)
            || provably_dead_lv(lv, store, infos, j, byte, &[j])
        {
            mark_nop(&mut infos[j]);
            lv.refresh_at(store, infos, j);
        }
    }
    changed
}

/// True when the byte the `setcc` at `j` wrote is written again, by another
/// `setcc`, before anything reads its register.  A `setcc` reads no
/// register and writes one byte, so the earlier one changed nothing that is
/// seen -- which the liveness cannot say, modelling a partial write as a
/// read of the rest.  (Every link of a chain sets the same scratch byte.)
fn byte_set_again_unread(store: &LineStore, infos: &[LineInfo], j: usize, byte: RegId) -> bool {
    let mask = 1u16 << byte;
    let mut n = j;
    loop {
        let Some(next) = next_real(store, infos, n) else {
            return false;
        };
        n = next;
        let info = infos[n];
        let t = info.trimmed(store.get(n));
        if info.is_barrier() || info.pinned || implicit_reg_refs(t.as_bytes()) & mask != 0 {
            return false;
        }
        if info.reg_refs & mask == 0 {
            continue;
        }
        return matches!(info.kind, LineKind::SetCC { reg } if reg == byte)
            && mnemonic(t).is_some_and(|(_, operand)| operand == REG_NAMES[3][byte as usize]);
    }
}

/// True when the flags read at `j` are those of an addition or a
/// subtraction (`add`, `adc`, `sub`, `sbb`): straight-line, flag-neutral
/// code leads from it to `j`.
fn reads_the_carry_of_arithmetic(store: &LineStore, infos: &[LineInfo], j: usize) -> bool {
    let mut p = j;
    loop {
        let Some(above) = prev_real(store, infos, p) else {
            return false;
        };
        p = above;
        let info = infos[p];
        if info.is_barrier() || info.pinned {
            return false;
        }
        let t = info.trimmed(store.get(p));
        if flags_effect(t) == FlagsEffect::Neutral {
            continue;
        }
        return ["adc", "sbb", "add", "sub"]
            .iter()
            .any(|stem| sized(t, stem).is_some());
    }
}

/// Take the carry out of the flag straight into the register it is kept in:
///
/// ```text
/// adcq   (%rdx,%r9,8), %rax         adcq   (%rdx,%r9,8), %rax
/// setc   %cl                 ->     movl   $0, %r11d
/// movzbl %cl, %r11d                 setc   %r11b
/// ```
///
/// Two instructions either way.  On the left the register is ready two
/// steps behind the flag (the extension reads the byte the `setc` wrote),
/// on the right one: the `mov` depends on nothing.  Round a limb loop that
/// path is the loop -- `bt`, `adc`, `setc`, extension -- and where the
/// extension is not eliminated at rename (Ice Lake with current microcode,
/// Zen) it is a cycle in four.  Measured there on four limb loops (2,048
/// limbs 400,000 times): 0.80, 0.99, 1.00 and 0.99 s before, 0.75 s each
/// after.
///
/// Only the carry of an `add`/`adc`/`sub`/`sbb` is rewritten, which is what
/// the full adder emits; a compare's `setcc` keeps the form the other folds
/// look for.  Runs once, late, for the same reason.
pub(super) fn widen_carry_in_place(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut liveness: Option<FileLiveness> = None;
    let mut changed = false;
    // Every link of a chain sets the same scratch byte, so one is dead only
    // once the links that follow have let go of theirs: from the last line
    // up, and again while a sweep still finds one (round a loop the link
    // that follows is the one above).
    while widen_carries_once(store, infos, &mut liveness) {
        changed = true;
    }
    changed
}

/// One sweep of [`widen_carry_in_place`], from the last line up.
fn widen_carries_once(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    liveness: &mut Option<FileLiveness>,
) -> bool {
    let mut changed = false;
    for j in (0..store.len()).rev() {
        if infos[j].is_nop() || !rewritable(infos, &[j]) {
            continue;
        }
        let Some((mnem, operand)) = mnemonic(infos[j].trimmed(store.get(j))) else {
            continue;
        };
        if carry_setcc(mnem) != Some(true) {
            continue;
        }
        let Some(byte) = plain_gp_operand(operand) else {
            continue;
        };
        if operand != REG_NAMES[3][byte as usize] || !reads_the_carry_of_arithmetic(store, infos, j)
        {
            continue;
        }
        // The extension: the next line that names the byte's register.
        // What lies between writes no flag and is no control flow.
        let byte_mask = 1u16 << byte;
        let mut between_refs = 0u16;
        let mut n = j;
        let extension = loop {
            let Some(next) = next_real(store, infos, n) else {
                break None;
            };
            n = next;
            let info = infos[n];
            let t = info.trimmed(store.get(n));
            if info.is_barrier() || info.pinned {
                break None;
            }
            let refs = info.reg_refs | implicit_reg_refs(t.as_bytes());
            if refs & byte_mask != 0 {
                break Some(n);
            }
            if flags_effect(t) != FlagsEffect::Neutral {
                break None;
            }
            between_refs |= refs;
        };
        let Some(k) = extension else {
            continue;
        };
        if !rewritable(infos, &[k]) {
            continue;
        }
        let Some((mnem, args)) = mnemonic(infos[k].trimmed(store.get(k))) else {
            continue;
        };
        let width = match mnem {
            "movzbl" => Width::L,
            "movzbq" => Width::Q,
            _ => continue,
        };
        let Some((src, dst)) = split_two_operands(args) else {
            continue;
        };
        let Some(reg) = width.register(dst) else {
            continue;
        };
        // From the `setc` on the register is the new one: nothing in
        // between may have read the old one.
        if src != REG_NAMES[3][byte as usize] || reg == RSP || between_refs & (1u16 << reg) != 0 {
            continue;
        }
        if byte != reg {
            // The scratch byte is not written any more.  Asked of the
            // rewritten program: a `setcc` is modelled as a read of the
            // rest of its register, which round a loop keeps the scratch
            // alive through this very pair.
            let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
            let dead = byte_set_again_unread(store, infos, k, byte)
                || provably_dead_lv(lv, store, infos, k, byte, &[j, k])
                || matches!(
                    lv.live_after_dropping_region(store, infos, k, j, k, byte_mask),
                    Some(live) if live & byte_mask == 0
                );
            if !dead {
                continue;
            }
        }
        let zero = format!("    movl $0, {}", Width::L.name(reg));
        let set = format!("    setc {}", REG_NAMES[3][reg as usize]);
        replace_line(store, &mut infos[j], j, zero);
        replace_line(store, &mut infos[k], k, set);
        if let Some(lv) = liveness.as_mut() {
            lv.refresh_at(store, infos, j);
        }
        changed = true;
    }
    changed
}

/// True when the flags a compare at `c` sets are read for CF alone: the
/// next flag user is an `adc`/`sbb` (which then redefines every flag), or a
/// carry `setcc` after which the flags are provably dead.  Only register
/// and memory moves may lie in between.
fn carry_is_the_only_flag_read(store: &LineStore, infos: &[LineInfo], c: usize) -> bool {
    let mut n = c;
    for _ in 0..4 {
        let Some(next) = next_real(store, infos, n) else {
            return false;
        };
        n = next;
        let t = infos[n].trimmed(store.get(n));
        let Some((mnem, _)) = mnemonic(t) else {
            return false;
        };
        if matches!(
            mnem,
            "adcb" | "adcw" | "adcl" | "adcq" | "sbbb" | "sbbw" | "sbbl" | "sbbq"
        ) {
            return true;
        }
        if carry_setcc(mnem).is_some() {
            return flags_dead_after(store, infos, n + 1);
        }
        if !matches!(mnem, "movq" | "movl") || flags_effect(t) != FlagsEffect::Neutral {
            return false;
        }
    }
    false
}

/// The nearest line above `c` that reads or writes the flags, when only
/// flag-neutral straight-line code that writes none of the `named`
/// registers separates the two: what the flags at `c` would be without `c`,
/// computed from the values the `named` registers still hold.
fn arithmetic_above(
    store: &LineStore,
    infos: &[LineInfo],
    c: usize,
    named: &[Option<RegId>],
) -> Option<usize> {
    let mut p = c;
    loop {
        p = prev_real(store, infos, p)?;
        let info = infos[p];
        if info.is_barrier() || info.pinned {
            return None;
        }
        let t = info.trimmed(store.get(p));
        if flags_effect(t) != FlagsEffect::Neutral {
            return Some(p);
        }
        if named
            .iter()
            .flatten()
            .any(|&fam| writes_family(&info, t, fam))
        {
            return None;
        }
    }
}

/// Remove a compare that recomputes the carry or borrow of the arithmetic
/// just before it, turning an `lea` addition back into the `add` that sets
/// the flag.  `cmp R, L` sets CF to `L <u R`; with `s' = s + x` and
/// `d' = a - x` modulo 2^width:
///
/// ```text
/// add x, s            ; cmp x, s      CF(add) = s' <u x       = CF(cmp)
/// mov a, s; add x, s  ; cmp a, s      CF(add) = s' <u a       = CF(cmp)
/// mov a, d; sub x, d  ; cmp x, a      CF(sub) = a  <u x       = CF(cmp)
/// lea (a,x), s        ; cmp x|a, s    -> mov a, s; add x, s
/// lea k(a), s         ; cmp $k|a, s   -> mov a, s; add $k, s
/// ```
///
/// A sum is below either addend exactly when the addition carried, and a
/// difference borrows exactly when the minuend is below the subtrahend.
/// The compare read its operands a second time; a deleted compare of a
/// memory operand therefore removes a repeated read, never the only one.
///
/// The arithmetic need not be the line above: the sum is usually stored
/// first (`c[0] += lo; hi += c[0] < lo` is `add; mov s, (m); cmp`).
/// Flag-neutral lines that write neither register the compare names may
/// stand between ([`arithmetic_above`]); the compare then names registers
/// and immediates only, since a store in between may be to the memory it
/// would read again.
pub(super) fn fold_carry_compare(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    for c in 0..store.len() {
        if infos[c].is_nop() || infos[c].pinned {
            continue;
        }
        let Some((width, right, left)) = sized(infos[c].trimmed(store.get(c)), "cmp") else {
            continue;
        };
        let Some(above) = prev_real(store, infos, c) else {
            continue;
        };
        // The line above, or the arithmetic that moves and stores of other
        // registers separate from the compare.
        let adjacent = flags_effect(infos[above].trimmed(store.get(above))) != FlagsEffect::Neutral
            || sized(infos[above].trimmed(store.get(above)), "lea").is_some();
        let p = if adjacent {
            above
        } else {
            let named = [width.register(left), width.register(right)];
            let registers_only =
                named[0].is_some() && (named[1].is_some() || immediate(right).is_some());
            match arithmetic_above(store, infos, c, &named) {
                Some(p) if registers_only => p,
                _ => continue,
            }
        };
        if !rewritable(infos, &[p, c]) {
            continue;
        }
        let prev = infos[p].trimmed(store.get(p));

        // An `lea` sum: rebuild it as the flag-setting add.
        if let Some((lea_width, address, dst)) = sized(prev, "lea") {
            if lea_width != width || left != dst {
                continue;
            }
            let Some(sum) = width.register(dst) else {
                continue;
            };
            // (copied addend, the other addend as a register or a literal)
            let rebuilt = if let Some((a, x)) = lea_sum(address) {
                let compared = width.register(right);
                (sum != a && sum != x && (compared == Some(a) || compared == Some(x)))
                    .then_some((a, Ok(x)))
            } else if let Some((k, a)) = lea_offset(address) {
                (sum != a && (immediate(right) == Some(k) || width.register(right) == Some(a)))
                    .then_some((a, Err(k)))
            } else {
                None
            };
            let Some((a, addend)) = rebuilt else {
                continue;
            };
            if sum == RSP || !carry_is_the_only_flag_read(store, infos, c) {
                continue;
            }
            let w = width.suffix();
            let copy = format!("    mov{w} {}, {dst}", width.name(a));
            let add = match addend {
                Ok(x) => format!("    add{w} {}, {dst}", width.name(x)),
                Err(k) => format!("    add{w} ${k}, {dst}"),
            };
            replace_line(store, &mut infos[p], p, copy);
            replace_line(store, &mut infos[c], c, add);
            changed = true;
            continue;
        }

        let redundant = if let Some((w, x, s)) = sized(prev, "add") {
            let Some(sum) = width.register(s) else {
                continue;
            };
            w == width
                && left == s
                && stable_operand(x, width, &[sum])
                && (right == x || copied_from(store, infos, p, width, s, x) == Some(right))
        } else if let Some((w, x, d)) = sized(prev, "sub") {
            let Some(difference) = width.register(d) else {
                continue;
            };
            w == width
                && right == x
                && stable_operand(x, width, &[difference])
                && copied_from(store, infos, p, width, d, x) == Some(left)
        } else {
            false
        };
        if redundant && carry_is_the_only_flag_read(store, infos, c) {
            mark_nop(&mut infos[c]);
            changed = true;
        }
    }
    changed
}

/// When the line before `p` is `mov %A, DST` at `width` -- so `DST` held a
/// copy of `%A` when the arithmetic at `p` ran -- the spelling of `%A`.
/// `%A` is a register other than `DST`, and `operand` (the arithmetic's
/// other input) does not mention `DST`.
fn copied_from<'a>(
    store: &'a LineStore,
    infos: &[LineInfo],
    p: usize,
    width: Width,
    dst: &str,
    operand: &str,
) -> Option<&'a str> {
    let before = prev_real(store, infos, p)?;
    let (w, src, copy_dst) = sized(infos[before].trimmed(store.get(before)), "mov")?;
    let target = width.register(dst)?;
    let source = width.register(src)?;
    (w == width && copy_dst == dst && source != target && !line_refs_family(operand, target))
        .then_some(src)
}

/// Fold the addend that follows a carry-in into it:
///
/// ```text
/// adc $0, h; add x, h   ->   adc x, h        h + CF + x
/// sbb $0, h; sub x, h   ->   sbb x, h        h - CF - x
/// ```
///
/// The 128-bit split adds the carry to one high word before the other
/// (`(a.hi + c) + b.hi`), so that this fold completes the two-instruction
/// `add; adc` form.  `x` is an immediate, a register other than `h`, or
/// memory not addressed through `h`; the flags after the pair must be dead
/// (one `adc` of both carries where the two-step form may not).
pub(super) fn fold_carry_addend(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    for i in 0..store.len() {
        if infos[i].is_nop() || infos[i].pinned {
            continue;
        }
        let line = infos[i].trimmed(store.get(i));
        let Some((with_carry, plain, (width, zero, dst))) = [("adc", "add"), ("sbb", "sub")]
            .into_iter()
            .find_map(|(with_carry, plain)| Some((with_carry, plain, sized(line, with_carry)?)))
        else {
            continue;
        };
        let Some(high) = width.register(dst) else {
            continue;
        };
        if zero != "$0" || high == RSP {
            continue;
        }
        let Some(n) = next_real(store, infos, i) else {
            continue;
        };
        let Some((next_width, addend, next_dst)) = sized(infos[n].trimmed(store.get(n)), plain)
        else {
            continue;
        };
        if next_width != width
            || next_dst != dst
            || !stable_operand(addend, width, &[high])
            || !rewritable(infos, &[i, n])
            || !flags_dead_after(store, infos, n + 1)
        {
            continue;
        }
        let merged = format!("    {with_carry}{} {addend}, {dst}", width.suffix());
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[n], n, merged);
        changed = true;
    }
    changed
}

/// Lines the product fold looks past after the `mul`: the high half being
/// parked or summed, and whatever the allocator put between the two halves.
const PRODUCT_WINDOW: usize = 8;

/// What those lines may be: `OPl`/`OPq SRC, DST`, writing `DST` and nothing
/// else but flags.  Anything with a second or an implicit output -- `xchg`,
/// `xadd`, a divide, a string instruction, a call -- ends the window.
const PRODUCT_WINDOW_STEMS: [&str; 9] =
    ["mov", "lea", "add", "adc", "sub", "sbb", "and", "or", "xor"];

/// A register that still holds what `source` held at line `at`: the
/// destination of an earlier `movq %source, %X` in the same straight-line
/// run, with neither register written in between.  Every line walked over
/// is one of [`PRODUCT_WINDOW_STEMS`], so "written" is exact.
fn earlier_copy_of(
    store: &LineStore,
    infos: &[LineInfo],
    at: usize,
    source: RegId,
) -> Option<RegId> {
    let mut written = 0u16;
    let mut k = at;
    for _ in 0..PRODUCT_WINDOW {
        k = prev_real(store, infos, k)?;
        let info = infos[k];
        let line = info.trimmed(store.get(k));
        if !PRODUCT_WINDOW_STEMS
            .iter()
            .any(|stem| sized(line, stem).is_some())
        {
            return None;
        }
        if let Some((Width::Q, src, dst)) = sized(line, "mov") {
            if let (Some(from), Some(copy)) = (Width::Q.register(src), Width::Q.register(dst)) {
                if from == source
                    && copy != source
                    && !matches!(copy, RAX | RDX | RSP)
                    && written & (1u16 << copy | 1u16 << source) == 0
                {
                    return Some(copy);
                }
            }
        }
        for fam in 0..=REG_GP_MAX {
            if writes_family(&info, line, fam) {
                written |= 1u16 << fam;
            }
        }
        // `source` was redefined here: anything earlier copied another value.
        if written & (1u16 << source) != 0 {
            return None;
        }
    }
    None
}

/// Fold the low half of a product into the `mul` that already computed it.
///
/// ```text
///     movq  %a, %rax
///     movq  %s, %b             # optional: a factor staged into a scratch
///     mulq  %b                 # %rdx:%rax = a * b   (imulq %b: same low half)
///     leaq  (%rdx,%r9), HIGH   # anything that leaves %rax and the factors be
///     imulq %b, %a             # low half again
/// ```
///
/// `imulq %b, %a` becomes `movq %rax, %a`; the three-operand spelling
/// `movq %a, %d; imulq %b, %d` becomes `movq %rax, %d` (nothing at all when
/// `%d` is `%rax`).  The low 64 bits of a product do not depend on the
/// signedness of the multiply, so the one-operand `imulq` qualifies too.
///
/// With a staged factor, `%s` names the same value as `%b` for as long as
/// neither is written, so the second multiply may spell the factor either
/// way -- a square is `movq %a, %rax; movq %a, %rcx; mulq %rcx; ...;
/// imulq %a, %a`.  So does an earlier copy of `%s` ([`earlier_copy_of`]):
/// a factor that arrives in `%rdx` is parked first, staged from `%rdx`,
/// and multiplied again from where it was parked.
///
/// The lines in between are only looked past, never moved.  They are
/// two-operand moves and arithmetic ([`PRODUCT_WINDOW_STEMS`]), whose one
/// register output is their last operand, and they write none of `%rax`,
/// `%a`, `%b`, `%s`.  The flags they leave are irrelevant; the flags of the
/// replaced `imul` must be dead.
pub(super) fn fold_product_pair(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    for m in 0..store.len() {
        if infos[m].is_nop() {
            continue;
        }
        let Some((Width::Q, a_reg, "%rax")) = sized(infos[m].trimmed(store.get(m)), "mov") else {
            continue;
        };
        let Some(a) = Width::Q.register(a_reg) else {
            continue;
        };
        let Some(mut n) = next_real(store, infos, m) else {
            continue;
        };
        // `movq %s, %b` (or a load into `%b`) between the accumulator load
        // and the multiply.
        let mut staged = None;
        if let Some((Width::Q, src, dst)) = sized(infos[n].trimmed(store.get(n)), "mov") {
            let Some(scratch) = Width::Q.register(dst) else {
                continue;
            };
            let Some(after) = next_real(store, infos, n) else {
                continue;
            };
            staged = Some((Width::Q.register(src), scratch));
            n = after;
        }
        let Some((mul, b_reg)) = mnemonic(infos[n].trimmed(store.get(n))) else {
            continue;
        };
        if !matches!(mul, "mulq" | "imulq") {
            continue;
        }
        // One-operand form only: `b_reg` is a bare register.
        let Some(b) = Width::Q.register(b_reg) else {
            continue;
        };
        if [a, b].iter().any(|fam| matches!(*fam, RAX | RDX | RSP)) {
            continue;
        }
        // The other names of the factor in `%b`: the register it was staged
        // from (unless that is `%rax` or `%rdx`, which the multiply itself
        // redefines) and an earlier copy of that register.  A staging move
        // into anything but `%b` -- above all into `%a`, which would no
        // longer hold the value copied to `%rax` -- is not this shape.
        let mut names = [b; 3];
        match staged {
            Some((_, scratch)) if scratch != b || scratch == a => continue,
            Some((Some(source), _)) => {
                if !matches!(source, RAX | RDX | RSP) {
                    names[1] = source;
                }
                // `n` is the multiply; the staging move is the line before.
                let stage = prev_real(store, infos, n).unwrap_or(n);
                if let Some(copy) = earlier_copy_of(store, infos, stage, source) {
                    names[2] = copy;
                }
            }
            _ => {}
        }
        let is_b = |x: RegId| names.contains(&x);
        let is_factor_pair = |x: RegId, y: RegId| (x == a && is_b(y)) || (is_b(x) && y == a);
        let guarded = [RAX, a, names[0], names[1], names[2]];
        let mut cursor = n;
        for _ in 0..PRODUCT_WINDOW {
            let Some(i) = next_real(store, infos, cursor) else {
                break;
            };
            if !rewritable(infos, &[i]) {
                break;
            }
            let line = infos[i].trimmed(store.get(i));
            // `imulq %b, %a`: the low half, in place.
            if let Some((Width::Q, x, y)) = sized(line, "imul") {
                if let (Some(x), Some(y)) = (Width::Q.register(x), Width::Q.register(y)) {
                    if is_factor_pair(x, y) && flags_dead_after(store, infos, i + 1) {
                        let low = format!("    movq %rax, {}", Width::Q.name(y));
                        replace_line(store, &mut infos[i], i, low);
                        changed = true;
                    }
                }
                break;
            }
            // `movq %a, %d; imulq %b, %d`: the low half into a third register.
            if let Some((_, src, dst)) = sized(line, "mov") {
                if let (Some(x), Some(d)) = (Width::Q.register(src), Width::Q.register(dst)) {
                    if (x == a || is_b(x)) && !guarded[1..].contains(&d) && !matches!(d, RDX | RSP)
                    {
                        let Some(j) = next_real(store, infos, i) else {
                            break;
                        };
                        let second = sized(infos[j].trimmed(store.get(j)), "imul");
                        if let Some((Width::Q, y, target)) = second {
                            if target == dst
                                && Width::Q.register(y).is_some_and(|y| is_factor_pair(x, y))
                                && rewritable(infos, &[j])
                                && flags_dead_after(store, infos, j + 1)
                            {
                                if d == RAX {
                                    mark_nop(&mut infos[i]);
                                } else {
                                    let low = format!("    movq %rax, {dst}");
                                    replace_line(store, &mut infos[i], i, low);
                                }
                                mark_nop(&mut infos[j]);
                                changed = true;
                                break;
                            }
                        }
                    }
                }
            }
            // Otherwise the line must leave the low half and the factors be.
            let info = infos[i];
            let plain = PRODUCT_WINDOW_STEMS
                .iter()
                .any(|stem| sized(line, stem).is_some());
            if !plain || guarded.iter().any(|&fam| writes_family(&info, line, fam)) {
                break;
            }
            cursor = i;
        }
    }
    changed
}

#[cfg(test)]
#[path = "multiword_tests.rs"]
mod tests;
