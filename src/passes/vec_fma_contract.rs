//! Packed FMA contraction for GNU/ext vector arithmetic (`vecfma`).
//!
//! The SIMD lowering (`src/ir/lowering/vector_simd.rs`) turns `a*b + c` on
//! a 16/32-byte float vector into a `VecMul*` whose only consumer is a
//! `VecAdd*`/`VecSub*` of the same register family.  The scalar pipeline
//! contracts the analogous `Mul`→`Add` pair in the backend detector
//! (`generation::emit_fused_mul_add`); nothing did the same for the packed
//! families, so under `-mfma` GCC's `vfmadd132ps` came out of lccc as
//! `vmulps` + `vaddps` — two roundings, two instructions, and a dependent
//! pair on the critical path of every vector kernel.
//!
//! This pass rewrites, per family `F ∈ {F32x4, F32x8, F64x2, F64x4}`:
//!
//! ```text
//!   %m = VecMulF a, b            (every use an absorbing add/sub in the
//!   %r = VecAddF %m, c  |  c, %m      same block — GCC's convert_mult_to_fma)
//!     ==>  %r = Fma_F a, b, c              r = c + a·b
//!   %r = VecSubF c, %m
//!     ==>  %r = Fnma_F a, b, c             r = c − a·b
//!   %r = VecSubF %m, c
//!     ==>  %r = Fms_F a, b, c              r = a·b − c
//! ```
//!
//! The target ops are the ones BB-SLP already emits for the same algebra
//! (`VecFmaF64x2`/`VecFnmaF64x2`/`VecFmaF32x4`/`VecFnmaF32x4`, and the
//! affine-map `VecMaddF64x4`/`VecMaddF32x8` with their `Signed(np, na)`
//! sign families) plus the `VecFmsF64x2`/`VecFmsF32x4` pair added for
//! the 128-bit `a·b − c` shape, with the SLP's argument contract
//! `[a, b, acc]`.
//!
//! Gating mirrors the scalar detector exactly:
//! * FMA3 must be available (`simplify::has_fma3()` — `-mfma`, or an
//!   enabling `-march`; cleared by `-mno-fma`).
//! * `-ffp-contract=off` never contracts; `fast` contracts freely; `on`
//!   contracts only when both intrinsics carry the same statement-root tag
//!   (`IrFunction::fp_expr_tags`, stamped by `emit_vec_binary`).
//! * A loop-carried accumulator update (`s = s + a[i]*b[i]`, the
//!   accumulator being the ADDEND) is NOT fused (`accumulator_values`):
//!   the scalar gate's latency argument and GCC's 256-bit tuning both keep
//!   the multiply off the serial chain.  `s = s*a[i] + c` (accumulator as
//!   a multiplicand) IS fused — the multiply is on the chain either way,
//!   and GCC fuses it.
//!
//! Single-rounding semantics are the C11 6.5p8 contraction licence, the
//! same one the scalar path takes; the result is bit-identical to GCC's
//! `-mfma` output for the same expression.
//!
//! Operands may alias (`x*x + c`, `a*b + a`): both emitters select the
//! FMA3 form from which operand the destination register holds and stage
//! at most one copy, so every aliasing is exact.
//!
//! Runs after `vecssa` (so the mul/add operands are register values rather
//! than alloca round trips) and before the vector temp promotion cleanups.

use crate::common::fp_contract::FpContract;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{BasicBlock, Instruction, IrFunction, IrModule, Operand, Value};

/// Fused forms for one register family: `acc + a·b`, `acc − a·b`,
/// `a·b − acc` and `−a·b − acc`.
#[derive(Clone, Copy)]
struct FusedOps {
    fma: IntrinsicOp,
    fnma: IntrinsicOp,
    fms: IntrinsicOp,
    fnms: IntrinsicOp,
}

/// The family's opcodes around a multiply: `(add, sub, sign-flip xor,
/// fused)`, or `None` for every other intrinsic.
fn family_of(mul: &IntrinsicOp) -> Option<(IntrinsicOp, IntrinsicOp, IntrinsicOp, FusedOps)> {
    Some(match mul {
        IntrinsicOp::VecMulF32x8 => (
            IntrinsicOp::VecAddF32x8,
            IntrinsicOp::VecSubF32x8,
            IntrinsicOp::VecXorF32x8,
            FusedOps {
                fma: IntrinsicOp::VecMaddF32x8,
                fnma: IntrinsicOp::VecMaddF32x8Signed(true, false),
                fms: IntrinsicOp::VecMaddF32x8Signed(false, true),
                fnms: IntrinsicOp::VecMaddF32x8Signed(true, true),
            },
        ),
        IntrinsicOp::VecMulF64x4 => (
            IntrinsicOp::VecAddF64x4,
            IntrinsicOp::VecSubF64x4,
            IntrinsicOp::VecXorF64x4,
            FusedOps {
                fma: IntrinsicOp::VecMaddF64x4,
                fnma: IntrinsicOp::VecMaddF64x4Signed(true, false),
                fms: IntrinsicOp::VecMaddF64x4Signed(false, true),
                fnms: IntrinsicOp::VecMaddF64x4Signed(true, true),
            },
        ),
        IntrinsicOp::VecMulF32x4 => (
            IntrinsicOp::VecAddF32x4,
            IntrinsicOp::VecSubF32x4,
            IntrinsicOp::VecXorF32x4,
            FusedOps {
                fma: IntrinsicOp::VecFmaF32x4,
                fnma: IntrinsicOp::VecFnmaF32x4,
                fms: IntrinsicOp::VecFmsF32x4,
                fnms: IntrinsicOp::VecFnmsF32x4,
            },
        ),
        IntrinsicOp::VecMulF64x2 => (
            IntrinsicOp::VecAddF64x2,
            IntrinsicOp::VecSubF64x2,
            IntrinsicOp::VecXorF64x2,
            FusedOps {
                fma: IntrinsicOp::VecFmaF64x2,
                fnma: IntrinsicOp::VecFnmaF64x2,
                fms: IntrinsicOp::VecFmsF64x2,
                fnms: IntrinsicOp::VecFnmsF64x2,
            },
        ),
        _ => return None,
    })
}

/// Whether `inst` is the family's sign flip of `product`
/// (`VecXor(product, −0.0)`, the lowering of unary minus on a float
/// vector): returns the negated value.
fn sign_flip_of(inst: &Instruction, xor: &IntrinsicOp, product: Value) -> Option<Value> {
    let Instruction::Intrinsic {
        dest: Some(dest),
        op,
        dest_ptr: None,
        args,
    } = inst
    else {
        return None;
    };
    if op != xor {
        return None;
    }
    let [Operand::Value(x), Operand::Const(c)] = args.as_slice() else {
        return None;
    };
    if *x != product {
        return None;
    }
    // Exactly the sign-bit mask: −0.0 in the lane type (`to_f64` keeps the
    // sign of a negative zero; `1.0 / v` distinguishes it from +0.0).
    let is_neg_zero = c.to_f64().is_some_and(|v| v == 0.0 && v.is_sign_negative());
    is_neg_zero.then_some(*dest)
}

/// Function-wide use counts (instructions and terminators).
fn use_counts(func: &IrFunction) -> FxHashMap<u32, usize> {
    let mut counts: FxHashMap<u32, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| *counts.entry(v).or_insert(0) += 1);
        }
        block
            .terminator
            .for_each_used_value(|v| *counts.entry(v).or_insert(0) += 1);
    }
    counts
}

/// A multiply eligible for contraction: `(dest, a, b, fused ops, add, sub)`.
/// `dest` is the PRODUCT value the consumers read — either the multiply's
/// own result or its sign flip (`negated`, the `-(a*b) ± c` shape), in
/// which case the fused family is mirrored: `c + (−p)` → fnma,
/// `(−p) − c` → fnms, `c − (−p)` → fma.
struct MulSite {
    dest: Value,
    a: Value,
    b: Value,
    negated: bool,
    add: IntrinsicOp,
    sub: IntrinsicOp,
    xor: IntrinsicOp,
    fused: FusedOps,
}

fn mul_site(inst: &Instruction) -> Option<MulSite> {
    let Instruction::Intrinsic {
        dest: Some(dest),
        op,
        dest_ptr: None,
        args,
    } = inst
    else {
        return None;
    };
    let (add, sub, xor, fused) = family_of(op)?;
    let [Operand::Value(a), Operand::Value(b)] = args.as_slice() else {
        return None;
    };
    Some(MulSite {
        dest: *dest,
        a: *a,
        b: *b,
        negated: false,
        add,
        sub,
        xor,
        fused,
    })
}

/// The fused opcode and `acc` for an add/sub consuming `site.dest`, if the
/// family has one for that operand position.
fn fused_form(site: &MulSite, op: &IntrinsicOp, args: &[Operand]) -> Option<(IntrinsicOp, Value)> {
    let [Operand::Value(x), Operand::Value(y)] = args else {
        return None;
    };
    let m = site.dest;
    let f = &site.fused;
    // (product as x, product as y) fused opcodes for this consumer.
    let (as_x, as_y) = if *op == site.add {
        // p + c  /  c + p
        let fused = if site.negated { f.fnma } else { f.fma };
        (fused, fused)
    } else if *op == site.sub {
        // p − c  /  c − p
        if site.negated {
            (f.fnms, f.fma)
        } else {
            (f.fms, f.fnma)
        }
    } else {
        return None;
    };
    if *x == m && *y != m {
        return Some((as_x, *y));
    }
    if *y == m && *x != m {
        return Some((as_y, *x));
    }
    None
}

/// Contract every eligible multiply in `block`; returns the number of
/// fused consumers.
///
/// GCC's rule (`convert_mult_to_fma`): a multiply is contracted when EVERY
/// use is an add/sub in the same basic block that can absorb it — each
/// consumer becomes its own FMA and the multiply disappears (one product
/// duplicated into n fused ops is still fewer instructions than the
/// multiply plus n adds, and it keeps the single-rounding result the
/// reference compiler produces for `m = a*b; x = m + c; y = d - m;`). A
/// use that cannot absorb the product (another multiply, a store, a use
/// in a different block, both operands of one add) vetoes the whole site.
fn contract_block(
    block: &mut BasicBlock,
    uses: &FxHashMap<u32, usize>,
    contract: FpContract,
    tags: &FxHashMap<u32, u64>,
    accs: &FxHashSet<u32>,
) -> usize {
    let mut fused = 0;
    let mut drop: Vec<usize> = Vec::new();
    let n = block.instructions.len();
    for i in 0..n {
        let Some(mut site) = mul_site(&block.instructions[i]) else {
            continue;
        };
        let mut total_uses = uses.get(&site.dest.0).copied().unwrap_or(0);
        if total_uses == 0 {
            continue;
        }
        // `-(a*b)`: a product whose ONLY use is the family's sign flip
        // contracts through the flip — the consumers of the negated value
        // take the mirrored fused forms (vfnmadd/vfnmsub), exactly GCC's
        // `convert_mult_to_fma` behaviour for a NEGATE_EXPR use.  The flip
        // is dropped together with the multiply.
        let mut flip_at: Option<usize> = None;
        if total_uses == 1 {
            if let Some(j) = (i + 1..n)
                .find(|&j| sign_flip_of(&block.instructions[j], &site.xor, site.dest).is_some())
            {
                let neg =
                    sign_flip_of(&block.instructions[j], &site.xor, site.dest).unwrap_or(site.dest);
                let neg_uses = uses.get(&neg.0).copied().unwrap_or(0);
                if neg_uses > 0 {
                    site.dest = neg;
                    site.negated = true;
                    total_uses = neg_uses;
                    flip_at = Some(j);
                }
            }
        }
        // Every consumer must be a later instruction of this block that
        // the family can absorb under the contraction contract.
        let mut rewrites: Vec<(usize, Instruction)> = Vec::new();
        let scan_from = flip_at.map_or(i + 1, |j| j + 1);
        for j in scan_from..n {
            let mut hit = false;
            block.instructions[j].for_each_used_value(|v| hit |= v == site.dest.0);
            if !hit {
                continue;
            }
            let Instruction::Intrinsic {
                dest: Some(r),
                op,
                dest_ptr: None,
                args,
            } = &block.instructions[j]
            else {
                break;
            };
            let Some((fused_op, acc)) = fused_form(&site, op, args) else {
                break;
            };
            if !contract.fuse_pair(tags.get(&site.dest.0).copied(), tags.get(&r.0).copied()) {
                break;
            }
            // Loop-carried accumulator update `acc = acc + a*b`: keep the
            // multiply off the serial chain (see `accumulator_values`).
            // Both the add's destination AND its addend must be on the
            // chain — exactly the scalar gate's `is_accumulator_update`
            // (`backend::generation`) and GCC's rule: `s = s*a[i] + c`
            // already has the multiply on the chain (the accumulator is a
            // multiplicand), so fusing it only removes latency, and GCC
            // emits `vfmadd132ps` there (the differential gate's
            // `cf_goto_loop` is bit-exact only when lccc fuses too).
            if accs.contains(&r.0) && accs.contains(&acc.0) {
                break;
            }
            rewrites.push((
                j,
                Instruction::Intrinsic {
                    dest: Some(*r),
                    op: fused_op,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(site.a),
                        Operand::Value(site.b),
                        Operand::Value(acc),
                    ],
                },
            ));
        }
        if rewrites.len() != total_uses {
            continue;
        }
        fused += rewrites.len();
        for (j, inst) in rewrites {
            block.instructions[j] = inst;
        }
        drop.push(i);
        if let Some(j) = flip_at {
            drop.push(j);
        }
    }
    if !drop.is_empty() {
        let drop: FxHashSet<usize> = drop.into_iter().collect();
        let keep_spans = block.source_spans.len() == block.instructions.len();
        let mut idx = 0;
        block.instructions.retain(|_| {
            idx += 1;
            !drop.contains(&(idx - 1))
        });
        if keep_spans {
            let mut idx = 0;
            block.source_spans.retain(|_| {
                idx += 1;
                !drop.contains(&(idx - 1))
            });
        }
    }
    fused
}

/// Loop-carried accumulators: phi destinations, multi-definition `Copy`
/// destinations (the phi-web shape), and — transitively — every value
/// copied or phi-merged INTO one (the backedge source sits on the same
/// serial chain).  Mirrors the scalar fusion gate's `accumulator_dests`
/// (`backend::generation`): fusing `acc = acc + a*b` puts the multiply on
/// the accumulator's serial dependency chain, and the fused op's latency
/// (4 cycles on every current core) exceeds the add's (2–3), so the
/// reduction loop runs SLOWER fused even though it executes fewer
/// instructions.  GCC's generic tuning makes the same call for 256-bit
/// chains (`X86_TUNE_AVOID_256FMA_CHAINS`), so the differential against
/// it stays bit-identical as well.
fn accumulator_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut accs: FxHashSet<u32> = FxHashSet::default();
    let mut edges: Vec<(u32, u32)> = Vec::new(); // (dest, src)
    let mut copy_defs: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Phi { dest, incoming, .. } => {
                    accs.insert(dest.0);
                    for (op, _) in incoming {
                        if let Operand::Value(v) = op {
                            edges.push((dest.0, v.0));
                        }
                    }
                }
                Instruction::Copy { dest, src } => {
                    if let Operand::Value(v) = src {
                        edges.push((dest.0, v.0));
                    }
                    let n = copy_defs.entry(dest.0).or_insert(0);
                    *n += 1;
                    if *n == 2 {
                        accs.insert(dest.0);
                    }
                }
                _ => {}
            }
        }
    }
    loop {
        let mut grew = false;
        for &(d, s) in &edges {
            if accs.contains(&d) && accs.insert(s) {
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    accs
}

fn contract_function(func: &mut IrFunction, contract: FpContract) -> usize {
    let uses = use_counts(func);
    let accs = accumulator_values(func);
    let tags = std::mem::take(&mut func.fp_expr_tags);
    let mut total = 0;
    for block in &mut func.blocks {
        total += contract_block(block, &uses, contract, &tags, &accs);
    }
    func.fp_expr_tags = tags;
    total
}

/// Entry point: contract packed mul+add/sub pairs module-wide under the
/// resolved contraction contract.  Returns the number of fused pairs.
pub(crate) fn contract_module(module: &mut IrModule, contract: FpContract) -> usize {
    if contract == FpContract::Off || !super::simplify::has_fma3() {
        return 0;
    }
    module
        .functions
        .iter_mut()
        .map(|f| contract_function(f, contract))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BlockId, IrConst, Terminator};

    fn intr(dest: u32, op: IntrinsicOp, args: &[u32]) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op,
            dest_ptr: None,
            args: args.iter().map(|&v| Operand::Value(Value(v))).collect(),
        }
    }

    fn func(instructions: Vec<Instruction>, ret: u32) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::Void, vec![], false);
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(Some(Operand::Value(Value(ret)))),
            source_spans: Vec::new(),
        }];
        f
    }

    fn ops(f: &IrFunction) -> Vec<IntrinsicOp> {
        f.blocks[0]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Intrinsic { op, .. } => Some(op.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn mul_add_contracts_to_madd_with_slp_arg_order() {
        let mut f = func(
            vec![
                intr(10, IntrinsicOp::VecMulF32x8, &[1, 2]),
                intr(11, IntrinsicOp::VecAddF32x8, &[3, 10]),
            ],
            11,
        );
        assert_eq!(contract_function(&mut f, FpContract::Fast), 1);
        assert_eq!(ops(&f), vec![IntrinsicOp::VecMaddF32x8]);
        let Instruction::Intrinsic { args, .. } = &f.blocks[0].instructions[0] else {
            panic!()
        };
        let got: Vec<u32> = args
            .iter()
            .map(|a| match a {
                Operand::Value(v) => v.0,
                _ => u32::MAX,
            })
            .collect();
        assert_eq!(got, vec![1, 2, 3]);
    }

    #[test]
    fn sub_forms_pick_signed_families() {
        let mut f = func(
            vec![
                intr(10, IntrinsicOp::VecMulF64x4, &[1, 2]),
                intr(11, IntrinsicOp::VecSubF64x4, &[3, 10]),
                intr(12, IntrinsicOp::VecMulF64x4, &[1, 2]),
                intr(13, IntrinsicOp::VecSubF64x4, &[12, 11]),
            ],
            13,
        );
        assert_eq!(contract_function(&mut f, FpContract::Fast), 2);
        assert_eq!(
            ops(&f),
            vec![
                IntrinsicOp::VecMaddF64x4Signed(true, false),
                IntrinsicOp::VecMaddF64x4Signed(false, true),
            ]
        );
    }

    #[test]
    fn narrow_families_fuse_fms_and_aliased_operands() {
        let mut f = func(
            vec![
                intr(10, IntrinsicOp::VecMulF32x4, &[1, 2]),
                intr(11, IntrinsicOp::VecSubF32x4, &[10, 3]),
                intr(12, IntrinsicOp::VecMulF32x4, &[1, 1]),
                intr(13, IntrinsicOp::VecAddF32x4, &[12, 11]),
                intr(14, IntrinsicOp::VecMulF64x2, &[4, 5]),
                intr(15, IntrinsicOp::VecSubF64x2, &[6, 14]),
            ],
            15,
        );
        assert_eq!(contract_function(&mut f, FpContract::Fast), 3);
        assert_eq!(
            ops(&f),
            vec![
                IntrinsicOp::VecFmsF32x4,
                IntrinsicOp::VecFmaF32x4,
                IntrinsicOp::VecFnmaF64x2
            ]
        );
    }

    #[test]
    fn every_use_must_absorb_the_product() {
        // Two adds absorb one product: both fuse (GCC duplicates the
        // multiply into each FMA).
        let mut f = func(
            vec![
                intr(10, IntrinsicOp::VecMulF32x8, &[1, 2]),
                intr(11, IntrinsicOp::VecAddF32x8, &[3, 10]),
                intr(12, IntrinsicOp::VecSubF32x8, &[4, 10]),
            ],
            12,
        );
        assert_eq!(contract_function(&mut f, FpContract::Fast), 2);
        assert_eq!(
            ops(&f),
            vec![
                IntrinsicOp::VecMaddF32x8,
                IntrinsicOp::VecMaddF32x8Signed(true, false)
            ]
        );
        // A non-absorbing use (the product feeds another multiply) vetoes
        // the whole site; `m + m` is not absorbable either.
        let mut g = func(
            vec![
                intr(10, IntrinsicOp::VecMulF32x8, &[1, 2]),
                intr(11, IntrinsicOp::VecAddF32x8, &[3, 10]),
                intr(12, IntrinsicOp::VecMulF32x8, &[10, 11]),
                intr(13, IntrinsicOp::VecMulF32x8, &[1, 3]),
                intr(14, IntrinsicOp::VecAddF32x8, &[13, 13]),
            ],
            14,
        );
        assert_eq!(contract_function(&mut g, FpContract::Fast), 0);
        // A use outside the block (the terminator) vetoes too.
        let mut h = func(
            vec![
                intr(10, IntrinsicOp::VecMulF32x8, &[1, 2]),
                intr(11, IntrinsicOp::VecAddF32x8, &[3, 10]),
            ],
            10,
        );
        assert_eq!(contract_function(&mut h, FpContract::Fast), 0);
        let mut k = func(
            vec![
                intr(10, IntrinsicOp::VecMulF32x8, &[1, 2]),
                intr(11, IntrinsicOp::VecAddF32x8, &[3, 10]),
            ],
            11,
        );
        assert_eq!(contract_function(&mut k, FpContract::Off), 0);
    }

    fn fused_args(f: &IrFunction, idx: usize) -> Vec<u32> {
        match &f.blocks[0].instructions[idx] {
            Instruction::Intrinsic { args, .. } => args
                .iter()
                .map(|a| match a {
                    Operand::Value(v) => v.0,
                    Operand::Const(_) => u32::MAX,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `-(a*b) - c` → vfnmsub, `-(a*b) + c` / `c + -(a*b)` → vfnmadd,
    /// `c - -(a*b)` → vfmadd; multiply AND sign flip disappear.
    #[test]
    fn negated_product_contracts_through_the_sign_flip() {
        let neg = |dest: u32, src: u32| Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op: IntrinsicOp::VecXorF32x8,
            dest_ptr: None,
            args: vec![
                Operand::Value(Value(src)),
                Operand::Const(IrConst::F32(-0.0)),
            ],
        };
        let shapes: [(fn(&mut Vec<Instruction>), IntrinsicOp, [u32; 3]); 4] = [
            (
                |v| v.push(intr(12, IntrinsicOp::VecSubF32x8, &[11, 3])),
                IntrinsicOp::VecMaddF32x8Signed(true, true),
                [1, 2, 3],
            ),
            (
                |v| v.push(intr(12, IntrinsicOp::VecAddF32x8, &[11, 3])),
                IntrinsicOp::VecMaddF32x8Signed(true, false),
                [1, 2, 3],
            ),
            (
                |v| v.push(intr(12, IntrinsicOp::VecAddF32x8, &[3, 11])),
                IntrinsicOp::VecMaddF32x8Signed(true, false),
                [1, 2, 3],
            ),
            (
                |v| v.push(intr(12, IntrinsicOp::VecSubF32x8, &[3, 11])),
                IntrinsicOp::VecMaddF32x8,
                [1, 2, 3],
            ),
        ];
        for (push_consumer, want_op, want_args) in shapes {
            let mut ins = vec![intr(10, IntrinsicOp::VecMulF32x8, &[1, 2]), neg(11, 10)];
            push_consumer(&mut ins);
            let mut f = func(ins, 12);
            assert_eq!(contract_function(&mut f, FpContract::Fast), 1);
            assert_eq!(ops(&f), vec![want_op.clone()], "{want_op:?}");
            assert_eq!(fused_args(&f, 0), want_args.to_vec());
        }
    }

    /// A sign flip that is not the product's only use, or a flip whose
    /// constant is not exactly −0.0, is not a contraction site.
    #[test]
    fn negated_product_requires_single_use_and_sign_mask() {
        // The product is ALSO stored: the flip may not absorb it.
        let mut ins = vec![
            intr(10, IntrinsicOp::VecMulF32x8, &[1, 2]),
            Instruction::Intrinsic {
                dest: Some(Value(11)),
                op: IntrinsicOp::VecXorF32x8,
                dest_ptr: None,
                args: vec![
                    Operand::Value(Value(10)),
                    Operand::Const(IrConst::F32(-0.0)),
                ],
            },
            intr(12, IntrinsicOp::VecSubF32x8, &[11, 3]),
            intr(13, IntrinsicOp::VecAddF32x8, &[10, 10]),
        ];
        let mut f = func(ins.clone(), 12);
        assert_eq!(contract_function(&mut f, FpContract::Fast), 0);
        // +0.0 is not a sign flip.
        ins.truncate(3);
        if let Instruction::Intrinsic { args, .. } = &mut ins[1] {
            args[1] = Operand::Const(IrConst::F32(0.0));
        }
        let mut f = func(ins, 12);
        assert_eq!(contract_function(&mut f, FpContract::Fast), 0);
    }

    #[test]
    fn loop_carried_accumulator_update_is_not_fused() {
        use crate::ir::reexports::Terminator;
        // Header phi %5 = φ(%4 entry, %11 body); body: %11 = %5 + %1*%2.
        let mut f = IrFunction::new("t".to_string(), IrType::Void, vec![], false);
        f.blocks = vec![
            BasicBlock {
                label: BlockId(0),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(1)),
                source_spans: Vec::new(),
            },
            BasicBlock {
                label: BlockId(1),
                instructions: vec![
                    Instruction::Phi {
                        dest: Value(5),
                        ty: IrType::F32,
                        incoming: vec![
                            (Operand::Value(Value(4)), BlockId(0)),
                            (Operand::Value(Value(11)), BlockId(1)),
                        ],
                    },
                    intr(10, IntrinsicOp::VecMulF32x8, &[1, 2]),
                    intr(11, IntrinsicOp::VecAddF32x8, &[5, 10]),
                    // An independent fusable pair in the same loop body.
                    intr(12, IntrinsicOp::VecMulF32x8, &[1, 3]),
                    intr(13, IntrinsicOp::VecAddF32x8, &[12, 2]),
                ],
                terminator: Terminator::CondBranch {
                    cond: Operand::Value(Value(13)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
                source_spans: Vec::new(),
            },
            BasicBlock {
                label: BlockId(2),
                instructions: vec![],
                terminator: Terminator::Return(Some(Operand::Value(Value(5)))),
                source_spans: Vec::new(),
            },
        ];
        assert_eq!(contract_function(&mut f, FpContract::Fast), 1);
        let ops: Vec<IntrinsicOp> = f.blocks[1]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Intrinsic { op, .. } => Some(op.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            ops,
            vec![
                IntrinsicOp::VecMulF32x8,
                IntrinsicOp::VecAddF32x8,
                IntrinsicOp::VecMaddF32x8
            ]
        );
    }

    #[test]
    fn loop_carried_multiplicand_is_fused() {
        use crate::ir::reexports::Terminator;
        // `s = s*a + c`: header phi %5 = φ(%4, %11); body %10 = %5*%1,
        // %11 = %10 + %3.  The accumulator is a MULTIPLICAND, so the
        // multiply already sits on the serial chain — fused (GCC
        // `vfmadd132ps`), unlike the addend shape above.
        let mut f = IrFunction::new("t".to_string(), IrType::Void, vec![], false);
        f.blocks = vec![
            BasicBlock {
                label: BlockId(0),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(1)),
                source_spans: Vec::new(),
            },
            BasicBlock {
                label: BlockId(1),
                instructions: vec![
                    Instruction::Phi {
                        dest: Value(5),
                        ty: IrType::F32,
                        incoming: vec![
                            (Operand::Value(Value(4)), BlockId(0)),
                            (Operand::Value(Value(11)), BlockId(1)),
                        ],
                    },
                    intr(10, IntrinsicOp::VecMulF32x8, &[5, 1]),
                    intr(11, IntrinsicOp::VecAddF32x8, &[10, 3]),
                ],
                terminator: Terminator::CondBranch {
                    cond: Operand::Value(Value(11)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
                source_spans: Vec::new(),
            },
            BasicBlock {
                label: BlockId(2),
                instructions: vec![],
                terminator: Terminator::Return(Some(Operand::Value(Value(5)))),
                source_spans: Vec::new(),
            },
        ];
        assert_eq!(contract_function(&mut f, FpContract::Fast), 1);
        // The multiply is dropped; phi then the fused op.
        assert_eq!(f.blocks[1].instructions.len(), 2);
        assert!(matches!(
            &f.blocks[1].instructions[1],
            Instruction::Intrinsic { op: IntrinsicOp::VecMaddF32x8, args, .. }
                if args.len() == 3 && args[2] == Operand::Value(Value(3))
        ));
    }

    #[test]
    fn on_expr_requires_matching_statement_tags() {
        let mut f = func(
            vec![
                intr(10, IntrinsicOp::VecMulF32x8, &[1, 2]),
                intr(11, IntrinsicOp::VecAddF32x8, &[3, 10]),
            ],
            11,
        );
        f.fp_expr_tags.insert(10, 7);
        f.fp_expr_tags.insert(11, 8);
        assert_eq!(contract_function(&mut f, FpContract::OnExpr), 0);
        f.fp_expr_tags.insert(11, 7);
        assert_eq!(contract_function(&mut f, FpContract::OnExpr), 1);
        assert_eq!(f.fp_expr_tags.len(), 2, "tags survive the pass");
    }
}
