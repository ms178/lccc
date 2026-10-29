use super::token::{Token, TokenKind, UnicodeCharKind};
use crate::common::encoding::decode_pua_byte;
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
    pub diagnostics: Vec<(String, crate::common::source::Span)>,
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
        self.diagnostics.push((
            msg.into(),
            Span::new(start as u32, end as u32, self.file_id),
        ));
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
        for (msg, _) in std::mem::take(&mut sub.diagnostics) {
            self.emit_lex_diag(msg, start, end);
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
                    let ch = self.lex_escape_char();
                    // C narrow strings store raw bytes, so Unicode escapes (\u, \U)
                    // must be UTF-8 encoded to match GCC/Clang behavior.
                    if (ch as u32) > 0xFF {
                        let mut buf = [0u8; 4];
                        let utf8 = ch.encode_utf8(&mut buf);
                        for byte in utf8.bytes() {
                            s.push(byte as char);
                        }
                    } else {
                        s.push(ch);
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
                    // Wide strings store code points directly (no UTF-8 encoding needed)
                    s.push(self.lex_escape_char());
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
                    s.push(self.lex_escape_char());
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

    fn lex_wide_char(&mut self, start: usize, kind: Option<UnicodeCharKind>) -> Token {
        self.pos += 1; // skip opening '
        let mut value: u32 = 0;
        if self.pos < self.input.len() && self.input[self.pos] != b'\'' {
            if self.input[self.pos] == b'\\' {
                self.pos += 1;
                let ch = self.lex_escape_char();
                value = ch as u32; // Unicode escapes return code point directly
            } else {
                // Check for PUA-encoded bytes first
                let (byte, consumed) = decode_pua_byte(&self.input, self.pos);
                if consumed > 1 {
                    value = byte as u32;
                    self.pos += consumed;
                } else {
                    // Decode UTF-8 to get Unicode code point
                    let byte = self.input[self.pos];
                    if byte < 0x80 {
                        value = byte as u32;
                        self.pos += 1;
                    } else {
                        let remaining = &self.input[self.pos..];
                        let end = std::cmp::min(remaining.len(), 4);
                        if let Ok(text) = std::str::from_utf8(&remaining[..end]) {
                            if let Some(ch) = text.chars().next() {
                                value = ch as u32;
                                self.pos += ch.len_utf8();
                            }
                        } else if let Ok(text) = std::str::from_utf8(remaining) {
                            if let Some(ch) = text.chars().next() {
                                value = ch as u32;
                                self.pos += ch.len_utf8();
                            }
                        } else {
                            value = byte as u32;
                            self.pos += 1;
                        }
                    }
                }
            }
        }
        // Skip any remaining chars until closing quote
        while self.pos < self.input.len() && self.input[self.pos] != b'\'' {
            self.pos += 1;
        }
        if self.pos < self.input.len() && self.input[self.pos] == b'\'' {
            self.pos += 1; // skip closing '
        }
        let span = Span::new(start as u32, self.pos as u32, self.file_id);
        match kind {
            // The code unit the constant denotes, in its unsigned type.
            Some(k @ UnicodeCharKind::Utf8) => {
                Token::new(TokenKind::UnicodeCharLiteral(value & 0xff, k), span)
            }
            Some(k @ UnicodeCharKind::Utf16) => {
                Token::new(TokenKind::UnicodeCharLiteral(value & 0xffff, k), span)
            }
            Some(k @ UnicodeCharKind::Utf32) => {
                Token::new(TokenKind::UnicodeCharLiteral(value, k), span)
            }
            // L'x' has type wchar_t = int.
            None => Token::new(TokenKind::IntLiteral(value as i64), span),
        }
    }

    fn lex_char(&mut self, start: usize) -> Token {
        self.pos += 1; // skip opening '
        let mut value: i32 = 0;
        let mut char_count = 0;
        while self.pos < self.input.len() && self.input[self.pos] != b'\'' {
            let ch = if self.input[self.pos] == b'\\' {
                self.pos += 1;
                self.lex_escape_char()
            } else {
                // Decode PUA-encoded bytes for non-UTF-8 source files
                let (byte, consumed) = decode_pua_byte(&self.input, self.pos);
                self.pos += consumed;
                byte as char
            };
            // C narrow char literals encode Unicode escapes as UTF-8 bytes
            // combined into a multi-byte int value, matching GCC behavior.
            if (ch as u32) > 0xFF {
                let mut buf = [0u8; 4];
                let utf8 = ch.encode_utf8(&mut buf);
                for byte in utf8.bytes() {
                    value = (value << 8) | (byte as i32);
                    char_count += 1;
                }
            } else {
                // Multi-character constant: shift previous value and add new byte
                value = (value << 8) | (ch as u8 as i32);
                char_count += 1;
            }
        }
        if self.pos < self.input.len() && self.input[self.pos] == b'\'' {
            self.pos += 1; // skip closing '
        }
        let span = Span::new(start as u32, self.pos as u32, self.file_id);
        if char_count <= 1 {
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

    fn lex_escape_char(&mut self) -> char {
        if self.pos >= self.input.len() {
            return '\0';
        }
        let ch = self.input[self.pos];
        self.pos += 1;
        match ch {
            b'n' => '\n',
            b't' => '\t',
            b'r' => '\r',
            b'\\' => '\\',
            b'\'' => '\'',
            b'"' => '"',
            b'a' => '\x07',
            b'b' => '\x08',
            b'e' | b'E' => '\x1b', // GNU extension: ESC
            b'f' => '\x0c',
            b'v' => '\x0b',
            b'o' if self.pos < self.input.len() && self.input[self.pos] == b'{' => {
                // C2y / GCC 15+ delimited octal escape: \o{...}
                let esc_start = self.pos.saturating_sub(2);
                self.pos += 1; // consume '{'
                let mut val = 0u32;
                let mut got = 0usize;
                while self.pos < self.input.len() && self.input[self.pos] != b'}' {
                    let b = self.input[self.pos];
                    if (b'0'..=b'7').contains(&b) {
                        val = val.saturating_mul(8).saturating_add((b - b'0') as u32);
                        got += 1;
                    } else {
                        self.emit_lex_diag(
                            "invalid digit in delimited octal escape sequence",
                            esc_start,
                            self.pos + 1,
                        );
                    }
                    self.pos += 1;
                }
                if self.pos < self.input.len() && self.input[self.pos] == b'}' {
                    self.pos += 1;
                } else {
                    self.emit_lex_diag(
                        "unterminated delimited octal escape sequence",
                        esc_start,
                        self.pos,
                    );
                }
                if got == 0 {
                    self.emit_lex_diag(
                        "empty delimited octal escape sequence",
                        esc_start,
                        self.pos,
                    );
                }
                (val as u8) as char
            }
            b'x' => {
                if self.pos < self.input.len() && self.input[self.pos] == b'{' {
                    // C2y / GCC 15+ delimited hex escape: \x{...}
                    let esc_start = self.pos.saturating_sub(2);
                    self.pos += 1; // consume '{'
                    let mut val = 0u32;
                    let mut got = 0usize;
                    while self.pos < self.input.len() && self.input[self.pos] != b'}' {
                        let b = self.input[self.pos];
                        if b.is_ascii_hexdigit() {
                            val = val
                                .saturating_mul(16)
                                .saturating_add(hex_digit_val(b) as u32);
                            got += 1;
                        } else {
                            self.emit_lex_diag(
                                "invalid digit in delimited hex escape sequence",
                                esc_start,
                                self.pos + 1,
                            );
                        }
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() && self.input[self.pos] == b'}' {
                        self.pos += 1;
                    } else {
                        self.emit_lex_diag(
                            "unterminated delimited hex escape sequence",
                            esc_start,
                            self.pos,
                        );
                    }
                    if got == 0 {
                        self.emit_lex_diag(
                            "empty delimited hex escape sequence",
                            esc_start,
                            self.pos,
                        );
                    }
                    (val as u8) as char
                } else {
                    // Hex escape: \xNN - consumes all hex digits, value truncated to byte
                    let mut val = 0u32;
                    while self.pos < self.input.len() && self.input[self.pos].is_ascii_hexdigit() {
                        val = val * 16 + hex_digit_val(self.input[self.pos]) as u32;
                        self.pos += 1;
                    }
                    // Truncate to byte value and use direct char mapping to avoid
                    // multi-byte UTF-8 encoding issues with values > 127
                    (val as u8) as char
                }
            }
            b'u' => {
                if self.pos < self.input.len() && self.input[self.pos] == b'{' {
                    // C2y / GCC 15+ delimited universal character name: \u{...}
                    let esc_start = self.pos.saturating_sub(2);
                    self.pos += 1; // consume '{'
                    let mut val = 0u32;
                    let mut got = 0usize;
                    while self.pos < self.input.len() && self.input[self.pos] != b'}' {
                        let b = self.input[self.pos];
                        if b.is_ascii_hexdigit() {
                            val = val
                                .saturating_mul(16)
                                .saturating_add(hex_digit_val(b) as u32);
                            got += 1;
                        } else {
                            self.emit_lex_diag(
                                "invalid digit in delimited universal character name",
                                esc_start,
                                self.pos + 1,
                            );
                        }
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() && self.input[self.pos] == b'}' {
                        self.pos += 1;
                    } else {
                        self.emit_lex_diag(
                            "unterminated delimited universal character name",
                            esc_start,
                            self.pos,
                        );
                    }
                    if got == 0 {
                        self.emit_lex_diag(
                            "empty delimited universal character name",
                            esc_start,
                            self.pos,
                        );
                    }
                    if (0xD800..=0xDFFF).contains(&val) || val > 0x10FFFF {
                        self.emit_lex_diag(
                            format!("invalid universal character name U+{:X}", val),
                            esc_start,
                            self.pos,
                        );
                        '\u{FFFD}'
                    } else {
                        char::from_u32(val).unwrap_or('\u{FFFD}')
                    }
                } else {
                    // Universal character name: \uNNNN (exactly 4 hex digits)
                    self.lex_unicode_escape(4)
                }
            }
            b'U' => {
                // Universal character name: \UNNNNNNNN (exactly 8 hex digits)
                self.lex_unicode_escape(8)
            }
            b'0'..=b'7' => {
                // Octal escape: \0 through \377 (1-3 octal digits)
                // Note: \0 alone produces null; \040 produces space (32), etc.
                let mut val = (ch - b'0') as u32;
                for _ in 0..2 {
                    if self.pos < self.input.len()
                        && self.input[self.pos] >= b'0'
                        && self.input[self.pos] <= b'7'
                    {
                        val = val * 8 + (self.input[self.pos] - b'0') as u32;
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                // Truncate to byte value to match C semantics
                (val as u8) as char
            }
            _ => ch as char,
        }
    }

    /// Parse a universal character name (\uNNNN or \UNNNNNNNN).
    /// `num_digits` is 4 for \u or 8 for \U.
    /// Returns the Unicode code point as a Rust char.
    // TODO: C11 requires exactly num_digits hex digits; emit diagnostic if fewer provided.
    // TODO: C11 §6.4.3 disallows certain code points (below 0x00A0 except 0x24/0x40/0x60,
    //       and surrogates 0xD800-0xDFFF). Validate and emit diagnostics for these.
    fn lex_unicode_escape(&mut self, num_digits: usize) -> char {
        let esc_start = self.pos.saturating_sub(2);
        let mut val = 0u32;
        let mut got = 0usize;
        for _ in 0..num_digits {
            if self.pos < self.input.len() && self.input[self.pos].is_ascii_hexdigit() {
                val = val * 16 + hex_digit_val(self.input[self.pos]) as u32;
                self.pos += 1;
                got += 1;
            } else {
                break;
            }
        }
        if got < num_digits {
            self.emit_lex_diag(
                format!(
                    "universal character name expects {} hex digits, got {}",
                    num_digits, got
                ),
                esc_start,
                self.pos,
            );
        }
        if (0xD800..=0xDFFF).contains(&val) || val > 0x10FFFF {
            self.emit_lex_diag(
                format!("invalid universal character name U+{:X}", val),
                esc_start,
                self.pos,
            );
            return '\u{FFFD}';
        }
        char::from_u32(val).unwrap_or('\u{FFFD}')
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
                let allowed = if valid_char {
                    if at_start {
                        is_identifier_start(value)
                    } else {
                        is_identifier_continue(value)
                    }
                } else {
                    false
                };
                if !allowed {
                    let message = if !valid_char {
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
        assert!(lexer.diagnostics[0].0.contains("requires an exponent"));
    }

    #[test]
    fn unknown_byte_at_eof_has_exact_byte_and_span() {
        let mut lexer = Lexer::new("@", 3);
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_eof());
        assert_eq!(lexer.diagnostics.len(), 1);
        assert_eq!(lexer.diagnostics[0].0, "stray '@' in program");
        assert_eq!(lexer.diagnostics[0].1, Span::new(0, 1, 3));
    }

    #[test]
    fn unknown_byte_does_not_consume_following_punctuation() {
        let mut lexer = Lexer::new("@;", 0);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Semicolon);
        assert_eq!(tokens[0].span, Span::new(1, 2, 0));
        assert_eq!(lexer.diagnostics[0].1, Span::new(0, 1, 0));
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
        assert_eq!(lexer.diagnostics[0].1, Span::new(0, 2, 0));
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
                lexer.diagnostics[0].0.contains(&format!("\\u{:04x}", bad)),
                "{src}: {:?}",
                lexer.diagnostics[0].0
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
                lexer.diagnostics.iter().any(|(m, _)| m == want),
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
