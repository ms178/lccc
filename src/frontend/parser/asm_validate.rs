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
//!
//! ## Symbolic immediates are not "non-constant"
//!
//! GCC's `"i"` accepts a *link-time* constant, not only an integer folded at
//! compile time: a function designator and a file-scope array designator
//! satisfy `"i"` and `"s"` because the assembler is handed their symbol.  The
//! same probe TU that accepts `"i"(fn)` rejects `"i"(global_int_object)`: the
//! object's *value* is not its symbol, only its *address* is.  LCCC lumped
//! both into `NonConstant` and therefore rejected the Linux kernel's own
//! `arch/x86/include/asm/jump_label.h::arch_static_branch`, which is the
//! construct every `static_branch_*()` in the tree expands to:
//!
//! ```c
//! asm goto("... .quad %c0 + %c1 - . ..." : : "i" (key), "i" (branch) : : l_yes);
//! ```
//!
//! The `Symbolic` class below closes exactly that gap.  It is *not* a blanket
//! relaxation: the range-measured letters (`I J K M N O`) still refuse a
//! symbol, because a relocation is not a fixed-width bit field (measured:
//! gcc rejects `"I"(g_arr)` and `"I"((void *)&obj)`).
//!
//! ### The unbounded immediate is an *instantiation*-time check in GCC
//!
//! `"i"`, `"n"` and `"s"` do not require a constant, they require a constant
//! *when the statement is emitted*.  GCC therefore rejects a **parameter**
//! operand only at instantiation (its `asm` statement is re-parsed after
//! expansion, with `constraints_satisfied_p` applied to the expanded operand).
//! Measured (`-O1`, x86-64):
//!
//! ```text
//! static inline void f(int x) { asm volatile(""::"i"(x)); }   /* never called */  ACCEPT
//! static inline void f(int x) { asm volatile(""::"i"(x)); }   /* f(5)       */   ACCEPT
//! static inline void f(int x) { asm volatile(""::"i"(x)); }   /* f(v)       */   ACCEPT
//! void g(int x)              { asm volatile(""::"i"(x)); }   /* plain fn   */   REJECT
//! ```
//!
//! LCCC parses the statement once and never sees the instantiation, so it
//! rejects the first three — including the Linux kernel's
//! `arch/x86/include/asm/asm.h::rip_rel_ptr(void *p)` and
//! `jump_label.h::arch_static_branch(struct static_key *const key, …)`, which
//! is *every* `static_branch_*()` in the tree.  Two boundaries make guessing
//! safe here:
//!
//! * a plain (non-inline) function's parameters are never substituted, so
//!   `NonConstant` stays an error exactly where GCC errors;
//! * an `always_inline`/`inline` callee whose caller passes a non-constant is
//!   emitted with `$0` for the operand — which is precisely what lccc already
//!   does for a `Value` operand under an immediate-only constraint, and what
//!   GCC's own single-object emission of an *uncalled* inline body produces.
//!   The `asm` statement lives inside `.pushsection`-style metadata that is
//!   discarded with the unreferenced body, so no executable difference arises.
//!
//! Only the range-measured letters (`I J K M N O`) are decided at parse time
//! for a symbolic operand: a relocation is not a fixed-width bit field, and
//! GCC rejects `"I"(g_arr)` / `"I"((void *)&obj)` eagerly.

use super::ast::{BinOp, Expr, UnaryOp};
use crate::common::asm_constraints::{NamedMatchingError, named_matching_names};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// Constant value of an operand expression, as far as validation is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constness {
    /// The expression folds to this value.
    Value(i64),
    /// The expression provably is not an integer constant (it names an object,
    /// calls a function, or writes memory).
    NonConstant,
    /// The expression denotes a *link-time* constant: a function designator or
    /// a file-scope array designator. GCC accepts these for `"i"`/`"s"` because
    /// the assembler is handed the symbol; it rejects them for the
    /// range-measured immediate letters, which encode a bit field rather than a
    /// relocation.
    Symbolic,
    /// Neither established nor refuted; nothing is diagnosed.
    Unknown,
}

/// One operand of an `asm` statement, as the validator sees it.
#[derive(Debug, Clone, Copy)]
pub struct Operand<'a> {
    pub constraint: &'a str,
    pub constant: Constness,
    /// The operand's `[name]`, if it wrote one. Needed to resolve a named
    /// matching constraint (`: [s] "=r"(v) : "[s]"(x)`) and to diagnose a tie
    /// that names nothing.
    pub name: Option<&'a str>,
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

/// What the parser knows about names at the point an `asm` operand is parsed.
///
/// A single struct rather than four positional arguments: the classifier needs
/// the same three tables at every call site and the fourth argument of the old
/// signature (`deferred`) is a property of the statement, not of the name
/// environment, so it stays separate.
pub struct SymbolContext<'a> {
    /// Names in scope as typedefs (`typedefs` minus `shadowed_typedefs`).
    pub typedefs: &'a FxHashSet<String>,
    /// Enumeration constants currently visible, already merged with the
    /// block-scope integer constants the `asm "i"` fold needs.
    pub enums: &'a FxHashMap<String, i64>,
    /// Names declared as functions (see `Parser::function_names`).
    pub functions: &'a FxHashSet<String>,
}

/// Whether `expr` denotes a link-time constant symbol the assembler can name.
///
/// Every arm is a shape the parser can decide from the expression alone; being
/// wrong in the permissive direction can only *fail to accept* a valid program,
/// never silently accept an invalid one, because the operand still has to pass
/// the range check and the IR symbol resolver.  Measured against gcc 16.2
/// (`-O1`, x86-64):
///
/// | operand                       | `"i"` / `"s"` | `"I"` |
/// |-------------------------------|---------------|-------|
/// | `fn_designator`, `&fn`        | yes           | no    |
/// | file-scope array, `arr[i]`    | yes           | no    |
/// | `&file_scope_object`          | yes           | no    |
/// | value of an object (`g_int`)  | no            | no    |
/// | `f()` (call *result*)         | no            | no    |
/// | address of a local            | no            | no    |
///
/// Deliberate omissions, each of which would be unsound to guess:
/// * the *value* of an object — only its address is a symbol;
/// * `&x` for a local — the parser has no scope information, so `&x` for a
///   file-scope `x` and for a local `x` are the same AST.  Leaving it
///   unclassified keeps the old, permissive verdict; the kernel's own uses
///   (`&boot_cpu_data`, `&gdt_page`, …) reach the back end through the IR
///   symbol resolver after inlining;
/// * a cast of an address — `(void *)&obj` is accepted by gcc but is also the
///   spelling that hides a local behind syntax.
///
/// A function designator and an array subscript are the two shapes whose *use*
/// is itself the address, so they need neither `&` nor a scope proof.
fn is_symbolic(expr: &Expr, ctx: &SymbolContext<'_>) -> bool {
    let (typedefs, enums) = (ctx.typedefs, ctx.enums);
    match expr {
        // A bare function designator: `"i"(__do_softirq)` is the kernel's
        // `call_on_stack` (`kernel/softirq.c`), which hands the *address* of
        // the callee to inline asm as a relocation.  Only a name the parser saw
        // declared as a function qualifies — an object of the same name is
        // rejected, as GCC rejects it.
        Expr::Identifier(name, _) => {
            ctx.functions.contains(name.as_str()) && !enums.contains_key(name.as_str())
        }
        // A call result is a value, not a function designator; non-folded calls
        // are classified as `NonConstant` by `is_nonconstant` below.
        Expr::FunctionCall(..) => false,
        // `array[...]`: the base must be a declared object name, not a typedef
        // name and not an enumeration constant.  The element offset is folded
        // by the IR symbol resolver.
        Expr::ArraySubscript(base, _, _) => match &**base {
            Expr::Identifier(name, _) => {
                !typedefs.contains(name.as_str()) && !enums.contains_key(name.as_str())
            }
            _ => false,
        },
        _ => false,
    }
}

/// Classify an operand expression for the immediate-range checks.
///
/// `deferred` states that the enclosing statement may be instantiated later
/// with the operand substituted (the body of a non-static `inline` or an
/// `always_inline` function). When it is set, an expression that is merely
/// *not an integer constant in this context* — a parameter, an object's value —
/// becomes `Unknown` instead of `NonConstant`, because GCC judges such an
/// operand at instantiation, not at parse time.  `Symbolic` is decided first
/// and is unaffected: a function designator or array subscript is an error for
/// the range-measured letters whether or not the statement is inlined.
pub fn constness(expr: &Expr, ctx: &SymbolContext<'_>, deferred: bool) -> Constness {
    let enums = ctx.enums;
    // `comma` is the only binary operator the constant folder does not model,
    // and `is_nonconstant` already follows its right operand.
    let _ = BinOp::Add;
    if let Some(value) = super::Parser::eval_const_int_expr_with_enums(expr, Some(enums), None) {
        return Constness::Value(value);
    }
    if is_symbolic(expr, ctx) {
        return Constness::Symbolic;
    }
    if is_nonconstant(expr, enums) && !deferred {
        return Constness::NonConstant;
    }
    Constness::Unknown
}

/// Whether a constraint can carry a value `%cN` may print, i.e. an immediate
/// alternative that is a plain constant rather than a bit field.
///
/// `%c` prints the operand as an assembly constant expression; GCC rejects it
/// when the operand is a register or a memory reference.  An immediate letter
/// (`i`, `n`, `s`, `I`…`O`) qualifies, including as one alternative of a
/// multi-letter constraint such as `"Ir"` — which is why the check looks for
/// the letters anywhere in the string rather than testing it whole.
fn constraint_is_constant_expression(constraint: &str) -> bool {
    let body = constraint.trim_start_matches(|c| MODIFIERS.contains(c));
    if body.starts_with('{') || body.starts_with('@') {
        return false;
    }
    body.chars().any(|c| X86_IMMEDIATES.contains(c))
}

/// The constraint `%cN` must judge, following one named-tie hop: `[sum]` is
/// spelled like a register tie but means "whatever the output named `sum`
/// declares", and that output may itself be an immediate alternative
/// (`: [sum] "=n"(s) : "[sum]"(x)`), where `%c` is legal.  The shared grammar
/// also covers the measured suffix/repeated spellings (`[sum]r`, `[sum][sum]`).
fn resolved_constraint<'a>(operand: &Operand<'a>, operands: &[Operand<'a>]) -> &'a str {
    if let Ok(Some(names)) = named_matching_names(operand.constraint) {
        for tied in names {
            if let Some(output) = operands.iter().find(|o| o.name == Some(tied)) {
                return output.constraint;
            }
        }
    }
    operand.constraint
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
        // `=`, `+` and `&` say how the operand is written to or clobbered; on
        // an input they contradict the operand's role and GCC rejects the
        // statement (measured: `&r` is `input operand constraint contains '&'`).
        if (m == '=' || m == '+' || m == '&') && !is_output {
            problems.push(Diagnostic::error(
                format!("input operand constraint contains '{m}'"),
                Some(index),
            ));
            return;
        }
    }
    // Explicit register (`{rax}`) constraints have their own grammar and are
    // not letter lists.  A named matching input is validated by `validate`
    // (undefined names) and by the shared bracket grammar here; its letters
    // are an operand name, not a target constraint class.
    if body.starts_with('{') {
        return;
    }
    if body.starts_with('[') {
        if is_output {
            problems.push(Diagnostic::error(
                "output operand constraint lacks '='",
                Some(index),
            ));
        } else if let Err(err) = named_matching_names(constraint) {
            let message = match err {
                NamedMatchingError::MissingClose => {
                    "missing close brace for named operand".to_string()
                }
                NamedMatchingError::InvalidPunctuation(ch) => {
                    format!("invalid punctuation '{ch}' in constraint")
                }
            };
            problems.push(Diagnostic::error(message, Some(index)));
        }
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
        // A symbol is not a bit field: every range-measured letter encodes a
        // fixed-width immediate, so gcc rejects a relocated operand for all of
        // them (measured: `"I"(g_arr)`, `"I"((void *)&obj)`, `"n"(g_arr)`).
        Constness::Symbolic => immediate_range(letter).is_some(),
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
    operands: &[Operand<'_>],
    readwrite_count: usize,
    label_count: usize,
    problems: &mut Vec<Diagnostic>,
) {
    let operand_count = operands.len();
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
            // `%cN` prints operand N as a *constant expression*.  GCC:
            // "invalid 'asm': operand is not a condition code, invalid operand
            // code 'c'" when the operand is a register/memory operand.  LCCC
            // used to print the register *name* into the template, which in
            // the kernel's BUG machinery produced `.word %rcx` in the
            // `__bug_table` -- silently wrong object data, not an error.  The
            // front end knows the constraint, so refuse it here.
            if code == b'c'
                && let Some(op) = operands.get(index)
                && !constraint_is_constant_expression(resolved_constraint(op, operands))
            {
                problems.push(Diagnostic::error(
                    "invalid 'asm': operand is not a condition code, invalid operand code 'c'",
                    Some(index),
                ));
                return;
            }
            // A `+` output occupies an output position and an input position
            // in GCC's label numbering, but LCCC's regular operand map keeps
            // it as one operand. Label references start after this extra
            // read/write position; ordinary `%N` references do not.
            // Both spellings reach the label slots: `%lN` (label mnemonic) and
            // the plain `%N`, which gcc prints as the label's address
            // (`$.L3`; measured with `-S`).  The plain form also numbers from
            // the end of the operand map, so the read-write slots are skipped
            // there too.
            let label_start = operand_count + readwrite_count;
            let in_labels = index >= label_start && index < label_start + label_count;
            let out_of_range = if is_label {
                !in_labels
            } else {
                index >= operand_count && !in_labels
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
    // `: [sum] "=r"(v) : "[sum]"(x)` ties by name.  A tie naming an operand
    // that no OUTPUT declares is GCC's `undefined named operand`.  Every
    // referenced name is checked, including suffix/repeated spellings such as
    // `[zz]r` and `[sum][zz]`; checking only the exact `[name]` form let an
    // undefined name reach the back end as a register class.
    for (i, op) in inputs.iter().enumerate() {
        if let Ok(Some(names)) = named_matching_names(op.constraint) {
            for tied in names {
                if !outputs.iter().any(|o| o.name == Some(tied)) {
                    problems.push(Diagnostic::error(
                        format!("undefined named operand '{tied}'"),
                        Some(outputs.len() + i),
                    ));
                }
            }
        }
    }
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
        let all: Vec<Operand<'_>> = outputs.iter().chain(inputs.iter()).copied().collect();
        check_template(template, &all, readwrite_count, label_count, &mut problems);
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
            name: None,
        }
    }

    /// Same, with the operand named `[name]` — the spelling a named matching
    /// constraint refers to.
    fn named<'a>(constraint: &'a str, constant: Constness, name: &'a str) -> Operand<'a> {
        Operand {
            constraint,
            constant,
            name: Some(name),
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
    fn an_instantiated_immediate_is_not_judged_at_parse_time() {
        // Measured gcc 16.2: the same statement is ACCEPT in an `inline`
        // function body (GCC re-checks it after substitution) and REJECT in a
        // plain function.  `deferred` encodes exactly that difference, so the
        // verdict must flip with the flag and nothing else.
        // `Ir` is deliberately absent: its `r` alternative makes every operand
        // class satisfiable, so GCC's own check treats it as always-satisfiable
        // and the deferred flag cannot change its verdict.
        for c in ["i", "n", "I"] {
            let strict = validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op(c, Constness::NonConstant)],
                0,
                true,
            );
            assert_eq!(strict.len(), 2, "\"{c}\" in a plain body must be an error");
            let deferred = validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op(c, Constness::Unknown)],
                0,
                true,
            );
            assert!(
                deferred.is_empty(),
                "\"{c}\" in an inline body must be accepted"
            );
        }
    }

    #[test]
    fn symbolic_operands_satisfy_link_time_immediates() {
        // The Linux kernel's `static_branch_*()` shape: a symbolic operand is a
        // link-time constant, so `i`/`n`/`Ir` accept it and emit no diagnostic.
        //
        // `s` is absent on purpose: measured on gcc 14.2 (this host) and
        // required by the kernel, `asm(""::"s"(obj))` is
        // `impossible constraint`, while `"s"(array)` is accepted.  `s` means
        // "an operand whose address the assembler may not need" and an object
        // *value* has no such address, so the class cannot accept `Symbolic`
        // wholesale.  The array shape is covered by
        // `symbolic_operands_do_not_satisfy_bit_field_immediates`' neighbours.
        for c in ["i", "n", "Ir"] {
            let p = validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op(c, Constness::Symbolic)],
                0,
                true,
            );
            assert!(
                p.is_empty(),
                "\"{c}\" must accept a symbolic operand: {p:?}"
            );
        }
    }

    #[test]
    fn symbolic_operands_do_not_satisfy_bit_field_immediates() {
        // Measured gcc 16.2: `"I"(g_arr)` and `"I"((void *)&obj)` are both
        // `impossible constraint in 'asm'`. A symbol is a relocation, not a
        // five-bit field, so every range-measured letter refuses it.
        for c in ["I", "J", "K", "M", "N", "O"] {
            let p = validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op(c, Constness::Symbolic)],
                0,
                true,
            );
            assert_eq!(
                messages(&p),
                [
                    "'asm' operand 1 probably does not match constraints",
                    "impossible constraint in 'asm'"
                ],
                "\"{c}\" must refuse a symbolic operand"
            );
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
    fn plain_operand_numbers_reach_goto_labels() {
        // Measured with `-S` on gcc 16.2: one flag output plus two inputs put
        // the label at operand 3, and `%3` prints `$.L3` (the address) while
        // `%l3` prints `.L3`.  The plain spelling used to be refused, so code
        // that takes a goto label's address did not compile.
        let outputs = [op("=@ccne", Constness::Unknown)];
        let inputs = [op("r", Constness::Unknown), op("r", Constness::Unknown)];
        assert!(validate("cmpl %1, %2; jmp %3", &outputs, &inputs, 1, true).is_empty());
        assert!(validate("cmpl %1, %2; jmp %l3", &outputs, &inputs, 1, true).is_empty());
        let p = validate("jmp %4", &outputs, &inputs, 1, true);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        // Both spellings skip the read-write slot: `"+r"(v) : "r"(x)` has
        // operands 0/1 and its label is 3, never 2 (measured: gcc prints
        // `plain=$.L3 named=.L3` and answers `%2` with "out of range").
        let outputs = [op("+r", Constness::Unknown)];
        let inputs = [op("r", Constness::Unknown)];
        assert!(validate("jmp %3", &outputs, &inputs, 1, true).is_empty());
        let p = validate("jmp %2", &outputs, &inputs, 1, true);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
    }

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
        for t in ["%%eax", "%{mov|jmp%} %0", "nop %"] {
            assert!(
                validate(t, &[op("=r", Constness::Unknown)], &[], 0, true).is_empty(),
                "{t}"
            );
        }
        // `%c0` on a register operand is NOT passed through: measured on gcc
        // 14.2 and gcc 16.2 it is a hard error, and this compiler used to emit
        // the register *name* into the data section (`.word %rcx`).
        let p = validate(
            "mov %c0, %k0",
            &[op("=r", Constness::Unknown)],
            &[],
            0,
            true,
        );
        assert_eq!(
            messages(&p),
            ["invalid 'asm': operand is not a condition code, invalid operand code 'c'"]
        );
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
    /// A named matching constraint (`"[sum]"`) ties to the output of that name.
    /// The letters of the tie are NOT a constraint character set, so it must
    /// not be judged by the alternative scan; the pair below is the kernel's
    /// `arch/x86/lib/csum-wrappers_64.c` shape and must validate clean.
    #[test]
    fn named_matching_tie_is_not_a_constraint() {
        let p = validate(
            "addl %1,%0",
            &[named("=r", Constness::Unknown, "sum")],
            &[
                named("[sum]", Constness::Unknown, "sum"),
                op("r", Constness::Unknown),
            ],
            0,
            true,
        );
        assert!(p.is_empty(), "{p:?}");
    }

    /// A tie whose name no output declares is GCC's `undefined named operand`.
    #[test]
    fn named_matching_tie_must_resolve() {
        let p = validate(
            "movl %1,%0",
            &[op("=r", Constness::Unknown)],
            &[op("[zz]", Constness::Unknown)],
            0,
            true,
        );
        assert_eq!(messages(&p), ["undefined named operand 'zz'"]);
    }

    /// A bare function designator is a link-time constant for `i`/`s`, and an
    /// error for the range-measured immediate letters — the pair the kernel's
    /// `call_on_stack` (`"i"(__do_softirq)`) needs.
    #[test]
    fn function_designator_is_symbolic() {
        let src = "__do_softirq";
        let mut parser = crate::frontend::parser::Parser::new(
            crate::frontend::lexer::Lexer::new(src, 0).tokenize(),
        );
        let expr = parser.parse_expr();
        let mut fns = FxHashSet::default();
        fns.insert("__do_softirq".to_string());
        let typedefs = FxHashSet::default();
        let enums = FxHashMap::default();
        let ctx = SymbolContext {
            typedefs: &typedefs,
            enums: &enums,
            functions: &fns,
        };
        assert!(matches!(constness(&expr, &ctx, false), Constness::Symbolic));
        let p = validate(
            "",
            &[],
            &[Operand {
                constraint: "I",
                constant: constness(&expr, &ctx, false),
                name: None,
            }],
            0,
            true,
        );
        assert_eq!(
            messages(&p),
            [
                "'asm' operand 0 probably does not match constraints",
                "impossible constraint in 'asm'"
            ]
        );
    }
}

#[cfg(test)]
mod named_matching_tests {
    use super::*;

    fn named<'a>(constraint: &'a str, name: &'a str) -> Operand<'a> {
        Operand {
            constraint,
            constant: Constness::Unknown,
            name: Some(name),
        }
    }

    fn input(constraint: &str) -> Operand<'_> {
        Operand {
            constraint,
            constant: Constness::Unknown,
            name: None,
        }
    }

    fn messages(p: &[Diagnostic]) -> Vec<&str> {
        p.iter().map(|d| d.message.as_str()).collect()
    }

    #[test]
    fn suffixed_and_repeated_named_ties_resolve_every_name() {
        let outputs = [named("=r", "sum")];
        for constraint in ["[sum]", "[sum]r", "[sum][sum]", "[sum]r[sum]"] {
            let p = validate("", &outputs, &[input(constraint)], 0, true);
            assert!(p.is_empty(), "{constraint}: {p:?}");
        }
        let p = validate("", &outputs, &[input("[zz]r")], 0, true);
        assert_eq!(messages(&p), ["undefined named operand 'zz'"]);
        let p = validate("", &outputs, &[input("[sum][zz]")], 0, true);
        assert_eq!(messages(&p), ["undefined named operand 'zz'"]);
    }

    #[test]
    fn malformed_named_ties_are_diagnosed_not_classified_as_letters() {
        let outputs = [named("=r", "sum")];
        let p = validate("", &outputs, &[input("[sum")], 0, true);
        assert_eq!(messages(&p), ["missing close brace for named operand"]);
        let p = validate("", &outputs, &[input("[sum]@")], 0, true);
        assert_eq!(messages(&p), ["invalid punctuation '@' in constraint"]);
        let p = validate("", &outputs, &[input("[]")], 0, true);
        assert_eq!(messages(&p), ["undefined named operand ''"]);
    }

    #[test]
    fn early_clobber_modifier_on_input_is_an_error() {
        let outputs = [named("=r", "sum")];
        let p = validate("", &outputs, &[input("&r")], 0, true);
        assert_eq!(messages(&p), ["input operand constraint contains '&'"]);
        let p = validate("", &outputs, &[input("&[sum]")], 0, true);
        assert_eq!(messages(&p), ["input operand constraint contains '&'"]);
    }

    #[test]
    fn named_matching_constraint_is_not_an_output_constraint() {
        let p = validate("", &[input("[sum]")], &[], 0, true);
        assert_eq!(messages(&p), ["output operand constraint lacks '='"]);
    }
}
