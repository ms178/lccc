//! Conditional-add elimination for signed power-of-two division.
//!
//! `div_by_const` expands a signed `x / 2^k` into the bias-add identity:
//!
//! ```text
//!     movl  %edi, %esi        ; S = x
//!     sarl  $31, %esi         ; sign broadcast
//!     shrl  $29, %esi         ; bias = (x < 0) ? 2^k-1 : 0
//!     leal  (%rdi,%rsi), %eax ; biased = x + bias
//!     sarl  $3, %eax          ; quotient = biased >> k
//! ```
//!
//! The value identity is exact for truncating division, but the machine
//! idiom GCC 16.2 emits is a conditionally added constant with a `cmov`
//! selecting between the two arms —
//!
//! ```text
//!     testl  %edi, %edi       ; SF of x
//!     leal   7(%rdi), %eax    ; biased = x + (2^k-1)
//!     cmovns %edi, %eax       ; x >= 0 ? x : biased
//!     sarl   $3, %eax
//! ```
//!
//! — five instructions down to four, and the scratch register that carried
//! the sign chain disappears.  This pass performs exactly that machine-idiom
//! rewrite on the emitted text.  The IR is untouched: the identity remains
//! the documented `div_by_const` expansion, and
//! `pow2_signed_expansions_match_truncating_semantics` still certifies the
//! value algebra this rewrite compiles.
//!
//! # When it is legal
//!
//! 1. **Exact block shape**: five instructions (blank and debug-attribution
//!    lines may interleave) spelling, in order, `mov{l,q} %X, %S`;
//!    `sar{l,q} $(31|63), %S`; `shr{l,q} $(32|64-k), %S`;
//!    `lea{l,q} (%X,%S), %T`; `sar{l,q} $k, %T` — all at one width, with
//!    the scratch `S ≠ X` and the `lea` destination `T ≠ X` (the conditional
//!    `cmov` source must still be live after the `lea`).
//! 2. **No flag reads inside the block** and no flag-reader between the
//!    block's final `sar` and the next flag-writer (or label) downstream:
//!    the block's trailing flags are the `sar`'s both before and after the
//!    rewrite, so every downstream reader keeps its value; unknown entry
//!    state at a label in between is refused rather than reasoned about.
//! 3. **The scratch dies with the block**: no reader of `%S` after the sign
//!    chain until its next definition — deleting the two shifts would
//!    otherwise strand later readers.
//! 4. **Immediates fit**: the conditional `lea` displaces `2^k-1` as a
//!    signed 32-bit displacement, so `1 ≤ k ≤ 31` (a wider shift keeps the
//!    original identity).
//! 5. **Nothing unusual in the window**: no labels, no comment-bearing
//!    lines, no pinned lines — each aborts rather than half-applies.
//!
//! The pass runs after `return_home`, so on terminal paths the `lea`
//! destination already *is* `%eax` and the rewrite lands straight in the
//! return register.  `CCC_PEEPHOLE_SKIP=pow2_cmov` disables it.

use super::super::types::*;
use super::identical_blocks::reads_flags;
use super::relay_and_lea::{plain_gp_operand, split_two_operands};

/// Rewrite every recognised sign-chain block.  Returns whether the text
/// changed.
pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    for end in 0..infos.len() {
        if let Some(block) = Block::discover(store, infos, end) {
            if block.apply(store, infos) {
                changed = true;
            }
        }
    }
    changed
}

/// A discovered five-instruction sign chain, anchored at its final `sar`.
struct Block {
    /// Instruction line indices: [mov, sign, bias, lea, sar].
    lines: [usize; 5],
    x_spelling: String,
    base_spelling: String,
    t_spelling: String,
    width: char,
    k: u32,
}

impl Block {
    fn discover(store: &LineStore, infos: &[LineInfo], end: usize) -> Option<Block> {
        let e = plain_sar(store, infos, end)?;
        // Walking down from the final sar: lea, bias shift, sign shift, mov.
        let lea_i = prev_non_silence(store, infos, end)?;
        let bias_i = prev_non_silence(store, infos, lea_i)?;
        let sign_i = prev_non_silence(store, infos, bias_i)?;
        let mov_i = prev_non_silence(store, infos, sign_i)?;

        let mov = plain_text(store, infos, mov_i)?;
        let sign = plain_text(store, infos, sign_i)?;
        let bias = plain_text(store, infos, bias_i)?;
        let lea = plain_text(store, infos, lea_i)?;

        let width = width_letter(store, infos, end)?;
        for line in [mov_i, sign_i, bias_i, lea_i] {
            if width_letter(store, infos, line)? != width {
                return None;
            }
        }

        // mov: `mov{l,q} %X, %S`
        let (mov_mnem, mov_rest) = split_head(mov)?;
        if mov_mnem != format!("mov{width}") {
            return None;
        }
        let (x_spelling, scratch) = split_two_operands(mov_rest)?;
        let x = plain_gp_operand(x_spelling)?;
        let s = plain_gp_operand(scratch)?;
        if x == s {
            return None;
        }

        // sign: `sar{l,q} $(31|63), %S`
        let (sign_mnem, sign_rest) = split_head(sign)?;
        if sign_mnem != format!("sar{width}") {
            return None;
        }
        let (sign_src, sign_dst) = split_two_operands(sign_rest)?;
        if plain_gp_operand(sign_dst)? != s {
            return None;
        }
        let full_shift: u32 = if width == 'l' { 31 } else { 63 };
        if imm_value(sign_src)? != full_shift {
            return None;
        }

        // bias: `shr{l,q} $(width_bits-k), %S`
        let (bias_mnem, bias_rest) = split_head(bias)?;
        if bias_mnem != format!("shr{width}") {
            return None;
        }
        let (bias_src, bias_dst) = split_two_operands(bias_rest)?;
        if plain_gp_operand(bias_dst)? != s {
            return None;
        }
        let width_bits: u32 = if width == 'l' { 32 } else { 64 };
        let bias_shift = imm_value(bias_src)?;
        if bias_shift < 1 || bias_shift >= width_bits {
            return None;
        }
        let k = width_bits - bias_shift;

        // lea: `lea{l,q} (%X, %S), %T` — two-register SIB, zero displacement.
        let (lea_mnem, lea_rest) = split_head(lea)?;
        if lea_mnem != format!("lea{width}") {
            return None;
        }
        let (mem, t_spelling) = split_two_operands(lea_rest)?;
        let t = plain_gp_operand(t_spelling)?;
        if t == x {
            return None; // the cmov source must survive the lea
        }
        let (base, index) = sib_pair(mem)?;
        if plain_gp_operand(base)? != x || plain_gp_operand(index)? != s {
            return None;
        }

        // sar: `sar{l,q} $k, %T`
        if e.t != t || e.k != k {
            return None;
        }
        if k == 0 || k > 31 {
            return None; // 2^k-1 must be a positive signed-32 displacement
        }

        // The five lines must be pure: editable, comment-free, non-flag-reading.
        let region = [mov_i, sign_i, bias_i, lea_i, end];
        if !all_editable(infos, &region) {
            return None;
        }
        for &j in &region {
            let raw = store.get(j);
            if raw.contains('#') {
                return None;
            }
            if matches!(infos[j].kind, LineKind::Label | LineKind::Ret) {
                return None;
            }
            if reads_flags(store, infos, j) {
                return None;
            }
        }

        // Scratch liveness: no reader of %S after the lea (its last chain
        // read) until the next definition of %S.
        if scratch_live_after(store, infos, lea_i, s) {
            return None;
        }

        // Downstream flag readers before the next writer (or an unknown
        // label boundary) observe flags the rewrite does not disturb —
        // the block's trailing flags are the final sar's either way — but
        // refuse labels so no unknown entry state is assumed.
        let mut j = end + 1;
        while j < infos.len() {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }
            if matches!(infos[j].kind, LineKind::Label) {
                break;
            }
            if reads_flags(store, infos, j) {
                return None;
            }
            let text = infos[j].trimmed(store.get(j));
            if writes_flags_text(text) {
                break;
            }
            j += 1;
        }

        Some(Block {
            lines: [mov_i, sign_i, bias_i, lea_i, end],
            x_spelling: x_spelling.to_string(),
            base_spelling: base.to_string(),
            t_spelling: t_spelling.to_string(),
            width,
            k,
        })
    }

    fn apply(&self, store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
        if !all_editable(infos, &self.lines) {
            return false;
        }
        let bias: i64 = (1i64 << self.k) - 1;
        let indent = "    ";
        let new_lines = [
            format!(
                "{indent}test{} {}, {}",
                self.width, self.x_spelling, self.x_spelling
            ),
            format!(
                "{indent}lea{} {bias}({}), {}",
                self.width, self.base_spelling, self.t_spelling
            ),
            format!("{indent}cmovns {}, {}", self.x_spelling, self.t_spelling),
            // index 3 replaces the lea slot; the final sar (index 4) stays.
        ];
        // Slot swap, four lines into five slots: mov←test, sign←lea,
        // bias←cmovns, lea←the original final sar (text unchanged), and the
        // old sar slot becomes a nop — one instruction shorter overall, with
        // every interleaved debug/blank line left exactly where it was.
        let sar_text = store.get(self.lines[4]).to_string();
        let edits = [
            (self.lines[0], new_lines[0].clone()),
            (self.lines[1], new_lines[1].clone()),
            (self.lines[2], new_lines[2].clone()),
            (self.lines[3], sar_text),
        ];
        for (line, text) in edits {
            if text.trim().is_empty() {
                return false;
            }
            replace_line(store, &mut infos[line], line, text);
        }
        mark_nop(&mut infos[self.lines[4]]);
        true
    }
}

struct Sar {
    t: RegId,
    k: u32,
}

fn plain_sar(store: &LineStore, infos: &[LineInfo], i: usize) -> Option<Sar> {
    if infos[i].is_nop()
        || matches!(
            infos[i].kind,
            LineKind::Label | LineKind::Ret | LineKind::Pop { .. }
        )
    {
        return None;
    }
    let raw = store.get(i);
    if raw.contains('#') {
        return None;
    }
    let text = infos[i].trimmed(raw);
    let (mnem, rest) = split_head(text)?;
    if mnem != "sarl" && mnem != "sarq" {
        return None;
    }
    let (src, dst) = split_two_operands(rest)?;
    Some(Sar {
        t: plain_gp_operand(dst)?,
        k: imm_value(src)?,
    })
}

fn plain_text<'a>(store: &'a LineStore, infos: &[LineInfo], i: usize) -> Option<&'a str> {
    if infos[i].is_nop() {
        return None;
    }
    let raw = store.get(i);
    if raw.contains('#') {
        return None;
    }
    Some(infos[i].trimmed(raw))
}

fn split_head(text: &str) -> Option<(String, &str)> {
    text.split_once(char::is_whitespace)
        .map(|(m, r)| (m.to_string(), r))
}

fn imm_value(src: &str) -> Option<u32> {
    let rest = src.strip_prefix('$')?;
    rest.parse::<i64>().ok().and_then(|v| u32::try_from(v).ok())
}

/// `(%base,%index)` with no displacement and no scale — the two-register SIB
/// the bias-add `lea` produces.
fn sib_pair(mem: &str) -> Option<(&str, &str)> {
    let inner = mem.strip_prefix('(')?.strip_suffix(')')?;
    let mut it = inner.split(',');
    let base = it.next()?;
    let index = it.next()?;
    if it.next().is_some() {
        return None; // explicit scale: not the plain bias add
    }
    Some((base.trim(), index.trim()))
}

fn width_letter(store: &LineStore, infos: &[LineInfo], i: usize) -> Option<char> {
    let raw = store.get(i);
    let text = infos[i].trimmed(raw);
    let (mnem, _) = split_head(text)?;
    mnem.chars().last().filter(|c| *c == 'l' || *c == 'q')
}

/// First non-silence line strictly before `i` (nop/inert/blank/comment and
/// `#`-carriers are silence).
fn prev_non_silence(store: &LineStore, infos: &[LineInfo], i: usize) -> Option<usize> {
    let mut c = i;
    loop {
        if c == 0 {
            return None;
        }
        c -= 1;
        if infos[c].is_steppable() {
            continue;
        }
        let raw = store.get(c);
        if raw.trim_start().starts_with('#') || raw.trim().is_empty() {
            continue;
        }
        if crate::backend::peephole_common::inert_directive(raw) {
            continue;
        }
        return Some(c);
    }
}

/// Text-level flag-writer classification — the writer-side mirror of
/// [`reads_flags`].  Shared with `lea_add_fuse`, whose fusion deletes an
/// add's flag write and must prove nothing observed it.
pub(super) fn writes_flags_text(text: &str) -> bool {
    let m = match text.split_once(char::is_whitespace) {
        Some((m, _)) => m,
        None => text.trim(),
    };
    let v = m.strip_prefix('v').unwrap_or(m); // AVX forms also write flags
    v.starts_with("add")
        || v.starts_with("sub")
        || v.starts_with("and")
        || v.starts_with("or")
        || v.starts_with("xor")
        || v.starts_with("adc")
        || v.starts_with("sbb")
        || v.starts_with("cmp")
        || v.starts_with("test")
        || v.starts_with("neg")
        || v.starts_with("inc")
        || v.starts_with("dec")
        || v.starts_with("shl")
        || v.starts_with("shr")
        || v.starts_with("sar")
        || v.starts_with("rol")
        || v.starts_with("ror")
        || v.starts_with("rcl")
        || v.starts_with("rcr")
        || v.starts_with("mul")
        || v.starts_with("div")
        || v.starts_with("imul")
        || v.starts_with("idiv")
        || v.starts_with("bt")
}

/// True when `%S` is read after `from` before it is redefined.
fn scratch_live_after(store: &LineStore, infos: &[LineInfo], from: usize, s: RegId) -> bool {
    for j in (from + 1)..infos.len() {
        if infos[j].is_nop() {
            continue;
        }
        let raw = store.get(j);
        if raw.contains('#') {
            continue; // comments emit nothing
        }
        let text = infos[j].trimmed(raw);
        let defines_s = split_two_operands(text)
            .and_then(|(_, dst)| plain_gp_operand(dst))
            .is_some_and(|f| f == s);
        if defines_s {
            return false; // the chain's definition is what bounds the window
        }
        if operand_mentions(text, s) {
            return true;
        }
    }
    false
}

/// Does any operand of `text` name family `fam` as a plain register?
fn operand_mentions(text: &str, fam: RegId) -> bool {
    let Some((_, rest)) = text.split_once(char::is_whitespace) else {
        return false;
    };
    rest.split([',', '(', ')', '\t', ' '])
        .filter(|w| !w.is_empty())
        .any(|w| plain_gp_operand(w) == Some(fam))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run ONLY `pow2_cmov` so every assertion is about this pass.
    fn run_cmov(asm: &str) -> String {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        run(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn frame(body: &str) -> String {
        format!("f:\n.cfi_startproc\n{body}.cfi_endproc\n")
    }

    #[test]
    fn sdiv32_identity_rewrites_to_the_cmov_form() {
        let out = run_cmov(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    shrl $29, %esi\n",
            "    leal (%rdi, %rsi), %eax\n",
            "    sarl $3, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("testl %edi, %edi"), "{out}");
        assert!(out.contains("leal 7(%rdi), %eax"), "{out}");
        assert!(out.contains("cmovns %edi, %eax"), "{out}");
        assert!(out.contains("sarl $3, %eax"), "{out}");
        assert!(!out.contains("%esi"), "sign scratch retired:\n{out}");
        assert!(!out.contains("leal (%rdi, %rsi)"), "{out}");
    }

    #[test]
    fn sdiv64_identity_rewrites_to_the_cmov_form() {
        let out = run_cmov(&frame(concat!(
            "    movq %rdi, %rsi\n",
            "    sarq $63, %rsi\n",
            "    shrq $61, %rsi\n",
            "    leaq (%rdi, %rsi), %rax\n",
            "    sarq $3, %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("testq %rdi, %rdi"), "{out}");
        assert!(out.contains("leaq 7(%rdi), %rax"), "{out}");
        assert!(out.contains("cmovns %rdi, %rax"), "{out}");
        assert!(out.contains("sarq $3, %rax"), "{out}");
        assert!(!out.contains("%rsi"), "sign scratch retired:\n{out}");
    }

    #[test]
    fn a_wide_64_bit_shift_keeps_the_identity() {
        // k = 32 needs displacement 4294967295, which is not a positive
        // signed-32 displacement — the original five lines must survive.
        let out = run_cmov(&frame(concat!(
            "    movq %rdi, %rsi\n",
            "    sarq $63, %rsi\n",
            "    shrq $32, %rsi\n",
            "    leaq (%rdi, %rsi), %rax\n",
            "    sarq $32, %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("shrq $32, %rsi"), "{out}");
        assert!(out.contains("leaq (%rdi, %rsi), %rax"), "{out}");
        assert!(!out.contains("cmovns"), "{out}");
    }

    #[test]
    fn a_scratch_still_live_after_the_block_blocks_the_rewrite() {
        let out = run_cmov(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    shrl $29, %esi\n",
            "    leal (%rdi, %rsi), %eax\n",
            "    sarl $3, %eax\n",
            "    movl %esi, %r8d\n",
            "    ret\n",
        )));
        assert!(out.contains("shrl $29, %esi"), "identity kept:\n{out}");
        assert!(!out.contains("cmovns"), "{out}");
    }

    #[test]
    fn the_cmov_source_must_survive_the_lea() {
        // T == X: `leal (%rdi,%rsi), %edi` destroys the dividend the cmov
        // must select from.
        let out = run_cmov(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    shrl $29, %esi\n",
            "    leal (%rdi, %rsi), %edi\n",
            "    sarl $3, %edi\n",
            "    ret\n",
        )));
        assert!(
            out.contains("leal (%rdi, %rsi), %edi"),
            "identity kept:\n{out}"
        );
        assert!(!out.contains("cmovns"), "{out}");
    }

    #[test]
    fn a_label_inside_the_block_declines() {
        let out = run_cmov(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            ".Ljoin:\n",
            "    shrl $29, %esi\n",
            "    leal (%rdi, %rsi), %eax\n",
            "    sarl $3, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("shrl $29, %esi"), "{out}");
        assert!(!out.contains("cmovns"), "{out}");
    }

    #[test]
    fn a_comment_bearing_line_in_the_block_declines() {
        let out = run_cmov(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi  # sign\n",
            "    shrl $29, %esi\n",
            "    leal (%rdi, %rsi), %eax\n",
            "    sarl $3, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("sarl $31, %esi"), "{out}");
        assert!(!out.contains("cmovns"), "{out}");
    }

    #[test]
    fn a_forward_flag_reader_is_refused() {
        // `setl` between the block and the next flag writer observes the
        // final sar's flags today; the rewrite must not run under it.
        let out = run_cmov(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    shrl $29, %esi\n",
            "    leal (%rdi, %rsi), %eax\n",
            "    sarl $3, %eax\n",
            "    setl %cl\n",
            "    ret\n",
        )));
        assert!(out.contains("shrl $29, %esi"), "{out}");
        assert!(!out.contains("cmovns"), "{out}");
    }

    #[test]
    fn the_skip_key_disables_the_pass_in_the_pipeline() {
        use super::super::super::peephole_optimize;
        use super::super::test_skips::skip_passes;
        let body = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    shrl $29, %esi\n",
            "    leal (%rdi, %rsi), %eax\n",
            "    sarl $3, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        {
            let _skip = skip_passes(&["pow2_cmov"]);
            let out = peephole_optimize(body.to_string());
            assert!(out.contains("shrl $29, %esi"), "skipped:\n{out}");
            assert!(!out.contains("cmovns"), "skipped:\n{out}");
        }
        let out = peephole_optimize(body.to_string());
        assert!(out.contains("cmovns %edi, %eax"), "enabled:\n{out}");
    }
}
