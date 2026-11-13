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
//! The temporary is written once by a whole-vector `VecStore` and read once,
//! by the store that follows it.  Both ends are whole-register copies of the
//! same 16 or 32 bytes, so the temporary and its two accesses can be deleted:
//! the producer's value goes straight to the final destination.
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
//! * the temporary's address is used **exactly three times in the whole
//!   function** -- twice by the `VecStore` (`dest_ptr` and the address
//!   operand) and once by the consuming store's source operand.  Any fourth
//!   use (an escaping pointer, a scalar read, a `Memcpy`, a call argument)
//!   means the bytes are observable somewhere other than this store, and the
//!   shape is refused.  Counting over the FUNCTION, not the block, is what
//!   makes that a proof rather than a hope.
//! * the two instructions are adjacent in one block, with the store second,
//!   so no instruction can read the temporary in between.
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

/// Forward every matching temporary store in one function; returns the number
/// of rewrites performed.
fn forward_in_function(func: &mut IrFunction) -> usize {
    let info = UseDefInfo::build(func);
    // (block, vecstore index, consuming store index) triples, resolved read-only
    // before anything is mutated so the use-def indices stay valid.
    let mut hits: Vec<(usize, usize, usize, Operand, u32, usize)> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for ii in 0..block.instructions.len().saturating_sub(1) {
            let (
                Instruction::Intrinsic {
                    dest: None,
                    op: store_op,
                    dest_ptr: Some(temp),
                    args: store_args,
                },
                Instruction::Intrinsic {
                    dest: None,
                    op: consumer_op,
                    dest_ptr: Some(consumer_dest),
                    args: consumer_args,
                },
            ) = (&block.instructions[ii], &block.instructions[ii + 1])
            else {
                continue;
            };
            let Some(width) = whole_vector_store_width(*store_op) else {
                continue;
            };
            let [Operand::Value(value), operand_t, disp_t] = store_args.as_slice() else {
                continue;
            };
            if address_form_store_width(*consumer_op) != Some(width) {
                continue;
            }
            // The temporary store must be an UNALIGNED whole-register store of
            // the base of `temp`, and the consumer must read that same base.
            if !matches!(operand_t, Operand::Value(v) if v == temp)
                || !matches!(disp_t, Operand::Const(IrConst::I64(0)))
            {
                continue;
            }
            let [Operand::Value(read_base)] = consumer_args.as_slice() else {
                continue;
            };
            if read_base != temp || consumer_dest == temp {
                continue;
            }
            // Exactly three uses, in exactly these two instructions.
            if info.use_count.get(temp.0 as usize).copied() != Some(3) {
                continue;
            }
            // `use_insts_of` collapses repeated reads inside ONE instruction,
            // so the count of 3 (checked above) is two sites here: the
            // temporary store reads its address twice, the consumer once.
            let want_store = crate::passes::use_def::UseLoc::instruction(bi as u32, ii as u32);
            let want_consumer =
                crate::passes::use_def::UseLoc::instruction(bi as u32, (ii + 1) as u32);
            let mut sites = info.use_insts_of(temp.0);
            if sites.next() != Some(want_store)
                || sites.next() != Some(want_consumer)
                || sites.next().is_some()
            {
                continue;
            }
            // The temporary must be a plain alloca of exactly this width.
            let is_temp_alloca = func.blocks.iter().any(|b| {
                b.instructions.iter().any(|inst| {
                    matches!(
                        inst,
                        Instruction::Alloca { dest, size, volatile: false, .. }
                            if dest.0 == temp.0 && *size == width
                    )
                })
            });
            if !is_temp_alloca {
                continue;
            }
            hits.push((bi, ii, ii + 1, Operand::Value(*value), temp.0, width));
        }
    }
    let mut rewrites = 0usize;
    for (bi, vecstore_idx, consumer_idx, value, _temp, width) in hits {
        let Some(op) = canonical_store_op(width) else {
            continue;
        };
        let block = &mut func.blocks[bi];
        let Instruction::Intrinsic {
            dest_ptr: Some(dest),
            ..
        } = &block.instructions[consumer_idx]
        else {
            continue;
        };
        let dest = *dest;
        block.instructions[consumer_idx] = Instruction::Intrinsic {
            dest: None,
            op,
            dest_ptr: Some(dest),
            args: vec![value, Operand::Value(dest), Operand::Const(IrConst::I64(0))],
        };
        let mut removed = vec![vecstore_idx];
        remove_instructions(block, &mut removed);
        rewrites += 1;
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

    /// A temporary with TWO readers keeps its store: forwarding would leave the
    /// second reader with no value to read.
    #[test]
    fn two_readers_are_refused() {
        let mut func = function(vec![
            alloca(20, 32),
            vec_store(IntrinsicOp::VecStoreF32x8, 11, 20),
            point_store(IntrinsicOp::StoreuPs256, 20, 3),
            point_store(IntrinsicOp::StoreuPs256, 20, 4),
        ]);
        assert_eq!(forward_in_function(&mut func), 0);
        assert_eq!(func.blocks[0].instructions.len(), 4);
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
