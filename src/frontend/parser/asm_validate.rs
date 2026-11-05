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

/// The target whose `asm` grammar the validator is applying.
///
/// Only the operand classes that this compiler implements are enumerated; the
/// enumeration exists because `=@cc<cond>` is the one operand class whose
/// *accepted spelling set* differs per target and whose back ends implement it
/// independently. Three states are enough:
///
/// * [`AsmTarget::X86`] — x86-64 and i686. The suffix is an x86 condition
///   suffix (`e`, `z`, `ne`, `nbe`, ...) validated against
///   [`crate::common::asm_constraints::CONDITION_CODES`], and the back ends
///   materialise the flag with `SETcc`.
/// * [`AsmTarget::Aarch64`] — the suffix is an ARM `ConditionHolds` condition
///   (`eq`, `cs`, `hi`, `le`, ...) validated against
///   [`crate::common::asm_constraints::AARCH64_FLAG_CONDITIONS`], and the back
///   end materialises it with `CSET`. The two spellings mean the same thing
///   only where they coincide (`ne`, `ge`, `le`): `"z"` names "zero clear" on
///   x86 and is not an AArch64 condition at all, `"cs"` the other way round.
/// * [`AsmTarget::Other`] — no flag-output support at all (RISC-V has no
///   condition register), so `@cc` is refused rather than ignored.
///
/// Before this type existed the validator carried a bare `is_x86: bool` and
/// every non-x86 target inherited the x86 decision to "leave `@cc` alone": the
/// AArch64 back end then stored an uninitialised GP register as the result of
/// `asm("cmp %1,%2" : "=@cceq"(r) ...)` and the RISC-V back end classified the
/// constraint as a plain `"r"`, both silent miscompiles of programs GCC accepts
/// (AArch64) or rejects (RISC-V).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsmTarget {
    X86,
    Aarch64,
    Other,
}

impl AsmTarget {
    /// Classify an ELF `e_machine` value the way `target_elf_machine()` reports
    /// it: `EM_386` (3), `EM_X86_64` (62), `EM_AARCH64` (183). Every other
    /// machine — RISC-V (`EM_RISCV` 243) included — has no flag outputs.
    pub const fn from_elf_machine(machine: u16) -> Self {
        match machine {
            3 | 62 => Self::X86,
            183 => Self::Aarch64,
            _ => Self::Other,
        }
    }

    /// Whether the back ends of this target can snapshot the condition
    /// register after the template (`SETcc` on x86, `CSET` on AArch64).
    pub const fn supports_flag_outputs(self) -> bool {
        matches!(self, Self::X86 | Self::Aarch64)
    }

    /// The condition-suffix rule of this target, as a predicate so that
    /// `check_constraint` stays a flat sequence of `else if` arms.
    fn accepts_flag_condition(self, suffix: &str) -> bool {
        match self {
            Self::X86 => crate::common::asm_constraints::cond_code(suffix).is_some(),
            Self::Aarch64 => {
                crate::common::asm_constraints::aarch64_flag_condition(suffix).is_some()
            }
            Self::Other => false,
        }
    }
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
// `s` is GCC's "symbolic/string-literal immediate" class: it is an immediate
// class for choosing an alternative, but unlike `i`/`n` it refuses a folded
// integer (`"s"(5)` is `impossible constraint`, measured gcc 16.2).  It is
// listed here so the alternative scanner sees it as an immediate letter and
// `constraint_is_immediate_only` treats the operand as an immediate.
const X86_IMMEDIATES: &str = "eEIJKLMNOGHinZs";
/// Modifier characters, legal in any constraint. `%` is GCC's commutative
/// modifier; the cost/selection modifiers are handled by CONSTRAINT_SKIP.
const MODIFIERS: &str = "=+&%";
/// GCC constraint characters that carry no register/memory class of their own:
/// cost modifiers (`? ! ^ $`), the "ignore next letter" marker (`*`), the
/// "rest is a comment" marker (`#`), and the output/early-clobber modifiers.
/// They are skipped while scanning an alternative, never second-guessed as an
/// unmodelled target class.
const CONSTRAINT_SKIP: &str = "=+&%?!^$*";

/// Split a constraint into GCC alternatives.
///
/// GCC separates alternatives with `,` and requires every operand of one `asm`
/// to declare the same number of them. Everything after the first `#` is a
/// constraint comment: ignored for choosing an alternative, but still checked
/// for stray punctuation (`"i#@@@"` is rejected by gcc 16.2, `"i#zzz"` is not).
/// The leading modifiers are stripped before splitting so `"%0,0"` yields the
/// two alternatives `0` and `0`, and `"=r,r"` yields `r` and `r`.
fn constraint_alternatives(constraint: &str) -> Vec<&str> {
    let body =
        constraint.trim_start_matches(|c| CONSTRAINT_SKIP.contains(c) || MODIFIERS.contains(c));
    let choosing = body.split('#').next().unwrap_or(body);
    choosing.split(',').collect()
}

/// Is `ch` legal in the ignored tail of a `#` constraint comment?
/// gcc still diagnoses stray punctuation there, so accepting `i#@@@` silently
/// would be a diagnostic-parity hole.
fn constraint_comment_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            ',' | '=' | '+' | '&' | '%' | '?' | '!' | '^' | '$' | '*' | '#' | '[' | ']' | '{' | '}'
        )
}

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
        // A string literal is a link-time constant object (a `.rodata` symbol):
        // gcc 16.2 accepts `"i"("x")`, `"s"("x")` and rejects `"n"("x")`.
        // Classifying it as `Symbolic` rather than `Unknown` is what gives `n`
        // its measured refusal while `i`/`s` keep their acceptance.
        Expr::StringLiteral(..) | Expr::WideStringLiteral(..) | Expr::Char16StringLiteral(..) => {
            true
        }
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
    target: AsmTarget,
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
    // Condition-code operands (`=@cc<suffix>`). Measured against gcc 16.2 on
    // x86-64: every suffix in the shared `asm_constraints::CONDITION_CODES`
    // table is accepted, and everything else -- a missing suffix, `pe`/`po`, or
    // junk -- is rejected with `unknown 'asm' flag output '<constraint>'`; an
    // `@cc` on an input is rejected with `invalid punctuation '@' in
    // constraint` everywhere.
    //
    // This check is what makes `gcc_cc_to_x86` total. Before it, an unknown
    // suffix was silently reclassified as `equal`, so `=@ccqq` compiled to
    // `sete` -- a miscompile of a program no other compiler accepts.
    //
    // The accepted *suffix set* is target-specific, and AArch64 does not
    // inherit the x86 one: GCC's AArch64 flag outputs spell the same condition
    // with the ARM `ConditionHolds` names (`eq`, `cs`, `hi`, ...), so `=@ccz`
    // -- the x86 spelling of "zero" -- is not a condition AArch64 accepts, and
    // `=@cceq` is not a condition x86 accepts. Validating the union, or
    // skipping the check on non-x86 targets as this function used to, turns a
    // user typo into a back-end `CSET`/`SETcc` with an unknown condition field
    // (an assembler error at best) or into a silently wrong register class.
    //
    // RISC-V is not a flag-output target at all: it has no condition register,
    // GCC's manual lists only ARM/AArch64, x86 and s390, and clang rejects the
    // constraint outright. The RISC-V back end used to classify `"=@ccz"` as
    // `"=r"` and hand back whatever the template left in a general register --
    // a silent miscompile -- so the operand is now refused while parsing.
    if let Some(suffix) = body.strip_prefix("@cc") {
        if !is_output {
            problems.push(Diagnostic::error(
                "invalid punctuation '@' in constraint",
                Some(index),
            ));
        } else if !target.supports_flag_outputs() {
            let message =
                format!("'{constraint}' flag output constraint is not supported on this target");
            problems.push(Diagnostic::error(message, Some(index)));
        } else if !target.accepts_flag_condition(suffix) {
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
    if target != AsmTarget::X86 {
        // Other targets validate the `=`/`+` rule only: their constraint
        // letters are not measured here and must not be second-guessed.
        return;
    }
    // GCC separates alternative constraint sets with COMMAS, and an operand is
    // accepted when at least ONE alternative is satisfiable.  Measured against
    // gcc 16.2: `"r,m"`, `"m,r"`, `"=r,r"`, `"i,r"` all compile; the old scan
    // treated `,` as an unmodelled letter and rejected them with
    // `impossible constraint in 'asm'` -- four gcc.c-torture/compile cases
    // (20000922-1.c, asmgoto-2.c, pr27528.c, pr35318.c) failed on this alone.
    let alts = constraint_alternatives(body);
    // Everything after `#` is a constraint comment for choosing an
    // alternative, but gcc still rejects stray punctuation there
    // (`"i#@@@"` is an error; `"i#zzz"` / `"i#*X"` are not).
    if let Some((_, comment)) = body.split_once('#') {
        if let Some(ch) = comment.chars().find(|c| !constraint_comment_char(*c)) {
            problems.push(Diagnostic::error(
                format!("invalid punctuation '{ch}' in constraint"),
                Some(index),
            ));
            return;
        }
    }
    // Measured gcc 16.2 semantics of an IMPOSSIBLE alternative (red-team
    // differential over 23 shapes): an unmodelled letter (`z`), a bare class
    // prefix (`Y`, `C`) and an empty alternative are all UNSATISFIABLE, not a
    // validity error -- `"z,r"` and `"r,z"` compile, `"z"` alone does not.
    // Silently reclassifying an unknown letter as GpReg would still be a
    // miscompile; this just moves the refusal to "no alternative works".
    // A completely empty constraint (`""`) is the one exception: gcc accepts
    // it as "no constraint at all".
    let choosing = body.split('#').next().unwrap_or(body);
    if choosing.is_empty() && !body.is_empty() {
        problems.push(Diagnostic::error(
            "inconsistent operand constraints in an 'asm'",
            Some(index),
        ));
        return;
    }
    let empty_constraint = body.is_empty();
    let mut any_satisfiable = false;
    let mut saw_unmodelled = false;
    for alt in &alts {
        let mut has_class = false;
        let mut immediate_only: Option<char> = None;
        let mut letters = alt.chars().peekable();
        while let Some(letter) = letters.next() {
            if letter.is_ascii_digit()
                || CONSTRAINT_SKIP.contains(letter)
                || MODIFIERS.contains(letter)
            {
                continue;
            }
            // `Yz` / `Yk` / `Ym` and `C`-prefixed pairs refine a class; the
            // letter after the prefix is not a class of its own. A prefix
            // WITHOUT that letter is impossible (`"Y"` and `"C"` are refused
            // by gcc, `"Yz"` and `"Ci"` compile).
            if letter == 'Y' || letter == 'C' {
                if letters.next().is_some() {
                    has_class = true;
                } else {
                    // A bare prefix names no class at all.
                    saw_unmodelled = true;
                }
                continue;
            }
            if X86_CLASSES.contains(letter) {
                has_class = true;
            } else if X86_IMMEDIATES.contains(letter) {
                immediate_only.get_or_insert(letter);
            } else {
                // UNMODELLED does NOT poison the whole alternative: gcc 16.2
                // accepts `rz`, `zr`, `zri`, `rzi` and `Yzz` because a class
                // letter is still present, and refuses only `z`/`zz` alone.
                // Keep scanning so a class letter later in the SAME
                // alternative can satisfy it.  A first draft that broke here
                // over-rejected all five shapes (red-team differential).
                saw_unmodelled = true;
            }
        }
        if has_class {
            any_satisfiable = true;
            break;
        }
        let Some(letter) = immediate_only else {
            // An empty alternative is unsatisfiable on its own, but harmless
            // when another alternative works (`"r,"` compiles in gcc). The
            // whole-empty constraint is the measured exception above.
            if empty_constraint {
                any_satisfiable = true;
                break;
            }
            continue;
        };
        if immediate_satisfiable(letter, operand.constant) {
            any_satisfiable = true;
            break;
        }
    }
    if !any_satisfiable {
        // gcc emits the "probably does not match constraints" warning for a
        // const/range/lone-modifier failure, but NOT when an unmodelled
        // letter is among the alternatives (`"z"`, `"z,i"`, `"C"`, `"Y"` all
        // produce just the single error).
        if !saw_unmodelled {
            problems.push(Diagnostic::warning(
                format!("'asm' operand {index} probably does not match constraints"),
                Some(index),
            ));
        }
        problems.push(Diagnostic::error(
            "impossible constraint in 'asm'",
            Some(index),
        ));
    }
}

/// Whether an immediate-only alternative of class `letter` can carry this
/// operand.  The rules are measured against gcc 16.2, not guessed:
///
/// | letter | `Value` | `Symbolic` (function / array / string) | `NonConstant` |
/// |--------|---------|----------------------------------------|---------------|
/// | `s`    | REJECT  | accept                                 | REJECT        |
/// | `n`    | accept  | REJECT                                 | REJECT        |
/// | `i`    | accept  | accept                                 | REJECT        |
/// | `I..O` | in range only                                 | REJECT | REJECT |
///
/// `Unknown` is the deferred-instantiation verdict: gcc judges an unbounded
/// immediate when the statement is emitted, so parse time must not refuse it.
fn immediate_satisfiable(letter: char, constant: Constness) -> bool {
    match letter {
        // `s` is a SYMBOL or a string literal, never a folded integer:
        // `"s"(5)` is `impossible constraint in 'asm'`.
        's' => !matches!(constant, Constness::Value(_) | Constness::NonConstant),
        // `n` requires a numeric constant known at assembly time; a symbol or
        // a string is a relocation, not an integer (`"n"(g_arr)`, `"n"("x")`).
        'n' => !matches!(constant, Constness::Symbolic | Constness::NonConstant),
        // The range-measured bit-field classes: a relocation is not a fixed
        // width bit field, and a folded value must fit the measured range.
        l if immediate_range(l).is_some() => match constant {
            Constness::Value(value) => {
                immediate_range(l).is_some_and(|(lo, hi)| (lo..=hi).contains(&value))
            }
            Constness::Unknown => true,
            Constness::NonConstant | Constness::Symbolic => false,
        },
        // `i`, `e`, `E`, `Z`, `G`, `H`, `L`: unbounded immediates that gcc
        // checks at instantiation. Only a provably dynamic operand is
        // impossible at parse time.
        _ => !matches!(constant, Constness::NonConstant),
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
    target: AsmTarget,
) -> Vec<Diagnostic> {
    let mut problems = Vec::new();
    // GCC requires every operand of ONE `asm` to declare the same number of
    // comma-separated alternatives; a single operand may have any number.
    // Measured gcc 16.2 error text: `operand constraints for 'asm' differ in
    // number of alternatives`. Without this, an output `"=r,r"` next to a
    // plain `"r"` input was accepted here and then emitted with the wrong
    // alternative selected by the back end.
    let alt_counts: Vec<usize> = outputs
        .iter()
        .chain(inputs.iter())
        .map(|op| constraint_alternatives(op.constraint).len())
        .collect();
    if alt_counts.len() > 1 && alt_counts.iter().any(|&n| n != alt_counts[0]) {
        problems.push(Diagnostic::error(
            "operand constraints for 'asm' differ in number of alternatives",
            None,
        ));
    }
    // A digit constraint ties an INPUT to an output operand by number. GCC
    // rejects a reference to an output that does not exist with
    // `matching constraint references invalid operand number`; the old
    // digit-start early return accepted `"0"(x)` with no outputs silently.
    let total_operands = outputs.len() + inputs.len();
    for (i, op) in inputs.iter().enumerate() {
        // `%` declares this operand commutative with the FOLLOWING operand;
        // gcc 16.2 refuses it on the last one with
        // "'%' constraint used with last operand". Measured with `"%"`,
        // `"%r"` and `"%0,0"` as the only input; pr35318.c's `%0,0` has a
        // following `"m,r"` operand and stays legal.
        if op.constraint.starts_with('%') && outputs.len() + i + 1 == total_operands {
            problems.push(Diagnostic::error(
                "'%' constraint used with last operand",
                Some(outputs.len() + i),
            ));
            continue;
        }
        let body = op
            .constraint
            .trim_start_matches(|c| MODIFIERS.contains(c) || CONSTRAINT_SKIP.contains(c));
        let choosing = body.split('#').next().unwrap_or(body);
        'alternatives: for alt in choosing.split(',') {
            // A matching digit can appear AFTER a class pair too: gcc 16.2
            // reads `"Yz0"` as `Yz` + a tie to output 0 and rejects it when
            // that output does not exist (red-team differential). Scan EVERY
            // digit run, not just a leading one. The trailing sentinel flushes
            // the last run without a second code path.
            let mut digits = String::new();
            for ch in alt.chars().chain(std::iter::once(' ')) {
                if ch.is_ascii_digit() {
                    digits.push(ch);
                    continue;
                }
                if digits.is_empty() {
                    continue;
                }
                let number = digits.parse::<usize>().unwrap_or(usize::MAX);
                if number >= outputs.len() {
                    problems.push(Diagnostic::error(
                        "matching constraint references invalid operand number",
                        Some(outputs.len() + i),
                    ));
                    break 'alternatives;
                }
                digits.clear();
            }
        }
    }
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
        check_constraint(op, i, true, target, &mut problems);
    }
    for (i, op) in inputs.iter().enumerate() {
        check_constraint(op, outputs.len() + i, false, target, &mut problems);
    }
    // The `%`-code set is measured on x86 only: other back ends map operands
    // through their own substitution pass, so a valid aarch64/RISC-V template
    // must not be second-guessed here.
    if target == AsmTarget::X86 && problems.iter().all(|p| !p.is_error) {
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
            AsmTarget::X86,
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
            AsmTarget::X86,
        );
        assert_eq!(messages(&p), ["input operand constraint contains '+'"]);
    }

    #[test]
    fn plus_modifier_on_output_is_fine() {
        let p = validate("", &[op("+r", Constness::Unknown)], &[], 0, AsmTarget::X86);
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
                AsmTarget::X86,
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
            AsmTarget::X86,
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
                    AsmTarget::X86,
                )
                .is_empty(),
                "{c}({good}) is in range"
            );
            let p = validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op(c, ok(bad))],
                0,
                AsmTarget::X86,
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
                AsmTarget::X86,
            );
            assert_eq!(strict.len(), 2, "\"{c}\" in a plain body must be an error");
            let deferred = validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op(c, Constness::Unknown)],
                0,
                AsmTarget::X86,
            );
            assert!(
                deferred.is_empty(),
                "\"{c}\" in an inline body must be accepted"
            );
        }
    }

    #[test]
    fn symbolic_operands_satisfy_link_time_immediates() {
        // The Linux kernel's `static_branch_*()` shape: a symbolic operand is
        // a link-time constant, so `i` and `Ir` accept it silently.
        //
        // `n` is deliberately NOT in this list any more.  The old version of
        // this test asserted `"n"(symbol)` was accepted, based on a gcc 14.2
        // measurement.  Re-measured on the oracle this module documents
        // (gcc-16 (GCC) 16.2.0, `-O2`, both an array designator and a function
        // designator):
        //
        //   asm volatile("" :: "n"(g));   -> impossible constraint in 'asm'
        //   asm volatile("" :: "n"(fn)); -> impossible constraint in 'asm'
        //   asm volatile("" :: "i"(g));  -> accepted
        //   asm volatile("" :: "i"(fn)); -> accepted
        //
        // `n` is a numeric immediate with a value known at assembly time; a
        // relocation is not such a value.  Asserting the 14.2 behaviour would
        // have kept a silent diagnostic-parity hole in the 16.2 oracle.
        for c in ["i", "Ir"] {
            let p = validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op(c, Constness::Symbolic)],
                0,
                AsmTarget::X86,
            );
            assert!(
                p.is_empty(),
                "\"{c}\" must accept a symbolic operand: {p:?}"
            );
        }
        assert_eq!(
            messages(&validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op("n", Constness::Symbolic)],
                0,
                AsmTarget::X86,
            )),
            [
                "'asm' operand 1 probably does not match constraints",
                "impossible constraint in 'asm'"
            ],
            "\"n\" must refuse a symbolic operand like gcc 16.2"
        );
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
                AsmTarget::X86,
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
                    AsmTarget::X86,
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
            AsmTarget::X86,
        );
        assert!(p.is_empty(), "{p:?}");
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("I", Constness::NonConstant)],
            0,
            AsmTarget::X86,
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
            AsmTarget::X86,
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
                AsmTarget::X86,
            )
            .is_empty()
        );
        let p = validate(
            "mov %2, %0",
            &[op("=r", Constness::Unknown)],
            &[op("r", Constness::Unknown)],
            0,
            AsmTarget::X86,
        );
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        // `%b1` names operand 1 through the `b` modifier.
        let p = validate(
            "%b1",
            &[op("=r", Constness::Unknown)],
            &[],
            0,
            AsmTarget::X86,
        );
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        let p = validate(
            "%b0",
            &[op("=r", Constness::Unknown)],
            &[],
            0,
            AsmTarget::X86,
        );
        assert!(p.is_empty());
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
                AsmTarget::X86,
            )
            .is_empty()
        );
        let p = validate(
            "jmp %l2",
            &[op("+r", Constness::Unknown)],
            &[op("r", Constness::Unknown)],
            1,
            AsmTarget::X86,
        );
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
    }

    #[test]
    fn asm_goto_labels_are_not_operands() {
        // `asm goto("jmp %l0" :::: done)` names label 0, not operand 0.
        let p = validate("jmp %l0", &[], &[], 1, AsmTarget::X86);
        assert!(p.is_empty());
        let p = validate("jmp %l1", &[], &[], 2, AsmTarget::X86);
        assert!(p.is_empty());
        let p = validate(
            "jmp %l0",
            &[op("=r", Constness::Unknown)],
            &[],
            1,
            AsmTarget::X86,
        );
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        let p = validate(
            "jmp %l1",
            &[op("=r", Constness::Unknown)],
            &[],
            1,
            AsmTarget::X86,
        );
        assert!(p.is_empty());
        let p = validate("jmp %l1", &[], &[], 1, AsmTarget::X86);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        // A plain `asm` labels nothing, so the same template is out of range.
        let p = validate("jmp %l0", &[], &[], 0, AsmTarget::X86);
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
        let p = validate("cmpl %1, %2; jmp %3", &outputs, &inputs, 1, AsmTarget::X86);
        assert!(p.is_empty());
        let p = validate("cmpl %1, %2; jmp %l3", &outputs, &inputs, 1, AsmTarget::X86);
        assert!(p.is_empty());
        let p = validate("jmp %4", &outputs, &inputs, 1, AsmTarget::X86);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        // Both spellings skip the read-write slot: `"+r"(v) : "r"(x)` has
        // operands 0/1 and its label is 3, never 2 (measured: gcc prints
        // `plain=$.L3 named=.L3` and answers `%2` with "out of range").
        let outputs = [op("+r", Constness::Unknown)];
        let inputs = [op("r", Constness::Unknown)];
        let p = validate("jmp %3", &outputs, &inputs, 1, AsmTarget::X86);
        assert!(p.is_empty());
        let p = validate("jmp %2", &outputs, &inputs, 1, AsmTarget::X86);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
    }

    fn label_indices_are_multi_digit_like_gcc() {
        // Measured: with 13 inputs, `gcc -S` accepts `%l13` (it emits the
        // label's own name) and rejects `%l12`. Reading a single digit would
        // accept `%l12` (as label 2) and reject `%l13` (as label 1), i.e. it
        // would invert both answers and false-reject valid `asm goto` code that
        // names a label with a two-digit index.
        let inputs: Vec<Operand<'_>> = (0..13).map(|_| op("r", Constness::Unknown)).collect();
        let p = validate("jmp %l13", &[], &inputs, 1, AsmTarget::X86);
        assert!(p.is_empty());
        let p = validate("jmp %l12", &[], &inputs, 1, AsmTarget::X86);
        assert_eq!(messages(&p), ["invalid 'asm': operand number out of range"]);
        let p = validate("jmp %l14", &[], &inputs, 1, AsmTarget::X86);
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
            AsmTarget::X86,
        );
        assert!(p.is_empty());
        let p = validate(
            "mov %2, %0",
            &[op("=r", Constness::Unknown)],
            &[op("r", Constness::Unknown)],
            0,
            AsmTarget::X86,
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
            let p = validate(
                "",
                &[op(&constraint, Constness::Unknown)],
                &[],
                0,
                AsmTarget::X86,
            );
            assert!(p.is_empty(), "{constraint} must be accepted");
        }
        // gcc: `unknown 'asm' flag output '<constraint>'` for all four.
        for bad in ["=@cc", "=@ccxx", "=@ccpe", "=@ccpo"] {
            let p = validate("", &[op(bad, Constness::Unknown)], &[], 0, AsmTarget::X86);
            let want = format!("unknown 'asm' flag output '{bad}'");
            assert_eq!(messages(&p), [want.as_str()], "{bad}");
        }
        // gcc: `invalid punctuation '@' in constraint` on an input operand.
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("@ccz", Constness::Unknown)],
            0,
            AsmTarget::X86,
        );
        assert_eq!(messages(&p), ["invalid punctuation '@' in constraint"]);
    }

    /// `validate` for a lone output operand with no labels, bound to one
    /// target. The per-target flag-output table below is a set of one-line
    /// cases, and spelling out five arguments per case would bury it.
    fn check(target: AsmTarget, constraint: &str) -> Vec<Diagnostic> {
        validate("", &[op(constraint, Constness::Unknown)], &[], 0, target)
    }

    /// The accepted `=@cc` suffix set is a property of the target, not of the
    /// constraint grammar: x86 spells "zero clear" as `z`, AArch64 as `eq`, and
    /// neither spelling may be reinterpreted on the other target. Before
    /// `AsmTarget` existed the validator skipped the check entirely off x86, so
    /// `=@ccz` on AArch64 -- or on RISC-V, which has no flag output at all --
    /// reached a back end that stored an uninitialised register as the result.
    #[test]
    fn flag_output_suffix_set_is_per_target() {
        // Every ARM `ConditionHolds` condition GCC accepts in this position.
        for cond in [
            "eq", "ne", "cs", "hs", "cc", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt",
            "gt", "le",
        ] {
            let c = format!("=@cc{cond}");
            let p = check(AsmTarget::Aarch64, &c);
            assert!(p.is_empty(), "{c} must be accepted on aarch64");
        }
        // A spelling from the other target's table is not a condition here.
        for (c, target) in [
            ("=@ccz", AsmTarget::Aarch64),
            ("=@ccnz", AsmTarget::Aarch64),
            ("=@cceq", AsmTarget::X86),
            ("=@cchs", AsmTarget::X86),
        ] {
            let p = check(target, c);
            let want = format!("unknown 'asm' flag output '{c}'");
            assert_eq!(messages(&p), [want.as_str()], "{c}");
        }
        // `al`/`nv` are ARM `ConditionHolds` conditions but not flag tests, and
        // GCC rejects both; a `cset` for either would be meaningless.
        for c in ["=@ccal", "=@ccnv"] {
            let p = check(AsmTarget::Aarch64, c);
            let want = format!("unknown 'asm' flag output '{c}'");
            assert_eq!(messages(&p), [want.as_str()], "{c}");
        }
        // RISC-V: the operand is refused for being a flag output at all, so the
        // message names the class rather than the suffix.
        for c in ["=@ccz", "=@cceq", "=@cc"] {
            let p = check(AsmTarget::Other, c);
            let want = format!("'{c}' flag output constraint is not supported on this target");
            assert_eq!(messages(&p), [want.as_str()], "{c}");
        }
        // `@` on an input is refused on every target, including the two with
        // flag outputs.
        let outputs = [op("=r", Constness::Unknown)];
        let inputs = [op("@cceq", Constness::Unknown)];
        for target in [AsmTarget::X86, AsmTarget::Aarch64, AsmTarget::Other] {
            let p = validate("", &outputs, &inputs, 0, target);
            assert_eq!(messages(&p), ["invalid punctuation '@' in constraint"]);
        }
    }

    /// `AsmTarget::from_elf_machine` is the only place the mapping from an ELF
    /// machine number to an `asm` grammar is written down, so pin it.
    #[test]
    fn asm_target_mapping_is_elf_machine_driven() {
        // EM_386, EM_X86_64, EM_AARCH64.
        assert_eq!(AsmTarget::from_elf_machine(3), AsmTarget::X86);
        assert_eq!(AsmTarget::from_elf_machine(62), AsmTarget::X86);
        assert_eq!(AsmTarget::from_elf_machine(183), AsmTarget::Aarch64);
        // EM_ARM, EM_RISCV, EM_NONE, EM_PPC, EM_PPC64.
        for m in [40u16, 243, 0, 20, 21] {
            assert_eq!(AsmTarget::from_elf_machine(m), AsmTarget::Other, "{m}");
        }
        assert!(!AsmTarget::Other.supports_flag_outputs());
        assert!(AsmTarget::X86.supports_flag_outputs());
        assert!(AsmTarget::Aarch64.supports_flag_outputs());
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
                AsmTarget::X86,
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
                validate(t, &[op("=r", Constness::Unknown)], &[], 0, AsmTarget::X86).is_empty(),
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
            AsmTarget::X86,
        );
        assert_eq!(
            messages(&p),
            ["invalid 'asm': operand is not a condition code, invalid operand code 'c'"]
        );
    }

    #[test]
    fn condition_code_modifier_follows_one_named_tie() {
        // `%c` prints a constant expression.  A named tie (`"[sum]"`) carries
        // the *output's* constraint, so the verdict has to look through it:
        // measured on gcc 16.2, `: [s] "=r"(s) : "[s]"(b)` with `%c1` is
        // `invalid 'asm': operand is not a condition code, invalid operand
        // code 'c'`, exactly like the untied register spelling, while a tie to
        // an immediate output is not that error (gcc only warns that a
        // matching constraint cannot take a register, which is about the tie
        // itself, not about `%c`).
        let outputs = [named("=r", Constness::Unknown, "s")];
        let inputs = [op("[s]", Constness::Unknown)];
        let p = validate("mov %c1, %0", &outputs, &inputs, 0, AsmTarget::X86);
        assert_eq!(
            messages(&p),
            ["invalid 'asm': operand is not a condition code, invalid operand code 'c'"]
        );
        let outputs = [named("=n", Constness::Unknown, "s")];
        let inputs = [op("[s]", Constness::Unknown)];
        let p = validate("mov %c1, %0", &outputs, &inputs, 0, AsmTarget::X86);
        assert!(
            p.iter()
                .all(|d| !d.message.contains("not a condition code")),
            "{p:?}"
        );
    }

    #[test]
    fn modelled_constraints_pass() {
        for c in [
            "r", "m", "g", "rm", "=r", "+r", "&r", "Yz", "x", "v", "Q", "A", "X", "t", "u",
            "{rax}", "@ccz", "0", "i", "f",
        ] {
            let p = validate("", &[op(c, Constness::Unknown)], &[], 0, AsmTarget::X86);
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
                AsmTarget::Other,
            )
            .is_empty()
        );
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("=r", Constness::Unknown)],
            0,
            AsmTarget::Other,
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
            AsmTarget::X86,
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
            AsmTarget::X86,
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
            AsmTarget::X86,
        );
        assert_eq!(
            messages(&p),
            [
                "'asm' operand 0 probably does not match constraints",
                "impossible constraint in 'asm'"
            ]
        );
    }
    // ---------------------------------------------------------------------
    // GCC 16.2 multi-alternative / modifier grammar (gcc.c-torture/compile
    // 20000922-1.c, asmgoto-2.c, pr27528.c, pr35318.c).
    //
    // Every expectation below was measured with
    // `gcc-16 (GCC) 16.2.0 -O2 -c` on x86-64; Clang 23.1.4 agrees on the
    // standard spellings.  The four torture files are the acceptance gate.
    // ---------------------------------------------------------------------

    #[test]
    fn comma_separated_alternatives_accept_any_satisfiable_one() {
        // A single operand may declare any number of alternatives; the
        // alternative-count consistency rule only applies between operands.
        for c in [
            "r,m", "m,r", "r,m,r", "ri,m", "i,r", "n,i", "s,r", "rm,r", "=r,m", "=m,r", "=r,r",
            "=r,r,r", "+r,m",
        ] {
            let p = validate("", &[op(c, Constness::Unknown)], &[], 0, AsmTarget::X86);
            assert!(p.is_empty(), "\"{c}\": {p:?}");
        }
    }

    #[test]
    fn only_an_all_impossible_alternative_set_is_refused() {
        // `I` (0..31) and `J` (0..63) both reject 100; adding a register
        // alternative makes the operand satisfiable exactly as gcc does.
        let p = validate(
            "",
            &[op("I,J", Constness::Value(100))],
            &[],
            0,
            AsmTarget::X86,
        );
        assert_eq!(
            messages(&p),
            [
                "'asm' operand 0 probably does not match constraints",
                "impossible constraint in 'asm'"
            ]
        );
        assert!(
            validate(
                "",
                &[op("I,r", Constness::Value(100))],
                &[],
                0,
                AsmTarget::X86,
            )
            .is_empty()
        );
    }

    #[test]
    fn gcc_cost_and_selection_modifiers_are_not_constraint_letters() {
        for c in [
            "r?", "!r", "^r", "$r", "r?m", "!r,m", "i#*X", "i#*r", "n#*X", "i#zzz",
        ] {
            let p = validate("", &[op(c, Constness::Unknown)], &[], 0, AsmTarget::X86);
            assert!(p.is_empty(), "\"{c}\": {p:?}");
        }
        // gcc still rejects stray punctuation in a `#` comment tail.
        for c in ["i#@@@", "i#*@"] {
            assert_eq!(
                messages(&validate(
                    "",
                    &[op(c, Constness::Unknown)],
                    &[],
                    0,
                    AsmTarget::X86,
                )),
                ["invalid punctuation '@' in constraint"],
                "\"{c}\""
            );
        }
    }

    #[test]
    fn symbolic_immediate_class_parity_with_gcc_16_2() {
        // `"s"(5)` and `"s"(x)` (a runtime parameter) are impossible;
        // `"s"(g_arr)`, `"s"(fn)` and `"s"("string")` are accepted.
        for c in [Constness::Value(5), Constness::NonConstant] {
            assert_eq!(
                messages(&validate("", &[op("s", c)], &[], 0, AsmTarget::X86,)),
                [
                    "'asm' operand 0 probably does not match constraints",
                    "impossible constraint in 'asm'"
                ],
                "{c:?}"
            );
        }
        for c in [Constness::Symbolic, Constness::Unknown] {
            assert!(
                validate("", &[op("s", c)], &[], 0, AsmTarget::X86).is_empty(),
                "{c:?}"
            );
        }
        // `"n"` is the mirror image: a symbol/string is refused, an integer
        // constant or a deferred operand is accepted.
        assert_eq!(
            messages(&validate(
                "",
                &[op("n", Constness::Symbolic)],
                &[],
                0,
                AsmTarget::X86,
            )),
            [
                "'asm' operand 0 probably does not match constraints",
                "impossible constraint in 'asm'"
            ]
        );
        assert!(
            validate(
                "",
                &[op("n", Constness::Value(400))],
                &[],
                0,
                AsmTarget::X86,
            )
            .is_empty()
        );
    }

    #[test]
    fn string_literals_are_link_time_symbolic_constants() {
        let span = crate::common::source::Span::dummy();
        let expr = Expr::StringLiteral("hello".into(), span);
        let ctx = SymbolContext {
            typedefs: &FxHashSet::default(),
            enums: &FxHashMap::default(),
            functions: &FxHashSet::default(),
        };
        assert_eq!(constness(&expr, &ctx, false), Constness::Symbolic);
        // `"i"("hi")` and `"s"("hi")` are accepted, `"n"("hi")` is not.
        for c in ["i", "s"] {
            assert!(
                validate("", &[op(c, Constness::Symbolic)], &[], 0, AsmTarget::X86,).is_empty()
            );
        }
    }

    #[test]
    fn operands_must_agree_on_alternative_count() {
        // gcc 16.2: `operand constraints for 'asm' differ in number of
        // alternatives`.
        let p = validate(
            "",
            &[op("=r", Constness::Unknown)],
            &[op("r,m", Constness::Unknown)],
            0,
            AsmTarget::X86,
        );
        assert_eq!(
            messages(&p),
            ["operand constraints for 'asm' differ in number of alternatives"]
        );
        // Consistent two-alternative forms (pr35318.c) validate clean.
        let p = validate(
            "",
            &[
                op("=r,r", Constness::Unknown),
                op("=r,r", Constness::Unknown),
            ],
            &[
                op("%0,0", Constness::Unknown),
                op("m,r", Constness::Unknown),
            ],
            0,
            AsmTarget::X86,
        );
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn redteam_impossible_alternatives_are_not_validity_errors() {
        // Measured gcc 16.2 (red-team differential over 23 spellings):
        // an unmodelled letter or a bare `Y`/`C` prefix makes ONE ALTERNATIVE
        // impossible, not the whole constraint invalid. `"z,r"` / `"r,z"`
        // compile; `"z"`, `"C"`, `"Y"`, `"z,i"` refuse with the SINGLE
        // "impossible constraint" diagnostic (no "probably does not match"
        // warning when an unmodelled alternative is present).
        for c in [
            "z,r", "r,z", "r,", "r,?", "Yz", "Cc", "", "rz", "zr", "zri", "rzi", "Yzz", "Yzr",
            "zz,r", "r,zr", "rz,r",
        ] {
            let p = validate("", &[op(c, Constness::Unknown)], &[], 0, AsmTarget::X86);
            assert!(p.is_empty(), "\"{c}\": {p:?}");
        }
        for c in ["z", "C", "Y"] {
            let p = validate("", &[op(c, Constness::Unknown)], &[], 0, AsmTarget::X86);
            assert_eq!(messages(&p), ["impossible constraint in 'asm'"], "\"{c}\"");
        }
        // With a DYNAMIC operand the immediate alternative is impossible too,
        // so the whole set fails. gcc emits just the single error because an
        // unmodelled alternative is present (no "probably does not match").
        for c in ["z,i", "i,z", "I,z"] {
            let p = validate("", &[op(c, Constness::NonConstant)], &[], 0, AsmTarget::X86);
            assert_eq!(messages(&p), ["impossible constraint in 'asm'"], "\"{c}\"");
        }
        // A constraint whose only content is a `#` comment has no alternative
        // at all: gcc's distinct diagnostic.
        assert_eq!(
            messages(&validate(
                "",
                &[op("#foo", Constness::Unknown)],
                &[],
                0,
                AsmTarget::X86,
            )),
            ["inconsistent operand constraints in an 'asm'"]
        );
        // A lone cost modifier with no class/immediate alternative: gcc emits
        // the warning AND the error.
        assert_eq!(
            messages(&validate(
                "",
                &[op("?", Constness::Unknown)],
                &[],
                0,
                AsmTarget::X86,
            )),
            [
                "'asm' operand 0 probably does not match constraints",
                "impossible constraint in 'asm'"
            ]
        );
    }

    #[test]
    fn redteam_commutative_modifier_needs_a_following_operand() {
        for c in ["%", "%r", "%0,0"] {
            assert_eq!(
                messages(&validate(
                    "",
                    &[],
                    &[op(c, Constness::Unknown)],
                    0,
                    AsmTarget::X86,
                )),
                ["'%' constraint used with last operand"],
                "\"{c}\""
            );
        }
        // With a following operand the `%` tie is legal (pr35318.c shape).
        let p = validate(
            "",
            &[op("=r,r", Constness::Unknown)],
            &[
                op("%0,0", Constness::Unknown),
                op("m,r", Constness::Unknown),
            ],
            0,
            AsmTarget::X86,
        );
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn matching_digit_constraints_must_reference_an_existing_output() {
        assert_eq!(
            messages(&validate(
                "",
                &[],
                &[op("0", Constness::Unknown)],
                0,
                AsmTarget::X86,
            )),
            ["matching constraint references invalid operand number"]
        );
        assert!(
            validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op("0", Constness::Unknown)],
                0,
                AsmTarget::X86,
            )
            .is_empty()
        );
        assert_eq!(
            messages(&validate(
                "",
                &[op("=r", Constness::Unknown)],
                &[op("1", Constness::Unknown)],
                0,
                AsmTarget::X86,
            )),
            ["matching constraint references invalid operand number"]
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
            let p = validate("", &outputs, &[input(constraint)], 0, AsmTarget::X86);
            assert!(p.is_empty(), "{constraint}: {p:?}");
        }
        let p = validate("", &outputs, &[input("[zz]r")], 0, AsmTarget::X86);
        assert_eq!(messages(&p), ["undefined named operand 'zz'"]);
        let p = validate("", &outputs, &[input("[sum][zz]")], 0, AsmTarget::X86);
        assert_eq!(messages(&p), ["undefined named operand 'zz'"]);
    }

    #[test]
    fn malformed_named_ties_are_diagnosed_not_classified_as_letters() {
        let outputs = [named("=r", "sum")];
        let p = validate("", &outputs, &[input("[sum")], 0, AsmTarget::X86);
        assert_eq!(messages(&p), ["missing close brace for named operand"]);
        let p = validate("", &outputs, &[input("[sum]@")], 0, AsmTarget::X86);
        assert_eq!(messages(&p), ["invalid punctuation '@' in constraint"]);
        let p = validate("", &outputs, &[input("[]")], 0, AsmTarget::X86);
        assert_eq!(messages(&p), ["undefined named operand ''"]);
    }

    #[test]
    fn early_clobber_modifier_on_input_is_an_error() {
        let outputs = [named("=r", "sum")];
        let p = validate("", &outputs, &[input("&r")], 0, AsmTarget::X86);
        assert_eq!(messages(&p), ["input operand constraint contains '&'"]);
        let p = validate("", &outputs, &[input("&[sum]")], 0, AsmTarget::X86);
        assert_eq!(messages(&p), ["input operand constraint contains '&'"]);
    }

    #[test]
    fn named_matching_constraint_is_not_an_output_constraint() {
        let p = validate("", &[input("[sum]")], &[], 0, AsmTarget::X86);
        assert_eq!(messages(&p), ["output operand constraint lacks '='"]);
    }
}
