//! Register-lane forwarding for whole-vector reloads of a patched temporary.
//!
//! # The shape
//!
//! A 32-byte `union { __m256 v; unsigned u[8]; }` whose vector member is
//! written whole and then one 32-bit lane is overwritten lowers to:
//!
//! ```text
//!   %t = alloca 32
//!   VecStoreF32x8 %v, %t          ; bits.v = v          (32-byte store)
//!   Store u32 %w, %t              ; bits.u[0] = w       (4-byte store)
//!   %r = VecLoadF32x8 %t          ; read bits.v back    (32-byte load)
//! ```
//!
//! The 32-byte load overlaps a 4-byte store that has not yet retired, so the
//! load cannot be store-forwarded.  Intel cores then stall the load until the
//! store commits, roughly 15-20 cycles per occurrence.  In
//! `tests/benchmark/programs/avx_fp_private.c` this one reload per iteration
//! accounted for most of the gap to Clang.  Measured on the same 2-core host
//! (three runs each, identical checksums): baseline lccc 34-36 ms; this pass
//! 15.6-16.5 ms; the same loop with only the stall removed by hand 12.5-13.1
//! ms; Clang 11.1-11.4 ms; GCC 16.2 30.2-30.4 ms.
//!
//! # The rewrite
//!
//! Every pending lane write is folded into a register value, so the reload no
//! longer touches memory:
//!
//! ```text
//!   %b1 = VecBroadcastI32x8 %w           ; w in every dword
//!   %r  = BlendPs256 %v, %b1, 1          ; take dword 0 from %b1, rest from %v
//! ```
//!
//! `vblendps` selects whole dwords bit-for-bit, so NaN payloads, signed zeros
//! and denormals survive the rewrite unchanged, and `vpbroadcastd`/`vbroadcastss`
//! copy the 32-bit pattern exactly.
//!
//! # Why the conditions are what they are
//!
//! The rewrite replaces a memory read with a value, so every condition below
//! is a correctness condition:
//!
//! * **Block-local.**  Every access to the temporary and to each address
//!   derived from it by a constant-offset `GetElementPtr` sits in one basic
//!   block.  Within a block, program order is the only ordering that matters,
//!   and the simulation below replays it exactly.  Any access elsewhere
//!   refuses the temporary.
//! * **Fully accounted.**  Each use is a recognised form: a whole-width
//!   `VecStore*` that defines all 32 bytes, a 32-bit scalar store to a
//!   constant lane, a whole-width `VecLoad*`, a scalar load, or a derived
//!   address.  An unrecognised use, a stored address (escape), or a volatile
//!   or non-32-bit-lane store refuses the temporary.
//! * **Defined before read.**  A whole-width reload is rewritten only after a
//!   defining store in the block.  Reading bytes no store has written would
//!   need the frame's contents, which the rewrite cannot know.
//! * **Exact liveness.**  A store is removed only when no surviving read
//!   observes its bytes: a read sees the latest write to each 32-bit lane, so
//!   a write shadowed in every lane it wrote, before any read, is dead.  A
//!   read that reaches a lane no write in the block has touched may see the
//!   previous iteration's bytes, so such lanes keep every write in the block
//!   alive.  Rewritten reloads read no memory.
//! * **x86-64 only.**  `BlendPs256`/`VecBroadcast*x8` exist only in the x86
//!   backend.  The pass is a no-op on every other target.
//!
//! Known residual (follow-up): the broadcast is emitted between the base
//! load and the blend, and its emitter flushes the base's pending register
//! value to a stack home and reloads it.  That costs one 32-byte store and
//! reload per rewrite, which accounts for the remaining gap to the
//! hand-patched loop.  Removing it needs the broadcast to keep the pending
//! value live, which is an emitter change rather than an IR one.
//!
//! The pass iterates to a fixpoint.  Each application retires at least one
//! reload with pending lanes and creates none, so it terminates.

use crate::common::fx_hash::FxHashMap;
use crate::common::source::Span;
use crate::common::types::{AddressSpace, IrType, target_arch_name};
use crate::ir::instruction::{replace_values_in_inst_map, replace_values_in_terminator_map};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{
    BasicBlock, Instruction, IrConst, IrFunction, IrModule, Operand, Value,
};
use crate::passes::use_def::{UseDefInfo, UseLoc};
use crate::passes::vector_temp_promotion::{whole_vector_load_width, whole_vector_store_width};

/// Byte width of the temporary this pass rewrites.
const TEMP_BYTES: usize = 32;

/// Width of one lane: the 32-bit element the scalar lane stores write.
const LANE_BYTES: usize = 4;

/// Number of 32-bit lanes in the temporary.
const LANES: usize = TEMP_BYTES / LANE_BYTES;

/// Size in bytes of a scalar type a memory access can move.
fn access_size(ty: IrType) -> Option<usize> {
    Some(match ty {
        IrType::I8 | IrType::U8 => 1,
        IrType::I16 | IrType::U16 => 2,
        IrType::I32 | IrType::U32 | IrType::F32 => 4,
        IrType::I64 | IrType::U64 | IrType::F64 | IrType::Ptr => 8,
        _ => return None,
    })
}

/// Scalar 32-bit lane types whose store can be folded into a register lane.
fn is_lane_type(ty: IrType) -> bool {
    matches!(ty, IrType::I32 | IrType::U32 | IrType::F32)
}

/// Broadcast that replicates a 32-bit scalar of `ty` into all eight dwords.
fn lane_broadcast(ty: IrType) -> IntrinsicOp {
    match ty {
        IrType::F32 => IntrinsicOp::VecBroadcastF32x8,
        _ => IntrinsicOp::VecBroadcastI32x8,
    }
}

/// How one instruction in the temporary's block touches it.
#[derive(Clone, Debug)]
enum Access {
    /// Whole-width store defining all 32 bytes as `value`.
    Define { value: Value },
    /// 32-bit scalar store of `val` to the constant lane `lane`.
    Lane {
        lane: usize,
        val: Operand,
        ty: IrType,
    },
    /// Whole-width load of the temporary into `dest`.
    WholeLoad { dest: Value },
    /// Scalar load of lanes `first..=last`: reads those lanes of the memory
    /// image.
    ScalarLoad { first: usize, last: usize },
    /// Constant-offset address derived from the temporary.
    Address,
}

/// One whole-width reload to replace with a register chain.
#[derive(Clone, Debug)]
struct Rewrite {
    index: usize,
    dest: Value,
    /// Value the chain starts from: the most recent whole-width definition.
    base: Value,
    /// Lanes written since `base`, ascending: `(lane, value, type)`.
    pending: Vec<(usize, Operand, IrType)>,
}

/// A planned rewrite of one temporary in one block.
#[derive(Debug)]
struct Plan {
    block: usize,
    rewrites: Vec<Rewrite>,
    /// Instructions to delete.  Empty unless every reader of the temporary is
    /// rewritten, because a surviving memory reader needs the stores.
    retire: Vec<usize>,
}

/// The address of `value` as a byte offset from the temporary, if derived.
fn offset_of(value: &Value, offsets: &FxHashMap<u32, usize>) -> Option<usize> {
    offsets.get(&value.0).copied()
}

/// Constant byte offset of a `GetElementPtr` index, if it is a non-negative
/// integer constant.
fn const_offset(operand: &Operand) -> Option<usize> {
    match operand {
        Operand::Const(IrConst::I64(v)) if *v >= 0 => usize::try_from(*v).ok(),
        Operand::Const(IrConst::I32(v)) if *v >= 0 => usize::try_from(*v).ok(),
        _ => None,
    }
}

/// Classify one instruction that touches the temporary or a derived address.
/// `None` refuses the whole temporary.
fn classify(inst: &Instruction, temp: Value, offsets: &FxHashMap<u32, usize>) -> Option<Access> {
    match inst {
        // Address arithmetic is validated where the derived address is
        // recorded (`plan_temp`, phase 1); here it only has to be classified.
        Instruction::GetElementPtr { .. } => Some(Access::Address),
        Instruction::Intrinsic {
            dest: None,
            op,
            dest_ptr: Some(store_dest),
            args,
            ..
        } => {
            if *store_dest != temp {
                return None;
            }
            let width = whole_vector_store_width(*op)?;
            match args.as_slice() {
                [
                    Operand::Value(value),
                    Operand::Value(addr),
                    Operand::Const(IrConst::I64(0)),
                ] if *addr == temp && width == TEMP_BYTES && *value != temp => {
                    Some(Access::Define { value: *value })
                }
                _ => None,
            }
        }
        Instruction::Intrinsic {
            dest: Some(dest),
            op,
            dest_ptr: None,
            args,
            ..
        } => {
            let width = whole_vector_load_width(*op)?;
            match args.as_slice() {
                [Operand::Value(addr), Operand::Const(IrConst::I64(0))]
                    if *addr == temp && width == TEMP_BYTES =>
                {
                    Some(Access::WholeLoad { dest: *dest })
                }
                _ => None,
            }
        }
        Instruction::Store {
            val,
            ptr,
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
            ..
        } => {
            let off = offset_of(ptr, offsets)?;
            // A stored address escapes the temporary: refuse.
            if let Operand::Value(v) = val {
                if offsets.contains_key(&v.0) {
                    return None;
                }
            }
            if !is_lane_type(*ty) || off % LANE_BYTES != 0 {
                return None;
            }
            Some(Access::Lane {
                lane: off / LANE_BYTES,
                val: val.clone(),
                ty: *ty,
            })
        }
        Instruction::Load {
            ptr,
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
            ..
        } => {
            let off = offset_of(ptr, offsets)?;
            let size = access_size(*ty)?;
            let end = off.checked_add(size)?;
            if end > TEMP_BYTES {
                return None;
            }
            Some(Access::ScalarLoad {
                first: off / LANE_BYTES,
                last: (end - 1) / LANE_BYTES,
            })
        }
        _ => None,
    }
}

/// Plan the rewrite of one temporary, or `None` when any use is refused.
fn plan_temp(func: &IrFunction, info: &UseDefInfo, temp: Value) -> Option<Plan> {
    // Phase 1: the block holding every access, and the derived addresses.
    let mut block: Option<usize> = None;
    let mut offsets: FxHashMap<u32, usize> = FxHashMap::default();
    offsets.insert(temp.0, 0);
    let mut in_block_uses: Vec<usize> = Vec::new();
    let bi = {
        // A temporary nobody reads has nothing to forward.
        info.use_insts_of(temp.0).next()?;
        for loc in info.use_insts_of(temp.0) {
            if loc.inst_idx == UseLoc::TERMINATOR {
                return None;
            }
            let b = loc.block_idx as usize;
            match block {
                None => block = Some(b),
                Some(prev) if prev != b => return None,
                Some(_) => {}
            }
        }
        block?
    };
    let insts = &func.blocks.get(bi)?.instructions;
    // Derived addresses, in definition order (a GEP's base precedes it).
    for inst in insts {
        if let Instruction::GetElementPtr {
            dest, base, offset, ..
        } = inst
        {
            if let Some(base_off) = offset_of(base, &offsets) {
                let off = const_offset(offset)?;
                let total = base_off.checked_add(off)?;
                if total >= TEMP_BYTES {
                    return None;
                }
                offsets.insert(dest.0, total);
            }
        }
    }
    // Phase 2: every use of every derived address must be in the same block
    // and recognised.  Collect the instruction indices involved.
    let mut addresses: Vec<u32> = offsets.keys().copied().collect();
    addresses.sort_unstable();
    for id in addresses {
        for loc in info.use_insts_of(id) {
            if loc.inst_idx == UseLoc::TERMINATOR || loc.block_idx as usize != bi {
                return None;
            }
            in_block_uses.push(loc.inst_idx as usize);
        }
    }
    in_block_uses.sort_unstable();
    in_block_uses.dedup();

    // Phase 3: classify each involved instruction in program order and
    // replay the temporary's memory image lane by lane.
    //
    // A write is observable only through a read that sees it as the latest
    // write to a lane.  `last_writer` tracks that; a read that reaches a lane
    // no write in this block has touched may see the previous iteration's
    // bytes, so such lanes are conservatively observed for every write in the
    // block.  Because every use of the temporary is in this block, nothing
    // else can observe the image after the block.
    let mut base: Option<Value> = None;
    let mut lanes: Vec<Option<(Operand, IrType)>> = vec![None; LANES];
    let mut last_writer: Vec<Option<usize>> = vec![None; LANES];
    let mut writers: Vec<(usize, u32)> = Vec::new();
    let mut observed: std::collections::BTreeSet<usize> = Default::default();
    let mut incoming: u32 = 0;
    let mut rewrites: Vec<Rewrite> = Vec::new();
    let mut unrewritten_reads = 0usize;
    let mut scalar_reads = 0usize;
    let all_lanes: u32 = (1u32 << LANES) - 1;
    let observe = |mask: u32,
                   last_writer: &[Option<usize>],
                   observed: &mut std::collections::BTreeSet<usize>,
                   incoming: &mut u32| {
        for lane in 0..LANES {
            if mask & (1u32 << lane) == 0 {
                continue;
            }
            match last_writer[lane] {
                Some(w) => {
                    observed.insert(w);
                }
                None => *incoming |= 1u32 << lane,
            }
        }
    };
    for &ii in &in_block_uses {
        let inst = insts.get(ii)?;
        let access = classify(inst, temp, &offsets)?;
        match access {
            Access::Address => {}
            Access::Define { value } => {
                base = Some(value);
                lanes.iter_mut().for_each(|slot| *slot = None);
                last_writer.iter_mut().for_each(|slot| *slot = Some(ii));
                writers.push((ii, all_lanes));
            }
            Access::Lane { lane, val, ty } => {
                if base.is_none() {
                    return None;
                }
                lanes[lane] = Some((val, ty));
                last_writer[lane] = Some(ii);
                writers.push((ii, 1u32 << lane));
            }
            Access::ScalarLoad { first, last } => {
                scalar_reads += 1;
                let mask = (first..=last).fold(0u32, |m, lane| m | (1u32 << lane));
                observe(mask, &last_writer, &mut observed, &mut incoming);
            }
            Access::WholeLoad { dest } => {
                let base_value = base?;
                let pending: Vec<(usize, Operand, IrType)> = lanes
                    .iter()
                    .enumerate()
                    .filter_map(|(lane, slot)| {
                        slot.as_ref().map(|(val, ty)| (lane, val.clone(), *ty))
                    })
                    .collect();
                if pending.is_empty() {
                    // Still a memory read of the whole temporary.
                    unrewritten_reads += 1;
                    observe(all_lanes, &last_writer, &mut observed, &mut incoming);
                } else {
                    rewrites.push(Rewrite {
                        index: ii,
                        dest,
                        base: base_value,
                        pending,
                    });
                }
            }
        }
    }
    if rewrites.is_empty() {
        return None;
    }
    // A write whose bytes reach an incoming read is observable from earlier
    // iterations, so it stays.
    for &(w, mask) in &writers {
        if mask & incoming != 0 {
            observed.insert(w);
        }
    }
    // Retire every write nothing observes.  Address arithmetic goes only when
    // no memory access survives that could use it.
    let mut retire: Vec<usize> = writers
        .iter()
        .map(|&(w, _)| w)
        .filter(|w| !observed.contains(w))
        .collect();
    let memory_user_survives = !observed.is_empty() || scalar_reads > 0 || unrewritten_reads > 0;
    if !memory_user_survives {
        retire.extend(
            in_block_uses.iter().copied().filter(|&ii| {
                matches!(classify(&insts[ii], temp, &offsets), Some(Access::Address))
            }),
        );
    }
    retire.sort_unstable();
    retire.dedup();
    Some(Plan {
        block: bi,
        rewrites,
        retire,
    })
}

/// Insert `prefix` at the front of the entry block, keeping its per-instruction
/// side tables in lockstep (or clearing them when they were not).
fn prepend_to_entry(block: &mut BasicBlock, prefix: Vec<Instruction>) {
    if prefix.is_empty() {
        return;
    }
    let n = prefix.len();
    let spans_in_lockstep = block.source_spans.len() == block.instructions.len();
    let ids_in_lockstep = block.source_inline_ids.len() == block.instructions.len();
    block.instructions.splice(0..0, prefix);
    if spans_in_lockstep {
        block
            .source_spans
            .splice(0..0, std::iter::repeat_n(Span::dummy(), n));
    } else {
        block.source_spans.clear();
    }
    if ids_in_lockstep {
        block
            .source_inline_ids
            .splice(0..0, std::iter::repeat_n(0u32, n));
    } else {
        block.source_inline_ids.clear();
    }
}

/// Replace the planned reloads with register chains and delete retired
/// accesses.
///
/// Vector values are homes: a vector-producing op writes its home through
/// `dest_ptr`, and the home *is* the value.  Each blend therefore writes a
/// fresh 32-byte home, and the last home replaces the reload's result
/// everywhere it is used.  The homes are allocated in the entry block so the
/// frame layout stays static.
fn apply_plan(func: &mut IrFunction, plan: Plan) {
    let mut next_value = func.next_value_id;
    let mut fresh = || {
        let v = Value(next_value);
        next_value += 1;
        v
    };
    let rewrite_at: FxHashMap<usize, Rewrite> =
        plan.rewrites.into_iter().map(|rw| (rw.index, rw)).collect();
    let retired: Vec<usize> = plan.retire;
    let mut renamed: FxHashMap<u32, u32> = FxHashMap::default();
    let mut hoisted_homes: Vec<Instruction> = Vec::new();

    let block: &mut BasicBlock = &mut func.blocks[plan.block];
    let old = std::mem::take(&mut block.instructions);
    let old_spans = std::mem::take(&mut block.source_spans);
    let old_ids = std::mem::take(&mut block.source_inline_ids);
    let spans_in_lockstep = old_spans.len() == old.len();
    let ids_in_lockstep = old_ids.len() == old.len();

    let mut instructions: Vec<Instruction> = Vec::with_capacity(old.len());
    let mut spans: Vec<Span> = Vec::new();
    let mut ids: Vec<u32> = Vec::new();
    for (index, inst) in old.into_iter().enumerate() {
        if retired.binary_search(&index).is_ok() {
            continue;
        }
        let span = spans_in_lockstep.then(|| old_spans[index].clone());
        let id = ids_in_lockstep.then(|| old_ids[index]);
        let mut push = |inst: Instruction| {
            instructions.push(inst);
            if let Some(span) = &span {
                spans.push(span.clone());
            }
            if let Some(id) = id {
                ids.push(id);
            }
        };
        match rewrite_at.get(&index) {
            Some(rw) => {
                let mut current = rw.base;
                for (lane, val, ty) in &rw.pending {
                    let broadcast = fresh();
                    push(Instruction::Intrinsic {
                        dest: Some(broadcast),
                        op: lane_broadcast(*ty),
                        dest_ptr: None,
                        args: vec![val.clone()],
                    });
                    let home = fresh();
                    hoisted_homes.push(Instruction::Alloca {
                        dest: home,
                        ty: IrType::Ptr,
                        size: TEMP_BYTES,
                        align: TEMP_BYTES,
                        volatile: false,
                        semantic_volatile: false,
                    });
                    push(Instruction::Intrinsic {
                        dest: None,
                        op: IntrinsicOp::BlendPs256,
                        dest_ptr: Some(home),
                        args: vec![
                            Operand::Value(current),
                            Operand::Value(broadcast),
                            Operand::Const(IrConst::I64(1i64 << lane)),
                        ],
                    });
                    current = home;
                }
                renamed.insert(rw.dest.0, current.0);
            }
            None => push(inst),
        }
    }
    block.instructions = instructions;
    if spans_in_lockstep {
        block.source_spans = spans;
    }
    if ids_in_lockstep {
        block.source_inline_ids = ids;
    }
    func.next_value_id = next_value;

    prepend_to_entry(&mut func.blocks[0], hoisted_homes);
    for block in func.blocks.iter_mut() {
        for inst in block.instructions.iter_mut() {
            replace_values_in_inst_map(inst, &renamed);
        }
        replace_values_in_terminator_map(&mut block.terminator, &renamed);
    }
}

/// Rewrite one function to a fixpoint.  Returns the number of temporaries
/// rewritten.
fn lane_patch_in_function(func: &mut IrFunction) -> usize {
    let mut rewritten = 0usize;
    loop {
        let info = UseDefInfo::build(func);
        let candidates: Vec<Value> = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|inst| match inst {
                Instruction::Alloca {
                    dest,
                    size,
                    volatile: false,
                    semantic_volatile: false,
                    ..
                } if *size == TEMP_BYTES => Some(*dest),
                _ => None,
            })
            .collect();
        let Some(plan) = candidates
            .into_iter()
            .find_map(|temp| plan_temp(func, &info, temp))
        else {
            break;
        };
        apply_plan(func, plan);
        rewritten += 1;
    }
    rewritten
}

/// Entry point: replace store-forwarding-stalled whole-vector reloads with
/// register lane chains.  x86-64 with AVX2 only; a no-op elsewhere.
pub(crate) fn forward_lane_patched_reloads(module: &mut IrModule) -> usize {
    // `vpbroadcastd` and the register-source `vbroadcastss` are AVX2
    // encodings, so the rewrite needs the same AVX2 gate the vectorizer
    // applies to ymm-shaped intrinsics.
    if target_arch_name() != "x86-64" || !crate::passes::vectorize::x86_avx2_available_pub() {
        return 0;
    }
    module.for_each_function(lane_patch_in_function)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::source::Span;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    fn function(blocks: Vec<Vec<Instruction>>) -> IrFunction {
        let mut func = IrFunction::new("t".into(), IrType::Void, Vec::new(), false);
        func.blocks = blocks
            .into_iter()
            .enumerate()
            .map(|(i, instructions)| BasicBlock {
                label: BlockId(i as u32),
                source_spans: vec![Span::dummy(); instructions.len()],
                source_inline_ids: Vec::new(),
                instructions,
                terminator: Terminator::Return(None),
            })
            .collect();
        func.next_value_id = 100;
        func
    }

    fn alloca32(dest: u32) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size: TEMP_BYTES,
            align: 32,
            volatile: false,
            semantic_volatile: false,
        }
    }

    /// `VecStoreF32x8 value -> temp`
    fn define(value: u32, temp: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: None,
            op: IntrinsicOp::VecStoreF32x8,
            dest_ptr: Some(Value(temp)),
            args: vec![
                Operand::Value(Value(value)),
                Operand::Value(Value(temp)),
                Operand::Const(IrConst::I64(0)),
            ],
        }
    }

    /// `Store u32 w -> ptr`
    fn lane_store(w: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(w)),
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    /// `dest = VecLoadF32x8 temp`
    fn reload(dest: u32, temp: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op: IntrinsicOp::VecLoadF32x8,
            dest_ptr: None,
            args: vec![Operand::Value(Value(temp)), Operand::Const(IrConst::I64(0))],
        }
    }

    fn gep(dest: u32, base: u32, offset: Operand) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset,
            ty: IrType::U32,
        }
    }

    fn count(func: &IrFunction, pred: impl Fn(&Instruction) -> bool) -> usize {
        func.blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter(|i| pred(i))
            .count()
    }

    fn is_reload(inst: &Instruction) -> bool {
        matches!(
            inst,
            Instruction::Intrinsic {
                op: IntrinsicOp::VecLoadF32x8,
                ..
            }
        )
    }

    fn is_blend(inst: &Instruction) -> bool {
        matches!(
            inst,
            Instruction::Intrinsic {
                op: IntrinsicOp::BlendPs256,
                ..
            }
        )
    }

    fn is_store(inst: &Instruction) -> bool {
        matches!(inst, Instruction::Store { .. })
    }

    fn blend_masks(func: &IrFunction) -> Vec<i64> {
        func.blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|inst| match inst {
                Instruction::Intrinsic {
                    op: IntrinsicOp::BlendPs256,
                    args,
                    ..
                } => match args.as_slice() {
                    [_, _, Operand::Const(IrConst::I64(mask))] => Some(*mask),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// Run the pass to fixpoint on one function; the IR must still verify
    /// structurally (value ids unique, no reload of a retired temporary).
    fn run(mut func: IrFunction) -> IrFunction {
        lane_patch_in_function(&mut func);
        func
    }

    #[test]
    fn lane_write_then_reload_becomes_blend_and_retires_stores() {
        let func = run(function(vec![vec![
            alloca32(20),
            define(11, 20),
            lane_store(12, 20, IrType::U32),
            reload(30, 20),
        ]]));
        assert_eq!(count(&func, is_reload), 0, "reload must be gone");
        assert_eq!(count(&func, is_blend), 1);
        assert_eq!(blend_masks(&func), vec![1]);
        assert_eq!(count(&func, is_store), 0, "no memory reader: stores retire");
        // The final blend writes a fresh 32-byte home in the entry block,
        // and that home replaces the reload's result.
        let home = match func.blocks[0].instructions.iter().find_map(|i| match i {
            Instruction::Intrinsic {
                op: IntrinsicOp::BlendPs256,
                dest_ptr: Some(h),
                ..
            } => Some(*h),
            _ => None,
        }) {
            Some(h) => h,
            None => panic!("no blend with a home destination"),
        };
        assert!(
            func.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::Alloca { dest, size: TEMP_BYTES, .. } if *dest == home
            )),
            "the home must be allocated in the entry block"
        );
    }

    #[test]
    fn scalar_reader_keeps_the_memory_image() {
        let func = run(function(vec![vec![
            alloca32(20),
            define(11, 20),
            lane_store(12, 20, IrType::U32),
            reload(30, 20),
            Instruction::Load {
                dest: Value(31),
                ptr: Value(20),
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ]]));
        assert_eq!(count(&func, is_reload), 0);
        assert_eq!(count(&func, is_blend), 1);
        // The scalar load reads lane 0, which the lane store wrote last.  The
        // define's bytes in lanes 1..7 are never read and lane 0 is shadowed,
        // so the define is dead and retires; the lane store and the load stay.
        assert_eq!(
            count(&func, is_store),
            1,
            "lane store must stay for the scalar load"
        );
        assert_eq!(
            count(&func, |i| matches!(
                i,
                Instruction::Intrinsic {
                    op: IntrinsicOp::VecStoreF32x8,
                    ..
                }
            )),
            0,
            "the define is shadowed in every lane that is read"
        );
    }

    #[test]
    fn define_observed_by_a_wide_read_stays() {
        // An unrewritten whole-width load (no lane pending) reads every lane
        // of the image, so the define that supplies them is observable.
        let func = run(function(vec![vec![
            alloca32(20),
            define(11, 20),
            reload(30, 20),
        ]]));
        assert_eq!(
            count(&func, |i| matches!(
                i,
                Instruction::Intrinsic {
                    op: IntrinsicOp::VecStoreF32x8,
                    ..
                }
            )),
            1
        );
        assert_eq!(
            count(&func, is_blend),
            0,
            "no lane pending: nothing to chain"
        );
    }

    #[test]
    fn write_shadowed_before_any_read_retires() {
        // define(11); lane 0 <- 12 (shadowed); define(13); lane 0 <- 14 (the
        // only lane the reload and the scalar load see).  Both earlier writes
        // are dead: the first lane store and the first define are overwritten
        // before any read, and the second define's other lanes are never read.
        let func = run(function(vec![vec![
            alloca32(20),
            define(11, 20),
            lane_store(12, 20, IrType::U32),
            define(13, 20),
            lane_store(14, 20, IrType::U32),
            reload(30, 20),
            Instruction::Load {
                dest: Value(31),
                ptr: Value(20),
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ]]));
        assert_eq!(count(&func, is_reload), 0);
        assert_eq!(count(&func, is_blend), 1);
        assert_eq!(
            count(&func, is_store),
            1,
            "only the observed lane store (%14) survives"
        );
        assert_eq!(
            count(&func, |i| matches!(
                i,
                Instruction::Intrinsic {
                    op: IntrinsicOp::VecStoreF32x8,
                    ..
                }
            )),
            0,
            "both defines are shadowed before any observation"
        );
    }

    #[test]
    fn multiple_lanes_chain_in_ascending_order_with_last_write_winning() {
        // Lane 0 is written twice (%12, then %14): only the last write is
        // chained.  Lane 1 is a 32-bit float store through a constant GEP.
        let func = run(function(vec![vec![
            alloca32(20),
            define(11, 20),
            lane_store(12, 20, IrType::U32),
            gep(21, 20, Operand::Const(IrConst::I64(4))),
            lane_store(13, 21, IrType::F32),
            lane_store(14, 20, IrType::U32),
            reload(30, 20),
        ]]));
        assert_eq!(count(&func, is_reload), 0);
        assert_eq!(count(&func, is_blend), 2);
        assert_eq!(blend_masks(&func), vec![1, 2]);
        let broadcasts: Vec<(IntrinsicOp, Operand)> = func.blocks[0]
            .instructions
            .iter()
            .filter_map(|inst| match inst {
                Instruction::Intrinsic {
                    op,
                    args,
                    dest: Some(_),
                    ..
                } if matches!(
                    op,
                    IntrinsicOp::VecBroadcastI32x8 | IntrinsicOp::VecBroadcastF32x8
                ) =>
                {
                    Some((*op, args[0].clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            broadcasts,
            vec![
                (IntrinsicOp::VecBroadcastI32x8, Operand::Value(Value(14))),
                (IntrinsicOp::VecBroadcastF32x8, Operand::Value(Value(13))),
            ],
            "lane 0 must carry the last write; lane 1 must use the float broadcast"
        );
    }

    #[test]
    fn reload_without_a_defining_store_is_refused() {
        let input = function(vec![vec![
            alloca32(20),
            lane_store(12, 20, IrType::U32),
            reload(30, 20),
        ]]);
        let before = format!("{:?}", input.blocks);
        let func = run(input);
        assert_eq!(format!("{:?}", func.blocks), before);
    }

    #[test]
    fn access_in_another_block_is_refused() {
        let input = function(vec![
            vec![
                alloca32(20),
                define(11, 20),
                lane_store(12, 20, IrType::U32),
            ],
            vec![reload(30, 20)],
        ]);
        let before = format!("{:?}", input.blocks);
        let func = run(input);
        assert_eq!(format!("{:?}", func.blocks), before);
    }

    #[test]
    fn runtime_lane_offset_is_refused() {
        // `base + runtime` cannot be a constant lane: the address-producing
        // GEP is not classified and the whole temporary is refused.
        let input = function(vec![vec![
            alloca32(20),
            define(11, 20),
            Instruction::GetElementPtr {
                dest: Value(21),
                base: Value(20),
                offset: Operand::Value(Value(40)),
                ty: IrType::U32,
            },
            lane_store(12, 21, IrType::U32),
            reload(30, 20),
        ]]);
        let before = format!("{:?}", input.blocks);
        let func = run(input);
        assert_eq!(format!("{:?}", func.blocks), before);
    }

    #[test]
    fn constant_gep_lane_is_chained_at_its_lane() {
        let func = run(function(vec![vec![
            alloca32(20),
            define(11, 20),
            gep(21, 20, Operand::Const(IrConst::I64(16))),
            lane_store(12, 21, IrType::U32),
            reload(30, 20),
        ]]));
        assert_eq!(count(&func, is_reload), 0);
        // Byte offset 16 is lane 4: mask 1 << 4.
        assert_eq!(blend_masks(&func), vec![16]);
        assert_eq!(
            count(&func, |i| matches!(i, Instruction::GetElementPtr { .. })),
            0
        );
    }

    #[test]
    fn non_lane_width_store_is_refused() {
        let input = function(vec![vec![
            alloca32(20),
            define(11, 20),
            Instruction::Store {
                val: Operand::Const(IrConst::I64(1)),
                ptr: Value(20),
                ty: IrType::U8,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
            reload(30, 20),
        ]]);
        let before = format!("{:?}", input.blocks);
        let func = run(input);
        assert_eq!(format!("{:?}", func.blocks), before);
    }

    #[test]
    fn stored_address_escapes_and_is_refused() {
        let input = function(vec![vec![
            alloca32(20),
            define(11, 20),
            Instruction::Store {
                val: Operand::Value(Value(20)),
                ptr: Value(20),
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
            reload(30, 20),
        ]]);
        let before = format!("{:?}", input.blocks);
        let func = run(input);
        assert_eq!(format!("{:?}", func.blocks), before);
    }

    #[test]
    fn volatile_store_is_refused() {
        let input = function(vec![vec![
            alloca32(20),
            define(11, 20),
            Instruction::Store {
                val: Operand::Value(Value(12)),
                ptr: Value(20),
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: true,
            },
            reload(30, 20),
        ]]);
        let before = format!("{:?}", input.blocks);
        let func = run(input);
        assert_eq!(format!("{:?}", func.blocks), before);
    }

    #[test]
    fn repeated_reloads_each_get_their_own_chain() {
        let func = run(function(vec![vec![
            alloca32(20),
            define(11, 20),
            lane_store(12, 20, IrType::U32),
            reload(30, 20),
            reload(31, 20),
        ]]));
        assert_eq!(count(&func, is_reload), 0);
        assert_eq!(count(&func, is_blend), 2);
    }

    #[test]
    fn alloca_of_other_width_is_ignored() {
        let mut input = function(vec![vec![
            Instruction::Alloca {
                dest: Value(20),
                ty: IrType::Ptr,
                size: 16,
                align: 16,
                volatile: false,
                semantic_volatile: false,
            },
            lane_store(12, 20, IrType::U32),
        ]]);
        let before = format!("{:?}", input.blocks);
        lane_patch_in_function(&mut input);
        assert_eq!(format!("{:?}", input.blocks), before);
    }
}
