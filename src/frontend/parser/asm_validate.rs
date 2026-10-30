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
const X86_CLASSES: &str = "abcdfglmopqrvxyUADQRSVYX";
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

/// Evaluate the integer truth value of the subset of constant conditions used
/// by inline-asm operands. `__builtin_constant_p` is intentionally not part of
/// the parser's general ICE evaluator, but its truth value is known whenever
/// its argument is a folded integer constant.
fn asm_condition_value(expr: &Expr, enums: &FxHashMap<String, i64>) -> Option<i64> {
    if let Some(value) = super::Parser::eval_const_int_expr_with_enums(expr, Some(enums), None) {
        return Some(value);
    }
    if let Expr::FunctionCall(callee, args, _) = expr {
        if matches!(callee.as_ref(), Expr::Identifier(name, _) if name == "__builtin_constant_p") {
            let arg = args.first()?;
            if super::Parser::eval_const_int_expr_with_enums(arg, Some(enums), None).is_some() {
                return Some(1);
            }
            if is_nonconstant(arg, enums) {
                return Some(0);
            }
        }
    }
    None
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
            if let Some(value) = asm_condition_value(cond, enums) {
                if value != 0 {
                    is_nonconstant(then, enums)
                } else {
                    is_nonconstant(els, enums)
                }
            } else {
                is_nonconstant(cond, enums)
                    || is_nonconstant(then, enums)
                    || is_nonconstant(els, enums)
            }
        }
        Expr::Comma(_, last, _) => is_nonconstant(last, enums),
        Expr::FunctionCall(callee, args, _) => {
            // The kernel's DEFINE()/order_base_2() asm operands contain
            // __builtin_constant_p and __builtin_clz{,ll} in their expanded
            // constant-expression trees. The parser's general ICE evaluator
            // intentionally does not execute builtins, but a pure builtin
            // with a provably constant argument is not evidence that the asm
            // operand is dynamic. Leave its value Unknown for the post-opt
            // IR check; only mark it NonConstant when the argument itself is
            // known to be runtime-dependent.
            let builtin = match callee.as_ref() {
                Expr::Identifier(name, _) => name.as_str(),
                _ => return true,
            };
            match builtin {
                "__builtin_constant_p" | "__builtin_clz" | "__builtin_clzl" | "__builtin_clzll" => {
                    args.first().is_none_or(|arg| is_nonconstant(arg, enums))
                }
                _ => true,
            }
        }
        Expr::ArraySubscript(..)
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
    // Explicit register (`{rax}`) and matching (`0`) constraints have their own
    // grammar and are not letter lists.
    if body.starts_with('{') || body.starts_with('[') {
        return;
    }
    // Condition-code operands (`=@cc<suffix>`). Measured against gcc 16.2: every
    // suffix in the shared `asm_constraints::CONDITION_CODES` table is accepted,
    // and everything else -- a missing suffix, `pe`/`po`, or junk -- is rejected
    // with `unknown 'asm' flag output '<constraint>'`; an `@cc` on an input is
    // rejected with `invalid punctuation '@' in constraint` everywhere.
    //
    // This check is what makes `gcc_cc_to_x86` total. Before it, an unknown
    // suffix was silently reclassified as `equal`, so `=@ccqq` compiled to
    // `sete` -- a miscompile of a program no other compiler accepts.
    if let Some(suffix) = body.strip_prefix("@cc") {
        if !is_output {
            problems.push(Diagnostic::error(
                "invalid punctuation '@' in constraint",
                Some(index),
            ));
        } else if is_x86 && crate::common::asm_constraints::cond_code(suffix).is_none() {
            // Non-x86 targets are left alone for the reason in the module
            // header: their condition-code support is not measured here and
            // must not be second-guessed.
            problems.push(Diagnostic::error(
                format!("unknown 'asm' flag output '{constraint}'"),
                Some(index),
            ));
        }
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
        // instead of naming a class of its own. `C`-prefixed constraints are
        // the same shape (measured: gcc 16.2 accepts `"Ci"(7)` and rejects
        // `"Ci"(x)` / `"Cj"(7)`, so the pair is a class, not a letter).
        if letter == 'Y' || letter == 'C' {
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
    readwrite_count: usize,
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
            // Two different index grammars, both measured against gcc:
            //   * a *label* reference consumes the whole decimal number
            //     (`%l13` names label 13, with 13 operands before it);
            //   * a plain operand reference consumes exactly one digit --
            //     x86 gcc answers `%10` with "operand number missing after
            //     %-letter" instead of reading operand ten.
            // A single rule for both silently mis-indexes one of them, and this
            // parser used to read one digit for both, so it rejected every
            // `asm goto` whose label index reached ten.
            let is_label = matches!(code, b'l' | b'L');
            let digits = &template[start..i];
            let index: usize = if is_label {
                digits.parse().unwrap_or(usize::MAX)
            } else {
                digits[..1].parse().unwrap_or(usize::MAX)
            };
            // A `+` output occupies an output position and an input position
            // in GCC's label numbering, but LCCC's regular operand map keeps
            // it as one operand. Label references start after this extra
            // read/write position; ordinary `%N` references do not.
            let out_of_range = if is_label {
                let label_start = operand_count + readwrite_count;
                index < label_start || index >= label_start + label_count
            } else {
                index >= operand_count
            };
            if out_of_range {
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
        let readwrite_count = outputs
            .iter()
            .filter(|output| output.constraint.contains('+'))
            .count();
        check_template(
            template,
            outputs.len() + inputs.len(),
            readwrite_count,
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
        // `C` is NOT in the list: a `C`-prefixed pair is a class (measured --
        // gcc 16.2 accepts `"Ci"(7)`), so the letter alone is not evidence.
        for letter in ["z", "j", "k", "h", "w", "P", "T", "W"] {
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
    fn constant_valued_builtin_operands_are_not_misclassified_as_dynamic() {
        let span = crate::common::source::Span::dummy();
        let constant_arg = Expr::IntLiteral(512, span);
        let constant_builtin = Expr::FunctionCall(
            Box::new(Expr::Identifier("__builtin_clzll".into(), span)),
            vec![constant_arg],
            span,
        );
        assert!(!is_nonconstant(&constant_builtin, &FxHashMap::default()));

        let constant_p = Expr::FunctionCall(
            Box::new(Expr::Identifier("__builtin_constant_p".into(), span)),
            vec![Expr::IntLiteral(512, span)],
            span,
        );
        let selected_constant = Expr::Conditional(
            Box::new(constant_p),
            Box::new(Expr::IntLiteral(9, span)),
            Box::new(Expr::FunctionCall(
                Box::new(Expr::Identifier("runtime_fallback".into(), span)),
                vec![],
                span,
            )),
            span,
        );
        assert!(!is_nonconstant(&selected_constant, &FxHashMap::default()));

        let dynamic_builtin = Expr::FunctionCall(
            Box::new(Expr::Identifier("__builtin_constant_p".into(), span)),
            vec![Expr::Identifier("runtime_value".into(), span)],
            span,
        );
        assert!(is_nonconstant(&dynamic_builtin, &FxHashMap::default()));
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
    fn asm_goto_label_indices_follow_readwrite_operand_numbering() {
        // GCC positions the first label after both slots of a `+` output.
        // The explicit input remains operand `%1` in the template mapping.
        assert!(
            validate(
                "mov %0, %0; test %1, %1; jz %l3",
                &[op("+r", Constness::Unknown)],
                &[op("r", Constness::Unknown)],
                1,
                true,
            )
            .is_empty()
        );
        let p = validate(
            "jmp %l2",
            &[op("+r", Constness::Unknown)],
            &[op("r", Constness::Unknown)],
            1,
            true,
        );
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
    }

    #[test]
    fn asm_goto_labels_are_not_operands() {
        // `asm goto("jmp %l0" :::: done)` names label 0, not operand 0.
        assert!(validate("jmp %l0", &[], &[], 1, true).is_empty());
        assert!(validate("jmp %l1", &[], &[], 2, true).is_empty());
        let p = validate("jmp %l0", &[op("=r", Constness::Unknown)], &[], 1, true);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        assert!(validate("jmp %l1", &[op("=r", Constness::Unknown)], &[], 1, true).is_empty());
        let p = validate("jmp %l1", &[], &[], 1, true);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        // A plain `asm` labels nothing, so the same template is out of range.
        let p = validate("jmp %l0", &[], &[], 0, true);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
    }

    #[test]
    fn label_indices_are_multi_digit_like_gcc() {
        // Measured: with 13 inputs, `gcc -S` accepts `%l13` (it emits the
        // label's own name) and rejects `%l12`. Reading a single digit would
        // accept `%l12` (as label 2) and reject `%l13` (as label 1), i.e. it
        // would invert both answers and false-reject valid `asm goto` code that
        // names a label with a two-digit index.
        let inputs: Vec<Operand<'_>> = (0..13).map(|_| op("r", Constness::Unknown)).collect();
        assert!(validate("jmp %l13", &[], &inputs, 1, true).is_empty());
        let p = validate("jmp %l12", &[], &inputs, 1, true);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        let p = validate("jmp %l14", &[], &inputs, 1, true);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
    }

    #[test]
    fn plain_operand_indices_stay_single_digit_like_gcc() {
        // x86 gcc answers `%10` with "operand number missing after %-letter",
        // so only one digit forms a plain operand index. With two operands
        // `%1` is in range and `%2` is not.
        let p = validate(
            "mov %1, %0",
            &[op("=r", Constness::Unknown)],
            &[op("r", Constness::Unknown)],
            0,
            true,
        );
        assert!(p.is_empty());
        let p = validate(
            "mov %2, %0",
            &[op("=r", Constness::Unknown)],
            &[op("r", Constness::Unknown)],
            0,
            true,
        );
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
    }

    #[test]
    fn flag_output_suffixes_are_exactly_the_measured_gcc_set() {
        // Every suffix in the shared table is accepted by gcc 16.2 (and by
        // clang); the table is what the validator, `setcc` emission and the
        // asm-goto fusion all read, so this pins that they cannot disagree.
        for suffix in [
            "e", "z", "ne", "nz", "s", "ns", "o", "no", "c", "nc", "a", "nbe", "ae", "nb", "b",
            "nae", "be", "na", "g", "nle", "ge", "nl", "l", "nge", "le", "ng", "p", "np",
        ] {
            let constraint = format!("=@cc{suffix}");
            let p = validate("", &[op(&constraint, Constness::Unknown)], &[], 0, true);
            assert!(p.is_empty(), "{constraint} must be accepted");
        }
        // gcc: `unknown 'asm' flag output '<constraint>'` for all four.
        for bad in ["=@cc", "=@ccxx", "=@ccpe", "=@ccpo"] {
            let p = validate("", &[op(bad, Constness::Unknown)], &[], 0, true);
            let want = format!("unknown 'asm' flag output '{bad}'");
            assert_eq!(messages(&p), [want.as_str()], "{bad}");
        }
        // gcc: `invalid punctuation '@' in constraint` on an input operand.
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("@ccz", Constness::Unknown)],
            0,
            true,
        );
        assert_eq!(messages(&p), ["invalid punctuation '@' in constraint"]);
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
