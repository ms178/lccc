//! Terminal return-home coalescing.
//!
//! Every value-returning function ends the same way: the allocator homes the
//! returned value somewhere in the general pool, and `operand_to_rax*` pays one
//! `mov` to carry it into `%rax` immediately before `ret` — a copy on the exit
//! path of *every* such function, worth nothing to anybody, because `ret` reads
//! `%rax` anyway.  GCC never materialises it: its instruction selection aims the
//! producer chain at the return register directly.  Measured on the LLVM
//! batch-3/4 probes, that single copy is the residual difference between lccc
//! and GCC 16.2 on form after form (`ring_srem_pow2_32`, `ring_sdiv_pow2_32`,
//! the `%`-window family, plain integer returns).
//!
//! The allocator cannot take this on: `%rax` is the staging register for
//! division, i128, casts and every `operand_to_rax` consumer, so it is excluded
//! from the home pool outright, and a hint toward a register outside the pool
//! is ignored.  The text, however, shows exactly what a safe retarget looks
//! like.  For a straight-line terminal window
//!
//! ```text
//!     <root>             ; defines %D in ONE reissuable instruction
//!     <chain…>           ; contiguously redefine %D (a live-in chain)
//!     mov{l,q} %D, %rax  ; the return copy
//!     pop …              ; epilogue pops, never %rax or %D
//!     ret                 ; reads %rax
//! ```
//!
//! the pass re-issues the root with `%rax` as its destination, renames the
//! chain's `%D` to `%rax`, and deletes the return copy:
//!
//! ```text
//!     before                         after
//!     movl  %edi, %esi               movl  %edi, %esi
//!     sarl  $31, %esi                sarl  $31, %esi
//!     shrl  $29, %esi                shrl  $29, %esi
//!     addl  %esi, %edi               leal  (%rdi,%rsi), %eax   ← 3-address root
//!     sarl  $3, %edi                 sarl  $3, %eax
//!     movl  %edi, %eax               ret
//!     ret
//! ```
//!
//! # When it is legal
//!
//! 1. **The copy is the last value instruction before `ret`** — only debug
//!    attribution, inert directives, blank lines and epilogue `pop`s may sit
//!    between them, and no pop may name `%rax` or the chain family (a pop of
//!    either would consume state this rewrite moves or invalidates).
//! 2. **The copy is `mov{l,q} %S, %rax` with plain GP operands**, `%S ≠ %rax`,
//!    `%S ∉ {%rsp, %rbp}`: it is the return-materialisation move, and `%S`
//!    (call it `%D`) is the chain family.
//! 3. **The chain is the contiguous run of `%D`-defining instructions
//!    immediately above the copy**, straight-line all the way: the walk stops
//!    at the first instruction that does not define `%D` (a label, call, store
//!    of the intermediate, pushed value, pinned or volatile line), and an
//!    empty run means the value has an earlier, interleaved definition and the
//!    pattern is abandoned rather than guessed at.
//! 4. **No implicit register traffic in the window.** Every collected line
//!    must score zero on the central [`implicit_reg_refs`] oracle — that is
//!    what keeps `mul`/`div`/`cqto`/`cltq`/`syscall`/`cpuid` (implicit
//!    `%rax:%rdx` readers and writers) from ever entering a chain they would
//!    corrupt.  One-operand forms do not even parse as definitions, so the
//!    common cases stop the walk before this check is needed.
//! 5. **No flag reader in the window.** Re-issuing an `add`/`sub` root as
//!    `lea` removes its flag write; a `setcc`/`cmov`/`adc`/`sbb` anywhere in
//!    the chain would then consume stale flags.  Flag *writers* are fine — the
//!    window ends at `ret`, and the ABI does not carry flags out.
//! 6. **The root re-issues in one instruction with a fresh `%rax`
//!    destination**: `mov` (register or immediate source), `add` (register or
//!    immediate source → `lea`, the only three-address form in the GP ALU —
//!    and `lea` also replaces the flag write with nothing, which is why rule 5
//!    exists), `sub` by an immediate (→ `lea` with the negated displacement),
//!    an existing `lea`, or a zero/sign-extending `movz*`/`movs*`.  Everything
//!    else (`and`, `or`, `xor`, variable shifts, `imul`, `neg`, memory-sourced
//!    `add`, …) would need a staging `mov` — the very copy being removed — so
//!    it aborts.
//! 7. **No middle chain line mentions `%rax` explicitly.** After the rename its
//!    destination *is* `%rax`, so a pre-existing `%rax` source would silently
//!    alias the chain value against whatever `%rax` held before (e.g.
//!    `subl %eax, %edi` must not become `subl %eax, %eax`).  Roots are
//!    exempt: `lea`/`mov` read their sources before writing the destination,
//!    so a root source of `%rax` still observes the old value.
//! 8. Frame families `%rsp`/`%rbp` are never chain families, pinned and
//!    volatile lines abort the pattern before any edit (the multi-line commit
//!    gate [`all_editable`] is checked first — a half-applied rewrite is how
//!    `fold_movslq_relay` once read an undefined register).
//!
//! The pass runs in the terminal phase of the pipeline (after epilogue
//! cross-jumping and the final dead-write sweep, before `return_bool`) and is
//! switchable with `CCC_PEEPHOLE_SKIP=return_home` for bisection.
//!
//! [`implicit_reg_refs`]: super::types::implicit_reg_refs
//! [`all_editable`]: super::types::all_editable

use super::super::types::*;
use super::helpers::replace_reg_family;
use super::identical_blocks::reads_flags;
use super::relay_and_lea::{is_relayable_family, plain_gp_operand, split_two_operands};

/// Terminal return-copy coalescing; returns `true` when a copy was retired.
pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    for r in 0..infos.len() {
        if infos[r].is_nop() || infos[r].kind != LineKind::Ret {
            continue;
        }
        if try_fold_at_ret(store, infos, r) {
            changed = true;
        }
    }
    changed
}

/// Lines a terminal window may span without reading or writing anything: debug
/// attribution, inert directives, blanks and already-deleted nops.  Raw `#`
/// comments are annotations the assembler drops; they are stepped over but
/// never deleted (the return copy itself must parse as an instruction).
fn terminal_silence(store: &LineStore, info: &LineInfo, i: usize) -> bool {
    if info.is_steppable() || info.kind == LineKind::Empty {
        return true;
    }
    let t = info.trimmed(store.get(i));
    t.trim_start().starts_with('#')
}

/// The width tier of a register spelling for family `fam` (0 = 64-bit,
/// 1 = 32-bit, 2 = 16-bit, 3 = 8-bit).  High-byte aliases (`%ah`) return
/// `None`: they are not canonical-low spellings and no rewrite here should
/// ever meet one.
fn width_tier(name: &str, fam: RegId) -> Option<usize> {
    let f = fam as usize;
    (0..4).position(|t| name == REG_NAMES[t][f])
}

/// Family-0 (`%rax`) spelling at the given width tier.
fn rax_alias(tier: usize) -> &'static str {
    REG_NAMES[tier][0]
}

/// A `$imm` operand whose text is a plain decimal integer.
fn imm_disp(text: &str) -> Option<i64> {
    let body = text.strip_prefix('$')?;
    body.parse::<i64>().ok()
}

/// Re-issue `root` with `%rax` as the destination, or `None` when the root
/// needs more than the one instruction the return copy is worth.  `text` is the
/// root line without any trailing comment; `d_fam` is the chain family.
fn reissue_root(text: &str, d_fam: RegId, force_32: bool) -> Option<String> {
    let (mnem, rest) = text.split_once(char::is_whitespace)?;
    let (src, dst) = split_two_operands(rest)?;
    if plain_gp_operand(dst)? != d_fam {
        return None;
    }
    let natural_tier = width_tier(dst, d_fam)?;
    // Width-down re-issue: a 32-bit truncating return copy drops bits 32..63,
    // so a single-instruction chain re-issues at the *copy's* width — every
    // whitelisted root family (mov, lea, add, sub-immediate) commutes with
    // low-32 truncation, making `leal`/`movl`-at-%eax exactly equal to the
    // original 64-bit compute followed by the copy.
    let dst_tier = if force_32 { 1 } else { natural_tier };
    let d64 = REG_NAMES[0][d_fam as usize];

    // Zero/sign extensions: three-address already, keep the operation and
    // the source verbatim (register, immediate or memory — all read before
    // the destination is written), and only move the destination to %rax.
    if matches!(
        mnem,
        "movzbl" | "movzwl" | "movsbq" | "movswq" | "movslq" | "movsbl" | "movsbw" | "movswl"
    ) {
        // Under width-down a `movs…q` becomes its 32-bit sibling: its result
        // truncated to 32 bits is exactly the sign-extension one stage
        // narrower (`movslq` is the identity at 32 bits — spelled `movl`).
        let out_mnem: String = if force_32 && mnem.ends_with('q') {
            if mnem == "movslq" {
                "movl".to_string()
            } else {
                format!("{}l", &mnem[..mnem.len() - 1])
            }
        } else {
            mnem.to_string()
        };
        return Some(format!("    {out_mnem} {src}, {}", rax_alias(dst_tier)));
    }

    // Existing lea: identical address computation, fresh destination.
    // The address may read %rax or %D — both are observed before the write.
    if matches!(mnem, "leal" | "leaq") {
        let out_mnem = if force_32 { "leal" } else { mnem };
        return Some(format!("    {out_mnem} {src}, {}", rax_alias(dst_tier)));
    }

    // Plain move: same source (register or immediate), destination in %rax.
    if matches!(mnem, "movb" | "movw" | "movl" | "movq") {
        let s_fam = match plain_gp_operand(src) {
            Some(f) => {
                // A copy *out of* %rax would collapse into a self-move; the
                // chain value must come from somewhere else.
                if f == 0 {
                    return None;
                }
                Some(f)
            }
            None if src.starts_with('$') => {
                // A symbolic 64-bit immediate can use a 64-bit relocation;
                // retargeting it as `movl` would require a truncating 32-bit
                // relocation and can turn a linkable object into overflow.
                // Numeric immediates have well-defined low-32-bit semantics.
                if force_32 && mnem == "movq" && imm_disp(src).is_none() {
                    return None;
                }
                None
            }
            None => return None, // memory or other opaque source
        };
        // Width-down: `movq %rN, %D` truncated is `movl %rNd, %eax` — the
        // source's low 32 bits are what the copy would have kept.
        let (out_mnem, out_src) = match (force_32, s_fam) {
            (true, Some(f)) if mnem == "movq" && width_tier(src, f) == Some(0) => {
                ("movl", REG_NAMES[1][f as usize].to_string())
            }
            (true, _) if mnem == "movq" => ("movl", src.to_string()),
            _ => (mnem, src.to_string()),
        };
        return Some(format!("    {out_mnem} {out_src}, {}", rax_alias(dst_tier)));
    }

    // Trailing-zero count: a three-address bit-scan written fresh from the
    // source and defined for every input (tzcnt answers the operand width
    // when the source is 0), so relocating its destination to the return
    // alias is the same operation -- `tzcntl %edi, %esi; movl %esi, %eax`
    // becomes `tzcntl %edi, %eax`. `bsf` stays out: its destination is
    // architecturally undefined for a zero source, so the destination
    // register would be observable; `lzcnt`/`bsr` have no demand-gate
    // evidence yet.
    if matches!(mnem, "tzcntl" | "tzcntq") {
        // Width-down (`force_32`) may relabel only the 32-bit count: a
        // `tzcntq` answer is NOT the low 32 bits of itself (when the low
        // half is zero the count lives in the upper half), so `tzcntq`
        // declines instead of becoming a `tzcntl` of the low half. A
        // `tzcntl` root already answers in [0, 32], which fits every
        // truncating copy. Source spellings must agree with the mnemonic;
        // native codegen always produces those, so a disagreement is a
        // fail-closed decline rather than a rewrite.
        if mnem == "tzcntq" {
            if force_32 {
                return None;
            }
            if plain_gp_operand(src).is_some_and(|f| width_tier(src, f) == Some(1)) {
                return None;
            }
        } else if plain_gp_operand(src).is_some_and(|f| width_tier(src, f) == Some(0)) {
            return None;
        }
        let out_mnem = if force_32 { "tzcntl" } else { mnem };
        return Some(format!("    {out_mnem} {src}, {}", rax_alias(dst_tier)));
    }

    // add: three-address lea.  Register sources join as the SIB index, the
    // destination family as the base; an immediate becomes the displacement.
    // lea_size law: a narrow result uses `leal` (a `leaw` would be a
    // partial-register write that merges with the destination's old value).
    if matches!(mnem, "addb" | "addw" | "addl" | "addq") {
        let lea_mnem = if dst_tier == 0 { "leaq" } else { "leal" };
        let lea_dst = rax_alias(if dst_tier == 0 { 0 } else { 1 });
        if let Some(s_fam) = plain_gp_operand(src) {
            if !is_relayable_family(s_fam) {
                return None;
            }
            let s64 = REG_NAMES[0][s_fam as usize];
            return Some(format!("    {lea_mnem} ({d64},{s64}), {lea_dst}"));
        }
        let disp = imm_disp(src)?;
        // The displacement must fit the lea encoding the same way the add's
        // imm32 did; reject anything that does not round-trip through i32.
        i32::try_from(disp).ok()?;
        return Some(format!("    {lea_mnem} {disp}({d64}), {lea_dst}"));
    }

    // sub by an immediate: lea with the negated displacement.  A register
    // sub has no three-address form — mov+sub costs exactly what the copy
    // costs — so it aborts here, like every other non-lea-able ALU root.
    if matches!(mnem, "subb" | "subw" | "subl" | "subq") {
        let lea_mnem = if dst_tier == 0 { "leaq" } else { "leal" };
        let lea_dst = rax_alias(if dst_tier == 0 { 0 } else { 1 });
        let disp = imm_disp(src)?;
        let neg = i64::from(i32::try_from(disp).ok()?.checked_neg()?);
        return Some(format!("    {lea_mnem} {neg}({d64}), {lea_dst}"));
    }

    None
}

/// Try the rewrite for the `ret` at index `r`.
fn try_fold_at_ret(store: &mut LineStore, infos: &mut [LineInfo], r: usize) -> bool {
    // ── 1. suffix window: [exit copy … ret) — silence and safe pops only ────
    let mut pops: Vec<(usize, RegId)> = Vec::new();
    let mut exit = r;
    loop {
        if exit == 0 {
            return false;
        }
        let prev = exit - 1;
        if terminal_silence(store, &infos[prev], prev) {
            exit = prev;
            continue;
        }
        if let LineKind::Pop { reg } = infos[prev].kind {
            pops.push((prev, reg));
            exit = prev;
            continue;
        }
        // First real instruction below the skipped tail: that is the copy.
        exit = prev;
        break;
    }
    if exit == r {
        return false; // nothing between the ret and its window start
    }

    // ── 2. the return copy itself ────────────────────────────────────────────
    if !all_editable(infos, &[exit]) {
        return false;
    }
    let raw = store.get(exit);
    if raw.contains('#') {
        return false; // comment-bearing copy: keep the rewrite simple
    }
    let t = infos[exit].trimmed(raw);
    let (mnem, rest) = match t.split_once(char::is_whitespace) {
        Some(pair) => pair,
        None => return false,
    };
    if !matches!(mnem, "movl" | "movq") {
        return false;
    }
    let (src, dst) = match split_two_operands(rest) {
        Some(pair) => pair,
        None => return false,
    };
    // The destination must be the return register itself: this is the
    // materialisation move, not an arbitrary copy that happens to precede ret.
    if plain_gp_operand(dst) != Some(0) {
        return false;
    }
    let Some(d_fam) = plain_gp_operand(src) else {
        return false; // immediate, memory or vector source: nothing to home
    };
    if d_fam == 0 || !is_relayable_family(d_fam) {
        return false;
    }
    // A pop of %rax would discard the very value the rewrite plants there; a
    // pop of %D would consume chain state after the root stopped writing it.
    for &(_, reg) in &pops {
        if reg == 0 || reg == d_fam {
            return false;
        }
    }
    let d_mask = 1u16 << d_fam;

    // ── 3. walk the contiguous chain of %D definitions down from the copy ───
    let mut chain: Vec<usize> = Vec::new();
    let mut c = exit;
    loop {
        if c == 0 {
            return false;
        }
        c -= 1;
        if terminal_silence(store, &infos[c], c) {
            continue; // debug attribution and blanks inside the chain
        }
        // Classify by text: a definition is an instruction whose last
        // top-level operand is the plain register %D.  Anything else — a
        // store of the intermediate, a label, a call, a push, `cqto`, a
        // one-operand instruction — ends the walk right here.
        let line = store.get(c);
        let line = infos[c].trimmed(line);
        let line = match line.split_once('#') {
            Some((before, _)) => before.trim_end(),
            None => line,
        };
        let mnem = line
            .split_once(char::is_whitespace)
            .map(|(m, _)| m)
            .unwrap_or("");
        // Flag-only two-operand instructions (`cmp`, `test`, `bt`) name a
        // register but never write it: treating one as a definition would
        // rename its operand to read the accumulator instead.
        let flag_only =
            mnem.starts_with("cmp") || mnem.starts_with("test") || mnem.starts_with("bt");
        let is_def = split_two_operands(line)
            .and_then(|(_, dst)| plain_gp_operand(dst))
            .is_some_and(|f| f == d_fam)
            && !flag_only;
        if !is_def {
            break;
        }
        if infos[c].pinned || infos[c].volatile_access {
            break; // never collected: the window simply ends above it
        }
        if implicit_reg_refs(line.as_bytes()) != 0 {
            break; // implicit traffic: end the walk conservatively
        }
        // Width law: only full 32/64-bit writers join a window.  A byte/word
        // ALU form merges into the register's old upper bytes — bytes that
        // move with the *register* when the rename retargets the chain into
        // the accumulator — and `movz`/`movs` with a clipped destination
        // spelling is equally suspect.  Both end the walk conservatively.
        let width_ok = if mnem.starts_with("movz") || mnem.starts_with("movs") {
            // Extenders: the destination must be spelled at 32/64 bits so the
            // rename's accumulator spelling is the same full define.
            split_two_operands(line)
                .and_then(|(_, dst)| plain_gp_operand(dst).and_then(|f| width_tier(dst, f)))
                .is_some_and(|t| t <= 1)
        } else {
            mnem.ends_with('l') || mnem.ends_with('q')
        };
        if !width_ok {
            break;
        }
        if reads_flags(store, infos, c) {
            return false; // a flag reader inside the window would go stale
        }
        chain.push(c);
        if chain.len() > 64 {
            return false; // pathological: refuse rather than scan a function
        }
    }
    // `chain[0]` is the instruction nearest the copy; the root is the last
    // collected one, i.e. the bottom of the run. An empty chain (interleaved
    // definition: the value is not a tail chain) declines without a panic.
    let Some(&root) = chain.last() else {
        return false;
    };
    // Truncation law: `movl %D, %eax` drops bits 32..63 of the chain value.
    // A ONE-instruction chain answers with a width-down re-issue of that
    // instruction (the whitelisted root families all commute with low-32
    // truncation).  In a longer chain the last writer is a renamed middle
    // that keeps its own width, so only when that middle writes 64 bits does
    // the dropped upper half become observable — the same contract
    // `relay_and_lea::chain_rename` enforces — and the window declines.
    let force_32 = mnem == "movl" && chain.len() == 1;
    if mnem == "movl" && chain.len() > 1 {
        let first = infos[chain[0]].trimmed(store.get(chain[0]));
        let first_is_64 = first
            .split_once(char::is_whitespace)
            .is_some_and(|(m, _)| m.ends_with('q'));
        if first_is_64 {
            return false;
        }
    }
    let middles = &chain[..chain.len() - 1];

    // ── 4. root re-issue ─────────────────────────────────────────────────────
    if !all_editable(infos, &[root]) {
        return false;
    }
    let root_raw = store.get(root);
    if root_raw.contains('#') {
        return false;
    }
    let root_text = infos[root].trimmed(root_raw).to_string();
    let Some(new_root) = reissue_root(&root_text, d_fam, force_32) else {
        return false;
    };
    // A root that *writes* flags cannot become a lea unless nothing in the
    // chain reads them; the local `reads_flags` veto above already refused
    // readers, and roots that read flags (adc/sbb/setcc) never re-issue.

    // Middle lines must not mention %rax before the rename: after it their
    // destination *is* %rax, and an explicit %rax source would alias the
    // chain value against the old accumulator contents.
    for &m in middles {
        if infos[m].reg_refs & 1 != 0 {
            return false;
        }
    }

    // ── 5. commit: all gates have passed, apply every edit atomically ───────
    let mut editables: Vec<usize> = middles.to_vec();
    editables.push(root);
    editables.push(exit);
    if !all_editable(infos, &editables) {
        return false;
    }
    replace_line(store, &mut infos[root], root, new_root);
    for &m in middles {
        if infos[m].reg_refs & d_mask == 0 {
            continue;
        }
        let before = infos[m].trimmed(store.get(m)).to_string();
        let after = replace_reg_family(&before, d_fam, 0);
        if after != before {
            replace_line(store, &mut infos[m], m, format!("    {after}"));
        }
    }
    mark_nop(&mut infos[exit]);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run ONLY `return_home`, so every assertion is about this pass rather
    /// than about whatever the rest of the pipeline would do to the same
    /// input (the established isolation pattern: `dead_writes` tests).
    fn run_home(asm: &str) -> String {
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
    fn signed_pow2_quotient_computes_straight_into_eax() {
        // `int f(int x) { return x / 8; }` as the emitter produces it today:
        // sign materialisation, bias add, shift, return copy.
        let out = run_home(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    shrl $29, %esi\n",
            "    addl %esi, %edi\n",
            "    sarl $3, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(
            !out.contains("movl %edi, %eax"),
            "return copy retired:\n{out}"
        );
        assert!(
            out.contains("leal (%rdi,%rsi), %eax"),
            "3-address root:\n{out}"
        );
        assert!(out.contains("sarl $3, %eax"), "chain renamed:\n{out}");
        // The sign materialisation above the root stays untouched.
        assert!(out.contains("movl %edi, %esi"), "{out}");
        assert!(out.contains("shrl $29, %esi"), "{out}");
    }

    #[test]
    fn remainder_chain_keeps_its_middle_and_drops_only_the_copy() {
        // `int f(int x) { return x % 4; }` — and/sub middle chain.
        let out = run_home(&frame(concat!(
            "    movl %edi, %esi\n",
            "    sarl $31, %esi\n",
            "    andl $3, %esi\n",
            "    addl %esi, %edi\n",
            "    andl $3, %edi\n",
            "    subl %esi, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(!out.contains("movl %edi, %eax"), "{out}");
        assert!(out.contains("leal (%rdi,%rsi), %eax"), "{out}");
        assert!(out.contains("andl $3, %eax"), "middle renamed:\n{out}");
        assert!(out.contains("subl %esi, %eax"), "{out}");
        assert!(out.contains("andl $3, %esi"), "sign mask untouched:\n{out}");
    }

    #[test]
    fn mov_root_fuses_into_the_return_register() {
        let out = run_home(&frame(concat!(
            "    movl %esi, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(!out.contains("movl %edi, %eax"), "{out}");
        assert!(out.contains("movl %esi, %eax"), "root re-issued:\n{out}");
    }

    #[test]
    fn sub_imm_root_becomes_a_lea_with_negated_displacement() {
        let out = run_home(&frame(concat!(
            "    subl $7, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(!out.contains("movl %edi, %eax"), "{out}");
        assert!(out.contains("leal -7(%rdi), %eax"), "{out}");
    }

    #[test]
    fn existing_lea_root_only_changes_destination() {
        let out = run_home(&frame(concat!(
            "    leal 1(%rdi), %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(!out.contains("movl %edi, %eax"), "{out}");
        assert!(out.contains("leal 1(%rdi), %eax"), "{out}");
    }

    #[test]
    fn zero_extending_root_fuses() {
        // The narrow-representation shape: `movzwl` builds the value, the copy
        // carries it home.  The extension moves to %eax directly.
        let out = run_home(&frame(concat!(
            "    movzwl %si, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(!out.contains("movl %edi, %eax"), "{out}");
        assert!(out.contains("movzwl %si, %eax"), "{out}");
    }

    #[test]
    fn flag_reader_inside_the_window_vetoes_the_rewrite() {
        // `cmov` is the flag reader that can actually join a window: it
        // defines %D as its last operand *and* consumes the flags the root's
        // add produced.  Re-issuing the root as lea would leave the cmov
        // reading whatever flags the comparison before the window wrote.
        let out = run_home(&frame(concat!(
            "    cmpl $3, %esi\n",
            "    addl %esi, %edi\n",
            "    cmovl %ecx, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("addl %esi, %edi"), "root untouched:\n{out}");
        assert!(out.contains("cmovl %ecx, %edi"), "flag reader kept:\n{out}");
        assert!(out.contains("movl %edi, %eax"), "copy kept:\n{out}");
    }

    #[test]
    fn flag_reader_below_the_root_is_outside_the_window_and_the_fold_fires() {
        // One-operand instructions do not parse as %D definitions, so `setl`
        // ends the walk and lands *below* the root: the window never contains
        // it, the flags it reads come from the untouched `add` below it, and
        // the extension root still fuses into the return register.
        let out = run_home(&frame(concat!(
            "    addl %esi, %edi\n",
            "    setl %dil\n",
            "    movzbl %dil, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(!out.contains("movl %edi, %eax"), "copy retired:\n{out}");
        assert!(out.contains("setl %dil"), "setcc survives:\n{out}");
        assert!(out.contains("movzbl %dil, %eax"), "root re-issued:\n{out}");
        assert!(
            out.contains("addl %esi, %edi"),
            "below-window add intact:\n{out}"
        );
    }

    #[test]
    fn a_store_of_the_intermediate_stops_the_walk() {
        // The chain is not contiguous: the value is spilled between the root
        // and the copy, so there is no tail to rename.
        let out = run_home(&frame(concat!(
            "    addl %esi, %edi\n",
            "    movl %edi, -4(%rbp)\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("addl %esi, %edi"), "{out}");
        assert!(out.contains("movl %edi, %eax"), "{out}");
    }

    #[test]
    fn implicit_rax_traffic_stops_the_walk() {
        // `cltd`/`idiv` implicitly read and write %rax:%rdx; the one-operand
        // form does not parse as a %D definition, the walk stops, and the
        // remaining run never reaches a re-issuable root.
        let out = run_home(&frame(concat!(
            "    movl %esi, %eax\n",
            "    cltd\n",
            "    idivl %edi\n",
            "    movl %eax, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("idivl %edi"), "{out}");
        assert!(out.contains("movl %edi, %eax"), "copy kept:\n{out}");
    }

    #[test]
    fn pop_of_the_return_register_vetoes_the_rewrite() {
        // The epilogue restores %rax after the copy: planting the value there
        // would leave the pop as the last writer before ret.
        let out = run_home(&frame(concat!(
            "    pushq %rax\n",
            "    movl %esi, %edi\n",
            "    movl %edi, %eax\n",
            "    popq %rax\n",
            "    ret\n",
        )));
        assert!(out.contains("movl %edi, %eax"), "{out}");
        assert!(out.contains("popq %rax"), "{out}");
    }

    #[test]
    fn safe_callee_save_pop_between_copy_and_ret_is_spanned() {
        let out = run_home(&frame(concat!(
            "    pushq %rbx\n",
            "    movl %esi, %edi\n",
            "    movl %edi, %eax\n",
            "    popq %rbx\n",
            "    ret\n",
        )));
        assert!(!out.contains("movl %edi, %eax"), "{out}");
        assert!(out.contains("movl %esi, %eax"), "{out}");
        assert!(out.contains("popq %rbx"), "{out}");
    }

    #[test]
    fn middle_chain_line_may_not_mention_rax() {
        // `subl %eax, %edi` reads the *old* %eax; renaming the destination to
        // %eax would make both operands the chain value.
        let out = run_home(&frame(concat!(
            "    addl %esi, %edi\n",
            "    subl %eax, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("movl %edi, %eax"), "copy kept:\n{out}");
        assert!(out.contains("subl %eax, %edi"), "{out}");
    }

    #[test]
    fn sixty_four_bit_returns_use_the_q_forms() {
        let out = run_home(&frame(concat!(
            "    addq %rsi, %rdi\n",
            "    movq %rdi, %rax\n",
            "    ret\n",
        )));
        assert!(!out.contains("movq %rdi, %rax"), "{out}");
        assert!(out.contains("leaq (%rdi,%rsi), %rax"), "{out}");
    }

    #[test]
    fn two_half_returns_are_left_alone() {
        // 128-bit / two-eightbyte returns materialise %rax *and* %rdx; the
        // first copy is not adjacent to ret, so nothing fires.
        let out = run_home(&frame(concat!(
            "    movl %edi, %eax\n",
            "    movl %esi, %edx\n",
            "    ret\n",
        )));
        assert!(out.contains("movl %edi, %eax"), "{out}");
        assert!(out.contains("movl %esi, %edx"), "{out}");
    }

    #[test]
    fn memory_sourced_add_root_is_refused() {
        // lea could express `disp(base,index)`, but the v1 root parser keeps
        // the rewrite to register/immediate sources where the addressing
        // semantics are trivially identical.
        let out = run_home(&frame(concat!(
            "    addl (%rbx), %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("addl (%rbx), %edi"), "{out}");
        assert!(out.contains("movl %edi, %eax"), "{out}");
    }

    #[test]
    fn a_label_between_copy_and_ret_vetoes_the_rewrite() {
        // Multiple returns layout: the copy feeds a block boundary, not this
        // ret's window.
        let out = run_home(&frame(concat!(
            "    movl %esi, %edi\n",
            "    movl %edi, %eax\n",
            ".Ldone:\n",
            "    ret\n",
        )));
        assert!(out.contains("movl %edi, %eax"), "{out}");
    }

    #[test]
    fn pinned_root_refuses_before_any_edit() {
        // The multi-line commit gate must fire before the first replacement:
        // a pinned line (parameter pre-store, volatile access) anywhere in the
        // collected window aborts the whole rewrite, never half of it.
        let asm = frame(concat!(
            "    movl %esi, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        ));
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let root = (0..store.len())
            .find(|&i| store.get(i).trim() == "movl %esi, %edi")
            .expect("root line present");
        infos[root].pinned = true;
        assert!(!run(&mut store, &mut infos), "pinned root refused");
        let joined: String = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("movl %esi, %edi"), "{joined}");
        assert!(joined.contains("movl %edi, %eax"), "{joined}");
    }

    #[test]
    fn a_64_bit_last_write_under_a_32_bit_return_copy_declines() {
        // The `chain_rename` truncation contract: `movl` must leave the
        // accumulator's bits 32..63 zero, so a 64-bit chain cannot be
        // re-issued straight into %eax under it.
        let out = run_home(&frame(concat!(
            "    leaq (%rcx, %rcx, 2), %r11\n",
            "    addq %rbx, %r11\n",
            "    movl %r11d, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("movl %r11d, %eax"), "copy kept:\n{out}");
        assert!(
            !out.contains("addq %rbx, %rax"),
            "no 64-bit re-issue:\n{out}"
        );
    }

    #[test]
    fn flag_only_two_operand_instructions_do_not_join_the_window() {
        // `cmpl $3, %edi` names %edi as its last operand but writes only
        // flags.  Mistaking it for a definition would rename the compare to
        // `cmpl $3, %eax` — against the accumulator's stale value.
        let out = run_home(&frame(concat!(
            "    movl %esi, %edi\n",
            "    cmpl $3, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("cmpl $3, %edi"), "compare untouched:\n{out}");
        assert!(out.contains("movl %edi, %eax"), "copy kept:\n{out}");
    }

    #[test]
    fn a_clipped_zext_destination_ends_the_walk() {
        // `movzbl %al, %bl` writes a byte-destination: the widening family
        // rename cannot spell that destination in the accumulator's widths,
        // so the walk stops and the copy stays.
        let out = run_home(&frame(concat!(
            "    movzbl %al, %bl\n",
            "    movq %rbx, %rax\n",
            "    ret\n",
        )));
        assert!(
            out.contains("movzbl %al, %bl"),
            "extender untouched:\n{out}"
        );
        assert!(out.contains("movq %rbx, %rax"), "copy kept:\n{out}");
    }

    #[test]
    fn a_single_64_bit_add_root_widths_down_under_a_32_bit_copy() {
        // The copy truncates to 32 bits, so the re-issue answers at 32 bits:
        // `leal` wraps exactly like `addq` followed by the truncation.
        let out = run_home(&frame(concat!(
            "    addq %r11, %r9\n",
            "    movl %r9d, %eax\n",
            "    ret\n",
        )));
        assert!(!out.contains("movl %r9d, %eax"), "copy retired:\n{out}");
        assert!(
            out.contains("leal (%r9,%r11), %eax"),
            "width-down lea:\n{out}"
        );
        assert!(!out.contains("leaq"), "no 64-bit re-issue:\n{out}");
    }

    #[test]
    fn a_single_64_bit_move_root_widths_down_under_a_32_bit_copy() {
        let out = run_home(&frame(concat!(
            "    movq %r11, %r9\n",
            "    movl %r9d, %eax\n",
            "    ret\n",
        )));
        assert!(!out.contains("movl %r9d, %eax"), "copy retired:\n{out}");
        assert!(out.contains("movl %r11d, %eax"), "width-down move:\n{out}");
    }

    #[test]
    fn a_carry_reader_in_the_chain_keeps_the_flag_writing_root() {
        // `rcl` consumes CF from the add root. Reissuing that root as LEA
        // would leave stale CF, so this live-in chain is not retargetable.
        let out = run_home(&frame(concat!(
            "    addl %ecx, %edi\n",
            "    rcll $1, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("addl %ecx, %edi"), "root kept:\\n{out}");
        assert!(out.contains("movl %edi, %eax"), "copy kept:\\n{out}");
    }

    #[test]
    fn a_symbolic_64_bit_immediate_is_not_width_down_relocated() {
        // A full-width symbol relocation cannot always be represented by a
        // 32-bit relocation. Keep the copy rather than silently requiring a
        // truncating relocation at link time.
        let out = run_home(&frame(concat!(
            "    movq $global_object, %r9\n",
            "    movl %r9d, %eax\n",
            "    ret\n",
        )));
        assert!(
            out.contains("movq $global_object, %r9"),
            "root kept:\n{out}"
        );
        assert!(out.contains("movl %r9d, %eax"), "copy kept:\n{out}");
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
            "    addl %esi, %edi\n",
            "    sarl $3, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        // Pin THIS pass alone: `pow2_cmov` legitimately rewrites the lea
        // form the enabled run produces, so keep it out of the evidence.
        let _late = skip_passes(&["pow2_cmov"]);
        {
            let _guard = skip_passes(&["return_home"]);
            let out = peephole_optimize(body.to_string());
            assert!(out.contains("movl %edi, %eax"), "skipped:\n{out}");
            assert!(out.contains("addl %esi, %edi"), "skipped:\n{out}");
        }
        // And with the pass enabled the same pipeline input loses the copy —
        // proving the skip key, not hope, is what gated the rewrite above.
        let out = peephole_optimize(body.to_string());
        assert!(!out.contains("movl %edi, %eax"), "enabled:\n{out}");
        assert!(out.contains("leal (%rdi,%rsi), %eax"), "enabled:\n{out}");
    }
}
