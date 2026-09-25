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
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{Instruction, IrModule, Operand, Value};

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

/// Delete stores to never-loaded static globals (Phase 11a: runs before
/// [`eliminate_dead_static_functions`], so a global whose stores all die is
/// itself removed as unreferenced).
///
/// After GVN's store-to-load forwarding, a `static` output buffer that the TU
/// never reads keeps only its stores (`out[i] = ...` in the corpus
/// `round_family_pass`: the same-iteration reload forwarded into registers,
/// `main` never touches `out`).  The stores are unobservable — no loads exist
/// anywhere in the TU — yet the backend faithfully emits them plus their
/// address math.  Every oracle deletes them (whole-program DSE); this pass is
/// lccc's TU-closed equivalent.
///
/// A global's stores are deleted only when ALL of the following hold
/// (anything unexpected bails on that global — fail-closed):
///
/// * **TU-local linkage**: `static`, not `extern`, not `common`, not `weak`
///   (a weak definition may be preempted/read cross-TU), no
///   `__attribute__((used))` (explicitly pinned), no custom `section`
///   (linker-script-observable, e.g. kernel section collectors).
///   A `const`-qualified global IS eligible: storing through it is C11
///   6.7.3p6 UB, and with no loads the UB store is unobservable either way.
/// * **No name-level escape**: the name appears in no toplevel-asm blob
///   ([`toplevel_asm_mentions`]), no global initializer (an init naming it
///   publishes the address), no inline-asm `input_symbols`, no
///   `__attribute__((alias))` alias target/name (the alias publishes the
///   address cross-TU), and no inline-asm template string (a store whose only
///   reader is `asm("movl out(%%rip), %0" : "=r"(v))` must be kept).
/// * **No value-level escape or read**: in every function, each `GlobalAddr`
///   naming the global seeds an address set propagated only through
///   `GetElementPtr` bases, `Copy`, bitwise-identical `Cast`s (pure renames:
///   [`IrType::cast_is_bitidentical_nop`] is the single source of truth; a
///   converting cast bails), and `Phi` (optimistically — see below).
///   Single-source chains — each derived pointer has exactly one address
///   lineage — so deleted-store sets for distinct globals are disjoint by
///   construction.  Every use of every set member must be a non-volatile
///   default-address-space `Store` THROUGH the address (recorded for
///   deletion).  A `Load` through one (a reader), the address as a stored
///   VALUE, a call argument, a cmp/binop operand, a memcpy endpoint, an asm
///   operand, or any terminator use bails the global.
///   In particular a self-referential global (`q = load out; store x -> q`)
///   bails: the load itself is a reader.  Pure calls are NOT exempted
///   (purity permits reads; the address provably never reaches them, but
///   checking that per-arg is not worth the audit surface — bail).
/// * **All definitions derive**: after the fixpoint EVERY definition of
///   EVERY set member must be a deriving form — `GlobalAddr` of this global,
///   in-set `GEP` base with non-address offset, in-set `Copy`/`Cast` source,
///   or a `Phi` whose EVERY arm is in the set.  This is what makes the
///   optimistic phi rule sound (a phi merging a foreign value fails here and
///   bails the global, recorded stores discarded), and it also covers
///   redefinition, where the IR permits it (a loop-latch `Copy` from any
///   other value bails — the use classification alone cannot see which
///   reaching def a use reads).
/// * **At least one store** is recorded (an address-taken-but-unused global
///   is DCE's job, and deleting zero stores must not count as progress).
///
/// Deletion mirrors DCE's sweep (instructions + 1:1 source spans compacted
/// together, stale spans cleared), then a DCE chaser runs on every touched
/// function so the orphaned stored-value and address chains (pure) disappear
/// in the same pass — the pipeline runs no DCE after Phase 11.
pub(crate) fn eliminate_dead_global_stores(module: &mut IrModule) {
    // Per-function use maps, built once and reused for every candidate
    // (collection is read-only; all deletions happen after).  Terminator
    // uses record `usize::MAX` so they always bail.
    struct FuncUses {
        use_locs: Vec<Vec<(usize, usize)>>,
        def_locs: Vec<Vec<(usize, usize)>>,
    }
    let mut func_uses: Vec<FuncUses> = Vec::with_capacity(module.functions.len());
    for func in &module.functions {
        let nvals = func.max_value_id() as usize + 1;
        let mut use_locs: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nvals];
        let mut def_locs: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nvals];
        if !func.is_declaration {
            for (bi, block) in func.blocks.iter().enumerate() {
                for (ii, inst) in block.instructions.iter().enumerate() {
                    if let Some(dest) = inst.dest() {
                        if let Some(slot) = def_locs.get_mut(dest.0 as usize) {
                            slot.push((bi, ii));
                        }
                    }
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            if let Some(slot) = use_locs.get_mut(v.0 as usize) {
                                slot.push((bi, ii));
                            }
                        }
                    });
                    crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                        if let Some(slot) = use_locs.get_mut(v.0 as usize) {
                            slot.push((bi, ii));
                        }
                    });
                }
                crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
                    if let Operand::Value(v) = op {
                        if let Some(slot) = use_locs.get_mut(v.0 as usize) {
                            slot.push((bi, usize::MAX));
                        }
                    }
                });
            }
        }
        func_uses.push(FuncUses { use_locs, def_locs });
    }

    // Global initializers naming a symbol publish its address: collect the
    // named set once (any candidate in it bails).
    let mut init_named: FxHashSet<String> = FxHashSet::default();
    for global in &module.globals {
        global.init.for_each_ref(&mut |name| {
            init_named.insert(String::from(name));
        });
    }
    // Inline-asm symbol operands publish addresses too.
    let mut asm_named: FxHashSet<String> = FxHashSet::default();
    // Inline-asm template strings may mention globals textually (e.g.
    // `asm("movl g(%%rip), %0" : "=r"(v))`). Collect them once.
    let mut inline_asm_templates: Vec<String> = Vec::new();
    for func in &module.functions {
        if func.is_declaration {
            continue;
        }
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::InlineAsm {
                    input_symbols,
                    template,
                    ..
                } = inst
                {
                    for s in input_symbols.iter().flatten() {
                        asm_named.insert(String::from(asm_symbol_base(s)));
                    }
                    if !template.is_empty() {
                        inline_asm_templates.push(template.clone());
                    }
                }
            }
        }
    }
    // Aliases publish addresses cross-TU: `static int out[4]; extern int
    // pub_out[4] __attribute__((alias("out")));` — other TUs read `out`
    // through `pub_out`, so stores to `out` must be kept.
    let mut alias_named: FxHashSet<String> = FxHashSet::default();
    for (alias_name, target, _) in &module.aliases {
        alias_named.insert(alias_name.to_string());
        alias_named.insert(target.to_string());
    }

    // F6: collect GlobalAddr definitions once for all functions — O(F*I)
    // instead of O(G*F*I). Map from global name to list of (func_idx, value).
    let mut global_addr_defs: FxHashMap<String, Vec<(usize, u32)>> = FxHashMap::default();
    for (fi, func) in module.functions.iter().enumerate() {
        if func.is_declaration {
            continue;
        }
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::GlobalAddr { dest, name } = inst {
                    global_addr_defs
                        .entry(name.clone())
                        .or_default()
                        .push((fi, dest.0));
                }
            }
        }
    }

    // (function, block, instruction) of every dead store, across globals
    // (disjoint by the single-lineage argument above).
    let mut dead: Vec<(usize, usize, usize)> = Vec::new();
    let mut touched_funcs: Vec<bool> = vec![false; module.functions.len()];

    for global in &module.globals {
        if global.is_extern
            || !global.is_static
            || global.is_common
            || global.is_weak
            || global.is_used
            || global.section.is_some()
        {
            continue;
        }
        if !module.toplevel_asm.is_empty()
            && toplevel_asm_mentions(&module.toplevel_asm, global.name.as_str())
        {
            continue;
        }
        if init_named.contains(&global.name)
            || asm_named.contains(&global.name)
            || alias_named.contains(&global.name)
        {
            continue;
        }
        if !inline_asm_templates.is_empty()
            && inline_asm_templates
                .iter()
                .any(|t| asm_mentions_symbol(t, global.name.as_str()))
        {
            continue;
        }
        // Flow walk: seed from every GlobalAddr naming this global (via
        // pre-collected map, not a per-global full scan).
        let Some(all_seeds) = global_addr_defs.get(&global.name) else {
            continue;
        };
        // Group seeds by function for locality.
        let mut seeds_by_func: FxHashMap<usize, Vec<u32>> = FxHashMap::default();
        for (fi, dest) in all_seeds {
            seeds_by_func.entry(*fi).or_default().push(*dest);
        }

        let mut stores: Vec<(usize, usize, usize)> = Vec::new();
        let mut bailed = false;
        // Only iterate functions that actually reference this global.
        for (fi, seeds) in seeds_by_func {
            let func = &module.functions[fi];
            // Per-function address set, seeded from this global's GlobalAddrs
            // in this function.
            let mut addr: FxHashSet<u32> = FxHashSet::default();
            let mut worklist: Vec<u32> = Vec::new();
            for d in seeds {
                if addr.insert(d) {
                    worklist.push(d);
                }
            }
            if worklist.is_empty() {
                continue;
            }
            let locs = &func_uses[fi].use_locs;
            while let Some(v) = worklist.pop() {
                let Some(uses) = locs.get(v as usize) else {
                    continue;
                };
                // No clone: `uses` is a &[ (bi,ii) ] borrowed from func_uses,
                // and `func` is borrowed immutably from module — no aliasing
                // conflict, so we can iterate the slice directly.
                for &(bi, ii) in uses.iter() {
                    if ii == usize::MAX {
                        bailed = true;
                        break;
                    }
                    let Some(inst) = func.blocks.get(bi).and_then(|b| b.instructions.get(ii))
                    else {
                        bailed = true;
                        break;
                    };
                    match inst {
                        Instruction::GetElementPtr {
                            base, offset, dest, ..
                        } => {
                            if *base != Value(v) {
                                // Used as the offset (or inconsistency):
                                // fail closed.
                                bailed = true;
                                break;
                            }
                            if let Operand::Value(w) = offset {
                                if addr.contains(&w.0) {
                                    bailed = true;
                                    break;
                                }
                            }
                            if addr.insert(dest.0) {
                                worklist.push(dest.0);
                            }
                        }
                        Instruction::Phi { dest, incoming, .. } => {
                            if !incoming
                                .iter()
                                .any(|(op, _)| *op == Operand::Value(Value(v)))
                            {
                                bailed = true;
                                break;
                            }
                            if addr.insert(dest.0) {
                                worklist.push(dest.0);
                            }
                        }
                        Instruction::Copy { dest, src } => {
                            if *src != Operand::Value(Value(v)) {
                                bailed = true;
                                break;
                            }
                            if addr.insert(dest.0) {
                                worklist.push(dest.0);
                            }
                        }
                        Instruction::Cast {
                            dest,
                            src,
                            from_ty,
                            to_ty,
                            ..
                        } => {
                            if *src != Operand::Value(Value(v))
                                || !IrType::cast_is_bitidentical_nop(*from_ty, *to_ty)
                            {
                                bailed = true;
                                break;
                            }
                            if addr.insert(dest.0) {
                                worklist.push(dest.0);
                            }
                        }
                        Instruction::Store {
                            val,
                            ptr,
                            seg_override,
                            volatile,
                            ..
                        } => {
                            if *val == Operand::Value(Value(v)) {
                                bailed = true;
                                break;
                            }
                            if *ptr != Value(v) {
                                bailed = true;
                                break;
                            }
                            if *volatile || *seg_override != AddressSpace::Default {
                                bailed = true;
                                break;
                            }
                            stores.push((fi, bi, ii));
                        }
                        _ => {
                            bailed = true;
                            break;
                        }
                    }
                }
                if bailed {
                    break;
                }
            }
            if !bailed {
                let defs = &func_uses[fi].def_locs;
                'verify: for v in addr.iter() {
                    let Some(dlocs) = defs.get(*v as usize) else {
                        continue;
                    };
                    for (bi, ii) in dlocs {
                        let Some(def) = func.blocks.get(*bi).and_then(|b| b.instructions.get(*ii))
                        else {
                            bailed = true;
                            break 'verify;
                        };
                        let ok = match def {
                            Instruction::GlobalAddr { name, .. } => *name == global.name,
                            Instruction::Phi { incoming, .. } => incoming.iter().all(
                                |(op, _)| matches!(op, Operand::Value(w) if addr.contains(&w.0)),
                            ),
                            Instruction::GetElementPtr { base, offset, .. } => {
                                addr.contains(&base.0)
                                    && !matches!(offset, Operand::Value(w) if addr.contains(&w.0))
                            }
                            Instruction::Copy { src, .. } => {
                                matches!(src, Operand::Value(w) if addr.contains(&w.0))
                            }
                            Instruction::Cast {
                                src,
                                from_ty,
                                to_ty,
                                ..
                            } => {
                                matches!(src, Operand::Value(w) if addr.contains(&w.0))
                                    && IrType::cast_is_bitidentical_nop(*from_ty, *to_ty)
                            }
                            _ => false,
                        };
                        if !ok {
                            bailed = true;
                            break 'verify;
                        }
                    }
                }
            }
            if bailed {
                break;
            }
        }
        if bailed || stores.is_empty() {
            continue;
        }
        for (fi, _, _) in &stores {
            touched_funcs[*fi] = true;
        }
        dead.extend(stores);
    }

    if dead.is_empty() {
        return;
    }
    dead.sort();
    let mut idx = 0;
    while idx < dead.len() {
        let (fi, bi, _) = dead[idx];
        let mut in_block: Vec<usize> = Vec::new();
        while idx < dead.len() && dead[idx].0 == fi && dead[idx].1 == bi {
            in_block.push(dead[idx].2);
            idx += 1;
        }
        in_block.sort();
        let block = &mut module.functions[fi].blocks[bi];
        let original_len = block.instructions.len();
        let has_spans = !block.source_spans.is_empty() && block.source_spans.len() == original_len;
        let mut kept_inst = Vec::with_capacity(original_len - in_block.len());
        let mut di = 0usize;
        for (i, inst) in block.instructions.drain(..).enumerate() {
            if di < in_block.len() && in_block[di] == i {
                di += 1;
            } else {
                kept_inst.push(inst);
            }
        }
        block.instructions = kept_inst;
        if has_spans {
            let mut kept_spans = Vec::with_capacity(original_len - in_block.len());
            let mut di = 0usize;
            for (i, span) in block.source_spans.drain(..).enumerate() {
                if di < in_block.len() && in_block[di] == i {
                    di += 1;
                } else {
                    kept_spans.push(span);
                }
            }
            block.source_spans = kept_spans;
        } else if !block.source_spans.is_empty() && block.source_spans.len() != original_len {
            block.source_spans.clear();
        }
        debug_assert_eq!(block.instructions.len(), original_len - in_block.len());
    }
    for (fi, touched) in touched_funcs.iter().enumerate() {
        if *touched {
            crate::passes::dce::eliminate_dead_code(&mut module.functions[fi]);
        }
    }
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
    if start < end { &s[start..end] } else { s }
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

fn mark_address_taken(
    name: &str,
    name_to_ids: &FxHashMap<&str, Vec<usize>>,
    address_taken: &mut [bool],
) {
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
        mark_named(
            alias_name.as_ref(),
            name_to_ids,
            &mut reachable,
            &mut worklist,
        );
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

    // F5: inline-asm template strings may mention static symbols textually
    // (e.g. `asm("movl g(%%rip), %0" : "=r"(v))`). Treat any such mention
    // as a root, just like toplevel-asm. This closes the same gap in both
    // dead-static passes; the old pass had the same blind spot.
    {
        let mut inline_templates: Vec<&str> = Vec::new();
        for func in &module.functions {
            if func.is_declaration {
                continue;
            }
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::InlineAsm { template, .. } = inst {
                        if !template.is_empty() {
                            inline_templates.push(template.as_str());
                        }
                    }
                }
            }
        }
        if !inline_templates.is_empty() {
            for (i, func) in module.functions.iter().enumerate() {
                if func.is_static
                    && !func.is_declaration
                    && !is_marked(&reachable, i)
                    && inline_templates
                        .iter()
                        .any(|t| asm_mentions_symbol(t, func.name.as_str()))
                {
                    mark_reachable(i, &mut reachable, &mut worklist);
                }
            }
            for (j, global) in module.globals.iter().enumerate() {
                let id = n_funcs + j;
                if global.is_static
                    && !global.is_extern
                    && !is_marked(&reachable, id)
                    && inline_templates
                        .iter()
                        .any(|t| asm_mentions_symbol(t, global.name.as_str()))
                {
                    mark_reachable(id, &mut reachable, &mut worklist);
                }
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
    // Collect inline-asm templates for symbol-attrs filtering too: a `.hidden`
    // for a symbol only mentioned in an inline-asm template must be kept.
    let inline_templates: Vec<String> = module
        .functions
        .iter()
        .filter(|f| !f.is_declaration)
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.instructions.iter())
        .filter_map(|inst| {
            if let Instruction::InlineAsm { template, .. } = inst {
                if !template.is_empty() {
                    Some(template.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    module.symbol_attrs.retain(|(name, is_weak, visibility)| {
        if *is_weak && visibility.is_none() {
            return true;
        }
        let n = name.as_str();
        if referenced.contains(n) {
            return true;
        }
        if has_toplevel_asm && toplevel_asm_mentions(&module.toplevel_asm, n) {
            return true;
        }
        !inline_templates.is_empty() && inline_templates.iter().any(|t| asm_mentions_symbol(t, n))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{
        BasicBlock, BlockId, CallInfo, GlobalInit, IrConst, IrFunction, IrGlobal, Terminator,
    };

    fn gs_global(name: &str) -> IrGlobal {
        IrGlobal {
            name: String::from(name),
            ty: IrType::F64,
            size: 8,
            align: 8,
            init: GlobalInit::Zero,
            is_static: true,
            is_extern: false,
            is_common: false,
            section: None,
            is_weak: false,
            visibility: None,
            has_explicit_align: false,
            is_const: false,
            is_used: false,
            is_thread_local: false,
        }
    }

    fn gs_block(insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
        }
    }

    fn gs_writer(
        name: &str,
        extra: Vec<Instruction>,
        term: Terminator,
        next_id: u32,
    ) -> IrFunction {
        // GlobalAddr 0 -> GEP 1 -> store; `extra` spliced before the return.
        let mut insts = vec![
            Instruction::GlobalAddr {
                dest: Value(0),
                name: String::from("out"),
            },
            Instruction::GetElementPtr {
                dest: Value(1),
                base: Value(0),
                offset: Operand::Const(IrConst::I64(0)),
                ty: IrType::F64,
            },
            Instruction::Store {
                val: Operand::Const(IrConst::F64(1.5)),
                ptr: Value(1),
                ty: IrType::F64,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ];
        insts.extend(extra);
        let mut f = IrFunction::new(String::from(name), IrType::Void, vec![], false);
        f.blocks = vec![gs_block(insts, term)];
        f.next_value_id = next_id;
        f
    }

    fn store_count(module: &IrModule) -> usize {
        module
            .functions
            .iter()
            .flat_map(|f| f.blocks.iter())
            .flat_map(|b| b.instructions.iter())
            .filter(|i| matches!(i, Instruction::Store { .. }))
            .count()
    }

    #[test]
    fn dead_global_store_deleted_and_global_removed() {
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        m.functions
            .push(gs_writer("w", vec![], Terminator::Return(None), 2));
        assert_eq!(store_count(&m), 1);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 0, "the dead store is gone");
        // The DCE chaser retired the orphaned address chain too.
        assert!(
            m.functions[0].blocks[0].instructions.is_empty(),
            "GlobalAddr + GEP retired by the chaser"
        );
        // Phase 11 then removes the unreferenced global (composition).
        eliminate_dead_static_functions(&mut m);
        assert!(m.globals.is_empty(), "out removed as unreferenced");
    }

    #[test]
    fn loaded_global_kept() {
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        // A reader: the store is observable, everything stays.
        let mut f = gs_writer(
            "w",
            vec![Instruction::Load {
                dest: Value(2),
                ptr: Value(1),
                ty: IrType::F64,
                seg_override: AddressSpace::Default,
                volatile: false,
            }],
            Terminator::Return(None),
            3,
        );
        // The load is dead (unused) but still a reader for this pass: DCE
        // owns dead-load removal, and this pass must not race it.  (In the
        // real pipeline DCE runs before Phase 11a and removes dead loads,
        // exposing the stores; here the load is present, so bail.)
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 1, "reader present: store kept");
        // And with the load gone (as post-DCE IR), the store deletes.
        m.functions[0].blocks[0].instructions.pop();
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 0, "post-DCE shape: store gone");
    }

    #[test]
    fn address_escape_kept() {
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        m.globals.push(gs_global("sink"));
        // The address is stored into another global: published, bail.
        let mut f = gs_writer("w", vec![], Terminator::Return(None), 4);
        f.blocks[0].instructions.extend(vec![
            Instruction::GlobalAddr {
                dest: Value(2),
                name: String::from("sink"),
            },
            Instruction::Store {
                val: Operand::Value(Value(0)),
                ptr: Value(2),
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ]);
        f.next_value_id = 3;
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        // `out`'s store is kept (its address was published), but `sink` is
        // itself never loaded, so the publishing store is dead and goes.
        assert_eq!(store_count(&m), 1, "escaped address: out's store kept");
        assert!(
            matches!(
                m.functions[0].blocks[0].instructions[2],
                Instruction::Store { ptr: Value(1), .. }
            ),
            "the survivor is out's store"
        );
        // Single-shot by design, but convergent: with the publication gone,
        // a second run deletes out's store too.
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 0, "second run: out's store gone");
    }

    #[test]
    fn call_argument_escape_kept() {
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        let f = gs_writer(
            "w",
            vec![Instruction::Call {
                func: String::from("f"),
                info: CallInfo {
                    args: vec![Operand::Value(Value(0))],
                    ..Default::default()
                },
            }],
            Terminator::Return(None),
            2,
        );
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 1, "call arg escape: kept");
    }

    #[test]
    fn extern_and_used_and_section_globals_kept() {
        for mut g in [gs_global("a"), gs_global("b"), gs_global("c")] {
            let mut m = IrModule::new();
            if g.name == "a" {
                g.is_static = false;
            } else if g.name == "b" {
                g.is_used = true;
            } else {
                g.section = Some(String::from(".data.keep"));
            }
            m.globals.push(g);
            let name = m.globals[0].name.clone();
            let mut f = gs_writer("w", vec![], Terminator::Return(None), 2);
            if let Instruction::GlobalAddr { name: n, .. } = &mut f.blocks[0].instructions[0] {
                *n = name;
            }
            m.functions.push(f);
            eliminate_dead_global_stores(&mut m);
            assert_eq!(store_count(&m), 1, "extern/used/section: kept");
        }
    }

    #[test]
    fn multi_function_stores_all_deleted() {
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        m.functions
            .push(gs_writer("w1", vec![], Terminator::Return(None), 2));
        m.functions
            .push(gs_writer("w2", vec![], Terminator::Return(None), 2));
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 0, "TU-wide: both stores gone");
    }

    #[test]
    fn initializer_reference_kept() {
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        // Another global's initializer names `out`: published, bail.
        let mut sink = gs_global("sink");
        sink.init = GlobalInit::GlobalAddr(String::from("out"));
        m.globals.push(sink);
        m.functions
            .push(gs_writer("w", vec![], Terminator::Return(None), 2));
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 1, "init-published: kept");
    }

    #[test]
    fn copy_chain_fires() {
        // GlobalAddr -> Copy -> store (the round_family_pass preheader
        // shape): copies are pure renames, the store still dies.
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        let mut f = IrFunction::new(String::from("w"), IrType::Void, vec![], false);
        f.blocks = vec![gs_block(
            vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: String::from("out"),
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::F64(1.5)),
                    ptr: Value(1),
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            Terminator::Return(None),
        )];
        f.next_value_id = 2;
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 0, "copy-rename chain: store gone");
    }

    #[test]
    fn latch_cycle_fires() {
        // Loop-latch redefinition from a derived pointer (124 = Copy(125),
        // 125 = GEP(124)): both defs derive, the fixpoint holds, stores die.
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        let mut f = IrFunction::new(String::from("w"), IrType::Void, vec![], false);
        f.blocks = vec![gs_block(
            vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: String::from("out"),
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::F64(1.5)),
                    ptr: Value(1),
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::GetElementPtr {
                    dest: Value(2),
                    base: Value(1),
                    offset: Operand::Const(IrConst::I64(16)),
                    ty: IrType::I8,
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                },
            ],
            Terminator::Return(None),
        )];
        f.next_value_id = 3;
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 0, "latch cycle: store gone");
    }

    #[test]
    fn hostile_redefinition_bails() {
        // Same shape, but the latch copy takes another global's address:
        // the use classification sees store-throughs only, so the
        // all-definitions verification must catch it.
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        m.globals.push(gs_global("buf"));
        let mut f = IrFunction::new(String::from("w"), IrType::Void, vec![], false);
        f.blocks = vec![gs_block(
            vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: String::from("out"),
                },
                Instruction::GlobalAddr {
                    dest: Value(3),
                    name: String::from("buf"),
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::F64(1.5)),
                    ptr: Value(1),
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(3)),
                },
            ],
            Terminator::Return(None),
        )];
        f.next_value_id = 4;
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 1, "hostile redefinition: kept");
    }

    #[test]
    fn phi_cycle_fires() {
        // `p = Phi[out, bump]; bump = GEP(p)` (the round_family_pass loop
        // header at Phase 11a, SSA form): the optimistic phi rule
        // bootstraps the cycle and verification confirms both arms.
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        let mut f = IrFunction::new(String::from("w"), IrType::Void, vec![], false);
        f.blocks = vec![gs_block(
            vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: String::from("out"),
                },
                Instruction::Phi {
                    dest: Value(1),
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(2)), BlockId(0)),
                    ],
                    ty: IrType::Ptr,
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::F64(1.5)),
                    ptr: Value(1),
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::GetElementPtr {
                    dest: Value(2),
                    base: Value(1),
                    offset: Operand::Const(IrConst::I64(16)),
                    ty: IrType::I8,
                },
            ],
            Terminator::Return(None),
        )];
        f.next_value_id = 3;
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 0, "phi cycle: store gone");
    }

    #[test]
    fn hostile_phi_bails() {
        // `p = Phi[out, foreign]` where the foreign arm never derives:
        // verification rejects the phi and the store survives.
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        m.globals.push(gs_global("buf"));
        let mut f = IrFunction::new(String::from("w"), IrType::Void, vec![], false);
        f.blocks = vec![gs_block(
            vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: String::from("out"),
                },
                Instruction::GlobalAddr {
                    dest: Value(3),
                    name: String::from("buf"),
                },
                Instruction::Phi {
                    dest: Value(1),
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(3)), BlockId(0)),
                    ],
                    ty: IrType::Ptr,
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::F64(1.5)),
                    ptr: Value(1),
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            Terminator::Return(None),
        )];
        f.next_value_id = 4;
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 1, "hostile phi: kept");
    }

    #[test]
    fn converting_cast_bails() {
        // Address -> F64 is a value convert, not a rename: the lineage ends.
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        let mut f = IrFunction::new(String::from("w"), IrType::Void, vec![], false);
        f.blocks = vec![gs_block(
            vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: String::from("out"),
                },
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::U64,
                    to_ty: IrType::F64,
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::F64(1.5)),
                    ptr: Value(1),
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            Terminator::Return(None),
        )];
        f.next_value_id = 2;
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 1, "converting cast: kept");
    }

    #[test]
    fn volatile_store_bails() {
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        let mut f = gs_writer("w", vec![], Terminator::Return(None), 2);
        if let Instruction::Store { volatile, .. } = &mut f.blocks[0].instructions[2] {
            *volatile = true;
        }
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 1, "volatile store: kept");
    }

    #[test]
    fn alias_target_bails() {
        // `static int out[4]; extern int pub_out[4] __attribute__((alias("out")));`
        // Other TUs read `out` through `pub_out`, so stores must be kept.
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        m.functions
            .push(gs_writer("w", vec![], Terminator::Return(None), 2));
        m.aliases
            .push((String::from("pub_out"), String::from("out"), false));
        assert_eq!(store_count(&m), 1);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 1, "alias target: store kept");
    }

    #[test]
    fn alias_name_bails() {
        let mut m = IrModule::new();
        let g = gs_global("pub_out");
        m.globals.push(g);
        let mut f = IrFunction::new(String::from("w"), IrType::Void, vec![], false);
        f.blocks = vec![gs_block(
            vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: String::from("pub_out"),
                },
                Instruction::GetElementPtr {
                    dest: Value(1),
                    base: Value(0),
                    offset: Operand::Const(IrConst::I64(0)),
                    ty: IrType::F64,
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::F64(1.5)),
                    ptr: Value(1),
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            Terminator::Return(None),
        )];
        f.next_value_id = 2;
        m.functions.push(f);
        m.aliases
            .push((String::from("pub_out"), String::from("out"), false));
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 1, "alias name: store kept");
    }

    #[test]
    fn inline_asm_template_bails() {
        let mut m = IrModule::new();
        m.globals.push(gs_global("g"));
        let mut f = IrFunction::new(String::from("w"), IrType::Void, vec![], false);
        f.blocks = vec![gs_block(
            vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: String::from("g"),
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::I32(5)),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::InlineAsm {
                    template: String::from("movl g(%%rip), %0"),
                    outputs: vec![("=r".to_string(), Value(1), None)],
                    inputs: vec![],
                    clobbers: vec![],
                    operand_types: vec![IrType::I32],
                    goto_labels: vec![],
                    input_symbols: vec![],
                    seg_overrides: vec![AddressSpace::Default],
                },
            ],
            Terminator::Return(None),
        )];
        f.next_value_id = 2;
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(
            store_count(&m),
            1,
            "inline-asm template mention: store kept"
        );
    }

    #[test]
    fn inline_asm_template_unrelated_ok() {
        let mut m = IrModule::new();
        m.globals.push(gs_global("out"));
        let mut f = gs_writer("w", vec![], Terminator::Return(None), 2);
        f.blocks[0].instructions.push(Instruction::InlineAsm {
            template: String::from("movl other(%%rip), %0"),
            outputs: vec![("=r".to_string(), Value(2), None)],
            inputs: vec![],
            clobbers: vec![],
            operand_types: vec![IrType::I32],
            goto_labels: vec![],
            input_symbols: vec![],
            seg_overrides: vec![AddressSpace::Default],
        });
        f.next_value_id = 3;
        m.functions.push(f);
        eliminate_dead_global_stores(&mut m);
        assert_eq!(store_count(&m), 0, "unrelated inline-asm: store gone");
    }
}
