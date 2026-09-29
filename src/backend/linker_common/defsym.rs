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

/// The symbols every lccc backend provides from its own layout, mirroring
/// the name set of `backend::elf::get_standard_linker_symbols`.
///
/// A `--defsym` right-hand side that names one of these is legal in GNU ld:
/// the magic symbol is resolved during expression evaluation, after
/// addresses are final. Backends therefore classify such names as defined
/// (so they are not rejected as typos) and defer them to post-layout
/// evaluation instead of aliasing them at apply time.
pub const LINKER_DEFINED_NAMES: &[&str] = &[
    "_GLOBAL_OFFSET_TABLE_",
    "_DYNAMIC",
    "__bss_start",
    // NOTE: no `__bss_end__` — GNU ld defines no such symbol on ELF targets
    // (see linker_symbols.rs); listing it here would accept `--defsym`
    // expressions GNU ld rejects as undefined symbols.
    "_edata",
    "edata",
    "_end",
    "__end",
    "end",
    "_etext",
    "etext",
    "__ehdr_start",
    "__executable_start",
    "__dso_handle",
    "__data_start",
    "data_start",
    "__init_array_start",
    "__init_array_end",
    "__fini_array_start",
    "__fini_array_end",
    "__preinit_array_start",
    "__preinit_array_end",
    "__rela_iplt_start",
    "__rela_iplt_end",
];

/// True if `name` is a layout-derived symbol the linker itself defines.
pub fn is_linker_defined(name: &str) -> bool {
    LINKER_DEFINED_NAMES.contains(&name)
}

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
    /// The expression reads the named symbol, which has no value yet at this
    /// point of the link (a placed symbol before layout).  A caller that
    /// evaluates in phases uses it as "not yet": the `-T` path defers such a
    /// statement until layout has placed the name.
    NeedsLayout(String),
}

impl DefsymError {
    /// The text GNU ld 2.47 prints for this failure, including its `--defsym:N`
    /// counter (binutils `ld/ldexp.c`, `ld/ldlex.l:lex_redirect` — and
    /// measured byte-for-byte against `/usr/bin/ld`):
    ///
    /// ```text
    /// --defsym:N: undefined symbol `X' referenced in expression
    /// --defsym:N / by zero
    /// --defsym:0: syntax error
    /// ```
    ///
    /// Note the `` `X' `` quoting (backquote + quote, not two straight
    /// quotes) and the missing colon in the divide-by-zero form — both are
    /// GNU quirks reproduced exactly.  (`ld: ` itself is the caller's prefix
    /// to add.)
    ///
    /// `N` is the 1-based position of the failing `--defsym` in the link's
    /// defsym list. The counter is a GNU quirk worth reproducing exactly:
    /// syntax errors always report 0 (the counter counts accepted symbols,
    /// and a syntax error accepts none), while undefined-symbol and
    /// division-by-zero failures report the defsym's own index.
    ///
    /// [`NeedsLayout`] has no GNU counterpart (GNU ld evaluates after
    /// layout); it names the symbol that had no value.
    pub fn gnu_message(&self, index: usize) -> String {
        match self {
            DefsymError::Syntax(_) => "--defsym:0: syntax error".to_string(),
            DefsymError::DivByZero => format!("--defsym:{index} / by zero"),
            DefsymError::UndefinedSymbol(name) => {
                format!("--defsym:{index}: undefined symbol `{name}' referenced in expression")
            }
            DefsymError::NeedsLayout(name) => {
                format!(
                    "--defsym:{index}: symbol `{name}' has no address yet at this point of the link"
                )
            }
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

/// Whether a (syntactically valid) expression denotes an ADDRESS in the
/// output rather than an absolute value, with `is_address` saying which
/// symbol names are addresses.
///
/// This is GNU ld's section-relative vs absolute distinction (ld/ldexp.c),
/// and it decides what the defined symbol is in a position-independent
/// output: an address slides with the load base (a PIE/`.so` needs
/// `R_X86_64_RELATIVE` for a pointer to it and exports it against its
/// section) while an absolute value does not (no RELATIVE, `SHN_ABS`).
/// Getting it wrong is a silent miscompile either way: `--defsym x=_start+4`
/// read through a GOT slot without RELATIVE points into unmapped memory, and
/// `--defsym size=_end-_start` with RELATIVE comes out off by the load bias.
///
/// The rule, which matches ld's for every expression it gives a meaning to:
/// count the address operands, adding across `+`, subtracting across `-`
/// (so `sym + 4` has one, `_end - _start` none), and let every other
/// operator (`*`, `/`, `%`, unary `-`, `~`) turn its operands into plain
/// numbers, as ld's `make_abs` does.  Exactly one address means the result
/// is an address.  (ld makes the sum of two addresses in different sections
/// absolute too; the sum of two in one section is meaningless in a PIE
/// either way.)  The answer depends only on WHICH names are addresses, not
/// on their values, so it can be decided before layout.
pub fn is_address_expression(
    expr: &str,
    is_address: impl Fn(&str) -> bool,
) -> Result<bool, DefsymError> {
    let mut ctx = Ctx {
        s: expr.as_bytes(),
        i: 0,
    };
    let degree = ctx.degree_expr(&is_address)?;
    ctx.skip_ws();
    if ctx.i != ctx.s.len() {
        return Err(DefsymError::Syntax(format!(
            "trailing '{}' at offset {}",
            ctx.s[ctx.i] as char, ctx.i
        )));
    }
    Ok(degree == 1)
}

/// Every distinct symbol name an expression mentions, in first-use order.
///
/// Walks the degree grammar (which never divides, so `1/(a-a)` still yields
/// `a`), i.e. exactly the tokens the evaluator will look up.
fn referenced_names(expr: &str) -> Result<Vec<String>, DefsymError> {
    let names = std::cell::RefCell::new(Vec::<String>::new());
    is_address_expression(expr, |n| {
        let mut v = names.borrow_mut();
        if !v.iter().any(|x| x == n) {
            v.push(n.to_string());
        }
        false
    })?;
    Ok(names.into_inner())
}

/// What the link, apart from `--defsym`, knows about a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkSym {
    /// Not defined in this link (a shared-library symbol counts: GNU ld
    /// reports it as undefined in an expression).
    Undefined,
    /// Defined by an input object at an address in the output.
    Address,
    /// Defined with an absolute value (`SHN_ABS`).
    Absolute,
}

/// Where one symbol reference inside a `--defsym` right-hand side binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    /// The value the `--defsym` statement with this position (0-based) gives.
    Defsym(usize),
    /// The definition the link has independently of `--defsym`: an input
    /// object's symbol or one of the layout symbols (`_end`, …).
    Link(String),
}

/// A `--defsym` right-hand side with its references bound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rhs {
    Constant(u64),
    Alias(Binding),
    /// The expression text and the binding of every name it mentions.
    Expression(String, Vec<(String, Binding)>),
}

/// One `--defsym NAME=EXPR`, planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub name: String,
    /// 1-based position: the `N` of GNU ld's `--defsym:N` diagnostics.
    pub index: usize,
    pub rhs: Rhs,
    /// Whether the value is an address in the output (slides with a PIE /
    /// `.so` load base) rather than an absolute value; see
    /// [`is_address_expression`].
    pub is_address: bool,
}

/// All `--defsym` options of a link, bound and ordered once so that every
/// backend -- and both phases of one link (classification before layout,
/// evaluation after it) -- agrees on what each right-hand side means.
///
/// Binding follows GNU ld, which runs `--defsym`s as linker-script
/// assignments, in order, over several passes (measured with ld 2.47):
///
/// 1. A name defined by an EARLIER `--defsym` means that definition's value
///    at that point: `a=1 b=a a=2` gives `b = 1`, `a = 2`.
/// 2. A statement's reference to its own target, with no earlier
///    `--defsym` of it, means the input's definition: `a=a+1` over an
///    object's `a` is that address plus one, and an error without one.
/// 3. A name defined only by LATER `--defsym`s means the last of them
///    (ld's final pass sees the values of the previous one): `a=b
///    b=_start+4` gives `a = b = _start+4`, in either order.
/// 4. Anything else is the link's own definition, or an error.
///
/// One deliberate divergence: a cycle through rule 3 (`a=b b=a`) is an
/// error naming the cycle.  ld silently defines both as 0 -- a value no one
/// wrote, which then flows into relocations.
#[derive(Clone, Debug, Default)]
pub struct DefsymPlan {
    pub statements: Vec<Statement>,
    /// Statement positions, dependencies before dependents.
    order: Vec<usize>,
}

impl DefsymPlan {
    /// Bind and classify `defs` (`(name, expression)` in command-line
    /// order).  `link` describes names defined outside `--defsym`; it is
    /// asked before any `--defsym` is applied.  Errors carry GNU ld's text.
    pub fn new(defs: &[(String, String)], link: impl Fn(&str) -> LinkSym) -> Result<Self, String> {
        let mut defined_at: std::collections::HashMap<&str, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, (name, _)) in defs.iter().enumerate() {
            defined_at.entry(name.as_str()).or_default().push(i);
        }
        let known = |n: &str| link(n) != LinkSym::Undefined || is_linker_defined(n);
        let bind = |n: &str, at: usize| -> Result<Binding, String> {
            let err = || DefsymError::UndefinedSymbol(n.to_string()).gnu_message(at + 1);
            if let Some(ds) = defined_at.get(n) {
                if let Some(&j) = ds.iter().rev().find(|&&j| j < at) {
                    return Ok(Binding::Defsym(j));
                }
                if n != defs[at].0 {
                    return Ok(Binding::Defsym(*ds.last().unwrap_or(&at)));
                }
            }
            if known(n) {
                Ok(Binding::Link(n.to_string()))
            } else {
                Err(err())
            }
        };
        let mut statements = Vec::with_capacity(defs.len());
        for (i, (name, expr)) in defs.iter().enumerate() {
            // Syntax only: which names exist is `bind`'s question.
            let rhs = match classify(expr, |_| true).map_err(|e| e.gnu_message(i + 1))? {
                Defsym::Constant(v) => Rhs::Constant(v),
                Defsym::Alias(t) => Rhs::Alias(bind(&t, i)?),
                Defsym::Expression(e) => {
                    let names = referenced_names(&e).map_err(|x| x.gnu_message(i + 1))?;
                    let refs = names
                        .into_iter()
                        .map(|n| bind(&n, i).map(|b| (n, b)))
                        .collect::<Result<Vec<_>, _>>()?;
                    Rhs::Expression(e, refs)
                }
            };
            statements.push(Statement {
                name: name.clone(),
                index: i + 1,
                rhs,
                is_address: false,
            });
        }
        let order = Self::topological_order(&statements)?;
        // Classify in dependency order: a reference to another `--defsym`
        // is an address exactly when that definition is one.
        for &i in &order {
            let is_addr_of = |b: &Binding, st: &[Statement]| match b {
                Binding::Defsym(j) => st[*j].is_address,
                Binding::Link(n) => link(n) == LinkSym::Address || is_linker_defined(n),
            };
            let is_address = match &statements[i].rhs {
                Rhs::Constant(_) => false,
                Rhs::Alias(b) => is_addr_of(b, &statements),
                Rhs::Expression(e, refs) => is_address_expression(e, |n| {
                    refs.iter()
                        .find(|(r, _)| r == n)
                        .is_some_and(|(_, b)| is_addr_of(b, &statements))
                })
                .map_err(|x| x.gnu_message(i + 1))?,
            };
            statements[i].is_address = is_address;
        }
        Ok(DefsymPlan { statements, order })
    }

    /// Dependencies before dependents; a cycle is an error naming it.
    fn topological_order(st: &[Statement]) -> Result<Vec<usize>, String> {
        // 0 = unvisited, 1 = on the DFS stack, 2 = done.  Iterative, so a
        // long alias chain from a generated command line cannot overflow
        // the stack.
        let mut state = vec![0u8; st.len()];
        let mut order = Vec::with_capacity(st.len());
        for root in 0..st.len() {
            if state[root] != 0 {
                continue;
            }
            let mut stack: Vec<(usize, Vec<usize>)> = vec![(root, st[root].defsym_deps())];
            state[root] = 1;
            while let Some((node, pending)) = stack.last_mut() {
                let node = *node;
                match pending.pop() {
                    Some(d) if state[d] == 1 => {
                        let from = stack.iter().position(|(n, _)| *n == d).unwrap_or(0);
                        let mut path: Vec<&str> = stack[from..]
                            .iter()
                            .map(|(n, _)| st[*n].name.as_str())
                            .collect();
                        path.push(st[d].name.as_str());
                        return Err(format!(
                            "--defsym:{}: circular reference: {}",
                            st[d].index,
                            path.join(" -> ")
                        ));
                    }
                    Some(d) if state[d] == 0 => {
                        state[d] = 1;
                        stack.push((d, st[d].defsym_deps()));
                    }
                    Some(_) => {}
                    None => {
                        state[node] = 2;
                        order.push(node);
                        stack.pop();
                    }
                }
            }
        }
        Ok(order)
    }

    /// The statement that finally defines each name (the last one naming
    /// it), in command-line order.
    pub fn finals(&self) -> Vec<&Statement> {
        let mut seen = std::collections::HashSet::new();
        let mut v: Vec<&Statement> = self
            .statements
            .iter()
            .rev()
            .filter(|s| seen.insert(s.name.as_str()))
            .collect();
        v.reverse();
        v
    }

    /// Follow alias-to-`--defsym` links from `st` to the statement whose
    /// right-hand side is not such an alias.
    pub fn root<'a>(&'a self, mut st: &'a Statement) -> &'a Statement {
        while let Rhs::Alias(Binding::Defsym(j)) = &st.rhs {
            st = &self.statements[*j];
        }
        st
    }

    /// The input symbol `st` is ultimately an alias of, if any: such a
    /// definition is a copy of that symbol (section, PLT/GOT and all)
    /// rather than a computed value.
    pub fn aliased_link_symbol<'a>(&'a self, st: &'a Statement) -> Option<&'a str> {
        match &self.root(st).rhs {
            Rhs::Alias(Binding::Link(n)) => Some(n.as_str()),
            _ => None,
        }
    }

    /// Names a statement binds to the link's own definition of ITS OWN
    /// target (rule 2): that definition is overridden by the `--defsym`
    /// itself, so the backend must keep a copy of it to evaluate against.
    pub fn shadowed_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for s in &self.statements {
            let mentions_self = match &s.rhs {
                Rhs::Alias(Binding::Link(n)) => n == &s.name,
                Rhs::Expression(_, refs) => refs
                    .iter()
                    .any(|(n, b)| n == &s.name && *b == Binding::Link(n.clone())),
                _ => false,
            };
            if mentions_self && !v.contains(&s.name.as_str()) {
                v.push(s.name.as_str());
            }
        }
        v
    }

    /// The values of the statements at positions `needed` and of everything
    /// they depend on, indexed by position (`None`: not computed).
    ///
    /// `link` gives the final value of a name's non-`--defsym` definition.
    /// Only what is needed is computed: an alias that became a copy of an
    /// input symbol has no value to compute unless another definition reads
    /// it, and a link path without final addresses (a `-T` link before
    /// layout) must not fail on a value nobody uses.  An error names the
    /// failing statement's position.
    pub fn evaluate_needed(
        &self,
        needed: &[usize],
        link: impl Fn(&str) -> Result<u64, DefsymError>,
    ) -> Result<Vec<Option<u64>>, (usize, DefsymError)> {
        let n = self.statements.len();
        let mut want = vec![false; n];
        let mut work: Vec<usize> = needed.to_vec();
        while let Some(i) = work.pop() {
            if !std::mem::replace(&mut want[i], true) {
                work.extend(self.statements[i].defsym_deps());
            }
        }
        let mut values: Vec<Option<u64>> = vec![None; n];
        for &i in self.order.iter().filter(|&&i| want[i]) {
            let st = &self.statements[i];
            let value_of = |b: &Binding, name: &str| -> Result<u64, DefsymError> {
                match b {
                    // Dependencies come first in `order` and are wanted.
                    Binding::Defsym(j) => values[*j].ok_or(DefsymError::UndefinedSymbol(
                        self.statements[*j].name.clone(),
                    )),
                    Binding::Link(l) => link(l).map_err(|e| match e {
                        DefsymError::UndefinedSymbol(_) => {
                            DefsymError::UndefinedSymbol(name.to_string())
                        }
                        other => other,
                    }),
                }
            };
            let v = match &st.rhs {
                Rhs::Constant(v) => Ok(*v),
                Rhs::Alias(b) => value_of(b, b_name(b, &st.name)),
                Rhs::Expression(e, refs) => {
                    let failed = std::cell::RefCell::new(None);
                    let r = eval_with_symbols(e, &|name| {
                        let (_, b) = refs.iter().find(|(r, _)| r == name)?;
                        value_of(b, name)
                            .map_err(|x| *failed.borrow_mut() = Some(x))
                            .ok()
                    });
                    match failed.into_inner() {
                        Some(x) => Err(x),
                        None => r,
                    }
                }
            }
            .map_err(|x| (i, x))?;
            values[i] = Some(v);
        }
        Ok(values)
    }
}

impl DefsymPlan {
    /// [`Self::evaluate_needed`] for a link with final addresses: `link`
    /// returns `None` for a name it does not define, and errors come back
    /// as GNU ld's text.
    pub fn evaluate(
        &self,
        needed: &[usize],
        link: impl Fn(&str) -> Option<u64>,
    ) -> Result<Vec<Option<u64>>, String> {
        self.evaluate_needed(needed, |n| {
            link(n).ok_or_else(|| DefsymError::UndefinedSymbol(n.to_string()))
        })
        .map_err(|(i, e)| e.gnu_message(self.statements[i].index))
    }
}

impl Statement {
    /// Positions of the `--defsym` statements this one reads.
    fn defsym_deps(&self) -> Vec<usize> {
        match &self.rhs {
            Rhs::Alias(Binding::Defsym(j)) => vec![*j],
            Rhs::Expression(_, refs) => refs
                .iter()
                .filter_map(|(_, b)| match b {
                    Binding::Defsym(j) => Some(*j),
                    Binding::Link(_) => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

fn b_name<'a>(b: &'a Binding, fallback: &'a str) -> &'a str {
    match b {
        Binding::Link(n) => n.as_str(),
        Binding::Defsym(_) => fallback,
    }
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

    /// The address degree of an expression (see [`is_address_expression`]):
    /// the same grammar as `eval_expr`, counting address operands instead of
    /// computing values.  Every atom is parsed by `eval_atom` itself, so the
    /// two walks cannot disagree about where a token ends.
    fn degree_expr(&mut self, is_address: &dyn Fn(&str) -> bool) -> Result<i64, DefsymError> {
        let mut lhs = self.degree_term(is_address)?;
        while let Some(op) = self.peek() {
            if op != b'+' && op != b'-' {
                break;
            }
            self.i += 1;
            let rhs = self.degree_term(is_address)?;
            lhs = if op == b'+' {
                lhs.saturating_add(rhs)
            } else {
                lhs.saturating_sub(rhs)
            };
        }
        Ok(lhs)
    }

    fn degree_term(&mut self, is_address: &dyn Fn(&str) -> bool) -> Result<i64, DefsymError> {
        let mut lhs = self.degree_unary(is_address)?;
        while let Some(op) = self.peek() {
            if op != b'*' && op != b'/' && op != b'%' {
                break;
            }
            self.i += 1;
            self.degree_unary(is_address)?;
            lhs = 0;
        }
        Ok(lhs)
    }

    fn degree_unary(&mut self, is_address: &dyn Fn(&str) -> bool) -> Result<i64, DefsymError> {
        match self.peek() {
            Some(b'-') | Some(b'~') => {
                self.i += 1;
                self.degree_unary(is_address)?;
                Ok(0)
            }
            Some(b'+') => {
                self.i += 1;
                self.degree_unary(is_address)
            }
            Some(b'(') => {
                self.i += 1;
                let d = self.degree_expr(is_address)?;
                match self.peek() {
                    Some(b')') => self.i += 1,
                    _ => return Err(DefsymError::Syntax("missing ')'".to_string())),
                }
                Ok(d)
            }
            _ => {
                let start = self.i;
                self.eval_atom(&|_| Some(0))?;
                let tok = std::str::from_utf8(&self.s[start..self.i])
                    .unwrap_or("")
                    .trim();
                Ok(i64::from(parse_number(tok).is_none() && is_address(tok)))
            }
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

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(n, e)| (n.to_string(), e.to_string()))
            .collect()
    }

    /// The link of the planner tests: `_start` at 0x1000 in an object
    /// (like GNU ld's `-pie` test link) and, where asked, an object `a`
    /// at 0x402000.
    fn plan_and_eval(defs: &[(&str, &str)], object_a: bool) -> Result<Vec<(String, u64)>, String> {
        let link = |n: &str| match n {
            "_start" => LinkSym::Address,
            "a" if object_a => LinkSym::Address,
            _ => LinkSym::Undefined,
        };
        let value = |n: &str| match n {
            "_start" => Some(0x1000),
            "a" if object_a => Some(0x402000),
            _ => None,
        };
        let plan = DefsymPlan::new(&pairs(defs), link)?;
        let finals: Vec<usize> = plan.finals().iter().map(|s| s.index - 1).collect();
        let values = plan.evaluate(&finals, value)?;
        Ok(finals
            .iter()
            .map(|&i| (plan.statements[i].name.clone(), values[i].unwrap()))
            .collect())
    }

    /// Order semantics measured on GNU ld 2.47 (`ld -pie`, `_start` at
    /// 0x1000): earlier definitions are read as they were, forward
    /// references see the last definition, a self reference sees the
    /// input's definition.
    #[test]
    fn plan_matches_gnu_ld_order_semantics() {
        let v = |d: &[(&str, &str)], obj| plan_and_eval(d, obj).unwrap();
        let s = |v: &[(&str, u64)]| -> Vec<(String, u64)> {
            v.iter().map(|(n, x)| (n.to_string(), *x)).collect()
        };
        assert_eq!(
            v(&[("a", "b"), ("b", "_start+4")], false),
            s(&[("a", 0x1004), ("b", 0x1004)])
        );
        assert_eq!(
            v(&[("b", "_start+4"), ("a", "b")], false),
            s(&[("b", 0x1004), ("a", 0x1004)])
        );
        assert_eq!(
            v(&[("a", "b"), ("b", "0x10")], false),
            s(&[("a", 0x10), ("b", 0x10)])
        );
        assert_eq!(
            v(&[("a", "b+1"), ("b", "0x10")], false),
            s(&[("a", 0x11), ("b", 0x10)])
        );
        assert_eq!(
            v(&[("a", "1"), ("b", "a"), ("a", "2")], false),
            s(&[("b", 1), ("a", 2)])
        );
        assert_eq!(v(&[("a", "a+1")], true), s(&[("a", 0x402001)]));
        // A later redefinition is what a forward reference sees.
        assert_eq!(
            v(&[("x", "b"), ("b", "1"), ("b", "2")], false),
            s(&[("x", 2), ("b", 2)])
        );
        assert_eq!(
            plan_and_eval(&[("a", "a+1")], false).unwrap_err(),
            "--defsym:1: undefined symbol `a' referenced in expression"
        );
    }

    /// A cycle is reported with its path.  (GNU ld silently defines every
    /// member as 0 -- a value nobody wrote.)
    #[test]
    fn plan_reports_cycles_with_their_path() {
        assert_eq!(
            plan_and_eval(&[("a", "b"), ("b", "a")], false).unwrap_err(),
            "--defsym:1: circular reference: a -> b -> a"
        );
        assert_eq!(
            plan_and_eval(&[("p", "q+1"), ("q", "r*2"), ("r", "p-_start")], false).unwrap_err(),
            "--defsym:1: circular reference: p -> q -> r -> p"
        );
        // Redefinitions are not cycles: `b` reads the first `a`.
        assert!(plan_and_eval(&[("a", "1"), ("b", "a+1"), ("a", "b")], false).is_ok());
    }

    /// Address or absolute follows the bound definitions, whatever their
    /// order on the command line.
    #[test]
    fn plan_classifies_through_forward_references() {
        let link = |n: &str| {
            if n == "_start" {
                LinkSym::Address
            } else {
                LinkSym::Undefined
            }
        };
        for defs in [
            pairs(&[
                ("a", "b"),
                ("b", "_start+4"),
                ("c", "b-_start"),
                ("d", "c+b"),
            ]),
            pairs(&[
                ("d", "c+b"),
                ("c", "b-_start"),
                ("b", "_start+4"),
                ("a", "b"),
            ]),
        ] {
            let plan = DefsymPlan::new(&defs, link).unwrap();
            let kind = |n: &str| {
                plan.finals()
                    .iter()
                    .find(|s| s.name == n)
                    .unwrap()
                    .is_address
            };
            assert!(kind("a") && kind("b") && kind("d"));
            assert!(!kind("c"));
        }
    }

    /// Only what is needed is evaluated: an alias whose value nobody reads
    /// does not fail a link that cannot compute it.
    #[test]
    fn plan_evaluates_only_what_is_needed() {
        let plan = DefsymPlan::new(&pairs(&[("a", "_start"), ("k", "7")]), |n| {
            if n == "_start" {
                LinkSym::Address
            } else {
                LinkSym::Undefined
            }
        })
        .unwrap();
        assert_eq!(
            plan.aliased_link_symbol(&plan.statements[0]),
            Some("_start")
        );
        let v = plan
            .evaluate_needed(&[1], |_| Err(DefsymError::NeedsLayout(String::new())))
            .unwrap();
        assert_eq!(v, vec![None, Some(7)]);
    }

    #[test]
    fn address_vs_absolute_follows_gnu_ld() {
        let addr = |n: &str| matches!(n, "_start" | "_end" | "a" | "b");
        for (e, want) in [
            ("_start", true),
            ("_start+4", true),
            ("4 + _start", true),
            ("(_start - 8) + 0x10", true),
            ("_end - _start + a", true),
            ("0x1000", false),
            ("0x1000 + 0x10", false),
            ("_end - _start", false),
            ("(_end - _start) / 2", false),
            ("_start * 1", false),
            ("-_start", false),
            ("~_start", false),
            ("4 - _start", false),
            ("a + b", false),
            ("absval + 1", false),
            ("absval + _start", true),
        ] {
            assert_eq!(is_address_expression(e, addr), Ok(want), "{e}");
        }
        assert!(is_address_expression("(_start", addr).is_err());
        assert!(is_address_expression("_start )", addr).is_err());
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
            assert!(classify(bad, defined).is_err(), "{bad}");
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
        // GNU-verbatim, including the missing colon (measured on /usr/bin/ld).
        assert_eq!(
            eval("1/0").unwrap_err().gnu_message(2),
            "--defsym:2 / by zero"
        );
    }

    #[test]
    fn an_undefined_symbol_in_an_expression_names_the_symbol() {
        let e = eval_with_symbols("_start + nosuch", &|_| None).unwrap_err();
        match &e {
            DefsymError::UndefinedSymbol(n) => assert_eq!(n, "_start"),
            other => panic!("expected UndefinedSymbol, got {other:?}"),
        }
        // GNU-verbatim, including the `X' quoting (binutils ld/ldexp.c).
        assert_eq!(
            e.gnu_message(1),
            "--defsym:1: undefined symbol `_start' referenced in expression"
        );
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
    fn the_needs_layout_error_names_the_symbol() {
        // No GNU counterpart; the defsym counter is kept for uniformity.
        let m = DefsymError::NeedsLayout("_start".into()).gnu_message(3);
        assert_eq!(
            m,
            "--defsym:3: symbol `_start' has no address yet at this point of the link"
        );
    }

    #[test]
    fn syntax_errors_always_report_counter_zero() {
        // GNU counts accepted symbols; a syntax error accepts none, so the
        // counter is always 0 — even for the second --defsym (measured).
        let e = classify("((", |_| true).unwrap_err();
        assert!(matches!(e, DefsymError::Syntax(_)), "{e:?}");
        assert_eq!(e.gnu_message(2), "--defsym:0: syntax error");
    }
}
