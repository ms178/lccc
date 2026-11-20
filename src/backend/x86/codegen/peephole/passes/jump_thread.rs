//! Bypass empty direct-jump trampolines left by allocation and dead copies.
//!
//! Retarget edges, never reorder code. Two deletions, both proved by the same
//! retargeting. The first is a relay the retargeting orphaned: labels no line
//! of the file mentions (so no computed goto, jump table or address comparison
//! can observe them), entered by no fallthrough, holding one direct `jmp`. The
//! skipped window contains only labels, comments, deleted lines and one
//! unconditional jump: registers, memory and flags are unchanged. The second is
//! a branch the retargeting landed on the label it already falls through to,
//! which is a no-op whichever way it is spelled — see [`falls_through_to`].
//! Keep function scope, inline asm,
//! pinned lines, semantic directives (including CFI and alignment padding),
//! short-only counter branches and cyclic/overlong chains fail-closed. Section
//! switches are the one directive family the sweep READS rather than refuses:
//! they do not emit bytes into the section being left, so they say something
//! precise about fallthrough (see [`remove_dead_relays`]).
//!
//! Switch jump-table ENTRIES thread through the same proof. A shared-body
//! `switch` routinely dispatches a case through a relay the branch rewrites
//! cannot retire — the table mention keeps the label alive, so every NULL
//! record sqlite_vdbe_serial decodes ran `jmp <next instruction>` straight
//! off the indirect dispatch. An entry is rewritten only inside a table the
//! codegen itself laid down (a run of entries directly under a `.Ljt_*:`
//! label, in both the absolute `.quad .LBBn` and the PIC
//! `.long .LBBn - .Ljt_k` spellings), so a user's computed-goto array —
//! whose element VALUES a program may compare — is never touched: those
//! arrive through `&&label` materialisation, not through a `.Ljt` table.
//! Retargeting an entry does not move, grow or renumber anything; the relay
//! itself then becomes unreferenced and the ordinary orphan sweep deletes it —
//! provided the sweep can still see that nothing falls through into it. A relay
//! emitted AFTER its table sits behind the directive that returns from the
//! table, so the sweep carries that verdict across the excursion instead of
//! reading the return as an ordinary line: GAS resumes each section where it
//! left off, and `remove_dead_relays` documents the identity gate that keeps
//! this from restoring a verdict into a different section.
use super::super::types::*;
use crate::backend::peephole_common::{SectionKind, SectionSwitch, SectionTracker, section_switch};
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
        .trim_ascii()
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

/// Does control fall from line `i` to `.LBB<target>` without executing anything?
///
/// Labels, deleted lines, inert comments and byteless debug attribution emit no
/// bytes, so the answer is yes when the target's definition comes before any
/// line that does emit. That is exactly the question a retarget needs answered
/// before it can delete the branch it was about to rewrite: if control arrives
/// at the target either way, the branch is a no-op — for a `jmp` because it is
/// unconditional, and for a `jcc` because both of its outcomes then lead to the
/// same instruction. Flags are not touched by a branch, so nothing else is
/// observable.
///
/// Scoped to the function window, and every line that could emit a byte — an
/// instruction, a data or alignment directive, inline asm, a pinned line —
/// answers no. Stacked labels answer yes: falling through `.LBB26:` into
/// `.LBB27:` executes the same instructions as jumping to `.LBB27` directly,
/// and `.LBB26` keeps its own references either way.
fn falls_through_to(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    hi: usize,
    target: u32,
) -> bool {
    for j in (i + 1)..hi {
        let info = infos[j];
        let s = info.trimmed(store.get(j));
        if info.is_nop() || info.kind == LineKind::Empty || inert_comment(s) {
            continue;
        }
        if info.kind == LineKind::Label {
            if local_label(label_text(s)) == Some(target) {
                return true;
            }
            continue;
        }
        if info.kind == LineKind::Directive
            && !info.pinned
            && info.is_steppable()
            && is_debug_marker_directive(s)
        {
            continue;
        }
        return false;
    }
    false
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
            // (Jump-table entries, the one directive family whose retarget is
            // proven by the same redirect map, have their own gated loop below.)
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
        // A retarget can land the branch on the label it ALREADY falls through
        // to, which makes it a no-op: `loop_global_promote.c` at -O2 carries
        // `    jmp .LBB27` directly above `.LBB27:` after the peepholes and no
        // such jump before them (measured on that TU with LCCC_NO_PEEPHOLE=1: 0
        // against 2), so the retargeting created it — and
        // `eliminate_redundant_jumps` ran three phases earlier, so nobody else
        // will notice.  Delete instead of write.  A compiler-owned marker in the
        // trailing comment has to survive, so such a line is still rewritten.
        if (comment.is_empty() || inert_comment(comment))
            && falls_through_to(store, infos, i, hi, *final_target)
        {
            mark_nop(&mut infos[i]);
            changed = true;
            continue;
        }
        let replacement = format!("    {mnemonic} .LBB{final_target} {comment}");
        replace_line(store, &mut infos[i], i, replacement);
        changed = true;
    }

    // ── Jump-table entries (see the module documentation) ──────────────────
    //
    // Dispatching through a table entry and executing the label's relay
    // window are the same transfer the branch loop above retargets: the
    // backward scan built `redirects` only across lines that emit no bytes
    // and carry no CFG or unwind meaning, so landing at the final target is
    // observationally identical.  The `in_table` gate restricts the rewrite
    // to entry runs directly under a `.Ljt_*:` label — the codegen's own
    // switch tables — and any other line (a section switch, an alignment
    // directive, an instruction) closes the run, so no `.quad .LBBn` that
    // happens to live elsewhere in the file is ever touched.
    let mut in_table = false;
    for i in lo..hi {
        let info = infos[i];
        let s = info.trimmed(store.get(i));
        if info.is_nop() || info.kind == LineKind::Empty || inert_comment(s) {
            continue;
        }
        if info.kind == LineKind::Label {
            in_table = label_text(s).starts_with(".Ljt_");
            continue;
        }
        if !in_table {
            continue;
        }
        // DIRECTIVE-LAW: selecting -- this loop reads directives to find the
        // exact two table-entry spellings the switch emitter produces and
        // rewrites nothing else; any other directive (section switch, align,
        // data) closes the run below instead of being stepped over.
        let entry = (info.kind == LineKind::Directive && !info.pinned)
            .then(|| table_entry_target(s))
            .flatten();
        let Some((target, pic_base)) = entry else {
            in_table = false; // any non-entry line ends the table run
            continue;
        };
        let Some(final_target) = resolved.get(&target) else {
            continue; // still a valid entry: the run continues
        };
        let replacement = match pic_base {
            Some(base) => format!("    .long .LBB{final_target} - {base}"),
            None => format!("    .quad .LBB{final_target}"),
        };
        replace_line(store, &mut infos[i], i, replacement);
        changed = true;
    }
    changed
}

/// Parse one switch-table entry: the absolute `.quad .LBBn` or the PIC
/// `.long .LBBn - .Ljt_k`, returning the target label and, for the PIC form,
/// the table-base symbol to reproduce verbatim.  Anything else — an extra
/// token, a non-`.LBB` target, a difference against something that is not a
/// `.Ljt_*` base — answers `None` (fail closed).
fn table_entry_target(s: &str) -> Option<(u32, Option<&str>)> {
    if let Some(rest) = s.strip_prefix(".quad ") {
        return Some((local_label(rest.trim_ascii())?, None));
    }
    let rest = s.strip_prefix(".long ")?;
    let (target, base) = rest.split_once('-')?;
    let base = base.trim_ascii();
    if !base.starts_with(".Ljt_") || base.contains(char::is_whitespace) {
        return None;
    }
    Some((local_label(target.trim_ascii())?, Some(base)))
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
        while let Some(at) = crate::backend::peephole_common::find_lbb(rest) {
            rest = &rest[at + 4..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if let Ok(label) = rest[..digits].parse() {
                used.insert(label);
            }
        }
    }
    (used, duplicate)
}

/// The identity of the section an `Enter`/`Push` switch selects: the name GAS
/// keys an ordinary section by, with flag/type/align arguments dropped.
/// A group, explicit unique ID or subsection cannot be collapsed to a name.
/// The bare forms are spelled the way `.section`
/// spells them, so `.text` and `.section .text,"ax",@progbits` compare equal —
/// which matters because the emitter uses both for one section (a bare `.text`
/// in `ipa_ra`'s hot/cold text against `EmitState::restore_current_text_section`
/// returning from a switch table). A subsection argument is KEPT, so `.text 1`
/// stays distinct from `.text`: another chunk of the same section is not a place
/// the previous chunk falls through into.
///
/// Only called on a line [`section_switch`] classified as `Enter`/`Push`, which
/// has already certified it is a single statement, so the name is the first
/// token and the identity ends at the first comma. `None` costs the restore,
/// never soundness.
fn section_identity(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let stmt = trimmed.strip_prefix('.')?;
    let end = stmt.find(char::is_whitespace).unwrap_or(stmt.len());
    match &stmt[..end] {
        "section" | "pushsection" => {
            let mut parts = stmt[end..].trim_start().split(',').map(str::trim);
            let ident = parts.next()?;
            if let Some(flags) = parts.next() {
                // `.pushsection name, subsection, ...` is also legal GAS.
                let flags = flags.strip_prefix('"')?.strip_suffix('"')?;
                if flags.contains(['G', '?']) || parts.any(|p| p == "unique") {
                    return None;
                }
            }
            (ident.starts_with('.') && !ident.contains(char::is_whitespace)).then_some(ident)
        }
        // The bare forms GAS accepts — measured on binutils 2.47 `as`, which
        // assembles `.text`, `.data`, `.bss` and `.text 1` and REJECTS a bare
        // `.rodata`/`.tdata`/`.tbss` ("unknown pseudo-op").  This arm list has
        // to agree with `section_switch`'s: an identity for a spelling the
        // shared classifier does not report would be dead code, and a spelling
        // it reports but this refuses would silently cost the restore.
        "text" | "data" | "bss" => {
            if stmt[end..].trim().is_empty() {
                Some(&trimmed[..1 + end])
            } else {
                Some(trimmed)
            }
        }
        _ => None,
    }
}

/// Where the assembly text is in section-IDENTITY terms. [`SectionTracker`]
/// answers "code or not" and handles `.pushsection`/`.previous` exactly, but a
/// fallthrough verdict may only be restored into the section that EARNED it, and
/// two code sections (`.text` against `.text.hot`) are two places in the object
/// file. So the sweep tracks names beside the shared tracker's kinds, with the
/// same push/pop shape GAS has.
#[derive(Clone, Debug)]
struct SectionIdentity {
    current: Option<String>,
    pushed: Vec<Option<String>>,
}

impl Default for SectionIdentity {
    /// A translation unit starts in `.text`: GAS's initial section, the one
    /// `SectionTracker::default` assumes and the value
    /// `EmitState::current_text_section` is constructed with.
    fn default() -> Self {
        Self::at(Some(".text"))
    }
}

impl SectionIdentity {
    /// A scan that starts inside a function body: the identity [`run`] tracked
    /// to that function's `.cfi_startproc`, or unknown.
    fn at(entry: Option<&str>) -> Self {
        SectionIdentity {
            current: entry.map(str::to_string),
            pushed: Vec::new(),
        }
    }

    fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Follow one switch and report the identity it lands in. `.popsection`
    /// hands back the name the matching `.pushsection` saved, so a round trip
    /// through a data section is still provable; `.previous` swaps two sections
    /// whose names the text does not carry, and an opaque (compound) statement
    /// may have done anything, so both end in unknown.
    fn apply(&mut self, switch: SectionSwitch, text: &str) -> Option<&str> {
        self.current = match switch {
            SectionSwitch::Enter(_) => section_identity(text).map(str::to_string),
            SectionSwitch::Push(_) => {
                self.pushed.push(self.current.take());
                section_identity(text).map(str::to_string)
            }
            SectionSwitch::Pop => self.pushed.pop().unwrap_or(None),
            SectionSwitch::Previous | SectionSwitch::Opaque => None,
        };
        self.current()
    }

    /// Opaque text — an `#APP` region that does not visibly leave the section
    /// it found ([`AsmRegionSections`]), a pinned directive, conditional
    /// assembly — forgets everything: a wrong name restores a verdict into a
    /// section that never earned it, which is a miscompile, while an unknown
    /// name only costs the deletion.
    fn forget(&mut self) {
        self.current = None;
        self.pushed.clear();
    }
}

/// What one `#APP` region does to the current section, read from its text.
///
/// The emitter resumes the function after a region without naming a section:
/// it relies on the region leaving the section it found, the contract GCC and
/// Clang rely on as well (both print a switch table as `.section .rodata` …
/// `.text` on the same belief). Forgetting the section at EVERY region, as
/// this sweep used to, made it inert in exactly the code that needs it: a
/// kernel translation unit has an `ALTERNATIVE`, a `__bug_table` or a
/// `__jump_table` entry in nearly every function, so no function after the
/// first one could prove its text contiguous around a switch table, and each
/// relay behind a table stayed in the object (objtool: `unreachable
/// instruction` in `ftrace_bug`, `rcu_preempt_ctxt_queue`,
/// `compute_live_registers`, `l1tf_apply_mitigation`).
///
/// So the region is READ instead. Its statements are split the way GAS splits
/// them, and the section is kept only when every switch in it is undone:
/// `.pushsection` by `.popsection`, a single `.section`/`.text`/`.data`/`.bss`
/// by `.previous`. Anything that could hide or duplicate a switch
/// ([`section_proof_ends`]), a quote that does not close, an unmatched pop
/// or a region that ends displaced still forgets, as before. A macro
/// call is not visible as one; for it the emitter's own contract stands, the
/// one that already decides where the rest of the function is assembled.
#[derive(Default)]
struct AsmRegionSections {
    /// `displaced` as saved by each open `.pushsection`.
    pushed: Vec<bool>,
    /// One `.section`-style switch is waiting for its `.previous`.
    displaced: bool,
    broken: bool,
}

impl AsmRegionSections {
    fn feed(&mut self, line: &str) {
        let mut rest = line;
        while !self.broken && !rest.is_empty() {
            let (statement, tail) = match first_asm_statement(rest) {
                Some(split) => split,
                None => {
                    self.broken = true;
                    return;
                }
            };
            self.statement(statement);
            rest = tail;
        }
    }

    fn statement(&mut self, statement: &str) {
        // `1: .pushsection x` — labels come first and switch nothing.
        let mut stmt = statement.trim_ascii();
        while let Some(colon) = stmt.find(':') {
            let label = &stmt[..colon];
            let is_label = !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'$'));
            if !is_label {
                break;
            }
            stmt = stmt[colon + 1..].trim_ascii_start();
        }
        let Some(directive) = stmt.strip_prefix('.') else {
            return;
        };
        let end = directive
            .find(|c: char| c.is_whitespace() || c == ',')
            .unwrap_or(directive.len());
        // GAS folds the case of a pseudo-op before it looks it up.
        let name = directive[..end].to_ascii_lowercase();
        match name.as_str() {
            "pushsection" => self.pushed.push(std::mem::take(&mut self.displaced)),
            "popsection" => match self.pushed.pop() {
                Some(displaced) => self.displaced = displaced,
                None => self.broken = true,
            },
            // Inside an open `.pushsection` the matching pop restores the
            // section, the previous section and the subsection alike.
            "section" | "text" | "data" | "bss" | "previous" | "subsection"
                if !self.pushed.is_empty() => {}
            "section" | "text" | "data" | "bss" => {
                // A second switch makes `.previous` return to the first.
                self.broken |= std::mem::replace(&mut self.displaced, true);
            }
            "previous" => self.broken |= !std::mem::take(&mut self.displaced),
            _ => self.broken |= section_proof_ends(&format!(".{name}")),
        }
    }

    /// The region left the section, the previous section and the
    /// `.pushsection` stack as it found them.
    fn neutral(&self) -> bool {
        !self.broken && !self.displaced && self.pushed.is_empty()
    }
}

/// Split the first statement off `text` the way GAS does for x86: `;` and a
/// newline separate statements, `#` starts a comment that runs to the end of
/// the line, and none of them counts inside a quoted string. `None` when a
/// quote does not close — the statement boundaries are then unknowable.
fn first_asm_statement(text: &str) -> Option<(&str, &str)> {
    let mut quote = None;
    let mut escaped = false;
    for (i, b) in text.bytes().enumerate() {
        if escaped {
            escaped = false;
        } else if let Some(q) = quote {
            if b == b'\\' {
                escaped = true;
            } else if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' => quote = Some(b),
                b';' | b'\n' | b'\r' => return Some((&text[..i], &text[i + 1..])),
                b'#' => {
                    let tail = text[i..].find('\n').map_or("", |n| &text[i + n + 1..]);
                    return Some((&text[..i], tail));
                }
                _ => {}
            }
        }
    }
    (quote.is_none() && !escaped).then_some((text, ""))
}

/// Cheap gate in front of [`section_switch`], which scans the whole statement
/// (quoting, separators) before it can answer. Only these first letters can
/// start a section pseudo-op, so the directives that dominate a real
/// translation unit — `.loc`, `.cfi_*`, `.quad`, `.long`, `.align`, `.globl`,
/// `.ident`, `.file` — are rejected on one byte instead of one scan. A false
/// positive costs nothing: [`section_switch`] still decides.
fn may_switch_section(s: &str) -> bool {
    matches!(
        s.trim_start().as_bytes().get(1),
        Some(b't' | b'd' | b'b' | b's' | b'p')
    ) || s.contains(';')
}

/// Directives that can move the next bytes somewhere this scan cannot follow
/// WITHOUT being a section switch [`section_switch`] reports: conditional
/// assembly can delete or duplicate a switch wholesale, `.subsection` moves to
/// another chunk of the same section, and a mode or syntax change re-spells
/// every line after it. Data directives are deliberately ABSENT — `.quad` and
/// `.align` are the switch table this proof exists to carry a verdict across,
/// and `.cfi_*` is absent too: it moves no bytes and changes no section, so the
/// generic arm below already reads it as the non-terminator it is.
fn section_proof_ends(s: &str) -> bool {
    let stmt = s.trim_start().strip_prefix('.').unwrap_or("");
    let name = stmt
        .split(|c: char| c.is_whitespace() || c == ',')
        .next()
        .unwrap_or("");
    name.starts_with("if")
        || matches!(
            name,
            "elseif"
                | "else"
                | "endif"
                | "macro"
                | "endm"
                | "rept"
                | "irp"
                | "irpc"
                | "endr"
                | "include"
                | "altmacro"
                | "subsection"
                | "code16"
                | "code16gcc"
                | "code32"
                | "code64"
                | "intel_syntax"
                | "att_syntax"
                | "arch"
        )
}

/// Skip the marker block directly attached to an unconditional sibling jump.
/// The markers stay on that jump; for the fallthrough proof they emit no bytes.
/// Do not step over markers elsewhere or through a directive or opaque line.
fn after_tail_markers(store: &LineStore, infos: &[LineInfo], start: usize, hi: usize) -> usize {
    let marker = |k: usize| {
        let info = infos[k];
        (!info.pinned && !info.is_nop() && info.kind != LineKind::InlineAsm)
            .then(|| info.trimmed(store.get(k)))
    };
    if start >= hi || marker(start) != Some("# LCCC_TAIL_CALL") {
        return start;
    }
    let mut k = start + 1;
    while k < hi
        && marker(k).is_some_and(|s| match s {
            "# LCCC_VA_CALL" | "# LCCC_CHAIN_CALL" => true,
            _ => {
                s.strip_prefix("# LCCC_CALL_ARGS ")
                    .or_else(|| s.strip_prefix("# LCCC_CALL_FP "))
                    .is_some_and(|n| n.parse::<u8>().is_ok())
                    || s.strip_prefix("# LCCC_IPA_KEEP 0x")
                        .is_some_and(|n| u16::from_str_radix(n, 16).is_ok())
            }
        })
    {
        k += 1;
    }
    k
}

/// Delete the relays [`thread_function`] orphaned: a run of unreferenced,
/// uniquely defined, unpinned labels (inert comments between them stay) that
/// follows an unconditional transfer and ends in one direct `jmp`. Nothing
/// can reach the run, so dropping it changes no execution, only dead bytes.
/// The ABI comments published on a sibling jump do not re-open fallthrough:
/// stopping at them left dead relays in Linux's `acpi_rs_move_data` and
/// `acpi_ds_exec_end_control_op` that objtool diagnosed as unreachable.
///
/// "Follows an unconditional transfer" is a property of the SECTION, not of the
/// line above it: GAS resumes every section exactly where it left off, so the
/// excursion a switch table makes — `.section .rodata`, the entries, then
/// `.section .text,"ax",@progbits` from `restore_current_text_section` — does
/// not interrupt the text section's control flow. Reading that returning
/// directive as an ordinary non-terminator is what left a relay emitted AFTER
/// the table alive: the entry rewrite had already retired its last reference,
/// so the object file carried a label nothing mentions and no fallthrough can
/// enter, which is precisely objtool's `unreachable instruction` (witness:
/// `nla_validate_int_range` in a 6.18.63 `lib/nlattr.o`, and
/// `tests/regression/switch_lut_bitpack.c` locally).
///
/// So the verdict earned before the excursion is carried across it — gated on
/// section IDENTITY, not on [`SectionKind`]: returning to a *different* code
/// section (`.text` into `.text.hot`) appends after bytes this scan never saw,
/// and only a same-section round trip may reuse the verdict. Nothing is ever
/// restored INTO a non-code section, where no instruction executes and this
/// sweep has no business deleting. One slot suffices because the emitter nests
/// no excursion inside an excursion; anything that could break the same-section
/// proof — `.pushsection`/`.popsection`/`.previous`, `.subsection`, a mode or
/// conditional-assembly directive, inline asm, a pinned line — drops the slot
/// instead of guessing. `entry_section` is the identity in effect at the
/// function's `.cfi_startproc`, tracked by [`run`]; `None` restores nothing.
fn remove_dead_relays(
    store: &LineStore,
    infos: &mut [LineInfo],
    lo: usize,
    hi: usize,
    entry_section: Option<&str>,
) -> bool {
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
        // Section state for THIS scan (see the function documentation): the
        // shared tracker answers code/non-code with GAS's push/pop semantics,
        // `identity` names the section so a round trip can be proven to return
        // to the same one, and `excursion` carries the single verdict a
        // code-to-non-code round trip may restore.
        let mut tracker = SectionTracker::default();
        let mut identity = SectionIdentity::at(entry_section);
        let mut excursion: Option<(String, bool)> = None;
        let mut region: Option<AsmRegionSections> = None;
        let mut k = lo;
        while k < hi && found.is_none() {
            let info = infos[k];
            let s = info.trimmed(store.get(k));
            if info.kind == LineKind::InlineAsm {
                region.get_or_insert_with(Default::default).feed(s);
            } else if region.take().is_some_and(|r| !r.neutral()) {
                identity.forget();
            }
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
            // A section switch this scan can read: carry the verdict across an
            // excursion that returns to the SAME section, forget the proof on
            // anything else.
            // DIRECTIVE-LAW: classifying -- these specific switches update
            // section state; they are not interchangeable with other directives.
            if info.kind == LineKind::Directive
                && !info.pinned
                && may_switch_section(s)
                && let Some(switch) = section_switch(s)
            {
                let leaving = tracker.current();
                let before = identity.current().map(str::to_string);
                tracker.apply(switch);
                let entering = tracker.current();
                let after = identity.apply(switch, s);
                // Restore BEFORE re-arming: the returning switch is itself a
                // switch, and it still has to see what the excursion saved.
                let restored = match (leaving, entering, after, &excursion) {
                    (
                        SectionKind::NonCode,
                        SectionKind::Code,
                        Some(name),
                        Some((from, verdict)),
                    ) if from == name => Some(*verdict),
                    _ => None,
                };
                excursion = match (leaving, entering) {
                    (SectionKind::Code, SectionKind::NonCode) => {
                        before.map(|from| (from, prev_transfers))
                    }
                    _ => None,
                };
                prev_transfers = restored.unwrap_or(false);
                k += 1;
                continue;
            }
            // Everything that can move the next bytes somewhere this scan
            // cannot follow ends the same-section proof. An `#APP` region puts
            // bytes of its own into the section, so no excursion verdict
            // crosses it; whether it also loses the section NAME is decided by
            // reading it ([`AsmRegionSections`], above). `section_proof_ends`
            // names the directives that reshape assembly without naming a
            // section. A pinned INSTRUCTION — a volatile access, a prefixed
            // transfer — is the compiler's own and switches nothing.
            let opaque_directive = (info.pinned || section_proof_ends(s))
                // DIRECTIVE-LAW: bails -- an opaque directive kills the proof.
                && info.kind == LineKind::Directive;
            if opaque_directive {
                identity.forget();
            }
            if opaque_directive || info.kind == LineKind::InlineAsm {
                excursion = None;
            }
            let mnemonic = s.split_whitespace().next().unwrap_or("");
            // `jmpq` is the indirect-dispatch spelling (`jmpq *%rdx`,
            // `jmpq *tbl(,%rax,8)`): unconditional like `jmp`, it never
            // falls through into the line below it.
            prev_transfers = !info.pinned
                && info.kind != LineKind::InlineAsm
                && info.kind != LineKind::Label
                && matches!(mnemonic, "jmp" | "jmpq" | "ret" | "retq" | "ud2");
            k += 1;
            if prev_transfers && matches!(info.kind, LineKind::Jmp | LineKind::JmpIndirect) {
                k = after_tail_markers(store, infos, k, hi);
            }
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
    // The section identity in effect at each function's `.cfi_startproc`, which
    // [`remove_dead_relays`] needs to prove that a switch-table excursion
    // returns to the SAME section. Seeded with `.text`: that is GAS's initial
    // section (`SectionTracker::default` starts there too) and the value
    // `EmitState::current_text_section` is constructed with, so the first
    // function of a TU that never emits an explicit switch still has an
    // identity. `None` means unknown, and unknown restores nothing.
    let mut section = SectionIdentity::default();
    let mut start_section: Option<String> = None;
    let mut region: Option<AsmRegionSections> = None;
    for i in 0..store.len() {
        let info = infos[i];
        let mut endproc = None;
        if info.kind == LineKind::InlineAsm {
            // An `#APP` region: read it, and decide at its end whether it
            // left the section it found (see [`AsmRegionSections`]).
            region
                .get_or_insert_with(Default::default)
                .feed(info.trimmed(store.get(i)));
            continue;
        }
        if region.take().is_some_and(|r| !r.neutral()) {
            // A wrong name would restore a fallthrough verdict into a section
            // that never earned it, so forget the identity instead.
            section.forget();
        }
        if info.pinned {
            // A pinned instruction is the compiler's own (a volatile access,
            // a prefixed transfer) and names no section; a pinned directive
            // is not something this scan can vouch for.
            // DIRECTIVE-LAW: bails -- an opaque directive kills the proof.
            if info.kind == LineKind::Directive {
                section.forget();
            }
        } else if info.kind == LineKind::Directive {
            let s = info.trimmed(store.get(i));
            if s == ".cfi_startproc" {
                start = Some(i + 1);
                start_section = section.current().map(str::to_string);
            } else if s == ".cfi_endproc" {
                // Deferred: `thread_function` below needs `store` mutably.
                endproc = Some(i);
            } else if may_switch_section(s)
                && let Some(switch) = section_switch(s)
            {
                section.apply(switch, s);
            } else if section_proof_ends(s) {
                section.forget();
            }
        }
        // DIRECTIVE-LAW: selecting function-boundary markers, not skipping
        // directives in a motion window (only inert debug attribution is transparent).
        if let Some(hi) = endproc
            && let Some(lo) = start.take()
            && thread_function(store, infos, lo, hi)
        {
            remove_dead_relays(store, infos, lo, hi, start_section.as_deref());
            changed = true;
        }
        if endproc.is_some() {
            start_section = None;
        }
    }
    changed
}

// Test-only whole-TU scan counter. THREAD-LOCAL, not a global atomic: the
// Rust test harness runs cases on independent threads, so a process-wide
// counter is incremented by unrelated tests and the scan-budget assertions
// become flaky (a red-team finding against the first version of this test).
// Zero cost outside unit-test builds.
#[cfg(test)]
std::thread_local! {
    static MENTION_SCAN_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `.LBBn` mention counts plus the set of multiply-defined labels.
type LabelCounts = (FxHashMap<u32, u32>, FxHashSet<u32>);

/// Every `.LBBn` mention count (definitions excluded), plus labels defined
/// more than once. [`label_uses`] answers "mentioned at all"; a conditional
/// tail call deletes the relay, so it needs "mentioned exactly once".
/// Test builds count real (as opposed to cache-verification) scans.
pub(super) fn label_mention_counts(store: &LineStore, infos: &[LineInfo]) -> LabelCounts {
    #[cfg(test)]
    MENTION_SCAN_CALLS.with(|c| c.set(c.get() + 1));
    scan_label_mentions(store, infos)
}

/// The pure scan. Test builds also call it directly to VERIFY a cache hit
/// without perturbing the tracked scan budget.
fn scan_label_mentions(store: &LineStore, infos: &[LineInfo]) -> LabelCounts {
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
        while let Some(at) = crate::backend::peephole_common::find_lbb(rest) {
            rest = &rest[at + 4..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if let Ok(label) = rest[..digits].parse() {
                *count.entry(label).or_insert(0) += 1;
            }
        }
    }
    (count, duplicate)
}

/// Hand out the counts for the CURRENT assembly text. A cache hit is sound
/// only because no rewrite happened since the counts were computed; in test
/// builds EVERY reuse is PROVEN by recomputing and comparing, so no unit test
/// can pass with a stale cache. The verification scan is untracked, keeping
/// the budget assertions a measurement of real work. In release builds this
/// compiles to one `Option::take` plus the scan on a miss -- nothing else.
#[inline]
fn current_counts(
    store: &LineStore,
    infos: &[LineInfo],
    cache: &mut Option<LabelCounts>,
) -> LabelCounts {
    match cache.take() {
        Some(hit) => {
            #[cfg(test)]
            {
                let fresh = scan_label_mentions(store, infos);
                assert_eq!(hit.0, fresh.0, "stale .LBB mention counts reused");
                assert_eq!(hit.1, fresh.1, "stale duplicate-label set reused");
            }
            hit
        }
        None => label_mention_counts(store, infos),
    }
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

/// Phase 8e entry point. Both passes share ONE whole-TU label analysis:
/// `hoist_common_arm_heads_cached` exits its fixpoint loop only on a FAILED
/// attempt, which stores counts for the FINAL text, so the fold can reuse
/// them without a second full scan. Every successful rewrite empties the
/// cache, so no pass ever sees stale counts. Test builds additionally assert
/// cache coherence on every reuse (see `current_counts`).
pub(super) fn run_phase8e(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    do_hoist: bool,
    do_fold: bool,
) -> bool {
    let mut cache: Option<LabelCounts> = None;
    let mut changed = false;
    if do_hoist {
        changed |= hoist_common_arm_heads_cached(store, infos, &mut cache);
    }
    if do_fold {
        changed |= fold_conditional_tail_calls_cached(store, infos, &mut cache);
    }
    changed
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
    let mut cache = None;
    hoist_common_arm_heads_cached(store, infos, &mut cache)
}

/// Shared-cache implementation, used by `run_phase8e` so the following
/// conditional-tail-call pass can reuse the same whole-TU analysis.
fn hoist_common_arm_heads_cached(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    cache: &mut Option<LabelCounts>,
) -> bool {
    let mut changed = false;
    let mut start = None;
    // `label_mention_counts` scans the WHOLE translation unit. Calling it
    // once per candidate attempt made this pass O(F^2) in the number of
    // functions (measured: 4,394,006 `find(".LBB")` calls for only 250
    // straight-line functions). The assembly text changes ONLY when a
    // rewrite succeeds, and `hoist_one_arm_head` is side-effect free on
    // every `continue` path, so a count computed for one state stays exact
    // for every subsequent failed attempt -- including the next function.
    // Cache it across failures and invalidate on the first success; the
    // values seen by each attempt are byte-for-byte what the old code
    // computed, so the emitted assembly cannot change.
    // `cache` is owned by the caller so both phase-8e passes share it.
    for i in 0..store.len() {
        if !infos[i].pinned && infos[i].kind == LineKind::Directive {
            let s = infos[i].trimmed(store.get(i));
            if s == ".cfi_startproc" {
                start = Some(i + 1);
            } else if s == ".cfi_endproc"
                && let Some(lo) = start.take()
            {
                loop {
                    let (count, duplicate) = current_counts(store, infos, cache);
                    if !hoist_one_arm_head(store, infos, lo, i, &count, &duplicate) {
                        *cache = Some((count, duplicate));
                        break;
                    }
                    changed = true;
                    // Text changed: the cache stays empty, so the next
                    // iteration recomputes against the new state.
                }
            }
        }
    }
    changed
}

fn hoist_one_arm_head(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    lo: usize,
    hi: usize,
    count: &FxHashMap<u32, u32>,
    duplicate: &FxHashSet<u32>,
) -> bool {
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
    let mut cache = None;
    fold_conditional_tail_calls_cached(store, infos, &mut cache)
}

fn fold_conditional_tail_calls_cached(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    cache: &mut Option<LabelCounts>,
) -> bool {
    let mut changed = false;
    let mut start = None;
    // Same whole-TU scan cache as `hoist_common_arm_heads`: the text only
    // changes on a successful fold, so counts stay exact across the many
    // functions that have no eligible conditional tail call.
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
                changed |= conditional_tail_calls_in(store, infos, lo, i, cache);
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
    cache: &mut Option<LabelCounts>,
) -> bool {
    let mut changed = false;
    // Recomputed ONLY after a rewrite (a fold removes one mention and one
    // definition, which can only shrink other labels' eligibility). All
    // read-only attempts -- including whole functions with no candidate --
    // reuse the counts for the current assembly state, which are identical
    // to a fresh scan because no line changed.
    loop {
        let (count, duplicate) = current_counts(store, infos, cache);
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
            if mnemonic == "jmp" || duplicate.contains(&label) {
                continue;
            }
            let Some(&d) = defs.get(&label) else {
                continue;
            };
            if count.get(&label) == Some(&1)
                && let Some(plan) = relay_plan(store, infos, lo, hi, d)
            {
                hit = Some((i, mnemonic.to_string(), d, plan));
                break;
            }
            // The mirrored shape: the jcc skips over the sibling call.
            if let Some(negated) = mnemonic
                .strip_prefix('j')
                .and_then(super::flag_peepholes::negate_cc)
                && let Some(q) = skipped_sibling_call(store, infos, i, d, &count, &duplicate)
            {
                hit = Some((i, format!("j{negated}"), q, (q, d)));
                break;
            }
        }
        let Some((i, mnemonic, d, (q, end))) = hit else {
            *cache = Some((count, duplicate));
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

/// The mirrored conditional sibling call: a `jcc` on line `i` whose target
/// label (line `d`) directly follows a codegen-marked sibling `jmp` on the
/// fall-through path becomes the negated `jcc` to the callee:
///
/// ```text
///     btl %edi, %ecx                  btl %edi, %ecx
///     jnc .LBB2                       jc hit@PLT
/// .LBB1:                      ->      # LCCC_TAIL_CALL
///     jmp hit@PLT                 .LBB1:
///     # LCCC_TAIL_CALL            .LBB2:
/// .LBB2:                              ret
///     ret
/// ```
///
/// Clang 23.1.4 emits exactly this (`jb hit@PLT` for the dav1d
/// `wedge_allowed_mask` test); GCC 16.2 keeps the jump over the call.  No
/// instruction separates the `jcc` from the `jmp`, so registers, flags,
/// memory and stack depth agree at both; deleting the `jmp` makes the taken
/// path of the old `jcc` the fall-through of the new one, which reaches the
/// same label through byteless lines only.  Every label between the two
/// branches must be unreferenced (nothing else may enter the deleted call),
/// and the `jmp`'s marker block must end at the target label itself.
/// Returns the line of the sibling `jmp`.
fn skipped_sibling_call(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    d: usize,
    count: &FxHashMap<u32, u32>,
    duplicate: &FxHashSet<u32>,
) -> Option<usize> {
    if d <= i {
        return None;
    }
    let mut q = i + 1;
    while q < d {
        if byteless(store, infos, q) {
            q += 1;
            continue;
        }
        if infos[q].kind != LineKind::Label || infos[q].pinned {
            break;
        }
        let label = local_label(label_text(infos[q].trimmed(store.get(q))))?;
        if duplicate.contains(&label) || count.get(&label).copied().unwrap_or(0) != 0 {
            return None;
        }
        q += 1;
    }
    let info = infos[q];
    if q >= d || info.pinned || info.kind != LineKind::Jmp {
        return None;
    }
    let mut fields = info.trimmed(store.get(q)).split_whitespace();
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
    let end = (q + 1..d)
        .find(|&k| !byteless(store, infos, k))
        .unwrap_or(d);
    (end == d).then_some(q)
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
                if store.get(i).trim_ascii() == line {
                    info.pinned = true;
                }
            }
        }
        let changed = run(&mut store, &mut infos);
        let text = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim_ascii())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!run(&mut store, &mut infos), "must reach a fixpoint");
        (changed, text)
    }

    /// Build a translation unit with `fns` straight-line procedures. None
    /// contains a conditional jump, so neither pass can rewrite anything and
    /// the assembly state is constant for the whole pass.
    fn straight_line_tu(fns: usize) -> (LineStore, Vec<LineInfo>) {
        let mut text = String::new();
        for f in 0..fns {
            text.push_str(&format!(
                "f{f}:\n.cfi_startproc\nmovl $0, %eax\nret\n.cfi_endproc\n"
            ));
        }
        let store = LineStore::new(text);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        (store, infos)
    }

    #[test]
    fn hoist_heads_does_not_rescan_per_function() {
        let (mut store, mut infos) = straight_line_tu(64);
        MENTION_SCAN_CALLS.with(|c| c.set(0));
        assert!(!hoist_common_arm_heads(&mut store, &mut infos));
        let scans = MENTION_SCAN_CALLS.with(|c| c.get());
        // Before the cache this was exactly 64 whole-TU scans (one per
        // function) and grew quadratically in the line count. The text never
        // changed, so one scan is the only correct-value computation needed.
        // Exact equality, not `<= 1`: a second scan is a real regression.
        assert_eq!(scans, 1, "whole-TU scans for 64 unchanged functions");
    }

    #[test]
    fn conditional_tail_calls_do_not_rescan_per_function() {
        let (mut store, mut infos) = straight_line_tu(64);
        MENTION_SCAN_CALLS.with(|c| c.set(0));
        assert!(!fold_conditional_tail_calls(&mut store, &mut infos));
        let scans = MENTION_SCAN_CALLS.with(|c| c.get());
        assert_eq!(scans, 1, "whole-TU scans for 64 unchanged functions");
    }

    /// Regression for the red-team finding that a process-wide AtomicUsize
    /// makes these budgets flaky: `cargo test` runs cases on independent
    /// threads, so an unrelated test calling `label_mention_counts` used to
    /// increment THIS test's counter. A thread-local counter cannot.
    #[test]
    fn scan_counter_is_thread_scoped() {
        MENTION_SCAN_CALLS.with(|c| c.set(0));
        let handle = std::thread::spawn(|| {
            let (store, infos) = straight_line_tu(2);
            for _ in 0..8 {
                let _ = label_mention_counts(&store, &infos);
            }
        });
        handle.join().expect("scan thread panicked");
        assert_eq!(
            MENTION_SCAN_CALLS.with(|c| c.get()),
            0,
            "an unrelated test thread perturbed this thread's scan budget"
        );
    }

    /// The two phase-8e passes now share ONE analysis. On an unchanged TU
    /// the old code scanned the whole translation unit twice; one scan is
    /// now sufficient and sufficient is provably exact (`current_counts`
    /// re-verifies every reuse in test builds).
    #[test]
    fn phase8e_shares_one_whole_tu_scan() {
        let (mut store, mut infos) = straight_line_tu(64);
        MENTION_SCAN_CALLS.with(|c| c.set(0));
        assert!(!run_phase8e(&mut store, &mut infos, true, true));
        assert_eq!(
            MENTION_SCAN_CALLS.with(|c| c.get()),
            1,
            "both passes together must scan the TU exactly once"
        );
        // Disabling either pass must not silently enable the other or scan.
        MENTION_SCAN_CALLS.with(|c| c.set(0));
        assert!(!run_phase8e(&mut store, &mut infos, false, false));
        assert_eq!(MENTION_SCAN_CALLS.with(|c| c.get()), 0);
    }

    /// A successful rewrite MUST invalidate the cache. Build a TU whose
    /// middle function has identical conditional arm heads; the other
    /// functions are straight-line and exercise cache reuse across
    /// function boundaries before and after the rewrite.
    #[test]
    fn a_successful_rewrite_invalidates_the_cache() {
        let mut text = String::new();
        for f in 0..4 {
            text.push_str(&format!(
                "f{f}:\n.cfi_startproc\nmovl $0, %eax\nret\n.cfi_endproc\n"
            ));
            if f == 1 {
                text.push_str(
                    "g:\n.cfi_startproc\n    testl %edi, %edi\n    je .LBB5\n\
                     .LBB4:\n    movq %rsi, %rdi\n    jmp g1@PLT\n.LBB5:\n\
                     movq %rsi, %rdi\n    jmp g2@PLT\n.cfi_endproc\n",
                );
            }
        }
        let store = LineStore::new(text);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        let mut store = store;
        MENTION_SCAN_CALLS.with(|c| c.set(0));
        assert!(hoist_common_arm_heads(&mut store, &mut infos));
        let scans = MENTION_SCAN_CALLS.with(|c| c.get());
        assert!(
            scans >= 2,
            "a rewrite must force a recomputation against the new text: {scans}"
        );
        // Every reuse on the surrounding functions was verified by
        // `current_counts`; a stale cache would have panicked above.
    }

    #[test]
    fn cached_scans_still_reach_a_fixpoint() {
        // `apply` asserts a second `run` is a no-op. A stale cache would keep
        // rewriting (or stop too early) and break that invariant on any of
        // these golden bodies; guard it explicitly with a rewrite-heavy one.
        let body = "jne .LBB1 # edge\nret\n.LBB1:\n# harmless\n.LBB2: # alias\njmp .LBB3\n.LBB3:\njmp .LBB4\n.LBB4:\nadcq $0, %rax\nret";
        let (changed, out) = apply(body, None);
        assert!(changed, "expected a rewrite: {out}");
        assert!(
            !out.contains(".LBB1\n"),
            "stale cache left an orphan: {out}"
        );
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
    fn a_tail_jumps_own_markers_do_not_reopen_fallthrough() {
        let tail = format!(
            "# {COMPILER_MARKER}TAIL_CALL\n# {COMPILER_MARKER}CALL_ARGS 2\n# {COMPILER_MARKER}CALL_FP 0"
        );
        let body = |jump: &str, markers: &str| {
            format!("jle .LBB3\n{jump}\n{markers}\n.LBB3:\njmp .LBB2\n.LBB2:\nret")
        };
        let direct = body("jmp hook@PLT", &tail);
        let (_, out) = apply(&direct, None);
        assert!(!out.contains(".LBB3"), "orphan should be gone: {out}");
        assert!(
            out.contains(&format!("jle .LBB2\njmp hook@PLT\n{tail}")),
            "{out}"
        );
        for (jump, extra) in [
            ("je hook@PLT", ""), // a conditional tail call DOES fall through
            ("jmp hook@PLT", "\n.cfi_restore_state"),
            ("jmp hook@PLT", "\n.p2align 4"),
        ] {
            assert!(
                apply(&body(jump, &format!("{tail}{extra}")), None)
                    .1
                    .contains(".LBB3:")
            );
        }
        let unknown = format!("{tail}\n# {COMPILER_MARKER}UNKNOWN");
        assert!(
            apply(&body("jmp hook@PLT", &unknown), None)
                .1
                .contains(".LBB3:")
        );
        let pinned = format!("# {COMPILER_MARKER}CALL_ARGS 2");
        assert!(apply(&direct, Some(&pinned)).1.contains(".LBB3:"));
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

    /// The sqlite_vdbe_serial witness: a shared-body switch case dispatches
    /// through a relay only the TABLE references, so the branch loop cannot
    /// retire it — the entry itself must thread. Absolute (`.quad`) form; the
    /// bypassed relay then orphans and the sweep removes it (the indirect
    /// `jmpq *` above it never falls through).
    #[test]
    fn absolute_table_entries_thread_through_relays() {
        let body = concat!(
            "jmpq *.Ljt_0(,%rax,8)\n",
            ".LBB1:\n",
            "jmp .LBB2\n",
            ".LBB2:\n",
            "ret\n",
            ".section .rodata\n",
            ".align 8\n",
            ".Ljt_0:\n",
            ".quad .LBB1\n",
            ".quad .LBB2\n",
            ".section .text\n",
        );
        let (changed, out) = apply(body, None);
        assert!(changed, "{out}");
        assert!(!out.contains(".quad .LBB1"), "entry threaded:\n{out}");
        assert_eq!(out.matches(".quad .LBB2").count(), 2, "{out}");
        assert!(!out.contains(".LBB1:"), "orphaned relay swept:\n{out}");
    }

    /// The PIC (`.long target - table`) spelling threads identically and
    /// reproduces the base symbol verbatim.
    #[test]
    fn pic_table_entries_thread_through_relays() {
        let body = concat!(
            "leaq .Ljt_0(%rip), %rcx\n",
            "movslq (%rcx,%rax,4), %rdx\n",
            "addq %rcx, %rdx\n",
            "jmpq *%rdx\n",
            ".LBB1:\n",
            "jmp .LBB2\n",
            ".LBB2:\n",
            "ret\n",
            ".section .rodata\n",
            ".align 4\n",
            ".Ljt_0:\n",
            ".long .LBB1 - .Ljt_0\n",
            ".long .LBB2 - .Ljt_0\n",
            ".section .text\n",
        );
        let (changed, out) = apply(body, None);
        assert!(changed, "{out}");
        assert!(!out.contains(".long .LBB1"), "entry threaded:\n{out}");
        assert_eq!(out.matches(".long .LBB2 - .Ljt_0").count(), 2, "{out}");
    }

    /// The witness for the section-scoped verdict. `switch_lut_bitpack.c` in the
    /// regression corpus and `nla_validate_int_range` in a 6.18.63 `lib/nlattr.o`
    /// both put the relay AFTER the table, so the line above it is
    /// `restore_current_text_section`'s `.section .text,"ax",@progbits`. Read as
    /// an ordinary directive that clears `prev_transfers`, the orphaned relay
    /// survives into the object file as a label nothing mentions and no
    /// fallthrough can enter: objtool's `unreachable instruction`. GAS resumes
    /// `.text` where the dispatch left it and `jmpq *%rdx` never falls through,
    /// so the verdict the excursion carried is the true one.
    #[test]
    fn relay_after_the_table_excursion_is_swept() {
        let body = concat!(
            "jmpq *%rdx\n",
            ".section .rodata\n",
            ".align 4\n",
            ".Ljt_0:\n",
            ".long .LBB1 - .Ljt_0\n",
            ".long .LBB2 - .Ljt_0\n",
            ".section .text,\"ax\",@progbits\n",
            ".LBB1:\n",
            "jmp .LBB2\n",
            ".LBB2:\n",
            "movl $42, %eax\n",
            "ret\n",
        );
        let (changed, out) = apply(body, None);
        assert!(changed, "{out}");
        assert!(!out.contains(".long .LBB1"), "entry threaded:\n{out}");
        assert!(
            !out.contains(".LBB1:"),
            "relay behind the section return swept:\n{out}"
        );
        // The returning directive is not part of the relay: what follows the
        // table still assembles into `.text`.
        assert!(out.contains(".section .text,\"ax\",@progbits"), "{out}");
        assert!(out.contains("movl $42, %eax"), "{out}");
    }

    /// The same shape with a predecessor that DOES fall through. The verdict
    /// carried across the excursion has to be the one the text section earned:
    /// `addq` falls into `.LBB1`, so retiring the relay would remove the only
    /// path that fallthrough has \u2014 a miscompile. This is the assertion that
    /// makes the identity gate load-bearing instead of decorative.
    #[test]
    fn fallthrough_before_the_excursion_keeps_the_relay() {
        let body = concat!(
            "addq $1, %rax\n",
            ".section .rodata\n",
            ".Ljt_0:\n",
            ".long .LBB1 - .Ljt_0\n",
            ".long .LBB2 - .Ljt_0\n",
            ".section .text,\"ax\",@progbits\n",
            ".LBB1:\n",
            "jmp .LBB2\n",
            ".LBB2:\n",
            "ret\n",
        );
        let (_, out) = apply(body, None);
        assert!(
            out.contains(".LBB1:"),
            "reachable relay must survive:\n{out}"
        );
        assert!(out.contains("jmp .LBB2"), "{out}");
        assert!(
            !out.contains(".long .LBB1"),
            "the entry itself still threads:\n{out}"
        );
    }

    /// Returning to a DIFFERENT code section appends after bytes this scan never
    /// saw, so the verdict stays behind in `.text` and the relay survives.
    #[test]
    fn a_different_code_section_does_not_inherit_the_verdict() {
        let body = concat!(
            "jmpq *%rdx\n",
            ".section .rodata\n",
            ".Ljt_0:\n",
            ".long .LBB1 - .Ljt_0\n",
            ".long .LBB2 - .Ljt_0\n",
            ".section .text.hot,\"ax\"\n",
            ".LBB1:\n",
            "jmp .LBB2\n",
            ".LBB2:\n",
            "ret\n",
        );
        let (_, out) = apply(body, None);
        assert!(
            out.contains(".LBB1:"),
            "verdict must not cross sections:\n{out}"
        );
    }

    /// `.text` IS `.section .text,"ax",@progbits` \u2014 one section, two spellings,
    /// and the emitter uses both (`ipa_ra` the bare one,
    /// `restore_current_text_section` the full one), so the identity has to be
    /// the name rather than the line.
    #[test]
    fn both_text_spellings_are_one_identity() {
        for ret in [".text", ".section .text,\"ax\",@progbits"] {
            let body = format!(
                "jmpq *%rdx\n.section .rodata\n.Ljt_0:\n.long .LBB1 - .Ljt_0\n\
                 .long .LBB2 - .Ljt_0\n{ret}\n.LBB1:\njmp .LBB2\n.LBB2:\nret\n"
            );
            let (_, out) = apply(&body, None);
            assert!(!out.contains(".LBB1:"), "return via {ret}:\n{out}");
        }
    }

    /// `.pushsection`/`.popsection` restore what GAS restores \u2014 the section AND
    /// the position within it \u2014 so the verdict survives the round trip, in both
    /// directions.
    #[test]
    fn pushsection_round_trip_carries_the_verdict_both_ways() {
        let tail = concat!(
            ".pushsection .data\n",
            ".quad 1\n",
            ".popsection\n",
            ".LBB1:\n",
            "jmp .LBB2\n",
            ".LBB2:\n",
            "ret\n",
        );
        // The `jne .LBB1` is what makes the sweep run at all: it threads to
        // `.LBB2`, which both orphans the relay and is the "changed" `run`
        // requires before it sweeps — `thread_function`'s contract is that the
        // only deletion is a relay the retargeting orphaned.
        let (_, out) = apply(&format!("jne .LBB1\nret\n{tail}"), None);
        assert!(out.contains("jne .LBB2"), "threaded:\n{out}");
        assert!(!out.contains(".LBB1:"), "`ret` cannot fall through:\n{out}");
        let (_, out) = apply(&format!("jne .LBB1\naddq $1, %rax\n{tail}"), None);
        assert!(out.contains(".LBB1:"), "`addq` falls through:\n{out}");
    }

    /// An `#APP` region between the dispatch and the table ends the proof:
    /// `pin_inline_asm_regions` reclassifies and pins every line of it, so a
    /// `.pushsection` with label arithmetic inside one (which kernel asm does
    /// use) is invisible here, and the markers themselves are barriers.
    #[test]
    fn inline_asm_between_dispatch_and_table_ends_the_proof() {
        let body = concat!(
            "jmpq *%rdx\n",
            "#APP\n",
            "    .pushsection .note.lccc\n",
            "#NO_APP\n",
            ".section .rodata\n",
            ".Ljt_0:\n",
            ".long .LBB1 - .Ljt_0\n",
            ".long .LBB2 - .Ljt_0\n",
            ".section .text,\"ax\",@progbits\n",
            ".LBB1:\n",
            "jmp .LBB2\n",
            ".LBB2:\n",
            "ret\n",
        );
        let (_, out) = apply(body, None);
        assert!(
            out.contains(".LBB1:"),
            "opaque text must end the proof:\n{out}"
        );
    }

    /// The kernel shape this sweep was inert on: a `WARN` (`ud2` and its
    /// `__bug_table` entry) ahead of the dispatch, in the same function. The
    /// region pushes and pops, so the section after it is the section before
    /// it, and the relay behind the table goes (objtool: `unreachable
    /// instruction` in `ftrace_bug` and three more of a 6.18.63 build).
    const BALANCED_REGION: &str = concat!(
        "#APP\n",
        "    1:\t ud2\n",
        "    .pushsection __bug_table,\"aw\"\n",
        "    2:\t.long 1b - .\t# bug_entry::bug_addr\n",
        "    .word 2307\t# bug_entry::flags\n",
        "    .org 2b + 8\n",
        "    .popsection\n",
        "    .pushsection .discard.annotate_insn,\"M\",@progbits,8 ; .long 1b - . ; .popsection\n",
        "#NO_APP\n",
    );
    const DISPATCH_AND_RELAY: &str = concat!(
        "jne .LBB1\n",
        "jmpq *.Ljt_0(,%rax,8)\n",
        ".section .rodata\n",
        ".Ljt_0:\n",
        ".quad .LBB1\n",
        ".quad .LBB2\n",
        ".section .text,\"ax\",@progbits\n",
        ".LBB1:\n",
        "jmp .LBB2\n",
        ".LBB2:\n",
        "ret\n",
    );

    #[test]
    fn a_balanced_asm_region_keeps_the_section() {
        let (changed, out) = apply(&format!("{BALANCED_REGION}{DISPATCH_AND_RELAY}"), None);
        assert!(changed, "{out}");
        assert!(!out.contains(".LBB1:"), "relay swept:\n{out}");
        assert!(out.contains("jne .LBB2"), "{out}");
        assert_eq!(out.matches(".quad .LBB2").count(), 2, "{out}");
        // The region itself reaches the assembler byte for byte.
        for line in BALANCED_REGION.lines() {
            assert!(out.contains(line.trim_ascii()), "{line:?} kept:\n{out}");
        }
    }

    /// The same across functions: the region is in the function before, and
    /// nothing names a section in between (`.text` functions follow each other
    /// without a directive).
    #[test]
    fn a_balanced_asm_region_in_an_earlier_function_keeps_the_section() {
        let text = format!(
            ".text\ng:\n.cfi_startproc\n{BALANCED_REGION}ret\n.cfi_endproc\n\
             f:\n.cfi_startproc\n{DISPATCH_AND_RELAY}.cfi_endproc\n"
        );
        let mut store = LineStore::new(text);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        assert!(run(&mut store, &mut infos));
        let out = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim_ascii())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!out.contains(".LBB1:"), "relay swept:\n{out}");
    }

    /// A pinned instruction (a volatile access, a prefixed transfer) is the
    /// compiler's own text and names no section. It used to end the proof, in
    /// every function that reads or writes a `volatile`.
    #[test]
    fn a_pinned_instruction_keeps_the_section() {
        let body = format!("movl (%rdi), %eax\n{DISPATCH_AND_RELAY}");
        let (_, out) = apply(&body, Some("movl (%rdi), %eax"));
        assert!(!out.contains(".LBB1:"), "relay swept:\n{out}");
    }

    /// Each way a region can leave another section current, or hide what it
    /// does, still ends the proof: the relay stays.
    #[test]
    fn an_asm_region_that_may_leave_another_section_ends_the_proof() {
        for region in [
            // Pushed and never popped.
            "    .pushsection .text.other,\"ax\"\n",
            // Popped without a push: whatever was pushed before the function.
            "    .popsection\n",
            // Switched and never switched back.
            "    .section .text.other,\"ax\"\n",
            "    .text 1\n",
            // Two switches: `.previous` returns to the first, not to `.text`.
            "    .section .a ; .section .b ; .previous\n",
            // `.previous` with nothing to undo swaps to an unknown section.
            "    .previous\n",
            // Another chunk of the same section.
            "    .subsection 1\n",
            // Conditional assembly and macros can hide or repeat a switch.
            "    .if 1\n    .pushsection .a\n    .endif\n    .popsection\n",
            "    .macro m\n    .endm\n",
            // A quote that does not close: the statements are unknowable.
            "    .pushsection \"unterminated\n    .popsection\n",
            // Case does not hide a pseudo-op from GAS, so not from this either.
            "    .PUSHSECTION .a\n",
        ] {
            let body = format!("#APP\n{region}#NO_APP\n{DISPATCH_AND_RELAY}");
            let (_, out) = apply(&body, None);
            assert!(
                out.contains(".LBB1:"),
                "{region:?} must end the proof:\n{out}"
            );
            assert!(
                out.contains("jne .LBB2"),
                "the branch still threads:\n{out}"
            );
        }
    }

    /// The statement reader on its own: separators, labels, comments, quotes
    /// and nesting, each decided the way GAS decides it.
    #[test]
    fn asm_region_sections_reads_statements_like_gas() {
        let neutral = |text: &str| {
            let mut region = AsmRegionSections::default();
            for line in text.lines() {
                region.feed(line);
            }
            region.neutral()
        };
        for text in [
            "",
            "nop",
            "1: .pushsection .a ; .long 1b - . ; .popsection",
            ".pushsection .a\n.section .b\n.previous\n.subsection 2\n.popsection",
            ".pushsection .a\n.pushsection .b\n.popsection\n.popsection",
            ".section .a ; .previous",
            ".section \".export_symbol\", \"a\" ; sym: ; .asciz \"GPL\" ; .previous",
            ".text ; .previous",
            // `;` and `#` inside a string separate nothing; a comment hides
            // the rest of its line only.
            ".pushsection .a ; .asciz \"x ; .section y # z\" ; .popsection",
            "nop # .section .hidden",
            ".pushsection .a # .popsection\n.popsection",
            // A label is not a directive, and a macro call is not visible.
            ".Lsection: nop",
            "extable_type_reg type=1, reg=%rax",
        ] {
            assert!(neutral(text), "{text:?} leaves the section it found");
        }
        for text in [
            ".pushsection .a",
            ".popsection",
            ".section .a",
            ".section .a ; .section .b ; .previous",
            ".section .a ; .previous ; .previous",
            ".data",
            ".bss",
            ".subsection 1",
            ".pushsection .a ; .popsection ; .popsection",
            "1: 2: .section .a",
            ".ifdef X\n.endif",
            ".rept 2\n.endr",
            ".asciz \"open",
        ] {
            assert!(!neutral(text), "{text:?} may leave another section");
        }
    }

    /// `.subsection` moves to another chunk of the SAME section: the chunk the
    /// next bytes land in is not the one that earned the verdict.
    #[test]
    fn subsection_ends_the_proof() {
        let body = concat!(
            "jmpq *%rdx\n",
            ".subsection 1\n",
            ".section .rodata\n",
            ".Ljt_0:\n",
            ".long .LBB1 - .Ljt_0\n",
            ".long .LBB2 - .Ljt_0\n",
            ".section .text,\"ax\",@progbits\n",
            ".LBB1:\n",
            "jmp .LBB2\n",
            ".LBB2:\n",
            "ret\n",
        );
        let (_, out) = apply(body, None);
        assert!(
            out.contains(".LBB1:"),
            "subsection must end the proof:\n{out}"
        );
    }

    #[test]
    fn section_identity_normalises_spellings_and_keeps_chunks_apart() {
        for (text, want) in [
            (".text", Some(".text")),
            (".section .text", Some(".text")),
            (".section .text,\"ax\",@progbits", Some(".text")),
            (".section .text,\"axR\",@progbits", Some(".text")),
            ("  .section .rodata  ", Some(".rodata")),
            (
                ".section .rodata.str1.1,\"aMS\",@progbits,1",
                Some(".rodata.str1.1"),
            ),
            (".data", Some(".data")),
            (".bss", Some(".bss")),
            (".pushsection .data", Some(".data")),
            // Another chunk of the same section is another identity.
            (".text 1", Some(".text 1")),
            // No name to read, or not a name at all.
            (".popsection", None),
            (".previous", None),
            (".section", None),
            (".section text", None),
            (".section .text 1", None),
            (".pushsection .text,1,\"ax\",@progbits", None),
            (".section .text,\"axG\",@progbits,grp,comdat", None),
            (".section .text,\"ax?\",@progbits", None),
            (".section .text,\"ax\",@progbits,unique,7", None),
            (".quad .LBB1", None),
            (".align 4", None),
            (".rodata", None),
        ] {
            assert_eq!(section_identity(text), want, "{text}");
        }
        // The two parsers must agree: every spelling `section_switch` reports as
        // entering a section has an identity, and `.rodata` bare is one GAS
        // rejects (measured on binutils 2.47 `as`), so neither accepts it.
        for text in [
            ".text",
            ".data",
            ".bss",
            ".section .rodata",
            ".pushsection .data",
        ] {
            assert!(
                section_switch(text).is_some() && section_identity(text).is_some(),
                "{text}"
            );
        }
        for text in [".rodata", ".tdata", ".loc 1 1", ".quad 0"] {
            assert!(section_switch(text).is_none(), "{text}");
        }
    }

    #[test]
    fn section_proof_ends_names_the_invisible_switches() {
        for ends in [
            ".if 0",
            ".else",
            ".endif",
            ".ifdef X",
            ".ifc 1,1",
            ".include \"another.s\"",
            ".ifndef X",
            ".macro m",
            ".endm",
            ".rept 4",
            ".irp x,1",
            ".irpc x,ab",
            ".altmacro",
            ".subsection 2",
            ".code16",
            ".code32",
            ".intel_syntax noprefix",
            ".att_syntax prefix",
            ".arch i386",
        ] {
            assert!(section_proof_ends(ends), "{ends}");
        }
        // Data, alignment and unwind directives are the switch table's own
        // furniture: ending the proof on them would undo the whole point.
        for keeps in [
            ".quad .LBB1",
            ".long .LBB2 - .Ljt_0",
            ".byte 0",
            ".align 4",
            ".p2align 4,,10",
            ".ascii \"x\"",
            ".cfi_def_cfa_offset 16",
            ".loc 1 7 3",
            ".text",
            ".section .rodata",
        ] {
            assert!(!section_proof_ends(keeps), "{keeps}");
        }
    }

    #[test]
    fn may_switch_section_filters_on_one_byte() {
        for yes in [
            ".text",
            ".data",
            ".bss",
            ".section .rodata",
            ".pushsection .data",
            ".popsection",
            ".previous",
            "  .text",
        ] {
            assert!(may_switch_section(yes), "{yes}");
        }
        // The directives that dominate a real TU must not reach the parser.
        for no in [
            ".loc 1 7 3",
            ".cfi_startproc",
            ".cfi_def_cfa_offset 16",
            ".quad 0",
            ".long 1",
            ".align 4",
            ".globl f",
            ".ident \"x\"",
            ".file 1 \"x.c\"",
            "jmp .LBB1",
            "",
            ".",
        ] {
            assert!(!may_switch_section(no), "{no}");
        }
    }

    /// A retarget that lands a branch on the label it already falls through to
    /// leaves a no-op branch behind.  Witness: `tests/regression/loop_global_promote.c`
    /// at -O2 carries `    jmp .LBB27` directly above `.LBB27:` after the
    /// peepholes and no such jump before them (measured on that TU with
    /// `LCCC_NO_PEEPHOLE=1`: 0 against 2), so the retargeting created it and
    /// `eliminate_redundant_jumps` had run three phases earlier.  Both spellings
    /// go: a `jmp` is unconditional, and a `jcc` whose two outcomes both arrive
    /// at the next instruction cannot be observed.
    #[test]
    fn a_retarget_onto_the_next_label_deletes_the_branch() {
        for head in ["jmp", "jne", "jb", "jge"] {
            let body = format!(
                "jb .LBB21\n{head} .LBB9\n.LBB27:\nret\n.LBB21:\nret\n.LBB9:\njmp .LBB27\n"
            );
            let (changed, out) = apply(&body, None);
            assert!(changed, "{head}: {out}");
            assert_eq!(
                out, "f:\n.cfi_startproc\njb .LBB21\n.LBB27:\nret\n.LBB21:\nret\n.cfi_endproc",
                "{head}: the no-op branch and the relay it targeted both go"
            );
        }
    }

    /// Stacked labels are transparent to fallthrough: arriving at `.LBB26:` and
    /// walking into `.LBB27:` executes the same instructions as jumping to
    /// `.LBB27`, and `.LBB26` keeps its own references either way.  This is the
    /// shape `eliminate_redundant_jumps` breaks on (it compares only the first
    /// label it finds), and 13 of the 24 survivors in a 1335-file census of the
    /// regression corpus are exactly this.
    #[test]
    fn a_retarget_onto_a_label_behind_stacked_labels_deletes_the_branch() {
        let body = "jmp .LBB9\n.LBB26:\n.LBB27:\nret\n.LBB9:\njmp .LBB27\n";
        let (changed, out) = apply(body, None);
        assert!(changed, "{out}");
        assert_eq!(
            out, "f:\n.cfi_startproc\n.LBB26:\n.LBB27:\nret\n.cfi_endproc",
            "both labels stay, the no-op branch goes:\n{out}"
        );
    }

    /// Byteless debug attribution between the branch and its target emits
    /// nothing, so the branch is still a no-op — and the marker itself is not
    /// part of the deletion.
    #[test]
    fn a_debug_marker_between_branch_and_target_is_transparent() {
        for marker in [".loc 1 7 3", r#".file 2 "x.c""#] {
            let body = format!("jmp .LBB9\n{marker}\n.LBB27:\nret\n.LBB9:\njmp .LBB27\n");
            let (_, out) = apply(&body, None);
            assert!(out.contains(marker), "{marker}: {out}");
            assert!(
                !out.contains("jmp .LBB27\n"),
                "no-op branch deleted:\n{out}"
            );
            assert!(!out.contains(".LBB9:"), "relay swept:\n{out}");
        }
    }

    /// Anything that emits a byte between the branch and the target makes the
    /// branch load-bearing, so it is rewritten, never deleted.
    #[test]
    fn a_retarget_past_emitting_lines_rewrites_instead_of_deleting() {
        let body = "jmp .LBB9\naddq $1, %rax\n.LBB27:\nret\n.LBB9:\njmp .LBB27\n";
        let (_, out) = apply(body, None);
        assert!(out.contains("jmp .LBB27\naddq $1, %rax"), "{out}");
        assert!(out.contains(".LBB27:"), "{out}");
        // A data directive is bytes too, and alignment padding is a fill.
        for between in [".quad 0", ".byte 0x90", ".p2align 4,0xf8"] {
            let body = format!("jmp .LBB9\n{between}\n.LBB27:\nret\n.LBB9:\njmp .LBB27\n");
            let (_, out) = apply(&body, None);
            assert!(out.contains("jmp .LBB27"), "{between}: {out}");
            assert!(out.contains(between), "{between}: {out}");
        }
        // Inline asm between the two is opaque and pinned.
        let body = "jmp .LBB9\n#APP\n    nop\n#NO_APP\n.LBB27:\nret\n.LBB9:\njmp .LBB27\n";
        let (_, out) = apply(body, None);
        assert!(out.contains("jmp .LBB27"), "{out}");
    }

    /// A compiler-owned marker on the branch line has to reach the assembler, so
    /// such a line is rewritten with its comment rather than deleted.  Built from
    /// the constant `inert_comment` matches, so the fixture cannot drift from the
    /// rule.
    #[test]
    fn a_marker_comment_on_the_branch_is_rewritten_not_deleted() {
        let marked = format!("jmp .LBB9 # {COMPILER_MARKER}keep");
        let body = format!("{marked}\n.LBB27:\nret\n.LBB9:\njmp .LBB27\n");
        let (_, out) = apply(&body, None);
        assert!(
            out.contains(&format!("jmp .LBB27 # {COMPILER_MARKER}keep")),
            "{out}"
        );
        // An ordinary human comment carries no contract, so that branch goes.
        let body = "jmp .LBB9 # edge\n.LBB27:\nret\n.LBB9:\njmp .LBB27\n";
        let (_, out) = apply(body, None);
        assert!(!out.contains("jmp .LBB9"), "{out}");
    }

    /// A `.quad .LBBn` outside a `.Ljt_*` run is somebody's data — a
    /// computed-goto array whose element values a program may compare — and
    /// must never be rewritten. A non-entry line inside a table closes the
    /// run for the same reason.
    #[test]
    fn table_entry_threading_fails_closed() {
        // No .Ljt label anywhere: the data mention pins the relay, nothing
        // changes.
        let body = "jmp .LBB2\n.LBB1:\njmp .LBB2\n.LBB2:\nret\n.quad .LBB1\n";
        let (_, out) = apply(body, None);
        assert!(out.contains(".quad .LBB1"), "{out}");
        // A foreign directive inside the run closes it: the entry after it
        // keeps its target.
        let body = concat!(
            "jmpq *.Ljt_0(,%rax,8)\n",
            ".LBB1:\n",
            "jmp .LBB2\n",
            ".LBB2:\n",
            "ret\n",
            ".Ljt_0:\n",
            ".byte 0\n",
            ".quad .LBB1\n",
        );
        let (_, out) = apply(body, None);
        assert!(out.contains(".quad .LBB1"), "{out}");
        // An entry difference against a non-.Ljt base is not a table entry.
        assert_eq!(table_entry_target(".long .LBB1 - .LBB2"), None);
        assert_eq!(table_entry_target(".quad .LBB1 + 8"), None);
        assert_eq!(table_entry_target(".quad other"), None);
        assert_eq!(
            table_entry_target(".long .LBB7 - .Ljt_3"),
            Some((7, Some(".Ljt_3")))
        );
        assert_eq!(table_entry_target(".quad .LBB7"), Some((7, None)));
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
            // The relay itself stays verbatim.  (The `je .LBB2` over the
            // marked `jmp fast@PLT` may still become `jne fast@PLT` -- the
            // mirrored fold -- which leaves `.LBB2` and its relay intact.)
            let (_, out) = apply(&dispatch(&relay));
            assert!(out.contains(&format!(".LBB2:\n{relay}")), "{relay}:\n{out}");
        }
    }

    #[test]
    fn shared_fallthrough_or_mentioned_relays_stay() {
        // Fall-through entry: the relay is reachable without the jcc.
        let (c, out) = apply(&format!(
            "    testl %edi, %edi\n    je .LBB2\n    call g@PLT\n.LBB2:\n    jmp slow@PLT\n    {T}\n"
        ));
        assert!(!c, "{out}");
        // Two branches to the relay: the relay stays (the `jne` over the
        // marked fast call is the mirrored fold's business).
        let (_, out) = apply(&format!(
            "    testl %edi, %edi\n    je .LBB2\n    testl %esi, %esi\n    jne .LBB2\n    jmp fast@PLT\n    {T}\n.LBB2:\n    jmp slow@PLT\n    {T}\n"
        ));
        assert!(
            out.contains("    je .LBB2\n")
                && out.contains("    je fast@PLT\n")
                && out.contains(&format!(".LBB2:\n    jmp slow@PLT\n    {T}\n")),
            "{out}"
        );
        // A jump-table / data mention: the relay stays.
        let (_, out) = apply(&format!(
            "{}    .quad .LBB2\n",
            dispatch(&format!("    jmp slow@PLT\n    {T}\n"))
        ));
        assert!(
            out.contains(&format!(".LBB2:\n    jmp slow@PLT\n    {T}\n")),
            "{out}"
        );
    }

    fn skip(between: &str, after: &str) -> String {
        format!(
            "    movl $244608, %ecx\n    btl %edi, %ecx\n    jnc .LBB2\n{between}    jmp hit@PLT\n    {T}\n    # LCCC_CALL_ARGS 1\n{after}.LBB2:\n    ret\n"
        )
    }

    /// dav1d `if (wedge_allowed_mask & (1 << bs)) hit(bs);`: the jcc jumps
    /// over the sibling call, so the negated jcc IS the call (Clang 23.1.4:
    /// `jb hit@PLT`).
    #[test]
    fn a_jcc_over_a_sibling_call_becomes_the_negated_conditional_call() {
        let (c, out) = apply(&skip(".LBB1:\n", ""));
        assert!(c, "{out}");
        assert!(
            out.contains("    jb hit@PLT\n    # LCCC_TAIL_CALL\n    # LCCC_CALL_ARGS 1\n"),
            "{out}"
        );
        assert!(!out.contains("jmp hit") && !out.contains("jnc"), "{out}");
        assert!(out.contains(".LBB2:\n    ret\n"), "{out}");
        // Every condition code negates exactly.
        for (cc, neg) in [
            ("je", "jne"),
            ("jl", "jge"),
            ("ja", "jbe"),
            ("js", "jns"),
            ("jp", "jnp"),
        ] {
            let body = skip("", "").replace("jnc .LBB2", &format!("{cc} .LBB2"));
            let (c, out) = apply(&body);
            assert!(
                c && out.contains(&format!("    {neg} hit@PLT\n")),
                "{cc}: {out}"
            );
        }
    }

    #[test]
    fn a_jcc_over_anything_but_a_lone_marked_sibling_call_stays() {
        for (between, after) in [
            // Work on the fall-through path before the call.
            ("    movl %esi, %edi\n", ""),
            // Code between the call's markers and the target label.
            ("", "    int3\n"),
            // A CFI row between the branches.
            ("    .cfi_remember_state\n", ""),
        ] {
            let (c, out) = apply(&skip(between, after));
            assert!(!c, "{between:?} {after:?}:\n{out}");
        }
        // A referenced label between: another path enters the call.
        let (c, out) = apply(&format!(
            "    btl %edi, %ecx\n    jnc .LBB2\n.LBB1:\n    jmp hit@PLT\n    {T}\n.LBB2:\n    ret\n    .quad .LBB1\n"
        ));
        assert!(!c, "{out}");
        // Unmarked `jmp sym`: may be anything.
        let (c, out) =
            apply("    btl %edi, %ecx\n    jnc .LBB2\n    jmp hit@PLT\n.LBB2:\n    ret\n");
        assert!(!c, "{out}");
        // The target label is not the one right after the call.
        let (c, out) = apply(&format!(
            "    btl %edi, %ecx\n    jnc .LBB3\n    jmp hit@PLT\n    {T}\n.LBB2:\n    ret\n.LBB3:\n    ret\n"
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
            .find(|&i| store.get(i).trim_ascii() == "testl %edx, %edx")
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
