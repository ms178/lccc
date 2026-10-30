//! Shared inline assembly constraint classification utilities.
//!
//! Used by the inline pass (symbol resolution), cfg_simplify (dead block detection),
//! and the backend (operand emission) to consistently identify immediate-only constraints.

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
    if stripped.starts_with('[') && stripped.ends_with(']') {
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
        Self { gcc, setcc, jcc, inverse_jcc }
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

/// Invert a full `jcc` mnemonic (`"je"` -> `"jne"`, `"jc"` -> `"jnc"`, ...).
pub fn inverse_jcc(jcc: &str) -> Option<&'static str> {
    CONDITION_CODES
        .iter()
        .find(|c| c.jcc == jcc)
        .map(|c| c.inverse_jcc)
}
