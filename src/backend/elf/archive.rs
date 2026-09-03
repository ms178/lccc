//! Archive (`.a`) and linker-script parsing.
//!
//! Handles regular and thin GNU/SVR4 archives, plus the `GROUP` / `INPUT`
//! linker-script forms used by distro `libc.so`-style inputs.
//!
//! # Design goals (performance-first, correctness as a hard constraint)
//!
//! - **Zero-copy** member access for regular archives: `(name, offset, size)`
//!   tuples point into the original buffer. Critical for large static
//!   libraries (glibc, libgcc, LLVM, kernel archives).
//! - Robust parsing of real-world archives from GNU `ar`, `llvm-ar`, and
//!   binutils (thin archives, `/SYM64/`, extended names with path separators).
//! - Strict validation — no silent `unwrap_or(0)` sizes that silently pull the
//!   wrong object (miscompilation / wrong link).
//! - Fast path for the archive symbol index (`/` / `/SYM64/`): map undefined
//!   symbols → member-header offsets and only materialize the needed objects.
//!   Major link-time win vs naïve full-member parsing on glibc / libstdc++ /
//!   LLVM-scale static links.
//! - Linker-script extraction for `GROUP`/`INPUT` + `AS_NEEDED`, with
//!   GNU-compatible case-insensitive keywords and `/* ... */` comments
//!   stripped from the whole script *before* directive matching.
//!
//! # Toolchain
//!
//! Edition 2021, MSRV 1.65 (`let`-else). Keep current via `rustup update`.
//!
//! # References
//!
//! - GNU ar / SVR4 archive format (60-byte header, `` `\n `` terminator).
//! - Thin magic `!<thin>\n` (headers + index only; member data on disk).
//! - Symbol table: big-endian count + member-header offsets + NUL-terminated
//!   names (`/` = u32 entries, `/SYM64/` = u64 entries).
//! - Extended name table `//`, entries terminated by `/\n` (LLVM Archive.cpp).
//! - Out of scope: BSD `__.SYMDEF`, AIX big archives, full ld script language
//!   (`SECTIONS` / `PHDRS` — see `linker_common/linker_script.rs`).

use std::borrow::Cow;

// ── Constants ────────────────────────────────────────────────────────────────

/// Fixed GNU/SVR4 archive member header size.
const HEADER_SIZE: usize = 60;

/// Magic terminator at bytes `[58..60]` of every member header.
const HEADER_TERMINATOR: &[u8; 2] = b"`\n";

/// Regular archive magic (8 bytes including newline).
const MAGIC_REGULAR: &[u8; 8] = b"!<arch>\n";

/// Thin archive magic (8 bytes including newline).
const MAGIC_THIN: &[u8; 8] = b"!<thin>\n";

// ── Small helpers ────────────────────────────────────────────────────────────

/// Archives pad member payloads to even byte boundaries (typically with `\n`).
#[inline(always)]
fn align_up_2(pos: usize) -> usize {
    pos.saturating_add(pos & 1)
}

/// Parse a space-padded decimal header field (size / date / uid / …).
///
/// Single checked pass over the raw bytes: no UTF-8 validation and no
/// intermediate `&str` on the success path. Rejects empty, non-decimal, and
/// overflowing content instead of silently becoming `0` (a silent zero yields
/// wrong member boundaries and can pull the wrong object into the link).
#[inline]
fn parse_decimal_field(raw: &[u8]) -> Result<usize, String> {
    let mut start = 0;
    let mut end = raw.len();
    while start < end && raw[start] == b' ' {
        start += 1;
    }
    while end > start && raw[end - 1] == b' ' {
        end -= 1;
    }
    let digits = &raw[start..end];
    if digits.is_empty() {
        return Err("archive header numeric field is empty".to_string());
    }
    let mut value: usize = 0;
    for &b in digits {
        if !b.is_ascii_digit() {
            return Err(format!(
                "invalid archive header decimal field '{}': non-digit character",
                String::from_utf8_lossy(digits)
            ));
        }
        value = match value
            .checked_mul(10)
            .and_then(|v| v.checked_add((b - b'0') as usize))
        {
            Some(v) => v,
            None => {
                return Err(format!(
                    "archive header decimal field '{}' overflows usize",
                    String::from_utf8_lossy(digits)
                ))
            }
        };
    }
    Ok(value)
}

/// Read a big-endian `u32` from a slice of at least 4 bytes.
#[inline(always)]
fn read_be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Read a big-endian `u64` from a slice of at least 8 bytes.
#[inline(always)]
fn read_be_u64(b: &[u8]) -> u64 {
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// Byte-level test for special pseudo-member names (padded with spaces).
#[inline]
fn name_field_is(name_raw: &[u8], special: &[u8]) -> bool {
    if name_raw.len() < special.len() || !name_raw.starts_with(special) {
        return false;
    }
    name_raw[special.len()..].iter().all(|&b| b == b' ')
}

/// 32-bit symbol table member: exactly `/`.
#[inline]
fn is_symbol_32(name_raw: &[u8]) -> bool {
    name_field_is(name_raw, b"/")
}

/// 64-bit symbol table member: exactly `/SYM64/`.
#[inline]
fn is_symbol_64(name_raw: &[u8]) -> bool {
    name_field_is(name_raw, b"/SYM64/")
}

/// GNU extended name table member: exactly `//`.
#[inline]
fn is_extended_name_table(name_raw: &[u8]) -> bool {
    name_field_is(name_raw, b"//")
}

/// Windows SDK / WDK pseudo-members — skipped, never treated as objects.
#[inline]
fn is_windows_pseudo_member(name_raw: &[u8]) -> bool {
    name_field_is(name_raw, b"/<XFGHASHMAP>/") || name_field_is(name_raw, b"/<ECSYMBOLS>/")
}

/// True for members whose payload is inline even in a thin archive.
#[inline]
fn has_inline_payload(name_raw: &[u8], thin: bool) -> bool {
    !thin
        || is_symbol_32(name_raw)
        || is_symbol_64(name_raw)
        || is_extended_name_table(name_raw)
        || is_windows_pseudo_member(name_raw)
}

/// Extract a GNU extended name from the `//` table at `name_off`.
///
/// Terminates at the **first** newline and strips a preceding `/` when
/// present (canonical `name/\n`). This keeps thin-archive paths with interior
/// slashes (`foo/bar.o`) intact — the first `/` must never truncate the name.
/// Matches LLVM `ArchiveMemberHeader::getName` for the `K_GNU` / `K_GNU64`
/// styles.
fn extract_extended_name(ext: &[u8], name_off: usize) -> String {
    if name_off >= ext.len() {
        return String::new();
    }
    let slice = &ext[name_off..];
    let end = match slice.iter().position(|&b| b == b'\n') {
        // Canonical "name/\n"; a bare "name\n" is tolerated the same way.
        Some(nl) if nl >= 1 && slice[nl - 1] == b'/' => nl - 1,
        Some(nl) => nl,
        // No newline: NUL-terminated or runs to the end of the table
        // (defensive; GNU ar always writes `/\n`).
        None => {
            let e = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
            if e >= 1 && slice[e - 1] == b'/' {
                e - 1
            } else {
                e
            }
        }
    };
    String::from_utf8_lossy(&slice[..end]).into_owned()
}

/// Parse a `/<decimal>` extended-name-table offset with GNU `strtol` semantics:
/// consume the leading decimal digits and stop at the first non-digit.
///
/// GNU ar / binutils write long-name references as `/123` (space-padded to 16
/// bytes), but the thin-archive rewrite pass (`ar mPi --thin`) can leave the
/// name field space-padded *after* the terminating `/`, producing forms like
/// `/4611          /`. BFD reads these via plain `strtol` in
/// `get_extended_arelt_filename`, so trailing slashes, padding spaces and
/// `/NNN:origin` compounds (emitted by binutils thin rewrites) are all parsed
/// by their decimal prefix. The kernel's `vmlinux.a` (produced by
/// `scripts/Makefile.vmlinux_a`'s `ar mPi --thin` rewrite) contains exactly
/// such slash-suffixed references alongside clean `/4635       ` ones; a
/// strict "digits to the end (minus one optional trailing slash)" parser
/// rejects them.
///
/// Returns `None` when there is no leading decimal digit.
#[inline]
fn parse_long_name_offset(rest: &[u8]) -> Option<usize> {
    let mut i = 0;
    let mut value: usize = 0;
    while i < rest.len() && rest[i].is_ascii_digit() {
        value = value
            .checked_mul(10)?
            .checked_add((rest[i] - b'0') as usize)?;
        i += 1;
    }
    (i > 0).then_some(value)
}

/// Resolve a member name from the raw 16-byte name field + optional `//` table.
///
/// Byte-level on the hot path: no UTF-8 conversion until the final name is
/// known. Handles short names (`foo.o/` → `foo.o`) and long-name references
/// (`/123`, `/123/`). Special names (`/`, `//`, `/SYM64/`, Windows pseudos)
/// must be filtered by the caller before this runs.
fn resolve_member_name(name_raw: &[u8], extended_names: Option<&[u8]>) -> String {
    // Right-trim space padding.
    let mut end = name_raw.len();
    while end > 0 && name_raw[end - 1] == b' ' {
        end -= 1;
    }
    let field = &name_raw[..end];

    if field.first() == Some(&b'/') {
        let rest = &field[1..];
        if !rest.is_empty() {
            if let (Some(ext), Some(off)) = (extended_names, parse_long_name_offset(rest)) {
                return extract_extended_name(ext, off);
            }
        }
        // Missing `//` table or non-decimal reference: surface the raw field
        // so the corruption stays visible instead of being silently renamed.
        return String::from_utf8_lossy(field).into_owned();
    }

    // Short name: GNU ar appends a `/` terminator.
    if end > 0 && field[end - 1] == b'/' {
        end -= 1;
    }
    String::from_utf8_lossy(&field[..end]).into_owned()
}

// ── Member walker (single source of truth for header semantics) ─────────────

/// One raw archive member as laid out in the file.
struct RawMember<'a> {
    /// The 16-byte raw name field (uninterpreted).
    name_raw: &'a [u8],
    /// Offset of the payload (header offset + 60).
    data_start: usize,
    /// Declared payload size from the header.
    size: usize,
}

/// Single-pass walker over archive member headers.
///
/// Shared by every public parser so header semantics — terminator check,
/// strict decimal parsing, thin-vs-regular payload advancement, even-byte
/// alignment — live in exactly one place. Bad terminators stop iteration
/// cleanly (trailing padding / truncation); a structurally valid header with
/// a malformed size is a hard error (corruption must not pass silently).
struct MemberIter<'a> {
    data: &'a [u8],
    pos: usize,
    thin: bool,
}

impl<'a> MemberIter<'a> {
    fn new(data: &'a [u8], thin: bool) -> Self {
        Self { data, pos: 8, thin }
    }
}

impl<'a> Iterator for MemberIter<'a> {
    type Item = Result<RawMember<'a>, String>;

    fn next(&mut self) -> Option<Self::Item> {
        let end = self.pos.saturating_add(HEADER_SIZE);
        if end > self.data.len() {
            return None;
        }
        let data = self.data;
        let header = &data[self.pos..end];
        if &header[58..60] != HEADER_TERMINATOR {
            return None; // trailing padding or truncation — stop cleanly
        }
        let size = match parse_decimal_field(&header[48..58]) {
            Ok(s) => s,
            Err(e) => return Some(Err(e)),
        };
        let data_start = self.pos + HEADER_SIZE;
        let name_raw = &header[0..16];
        self.pos = advance_pos(
            data_start,
            size,
            self.thin,
            has_inline_payload(name_raw, self.thin),
        );
        Some(Ok(RawMember {
            name_raw,
            data_start,
            size,
        }))
    }
}

/// Advance past a member payload to the next header position.
///
/// In thin archives only pseudo-members with inline payloads (`/`, `/SYM64/`,
/// `//`, Windows pseudo-members) contribute `size` bytes; regular thin members
/// are header-only. Regular-archive members always contribute `size` bytes.
/// Payloads are padded to an even boundary (usually with `\n`).
///
/// Progress is structural: `data_start = header_offset + 60 > header_offset`,
/// so the returned position always moves forward — no explicit guard needed.
#[inline]
fn advance_pos(data_start: usize, size: usize, thin: bool, inline_payload: bool) -> usize {
    let payload = if thin && !inline_payload { 0 } else { size };
    let next = data_start.saturating_add(payload);
    next.saturating_add(next & 1)
}

/// The inline payload slice of a member, if fully present in `data`.
#[inline]
fn payload_range<'a>(data: &'a [u8], m: &RawMember<'_>) -> Option<&'a [u8]> {
    let end = m.data_start.checked_add(m.size)?;
    data.get(m.data_start..end)
}

// ── Public archive predicates & parsers ──────────────────────────────────────

/// Returns `true` if `data` starts with the thin archive magic `!<thin>\n`.
#[inline]
#[must_use]
pub fn is_thin_archive(data: &[u8]) -> bool {
    data.starts_with(MAGIC_THIN)
}

/// Returns `true` if `data` starts with the regular archive magic `!<arch>\n`.
#[inline]
#[must_use]
pub fn is_regular_archive(data: &[u8]) -> bool {
    data.starts_with(MAGIC_REGULAR)
}

/// Parse a GNU thin archive (`.a` with `!<thin>\n` magic).
///
/// Returns member filenames (often relative paths containing `/`). Member
/// **data is not stored inline** — only headers, the name table, and the
/// symbol index. The caller must `fs::read` each path, resolved relative to
/// the archive's own directory.
///
/// Skips the symbol tables (`/`, `/SYM64/`), the extended name table (`//`),
/// and Windows pseudo-members.
pub fn parse_thin_archive_members(data: &[u8]) -> Result<Vec<String>, String> {
    if !is_thin_archive(data) {
        return Err("not a thin archive file".to_string());
    }
    let mut members = Vec::with_capacity((data.len() / 96).clamp(4, 8192));
    let mut extended_names: Option<&[u8]> = None;

    for m in MemberIter::new(data, true) {
        let m = m?;
        let name_raw = m.name_raw;

        if is_symbol_32(name_raw) || is_symbol_64(name_raw) || is_windows_pseudo_member(name_raw) {
            continue; // inline in thin archives; already skipped by the walker
        }
        if is_extended_name_table(name_raw) {
            if let Some(table) = payload_range(data, &m).filter(|t| !t.is_empty()) {
                extended_names = Some(table);
            }
            continue;
        }

        let member_name = resolve_member_name(name_raw, extended_names);
        if !member_name.is_empty() {
            members.push(member_name);
        }
    }

    Ok(members)
}

/// Parse a GNU-format static archive (`.a` with `!<arch>\n` magic).
///
/// Returns `(name, data_offset, data_size)` tuples; offsets point into the
/// original `data` slice → **zero-copy** member access via
/// [`archive_member_data`].
///
/// Handles the extended name table (`//`), 32/64-bit symbol tables
/// (`/`, `/SYM64/` — skipped here; use [`parse_archive_symbol_table`] for the
/// index), even-byte alignment padding, and truncated trailing members
/// (omitted when the declared payload would overrun the file).
pub fn parse_archive_members(data: &[u8]) -> Result<Vec<(String, usize, usize)>, String> {
    if !is_regular_archive(data) {
        return Err("not a valid archive file".to_string());
    }
    let mut members = Vec::with_capacity((data.len() / 96).clamp(4, 8192));
    let mut extended_names: Option<&[u8]> = None;

    for m in MemberIter::new(data, false) {
        let m = m?;
        let name_raw = m.name_raw;

        if is_symbol_32(name_raw) || is_symbol_64(name_raw) || is_windows_pseudo_member(name_raw) {
            continue;
        }
        if is_extended_name_table(name_raw) {
            if let Some(table) = payload_range(data, &m).filter(|t| !t.is_empty()) {
                extended_names = Some(table);
            }
            continue;
        }

        // Emit only members whose full payload is present (robust vs truncation).
        let member_name = resolve_member_name(name_raw, extended_names);
        if !member_name.is_empty() {
            if let Some(payload) = payload_range(data, &m) {
                members.push((member_name, m.data_start, payload.len()));
            }
        }
    }

    Ok(members)
}

/// Parse the GNU archive symbol index (`/` or `/SYM64/` member).
///
/// Returns `(symbol_name, member_header_offset)` pairs where the offset
/// points at the **60-byte header** of the defining member inside `data`.
/// Offsets are taken from the index verbatim; paranoid callers can validate
/// them against [`parse_archive_members`] output.
///
/// Format (big-endian): `u32`/`u64` count, `count` offsets to member headers,
/// then NUL-terminated symbol names. Works for regular and thin archives
/// (the index is always inline). Returns an empty vec when no symbol table is
/// present (valid: `ranlib` not run).
pub fn parse_archive_symbol_table(data: &[u8]) -> Result<Vec<(String, usize)>, String> {
    let thin = is_thin_archive(data);
    if !thin && !is_regular_archive(data) {
        return Err("not a valid archive file".to_string());
    }

    for m in MemberIter::new(data, thin) {
        let m = m?;
        let is_64 = is_symbol_64(m.name_raw);
        if is_64 || is_symbol_32(m.name_raw) {
            let payload = payload_range(data, &m)
                .ok_or_else(|| "archive symbol table member is truncated".to_string())?;
            return parse_symbol_table_payload(payload, is_64);
        }
    }

    Ok(Vec::new())
}

fn parse_symbol_table_payload(symtab: &[u8], is_64: bool) -> Result<Vec<(String, usize)>, String> {
    // Header: count, then `count` big-endian offsets, then NUL-terminated names.
    let (count, base, width) = if is_64 {
        if symtab.len() < 8 {
            return Err("SYM64 symbol table too small".to_string());
        }
        let count = read_be_u64(symtab) as usize;
        // Bound the count by what the payload can possibly hold: ≥ 8 bytes of
        // offset per entry (memory-DoS guard on hostile inputs).
        let max_count = (symtab.len() - 8) / 8;
        if count > max_count {
            return Err(format!(
                "SYM64 symbol count {count} exceeds payload capacity {max_count}"
            ));
        }
        (count, 8, 8)
    } else {
        if symtab.len() < 4 {
            return Err("symbol table too small".to_string());
        }
        let count = read_be_u32(symtab) as usize;
        let max_count = (symtab.len() - 4) / 4;
        if count > max_count {
            return Err(format!(
                "symbol count {count} exceeds payload capacity {max_count}"
            ));
        }
        (count, 4, 4)
    };

    // `count * width <= symtab.len() - base`, so no overflow is possible here.
    let offsets_end = base + count * width;
    let offsets = &symtab[base..offsets_end];

    let mut result = Vec::with_capacity(count);
    let mut str_off = offsets_end;

    for i in 0..count {
        let member_off = if is_64 {
            read_be_u64(&offsets[i * 8..i * 8 + 8]) as usize
        } else {
            read_be_u32(&offsets[i * 4..i * 4 + 4]) as usize
        };

        if str_off >= symtab.len() {
            return Err(if is_64 {
                "SYM64 symbol table truncated (names)".to_string()
            } else {
                "symbol table truncated (names)".to_string()
            });
        }
        let rest = &symtab[str_off..];
        let name_len = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        if name_len > 0 {
            result.push((
                String::from_utf8_lossy(&rest[..name_len]).into_owned(),
                member_off,
            ));
        }
        // Consume the NUL when present; a missing terminator pushes `str_off`
        // past the end, which the check above reports as truncation.
        str_off = str_off.saturating_add(name_len + 1);
    }

    Ok(result)
}

/// Convenience: member names only (thin → filenames; regular → member names).
#[allow(dead_code)]
pub fn list_archive_member_names(data: &[u8]) -> Result<Vec<String>, String> {
    if is_thin_archive(data) {
        parse_thin_archive_members(data)
    } else if is_regular_archive(data) {
        Ok(parse_archive_members(data)?
            .into_iter()
            .map(|(n, _, _)| n)
            .collect())
    } else {
        Err("not a valid archive file".to_string())
    }
}

/// Zero-copy slice of a regular-archive member payload.
///
/// Returns `None` if the range is out of bounds (thin members have no inline
/// data — use the filesystem path from [`parse_thin_archive_members`]).
#[inline]
#[must_use]
pub fn archive_member_data(data: &[u8], offset: usize, size: usize) -> Option<&[u8]> {
    data.get(offset..offset.checked_add(size)?)
}

// ── Linker script parsing (GROUP / INPUT) ────────────────────────────────────

/// An entry found in a GNU linker script directive (`GROUP` or `INPUT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkerScriptEntry {
    /// Absolute or relative file path
    /// (e.g. `/lib/x86_64-linux-gnu/libc.so.6`, `crt1.o`, `libfoo.a`).
    Path(String),
    /// A `-l` library reference (e.g. `-ltinfo` → `tinfo`).
    Lib(String),
}

/// Parse a GNU linker script for `GROUP` / `INPUT` and return path entries only.
///
/// Prefer [`parse_linker_script_entries`] when library search paths are available.
pub fn parse_linker_script(content: &str) -> Option<Vec<String>> {
    let entries = parse_linker_script_entries(content)?;
    let paths: Vec<String> = entries
        .into_iter()
        .filter_map(|e| match e {
            LinkerScriptEntry::Path(p) => Some(p),
            LinkerScriptEntry::Lib(_) => None,
        })
        .collect();
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

/// Parse a GNU linker script, returning all entries including `-l` references.
///
/// Handles the common real-world form used by distro `libc.so` / `libm.so`:
///
/// ```text
/// GROUP ( /lib/x86_64-linux-gnu/libc.so.6
///         /usr/lib/x86_64-linux-gnu/libc_nonshared.a
///         AS_NEEDED ( /lib64/ld-linux-x86-64.so.2 ) )
/// ```
///
/// - Collects **all** top-level `GROUP`/`INPUT` directives, not just the first.
/// - `/* ... */` comments are stripped from the **whole script** before
///   directive matching (a commented-out `GROUP` is not a directive).
/// - Skips the contents of `AS_NEEDED ( ... )`, nested to any depth
///   (runtime `DT_NEEDED` only — not static-link inputs).
/// - Treats unknown non-keyword tokens as paths (captures bare `crt1.o` etc.).
/// - `-lname` → [`LinkerScriptEntry::Lib`]; `-l:filename` → exact filename as
///   [`LinkerScriptEntry::Path`] (GNU ld semantics).
/// - Keywords matched case-insensitively (GNU ld behaviour); quotes stripped.
///
/// Intentionally a focused extractor, not a full ld script engine
/// (`SECTIONS` / `PHDRS` etc. are out of scope).
pub fn parse_linker_script_entries(content: &str) -> Option<Vec<LinkerScriptEntry>> {
    let content = strip_block_comments(content);

    let mut entries = Vec::new();
    let mut search_from = 0;

    while let Some(rel) = find_directive(&content[search_from..]) {
        let directive_start = search_from + rel;
        let rest = &content[directive_start..];

        // Directive body. A missing '(' or unmatched ')' means this occurrence
        // is not a usable directive — resume scanning just past the keyword.
        let Some(open) = rest.find('(') else {
            search_from = directive_start + 1;
            continue;
        };
        let Some(close) = find_matching_paren(rest, open) else {
            search_from = directive_start + 1;
            continue;
        };

        collect_region_entries(&rest[open + 1..close], &mut entries);
        search_from = directive_start + close + 1;
    }

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Find the index of the `)` matching the `(` at `open` (nesting-aware).
fn find_matching_paren(s: &str, open: usize) -> Option<usize> {
    let mut depth: usize = 0;
    for (i, ch) in s[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Classify the tokens of one `GROUP`/`INPUT` body into entries.
fn collect_region_entries(region: &str, entries: &mut Vec<LinkerScriptEntry>) {
    let mut as_needed_depth: usize = 0;

    for token in tokenize_linker_script_region(region) {
        match token {
            "(" => continue,
            ")" => {
                as_needed_depth = as_needed_depth.saturating_sub(1);
                continue;
            }
            // Counted even while already inside AS_NEEDED so nesting resolves.
            t if "AS_NEEDED".eq_ignore_ascii_case(t) => {
                as_needed_depth += 1;
                continue;
            }
            t if is_ignored_ld_keyword(t) => continue,
            t if as_needed_depth > 0 => continue, // DT_NEEDED only
            t => classify_token(t, entries),
        }
    }
}

/// Turn one non-keyword token into an entry (`-l…` special-cased, else path).
#[inline]
fn classify_token(token: &str, entries: &mut Vec<LinkerScriptEntry>) {
    if let Some(rest) = token.strip_prefix("-l") {
        if let Some(filename) = rest.strip_prefix(':') {
            // `-l:filename` — link against this exact file name.
            if !filename.is_empty() {
                entries.push(LinkerScriptEntry::Path(filename.to_string()));
            }
        } else if !rest.is_empty() {
            entries.push(LinkerScriptEntry::Lib(rest.to_string()));
        }
        return;
    }
    // Everything else is a path: crt*.o, relative, versioned .so, absolute,
    // libfoo.a. Strip surrounding quotes if present.
    let path = token.trim_matches('"').trim_matches('\'');
    if !path.is_empty() {
        entries.push(LinkerScriptEntry::Path(path.to_string()));
    }
}

/// Other ld keywords that may appear inside a directive body; ignored.
#[inline]
fn is_ignored_ld_keyword(token: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "KEEP",
        "SORT",
        "SORT_BY_NAME",
        "SORT_BY_ALIGNMENT",
        "SORT_NONE",
        "EXTERN",
        "OUTPUT_FORMAT",
        "OUTPUT_ARCH",
        "TARGET",
        "SEARCH_DIR",
        "ENTRY",
        "STARTUP",
        "INCLUDE",
        "OUTPUT",
        "GROUP",
        "INPUT",
    ];
    // `str::eq_ignore_ascii_case` compares lengths first, so the scan is cheap
    // and allocation-free (unlike `to_ascii_uppercase` per token).
    KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(token))
}

/// Case-insensitive search for the next `GROUP` or `INPUT` keyword at a word
/// boundary (GNU ld keywords are case-insensitive; `MYGROUP`/`GROUPX` must
/// not match).
fn find_directive(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i + 5 <= len {
        // Fast reject on the lower-cased first byte: only 'G'/'g' or 'I'/'i'
        // can map to the targets.
        match bytes[i] | 0x20 {
            b'g' => {
                if eq_ignore_ascii_case(&bytes[i..i + 5], b"GROUP") && word_boundary(bytes, i, 5) {
                    return Some(i);
                }
            }
            b'i' => {
                if eq_ignore_ascii_case(&bytes[i..i + 5], b"INPUT") && word_boundary(bytes, i, 5) {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// True when `bytes[start..start + word_len]` is not adjacent to identifier
/// bytes on either side.
#[inline]
fn word_boundary(bytes: &[u8], start: usize, word_len: usize) -> bool {
    (start == 0 || !is_ident_byte(bytes[start - 1]))
        && (start + word_len == bytes.len() || !is_ident_byte(bytes[start + word_len]))
}

#[inline(always)]
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Length-first ASCII case-insensitive slice comparison.
#[inline]
fn eq_ignore_ascii_case(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// Strip `/* ... */` block comments, replacing each with a single space so
/// surrounding tokens stay separated (GNU ld comment semantics, non-nested).
///
/// Returns a borrow when no comment opener is present (the common case), so
/// directive scanning pays nothing on comment-free scripts. Runs entirely on
/// bytes and copies only clean runs — multi-byte UTF-8 paths pass through
/// unchanged (never re-encoded per byte).
fn strip_block_comments(s: &str) -> Cow<'_, str> {
    if !s.contains("/*") {
        return Cow::Borrowed(s);
    }

    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut run_start = 0; // start of the pending non-comment run
    let mut i = 0;

    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            out.push_str(&s[run_start..i]);
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push(' ');
            run_start = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[run_start..]);
    Cow::Owned(out)
}

/// Tokenize on whitespace; keep quoted strings intact (quotes included in the
/// token, trimmed later by `classify_token`); emit `(` / `)` as separate
/// tokens so `AS_NEEDED(path)` still splits correctly.
///
/// Tokens borrow from the input — zero per-token allocation.
fn tokenize_linker_script_region(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
        } else if b == b'(' || b == b')' {
            tokens.push(&s[i..i + 1]);
            i += 1;
        } else if b == b'"' || b == b'\'' {
            let quote = b;
            let start = i;
            i += 1;
            while i < len && bytes[i] != quote {
                i += 1;
            }
            i = (i + 1).min(len); // include the closing quote when present
            tokens.push(&s[start..i]);
        } else {
            let start = i;
            i += 1;
            while i < len {
                let b = bytes[i];
                if b.is_ascii_whitespace() || b == b'(' || b == b')' || b == b'"' || b == b'\'' {
                    break;
                }
                i += 1;
            }
            tokens.push(&s[start..i]);
        }
    }
    tokens
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── synthetic archive builders ──

    /// 60-byte member header with `name` (raw, space-padded) and `size`.
    fn member_header(name: &[u8], size: usize) -> [u8; HEADER_SIZE] {
        let mut h = [b' '; HEADER_SIZE];
        let n = name.len().min(16);
        h[..n].copy_from_slice(&name[..n]);
        let size_s = format!("{size:<10}");
        h[48..58].copy_from_slice(size_s.as_bytes());
        h[58..60].copy_from_slice(HEADER_TERMINATOR);
        h
    }

    /// Append header + inline payload + even-alignment pad to `out`.
    fn push_member(out: &mut Vec<u8>, name: &[u8], payload: &[u8]) {
        out.extend_from_slice(&member_header(name, payload.len()));
        out.extend_from_slice(payload);
        if out.len() % 2 != 0 {
            out.push(b'\n');
        }
    }

    /// Append header only (thin member: payload lives on disk).
    fn push_thin_member(out: &mut Vec<u8>, name: &[u8], size: usize) {
        out.extend_from_slice(&member_header(name, size));
    }

    // ── helpers ──

    #[test]
    fn thin_and_regular_magic() {
        assert!(is_thin_archive(b"!<thin>\n........"));
        assert!(!is_thin_archive(b"!<arch>\n........"));
        assert!(is_regular_archive(b"!<arch>\n........"));
        assert!(!is_regular_archive(b"!<thin>\n........"));
        assert!(!is_thin_archive(b"short"));
        assert!(!is_regular_archive(b"short"));
    }

    #[test]
    fn align_and_decimal() {
        assert_eq!(align_up_2(0), 0);
        assert_eq!(align_up_2(1), 2);
        assert_eq!(align_up_2(2), 2);
        assert_eq!(parse_decimal_field(b"123       ").unwrap(), 123);
        assert!(parse_decimal_field(b"          ").is_err());
        assert!(parse_decimal_field(b"12x       ").is_err());
        assert!(parse_decimal_field(b"-1        ").is_err());
        assert!(parse_decimal_field(b"9999999999999999999999").is_err()); // overflow
    }

    #[test]
    fn extended_name_thin_path() {
        // Path with interior '/': must not truncate at the first slash.
        assert_eq!(extract_extended_name(b"foo/bar.o/\n", 0), "foo/bar.o");
        assert_eq!(extract_extended_name(b"plain.o/\n", 0), "plain.o");
        assert_eq!(extract_extended_name(b"nonewline", 0), "nonewline");
        assert_eq!(extract_extended_name(b"x/\ny/\n", 3), "y");
        assert_eq!(extract_extended_name(b"table", 99), "");
    }

    #[test]
    fn empty_regular_archive() {
        let data = b"!<arch>\n";
        assert!(parse_archive_members(data).unwrap().is_empty());
        assert!(parse_archive_symbol_table(data).unwrap().is_empty());
    }

    #[test]
    fn parse_simple_regular_member() {
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_REGULAR);
        push_member(&mut data, b"foo.o/", b"hellopayload");

        let members = parse_archive_members(&data).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].0, "foo.o");
        let slice = archive_member_data(&data, members[0].1, members[0].2).unwrap();
        assert_eq!(slice, b"hellopayload");
    }

    #[test]
    fn symbol_table_32() {
        let mut sym_payload = Vec::new();
        sym_payload.extend_from_slice(&2u32.to_be_bytes());
        sym_payload.extend_from_slice(&100u32.to_be_bytes());
        sym_payload.extend_from_slice(&200u32.to_be_bytes());
        sym_payload.extend_from_slice(b"sym_a\0sym_b\0");

        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_REGULAR);
        push_member(&mut data, b"/", &sym_payload);

        let syms = parse_archive_symbol_table(&data).unwrap();
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0], ("sym_a".to_string(), 100usize));
        assert_eq!(syms[1], ("sym_b".to_string(), 200usize));
    }

    #[test]
    fn symbol_table_64() {
        let mut sym_payload = Vec::new();
        sym_payload.extend_from_slice(&1u64.to_be_bytes());
        sym_payload.extend_from_slice(&300u64.to_be_bytes());
        sym_payload.extend_from_slice(b"big_sym\0");

        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_REGULAR);
        push_member(&mut data, b"/SYM64/", &sym_payload);

        let syms = parse_archive_symbol_table(&data).unwrap();
        assert_eq!(syms, vec![("big_sym".to_string(), 300usize)]);
    }

    #[test]
    fn symbol_index_end_to_end() {
        // Symbol table first (as GNU ar writes it), then a real member.
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_REGULAR);

        // `/` symbol table: one symbol defined by the member that follows.
        let member_header_offset = data.len() + HEADER_SIZE + 12; // 4 + 4 + "foo\0"
        let mut sym = Vec::new();
        sym.extend_from_slice(&1u32.to_be_bytes());
        sym.extend_from_slice(&(member_header_offset as u32).to_be_bytes());
        sym.extend_from_slice(b"foo\0");
        push_member(&mut data, b"/", &sym);

        push_member(&mut data, b"def.o/", b"\0payload");

        let syms = parse_archive_symbol_table(&data).unwrap();
        let members = parse_archive_members(&data).unwrap();
        assert_eq!(members.len(), 1);
        // Index offsets point at member *headers*: data offset − 60.
        assert_eq!(syms, vec![("foo".to_string(), members[0].1 - HEADER_SIZE)]);
    }

    #[test]
    fn extended_names_regular_archive() {
        let table = b"really/long/name.o/\n"; // 21 bytes (odd → padded)
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_REGULAR);
        push_member(&mut data, b"//", table);
        push_member(&mut data, b"/0", b"data"); // reference to offset 0 in `//`

        let members = parse_archive_members(&data).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].0, "really/long/name.o");
        let slice = archive_member_data(&data, members[0].1, members[0].2).unwrap();
        assert_eq!(slice, b"data");
    }

    #[test]
    fn thin_archive_extended_names() {
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_THIN);
        push_member(&mut data, b"//", b"dir/file.o/\n");
        push_thin_member(&mut data, b"/0", 1462); // on-disk size; irrelevant here

        assert_eq!(
            parse_thin_archive_members(&data).unwrap(),
            vec!["dir/file.o".to_string()]
        );
    }

    // ── Linux vmlinux.a / binutils thin-rewrite name-field shapes ──

    /// GNU `strtol` prefix semantics for extended-name refs: leading decimal
    /// digits only, anything after the first non-digit is ignored. Covers the
    /// exact 16-byte name-field shapes found in a Linux 6.18 `vmlinux.a`
    /// produced by GNU ar 2.44's `ar mPi --thin` rewrite:
    /// `/4611          /` (space padding *after* the terminator slash),
    /// `/4628           ` (plain space padding) and `/123:origin` compounds.
    #[test]
    fn parse_long_name_offset_strtol_prefix_semantics() {
        // Slash-suffixed + space-padded (kernel `ar mPi --thin` rewrite).
        assert_eq!(parse_long_name_offset(b"4611          /"), Some(4611));
        // Plain space-padded (normal GNU ar thin write).
        assert_eq!(parse_long_name_offset(b"4628           "), Some(4628));
        // `/NNN:origin` compound (binutils thin rewrites).
        assert_eq!(parse_long_name_offset(b"123:0xabcdef"), Some(123));
        // No leading digit → no reference.
        assert_eq!(parse_long_name_offset(b""), None);
        assert_eq!(parse_long_name_offset(b" 5"), None);
        assert_eq!(parse_long_name_offset(b"x.o"), None);
    }

    /// End-to-end: resolve every vmlinux.a name-field shape against the `//`
    /// extended-name table, exactly as the kernel link does.
    #[test]
    fn parse_vmlinux_a_and_binutils_thin_name_ref() {
        // `//` table: "kernel/sysctl.o/" (16 B) at 0, newline at 16;
        // "arch/x86/entry/syscall_64.o/" (28 B) at 17;
        // "another/name.o/" (15 B) at 46; "objtool.o/" (10 B) at 62.
        let table =
            b"kernel/sysctl.o/\narch/x86/entry/syscall_64.o/\nanother/name.o/\nobjtool.o/\n";

        // Reference shapes as they appear in the 16-byte name fields of
        // real GNU-ar thin archives (vmlinux.a mix). Built as exactly
        // 16 bytes: `{:<15}/` = slash terminator after space padding,
        // `{:<16}` = plain space padding, `{:<16}` on a compound ref.
        let slash0 = format!("{:<15}/", "/0");
        let slash17 = format!("{:<15}/", "/17");
        let compound46 = format!("{:<16}", "/46:3");
        let clean62 = format!("{:<16}", "/62");
        assert!(slash0.len() == 16 && slash17.len() == 16);
        assert!(compound46.len() == 16 && clean62.len() == 16);

        assert_eq!(
            resolve_member_name(slash0.as_bytes(), Some(table)),
            "kernel/sysctl.o"
        );
        assert_eq!(
            resolve_member_name(slash17.as_bytes(), Some(table)),
            "arch/x86/entry/syscall_64.o"
        );
        assert_eq!(
            resolve_member_name(compound46.as_bytes(), Some(table)),
            "another/name.o"
        );
        assert_eq!(
            resolve_member_name(clean62.as_bytes(), Some(table)),
            "objtool.o"
        );

        // Full thin-archive round trip with all shapes mixed.
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_THIN);
        push_member(&mut data, b"//", table);
        push_thin_member(&mut data, slash0.as_bytes(), 1462);
        push_thin_member(&mut data, slash17.as_bytes(), 8216);
        push_thin_member(&mut data, compound46.as_bytes(), 120);
        push_thin_member(&mut data, clean62.as_bytes(), 4096);

        assert_eq!(
            parse_thin_archive_members(&data).unwrap(),
            vec![
                "kernel/sysctl.o".to_string(),
                "arch/x86/entry/syscall_64.o".to_string(),
                "another/name.o".to_string(),
                "objtool.o".to_string(),
            ]
        );
    }

    #[test]
    fn thin_archive_with_symbol_table() {
        let mut sym = Vec::new();
        sym.extend_from_slice(&1u32.to_be_bytes());
        sym.extend_from_slice(&1234u32.to_be_bytes());
        sym.extend_from_slice(b"s\0");

        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_THIN);
        push_member(&mut data, b"/", &sym); // inline even in thin archives
        push_thin_member(&mut data, b"t.o/", 4);

        let syms = parse_archive_symbol_table(&data).unwrap();
        assert_eq!(syms, vec![("s".to_string(), 1234usize)]);
        assert_eq!(
            parse_thin_archive_members(&data).unwrap(),
            vec!["t.o".to_string()]
        );
    }

    #[test]
    fn truncated_member_omitted() {
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_REGULAR);
        data.extend_from_slice(&member_header(b"lost.o/", 999)); // no payload
                                                                 // Declared overrun → member skipped, no error, iteration terminates.
        assert!(parse_archive_members(&data).unwrap().is_empty());
    }

    #[test]
    fn alignment_padding_two_members() {
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_REGULAR);
        push_member(&mut data, b"a.o/", b"abc"); // odd payload → one pad byte
        push_member(&mut data, b"b.o/", b"defg");

        let members = parse_archive_members(&data).unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].0, "a.o");
        assert_eq!(members[1].0, "b.o");
        // payload + 1 pad byte + next 60-byte header
        assert_eq!(members[1].1, members[0].1 + members[0].2 + 1 + HEADER_SIZE);
    }

    #[test]
    fn windows_pseudo_members_skipped() {
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_REGULAR);
        push_member(&mut data, b"/<ECSYMBOLS>/", b"junk");
        push_member(&mut data, b"real.o/", b"ok");

        let members = parse_archive_members(&data).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].0, "real.o");
    }

    #[test]
    fn list_names_dispatch() {
        let mut thin = Vec::new();
        thin.extend_from_slice(MAGIC_THIN);
        push_thin_member(&mut thin, b"x.o/", 1);
        assert_eq!(
            list_archive_member_names(&thin).unwrap(),
            vec!["x.o".to_string()]
        );

        let mut reg = Vec::new();
        reg.extend_from_slice(MAGIC_REGULAR);
        push_member(&mut reg, b"y.o/", b"z");
        assert_eq!(
            list_archive_member_names(&reg).unwrap(),
            vec!["y.o".to_string()]
        );

        assert!(list_archive_member_names(b"garbage").is_err());
    }

    // ── linker scripts ──

    #[test]
    fn linker_script_basic_group() {
        let script = r#"
            /* GNU ld script */
            GROUP ( /lib/x86_64-linux-gnu/libc.so.6
                    /usr/lib/x86_64-linux-gnu/libc_nonshared.a
                    AS_NEEDED ( /lib64/ld-linux-x86-64.so.2 ) )
        "#;
        let entries = parse_linker_script_entries(script).expect("entries");
        assert_eq!(entries.len(), 2);
        assert!(matches!(&entries[0], LinkerScriptEntry::Path(p) if p.contains("libc.so.6")));
        assert!(matches!(&entries[1], LinkerScriptEntry::Path(p) if p.contains("nonshared")));
    }

    #[test]
    fn linker_script_case_insensitive() {
        let script = "group ( /lib/foo.so )";
        let entries = parse_linker_script_entries(script).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(matches!(&entries[0], LinkerScriptEntry::Path(p) if p == "/lib/foo.so"));
    }

    #[test]
    fn linker_script_crt_and_lib() {
        let script = "INPUT ( crt1.o crti.o -lc crtn.o )";
        assert_eq!(
            parse_linker_script_entries(script).unwrap(),
            vec![
                LinkerScriptEntry::Path("crt1.o".into()),
                LinkerScriptEntry::Path("crti.o".into()),
                LinkerScriptEntry::Lib("c".into()),
                LinkerScriptEntry::Path("crtn.o".into()),
            ]
        );
    }

    #[test]
    fn linker_script_paths_only_filter() {
        let script = "GROUP ( /lib/foo.so -lbar )";
        assert_eq!(
            parse_linker_script(script).unwrap(),
            vec!["/lib/foo.so".to_string()]
        );
    }

    #[test]
    fn linker_script_as_needed_no_space() {
        let script = "GROUP ( /lib/a.so AS_NEEDED(/lib64/ld.so) /lib/b.a )";
        let entries = parse_linker_script_entries(script).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(&entries[0], LinkerScriptEntry::Path(p) if p.ends_with("a.so")));
        assert!(matches!(&entries[1], LinkerScriptEntry::Path(p) if p.ends_with("b.a")));
    }

    #[test]
    fn linker_script_nested_as_needed() {
        let script = "GROUP ( a.so AS_NEEDED ( AS_NEEDED ( b.so ) ) c.so )";
        assert_eq!(
            parse_linker_script_entries(script).unwrap(),
            vec![
                LinkerScriptEntry::Path("a.so".into()),
                LinkerScriptEntry::Path("c.so".into()),
            ]
        );
    }

    #[test]
    fn linker_script_multiple_directives() {
        let script = "INPUT(a.o) GROUP(b.a -lc)";
        assert_eq!(
            parse_linker_script_entries(script).unwrap(),
            vec![
                LinkerScriptEntry::Path("a.o".into()),
                LinkerScriptEntry::Path("b.a".into()),
                LinkerScriptEntry::Lib("c".into()),
            ]
        );
    }

    #[test]
    fn linker_script_comments_and_quotes() {
        let script = "/* GROUP ( hidden.so ) */\n\
                      GROUP ( a.o /* inline */ \"/opt/my lib/libb.so\" )";
        assert_eq!(
            parse_linker_script_entries(script).unwrap(),
            vec![
                LinkerScriptEntry::Path("a.o".into()),
                LinkerScriptEntry::Path("/opt/my lib/libb.so".into()),
            ]
        );
    }

    #[test]
    fn linker_script_word_boundaries() {
        // `MYGROUP` and `GROUPX` must not match.
        assert!(parse_linker_script_entries("MYGROUP ( a.o )").is_none());
        assert!(parse_linker_script_entries("GROUPX ( a.o )").is_none());
        // A commented-out directive is not a directive.
        assert!(parse_linker_script_entries("/* GROUP ( a.o ) */").is_none());
    }

    #[test]
    fn linker_script_l_colon_and_dash_l() {
        let script = "INPUT ( -l:libfoo.so.1 -lz )";
        assert_eq!(
            parse_linker_script_entries(script).unwrap(),
            vec![
                LinkerScriptEntry::Path("libfoo.so.1".into()),
                LinkerScriptEntry::Lib("z".into()),
            ]
        );
    }

    // ── malformed input ──

    #[test]
    fn reject_bad_size_field() {
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC_REGULAR);
        data.extend_from_slice(&member_header(b"x.o/", 0));
        // Overwrite the size field with exactly 10 non-digit bytes.
        let n = data.len();
        data[n - 12..n - 2].copy_from_slice(b"notadecima");

        assert!(parse_archive_members(&data).is_err());
    }
}
