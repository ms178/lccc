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

pub const CONDITION_CODES: [CondCode; 28] = [
    CondCode { gcc: "e", setcc: "e", jcc: "je", inverse_jcc: "jne" },
    CondCode { gcc: "z", setcc: "e", jcc: "je", inverse_jcc: "jne" },
    CondCode { gcc: "ne", setcc: "ne", jcc: "jne", inverse_jcc: "je" },
    CondCode { gcc: "nz", setcc: "ne", jcc: "jne", inverse_jcc: "je" },
    CondCode { gcc: "s", setcc: "s", jcc: "js", inverse_jcc: "jns" },
    CondCode { gcc: "ns", setcc: "ns", jcc: "jns", inverse_jcc: "js" },
    CondCode { gcc: "o", setcc: "o", jcc: "jo", inverse_jcc: "jno" },
    CondCode { gcc: "no", setcc: "no", jcc: "jno", inverse_jcc: "jo" },
    CondCode { gcc: "c", setcc: "c", jcc: "jc", inverse_jcc: "jnc" },
    CondCode { gcc: "nc", setcc: "nc", jcc: "jnc", inverse_jcc: "jc" },
    CondCode { gcc: "a", setcc: "a", jcc: "ja", inverse_jcc: "jbe" },
    CondCode { gcc: "nbe", setcc: "a", jcc: "ja", inverse_jcc: "jbe" },
    CondCode { gcc: "ae", setcc: "ae", jcc: "jae", inverse_jcc: "jb" },
    CondCode { gcc: "nb", setcc: "ae", jcc: "jae", inverse_jcc: "jb" },
    CondCode { gcc: "b", setcc: "b", jcc: "jb", inverse_jcc: "jae" },
    CondCode { gcc: "nae", setcc: "b", jcc: "jb", inverse_jcc: "jae" },
    CondCode { gcc: "be", setcc: "be", jcc: "jbe", inverse_jcc: "ja" },
    CondCode { gcc: "na", setcc: "be", jcc: "jbe", inverse_jcc: "ja" },
    CondCode { gcc: "g", setcc: "g", jcc: "jg", inverse_jcc: "jle" },
    CondCode { gcc: "nle", setcc: "g", jcc: "jg", inverse_jcc: "jle" },
    CondCode { gcc: "ge", setcc: "ge", jcc: "jge", inverse_jcc: "jl" },
    CondCode { gcc: "nl", setcc: "ge", jcc: "jge", inverse_jcc: "jl" },
    CondCode { gcc: "l", setcc: "l", jcc: "jl", inverse_jcc: "jge" },
    CondCode { gcc: "nge", setcc: "l", jcc: "jl", inverse_jcc: "jge" },
    CondCode { gcc: "le", setcc: "le", jcc: "jle", inverse_jcc: "jg" },
    CondCode { gcc: "ng", setcc: "le", jcc: "jle", inverse_jcc: "jg" },
    CondCode { gcc: "p", setcc: "p", jcc: "jp", inverse_jcc: "jnp" },
    CondCode { gcc: "np", setcc: "np", jcc: "jnp", inverse_jcc: "jp" },
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
