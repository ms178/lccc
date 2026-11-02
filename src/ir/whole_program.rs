//! Whole-program multi-TU IR merging and symbol internalization (`-fwhole-program`
//! + `__attribute__((externally_visible))`, `MB-15`).
//!
//! Supports both:
//! 1. Single-TU `-fwhole-program` (`-c`, `-S`, or single-input full link), matching
//!    GCC 16.2's `whole_program` IPA visibility promotion.
//! 2. Multi-TU whole-program compilation (`lccc -fwhole-program -O2 a.c b.c -o app`),
//!    merging all C translation units into one unified `IrModule` before running
//!    the middle-end optimization pipeline and x86-64 `IPA-RA`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::instruction::{BasicBlock, BlockId, Instruction, Terminator};
use crate::ir::module::{GlobalInit, IrFunction, IrGlobal, IrModule};

/// Merge one or more per-TU `IrModule`s into a single unified `IrModule`.
pub fn merge_modules(mut modules: Vec<IrModule>) -> Result<IrModule, String> {
    if modules.len() <= 1 {
        return Ok(modules.pop().unwrap_or_default());
    }

    // Step 1: Count symbol names across all TUs so we rename `is_static` symbols
    // whenever another TU defines or declares the same identifier (plus all
    // `.LC*` string literal labels in TUs > 0).
    let mut name_tu_counts: FxHashMap<String, usize> = FxHashMap::default();
    for m in &modules {
        let mut seen_in_tu: FxHashSet<&str> = FxHashSet::default();
        for f in &m.functions {
            if seen_in_tu.insert(f.name.as_str()) {
                *name_tu_counts.entry(f.name.clone()).or_insert(0) += 1;
            }
        }
        for g in &m.globals {
            if seen_in_tu.insert(g.name.as_str()) {
                *name_tu_counts.entry(g.name.clone()).or_insert(0) += 1;
            }
        }
    }

    // Step 2: Renumber BlockIds globally across all TUs and apply per-TU static
    // + string-literal alpha-renaming.
    let mut next_global_block_id: u32 = 0;
    for (tu_idx, m) in modules.iter_mut().enumerate() {
        let block_offset = next_global_block_id;
        let max_in_tu = renumber_module_blocks(m, block_offset);
        next_global_block_id = max_in_tu;

        let rename_map = build_tu_rename_map(m, tu_idx, &name_tu_counts);
        if !rename_map.is_empty() {
            apply_tu_renames(m, &rename_map);
        }
    }

    // Step 3: Merge all TUs into a unified module, resolving cross-TU linkage.
    let mut out = IrModule::new();
    let mut func_index: FxHashMap<String, usize> = FxHashMap::default();
    let mut global_index: FxHashMap<String, usize> = FxHashMap::default();

    for m in modules {
        merge_one_tu(&mut out, m, &mut func_index, &mut global_index)?;
    }

    // Prune `extern_function_symbols` for any function now defined in the merged module.
    let defined_fns: FxHashSet<String> = out
        .functions
        .iter()
        .filter(|f| !f.is_declaration)
        .map(|f| {
            out.asm_labels
                .get(&f.name)
                .cloned()
                .unwrap_or_else(|| f.name.clone())
        })
        .collect();
    out.extern_function_symbols
        .retain(|sym| !defined_fns.contains(sym));

    Ok(out)
}

/// Internalize non-`main`, non-`externally_visible` definitions under `-fwhole-program`.
pub fn internalize_module(module: &mut IrModule, export_dynamic: bool) {
    if export_dynamic {
        return;
    }

    let mut roots: FxHashSet<String> = FxHashSet::default();
    roots.insert("main".to_string());
    roots.insert("_start".to_string());
    for sym in &module.externally_visible_symbols {
        roots.insert(sym.clone());
    }
    for sym in &module.constructors {
        roots.insert(sym.clone());
    }
    for sym in &module.destructors {
        roots.insert(sym.clone());
    }
    for (alias, target, _) in &module.aliases {
        roots.insert(alias.clone());
        roots.insert(target.clone());
    }
    for (sym, resolver, _, _) in &module.ifuncs {
        roots.insert(sym.clone());
        roots.insert(resolver.clone());
    }
    for (fn_name, _) in &module.symver_directives {
        roots.insert(fn_name.clone());
    }

    // Preserve any symbol referenced via inline asm `"i"` input_symbols or top-level asm.
    for func in &module.functions {
        for b in &func.blocks {
            for inst in &b.instructions {
                if let Instruction::InlineAsm { input_symbols, .. } = inst {
                    for sym in input_symbols.iter().flatten() {
                        roots.insert(sym.clone());
                    }
                }
            }
        }
    }
    let has_toplevel_asm = !module.toplevel_asm.is_empty();

    for func in &mut module.functions {
        if func.is_declaration || func.is_gnu_inline_def || func.is_static || func.is_used {
            continue;
        }
        let asm_name = module
            .asm_labels
            .get(&func.name)
            .map(|s| s.as_str())
            .unwrap_or(func.name.as_str());
        if roots.contains(&func.name) || roots.contains(asm_name) {
            continue;
        }
        if has_toplevel_asm && mentioned_in_toplevel_asm(&module.toplevel_asm, asm_name) {
            continue;
        }
        func.is_static = true;
        func.is_weak = false;
        func.visibility = None;
    }

    for g in &mut module.globals {
        if g.is_extern || g.is_static || g.is_used {
            continue;
        }
        let asm_name = module
            .asm_labels
            .get(&g.name)
            .map(|s| s.as_str())
            .unwrap_or(g.name.as_str());
        if roots.contains(&g.name) || roots.contains(asm_name) {
            continue;
        }
        if has_toplevel_asm && mentioned_in_toplevel_asm(&module.toplevel_asm, asm_name) {
            continue;
        }
        g.is_static = true;
        g.is_weak = false;
        g.is_common = false;
        g.visibility = None;
    }
}

fn mentioned_in_toplevel_asm(chunks: &[String], sym: &str) -> bool {
    chunks.iter().any(|asm| asm.contains(sym))
}

/// Shift all `BlockId`s in `m` by `offset`, returning the next unused `BlockId`.
fn renumber_module_blocks(m: &mut IrModule, offset: u32) -> u32 {
    let mut max_seen = offset;
    for func in &mut m.functions {
        if func.is_declaration {
            continue;
        }
        if offset == 0 {
            for b in &func.blocks {
                max_seen = max_seen.max(b.label.0.saturating_add(1));
            }
            max_seen = max_seen.max(func.next_label);
            continue;
        }
        for b in &mut func.blocks {
            b.label = BlockId(b.label.0 + offset);
            max_seen = max_seen.max(b.label.0 + 1);
            shift_block_refs(b, offset);
        }
        func.terminator_spans.shift(offset);
        for bid in &mut func.global_init_label_blocks {
            bid.0 += offset;
            max_seen = max_seen.max(bid.0 + 1);
        }
        if func.next_label > 0 {
            func.next_label += offset;
            max_seen = max_seen.max(func.next_label);
        } else {
            func.next_label = max_seen;
        }
    }
    if offset > 0 {
        for g in &mut m.globals {
            shift_global_init_block_labels(&mut g.init, offset);
        }
    }
    max_seen
}

fn shift_block_refs(b: &mut BasicBlock, offset: u32) {
    for inst in &mut b.instructions {
        match inst {
            Instruction::Phi { incoming, .. } => {
                for (_, bid) in incoming {
                    bid.0 += offset;
                }
            }
            Instruction::LabelAddr { label, .. } => {
                label.0 += offset;
            }
            Instruction::InlineAsm { goto_labels, .. } => {
                for (_, bid) in goto_labels {
                    bid.0 += offset;
                }
            }
            Instruction::NonlocalGoto { label, .. } => {
                shift_lbb_string(label, offset);
            }
            _ => {}
        }
    }
    match &mut b.terminator {
        Terminator::Branch(bid) => bid.0 += offset,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            true_label.0 += offset;
            false_label.0 += offset;
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for bid in possible_targets {
                bid.0 += offset;
            }
        }
        Terminator::Switch { cases, default, .. } => {
            for (_, bid) in cases {
                bid.0 += offset;
            }
            default.0 += offset;
        }
        Terminator::Return(_) | Terminator::Unreachable => {}
    }
}

fn shift_lbb_string(s: &mut String, offset: u32) {
    if let Some(num_str) = s.strip_prefix(".LBB")
        && let Ok(id) = num_str.parse::<u32>()
    {
        *s = format!(".LBB{}", id + offset);
    }
}

fn shift_global_init_block_labels(init: &mut GlobalInit, offset: u32) {
    match init {
        GlobalInit::GlobalAddr(s) | GlobalInit::GlobalAddrOffset(s, _) => {
            shift_lbb_string(s, offset);
        }
        GlobalInit::GlobalLabelDiff(a, b, _) => {
            shift_lbb_string(a, offset);
            shift_lbb_string(b, offset);
        }
        GlobalInit::Compound(items) => {
            for item in items {
                shift_global_init_block_labels(item, offset);
            }
        }
        _ => {}
    }
}

fn build_tu_rename_map(
    m: &IrModule,
    tu_idx: usize,
    name_tu_counts: &FxHashMap<String, usize>,
) -> FxHashMap<String, String> {
    let mut map = FxHashMap::default();
    if tu_idx > 0 {
        for (lbl, _) in &m.string_literals {
            map.insert(lbl.clone(), format!("{lbl}.wp{tu_idx}"));
        }
        for (lbl, _) in &m.wide_string_literals {
            map.insert(lbl.clone(), format!("{lbl}.wp{tu_idx}"));
        }
        for (lbl, _) in &m.char16_string_literals {
            map.insert(lbl.clone(), format!("{lbl}.wp{tu_idx}"));
        }
    }
    for f in &m.functions {
        if f.is_static && !f.is_declaration && name_tu_counts.get(&f.name).copied().unwrap_or(0) > 1
        {
            map.insert(f.name.clone(), format!("{}.__wp{tu_idx}", f.name));
        }
    }
    for g in &m.globals {
        if g.is_static
            && !g.is_extern
            && (tu_idx > 0 && g.name.contains('.')
                || name_tu_counts.get(&g.name).copied().unwrap_or(0) > 1)
        {
            map.insert(g.name.clone(), format!("{}.__wp{tu_idx}", g.name));
        }
    }
    map
}

fn rename_sym(s: &mut String, map: &FxHashMap<String, String>) {
    if let Some(new_name) = map.get(s.as_str()) {
        *s = new_name.clone();
    }
}

fn apply_tu_renames(m: &mut IrModule, map: &FxHashMap<String, String>) {
    for (lbl, _) in &mut m.string_literals {
        rename_sym(lbl, map);
    }
    for (lbl, _) in &mut m.wide_string_literals {
        rename_sym(lbl, map);
    }
    for (lbl, _) in &mut m.char16_string_literals {
        rename_sym(lbl, map);
    }
    for f in &mut m.functions {
        if f.is_static {
            rename_sym(&mut f.name, map);
        }
        for b in &mut f.blocks {
            for inst in &mut b.instructions {
                match inst {
                    Instruction::Call { func, .. } | Instruction::InitTrampoline { func, .. } => {
                        rename_sym(func, map);
                    }
                    Instruction::GlobalAddr { name, .. }
                    | Instruction::PgoCounterInc { name, .. } => {
                        rename_sym(name, map);
                    }
                    Instruction::InlineAsm { input_symbols, .. } => {
                        for sym in input_symbols.iter_mut().flatten() {
                            rename_sym(sym, map);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for g in &mut m.globals {
        if g.is_static {
            rename_sym(&mut g.name, map);
        }
        rename_global_init(&mut g.init, map);
    }
    for c in &mut m.constructors {
        rename_sym(c, map);
    }
    for d in &mut m.destructors {
        rename_sym(d, map);
    }
    for (alias, target, _) in &mut m.aliases {
        rename_sym(alias, map);
        rename_sym(target, map);
    }
    for (sym, resolver, _, _) in &mut m.ifuncs {
        rename_sym(sym, map);
        rename_sym(resolver, map);
    }
    for (fn_name, _) in &mut m.symver_directives {
        rename_sym(fn_name, map);
    }

    let old_aligns = std::mem::take(&mut m.function_alignments);
    for (mut k, v) in old_aligns {
        rename_sym(&mut k, map);
        m.function_alignments.insert(k, v);
    }
    let old_tls = std::mem::take(&mut m.tls_model_attrs);
    for (mut k, v) in old_tls {
        rename_sym(&mut k, map);
        m.tls_model_attrs.insert(k, v);
    }
    let old_ev = std::mem::take(&mut m.externally_visible_symbols);
    for mut k in old_ev {
        rename_sym(&mut k, map);
        m.externally_visible_symbols.insert(k);
    }
    let old_zr = std::mem::take(&mut m.zero_call_used_regs_attrs);
    for (mut k, v) in old_zr {
        rename_sym(&mut k, map);
        m.zero_call_used_regs_attrs.insert(k, v);
    }
}

fn rename_global_init(init: &mut GlobalInit, map: &FxHashMap<String, String>) {
    match init {
        GlobalInit::GlobalAddr(s) | GlobalInit::GlobalAddrOffset(s, _) => rename_sym(s, map),
        GlobalInit::GlobalLabelDiff(a, b, _) => {
            rename_sym(a, map);
            rename_sym(b, map);
        }
        GlobalInit::Compound(items) => {
            for item in items {
                rename_global_init(item, map);
            }
        }
        _ => {}
    }
}

fn merge_one_tu(
    out: &mut IrModule,
    m: IrModule,
    func_index: &mut FxHashMap<String, usize>,
    global_index: &mut FxHashMap<String, usize>,
) -> Result<(), String> {
    for (k, v) in m.function_alignments {
        let entry = out.function_alignments.entry(k).or_insert(0);
        *entry = (*entry).max(v);
    }
    out.extern_function_symbols
        .extend(m.extern_function_symbols);
    out.string_literals.extend(m.string_literals);
    out.wide_string_literals.extend(m.wide_string_literals);
    out.char16_string_literals.extend(m.char16_string_literals);
    for c in m.constructors {
        if !out.constructors.contains(&c) {
            out.constructors.push(c);
        }
    }
    for d in m.destructors {
        if !out.destructors.contains(&d) {
            out.destructors.push(d);
        }
    }
    out.aliases.extend(m.aliases);
    out.ifuncs.extend(m.ifuncs);
    out.toplevel_asm.extend(m.toplevel_asm);
    out.symbol_attrs.extend(m.symbol_attrs);
    out.asm_labels.extend(m.asm_labels);
    out.symver_directives.extend(m.symver_directives);
    out.tls_model_attrs.extend(m.tls_model_attrs);
    out.externally_visible_symbols
        .extend(m.externally_visible_symbols);
    out.zero_call_used_regs_attrs
        .extend(m.zero_call_used_regs_attrs);

    for f in m.functions {
        if f.is_static {
            out.functions.push(f);
            continue;
        }
        if let Some(&idx) = func_index.get(&f.name) {
            merge_external_function(&mut out.functions[idx], f)?;
        } else {
            let idx = out.functions.len();
            func_index.insert(f.name.clone(), idx);
            out.functions.push(f);
        }
    }

    for g in m.globals {
        if g.is_static {
            out.globals.push(g);
            continue;
        }
        if let Some(&idx) = global_index.get(&g.name) {
            merge_external_global(&mut out.globals[idx], g)?;
        } else {
            let idx = out.globals.len();
            global_index.insert(g.name.clone(), idx);
            out.globals.push(g);
        }
    }
    Ok(())
}

fn func_strength(f: &IrFunction) -> u8 {
    if f.is_declaration {
        0
    } else if f.is_gnu_inline_def {
        1
    } else if f.is_weak {
        2
    } else {
        3
    }
}

fn merge_external_function(existing: &mut IrFunction, incoming: IrFunction) -> Result<(), String> {
    let s_old = func_strength(existing);
    let s_new = func_strength(&incoming);
    if s_old == 3 && s_new == 3 {
        return Err(format!("multiple definition of `{}`", incoming.name));
    }
    if s_new > s_old {
        let was_used = existing.is_used;
        let vis = existing.visibility.clone();
        *existing = incoming;
        existing.is_used |= was_used;
        if existing.visibility.is_none() {
            existing.visibility = vis;
        }
    } else {
        existing.is_used |= incoming.is_used;
        if existing.visibility.is_none() {
            existing.visibility = incoming.visibility;
        }
    }
    Ok(())
}

fn global_strength(g: &IrGlobal) -> u8 {
    if g.is_extern {
        0
    } else if g.is_weak {
        1
    } else if g.is_common {
        2 // COMMON symbol (`__attribute__((common))`)
    } else {
        3 // Strong definition (`-fno-common` C23/GCC 16 default)
    }
}

fn merge_external_global(existing: &mut IrGlobal, incoming: IrGlobal) -> Result<(), String> {
    let s_old = global_strength(existing);
    let s_new = global_strength(&incoming);
    if s_old == 3 && s_new == 3 {
        return Err(format!("multiple definition of `{}`", incoming.name));
    }
    if s_new > s_old {
        let max_align = existing.align.max(incoming.align);
        let max_size = existing.size.max(incoming.size);
        let was_used = existing.is_used;
        let vis = existing.visibility.clone();
        *existing = incoming;
        existing.align = max_align;
        existing.size = max_size.max(existing.size);
        existing.is_used |= was_used;
        if existing.visibility.is_none() {
            existing.visibility = vis;
        }
    } else {
        existing.align = existing.align.max(incoming.align);
        existing.size = existing.size.max(incoming.size);
        existing.is_used |= incoming.is_used;
        if existing.visibility.is_none() {
            existing.visibility = incoming.visibility;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;

    fn mk_defined_fn(name: &str, is_static: bool) -> IrFunction {
        let mut f = IrFunction::new(name.to_string(), IrType::I32, Vec::new(), false);
        f.is_static = is_static;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: Vec::new(),
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        f
    }

    #[test]
    fn merges_conflicting_statics_and_internalizes_non_roots() {
        let mut m1 = IrModule::new();
        m1.functions.push(mk_defined_fn("helper", true));
        m1.functions.push(mk_defined_fn("main", false));
        m1.functions.push(mk_defined_fn("public_api", false));
        m1.externally_visible_symbols
            .insert("public_api".to_string());
        m1.string_literals
            .push((".LC0".to_string(), "one".to_string()));

        let mut m2 = IrModule::new();
        m2.functions.push(mk_defined_fn("helper", true));
        m2.functions.push(mk_defined_fn("internal_only", false));
        m2.string_literals
            .push((".LC0".to_string(), "two".to_string()));

        let res = merge_modules(vec![m1, m2]);
        assert!(res.is_ok());
        let mut merged = res.unwrap_or_default();
        assert_eq!(merged.string_literals.len(), 2);
        assert_ne!(merged.string_literals[0].0, merged.string_literals[1].0);

        internalize_module(&mut merged, false);
        for f in &merged.functions {
            match f.name.as_str() {
                "main" | "public_api" => assert!(!f.is_static, "{} must remain external", f.name),
                _ => assert!(f.is_static, "{} must be internalized", f.name),
            }
        }
    }
}
