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
    if i == bytes.len() {
        None
    } else {
        Some(i)
    }
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
    splice_source_rewrite(line, trimmed, src_lo, trimmed.len(), old_reg, new_reg)
}

/// Replace `old_reg` in source operands of an **AT&T** instruction.
///
/// `addq %rax, %rbx`: dest is the last comma-separated operand. Sources
/// are everything before that comma. `pushq %rax` (no comma) returns
/// `None` — the single operand is both source and dest.
#[allow(dead_code)]
pub(crate) fn replace_source_reg_att(line: &str, old_reg: &str, new_reg: &str) -> Option<String> {
    if old_reg.is_empty() || old_reg == new_reg {
        return None;
    }
    let trimmed = line.trim();
    let op_at = operands_start(trimmed)?;
    let args = &trimmed[op_at..];
    let comma = args.rfind(',')?;
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
    pub(crate) fn new(asm: String) -> Self {
        let bytes = asm.as_bytes();
        let total = bytes.len();
        let mut entries = Vec::with_capacity(total / 20 + 1);
        let mut start = 0usize;
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\n' {
                entries.push(pack_span(start, i - start));
                start = i + 1;
            }
        }
        if start < total || entries.is_empty() {
            entries.push(pack_span(start, total - start));
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
    pub(crate) fn replace(&mut self, idx: usize, new_text: String) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let src = "add x1, x2  // café";
        let out = replace_whole_word(src, "x1", "x9");
        assert_eq!(out, "add x9, x2  // café");
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
