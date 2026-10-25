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

/// Advance past a GAS character literal starting at `quote_idx` (`bytes[quote_idx] == b'\''`).
///
/// In GAS (`do_scrub_chars`), a character literal is `'X` or `'\X`, optionally
/// closed by a trailing `'`. Because the literal is bounded to a single character
/// (or escape pair), any `,`, `;`, `#`, `//`, `@`, or `(`/`)'` inside the literal
/// is data rather than punctuation, and an unclosed `'x` never runs past that single character.
pub fn skip_gas_char_literal(bytes: &[u8], quote_idx: usize) -> usize {
    let mut i = quote_idx + 1;
    if i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 1;
            if i < bytes.len() {
                i += std::str::from_utf8(&bytes[i..])
                    .ok()
                    .and_then(|s| s.chars().next())
                    .map_or(1, char::len_utf8);
            }
        } else {
            i += std::str::from_utf8(&bytes[i..])
                .ok()
                .and_then(|s| s.chars().next())
                .map_or(1, char::len_utf8);
        }
        if i < bytes.len() && bytes[i] == b'\'' {
            i += 1;
        }
    }
    i
}

/// Trim leading and trailing ASCII whitespace on an assembly line or operand
/// slice while preserving the single payload whitespace byte of a trailing
/// unclosed GAS character literal (`... ' ` or `... '\ `).
pub fn trim_asm_line(s: &str) -> &str {
    let s = s.trim_start();
    if s.is_empty() {
        return s;
    }
    let bytes = s.as_bytes();
    let mut in_string = false;
    let mut i = 0usize;
    let mut keep_end = 0usize;
    while i < bytes.len() {
        if in_string {
            if bytes[i] == b'\\' {
                i += 1;
                if i < bytes.len() {
                    i += std::str::from_utf8(&bytes[i..])
                        .ok()
                        .and_then(|rest| rest.chars().next())
                        .map_or(1, char::len_utf8);
                }
                keep_end = i;
                continue;
            }
            if bytes[i] == b'"' {
                in_string = false;
            }
            i += 1;
            keep_end = i;
            continue;
        }
        match bytes[i] {
            b'"' => {
                in_string = true;
                i += 1;
                keep_end = i;
            }
            b'\'' => {
                i = skip_gas_char_literal(bytes, i);
                keep_end = i;
            }
            b if b.is_ascii_whitespace() => {
                i += 1;
            }
            _ => {
                i += std::str::from_utf8(&bytes[i..])
                    .ok()
                    .and_then(|rest| rest.chars().next())
                    .map_or(1, char::len_utf8);
                keep_end = i;
            }
        }
    }
    &s[..keep_end]
}

/// Trim ASCII whitespace around an assembler operand while preserving the
/// payload whitespace byte of an unclosed GAS character literal (`' ` or `'\ `).
pub fn trim_data_arg(s: &str) -> &str {
    trim_asm_line(s)
}

/// Split a comma-separated assembler operand list on top-level commas,
/// respecting `"..."` strings (with `\.` escapes), GAS character literals
/// (`'c'` / `'\n'` / unclosed `'c`), and balanced parentheses `(...)`.
pub fn split_comma_args(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut in_string = false;
    let mut paren_depth = 0i32;

    while i < bytes.len() {
        if in_string {
            if bytes[i] == b'\\' {
                i += 1;
                if i < bytes.len() {
                    i += std::str::from_utf8(&bytes[i..])
                        .ok()
                        .and_then(|rest| rest.chars().next())
                        .map_or(1, char::len_utf8);
                }
                continue;
            }
            if bytes[i] == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match bytes[i] {
            b'"' => {
                in_string = true;
                i += 1;
            }
            b'\'' => {
                i = skip_gas_char_literal(bytes, i);
            }
            b'(' => {
                paren_depth += 1;
                i += 1;
            }
            b')' => {
                paren_depth = (paren_depth - 1).max(0);
                i += 1;
            }
            b',' if paren_depth == 0 => {
                parts.push(&s[start..i]);
                i += 1;
                start = i;
            }
            _ => {
                i += 1;
            }
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Token type for the expression evaluator.
#[derive(Debug)]
enum ExprToken {
    Num(i64),
    Op(char),
    Op2(&'static str),
    /// A symbol or the location counter `.`; produced only in symbol mode
    /// ([`eval_section_expr`]) -- the numeric evaluator keeps rejecting
    /// identifiers at the lexer, exactly as before.
    Sym(String),
}

/// Tokenize an expression string into ExprTokens.
fn tokenize_expr(s: &str) -> Result<Vec<ExprToken>, String> {
    tokenize_expr_mode(s, false)
}

/// `[A-Za-z_.$]` starts a GAS symbol; digits and those continue one.
fn is_sym_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'.' || c == b'$'
}

/// The end of a numeric local-label reference (`1b`, `42f`) starting at
/// `i`: digits, then `b` or `f`, not followed by a symbol character (`0b1`
/// is binary 1, `1fx` no reference).
pub fn numeric_label_ref_end(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    while j < bytes.len() && bytes[j].is_ascii_digit() {
        j += 1;
    }
    let suffix = *bytes.get(j)?;
    if j == i || (suffix != b'b' && suffix != b'f') {
        return None;
    }
    match bytes.get(j + 1) {
        Some(&n) if is_sym_start(n) || n.is_ascii_digit() => None,
        _ => Some(j + 1),
    }
}

/// `expr` with every numeric local-label reference (`1b`, `42f`) replaced
/// by `resolve`'s name for it (unchanged when `resolve` returns `None`).
/// Only whole tokens are references: digits inside a symbol name (`x1b`)
/// and numbers of any radix (`0x1f`, `0b1`) are copied as they are, while
/// `0b` alone is label 0 backward.
pub fn rewrite_numeric_label_refs(
    expr: &str,
    mut resolve: impl FnMut(&str) -> Option<String>,
) -> String {
    let bytes = expr.as_bytes();
    let is_sym = |b: u8| is_sym_start(b) || b.is_ascii_digit();
    let mut out = String::with_capacity(expr.len());
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        if bytes[i].is_ascii_digit() {
            if let Some(end) = numeric_label_ref_end(bytes, i)
                && let Some(name) = resolve(&expr[i..end])
            {
                out.push_str(&name);
                i = end;
                continue;
            }
            while i < bytes.len() && is_sym(bytes[i]) {
                i += 1;
            }
        } else if is_sym_start(bytes[i]) {
            while i < bytes.len() && is_sym(bytes[i]) {
                i += 1;
            }
        } else {
            i += expr[i..].chars().next().map_or(1, char::len_utf8);
        }
        out.push_str(&expr[start..i]);
    }
    out
}

/// `s` is one GAS symbol name and nothing else (`.` alone is the location
/// counter, not a symbol).
pub fn is_bare_symbol(s: &str) -> bool {
    let b = s.as_bytes();
    s != "."
        && b.first().is_some_and(|&c| is_sym_start(c))
        && b.iter().all(|&c| is_sym_start(c) || c.is_ascii_digit())
}

fn tokenize_expr_mode(s: &str, symbols: bool) -> Result<Vec<ExprToken>, String> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Two-character operators first: << >> <= >= == != && || <>.
        // A single < or > is GAS's COMPARISON operator (see eval_compare),
        // not part of a shift — `mov $0 < 1, %esp` is a complete
        // expression (GAS 2.47 byte-probed: evaluates to -1, i.e. TRUE is
        // all-ones — not C's 1).
        if c == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'<' {
            tokens.push(ExprToken::Op2("<<"));
            i += 2;
        } else if c == b'>' && i + 1 < bytes.len() && bytes[i + 1] == b'>' {
            tokens.push(ExprToken::Op2(">>"));
            i += 2;
        } else if c == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'=' {
            tokens.push(ExprToken::Op2("<="));
            i += 2;
        } else if c == b'>' && i + 1 < bytes.len() && bytes[i + 1] == b'=' {
            tokens.push(ExprToken::Op2(">="));
            i += 2;
        } else if c == b'=' && i + 1 < bytes.len() && bytes[i + 1] == b'=' {
            tokens.push(ExprToken::Op2("=="));
            i += 2;
        } else if c == b'!' && i + 1 < bytes.len() && bytes[i + 1] == b'=' {
            tokens.push(ExprToken::Op2("!="));
            i += 2;
        } else if c == b'&' && i + 1 < bytes.len() && bytes[i + 1] == b'&' {
            tokens.push(ExprToken::Op2("&&"));
            i += 2;
        } else if c == b'|' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
            tokens.push(ExprToken::Op2("||"));
            i += 2;
        } else if c == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'>' {
            // GAS accepts `<>` as an alternative spelling of `!=`
            // (`mov $1 <> 0, %ecx` — byte-probed on 2.47: b9 ff ff ff ff).
            tokens.push(ExprToken::Op2("!="));
            i += 2;
        } else if symbols && is_sym_start(c) {
            let start = i;
            i += 1;
            while i < bytes.len() && (is_sym_start(bytes[i]) || bytes[i].is_ascii_digit()) {
                i += 1;
            }
            tokens.push(ExprToken::Sym(s[start..i].to_string()));
        } else if symbols && let Some(end) = numeric_label_ref_end(bytes, i) {
            // `1b` / `1f`: a reference to the nearest numeric label `1:`
            // backward / forward, a symbol (GAS), not the integer 1.
            tokens.push(ExprToken::Sym(s[i..end].to_string()));
            i = end;
        } else if c == b'('
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
            || c == b'<'
            || c == b'>'
        {
            tokens.push(ExprToken::Op(c as char));
            i += 1;
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

/// GAS operator precedence, loosest to tightest (binutils manual,
/// "Operator Precedence"): || < && < | < ^ < & < == != <> < < > <= >= <
/// << >> < + - < * / % < unary. Every comparison and logical operator yields
/// -1 (all ones) for TRUE and 0 for false -- the shell convention, NOT C's 1.
/// `mov $0 < 1, %esp` is therefore bc ff ff ff ff (GAS 2.47 byte-probed).
const PRECEDENCE: [&[&str]; 10] = [
    &["||"],
    &["&&"],
    &["|"],
    &["^"],
    &["&"],
    &["==", "!="],
    &["<", ">", "<=", ">="],
    &["<<", ">>"],
    &["+", "-"],
    &["*", "/", "%"],
];

/// The value domain the one grammar evaluates in: plain integers for
/// [`parse_integer_expr`], section-relative terms for [`eval_section_expr`].
trait ExprDomain: Sized {
    fn num(v: i64) -> Self;
    fn binary(self, op: &'static str, rhs: Self) -> Result<Self, String>;
    fn unary(self, op: char) -> Result<Self, String>;
}

/// The integer law of every binary operator (GAS 2.47, measured).
fn int_binary(op: &str, l: i64, r: i64) -> i64 {
    match op {
        "||" => -i64::from(l != 0 || r != 0),
        "&&" => -i64::from(l != 0 && r != 0),
        "|" => l | r,
        "^" => l ^ r,
        "&" => l & r,
        "==" => -i64::from(l == r),
        "!=" => -i64::from(l != r),
        "<" => -i64::from(l < r),
        ">" => -i64::from(l > r),
        "<=" => -i64::from(l <= r),
        ">=" => -i64::from(l >= r),
        // A shift count outside 0..64 warns ("shift count out of range (64 is
        // not between 0 and 63)") and yields 0. Letting Rust mask the count
        // (1 << 64 -> 1 << 0 -> 1) made `.rept 1<<64` expand once instead of
        // zero times, and a negative count panicked the assembler.
        "<<" => {
            if (0..64).contains(&r) {
                l << r
            } else {
                0
            }
        }
        ">>" => {
            if (0..64).contains(&r) {
                ((l as u64) >> r) as i64
            } else {
                0
            }
        }
        // GAS evaluates in bignum and keeps the low bits; wrapping arithmetic
        // reproduces them identically in every profile (plain +,- would panic
        // in debug on overflow).
        "+" => l.wrapping_add(r),
        "-" => l.wrapping_sub(r),
        "*" => l.wrapping_mul(r),
        // `/0` WARNS ("division by zero") and yields the left-hand side (`7/0`
        // = 7, `0/0` = 0); `.rept 7/0` therefore expands 7 times with rc 0.
        // `MIN/-1` would panic Rust in every profile; GAS's bignum result
        // -2^63 has the i64::MIN pattern.
        "/" => {
            if r == 0 {
                l
            } else if r == -1 {
                l.wrapping_neg()
            } else {
                l / r
            }
        }
        // Same law: `%0` warns and yields 0 (`7%0` = 0); `x % -1` is 0 for
        // every x, and Rust would panic on `MIN % -1`.
        "%" => {
            if r == 0 || r == -1 {
                0
            } else {
                l % r
            }
        }
        _ => unreachable!("operator {op} is not in PRECEDENCE"),
    }
}

/// The integer law of every unary operator.
fn int_unary(op: char, v: i64) -> i64 {
    match op {
        // wrapping: `-(1<<63)` must be i64::MIN (GAS keeps the two's-
        // complement pattern) instead of a debug-profile panic.
        '-' => v.wrapping_neg(),
        '+' => v,
        '~' => !v,
        '!' => i64::from(v == 0),
        _ => unreachable!("unary operator {op}"),
    }
}

impl ExprDomain for i64 {
    fn num(v: i64) -> Self {
        v
    }
    fn binary(self, op: &'static str, rhs: Self) -> Result<Self, String> {
        Ok(int_binary(op, self, rhs))
    }
    fn unary(self, op: char) -> Result<Self, String> {
        Ok(int_unary(op, self))
    }
}

/// Recursive-descent evaluation of a token stream in domain `V`; `sym`
/// resolves [`ExprToken::Sym`] (never produced for the numeric domain).
struct ExprParser<'a, V, F: FnMut(&str) -> Result<V, String>> {
    tokens: &'a [ExprToken],
    pos: usize,
    sym: F,
}

impl<V: ExprDomain, F: FnMut(&str) -> Result<V, String>> ExprParser<'_, V, F> {
    fn expr(&mut self) -> Result<V, String> {
        self.level(0)
    }

    fn peek_op(&self, ops: &[&'static str]) -> Option<&'static str> {
        let spelled: &str = match self.tokens.get(self.pos)? {
            ExprToken::Op(c) => match c {
                '|' => "|",
                '^' => "^",
                '&' => "&",
                '<' => "<",
                '>' => ">",
                '+' => "+",
                '-' => "-",
                '*' => "*",
                '/' => "/",
                '%' => "%",
                _ => return None,
            },
            ExprToken::Op2(op) => op,
            _ => return None,
        };
        ops.iter().copied().find(|&op| op == spelled)
    }

    fn level(&mut self, level: usize) -> Result<V, String> {
        let Some(ops) = PRECEDENCE.get(level) else {
            return self.unary();
        };
        let mut val = self.level(level + 1)?;
        while let Some(op) = self.peek_op(ops) {
            self.pos += 1;
            let rhs = self.level(level + 1)?;
            val = val.binary(op, rhs)?;
        }
        Ok(val)
    }

    fn unary(&mut self) -> Result<V, String> {
        let Some(token) = self.tokens.get(self.pos) else {
            return Err("unexpected end of expression".to_string());
        };
        match token {
            ExprToken::Op(op @ ('-' | '+' | '~' | '!')) => {
                let op = *op;
                self.pos += 1;
                self.unary()?.unary(op)
            }
            ExprToken::Op('(') => {
                self.pos += 1;
                let val = self.expr()?;
                if matches!(self.tokens.get(self.pos), Some(ExprToken::Op(')'))) {
                    self.pos += 1;
                } else {
                    return Err("missing closing parenthesis".to_string());
                }
                Ok(val)
            }
            ExprToken::Num(v) => {
                let v = *v;
                self.pos += 1;
                Ok(V::num(v))
            }
            ExprToken::Sym(name) => {
                let name = name.clone();
                self.pos += 1;
                (self.sym)(&name)
            }
            other => Err(format!("unexpected token in expression: {:?}", other)),
        }
    }
}

/// Evaluate a whole token stream; trailing tokens are an error.
fn eval_all<V: ExprDomain>(
    tokens: &[ExprToken],
    sym: impl FnMut(&str) -> Result<V, String>,
) -> Result<V, String> {
    if tokens.is_empty() {
        return Err("empty expression".to_string());
    }
    let mut parser = ExprParser {
        tokens,
        pos: 0,
        sym,
    };
    let val = parser.expr()?;
    if parser.pos < tokens.len() {
        return Err(format!(
            "unexpected trailing token in expression: {:?}",
            tokens[parser.pos]
        ));
    }
    Ok(val)
}

// ── section-relative expressions ───────────────────────────────────────────

/// A GAS expression value: `constant + sum(coeff * base(section))`.
///
/// This is the assembler's operand-section algebra (GAS `expr.c`): a label
/// is its section's base plus an offset, `+`/`-` combine terms, so the
/// difference of two labels in one section is absolute, `*` scales terms by
/// an absolute factor, and every other operator needs absolute operands.
/// Sections are opaque keys chosen by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionExpr<K> {
    pub constant: i64,
    terms: Vec<(K, i64)>,
}

impl<K: Clone + PartialEq> SectionExpr<K> {
    pub fn absolute(v: i64) -> Self {
        Self {
            constant: v,
            terms: Vec::new(),
        }
    }

    /// `offset` bytes into `section`.
    pub fn at(section: K, offset: i64) -> Self {
        Self {
            constant: offset,
            terms: vec![(section, 1)],
        }
    }

    /// `coeff * base(key)`: one symbolic term with no constant.
    pub fn term(key: K, coeff: i64) -> Self {
        Self::absolute(0).add_scaled(
            Self {
                constant: 0,
                terms: vec![(key, 1)],
            },
            coeff,
        )
    }

    pub fn is_absolute(&self) -> bool {
        self.terms.is_empty()
    }

    /// The non-zero `(base, coefficient)` terms, in first-seen order.
    pub fn terms(&self) -> &[(K, i64)] {
        &self.terms
    }

    /// `self + sign * rhs`, cancelling terms whose coefficients sum to 0.
    pub fn add_scaled(mut self, rhs: Self, sign: i64) -> Self {
        self.constant = self.constant.wrapping_add(rhs.constant.wrapping_mul(sign));
        for (k, c) in rhs.terms {
            match self.terms.iter_mut().find(|(sk, _)| *sk == k) {
                Some((_, sc)) => *sc = sc.wrapping_add(c.wrapping_mul(sign)),
                None => self.terms.push((k, c.wrapping_mul(sign))),
            }
        }
        self.terms.retain(|&(_, c)| c != 0);
        self
    }

    pub fn scale(mut self, factor: i64) -> Self {
        self.constant = self.constant.wrapping_mul(factor);
        for (_, c) in &mut self.terms {
            *c = c.wrapping_mul(factor);
        }
        self.terms.retain(|&(_, c)| c != 0);
        self
    }
}

impl<K: Clone + PartialEq> ExprDomain for SectionExpr<K> {
    fn num(v: i64) -> Self {
        Self::absolute(v)
    }
    fn binary(self, op: &'static str, rhs: Self) -> Result<Self, String> {
        match op {
            "+" => Ok(self.add_scaled(rhs, 1)),
            "-" => Ok(self.add_scaled(rhs, -1)),
            "*" if rhs.is_absolute() => Ok(self.scale(rhs.constant)),
            "*" if self.is_absolute() => Ok(rhs.scale(self.constant)),
            _ if self.is_absolute() && rhs.is_absolute() => {
                Ok(Self::absolute(int_binary(op, self.constant, rhs.constant)))
            }
            _ => Err(format!(
                "operator `{op}` needs absolute operands (a symbol's address is not known until link time)"
            )),
        }
    }
    fn unary(self, op: char) -> Result<Self, String> {
        match op {
            '+' => Ok(self),
            '-' => Ok(self.scale(-1)),
            _ if self.is_absolute() => Ok(Self::absolute(int_unary(op, self.constant))),
            _ => Err(format!("operator `{op}` needs an absolute operand")),
        }
    }
}

/// Evaluate `s` with symbols: `resolve` maps each symbol (and `.`, the
/// location counter) to its section-relative value, or errors. Integer
/// literals, precedence and every operator law are exactly those of
/// [`parse_integer_expr`]; only the value domain differs.
pub fn eval_section_expr<K: Clone + PartialEq>(
    s: &str,
    resolve: impl FnMut(&str) -> Result<SectionExpr<K>, String>,
) -> Result<SectionExpr<K>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty expression".to_string());
    }
    let tokens = tokenize_expr_mode(s, true)?;
    eval_all(&tokens, resolve)
}

/// The symbol names `s` mentions, in order (`.` included), or the
/// tokenizer's error.
pub fn expr_symbols(s: &str) -> Result<Vec<String>, String> {
    Ok(tokenize_expr_mode(s, true)?
        .into_iter()
        .filter_map(|t| match t {
            ExprToken::Sym(n) => Some(n),
            _ => None,
        })
        .collect())
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
    let s = trim_data_arg(s);
    if s.is_empty() {
        return Err("empty integer".to_string());
    }

    // Character literal: 'c', '\n', or unclosed GAS 'c (including 'u/'l/' ').
    // Must run BEFORE C integer suffix stripping so `'u`/`'l`/`'z` aren't truncated.
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
            i += std::str::from_utf8(&bytes[i..])
                .ok()
                .and_then(|rest| rest.chars().next())
                .map_or(1, char::len_utf8);
        }
        if i < bytes.len() && bytes[i] == b'\'' {
            i += 1;
        }
        if i == bytes.len() {
            return Ok(val);
        }
        return Err("character literal has trailing content".to_string());
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
    let s = trim_data_arg(s);
    if s.is_empty() {
        return Err("empty integer".to_string());
    }
    // Fast path: try simple integer first
    if let Ok(val) = parse_single_integer(s) {
        return Ok(val);
    }
    let tokens = tokenize_expr(s)?;
    eval_all(&tokens, |name| {
        Err::<i64, _>(format!("unexpected symbol '{name}' in integer expression"))
    })
}

// ── alignment directive tail ───────────────────────────────────────────────

/// The `[, fill[, max]]` tail of a GAS alignment directive, with the unit of
/// the leading operand left to the caller (`.p2align` takes an exponent,
/// `.balign` a byte count, and aarch64 `.align` a byte count again while
/// RISC-V `.align` is exponent-form — three units, one tail grammar).
///
/// This lives here, shared, because the tail is where two backends that each
/// grew their own parser had already diverged: one honored fill and cap, the
/// other silently dropped both, so the same `.s` source assembled to different
/// bytes (and therefore different addresses for every later label) depending on
/// which target was being built. The rules below are the `read.c` ones,
/// re-measured against GNU as 2.47.20260726 on aarch64-linux-gnu and
/// riscv64-linux-gnu:
///
/// * `fill` is any integer, stored as its LOW BYTE (`-1` -> `0xff`,
///   `300` -> `0x2c`, `256` -> `0x00`); an empty slot (`.p2align 3,,5`) means
///   "no explicit fill", which is what switches on the target's NOP law.
/// * `max` is a padding cap in bytes: the directive is skipped when honoring
///   it would insert MORE than `max`. `0` and a negative value mean unlimited
///   (so `.p2align 4,,0` still pads), and the comparison is against the
///   boundary padding, before any target-specific growth of that run.
/// * More operands than that (`.balign 8,,, -1`) is an error, not a suffix to
///   ignore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlignTail {
    pub fill: Option<u8>,
    pub max_pad: Option<u64>,
}

/// Why a `[, fill[, max]]` tail was rejected.
///
/// A type rather than a message string because a target may legitimately ignore
/// one operand and still have to report the other.  RISC-V's `.balign` forwards
/// `BadFill`, `BadMax` and `TooMany` are all real rejections: `s_align` reads
/// the max field with `get_absolute_expression()` before any target hook runs,
/// so `.balign 8,,junk` is an error on riscv64 and on aarch64 alike (measured on
/// as 2.47.20260726).  Swallowing `BadMax` on RISC-V would rest on the theory
/// that its `.p2align` handler ignores the cap -- but "ignored when the padding is
/// emitted" and "never parsed" are different claims, and the second one is
/// false.  Triage by `e.contains("fill")` would let a reworded message silently
/// change which inputs the assembler accepts; the variants are the contract and
/// [`AlignTailError::to_message`] spells them the way GAS does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlignTailError {
    /// The fill slot is present but is not an integer.
    BadFill { directive: String, token: String },
    /// The max-pad slot is present but is not an integer.
    BadMax { directive: String, token: String },
    /// More operands than `fill` and `max`.
    TooMany { directive: String },
}

impl AlignTailError {
    /// The user-facing text.  Byte-identical to the string this parser returned
    /// before the type existed, so every test of the wording keeps pinning it.
    pub fn to_message(&self) -> String {
        match self {
            Self::BadFill { directive, token } => {
                format!("{directive}: fill `{token}` is not an integer")
            }
            Self::BadMax { directive, token } => {
                format!("{directive}: max-pad `{token}` is not an integer")
            }
            Self::TooMany { directive } => format!("{directive}: too many operands"),
        }
    }
}

impl core::fmt::Display for AlignTailError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_message())
    }
}

/// Parse the tail after the alignment operand. `integer` is the caller's own
/// expression evaluator, because each backend resolves symbols and different
/// radix forms through its own path.
///
/// An absent, empty, zero or negative `max` all mean *no cap* -- GAS only
/// consults the value when it is strictly positive, measured on
/// riscv64-linux-gnu-as, where `.p2align 4,0x13,0` still pads the full 15 bytes.
/// The `v <= 0` arm below therefore must not be "corrected" into `Some(0)`: that
/// would turn a no-op into a suppressed alignment on valid input, in every
/// backend that shares this parser (x86, arm, riscv, and the `.balign`/`.p2align`
/// spellings alike).
///
/// `TooMany` is checked last, after both operands have been evaluated, so a
/// directive with a garbage third operand *and* a fourth reports the garbage
/// rather than the arity. Both are hard errors -- nothing is accepted silently --
/// and the ordering is deliberate: an unparseable operand is the more specific
/// diagnosis, and leading with the arity complaint would hide it.
pub fn parse_align_tail<F>(
    rest: &str,
    directive: &str,
    mut integer: F,
) -> Result<AlignTail, AlignTailError>
where
    F: FnMut(&str) -> Result<i64, String>,
{
    let mut parts = rest.split(',');
    let d = || directive.to_string();
    let fill = match parts.next().map(str::trim) {
        None | Some("") => None,
        Some(tok) => {
            let v = integer(tok).map_err(|_| AlignTailError::BadFill {
                directive: d(),
                token: tok.to_string(),
            })?;
            // GAS stores the low byte of any integer (see the type docs).
            Some(v as u8)
        }
    };
    let max_pad = match parts.next().map(str::trim) {
        None | Some("") => None,
        Some(tok) => {
            let v = integer(tok).map_err(|_| AlignTailError::BadMax {
                directive: d(),
                token: tok.to_string(),
            })?;
            if v <= 0 { None } else { Some(v as u64) }
        }
    };
    if parts.next().is_some() {
        return Err(AlignTailError::TooMany { directive: d() });
    }
    Ok(AlignTail { fill, max_pad })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Numeric local-label references are whole tokens only: the three
    /// per-target copies this replaced each read some of these wrong
    /// (`0x1f` as `0x` + `1f`, `x1b` as `x` + `1b`, `0b` as a binary prefix,
    /// `1B`/`1F` as references although GAS knows only lowercase).
    #[test]
    fn numeric_label_refs_are_whole_tokens() {
        let rename = |e: &str| rewrite_numeric_label_refs(e, |r| Some(format!("<{r}>")));
        assert_eq!(rename("2b + 1 - 1b"), "<2b> + 1 - <1b>");
        assert_eq!(rename("(3f-2b)>>2"), "(<3f>-<2b>)>>2");
        assert_eq!(rename("0b"), "<0b>");
        assert_eq!(rename("0b1 + 0x1f + 0x1b"), "0b1 + 0x1f + 0x1b");
        assert_eq!(rename("x1b - 1b"), "x1b - <1b>");
        assert_eq!(rename(".L1f + 1fx + 1B + 1F"), ".L1f + 1fx + 1B + 1F");
        assert_eq!(rename("10f"), "<10f>");
        // An unresolvable reference is left as written.
        assert_eq!(rewrite_numeric_label_refs("1b", |_| None), "1b");
    }

    #[test]
    fn numeric_label_refs_are_symbols_in_expressions() {
        assert_eq!(
            expr_symbols("(2b - 1b) * 4 + x1 - .").unwrap(),
            ["2b", "1b", "x1", "."]
        );
        assert_eq!(expr_symbols("0b1 + 0x1f").unwrap(), Vec::<String>::new());
        let v =
            eval_section_expr::<String>("2f - 1b + 3", |n| Ok(SectionExpr::term(n.to_string(), 1)))
                .unwrap();
        assert_eq!(v.terms(), [("2f".to_string(), 1), ("1b".to_string(), -1)]);
        assert_eq!(v.constant, 3);
        // Without symbols allowed, `1b` is not a number.
        assert!(parse_integer_expr("1b").is_err());
        assert_eq!(parse_integer_expr("0b11"), Ok(3));
    }

    /// One grammar, two domains: on constant input the section evaluator
    /// must agree with `parse_integer_expr` bit for bit -- precedence, GAS's
    /// -1-is-true comparisons, `/0`, `%-1`, shift ranges, wrapping.
    #[test]
    fn section_domain_matches_integer_domain_on_constants() {
        for e in [
            "1+2*3",
            "(1+2)*3",
            "1<<3+1",
            "7/0",
            "0/0",
            "7%0",
            "5%-1",
            "1<<64",
            "1>>-1",
            "-9223372036854775807-1",
            "(-9223372036854775807-1)/-1",
            "0<1",
            "2==2",
            "1<>0",
            "3&&0",
            "3||0",
            "!5",
            "!0",
            "~0",
            "6&3|8^1",
            "0x10-0b11",
            "'a'+1",
            "1 < 2 + 3",
            "10-3-2",
            "2*3%4",
            "-(1<<63)",
        ] {
            let int = parse_integer_expr(e).unwrap_or_else(|err| panic!("{e}: {err}"));
            let sec = eval_section_expr::<u8>(e, |n| Err(format!("no symbol {n}")))
                .unwrap_or_else(|err| panic!("{e}: {err}"));
            assert!(sec.is_absolute(), "{e}");
            assert_eq!(sec.constant, int, "{e}");
        }
    }

    /// GAS operand-section algebra: labels of one section cancel in a
    /// difference, scale by absolute factors, and any other operator on a
    /// relocatable operand is refused rather than folded.
    #[test]
    fn section_algebra_cancels_scales_and_refuses() {
        let resolve = |n: &str| -> Result<SectionExpr<u8>, String> {
            match n {
                "a" => Ok(SectionExpr::at(1, 4)),
                "b" => Ok(SectionExpr::at(1, 20)),
                "c" => Ok(SectionExpr::at(2, 8)),
                _ => Err(format!("unknown {n}")),
            }
        };
        let v = eval_section_expr("b - a", resolve).unwrap();
        assert!(v.is_absolute() && v.constant == 16);
        let v = eval_section_expr("(b - a) * 2 + (c - c)", resolve).unwrap();
        assert!(v.is_absolute() && v.constant == 32);
        let v = eval_section_expr("2 * (b - a) - (b - a)", resolve).unwrap();
        assert!(v.is_absolute() && v.constant == 16);
        assert!(!eval_section_expr("a + 4", resolve).unwrap().is_absolute());
        assert!(!eval_section_expr("c - a", resolve).unwrap().is_absolute());
        assert!(
            !eval_section_expr("a * 2 - a", resolve)
                .unwrap()
                .is_absolute()
        );
        for e in ["a & 1", "a / 2", "~a", "a << 1", "a == b", "a * c"] {
            assert!(
                eval_section_expr(e, resolve).is_err(),
                "{e} must be refused"
            );
        }
        assert!(eval_section_expr("d", resolve).is_err());
        // The integer evaluator still rejects identifiers at the lexer.
        assert!(parse_integer_expr("a + 1").is_err());
    }

    /// GNU as 2.47 (measured): a decimal literal in 2^63..2^64-1 is the
    /// two's-complement pattern (negative as i64); anything ABOVE u64::MAX
    /// is an expression error, not a wrap to some other value.
    #[test]
    fn gas247_literal_range_edges() {
        assert_eq!(
            parse_integer_expr("18446744073709551615").ok(),
            Some(-1),
            "u64max is the all-ones pattern"
        );
        assert_eq!(
            parse_integer_expr("9223372036854775808").ok(),
            Some(i64::MIN),
            "2^63 is its two's-complement pattern"
        );
        assert_eq!(
            parse_integer_expr("0xffffffffffffffff").ok(),
            Some(-1),
            "hex u64max likewise"
        );
        assert!(
            parse_integer_expr("18446744073709551616").is_err(),
            "2^64 must be an expression error, got {:?}",
            parse_integer_expr("18446744073709551616")
        );
    }

    /// test the GAS comparison semantics
    #[test]
    fn test_gas_comparisons() {
        // TRUE is -1 (all ones), FALSE is 0 — GAS/shell convention.
        assert_eq!(parse_integer_expr("0 < 1").unwrap(), -1);
        assert_eq!(parse_integer_expr("1 < 0").unwrap(), 0);
        assert_eq!(parse_integer_expr("0 <= 0").unwrap(), -1);
        assert_eq!(parse_integer_expr("0 >= 1").unwrap(), 0);
        assert_eq!(parse_integer_expr("1 > 0").unwrap(), -1);
        assert_eq!(parse_integer_expr("0 > 1").unwrap(), 0);
        assert_eq!(parse_integer_expr("0 == 0").unwrap(), -1);
        assert_eq!(parse_integer_expr("0 == 1").unwrap(), 0);
        assert_eq!(parse_integer_expr("1 != 0").unwrap(), -1);
        assert_eq!(parse_integer_expr("1 <> 0").unwrap(), -1);
        assert_eq!(parse_integer_expr("0 != 0").unwrap(), 0);
        // Logical connectives share the -1/0 convention.
        assert_eq!(parse_integer_expr("1 && 2").unwrap(), -1);
        assert_eq!(parse_integer_expr("1 && 0").unwrap(), 0);
        assert_eq!(parse_integer_expr("0 || 0").unwrap(), 0);
        assert_eq!(parse_integer_expr("0 || 3").unwrap(), -1);
        // Precedence: comparison binds TIGHTER than & and |, looser than
        // shift, and +/- sit between shift and comparison.
        assert_eq!(parse_integer_expr("(1 < 2) + (3 < 4)").unwrap(), -2);
        assert_eq!(parse_integer_expr("1 < 2 == 3 < 4").unwrap(), -1);
        assert_eq!(parse_integer_expr("4 > 1 << 1").unwrap(), -1);
        assert_eq!(parse_integer_expr("2 + 2 == 4").unwrap(), -1);
        // A shift still beats a comparison: 1 < (2 << 3).
        assert_eq!(parse_integer_expr("1 < 2 << 3").unwrap(), -1);
        assert_eq!(parse_integer_expr("16 < 1 << 3").unwrap(), 0);
    }

    #[test]
    fn gas247_shift_out_of_range_is_zero() {
        // GNU as 2.47 (measured): a shift count outside 0..64 warns
        // ("shift count out of range (64 is not between 0 and 63)") and
        // yields 0.  Rust's masked shift (1 << 64 == 1) made `.rept
        // 1<<64` expand once instead of zero times, and `1 << -1`
        // panicked the assembler instead of yielding a value.
        assert_eq!(parse_integer_expr("1 << 64").unwrap(), 0);
        assert_eq!(parse_integer_expr("1 << 65").unwrap(), 0);
        assert_eq!(parse_integer_expr("1 << -1").unwrap(), 0);
        assert_eq!(parse_integer_expr("1 >> 64").unwrap(), 0);
        assert_eq!(parse_integer_expr("8 >> -2").unwrap(), 0);
        // In-range counts keep the hardware result.
        assert_eq!(parse_integer_expr("1 << 63").unwrap(), i64::MIN);
        assert_eq!(parse_integer_expr("(1 << 62) >> 62").unwrap(), 1);
        assert_eq!(parse_integer_expr("255 >> 4").unwrap(), 15);
    }

    #[test]
    fn gas247_divmod_zero_and_wrap_law() {
        // GNU as 2.47 (measured): div/mod by zero WARN and yield the
        // left-hand side / 0 (`7/0`=7, `7%0`=0, `0/0`=0); `.rept 7/0`
        // expands 7 times with rc 0, so hard-erroring on a zero
        // divisor rejected input GAS accepts.  Overflowing +/-/* and
        // `MIN/-1` keep the bignum low bits in every profile.
        assert_eq!(parse_integer_expr("7 / 0").unwrap(), 7);
        assert_eq!(parse_integer_expr("7 % 0").unwrap(), 0);
        assert_eq!(parse_integer_expr("0 / 0").unwrap(), 0);
        assert_eq!(parse_integer_expr("(1 << 63) / -1").unwrap(), i64::MIN);
        assert_eq!(parse_integer_expr("(1 << 63) % -1").unwrap(), 0);
        assert_eq!(parse_integer_expr("(1 << 62) * 4").unwrap(), 0);
        assert_eq!(parse_integer_expr("(1 << 63) - 1").unwrap(), i64::MAX);
        assert_eq!(parse_integer_expr("-(1 << 63)").unwrap(), i64::MIN);
        // Truncated sign law, measured: -7/3 = -2, -7%3 = -1, 7%-3 = 1.
        assert_eq!(parse_integer_expr("(-7) / 3").unwrap(), -2);
        assert_eq!(parse_integer_expr("(-7) % 3").unwrap(), -1);
        assert_eq!(parse_integer_expr("7 % (-3)").unwrap(), 1);
    }

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

    /// The `[, fill[, max]]` grammar, measured against GNU as 2.47 on the two
    /// pinned targets.  These are the rules every front end shares -- low-byte
    /// fill, empty slot means "absent", `max <= 0` means unlimited, one operand
    /// too many is an error -- so they belong here rather than being rediscovered
    /// per target.  The `max <= 0` rule and the low-byte truncation in particular
    /// are invisible from a codegen diff alone: a target that clamped the fill
    /// instead of truncating it still assembles, and only the pad *content*
    /// differs.
    ///
    /// `rest` is what the caller's split left after the alignment operand, so the
    /// first slot here is the fill and the second is the cap: `".p2align 4,3,8"`
    /// reaches this function as `"3,8"`.
    #[test]
    fn gas247_align_tail_grammar() {
        let tail = |t: &str| parse_align_tail(t, ".p2align", parse_integer_expr);
        assert_eq!(
            tail("").unwrap(),
            AlignTail {
                fill: None,
                max_pad: None
            },
            "a bare directive has no explicit fill and no cap"
        );
        assert_eq!(
            tail("3").unwrap(),
            AlignTail {
                fill: Some(3),
                max_pad: None
            }
        );
        // The fill is the LOW BYTE of any integer, not a range check.
        for (src, want) in [
            ("-1", 0xffu8),
            ("300", 0x2c),
            ("256", 0x00),
            ("0x1ff", 0xff),
        ] {
            assert_eq!(
                tail(src).unwrap().fill,
                Some(want),
                "fill {src} is the low byte of the expression"
            );
        }
        // An empty slot is absent, not zero, and does not consume the next field.
        assert_eq!(
            tail(",").unwrap(),
            AlignTail {
                fill: None,
                max_pad: None
            }
        );
        assert_eq!(tail("-1,").unwrap().fill, Some(0xff));
        // `max` is unlimited at 0 and below -- `.p2align 4,,0` still pads.
        for src in ["0", "-1", "-4096", ""] {
            assert_eq!(
                tail(&format!(",{src}")).unwrap().max_pad,
                None,
                "max {src:?} must mean unlimited"
            );
        }
        assert_eq!(tail(",2048").unwrap().max_pad, Some(2048));
        assert_eq!(
            tail(",18446744073709551615").unwrap().max_pad,
            None,
            "u64max is the all-ones pattern, i.e. negative as i64, i.e. unlimited"
        );
        // The three verdicts, each with the exact text a target forwards.
        assert_eq!(
            tail("junk").unwrap_err().to_message(),
            ".p2align: fill `junk` is not an integer"
        );
        assert_eq!(
            tail(",junk").unwrap_err().to_message(),
            ".p2align: max-pad `junk` is not an integer"
        );
        assert_eq!(
            tail(",1,2").unwrap_err().to_message(),
            ".p2align: too many operands"
        );
        assert_eq!(
            tail(",,1").unwrap_err().to_message(),
            ".p2align: too many operands",
            "three slots is a fourth operand on the directive, even when empty"
        );
        // The variants, not just the text: a target that ignores one verdict has
        // to be able to name it.
        assert!(matches!(
            tail("junk").unwrap_err(),
            AlignTailError::BadFill { .. }
        ));
        assert!(matches!(
            tail(",junk").unwrap_err(),
            AlignTailError::BadMax { .. }
        ));
        assert!(matches!(
            tail(",1,2").unwrap_err(),
            AlignTailError::TooMany { .. }
        ));
        // `Display` and `to_message` are one text, so a target that renames the
        // directive in its diagnostic cannot diverge between the two paths.
        let e = tail("junk").unwrap_err();
        assert_eq!(e.to_string(), e.to_message());
        // The label follows the caller's directive name: the same tail parsed for
        // `.balign` blames `.balign`.
        assert_eq!(
            parse_align_tail("junk", ".balign", parse_integer_expr)
                .unwrap_err()
                .to_message(),
            ".balign: fill `junk` is not an integer"
        );
    }

    #[test]
    fn gas_char_literals_split_and_eval_cleanly() {
        let parts = split_comma_args("'A', ' ', ',', ';', '#', 'u, 'l, ' ");
        let vals: Vec<i64> = parts
            .into_iter()
            .map(|p| parse_integer_expr(trim_data_arg(p)).unwrap())
            .collect();
        assert_eq!(
            vals,
            vec![
                b'A' as i64,
                b' ' as i64,
                b',' as i64,
                b';' as i64,
                b'#' as i64,
                b'u' as i64,
                b'l' as i64,
                b' ' as i64,
            ]
        );
    }
}
