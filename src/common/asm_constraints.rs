//! Shared inline assembly constraint classification utilities.
//!
//! Used by the inline pass (symbol resolution), cfg_simplify (dead block detection),
//! and the backend (operand emission) to consistently identify immediate-only constraints.

/// A malformed GNU named matching constraint (`[name]`, `[name]r`, `[a][b]`).
///
/// The parser owns the diagnostic wording; the back end only needs the names.
/// Keeping the grammar here prevents the two from disagreeing about what a
/// named tie is, which is how `[sum]r` once became a register class instead of
/// a tie to the output named `sum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedMatchingError {
    MissingClose,
    InvalidPunctuation(char),
}

/// Return every output name referenced by a GNU named matching constraint.
///
/// GCC 16.2 accepts a suffix after the closing bracket (`[sum]r`), repeated
/// references (`[sum][sum]`), and a `#` constraint comment.  The letters inside
/// the brackets are an operand name, not constraint letters; the letters after
/// them do not turn the reference into an immediate/register class.  This
/// routine therefore owns only bracket structure and the punctuation measured
/// in a named matching constraint, leaving target-specific class semantics to
/// the validator.
pub fn named_matching_names(constraint: &str) -> Result<Option<Vec<&str>>, NamedMatchingError> {
    let body = constraint.trim_start_matches(['=', '+', '&', '%']);
    if !body.starts_with('[') {
        return Ok(None);
    }
    let mut names = Vec::new();
    let mut rest = body;
    while let Some(ch) = rest.chars().next() {
        match ch {
            '#' => break,
            '[' => {
                rest = &rest[1..];
                let close = rest.find(']').ok_or(NamedMatchingError::MissingClose)?;
                names.push(&rest[..close]);
                rest = &rest[close + 1..];
            }
            ']' => return Err(NamedMatchingError::InvalidPunctuation(ch)),
            '!' | '$' | '*' | '?' | '^' => rest = &rest[ch.len_utf8()..],
            _ if ch.is_ascii_alphanumeric() => rest = &rest[ch.len_utf8()..],
            _ => return Err(NamedMatchingError::InvalidPunctuation(ch)),
        }
    }
    Ok(Some(names))
}

/// Check whether a constraint string is purely immediate (no register or memory alternatives).
///
/// Returns true for constraints like "i", "n", "e" that ONLY accept compile-time constants.
/// Returns false for multi-alternative constraints like "ri", "Ir", "g" that also accept
/// registers or memory operands.
///
/// The constraint is first stripped of output/early-clobber modifiers ('=', '+', '&').
/// Named operand references like "[name]" are not immediate constraints.
pub fn constraint_is_immediate_only(constraint: &str) -> bool {
    let stripped = constraint.trim_start_matches(['=', '+', '&', '%']);
    if stripped.is_empty() {
        return false;
    }
    // A bracketed operand name is never an immediate class, even when the
    // spelling is malformed; the parser owns that diagnostic.  Falling through
    // to the letter scan would let `[i` or `[sum]i` masquerade as `"i"`.
    if stripped.starts_with('[') {
        return false;
    }
    // Must have at least one immediate letter
    let has_imm = stripped.chars().any(|c| {
        matches!(
            c,
            'i' | 'I' | 'n' | 'N' | 'e' | 'E' | 'K' | 'M' | 'G' | 'H' | 'J' | 'L' | 'O'
        )
    });
    if !has_imm {
        return false;
    }
    // Must NOT have any register or memory alternative
    let has_reg_or_mem = stripped.chars().any(|c| {
        matches!(
            c,
            'r' | 'q' | 'R' | 'l' |           // GP register
        'g' |                              // general (reg + mem + imm)
        'x' | 'v' | 'Y' |                 // FP register
        'a' | 'b' | 'c' | 'd' | 'S' | 'D' | // specific register
        'm' | 'o' | 'V' | 'p' | 'Q' // memory
        )
    });
    !has_reg_or_mem && !stripped.chars().any(|c| c.is_ascii_digit())
}

/// One GCC condition-code suffix and every encoding a back end needs for it.
///
/// This table is the single source of truth for three consumers that used to
/// carry their own, drifting copies:
///
///   * the parser (`frontend::parser::asm_validate`), which must accept exactly
///     the suffixes GCC accepts and reject everything else;
///   * `setcc` emission for `=@cc<suffix>` output operands;
///   * `asm goto` flag-output branch fusion, which additionally needs the
///     inverted jump form.
///
/// The set was measured, not assumed: all 28 entries below are accepted by
/// `gcc`, and `pe`/`po` -- which both GCC and Clang reject with
/// `unknown 'asm' flag output '=@ccpe'` -- are deliberately absent, so a
/// program this compiler accepts is always a program GCC accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CondCode {
    /// GCC suffix as written in the constraint (`"z"`, `"nbe"`, ...).
    pub gcc: &'static str,
    /// x86 `setcc` suffix, i.e. the part after `set` (`"e"`, `"ne"`, ...).
    pub setcc: &'static str,
    /// Full x86 `jcc` mnemonic (`"je"`, `"jne"`, ...).
    pub jcc: &'static str,
    /// Full `jcc` mnemonic for the logically negated condition.
    pub inverse_jcc: &'static str,
}

impl CondCode {
    /// Const constructor: keeps one table row on one line, which is what makes
    /// the table readable as a table and stable under `cargo fmt --check`. A
    /// struct literal in an array element is reflowed to four lines each.
    const fn new(
        gcc: &'static str,
        setcc: &'static str,
        jcc: &'static str,
        inverse_jcc: &'static str,
    ) -> Self {
        Self {
            gcc,
            setcc,
            jcc,
            inverse_jcc,
        }
    }
}

pub const CONDITION_CODES: [CondCode; 28] = [
    CondCode::new("e", "e", "je", "jne"),
    CondCode::new("z", "e", "je", "jne"),
    CondCode::new("ne", "ne", "jne", "je"),
    CondCode::new("nz", "ne", "jne", "je"),
    CondCode::new("s", "s", "js", "jns"),
    CondCode::new("ns", "ns", "jns", "js"),
    CondCode::new("o", "o", "jo", "jno"),
    CondCode::new("no", "no", "jno", "jo"),
    CondCode::new("c", "c", "jc", "jnc"),
    CondCode::new("nc", "nc", "jnc", "jc"),
    CondCode::new("a", "a", "ja", "jbe"),
    CondCode::new("nbe", "a", "ja", "jbe"),
    CondCode::new("ae", "ae", "jae", "jb"),
    CondCode::new("nb", "ae", "jae", "jb"),
    CondCode::new("b", "b", "jb", "jae"),
    CondCode::new("nae", "b", "jb", "jae"),
    CondCode::new("be", "be", "jbe", "ja"),
    CondCode::new("na", "be", "jbe", "ja"),
    CondCode::new("g", "g", "jg", "jle"),
    CondCode::new("nle", "g", "jg", "jle"),
    CondCode::new("ge", "ge", "jge", "jl"),
    CondCode::new("nl", "ge", "jge", "jl"),
    CondCode::new("l", "l", "jl", "jge"),
    CondCode::new("nge", "l", "jl", "jge"),
    CondCode::new("le", "le", "jle", "jg"),
    CondCode::new("ng", "le", "jle", "jg"),
    CondCode::new("p", "p", "jp", "jnp"),
    CondCode::new("np", "np", "jnp", "jp"),
];

/// Look up a GCC condition-code suffix (`"e"`, `"nbe"`, ...).
///
/// Linear scan on purpose: at most 28 comparisons of one- or two-byte strings,
/// executed once per `@cc` operand that is actually emitted or validated -- not
/// per IR instruction -- so the branch predictor handles it for free and a
/// hand-sorted table could not be any cheaper.
pub fn cond_code(gcc_suffix: &str) -> Option<&'static CondCode> {
    CONDITION_CODES.iter().find(|c| c.gcc == gcc_suffix)
}

/// AArch64 condition suffixes accepted after `@cc` in a flag output.
///
/// GCC documents the AArch64 flag outputs as "the standard conditions defined
/// in the ARM ARM for `ConditionHolds`", and the set it actually accepts is the
/// sixteen orderable conditions below: the four NZCV tests (`eq`, `ne`, `mi`,
/// `pl`), the two carry tests with both of their architectural spellings
/// (`cs`/`hs` for "carry set", `cc`/`lo` for "carry clear"), the two overflow
/// tests (`vs`, `vc`), and the six unsigned/signed relation pairs
/// (`hi`/`ls`, `ge`/`lt`, `gt`/`le`).
///
/// `al` and `nv` are deliberately absent. They are ARM `ConditionHolds`
/// conditions, but they are not *flag tests* -- `al` is "always" and `nv` is a
/// behavioural `udf` -- so a `cset`-based materialisation of either is
/// meaningless. GCC rejects both in this position, which keeps the rule this
/// table encodes simple: every suffix that passes the parser has a `CSET`
/// spelling, and every suffix the parser rejects is one GCC rejects too.
///
/// Unlike `CONDITION_CODES` (the x86 table) the entries carry no `setcc`/`jcc`
/// spelling: AArch64 has no condition-code *register* to snapshot, so a flag
/// output there is a `CSET` reading NZCV directly, and the suffix itself *is*
/// the instruction's condition field.
pub const AARCH64_FLAG_CONDITIONS: [&str; 16] = [
    "eq", "ne", "cs", "hs", "cc", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le",
];

/// Look up an AArch64 `@cc` suffix.
///
/// Returns the suffix unchanged (it is already the `CSET` condition field) when
/// the target accepts it, `None` otherwise. Linear scan over sixteen `&str`
/// comparisons, executed once per flag-output operand: cheaper than building a
/// match with sixteen arms and identical in effect.
pub fn aarch64_flag_condition(suffix: &str) -> Option<&'static str> {
    AARCH64_FLAG_CONDITIONS.iter().find(|c| **c == suffix).copied()
}

/// Invert a full `jcc` mnemonic (`"je"` -> `"jne"`, `"jc"` -> `"jnc"`, ...).
pub fn inverse_jcc(jcc: &str) -> Option<&'static str> {
    CONDITION_CODES
        .iter()
        .find(|c| c.jcc == jcc)
        .map(|c| c.inverse_jcc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The AArch64 flag-output table is a closed set: every entry maps to
    /// itself (the suffix *is* the `CSET` condition field), nothing else is
    /// accepted, and the x86 spellings (`z`, `nz`, `nbe`, ...) deliberately do
    /// not appear in it.
    #[test]
    fn aarch64_flag_conditions_are_the_cset_set() {
        for c in AARCH64_FLAG_CONDITIONS {
            assert_eq!(aarch64_flag_condition(c), Some(c), "{c}");
        }
        for bad in [
            "", "z", "nz", "e", "nbe", "nae", "al", "nv", "pe", "po", "EQ", "eqq", "eq ",
        ] {
            assert_eq!(aarch64_flag_condition(bad), None, "{bad:?}");
        }
        // The two tables overlap on purpose (`ne`, `ge`, `le` are the same
        // words on both targets) but never on a spelling that means something
        // *different* on the other target; each aarch64 entry is exactly the
        // `ConditionHolds` name, so the value returned is the input.
        for (suffix, cset) in [
            ("ne", "ne"),
            ("ge", "ge"),
            ("le", "le"),
            ("hs", "hs"),
            ("lo", "lo"),
        ] {
            assert_eq!(aarch64_flag_condition(suffix), Some(cset));
        }
        // Conversely, the x86-only spellings are not ARM conditions, so a
        // program that says `=@ccz` is rejected on AArch64 and one that says
        // `=@cchs` is rejected on x86 -- neither is silently reinterpreted.
        // (`ne`, `ge` and `le` are shared words; `z`, `nz`, `nbe`, `nae`,
        // `nl`, `nle`, `ng`, `np` are x86-only.)
        for x86_only in ["z", "nz", "nbe", "nae", "nl", "nle", "ng", "np"] {
            assert_eq!(aarch64_flag_condition(x86_only), None, "{x86_only}");
            assert!(cond_code(x86_only).is_some(), "{x86_only} must stay an x86 suffix");
        }
    }

    #[test]
    fn named_matching_names_cover_the_measured_gcc_spellings() {
        for (constraint, expected) in [
            ("[sum]", vec!["sum"]),
            ("[sum]r", vec!["sum"]),
            ("[sum][sum]", vec!["sum", "sum"]),
            ("[sum]r[sum]", vec!["sum", "sum"]),
            ("[sum][other]", vec!["sum", "other"]),
            ("[sum]#comment", vec!["sum"]),
            ("[]", vec![""]),
        ] {
            assert_eq!(
                named_matching_names(constraint).unwrap(),
                Some(expected),
                "{constraint}"
            );
        }
        assert_eq!(named_matching_names("r").unwrap(), None);
        assert_eq!(named_matching_names("0").unwrap(), None);
        assert_eq!(named_matching_names("i").unwrap(), None);
    }

    #[test]
    fn named_matching_names_reject_malformed_brackets_and_punctuation() {
        assert_eq!(
            named_matching_names("[sum"),
            Err(NamedMatchingError::MissingClose)
        );
        assert_eq!(
            named_matching_names("[sum]["),
            Err(NamedMatchingError::MissingClose)
        );
        for ch in ['@', '_', ' ', '.', '/', '\\', ';', '\'', '(', ')', '}', ']'] {
            let constraint = format!("[sum]{ch}");
            assert_eq!(
                named_matching_names(&constraint),
                Err(NamedMatchingError::InvalidPunctuation(ch)),
                "{constraint}"
            );
        }
    }

    #[test]
    fn named_matching_constraints_are_not_immediate_only_classes() {
        for constraint in ["[sum]", "[sum]i", "[sum][sum]n", "[sum]r"] {
            assert!(!constraint_is_immediate_only(constraint), "{constraint}");
        }
        assert!(constraint_is_immediate_only("i"));
        assert!(!constraint_is_immediate_only("Ir"));
    }
}
