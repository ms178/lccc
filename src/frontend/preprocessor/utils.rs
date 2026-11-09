//! Shared utility functions for the preprocessor module.
//!
//! Provides both char-oriented and byte-oriented helpers for scanning C source text.
//! The byte-oriented variants (`*_byte`, `*_bytes`) are preferred in hot paths since
//! they avoid the overhead of `Vec<char>` allocation. The char-oriented versions are
//! retained for code that still operates on `&[char]`.

/// Length of a multi-byte `<` punctuator. Text-level macro scanners must
/// consume it as a token: the `%:` bytes inside `<%:` belong to `<%` and `:`,
/// not to a stringize operator. `<<` likewise wins over a following `<%`.
#[inline]
pub(super) fn less_punctuator_len(bytes: &[u8]) -> usize {
    match bytes {
        [b'<', b'<', b'=', ..] => 3,
        [b'<', b'<' | b'=' | b':' | b'%', ..] => 2,
        _ => 0,
    }
}

/// Length of a `##` preprocessing token, including its C digraph spelling.
#[inline]
pub(super) fn paste_token_len(bytes: &[u8]) -> usize {
    match bytes {
        [b'#', b'#', ..] => 2,
        [b'%', b':', b'%', b':', ..] => 4,
        _ => 0,
    }
}

/// Length of a single `#` preprocessing token. Maximal munch makes `##` and
/// `%:%:` a different token, so neither may introduce a directive/stringize.
#[inline]
pub(super) fn hash_token_len(bytes: &[u8]) -> usize {
    match bytes {
        [b'#', b'#', ..] | [b'%', b':', b'%', b':', ..] => 0,
        [b'#', ..] => 1,
        [b'%', b':', ..] => 2,
        _ => 0,
    }
}

/// Check if a character can start a C identifier.
/// GCC extension: '$' is allowed in identifiers (-fdollars-in-identifiers, on by default).
pub fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

/// Check if a character can continue a C identifier.
/// GCC extension: '$' is allowed in identifiers (-fdollars-in-identifiers, on by default).
pub fn is_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Check if a byte can start a C identifier (ASCII letter, underscore, or dollar sign).
/// GCC extension: '$' is allowed in identifiers (-fdollars-in-identifiers, on by default).
#[inline(always)]
pub fn is_ident_start_byte(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

/// Check if a byte can continue a C identifier (ASCII alphanumeric, underscore, or dollar sign).
/// GCC extension: '$' is allowed in identifiers (-fdollars-in-identifiers, on by default).
#[inline(always)]
pub fn is_ident_cont_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Extract a `&str` slice from `bytes[start..end]`.
///
/// C identifiers consist only of ASCII letters, digits, underscores, and dollar signs,
/// which are all valid single-byte UTF-8 characters.
#[inline(always)]
pub fn bytes_to_str(bytes: &[u8], start: usize, end: usize) -> &str {
    std::str::from_utf8(&bytes[start..end]).expect("bytes_to_str: input is not valid UTF-8")
}

/// Whether `ident`, met directly before a quote, is an encoding prefix and so
/// the first part of a character or string literal token (`L'x'`, `u'x'`,
/// `U'x'`, `u8'x'` and their string forms) rather than an identifier: such a
/// prefix is never macro-expanded and never replaced by 0 in `#if`.
pub(super) fn is_literal_prefix(ident: &str, next: Option<u8>) -> bool {
    matches!(next, Some(b'\'' | b'"')) && matches!(ident, "L" | "u" | "U" | "u8")
}

/// Append the character that starts at byte `i` of `text` to `out` and return
/// the index after it.
///
/// The byte scanners in this module walk `text.as_bytes()` but build `String`
/// output; copying a non-ASCII character there one `b as char` per byte
/// re-encodes every byte as its own Latin-1 character (`é`, C3 A9, becomes
/// `Ã©`, C3 83 C2 A9), which changes string literals and character constants.
/// `i` is a character boundary at every call site (the scanners only stop on
/// ASCII bytes); if it ever is not, the byte is skipped rather than mangled.
pub(super) fn push_char_at(out: &mut String, text: &str, i: usize) -> usize {
    match text.get(i..).and_then(|rest| rest.chars().next()) {
        Some(ch) => {
            out.push(ch);
            i + ch.len_utf8()
        }
        None => i + 1,
    }
}

/// Skip past a string or character literal in a byte slice, starting at position `i`.
/// Returns the position after the closing quote. Handles backslash escapes.
/// Byte-oriented version for code that processes `&[u8]` directly.
pub fn skip_literal_bytes(bytes: &[u8], start: usize, quote: u8) -> usize {
    let len = bytes.len();
    let mut i = start + 1; // skip opening quote
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            i += 2;
        } else if bytes[i] == quote {
            return i + 1;
        } else {
            i += 1;
        }
    }
    i
}

/// Copy a string or character literal from a byte slice into a byte buffer result.
/// Returns the position after the closing quote. Handles backslash escapes.
/// Copies raw bytes without any char conversion to preserve UTF-8 sequences.
pub fn copy_literal_bytes_raw(
    bytes: &[u8],
    start: usize,
    quote: u8,
    result: &mut Vec<u8>,
) -> usize {
    let len = bytes.len();
    result.push(bytes[start]); // opening quote
    let mut i = start + 1;
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            result.push(bytes[i]);
            result.push(bytes[i + 1]);
            i += 2;
        } else if bytes[i] == quote {
            result.push(bytes[i]);
            return i + 1;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    i
}

/// Copy a string or character literal from a byte slice into a String result.
/// Returns the position after the closing quote. Handles backslash escapes.
/// Copies the entire literal as a single `&str` slice for efficiency, rather than
/// pushing individual bytes.
pub fn copy_literal_bytes_to_string(
    bytes: &[u8],
    start: usize,
    quote: u8,
    result: &mut String,
) -> usize {
    let len = bytes.len();
    // Find the end of the literal first, then copy as a single &str slice
    // (the common case), avoiding per-byte push.
    let mut i = start + 1; // skip opening quote
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            i += 2; // skip escape sequence
        } else if bytes[i] == quote {
            i += 1; // include closing quote
            // Source text is valid UTF-8 and we copy a contiguous substring.
            let slice =
                std::str::from_utf8(&bytes[start..i]).expect("literal copy produced non-UTF8");
            result.push_str(slice);
            return i;
        } else {
            i += 1;
        }
    }
    // Unterminated literal - copy what we have
    let slice = std::str::from_utf8(&bytes[start..i]).expect("literal copy produced non-UTF8");
    result.push_str(slice);
    i
}

// ── C23 pp-number lexing ────────────────────────────────────────────────────
//
// C23 6.4.8 adds `' digit` and `' nondigit` to the pp-number grammar, so a
// digit separator (`1'000'000`, `0xFF'FF`) belongs to the number instead of
// opening a character constant. Before C23 a `'` always starts one (GCC 16.2
// and Clang 23.1.3 agree). The dialect is a property of the whole
// translation unit; it lives in a thread-local so every scanner (most are
// free functions) consults one fact. `Preprocessor::new` resets it and
// `Preprocessor::set_c23` sets it; compilation and each unit test run on
// their own thread.

thread_local! {
    static C23_DIALECT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static GNU_DIALECT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Select (or clear) the C23 preprocessing dialect for this thread.
pub(super) fn set_c23_dialect(on: bool) {
    C23_DIALECT.with(|c| c.set(on));
}

/// Whether this thread preprocesses C23.
pub(super) fn c23_dialect() -> bool {
    C23_DIALECT.with(std::cell::Cell::get)
}

/// Select (or clear) GNU extensions for this thread's preprocessing dialect.
///
/// The preprocessor needs it for the directives that GCC enables as extensions
/// (`#elifdef`, `#elifndef`), which are errors in a strict pre-C23 dialect.  It
/// travels next to the revision because a directive decision must never be made
/// from one switch alone.
pub(super) fn set_gnu_dialect(on: bool) {
    GNU_DIALECT.with(|c| c.set(on));
}

/// Whether this thread preprocesses a GNU dialect.
pub(super) fn gnu_dialect() -> bool {
    GNU_DIALECT.with(std::cell::Cell::get)
}

/// End of the pp-number starting at `bytes[i]` (a digit, or `.` followed by
/// a digit): identifier characters, `.`, an exponent sign after `e E p P`,
/// and in C23 a `'` followed by an identifier character.
pub(super) fn ppnumber_end(bytes: &[u8], i: usize) -> usize {
    let c23 = c23_dialect();
    let len = bytes.len();
    let mut j = i;
    while j < len {
        let c = bytes[j];
        let continues = c.is_ascii_alphanumeric()
            || c == b'.'
            || c == b'_'
            || ((c == b'+' || c == b'-')
                && j > i
                && matches!(bytes[j - 1], b'e' | b'E' | b'p' | b'P'))
            || (c23
                && c == b'\''
                && j > i
                && bytes
                    .get(j + 1)
                    .is_some_and(|&n| n.is_ascii_alphanumeric() || n == b'_'));
        if !continues {
            break;
        }
        j += 1;
    }
    j
}

/// Whether the `'` at `bytes[i]` is a C23 digit separator: inside a
/// pp-number (the token it continues starts with a digit, or `.` digit) and
/// followed by an identifier character. For scanners that walk byte by
/// byte; `u8'a'`, `L'a'` and `x1'a'` stay character constants because their
/// token starts with a letter.
pub(super) fn is_digit_separator(bytes: &[u8], i: usize) -> bool {
    if bytes.get(i) != Some(&b'\'') || !c23_dialect() {
        return false;
    }
    if !bytes
        .get(i + 1)
        .is_some_and(|&n| n.is_ascii_alphanumeric() || n == b'_')
    {
        return false;
    }
    let in_token = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'\'';
    let starts_ppnumber = |k: usize| {
        bytes[k].is_ascii_digit()
            || (bytes[k] == b'.' && bytes.get(k + 1).is_some_and(u8::is_ascii_digit))
    };
    let run_start = |mut k: usize| {
        while k > 0 && in_token(bytes[k - 1]) {
            k -= 1;
        }
        k
    };
    let mut k = run_start(i);
    // An exponent sign joins two runs into one pp-number (`1e+1'0`) only when
    // the run before it is itself a pp-number (`abe+1'0` is three tokens).
    while k >= 2
        && matches!(bytes[k - 1], b'+' | b'-')
        && matches!(bytes[k - 2], b'e' | b'E' | b'p' | b'P')
    {
        let prev = run_start(k - 2);
        if !starts_ppnumber(prev) {
            break;
        }
        k = prev;
    }
    k < i && starts_ppnumber(k)
}

/// Skip ASCII whitespace and C comments, returning the offset of the next
/// preprocessing token (or `bytes.len()`).  Bounded and progress-making: an
/// unterminated comment consumes to EOL exactly like the resolver's recovery.
#[inline]
pub(super) fn skip_blanks_and_comments(bytes: &[u8], mut i: usize) -> usize {
    let len = bytes.len();
    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
        } else if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = if i + 1 < len { i + 2 } else { len };
        } else if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
        } else {
            break;
        }
    }
    i
}

/// Length of the single preprocessing token starting at `bytes[i]` (`i` must
/// already point at a token, not trivia).  Maximal munch for punctuators,
/// escape-aware literals, C23 pp-numbers, and prefixed string/char literals.
/// Returns 0 at EOL.  Non-ASCII unknown bytes consume one whole UTF-8 scalar
/// so callers never slice through a code point.
///
/// `scoped_colon` selects the `::` token: C23 and Clang's extension lex it as
/// one punctuator, while GCC's pre-C23 C lexer sees two `:` tokens.  The query
/// resolvers pass the persona/dialect fact explicitly instead of letting a
/// malformed operand's token boundary depend on the operator's spelling.
pub(super) fn pp_token_len(bytes: &[u8], i: usize, scoped_colon: bool) -> usize {
    let len = bytes.len();
    if i >= len {
        return 0;
    }
    let rest = &bytes[i..];
    let b = rest[0];
    // Prefixed literals are one token (`u8"x"`, `L'c'`), not identifier+literal.
    // Dispatch on the first byte so the ordinary identifier path does not run
    // four `starts_with` probes.
    let prefix_len = match b {
        b'u' if rest.starts_with(b"u8") => 2,
        b'L' | b'u' | b'U' => 1,
        _ => 0,
    };
    if prefix_len != 0 {
        let q = i + prefix_len;
        if q < len && (bytes[q] == b'"' || bytes[q] == b'\'') {
            return skip_literal_bytes(bytes, q, bytes[q]) - i;
        }
    }
    if is_ident_start_byte(b) {
        let mut j = i + 1;
        while j < len && is_ident_cont_byte(bytes[j]) {
            j += 1;
        }
        return j - i;
    }
    if b.is_ascii_digit() || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
        return ppnumber_end(bytes, i) - i;
    }
    if b == b'"' || (b == b'\'' && !is_digit_separator(bytes, i)) {
        return skip_literal_bytes(bytes, i, b) - i;
    }
    // First-byte dispatch replaces a 29-entry longest-prefix table.  The
    // shared digraph helpers keep `<%:`, `%:%:` and `##` maximal munch in one
    // place; every arm below is O(1) in the length of the remaining input.
    match b {
        b'<' => {
            let n = less_punctuator_len(rest);
            if n != 0 { n } else { 1 }
        }
        b'%' => {
            let n = paste_token_len(rest);
            if n != 0 {
                n
            } else if matches!(rest.get(1), Some(b':' | b'>' | b'=')) {
                2
            } else {
                1
            }
        }
        b'#' => {
            if paste_token_len(rest) != 0 {
                2
            } else {
                1
            }
        }
        b'.' if rest.starts_with(b"...") => 3,
        b':' => match rest.get(1) {
            Some(b':') if scoped_colon => 2,
            Some(b'>') => 2,
            _ => 1,
        },
        b'>' => match rest {
            [b'>', b'>', b'=', ..] => 3,
            [b'>', b'>' | b'=', ..] => 2,
            _ => 1,
        },
        b'-' if matches!(rest.get(1), Some(b'>' | b'-' | b'=')) => 2,
        b'+' if matches!(rest.get(1), Some(b'+' | b'=')) => 2,
        b'=' | b'!' if matches!(rest.get(1), Some(b'=')) => 2,
        b'&' if matches!(rest.get(1), Some(b'&' | b'=')) => 2,
        b'|' if matches!(rest.get(1), Some(b'|' | b'=')) => 2,
        b'*' | b'/' | b'^' if matches!(rest.get(1), Some(b'=')) => 2,
        _ if b < 0x80 => 1,
        _ => {
            // Validate at most four bytes, never the whole remaining line.
            // The previous `from_utf8(rest)` made a run of unknown non-ASCII
            // tokens quadratic; this keeps each token O(1) and still refuses
            // to slice through a valid scalar or accept an invalid sequence.
            let width = match b {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 1,
            }
            .min(rest.len());
            if std::str::from_utf8(&rest[..width]).is_ok() {
                width
            } else {
                1
            }
        }
    }
}

#[cfg(test)]
mod digraph_token_tests {
    use super::*;

    #[test]
    fn alternate_hash_tokens_use_maximal_munch() {
        for (input, hash, paste) in [
            (&b"#x"[..], 1, 0),
            (b"%:x", 2, 0),
            (b"##x", 0, 2),
            (b"%:%:x", 0, 4),
            (b"%>x", 0, 0),
            (b"", 0, 0),
        ] {
            assert_eq!(hash_token_len(input), hash, "{input:?}");
            assert_eq!(paste_token_len(input), paste, "{input:?}");
        }
        assert_eq!(less_punctuator_len(b"<%:x"), 2);
        assert_eq!(less_punctuator_len(b"<<%:x"), 2);
        assert_eq!(less_punctuator_len(b"<<=%:x"), 3);
        assert_eq!(less_punctuator_len(b"<x"), 0);
    }
}

/// Test-only dialect switch with panic-safe restore.  Rust's default harness
/// gives each test a thread, but a custom harness may run several on one
/// thread; leaking C23 state would make tokenization tests order-dependent.
#[cfg(test)]
pub(super) fn with_c23<R>(on: bool, f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            set_c23_dialect(self.0);
        }
    }
    let _guard = Restore(c23_dialect());
    set_c23_dialect(on);
    f()
}

#[cfg(test)]
mod pp_token_tests {
    use super::*;

    #[test]
    fn trivia_skip_is_bounded_and_progress_making() {
        assert_eq!(skip_blanks_and_comments(b"  /* ) */ x", 0), 10);
        assert_eq!(skip_blanks_and_comments(b"// line\nx", 0), 8);
        assert_eq!(skip_blanks_and_comments(b"/* unterminated", 0), 15);
        assert_eq!(skip_blanks_and_comments(b"x", 0), 0);
        assert_eq!(skip_blanks_and_comments(b"", 0), 0);

        // Every suffix offset is bounded, idempotent, and either lands on a
        // non-trivia byte or at EOL.  This pins the manual index arithmetic
        // without relying on one hand-picked comment shape.
        let s = b" \t/* ) \" */ x // c\n y /* open";
        for i in 0..=s.len() {
            let j = skip_blanks_and_comments(s, i);
            assert!((i..=s.len()).contains(&j), "offset {i} -> {j}");
            assert_eq!(skip_blanks_and_comments(s, j), j);
            if j < s.len() {
                assert!(!s[j].is_ascii_whitespace());
                assert!(!(s[j] == b'/' && matches!(s.get(j + 1), Some(b'*' | b'/'))));
            }
        }
    }

    #[test]
    fn token_length_uses_maximal_munch_and_whole_literals() {
        for (input, want) in [
            (&b"abc_123"[..], 7),
            (b"$x", 2),
            (b"123abc", 6),
            (b".5f", 3),
            (b"\"a\\\"b\"x", 6),
            (b"'\\''x", 4),
            (b"u8\"x\"y", 5),
            (b"L'c'z", 4),
            (b"u'c'z", 4),
            (b"U'c'z", 4),
            (b"->x", 2),
            (b"...x", 3),
            (b"<<=x", 3),
            (b">>=x", 3),
            (b"%:%:x", 4),
            (b"##x", 2),
            (b"<%:x", 2),
            (b":>x", 2),
            (b"+x", 1),
            (b"", 0),
        ] {
            assert_eq!(pp_token_len(input, 0, true), want, "{input:?}");
        }
        // A non-ASCII unknown token consumes one scalar, never half a code point.
        assert_eq!(pp_token_len("éx".as_bytes(), 0, true), 2);
        assert_eq!(pp_token_len("€x".as_bytes(), 0, true), 3);
        assert_eq!(pp_token_len("𐀀x".as_bytes(), 0, true), 4);
    }

    #[test]
    fn punctuator_dispatch_covers_every_c_token_and_near_miss() {
        // Expected lengths are independent of the implementation's dispatch
        // table: all C punctuators, digraphs, and the prefixes most likely to
        // be mis-munched.  `::` is tested under both lexer contracts.
        for (input, scoped, want) in [
            (&b"..."[..], true, 3),
            (b"..", true, 1),
            (b"<<=", true, 3),
            (b">>=", true, 3),
            (b"->", true, 2),
            (b"++", true, 2),
            (b"--", true, 2),
            (b"<<", true, 2),
            (b">>", true, 2),
            (b"<=", true, 2),
            (b">=", true, 2),
            (b"==", true, 2),
            (b"!=", true, 2),
            (b"&&", true, 2),
            (b"||", true, 2),
            (b"+=", true, 2),
            (b"-=", true, 2),
            (b"*=", true, 2),
            (b"/=", true, 2),
            (b"%=", true, 2),
            (b"&=", true, 2),
            (b"^=", true, 2),
            (b"|=", true, 2),
            (b"##", true, 2),
            (b"%:%:", true, 4),
            (b"%:", true, 2),
            (b"%>", true, 2),
            (b"<:", true, 2),
            (b":>", true, 2),
            (b"<%", true, 2),
            (b"::", true, 2),
            (b"::", false, 1),
            (b":::", true, 2),
            (b":::", false, 1),
            (b":", true, 1),
            (b";", true, 1),
            (b"=", true, 1),
            (b"!", true, 1),
            (b"&", true, 1),
            (b"|", true, 1),
            (b"^", true, 1),
            (b"?", true, 1),
            (b"@", true, 1),
            (b"\\", true, 1),
        ] {
            assert_eq!(
                pp_token_len(input, 0, scoped),
                want,
                "{input:?} scoped={scoped}"
            );
        }
    }

    #[test]
    fn c23_digit_separators_stay_inside_the_pp_number() {
        with_c23(true, || {
            assert_eq!(pp_token_len(b"1'000+2", 0, true), 5);
            assert_eq!(pp_token_len(b"0x1p-3'0f)", 0, true), 9);
        });
        with_c23(false, || {
            assert_eq!(pp_token_len(b"1'000+2", 0, true), 1);
        });
    }

    #[test]
    fn token_lengths_are_positive_bounded_and_respect_utf8_boundaries() {
        let s = "a$9 .5 u8\"é\" 'x' :: >>= %:%: € 𐀀 \\";
        let bytes = s.as_bytes();
        for i in 0..bytes.len() {
            if !s.is_char_boundary(i) {
                continue;
            }
            let j = skip_blanks_and_comments(bytes, i);
            if j == bytes.len() {
                assert_eq!(pp_token_len(bytes, j, true), 0);
                continue;
            }
            let n = pp_token_len(bytes, j, true);
            assert!(n > 0 && j + n <= bytes.len(), "offset {j} length {n}");
            assert!(
                s.is_char_boundary(j + n),
                "offset {j} length {n} split a scalar"
            );
        }
    }
}

#[cfg(test)]
mod digit_separator_tests {
    use super::*;

    #[test]
    fn separators_are_recognised_only_inside_c23_pp_numbers() {
        with_c23(true, || {
            let s = b"x = 1'000 + 0xFF'FF + 1e+1'0 + .5'5;";
            for (i, &b) in s.iter().enumerate() {
                if b == b'\'' {
                    assert!(is_digit_separator(s, i), "offset {i}");
                }
            }
            for lit in [
                &b"c = u8'a';"[..],
                b"c = L'a';",
                b"c = x1'a';",
                b"c = abe+1 ? 'a' : 0;",
            ] {
                let q = lit.iter().position(|&b| b == b'\'').unwrap();
                assert!(
                    !is_digit_separator(lit, q),
                    "{}",
                    String::from_utf8_lossy(lit)
                );
            }
            assert!(
                !is_digit_separator(b"1' ", 1),
                "a separator needs a following digit or nondigit"
            );
            assert_eq!(ppnumber_end(b"1'000'000;", 0), 9);
            assert_eq!(ppnumber_end(b"0x1p-3'0f)", 0), 9);
        });
        with_c23(false, || {
            assert!(!is_digit_separator(b"1'000", 1));
            assert_eq!(ppnumber_end(b"1'000", 0), 1);
        });
    }
}
