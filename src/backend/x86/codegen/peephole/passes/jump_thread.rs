//! Bypass empty direct-jump trampolines left by allocation and dead copies.
//!
//! Retarget edges, never reorder code. The only deletion is a relay the
//! retargeting orphaned: labels no line of the file mentions (so no computed
//! goto, jump table or address comparison can observe them), entered by no
//! fallthrough, holding one direct `jmp`. The skipped window
//! contains only labels, comments, deleted lines and one unconditional jump:
//! registers, memory and flags are unchanged. Keep function scope, inline asm,
//! pinned lines, semantic directives (including CFI and alignment padding),
//! short-only counter branches and cyclic/overlong chains fail-closed.
use super::super::types::*;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

pub(super) fn local_label(s: &str) -> Option<u32> {
    let n = s.strip_prefix(".LBB")?;
    // Canonical spelling prevents distinct assembler symbols (e.g. 01 and 1)
    // from collapsing to one numeric key; oversized labels fail closed.
    if n.is_empty() || (n.len() > 1 && n.starts_with('0')) {
        return None;
    }
    n.bytes()
        .all(|c| c.is_ascii_digit())
        .then(|| n.parse().ok())
        .flatten()
}

pub(super) fn label_text(s: &str) -> &str {
    s.split('#')
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches(':')
}

fn branch(s: &str) -> Option<(&str, u32)> {
    if !s.starts_with('j') {
        return None;
    }
    let mut fields = s.split('#').next()?.split_whitespace();
    let mnemonic = fields.next()?;
    let target = fields.next()?;
    // A bounded stack buffer reuses the shared recognizer without a heap
    // allocation or changing its legacy literal-space contract.
    let mut canonical = [b' '; 8];
    if mnemonic.len() >= canonical.len() {
        return None;
    }
    canonical[..mnemonic.len()].copy_from_slice(mnemonic.as_bytes());
    let opcode = std::str::from_utf8(&canonical).ok()?;
    if (mnemonic != "jmp" && !is_conditional_jump(opcode))
        || fields.next().is_some()
        || matches!(mnemonic, "jcxz" | "jecxz" | "jrcxz")
    {
        return None;
    }
    Some((mnemonic, local_label(target)?))
}

fn thread_function(store: &mut LineStore, infos: &mut [LineInfo], lo: usize, hi: usize) -> bool {
    let mut labels = FxHashSet::default();
    for i in lo..hi {
        if infos[i].kind == LineKind::Label && !infos[i].pinned {
            let label = label_text(infos[i].trimmed(store.get(i)));
            if let Some(label) = local_label(label)
                && !labels.insert(label)
            {
                return false; // Ambiguous label: never guess which definition.
            }
        }
    }
    let mut redirects = FxHashMap::default();
    let mut next: Option<u32> = None;
    for i in (lo..hi).rev() {
        let info = infos[i];
        let s = info.trimmed(store.get(i));
        if info.pinned || info.kind == LineKind::InlineAsm {
            next = None;
            continue;
        }
        match info.kind {
            LineKind::Nop | LineKind::Empty => {}
            // Human comments emit no bytes. Keep assembler quarantine and
            // compiler-owned liveness/ABI markers opaque to this CFG rewrite.
            _ if inert_comment(s) => {}
            LineKind::Label => {
                if let Some(label) = local_label(label_text(s)).filter(|l| labels.contains(l)) {
                    if let Some(target) = next {
                        redirects.insert(label, target);
                    }
                } else {
                    next = None;
                }
            }
            LineKind::Directive if info.is_steppable() && is_debug_marker_directive(s) => {}
            _ => {
                next = branch(s).and_then(|(op, target)| {
                    (op == "jmp" && labels.contains(&target)).then_some(target)
                });
            }
        }
    }
    let mut resolved = FxHashMap::default();
    for (label, first) in &redirects {
        let mut seen = FxHashSet::default();
        seen.insert(*label);
        let mut target = *first;
        for _ in 0..64 {
            if !seen.insert(target) {
                break;
            }
            if let Some(next) = redirects.get(&target) {
                target = *next;
            } else {
                resolved.insert(*label, target);
                break;
            }
        }
    }
    let mut changed = false;
    for i in lo..hi {
        if infos[i].pinned
            // DIRECTIVE-LAW: selecting -- rewrite branches only, never directives.
            || matches!(
                infos[i].kind,
                LineKind::Nop
                    | LineKind::Empty
                    | LineKind::Directive
                    | LineKind::Label
                    | LineKind::InlineAsm
            )
        {
            continue;
        }
        let s = infos[i].trimmed(store.get(i));
        let Some((mnemonic, target)) = branch(s) else {
            continue;
        };
        let Some(final_target) = resolved.get(&target) else {
            continue;
        };
        let comment = s.find('#').map(|at| &s[at..]).unwrap_or("");
        let replacement = format!("    {mnemonic} .LBB{final_target} {comment}");
        replace_line(store, &mut infos[i], i, replacement);
        changed = true;
    }
    changed
}

/// The prefix that makes a `#` comment compiler-owned rather than a human's.
///
/// Spelled once because two things depend on the same text: the fixtures below,
/// and the environment-knob census (`scripts/audit_census.py`), which counts
/// every `LCCC_<NAME>` literal in `src/` as an env knob.  A fixture that spells
/// a marker out in full therefore registers a knob that does not exist, in both
/// `knob_literals` and `knobs_undocumented`.  The rule is a prefix match, so the
/// prefix *is* the contract; naming it says so and keeps the fixtures honest.
const COMPILER_MARKER: &str = "LCCC_";

/// Human comments emit no bytes; assembler quarantine and compiler-owned
/// markers stay opaque.
fn inert_comment(s: &str) -> bool {
    s.starts_with('#') && s != "#APP" && s != "#NO_APP" && !s.contains(COMPILER_MARKER)
}

/// Every `.LBBn` mentioned by a live line other than as its own definition,
/// plus the labels defined more than once. Comments count: over-approximating
/// references only keeps more code.
fn label_uses(store: &LineStore, infos: &[LineInfo]) -> (FxHashSet<u32>, FxHashSet<u32>) {
    let mut used = FxHashSet::default();
    let mut defined = FxHashSet::default();
    let mut duplicate = FxHashSet::default();
    for (i, info) in infos.iter().enumerate() {
        if info.is_nop() {
            continue;
        }
        let mut s = info.trimmed(store.get(i));
        if info.kind == LineKind::Label {
            if let Some(label) = local_label(label_text(s))
                && !defined.insert(label)
            {
                duplicate.insert(label);
            }
            s = s.split_once(':').map_or("", |(_, rest)| rest);
        }
        let mut rest = s;
        while let Some(at) = rest.find(".LBB") {
            rest = &rest[at + 4..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if let Ok(label) = rest[..digits].parse() {
                used.insert(label);
            }
        }
    }
    (used, duplicate)
}

/// Delete the relays [`thread_function`] orphaned: a run of unreferenced,
/// uniquely defined, unpinned labels (inert comments between them stay) that
/// follows an unconditional transfer and ends in one direct `jmp`. Nothing
/// can reach the run, so dropping it changes no execution, only dead bytes.
fn remove_dead_relays(store: &LineStore, infos: &mut [LineInfo], lo: usize, hi: usize) -> bool {
    let mut changed = false;
    loop {
        let (used, duplicate) = label_uses(store, infos);
        let dead_label = |infos: &[LineInfo], k: usize| {
            infos[k].kind == LineKind::Label
                && !infos[k].pinned
                && local_label(label_text(infos[k].trimmed(store.get(k))))
                    .is_some_and(|l| !used.contains(&l) && !duplicate.contains(&l))
        };
        let mut found = None;
        let mut prev_transfers = false;
        let mut k = lo;
        while k < hi && found.is_none() {
            let info = infos[k];
            let s = info.trimmed(store.get(k));
            if info.is_nop() || info.kind == LineKind::Empty || inert_comment(s) {
                k += 1;
                continue;
            }
            if prev_transfers && dead_label(infos, k) {
                let mut labels = vec![k];
                let mut q = k + 1;
                while q < hi {
                    let qs = infos[q].trimmed(store.get(q));
                    if infos[q].is_nop() || infos[q].kind == LineKind::Empty || inert_comment(qs) {
                        q += 1;
                    } else if dead_label(infos, q) {
                        labels.push(q);
                        q += 1;
                    } else {
                        break;
                    }
                }
                if q < hi
                    && !infos[q].pinned
                    && infos[q].kind != LineKind::InlineAsm
                    && branch(infos[q].trimmed(store.get(q))).is_some_and(|(m, _)| m == "jmp")
                {
                    labels.push(q);
                    found = Some(labels);
                }
            }
            let mnemonic = s.split_whitespace().next().unwrap_or("");
            prev_transfers = !info.pinned
                && info.kind != LineKind::InlineAsm
                && info.kind != LineKind::Label
                && matches!(mnemonic, "jmp" | "ret" | "retq" | "ud2");
            k += 1;
        }
        let Some(lines) = found else {
            return changed;
        };
        for line in lines {
            mark_nop(&mut infos[line]);
        }
        changed = true;
    }
}

pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut start = None;
    let mut changed = false;
    for i in 0..store.len() {
        // DIRECTIVE-LAW: selecting function-boundary markers, not skipping
        // directives in a motion window (only inert debug attribution is transparent).
        if infos[i].pinned || infos[i].kind != LineKind::Directive {
            continue;
        }
        let s = infos[i].trimmed(store.get(i));
        if s == ".cfi_startproc" {
            start = Some(i + 1);
        } else if s == ".cfi_endproc" {
            if let Some(lo) = start.take() {
                if thread_function(store, infos, lo, i) {
                    remove_dead_relays(store, infos, lo, i);
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Every `.LBBn` mention count (definitions excluded), plus labels defined
/// more than once.  [`label_uses`] answers "mentioned at all"; a conditional
/// tail call deletes the relay, so it needs "mentioned exactly once".
pub(super) fn label_mention_counts(
    store: &LineStore,
    infos: &[LineInfo],
) -> (FxHashMap<u32, u32>, FxHashSet<u32>) {
    let mut count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut defined = FxHashSet::default();
    let mut duplicate = FxHashSet::default();
    for (i, info) in infos.iter().enumerate() {
        if info.is_nop() {
            continue;
        }
        let mut s = info.trimmed(store.get(i));
        if info.kind == LineKind::Label {
            if let Some(label) = local_label(label_text(s))
                && !defined.insert(label)
            {
                duplicate.insert(label);
            }
            s = s.split_once(':').map_or("", |(_, rest)| rest);
        }
        let mut rest = s;
        while let Some(at) = rest.find(".LBB") {
            rest = &rest[at + 4..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if let Ok(label) = rest[..digits].parse() {
                *count.entry(label).or_insert(0) += 1;
            }
        }
    }
    (count, duplicate)
}

/// Lines that emit no bytes and carry no CFG or unwind meaning: deleted lines,
/// blank lines, every `#` comment (compiler markers included -- the caller
/// moves the tail call's own marker block explicitly) and inert debug
/// attribution (`.loc`; the decision never depends on one, so `-g` cannot
/// change the code).
fn byteless(store: &LineStore, infos: &[LineInfo], k: usize) -> bool {
    let info = infos[k];
    let s = info.trimmed(store.get(k));
    info.is_nop()
        || info.kind == LineKind::Empty
        || (s.starts_with('#') && s != "#APP" && s != "#NO_APP")
        || (info.kind == LineKind::Directive && info.is_steppable() && is_debug_marker_directive(s))
}

/// Hoist the identical first instruction of both arms of a `jcc` above it:
///
/// ```text
///     testl %edi, %edi                testl %edi, %edi
///     je .LBB5                        movq %rsi, %rdi
/// .LBB4:                      ->  .LBB4:
///     movq %rsi, %rdi                 je .LBB5
///     jmp g1@PLT                      jmp g1@PLT
/// .LBB5:                          .LBB5:
///     movq %rsi, %rdi                 jmp g2@PLT
///     jmp g2@PLT
/// ```
///
/// GCC 16.2 emits the hoisted form for `return c ? g1(x) : g2(x);` (Clang
/// 23.1.4 keeps both copies); together with [`fold_conditional_tail_calls`]
/// it is four instructions where GCC needs five and Clang six.
///
/// Every path executes the instruction exactly once, before instead of after
/// the `jcc`.  That commutes because the `jcc` reads only EFLAGS and writes
/// nothing, and the instruction neither reads nor writes EFLAGS
/// ([`super::flag_peepholes::flags_effect`] `Neutral`), so it sees the same
/// registers and memory and the branch sees the same flags.  It must not be a
/// barrier (calls, pushes, pops, control flow, data directives, inline asm),
/// pinned, or a `%rsp` write: the CFI rows in force do not move with it.
///
/// No line is inserted: the `jcc` and the fall-through arm's head swap text,
/// which carries the fall-through arm's labels above the `jcc` -- sound only
/// because no line mentions them (they have no other predecessor) -- and the
/// target arm's copy is deleted, which needs its label mentioned by this
/// `jcc` alone and not entered by fall-through.
pub(super) fn hoist_common_arm_heads(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let mut start = None;
    for i in 0..store.len() {
        if !infos[i].pinned && infos[i].kind == LineKind::Directive {
            let s = infos[i].trimmed(store.get(i));
            if s == ".cfi_startproc" {
                start = Some(i + 1);
            } else if s == ".cfi_endproc"
                && let Some(lo) = start.take()
            {
                while hoist_one_arm_head(store, infos, lo, i) {
                    changed = true;
                }
            }
        }
    }
    changed
}

fn hoist_one_arm_head(store: &mut LineStore, infos: &mut [LineInfo], lo: usize, hi: usize) -> bool {
    let (count, duplicate) = label_mention_counts(store, infos);
    let mentions = |k: usize| {
        local_label(label_text(infos[k].trimmed(store.get(k))))
            .filter(|l| !duplicate.contains(l))
            .map(|l| count.get(&l).copied().unwrap_or(0))
    };
    for j in lo..hi {
        if infos[j].is_nop() || infos[j].pinned || infos[j].kind != LineKind::CondJmp {
            continue;
        }
        let Some((mnemonic, target)) = branch(infos[j].trimmed(store.get(j))) else {
            continue;
        };
        if mnemonic == "jmp" {
            continue;
        }
        // Fall-through arm: only unmentioned labels and byteless lines up to
        // its first instruction.
        let mut a = j + 1;
        let mut ok = true;
        while a < hi {
            if byteless(store, infos, a) {
                a += 1;
            } else if infos[a].kind == LineKind::Label && !infos[a].pinned {
                if mentions(a) != Some(0) {
                    ok = false;
                    break;
                }
                a += 1;
            } else {
                break;
            }
        }
        if !ok || a >= hi {
            continue;
        }
        // Target arm: its label, mentioned once (by this jcc), entered by no
        // fall-through, then byteless lines up to its first instruction.
        let Some(d) = (lo..hi).find(|&k| {
            infos[k].kind == LineKind::Label
                && !infos[k].is_nop()
                && local_label(label_text(infos[k].trimmed(store.get(k)))) == Some(target)
        }) else {
            continue;
        };
        if infos[d].pinned || mentions(d) != Some(1) || d <= a {
            continue;
        }
        let Some(p) = (lo..d).rev().find(|&k| !byteless(store, infos, k)) else {
            continue;
        };
        let prev_mn = infos[p]
            .trimmed(store.get(p))
            .split_whitespace()
            .next()
            .unwrap_or("");
        if infos[p].pinned
            || infos[p].kind == LineKind::InlineAsm
            || !matches!(prev_mn, "jmp" | "ret" | "retq" | "ud2")
        {
            continue;
        }
        let Some(b) = (d + 1..hi).find(|&k| !byteless(store, infos, k)) else {
            continue;
        };
        let head = infos[a].trimmed(store.get(a));
        if head != infos[b].trimmed(store.get(b)) || !hoistable(&infos[a], head) || infos[b].pinned
        {
            continue;
        }
        let head = head.to_string();
        let jcc = store.get(j).to_string();
        replace_line(store, &mut infos[j], j, format!("    {head}"));
        replace_line(store, &mut infos[a], a, jcc);
        mark_nop(&mut infos[b]);
        return true;
    }
    false
}

/// A straight-line instruction that commutes with a `jcc` and may move above
/// it without moving any CFI row (see [`hoist_common_arm_heads`]).
fn hoistable(info: &LineInfo, t: &str) -> bool {
    !info.pinned
        && !info.is_barrier()
        && !matches!(info.kind, LineKind::Empty | LineKind::Nop)
        && super::flag_peepholes::flags_effect(t) == super::flag_peepholes::FlagsEffect::Neutral
        && super::helpers::get_dest_reg(info) != REG_RSP
        && !t.contains("%rsp")
        && !t.contains("%esp")
}

/// `jcc .LBBn` whose only target block is a codegen-marked sibling call
/// becomes the conditional sibling call `jcc sym`:
///
/// ```text
///     testb $32, G(%rip)              testb $32, G(%rip)
///     je .LBB2                        je slow@PLT
///     jmp fast@PLT            ->      jmp fast@PLT
/// .LBB2:
///     jmp slow@PLT
/// ```
///
/// Clang 23.1.4 emits this form (`je slow@PLT`); GCC 16.2 keeps the relay
/// block (one instruction more).  Control reaches the relay's `jmp` from the
/// `jcc` through zero instructions, so registers, flags, memory and the stack
/// depth are identical at both: the taken edge performs the very same tail
/// call.  The CFI rows in force at the `jcc` describe that state exactly
/// (CFI is positional), which is why a relay holding ANY directive other than
/// inert debug attribution is refused -- a `.cfi_*` there would mean the
/// relay's linear CFI state differs from the jump site's.
///
/// Soundness conditions, each checked:
/// * the `jmp` is published as a sibling call (`# LCCC_TAIL_CALL`,
///   [`super::liveness::tail_call_marked`]) and targets a non-local symbol;
///   an unmarked `jmp sym` may be anything and stays put;
/// * the relay label is uniquely defined, unpinned, mentioned by exactly this
///   one line in the whole file (no other branch, jump table, computed goto
///   or comment), and not entered by fall-through, so deleting the relay
///   removes only unreachable bytes;
/// * the relay holds nothing but the `jmp` and byteless lines up to the next
///   label or the end of the function.
///
/// The marker block moves behind the new `jcc`, so both liveness oracles keep
/// modelling the taken edge as the tail call (`liveness`/`fp_liveness`
/// `CondJmp` arms).  Code size never grows: the relay's 5-byte `jmp` goes and
/// the `jcc` widens by at most 4 bytes (rel8 -> rel32).
pub(super) fn fold_conditional_tail_calls(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let mut start = None;
    let len = store.len();
    let mut i = 0;
    while i < len {
        if !infos[i].pinned && infos[i].kind == LineKind::Directive {
            let s = infos[i].trimmed(store.get(i));
            if s == ".cfi_startproc" {
                start = Some(i + 1);
            } else if s == ".cfi_endproc"
                && let Some(lo) = start.take()
            {
                changed |= conditional_tail_calls_in(store, infos, lo, i);
            }
        }
        i += 1;
    }
    changed
}

fn conditional_tail_calls_in(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    lo: usize,
    hi: usize,
) -> bool {
    let mut changed = false;
    // Recomputed after every rewrite: a fold removes one mention and one
    // definition, which can only shrink other labels' eligibility.
    loop {
        let (count, duplicate) = label_mention_counts(store, infos);
        let mut defs: FxHashMap<u32, usize> = FxHashMap::default();
        for k in lo..hi {
            if infos[k].kind == LineKind::Label && !infos[k].is_nop() && !infos[k].pinned {
                if let Some(l) = local_label(label_text(infos[k].trimmed(store.get(k)))) {
                    defs.insert(l, k);
                }
            }
        }
        let mut hit = None;
        for i in lo..hi {
            if infos[i].is_nop() || infos[i].pinned || infos[i].kind != LineKind::CondJmp {
                continue;
            }
            let Some((mnemonic, label)) = branch(infos[i].trimmed(store.get(i))) else {
                continue;
            };
            if mnemonic == "jmp" || duplicate.contains(&label) || count.get(&label) != Some(&1) {
                continue;
            }
            let Some(&d) = defs.get(&label) else {
                continue;
            };
            if let Some(plan) = relay_plan(store, infos, lo, hi, d) {
                hit = Some((i, mnemonic.to_string(), d, plan));
                break;
            }
        }
        let Some((i, mnemonic, d, (q, end))) = hit else {
            return changed;
        };
        let target = infos[q]
            .trimmed(store.get(q))
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string();
        // The marker block that follows the tail `jmp` (`# LCCC_TAIL_CALL`
        // and the suppressed call's own markers) moves with it.
        let markers: Vec<String> = (q + 1..end)
            .filter(|&k| !infos[k].is_nop())
            .map(|k| infos[k].trimmed(store.get(k)))
            .take_while(|m| m.starts_with("# LCCC_"))
            .map(|m| format!("    {m}"))
            .collect();
        let comment = {
            let s = infos[i].trimmed(store.get(i));
            s.find('#')
                .map(|at| s[at..].to_string())
                .unwrap_or_default()
        };
        let mut text = format!("    {mnemonic} {target}");
        if !comment.is_empty() {
            text.push(' ');
            text.push_str(&comment);
        }
        for m in &markers {
            text.push('\n');
            text.push_str(m);
        }
        for k in d..end {
            mark_nop(&mut infos[k]);
        }
        replace_line(store, &mut infos[i], i, text);
        changed = true;
    }
}

/// For the relay label on line `d`: `(jmp_line, block_end)` when the block is
/// exactly one codegen-marked sibling `jmp` (plus byteless lines) that no
/// fall-through enters; `block_end` is the next label or `hi`.
fn relay_plan(
    store: &LineStore,
    infos: &[LineInfo],
    lo: usize,
    hi: usize,
    d: usize,
) -> Option<(usize, usize)> {
    // Not entered by fall-through: the previous real line transfers away.
    let mut p = d;
    loop {
        if p <= lo {
            return None;
        }
        p -= 1;
        if !byteless(store, infos, p) {
            break;
        }
    }
    let prev = infos[p];
    let prev_mn = prev
        .trimmed(store.get(p))
        .split_whitespace()
        .next()
        .unwrap_or("");
    if prev.pinned
        || prev.kind == LineKind::InlineAsm
        || prev.kind == LineKind::Label
        || !matches!(prev_mn, "jmp" | "ret" | "retq" | "ud2")
    {
        return None;
    }
    let q = (d + 1..hi).find(|&k| !byteless(store, infos, k))?;
    let info = infos[q];
    if info.pinned || info.kind != LineKind::Jmp {
        return None;
    }
    let jt = info.trimmed(store.get(q));
    let mut fields = jt.split_whitespace();
    if fields.next() != Some("jmp") {
        return None;
    }
    let target = fields.next()?;
    if fields.next().is_some()
        || target.starts_with('*')
        || target.starts_with('.')
        || !super::liveness::tail_call_marked(store, infos, q)
    {
        return None;
    }
    let end = (q + 1..hi)
        .find(|&k| !byteless(store, infos, k))
        .unwrap_or(hi);
    (end == hi || infos[end].kind == LineKind::Label).then_some((q, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(body: &str, pin: Option<&str>) -> (bool, String) {
        let mut store = LineStore::new(format!("f:\n.cfi_startproc\n{body}\n.cfi_endproc\n"));
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        if let Some(line) = pin {
            for (i, info) in infos.iter_mut().enumerate() {
                if store.get(i).trim() == line {
                    info.pinned = true;
                }
            }
        }
        let changed = run(&mut store, &mut infos);
        let text = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!run(&mut store, &mut infos), "must reach a fixpoint");
        (changed, text)
    }

    #[test]
    fn threads_chain_aliases_comments_and_drops_orphaned_relays() {
        for ws in [" ", "\t", "  \t "] {
            let body = format!(
                "jne{ws}.LBB1 # edge\nret\n.LBB1:\n# harmless\n.LBB2: # alias\njmp{ws}.LBB3\n.LBB3:\njmp{ws}.LBB4\n.LBB4:\nadcq $0, %rax\nret"
            );
            let (changed, out) = apply(&body, None);
            assert!(changed, "separator {ws:?}: {out}");
            assert_eq!(
                out,
                "f:\n.cfi_startproc\njne .LBB4 # edge\nret\n# harmless\n.LBB4:\nadcq $0, %rax\nret\n.cfi_endproc",
                "separator {ws:?}"
            );
        }
    }

    /// Only the orphaned, unreachable relay goes: a fallthrough entry, any
    /// mention (data, comment, `leaq`), pinning or a compiler marker keeps it.
    #[test]
    fn orphaned_relay_removal_fails_closed() {
        let (_, out) = apply(
            "jle .LBB3\nmovl $1, %eax\nret\n.LBB3:\njmp .LBB2\n.LBB2:\nret",
            None,
        );
        assert!(out.contains("jle .LBB2") && !out.contains(".LBB3"), "{out}");
        for keep in [
            "jle .LBB3\nmovl $1, %eax\n.LBB3:\njmp .LBB2\n.LBB2:\nret",
            "jle .LBB3\nret\n.LBB3:\njmp .LBB2\n.LBB2:\nret\n.quad .LBB3",
            "jle .LBB3\nret # .LBB3\n.LBB3:\njmp .LBB2\n.LBB2:\nret",
            "jle .LBB3\nret\n.LBB3:\njmp .LBB2\n.LBB2:\nret\n.LBB3:",
        ] {
            let (_, out) = apply(keep, None);
            assert!(out.contains(".LBB3:"), "{keep}: {out}");
        }
        // A compiler-owned marker keeps the relay too.  Built from the constant
        // `inert_comment` matches, so this fixture cannot drift from the rule and
        // contributes no `LCCC_<NAME>` literal for the knob census to count.
        let marker = format!("# {COMPILER_MARKER}relay");
        let marked = format!("jle .LBB3\nret\n{marker}\n.LBB3:\njmp .LBB2\n.LBB2:\nret");
        let (_, out) = apply(&marked, None);
        assert!(out.contains(".LBB3:"), "{marked}: {out}");
        let body = "jle .LBB3\nret\n.LBB3:\njmp .LBB2\n.LBB2:\nret";
        assert!(apply(body, Some(".LBB3:")).1.contains(".LBB3:"));
    }

    #[test]
    fn debug_markers_are_transparent_but_lookalikes_are_not() {
        for marker in [".loc 1 7 3", r#".file 2 "x.c""#, ".loc\t1 7 3"] {
            let body = format!("jne .LBB1\nret\n.LBB1:\n{marker}\njmp .LBB2\n.LBB2:\nret");
            let (changed, out) = apply(&body, None);
            assert!(changed && out.contains("jne .LBB2"), "{out}");
            assert!(out.contains(marker));
            assert!(!apply(&body, Some(marker)).0);
        }
        for marker in [
            ".location 1",
            ".file_data 2",
            ".cfi_remember_state",
            ".balign 16",
            ".quad .LBB2",
        ] {
            let body = format!("jne .LBB1\nret\n.LBB1:\n{marker}\njmp .LBB2\n.LBB2:\nret");
            assert!(!apply(&body, None).0, "{marker}");
        }
    }

    #[test]
    fn retains_address_taken_jump_table_and_fallthrough_labels() {
        let body = "leaq .LBB1(%rip), %rax\njne .LBB1\n.LBB1:\njmp .LBB2\n.LBB2:\nret\n.quad .LBB1\n.long .LBB1-.LBB2";
        let (_, out) = apply(body, None);
        for text in [
            "leaq .LBB1(%rip)",
            ".quad .LBB1",
            ".long .LBB1-.LBB2",
            ".LBB1:",
            "jmp .LBB2",
        ] {
            assert!(out.contains(text), "{text}: {out}");
        }
        for label in [".LBB01", ".LBB4294967296", ".LBB", ".LBB-1"] {
            assert!(local_label(label).is_none());
        }
    }

    #[test]
    fn refuses_effects_directives_and_pinned_trampolines() {
        for barrier in [
            "addq $1, %rax",
            "movq %rax, (%rdi)",
            "testl %eax, %eax",
            "call other",
            "endbr64",
            ".cfi_def_cfa_offset 16",
            ".p2align 4,0xcc",
            ".byte 0xcc",
            ".section .data",
            "#APP\nmovq %rax, %rbx\n#NO_APP",
            "#APP\njmp .LBB2\n#NO_APP",
        ] {
            let body = format!("jne .LBB1\nret\n.LBB1:\n{barrier}\njmp .LBB2\n.LBB2:\nret");
            assert!(!apply(&body, None).0, "crossed {barrier}");
        }
        let body = "jne .LBB1\nret\n.LBB1:\njmp .LBB2\n.LBB2:\nret";
        assert!(!apply(body, Some("jmp .LBB2")).0);
        let (_, out) = apply(body, Some("jne .LBB1"));
        assert!(out.contains("jne .LBB1"));
    }

    #[test]
    fn refuses_cycles_unknown_targets_indirect_and_short_only_branches() {
        for body in [
            "jne .LBB1\nret\n.LBB1:\njmp .LBB1",
            "jne .LBB1\nret\n.LBB1:\njmp .LBB2\n.LBB2:\njmp .LBB1",
            "jne .LBB1\nret\n.LBB1:\njmp .LBB99",
            "jne .LBB1\nret\n.LBB1:\njmp *%rax",
            "jrcxz .LBB1\nret\n.LBB1:\njmp .LBB2\n.LBB2:\nret",
            "jecxz .LBB1\nret\n.LBB1:\njmp .LBB2\n.LBB2:\nret",
            "jcxz .LBB1\nret\n.LBB1:\njmp .LBB2\n.LBB2:\nret",
        ] {
            assert!(!apply(body, None).0, "{body}");
        }
    }

    #[test]
    fn refuses_overlong_chains_cross_function_edges_and_duplicate_labels() {
        let mut body = "jne .LBB0\nret\n".to_owned();
        for i in 0..66 {
            body += &format!(".LBB{i}:\njmp .LBB{}\n", i + 1);
        }
        body += ".LBB66:\nret";
        // Tail portions may shorten, but the over-budget entry is unchanged.
        let mut store = LineStore::new(format!(".cfi_startproc\n{body}\n.cfi_endproc"));
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        run(&mut store, &mut infos);
        assert_eq!(store.get(1), "jne .LBB0");
        assert!(
            !apply(
                "jne .LBB1\nret\n.LBB1:\njmp .LBB2\n.cfi_endproc\ng:\n.cfi_startproc\n.LBB2:\nret",
                None
            )
            .0
        );
        assert!(!apply("jne .LBB1\n.LBB1:\njmp .LBB2\n.LBB1:\n.LBB2:\nret", None).0);
    }
}

#[cfg(test)]
mod conditional_tail_call_tests {
    use super::*;

    const T: &str = "# LCCC_TAIL_CALL";

    fn apply(body: &str) -> (bool, String) {
        let mut store = LineStore::new(format!("f:\n.cfi_startproc\n{body}.cfi_endproc\n"));
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        let changed = fold_conditional_tail_calls(&mut store, &mut infos);
        let text = store.build_result(|i| infos[i].is_nop());
        // Idempotent on its own output.
        let mut s2 = LineStore::new(text.clone());
        let mut i2: Vec<_> = (0..s2.len()).map(|i| classify_line(s2.get(i))).collect();
        assert!(
            !fold_conditional_tail_calls(&mut s2, &mut i2),
            "fixpoint:\n{text}"
        );
        (changed, text)
    }

    fn dispatch(relay: &str) -> String {
        format!(
            "    testb $32, G(%rip)\n    je .LBB2\n    jmp fast@PLT\n    {T}\n    # LCCC_CALL_ARGS 0\n.LBB2:\n{relay}"
        )
    }

    #[test]
    fn marked_relay_becomes_a_conditional_sibling_call() {
        let (c, out) = apply(&dispatch(&format!(
            "    # LCCC_CALL_ARGS 0\n    jmp slow@PLT\n    {T}\n    # LCCC_CALL_ARGS 0\n    # LCCC_CALL_FP 0\n"
        )));
        assert!(c, "{out}");
        assert!(out.contains("    je slow@PLT\n    # LCCC_TAIL_CALL\n    # LCCC_CALL_ARGS 0\n    # LCCC_CALL_FP 0\n"), "{out}");
        assert!(!out.contains(".LBB2") && !out.contains("jmp slow"), "{out}");
    }

    #[test]
    fn debug_rows_do_not_change_the_decision() {
        let (c, out) = apply(&dispatch(&format!(
            ".loc 1 3 0\n    jmp slow@PLT\n    {T}\n.loc 1 4 0\n"
        )));
        assert!(c && out.contains("je slow@PLT"), "{out}");
    }

    #[test]
    fn unmarked_local_indirect_and_cfi_relays_stay() {
        for relay in [
            // Unmarked: may be anything (an inline retpoline, a foreign label).
            "    jmp slow@PLT\n".to_string(),
            // Local label target: ordinary edge, jump threading's business.
            format!("    jmp .LBB9\n    {T}\n.LBB9:\n    ret\n"),
            // Indirect sibling call: no conditional indirect jump exists.
            format!("    jmp *%r11\n    {T}\n"),
            // A CFI row in the relay: its linear unwind state differs.
            format!("    .cfi_def_cfa_offset 8\n    jmp slow@PLT\n    {T}\n"),
            // More than the jump.
            format!("    movl %esi, %edi\n    jmp slow@PLT\n    {T}\n"),
            // Code after the jump before the next label (SLS int3).
            format!("    jmp slow@PLT\n    {T}\n    int3\n"),
        ] {
            let (c, out) = apply(&dispatch(&relay));
            assert!(!c, "{relay}:\n{out}");
        }
    }

    #[test]
    fn shared_fallthrough_or_mentioned_relays_stay() {
        // Fall-through entry: the relay is reachable without the jcc.
        let (c, out) = apply(&format!(
            "    testl %edi, %edi\n    je .LBB2\n    call g@PLT\n.LBB2:\n    jmp slow@PLT\n    {T}\n"
        ));
        assert!(!c, "{out}");
        // Two branches to the relay.
        let (c, out) = apply(&format!(
            "    testl %edi, %edi\n    je .LBB2\n    testl %esi, %esi\n    jne .LBB2\n    jmp fast@PLT\n    {T}\n.LBB2:\n    jmp slow@PLT\n    {T}\n"
        ));
        assert!(!c, "{out}");
        // A jump-table / data mention.
        let (c, out) = apply(&format!(
            "{}    .quad .LBB2\n",
            dispatch(&format!("    jmp slow@PLT\n    {T}\n"))
        ));
        assert!(!c, "{out}");
    }

    #[test]
    fn liveness_models_the_taken_edge_as_the_tail_call() {
        use super::super::liveness::FileLiveness;
        // %rdi feeds the conditional tail call (1 argument); %rsi is dead
        // on both edges; %rax is not live into a tail call.
        let asm = format!(
            "f:\n.cfi_startproc\n    movl $1, %esi\n    movl $2, %eax\n    testl %edx, %edx\n    je slow@PLT\n    {T}\n    # LCCC_CALL_ARGS 1\n    # LCCC_CALL_FP 0\n    xorl %eax, %eax\n    ret\n.cfi_endproc\n"
        );
        let store = LineStore::new(asm);
        let infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let lv = FileLiveness::new(&store, &infos);
        let test = (0..store.len())
            .find(|&i| store.get(i).trim() == "testl %edx, %edx")
            .unwrap();
        assert_eq!(
            lv.live_after(test, 7),
            Some(true),
            "%rdi feeds the tail call"
        );
        assert_eq!(lv.live_after(test, 6), Some(false), "%rsi is dead");
        assert_eq!(lv.live_after(test, 0), Some(false), "%rax is not owed");
        // Unmarked: the foreign target is unknown, never guessed.
        let asm =
            "f:\n.cfi_startproc\n    testl %edx, %edx\n    je slow@PLT\n    ret\n.cfi_endproc\n";
        let store = LineStore::new(asm.to_string());
        let infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let lv = FileLiveness::new(&store, &infos);
        assert_eq!(lv.live_after(2, 6), None);
    }
}

#[cfg(test)]
mod arm_head_hoist_tests {
    use super::*;

    fn apply(body: &str) -> (bool, String) {
        let mut store = LineStore::new(format!("f:\n.cfi_startproc\n{body}.cfi_endproc\n"));
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        let changed = hoist_common_arm_heads(&mut store, &mut infos);
        (changed, store.build_result(|i| infos[i].is_nop()))
    }

    const SEL: &str = "    testl %edi, %edi\n    je .LBB5\n.LBB4:\n    movq %rsi, %rdi\n    jmp g1@PLT\n.LBB5:\n    movq %rsi, %rdi\n    jmp g2@PLT\n";

    #[test]
    fn identical_heads_move_above_the_branch() {
        let (c, out) = apply(SEL);
        assert!(c, "{out}");
        assert!(
            out.contains("    testl %edi, %edi\n    movq %rsi, %rdi\n.LBB4:\n    je .LBB5\n    jmp g1@PLT\n.LBB5:\n    jmp g2@PLT\n"),
            "{out}"
        );
    }

    #[test]
    fn flag_writers_shared_or_entered_arms_and_rsp_stay() {
        for body in [
            // Different heads.
            SEL.replacen("movq %rsi, %rdi", "movq %rdx, %rdi", 1),
            // A flag writer would clobber the branch's flags.
            SEL.replace("movq %rsi, %rdi", "addl $1, %esi"),
            // A flag reader would see the flags at a different point -- it
            // is the same point, but `setcc` is not Neutral: refused.
            SEL.replace("movq %rsi, %rdi", "sete %al"),
            // The fall-through label has another predecessor.
            format!("{SEL}    jmp .LBB4\n"),
            // The target is entered by fall-through.
            SEL.replace("    jmp g1@PLT\n", ""),
            // A stack-pointer write: CFI rows would not move with it.
            SEL.replace("movq %rsi, %rdi", "leaq 8(%rsp), %rsp"),
            // A push is a barrier.
            SEL.replace("movq %rsi, %rdi", "pushq %rbx"),
        ] {
            let (c, out) = apply(&body);
            assert!(!c, "{body}\n->\n{out}");
        }
    }

    #[test]
    fn repeated_heads_hoist_one_at_a_time_to_a_fixpoint() {
        let body = SEL.replace("movq %rsi, %rdi\n", "movq %rsi, %rdi\n    movl $3, %esi\n");
        let (c, out) = apply(&body);
        assert!(c, "{out}");
        assert!(
            out.contains(
                "    testl %edi, %edi\n    movq %rsi, %rdi\n.LBB4:\n    movl $3, %esi\n    je .LBB5\n    jmp g1@PLT\n.LBB5:\n    jmp g2@PLT\n"
            ),
            "{out}"
        );
        assert_eq!(out.matches("movl $3, %esi").count(), 1, "{out}");
        assert_eq!(out.matches("movq %rsi, %rdi").count(), 1, "{out}");
    }
}
