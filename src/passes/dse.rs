//! Dead Store Elimination (DSE) — OP-13.
//!
//! Removes stores whose value is provably overwritten later in the same
//! basic block before any possible read:
//!
//! ```c
//! int x; x = 1; x = 2; x = 3; use(&x);   // only `x = 3` survives
//! s->a = 1; s->a = 2; s->b = 3;          // first store dies
//! *p = 1; *p = 2;                         // first store dies (any pointer SSA value)
//! ```
//!
//! Algorithm (same-block backward scan, fail-closed):
//!
//! * A **cell** is `(root, byte offset, size)`. Roots are allocas, globals
//!   (by symbol name — two `GlobalAddr` instructions for one symbol are the
//!   same root even as distinct SSA values), or an arbitrary pointer SSA
//!   value (params, phis, computed pointers).
//! * Scanning a block backwards, a "pending" set holds later stores that
//!   have not yet been proven observable. When the scan reaches a store
//!   whose cell is fully covered by a pending store (same root, same
//!   offset, pending size >= size) with **no intervening read of that
//!   cell**, the earlier store is dead: any reader between the two would
//!   have killed the pending entry first.
//! * Read events (loads, calls, atomics, memcpy sources, inline asm,
//!   varargs, ...) kill pending entries they may alias:
//!   - a load with a resolved `(alloca/global root, const offset)` kills
//!     only overlapping pending cells of the same root;
//!   - a load through any other pointer kills everything it may name;
//!   - calls and other opaque memory events kill everything open.
//! * Address-escape analysis: an alloca root is **closed** when no pointer
//!   derived from it ever flows anywhere except `Load.ptr`, `Store.ptr`,
//!   and constant-offset `GEP.base`. Closed allocas cannot be named by any
//!   other pointer (including callees), so their pending entries survive
//!   unresolvable loads and calls. Globals are always open (other
//!   translation units can hold their address).
//! * Volatile stores are never eliminated but may overwrite (kill) earlier
//!   stores; volatile loads read memory and kill like ordinary loads.
//!   Segment-overridden accesses target a different address space: they can
//!   neither alias default-space cells nor be eliminated here.
//!
//! Scope is deliberately same-block. Cross-block DSE needs post-dominator
//! plus read-between proofs — that is the `segments`-based OP-33 follow-up.
//!
//! Disable with `CCC_NO_DSE=1`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{Instruction, IrFunction, Operand, Value};

/// Byte size of an IR access type for cell overlap math. `-1` marks types
/// whose cell extent is not modeled (vectors and anything unexpected):
/// such stores are never elimination targets and conservatively kill
/// same-root pending entries.
fn access_size(ty: IrType) -> i64 {
    match ty {
        IrType::I8 | IrType::U8 => 1,
        IrType::I16 | IrType::U16 => 2,
        IrType::I32 | IrType::U32 | IrType::F32 => 4,
        IrType::I64 | IrType::U64 | IrType::F64 | IrType::Ptr => 8,
        IrType::I128 | IrType::U128 | IrType::F128 => 16,
        _ => -1,
    }
}

/// Identity of the object a cell lives in.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum CellRoot {
    /// A stack object (alloca value id). Distinct allocas never alias.
    Alloca(u32),
    /// A global symbol (by name). Distinct symbols never alias.
    Global(Box<str>),
    /// Any other pointer SSA value (param, phi, computed pointer). Equal ids
    /// are the same pointer; distinct ids may still denote one address.
    Other(u32),
}

/// Pointer-defining fact extracted once per function.
#[derive(Clone, Debug)]
enum PtrDef {
    Alloca,
    Global(Box<str>),
    /// Constant-offset GEP: address = base + delta bytes.
    GepConst {
        base: u32,
        delta: i64,
    },
    /// Variable-offset GEP: address = base + unknown.
    GepVar {
        base: u32,
    },
    CopyOf(u32),
    /// Param, phi, or anything computed: opaque root by value id.
    Opaque,
}

/// A resolved memory cell.
#[derive(Clone, Debug)]
struct CellAddr {
    root: CellRoot,
    /// `None` = variable offset (root known, position unknown).
    offset: Option<i64>,
}

/// Pending later store that may overwrite an earlier cell.
struct Pending {
    root: CellRoot,
    offset: i64,
    size: i64,
}

struct DseContext {
    defs: FxHashMap<u32, PtrDef>,
    /// Alloca value ids whose address provably never escapes.
    closed_allocas: FxHashSet<u32>,
    /// This function IS a non-local-goto target (it holds a
    /// NonlocalGotoSave area): any callee can transfer control to a label
    /// alias inside this function, creating a re-entry edge invisible to
    /// the per-block CFG.  Closed-alloca survival across opaque events is
    /// unsound in that case.
    has_nonlocal_targets: bool,
}

impl DseContext {
    fn build(func: &IrFunction) -> Self {
        let mut defs: FxHashMap<u32, PtrDef> = FxHashMap::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Alloca { dest, .. } => {
                        defs.insert(dest.0, PtrDef::Alloca);
                    }
                    Instruction::GlobalAddr { dest, name } => {
                        defs.insert(dest.0, PtrDef::Global(name.as_str().into()));
                    }
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => {
                        let d = match offset {
                            Operand::Const(c) => PtrDef::GepConst {
                                base: base.0,
                                delta: c.to_i64().unwrap_or(i64::MAX),
                            },
                            Operand::Value(_) => PtrDef::GepVar { base: base.0 },
                        };
                        defs.insert(dest.0, d);
                    }
                    Instruction::Copy { dest, src } => {
                        if let Operand::Value(s) = src {
                            defs.insert(dest.0, PtrDef::CopyOf(s.0));
                        }
                    }
                    _ => {}
                }
            }
        }
        let closed_allocas = compute_closed_allocas(func, &defs);
        let has_nonlocal_targets = func.blocks.iter().any(|block| {
            block
                .instructions
                .iter()
                .any(|inst| matches!(inst, Instruction::NonlocalGotoSave { .. }))
        });
        DseContext {
            defs,
            closed_allocas,
            has_nonlocal_targets,
        }
    }

    /// Resolve a pointer value to a cell address. Values with no defining
    /// instruction (params) are opaque roots by value id.
    fn resolve(&self, v: Value) -> Option<CellAddr> {
        let mut cur = v;
        let mut offset: i64 = 0;
        loop {
            let def = match self.defs.get(&cur.0) {
                Some(d) => d,
                None => {
                    return Some(CellAddr {
                        root: CellRoot::Other(cur.0),
                        offset: Some(offset),
                    })
                }
            };
            match def {
                PtrDef::CopyOf(src) => cur = Value(*src),
                PtrDef::GepConst { base, delta } => {
                    offset = offset.saturating_add(*delta);
                    cur = Value(*base);
                }
                PtrDef::GepVar { base } => {
                    return Some(CellAddr {
                        root: self.root_of(Value(*base))?,
                        offset: None,
                    });
                }
                PtrDef::Alloca => {
                    return Some(CellAddr {
                        root: CellRoot::Alloca(cur.0),
                        offset: Some(offset),
                    });
                }
                PtrDef::Global(name) => {
                    return Some(CellAddr {
                        root: CellRoot::Global(name.clone()),
                        offset: Some(offset),
                    });
                }
                PtrDef::Opaque => {
                    return Some(CellAddr {
                        root: CellRoot::Other(cur.0),
                        offset: Some(offset),
                    });
                }
            }
        }
    }

    fn root_of(&self, v: Value) -> Option<CellRoot> {
        self.resolve(v).map(|c| c.root)
    }
}

/// Alloca roots whose derived pointers are only ever used as `Load.ptr`,
/// `Store.ptr`, or the base of a pointer chain that ends in such uses.
/// `derived[v] = root` maps every SSA value that is a const-offset-derived
/// pointer of an alloca.
fn compute_closed_allocas(func: &IrFunction, defs: &FxHashMap<u32, PtrDef>) -> FxHashSet<u32> {
    let mut derived: FxHashMap<u32, u32> = FxHashMap::default();
    let mut closed: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { dest, .. } => {
                    derived.insert(dest.0, dest.0);
                    closed.insert(dest.0);
                }
                Instruction::Copy { dest, src } => {
                    if let Operand::Value(s) = src {
                        if let Some(&root) = derived.get(&s.0) {
                            derived.insert(dest.0, root);
                        }
                    }
                }
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(_),
                    ..
                } => {
                    if let Some(&root) = derived.get(&base.0) {
                        derived.insert(dest.0, root);
                    }
                }
                _ => {}
            }
        }
    }
    let open = |v: u32, derived: &FxHashMap<u32, u32>, closed: &mut FxHashSet<u32>| {
        if let Some(&root) = derived.get(&v) {
            closed.remove(&root);
        }
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                // Blessed positions: reading/writing through the pointer.
                Instruction::Load { ptr, .. } => {
                    let _ = ptr;
                }
                Instruction::Store { val, ptr, .. } => {
                    // Storing a derived pointer VALUE takes its address.
                    if let Operand::Value(pv) = val {
                        open(pv.0, &derived, &mut closed);
                    }
                    let _ = ptr;
                }
                // Deriving positions (already in `derived`).
                Instruction::Copy { .. } | Instruction::Alloca { .. } => {}
                Instruction::GetElementPtr {
                    base, offset, dest, ..
                } => {
                    if !matches!(offset, Operand::Const(_)) {
                        // Variable-offset GEP on an alloca chain: the result
                        // is an inspected pointer — the root may escape via
                        // this value even if unused, be conservative.
                        open(base.0, &derived, &mut closed);
                        open(dest.0, &derived, &mut closed);
                    }
                }
                // Everything else: any derived pointer referenced anywhere
                // (call args, phi inputs, selects, atomics, asm inputs AND
                // outputs, memcpy endpoints, intrinsic args/dest_ptr, stored
                // values, casts...) opens it. The operand visitor covers
                // value positions; the value-use visitor covers the pointer
                // positions the operand visitor deliberately skips (memcpy
                // dest/src, inline-asm outputs, intrinsic dest_ptr, va_list
                // pointers, stack-restore pointers).
                _ => {
                    crate::backend::liveness::for_each_operand_in_instruction(
                        inst,
                        &mut |op: &Operand| {
                            if let Operand::Value(v) = op {
                                open(v.0, &derived, &mut closed);
                            }
                        },
                    );
                    crate::backend::liveness::for_each_value_use_in_instruction(
                        inst,
                        &mut |v: &Value| open(v.0, &derived, &mut closed),
                    );
                }
            }
        }
    }
    for block in &func.blocks {
        block
            .terminator
            .for_each_used_value(|id| open(id, &derived, &mut closed));
    }
    closed
}

/// May two roots denote the same object?
fn roots_may_alias(a: &CellRoot, b: &CellRoot) -> bool {
    match (a, b) {
        (CellRoot::Alloca(x), CellRoot::Alloca(y)) => x == y,
        (CellRoot::Global(x), CellRoot::Global(y)) => x == y,
        // Distinct allocas / distinct globals are provably disjoint.
        (CellRoot::Alloca(_), CellRoot::Global(_)) | (CellRoot::Global(_), CellRoot::Alloca(_)) => {
            false
        }
        // An arbitrary pointer can name any open object.
        (CellRoot::Other(_), _) | (_, CellRoot::Other(_)) => true,
    }
}

fn overlaps(a_off: i64, a_sz: i64, b_off: i64, b_sz: i64) -> bool {
    a_off < b_off + b_sz && b_off < a_off + a_sz
}

/// Kill every pending entry that `root` (at unknown offset) may overwrite.
fn kill_unknown_offset(pending: &mut Vec<Pending>, root: &CellRoot) {
    pending.retain(|p| !roots_may_alias(&p.root, root));
}

/// Kill everything an opaque memory event (call, asm, memcpy, atomics...)
/// could touch.  Closed allocas survive only when this function cannot be
/// re-entered by a non-local goto (see `has_nonlocal_targets`): no other
/// pointer, and no callee, can otherwise name them.
fn kill_opaque(pending: &mut Vec<Pending>, ctx: &DseContext) {
    if ctx.has_nonlocal_targets {
        // Non-local-goto re-entry: this frame carries a NonlocalGotoSave
        // area, so any callee can transfer control to a label alias inside
        // THIS function.  Execution resumes on an edge invisible to the
        // per-block CFG, where a load of a "closed" alloca cell can run
        // BEFORE the in-block overwriter this scan treated as covering the
        // store.  Closed-alloca survival is unsound here; opaque events
        // kill everything open.
        pending.clear();
        return;
    }
    pending.retain(|p| matches!(&p.root, CellRoot::Alloca(a) if ctx.closed_allocas.contains(a)));
}

/// Backward scan of one block. Returns indices of dead stores.
fn dse_block(block: &crate::ir::reexports::BasicBlock, ctx: &DseContext) -> Vec<usize> {
    let mut pending: Vec<Pending> = Vec::new();
    let mut dead: Vec<usize> = Vec::new();
    for ii in (0..block.instructions.len()).rev() {
        let inst = &block.instructions[ii];
        match inst {
            Instruction::Store {
                ptr,
                ty,
                seg_override,
                volatile,
                ..
            } => {
                if *seg_override != AddressSpace::Default {
                    continue; // other address space: isolated
                }
                let size = access_size(*ty);
                let addr = match ctx.resolve(*ptr) {
                    Some(a) => a,
                    None => {
                        // Unresolvable pointer: the store may write anywhere
                        // reachable. Fail closed.
                        kill_opaque(&mut pending, ctx);
                        continue;
                    }
                };
                if let Some(off) = addr.offset {
                    if size >= 0 {
                        // Exact cell: is a pending later store fully covering it?
                        if !*volatile {
                            if let Some(_p) = pending
                                .iter()
                                .find(|p| p.root == addr.root && p.offset == off && p.size >= size)
                            {
                                dead.push(ii);
                                // The dead store is removed, but the pending
                                // overwriter stays: an even earlier store to
                                // the same cell is equally dead.
                                continue;
                            }
                        }
                        pending.push(Pending {
                            root: addr.root,
                            offset: off,
                            size,
                        });
                        continue;
                    }
                    // Exotic type: not an elimination target; conservatively
                    // kill same-root pending entries.
                    kill_unknown_offset(&mut pending, &addr.root);
                    continue;
                }
                // Variable-offset store: kills same-root pendings; cannot be
                // an exact overwriter.
                kill_unknown_offset(&mut pending, &addr.root);
            }
            Instruction::Load {
                ptr,
                ty,
                seg_override,
                ..
            } => {
                if *seg_override != AddressSpace::Default {
                    continue;
                }
                match ctx.resolve(*ptr) {
                    Some(CellAddr {
                        root,
                        offset: Some(off),
                    }) => {
                        let sz = access_size(*ty);
                        let sz = if sz >= 0 { sz } else { 16 };
                        pending.retain(|p| {
                            if roots_may_alias(&p.root, &root) {
                                // Same object (or unknown overlap): kill on
                                // byte-range intersection.
                                !overlaps(p.offset, p.size, off, sz)
                            } else {
                                true
                            }
                        });
                    }
                    Some(CellAddr { root, offset: None }) => {
                        kill_unknown_offset(&mut pending, &root)
                    }
                    None => kill_opaque(&mut pending, ctx),
                }
            }
            Instruction::Memcpy { .. }
            | Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::InlineAsm { .. }
            | Instruction::VaStart { .. }
            | Instruction::VaEnd { .. }
            | Instruction::VaCopy { .. }
            | Instruction::VaArg { .. }
            | Instruction::VaArgStruct { .. }
            | Instruction::AtomicRmw { .. }
            | Instruction::AtomicInc { .. }
            | Instruction::AtomicCmpxchg { .. }
            | Instruction::AtomicLoad { .. }
            | Instruction::AtomicStore { .. }
            | Instruction::Fence { .. }
            | Instruction::DynAlloca { .. }
            | Instruction::StackRestore { .. } => {
                kill_opaque(&mut pending, ctx);
            }
            Instruction::Intrinsic { op, dest_ptr, .. } => {
                if !op.is_pure() || dest_ptr.is_some() {
                    kill_opaque(&mut pending, ctx);
                }
            }
            // Pure instructions (BinOp, Cmp, Cast, Copy, Phi, GEP,
            // GlobalAddr, ParamRef, Select, UnaryOp, LabelAddr, StackSave,
            // GetReturn*/SetReturn*, PgoCounterInc) neither read nor write
            // user memory.
            _ => {}
        }
    }
    // The scan ran in reverse, so `dead` is descending; the removal filter
    // expects ascending indices.
    dead.sort_unstable();
    dead
}

/// Run same-block dead store elimination on one function.
pub(crate) fn eliminate_dead_stores(func: &mut IrFunction) -> usize {
    if func.blocks.is_empty() || std::env::var_os("CCC_NO_DSE").is_some() {
        return 0;
    }
    let ctx = DseContext::build(func);
    let mut total = 0usize;
    for block in func.blocks.iter_mut() {
        let dead = dse_block(block, &ctx);
        if dead.is_empty() {
            continue;
        }
        total += dead.len();
        // `dead` is ascending by construction (the scan runs in reverse).
        // Remove those instruction indices in one stable pass.
        let mut di = 0usize;
        let mut span = std::mem::take(&mut block.source_spans);
        block.instructions = block
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(ii, inst)| {
                let keep = if di < dead.len() && dead[di] == ii {
                    di += 1;
                    false
                } else {
                    true
                };
                keep.then(|| inst.clone())
            })
            .collect();
        if span.len() >= dead.len() {
            // Rebuild spans in parallel: drop the same positions.
            let mut di = 0usize;
            let mut new_span = Vec::with_capacity(span.len() - dead.len());
            for (ii, s) in span.drain(..).enumerate() {
                if di < dead.len() && dead[di] == ii {
                    di += 1;
                } else {
                    new_span.push(s);
                }
            }
            span = new_span;
        }
        block.source_spans = span;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Terminator};

    fn mkfunc() -> IrFunction {
        IrFunction::new("t".to_string(), IrType::Void, vec![], false)
    }

    fn store_const(dest_ptr: u32, imm: i32, idx: usize) -> Instruction {
        let _ = idx;
        Instruction::Store {
            volatile: false,
            val: Operand::Const(IrConst::I32(imm)),
            ptr: Value(dest_ptr),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
        }
    }

    #[test]
    fn overwritten_alloca_store_dies() {
        let mut f = mkfunc();
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                store_const(0, 1, 1),
                store_const(0, 2, 2),
                store_const(0, 3, 3),
                Instruction::Load {
                    volatile: false,
                    dest: Value(9),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = eliminate_dead_stores(&mut f);
        assert_eq!(n, 2, "x=1 and x=2 are overwritten before the load");
        let stores: Vec<_> = f.blocks[0]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Store { .. }))
            .collect();
        assert_eq!(stores.len(), 1);
    }

    #[test]
    fn load_between_stores_keeps_both() {
        let mut f = mkfunc();
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                store_const(0, 1, 1),
                Instruction::Load {
                    volatile: false,
                    dest: Value(7),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                store_const(0, 2, 2),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = eliminate_dead_stores(&mut f);
        assert_eq!(n, 0, "the load observes the first store");
    }

    #[test]
    fn nonlocal_goto_target_keeps_cross_call_store() {
        // This frame IS a non-local-goto target (NonlocalGotoSave): a
        // callee can jump to a label alias inside this function and resume
        // on a CFG-invisible edge where x is loaded BEFORE the in-block
        // overwriting store runs.  The closed-alloca store before the call
        // must therefore survive DSE, while the plain dead store on the
        // second cell (no call between the pair) is still eliminated.
        let mut f = mkfunc();
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::NonlocalGotoSave {
                    frame: Value(2),
                    rbp_off: 8,
                    rsp_off: 16,
                },
                store_const(0, 1, 1),
                Instruction::Call {
                    func: "sink".to_string(),
                    info: crate::ir::reexports::CallInfo {
                        args: vec![],
                        arg_types: vec![],
                        ..crate::ir::reexports::CallInfo::default()
                    },
                },
                store_const(0, 2, 2),
                store_const(1, 3, 3),
                store_const(1, 4, 4),
                Instruction::Load {
                    volatile: false,
                    dest: Value(9),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = eliminate_dead_stores(&mut f);
        assert_eq!(
            n, 1,
            "cell-1 store 3 dies (overwritten, no call between); the \
             cross-call cell-0 store must survive the goto re-entry hazard"
        );
    }

    #[test]
    fn closed_alloca_store_dies_across_call_without_goto_targets() {
        // Control for nonlocal_goto_target_keeps_cross_call_store: the
        // identical shape WITHOUT a NonlocalGotoSave — no re-entry edge
        // exists, closed-alloca survival across the opaque call is sound,
        // and the pre-call store is eliminated as usual.
        let mut f = mkfunc();
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                store_const(0, 1, 1),
                Instruction::Call {
                    func: "sink".to_string(),
                    info: crate::ir::reexports::CallInfo {
                        args: vec![],
                        arg_types: vec![],
                        ..crate::ir::reexports::CallInfo::default()
                    },
                },
                store_const(0, 2, 2),
                Instruction::Load {
                    volatile: false,
                    dest: Value(9),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = eliminate_dead_stores(&mut f);
        assert_eq!(n, 1, "x=1 is overwritten by x=2 after the (non-goto) call");
    }

    #[test]
    fn struct_field_stores_dont_interfere() {
        let mut f = mkfunc();
        // struct { int a, b; } *s;  s->a=1; s->a=2; s->b=3;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::GetElementPtr {
                    dest: Value(2),
                    base: Value(1),
                    offset: Operand::Const(IrConst::I64(0)),
                    ty: IrType::Ptr,
                },
                store_const(2, 1, 1),
                store_const(2, 2, 2),
                Instruction::GetElementPtr {
                    dest: Value(3),
                    base: Value(1),
                    offset: Operand::Const(IrConst::I64(4)),
                    ty: IrType::Ptr,
                },
                store_const(3, 3, 3),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = eliminate_dead_stores(&mut f);
        assert_eq!(n, 1, "s->a=1 dies; s->a=2 and s->b=3 stay");
        let stores: Vec<i32> = f.blocks[0]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Store {
                    val: Operand::Const(IrConst::I32(v)),
                    ..
                } => Some(*v),
                _ => None,
            })
            .collect();
        assert_eq!(stores, vec![2, 3]);
    }

    #[test]
    fn call_between_stores_blocks_kill() {
        let mut f = mkfunc();
        // int x; x=1; sink(); x=2;  — sink may read x? x is a CLOSED alloca
        // (address never escapes), so the call cannot name it: x=1 still
        // dies. Then verify the opposite: when &x escapes to a call, both
        // stores survive.
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                store_const(0, 1, 1),
                Instruction::Call {
                    func: "sink".to_string(),
                    info: crate::ir::reexports::CallInfo {
                        args: vec![Operand::Value(Value(0))],
                        arg_types: vec![IrType::Ptr],
                        ..crate::ir::reexports::CallInfo::default()
                    },
                },
                store_const(0, 2, 3),
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = eliminate_dead_stores(&mut f);
        assert_eq!(
            n, 0,
            "&x escapes into sink(): the first store is observable"
        );
    }

    #[test]
    fn volatile_store_never_eliminated_but_overwrites() {
        let mut f = mkfunc();
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                store_const(0, 1, 1),
                Instruction::Store {
                    volatile: true,
                    val: Operand::Const(IrConst::I32(2)),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let n = eliminate_dead_stores(&mut f);
        assert_eq!(n, 1, "x=1 is overwritten by the volatile store");
        assert!(f.blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Store { volatile: true, .. })));
    }
}
