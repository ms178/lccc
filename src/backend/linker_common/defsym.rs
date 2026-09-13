//! `--defsym SYMBOL=EXPRESSION`, classified the way GNU ld classifies it.
//!
//! Three of the four ELF backends had a loop that read `--defsym A=B`, looked `B`
//! up in the symbol table, and copied it to `A` if it was found. That is the alias
//! form only. A constant (`--defsym far=0x1000`) and an arithmetic expression
//! (`--defsym end_of_text=_start+4`) both failed the lookup, and the loop's
//! `if let Some(..)` made the failure silent: the symbol stayed undefined and the
//! link died later with "undefined symbols: far", which points at the reference
//! rather than at the definition the user just gave.
//!
//! Classification therefore happens here, once, and every backend asks the same
//! question, so they cannot disagree about what `--defsym a=b+4` means. The
//! spelling of numbers follows GNU ld, measured rather than assumed:
//!
//! ```text
//! 1G  1T  0b1011  0o17  0x1f  0777  42      accepted
//! (1  1+  1 2  1/0  0x                        rejected
//! ```
//!
//! `1/0` is reported as a division by zero and not as a syntax error, because
//! that is what bfd says and a user who typed it needs to hear which of the two
//! mistakes they made.

/// What a `--defsym` right-hand side turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Defsym {
    /// Another symbol in this link: the definition copies its address. This is
    /// the form the backends already supported, and the only one that can be
    /// resolved before layout.
    Alias(String),
    /// A numeric constant: an absolute symbol with this value.
    Constant(u64),
    /// An arithmetic expression over symbols and constants. Its value is not
    /// known until addresses are final, so the caller decides when to evaluate
    /// it; see [`eval_with_symbols`].
    Expression(String),
}

/// Why a `--defsym` right-hand side could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefsymError {
    /// The text is not a number, not a symbol and not a well-formed expression.
    Syntax(String),
    /// An expression divided by zero. GNU ld reports this distinctly from a
    /// syntax error, and so do we.
    DivByZero,
    /// An expression named a symbol the link does not define.
    UndefinedSymbol(String),
    /// The expression is well formed but this backend has no point at which it
    /// can evaluate it (expressions need final addresses; a backend that applies
    /// `--defsym` before layout can only handle aliases and constants).
    NeedsLayout(String),
}

impl DefsymError {
    /// The user-facing text.
    pub fn message(&self) -> String {
        match self {
            DefsymError::Syntax(what) => format!("syntax error in --defsym expression: {what}"),
            DefsymError::DivByZero => "/ by zero in --defsym expression".to_string(),
            DefsymError::UndefinedSymbol(name) => {
                format!("undefined symbol '{name}' referenced in --defsym expression")
            }
            DefsymError::NeedsLayout(expr) => format!(
                "--defsym expression '{expr}' needs final symbol addresses, which this \
                 link path does not have; use a constant or a symbol alias"
            ),
        }
    }
}

/// Parse one GNU ld number token: decimal, `0x`/`0b`/`0o` with either case,
/// traditional leading-zero octal, and a binary scale suffix.
///
/// The suffixes multiply by powers of 1024, not 1000 -- `1K` is 1024 in every
/// assembler and linker in common use, and a linker that quietly meant 1000 would
/// place a symbol 24 bytes off with no way for the user to see why.
pub fn parse_number(token: &str) -> Option<u64> {
    let t = token.trim();
    if t.is_empty() {
        return None;
    }
    // A number token carries no sign. `u64::from_str_radix` would happily read
    // "+1" as 1, and then `--defsym x=+1` would classify as a constant while
    // `--defsym x=-1` classified as an expression -- two spellings of the same
    // idea taking different code paths. Signs are the expression grammar's job
    // (see `eval_unary`), which handles both uniformly.
    if t.starts_with('+') || t.starts_with('-') {
        return None;
    }

    // The scale suffix comes off first. It says nothing about the radix, but the
    // radix detection has to see the digits it applies to: bfd reads `0777M` as
    // 511 MiB, and the only way to get that is to recognise traditional octal
    // after the suffix is gone. Detecting the radix first and stripping the
    // suffix afterwards reads `0777M` as 777 MiB -- not an error, a silently
    // wrong address, which is the worst outcome available here.
    //
    // K and M, either case, multiplying by 1024 and 1024^2. Measured with
    // /usr/bin/ld 2.44 (GNU Binutils for Debian) by linking with --defsym and
    // reading the value back out of the output symbol table:
    //
    //   1K -> 0x400        1m -> 0x100000     0x10K -> 0x4000
    //   0777M -> 0x1ff00000      1025K -> 0x100400   (exact below u64::MAX)
    //   4G, 1T, 1P, 1E, 1Z -> syntax error (no such suffixes)
    //   0b1011K -> syntax error        0o17K -> syntax error
    //
    // Accepting G or T would make lccc-ld laxer than the linker it replaces: the
    // build links here and is refused by GNU ld, which is the one direction a
    // compatibility divergence must not go.
    let (body, mul) = match t.as_bytes().last() {
        Some(&c) if t.len() > 1 => match c.to_ascii_uppercase() {
            b'K' => (&t[..t.len() - 1], 1024u64),
            b'M' => (&t[..t.len() - 1], 1024 * 1024),
            _ => (t, 1),
        },
        _ => (t, 1),
    };
    // The suffix belongs to the decimal, hexadecimal and traditional-octal
    // spellings only; on an explicitly prefixed binary or octal literal bfd
    // reports a syntax error, so so do we.
    if mul != 1
        && (body.starts_with("0b")
            || body.starts_with("0B")
            || body.starts_with("0o")
            || body.starts_with("0O"))
    {
        return None;
    }
    let (digits, radix, traditional_octal) = if let Some(d) =
        body.strip_prefix("0x").or_else(|| body.strip_prefix("0X"))
    {
        (d, 16, false)
    } else if let Some(d) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
        (d, 2, false)
    } else if let Some(d) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
        (d, 8, false)
    } else if body.len() > 1 && body.starts_with('0') && body.bytes().all(|b| b.is_ascii_digit()) {
        // Traditional octal: a leading zero with decimal digits. "09" is not
        // a valid octal number but is unambiguously meant as nine, so a
        // failed octal scan falls back to decimal below.
        (&body[1..], 8, true)
    } else {
        (body, 10, false)
    };
    if digits.is_empty() {
        return None;
    }
    // Overflow saturates at u64::MAX rather than being rejected or wrapped,
    // because that is what bfd does, measured: --defsym s=18446744073709551616,
    // s=0x10000000000000000 and s=99999999999999999999999999 all link, and all
    // three place the symbol at 0xffffffffffffffff. Wrapping would place it at
    // 0 -- a plausible address, therefore a silent miscompile -- and rejecting
    // would be stricter than the linker being replaced.
    let v = scan_saturating(digits, radix).or_else(|| {
        if traditional_octal {
            scan_saturating(body, 10)
        } else {
            None
        }
    })?;
    Some(v.saturating_mul(mul))
}

/// Scan digits in `radix`, saturating at `u64::MAX` on overflow.
///
/// A hand-rolled scan instead of `u64::from_str_radix` because the two failure
/// modes must stay distinguishable: an invalid digit means "this is not a
/// number", while overflow means "this number is larger than an address", and
/// bfd's answer to the second is `u64::MAX`, not an error.
fn scan_saturating(digits: &str, radix: u32) -> Option<u64> {
    if digits.is_empty() {
        return None;
    }
    let mut acc: u64 = 0;
    for b in digits.bytes() {
        let d = u64::from((b as char).to_digit(radix)?);
        acc = acc.saturating_mul(u64::from(radix)).saturating_add(d);
    }
    Some(acc)
}

/// True for a token that can only be a symbol name: it is not a number, and it is
/// made of characters a symbol may contain.
fn is_symbol_token(t: &str) -> bool {
    !t.is_empty()
        && t.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'$' || b >= 0x80)
        && !t.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && parse_number(t).is_none()
}

/// Classify the right-hand side of `--defsym SYMBOL=EXPR`.
///
/// `is_defined` answers whether a name refers to a symbol this link has; it is
/// what separates `--defsym alias=real_function` from `--defsym x=nosuchthing`,
/// and the second one has to be an error rather than a silently dropped alias.
pub fn classify(expr: &str, is_defined: impl Fn(&str) -> bool) -> Result<Defsym, DefsymError> {
    let e = expr.trim();
    if e.is_empty() {
        return Err(DefsymError::Syntax("empty expression".to_string()));
    }
    if let Some(v) = parse_number(e) {
        return Ok(Defsym::Constant(v));
    }
    if is_symbol_token(e) {
        return if is_defined(e) {
            Ok(Defsym::Alias(e.to_string()))
        } else {
            Err(DefsymError::UndefinedSymbol(e.to_string()))
        };
    }
    // Anything else has to be an expression. Validate it here rather than at
    // evaluation time so that a typo is reported even on a link path that would
    // never have got as far as evaluating it.
    let mut ctx = Ctx {
        s: e.as_bytes(),
        i: 0,
    };
    ctx.expr()?;
    ctx.skip_ws();
    if ctx.i != ctx.s.len() {
        return Err(DefsymError::Syntax(format!(
            "unexpected '{}' at offset {}",
            ctx.s[ctx.i] as char, ctx.i
        )));
    }
    Ok(Defsym::Expression(e.to_string()))
}

/// Evaluate a constant expression (no symbols).
pub fn eval(expr: &str) -> Result<u64, DefsymError> {
    eval_with_symbols(expr, &|_| None)
}

/// Evaluate an expression, resolving symbol names through `lookup`.
///
/// Arithmetic is unsigned 64-bit and wraps, which is what a linker address
/// expression means: `_end - _start` is a size, and a symbol below another one
/// produces the two's-complement difference rather than an error. Division by zero
/// is the one arithmetic failure that is reported, because no address semantics
/// can excuse it.
pub fn eval_with_symbols(
    expr: &str,
    lookup: &dyn Fn(&str) -> Option<u64>,
) -> Result<u64, DefsymError> {
    let mut ctx = Ctx {
        s: expr.as_bytes(),
        i: 0,
    };
    let v = ctx.eval_expr(lookup)?;
    ctx.skip_ws();
    if ctx.i != ctx.s.len() {
        return Err(DefsymError::Syntax(format!(
            "trailing '{}' at offset {}",
            ctx.s[ctx.i] as char, ctx.i
        )));
    }
    Ok(v)
}

/// A recursive-descent parser over the expression grammar GNU ld accepts:
/// `+ - * / %` with the usual precedence, parentheses, unary minus, numbers and
/// symbols. No allocation per token, no intermediate string.
struct Ctx<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Ctx<'a> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.s.get(self.i).copied()
    }

    /// Syntax-check only, without evaluating: used by [`classify`] so a malformed
    /// expression is rejected on paths that could never evaluate it.
    ///
    /// Every symbol is treated as defined here. Which symbols exist is the
    /// caller's question, and it changes between classification and evaluation --
    /// a link path that classifies before layout cannot answer it yet. Passing a
    /// lookup that always returned `None` instead would report "undefined symbol"
    /// for the first name in every expression, i.e. reject all of them.
    fn expr(&mut self) -> Result<(), DefsymError> {
        self.eval_expr(&|_| Some(0)).map(|_| ())
    }

    fn eval_expr(&mut self, lookup: &dyn Fn(&str) -> Option<u64>) -> Result<u64, DefsymError> {
        let mut lhs = self.eval_term(lookup)?;
        while let Some(op) = self.peek() {
            if op != b'+' && op != b'-' {
                break;
            }
            self.i += 1;
            let rhs = self.eval_term(lookup)?;
            lhs = if op == b'+' {
                lhs.wrapping_add(rhs)
            } else {
                lhs.wrapping_sub(rhs)
            };
        }
        Ok(lhs)
    }

    fn eval_term(&mut self, lookup: &dyn Fn(&str) -> Option<u64>) -> Result<u64, DefsymError> {
        let mut lhs = self.eval_unary(lookup)?;
        while let Some(op) = self.peek() {
            if op != b'*' && op != b'/' && op != b'%' {
                break;
            }
            self.i += 1;
            let rhs = self.eval_unary(lookup)?;
            lhs = match op {
                b'*' => lhs.wrapping_mul(rhs),
                b'/' => {
                    if rhs == 0 {
                        return Err(DefsymError::DivByZero);
                    }
                    lhs / rhs
                }
                _ => {
                    if rhs == 0 {
                        return Err(DefsymError::DivByZero);
                    }
                    lhs % rhs
                }
            };
        }
        Ok(lhs)
    }

    fn eval_unary(&mut self, lookup: &dyn Fn(&str) -> Option<u64>) -> Result<u64, DefsymError> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Ok((self.eval_unary(lookup)?).wrapping_neg())
            }
            Some(b'+') => {
                self.i += 1;
                self.eval_unary(lookup)
            }
            Some(b'~') => {
                self.i += 1;
                Ok(!(self.eval_unary(lookup)?))
            }
            _ => self.eval_atom(lookup),
        }
    }

    fn eval_atom(&mut self, lookup: &dyn Fn(&str) -> Option<u64>) -> Result<u64, DefsymError> {
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let v = self.eval_expr(lookup)?;
                match self.peek() {
                    Some(b')') => self.i += 1,
                    _ => return Err(DefsymError::Syntax("missing ')'".to_string())),
                }
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() => {
                let start = self.i;
                while self.i < self.s.len() {
                    let b = self.s[self.i];
                    if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' {
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                let tok = std::str::from_utf8(&self.s[start..self.i])
                    .map_err(|_| DefsymError::Syntax("non-UTF-8 token".to_string()))?;
                parse_number(tok)
                    .ok_or_else(|| DefsymError::Syntax(format!("'{tok}' is not a number")))
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' || c == b'.' || c == b'$' => {
                let start = self.i;
                while self.i < self.s.len() {
                    let b = self.s[self.i];
                    if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'$' || b >= 0x80
                    {
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                let name = std::str::from_utf8(&self.s[start..self.i])
                    .map_err(|_| DefsymError::Syntax("non-UTF-8 symbol".to_string()))?;
                lookup(name).ok_or_else(|| DefsymError::UndefinedSymbol(name.to_string()))
            }
            Some(c) => Err(DefsymError::Syntax(format!("unexpected '{}'", c as char))),
            None => Err(DefsymError::Syntax(
                "expression ends where a value was expected".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defined(name: &str) -> bool {
        matches!(name, "real" | "_start" | "_end" | "a" | "b")
    }

    #[test]
    fn numbers_follow_the_gnu_ld_spellings() {
        // Every value here was run through /usr/bin/ld 2.44 (GNU Binutils for
        // Debian) with --defsym and read back out of the linked binary's symbol
        // table, so this records measurements rather than intent.
        for (text, want) in [
            ("42", 42u64),
            ("0x1f", 31),
            ("0X1F", 31),
            ("0b1011", 11),
            ("0o17", 15),
            ("0777", 511),
            ("0", 0),
            ("09", 9), // not valid octal; bfd reads it as decimal
            ("1K", 1024),
            ("1k", 1024),
            ("1M", 1024 * 1024),
            ("1m", 1024 * 1024),
            ("0x10K", 0x4000),      // suffix applies after the radix scan
            ("0777M", 0x1ff0_0000), // 511 MiB, exactly what bfd produced
            ("1025K", 0x10_0400),   // exact arithmetic, no rounding
        ] {
            assert_eq!(parse_number(text), Some(want), "{text}");
            assert_eq!(
                classify(text, defined),
                Ok(Defsym::Constant(want)),
                "{text}"
            );
        }
    }

    #[test]
    fn the_suffix_set_is_exactly_k_and_m() {
        // bfd rejects every scale suffix above M: 4G, 1T, 1P, 1E and 1Z are all
        // syntax errors in ld 2.44. Accepting them would be the dangerous kind of
        // divergence -- lccc-ld links the build, GNU ld refuses it, and the
        // failure surfaces on someone else's machine.
        for bad in ["1G", "4G", "1T", "2T", "1P", "1E", "1Z", "1g", "1t"] {
            assert_eq!(parse_number(bad), None, "{bad} must not be a scale suffix");
            assert!(matches!(classify(bad, defined), Err(_)), "{bad}");
        }
        // A suffix on an explicitly prefixed binary or octal literal is a syntax
        // error in bfd, even though the bare literal is fine.
        assert_eq!(parse_number("0b1011K"), None);
        assert_eq!(parse_number("0b1011"), Some(11));
        assert_eq!(parse_number("0o17K"), None);
        assert_eq!(parse_number("0o17"), Some(15));
        // ...while the traditional octal spelling does take one: 0777M is 511 MiB.
        assert_eq!(parse_number("0777M"), Some(0x1ff0_0000));
    }

    #[test]
    fn scale_suffixes_are_1024_not_1000() {
        // A linker that meant 1000 here would place a symbol 24 bytes off with no
        // diagnostic anywhere. bfd says 0x400 for 1K and 0x100000 for 1M.
        assert_eq!(parse_number("1K"), Some(1024));
        assert_eq!(parse_number("1M"), Some(1024 * 1024));
        assert_ne!(parse_number("1K"), Some(1000));
    }

    #[test]
    fn malformed_numbers_are_not_numbers() {
        for bad in ["0x", "0b", "1_0", "1.5", "-", "+1", "-1", "1 2", "0xg"] {
            assert_eq!(parse_number(bad), None, "{bad:?} must not parse");
        }
    }

    #[test]
    fn a_known_symbol_is_an_alias_and_an_unknown_one_is_an_error() {
        assert_eq!(classify("real", defined), Ok(Defsym::Alias("real".into())));
        // The defect this replaces: an unknown name silently produced no
        // definition at all, and the link failed later blaming the reference.
        match classify("nosuchsym", defined) {
            Err(DefsymError::UndefinedSymbol(n)) => assert_eq!(n, "nosuchsym"),
            other => panic!("expected UndefinedSymbol, got {other:?}"),
        }
    }

    #[test]
    fn expressions_are_recognised_and_syntax_checked_at_classify_time() {
        assert!(matches!(
            classify("_start+4", defined),
            Ok(Defsym::Expression(_))
        ));
        assert!(matches!(
            classify("(_end - _start) / 2", defined),
            Ok(Defsym::Expression(_))
        ));
        // Rejected by GNU ld as syntax errors, and rejected here for the same
        // reason rather than being treated as a symbol name.
        for bad in ["(1", "1+", "1 2", "()", "*3", "1++"] {
            assert!(
                matches!(classify(bad, defined), Err(DefsymError::Syntax(_))),
                "{bad:?} must be a syntax error"
            );
        }
    }

    #[test]
    fn evaluation_matches_hand_computed_values() {
        let lookup = |n: &str| match n {
            "_start" => Some(0x401000u64),
            "_end" => Some(0x402000u64),
            "a" => Some(7u64),
            "b" => Some(3u64),
            _ => None,
        };
        assert_eq!(eval_with_symbols("_start+4", &lookup), Ok(0x401004));
        assert_eq!(eval_with_symbols("_end - _start", &lookup), Ok(0x1000));
        assert_eq!(eval_with_symbols("(_end - _start) / 2", &lookup), Ok(0x800));
        // Precedence and parentheses.
        assert_eq!(eval_with_symbols("a + b * 2", &lookup), Ok(13));
        assert_eq!(eval_with_symbols("(a + b) * 2", &lookup), Ok(20));
        assert_eq!(eval_with_symbols("a % b", &lookup), Ok(1));
        assert_eq!(eval_with_symbols("-a + 10", &lookup), Ok(3));
        assert_eq!(eval_with_symbols("~0", &lookup), Ok(u64::MAX));
        // Plain constants need no symbols at all.
        assert_eq!(eval("0x1000 + 1K"), Ok(0x1400));
        // A signed right-hand side is an expression, not a number token, and it
        // still evaluates: -1 is the all-ones address, which is what a linker
        // means by it.
        assert!(matches!(classify("-1", defined), Ok(Defsym::Expression(_))));
        assert_eq!(eval("-1"), Ok(u64::MAX));
    }

    #[test]
    fn division_by_zero_is_reported_as_itself() {
        // bfd says "/ by zero"; a syntax error here would send the user looking
        // for a typo instead of at the divisor.
        assert_eq!(eval("1/0"), Err(DefsymError::DivByZero));
        assert_eq!(eval("1 % 0"), Err(DefsymError::DivByZero));
        assert_eq!(eval("(2+2)/(1-1)"), Err(DefsymError::DivByZero));
        assert!(eval("1/0").unwrap_err().message().contains("/ by zero"));
    }

    #[test]
    fn an_undefined_symbol_in_an_expression_names_the_symbol() {
        let e = eval_with_symbols("_start + nosuch", &|_| None).unwrap_err();
        match &e {
            DefsymError::UndefinedSymbol(n) => assert_eq!(n, "_start"),
            other => panic!("expected UndefinedSymbol, got {other:?}"),
        }
        assert!(e.message().contains("undefined symbol '_start'"));
    }

    #[test]
    fn subtraction_wraps_like_an_address_difference_should() {
        // A symbol below another one yields the two's-complement difference;
        // erroring here would make `_end - _start` unusable whenever the layout
        // puts them the other way round.
        assert_eq!(eval("0 - 1"), Ok(u64::MAX));
        assert_eq!(
            eval_with_symbols("_start - _end", &|n| match n {
                "_start" => Some(1u64),
                "_end" => Some(3u64),
                _ => None,
            }),
            Ok(u64::MAX - 1)
        );
    }

    #[test]
    fn whitespace_is_ignored_where_gnu_ld_ignores_it() {
        // Leading and trailing space is trimmed; space *inside* the expression is
        // part of the text the user wrote and is kept, so diagnostics quote it
        // back exactly. What must be equal is the meaning, not the spelling.
        assert_eq!(
            classify("  _start + 4  ", defined),
            Ok(Defsym::Expression("_start + 4".to_string()))
        );
        let lookup = |n: &str| if n == "_start" { Some(0x1000u64) } else { None };
        assert_eq!(
            eval_with_symbols("  _start + 4  ", &lookup),
            eval_with_symbols("_start+4", &lookup)
        );
        assert_eq!(eval(" 1 + 2 "), Ok(3));
        // But not inside a number: "1 2" is two tokens, which is a syntax error.
        assert!(matches!(
            classify("1 2", defined),
            Err(DefsymError::Syntax(_))
        ));
    }

    #[test]
    fn the_needs_layout_error_says_what_to_use_instead() {
        let m = DefsymError::NeedsLayout("_start+4".into()).message();
        assert!(m.contains("_start+4"), "{m}");
        assert!(m.contains("constant"), "{m}");
        assert!(m.contains("alias"), "{m}");
    }
}
