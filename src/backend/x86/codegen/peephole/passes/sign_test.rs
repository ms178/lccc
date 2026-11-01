//! Sign tests materialized as a shift.
//!
//! A boolean `x < 0` (or the top bit of a byte, word, ... set) reaches the
//! assembler through the generic compare-and-`setcc` path:
//!
//! ```text
//! xorl  %eax, %eax          cmpb  $0, (%rdi)          testb $128, (%rdi)
//! testl %edi, %edi          setl  %dl                 setne %sil
//! setl  %al                 movzbl %dl, %eax          movzbl %sil, %eax
//! ```
//!
//! The answer is the sign bit itself, so one logical shift of the value
//! produces it already zero-extended (and a `test $1` answer is one `and`):
//!
//! ```text
//! movl  %edi, %eax          movsbl (%rdi), %eax       movzbl (%rdi), %eax
//! shrl  $31, %eax           shrl  $31, %eax           shrl  $7, %eax
//! ```
//!
//! GCC 16.2, Clang 23.1.4 and ICX 2026.2 emit exactly the right-hand forms
//! (`int n(int x){return x<0;}`, `int n(signed char*p){return *p<0;}`,
//! `int b(unsigned char*p){return (*p&0x80)!=0;}`; for bit 0 of a wider
//! load, `(*p & 0x100000000L) != 0` -> `movzbl 4(%rdi),%eax; andl $1,%eax`):
//! two instructions, no
//! partial-register `setcc` write and no flags dependency, against three.
//!
//! # Legality
//!
//! * The rewritten lines must be consecutive live instructions (retired,
//!   blank and byteless comment lines between them are transparent; a label
//!   is not, it is a join).
//! * The flags the compare produced must be dead after the sequence: the
//!   shift leaves different flags behind.
//! * `setcc %Xb; movzbl %Xb, %eD` with `X != D`: the rewrite no longer writes
//!   `X`, so `X` must be dead after the `movzbl` (asked of [`FileLiveness`]).
//! * No line may be pinned.
//!
//! `CCC_PEEPHOLE_SKIP=sign_test` disables the pass.

use super::super::types::*;
use super::flag_peepholes::{flags_dead_after, is_byteless_comment};
use super::liveness::FileLiveness;
use super::relay_and_lea::split_two_operands;

/// The next live instruction line after `i`, skipping retired, blank and
/// byteless comment lines.
fn next_live(store: &LineStore, infos: &[LineInfo], i: usize) -> Option<usize> {
    (i + 1..store.len()).find(|&n| {
        let info = &infos[n];
        !(info.is_nop()
            || info.kind == LineKind::Empty
            || is_byteless_comment(info.trimmed(store.get(n))))
    })
}

/// Operand width in bytes from an AT&T size suffix.
fn suffix_width(s: char) -> Option<u32> {
    match s {
        'b' => Some(1),
        'w' => Some(2),
        'l' => Some(4),
        'q' => Some(8),
        _ => None,
    }
}

/// `REG_NAMES` row for a width in bytes.
fn row(width: u32) -> usize {
    match width {
        8 => 0,
        4 => 1,
        2 => 2,
        _ => 3,
    }
}

/// The family of `%reg` when it is spelled at exactly `width` bytes.
fn reg_of_width(op: &str, width: u32) -> Option<RegId> {
    let fam = register_family_fast(op);
    (usize::from(fam) < 16 && REG_NAMES[row(width)][usize::from(fam)] == op).then_some(fam)
}

/// A compare whose flags decide "the sign bit of the tested value".
struct SignCompare<'a> {
    /// The tested operand: a register family or a memory operand.
    source: Source<'a>,
    width: u32,
    /// The `setcc` mnemonics that read the sign answer (not its negation).
    setcc: &'static [&'static str],
    /// `test $1, op`: the answer is bit 0, not the sign bit.
    low_bit: bool,
}

enum Source<'a> {
    Reg(RegId),
    Mem(&'a str),
}

/// `test{w} %r, %r`, `cmp{w} $0, op` (the sign answer is `setl`/`sets`: OF is
/// clear after both), `test{w} $signbit, mem` or `test{w} $1, op` (the answer
/// is `setne`).
fn parse_sign_compare(t: &str) -> Option<SignCompare<'_>> {
    let (mn, rest) = t.split_once(' ')?;
    let (src, dst) = split_two_operands(rest.trim_start())?;
    let width = suffix_width(mn.chars().last()?)?;
    const LESS: &[&str] = &["setl", "sets"];
    const NONZERO: &[&str] = &["setne", "setnz"];
    match &mn[..mn.len() - 1] {
        "test" if src == dst && src.starts_with('%') => Some(SignCompare {
            source: Source::Reg(reg_of_width(src, width)?),
            width,
            setcc: LESS,
            low_bit: false,
        }),
        "cmp" if src == "$0" => Some(SignCompare {
            source: if dst.starts_with('%') {
                Source::Reg(reg_of_width(dst, width)?)
            } else {
                Source::Mem(dst)
            },
            width,
            setcc: LESS,
            low_bit: false,
        }),
        "test" if src == "$1" => Some(SignCompare {
            source: if dst.starts_with('%') {
                Source::Reg(reg_of_width(dst, width)?)
            } else {
                Source::Mem(dst)
            },
            width,
            setcc: NONZERO,
            low_bit: true,
        }),
        "test" if dst.contains('(') => {
            let imm: i64 = src.strip_prefix('$')?.parse().ok()?;
            let bits = width * 8;
            let mask = if bits == 64 {
                u64::MAX
            } else {
                (1u64 << bits) - 1
            };
            ((imm as u64) & mask == 1u64 << (bits - 1)).then_some(SignCompare {
                source: Source::Mem(dst),
                width,
                setcc: NONZERO,
                low_bit: false,
            })
        }
        _ => None,
    }
}

/// The two replacement lines: load or copy the tested value into `%eD`
/// (`%rD` for 64-bit), then shift its sign bit down to bit 0.
///
/// A `setne` after `test $signbit` reads the top bit of an unsigned view, so
/// the load zero-extends; the `setl` forms sign-extend (either is correct for
/// a shift by `bits - 1` of the 32-bit register -- both put the value's sign
/// bit at bit 31 -- but matching the compiler oracles keeps the encoding
/// minimal: `movzbl`/`movsbl` are the same size).
fn replacement(c: &SignCompare<'_>, d: RegId, zero_extend: bool) -> (String, String) {
    let d32 = REG_NAMES[1][usize::from(d)];
    let d64 = REG_NAMES[0][usize::from(d)];
    let src = match c.source {
        Source::Reg(r) => REG_NAMES[row(c.width)][usize::from(r)],
        Source::Mem(m) => m,
    };
    if c.low_bit {
        // Bit 0 is in the operand's first byte whatever its width; a copy of
        // the low 32 bits of a register holds it as well.
        let load = match c.source {
            Source::Reg(r) => format!("    movl {}, {d32}", REG_NAMES[1][usize::from(r)]),
            Source::Mem(m) => format!("    movzbl {m}, {d32}"),
        };
        return (load, format!("    andl $1, {d32}"));
    }
    let (load, dst, shift) = match (c.width, zero_extend) {
        (1, true) => ("movzbl", d32, "shrl $7"),
        (2, true) => ("movzwl", d32, "shrl $15"),
        (1, false) => ("movsbl", d32, "shrl $31"),
        (2, false) => ("movswl", d32, "shrl $31"),
        (4, _) => ("movl", d32, "shrl $31"),
        _ => ("movq", d64, "shrq $63"),
    };
    (
        format!("    {load} {src}, {dst}"),
        format!("    {shift}, {dst}"),
    )
}

/// Rewrite materialized sign tests as a copy (or load) plus a shift.
pub(super) fn materialize_sign_tests(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let mut liveness: Option<FileLiveness> = None;
    let mut i = 0;
    while i < store.len() {
        let Some(edit) = match_at(store, infos, i, &mut liveness) else {
            i += 1;
            continue;
        };
        let Edit {
            first,
            rewrite,
            retire,
            lines,
        } = edit;
        replace_line(store, &mut infos[first], first, lines.0);
        replace_line(store, &mut infos[rewrite], rewrite, lines.1);
        if let Some(r) = retire {
            mark_nop(&mut infos[r]);
        }
        changed = true;
        liveness = None;
        i = rewrite + 1;
    }
    changed
}

struct Edit {
    first: usize,
    rewrite: usize,
    retire: Option<usize>,
    lines: (String, String),
}

/// Recognise one of the three shapes starting at line `i`.
fn match_at(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    liveness: &mut Option<FileLiveness>,
) -> Option<Edit> {
    if infos[i].is_nop() || infos[i].pinned {
        return None;
    }
    let t0 = infos[i].trimmed(store.get(i));
    // Shape 1: `xorl %eD, %eD; test %S, %S; setl %Db`.
    if let Some(rest) = t0.strip_prefix("xorl ") {
        let (a, b) = split_two_operands(rest)?;
        let d = reg_of_width(a, 4).filter(|_| a == b)?;
        let j = next_live(store, infos, i)?;
        let c = parse_sign_compare(infos[j].trimmed(store.get(j)))?;
        let Source::Reg(s) = c.source else {
            return None;
        };
        let k = next_live(store, infos, j)?;
        let (set, x) = infos[k].trimmed(store.get(k)).split_once(' ')?;
        if s == d
            || !c.setcc.contains(&set)
            || reg_of_width(x.trim(), 1) != Some(d)
            || [j, k].iter().any(|&n| infos[n].pinned)
            || !flags_dead_after(store, infos, k + 1)
        {
            return None;
        }
        return Some(Edit {
            first: i,
            rewrite: j,
            retire: Some(k),
            lines: replacement(&c, d, false),
        });
    }
    // Shapes 2 and 3: `cmp $0, op` / `test $signbit, mem`, then
    // `setcc %Xb; movzbl %Xb, %eD`.
    let c = parse_sign_compare(t0)?;
    let j = next_live(store, infos, i)?;
    let (set, x) = infos[j].trimmed(store.get(j)).split_once(' ')?;
    if !c.setcc.contains(&set) {
        return None;
    }
    let x = reg_of_width(x.trim(), 1)?;
    let k = next_live(store, infos, j)?;
    let (a, b) = split_two_operands(infos[k].trimmed(store.get(k)).strip_prefix("movzbl ")?)?;
    let d = reg_of_width(b, 4)?;
    if reg_of_width(a, 1) != Some(x)
        || [j, k].iter().any(|&n| infos[n].pinned)
        || !flags_dead_after(store, infos, k + 1)
    {
        return None;
    }
    if x != d {
        let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
        if lv.live_after(k, x) != Some(false) {
            return None;
        }
    }
    let zero_extend = c.setcc.contains(&"setne");
    Some(Edit {
        first: i,
        rewrite: k,
        retire: Some(j),
        lines: replacement(&c, d, zero_extend),
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::types::{LineStore, classify_line};
    use super::*;

    /// The prologue's return-register markers, as `prologue.rs` emits them
    /// for an `int` function: `%rdx` is not read by its `ret`.
    const INT_RET: &str = "    # LCCC_RET_XMM 0\n    # LCCC_RET_RDX 0\n    # LCCC_RET_RAX 1\n";

    fn run(body: &str) -> (bool, Vec<String>) {
        run_with(INT_RET, body)
    }

    fn run_with(markers: &str, body: &str) -> (bool, Vec<String>) {
        let src = format!("f:\n.cfi_startproc\n{markers}{body}    ret\n.cfi_endproc\n");
        let mut store = LineStore::new(src);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let changed = materialize_sign_tests(&mut store, &mut infos);
        let out = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .filter(|l| {
                !l.is_empty()
                    && !l.starts_with('.')
                    && !l.starts_with("# LCCC_RET_")
                    && !l.ends_with(':')
                    && l != "ret"
            })
            .collect();
        (changed, out)
    }

    #[test]
    fn register_sign_tests_become_shifts() {
        for (test, set, want) in [
            (
                "testl %edi, %edi",
                "setl",
                ["movl %edi, %eax", "shrl $31, %eax"],
            ),
            (
                "testl %edi, %edi",
                "sets",
                ["movl %edi, %eax", "shrl $31, %eax"],
            ),
            (
                "testq %rdi, %rdi",
                "setl",
                ["movq %rdi, %rax", "shrq $63, %rax"],
            ),
            (
                "testb %sil, %sil",
                "setl",
                ["movsbl %sil, %eax", "shrl $31, %eax"],
            ),
            (
                "cmpw $0, %si",
                "setl",
                ["movswl %si, %eax", "shrl $31, %eax"],
            ),
            (
                "testb $1, %sil",
                "setne",
                ["movl %esi, %eax", "andl $1, %eax"],
            ),
        ] {
            let (changed, out) = run(&format!("    xorl %eax, %eax\n    {test}\n    {set} %al\n"));
            assert!(changed, "{test}");
            assert_eq!(out, want, "{test}");
        }
    }

    #[test]
    fn memory_sign_tests_become_loads_and_shifts() {
        for (head, set, want) in [
            (
                "cmpb $0, (%rdi)",
                "setl",
                ["movsbl (%rdi), %eax", "shrl $31, %eax"],
            ),
            (
                "cmpl $0, 8(%rdi)",
                "sets",
                ["movl 8(%rdi), %eax", "shrl $31, %eax"],
            ),
            (
                "cmpq $0, (%rdi,%rsi,8)",
                "setl",
                ["movq (%rdi,%rsi,8), %rax", "shrq $63, %rax"],
            ),
            (
                "testb $128, (%rdi)",
                "setne",
                ["movzbl (%rdi), %eax", "shrl $7, %eax"],
            ),
            (
                "testb $-128, 3(%rdi)",
                "setne",
                ["movzbl 3(%rdi), %eax", "shrl $7, %eax"],
            ),
            (
                "testw $-32768, (%rdi)",
                "setne",
                ["movzwl (%rdi), %eax", "shrl $15, %eax"],
            ),
            (
                "testb $1, 4(%rdi)",
                "setne",
                ["movzbl 4(%rdi), %eax", "andl $1, %eax"],
            ),
            (
                "testl $1, (%rdi)",
                "setne",
                ["movzbl (%rdi), %eax", "andl $1, %eax"],
            ),
            (
                "testq $1, %rsi",
                "setne",
                ["movl %esi, %eax", "andl $1, %eax"],
            ),
        ] {
            let (changed, out) = run(&format!(
                "    {head}\n    {set} %dl\n    movzbl %dl, %eax\n"
            ));
            assert!(changed, "{head}");
            assert_eq!(out, want, "{head}");
        }
    }

    #[test]
    fn refusals() {
        for body in [
            // Not the sign: a value bit, the negated answer, a nonzero compare.
            "    testb $64, (%rdi)\n    setne %dl\n    movzbl %dl, %eax\n",
            "    cmpb $0, (%rdi)\n    setge %dl\n    movzbl %dl, %eax\n",
            "    cmpb $1, (%rdi)\n    setl %dl\n    movzbl %dl, %eax\n",
            "    testb $128, (%rdi)\n    setl %dl\n    movzbl %dl, %eax\n",
            "    testb $1, (%rdi)\n    sete %dl\n    movzbl %dl, %eax\n",
            "    testb $3, (%rdi)\n    setne %dl\n    movzbl %dl, %eax\n",
            // Flags still read afterwards.
            "    cmpb $0, (%rdi)\n    setl %dl\n    movzbl %dl, %eax\n    jl .L1\n.L1:\n",
            // The setcc scratch is read later.
            "    cmpb $0, (%rdi)\n    setl %dl\n    movzbl %dl, %eax\n    addl %edx, %eax\n",
            // A label between the compare and its setcc is a join.
            "    xorl %eax, %eax\n    testl %edi, %edi\n.L2:\n    setl %al\n",
            // The xor'ed register is the tested one, or not the setcc target.
            "    xorl %eax, %eax\n    testl %ecx, %ecx\n    setl %cl\n",
            // Mismatched spellings of the tested register.
            "    xorl %eax, %eax\n    testl %edi, %esi\n    setl %al\n",
        ] {
            let (changed, out) = run(body);
            assert!(!changed, "{body}\n{out:?}");
        }
    }

    #[test]
    fn unmarked_ret_keeps_rdx_live() {
        // Without `# LCCC_RET_RDX 0` the `ret` may return in `%rax:%rdx`, so
        // the `setcc %dl` scratch is live and must keep being written.
        let body = "    cmpb $0, (%rdi)\n    setl %dl\n    movzbl %dl, %eax\n";
        assert!(!run_with("", body).0);
        assert!(run_with(INT_RET, body).0);
    }

    #[test]
    fn transparent_lines_and_same_register() {
        // Comments and blank lines between the three lines are transparent;
        // the setcc scratch may be the destination itself.
        let (changed, out) =
            run("    cmpb $0, (%rdi)\n    # note\n\n    setl %al\n    movzbl %al, %eax\n");
        assert!(changed);
        assert_eq!(out, ["movsbl (%rdi), %eax", "# note", "shrl $31, %eax"]);
    }
}
