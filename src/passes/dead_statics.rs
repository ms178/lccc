//! Dead static function and global elimination.
//!
//! After optimization passes eliminate dead code paths, some static inline
//! functions and static const globals from headers may no longer be referenced.
//! Keeping them wastes code size and may cause linker errors if they reference
//! symbols that don't exist in this translation unit.
//!
//! Algorithm: assign a dense ID to every function and global (not to names),
//! build the intra-module reference graph, then run worklist reachability from
//! roots (externally visible / explicitly used symbols, aliases, ctors/dtors,
//! toplevel-asm mentions, and address-taken static always_inline functions).
//! Unreachable static definitions are removed; `symbol_attrs` is filtered to
//! surviving / still-referenced names so leftover visibility directives cannot
//! break the assembler or linker.
//!
//! Invariant: analysis borrows `module` immutably. All borrowed maps are
//! dropped before the module is mutated.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrModule};

/// Remove internal-linkage (static) functions and globals that are unreachable.
pub(crate) fn eliminate_dead_static_functions(module: &mut IrModule) {
    let n_funcs = module.functions.len();

    // Phases 1–4 borrow module strings; the block ends that borrow before we
    // mutate `module` in phases 5–6.
    let (reachable, address_taken) = {
        let name_to_ids = build_name_index(module);
        let nsyms = n_funcs + module.globals.len();
        let (func_refs, global_refs, address_taken) =
            build_refs_and_address_taken(module, &name_to_ids, nsyms);
        let reachable = compute_reachability(
            module,
            n_funcs,
            nsyms,
            &func_refs,
            &global_refs,
            &address_taken,
            &name_to_ids,
        );
        (reachable, address_taken)
    };

    remove_unreachable(module, n_funcs, &reachable, &address_taken);
    filter_symbol_attrs(module);
}

/// Phase 1: map each symbol *instance* to a dense ID.
///
/// * function `i`  →  id `i`
/// * global   `j`  →  id `n_funcs + j`
///
/// Names are *not* identities. Two symbols that share a name (duplicate IR
/// names, empty names, function + global) each get their own node; a reference
/// to that name marks every matching node. Collapsing them onto one ID used to
/// drop outgoing edges of all but the last occupant — a miscompilation hazard.
fn build_name_index(module: &IrModule) -> FxHashMap<&str, Vec<usize>> {
    let mut name_to_ids: FxHashMap<&str, Vec<usize>> = FxHashMap::default();
    name_to_ids.reserve(module.functions.len() + module.globals.len());

    for (i, func) in module.functions.iter().enumerate() {
        name_to_ids.entry(func.name.as_str()).or_default().push(i);
    }
    let n_funcs = module.functions.len();
    for (j, global) in module.globals.iter().enumerate() {
        name_to_ids
            .entry(global.name.as_str())
            .or_default()
            .push(n_funcs + j);
    }
    name_to_ids
}

#[inline]
fn ids_for_name<'a>(name_to_ids: &'a FxHashMap<&str, Vec<usize>>, name: &str) -> &'a [usize] {
    match name_to_ids.get(name) {
        Some(ids) => ids.as_slice(),
        None => &[],
    }
}

/// Extract the base symbol from an inline-asm operand (`foo+8`, `foo-4`,
/// `foo@GOTPCREL`, `*foo`, `$foo`).
fn asm_symbol_base(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0;
    while start < bytes.len() && !is_asm_ident_start(bytes[start]) {
        start += 1;
    }
    let mut end = start;
    while end < bytes.len() && is_asm_ident_char(bytes[end]) {
        end += 1;
    }
    if start < end {
        &s[start..end]
    } else {
        s
    }
}

#[inline]
fn is_asm_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'.'
}

#[inline]
fn is_asm_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'$'
}

/// True iff `name` occurs in `asm` as its own token, not as a substring of a
/// longer identifier (`log` must not match `logarithm`; `foo` must not match
/// `foobar`). Names that themselves contain `@`/`+` still match exactly.
fn asm_mentions_symbol(asm: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let bytes = asm.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = asm[search_from..].find(name) {
        let abs = search_from + rel;
        let before_ok = abs == 0 || !is_asm_ident_char(bytes[abs - 1]);
        let after = abs + name.len();
        let after_ok = after == bytes.len() || !is_asm_ident_char(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        // `name` is a valid UTF-8 substring of `asm`, so this stays on a
        // character boundary (unlike `abs + 1`).
        search_from = abs + name.len();
    }
    false
}

fn toplevel_asm_mentions<S: AsRef<str>>(blobs: &[S], name: &str) -> bool {
    blobs.iter().any(|s| asm_mentions_symbol(s.as_ref(), name))
}

/// Visit every symbol name an instruction can reference.
///
/// The `bool` is `true` when the reference takes the symbol's address
/// (`GlobalAddr`, inline-asm operand) rather than calling it directly.
///
/// Any new `Instruction` variant that can name a function or global **must**
/// be added here; both the reference graph and `symbol_attrs` filtering go
/// through this helper.
fn for_each_instruction_symbol<'a>(inst: &'a Instruction, mut visit: impl FnMut(&'a str, bool)) {
    match inst {
        Instruction::Call { func: callee, .. } => visit(callee.as_str(), false),
        Instruction::GlobalAddr { name, .. } => visit(name.as_str(), true),
        Instruction::InlineAsm { input_symbols, .. } => {
            for s in input_symbols.iter().flatten() {
                visit(asm_symbol_base(s), true);
            }
        }
        _ => {}
    }
}

/// Append every module symbol named `name` to `refs`. External names are
/// ignored — they are not candidates for intra-module DCE, and inventing IDs
/// for them previously produced unregistered orphans.
fn push_named_refs(name: &str, name_to_ids: &FxHashMap<&str, Vec<usize>>, refs: &mut Vec<usize>) {
    refs.extend_from_slice(ids_for_name(name_to_ids, name));
}

fn mark_address_taken(name: &str, name_to_ids: &FxHashMap<&str, Vec<usize>>, address_taken: &mut [bool]) {
    for &id in ids_for_name(name_to_ids, name) {
        if let Some(slot) = address_taken.get_mut(id) {
            *slot = true;
        }
    }
}

fn finalize_refs(refs: &mut Vec<usize>) {
    refs.sort_unstable();
    refs.dedup();
}

/// Phases 2–3: per-function / per-global reference lists and the address-taken
/// bitvector, in a single instruction walk.
fn build_refs_and_address_taken(
    module: &IrModule,
    name_to_ids: &FxHashMap<&str, Vec<usize>>,
    nsyms: usize,
) -> (Vec<Vec<usize>>, Vec<Vec<usize>>, Vec<bool>) {
    let mut address_taken = vec![false; nsyms];

    let mut func_refs = Vec::with_capacity(module.functions.len());
    for func in &module.functions {
        if func.is_declaration {
            func_refs.push(Vec::new());
            continue;
        }
        let mut refs = Vec::with_capacity(16);
        for block in &func.blocks {
            for inst in &block.instructions {
                for_each_instruction_symbol(inst, |name, takes_address| {
                    push_named_refs(name, name_to_ids, &mut refs);
                    if takes_address {
                        mark_address_taken(name, name_to_ids, &mut address_taken);
                    }
                });
            }
        }
        finalize_refs(&mut refs);
        func_refs.push(refs);
    }

    let mut global_refs = Vec::with_capacity(module.globals.len());
    for global in &module.globals {
        let mut refs = Vec::with_capacity(16);
        global.init.for_each_ref(&mut |name| {
            push_named_refs(name, name_to_ids, &mut refs);
            // An initializer that names a symbol takes its address
            // (function pointer, object address, label difference, …).
            mark_address_taken(name, name_to_ids, &mut address_taken);
        });
        finalize_refs(&mut refs);
        global_refs.push(refs);
    }

    (func_refs, global_refs, address_taken)
}

#[inline]
fn is_marked(bits: &[bool], id: usize) -> bool {
    bits.get(id).copied().unwrap_or(false)
}

/// Mark `id` reachable and enqueue it. Out-of-range IDs are ignored; we never
/// invent IDs after the bitvectors are sized, so a resize/panic path is gone.
fn mark_reachable(id: usize, reachable: &mut [bool], worklist: &mut Vec<usize>) {
    if let Some(slot) = reachable.get_mut(id) {
        if !*slot {
            *slot = true;
            worklist.push(id);
        }
    }
}

fn mark_named(
    name: &str,
    name_to_ids: &FxHashMap<&str, Vec<usize>>,
    reachable: &mut [bool],
    worklist: &mut Vec<usize>,
) {
    for &id in ids_for_name(name_to_ids, name) {
        mark_reachable(id, reachable, worklist);
    }
}

/// Phase 4: worklist reachability from roots.
///
/// Roots:
/// * non-static function definitions, or anything with `is_used`
/// * non-static / common / `used` globals (extern declarations are skipped;
///   they have no initializer edges and are always retained later)
/// * aliases (alias name **and** target)
/// * constructors and destructors
/// * address-taken static `always_inline` definitions (function-pointer
///   identity; their callees must survive too)
/// * static symbols whose names appear as tokens in toplevel asm
///
/// Propagation order is LIFO (DFS). For a pure reachability fixpoint this is
/// equivalent to BFS; each ID is enqueued at most once.
fn compute_reachability(
    module: &IrModule,
    n_funcs: usize,
    nsyms: usize,
    func_refs: &[Vec<usize>],
    global_refs: &[Vec<usize>],
    address_taken: &[bool],
    name_to_ids: &FxHashMap<&str, Vec<usize>>,
) -> Vec<bool> {
    let mut reachable = vec![false; nsyms];
    let mut worklist = Vec::with_capacity(nsyms.min(64));

    for (i, func) in module.functions.iter().enumerate() {
        if func.is_declaration {
            continue;
        }
        if !func.is_static || func.is_used {
            mark_reachable(i, &mut reachable, &mut worklist);
        }
    }

    for (j, global) in module.globals.iter().enumerate() {
        if global.is_extern {
            continue;
        }
        if !global.is_static || global.is_common || global.is_used {
            mark_reachable(n_funcs + j, &mut reachable, &mut worklist);
        }
    }

    for (alias_name, target, _) in &module.aliases {
        mark_named(alias_name.as_ref(), name_to_ids, &mut reachable, &mut worklist);
        mark_named(target.as_ref(), name_to_ids, &mut reachable, &mut worklist);
    }

    for ctor in &module.constructors {
        mark_named(ctor.as_ref(), name_to_ids, &mut reachable, &mut worklist);
    }
    for dtor in &module.destructors {
        mark_named(dtor.as_ref(), name_to_ids, &mut reachable, &mut worklist);
    }

    // Conservatively keep address-taken static always_inline functions even
    // when the taking site itself is dead. Their bodies may still be emitted
    // as function-pointer identities, so everything they reference must live.
    // (A live GlobalAddr/InlineAsm already creates a graph edge; this covers
    // the residual case and keeps dependent symbols consistent with Phase 5.)
    for (i, func) in module.functions.iter().enumerate() {
        if func.is_declaration {
            continue;
        }
        if func.is_static && func.is_always_inline && is_marked(address_taken, i) {
            mark_reachable(i, &mut reachable, &mut worklist);
        }
    }

    if !module.toplevel_asm.is_empty() {
        for (i, func) in module.functions.iter().enumerate() {
            if func.is_static
                && !func.is_declaration
                && !is_marked(&reachable, i)
                && toplevel_asm_mentions(&module.toplevel_asm, func.name.as_str())
            {
                mark_reachable(i, &mut reachable, &mut worklist);
            }
        }
        for (j, global) in module.globals.iter().enumerate() {
            let id = n_funcs + j;
            if global.is_static
                && !global.is_extern
                && !is_marked(&reachable, id)
                && toplevel_asm_mentions(&module.toplevel_asm, global.name.as_str())
            {
                mark_reachable(id, &mut reachable, &mut worklist);
            }
        }
    }

    while let Some(sid) = worklist.pop() {
        if sid < n_funcs {
            if let Some(refs) = func_refs.get(sid) {
                for &ref_id in refs {
                    mark_reachable(ref_id, &mut reachable, &mut worklist);
                }
            }
        } else if let Some(refs) = global_refs.get(sid - n_funcs) {
            for &ref_id in refs {
                mark_reachable(ref_id, &mut reachable, &mut worklist);
            }
        }
    }

    reachable
}

/// Phase 5: drop unreachable static definitions.
///
/// Declarations, non-static symbols, externs and common globals are retained
/// unconditionally (they either generate no code or have linkage obligations
/// outside this TU).
fn remove_unreachable(
    module: &mut IrModule,
    n_funcs: usize,
    reachable: &[bool],
    address_taken: &[bool],
) {
    let mut func_pos = 0usize;
    module.functions.retain(|func| {
        let pos = func_pos;
        func_pos += 1;
        if func.is_declaration {
            return true;
        }
        if func.is_static && func.is_always_inline {
            return is_marked(address_taken, pos) || is_marked(reachable, pos);
        }
        if !func.is_static {
            return true;
        }
        is_marked(reachable, pos)
    });

    let mut global_pos = 0usize;
    module.globals.retain(|global| {
        let pos = global_pos;
        global_pos += 1;
        if global.is_extern || !global.is_static || global.is_common {
            return true;
        }
        is_marked(reachable, n_funcs + pos)
    });
}

/// Phase 6: keep `symbol_attrs` only for names that still exist or are still
/// referenced. Visibility directives for deleted symbols become assembler /
/// linker errors (`.hidden foo` with no `foo`).
///
/// Weak-only directives (no visibility) are kept: they may apply to symbols
/// defined in another TU and must remain weak-undefined rather than strong.
fn filter_symbol_attrs(module: &mut IrModule) {
    // `GlobalInit::for_each_ref` only yields a callback-scoped `&str`, so
    // initializer names that we want in the long-lived set have to be owned.
    // Instruction / symbol names can be borrowed directly from the module.
    let mut init_names: Vec<String> = Vec::new();
    for global in &module.globals {
        global.init.for_each_ref(&mut |name| {
            init_names.push(String::from(name));
        });
    }

    let mut referenced: FxHashSet<&str> = FxHashSet::default();
    for n in &init_names {
        referenced.insert(n.as_str());
    }

    for func in &module.functions {
        referenced.insert(func.name.as_str());
        if func.is_declaration {
            continue;
        }
        for block in &func.blocks {
            for inst in &block.instructions {
                for_each_instruction_symbol(inst, |name, _takes_address| {
                    referenced.insert(name);
                });
            }
        }
    }
    for global in &module.globals {
        referenced.insert(global.name.as_str());
    }
    for (alias_name, target, _) in &module.aliases {
        referenced.insert(alias_name.as_ref());
        referenced.insert(target.as_ref());
    }
    for ctor in &module.constructors {
        referenced.insert(ctor.as_ref());
    }
    for dtor in &module.destructors {
        referenced.insert(dtor.as_ref());
    }

    let has_toplevel_asm = !module.toplevel_asm.is_empty();

    module.symbol_attrs.retain(|(name, is_weak, visibility)| {
        if *is_weak && visibility.is_none() {
            return true;
        }
        let n = name.as_str();
        if referenced.contains(n) {
            return true;
        }
        has_toplevel_asm && toplevel_asm_mentions(&module.toplevel_asm, n)
    });
}
