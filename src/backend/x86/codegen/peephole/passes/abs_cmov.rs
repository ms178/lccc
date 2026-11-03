//! The sign-magnitude identity as the negate/conditional-move idiom.
//!
//! `sign_magnitude` lowers `x < 0 ? -x : x` -- and `abs`/`labs`/`llabs`,
//! which the frontend expands to that select -- to the branch-free identity
//! `(x ^ (x >> 31)) - (x >> 31)`, emitted as
//!
//! ```text
//!     movl  %edi, %esi        ; S = x
//!     sarl  $31, %esi         ; S = x < 0 ? -1 : 0
//!     xorl  %esi, %edi        ; X = x ^ S
//!     subl  %esi, %edi        ; X = (x ^ S) - S = |x|
//! ```
//!
//! GCC 16.2 and Clang 23.1.4 emit the negate/conditional-move idiom instead:
//!
//! ```text
//!     movl  %edi, %eax        ; R = x
//!     negl  %eax              ; R = -x, SF = (-x < 0)
//!     cmovs %edi, %eax        ; SF ? x : -x
//! ```
//!
//! Three instructions instead of four (five when the result is then copied
//! out), and a two-cycle dependency chain (`neg` -> `cmov`) instead of three
//! (`sar` -> `xor` -> `sub`).  The values agree for every `x`: for `x > 0`,
//! `-x < 0` selects `x`; for `x == 0` both arms are 0; for a negative `x`
//! other than the minimum, `-x > 0` keeps `-x`; for the minimum, `-x` wraps
//! to the minimum, whose sign bit selects `x` -- the minimum, which is also
//! what the identity's wrap-around produces.  This pass rewrites the emitted
//! text only; the IR keeps the identity, which `sign_magnitude`'s own tests
//! certify.
//!
//! # Shapes
//!
//! All lines share one width (`l` with `sar $31`, `q` with `sar $63`), and
//! the registers are plain general-purpose registers with `S != X`:
//!
//! * **in place** -- `mov %X, %S; sar $w-1, %S; xor %S, %X; sub %S, %X`
//!   becomes `mov %X, %S; neg %X; cmovs %S, %X`;
//! * **in place, copied out** -- the same four lines followed by
//!   `mov %X, %R` (`R != X`, any `R` including `S`) with `X` dead after the
//!   copy become `mov %X, %R; neg %R; cmovs %X, %R`: the result lands in `R`
//!   directly (`return_home` leaves this copy in place when `X` is an
//!   argument register);
//! * **out of place** -- `mov %X, %S; sar $w-1, %S; mov %X, %R; xor %S, %R;
//!   sub %S, %R` (`R` distinct from `X` and `S`) becomes `mov %X, %R;
//!   neg %R; cmovs %X, %R`.
//!
//! # When it is legal
//!
//! 1. **The mask dies with the block**: `%S` is dead after the `sub` (the
//!    rewrite leaves `x` in it, or nothing), by the exact CFG dataflow of
//!    `liveness::FileLiveness` -- branches followed, `ret` deciding by the
//!    prologue's return-register markers -- with the fail-closed linear scan
//!    of `pow2_cmov` where that function is unanalysable.  The copied-out
//!    shape also needs `%X` dead after the copy: it keeps `x`, not `|x|`.
//! 2. **No flag reader observes the change**: the block's trailing flags are
//!    the `sub`'s before and the `neg`'s after, so a reader between the block
//!    and the next definite flag writer refuses the rewrite, and so does a
//!    label in between (unknown entry state is not reasoned about).  Nothing
//!    inside the block reads flags; the new `cmovs` reads the `neg` right
//!    before it.
//! 3. **Nothing unusual in the window**: no labels, no comment-bearing
//!    lines, no pinned lines, no `%rsp` -- each aborts rather than
//!    half-applies.
//!
//! The pass runs right after `pow2_cmov`, after `return_home`.
//! `CCC_PEEPHOLE_SKIP=abs_cmov` disables it.

use super::super::types::*;
use super::identical_blocks::reads_flags;
use super::liveness::FileLiveness;
use super::pow2_cmov::{
    definitely_kills_any_flags_text, imm_value, plain_text, prev_non_silence, scratch_live_after,
    split_head, width_letter,
};
use super::relay_and_lea::{plain_gp_operand, split_two_operands};

/// `%rsp`'s family: never a value register here.
const RSP: RegId = 4;

/// Rewrite every recognised identity block.  Returns whether the text
/// changed.
pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    // Exact CFG liveness, built on the first candidate and never refreshed: a
    // rewrite keeps every register's value outside the block except the dead
    // mask (and, for the copied-out shape, the dead `X`), so later answers
    // can only become "dead" -- a stale "live" is conservative.
    let mut liveness: Option<FileLiveness> = None;
    for end in 0..infos.len() {
        if let Some(block) = Block::discover(store, infos, end, &mut liveness)
            && block.apply(store, infos)
        {
            changed = true;
        }
    }
    changed
}

/// A discovered identity, anchored at its `sub`.
struct Block {
    /// Lines the rewrite owns, in order: the three survivors first, then the
    /// lines it deletes.
    lines: Vec<usize>,
    /// The three replacement instructions.
    text: [String; 3],
}

impl Block {
    fn discover(
        store: &LineStore,
        infos: &[LineInfo],
        end: usize,
        liveness: &mut Option<FileLiveness>,
    ) -> Option<Block> {
        // sub: `sub{w} %S, %D`
        let sub = plain_text(store, infos, end)?;
        let width = width_letter(store, infos, end)?;
        let (sub_mnem, sub_rest) = split_head(sub)?;
        if sub_mnem != format!("sub{width}") {
            return None;
        }
        let (s_spelling, d_spelling) = split_two_operands(sub_rest)?;
        let s = plain_gp_operand(s_spelling)?;
        let d = plain_gp_operand(d_spelling)?;
        if s == d || s == RSP || d == RSP {
            return None;
        }

        // xor: `xor{w} %S, %D`
        let xor_i = prev_non_silence(store, infos, end)?;
        let (xor_mnem, xor_rest) = split_head(plain_text(store, infos, xor_i)?)?;
        if xor_mnem != format!("xor{width}") {
            return None;
        }
        let (xs, xd) = split_two_operands(xor_rest)?;
        if xs != s_spelling || xd != d_spelling {
            return None;
        }

        // Either `mov %X, %S; sar; xor; sub` (in place, D == X) or
        // `mov %X, %S; sar; mov %X, %D; xor; sub` (out of place).
        let mut i = prev_non_silence(store, infos, xor_i)?;
        let mut out_of_place: Option<(usize, String)> = None;
        if let Some((mnem, rest)) = plain_text(store, infos, i).and_then(split_head)
            && mnem == format!("mov{width}")
            && let Some((src, dst)) = split_two_operands(rest)
            && dst == d_spelling
        {
            out_of_place = Some((i, src.to_string()));
            i = prev_non_silence(store, infos, i)?;
        }

        // sar: `sar{w} $(31|63), %S`
        let sar_i = i;
        let (sar_mnem, sar_rest) = split_head(plain_text(store, infos, sar_i)?)?;
        if sar_mnem != format!("sar{width}") {
            return None;
        }
        let (count, sar_dst) = split_two_operands(sar_rest)?;
        let full_shift = if width == 'l' { 31 } else { 63 };
        if sar_dst != s_spelling || imm_value(count)? != full_shift {
            return None;
        }

        // mov: `mov{w} %X, %S`
        let mov_i = prev_non_silence(store, infos, sar_i)?;
        let (mov_mnem, mov_rest) = split_head(plain_text(store, infos, mov_i)?)?;
        if mov_mnem != format!("mov{width}") {
            return None;
        }
        let (x_spelling, mov_dst) = split_two_operands(mov_rest)?;
        if mov_dst != s_spelling {
            return None;
        }
        let x = plain_gp_operand(x_spelling)?;
        if x == s || x == RSP {
            return None;
        }
        match &out_of_place {
            // The copy must read the same `x`, into a register that is
            // neither `x` (the cmov source) nor the mask.
            Some((_, src)) if src != x_spelling || d == x => return None,
            None if d != x => return None,
            _ => {}
        }

        let mut region = vec![mov_i, sar_i, xor_i, end];
        if let Some((copy_i, _)) = out_of_place {
            region.push(copy_i);
        }
        if !all_editable(infos, &region) {
            return None;
        }
        for &j in &region {
            if matches!(infos[j].kind, LineKind::Label | LineKind::Ret)
                || store.get(j).contains('#')
                || reads_flags(store, infos, j)
            {
                return None;
            }
        }

        let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
        let live_after = |line: usize, reg: RegId| {
            lv.live_after(line, reg)
                .unwrap_or_else(|| scratch_live_after(store, infos, line, reg))
        };
        if live_after(end, s) {
            return None;
        }

        // In place, optionally copied out: `mov %X, %R` right after the sub
        // with `X` dead after it lets the result land in `R`.
        let copy_out = if out_of_place.is_none() {
            next_non_silence(store, infos, end).and_then(|c| {
                let (mnem, rest) = split_head(plain_text(store, infos, c)?)?;
                if mnem != format!("mov{width}") || infos[c].pinned {
                    return None;
                }
                let (src, dst) = split_two_operands(rest)?;
                let r = plain_gp_operand(dst)?;
                (src == x_spelling && r != x && r != RSP && !live_after(c, x))
                    .then(|| (c, dst.to_string()))
            })
        } else {
            None
        };

        let last = copy_out.as_ref().map_or(end, |(c, _)| *c);
        if flags_observed_after(store, infos, last) {
            return None;
        }

        let indent = "    ";
        let (lines, text) = match (&out_of_place, &copy_out) {
            (Some((copy_i, _)), _) => (
                // mov X,S | sar | mov X,D | xor | sub  ->  mov X,D | neg D | cmovs X,D
                vec![*copy_i, xor_i, end, mov_i, sar_i],
                [
                    format!("{indent}mov{width} {x_spelling}, {d_spelling}"),
                    format!("{indent}neg{width} {d_spelling}"),
                    format!("{indent}cmovs {x_spelling}, {d_spelling}"),
                ],
            ),
            (None, Some((copy_i, r_spelling))) => (
                // mov X,S | sar | xor | sub | mov X,R  ->  mov X,R | neg R | cmovs X,R
                vec![xor_i, end, *copy_i, mov_i, sar_i],
                [
                    format!("{indent}mov{width} {x_spelling}, {r_spelling}"),
                    format!("{indent}neg{width} {r_spelling}"),
                    format!("{indent}cmovs {x_spelling}, {r_spelling}"),
                ],
            ),
            (None, None) => (
                // mov X,S | sar | xor | sub  ->  mov X,S | neg X | cmovs S,X
                vec![mov_i, xor_i, end, sar_i],
                [
                    format!("{indent}mov{width} {x_spelling}, {s_spelling}"),
                    format!("{indent}neg{width} {x_spelling}"),
                    format!("{indent}cmovs {s_spelling}, {x_spelling}"),
                ],
            ),
        };
        Some(Block { lines, text })
    }

    fn apply(&self, store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
        if !all_editable(infos, &self.lines) {
            return false;
        }
        // The three survivors take the replacement text in program order (the
        // slots are listed ascending within each shape); every other owned
        // line is deleted.  Interleaved debug and blank lines stay put.
        let mut survivors = self.lines[..3].to_vec();
        survivors.sort_unstable();
        for (slot, text) in survivors.into_iter().zip(self.text.iter()) {
            replace_line(store, &mut infos[slot], slot, text.clone());
        }
        for &line in &self.lines[3..] {
            mark_nop(&mut infos[line]);
        }
        true
    }
}

/// Whether something can observe the block's trailing flags: a reader before
/// the next definite flag writer, or -- failing closed -- a label, a jump, an
/// inline-asm region or a non-inert directive first (whatever lies beyond
/// is not followed).  `ret` and `call` end the scan: the psABI makes the
/// arithmetic flags dead across both.
fn flags_observed_after(store: &LineStore, infos: &[LineInfo], last: usize) -> bool {
    for j in (last + 1)..infos.len() {
        if infos[j].is_steppable() || infos[j].kind == LineKind::Empty {
            continue;
        }
        let text = infos[j].trimmed(store.get(j));
        match infos[j].kind {
            LineKind::Label | LineKind::Jmp | LineKind::JmpIndirect | LineKind::InlineAsm => {
                return true;
            }
            LineKind::Directive => return true, // non-inert: inert ones are steppable
            LineKind::Ret | LineKind::Call => return false,
            _ => {}
        }
        if super::is_opaque_control_transfer(text) || reads_flags(store, infos, j) {
            return true;
        }
        if definitely_kills_any_flags_text(text) {
            return false;
        }
    }
    false
}

/// First non-silence line strictly after `i` (the mirror of
/// `pow2_cmov::prev_non_silence`).
fn next_non_silence(store: &LineStore, infos: &[LineInfo], i: usize) -> Option<usize> {
    let mut c = i + 1;
    while c < infos.len() {
        if !infos[c].is_steppable() {
            let raw = store.get(c);
            if !(raw.trim_start().starts_with('#')
                || raw.trim().is_empty()
                || crate::backend::peephole_common::inert_directive(raw))
            {
                return Some(c);
            }
        }
        c += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run ONLY `abs_cmov` so every assertion is about this pass.
    fn run_abs(asm: &str) -> String {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        run(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// An `int` function (the prologue's markers say `%eax` is returned).
    fn frame(body: &str) -> String {
        format!(
            "f:\n.cfi_startproc\n    # LCCC_RET_XMM 0\n    # LCCC_RET_RDX 0\n    # LCCC_RET_RAX 1\n{body}.cfi_endproc\n"
        )
    }

    fn instructions(out: &str) -> Vec<&str> {
        out.lines()
            .filter(|l| !l.starts_with('#') && !l.starts_with('.') && !l.ends_with(':'))
            .collect()
    }

    #[test]
    fn abs32_in_place_becomes_neg_cmovs() {
        let out = run_abs(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    xorl %esi, %edi\n",
            "    subl %esi, %edi\n",
            "    movl %edi, %ecx\n",
            "    addl %edi, %ecx\n",
            "    movl %ecx, %eax\n",
            "    ret\n",
        )));
        assert_eq!(
            instructions(&out)[..3],
            ["movl %edi, %esi", "negl %edi", "cmovs %esi, %edi"],
            "{out}"
        );
        assert!(!out.contains("sarl"), "{out}");
    }

    #[test]
    fn abs64_copied_out_lands_in_the_return_register() {
        let out = run_abs(&frame(concat!(
            "    movq %rdi, %rsi\n",
            "    sarq $63, %rsi\n",
            "    xorq %rsi, %rdi\n",
            "    subq %rsi, %rdi\n",
            "    movq %rdi, %rax\n",
            "    ret\n",
        )));
        assert_eq!(
            instructions(&out),
            ["movq %rdi, %rax", "negq %rax", "cmovs %rdi, %rax", "ret"],
            "{out}"
        );
    }

    #[test]
    fn abs32_copied_out_lands_in_the_return_register() {
        let out = run_abs(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    xorl %esi, %edi\n",
            "    subl %esi, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert_eq!(
            instructions(&out),
            ["movl %edi, %eax", "negl %eax", "cmovs %edi, %eax", "ret"],
            "{out}"
        );
    }

    #[test]
    fn a_copy_into_the_mask_register_is_the_result_register() {
        let out = run_abs(&frame(concat!(
            "    movl %edi, %eax\n",
            "    sarl $31, %eax\n",
            "    xorl %eax, %edi\n",
            "    subl %eax, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert_eq!(
            instructions(&out),
            ["movl %edi, %eax", "negl %eax", "cmovs %edi, %eax", "ret"],
            "{out}"
        );
    }

    #[test]
    fn out_of_place_keeps_x() {
        let out = run_abs(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    movl %edi, %eax\n",
            "    xorl %esi, %eax\n",
            "    subl %esi, %eax\n",
            "    addl %edi, %eax\n",
            "    ret\n",
        )));
        assert_eq!(
            instructions(&out),
            [
                "movl %edi, %eax",
                "negl %eax",
                "cmovs %edi, %eax",
                "addl %edi, %eax",
                "ret"
            ],
            "{out}"
        );
    }

    #[test]
    fn x_live_after_the_copy_keeps_the_copy() {
        // `%edi` (= |x|) is read after the copy: rewrite in place, keep it.
        let out = run_abs(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    xorl %esi, %edi\n",
            "    subl %esi, %edi\n",
            "    movl %edi, %eax\n",
            "    imull %edi, %eax\n",
            "    ret\n",
        )));
        assert_eq!(
            instructions(&out),
            [
                "movl %edi, %esi",
                "negl %edi",
                "cmovs %esi, %edi",
                "movl %edi, %eax",
                "imull %edi, %eax",
                "ret"
            ],
            "{out}"
        );
    }

    #[test]
    fn a_live_mask_blocks_the_rewrite() {
        let out = run_abs(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    xorl %esi, %edi\n",
            "    subl %esi, %edi\n",
            "    leal (%rdi,%rsi), %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("sarl $31, %esi"), "{out}");
        assert!(!out.contains("cmovs"), "{out}");
    }

    #[test]
    fn a_flag_reader_after_the_block_blocks_the_rewrite() {
        // `setz` reads the sub's ZF (|x| == 0); after the rewrite it would
        // read the neg's, which happens to agree -- but SF/CF/OF would not,
        // and the pass does not reason per flag.
        let out = run_abs(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    xorl %esi, %edi\n",
            "    subl %esi, %edi\n",
            "    sets %al\n",
            "    movzbl %al, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("subl %esi, %edi"), "{out}");
        assert!(!out.contains("cmovs"), "{out}");
    }

    #[test]
    fn a_label_after_the_block_blocks_the_rewrite() {
        let out = run_abs(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    xorl %esi, %edi\n",
            "    subl %esi, %edi\n",
            ".Lnext:\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("subl %esi, %edi"), "{out}");
    }

    #[test]
    fn a_flag_writer_after_the_block_clears_the_reader_check() {
        let out = run_abs(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    xorl %esi, %edi\n",
            "    subl %esi, %edi\n",
            "    cmpl $9, %edi\n",
            "    setg %al\n",
            "    movzbl %al, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("negl %edi"), "{out}");
        assert!(out.contains("cmovs %esi, %edi"), "{out}");
    }

    #[test]
    fn not_the_identity_is_left_alone() {
        for (sar, xor, sub) in [
            ("sarl $30, %esi", "xorl %esi, %edi", "subl %esi, %edi"), // not a sign mask
            ("sarq $63, %rsi", "xorl %esi, %edi", "subl %esi, %edi"), // mixed widths
            ("sarl $31, %esi", "xorl %edi, %esi", "subl %esi, %edi"), // xor into the mask
            ("sarl $31, %esi", "xorl %esi, %edi", "subl %edi, %esi"), // reversed sub
            ("sarl $31, %esi", "orl %esi, %edi", "subl %esi, %edi"),  // not a xor
            ("sarl $31, %esi", "xorl %esi, %edi", "addl %esi, %edi"), // not a sub
        ] {
            let body = format!(
                "    movl %edi, %esi\n    {sar}\n    {xor}\n    {sub}\n    movl $0, %esi\n    movl %edi, %eax\n    ret\n"
            );
            let out = run_abs(&frame(&body));
            assert!(!out.contains("cmovs"), "{body}\n->\n{out}");
            assert!(out.contains(sar), "{out}");
        }
    }

    #[test]
    fn the_mask_must_come_from_the_same_x() {
        let out = run_abs(&frame(concat!(
            "    movl %edx, %esi\n",
            "    sarl $31, %esi\n",
            "    xorl %esi, %edi\n",
            "    subl %esi, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(!out.contains("cmovs"), "{out}");
    }

    #[test]
    fn a_pinned_line_blocks_the_rewrite() {
        let asm = frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    xorl %esi, %edi\n",
            "    subl %esi, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        ));
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let sar = (0..store.len())
            .find(|&i| store.get(i).contains("sarl"))
            .unwrap();
        infos[sar].pinned = true;
        assert!(!run(&mut store, &mut infos));
    }

    #[test]
    fn the_rewrite_is_value_exact_on_the_boundary_inputs() {
        // The identity and the idiom, evaluated as the machine does.
        fn identity(x: i32) -> i32 {
            let s = x >> 31;
            (x ^ s).wrapping_sub(s)
        }
        fn idiom(x: i32) -> i32 {
            let r = x.wrapping_neg();
            if r < 0 { x } else { r }
        }
        for x in [
            i32::MIN,
            i32::MIN + 1,
            -2,
            -1,
            0,
            1,
            2,
            i32::MAX - 1,
            i32::MAX,
        ] {
            assert_eq!(identity(x), idiom(x), "{x}");
        }
        fn identity64(x: i64) -> i64 {
            let s = x >> 63;
            (x ^ s).wrapping_sub(s)
        }
        fn idiom64(x: i64) -> i64 {
            let r = x.wrapping_neg();
            if r < 0 { x } else { r }
        }
        for x in [i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX] {
            assert_eq!(identity64(x), idiom64(x), "{x}");
        }
    }
}
