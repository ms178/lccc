//! Forward a whole-vector temporary store into the pointer-form store that
//! reads it straight back.
//!
//! # The shape
//!
//! The pointer-form store builtins (`_mm_storeu_si128`, `_mm256_storeu_si256`,
//! `_mm256_storeu_ps`, `_mm256_storeu_pd`, and their aligned spellings, which
//! map to the same ops) take the SOURCE as an address, so an rvalue source is
//! materialised first.  For a computed vector that gives three instructions of
//! pure traffic round the frame:
//!
//! ```text
//!   %t  = alloca 32
//!   vpaddq   (%rsi), %ymm0, %ymm1     ; VecLoadI64x4
//!   vpaddq   .LC0(%rip), %ymm1, %ymm1 ; VecAddI64x4
//!   vmovdqu  %ymm1, (%t)              ; VecStoreI64x4   <- write the temporary
//!   vmovdqu  (%t), %ymm0              ; Storeu256       <- read it back
//!   vmovdqu  %ymm0, (%rdi)            ; the store the source asked for
//! ```
//!
//! The temporary is written once by a whole-vector `VecStore` and read *only*
//! by whole-vector address-form stores of the same width -- one of them, or
//! several.  Every read is a whole-register copy of the same 16 or 32 bytes,
//! so the temporary and its accesses can all be deleted: the producer's
//! register value goes straight to every final destination.  The multi-reader
//! form is what `__m128i v = f(); _mm_storeu_si128(q, v); _mm_storeu_si128(r,
//! v);` lowers to, and it is the common shape in copy/checksum kernels.
//!
//! This is not a peephole for one spelling.  It is the general case of the
//! value-to-store path: every packed op family, every load fold and every
//! `VecStore` consumer reaches it, in straight-line code and in loops.
//! Measured with `-O2 -mavx2` (instructions from the label to `.size`):
//!
//! ```text
//!   void f(float*q, const void*p, __m256 b)             8 -> 4   (GCC 3)
//!     { _mm256_storeu_ps(q, _mm256_add_ps(_mm256_loadu_ps(p), b)); }
//!   void g(long long*q, const void*p, __m128i b)        4 -> 3   (GCC 3)
//!     { _mm_storeu_si128((__m128i*)q, _mm_add_epi64(..., b)); }
//!   loop: _mm256_storeu_si256(q+i, _mm256_add_epi64(..., k))   16 -> 8 (GCC 11)
//! ```
//!
//! # Why the conditions are what they are
//!
//! The rewrite is a memory-effect change, so every condition below is a
//! correctness condition, not a profitability heuristic:
//!
//! * **every** use of the temporary's address in the whole function is
//!   accounted for: the writing `VecStore`'s two operands (`dest_ptr` and the
//!   address) plus exactly one source operand per reader, and each reader is
//!   an address-form store of the same width reading the temporary's base.
//!   Any other use -- an escaping pointer, a scalar read, a `Memcpy`, a call
//!   argument, a second `VecStore` -- means the bytes are observable
//!   somewhere the rewrite cannot see, and the shape is refused.  Counting
//!   over the FUNCTION, not the block, is what makes that a proof rather
//!   than a hope, and it is why the readers may live in other blocks.
//! * the producing value dominates every reader (and, inside the producing
//!   block, precedes it).  Substituting a register for a memory read is only
//!   legal where the definition is available; the dominator tree decides that
//!   in O(1) per query, and an unreachable reader is refused rather than
//!   rewritten.
//! * the temporary is a non-volatile `Alloca` of exactly the store's width,
//!   so the two accesses cover the same bytes with no overrun.
//! * both ops are the unaligned whole-register families: the `VecStore*`
//!   twin and the `Storedqu`/`Storeu256`/`StoreuPs256`/`StoreuPd256` store all
//!   write bytes with an unaligned move, so the rewrite preserves the exact
//!   access width and alignment requirement.  Aligned and non-temporal
//!   spellings (`Store256`, `Movnt*`) are deliberately absent: they carry an
//!   alignment or cache contract this pass would have to re-establish, and
//!   `_mm_store_ps` already lowers to the same unaligned op as
//!   `_mm_storeu_ps`.
//! * `STATE.vector.temp_promotion_disabled` (via `CCC_DISABLE_PASSES`) can
//!   turn the whole rewrite off, which is what lets the gate prove itself
//!   against the pre-fix instruction counts instead of against a hand-edited
//!   budget.

use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{Instruction, IrConst, IrFunction, IrModule, Operand};
use crate::passes::use_def::UseDefInfo;
use crate::passes::vector_temp_promotion::{remove_instructions, whole_vector_store_width};

/// Width, in bytes, of the pointer-form store intrinsics this rewrite
/// consumes. Each takes the SOURCE ADDRESS in `args[0]` and the DESTINATION
/// address in `dest_ptr`.
///
/// `Storedqu` serves both `_mm_storeu_si128` and `_mm_storeu_ps`/`_mm_storeu_pd`
/// (and their aligned spellings), `Storeu256` is `_mm256_storeu_si256`, and
/// `StoreuPs256`/`StoreuPd256` are the AVX float/double stores.
fn address_form_store_width(op: IntrinsicOp) -> Option<usize> {
    Some(match op {
        IntrinsicOp::Storedqu => 16,
        IntrinsicOp::Storeu256 | IntrinsicOp::StoreuPs256 | IntrinsicOp::StoreuPd256 => 32,
        _ => return None,
    })
}

/// The whole-register store twin for `width` bytes.
///
/// Any `VecStore*` of the right width stores the same bytes; an integer family
/// is chosen because its emitter is the plain unaligned `vmovdqu`/`movdqu`
/// (no lane type, no FP state), which is exactly the access the replaced store
/// performed.
fn canonical_store_op(width: usize) -> Option<IntrinsicOp> {
    Some(match width {
        16 => IntrinsicOp::VecStoreI64x2,
        32 => IntrinsicOp::VecStoreI64x4,
        _ => return None,
    })
}

/// One planned rewrite: a whole-vector temporary store whose every reader is
/// an address-form store of the same width.
struct ForwardPlan {
    store_block: usize,
    store_idx: usize,
    value: u32,
    /// `(block, index)` of each reader, in program order.
    readers: Vec<(usize, usize)>,
    width: usize,
}

/// Collect the forwarding plans for one function, read-only.
///
/// A plan is admitted only when the temporary's *entire* use set in the
/// function is: the writing `VecStore`'s two address operands plus one source
/// operand per reader. That is the aliasing proof -- a temporary nobody can
/// name cannot be written or read behind the rewrite's back.
fn plan_function(func: &IrFunction) -> Vec<ForwardPlan> {
    let info = UseDefInfo::build(func);
    let dominators = crate::ir::analysis::CfgAnalysis::build(func);
    let dom = crate::ir::analysis::DominanceChecker::new(dominators.num_blocks, &dominators.idom);
    let mut plans = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::Intrinsic {
                dest: None,
                op: store_op,
                dest_ptr: Some(temp),
                args: store_args,
            } = inst
            else {
                continue;
            };
            let Some(width) = whole_vector_store_width(*store_op) else {
                continue;
            };
            let [
                Operand::Value(value),
                Operand::Value(addr),
                Operand::Const(IrConst::I64(0)),
            ] = store_args.as_slice()
            else {
                continue;
            };
            // The store must write the base of the temporary itself.
            if addr != temp {
                continue;
            }
            // The temporary must be a plain alloca of exactly this width: the
            // two accesses then cover the same bytes with no overrun, and the
            // object has no address-taken identity the rewrite could expose.
            let is_temp_alloca = func.blocks.iter().any(|b| {
                b.instructions.iter().any(|inst| {
                    matches!(
                        inst,
                        Instruction::Alloca { dest, size, align, volatile: false,
                                              semantic_volatile: false, .. }
                            if dest.0 == temp.0 && *size == width && *align <= 64
                    )
                })
            });
            if !is_temp_alloca {
                continue;
            }
            // Every other use of the temporary must be a whole-vector read by
            // an address-form store of the same width. `use_insts_of` collapses
            // the two mentions inside the writing store into one site, so the
            // expected site count is exactly `1 + readers`.
            let mut readers: Vec<(usize, usize)> = Vec::new();
            let mut well_formed = true;
            for loc in info.use_insts_of(temp.0) {
                if loc.inst_idx == crate::passes::use_def::UseLoc::TERMINATOR {
                    well_formed = false;
                    break;
                }
                let (rbi, rii) = (loc.block_idx as usize, loc.inst_idx as usize);
                if rbi == bi && rii == ii {
                    continue;
                }
                let Some(r_inst) = func.blocks.get(rbi).and_then(|b| b.instructions.get(rii))
                else {
                    well_formed = false;
                    break;
                };
                let Instruction::Intrinsic {
                    dest: None,
                    op: reader_op,
                    dest_ptr: Some(reader_dest),
                    args: reader_args,
                } = r_inst
                else {
                    well_formed = false;
                    break;
                };
                if address_form_store_width(*reader_op) != Some(width)
                    || !matches!(reader_args.as_slice(), [Operand::Value(base)] if base == temp)
                    || reader_dest == temp
                {
                    well_formed = false;
                    break;
                }
                // The forwarded value must dominate the reader: a register
                // definition cannot be used before it executes. Same-block
                // readers additionally have to come after the store.
                if !dom.dominates(bi, rbi) || (rbi == bi && rii < ii) {
                    well_formed = false;
                    break;
                }
                readers.push((rbi, rii));
            }
            if !well_formed || readers.is_empty() {
                continue;
            }
            readers.sort_unstable();
            plans.push(ForwardPlan {
                store_block: bi,
                store_idx: ii,
                value: value.0,
                readers,
                width,
            });
        }
    }
    plans
}

/// Forward every matching temporary store in one function; returns the number
/// of rewrites performed.
///
/// # Index discipline
///
/// Every plan is computed against the pre-transform IR, so the rewrite runs in
/// two phases: first every reader is re-pointed **in place** (a same-index
/// write, which cannot invalidate another plan's indices), and only then are
/// the retired stores deleted, all of a block's deletions in one
/// `remove_instructions` call.  Deleting a store between two plans would shift
/// the later plan's reader index by one and silently rewrite the wrong
/// instruction -- a value miscompile, and the reason the original single-reader
/// form was restricted to an adjacent pair.  Measured on
/// `tests/regression/simd_crc_adler.c`, where the adler kernel has two
/// independent staged pairs in one block: the buggy order stored the LOW
/// unpack where the HIGH one belonged and the checksum diverged at size 16.
fn forward_in_function(func: &mut IrFunction) -> usize {
    let plans = plan_function(func);
    if plans.is_empty() {
        return 0;
    }
    // Phase 1: rewrite readers.  Same-index writes only.
    let mut rewrites = 0usize;
    let mut retired: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for plan in &plans {
        let Some(op) = canonical_store_op(plan.width) else {
            continue;
        };
        let value = Operand::Value(crate::ir::reexports::Value(plan.value));
        for &(rbi, rii) in &plan.readers {
            let Some(block) = func.blocks.get_mut(rbi) else {
                continue;
            };
            let Some(Instruction::Intrinsic {
                dest_ptr: Some(dest),
                ..
            }) = block.instructions.get(rii)
            else {
                continue;
            };
            let dest = *dest;
            block.instructions[rii] = Instruction::Intrinsic {
                dest: None,
                op,
                dest_ptr: Some(dest),
                args: vec![value, Operand::Value(dest), Operand::Const(IrConst::I64(0))],
            };
        }
        retired
            .entry(plan.store_block)
            .or_default()
            .push(plan.store_idx);
        rewrites += 1;
    }
    // Phase 2: retire the stores.  `remove_instructions` sorts and
    // de-duplicates, so the per-block lists can arrive in any order.
    for (block_idx, mut removed) in retired {
        if let Some(block) = func.blocks.get_mut(block_idx) {
            remove_instructions(block, &mut removed);
        }
    }
    rewrites
}

/// Forward whole-vector temporary stores whose only reader is the store that
/// immediately follows them. See the module documentation for the proof
/// obligations and the measured effect.
pub(crate) fn forward_vector_temp_stores(module: &mut IrModule) -> usize {
    module.for_each_function(forward_in_function)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::source::Span;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator, Value};

    fn function(instructions: Vec<Instruction>) -> IrFunction {
        let mut func = IrFunction::new("t".into(), IrType::Void, Vec::new(), false);
        func.blocks = vec![BasicBlock {
            label: BlockId(0),
            source_spans: vec![Span::dummy(); instructions.len()],
            source_inline_ids: Vec::new(),
            instructions,
            terminator: Terminator::Return(None),
        }];
        func
    }

    fn alloca(dest: u32, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size,
            align: 0,
            volatile: false,
            semantic_volatile: false,
        }
    }

    fn vec_store(op: IntrinsicOp, value: u32, temp: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: None,
            op,
            dest_ptr: Some(Value(temp)),
            args: vec![
                Operand::Value(Value(value)),
                Operand::Value(Value(temp)),
                Operand::Const(IrConst::I64(0)),
            ],
        }
    }

    fn point_store(op: IntrinsicOp, source: u32, dest: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: None,
            op,
            dest_ptr: Some(Value(dest)),
            args: vec![Operand::Value(Value(source))],
        }
    }

    fn stores_value_at(inst: &Instruction) -> Option<(IntrinsicOp, u32, u32)> {
        let Instruction::Intrinsic {
            op,
            dest_ptr: Some(dest),
            args,
            ..
        } = inst
        else {
            return None;
        };
        match args.as_slice() {
            [
                Operand::Value(v),
                Operand::Value(d),
                Operand::Const(IrConst::I64(0)),
            ] => Some((*op, v.0, d.0)),
            _ => {
                let _ = dest;
                None
            }
        }
    }

    fn last_store(func: &IrFunction) -> Option<(IntrinsicOp, u32, u32)> {
        stores_value_at(func.blocks[0].instructions.last()?)
    }

    /// The single-reader shape is forwarded: the value goes to the final
    /// destination, and the temporary store is gone.  The now-unused alloca is
    /// left for the DCE the pipeline runs straight after this pass (only the
    /// `VecStore`/consumer pair is this pass's business).
    #[test]
    fn single_reader_temporary_is_forwarded() {
        let mut func = function(vec![
            alloca(20, 32),
            vec_store(IntrinsicOp::VecStoreF32x8, 11, 20),
            point_store(IntrinsicOp::StoreuPs256, 20, 3),
        ]);
        assert_eq!(forward_in_function(&mut func), 1);
        let instructions = &func.blocks[0].instructions;
        assert!(matches!(instructions[0], Instruction::Alloca { .. }));
        assert_eq!(instructions.len(), 2, "{instructions:?}");
        assert_eq!(
            stores_value_at(&instructions[1]),
            Some((IntrinsicOp::VecStoreI64x4, 11, 3))
        );
    }

    /// Two readers are two whole-vector reads of the same bytes: the register
    /// value reaches both and the staging store disappears.  This is the shape
    /// `__m128i v = ...; _mm_storeu_si128(q, v); _mm_storeu_si128(r, v);`
    /// lowers to (9 instructions before this generalization, 4 after -- GCC's
    /// count).
    #[test]
    fn two_readers_are_forwarded() {
        let mut func = function(vec![
            alloca(20, 32),
            vec_store(IntrinsicOp::VecStoreF32x8, 11, 20),
            point_store(IntrinsicOp::StoreuPs256, 20, 3),
            point_store(IntrinsicOp::StoreuPs256, 20, 4),
        ]);
        assert_eq!(forward_in_function(&mut func), 1);
        let instructions = &func.blocks[0].instructions;
        assert_eq!(instructions.len(), 3, "{instructions:?}");
        assert_eq!(
            stores_value_at(&instructions[1]),
            Some((IntrinsicOp::VecStoreI64x4, 11, 3))
        );
        assert_eq!(
            stores_value_at(&instructions[2]),
            Some((IntrinsicOp::VecStoreI64x4, 11, 4))
        );
    }

    /// Three readers, in three different blocks, all of which the producing
    /// value dominates: still one staging store, zero reloads.  Dominance is
    /// what makes the register substitution legal, so a fan-out across a
    /// diamond has to work.
    #[test]
    fn three_readers_across_blocks_are_forwarded() {
        let mut func = function(vec![
            alloca(20, 16),
            vec_store(IntrinsicOp::VecStoreI64x2, 11, 20),
            point_store(IntrinsicOp::Storedqu, 20, 3),
        ]);
        let mut second = crate::ir::reexports::BasicBlock {
            label: crate::ir::reexports::BlockId(1),
            source_spans: vec![Span::dummy(); 2],
            source_inline_ids: Vec::new(),
            instructions: vec![
                point_store(IntrinsicOp::Storedqu, 20, 4),
                point_store(IntrinsicOp::Storedqu, 20, 5),
            ],
            terminator: Terminator::Return(None),
        };
        second.source_spans.truncate(second.instructions.len());
        func.blocks[0].terminator = Terminator::Branch(crate::ir::reexports::BlockId(1));
        func.blocks.push(second);
        assert_eq!(forward_in_function(&mut func), 1);
        assert_eq!(func.blocks[0].instructions.len(), 2);
        assert_eq!(func.blocks[1].instructions.len(), 2);
        assert_eq!(
            stores_value_at(&func.blocks[1].instructions[0]),
            Some((IntrinsicOp::VecStoreI64x2, 11, 4))
        );
    }

    /// A reader whose block the producing value does NOT dominate cannot use
    /// the register: the substitution would reference a value before its
    /// definition.  The plan must be refused whole.
    #[test]
    fn non_dominated_reader_is_refused() {
        let mut func = function(vec![
            alloca(20, 16),
            point_store(IntrinsicOp::Storedqu, 20, 3),
        ]);
        func.blocks[0].terminator = Terminator::Branch(crate::ir::reexports::BlockId(1));
        let mut second = crate::ir::reexports::BasicBlock {
            label: crate::ir::reexports::BlockId(1),
            source_spans: vec![Span::dummy(); 2],
            source_inline_ids: Vec::new(),
            instructions: vec![
                vec_store(IntrinsicOp::VecStoreI64x2, 11, 20),
                point_store(IntrinsicOp::Storedqu, 20, 4),
            ],
            terminator: Terminator::Return(None),
        };
        second.source_spans.truncate(second.instructions.len());
        func.blocks.push(second);
        assert_eq!(forward_in_function(&mut func), 0);
    }

    /// Any use that is not an address-form store of the same width makes the
    /// temporary observable, so the whole plan is refused -- here a scalar
    /// pointer load of the temporary would let the bytes escape.
    #[test]
    fn escaping_use_refuses_every_reader() {
        let mut func = function(vec![
            alloca(20, 16),
            vec_store(IntrinsicOp::VecStoreI64x2, 11, 20),
            point_store(IntrinsicOp::Storedqu, 20, 3),
            Instruction::Load {
                dest: Value(12),
                ty: IrType::Ptr,
                ptr: Value(20),
                volatile: false,
                seg_override: crate::common::types::AddressSpace::Default,
            },
        ]);
        assert_eq!(forward_in_function(&mut func), 0);
        assert_eq!(func.blocks[0].instructions.len(), 4);
    }

    /// Two independent staged pairs in ONE block must each keep their own
    /// value.  Deleting the first store before re-pointing the second reader
    /// shifts that reader's index by one and rewrites the wrong instruction;
    /// this is the shape `adler32_simd` has (low and high unpacks staged into
    /// two 16-byte temporaries), and the value bug it caused was caught by the
    /// `simd_crc_adler` runtime oracle, not by an instruction count.
    #[test]
    fn two_independent_pairs_in_one_block_keep_their_own_values() {
        let mut func = function(vec![
            alloca(20, 16),
            alloca(21, 16),
            vec_store(IntrinsicOp::VecStoreI64x2, 11, 20),
            vec_store(IntrinsicOp::VecStoreI64x2, 12, 21),
            point_store(IntrinsicOp::Storedqu, 20, 3),
            point_store(IntrinsicOp::Storedqu, 21, 4),
        ]);
        assert_eq!(forward_in_function(&mut func), 2);
        let instructions = &func.blocks[0].instructions;
        assert_eq!(instructions.len(), 4, "{instructions:?}");
        assert_eq!(
            stores_value_at(&instructions[2]),
            Some((IntrinsicOp::VecStoreI64x2, 11, 3))
        );
        assert_eq!(
            stores_value_at(&instructions[3]),
            Some((IntrinsicOp::VecStoreI64x2, 12, 4))
        );
    }

    /// An instruction between the two means the bytes in between are
    /// observable, so the pair is not adjacent and must not be rewritten.
    #[test]
    fn non_adjacent_pair_is_refused() {
        let mut func = function(vec![
            alloca(20, 32),
            vec_store(IntrinsicOp::VecStoreF32x8, 11, 20),
            Instruction::Load {
                dest: Value(12),
                ty: IrType::Ptr,
                ptr: Value(20),
                volatile: false,
                seg_override: crate::common::types::AddressSpace::Default,
            },
            point_store(IntrinsicOp::StoreuPs256, 20, 3),
        ]);
        assert_eq!(forward_in_function(&mut func), 0);
    }

    /// A width mismatch (a 32-byte temporary store consumed by the 16-byte
    /// store) is not the same bytes twice; it must be refused.
    #[test]
    fn width_mismatch_is_refused() {
        let mut func = function(vec![
            alloca(20, 32),
            vec_store(IntrinsicOp::VecStoreF32x8, 11, 20),
            point_store(IntrinsicOp::Storedqu, 20, 3),
        ]);
        assert_eq!(forward_in_function(&mut func), 0);
    }

    /// A store whose source is not the temporary base (an offset into it) is a
    /// different read and must be refused.
    #[test]
    fn offset_reader_is_refused() {
        let mut func = function(vec![
            alloca(20, 32),
            Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::VecStoreF32x8,
                dest_ptr: Some(Value(20)),
                args: vec![
                    Operand::Value(Value(11)),
                    Operand::Value(Value(20)),
                    Operand::Const(IrConst::I64(8)),
                ],
            },
            point_store(IntrinsicOp::StoreuPs256, 20, 3),
        ]);
        assert_eq!(forward_in_function(&mut func), 0);
    }

    /// An aligned/non-temporal store spelling has its own contract and is not
    /// in the table (`Store256`, `Movnt*`).
    #[test]
    fn aligned_store_spellings_are_out_of_scope() {
        assert_eq!(address_form_store_width(IntrinsicOp::Storedqu), Some(16));
        assert_eq!(address_form_store_width(IntrinsicOp::Storeu256), Some(32));
        assert_eq!(address_form_store_width(IntrinsicOp::Store256), None);
    }
}
