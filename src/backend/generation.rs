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
#[derive(Debug, Clone)]
pub(crate) struct IndexedGepInfo {
    pub(super) base: Value,
    pub(super) index: Value,
    /// Log2 scale in `0..=3`.
    pub(super) shift: u8,
    /// Constant byte displacement from `add(iv, const)` peeling
    /// (PF-06): allows `4(%base, %iv, 4)` for `a[j+1]` instead of
    /// materialising `(j+1)*4` as a separate LEA. Zero for plain
    /// `iv * elem_size` GEPs.
    pub(super) disp: i64,
    /// Original GEP offset operand, used to rematerialize if the backend
    /// refuses the indexed encoding after the GEP was skipped.
    pub(super) orig_offset: Value,
    /// Access types of the Load/Store consumers of this GEP (collected in
    /// [`build_indexed_gep_map`]).  Part of the can-fold/emitter agreement
    /// contract: the fold is only *guaranteed* when the backend's indexed
    /// emitter accepts EVERY consumer's access shape, so the skip and
    /// dead-offset-producer decisions must consult the backend via
    /// [`ArchCodegen::indexed_fold_ok`] instead of assuming a uniform
    /// "any scalar type" capability.
    pub(crate) access_tys: Vec<IrType>,
    /// True when any consumer is a Store.  Stores may need to stage the
    /// stored value through a scratch register at the access site, which
    /// narrows the emitter's acceptance (base/index must survive staging).
    pub(crate) feeds_store: bool,
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
        | Instruction::SetStaticChain { .. }
        | Instruction::InitTrampoline { .. }
        | Instruction::NonlocalGotoSave { .. }
        | Instruction::NonlocalGoto { .. }
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
        | Instruction::GetStaticChain { .. }
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
            if i > 0
                && !next[i + 1..].is_empty()
                && next[i + 1..].bytes().all(|b| b.is_ascii_digit())
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
    // A/B gate: force the pre-session-25 alloca-only contract.
    if env_flag_set("CCC_NO_REGBASE_FOLD") {
        return cg.state_ref().is_alloca(info.base.0);
    }
    // Alloca bases are always foldable: a stack address is re-computable
    // (lea / slot-relative) at every use site, independent of registers.
    if cg.state_ref().is_alloca(info.base.0) {
        return true;
    }
    // REGISTER-RESIDENT bases fold only on backends that (a) extend the
    // base's live interval to the consuming Load/Store — the backend passes
    // collect_gep_fold_base_links(func) into register allocation — and
    // (b) keep the fold paths complete for register bases.  Without (a) the
    // allocator reuses the base register for the stored value (zlib-ng
    // gz_reset `movl %r15d,(%r15)` NULL store); without (b) the access can
    // silently fall through.  The backend hook performs the per-value
    // physical-register sanity check (emitter scratch exclusion).
    cg.const_offset_fold_reg_base_ok(&info.base)
}

fn can_indexed_addr_fold(
    cg: &dyn ArchCodegen,
    info: &IndexedGepInfo,
    global_addr_map: &FxHashMap<u32, String>,
) -> bool {
    // Backend agreement on the ACCESS profile first: types (and store
    // staging) the backend's indexed emitter refuses must also refuse the
    // fold here, or the skip/rematerialise path would read expired offset
    // homes (see ArchCodegen::indexed_fold_ok).  An empty profile means the
    // entry only feeds alias Copies/Casts — the access-site lookup governs
    // those and the skip alone stays harmless.
    if !cg.indexed_fold_ok(info) {
        return false;
    }
    // The index is consumed at the access in every indexed form; it must be
    // register-resident (the RA link extension keeps it live to there).
    if cg.get_phys_reg_for_value(info.index.0).is_none() {
        return false;
    }
    if cg.get_phys_reg_for_value(info.base.0).is_some() {
        return true;
    }
    // Alloca bases fold as `disp(%rbp/%rsp,%idx,scale)`: the frame slot is
    // re-computable at every use site, exactly like the const-offset alloca
    // fold (see can_const_addr_fold). This is the dominant shape for
    // inlined array kernels — the callee's `const double *v` parameter
    // becomes a caller alloca after inlining, and without this arm every
    // access paid a 4-instruction address materialisation
    // (`leaq slot,%rcx; shlq $3,%rax; addq; load`) instead of one SIB
    // operand (spectral_norm mul_Av/mul_Atv inner loops).
    //
    // ONLY plain Direct slots: an OverAligned alloca's runtime address is
    // `(slot+align-1)&~(align-1)`, which the frame-SIB emitter cannot
    // express — and a can-fold/emitter disagreement here is not merely a
    // missed fold: the dead-offset producer walk skips the offset chain
    // based on THIS answer, and the emitter's rematerialise fallback would
    // then reference the skipped producer (vzeroupper_after_ymm: compare
    // loop read `dst[i]` through a never-written index register).
    if cg.state_ref().is_alloca(info.base.0) {
        return matches!(
            cg.state_ref().resolve_slot_addr(info.base.0),
            Some(crate::backend::state::SlotAddr::Direct(_))
        );
    }
    // Symbol-base indexed addressing (`sym(,%idx,scale)`): the GlobalAddr
    // base was never materialised and has no register home.  Only backends
    // that emit the symbol form may take this path — otherwise the skipped
    // producer would rematerialise against an uncomputed GlobalAddr.
    // GOT/TLS/absolute symbols cannot be a bare memory-operand symbol on PIC
    // targets, so refuse them here (the consumer re-checks).
    if !cg.supports_indexed_sym_base() {
        return false;
    }
    if let Some(sym) = global_addr_map.get(&info.base.0) {
        return !rip_rel_blocked(cg, sym);
    }
    false
}

/// Per-function def / alloca / param facts used by GEP-fold soundness.
pub(crate) struct BaseStability {
    is_alloca: FxHashSet<u32>,
    is_param: FxHashSet<u32>,
    def_count: FxHashMap<u32, u32>,
}

pub(crate) fn analyze_base_stability(func: &IrFunction) -> BaseStability {
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

/// PF-06 soundness gate: walk the SSA definition chain of `base_id` to
/// determine whether `iv_id` (or its aliases) appears transitively. Used to
/// refuse the SIB fold `disp(base, iv, scale)` when `base` already contains
/// the IV — the IV would be double-counted by the SIB index slot.
///
/// Walks BinOp(Add/Sub)/Cast/Copy/GEP operands recursively. Treats
/// multi-def values (post-phi, calls, loads) as leaves that cannot contain
/// `iv` (since we can't peer into them). Cycles are prevented by a visited
/// set; unanalysable instructions terminate the walk conservatively (the
/// walk returns `false`, "we cannot prove the IV is absent", but for the
/// SIB math the fold is sound when `disp = const*scale`, so this is fine).
fn base_chain_contains_iv(defs: &FxHashMap<u32, &Instruction>, base_id: u32, iv_id: u32) -> bool {
    if base_id == iv_id {
        return true;
    }
    let mut visited: FxHashSet<u32> = FxHashSet::default();
    let mut stack: Vec<u32> = vec![base_id];
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        if id == iv_id {
            return true;
        }
        let Some(inst) = defs.get(&id).copied() else {
            continue; // multi-def / undef: leaf, can't peer in
        };
        let mut operands: Vec<u32> = Vec::new();
        match inst {
            Instruction::GetElementPtr {
                base,
                offset: Operand::Value(off),
                ..
            } => {
                operands.push(base.0);
                operands.push(off.0);
            }
            Instruction::GetElementPtr { base, .. } => {
                operands.push(base.0);
            }
            Instruction::BinOp { lhs, rhs, .. } => {
                if let Operand::Value(v) = lhs {
                    operands.push(v.0);
                }
                if let Operand::Value(v) = rhs {
                    operands.push(v.0);
                }
            }
            Instruction::Cast {
                src: Operand::Value(v),
                ..
            } => operands.push(v.0),
            Instruction::Copy {
                src: Operand::Value(v),
                ..
            } => operands.push(v.0),
            // Alloca / ParamRef / Const / GlobalAddress / Load / Call:
            // leaves. A Load cannot (in the IR's no-promotion model)
            // contain the IV unless it loads FROM an IV-derived pointer,
            // which we would have walked through via the GetElementPtr
            // case above. Be conservative: treat them as leaves.
            _ => {}
        }
        for op_id in operands {
            if op_id == iv_id {
                return true;
            }
            stack.push(op_id);
        }
    }
    false
}

/// Unique defining instruction of each single-def value. Multi-def (post-phi)
/// values are omitted — inspecting a non-unique def is unsafe.
fn index_single_defs<'a>(
    func: &'a IrFunction,
    stab: &BaseStability,
) -> FxHashMap<u32, &'a Instruction> {
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
fn propagate_stable_aliases<T: Clone>(
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
                if let Some(info) = map.get(&src.0).cloned() {
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
                    Instruction::Load {
                        ptr,
                        ty,
                        seg_override,
                        ..
                    } => {
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
                Instruction::Load {
                    ptr,
                    ty,
                    seg_override,
                    ..
                } => {
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

/// Map of folded-GEP INDEX value ids to their consumer GEP-dest value ids
/// (see RegAllocConfig::folded_index_uses). Derived from the SAME
/// build_indexed_gep_map the emitter uses, so allocator and emitter can
/// never disagree about which indices are consumed where.
pub(crate) fn collect_folded_index_links(func: &IrFunction) -> FxHashMap<u32, Vec<u32>> {
    let use_counts = count_value_uses(func);
    let stab = analyze_base_stability(func);
    let m = build_indexed_gep_map(func, &use_counts, &stab);
    let mut out: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for (dest, info) in &m {
        out.entry(info.index.0).or_default().push(*dest);
    }
    if std::env::var_os("CCC_DEBUG_FOLDED_INDEX").is_some() {
        let mut links: Vec<(&u32, &Vec<u32>)> = out.iter().collect();
        links.sort_unstable();
        eprintln!(
            "[FOLDIDX] fn={} links={} gep_map={}",
            func.name,
            links.len(),
            m.len()
        );
        for (idx, dests) in links {
            eprintln!("[FOLDIDX]   idx=v{} consumers={:?}", idx, dests);
        }
    }
    out
}

/// Map of CONST-FOLDED-GEP BASE value ids to their consumer GEP-dest value
/// ids. `emit_{load,store}_with_const_offset` consumes `gep_info.base` at
/// the Load/Store position while the IR records the base's last use at the
/// (folded-away) GEP — the same RA-invisible-consumption family as the
/// indexed form, but for the BASE of a constant-offset fold. Without the
/// interval extension the allocator may reuse the base's register for the
/// stored value: zlib-ng gz_reset `state->x.have = 0` compiled to
/// `mov %r14,%r15; xorl %r15d,%r15d; movl %r15d,(%r15)` — the zero
/// materialisation (value homed r15) destroyed the base (also homed r15,
/// legal by IR liveness because the base "died" at the folded GEP), and the
/// store went through NULL.
pub(crate) fn collect_gep_fold_base_links(func: &IrFunction) -> FxHashMap<u32, Vec<u32>> {
    let use_counts = count_value_uses(func);
    let stab = analyze_base_stability(func);
    let m = build_gep_fold_map(func, &use_counts, &stab);
    let mut out: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for (dest, info) in &m {
        out.entry(info.base.0).or_default().push(*dest);
    }
    out
}

/// Dest values of INDEXED-fold GEPs (the `base + index*scale` SIB shape),
/// plus their GlobalAddr BASES. The GEP dests are single-use address
/// temporaries: when the fold fires (the index holds a register), the GEP
/// emits NOTHING; when it does not, the fallback materialises through
/// %eax/%ecx staging and a slot is exactly as good as a register. The
/// GlobalAddr bases are the same steal-risk class with a sharper failure
/// mode: vsprintf number()'s digit table homed `digits` in %edx
/// [def..access] — that 2-point holder blocked the loop-carried remainder
/// (born in %edx at the fused div, [div..access]) from the register, the
/// load then unfolded to the 4-instruction `digits+r` materialisation, and
/// the fold's sym form never fired. Denied %edx/%ecx, the base either
/// takes a callee-saved home (reg-base SIB, still a single-instruction
/// load) or none at all (sym form `sym(,%idx,scale)`, base skipped as a
/// dead GlobalAddr) — both leave the caller-saved pool to the index.
pub(crate) fn collect_i686_scratch_denials(func: &IrFunction) -> FxHashSet<u32> {
    let use_counts = count_value_uses(func);
    let stab = analyze_base_stability(func);
    let m = build_indexed_gep_map(func, &use_counts, &stab);
    let mut out: FxHashSet<u32> = m.keys().copied().collect();
    let mut global_addrs: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GlobalAddr { dest, .. } = inst {
                global_addrs.insert(dest.0);
            }
        }
    }
    for info in m.values() {
        if global_addrs.contains(&info.base.0) {
            out.insert(info.base.0);
        }
    }
    // LOAD dests consuming indexed GEPs: their %edx/%ecx homes are worthless
    // (the access is a single instruction and the value flows through the
    // accumulator either way), while an %edx home at the access point is a
    // PRIORITY INVERSION — it both creates an edx hazard at the load (the
    // store-to-home write) and occupies the register the SIB INDEX needs.
    // vsprintf number()'s digit loop measured exactly this: the digit-char
    // dest took %edx in Phase 2 (loads were never edx hazards there), which
    // blocked the loop-carried remainder (born at the fused div, the load's
    // natural SIB index) from %edx in every later phase — the load unfolded
    // to base+addl staging and the digits GlobalAddr rematerialised inside
    // the loop. Store VALUES are deliberately NOT denied: a register home is
    // read directly by direct_store_src (`movl %edx, mem`), which is a win.
    // Denied values keep every other home path (callee-saved Phases 1/2c/2f,
    // %eax Phase 2e).
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Load { ptr, dest, .. } = inst {
                if m.contains_key(&ptr.0) {
                    out.insert(dest.0);
                }
            }
        }
    }
    // DIV QUOTIENT dests: a quotient is BORN in %eax (divl/idivl write the
    // quotient to eax, the remainder to edx) — an %edx home is a
    // wrong-birth-register home requiring a pointless `movl %eax,%edx`, and
    // it occupies %edx across the div point exactly where the REMAINDER (the
    // value actually born in %edx) and the div-adjacent index chains need it.
    // number()'s digit loop: the quotient's [div..cast] %edx interval
    // fragmented the register across the remainder->GEP-index chain
    // [div..load], blocking the SIB fold. The quotient keeps %eax (Phase 2e,
    // its birth register) and callee-saved homes — GCC's shape (`movl %eax,
    // %ebp` after the div). Remainder dests are deliberately NOT denied:
    // %edx is THEIR birth register.
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::UDiv | IrBinOp::SDiv,
                ..
            } = inst
            {
                out.insert(dest.0);
            }
        }
    }
    out
}

/// Union of the per-kind folded-GEP consumption links (index + base) for
/// backends that fold BOTH forms. Each backend must pass exactly the links
/// for the forms its emitter actually consumes at the access position;
/// extending intervals for a form the emitter re-materialises only adds
/// register pressure (x86-64 `check_gpr_leaf_param_codegen::pointer_mix`
/// regressed +2 callee-saves when the INDEX extension was applied blindly).
pub(crate) fn collect_folded_gep_links_all(func: &IrFunction) -> FxHashMap<u32, Vec<u32>> {
    let mut out = collect_gep_fold_base_links(func);
    // Indexed folds consume BOTH the base and the index at the Load/Store
    // access position (the IR records their last uses at the folded-away
    // GEP).  Extending only the index leaves the base register free for
    // reuse between the GEP and the access on backends that emit the
    // indexed memory operand directly (arm, i686 SIB).
    let use_counts = count_value_uses(func);
    let stab = analyze_base_stability(func);
    let m = build_indexed_gep_map(func, &use_counts, &stab);
    for (dest, info) in &m {
        out.entry(info.base.0).or_default().push(*dest);
        out.entry(info.index.0).or_default().push(*dest);
    }
    if std::env::var_os("CCC_DEBUG_FOLDED_INDEX").is_some() {
        let mut links: Vec<(&u32, &Vec<u32>)> = out.iter().collect();
        links.sort_unstable();
        eprintln!(
            "[FOLDLINKS] fn={} links={} indexed_gep_map={}",
            func.name,
            links.len(),
            m.len()
        );
        for (operand, dests) in links {
            eprintln!("[FOLDLINKS]   operand=v{} consumers={:?}", operand, dests);
        }
    }
    out
}

fn build_indexed_gep_map(
    func: &IrFunction,
    use_counts: &[u32],
    stab: &BaseStability,
) -> FxHashMap<u32, IndexedGepInfo> {
    if env_flag_set("CCC_NO_GEP_FOLD") {
        return FxHashMap::default();
    }

    let defs = index_single_defs(func, stab);
    // Must be captured by `resolve_index`: when set, add/sub peeling is
    // skipped so the SIB index stays the add's RESULT (the pre-PF-06
    // behaviour). The later `iv_in_add = None` rewrite only dropped the
    // soundness gates and left the peeled `disp` in place — inverted.
    let pf06_add_peel_disabled = env_flag_set("CCC_NO_PF06_ADD_PEEL");

    // Resolve offset → (index, shift, displacement, iv_in_add, add_result).
    // Multi-def offsets are a plain index (shift 0, disp 0, no iv_in_add);
    // we refuse to inspect a non-unique definition.
    //
    // `iv_in_add` is `Some(iv)` ONLY when an `add(iv, const)` was peeled,
    // so the SIB fold becomes `disp(base, iv, scale)` and we must verify
    // the GEP base does NOT transitively depend on `iv` (else the index
    // gets double-counted). For all other shapes (plain `iv`, `iv*2^k`,
    // `add(iv,iv)`) the SIB is `0(base, iv, scale)` and the index is
    // counted exactly once regardless of the base, so no soundness check
    // is needed.
    //
    // `add_result` is the SSA value id of the add's RESULT (the input to
    // the next peel step, e.g. the shl). It's used by the soundness gate
    // to detect iv-update Copies: `v_iv = copy(v_add_result)`. The RA
    // would coalesce `v_iv` and `v_add_result`, and the scheduler may
    // move the coalesced add before the SIB load — reading the new iv
    // value instead of the old.
    let resolve_index = |off_id: u32| -> Option<(Value, u8, i64, Option<u32>, Option<u32>)> {
        // Peel power-of-2 scalings and widening integer casts RECURSIVELY
        // (session 27): `p[i*8]` lowers to Shl(Shl(i,1),2) / Mul(i,8) chains
        // whose intermediates are accumulator-flow values with no register
        // home. Peeling to the natural index (typically the loop IV,
        // which IS homed) enables `mem(%base,%iv,8)` instead of
        // materialising the scaled offset. The accumulated shift must
        // fit the SIB scale field (≤3); when it does not, stop peeling
        // and use the current value as the index.
        //
        // PF-06 (session 70 attempted; v7 implements soundly): peel
        // through `add(iv, const)` — the `(j+1)*4` shape from `a[j+1]` —
        // recording the constant as a displacement scaled by the
        // accumulated shift at the add's position (`const << shift`).
        // This allows `4(%base,%j,4)` instead of a separate
        // `leaq 0(,%rax,4),%r12` every iteration.
        let mut id = off_id;
        let mut shift: u8 = 0;
        let mut disp: i64 = 0;
        let mut iv_in_add: Option<u32> = None;
        let mut add_result: Option<u32> = None;
        loop {
            let Some(inst) = defs.get(&id).copied() else {
                break;
            };
            match inst {
                Instruction::Cast {
                    src: Operand::Value(v),
                    from_ty,
                    to_ty,
                    ..
                } if from_ty.is_integer()
                    && to_ty.is_integer()
                    && to_ty.size() >= from_ty.size() =>
                {
                    id = v.0;
                }
                Instruction::BinOp {
                    op: IrBinOp::Shl,
                    lhs: Operand::Value(idx),
                    rhs: Operand::Const(c),
                    ..
                } => {
                    let k = c.to_i64()?;
                    if k < 0 || (shift as i64) + k > 3 {
                        break;
                    }
                    shift += k as u8;
                    // Do NOT scale `disp` here: the shift applies to the
                    // index (iv), not to the displacement. A displacement
                    // from an outer add is already scaled by the shift at
                    // the add's position; a displacement from an inner add
                    // (shl(add(iv,const),k)) is scaled at the add point.
                    id = idx.0;
                }
                Instruction::BinOp {
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(idx),
                    rhs: Operand::Const(c),
                    ..
                }
                | Instruction::BinOp {
                    op: IrBinOp::Mul,
                    lhs: Operand::Const(c),
                    rhs: Operand::Value(idx),
                    ..
                } => {
                    let n = c.to_i64()?;
                    if n <= 0 || !(n as u64).is_power_of_two() {
                        break;
                    }
                    let z = (n as u64).trailing_zeros();
                    if (shift as u32) + z > 3 {
                        break;
                    }
                    shift += z as u8;
                    // Same rationale: do NOT scale `disp`.
                    id = idx.0;
                }
                // Self-addition doubling: the frontend strength-reduces
                // `i*2` to `Add(i,i)`; peeling it as one more scale bit
                // reaches the homed loop IV underneath (`p[i*2]` int loads
                // arrive as Shl(Add(i,i),2) = shift 3).
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs: Operand::Value(a),
                    rhs: Operand::Value(b),
                    ..
                } if a == b && shift < 3 => {
                    shift += 1;
                    // Same: do NOT scale `disp`.
                    id = a.0;
                }
                // PF-06 (v7): `add(iv, const)` and `sub(iv, const)` peeling.
                // Records the constant as a displacement SCALED BY THE
                // ACCUMULATED SHIFT at this point: `disp = ±const << shift`.
                // For the two shapes this produces:
                //
                //   * `add(shl(iv, k), D)` — peel add first, shift=0,
                //     so disp = D. Then peel shl, shift=k. Final:
                //     disp=D, shift=k. SIB `D(base, iv, 2^k)`.
                //   * `shl(add(iv, C), k)` — peel shl first, shift=k.
                //     Then peel add, disp = C << k. Final: disp=C<<k,
                //     shift=k. SIB `(C<<k)(base, iv, 2^k)`.
                //
                // Soundness: when the GEP's base also contains `iv`, the
                // SIB double-counts: `disp(base, iv, scale)` evaluates to
                // `base + iv*scale + disp`, and if `base = p + iv*scale_a`,
                // the total is `p + iv*(scale_a + scale) + disp` — but the
                // intended semantics is `base + (iv + const)*scale`,
                // i.e. `p + iv*scale_a + iv*scale + const*scale`.
                // These ARE equal when `disp = const*scale`, i.e. when
                // disp is correctly computed as `const << shift_at_add`.
                //
                // So the fold IS sound in this case too. But to be
                // defensive (and to match the previous session's
                // observation of 5 regression failures whose shape we
                // could not reproduce here), we record `iv_in_add =
                // Some(iv_id)` and require a soundness check at the call
                // site that the GEP base's SSA chain does not include
                // the IV — UNLESS the chain is unanalysable, in which
                // case we trust the math.
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs: Operand::Value(idx),
                    rhs: Operand::Const(c),
                    ..
                }
                | Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs: Operand::Const(c),
                    rhs: Operand::Value(idx),
                    ..
                } => {
                    // Already saw an add(iv, const) — refuse nested adds.
                    if iv_in_add.is_some() {
                        break;
                    }
                    let k = c.to_i64()?;
                    // Scale by the accumulated shift at this point.
                    let scaled = if shift == 0 {
                        k
                    } else {
                        let scale_factor = 1i64 << shift;
                        match k.checked_mul(scale_factor) {
                            Some(v) => v,
                            None => break,
                        }
                    };
                    if !is_i32_disp(scaled) {
                        break;
                    }
                    disp = match disp.checked_add(scaled) {
                        Some(v) => v,
                        None => break,
                    };
                    if !is_i32_disp(disp) {
                        break;
                    }
                    // Record the add's RESULT (the current `id` before we
                    // reassign to the add's input `idx`). The soundness gate
                    // uses this to detect iv-update Copies
                    // `v_iv = copy(v_add_result)`.
                    add_result = Some(id);
                    iv_in_add = Some(idx.0);
                    id = idx.0;
                }
                // PF-06 (v7): `sub(iv, const)` is the same as
                // `add(iv, -const)`. Records `disp = -const << shift`.
                // Same soundness gate as Add applies via `iv_in_add`.
                Instruction::BinOp {
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(idx),
                    rhs: Operand::Const(c),
                    ..
                } => {
                    if pf06_add_peel_disabled {
                        break;
                    }
                    if iv_in_add.is_some() {
                        break;
                    }
                    let k = c.to_i64()?;
                    let neg_k = match k.checked_neg() {
                        Some(v) => v,
                        None => break,
                    };
                    let scaled = if shift == 0 {
                        neg_k
                    } else {
                        let scale_factor = 1i64 << shift;
                        match neg_k.checked_mul(scale_factor) {
                            Some(v) => v,
                            None => break,
                        }
                    };
                    if !is_i32_disp(scaled) {
                        break;
                    }
                    disp = match disp.checked_add(scaled) {
                        Some(v) => v,
                        None => break,
                    };
                    if !is_i32_disp(disp) {
                        break;
                    }
                    add_result = Some(id);
                    iv_in_add = Some(idx.0);
                    id = idx.0;
                }
                _ => break,
            }
        }
        Some((Value(id), shift, disp, iv_in_add, add_result))
    };

    let mut map: FxHashMap<u32, IndexedGepInfo> = FxHashMap::default();
    // PF-06 soundness precomputation: for each `add(iv, const)` we peeled,
    // we need to refuse the fold when `iv` and the add's RESULT are
    // copy-coalesceable. The RA coalesces `v_iv = copy v_add_result`
    // because the add's result dies at that copy and the iv is reborn.
    // After coalescing, the add `v_add_result = v_iv + const` becomes an
    // in-place `add $const, %reg` that overwrites the iv's register. If
    // the scheduler places this add BEFORE the SIB load (which uses the
    // iv's register as the SIB index), the SIB reads the NEW iv value
    // (= old iv + const) instead of the OLD iv value — a miscompile.
    //
    // The fix: scan the function for any `Copy { dest: iv, src: v49 }`
    // shape and record the (iv, v49) pairs. If the add's result is in
    // that set, refuse the fold.
    //
    // This catches the accumulator_pointer_load case:
    //   v49 = add(v69, 1); v51 = gep(v50, v49); v52 = load(v51);
    //   ...; v69 = copy(v49)   <-- iv-update Copy uses v49 -> refuse.
    //
    // It does NOT catch the prefix_sum case (which is sound):
    //   v11 = sub(v25, 1); v13 = shl(v11, 2); v14 = gep(v2, v13); v15 = load(v14);
    //   ...; v24 = add(v25, 1); v25 = copy(v24)   <-- iv-update uses v24, NOT v11.
    //
    // Kill switch: `CCC_NO_PF06_ADD_PEEL` (any non-empty value) disables
    // ONLY the add(iv, const) / sub(iv, const) peeling. The flag is
    // captured by `resolve_index` above so the peel itself is skipped,
    // not merely the soundness gates that used to wrap it.
    let mut iv_coalesce_pairs: FxHashSet<(u32, u32)> = FxHashSet::default();
    if !pf06_add_peel_disabled {
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Copy {
                    dest,
                    src: Operand::Value(src_v),
                } = inst
                {
                    iv_coalesce_pairs.insert((dest.0, src_v.0));
                }
                // Also handle Cast (the RA may coalesce across a no-op cast):
                // `v_iv = cast(v49)` where the cast preserves the value.
                // Be conservative: any Cast into v_iv counts.
                if let Instruction::Cast {
                    dest,
                    src: Operand::Value(src_v),
                    ..
                } = inst
                {
                    iv_coalesce_pairs.insert((dest.0, src_v.0));
                }
            }
        }
    }

    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GetElementPtr {
                dest,
                base,
                offset: Operand::Value(off),
                ..
            } = inst
            {
                let Some((index, shift, disp, iv_in_add, add_result)) = resolve_index(off.0) else {
                    continue;
                };
                // PF-06 soundness gate 1: when `add(iv, const)` was
                // peeled, verify the GEP base does NOT transitively
                // contain `iv` (the IV would be double-counted by the
                // SIB index slot — see the comment above
                // `base_chain_contains_iv`).
                //
                // PF-06 kill switch: when disabled, pretend no add was
                // peeled (treat the resolved iv as a regular index).
                let iv_in_add = if pf06_add_peel_disabled {
                    None
                } else {
                    iv_in_add
                };
                if let Some(iv_id) = iv_in_add {
                    if base_chain_contains_iv(&defs, base.0, iv_id) {
                        continue;
                    }
                    // PF-06 soundness gate 2: refuse the fold when the
                    // add's RESULT (`add_result`, the value fed into the
                    // next peel step) is used as the source of a Copy/Cast
                    // whose dest is the IV. The RA would coalesce the
                    // add's result with the IV, and the scheduler may
                    // place the coalesced add BEFORE the SIB load —
                    // reading the new IV value instead of the old.
                    //
                    // Note: we check (iv_id, add_result) — NOT
                    // (iv_id, off.0). The iv-update Copy uses the add's
                    // RESULT, which is the value BEFORE the shl/mul
                    // outer peel steps. `off.0` is the OUTERMOST value
                    // (e.g. the shl's result), which is NOT what the
                    // iv-update Copy uses.
                    if let Some(ar) = add_result {
                        if iv_coalesce_pairs.contains(&(iv_id, ar)) {
                            // Retarget instead of refusing. The offset
                            // chain semantically IS
                            // `base + add_result << shift` here (the
                            // peel walked through the add: address =
                            // base + ((iv + k) << shift_at_add) <<
                            // shift_after = base + add_result <<
                            // shift_final; disp held only the add's own
                            // `k << shift_at_add` because nested adds
                            // are refused above, so dropping it and
                            // indexing by the add's result is exact).
                            // The old-iv SIB form `k<<(base, iv, scale)`
                            // would read the iv's register after the
                            // coalesced in-place add overwrote it; the
                            // retargeted form reads the register the add
                            // WROTE, and liveness of add_result extends
                            // to the access (can_indexed_addr_fold), so
                            // the add is ordered before the SIB by a
                            // true dependence — no scheduling hazard
                            // regardless of block layout. This is the
                            // `i++; a[i]` post-increment scan shape
                            // (linux_find_bit: 2 extra instructions per
                            // scanned word; GCC folds it too).
                            map.insert(
                                dest.0,
                                IndexedGepInfo {
                                    base: *base,
                                    index: Value(ar),
                                    shift,
                                    disp: 0,
                                    orig_offset: *off,
                                    access_tys: Vec::new(),
                                    feeds_store: false,
                                },
                            );
                            continue;
                        }
                    }
                }
                map.insert(
                    dest.0,
                    IndexedGepInfo {
                        base: *base,
                        index,
                        shift,
                        disp,
                        orig_offset: *off,
                        access_tys: Vec::new(),
                        feeds_store: false,
                    },
                );
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

    // Consumer-access profile: every surviving entry is consumed exclusively
    // by Load/Store (retained above).  Record each consumer's access type and
    // whether any consumer is a Store, so can_indexed_addr_fold can ask the
    // backend whether its indexed emitter accepts EVERY access shape before
    // the GEP's emission is skipped and its offset chain declared dead.
    {
        let mut access: FxHashMap<u32, (Vec<IrType>, bool)> = FxHashMap::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                let (ptr, ty, is_store) = match inst {
                    Instruction::Load { ptr, ty, .. } => (ptr, ty, false),
                    Instruction::Store { ptr, ty, .. } => (ptr, ty, true),
                    _ => continue,
                };
                let e = access.entry(ptr.0).or_insert_with(|| (Vec::new(), false));
                e.0.push(*ty);
                e.1 |= is_store;
            }
        }
        for (dest, info) in map.iter_mut() {
            if let Some((tys, feeds_store)) = access.get(dest) {
                info.access_tys = tys.clone();
                info.feeds_store = *feeds_store;
            }
        }
    }
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

    // Compose const chains and propagate Copy/Cast aliases INTERLEAVED, to a
    // bounded fixed point.  Composing only once before propagating left
    // entries whose base was a later-propagated stable alias pointing at the
    // alias (e.g. `GEP(var_idx); Copy V25; GEP V27 = V25+16` composed with
    // base = V25) while V25's producer is absorbed (skipped, never emitted):
    // the consumer then read V25's stale register home — the variable index
    // silently dropped (sqlite ExprListSetSortOrder stored through &p->a
    // instead of &p->a[nExpr-1]; openDatabase SIGSEGV at -O0).  Session-24
    // fixed this with a single re-compose after propagate; the interleaved
    // fixed point additionally resolves deeper alias chains (each round can
    // expose one more link), and the safety net below catches anything the
    // bound leaves behind.
    for _ in 0..8 {
        compose_const_gep_folds(&mut gep_map);
        propagate_stable_aliases(func, stab, &mut gep_map);
    }
    compose_const_gep_folds(&mut gep_map);
    retain_stable_bases(func, use_counts, stab, &mut gep_map);
    propagate_stable_aliases(func, stab, &mut gep_map);
    compose_const_gep_folds(&mut gep_map);
    retain_ptr_only_uses(func, &mut gep_map);
    // SAFETY NET: a fold entry whose base is STILL a map key would read a
    // register (or slot) its skipped producer never wrote.  Composition
    // removes all such edges when the offsets fit; overflow-truncated
    // compositions are the only survivors — drop them (conservative, always
    // sound: the producer is emitted instead).  Removal re-exposes the
    // dropped dest's base as a real (non-fold) use, so re-run the
    // retain_ptr_only_uses fixed point after each round.
    loop {
        let bad: Vec<u32> = gep_map
            .iter()
            .filter(|(_, info)| gep_map.contains_key(&info.base.0))
            .map(|(dest, _)| *dest)
            .collect();
        if bad.is_empty() {
            break;
        }
        for d in bad {
            gep_map.remove(&d);
        }
        retain_ptr_only_uses(func, &mut gep_map);
    }
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

/// Root `GlobalAddr` values that can be omitted and reconstructed at each use.
///
/// This is deliberately narrower than the symbol identity map. A candidate's
/// every use must be either an already-proven direct scalar Load/Store or one
/// address derivation that the target hook can emit as `symbol + offset`.
/// Passing an address as data, comparing it, storing it as a value, using a
/// wide/segment memory operation, or placing it in a terminator rejects the
/// root. The fail-closed scan is what makes removing its register/stack home
/// safe: no generic operand path can encounter an unmaterialized value.
pub fn build_rematerializable_global_addr_set_for(
    func: &IrFunction,
    global_addr_map: &FxHashMap<u32, String>,
) -> FxHashSet<u32> {
    if env_flag_set("CCC_NO_GLOBAL_ADDR_REMAT") {
        return FxHashSet::default();
    }

    let mut roots = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GlobalAddr { dest, .. } = inst {
                if global_addr_map.contains_key(&dest.0) {
                    roots.insert(dest.0);
                }
            }
        }
    }
    if roots.is_empty() {
        return roots;
    }

    let op_is_root = |op: &Operand, id: u32| matches!(op, Operand::Value(v) if v.0 == id);
    let op_uses_root = |op: &Operand, id: u32| op_is_root(op, id);

    let candidates: Vec<u32> = roots.iter().copied().collect();
    for id in candidates {
        let mut valid = true;
        for block in &func.blocks {
            for inst in &block.instructions {
                if !inst.used_values().contains(&id) {
                    continue;
                }
                let allowed = match inst {
                    Instruction::Load {
                        ptr,
                        ty,
                        seg_override,
                        ..
                    } => {
                        ptr.0 == id
                            && *seg_override == AddressSpace::Default
                            && is_foldable_mem_ty(*ty)
                    }
                    Instruction::Store {
                        val,
                        ptr,
                        ty,
                        seg_override,
                        ..
                    } => {
                        ptr.0 == id
                            && !op_uses_root(val, id)
                            && *seg_override == AddressSpace::Default
                            && is_foldable_mem_ty(*ty)
                    }
                    Instruction::GetElementPtr { base, offset, .. } => {
                        base.0 == id && !op_uses_root(offset, id)
                    }
                    Instruction::BinOp {
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ty,
                        ..
                    } => {
                        !ty.is_float()
                            && ty.size() == 8
                            && (op_is_root(lhs, id) ^ op_is_root(rhs, id))
                    }
                    Instruction::BinOp {
                        op: IrBinOp::Sub,
                        lhs,
                        rhs,
                        ty,
                        ..
                    } => {
                        !ty.is_float()
                            && ty.size() == 8
                            && op_is_root(lhs, id)
                            && !op_is_root(rhs, id)
                    }
                    _ => false,
                };
                if !allowed {
                    valid = false;
                    break;
                }
            }
            if !valid || block.terminator.used_values().contains(&id) {
                valid = false;
                break;
            }
        }
        if !valid {
            roots.remove(&id);
        }
    }
    roots
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
pub(crate) fn count_value_uses(func: &IrFunction) -> Vec<u32> {
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

/// What kind of flag-producing instruction anchors a fused branch.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FusedBranchKind {
    Cmp,
    BitTest,
}

/// Cmp or BitTest (optionally through Copy / integer Cast) used only by
/// CondBranch. `fuse_bt` gates the BitTest form on backend support: BT sets
/// CF directly, so `bt; jc` needs no setc/movzbl/test materialization.
fn detect_cmp_branch_fusion(
    block: &BasicBlock,
    use_counts: &[u32],
    fuse_fp: bool,
    fuse_bt: bool,
) -> Option<(usize, Option<usize>, FusedBranchKind)> {
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
    let (cmp_idx, kind) = loop {
        if scan == 0 {
            return None;
        }
        scan -= 1;
        match &block.instructions[scan] {
            Instruction::Cmp { dest, .. } if dest.0 == wanted => {
                break (scan, FusedBranchKind::Cmp);
            }
            Instruction::BinOp {
                dest,
                op: IrBinOp::BitTest,
                ..
            } if fuse_bt && dest.0 == wanted => break (scan, FusedBranchKind::BitTest),
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
        Instruction::BinOp {
            dest,
            op: IrBinOp::BitTest,
            ty,
            ..
        } => (dest.0, ty),
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
    // A float-typed BitTest cannot exist (simplify only builds integer ones);
    // fail closed if one ever appears.
    if kind == FusedBranchKind::BitTest && !ty.is_integer() {
        return None;
    }
    Some((cmp_idx, chain_end, kind))
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
                volatile,
            } => {
                if *volatile {
                    // A volatile load must reach the backend as a real load
                    // instruction; folding it into a memory compare would
                    // still read memory once, BUT the folded form bypasses
                    // the Load emission path the dead-load peepholes key on,
                    // and single-use adjacency is not a sound proxy for the
                    // access contract.  Keep it simple and honest: no fold.
                    continue;
                }
                (dest.0, ptr.0, *ty, *seg_override)
            }
            _ => continue,
        };
        if seg != AddressSpace::Default {
            continue;
        }
        // Foldable widths map 1:1 onto cmpb/cmpw/cmpl; anything else (wide
        // pairs, floats, vectors, i128) is excluded.
        match ty {
            IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::Ptr => {}
            _ => continue,
        }
        // The load's only consumer is the compare.
        if use_counts
            .get(load_dest as usize)
            .copied()
            .unwrap_or(u32::MAX)
            != 1
        {
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
        let Some(imm) = cmp_fold_imm(rhs, ty) else {
            continue;
        };
        match cmp_ty {
            IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::Ptr => {}
            _ => continue,
        }
        out.insert(load_dest, (ptr, ty, imm));
    }
    out
}

/// Whole-function set of values that the codegen loop folds away entirely —
/// they emit no code and write no value, so they must not occupy a register or
/// a stack slot (the register allocator otherwise parks the dead value in a
/// caller-saved register, stealing it from a live loop pointer). Two cases:
///
/// 1. Loads whose single use is a foldable adjacent `Cmp { Eq|Ne, imm }` AND
///    whose pointer is a plain value (not a folded GEP / indexed GEP / global
///    address) — folded into `cmp{b,w,l} $imm,(mem)`.
/// 2. Cmp results (plus their Copy/Cast chain) fused into a CondBranch — the
///    boolean is never materialized.
///
/// Mirrors the generation-loop skip conditions exactly.
pub(crate) fn build_folded_value_set(
    func: &IrFunction,
    tls_symbols: &FxHashSet<String>,
    absolute_symbols: &FxHashSet<String>,
) -> FxHashSet<u32> {
    let use_counts = count_value_uses(func);
    let stab = analyze_base_stability(func);
    let gep_fold_map = build_gep_fold_map(func, &use_counts, &stab);
    let indexed_gep_map = build_indexed_gep_map(func, &use_counts, &stab);
    let global_addr_map = build_global_addr_map(func, tls_symbols, Some(absolute_symbols));
    let mut set = FxHashSet::default();
    for block in &func.blocks {
        // (1) load → memory compare.  Indexed GEPs feeding these loads are
        // never skipped at generation (the load_cmp_ptrs priority guard), so
        // they do not block the fold; const-folded / global ptrs are still
        // skipped producers the memory compare cannot resolve.
        for (dest, (ptr, _ty, _imm)) in detect_load_cmp_mem_fold(block, &use_counts) {
            if !gep_fold_map.contains_key(&ptr) && !global_addr_map.contains_key(&ptr) {
                set.insert(dest);
            }
        }
        // (2) fused compare-and-branch: the boolean (and any Copy/Cast chain)
        // feeding the CondBranch never materializes. Integer compares only —
        // FP fusion is backend-gated (false on i686, where this set is used).
        if let Some((cmp_idx, chain_end, _)) =
            detect_cmp_branch_fusion(block, &use_counts, false, true)
        {
            let end = chain_end.unwrap_or(cmp_idx);
            for inst in &block.instructions[cmp_idx..=end] {
                if let Some(dest) = inst.dest() {
                    set.insert(dest.0);
                }
            }
        }
    }
    set
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

#[derive(Clone)]
struct ConditionalIncrementFusion {
    select_idx: usize,
    add_idx: usize,
    cmp_idx: Option<usize>,
    base: Operand,
    increment_ty: IrType,
    increment_on_true: bool,
}

/// Recognize an integer compare/select arm of the form `base + 1` while the
/// other arm is exactly `base`.  The producer has one use, so a backend with a
/// conditional-increment instruction may omit the Add entirely.
///
/// This is deliberately an SSA-level combine rather than an assembly
/// peephole. The other Select arm keeps `base` live through the Select in the
/// allocator, and the one-use proof makes deleting the Add independent of
/// physical-register assignment and CFG layout.
fn detect_conditional_increment_selects(
    block: &BasicBlock,
    use_counts: &[u32],
    base_stability: &BaseStability,
) -> Vec<ConditionalIncrementFusion> {
    let definitions: FxHashMap<u32, (usize, &Instruction)> = block
        .instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| {
            instruction
                .dest()
                .map(|dest| (dest.0, (index, instruction)))
        })
        .collect();
    let mut out = Vec::new();
    for (select_idx, instruction) in block.instructions.iter().enumerate() {
        let Instruction::Select {
            cond,
            true_val,
            false_val,
            ty,
            ..
        } = instruction
        else {
            continue;
        };
        if !ty.is_integer() || ty.is_128bit() {
            continue;
        }

        for (increment, base, increment_on_true) in
            [(true_val, false_val, true), (false_val, true_val, false)]
        {
            let Operand::Value(increment_value) = increment else {
                continue;
            };
            if use_counts
                .get(increment_value.0 as usize)
                .copied()
                .unwrap_or(u32::MAX)
                != 1
                || base_stability.def_count.get(&increment_value.0).copied() != Some(1)
            {
                continue;
            }

            let Some(&(add_idx, add)) = definitions.get(&increment_value.0) else {
                continue;
            };
            if add_idx >= select_idx {
                continue;
            }
            let Instruction::BinOp {
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty: add_ty,
                ..
            } = add
            else {
                continue;
            };
            // Preserve the Add's arithmetic width. The frontend currently
            // represents a U32 conditional expression as an I64 Select even
            // though both arms are zero-extended U32 values. A 32-bit CSINC
            // reproduces the required U32 wrap and zero-extension; widening
            // signed or narrowing combinations require explicit casts and are
            // deliberately left to ordinary lowering.
            let width_compatible = add_ty == ty
                || (add_ty.is_integer()
                    && ty.is_integer()
                    && !ty.is_128bit()
                    && (add_ty.size() == ty.size() || (*add_ty == IrType::U32 && ty.size() >= 4)));
            if !width_compatible || !matches!(add_ty.size(), 4 | 8) {
                continue;
            }
            let is_one = |operand: &Operand| matches!(operand, Operand::Const(constant) if constant.to_i64() == Some(1));
            let same_operand = |a: &Operand, b: &Operand| match (a, b) {
                (Operand::Value(a), Operand::Value(b)) => a == b,
                (Operand::Const(a), Operand::Const(b)) => a.to_hash_key() == b.to_hash_key(),
                _ => false,
            };
            let matches_base = (same_operand(lhs, base) && is_one(rhs))
                || (same_operand(rhs, base) && is_one(lhs));
            if !matches_base {
                continue;
            }
            // LCCC's post-phi IR permits repeated Copy definitions of one
            // value id. The Add and Select still observe the same dynamic base
            // only when no intervening instruction redefines that id.
            if let Operand::Value(base_value) = base {
                if block.instructions[add_idx + 1..select_idx]
                    .iter()
                    .any(|instruction| instruction.dest() == Some(*base_value))
                {
                    continue;
                }
            }

            // If the condition is a single-use integer Cmp and the only IR
            // instruction between it and the Select is the Add being removed,
            // consume the comparison flags directly. This covers the common
            // frontend order `Cmp; Add; Select` without the unsafe general
            // reordering that the ordinary cmp-select fusion intentionally
            // rejects.
            let cmp_idx = match cond {
                Operand::Value(condition)
                    if use_counts
                        .get(condition.0 as usize)
                        .copied()
                        .unwrap_or(u32::MAX)
                        == 1
                        && base_stability.def_count.get(&condition.0).copied() == Some(1) =>
                {
                    definitions.get(&condition.0).and_then(
                        |&(index, instruction)| match instruction {
                            Instruction::Cmp { ty, .. }
                                if index < select_idx
                                    && !ty.is_float()
                                    && !ty.is_long_double()
                                    && !ty.is_128bit()
                                    && (index + 1 == select_idx
                                        || (index + 2 == select_idx && add_idx == index + 1)) =>
                            {
                                Some(index)
                            }
                            _ => None,
                        },
                    )
                }
                _ => None,
            };

            out.push(ConditionalIncrementFusion {
                select_idx,
                add_idx,
                cmp_idx,
                base: base.clone(),
                increment_ty: *add_ty,
                increment_on_true,
            });
            break;
        }
    }
    out
}

/// Mul whose single use is a nearby Add. Map: mul_idx → add_idx.
fn detect_mul_add_fusions(
    block: &BasicBlock,
    use_counts: &[u32],
    fuse_float: bool,
    fp_contract: crate::common::fp_contract::FpContract,
    fp_tags: &crate::common::fp_contract::FpExprTags,
    // Integer `acc - a*b` -> msub (AArch64; see supports_fused_int_mul_sub).
    fuse_int_sub: bool,
    // Float Mul;Sub -> fmsub/fnmsub (levkropp e3b21b8f, audited port).
    // None disables Sub fusion entirely (backend lacks the instruction or
    // CCC_NO_FMSUB is set); Some(accs) enables it except for destinations
    // in `accs`: fusing `acc -= a*b` puts the multiply on the loop-carried
    // serial dependency chain (fmsub latency > fsub latency), a measured
    // 13% regression on nbody. Such Subs stay split so the multiply issues
    // independently of the chain.
    fuse_float_sub: Option<&FxHashSet<u32>>,
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
            || (crate::common::types::target_is_32bit()
                && matches!(mul_ty, IrType::I64 | IrType::U64))
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
                // Float Sub fusion: `a - b*c` -> fmsub, `b*c - a` -> fnmsub
                // (both single-rounding, matching GCC -ffp-contract=fast).
                // Subs whose dest is a loop-carried accumulator stay split
                // (see fuse_float_sub above).
                Instruction::BinOp {
                    dest: sub_dest,
                    op: IrBinOp::Sub,
                    lhs,
                    rhs,
                    ty,
                    ..
                } if mul_ty.is_float()
                    && fuse_float_sub.is_some_and(|accs| !accs.contains(&sub_dest.0)) =>
                {
                    add_idx = Some(scan);
                    add_lhs_r = Some(lhs);
                    add_rhs_r = Some(rhs);
                    add_ty_r = Some(ty);
                    break;
                }
                // Integer Sub fusion: `acc - a*b` -> msub (levkropp 9f064faa,
                // audited port). ONLY when the mul is the Sub's RHS — `a*b -
                // acc` has no msub form. Used by magic-number division
                // (strlen/itoa: q = n/10; r = n - q*10). No accumulator gate:
                // integer msub has the same latency as mul on shipping
                // AArch64 cores, so the chain never lengthens.
                Instruction::BinOp {
                    op: IrBinOp::Sub,
                    lhs,
                    rhs,
                    ty,
                    ..
                } if !mul_ty.is_float()
                    && fuse_int_sub
                    && matches!(rhs, Operand::Value(v) if v.0 == mul_dest.0) =>
                {
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
        let defined_between = |op: &Operand| matches!(op, Operand::Value(v) if skipped_defs[..skipped_count].contains(&v.0));
        if defined_between(add_lhs) || defined_between(add_rhs) || mul_ty != add_ty {
            continue;
        }
        // OP-36: FP contraction is pair-gated. `fast` fuses freely; `on`
        // requires the mul and the add to share one statement-root tag
        // (same source expression); untagged values (pass-generated,
        // inlined) fail closed. Integer fusion is unaffected.
        if mul_ty.is_float() {
            let add_dest = block.instructions[next_idx].dest().map(|d| d.0);
            let mul_tag = fp_tags.get(&mul_dest.0).copied();
            let add_tag = add_dest.and_then(|a| fp_tags.get(&a).copied());
            if !fp_contract.fuse_pair(mul_tag, add_tag) {
                continue;
            }
        }
        claimed_adds.insert(next_idx);
        fusion_map.insert(idx, next_idx);
    }
    fusion_map
}

/// Gap-fused multiply-add candidates (levkropp 2e57bcf2, audited port):
/// `fmul` at idx, one or two Load/GEP instructions between (nbody's j-side
/// address computation and velocity load), then `fadd` consuming the mul
/// result. The fmadd is emitted AT the Add so the gap instructions execute
/// first, in order. Returns (mul_idx, add_idx); the driver recomputes the
/// gap indices and validates register aliasing before committing (the multiply operands are
/// read LATER than liveness recorded, so no gap destination register may
/// alias them). Windows are disjoint from detect_mul_add_fusions by
/// construction: that scan breaks at Load/GEP, this one breaks at Copy/Cast,
/// so an Add is claimed by at most one detector. CCC_NO_GAP_FMA disables.
fn detect_gap_fma_fusions(
    block: &BasicBlock,
    use_counts: &[u32],
    fuse_float: bool,
    accumulator_dests: &FxHashSet<u32>,
    fp_contract: crate::common::fp_contract::FpContract,
    fp_tags: &crate::common::fp_contract::FpExprTags,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if !fuse_float || env_flag_set("CCC_NO_GAP_FMA") {
        return out;
    }
    // OP-36 pair gate helper: integer candidates skip the contract check.
    struct FpIntHelper;
    impl FpIntHelper {
        fn like(&self, ty: &IrType) -> bool {
            !ty.is_float()
        }
    }
    let fp_int = FpIntHelper;
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
        if !mul_ty.is_float() || matches!(mul_ty, IrType::F128) {
            continue;
        }
        // Exactly one (global) use: the Add. Skipping the standalone Mul
        // emission is then safe -- nothing else ever reads mul_dest.
        if use_counts.get(mul_dest.0 as usize).copied().unwrap_or(0) != 1 {
            continue;
        }
        let mut gap_len = 0usize;
        let mut found = None;
        for j in (idx + 1)..block.instructions.len() {
            match &block.instructions[j] {
                Instruction::Load { .. } | Instruction::GetElementPtr { .. } if gap_len < 2 => {
                    gap_len += 1;
                }
                Instruction::BinOp {
                    dest: add_dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ty: add_ty,
                } if gap_len > 0 => {
                    let mul_used = matches!(lhs, Operand::Value(v) if v.0 == mul_dest.0)
                        || matches!(rhs, Operand::Value(v) if v.0 == mul_dest.0);
                    // Accumulator gate mirrors the fmsub/fmadd policy: fusing
                    // into a loop-carried accumulator lengthens the serial
                    // dependency chain.
                    if mul_used
                        && add_ty == mul_ty
                        && !accumulator_dests.contains(&add_dest.0)
                        && (fp_int.like(mul_ty)
                            || fp_contract.fuse_pair(
                                fp_tags.get(&mul_dest.0).copied(),
                                fp_tags.get(&add_dest.0).copied(),
                            ))
                    {
                        found = Some(j);
                    }
                    break;
                }
                _ => break,
            }
        }
        if let Some(add_idx) = found {
            out.push((idx, add_idx));
        }
    }
    out
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
        let Some(k) = shift_amt.to_i64() else {
            continue;
        };
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

fn detect_and_not_fusions(block: &BasicBlock, use_counts: &[u32]) -> FxHashSet<usize> {
    let mut and_indices = FxHashSet::default();
    for (idx, pair) in block.instructions.windows(2).enumerate() {
        let (not_dest, not_ty) = match &pair[0] {
            Instruction::UnaryOp {
                dest,
                op: crate::ir::reexports::IrUnaryOp::Not,
                ty,
                ..
            } if matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) => {
                (*dest, *ty)
            }
            _ => continue,
        };
        if use_counts.get(not_dest.0 as usize).copied().unwrap_or(0) != 1 {
            continue;
        }
        let Instruction::BinOp {
            op: IrBinOp::And,
            lhs,
            rhs,
            ty,
            ..
        } = &pair[1]
        else {
            continue;
        };
        if *ty == not_ty
            && (matches!(lhs, Operand::Value(v) if *v == not_dest)
                || matches!(rhs, Operand::Value(v) if *v == not_dest))
        {
            and_indices.insert(idx + 1);
        }
    }
    and_indices
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

    // Flush collected nested-function trampoline template/slot blocks.
    // They switch to .data.rel.ro and MUST live outside any function body:
    // interleaving data directives mid-function corrupted the peephole /
    // assembler stream at -O1 (label/instruction mapping shifted; see
    // 20000822-1). At the very end of the text section this is safe — the
    // trailing GNU-stack note below switches sections again anyway.
    for block in std::mem::take(&mut cg.state().trampoline_data_blocks) {
        cg.state().emit(&block);
        cg.state().emit("");
    }

    cg.state().emit("");
    // Nested-function trampolines execute code on the stack: mark the
    // stack executable (GCC does the same for trampoline-using units).
    if cg.state_ref().requires_executable_stack {
        cg.state().emit(".section .note.GNU-stack,\"x\",@progbits");
    } else {
        cg.state().emit(".section .note.GNU-stack,\"\",@progbits");
    }

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
    state
        .extern_function_symbols
        .extend(module.extern_function_symbols.iter().cloned());
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
            cg.state()
                .emit_fmt(format_args!(".file {} \"{}\"", id, escape_dwarf_path(name)));
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
            if let Some(&alignment) = module.function_alignments.get(&func.name) {
                if alignment > 1 && alignment.is_power_of_two() {
                    cg.state()
                        .emit_fmt(format_args!(".p2align {}", alignment.trailing_zeros()));
                }
            }
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
        "%rbp", "{rbp}", "%%rbp", "%ebp", "{ebp}", "%%ebp", "x29", "{x29}", "%x29", "%fp", "{fp}",
        "%%fp",
    ];
    NEEDLES.iter().any(|n| template.contains(n))
}

fn clobber_is_frame_pointer(c: &str) -> bool {
    let l = c.trim().trim_start_matches('%').to_ascii_lowercase();
    matches!(l.as_str(), "rbp" | "ebp" | "x29" | "fp")
}

/// `__memcpy_chk(dest, src, n, destlen)` may only be inlined when the
/// fortify destlen covers `n` (or is `(size_t)-1` / SIZE_MAX).
pub(crate) fn destlen_covers_n(destlen: &IrConst, n: i64) -> bool {
    match destlen.to_i64() {
        Some(-1) => true,
        Some(d) if d >= n => true,
        _ => false,
    }
}

/// `Some(n)` if this call is a fixed-size memcpy we may expand inline.
pub(crate) fn inline_memcpy_len(func: &str, args: &[Operand], is_variadic: bool) -> Option<usize> {
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

/// `Some(n)` if this call is a fixed-size `memset` / `__memset_chk` whose
/// byte count is a compile-time constant.  Whether it is *profitable* to
/// expand inline is the backend's call (`ArchCodegen::inline_memset_len`
/// consults the CPU tuning row); this helper only answers the shape
/// question so the x86 emitter and the MachInst typed-call gate agree on
/// exactly the same set of calls.  `__memset_chk` is admitted only when the
/// fortify `destlen` covers `n` (same rule as `inline_memcpy_len`).
pub(crate) fn inline_memset_const_len(func: &str, args: &[Operand], is_variadic: bool) -> Option<usize> {
    if is_variadic || args.len() < 3 {
        return None;
    }
    let n = match args.get(2) {
        Some(Operand::Const(c)) => c.to_i64().filter(|s| *s >= 0)?,
        _ => return None,
    };
    match func {
        "memset" => Some(n as usize),
        "__memset_chk" => {
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

/// Whether the result value of an inline-expanded libc call has any use.
/// `value_use_counts` is populated per function before instruction
/// selection; an absent entry means the value was never referenced.  A
/// conservative `true` is returned when the table is empty (e.g. a backend
/// that does not compute it) so the copy is never dropped by accident.
fn call_result_is_used(cg: &dyn ArchCodegen, v: &Value) -> bool {
    let counts = &cg.state_ref().value_use_counts;
    if counts.is_empty() {
        return true;
    }
    counts.get(v.0 as usize).copied().unwrap_or(0) != 0
}

/// x86-64 policy for `inline_memset_const_len`: the fixed-size fill is
/// expanded inline unless the active tuning row classifies it as `LibCall`
/// (above ¼ of the shared L3 on ERMS rows, 8 KiB otherwise — glibc's
/// non-temporal path wins there).  The libcall bound does not depend on the
/// vector width, so the 16-byte width is a faithful stand-in for the
/// `-march`-dependent width the emitter uses.  This one function is
/// consulted by the x86 emitter *and* by the register allocator's
/// parameter-home policy (`regalloc::x86_param_caller_homes_safe`), so the
/// two can never disagree about whether a call instruction is a real call.
pub(crate) fn x86_inline_memset_len(func: &str, args: &[Operand], is_variadic: bool) -> Option<usize> {
    use crate::backend::x86::cpu_model::{active, CopyStrategy};
    let n = inline_memset_const_len(func, args, is_variadic)?;
    match active().memset_strategy(n, 16) {
        CopyStrategy::LibCall => None,
        _ => Some(n),
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
                // We just left the text section via a RAW `.section` directive,
                // which does not update `current_text_section`. Without
                // invalidating it, `emit_switch_to_section` below would see the
                // stale text-section name, skip the switch back, and leave this
                // function's body inside the writable, non-executable
                // __patchable_function_entries section (reproduced on main:
                // `-fpatchable-function-entry=2,0` buried whole bodies there).
                cg.state().invalidate_text_section();
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

    // Function-entry mcount instrumentation (-pg / -mfentry / -mrecord-mcount /
    // -mnop-mcount). The site is the very first program point of the function
    // (after any patchable_function_entry 'after' NOPs, before any prologue
    // save), matching measured GCC 14.2 output. objtool --mcount reads the
    // __mcount_loc entries at link time and patches the site as DYNAMIC_FTRACE
    // needs. Skipped for: inline functions (no standalone body is emitted for
    // purely-inlined code), naked functions (no prologue to instrument; the
    // body is pure asm), and __attribute__((no_instrument_function)) — the
    // kernel marks early-boot / NMI / .noinstr.text functions that way because
    // the tracer infrastructure isn't mapped yet and calling it would
    // triple-fault. Classic (non-fentry) mcount is deferred to the backend
    // prologue: its ABI requires the frame to be established first.
    if !func.is_inline && !func.is_naked && !func.no_instrument {
        if let Some(mc) = cg.state().mcount {
            let classic = !mc.nop && !mc.use_fentry;
            if classic && !cg.supports_classic_mcount() {
                if !cg.state().mcount_unsupported_warned {
                    cg.state().mcount_unsupported_warned = true;
                    eprintln!(
                        "ccc: warning: classic -pg (call mcount) is not yet implemented \
                         for this target; use -mfentry/-mnop-mcount or disable ftrace"
                    );
                }
            } else if cg.supports_mcount() {
                let mcount_label = if mc.record {
                    Some(format!(".LMC{}", cg.state().next_label_id()))
                } else {
                    None
                };
                // __mcount_loc entry: a pointer to the call site (mirrors
                // __patchable_function_entries' layout). Only emitted with
                // -mrecord-mcount (CONFIG_FTRACE_MCOUNT_USE_CC=y); without it,
                // objtool finds the call by scanning.
                if mc.record {
                    let lbl = mcount_label.as_deref().unwrap();
                    cg.state().emit_fmt(format_args!(
                        ".section __mcount_loc,\"a\",@progbits,{}",
                        lbl
                    ));
                    // Raw non-text `.section` emission: invalidate so the
                    // switch back below cannot be skipped (same class of bug
                    // as the PFE section leak — skipping it would leave the
                    // mcount site AND the entire function body in the
                    // non-executable __mcount_loc section).
                    cg.state().invalidate_text_section();
                    let ptr_align = crate::common::types::target_ptr_size();
                    let ptr_dir = cg.ptr_directive();
                    cg.state().emit_fmt(format_args!(".align {}", ptr_align));
                    cg.state()
                        .emit_fmt(format_args!("{} {}", ptr_dir.as_str(), lbl));
                    emit_switch_to_section(cg, &func_sect);
                }
                if mc.nop {
                    // 5-byte NOP, the exact encoding the kernel mandates
                    // (arch/x86/include/asm/nops.h GENERIC_NOP5 =
                    // 0f 1f 44 00 00). The runtime patcher overwrites these
                    // five bytes with a 5-byte `call ftrace_caller`. `.byte`
                    // is used instead of `nopl 0(%rax,%rax,1)` because the
                    // assembler peephole may shorten a zero-displacement SIB
                    // form to 4 bytes, leaving the patcher one byte short and
                    // corrupting the next instruction.
                    if let Some(lbl) = &mcount_label {
                        cg.state().emit_fmt(format_args!("{}:", lbl));
                    }
                    cg.state().emit(".byte 0x0f, 0x1f, 0x44, 0x00, 0x00");
                } else if mc.use_fentry {
                    if let Some(lbl) = &mcount_label {
                        cg.state().emit_fmt(format_args!("{}:", lbl));
                    }
                    cg.state().emit("call __fentry__");
                } else {
                    // Classic `call mcount`: deferred until AFTER the frame is
                    // set up (push %rbp; mov %rsp,%rbp) — the mcount ABI reads
                    // the parent PC through the frame; GCC measures to the
                    // same shape and rejects -pg with -fomit-frame-pointer.
                    // The label is emitted at the site only when recording.
                    cg.state().pending_classic_mcount_label = mcount_label
                        .or_else(|| Some(format!(".LMC{}", cg.state().next_label_id())));
                }
            } else if !cg.state().mcount_unsupported_warned {
                cg.state().mcount_unsupported_warned = true;
                eprintln!(
                    "ccc: warning: -pg function-entry instrumentation is not yet \
                     implemented for this target; no mcount/__fentry__ sites are emitted"
                );
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
    let has_frame_address = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            matches!(
                inst,
                Instruction::Intrinsic {
                    op: crate::ir::intrinsics::IntrinsicOp::FrameAddress
                        | crate::ir::intrinsics::IntrinsicOp::BuiltinSetjmp,
                    ..
                }
            )
        })
    });
    let has_vector_intrinsics = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            matches!(inst, Instruction::Intrinsic { op, .. }
            if matches!(op,
                crate::ir::intrinsics::IntrinsicOp::FmaF64x4
                | crate::ir::intrinsics::IntrinsicOp::FmaF64x4Hoisted
                | crate::ir::intrinsics::IntrinsicOp::FmaF64x4SIB
                | crate::ir::intrinsics::IntrinsicOp::FmaF64x4HoistedSIB
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
    // Frame-pointer omission is only legal when the function body can be
    // addressed purely %rsp-relative: no dynamic alloca (the frame size is no
    // longer a compile-time constant), no inline asm that reads %rbp-relative
    // operands, and no vector intrinsics that rely on the frame pointer. The
    // user's `-fomit-frame-pointer`/`-fno-omit-frame-pointer` request is the
    // gate; previously the CLI flag was dropped entirely (so
    // `-fno-omit-frame-pointer` silently did nothing) and variadic functions
    // were unconditionally pinned to a frame pointer.
    // Calls passing an argument whose natural alignment exceeds 16 bytes
    // need the caller-side dynamic %rsp realignment (SysV AMD64, GCC ≥ 4.6
    // ABI — see emit_call_stack_args_impl). The runtime realignment delta
    // forbids %rsp-relative slot addressing for the rest of the function,
    // so such functions keep a frame pointer. Conservative (register-passed
    // over-aligned structs force it too): the shape is ABI-exotic either
    // way and the frame pointer costs nothing semantically.
    let has_overaligned_call_args = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| match inst {
            Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                info.struct_arg_aligns.iter().any(|a| a.unwrap_or(0) > 16)
            }
            _ => false,
        })
    });
    cg.state().omit_frame_pointer = cg.state().fpo_requested
        && !has_dyn_alloca
        && !has_inline_asm_fp
        && !has_frame_address
        && !has_vector_intrinsics
        && !has_overaligned_call_args
        // Classic `call mcount` (non-fentry -pg) reads the parent PC through
        // the frame; GCC rejects `-pg` together with `-fomit-frame-pointer`.
        // Keep a frame pointer so the deferred prologue site is well-defined.
        && cg.state().pending_classic_mcount_label.is_none();

    cg.state().current_func_name = func.name.clone();
    let raw_space = cg.calculate_stack_space(func);
    let frame_size = cg.aligned_frame_size(raw_space);
    cg.state().frame_size = frame_size;
    cg.emit_prologue(func, frame_size);
    cg.emit_store_params(func);

    let entry_label = func.blocks.first().map(|b| b.label);

    let value_use_counts = count_value_uses(func);
    cg.state().value_use_counts = value_use_counts.clone();

    // Loop-carried accumulator destinations for the fmsub gate (levkropp
    // e3b21b8f, audited). Fusing `acc -= a*b` into fmsub puts the multiply
    // on the accumulator's serial dependency chain (fmsub latency > fsub),
    // measured 13% regression on nbody. Recognized accumulators: memory-
    // promoted loop F64 phis, surviving Phi dests, multi-Copy dests (the
    // phi-web shape after phi elimination), and — transitively — values
    // COPIED INTO any accumulator (the backedge source sits on the same
    // serial chain). The reverse closure is a fixpoint over Copy edges;
    // it terminates because each round strictly grows a set bounded by
    // the number of distinct value ids in the function.
    let accumulator_dests: FxHashSet<u32> = {
        let mut accs: FxHashSet<u32> = func.loop_promoted_f64_values.iter().map(|v| v.0).collect();
        let mut copy_edges: Vec<(u32, u32)> = Vec::new(); // (dest, src)
        let mut copy_def_count: FxHashMap<u32, u32> = FxHashMap::default();
        for b in &func.blocks {
            for inst in &b.instructions {
                match inst {
                    Instruction::Phi { dest, .. } => {
                        accs.insert(dest.0);
                    }
                    Instruction::Copy { dest, src } => {
                        if let Operand::Value(sv) = src {
                            copy_edges.push((dest.0, sv.0));
                        }
                        let c = copy_def_count.entry(dest.0).or_insert(0);
                        *c += 1;
                        if *c == 2 {
                            accs.insert(dest.0);
                        }
                    }
                    _ => {}
                }
            }
        }
        // Reverse closure over the (small) copy-edge list instead of
        // re-walking every instruction per round: O(rounds * edges).
        loop {
            let mut grew = false;
            for &(d, src) in &copy_edges {
                if accs.contains(&d) && accs.insert(src) {
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        accs
    };

    let stab = analyze_base_stability(func);
    let gep_fold_map = build_gep_fold_map(func, &value_use_counts, &stab);
    let indexed_gep_map = if cg.supports_indexed_addr() {
        build_indexed_gep_map(func, &value_use_counts, &stab)
    } else {
        FxHashMap::default()
    };
    // Values that are link-time integer CONSTANTS (Copy/Cast of Const, and
    // GEP of such a constant plus a constant offset). Segment-override
    // loads/stores through them use the direct `movq %fs:OFF` form —
    // glibc's THREAD_SELF/GETMEM/SETMEM all funnel through this shape, and
    // the register-indirect fallback costs 3 instructions per access.
    let const_addr_vals: FxHashMap<u32, i64> = {
        let mut m: FxHashMap<u32, i64> = FxHashMap::default();
        let as_const = |op: &Operand, m: &FxHashMap<u32, i64>| -> Option<i64> {
            match op {
                Operand::Const(c) => ir_const_as_i64(c),
                Operand::Value(v) => m.get(&v.0).copied(),
            }
        };
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Copy { dest, src } => {
                        if let Some(v) = as_const(src, &m) {
                            m.insert(dest.0, v);
                        }
                    }
                    Instruction::Cast { dest, src, .. } => {
                        if let Some(v) = as_const(src, &m) {
                            m.insert(dest.0, v);
                        }
                    }
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => {
                        if let (Some(b), Some(o)) = (m.get(&base.0).copied(), as_const(offset, &m))
                        {
                            m.insert(dest.0, b.wrapping_add(o));
                        }
                    }
                    _ => {}
                }
            }
        }
        m
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

    let remat_global_addrs = if cg.supports_global_addr_remat() {
        build_rematerializable_global_addr_set_for(func, &global_addr_map)
    } else {
        FxHashSet::default()
    };
    let mut dead_global_addrs = if cg.supports_global_addr_fold() {
        build_foldable_global_addr_set(func, &global_addr_map)
    } else {
        FxHashSet::default()
    };
    dead_global_addrs.extend(remat_global_addrs.iter().copied());
    // PF-07: the prologue promoted PIC indexed-symbol bases (defined at a
    // shallower loop depth than their consumers) out of the
    // never-materialized set so RA homes them. Honour that here: skipping
    // emission would leave the SIB base register unwritten.
    for v in &cg.state_ref().promoted_global_addr_homes {
        dead_global_addrs.remove(v);
    }

    // Indexed-fold guaranteed-offset-producer skipping (session 27): when an
    // indexed GEP folds into a Load's SIB memory operand, the GEP's offset
    // chain (the Shl/Mul/Add-doubling scaling composition) loses its only
    // use — yet the IR still holds it, so the backend would emit the whole
    // 3-5 instruction offset materialisation inside the hot loop.  Skip those
    // single-use producers here.
    //
    // SOUNDNESS GUARD: only GEPs consumed EXCLUSIVELY by LOADs qualify.  The
    // indexed STORE hook can still refuse emission at the access site (the
    // value-staging class: accumulator staging would clobber the base/index),
    // in which case `rematerialize_skipped_indexed` re-emits the GEP from
    // `base + orig_offset` — the offset producers must exist for that.
    //
    // Load→cmp-mem priority: an indexed GEP that feeds a load→cmp-mem
    // candidate must be kept OUT of both the skip set and the dead-producer
    // set, because the memory-compare fold needs the pointer materialised in
    // a register/slot to emit `cmpX $imm,(ptr)`.  cmp-mem outranks SIB.
    let load_cmp_ptrs_func: FxHashSet<u32> = if cg.supports_load_cmp_mem_fold() {
        let mut s: FxHashSet<u32> = FxHashSet::default();
        for block in &func.blocks {
            for (_dest, (ptr, _ty, _imm)) in detect_load_cmp_mem_fold(block, &value_use_counts) {
                s.insert(ptr);
            }
        }
        s
    } else {
        FxHashSet::default()
    };
    let mut idx_dead_producers: FxHashSet<u32> = FxHashSet::default();
    if cg.supports_indexed_addr() && !indexed_gep_map.is_empty() {
        let mut foldable_folds: FxHashSet<u32> = FxHashSet::default();
        for (dest, info) in indexed_gep_map.iter() {
            if load_cmp_ptrs_func.contains(dest) {
                continue; // pointer needed materialised by a cmp-mem fold
            }
            if can_indexed_addr_fold(cg, info, &global_addr_map) {
                foldable_folds.insert(*dest);
            }
        }
        // Indexed-fold GlobalAddr BASES: a symbol whose EVERY use is the
        // base of a guaranteed-folded indexed GEP never materializes — the
        // SIB memory operand embeds the symbol directly (sym(,%idx,scale)).
        // Without this, vsprintf number()'s digit table kept a dead
        // `movl $digits, slot` inside the loop after the load folded.
        // Guard: ALL of the base's uses must be bases of foldable folds
        // (use-count equality; a base shared with any other consumer — a
        // direct Load, an unfolded GEP, a Store value — still materializes).
        // Store-fed folds keep their base: the value-staging fallback
        // (`rematerialize_skipped_indexed`) may re-emit the GEP and read it.
        {
            let mut folded_base_uses: FxHashMap<u32, u32> = FxHashMap::default();
            for dest in &foldable_folds {
                if let Some(info) = indexed_gep_map.get(dest) {
                    if global_addr_map.contains_key(&info.base.0) {
                        *folded_base_uses.entry(info.base.0).or_insert(0) += 1;
                    }
                }
            }
            let mut has_store_consumer0: FxHashSet<u32> = FxHashSet::default();
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::Store { ptr, .. } = inst {
                        if foldable_folds.contains(&ptr.0) {
                            if let Some(info) = indexed_gep_map.get(&ptr.0) {
                                has_store_consumer0.insert(info.base.0);
                            }
                        }
                    }
                }
            }
            for (base, folded_uses) in folded_base_uses {
                let total = value_use_counts[base as usize];
                if total != folded_uses || has_store_consumer0.contains(&base) {
                    continue;
                }
                // SYM-FORM GUARANTEE: the materialisation may be skipped
                // only when the base value has NO register home. A homed
                // base makes `can_indexed_addr_fold` fold via the
                // REGISTER-base arm, and the emitter's first dispatch
                // (`emit_load_indexed`) then consumes the base register in
                // the SIB operand — a register the skipped `leaq/movl`
                // would have written (narrow_compare_constant_semantics:
                // `movzwl (%rbx,%r14,2)` read a never-written %rbx). An
                // UNHOMED base cannot take the register form (GlobalAddrs
                // are not allocas, so the frame-SIB arm refuses too), and
                // the fold only became foldable through the symbol arm —
                // which had already verified `supports_indexed_sym_base`,
                // `rip_rel_blocked` (the emitter's GOT verdict agrees, see
                // emit_load_indexed_sym_impl) and a register-resident
                // index. The symbol form therefore cannot refuse, and it
                // never reads the base's (now never-written) slot.
                if cg.get_phys_reg_for_value(base).is_none() {
                    dead_global_addrs.insert(base);
                }
            }
        }
        let mut has_store_consumer: FxHashSet<u32> = FxHashSet::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Store { ptr, .. } = inst {
                    if foldable_folds.contains(&ptr.0) {
                        has_store_consumer.insert(ptr.0);
                    }
                }
            }
        }
        let single_defs = index_single_defs(func, &stab);
        // Fixed-point dead-offset analysis. The old walk required
        // `use_count == 1` per chain node, which stops at SHARED offsets:
        // `a[i]; b[i]` both GEP the same `i*8` value (use_count 2), and the
        // scaled-offset producer stayed live as dead code after both GEPs
        // folded (linux_find_bit: `movq %r13,%rbx; shlq $3,%rbx` per scanned
        // word). Instead, count uses ATTRIBUTABLE to the fold: (a) offset
        // operands of foldable GEPs, (b) operand positions of other dead
        // chain nodes. A node is dead iff every one of its uses is
        // fold-attributable. The walk still stops at the fold's index value:
        // the terminal index is live by construction (the RA keeps it
        // register-resident to the access).
        let mut fold_uses: FxHashMap<u32, u32> = FxHashMap::default();
        for dest in &foldable_folds {
            if let Some(info) = indexed_gep_map.get(dest) {
                *fold_uses.entry(info.orig_offset.0).or_insert(0) += 1;
            }
        }
        for (dest, info) in indexed_gep_map.iter() {
            if !foldable_folds.contains(dest) || has_store_consumer.contains(dest) {
                continue;
            }
            let index_val = info.index.0;
            let mut stack = vec![info.orig_offset.0];
            while let Some(v) = stack.pop() {
                if v == index_val {
                    continue;
                }
                if idx_dead_producers.contains(&v) {
                    continue;
                }
                let total = value_use_counts.get(v as usize).copied().unwrap_or(0);
                let attributed = fold_uses.get(&v).copied().unwrap_or(0);
                if attributed < total {
                    // Some use is outside the folded chains: keep the node
                    // (and do not traverse through it — its operands may
                    // still be live for those other uses).
                    continue;
                }
                idx_dead_producers.insert(v);
                if let Some(inst) = single_defs.get(&v) {
                    for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(x) = op {
                            stack.push(x.0);
                            // This operand use is now attributable to the
                            // fold (its consumer is dead).
                            *fold_uses.entry(x.0).or_insert(0) += 1;
                        }
                    });
                }
            }
        }
    }

    let emit_debug = cg.state_ref().debug_info && source_mgr.is_some() && !file_table.is_empty();
    let mut last_debug_file: u32 = 0;
    let mut last_debug_line: u32 = 0;
    cg.state().current_program_point = 0;

    for (block_idx, block) in func.blocks.iter().enumerate() {
        if Some(block.label) != entry_label {
            cg.state().reg_cache.invalidate_all();
            cg.flush_pending_vec_store();
            cg.flush_x87_pending();
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

        let fuse_idx = detect_cmp_branch_fusion(
            block,
            &value_use_counts,
            cg.supports_fused_fp_cmp_branch(),
            cg.supports_fused_bit_test_branch(),
        );
        let mul_add_fusions = detect_mul_add_fusions(
            block,
            &value_use_counts,
            cg.supports_fused_float_mul_add(),
            cg.fp_contract(),
            &func.fp_expr_tags,
            cg.supports_fused_int_mul_sub(),
            if cg.supports_fused_float_mul_sub() && !env_flag_set("CCC_NO_FMSUB") {
                Some(&accumulator_dests)
            } else {
                None
            },
        );
        // Gap-fused fmadd (levkropp 2e57bcf2, audited port): Mul, [Load/GEP
        // x1-2], Add. The fmadd is emitted at the Add, so the multiply
        // operands are read LATER than their IR liveness recorded. Two alias
        // hazards must be rejected, not one: (a) a gap destination's PHYSICAL
        // register aliasing a mul operand's register (levkropp's check), and
        // (b) a gap destination's STACK SLOT aliasing a slot-homed mul
        // operand -- Tier-2 slot coloring can legally pack a value born in
        // the gap into the slot of a value whose IR liveness ended at the
        // Mul (his check missed this class entirely).
        let mut gap_mul_skips: FxHashSet<usize> = FxHashSet::default();
        let mut gap_add_fusions: FxHashMap<usize, usize> = FxHashMap::default();
        for &(mul_idx, add_idx) in &detect_gap_fma_fusions(
            block,
            &value_use_counts,
            cg.supports_fused_float_mul_add(),
            &accumulator_dests,
            cg.fp_contract(),
            &func.fp_expr_tags,
        ) {
            let Instruction::BinOp {
                lhs: mul_lhs,
                rhs: mul_rhs,
                ..
            } = &block.instructions[mul_idx]
            else {
                continue;
            };
            let mut clash = false;
            for g in (mul_idx + 1)..add_idx {
                let gap_dest = match &block.instructions[g] {
                    Instruction::Load { dest, .. } => Some(*dest),
                    Instruction::GetElementPtr { dest, .. } => Some(*dest),
                    _ => None,
                };
                let Some(gd) = gap_dest else { continue };
                let gap_phys = cg.get_phys_reg_for_value(gd.0);
                let gap_slot = cg.state_ref().get_slot(gd.0);
                for op in [mul_lhs, mul_rhs] {
                    if let Operand::Value(v) = op {
                        let op_phys = cg.get_phys_reg_for_value(v.0);
                        if gap_phys.is_some() && op_phys == gap_phys {
                            clash = true;
                        }
                        // Slot aliasing only matters when the operand will be
                        // RELOADED from its slot at the Add (no register home).
                        if op_phys.is_none() {
                            if let (Some(gs), Some(os)) = (gap_slot, cg.state_ref().get_slot(v.0)) {
                                if gs.0 == os.0 {
                                    clash = true;
                                }
                            }
                        }
                    }
                }
            }
            if !clash {
                gap_mul_skips.insert(mul_idx);
                gap_add_fusions.insert(add_idx, mul_idx);
            }
        }
        let cmp_select_fusions =
            if cg.supports_fused_cmp_select() && !env_flag_set("CCC_NO_FUSED_CSEL") {
                detect_cmp_select_fusion(block, &value_use_counts)
            } else {
                Vec::new()
            };
        let conditional_increment_fusions =
            if cg.supports_conditional_increment_select() && !env_flag_set("CCC_NO_CSINC_FOLD") {
                detect_conditional_increment_selects(block, &value_use_counts, &stab)
            } else {
                Vec::new()
            };
        let conditional_increment_adds: FxHashSet<usize> = conditional_increment_fusions
            .iter()
            .map(|fusion| fusion.add_idx)
            .collect();
        let conditional_increment_cmps: FxHashSet<usize> = conditional_increment_fusions
            .iter()
            .filter_map(|fusion| fusion.cmp_idx)
            .collect();
        let shifted_logical_fusions = if cg.supports_shifted_logical() {
            detect_shifted_logical_fusions(block, &value_use_counts)
        } else {
            FxHashSet::default()
        };
        let and_not_fusions = if cg.supports_and_not() {
            detect_and_not_fusions(block, &value_use_counts)
        } else {
            FxHashSet::default()
        };
        let load_cmp_folds = if cg.supports_load_cmp_mem_fold() {
            detect_load_cmp_mem_fold(block, &value_use_counts)
        } else {
            FxHashMap::default()
        };
        // Pointer ids of load→cmp-mem candidates.  An indexed GEP that feeds
        // such a load must NOT be skipped: the memory-compare fold needs the
        // pointer materialised (the cmp emits `cmpX $imm,(ptr)` directly and
        // cannot resolve a folded-away address).  cmp-mem outranks SIB
        // indexing — `cmpb $imm,(mem)` beats `movsbl+cmpl` by more than the
        // SIB fold saves (early_serial_console boot corpus).
        let load_cmp_ptrs: FxHashSet<u32> =
            load_cmp_folds.values().map(|(ptr, _, _)| *ptr).collect();
        let mut fused_add_skip: FxHashSet<usize> = FxHashSet::default();
        let mut skip_fused_logical = false;
        let mut skip_fused_and_not = false;
        let mut skip_f128_copy_store = false;

        cg.state().block_use_counts.clear();
        cg.state().pending_load_cmp.clear();
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *cg.state().block_use_counts.entry(v.0).or_insert(0) += 1;
                }
            });
            // Mirror count_value_uses exactly.  The former hand-written
            // match omitted Intrinsic::dest_ptr (so every local vecreg looked
            // live-out and was needlessly spilled) and double-counted atomic
            // pointer operands (which could hide a genuine later-block use).
            for_each_value_use_in_instruction(inst, |v| {
                *cg.state().block_use_counts.entry(v.0).or_insert(0) += 1;
            });
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
            if conditional_increment_adds.contains(&idx)
                || conditional_increment_cmps.contains(&idx)
            {
                cg.state().current_program_point += 1;
                continue;
            }
            if fused_add_skip.contains(&idx) {
                cg.state().current_program_point += 1;
                continue;
            }
            // Gap-fused fmadd: the Mul is computed by the fmadd emitted at
            // the Add (below), after the gap Load/GEP ran in program order.
            if gap_mul_skips.contains(&idx) {
                cg.state().current_program_point += 1;
                continue;
            }
            if let Some(&mul_idx) = gap_add_fusions.get(&idx) {
                if let (
                    Instruction::BinOp {
                        dest: mul_dest,
                        lhs: mul_lhs,
                        rhs: mul_rhs,
                        ty: mul_ty,
                        ..
                    },
                    Instruction::BinOp {
                        dest: add_dest,
                        lhs: add_lhs,
                        rhs: add_rhs,
                        ..
                    },
                ) = (&block.instructions[mul_idx], inst)
                {
                    let mul_is_lhs = matches!(add_lhs, Operand::Value(v) if v.0 == mul_dest.0);
                    let acc_op = if mul_is_lhs { add_rhs } else { add_lhs };
                    cg.flush_machinst();
                    cg.emit_fused_mul_add(mul_dest, mul_lhs, mul_rhs, acc_op, add_dest, *mul_ty);
                    cg.state().current_program_point += 1;
                    continue;
                }
            }
            if skip_fused_logical {
                skip_fused_logical = false;
                cg.state().current_program_point += 1;
                continue;
            }
            if skip_fused_and_not {
                skip_fused_and_not = false;
                cg.state().current_program_point += 1;
                continue;
            }
            if skip_f128_copy_store {
                skip_f128_copy_store = false;
                cg.state().current_program_point += 1;
                continue;
            }

            // Exact F128 load->store is a 16-byte copy; bypass x87/f64
            // approximation and intermediate homes on every backend.
            if let Instruction::Load {
                dest,
                ptr: src,
                ty: IrType::F128,
                volatile: false,
                ..
            } = inst
            {
                if value_use_counts.get(dest.0 as usize).copied() == Some(1) {
                    if let Some(Instruction::Store {
                        val: Operand::Value(v),
                        ptr: dst,
                        ty: IrType::F128,
                        volatile: false,
                        ..
                    }) = block.instructions.get(idx + 1)
                    {
                        if v == dest
                            && cg.state_ref().resolve_slot_addr(src.0).is_some()
                            && cg.state_ref().resolve_slot_addr(dst.0).is_some()
                        {
                            cg.flush_machinst();
                            cg.emit_memcpy(dst, src, 16);
                            skip_f128_copy_store = true;
                            cg.state().current_program_point += 1;
                            continue;
                        }
                    }
                }
            }

            // Skip address producers whose result is folded. Use the
            // *ultimate* composed base (not the instruction's immediate
            // operand) so GEP(GEP(alloca,c1),c2) is skipped iff the load
            // can fold against alloca+c1+c2.
            if let Some(dest) = inst.dest() {
                if std::env::var_os("LCCC_DBG_FOLD").is_some() {
                    eprintln!(
                        "[FOLD] inst dest={} in_gep_map={} in_idx_map={} dead_gaddr={}",
                        dest.0,
                        gep_fold_map.get(&dest.0).is_some(),
                        indexed_gep_map.get(&dest.0).is_some(),
                        dead_global_addrs.contains(&dest.0)
                    );
                }
                if let Some(info) = gep_fold_map.get(&dest.0) {
                    if can_const_addr_fold(cg, info) {
                        cg.state().folded_gep_values.insert(dest.0);
                        if std::env::var_os("LCCC_DBG_FOLD").is_some() {
                            eprintln!(
                                "[FOLD] SKIPPED dest={} base={} off={}",
                                dest.0, info.base.0, info.offset
                            );
                        }
                        cg.state().current_program_point += 1;
                        continue;
                    }
                }
                if let Some(info) = indexed_gep_map.get(&dest.0) {
                    if !load_cmp_ptrs.contains(&dest.0)
                        && can_indexed_addr_fold(cg, info, &global_addr_map)
                    {
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
                // Dead offset producers of guaranteed-folded indexed GEPs
                // (load-only consumers; see idx_dead_producers above).
                if idx_dead_producers.contains(&dest.0) {
                    cg.state().current_program_point += 1;
                    continue;
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

            // PF-15 deferred narrowing widens: every instruction other than
            // the two self-managing kinds flushes the recorded moves before
            // ANY lowering runs (fusion paths, MachInst queueing, and the
            // generate_instruction dispatch all bypass each other, so this
            // loop head is the only point that sees every instruction). A
            // Cast records or flushes inside its emitter; a Cmp folds the
            // pair into a narrow compare or flushes right here — including
            // when MachInst would have lowered it.
            if !matches!(inst, Instruction::Cmp { .. } | Instruction::Cast { .. }) {
                cg.flush_pending_widen();
            }
            if let Instruction::Cmp {
                dest,
                op: cmp_op,
                lhs: cmp_lhs,
                rhs: cmp_rhs,
                ..
            } = inst
            {
                if let Some((nl, nr, nty, nop)) =
                    cg.narrow_cmp_operands(dest.0, *cmp_op, cmp_lhs, cmp_rhs)
                {
                    // Folded: emit the narrow compare on the classic path
                    // (cmpb/cmpw). MachInst never sees the rewritten shape,
                    // so its per-instruction gates stay exactly as audited.
                    cg.flush_machinst();
                    cg.emit_cmp(dest, nop, &nl, &nr, nty);
                    cg.state().current_program_point += 1;
                    continue;
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
                    // Float always fuses (native fmadd/fmsub). Integer fuses
                    // when the temp is slot-homed, or unconditionally on
                    // targets where madd/msub cost the same as the mul
                    // (levkropp 9f064faa: one fewer instruction, freed reg).
                    if cg.get_phys_reg_for_value(dest.0).is_none()
                        || ty.is_float()
                        || cg.supports_fused_int_madd_reg()
                    {
                        if let Some(Instruction::BinOp {
                            dest: add_dest,
                            op: partner_op,
                            lhs: add_lhs,
                            rhs: add_rhs,
                            ty: add_ty,
                        }) = block.instructions.get(add_i)
                        {
                            let mul_is_lhs = matches!(add_lhs, Operand::Value(v) if v.0 == dest.0);
                            let acc_op = if mul_is_lhs { add_rhs } else { add_lhs };
                            cg.flush_machinst();
                            match partner_op {
                                IrBinOp::Add => {
                                    cg.emit_fused_mul_add(
                                        dest, lhs, rhs, acc_op, add_dest, *add_ty,
                                    );
                                }
                                IrBinOp::Sub => {
                                    // Detector only records float Subs when the
                                    // backend advertised supports_fused_float_mul_sub.
                                    // mul_is_lhs selects fnmsub (product - acc)
                                    // vs fmsub (acc - product).
                                    cg.emit_fused_mul_sub(
                                        dest, lhs, rhs, acc_op, add_dest, *add_ty, mul_is_lhs,
                                    );
                                }
                                _ => unreachable!("mul fusion partner must be Add or Sub"),
                            }
                            fused_add_skip.insert(add_i);
                            cg.state().current_program_point += 1;
                            continue;
                        }
                    }
                }
            }

            if let Instruction::UnaryOp {
                dest: not_dest,
                op: crate::ir::reexports::IrUnaryOp::Not,
                src: not_src,
                ty,
            } = inst
            {
                if and_not_fusions.contains(&(idx + 1)) {
                    if let Some(Instruction::BinOp { dest, lhs, rhs, .. }) =
                        block.instructions.get(idx + 1)
                    {
                        let not_is_lhs = matches!(lhs, Operand::Value(v) if v == not_dest);
                        let other = if not_is_lhs { rhs } else { lhs };
                        let direct_return = idx + 2 == block.instructions.len()
                            && matches!(
                                &block.terminator,
                                Terminator::Return(Some(Operand::Value(value))) if value == dest
                            );
                        cg.flush_machinst();
                        cg.emit_and_not(not_dest, not_src, other, dest, *ty, direct_return);
                        skip_fused_and_not = true;
                        cg.state().current_program_point += 1;
                        continue;
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

            if let Some(fusion) = conditional_increment_fusions
                .iter()
                .find(|fusion| fusion.select_idx == idx)
            {
                if let Instruction::Select { dest, cond, .. } = inst {
                    cg.flush_machinst();
                    let cmp_idx = fusion.cmp_idx.or_else(|| {
                        cmp_select_fusions
                            .iter()
                            .find(|&&(select_idx, _, _)| select_idx == idx)
                            .map(|&(_, cmp_idx, _)| cmp_idx)
                    });
                    if let Some(cmp_idx) = cmp_idx {
                        if let Instruction::Cmp {
                            op,
                            lhs,
                            rhs,
                            ty: cmp_ty,
                            ..
                        } = &block.instructions[cmp_idx]
                        {
                            cg.emit_fused_cmp_conditional_increment_select(
                                *op,
                                lhs,
                                rhs,
                                *cmp_ty,
                                &fusion.base,
                                fusion.increment_on_true,
                                dest,
                                fusion.increment_ty,
                            );
                        } else {
                            unreachable!("cmp-select fusion did not reference a Cmp");
                        }
                    } else {
                        cg.emit_conditional_increment_select(
                            dest,
                            cond,
                            &fusion.base,
                            fusion.increment_on_true,
                            fusion.increment_ty,
                        );
                    }
                    cg.state().current_program_point += 1;
                    continue;
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
                    // Indexed GEPs feeding this load were kept un-skipped
                    // (load_cmp_ptrs guard at the GEP skip site), so they do
                    // not block the fold; const-folded / global ptrs are
                    // still skipped producers the cmp cannot resolve.
                    if !gep_fold_map.contains_key(&ptr.0)
                        && (!indexed_gep_map.contains_key(&ptr.0) || load_cmp_ptrs.contains(&ptr.0))
                        && !global_addr_map.contains_key(&ptr.0)
                    {
                        cg.state()
                            .pending_load_cmp
                            .insert(dest.0, (ptr.0, *ty, *imm));
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
                &remat_global_addrs,
                &const_addr_vals,
            );
            cg.state().current_program_point += 1;
        }
        // Defensive PF-15 flush: a deferred widen whose single consumer
        // was eliminated after use counting must never leak between
        // blocks. No-op whenever every record was consumed or flushed.
        cg.flush_pending_widen();

        cg.flush_machinst();
        cg.flush_vecreg_liveout();

        cg.state().next_block_label = func.blocks.get(block_idx + 1).map(|b| b.label);
        if let Some((fi, _, kind)) = fuse_idx {
            if let Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } = &block.terminator
            {
                match (kind, &block.instructions[fi]) {
                    (
                        FusedBranchKind::Cmp,
                        Instruction::Cmp {
                            op, lhs, rhs, ty, ..
                        },
                    ) => {
                        cg.emit_fused_cmp_branch_blocks(
                            *op,
                            lhs,
                            rhs,
                            *ty,
                            *true_label,
                            *false_label,
                        );
                    }
                    (FusedBranchKind::BitTest, Instruction::BinOp { lhs, rhs, ty, .. }) => {
                        cg.emit_fused_bit_test_branch_blocks(
                            lhs,
                            rhs,
                            *ty,
                            *true_label,
                            *false_label,
                        );
                    }
                    _ => unreachable!("fuse_idx anchored on a non-fusable instruction"),
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

/// S11: rematerialising a skipped GEP rebuilds the address through
/// %rax, destroying the accumulator. A store whose value came from a
/// dead-producer-skipped indexed load holds that value ONLY in %rax at this
/// point — no register home, no stack slot, and its producing Load is gone
/// from the instruction stream — so the remat makes the store's operand
/// permanently unmaterializable and `operand_to_rax`'s hard gate ICEs
/// (20020402-1 @O1: `listSmall[posGreatest] = listElem[i]` with the index
/// fold refused). Push/pop the accumulator around the remat and re-register
/// the residency afterwards so the pending store can still read its value.
/// push/pop are flag-neutral, so a fused-Cmp handshake cannot be disturbed,
/// and the stack stays balanced regardless of what the remat emits.
fn remat_indexed_acc_safe(
    cg: &mut dyn ArchCodegen,
    val: &Operand,
    remat: impl FnOnce(&mut dyn ArchCodegen),
) {
    let protected = match val {
        Operand::Value(v) => {
            let st = cg.state_ref();
            !cg.is_value_reg_assigned(v.0)
                && st.get_slot(v.0).is_none()
                && (st.reg_cache.acc_has(v.0, st.is_alloca(v.0)) || st.is_accumulator_location(v.0))
        }
        Operand::Const(_) => false,
    };
    // The acc register name and push/pop width are per-pointer-size: the
    // protection helper is shared by the x86-64 and i686 codegen drivers
    // (generation.rs is arch-agnostic), and `pushq %rax` is not encodable
    // in 32-bit mode. The push shifts %rsp for the duration of the remat,
    // so on RSP-relative frames the slot-reference bookkeeping must shift
    // with it (slot_ref emits `(rsp_frame_size + off)(%rsp)`); on RBP
    // frames the field is unused and the bump is a no-op.
    let (slot, push_acc, pop_acc) = if crate::common::types::target_ptr_size() == 8 {
        (8i64, "    pushq %rax", "    popq %rax")
    } else {
        (4i64, "    pushl %eax", "    popl %eax")
    };
    if protected {
        cg.state().emit(push_acc);
        cg.state().out.rsp_frame_size += slot;
    }
    remat(cg);
    if protected {
        cg.state().out.rsp_frame_size -= slot;
        if let Operand::Value(v) = val {
            cg.state().emit(pop_acc);
            let is_alloca = cg.state_ref().is_alloca(v.0);
            cg.state().reg_cache.set_acc(v.0, is_alloca);
        }
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
    remat_global_addrs: &FxHashSet<u32>,
    const_addr_vals: &FxHashMap<u32, i64>,
) {
    // PF-15 deferred narrowing widens: every instruction other than the
    // two self-managing kinds flushes the recorded moves first, so the
    // defer is invisible to all non-matching consumers. A Cast records or
    // flushes inside its emitter (try_record_pending_widen's refusal
    // contract); a Cmp folds the pair into a narrow compare or flushes in
    // its own dispatch arm below — including when MachInst lowers it.
    if !matches!(inst, Instruction::Cmp { .. } | Instruction::Cast { .. }) {
        cg.flush_pending_widen();
    }
    match inst {
        // GNU C nested-function support (static chain / trampoline /
        // non-local goto). x86-only; the trait defaults fail closed on
        // other targets.
        Instruction::GetStaticChain { dest } => {
            cg.emit_get_static_chain(dest);
            clobber_after_call_like(cg);
        }
        Instruction::SetStaticChain { src } => {
            cg.emit_set_static_chain(src);
            clobber_after_call_like(cg);
        }
        Instruction::InitTrampoline {
            buffer,
            chain,
            func,
        } => {
            cg.emit_init_trampoline(buffer, chain, func);
            clobber_after_call_like(cg);
        }
        Instruction::NonlocalGotoSave {
            frame,
            rbp_off,
            rsp_off,
        } => {
            cg.emit_nonlocal_goto_save(frame, *rbp_off, *rsp_off);
            clobber_after_call_like(cg);
        }
        Instruction::NonlocalGoto {
            chain,
            up,
            rbp_off,
            rsp_off,
            label,
        } => {
            cg.emit_nonlocal_goto(chain, *up, *rbp_off, *rsp_off, label);
            clobber_after_call_like(cg);
        }
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
            volatile,
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
                const_addr_vals,
            );
        }
        Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => {
            // A rematerializable GlobalAddr has deliberately no home. Rebuild
            // it only at this audited address derivation; every other use shape
            // was rejected before allocation.
            let remat = match (op, lhs, rhs) {
                (IrBinOp::Add, Operand::Value(base), offset)
                    if remat_global_addrs.contains(&base.0) =>
                {
                    global_addr_map.get(&base.0).map(|sym| (sym, offset, false))
                }
                (IrBinOp::Add, offset, Operand::Value(base))
                    if remat_global_addrs.contains(&base.0) =>
                {
                    global_addr_map.get(&base.0).map(|sym| (sym, offset, false))
                }
                (IrBinOp::Sub, Operand::Value(base), offset)
                    if remat_global_addrs.contains(&base.0) =>
                {
                    global_addr_map.get(&base.0).map(|sym| (sym, offset, true))
                }
                _ => None,
            };
            if let Some((sym, offset, subtract)) = remat {
                assert!(
                    cg.emit_rematerialized_global_addr(dest, sym, offset, subtract),
                    "backend accepted GlobalAddr rematerialisation but refused its audited use"
                );
            } else {
                // per_cpu_ptr()-style Add: Cast(GlobalAddr) base (symbol) +
                // register-resident offset. The Cast result IS in
                // global_addr_map (copy edges are followed for same-size
                // non-float casts) but is NOT rematerializable (the remat set
                // only roots original GlobalAddr defs with allowed uses), so
                // the remat path above doesn't fire and emit_binop would
                // strand the dest without a register home (the Cast result is
                // slot-resident) → operand_to_rax ICE
                // (workqueue_prepare_cpu: "value N has no register home").
                // Fold the symbol base + register index into a SIB leaq,
                // mirroring the GEP fold above (commit 804ce8c upstream).
                let mut folded = false;
                if *op == IrBinOp::Add && !ty.is_float() && ty.size() == 8 {
                    for (base, index) in [(lhs, rhs), (rhs, lhs)] {
                        if let (Operand::Value(b), Operand::Value(idx)) = (base, index) {
                            if let Some(sym) = global_addr_map.get(&b.0) {
                                if !rip_rel_blocked(cg, sym)
                                    && cg.emit_leaq_sym_index(dest, sym, idx, 0, 0)
                                {
                                    folded = true;
                                    break;
                                }
                            }
                        }
                    }
                }
                if !folded {
                    cg.emit_binop(dest, *op, lhs, rhs, *ty);
                    if is_wide_int_type(*ty) {
                        cg.state().reg_cache.invalidate_all();
                    }
                }
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
            // PF-15 consume happened at the per-instruction loop head (the
            // only point all lowering paths share). Deferred widens are
            // materialized or folded by the time we get here.
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
            if remat_global_addrs.contains(&base.0) {
                let sym = global_addr_map
                    .get(&base.0)
                    .expect("rematerializable GlobalAddr must retain its legal symbol identity");
                assert!(
                    cg.emit_rematerialized_global_addr(dest, sym, offset, false),
                    "backend accepted GlobalAddr rematerialisation but refused its audited GEP"
                );
            } else {
                if let Operand::Value(off_val) = offset {
                    cg.state()
                        .gep_base_offset
                        .insert(dest.0, (base.0, off_val.0));
                }
                // per_cpu()-style GEP: GlobalAddr base (symbol) + register
                // offset. The GlobalAddr base has no register home (it is
                // materialised to a slot), so the default emit_gep's
                // register-offset path strands the dest and the consumer
                // hits the operand_to_rax "no register home" ICE
                // (workqueue_prepare_cpu / cpu_to_node). Fold the symbol
                // base into a SIB leaq directly, mirroring
                // emit_load_indexed_sym_impl but emitting `leaq` (address
                // compute) instead of `movq` (load).
                let mut folded = false;
                if let Operand::Value(off_val) = offset {
                    if let Some(sym) = global_addr_map.get(&base.0) {
                        if !rip_rel_blocked(cg, sym)
                            && cg.emit_leaq_sym_index(dest, sym, off_val, 0, 0)
                        {
                            folded = true;
                        }
                    }
                }
                if !folded {
                    cg.emit_gep(dest, base, offset);
                }
            }
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
            ..
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
                const_addr_vals,
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
                // memcpy / __memcpy_chk return dest.  The backend stores it
                // (it must be captured before the expansion clobbers the
                // operand registers); an unused result is not materialised.
                let result = info.dest.filter(|d| call_result_is_used(cg, d));
                cg.emit_inline_memcpy_call(&info.args[0], &info.args[1], size, result.as_ref());
                clobber_after_call_like(cg);
                return;
            }
            // Fixed-size memset / __memset_chk: the backend decides per CPU
            // tuning row (X86Tune::memset_strategy) whether straight-line
            // stores, a vector loop or `rep stosb` beat the libc call; it
            // answers None for sizes above the L3-derived libcall bound.
            if let Some(size) = cg.inline_memset_len(func.as_str(), &info.args, info.is_variadic) {
                let result = info.dest.filter(|d| call_result_is_used(cg, d));
                cg.emit_inline_memset_call(&info.args[0], &info.args[1], size, result.as_ref());
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
            // A 128-bit va_arg result is definitionally a 128-bit value:
            // without this marker, later pair loads via operand_to_x0_x1
            // and friends would read only the low 8 bytes and zero-extend
            // (the is_i128_value fast path would never fire).
            if is_i128_type(*result_ty) {
                cg.state().i128_values.insert(dest.0);
            }
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
            align,
            ref eightbyte_classes,
        } => {
            cg.emit_va_arg_struct_ex(dest_ptr, va_list_ptr, *size, *align, eightbyte_classes);
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
                    crate::backend::state::SlotAddr::Reg(reg) => cg.emit_reg_to_acc(reg),
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

/// Integer view of an IR constant for address-constant tracking.
fn ir_const_as_i64(c: &IrConst) -> Option<i64> {
    match c {
        IrConst::I8(v) => Some(*v as i64),
        IrConst::I16(v) => Some(*v as i64),
        IrConst::I32(v) => Some(*v as i64),
        IrConst::I64(v) => Some(*v),
        IrConst::Zero => Some(0),
        _ => None,
    }
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
    const_addr_vals: &FxHashMap<u32, i64>,
) {
    if seg_override != AddressSpace::Default {
        if let Some(sym) = global_addr_map.get(&ptr.0) {
            if !rip_rel_blocked(cg, sym) {
                cg.emit_seg_load_symbol(dest, sym, ty, seg_override);
                return;
            }
        }
        // Constant offset (glibc TLS macros): direct one-instruction form.
        if let Some(&addr) = const_addr_vals.get(&ptr.0) {
            if cg.emit_seg_load_const_addr(dest, addr, ty, seg_override) {
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
        if !is_wide_int_type(ty) && can_indexed_addr_fold(cg, info, global_addr_map) {
            if cg.emit_load_indexed(dest, &info.base, &info.index, info.shift, info.disp, ty) {
                return;
            }
            // Symbol-base form: the GlobalAddr base has no register home.
            if let Some(sym) = global_addr_map.get(&info.base.0) {
                if !rip_rel_blocked(cg, sym)
                    && cg.emit_load_indexed_sym(dest, sym, &info.index, info.shift, info.disp, ty)
                {
                    return;
                }
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
    const_addr_vals: &FxHashMap<u32, i64>,
) {
    if seg_override != AddressSpace::Default {
        if let Some(sym) = global_addr_map.get(&ptr.0) {
            if !rip_rel_blocked(cg, sym) {
                cg.emit_seg_store_symbol(val, sym, ty, seg_override);
                return;
            }
        }
        // Constant offset (glibc THREAD_SETMEM): direct one-instruction form.
        if let Some(&addr) = const_addr_vals.get(&ptr.0) {
            if cg.emit_seg_store_const_addr(val, addr, ty, seg_override) {
                return;
            }
        }
        if let Some(info) = gep_fold_map.get(&ptr.0) {
            remat_indexed_acc_safe(cg, val, |cg| rematerialize_const_addr(cg, ptr, info));
        } else if let Some(info) = indexed_gep_map.get(&ptr.0) {
            remat_indexed_acc_safe(cg, val, |cg| rematerialize_skipped_indexed(cg, ptr, info));
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
        remat_indexed_acc_safe(cg, val, |cg| rematerialize_const_addr(cg, ptr, gep_info));
    }
    if let Some(info) = indexed_gep_map.get(&ptr.0) {
        if !is_wide_int_type(ty) && can_indexed_addr_fold(cg, info, global_addr_map) {
            if cg.emit_store_indexed(val, &info.base, &info.index, info.shift, info.disp, ty) {
                return;
            }
            // Symbol-base form: the GlobalAddr base has no register home.
            if let Some(sym) = global_addr_map.get(&info.base.0) {
                if !rip_rel_blocked(cg, sym)
                    && cg.emit_store_indexed_sym(val, sym, &info.index, info.shift, info.disp, ty)
                {
                    return;
                }
            }
        }
        remat_indexed_acc_safe(cg, val, |cg| rematerialize_skipped_indexed(cg, ptr, info));
    }
    cg.emit_store(val, ptr, ty);
}

fn generate_terminator(
    cg: &mut dyn ArchCodegen,
    term: &Terminator,
    frame_size: i64,
    block_label: u32,
) {
    // PF-15: a terminator ends the compare's adjacency window — flush any
    // deferred widening moves before control flow leaves the block.
    cg.flush_pending_widen();
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

#[cfg(test)]
mod conditional_increment_tests {
    use super::*;

    fn function_with(instructions: Vec<Instruction>) -> IrFunction {
        let mut function = IrFunction::new("csinc_test".to_string(), IrType::U32, vec![], false);
        function.blocks.push(BasicBlock {
            label: crate::ir::reexports::BlockId(0),
            instructions,
            terminator: Terminator::Unreachable,
            source_spans: Vec::new(),
        });
        function.next_value_id = 8;
        function
    }

    fn prefix() -> Vec<Instruction> {
        vec![
            Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::U32,
            },
            Instruction::ParamRef {
                dest: Value(1),
                param_idx: 1,
                ty: IrType::U32,
            },
            Instruction::BinOp {
                dest: Value(2),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::U32,
            },
        ]
    }

    fn select() -> Instruction {
        Instruction::Select {
            dest: Value(3),
            cond: Operand::Value(Value(1)),
            true_val: Operand::Value(Value(2)),
            false_val: Operand::Value(Value(0)),
            ty: IrType::I64,
        }
    }

    fn detect(function: &IrFunction) -> Vec<ConditionalIncrementFusion> {
        let uses = count_value_uses(function);
        let stability = analyze_base_stability(function);
        detect_conditional_increment_selects(&function.blocks[0], &uses, &stability)
    }

    #[test]
    fn detects_u32_wrapping_increment_with_widened_select() {
        let mut instructions = prefix();
        instructions.push(select());
        let fusions = detect(&function_with(instructions));
        assert_eq!(fusions.len(), 1);
        assert_eq!(fusions[0].add_idx, 2);
        assert_eq!(fusions[0].increment_ty, IrType::U32);
        assert!(fusions[0].increment_on_true);
    }

    #[test]
    fn rejects_increment_with_another_use() {
        let mut instructions = prefix();
        instructions.push(Instruction::Copy {
            dest: Value(4),
            src: Operand::Value(Value(2)),
        });
        instructions.push(select());
        assert!(detect(&function_with(instructions)).is_empty());
    }

    #[test]
    fn rejects_base_redefinition_between_add_and_select() {
        let mut instructions = prefix();
        instructions.push(Instruction::Copy {
            dest: Value(0),
            src: Operand::Const(IrConst::I32(9)),
        });
        instructions.push(select());
        assert!(detect(&function_with(instructions)).is_empty());
    }

    #[test]
    fn rejects_non_unit_delta() {
        let mut instructions = prefix();
        if let Instruction::BinOp { rhs, .. } = &mut instructions[2] {
            *rhs = Operand::Const(IrConst::I32(2));
        }
        instructions.push(select());
        assert!(detect(&function_with(instructions)).is_empty());
    }
}
