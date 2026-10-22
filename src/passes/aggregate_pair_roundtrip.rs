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
//! whole thing becomes one `Memcpy`.  This is the difference between the stock
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
//!   than 16 bytes (a same-object self copy is exact; a partial overlap is
//!   what C leaves undefined and what the element-wise sequence "implemented"
//!   one way);
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
//! target and lets the existing `Memcpy` lowering -- which already picks
//! `movdqu`/`movupd`, folded memory operands and store-to-load forwarding --
//! do the emit.

use crate::common::fx_hash::FxHashMap;
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

/// Bytes covered by the pattern: two 8-byte halves.
const COPY_BYTES: usize = 16;
const HALF: i64 = 8;

/// A pointer's provenance: `(root, byte offset)` where `root` is the value the
/// `gep` chain bottoms out at.  `None` means the chain contains a non-constant
/// offset (a runtime index), which is still a *usable* root: only the offset is
/// unknown.
fn root_of(func: &IrFunction, v: Value) -> (u32, Option<i64>) {
    let (mut cur, mut off) = (v.0, 0i64);
    for _ in 0..32 {
        let mut next = None;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::GetElementPtr {
                    dest, base, offset, ..
                } = inst
                {
                    if dest.0 == cur {
                        next = Some((
                            base.0,
                            match offset {
                                Operand::Const(IrConst::I64(o)) => Some(*o),
                                _ => None,
                            },
                        ));
                    }
                }
            }
            if next.is_some() {
                break;
            }
        }
        match next {
            None => return (cur, Some(off)),
            Some((base, None)) => return (base, None),
            Some((base, Some(d))) => match off.checked_add(d) {
                Some(next) => {
                    off = next;
                    cur = base;
                }
                None => return (base, None),
            },
        }
    }
    (cur, None)
}

/// `a - b` in bytes when it is provable: equal pointers are 0, a `gep` of the
/// other is its constant offset, and two `gep`s off one root differ by their
/// offsets.  `None` means "cannot prove" -- callers fail closed.
fn offset_from(func: &IrFunction, a: Value, b: Value) -> Option<i64> {
    if a.0 == b.0 {
        return Some(0);
    }
    // A direct `gep b, k`.
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GetElementPtr {
                dest, base, offset, ..
            } = inst
            {
                if dest.0 == a.0 && base.0 == b.0 {
                    if let Operand::Const(IrConst::I64(k)) = offset {
                        return Some(*k);
                    }
                }
            }
        }
    }
    let (ra, oa) = root_of(func, a);
    let (rb, ob) = root_of(func, b);
    if ra == rb {
        return match (oa, ob) {
            (Some(x), Some(y)) => Some(x - y),
            _ => None,
        };
    }
    None
}

fn is_cst(v: &Operand, want: i64) -> bool {
    match v {
        Operand::Const(IrConst::I64(x)) => *x == want,
        Operand::Const(IrConst::I32(x)) => i64::from(*x) == want,
        _ => false,
    }
}

fn clobbers(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Store { .. }
            | Instruction::Memcpy { .. }
            | Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::AtomicStore { .. }
            | Instruction::AtomicRmw { .. }
            | Instruction::AtomicCmpxchg { .. }
            | Instruction::Fence { .. }
            | Instruction::VaArg { .. }
            | Instruction::VaArgStruct { .. }
            | Instruction::VaArgVector { .. }
            | Instruction::VaStart { .. }
            | Instruction::VaCopy { .. }
            | Instruction::StackRestore { .. }
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
/// home.  A store elsewhere is treated as an opaque clobber.
fn store_root(func: &IrFunction, ptr: Value) -> Option<u32> {
    let (root, _) = root_of(func, ptr);
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                if dest.0 == root {
                    return Some(root);
                }
            }
        }
    }
    None
}

/// Whether a 16-byte copy from `src` to `dst` can be proven not to overlap
/// partially.  Same pointer (an exact self copy) and same-root objects at
/// least 16 bytes apart are fine, as are two distinct stack homes and a stack
/// home against a pointer this frame cannot name: a pointer parameter can
/// never point into the current invocation's frame, so a destination that is
/// a non-escaping-by-construction stack home cannot alias it.
fn provably_disjoint(func: &IrFunction, src: Value, dst: Value) -> bool {
    if let Some(d) = offset_from(func, dst, src) {
        return d.unsigned_abs() >= COPY_BYTES as u64;
    }
    let (sroot, _) = root_of(func, src);
    let (droot, _) = root_of(func, dst);
    if sroot == droot {
        return false;
    }
    let s_home = store_root(func, src).is_some();
    let d_home = store_root(func, dst).is_some();
    // Distinct stack homes, or a stack home against an unnameable pointer.
    s_home != d_home || (s_home && d_home)
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut total = 0;
    // Each successful rewrite removes at least five instructions, so the loop
    // terminates; the bound is a belt-and-braces guard for a malformed IR.
    for _ in 0..4096 {
        let Some(c) = find_one(func) else {
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

fn find_one(func: &IrFunction) -> Option<Candidate> {
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
            {
                continue;
            }

            // The two halves must be exactly 8 bytes apart on both sides.
            if offset_from(func, hi_src.ptr, lo_src.ptr) != Some(HALF)
                || offset_from(func, q_hi, q_lo) != Some(HALF)
            {
                continue;
            }
            if !provably_disjoint(func, lo_src.ptr, q_lo) {
                continue;
            }
            let (p_root, _) = root_of(func, lo_src.ptr);
            let (q_root, _) = root_of(func, q_lo);

            // The read of `p` moves later (to the store site), so an
            // intervening memory write is only harmless when it provably
            // cannot touch `p` or `q`.  A store into a stack home qualifies:
            // its provenance base is the alloca, and `p`/`q` are either
            // parameters or the same alloca -- both cases settle below.
            // Anything unprovable (a call, an atomic, a fence, a memcpy, an
            // escaping pointer) refuses.
            let first = lo_src.load_index.min(hi_src.load_index);
            let last = store_lo_i.max(store_hi_i);
            let replaced = [store_lo_i, store_hi_i];
            let mut blocked = false;
            for (ii, inst) in block.instructions.iter().enumerate() {
                if ii < first || ii > last || replaced.contains(&ii) || !clobbers(inst) {
                    continue;
                }
                if let Instruction::Store {
                    ptr,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                    ..
                } = inst
                {
                    let store_root = store_root(func, *ptr);
                    if store_root.is_none()
                        || store_root == Some(p_root)
                        || store_root == Some(q_root)
                    {
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
        if dest.0 != cv.0 || !matches!(from_ty, IrType::I64 | IrType::U64) {
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
        // src = base + 8, dst = base: 16 bytes from base+8 into base overlaps.
        let mut insts = vec![gep(4, 2, 8)];
        insts.extend(roundtrip(4, 2));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "a partial overlap must refuse");
    }

    #[test]
    fn a_non_adjacent_pair_keeps_the_round_trip() {
        let mut insts = roundtrip(2, 3);
        insts[1] = gep(11, 2, 16); // high half 16 bytes away, not 8
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn a_32_bit_source_is_not_matched() {
        let mut insts = roundtrip(2, 3);
        insts[3] = cast(13, 12, IrType::U32, IrType::I128);
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }
}
