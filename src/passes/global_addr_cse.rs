//! Substitution-based GlobalAddr CSE.
//!
//! The frontend emits one `GlobalAddr` per source-level access, so a loop
//! that touches several file-scope arrays keeps a distinct SSA address value
//! live for every site. Those values are linker constants of the same
//! symbol and are interchangeable. Merging them cuts register pressure
//! (fannkuch's perm/perm1/count each materialized 4–6× on levkropp/lccc).
//!
//! This is **not** GVN Copy-insertion. Pointer-valued Copy chains are the
//! stale-base landmine that keeps GEP CSE disabled; we rewrite uses onto a
//! dominance-safe canonical value and delete the duplicate instruction.
//!
//! Canonical placement is dominance-safe and lifetime-minimal:
//! - a cold non-loop singleton stays at its original site (moving it performs
//!   no CSE and eagerly lengthens execution/liveness);
//! - a loop-executed address moves to the innermost containing loop's immediate
//!   preheader, after any cold guard but before repeated iterations;
//! - duplicates outside loops merge only when an existing occurrence already
//!   dominates the others; mutually-exclusive branches remain branch-local,
//!   matching GCC/Clang/ICC/ICX and avoiding eager materialization.
//! This retains the register-pressure win without unconditional entry hoisting.
//!
//! Class split (the levkropp original mixed these and fought RIP remat):
//! - **Foldable**: every use is a Load/Store pointer, an absorbed GEP/Add/Sub
//!   address producer, or a Copy/same-size Cast of the address. These stay
//!   in `never_materialized` after CSE and become `sym(%rip)` / SIB folds.
//! - **Must-materialize**: any other use (call arg, stored value, inline-asm
//!   operand, intrinsic dest_ptr, return, …). CSE within this class is the
//!   register-pressure win.
//! Mixing the two classes is forbidden: folding a RIP-only address into a
//! value that must occupy a register would pin `window`/`crc_table` in a
//! GPR and undo RA-01.
//!
//! Substitution walks `Instruction::for_each_operand_mut` **and**
//! `for_each_value_use_mut` so Intrinsic `dest_ptr` and InlineAsm outputs
//! cannot be left dangling (the levkropp TCE helper missed both).
//!
//! Kill switches: `CCC_NO_GADDR_CSE`, `CCC_DISABLE_PASSES=gaddrcse`.
//!
//! Scheduling: the pass **must** run before the first GVN. GVN already CSEs
//! `GlobalAddr` (same-block, class-blind Copies). A late-only wiring after
//! post-structural-inline therefore observes `merged=0` on every fannkuch-like
//! shape — the duplicates are already Copies. GVN's GlobalAddr key is also
//! class-split so a later GVN cannot pin a RIP-foldable address into a GPR.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrFunction, IrModule, Operand, Value};

fn debug_merged(name: &str, n: usize) {
    if std::env::var_os("CCC_DEBUG_GADDR_CSE").is_some() {
        eprintln!("[GADDR_CSE] fn={} merged={}", name, n);
    }
}

/// Resolve GNU `__attribute__((alias))` chains the same way GVN does, so
/// `GlobalAddr "foo"` and `GlobalAddr "bar"` of an alias pair share a web.
fn alias_canon_map(module: &IrModule) -> FxHashMap<String, String> {
    let direct: FxHashMap<String, String> = module
        .aliases
        .iter()
        .map(|(alias, target, _)| (alias.clone(), target.clone()))
        .collect();
    let mut out = FxHashMap::default();
    for alias in direct.keys() {
        let mut current = alias.as_str();
        let mut seen = FxHashSet::default();
        while let Some(next) = direct.get(current) {
            if !seen.insert(current.to_string()) {
                break;
            }
            current = next;
        }
        out.insert(alias.clone(), current.to_string());
    }
    out
}

fn canon_name<'a>(name: &'a str, aliases: &'a FxHashMap<String, String>) -> &'a str {
    aliases.get(name).map(String::as_str).unwrap_or(name)
}

pub(crate) fn run_module(module: &mut IrModule) -> usize {
    let aliases = alias_canon_map(module);
    // TLS symbols: their GlobalAddr costs TWO instructions on x86-64
    // (movq %fs:0,%r + leaq sym@TPOFF(%r)), and unlike RIP-addressable
    // globals there is no `sym(,%idx,scale)` absolute form to protect —
    // the indexed form works from the materialized base register either
    // way. So TLS addresses are ALWAYS CSE/hoist candidates, even when
    // they feed variable-index GEPs (glibc __thread arrays re-derived the
    // base per element access: 2 extra instructions per loop iteration).
    let tls: FxHashSet<String> = module
        .globals
        .iter()
        .filter(|g| g.is_thread_local)
        .map(|g| g.name.clone())
        .collect();
    module.for_each_function(|f| run_with_aliases_tls(f, &aliases, &tls))
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    run_with_aliases_tls(func, &FxHashMap::default(), &FxHashSet::default())
}

/// GlobalAddr values feeding variable-index GEPs stay at their original sites.
/// Hoisting/CSE lengthens the base live range and can evict the natural index,
/// destroying x86 `sym(,%idx,scale)` selection (gzip CRC regression).
///
/// OP-34 stride gate: site-locality exists solely to protect that SIB form.
/// When the variable offset carries a constant stride SIB cannot encode
/// (struct strides like 24/32/56, negative or oversized scales), the indexed
/// form is unreachable no matter where the base lives — site-locality then
/// only duplicates the address chain. Every `s.field` access of a stride-56
/// struct loop re-materialised `GA + j*56` (nbody: seven marching pointers,
/// GPR flood, stack-slot relay). Such bases are ordinary CSE candidates: GVN
/// unifies the `GA + idx*stride` GEPs into one, IVSR produces a single
/// marching pointer, and field offsets fold into displacements at the uses.
pub(crate) fn classify_site_local_indexed(func: &IrFunction) -> FxHashSet<u32> {
    let mut globals = FxHashSet::default();
    let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
    let mut indexed_bases = Vec::new();
    // Constant multiplicands of index expressions: dest value id -> constant.
    // `Mul(idx, C)` / `C * idx` and `Shl(idx, K)` (the frontend's two forms of
    // byte-stride scaling). Used to decide whether the indexed GEP could ever
    // select `sym(,%idx,scale)` addressing.
    let mut mul_const: FxHashMap<u32, i64> = FxHashMap::default();
    let mut shl_const: FxHashMap<u32, i64> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp {
                    dest,
                    op: crate::ir::reexports::IrBinOp::Mul,
                    rhs: Operand::Const(c),
                    ..
                }
                | Instruction::BinOp {
                    dest,
                    op: crate::ir::reexports::IrBinOp::Mul,
                    lhs: Operand::Const(c),
                    ..
                } => {
                    if let Some(c) = c.to_i64() {
                        mul_const.insert(dest.0, c);
                    }
                }
                Instruction::BinOp {
                    dest,
                    op: crate::ir::reexports::IrBinOp::Shl,
                    rhs: Operand::Const(c),
                    ..
                } => {
                    // Guard the `1 << k` below: shifts outside 0..63 are
                    // either nonsense or would overflow i64.
                    if let Some(k) = c.to_i64().filter(|&k| (0..63).contains(&k)) {
                        shl_const.insert(dest.0, k);
                    }
                }
                _ => {}
            }
        }
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                *def_count.entry(dest.0).or_insert(0) += 1;
            }
            match inst {
                Instruction::GlobalAddr { dest, .. } => {
                    globals.insert(dest.0);
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } => {
                    parent.insert(dest.0, src.0);
                }
                Instruction::Cast {
                    dest,
                    src: Operand::Value(src),
                    from_ty,
                    to_ty,
                    ..
                } if from_ty.size() == to_ty.size() && !from_ty.is_float() && !to_ty.is_float() => {
                    parent.insert(dest.0, src.0);
                }
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(_),
                    ..
                } => {
                    parent.insert(dest.0, base.0);
                }
                Instruction::GetElementPtr {
                    base,
                    offset: Operand::Value(idx),
                    ..
                } => {
                    // Stride of this index: a Mul/Shl constant when the offset
                    // is a scaled index, 1 when it is a bare (opaque) index.
                    let stride = mul_const
                        .get(&idx.0)
                        .copied()
                        .or_else(|| shl_const.get(&idx.0).map(|&k| 1i64 << k))
                        .unwrap_or(1);
                    if matches!(stride, 1 | 2 | 4 | 8) {
                        indexed_bases.push(base.0);
                    }
                }
                Instruction::BinOp {
                    dest,
                    op: crate::ir::reexports::IrBinOp::Add | crate::ir::reexports::IrBinOp::Sub,
                    lhs: Operand::Value(base),
                    rhs: Operand::Const(_),
                    ..
                } => {
                    parent.insert(dest.0, base.0);
                }
                Instruction::BinOp {
                    dest,
                    op: crate::ir::reexports::IrBinOp::Add,
                    lhs: Operand::Const(_),
                    rhs: Operand::Value(base),
                    ..
                } => {
                    parent.insert(dest.0, base.0);
                }
                _ => {}
            }
        }
    }
    parent.retain(|dest, _| def_count.get(dest).copied() == Some(1));

    let mut out = FxHashSet::default();
    for base in indexed_bases {
        let mut current = base;
        let mut seen = FxHashSet::default();
        while seen.insert(current) {
            if globals.contains(&current) {
                out.insert(current);
                break;
            }
            let Some(&next) = parent.get(&current) else {
                break;
            };
            current = next;
        }
    }
    out
}

fn dominates(a: usize, mut b: usize, idom: &[usize]) -> bool {
    loop {
        if a == b {
            return true;
        }
        let next = idom.get(b).copied().unwrap_or(usize::MAX);
        if next == usize::MAX || next == b {
            return false;
        }
        b = next;
    }
}

/// Choose the narrowest dominance-safe placement. A non-loop singleton stays
/// where it is (moving it is pure lifetime growth); values executed in a loop
/// move to that loop's immediate preheader. Outside loops, only an existing
/// occurrence that already dominates all others may become canonical.
fn choose_placement(
    blocks: &[usize],
    cfg: &crate::ir::analysis::CfgAnalysis,
    loops: &[super::loop_analysis::NaturalLoop],
    intrinsic_blocks: &FxHashSet<usize>,
) -> Option<usize> {
    let containing = loops
        .iter()
        .filter(|lp| blocks.iter().all(|b| lp.body.contains(b)))
        .min_by_key(|lp| lp.body.len());
    if let Some(lp) = containing {
        // Intrinsic loops still carry hidden accumulator/XMM locations. CSE at
        // existing sites is safe, but extending a new home across the loop is
        // blocked until RA-23 makes those locations explicit.
        if !lp.body.iter().any(|b| intrinsic_blocks.contains(b)) {
            let preheader = cfg.idom.get(lp.header).copied()?;
            if preheader != usize::MAX && preheader != lp.header {
                return Some(preheader);
            }
        }
    }
    if blocks.len() < 2 {
        return None;
    }

    // Outside loops, never synthesize an eager common-dominator definition for
    // mutually-exclusive branches. GCC/Clang/ICC/ICX all keep those branch-local.
    // Reuse an existing occurrence only when it already dominates every other
    // occurrence, selecting the deepest such definition to minimize lifetime.
    let depth = |mut b: usize| {
        let mut n = 0usize;
        loop {
            let next = cfg.idom.get(b).copied().unwrap_or(usize::MAX);
            if next == usize::MAX || next == b {
                break;
            }
            n += 1;
            b = next;
        }
        n
    };
    blocks
        .iter()
        .copied()
        .filter(|&candidate| blocks.iter().all(|&b| dominates(candidate, b, &cfg.idom)))
        .max_by_key(|&candidate| depth(candidate))
}

pub(crate) fn run_with_aliases(
    func: &mut IrFunction,
    aliases: &FxHashMap<String, String>,
) -> usize {
    run_with_aliases_tls(func, aliases, &FxHashSet::default())
}

pub(crate) fn run_with_aliases_tls(
    func: &mut IrFunction,
    aliases: &FxHashMap<String, String>,
    tls_symbols: &FxHashSet<String>,
) -> usize {
    if func.blocks.is_empty() {
        debug_merged(&func.name, 0);
        return 0;
    }
    if std::env::var_os("CCC_NO_GADDR_CSE").is_some() {
        debug_merged(&func.name, 0);
        return 0;
    }
    let must_mat = classify_must_materialize(func);
    let site_local = classify_site_local_indexed(func);

    // Group every movable materialization by canonical symbol and use class.
    // Site-local indexed bases deliberately keep distinct webs. TLS is the
    // sole safe exception to the class split: unlike a normal GlobalAddr it
    // cannot become a RIP/SIB memory operand, so both classes must materialize
    // the same thread-relative base. Sharing it avoids repeated `%fs:0` reads.
    let mut groups: FxHashMap<(String, bool), Vec<(usize, usize, Value)>> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Instruction::GlobalAddr { dest, name } = inst {
                let is_tls = tls_symbols.contains(name);
                if site_local.contains(&dest.0) && !is_tls {
                    continue;
                }
                groups
                    .entry((
                        canon_name(name, aliases).to_string(),
                        !is_tls && must_mat.contains(&dest.0),
                    ))
                    .or_default()
                    .push((bi, ii, *dest));
            }
        }
    }

    if groups.is_empty()
        || groups
            .values()
            .all(|occurs| occurs.len() == 1 && occurs[0].0 == 0)
    {
        debug_merged(&func.name, 0);
        return 0;
    }
    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    let loops = super::loop_analysis::find_natural_loops(
        func.blocks.len(),
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    );
    let intrinsic_blocks: FxHashSet<usize> = func
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(bi, block)| {
            block
                .instructions
                .iter()
                .any(|inst| matches!(inst, Instruction::Intrinsic { .. }))
                .then_some(bi)
        })
        .collect();

    let mut next_id = func
        .next_value_id
        .max(func.max_value_id().saturating_add(1));
    let mut subst: FxHashMap<u32, u32> = FxHashMap::default();
    let mut delete_ids: FxHashSet<u32> = FxHashSet::default();
    let mut inserts: FxHashMap<usize, Vec<(String, bool, Value)>> = FxHashMap::default();

    for ((name, class), occurrences) in groups {
        let blocks: Vec<usize> = occurrences.iter().map(|x| x.0).collect();
        let Some(place) = choose_placement(&blocks, &cfg, &loops, &intrinsic_blocks) else {
            continue;
        };

        // Reuse the earliest existing definition in the placement block when
        // possible. Otherwise create one at the block's Phi-safe prefix.
        let existing = occurrences
            .iter()
            .filter(|x| x.0 == place)
            .min_by_key(|x| x.1)
            .copied();
        let canonical = if let Some((_, _, value)) = existing {
            value
        } else {
            let value = Value(next_id);
            next_id += 1;
            inserts
                .entry(place)
                .or_default()
                .push((name.clone(), class, value));
            value
        };

        for &(_, _, value) in &occurrences {
            if value != canonical {
                subst.insert(value.0, canonical.0);
                delete_ids.insert(value.0);
            }
        }
    }

    if delete_ids.is_empty() {
        debug_merged(&func.name, 0);
        return 0;
    }
    rewrite_uses(func, &subst);

    // Rebuild each affected block once. This keeps instruction/source-span
    // indices synchronized and avoids insertion indices shifting each other.
    let dummy = crate::common::source::Span::dummy();
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        let mut add = inserts.remove(&bi).unwrap_or_default();
        let has_delete = block.instructions.iter().any(|inst| {
            matches!(
                inst, Instruction::GlobalAddr { dest, .. } if delete_ids.contains(&dest.0)
            )
        });
        if add.is_empty() && !has_delete {
            continue;
        }
        add.sort_by(|a, b| (a.0.as_str(), a.1, a.2 .0).cmp(&(b.0.as_str(), b.1, b.2 .0)));
        let insert_at = block
            .instructions
            .iter()
            .take_while(|inst| {
                matches!(
                    inst,
                    Instruction::Phi { .. }
                        | Instruction::Alloca { .. }
                        | Instruction::ParamRef { .. }
                )
            })
            .count();
        let old_insts = std::mem::take(&mut block.instructions);
        let old_len = old_insts.len();
        let lockstep = block.source_spans.len() == old_len;
        let old_spans = if lockstep {
            std::mem::take(&mut block.source_spans)
        } else {
            block.source_spans.clear();
            Vec::new()
        };
        let mut new_insts = Vec::with_capacity(old_insts.len() + add.len());
        let mut new_spans = if lockstep {
            Vec::with_capacity(old_spans.len() + add.len())
        } else {
            Vec::new()
        };
        for (ii, inst) in old_insts.into_iter().enumerate() {
            if ii == insert_at {
                for (name, _, dest) in &add {
                    new_insts.push(Instruction::GlobalAddr {
                        dest: *dest,
                        name: name.clone(),
                    });
                    if lockstep {
                        new_spans.push(dummy);
                    }
                }
            }
            let remove = matches!(&inst, Instruction::GlobalAddr { dest, .. } if delete_ids.contains(&dest.0));
            if !remove {
                new_insts.push(inst);
                if lockstep {
                    new_spans.push(old_spans[ii]);
                }
            }
        }
        if insert_at == old_len && !add.is_empty() {
            // Empty/Phi-only block: insertion point is at the old end.
            for (name, _, dest) in &add {
                new_insts.push(Instruction::GlobalAddr {
                    dest: *dest,
                    name: name.clone(),
                });
                if lockstep {
                    new_spans.push(dummy);
                }
            }
        }
        block.instructions = new_insts;
        if lockstep {
            block.source_spans = new_spans;
        }
    }
    func.next_value_id = next_id;
    let n = delete_ids.len();
    debug_merged(&func.name, n);
    n
}

/// A GlobalAddr dest *must materialize* if any use is not a foldable memory
/// pointer or an absorbed address producer. Conservative: unknown shapes
/// (calls, asm, intrinsics, stored *values*) force materialization.
pub(crate) fn classify_must_materialize(func: &IrFunction) -> FxHashSet<u32> {
    let mut gaddrs: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GlobalAddr { dest, .. } = inst {
                gaddrs.insert(dest.0);
            }
        }
    }
    if gaddrs.is_empty() {
        return FxHashSet::default();
    }

    // Address producers derived from a GlobalAddr (GEP/Add/Sub/Copy/Cast)
    // that are themselves only used as memory pointers do not force the
    // GlobalAddr to materialize — that is the RIP/SIB fold contract.
    // `parent[dest] = sources` lets a must-materialize use of a derived
    // address (Intrinsic dest_ptr, call arg, …) poison the originating
    // GlobalAddr, not just the derived id.
    let mut derived: FxHashSet<u32> = gaddrs.clone();
    let mut parent: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut parent_edges: FxHashSet<(u32, u32)> = FxHashSet::default();
    let mut link = |dest: u32,
                    src: u32,
                    derived: &mut FxHashSet<u32>,
                    parent: &mut FxHashMap<u32, Vec<u32>>| {
        if parent_edges.insert((dest, src)) {
            parent.entry(dest).or_default().push(src);
        }
        derived.insert(dest)
    };
    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::GetElementPtr { dest, base, .. } if derived.contains(&base.0) => {
                        if link(dest.0, base.0, &mut derived, &mut parent) {
                            changed = true;
                        }
                    }
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    } if derived.contains(&src.0) => {
                        if link(dest.0, src.0, &mut derived, &mut parent) {
                            changed = true;
                        }
                    }
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(src),
                        from_ty,
                        to_ty,
                        ..
                    } if derived.contains(&src.0)
                        && from_ty.size() == to_ty.size()
                        && !from_ty.is_float()
                        && !to_ty.is_float() =>
                    {
                        if link(dest.0, src.0, &mut derived, &mut parent) {
                            changed = true;
                        }
                    }
                    Instruction::BinOp {
                        dest,
                        op: crate::ir::reexports::IrBinOp::Add | crate::ir::reexports::IrBinOp::Sub,
                        lhs,
                        rhs,
                        ..
                    } => {
                        if let Operand::Value(v) = lhs {
                            if derived.contains(&v.0)
                                && link(dest.0, v.0, &mut derived, &mut parent)
                            {
                                changed = true;
                            }
                        }
                        if let Operand::Value(v) = rhs {
                            if derived.contains(&v.0)
                                && link(dest.0, v.0, &mut derived, &mut parent)
                            {
                                changed = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut must = FxHashSet::default();
    let mut mark = |id: u32| {
        let mut stack = vec![id];
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            if gaddrs.contains(&cur) {
                must.insert(cur);
            }
            if let Some(ps) = parent.get(&cur) {
                stack.extend(ps.iter().copied());
            }
        }
    };

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load { ptr, .. } => {
                    // Pointer use of a derived address is foldable.
                    let _ = ptr;
                }
                Instruction::Store { val, ptr, .. } => {
                    if let Operand::Value(v) = val {
                        mark(v.0);
                    }
                    let _ = ptr;
                }
                Instruction::GetElementPtr { .. }
                | Instruction::Copy { .. }
                | Instruction::Cast { .. }
                | Instruction::BinOp {
                    op: crate::ir::reexports::IrBinOp::Add | crate::ir::reexports::IrBinOp::Sub,
                    ..
                } => {}
                _ => {
                    inst.for_each_used_value(|id| mark(id));
                }
            }
        }
        block.terminator.for_each_used_value(|id| mark(id));
    }
    must
}

fn rewrite_uses(func: &mut IrFunction, subst: &FxHashMap<u32, u32>) {
    let rewrite_val = |v: &mut Value| {
        if let Some(&to) = subst.get(&v.0) {
            *v = Value(to);
        }
    };
    let rewrite_op = |op: &mut Operand| {
        if let Operand::Value(v) = op {
            rewrite_val(v);
        }
    };
    for block in func.blocks.iter_mut() {
        for inst in &mut block.instructions {
            inst.for_each_operand_mut(rewrite_op);
            inst.for_each_value_use_mut(rewrite_val);
        }
        block.terminator.for_each_operand_mut(rewrite_op);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{BasicBlock, BlockId, IrFunction, Operand, Terminator};

    fn empty_func() -> IrFunction {
        let mut f = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        f.next_value_id = 10;
        f
    }

    fn store(ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(0)),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    #[test]
    fn merges_same_symbol_in_block_foldable() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "g".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "g".to_string(),
                },
                store(2),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 1);
        assert_eq!(func.blocks[0].instructions.len(), 2);
        match &func.blocks[0].instructions[1] {
            Instruction::Store { ptr, .. } => assert_eq!(ptr.0, 1),
            _ => panic!("expected store"),
        }
    }

    #[test]
    fn entry_block_canonical_wins_cross_block() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::GlobalAddr {
                dest: Value(1),
                name: "g".to_string(),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                load(6, 5),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 1);
        assert_eq!(func.blocks[1].instructions.len(), 1);
        match &func.blocks[1].instructions[0] {
            Instruction::Load { ptr, .. } => assert_eq!(ptr.0, 1),
            _ => panic!("expected load"),
        }
    }

    #[test]
    fn distinct_symbols_untouched() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "a".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "b".to_string(),
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(run(&mut func), 0);
        assert_eq!(func.blocks[0].instructions.len(), 2);
    }

    #[test]
    fn does_not_mix_foldable_with_must_materialize() {
        // v1 is only a load pointer (foldable). v2 is passed to a call
        // (must materialize). CSE must not merge them: that would pin the
        // RIP-foldable address in a register.
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "g".to_string(),
                },
                load(3, 1),
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "g".to_string(),
                },
                Instruction::Call {
                    func: "use_ptr".to_string(),
                    info: crate::ir::reexports::CallInfo {
                        dest: None,
                        args: vec![Operand::Value(Value(2))],
                        arg_types: vec![IrType::Ptr],
                        return_type: IrType::Void,
                        is_variadic: false,
                        num_fixed_args: 1,
                        struct_arg_sizes: vec![],
                        struct_arg_aligns: vec![],
                        struct_arg_classes: vec![],
                        struct_arg_riscv_float_classes: vec![],
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false,
                        is_fastcall: false,
                        is_pure: false,
                        is_const: false,
                        ret_eightbyte_classes: vec![],
                        ret_is_f128_sse: false,
                    },
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(run(&mut func), 0);
        assert_eq!(func.blocks[0].instructions.len(), 4);
    }

    #[test]
    fn tls_can_merge_foldable_and_materialized_uses() {
        // Normal globals must retain the class split above because a Load can
        // become RIP-relative. TLS always needs a `%fs`-relative base in a
        // register, so duplicate materializations only add thread-pointer
        // reads and live ranges.
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "tls_g".to_string(),
                },
                load(3, 1),
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "tls_g".to_string(),
                },
                Instruction::Call {
                    func: "use_ptr".to_string(),
                    info: crate::ir::reexports::CallInfo {
                        dest: None,
                        args: vec![Operand::Value(Value(2))],
                        arg_types: vec![IrType::Ptr],
                        return_type: IrType::Void,
                        is_variadic: false,
                        num_fixed_args: 1,
                        struct_arg_sizes: vec![],
                        struct_arg_aligns: vec![],
                        struct_arg_classes: vec![],
                        struct_arg_riscv_float_classes: vec![],
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false,
                        is_fastcall: false,
                        is_pure: false,
                        is_const: false,
                        ret_eightbyte_classes: vec![],
                        ret_is_f128_sse: false,
                    },
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let tls = FxHashSet::from_iter(["tls_g".to_string()]);
        assert_eq!(
            run_with_aliases_tls(&mut func, &FxHashMap::default(), &tls),
            1
        );
        assert_eq!(func.blocks[0].instructions.len(), 3);
        match &func.blocks[0].instructions[2] {
            Instruction::Call { info, .. } => assert_eq!(info.args, vec![Operand::Value(Value(1))]),
            other => panic!("expected rewritten call, got {other:?}"),
        }
    }

    #[test]
    fn rewrites_intrinsic_dest_ptr() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "buf".to_string(),
                },
                Instruction::Intrinsic {
                    dest: None,
                    op: crate::ir::intrinsics::IntrinsicOp::Storedqu,
                    dest_ptr: Some(Value(1)),
                    args: vec![Operand::Value(Value(8))],
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "buf".to_string(),
                },
                Instruction::Intrinsic {
                    dest: None,
                    op: crate::ir::intrinsics::IntrinsicOp::Storedqu,
                    dest_ptr: Some(Value(2)),
                    args: vec![Operand::Value(Value(9))],
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 1);
        assert_eq!(func.blocks[0].instructions.len(), 3);
        match &func.blocks[0].instructions[2] {
            Instruction::Intrinsic {
                dest_ptr: Some(p), ..
            } => assert_eq!(p.0, 1),
            other => panic!("expected intrinsic, got {other:?}"),
        }
    }

    #[test]
    fn keeps_source_spans_lockstep() {
        let mut func = empty_func();
        let dummy = crate::common::source::Span::dummy();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "g".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "g".to_string(),
                },
                store(2),
            ],
            terminator: Terminator::Return(None),
            source_spans: vec![dummy, dummy, dummy],
        });
        run(&mut func);
        assert_eq!(
            func.blocks[0].source_spans.len(),
            func.blocks[0].instructions.len()
        );
    }

    #[test]
    fn single_cold_non_entry_stays_local() {
        // Moving a singleton out of a non-loop block performs no CSE and only
        // lengthens its lifetime/eager execution.
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                load(6, 5),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(run(&mut func), 0);
        assert!(func.blocks[0].instructions.is_empty());
        assert!(matches!(
            func.blocks[1].instructions[0],
            Instruction::GlobalAddr { dest: Value(5), .. }
        ));
    }

    #[test]
    fn singleton_in_loop_hoists_to_preheader() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: vec![],
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                load(6, 5),
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        assert_eq!(run(&mut func), 1);
        let Instruction::GlobalAddr { dest, .. } = &func.blocks[0].instructions[0] else {
            panic!("expected preheader GlobalAddr");
        };
        assert!(
            matches!(func.blocks[2].instructions[0], Instruction::Load { ptr, .. } if ptr.0 == dest.0)
        );
    }

    #[test]
    fn intrinsic_loop_singleton_is_not_lifetime_extended() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: vec![],
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                Instruction::Intrinsic {
                    dest: Some(Value(7)),
                    op: crate::ir::intrinsics::IntrinsicOp::Rdtsc,
                    dest_ptr: None,
                    args: vec![],
                },
                load(6, 5),
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        assert_eq!(run(&mut func), 0);
        assert!(func.blocks[0].instructions.is_empty());
        assert!(matches!(
            func.blocks[2].instructions[0],
            Instruction::GlobalAddr { .. }
        ));
    }

    #[test]
    fn mutually_exclusive_blocks_stay_branch_local() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                load(6, 5),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(7),
                    name: "g".to_string(),
                },
                load(8, 7),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 0);
        assert!(func.blocks[0].instructions.is_empty());
        assert!(matches!(
            func.blocks[1].instructions[0],
            Instruction::GlobalAddr { dest: Value(5), .. }
        ));
        assert!(matches!(
            func.blocks[2].instructions[0],
            Instruction::GlobalAddr { dest: Value(7), .. }
        ));
    }

    #[test]
    fn dominating_cross_block_occurrence_is_reused() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                load(6, 5),
            ],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: vec![],
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(7),
                    name: "g".to_string(),
                },
                load(8, 7),
            ],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        assert_eq!(run(&mut func), 1);
        assert!(
            matches!(func.blocks[2].instructions[0], Instruction::Load { ptr, .. } if ptr.0 == 5)
        );
    }

    #[test]
    fn aliases_share_a_web() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "foo".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "bar".to_string(),
                },
                store(2),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let mut aliases = FxHashMap::default();
        aliases.insert("bar".to_string(), "foo".to_string());
        let n = run_with_aliases(&mut func, &aliases);
        assert_eq!(n, 1);
        match &func.blocks[0].instructions[1] {
            Instruction::Store { ptr, .. } => assert_eq!(ptr.0, 1),
            other => panic!("expected store, got {other:?}"),
        }
    }

    #[test]
    fn variable_index_global_addrs_remain_site_local() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "table".to_string(),
                },
                Instruction::GetElementPtr {
                    dest: Value(2),
                    base: Value(1),
                    offset: Operand::Value(Value(9)),
                    ty: IrType::Ptr,
                },
                load(3, 2),
                Instruction::GlobalAddr {
                    dest: Value(4),
                    name: "table".to_string(),
                },
                Instruction::GetElementPtr {
                    dest: Value(5),
                    base: Value(4),
                    offset: Operand::Value(Value(8)),
                    ty: IrType::Ptr,
                },
                load(6, 5),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(run(&mut func), 0);
        assert_eq!(
            func.blocks[0]
                .instructions
                .iter()
                .filter(|i| matches!(i, Instruction::GlobalAddr { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn derived_variable_index_bases_remain_site_local() {
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "table".to_string(),
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                },
                Instruction::GetElementPtr {
                    dest: Value(3),
                    base: Value(2),
                    offset: Operand::Value(Value(9)),
                    ty: IrType::Ptr,
                },
                load(4, 3),
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "table".to_string(),
                },
                Instruction::Cast {
                    dest: Value(6),
                    src: Operand::Value(Value(5)),
                    from_ty: IrType::Ptr,
                    to_ty: IrType::U64,
                },
                Instruction::GetElementPtr {
                    dest: Value(7),
                    base: Value(6),
                    offset: Operand::Value(Value(8)),
                    ty: IrType::Ptr,
                },
                load(10, 7),
            ],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        assert_eq!(run(&mut func), 0);
        assert_eq!(classify_site_local_indexed(&func).len(), 2);
    }

    #[test]
    fn cold_singletons_preserve_class_split() {
        // Foldable load and call-arg classes of the same symbol must remain
        // distinct. As cold singletons they also stay in their original block.
        let mut func = empty_func();
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(5),
                    name: "g".to_string(),
                },
                load(6, 5),
                Instruction::GlobalAddr {
                    dest: Value(7),
                    name: "g".to_string(),
                },
                Instruction::Call {
                    func: "use_ptr".to_string(),
                    info: crate::ir::reexports::CallInfo {
                        dest: None,
                        args: vec![Operand::Value(Value(7))],
                        arg_types: vec![IrType::Ptr],
                        return_type: IrType::Void,
                        is_variadic: false,
                        num_fixed_args: 1,
                        struct_arg_sizes: vec![],
                        struct_arg_aligns: vec![],
                        struct_arg_classes: vec![],
                        struct_arg_riscv_float_classes: vec![],
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false,
                        is_fastcall: false,
                        is_pure: false,
                        is_const: false,
                        ret_eightbyte_classes: vec![],
                        ret_is_f128_sse: false,
                    },
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = run(&mut func);
        assert_eq!(n, 0);
        assert!(func.blocks[0].instructions.is_empty());
        let local_gaddrs = func.blocks[1]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::GlobalAddr { .. }))
            .count();
        assert_eq!(local_gaddrs, 2);
    }
}
