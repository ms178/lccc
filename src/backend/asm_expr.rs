/// Shared assembly expression evaluator for all assembler backends (x86, i686, ARM, RISC-V).
///
/// Supports arithmetic expressions with proper operator precedence:
///   - Parentheses: `(expr)`
///   - Bitwise OR: `|`
///   - Bitwise XOR: `^`
///   - Bitwise AND: `&`
///   - Shifts: `<<`, `>>`
///   - Addition/Subtraction: `+`, `-`
///   - Multiplication/Division/Modulo: `*`, `/`, `%`
///   - Unary: `-`, `+`, `~`, `!` (logical NOT)
///
/// Integer literals: decimal, hex (0x), binary (0b), octal (leading 0),
/// character literals ('c', '\n').
/// Used by all four assembler backends (x86, i686, ARM, RISC-V).
/// Map a C escape character to its ASCII value (e.g., b'n' -> 10 for '\n').
fn char_escape_value(esc: u8) -> i64 {
    match esc {
        b'n' => 10,
        b't' => 9,
        b'r' => 13,
        b'0' => 0,
        b'\\' => b'\\' as i64,
        b'\'' => b'\'' as i64,
        b'"' => b'"' as i64,
        b'a' => 7,
        b'b' => 8,
        b'f' => 12,
        b'v' => 11,
        _ => esc as i64,
    }
}

/// Token type for the expression evaluator.
#[derive(Debug)]
enum ExprToken {
    Num(i64),
    Op(char),
    Op2(&'static str),
}

/// Tokenize an expression string into ExprTokens.
fn tokenize_expr(s: &str) -> Result<Vec<ExprToken>, String> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'('
            || c == b')'
            || c == b'+'
            || c == b'-'
            || c == b'*'
            || c == b'/'
            || c == b'%'
            || c == b'&'
            || c == b'|'
            || c == b'^'
            || c == b'~'
            || c == b'!'
        {
            tokens.push(ExprToken::Op(c as char));
            i += 1;
        } else if c == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'<' {
            tokens.push(ExprToken::Op2("<<"));
            i += 2;
        } else if c == b'>' && i + 1 < bytes.len() && bytes[i + 1] == b'>' {
            tokens.push(ExprToken::Op2(">>"));
            i += 2;
        } else if c == b'\'' {
            // Character literal: 'c' or '\n', '\t', '\\', etc.
            i += 1; // skip opening quote
            if i >= bytes.len() {
                return Err("unterminated character literal".to_string());
            }
            let ch_val = if bytes[i] == b'\\' {
                i += 1;
                if i >= bytes.len() {
                    return Err("unterminated character escape".to_string());
                }
                let esc = bytes[i];
                i += 1;
                char_escape_value(esc)
            } else {
                let val = bytes[i] as i64;
                i += 1;
                val
            };
            // Skip closing quote if present
            if i < bytes.len() && bytes[i] == b'\'' {
                i += 1;
            }
            tokens.push(ExprToken::Num(ch_val));
        } else if c.is_ascii_digit() {
            let start = i;
            if c == b'0' && i + 1 < bytes.len() && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
                i += 2;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
            } else if c == b'0'
                && i + 1 < bytes.len()
                && (bytes[i + 1] == b'b' || bytes[i + 1] == b'B')
            {
                i += 2;
                while i < bytes.len() && (bytes[i] == b'0' || bytes[i] == b'1') {
                    i += 1;
                }
            } else {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            // C integer suffixes (u/U/l/L/z/Z, incl. LL/ULL combos) are valid
            // in assembler constant expressions (e.g. `$(1U<<1)` from glibc).
            let mut suffix_len = 0;
            while i + suffix_len < bytes.len()
                && matches!(
                    bytes[i + suffix_len],
                    b'u' | b'U' | b'l' | b'L' | b'z' | b'Z'
                )
                && suffix_len < 4
            {
                suffix_len += 1;
            }
            i += suffix_len;
            let num_str = &s[start..i];
            let val = parse_single_integer(num_str)?;
            tokens.push(ExprToken::Num(val));
        } else {
            return Err(format!("unexpected char '{}' in expression", c as char));
        }
    }
    Ok(tokens)
}

fn eval_tokens(tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
    eval_or(tokens, pos)
}

fn eval_or(tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
    let mut val = eval_xor(tokens, pos)?;
    while *pos < tokens.len() {
        if matches!(&tokens[*pos], ExprToken::Op('|')) {
            *pos += 1;
            val |= eval_xor(tokens, pos)?;
        } else {
            break;
        }
    }
    Ok(val)
}

fn eval_xor(tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
    let mut val = eval_and(tokens, pos)?;
    while *pos < tokens.len() {
        if matches!(&tokens[*pos], ExprToken::Op('^')) {
            *pos += 1;
            val ^= eval_and(tokens, pos)?;
        } else {
            break;
        }
    }
    Ok(val)
}

fn eval_and(tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
    let mut val = eval_shift(tokens, pos)?;
    while *pos < tokens.len() {
        if matches!(&tokens[*pos], ExprToken::Op('&')) {
            *pos += 1;
            val &= eval_shift(tokens, pos)?;
        } else {
            break;
        }
    }
    Ok(val)
}

fn eval_shift(tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
    let mut val = eval_add(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            ExprToken::Op2("<<") => {
                *pos += 1;
                val <<= eval_add(tokens, pos)?;
            }
            ExprToken::Op2(">>") => {
                *pos += 1;
                val = ((val as u64) >> eval_add(tokens, pos)?) as i64;
            }
            _ => break,
        }
    }
    Ok(val)
}

fn eval_add(tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
    let mut val = eval_mul(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            ExprToken::Op('+') => {
                *pos += 1;
                val += eval_mul(tokens, pos)?;
            }
            ExprToken::Op('-') => {
                *pos += 1;
                val -= eval_mul(tokens, pos)?;
            }
            _ => break,
        }
    }
    Ok(val)
}

fn eval_mul(tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
    let mut val = eval_unary(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            ExprToken::Op('*') => {
                *pos += 1;
                val *= eval_unary(tokens, pos)?;
            }
            ExprToken::Op('/') => {
                *pos += 1;
                let rhs = eval_unary(tokens, pos)?;
                if rhs == 0 {
                    return Err("division by zero".to_string());
                }
                val /= rhs;
            }
            ExprToken::Op('%') => {
                *pos += 1;
                let rhs = eval_unary(tokens, pos)?;
                if rhs == 0 {
                    return Err("modulo by zero".to_string());
                }
                val %= rhs;
            }
            _ => break,
        }
    }
    Ok(val)
}

fn eval_unary(tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
    if *pos >= tokens.len() {
        return Err("unexpected end of expression".to_string());
    }
    match &tokens[*pos] {
        ExprToken::Op('-') => {
            *pos += 1;
            Ok(-eval_unary(tokens, pos)?)
        }
        ExprToken::Op('+') => {
            *pos += 1;
            eval_unary(tokens, pos)
        }
        ExprToken::Op('~') => {
            *pos += 1;
            Ok(!eval_unary(tokens, pos)?)
        }
        ExprToken::Op('!') => {
            *pos += 1;
            Ok(if eval_unary(tokens, pos)? == 0 { 1 } else { 0 })
        }
        ExprToken::Op('(') => {
            *pos += 1;
            let val = eval_tokens(tokens, pos)?;
            if *pos < tokens.len() && matches!(&tokens[*pos], ExprToken::Op(')')) {
                *pos += 1;
            } else {
                return Err("missing closing parenthesis".to_string());
            }
            Ok(val)
        }
        ExprToken::Num(v) => {
            let v = *v;
            *pos += 1;
            Ok(v)
        }
        other => Err(format!("unexpected token in expression: {:?}", other)),
    }
}

/// The radix a bare integer literal is spelled in.
///
/// The evaluator's width per radix is part of its observable behaviour, not an
/// implementation detail: hex and decimal were widened to `u64` (so
/// `0xffffffffffffffff` and `18446744073709551615` are literals) while binary
/// and octal stayed `i64` (`0b1` plus 62 more ones is the widest binary
/// literal, `0777777777777777777777` the widest octal).  Unifying the four
/// would silently change what assembly the assembler accepts, so each radix
/// carries its own bound — and now carries it in exactly one place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Radix {
    Decimal,
    Hex,
    Binary,
    Octal,
}

impl Radix {
    #[inline]
    fn base(self) -> u64 {
        match self {
            Radix::Decimal => 10,
            Radix::Hex => 16,
            Radix::Binary => 2,
            Radix::Octal => 8,
        }
    }

    /// The word the evaluator's error message has always used for this radix
    /// (`bad hex: ...`), with `Decimal` spelling the generic `bad integer: ...`.
    #[inline]
    fn label(self) -> &'static str {
        match self {
            Radix::Hex => "hex",
            Radix::Binary => "binary",
            Radix::Octal => "octal",
            Radix::Decimal => "integer",
        }
    }
}

/// Select the radix of a suffix-stripped literal body and return its digit span.
///
/// A leading zero selects hex, binary or octal and NEVER falls back to decimal:
/// `09` is spelled octal, so it is not a literal (GAS agrees, and the
/// evaluator's own `from_str_radix(.., 8)` rejected it the same way).
#[inline]
fn literal_shape(body: &[u8]) -> (Radix, &[u8]) {
    if body.len() > 1 && body[0] == b'0' {
        match body[1] {
            b'x' | b'X' => return (Radix::Hex, &body[2..]),
            b'b' | b'B' => return (Radix::Binary, &body[2..]),
            // The octal test is the evaluator's own: every byte a DECIMAL digit
            // (so `0z9` is not octal-shaped) and the value read in base 8.
            _ if body.iter().all(|b| b.is_ascii_digit()) => return (Radix::Octal, body),
            _ => {}
        }
    }
    (Radix::Decimal, body)
}

/// The radix a bare literal is spelled in, for error messages.
///
/// Radix selection lives here once: [`bare_integer_literal`] uses the same
/// function, so a message can never name a radix the recognizer did not try.
pub fn bare_integer_radix(bytes: &[u8]) -> Radix {
    // The recognizer's own selection, so a message can never name a radix that
    // was not attempted: behind a sign `literal_shape` falls through to
    // `Decimal`, which is why `+0x1f` has always been reported as `bad integer`
    // rather than `bad hex`.
    literal_shape(strip_integer_suffixes(bytes)).0
}

/// Strip the C integer suffixes (`u`/`U`/`l`/`L`/`z`/`Z`, incl. LL/ULL combos)
/// that are valid in assembler constant expressions (`$(1U<<1)` from glibc).
#[inline]
fn strip_integer_suffixes(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && matches!(bytes[end - 1], b'u' | b'U' | b'l' | b'L' | b'z' | b'Z') {
        end -= 1;
    }
    &bytes[..end]
}

/// Recognize AND value a bare integer literal in one allocation-free pass.
///
/// ONE copy of the integer grammar, shared by the evaluator
/// ([`parse_single_integer`], which values the literal) and by the x86
/// assembler's `is_label_like`, which asks only the yes/no question.  That
/// predicate runs for every candidate symbol of every operand of every assembly
/// line, and the `Result<_, String>` it used to call allocates an error `String`
/// on each miss — twice, because `tokenize_expr` then fails on the identifier's
/// leading character as well.  Sharing the grammar keeps the hot path
/// allocation-free WITHOUT keeping a second recognizer to drift.
///
/// The result is the evaluator's `i64`, so a hex literal above `i64::MAX` comes
/// back as its two's-complement pattern exactly as `u64 as i64` did before.
///
/// Two shapes deserve a word, because both are behaviour rather than detail and
/// a deduplication that moved either would be a silent semantic change:
///
///   * the WIDTH per radix: hex and decimal were widened to `u64` (so
///     `0xffffffffffffffff` and `18446744073709551615` are literals) while
///     binary and octal stayed `i64`.  A negative body may reach the type's
///     most-negative value, whose magnitude is one GREATER than its maximum —
///     `parse::<i64>` accepts `-9223372036854775808` and rejects
///     `9223372036854775808` — and `u64` has no negative at all, so `0x-1`
///     stays rejected;
///   * a sign INSIDE the body: `from_str_radix` and `str::parse` each accept one
///     (`0x+0`, `0b-1`), so the evaluator accepted them, and GAS assembles
///     `.long 0x+0` as well — as `0x` followed by `+0`, a different mechanism
///     with the same answer.  One sign is therefore honoured here.  A SECOND is
///     not (`--42`): the evaluator's tokenizer values that as `-(-42)`, the same
///     42 the single-literal path produced, so `parse_integer_expr` is
///     unchanged.  `tokenize_expr` only ever hands this function a slice
///     starting at a digit, so it cannot see a sign at all;
///     `the_public_evaluator_is_unchanged_by_the_shared_grammar` pins both
///     claims against the values the old paths produced.
pub fn bare_integer_literal(bytes: &[u8]) -> Option<i64> {
    let lit = strip_integer_suffixes(bytes);
    if lit.is_empty() {
        return None;
    }
    // Radix selection sees the text as given, which is why `+0x1f` is decimal
    // (and therefore not a literal): the prefix match never fires behind a sign.
    let (radix, digits) = literal_shape(lit);
    let (negative, digits) = match digits.first() {
        Some(b'+') => (false, &digits[1..]),
        Some(b'-') => (true, &digits[1..]),
        _ => (false, digits),
    };
    if digits.is_empty() {
        return None;
    }
    let bound = match (radix, negative) {
        (Radix::Hex, true) => return None, // u64 has no negative values
        (Radix::Hex, false) | (Radix::Decimal, false) => u64::MAX,
        (Radix::Binary, false) | (Radix::Octal, false) => i64::MAX as u64,
        (_, true) => 1u64 << 63, // |i64::MIN|
    };
    let base = radix.base();
    let mut magnitude: u64 = 0;
    for &b in digits {
        let digit = match b {
            b'0'..=b'9' => u64::from(b - b'0'),
            b'a'..=b'f' => u64::from(b - b'a' + 10),
            b'A'..=b'F' => u64::from(b - b'A' + 10),
            _ => return None,
        };
        if digit >= base {
            return None;
        }
        magnitude = magnitude.checked_mul(base)?.checked_add(digit)?;
    }
    if magnitude > bound {
        return None;
    }
    let value = magnitude as i64;
    Some(if negative {
        value.wrapping_neg()
    } else {
        value
    })
}

/// Parse a single integer value (no arithmetic expressions).
/// Supports decimal, hex (0x/0X), binary (0b/0B), octal (leading 0),
/// and character literals ('c', '\n', etc.).
fn parse_single_integer(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty integer".to_string());
    }

    // Strip C integer suffixes (u/U/l/L/z/Z, incl. LL/ULL combos).
    let s = {
        let bytes = s.as_bytes();
        let mut end = bytes.len();
        while end > 0 && matches!(bytes[end - 1], b'u' | b'U' | b'l' | b'L' | b'z' | b'Z') {
            end -= 1;
        }
        &s[..end]
    };
    if s.is_empty() {
        return Err("integer has only a suffix".to_string());
    }

    // Character literal: 'c' or '\n' etc. Must be the entire string
    // (e.g., "'!'" or "'\n'"), not part of a larger expression.
    if s.starts_with('\'') {
        let bytes = s.as_bytes();
        let mut i = 1;
        if i >= bytes.len() {
            return Err("unterminated character literal".to_string());
        }
        let val;
        if bytes[i] == b'\\' {
            i += 1;
            if i >= bytes.len() {
                return Err("unterminated character escape".to_string());
            }
            val = char_escape_value(bytes[i]);
            i += 1;
        } else {
            val = bytes[i] as i64;
            i += 1;
        }
        // Skip closing quote
        if i < bytes.len() && bytes[i] == b'\'' {
            i += 1;
        }
        // Only accept if we consumed the whole string
        if i == bytes.len() {
            return Ok(val);
        }
        // Otherwise, fall through to let tokenize_expr handle it as an expression
        return Err("character literal has trailing content".to_string());
    }

    let (negative, s) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };

    // Radix selection, per-radix width, suffix stripping and the leading '+'
    // rule are the shared grammar's business, not this function's: the x86
    // assembler asks the same yes/no question through `bare_integer_literal` on
    // a path that must not allocate, and two recognizers for one grammar drift.
    let Some(val) = bare_integer_literal(s.as_bytes()) else {
        return Err(format!(
            "bad {}: {}",
            bare_integer_radix(s.as_bytes()).label(),
            s
        ));
    };

    Ok(if negative { val.wrapping_neg() } else { val })
}

/// Parse an integer expression with full operator precedence.
/// Supports: |, ^, &, <<, >>, +, -, *, /, %, ~, parentheses.
/// Falls back to simple integer parsing for plain numbers.
pub fn parse_integer_expr(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty integer".to_string());
    }
    // Fast path: try simple integer first
    if let Ok(val) = parse_single_integer(s) {
        return Ok(val);
    }
    let tokens = tokenize_expr(s)?;
    if tokens.is_empty() {
        return Err("empty expression".to_string());
    }
    let mut pos = 0;
    let val = eval_tokens(&tokens, &mut pos)?;
    if pos < tokens.len() {
        return Err(format!(
            "unexpected trailing token in expression: {:?}",
            tokens[pos]
        ));
    }
    Ok(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_integers() {
        assert_eq!(parse_integer_expr("42").unwrap(), 42);
        assert_eq!(parse_integer_expr("-1").unwrap(), -1);
        assert_eq!(parse_integer_expr("0xFF").unwrap(), 255);
        assert_eq!(parse_integer_expr("0b1010").unwrap(), 10);
        assert_eq!(parse_integer_expr("0644").unwrap(), 420);
    }

    #[test]
    fn test_arithmetic() {
        assert_eq!(parse_integer_expr("8 * 16 + 8 * 8").unwrap(), 192);
        assert_eq!(parse_integer_expr("(8 * 16 + 8 * 8)").unwrap(), 192);
        assert_eq!(
            parse_integer_expr("8*2 + (8 * 16 + 8 * 8) + 64").unwrap(),
            272
        );
        assert_eq!(parse_integer_expr("-(8 * 8 + 8 * 8 + 16)").unwrap(), -144);
        assert_eq!(
            parse_integer_expr("-(8 * 8 + 8 * 8 + 16)+0*8").unwrap(),
            -144
        );
    }

    #[test]
    fn test_bitwise() {
        assert_eq!(parse_integer_expr("0x100 | 0x4000 | 17").unwrap(), 0x4111);
        assert_eq!(parse_integer_expr("0xFF & 0x0F").unwrap(), 0x0F);
        assert_eq!(parse_integer_expr("1 << 5").unwrap(), 32);
        assert_eq!(parse_integer_expr("~0").unwrap(), -1);
        assert_eq!(parse_integer_expr("0xFF ^ 0x0F").unwrap(), 0xF0);
    }

    #[test]
    fn test_logical_not() {
        // GAS logical NOT: !0 = 1, !1 = 0, !nonzero = 0
        assert_eq!(parse_integer_expr("!0").unwrap(), 1);
        assert_eq!(parse_integer_expr("!1").unwrap(), 0);
        assert_eq!(parse_integer_expr("!42").unwrap(), 0);
        // Used in FFmpeg: 1+!0 = 2
        assert_eq!(parse_integer_expr("1+!0").unwrap(), 2);
        assert_eq!(parse_integer_expr("1+!1").unwrap(), 1);
    }

    #[test]
    fn test_complex() {
        // libffi CALL_CONTEXT_SIZE = (N_V_ARG_REG * 16 + N_X_ARG_REG * 8) where N_V=8, N_X=8
        assert_eq!(parse_integer_expr("(8 * 16 + 8 * 8)").unwrap(), 192);
        // ffi_closure_SYSV_FS = (8*2 + CALL_CONTEXT_SIZE + 64)
        assert_eq!(
            parse_integer_expr("(8*2 + (8 * 16 + 8 * 8) + 64)").unwrap(),
            272
        );
        // musl vfork: CLONE_VM | CLONE_VFORK | SIGCHLD
        assert_eq!(parse_integer_expr("0x100 | 0x4000 | 17").unwrap(), 0x4111);
    }

    #[test]
    fn test_operator_precedence() {
        // * binds tighter than +
        assert_eq!(parse_integer_expr("2 + 3 * 4").unwrap(), 14);
        assert_eq!(parse_integer_expr("3 * 4 + 2").unwrap(), 14);
        // | binds looser than +
        assert_eq!(parse_integer_expr("1 | 2 + 4").unwrap(), 7); // 1 | (2+4) = 1 | 6 = 7
    }

    #[test]
    fn test_character_literals() {
        // Simple ASCII characters
        assert_eq!(parse_integer_expr("'!'").unwrap(), 33);
        assert_eq!(parse_integer_expr("'A'").unwrap(), 65);
        assert_eq!(parse_integer_expr("'0'").unwrap(), 48);
        assert_eq!(parse_integer_expr("' '").unwrap(), 32);
        // Escape sequences
        assert_eq!(parse_integer_expr("'\\n'").unwrap(), 10);
        assert_eq!(parse_integer_expr("'\\t'").unwrap(), 9);
        assert_eq!(parse_integer_expr("'\\r'").unwrap(), 13);
        assert_eq!(parse_integer_expr("'\\0'").unwrap(), 0);
        assert_eq!(parse_integer_expr("'\\\\").unwrap(), 92);
        assert_eq!(parse_integer_expr("'\\''").unwrap(), 39);
        // Character literals in expressions
        assert_eq!(parse_integer_expr("'A' + 1").unwrap(), 66);
        assert_eq!(parse_integer_expr("'!' | 0x80").unwrap(), 0xA1);
    }
}
