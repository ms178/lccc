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
//! source, and one `mulq` for the product.  Four independent rewrites:
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
//!   `mul` and the `imul` it replaces, and the `imul`'s flags dead.
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
const RBP: RegId = 5;

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
    Some((&t[..at], t[at..].trim()))
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
    let mut parts = inner.split(',').map(str::trim);
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
    let offset = address[..open].trim().parse::<i64>().ok()?;
    let base = address[open..].strip_prefix('(')?.strip_suffix(')')?.trim();
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
        if matches!(dest, RSP | RBP) {
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
    if carriers.contains(&value) || matches!(dest, RSP | RBP) || matches!(value, RSP) {
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
pub(super) fn fold_carry_compare(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    for c in 0..store.len() {
        if infos[c].is_nop() || infos[c].pinned {
            continue;
        }
        let Some((width, right, left)) = sized(infos[c].trimmed(store.get(c)), "cmp") else {
            continue;
        };
        let Some(p) = prev_real(store, infos, c) else {
            continue;
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
            if matches!(sum, RSP | RBP) || !carry_is_the_only_flag_read(store, infos, c) {
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
        if zero != "$0" || matches!(high, RSP | RBP) {
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
/// parked, and whatever register or stack moves the allocator put between
/// the two halves.
const PRODUCT_WINDOW: usize = 8;

/// Fold the low half of a product into the `mul` that already computed it.
///
/// ```text
///     movq  %a, %rax
///     mulq  %b                 # %rdx:%rax = a * b   (imulq %b: same low half)
///     movq  %rdx, HIGH         # any moves that leave %rax, %a, %b alone
///     imulq %b, %a             # low half again
/// ```
///
/// `imulq %b, %a` becomes `movq %rax, %a`; the three-operand spelling
/// `movq %a, %d; imulq %b, %d` becomes `movq %rax, %d` (nothing at all when
/// `%d` is `%rax`).  The low 64 bits of a product do not depend on the
/// signedness of the multiply, so the one-operand `imulq` qualifies too.
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
        let Some(n) = next_real(store, infos, m) else {
            continue;
        };
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
        let is_factor_pair = |x: RegId, y: RegId| (x == a && y == b) || (x == b && y == a);
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
            let Some((_, src, dst)) = sized(line, "mov") else {
                break;
            };
            // `movq %a, %d; imulq %b, %d`: the low half into a third register.
            if let (Some(x), Some(d)) = (Width::Q.register(src), Width::Q.register(dst)) {
                if (x == a || x == b) && d != a && d != b && !matches!(d, RDX | RSP) {
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
            // Otherwise the move must leave the low half and both factors be.
            let info = infos[i];
            if flags_effect(line) != FlagsEffect::Neutral
                || [RAX, a, b]
                    .iter()
                    .any(|&fam| writes_family(&info, line, fam))
            {
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
