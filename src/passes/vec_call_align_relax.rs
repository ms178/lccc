//! Call-boundary alignment relaxation for over-aligned vector homes
//! (x86-64 only; see the pipeline gate in `mod.rs`).
//!
//! `vector_temp_promotion::downgrade_nonescaping_vector_align` relaxes the
//! >16-byte alignment of a vector stack home when every use is provably
//! alignment-insensitive, but it fails closed on ANY call use.  That is the
//! right default — a pointer handed to a callee escapes — except for two
//! shapes the x86-64 backend itself marshals and whose callee never sees
//! the address:
//!
//! 1. a register-passed one-vector-register aggregate argument (psABI
//!    `[Sse, SseUp...]`, 16/32/64 bytes): the caller loads the object with
//!    the UNALIGNED `movdqu`/`vmovdqu`/`vmovdqu64` family
//!    (`calls::vector_reg_move`, "unaligned on purpose") or copies it to
//!    the outgoing area with `rep movsb`; the callee receives `%xmm/%ymm/
//!    %zmm` (or a fresh stack copy), never the home's address;
//! 2. the hidden `%ymm0`/`%zmm0` return buffer of a call in the sret SHAPE
//!    (`call_abi::hidden_vector_return_bytes`): the psABI returns the value
//!    in a register, the hidden pointer consumes no argument register and
//!    is never passed — the buffer is written AFTER the call by caller-side
//!    code, again through `vector_reg_move`.
//!
//! `v8 f(v8 a){ return ext8(a); }` is the canonical victim: `a`'s capture
//! home and the result buffer both carry `align: 32`, which costs the
//! function a `leaq`+`andq $-32` masked address per access and (before the
//! `call_arg_realignment_pins_frame` fix) its %rsp-only frame — for memory
//! no instruction ever accesses with an alignment assumption.
//!
//! The analysis is deliberately narrow and fail-closed.  Only direct uses
//! and `GetElementPtr`-with-constant-offset / `Copy` derivations are
//! followed; every other appearance of the home (or a derivation of it) —
//! terminators, phis, selects, casts, atomics, inline asm, unlisted
//! intrinsic positions, pointer stored as DATA, indirect-call targets,
//! non-SysV calling conventions (Win64 passes vectors by reference) —
//! keeps the declared alignment.  Relaxation stops at 16: a 16-byte slot
//! is a direct frame slot (no masked addressing) and still satisfies every
//! aligned-SSE contract the backend may apply to it.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::AddressSpace;
use crate::ir::instruction::Instruction;
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{IrFunction, IrModule, Operand};

/// Relax the alignment of call-marshalled vector homes across `module`.
/// Returns the number of allocas changed.
pub(crate) fn relax_call_marshalled_vector_align(module: &mut IrModule) -> usize {
    module
        .functions
        .iter_mut()
        .filter(|func| !func.is_declaration && !func.blocks.is_empty())
        .map(relax_in_function)
        .sum()
}

/// The whole-register unaligned `VecLoad*` reader family (`emit_vec_load`:
/// `args[0]` = base address, `args[1]` = constant byte displacement).
fn whole_vector_load(op: IntrinsicOp) -> bool {
    use IntrinsicOp as O;
    matches!(
        op,
        O::VecLoadF32x4
            | O::VecLoadF32x8
            | O::VecLoadF64x2
            | O::VecLoadF64x4
            | O::VecLoadI8x16
            | O::VecLoadI8x32
            | O::VecLoadI16x8
            | O::VecLoadI32x4
            | O::VecLoadI32x8
            | O::VecLoadI64x2
            | O::VecLoadI64x4
    )
}

/// The whole-register unaligned `VecStore*` writer family (`emit_vec_store`:
/// `args[0]` = register value, `args[1]` = destination address = `dest_ptr`,
/// `args[2]` = constant byte displacement).
fn whole_vector_store(op: IntrinsicOp) -> bool {
    use IntrinsicOp as O;
    matches!(
        op,
        O::VecStoreF32x4
            | O::VecStoreF32x8
            | O::VecStoreF64x2
            | O::VecStoreF64x4
            | O::VecStoreI8x16
            | O::VecStoreI8x32
            | O::VecStoreI16x8
            | O::VecStoreI32x4
            | O::VecStoreI32x8
            | O::VecStoreI64x2
            | O::VecStoreI64x4
    )
}

fn operand_aliases(op: &Operand, aliases: &FxHashSet<u32>) -> bool {
    matches!(op, Operand::Value(v) if aliases.contains(&v.0))
}

/// Whether every appearance of an alias inside one call is a position the
/// backend marshals without exposing the address:
/// * `args[i]` with the one-vector-register classes (`[Sse, SseUp...]`)
///   recorded for a by-value aggregate of that exact size;
/// * `args[0]` when the call has the hidden vector-return shape.
fn call_use_is_marshalled(
    info: &crate::ir::instruction::CallInfo,
    aliases: &FxHashSet<u32>,
    home_size: usize,
) -> bool {
    // Win64 passes 16/32/64-byte vectors by reference: the address escapes.
    if info.calling_convention != crate::common::types::CallingConvention::Default
        || info.is_fastcall
        || info.regparm.is_some()
    {
        return false;
    }
    let hidden_vector_ret =
        crate::backend::call_abi::hidden_vector_return_bytes(info.is_sret, &info.ret_eightbyte_classes);
    for (i, arg) in info.args.iter().enumerate() {
        if !operand_aliases(arg, aliases) {
            continue;
        }
        if i == 0 && hidden_vector_ret == Some(home_size) {
            continue;
        }
        let register_vector_arg = info
            .struct_arg_classes
            .get(i)
            .and_then(|c| crate::common::sysv_classify::vector_reg_bytes_of(c))
            == Some(home_size)
            && info.struct_arg_sizes.get(i).copied().flatten() == Some(home_size);
        if !register_vector_arg {
            return false;
        }
    }
    true
}

fn relax_in_function(func: &mut IrFunction) -> usize {
    // Candidates: non-volatile 16/32/64-byte homes declared over-aligned.
    let mut candidates: FxHashMap<u32, usize> =
        FxHashMap::with_capacity_and_hasher(8, Default::default());
    let mut call_seen = false;
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca {
                    dest,
                    size,
                    align,
                    volatile,
                    semantic_volatile,
                    ..
                } => {
                    if matches!(*size, 16 | 32 | 64)
                        && *align > 16
                        && !*volatile
                        && !*semantic_volatile
                    {
                        candidates.insert(dest.0, *size);
                    }
                }
                Instruction::Call { .. } | Instruction::CallIndirect { .. } => call_seen = true,
                _ => {}
            }
        }
    }
    // This pass exists ONLY for the call-marshalled shapes; everything else
    // is `downgrade_nonescaping_vector_align`'s job. No call, no work.
    if candidates.is_empty() || !call_seen {
        return 0;
    }

    // Exact derived names per candidate: the home itself plus
    // GEP-with-constant-offset / Copy chains. Fixed point over the blocks
    // (derivations may appear in any block order).
    let mut alias_sets: FxHashMap<u32, FxHashSet<u32>> = candidates
        .keys()
        .map(|&root| {
            let mut s = FxHashSet::with_capacity_and_hasher(4, Default::default());
            s.insert(root);
            (root, s)
        })
        .collect();
    loop {
        let mut grew = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let (dest, src, const_offset) = match inst {
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => (dest.0, base.0, matches!(offset, Operand::Const(_))),
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(v),
                    } => (dest.0, v.0, true),
                    _ => continue,
                };
                for set in alias_sets.values_mut() {
                    if const_offset && set.contains(&src) && set.insert(dest) {
                        grew = true;
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }

    // Verify every use of every alias; drop a candidate on the first use
    // outside the allowlist.
    let mut safe: FxHashSet<u32> = candidates.keys().copied().collect();
    for block in &func.blocks {
        for inst in &block.instructions {
            let roots: Vec<u32> = safe
                .iter()
                .copied()
                .filter(|root| {
                    let set = &alias_sets[root];
                    let mut used = false;
                    inst.for_each_used_value(|v| used |= set.contains(&v));
                    used
                })
                .collect();
            if roots.is_empty() {
                continue;
            }
            for root in roots {
                let aliases = &alias_sets[&root];
                let ok = match inst {
                    // The derivations themselves: a variable (non-constant)
                    // GEP offset, or an alias appearing as the OFFSET, is an
                    // address observation and fails below.
                    Instruction::GetElementPtr { base, offset, .. } => {
                        aliases.contains(&base.0)
                            && matches!(offset, Operand::Const(_))
                            && !operand_aliases(offset, aliases)
                    }
                    Instruction::Copy { src, .. } => operand_aliases(src, aliases),
                    // Scalar dereference: x86-64 scalar and 16-byte slot
                    // accesses are emitted alignment-tolerant; the address
                    // itself is not exposed.
                    Instruction::Load {
                        ptr, seg_override, ..
                    } => aliases.contains(&ptr.0) && *seg_override == AddressSpace::Default,
                    Instruction::Store {
                        val,
                        ptr,
                        seg_override,
                        ..
                    } => {
                        aliases.contains(&ptr.0)
                            && *seg_override == AddressSpace::Default
                            && !operand_aliases(val, aliases)
                    }
                    // Block copies read/write with alignment-agnostic code
                    // (same contract `downgrade_nonescaping_vector_align`
                    // relies on).
                    Instruction::Memcpy { .. } => true,
                    // Whole-register unaligned vector readers/writers, in
                    // their address position only.
                    Instruction::Intrinsic {
                        op,
                        args,
                        dest_ptr,
                        ..
                    } => {
                        if whole_vector_load(*op) {
                            dest_ptr.is_none()
                                && args.len() == 2
                                && matches!(args[1], Operand::Const(_))
                                && !operand_aliases(&args[1], aliases)
                        } else if whole_vector_store(*op) {
                            args.len() == 3
                                && !operand_aliases(&args[0], aliases)
                                && matches!(args[2], Operand::Const(_))
                        } else {
                            false
                        }
                    }
                    Instruction::Call { info, .. } => {
                        call_use_is_marshalled(info, aliases, candidates[&root])
                    }
                    Instruction::CallIndirect { func_ptr, info } => {
                        !operand_aliases(func_ptr, aliases)
                            && call_use_is_marshalled(info, aliases, candidates[&root])
                    }
                    _ => false,
                };
                if !ok {
                    safe.remove(&root);
                }
            }
        }
        // Any terminator use (returned address, switch scrutinee, ...)
        // observes the address: fail closed.
        let mut terminator_uses: Vec<u32> = Vec::new();
        block.terminator.for_each_used_value(|v| {
            for (&root, set) in &alias_sets {
                if set.contains(&v) {
                    terminator_uses.push(root);
                }
            }
        });
        for root in terminator_uses {
            safe.remove(&root);
        }
    }

    if safe.is_empty() {
        return 0;
    }
    let mut changed = 0usize;
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Alloca { dest, align, .. } = inst {
                if safe.contains(&dest.0) && *align > 16 {
                    *align = 16;
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
    use crate::common::types::{EightbyteClass, IrType};
    use crate::ir::instruction::CallInfo;
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Terminator, Value};

    fn block(label: u32, instructions: Vec<Instruction>) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            source_spans: vec![Span::dummy(); instructions.len()],
            source_inline_ids: Vec::new(),
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

    fn ymm_call(args: Vec<u32>, sret_vector: bool) -> Instruction {
        let classes = vec![
            EightbyteClass::Sse,
            EightbyteClass::SseUp,
            EightbyteClass::SseUp,
            EightbyteClass::SseUp,
        ];
        let n = args.len();
        let mut info = CallInfo {
            calling_convention: crate::common::types::CallingConvention::Default,
            dest: Some(Value(90)),
            args: args.into_iter().map(|v| Operand::Value(Value(v))).collect(),
            arg_types: vec![IrType::Ptr; n],
            return_type: IrType::Ptr,
            is_variadic: false,
            num_fixed_args: n,
            struct_arg_sizes: vec![Some(32); n],
            struct_arg_aligns: vec![Some(32); n],
            struct_arg_classes: vec![classes.clone(); n],
            struct_arg_fp_agg_classes: vec![None; n],
            struct_arg_is_f128_sse: vec![false; n],
            ret_is_f128_sse: false,
            ret_is_vector128: false,
            is_sret: false,
            is_fastcall: false,
            regparm: None,
            is_pure: false,
            is_const: false,
            ret_eightbyte_classes: Vec::new(),
            assume_library: false,
            cold: false,
        };
        if sret_vector {
            info.is_sret = true;
            info.ret_eightbyte_classes = classes;
            info.struct_arg_sizes[0] = None;
            info.struct_arg_aligns[0] = None;
            info.struct_arg_classes[0] = Vec::new();
        }
        Instruction::Call {
            func: "ext8".into(),
            info,
        }
    }

    fn alignment_of(func: &IrFunction, id: u32) -> usize {
        for b in &func.blocks {
            for inst in &b.instructions {
                if let Instruction::Alloca { dest, align, .. } = inst {
                    if dest.0 == id {
                        return *align;
                    }
                }
            }
        }
        panic!("alloca {id} not found");
    }

    /// `v8 f(v8 a){ return ext8(a); }`: the capture home (register-passed
    /// %ymm argument) and the hidden return buffer both relax to 16.
    #[test]
    fn register_vector_arg_and_hidden_return_buffer_relax() {
        let mut f = function(
            "ret256",
            vec![block(
                0,
                vec![
                    alloca(1, 32, 32),
                    alloca(3, 32, 32),
                    ymm_call(vec![3, 1], true),
                    Instruction::Intrinsic {
                        dest: Some(Value(4)),
                        op: IntrinsicOp::VecLoadF32x8,
                        dest_ptr: None,
                        args: vec![Operand::Value(Value(3)), Operand::Const(IrConst::I64(0))],
                    },
                ],
            )],
        );
        f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(4))));
        assert_eq!(relax_in_function(&mut f), 2);
        assert_eq!(alignment_of(&f, 1), 16);
        assert_eq!(alignment_of(&f, 3), 16);
    }

    /// A genuine MEMORY-class sret buffer (no vector return classes): the
    /// callee writes through the pointer, possibly with aligned stores.
    #[test]
    fn plain_sret_buffer_is_refused() {
        let mut f = function(
            "memret",
            vec![block(
                0,
                vec![alloca(3, 32, 32), {
                    let mut call = ymm_call(vec![3], false);
                    if let Instruction::Call { info, .. } = &mut call {
                        info.is_sret = true;
                        info.struct_arg_sizes[0] = None;
                        info.struct_arg_aligns[0] = None;
                        info.struct_arg_classes[0] = Vec::new();
                    }
                    call
                }],
            )],
        );
        assert_eq!(relax_in_function(&mut f), 0);
        assert_eq!(alignment_of(&f, 3), 32);
    }

    /// A pointer argument without the one-vector-register classes escapes
    /// (`g(&a)`): the home keeps its declared alignment.
    #[test]
    fn plain_pointer_escape_is_refused() {
        let mut f = function(
            "escape",
            vec![block(
                0,
                vec![alloca(1, 32, 32), {
                    let mut call = ymm_call(vec![1], false);
                    if let Instruction::Call { info, .. } = &mut call {
                        info.struct_arg_sizes[0] = None;
                        info.struct_arg_aligns[0] = None;
                        info.struct_arg_classes[0] = Vec::new();
                    }
                    call
                }],
            )],
        );
        assert_eq!(relax_in_function(&mut f), 0);
        assert_eq!(alignment_of(&f, 1), 32);
    }

    /// The address stored as DATA escapes even when the same call also
    /// consumes the home as a register-passed vector.
    #[test]
    fn address_stored_as_data_is_refused() {
        let mut f = function(
            "stored",
            vec![block(
                0,
                vec![
                    alloca(1, 32, 32),
                    alloca(2, 8, 0),
                    Instruction::Store {
                        val: Operand::Value(Value(1)),
                        ptr: Value(2),
                        ty: IrType::Ptr,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    ymm_call(vec![1], false),
                ],
            )],
        );
        assert_eq!(relax_in_function(&mut f), 0);
        assert_eq!(alignment_of(&f, 1), 32);
    }

    /// Scalar lane reads through a constant GEP stay allowed (the
    /// `lane_extract` shape), and a variable GEP offset fails closed.
    #[test]
    fn constant_gep_lane_read_relaxes_variable_gep_refuses() {
        let mut f = function(
            "lanes",
            vec![block(
                0,
                vec![
                    alloca(1, 32, 32),
                    Instruction::GetElementPtr {
                        dest: Value(2),
                        base: Value(1),
                        offset: Operand::Const(IrConst::I64(12)),
                        ty: IrType::F32,
                    },
                    Instruction::Load {
                        dest: Value(3),
                        ptr: Value(2),
                        ty: IrType::F32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    ymm_call(vec![1], false),
                ],
            )],
        );
        assert_eq!(relax_in_function(&mut f), 1);
        assert_eq!(alignment_of(&f, 1), 16);

        let mut g = function(
            "varlane",
            vec![block(
                0,
                vec![
                    alloca(1, 32, 32),
                    Instruction::GetElementPtr {
                        dest: Value(2),
                        base: Value(1),
                        offset: Operand::Value(Value(9)),
                        ty: IrType::F32,
                    },
                    Instruction::Load {
                        dest: Value(3),
                        ptr: Value(2),
                        ty: IrType::F32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    ymm_call(vec![1], false),
                ],
            )],
        );
        assert_eq!(relax_in_function(&mut g), 0);
        assert_eq!(alignment_of(&g, 1), 32);
    }

    /// Win64 passes vectors by reference: the address escapes to the callee.
    #[test]
    fn microsoft_x64_call_is_refused() {
        let mut f = function(
            "win64",
            vec![block(
                0,
                vec![alloca(1, 32, 32), {
                    let mut call = ymm_call(vec![1], false);
                    if let Instruction::Call { info, .. } = &mut call {
                        info.calling_convention =
                            crate::common::types::CallingConvention::MicrosoftX64;
                    }
                    call
                }],
            )],
        );
        assert_eq!(relax_in_function(&mut f), 0);
        assert_eq!(alignment_of(&f, 1), 32);
    }

    /// Without any call the pass leaves the function to
    /// `downgrade_nonescaping_vector_align` entirely.
    #[test]
    fn no_call_no_work() {
        let mut f = function("nocall", vec![block(0, vec![alloca(1, 32, 32)])]);
        assert_eq!(relax_in_function(&mut f), 0);
        assert_eq!(alignment_of(&f, 1), 32);
    }
}
