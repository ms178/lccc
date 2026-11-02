use super::token::{Token, TokenKind, UnicodeCharKind};
use crate::common::encoding::decode_pua_byte;
use crate::common::error::{Diagnostic, Severity, WarningKind};
use crate::common::source::Span;

/// C lexer that tokenizes source input with source locations.
pub struct Lexer {
    input: Vec<u8>,
    pos: usize,
    file_id: u32,
    gnu_extensions: bool,
    /// C23 dialect: C23 keywords and digit separators (`1'000`).
    c23: bool,
    at_line_start: bool,
    ident_pool: std::collections::HashMap<Vec<u8>, String>,
    /// Every diagnostic the lexer produced, errors and warnings alike, in the
    /// order the scanner met them.
    ///
    /// The lexer cannot report anything itself: warning selection
    /// (`-Wmultichar`, `-Wno-…`, `-w`, `-Werror[=…]`), the system-header filter
    /// and the per-location dedupe all live in
    /// [`crate::common::error::DiagnosticEngine`], which needs the source
    /// manager this phase is still filling in. It therefore records
    /// `Diagnostic`s here and the driver hands them to the engine, in this
    /// order, once spans resolve.
    ///
    /// One ordered stream rather than errors-plus-warnings: GCC interleaves the
    /// two by *scan position*, not by severity — in `u8'\xc3\xa9a'` the
    /// escape-range warning precedes the encoding-prefix error of the same
    /// literal, while `'\U00110000'` warns about the escape before it warns
    /// about the character count — and sorting by source span gets both of
    /// those backwards, because a literal's span starts before its escapes do.
    pub diagnostics: Vec<Diagnostic>,
}

/// The form an escape sequence was written in.
///
/// Two readings depend on it. A *character* escape (`\u00e9`, `\N{SNOWMAN}`)
/// names a code point: a narrow literal encodes it in the execution character
/// set (UTF-8), and a `u` literal has to fit it into a single UTF-16 code unit.
/// Every other escape names a numeric value that each literal truncates to its
/// own width. Choosing between the two by magnitude ("above 0xFF means UTF-8")
/// is wrong for exactly U+0080..U+00FF: `"\u00e9"` is two bytes there, while
/// `"\xE9"` is one (measured against gcc-16.2 on x86-64-linux-gnu: C3 A9 and
/// E9).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EscapeKind {
    /// `\n`, `\t`, `\'`, …
    Simple,
    /// `\xNN` and the C2y delimited `\x{…}`.
    Hex,
    /// `\NNN` and the C2y delimited `\o{…}`.
    Octal,
    /// `\uNNNN`, `\UNNNNNNNN` and the delimited `\u{…}`.
    Universal,
    /// C++23's `\N{NAME}`, which GCC 16.2 and lccc also accept in C.
    Named,
}

/// One escape sequence, carrying the FULL numeric value.
///
/// GCC decides what fits from the literal's width rather than from the escape
/// that was written: `u'\x100'` is 0x100 while `'\x100'` is 0 with `hex escape
/// sequence out of range`, and `'\777'` is -1 while `u'\777'` is 0x1FF
/// (measured on gcc-16.2). A byte-clamped `char` cannot carry that difference,
/// so the value stays numeric here and `lex_char` / `lex_wide_char` apply the
/// width of the literal they are building.
#[derive(Clone, Copy)]
struct RawEscape {
    value: u32,
    kind: EscapeKind,
    /// Source span of the whole escape, from the backslash to its last digit.
    span: Span,
    /// A character escape named a code point above U+10FFFF. GCC 16.2 warns
    /// (`\U00110000 is outside the UCS codespace`) and keeps the value; only a
    /// character constant can, because a string literal's payload is a Rust
    /// `String` (documented divergence, backlog GP-29).
    out_of_ucs: bool,
    /// A hex or octal escape wrote more digits than 32 bits can hold
    /// (`\x100000000`), so its value already wrapped.
    blew: bool,
    /// How many characters GCC counts this escape for when it *rejects* it,
    /// as reported by `multi-character literal with N characters exceeds 'int'
    /// size of 4 bytes`. `None` on the paths where the count follows from the
    /// value (one character, or the length of its UTF-8 encoding in a narrow
    /// literal); `Some(0)` for the escapes GCC's error recovery consumes
    /// without producing anything (`'\xg'` is a ONE-character constant, so it
    /// warns about neither a multi-character constant nor the 'int' size), and
    /// `Some(n)` for the two recovery shapes that count as `n` characters
    /// (`'\N{NO SUCH NAME}'` counts as two). Measured on gcc-16.2, because
    /// these counts are what the diagnostics say.
    contributed: Option<u8>,
}

impl RawEscape {
    /// Whether the escape named a character rather than a numeric byte value.
    fn is_character(self) -> bool {
        matches!(self.kind, EscapeKind::Universal | EscapeKind::Named)
    }

    /// GCC's wording for a value that does not fit the literal: a hex or octal
    /// escape states its own form, and characters are judged by the caller.
    fn range_message(self) -> Option<&'static str> {
        match self.kind {
            EscapeKind::Hex => Some("hex escape sequence out of range"),
            EscapeKind::Octal => Some("octal escape sequence out of range"),
            _ => None,
        }
    }
}

/// Encode `value` as generalized UTF-8 into `buf`, returning the used prefix.
///
/// Values up to U+10FFFF are plain UTF-8. Above that GCC 16.2 still encodes the
/// value, with the traditional 5- and 6-byte forms, so a narrow character
/// constant keeps the number it was written with: `'\U00110000'` is
/// F4 90 80 80 and `'\U7FFFFFFF'` is FD BF BF BF BF BF (measured). Values
/// above 0x7FFFFFFF never reach here (they are `… is not a valid universal
/// character`), and surrogates are errors in every literal.
fn generalized_utf8(value: u32, buf: &mut [u8; 6]) -> &[u8] {
    if let Some(ch) = char::from_u32(value) {
        let len = ch.len_utf8();
        ch.encode_utf8(buf);
        return &buf[..len];
    }
    // The traditional UTF-8 lengths, which GCC still uses above the UCS
    // codespace: 4 bytes up to U+1FFFFF, 5 up to U+3FFFFFF, 6 to 0x7FFFFFFF.
    let len = if value < 0x20_0000 {
        4
    } else if value < 0x400_0000 {
        5
    } else if value < 0x8000_0000 {
        6
    } else {
        // Rejected before this point; emit nothing rather than a wrong byte.
        return &buf[..0];
    };
    let lead = match len {
        4 => 0xF0u8,
        5 => 0xF8u8,
        _ => 0xFCu8,
    };
    buf[0] = lead | (value >> (6 * (len - 1))) as u8;
    for index in 1..len {
        let shift = 6 * (len - 1 - index);
        buf[index] = 0x80 | ((value >> shift) & 0x3F) as u8;
    }
    &buf[..len]
}

impl Lexer {
    pub fn new(input: &str, file_id: u32) -> Self {
        Self {
            input: input.bytes().collect(),
            pos: 0,
            file_id,
            gnu_extensions: true,
            c23: false,
            at_line_start: true,
            ident_pool: std::collections::HashMap::with_capacity(512),
            diagnostics: Vec::new(),
        }
    }

    pub fn set_gnu_extensions(&mut self, enabled: bool) {
        self.gnu_extensions = enabled;
    }

    /// Select the C23 dialect (keywords and digit separators).
    pub fn set_c23(&mut self, enabled: bool) {
        self.c23 = enabled;
    }

    fn intern_ident(&mut self, bytes: &[u8]) -> String {
        if let Some(existing) = self.ident_pool.get(bytes) {
            return existing.clone();
        }
        let s = std::str::from_utf8(bytes).unwrap_or("").to_string();
        self.ident_pool.insert(bytes.to_vec(), s.clone());
        s
    }

    fn emit_lex_diag(&mut self, msg: impl Into<String>, start: usize, end: usize) {
        self.diagnostics
            .push(Diagnostic::error(msg).with_span(Span::new(
                start as u32,
                end as u32,
                self.file_id,
            )));
    }

    /// Report an unconditional warning.
    ///
    /// GCC prints this class with no `[-W…]` label, and only `-w` or `-Werror`
    /// act on it (`character not encodable in a single code unit`,
    /// `multi-character literal cannot have an encoding prefix`,
    /// `… is outside the UCS codespace`, `hex escape sequence out of range`:
    /// all measured on gcc-16.2) — which is exactly how the diagnostic engine
    /// treats a warning with no category.
    fn warn(&mut self, msg: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::warning(msg).with_span(span));
    }

    /// Report a warning the user selects or silences by name
    /// (`multi-character character constant` is `-Wmultichar`, on by default).
    fn warn_kind(&mut self, msg: impl Into<String>, span: Span, kind: WarningKind) {
        self.diagnostics
            .push(Diagnostic::warning_with_kind(msg, kind).with_span(span));
    }

    /// `hex`/`octal escape sequence out of range` — GCC checks the numeric
    /// value against the width of one element of the literal being built, not
    /// against a byte (measured: `'\x100'`, `u'\x100'` and `U'\x100000000'`
    /// warn, `u'\x100'` and `L'\x100'` do not).
    ///
    /// `at` is the span GCC points the warning at: the whole character
    /// constant (`^~~~~~~` under `'\x100'`), but in a string the position one
    /// past the escape (`"\x1FF"` puts it under the closing quote).
    fn warn_escape_range(&mut self, escape: RawEscape, max_unit: u32, at: Span) {
        if let Some(message) = escape.range_message() {
            if escape.blew || escape.value > max_unit {
                self.warn(message, at);
            }
        }
    }

    /// Whether the diagnostics recorded since `from` include an error.
    ///
    /// GCC reports the empty character constant only for a literal that was
    /// empty on its own terms: `'\x'` and `'\N{unterminated'` consume every
    /// character they were given while reporting an error, and GCC does not
    /// pile an "empty character constant" on top (measured).
    fn saw_error_since(&self, from: usize) -> bool {
        self.diagnostics[from.min(self.diagnostics.len())..]
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    /// The location GCC reports a string-literal escape's range at: one byte
    /// past the escape as written, or its last byte at end of input.
    fn escape_tail_span(&self, escape: RawEscape) -> Span {
        let end = escape.span.end as usize;
        let last = self.input.len().saturating_sub(1);
        let start = end.min(last);
        Span::new(
            start as u32,
            (start + 1).min(self.input.len()) as u32,
            self.file_id,
        )
    }

    /// The escape sequence as written (`\u00e9`), for the diagnostics whose
    /// GCC wording quotes it.
    fn escape_text(&self, span: Span) -> String {
        String::from_utf8_lossy(&self.input[span.start as usize..span.end as usize]).into_owned()
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        // Estimate ~1 token per 5 bytes of source (typical for C code).
        let mut tokens = Vec::with_capacity(self.input.len() / 4 + 64);
        loop {
            let tok = self.next_token();
            let is_eof = tok.is_eof();
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        tokens
    }

    fn next_token(&mut self) -> Token {
        loop {
            self.skip_whitespace_and_comments();

            if self.pos >= self.input.len() {
                return Token::new(
                    TokenKind::Eof,
                    Span::new(self.pos as u32, self.pos as u32, self.file_id),
                );
            }

            let start = self.pos;
            let ch = self.input[self.pos];

            // Number literals
            if ch.is_ascii_digit()
                || (ch == b'.' && self.peek_next().is_some_and(|c| c.is_ascii_digit()))
            {
                return self.lex_number(start);
            }

            // String literals
            if ch == b'"' {
                return self.lex_string(start);
            }

            // Character literals
            if ch == b'\'' {
                return self.lex_char(start);
            }

            // Identifiers and keywords
            // GCC extension: '$' is allowed in identifiers (-fdollars-in-identifiers, on by default)
            //
            // Extended identifiers (C99 6.4.2.1, GCC 16.2 accepts all three
            // spellings in every dialect):
            //   `\uXXXX` / `\UXXXXXXXX` universal character names, and raw
            //   UTF-8 encoded identifier characters. Both start here because
            //   the leading byte is not ASCII-alphabetic.
            if ch == b'_' || ch == b'$' || ch.is_ascii_alphabetic() {
                // `lex_identifier` extends itself into a UCN/UTF-8
                // continuation, so the common case scans the name exactly once.
                return self.lex_identifier(start);
            }
            if ch >= 0x80 || ch == b'\\' {
                if let Some(tok) = self.try_lex_extended_identifier(start) {
                    return tok;
                }
                if ch == b'\\' {
                    // Not an identifier: a backslash outside a literal is the
                    // stray '\' GCC reports.
                    self.pos += 1;
                    self.emit_lex_diag("stray '\\' in program", start, self.pos);
                    continue;
                }
            }

            // Punctuation and operators
            if let Some(token) = self.lex_punctuation(start) {
                return token;
            }
            // The offending scalar was consumed; recover without recursion.
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Skip whitespace
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
                if self.input[self.pos] == b'\n' {
                    self.at_line_start = true;
                }
                self.pos += 1;
            }

            if self.pos >= self.input.len() {
                return;
            }

            // Skip GCC-style line markers: # <number> "filename"
            // These are emitted by the preprocessor and must not be lexed as tokens.
            // A line marker is a '#' at the start of a line (after optional whitespace)
            // followed by a digit.
            if self.at_line_start && self.input[self.pos] == b'#' && self.is_line_marker() {
                // Skip the entire line
                while self.pos < self.input.len() && self.input[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            self.at_line_start = false;

            // Skip line comments
            if self.pos + 1 < self.input.len()
                && self.input[self.pos] == b'/'
                && self.input[self.pos + 1] == b'/'
            {
                while self.pos < self.input.len() && self.input[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }

            // Skip block comments
            if self.pos + 1 < self.input.len()
                && self.input[self.pos] == b'/'
                && self.input[self.pos + 1] == b'*'
            {
                self.pos += 2;
                while self.pos + 1 < self.input.len() {
                    if self.input[self.pos] == b'*' && self.input[self.pos + 1] == b'/' {
                        self.pos += 2;
                        break;
                    }
                    self.pos += 1;
                }
                continue;
            }

            break;
        }
    }

    /// Check if the current position is at a GCC-style line marker.
    /// A line marker is `# <digit>` at the start of a line (i.e., the '#' is
    /// either at position 0 or preceded by a newline).
    fn accumulate_digits(bytes: &[u8], base: u32) -> (u64, bool) {
        let mut val: u64 = 0;
        let mut overflow = false;
        let base64 = base as u64;
        for &b in bytes {
            let digit = match b {
                b'0'..=b'9' => (b - b'0') as u64,
                b'a'..=b'f' => (b - b'a' + 10) as u64,
                b'A'..=b'F' => (b - b'A' + 10) as u64,
                _ => continue,
            };
            if digit >= base64 {
                continue;
            }
            let (m, o1) = val.overflowing_mul(base64);
            let (a, o2) = m.overflowing_add(digit);
            if o1 || o2 {
                overflow = true;
                val = u64::MAX;
            } else {
                val = a;
            }
        }
        (val, overflow)
    }

    fn is_line_marker(&self) -> bool {
        // Must be at '#'
        if self.pos >= self.input.len() || self.input[self.pos] != b'#' {
            return false;
        }
        // '#' must be at the start of a line
        if self.pos > 0 && self.input[self.pos - 1] != b'\n' {
            return false;
        }
        // Next non-space char must be a digit
        let mut j = self.pos + 1;
        while j < self.input.len() && self.input[j] == b' ' {
            j += 1;
        }
        j < self.input.len() && self.input[j].is_ascii_digit()
    }

    fn peek_next(&self) -> Option<u8> {
        if self.pos + 1 < self.input.len() {
            Some(self.input[self.pos + 1])
        } else {
            None
        }
    }

    fn lex_number(&mut self, start: usize) -> Token {
        if self.c23 {
            if let Some(tok) = self.lex_number_with_separators(start) {
                return tok;
            }
        }
        if self.pos + 1 < self.input.len()
            && self.input[self.pos] == b'0'
            && (self.input[self.pos + 1] == b'x' || self.input[self.pos + 1] == b'X')
        {
            return self.lex_hex_number(start);
        }

        if self.pos + 1 < self.input.len()
            && self.input[self.pos] == b'0'
            && (self.input[self.pos + 1] == b'b' || self.input[self.pos + 1] == b'B')
        {
            return self.lex_binary_number(start);
        }

        if self.pos + 1 < self.input.len()
            && self.input[self.pos] == b'0'
            && (self.input[self.pos + 1] == b'o' || self.input[self.pos + 1] == b'O')
        {
            return self.lex_prefixed_octal_number(start);
        }

        if let Some(tok) = self.lex_octal_number(start) {
            return tok;
        }

        self.lex_decimal_number(start)
    }

    /// C23 6.4.4.1/6.4.4.2: a `'` between two digits of a numeric constant is
    /// a digit separator with no effect on its value. When the pp-number at
    /// `self.pos` contains one, the constant is lexed from its separator-free
    /// spelling by the ordinary scanners and spans the original text. A `'`
    /// not between two digits (`1''0`, `0x'1`, `1'.5`) is diagnosed, as GCC
    /// does. Returns `None` when the number has no separator.
    fn lex_number_with_separators(&mut self, start: usize) -> Option<Token> {
        let input = &self.input;
        let mut end = self.pos;
        let mut has_separator = false;
        while end < input.len() {
            let c = input[end];
            let continues = c.is_ascii_alphanumeric()
                || c == b'.'
                || c == b'_'
                || ((c == b'+' || c == b'-')
                    && end > self.pos
                    && matches!(input[end - 1], b'e' | b'E' | b'p' | b'P'))
                || (c == b'\''
                    && end > self.pos
                    && input
                        .get(end + 1)
                        .is_some_and(|&n| n.is_ascii_alphanumeric() || n == b'_'));
            if !continues {
                break;
            }
            has_separator |= c == b'\'';
            end += 1;
        }
        if !has_separator {
            return None;
        }
        let is_hex =
            input[self.pos] == b'0' && matches!(input.get(self.pos + 1), Some(b'x' | b'X'));
        let is_prefixed_nondec = input[self.pos] == b'0'
            && matches!(
                input.get(self.pos + 1),
                Some(b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
            );
        let is_digit = |c: u8| {
            if is_hex {
                c.is_ascii_hexdigit()
            } else {
                c.is_ascii_digit()
            }
        };
        let mut clean = Vec::with_capacity(end - self.pos);
        let mut misplaced = Vec::new();
        for k in self.pos..end {
            let c = input[k];
            if c == b'\'' {
                // `0x'1`, `0b'1`, `0o'1`: the `'` follows the prefix, not a digit.
                let prev_ok = is_digit(input[k - 1]) && !(is_prefixed_nondec && k == self.pos + 2);
                if !(prev_ok && is_digit(input[k + 1])) {
                    misplaced.push(k);
                }
            } else {
                clean.push(c);
            }
        }
        for k in misplaced {
            self.emit_lex_diag("digit separator outside digit sequence", k, k + 1);
        }
        let text = String::from_utf8_lossy(&clean).into_owned();
        let mut sub = Lexer::new(&text, self.file_id);
        sub.set_gnu_extensions(self.gnu_extensions);
        sub.set_c23(self.c23);
        let kind = sub.lex_number(0).kind;
        if sub.pos != text.len() {
            self.emit_lex_diag(
                format!(
                    "invalid suffix \"{}\" on numeric constant",
                    &text[sub.pos..]
                ),
                start,
                end,
            );
        }
        // A sub-lexer's diagnostics describe the same span; re-anchor them.
        let sub_diags = std::mem::take(&mut sub.diagnostics);
        let span = Span::new(start as u32, end as u32, self.file_id);
        for mut diag in sub_diags {
            diag.span = Some(span);
            self.diagnostics.push(diag);
        }
        self.pos = end;
        Some(Token::new(
            kind,
            Span::new(start as u32, end as u32, self.file_id),
        ))
    }

    /// Lex a hexadecimal integer or hex float literal (0x/0X prefix).
    fn lex_hex_number(&mut self, start: usize) -> Token {
        self.pos += 2;
        let hex_start = self.pos;
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_hexdigit() {
            self.pos += 1;
        }

        // Check for hex float: 0x<digits>.<digits>p<exp> or 0x<digits>p<exp>
        let has_dot = self.pos < self.input.len() && self.input[self.pos] == b'.';
        let after_dot_has_p = if has_dot {
            let mut look = self.pos + 1;
            while look < self.input.len() && self.input[look].is_ascii_hexdigit() {
                look += 1;
            }
            look < self.input.len() && (self.input[look] == b'p' || self.input[look] == b'P')
        } else {
            false
        };
        let has_p = self.pos < self.input.len()
            && (self.input[self.pos] == b'p' || self.input[self.pos] == b'P');

        if has_dot && after_dot_has_p || has_p {
            return self.lex_hex_float(start, has_dot);
        }

        // Regular hex integer
        let (value, overflow) = Self::accumulate_digits(&self.input[hex_start..self.pos], 16);
        if overflow {
            self.emit_lex_diag(
                "integer literal is too large to be represented",
                hex_start,
                self.pos,
            );
        }
        self.finish_int_literal(value, true, start)
    }

    /// Lex a hex float literal: 0x<int_hex>.<frac_hex>p<+/->exp
    fn lex_hex_float(&mut self, start: usize, has_dot: bool) -> Token {
        if has_dot {
            self.pos += 1;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_hexdigit() {
                self.pos += 1;
            }
        }
        // Scan only; conversion uses integer guard/sticky rounding below.
        // Accumulating into u64 or scaling with powi loses long significands,
        // subnormal literals and huge exponents before the target rounding.
        if self.pos < self.input.len() && matches!(self.input[self.pos], b'p' | b'P') {
            self.pos += 1;
            if self.pos < self.input.len() && matches!(self.input[self.pos], b'+' | b'-') {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            if self.pos == exp_start {
                self.emit_lex_diag(
                    "hexadecimal floating constant requires an exponent",
                    start,
                    self.pos,
                );
            }
        }
        let number_end = self.pos;
        let (float_kind, _is_imaginary) = self.parse_float_suffix();
        let hex_text = std::str::from_utf8(&self.input[start..number_end]).unwrap_or("0x0p0");
        let value = crate::common::hex_float::parse_f64(hex_text, float_kind == 1);
        let span = Span::new(start as u32, self.pos as u32, self.file_id);
        match float_kind {
            1 => Token::new(TokenKind::FloatLiteralF32(value), span),
            2 => {
                let hex_text = std::str::from_utf8(&self.input[start..self.pos]).unwrap_or("0x0p0");
                let f128_bytes = crate::common::hex_float::parse_long_double(
                    hex_text,
                    crate::common::types::target_long_double_is_f128(),
                );
                let value = crate::common::long_double::f128_bytes_to_f64(&f128_bytes);
                Token::new(TokenKind::FloatLiteralLongDouble(value, f128_bytes), span)
            }
            3 => {
                let hex_text = std::str::from_utf8(&self.input[start..self.pos]).unwrap_or("0x0p0");
                let f128_bytes =
                    crate::common::long_double::parse_long_double_to_f128_bytes(hex_text);
                let value = crate::common::long_double::f128_bytes_to_f64(&f128_bytes);
                Token::new(TokenKind::FloatLiteralF128(value, f128_bytes), span)
            }
            7 => Token::new(
                TokenKind::FloatLiteralHalf(
                    false,
                    crate::common::half_float::f64_to_f16_bits(value),
                ),
                span,
            ),
            8 => Token::new(
                TokenKind::FloatLiteralHalf(
                    true,
                    crate::common::half_float::f64_to_bf16_bits(value),
                ),
                span,
            ),
            _ => Token::new(TokenKind::FloatLiteral(value), span),
        }
    }

    /// Parse a simple float suffix (f/F → 1, l/L → 2, else 0). No imaginary handling.
    fn parse_simple_float_suffix(&mut self) -> u8 {
        if self.pos < self.input.len()
            && (self.input[self.pos] == b'f' || self.input[self.pos] == b'F')
        {
            self.pos += 1;
            1
        } else if self.pos < self.input.len()
            && (self.input[self.pos] == b'l' || self.input[self.pos] == b'L')
        {
            self.pos += 1;
            2
        } else {
            0
        }
    }

    /// Lex a binary integer literal (0b/0B prefix).
    fn lex_binary_number(&mut self, start: usize) -> Token {
        self.pos += 2;
        let bin_start = self.pos;
        while self.pos < self.input.len()
            && (self.input[self.pos] == b'0' || self.input[self.pos] == b'1')
        {
            self.pos += 1;
        }
        let (value, overflow) = Self::accumulate_digits(&self.input[bin_start..self.pos], 2);
        if overflow {
            self.emit_lex_diag(
                "integer literal is too large to be represented",
                bin_start,
                self.pos,
            );
        }
        self.finish_int_literal(value, true, start)
    }

    /// Lex a C2y/GCC 15+ prefixed octal integer literal (`0o` / `0O`).
    fn lex_prefixed_octal_number(&mut self, start: usize) -> Token {
        self.pos += 2;
        let oct_start = self.pos;
        while self.pos < self.input.len()
            && self.input[self.pos] >= b'0'
            && self.input[self.pos] <= b'7'
        {
            self.pos += 1;
        }
        if self.pos == oct_start {
            self.emit_lex_diag(
                "invalid octal constant: expected octal digit after '0o'",
                start,
                self.pos,
            );
        } else if self.pos < self.input.len() && matches!(self.input[self.pos], b'8' | b'9') {
            let bad_start = self.pos;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            self.emit_lex_diag("invalid digit in octal constant", bad_start, self.pos);
        }
        let (value, overflow) = Self::accumulate_digits(&self.input[oct_start..self.pos], 8);
        if overflow {
            self.emit_lex_diag(
                "integer literal is too large to be represented",
                oct_start,
                self.pos,
            );
        }
        self.finish_int_literal(value, true, start)
    }

    /// Try to lex an octal literal. Returns None if the token turns out to be decimal/float.
    fn lex_octal_number(&mut self, start: usize) -> Option<Token> {
        if self.input[self.pos] != b'0' || !self.peek_next().is_some_and(|c| c.is_ascii_digit()) {
            return None;
        }
        let saved_pos = self.pos;
        self.pos += 1;
        let oct_start = self.pos;
        while self.pos < self.input.len()
            && self.input[self.pos] >= b'0'
            && self.input[self.pos] <= b'7'
        {
            self.pos += 1;
        }
        // Float indicator or non-octal digit → backtrack to decimal.
        // But '.' followed by '..' is ellipsis, not a decimal point — keep the octal.
        if self.pos < self.input.len()
            && matches!(self.input[self.pos], b'.' | b'e' | b'E' | b'8' | b'9')
        {
            let is_ellipsis = self.input[self.pos] == b'.'
                && self.pos + 2 < self.input.len()
                && self.input[self.pos + 1] == b'.'
                && self.input[self.pos + 2] == b'.';
            if !is_ellipsis {
                self.pos = saved_pos;
                return None;
            }
        }
        let (value, overflow) = Self::accumulate_digits(&self.input[oct_start..self.pos], 8);
        if overflow {
            self.emit_lex_diag(
                "integer literal is too large to be represented",
                oct_start,
                self.pos,
            );
        }
        Some(self.finish_int_literal(value, true, start))
    }

    /// Common integer literal finish: parse suffix, return token.
    fn finish_int_literal(&mut self, value: u64, is_hex_or_octal: bool, start: usize) -> Token {
        let (is_unsigned, is_long, is_long_long, is_imaginary) = self.parse_int_suffix();
        if is_imaginary {
            let span = Span::new(start as u32, self.pos as u32, self.file_id);
            return Token::new(TokenKind::ImaginaryLiteral(value as f64), span);
        }
        self.make_int_token(
            value,
            is_unsigned,
            is_long,
            is_long_long,
            is_hex_or_octal,
            start,
        )
    }

    /// Lex a decimal integer or float literal.
    fn lex_decimal_number(&mut self, start: usize) -> Token {
        let mut is_float = false;
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }

        // Check for decimal point, but NOT if it's the start of '...' (ellipsis).
        // E.g. `2...15` from GCC case range `case 2 ... 15:` must lex as `2` `...` `15`,
        // not as float `2.` followed by invalid `..15`.
        if self.pos < self.input.len()
            && self.input[self.pos] == b'.'
            && !(self.pos + 2 < self.input.len()
                && self.input[self.pos + 1] == b'.'
                && self.input[self.pos + 2] == b'.')
        {
            is_float = true;
            self.pos += 1;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }

        if self.pos < self.input.len()
            && (self.input[self.pos] == b'e' || self.input[self.pos] == b'E')
        {
            is_float = true;
            self.pos += 1;
            if self.pos < self.input.len()
                && (self.input[self.pos] == b'+' || self.input[self.pos] == b'-')
            {
                self.pos += 1;
            }
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }

        // Save the end position of the number digits before parsing suffixes
        // (which advance self.pos further).
        let num_end = self.pos;

        if is_float {
            let (float_kind, is_imaginary) = self.parse_float_suffix();
            // Borrow the digit text as &str without allocating a String.
            let text = std::str::from_utf8(&self.input[start..num_end]).unwrap_or("0");
            self.make_float_token(text, float_kind, is_imaginary, start)
        } else {
            // Parse the integer value directly from the &str borrow (no heap allocation).
            let (uvalue, overflow) = Self::accumulate_digits(&self.input[start..num_end], 10);
            if overflow {
                self.emit_lex_diag(
                    "integer literal is too large to be represented",
                    start,
                    num_end,
                );
            }
            self.finish_int_literal(uvalue, false, start)
        }
    }

    /// Parse float suffix with imaginary support (GCC extension).
    /// Returns (float_kind, is_imaginary) where float_kind:
    ///   0=double, 1=float, 2=long double, 3=_Float128 (binary128).
    /// Supports the GCC 7+ _FloatN suffixes: f16/f32/f64/f32x/f64x/f128/f128x
    /// (mapped to the x86-64 type of the same format) and q/Q for _Float128.
    fn parse_float_suffix(&mut self) -> (u8, bool) {
        let mut is_imaginary = false;
        let float_kind = if self.pos < self.input.len()
            && (self.input[self.pos] == b'f' || self.input[self.pos] == b'F')
        {
            self.pos += 1;
            let mut kind = 1u8;
            let start = self.pos;
            while self.pos < self.input.len()
                && (self.input[self.pos].is_ascii_digit()
                    || self.input[self.pos] == b'x'
                    || self.input[self.pos] == b'X')
            {
                self.pos += 1;
            }
            let suffix = &self.input[start..self.pos];
            if !suffix.is_empty() {
                let suf = String::from_utf8_lossy(suffix).to_ascii_lowercase();
                kind = match suf.as_str() {
                    "16" => 7,
                    "32" => 1,
                    "64" => 0,
                    "32x" => 0,
                    "64x" => 2,
                    "128" => 3,
                    "128x" => 3,
                    _ => 1,
                };
            }
            if self.pos < self.input.len()
                && (self.input[self.pos] == b'i' || self.input[self.pos] == b'I')
            {
                self.pos += 1;
                is_imaginary = true;
            }
            kind
        } else if self.pos < self.input.len()
            && (self.input[self.pos] == b'd' || self.input[self.pos] == b'D')
        {
            // C23 decimal floating suffixes: df/Df -> _Decimal32,
            // dd/DD -> _Decimal64, dl/DL -> _Decimal128 (case-insensitive).
            // A bare 'd'/'D' is NOT consumed here (not a valid suffix; the
            // caller treats the token as a double with a trailing identifier,
            // producing the same diagnostics GCC emits).
            if self.pos + 1 < self.input.len() {
                let second = self.input[self.pos + 1].to_ascii_lowercase();
                if second == b'f' || second == b'd' || second == b'l' {
                    self.pos += 2;
                    let kind = match second {
                        b'f' => 4u8,
                        b'd' => 5u8,
                        _ => 6u8,
                    };
                    return (kind, false);
                }
            }
            0
        } else if self.pos < self.input.len()
            && (self.input[self.pos] == b'q' || self.input[self.pos] == b'Q')
        {
            // __float128 / _Float128 GNU suffix
            self.pos += 1;
            if self.pos < self.input.len()
                && (self.input[self.pos] == b'i' || self.input[self.pos] == b'I')
            {
                self.pos += 1;
                is_imaginary = true;
            }
            3
        } else if self.pos < self.input.len()
            && (self.input[self.pos] == b'l' || self.input[self.pos] == b'L')
        {
            self.pos += 1;
            if self.pos < self.input.len()
                && (self.input[self.pos] == b'i' || self.input[self.pos] == b'I')
            {
                self.pos += 1;
                is_imaginary = true;
            }
            2
        } else if self.pos < self.input.len()
            && (self.input[self.pos] == b'i' || self.input[self.pos] == b'I')
        {
            self.pos += 1;
            is_imaginary = true;
            if self.pos < self.input.len()
                && (self.input[self.pos] == b'f' || self.input[self.pos] == b'F')
            {
                self.pos += 1;
                1
            } else if self.pos < self.input.len()
                && (self.input[self.pos] == b'l' || self.input[self.pos] == b'L')
            {
                self.pos += 1;
                2
            } else {
                0
            }
        } else if self.pos + 4 <= self.input.len()
            && self.input[self.pos..self.pos + 4].eq_ignore_ascii_case(b"bf16")
            && (self.pos + 4 == self.input.len()
                || (!self.input[self.pos + 4].is_ascii_alphanumeric()
                    && self.input[self.pos + 4] != b'_'))
        {
            self.pos += 4;
            return (8, false);
        } else {
            0
        };
        // Also consume trailing 'j'/'J' suffix (C99/GCC alternative for imaginary)
        if !is_imaginary
            && self.pos < self.input.len()
            && (self.input[self.pos] == b'j' || self.input[self.pos] == b'J')
        {
            self.pos += 1;
            is_imaginary = true;
        }
        (float_kind, is_imaginary)
    }

    /// Construct a float/imaginary token from parsed components.
    fn make_float_token(
        &self,
        text: &str,
        float_kind: u8,
        is_imaginary: bool,
        start: usize,
    ) -> Token {
        // A `float` literal rounds once, decimal -> binary32 (C11 6.4.4.2);
        // parsing as f64 and narrowing later double-rounds. Every f32 is an
        // exact f64, so the later narrowing is exact.
        let value: f64 = if float_kind == 1 {
            text.parse::<f32>().map_or(0.0, f64::from)
        } else {
            text.parse().unwrap_or(0.0)
        };
        let span = Span::new(start as u32, self.pos as u32, self.file_id);
        if is_imaginary {
            match float_kind {
                1 => Token::new(TokenKind::ImaginaryLiteralF32(value), span),
                2 => {
                    // The literal's value is its single rounding to the
                    // target long double format; the f64 shadow derives from
                    // that value (C: `(double)0.1L` rounds the long double).
                    let f128_bytes =
                        crate::common::long_double::parse_long_double_value_bytes(text);
                    let value = crate::common::long_double::f128_bytes_to_f64(&f128_bytes);
                    Token::new(
                        TokenKind::ImaginaryLiteralLongDouble(value, f128_bytes),
                        span,
                    )
                }
                _ => Token::new(TokenKind::ImaginaryLiteral(value), span),
            }
        } else {
            match float_kind {
                1 => Token::new(TokenKind::FloatLiteralF32(value), span),
                2 => {
                    let f128_bytes =
                        crate::common::long_double::parse_long_double_value_bytes(text);
                    let value = crate::common::long_double::f128_bytes_to_f64(&f128_bytes);
                    Token::new(TokenKind::FloatLiteralLongDouble(value, f128_bytes), span)
                }
                3 => {
                    let f128_bytes =
                        crate::common::long_double::parse_long_double_to_f128_bytes(text);
                    Token::new(TokenKind::FloatLiteralF128(value, f128_bytes), span)
                }
                4 | 5 | 6 => {
                    // C23 decimal floating literal: encode the exact decimal
                    // value into the BID bit pattern for the target width.
                    let width = match float_kind {
                        4 => 32u8,
                        5 => 64u8,
                        _ => 128u8,
                    };
                    let bits = crate::common::decimal::parse_decimal_literal(text)
                        .map(|lit| match width {
                            32 => {
                                let b = crate::common::decimal::encode_bid32(
                                    false,
                                    &lit.digits,
                                    lit.exponent,
                                )
                                .to_le_bytes();
                                let mut out = [0u8; 16];
                                out[..4].copy_from_slice(&b);
                                out
                            }
                            64 => {
                                let b = crate::common::decimal::encode_bid64(
                                    false,
                                    &lit.digits,
                                    lit.exponent,
                                )
                                .to_le_bytes();
                                let mut out = [0u8; 16];
                                out[..8].copy_from_slice(&b);
                                out
                            }
                            _ => {
                                let (hi, lo) = crate::common::decimal::encode_bid128(
                                    false,
                                    &lit.digits,
                                    lit.exponent,
                                );
                                let mut out = [0u8; 16];
                                out[..8].copy_from_slice(&lo.to_le_bytes());
                                out[8..].copy_from_slice(&hi.to_le_bytes());
                                out
                            }
                        })
                        .unwrap_or([0u8; 16]);
                    Token::new(TokenKind::FloatLiteralDecimal(width, bits), span)
                }
                7 => Token::new(
                    TokenKind::FloatLiteralHalf(
                        false,
                        crate::common::half_float::f64_to_f16_bits(value),
                    ),
                    span,
                ),
                8 => Token::new(
                    TokenKind::FloatLiteralHalf(
                        true,
                        crate::common::half_float::f64_to_bf16_bits(value),
                    ),
                    span,
                ),
                _ => Token::new(TokenKind::FloatLiteral(value), span),
            }
        }
    }

    /// Parse integer suffix and return (is_unsigned, is_long, is_imaginary).
    /// is_long is true for l/L or ll/LL suffixes.
    /// is_imaginary is true for trailing 'i' suffix (GCC extension: 5i).
    /// Parse integer suffix and return (is_unsigned, is_long, is_long_long, is_imaginary).
    /// is_long is true for single l/L suffix. is_long_long is true for ll/LL suffix.
    fn parse_int_suffix(&mut self) -> (bool, bool, bool, bool) {
        let mut is_imaginary = false;
        // First check for standalone 'i'/'I' imaginary suffix (GCC extension: 5i, 5I)
        // Must check this before the main loop since 'i'/'I' alone means imaginary, not a regular suffix
        if self.pos < self.input.len()
            && (self.input[self.pos] == b'i' || self.input[self.pos] == b'I')
        {
            // Check it's not the start of an identifier (like 'int')
            let next = if self.pos + 1 < self.input.len() {
                self.input[self.pos + 1]
            } else {
                0
            };
            if !next.is_ascii_alphanumeric() && next != b'_' {
                self.pos += 1; // consume 'i'/'I' as imaginary suffix
                return (false, false, false, true);
            }
        }

        let mut is_unsigned = false;
        let mut is_long = false;
        let mut is_long_long = false;
        loop {
            if self.pos < self.input.len()
                && (self.input[self.pos] == b'u' || self.input[self.pos] == b'U')
            {
                is_unsigned = true;
                self.pos += 1;
            } else if self.pos < self.input.len()
                && (self.input[self.pos] == b'l' || self.input[self.pos] == b'L')
            {
                self.pos += 1;
                // Check for second l/L for ll/LL
                if self.pos < self.input.len()
                    && (self.input[self.pos] == b'l' || self.input[self.pos] == b'L')
                {
                    is_long_long = true;
                    self.pos += 1;
                } else {
                    is_long = true;
                }
            } else if self.pos + 1 < self.input.len()
                && ((self.input[self.pos] == b'w' && self.input[self.pos + 1] == b'b')
                    || (self.input[self.pos] == b'W' && self.input[self.pos + 1] == b'B'))
            {
                // C23 6.4.4.1: `wb`/`WB` (and `uwb`/`UWB`/`wbu`/`WBU`) _BitInt literal suffix.
                self.pos += 2;
            } else {
                break;
            }
        }
        // Consume trailing 'i'/'I'/'j'/'J' for GCC imaginary suffix (e.g., 5li, 5ui, 5ULi, 5I)
        if self.pos < self.input.len()
            && (self.input[self.pos] == b'i'
                || self.input[self.pos] == b'I'
                || self.input[self.pos] == b'j'
                || self.input[self.pos] == b'J')
        {
            let next = if self.pos + 1 < self.input.len() {
                self.input[self.pos + 1]
            } else {
                0
            };
            if !next.is_ascii_alphanumeric() && next != b'_' {
                self.pos += 1;
                is_imaginary = true;
            }
        }
        (is_unsigned, is_long, is_long_long, is_imaginary)
    }

    /// Create the appropriate token kind based on integer value, suffix, and base info.
    /// For hex/octal literals, C promotes: int -> unsigned int -> long -> unsigned long -> long long -> unsigned long long.
    /// For decimal literals: int -> long -> long long (no implicit unsigned).
    fn make_int_token(
        &self,
        value: u64,
        is_unsigned: bool,
        is_long: bool,
        is_long_long: bool,
        is_hex_or_octal: bool,
        start: usize,
    ) -> Token {
        let span = Span::new(start as u32, self.pos as u32, self.file_id);
        if is_unsigned && is_long_long {
            // Explicit ULL suffix: always unsigned long long (64-bit)
            Token::new(TokenKind::ULongLongLiteral(value), span)
        } else if is_unsigned && is_long {
            // Explicit UL suffix: unsigned long
            Token::new(TokenKind::ULongLiteral(value), span)
        } else if is_unsigned {
            if value > u32::MAX as u64 {
                Token::new(TokenKind::ULongLiteral(value), span)
            } else {
                Token::new(TokenKind::UIntLiteral(value), span)
            }
        } else if is_long_long {
            // Explicit LL suffix: always long long (64-bit)
            if is_hex_or_octal && value > i64::MAX as u64 {
                Token::new(TokenKind::ULongLongLiteral(value), span)
            } else {
                Token::new(TokenKind::LongLongLiteral(value as i64), span)
            }
        } else if is_long {
            // Explicit L suffix: C11 6.4.4.1 type promotion for hex/octal:
            //   long -> unsigned long -> long long -> unsigned long long
            // On ILP32, long is 32-bit so values > i32::MAX need promotion.
            if is_hex_or_octal && crate::common::types::target_is_32bit() {
                if value <= i32::MAX as u64 {
                    Token::new(TokenKind::LongLiteral(value as i64), span)
                } else if value <= u32::MAX as u64 {
                    Token::new(TokenKind::ULongLiteral(value), span)
                } else if value <= i64::MAX as u64 {
                    Token::new(TokenKind::LongLongLiteral(value as i64), span)
                } else {
                    Token::new(TokenKind::ULongLongLiteral(value), span)
                }
            } else if is_hex_or_octal && value > i64::MAX as u64 {
                // LP64: long is 64-bit, value exceeds signed range -> unsigned long
                Token::new(TokenKind::ULongLiteral(value), span)
            } else {
                Token::new(TokenKind::LongLiteral(value as i64), span)
            }
        } else if is_hex_or_octal {
            // Hex/octal: int -> unsigned int -> long -> unsigned long
            if value <= i32::MAX as u64 {
                Token::new(TokenKind::IntLiteral(value as i64), span)
            } else if value <= u32::MAX as u64 {
                Token::new(TokenKind::UIntLiteral(value), span)
            } else if value <= i64::MAX as u64 {
                Token::new(TokenKind::LongLiteral(value as i64), span)
            } else {
                Token::new(TokenKind::ULongLiteral(value), span)
            }
        } else {
            // Decimal with no suffix: C11 6.4.4.1 Table 6
            // Type sequence: int -> long int -> long long int
            if value > i64::MAX as u64 {
                // Doesn't fit in any signed type; implementation-defined, use unsigned long
                Token::new(TokenKind::ULongLiteral(value), span)
            } else if crate::common::types::target_is_32bit() {
                // ILP32: int (32) -> long (32) -> long long (64)
                if value <= i32::MAX as u64 {
                    Token::new(TokenKind::IntLiteral(value as i64), span)
                } else {
                    // Doesn't fit in int or long (both 32-bit), promote to long long
                    Token::new(TokenKind::LongLongLiteral(value as i64), span)
                }
            } else {
                // LP64: int (32) -> long (64)
                if value <= i32::MAX as u64 {
                    Token::new(TokenKind::IntLiteral(value as i64), span)
                } else {
                    // Doesn't fit in int, promote to long
                    Token::new(TokenKind::LongLiteral(value as i64), span)
                }
            }
        }
    }

    fn lex_string(&mut self, start: usize) -> Token {
        self.pos += 1; // skip opening "
        let mut s = String::with_capacity(32);
        while self.pos < self.input.len() && self.input[self.pos] != b'"' {
            if self.input[self.pos] == b'\\' {
                self.pos += 1;
                if self.pos < self.input.len() {
                    let escape = self.lex_escape_text(0xFF);
                    // A narrow string is a byte string. `s` holds one char per
                    // output byte, so a UCN contributes one char per UTF-8 byte
                    // and every other escape contributes exactly one byte.
                    if escape.is_character() {
                        let mut buf = [0u8; 6];
                        for byte in generalized_utf8(escape.value, &mut buf) {
                            s.push(*byte as char);
                        }
                    } else {
                        s.push((escape.value & 0xFF) as u8 as char);
                    }
                }
            } else {
                // Decode PUA-encoded bytes back to original values for
                // non-UTF-8 source files (e.g., EUC-JP string literals)
                let (byte, consumed) = decode_pua_byte(&self.input, self.pos);
                s.push(byte as char);
                self.pos += consumed;
            }
        }
        if self.pos < self.input.len() {
            self.pos += 1; // skip closing "
        }
        Token::new(
            TokenKind::StringLiteral(s),
            Span::new(start as u32, self.pos as u32, self.file_id),
        )
    }

    fn lex_wide_string(&mut self, start: usize) -> Token {
        self.pos += 1; // skip opening "
        let mut s = String::with_capacity(32);
        while self.pos < self.input.len() && self.input[self.pos] != b'"' {
            if self.input[self.pos] == b'\\' {
                self.pos += 1;
                if self.pos < self.input.len() {
                    // Wide strings store code points directly (no UTF-8
                    // encoding needed), so an escape keeps the full 32-bit
                    // value rather than one byte of it.
                    let escape = self.lex_escape_text(u32::MAX);
                    s.push(char::from_u32(escape.value).unwrap_or('\u{FFFD}'));
                }
            } else {
                // Check for PUA-encoded bytes first (from non-UTF-8 source files)
                let (byte, consumed) = decode_pua_byte(&self.input, self.pos);
                if consumed > 1 {
                    // PUA byte: decode back to original byte value
                    s.push(byte as char);
                    self.pos += consumed;
                } else {
                    // Decode UTF-8 character for wide string
                    let byte = self.input[self.pos];
                    if byte < 0x80 {
                        s.push(byte as char);
                        self.pos += 1;
                    } else {
                        // Multi-byte UTF-8: decode to a single Unicode code point
                        let remaining = &self.input[self.pos..];
                        let end = std::cmp::min(4, remaining.len());
                        if let Ok(text) = std::str::from_utf8(&remaining[..end]) {
                            if let Some(ch) = text.chars().next() {
                                s.push(ch);
                                self.pos += ch.len_utf8();
                            } else {
                                s.push(byte as char);
                                self.pos += 1;
                            }
                        } else if let Ok(text) = std::str::from_utf8(remaining) {
                            if let Some(ch) = text.chars().next() {
                                s.push(ch);
                                self.pos += ch.len_utf8();
                            } else {
                                s.push(byte as char);
                                self.pos += 1;
                            }
                        } else {
                            s.push(byte as char);
                            self.pos += 1;
                        }
                    }
                }
            }
        }
        if self.pos < self.input.len() {
            self.pos += 1; // skip closing "
        }
        Token::new(
            TokenKind::WideStringLiteral(s),
            Span::new(start as u32, self.pos as u32, self.file_id),
        )
    }

    /// Lex a u"..." char16_t string literal. Same parsing as wide string but produces
    /// Char16StringLiteral token. The Rust String stores Unicode chars; the downstream
    /// pipeline stores their UTF-16 encoding (`str::encode_utf16`, C11 6.4.5p6), so a
    /// code point above U+FFFF becomes a surrogate pair and every length is counted
    /// in code units.
    fn lex_char16_string(&mut self, start: usize) -> Token {
        self.pos += 1; // skip opening "
        let mut s = String::with_capacity(32);
        while self.pos < self.input.len() && self.input[self.pos] != b'"' {
            if self.input[self.pos] == b'\\' {
                self.pos += 1;
                if self.pos < self.input.len() {
                    // `u"…"` holds UTF-16 code units, so an escape keeps
                    // the full 16-bit value rather than one byte of it.
                    let escape = self.lex_escape_text(0xFFFF);
                    s.push(char::from_u32(escape.value).unwrap_or('\u{FFFD}'));
                }
            } else {
                // Check for PUA-encoded bytes first
                let (byte, consumed) = decode_pua_byte(&self.input, self.pos);
                if consumed > 1 {
                    s.push(byte as char);
                    self.pos += consumed;
                } else {
                    let byte = self.input[self.pos];
                    if byte < 0x80 {
                        s.push(byte as char);
                        self.pos += 1;
                    } else {
                        let remaining = &self.input[self.pos..];
                        let end = std::cmp::min(4, remaining.len());
                        if let Ok(text) = std::str::from_utf8(&remaining[..end]) {
                            if let Some(ch) = text.chars().next() {
                                s.push(ch);
                                self.pos += ch.len_utf8();
                            } else {
                                s.push(byte as char);
                                self.pos += 1;
                            }
                        } else if let Ok(text) = std::str::from_utf8(remaining) {
                            if let Some(ch) = text.chars().next() {
                                s.push(ch);
                                self.pos += ch.len_utf8();
                            } else {
                                s.push(byte as char);
                                self.pos += 1;
                            }
                        } else {
                            s.push(byte as char);
                            self.pos += 1;
                        }
                    }
                }
            }
        }
        if self.pos < self.input.len() {
            self.pos += 1; // skip closing "
        }
        Token::new(
            TokenKind::Char16StringLiteral(s),
            Span::new(start as u32, self.pos as u32, self.file_id),
        )
    }

    /// Lex a `u8`/`u`/`U`/`L` character literal.
    ///
    /// The value is the code unit the constant denotes, and GCC 16.2 derives
    /// that unit — with the diagnostics it prints — from the literal's own
    /// width. Everything below is measured against gcc-16.2 on
    /// x86-64-linux-gnu:
    ///
    /// * a character that does not fit the unit (`u'\U0001F600'`) warns
    ///   `character not encodable in a single code unit` and keeps that
    ///   character's LAST code unit, i.e. the low surrogate (0xDE00), not the
    ///   low 16 bits of the code point (0xF600);
    /// * `u8` refuses such a character, and refuses a multi-character literal
    ///   (`multi-character literal cannot have an encoding prefix` is an error
    ///   there, a warning for `u`/`U`/`L`);
    /// * a multi-character wide literal keeps its LAST character (`u'ab'` is
    ///   'b'), while the narrow `'ab'` accumulates its bytes;
    /// * `\x`/`\ooo` escapes are byte values: they truncate to the width of the
    ///   unit with `hex`/`octal escape sequence out of range` (`u'\x100'` is
    ///   0x100 and clean, `u'\x10000'` is 0 with the warning, `'\x100'` is 0
    ///   with the warning);
    /// * `''` is `empty character constant`, an error in every form.
    fn lex_wide_char(&mut self, start: usize, kind: Option<UnicodeCharKind>) -> Token {
        self.pos += 1; // skip opening '
        let diagnostics_before = self.diagnostics.len();
        // Width of one code unit of this literal: the clamp for byte escapes.
        let max_unit: u32 = match kind {
            Some(UnicodeCharKind::Utf8) => 0xFF,
            Some(UnicodeCharKind::Utf16) => 0xFFFF,
            _ => u32::MAX,
        };
        // Whether a character needs more than one code unit of this literal.
        // `u8` counts UTF-8 bytes, so every character above 0x7F needs two.
        let unit_overflow = |value: u32| -> bool {
            match kind {
                Some(UnicodeCharKind::Utf8) => value > 0x7F,
                Some(UnicodeCharKind::Utf16) => value > 0xFFFF,
                _ => false,
            }
        };
        let mut value: u32 = 0;
        let mut characters = 0usize;
        // The constant denotes the last character, so only that one decides the
        // encodability diagnostics.
        let mut last_character_overflows = false;
        while self.pos < self.input.len() && self.input[self.pos] != b'\'' {
            last_character_overflows = false;
            if self.input[self.pos] == b'\\' {
                self.pos += 1;
                let escape = self.lex_escape_raw();
                if escape.is_character()
                    && escape.out_of_ucs
                    && matches!(kind, Some(UnicodeCharKind::Utf16))
                {
                    // A code point above U+10FFFF has no UTF-16 unit at all.
                    // GCC reports the conversion failure where it meets the
                    // escape — once per offending escape, in scan order — and
                    // the escape contributes no character to the literal
                    // (`u'\U00110000a'` is a ONE-character constant, so it
                    // warns about no encoding prefix).
                    self.emit_lex_diag(
                        "converting UCN to execution character set: \
                         Invalid or incomplete multibyte or wide character",
                        escape.span.start as usize,
                        escape.span.end as usize,
                    );
                    continue;
                }
                // A rejected escape can contribute no character at all
                // (`u'\xg'` is a one-character constant in GCC too).
                characters += usize::from(escape.contributed.unwrap_or(1) > 0);
                if escape.is_character() {
                    last_character_overflows = unit_overflow(escape.value);
                    value = escape.value;
                } else {
                    let at = Span::new(start as u32, escape.span.end, self.file_id);
                    self.warn_escape_range(escape, max_unit, at);
                    value = escape.value & max_unit;
                }
            } else {
                // One source character: a PUA byte stands for a source byte
                // (non-UTF-8 encodings), a UTF-8 sequence for one character of
                // the source character set, whose code point is the value.
                let (byte, consumed) = decode_pua_byte(&self.input, self.pos);
                if consumed > 1 {
                    value = byte as u32;
                    self.pos += consumed;
                } else if byte < 0x80 {
                    value = byte as u32;
                    self.pos += 1;
                } else {
                    let remaining = &self.input[self.pos..];
                    let end = std::cmp::min(remaining.len(), 4);
                    let decoded = std::str::from_utf8(&remaining[..end])
                        .ok()
                        .and_then(|text| text.chars().next())
                        .or_else(|| {
                            std::str::from_utf8(remaining)
                                .ok()
                                .and_then(|text| text.chars().next())
                        });
                    match decoded {
                        Some(ch) => {
                            value = ch as u32;
                            self.pos += ch.len_utf8();
                        }
                        None => {
                            value = byte as u32;
                            self.pos += 1;
                        }
                    }
                }
                characters += 1;
                last_character_overflows = unit_overflow(value);
            }
        }
        if self.pos < self.input.len() && self.input[self.pos] == b'\'' {
            self.pos += 1; // skip closing '
        }
        let span = Span::new(start as u32, self.pos as u32, self.file_id);
        if characters == 0 {
            if !self.saw_error_since(diagnostics_before) {
                self.emit_lex_diag("empty character constant", start, self.pos);
            }
            return Token::new(
                match kind {
                    Some(unit_kind) => TokenKind::UnicodeCharLiteral(0, unit_kind),
                    None => TokenKind::IntLiteral(0),
                },
                span,
            );
        }
        let multi = characters > 1;
        match kind {
            Some(unit_kind @ UnicodeCharKind::Utf8) => {
                if multi {
                    self.emit_lex_diag(
                        "multi-character literal cannot have an encoding prefix",
                        start,
                        self.pos,
                    );
                } else if last_character_overflows {
                    self.emit_lex_diag(
                        "character not encodable in a single code unit",
                        start,
                        self.pos,
                    );
                }
                Token::new(TokenKind::UnicodeCharLiteral(value, unit_kind), span)
            }
            Some(unit_kind @ UnicodeCharKind::Utf16) => {
                let mut unit_value = value;
                if multi {
                    self.warn(
                        "multi-character literal cannot have an encoding prefix",
                        span,
                    );
                }
                if last_character_overflows {
                    // The value is the LAST character's low surrogate even
                    // when the literal is multi-character (`u'a\U0001F600'`
                    // is 0xDE00), but GCC then reports only the encoding
                    // prefix.
                    if !multi {
                        self.warn("character not encodable in a single code unit", span);
                    }
                    unit_value = Self::low_surrogate(value);
                }
                Token::new(TokenKind::UnicodeCharLiteral(unit_value, unit_kind), span)
            }
            Some(unit_kind @ UnicodeCharKind::Utf32) => {
                if multi {
                    self.warn(
                        "multi-character literal cannot have an encoding prefix",
                        span,
                    );
                }
                Token::new(TokenKind::UnicodeCharLiteral(value, unit_kind), span)
            }
            // `L'x'` has type wchar_t, which is int on every target here.
            None => {
                if multi {
                    self.warn(
                        "multi-character literal cannot have an encoding prefix",
                        span,
                    );
                }
                Token::new(TokenKind::IntLiteral(value as i64), span)
            }
        }
    }

    /// Lex a narrow character constant.
    ///
    /// A narrow constant accumulates the execution character set's bytes, high
    /// byte first, into an `int`. GCC 16.2 diagnoses the literals whose value
    /// cannot survive that (measured):
    ///
    /// * two to four characters — `multi-character character constant`, which
    ///   `-Wno-multichar` silences (this is `-Wmultichar`, on by default);
    /// * five or more — `multi-character literal with N characters exceeds
    ///   'int' size of 4 bytes`, which has NO flag (GCC prints no label and
    ///   `-Wno-multichar` does not silence it) and keeps the low four bytes;
    /// * `''` — `empty character constant`, an error;
    /// * a `\x`/`\ooo` escape above 0xFF — `hex`/`octal escape sequence out of
    ///   range`, keeping the low byte, while the same escape in a wide literal
    ///   keeps its own width.
    fn lex_char(&mut self, start: usize) -> Token {
        self.pos += 1; // skip opening '
        let diagnostics_before = self.diagnostics.len();
        let mut value: i32 = 0;
        let mut characters = 0usize;
        while self.pos < self.input.len() && self.input[self.pos] != b'\'' {
            if self.input[self.pos] == b'\\' {
                self.pos += 1;
                let escape = self.lex_escape_raw();
                if let Some(contributed) = escape.contributed {
                    // A rejected escape: GCC's recovery decides how many
                    // characters (if any) it leaves in the constant, and adds
                    // no bytes of its own (`'\xg'` is just `g`).
                    if contributed > 0 {
                        let at = Span::new(start as u32, escape.span.end, self.file_id);
                        self.warn_escape_range(escape, 0xFF, at);
                        value = (value << 8) | ((escape.value & 0xFF) as i32);
                        characters += contributed as usize;
                    }
                } else if escape.is_character() {
                    // A universal character name is a CHARACTER: the execution
                    // character set maps it to 1-6 bytes of UTF-8. `'\u00e9'`
                    // is C3 A9 — a two-character constant, which GCC warns
                    // about — while `'\xe9'` is the single byte E9.
                    let mut buf = [0u8; 6];
                    for byte in generalized_utf8(escape.value, &mut buf) {
                        value = (value << 8) | (*byte as i32);
                        characters += 1;
                    }
                } else {
                    // Reported now, in scan order: `'\400\uD800'` warns about
                    // the range before it errors about the name (measured). The
                    // span is the literal so far, which is what GCC underlines.
                    let at = Span::new(start as u32, escape.span.end, self.file_id);
                    self.warn_escape_range(escape, 0xFF, at);
                    value = (value << 8) | ((escape.value & 0xFF) as i32);
                    characters += 1;
                }
            } else {
                // Decode PUA-encoded bytes for non-UTF-8 source files
                let (byte, consumed) = decode_pua_byte(&self.input, self.pos);
                self.pos += consumed;
                value = (value << 8) | (byte as i32);
                characters += 1;
            }
        }
        if self.pos < self.input.len() && self.input[self.pos] == b'\'' {
            self.pos += 1; // skip closing '
        }
        let span = Span::new(start as u32, self.pos as u32, self.file_id);
        if characters == 0 {
            if !self.saw_error_since(diagnostics_before) {
                self.emit_lex_diag("empty character constant", start, self.pos);
            }
            return Token::new(TokenKind::IntLiteral(0), span);
        }
        if characters > 4 {
            self.warn(
                format!(
                    "multi-character literal with {} characters exceeds 'int' size of 4 bytes",
                    characters
                ),
                span,
            );
        } else if characters > 1 {
            self.warn_kind(
                "multi-character character constant",
                span,
                WarningKind::Multichar,
            );
        }
        if characters <= 1 {
            // Single character: use CharLiteral with the char value
            let ch = if value == 0 {
                '\0'
            } else {
                (value as u8) as char
            };
            Token::new(TokenKind::CharLiteral(ch), span)
        } else {
            // Multi-character constant: produce an IntLiteral with the combined value
            Token::new(TokenKind::IntLiteral(value as i64), span)
        }
    }

    /// The low surrogate of the pair UTF-16 uses for `value`.
    ///
    /// GCC keeps the last code unit of the character it cannot encode
    /// (`u'\U0001F600'` is 0xDE00 = DC00 | ((1F600 - 10000) & 3FF), and
    /// `u'\U0010FFFF'` is 0xDFFF), which is what a target that truncates the
    /// character would see.
    fn low_surrogate(value: u32) -> u32 {
        0xDC00 | ((value.wrapping_sub(0x1_0000)) & 0x3FF)
    }

    /// Read one escape sequence for a *string* literal, whose payload is a Rust
    /// `String`: the value is clamped to `max_unit`, the width of one element
    /// of that literal (0xFF for a narrow or `u8` string, 0xFFFF for `u"…"`,
    /// `u32::MAX` for `L"…"`/`U"…"`), exactly as GCC clamps it.
    fn lex_escape_text(&mut self, max_unit: u32) -> RawEscape {
        let mut escape = self.lex_escape_sequence();
        if escape.out_of_ucs {
            // GCC 16.2 warns `\U00110000 is outside the UCS codespace` and
            // keeps the code point, encoding it in generalized UTF-8. A Rust
            // `String` cannot hold one above U+10FFFF, so the literal is
            // rejected instead (documented divergence, backlog GP-29).
            let text = self.escape_text(escape.span);
            self.emit_lex_diag(
                format!("{} is outside the UCS codespace", text),
                escape.span.start as usize,
                escape.span.end as usize,
            );
            escape.value = 0xFFFD;
            escape.out_of_ucs = false;
            return escape;
        }
        let at = self.escape_tail_span(escape);
        self.warn_escape_range(escape, max_unit, at);
        if !escape.is_character() {
            escape.value &= max_unit;
        }
        escape
    }

    /// Read one escape sequence for a *character constant*.
    ///
    /// GCC reports a code point above U+10FFFF here and keeps the value — a
    /// character constant is the one place that can represent it — and every
    /// other decision (range, encodability) belongs to the literal, which knows
    /// its own width.
    fn lex_escape_raw(&mut self) -> RawEscape {
        let escape = self.lex_escape_sequence();
        if escape.out_of_ucs {
            let text = self.escape_text(escape.span);
            self.warn(
                format!("{} is outside the UCS codespace", text),
                escape.span,
            );
        }
        escape
    }

    /// Parse one escape sequence, emitting only the diagnostics that do not
    /// depend on the literal it appears in.
    ///
    /// `self.pos` is at the escape letter; the caller has consumed the
    /// backslash.
    fn lex_escape_sequence(&mut self) -> RawEscape {
        let esc_start = self.pos.saturating_sub(1); // the backslash
        let mut escape = RawEscape {
            value: 0,
            kind: EscapeKind::Simple,
            span: Span::new(esc_start as u32, esc_start as u32, self.file_id),
            out_of_ucs: false,
            blew: false,
            contributed: None,
        };
        if self.pos >= self.input.len() {
            return escape;
        }
        let ch = self.input[self.pos];
        self.pos += 1;
        match ch {
            b'n' => escape.value = '\n' as u32,
            b't' => escape.value = '\t' as u32,
            b'r' => escape.value = '\r' as u32,
            b'\\' => escape.value = '\\' as u32,
            b'\'' => escape.value = '\'' as u32,
            b'"' => escape.value = '"' as u32,
            // A standard escape, spelled so that a trigraph cannot form.
            b'?' => escape.value = '?' as u32,
            b'a' => escape.value = 0x07,
            b'b' => escape.value = 0x08,
            b'e' | b'E' => escape.value = 0x1b, // GNU extension: ESC
            b'f' => escape.value = 0x0c,
            b'v' => escape.value = 0x0b,
            b'o' => {
                // C2y / GCC 15+ delimited octal escape: \o{...}. Without its
                // brace `\o` is not an escape at all.
                escape.kind = EscapeKind::Octal;
                if self.pos < self.input.len() && self.input[self.pos] == b'{' {
                    let (value, blew, contributed) = self.lex_delimited_escape(8, b'o', esc_start);
                    escape.value = value;
                    escape.blew = blew;
                    escape.contributed = contributed;
                } else {
                    self.emit_lex_diag("'\\o' not followed by '{'", esc_start, self.pos);
                    escape.kind = EscapeKind::Simple;
                    escape.value = 'o' as u32;
                }
            }
            b'x' => {
                escape.kind = EscapeKind::Hex;
                if self.pos < self.input.len() && self.input[self.pos] == b'{' {
                    // C2y / GCC 15+ delimited hex escape: \x{...}
                    let (value, blew, contributed) = self.lex_delimited_escape(16, b'x', esc_start);
                    escape.value = value;
                    escape.blew = blew;
                    escape.contributed = contributed;
                } else {
                    // Hex escape: \xNN - consumes all hex digits. The value is
                    // kept whole: whether it fits is the literal's decision.
                    let digits_start = self.pos;
                    let (value, blew) = self.lex_radix_digits(16);
                    if self.pos == digits_start {
                        self.emit_lex_diag(
                            "'\\x' used with no following hex digits",
                            esc_start,
                            self.pos,
                        );
                        // The failed escape contributed nothing: `'\xg'` is a
                        // one-character constant in GCC, which is why it
                        // carries no multi-character diagnostic at all.
                        escape.contributed = Some(0);
                    }
                    escape.value = value;
                    escape.blew = blew;
                }
            }
            b'u' => {
                escape.kind = EscapeKind::Universal;
                if self.pos < self.input.len() && self.input[self.pos] == b'{' {
                    // C2y / GCC 15+ delimited universal character name: \u{...}
                    let (value, blew, contributed) = self.lex_delimited_escape(16, b'u', esc_start);
                    escape.value = value;
                    escape.blew = blew;
                    escape.contributed = contributed;
                } else {
                    Self::take_ucn(&mut escape, self.lex_ucn_digits(4));
                }
            }
            b'U' => {
                escape.kind = EscapeKind::Universal;
                Self::take_ucn(&mut escape, self.lex_ucn_digits(8));
            }
            b'N' => {
                if !self.lex_named_ucn(&mut escape) {
                    // Without its `{NAME}` GCC reads `\N` exactly like `\u`:
                    // four hex digits, the same "incomplete" error echoing
                    // what it read, and the same character count — so
                    // `'\Nabcd'` is U+ABCD (three UTF-8 bytes, which makes it
                    // a multi-character constant) while `'\Na'` is one
                    // character and says nothing further.
                    escape.kind = EscapeKind::Universal;
                    Self::take_ucn(&mut escape, self.lex_ucn_digits(4));
                }
            }
            b'0'..=b'7' => {
                // Octal escape: \0 through \377 (1-3 octal digits)
                escape.kind = EscapeKind::Octal;
                let mut value = (ch - b'0') as u32;
                for _ in 0..2 {
                    if self.pos < self.input.len()
                        && self.input[self.pos] >= b'0'
                        && self.input[self.pos] <= b'7'
                    {
                        value = value * 8 + (self.input[self.pos] - b'0') as u32;
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                escape.value = value;
            }
            // Everything else is an unknown escape: GCC warns (no flag, so
            // only `-w` and `-Werror` act on it) and keeps the character. The
            // three exceptions are measured: `\%`, `\[` and `\{` are silent.
            // A non-graphical byte is spelled in octal (`'\040'` for a space,
            // `'\303'` for the first byte of `é`), which is why the message is
            // built from the byte and not from the source text.
            other => {
                if !matches!(other, b'%' | b'[' | b'{') {
                    let spelled = if other.is_ascii_graphic() {
                        (other as char).to_string()
                    } else {
                        format!("{:03o}", other)
                    };
                    self.warn(
                        format!("unknown escape sequence: '\\{}'", spelled),
                        Span::new(esc_start as u32, self.pos as u32, self.file_id),
                    );
                }
                escape.value = other as u32;
            }
        }
        escape.span = Span::new(esc_start as u32, self.pos as u32, self.file_id);
        if matches!(escape.kind, EscapeKind::Universal | EscapeKind::Named) {
            // A surrogate is never a character, and GCC 16.2 rejects anything
            // past the int range: both are `… is not a valid universal
            // character` in every literal and in identifiers (measured). A code
            // point that is merely outside the UCS codespace is a warning the
            // literal reports, because GCC keeps the value.
            if (0xD800..=0xDFFF).contains(&escape.value) || escape.value > 0x7FFF_FFFF {
                let text = self.escape_text(escape.span);
                self.emit_lex_diag(
                    format!("{} is not a valid universal character", text),
                    esc_start,
                    self.pos,
                );
                // A rejected escape is one ordinary character, and GCC keeps
                // reporting what the rest of the literal does with it
                // (`'\uD800a'` still warns `multi-character character
                // constant`, `u8'\uD800a'` still errors about its prefix).
                escape.value = 0xFFFD;
                escape.kind = EscapeKind::Simple;
            } else if escape.value > 0x10FFFF {
                escape.out_of_ucs = true;
            }
        }
        escape
    }

    /// Record a universal character name's value, or mark the escape rejected
    /// when the name was incomplete.
    fn take_ucn(escape: &mut RawEscape, value: Option<u32>) {
        match value {
            Some(value) => escape.value = value,
            // An incomplete name keeps scanning as the letter it could not
            // complete: GCC counts one character for it and reports nothing
            // further, so `'\U1234'` (the four digits it did read) is a
            // one-character constant while `'\U{110000}'` (no digit read, eight
            // characters left over) is `multi-character literal with 9
            // characters exceeds 'int' size of 4 bytes` (both measured).
            None => {
                escape.value = 0xFFFD;
                escape.kind = EscapeKind::Simple;
            }
        }
    }

    /// Read the digits of a `\x…`/`\o{…}`-style escape in `radix`, returning the
    /// value and whether it outgrew 32 bits.
    fn lex_radix_digits(&mut self, radix: u32) -> (u32, bool) {
        let mut value = 0u32;
        let mut blew = false;
        while self.pos < self.input.len() {
            let digit = match (radix, self.input[self.pos]) {
                (16, byte) if byte.is_ascii_hexdigit() => hex_digit_val(byte) as u32,
                (8, byte @ b'0'..=b'7') => (byte - b'0') as u32,
                _ => break,
            };
            match value.checked_mul(radix).and_then(|v| v.checked_add(digit)) {
                Some(next) => value = next,
                None => {
                    value = value.wrapping_mul(radix).wrapping_add(digit);
                    blew = true;
                }
            }
            self.pos += 1;
        }
        (value, blew)
    }

    /// Parse a C2y delimited escape (`\x{…}`, `\o{…}`, `\u{…}`).
    ///
    /// GCC 16.2 decides by lookahead whether the braces spell one at all, and
    /// every branch below is its measured behaviour: digits of the right radix
    /// followed by `}` are a delimited escape; `{}` is `empty delimited escape
    /// sequence`; anything else makes the braces ordinary characters, after
    /// `'\x' used with no following hex digits` (`\x{` with no digits) or
    /// `'\x{' not terminated with '}' after \x{41` (echoing what was read, and
    /// what `\o{7x}` and `\u{12z}` also report). That is why `'\x{z}'` still
    /// reaches the multi-character literal warning while `'\x{}'` does not.
    fn lex_delimited_escape(
        &mut self,
        radix: u32,
        letter: u8,
        esc_start: usize,
    ) -> (u32, bool, Option<u8>) {
        self.pos += 1; // consume '{'
        let digits_start = self.pos;
        let (value, blew) = self.lex_radix_digits(radix);
        let digits_end = self.pos;
        if self.pos < self.input.len() && self.input[self.pos] == b'}' {
            self.pos += 1;
            // A terminated escape contributes its character, except `\x{}`
            // and `\o{}`: the empty hex and octal forms contribute nothing,
            // while the empty `\u{}` is one NUL character (measured).
            if digits_end == digits_start {
                self.emit_lex_diag("empty delimited escape sequence", esc_start, self.pos);
                if letter != b'u' {
                    return (value, blew, Some(0));
                }
            }
            return (value, blew, None);
        }
        // Unterminated: GCC keeps what it read (or the digits and brace for
        // `\x{`), resumes at the character that broke the escape, and counts
        // the escape as nothing for `\x{`/`\o{` but as one character for
        // `\u{` (all measured, and all observable in the count GCC prints).
        let text = self.escape_text(Span::new(esc_start as u32, digits_end as u32, self.file_id));
        if digits_end == digits_start && letter == b'x' {
            self.emit_lex_diag(
                "'\\x' used with no following hex digits",
                esc_start,
                digits_end,
            );
        } else {
            self.emit_lex_diag(
                format!(
                    "'\\{}{{' not terminated with '}}' after {}",
                    letter as char, text
                ),
                esc_start,
                digits_end,
            );
        }
        // Resume at the character that broke the escape; `\x{` has already
        // consumed its brace, so the byte after it is the first ordinary
        // character (`'\x{zz}'` is a TWO-character constant: `z` and `}`).
        self.pos = digits_end;
        (0, false, Some(if letter == b'u' { 1 } else { 0 }))
    }

    /// Parse a universal character name (`\uNNNN` or `\UNNNNNNNN`) and return
    /// its code point. `num_digits` is 4 for `\u` or 8 for `\U`.
    fn lex_ucn_digits(&mut self, num_digits: usize) -> Option<u32> {
        let esc_start = self.pos.saturating_sub(2);
        let mut value = 0u32;
        let mut got = 0usize;
        for _ in 0..num_digits {
            if self.pos < self.input.len() && self.input[self.pos].is_ascii_hexdigit() {
                value = value * 16 + hex_digit_val(self.input[self.pos]) as u32;
                self.pos += 1;
                got += 1;
            } else {
                break;
            }
        }
        if got < num_digits {
            // GCC echoes what it read: `incomplete universal character name
            // \u12`, and the same shape for `\U`.
            let text = self.escape_text(Span::new(esc_start as u32, self.pos as u32, self.file_id));
            self.emit_lex_diag(
                format!("incomplete universal character name {}", text),
                esc_start,
                self.pos,
            );
            // GCC reports the incomplete escape alone: `'\U1234'` gets no
            // `multi-character character constant` warning on top of it.
            return None;
        }
        Some(value)
    }

    /// Parse C++23's named universal character escape `\N{NAME}` into
    /// `escape`, returning whether it *was* one.
    ///
    /// `false` means the name is missing its `{`: GCC reports that and then
    /// reads the escape the way `\u` is read, which is the caller's job
    /// because only it knows how many hex digits `\u` takes. Every other
    /// shape fills in `escape` completely, including the character count its
    /// error recovery leaves behind (see [`RawEscape::contributed`]).
    fn lex_named_ucn(&mut self, escape: &mut RawEscape) -> bool {
        let esc_start = self.pos.saturating_sub(2);
        if self.pos >= self.input.len() || self.input[self.pos] != b'{' {
            // GCC reports the missing brace and then re-reads the escape the
            // way `\u` is read; the caller does that, because only it knows
            // how many hex digits `\u` would take.
            self.emit_lex_diag("'\\N' not followed by '{'", esc_start, self.pos);
            return false;
        }
        self.pos += 1; // consume '{'
        let name_start = self.pos;
        // The name ends at `}`. A quote, a newline or the end of input ends the
        // *literal* instead, which is GCC's reading of
        // `"\N{LATIN SMALL LETTER A"` ("not terminated with '}'").
        while self.pos < self.input.len()
            && !matches!(self.input[self.pos], b'}' | b'"' | b'\'' | b'\n')
        {
            self.pos += 1;
        }
        escape.kind = EscapeKind::Named;
        if self.pos >= self.input.len() || self.input[self.pos] != b'}' {
            self.emit_lex_diag(
                format!(
                    "'\\N{{' not terminated with '}}' after \\N{{{}",
                    String::from_utf8_lossy(&self.input[name_start..self.pos])
                ),
                esc_start,
                self.pos,
            );
            // GCC gives up on the literal here: the escape comes out of the
            // count entirely, so the constant is neither multi-character nor
            // "empty" — it only has the one error.
            escape.value = '\u{FFFD}' as u32;
            escape.kind = EscapeKind::Simple;
            escape.contributed = Some(0);
            return true;
        }
        self.pos += 1; // consume '}'
        let name = &self.input[name_start..self.pos - 1];
        if name.is_empty() {
            self.emit_lex_diag(
                "empty named universal character escape sequence",
                esc_start,
                self.pos,
            );
            escape.value = '\u{FFFD}' as u32;
            escape.kind = EscapeKind::Simple;
            escape.contributed = Some(1);
            return true;
        }
        match super::ucn_name::lookup(name) {
            Some(value) => {
                escape.value = value;
                true
            }
            None => {
                self.emit_lex_diag(
                    format!(
                        "'\\N{{{}}}' is not a valid universal character",
                        String::from_utf8_lossy(name)
                    ),
                    esc_start,
                    self.pos,
                );
                // An invalid name counts as two characters in GCC's tally
                // (`'\N{NO SUCH NAME}zzzz'` is six, not seven).
                escape.value = '\u{FFFD}' as u32;
                escape.kind = EscapeKind::Named;
                escape.contributed = Some(2);
                true
            }
        }
    }

    /// Lex an identifier whose first byte is a `\\u`/`\\U` universal character
    /// name or a UTF-8 lead byte. `self.pos` is still at `start`; `None` means
    /// the bytes are not an identifier, so the caller falls through to its
    /// stray-character handling.
    fn try_lex_extended_identifier(&mut self, start: usize) -> Option<Token> {
        let first = *self.input.get(start)?;
        if first < 0x80 && first != b'\\' {
            return None;
        }
        self.finish_extended_identifier(start, String::new())
    }

    /// Scan the rest of an identifier whose ASCII prefix `start..self.pos` is
    /// already consumed and whose decoded text is `name` (empty when the name
    /// starts with a UCN or a UTF-8 character).
    ///
    /// The decoded name is the UTF-8 spelling of the identifier, so
    /// `caf\u00e9`, `café` and `caf\U000000E9` all intern to the same symbol,
    /// which is what C99 6.4.2.1 requires (a UCN denotes the character, not a
    /// textual escape).
    ///
    /// Validity is decided by the measured Annex D tables in
    /// [`super::ucn_id_table`], not by a Unicode property: GCC 16.2 accepts
    /// `€` and U+0301 and rejects U+00A9 and the private-use planes, and every
    /// diagnostic below reuses GCC's wording because these messages are part
    /// of the differential contract (see
    /// `tests/regression/check_ucn_identifier_table.sh`).
    fn finish_extended_identifier(&mut self, start: usize, mut name: String) -> Option<Token> {
        use super::ucn_id_table::{is_identifier_continue, is_identifier_start};
        let mut pos = self.pos.max(start);
        loop {
            let Some(&lead) = self.input.get(pos) else {
                break;
            };
            let at_start = name.is_empty();
            let (ch, len) = if lead == b'\\' {
                let Some((value, len)) = Self::decode_identifier_ucn(&self.input[pos..]) else {
                    // A malformed or truncated escape ends the identifier; the
                    // backslash is then reported as a stray character.
                    break;
                };
                let spelling = std::str::from_utf8(&self.input[pos..pos + len]).unwrap_or_default();
                // GCC rejects a surrogate code point or anything above the
                // 31-bit UCN range as "not a valid universal character", and
                // reports the escape exactly as it was spelled (`\ud800`
                // stays lower case).
                let valid_char = value <= 0x7FFF_FFFF && !(0xD800..=0xDFFF).contains(&value);
                // A named escape is decodable but never legal in an
                // identifier (no `\N` production exists there, in C or C++),
                // whatever character it denotes, so it is rejected before the
                // Annex D tables get a say.
                let named = self.input[pos..].starts_with(b"\\N{");
                let allowed = !named
                    && if valid_char {
                        if at_start {
                            is_identifier_start(value)
                        } else {
                            is_identifier_continue(value)
                        }
                    } else {
                        false
                    };
                if !allowed {
                    let message = if named {
                        format!("universal character {spelling} is not valid in an identifier")
                    } else if !valid_char {
                        format!("{spelling} is not a valid universal character")
                    } else if at_start && is_identifier_continue(value) {
                        format!(
                            "universal character {spelling} is not valid at the start of an identifier"
                        )
                    } else {
                        format!("universal character {spelling} is not valid in an identifier")
                    };
                    // The escape remains part of the identifier: GCC reports a
                    // single diagnostic here and keeps parsing, so the token
                    // carries the decoded name.
                    self.pos = pos + len;
                    self.emit_lex_diag(message, start, self.pos);
                    if valid_char {
                        name.push(char::from_u32(value).unwrap_or('\u{FFFD}'));
                    }
                    return Some(Token::new(
                        TokenKind::Identifier(name),
                        Span::new(start as u32, self.pos as u32, self.file_id),
                    ));
                }
                (char::from_u32(value).unwrap_or('\u{FFFD}'), len)
            } else {
                let len = match lead {
                    0x00..=0x7F => 1,
                    0xC0..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    0xF0..=0xF7 => 4,
                    _ => break,
                };
                if self.input.len() < pos + len {
                    break;
                }
                match std::str::from_utf8(&self.input[pos..pos + len])
                    .ok()
                    .and_then(|s| s.chars().next())
                {
                    Some(c) => (c, len),
                    None => break,
                }
            };
            if lead != b'\\' && ch >= '\u{80}' {
                // Raw UTF-8 is judged by the same table, with GCC's distinct
                // wording for a character that may only appear inside an
                // identifier (`extended character ́ is not valid at the start
                // of an identifier`).
                if at_start {
                    if !is_identifier_start(ch as u32) {
                        if is_identifier_continue(ch as u32) {
                            self.pos = pos + len;
                            self.emit_lex_diag(
                                format!(
                                    "extended character {ch} is not valid at the start of an identifier"
                                ),
                                start,
                                self.pos,
                            );
                            name.push(ch);
                            return Some(Token::new(
                                TokenKind::Identifier(name),
                                Span::new(start as u32, self.pos as u32, self.file_id),
                            ));
                        }
                        return None;
                    }
                } else if !is_identifier_continue(ch as u32) {
                    break;
                }
            } else if lead != b'\\' && !(ch == '_' || ch == '$' || ch.is_ascii_alphanumeric()) {
                // ASCII that the caller did not consume: not an identifier
                // character, and not this function's business.
                if at_start {
                    return None;
                }
                break;
            }
            pos += len;
            name.push(ch);
        }
        if name.is_empty() {
            return None;
        }
        self.pos = pos;
        let span = Span::new(start as u32, pos as u32, self.file_id);
        if let Some(kw) = TokenKind::from_keyword(&name, self.gnu_extensions, self.c23) {
            return Some(Token::new(kw, span));
        }
        Some(Token::new(TokenKind::Identifier(name), span))
    }

    /// Decode one `\\uXXXX` / `\\UXXXXXXXX` universal character name at the
    /// start of `input`, returning the code point (not a `char`: a surrogate or
    /// an out-of-range value is diagnosed, not discarded) and the byte length
    /// consumed. `None` when the bytes are not a well-formed 6/10-byte escape,
    /// including a truncated one, which GCC also treats as not-a-UCN.
    fn decode_identifier_ucn(input: &[u8]) -> Option<(u32, usize)> {
        let digits = match input.get(1) {
            Some(b'u') => 4,
            Some(b'U') => 8,
            Some(b'N') => {
                // C23 spells a named escape `\N{NAME}`. It is *never* valid in
                // an identifier (only `\u`/`\U` are allowed there), but
                // decoding it here is what lets the caller report GCC's
                // "universal character \N{...} is not valid in an identifier"
                // instead of a stray backslash. A name that is not well formed
                // is left to the stray-character path, exactly as GCC does for
                // `int \N{BOGUS};`.
                let rest = input.get(2..)?;
                if rest.first() != Some(&b'{') {
                    return None;
                }
                let close = rest.iter().position(|&b| b == b'}')?;
                let name = &rest[1..close];
                if name.len() > crate::frontend::lexer::ucn_name_table::MAX_NAME_LEN
                    || name.iter().any(|&b| {
                        !(b.is_ascii_uppercase() || b.is_ascii_digit() || b == b' ' || b == b'-')
                    })
                {
                    return None;
                }
                let value = crate::frontend::lexer::ucn_name::lookup(name)?;
                return Some((value, 3 + close));
            }
            _ => return None,
        };
        let mut value: u32 = 0;
        for &b in input.get(2..2 + digits)? {
            if !b.is_ascii_hexdigit() {
                return None;
            }
            value = value * 16 + u32::from(hex_digit_val(b));
        }
        Some((value, 2 + digits))
    }

    fn lex_identifier(&mut self, start: usize) -> Token {
        // ASCII prefix only. A backslash (UCN) or high byte (raw UTF-8) ends
        // this loop and is decoded by `finish_extended_identifier`, which
        // interns `caf\u00e9`, `café` and `caf\U000000E9` as one symbol per
        // C99 6.4.2.1 and enforces the measured Annex D tables. Skipping the
        // UCN bytes here would leave the literal escape text in the token and
        // silently bypass both the conversion and the validity diagnostics.
        while self.pos < self.input.len()
            && (self.input[self.pos] == b'_'
                || self.input[self.pos] == b'$'
                || self.input[self.pos].is_ascii_alphanumeric())
        {
            self.pos += 1;
        }
        // An extended identifier may continue with a universal character name
        // or a UTF-8 character (`café`, `caf\u00e9`). The ASCII prefix is
        // already consumed, so the continuation is scanned from here instead of
        // re-scanning the whole name.
        if self
            .input
            .get(self.pos)
            .is_some_and(|&b| b >= 0x80 || b == b'\\')
        {
            let name = String::from_utf8_lossy(&self.input[start..self.pos]).into_owned();
            if let Some(tok) = self.finish_extended_identifier(start, name) {
                return tok;
            }
        }

        // Check for wide/unicode char/string prefixes: L'x', L"...", u'x', u"...", U'x', U"..."
        if self.pos < self.input.len() {
            let text_len = self.pos - start;
            let next = self.input[self.pos];
            if next == b'\'' || next == b'"' {
                let prefix = &self.input[start..self.pos];
                let is_wide_prefix = match text_len {
                    1 => prefix[0] == b'L' || prefix[0] == b'u' || prefix[0] == b'U',
                    2 => prefix == b"u8",
                    _ => false,
                };
                if is_wide_prefix {
                    if next == b'\'' {
                        // Wide/unicode char literal: value is the Unicode code point
                        let kind = match prefix {
                            b"u8" => Some(UnicodeCharKind::Utf8),
                            b"u" => Some(UnicodeCharKind::Utf16),
                            b"U" => Some(UnicodeCharKind::Utf32),
                            _ => None, // L'x': wchar_t, which is int on every target here
                        };
                        return self.lex_wide_char(start, kind);
                    } else {
                        // Wide/unicode string literal: L"...", u"...", U"...", u8"..."
                        let is_wide_32 = text_len == 1 && (prefix[0] == b'L' || prefix[0] == b'U');
                        let is_char16 = text_len == 1 && prefix[0] == b'u';
                        if is_wide_32 {
                            return self.lex_wide_string(start);
                        } else if is_char16 {
                            // u"..." - char16_t string (16-bit elements)
                            return self.lex_char16_string(start);
                        } else {
                            // u8"..." - UTF-8 string, same as narrow string
                            return self.lex_string(start);
                        }
                    }
                }
            }
        }

        let text = std::str::from_utf8(&self.input[start..self.pos]).unwrap_or("");
        let span = Span::new(start as u32, self.pos as u32, self.file_id);

        // Check for synthetic pragma pack directives emitted by preprocessor
        if let Some(pack_tok) = Self::try_pragma_pack_token(text) {
            return Token::new(pack_tok, span);
        }

        // Check for synthetic pragma visibility directives emitted by preprocessor
        if let Some(vis_tok) = Self::try_pragma_visibility_token(text) {
            return Token::new(vis_tok, span);
        }

        if let Some(kw) = TokenKind::from_keyword(text, self.gnu_extensions, self.c23) {
            Token::new(kw, span)
        } else {
            let bytes = self.input[start..self.pos].to_vec();
            let name = self.intern_ident(&bytes);
            Token::new(TokenKind::Identifier(name), span)
        }
    }

    /// Recognize synthetic pragma pack identifiers emitted by the preprocessor.
    /// Format: __ccc_pack_set_N, __ccc_pack_push_N, __ccc_pack_push_only,
    ///         __ccc_pack_pop, __ccc_pack_reset
    fn try_pragma_pack_token(text: &str) -> Option<TokenKind> {
        if let Some(rest) = text.strip_prefix("__ccc_pack_") {
            if rest == "pop" {
                Some(TokenKind::PragmaPackPop)
            } else if rest == "reset" {
                Some(TokenKind::PragmaPackReset)
            } else if rest == "push_only" {
                Some(TokenKind::PragmaPackPushOnly)
            } else if let Some(n_str) = rest.strip_prefix("set_") {
                if let Ok(n) = n_str.parse::<usize>() {
                    Some(TokenKind::PragmaPackSet(n))
                } else {
                    None
                }
            } else if let Some(n_str) = rest.strip_prefix("push_") {
                if let Ok(n) = n_str.parse::<usize>() {
                    Some(TokenKind::PragmaPackPush(n))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Recognize synthetic pragma visibility identifiers emitted by the preprocessor.
    /// Format: __ccc_visibility_push_VISIBILITY, __ccc_visibility_pop
    fn try_pragma_visibility_token(text: &str) -> Option<TokenKind> {
        if let Some(rest) = text.strip_prefix("__ccc_visibility_") {
            if rest == "pop" {
                Some(TokenKind::PragmaVisibilityPop)
            } else if let Some(vis) = rest.strip_prefix("push_") {
                match vis {
                    "hidden" | "default" | "protected" | "internal" => {
                        Some(TokenKind::PragmaVisibilityPush(vis.to_string()))
                    }
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    fn lex_punctuation(&mut self, start: usize) -> Option<Token> {
        let ch = self.input[self.pos];
        self.pos += 1;

        let kind = match ch {
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'{' => TokenKind::LBrace,
            b'}' => TokenKind::RBrace,
            b'[' => TokenKind::LBracket,
            b']' => TokenKind::RBracket,
            b';' => TokenKind::Semicolon,
            b',' => TokenKind::Comma,
            b'~' => TokenKind::Tilde,
            b'?' => TokenKind::Question,
            b':' => {
                // Digraph `:>` is `]` (C95 6.4.6, accepted by GCC in every
                // dialect; `:>` is the only spelling whose meaning changes
                // when it is not recognised, so it is lexed here).
                if self.input.get(self.pos) == Some(&b'>') {
                    self.pos += 1;
                    TokenKind::RBracket
                } else {
                    TokenKind::Colon
                }
            }
            b'#' => {
                if self.pos < self.input.len() && self.input[self.pos] == b':' {
                    // Digraph #: → ##
                    self.pos += 1;
                    TokenKind::HashHash
                } else if self.pos < self.input.len() && self.input[self.pos] == b'#' {
                    self.pos += 1;
                    TokenKind::HashHash
                } else {
                    TokenKind::Hash
                }
            }
            b'.' => {
                if self.pos + 1 < self.input.len()
                    && self.input[self.pos] == b'.'
                    && self.input[self.pos + 1] == b'.'
                {
                    self.pos += 2;
                    TokenKind::Ellipsis
                } else {
                    TokenKind::Dot
                }
            }
            b'+' => {
                if self.pos < self.input.len() {
                    match self.input[self.pos] {
                        b'+' => {
                            self.pos += 1;
                            TokenKind::PlusPlus
                        }
                        b'=' => {
                            self.pos += 1;
                            TokenKind::PlusAssign
                        }
                        _ => TokenKind::Plus,
                    }
                } else {
                    TokenKind::Plus
                }
            }
            b'-' => {
                if self.pos < self.input.len() {
                    match self.input[self.pos] {
                        b'-' => {
                            self.pos += 1;
                            TokenKind::MinusMinus
                        }
                        b'=' => {
                            self.pos += 1;
                            TokenKind::MinusAssign
                        }
                        b'>' => {
                            self.pos += 1;
                            TokenKind::Arrow
                        }
                        _ => TokenKind::Minus,
                    }
                } else {
                    TokenKind::Minus
                }
            }
            b'*' => {
                if self.pos < self.input.len() && self.input[self.pos] == b'=' {
                    self.pos += 1;
                    TokenKind::StarAssign
                } else {
                    TokenKind::Star
                }
            }
            b'/' => {
                if self.pos < self.input.len() && self.input[self.pos] == b'=' {
                    self.pos += 1;
                    TokenKind::SlashAssign
                } else {
                    TokenKind::Slash
                }
            }
            b'%' => match self.input.get(self.pos) {
                // Digraph punctuators (C95 6.4.6, accepted by GCC in every
                // dialect): `%:` is `#`, `%:%:` is `##`, `%>` is `}`.
                Some(b'=') => {
                    self.pos += 1;
                    TokenKind::PercentAssign
                }
                Some(b'>') => {
                    self.pos += 1;
                    TokenKind::RBrace
                }
                Some(b':') => {
                    self.pos += 1;
                    if self.input.get(self.pos..self.pos + 2) == Some(b"%:") {
                        self.pos += 2;
                        TokenKind::HashHash
                    } else {
                        TokenKind::Hash
                    }
                }
                _ => TokenKind::Percent,
            },
            b'&' => {
                if self.pos < self.input.len() {
                    match self.input[self.pos] {
                        b'&' => {
                            self.pos += 1;
                            TokenKind::AmpAmp
                        }
                        b'=' => {
                            self.pos += 1;
                            TokenKind::AmpAssign
                        }
                        _ => TokenKind::Amp,
                    }
                } else {
                    TokenKind::Amp
                }
            }
            b'|' => {
                if self.pos < self.input.len() {
                    match self.input[self.pos] {
                        b'|' => {
                            self.pos += 1;
                            TokenKind::PipePipe
                        }
                        b'=' => {
                            self.pos += 1;
                            TokenKind::PipeAssign
                        }
                        _ => TokenKind::Pipe,
                    }
                } else {
                    TokenKind::Pipe
                }
            }
            b'^' => {
                if self.pos < self.input.len() && self.input[self.pos] == b'=' {
                    self.pos += 1;
                    TokenKind::CaretAssign
                } else {
                    TokenKind::Caret
                }
            }
            b'!' => {
                if self.pos < self.input.len() && self.input[self.pos] == b'=' {
                    self.pos += 1;
                    TokenKind::BangEqual
                } else {
                    TokenKind::Bang
                }
            }
            b'=' => {
                if self.pos < self.input.len() && self.input[self.pos] == b'=' {
                    self.pos += 1;
                    TokenKind::EqualEqual
                } else {
                    TokenKind::Assign
                }
            }
            b'<' => {
                if self.pos < self.input.len() {
                    match self.input[self.pos] {
                        // Digraph `<:` is `[` (C95 6.4.6).
                        b':' => {
                            // Digraph <: → [
                            self.pos += 1;
                            TokenKind::LBracket
                        }
                        // Digraph `<%` is `{` (C95 6.4.6).
                        b'%' => {
                            // Digraph <% → {
                            self.pos += 1;
                            TokenKind::LBrace
                        }
                        b'<' => {
                            self.pos += 1;
                            if self.pos < self.input.len() && self.input[self.pos] == b'=' {
                                self.pos += 1;
                                TokenKind::LessLessAssign
                            } else {
                                TokenKind::LessLess
                            }
                        }
                        b'=' => {
                            self.pos += 1;
                            TokenKind::LessEqual
                        }
                        _ => TokenKind::Less,
                    }
                } else {
                    TokenKind::Less
                }
            }
            b'>' => {
                if self.pos < self.input.len() {
                    match self.input[self.pos] {
                        b'>' => {
                            self.pos += 1;
                            if self.pos < self.input.len() && self.input[self.pos] == b'=' {
                                self.pos += 1;
                                TokenKind::GreaterGreaterAssign
                            } else {
                                TokenKind::GreaterGreater
                            }
                        }
                        b'=' => {
                            self.pos += 1;
                            TokenKind::GreaterEqual
                        }
                        _ => TokenKind::Greater,
                    }
                } else {
                    TokenKind::Greater
                }
            }
            _ => {
                // Non-ASCII or unknown character: skip any remaining bytes of
                // a multi-byte UTF-8 sequence (including PUA-encoded bytes from
                // non-UTF-8 source files) and continue tokenizing.
                // The leading byte was consumed before this match. Reading
                // self.input[self.pos] instead skips its successor and reads
                // past EOF for a final unknown byte.
                let bad_start = start;
                // A byte of a non-UTF-8 source file was re-encoded into the
                // private use area on load; report the original byte, not the
                // lead byte of its PUA encoding (`stray '\377'`, not `'\356'`).
                let (bad, _) = decode_pua_byte(&self.input, start);
                while self.pos < self.input.len() && (self.input[self.pos] & 0xC0) == 0x80 {
                    self.pos += 1;
                }
                self.emit_lex_diag(stray_character_message(bad), bad_start, self.pos);
                return None;
            }
        };

        Some(Token::new(
            kind,
            Span::new(start as u32, self.pos as u32, self.file_id),
        ))
    }
}

/// GCC 16.2's wording for a byte that cannot begin a token: a printable ASCII
/// character is quoted as itself, anything else by its octal value
/// (`stray '@' in program`, `stray '\1' in program`, `stray '\302' in program`
/// — the last one is the lead byte of a multi-byte character that the
/// identifier rules rejected).
fn stray_character_message(byte: u8) -> String {
    if byte.is_ascii_graphic() {
        format!("stray '{}' in program", byte as char)
    } else {
        format!("stray '\\{:o}' in program", byte)
    }
}

fn hex_digit_val(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn hexadecimal_literals_round_directly_to_their_token_type() {
        let mut lexer = Lexer::new("0x1p-1074 0x1.000001000000000000001p0f 0x3p-16495Q", 0);
        let tokens = lexer.tokenize();
        assert!(lexer.diagnostics.is_empty());
        assert!(matches!(tokens[0].kind, TokenKind::FloatLiteral(v) if v.to_bits() == 1));
        assert!(
            matches!(tokens[1].kind, TokenKind::FloatLiteralF32(v) if (v as f32).to_bits() == 0x3f800001)
        );
        assert!(
            matches!(tokens[2].kind, TokenKind::FloatLiteralF128(_, bytes) if u128::from_le_bytes(bytes) == 2)
        );
    }

    #[test]
    fn hexadecimal_long_double_uses_target_precision_but_q_stays_quad() {
        use crate::common::types::{set_target_long_double_is_f128, target_long_double_is_f128};
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                set_target_long_double_is_f128(self.0);
            }
        }
        let _restore = Restore(target_long_double_is_f128());
        let literal = format!("0x1.{}1{}1p0", "0".repeat(15), "0".repeat(40));
        for quad in [false, true] {
            set_target_long_double_is_f128(quad);
            let mut lexer = Lexer::new(&format!("{literal}L {literal}Q"), 0);
            let tokens = lexer.tokenize();
            assert!(
                matches!(tokens[0].kind, TokenKind::FloatLiteralLongDouble(_, bytes)
                if bytes == crate::common::hex_float::parse_long_double(&literal, quad))
            );
            assert!(
                matches!(tokens[1].kind, TokenKind::FloatLiteralF128(_, bytes)
                if bytes == crate::common::hex_float::parse_f128(&literal, false))
            );
        }
    }

    #[test]
    fn hexadecimal_wide_token_approximation_matches_its_rounded_carrier() {
        let literal = format!("0x1.{}8{}1p0", "0".repeat(13), "0".repeat(40));
        let mut lexer = Lexer::new(&format!("{literal}L {literal}Q"), 0);
        let tokens = lexer.tokenize();
        assert!(matches!(tokens[0].kind, TokenKind::FloatLiteralLongDouble(v, _) if v == 1.0));
        assert!(matches!(tokens[1].kind, TokenKind::FloatLiteralF128(v, _) if v == 1.0));
    }

    #[test]
    fn hexadecimal_literal_without_exponent_digits_is_diagnosed() {
        let mut lexer = Lexer::new("0x1p+;", 0);
        lexer.tokenize();
        assert_eq!(lexer.diagnostics.len(), 1);
        assert!(
            lexer.diagnostics[0]
                .message
                .contains("requires an exponent")
        );
    }

    #[test]
    fn unknown_byte_at_eof_has_exact_byte_and_span() {
        let mut lexer = Lexer::new("@", 3);
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_eof());
        assert_eq!(lexer.diagnostics.len(), 1);
        assert_eq!(lexer.diagnostics[0].message, "stray '@' in program");
        assert_eq!(lexer.diagnostics[0].span, Some(Span::new(0, 1, 3)));
    }

    #[test]
    fn unknown_byte_does_not_consume_following_punctuation() {
        let mut lexer = Lexer::new("@;", 0);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Semicolon);
        assert_eq!(tokens[0].span, Span::new(1, 2, 0));
        assert_eq!(lexer.diagnostics[0].span, Some(Span::new(0, 1, 0)));
    }

    #[test]
    fn unicode_recovery_preserves_complete_scalar_span() {
        // U+00A0 is the one UTF-8 class GCC 16.2 refuses in an identifier
        // (`stray '\302' in program`): every non-whitespace code point,
        // including `€`, is an identifier character there.
        let mut lexer = Lexer::new("\u{a0};", 0);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Semicolon);
        assert_eq!(tokens[0].span, Span::new(2, 3, 0));
        assert_eq!(lexer.diagnostics.len(), 1);
        assert_eq!(lexer.diagnostics[0].span, Some(Span::new(0, 2, 0)));
    }

    #[test]
    fn non_ascii_identifiers_follow_gcc_not_xid() {
        // Measured against GCC 16.2: `€` (Sc), U+200B (Cf), emoji and combining
        // marks are accepted in identifiers, and `caf\u00e9`, `café` and
        // `caf\U000000E9` must be the same identifier (C99 6.4.2.1).
        let mut lexer = Lexer::new(r"caf\u00e9 caf\u00e92 caf\U000000E9", 0);
        let tokens = lexer.tokenize();
        assert!(lexer.diagnostics.is_empty(), "{:?}", lexer.diagnostics);
        let names: Vec<_> = tokens
            .iter()
            .filter_map(|t| match &t.kind {
                TokenKind::Identifier(n) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["café", "café2", "café"]);

        let mut lexer = Lexer::new(r"int \u20ac = 1; int caf\u20ac = 2;", 0);
        let tokens = lexer.tokenize();
        assert!(lexer.diagnostics.is_empty(), "{:?}", lexer.diagnostics);
        for want in ["€", "caf€"] {
            assert!(
                tokens
                    .iter()
                    .any(|t| matches!(&t.kind, TokenKind::Identifier(n) if n == want)),
                "missing {want}"
            );
        }

        // A UCN below U+00A0 (or a whitespace code point) is a hard error, and
        // the token still carries the decoded name so parsing can continue.
        for (src, bad) in [
            (r"int \u0041bc = 1;", 0x41u32),
            (r"int \u0024 = 1;", 0x24),
            (r"int a\u00a0b = 1;", 0xa0),
        ] {
            let mut lexer = Lexer::new(src, 0);
            lexer.tokenize();
            assert_eq!(lexer.diagnostics.len(), 1, "{src}");
            assert!(
                lexer.diagnostics[0]
                    .message
                    .contains(&format!("\\u{:04x}", bad)),
                "{src}: {:?}",
                lexer.diagnostics[0].message
            );
        }
    }

    #[test]
    fn identifier_ucn_classes_follow_the_measured_annex_d_tables() {
        use super::super::ucn_id_table::{is_identifier_continue, is_identifier_start};
        // Accepted everywhere (Annex D.1). Each of these is outside
        // `XID_Start`, so a Unicode-property approximation would reject them.
        for cp in [
            0xA8, 0xAA, 0xE9, 0x3B1, 0x203F, 0x20AC, 0x200B, 0xFFFD, 0x1F600,
        ] {
            assert!(
                is_identifier_start(cp),
                "U+{cp:04X} must start an identifier"
            );
            assert!(
                is_identifier_continue(cp),
                "U+{cp:04X} must continue an identifier"
            );
        }
        // Accepted only inside an identifier (Annex D.2): combining marks.
        for cp in [0x301, 0x345, 0x20D0, 0xFE20] {
            assert!(
                !is_identifier_start(cp),
                "U+{cp:04X} must not start an identifier"
            );
            assert!(
                is_identifier_continue(cp),
                "U+{cp:04X} must continue an identifier"
            );
        }
        // Rejected in both positions: Latin-1 symbols, private use, plane 16,
        // and the ideographic space (all of them non-whitespace or whitespace,
        // but none of them in Annex D).
        for cp in [
            0xA0, 0xA1, 0xA9, 0x1680, 0x2028, 0x3000, 0xE000, 0xF8FF, 0x10FFFF,
        ] {
            assert!(
                !is_identifier_start(cp),
                "U+{cp:04X} must not start an identifier"
            );
            assert!(
                !is_identifier_continue(cp),
                "U+{cp:04X} must not continue an identifier"
            );
        }
        // Below U+00A0 the table is empty by construction (GCC rejects the whole
        // range in an identifier), and the table must stay sorted and disjoint.
        assert!(!is_identifier_continue(0x41));
        let cont = super::super::ucn_id_table::ID_CONTINUE;
        for pair in cont.windows(2) {
            assert!(
                pair[0].1 < pair[1].0,
                "identifier_continue table is unsorted or overlapping at {:?}",
                pair
            );
        }
    }

    #[test]
    fn stray_characters_use_gcc_wording() {
        // Measured: printable ASCII is quoted as itself, everything else in
        // octal, and a byte of a non-UTF-8 file is reported as that byte rather
        // than as the lead byte of its private-use carrier.
        for (source, want) in [
            ("int a@b;", "stray '@' in program"),
            ("int a`b;", "stray '`' in program"),
            ("int a\u{7f}b;", "stray '\\177' in program"),
            ("int a\u{a9}b;", "stray '\\302' in program"),
        ] {
            let mut lexer = Lexer::new(source, 0);
            lexer.tokenize();
            assert!(
                lexer.diagnostics.iter().any(|diag| diag.message == want),
                "{source}: {:?}",
                lexer.diagnostics
            );
        }
    }

    #[test]
    fn long_unknown_run_uses_bounded_call_stack() {
        let mut lexer = Lexer::new(&"@".repeat(100_000), 0);
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_eof());
        assert_eq!(lexer.diagnostics.len(), 100_000);
    }
}

#[cfg(test)]
mod escape_tests {
    use super::*;

    /// Narrow strings and character constants must distinguish the two escape
    /// families: a universal character name names a CHARACTER and is encoded
    /// in UTF-8, while `\xHH` and `\NNN` name a single BYTE.
    ///
    /// The rule is decided by the escape letter, not by the value's magnitude:
    /// a magnitude test gets exactly U+0080..U+00FF wrong, where `"\u00e9`
    /// must be the two bytes C3 A9 while `"\xe9` stays the one byte E9.
    /// Every expectation here is the gcc-16.2 one (measured on
    /// x86-64-linux-gnu).
    fn narrow_string(source: &str) -> Vec<u8> {
        let mut lexer = Lexer::new(source, 0);
        let tokens = lexer.tokenize();
        assert!(lexer.diagnostics.is_empty(), "{:?}", lexer.diagnostics);
        match &tokens[0].kind {
            TokenKind::StringLiteral(s) => s.chars().map(|c| c as u32 as u8).collect(),
            other => panic!("expected a narrow string literal, got {other:?}"),
        }
    }

    fn narrow_char(source: &str) -> TokenKind {
        let (kind, messages) = narrow_char_with_diagnostics(source);
        assert!(messages.is_empty(), "{source}: {messages:?}");
        kind
    }

    /// The token plus every diagnostic, for the literals GCC *does* warn about
    /// (`multi-character character constant` is `-Wmultichar`, on by default).
    fn narrow_char_with_diagnostics(source: &str) -> (TokenKind, Vec<String>) {
        let mut lexer = Lexer::new(source, 0);
        let tokens = lexer.tokenize();
        let messages = lexer
            .diagnostics
            .iter()
            .map(|diag| diag.message.clone())
            .collect();
        (tokens[0].kind.clone(), messages)
    }

    #[test]
    fn universal_character_name_is_utf8_in_a_narrow_string() {
        assert_eq!(narrow_string(r#""\u00e9""#), vec![0xC3, 0xA9]);
        assert_eq!(
            narrow_string(r#""\U0001F600""#),
            vec![0xF0, 0x9F, 0x98, 0x80]
        );
        assert_eq!(narrow_string(r#""\u0041""#), vec![0x41]);
    }

    #[test]
    fn numeric_escapes_stay_single_bytes_in_a_narrow_string() {
        assert_eq!(narrow_string(r#""\xe9""#), vec![0xE9]);
        assert_eq!(narrow_string(r#""\351""#), vec![0xE9]);
        assert_eq!(narrow_string(r#""\xff""#), vec![0xFF]);
        // Adjacent escapes must not merge into one character.
        assert_eq!(narrow_string(r#""\u00e9\xe9""#), vec![0xC3, 0xA9, 0xE9]);
    }

    #[test]
    fn universal_character_name_in_a_character_constant_is_its_utf8_bytes() {
        // GCC reads `'\u00e9'` as a multi-character constant whose value is
        // the UTF-8 bytes high-first, 0xC3A9, which it then truncates on
        // assignment; clang rejects it instead. GCC's reading is tracked.
        // …and GCC 16.2 warns about it, exactly as it does for `'ab'`:
        // `-Wmultichar` is on by default, and a character of several bytes
        // makes the constant multi-character.
        let (kind, messages) = narrow_char_with_diagnostics(r#"'\u00e9'"#);
        assert!(matches!(kind, TokenKind::IntLiteral(0xC3A9)));
        assert_eq!(messages, vec!["multi-character character constant"]);
        assert!(matches!(narrow_char(r#"'\xe9'"#), TokenKind::CharLiteral(c) if c == '\u{e9}'));
        assert!(matches!(narrow_char(r#"'\351'"#), TokenKind::CharLiteral(c) if c == '\u{e9}'));
    }

    #[test]
    fn wide_literals_keep_code_points_not_utf8() {
        let mut lexer = Lexer::new(r#"L"\u00e9" L'\u00e9'"#, 0);
        let tokens = lexer.tokenize();
        assert!(matches!(&tokens[0].kind, TokenKind::WideStringLiteral(s) if s == "\u{e9}"));
        // `wchar_t` is int here, so a wide character constant is an int
        // holding the code point (0xE9), not the UTF-8 encoding.
        assert!(matches!(tokens[1].kind, TokenKind::IntLiteral(0xE9)));
    }

    /// `\N{NAME}` (C++23, accepted in C by GCC/Clang/lccc) is a universal
    /// character name spelled with a Unicode character name, so it obeys every
    /// rule `\uXXXX` obeys. The
    /// expectations are the gcc-16.2 ones measured on x86-64-linux-gnu
    /// (`"\N{LATIN SMALL LETTER E WITH ACUTE}"` -> C3 A9, `"\N{SNOWMAN}"` ->
    /// E2 98 83, `(int)'\N{LATIN SMALL LETTER E WITH ACUTE}'` -> 0xC3A9).
    #[test]
    fn named_universal_character_escapes() {
        assert_eq!(
            narrow_string(r#""\N{LATIN SMALL LETTER E WITH ACUTE}""#),
            vec![0xC3, 0xA9]
        );
        assert_eq!(narrow_string(r#""\N{SNOWMAN}""#), vec![0xE2, 0x98, 0x83]);
        assert_eq!(narrow_string(r#""\N{DIGIT ONE}""#), vec![0x31]);
        // The algorithmic families: an explicit name, a hex-suffix family name
        // and a composed Hangul syllable, all in one literal.
        assert_eq!(
            narrow_string(r#""\N{CJK UNIFIED IDEOGRAPH-4E00}""#),
            vec![0xE4, 0xB8, 0x80]
        );
        assert_eq!(
            narrow_string(r#""\N{HANGUL SYLLABLE GA}""#),
            vec![0xEA, 0xB0, 0x80]
        );
        assert_eq!(
            narrow_string(r#""\N{GRINNING FACE}""#),
            vec![0xF0, 0x9F, 0x98, 0x80]
        );
        // A character constant reads it like `'\u00e9'`: multi-character
        // constant of the UTF-8 bytes, high byte first.
        let (kind, messages) =
            narrow_char_with_diagnostics(r#"'\N{LATIN SMALL LETTER E WITH ACUTE}'"#);
        assert!(matches!(kind, TokenKind::IntLiteral(0xC3A9)));
        assert_eq!(messages, vec!["multi-character character constant"]);
        // Wide and UTF-16/32 literals keep the code point.
        let mut lexer = Lexer::new(r#"u"\N{SNOWMAN}" U"\N{GRINNING FACE}""#, 0);
        let tokens = lexer.tokenize();
        assert!(lexer.diagnostics.is_empty(), "{:?}", lexer.diagnostics);
        assert!(matches!(&tokens[0].kind, TokenKind::Char16StringLiteral(s) if s == "\u{2603}"));
        assert!(matches!(&tokens[1].kind, TokenKind::WideStringLiteral(s) if s == "\u{1F600}"));
    }

    /// Every diagnostic below is GCC 16.2's wording, verbatim: these messages
    /// are a differential contract with the oracle.
    #[test]
    fn named_universal_character_escape_diagnostics() {
        fn messages(source: &str) -> Vec<String> {
            let mut lexer = Lexer::new(source, 0);
            let _ = lexer.tokenize();
            lexer
                .diagnostics
                .into_iter()
                .map(|diag| diag.message)
                .collect()
        }
        assert_eq!(
            messages(r#""\N{}""#),
            vec!["empty named universal character escape sequence".to_string()]
        );
        assert_eq!(
            messages(r#""\N{NOT A REAL NAME}""#),
            vec!["'\\N{NOT A REAL NAME}' is not a valid universal character".to_string()]
        );
        // Lower case is not a name: the Unicode data spells names in capitals.
        assert_eq!(
            messages(r#""\N{snowman}""#),
            vec!["'\\N{snowman}' is not a valid universal character".to_string()]
        );
        assert_eq!(
            messages(r#""\N{LATIN SMALL LETTER A""#),
            vec!["'\\N{' not terminated with '}' after \\N{LATIN SMALL LETTER A".to_string()]
        );
        assert_eq!(
            messages(r#""\N""#),
            vec![
                "'\\N' not followed by '{'".to_string(),
                "incomplete universal character name \\N".to_string()
            ]
        );
    }
    // ---------------------------------------------------------------------
    // Character-constant encoding.
    //
    // Every number and every message below is GCC 16.2's, measured on
    // x86-64-linux-gnu; `tests/regression/check_char_literal_encoding.py`
    // re-derives all of them from the oracle so these stay honest.
    // ---------------------------------------------------------------------

    #[test]
    fn narrow_character_constants_accumulate_bytes() {
        // One character is a `char`; several become an int holding the bytes,
        // high byte first, which is the value `'ab'` and `'\u00e9'` denote.
        assert!(matches!(narrow_char("'a'"), TokenKind::CharLiteral('a')));
        // (`narrow_char` insists on a silent literal, and every multi-character
        // constant warns — that is the next test's subject.)
        let multi = |source: &str| narrow_char_with_diagnostics(source).0;
        assert!(matches!(multi("'ab'"), TokenKind::IntLiteral(0x6162)));
        assert!(matches!(multi("'abcd'"), TokenKind::IntLiteral(0x61626364)));
        // Five characters keep the low four bytes: b c d e.
        assert!(matches!(
            multi("'abcde'"),
            TokenKind::IntLiteral(0x62636465)
        ));
        // A byte escape is one byte whatever the code point would suggest.
        assert!(matches!(
            narrow_char(r"'\xe9'"),
            TokenKind::CharLiteral('\u{e9}')
        ));
    }

    #[test]
    fn narrow_character_constant_diagnostics_follow_the_character_count() {
        // Two to four characters are `-Wmultichar`; five or more are the
        // unconditional "exceeds 'int' size" warning, which GCC prints with no
        // label at all.
        let (kind, messages) = narrow_char_with_diagnostics("'ab'");
        assert!(matches!(kind, TokenKind::IntLiteral(0x6162)));
        assert_eq!(messages, vec!["multi-character character constant"]);

        let (_, messages) = narrow_char_with_diagnostics("'abcde'");
        assert_eq!(
            messages,
            vec!["multi-character literal with 5 characters exceeds 'int' size of 4 bytes"]
        );

        // `''` is an error, not a zero constant.
        let (_, messages) = narrow_char_with_diagnostics("''");
        assert_eq!(messages, vec!["empty character constant"]);

        // The escape-range warning is emitted before the character-count one.
        let (_, messages) = narrow_char_with_diagnostics(r"'a\x100'");
        assert_eq!(
            messages,
            vec![
                "hex escape sequence out of range",
                "multi-character character constant"
            ]
        );
    }

    #[test]
    fn unknown_escapes_warn_with_gcc_wording() {
        // GCC keeps the character and warns with no `-W` name, so `-w` and
        // `-Werror` are the only switches that act on it. Three letters are
        // silently accepted (`\\%`, `\\[`, `\\{`), and a non-graphical byte is
        // spelled in octal — a space is `'\\040'`, and the first byte of `é`
        // is `'\\303'`, not the character.
        for source in [r"'\q'", r"'\8'", r"'\9'", r"'\-'", r"'\@'", r"'\A'"] {
            let (_, messages) = narrow_char_with_diagnostics(source);
            assert_eq!(
                messages,
                vec![format!("unknown escape sequence: '{}'", &source[1..3])],
                "{source}"
            );
        }
        // The value is the character itself, one character wide.
        assert!(matches!(
            narrow_char_with_diagnostics(r"'\q'").0,
            TokenKind::CharLiteral('q')
        ));
        assert!(matches!(
            narrow_char_with_diagnostics(r"'\8'").0,
            TokenKind::CharLiteral('8')
        ));
        for source in [r"'\%'", r"'\['", r"'\{'", r"'\?'"] {
            let (_, messages) = narrow_char_with_diagnostics(source);
            assert!(messages.is_empty(), "{source}: {messages:?}");
        }
        let (_, messages) = narrow_char_with_diagnostics(r"'\ '");
        assert_eq!(messages, vec!["unknown escape sequence: '\\040'"]);
        let (_, messages) = narrow_char_with_diagnostics("'\\é'");
        assert_eq!(
            messages,
            vec![
                "unknown escape sequence: '\\303'",
                "multi-character character constant"
            ]
        );
        // Strings share the escape reader, so they share the warning.
        let mut lexer = Lexer::new("\"\\q\"", 0);
        lexer.tokenize();
        assert_eq!(
            lexer.diagnostics[0].message,
            "unknown escape sequence: '\\q'"
        );
    }

    #[test]
    fn rejected_escapes_contribute_no_character() {
        // Measured on gcc-16.2: `'\xg'` is a ONE-character constant, so it
        // warns about neither a multi-character constant nor the 'int' size,
        // and it does not collect an `empty character constant` on top of its
        // own error. The `}` of `'\x{zz}'` is an ordinary character and counts.
        let (kind, messages) = narrow_char_with_diagnostics(r"'\xg'");
        assert!(matches!(kind, TokenKind::CharLiteral('g')));
        assert_eq!(messages, vec!["'\\x' used with no following hex digits"]);

        let (_, messages) = narrow_char_with_diagnostics(r"'\xgabde'");
        assert_eq!(
            messages,
            vec![
                "'\\x' used with no following hex digits",
                "multi-character literal with 5 characters exceeds 'int' size of 4 bytes"
            ]
        );

        let (_, messages) = narrow_char_with_diagnostics(r"'\x{zzzzz}'");
        assert_eq!(
            messages,
            vec![
                "'\\x' used with no following hex digits",
                "multi-character literal with 6 characters exceeds 'int' size of 4 bytes"
            ]
        );

        // An unterminated name consumes the literal and says nothing further.
        let (_, messages) = narrow_char_with_diagnostics(r"'\N{zzzzzzzz'");
        assert_eq!(
            messages,
            vec!["'\\N{' not terminated with '}' after \\N{zzzzzzzz"]
        );

        // An invalid name is two characters in a narrow literal and one in a
        // wide one, which is exactly what GCC reports.
        let (_, messages) = narrow_char_with_diagnostics(r"'\N{NO SUCH NAME}zzzz'");
        assert_eq!(
            messages,
            vec![
                "'\\N{NO SUCH NAME}' is not a valid universal character",
                "multi-character literal with 6 characters exceeds 'int' size of 4 bytes"
            ]
        );
        assert_eq!(
            narrow_wide_messages(r"u'\N{NO SUCH NAME}'"),
            vec!["'\\N{NO SUCH NAME}' is not a valid universal character"]
        );
        assert_eq!(
            narrow_wide_messages(r"u8'\N{NO SUCH NAME}'"),
            vec![
                "'\\N{NO SUCH NAME}' is not a valid universal character",
                "character not encodable in a single code unit"
            ]
        );
    }

    #[test]
    fn named_escape_without_braces_reads_four_hex_digits() {
        // GCC reads `\N` without its `{NAME}` exactly like `\u`: four hex
        // digits, one character when it cannot complete them, and the UTF-8
        // bytes of the code point when it can (`'\Nabcd'` is U+ABCD).
        // U+ABCD is the three UTF-8 bytes EA AF 8D, so the constant is a
        // three-character one and carries the `-Wmultichar` warning with it.
        let (kind, messages) = narrow_char_with_diagnostics(r"'\Nabcd'");
        assert!(matches!(kind, TokenKind::IntLiteral(0xEAAF8D)));
        assert_eq!(
            messages,
            vec![
                "'\\N' not followed by '{'",
                "multi-character character constant"
            ]
        );
        let (_, messages) = narrow_char_with_diagnostics(r"'\Na'");
        assert_eq!(
            messages,
            vec![
                "'\\N' not followed by '{'",
                "incomplete universal character name \\Na"
            ]
        );
        let (_, messages) = narrow_char_with_diagnostics(r"'\Nabczzzzzzz'");
        assert_eq!(
            messages,
            vec![
                "'\\N' not followed by '{'",
                "incomplete universal character name \\Nabc",
                "multi-character literal with 8 characters exceeds 'int' size of 4 bytes"
            ]
        );
    }

    #[test]
    fn utf16_conversion_failure_is_reported_at_the_escape() {
        // The escape above U+10FFFF contributes no character, so
        // `u'\U00110000a'` is a one-character constant with no prefix warning,
        // while three surviving characters keep theirs. The error is emitted in
        // scan order, once per offending escape.
        assert_eq!(
            narrow_wide_messages(r"u'\U00110000a'"),
            vec![
                "\\U00110000 is outside the UCS codespace",
                "converting UCN to execution character set: Invalid or incomplete multibyte or wide character"
            ]
        );
        assert_eq!(
            narrow_wide_messages(r"u'\n\x1FF\U00110000Z'"),
            vec![
                "\\U00110000 is outside the UCS codespace",
                "converting UCN to execution character set: Invalid or incomplete multibyte or wide character",
                "multi-character literal cannot have an encoding prefix"
            ]
        );
        assert_eq!(
            narrow_wide_messages(r"u'\U00110000\U00110000'"),
            vec![
                "\\U00110000 is outside the UCS codespace",
                "converting UCN to execution character set: Invalid or incomplete multibyte or wide character",
                "\\U00110000 is outside the UCS codespace",
                "converting UCN to execution character set: Invalid or incomplete multibyte or wide character"
            ]
        );
    }

    /// Every diagnostic message of a wide character constant, in order.
    fn narrow_wide_messages(source: &str) -> Vec<String> {
        let mut lexer = Lexer::new(source, 0);
        lexer.tokenize();
        lexer
            .diagnostics
            .iter()
            .map(|diag| diag.message.clone())
            .collect()
    }

    #[test]
    fn universal_character_names_above_the_codespace_keep_their_bytes() {
        // A narrow constant encodes a character in generalized UTF-8, the
        // traditional 4/5/6-byte forms included, so the value survives.
        // Values are the low four bytes of the byte string GCC builds, so they
        // read as negative `int`s here.
        let wide = |source: &str| narrow_char_with_diagnostics(source).0;
        assert!(matches!(
            wide(r"'\U00110000'"),
            TokenKind::IntLiteral(value) if value == 0xF4_90_80_80u32 as i32 as i64
        ));
        assert!(matches!(
            wide(r"'\U7FFFFFFF'"),
            // Six bytes (FD BF BF BF BF BF); the constant keeps the low four.
            TokenKind::IntLiteral(value) if value == 0xBF_BF_BF_BFu32 as i32 as i64
        ));
        // …and a surrogate is never a character: error, and one character
        // counted, so `'\uD800a'` is still a two-character constant.
        let (_, messages) = narrow_char_with_diagnostics(r"'\uD800a'");
        assert_eq!(
            messages,
            vec![
                r"\uD800 is not a valid universal character",
                "multi-character character constant"
            ]
        );
    }

    #[test]
    fn utf16_constants_keep_the_low_surrogate() {
        // 0xDE00 is the second code unit of the pair U+1F600 encodes as,
        // not the low 16 bits of the code point (0xF600).
        let mut lexer = Lexer::new(r#"u'\U0001F600'"#, 0);
        let tokens = lexer.tokenize();
        assert!(matches!(
            tokens[0].kind,
            TokenKind::UnicodeCharLiteral(0xDE00, UnicodeCharKind::Utf16)
        ));
        let messages: Vec<_> = lexer
            .diagnostics
            .iter()
            .map(|diag| diag.message.clone())
            .collect();
        assert_eq!(
            messages,
            vec!["character not encodable in a single code unit"]
        );
        // The edges of the range are the surrogate range's edges.
        for (source, expected) in [(r"u'\U00010000'", 0xDC00), (r"u'\U0010FFFF'", 0xDFFF)] {
            let mut lexer = Lexer::new(source, 0);
            let tokens = lexer.tokenize();
            assert!(
                matches!(tokens[0].kind, TokenKind::UnicodeCharLiteral(value, _) if value == expected),
                "{source}"
            );
        }
    }

    #[test]
    fn utf8_constants_reject_characters_that_need_two_code_units() {
        // `u8'\u00e9'` is an error; the byte escape `u8'\xe9'` is not.
        let mut lexer = Lexer::new(r"u8'\u00e9'", 0);
        let tokens = lexer.tokenize();
        assert!(matches!(
            tokens[0].kind,
            TokenKind::UnicodeCharLiteral(0xE9, UnicodeCharKind::Utf8)
        ));
        assert_eq!(
            lexer.diagnostics[0].message,
            "character not encodable in a single code unit"
        );
        assert_eq!(
            lexer.diagnostics[0].severity,
            crate::common::error::Severity::Error
        );
        // The byte escape stays a byte: no diagnostic at all.
        assert!(matches!(
            narrow_char("u8'\\xe9'"),
            TokenKind::UnicodeCharLiteral(0xE9, UnicodeCharKind::Utf8)
        ));
    }

    #[test]
    fn wide_multi_character_literals_keep_the_last_character() {
        // The narrow `'ab'` accumulates; every wide form keeps the last
        // character and warns, because an encoding prefix cannot be combined
        // with a multi-character literal.
        for (source, expected) in [
            ("u'ab'", 0x62u32),
            ("U'ab'", 0x62),
            ("L'ab'", 0x62),
            (r"u'a\U0001F600'", 0xDE00),
        ] {
            let mut lexer = Lexer::new(source, 0);
            let tokens = lexer.tokenize();
            let value = match tokens[0].kind {
                TokenKind::UnicodeCharLiteral(value, _) => value,
                TokenKind::IntLiteral(value) => value as u32,
                ref other => panic!("{source}: {other:?}"),
            };
            assert_eq!(value, expected, "{source}");
            assert!(
                lexer
                    .diagnostics
                    .iter()
                    .any(|diag| diag.message
                        == "multi-character literal cannot have an encoding prefix"),
                "{source}: {:?}",
                lexer.diagnostics
            );
        }
        // `u8` refuses the literal outright.
        let mut lexer = Lexer::new("u8'ab'", 0);
        lexer.tokenize();
        assert_eq!(
            lexer.diagnostics[0].message,
            "multi-character literal cannot have an encoding prefix"
        );
        assert_eq!(
            lexer.diagnostics[0].severity,
            crate::common::error::Severity::Error
        );
    }

    #[test]
    fn escape_range_is_judged_against_the_literal_width() {
        // The same escape is out of range for a byte-sized element and clean
        // for a wider one.
        let cases = [
            (r"'\x100'", Some("hex escape sequence out of range")),
            (r"u8'\x100'", Some("hex escape sequence out of range")),
            (r"u'\x100'", None),
            (r"L'\x100'", None),
            (r"u'\x10000'", Some("hex escape sequence out of range")),
            (r"L'\x100000000'", Some("hex escape sequence out of range")),
            (r"'\400'", Some("octal escape sequence out of range")),
            (r"u'\400'", None),
        ];
        for (source, expected) in cases {
            let mut lexer = Lexer::new(source, 0);
            lexer.tokenize();
            let warnings: Vec<_> = lexer
                .diagnostics
                .iter()
                .filter(|diag| diag.message.contains("escape sequence out of range"))
                .map(|diag| diag.message.clone())
                .collect();
            match expected {
                Some(message) => assert_eq!(warnings, vec![message.to_string()], "{source}"),
                None => assert!(warnings.is_empty(), "{source}: {warnings:?}"),
            }
        }
    }

    #[test]
    fn multicharacter_warning_carries_the_multichar_category() {
        // `-Wmultichar` is a real option, so the warning must name its
        // category; the "exceeds 'int' size" variant names none, because GCC
        // does not let `-Wno-multichar` silence it.
        let mut lexer = Lexer::new("'ab' 'abcde'", 0);
        lexer.tokenize();
        assert_eq!(
            lexer.diagnostics[0].warning_kind,
            Some(crate::common::error::WarningKind::Multichar)
        );
        assert_eq!(lexer.diagnostics[1].warning_kind, None);
    }
}
