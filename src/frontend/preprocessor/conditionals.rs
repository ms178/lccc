//! Conditional compilation tracking for the C preprocessor.
//!
//! Handles #if, #ifdef, #ifndef, #elif, #else, #endif directives
//! by maintaining a stack of conditional states.
//!
//! Performance: All scanning operates on byte slices (`&[u8]`) to avoid
//! the overhead of `Vec<char>` allocation. Since C preprocessor expressions
//! are ASCII, this is safe and efficient. Identifiers and numbers are parsed
//! directly from byte spans using `bytes_to_str()`, eliminating intermediate
//! String allocations.

use super::macro_defs::MacroTable;
use super::utils::{
    bytes_to_str, is_digit_separator, is_ident_cont_byte, is_ident_start_byte, is_literal_prefix,
    ppnumber_end, skip_literal_bytes,
};
use crate::common::types::CType;
use crate::frontend::lexer::Lexer;
use crate::frontend::lexer::token::{TokenKind, UnicodeCharKind};

/// State of a single conditional (#if/#ifdef/#ifndef block).
#[derive(Debug, Clone, Copy)]
struct ConditionalState {
    /// Whether any branch in this if/elif/else chain has been true
    any_branch_taken: bool,
    /// Whether the current branch is active (code should be emitted)
    current_branch_active: bool,
    /// Whether the parent context is active
    parent_active: bool,
}

/// Tracks the stack of nested conditionals.
#[derive(Debug)]
pub struct ConditionalStack {
    stack: Vec<ConditionalState>,
}

impl ConditionalStack {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    /// Returns true if code should currently be emitted.
    pub fn is_active(&self) -> bool {
        self.stack
            .last()
            .is_none_or(|s| s.current_branch_active && s.parent_active)
    }

    /// Push a new #if/#ifdef/#ifndef.
    pub fn push_if(&mut self, condition: bool) {
        let parent_active = self.is_active();
        let active = parent_active && condition;
        self.stack.push(ConditionalState {
            any_branch_taken: condition,
            current_branch_active: active,
            parent_active,
        });
    }

    /// An elif expression is evaluated only in an active parent with no
    /// previously selected branch. Skipped expressions must not diagnose.
    pub fn needs_elif_evaluation(&self) -> bool {
        self.stack
            .last()
            .is_some_and(|s| s.parent_active && !s.any_branch_taken)
    }

    /// Handle #elif.
    pub fn handle_elif(&mut self, condition: bool) {
        if let Some(state) = self.stack.last_mut() {
            if state.any_branch_taken {
                // A previous branch was taken, skip this one
                state.current_branch_active = false;
            } else if state.parent_active && condition {
                state.current_branch_active = true;
                state.any_branch_taken = true;
            } else {
                state.current_branch_active = false;
            }
        }
    }

    /// Handle #else.
    pub fn handle_else(&mut self) {
        if let Some(state) = self.stack.last_mut() {
            if state.any_branch_taken {
                state.current_branch_active = false;
            } else {
                state.current_branch_active = state.parent_active;
                state.any_branch_taken = true;
            }
        }
    }

    /// Handle #endif.
    pub fn handle_endif(&mut self) {
        self.stack.pop();
    }
}

impl Default for ConditionalStack {
    fn default() -> Self {
        Self::new()
    }
}

/// Evaluate a preprocessor #if expression.
/// This handles basic constant expressions, defined() operator, and macro expansion.
pub fn evaluate_condition(expr: &str, macros: &MacroTable) -> (bool, Vec<String>) {
    // First expand macros in the expression (except inside defined())
    let expanded = expand_condition_macros(expr, macros);
    // Then evaluate the constant expression
    let tokens = tokenize_expr(&expanded, CharModel::current());
    let mut parser = ExprParser::new(&tokens);
    let result = parser.parse_ternary().0 != 0;
    (result, parser.undefined)
}

/// Expand macros in a condition expression, handling `defined(X)` and `defined X` specially.
/// Character and string literals are preserved verbatim (not subject to macro expansion).
///
/// Operates on byte slices for performance: avoids allocating Vec<char>.
fn expand_condition_macros(expr: &str, macros: &MacroTable) -> String {
    let mut result = String::with_capacity(expr.len());
    let bytes = expr.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Skip character literals verbatim (don't expand macros inside)
        if b == b'\'' && !is_digit_separator(bytes, i) {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'\'' {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < len {
                i += 1; // closing quote
            }
            result.push_str(bytes_to_str(bytes, start, i));
            continue;
        }

        // Skip string literals verbatim
        if b == b'"' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < len {
                i += 1; // closing quote
            }
            result.push_str(bytes_to_str(bytes, start, i));
            continue;
        }

        if i + 7 <= len && &bytes[i..i + 7] == b"defined" {
            // Check it's not part of a larger identifier
            let before_ok = i == 0 || !is_ident_cont_byte(bytes[i - 1]);
            let after_ok = i + 7 >= len || !is_ident_cont_byte(bytes[i + 7]);
            if before_ok && after_ok {
                let start = i;
                i += 7;

                // Skip whitespace
                while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                    i += 1;
                }

                if i < len && bytes[i] == b'(' {
                    // defined(MACRO) -- copy through closing paren
                    i += 1;
                    while i < len && bytes[i] != b')' {
                        i += 1;
                    }
                    if i < len {
                        i += 1; // closing paren
                    }
                } else if i < len && is_ident_start_byte(bytes[i]) {
                    // defined MACRO
                    while i < len && is_ident_cont_byte(bytes[i]) {
                        i += 1;
                    }
                }
                result.push_str(bytes_to_str(bytes, start, i));
                continue;
            }
        }

        // Skip pp-number tokens: a digit (or .digit) followed by alphanumeric chars,
        // dots, underscores, and exponent signs (e+, e-, p+, p-).
        // This prevents macro expansion of suffixes like 'u' in '1u', 'L' in '1L', etc.
        if b.is_ascii_digit() || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            while i < len {
                if bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'.'
                    || bytes[i] == b'_'
                    || ((bytes[i] == b'+' || bytes[i] == b'-')
                        && i > 0
                        && matches!(bytes[i - 1], b'e' | b'E' | b'p' | b'P'))
                {
                    i += 1;
                } else {
                    break;
                }
            }
            result.push_str(bytes_to_str(bytes, start, i));
            continue;
        }

        if is_ident_start_byte(b) {
            let start = i;
            i += 1;
            while i < len && is_ident_cont_byte(bytes[i]) {
                i += 1;
            }
            let ident = bytes_to_str(bytes, start, i);

            // An encoding prefix is part of the literal after it, never a
            // macro: `#define L 0` leaves `L'a'` alone.
            if is_literal_prefix(ident, bytes.get(i).copied()) {
                result.push_str(ident);
                continue;
            }

            // Check if this identifier is a pp-number suffix (e.g., 1u, 0xFFULL, 1.0f).
            // If the previous character in result is a digit, this is part of a number
            // token and should NOT be expanded as a macro.
            let is_ppnum_suffix = {
                let rb = result.as_bytes();
                !rb.is_empty() && rb[rb.len() - 1].is_ascii_alphanumeric()
            };
            if is_ppnum_suffix {
                result.push_str(ident);
                continue;
            }

            // Expand macro if it exists
            if let Some(mac) = macros.get(ident) {
                if !mac.is_function_like {
                    result.push_str(&mac.body);
                } else {
                    result.push_str(ident);
                }
            } else {
                // Undefined identifiers become 0 in #if expressions
                result.push_str(ident);
            }
            continue;
        }

        // Any other character, copied whole: `i` is always on a character
        // boundary, and a non-ASCII character is several bytes (pushing
        // `b as char` would turn each byte into its own Latin-1 character).
        let ch = expr[i..].chars().next().unwrap_or(b as char);
        result.push(ch);
        i += ch.len_utf8();
    }

    result
}

#[cfg(test)]
/// Evaluate a constant expression for #if directives.
/// Supports: integer literals, defined(X), !, &&, ||, ==, !=, <, >, <=, >=, +, -, *, /, %, (), unary -, ~
/// Per C99 6.10.1, preprocessor integer expressions use intmax_t (signed) or uintmax_t (unsigned)
/// depending on whether any operand has a 'u'/'U' suffix.
pub fn eval_const_expr(expr: &str) -> bool {
    eval_const_expr_with(expr, CharModel::current())
}

/// `eval_const_expr` for an explicit character-type model (the unit tests
/// cover every target's model without touching the process-wide settings).
fn eval_const_expr_with(expr: &str, model: CharModel) -> bool {
    let expr = expr.trim();
    if expr.is_empty() {
        return false;
    }

    let tokens = tokenize_expr(expr, model);
    let mut parser = ExprParser::new(&tokens);
    let (result, _is_unsigned) = parser.parse_comma();
    result != 0
}

/// Simple token for expression evaluation.
/// Num holds (value_as_i64, is_unsigned). The i64 stores the bit pattern;
/// for unsigned values, interpret as u64 via `as u64`.
#[derive(Debug, Clone, PartialEq)]
enum ExprToken {
    Num(i64, bool),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Defined,
}

/// Tokenize a preprocessor expression, operating on byte slices for performance.
/// Scan a pp-number in `#if` / `#elif` and return `(token, next_index)`.
fn scan_pp_number_token(bytes: &[u8], start: usize) -> (ExprToken, usize) {
    let end = ppnumber_end(bytes, start);
    let spelling: String = bytes_to_str(bytes, start, end)
        .chars()
        .filter(|&c| c != '\'')
        .collect();
    let digits_end = spelling
        .find(|c: char| matches!(c, 'u' | 'U' | 'l' | 'L' | 'w' | 'W'))
        .unwrap_or(spelling.len());
    let (digits, suffix) = spelling.split_at(digits_end);
    let (raw_val, is_hex_or_oct) = if let Some(h) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        (u64::from_str_radix(h, 16).unwrap_or(0), true)
    } else if let Some(bin) = digits
        .strip_prefix("0b")
        .or_else(|| digits.strip_prefix("0B"))
    {
        (u64::from_str_radix(bin, 2).unwrap_or(0), true)
    } else if let Some(oct) = digits
        .strip_prefix("0o")
        .or_else(|| digits.strip_prefix("0O"))
    {
        (u64::from_str_radix(oct, 8).unwrap_or(0), true)
    } else if digits.len() > 1 && digits.starts_with('0') {
        (u64::from_str_radix(&digits[1..], 8).unwrap_or(0), true)
    } else {
        (digits.parse::<u64>().unwrap_or(0), false)
    };
    let mut is_unsigned = suffix
        .bytes()
        .take_while(|b| matches!(b, b'u' | b'U' | b'l' | b'L' | b'w' | b'W' | b'b' | b'B'))
        .any(|b| b == b'u' || b == b'U');
    if !is_unsigned && raw_val > i64::MAX as u64 {
        is_unsigned = true;
    }
    let _ = is_hex_or_oct;
    (ExprToken::Num(raw_val as i64, is_unsigned), end)
}

/// The three `wchar_t` types the supported targets and flags produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wchar {
    /// `int`: x86-64, i686, RISC-V.
    Int,
    /// `unsigned int`: AArch64.
    UInt,
    /// `unsigned short`: `-fshort-wchar`.
    UShort,
}

/// How the target types the character constants `#if` can meet.
#[derive(Debug, Clone, Copy)]
struct CharModel {
    /// Plain `char` is unsigned (AArch64, RISC-V, `-funsigned-char`).
    char_unsigned: bool,
    wchar: Wchar,
}

impl CharModel {
    fn current() -> Self {
        Self {
            char_unsigned: crate::common::types::char_is_unsigned(),
            wchar: match crate::common::types::wchar_ctype() {
                CType::UShort => Wchar::UShort,
                CType::UInt => Wchar::UInt,
                _ => Wchar::Int,
            },
        }
    }
}

/// The `#if` operand a character-constant token denotes, as GCC's
/// `cpp_interpret_charconst` and `eval_token` compute it (measured on
/// gcc-16.2, x86-64 and `-m32`, with and without `-funsigned-char` and
/// `-fshort-wchar`):
///
/// * the VALUE is the constant's value in C: a single narrow character is a
///   `char` widened to `int` (`'\377'` is -1 where `char` is signed, 255
///   where it is not), a multi-character constant is an `int` (`'\377\377\377\377'`
///   is -1 everywhere), and a prefixed constant is its code unit in its own
///   type (`L'\xffffffff'` is the `int` -1 on x86, the `unsigned int`
///   0xffffffff on AArch64);
/// * the operand is UNSIGNED (so it drags the comparison to `uintmax_t`) when
///   that type is unsigned: `u'x'`, `U'x'`, `u8'x'` always, `L'x'` where
///   `wchar_t` is, and a single narrow character where plain `char` is —
///   `'a' > -1` is false under `-funsigned-char` in GCC and Clang alike. A
///   multi-character constant stays signed (GCC; Clang makes it unsigned
///   with `char`, which C11 6.4.4.4p10's `int` does not support).
fn char_constant_operand(kind: &TokenKind, model: CharModel) -> Option<PpVal> {
    Some(match *kind {
        TokenKind::CharLiteral(c) => {
            let byte = c as u32 as u8;
            if model.char_unsigned {
                (i64::from(byte), true)
            } else {
                (i64::from(byte as i8), false)
            }
        }
        TokenKind::IntLiteral(value) => (i64::from(value as i32), false),
        TokenKind::UnicodeCharLiteral(value, unit) => match unit {
            UnicodeCharKind::Utf8 => (i64::from(value & 0xFF), true),
            UnicodeCharKind::Utf16 => (i64::from(value & 0xFFFF), true),
            UnicodeCharKind::Utf32 => (i64::from(value), true),
            UnicodeCharKind::Wide => match model.wchar {
                Wchar::Int => (i64::from(value as i32), false),
                Wchar::UInt => (i64::from(value), true),
                Wchar::UShort => (i64::from(value & 0xFFFF), true),
            },
        },
        _ => return None,
    })
}

/// Scan a character constant (encoding prefix included) in `#if` / `#elif`
/// and return `(token, next_index)`.
///
/// The C lexer decodes the spelling, so escapes, universal character names,
/// UTF-8 source characters, multi-character accumulation and code-unit
/// widths are exactly the compiler's; only the operand's signedness is the
/// preprocessor's own (see `char_constant_operand`). A spelling that is not
/// one character constant evaluates to 0, as before.
fn scan_pp_char_token(bytes: &[u8], start: usize, model: CharModel) -> (ExprToken, usize) {
    let quote = start
        + bytes[start..]
            .iter()
            .position(|&b| b == b'\'')
            .unwrap_or(bytes.len() - start);
    let end = skip_literal_bytes(bytes, quote, b'\'').min(bytes.len());
    let wide_unit16 = model.wchar == Wchar::UShort;
    let value = Lexer::character_constant(bytes_to_str(bytes, start, end), wide_unit16)
        .and_then(|kind| char_constant_operand(&kind, model))
        .unwrap_or((0, false));
    (ExprToken::Num(value.0, value.1), end)
}

/// Avoids allocating Vec<char>; numbers are parsed directly from byte spans.
fn tokenize_expr(expr: &str, model: CharModel) -> Vec<ExprToken> {
    let mut tokens = Vec::with_capacity(16);
    let bytes = expr.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Skip whitespace
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Number: the whole pp-number (C23 digit separators included) is one
        // token; its value comes from the separator-free spelling.
        if b.is_ascii_digit() {
            let (tok, next) = scan_pp_number_token(bytes, i);
            i = next;
            tokens.push(tok);
            continue;
        }

        // Character literal
        if b == b'\'' && !is_digit_separator(bytes, i) {
            let (tok, next) = scan_pp_char_token(bytes, i, model);
            i = next;
            tokens.push(tok);
            continue;
        }

        // Identifier or 'defined'
        if is_ident_start_byte(b) {
            let start = i;
            i += 1;
            while i < len && is_ident_cont_byte(bytes[i]) {
                i += 1;
            }
            let ident = bytes_to_str(bytes, start, i);
            if bytes.get(i) == Some(&b'\'') && is_literal_prefix(ident, Some(b'\'')) {
                let (tok, next) = scan_pp_char_token(bytes, start, model);
                i = next;
                tokens.push(tok);
                continue;
            }
            if ident == "defined" {
                tokens.push(ExprToken::Defined);
            } else {
                tokens.push(ExprToken::Ident(ident.to_string()));
            }
            continue;
        }

        // Operators (two-character operators checked first)
        if i + 1 < len {
            let next = bytes[i + 1];
            let two_char_op: Option<&'static str> = match (b, next) {
                (b'!', b'=') => Some("!="),
                (b'&', b'&') => Some("&&"),
                (b'|', b'|') => Some("||"),
                (b'=', b'=') => Some("=="),
                (b'<', b'=') => Some("<="),
                (b'<', b'<') => Some("<<"),
                (b'>', b'=') => Some(">="),
                (b'>', b'>') => Some(">>"),
                _ => None,
            };
            if let Some(op) = two_char_op {
                tokens.push(ExprToken::Op(op));
                i += 2;
                continue;
            }
        }

        // Single-character operators and parens
        match b {
            b'(' => {
                tokens.push(ExprToken::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(ExprToken::RParen);
                i += 1;
            }
            b'!' => {
                tokens.push(ExprToken::Op("!"));
                i += 1;
            }
            b'&' => {
                tokens.push(ExprToken::Op("&"));
                i += 1;
            }
            b'|' => {
                tokens.push(ExprToken::Op("|"));
                i += 1;
            }
            b'+' => {
                tokens.push(ExprToken::Op("+"));
                i += 1;
            }
            b'-' => {
                tokens.push(ExprToken::Op("-"));
                i += 1;
            }
            b'*' => {
                tokens.push(ExprToken::Op("*"));
                i += 1;
            }
            b'/' => {
                tokens.push(ExprToken::Op("/"));
                i += 1;
            }
            b'%' => {
                tokens.push(ExprToken::Op("%"));
                i += 1;
            }
            b'~' => {
                tokens.push(ExprToken::Op("~"));
                i += 1;
            }
            b'^' => {
                tokens.push(ExprToken::Op("^"));
                i += 1;
            }
            b'?' => {
                tokens.push(ExprToken::Op("?"));
                i += 1;
            }
            b':' => {
                tokens.push(ExprToken::Op(":"));
                i += 1;
            }
            b',' => {
                tokens.push(ExprToken::Op(","));
                i += 1;
            }
            b'<' => {
                tokens.push(ExprToken::Op("<"));
                i += 1;
            }
            b'>' => {
                tokens.push(ExprToken::Op(">"));
                i += 1;
            }
            _ => {
                i += 1;
            } // skip unknown
        }
    }

    tokens
}

/// Recursive descent parser for preprocessor constant expressions.
/// All parse functions return (value_as_i64, is_unsigned).
/// The i64 stores the bit pattern; when is_unsigned is true, interpret via `as u64`.
/// Per C99 6.10.1: if either operand is unsigned, convert both to unsigned for the operation.
struct ExprParser<'a> {
    tokens: &'a [ExprToken],
    pos: usize,
    /// Parse inactive operands for grammar, but suppress their diagnostics.
    quiet: bool,
    undefined: Vec<String>,
}

/// Result of a preprocessor expression: (bit_pattern_as_i64, is_unsigned)
type PpVal = (i64, bool);

impl<'a> ExprParser<'a> {
    fn new(tokens: &'a [ExprToken]) -> Self {
        Self {
            tokens,
            pos: 0,
            quiet: false,
            undefined: Vec::new(),
        }
    }

    fn peek(&self) -> Option<&ExprToken> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&ExprToken> {
        let tok = self.tokens.get(self.pos);
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    /// Check if the next token matches a given operator string
    fn peek_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(ExprToken::Op(s)) if *s == op)
    }

    /// The comma operator (C11 6.5.17p3): left-associative, lower precedence
    /// than every other operator, and its value is its RIGHT operand.
    ///
    /// A constant *expression* excludes the comma operator (6.6p3), but both
    /// oracles evaluate `#if (A, B)` as `B` rather than diagnosing the line,
    /// and macro-generated condition lists do reach this form. Without the
    /// production the `, B` was simply left unparsed, so lccc tested `A` --
    /// `#if (3, 0)` took the true branch where GCC and Clang both take the
    /// false one.
    fn parse_comma(&mut self) -> PpVal {
        let mut value = self.parse_ternary();
        while self.peek_op(",") {
            self.advance();
            value = self.parse_ternary();
        }
        value
    }

    fn parse_ternary(&mut self) -> PpVal {
        let cond = self.parse_or();
        if self.peek_op("?") {
            self.advance(); // ?
            let quiet = self.quiet;
            self.quiet = quiet || cond.0 == 0;
            let then_val = self.parse_ternary();
            if self.peek_op(":") {
                self.advance(); // :
            }
            self.quiet = quiet || cond.0 != 0;
            let else_val = self.parse_ternary();
            self.quiet = quiet;
            if cond.0 != 0 { then_val } else { else_val }
        } else {
            cond
        }
    }

    fn parse_or(&mut self) -> PpVal {
        let mut left = self.parse_and();
        while self.peek_op("||") {
            self.advance();
            let quiet = self.quiet;
            self.quiet = quiet || left.0 != 0;
            let right = self.parse_and();
            self.quiet = quiet;
            // Logical operators always produce signed int result
            left = (if left.0 != 0 || right.0 != 0 { 1 } else { 0 }, false);
        }
        left
    }

    fn parse_and(&mut self) -> PpVal {
        let mut left = self.parse_bitor();
        while self.peek_op("&&") {
            self.advance();
            let quiet = self.quiet;
            self.quiet = quiet || left.0 == 0;
            let right = self.parse_bitor();
            self.quiet = quiet;
            left = (if left.0 != 0 && right.0 != 0 { 1 } else { 0 }, false);
        }
        left
    }

    fn parse_bitor(&mut self) -> PpVal {
        let mut left = self.parse_bitxor();
        while self.peek_op("|") {
            self.advance();
            let right = self.parse_bitxor();
            let u = left.1 || right.1;
            left = (left.0 | right.0, u);
        }
        left
    }

    fn parse_bitxor(&mut self) -> PpVal {
        let mut left = self.parse_bitand();
        while self.peek_op("^") {
            self.advance();
            let right = self.parse_bitand();
            let u = left.1 || right.1;
            left = (left.0 ^ right.0, u);
        }
        left
    }

    fn parse_bitand(&mut self) -> PpVal {
        let mut left = self.parse_equality();
        while self.peek_op("&") {
            self.advance();
            let right = self.parse_equality();
            let u = left.1 || right.1;
            left = (left.0 & right.0, u);
        }
        left
    }

    fn parse_equality(&mut self) -> PpVal {
        let mut left = self.parse_relational();
        loop {
            if self.peek_op("==") {
                self.advance();
                let right = self.parse_relational();
                let u = left.1 || right.1;
                let eq = if u {
                    (left.0 as u64) == (right.0 as u64)
                } else {
                    left.0 == right.0
                };
                left = (if eq { 1 } else { 0 }, false);
            } else if self.peek_op("!=") {
                self.advance();
                let right = self.parse_relational();
                let u = left.1 || right.1;
                let ne = if u {
                    (left.0 as u64) != (right.0 as u64)
                } else {
                    left.0 != right.0
                };
                left = (if ne { 1 } else { 0 }, false);
            } else {
                break;
            }
        }
        left
    }

    fn parse_relational(&mut self) -> PpVal {
        let mut left = self.parse_shift();
        loop {
            if self.peek_op("<") {
                self.advance();
                let right = self.parse_shift();
                let u = left.1 || right.1;
                let cmp = if u {
                    (left.0 as u64) < (right.0 as u64)
                } else {
                    left.0 < right.0
                };
                left = (if cmp { 1 } else { 0 }, false);
            } else if self.peek_op(">") {
                self.advance();
                let right = self.parse_shift();
                let u = left.1 || right.1;
                let cmp = if u {
                    (left.0 as u64) > (right.0 as u64)
                } else {
                    left.0 > right.0
                };
                left = (if cmp { 1 } else { 0 }, false);
            } else if self.peek_op("<=") {
                self.advance();
                let right = self.parse_shift();
                let u = left.1 || right.1;
                let cmp = if u {
                    (left.0 as u64) <= (right.0 as u64)
                } else {
                    left.0 <= right.0
                };
                left = (if cmp { 1 } else { 0 }, false);
            } else if self.peek_op(">=") {
                self.advance();
                let right = self.parse_shift();
                let u = left.1 || right.1;
                let cmp = if u {
                    (left.0 as u64) >= (right.0 as u64)
                } else {
                    left.0 >= right.0
                };
                left = (if cmp { 1 } else { 0 }, false);
            } else {
                break;
            }
        }
        left
    }

    fn parse_shift(&mut self) -> PpVal {
        let mut left = self.parse_additive();
        loop {
            if self.peek_op("<<") {
                self.advance();
                let right = self.parse_additive();
                // Shift: result has type of the left operand
                let shift_amt = right.0 as u32;
                if left.1 {
                    left = ((left.0 as u64).wrapping_shl(shift_amt) as i64, true);
                } else {
                    left = (left.0.wrapping_shl(shift_amt), left.1);
                }
            } else if self.peek_op(">>") {
                self.advance();
                let right = self.parse_additive();
                let shift_amt = right.0 as u32;
                if left.1 {
                    // Unsigned: logical shift right
                    left = ((left.0 as u64).wrapping_shr(shift_amt) as i64, true);
                } else {
                    // Signed: arithmetic shift right
                    left = (left.0.wrapping_shr(shift_amt), false);
                }
            } else {
                break;
            }
        }
        left
    }

    fn parse_additive(&mut self) -> PpVal {
        let mut left = self.parse_multiplicative();
        loop {
            if self.peek_op("+") {
                self.advance();
                let right = self.parse_multiplicative();
                let u = left.1 || right.1;
                // Wrapping add works the same for signed and unsigned at bit level
                left = (left.0.wrapping_add(right.0), u);
            } else if self.peek_op("-") {
                self.advance();
                let right = self.parse_multiplicative();
                let u = left.1 || right.1;
                left = (left.0.wrapping_sub(right.0), u);
            } else {
                break;
            }
        }
        left
    }

    fn parse_multiplicative(&mut self) -> PpVal {
        let mut left = self.parse_unary();
        loop {
            if self.peek_op("*") {
                self.advance();
                let right = self.parse_unary();
                let u = left.1 || right.1;
                if u {
                    left = ((left.0 as u64).wrapping_mul(right.0 as u64) as i64, true);
                } else {
                    left = (left.0.wrapping_mul(right.0), false);
                }
            } else if self.peek_op("/") {
                self.advance();
                let right = self.parse_unary();
                let u = left.1 || right.1;
                if right.0 == 0 {
                    left = (0, u);
                } else if u {
                    left = ((left.0 as u64).wrapping_div(right.0 as u64) as i64, true);
                } else {
                    // Avoid signed overflow on i64::MIN / -1
                    if left.0 == i64::MIN && right.0 == -1 {
                        left = (i64::MIN, false);
                    } else {
                        left = (left.0 / right.0, false);
                    }
                }
            } else if self.peek_op("%") {
                self.advance();
                let right = self.parse_unary();
                let u = left.1 || right.1;
                if right.0 == 0 {
                    left = (0, u);
                } else if u {
                    left = ((left.0 as u64).wrapping_rem(right.0 as u64) as i64, true);
                } else if left.0 == i64::MIN && right.0 == -1 {
                    left = (0, false);
                } else {
                    left = (left.0 % right.0, false);
                }
            } else {
                break;
            }
        }
        left
    }

    fn parse_unary(&mut self) -> PpVal {
        if self.peek_op("!") {
            self.advance();
            let val = self.parse_unary();
            // Logical NOT always produces signed int
            return (if val.0 == 0 { 1 } else { 0 }, false);
        }
        if self.peek_op("-") {
            self.advance();
            let val = self.parse_unary();
            // Unary minus: if unsigned, the result is still unsigned (wrapping)
            if val.1 {
                return ((val.0 as u64).wrapping_neg() as i64, true);
            } else {
                return (val.0.wrapping_neg(), false);
            }
        }
        if self.peek_op("+") {
            self.advance();
            return self.parse_unary();
        }
        if self.peek_op("~") {
            self.advance();
            let val = self.parse_unary();
            // Bitwise NOT preserves signedness
            return (!val.0, val.1);
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> PpVal {
        match self.peek().cloned() {
            Some(ExprToken::Num(n, u)) => {
                self.advance();
                (n, u)
            }
            Some(ExprToken::LParen) => {
                self.advance();
                let val = self.parse_comma();
                if self.peek() == Some(&ExprToken::RParen) {
                    self.advance();
                }
                val
            }
            Some(ExprToken::Defined) => {
                self.advance();
                // defined(X) or defined X
                let has_paren = if self.peek() == Some(&ExprToken::LParen) {
                    self.advance();
                    true
                } else {
                    false
                };
                // The identifier - in our evaluation, we've already resolved
                // defined() to 0 or 1, so identifiers here mean "not defined"
                if let Some(ExprToken::Ident(_)) = self.peek() {
                    self.advance();
                }
                if has_paren && self.peek() == Some(&ExprToken::RParen) {
                    self.advance();
                }
                // If we reach here during evaluation, the macro was not resolved
                // at expansion time, so it's not defined
                (0, false)
            }
            Some(ExprToken::Ident(ref name)) => {
                let name = name.clone();
                self.advance();
                // Undefined identifiers in #if evaluate to 0
                // Special case: "true" and "false"
                match name.as_str() {
                    "true" if super::utils::c23_dialect() => (1, false),
                    "false" if super::utils::c23_dialect() => (0, false),
                    _ => {
                        if !self.quiet {
                            self.undefined.push(name);
                        }
                        (0, false)
                    }
                }
            }
            _ => {
                self.advance(); // skip unknown
                (0, false)
            }
        }
    }
}

#[cfg(test)]
mod comma_operator_tests {
    use super::eval_const_expr;

    #[test]
    fn the_value_is_the_right_operand() {
        // Measured on gcc-16.2 and clang-23.1.4: both take the false branch for
        // `#if (3, 0)`, i.e. the comma yields its right operand.
        assert!(!eval_const_expr("(3, 0)"));
        assert!(eval_const_expr("(0, 1)"));
        assert!(eval_const_expr("0, 1"));
        assert!(!eval_const_expr("1, 0"));
    }

    #[test]
    fn comma_binds_looser_than_every_other_operator() {
        assert!(eval_const_expr("1 + 2, 4 > 3"));
        assert!(!eval_const_expr("1 + 2, 3 > 4"));
        assert!(eval_const_expr("(1 ? 0 : 1), 2"));
    }

    #[test]
    fn nested_parentheses_keep_their_own_comma() {
        assert!(eval_const_expr("((1, 2), 3)"));
        assert!(!eval_const_expr("((1, 2), 0)"));
    }
}

#[cfg(test)]
mod char_constant_tests {
    use super::{CharModel, Wchar, eval_const_expr_with};

    /// x86-64 and i686: signed `char`, signed 32-bit `wchar_t`.
    const X86: CharModel = CharModel {
        char_unsigned: false,
        wchar: Wchar::Int,
    };
    /// RISC-V, or x86 under `-funsigned-char`: unsigned `char`, signed `wchar_t`.
    const UCHAR: CharModel = CharModel {
        char_unsigned: true,
        wchar: Wchar::Int,
    };
    /// x86 under `-fshort-wchar`: signed `char`, 16-bit unsigned `wchar_t`.
    const SHORT: CharModel = CharModel {
        char_unsigned: false,
        wchar: Wchar::UShort,
    };
    /// AArch64: unsigned `char`, unsigned 32-bit `wchar_t`.
    const AARCH64: CharModel = CharModel {
        char_unsigned: true,
        wchar: Wchar::UInt,
    };

    /// Every row is `(expression, [X86, UCHAR, SHORT, AARCH64])`. The first
    /// three columns are gcc-16.2's answers (`-E` on x86-64, then with
    /// `-funsigned-char`, then with `-fshort-wchar`; `-m32` equals x86-64);
    /// the AArch64 column applies the same GCC rules to that ABI (Clang 23
    /// agrees except on the two rules GCC and Clang disagree about, the sign
    /// of a multi-character constant and which character of `L'ab'` counts).
    const MATRIX: &[(&str, [bool; 4])] = &[
        (r"L'\400'", [true, true, true, true]),
        (r"L'\400' == 256", [true, true, true, true]),
        (r"L'\xffffffff' < 0", [true, true, false, false]),
        (r"L'\xffffffff' > -1", [false, false, false, false]),
        (r"u'\xffff' == 65535", [true, true, true, true]),
        (r"u'a' > -1", [false, false, false, false]),
        (r"U'\xffffffff' > 0", [true, true, true, true]),
        (r"U'a' > -1", [false, false, false, false]),
        (r"'\377' < 0", [true, false, true, false]),
        (r"'a' > -1", [true, false, true, false]),
        (r"'\377' > -1", [false, false, false, false]),
        (r"'ab' == 24930", [true, true, true, true]),
        (r"'\377\377\377\377' == -1", [true, true, true, true]),
        (r"'ab' > -1", [true, true, true, true]),
        (r"L'ab' == 'b'", [true, true, true, true]),
        (r"L'a' > -1", [true, true, false, false]),
        (
            r"'\x41' == 65 && '\101' == 65 && '\n' == 10 && '\\' == 92 && '\'' == 39",
            [true; 4],
        ),
        (
            r"L'\x10FFFF' == 0x10FFFF && U'\U0010FFFF' == 0x10FFFF && u'\u00e9' == 0xe9",
            [true, true, false, true],
        ),
        (r"'\x80' == -128", [true, false, true, false]),
        (r"L'\u00e9' == 233", [true, true, true, true]),
        (r"'abcd' == 0x61626364", [true, true, true, true]),
        (r"'\0' == 0 && !'\0'", [true, true, true, true]),
        (r"(L'\xffffffff' >> 31) == -1", [true, true, false, false]),
        (
            r"L'\x80000000' == -2147483647 - 1",
            [true, true, false, false],
        ),
        // `u8` is `unsigned char` in C23: unsigned on every target.
        (r"u8'a' == 97 && u8'a' > -1", [false, false, false, false]),
        (r"u8'a' == 97", [true, true, true, true]),
        // A character beyond the BMP keeps its low surrogate in a 16-bit
        // `wchar_t`, exactly as `u'\U0001F600'` does.
        (r"L'\U0001F600' == 0xDE00", [false, false, true, false]),
        (r"L'\U0001F600' == 0x1F600", [true, true, false, true]),
        // A UTF-8 source character is one wide character but two narrow bytes.
        (
            "'\u{e9}' == 0xC3A9 && L'\u{e9}' == 0xE9",
            [true, true, true, true],
        ),
    ];

    #[test]
    fn character_constants_evaluate_like_gcc_on_every_target() {
        for (expr, expected) in MATRIX {
            for (model, want) in [X86, UCHAR, SHORT, AARCH64].into_iter().zip(expected) {
                assert_eq!(
                    eval_const_expr_with(expr, model),
                    *want,
                    "#if {expr} under {model:?}"
                );
            }
        }
    }

    #[test]
    fn a_wide_constant_is_one_operand_not_an_identifier_and_a_char() {
        // torture/execute/widechar-1.c: `L'\400'` is 256, so `#if L'\400'`
        // is taken. The prefix used to be read as the identifier `L` (0)
        // followed by the narrow `'\400'` (0, truncated).
        assert!(eval_const_expr_with(r"L'\400' - 255 == 1", X86));
        assert!(eval_const_expr_with(r"(L'\400') == 256 ? 1 : 0", X86));
        assert!(eval_const_expr_with(r"u8'\x7f' + U'\x1' == 128", X86));
    }

    #[test]
    fn a_malformed_constant_still_evaluates_to_zero() {
        // An unterminated or empty constant is an error in the compiler
        // proper; here it must neither panic nor swallow the operator after it.
        assert!(!eval_const_expr_with("'", X86));
        assert!(!eval_const_expr_with("L'", X86));
        assert!(eval_const_expr_with("'' == 0", X86));
    }
}
