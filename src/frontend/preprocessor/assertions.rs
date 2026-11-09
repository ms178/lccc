//! GNU preprocessor assertions: `#assert`, `#unassert`, and `#predicate(answer)`
//! tests in `#if` / `#elif` expressions.
//!
//! Semantics mirror GCC 16.2 (all witnessed natively, 2026-11-09, and against
//! `gcc/config/{i386/i386-c.cc,linux.h}` of the upstream sources):
//!
//! * `#assert pred(answer)` records an answer for `pred`; duplicates are
//!   idempotent. The answer is every token up to the FIRST `)` — parentheses
//!   do not nest inside an answer (`#assert foo((a))` records `(a` and warns
//!   about the trailing `)`).
//! * `#unassert pred` drops every answer; `#unassert pred(answer)` drops that
//!   answer only. Unasserting an unknown predicate is not diagnosed.
//! * In `#if`, `#pred(answer)` is 1 when that answer was asserted and
//!   `#pred` (no parenthesis) is 1 when ANY answer exists for `pred`.
//! * Answers compare as token sequences: the SPELLING of each token and the
//!   PRESENCE of whitespace between tokens are significant, the amount is
//!   not (`a b` == `a  b`, but `a+b` != `a + b` — witnessed).
//! * Predefined assertions (GCC target sources): every Linux target asserts
//!   `system=linux`, `system=unix`, `system=posix`; x86-64 adds
//!   `cpu=x86_64`/`machine=x86_64` and ia32 adds `cpu=i386`/`machine=i386`.
//!   AArch64 and RISC-V define none (their `*-c.cc` never call
//!   `builtin_assert`).
//! * `-A pred=answer` asserts from the command line; `-A -pred=answer`
//!   cancels exactly that answer.

use crate::common::fx_hash::FxHashMap;

/// Byte that introduces a resolved assertion value in a conditional
/// expression ("\x06" + '0'/'1'). MUST stay clear of the macro expander's
/// internal markers 0x01..=0x04 (`macro_defs::{BLUE_PAINT_MARKER,
/// PASTE_PROTECT_START, PASTE_PROTECT_END, TOKEN_BOUNDARY}`), which
/// `normalize_internal_markers` strips from every expanded line: a stripped
/// marker would fuse the value digit with adjacent tokens ("1#cpu(x)" would
/// silently become the number 10).
pub(crate) const ASSERTION_MARKER: u8 = 0x06;
const ASSERTION_MARKER_CHAR: char = '\u{6}';

/// The table of live assertions: predicate name -> canonicalized answers.
///
/// Answer vectors stay insertion-ordered; lookups compare canonical strings.
/// Real-world predicate/answer counts are tiny (a handful), so a `Vec` per
/// predicate beats any set structure on both memory and lookup time.
#[derive(Debug, Clone, Default)]
pub struct AssertionTable {
    map: FxHashMap<String, Vec<String>>,
}

impl AssertionTable {
    /// Table preloaded with the predefined assertions GCC's target headers
    /// install for `target` (an LCCC target name as used by
    /// `Preprocessor::set_target`).
    pub fn for_target(target: &str) -> Self {
        let mut table = Self::default();
        // gcc/config/linux.h LINUX_TARGET_OS_CPP_BUILTINS, all targets here
        // are *-linux-gnu.
        for answer in ["linux", "unix", "posix"] {
            table.assert_answer("system", answer);
        }
        // gcc/config/i386/i386-c.cc; aarch64/riscv ports assert nothing.
        let cpu = match target {
            "x86_64" => Some("x86_64"),
            "i686" => Some("i386"),
            _ => None,
        };
        if let Some(cpu) = cpu {
            table.assert_answer("cpu", cpu);
            table.assert_answer("machine", cpu);
        }
        table
    }

    /// Record `predicate(answer)`. The answer must already be canonical
    /// (see [`canonicalize_answer`]). Duplicate assertions are idempotent.
    pub fn assert_answer(&mut self, predicate: &str, answer: &str) {
        let answers = self.map.entry(predicate.to_string()).or_default();
        if !answers.iter().any(|a| a == answer) {
            answers.push(answer.to_string());
        }
    }

    /// `#unassert pred(answer)`: drop one answer. Unknown predicates and
    /// answers are silently ignored (GCC behaviour).
    pub fn unassert_answer(&mut self, predicate: &str, answer: &str) {
        if let Some(answers) = self.map.get_mut(predicate) {
            answers.retain(|a| a != answer);
            if answers.is_empty() {
                self.map.remove(predicate);
            }
        }
    }

    /// `#unassert pred`: drop every answer for the predicate.
    pub fn unassert_all(&mut self, predicate: &str) {
        self.map.remove(predicate);
    }

    /// `#pred(answer)` in `#if`: is this exact answer asserted?
    pub fn holds(&self, predicate: &str, answer: &str) -> bool {
        self.map
            .get(predicate)
            .is_some_and(|answers| answers.iter().any(|a| a == answer))
    }

    /// `#pred` in `#if`: does any answer exist for the predicate?
    pub fn any(&self, predicate: &str) -> bool {
        self.map.contains_key(predicate)
    }
}

/// Canonicalize an assertion answer to GCC's comparison semantics: the
/// spelling of each preprocessing token plus a single space wherever the
/// source had any whitespace between tokens. Returns `None` for an answer
/// with no tokens (`#assert foo()` is "predicate's answer is empty").
///
/// Tokens are scanned with C pp-token rules: identifiers, pp-numbers,
/// character/string literals (escapes honoured) and punctuators (longest
/// match first, so `a+b` keeps `+` attached while `a ++ b` differs from
/// `a + + b` exactly as in cpplib).
pub fn canonicalize_answer(answer: &str) -> Option<String> {
    use super::utils::{is_ident_cont_byte, is_ident_start_byte, ppnumber_end};
    let bytes = answer.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(answer.len());
    let mut i = 0;
    let mut pending_space = false;
    let mut any = false;
    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            pending_space = true;
            i += 1;
            continue;
        }
        let start = i;
        if is_ident_start_byte(b) {
            i += 1;
            while i < len && is_ident_cont_byte(bytes[i]) {
                i += 1;
            }
        } else if b.is_ascii_digit()
            || (b == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i = ppnumber_end(bytes, i);
        } else if b == b'"' || b == b'\'' {
            i += 1;
            while i < len && bytes[i] != b {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
        } else {
            i += punctuator_len(&bytes[i..]);
        }
        if any && pending_space {
            out.push(' ');
        }
        // Answers arrive from source lines, which are valid UTF-8; the scan
        // only splits at ASCII boundaries, so the slice stays valid UTF-8.
        out.push_str(&answer[start..i]);
        pending_space = false;
        any = true;
    }
    any.then_some(out)
}

/// Length of the longest C punctuator at the head of `bytes` (at least 1:
/// any unrecognised byte is its own token, like cpplib's CPP_OTHER).
fn punctuator_len(bytes: &[u8]) -> usize {
    const THREE: [&[u8]; 3] = [b"...", b"<<=", b">>="];
    const TWO: [&[u8]; 19] = [
        b"->", b"++", b"--", b"<<", b">>", b"<=", b">=", b"==", b"!=", b"&&", b"||", b"+=", b"-=",
        b"*=", b"/=", b"%=", b"&=", b"^=", b"|=",
    ];
    if THREE.iter().any(|p| bytes.starts_with(p)) {
        3
    } else if TWO.iter().any(|p| bytes.starts_with(p)) {
        2
    } else {
        1
    }
}

/// One diagnostic produced while resolving assertions in a `#if`/`#elif`
/// expression. `col` is a 1-based byte offset into the expression text.
#[derive(Debug, PartialEq, Eq)]
pub struct AssertionIncident {
    pub col: usize,
    pub message: String,
    pub is_error: bool,
    /// True for the `-Wdeprecated` "assertions are a deprecated extension"
    /// note; false for hard syntax errors.
    pub deprecated: bool,
}

/// Replace every `#predicate(answer)` / `#predicate` test in a conditional
/// expression with an inline marker token (`\x01` + `'1'`/`'0'`) that the
/// expression tokenizer turns into [`ExprToken::Assertion`]. Character and
/// string literals are copied verbatim. Predicates and answers are consumed
/// here, BEFORE macro expansion, which is exactly why they are never
/// macro-expanded (GCC behaviour; `#if empty#cpu(m68k)` from
/// gcc.c-torture 950919-1.c relies on it).
///
/// Malformed assertions follow GCC 16.2: a `#` with no identifier is
/// "assertion without predicate", an unterminated answer is "missing ')' to
/// complete answer", an empty answer is "predicate's answer is empty". All
/// three substitute a false marker so evaluation can continue; the caller
/// promotes the incident to a preprocessor error.
pub fn resolve_assertions(expr: &str, table: &AssertionTable) -> (String, Vec<AssertionIncident>) {
    use super::utils::{is_ident_cont_byte, is_ident_start_byte};
    let bytes = expr.as_bytes();
    let len = bytes.len();
    if !bytes.contains(&b'#') {
        return (expr.to_string(), Vec::new());
    }
    let mut out = String::with_capacity(expr.len());
    let mut incidents = Vec::new();
    let mut i = 0;
    let mut seg = 0; // start of the pending verbatim segment
    while i < len {
        let b = bytes[i];
        // Skip literals wholesale so a '#' inside one is never an assertion.
        if b == b'"' || b == b'\'' {
            i += 1;
            while i < len && bytes[i] != b {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }
        if b != b'#' {
            i += 1;
            continue;
        }
        out.push_str(&expr[seg..i]);
        let hash_col = i + 1; // 1-based
        incidents.push(AssertionIncident {
            col: hash_col,
            message: "assertions are a deprecated extension".to_string(),
            is_error: false,
            deprecated: true,
        });
        i += 1;
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len || !is_ident_start_byte(bytes[i]) {
            incidents.push(AssertionIncident {
                col: i.min(len) + 1,
                message: "assertion without predicate".to_string(),
                is_error: true,
                deprecated: false,
            });
            out.push_str("\u{6}0");
            seg = i;
            continue;
        }
        let pred_start = i;
        while i < len && is_ident_cont_byte(bytes[i]) {
            i += 1;
        }
        let predicate = &expr[pred_start..i];
        // Optional whitespace, then an optional parenthesized answer.
        let mut j = i;
        while j < len && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let value = if j < len && bytes[j] == b'(' {
            // The answer is every token up to the FIRST ')': parentheses do
            // not nest inside answers (witnessed with `#foo((a))`).
            let answer_start = j + 1;
            let close = bytes[answer_start..]
                .iter()
                .position(|&c| c == b')')
                .map(|off| answer_start + off);
            match close {
                None => {
                    incidents.push(AssertionIncident {
                        col: len + 1,
                        message: "missing ')' to complete answer".to_string(),
                        is_error: true,
                        deprecated: false,
                    });
                    i = len;
                    false
                }
                Some(close) => {
                    i = close + 1;
                    match canonicalize_answer(&expr[answer_start..close]) {
                        None => {
                            incidents.push(AssertionIncident {
                                col: close + 1,
                                message: "predicate's answer is empty".to_string(),
                                is_error: true,
                                deprecated: false,
                            });
                            false
                        }
                        Some(answer) => table.holds(predicate, &answer),
                    }
                }
            }
        } else {
            table.any(predicate)
        };
        out.push(ASSERTION_MARKER_CHAR);
        out.push(if value { '1' } else { '0' });
        seg = i;
    }
    out.push_str(&expr[seg..]);
    (out, incidents)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> AssertionTable {
        let mut t = AssertionTable::default();
        t.assert_answer("foo", "bar");
        t.assert_answer("foo", "a b");
        t
    }

    #[test]
    fn canonicalization_matches_cpplib_token_rules() {
        // Amount of whitespace is insignificant, presence is significant.
        assert_eq!(canonicalize_answer(" a  b "), Some("a b".to_string()));
        assert_eq!(canonicalize_answer("a b"), Some("a b".to_string()));
        assert_eq!(canonicalize_answer("a+b"), Some("a+b".to_string()));
        assert_ne!(canonicalize_answer("a + b"), canonicalize_answer("a+b"));
        // Multi-character punctuators stay single tokens.
        assert_eq!(canonicalize_answer("a<<=b"), Some("a<<=b".to_string()));
        assert_ne!(canonicalize_answer("a< <=b"), canonicalize_answer("a<<=b"));
        // Literals keep their exact spelling, including inner spaces.
        assert_eq!(
            canonicalize_answer("\"x  y\""),
            Some("\"x  y\"".to_string())
        );
        assert_eq!(canonicalize_answer("  "), None);
        assert_eq!(canonicalize_answer(""), None);
    }

    #[test]
    fn predefined_tables_match_gcc_target_sources() {
        let x64 = AssertionTable::for_target("x86_64");
        assert!(x64.holds("system", "linux"));
        assert!(x64.holds("system", "unix"));
        assert!(x64.holds("system", "posix"));
        assert!(x64.holds("cpu", "x86_64") && x64.holds("machine", "x86_64"));
        assert!(!x64.holds("cpu", "i386"));
        let ia32 = AssertionTable::for_target("i686");
        assert!(ia32.holds("cpu", "i386") && ia32.holds("machine", "i386"));
        assert!(!ia32.holds("cpu", "x86_64"));
        for t in ["aarch64", "riscv64"] {
            let table = AssertionTable::for_target(t);
            assert!(table.holds("system", "linux"));
            assert!(!table.any("cpu") && !table.any("machine"), "{t}");
        }
    }

    #[test]
    fn unassert_semantics() {
        let mut t = table();
        assert!(t.holds("foo", "bar") && t.any("foo"));
        t.unassert_answer("foo", "bar");
        assert!(!t.holds("foo", "bar") && t.any("foo"));
        t.unassert_answer("foo", "a b");
        assert!(!t.any("foo"));
        // Unknown unasserts are silent no-ops.
        t.unassert_all("nothere");
        t.unassert_answer("nothere", "x");
    }

    #[test]
    fn resolve_rewrites_tests_to_markers() {
        let t = table();
        let (out, inc) = resolve_assertions("#foo(bar) && !#foo(baz) && #foo", &t);
        assert_eq!(out, "\u{6}1 && !\u{6}0 && \u{6}1");
        assert_eq!(inc.iter().filter(|i| i.deprecated).count(), 3);
        assert!(inc.iter().all(|i| !i.is_error));
        // Whitespace tolerated after '#', around the predicate and inside
        // the answer; amount-insensitive comparison.
        let (out, _) = resolve_assertions("# foo (  a   b )", &t);
        assert_eq!(out, "\u{6}1");
        // '#' inside literals is not an assertion.
        let (out, inc) = resolve_assertions("'#' == 35 && \"#foo\"[0]", &t);
        assert_eq!(out, "'#' == 35 && \"#foo\"[0]");
        assert!(inc.is_empty());
    }

    #[test]
    fn resolve_diagnoses_malformed_assertions_like_gcc() {
        let t = table();
        let (out, inc) = resolve_assertions("#", &t);
        assert_eq!(out, "\u{6}0");
        assert!(
            inc.iter()
                .any(|i| i.is_error && i.message == "assertion without predicate")
        );
        let (_, inc) = resolve_assertions("#foo(unterminated", &t);
        assert!(
            inc.iter()
                .any(|i| i.is_error && i.message == "missing ')' to complete answer")
        );
        let (_, inc) = resolve_assertions("#foo()", &t);
        assert!(
            inc.iter()
                .any(|i| i.is_error && i.message == "predicate's answer is empty")
        );
        // Answers stop at the first ')': the remainder stays in the stream.
        let (out, _) = resolve_assertions("#foo((a))", &t);
        assert_eq!(out, "\u{6}0)");
    }
}
