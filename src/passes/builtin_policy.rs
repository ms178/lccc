//! Publish one compilation's builtin policy onto its IR, once, right after
//! lowering.
//!
//! Two facts decide whether lccc may treat a call as the standard library
//! function, and both are known here and nowhere earlier (a definition can
//! follow its first call): the command line (`-fno-builtin`,
//! `-ffreestanding`, ... -- the driver's [`BuiltinFlags`]) and what the
//! translation unit does with the library's names ([`SymbolInventory`]).
//! The answers travel WITH the IR, never as process state, so inlining,
//! outlining and cloning carry them along:
//!
//! * **An existing call** to `name` gets `CallInfo::assume_library` when the
//!   command line permits `name` and the TU neither defines nor redirects
//!   it.  Folding and inline expansion read only that fact.  A definition of
//!   `memset` says nothing about `memcpy`: GCC 16.2, Clang 23.1 and main's
//!   #844 port all keep expanding `memcpy(d, s, 8)` beside a TU-defined
//!   `memset`, and so does this.
//! * **A synthesized call** (`loop_memset`: memset; `loop_idiom`: memcpy,
//!   memmove; `fortify_fold`: puts, fputs, putchar, fputc, fwrite) to `f` is
//!   created only when `func.pass_policy.builtins` permits it.  It is
//!   withdrawn from the whole TU when the TU defines, or makes opaque, any
//!   name in [`reach`]`(f)`: the new call would land in the TU's own code.
//!   Inside that code it recurses forever (GCC 16.2 turns the loop inside a
//!   TU-defined `memset` into `call memset`; Clang 23.1 does it once a
//!   helper is inlined into `memset`), and anywhere else it adds calls the
//!   program never made to a function the program wrote (main's
//!   `check_libcall_synthesis_no_selfcall.sh` runtime row counts them).
//!   A per-function reachability rule would avoid only the first; it was
//!   built and measured (S64): none of the 61 corpus programs nor zstd
//!   defines any of these names, so its extra precision is worth nothing
//!   measurable and its weaker contract is not worth its call graph.
//!
//! What a TU does to a library name ([`SymbolInventory`]):
//!
//! * **defines it with a body** -- a function definition whose C name is
//!   the symbol.  NOT an `extern inline __attribute__((gnu_inline))` body:
//!   it is lowered only for inlining and becomes a declaration (glibc's
//!   `_FORTIFY_SOURCE` headers define `memcpy`, `memset` and `printf` that
//!   way; counting them would withdraw the builtins from every fortified
//!   TU).
//! * **makes it opaque** -- the name reaches something this pass cannot
//!   analyse, so every function treats it as defined: an `__asm__("label")`
//!   that re-spells it (on a declaration or a definition; a label equal to
//!   the name is an identity), an `__attribute__((alias))` or `#pragma weak
//!   a = b` naming it (alias and target), or any mention in top-level
//!   assembly.  The assembly reading is deliberately a mention, not a parse
//!   (`common::asm_scan`, shared with `dead_statics`): GAS has too many ways
//!   to define a symbol (`s:`, `s = e`, `.set`, `.equ`, `.equiv`, `.eqv`,
//!   `.weakref`, ...) for an enumeration to be safe, and a missed one is a
//!   self-recursion; a blob that merely calls the function costs one
//!   optimization in a rare TU.
//!
//! An `__asm__` label, alias or top-level-asm mention and a body are
//! treated alike: each means calls to the name do not reach the library.
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::pass_policy::{BuiltinFlags, Libcall};
use crate::ir::instruction::Instruction;
use crate::ir::module::IrModule;

/// The library functions a synthesized call to `f` may execute, through
/// public (interposable) symbols, besides `f` itself: a TU definition of
/// any of them is reached by the new call.  Evidence, not guesses -- an
/// extra edge costs an optimization, a missing one a recursion:
///
/// * `memmove` -> `memcpy`: musl `src/string/memmove.c` returns
///   `memcpy(d, s, n)` for the disjoint case (newlib and historical BSD do
///   the same).
/// * `puts` -> `fputs` -> `fwrite`, `strlen`: musl `src/stdio/puts.c` calls
///   `fputs(s, stdout)`, and `src/stdio/fputs.c` calls `strlen(s)` and
///   `fwrite(s, 1, l, f)`.
///
/// glibc reaches none of these through public symbols, and no measured
/// `memcpy`/`memset`/`putchar`/`fputc`/`fwrite` calls a public sibling.
fn reach(f: &str) -> &'static [&'static str] {
    match f {
        "memmove" => &["memmove", "memcpy"],
        "puts" => &["puts", "fputs", "fwrite", "strlen"],
        "fputs" => &["fputs", "fwrite", "strlen"],
        "memset" => &["memset"],
        "memcpy" => &["memcpy"],
        "putchar" => &["putchar"],
        "fputc" => &["fputc"],
        "fwrite" => &["fwrite"],
        _ => &[],
    }
}

/// The calls a pass can create (see the module docs).
const SYNTHESIZED: [Libcall; 8] = [
    Libcall::Memset,
    Libcall::Memcpy,
    Libcall::Memmove,
    Libcall::Puts,
    Libcall::Fputs,
    Libcall::Putchar,
    Libcall::Fputc,
    Libcall::Fwrite,
];

/// What the translation unit does with symbol names (see the module docs).
#[derive(Debug, Default)]
pub(crate) struct SymbolInventory {
    /// Symbols a function definition in the TU implements.
    bodies: FxHashSet<String>,
    /// Names whose calls do not reach the library function and whose
    /// implementation this pass cannot see.
    opaque: FxHashSet<String>,
}

impl SymbolInventory {
    pub(crate) fn collect(module: &IrModule) -> Self {
        let mut inv = SymbolInventory::default();
        for func in &module.functions {
            match module.asm_labels.get(&func.name) {
                // The C name is re-spelled: calls to it reach `label`, and
                // a definition defines `label`, not the C name.
                Some(label) if label != &func.name => {
                    inv.opaque.insert(func.name.clone());
                    if !func.is_declaration {
                        inv.opaque.insert(label.clone());
                    }
                }
                _ if func.is_declaration || func.is_gnu_inline_def => {}
                _ => {
                    inv.bodies.insert(func.name.clone());
                }
            }
        }
        for (alias, target, _weak) in &module.aliases {
            inv.opaque.insert(alias.clone());
            inv.opaque.insert(target.clone());
        }
        inv
    }

    /// Does the TU answer calls to `name` itself (or send them elsewhere)?
    /// `asm` is the module's top-level assembly.
    pub(crate) fn blocks_library_assumption(&self, name: &str, asm: &[String]) -> bool {
        self.bodies.contains(name)
            || self.opaque.contains(name)
            || crate::common::asm_scan::any_blob_mentions(asm, name)
    }
}

pub(crate) fn apply(module: &mut IrModule, cli: &BuiltinFlags) {
    let inventory = SymbolInventory::collect(module);
    let asm = &module.toplevel_asm;
    let mut policy = cli.policy();
    for l in SYNTHESIZED {
        if reach(l.name())
            .iter()
            .any(|name| inventory.blocks_library_assumption(name, asm))
        {
            policy.withdraw_libcall(l);
        }
    }
    let asm_labels = &module.asm_labels;
    let mut blocked: FxHashMap<String, bool> = FxHashMap::default();
    for func in &mut module.functions {
        let mut function_policy = policy;
        // A GNU extern-inline body is not an out-of-line TU definition, so it
        // must not globally hide the real libc symbol.  But if that body is
        // cloned, optimizing its own implementation into a call to the same
        // effective symbol changes the inline definition's semantics.  Carry
        // the local withdrawal with the body; the inliner conservatively
        // merges it into callers that receive the clone.
        if func.is_gnu_inline_def {
            function_policy.withdraw_name(&func.name);
            if let Some(label) = asm_labels.get(&func.name) {
                function_policy.withdraw_name(label);
            }
        }
        func.pass_policy.builtins = function_policy;
        for inst in func
            .blocks
            .iter_mut()
            .flat_map(|b| b.instructions.iter_mut())
        {
            let Instruction::Call { func: callee, info } = inst else {
                continue;
            };
            // Lowering set it for `__builtin_*` spellings and the compiler's
            // own copies; those are the builtin whatever the TU defines.
            if info.assume_library {
                continue;
            }
            let blocks = match blocked.get(callee.as_str()) {
                Some(&b) => b,
                None => {
                    let b = inventory.blocks_library_assumption(callee, asm);
                    blocked.insert(callee.clone(), b);
                    b
                }
            };
            info.assume_library = cli.may_assume(callee) && !blocks;
        }
    }
}

/// Largest constant `memcpy` length [`canonicalize_library_memcpy`] turns
/// into the native `Memcpy` instruction.  64 bytes is the x86-64 emitter's
/// straight-line window (`emit_memcpy_impl_impl`: one `movdqu`/`movq` pair
/// per chunk; above it the emitter switches to a copy loop, which a tuned
/// libc `memcpy` beats) and lies inside AArch64's 256-byte `ldp`/`stp`
/// window.  Everything the copy-aware passes model (aggregate SROA, the
/// store/load and memcpy-chain forwards, DSE) is far below it.
const LIBRARY_MEMCPY_INLINE_MAX: i64 = 64;

/// Give a plain `memcpy(d, s, n)` with a small constant `n` the same IR as
/// `__builtin_memcpy(d, s, n)`: the native `Memcpy` instruction.
///
/// Lowering keeps every plain spelling a `Call`, because whether the name
/// reaches the C library is known only once [`apply`] has seen the whole
/// TU.  After it, `CallInfo::assume_library` is that answer (false under
/// `-fno-builtin[-memcpy]`, `-ffreestanding`, or when the TU defines,
/// relabels or aliases `memcpy`), so a call carrying it has exactly the C
/// contract `Instruction::Memcpy` assumes: `n` bytes copied, no overlap
/// (overlap is UB for `memcpy`), destination returned.  GCC 16.2 and Clang
/// 23.1 make the same identification for `memcpy` under the same flags.
/// Without it a user-written `memcpy(&tmp, p, sizeof tmp)` stays opaque to
/// every pass that models copies -- the struct round-trip through a local
/// (`memcpy(&s, src, 16); s.f++; memcpy(dst, &s, 16)`) was left as two
/// copies plus a stack reload, while the `__builtin_` spelling of the same
/// code is split into registers.
///
/// The result, when the call has one, becomes a `Copy` of the destination
/// operand; copy propagation removes it.  `n` is accepted as a literal or as
/// a `Copy`/`Cast` of one (the size_t promotion of an `int` literal), and
/// only when it is in `1..=LIBRARY_MEMCPY_INLINE_MAX` -- a positive value
/// that small is unchanged by any integer cast of at least 32 bits, so the
/// cast's direction never matters.  Targets whose `Memcpy` expansion is
/// slower than the library call are left alone: RISC-V expands it as a
/// byte loop and i686 as `rep movsb`.
pub(crate) fn canonicalize_library_memcpy(module: &mut IrModule, target: crate::backend::Target) {
    use crate::backend::Target;
    use crate::ir::reexports::{IrConst, Operand, Value};
    if !matches!(target, Target::X86_64 | Target::Aarch64) {
        return;
    }
    fn small_len(c: &IrConst) -> Option<usize> {
        let v = match *c {
            IrConst::I128(v) => i64::try_from(v).ok()?,
            other => other.to_i64()?,
        };
        (1..=LIBRARY_MEMCPY_INLINE_MAX)
            .contains(&v)
            .then_some(v as usize)
    }
    fn is_candidate(func: &str, info: &crate::ir::instruction::CallInfo) -> bool {
        func == "memcpy"
            && info.assume_library
            && !info.is_variadic
            && info.args.len() == 3
            && matches!(info.args[0], Operand::Value(_))
            && matches!(info.args[1], Operand::Value(_))
    }
    for func in &mut module.functions {
        if func.is_declaration {
            continue;
        }
        let any = func.blocks.iter().any(|b| {
            b.instructions
                .iter()
                .any(|i| matches!(i, Instruction::Call { func: f, info } if is_candidate(f, info)))
        });
        if !any {
            continue;
        }
        // Constant definitions a size operand may name (SSA: one def each).
        let mut const_of: FxHashMap<Value, usize> = FxHashMap::default();
        for inst in func.blocks.iter().flat_map(|b| b.instructions.iter()) {
            let (dest, c) = match inst {
                Instruction::Copy {
                    dest,
                    src: Operand::Const(c),
                } => (*dest, c),
                Instruction::Cast {
                    dest,
                    src: Operand::Const(c),
                    from_ty,
                    to_ty,
                } if from_ty.is_integer() && to_ty.is_integer() && to_ty.size() >= 4 => (*dest, c),
                _ => continue,
            };
            if let Some(n) = small_len(c) {
                const_of.insert(dest, n);
            }
        }
        for block in &mut func.blocks {
            let has_spans = block.source_spans.len() == block.instructions.len();
            let mut i = 0;
            while i < block.instructions.len() {
                let rewrite = match &block.instructions[i] {
                    Instruction::Call { func: f, info } if is_candidate(f, info) => {
                        let size = match &info.args[2] {
                            Operand::Const(c) => small_len(c),
                            Operand::Value(v) => const_of.get(v).copied(),
                        };
                        match (size, &info.args[0], &info.args[1]) {
                            (Some(size), Operand::Value(d), Operand::Value(s)) => {
                                Some((*d, *s, size, info.dest))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let Some((dest, src, size, result)) = rewrite else {
                    i += 1;
                    continue;
                };
                block.instructions[i] = Instruction::Memcpy { dest, src, size };
                i += 1;
                if let Some(r) = result {
                    block.instructions.insert(
                        i,
                        Instruction::Copy {
                            dest: r,
                            src: Operand::Value(dest),
                        },
                    );
                    if has_spans {
                        let span = block.source_spans[i - 1];
                        block.source_spans.insert(i, span);
                    }
                    i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::pass_policy::BuiltinPolicy;
    use crate::common::types::IrType;
    use crate::ir::instruction::{BasicBlock, CallInfo, Terminator};
    use crate::ir::module::IrFunction;
    use crate::ir::reexports::BlockId;

    fn func(name: &str, body: bool, calls: &[&str]) -> IrFunction {
        let mut f = IrFunction::new(name.to_string(), IrType::Void, vec![], false);
        f.is_declaration = !body;
        if body {
            f.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: calls
                    .iter()
                    .map(|c| Instruction::Call {
                        func: (*c).to_string(),
                        info: CallInfo::default(),
                    })
                    .collect(),
                terminator: Terminator::Return(None),
                source_spans: Vec::new(),
            });
        }
        f
    }

    fn static_func(name: &str, calls: &[&str]) -> IrFunction {
        let mut f = func(name, true, calls);
        f.is_static = true;
        f
    }

    fn module(funcs: Vec<IrFunction>) -> IrModule {
        let mut m = IrModule::new();
        m.functions = funcs;
        m
    }

    fn assumed(m: &IrModule, caller: &str) -> Vec<(String, bool)> {
        let f = m.functions.iter().find(|f| f.name == caller).unwrap();
        f.blocks[0]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Call { func, info } => Some((func.clone(), info.assume_library)),
                _ => None,
            })
            .collect()
    }

    fn may_synthesize(m: &IrModule, in_func: &str, l: Libcall) -> bool {
        let f = m.functions.iter().find(|f| f.name == in_func).unwrap();
        f.pass_policy.builtins.may_synthesize(l)
    }

    fn pairs(v: &[(&str, bool)]) -> Vec<(String, bool)> {
        v.iter().map(|(n, a)| (n.to_string(), *a)).collect()
    }

    #[test]
    fn hosted_tu_assumes_every_library_call() {
        let mut m = module(vec![func("main", true, &["memset", "printf", "my_fn"])]);
        apply(&mut m, &BuiltinFlags::default());
        assert!(assumed(&m, "main").iter().all(|(_, a)| *a));
        assert_eq!(m.functions[0].pass_policy.builtins, BuiltinPolicy::HOSTED);
    }

    /// GCC 16.2, Clang 23.1 and #844 all expand `memcpy(d, s, 8)` beside a
    /// TU-defined `memset`: a definition withdraws the assumption for its own
    /// name only.
    #[test]
    fn a_definition_withdraws_its_own_name_only() {
        let mut m = module(vec![
            func("memset", true, &[]),
            func("main", true, &["memcpy", "memset", "puts"]),
        ]);
        apply(&mut m, &BuiltinFlags::default());
        assert_eq!(
            assumed(&m, "main"),
            pairs(&[("memcpy", true), ("memset", false), ("puts", true)])
        );
    }

    /// From #844: a declaration says the symbol is referenced, not provided.
    #[test]
    fn declarations_and_gnu_inline_bodies_are_not_definitions() {
        let mut wrapper = func("memcpy", true, &["__memcpy_chk"]);
        wrapper.is_gnu_inline_def = true;
        let mut m = module(vec![
            wrapper,
            func("printf", false, &[]),
            func("main", true, &["memcpy", "printf"]),
        ]);
        apply(&mut m, &BuiltinFlags::default());
        assert!(assumed(&m, "main").iter().all(|(_, a)| *a));
        assert!(may_synthesize(&m, "main", Libcall::Memcpy));
    }

    /// The loop inside `memset` must not become `call memset` (GCC 16.2
    /// does that), nor may a helper `memset` reaches (Clang 23.1 does that
    /// once the helper is inlined), nor may any other function add calls to
    /// the TU's own `memset`.  The other synthesized calls stay available.
    #[test]
    fn a_definition_withdraws_its_synthesis_from_the_whole_tu() {
        let mut m = module(vec![
            static_func("fill", &[]),
            func("memset", true, &["fill"]),
            static_func("unrelated", &[]),
        ]);
        apply(&mut m, &BuiltinFlags::default());
        for f in ["memset", "fill", "unrelated"] {
            assert!(!may_synthesize(&m, f, Libcall::Memset), "{f}");
            for l in [Libcall::Memcpy, Libcall::Memmove, Libcall::Puts] {
                assert!(may_synthesize(&m, f, l), "{f} {l:?}");
            }
        }
    }

    /// From #844: a synthesized `memmove` reaches the public `memcpy` (musl
    /// `memmove.c`), so a TU that defines `memcpy` may not create one;
    /// `memcpy` itself does not reach `memmove`.
    #[test]
    fn memmove_synthesis_is_closed_over_memcpy() {
        let mut m = module(vec![func("memcpy", true, &[]), static_func("other", &[])]);
        apply(&mut m, &BuiltinFlags::default());
        for f in ["memcpy", "other"] {
            assert!(!may_synthesize(&m, f, Libcall::Memmove), "{f}");
            assert!(!may_synthesize(&m, f, Libcall::Memcpy), "{f}");
            assert!(may_synthesize(&m, f, Libcall::Memset), "{f}");
        }

        let mut m = module(vec![func("memmove", true, &[])]);
        apply(&mut m, &BuiltinFlags::default());
        assert!(may_synthesize(&m, "memmove", Libcall::Memcpy));
    }

    /// musl's `puts` calls the public `fputs`, which calls `fwrite` and
    /// `strlen`: a `printf("%s\n", s)` inside a TU-defined `fputs` must not
    /// become `puts(s)`.
    #[test]
    fn puts_synthesis_is_closed_over_fputs_fwrite_and_strlen() {
        for defined in ["fputs", "fwrite", "strlen"] {
            let mut m = module(vec![func(defined, true, &[])]);
            apply(&mut m, &BuiltinFlags::default());
            assert!(!may_synthesize(&m, defined, Libcall::Puts), "{defined}");
            assert!(may_synthesize(&m, defined, Libcall::Putchar), "{defined}");
        }
    }

    /// From #844: an asm label re-spells the name, on a definition (the
    /// label is defined) or a declaration (nothing is); a label equal to the
    /// name is an identity.  The names are opaque, so no function may
    /// synthesize a call that reaches them.
    #[test]
    fn asm_labels_redirect_and_self_labels_do_not() {
        let mut m = module(vec![
            func("memcpy", false, &[]),
            func("main", true, &["memcpy", "memset"]),
        ]);
        m.asm_labels
            .insert("memcpy".to_string(), "my_copy".to_string());
        apply(&mut m, &BuiltinFlags::default());
        assert_eq!(
            assumed(&m, "main"),
            pairs(&[("memcpy", false), ("memset", true)])
        );
        assert!(!may_synthesize(&m, "main", Libcall::Memcpy));
        assert!(!may_synthesize(&m, "main", Libcall::Memmove));
        assert!(may_synthesize(&m, "main", Libcall::Memset));

        let mut m = module(vec![
            func("my_set", true, &[]),
            func("main", true, &["memset", "my_set"]),
        ]);
        m.asm_labels
            .insert("my_set".to_string(), "memset".to_string());
        apply(&mut m, &BuiltinFlags::default());
        assert!(!may_synthesize(&m, "main", Libcall::Memset));
        assert_eq!(
            assumed(&m, "main"),
            pairs(&[("memset", false), ("my_set", false)])
        );

        let mut m = module(vec![
            func("memcpy", false, &[]),
            func("main", true, &["memcpy"]),
        ]);
        m.asm_labels
            .insert("memcpy".to_string(), "memcpy".to_string());
        apply(&mut m, &BuiltinFlags::default());
        assert_eq!(assumed(&m, "main"), pairs(&[("memcpy", true)]));
        assert!(may_synthesize(&m, "main", Libcall::Memcpy));
    }

    #[test]
    fn gnu_inline_libcall_body_keeps_a_local_withdrawal() {
        let mut inline = func("memset", true, &[]);
        inline.is_gnu_inline_def = true;
        let mut m = module(vec![inline, func("main", true, &[])]);
        apply(&mut m, &BuiltinFlags::default());

        assert!(
            may_synthesize(&m, "main", Libcall::Memset),
            "no out-of-line TU definition hides libc from ordinary callers"
        );
        assert!(
            !may_synthesize(&m, "memset", Libcall::Memset),
            "the inline implementation must not rewrite itself into libc"
        );
    }

    /// From #844: an alias defines both names.
    #[test]
    fn an_alias_makes_both_names_opaque() {
        let mut m = module(vec![
            func("impl_puts", true, &[]),
            func("main", true, &["puts", "printf"]),
        ]);
        m.aliases
            .push(("puts".to_string(), "impl_puts".to_string(), false));
        apply(&mut m, &BuiltinFlags::default());
        assert_eq!(
            assumed(&m, "main"),
            pairs(&[("puts", false), ("printf", true)])
        );
        assert!(!may_synthesize(&m, "main", Libcall::Puts));
        assert!(may_synthesize(&m, "main", Libcall::Putchar));
    }

    /// From #844: top-level assembly that mentions a name makes it opaque,
    /// whichever GAS spelling defines it; a substring is not a mention.
    #[test]
    fn toplevel_assembly_mentions_are_opaque() {
        for blob in [
            ".globl memcpy\nmemcpy: rep movsb; ret",
            "memcpy = __my_memcpy",
            ".equiv memcpy, other",
            ".weakref memcpy, impl",
        ] {
            let mut m = module(vec![func("main", true, &["memcpy", "memset"])]);
            m.toplevel_asm.push(blob.to_string());
            apply(&mut m, &BuiltinFlags::default());
            assert_eq!(
                assumed(&m, "main"),
                pairs(&[("memcpy", false), ("memset", true)]),
                "{blob}"
            );
            assert!(!may_synthesize(&m, "main", Libcall::Memmove), "{blob}");
        }
        let mut m = module(vec![func("main", true, &["memcpy"])]);
        m.toplevel_asm.push("call memcpy_fast".to_string());
        apply(&mut m, &BuiltinFlags::default());
        assert_eq!(assumed(&m, "main"), pairs(&[("memcpy", true)]));
    }

    #[test]
    fn per_name_withdrawal_reaches_any_call() {
        let mut m = module(vec![func("main", true, &["strlen", "sqrt", "memset"])]);
        let mut cli = BuiltinFlags::default();
        cli.withdraw_name("strlen");
        apply(&mut m, &cli);
        assert_eq!(
            assumed(&m, "main"),
            pairs(&[("strlen", false), ("sqrt", true), ("memset", true)])
        );
    }

    #[test]
    fn cli_withdrawal_spares_builtin_spellings() {
        let mut f = func("main", true, &["memset", "memset"]);
        if let Instruction::Call { info, .. } = &mut f.blocks[0].instructions[1] {
            info.assume_library = true; // lowered from `__builtin_memset`
        }
        let mut m = module(vec![f]);
        let mut cli = BuiltinFlags::default();
        cli.withdraw_all();
        apply(&mut m, &cli);
        assert_eq!(
            assumed(&m, "main"),
            pairs(&[("memset", false), ("memset", true)])
        );
        assert!(!may_synthesize(&m, "main", Libcall::Memset));
    }
}

#[cfg(test)]
mod library_memcpy_tests {
    use super::*;
    use crate::backend::Target;
    use crate::common::source::Span;
    use crate::common::types::IrType;
    use crate::ir::instruction::{BasicBlock, CallInfo, Terminator};
    use crate::ir::module::IrFunction;
    use crate::ir::reexports::{BlockId, IrConst, Operand, Value};

    /// `memcpy(v1, v2, len)` with result `v9`, after an optional size def.
    fn copy_fn(size_def: Option<Instruction>, len: Operand, assume: bool) -> IrModule {
        let mut f = IrFunction::new("f".to_string(), IrType::Void, vec![], false);
        let mut insts: Vec<Instruction> = size_def.into_iter().collect();
        insts.push(Instruction::Call {
            func: "memcpy".to_string(),
            info: CallInfo {
                dest: Some(Value(9)),
                args: vec![Operand::Value(Value(1)), Operand::Value(Value(2)), len],
                arg_types: vec![IrType::Ptr, IrType::Ptr, IrType::U64],
                return_type: IrType::Ptr,
                assume_library: assume,
                ..CallInfo::default()
            },
        });
        let spans = (0..insts.len() as u32)
            .map(|i| Span::new(i, i + 1, 0))
            .collect();
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(None),
            source_spans: spans,
        });
        let mut m = IrModule::new();
        m.functions = vec![f];
        m
    }

    fn body(m: &IrModule) -> &BasicBlock {
        &m.functions[0].blocks[0]
    }

    fn rewritten(m: &IrModule, size: usize) -> bool {
        let b = body(m);
        let n = b.instructions.len();
        n >= 2
            && matches!(b.instructions[n - 2],
                Instruction::Memcpy { dest: Value(1), src: Value(2), size: s } if s == size)
            && matches!(
                b.instructions[n - 1],
                Instruction::Copy {
                    dest: Value(9),
                    src: Operand::Value(Value(1))
                }
            )
            && b.source_spans.len() == n
            && b.source_spans[n - 1] == b.source_spans[n - 2]
    }

    fn untouched(m: &IrModule) -> bool {
        matches!(body(m).instructions.last(), Some(Instruction::Call { .. }))
    }

    #[test]
    fn literal_length_becomes_native_memcpy_and_result_copy() {
        let mut m = copy_fn(None, Operand::Const(IrConst::I64(16)), true);
        canonicalize_library_memcpy(&mut m, Target::X86_64);
        assert!(rewritten(&m, 16));
        let mut m = copy_fn(None, Operand::Const(IrConst::I64(64)), true);
        canonicalize_library_memcpy(&mut m, Target::Aarch64);
        assert!(rewritten(&m, 64));
    }

    /// The size_t promotion of an `int` literal reaches the call as a Cast.
    #[test]
    fn promoted_literal_length_is_seen_through_the_cast() {
        let cast = Instruction::Cast {
            dest: Value(5),
            src: Operand::Const(IrConst::I32(24)),
            from_ty: IrType::I32,
            to_ty: IrType::U64,
        };
        let mut m = copy_fn(Some(cast), Operand::Value(Value(5)), true);
        canonicalize_library_memcpy(&mut m, Target::X86_64);
        assert!(rewritten(&m, 24));
        assert_eq!(body(&m).instructions.len(), 3);
    }

    /// `-fno-builtin`, `-ffreestanding` or a TU-defined `memcpy` leave
    /// `assume_library` false: the call may not be the C function.
    #[test]
    fn a_call_not_known_to_reach_the_library_stays_a_call() {
        let mut m = copy_fn(None, Operand::Const(IrConst::I64(16)), false);
        canonicalize_library_memcpy(&mut m, Target::X86_64);
        assert!(untouched(&m));
    }

    #[test]
    fn a_tu_defined_memcpy_blocks_the_rewrite_end_to_end() {
        let mut m = copy_fn(None, Operand::Const(IrConst::I64(16)), false);
        let mut def = IrFunction::new("memcpy".to_string(), IrType::Ptr, vec![], false);
        def.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        m.functions.push(def);
        apply(&mut m, &BuiltinFlags::default());
        canonicalize_library_memcpy(&mut m, Target::X86_64);
        assert!(untouched(&m));
    }

    #[test]
    fn zero_oversized_and_runtime_lengths_stay_calls() {
        for len in [
            Operand::Const(IrConst::I64(0)),
            Operand::Const(IrConst::I64(65)),
            Operand::Const(IrConst::I64(-1)),
            Operand::Const(IrConst::I128(1 << 64 | 16)),
            Operand::Value(Value(7)),
        ] {
            let mut m = copy_fn(None, len, true);
            canonicalize_library_memcpy(&mut m, Target::X86_64);
            assert!(untouched(&m));
        }
    }

    /// RISC-V expands `Memcpy` as a byte loop and i686 as `rep movsb`; both
    /// lose to the library call, so neither target is rewritten.
    #[test]
    fn targets_with_a_slow_memcpy_expansion_keep_the_call() {
        for t in [Target::Riscv64, Target::I686] {
            let mut m = copy_fn(None, Operand::Const(IrConst::I64(16)), true);
            canonicalize_library_memcpy(&mut m, t);
            assert!(untouched(&m));
        }
    }
}
