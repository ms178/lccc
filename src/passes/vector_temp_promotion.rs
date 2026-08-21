//! Late cleanup for pointer-backed SIMD values.
//!
//! LCCC's C intrinsic lowering represents a 128/256-bit vector value by the
//! address of a stack home.  Source assignments consequently tend to leave
//! this shape in late IR:
//!
//! ```text
//!   %tmp = alloca 32, align 32
//!   intrinsic op(...), dest_ptr = %tmp
//!   memcpy %var, %tmp, 32
//! ```
//!
//! This module performs three related, deliberately conservative cleanups:
//!
//! 1. write a full-width intrinsic result directly to `%var` and remove the
//!    temporary/copy;
//! 2. forward a direct SIMD load's source into a compatible pointer-backed
//!    vector consumer when intervening memory effects cannot change it;
//! 3. relax unobservable alignment above 16 bytes to the cheapest alignment
//!    required by the remaining uses.  Most vector homes become ordinary
//!    stack slots; operations such as `movntdq` retain their mandatory 16-byte
//!    alignment without paying the >16-byte runtime-addressing cost.
//!
//! Correctness is fail-closed.  Unequal SSA IDs are not an alias proof,
//! pointer-derived aliases are followed, atomics/fences/volatile accesses are
//! forwarding barriers, and only explicitly classified intrinsic arguments
//! may consume forwarded vector bytes.  Keeping the intrinsic signature table
//! here small and explicit is intentional: an unknown operation loses an
//! optimization rather than changing the program.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::instruction::Instruction;
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{BasicBlock, IrFunction, IrModule, Operand, Value};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug)]
struct AllocaInfo {
    block: usize,
    instruction: usize,
    size: usize,
    volatile: bool,
    semantic_volatile: bool,
}

fn map_capacity(func: &IrFunction) -> usize {
    let cached = func.next_value_id as usize;
    if cached != 0 {
        return cached.clamp(8, 4096);
    }
    func.blocks
        .iter()
        .map(|block| block.instructions.len())
        .sum::<usize>()
        .clamp(8, 4096)
}

fn collect_allocas(func: &IrFunction) -> FxHashMap<u32, AllocaInfo> {
    let mut allocas = FxHashMap::with_capacity_and_hasher(
        (map_capacity(func) / 8).max(8),
        Default::default(),
    );
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for (instruction_idx, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Alloca {
                dest,
                size,
                volatile,
                semantic_volatile,
                ..
            } = inst
            {
                allocas.insert(
                    dest.0,
                    AllocaInfo {
                        block: block_idx,
                        instruction: instruction_idx,
                        size: *size,
                        volatile: *volatile,
                        semantic_volatile: *semantic_volatile,
                    },
                );
            }
        }
    }
    allocas
}

/// Direct, full-width loads for which `args[0]` denotes exactly the bytes
/// written to `dest_ptr`.  Offset, masked and partial loads are excluded.
fn direct_vector_load_width(op: IntrinsicOp) -> Option<usize> {
    match op {
        IntrinsicOp::Loaddqu => Some(16),
        IntrinsicOp::Loadu256
        | IntrinsicOp::Load256
        | IntrinsicOp::LoaduPs256
        | IntrinsicOp::LoaduPd256 => Some(32),
        _ => None,
    }
}

/// Width of a pointer-backed vector operand at an intrinsic argument position.
///
/// This is both a type-safety gate for load forwarding and part of alignment
/// relaxation.  Address operands (for example the base of `LoadF64x4`) are
/// deliberately not vector-value operands even though the backend also reads
/// memory through them.  Scalar/immediate positions are likewise excluded.
fn pointer_vector_arg_width(op: IntrinsicOp, index: usize) -> Option<usize> {
    use IntrinsicOp as O;

    if let Some(width) = direct_vector_load_width(op) {
        return (index == 0).then_some(width);
    }
    match op {
        // Mixed-width AVX lane construction/casts.
        O::Broadcast128to256 | O::Zext128to256 => {
            return (index == 0).then_some(16);
        }
        O::Cast256to128 => return (index == 0).then_some(32),
        O::Insert128to256 => {
            return match index {
                0 => Some(32),
                1 => Some(16),
                _ => None,
            };
        }
        // 128->256-bit widening extensions read a 128-bit source.
        O::Pmovzxbw256 | O::Pmovzxbd256 | O::Pmovzxwd256
        | O::Pmovsxbw256 | O::Pmovsxbd256 | O::Pmovsxwd256 => {
            return (index == 0).then_some(16);
        }
        _ => {}
    }

    let width = match op {
        // 128-bit stores and scalar extract/reduction consumers.
        O::Storedqu | O::Storeldi128 | O::Movntdq | O::Movntpd
        | O::Pmovmskb128 | O::Cvtsi128Si32 | O::Cvtsi128Si64
        | O::HorizontalAddF64x2 | O::HorizontalAddI32x4
        | O::Pabsb128 | O::Pabsw128 | O::Pabsd128
        | O::Pmovzxbw128 | O::Pmovzxwd128 | O::Aesimc128 => {
            return (index == 0).then_some(16);
        }

        // Unary 128-bit vector + immediate forms.
        O::Aeskeygenassist128 | O::Pslldqi128 | O::Psrldqi128
        | O::Psllqi128 | O::Psrlqi128 | O::Psllwi128 | O::Psrlwi128
        | O::Psrawi128 | O::Psradi128 | O::Pslldi128 | O::Psrldi128
        | O::Pshufd128 | O::Pshuflw128 | O::Pshufhw128
        | O::Pextrw128 | O::Pextrd128 | O::Pextrb128 | O::Pextrq128
        | O::Pinsrw128 | O::Pinsrd128 | O::Pinsrb128 | O::Pinsrq128 => {
            return (index == 0).then_some(16);
        }

        // Binary 128-bit vector operations.  Operations with an immediate
        // have that immediate after the two vector operands.
        O::Pcmpeqb128 | O::Pcmpeqd128 | O::Psubusb128 | O::Psubsb128
        | O::Por128 | O::Pand128 | O::Pxor128
        | O::AddPs128 | O::SubPs128 | O::MulPs128
        | O::AddPd128 | O::SubPd128 | O::MulPd128
        | O::Pmuludq128 | O::Pmuldq128 | O::Pmulld128
        | O::Paddw128 | O::Psubw128 | O::Paddb128 | O::Psubb128
        | O::Psubusw128 | O::Psadbw128 | O::Pmullw128
        | O::Pmaddubsw128 | O::Phaddw128 | O::Phaddd128
        | O::Pshufb128 | O::Palignr128 | O::Pmaxub128 | O::Pminub128
        | O::Pblendw128 | O::Pclmulqdq128
        | O::Aesenc128 | O::Aesenclast128 | O::Aesdec128 | O::Aesdeclast128
        | O::Psllw128 | O::Psrlw128 | O::Pmulhw128 | O::Pmaddwd128
        | O::Pcmpgtw128 | O::Pcmpgtb128 | O::Paddd128 | O::Psubd128
        | O::Packssdw128 | O::Packsswb128 | O::Packuswb128
        | O::Punpcklbw128 | O::Punpckhbw128 | O::Punpcklwd128 | O::Punpckhwd128
        | O::Paddusb128 | O::Paddsb128 | O::Paddusw128 | O::Paddsw128
        | O::Psubsw128 | O::Pandn128 | O::Pcmpeqw128 | O::Pcmpgtd128
        | O::Pavgb128 | O::Pavgw128 | O::Pminsw128 | O::Pmaxsw128
        | O::Pmulhuw128 | O::Paddq128 | O::Psubq128
        | O::Punpckldq128 | O::Punpckhdq128 | O::Punpcklqdq128
        | O::Punpckhqdq128 | O::AddF64x2 | O::MulF64x2 | O::AddI32x4 => 16,

        // PBLENDVB consumes state, true value and mask as vectors.
        O::Pblendvb128 => return (index < 3).then_some(16),

        // 256-bit stores and unary consumers.
        O::Storeu256 | O::Store256 | O::StoreuPs256 | O::StoreuPd256
        | O::Pmovmskb256 | O::HorizontalAddF64x4 | O::HorizontalAddI32x8
        | O::Pabsb256 | O::Pabsw256 | O::Pabsd256
        | O::Psllidi256 | O::Psrlidi256 | O::Psllwi256 | O::Psrlwi256
        | O::Pshufd256 | O::Pslldqi256 | O::Psrldqi256
        | O::Psllqi256 | O::Psrlqi256 | O::Psrawi256 | O::Psradi256
        | O::Permute4x64 | O::Extracti128 => {
            return (index == 0).then_some(32);
        }

        // Binary 256-bit vector operations.
        O::Paddb256 | O::Paddw256 | O::Paddd256
        | O::Psubb256 | O::Psubw256 | O::Psubusw256
        | O::Psadbw256 | O::Pmaddubsw256 | O::Pmaddwd256
        | O::Pcmpeqb256 | O::Pcmpgtb256 | O::Pshufb256
        | O::Pmaxub256 | O::Pminub256 | O::Pxor256 | O::Por256 | O::Pand256
        | O::Pmulld256 | O::Psubd256 | O::Paddq256 | O::Psubq256
        | O::Pandn256 | O::Pcmpeqd256 | O::Pcmpeqq256
        | O::Pcmpgtd256 | O::Pcmpgtq256
        | O::AddPs256 | O::SubPs256 | O::MulPs256
        | O::AddPd256 | O::SubPd256 | O::MulPd256
        | O::Punpcklbw256 | O::Punpckhbw256 | O::Punpcklwd256
        | O::Punpckhwd256 | O::Punpckldq256 | O::Punpckhdq256
        | O::Punpcklqdq256 | O::Punpckhqdq256
        | O::Pmullw256 | O::Pmulhw256 | O::Pminsd256 | O::Pmaxsd256
        | O::Packssdw256 | O::Packuswb256 | O::Phaddw256 | O::Phaddd256
        | O::Pmuludq256 | O::AddF64x4 | O::MulF64x4 | O::AddI32x8 => 32,

        // Two vector operands followed by an immediate/control operand.
        O::Permute2x128 => 32,

        // Index vector + data vector.
        O::Permutevar8x32 => 32,

        // FMA memory forms: only the B vector is a pointer-backed vector
        // value; A is a scalar address and dest_ptr is the accumulator.
        O::FmaF64x2 => return (index == 1).then_some(16),
        O::FmaF64x4 => return (index == 1).then_some(32),
        O::FmaF64x2Hoisted => return (index == 0).then_some(16),
        O::FmaF64x4Hoisted => return (index == 0).then_some(32),

        _ => return None,
    };
    (index < 2).then_some(width)
}

/// Minimum stack alignment required when an intrinsic argument refers to an
/// alloca.  `Some(0)` means the backend uses an unaligned-safe access; `None`
/// means that this pass cannot prove the use safe.  Address-valued scalar or
/// immediate positions therefore fail closed rather than being mistaken for
/// vector data.
fn intrinsic_arg_required_alignment(op: IntrinsicOp, index: usize) -> Option<usize> {
    use IntrinsicOp as O;
    if pointer_vector_arg_width(op, index).is_some() {
        return Some(0);
    }
    let safe = match op {
        // Partial load: eight bytes are read and the upper half is zeroed.  It
        // is alignment-safe but intentionally not forwardable as a 16-byte load.
        O::Loadldi128 => index == 0,

        // Legacy vectorizer loads use base + byte offset.  Only the base is an
        // address; accepting the offset would hide an observable pointer-to-int use.
        O::LoadF64x4 | O::LoadF64x2 | O::LoadI32x8 | O::LoadI32x4 => index == 0,

        // All accesses in these vectorizer memory forms are scalar/unaligned.
        O::FmaF64x2 | O::FmaF64x4 => index < 2,
        O::FmaF64x2Hoisted | O::FmaF64x4Hoisted => index == 0,
        O::FmaF64x4SIB => index < 3,
        O::BroadcastLoadF64 | O::Rdtscp | O::Clflush => index == 0,
        _ => false,
    };
    safe.then_some(0)
}

fn intrinsic_dest_required_alignment(
    op: IntrinsicOp,
    has_result: bool,
    alloca_size: usize,
) -> Option<usize> {
    use IntrinsicOp as O;

    // Compiler-created vector result homes are emitted through unaligned-safe
    // stores.  Exact width also rejects partial/RMW operations accidentally
    // listed by a broad intrinsic family query.
    if has_result
        && op.vector_result_width().map(|width| width as usize) == Some(alloca_size)
    {
        return Some(0);
    }

    match op {
        O::Storedqu
        | O::Storeldi128
        | O::Storeu256
        | O::Store256
        | O::StoreuPs256
        | O::StoreuPd256
        | O::Movnti
        | O::Movnti64
        | O::FmaF64x2
        | O::FmaF64x2Hoisted
        | O::FmaF64x4
        | O::FmaF64x4Hoisted
        | O::FmaF64x4SIB => Some(0),
        // x86 non-temporal vector stores fault unless the destination is
        // 16-byte aligned.  Alignment 16 is still a direct stack slot and
        // therefore avoids the expensive runtime alignment sequence.
        O::Movntdq | O::Movntpd => Some(16),
        _ => None,
    }
}

fn instruction_uses_any(inst: &Instruction, values: &FxHashSet<u32>) -> bool {
    let mut found = false;
    inst.for_each_used_value(|value| found |= values.contains(&value));
    found
}

fn operand_is_alias(op: &Operand, aliases: &FxHashSet<u32>) -> bool {
    matches!(op, Operand::Value(value) if aliases.contains(&value.0))
}

fn operand_is_value(op: Operand, value: Value) -> bool {
    matches!(op, Operand::Value(candidate) if candidate == value)
}

fn add_alias_edge(graph: &mut FxHashMap<u32, Vec<u32>>, source: u32, dest: u32) {
    graph.entry(source).or_default().push(dest);
}

/// Build exact pointer-preserving and conservative address-influence graphs.
/// The latter is only used to block store motion, never to prove no-alias.
fn build_alias_graphs(
    func: &IrFunction,
) -> (FxHashMap<u32, Vec<u32>>, FxHashMap<u32, Vec<u32>>) {
    let capacity = (map_capacity(func) / 4).max(8);
    let mut exact = FxHashMap::with_capacity_and_hasher(capacity, Default::default());
    let mut conservative = FxHashMap::with_capacity_and_hasher(capacity, Default::default());

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GetElementPtr { dest, base, .. } => {
                    add_alias_edge(&mut exact, base.0, dest.0);
                    add_alias_edge(&mut conservative, base.0, dest.0);
                    // An address used as a byte offset can still influence the
                    // resulting pointer after pointer-to-integer folding.
                    if let Instruction::GetElementPtr {
                        offset: Operand::Value(offset),
                        ..
                    } = inst
                    {
                        add_alias_edge(&mut conservative, offset.0, dest.0);
                    }
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Value(source),
                } => {
                    add_alias_edge(&mut exact, source.0, dest.0);
                    add_alias_edge(&mut conservative, source.0, dest.0);
                }
                Instruction::Cast {
                    dest,
                    src: Operand::Value(source),
                    from_ty,
                    to_ty,
                } => {
                    if *from_ty == IrType::Ptr && *to_ty == IrType::Ptr {
                        add_alias_edge(&mut exact, source.0, dest.0);
                    }
                    add_alias_edge(&mut conservative, source.0, dest.0);
                }
                Instruction::Phi { dest, ty, incoming } => {
                    for (operand, _) in incoming {
                        if let Operand::Value(source) = operand {
                            if *ty == IrType::Ptr {
                                add_alias_edge(&mut exact, source.0, dest.0);
                            }
                            add_alias_edge(&mut conservative, source.0, dest.0);
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
                    for operand in [true_val, false_val] {
                        if let Operand::Value(source) = operand {
                            if *ty == IrType::Ptr {
                                add_alias_edge(&mut exact, source.0, dest.0);
                            }
                            add_alias_edge(&mut conservative, source.0, dest.0);
                        }
                    }
                    if let Operand::Value(source) = cond {
                        add_alias_edge(&mut conservative, source.0, dest.0);
                    }
                }
                Instruction::BinOp { dest, .. } | Instruction::UnaryOp { dest, .. } => {
                    inst.for_each_used_value(|source| {
                        add_alias_edge(&mut conservative, source, dest.0)
                    });
                }
                _ => {}
            }
        }
    }
    (exact, conservative)
}

fn alias_closure(root: u32, graph: &FxHashMap<u32, Vec<u32>>) -> FxHashSet<u32> {
    let mut aliases = FxHashSet::with_capacity_and_hasher(8, Default::default());
    let mut work = VecDeque::with_capacity(8);
    aliases.insert(root);
    work.push_back(root);
    while let Some(value) = work.pop_front() {
        if let Some(successors) = graph.get(&value) {
            for &successor in successors {
                if aliases.insert(successor) {
                    work.push_back(successor);
                }
            }
        }
    }
    aliases
}

struct DestinationFacts {
    aliases: FxHashSet<u32>,
    escaped: bool,
}

fn analyze_destination(
    func: &IrFunction,
    root: u32,
    conservative_graph: &FxHashMap<u32, Vec<u32>>,
) -> DestinationFacts {
    let aliases = alias_closure(root, conservative_graph);
    let mut escaped = false;

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                // The alloca address is stored as data and may subsequently be
                // recovered under an unrelated SSA ID.
                Instruction::Store { val, .. } => escaped |= operand_is_alias(val, &aliases),
                Instruction::AtomicStore { val, .. }
                | Instruction::AtomicRmw { val, .. } => {
                    escaped |= operand_is_alias(val, &aliases)
                }
                Instruction::AtomicCmpxchg {
                    expected, desired, ..
                } => {
                    escaped |= operand_is_alias(expected, &aliases)
                        || operand_is_alias(desired, &aliases)
                }
                Instruction::Call { .. }
                | Instruction::CallIndirect { .. }
                | Instruction::InlineAsm { .. }
                | Instruction::StackRestore { .. } => {
                    escaped |= instruction_uses_any(inst, &aliases)
                }
                _ => {}
            }
        }
        block
            .terminator
            .for_each_used_value(|value| escaped |= aliases.contains(&value));
    }

    DestinationFacts { aliases, escaped }
}

fn destination_unobserved_between(
    instructions: &[Instruction],
    from: usize,
    to: usize,
    facts: &DestinationFacts,
) -> bool {
    if facts.escaped {
        return false;
    }
    if from >= to || from >= instructions.len() {
        return true;
    }
    let end = to.min(instructions.len());
    for inst in &instructions[from..end] {
        if instruction_uses_any(inst, &facts.aliases)
            || matches!(
                inst,
                Instruction::Call { .. }
                    | Instruction::CallIndirect { .. }
                    | Instruction::InlineAsm { .. }
                    | Instruction::StackRestore { .. }
            )
        {
            return false;
        }
    }
    true
}

/// Remove instruction indices while maintaining BasicBlock's parallel source
/// span table.  The input is normalized here so callers can cheaply append
/// removals from independent rewrites.
fn remove_instructions(block: &mut BasicBlock, removed: &mut Vec<usize>) {
    if removed.is_empty() {
        return;
    }
    removed.sort_unstable();
    removed.dedup();
    debug_assert!(removed.last().is_none_or(|&index| index < block.instructions.len()));

    let instruction_count = block.instructions.len();
    let spans_in_lockstep = block.source_spans.len() == instruction_count;
    if !block.source_spans.is_empty() && !spans_in_lockstep {
        // Stale line mappings are worse than absent mappings.
        block.source_spans.clear();
    }

    let mut next_removed = 0usize;
    let mut index = 0usize;
    block.instructions.retain(|_| {
        let drop = removed.get(next_removed).copied() == Some(index);
        next_removed += usize::from(drop);
        index += 1;
        !drop
    });

    if spans_in_lockstep {
        let mut next_removed = 0usize;
        let mut index = 0usize;
        block.source_spans.retain(|_| {
            let drop = removed.get(next_removed).copied() == Some(index);
            next_removed += usize::from(drop);
            index += 1;
            !drop
        });
    }
}

#[derive(Clone, Copy, Debug)]
struct Promotion {
    block: usize,
    producer: usize,
    memcpy: usize,
    tmp: u32,
    destination: u32,
}

/// Promote full-width intrinsic result temporaries into copy destinations.
pub(crate) fn promote_vector_temps(module: &mut IrModule) -> usize {
    module
        .functions
        .iter_mut()
        .filter(|func| !func.is_declaration && !func.blocks.is_empty())
        .map(promote_in_function)
        .sum()
}

fn promote_in_function(func: &mut IrFunction) -> usize {
    let allocas = collect_allocas(func);
    if allocas.is_empty() {
        return 0;
    }

    let mut uses = FxHashMap::with_capacity_and_hasher(allocas.len(), Default::default());
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|value| {
                if allocas.contains_key(&value) {
                    *uses.entry(value).or_insert(0usize) += 1;
                }
            });
        }
        block.terminator.for_each_used_value(|value| {
            if allocas.contains_key(&value) {
                *uses.entry(value).or_insert(0usize) += 1;
            }
        });
    }

    let (_, conservative_graph) = build_alias_graphs(func);
    let mut destination_facts: FxHashMap<u32, DestinationFacts> =
        FxHashMap::with_capacity_and_hasher(8, Default::default());
    let mut promotions = Vec::new();

    for (block_idx, block) in func.blocks.iter().enumerate() {
        let mut producer_at = FxHashMap::with_capacity_and_hasher(4, Default::default());
        for (instruction_idx, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Intrinsic {
                    dest: Some(_),
                    op,
                    dest_ptr: Some(tmp),
                    ..
                } => {
                    let Some(info) = allocas.get(&tmp.0) else {
                        continue;
                    };
                    if op.vector_result_width().map(|width| width as usize) == Some(info.size)
                        && !info.volatile
                        && !info.semantic_volatile
                    {
                        producer_at.insert(tmp.0, instruction_idx);
                    }
                }
                Instruction::Memcpy { dest, src, size } => {
                    let Some(&producer) = producer_at.get(&src.0) else {
                        continue;
                    };
                    let (Some(tmp), Some(destination)) =
                        (allocas.get(&src.0), allocas.get(&dest.0))
                    else {
                        continue;
                    };
                    if src == dest
                        || uses.get(&src.0) != Some(&2)
                        || tmp.size != *size
                        || destination.size < *size
                        || tmp.volatile
                        || tmp.semantic_volatile
                        || destination.volatile
                        || destination.semantic_volatile
                    {
                        continue;
                    }

                    let facts = destination_facts.entry(dest.0).or_insert_with(|| {
                        analyze_destination(func, dest.0, &conservative_graph)
                    });
                    if destination_unobserved_between(
                        &block.instructions,
                        producer + 1,
                        instruction_idx,
                        facts,
                    ) {
                        promotions.push(Promotion {
                            block: block_idx,
                            producer,
                            memcpy: instruction_idx,
                            tmp: src.0,
                            destination: dest.0,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    if promotions.is_empty() {
        return 0;
    }

    let mut removals = vec![Vec::new(); func.blocks.len()];
    let mut applied = 0usize;
    for promotion in promotions {
        let inst = &mut func.blocks[promotion.block].instructions[promotion.producer];
        match inst {
            Instruction::Intrinsic {
                dest_ptr: Some(pointer),
                ..
            } if pointer.0 == promotion.tmp => {
                *pointer = Value(promotion.destination);
            }
            _ => {
                debug_assert!(false, "vector-temp producer changed during planning");
                continue;
            }
        }
        removals[promotion.block].push(promotion.memcpy);
        let tmp = allocas[&promotion.tmp];
        removals[tmp.block].push(tmp.instruction);
        applied += 1;
    }

    for (block, removed) in func.blocks.iter_mut().zip(&mut removals) {
        remove_instructions(block, removed);
    }
    applied
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerRoot {
    Alloca(u32),
    // Distinct global symbol names are not an alias proof because ELF aliases
    // can bind multiple names to one object.
    Global,
    Param { index: usize, noalias: bool },
}

fn common_root<'a>(
    operands: impl IntoIterator<Item = &'a Operand>,
    roots: &FxHashMap<u32, PointerRoot>,
) -> Option<PointerRoot> {
    let mut common = None;
    for operand in operands {
        let Operand::Value(value) = operand else {
            return None;
        };
        let root = *roots.get(&value.0)?;
        if common.is_some_and(|old| old != root) {
            return None;
        }
        common = Some(root);
    }
    common
}

/// Compute exact roots only through operations that preserve pointer object
/// identity.  Unknown roots may alias anything.
fn pointer_roots(func: &IrFunction) -> FxHashMap<u32, PointerRoot> {
    let mut roots = FxHashMap::with_capacity_and_hasher(map_capacity(func), Default::default());
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { dest, .. } => {
                    roots.insert(dest.0, PointerRoot::Alloca(dest.0));
                }
                Instruction::GlobalAddr { dest, .. } => {
                    roots.insert(dest.0, PointerRoot::Global);
                }
                Instruction::ParamRef {
                    dest,
                    param_idx,
                    ty: IrType::Ptr,
                } => {
                    roots.insert(
                        dest.0,
                        PointerRoot::Param {
                            index: *param_idx,
                            noalias: func.params.get(*param_idx).is_some_and(|param| param.noalias),
                        },
                    );
                }
                _ => {}
            }
        }
    }

    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let derived = match inst {
                    Instruction::GetElementPtr { dest, base, .. } => {
                        roots.get(&base.0).copied().map(|root| (dest.0, root))
                    }
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(source),
                    } => roots.get(&source.0).copied().map(|root| (dest.0, root)),
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(source),
                        from_ty: IrType::Ptr,
                        to_ty: IrType::Ptr,
                    } => roots.get(&source.0).copied().map(|root| (dest.0, root)),
                    // Lowering represents pointer arithmetic as integer-width
                    // Add/Sub after the base pointer has entered IR.  In a
                    // defined C program, adding an integer offset preserves the
                    // base object; adding two pointers is not valid C.  Require
                    // exactly one known root so pointer differences and mixed
                    // roots never become an alias proof.
                    Instruction::BinOp {
                        dest,
                        op: crate::ir::reexports::IrBinOp::Add,
                        lhs,
                        rhs,
                        ..
                    } => match (lhs, rhs) {
                        (Operand::Value(lhs), Operand::Value(rhs)) => {
                            match (roots.get(&lhs.0), roots.get(&rhs.0)) {
                                (Some(root), None) | (None, Some(root)) => {
                                    Some((dest.0, *root))
                                }
                                _ => None,
                            }
                        }
                        (Operand::Value(value), Operand::Const(_))
                        | (Operand::Const(_), Operand::Value(value)) => {
                            roots.get(&value.0).copied().map(|root| (dest.0, root))
                        }
                        _ => None,
                    },
                    Instruction::BinOp {
                        dest,
                        op: crate::ir::reexports::IrBinOp::Sub,
                        lhs: Operand::Value(base),
                        rhs,
                        ..
                    } if !matches!(rhs, Operand::Value(value) if roots.contains_key(&value.0)) => {
                        roots.get(&base.0).copied().map(|root| (dest.0, root))
                    }
                    Instruction::Select {
                        dest,
                        true_val,
                        false_val,
                        ty: IrType::Ptr,
                        ..
                    } => common_root([true_val, false_val], &roots)
                        .map(|root| (dest.0, root)),
                    Instruction::Phi {
                        dest,
                        incoming,
                        ty: IrType::Ptr,
                    } => common_root(incoming.iter().map(|(operand, _)| operand), &roots)
                        .map(|root| (dest.0, root)),
                    _ => None,
                };
                if let Some((dest, root)) = derived {
                    if let std::collections::hash_map::Entry::Vacant(entry) = roots.entry(dest) {
                        entry.insert(root);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    roots
}

fn roots_proven_distinct(lhs: PointerRoot, rhs: PointerRoot) -> bool {
    if lhs == rhs {
        return false;
    }
    match (lhs, rhs) {
        (PointerRoot::Alloca(_), _) | (_, PointerRoot::Alloca(_)) => true,
        (
            PointerRoot::Param { noalias: lhs, .. },
            PointerRoot::Param { noalias: rhs, .. },
        ) => lhs || rhs,
        (PointerRoot::Param { noalias, .. }, PointerRoot::Global)
        | (PointerRoot::Global, PointerRoot::Param { noalias, .. }) => noalias,
        (PointerRoot::Global, PointerRoot::Global) => false,
    }
}

struct LocalAliasFacts {
    /// Vector-sized allocas whose address cannot be recovered through memory or
    /// opaque code.
    nonescaping: FxHashSet<u32>,
    /// Conservative address-influence relation: value -> possible alloca roots.
    owners: FxHashMap<u32, Vec<u32>>,
}

fn local_alias_facts(
    func: &IrFunction,
    allocas: &FxHashMap<u32, AllocaInfo>,
    conservative_graph: &FxHashMap<u32, Vec<u32>>,
) -> LocalAliasFacts {
    let roots: Vec<u32> = allocas
        .iter()
        .filter_map(|(&value, info)| (info.size == 16 || info.size == 32).then_some(value))
        .collect();
    let mut nonescaping: FxHashSet<u32> = roots.iter().copied().collect();
    let mut owners = FxHashMap::with_capacity_and_hasher(roots.len() * 2, Default::default());
    for root in roots {
        for alias in alias_closure(root, conservative_graph) {
            owners.entry(alias).or_insert_with(Vec::new).push(root);
        }
    }

    let escape_value = |value: u32,
                        nonescaping: &mut FxHashSet<u32>,
                        owners: &FxHashMap<u32, Vec<u32>>| {
        if let Some(roots) = owners.get(&value) {
            for root in roots {
                nonescaping.remove(root);
            }
        }
    };
    let escape_operand = |operand: &Operand,
                          nonescaping: &mut FxHashSet<u32>,
                          owners: &FxHashMap<u32, Vec<u32>>| {
        if let Operand::Value(value) = operand {
            escape_value(value.0, nonescaping, owners);
        }
    };

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Store { val, .. } => {
                    escape_operand(val, &mut nonescaping, &owners)
                }
                Instruction::AtomicStore { val, .. }
                | Instruction::AtomicRmw { val, .. } => {
                    escape_operand(val, &mut nonescaping, &owners)
                }
                Instruction::AtomicCmpxchg {
                    expected, desired, ..
                } => {
                    escape_operand(expected, &mut nonescaping, &owners);
                    escape_operand(desired, &mut nonescaping, &owners);
                }
                Instruction::Call { .. }
                | Instruction::CallIndirect { .. }
                | Instruction::InlineAsm { .. }
                | Instruction::StackRestore { .. } => inst.for_each_used_value(|value| {
                    escape_value(value, &mut nonescaping, &owners)
                }),
                _ => {}
            }
        }
        block.terminator.for_each_used_value(|value| {
            escape_value(value, &mut nonescaping, &owners)
        });
    }
    LocalAliasFacts {
        nonescaping,
        owners,
    }
}

#[derive(Clone, Copy)]
struct AvailableLoad {
    source: Operand,
    width: usize,
    producer: (usize, usize),
}

/// Does a write through `pointer` possibly clobber the object identified by
/// `target`?  `write_root` is the pointer's own object root when derivable.
///
/// A target alloca whose address has never escaped can only be reached by a
/// pointer that is provably derived from it, so a write through an
/// unrelated/externally-derived pointer cannot touch it.  That refinement
/// keeps forwards alive across unrelated stores without ever assuming SSA
/// value inequality proves no-alias.
fn write_may_clobber(
    pointer: Value,
    target: Value,
    write_root: Option<PointerRoot>,
    roots: &FxHashMap<u32, PointerRoot>,
    local_aliases: &LocalAliasFacts,
) -> bool {
    let target_root = roots.get(&target.0).copied();
    match (target_root, write_root) {
        (Some(target_root), Some(write_root)) => {
            !roots_proven_distinct(target_root, write_root)
        }
        (Some(target_root), None) => match target_root {
            PointerRoot::Alloca(root) if local_aliases.nonescaping.contains(&root) => {
                local_aliases
                    .owners
                    .get(&pointer.0)
                    .is_some_and(|owners| owners.contains(&root))
            }
            _ => true,
        },
        // The target has no derivable root: fail closed.
        (None, _) => true,
    }
}

/// A write through `pointer` kills a pending `(slot, source)` forward when it
/// may clobber EITHER object: the slot a later consumer will read, or the
/// source the load copied.  Checking only the source (or only the slot) is
/// unsound — a reassignment `v = load(p); v = setzero(); use(v)` writes the
/// slot itself, and forwarding `use(v)` to `p` would read stale data.
fn invalidate_for_value_write(
    available: &mut FxHashMap<u32, AvailableLoad>,
    pointer: Value,
    roots: &FxHashMap<u32, PointerRoot>,
    local_aliases: &LocalAliasFacts,
) {
    let write_root = roots.get(&pointer.0).copied();
    available.retain(|slot, load| {
        // Slots are allocas, so roots[slot] is always Some; the check still
        // fails closed if that invariant ever breaks.
        if write_may_clobber(pointer, Value(*slot), write_root, roots, local_aliases) {
            return false;
        }
        match load.source {
            Operand::Value(source) => {
                !write_may_clobber(pointer, source, write_root, roots, local_aliases)
            }
            // A constant source denotes a fixed (absolute) address.  The only
            // write that PROVABLY cannot reach it is one confined to an
            // alloca-derived pointer: a fresh stack alloca can never equal a
            // compile-time address, and a write through a pointer derived
            // from one stays inside that alloca.  A plain parameter, a global
            // symbol, or an opaque pointer may legally target the absolute
            // address (the caller can pass it, a linker script can place a
            // symbol there, a load result is unknown), so those must
            // invalidate the forward — otherwise a promoted consumer
            // re-reads the address after an intervening store and observes
            // the wrong value.
            Operand::Const(_) => matches!(write_root, Some(PointerRoot::Alloca(_))),
        }
    });
}

/// Apply ordering and memory-write effects after an instruction's reads have
/// consumed available loads.
fn invalidate_for_memory_effects(
    inst: &Instruction,
    available: &mut FxHashMap<u32, AvailableLoad>,
    roots: &FxHashMap<u32, PointerRoot>,
    local_aliases: &LocalAliasFacts,
) {
    match inst {
        Instruction::Store { ptr, volatile, .. } => {
            if *volatile {
                available.clear();
            } else {
                invalidate_for_value_write(available, *ptr, roots, local_aliases);
            }
        }
        Instruction::Load { volatile: true, .. }
        | Instruction::AtomicLoad { .. }
        | Instruction::Fence { .. } => available.clear(),
        Instruction::Memcpy { dest, .. } => {
            invalidate_for_value_write(available, *dest, roots, local_aliases)
        }
        Instruction::VaArg { va_list_ptr, .. }
        | Instruction::VaStart { va_list_ptr }
        | Instruction::VaEnd { va_list_ptr } => {
            invalidate_for_value_write(available, *va_list_ptr, roots, local_aliases)
        }
        Instruction::VaCopy { dest_ptr, .. } => {
            invalidate_for_value_write(available, *dest_ptr, roots, local_aliases)
        }
        Instruction::VaArgStruct {
            dest_ptr,
            va_list_ptr,
            ..
        } => {
            invalidate_for_value_write(available, *dest_ptr, roots, local_aliases);
            invalidate_for_value_write(available, *va_list_ptr, roots, local_aliases);
        }
        Instruction::AtomicRmw { .. }
        | Instruction::AtomicInc { .. }
        | Instruction::AtomicCmpxchg { .. }
        | Instruction::AtomicStore { .. } => available.clear(),
        Instruction::Intrinsic {
            dest_ptr: Some(pointer),
            ..
        } => invalidate_for_value_write(available, *pointer, roots, local_aliases),
        Instruction::Intrinsic {
            op,
            dest_ptr: None,
            ..
        } => {
            // An intrinsic with no destination pointer either only reads
            // memory (pure) or produces a register-resident vector result and
            // touches no memory.  VecStoreI64x2 is the exception: it appears in
            // produces_vector_value for slot sizing but is the memory-form
            // vector store that writes through args[1] — it must be a full
            // barrier here.
            if !op.is_pure()
                && (!op.produces_vector_value() || matches!(op, IntrinsicOp::VecStoreI64x2))
            {
                available.clear();
            }
        }
        Instruction::Call { .. }
        | Instruction::CallIndirect { .. }
        | Instruction::InlineAsm { .. }
        | Instruction::PgoCounterInc { .. }
        | Instruction::StackRestore { .. } => available.clear(),
        Instruction::Alloca { .. }
        | Instruction::DynAlloca { .. }
        | Instruction::Load { .. }
        | Instruction::BinOp { .. }
        | Instruction::UnaryOp { .. }
        | Instruction::Cmp { .. }
        | Instruction::GetElementPtr { .. }
        | Instruction::Cast { .. }
        | Instruction::Copy { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::Phi { .. }
        | Instruction::LabelAddr { .. }
        | Instruction::GetReturnF64Second { .. }
        | Instruction::SetReturnF64Second { .. }
        | Instruction::GetReturnF32Second { .. }
        | Instruction::SetReturnF32Second { .. }
        | Instruction::GetReturnF128Second { .. }
        | Instruction::SetReturnF128Second { .. }
        | Instruction::Select { .. }
        | Instruction::StackSave { .. }
        | Instruction::ParamRef { .. } => {}
    }
}

#[derive(Clone, Copy)]
struct ReadSites {
    first: (usize, usize),
    multiple: bool,
}

fn single_read_sites(
    func: &IrFunction,
    allocas: &FxHashMap<u32, AllocaInfo>,
) -> FxHashMap<u32, ReadSites> {
    let mut sites = FxHashMap::with_capacity_and_hasher(allocas.len(), Default::default());
    let mut record = |value: u32, site: (usize, usize)| {
        if !allocas.contains_key(&value) {
            return;
        }
        sites
            .entry(value)
            .and_modify(|summary: &mut ReadSites| summary.multiple |= summary.first != site)
            .or_insert(ReadSites {
                first: site,
                multiple: false,
            });
    };

    for (block_idx, block) in func.blocks.iter().enumerate() {
        for (instruction_idx, inst) in block.instructions.iter().enumerate() {
            let site = (block_idx, instruction_idx);
            match inst {
                // A direct load writes dest_ptr; only its source arguments read
                // memory/value state.  Every other dest_ptr is conservatively
                // treated as a use because some intrinsics are read-modify-write.
                Instruction::Intrinsic {
                    op,
                    dest_ptr: Some(_),
                    args,
                    ..
                } if direct_vector_load_width(*op).is_some() => {
                    for arg in args {
                        if let Operand::Value(value) = arg {
                            record(value.0, site);
                        }
                    }
                }
                _ => inst.for_each_used_value(|value| record(value, site)),
            }
        }
        let terminator_site = (block_idx, usize::MAX);
        block
            .terminator
            .for_each_used_value(|value| record(value, terminator_site));
    }
    sites
}

#[derive(Clone, Copy)]
struct LoadPatch {
    consumer: usize,
    argument: usize,
    source: Operand,
    producer: (usize, usize),
}

/// Forward direct vector loads into compatible first consumers.
pub(crate) fn fuse_vector_loads(module: &mut IrModule) -> usize {
    module
        .functions
        .iter_mut()
        .filter(|func| !func.is_declaration && !func.blocks.is_empty())
        .map(fuse_in_function)
        .sum()
}

fn fuse_in_function(func: &mut IrFunction) -> usize {
    let allocas = collect_allocas(func);
    if allocas.is_empty() {
        return 0;
    }
    let roots = pointer_roots(func);
    let (_, conservative_graph) = build_alias_graphs(func);
    let local_aliases = local_alias_facts(func, &allocas, &conservative_graph);
    let read_sites = single_read_sites(func, &allocas);
    let mut plans: Vec<Vec<LoadPatch>> = vec![Vec::new(); func.blocks.len()];

    for (block_idx, block) in func.blocks.iter().enumerate() {
        let mut available: FxHashMap<u32, AvailableLoad> =
            FxHashMap::with_capacity_and_hasher(4, Default::default());
        for (instruction_idx, inst) in block.instructions.iter().enumerate() {
            let direct_load = match inst {
                Instruction::Intrinsic {
                    op,
                    dest_ptr: Some(destination),
                    args,
                    ..
                } => direct_vector_load_width(*op)
                    .map(|width| (*op, *destination, args, width)),
                _ => None,
            };

            if let Some((op, destination, args, width)) = direct_load {
                let mut effective_source = args.first().copied();
                if args.len() == 1 {
                    if let Operand::Value(value) = args[0] {
                        if let Some(load) = available.get(&value.0).copied() {
                            if pointer_vector_arg_width(op, 0) == Some(load.width)
                                && load.width == width
                                && !operand_is_value(load.source, destination)
                            {
                                plans[block_idx].push(LoadPatch {
                                    consumer: instruction_idx,
                                    argument: 0,
                                    source: load.source,
                                    producer: load.producer,
                                });
                                effective_source = Some(load.source);
                            }
                        }
                    }
                }

                // Every argument is a read, even when it was not forwardable.
                for arg in args {
                    if let Operand::Value(value) = arg {
                        available.remove(&value.0);
                    }
                }
                invalidate_for_value_write(
                    &mut available,
                    destination,
                    &roots,
                    &local_aliases,
                );

                if let Some(source) = effective_source {
                    if args.len() == 1
                        && !operand_is_value(source, destination)
                        // The legacy SSE path benefits from forwarding its
                        // first Loaddqu consumer even when a later consumer
                        // keeps the home live: doing so shortens GPR address
                        // lifetimes in adler32-style unpack chains.  The AVX
                        // backend instead keeps multi-use values in a vector
                        // register; partial forwarding there caused an extra
                        // reload and larger frame in measured intrinsic suites.
                        && (op == IntrinsicOp::Loaddqu
                            || read_sites
                                .get(&destination.0)
                                .is_some_and(|summary| !summary.multiple))
                        && allocas.get(&destination.0).is_some_and(|info| {
                            info.size == width && !info.volatile && !info.semantic_volatile
                        })
                    {
                        available.insert(
                            destination.0,
                            AvailableLoad {
                                source,
                                width,
                                producer: (block_idx, instruction_idx),
                            },
                        );
                    }
                }
                continue;
            }

            match inst {
                Instruction::Intrinsic { op, args, .. } => {
                    for (argument_idx, arg) in args.iter().enumerate() {
                        let Operand::Value(value) = arg else {
                            continue;
                        };
                        let Some(load) = available.get(&value.0).copied() else {
                            continue;
                        };
                        if pointer_vector_arg_width(*op, argument_idx) == Some(load.width) {
                            plans[block_idx].push(LoadPatch {
                                consumer: instruction_idx,
                                argument: argument_idx,
                                source: load.source,
                                producer: load.producer,
                            });
                        }
                    }
                    // The first instruction that reads a slot consumes the
                    // availability.  All compatible occurrences in this one
                    // instruction were considered above.
                    for arg in args {
                        if let Operand::Value(value) = arg {
                            available.remove(&value.0);
                        }
                    }
                }
                _ => inst.for_each_used_value(|value| {
                    available.remove(&value);
                }),
            }
            invalidate_for_memory_effects(inst, &mut available, &roots, &local_aliases);
        }
    }

    let patch_count = plans.iter().map(Vec::len).sum();
    if patch_count == 0 {
        return 0;
    }

    let mut forwarded_loads = FxHashSet::with_capacity_and_hasher(
        patch_count,
        Default::default(),
    );
    for (block, patches) in func.blocks.iter_mut().zip(&plans) {
        for patch in patches {
            let Some(Instruction::Intrinsic { args, .. }) =
                block.instructions.get_mut(patch.consumer)
            else {
                debug_assert!(false, "vector-load consumer changed during planning");
                continue;
            };
            let Some(argument) = args.get_mut(patch.argument) else {
                debug_assert!(false, "vector-load argument changed during planning");
                continue;
            };
            *argument = patch.source;
            forwarded_loads.insert(patch.producer);
        }
    }

    // Remove only loads which actually supplied a forwarded operand and whose
    // destination slot has no remaining reference.  Counting after patching is
    // important; direct-load dest_ptr is a write, not a read.
    let mut forwarded_slots = FxHashSet::with_capacity_and_hasher(
        forwarded_loads.len(),
        Default::default(),
    );
    for &(block_idx, instruction_idx) in &forwarded_loads {
        if let Instruction::Intrinsic {
            dest_ptr: Some(destination),
            ..
        } = &func.blocks[block_idx].instructions[instruction_idx]
        {
            forwarded_slots.insert(destination.0);
        }
    }

    // Also track the load's SSA result (dest), if present, so removal can only
    // fire when that register result has no reader either.  A future lowering
    // that exposes the register result must not be broken by load deletion.
    let mut load_result_slots: FxHashMap<u32, Option<u32>> = FxHashMap::default();
    for &(block_idx, instruction_idx) in &forwarded_loads {
        if let Instruction::Intrinsic {
            dest,
            dest_ptr: Some(destination),
            ..
        } = &func.blocks[block_idx].instructions[instruction_idx]
        {
            if let Some(result) = dest {
                load_result_slots.insert(destination.0, Some(result.0));
            }
        }
    }

    let mut remaining = FxHashMap::with_capacity_and_hasher(
        forwarded_slots.len(),
        Default::default(),
    );
    let mut remaining_results = FxHashMap::with_capacity_and_hasher(
        load_result_slots.len(),
        Default::default(),
    );
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Intrinsic {
                    op,
                    dest_ptr: Some(_),
                    args,
                    ..
                } if direct_vector_load_width(*op).is_some() => {
                    for arg in args {
                        if let Operand::Value(value) = arg {
                            if forwarded_slots.contains(&value.0) {
                                *remaining.entry(value.0).or_insert(0usize) += 1;
                            }
                        }
                    }
                }
                _ => inst.for_each_used_value(|value| {
                    if forwarded_slots.contains(&value) {
                        *remaining.entry(value).or_insert(0usize) += 1;
                    }
                    if load_result_slots.values().any(|slot| *slot == Some(value)) {
                        *remaining_results.entry(value).or_insert(0usize) += 1;
                    }
                }),
            }
        }
        block.terminator.for_each_used_value(|value| {
            if forwarded_slots.contains(&value) {
                *remaining.entry(value).or_insert(0usize) += 1;
            }
            if load_result_slots.values().any(|slot| *slot == Some(value)) {
                *remaining_results.entry(value).or_insert(0usize) += 1;
            }
        });
    }

    let mut removals = vec![Vec::new(); func.blocks.len()];
    for &(block_idx, instruction_idx) in &forwarded_loads {
        let Instruction::Intrinsic {
            op,
            dest,
            dest_ptr: Some(destination),
            args,
            ..
        } = &func.blocks[block_idx].instructions[instruction_idx]
        else {
            continue;
        };
        let result_dead = match (dest, load_result_slots.get(&destination.0)) {
            (Some(reg), Some(Some(_))) => {
                remaining_results.get(&reg.0).copied().unwrap_or(0) == 0
            }
            _ => true,
        };
        let removable = direct_vector_load_width(*op).is_some()
            && args.len() == 1
            && allocas.get(&destination.0).is_some_and(|info| {
                !info.volatile && !info.semantic_volatile
            })
            && remaining.get(&destination.0).copied().unwrap_or(0) == 0
            && result_dead;
        if removable {
            removals[block_idx].push(instruction_idx);
        }
    }
    for (block, removed) in func.blocks.iter_mut().zip(&mut removals) {
        remove_instructions(block, removed);
    }

    patch_count
}

fn remove_alias_owners(
    value: u32,
    owners: &FxHashMap<u32, Vec<u32>>,
    safe: &mut FxHashSet<u32>,
) {
    if let Some(roots) = owners.get(&value) {
        for root in roots {
            safe.remove(root);
        }
    }
}

fn remove_operand_owners(
    operand: &Operand,
    owners: &FxHashMap<u32, Vec<u32>>,
    safe: &mut FxHashSet<u32>,
) {
    if let Operand::Value(value) = operand {
        remove_alias_owners(value.0, owners, safe);
    }
}

/// Relax unobservable >16-byte stack alignment to the minimum required by uses.
pub(crate) fn downgrade_nonescaping_vector_align(module: &mut IrModule) -> usize {
    module
        .functions
        .iter_mut()
        .filter(|func| !func.is_declaration && !func.blocks.is_empty())
        .map(downgrade_in_function)
        .sum()
}

fn downgrade_in_function(func: &mut IrFunction) -> usize {
    let mut sizes = FxHashMap::with_capacity_and_hasher(8, Default::default());
    let mut required_align = FxHashMap::with_capacity_and_hasher(8, Default::default());
    let mut safe = FxHashSet::with_capacity_and_hasher(8, Default::default());
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                size,
                align,
                volatile,
                semantic_volatile,
                ..
            } = inst
            {
                if (*size == 16 || *size == 32)
                    && *align > 16
                    && !*volatile
                    && !*semantic_volatile
                {
                    sizes.insert(dest.0, *size);
                    required_align.insert(dest.0, 0usize);
                    safe.insert(dest.0);
                }
            }
        }
    }
    if safe.is_empty() {
        return 0;
    }

    // Map each exact derived pointer back to every candidate alloca it may
    // select.  A select/phi can therefore carry several owners without making
    // the unsafe single-root assumption used by many lightweight analyses.
    let (exact_graph, _) = build_alias_graphs(func);
    let candidates: Vec<u32> = safe.iter().copied().collect();
    let mut owners: FxHashMap<u32, Vec<u32>> = FxHashMap::with_capacity_and_hasher(
        candidates.len() * 2,
        Default::default(),
    );
    for root in candidates {
        for alias in alias_closure(root, &exact_graph) {
            owners.entry(alias).or_default().push(root);
        }
    }

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { .. }
                | Instruction::GlobalAddr { .. }
                | Instruction::LabelAddr { .. }
                | Instruction::StackSave { .. }
                | Instruction::Fence { .. }
                | Instruction::GetReturnF64Second { .. }
                | Instruction::GetReturnF32Second { .. }
                | Instruction::GetReturnF128Second { .. }
                | Instruction::ParamRef { .. }
                | Instruction::PgoCounterInc { .. } => {}

                // Dereferencing the object does not expose its address and all
                // scalar accesses remain naturally aligned at 16.
                Instruction::Load { .. } => {}
                Instruction::Store { val, .. } => {
                    // ptr is safe; storing the pointer itself as data is not.
                    remove_operand_owners(val, &owners, &mut safe);
                }
                Instruction::Memcpy { .. } => {}

                // Exact pointer-preserving derivations are checked at their
                // eventual use rather than rejected at the derivation itself.
                Instruction::GetElementPtr { offset, .. } => {
                    remove_operand_owners(offset, &owners, &mut safe);
                }
                Instruction::Copy { .. } => {}
                Instruction::Cast {
                    src,
                    from_ty: IrType::Ptr,
                    to_ty: IrType::Ptr,
                    ..
                } => {
                    let _ = src;
                }
                Instruction::Phi {
                    ty: IrType::Ptr, ..
                } => {}
                Instruction::Select {
                    cond,
                    ty: IrType::Ptr,
                    ..
                } => remove_operand_owners(cond, &owners, &mut safe),

                Instruction::Intrinsic {
                    dest,
                    op,
                    dest_ptr,
                    args,
                } => {
                    if let Some(pointer) = dest_ptr {
                        if let Some(roots) = owners.get(&pointer.0) {
                            for root in roots {
                                if !safe.contains(root) {
                                    continue;
                                }
                                if let Some(required) = intrinsic_dest_required_alignment(
                                    *op,
                                    dest.is_some(),
                                    sizes[root],
                                ) {
                                    let current = required_align.entry(*root).or_insert(0);
                                    *current = (*current).max(required);
                                } else {
                                    safe.remove(root);
                                }
                            }
                        }
                    }
                    for (index, arg) in args.iter().enumerate() {
                        let Operand::Value(value) = arg else {
                            continue;
                        };
                        if let Some(roots) = owners.get(&value.0) {
                            if let Some(required) = intrinsic_arg_required_alignment(*op, index) {
                                for root in roots {
                                    if safe.contains(root) {
                                        let current = required_align.entry(*root).or_insert(0);
                                        *current = (*current).max(required);
                                    }
                                }
                            } else {
                                for root in roots {
                                    safe.remove(root);
                                }
                            }
                        }
                    }
                }

                // Everything not explicitly proven alignment-insensitive fails
                // closed, including atomics, calls, asm and pointer arithmetic.
                _ => inst.for_each_used_value(|value| {
                    remove_alias_owners(value, &owners, &mut safe)
                }),
            }
        }
        block.terminator.for_each_used_value(|value| {
            remove_alias_owners(value, &owners, &mut safe)
        });
    }

    if safe.is_empty() {
        return 0;
    }
    let mut changed = 0usize;
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Alloca { dest, align, .. } = inst {
                if safe.contains(&dest.0) {
                    *align = required_align.get(&dest.0).copied().unwrap_or(0);
                    changed += 1;
                }
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::source::Span;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{
        AtomicOrdering, BlockId, IrBinOp, IrConst, IrParam, Terminator,
    };

    fn block(label: u32, instructions: Vec<Instruction>) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            source_spans: vec![Span::dummy(); instructions.len()],
            instructions,
            terminator: Terminator::Return(None),
        }
    }

    fn function(name: &str, blocks: Vec<BasicBlock>) -> IrFunction {
        let mut function = IrFunction::new(name.into(), IrType::I32, Vec::new(), false);
        function.blocks = blocks;
        function
    }

    fn alloca(dest: u32, size: usize, align: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size,
            align,
            volatile: false,
            semantic_volatile: false,
        }
    }

    fn intrinsic(
        dest: Option<u32>,
        op: IntrinsicOp,
        dest_ptr: Option<u32>,
        args: Vec<Operand>,
    ) -> Instruction {
        Instruction::Intrinsic {
            dest: dest.map(Value),
            op,
            dest_ptr: dest_ptr.map(Value),
            args,
        }
    }

    #[test]
    fn load_classifier_rejects_partial_and_offset_loads() {
        assert_eq!(direct_vector_load_width(IntrinsicOp::Loaddqu), Some(16));
        assert_eq!(direct_vector_load_width(IntrinsicOp::Loadu256), Some(32));
        assert_eq!(direct_vector_load_width(IntrinsicOp::Load256), Some(32));
        assert_eq!(direct_vector_load_width(IntrinsicOp::Loadldi128), None);
        assert_eq!(direct_vector_load_width(IntrinsicOp::LoadF64x4), None);
        assert_eq!(direct_vector_load_width(IntrinsicOp::MaskLoaduEpi8_256), None);
    }

    #[test]
    fn intrinsic_signature_distinguishes_vector_and_scalar_arguments() {
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::Pcmpeqb256, 0), Some(32));
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::Pcmpeqb256, 1), Some(32));
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::Pinsrd128, 0), Some(16));
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::Pinsrd128, 1), None);
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::SetEpi32_256, 0), None);
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::FmaF64x4, 0), None);
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::FmaF64x4, 1), Some(32));
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::Insert128to256, 0), Some(32));
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::Insert128to256, 1), Some(16));
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::Insert128to256, 2), None);
    }

    #[test]
    fn widening_extension_reads_a_128_bit_source() {
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::Pmovzxbw256, 0), Some(16));
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::Pmovsxwd256, 0), Some(16));
        assert_eq!(pointer_vector_arg_width(IntrinsicOp::Pmovzxbw256, 1), None);
    }

    #[test]
    fn compaction_preserves_source_span_lockstep() {
        let mut block = block(
            0,
            vec![
                alloca(0, 32, 32),
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(7)),
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Const(IrConst::I32(9)),
                },
            ],
        );
        let mut removed = vec![2, 0, 0];
        remove_instructions(&mut block, &mut removed);
        assert_eq!(block.instructions.len(), 1);
        assert_eq!(block.source_spans.len(), 1);
        assert!(matches!(
            block.instructions[0],
            Instruction::Copy { dest: Value(1), .. }
        ));
    }

    #[test]
    fn promotion_removes_entry_alloca_and_keeps_spans_synchronized() {
        let entry = block(0, vec![alloca(0, 32, 32), alloca(1, 32, 32)]);
        let body = block(
            1,
            vec![
                intrinsic(
                    Some(2),
                    IntrinsicOp::Pcmpeqb256,
                    Some(0),
                    vec![Operand::Value(Value(3)), Operand::Value(Value(4))],
                ),
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(0),
                    size: 32,
                },
            ],
        );
        let mut function = function("promote", vec![entry, body]);
        assert_eq!(promote_in_function(&mut function), 1);
        assert_eq!(function.blocks[0].instructions.len(), 1);
        assert_eq!(function.blocks[0].source_spans.len(), 1);
        assert_eq!(function.blocks[1].instructions.len(), 1);
        assert_eq!(function.blocks[1].source_spans.len(), 1);
        assert!(matches!(
            function.blocks[1].instructions[0],
            Instruction::Intrinsic { dest_ptr: Some(Value(1)), .. }
        ));
    }

    #[test]
    fn derived_destination_access_blocks_promotion() {
        let mut function = function(
            "alias_window",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    alloca(1, 32, 32),
                    Instruction::Copy {
                        dest: Value(5),
                        src: Operand::Value(Value(1)),
                    },
                    intrinsic(
                        Some(2),
                        IntrinsicOp::Pcmpeqb256,
                        Some(0),
                        vec![Operand::Value(Value(3)), Operand::Value(Value(4))],
                    ),
                    Instruction::Load {
                        dest: Value(6),
                        ptr: Value(5),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    Instruction::Memcpy {
                        dest: Value(1),
                        src: Value(0),
                        size: 32,
                    },
                ],
            )],
        );
        assert_eq!(promote_in_function(&mut function), 0);
    }

    #[test]
    fn stored_destination_address_blocks_promotion_even_outside_window() {
        let mut function = function(
            "escaped_destination",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    alloca(1, 32, 32),
                    Instruction::Store {
                        val: Operand::Value(Value(1)),
                        ptr: Value(9),
                        ty: IrType::Ptr,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    intrinsic(
                        Some(2),
                        IntrinsicOp::Pcmpeqb256,
                        Some(0),
                        vec![Operand::Value(Value(3)), Operand::Value(Value(4))],
                    ),
                    Instruction::Memcpy {
                        dest: Value(1),
                        src: Value(0),
                        size: 32,
                    },
                ],
            )],
        );
        assert_eq!(promote_in_function(&mut function), 0);
    }

    #[test]
    fn load_forwarding_rejects_aliasing_source_write() {
        let mut function = function(
            "clobber",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    intrinsic(
                        Some(1),
                        IntrinsicOp::Loadu256,
                        Some(0),
                        vec![Operand::Value(Value(10))],
                    ),
                    Instruction::Store {
                        val: Operand::Const(IrConst::I32(1)),
                        ptr: Value(10),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    intrinsic(
                        Some(2),
                        IntrinsicOp::Pcmpeqb256,
                        Some(20),
                        vec![Operand::Value(Value(0)), Operand::Value(Value(21))],
                    ),
                ],
            )],
        );
        assert_eq!(fuse_in_function(&mut function), 0);
        assert_eq!(function.blocks[0].instructions.len(), 4);
    }

    #[test]
    fn load_forwarding_keeps_candidates_across_proven_distinct_writes() {
        let mut function = function(
            "distinct",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    alloca(10, 32, 16),
                    alloca(11, 4, 4),
                    intrinsic(
                        Some(1),
                        IntrinsicOp::Loadu256,
                        Some(0),
                        vec![Operand::Value(Value(10))],
                    ),
                    Instruction::Store {
                        val: Operand::Const(IrConst::I32(1)),
                        ptr: Value(11),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    intrinsic(
                        Some(2),
                        IntrinsicOp::Pcmpeqb256,
                        Some(20),
                        vec![Operand::Value(Value(0)), Operand::Value(Value(21))],
                    ),
                ],
            )],
        );
        assert_eq!(fuse_in_function(&mut function), 1);
        assert_eq!(function.blocks[0].instructions.len(), 5);
        let Instruction::Intrinsic { args, .. } = &function.blocks[0].instructions[4] else {
            panic!("expected consumer");
        };
        assert!(matches!(args[0], Operand::Value(Value(10))));
    }

    #[test]
    fn constant_address_source_rejects_plain_parameter_write() {
        // A load whose source is an absolute (constant) address must NOT
        // survive a write through a plain parameter: the caller may pass
        // exactly that address, so the write can hit the source and a
        // promoted consumer would re-read post-store memory.
        let param = IrParam {
            ty: IrType::Ptr,
            noalias: false,
            struct_size: None,
            struct_align: None,
            struct_eightbyte_classes: Vec::new(),
            is_f128_sse: false,
            riscv_float_class: None,
        };
        let mut function = IrFunction::new("const_src_param".into(), IrType::I32, vec![param], false);
        function.blocks = vec![block(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(30),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                alloca(0, 32, 32),
                intrinsic(
                    Some(1),
                    IntrinsicOp::Loadu256,
                    Some(0),
                    vec![Operand::Const(IrConst::I64(4096))],
                ),
                Instruction::Store {
                    val: Operand::Const(IrConst::I32(1)),
                    ptr: Value(30),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                intrinsic(
                    Some(2),
                    IntrinsicOp::Pcmpeqb256,
                    Some(20),
                    vec![Operand::Value(Value(0)), Operand::Value(Value(21))],
                ),
            ],
        )];
        assert_eq!(fuse_in_function(&mut function), 0);
        let Instruction::Intrinsic { args, .. } =
            function.blocks[0].instructions.last().unwrap()
        else {
            panic!("expected consumer");
        };
        assert!(
            matches!(args[0], Operand::Value(Value(0))),
            "param write must invalidate a constant-address forward"
        );
    }

    #[test]
    fn constant_address_source_survives_alloca_confined_write() {
        // The one write that provably cannot reach an absolute address is a
        // write through a pointer derived from a stack alloca.  That forward
        // is the legitimate optimization and must survive.
        let mut function = function(
            "const_src_alloca",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    alloca(50, 4, 4),
                    intrinsic(
                        Some(1),
                        IntrinsicOp::Loadu256,
                        Some(0),
                        vec![Operand::Const(IrConst::I64(4096))],
                    ),
                    Instruction::Store {
                        val: Operand::Const(IrConst::I32(1)),
                        ptr: Value(50),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    intrinsic(
                        Some(2),
                        IntrinsicOp::Pcmpeqb256,
                        Some(20),
                        vec![Operand::Value(Value(0)), Operand::Value(Value(21))],
                    ),
                ],
            )],
        );
        assert_eq!(fuse_in_function(&mut function), 1);
        let Instruction::Intrinsic { args, .. } =
            function.blocks[0].instructions.last().unwrap()
        else {
            panic!("expected consumer");
        };
        assert!(
            matches!(args[0], Operand::Const(IrConst::I64(4096))),
            "alloca-confined write must keep the constant-address forward"
        );
    }

    #[test]
    fn intrinsic_write_to_slot_invalidates_pending_load() {
        // v = load(p); v = setzero(); use(v)  — the reassignment writes the
        // slot itself.  Forwarding use(v) to p would read stale data; the
        // slot-vs-write check must kill the forward.
        let mut function = function(
            "slot_reassign",
            vec![block(
                0,
                vec![
                    alloca(1, 32, 32), // load slot / variable
                    alloca(6, 32, 32), // consumer result slot
                    intrinsic(Some(3), IntrinsicOp::Loadu256, Some(1), vec![Operand::Value(Value(2))]),
                    intrinsic(Some(4), IntrinsicOp::Setzero256, Some(1), vec![]),
                    intrinsic(
                        Some(5),
                        IntrinsicOp::Paddb256,
                        Some(6),
                        vec![Operand::Value(Value(1)), Operand::Value(Value(8))],
                    ),
                ],
            )],
        );
        fuse_in_function(&mut function);
        let Instruction::Intrinsic { args, .. } =
            function.blocks[0].instructions.last().unwrap()
        else {
            panic!("expected consumer");
        };
        assert!(
            matches!(args[0], Operand::Value(Value(1))),
            "forward after slot reassignment must not reach the stale source"
        );
    }

    #[test]
    fn vec_store_memory_form_is_a_forwarding_barrier() {
        // VecStoreI64x2 (memory form, dest_ptr: None) writes through args[1].
        // It must not inherit the register-result exemption of
        // produces_vector_value.
        let mut function = function(
            "vec_store",
            vec![block(
                0,
                vec![
                    alloca(1, 32, 32),
                    alloca(6, 32, 32),
                    intrinsic(Some(3), IntrinsicOp::Loadu256, Some(1), vec![Operand::Value(Value(2))]),
                    intrinsic(
                        None,
                        IntrinsicOp::VecStoreI64x2,
                        None,
                        vec![
                            Operand::Value(Value(7)),
                            Operand::Value(Value(2)),
                            Operand::Value(Value(8)),
                        ],
                    ),
                    intrinsic(
                        Some(5),
                        IntrinsicOp::Paddb256,
                        Some(6),
                        vec![Operand::Value(Value(1)), Operand::Value(Value(9))],
                    ),
                ],
            )],
        );
        fuse_in_function(&mut function);
        let Instruction::Intrinsic { args, .. } =
            function.blocks[0].instructions.last().unwrap()
        else {
            panic!("expected consumer");
        };
        // The VecStore wrote through Value(2), the load's source: the forward
        // must have been killed.
        assert!(matches!(args[0], Operand::Value(Value(1))));
    }

    #[test]
    fn load_with_multiple_consumer_sites_is_not_partially_forwarded() {
        let mut function = function(
            "multi_consumer",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 0),
                    intrinsic(
                        Some(1),
                        IntrinsicOp::LoaduPs256,
                        Some(0),
                        vec![Operand::Value(Value(10))],
                    ),
                    intrinsic(
                        Some(2),
                        IntrinsicOp::AddPs256,
                        Some(20),
                        vec![Operand::Value(Value(0)), Operand::Value(Value(21))],
                    ),
                    intrinsic(
                        Some(3),
                        IntrinsicOp::MulPs256,
                        Some(22),
                        vec![Operand::Value(Value(0)), Operand::Value(Value(23))],
                    ),
                ],
            )],
        );
        assert_eq!(fuse_in_function(&mut function), 0);
        assert!(matches!(
            function.blocks[0].instructions[2],
            Instruction::Intrinsic { ref args, .. }
                if matches!(args[0], Operand::Value(Value(0)))
        ));
    }

    #[test]
    fn pointer_arithmetic_between_load_and_consumer_does_not_block_forwarding() {
        let mut function = function(
            "address_arithmetic",
            vec![block(
                0,
                vec![
                    alloca(14, 32, 0),
                    alloca(28, 32, 0),
                    alloca(4, 32, 0),
                    Instruction::Copy {
                        dest: Value(17),
                        src: Operand::Const(IrConst::I64(4)),
                    },
                    Instruction::BinOp {
                        dest: Value(18),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(4)),
                        rhs: Operand::Value(Value(17)),
                        ty: IrType::I64,
                    },
                    intrinsic(
                        Some(20),
                        IntrinsicOp::Loadu256,
                        Some(14),
                        vec![Operand::Value(Value(18))],
                    ),
                    Instruction::BinOp {
                        dest: Value(25),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(4)),
                        rhs: Operand::Value(Value(17)),
                        ty: IrType::I64,
                    },
                    intrinsic(Some(26), IntrinsicOp::Setzero256, Some(28), vec![]),
                    intrinsic(
                        Some(30),
                        IntrinsicOp::Pcmpeqb256,
                        Some(28),
                        vec![Operand::Value(Value(14)), Operand::Value(Value(25))],
                    ),
                ],
            )],
        );
        assert_eq!(fuse_in_function(&mut function), 1);
        let Instruction::Intrinsic { args, .. } = function.blocks[0].instructions.last().unwrap() else {
            panic!("expected consumer");
        };
        assert!(matches!(args[0], Operand::Value(Value(18))));
    }

    #[test]
    fn fence_and_atomic_load_are_forwarding_barriers() {
        for barrier in [
            Instruction::Fence {
                ordering: AtomicOrdering::SeqCst,
            },
            Instruction::AtomicLoad {
                dest: Value(30),
                ptr: Operand::Value(Value(31)),
                ty: IrType::I32,
                ordering: AtomicOrdering::Acquire,
            },
        ] {
            let mut function = function(
                "barrier",
                vec![block(
                    0,
                    vec![
                        alloca(0, 32, 32),
                        intrinsic(
                            Some(1),
                            IntrinsicOp::Loadu256,
                            Some(0),
                            vec![Operand::Value(Value(10))],
                        ),
                        barrier,
                        intrinsic(
                            Some(2),
                            IntrinsicOp::Pcmpeqb256,
                            Some(20),
                            vec![Operand::Value(Value(0)), Operand::Value(Value(21))],
                        ),
                    ],
                )],
            );
            assert_eq!(fuse_in_function(&mut function), 0);
        }
    }

    #[test]
    fn scalar_intrinsic_argument_is_never_forwarded() {
        let mut function = function(
            "scalar_arg",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    intrinsic(
                        Some(1),
                        IntrinsicOp::Loadu256,
                        Some(0),
                        vec![Operand::Value(Value(10))],
                    ),
                    intrinsic(
                        Some(2),
                        IntrinsicOp::SetEpi32_256,
                        Some(20),
                        vec![Operand::Value(Value(0))],
                    ),
                ],
            )],
        );
        assert_eq!(fuse_in_function(&mut function), 0);
    }

    #[test]
    fn alignment_relaxation_tracks_safe_gep_aliases() {
        let mut function = function(
            "align_gep",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    Instruction::GetElementPtr {
                        dest: Value(1),
                        base: Value(0),
                        offset: Operand::Const(IrConst::I64(8)),
                        ty: IrType::I8,
                    },
                    Instruction::Load {
                        dest: Value(2),
                        ptr: Value(1),
                        ty: IrType::I64,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                ],
            )],
        );
        assert_eq!(downgrade_in_function(&mut function), 1);
        assert!(matches!(
            function.blocks[0].instructions[0],
            Instruction::Alloca { align: 0, .. }
        ));
    }

    #[test]
    fn alignment_observed_as_integer_is_not_relaxed() {
        let mut function = function(
            "align_integer",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    Instruction::BinOp {
                        dest: Value(1),
                        op: IrBinOp::And,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I64(31)),
                        ty: IrType::I64,
                    },
                ],
            )],
        );
        assert_eq!(downgrade_in_function(&mut function), 0);
    }

    #[test]
    fn vectorizer_offset_cannot_hide_an_address_observation() {
        let mut function = function(
            "offset_address",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    intrinsic(
                        Some(2),
                        IntrinsicOp::LoadF64x4,
                        Some(3),
                        vec![Operand::Value(Value(4)), Operand::Value(Value(0))],
                    ),
                ],
            )],
        );
        assert_eq!(downgrade_in_function(&mut function), 0);
    }

    #[test]
    fn unaligned_store_destination_allows_alignment_relaxation() {
        let mut function = function(
            "storeu",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    alloca(1, 32, 16),
                    intrinsic(
                        None,
                        IntrinsicOp::Storeu256,
                        Some(0),
                        vec![Operand::Value(Value(1))],
                    ),
                ],
            )],
        );
        assert_eq!(downgrade_in_function(&mut function), 1);
        assert!(matches!(
            function.blocks[0].instructions[0],
            Instruction::Alloca { align: 0, .. }
        ));
    }

    #[test]
    fn non_temporal_store_retains_required_align16() {
        let mut function = function(
            "movnt",
            vec![block(
                0,
                vec![
                    alloca(0, 32, 32),
                    alloca(1, 16, 16),
                    intrinsic(
                        None,
                        IntrinsicOp::Movntdq,
                        Some(0),
                        vec![Operand::Value(Value(1))],
                    ),
                ],
            )],
        );
        assert_eq!(downgrade_in_function(&mut function), 1);
        assert!(matches!(
            function.blocks[0].instructions[0],
            Instruction::Alloca { align: 16, .. }
        ));
    }

    #[test]
    fn volatile_and_atomic_allocas_are_not_relaxed() {
        let mut volatile = function(
            "volatile",
            vec![block(
                0,
                vec![Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::Ptr,
                    size: 32,
                    align: 32,
                    volatile: false,
                    semantic_volatile: true,
                }],
            )],
        );
        assert_eq!(downgrade_in_function(&mut volatile), 0);

        let mut atomic = function(
            "atomic",
            vec![block(
                0,
                vec![
                    alloca(0, 16, 32),
                    Instruction::AtomicLoad {
                        dest: Value(1),
                        ptr: Operand::Value(Value(0)),
                        ty: IrType::I128,
                        ordering: AtomicOrdering::SeqCst,
                    },
                ],
            )],
        );
        assert_eq!(downgrade_in_function(&mut atomic), 0);
    }

    #[test]
    fn pointer_root_survives_lowered_integer_width_offset_arithmetic() {
        let param = IrParam {
            ty: IrType::Ptr,
            noalias: false,
            struct_size: None,
            struct_align: None,
            struct_eightbyte_classes: Vec::new(),
            is_f128_sse: false,
            riscv_float_class: None,
        };
        let mut function = IrFunction::new("pointer_add".into(), IrType::I32, vec![param], false);
        function.blocks.push(block(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::Cast {
                    dest: Value(2),
                    src: Operand::Const(IrConst::I32(4)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Value(Value(2)),
                    ty: IrType::I64,
                },
            ],
        ));
        assert_eq!(
            pointer_roots(&function).get(&3),
            Some(&PointerRoot::Param { index: 0, noalias: false })
        );
    }

    #[test]
    fn restrict_roots_are_distinct_but_plain_parameters_are_not() {
        let restricted = PointerRoot::Param {
            index: 0,
            noalias: true,
        };
        let plain = PointerRoot::Param {
            index: 1,
            noalias: false,
        };
        assert!(roots_proven_distinct(restricted, plain));
        assert!(!roots_proven_distinct(
            plain,
            PointerRoot::Param {
                index: 2,
                noalias: false,
            },
        ));
        let _param_shape = IrParam {
            ty: IrType::Ptr,
            noalias: true,
            struct_size: None,
            struct_align: None,
            struct_eightbyte_classes: Vec::new(),
            is_f128_sse: false,
            riscv_float_class: None,
        };
    }
}
