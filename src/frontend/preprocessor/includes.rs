//! Handles `#include` and `#include_next` directive resolution, file path lookup,
//! and synthetic header declaration injection.

use std::path::{Path, PathBuf};

use super::macro_defs::{MacroDef, parse_define};
use super::pipeline::Preprocessor;

/// Maximum recursive inclusion depth, matching GCC's default of 200.
/// Prevents infinite inclusion loops in files without `#pragma once`.
const MAX_INCLUDE_DEPTH: usize = 200;

/// C23 `__has_embed` result, matching the `__STDC_EMBED_*` macro values.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EmbedResult {
    /// `__STDC_EMBED_NOT_FOUND__`
    NotFound = 0,
    /// `__STDC_EMBED_FOUND__`
    Found = 1,
    /// `__STDC_EMBED_EMPTY__`
    Empty = 2,
}

/// Parsed C23 `#embed` parameter sequence.
#[derive(Default)]
pub(super) struct EmbedParams {
    pub limit: Option<usize>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub if_empty: Option<String>,
    /// Clang vendor extension `clang::offset(N)`: skip the first N bytes of
    /// the resource before `limit` applies. Recognized because the corpus is
    /// Clang-derived and the feature-detection tests probe it.
    pub offset: Option<usize>,
}

/// Failure classes of `#embed`/`__has_embed` argument parsing. `#embed`
/// diagnoses all of them; `__has_embed` only diagnoses the syntax errors
/// (an UNKNOWN parameter is the standard feature-detection channel and must
/// stay silent, yielding `__STDC_EMBED_NOT_FOUND__`).
enum EmbedSpecError {
    NoFilename,
    UnknownParam(String),
    UnterminatedParam(String),
    InvalidLimit(String, String),
    /// A parameter appeared twice (C23 6.10.15: each embed-param may
    /// appear at most once).
    DuplicateParam(String),
    /// Non-parameter garbage after the parameter list.
    TrailingGarbage(String),
}

/// Push a byte as decimal ASCII without allocations or divisions by
/// variable denominators: branch on digit count, then fixed 10/100 steps.
/// Hot path of `#embed` expansion (runs once per embedded byte).
#[inline]
fn push_u8_decimal(out: &mut String, b: u8) {
    if b >= 100 {
        out.push(char::from(b'0' + b / 100));
        out.push(char::from(b'0' + (b / 10) % 10));
        out.push(char::from(b'0' + b % 10));
    } else if b >= 10 {
        out.push(char::from(b'0' + b / 10));
        out.push(char::from(b'0' + b % 10));
    } else {
        out.push(char::from(b'0' + b));
    }
}

/// Read only the prefix of `path` that `#embed` can possibly emit:
/// `offset + limit` bytes when a limit is present (the expansion skips
/// `offset` and takes at most `limit`), the whole file otherwise. This is
/// the bounded-I/O contract of the directive — embedding a 16-byte header
/// of a multi-megabyte blob must not slurp the blob — while leaving the
/// offset/limit slicing to `expand_embed_bytes` (exactly one owner of that
/// arithmetic; a second offset here double-skipped and truncated output).
/// The read loop tolerates short reads and EINTR per POSIX.
fn read_embed_bounded(
    path: &std::path::Path,
    offset: usize,
    limit: Option<usize>,
) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let Some(n) = limit.map(|l| offset.saturating_add(l)) else {
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        return Ok(buf);
    };
    let mut buf = vec![0u8; n];
    let mut filled = 0usize;
    while filled < n {
        match f.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(k) => filled += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    buf.truncate(filled);
    Ok(buf)
}

/// Parse a C integer preprocessing number or character literal as used in
/// `limit`/`clang::offset` embed parameters: decimal, 0x/0X hex, 0b/0B
/// binary, 0-prefixed octal, optional u/U/l/L suffixes, or a char literal
/// (simple and hex/octal escapes). Rejects any trailing garbage so
/// `limit(12abc)` or `limit(1 + 2)` are invalid, matching Clang's
/// single-pp-number contract (the spec requires a preprocessing number,
/// not an arbitrary constant expression).
fn parse_embed_int(text: &str) -> Option<u64> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with('\'') {
        let inner = s.strip_suffix('\'')?.strip_prefix('\'')?;
        let b = inner.as_bytes();
        if b.is_empty() {
            return None;
        }
        if b[0] != b'\\' {
            // Single unescaped character.
            return if b.len() == 1 {
                Some(b[0] as u64)
            } else {
                None
            };
        }
        if b.len() < 2 {
            return None;
        }
        if b[1] == b'x' || b[1] == b'X' {
            let hex = std::str::from_utf8(&b[2..]).ok()?;
            if hex.is_empty() {
                return None;
            }
            return u64::from_str_radix(hex, 16).ok();
        }
        if b[1].is_ascii_digit() {
            // Octal escape: \0, \07, \077, ...
            let oct = std::str::from_utf8(&b[1..]).ok()?;
            if !oct.bytes().all(|c| (b'0'..=b'7').contains(&c)) {
                return None;
            }
            return u64::from_str_radix(oct, 8).ok();
        }
        let simple: Option<u64> = match b[1] {
            b'n' => Some(0x0a),
            b't' => Some(0x09),
            b'r' => Some(0x0d),
            b'a' => Some(0x07),
            b'b' => Some(0x08),
            b'f' => Some(0x0c),
            b'v' => Some(0x0b),
            b'\'' => Some(0x27),
            b'"' => Some(0x22),
            b'\\' => Some(0x5c),
            b'?' => Some(0x3f),
            _ => None,
        };
        if let Some(v) = simple {
            return if b.len() == 2 { Some(v) } else { None };
        }
        return None;
    }
    let (digits, radix) = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (h, 16)
    } else if let Some(bn) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        (bn, 2)
    } else if s.len() > 1 && s.starts_with('0') && s.bytes().all(|c| c.is_ascii_digit()) {
        (s, 8)
    } else {
        (s, 10)
    };
    let mut digits = digits;
    // Strip integer suffixes (u/U/l/L in any order, at most one of each
    // width class is valid C, but be permissive: any trailing ulUL run).
    while digits
        .bytes()
        .last()
        .is_some_and(|c| matches!(c, b'u' | b'U' | b'l' | b'L'))
    {
        digits = &digits[..digits.len() - 1];
    }
    if digits.is_empty() {
        return None;
    }
    u64::from_str_radix(digits, radix).ok()
}

/// Compiler-reserved x86 intrinsic headers supplied with LCCC.  They need to
/// win over a user-supplied GCC builtin include directory: modern GCC's
/// `immintrin.h` unconditionally pulls AVX-512/_Float16 declarations that the
/// C frontend intentionally does not model yet, causing an unrelated AVX2
/// feature probe to fail before it reaches any AVX2 intrinsic.
///
/// This is intentionally a narrow reserved-header mechanism rather than a
/// global reordering of `-I`: project headers and ordinary system headers
/// retain the standard user-path precedence rules.  The check is two-tier:
/// 1) an explicit allowlist for the common umbrella headers, and
/// 2) a dynamic fallback for any `*intrin.h` that actually exists in the
/// bundled directory.  This covers newer VAES/VPCLMULQDQ/GFNI/CET headers
/// without hard-coding an ever-growing list, while still avoiding a blanket
/// “all system headers win” policy that would break user -I shadowing for
/// non-intrinsic files.
const BUNDLED_X86_INTRINSIC_HEADERS: &[&str] = &[
    "immintrin.h",
    "x86intrin.h",
    "xmmintrin.h",
    "emmintrin.h",
    "pmmintrin.h",
    "tmmintrin.h",
    "smmintrin.h",
    "nmmintrin.h",
    "avxintrin.h",
    "avx2intrin.h",
    "avx512fintrin.h",
    "fmaintrin.h",
    "bmi2intrin.h",
    "mmintrin.h",
    "shaintrin.h",
    "vaesintrin.h",
    "vpclmulqdqintrin.h",
    "gfniintrin.h",
    "cetintrin.h",
    "avx512bwintrin.h",
    "avx512cdintrin.h",
    "avx512dqintrin.h",
    "avx512vlintrin.h",
];

#[inline]
fn is_bundled_x86_intrinsic_header(include_path: &str) -> bool {
    if BUNDLED_X86_INTRINSIC_HEADERS.contains(&include_path) {
        return true;
    }
    // Dynamic fallback: any *intrin.h that is known to be bundled should be
    // treated as compiler-reserved.  This keeps the list from going stale as
    // new ISA extensions appear, while still not reserving arbitrary headers.
    if include_path.ends_with("intrin.h") {
        // Quick heuristic: intrinsic headers are flat, no directory component
        // in the include path for <*.h> includes.  Avoid reserving something
        // like "foo/barintrin.h" that is not an LCCC-owned intrinsic.
        if !include_path.contains('/') && !include_path.contains('\\') {
            return true;
        }
    }
    false
}

/// Detect if a source file has a classic include guard pattern.
///
/// The pattern we detect is:
///   - First non-blank, non-comment directive is `#ifndef GUARD_MACRO`
///   - Second directive is `#define GUARD_MACRO` (same macro name, no value or any value)
///   - Last directive is `#endif`
///   - No code/tokens exist outside the `#ifndef`/`#endif` wrapper
///
/// Returns `Some(guard_macro_name)` if the pattern is detected, `None` otherwise.
///
/// This function operates on the raw source text (before preprocessing) and
/// uses a lightweight scan that doesn't require full lexing. It handles:
///   - C-style (`/* ... */`) and C++-style (`// ...`) comments
///   - Line continuations (`\` at end of line)
///   - Whitespace and blank lines before/after the guard
///
/// Intentionally conservative: returns None for anything unusual (e.g., code
/// before the `#ifndef`, `#else` branches, multiple `#endif`s at the same level).
fn detect_include_guard(source: &str) -> Option<String> {
    // We scan directive lines at the top level. We need to track:
    // 1. Whether we've seen #ifndef MACRO as the first directive
    // 2. Whether the next directive is #define MACRO
    // 3. Whether #endif is the last directive at depth 0
    // 4. That no non-whitespace content exists outside the guard

    let mut guard_macro: Option<String> = None;
    let mut found_ifndef = false;
    let mut found_define = false;
    let mut found_endif = false;
    let mut if_depth: i32 = 0;
    let mut has_content_before_guard = false;
    let mut has_content_after_endif = false;
    let mut in_block_comment = false;

    for raw_line in source.lines() {
        // Handle block comments that span lines
        let line = if in_block_comment {
            if let Some(end_pos) = raw_line.find("*/") {
                in_block_comment = false;
                &raw_line[end_pos + 2..]
            } else {
                continue;
            }
        } else {
            raw_line
        };

        // Strip block comments within the line (simple single-line handling)
        let line = strip_inline_comments(line, &mut in_block_comment);
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // If we already saw the final #endif, any non-empty line means
        // there's content after the guard -> not a valid include guard
        if found_endif {
            has_content_after_endif = true;
            break;
        }

        if let Some(after_hash_raw) = trimmed.strip_prefix('#') {
            let after_hash = after_hash_raw.trim_start();

            // Handle line continuations in directives
            let after_hash = if after_hash.ends_with('\\') {
                // For guard detection, we only care about simple single-line directives.
                // Multi-line #define is fine as long as the macro name is on the first line.
                after_hash.trim_end_matches('\\').trim_end()
            } else {
                after_hash
            };

            if after_hash.starts_with("ifndef") && !found_ifndef && if_depth == 0 {
                // First directive must be #ifndef
                if has_content_before_guard {
                    return None;
                }
                let rest = after_hash["ifndef".len()..].trim();
                let macro_name = extract_identifier(rest)?;
                if macro_name.is_empty() {
                    return None;
                }
                guard_macro = Some(macro_name);
                found_ifndef = true;
                if_depth = 1;
            } else if !found_ifndef {
                // First directive is not #ifndef -> no guard
                return None;
            } else if !found_define && found_ifndef && if_depth == 1 {
                // Second directive should be #define with the same macro
                if after_hash.starts_with("define") {
                    let rest = after_hash.strip_prefix("define").unwrap().trim();
                    let macro_name = extract_identifier(rest)?;
                    if let Some(ref guard) = guard_macro {
                        if macro_name == *guard {
                            found_define = true;
                        } else {
                            return None; // #define of a different macro
                        }
                    }
                } else {
                    return None; // Second directive is not #define
                }
            } else {
                // Inside the guard body - track nesting depth
                if after_hash.starts_with("if") {
                    // Covers #if, #ifdef, #ifndef
                    if_depth += 1;
                } else if after_hash.starts_with("endif") {
                    if_depth -= 1;
                    if if_depth == 0 {
                        // This #endif closes the guard
                        found_endif = true;
                    }
                } else if (after_hash.starts_with("else") || after_hash.starts_with("elif"))
                    && if_depth == 1
                {
                    // An #else/#elif at the outermost guard level means the
                    // header has different behavior on re-inclusion (e.g.,
                    // libev's ev_wrap.h defines macros on first include and
                    // #undef's them on second include via the #else branch).
                    // Such headers must NOT be skipped on re-inclusion.
                    return None;
                }
                // Other directives (#define, #include, #elif, #else, etc.)
                // inside nested #if blocks are fine
            }
        } else {
            // Non-directive, non-empty line
            if !found_ifndef {
                // Content before the #ifndef -> no guard
                has_content_before_guard = true;
            }
            // Content inside the guard is fine
            // Content after the guard (found_endif) is handled at the top of the loop
        }
    }

    if found_ifndef && found_define && found_endif && !has_content_after_endif {
        guard_macro
    } else {
        None
    }
}

/// Extract a C identifier from the beginning of a string.
/// Returns Some(identifier) or None if no valid identifier starts the string.
fn extract_identifier(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return Some(String::new());
    }
    let bytes = s.as_bytes();
    if !super::utils::is_ident_start(bytes[0] as char) {
        return None;
    }
    let mut end = 1;
    while end < bytes.len() && super::utils::is_ident_cont(bytes[end] as char) {
        end += 1;
    }
    Some(s[..end].to_string())
}

/// Strip inline block comments from a single line.
/// Updates `in_block_comment` state for multi-line block comments.
/// Also strips line comments (// ...).
fn strip_inline_comments<'a>(
    line: &'a str,
    in_block_comment: &mut bool,
) -> std::borrow::Cow<'a, str> {
    // Fast path: no comment markers at all
    if !line.contains("/*") && !line.contains("//") {
        return std::borrow::Cow::Borrowed(line);
    }

    let mut result = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if *in_block_comment {
            if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                *in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            *in_block_comment = true;
            i += 2;
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            // Line comment - ignore rest of line
            break;
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    std::borrow::Cow::Owned(result)
}

/// Read a C source file, tolerating non-UTF-8 content.
/// Valid UTF-8 files are returned as-is. Non-UTF-8 bytes are encoded as PUA
/// code points which the lexer decodes back to raw bytes.
fn read_c_source_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(crate::common::encoding::bytes_to_string(bytes))
}

/// Make a path absolute without resolving symlinks.
///
/// Unlike `std::fs::canonicalize`, this preserves symlinks in the path.
/// This is important for `#include "..."` resolution: GCC searches for
/// included files relative to the directory where the including file was
/// found (through symlinks), not relative to the symlink target's directory.
///
/// For example, if `build/local_scan.h -> ../src/local_scan.h` includes
/// `"config.h"`, we should search in `build/` (the symlink's directory),
/// not in `../src/` (the target's directory).
pub(super) fn make_absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        // Clean up . and .. components without resolving symlinks
        clean_path(path)
    } else if let Ok(cwd) = std::env::current_dir() {
        clean_path(&cwd.join(path))
    } else {
        path.to_path_buf()
    }
}

/// Format a path for use in `#line` directives.
///
/// GCC emits relative paths in `#line` directives when the file is under the
/// current working directory, and absolute paths otherwise. We replicate this
/// behavior because some build systems (notably Perl's `makedepend`) parse
/// `#line` directives and add compilation recipes for `.c` dependencies that
/// contain a `/` in their path. Using absolute paths would cause files like
/// `vutil.c` (which is `#include`d from `util.c`) to match the pattern and
/// generate a spurious inline recipe that overrides the correct `.c.o` rule.
pub(super) fn format_path_for_line_directive(path: &Path) -> String {
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(relative) = path.strip_prefix(&cwd) {
            return relative.display().to_string();
        }
    }
    path.display().to_string()
}

/// Clean a path by resolving `.` and `..` components without following symlinks.
fn clean_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => { /* skip */ }
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => {
                result.push(other);
            }
        }
    }
    result
}

/// Normalize a macro-expanded include path by removing spaces inserted during
/// expansion.
///
/// During macro expansion, the preprocessor may insert spaces between tokens for
/// two reasons:
/// 1. Anti-paste spaces to prevent "//" from being lexed as a comment start
///    (see `would_paste_tokens` in macro_defs.rs).
/// 2. Spaces preserved from the original macro body between token positions.
///
/// Both corrupt file paths in computed `#include` directives. For example:
///
///   #define incdir tests/
///   #define funnyname 42test.h
///   #define incname < incdir funnyname >
///   #include incname
///
/// produces "tests/ 42test.h" instead of "tests/42test.h". Removing all spaces
/// fixes this. This should only be called for macro-expanded paths; direct
/// include paths (e.g., `#include "path with spaces/foo.h"`) are not affected.
fn normalize_include_path(path: String) -> String {
    if path.contains(' ') {
        path.replace(' ', "")
    } else {
        path
    }
}

impl Preprocessor {
    /// Handle #include directive. Returns the preprocessed content of the included file,
    /// or None if the include couldn't be resolved (falls back to old behavior).
    /// `line_num` is the 1-based source line and `col` is the 1-based column of `#`
    /// for diagnostics.
    pub(super) fn handle_include(
        &mut self,
        path: &str,
        line_num: usize,
        col: usize,
    ) -> Option<String> {
        let path = path.trim();

        // Expand macros in include path (for computed includes)
        let (path, was_macro_expanded) = if !path.starts_with('<') && !path.starts_with('"') {
            (self.macros.expand_line(path), true)
        } else {
            (path.to_string(), false)
        };
        let path = path.trim();

        let (include_path, is_system) = if path.starts_with('<') {
            let end = path.find('>').unwrap_or(path.len());
            (path[1..end].to_string(), true)
        } else if let Some(rest) = path.strip_prefix('"') {
            let end = rest.find('"').unwrap_or(rest.len());
            (rest[..end].to_string(), false)
        } else {
            (path.to_string(), false)
        };

        // Only normalize paths that were produced by macro expansion, since
        // those may have spurious spaces from token separation. Direct paths
        // (e.g., #include "My Headers/foo.h") should be left as-is.
        let include_path = if was_macro_expanded {
            normalize_include_path(include_path)
        } else {
            include_path
        };

        self.includes.push(include_path.clone());

        // Always inject compiler-builtin macros for well-known headers
        // (e.g., stdarg.h's va_start/va_end, stdbool.h's true/false).
        // These use compiler builtins and should always be available.
        self.inject_builtin_macros_for_header(&include_path);

        if !self.resolve_includes {
            // When not resolving includes, inject all fallback declarations
            // since we won't have any real headers to provide them.
            self.inject_fallback_declarations_for_header(&include_path);
            return None;
        }

        // Resolve the include path to an actual file (and record it for
        // make-dependency output; __has_include probes never reach here)
        if let Some(resolved_path) = self.resolve_and_record_include_path(&include_path, is_system)
        {
            // Check for #pragma once (path and device/inode identity).
            if self.is_pragma_once_file(&resolved_path) {
                return Some(String::new());
            }

            // Check for include guard: if this file has a known guard macro and
            // that macro is still defined, skip re-processing entirely.
            if let Some(guard) = self.include_guard_macros.get(&resolved_path) {
                if self.macros.is_defined(guard) {
                    return Some(String::new());
                }
            }

            // Check for excessive recursive inclusion.
            // Files WITHOUT #pragma once are allowed to be re-included with different
            // macro definitions active (e.g., TCC's x86_64-gen.c includes itself via
            // tcc.h with TARGET_DEFS_ONLY defined). Only block when nesting is excessive.
            {
                let depth = self
                    .include_stack
                    .iter()
                    .filter(|p| *p == &resolved_path)
                    .count();
                if depth >= MAX_INCLUDE_DEPTH {
                    return Some(String::new());
                }
            }

            // Read the file
            match read_c_source_file(&resolved_path) {
                Ok(content) => {
                    // Detect include guard pattern in the raw source before preprocessing.
                    // We do this before preprocessing so we're analyzing the original
                    // structure of the file, not the expanded output.
                    let detected_guard = detect_include_guard(&content);

                    // Push onto include stack
                    self.include_stack.push(resolved_path.clone());

                    // Format path for line directive (relative when under cwd, matching GCC)
                    let display_path = format_path_for_line_directive(&resolved_path);

                    // Update __FILE__ (uses set_file to avoid full MacroDef allocation)
                    let old_file = self.macros.get_file_body().map(|s| s.to_string());
                    self.macros.set_file(format!("\"{}\"", display_path));

                    // Emit line marker for entering the included file
                    // Flag 1 indicates entering a new include file (GCC convention)
                    let mut result = format!("# 1 \"{}\" 1\n", display_path);

                    // Preprocess the included content
                    result.push_str(&self.preprocess_included(&content));

                    // Restore __FILE__
                    if let Some(old) = old_file {
                        self.macros.set_file(old);
                    }

                    // Pop include stack
                    self.include_stack.pop();

                    // Register the include guard for future fast-path skipping.
                    // We do this after preprocessing so that the guard macro is
                    // now defined (the #define inside the file was processed).
                    if let Some(guard) = detected_guard {
                        self.include_guard_macros.insert(resolved_path, guard);
                    }

                    Some(result)
                }
                Err(_) => {
                    // Silently skip unresolvable includes (many system headers
                    // may not be needed if builtins provide their macros).
                    // Inject fallback declarations since the real header failed to load.
                    self.inject_fallback_declarations_for_header(&include_path);
                    None
                }
            }
        } else {
            // Header not found - inject fallback type/extern declarations for
            // well-known standard headers so compilation can still proceed.
            self.inject_fallback_declarations_for_header(&include_path);

            // Emit a fatal error with source location.
            // Compute approximate column of the include path for better diagnostics.
            // The include path token is at col (of '#') + len("include") + whitespace.
            let include_path_col = col + "include ".len();
            self.errors.push(super::pipeline::PreprocessorDiagnostic {
                file: self.current_file(),
                line: line_num,
                col: include_path_col,
                message: format!("{}: No such file or directory", include_path),
            });
            None
        }
    }

    /// Handle #include_next directive (GCC extension).
    /// Searches for the header starting from the next include path after the one
    /// that contained the current file.
    /// `line_num` is the 1-based source line and `col` is the 1-based column of `#`
    /// for diagnostics.
    pub(super) fn handle_include_next(
        &mut self,
        path: &str,
        line_num: usize,
        col: usize,
    ) -> Option<String> {
        let path = path.trim();

        // Parse the include path
        let (include_path, _is_system, was_macro_expanded) = if path.starts_with('<') {
            let end = path.find('>').unwrap_or(path.len());
            (path[1..end].to_string(), true, false)
        } else if let Some(rest) = path.strip_prefix('"') {
            let end = rest.find('"').unwrap_or(rest.len());
            (rest[..end].to_string(), false, false)
        } else {
            // Try macro expansion
            let expanded = self.macros.expand_line(path);
            let expanded = expanded.trim().to_string();
            if expanded.starts_with('<') {
                let end = expanded.find('>').unwrap_or(expanded.len());
                (expanded[1..end].to_string(), true, true)
            } else if let Some(rest) = expanded.strip_prefix('"') {
                let end = rest.find('"').unwrap_or(rest.len());
                (rest[..end].to_string(), false, true)
            } else {
                (expanded, false, true)
            }
        };

        let include_path = if was_macro_expanded {
            normalize_include_path(include_path)
        } else {
            include_path
        };

        if !self.resolve_includes {
            return None;
        }

        // Get the current file path for include_next resolution
        let current_file = self.include_stack.last().cloned();

        // Resolve using include_next semantics (and record for make deps;
        // the system verdict follows the resolved path's directory).
        if let Some(resolved_path) =
            self.resolve_include_next_path(&include_path, current_file.as_ref())
        {
            let system_dir = self.is_system_directory(&resolved_path);
            self.record_dep_file(resolved_path.clone(), system_dir);
            // Check for #pragma once (path and device/inode identity).
            if self.is_pragma_once_file(&resolved_path) {
                return Some(String::new());
            }

            // Check for include guard
            if let Some(guard) = self.include_guard_macros.get(&resolved_path) {
                if self.macros.is_defined(guard) {
                    return Some(String::new());
                }
            }

            // Check for excessive recursive inclusion
            {
                let depth = self
                    .include_stack
                    .iter()
                    .filter(|p| *p == &resolved_path)
                    .count();
                if depth >= MAX_INCLUDE_DEPTH {
                    return Some(String::new());
                }
            }

            // Read and preprocess the file
            match read_c_source_file(&resolved_path) {
                Ok(content) => {
                    let detected_guard = detect_include_guard(&content);

                    self.include_stack.push(resolved_path.clone());

                    // Format path for line directive (relative when under cwd, matching GCC)
                    let display_path = format_path_for_line_directive(&resolved_path);

                    let old_file = self.macros.get_file_body().map(|s| s.to_string());
                    self.macros.set_file(format!("\"{}\"", display_path));

                    // Emit line marker for entering the included file
                    // Flag 1 indicates entering a new include file (GCC convention)
                    let mut result = format!("# 1 \"{}\" 1\n", display_path);

                    result.push_str(&self.preprocess_included(&content));

                    if let Some(old) = old_file {
                        self.macros.set_file(old);
                    }

                    self.include_stack.pop();

                    if let Some(guard) = detected_guard {
                        self.include_guard_macros.insert(resolved_path, guard);
                    }

                    Some(result)
                }
                Err(_) => None,
            }
        } else {
            // Fall back to regular include if include_next can't find it
            self.handle_include(path, line_num, col)
        }
    }

    // ====================================================================
    // C23 #embed / __has_embed (N3018) — binary resource inclusion.
    //
    // `#embed "file" [params]` is replaced by a comma-separated list of
    // integer byte values; `__has_embed(...)` probes the same search and
    // yields __STDC_EMBED_NOT_FOUND__/__STDC_EMBED_FOUND__/
    // __STDC_EMBED_EMPTY__ (0/1/2).  The resource search reuses the
    // #include machinery: quoted names search the including file's
    // directory first, chevron names only the include paths — exactly the
    // contract the corpus tests exercise.
    // ====================================================================

    /// Parse the `<filename> [param(args)...]` tail shared by `#embed` and
    /// `__has_embed`. C23 forms (3)/(5): if the filename is not directly
    /// `"..."`/`<...>`, the tail undergoes macro replacement first
    /// (`#embed __FILE__ limit(2)`), then matching is retried.
    fn parse_embed_spec(
        &mut self,
        text: &str,
    ) -> Result<(String, bool, EmbedParams, String), EmbedSpecError> {
        // Match a string/chevron filename at the start of `s`.
        fn take_filename(s: &str) -> Option<(String, bool, usize)> {
            if let Some(rest) = s.strip_prefix('"') {
                let end = rest.find('"')?;
                Some((rest[..end].to_string(), false, end + 2))
            } else if let Some(rest) = s.strip_prefix('<') {
                let end = rest.find('>')?;
                Some((rest[..end].to_string(), true, end + 2))
            } else {
                None
            }
        }

        let trimmed = text.trim_start();
        let (filename, is_system, after, source) = match take_filename(trimmed) {
            Some((f, sys, consumed)) => (f, sys, consumed, None),
            None => {
                let expanded = self.macros.expand_line(text);
                let trimmed = expanded.trim_start();
                match take_filename(trimmed) {
                    Some((f, sys, consumed)) => (f, sys, consumed, Some(trimmed.to_string())),
                    None => return Err(EmbedSpecError::NoFilename),
                }
            }
        };
        let rest = match &source {
            Some(expanded) => &expanded[after..],
            None => &trimmed[after..],
        };
        let (params, remainder) = self.parse_embed_params(rest)?;
        Ok((filename, is_system, params, remainder.to_string()))
    }

    /// Parse the parameter sequence: `name(tokens)...` pairs with balanced
    /// parentheses. Failures are returned, not diagnosed: `#embed` turns
    /// every variant into an error, while `__has_embed` must silently yield
    /// __STDC_EMBED_NOT_FOUND__ for UNSUPPORTED parameters (that is how
    //  vendor extensions are feature-detected), only diagnosing true
    /// syntax errors.
    /// Returns the parsed parameters plus the UNCONSUMED remainder of the
    /// input: C23 inline `#embed` may be followed by ordinary tokens on the
    /// same line (`{#embed "f"};`), so only the callers that own the whole
    /// line (the directive form, `__has_embed`) may treat leftover text as
    /// an error.
    fn parse_embed_params<'a>(
        &mut self,
        mut rest: &'a str,
    ) -> Result<(EmbedParams, &'a str), EmbedSpecError> {
        let mut params = EmbedParams::default();
        let mut seen = std::collections::BTreeSet::<&'static str>::new();
        loop {
            rest = rest.trim_start();
            // Commas between parameters are optional (Clang accepts both
            // `limit(1), prefix(x)` and whitespace separation).
            while let Some(r) = rest.strip_prefix(',') {
                rest = r.trim_start();
            }
            if rest.is_empty() {
                return Ok((params, rest));
            }
            // Parameter name: identifier, optionally vendor-qualified a::b.
            let name_len = {
                let mut n = 0;
                let b = rest.as_bytes();
                while n < b.len() && (b[n].is_ascii_alphanumeric() || b[n] == b'_') {
                    n += 1;
                }
                if n < b.len() && b[n] == b':' && n + 1 < b.len() && b[n + 1] == b':' {
                    n += 2;
                    while n < b.len() && (b[n].is_ascii_alphanumeric() || b[n] == b'_') {
                        n += 1;
                    }
                }
                n
            };
            if name_len == 0 {
                // Not a parameter: hand the leftover text back to the
                // caller (inline tokens for the splice form, garbage for
                // the whole-line forms).
                return Ok((params, rest));
            }
            let name = &rest[..name_len];
            rest = rest[name_len..].trim_start();
            // Balanced-paren argument, if present. The balance scan is
            // literal-aware: parens inside string/char literals do not
            // count (suffix(, -1) style arguments aside, a vendor param
            // like x(")") must not close early).
            let mut args = String::new();
            if let Some(stripped) = rest.strip_prefix('(') {
                let b = stripped.as_bytes();
                let mut depth = 1usize;
                let mut end = None;
                let mut i = 0usize;
                while i < b.len() {
                    let c = b[i];
                    if c == b'"' || c == b'\'' {
                        let quote = c;
                        i += 1;
                        while i < b.len() && b[i] != quote {
                            if b[i] == b'\\' && i + 1 < b.len() {
                                i += 1;
                            }
                            i += 1;
                        }
                    } else {
                        match c {
                            b'(' => depth += 1,
                            b')' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = Some(i);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    i += 1;
                }
                match end {
                    Some(i) => {
                        args = stripped[..i].to_string();
                        rest = &stripped[i + 1..];
                    }
                    None => {
                        return Err(EmbedSpecError::UnterminatedParam(name.to_string()));
                    }
                }
            }
            // Duplicate detection (each embed-param at most once).
            let canon: &'static str = match name {
                "limit" => "limit",
                "prefix" => "prefix",
                "suffix" => "suffix",
                "if_empty" => "if_empty",
                "clang::offset" => "clang::offset",
                _ => {
                    return Err(EmbedSpecError::UnknownParam(name.to_string()));
                }
            };
            if !seen.insert(canon) {
                return Err(EmbedSpecError::DuplicateParam(canon.to_string()));
            }
            match canon {
                "limit" => match parse_embed_int(&args) {
                    Some(v) => {
                        params.limit = Some(usize::try_from(v).unwrap_or(usize::MAX));
                    }
                    None => {
                        return Err(EmbedSpecError::InvalidLimit(
                            "limit".to_string(),
                            args.trim().to_string(),
                        ));
                    }
                },
                "prefix" => params.prefix = Some(args),
                "suffix" => params.suffix = Some(args),
                "if_empty" => params.if_empty = Some(args),
                _ => match parse_embed_int(&args) {
                    // clang::offset
                    Some(v) => {
                        params.offset = Some(usize::try_from(v).unwrap_or(usize::MAX));
                    }
                    None => {
                        return Err(EmbedSpecError::InvalidLimit(
                            "clang::offset".to_string(),
                            args.trim().to_string(),
                        ));
                    }
                },
            }
        }
    }

    fn embed_diagnose_spec_error(&mut self, e: &EmbedSpecError, line_num: usize, col: usize) {
        let message = match e {
            EmbedSpecError::NoFilename => "expected \"FILENAME\" or <FILENAME>".to_string(),
            EmbedSpecError::UnknownParam(n) => {
                format!("unknown embed preprocessor parameter '{}'", n)
            }
            EmbedSpecError::UnterminatedParam(n) => {
                format!("unterminated embed parameter '{}' argument", n)
            }
            EmbedSpecError::InvalidLimit(n, v) => {
                format!("invalid embed parameter '{}' value '{}'", n, v)
            }
            EmbedSpecError::DuplicateParam(n) => format!("duplicate embed parameter '{}'", n),
            EmbedSpecError::TrailingGarbage(g) => {
                format!("unexpected token '{}' after embed parameters", g)
            }
        };
        self.errors.push(super::pipeline::PreprocessorDiagnostic {
            file: self.current_file(),
            line: line_num,
            col,
            message,
        });
    }

    /// C23 `#embed`: expand to the comma-separated byte list (with
    /// prefix/suffix/if_empty applied). Returns the replacement text, or
    /// None after queueing a diagnostic.
    pub(super) fn expand_embed(
        &mut self,
        rest: &str,
        line_num: usize,
        col: usize,
    ) -> Option<String> {
        let spec = match self.parse_embed_spec(rest) {
            Ok(s) => s,
            Err(e) => {
                self.embed_diagnose_spec_error(&e, line_num, col);
                return None;
            }
        };
        let (filename, is_system, params, remainder) = spec;
        // Whole-line form: the directive owns the rest of the line, so
        // anything that is not embed syntax is malformed input.
        let leftover = remainder.trim();
        if !leftover.is_empty() {
            self.embed_diagnose_spec_error(
                &EmbedSpecError::TrailingGarbage(leftover.chars().take(16).collect()),
                line_num,
                col,
            );
            return None;
        }
        let bytes = self.read_embed_resource(&filename, is_system, &params, line_num, col)?;
        self.expand_embed_bytes(&bytes, params)
    }

    /// Inline (mid-line) `#embed` as spliced by the pipeline: the same
    /// expansion as the directive form, but unconsumed trailing text is
    /// handed back instead of diagnosed, because it is ordinary code.
    pub(super) fn expand_embed_inline(
        &mut self,
        rest: &str,
        line_num: usize,
        col: usize,
    ) -> Option<(String, String)> {
        let spec = match self.parse_embed_spec(rest) {
            Ok(s) => s,
            Err(e) => {
                self.embed_diagnose_spec_error(&e, line_num, col);
                return None;
            }
        };
        let (filename, is_system, params, remainder) = spec;
        let bytes = self.read_embed_resource(&filename, is_system, &params, line_num, col)?;
        let text = self.expand_embed_bytes(&bytes, params)?;
        Some((text, remainder))
    }

    /// Resolve and read an `#embed` resource, bounded by the parameters:
    /// with `limit(N)` only the needed prefix is read from disk (large
    /// binary resources must not pay O(file) I/O for a small slice); with
    /// `clang::offset(K)` the read starts at K. Missing or unreadable
    /// resources diagnose `'FILE' file not found` and yield None.
    fn read_embed_resource(
        &mut self,
        filename: &str,
        is_system: bool,
        params: &EmbedParams,
        line_num: usize,
        col: usize,
    ) -> Option<Vec<u8>> {
        if filename.is_empty() {
            self.errors.push(super::pipeline::PreprocessorDiagnostic {
                file: self.current_file(),
                line: line_num,
                col,
                message: "empty filename".to_string(),
            });
            return None;
        }
        let Some(resolved) = self.resolve_and_record_include_path(filename, is_system) else {
            self.errors.push(super::pipeline::PreprocessorDiagnostic {
                file: self.current_file(),
                line: line_num,
                col,
                message: format!("'{}' file not found", filename),
            });
            return None;
        };
        match read_embed_bounded(&resolved, params.offset.unwrap_or(0), params.limit) {
            Ok(b) => Some(b),
            Err(_) => {
                self.errors.push(super::pipeline::PreprocessorDiagnostic {
                    file: self.current_file(),
                    line: line_num,
                    col,
                    message: format!("'{}' file not found", filename),
                });
                None
            }
        }
    }

    /// Shared byte-list builder: clang::offset(N) skips the first N bytes;
    /// limit bounds the count of what remains; prefix/suffix/if_empty
    /// applied per C23 6.10.15.
    fn expand_embed_bytes(&self, bytes: &[u8], params: EmbedParams) -> Option<String> {
        // clang::offset(N) skips the first N bytes; limit bounds the count
        // of what remains.
        let skip = params.offset.unwrap_or(0).min(bytes.len());
        let remaining = &bytes[skip..];
        let take = match params.limit {
            Some(n) if n < remaining.len() => n,
            _ => remaining.len(),
        };
        if take == 0 {
            // Empty resource (or limit(0)): the if_empty token sequence,
            // which may itself be empty.
            return Some(params.if_empty.unwrap_or_default());
        }
        let prefix_len = params.prefix.as_deref().map_or(0, str::len);
        let suffix_len = params.suffix.as_deref().map_or(0, str::len);
        // Upper bound: 3 digits + ", " per byte.
        let mut out = String::with_capacity(prefix_len + suffix_len + take * 5);
        if let Some(p) = &params.prefix {
            out.push_str(p);
        }
        let mut first = true;
        for &b in &remaining[..take] {
            if !first {
                out.push_str(", ");
            }
            first = false;
            push_u8_decimal(&mut out, b);
        }
        if let Some(sfx) = &params.suffix {
            out.push_str(sfx);
        }
        Some(out)
    }

    /// C23 `__has_embed`: probe without emitting tokens. Unsupported
    /// parameters yield NOT_FOUND silently (feature detection); genuine
    /// syntax errors diagnose and also yield NOT_FOUND.
    pub(super) fn probe_embed(&mut self, rest: &str, diag_line: usize) -> EmbedResult {
        let spec = match self.parse_embed_spec(rest) {
            Ok(s) => s,
            // Unsupported/vendor parameters and non-parameter garbage are
            // the feature-detection channel of __has_embed: silently
            // NOT_FOUND (C23 6.10.15). Malformed syntax diagnoses.
            Err(EmbedSpecError::UnknownParam(_))
            | Err(EmbedSpecError::DuplicateParam(_))
            | Err(EmbedSpecError::TrailingGarbage(_)) => return EmbedResult::NotFound,
            Err(e) => {
                self.embed_diagnose_spec_error(&e, diag_line, 0);
                return EmbedResult::NotFound;
            }
        };
        let (filename, is_system, params, remainder) = spec;
        if !remainder.trim().is_empty() {
            // Unparseable tail: feature-detection channel stays silent.
            return EmbedResult::NotFound;
        }
        if filename.is_empty() {
            self.errors.push(super::pipeline::PreprocessorDiagnostic {
                file: self.current_file(),
                line: diag_line,
                col: 0,
                message: "empty filename".to_string(),
            });
            return EmbedResult::NotFound;
        }
        let Some(resolved) = self.resolve_include_path(&filename, is_system) else {
            return EmbedResult::NotFound;
        };
        if params.limit == Some(0) {
            return EmbedResult::Empty;
        }
        match std::fs::metadata(&resolved) {
            // Directories are not embeddable resources: only regular files
            // report Found/Empty (consistent with the #embed read path,
            // where reading a directory fails).
            Ok(md) if !md.file_type().is_file() => EmbedResult::NotFound,
            Ok(md) if md.len() <= params.offset.unwrap_or(0) as u64 => EmbedResult::Empty,
            Ok(_) => EmbedResult::Found,
            Err(_) => EmbedResult::NotFound,
        }
    }

    /// Resolve an include path using #include_next semantics: search from the
    /// next include path after the one containing the current file.
    /// `current_file` is the full path to the file containing the #include_next.
    pub(super) fn resolve_include_next_path(
        &self,
        include_path: &str,
        current_file: Option<&PathBuf>,
    ) -> Option<PathBuf> {
        // Collect all search paths in order:
        // -iquote -> -I -> -isystem -> default system -> -idirafter
        let all_paths: Vec<&Path> = self
            .quote_include_paths
            .iter()
            .chain(self.include_paths.iter())
            .chain(self.isystem_include_paths.iter())
            .chain(self.system_include_paths.iter())
            .chain(self.after_include_paths.iter())
            .map(|p| p.as_path())
            .collect();

        // Canonicalize the current file path for comparison
        let current_file_canon = current_file.and_then(|f| std::fs::canonicalize(f).ok());

        // Find which search path contains the current file by checking if
        // search_path/include_path resolves to the same file as the current file.
        // This correctly handles subdirectory includes (e.g., sys/types.h).
        let mut found_current = false;
        if let Some(ref cur_canon) = current_file_canon {
            for search_path in &all_paths {
                let candidate = search_path.join(include_path);
                if candidate.is_file() {
                    if let Ok(candidate_canon) = std::fs::canonicalize(&candidate) {
                        if &candidate_canon == cur_canon {
                            found_current = true;
                            continue;
                        }
                    }
                }
                if found_current {
                    let candidate = search_path.join(include_path);
                    if candidate.is_file() {
                        return Some(make_absolute(&candidate));
                    }
                }
            }
        }

        // Fallback: if we couldn't find the current file in any search path,
        // search all paths but skip any that resolve to the current file.
        if !found_current {
            for search_path in &all_paths {
                let candidate = search_path.join(include_path);
                if candidate.is_file() {
                    // Use canonicalize for comparison to detect same-file
                    let candidate_canon = std::fs::canonicalize(&candidate).ok();
                    if let (Some(cur), Some(cand)) = (&current_file_canon, &candidate_canon) {
                        if cur == cand {
                            continue;
                        }
                    }
                    return Some(make_absolute(&candidate));
                }
            }
        }

        None
    }

    /// Resolve an include path to an actual file path.
    /// For "file.h": search current dir, then -I paths, then system paths.
    /// For <file.h>: search -I paths, then system paths.
    ///
    /// Returns the path WITHOUT resolving symlinks, matching GCC behavior.
    /// This ensures that `#include "..."` searches relative to the directory
    /// where the including file was found (through symlinks), not the target.
    ///
    /// Uses a cache to avoid repeated filesystem probing for the same include
    /// path from the same context. The cache key includes the current directory
    /// for quoted includes (since resolution depends on it).
    pub fn resolve_include_path(&mut self, include_path: &str, is_system: bool) -> Option<PathBuf> {
        // Compute cache key: (include_path, is_system, current_dir_for_quoted_includes)
        let current_dir_key = if !is_system {
            self.include_stack
                .last()
                .and_then(|f| f.parent().map(|p| p.to_path_buf()))
                .unwrap_or_default()
        } else {
            PathBuf::new()
        };
        let cache_key = (include_path.to_string(), is_system, current_dir_key);

        if let Some(cached) = self.include_resolve_cache.get(&cache_key) {
            return cached.clone();
        }

        let result = self.resolve_include_path_uncached(include_path, is_system);
        self.include_resolve_cache.insert(cache_key, result.clone());
        result
    }

    /// Record a successfully resolved include for make-dependency output
    /// (-MD/-MMD).  First occurrence wins (GCC lists each file once, even
    /// when guard-less includes reprocess the content); the bool is the
    /// system-directory verdict consumed by the -MMD/-MM filter.
    pub(super) fn record_dep_file(&mut self, path: PathBuf, system_dir: bool) {
        if self.dep_seen.insert(path.clone()) {
            self.dep_files.push((path, system_dir));
        }
    }

    /// True when `path` lives in a SYSTEM directory (-isystem, the default
    /// system search paths, -idirafter, or the compiler's bundled-header
    /// set).  GCC's -MMD/-MM dependency filter is directory-based, and the
    /// verdict must follow the FILE's location — not the directive's
    /// `<>`/`""` spelling and not the search step that matched: a quoted
    /// include resolved next to an including file that itself sits in a
    /// system directory IS a system header (`/usr/include/x.h` including
    /// `"y.h"` from the same directory), which neither spelling- nor
    /// step-based verdicts get right.
    pub(super) fn is_system_directory(&self, path: &Path) -> bool {
        let bundled = Preprocessor::bundled_include_dir();
        for dir in self
            .isystem_include_paths
            .iter()
            .chain(self.system_include_paths.iter())
            .chain(self.after_include_paths.iter())
            .chain(bundled.iter())
        {
            if path.starts_with(dir) {
                return true;
            }
        }
        false
    }

    /// Resolve an include AND record it for dependency output: the entry
    /// point for `#include` / `#include_next`, i.e. files actually opened.
    /// `__has_include` probes must keep using the plain resolver — GCC
    /// does not list probed-but-not-included headers in deps.
    pub(super) fn resolve_and_record_include_path(
        &mut self,
        include_path: &str,
        is_system: bool,
    ) -> Option<PathBuf> {
        let resolved = self.resolve_include_path(include_path, is_system)?;
        let system_dir = self.is_system_directory(&resolved);
        self.record_dep_file(resolved.clone(), system_dir);
        Some(resolved)
    }

    /// Uncached include path resolution. Called by `resolve_include_path` on cache miss.
    fn resolve_include_path_uncached(
        &self,
        include_path: &str,
        is_system: bool,
    ) -> Option<PathBuf> {
        // For quoted includes (#include "..."), search in this order:
        //   1. Current file's directory
        //   2. -iquote paths
        //   3. -I paths
        //   4. -isystem paths
        //   5. Default system paths
        //   6. -idirafter paths
        //
        // For system includes (#include <...>), search in this order:
        //   1. compiler-reserved bundled x86 intrinsic headers
        //   2. -I paths
        //   3. -isystem paths
        //   4. Default system paths
        //   5. -idirafter paths
        //
        // The reserved-header step is active only when the bundled directory
        // remains in the configured system search list, preserving -nostdinc
        // behavior. It avoids GCC's newer unsupported intrinsic umbrella
        // headers shadowing LCCC's ABI-matched intrinsic declarations.
        if is_system && is_bundled_x86_intrinsic_header(include_path) {
            if let Some(bundled) = Preprocessor::bundled_include_dir() {
                if self
                    .system_include_paths
                    .iter()
                    .any(|path| path == &bundled)
                {
                    let candidate = bundled.join(include_path);
                    if candidate.is_file() {
                        return Some(make_absolute(&candidate));
                    }
                }
            }
        }

        if !is_system {
            // Step 1: Search relative to the current file's directory
            if let Some(current_file) = self.include_stack.last() {
                if let Some(current_dir) = current_file.parent() {
                    let candidate = current_dir.join(include_path);
                    if candidate.is_file() {
                        return Some(make_absolute(&candidate));
                    }
                }
            }
            // Also try relative to the original source file directory
            if !self.filename.is_empty() {
                if let Some(parent) = Path::new(&self.filename).parent() {
                    let candidate = parent.join(include_path);
                    if candidate.is_file() {
                        return Some(make_absolute(&candidate));
                    }
                }
            }

            // Step 2: Search -iquote paths (quoted includes only)
            for dir in &self.quote_include_paths {
                let candidate = dir.join(include_path);
                if candidate.is_file() {
                    return Some(make_absolute(&candidate));
                }
            }
        }

        // Step 3: Search -I paths (user directories)
        for dir in &self.include_paths {
            let candidate = dir.join(include_path);
            if candidate.is_file() {
                return Some(make_absolute(&candidate));
            }
        }

        // Step 4: Search -isystem paths (system directories)
        for dir in &self.isystem_include_paths {
            let candidate = dir.join(include_path);
            if candidate.is_file() {
                return Some(make_absolute(&candidate));
            }
        }

        // Step 5: Search default system include paths
        for dir in &self.system_include_paths {
            let candidate = dir.join(include_path);
            if candidate.is_file() {
                return Some(make_absolute(&candidate));
            }
        }

        // Step 6: Search -idirafter paths (system directories, GCC parity)
        for dir in &self.after_include_paths {
            let candidate = dir.join(include_path);
            if candidate.is_file() {
                return Some(make_absolute(&candidate));
            }
        }

        None
    }

    /// Inject compiler-builtin macros for well-known standard headers.
    /// These are always injected regardless of whether the real header is found,
    /// because they expand to compiler builtins (`__builtin_*`) that only our
    /// compiler understands.
    fn inject_builtin_macros_for_header(&mut self, header: &str) {
        match header {
            "stdbool.h" => {
                // Define true/false macros only when stdbool.h is explicitly included
                crate::frontend::preprocessor::builtin_macros::define_stdbool_true_false(
                    &mut self.macros,
                );
            }
            "complex.h" => {
                // C99 <complex.h> support - define standard complex macros
                self.macros
                    .define(parse_define("complex _Complex").expect("static define"));
                self.macros.define(
                    parse_define("_Complex_I (__extension__ 1.0fi)").expect("static define"),
                );
                self.macros
                    .define(parse_define("I _Complex_I").expect("static define"));
                self.macros
                    .define(parse_define("__STDC_IEC_559_COMPLEX__ 1").expect("static define"));
            }
            "stdarg.h" => {
                // Define va_start/va_arg/va_end/va_copy as macros expanding to builtins.
                // These are always needed because they expand to __builtin_* forms.
                // Note: va_list / __gnuc_va_list / __builtin_va_list typedefs are NOT
                // injected as text here. The parser pre-registers them as type names
                // (parse.rs), sema seeds them in type_context.rs, and the IR lowerer
                // provides the target-specific ABI types (types_seed.rs). Injecting
                // typedef text via pending_injections would break if stdarg.h is included
                // from within a nested header, since the injected text gets emitted at
                // the include boundary -- potentially in the middle of an initializer.
                // C23 (N2975) made the second parameter optional, and a
                // variadic function may now have NO named parameter at all
                // (`void f(...)`), so `va_start(ap)` must work. The second
                // argument is not evaluated in any C dialect, and
                // `__builtin_va_start`'s lowering reads only `args.first()`,
                // so one variadic macro serves every dialect: `va_start(ap)`
                // and `va_start(ap, last)` both lower identically.
                self.macros.define(MacroDef {
                    name: "va_start".to_string(),
                    is_function_like: true,
                    params: vec!["ap".to_string()],
                    is_variadic: true,
                    has_named_variadic: false,
                    body: "__builtin_va_start(ap)".to_string(),
                });
                self.macros.define(MacroDef {
                    name: "va_end".to_string(),
                    is_function_like: true,
                    params: vec!["ap".to_string()],
                    is_variadic: false,
                    has_named_variadic: false,
                    body: "__builtin_va_end(ap)".to_string(),
                });
                self.macros.define(MacroDef {
                    name: "va_copy".to_string(),
                    is_function_like: true,
                    params: vec!["dest".to_string(), "src".to_string()],
                    is_variadic: false,
                    has_named_variadic: false,
                    body: "__builtin_va_copy(dest,src)".to_string(),
                });
                // va_arg is special syntax: __builtin_va_arg(ap, type)
                // It's handled by the parser as a special built-in, so we define
                // the macro to expand to __builtin_va_arg which the lexer recognizes.
                self.macros.define(MacroDef {
                    name: "va_arg".to_string(),
                    is_function_like: true,
                    params: vec!["ap".to_string(), "type".to_string()],
                    is_variadic: false,
                    has_named_variadic: false,
                    body: "__builtin_va_arg(ap,type)".to_string(),
                });
                // __gnuc_va_list is also handled natively by the parser/sema/lowerer
                // (see comment above about not injecting typedef text).
            }
            _ => {}
        }
    }

    /// Inject fallback type definitions and extern declarations for standard
    /// headers. Only called when no real header file was found, to provide
    /// minimal definitions so compilation can proceed. When the project provides
    /// its own headers (e.g., dietlibc, musl), these are NOT injected to avoid
    /// conflicting type definitions.
    fn inject_fallback_declarations_for_header(&mut self, header: &str) {
        match header {
            "stdio.h" => {
                // FILE type and standard streams (fallback only)
                self.pending_injections
                    .push("typedef struct _IO_FILE FILE;\n".to_string());
                self.pending_injections
                    .push("extern FILE *stdin;\n".to_string());
                self.pending_injections
                    .push("extern FILE *stdout;\n".to_string());
                self.pending_injections
                    .push("extern FILE *stderr;\n".to_string());
            }
            "errno.h" => {
                // errno is typically a macro expanding to (*__errno_location())
                // but for our purposes, treat it as an extern int
                self.pending_injections
                    .push("extern int errno;\n".to_string());
            }
            "complex.h" => {
                // Declare complex math functions (fallback only)
                self.pending_injections.push(
                    concat!(
                        "double creal(double _Complex __z);\n",
                        "float crealf(float _Complex __z);\n",
                        "long double creall(long double _Complex __z);\n",
                        "double cimag(double _Complex __z);\n",
                        "float cimagf(float _Complex __z);\n",
                        "long double cimagl(long double _Complex __z);\n",
                        "double _Complex conj(double _Complex __z);\n",
                        "float _Complex conjf(float _Complex __z);\n",
                        "long double _Complex conjl(long double _Complex __z);\n",
                        "double cabs(double _Complex __z);\n",
                        "float cabsf(float _Complex __z);\n",
                        "double carg(double _Complex __z);\n",
                        "float cargf(float _Complex __z);\n",
                    )
                    .to_string(),
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod intrinsic_include_tests {
    use super::*;

    #[test]
    fn system_intrinsics_prefer_bundled_headers_over_user_i_path() {
        let bundled = Preprocessor::bundled_include_dir()
            .expect("test tree must contain bundled intrinsic headers");
        let expected = std::fs::canonicalize(bundled.join("immintrin.h"))
            .expect("bundled immintrin.h must exist");

        let unique = format!(
            "lccc_fake_intrin_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos()
        );
        let fake_dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&fake_dir).expect("create fake include directory");
        std::fs::write(fake_dir.join("immintrin.h"), "#error wrong header\n")
            .expect("write fake immintrin.h");

        let mut preprocessor = Preprocessor::new();
        preprocessor.add_include_path(fake_dir.to_str().expect("utf8 temp path"));
        let resolved = preprocessor
            .resolve_include_path("immintrin.h", true)
            .expect("resolve compiler intrinsic header");
        assert_eq!(
            std::fs::canonicalize(resolved).expect("canonical resolved header"),
            expected,
            "compiler-reserved intrinsic headers must not be shadowed by -I GCC paths"
        );

        std::fs::remove_dir_all(fake_dir).expect("remove fake include directory");
    }
}

#[cfg(test)]
mod embed_unit_tests {
    use super::{parse_embed_int, read_embed_bounded};

    #[test]
    fn embed_int_radices_and_suffixes() {
        assert_eq!(parse_embed_int("0"), Some(0));
        assert_eq!(parse_embed_int("255"), Some(255));
        assert_eq!(parse_embed_int("0x10"), Some(16));
        assert_eq!(parse_embed_int("0XFF"), Some(255));
        assert_eq!(parse_embed_int("0b101"), Some(5));
        assert_eq!(parse_embed_int("010"), Some(8));
        assert_eq!(parse_embed_int("12u"), Some(12));
        assert_eq!(parse_embed_int("12UL"), Some(12));
        assert_eq!(parse_embed_int(" 12 \t"), Some(12));
        assert_eq!(parse_embed_int("u64"), None); // not a pp-number
    }

    #[test]
    fn embed_int_rejects_malformed() {
        assert_eq!(parse_embed_int(""), None);
        assert_eq!(parse_embed_int("1 + 2"), None); // not a single pp-number
        assert_eq!(parse_embed_int("0x"), None);
        assert_eq!(parse_embed_int("-1"), None); // limit is non-negative
        assert_eq!(parse_embed_int("09"), None); // invalid octal digit
        assert_eq!(parse_embed_int("12abc"), None);
        assert_eq!(parse_embed_int("18446744073709551616"), None); // u64 overflow
        assert_eq!(parse_embed_int("18446744073709551615"), Some(u64::MAX));
    }

    #[test]
    fn embed_int_char_literals() {
        assert_eq!(parse_embed_int("'A'"), Some(65));
        assert_eq!(parse_embed_int("'*'"), Some(42));
        assert_eq!(parse_embed_int("'\\n'"), Some(10));
        assert_eq!(parse_embed_int("'\\''"), Some(0x27));
        assert_eq!(parse_embed_int("'\\\\'"), Some(0x5c));
        assert_eq!(parse_embed_int("'\\x41'"), Some(65));
        assert_eq!(parse_embed_int("'\\101'"), Some(65));
        assert_eq!(parse_embed_int("'\\0'"), Some(0));
        assert_eq!(parse_embed_int("''"), None);
        assert_eq!(parse_embed_int("'ab'"), None); // multichar not a pp-number
        assert_eq!(parse_embed_int("'"), None);
    }

    #[test]
    fn embed_bounded_read_slices() {
        let dir = std::env::temp_dir().join(format!("lccc_embed_ut_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("resource.bin");
        let data: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
        std::fs::write(&path, &data).unwrap();

        // Unbounded: whole file.
        assert_eq!(read_embed_bounded(&path, 0, None).unwrap().len(), 10_000);
        // Prefix slice: limit bounds the read.
        assert_eq!(read_embed_bounded(&path, 0, Some(4)).unwrap(), data[..4]);
        // Offset + limit: reads exactly the offset+limit prefix; the
        // offset slice itself is owned by expand_embed_bytes.
        assert_eq!(
            read_embed_bounded(&path, 250, Some(16)).unwrap(),
            data[..266]
        );
        // Offset+limit beyond EOF truncates to what exists.
        assert_eq!(read_embed_bounded(&path, 9_998, Some(16)).unwrap(), data);
        // Missing file is an error.
        assert!(read_embed_bounded(&dir.join("nope.bin"), 0, Some(1)).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
