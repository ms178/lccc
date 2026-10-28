//! Inline-assembly operand validation.
//!
//! GCC diagnoses a malformed `asm` operand *while parsing* the statement. The
//! messages below are the contract, measured against gcc 16.2 on x86-64:
//!
//! | source                                  | GCC 16.2 |
//! |-----------------------------------------|----------|
//! | `: "=r"(r) : "=r"(x)`                   | `error: input operand constraint contains '='` |
//! | `: "=r"(r) : "+r"(x)`                   | `error: input operand constraint contains '+'` |
//! | `: "=r"(r) : "z"(x)` / `"j"(x)`         | `error: impossible constraint in 'asm'` |
//! | `: "=r"(r) : "I"(x)` (non-constant)     | `warning: 'asm' operand 1 probably does not match constraints` + `error: impossible constraint in 'asm'` |
//! | template `%7` with two operands         | `error: invalid 'asm': operand number out of range` |
//!
//! `asm_validate::validate` reproduces those three classes. Why this matters:
//! the x86 back ends classify constraints in a `match` whose catch-all arm is
//! `_ => {}`, so an unmodelled letter such as `"z"` used to be silently
//! reclassified as a general-purpose register and the statement was emitted
//! with the wrong register class — a miscompile, not a rejected program.
//!
//! Deliberate conservatism, so that no *valid* program is rejected:
//! * a target other than x86-64/i686 validates only the `=`/`+`-on-input rule;
//! * a letter is refused only when this compiler cannot honour it (the set is
//!   the union of what the x86 classifier arms accept plus the x87/SSE
//!   specials), never merely because it looks unusual;
//! * an immediate constraint is checked only for the ranges that were
//!   measured (`I J K M N O`), and only when the operand expression folds to a
//!   constant or is provably not one. `e E L Z G H n` are left alone.

use super::ast::{BinOp, Expr, UnaryOp};
use crate::common::fx_hash::FxHashMap;

/// Constant value of an operand expression, as far as validation is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constness {
    /// The expression folds to this value.
    Value(i64),
    /// The expression provably is not an integer constant (it names an object,
    /// calls a function, or writes memory).
    NonConstant,
    /// Neither established nor refuted; nothing is diagnosed.
    Unknown,
}

/// One operand of an `asm` statement, as the validator sees it.
pub struct Operand<'a> {
    pub constraint: &'a str,
    pub constant: Constness,
}

/// One diagnostic, plus the operand it belongs to (`None` = the template).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub is_error: bool,
    pub operand: Option<usize>,
}

impl Diagnostic {
    fn error(message: impl Into<String>, operand: Option<usize>) -> Self {
        Self {
            message: message.into(),
            is_error: true,
            operand,
        }
    }
    fn warning(message: impl Into<String>, operand: Option<usize>) -> Self {
        Self {
            message: message.into(),
            is_error: false,
            operand,
        }
    }
}

/// Constraint characters the x86 back ends actually classify: register, memory
/// and symbolic classes on the left, immediate-only classes on the right.
/// Anything else used to fall through `match { _ => {} }` into `GpReg`.
const X86_CLASSES: &str = "abcdfglmopqrvxyXADQRSVY";
const X86_IMMEDIATES: &str = "eEIJKLMNOGHinZ";
/// Modifier characters, legal in any constraint.
const MODIFIERS: &str = "=+&%";

/// Measured value ranges of the integer immediate classes. Only the letters
/// whose boundary was probed against gcc 16.2 appear here; a letter absent from
/// this table is never range-checked.
fn immediate_range(letter: char) -> Option<(i64, i64)> {
    match letter {
        'I' => Some((0, 31)),
        'J' => Some((0, 63)),
        'K' => Some((-128, 127)),
        'M' => Some((0, 3)),
        'N' => Some((0, 255)),
        'O' => Some((0, 127)),
        _ => None,
    }
}

/// Whether `expr` provably is not an integer constant expression. Conservative:
/// shapes this compiler does not reason about report `false`.
fn is_nonconstant(expr: &Expr, enums: &FxHashMap<String, i64>) -> bool {
    match expr {
        Expr::Identifier(name, _) => !enums.contains_key(name.as_str()),
        Expr::BinaryOp(_, lhs, rhs, _) => is_nonconstant(lhs, enums) || is_nonconstant(rhs, enums),
        Expr::UnaryOp(op, inner, _) => {
            !matches!(
                op,
                UnaryOp::Plus | UnaryOp::Neg | UnaryOp::BitNot | UnaryOp::LogicalNot
            ) || is_nonconstant(inner, enums)
        }
        Expr::Cast(_, inner, _, _) | Expr::BitCast(_, inner, _) => is_nonconstant(inner, enums),
        Expr::Conditional(cond, then, els, _) => {
            is_nonconstant(cond, enums) || is_nonconstant(then, enums) || is_nonconstant(els, enums)
        }
        Expr::Comma(_, last, _) => is_nonconstant(last, enums),
        Expr::FunctionCall(..)
        | Expr::ArraySubscript(..)
        | Expr::MemberAccess(..)
        | Expr::PointerMemberAccess(..)
        | Expr::Assign(..)
        | Expr::CompoundAssign(..)
        | Expr::PostfixOp(..)
        | Expr::StmtExpr(..)
        | Expr::VaArg(..)
        | Expr::Deref(..) => true,
        _ => false,
    }
}

/// Classify an operand expression for the immediate-range checks.
pub fn constness(expr: &Expr, enums: &FxHashMap<String, i64>) -> Constness {
    // `comma` is the only binary operator the constant folder does not model,
    // and `is_nonconstant` already follows its right operand.
    let _ = BinOp::Add;
    if let Some(value) = super::Parser::eval_const_int_expr_with_enums(expr, Some(enums), None) {
        return Constness::Value(value);
    }
    if is_nonconstant(expr, enums) {
        Constness::NonConstant
    } else {
        Constness::Unknown
    }
}

/// Validate one constraint string.
fn check_constraint(
    operand: &Operand<'_>,
    index: usize,
    is_output: bool,
    is_x86: bool,
    problems: &mut Vec<Diagnostic>,
) {
    let constraint = operand.constraint;
    let body = constraint.trim_start_matches(|c| MODIFIERS.contains(c));
    for m in constraint[..constraint.len() - body.len()].chars() {
        // `=` and `+` say how the operand is written to; on an input they
        // contradict the operand's role and GCC rejects the statement.
        if (m == '=' || m == '+') && !is_output {
            problems.push(Diagnostic::error(
                format!("input operand constraint contains '{m}'"),
                Some(index),
            ));
            return;
        }
    }
    // Explicit register (`{rax}`), condition code (`@cc`), or matching (`0`).
    if body.starts_with('{') || body.starts_with("@cc") {
        return;
    }
    if body.is_empty() || body.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return;
    }
    // The x87 top-of-stack classes are whole-constraint specials, not letters to
    // be searched for (`"t"` = st(0), `"u"` = st(1), handled by
    // `classify_constraint` before it walks the characters).
    if body == "t" || body == "u" {
        return;
    }
    if !is_x86 {
        // Other targets validate the `=`/`+` rule only: their constraint
        // letters are not measured here and must not be second-guessed.
        return;
    }
    // Alternatives: a constraint string is a comma-free list of alternatives
    // (`"Ir"` = "any register or an immediate"). GCC only rejects the operand
    // when *every* alternative is impossible, so an immediate-only letter is
    // judged only when the string offers nothing else.
    let mut has_class = false;
    let mut immediate_only: Option<char> = None;
    let mut letters = body.chars().peekable();
    while let Some(letter) = letters.next() {
        if letter.is_ascii_digit() || MODIFIERS.contains(letter) {
            continue;
        }
        // `Yz` / `Yk` / `Ym`: the letter after `Y` refines the register class
        // instead of naming a class of its own.
        if letter == 'Y' {
            letters.next();
            has_class = true;
            continue;
        }
        if X86_CLASSES.contains(letter) {
            has_class = true;
        } else if X86_IMMEDIATES.contains(letter) {
            immediate_only.get_or_insert(letter);
        } else {
            problems.push(Diagnostic::error(
                "impossible constraint in 'asm'",
                Some(index),
            ));
            return;
        }
    }
    if has_class {
        return;
    }
    let Some(letter) = immediate_only else {
        return;
    };
    let unsatisfiable = match operand.constant {
        Constness::NonConstant => true,
        Constness::Value(value) => {
            immediate_range(letter).is_some_and(|(lo, hi)| !(lo..=hi).contains(&value))
        }
        Constness::Unknown => false,
    };
    if unsatisfiable {
        // GCC emits both, in this order, on one operand.
        problems.push(Diagnostic::warning(
            format!("'asm' operand {index} probably does not match constraints"),
            Some(index),
        ));
        problems.push(Diagnostic::error(
            "impossible constraint in 'asm'",
            Some(index),
        ));
    }
}

/// Template `%`-codes: `%N` must name an existing operand. A `%` modifier that
/// this compiler does not rewrite is left alone deliberately — an unknown code
/// reaching the assembler is a loud failure, not silent wrong code.
fn check_template(
    template: &str,
    operand_count: usize,
    label_count: usize,
    problems: &mut Vec<Diagnostic>,
) {
    const MODIFIED_CODES: &str = "bhwqkalPpzdcVnR";
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            i += 1;
            continue;
        }
        i += 1;
        if i >= bytes.len() {
            // A trailing `%` is consumed literally by the substitution pass.
            return;
        }
        let code = bytes[i];
        if code == b'%' {
            i += 1;
            continue;
        }
        if code.is_ascii_digit() || MODIFIED_CODES.contains(code as char) {
            if code.is_ascii_digit() && code != b'0' {
                // A plain `%<digit>` index: only the single leading digit forms
                // an index in GCC's x86 syntax.
            }
            let has_number = if code.is_ascii_digit() {
                true
            } else {
                i += 1;
                i < bytes.len() && bytes[i].is_ascii_digit()
            };
            if !has_number {
                continue;
            }
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            // `%10` is not operand ten in GCC's x86 syntax: a single digit
            // follows the `%` (or the modifier), so only the first digit counts.
            let index: usize = template[start..start + 1].parse().unwrap_or(usize::MAX);
            // `%lN` / `%LN` name a *label* of an `asm goto`, not an operand;
            // GCC numbers them in a separate space that is empty for plain
            // `asm`, so the bound is the operand count plus the label count.
            let bound = if matches!(code, b'l' | b'L') {
                operand_count + label_count
            } else {
                operand_count
            };
            if index >= bound {
                problems.push(Diagnostic::error(
                    "invalid 'asm': operand number out of range",
                    None,
                ));
                return;
            }
            continue;
        }
        i += 1;
    }
}

/// Validate an `asm` statement: `outputs` first, then `inputs`, exactly the
/// order GCC numbers the operands in.
pub fn validate(
    template: &str,
    outputs: &[Operand<'_>],
    inputs: &[Operand<'_>],
    label_count: usize,
    is_x86: bool,
) -> Vec<Diagnostic> {
    let mut problems = Vec::new();
    for (i, op) in outputs.iter().enumerate() {
        check_constraint(op, i, true, is_x86, &mut problems);
    }
    for (i, op) in inputs.iter().enumerate() {
        check_constraint(op, outputs.len() + i, false, is_x86, &mut problems);
    }
    // The `%`-code set is measured on x86 only: other back ends map operands
    // through their own substitution pass, so a valid aarch64/RISC-V template
    // must not be second-guessed here.
    if is_x86 && problems.iter().all(|p| !p.is_error) {
        check_template(
            template,
            outputs.len() + inputs.len(),
            label_count,
            &mut problems,
        );
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op<'a>(constraint: &'a str, constant: Constness) -> Operand<'a> {
        Operand {
            constraint,
            constant,
        }
    }

    fn ok(c: impl Into<i64>) -> Constness {
        Constness::Value(c.into())
    }

    fn messages(p: &[Diagnostic]) -> Vec<&str> {
        p.iter().map(|d| d.message.as_str()).collect()
    }

    #[test]
    fn equal_modifier_on_input_is_an_error() {
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("=r", Constness::Unknown)],
            0,
            true,
        );
        assert_eq!(messages(&p), ["input operand constraint contains '='"]);
        assert!(p[0].is_error);
        assert_eq!(p[0].operand, Some(1));
    }

    #[test]
    fn plus_modifier_on_input_is_an_error() {
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("+r", Constness::Unknown)],
            0,
            true,
        );
        assert_eq!(messages(&p), ["input operand constraint contains '+'"]);
    }

    #[test]
    fn plus_modifier_on_output_is_fine() {
        let p = validate("", &[op("+r", Constness::Unknown)], &[], 0, true);
        assert!(p.is_empty());
    }

    #[test]
    fn unmodelled_letters_are_refused_not_reclassified() {
        // Every one of these is the `_ => {}` fallback in the x86 classifier.
        for letter in ["z", "j", "k", "h", "w", "P", "T", "W", "C"] {
            let p = validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op(letter, Constness::Unknown)],
                0,
                true,
            );
            assert_eq!(messages(&p), ["impossible constraint in 'asm'"], "{letter}");
        }
    }

    #[test]
    fn immediate_with_nonconstant_operand_is_rejected_like_gcc() {
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("I", Constness::NonConstant)],
            0,
            true,
        );
        assert_eq!(
            messages(&p),
            [
                "'asm' operand 1 probably does not match constraints",
                "impossible constraint in 'asm'"
            ]
        );
        assert!(!p[0].is_error && p[1].is_error);
    }

    #[test]
    fn immediate_range_is_measured() {
        for (c, good, bad) in [
            ("I", 31, 32),
            ("J", 63, 64),
            ("K", -128, -129),
            ("M", 3, 4),
            ("N", 255, 256),
            ("O", 127, 128),
        ] {
            assert!(
                validate(
                    "",
                    &[op("=r", Constness::Unknown)],
                    &[op(c, ok(good))],
                    0,
                    true
                )
                .is_empty(),
                "{c}({good}) is in range"
            );
            let p = validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op(c, ok(bad))],
                0,
                true,
            );
            assert_eq!(p.len(), 2, "{c}({bad}) is out of range");
        }
    }

    #[test]
    fn unmeasured_immediates_are_left_alone() {
        // `e`, `Z` and `n` accept 300; `E`, `L`, `G`, `H` are not probed here.
        for c in ["e", "Z", "n", "i"] {
            assert!(
                validate(
                    "",
                    &[op("=r", Constness::Unknown)],
                    &[op(c, ok(300))],
                    0,
                    true
                )
                .is_empty(),
                "{c}(300) is legal"
            );
        }
    }

    #[test]
    fn a_class_alternative_softens_the_immediate_rule() {
        // `"Ir"` with a variable is what `btsq %2, %1` in the kernel writes:
        // the `r` alternative satisfies the operand, so nothing is diagnosed.
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("Ir", Constness::NonConstant)],
            0,
            true,
        );
        assert!(p.is_empty(), "{p:?}");
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("I", Constness::NonConstant)],
            0,
            true,
        );
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn unknown_constant_folds_are_not_second_guessed() {
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("I", Constness::Unknown)],
            0,
            true,
        );
        assert!(p.is_empty());
    }

    #[test]
    fn template_operand_numbers_are_bounded() {
        assert!(
            validate(
                "mov %1, %0",
                &[op("=r", Constness::Unknown)],
                &[op("r", Constness::Unknown)],
                0,
                true
            )
            .is_empty()
        );
        let p = validate(
            "mov %2, %0",
            &[op("=r", Constness::Unknown)],
            &[op("r", Constness::Unknown)],
            0,
            true,
        );
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        // `%b1` names operand 1 through the `b` modifier.
        let p = validate("%b1", &[op("=r", Constness::Unknown)], &[], 0, true);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        assert!(validate("%b0", &[op("=r", Constness::Unknown)], &[], 0, true).is_empty());
    }

    #[test]
    fn asm_goto_labels_are_not_operands() {
        // `asm goto("jmp %l0" :::: done)` names label 0, not operand 0.
        assert!(validate("jmp %l0", &[], &[], 1, true).is_empty());
        assert!(validate("jmp %l1", &[], &[], 2, true).is_empty());
        let p = validate("jmp %l1", &[], &[], 1, true);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        // A plain `asm` labels nothing, so the same template is out of range.
        let p = validate("jmp %l0", &[], &[], 0, true);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
    }

    #[test]
    fn x87_register_class_is_implemented_not_impossible() {
        // gcc 16.2 accepts "=f"/"f"/"t"/"u"; the x86 back ends classify them
        // as x87 stack slots, so the validator must not refuse them.
        for c in ["=f", "f", "+f", "=t", "=u", "%f"] {
            let out = if c.contains('=') || c.contains('+') {
                c
            } else {
                "=r"
            };
            let p = validate(
                "",
                &[op(out, Constness::Unknown)],
                &[op(c, Constness::Unknown)],
                0,
                true,
            );
            assert!(
                p.iter()
                    .all(|d| d.message != "impossible constraint in 'asm'"),
                "{c}: {p:?}"
            );
        }
    }

    #[test]
    fn template_passes_through_what_it_cannot_judge() {
        for t in ["%%eax", "%{mov|jmp%} %0", "mov %c0, %k0", "nop %"] {
            assert!(
                validate(t, &[op("=r", Constness::Unknown)], &[], 0, true).is_empty(),
                "{t}"
            );
        }
    }

    #[test]
    fn modelled_constraints_pass() {
        for c in [
            "r", "m", "g", "rm", "=r", "+r", "&r", "Yz", "x", "v", "Q", "A", "X", "t", "u",
            "{rax}", "@ccz", "0", "i", "f",
        ] {
            let p = validate("", &[op(c, Constness::Unknown)], &[], 0, true);
            assert!(p.is_empty(), "{c}: {p:?}");
        }
    }

    #[test]
    fn non_x86_targets_check_only_the_modifier_rule() {
        assert!(
            validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op("w", Constness::Unknown)],
                0,
                false
            )
            .is_empty()
        );
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("=r", Constness::Unknown)],
            0,
            false,
        );
        assert_eq!(messages(&p), ["input operand constraint contains '='"]);
    }
}
