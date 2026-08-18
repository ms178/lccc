//! Module, function, and instruction generation dispatch.
//!
//! This module contains the top-level entry points that drive code generation:
//! - `generate_module`: emits data sections and iterates over functions
//! - `generate_function`: emits prologue, basic blocks, and epilogue
//! - `generate_instruction`: dispatches each IR instruction to arch trait methods
//! - `generate_terminator`: dispatches terminators to arch trait methods
//!
//! These functions are arch-independent — they use the `ArchCodegen` trait to call
//! into the backend-specific implementations.

use super::common;
use super::liveness::{
    for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction,
};
use super::traits::ArchCodegen;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::{SourceManager, Span};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{
    BasicBlock, GlobalInit, Instruction, IrBinOp, IrConst, IrFunction, IrModule, Operand,
    Terminator, Value,
};
use std::collections::hash_map::Entry;

/// Information about a GEP with a constant offset that can be folded into
/// Load/Store addressing modes. Instead of computing `base + offset` as a
/// separate instruction and spilling to stack, the constant offset is merged
/// directly into the memory operand of the subsequent load/store.
#[derive(Debug, Clone, Copy)]
pub(super) struct GepFoldInfo {
    /// The base pointer value (an alloca or previously-computed pointer).
    pub(super) base: Value,
    /// The constant byte offset to add to the base address.
    pub(super) offset: i64,
}

/// Information about a GEP with a *variable* offset that can be folded into
/// indexed (register+register) addressing on targets that support it.
/// `GEP(base, idx<<shift)` becomes `[base_reg, idx_reg, lsl #shift]`.
#[derive(Debug, Clone, Copy)]
pub(super) struct IndexedGepInfo {
    /// The base pointer value.
    pub(super) base: Value,
    /// The (unscaled) index value.
    pub(super) index: Value,
    /// Log2 scale: offset = index << shift (0 for byte offsets).
    pub(super) shift: u8,
    /// Original GEP offset operand. Needed to rematerialize the address if a
    /// later `emit_load_indexed`/`emit_store_indexed` refuses the fold after
    /// the GEP itself was skipped.
    pub(super) orig_offset: Value,
}

/// Presence-only env flag. Avoids allocating the value string on every lookup.
fn env_flag_set(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

/// Strip a `name+off` / `name-off` suffix produced by [`build_global_addr_map`].
/// Only a trailing `+digits` / `-digits` is treated as an offset; any other
/// `+`/`-` is assumed to be part of the symbol (e.g. C++ mangling is not
/// produced in this form by the map builder).
fn asm_symbol_basename(sym: &str) -> &str {
    if let Some(i) = sym.rfind('+') {
        if i > 0 && !sym[i + 1..].is_empty() && sym[i + 1..].bytes().all(|b| b.is_ascii_digit()) {
            return &sym[..i];
        }
    }
    if let Some(i) = sym.rfind('-') {
        if i > 0 && !sym[i + 1..].is_empty() && sym[i + 1..].bytes().all(|b| b.is_ascii_digit()) {
            return &sym[..i];
        }
    }
    sym
}

/// Escape `\`, `"`, and newlines so a path is safe inside a GAS `.file "..."`.
fn escape_dwarf_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// True iff `offset` is a displacement every backend can encode as a 32-bit
/// signed addressing-mode immediate.
///
/// Unsigned 32-bit wrap (U32 `-1` = `0xFFFF_FFFF`) is recovered only when
/// `allow_u32_wrap` is set — i.e. the producing Add's result type is ≤ 32
/// bits. Doing this for a 64-bit constant would turn `base + 4294967295`
/// into `base - 1` on LP64.
fn foldable_const_disp(c: &IrConst, allow_u32_wrap: bool) -> Option<i64> {
    let v = c.to_i64()?;
    if (i32::MIN as i64..=i32::MAX as i64).contains(&v) {
        return Some(v);
    }
    if allow_u32_wrap && v > i32::MAX as i64 && v <= u32::MAX as i64 {
        return Some(v as i32 as i64);
    }
    None
}

/// Per-function def / alloca / param facts used by GEP-fold soundness.
struct BaseStability {
    is_alloca: FxHashSet<u32>,
    is_param: FxHashSet<u32>,
    def_count: FxHashMap<u32, u32>,
}

fn analyze_base_stability(func: &IrFunction) -> BaseStability {
    let mut stab = BaseStability {
        is_alloca: FxHashSet::default(),
        is_param: FxHashSet::default(),
        def_count: FxHashMap::default(),
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { dest, .. } => {
                    stab.is_alloca.insert(dest.0);
                }
                Instruction::ParamRef { dest, .. } => {
                    stab.is_param.insert(dest.0);
                }
                _ => {}
            }
            if let Some(d) = inst.dest() {
                *stab.def_count.entry(d.0).or_insert(0) += 1;
            }
        }
    }
    stab
}

fn base_is_fold_stable(
    stab: &BaseStability,
    base: u32,
    dest: u32,
    adjacent_base_stable: &FxHashSet<u32>,
) -> bool {
    stab.is_alloca.contains(&base)
        || stab.is_param.contains(&base)
        || stab.def_count.get(&base).copied().unwrap_or(0) == 1
        || adjacent_base_stable.contains(&dest)
}

/// Index of the unique defining instruction of each single-def value.
/// Post-phi IR is not SSA: multi-def values are omitted (unsafe to inspect).
fn index_single_defs(func: &IrFunction) -> FxHashMap<u32, &Instruction> {
    let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                *def_count.entry(d.0).or_insert(0) += 1;
            }
        }
    }
    let mut index = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                if def_count.get(&d.0).copied() == Some(1) {
                    index.insert(d.0, inst);
                }
            }
        }
    }
    index
}

/// Adjacent `(addr_producer, Load/Store-of-that-addr)` pairs whose producer
/// is single-use. Between those two instructions the base cannot be
/// redefined, so a multi-def phi-web base is still fold-safe.
fn adjacent_addr_producers(
    func: &IrFunction,
    use_counts: &[u32],
    candidates: &FxHashSet<u32>,
) -> FxHashSet<u32> {
    let mut adjacent = FxHashSet::default();
    for block in &func.blocks {
        for pair in block.instructions.windows(2) {
            let Some(dest) = pair[0].dest() else { continue };
            if !candidates.contains(&dest.0)
                || use_counts.get(dest.0 as usize).copied().unwrap_or(0) != 1
            {
                continue;
            }
            if matches!(
                &pair[1],
                Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } if ptr.0 == dest.0
            ) {
                adjacent.insert(dest.0);
            }
        }
    }
    adjacent
}

/// Flatten `GEP(GEP(base, c1), c2)` / `Add(Add(base, c1), c2)` chains so the
/// ultimate load/store sees a single displacement. Offset overflow refuses
/// the compose (the inner producer stays as the base).
fn compose_const_gep_folds(map: &mut FxHashMap<u32, GepFoldInfo>) {
    if map.is_empty() {
        return;
    }
    let keys: Vec<u32> = map.keys().copied().collect();
    // Longest possible flatten is `keys.len()` hops; bound the loop so a
    // degenerate cycle cannot hang codegen.
    for _ in 0..keys.len() {
        let mut changed = false;
        for &k in &keys {
            let Some(info) = map.get(&k).copied() else {
                continue;
            };
            let Some(parent) = map.get(&info.base.0).copied() else {
                continue;
            };
            let Some(off) = info.offset.checked_add(parent.offset) else {
                continue;
            };
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&off) {
                continue;
            }
            if info.base.0 != parent.base.0 || info.offset != off {
                map.insert(
                    k,
                    GepFoldInfo {
                        base: parent.base,
                        offset: off,
                    },
                );
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

fn is_foldable_mem_ty(ty: IrType, seg: AddressSpace) -> bool {
    !is_wide_int_type(ty) && ty != IrType::F128 && seg == AddressSpace::Default
}

/// Build a map of value-offset GEPs foldable into indexed addressing.
/// Conditions: offset is a Value that resolves to `index` or `index << const`
/// (or `index * 2^k`), and the GEP dest is used only as a Load/Store pointer.
fn build_indexed_gep_map(func: &IrFunction, use_counts: &[u32]) -> FxHashMap<u32, IndexedGepInfo> {
    if env_flag_set("CCC_NO_GEP_FOLD") {
        return FxHashMap::default();
    }

    let defs = index_single_defs(func);

    // Resolve an offset value to (index, shift): look through a single
    // `Shl(idx, k)` / `Mul(idx, 2^k)` / widening Cast of those.
    // Multi-def offsets are treated as a plain index (shift 0) — we refuse
    // to inspect a non-unique definition.
    let resolve_index = |off_id: u32| -> Option<(Value, u8)> {
        let mut cur = defs.get(&off_id).copied();
        // Peel a single widening integer cast (i32 -> i64 index).
        if let Some(Instruction::Cast {
            src: Operand::Value(v),
            from_ty,
            to_ty,
            ..
        }) = cur
        {
            if from_ty.is_integer() && to_ty.is_integer() && to_ty.size() >= from_ty.size() {
                cur = defs.get(&v.0).copied();
            }
        }
        match cur {
            Some(Instruction::BinOp {
                op: IrBinOp::Shl,
                lhs: Operand::Value(idx),
                rhs: Operand::Const(c),
                ..
            }) => {
                let k = c.to_i64()?;
                if (0..=3).contains(&k) {
                    Some((*idx, k as u8))
                } else {
                    None
                }
            }
            Some(Instruction::BinOp {
                op: IrBinOp::Mul,
                lhs: Operand::Value(idx),
                rhs: Operand::Const(c),
                ..
            })
            | Some(Instruction::BinOp {
                op: IrBinOp::Mul,
                lhs: Operand::Const(c),
                rhs: Operand::Value(idx),
                ..
            }) => {
                let n = c.to_i64()?;
                if n > 0 && (n as u64).is_power_of_two() && n <= 8 {
                    Some((*idx, n.trailing_zeros() as u8))
                } else {
                    None
                }
            }
            _ => {
                // Plain value offset (or multi-def / unresolved): index with shift 0.
                Some((Value(off_id), 0))
            }
        }
    };

    let mut map: FxHashMap<u32, IndexedGepInfo> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GetElementPtr {
                dest,
                base,
                offset: Operand::Value(off),
                ..
            } = inst
            {
                if let Some((index, shift)) = resolve_index(off.0) {
                    map.insert(
                        dest.0,
                        IndexedGepInfo {
                            base: *base,
                            index,
                            shift,
                            orig_offset: *off,
                        },
                    );
                }
            }
        }
    }
    if map.is_empty() {
        return map;
    }

    // Same "used only as Load/Store ptr" verification as the constant-offset
    // fold. Sub-word + nonzero-shift legality is left to `emit_load_indexed`
    // (x86 SIB can encode `movzbl (b,i,s)`; AArch64 cannot). If the backend
    // refuses, generate_load rematerializes via `orig_offset`.
    let mut non_ptr_uses: FxHashSet<u32> = FxHashSet::default();
    let mut mark_non_ptr = |id: u32| {
        if map.contains_key(&id) {
            non_ptr_uses.insert(id);
        }
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load {
                    ptr,
                    ty,
                    seg_override,
                    ..
                } => {
                    if !is_foldable_mem_ty(*ty, *seg_override) {
                        mark_non_ptr(ptr.0);
                    }
                }
                Instruction::Store {
                    val,
                    ptr,
                    ty,
                    seg_override,
                    ..
                } => {
                    if let Operand::Value(v) = val {
                        mark_non_ptr(v.0);
                    }
                    if !is_foldable_mem_ty(*ty, *seg_override) {
                        mark_non_ptr(ptr.0);
                    }
                }
                _ => {
                    for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            mark_non_ptr(v.0);
                        }
                    });
                    for_each_value_use_in_instruction(inst, |v| mark_non_ptr(v.0));
                }
            }
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                mark_non_ptr(v.0);
            }
        });
    }
    for val_id in &non_ptr_uses {
        map.remove(val_id);
    }

    // Multi-def base soundness (mirrors `build_gep_fold_map`).
    let stab = analyze_base_stability(func);
    let cand: FxHashSet<u32> = map.keys().copied().collect();
    let adjacent = adjacent_addr_producers(func, use_counts, &cand);
    map.retain(|dest, info| base_is_fold_stable(&stab, info.base.0, *dest, &adjacent));

    map.retain(|val_id, _| {
        (*val_id as usize) < use_counts.len() && use_counts[*val_id as usize] > 0
    });
    map
}

/// Build a map of GEP/Add destinations that can be folded into Load/Store.
///
/// A GEP/Add is foldable when:
/// 1. Its offset is a compile-time constant
/// 2. The constant fits in a 32-bit signed displacement
/// 3. The result is only used as the ptr operand of Load/Store (or as the
///    base of another foldable GEP/Add that absorbs it)
/// 4. The base is frame-constant (alloca), a parameter, single-def, or the
///    producer and its sole memory user are adjacent
fn build_gep_fold_map(func: &IrFunction, use_counts: &[u32]) -> FxHashMap<u32, GepFoldInfo> {
    if env_flag_set("CCC_NO_GEP_FOLD") {
        return FxHashMap::default();
    }
    let mut gep_map: FxHashMap<u32, GepFoldInfo> = FxHashMap::default();

    // Phase 1: Collect foldable pointer-producing instructions.
    // (a) GetElementPtr with constant offset.
    // (b) BinOp(Add, base_value, Const) — pointer increments from `p++`-style
    //     loop-carried variables lower to Add, not GEP.
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(c),
                    ..
                } => {
                    // GEP offsets are typically pointer-width; refuse unsigned
                    // wrap so `base + 4294967295` cannot become `base - 1`.
                    let Some(offset_val) = foldable_const_disp(c, false) else {
                        continue;
                    };
                    gep_map.insert(
                        dest.0,
                        GepFoldInfo {
                            base: *base,
                            offset: offset_val,
                        },
                    );
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(base),
                    rhs: Operand::Const(c),
                    ty,
                }
                | Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Const(c),
                    rhs: Operand::Value(base),
                    ty,
                } => {
                    if ty.is_float() || ty.is_long_double() {
                        continue;
                    }
                    // 32-bit (and narrower) integer wrap is well-defined and
                    // matches the addressing-mode displacement we emit.
                    let allow_wrap = ty.size() <= 4;
                    let Some(offset_val) = foldable_const_disp(c, allow_wrap) else {
                        continue;
                    };
                    gep_map.insert(
                        dest.0,
                        GepFoldInfo {
                            base: *base,
                            offset: offset_val,
                        },
                    );
                }
                _ => {}
            }
        }
    }

    if gep_map.is_empty() {
        return gep_map;
    }
    let original_count = gep_map.len();

    compose_const_gep_folds(&mut gep_map);

    // The IR is NOT SSA at codegen time (phi elimination has run), so a base
    // value could be redefined between the GEP/Add and its Load/Store.
    // Folding would then use the NEW base with the OLD offset.
    let stab = analyze_base_stability(func);
    let cand: FxHashSet<u32> = gep_map.keys().copied().collect();
    let adjacent_base_stable = adjacent_addr_producers(func, use_counts, &cand);
    gep_map.retain(|dest, info| {
        base_is_fold_stable(&stab, info.base.0, *dest, &adjacent_base_stable)
    });

    // Phase 2: Verify each candidate is ONLY used as a Load/Store ptr, or as
    // the absorbed base of another still-foldable GEP/Add. Fixed-point: if a
    // consumer is later invalidated, its base use becomes a real use.
    loop {
        let mut non_ptr_uses: FxHashSet<u32> = FxHashSet::default();
        let mut mark_non_ptr = |id: u32| {
            if gep_map.contains_key(&id) {
                non_ptr_uses.insert(id);
            }
        };

        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Load {
                        ptr,
                        ty,
                        seg_override,
                        ..
                    } => {
                        if !is_foldable_mem_ty(*ty, *seg_override) {
                            mark_non_ptr(ptr.0);
                        }
                    }
                    Instruction::Store {
                        val,
                        ptr,
                        ty,
                        seg_override,
                        ..
                    } => {
                        if let Operand::Value(v) = val {
                            mark_non_ptr(v.0);
                        }
                        if !is_foldable_mem_ty(*ty, *seg_override) {
                            mark_non_ptr(ptr.0);
                        }
                    }
                    // Foldable GEP: dest is rewritten to (ultimate_base + const),
                    // so the *base* use is absorbed. A Value offset is a real use.
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => {
                        if gep_map.contains_key(&dest.0) {
                            if let Operand::Value(v) = offset {
                                mark_non_ptr(v.0);
                            }
                        } else {
                            mark_non_ptr(base.0);
                            if let Operand::Value(v) = offset {
                                mark_non_ptr(v.0);
                            }
                        }
                    }
                    // Foldable Add: one operand is the absorbed base, the other
                    // is Const. A stray Value on the const side is still a use
                    // but cannot occur for a candidate we inserted.
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ..
                    } if gep_map.contains_key(&dest.0) => {
                        if !matches!(lhs, Operand::Const(_)) {
                            if let Operand::Value(_) = lhs {
                                // lhs is the base — absorbed.
                            }
                        }
                        if !matches!(rhs, Operand::Const(_)) {
                            if let Operand::Value(_) = rhs {
                                // rhs is the base — absorbed.
                            }
                        }
                    }
                    _ => {
                        for_each_operand_in_instruction(inst, |op| {
                            if let Operand::Value(v) = op {
                                mark_non_ptr(v.0);
                            }
                        });
                        for_each_value_use_in_instruction(inst, |v| mark_non_ptr(v.0));
                    }
                }
            }
            for_each_operand_in_terminator(&block.terminator, |op| {
                if let Operand::Value(v) = op {
                    mark_non_ptr(v.0);
                }
            });
        }

        let before = gep_map.len();
        for val_id in &non_ptr_uses {
            gep_map.remove(val_id);
        }
        if gep_map.len() == before {
            break;
        }
    }

    gep_map.retain(|val_id, _| {
        (*val_id as usize) < use_counts.len() && use_counts[*val_id as usize] > 0
    });

    if env_flag_set("CCC_DEBUG_GEPFOLD") {
        eprintln!(
            "[GEPFOLD] total_candidates={} remaining={}",
            original_count,
            gep_map.len()
        );
    }

    gep_map
}

/// Public wrapper for backends needing the fold preview before regalloc.
pub fn build_global_addr_map_for(
    func: &IrFunction,
    tls_symbols: &FxHashSet<String>,
) -> FxHashMap<u32, String> {
    build_global_addr_map(func, tls_symbols, None)
}

/// Public wrapper for build_foldable_global_addr_set.
pub fn build_foldable_global_addr_set_for(
    func: &IrFunction,
    global_addr_map: &FxHashMap<u32, String>,
) -> FxHashSet<u32> {
    build_foldable_global_addr_set(func, global_addr_map)
}

/// Build a map from Value IDs to global symbol names (with optional offsets).
/// Maps `GlobalAddr { name }` to `"name"`, and values produced by
/// `GEP`/`Add`/`Copy`/same-size Cast of those to `"name+offset"`.
/// TLS and absolute (`.set sym, <imm>`) symbols are excluded because they
/// must not be folded into RIP-relative accesses.
fn build_global_addr_map(
    func: &IrFunction,
    tls_symbols: &FxHashSet<String>,
    absolute_symbols: Option<&FxHashSet<String>>,
) -> FxHashMap<u32, String> {
    let excluded = |name: &str| {
        tls_symbols.contains(name) || absolute_symbols.is_some_and(|s| s.contains(name))
    };

    let mut map: FxHashMap<u32, String> = FxHashMap::default();
    // GEP/Add edges: dest -> (base, offset). Copy/Cast edges: dest -> src.
    // Collected first so propagation is independent of block order.
    let mut const_off_edges: Vec<(u32, u32, i64)> = Vec::new();
    let mut copy_edges: Vec<(u32, u32)> = Vec::new();

    let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                *def_count.entry(dest.0).or_insert(0) += 1;
            }
        }
    }

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GlobalAddr { dest, name } => {
                    if !excluded(name.as_str()) {
                        map.insert(dest.0, name.clone());
                    }
                }
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(c),
                    ..
                } => {
                    if def_count.get(&dest.0).copied() == Some(1) {
                        if let Some(off) = foldable_const_disp(c, false) {
                            const_off_edges.push((dest.0, base.0, off));
                        }
                    }
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(base),
                    rhs: Operand::Const(c),
                    ty,
                }
                | Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Const(c),
                    rhs: Operand::Value(base),
                    ty,
                } if !ty.is_float() && !ty.is_long_double() => {
                    if def_count.get(&dest.0).copied() == Some(1) {
                        if let Some(off) = foldable_const_disp(c, ty.size() <= 4) {
                            const_off_edges.push((dest.0, base.0, off));
                        }
                    }
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } if def_count.get(&dest.0).copied() == Some(1) => {
                    copy_edges.push((dest.0, src.0));
                }
                Instruction::Cast {
                    dest,
                    src: Operand::Value(src),
                    from_ty,
                    to_ty,
                    ..
                } if def_count.get(&dest.0).copied() == Some(1)
                    && from_ty.size() == to_ty.size()
                    && !from_ty.is_float()
                    && !to_ty.is_float() =>
                {
                    // Same-size integer/pointer casts preserve symbol identity.
                    copy_edges.push((dest.0, src.0));
                }
                _ => {}
            }
        }
    }

    let mut users: FxHashMap<u32, Vec<(u32, i64)>> = FxHashMap::default();
    for (dest, src, off) in const_off_edges {
        users.entry(src).or_default().push((dest, off));
    }
    for (dest, src) in copy_edges {
        users.entry(src).or_default().push((dest, 0));
    }

    let mut queue: Vec<u32> = map.keys().copied().collect();
    let mut head = 0;
    while head < queue.len() {
        let src = queue[head];
        head += 1;
        let Some(name) = map.get(&src).cloned() else {
            continue;
        };
        if let Some(destinations) = users.get(&src) {
            for &(dest, off) in destinations {
                if map.contains_key(&dest) {
                    continue;
                }
                let sym = if off == 0 {
                    name.clone()
                } else if off > 0 {
                    format!("{}+{}", name, off)
                } else {
                    format!("{}{}", name, off)
                };
                map.insert(dest, sym);
                queue.push(dest);
            }
        }
    }
    map
}

/// Values (GlobalAddr *and* GEP/Add/Copy derived identities) that are dead
/// after RIP-relative Load/Store folding. A value is dead when EVERY use is
/// either a foldable Load/Store pointer, or the absorbed base of another
/// still-dead derived address.
fn build_foldable_global_addr_set(
    func: &IrFunction,
    global_addr_map: &FxHashMap<u32, String>,
) -> FxHashSet<u32> {
    let mut live: FxHashSet<u32> = global_addr_map.keys().copied().collect();
    if live.is_empty() {
        return live;
    }

    loop {
        let mut bad: FxHashSet<u32> = FxHashSet::default();
        let mut mark = |id: u32, bad: &mut FxHashSet<u32>| {
            if live.contains(&id) {
                bad.insert(id);
            }
        };

        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Load {
                        ptr,
                        ty,
                        seg_override,
                        ..
                    } => {
                        let foldable = live.contains(&ptr.0)
                            && global_addr_map.contains_key(&ptr.0)
                            && is_foldable_mem_ty(*ty, *seg_override);
                        if !foldable {
                            mark(ptr.0, &mut bad);
                        }
                    }
                    Instruction::Store {
                        val,
                        ptr,
                        ty,
                        seg_override,
                        ..
                    } => {
                        let foldable = live.contains(&ptr.0)
                            && global_addr_map.contains_key(&ptr.0)
                            && is_foldable_mem_ty(*ty, *seg_override);
                        if !foldable {
                            mark(ptr.0, &mut bad);
                        }
                        if let Operand::Value(v) = val {
                            mark(v.0, &mut bad);
                        }
                    }
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => {
                        if live.contains(&dest.0) {
                            if let Operand::Value(v) = offset {
                                mark(v.0, &mut bad);
                            }
                            // base absorbed into dest's symbol+offset identity
                        } else {
                            mark(base.0, &mut bad);
                            if let Operand::Value(v) = offset {
                                mark(v.0, &mut bad);
                            }
                        }
                    }
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ..
                    } if live.contains(&dest.0) => {
                        // Absorbed: dest is in the map, so one side is the
                        // symbol base and the other is a constant.
                        let _ = (lhs, rhs);
                    }
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(_),
                    } if live.contains(&dest.0) => {}
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(_),
                        ..
                    } if live.contains(&dest.0) => {}
                    _ => {
                        for v in inst.used_values() {
                            mark(v, &mut bad);
                        }
                    }
                }
            }
            for v in block.terminator.used_values() {
                mark(v, &mut bad);
            }
        }

        let before = live.len();
        for id in bad {
            live.remove(&id);
        }
        if live.len() == before {
            break;
        }
    }
    live
}

/// Build a set of GlobalAddr value IDs that are used as Load/Store pointers.
/// In kernel code model, GlobalAddr values used only as integer values
/// (e.g., `(unsigned long)_text`) need absolute addressing (R_X86_64_32S)
/// to produce the linked virtual address. But GlobalAddr values used as
/// Load/Store pointers need RIP-relative addressing so they work at any
/// physical/virtual address during early boot.
fn build_global_addr_ptr_set(func: &IrFunction) -> FxHashSet<u32> {
    let mut global_addrs: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GlobalAddr { dest, .. } = inst {
                global_addrs.insert(dest.0);
            }
        }
    }
    if global_addrs.is_empty() {
        return FxHashSet::default();
    }

    // Derivation edges dest -> src, collected independently of block order.
    // A worklist then floods GlobalAddr identity through the graph so a
    // Copy/GEP/Cast/Phi/Select/Add in an earlier-iterated block still sees
    // a GlobalAddr defined later in the function.
    let mut edges: Vec<(u32, u32)> = Vec::new();
    let mut ptr_uses: Vec<u32> = Vec::new();

    let track_op = |dest: u32, op: &Operand, edges: &mut Vec<(u32, u32)>| {
        if let Operand::Value(v) = op {
            edges.push((dest, v.0));
        }
    };

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GetElementPtr { dest, base, .. } => {
                    edges.push((dest.0, base.0));
                }
                Instruction::Copy { dest, src } => {
                    track_op(dest.0, src, &mut edges);
                }
                Instruction::Cast { dest, src, .. } => {
                    track_op(dest.0, src, &mut edges);
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add | IrBinOp::Sub,
                    lhs,
                    rhs,
                    ..
                } => {
                    // Pointer arithmetic preserves "this is a GlobalAddr-derived ptr".
                    track_op(dest.0, lhs, &mut edges);
                    track_op(dest.0, rhs, &mut edges);
                }
                Instruction::Phi { dest, incoming, .. } => {
                    for (op, _) in incoming {
                        track_op(dest.0, op, &mut edges);
                    }
                }
                Instruction::Select {
                    dest,
                    true_val,
                    false_val,
                    ..
                } => {
                    track_op(dest.0, true_val, &mut edges);
                    track_op(dest.0, false_val, &mut edges);
                }
                Instruction::Load { ptr, .. } => ptr_uses.push(ptr.0),
                Instruction::Store { ptr, .. } => ptr_uses.push(ptr.0),
                Instruction::Memcpy { dest, src, .. } => {
                    ptr_uses.push(dest.0);
                    ptr_uses.push(src.0);
                }
                Instruction::AtomicLoad { ptr, .. }
                | Instruction::AtomicStore { ptr, .. }
                | Instruction::AtomicRmw { ptr, .. }
                | Instruction::AtomicCmpxchg { ptr, .. } => {
                    if let Operand::Value(v) = ptr {
                        ptr_uses.push(v.0);
                    }
                }
                Instruction::AtomicInc { ptr, .. } => {
                    ptr_uses.push(ptr.0);
                }
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    // Conservatively mark GlobalAddr passed to calls as pointer use.
                    for arg in &info.args {
                        if let Operand::Value(v) = arg {
                            ptr_uses.push(v.0);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut derived_from: FxHashMap<u32, u32> = FxHashMap::default();
    let mut users: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for (dest, src) in &edges {
        users.entry(*src).or_default().push(*dest);
    }
    let mut queue: Vec<u32> = global_addrs.iter().copied().collect();
    for &g in &queue {
        derived_from.insert(g, g);
    }
    let mut head = 0;
    while head < queue.len() {
        let src = queue[head];
        head += 1;
        let orig = derived_from[&src];
        if let Some(dests) = users.get(&src) {
            for &dest in dests {
                if let Entry::Vacant(e) = derived_from.entry(dest) {
                    e.insert(orig);
                    queue.push(dest);
                }
            }
        }
    }

    let mut ptr_set: FxHashSet<u32> = FxHashSet::default();
    for id in ptr_uses {
        if let Some(&orig) = derived_from.get(&id) {
            ptr_set.insert(orig);
        }
    }
    ptr_set
}

/// Returns the number of times each IR Value is used as an operand in
/// instructions or terminators. Indexed by Value ID; used to identify
/// single-use values eligible for compare-branch fusion.
fn count_value_uses(func: &IrFunction) -> Vec<u32> {
    // Size from both defs and uses so an out-of-range operand cannot silently
    // report use-count 0 (which would make `== 1` checks spuriously fail
    // closed in some detectors and spuriously pass in others).
    let mut max_id: u32 = 0;
    let bump = |max_id: &mut u32, id: u32| {
        if id > *max_id {
            *max_id = id;
        }
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                bump(&mut max_id, dest.0);
            }
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    bump(&mut max_id, v.0);
                }
            });
            for_each_value_use_in_instruction(inst, |v| bump(&mut max_id, v.0));
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                bump(&mut max_id, v.0);
            }
        });
    }
    let mut counts = vec![0u32; max_id as usize + 1];

    let mut count_id = |id: u32| {
        if (id as usize) < counts.len() {
            counts[id as usize] += 1;
        }
    };

    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    count_id(v.0);
                }
            });
            for_each_value_use_in_instruction(inst, |v| count_id(v.0));
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                count_id(v.0);
            }
        });
    }
    counts
}

/// Detect if a block ends in a Cmp (optionally through Copy/integer Cast)
/// whose result is only used by the CondBranch terminator.
fn detect_cmp_branch_fusion(
    block: &BasicBlock,
    use_counts: &[u32],
    fuse_fp: bool,
) -> Option<(usize, Option<usize>)> {
    let (cond, _, _) = match &block.terminator {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => (cond, true_label, false_label),
        _ => return None,
    };

    let cond_val = match cond {
        Operand::Value(v) => v,
        _ => return None,
    };

    // Walk backward through the complete value-preserving boolean chain:
    //
    //   Cmp -> [Copy | integer Cast]* -> CondBranch
    //
    // Cmp results are exactly 0 or 1, so integer width/sign casts preserve
    // their truth value. Frontend integer promotions commonly leave several
    // casts here (for example i8 -> signed char -> int).
    let n = block.instructions.len();
    if n == 0 {
        return None;
    }
    let mut wanted = cond_val.0;
    let mut scan = n;
    let mut chain_end = None;
    let cmp_idx = loop {
        if scan == 0 {
            return None;
        }
        scan -= 1;
        match &block.instructions[scan] {
            Instruction::Cmp { dest, .. } if dest.0 == wanted => break scan,
            Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } if dest.0 == wanted => {
                if use_counts.get(dest.0 as usize).copied().unwrap_or(u32::MAX) != 1 {
                    return None;
                }
                chain_end.get_or_insert(n - 1);
                wanted = src.0;
            }
            Instruction::Cast {
                dest,
                src: Operand::Value(src),
                from_ty,
                to_ty,
            } if dest.0 == wanted && from_ty.is_integer() && to_ty.is_integer() => {
                if use_counts.get(dest.0 as usize).copied().unwrap_or(u32::MAX) != 1 {
                    return None;
                }
                chain_end.get_or_insert(n - 1);
                wanted = src.0;
            }
            _ => return None,
        }
    };

    let cmp_dest = match &block.instructions[cmp_idx] {
        Instruction::Cmp { dest, .. } => dest.0,
        _ => unreachable!(),
    };
    if wanted != cmp_dest
        || use_counts
            .get(cmp_dest as usize)
            .copied()
            .unwrap_or(u32::MAX)
            != 1
    {
        return None;
    }

    let ty = match &block.instructions[cmp_idx] {
        Instruction::Cmp { ty, .. } => ty,
        _ => return None,
    };

    // Don't fuse wide-int comparisons (they have special codegen paths).
    // FP comparisons only on backends with a fused FP compare-and-branch
    // (AArch64: fcmp + b.cc with cond codes identical to the cset path,
    // so NaN semantics are preserved bit-for-bit) — and ONLY for the
    // hardware types F32/F64. F128/long-double lowers to soft-float
    // libcalls; routing it through a d-register fcmp reads half the value.
    if is_wide_int_type(*ty) {
        return None;
    }
    if ty.is_float() && !(fuse_fp && matches!(ty, IrType::F32 | IrType::F64)) {
        return None;
    }
    if crate::common::types::target_is_32bit() && matches!(ty, IrType::I64 | IrType::U64) {
        return None;
    }

    Some((cmp_idx, chain_end))
}

/// Detect compare-and-select fusion opportunities within a block.
///
/// Finds `Select` instructions whose condition is an integer Cmp result used
/// nowhere else (optionally through one dead Copy or integer Cast).
///
/// Returns one entry per fusable Select: (select_idx, cmp_idx, dead_mid_idx).
fn detect_cmp_select_fusion(
    block: &BasicBlock,
    use_counts: &[u32],
) -> Vec<(usize, usize, Option<usize>)> {
    let uses = |v: u32| -> u32 { use_counts.get(v as usize).copied().unwrap_or(u32::MAX) };
    let mut out = Vec::new();
    for (sel_idx, inst) in block.instructions.iter().enumerate() {
        let Instruction::Select {
            cond: Operand::Value(cond_v),
            ty: sel_ty,
            ..
        } = inst
        else {
            continue;
        };
        if sel_ty.is_long_double() || sel_ty.is_128bit() {
            continue;
        }
        // Find the defining Cmp of the select condition, either directly or
        // through one Copy/integer-Cast. Post-phi IR is not SSA, so we only
        // accept the immediately-adjacent shapes (see below).
        let mut cmp_idx = None;
        let mut mid_idx = None;
        if sel_idx == 0 {
            continue;
        }
        match &block.instructions[sel_idx - 1] {
            Instruction::Cmp { dest, .. } if dest.0 == cond_v.0 => {
                cmp_idx = Some(sel_idx - 1);
            }
            Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } if dest.0 == cond_v.0 && sel_idx >= 2 => {
                if let Instruction::Cmp { dest: cd, .. } = &block.instructions[sel_idx - 2] {
                    if cd.0 == src.0 {
                        cmp_idx = Some(sel_idx - 2);
                        mid_idx = Some(sel_idx - 1);
                    }
                }
            }
            Instruction::Cast {
                dest,
                src: Operand::Value(src),
                from_ty,
                to_ty,
            } if dest.0 == cond_v.0
                && sel_idx >= 2
                && from_ty.is_integer()
                && to_ty.is_integer() =>
            {
                if let Instruction::Cmp { dest: cd, .. } = &block.instructions[sel_idx - 2] {
                    if cd.0 == src.0 {
                        cmp_idx = Some(sel_idx - 2);
                        mid_idx = Some(sel_idx - 1);
                    }
                }
            }
            _ => {}
        }
        let Some(ci) = cmp_idx else { continue };
        let Instruction::Cmp {
            dest: cmp_dest,
            ty: cmp_ty,
            ..
        } = &block.instructions[ci]
        else {
            continue;
        };
        if cmp_ty.is_float() || cmp_ty.is_long_double() || cmp_ty.is_128bit() {
            continue;
        }
        if crate::common::types::target_is_32bit() && matches!(cmp_ty, IrType::I64 | IrType::U64)
        {
            continue;
        }
        if mid_idx.is_some() {
            if uses(cmp_dest.0) != 1 || uses(cond_v.0) != 1 {
                continue;
            }
        } else if uses(cmp_dest.0) != 1 {
            continue;
        }
        out.push((sel_idx, ci, mid_idx));
    }
    out
}

/// Detect multiply-add fusion opportunities within a block.
///
/// Finds `BinOp::Mul` instructions whose result is used exactly once as an
/// operand of a nearby `BinOp::Add`. Returns mul_idx -> add_idx.
fn detect_mul_add_fusions(
    block: &BasicBlock,
    use_counts: &[u32],
    fuse_float: bool,
) -> FxHashMap<usize, usize> {
    let mut fusion_map: FxHashMap<usize, usize> = FxHashMap::default();
    // An Add may be claimed by at most one Mul. `a*b + c*d` must not fuse
    // the first multiply with an add whose other operand is itself a
    // not-yet-executed multiply.
    let mut claimed_adds: FxHashSet<usize> = FxHashSet::default();

    for (idx, inst) in block.instructions.iter().enumerate() {
        let (mul_dest, mul_ty) = match inst {
            Instruction::BinOp {
                dest,
                op: IrBinOp::Mul,
                ty,
                ..
            } => (dest, ty),
            _ => continue,
        };

        if (mul_ty.is_float() && !fuse_float)
            || matches!(mul_ty, IrType::F128 | IrType::I128 | IrType::U128)
        {
            continue;
        }

        let mul_uses = use_counts.get(mul_dest.0 as usize).copied().unwrap_or(0);
        if mul_uses != 1 {
            continue;
        }

        // SOUNDNESS: the fused sequence is emitted at the MUL's program
        // point, i.e. BEFORE any skipped instruction executes. Skipped
        // Copy/Cast instructions therefore must not DEFINE any value the
        // Add consumes.
        let mut add_idx = None;
        let mut add_lhs_r = None;
        let mut add_rhs_r = None;
        let mut add_ty_r = None;
        let mut skipped_defs: [u32; 6] = [u32::MAX; 6];
        let mut skipped_count = 0usize;
        let max_scan = (idx + 6).min(block.instructions.len());
        for scan in (idx + 1)..max_scan {
            match &block.instructions[scan] {
                Instruction::Copy { dest, .. } | Instruction::Cast { dest, .. } => {
                    if skipped_count < skipped_defs.len() {
                        skipped_defs[skipped_count] = dest.0;
                        skipped_count += 1;
                    }
                    continue;
                }
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ty,
                    ..
                } => {
                    add_idx = Some(scan);
                    add_lhs_r = Some(lhs);
                    add_rhs_r = Some(rhs);
                    add_ty_r = Some(ty);
                    break;
                }
                _ => break,
            }
        }
        let (Some(next_idx), Some(add_lhs), Some(add_rhs), Some(add_ty)) =
            (add_idx, add_lhs_r, add_rhs_r, add_ty_r)
        else {
            continue;
        };
        if claimed_adds.contains(&next_idx) {
            continue;
        }

        let mul_is_lhs = matches!(add_lhs, Operand::Value(v) if v.0 == mul_dest.0);
        let mul_is_rhs = matches!(add_rhs, Operand::Value(v) if v.0 == mul_dest.0);
        if !mul_is_lhs && !mul_is_rhs {
            continue;
        }

        let defined_between = |op: &Operand| {
            matches!(op, Operand::Value(v) if skipped_defs[..skipped_count].contains(&v.0))
        };
        if defined_between(add_lhs) || defined_between(add_rhs) {
            continue;
        }

        if mul_ty != add_ty {
            continue;
        }

        claimed_adds.insert(next_idx);
        fusion_map.insert(idx, next_idx);
    }

    fusion_map
}

/// Find adjacent `shift; logical` pairs that AArch64 can encode as one
/// shifted-register logical instruction. Requiring one use makes eliding the
/// standalone shift safe.
fn detect_shifted_logical_fusions(block: &BasicBlock, use_counts: &[u32]) -> FxHashSet<usize> {
    let mut logical_indices = FxHashSet::default();
    for (idx, pair) in block.instructions.windows(2).enumerate() {
        let (shift_dest, shift_ty, shift_amt) = match &pair[0] {
            Instruction::BinOp {
                dest,
                op,
                rhs: Operand::Const(c),
                ty,
                ..
            } if matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr)
                && matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) =>
            {
                (dest, ty, c)
            }
            _ => continue,
        };
        // AArch64 shifted-register logicals encode a 0..=63 (or 0..=31) amount.
        let Some(k) = shift_amt.to_i64() else { continue };
        let max_shift = if matches!(shift_ty, IrType::I64 | IrType::U64) {
            63
        } else {
            31
        };
        if k < 0 || k > max_shift {
            continue;
        }
        if use_counts.get(shift_dest.0 as usize).copied().unwrap_or(0) != 1 {
            continue;
        }
        let (lhs, rhs, logical_ty) = match &pair[1] {
            Instruction::BinOp {
                op, lhs, rhs, ty, ..
            } if matches!(op, IrBinOp::And | IrBinOp::Or | IrBinOp::Xor) => (lhs, rhs, ty),
            _ => continue,
        };
        let uses_shift = matches!(lhs, Operand::Value(v) if v.0 == shift_dest.0)
            || matches!(rhs, Operand::Value(v) if v.0 == shift_dest.0);
        if uses_shift && logical_ty == shift_ty {
            logical_indices.insert(idx + 1);
        }
    }
    logical_indices
}

/// Text section a function body must live in, honouring custom `section`
/// attributes and `-ffunction-sections`.
fn function_text_section(func: &IrFunction, function_sections: bool) -> String {
    if let Some(ref sect) = func.section {
        sect.clone()
    } else if function_sections {
        format!(".text.{}", func.name)
    } else {
        ".text".to_string()
    }
}

fn emit_switch_to_section(cg: &mut dyn ArchCodegen, sect: &str) {
    if sect == ".text" {
        cg.state().emit(".section .text,\"ax\",@progbits");
    } else {
        cg.state()
            .emit_fmt(format_args!(".section {},\"ax\",@progbits", sect));
    }
    cg.state().current_text_section = sect.to_string();
}

/// Generate assembly for an IR module with debug info support.
/// Sets `debug_info` on the codegen state before proceeding.
pub fn generate_module_with_debug(
    cg: &mut dyn ArchCodegen,
    module: &IrModule,
    debug_info: bool,
    source_mgr: Option<&crate::common::source::SourceManager>,
) -> String {
    cg.state().debug_info = debug_info;
    generate_module(cg, module, source_mgr)
}

pub fn generate_module(
    cg: &mut dyn ArchCodegen,
    module: &IrModule,
    source_mgr: Option<&crate::common::source::SourceManager>,
) -> String {
    pre_size_output_buffer(cg, module);
    collect_symbol_sets(cg, module);
    let file_table = build_and_emit_dwarf_file_table(cg, module, source_mgr);

    let ptr_dir = cg.ptr_directive();
    let pic_mode = cg.state_ref().pic_mode;
    common::emit_data_sections(&mut cg.state().out, module, ptr_dir, pic_mode);

    // Emit top-level asm("...") directives verbatim (e.g., musl's _start definition).
    // Switch to .text first so that labels/code in the asm land in the correct section.
    if !module.toplevel_asm.is_empty() {
        cg.state().emit(".text");
        for asm_str in &module.toplevel_asm {
            cg.state().emit(asm_str);
        }
    }

    // Numeric `.set sym, <number>` directives in top-level asm define ABSOLUTE
    // symbols (glibc localeinfo.h _NL_CURRENT_DEFINE). Their addresses are
    // link-time constants and must never go through the GOT.
    for asm_str in &module.toplevel_asm {
        for line in asm_str.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix(".set ") {
                let mut it = rest.splitn(2, ',');
                if let (Some(sym), Some(val)) = (it.next(), it.next()) {
                    let sym = sym.trim();
                    let val = val.trim();
                    if !sym.is_empty() && looks_like_absolute_imm(val) {
                        cg.state().absolute_symbols.insert(sym.to_string());
                    }
                }
            }
        }
    }

    let referenced_symbols = collect_referenced_symbols(module);
    emit_extern_visibility_directives(cg, module, &referenced_symbols);
    emit_functions_and_sections(cg, module, source_mgr, &file_table);
    emit_aliases(cg, module);
    emit_symver_directives(cg, module);
    emit_symbol_attrs(cg, module, &referenced_symbols);
    emit_init_fini_arrays(cg, module, ptr_dir);

    // Emit architecture-specific runtime helper stubs (e.g., i686 __divdi3)
    cg.emit_runtime_stubs();

    // Emit floating-point constant pool (.rodata)
    cg.state().emit_fp_const_pool();

    // Emit .note.GNU-stack section to indicate non-executable stack
    cg.state().emit("");
    cg.state().emit(".section .note.GNU-stack,\"\",@progbits");

    std::mem::take(&mut cg.state().out.buf)
}

/// Conservative recogniser for numeric `.set` right-hand sides (`42`, `-1`, `0x1f`).
fn looks_like_absolute_imm(val: &str) -> bool {
    let s = val.strip_prefix(['+', '-']).unwrap_or(val);
    if s.is_empty() {
        return false;
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    s.chars().all(|c| c.is_ascii_digit())
}

/// Pre-size the output buffer based on total IR instruction count to avoid
/// repeated reallocations. Each IR instruction typically generates ~40 bytes
/// of assembly text.
fn pre_size_output_buffer(cg: &mut dyn ArchCodegen, module: &IrModule) {
    let total_insts: usize = module
        .functions
        .iter()
        .map(|f| {
            f.blocks
                .iter()
                .map(|b| b.instructions.len() + 1)
                .sum::<usize>()
        })
        .sum();
    let estimated_bytes = (total_insts * 40).clamp(256 * 1024, 64 * 1024 * 1024);
    let state = cg.state();
    if state.out.buf.capacity() < estimated_bytes {
        state
            .out
            .buf
            .reserve(estimated_bytes - state.out.buf.capacity());
    }
}

/// Build the sets of locally-defined, thread-local, and weak extern symbols.
fn collect_symbol_sets(cg: &mut dyn ArchCodegen, module: &IrModule) {
    let state = cg.state();
    for func in &module.functions {
        if func.is_static
            || matches!(
                func.visibility.as_deref(),
                Some("hidden" | "internal" | "protected")
            )
        {
            state.local_symbols.insert(func.name.clone());
        }
    }
    for global in &module.globals {
        if global.is_static
            || matches!(
                global.visibility.as_deref(),
                Some("hidden" | "internal" | "protected")
            )
        {
            state.local_symbols.insert(global.name.clone());
        }
        if global.is_thread_local {
            state.tls_symbols.insert(global.name.clone());
        }
        if global.is_weak && global.is_extern {
            state.weak_extern_symbols.insert(global.name.clone());
        }
    }
    for (name, is_weak, visibility) in &module.symbol_attrs {
        if matches!(
            visibility.as_deref(),
            Some("hidden" | "internal" | "protected")
        ) {
            state.local_symbols.insert(name.clone());
        }
        if *is_weak {
            state.weak_extern_symbols.insert(name.clone());
        }
    }
    for (label, _) in &module.string_literals {
        state.local_symbols.insert(label.clone());
    }
    for (label, _) in &module.wide_string_literals {
        state.local_symbols.insert(label.clone());
    }
}

/// Build the DWARF file table and emit .file directives when debug info is enabled.
fn build_and_emit_dwarf_file_table(
    cg: &mut dyn ArchCodegen,
    module: &IrModule,
    source_mgr: Option<&crate::common::source::SourceManager>,
) -> FxHashMap<String, u32> {
    if !cg.state_ref().debug_info {
        return FxHashMap::default();
    }
    let sm = match source_mgr {
        Some(sm) => sm,
        None => return FxHashMap::default(),
    };

    let mut table: FxHashMap<String, u32> = FxHashMap::default();
    let mut next_id: u32 = 1;
    for func in &module.functions {
        if func.is_declaration {
            continue;
        }
        for block in &func.blocks {
            for span in &block.source_spans {
                if span.start == 0 && span.end == 0 {
                    continue;
                }
                let loc = sm.resolve_span(*span);
                if let Entry::Vacant(e) = table.entry(loc.file) {
                    e.insert(next_id);
                    next_id += 1;
                }
            }
        }
    }

    if !table.is_empty() {
        let mut entries: Vec<(&String, &u32)> = table.iter().collect();
        entries.sort_by_key(|(_name, id)| *id);
        for (name, id) in entries {
            cg.state().emit_fmt(format_args!(
                ".file {} \"{}\"",
                id,
                escape_dwarf_path(name)
            ));
        }
    }
    table
}

/// Collect the set of symbols actually referenced in this translation unit.
fn collect_referenced_symbols(module: &IrModule) -> FxHashSet<String> {
    let mut refs = FxHashSet::default();

    for func in &module.functions {
        if func.is_declaration {
            continue;
        }
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Call { func: callee, .. } => {
                        refs.insert(callee.clone());
                    }
                    Instruction::GlobalAddr { name, .. } => {
                        refs.insert(name.clone());
                    }
                    Instruction::InlineAsm { input_symbols, .. } => {
                        for s in input_symbols.iter().flatten() {
                            let base = s.split('+').next().unwrap_or(s);
                            refs.insert(base.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    for global in &module.globals {
        fn collect_global_refs(init: &GlobalInit, refs: &mut FxHashSet<String>) {
            match init {
                GlobalInit::GlobalAddr(name) | GlobalInit::GlobalAddrOffset(name, _) => {
                    refs.insert(name.clone());
                }
                GlobalInit::GlobalLabelDiff(a, b, _) => {
                    refs.insert(a.clone());
                    refs.insert(b.clone());
                }
                GlobalInit::Compound(inits) => {
                    for sub in inits {
                        collect_global_refs(sub, refs);
                    }
                }
                _ => {}
            }
        }
        collect_global_refs(&global.init, &mut refs);
    }

    for asm_str in &module.toplevel_asm {
        for (sym_name, _, _) in &module.symbol_attrs {
            if asm_str.contains(sym_name.as_str()) {
                refs.insert(sym_name.clone());
            }
        }
    }

    for func in &module.functions {
        if !func.is_declaration {
            refs.insert(func.name.clone());
        }
    }
    for global in &module.globals {
        if !global.is_extern {
            refs.insert(global.name.clone());
        }
    }
    refs
}

/// Emit visibility directives for declaration-only (extern) functions with
/// non-default visibility, but only if they are actually referenced.
fn emit_extern_visibility_directives(
    cg: &mut dyn ArchCodegen,
    module: &IrModule,
    referenced_symbols: &FxHashSet<String>,
) {
    for func in &module.functions {
        if func.is_declaration && referenced_symbols.contains(&func.name) {
            cg.state().emit_visibility(&func.name, &func.visibility);
        }
    }
}

/// Emit text section, handle custom sections, and generate code for each function.
/// Always re-selects the function's section immediately before emission so a
/// previous function's `.rodata` constant pool or PFE section cannot leak.
fn emit_functions_and_sections(
    cg: &mut dyn ArchCodegen,
    module: &IrModule,
    source_mgr: Option<&crate::common::source::SourceManager>,
    file_table: &FxHashMap<String, u32>,
) {
    let function_sections = cg.state().function_sections;
    if !function_sections {
        cg.state().emit(".section .text,\"ax\",@progbits");
        cg.state().current_text_section = ".text".to_string();
    }
    for func in &module.functions {
        // GNU89/gnu_inline semantics: `extern inline __attribute__((gnu_inline))`
        // bodies exist ONLY for inlining; no standalone definition is ever
        // emitted. Match GCC: never emit.
        if func.is_gnu_inline_def {
            continue;
        }
        if !func.is_declaration {
            let sect = function_text_section(func, function_sections);
            emit_switch_to_section(cg, &sect);
            generate_function(cg, func, source_mgr, file_table);
        }
    }
}

/// Emit symbol aliases from __attribute__((alias("target"))).
fn emit_aliases(cg: &mut dyn ArchCodegen, module: &IrModule) {
    for (alias_name, target_name, is_weak) in &module.aliases {
        // The alias name is already asm-resolved by the lowerer.
        // Resolving it AGAIN here would corrupt glibc hidden_ver aliases.
        // The TARGET may still need asm resolution.
        let target_resolved = module.asm_labels.get(target_name).unwrap_or(target_name);
        // Self-alias guard: `.set x, x` after resolution would create a
        // duplicate symbol entry; GCC folds it silently.
        if alias_name.as_str() == target_resolved.as_str() {
            continue;
        }
        cg.state().emit("");
        if *is_weak {
            cg.state().emit_fmt(format_args!(".weak {}", alias_name));
        } else {
            cg.state().emit_fmt(format_args!(".globl {}", alias_name));
        }
        cg.state()
            .emit_fmt(format_args!(".set {},{}", alias_name, target_resolved));
    }
}

/// Emit .symver directives from __attribute__((symver("name@@VERSION"))).
fn emit_symver_directives(cg: &mut dyn ArchCodegen, module: &IrModule) {
    for (func_name, symver_str) in &module.symver_directives {
        cg.state()
            .emit_fmt(format_args!(".symver {},{}", func_name, symver_str));
    }
}

/// Emit .weak/.hidden directives for declaration symbols that are referenced.
fn emit_symbol_attrs(
    cg: &mut dyn ArchCodegen,
    module: &IrModule,
    referenced_symbols: &FxHashSet<String>,
) {
    let defined_labels: FxHashSet<&str> = module.globals.iter().map(|g| g.name.as_str()).collect();
    for (name, is_weak, visibility) in &module.symbol_attrs {
        let resolved = module
            .asm_labels
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or(name.as_str());
        if !referenced_symbols.contains(name) && !defined_labels.contains(resolved) {
            continue;
        }
        if *is_weak {
            cg.state().emit_fmt(format_args!(".weak {}", resolved));
        }
        cg.state().emit_visibility(resolved, visibility);
    }
}

/// Emit .init_array and .fini_array sections for constructor/destructor functions.
fn emit_init_fini_arrays(
    cg: &mut dyn ArchCodegen,
    module: &IrModule,
    ptr_dir: super::common::PtrDirective,
) {
    let align = crate::common::types::target_ptr_size();
    if !module.constructors.is_empty() {
        cg.state().emit("");
        cg.state().emit(".section .init_array,\"aw\",@init_array");
        cg.state()
            .emit_fmt(format_args!(".align {}", ptr_dir.align_arg(align)));
        for ctor in &module.constructors {
            cg.state()
                .emit_fmt(format_args!("{} {}", ptr_dir.as_str(), ctor));
        }
    }
    if !module.destructors.is_empty() {
        cg.state().emit("");
        cg.state().emit(".section .fini_array,\"aw\",@fini_array");
        cg.state()
            .emit_fmt(format_args!(".align {}", ptr_dir.align_arg(align)));
        for dtor in &module.destructors {
            cg.state()
                .emit_fmt(format_args!("{} {}", ptr_dir.as_str(), dtor));
        }
    }
}

fn template_mentions_frame_pointer(template: &str) -> bool {
    // x86-64 / i686 / AArch64 frame-pointer names that inline asm may capture.
    const NEEDLES: &[&str] = &[
        "%rbp", "{rbp}", "%%rbp", "%ebp", "{ebp}", "%%ebp", "x29", "{x29}",
        "fp", "{fp}",
    ];
    NEEDLES.iter().any(|n| template.contains(n))
}

/// Generate code for a single function.
fn generate_function(
    cg: &mut dyn ArchCodegen,
    func: &IrFunction,
    source_mgr: Option<&SourceManager>,
    file_table: &FxHashMap<String, u32>,
) {
    cg.state().reset_for_function();

    if env_flag_set("CCC_DUMP_IR")
        && std::env::var("CCC_DUMP_IR_FUNC")
            .map(|f| func.name.contains(&f))
            .unwrap_or(true)
    {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "\n//=== IR dump: {} ===", func.name);
        for (bi, block) in func.blocks.iter().enumerate() {
            let _ = writeln!(out, "block {} ({}):", bi, block.label);
            for inst in &block.instructions {
                let _ = writeln!(out, "  {:?}", inst);
            }
            let _ = writeln!(out, "  term: {:?}", block.terminator);
        }
        eprintln!("{}", out);
    }

    let type_dir = cg.function_type_directive();
    let func_sect = cg.state_ref().current_text_section.clone();

    cg.state()
        .emit_linkage(&func.name, func.is_static, func.is_weak);
    cg.state().emit_visibility(&func.name, &func.visibility);

    // Emit patchable function entry NOP padding (-fpatchable-function-entry=N,M).
    // Skip patchable entries for inline functions: emitting
    // __patchable_function_entries for every static inline from a header
    // overwhelms the kernel's ftrace initialization.
    let emit_patchable = !func.is_inline;
    if emit_patchable {
        if let Some((total, before)) = cg.state().patchable_function_entry {
            if total > 0 {
                let pfe_id = cg.state().next_label_id();
                let pfe_label = format!(".LPFE{}", pfe_id);

                cg.state().emit_fmt(format_args!(
                    ".section __patchable_function_entries,\"awo\",@progbits,{}",
                    pfe_label
                ));
                let pfe_align = crate::common::types::target_ptr_size();
                let pfe_dir = cg.ptr_directive();
                cg.state().emit_fmt(format_args!(".align {}", pfe_align));
                cg.state()
                    .emit_fmt(format_args!("{} {}", pfe_dir.as_str(), pfe_label));

                // Restore THIS function's section (custom / .text.foo / .text),
                // not a hard-coded `.text` that would break -ffunction-sections
                // and leave the body in the PFE section.
                emit_switch_to_section(cg, &func_sect);

                // Align the NOP area / entry after we are back in the
                // function section. Emitting `.p2align` before the PFE
                // switch would pad the previous section instead.
                if let Some(log2) = cg.function_alignment_log2() {
                    cg.state().emit_fmt(format_args!(".p2align {}", log2));
                }

                cg.state().emit_fmt(format_args!("{}:", pfe_label));
                for _ in 0..before {
                    cg.state().emit("nop");
                }
            } else if let Some(log2) = cg.function_alignment_log2() {
                cg.state().emit_fmt(format_args!(".p2align {}", log2));
            }
        } else if let Some(log2) = cg.function_alignment_log2() {
            cg.state().emit_fmt(format_args!(".p2align {}", log2));
        }
    } else if let Some(log2) = cg.function_alignment_log2() {
        cg.state().emit_fmt(format_args!(".p2align {}", log2));
    }

    cg.state()
        .emit_fmt(format_args!(".type {}, {}", func.name, type_dir));
    cg.state().emit_fmt(format_args!("{}:", func.name));
    let emit_cfi = cg.state().emit_cfi;
    if emit_cfi {
        cg.state().emit(".cfi_startproc");
    }

    if emit_patchable {
        if let Some((total, before)) = cg.state().patchable_function_entry {
            let after = total.saturating_sub(before);
            for _ in 0..after {
                cg.state().emit("nop");
            }
        }
    }

    // Naked functions: emit only inline asm blocks, no prologue/epilogue/params.
    if func.is_naked {
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::InlineAsm { template, .. } = inst {
                    cg.emit_raw_inline_asm(template);
                }
            }
        }
        if emit_cfi {
            cg.state().emit(".cfi_endproc");
        }
        cg.state()
            .emit_fmt(format_args!(".size {}, .-{}", func.name, func.name));
        cg.state().emit("");
        return;
    }

    let has_dyn_alloca = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            matches!(
                inst,
                Instruction::DynAlloca { .. } | Instruction::StackRestore { .. }
            )
        })
    });
    cg.state().has_dyn_alloca = has_dyn_alloca;
    cg.state().uses_sret = func.uses_sret;

    let has_inline_asm_fp = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            if let Instruction::InlineAsm { template, .. } = inst {
                template_mentions_frame_pointer(template)
            } else {
                false
            }
        })
    });
    let has_vector_intrinsics = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            matches!(inst, Instruction::Intrinsic { op, .. }
            if matches!(op,
                crate::ir::intrinsics::IntrinsicOp::FmaF64x4
                | crate::ir::intrinsics::IntrinsicOp::FmaF64x2
                | crate::ir::intrinsics::IntrinsicOp::FmaF64x2Hoisted
                | crate::ir::intrinsics::IntrinsicOp::VecZeroF64x4
                | crate::ir::intrinsics::IntrinsicOp::VecZeroF64x2
                | crate::ir::intrinsics::IntrinsicOp::VecZeroF32x8
                | crate::ir::intrinsics::IntrinsicOp::VecZeroF32x4
                | crate::ir::intrinsics::IntrinsicOp::LoadF64x4
                | crate::ir::intrinsics::IntrinsicOp::LoadF64x2
            ))
        })
    });
    cg.state().omit_frame_pointer =
        !has_dyn_alloca && !func.is_variadic && !has_inline_asm_fp && !has_vector_intrinsics;

    cg.state().current_func_name = func.name.clone();
    let raw_space = cg.calculate_stack_space(func);
    let frame_size = cg.aligned_frame_size(raw_space);
    cg.state().frame_size = frame_size;
    cg.emit_prologue(func, frame_size);

    cg.emit_store_params(func);

    let entry_label = func.blocks.first().map(|b| b.label);

    let value_use_counts = count_value_uses(func);
    cg.state().value_use_counts = value_use_counts.clone();

    let gep_fold_map = build_gep_fold_map(func, &value_use_counts);

    let indexed_gep_map = if cg.supports_indexed_addr() {
        build_indexed_gep_map(func, &value_use_counts)
    } else {
        FxHashMap::default()
    };

    let global_addr_map = build_global_addr_map(
        func,
        &cg.state_ref().tls_symbols,
        Some(&cg.state_ref().absolute_symbols),
    );

    let global_addr_ptr_set = if cg.state_ref().code_model_kernel {
        build_global_addr_ptr_set(func)
    } else {
        FxHashSet::default()
    };

    let dead_global_addrs = if cg.supports_global_addr_fold() {
        build_foldable_global_addr_set(func, &global_addr_map)
    } else {
        FxHashSet::default()
    };

    let emit_debug = cg.state_ref().debug_info && source_mgr.is_some() && !file_table.is_empty();
    let mut last_debug_file: u32 = 0;
    let mut last_debug_line: u32 = 0;
    cg.state().current_program_point = 0;

    for (block_idx, block) in func.blocks.iter().enumerate() {
        if Some(block.label) != entry_label {
            cg.state().reg_cache.invalidate_all();
            cg.flush_pending_vec_store();
            cg.state().vec_last_store_slot = None;
            cg.state().vec_last_store_val = None;
            cg.state().vec_last_store_reg = false;
            cg.state().sse_last_store_slot = None;
            cg.state().sse_last_store_val = None;
            cg.state().sse_last_store_reg = false;
            if crate::pgo::block_align_active() {
                if let Some(log2) = crate::pgo::block_align(block.label.0) {
                    cg.state().emit_fmt(format_args!(".p2align {}", log2));
                }
            }
            cg.state().out.emit_block_label(block.label.0);
        }

        let fuse_idx =
            detect_cmp_branch_fusion(block, &value_use_counts, cg.supports_fused_fp_cmp_branch());
        let mul_add_fusions =
            detect_mul_add_fusions(block, &value_use_counts, cg.supports_fused_float_mul_add());
        let cmp_select_fusions = if cg.supports_fused_cmp_select() && !env_flag_set("CCC_NO_FUSED_CSEL")
        {
            detect_cmp_select_fusion(block, &value_use_counts)
        } else {
            Vec::new()
        };
        let shifted_logical_fusions = if cg.supports_shifted_logical() {
            detect_shifted_logical_fusions(block, &value_use_counts)
        } else {
            FxHashSet::default()
        };
        let mut fused_add_skip: FxHashSet<usize> = FxHashSet::default();
        let mut skip_fused_logical = false;

        cg.state().block_use_counts.clear();
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *cg.state().block_use_counts.entry(v.0).or_insert(0) += 1;
                }
            });
            match inst {
                Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } => {
                    *cg.state().block_use_counts.entry(ptr.0).or_insert(0) += 1;
                }
                Instruction::GetElementPtr { base, .. } => {
                    *cg.state().block_use_counts.entry(base.0).or_insert(0) += 1;
                }
                Instruction::Memcpy { dest, src, .. } => {
                    *cg.state().block_use_counts.entry(dest.0).or_insert(0) += 1;
                    *cg.state().block_use_counts.entry(src.0).or_insert(0) += 1;
                }
                Instruction::AtomicLoad { ptr, .. }
                | Instruction::AtomicStore { ptr, .. }
                | Instruction::AtomicRmw { ptr, .. }
                | Instruction::AtomicCmpxchg { ptr, .. } => {
                    if let Operand::Value(v) = ptr {
                        *cg.state().block_use_counts.entry(v.0).or_insert(0) += 1;
                    }
                }
                Instruction::AtomicInc { ptr, .. } => {
                    *cg.state().block_use_counts.entry(ptr.0).or_insert(0) += 1;
                }
                _ => {}
            }
        }

        for (idx, inst) in block.instructions.iter().enumerate() {
            if Some(idx) == fuse_idx.map(|f| f.0) {
                cg.state().current_program_point += 1;
                continue;
            }
            if fuse_idx
                .and_then(|f| f.1.map(|end| idx > f.0 && idx <= end))
                .unwrap_or(false)
            {
                cg.state().current_program_point += 1;
                continue;
            }
            if cmp_select_fusions
                .iter()
                .any(|&(_, ci, copy_i)| ci == idx || copy_i == Some(idx))
            {
                cg.state().current_program_point += 1;
                continue;
            }
            if fused_add_skip.contains(&idx) {
                cg.state().current_program_point += 1;
                continue;
            }
            if skip_fused_logical {
                skip_fused_logical = false;
                cg.state().current_program_point += 1;
                continue;
            }

            // Skip address producers whose result is folded into a later
            // Load/Store (const GEP, Add-as-GEP, indexed GEP) or into a
            // RIP-relative symbol access.
            if let Some(dest) = inst.dest() {
                if dead_global_addrs.contains(&dest.0) {
                    match inst {
                        Instruction::GlobalAddr { name, .. } => {
                            let skip = !cg.state_ref().needs_got_for_addr(name)
                                && !cg.state_ref().tls_symbols.contains(name.as_str())
                                && !cg.state_ref().absolute_symbols.contains(name.as_str());
                            if skip {
                                cg.state().current_program_point += 1;
                                continue;
                            }
                        }
                        Instruction::GetElementPtr { .. }
                        | Instruction::Copy { .. }
                        | Instruction::Cast { .. }
                        | Instruction::BinOp {
                            op: IrBinOp::Add, ..
                        } => {
                            cg.state().current_program_point += 1;
                            continue;
                        }
                        _ => {}
                    }
                }
            }

            if let Instruction::GetElementPtr { dest, base, .. } = inst {
                if gep_fold_map.contains_key(&dest.0)
                    && (cg.state_ref().is_alloca(base.0)
                        || cg.get_phys_reg_for_value(base.0).is_some())
                {
                    cg.state().folded_gep_values.insert(dest.0);
                    cg.state().current_program_point += 1;
                    continue;
                }
                if let Some(info) = indexed_gep_map.get(&dest.0) {
                    if cg.get_phys_reg_for_value(base.0).is_some()
                        && cg.get_phys_reg_for_value(info.index.0).is_some()
                    {
                        cg.state().folded_gep_values.insert(dest.0);
                        cg.state().current_program_point += 1;
                        continue;
                    }
                }
            }

            // Same skip for pointer `Add` that `build_gep_fold_map` accepted.
            // Previously the Add was still emitted (lea/add + spill) even
            // though every use was rewritten to a folded addressing mode.
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ..
            } = inst
            {
                if let Some(info) = gep_fold_map.get(&dest.0) {
                    let base_ok = cg.state_ref().is_alloca(info.base.0)
                        || cg.get_phys_reg_for_value(info.base.0).is_some();
                    // Confirm this really is the `base + const` shape.
                    let is_ptr_add = matches!(
                        (lhs, rhs),
                        (Operand::Value(_), Operand::Const(_))
                            | (Operand::Const(_), Operand::Value(_))
                    );
                    if base_ok && is_ptr_add {
                        cg.state().folded_gep_values.insert(dest.0);
                        cg.state().current_program_point += 1;
                        continue;
                    }
                }
            }

            if emit_debug {
                if let Some(span) = block.source_spans.get(idx) {
                    emit_loc_directive(
                        cg,
                        span,
                        source_mgr.expect("debug mode requires source manager"),
                        file_table,
                        &mut last_debug_file,
                        &mut last_debug_line,
                    );
                }
            }

            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Mul,
                lhs,
                rhs,
                ty,
            } = inst
            {
                if let Some(&add_i) = mul_add_fusions.get(&idx) {
                    if cg.get_phys_reg_for_value(dest.0).is_none() || ty.is_float() {
                        if let Some(Instruction::BinOp {
                            dest: add_dest,
                            lhs: add_lhs,
                            rhs: add_rhs,
                            ty: add_ty,
                            ..
                        }) = block.instructions.get(add_i)
                        {
                            let mul_is_lhs = matches!(add_lhs, Operand::Value(v) if v.0 == dest.0);
                            let acc_op = if mul_is_lhs { add_rhs } else { add_lhs };
                            // Flush so a buffered MachInst producer of `lhs`
                            // is materialised before the fused sequence reads it.
                            cg.flush_machinst();
                            cg.emit_fused_mul_add(dest, lhs, rhs, acc_op, add_dest, *add_ty);
                            fused_add_skip.insert(add_i);
                            cg.state().current_program_point += 1;
                            continue;
                        }
                    }
                }
            }

            if let Instruction::BinOp {
                dest: shift_dest,
                op: shift_op,
                lhs: shift_lhs,
                rhs: shift_amount,
                ty,
            } = inst
            {
                if shifted_logical_fusions.contains(&(idx + 1)) {
                    if let Some(Instruction::BinOp {
                        dest,
                        op: logical_op,
                        lhs,
                        rhs,
                        ..
                    }) = block.instructions.get(idx + 1)
                    {
                        let shift_is_lhs = matches!(lhs, Operand::Value(v) if v.0 == shift_dest.0);
                        let other = if shift_is_lhs { rhs } else { lhs };
                        cg.flush_machinst();
                        cg.emit_shifted_logical(
                            shift_dest,
                            *shift_op,
                            shift_lhs,
                            shift_amount,
                            *logical_op,
                            other,
                            dest,
                            *ty,
                        );
                        skip_fused_logical = true;
                        cg.state().current_program_point += 1;
                        continue;
                    }
                }
            }

            if let Some(&(_, ci, _)) = cmp_select_fusions.iter().find(|&&(si, _, _)| si == idx) {
                if let (
                    Instruction::Select {
                        dest,
                        true_val,
                        false_val,
                        ty: sel_ty,
                        ..
                    },
                    Instruction::Cmp {
                        op,
                        lhs,
                        rhs,
                        ty: cmp_ty,
                        ..
                    },
                ) = (inst, &block.instructions[ci])
                {
                    cg.flush_machinst();
                    cg.emit_fused_cmp_select(
                        *op, lhs, rhs, *cmp_ty, true_val, false_val, dest, *sel_ty,
                    );
                    cg.state().current_program_point += 1;
                    continue;
                }
            }

            if cg.try_lower_machinst(inst, &dead_global_addrs) {
                cg.state().current_program_point += 1;
                continue;
            }
            cg.flush_machinst();

            generate_instruction(
                cg,
                inst,
                &gep_fold_map,
                &indexed_gep_map,
                &global_addr_map,
                &global_addr_ptr_set,
                &dead_global_addrs,
            );
            cg.state().current_program_point += 1;
        }

        cg.flush_machinst();
        cg.flush_vecreg_liveout();

        cg.state().next_block_label = func.blocks.get(block_idx + 1).map(|b| b.label);
        if let Some((fi, _)) = fuse_idx {
            if let Instruction::Cmp {
                dest: _,
                op,
                lhs,
                rhs,
                ty,
            } = &block.instructions[fi]
            {
                if let Terminator::CondBranch {
                    cond: _,
                    true_label,
                    false_label,
                } = &block.terminator
                {
                    cg.emit_fused_cmp_branch_blocks(*op, lhs, rhs, *ty, *true_label, *false_label);
                }
            }
        } else {
            generate_terminator(cg, &block.terminator, frame_size, block.label.0);
        }
        cg.state().current_program_point += 1;
    }

    if emit_cfi {
        cg.state().emit(".cfi_endproc");
    }
    cg.state()
        .emit_fmt(format_args!(".size {}, .-{}", func.name, func.name));
    cg.state().emit("");
    cg.emit_vector_const_rodata();
    // `emit_vector_const_rodata` (and the FP const pool) switch to .rodata.
    // Restore the function section so a subsequent function without its own
    // `.section` directive cannot land in .rodata.
    emit_switch_to_section(cg, &func_sect);
}

/// Emit a `.loc` directive if the source location for this instruction differs
/// from the previously emitted location.
fn emit_loc_directive(
    cg: &mut dyn ArchCodegen,
    span: &Span,
    source_mgr: &SourceManager,
    file_table: &FxHashMap<String, u32>,
    last_file: &mut u32,
    last_line: &mut u32,
) {
    if span.start == 0 && span.end == 0 {
        return;
    }
    let loc = source_mgr.resolve_span(*span);
    if let Some(&dwarf_file_id) = file_table.get(&loc.file) {
        if dwarf_file_id != *last_file || loc.line != *last_line {
            cg.state().emit_fmt(format_args!(
                ".loc {} {} {}",
                dwarf_file_id, loc.line, loc.column
            ));
            *last_file = dwarf_file_id;
            *last_line = loc.line;
        }
    }
}

fn clobber_after_call_like(cg: &mut dyn ArchCodegen) {
    cg.state().reg_cache.invalidate_all();
    cg.flush_pending_vec_store();
    cg.state().invalidate_vec_peephole();
}

/// True if `sym` (possibly `name+off`) must not be used as a RIP-relative
/// memory operand: GOT-indirect, TLS, or absolute `.set` symbol.
fn rip_rel_blocked(cg: &dyn ArchCodegen, sym: &str) -> bool {
    let base = asm_symbol_basename(sym);
    cg.state_ref().needs_got_for_addr(base)
        || cg.state_ref().needs_got_for_addr(sym)
        || cg.state_ref().tls_symbols.contains(base)
        || cg.state_ref().absolute_symbols.contains(base)
}

/// Dispatch a single IR instruction to the appropriate arch method.
pub(super) fn generate_instruction(
    cg: &mut dyn ArchCodegen,
    inst: &Instruction,
    gep_fold_map: &FxHashMap<u32, GepFoldInfo>,
    indexed_gep_map: &FxHashMap<u32, IndexedGepInfo>,
    global_addr_map: &FxHashMap<u32, String>,
    global_addr_ptr_set: &FxHashSet<u32>,
    dead_global_addrs: &FxHashSet<u32>,
) {
    match inst {
        Instruction::PgoCounterInc {
            name,
            offset,
            atomic,
        } => {
            if env_flag_set("LCCC_PGO_NOP_COUNTERS") {
                cg.emit_pgo_counter_nop(name, *offset, *atomic);
            } else {
                cg.emit_pgo_counter_inc(name, *offset, *atomic);
            }
        }
        Instruction::Alloca { .. } => {}
        Instruction::Copy { dest, src } => {
            generate_copy(cg, dest, src);
        }

        Instruction::Load {
            dest,
            ptr,
            ty,
            seg_override,
        } => {
            generate_load(
                cg,
                dest,
                ptr,
                *ty,
                *seg_override,
                gep_fold_map,
                indexed_gep_map,
                global_addr_map,
            );
        }
        Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => {
            cg.emit_binop(dest, *op, lhs, rhs, *ty);
            if is_wide_int_type(*ty) {
                cg.state().reg_cache.invalidate_all();
            }
        }
        Instruction::UnaryOp { dest, op, src, ty } => {
            cg.emit_unaryop(dest, *op, src, *ty);
            if is_wide_int_type(*ty) {
                cg.state().reg_cache.invalidate_all();
            }
        }
        Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => {
            cg.emit_cmp(dest, *op, lhs, rhs, *ty);
        }
        Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        } => {
            cg.emit_cast(dest, src, *from_ty, *to_ty);
            if is_wide_int_type(*to_ty) || is_wide_int_type(*from_ty) {
                cg.state().reg_cache.invalidate_all();
            }
        }
        Instruction::GetElementPtr {
            dest, base, offset, ..
        } => {
            if let Operand::Value(off_val) = offset {
                cg.state()
                    .gep_base_offset
                    .insert(dest.0, (base.0, off_val.0));
            }
            cg.emit_gep(dest, base, offset);
        }
        Instruction::GlobalAddr { dest, name } => {
            let is_dead = dead_global_addrs.contains(&dest.0)
                && !cg.state_ref().needs_got_for_addr(name)
                && !cg.state_ref().tls_symbols.contains(name.as_str())
                && !cg.state_ref().absolute_symbols.contains(name.as_str());
            if !is_dead {
                if cg.state_ref().tls_symbols.contains(name.as_str()) {
                    cg.emit_tls_global_addr(dest, name);
                } else if cg.state_ref().absolute_symbols.contains(name.as_str())
                    || (cg.state_ref().code_model_kernel && !global_addr_ptr_set.contains(&dest.0))
                {
                    cg.emit_global_addr_absolute(dest, name);
                } else {
                    cg.emit_global_addr(dest, name);
                }
            }
        }
        Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        } => {
            cg.emit_select(dest, cond, true_val, false_val, *ty);
        }
        Instruction::LabelAddr { dest, label } => {
            cg.emit_label_addr(dest, &label.as_label());
        }

        Instruction::Store {
            val,
            ptr,
            ty,
            seg_override,
        } => {
            generate_store(
                cg,
                val,
                ptr,
                *ty,
                *seg_override,
                gep_fold_map,
                indexed_gep_map,
                global_addr_map,
            );
            clobber_after_call_like(cg);
        }
        Instruction::DynAlloca { dest, size, align } => {
            cg.emit_dyn_alloca(dest, size, *align);
            cg.state().protected_slot_values.insert(dest.0);
            clobber_after_call_like(cg);
        }
        Instruction::Call { func, info } => {
            // Inline fixed-size memcpy only. memmove is NOT inlined (overlap).
            // GATE ON BACKEND SUPPORT: the trait default of
            // emit_inline_memcpy_call is an EMPTY body.
            let inline_copy = cg.supports_inline_memcpy_call()
                && matches!(func.as_str(), "memcpy" | "__memcpy_chk")
                && info.args.len() >= 3
                && matches!(
                    info.args.get(2),
                    Some(Operand::Const(c)) if c.to_i64().is_some_and(|s| (0..=32).contains(&s))
                )
                && !info.is_variadic;
            if inline_copy {
                if let Some(Operand::Const(c)) = info.args.get(2) {
                    let size = c.to_i64().unwrap_or(0) as usize;
                    cg.emit_inline_memcpy_call(&info.args[0], &info.args[1], size);
                    // memcpy returns its destination pointer. Dropping that
                    // store miscompiles `p = memcpy(...)`.
                    if let Some(dest) = info.dest {
                        cg.emit_copy_value(&dest, &info.args[0]);
                    }
                    clobber_after_call_like(cg);
                    return;
                }
            }
            cg.emit_call(
                &info.args,
                &info.arg_types,
                Some(func),
                None,
                info.dest,
                info.return_type,
                info.is_variadic,
                info.num_fixed_args,
                &info.struct_arg_sizes,
                &info.struct_arg_aligns,
                &info.struct_arg_classes,
                &info.struct_arg_riscv_float_classes,
                &info.struct_arg_is_f128_sse,
                info.is_sret,
                info.is_fastcall,
                &info.ret_eightbyte_classes,
                info.ret_is_f128_sse,
            );
            clobber_after_call_like(cg);
        }
        Instruction::CallIndirect { func_ptr, info } => {
            cg.emit_call(
                &info.args,
                &info.arg_types,
                None,
                Some(func_ptr),
                info.dest,
                info.return_type,
                info.is_variadic,
                info.num_fixed_args,
                &info.struct_arg_sizes,
                &info.struct_arg_aligns,
                &info.struct_arg_classes,
                &info.struct_arg_riscv_float_classes,
                &info.struct_arg_is_f128_sse,
                info.is_sret,
                info.is_fastcall,
                &info.ret_eightbyte_classes,
                info.ret_is_f128_sse,
            );
            clobber_after_call_like(cg);
        }
        Instruction::Memcpy { dest, src, size } => {
            cg.emit_memcpy(dest, src, *size);
            clobber_after_call_like(cg);
        }
        Instruction::VaArg {
            dest,
            va_list_ptr,
            result_ty,
        } => {
            cg.emit_va_arg(dest, va_list_ptr, *result_ty);
            clobber_after_call_like(cg);
        }
        Instruction::VaStart { va_list_ptr } => {
            cg.emit_va_start(va_list_ptr);
            clobber_after_call_like(cg);
        }
        Instruction::VaEnd { va_list_ptr } => {
            cg.emit_va_end(va_list_ptr);
            clobber_after_call_like(cg);
        }
        Instruction::VaCopy { dest_ptr, src_ptr } => {
            cg.emit_va_copy(dest_ptr, src_ptr);
            clobber_after_call_like(cg);
        }
        Instruction::VaArgStruct {
            dest_ptr,
            va_list_ptr,
            size,
            ref eightbyte_classes,
        } => {
            cg.emit_va_arg_struct_ex(dest_ptr, va_list_ptr, *size, eightbyte_classes);
            clobber_after_call_like(cg);
        }
        Instruction::AtomicRmw {
            dest,
            op,
            ptr,
            val,
            ty,
            ordering,
        } => {
            cg.emit_atomic_rmw(dest, *op, ptr, val, *ty, *ordering);
            clobber_after_call_like(cg);
        }
        Instruction::AtomicInc {
            ptr,
            offset,
            ty,
            ordering,
        } => {
            cg.emit_atomic_inc(ptr, *offset, *ty, *ordering);
            clobber_after_call_like(cg);
        }
        Instruction::AtomicCmpxchg {
            dest,
            ptr,
            expected,
            desired,
            ty,
            success_ordering,
            failure_ordering,
            returns_bool,
        } => {
            cg.emit_atomic_cmpxchg(
                dest,
                ptr,
                expected,
                desired,
                *ty,
                *success_ordering,
                *failure_ordering,
                *returns_bool,
            );
            clobber_after_call_like(cg);
        }
        Instruction::AtomicLoad {
            dest,
            ptr,
            ty,
            ordering,
        } => {
            cg.emit_atomic_load(dest, ptr, *ty, *ordering);
            clobber_after_call_like(cg);
        }
        Instruction::AtomicStore {
            ptr,
            val,
            ty,
            ordering,
        } => {
            cg.emit_atomic_store(ptr, val, *ty, *ordering);
            clobber_after_call_like(cg);
        }
        Instruction::Fence { ordering } => {
            cg.emit_fence(*ordering);
            clobber_after_call_like(cg);
        }
        Instruction::Phi { .. } => { /* resolved before codegen */ }
        Instruction::GetReturnF64Second { dest } => {
            cg.emit_get_return_f64_second(dest);
            clobber_after_call_like(cg);
        }
        Instruction::SetReturnF64Second { src } => {
            cg.emit_set_return_f64_second(src);
            clobber_after_call_like(cg);
        }
        Instruction::GetReturnF32Second { dest } => {
            cg.emit_get_return_f32_second(dest);
            clobber_after_call_like(cg);
        }
        Instruction::SetReturnF32Second { src } => {
            cg.emit_set_return_f32_second(src);
            clobber_after_call_like(cg);
        }
        Instruction::GetReturnF128Second { dest } => {
            cg.emit_get_return_f128_second(dest);
            clobber_after_call_like(cg);
        }
        Instruction::SetReturnF128Second { src } => {
            cg.emit_set_return_f128_second(src);
            clobber_after_call_like(cg);
        }
        Instruction::InlineAsm {
            template,
            outputs,
            inputs,
            clobbers,
            operand_types,
            goto_labels,
            input_symbols,
            seg_overrides,
        } => {
            cg.emit_inline_asm_with_segs(
                template,
                outputs,
                inputs,
                clobbers,
                operand_types,
                goto_labels,
                input_symbols,
                seg_overrides,
            );
            clobber_after_call_like(cg);
        }
        Instruction::Intrinsic {
            dest,
            op,
            dest_ptr,
            args,
        } => {
            cg.emit_intrinsic(dest, op, dest_ptr, args);
            clobber_after_call_like(cg);
        }
        Instruction::StackSave { dest } => {
            cg.emit_stack_save(dest);
            clobber_after_call_like(cg);
        }
        Instruction::StackRestore { ptr } => {
            cg.emit_stack_restore(ptr);
            clobber_after_call_like(cg);
        }
        Instruction::ParamRef {
            dest,
            param_idx,
            ty,
        } => {
            cg.emit_param_ref(dest, *param_idx, *ty);
            clobber_after_call_like(cg);
        }
    }
}

/// Generate a Copy instruction, handling coalesced slots, i128, and wide values.
fn generate_copy(cg: &mut dyn ArchCodegen, dest: &Value, src: &Operand) {
    // When the source is an alloca, the Copy must materialize the alloca's
    // ADDRESS, not load a value from the alloca's slot or register.
    if let Operand::Value(src_val) = src {
        if cg.state_ref().is_alloca(src_val.0) {
            if let Some(addr) = cg.state_ref().resolve_slot_addr(src_val.0) {
                match addr {
                    crate::backend::state::SlotAddr::OverAligned(slot, id) => {
                        cg.emit_alloca_aligned_addr_to_acc(slot, id);
                    }
                    crate::backend::state::SlotAddr::Direct(slot) => {
                        cg.emit_gep_direct_const(slot, 0);
                    }
                    crate::backend::state::SlotAddr::Indirect(_) => {
                        unreachable!("alloca address cannot use an indirect slot");
                    }
                }
                cg.state().reg_cache.set_acc(src_val.0, true);
                cg.emit_store_result(dest);
                return;
            }
        }
    }

    // Skip Copy when dest and src share the same stack slot (from copy coalescing).
    // Valid ONLY when neither side is register-resident.
    if let Operand::Value(src_val) = src {
        if !cg.is_value_reg_assigned(dest.0) && !cg.is_value_reg_assigned(src_val.0) {
            let dest_slot = cg.state_ref().get_slot(dest.0);
            let src_slot = cg.state_ref().get_slot(src_val.0);
            if let (Some(ds), Some(ss)) = (dest_slot, src_slot) {
                if ds.0 == ss.0 {
                    if cg.state_ref().reg_cache.acc_has(src_val.0, false) {
                        cg.state().reg_cache.set_acc(dest.0, false);
                    }
                    return;
                }
            }
        }
    }

    let is_i128_copy = match src {
        Operand::Value(v) => cg.state_ref().is_i128_value(v.0),
        Operand::Const(IrConst::I128(_)) => true,
        _ => false,
    };
    if is_i128_copy {
        cg.state().i128_values.insert(dest.0);
        cg.emit_copy_i128(dest, src);
        cg.state().reg_cache.invalidate_all();
        return;
    }

    let is_wide = match src {
        Operand::Value(v) => cg.state_ref().is_wide_value(v.0),
        Operand::Const(IrConst::F64(_)) => crate::common::types::target_is_32bit(),
        Operand::Const(IrConst::I64(val)) => {
            crate::common::types::target_is_32bit()
                && (*val < i32::MIN as i64 || *val > u32::MAX as i64)
        }
        _ => false,
    };
    if is_wide {
        cg.state().wide_values.insert(dest.0);
    }
    cg.emit_copy_value(dest, src);
}

/// If an indexed fold was refused after the GEP was skipped, rematerialize
/// the address into `ptr` so the generic load/store path is well-defined.
fn rematerialize_skipped_indexed(
    cg: &mut dyn ArchCodegen,
    ptr: &Value,
    info: &IndexedGepInfo,
) {
    if cg.state_ref().folded_gep_values.contains(&ptr.0) {
        cg.emit_gep(ptr, &info.base, &Operand::Value(info.orig_offset));
        cg.state().folded_gep_values.remove(&ptr.0);
    }
}

/// Generate a Load instruction with segment override, kernel code model,
/// and GEP folding support.
fn generate_load(
    cg: &mut dyn ArchCodegen,
    dest: &Value,
    ptr: &Value,
    ty: IrType,
    seg_override: AddressSpace,
    gep_fold_map: &FxHashMap<u32, GepFoldInfo>,
    indexed_gep_map: &FxHashMap<u32, IndexedGepInfo>,
    global_addr_map: &FxHashMap<u32, String>,
) {
    if seg_override != AddressSpace::Default {
        if let Some(sym) = global_addr_map.get(&ptr.0) {
            if !rip_rel_blocked(cg, sym) {
                cg.emit_seg_load_symbol(dest, sym, ty, seg_override);
                return;
            }
        }
        cg.emit_seg_load(dest, ptr, ty, seg_override);
        return;
    }
    if cg.supports_global_addr_fold() && is_foldable_mem_ty(ty, AddressSpace::Default) {
        if let Some(sym) = global_addr_map.get(&ptr.0) {
            if !rip_rel_blocked(cg, sym) {
                cg.emit_global_load_rip_rel(dest, sym, ty);
                return;
            }
        }
    }
    if let Some(gep_info) = gep_fold_map.get(&ptr.0) {
        if !is_wide_int_type(ty)
            && (cg.state_ref().is_alloca(gep_info.base.0)
                || cg.get_phys_reg_for_value(gep_info.base.0).is_some())
        {
            cg.emit_load_with_const_offset(dest, &gep_info.base, gep_info.offset, ty);
            return;
        }
    }
    if let Some(info) = indexed_gep_map.get(&ptr.0) {
        if !is_wide_int_type(ty)
            && cg.get_phys_reg_for_value(info.base.0).is_some()
            && cg.get_phys_reg_for_value(info.index.0).is_some()
        {
            if cg.emit_load_indexed(dest, &info.base, &info.index, info.shift, ty) {
                return;
            }
            rematerialize_skipped_indexed(cg, ptr, info);
        }
    }
    cg.emit_load(dest, ptr, ty);
    if is_wide_int_type(ty) {
        cg.state().reg_cache.invalidate_all();
    }
}

/// Generate a Store instruction with segment override, kernel code model,
/// and GEP folding support.
fn generate_store(
    cg: &mut dyn ArchCodegen,
    val: &Operand,
    ptr: &Value,
    ty: IrType,
    seg_override: AddressSpace,
    gep_fold_map: &FxHashMap<u32, GepFoldInfo>,
    indexed_gep_map: &FxHashMap<u32, IndexedGepInfo>,
    global_addr_map: &FxHashMap<u32, String>,
) {
    if seg_override != AddressSpace::Default {
        if let Some(sym) = global_addr_map.get(&ptr.0) {
            if !rip_rel_blocked(cg, sym) {
                cg.emit_seg_store_symbol(val, sym, ty, seg_override);
                return;
            }
        }
        cg.emit_seg_store(val, ptr, ty, seg_override);
        return;
    }
    if cg.supports_global_addr_fold() && is_foldable_mem_ty(ty, AddressSpace::Default) {
        if let Some(sym) = global_addr_map.get(&ptr.0) {
            if !rip_rel_blocked(cg, sym) {
                cg.emit_global_store_rip_rel(val, sym, ty);
                return;
            }
        }
    }
    if let Some(gep_info) = gep_fold_map.get(&ptr.0) {
        if !is_wide_int_type(ty)
            && (cg.state_ref().is_alloca(gep_info.base.0)
                || cg.get_phys_reg_for_value(gep_info.base.0).is_some())
        {
            cg.emit_store_with_const_offset(val, &gep_info.base, gep_info.offset, ty);
            return;
        }
    }
    if let Some(info) = indexed_gep_map.get(&ptr.0) {
        if !is_wide_int_type(ty)
            && cg.get_phys_reg_for_value(info.base.0).is_some()
            && cg.get_phys_reg_for_value(info.index.0).is_some()
        {
            if cg.emit_store_indexed(val, &info.base, &info.index, info.shift, ty) {
                return;
            }
            rematerialize_skipped_indexed(cg, ptr, info);
        }
    }
    cg.emit_store(val, ptr, ty);
}

/// Dispatch a terminator to the appropriate arch method.
fn generate_terminator(
    cg: &mut dyn ArchCodegen,
    term: &Terminator,
    frame_size: i64,
    block_label: u32,
) {
    match term {
        Terminator::Return(val) => {
            cg.emit_return(val.as_ref(), frame_size);
        }
        Terminator::Branch(label) => {
            cg.emit_branch_to_block(*label);
        }
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => {
            crate::pgo::set_cond_fallthrough(crate::pgo::cond_fallthrough(block_label));
            cg.emit_cond_branch_blocks(cond, *true_label, *false_label);
            crate::pgo::set_cond_fallthrough(None);
        }
        Terminator::IndirectBranch { target, .. } => {
            cg.emit_indirect_branch(target);
        }
        Terminator::Switch {
            val,
            cases,
            default,
            ty,
        } => {
            crate::pgo::set_switch_hint(crate::pgo::switch_hint(block_label));
            cg.emit_switch(val, cases, default, *ty);
            crate::pgo::set_switch_hint(None);
        }
        Terminator::Unreachable => {
            cg.emit_unreachable();
        }
    }
}

/// Check if an IR type is a 128-bit integer type (I128 or U128).
pub fn is_i128_type(ty: IrType) -> bool {
    matches!(ty, IrType::I128 | IrType::U128)
}

/// Check if a type is "wide" — needs register-pair operations on the current target.
///
/// Only I128/U128 on all targets. On i686, I64/U64 BinOps are handled via
/// the i686-specific overrides. We don't include I64/U64 here because the
/// framework-level effects (disabling GEP folding, fused branches, cache
/// invalidation) would cause excessive overhead on widened I32 arithmetic.
pub fn is_wide_int_type(ty: IrType) -> bool {
    matches!(ty, IrType::I128 | IrType::U128)
}

pub use super::stack_layout::{
    calculate_stack_space_common, collect_inline_asm_callee_saved,
    collect_inline_asm_callee_saved_with_generic, filter_available_regs, find_param_alloca,
    run_regalloc_and_merge_clobbers,
};
