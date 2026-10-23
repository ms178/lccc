//! Shared peephole-optimizer utilities used by ARM, RISC-V, x86, and i686.
//!
//! Text-level operations on assembly lines:
//! - whole-word register/symbol matching (never `x1` inside `x11` or `main.x1.0`)
//! - source-operand register rewrite (dest-first *and* AT&T dest-last)
//! - [`LineStore`]: one backing buffer + offset table, no per-line `String`
//!
//! # Word characters
//!
//! Identifier characters are ASCII alphanumeric, `.`, and `_` (ELF symbols,
//! GAS local labels). `%` is *not* an identifier character, so `%rax` is
//! `%` + word `rax`. Callers that must distinguish the two pass the full
//! token (`"%rax"`).
//!
//! # Dest position
//!
//! [`replace_source_reg_in_instruction`] is dest-first (`add x0, x1, x2`,
//! Intel `add rax, rbx`). [`replace_source_reg_att`] is dest-last
//! (`addq %rax, %rbx`). Using the dest-first helper on AT&T rewrites the
//! destination — a silent mis-rewrite. Stores (`str x0, [x1]`) put the
//! stored register *before* the first comma; do not use dest-first there.

// ── Word-boundary matching ───────────────────────────────────────────────────

/// ASCII alphanumeric, `.`, or `_` — ELF symbols and GAS labels.
#[inline]
pub(crate) fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'_'
}

/// Next whole-word occurrence of `word` in `bytes` at or after `from`.
fn find_whole_word(bytes: &[u8], word: &[u8], from: usize) -> Option<usize> {
    let wlen = word.len();
    if wlen == 0 || wlen > bytes.len() {
        return None;
    }
    let first = word[0];
    let mut i = from;
    while i + wlen <= bytes.len() {
        if bytes[i] == first && &bytes[i..i + wlen] == word {
            let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            let after_ok = i + wlen == bytes.len() || !is_ident_char(bytes[i + wlen]);
            if before_ok && after_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Replace every whole-word occurrence of `old` with `new`.
///
/// Empty `old` is a no-op (a zero-length match at every byte would loop
/// forever). Output is byte-preserving: unmatched UTF-8 comments stay UTF-8.
/// The previous `bytes[i] as char` path recoded every high byte as Latin-1.
pub(crate) fn replace_whole_word(text: &str, old: &str, new: &str) -> String {
    if old.is_empty() || old == new {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let old_b = old.as_bytes();
    let Some(mut i) = find_whole_word(bytes, old_b, 0) else {
        return text.to_owned();
    };

    let mut out = Vec::with_capacity(text.len());
    let mut copied = 0usize;
    let old_len = old_b.len();
    let new_b = new.as_bytes();
    loop {
        out.extend_from_slice(&bytes[copied..i]);
        out.extend_from_slice(new_b);
        copied = i + old_len;
        match find_whole_word(bytes, old_b, copied) {
            Some(n) => i = n,
            None => break,
        }
    }
    out.extend_from_slice(&bytes[copied..]);
    // `old`/`new`/`text` are valid UTF-8; we only splice at char boundaries
    // (whole-word matches cannot start or end mid-code-point: ident chars
    // are ASCII, and `old` itself is a `&str`).
    debug_assert!(std::str::from_utf8(&out).is_ok());
    // SAFETY: concatenation of UTF-8 slices of `text` and `new`.
    unsafe { String::from_utf8_unchecked(out) }
}

/// Is the whole-word match at `at` a symbol reference rather than a register?
///
/// A C identifier may spell a register name (`long x20;`, `int a1;`, `void
/// w2(void)` are all legal), and relocation syntax is what tells them apart:
/// `:got_lo12:x20` and `=x1` (AArch64), `%pcrel_lo(a1)` (RISC-V) -- but not
/// the base register of `8(a1)`.
fn is_symbol_position(bytes: &[u8], at: usize) -> bool {
    let Some(prev) = at.checked_sub(1).map(|p| bytes[p]) else {
        return false;
    };
    match prev {
        b':' | b'=' => true,
        b'(' => {
            let mut p = at - 1;
            while p > 0 && is_ident_char(bytes[p - 1]) {
                p -= 1;
            }
            p > 0 && bytes[p - 1] == b'%' && p < at - 1
        }
        _ => false,
    }
}

/// The byte span of `trimmed` (one AArch64 or RISC-V instruction) whose
/// register names may be renamed, or `None` when no operand is a register.
///
/// Direct branch and call targets are symbols (`bl w2`, `call a1`), and so is
/// every operand after the destination of an address-forming pseudo-op or a
/// PC-relative literal load (`adrp x0, x20`, `lga t0, a1`, `ldr x0, sym`).
/// Directives and labels are left alone: after a coalescing rename
/// `.cfi_restore x20` still describes x20, which the renamed code no longer
/// touches.
fn register_operand_span(trimmed: &str) -> Option<(usize, usize)> {
    if trimmed.starts_with('.') || trimmed.ends_with(':') {
        return None;
    }
    let op_at = operands_start(trimmed)?;
    let mnemonic = &trimmed[..trimmed.find(|c: char| c.is_ascii_whitespace())?];
    if matches!(mnemonic, "b" | "bl" | "call" | "tail" | "j" | "jal") {
        return None;
    }
    let first_end = trimmed[op_at..].find(',').map(|c| op_at + c);
    let literal_load =
        mnemonic.starts_with("ldr") && first_end.is_some_and(|c| !trimmed[c..].contains('['));
    if matches!(mnemonic, "adr" | "adrp" | "la" | "lla" | "lga") || literal_load {
        return Some((op_at, first_end.unwrap_or(trimmed.len())));
    }
    Some((op_at, trimmed.len()))
}

/// Rename `old` to `new` in `trimmed[lo..hi]`, skipping symbol references.
fn rename_in_span(trimmed: &str, lo: usize, hi: usize, old: &str, new: &str) -> Option<String> {
    let bytes = trimmed.as_bytes();
    let mut out = String::with_capacity(trimmed.len() + 4);
    let mut copied = 0usize;
    let mut from = lo;
    while let Some(i) = find_whole_word(bytes, old.as_bytes(), from) {
        if i + old.len() > hi {
            break;
        }
        from = i + old.len();
        if is_symbol_position(bytes, i) {
            continue;
        }
        out.push_str(&trimmed[copied..i]);
        out.push_str(new);
        copied = i + old.len();
    }
    if copied == 0 {
        return None;
    }
    out.push_str(&trimmed[copied..]);
    Some(out)
}

/// Rename every REGISTER occurrence of `old` in one AArch64 or RISC-V line.
///
/// The register-renaming passes used [`replace_whole_word`], which cannot
/// tell `x20` the register from `x20` the symbol and so redirected
/// `ldr x0, [x0, :got_lo12:x20]`, `lga t0, a1` or `bl w2` to another object
/// whenever the renamed register shared a name with a global.
pub(crate) fn rename_register_operand(line: &str, old: &str, new: &str) -> String {
    if old.is_empty() || old == new {
        return line.to_owned();
    }
    let trimmed = line.trim();
    let Some((lo, hi)) = register_operand_span(trimmed) else {
        return line.to_owned();
    };
    match rename_in_span(trimmed, lo, hi, old, new) {
        Some(renamed) => {
            let leading = line.len() - line.trim_start().len();
            format!("{}{renamed}", &line[..leading])
        }
        None => line.to_owned(),
    }
}

/// `true` if `text` contains `word` at a word boundary. Empty `word` is never
/// a match (vacuous empty matches at punctuation / EOF are not useful).
pub(crate) fn has_whole_word(text: &str, word: &str) -> bool {
    find_whole_word(text.as_bytes(), word.as_bytes(), 0).is_some()
}

// ── Source-operand rewrite ───────────────────────────────────────────────────

/// Byte index of the first operand after the mnemonic, or `None`.
///
/// Accepts any ASCII whitespace (`add x0,…` and `add\tx0,…`). Multiple
/// spaces/tabs after the mnemonic are skipped so the returned slice starts
/// at the first operand character.
/// Find the last comma that separates OPERANDS rather than SIB address fields.
///
/// An AT&T memory operand carries its own commas inside balanced parentheses —
/// `DISP(%base,%index,scale)` — so a plain `rfind(',')` / `memrchr(',')` does
/// NOT locate an instruction's destination operand. It locates the comma
/// between the SIB base and its index whenever the memory operand is the LAST
/// one, i.e. for every store and read-modify-write through an indexed address:
///
/// ```text
///     movb %al, (%r10,%r12)     rfind(',')  -> ... ,%r12)   WRONG dest
///                           ^-- the top-level comma is the one after %al
/// ```
///
/// Text after that comma (`%r12)`) contains no `(`, so predicates of the form
/// "the destination is a memory operand" silently answered *false* for indexed
/// stores. This is the single canonical splitter for that decision; operand
/// parsers on every AT&T backend must use it instead of re-deriving the split.
///
/// Parenthesis depth is tracked with `saturating_sub` so malformed input can
/// never underflow; a `)` without a matching `(` simply stays at depth 0.
#[inline]
pub(crate) fn last_top_level_comma(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0u32;
    let mut last = None;
    for (idx, &byte) in bytes.iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => last = Some(idx),
            _ => {}
        }
    }
    last
}

/// Split an AT&T operand list at its TOP-LEVEL commas: the whole-operand
/// form of [`last_top_level_comma`]. A memory operand carries its own commas
/// inside balanced parentheses — `DISP(%base,%index,scale)` — so a naive
/// `split(',')` shreds SIB addresses into phantom operands. Backends must
/// use this splitter (or the last-comma form above) rather than re-deriving
/// the walk; the shared implementation is what keeps the x86-64 and i686
/// operand views from drifting apart.
///
/// The input is the operand list AFTER the mnemonic (leading whitespace is
/// tolerated; each part retains its surrounding whitespace). Semantics match
/// the historical i686 walkers exactly: the depth counter is a plain `i32`
/// that may go negative on malformed (unbalanced) input, which suppresses
/// further splits — the conservative direction.
pub(crate) fn split_top_level_commas(operands: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in operands.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    parts.push(cur);
    parts
}

fn operands_start(trimmed: &str) -> Option<usize> {
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i == 0 || i == bytes.len() {
        return None;
    }
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i == bytes.len() { None } else { Some(i) }
}

fn splice_source_rewrite(
    line: &str,
    trimmed: &str,
    src_abs_lo: usize,
    src_abs_hi: usize,
    old_reg: &str,
    new_reg: &str,
) -> Option<String> {
    let src = &trimmed[src_abs_lo..src_abs_hi];
    let rewritten = replace_whole_word(src, old_reg, new_reg);
    if rewritten == src {
        return None;
    }
    let leading = line.len() - line.trim_start().len();
    let mut out = String::with_capacity(leading + trimmed.len() + new_reg.len());
    out.push_str(&line[..leading]);
    out.push_str(&trimmed[..src_abs_lo]);
    out.push_str(&rewritten);
    out.push_str(&trimmed[src_abs_hi..]);
    Some(out)
}

/// Replace `old_reg` in source operands of a **dest-first** instruction.
///
/// `add x0, x1, x2` / Intel `add rax, rbx`: everything after the first
/// comma is source. Returns `None` if nothing changed, the line has no
/// mnemonic/operand split, or there is no comma (no distinct dest).
///
/// Leading whitespace is preserved. Do **not** use for AT&T (`addq %src, %dst`)
/// or ARM/RISC-V stores (`str w0, [x1]`) — the stored register sits before
/// the first comma.
pub(crate) fn replace_source_reg_in_instruction(
    line: &str,
    old_reg: &str,
    new_reg: &str,
) -> Option<String> {
    if old_reg.is_empty() || old_reg == new_reg {
        return None;
    }
    let trimmed = line.trim();
    let op_at = operands_start(trimmed)?;
    let comma = trimmed[op_at..].find(',')?;
    let src_lo = op_at + comma;
    // Symbol operands (`lga t0, a1`, `:got_lo12:x20`) are not sources.
    let (_, hi) = register_operand_span(trimmed)?;
    if src_lo >= hi {
        return None;
    }
    let renamed = rename_in_span(trimmed, src_lo, hi, old_reg, new_reg)?;
    let leading = line.len() - line.trim_start().len();
    Some(format!("{}{renamed}", &line[..leading]))
}

/// Replace `old_reg` in source operands of an **AT&T** instruction.
///
/// `addq %rax, %rbx`: dest is the last comma-separated operand. Sources
/// are everything before that comma. `pushq %rax` (no comma) returns
/// `None` — the single operand is both source and dest.
// Dead in production builds (the x86 peephole rewrites destinations only);
// exercised by the `att_replaces_only_sources` test. Scope the `expect` to
// non-test builds so the deadness assertion stays compiler-enforced for
// shipped configurations while test builds see a genuinely called function.
#[cfg_attr(not(test), expect(dead_code))]
pub(crate) fn replace_source_reg_att(line: &str, old_reg: &str, new_reg: &str) -> Option<String> {
    if old_reg.is_empty() || old_reg == new_reg {
        return None;
    }
    let trimmed = line.trim();
    let op_at = operands_start(trimmed)?;
    let args = &trimmed[op_at..];
    // Top-level split only: `movb %al, (%r10,%r12)` has its LAST comma inside
    // the SIB address, so `rfind(',')` would treat `%r12)` as the destination
    // and splice the source rewrite into the middle of the memory operand.
    let comma = last_top_level_comma(args.as_bytes())?;
    splice_source_rewrite(line, trimmed, op_at, op_at + comma, old_reg, new_reg)
}

// ── LineStore ────────────────────────────────────────────────────────────────
//
// Peephole walks and occasionally rewrites individual lines. A `Vec<String>`
// costs 24 bytes of heap header per line plus a separate allocation.
// `LineStore` keeps the original text in one `String` and records `(start, len)`
// per line (8 bytes). Rewritten lines live in a small side buffer; a second
// `replace` of the same index overwrites that slot instead of leaking it.

/// `(start, len)` into `original`, or a replacement-slot index when
/// `len == u32::MAX` (`start` is then the index into `replacements`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineEntry {
    start: u32,
    len: u32,
}

const REPLACED: u32 = u32::MAX;

pub(crate) struct LineStore {
    original: String,
    entries: Vec<LineEntry>,
    replacements: Vec<String>,
}

/// Length of `line` excluding its trailing ASCII blanks — the same set
/// `str::trim_end` removes, minus `\n`, which cannot appear inside a line.
///
/// This is the only place trailing blanks are removed. `LineStore::new`
/// shortens every span with it and `LineStore::replace` truncates the
/// replacement with it, and those two are the store's only writers (its whole
/// API is `new`, `get`, `len`, `replace`). `LineInfo::trimmed` therefore has
/// nothing left to trim and is a pure slice; it calls this function only from a
/// `debug_assert` that pins the invariant for any future caller that bypasses
/// the store.
///
/// That division is measured, not stylistic. Trimming per *access* instead of
/// per *write* cost 24% of a 33 s compile of `mm/page_alloc.c` (7764 lines,
/// kernel flags) with `str::trim_end`, which decodes UTF-8 backwards through a
/// `char::is_whitespace` predicate, and still 6% with this byte scan: 473 call
/// sites inside the peephole's fixed-point loop reach ~10^8 calls on a large TU.
/// Removing the per-access trim restored the baseline and left the emitted
/// object byte-identical, including the boot image's sha256.
///
/// `str::trim_end` is not interchangeable here: it also strips Unicode blanks
/// (U+00A0 and friends), which no emitter in this crate produces. Such a line
/// keeps its trailing bytes and simply fails to match a pattern — the
/// conservative direction, and the behaviour before trailing blanks were trimmed
/// at all. `passes/whitespace_invariance.rs` pins the ASCII property end to end.
///
/// Trailing whitespace is semantically neutral in every assembly syntax GAS
/// accepts, so no pass may be able to observe it.  Enforcing that here, once,
/// is what makes it true for all of them: passes reach line text both through
/// `LineInfo::trimmed` and through raw `store.get(i)` slices (the store-forwarding
/// and relay passes slice operands out of the raw text), and only the store sees
/// both.  It is also free in the common case — a compiler-emitted line has no
/// trailing blanks, so this is one predictable comparison per line, paid while
/// the line is already being scanned for its `\n`.
///
/// Without it, a single trailing space after `movq 152(%rsp), %rax` was enough
/// to stop the store-to-load forward of the `movq $0, 152(%rsp)` above it in
/// lccc's own sha256 output: the load's operand slice carried the blank, the
/// slot comparison missed, and the constant stayed in memory.
/// `passes/whitespace_invariance.rs` asserts the property over the repository's
/// real assembly under seven paddings, CRLF included.
#[inline]
pub(crate) fn line_len_without_trailing_blanks(line: &[u8]) -> usize {
    let mut end = line.len();
    while end > 0 && matches!(line[end - 1], b' ' | b'\t' | b'\r' | 0x0b | 0x0c) {
        end -= 1;
    }
    end
}

#[inline]
fn pack_span(start: usize, len: usize) -> LineEntry {
    debug_assert!(
        start <= u32::MAX as usize && len <= u32::MAX as usize,
        "LineStore span exceeds u32 ({start}+{len})"
    );
    LineEntry {
        start: start as u32,
        len: len as u32,
    }
}

impl LineStore {
    /// Split `asm` on `\n`. A trailing newline does not produce an extra
    /// empty line. An empty input is one empty line so `len() == 1` and
    /// `get(0) == ""` (same as the previous implementation).
    ///
    /// Every span excludes the line's trailing blanks (see
    /// [`line_len_without_trailing_blanks`]), so `get` never hands a pass text
    /// that differs from the same line without padding.  The bytes stay in
    /// `original`; only the span is shorter — no copy, no allocation.
    pub(crate) fn new(asm: String) -> Self {
        let bytes = asm.as_bytes();
        let total = bytes.len();
        let mut entries = Vec::with_capacity(total / 20 + 1);
        let mut start = 0usize;
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\n' {
                entries.push(pack_span(
                    start,
                    line_len_without_trailing_blanks(&bytes[start..i]),
                ));
                start = i + 1;
            }
        }
        if start < total || entries.is_empty() {
            entries.push(pack_span(
                start,
                line_len_without_trailing_blanks(&bytes[start..total]),
            ));
        }
        LineStore {
            original: asm,
            entries,
            replacements: Vec::new(),
        }
    }

    #[inline]
    pub(crate) fn get(&self, idx: usize) -> &str {
        let e = &self.entries[idx];
        if e.len == REPLACED {
            &self.replacements[e.start as usize]
        } else {
            let start = e.start as usize;
            let end = start + e.len as usize;
            &self.original[start..end]
        }
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Replace line `idx`. A second replace of the same index overwrites
    /// the existing slot (the old path pushed a new `String` every time
    /// and left the previous one unreachable).
    ///
    /// The replacement obeys the same no-trailing-blanks invariant as the
    /// original spans: a pass that formats its own text with a trailing space
    /// (they do — several build `format!("{indent}{mnem} {src}, {dst} ")` style
    /// strings) would otherwise reintroduce, for later passes, exactly the
    /// byte the store removed on the way in.  `truncate` is O(1) here: the tail
    /// is ASCII, so the cut is always a char boundary.
    pub(crate) fn replace(&mut self, idx: usize, mut new_text: String) {
        let end = line_len_without_trailing_blanks(new_text.as_bytes());
        new_text.truncate(end);
        let e = &mut self.entries[idx];
        if e.len == REPLACED {
            self.replacements[e.start as usize] = new_text;
            return;
        }
        let rep_idx = self.replacements.len();
        debug_assert!(rep_idx <= u32::MAX as usize);
        self.replacements.push(new_text);
        *e = LineEntry {
            start: rep_idx as u32,
            len: REPLACED,
        };
    }

    /// Insert a new line at `idx`, shifting every later line's index up by
    /// one.  The text lands in the replacements arena (a `REPLACED`-style
    /// entry pointing at it), so no original-buffer surgery happens.
    ///
    /// Only for rare, structural rewrites (the loop-invariant LEA hoist
    /// placing an instruction on a fall-through entry path): callers MUST
    /// adjust every index they hold at or above `idx`, including loop
    /// bounds discovered before the insertion.  O(len) in the entry splice.
    pub(crate) fn insert_line(&mut self, idx: usize, text: String) {
        let end = line_len_without_trailing_blanks(text.as_bytes());
        let mut new_text = text;
        new_text.truncate(end);
        let rep_idx = self.replacements.len();
        debug_assert!(rep_idx <= u32::MAX as usize);
        self.replacements.push(new_text);
        self.entries.insert(
            idx,
            LineEntry {
                start: rep_idx as u32,
                len: REPLACED,
            },
        );
    }

    /// Concatenate kept lines, each followed by `\n`.
    pub(crate) fn build_result(&self, skip: impl Fn(usize) -> bool) -> String {
        let mut result = String::with_capacity(self.original.len());
        for i in 0..self.entries.len() {
            if !skip(i) {
                result.push_str(self.get(i));
                result.push('\n');
            }
        }
        result
    }
}

// ── Assembler-directive effects ────────────────────────────────────────────────

/// Directives that emit DATA -- bytes into the section being assembled.
///
/// A `.byte`/`.long`/`.quad` sequence inside a function body can encode any
/// instruction -- `adc`, `clc`, a branch, `int3` -- that no mnemonic table can
/// see, exactly like inline asm, so a walker must not step over it
/// as if it had no effect.  The question this answers is deliberately narrow:
/// **can this line put bytes into the current section, or change what the lines
/// around it mean?**  A directive is benign only if it does neither:
///
/// * it emits nothing into the current section.  `.cfi_*` writes `.eh_frame`,
///   `.loc`/`.file` write the line tables, `.globl`/`.type`/`.size` are symbol
///   bookkeeping, `.comm`/`.lcomm` reserve space in `.bss` rather than here,
///   `.ident` goes to `.comment`; and
/// * it does not change how neighbouring lines are assembled.  `.code16`,
///   `.code32`, `.arch`, `.intel_syntax`, `.att_syntax` re-interpret every
///   following instruction, and `.text`, `.data`, `.section .rodata`,
///   `.pushsection`, `.previous` move the *following* lines out of the section
///   this window was parsed in.  None of them emits a byte here, yet each makes
///   "the contiguous run of code I analysed" false, which is the assumption
///   every caller of this predicate rests on.  (`engineering/evidence/
///   2026-10-08-pr-audit/directive-census.txt` counts `.section` 68 times in
///   function bodies -- jump-table islands in `.rodata` -- so this is not a
///   hypothetical shape.)
///
/// Anything the classification does not name is data, because "unrecognized" is
/// not "harmless".  That is also why `.zero`, `.skip`, `.space` and `.fill` are
/// *not* benign: all four emit bytes, the first three zeros, and `0x00 0x00`
/// decodes as `add %al,(%rax)` -- a flag writer -- while
/// `.fill 1,1,0xF8` is a bare `clc`.  Their reach inside function bodies is
/// measured at 0 across the 106-source corpora, so the caution refuses no fold
/// this compiler has ever wanted to make.
///
/// The alignment family needs a rule rather than a verdict: in a text section
/// `.p2align 4` fills with nops (harmless), but the fill byte is the
/// programmer's choice -- `.p2align 4,0xf8` fills with `clc` and
/// `.p2align 4,0xcc` with `int3`, both of which a flag or control-flow walk must
/// not step over.  So an explicit fill operand makes an alignment directive data
/// (`alignment_fill_is_explicit`), while `.p2align 4,,10` -- empty fill, `max`
/// only -- stays benign.
pub(crate) fn directive_emits_data(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.starts_with('.') {
        return false;
    }
    // A metadata-looking first statement cannot certify the whole line.
    // In particular CFI followed by `; .byte ...` contains executable bytes.
    let Some(trimmed) = single_asm_statement(trimmed) else {
        return true;
    };
    let rest = &trimmed[1..];
    // A local label like `.Ltmp3:` is dotted but not a directive: it emits no
    // bytes and is not what this predicate is for.  Callers select on
    // `LineKind::Directive` first (labels are a barrier of their own there), so
    // this only matters to a caller that hands the classifier raw text -- which
    // is precisely how a "directive" predicate gets reused by a census script or
    // a new pass, and an answer of "data" for a label would be silently wrong.
    if trimmed.ends_with(':') {
        return false;
    }
    let name = rest
        .split(|c: char| c.is_whitespace() || c == ',')
        .next()
        .unwrap_or("");
    let args = &rest[name.len()..];
    // CFI never touches the section being assembled.  Prefix-matched on
    // purpose: the previous version enumerated fifteen `.cfi_*` names, which
    // silently classified the unlisted `.cfi_adjust_cfa_offset` -- the one in
    // every prologue -- as data.  That cost no folds (census: 0 in bodies) but
    // the enumeration was wrong in the over-conservative direction, and a
    // classifier that is wrong in either direction gets trusted in the other.
    if name.starts_with("cfi_") {
        return false;
    }
    if matches!(
        name,
        "p2align" | "p2alignw" | "p2alignl" | "align" | "balign" | "balignw" | "balignl"
    ) {
        return alignment_fill_is_explicit(args);
    }
    // Integrated-assembler alignment marker: default NOP padding only.
    if name == "lccc_tight_loop" {
        return false;
    }
    if matches!(
        name,
        // Positional/debug/symbol bookkeeping: no bytes here, no re-reading of
        // what follows.  Deliberately *no* `.text`/`.data`/`.section` family and
        // no `.arch`/`.code*`/`._syntax` members -- see the doc comment.
        // `.end` is absent too: it closes `.macro`/`.if` blocks whose *opener*
        // is already unrecognised (hence data), and a verdict that is only right
        // because of what some other line does is not a verdict.
        "loc"
            | "file"
            | "ident"
            | "globl"
            | "global"
            | "weak"
            | "local"
            | "type"
            | "size"
            | "set"
            | "equ"
            | "comm"
            | "lcomm"
    ) {
        return false;
    }
    // `.byte`, `.2byte`/`.word`, `.4byte`/`.long`, `.8byte`/`.quad`, `.value`,
    // `.inst`, `.insn`, `.ascii`, `.asciz`, `.string`, `.uleb128`, `.sleb128`,
    // `.float`, `.double`, `.zero`, `.skip`, `.space`, `.fill`, `.if`/`.else`,
    // `.macro`/`.rept`/`.irp` blocks, section and mode switches -- and
    // anything unrecognized.
    true
}

/// True when an alignment directive names its own fill byte.
///
/// `args` is everything after the directive name: GAS takes
/// `.p2align <align>[, <fill>[, <max>]]`, so the second comma-separated field
/// is the fill -- present and non-empty means the padding is arbitrary data,
/// present and *empty* (`.p2align 4,,10`) means the default nops/zeros.  Fields
/// are split on commas, not on a whitespace-or-comma class, because ", " and
/// ",," must not look alike to a fill test.
pub(crate) fn alignment_fill_is_explicit(args: &str) -> bool {
    let mut fields = args.split(',');
    let _ = fields.next(); // <align>
    matches!(fields.next(), Some(fill) if !fill.trim().is_empty())
}

/// Byte length of `b` with a trailing, unquoted `#` comment removed.
///
/// GAS treats an unquoted `#` as start-of-comment, so the *syntax* of a line
/// ends there -- but the comment is only invisible to the parser, not to the
/// line classifiers, which must not look at the last byte of the line (that
/// reads `.long 4  # note:` as a *label*).  Quoting is honoured because `#` is an
/// ordinary character inside `"..."` and `'...'`: cutting `.string "a#b:"` at
/// the `#` would turn a data directive into the string `a`.
///
/// A line that *begins* with `#` is returned unchanged: `#APP`, `#NO_APP` and
/// the `# LCCC_*` markers are whole-line directives whose text is meaningful to
/// this compiler, and stripping them would erase the inline-asm quarantine.
pub(crate) fn line_len_without_trailing_comment(b: &[u8]) -> usize {
    // A line that *begins* with `#` is a comment line, not an instruction with
    // a comment: `#APP`, `#NO_APP` and the `# LCCC_*` return/ABI markers are
    // read from the line text by other passes, so cutting at the `#` would
    // delete them and silently disable the inline-asm quarantine.
    if b.first() == Some(&b'#') {
        return b.len();
    }
    let mut i = 0;
    let mut in_str: Option<u8> = None;
    while i < b.len() {
        match in_str {
            Some(q) => {
                if b[i] == b'\\' {
                    i += 1; // escaped byte inside the quote
                } else if b[i] == q {
                    in_str = None;
                }
            }
            None => match b[i] {
                b'"' | b'\'' => in_str = Some(b[i]),
                // Cut here, then drop the blanks the comment left behind: a
                // classifier that reads the END of the line (the label colon)
                // must not see a space where the instruction text ended.
                b'#' => {
                    let mut end = i;
                    while end > 0 && matches!(b[end - 1], b' ' | b'\t' | b'\r') {
                        end -= 1;
                    }
                    return end;
                }
                _ => {}
            },
        }
        i += 1;
    }
    b.len()
}

/// Return one complete statement, without a trailing comment. Unknown quoting
/// and statement separators fail closed. This is shared by both directive
/// predicates so metadata cannot hide executable statements from either one.
fn single_asm_statement(text: &str) -> Option<&str> {
    let mut quote = None;
    let mut escaped = false;
    for (i, b) in text.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(q) = quote {
            if b == b'\\' {
                escaped = true;
            } else if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'#' => return Some(text[..i].trim_end()),
                b';' | b'\n' | b'\r' => return None,
                _ => {}
            }
        }
    }
    (quote.is_none() && !escaped).then_some(text)
}

/// Exact debug-attribution tokens, excluding compound assembler statements.
/// Honor quoted filenames, escapes and trailing comments without allocation.
pub(crate) fn is_debug_marker_directive(text: &str) -> bool {
    let text = text.trim();
    matches!(text.split_whitespace().next(), Some(".loc" | ".file"))
        && single_asm_statement(text).is_some()
}

/// Directives a peephole window may step over: they emit no bytes, reference no
/// data whose value depends on code layout, and cannot switch section.
///
/// This is *the* directive law for both x86-64 and i686, in one place, because
/// the two backends had drifted apart on it: x86-64 treated every directive as
/// a barrier (sound, but it blinded every locality window at the first `.loc`
/// on a `-g` command line), while i686 treated *none* of them as one and
/// justified it with "a directive emits no instruction" -- which is true of
/// `.cfi_startproc` and false of everything that puts bytes in the section:
///
/// * `.long .L1-.`, `.quad sym`, `.string "x"`, `.byte` -- these *are* content.
///   A jump table lowered into `.text` is one of them: it holds label
///   differences, so a pass that rewrites or moves lines across it changes what
///   the table means.
/// * `.p2align 4,0xcc` -- alignment with an explicit fill emits a trap (or any
///   byte the author chose) into the middle of the region a window just
///   certified as contiguous. Without a fill the padding is nops, which is why
///   the check is on the fill and not on the mnemonic.
/// * `.section`, `.text`, `.data`, `.bss` -- subsequent lines belong to another
///   section; position here is meaning, not layout.
/// * `.ifdef`/`.if`/`.else`/`.endif` -- crossing them changes whether a line is
///   assembled at all.
///
/// Everything else on the allowlist below emits nothing *and* says nothing about
/// the rest of the file: ordinary unwind info (`.cfi_*`, but see the
/// `.cfi_sections` exception in the body -- it emits no bytes yet moves every
/// following FDE between sections), line tables (`.loc`, `.file`),
/// function-boundary symbols (`.LFB`), symbol metadata (`.size`, `.type`,
/// `.globl`, `.weak`, `.hidden`, `.set`, `.equ`).
/// Unknown directives are opaque: the cost of a missed fold is a few bytes, the
/// cost of a wrong one is a miscompiled object.
pub(crate) fn inert_directive(text: &str) -> bool {
    let t = text.trim();
    if !t.starts_with('.') {
        return false;
    }
    let Some(t) = single_asm_statement(t) else {
        return false;
    };
    let name = t.split_whitespace().next().unwrap_or("");
    if name.starts_with(".cfi_") {
        // No text bytes, but .cfi_sections redirects subsequent unwind FDEs.
        return name != ".cfi_sections";
    }
    if (t.starts_with(".LFB") && t.ends_with(':'))
        || matches!(
            name,
            ".loc" | ".file" | ".size" | ".type" | ".globl" | ".weak" | ".hidden" | ".set" | ".equ"
        )
    {
        return true;
    }
    if matches!(
        name,
        ".p2align" | ".p2alignw" | ".p2alignl" | ".balign" | ".balignw" | ".balignl" | ".align"
    ) {
        return !alignment_fill_is_explicit(&t[name.len()..]);
    }
    // The private tight-loop marker is an alignment request resolved by the
    // integrated assembler: at most default NOP padding at this position,
    // never data, registers, flags or memory. Treating it as opaque made every
    // peephole analysis (FP liveness, slot forwarding, memory folding) give up
    // on each function containing a tight loop, but only in direct `-c`
    // compiles, where the marker exists. `-S` output never carries it.
    if name == ".lccc_tight_loop" {
        return true;
    }
    false
}

#[cfg(test)]
mod inert_directive_tests {
    use super::*;

    #[test]
    fn compound_metadata_must_not_hide_executable_bytes() {
        for prefix in [
            ".cfi_def_cfa_offset 8",
            ".size f, .-f",
            ".type f, @function",
            ".globl f",
            ".weak f",
            ".hidden f",
            ".set a, 0",
            ".equ b, 0",
            ".p2align 0",
            ".balign 1",
            ".loc 1 7 3",
            ".file 1 \"x.c\"",
        ] {
            for suffix in [".byte 0xf9", "stc", ".Lafter:"] {
                let text = format!("{prefix}; {suffix}");
                assert!(!inert_directive(&text), "stepped across {text}");
                assert!(directive_emits_data(&text), "hid bytes in {text}");
            }
        }
    }

    #[test]
    fn directive_tokens_comments_and_quotes_have_one_lexical_contract() {
        for text in [
            ".p2alignment 0",
            ".balignment 1",
            ".align_extra 4",
            ".LFB_not_a_label",
        ] {
            assert!(!inert_directive(text), "unknown token: {text}");
        }
        for text in [
            r#".file 1 "semi;#colon.c""#,
            ".cfi_def_cfa_offset 8 # ; .byte 0xf9",
        ] {
            assert!(inert_directive(text), "comment/quoted punctuation: {text}");
            assert!(
                !directive_emits_data(text),
                "comment/quoted punctuation: {text}"
            );
        }
        assert!(inert_directive(".globl\tf"));
        assert!(inert_directive(".type\tf, @function"));
    }

    #[test]
    fn debug_tokens_reject_statement_chains_and_lookalikes() {
        for text in [
            ".loc 1 7 3; .byte 0xcc",
            ".file 2 \"x.c\"; ret",
            ".location 1",
            ".file_data 1",
            ".file 2 \"unterminated",
        ] {
            assert!(!is_debug_marker_directive(text), "{text}");
            assert!(!inert_directive(text), "{text}");
        }
    }

    #[test]
    fn inert_directives_are_the_ones_that_emit_and_reference_nothing() {
        for t in [
            ".cfi_startproc",
            ".cfi_adjust_cfa_offset 16",
            ".cfi_def_cfa_register 6",
            ".loc 1 12 4",
            ".loc\t1 12 4",
            ".file\t1 \"k.c\"",
            ".file 1 \"k.c\"",
            ".file 1 \"semi;hash#name.c\"",
            ".loc 1 7 3 # ; .byte 0xcc",
            ".LFB0:",
            ".size f, .-f",
            ".type f, @function",
            ".globl main",
            ".hidden f",
            ".p2align 4",
            ".balign 16",
            ".align 16",
            ".lccc_tight_loop .LBB7",
        ] {
            assert!(inert_directive(t), "{t} must be steppable");
        }
    }

    #[test]
    fn tight_loop_marker_is_alignment_not_data() {
        // Like a default-fill `.p2align`: steppable and byte-free for the
        // data classifier. An opaque marker disabled peephole analyses in
        // every direct `-c` function containing a tight loop.
        for t in [".lccc_tight_loop .LBB7", "\t.lccc_tight_loop .L1"] {
            assert!(inert_directive(t), "{t}");
            assert!(!directive_emits_data(t), "{t}");
        }
        assert!(directive_emits_data(".p2align 4, 0xcc"));
    }

    #[test]
    fn content_and_layout_directives_are_opaque() {
        for t in [
            // Data: a jump table in .text holds label differences.
            ".long .L1-.",
            ".quad f",
            ".string \"x\"",
            ".byte 3, 4",
            ".value 0x1234",
            // Explicit fill puts a byte (here int3) in the window.
            ".p2align 4,0xcc",
            ".p2align 4, 0x90",
            ".balign 16, 0xcc",
            // Section switches and conditionals make position meaningful -- and
            // `.cfi_sections` is a section switch for the unwind stream: it is
            // what decides whether `.eh_frame` exists in the object at all.
            ".cfi_sections .debug_frame",
            ".section .rodata",
            ".text",
            ".data",
            ".bss",
            ".ifdef FOO",
            ".endif",
            // Unknown: fail closed.
            ".unhandled_pseudo 1",
        ] {
            assert!(!inert_directive(t), "{t} must block a window");
        }
    }

    #[test]
    fn trailing_comments_are_cut_unless_they_are_the_whole_line() {
        // Compare the surviving prefix, not an index: the point is that the
        // comment is gone and the instruction text is not.
        let cut = |s: &[u8]| -> Vec<u8> {
            let n = line_len_without_trailing_comment(s);
            s[..n].to_vec()
        };
        assert_eq!(cut(b".long 4  # note:"), b".long 4");
        assert_eq!(cut(b"foo:  # c"), b"foo:");
        // Trailing blanks the comment *covered* must go too, or a classifier
        // that reads the end of the line (the label colon) sees a space instead
        // of the label.
        assert_eq!(cut(b".LBB3:   # note"), b".LBB3:");
        // `#` inside quotes is data, not a comment.
        assert_eq!(cut(b".string \"a#b:\""), b".string \"a#b:\"");
        assert_eq!(cut(b".string \"a\\\"#b\""), b".string \"a\\\"#b\"");
        // Whole-line `#` markers keep their text: the inline-asm quarantine and
        // the `# LCCC_*` hints are read from it.
        assert_eq!(cut(b"#APP"), b"#APP");
        assert_eq!(cut(b"# NO_APP"), b"# NO_APP");
        // No comment at all.
        assert_eq!(cut(b"movl %eax, %ecx"), b"movl %eax, %ecx");
    }
}

#[cfg(test)]
mod directive_law_enforcement {
    //! The directive law must be *enforced*, not only documented.
    //!
    //! Every bug in this family had the same shape: a window walker decided for
    //! itself that "a directive is not a real instruction" and stepped over it.
    //! That is right for `.loc` and wrong for `.long .L1-.` (a jump-table entry
    //! whose value is a code distance), `.p2align 4,0xcc` (an inserted trap) and
    //! `.section` (position is meaning). `inert_directive` is the one answer;
    //! this test is what keeps a new pass from inventing a second one by
    //! grep-forbidding the local decision.

    use std::fs;
    use std::path::Path;

    /// Lines in the peephole walkers that decide directive opacity by hand.
    ///
    /// `kind != LineKind::Directive` is the skipping idiom (the unsound one: it
    /// steps over data, fill and section-switch directives too). Blocking with
    /// `kind == LineKind::Directive` is safe, so it is not policed here -- only
    /// the loosening direction is.
    fn violations(root: &Path, dir: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![root.join(dir)];
        while let Some(p) = stack.pop() {
            let rd = match fs::read_dir(&p) {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|x| x.to_str()) != Some("rs") {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let lines: Vec<&str> = text.lines().collect();
                for (i, raw) in lines.iter().enumerate() {
                    // Judge the code, not the prose.  Several tests and doc
                    // comments in these backends *describe* the forbidden pattern
                    // in order to explain why it is forbidden; matching that text
                    // made the guard fire on explanation, which trains people to
                    // reword comments instead of fixing code.
                    let line = code_part(raw);
                    if !line.contains("!= LineKind::Directive") {
                        continue;
                    }
                    // An exemption has to be *written down* next to the use, so
                    // a reviewer sees a decision rather than a drift.
                    // A marker on the same line or on the line just above both
                    // count: rustfmt keeps a comment above, and it is adjacent
                    // either way, which is all a reviewer needs to see.
                    // A marker anywhere in the comment block directly above also
                    // counts: the explanation belongs to the statement, and
                    // rustfmt decides how many lines it takes.
                    let marked = raw.contains("DIRECTIVE-LAW:")
                        || lines[..i]
                            .iter()
                            .rev()
                            .take_while(|l| l.trim_start().starts_with("//"))
                            .any(|l| l.contains("DIRECTIVE-LAW:"));
                    if marked {
                        continue;
                    }
                    out.push(format!(
                        "{}:{}: {}",
                        path.strip_prefix(root).unwrap_or(&path).display(),
                        i + 1,
                        line.trim()
                    ));
                }
            }
        }
        out
    }

    #[test]
    fn no_x86_or_i686_walker_skips_directives_by_hand() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("backend");
        let mut bad = violations(&root, "x86/codegen/peephole");
        bad.extend(violations(&root, "i686/codegen"));
        assert!(
            bad.is_empty(),
            "directive opacity must come from `inert_directive`/`is_steppable`, \
             not a local `!= LineKind::Directive` test (mark a deliberate \
             exception with `DIRECTIVE-LAW:` on the line):\n{}",
            bad.join("\n")
        );
    }

    /// Everything before an unquoted `//`.
    fn code_part(line: &str) -> &str {
        let bytes = line.as_bytes();
        let mut in_string = false;
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' if in_string => i += 1,
                b'"' => in_string = !in_string,
                b'/' if !in_string && bytes.get(i + 1) == Some(&b'/') => return &line[..i],
                _ => {}
            }
            i += 1;
        }
        line
    }

    #[test]
    fn the_detector_actually_detects() {
        // A guard that cannot fail is the failure mode this repository has been
        // burned by before, so the detector is tested against its own pattern.
        let tmp = std::env::temp_dir().join("lccc-directive-guard-self-test");
        let _ = fs::remove_dir_all(&tmp);
        let dir = tmp.join("x86/codegen/peephole/passes");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("bad.rs"),
            "fn w(i: &[LineInfo]) {\n    if i[0].kind != LineKind::Directive { }\n}\n",
        )
        .unwrap();
        // Explaining the pattern is not using it: a doc comment and a trailing
        // comment both have to be ignored, or the guard gets reworded away.
        fs::write(
            dir.join("prose.rs"),
            "/// A walker once tested `kind != LineKind::Directive` and was wrong.\n\
             fn w(i: &[LineInfo]) {\n    let _ = i; // != LineKind::Directive is the old bug\n}\n",
        )
        .unwrap();
        // And a written-down exemption next to a real use is honoured, in both of
        // the places rustfmt can leave it.
        fs::write(
            dir.join("exempt.rs"),
            "fn w(i: &[LineInfo]) {\n    // DIRECTIVE-LAW: selecting\n    if i[0].kind != LineKind::Directive { }\n}\n",
        )
        .unwrap();
        let v = violations(&tmp, "x86/codegen/peephole");
        fs::remove_dir_all(&tmp).unwrap();
        assert_eq!(
            v.len(),
            1,
            "the fixture must be reported, prose must not: {v:?}"
        );
        assert!(v[0].contains("bad.rs:2"), "line number: {}", v[0]);
    }

    #[test]
    fn the_steppable_predicate_is_wired_into_every_loose_walker() {
        // The two walkers that once skipped every directive must go through
        // the shared predicate; if a refactor drops the call, the fix is gone
        // even though the law still reads fine.
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backend/x86/codegen/peephole/passes");
        let mut users = 0usize;
        for f in ["dead_writes.rs", "liveness.rs"] {
            let t = std::fs::read_to_string(root.join(f)).expect(f);
            users += t.matches("is_steppable()").count();
        }
        assert!(
            users >= 2,
            "expected both walkers to call `is_steppable`, saw {users}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_rename_leaves_register_named_symbols_alone() {
        let r = rename_register_operand;
        assert_eq!(
            r("    ldr x0, [x0, :got_lo12:x20]", "x20", "x1"),
            "    ldr x0, [x0, :got_lo12:x20]"
        );
        assert_eq!(
            r("    ldr x0, [x0, :got_lo12:x20]", "x0", "x9"),
            "    ldr x9, [x9, :got_lo12:x20]"
        );
        assert_eq!(
            r("    adrp x20, :got:x20", "x20", "x1"),
            "    adrp x1, :got:x20"
        );
        assert_eq!(r("    adrp x0, x20", "x20", "x1"), "    adrp x0, x20");
        assert_eq!(r("    ldr x0, x20", "x20", "x1"), "    ldr x0, x20");
        assert_eq!(r("    ldr x0, =x1", "x1", "x2"), "    ldr x0, =x1");
        assert_eq!(r("    bl w2", "w2", "w9"), "    bl w2");
        assert_eq!(r("    blr x8", "x8", "x9"), "    blr x9");
        assert_eq!(r("    lga t0, a1", "a1", "t1"), "    lga t0, a1");
        assert_eq!(r("    lga t0, a1", "t0", "t1"), "    lga t1, a1");
        assert_eq!(
            r("    ld a0, %pcrel_lo(a1)(a1)", "a1", "t1"),
            "    ld a0, %pcrel_lo(a1)(t1)"
        );
        assert_eq!(r("    ld a0, 8(a1)", "a1", "t1"), "    ld a0, 8(t1)");
        assert_eq!(r("    call a1", "a1", "t1"), "    call a1");
        assert_eq!(
            r("    .cfi_restore x20", "x20", "x1"),
            "    .cfi_restore x20"
        );
        assert_eq!(r("\tadd\tx0, x1, x1", "x1", "x5"), "\tadd\tx0, x5, x5");
        assert_eq!(r("    add x11, x1", "x1", "x5"), "    add x11, x5");
    }

    #[test]
    fn source_rename_leaves_register_named_symbols_alone() {
        let r = replace_source_reg_in_instruction;
        assert_eq!(r("    ldr x0, [x0, :lo12:x20]", "x20", "x1"), None);
        assert_eq!(
            r("    ldr x0, [x0, :lo12:x20]", "x0", "x9").as_deref(),
            Some("    ldr x0, [x9, :lo12:x20]")
        );
        assert_eq!(r("    lga t0, a1", "a1", "t1"), None);
        assert_eq!(r("    adrp x0, x1", "x1", "x2"), None);
    }

    /// The store is the single place where the peephole's whitespace invariance
    /// is enforced: passes read line text both through `LineInfo::trimmed` and
    /// through raw `get()` slices (store forwarding and the relay passes slice
    /// operands out of the raw text), and only the store sees both paths.
    #[test]
    fn the_store_never_hands_a_pass_trailing_whitespace() {
        for padding in [" ", "    ", "\t", "        ", "\r", " \r", "  \t "] {
            let text = format!("f:\n    movq %rax, %rbx{padding}\n    ret{padding}\n");
            let store = LineStore::new(text);
            assert_eq!(store.len(), 3, "padding {padding:?} changed the line count");
            for i in 0..store.len() {
                let line = store.get(i);
                assert_eq!(
                    line,
                    line.trim_end(),
                    "padding {padding:?} survived into line {i}: {line:?}"
                );
            }
            let built = store.build_result(|_| false);
            assert!(
                built.split('\n').all(|line| line.trim_end() == line),
                "build_result re-emitted trailing whitespace for {padding:?}: {built:?}"
            );
        }

        // A line that is only whitespace is an empty line, not a padded one.
        let store = LineStore::new("f:\n   \n\t\nret\n".to_string());
        assert_eq!(store.get(1), "");
        assert_eq!(store.get(2), "");

        // Pass-generated replacements obey the same rule, so a later pass cannot
        // see blanks an earlier one formatted into its own text.
        let mut store = LineStore::new("f:\n    ret\n".to_string());
        store.replace(1, "    movq %rax, %rbx \t".to_string());
        assert_eq!(store.get(1), "    movq %rax, %rbx");
        store.replace(1, "    movq %rax, %rbx   ".to_string());
        assert_eq!(store.get(1), "    movq %rax, %rbx");

        // The documented edge contracts are unchanged by the trim.
        let store = LineStore::new(String::new());
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(0), "");
        let store = LineStore::new("a\nb\n".to_string());
        assert_eq!(store.len(), 2, "a trailing newline must not add a line");
    }

    #[test]
    fn ident_chars() {
        assert!(is_ident_char(b'x'));
        assert!(is_ident_char(b'1'));
        assert!(is_ident_char(b'.'));
        assert!(is_ident_char(b'_'));
        assert!(!is_ident_char(b'%'));
        assert!(!is_ident_char(b','));
        assert!(!is_ident_char(b' '));
        assert!(!is_ident_char(b'['));
    }

    #[test]
    fn whole_word_does_not_match_inside_longer_reg() {
        assert!(has_whole_word("add x1, x2", "x1"));
        assert!(!has_whole_word("add x11, x2", "x1"));
        assert!(!has_whole_word("add x10, x2", "x1"));
        assert!(!has_whole_word("main.x1.0", "x1"));
        assert!(has_whole_word("bl main.x1.0", "main.x1.0"));
        assert!(!has_whole_word("r8b", "r8"));
        assert!(has_whole_word("%rax", "rax"));
        assert!(has_whole_word("%rax", "%rax"));
    }

    #[test]
    fn empty_pattern_is_never_a_match() {
        assert!(!has_whole_word("add x0, x1", ""));
        assert_eq!(replace_whole_word("add x0, x1", "", "x9"), "add x0, x1");
        assert_eq!(replace_whole_word("add x0, x1", "x0", "x0"), "add x0, x1");
    }

    #[test]
    fn replace_whole_word_boundaries() {
        assert_eq!(
            replace_whole_word("add x1, x11, x1", "x1", "x9"),
            "add x9, x11, x9"
        );
        assert_eq!(replace_whole_word("x1", "x1", "sp"), "sp");
        assert_eq!(replace_whole_word("xx1x", "x1", "z"), "xx1x");
    }

    #[test]
    fn replace_whole_word_preserves_utf8() {
        // Cover 2-, 3-, and 4-byte code points. A `u8 as char` rebuilding
        // loop turns each of these source bytes into a second UTF-8 sequence.
        let src = "add x1, x2  // café € 🦀";
        let expected = "add x9, x2  // café € 🦀";
        let out = replace_whole_word(src, "x1", "x9");
        assert_eq!(out, expected);
        assert_eq!(out.as_bytes(), expected.as_bytes());
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn dest_first_replaces_only_sources() {
        let line = "  add x0, x1, x2";
        let out = replace_source_reg_in_instruction(line, "x1", "x9").unwrap();
        assert_eq!(out, "  add x0, x9, x2");
        // dest is not a source
        assert!(replace_source_reg_in_instruction(line, "x0", "x9").is_none());
        // no comma
        assert!(replace_source_reg_in_instruction("  ret", "x0", "x9").is_none());
        // tab after mnemonic
        let tab = replace_source_reg_in_instruction("\tadd\tx0, x1, x2", "x1", "x9").unwrap();
        assert_eq!(tab, "\tadd\tx0, x9, x2");
    }

    #[test]
    fn att_replaces_only_sources() {
        let line = "  addq %rax, %rbx";
        let out = replace_source_reg_att(line, "%rax", "%rcx").unwrap();
        assert_eq!(out, "  addq %rcx, %rbx");
        assert!(replace_source_reg_att(line, "%rbx", "%rcx").is_none());
        // dest-first helper would wrongly rewrite AT&T dest
        let wrong = replace_source_reg_in_instruction(line, "%rbx", "%rcx").unwrap();
        assert_eq!(wrong, "  addq %rax, %rcx");
    }

    #[test]
    fn last_top_level_comma_skips_sib_address_fields() {
        // Store through an indexed address: the LAST comma is inside the SIB,
        // so a plain rfind(',') would report `%r12)` as the destination.
        let l = "movb %al, (%r10,%r12)";
        let c = last_top_level_comma(l.as_bytes()).unwrap();
        assert_eq!(l[..c].trim(), "movb %al");
        assert_eq!(l[c + 1..].trim(), "(%r10,%r12)");
        // A scale field adds a third comma inside the address.
        let l = "movq %rax, 16(%rbx,%rcx,8)";
        let c = last_top_level_comma(l.as_bytes()).unwrap();
        assert_eq!(l[c + 1..].trim(), "16(%rbx,%rcx,8)");
        // Load: the destination is the register; address commas are skipped.
        let l = "movzbl (%r10,%r12), %edx";
        let c = last_top_level_comma(l.as_bytes()).unwrap();
        assert_eq!(l[c + 1..].trim(), "%edx");
        // Plain register-register form is unaffected by the depth tracking.
        let l = "addq %rax, %rbx";
        let c = last_top_level_comma(l.as_bytes()).unwrap();
        assert_eq!(l[c + 1..].trim(), "%rbx");
        // A single-operand form has NO top-level comma; rfind(',') would
        // invent a destination out of the SIB field.
        assert_eq!(last_top_level_comma("incl (%rbx,%rcx,4)".as_bytes()), None);
        assert_eq!(last_top_level_comma(b""), None);
        assert_eq!(last_top_level_comma(b"movl %eax"), None);
        // Unbalanced parentheses must not panic or underflow the depth.
        let _ = last_top_level_comma("movb %al, )%r10,%r12(".as_bytes());
    }

    #[test]
    fn att_source_rewrite_keeps_indexed_memory_destination_intact() {
        // The source region ends at the TOP-LEVEL comma, so the indexed memory
        // destination is never spliced into.
        let out = replace_source_reg_att("  movb %al, (%r10,%r12)", "%al", "%cl").unwrap();
        assert_eq!(out, "  movb %cl, (%r10,%r12)");
        // A register appearing only in the destination is not a source.
        assert!(replace_source_reg_att("  movb %al, (%r10,%r12)", "%r12", "%r13").is_none());
    }

    #[test]
    fn line_store_split_and_roundtrip() {
        let s = LineStore::new("a\nb\nc".into());
        assert_eq!(s.len(), 3);
        assert_eq!(s.get(0), "a");
        assert_eq!(s.get(1), "b");
        assert_eq!(s.get(2), "c");
        assert_eq!(s.build_result(|_| false), "a\nb\nc\n");

        let trailing = LineStore::new("a\nb\n".into());
        assert_eq!(trailing.len(), 2);
        assert_eq!(trailing.get(0), "a");
        assert_eq!(trailing.get(1), "b");

        let empty = LineStore::new(String::new());
        assert_eq!(empty.len(), 1);
        assert_eq!(empty.get(0), "");
    }

    #[test]
    fn line_store_replace_overwrites_slot() {
        let mut s = LineStore::new("a\nb\nc".into());
        s.replace(1, "B1".into());
        s.replace(1, "B2".into());
        assert_eq!(s.get(1), "B2");
        assert_eq!(s.replacements.len(), 1, "second replace must not leak");
        assert_eq!(s.build_result(|i| i == 0), "B2\nc\n");
    }
}
