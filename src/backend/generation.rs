//! Module, function, and instruction generation dispatch.
//!
//! Arch-independent entry points (`generate_module` / `generate_function` /
//! `generate_instruction` / `generate_terminator`) that drive `ArchCodegen`.
//!
//! Addressing-mode contract
//! ------------------------
//! A pointer producer (GEP / `Add` / `Sub` / copy-of-those) may be *skipped*
//! only when every later Load/Store of that dest is guaranteed to fold, or
//! when `generate_load`/`generate_store` can rematerialize it. The skip set
//! is `state.folded_gep_values`. Hitting a folded dest on the generic path
//! without rematerializing is a miscompile (use of an uncomputed pointer).

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

/// GEP/`Add`/`Sub` with a constant offset foldable into a Load/Store.
#[derive(Debug, Clone, Copy)]
pub(super) struct GepFoldInfo {
    /// Ultimate base (after const-chain composition).
    pub(super) base: Value,
    /// Signed byte displacement, guaranteed to fit in `i32`.
    pub(super) offset: i64,
}

/// `GEP(base, idx<<shift)` foldable into `[base, index, lsl #shift]`.
#[derive(Debug, Clone, Copy)]
pub(super) struct IndexedGepInfo {
    pub(super) base: Value,
    pub(super) index: Value,
    /// Log2 scale in `0..=3`.
    pub(super) shift: u8,
    /// Original GEP offset operand, used to rematerialize if the backend
    /// refuses the indexed encoding after the GEP was skipped.
    pub(super) orig_offset: Value,
}

fn env_flag_set(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

/// Whether generic lowering may overwrite a vector/SSE scratch register.
///
/// Intrinsics are deliberately excluded: their consumers maintain a tightly
/// scoped forwarding chain of their own. Calls and other opaque operations
/// are conservatively classified as clobbers. Kept shared with deferred-store
/// analysis so eligibility and emission cannot drift.
pub(crate) fn instruction_may_clobber_vector_scratch(inst: &Instruction) -> bool {
    match inst {
        Instruction::Intrinsic { .. } => false,

        Instruction::Load { ty, .. }
        | Instruction::BinOp { ty, .. }
        | Instruction::UnaryOp { ty, .. }
        | Instruction::Cmp { ty, .. }
        | Instruction::ParamRef { ty, .. } => {
            ty.is_float() || ty.is_128bit() || ty.is_long_double()
        }
        Instruction::Cast { from_ty, to_ty, .. } => {
            from_ty.is_float()
                || from_ty.is_128bit()
                || from_ty.is_long_double()
                || to_ty.is_float()
                || to_ty.is_128bit()
                || to_ty.is_long_double()
        }

        // Copy lacks type information, and Select can absorb a floating-point
        // comparison during fused lowering. Treat both conservatively.
        Instruction::Copy { .. }
        | Instruction::Select { .. }
        | Instruction::DynAlloca { .. }
        | Instruction::Store { .. }
        | Instruction::Call { .. }
        | Instruction::CallIndirect { .. }
        | Instruction::InlineAsm { .. }
        | Instruction::Memcpy { .. }
        | Instruction::VaArg { .. }
        | Instruction::VaArgStruct { .. }
        | Instruction::VaStart { .. }
        | Instruction::VaEnd { .. }
        | Instruction::VaCopy { .. }
        | Instruction::AtomicRmw { .. }
        | Instruction::AtomicCmpxchg { .. }
        | Instruction::AtomicLoad { .. }
        | Instruction::AtomicStore { .. }
        | Instruction::AtomicInc { .. }
        | Instruction::Fence { .. }
        | Instruction::GetReturnF64Second { .. }
        | Instruction::SetReturnF64Second { .. }
        | Instruction::GetReturnF32Second { .. }
        | Instruction::SetReturnF32Second { .. }
        | Instruction::GetReturnF128Second { .. }
        | Instruction::SetReturnF128Second { .. } => true,

        // These lower entirely through integer/address machinery or emit no
        // machine operation at their IR position.
        Instruction::Alloca { .. }
        | Instruction::GetElementPtr { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::PgoCounterInc { .. }
        | Instruction::Phi { .. }
        | Instruction::LabelAddr { .. }
        | Instruction::StackSave { .. }
        | Instruction::StackRestore { .. } => false,
    }
}

/// Strip every trailing `+digits` / `-digits` suffix (`foo+4+8` → `foo`).
fn asm_symbol_basename(sym: &str) -> &str {
    let mut s = sym;
    loop {
        let mut next = s;
        if let Some(i) = next.rfind('+') {
            if i > 0 && !next[i + 1..].is_empty() && next[i + 1..].bytes().all(|b| b.is_ascii_digit())
            {
                next = &next[..i];
            }
        }
        if let Some(i) = next.rfind('-') {
            if i > 0
                && !next[i + 1..].is_empty()
                && next[i + 1..].bytes().all(|b| b.is_ascii_digit())
            {
                next = &next[..i];
            }
        }
        if next.len() == s.len() {
            return s;
        }
        s = next;
    }
}

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

/// Displacement every backend can encode as a 32-bit signed mem operand.
///
/// `allow_u32_wrap` recovers `U32 0xFFFF_FFFF → -1` only for ≤32-bit adds.
/// Doing this for a 64-bit add would turn `base + 4294967295` into `base-1`.
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

fn is_i32_disp(off: i64) -> bool {
    (i32::MIN as i64..=i32::MAX as i64).contains(&off)
}

fn is_foldable_mem_ty(ty: IrType) -> bool {
    !is_wide_int_type(ty) && ty != IrType::F128
}

/// True iff `sym` (possibly `name+off`) must not be a RIP/PC-relative mem op.
fn rip_rel_blocked(cg: &dyn ArchCodegen, sym: &str) -> bool {
    let base = asm_symbol_basename(sym);
    cg.state_ref().needs_got_for_addr(base)
        || cg.state_ref().tls_symbols.contains(base)
        || cg.state_ref().absolute_symbols.contains(base)
}

/// Drop identities that cannot legally become a RIP/PC-relative mem operand.
/// Must run before [`build_foldable_global_addr_set`]: otherwise a skipped
/// producer + refused RIP-rel is a use of an uncomputed pointer.
fn filter_rip_legal_symbols(cg: &dyn ArchCodegen, map: &mut FxHashMap<u32, String>) {
    map.retain(|_, sym| !rip_rel_blocked(cg, sym));
}

fn can_const_addr_fold(cg: &dyn ArchCodegen, info: &GepFoldInfo) -> bool {
    cg.state_ref().is_alloca(info.base.0) || cg.get_phys_reg_for_value(info.base.0).is_some()
}

fn can_indexed_addr_fold(cg: &dyn ArchCodegen, info: &IndexedGepInfo) -> bool {
    cg.get_phys_reg_for_value(info.base.0).is_some()
        && cg.get_phys_reg_for_value(info.index.0).is_some()
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

/// Unique defining instruction of each single-def value. Multi-def (post-phi)
/// values are omitted — inspecting a non-unique def is unsafe.
fn index_single_defs<'a>(func: &'a IrFunction, stab: &BaseStability) -> FxHashMap<u32, &'a Instruction> {
    let mut index = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                if stab.def_count.get(&d.0).copied() == Some(1) {
                    index.insert(d.0, inst);
                }
            }
        }
    }
    index
}

/// Adjacent `(addr_producer, Load/Store-of-that-addr)` pairs whose producer
/// is single-use. Only the producer dest is proven stable — NOT aliases
/// introduced by later copy-propagation (`GEP; redef base; Copy; Load`).
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

/// Flatten `GEP/Add/Sub` const chains into one displacement. Overflow refuses
/// the hop (the inner producer stays as the base). Bounded to prevent cycles.
fn compose_const_gep_folds(map: &mut FxHashMap<u32, GepFoldInfo>) {
    if map.is_empty() {
        return;
    }
    let keys: Vec<u32> = map.keys().copied().collect();
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
            if !is_i32_disp(off) {
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

/// Copy / same-size integer-or-pointer Cast of an *already-stable* fold is
/// the same address. Must run AFTER the stability retain so a `Copy; Load`
/// adjacency cannot launder a multi-def base.
fn propagate_stable_aliases<T: Copy>(
    func: &IrFunction,
    stab: &BaseStability,
    map: &mut FxHashMap<u32, T>,
) {
    if map.is_empty() {
        return;
    }
    for _ in 0..=map.len() {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let (dest, src) = match inst {
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    } => (*dest, *src),
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(src),
                        from_ty,
                        to_ty,
                        ..
                    } if from_ty.size() == to_ty.size()
                        && !from_ty.is_float()
                        && !to_ty.is_float()
                        && !from_ty.is_long_double()
                        && !to_ty.is_long_double() =>
                    {
                        (*dest, *src)
                    }
                    _ => continue,
                };
                if stab.def_count.get(&dest.0).copied() != Some(1) || map.contains_key(&dest.0) {
                    continue;
                }
                if let Some(&info) = map.get(&src.0) {
                    map.insert(dest.0, info);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

fn retain_stable_bases(
    func: &IrFunction,
    use_counts: &[u32],
    stab: &BaseStability,
    map: &mut FxHashMap<u32, GepFoldInfo>,
) {
    let cand: FxHashSet<u32> = map.keys().copied().collect();
    let adjacent = adjacent_addr_producers(func, use_counts, &cand);
    map.retain(|dest, info| base_is_fold_stable(stab, info.base.0, *dest, &adjacent));
}

fn retain_stable_indexed(
    func: &IrFunction,
    use_counts: &[u32],
    stab: &BaseStability,
    map: &mut FxHashMap<u32, IndexedGepInfo>,
) {
    let cand: FxHashSet<u32> = map.keys().copied().collect();
    let adjacent = adjacent_addr_producers(func, use_counts, &cand);
    map.retain(|dest, info| base_is_fold_stable(stab, info.base.0, *dest, &adjacent));
}

/// Drop candidates that have a use which is not a foldable Load/Store ptr
/// and not an absorbed base of another still-foldable producer. Fixed-point
/// so invalidating a consumer re-exposes its base as a real use.
fn retain_ptr_only_uses(func: &IrFunction, map: &mut FxHashMap<u32, GepFoldInfo>) {
    loop {
        let mut bad: FxHashSet<u32> = FxHashSet::default();
        let mut mark = |id: u32| {
            if map.contains_key(&id) {
                bad.insert(id);
            }
        };
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Load { ptr, ty, seg_override, .. } => {
                        if !is_foldable_mem_ty(*ty) || *seg_override != AddressSpace::Default {
                            mark(ptr.0);
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
                            mark(v.0);
                        }
                        if !is_foldable_mem_ty(*ty) || *seg_override != AddressSpace::Default {
                            mark(ptr.0);
                        }
                    }
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => {
                        if map.contains_key(&dest.0) {
                            if let Operand::Value(v) = offset {
                                mark(v.0);
                            }
                        } else {
                            mark(base.0);
                            if let Operand::Value(v) = offset {
                                mark(v.0);
                            }
                        }
                    }
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add | IrBinOp::Sub,
                        ..
                    } if map.contains_key(&dest.0) => {}
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(_),
                    } if map.contains_key(&dest.0) => {}
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(_),
                        ..
                    } if map.contains_key(&dest.0) => {}
                    _ => {
                        for_each_operand_in_instruction(inst, |op| {
                            if let Operand::Value(v) = op {
                                mark(v.0);
                            }
                        });
                        for_each_value_use_in_instruction(inst, |v| mark(v.0));
                    }
                }
            }
            for_each_operand_in_terminator(&block.terminator, |op| {
                if let Operand::Value(v) = op {
                    mark(v.0);
                }
            });
        }
        let before = map.len();
        for id in bad {
            map.remove(&id);
        }
        if map.len() == before {
            break;
        }
    }
}

fn retain_indexed_ptr_only_uses(func: &IrFunction, map: &mut FxHashMap<u32, IndexedGepInfo>) {
    let mut bad: FxHashSet<u32> = FxHashSet::default();
    let mut mark = |id: u32| {
        if map.contains_key(&id) {
            bad.insert(id);
        }
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load { ptr, ty, seg_override, .. } => {
                    if !is_foldable_mem_ty(*ty) || *seg_override != AddressSpace::Default {
                        mark(ptr.0);
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
                        mark(v.0);
                    }
                    if !is_foldable_mem_ty(*ty) || *seg_override != AddressSpace::Default {
                        mark(ptr.0);
                    }
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Value(_),
                } if map.contains_key(&dest.0) => {}
                Instruction::Cast {
                    dest,
                    src: Operand::Value(_),
                    ..
                } if map.contains_key(&dest.0) => {}
                _ => {
                    for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            mark(v.0);
                        }
                    });
                    for_each_value_use_in_instruction(inst, |v| mark(v.0));
                }
            }
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                mark(v.0);
            }
        });
    }
    for id in bad {
        map.remove(&id);
    }
}

fn retain_used(map: &mut FxHashMap<u32, impl Sized>, use_counts: &[u32]) {
    map.retain(|val_id, _| {
        (*val_id as usize) < use_counts.len() && use_counts[*val_id as usize] > 0
    });
}

/// Variable-offset GEPs foldable into indexed addressing.
fn build_indexed_gep_map(
    func: &IrFunction,
    use_counts: &[u32],
    stab: &BaseStability,
) -> FxHashMap<u32, IndexedGepInfo> {
    if env_flag_set("CCC_NO_GEP_FOLD") {
        return FxHashMap::default();
    }

    let defs = index_single_defs(func, stab);

    // Resolve offset → (index, shift). Multi-def offsets are a plain index
    // (shift 0); we refuse to inspect a non-unique definition.
    let resolve_index = |off_id: u32| -> Option<(Value, u8)> {
        let mut cur = defs.get(&off_id).copied();
        loop {
            match cur {
                Some(Instruction::Cast {
                    src: Operand::Value(v),
                    from_ty,
                    to_ty,
                    ..
                }) if from_ty.is_integer()
                    && to_ty.is_integer()
                    && to_ty.size() >= from_ty.size() =>
                {
                    cur = defs.get(&v.0).copied();
                }
                _ => break,
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
            _ => Some((Value(off_id), 0)),
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

    retain_indexed_ptr_only_uses(func, &mut map);
    retain_stable_indexed(func, use_counts, stab, &mut map);
    propagate_stable_aliases(func, stab, &mut map);
    retain_indexed_ptr_only_uses(func, &mut map);
    retain_used(&mut map, use_counts);
    map
}

/// Const-offset GEP / `Add` / `Sub` destinations foldable into Load/Store.
fn build_gep_fold_map(
    func: &IrFunction,
    use_counts: &[u32],
    stab: &BaseStability,
) -> FxHashMap<u32, GepFoldInfo> {
    if env_flag_set("CCC_NO_GEP_FOLD") {
        return FxHashMap::default();
    }
    let mut gep_map: FxHashMap<u32, GepFoldInfo> = FxHashMap::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(c),
                    ..
                } => {
                    // Pointer-width GEP: never wrap a 64-bit `+4294967295`.
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
                    let Some(offset_val) = foldable_const_disp(c, ty.size() <= 4) else {
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
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(base),
                    rhs: Operand::Const(c),
                    ty,
                } => {
                    if ty.is_float() || ty.is_long_double() {
                        continue;
                    }
                    let Some(pos) = foldable_const_disp(c, ty.size() <= 4) else {
                        continue;
                    };
                    let Some(offset_val) = 0i64.checked_sub(pos) else {
                        continue;
                    };
                    if !is_i32_disp(offset_val) {
                        continue;
                    }
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
    retain_stable_bases(func, use_counts, stab, &mut gep_map);
    propagate_stable_aliases(func, stab, &mut gep_map);
    retain_ptr_only_uses(func, &mut gep_map);
    retain_used(&mut gep_map, use_counts);

    if env_flag_set("CCC_DEBUG_GEPFOLD") {
        eprintln!(
            "[GEPFOLD] total_candidates={} remaining={}",
            original_count,
            gep_map.len()
        );
    }
    gep_map
}

pub fn build_global_addr_map_for(
    func: &IrFunction,
    tls_symbols: &FxHashSet<String>,
) -> FxHashMap<u32, String> {
    build_global_addr_map(func, tls_symbols, None)
}

pub fn build_foldable_global_addr_set_for(
    func: &IrFunction,
    global_addr_map: &FxHashMap<u32, String>,
) -> FxHashSet<u32> {
    build_foldable_global_addr_set(func, global_addr_map)
}

/// `GlobalAddr` → `"name"`, plus `GEP`/`Add`/`Sub`/`Copy`/same-size Cast of
/// those → `"name+offset"`. TLS and absolute (`.set sym, <imm>`) symbols are
/// excluded: they must not become RIP-relative memory operands.
fn build_global_addr_map(
    func: &IrFunction,
    tls_symbols: &FxHashSet<String>,
    absolute_symbols: Option<&FxHashSet<String>>,
) -> FxHashMap<u32, String> {
    let excluded = |name: &str| {
        tls_symbols.contains(name) || absolute_symbols.is_some_and(|s| s.contains(name))
    };

    let mut map: FxHashMap<u32, String> = FxHashMap::default();
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
    let single = |id: u32| def_count.get(&id).copied() == Some(1);

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
                } if single(dest.0) => {
                    if let Some(off) = foldable_const_disp(c, false) {
                        const_off_edges.push((dest.0, base.0, off));
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
                } if !ty.is_float() && !ty.is_long_double() && single(dest.0) => {
                    if let Some(off) = foldable_const_disp(c, ty.size() <= 4) {
                        const_off_edges.push((dest.0, base.0, off));
                    }
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(base),
                    rhs: Operand::Const(c),
                    ty,
                } if !ty.is_float() && !ty.is_long_double() && single(dest.0) => {
                    if let Some(pos) = foldable_const_disp(c, ty.size() <= 4) {
                        if let Some(off) = 0i64.checked_sub(pos) {
                            if is_i32_disp(off) {
                                const_off_edges.push((dest.0, base.0, off));
                            }
                        }
                    }
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } if single(dest.0) => {
                    copy_edges.push((dest.0, src.0));
                }
                Instruction::Cast {
                    dest,
                    src: Operand::Value(src),
                    from_ty,
                    to_ty,
                    ..
                } if single(dest.0)
                    && from_ty.size() == to_ty.size()
                    && !from_ty.is_float()
                    && !to_ty.is_float() =>
                {
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

/// Values whose *every* use is a RIP-foldable Load/Store ptr (or the absorbed
/// base of another still-dead derived address). The incoming map must already
/// have GOT/TLS/abs symbols stripped.
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
                    Instruction::Load { ptr, ty, .. } => {
                        // Segment-overridden loads of a mapped symbol still
                        // fold (`emit_seg_load_symbol`); they do not need lea.
                        let foldable = live.contains(&ptr.0)
                            && global_addr_map.contains_key(&ptr.0)
                            && is_foldable_mem_ty(*ty);
                        if !foldable {
                            mark(ptr.0, &mut bad);
                        }
                    }
                    Instruction::Store { val, ptr, ty, .. } => {
                        let foldable = live.contains(&ptr.0)
                            && global_addr_map.contains_key(&ptr.0)
                            && is_foldable_mem_ty(*ty);
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
                        } else {
                            mark(base.0, &mut bad);
                            if let Operand::Value(v) = offset {
                                mark(v.0, &mut bad);
                            }
                        }
                    }
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add | IrBinOp::Sub,
                        ..
                    } if live.contains(&dest.0) => {}
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

/// GlobalAddr ids that flow to a memory pointer (kernel code model: those
/// need RIP-relative addressing; integer-only uses want R_X86_64_32S).
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
                Instruction::Copy { dest, src } => track_op(dest.0, src, &mut edges),
                Instruction::Cast { dest, src, .. } => track_op(dest.0, src, &mut edges),
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add | IrBinOp::Sub,
                    lhs,
                    rhs,
                    ..
                } => {
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
                    if let Operand::Value(v) = ptr {
                        ptr_uses.push(v.0);
                    }
                }
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
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

/// Use counts indexed by Value ID. Sized from both defs *and* uses so an
/// out-of-range operand cannot silently report 0.
fn count_value_uses(func: &IrFunction) -> Vec<u32> {
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

/// Cmp (optionally through Copy / integer Cast) used only by CondBranch.
fn detect_cmp_branch_fusion(
    block: &BasicBlock,
    use_counts: &[u32],
    fuse_fp: bool,
) -> Option<(usize, Option<usize>)> {
    let cond = match &block.terminator {
        Terminator::CondBranch { cond, .. } => cond,
        _ => return None,
    };
    let cond_val = match cond {
        Operand::Value(v) => v,
        _ => return None,
    };

    // Cmp → [Copy | integer Cast]* → CondBranch. Cmp is exactly 0/1, so
    // integer width/sign casts preserve truth value (i8 → int promotions).
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

    let (cmp_dest, ty) = match &block.instructions[cmp_idx] {
        Instruction::Cmp { dest, ty, .. } => (dest.0, ty),
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

    if is_wide_int_type(*ty) {
        return None;
    }
    // F32/F64 only, and only when the backend's fused CC matches cset
    // (AArch64). F128/long-double is a libcall; a d-reg fcmp reads half.
    if ty.is_float() && !(fuse_fp && matches!(ty, IrType::F32 | IrType::F64)) {
        return None;
    }
    if crate::common::types::target_is_32bit() && matches!(ty, IrType::I64 | IrType::U64) {
        return None;
    }
    Some((cmp_idx, chain_end))
}

/// `rhs` is a zero constant of any integer width (or the canonical Zero).
pub(crate) fn cmp_rhs_is_zero(rhs: &Operand) -> bool {
    matches!(rhs, Operand::Const(IrConst::Zero))
        || matches!(
            rhs,
            Operand::Const(IrConst::I8(0) | IrConst::I16(0) | IrConst::I32(0))
        )
}

/// The immediate a `Cmp { Eq|Ne }` may be folded with a sub-word load into
/// `cmp{b,w} $imm, (mem)`. For `Eq`/`Ne` the memory compare agrees with the
/// load+register-compare exactly on ZF whenever the constant is in a range
/// where both sign- and zero-extending the sub-word value keep `== imm` and
/// `byte/word == imm` equivalent: `[0, 127]` for bytes, `[0, 32767]` for
/// words (the constant is then non-negative, so a match can only occur for
/// values whose extension equals the value itself). Full 32-bit loads accept
/// any i32. Returns `None` when the operand is not such a constant.
pub(crate) fn cmp_fold_imm(rhs: &Operand, load_ty: IrType) -> Option<i64> {
    let v = match rhs {
        Operand::Const(IrConst::Zero) => 0,
        Operand::Const(IrConst::I8(v)) => *v as i64,
        Operand::Const(IrConst::I16(v)) => *v as i64,
        Operand::Const(IrConst::I32(v)) => *v as i64,
        _ => return None,
    };
    match load_ty {
        IrType::I8 | IrType::U8 => (0..128).contains(&v).then_some(v),
        IrType::I16 | IrType::U16 => (0..32768).contains(&v).then_some(v),
        IrType::I32 | IrType::U32 | IrType::Ptr => Some(v),
        _ => None,
    }
}

/// Adjacent `load; cmp { Eq | Ne, rhs = imm }` pairs where the load's single
/// use is the compare. The backend folds these into `cmpb/cmpw/cmpl $imm,
/// (mem)`, eliminating a sub-word load + register-compare pair (and the
/// register that held the loaded value, which on the accumulator-based i686
/// backend is the dominant string-loop bloat vs GCC). Returns a map from the
/// Load's dest value id to (pointer value id, loaded type, compare immediate).
///
/// Soundness: for `Eq`/`Ne` only the ZF flag is consumed, and
/// `cmp{b,w,l} $imm, (mem)` agrees exactly with the folded `movX + cmpl` on ZF
/// for the immediates admitted by `cmp_fold_imm` (see there). The adjacency
/// requirement guarantees the pointer is still live at the compare site (no
/// instruction can redefine it in between), and `use_count == 1` guarantees no
/// other consumer reads the skipped load's value.
fn detect_load_cmp_mem_fold(
    block: &BasicBlock,
    use_counts: &[u32],
) -> FxHashMap<u32, (u32, IrType, i64)> {
    use crate::ir::reexports::IrCmpOp;
    let mut out = FxHashMap::default();
    let n = block.instructions.len();
    for i in 0..n.saturating_sub(1) {
        let (load_dest, ptr, ty, seg) = match &block.instructions[i] {
            Instruction::Load {
                dest,
                ptr,
                ty,
                seg_override,
            } => (dest.0, ptr.0, *ty, *seg_override),
            _ => continue,
        };
        if seg != AddressSpace::Default {
            continue;
        }
        // Foldable widths map 1:1 onto cmpb/cmpw/cmpl; anything else (wide
        // pairs, floats, vectors, i128) is excluded.
        match ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32 | IrType::Ptr => {}
            _ => continue,
        }
        // The load's only consumer is the compare.
        if use_counts.get(load_dest as usize).copied().unwrap_or(u32::MAX) != 1 {
            continue;
        }
        let (op, lhs, rhs, cmp_ty) = match &block.instructions[i + 1] {
            Instruction::Cmp {
                op, lhs, rhs, ty, ..
            } => (*op, lhs, rhs, *ty),
            _ => continue,
        };
        if !matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
            continue;
        }
        if !matches!(lhs, Operand::Value(v) if v.0 == load_dest) {
            continue;
        }
        let Some(imm) = cmp_fold_imm(rhs, ty) else { continue };
        match cmp_ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32 | IrType::Ptr => {}
            _ => continue,
        }
        out.insert(load_dest, (ptr, ty, imm));
    }
    out
}

/// Adjacent `cmp; select` / `cmp; copy|zcast; select`.
/// Returns `(select_idx, cmp_idx, dead_mid_idx)`.
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
        if sel_ty.is_long_double() || sel_ty.is_128bit() || sel_idx == 0 {
            continue;
        }
        // Adjacent only: re-emitting the Cmp at the Select clobbers any
        // scratch the intervening ops reused (pgo_sections k-87 bug).
        let mut cmp_idx = None;
        let mut mid_idx = None;
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
        if crate::common::types::target_is_32bit() && matches!(cmp_ty, IrType::I64 | IrType::U64) {
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

/// Mul whose single use is a nearby Add. Map: mul_idx → add_idx.
fn detect_mul_add_fusions(
    block: &BasicBlock,
    use_counts: &[u32],
    fuse_float: bool,
) -> FxHashMap<usize, usize> {
    let mut fusion_map: FxHashMap<usize, usize> = FxHashMap::default();
    // `a*b + c*d` must not fuse the first mul with an add whose other
    // operand is a not-yet-executed mul.
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
        if use_counts.get(mul_dest.0 as usize).copied().unwrap_or(0) != 1 {
            continue;
        }

        // Fused sequence is emitted AT the mul, before skipped insts run.
        // Skipped Copy/Cast must not define an Add operand (nbody energy()).
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
        if defined_between(add_lhs) || defined_between(add_rhs) || mul_ty != add_ty {
            continue;
        }
        claimed_adds.insert(next_idx);
        fusion_map.insert(idx, next_idx);
    }
    fusion_map
}

/// Adjacent `shift-imm; logical` that AArch64 encodes as one instruction.
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
        let Some(k) = shift_amt.to_i64() else { continue };
        let max_shift = if matches!(shift_ty, IrType::I64 | IrType::U64) {
            63
        } else {
            31
        };
        if !(0..=max_shift).contains(&k) {
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

fn function_text_section(func: &IrFunction, function_sections: bool) -> String {
    if let Some(ref sect) = func.section {
        if !sect.is_empty() {
            return sect.clone();
        }
    }
    if function_sections {
        format!(".text.{}", func.name)
    } else {
        ".text".to_string()
    }
}

fn emit_switch_to_section(cg: &mut dyn ArchCodegen, sect: &str) {
    let sect = if sect.is_empty() { ".text" } else { sect };
    if cg.state_ref().current_text_section == sect {
        return;
    }
    if sect == ".text" {
        cg.state().emit(".text");
    } else {
        cg.state()
            .emit_fmt(format_args!(".section {},\"ax\",@progbits", sect));
    }
    cg.state().current_text_section = sect.to_string();
}

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

    // Top-level asm("...") (e.g. musl `_start`). Switch to .text first so
    // labels/code land in the correct section.
    if !module.toplevel_asm.is_empty() {
        cg.state().emit(".text");
        for asm_str in &module.toplevel_asm {
            cg.state().emit(asm_str);
        }
    }

    // Numeric `.set sym, <number>` → absolute symbols (glibc `_NL_CURRENT_DEFINE`).
    // Addresses are link-time constants and must never go through the GOT.
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

    cg.emit_runtime_stubs();
    cg.state().emit_fp_const_pool();

    cg.state().emit("");
    cg.state().emit(".section .note.GNU-stack,\"\",@progbits");

    std::mem::take(&mut cg.state().out.buf)
}

/// Conservative recogniser for numeric `.set` RHSes (`42`, `-1`, `0x1f`).
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

fn build_and_emit_dwarf_file_table(
    cg: &mut dyn ArchCodegen,
    module: &IrModule,
    source_mgr: Option<&crate::common::source::SourceManager>,
) -> FxHashMap<String, u32> {
    if !cg.state_ref().debug_info {
        return FxHashMap::default();
    }
    let Some(sm) = source_mgr else {
        return FxHashMap::default();
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
        cg.state().emit(".text");
        cg.state().current_text_section = ".text".to_string();
    }
    for func in &module.functions {
        // GNU89/gnu_inline: `extern inline __attribute__((gnu_inline))`
        // bodies exist ONLY for inlining. Match GCC: never emit a standalone
        // copy (glibc _FORTIFY_SOURCE wrappers lose va_arg_pack otherwise).
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
        // Alias name is already asm-resolved by the lowerer. Resolving it
        // again corrupts glibc hidden_ver (`__strdup` → self-alias).
        // The TARGET may still need asm resolution.
        let target_resolved = module.asm_labels.get(target_name).unwrap_or(target_name);
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

fn emit_symver_directives(cg: &mut dyn ArchCodegen, module: &IrModule) {
    for (func_name, symver_str) in &module.symver_directives {
        cg.state()
            .emit_fmt(format_args!(".symver {},{}", func_name, symver_str));
    }
}

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

/// Frame-pointer names that inline asm may capture. Bare `"fp"` is *not*
/// listed: it matches `fpsr` / `__fp16` and wrongly forces a frame pointer.
fn template_mentions_frame_pointer(template: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "%rbp", "{rbp}", "%%rbp", "%ebp", "{ebp}", "%%ebp", "x29", "{x29}", "%x29", "%fp",
        "{fp}", "%%fp",
    ];
    NEEDLES.iter().any(|n| template.contains(n))
}

fn clobber_is_frame_pointer(c: &str) -> bool {
    let l = c.trim().trim_start_matches('%').to_ascii_lowercase();
    matches!(l.as_str(), "rbp" | "ebp" | "x29" | "fp")
}

/// `__memcpy_chk(dest, src, n, destlen)` may only be inlined when the
/// fortify destlen covers `n` (or is `(size_t)-1` / SIZE_MAX).
fn destlen_covers_n(destlen: &IrConst, n: i64) -> bool {
    match destlen.to_i64() {
        Some(-1) => true,
        Some(d) if d >= n => true,
        _ => false,
    }
}

/// `Some(n)` if this call is a fixed-size memcpy we may expand inline.
fn inline_memcpy_len(func: &str, args: &[Operand], is_variadic: bool) -> Option<usize> {
    if is_variadic || args.len() < 3 {
        return None;
    }
    let n = match args.get(2) {
        Some(Operand::Const(c)) => c.to_i64().filter(|s| (0..=32).contains(s))?,
        _ => return None,
    };
    match func {
        "memcpy" => Some(n as usize),
        "__memcpy_chk" => {
            if args.len() < 4 {
                return None;
            }
            match args.get(3) {
                Some(Operand::Const(d)) if destlen_covers_n(d, n) => Some(n as usize),
                _ => None,
            }
        }
        _ => None,
    }
}

fn generate_function(
    cg: &mut dyn ArchCodegen,
    func: &IrFunction,
    source_mgr: Option<&SourceManager>,
    file_table: &FxHashMap<String, u32>,
) {
    // Derive the section from the function, not from `current_text_section`.
    // `reset_for_function` may clear that field to `""`, and restoring it
    // would emit `.section ,"ax",@progbits`.
    let function_sections = cg.state_ref().function_sections;
    let func_sect = function_text_section(func, function_sections);

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

    cg.state()
        .emit_linkage(&func.name, func.is_static, func.is_weak);
    cg.state().emit_visibility(&func.name, &func.visibility);

    // -fpatchable-function-entry=N,M. Skip for inline functions: emitting
    // __patchable_function_entries for every static inline from a header
    // overwhelms the kernel's ftrace initialization (~1400 vs ~5).
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
            if let Instruction::InlineAsm {
                template, clobbers, ..
            } = inst
            {
                template_mentions_frame_pointer(template)
                    || clobbers.iter().any(|c| clobber_is_frame_pointer(c))
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

    let stab = analyze_base_stability(func);
    let gep_fold_map = build_gep_fold_map(func, &value_use_counts, &stab);
    let indexed_gep_map = if cg.supports_indexed_addr() {
        build_indexed_gep_map(func, &value_use_counts, &stab)
    } else {
        FxHashMap::default()
    };

    let mut global_addr_map = build_global_addr_map(
        func,
        &cg.state_ref().tls_symbols,
        Some(&cg.state_ref().absolute_symbols),
    );
    // GOT-required identities must not enter the dead-lea set.
    if cg.supports_global_addr_fold() {
        filter_rip_legal_symbols(cg, &mut global_addr_map);
    } else {
        // Even without RIP-rel folding, a GOT-required name+off must not be
        // treated as a direct symbol operand by emit_seg_load_symbol.
        filter_rip_legal_symbols(cg, &mut global_addr_map);
    }

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
        let cmp_select_fusions =
            if cg.supports_fused_cmp_select() && !env_flag_set("CCC_NO_FUSED_CSEL") {
                detect_cmp_select_fusion(block, &value_use_counts)
            } else {
                Vec::new()
            };
        let shifted_logical_fusions = if cg.supports_shifted_logical() {
            detect_shifted_logical_fusions(block, &value_use_counts)
        } else {
            FxHashSet::default()
        };
        let load_cmp_folds = if cg.supports_load_cmp_mem_fold() {
            detect_load_cmp_mem_fold(block, &value_use_counts)
        } else {
            FxHashMap::default()
        };
        let mut fused_add_skip: FxHashSet<usize> = FxHashSet::default();
        let mut skip_fused_logical = false;

        cg.state().block_use_counts.clear();
        cg.state().pending_load_cmp.clear();
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
                    if let Operand::Value(v) = ptr {
                        *cg.state().block_use_counts.entry(v.0).or_insert(0) += 1;
                    }
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

            // Skip address producers whose result is folded. Use the
            // *ultimate* composed base (not the instruction's immediate
            // operand) so GEP(GEP(alloca,c1),c2) is skipped iff the load
            // can fold against alloca+c1+c2.
            if let Some(dest) = inst.dest() {
                if let Some(info) = gep_fold_map.get(&dest.0) {
                    if can_const_addr_fold(cg, info) {
                        cg.state().folded_gep_values.insert(dest.0);
                        cg.state().current_program_point += 1;
                        continue;
                    }
                }
                if let Some(info) = indexed_gep_map.get(&dest.0) {
                    if can_indexed_addr_fold(cg, info) {
                        cg.state().folded_gep_values.insert(dest.0);
                        cg.state().current_program_point += 1;
                        continue;
                    }
                }
                // Map is already GOT/TLS/abs-filtered: every dead identity
                // is a legal RIP-rel (or seg-symbol) operand.
                if dead_global_addrs.contains(&dest.0) {
                    match inst {
                        Instruction::GlobalAddr { .. }
                        | Instruction::GetElementPtr { .. }
                        | Instruction::Copy { .. }
                        | Instruction::Cast { .. }
                        | Instruction::BinOp {
                            op: IrBinOp::Add | IrBinOp::Sub,
                            ..
                        } => {
                            cg.state().current_program_point += 1;
                            continue;
                        }
                        _ => {}
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

            // A deferred vector result lives only in a SIMD scratch register
            // until its consuming intrinsic. Materialise it before an unrelated
            // instruction that can reuse those scratch registers, and drop only
            // scratch aliases (allocator-managed xmm/ymm homes stay live).
            if instruction_may_clobber_vector_scratch(inst) {
                cg.flush_pending_vec_store();
                cg.state().invalidate_vec_scratch_peephole();
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

            // Fold `load; cmp { Eq|Ne, 0 }` into a memory compare: skip the
            // load and hand its (ptr, ty) to the Cmp, which emits
            // `cmp{b,w,l} $0, (mem)`. The detection already proved the load's
            // single use is the adjacent Cmp; only skip plain value pointers
            // (a folded GEP / global-addr / indexed base is emitted through
            // generate_load and is not resolvable by the memory-compare path).
            if let Instruction::Load { dest, ptr, ty, .. } = inst {
                if let Some((_, _, imm)) = load_cmp_folds.get(&dest.0) {
                    if !gep_fold_map.contains_key(&ptr.0)
                        && !indexed_gep_map.contains_key(&ptr.0)
                        && !global_addr_map.contains_key(&ptr.0)
                    {
                        cg.state().pending_load_cmp.insert(dest.0, (ptr.0, *ty, *imm));
                        cg.state().current_program_point += 1;
                        continue;
                    }
                }
            }

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
            if let Instruction::Cmp { op, lhs, rhs, ty, .. } = &block.instructions[fi] {
                if let Terminator::CondBranch {
                    true_label,
                    false_label,
                    ..
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
    // `emit_vector_const_rodata` switches to .rodata. Restore so the next
    // function cannot land in .rodata if it forgets its own `.section`.
    emit_switch_to_section(cg, &func_sect);
}

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

fn rematerialize_const_addr(cg: &mut dyn ArchCodegen, ptr: &Value, info: &GepFoldInfo) {
    if cg.state_ref().folded_gep_values.contains(&ptr.0) {
        cg.emit_gep(ptr, &info.base, &Operand::Const(IrConst::I64(info.offset)));
        cg.state().folded_gep_values.remove(&ptr.0);
    }
}

fn rematerialize_skipped_indexed(cg: &mut dyn ArchCodegen, ptr: &Value, info: &IndexedGepInfo) {
    if cg.state_ref().folded_gep_values.contains(&ptr.0) {
        cg.emit_gep(ptr, &info.base, &Operand::Value(info.orig_offset));
        cg.state().folded_gep_values.remove(&ptr.0);
    }
}

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
            // Defence in depth: the block loop already skipped dead addrs.
            let is_dead = dead_global_addrs.contains(&dest.0);
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
            // __memcpy_chk is inlined only when destlen covers n.
            let inline_len = if cg.supports_inline_memcpy_call() {
                inline_memcpy_len(func.as_str(), &info.args, info.is_variadic)
            } else {
                None
            };
            if let Some(size) = inline_len {
                cg.emit_inline_memcpy_call(&info.args[0], &info.args[1], size);
                // memcpy / __memcpy_chk return dest. Dropping that store
                // miscompiles `p = memcpy(...)`.
                if let Some(dest) = info.dest {
                    cg.emit_copy_value(&dest, &info.args[0]);
                }
                clobber_after_call_like(cg);
                return;
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
            // Intrinsics are the PRODUCERS of the vector-peephole state, not
            // clobberers of it: the individual emitters flush the pending
            // vector store and invalidate the peephole at their START, and a
            // scalar-in-XMM result producer (FixedDistance*, VecHorizontalAdd*)
            // records itself in `direct_fp_result` at its END for the return
            // path to consume. Running clobber_after_call_like() here cleared
            // direct_fp_result right after it was set, so the F32/F64 result
            // was re-materialised from its never-written home slot (the
            // FixedDistance SLP kernels returned 0).
            cg.emit_intrinsic(dest, op, dest_ptr, args);
            cg.state().reg_cache.invalidate_all();
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

fn generate_copy(cg: &mut dyn ArchCodegen, dest: &Value, src: &Operand) {
    // Alloca source: materialize the ADDRESS, not a load from the slot.
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

    // Same-slot elision is valid ONLY when neither side is register-resident.
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
        // If the GlobalAddr/GEP was skipped as dead, rematerialize first.
        if let Some(info) = gep_fold_map.get(&ptr.0) {
            rematerialize_const_addr(cg, ptr, info);
        } else if let Some(info) = indexed_gep_map.get(&ptr.0) {
            rematerialize_skipped_indexed(cg, ptr, info);
        }
        cg.emit_seg_load(dest, ptr, ty, seg_override);
        return;
    }
    if cg.supports_global_addr_fold() && is_foldable_mem_ty(ty) {
        if let Some(sym) = global_addr_map.get(&ptr.0) {
            if !rip_rel_blocked(cg, sym) {
                cg.emit_global_load_rip_rel(dest, sym, ty);
                return;
            }
        }
    }
    if let Some(gep_info) = gep_fold_map.get(&ptr.0) {
        if !is_wide_int_type(ty) && can_const_addr_fold(cg, gep_info) {
            cg.emit_load_with_const_offset(dest, &gep_info.base, gep_info.offset, ty);
            return;
        }
        rematerialize_const_addr(cg, ptr, gep_info);
    }
    if let Some(info) = indexed_gep_map.get(&ptr.0) {
        if !is_wide_int_type(ty) && can_indexed_addr_fold(cg, info) {
            if cg.emit_load_indexed(dest, &info.base, &info.index, info.shift, ty) {
                return;
            }
        }
        rematerialize_skipped_indexed(cg, ptr, info);
    }
    cg.emit_load(dest, ptr, ty);
    if is_wide_int_type(ty) {
        cg.state().reg_cache.invalidate_all();
    }
}

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
        if let Some(info) = gep_fold_map.get(&ptr.0) {
            rematerialize_const_addr(cg, ptr, info);
        } else if let Some(info) = indexed_gep_map.get(&ptr.0) {
            rematerialize_skipped_indexed(cg, ptr, info);
        }
        cg.emit_seg_store(val, ptr, ty, seg_override);
        return;
    }
    if cg.supports_global_addr_fold() && is_foldable_mem_ty(ty) {
        if let Some(sym) = global_addr_map.get(&ptr.0) {
            if !rip_rel_blocked(cg, sym) {
                cg.emit_global_store_rip_rel(val, sym, ty);
                return;
            }
        }
    }
    if let Some(gep_info) = gep_fold_map.get(&ptr.0) {
        if !is_wide_int_type(ty) && can_const_addr_fold(cg, gep_info) {
            cg.emit_store_with_const_offset(val, &gep_info.base, gep_info.offset, ty);
            return;
        }
        rematerialize_const_addr(cg, ptr, gep_info);
    }
    if let Some(info) = indexed_gep_map.get(&ptr.0) {
        if !is_wide_int_type(ty) && can_indexed_addr_fold(cg, info) {
            if cg.emit_store_indexed(val, &info.base, &info.index, info.shift, ty) {
                return;
            }
        }
        rematerialize_skipped_indexed(cg, ptr, info);
    }
    cg.emit_store(val, ptr, ty);
}

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

pub fn is_i128_type(ty: IrType) -> bool {
    matches!(ty, IrType::I128 | IrType::U128)
}

/// Wide = needs register-pair ops. Only I128/U128. i686 I64 is handled by
/// the i686 overrides; including it here would disable GEP folding / fused
/// branches on everyday widened I32 arithmetic.
pub fn is_wide_int_type(ty: IrType) -> bool {
    matches!(ty, IrType::I128 | IrType::U128)
}

pub use super::stack_layout::{
    calculate_stack_space_common, collect_inline_asm_callee_saved,
    collect_inline_asm_callee_saved_with_generic, filter_available_regs, find_param_alloca,
    run_regalloc_and_merge_clobbers, run_regalloc_and_merge_clobbers_ex,
};
