//! Shared utility functions for the preprocessor module.
//!
//! Provides both char-oriented and byte-oriented helpers for scanning C source text.
//! The byte-oriented variants (`*_byte`, `*_bytes`) are preferred in hot paths since
//! they avoid the overhead of `Vec<char>` allocation. The char-oriented versions are
//! retained for code that still operates on `&[char]`.

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
}

/// Select (or clear) the C23 preprocessing dialect for this thread.
pub(super) fn set_c23_dialect(on: bool) {
    C23_DIALECT.with(|c| c.set(on));
}

/// Whether this thread preprocesses C23.
pub(super) fn c23_dialect() -> bool {
    C23_DIALECT.with(std::cell::Cell::get)
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

#[cfg(test)]
mod digit_separator_tests {
    use super::*;

    fn with_c23<R>(on: bool, f: impl FnOnce() -> R) -> R {
        set_c23_dialect(on);
        let r = f();
        set_c23_dialect(false);
        r
    }

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
