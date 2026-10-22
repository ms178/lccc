//! Collapse a 16-byte aggregate round trip through the integer unit.
//!
//! LCCC has no IR vector type: a `__m128d` / `__m128i` / `__m128` value is a
//! 16-byte aggregate, and an aggregate *value* that crosses a call boundary in
//! a register pair travels as `I128` (see `src/ir/lowering/aggregate_abi.rs`,
//! "packed as I128 in the IR").  When such a function is then inlined -- and
//! every SSE intrinsic wrapper in `include/xmmintrin.h` /
//! `include/emmintrin.h` is `__always_inline__` -- the packing and the
//! unpacking it was written for both survive, and the aggregate round trip
//! becomes eight integer instructions and two GPRs:
//!
//! ```text
//!   %lo  = load %p            (I64)
//!   %hp  = gep %p, 8
//!   %hi  = load %hp           (I64)
//!   %clo = cast %lo -> I128
//!   %chi = cast %hi -> I128
//!   %sh  = shl %chi, 64
//!   %agg = or  %sh, %clo
//!   %dlo = cast shr %agg      -> I64 ; store %q
//!   %shr = lshr %agg, 64
//!   %dhi = cast shr %shr      -> I64 ; store %q + 8
//! ```
//!
//! The net effect of that sequence is a 16-byte copy from `%p` to `%q`, so the
//! whole thing becomes one memory-backed `Loaddqu` intrinsic.  This is the difference between the stock
//! lowering of `_mm_loadu_pd` and what GCC and Clang emit, and it is why every
//! row of `tests/regression/check_sse2_codegen_matrix.py` used to cost nine
//! more instructions than both oracles: the copy, not the SSE op, was the
//! cost.
//!
//! Correctness is fail-closed and the match is deliberately narrow:
//!
//! * every instruction of the pattern must be in ONE basic block, and each
//!   intermediate value must have exactly one use in the whole function (so
//!   deleting it cannot strand a reader);
//! * the two loads must address `p` and `p + 8` (same base, provable 8-byte
//!   step) and the two stores must address `q` and `q + 8`, so the pattern
//!   really is a whole 16-byte object and never a lane-pair subset;
//! * no memory-clobbering instruction may sit between the loads and the
//!   stores: the rewrite moves the read of `p` later (to the store site), and
//!   a store, call, memcpy, atomic or fence in between could change what `p`
//!   reads;
//! * if `p` and `q` are the same base, the objects must not overlap by less
//!   than 16 bytes. A scalar load/store sequence can legally overlap; it is
//!   not a memcpy call and no memcpy non-overlap contract can be assumed;
//! * low-half widening must be unsigned: sign extension would corrupt the
//!   packed high half whenever the low half's top bit is set;
//! * multi-definition value ids (post-phi-coalescing webs) are refused
//!   outright, the same guard `load_forward` and `gvn` use: their content is
//!   position dependent, so a single-use count is not a proof.
//!
//! Pass name for `CCC_DISABLE_PASSES`: "aggcopy16".
//!
//! WHY A PASS AND NOT A BACKEND PEEPHOLE: the pack/unpack pair is separated
//! from the copy it implements by the call/return structure, and it appears in
//! four different backend paths (SSE return, SSE argument, `va_arg`, union
//! member forwarding).  Rewriting it once in the IR removes it for every
//! target. A memory-backed `Loaddqu` exposes the copy to the existing vector
//! forwarding machinery, which can fold it into a consumer's memory operand.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::provenance::{self, ChainStep, RootLeaf};
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

/// Bytes covered by the pattern: two 8-byte halves.
const COPY_BYTES: usize = 16;
const HALF: i64 = 8;

/// Immutable facts are indexed once per search, not rediscovered by scanning
/// every block at each provenance hop. A rewritten function gets fresh facts.
struct Facts<'a> {
    defs: FxHashMap<u32, &'a Instruction>,
    multi_def: FxHashSet<u32>,
    uses: FxHashMap<u32, usize>,
}

impl<'a> Facts<'a> {
    fn new(func: &'a IrFunction) -> Self {
        let mut facts = Self {
            defs: FxHashMap::default(),
            multi_def: FxHashSet::default(),
            uses: FxHashMap::default(),
        };
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    if facts.defs.insert(dest.0, inst).is_some() {
                        facts.multi_def.insert(dest.0);
                    }
                }
                inst.for_each_used_value(|id| *facts.uses.entry(id).or_default() += 1);
            }
            block.terminator.for_each_used_value(|id| {
                *facts.uses.entry(id).or_default() += 1;
            });
        }
        facts
    }

    /// Resolve an immutable address chain. Unknown offsets keep their object
    /// root but cannot establish a byte distance. Cycles, overflow, and mutable
    /// copy webs fail closed rather than presenting a fabricated root.
    fn root_of(&self, v: Value) -> Option<(u32, Option<i64>)> {
        let (mut cur, mut offset) = (v.0, Some(0i64));
        for _ in 0..32 {
            if self.multi_def.contains(&cur) {
                return None;
            }
            let Some(inst) = self.defs.get(&cur) else {
                return Some((cur, offset));
            };
            if let Instruction::GetElementPtr {
                base, offset: step, ..
            } = inst
            {
                offset = match (offset, step) {
                    (Some(sum), Operand::Const(IrConst::I64(delta))) => {
                        Some(sum.checked_add(*delta)?)
                    }
                    _ => None,
                };
                cur = base.0;
            } else {
                match provenance::chain_step(inst) {
                    ChainStep::Follow(base) => cur = base.0,
                    ChainStep::Stop => return Some((cur, offset)),
                }
            }
        }
        None
    }

    /// Prove a relative displacement even when the shared base is itself a
    /// dynamic GEP. Flattening both offsets to their object root would lose
    /// this common case (`p + i`, then `(p + i) + 8`).
    fn displacement_to(&self, mut a: Value, b: Value) -> Option<i64> {
        let mut offset = 0i64;
        for _ in 0..32 {
            if a == b {
                return Some(offset);
            }
            let inst = self.defs.get(&a.0)?;
            if let Instruction::GetElementPtr {
                base,
                offset: Operand::Const(IrConst::I64(delta)),
                ..
            } = inst
            {
                offset = offset.checked_add(*delta)?;
                a = *base;
            } else if matches!(inst, Instruction::GetElementPtr { .. }) {
                return None;
            } else {
                match provenance::chain_step(inst) {
                    ChainStep::Follow(base) => a = base,
                    ChainStep::Stop => return None,
                }
            }
        }
        None
    }

    fn offset_from(&self, a: Value, b: Value) -> Option<i64> {
        // Validate both complete chains before accepting a shared interior
        // node: that node may be an immutable GEP of a redefined address web.
        let (ra, oa) = self.root_of(a)?;
        let (rb, ob) = self.root_of(b)?;
        if let Some(delta) = self.displacement_to(a, b) {
            return Some(delta);
        }
        if let Some(delta) = self.displacement_to(b, a) {
            return delta.checked_neg();
        }
        if ra == rb { oa?.checked_sub(ob?) } else { None }
    }

    /// Cross-root disjointness is a typed-leaf property, never an inequality
    /// between SSA ids. Loaded/selected/integer-laundered addresses can point
    /// into this frame; incoming pointer parameters and global objects cannot.
    fn distinct_objects(&self, a: Value, b: Value) -> bool {
        let (Some((ra, _)), Some((rb, _))) = (self.root_of(a), self.root_of(b)) else {
            return false;
        };
        if ra == rb {
            return false;
        }
        let leaf = |id| self.defs.get(&id).map(|inst| provenance::root_leaf(inst));
        matches!(
            (leaf(ra), leaf(rb)),
            (
                Some(RootLeaf::Alloca(_)),
                Some(RootLeaf::Alloca(_) | RootLeaf::Param(_) | RootLeaf::Global(_))
            ) | (
                Some(RootLeaf::Param(_) | RootLeaf::Global(_)),
                Some(RootLeaf::Alloca(_))
            )
        )
    }

    fn provably_disjoint(&self, src: Value, dst: Value) -> bool {
        self.offset_from(dst, src)
            .is_some_and(|d| d.unsigned_abs() >= COPY_BYTES as u64)
            || self.distinct_objects(src, dst)
    }
}

fn is_cst(v: &Operand, want: i64) -> bool {
    match v {
        Operand::Const(IrConst::I64(x)) => *x == want,
        Operand::Const(IrConst::I32(x)) => i64::from(*x) == want,
        _ => false,
    }
}

fn clobbers(inst: &Instruction) -> bool {
    inst.may_write_memory()
        || matches!(
            inst,
            Instruction::Fence { .. }
                | Instruction::AtomicLoad { .. }
                | Instruction::StackRestore { .. }
                | Instruction::Load { volatile: true, .. }
        )
}

/// Immutable facts about one candidate, resolved before any mutation.
struct Candidate {
    block: usize,
    drop: Vec<usize>,
    insert_at: usize,
    dest_ptr: Value,
    src_ptr: Value,
    /// (first, last) instruction indexes the clobber proof covers.
    span: (usize, usize),
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    // Cheap prescan: without a single `or` at I128 there is no packed pair to
    // find, and most functions have none. Skip the facts build entirely.
    let any_packed_or = func.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
        matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::Or,
                ty: IrType::I128,
                ..
            }
        )
    });
    if !any_packed_or {
        return 0;
    }
    let mut total = 0;
    // Each successful rewrite removes at least five instructions, so the loop
    // terminates; the bound is a belt-and-braces guard for a malformed IR.
    // One sweep collects every non-overlapping candidate under a single
    // `Facts` build and applies them per block, latest first, so earlier
    // indexes stay valid. A TU dense in intrinsic wrappers otherwise pays a
    // full-function facts rebuild plus full block rescans per rewrite --
    // O(rewrites x function size) for what is one linear analysis.
    for _ in 0..4096 {
        let candidates = find_all(func);
        if candidates.is_empty() {
            break;
        }
        total += candidates.len();
        for c in &candidates {
            apply(func, c);
        }
        if total >= 4096 {
            break;
        }
    }
    total
}

fn find_all(func: &IrFunction) -> Vec<Candidate> {
    let facts = Facts::new(func);
    let single_use = |v: u32| facts.uses.get(&v).copied().unwrap_or(0) == 1;
    let mut found: Vec<Candidate> = Vec::new();

    for (bi, block) in func.blocks.iter().enumerate() {
        // Same prescan per block: the def index below is only worth building
        // for a block that can hold a candidate at all.
        if !block.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::Or,
                    ty: IrType::I128,
                    ..
                }
            )
        }) {
            continue;
        }
        // Dest -> instruction index within this block, built once. The half
        // matchers resolve their chains through it instead of rescanning the
        // block per candidate (which made `cast_of_load` quadratic in block
        // size on intrinsic-dense code).
        let mut def_at: FxHashMap<u32, usize> = FxHashMap::default();
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                def_at.insert(dest.0, ii);
            }
        }
        for (oi, inst) in block.instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest: agg,
                op: IrBinOp::Or,
                lhs,
                rhs,
                ty: IrType::I128,
            } = inst
            else {
                continue;
            };
            // The Or must be `shl(hi) | lo`, possibly with the operands swapped.
            let mut sides = [(lhs, rhs), (rhs, lhs)];
            let mut matched = None;
            for (a, b) in sides.iter_mut() {
                let (Some(hi_src), Some(lo_src)) =
                    (shl_source(block, &def_at, a), lo_source(block, &def_at, b))
                else {
                    continue;
                };
                matched = Some((hi_src, lo_src));
                break;
            }
            let Some((hi_src, lo_src)) = matched else {
                continue;
            };

            // Exactly two uses of `agg`: the low truncation and the 64-shift.
            let mut trunc = None;
            let mut shifted = None;
            for (ii, other) in block.instructions.iter().enumerate() {
                match other {
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(s),
                        from_ty: IrType::I128,
                        to_ty: IrType::I64,
                    } if s.0 == agg.0 => trunc = Some((ii, dest.0)),
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::LShr,
                        lhs: Operand::Value(s),
                        rhs,
                        ty: IrType::I128,
                    } if s.0 == agg.0 && is_cst(rhs, 64) => shifted = Some((ii, dest.0)),
                    _ => {}
                }
            }
            let (Some((trunc_i, trunc_dest)), Some((shift_i, shift_dest))) = (trunc, shifted)
            else {
                continue;
            };
            if facts.uses.get(&agg.0).copied().unwrap_or(0) != 2 {
                continue;
            }

            // The two stores, one for each half.
            let mut store_lo = None;
            let mut store_hi = None;
            for (ii, other) in block.instructions.iter().enumerate() {
                let Instruction::Store {
                    val: Operand::Value(v),
                    ptr,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                } = other
                else {
                    continue;
                };
                if v.0 == trunc_dest && ii > trunc_i {
                    store_lo = Some((ii, *ptr));
                }
            }
            // The high half is `cast (lshr agg, 64)`; find that cast then its store.
            let mut hi_cast = None;
            for (ii, other) in block.instructions.iter().enumerate() {
                if let Instruction::Cast {
                    dest,
                    src: Operand::Value(s),
                    from_ty: IrType::I128,
                    to_ty: IrType::I64,
                } = other
                {
                    if s.0 == shift_dest && ii > shift_i {
                        hi_cast = Some((ii, dest.0));
                    }
                }
            }
            let Some((hi_cast_i, hi_cast_dest)) = hi_cast else {
                continue;
            };
            for (ii, other) in block.instructions.iter().enumerate() {
                let Instruction::Store {
                    val: Operand::Value(v),
                    ptr,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                } = other
                else {
                    continue;
                };
                if v.0 == hi_cast_dest && ii > hi_cast_i {
                    store_hi = Some((ii, *ptr));
                }
            }
            let (Some((store_lo_i, q_lo)), Some((store_hi_i, q_hi))) = (store_lo, store_hi) else {
                continue;
            };

            // Every intermediate must be single-use, single-definition, and
            // the halves must be exactly 8 bytes apart on both sides.
            let mut hops = vec![
                lo_src.cast_dest,
                hi_src.cast_dest,
                hi_src.shift_dest,
                agg.0,
                trunc_dest,
                shift_dest,
                hi_cast_dest,
            ];
            hops.extend(lo_src.load_dest);
            hops.extend(hi_src.load_dest);
            if hops.iter().any(|id| facts.multi_def.contains(id)) {
                continue;
            }
            if !single_use(shift_dest)
                || !lo_src.load_dest.is_some_and(single_use)
                || !hi_src.load_dest.is_some_and(single_use)
                || !single_use(hi_src.shift_dest)
                || !single_use(trunc_dest)
                || !single_use(hi_cast_dest)
                || !single_use(lo_src.cast_dest)
                || !single_use(hi_src.cast_dest)
            {
                continue;
            }

            // The two halves must be exactly 8 bytes apart on both sides.
            if facts.offset_from(hi_src.ptr, lo_src.ptr) != Some(HALF)
                || facts.offset_from(q_hi, q_lo) != Some(HALF)
            {
                continue;
            }
            if !facts.provably_disjoint(lo_src.ptr, q_lo) {
                continue;
            }
            // Both scalar stores move to the later store site. In addition
            // to protecting the delayed source read, protect observers of the
            // earlier destination write. Unknown effects are barriers.
            let first = lo_src.load_index.min(hi_src.load_index);
            let first_store = store_lo_i.min(store_hi_i);
            let last = store_lo_i.max(store_hi_i);
            let mut blocked = false;
            for (ii, inst) in block
                .instructions
                .iter()
                .enumerate()
                .take(last + 1)
                .skip(first)
            {
                if ii == store_lo_i || ii == store_hi_i {
                    continue;
                }
                if clobbers(inst) {
                    if let Instruction::Store {
                        ptr,
                        volatile: false,
                        seg_override: AddressSpace::Default,
                        ..
                    } = inst
                    {
                        if facts.distinct_objects(*ptr, lo_src.ptr)
                            && facts.distinct_objects(*ptr, q_lo)
                        {
                            continue;
                        }
                    }
                    blocked = true;
                    break;
                }
                if ii > first_store {
                    let observes_destination = match inst {
                        Instruction::Load { ptr, .. } => !facts.distinct_objects(*ptr, q_lo),
                        Instruction::Intrinsic { op, .. } => op.reads_pointer_arg(),
                        _ => false,
                    };
                    if observes_destination {
                        blocked = true;
                        break;
                    }
                }
            }
            if blocked {
                continue;
            }

            let mut drop = vec![
                lo_src.load_index,
                hi_src.load_index,
                lo_src.cast_index,
                hi_src.cast_index,
                hi_src.shift_index,
                oi,
                trunc_i,
                shift_i,
                hi_cast_i,
                store_lo_i,
                store_hi_i,
            ];
            drop.sort_unstable();
            drop.dedup();
            // Two candidates in one block may only be batched when their
            // pattern spans are disjoint: each was validated against an IR
            // that still contains the other, so an overlap would invalidate
            // the clobber-window proof. Overlapping candidates are left for
            // the next sweep, which revalidates them from scratch.
            let span = (first, last);
            let overlaps = found
                .iter()
                .filter(|c| c.block == bi)
                .any(|c| c.span.0 <= span.1 && span.0 <= c.span.1);
            if !overlaps {
                found.push(Candidate {
                    block: bi,
                    drop,
                    insert_at: last,
                    dest_ptr: q_lo,
                    src_ptr: lo_src.ptr,
                    span,
                });
            }
        }
    }
    // Latest insertion first within each block, so applying one candidate
    // never shifts the indexes of an earlier one in the same block.
    found.sort_by(|a, b| a.block.cmp(&b.block).then(b.insert_at.cmp(&a.insert_at)));
    found
}

/// The load side of one half: the `cast` of a load into `I128`.
#[derive(Clone)]
struct HalfLoad {
    ptr: Value,
    load_dest: Option<u32>,
    load_index: usize,
    cast_index: usize,
    cast_dest: u32,
    shift_index: usize,
    shift_dest: u32,
}

/// `shl (cast %hi -> i128), 64`. The high half accepts a signed widening:
/// every sign-extension bit is shifted out by the `<< 64`.
fn shl_source(
    block: &crate::ir::reexports::BasicBlock,
    def_at: &FxHashMap<u32, usize>,
    op: &Operand,
) -> Option<HalfLoad> {
    let Operand::Value(sh) = op else { return None };
    let si = *def_at.get(&sh.0)?;
    let Instruction::BinOp {
        dest,
        op: IrBinOp::Shl,
        lhs,
        rhs,
        ty: IrType::I128,
    } = &block.instructions[si]
    else {
        return None;
    };
    if dest.0 != sh.0 || !is_cst(rhs, 64) {
        return None;
    }
    let (ci, cd, ptr, ld, li) = cast_of_load(block, def_at, lhs)?;
    Some(HalfLoad {
        ptr,
        load_dest: ld,
        load_index: li,
        cast_index: ci,
        cast_dest: cd,
        shift_index: si,
        shift_dest: dest.0,
    })
}

/// `cast (load %lo) -> i128`, unsigned only: a signed widening would fill the
/// packed high half with ones whenever the low half's top bit is set.
fn lo_source(
    block: &crate::ir::reexports::BasicBlock,
    def_at: &FxHashMap<u32, usize>,
    op: &Operand,
) -> Option<HalfLoad> {
    let (ci, cd, ptr, ld, li) = cast_of_load(block, def_at, op)?;
    if !matches!(
        block.instructions[ci],
        Instruction::Cast {
            from_ty: IrType::U64,
            ..
        }
    ) {
        return None;
    }
    Some(HalfLoad {
        ptr,
        load_dest: ld,
        load_index: li,
        cast_index: ci,
        cast_dest: cd,
        shift_index: usize::MAX,
        shift_dest: 0,
    })
}

/// Resolve `cast (load %p) -> i128` to (cast index, cast dest, ptr, load dest,
/// load index) through the block's def index.
fn cast_of_load(
    block: &crate::ir::reexports::BasicBlock,
    def_at: &FxHashMap<u32, usize>,
    op: &Operand,
) -> Option<(usize, u32, Value, Option<u32>, usize)> {
    let Operand::Value(cv) = op else { return None };
    let ci = *def_at.get(&cv.0)?;
    let Instruction::Cast {
        dest,
        src: Operand::Value(s),
        from_ty,
        to_ty: IrType::I128,
    } = &block.instructions[ci]
    else {
        return None;
    };
    if dest.0 != cv.0 || !matches!(from_ty, IrType::I64 | IrType::U64) {
        return None;
    }
    let li = *def_at.get(&s.0)?;
    let Instruction::Load {
        dest: ld,
        ptr,
        ty: IrType::I64,
        seg_override: AddressSpace::Default,
        volatile: false,
    } = &block.instructions[li]
    else {
        return None;
    };
    (ld.0 == s.0).then_some((ci, cv.0, *ptr, Some(ld.0), li))
}

fn apply(func: &mut IrFunction, c: &Candidate) {
    // The replacement is the *intrinsic* 128-bit copy pair, not a `Memcpy`.
    // `Loaddqu`/`Storedqu` are what `vector_temp_promotion::fuse_vector_loads`
    // recognises as a forwardable vector producer and consumer, so the copy
    // this pattern implemented then disappears into the consuming SIMD
    // operation's memory operand instead of being emitted at all:
    //
    //   Loaddqu  %h <- %p          ==>  AddPd128 reads %p directly
    //   AddPd128 %r <- %h1, %h2         (no %h1/%h2 slot traffic)
    //   Storedqu %q <- %r          ==>  the result is stored to %q directly
    //
    // A `Memcpy` here would be correct but opaque to that forwarding.
    let result = Value(func.next_value_id);
    func.next_value_id += 1;
    let block = &mut func.blocks[c.block];
    let mut rewritten = Vec::with_capacity(block.instructions.len());
    for (ii, inst) in block.instructions.iter().enumerate() {
        if ii == c.insert_at {
            rewritten.push(Instruction::Intrinsic {
                dest: Some(result),
                op: crate::ir::intrinsics::IntrinsicOp::Loaddqu,
                dest_ptr: Some(c.dest_ptr),
                args: vec![Operand::Value(c.src_ptr)],
            });
        }
        if c.drop.binary_search(&ii).is_ok() {
            continue;
        }
        rewritten.push(inst.clone());
    }
    block.instructions = rewritten;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType as T;
    use crate::ir::reexports::{
        BasicBlock, BlockId, Instruction, IrConst, Operand, Terminator, Value,
    };

    fn func_with(insts: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), T::Void, Vec::new(), false);
        f.next_value_id = 100;
        let mut insts = insts;
        insts.insert(
            0,
            Instruction::ParamRef {
                dest: Value(2),
                param_idx: 0,
                ty: IrType::Ptr,
            },
        );
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        }];
        f
    }

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn cast(dest: u32, src: u32, from: IrType, to: IrType) -> Instruction {
        Instruction::Cast {
            dest: Value(dest),
            src: Operand::Value(Value(src)),
            from_ty: from,
            to_ty: to,
        }
    }

    fn store(val: u32, ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn bin(op: IrBinOp, dest: u32, lhs: u32, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs: Operand::Value(Value(lhs)),
            rhs,
            ty,
        }
    }

    fn gep(dest: u32, base: u32, off: i64) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Const(IrConst::I64(off)),
            ty: IrType::I8,
        }
    }

    /// The exact shape `_mm_loadu_pd` + a 16-byte store lowers to today.
    fn roundtrip(src: u32, dst: u32) -> Vec<Instruction> {
        roundtrip_or_order(src, dst, false)
    }

    fn roundtrip_or_order(src: u32, dst: u32, swap_or: bool) -> Vec<Instruction> {
        vec![
            // The destination of an aggregate copy is a stack home; the
            // disjointness proof keys on that.
            Instruction::Alloca {
                dest: Value(dst),
                ty: IrType::Ptr,
                size: 16,
                align: 16,
                volatile: false,
                semantic_volatile: false,
            },
            load(10, src),
            gep(11, src, 8),
            load(12, 11),
            cast(13, 12, IrType::U64, IrType::I128),
            cast(14, 10, IrType::U64, IrType::I128),
            bin(
                IrBinOp::Shl,
                15,
                13,
                Operand::Const(IrConst::I64(64)),
                IrType::I128,
            ),
            if swap_or {
                bin(IrBinOp::Or, 16, 14, Operand::Value(Value(15)), IrType::I128)
            } else {
                bin(IrBinOp::Or, 16, 15, Operand::Value(Value(14)), IrType::I128)
            },
            cast(17, 16, IrType::I128, IrType::I64),
            store(17, dst),
            bin(
                IrBinOp::LShr,
                18,
                16,
                Operand::Const(IrConst::I64(64)),
                IrType::I128,
            ),
            cast(19, 18, IrType::I128, IrType::I64),
            gep(20, dst, 8),
            store(19, 20),
        ]
    }

    #[test]
    fn a_sixteen_byte_round_trip_becomes_one_128_bit_load() {
        let mut f = func_with(roundtrip(2, 3));
        assert_eq!(run(&mut f), 1, "the canonical shape must be recognised");
        let insts = &f.blocks[0].instructions;
        // The rewrite is the intrinsic 128-bit copy pair (`Loaddqu`), which the
        // later vector-load forwarding turns into the consumer's memory
        // operand instead of emitting the copy at all.
        let loads: Vec<&Instruction> = insts
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::Intrinsic {
                        op: crate::ir::intrinsics::IntrinsicOp::Loaddqu,
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(
            loads.len(),
            1,
            "expected one 16-byte Loaddqu, got {insts:?}"
        );
        assert!(
            matches!(
                loads[0],
                Instruction::Intrinsic {
                    dest_ptr: Some(Value(3)),
                    args,
                    ..
                } if matches!(args.as_slice(), [Operand::Value(Value(2))])
            ),
            "the copy must read the source pointer into the destination home"
        );
        // None of the integer-carrier work may survive.
        assert!(
            !insts.iter().any(|i| matches!(
                i,
                Instruction::Store { .. }
                    | Instruction::Load { .. }
                    | Instruction::BinOp {
                        ty: IrType::I128,
                        ..
                    }
            )),
            "the I128 pack/unpack must be gone: {insts:?}"
        );
    }

    #[test]
    fn a_swapped_or_operand_order_is_recognised() {
        let insts = roundtrip_or_order(2, 3, true);
        let mut f = func_with(insts);
        assert_eq!(
            run(&mut f),
            1,
            "or(lo, shl) must collapse as well as or(shl, lo)"
        );
        assert!(
            !f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::BinOp {
                    ty: IrType::I128,
                    ..
                }
            )),
            "swapped operand order must collapse too"
        );
    }

    #[test]
    fn a_store_between_the_halves_keeps_the_round_trip() {
        let mut insts = roundtrip(2, 3);
        insts.insert(8, store(17, 99));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "an intervening clobber must refuse");
    }

    #[test]
    fn a_shared_intermediate_keeps_the_round_trip() {
        let mut insts = roundtrip(2, 3);
        // A second reader of the packed aggregate.
        insts.push(cast(21, 16, IrType::I128, IrType::I64));
        insts.push(store(21, 98));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn overlapping_source_and_destination_keep_the_round_trip() {
        // src = dst + 8: immutable addresses, but overlapping byte ranges.
        let mut insts = roundtrip(4, 3);
        insts.insert(1, gep(4, 3, 8));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "a partial overlap must refuse");
    }

    #[test]
    fn a_non_adjacent_pair_keeps_the_round_trip() {
        let mut insts = roundtrip(2, 3);
        insts[2] = gep(11, 2, 16); // high half 16 bytes away, not 8
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn every_deleted_value_must_have_no_external_reader() {
        for value in [10, 12, 18] {
            let mut insts = roundtrip(2, 3);
            insts.push(Instruction::Copy {
                dest: Value(40),
                src: Operand::Value(Value(value)),
            });
            let mut f = func_with(insts);
            assert_eq!(run(&mut f), 0, "external reader of %{value}");
        }
    }

    #[test]
    fn signed_high_half_extension_is_a_bitwise_pack() {
        // The high half's widening is shifted left by 64, so every
        // sign-extension bit leaves the value: a signed cast there is the
        // same pack, and refusing it would only lose copies.
        let mut insts = roundtrip(2, 3);
        insts[4] = cast(13, 12, IrType::I64, IrType::I128);
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 1, "the << 64 discards the sign extension");
    }

    #[test]
    fn signed_low_half_extension_is_not_a_bitwise_pack() {
        let mut insts = roundtrip(2, 3);
        insts[5] = cast(14, 10, IrType::I64, IrType::I128);
        let mut f = func_with(insts);
        assert_eq!(
            run(&mut f),
            0,
            "a negative low half fills the high half with ones"
        );
    }

    #[test]
    fn a_destination_observer_between_stores_blocks_the_rewrite() {
        let mut insts = roundtrip(2, 3);
        insts.insert(10, load(40, 3));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "the observer must see the first store");
    }

    #[test]
    fn a_memory_backed_simd_observer_blocks_store_relocation() {
        let mut insts = roundtrip(2, 3);
        insts.insert(
            10,
            Instruction::Intrinsic {
                dest: Some(Value(40)),
                op: crate::ir::intrinsics::IntrinsicOp::AddPd128,
                dest_ptr: None,
                args: vec![Operand::Value(Value(3)), Operand::Value(Value(50))],
            },
        );
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn an_intrinsic_write_is_a_memory_barrier() {
        let mut insts = roundtrip(2, 3);
        insts.insert(
            8,
            Instruction::Intrinsic {
                dest: Some(Value(40)),
                op: crate::ir::intrinsics::IntrinsicOp::Loaddqu,
                dest_ptr: Some(Value(2)),
                args: vec![Operand::Value(Value(50))],
            },
        );
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "the source is changed before the stores");
    }

    #[test]
    fn an_unknown_pointer_is_not_disjoint_from_a_stack_home() {
        let mut insts = roundtrip(4, 3);
        insts.insert(
            1,
            Instruction::Copy {
                dest: Value(4),
                src: Operand::Value(Value(3)),
            },
        );
        insts.insert(2, gep(5, 4, 8));
        // Two adjacent halves read from a partly overlapping alias of dst.
        insts[3] = load(10, 5);
        insts[4] = gep(11, 5, 8);
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "Copy must not hide overlap with an alloca");
    }

    #[test]
    fn a_redefined_address_blocks_the_rewrite() {
        let mut insts = roundtrip(2, 3);
        insts.insert(1, gep(2, 50, 0));
        insts.insert(8, gep(2, 50, 16));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "address webs are not immutable SSA roots");
    }

    #[test]
    fn adjacent_halves_of_a_dynamic_address_still_collapse() {
        let mut insts = roundtrip(4, 3);
        insts.insert(
            1,
            Instruction::GetElementPtr {
                dest: Value(4),
                base: Value(2),
                offset: Operand::Value(Value(50)),
                ty: IrType::I8,
            },
        );
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 1, "the shared dynamic offset cancels exactly");
    }

    #[test]
    fn pointer_distance_overflow_fails_closed() {
        let f = func_with(vec![gep(40, 2, i64::MAX), gep(41, 2, i64::MIN)]);
        let facts = Facts::new(&f);
        assert_eq!(facts.offset_from(Value(40), Value(41)), None);
        assert_eq!(facts.offset_from(Value(41), Value(40)), None);
    }

    #[test]
    fn integer_parameters_do_not_prove_stack_disjointness() {
        let mut f = func_with(roundtrip(2, 3));
        if let Instruction::ParamRef { ty, .. } = &mut f.blocks[0].instructions[0] {
            *ty = IrType::U64;
        }
        assert_eq!(run(&mut f), 0, "an integer can name any live address");
    }

    #[test]
    fn volatile_and_atomic_operations_are_relocation_barriers() {
        let mut volatile = load(40, 50);
        if let Instruction::Load { volatile, .. } = &mut volatile {
            *volatile = true;
        }
        let atomic = Instruction::AtomicInc {
            ptr: Operand::Value(Value(50)),
            offset: 0,
            ty: IrType::I64,
            ordering: crate::ir::ops::AtomicOrdering::SeqCst,
        };
        for barrier in [volatile, atomic] {
            let mut insts = roundtrip(2, 3);
            insts.insert(8, barrier);
            let mut f = func_with(insts);
            assert_eq!(run(&mut f), 0);
        }
    }

    #[test]
    fn a_32_bit_source_is_not_matched() {
        let mut insts = roundtrip(2, 3);
        insts[4] = cast(13, 12, IrType::U32, IrType::I128);
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }
}
