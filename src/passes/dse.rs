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
//!   whose cell is fully covered by pending stores (same root, the union of
//!   their byte ranges contains the store's; see `covered`) with **no
//!   intervening read of that cell**, the earlier store is dead: any reader
//!   between the two would have killed the pending entries first. Vector store intrinsics
//!   (`VecStore*`) are stores of their exact width here: they cover earlier
//!   stores, die under later ones, and read nothing.
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
//! * A 16-byte *load intrinsic* (`Loaddqu`, `Loadldi128`) whose loaded value
//!   has no uses and whose destination slot is a **private** alloca —
//!   mentioned exactly once in the whole function, by that instruction — is
//!   removed outright. The generated SIMD wrappers read a builtin's result
//!   back out of the slot it wrote (`__CCC_M128D_FROM_BUILTIN(p)` is
//!   `*(__m128d *)p`), and when that value is immediately recycled as an
//!   operand pointer by the next intrinsic — `_mm_storeu_pd(o,
//!   _mm_min_pd(x, y))` — the dereference is a load into a fresh temp that
//!   nothing reads. It costs a slot store and a reload per occurrence, and it
//!   stops the value from being forwarded: measured on the SSE2 codegen
//!   matrix, `minpd`/`maxpd`/`divpd`/`shufpd`/`cmppd` all carried 6-20
//!   instructions of this traffic over the same kernel written with `addpd`.
//! * Volatile stores are never eliminated but may overwrite (kill) earlier
//!   stores; volatile loads read memory and kill like ordinary loads.
//!   Segment-relative reads are opaque barriers: their offsets cannot be
//!   compared to default-space offsets. Segment stores supply no coverage
//!   proof and are never eliminated here.
//! * A C volatile *object* (an alloca with `semantic_volatile`) is written
//!   where the program writes it, its initializer included: a volatile store
//!   of it is an event like a read, which hides nothing and ends what the
//!   stores behind it can hide, so `volatile struct s v = {1, 2}; v.a = 3;`
//!   keeps the 1 it began with.  What an initializer writes twice it still
//!   writes once -- the zero fill under the member stores is dead under
//!   them, as for any object -- and [`close_volatile_objects`] then makes
//!   each access of the object the volatile instruction every later pass
//!   and the backends respect.
//!
//! Scope is deliberately same-block. Cross-block DSE needs post-dominator
//! plus read-between proofs — that is the `segments`-based OP-33 follow-up.
//!
//! Disable with `CCC_NO_DSE=1`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{Instruction, IrFunction, Operand, Value};

/// `(definitely, possibly)` written/read bytes of an IR access type for cell
/// overlap math (`IrType::access_extent`).  `None` marks types whose cell
/// extent is not modeled: such stores are never elimination targets and
/// conservatively kill same-root pending entries.
fn access_extent(ty: IrType) -> Option<(i64, i64)> {
    ty.access_extent()
}

/// Identity of the object a cell lives in.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum CellRoot {
    /// A stack object (alloca value id). Distinct allocas never alias.
    Alloca(u32),
    /// A global symbol (by name). Distinct names may be GNU aliases.
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
    /// Bytes the later store DEFINITELY writes (the `access_extent` lower
    /// bound): only those prove an earlier store dead.
    size: i64,
}

struct DseContext {
    defs: FxHashMap<u32, PtrDef>,
    /// Mutable copy-web values cannot be resolved with a function-wide map.
    multi_defs: FxHashSet<u32>,
    /// Alloca value ids whose address provably never escapes.
    closed_allocas: FxHashSet<u32>,
    /// Alloca value ids of C volatile objects (`semantic_volatile`).
    volatile_objects: FxHashSet<u32>,
    /// This function IS a non-local-goto target (it holds a
    /// NonlocalGotoSave area): any callee can transfer control to a label
    /// alias inside this function, creating a re-entry edge invisible to
    /// the per-block CFG.  Closed-alloca survival across opaque events is
    /// unsound in that case.
    has_nonlocal_targets: bool,
}

impl DseContext {
    fn build(func: &IrFunction) -> Self {
        let multi_defs = super::gvn::find_multi_def_values(func);
        let mut defs: FxHashMap<u32, PtrDef> = FxHashMap::default();
        let mut volatile_objects = FxHashSet::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if inst.dest().is_some_and(|v| multi_defs.contains(&v.0)) {
                    continue;
                }
                match inst {
                    Instruction::Alloca {
                        dest,
                        semantic_volatile,
                        ..
                    } => {
                        defs.insert(dest.0, PtrDef::Alloca);
                        if *semantic_volatile {
                            volatile_objects.insert(dest.0);
                        }
                    }
                    Instruction::GlobalAddr { dest, name } => {
                        defs.insert(dest.0, PtrDef::Global(name.as_str().into()));
                    }
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => {
                        let d = match offset {
                            Operand::Const(c) => {
                                match c.to_i128().and_then(|n| i64::try_from(n).ok()) {
                                    Some(delta) => PtrDef::GepConst {
                                        base: base.0,
                                        delta,
                                    },
                                    None => PtrDef::GepVar { base: base.0 },
                                }
                            }
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
        let closed_allocas = compute_closed_allocas(func, &multi_defs);
        let has_nonlocal_targets = func.blocks.iter().any(|block| {
            block
                .instructions
                .iter()
                .any(|inst| matches!(inst, Instruction::NonlocalGotoSave { .. }))
        });
        DseContext {
            defs,
            multi_defs,
            closed_allocas,
            volatile_objects,
            has_nonlocal_targets,
        }
    }

    /// Is `root` a C volatile object of this function?
    fn is_volatile_object(&self, root: &CellRoot) -> bool {
        matches!(root, CellRoot::Alloca(id) if self.volatile_objects.contains(id))
    }

    /// Resolve a pointer value to a cell address. Values with no defining
    /// instruction (params) are opaque roots by value id.
    fn resolve(&self, v: Value) -> Option<CellAddr> {
        let mut cur = v;
        let mut offset = Some(0i64);
        let mut seen = FxHashSet::default();
        loop {
            if self.multi_defs.contains(&cur.0) || !seen.insert(cur.0) {
                return None;
            }
            let def = match self.defs.get(&cur.0) {
                Some(d) => d,
                None => {
                    return Some(CellAddr {
                        root: CellRoot::Other(cur.0),
                        offset,
                    });
                }
            };
            match def {
                PtrDef::CopyOf(src) => cur = Value(*src),
                PtrDef::GepConst { base, delta } => {
                    offset = offset.and_then(|off| off.checked_add(*delta));
                    cur = Value(*base);
                }
                PtrDef::GepVar { base } => {
                    offset = None;
                    cur = Value(*base);
                }
                PtrDef::Alloca => {
                    return Some(CellAddr {
                        root: CellRoot::Alloca(cur.0),
                        offset,
                    });
                }
                PtrDef::Global(name) => {
                    return Some(CellAddr {
                        root: CellRoot::Global(name.clone()),
                        offset,
                    });
                }
                PtrDef::Opaque => {
                    return Some(CellAddr {
                        root: CellRoot::Other(cur.0),
                        offset,
                    });
                }
            }
        }
    }
}

/// Alloca roots whose derived pointers are only ever used as `Load.ptr`,
/// `Store.ptr`, or the base of a pointer chain that ends in such uses.
/// Escape reachability is independent of block storage order and retains
/// every defining edge of mutable copy webs.
fn compute_closed_allocas(func: &IrFunction, multi_defs: &FxHashSet<u32>) -> FxHashSet<u32> {
    // Trace escape seeds BACKWARDS through all defining edges. A forward
    // single scan depends on block storage order and loses one of the roots
    // of a multi-defined Copy. Both can falsely call an escaping alloca closed.
    let mut parents: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut closed = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { dest, .. } => {
                    closed.insert(dest.0);
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } => {
                    parents.entry(dest.0).or_default().push(src.0);
                }
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(_),
                    ..
                } => {
                    parents.entry(dest.0).or_default().push(base.0);
                }
                _ => {}
            }
        }
    }
    let mut seeds = FxHashSet::default();
    let mut open = |id: u32| {
        if parents.contains_key(&id) || closed.contains(&id) {
            seeds.insert(id);
        }
    };
    // An unresolved mutable pointer may itself read an alloca. Such roots
    // must not survive kill_opaque merely because its uses look nonescaping.
    for &id in multi_defs {
        open(id);
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { .. }
                | Instruction::Copy { .. }
                | Instruction::Load { .. }
                | Instruction::GetElementPtr {
                    offset: Operand::Const(_),
                    ..
                } => {}
                Instruction::Store {
                    val: Operand::Value(v),
                    ..
                } => open(v.0),
                Instruction::Store { .. } => {}
                _ => {
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            open(v.0);
                        }
                    });
                    crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                        open(v.0)
                    });
                }
            }
        }
        block.terminator.for_each_used_value(&mut open);
    }
    let mut work: Vec<u32> = seeds.iter().copied().collect();
    while let Some(id) = work.pop() {
        closed.remove(&id);
        if let Some(sources) = parents.get(&id) {
            for &src in sources {
                if seeds.insert(src) {
                    work.push(src);
                }
            }
        }
    }
    closed
}

/// May two roots denote the same object?
fn roots_may_alias(a: &CellRoot, b: &CellRoot) -> bool {
    match (a, b) {
        (CellRoot::Alloca(x), CellRoot::Alloca(y)) => x == y,
        // Function-local analysis has no module alias/interposition proof.
        // GNU aliases can bind distinct symbol names to the SAME storage.
        (CellRoot::Global(_), CellRoot::Global(_)) => true,
        // Stack allocations are disjoint from each other and global storage.
        (CellRoot::Alloca(_), CellRoot::Global(_)) | (CellRoot::Global(_), CellRoot::Alloca(_)) => {
            false
        }
        // An arbitrary pointer can name any open object.
        (CellRoot::Other(_), _) | (_, CellRoot::Other(_)) => true,
    }
}

fn overlaps(a_off: i64, a_sz: i64, b_off: i64, b_sz: i64) -> bool {
    match (a_off.checked_add(a_sz), b_off.checked_add(b_sz)) {
        (Some(a_end), Some(b_end)) => a_off < b_end && b_off < a_end,
        _ => true, // an unrepresentable range is not a disjointness proof
    }
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

/// Do the pending later writes to `root` together definitely overwrite every
/// byte of `[off, off + len)`? Reads kill pending entries on any byte
/// overlap, so each surviving entry proves that no read of ITS bytes
/// intervenes; when their union contains the range, every byte of the
/// earlier store is overwritten before anything can observe it. One entry
/// may contain the range (one 32-byte vector store hides the four 8-byte
/// stores under it), or several may tile it: the zero fill of an aggregate
/// initializer (`E e = {5, 6}` stores an 8-byte zero, then both 4-byte
/// fields) is dead under its field stores.
fn covered(pending: &[Pending], root: &CellRoot, off: i64, len: i64) -> bool {
    let Some(end) = off.checked_add(len) else {
        return false;
    };
    let mut parts: Vec<(i64, i64)> = Vec::new();
    for p in pending.iter().filter(|p| p.root == *root) {
        let Some(p_end) = p.offset.checked_add(p.size) else {
            continue;
        };
        if p.offset <= off && end <= p_end {
            return true;
        }
        if p.offset < end && off < p_end {
            parts.push((p.offset, p_end));
        }
    }
    if parts.len() < 2 {
        return false;
    }
    parts.sort_unstable();
    let mut reach = off;
    for (start, stop) in parts {
        if start > reach {
            return false;
        }
        reach = reach.max(stop);
        if reach >= end {
            return true;
        }
    }
    false
}

/// The `(root, constant byte offset)` a vector store writes at, when its
/// base resolves to a constant-offset cell and its index is a constant.
fn vec_store_cell(
    ctx: &DseContext,
    base: &Operand,
    index: &Operand,
    disp: i64,
) -> Option<(CellRoot, i64)> {
    let Operand::Value(base) = base else {
        return None;
    };
    let index = match index {
        Operand::Const(c) => c.to_i64()?,
        Operand::Value(_) => return None,
    };
    let addr = ctx.resolve(*base)?;
    let off = addr.offset?.checked_add(index)?.checked_add(disp)?;
    Some((addr.root, off))
}

/// Which stores a scan may remove.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Any store that is dead.
    Function,
    /// Only what an initializer of a C volatile object wrote twice: the
    /// plain stores into such an object ([`close_volatile_objects`]).
    VolatileObjects,
}

/// Backward scan of one block. Returns indices of dead stores.
fn dse_block(
    block: &crate::ir::reexports::BasicBlock,
    ctx: &DseContext,
    scope: Scope,
) -> Vec<usize> {
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
                    continue; // no coverage proof; a write alone observes no earlier store
                }
                let extent = access_extent(*ty);
                let addr = match ctx.resolve(*ptr) {
                    Some(a) => a,
                    None => {
                        // No coverage proof, but an ordinary store cannot
                        // observe the value hidden by a later overwriter.
                        continue;
                    }
                };
                let in_volatile_object = ctx.is_volatile_object(&addr.root);
                if *volatile && in_volatile_object {
                    // A volatile store of a C volatile object is an event,
                    // as a read of it is: what the object held up to here
                    // was there to be seen, so the stores behind this one
                    // hide nothing of the bytes it names, and it hides
                    // nothing itself.  `volatile int x = 1; x = 2;` writes
                    // twice (GCC 16.2 and Clang 23.1 both do); the 1 used to
                    // be dead under the 2.
                    match (addr.offset, extent) {
                        (Some(off), Some((_, may_write))) => pending.retain(|p| {
                            p.root != addr.root || !overlaps(p.offset, p.size, off, may_write)
                        }),
                        _ => pending.retain(|p| p.root != addr.root),
                    }
                    continue;
                }
                if let Some(off) = addr.offset {
                    if let Some((written, may_write)) = extent {
                        // Exact cell: does a pending later store definitely
                        // overwrite every byte this one may write?
                        if !*volatile
                            && (scope == Scope::Function || in_volatile_object)
                            && covered(&pending, &addr.root, off, may_write)
                        {
                            dead.push(ii);
                            // The dead store is removed, but the pending
                            // overwriter stays: an even earlier store to
                            // the same cell is equally dead.
                            continue;
                        }
                        pending.push(Pending {
                            root: addr.root,
                            offset: off,
                            size: written,
                        });
                        continue;
                    }
                    // Unknown extent cannot establish overwrite coverage.
                    continue;
                }
                // A variable-offset store contributes no coverage, but does
                // not invalidate a later overwrite's coverage either.
            }
            Instruction::Load {
                ptr,
                ty,
                seg_override,
                ..
            } => {
                if *seg_override != AddressSpace::Default {
                    kill_opaque(&mut pending, ctx);
                    continue;
                }
                match ctx.resolve(*ptr) {
                    Some(CellAddr {
                        root,
                        offset: Some(off),
                    }) => {
                        let sz = access_extent(*ty).map_or(16, |(_, may_read)| may_read);
                        pending.retain(|p| {
                            if roots_may_alias(&p.root, &root) {
                                // Same object (or unknown overlap): kill on
                                // byte-range intersection.
                                // Offsets are relative to their roots. Offsets
                                // 0 and 8 do not prove p and q+8 disjoint when
                                // q may equal p-8. Only a shared root permits
                                // comparing these coordinate systems.
                                p.root == root && !overlaps(p.offset, p.size, off, sz)
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
            Instruction::Intrinsic {
                op, dest_ptr, args, ..
            } => {
                // A vector store is a plain write of a known width
                // (`vec_intrinsic_access`: `[src, base, index, disp]`): it
                // reads nothing, so it never kills a pending entry, and at a
                // resolved constant address it is a store like any other --
                // an overwriter for earlier stores it covers, and dead when
                // a later write covers it. An unresolvable one, like an
                // unresolvable `Store`, proves nothing either way.
                if let Some((true, bytes, base, index, disp)) =
                    super::alias::vec_intrinsic_access(op, args)
                {
                    if let Some((root, off)) = vec_store_cell(ctx, base, index, disp) {
                        if (scope == Scope::Function || ctx.is_volatile_object(&root))
                            && covered(&pending, &root, off, bytes)
                        {
                            dead.push(ii);
                        } else {
                            pending.push(Pending {
                                root,
                                offset: off,
                                size: bytes,
                            });
                        }
                    }
                    continue;
                }
                // Pure means no observable effects, not independence from memory.
                if !op.is_pure() || dest_ptr.is_some() || op.may_read_memory() {
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

/// Does this instruction only read memory into `dest_ptr`, with no other
/// observable effect?  Exactly the 16-byte load intrinsic families; the list
/// is explicit rather than derived from `is_pure()` because a load intrinsic
/// is a *write* to its destination slot while `is_pure()` is about the absence
/// of memory effects altogether.
fn is_pure_slot_load(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(op, O::Loaddqu | O::Loadldi128)
}

/// Remove 16-byte load intrinsics whose loaded value is dead and whose
/// destination slot is private to them.
///
/// Fail-closed: the slot must be an alloca mentioned EXACTLY ONCE in the whole
/// function (this instruction's `dest_ptr`), so nothing else can read what it
/// writes; the loaded value must have no operand or terminator uses.  See the
/// module header for why this shape is common enough to be worth a pass of its
/// own.
pub(crate) fn eliminate_dead_private_slot_loads(func: &mut IrFunction) -> usize {
    // No environment read here on purpose: both call sites own their gate.
    // `eliminate_dead_stores` has already checked `CCC_NO_DSE`, and the driver
    // phase (`deadslotload`) goes through `pass_disabled`.  A third read would
    // also grow the audited pass-pipeline environment-read budget for nothing.
    if func.blocks.is_empty() {
        return 0;
    }
    // Mention count per value: every operand of every instruction (which
    // includes an intrinsic's `dest_ptr`) plus every terminator use.
    let mut mentions: FxHashMap<u32, usize> = FxHashMap::default();
    let mut alloca_defs: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                alloca_defs.insert(dest.0);
            }
            // Two iterators, because they cover disjoint fields: the operand
            // walk visits `Intrinsic.args`, and `dest_ptr` is only visible to
            // the *use* walk.  Counting one and not the other counted a slot
            // written by the instruction that the candidate also names as its
            // destination as "mentioned once", which deleted a live load
            // (`simd_movnt`: the `movntdq` destination slot disappeared).
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *mentions.entry(v.0).or_insert(0) += 1;
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                *mentions.entry(v.0).or_insert(0) += 1;
            });
        }
        block.terminator.for_each_used_value(|v: u32| {
            *mentions.entry(v).or_insert(0) += 1;
        });
    }

    // `mentions` counts operands only, so a value with no entry is unused.
    let mut removed = 0usize;
    for block in func.blocks.iter_mut() {
        let keep: Vec<bool> = block
            .instructions
            .iter()
            .map(|inst| match inst {
                Instruction::Intrinsic {
                    dest: Some(d),
                    op,
                    dest_ptr: Some(slot),
                    ..
                } => {
                    !(is_pure_slot_load(op)
                        && alloca_defs.contains(&slot.0)
                        && mentions.get(&slot.0).copied() == Some(1)
                        && mentions.get(&d.0).copied().unwrap_or(0) == 0)
                }
                _ => true,
            })
            .collect();
        removed += keep.iter().filter(|&&k| !k).count();
        let mut i = 0;
        block.retain_insts(|_| {
            let k = keep[i];
            i += 1;
            k
        });
    }
    removed
}

/// Remove from every block the stores a scan of `scope` finds dead.
fn remove_dead_stores(func: &mut IrFunction, ctx: &DseContext, scope: Scope) -> usize {
    let mut total = 0;
    for block in func.blocks.iter_mut() {
        let dead = dse_block(block, ctx, scope);
        if dead.is_empty() {
            continue;
        }
        total += dead.len();
        // `dead` is ascending by construction (the scan runs in reverse).
        // One stable retain keeps instructions, spans and inline ids in
        // lockstep; cloning the instruction list here also duplicated every
        // operand vector for no reason.
        let mut di = 0usize;
        let mut ii = 0usize;
        block.retain_insts(|_| {
            let keep = if di < dead.len() && dead[di] == ii {
                di += 1;
                false
            } else {
                true
            };
            ii += 1;
            keep
        });
    }
    total
}

/// Run same-block dead store elimination on one function.
pub(crate) fn eliminate_dead_stores(func: &mut IrFunction) -> usize {
    if func.blocks.is_empty() || std::env::var_os("CCC_NO_DSE").is_some() {
        return 0;
    }
    let total = eliminate_dead_private_slot_loads(func);
    let ctx = DseContext::build(func);
    total + remove_dead_stores(func, &ctx, Scope::Function)
}

/// Make every access of a C volatile object the volatile instruction it is.
/// Returns the number of instructions removed and marked.
///
/// The lowering marks the accesses it makes through an lvalue of volatile
/// type.  What it writes of an initializer it writes as plain stores --
/// `volatile struct s v = {1, 2};` is a zero fill and a store for each
/// member -- and so do the paths that reach into an object without an
/// lvalue of their own (`__real__ z = 3.0` of a volatile `_Complex
/// double`).  A plain store is one the optimizer may take for dead and the
/// backend may split:
///
/// ```c
/// volatile struct s v = {1, 2};  v.a = 3;   // the 1 was never written
/// volatile char c[8] = "abcdefg"; c[0] = 'x';   // nor the 'a'
/// __real__ z = 3.0;   // two 4-byte stores of one volatile double
/// ```
///
/// (GCC 16.2 writes the initializer whole and then the assignment, and the
/// double in one `movq`.)  So, once per function and before any pass:
///
/// 1. with `merge_initializers`, what an initializer wrote twice is written
///    once: the scan of [`Scope::VolatileObjects`] removes a plain store
///    into a volatile object that later plain stores cover -- the zero fill
///    under the members -- and nothing else of the function.  A volatile
///    store covers nothing there (`dse_block`);
/// 2. every load and store whose address is in a volatile object (the
///    alloca, through copies and element addresses, with constant or
///    variable offsets: [`DseContext::resolve`]) becomes volatile.
///
/// From there on the object's accesses are instructions every pass leaves
/// alone and every backend emits in place, one access each, and no pass has
/// to ask what an address is an address of.  Without optimization there is
/// nothing to merge and step 2 alone runs.
pub(crate) fn close_volatile_objects(func: &mut IrFunction, merge_initializers: bool) -> usize {
    let has_volatile_object = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            matches!(
                inst,
                Instruction::Alloca {
                    semantic_volatile: true,
                    ..
                }
            )
        })
    });
    if !has_volatile_object {
        return 0;
    }
    let ctx = DseContext::build(func);
    let mut changed = if merge_initializers {
        remove_dead_stores(func, &ctx, Scope::VolatileObjects)
    } else {
        0
    };
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            let (ptr, volatile) = match inst {
                Instruction::Load { ptr, volatile, .. }
                | Instruction::Store { ptr, volatile, .. } => (*ptr, volatile),
                _ => continue,
            };
            if !*volatile
                && ctx
                    .resolve(ptr)
                    .is_some_and(|addr| ctx.is_volatile_object(&addr.root))
            {
                *volatile = true;
                changed += 1;
            }
        }
    }
    changed
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
    fn unknown_writes_do_not_observe_values_but_unknown_reads_do() {
        for reader in [false, true] {
            for volatile in [false, true] {
                let mut f = mkfunc();
                let mut unknown = store_const(2, 3, 0);
                if let Instruction::Store { volatile: v, .. } = &mut unknown {
                    *v = volatile;
                }
                let mut instructions = vec![
                    Instruction::GetElementPtr {
                        dest: Value(2),
                        base: Value(0),
                        offset: Operand::Value(Value(1)),
                        ty: IrType::I32,
                    },
                    store_const(0, 1, 0),
                    unknown,
                ];
                if reader {
                    instructions.push(Instruction::Load {
                        dest: Value(3),
                        ptr: Value(2),
                        ty: IrType::I32,
                        volatile: false,
                        seg_override: AddressSpace::Default,
                    });
                }
                instructions.push(store_const(0, 2, 0));
                f.blocks.push(BasicBlock {
                    label: BlockId(0),
                    instructions,
                    terminator: Terminator::Return(None),
                    source_spans: vec![],
                    source_inline_ids: Vec::new(),
                });
                assert_eq!(eliminate_dead_stores(&mut f), usize::from(!reader));
                assert!(
                    f.blocks[0]
                        .instructions
                        .iter()
                        .any(|i| matches!(i, Instruction::Store { ptr: Value(2), .. }))
                );
            }
        }
    }

    #[test]
    fn memory_intrinsics_observe_stores_for_stack_and_parameter_roots() {
        use crate::ir::intrinsics::IntrinsicOp as O;
        for op in [
            O::VecLoadF64x2,
            O::VecLoadF64x4,
            O::VecLoadF32x4,
            O::VecLoadF32x8,
            O::VecLoadI32x4,
            O::VecLoadI32x8,
            O::VecLoadI64x2,
            O::VecLoadI64x4,
            O::VecLoadI16x8,
            O::VecLoadI8x16,
            O::VecLoadI8x32,
            O::VecLoadI16x16,
            O::VecLoadI32x4Pair,
            O::VecLoadWidenI32ToI64x2,
            O::LoadF64x2,
            O::LoadF64x4,
            O::LoadI32x4,
            O::LoadI32x8,
            O::Loadu256,
            O::Load256,
            O::LoaduPs256,
            O::LoaduPd256,
            O::Loadu512,
        ] {
            assert!(op.may_read_memory());
            for stack in [false, true] {
                for reader in [false, true] {
                    let mut f = mkfunc();
                    let mut instructions = vec![
                        if stack {
                            Instruction::Alloca {
                                dest: Value(0),
                                ty: IrType::I32,
                                size: 64,
                                align: 64,
                                volatile: false,
                                semantic_volatile: false,
                            }
                        } else {
                            Instruction::ParamRef {
                                dest: Value(0),
                                param_idx: 0,
                                ty: IrType::Ptr,
                            }
                        },
                        store_const(0, 17, 0),
                    ];
                    if reader {
                        instructions.push(Instruction::Intrinsic {
                            dest: Some(Value(1)),
                            op,
                            dest_ptr: None,
                            args: vec![Operand::Value(Value(0)), Operand::Const(IrConst::I64(0))],
                        });
                    }
                    instructions.push(store_const(0, 29, 1));
                    f.blocks.push(BasicBlock {
                        label: BlockId(0),
                        instructions,
                        terminator: Terminator::Return(None),
                        source_spans: vec![],
                        source_inline_ids: Vec::new(),
                    });
                    assert_eq!(
                        eliminate_dead_stores(&mut f),
                        usize::from(!reader),
                        "{op:?}, stack={stack}, reader={reader}"
                    );
                }
            }
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
            source_inline_ids: Vec::new(),
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
            source_inline_ids: Vec::new(),
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
            source_inline_ids: Vec::new(),
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
            source_inline_ids: Vec::new(),
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
            source_inline_ids: Vec::new(),
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
            source_inline_ids: Vec::new(),
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
            source_inline_ids: Vec::new(),
        });
        let n = eliminate_dead_stores(&mut f);
        assert_eq!(n, 1, "x=1 is overwritten by the volatile store");
        assert!(
            f.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Store { volatile: true, .. }))
        );
    }
}

#[cfg(test)]
mod displaced_alias_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Terminator};

    fn gep(dest: u32, base: u32, offset: i64) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Const(IrConst::I64(offset)),
            ty: IrType::Ptr,
        }
    }
    fn store(ptr: u32) -> Instruction {
        Instruction::Store {
            ptr: Value(ptr),
            val: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
            volatile: false,
            seg_override: AddressSpace::Default,
        }
    }
    fn function(instructions: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("displaced".into(), IrType::Void, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        f
    }

    #[test]
    fn distinct_global_names_are_not_a_disjointness_proof() {
        let mut f = function(vec![
            Instruction::GlobalAddr {
                dest: Value(0),
                name: "object".into(),
            },
            Instruction::GlobalAddr {
                dest: Value(1),
                name: "alias_object".into(),
            },
            store(0),
            Instruction::Load {
                dest: Value(2),
                ptr: Value(1),
                ty: IrType::I32,
                volatile: false,
                seg_override: AddressSpace::Default,
            },
            store(0),
        ]);
        assert_eq!(eliminate_dead_stores(&mut f), 0);
    }

    #[test]
    fn read_offsets_are_comparable_only_for_the_same_root() {
        for same_root in [false, true] {
            for offset in -12..=12 {
                let mut f = function(vec![
                    gep(2, if same_root { 0 } else { 1 }, offset),
                    store(0),
                    Instruction::Load {
                        dest: Value(3),
                        ptr: Value(2),
                        ty: IrType::I32,
                        volatile: false,
                        seg_override: AddressSpace::Default,
                    },
                    store(0),
                ]);
                let expected = usize::from(same_root && !(-3..=3).contains(&offset));
                assert_eq!(
                    eliminate_dead_stores(&mut f),
                    expected,
                    "same={same_root}, off={offset}"
                );
            }
        }
    }

    #[test]
    fn segment_relative_read_is_not_a_default_space_offset_proof() {
        let mut f = function(vec![
            store(0),
            Instruction::Load {
                dest: Value(3),
                ptr: Value(1),
                ty: IrType::I32,
                volatile: false,
                seg_override: AddressSpace::SegFs,
            },
            store(0),
        ]);
        assert_eq!(eliminate_dead_stores(&mut f), 0);
    }

    #[test]
    fn redefined_pointer_is_not_a_function_wide_ssa_fact() {
        let mut f = function(vec![
            Instruction::Copy {
                dest: Value(0),
                src: Operand::Value(Value(1)),
            },
            store(0),
            Instruction::Copy {
                dest: Value(0),
                src: Operand::Value(Value(2)),
            },
            store(0),
        ]);
        assert_eq!(eliminate_dead_stores(&mut f), 0);
    }

    #[test]
    fn unrelated_multidef_values_do_not_disable_independent_stores() {
        let mut f = function(vec![
            Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(1)),
            },
            store(0),
            Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(2)),
            },
            store(0),
        ]);
        assert_eq!(eliminate_dead_stores(&mut f), 1);
    }

    #[test]
    fn mutable_pointer_and_its_derived_chain_are_unresolvable() {
        let f = function(vec![
            Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(1)),
            },
            gep(10, 9, 4),
            Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(2)),
            },
        ]);
        let ctx = DseContext::build(&f);
        assert!(ctx.resolve(Value(9)).is_none());
        assert!(ctx.resolve(Value(10)).is_none());
        assert!(ctx.resolve(Value(1)).is_some());
    }

    #[test]
    fn escape_graph_is_order_independent_and_keeps_every_mutable_root() {
        let f = function(vec![
            Instruction::Alloca {
                dest: Value(0),
                size: 4,
                align: 4,
                ty: IrType::I32,
                volatile: false,
                semantic_volatile: false,
            },
            Instruction::Alloca {
                dest: Value(1),
                size: 4,
                align: 4,
                ty: IrType::I32,
                volatile: false,
                semantic_volatile: false,
            },
            Instruction::Alloca {
                dest: Value(2),
                size: 4,
                align: 4,
                ty: IrType::I32,
                volatile: false,
                semantic_volatile: false,
            },
            Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(0)),
            },
            Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(1)),
            },
        ]);
        let ctx = DseContext::build(&f);
        assert!(!ctx.closed_allocas.contains(&0));
        assert!(!ctx.closed_allocas.contains(&1));
        assert!(ctx.closed_allocas.contains(&2));
        // Valid dominance, deliberately non-topological block storage:
        // entry -> definition -> use, stored as entry/use/definition.
        let mut escaped = function(vec![Instruction::Alloca {
            dest: Value(0),
            size: 4,
            align: 4,
            ty: IrType::I32,
            volatile: false,
            semantic_volatile: false,
        }]);
        escaped.return_type = IrType::Ptr;
        escaped.blocks[0].terminator = Terminator::Branch(BlockId(2));
        escaped.blocks.push(BasicBlock {
            label: BlockId(1),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
            instructions: vec![Instruction::Copy {
                dest: Value(8),
                src: Operand::Value(Value(9)),
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(8)))),
        });
        escaped.blocks.push(BasicBlock {
            label: BlockId(2),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
            instructions: vec![Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(0)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
        });
        assert!(!DseContext::build(&escaped).closed_allocas.contains(&0));
    }

    #[test]
    fn both_canonical_visitor_argument_kinds_open_derived_roots() {
        let mut instructions = Vec::new();
        for id in 0..3 {
            instructions.push(Instruction::Alloca {
                dest: Value(id),
                size: 8,
                align: 8,
                ty: IrType::I64,
                volatile: false,
                semantic_volatile: false,
            });
        }
        instructions.extend([
            Instruction::Copy {
                dest: Value(3),
                src: Operand::Value(Value(0)),
            },
            Instruction::Copy {
                dest: Value(4),
                src: Operand::Value(Value(1)),
            },
            // Cast exposes &Operand; Memcpy exposes bare &Value uses.
            Instruction::Cast {
                dest: Value(5),
                src: Operand::Value(Value(3)),
                from_ty: IrType::Ptr,
                to_ty: IrType::U64,
            },
            Instruction::Memcpy {
                dest: Value(3),
                src: Value(4),
                size: 8,
            },
        ]);
        let f = function(instructions);
        let ctx = DseContext::build(&f);
        assert!(!ctx.closed_allocas.contains(&0));
        assert!(!ctx.closed_allocas.contains(&1));
        assert!(
            ctx.closed_allocas.contains(&2),
            "unrelated storage stays closed"
        );
    }

    #[test]
    fn unresolved_mutable_read_opens_its_objects_but_not_an_independent_alloca() {
        let mut instructions = Vec::new();
        for id in 0..3 {
            instructions.push(Instruction::Alloca {
                dest: Value(id),
                size: 4,
                align: 4,
                ty: IrType::I32,
                volatile: false,
                semantic_volatile: false,
            });
        }
        instructions.extend([
            Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(0)),
            },
            store(0),
            store(2),
            Instruction::Load {
                dest: Value(10),
                ptr: Value(9),
                ty: IrType::I32,
                volatile: false,
                seg_override: AddressSpace::Default,
            },
            Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(1)),
            },
            store(0),
            store(2),
        ]);
        let mut f = function(instructions);
        f.return_type = IrType::I32;
        f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(10))));
        assert_eq!(eliminate_dead_stores(&mut f), 1);
        let stores = |id| {
            f.blocks[0]
                .instructions
                .iter()
                .filter(|i| matches!(i, Instruction::Store { ptr, .. } if ptr.0 == id))
                .count()
        };
        assert_eq!(stores(0), 2, "mutable read observes the first store");
        assert_eq!(stores(2), 1, "unrelated closed object still benefits");
    }

    #[test]
    fn overflowing_and_cyclic_paths_fail_closed_without_recursion() {
        let f = function(vec![gep(2, 0, i64::MAX), gep(3, 2, 1)]);
        let ctx = DseContext::build(&f);
        assert_eq!(ctx.resolve(Value(3)).unwrap().offset, None);
        assert!(overlaps(i64::MAX, 4, 0, 4));
        assert!(!overlaps(i64::MIN, 4, i64::MAX - 4, 4));
        let mut f = function(vec![gep(1, 2, 0), gep(2, 1, 0)]);
        assert!(DseContext::build(&f).resolve(Value(1)).is_none());
        if let Instruction::GetElementPtr { offset, .. } = &mut f.blocks[0].instructions[0] {
            *offset = Operand::Value(Value(9));
        }
        assert!(DseContext::build(&f).resolve(Value(1)).is_none());
        let f = function(vec![Instruction::GetElementPtr {
            dest: Value(2),
            base: Value(0),
            offset: Operand::Const(IrConst::I128(i128::MAX)),
            ty: IrType::Ptr,
        }]);
        assert_eq!(
            DseContext::build(&f).resolve(Value(2)).unwrap().offset,
            None
        );
    }

    fn alloca(dest: u32, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size,
            align: 8,
            volatile: false,
            semantic_volatile: false,
        }
    }
    fn store64(ptr: u32) -> Instruction {
        Instruction::Store {
            ptr: Value(ptr),
            val: Operand::Const(IrConst::I64(0)),
            ty: IrType::I64,
            volatile: false,
            seg_override: AddressSpace::Default,
        }
    }
    fn vec_store(op: crate::ir::intrinsics::IntrinsicOp, base: u32, index: Operand) -> Instruction {
        Instruction::Intrinsic {
            dest: None,
            op,
            dest_ptr: Some(Value(base)),
            args: vec![
                Operand::Value(Value(90)),
                Operand::Value(Value(base)),
                index,
            ],
        }
    }
    fn surviving_stores(f: &IrFunction) -> Vec<usize> {
        f.blocks[0]
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, i)| matches!(i, Instruction::Store { .. } | Instruction::Intrinsic { .. }))
            .map(|(n, _)| n)
            .collect()
    }

    /// The zero-initializer shape SLP leaves behind: a 32-byte vector store
    /// over the four 8-byte stores at offsets 0..24 hides them all (byte
    /// containment, not offset equality); the store at 32 and a store
    /// straddling the vector's end (28..36) survive. A load of offset 8
    /// between them kills the whole pending 32-byte entry (entries are not
    /// split per byte), so every store under it survives: sound, and
    /// exactly as conservative as for a scalar overwriter.
    #[test]
    fn vector_store_covers_the_scalar_stores_beneath_it() {
        use crate::ir::intrinsics::IntrinsicOp as O;
        let c = |v| Operand::Const(IrConst::I64(v));
        for read_8 in [false, true] {
            let mut insts = vec![
                alloca(0, 48),
                gep(1, 0, 8),
                gep(2, 0, 16),
                gep(3, 0, 24),
                gep(4, 0, 32),
                gep(5, 0, 28),
                store64(0), // 6
                store64(1), // 7
                store64(2), // 8
                store64(3), // 9
                store64(4), // 10
                store64(5), // 11
            ];
            if read_8 {
                insts.push(Instruction::Load {
                    volatile: false,
                    dest: Value(20),
                    ptr: Value(1),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                });
            }
            insts.push(vec_store(O::VecStoreI64x4, 0, c(0)));
            let mut f = function(insts);
            let removed = eliminate_dead_stores(&mut f);
            assert_eq!(removed, if read_8 { 0 } else { 4 }, "read_8={read_8}");
            let n = surviving_stores(&f).len();
            assert_eq!(n, if read_8 { 7 } else { 3 }, "read_8={read_8}");
            let has_store_to = |ptr: u32| {
                f.blocks[0]
                    .instructions
                    .iter()
                    .any(|i| matches!(i, Instruction::Store { ptr: p, .. } if p.0 == ptr))
            };
            assert!(has_store_to(4) && has_store_to(5));
            for ptr in [0, 1, 2, 3] {
                assert_eq!(has_store_to(ptr), read_8, "ptr={ptr} read_8={read_8}");
            }
        }
    }

    /// A vector store is itself a store: it dies under a later write that
    /// contains it (16 bytes at 8 under 32 at 0, via the index operand), and
    /// survives one that only overlaps it.
    #[test]
    fn vector_store_dies_under_a_later_covering_write() {
        use crate::ir::intrinsics::IntrinsicOp as O;
        let c = |v| Operand::Const(IrConst::I64(v));
        let mut f = function(vec![
            alloca(0, 64),
            vec_store(O::VecStoreI64x2, 0, c(8)),
            vec_store(O::VecStoreI64x4, 0, c(0)),
        ]);
        assert_eq!(eliminate_dead_stores(&mut f), 1);
        assert!(matches!(
            f.blocks[0].instructions[1],
            Instruction::Intrinsic {
                op: O::VecStoreI64x4,
                ..
            }
        ));
        let mut f = function(vec![
            alloca(0, 64),
            vec_store(O::VecStoreI64x2, 0, c(24)),
            vec_store(O::VecStoreI64x4, 0, c(0)),
        ]);
        assert_eq!(eliminate_dead_stores(&mut f), 0);
    }

    /// A vector store at an unresolvable address (variable index) covers
    /// nothing, and -- reading nothing -- does not hide an earlier store
    /// from a later overwriter either.
    #[test]
    fn unresolved_vector_store_neither_covers_nor_observes() {
        use crate::ir::intrinsics::IntrinsicOp as O;
        let mut f = function(vec![
            alloca(0, 64),
            store64(0),
            vec_store(O::VecStoreI64x4, 0, Operand::Value(Value(30))),
            store64(0),
        ]);
        assert_eq!(eliminate_dead_stores(&mut f), 1);
        assert_eq!(surviving_stores(&f), vec![1, 2]);
        let mut f = function(vec![
            alloca(0, 64),
            store64(0),
            vec_store(O::VecStoreI64x4, 0, Operand::Value(Value(30))),
        ]);
        assert_eq!(eliminate_dead_stores(&mut f), 0);
    }

    fn store_ty(ptr: u32, ty: IrType) -> Instruction {
        Instruction::Store {
            ptr: Value(ptr),
            val: Operand::Const(IrConst::I64(0)),
            ty,
            volatile: false,
            seg_override: AddressSpace::Default,
        }
    }

    fn load_i32(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            volatile: false,
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
        }
    }

    /// `E e = {5, 6}`: an 8-byte zero fill followed by both 4-byte fields.
    /// No single later store contains the fill, but together they do.
    #[test]
    fn field_stores_together_cover_an_aggregate_zero_fill() {
        let mut f = function(vec![
            alloca(0, 8),
            gep(1, 0, 4),
            store_ty(0, IrType::I64),
            store(0),
            store(1),
        ]);
        assert_eq!(eliminate_dead_stores(&mut f), 1);
        assert!(
            !f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::Store {
                    ty: IrType::I64,
                    ..
                }
            )),
            "the zero fill is dead"
        );
    }

    /// The tiling proof needs every byte: a read between the two field
    /// stores observes the fill's upper half, and a gap leaves bytes of the
    /// fill visible to a later reader. Overlapping later stores still tile.
    #[test]
    fn partial_tiling_keeps_the_earlier_store() {
        // fill; field 0; read of field 1 (observes the fill); field 1.
        let mut read_between = function(vec![
            alloca(0, 8),
            gep(1, 0, 4),
            store_ty(0, IrType::I64),
            store(0),
            load_i32(9, 1),
            store(1),
        ]);
        assert_eq!(eliminate_dead_stores(&mut read_between), 0);
        // fill (8 bytes at 0); 4 bytes at 0 and 2 bytes at 4: bytes 6..8 open.
        let mut gap = function(vec![
            alloca(0, 8),
            gep(1, 0, 4),
            store_ty(0, IrType::I64),
            store(0),
            store_ty(1, IrType::I16),
        ]);
        assert_eq!(eliminate_dead_stores(&mut gap), 0);
        // fill; 4 bytes at 0, 4 at 2 and 4 at 4: overlapping, but complete.
        let mut overlapping = function(vec![
            alloca(0, 8),
            gep(1, 0, 4),
            gep(2, 0, 2),
            store_ty(0, IrType::I64),
            store(0),
            store(2),
            store(1),
        ]);
        assert_eq!(eliminate_dead_stores(&mut overlapping), 1);
    }
}

#[cfg(test)]
mod volatile_object_tests {
    //! A C volatile object is written where the program writes it: no
    //! volatile store hides one of its stores, what its initializer wrote
    //! twice is written once, and [`close_volatile_objects`] leaves each of
    //! its accesses a volatile instruction.
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Terminator};

    fn alloca(dest: u32, size: usize, volatile: bool, semantic_volatile: bool) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::I64,
            size,
            align: 4,
            volatile,
            semantic_volatile,
        }
    }
    fn gep(dest: u32, base: u32, offset: Operand) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset,
            ty: IrType::I32,
        }
    }
    fn at(n: i64) -> Operand {
        Operand::Const(IrConst::I64(n))
    }
    fn store(ptr: u32, val: Operand, ty: IrType, volatile: bool) -> Instruction {
        Instruction::Store {
            volatile,
            val,
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
        }
    }
    fn int(n: i32) -> Operand {
        Operand::Const(IrConst::I32(n))
    }
    fn load(dest: u32, ptr: u32, volatile: bool) -> Instruction {
        Instruction::Load {
            volatile,
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
        }
    }
    fn function(instructions: Vec<Instruction>, result: Option<u32>) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        f.next_value_id = 100;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(result.map(|v| Operand::Value(Value(v)))),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        f
    }
    /// `(value stored, volatile)` of the stores left, in order.
    fn stores(f: &IrFunction) -> Vec<(i64, bool)> {
        f.blocks[0]
            .instructions
            .iter()
            .filter_map(|inst| match inst {
                Instruction::Store {
                    val: Operand::Const(k),
                    volatile,
                    ..
                } => Some((k.to_i64().expect("an integer"), *volatile)),
                _ => None,
            })
            .collect()
    }

    /// `volatile struct s { int a, b; } v = {1, 2}; v.a = 3; return v.b;` as
    /// the lowering writes it: the fill, a plain store for each member, and
    /// the volatile accesses the program makes.  `semantic` is whether the
    /// alloca is a C volatile object or only kept in memory.
    fn initialized_and_assigned(semantic: bool) -> IrFunction {
        function(
            vec![
                alloca(0, 8, true, semantic),
                store(0, at(0), IrType::I64, false),
                store(0, int(1), IrType::I32, false),
                gep(1, 0, at(4)),
                store(1, int(2), IrType::I32, false),
                store(0, int(3), IrType::I32, true),
                load(2, 1, true),
            ],
            Some(2),
        )
    }

    #[test]
    fn a_volatile_store_hides_nothing_of_a_volatile_object() {
        // The fill is dead under the two member stores; the 1 is not dead
        // under the volatile 3.
        let mut f = initialized_and_assigned(true);
        assert_eq!(eliminate_dead_stores(&mut f), 1);
        assert_eq!(stores(&f), [(1, false), (2, false), (3, true)]);

        // The control: memory that is only kept out of a register is
        // overwritten by a volatile store like any other.
        let mut f = initialized_and_assigned(false);
        assert_eq!(eliminate_dead_stores(&mut f), 2);
        assert_eq!(stores(&f), [(2, false), (3, true)]);

        // `volatile int x = 1; x = 2;` with the initializer a plain store.
        let mut f = function(
            vec![
                alloca(0, 4, true, true),
                store(0, int(1), IrType::I32, false),
                store(0, int(2), IrType::I32, true),
            ],
            None,
        );
        assert_eq!(eliminate_dead_stores(&mut f), 0);
        assert_eq!(stores(&f), [(1, false), (2, true)]);
    }

    #[test]
    fn a_volatile_access_ends_what_later_stores_of_the_object_hide() {
        // Two plain stores of one cell are one initializer's, and the later
        // stands ...
        let mut f = function(
            vec![
                alloca(0, 4, true, true),
                store(0, int(1), IrType::I32, false),
                store(0, int(2), IrType::I32, false),
            ],
            None,
        );
        assert_eq!(eliminate_dead_stores(&mut f), 1);
        assert_eq!(stores(&f), [(2, false)]);

        // ... unless the program wrote the object between them, or read it:
        // then the first was there to be seen.
        for between in [store(0, int(5), IrType::I32, true), load(2, 0, true)] {
            let mut f = function(
                vec![
                    alloca(0, 4, true, true),
                    store(0, int(1), IrType::I32, false),
                    between,
                    store(0, int(2), IrType::I32, false),
                ],
                None,
            );
            assert_eq!(eliminate_dead_stores(&mut f), 0);
        }

        // A volatile store of other bytes of the object ends nothing for
        // these: the fill is dead under the two member stores around it.
        let mut f = function(
            vec![
                alloca(0, 12, true, true),
                store(0, at(0), IrType::I64, false),
                store(0, int(1), IrType::I32, false),
                gep(1, 0, at(4)),
                gep(3, 0, at(8)),
                store(3, int(9), IrType::I32, true),
                store(1, int(2), IrType::I32, false),
            ],
            None,
        );
        assert_eq!(eliminate_dead_stores(&mut f), 1);
        assert_eq!(stores(&f), [(1, false), (9, true), (2, false)]);

        // One at an offset nobody knows may name any of them.
        let mut f = function(
            vec![
                alloca(0, 8, true, true),
                Instruction::ParamRef {
                    dest: Value(9),
                    param_idx: 0,
                    ty: IrType::I64,
                },
                store(0, at(0), IrType::I64, false),
                store(0, int(1), IrType::I32, false),
                gep(1, 0, at(4)),
                gep(3, 0, Operand::Value(Value(9))),
                store(3, int(9), IrType::I32, true),
                store(1, int(2), IrType::I32, false),
            ],
            None,
        );
        assert_eq!(eliminate_dead_stores(&mut f), 0);
    }

    #[test]
    fn closing_merges_the_initializer_and_marks_every_access() {
        let mut f = initialized_and_assigned(true);
        // The fill goes, the two member stores are marked.
        assert_eq!(close_volatile_objects(&mut f, true), 3);
        assert_eq!(stores(&f), [(1, true), (2, true), (3, true)]);
        // Closed, the function is one DSE has nothing to say about ...
        assert_eq!(eliminate_dead_stores(&mut f), 0);
        // ... and closing it again changes nothing.
        let before = format!("{f:?}");
        assert_eq!(close_volatile_objects(&mut f, true), 0);
        assert_eq!(format!("{f:?}"), before);

        // Without optimization nothing is merged: the fill is written too.
        let mut f = initialized_and_assigned(true);
        assert_eq!(close_volatile_objects(&mut f, false), 3);
        assert_eq!(stores(&f), [(0, true), (1, true), (2, true), (3, true)]);
    }

    #[test]
    fn closing_reaches_an_access_through_any_address_in_the_object() {
        let mut f = function(
            vec![
                alloca(0, 16, true, true),
                Instruction::ParamRef {
                    dest: Value(9),
                    param_idx: 0,
                    ty: IrType::I64,
                },
                // A constant offset on a constant offset, a variable
                // offset, and a copy of an element address.
                gep(1, 0, at(4)),
                gep(2, 1, at(4)),
                gep(3, 0, Operand::Value(Value(9))),
                Instruction::Copy {
                    dest: Value(4),
                    src: Operand::Value(Value(2)),
                },
                store(2, int(1), IrType::I32, false),
                store(3, int(2), IrType::I32, false),
                load(5, 4, false),
                load(6, 3, false),
            ],
            Some(5),
        );
        assert_eq!(close_volatile_objects(&mut f, true), 4);
        assert!(f.blocks[0].instructions.iter().all(|inst| !matches!(
            inst,
            Instruction::Load {
                volatile: false,
                ..
            } | Instruction::Store {
                volatile: false,
                ..
            }
        )));
    }

    #[test]
    fn closing_leaves_every_other_object_as_it_is() {
        // An ordinary local with a dead store, memory that is only kept out
        // of a register, and a pointer parameter: not one instruction of
        // them is removed or marked, with a volatile object in the function
        // or without one.
        let others = |with_volatile_object: bool| {
            let mut insts = vec![
                alloca(0, 4, false, false),
                alloca(1, 4, true, false),
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                store(0, int(1), IrType::I32, false),
                store(0, int(2), IrType::I32, false),
                store(1, int(3), IrType::I32, false),
                store(1, int(4), IrType::I32, false),
                store(2, int(5), IrType::I32, false),
                load(5, 0, false),
                load(6, 1, false),
                load(7, 2, false),
            ];
            if with_volatile_object {
                insts.push(alloca(8, 4, true, true));
                insts.push(store(8, int(6), IrType::I32, false));
            }
            function(insts, Some(5))
        };
        let mut f = others(false);
        let before = format!("{f:?}");
        assert_eq!(close_volatile_objects(&mut f, true), 0);
        assert_eq!(format!("{f:?}"), before);

        let mut f = others(true);
        assert_eq!(close_volatile_objects(&mut f, true), 1);
        assert_eq!(
            stores(&f),
            [
                (1, false),
                (2, false),
                (3, false),
                (4, false),
                (5, false),
                (6, true)
            ]
        );
        assert!(
            f.blocks[0]
                .instructions
                .iter()
                .all(|inst| !matches!(inst, Instruction::Load { volatile: true, .. }))
        );
    }
}
