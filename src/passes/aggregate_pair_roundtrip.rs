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
//! integer carrier becomes one `Loaddqu` intrinsic. This is the difference
//! between the stock lowering of `_mm_loadu_pd` and what GCC and Clang emit,
//! and it is why every row of `tests/regression/check_sse2_codegen_matrix.py`
//! used to cost nine more instructions than both oracles: the copy, not the SSE
//! op, was the cost.
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
//! * if `p` and `q` share an object, the ranges must be identical or
//!   non-overlapping. Partial overlap is valid for the original staged
//!   load/store sequence, but the replacement's later 16-byte read could see
//!   the first half-store and produce a different result;
//! * multi-definition value ids (post-phi-coalescing webs) are refused
//!   outright, the same guard `load_forward` and `gvn` use: their content is
//!   position dependent, so a single-use count is not a proof.
//!
//! Pass name for `CCC_DISABLE_PASSES`: "aggcopy16".
//!
//! WHY A PASS AND NOT A BACKEND PEEPHOLE: the pack/unpack pair is separated
//! from the copy it implements by the call/return structure, and it appears in
//! four different backend paths (SSE return, SSE argument, `va_arg`, union
//! member forwarding). Rewriting it once in the IR exposes `Loaddqu` to every
//! backend. On x86 the existing vector-temp forwarding recognizes the paired
//! `Loaddqu`/`Storedqu` operations and folds the copy into the consuming SIMD
//! memory operand; other backends lower the same 16-byte copy operation.

use super::module_symbol_facts::ModuleSymbolFacts;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::provenance::{ChainStep, RootLeaf, chain_step, root_leaf};
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

/// Bytes covered by the pattern: two 8-byte halves.
const COPY_BYTES: usize = 16;
const HALF: i64 = 8;

/// The identity of a root object the shared IR provenance rules can prove.
/// Unknown roots are represented by `None`, never by the SSA id of an opaque
/// producer such as a pointer load or a call result.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ObjectRoot {
    Global(String),
    Alloca(u32),
    Param { index: usize, noalias: bool },
}

/// Return the unique definition of `value`. Duplicate definitions make the
/// pointer chain position-dependent, so they cannot support an alias proof.
fn unique_def(func: &IrFunction, value: Value) -> Option<&Instruction> {
    let mut found = None;
    for block in &func.blocks {
        for inst in &block.instructions {
            if inst.dest() == Some(value) {
                if found.is_some() {
                    return None;
                }
                found = Some(inst);
            }
        }
    }
    found
}

/// Trace pointer provenance to a typed object root and, when possible, its
/// constant byte offset. This pass uses the canonical `root_leaf` and
/// `chain_step` rules: GEP/Copy/Ptr-to-Ptr-cast chains are followed, while a
/// load, call, integer-pointer conversion, unknown definition, or cycle fails
/// closed. A dynamic GEP offset retains the root but loses only the offset.
fn root_and_offset(func: &IrFunction, start: Value) -> Option<(ObjectRoot, Option<i64>)> {
    let mut cur = start;
    let mut offset = Some(0i64);
    let mut seen = FxHashSet::default();
    for _ in 0..128 {
        if !seen.insert(cur) {
            return None;
        }
        let inst = unique_def(func, cur)?;
        match root_leaf(inst) {
            RootLeaf::Global(name) => return Some((ObjectRoot::Global(name), offset)),
            RootLeaf::Alloca(id) => return Some((ObjectRoot::Alloca(id), offset)),
            RootLeaf::Param(index) => {
                let noalias = func.params.get(index).is_some_and(|param| param.noalias);
                return Some((ObjectRoot::Param { index, noalias }, offset));
            }
            RootLeaf::NotLeaf => {}
        }
        if let Instruction::GetElementPtr { offset: step, .. } = inst {
            let step = match step {
                Operand::Const(IrConst::I64(bytes)) => Some(*bytes),
                Operand::Const(IrConst::I32(bytes)) => Some(i64::from(*bytes)),
                _ => None,
            };
            offset = match (offset, step) {
                (Some(total), Some(step)) => total.checked_add(step),
                _ => None,
            };
        }
        match chain_step(inst) {
            ChainStep::Follow(next) => cur = next,
            ChainStep::Stop => return None,
        }
    }
    None
}

fn same_root(a: &ObjectRoot, b: &ObjectRoot) -> bool {
    match (a, b) {
        (ObjectRoot::Global(a), ObjectRoot::Global(b)) => a == b,
        (ObjectRoot::Alloca(a), ObjectRoot::Alloca(b)) => a == b,
        (ObjectRoot::Param { index: a, .. }, ObjectRoot::Param { index: b, .. }) => a == b,
        _ => false,
    }
}

/// Object/restrict rules for this copy proof: distinct current-frame stack
/// homes cannot alias; a current-frame alloca cannot alias an incoming
/// parameter; a global may alias an ordinary parameter; and two parameters
/// need a restrict/noalias contract. Distinct global names are disjoint only
/// when module facts prove both are strong private definitions and neither
/// participates in an alias/asm-label redirect. Unknown roots never reach this
/// function because `root_and_offset` returns `None` for them.
fn roots_proven_distinct(a: &ObjectRoot, b: &ObjectRoot, facts: &ModuleSymbolFacts) -> bool {
    match (a, b) {
        (ObjectRoot::Alloca(a), ObjectRoot::Alloca(b)) => a != b,
        (ObjectRoot::Global(a), ObjectRoot::Global(b)) => {
            a != b
                && facts.local_globals.contains(a)
                && facts.local_globals.contains(b)
                && !facts.aliased_names.contains(a)
                && !facts.aliased_names.contains(b)
        }
        (ObjectRoot::Alloca(_), ObjectRoot::Global(_))
        | (ObjectRoot::Global(_), ObjectRoot::Alloca(_))
        | (ObjectRoot::Alloca(_), ObjectRoot::Param { .. })
        | (ObjectRoot::Param { .. }, ObjectRoot::Alloca(_)) => true,
        (ObjectRoot::Global(_), ObjectRoot::Param { noalias, .. })
        | (ObjectRoot::Param { noalias, .. }, ObjectRoot::Global(_)) => *noalias,
        (
            ObjectRoot::Param {
                index: a_index,
                noalias: a_noalias,
            },
            ObjectRoot::Param {
                index: b_index,
                noalias: b_noalias,
            },
        ) => a_index != b_index && (*a_noalias || *b_noalias),
    }
}

/// Follow only provenance-preserving edges from `start` until `target` is
/// reached, accumulating the constant byte delta. This relative proof remains
/// available when both pointers share a dynamic GEP: the dynamic offset is
/// already embodied in the common SSA value and cancels out.
fn offset_to_ancestor(func: &IrFunction, start: Value, target: Value) -> Option<i64> {
    let mut current = start;
    let mut offset = 0i64;
    let mut seen = FxHashSet::default();
    for _ in 0..128 {
        if current == target {
            return Some(offset);
        }
        if !seen.insert(current) {
            return None;
        }
        let inst = unique_def(func, current)?;
        if let Instruction::GetElementPtr { offset: step, .. } = inst {
            let step = match step {
                Operand::Const(IrConst::I64(bytes)) => *bytes,
                Operand::Const(IrConst::I32(bytes)) => i64::from(*bytes),
                _ => return None,
            };
            offset = offset.checked_add(step)?;
        }
        match chain_step(inst) {
            ChainStep::Follow(next) => current = next,
            ChainStep::Stop => return None,
        }
    }
    None
}

/// `a - b` in bytes when it is provable: equal pointers are 0; a pointer
/// directly derived from the other has a relative offset even when the shared
/// base has a dynamic GEP; otherwise two chains rooted at the same object must
/// have known absolute offsets. `None` means "cannot prove" -- callers fail
/// closed.
fn offset_from(func: &IrFunction, a: Value, b: Value) -> Option<i64> {
    if a == b {
        return Some(0);
    }
    if let Some(offset) = offset_to_ancestor(func, a, b) {
        return Some(offset);
    }
    if let Some(offset) = offset_to_ancestor(func, b, a) {
        return offset.checked_neg();
    }
    let (a_root, a_offset) = root_and_offset(func, a)?;
    let (b_root, b_offset) = root_and_offset(func, b)?;
    if !same_root(&a_root, &b_root) {
        return None;
    }
    a_offset?.checked_sub(b_offset?)
}

fn is_cst(v: &Operand, want: i64) -> bool {
    match v {
        Operand::Const(IrConst::I64(x)) => *x == want,
        Operand::Const(IrConst::I32(x)) => i64::from(*x) == want,
        _ => false,
    }
}

/// Memory-writing operations and ordering/observation barriers across which
/// this pass may not move the source read. `may_write_memory` is exhaustive;
/// the extra cases cover operations that order or expose memory without
/// writing bytes (fences, atomic/volatile loads, and stack restoration).
fn memory_motion_barrier(inst: &Instruction) -> bool {
    inst.may_write_memory()
        || matches!(
            inst,
            Instruction::Load { volatile: true, .. }
                | Instruction::AtomicLoad { .. }
                | Instruction::AtomicStore { .. }
                | Instruction::AtomicRmw { .. }
                | Instruction::AtomicInc { .. }
                | Instruction::AtomicCmpxchg { .. }
                | Instruction::Fence { .. }
                | Instruction::StackRestore { .. }
                | Instruction::Intrinsic {
                    op: crate::ir::intrinsics::IntrinsicOp::Lfence
                        | crate::ir::intrinsics::IntrinsicOp::Mfence
                        | crate::ir::intrinsics::IntrinsicOp::Sfence,
                    ..
                }
        )
}

/// Immutable facts about one candidate, resolved before any mutation.
struct Candidate {
    block: usize,
    drop: Vec<usize>,
    insert_at: usize,
    dest_ptr: Value,
    src_ptr: Value,
}

/// The alloca a store destination provably belongs to, when it is a stack
/// home. Unknown or non-stack destinations remain opaque to the clobber scan.
fn store_root(func: &IrFunction, ptr: Value) -> Option<u32> {
    match root_and_offset(func, ptr)?.0 {
        ObjectRoot::Alloca(root) => Some(root),
        ObjectRoot::Global(_) | ObjectRoot::Param { .. } => None,
    }
}

/// Whether a 16-byte copy from `src` to `dst` can be proven not to overlap
/// partially. Same-pointer copies are exact; same-object pointers are safe
/// only at offset zero or at least 16 bytes apart. Distinct typed roots follow
/// the same C object/restrict rules as the vectorizer. Unknown roots (notably
/// loaded/call-returned pointers) fail closed instead of being mistaken for
/// pointers outside the current stack frame.
fn provably_disjoint(func: &IrFunction, src: Value, dst: Value, facts: &ModuleSymbolFacts) -> bool {
    if src == dst {
        return true;
    }
    let Some((src_root, src_offset)) = root_and_offset(func, src) else {
        return false;
    };
    let Some((dst_root, dst_offset)) = root_and_offset(func, dst) else {
        return false;
    };
    if same_root(&src_root, &dst_root) {
        return match (src_offset, dst_offset) {
            (Some(src_offset), Some(dst_offset)) => {
                dst_offset.checked_sub(src_offset).is_some_and(|distance| {
                    distance == 0 || distance.unsigned_abs() >= COPY_BYTES as u64
                })
            }
            _ => false,
        };
    }
    roots_proven_distinct(&src_root, &dst_root, facts)
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    run_with_symbol_facts(func, &ModuleSymbolFacts::default())
}

pub(crate) fn run_with_symbol_facts(func: &mut IrFunction, facts: &ModuleSymbolFacts) -> usize {
    let mut total = 0;
    // Each successful rewrite removes at least five instructions, so the loop
    // terminates; the bound is a belt-and-braces guard for a malformed IR.
    for _ in 0..4096 {
        let Some(c) = find_one(func, facts) else {
            break;
        };
        if super::aggcopy_debug_enabled() {
            eprintln!(
                "aggcopy16: block {} {} -> {} ({} instructions collapsed)",
                c.block,
                c.src_ptr.0,
                c.dest_ptr.0,
                c.drop.len()
            );
        }
        apply(func, &c);
        total += 1;
    }
    total
}

fn find_one(func: &IrFunction, facts: &ModuleSymbolFacts) -> Option<Candidate> {
    let multi_def = super::gvn::find_multi_def_values(func);
    // Use counts over the whole function, so a single-use proof also covers
    // uses in other blocks.
    let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|id| *uses.entry(id).or_default() += 1);
        }
        block
            .terminator
            .for_each_used_value(|id| *uses.entry(id).or_default() += 1);
    }
    let single_use = |v: u32| uses.get(&v).copied().unwrap_or(0) == 1;

    for (bi, block) in func.blocks.iter().enumerate() {
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
                let (Some(hi_src), Some(lo_src)) = (shl_source(block, a), lo_source(block, b))
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
            if uses.get(&agg.0).copied().unwrap_or(0) != 2 {
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
            if hops.iter().any(|id| multi_def.contains(id)) {
                continue;
            }
            if !single_use(hi_src.shift_dest)
                || !single_use(trunc_dest)
                || !single_use(hi_cast_dest)
                || !single_use(lo_src.cast_dest)
                || !single_use(hi_src.cast_dest)
                || !single_use(shift_dest)
                || lo_src.load_dest.is_some_and(|id| !single_use(id))
                || hi_src.load_dest.is_some_and(|id| !single_use(id))
            {
                continue;
            }

            // The two halves must be exactly 8 bytes apart on both sides.
            if offset_from(func, hi_src.ptr, lo_src.ptr) != Some(HALF)
                || offset_from(func, q_hi, q_lo) != Some(HALF)
            {
                continue;
            }
            if !provably_disjoint(func, lo_src.ptr, q_lo, facts) {
                continue;
            }
            let Some((p_root, _)) = root_and_offset(func, lo_src.ptr) else {
                continue;
            };
            let Some((q_root, _)) = root_and_offset(func, q_lo) else {
                continue;
            };

            // The read of `p` moves later (to the store site), so an
            // intervening write, ordering operation, or volatile access is
            // only harmless when we prove it cannot affect the source or
            // destination. Ordinary stores to a distinct stack home are the
            // sole refined case; every other effect fails closed.
            let first = lo_src.load_index.min(hi_src.load_index);
            let last = store_lo_i.max(store_hi_i);
            let replaced = [store_lo_i, store_hi_i];
            let mut blocked = false;
            for (ii, inst) in block.instructions.iter().enumerate() {
                if ii < first || ii > last || replaced.contains(&ii) || !memory_motion_barrier(inst)
                {
                    continue;
                }
                if let Instruction::Store {
                    ptr,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                    ..
                } = inst
                {
                    let Some(write_root) = store_root(func, *ptr) else {
                        blocked = true;
                        break;
                    };
                    let aliases_p =
                        matches!(p_root, ObjectRoot::Alloca(root) if root == write_root);
                    let aliases_q =
                        matches!(q_root, ObjectRoot::Alloca(root) if root == write_root);
                    if aliases_p || aliases_q {
                        blocked = true;
                        break;
                    }
                } else {
                    blocked = true;
                    break;
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
            drop.extend(lo_src.extra);
            drop.extend(hi_src.extra);
            drop.sort_unstable();
            drop.dedup();
            return Some(Candidate {
                block: bi,
                drop,
                insert_at: last,
                dest_ptr: q_lo,
                src_ptr: lo_src.ptr,
            });
        }
    }
    None
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
    extra: Vec<usize>,
}

/// `shl (cast %hi -> i128), 64`
fn shl_source(block: &crate::ir::reexports::BasicBlock, op: &Operand) -> Option<HalfLoad> {
    let Operand::Value(sh) = op else { return None };
    for (si, inst) in block.instructions.iter().enumerate() {
        let Instruction::BinOp {
            dest,
            op: IrBinOp::Shl,
            lhs,
            rhs,
            ty: IrType::I128,
        } = inst
        else {
            continue;
        };
        if dest.0 != sh.0 || !is_cst(rhs, 64) {
            continue;
        }
        let (ci, cd, ptr, ld, li) = cast_of_load(block, lhs)?;
        return Some(HalfLoad {
            ptr,
            load_dest: ld,
            load_index: li,
            cast_index: ci,
            cast_dest: cd,
            shift_index: si,
            shift_dest: dest.0,
            extra: Vec::new(),
        });
    }
    None
}

/// `cast (load %lo) -> i128`
fn lo_source(block: &crate::ir::reexports::BasicBlock, op: &Operand) -> Option<HalfLoad> {
    let (ci, cd, ptr, ld, li) = cast_of_load(block, op)?;
    Some(HalfLoad {
        ptr,
        load_dest: ld,
        load_index: li,
        cast_index: ci,
        cast_dest: cd,
        shift_index: usize::MAX,
        shift_dest: 0,
        extra: Vec::new(),
    })
}

/// Resolve `cast (load %p) -> i128` to (cast index, cast dest, ptr, load dest,
/// load index), plus any `gep` that only feeds the load.
fn cast_of_load(
    block: &crate::ir::reexports::BasicBlock,
    op: &Operand,
) -> Option<(usize, u32, Value, Option<u32>, usize)> {
    let Operand::Value(cv) = op else { return None };
    for (ci, inst) in block.instructions.iter().enumerate() {
        let Instruction::Cast {
            dest,
            src: Operand::Value(s),
            from_ty,
            to_ty: IrType::I128,
        } = inst
        else {
            continue;
        };
        // This is a bit-preserving pack of an I64 load into the low/high
        // half of an I128 carrier. A signed I64 -> I128 cast sign-extends;
        // accepting it (especially on the low half) would let the `or` smear
        // sign bits into the other half, so only the explicit zero-extension
        // form is equivalent to a raw 16-byte copy.
        if dest.0 != cv.0 || *from_ty != IrType::U64 {
            continue;
        }
        for (li, other) in block.instructions.iter().enumerate() {
            let Instruction::Load {
                dest: ld,
                ptr,
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
                volatile: false,
            } = other
            else {
                continue;
            };
            if ld.0 == s.0 {
                return Some((ci, cv.0, *ptr, Some(ld.0), li));
            }
        }
    }
    None
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
    let block = &mut func.blocks[c.block];
    let mut rewritten = Vec::with_capacity(block.instructions.len());
    for (ii, inst) in block.instructions.iter().enumerate() {
        if ii == c.insert_at {
            rewritten.push(Instruction::Intrinsic {
                // `Loaddqu` writes the vector to `dest_ptr`; it has no SSA
                // result (the backend does not define `dest`).
                dest: None,
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
        f.params = vec![crate::ir::reexports::IrParam {
            ty: IrType::Ptr,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: Vec::new(),
            is_f128_sse: false,
            riscv_float_class: None,
        }];
        f.next_value_id = 256;
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        }];
        f
    }

    fn param_ref(dest: u32, param_idx: usize) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx,
            ty: IrType::Ptr,
        }
    }

    fn global_addr(dest: u32, name: &str) -> Instruction {
        Instruction::GlobalAddr {
            dest: Value(dest),
            name: name.to_string(),
        }
    }

    fn alloca(dest: u32, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        }
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

    fn store_const(ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Const(IrConst::I64(0)),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn store_ptr(val: u32, ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn load_ptr(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::Ptr,
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
    /// `src` is a pointer parameter and `dst` is a local 16-byte home.
    fn roundtrip(src: u32, dst: u32) -> Vec<Instruction> {
        roundtrip_or_order(src, dst, false)
    }

    fn global_roundtrip(src: &str, dst: &str) -> Vec<Instruction> {
        let mut insts = vec![global_addr(2, src), global_addr(3, dst)];
        insts.extend(roundtrip_body(2, 3, false));
        insts
    }

    fn roundtrip_or_order(src: u32, dst: u32, swap_or: bool) -> Vec<Instruction> {
        let mut insts = vec![param_ref(src, 0), alloca(dst, 16)];
        insts.extend(roundtrip_body(src, dst, swap_or));
        insts
    }

    /// The pack/unpack sequence without pointer definitions. Tests use it to
    /// build aliasing roots other than an incoming parameter.
    fn roundtrip_body(src: u32, dst: u32, swap_or: bool) -> Vec<Instruction> {
        vec![
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
        let next_value_id = f.next_value_id;
        assert_eq!(run(&mut f), 1, "the canonical shape must be recognised");
        assert_eq!(
            f.next_value_id, next_value_id,
            "the memory copy has no SSA result"
        );
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
                        dest: None,
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

    fn insert_after_high_load(insts: &mut Vec<Instruction>, effect: Instruction) {
        let high_load = insts
            .iter()
            .position(|inst| {
                matches!(
                    inst,
                    Instruction::Load {
                        dest: Value(12),
                        ..
                    }
                )
            })
            .expect("high-half source load");
        insts.insert(high_load + 1, effect);
    }

    #[test]
    fn an_unknown_store_between_the_loads_keeps_the_round_trip() {
        let mut insts = roundtrip(2, 3);
        insert_after_high_load(&mut insts, store_const(99));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "an unproven intervening store must refuse");
    }

    #[test]
    fn a_store_to_a_proven_distinct_stack_home_is_allowed() {
        let mut insts = roundtrip(2, 3);
        insert_after_high_load(&mut insts, alloca(30, 8));
        let at = insts
            .iter()
            .position(|inst| {
                matches!(
                    inst,
                    Instruction::Alloca {
                        dest: Value(30),
                        ..
                    }
                )
            })
            .unwrap()
            + 1;
        insts.insert(at, store_const(30));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 1, "a disjoint local store should not block");
    }

    #[test]
    fn strong_private_globals_can_prove_a_nonoverlapping_copy() {
        let facts = ModuleSymbolFacts {
            aliased_names: FxHashSet::default(),
            local_globals: ["source", "destination"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        let mut f = func_with(global_roundtrip("source", "destination"));
        assert_eq!(
            run_with_symbol_facts(&mut f, &facts),
            1,
            "unredirected strong private globals retain the copy optimization"
        );
    }

    #[test]
    fn a_global_alias_is_not_mistaken_for_a_disjoint_object() {
        let facts = ModuleSymbolFacts {
            aliased_names: ["source", "alias_source"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            // Deliberately include both names: the alias marker, not just the
            // linkage class, must defeat the disjointness proof.
            local_globals: ["source", "alias_source", "destination"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        let mut f = func_with(global_roundtrip("source", "alias_source"));
        assert_eq!(
            run_with_symbol_facts(&mut f, &facts),
            0,
            "a GNU alias can make distinct GlobalAddr names overlap"
        );
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
        // src = base, dst = base + 8. The first destination half overwrites
        // the source's second half, so moving both source loads to the store
        // site would change the value being copied.
        let mut insts = vec![alloca(2, 32), gep(3, 2, 8)];
        insts.extend(roundtrip_body(2, 3, false));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "a partial overlap must refuse");
    }

    #[test]
    fn a_copy_chain_cannot_hide_a_partial_stack_alias() {
        let mut insts = vec![alloca(2, 32), gep(4, 2, 0)];
        insts.push(Instruction::Copy {
            dest: Value(5),
            src: Operand::Value(Value(4)),
        });
        insts.push(gep(3, 2, 8));
        insts.extend(roundtrip_body(5, 3, false));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "Copy must not hide the shared alloca root");
    }

    #[test]
    fn a_pointer_cast_chain_cannot_hide_a_partial_stack_alias() {
        let mut insts = vec![alloca(2, 32), gep(4, 2, 0)];
        insts.push(Instruction::Cast {
            dest: Value(5),
            src: Operand::Value(Value(4)),
            from_ty: IrType::Ptr,
            to_ty: IrType::Ptr,
        });
        insts.push(gep(3, 2, 8));
        insts.extend(roundtrip_body(5, 3, false));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "Ptr-to-Ptr casts must preserve provenance");
    }

    #[test]
    fn a_loaded_pointer_is_not_assumed_outside_the_current_frame() {
        // `src` is loaded from a local pointer slot and points at the same
        // stack object as `dst` (with an 8-byte offset). The old numeric-root
        // test classified this opaque load result as "not a stack home" and
        // incorrectly treated it as disjoint from the destination alloca.
        let mut insts = vec![
            alloca(2, 32),
            gep(3, 2, 8),
            alloca(4, 8),
            store_ptr(2, 4),
            load_ptr(5, 4),
        ];
        insts.extend(roundtrip_body(5, 3, false));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "loaded pointers must fail closed");
    }

    #[test]
    fn extra_uses_of_either_source_load_prevent_deletion() {
        for load_value in [10, 12] {
            let mut insts = roundtrip(2, 3);
            insts.push(cast(30, load_value, IrType::U64, IrType::I128));
            let mut f = func_with(insts);
            assert_eq!(
                run(&mut f),
                0,
                "source load %{} has a second reader",
                load_value
            );
        }
    }

    #[test]
    fn an_extra_use_of_the_right_shift_result_prevents_deletion() {
        let mut insts = roundtrip(2, 3);
        let shift = insts
            .iter()
            .position(|inst| {
                matches!(
                    inst,
                    Instruction::BinOp {
                        dest: Value(18),
                        ..
                    }
                )
            })
            .unwrap();
        // Keep the candidate's original high cast after this extra use so it
        // remains the cast selected as the store-producing half.
        insts.insert(shift + 1, cast(30, 18, IrType::I128, IrType::I64));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "the shift result is no longer single-use");
    }

    #[test]
    fn non_store_memory_and_ordering_effects_block_motion() {
        let effects = vec![
            Instruction::AtomicLoad {
                dest: Value(30),
                ptr: Operand::Value(Value(2)),
                ty: IrType::I64,
                ordering: crate::ir::ops::AtomicOrdering::SeqCst,
            },
            Instruction::Load {
                dest: Value(30),
                ptr: Value(2),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
                volatile: true,
            },
            Instruction::InlineAsm {
                template: "".into(),
                outputs: Vec::new(),
                inputs: Vec::new(),
                clobbers: vec!["memory".into()],
                operand_types: Vec::new(),
                goto_labels: Vec::new(),
                input_symbols: Vec::new(),
                seg_overrides: Vec::new(),
            },
            Instruction::Intrinsic {
                dest: None,
                op: crate::ir::intrinsics::IntrinsicOp::Storedqu,
                dest_ptr: Some(Value(2)),
                args: vec![Operand::Value(Value(31))],
            },
            Instruction::PgoCounterInc {
                name: "counter".into(),
                offset: 0,
                atomic: false,
            },
            Instruction::Intrinsic {
                dest: None,
                op: crate::ir::intrinsics::IntrinsicOp::Mfence,
                dest_ptr: None,
                args: Vec::new(),
            },
        ];
        for effect in effects {
            let label = format!("{effect:?}");
            let mut insts = roundtrip(2, 3);
            insts.insert(0, alloca(31, 16));
            insert_after_high_load(&mut insts, effect);
            let mut f = func_with(insts);
            assert_eq!(run(&mut f), 0, "must not move a read across {label}");
        }
    }

    #[test]
    fn a_non_adjacent_pair_keeps_the_round_trip() {
        let mut insts = roundtrip(2, 3);
        let high_ptr = insts
            .iter_mut()
            .find(|inst| {
                matches!(
                    inst,
                    Instruction::GetElementPtr {
                        dest: Value(11),
                        ..
                    }
                )
            })
            .unwrap();
        *high_ptr = gep(11, 2, 16); // high half 16 bytes away, not 8
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn a_signed_low_half_is_not_mistaken_for_a_bitwise_copy() {
        let mut insts = roundtrip(2, 3);
        let low_cast = insts
            .iter_mut()
            .find(|inst| {
                matches!(
                    inst,
                    Instruction::Cast {
                        dest: Value(14),
                        ..
                    }
                )
            })
            .unwrap();
        *low_cast = cast(14, 10, IrType::I64, IrType::I128);
        let mut f = func_with(insts);
        assert_eq!(
            run(&mut f),
            0,
            "a signed widening cast can smear the low half's sign into the high half"
        );
    }

    #[test]
    fn a_32_bit_source_is_not_matched() {
        let mut insts = roundtrip(2, 3);
        let high_cast = insts
            .iter_mut()
            .find(|inst| {
                matches!(
                    inst,
                    Instruction::Cast {
                        dest: Value(13),
                        ..
                    }
                )
            })
            .unwrap();
        *high_cast = cast(13, 12, IrType::U32, IrType::I128);
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }
}
