//! Tests for the invariant-index hoist.
//!
//! Every rewrite is checked twice: structurally (what moved where), and by
//! translation validation -- the function is executed before and after on a
//! small reference interpreter and must make the same stores to the same
//! addresses, for inputs that include negative indices and zero-trip loops.
//! The interpreter panics on a value read before its definition, so a
//! hoisted operand that does not dominate the preheader fails the test even
//! if the verifier misses it.

use super::*;
use crate::common::types::AddressSpace;
use crate::ir::reexports::{BasicBlock, BlockId, IrCmpOp, IrConst, Terminator};

const BASE: u32 = 0;
const INV: u32 = 1;
const N: u32 = 2;
const J: u32 = 3;
const J_NEXT: u32 = 4;
const TEST: u32 = 5;
const BASE2: u32 = 6;

fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(label),
        instructions,
        terminator,
        source_spans: Vec::new(),
        source_inline_ids: Vec::new(),
    }
}

fn val(id: u32) -> Operand {
    Operand::Value(Value(id))
}

fn int(ty: IrType, value: i64) -> Operand {
    Operand::Const(match ty {
        IrType::I32 | IrType::U32 => IrConst::I32(value as i32),
        _ => IrConst::I64(value),
    })
}

fn binop(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest: Value(dest),
        op,
        lhs,
        rhs,
        ty,
    }
}

fn cast(dest: u32, src: u32, from_ty: IrType, to_ty: IrType) -> Instruction {
    Instruction::Cast {
        dest: Value(dest),
        src: val(src),
        from_ty,
        to_ty,
    }
}

fn gep(dest: u32, base: u32, offset: u32) -> Instruction {
    Instruction::GetElementPtr {
        dest: Value(dest),
        base: Value(base),
        offset: val(offset),
        ty: IrType::I64,
    }
}

fn store(value: Operand, ptr: u32) -> Instruction {
    Instruction::Store {
        val: value,
        ptr: Value(ptr),
        ty: IrType::I64,
        seg_override: AddressSpace::Default,
        volatile: false,
    }
}

/// A counted loop over `j` in `[0, n)`:
///
/// ```text
///   B0: base, inv, n, base2 = params                 -> B1
///   B1: j = phi [0, B0], [j', B2];  j < n ?          -> B2 : B3
///   B2: body;  j' = j + 1                            -> B1
///   B3: return
/// ```
fn counted_loop(index_ty: IrType, body: Vec<Instruction>) -> IrFunction {
    let param = |dest: u32, index: usize, ty: IrType| Instruction::ParamRef {
        dest: Value(dest),
        param_idx: index,
        ty,
    };
    let mut body = body;
    body.push(binop(
        J_NEXT,
        IrBinOp::Add,
        val(J),
        int(index_ty, 1),
        index_ty,
    ));
    let mut func = IrFunction::new("f".to_string(), IrType::I32, vec![], false);
    func.blocks = vec![
        block(
            0,
            vec![
                param(BASE, 0, IrType::Ptr),
                param(INV, 1, index_ty),
                param(N, 2, index_ty),
                param(BASE2, 3, IrType::Ptr),
            ],
            Terminator::Branch(BlockId(1)),
        ),
        block(
            1,
            vec![
                Instruction::Phi {
                    dest: Value(J),
                    ty: index_ty,
                    incoming: vec![(int(index_ty, 0), BlockId(0)), (val(J_NEXT), BlockId(2))],
                },
                Instruction::Cmp {
                    dest: Value(TEST),
                    op: IrCmpOp::Slt,
                    lhs: val(J),
                    rhs: val(N),
                    ty: index_ty,
                },
            ],
            Terminator::CondBranch {
                cond: val(TEST),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ),
        block(2, body, Terminator::Branch(BlockId(1))),
        block(3, Vec::new(), Terminator::Return(None)),
    ];
    func.next_value_id = 100;
    func.next_label = 4;
    func
}

/// `base[inv + j] = j` with a 32-bit signed index: the shape of the header.
fn int_index_body() -> Vec<Instruction> {
    vec![
        binop(10, IrBinOp::Add, val(INV), val(J), IrType::I32),
        cast(11, 10, IrType::I32, IrType::I64),
        binop(12, IrBinOp::Shl, val(11), int(IrType::I64, 3), IrType::I64),
        gep(13, BASE, 12),
        store(val(J), 13),
    ]
}

// ---------------------------------------------------------------------------
// Reference interpreter
// ---------------------------------------------------------------------------

fn width(ty: IrType) -> u32 {
    match ty {
        IrType::I8 | IrType::U8 => 8,
        IrType::I16 | IrType::U16 => 16,
        IrType::I32 | IrType::U32 => 32,
        _ => 64,
    }
}

fn wrap(value: u64, ty: IrType) -> u64 {
    match width(ty) {
        64 => value,
        bits => value & ((1u64 << bits) - 1),
    }
}

fn sign_extend(value: u64, ty: IrType) -> u64 {
    let shift = 64 - width(ty);
    (((value << shift) as i64) >> shift) as u64
}

fn read(values: &FxHashMap<u32, u64>, operand: &Operand) -> u64 {
    match operand {
        Operand::Value(v) => *values
            .get(&v.0)
            .unwrap_or_else(|| panic!("value {} read before its definition", v.0)),
        Operand::Const(c) => c.to_i64().expect("integer constant") as u64,
    }
}

/// Every store the function makes, as `(address, value)` in execution order.
fn stores(func: &IrFunction, params: &[u64]) -> Vec<(u64, u64)> {
    let index_of = |label: BlockId| {
        func.blocks
            .iter()
            .position(|b| b.label == label)
            .expect("branch target exists")
    };
    let mut values: FxHashMap<u32, u64> = FxHashMap::default();
    let mut trace = Vec::new();
    let mut current = 0;
    let mut previous = BlockId(u32::MAX);
    for _ in 0..100_000 {
        let block = &func.blocks[current];
        // Phis read their inputs simultaneously on entry.
        let entering: Vec<(u32, u64)> = block
            .instructions
            .iter()
            .filter_map(|inst| match inst {
                Instruction::Phi { dest, ty, incoming } => {
                    let (operand, _) = incoming
                        .iter()
                        .find(|(_, from)| *from == previous)
                        .expect("phi has an input for the edge taken");
                    Some((dest.0, wrap(read(&values, operand), *ty)))
                }
                _ => None,
            })
            .collect();
        values.extend(entering);
        for inst in &block.instructions {
            match inst {
                Instruction::Phi { .. } => {}
                Instruction::ParamRef {
                    dest,
                    param_idx,
                    ty,
                } => {
                    // Parameters past the sample are further array bases.
                    let value = params
                        .get(*param_idx)
                        .copied()
                        .unwrap_or(0x7200_0000_0000 + ((*param_idx as u64) << 36));
                    values.insert(dest.0, wrap(value, *ty));
                }
                Instruction::Copy { dest, src } => {
                    let v = read(&values, src);
                    values.insert(dest.0, v);
                }
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    let v = wrap(read(&values, src), *from_ty);
                    let extended = if from_ty.is_signed() {
                        sign_extend(v, *from_ty)
                    } else {
                        v
                    };
                    values.insert(dest.0, wrap(extended, *to_ty));
                }
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let (a, b) = (read(&values, lhs), read(&values, rhs));
                    let result = match op {
                        IrBinOp::Add => a.wrapping_add(b),
                        IrBinOp::Sub => a.wrapping_sub(b),
                        IrBinOp::Mul => a.wrapping_mul(b),
                        IrBinOp::Shl => a.wrapping_shl(b as u32),
                        other => panic!("interpreter has no {other:?}"),
                    };
                    values.insert(dest.0, wrap(result, *ty));
                }
                Instruction::Cmp {
                    dest,
                    op: IrCmpOp::Slt,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let a = sign_extend(wrap(read(&values, lhs), *ty), *ty) as i64;
                    let b = sign_extend(wrap(read(&values, rhs), *ty), *ty) as i64;
                    values.insert(dest.0, u64::from(a < b));
                }
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => {
                    let address =
                        read(&values, &Operand::Value(*base)).wrapping_add(read(&values, offset));
                    values.insert(dest.0, address);
                }
                Instruction::Store { val, ptr, .. } => {
                    trace.push((read(&values, &Operand::Value(*ptr)), read(&values, val)));
                }
                other => panic!("interpreter has no {other:?}"),
            }
        }
        previous = block.label;
        current = match &block.terminator {
            Terminator::Return(_) => return trace,
            Terminator::Branch(target) => index_of(*target),
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } => index_of(if read(&values, cond) != 0 {
                *true_label
            } else {
                *false_label
            }),
            other => panic!("interpreter has no {other:?}"),
        };
    }
    panic!("no return within the step budget");
}

/// Parameter sets `(base, inv, n, base2)`: positive and negative invariant
/// parts, a zero-trip loop, and sums near the top of the `int` range that
/// stay inside it (the signed form is only defined there).
fn samples() -> Vec<[u64; 4]> {
    let base = 0x7000_0000_0000u64;
    let base2 = 0x7100_0000_0000u64;
    [
        (0i64, 5i64),
        (7, 5),
        (-3, 6),
        (-1000, 3),
        (123_456, 0),
        (i64::from(i32::MAX) - 8, 8),
        (i64::from(i32::MIN), 4),
    ]
    .into_iter()
    .map(|(inv, n)| [base, inv as u64, n as u64, base2])
    .collect()
}

fn assert_verifies(func: &IrFunction) {
    let mut violations = Vec::new();
    crate::passes::verify::verify_function(func, "index_hoist_test", &mut violations);
    assert!(violations.is_empty(), "verifier: {violations:?}");
}

/// Run the pass, require `expected` rewrites, a clean verifier and the same
/// stores as before on every sample.
fn check(func: &mut IrFunction, expected: usize) {
    let before: Vec<_> = samples().iter().map(|p| stores(func, p)).collect();
    assert_verifies(func);
    let rewritten = run(func);
    assert_eq!(rewritten, expected, "number of rewritten addresses");
    assert_verifies(func);
    for (params, before) in samples().iter().zip(&before) {
        assert_eq!(
            &stores(func, params),
            before,
            "stores differ for parameters {params:?}"
        );
    }
    // A second run finds nothing left to split.
    assert_eq!(run(func), 0, "the pass must be idempotent");
}

fn count(block: &BasicBlock, pred: impl Fn(&Instruction) -> bool) -> usize {
    block.instructions.iter().filter(|i| pred(i)).count()
}

fn is_gep(inst: &Instruction) -> bool {
    matches!(inst, Instruction::GetElementPtr { .. })
}

fn is_add_of(inst: &Instruction, a: u32, b: u32) -> bool {
    matches!(
        inst,
        Instruction::BinOp { op: IrBinOp::Add, lhs: Operand::Value(x), rhs: Operand::Value(y), .. }
            if (x.0, y.0) == (a, b) || (x.0, y.0) == (b, a)
    )
}

// ---------------------------------------------------------------------------
// What is rewritten
// ---------------------------------------------------------------------------

#[test]
fn signed_int_sum_is_split() {
    let mut func = counted_loop(IrType::I32, int_index_body());
    check(&mut func, 1);

    // Preheader: sext inv, scale, hoisted base -- after the parameters.
    let pre = &func.blocks[0].instructions;
    assert_eq!(pre.len(), 7, "{pre:?}");
    assert!(matches!(
        &pre[4],
        Instruction::Cast { src: Operand::Value(v), from_ty: IrType::I32, to_ty: IrType::I64, .. }
            if v.0 == INV
    ));
    assert!(matches!(
        &pre[5],
        Instruction::BinOp {
            op: IrBinOp::Shl,
            rhs: Operand::Const(_),
            ty: IrType::I64,
            ..
        }
    ));
    let hoisted = match &pre[6] {
        Instruction::GetElementPtr {
            dest,
            base,
            ty: IrType::I64,
            ..
        } if base.0 == BASE => *dest,
        other => panic!("expected the hoisted base, got {other:?}"),
    };

    // Body: the access is indexed by the counter alone, off the hoisted
    // base; the sum, its extension and its scale are gone.
    let body = &func.blocks[2];
    assert_eq!(count(body, |i| is_add_of(i, INV, J)), 0, "the sum must die");
    assert!(body.instructions.iter().any(|i| matches!(
        i,
        Instruction::Cast { src: Operand::Value(v), from_ty: IrType::I32, .. } if v.0 == J
    )));
    assert!(
        body.instructions
            .iter()
            .any(|i| matches!(i, Instruction::GetElementPtr { base, .. } if *base == hoisted))
    );
    assert_eq!(body.instructions.len(), 5, "{:?}", body.instructions);
}

#[test]
fn operand_order_of_the_sum_does_not_matter() {
    let mut body = int_index_body();
    body[0] = binop(10, IrBinOp::Add, val(J), val(INV), IrType::I32);
    let mut func = counted_loop(IrType::I32, body);
    check(&mut func, 1);
}

#[test]
fn pointer_width_sum_is_split_without_a_cast() {
    for ty in [IrType::I64, IrType::U64] {
        let body = vec![
            binop(10, IrBinOp::Add, val(INV), val(J), ty),
            binop(12, IrBinOp::Shl, val(10), int(IrType::I64, 3), ty),
            gep(13, BASE, 12),
            store(val(J), 13),
        ];
        let mut func = counted_loop(IrType::I64, body);
        check(&mut func, 1);
        assert_eq!(
            count(&func.blocks[0], |i| matches!(i, Instruction::Cast { .. })),
            0
        );
        assert_eq!(count(&func.blocks[0], is_gep), 1);
    }
}

#[test]
fn unscaled_and_multiplied_indices_are_split() {
    // A byte array: no scale at all.
    let body = vec![
        binop(10, IrBinOp::Add, val(INV), val(J), IrType::I32),
        cast(11, 10, IrType::I32, IrType::I64),
        gep(13, BASE, 11),
        store(val(J), 13),
    ];
    check(&mut counted_loop(IrType::I32, body), 1);

    // A 24-byte element: `mul`, with the constant on either side.
    for const_first in [false, true] {
        let (lhs, rhs) = if const_first {
            (int(IrType::I64, 24), val(11))
        } else {
            (val(11), int(IrType::I64, 24))
        };
        let body = vec![
            binop(10, IrBinOp::Add, val(INV), val(J), IrType::I32),
            cast(11, 10, IrType::I32, IrType::I64),
            binop(12, IrBinOp::Mul, lhs, rhs, IrType::I64),
            gep(13, BASE, 12),
            store(val(J), 13),
        ];
        let mut func = counted_loop(IrType::I32, body);
        check(&mut func, 1);
        assert_eq!(
            count(&func.blocks[0], |i| matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::Mul,
                    ..
                }
            )),
            1,
            "the invariant half is scaled in the preheader"
        );
    }
}

#[test]
fn sum_with_another_reader_survives_and_the_access_is_still_split() {
    let mut body = int_index_body();
    // The sum is also stored as a value: it keeps a reader of its own.
    body.push(cast(21, 10, IrType::I32, IrType::I64));
    body.push(store(val(21), BASE2));
    let mut func = counted_loop(IrType::I32, body);
    check(&mut func, 1);
    assert_eq!(
        count(&func.blocks[2], |i| is_add_of(i, INV, J)),
        1,
        "a sum that is still read must stay"
    );
}

#[test]
fn two_arrays_share_one_index_and_get_one_base_each() {
    // a[inv + j] and b[inv + j] through ONE extension and ONE scale: both
    // readers of each are rewritten, so the chain still dies.
    let body = vec![
        binop(10, IrBinOp::Add, val(INV), val(J), IrType::I32),
        cast(11, 10, IrType::I32, IrType::I64),
        binop(12, IrBinOp::Shl, val(11), int(IrType::I64, 3), IrType::I64),
        gep(13, BASE, 12),
        store(val(J), 13),
        gep(14, BASE2, 12),
        store(val(N), 14),
    ];
    let mut func = counted_loop(IrType::I32, body);
    check(&mut func, 2);
    assert_eq!(
        count(&func.blocks[0], is_gep),
        2,
        "one hoisted base per array"
    );
    let body = &func.blocks[2];
    assert_eq!(count(body, |i| is_add_of(i, INV, J)), 0);
    assert_eq!(
        count(body, |i| matches!(i, Instruction::Cast { .. })),
        1,
        "the variant index is built once and shared within the block"
    );
}

#[test]
fn same_array_and_addend_share_one_hoisted_base() {
    // Two separate chains for the same `base[inv + j]`.
    let mut body = int_index_body();
    body.extend([
        binop(20, IrBinOp::Add, val(INV), val(J), IrType::I32),
        cast(21, 20, IrType::I32, IrType::I64),
        binop(22, IrBinOp::Shl, val(21), int(IrType::I64, 3), IrType::I64),
        gep(23, BASE, 22),
        store(val(N), 23),
    ]);
    let mut func = counted_loop(IrType::I32, body);
    check(&mut func, 2);
    assert_eq!(count(&func.blocks[0], is_gep), 1);
}

/// One access per scale: `count` different hoisted bases off ONE array and
/// ONE addend.
fn one_array_many_scales(count: u32) -> Vec<Instruction> {
    let mut body = Vec::new();
    for k in 0..count {
        let v = 20 + k * 10;
        body.extend([
            binop(v, IrBinOp::Add, val(INV), val(J), IrType::I32),
            cast(v + 1, v, IrType::I32, IrType::I64),
            binop(
                v + 2,
                IrBinOp::Mul,
                val(v + 1),
                int(IrType::I64, 8 * (i64::from(k) + 1)),
                IrType::I64,
            ),
            gep(v + 3, BASE, v + 2),
            store(val(J), v + 3),
        ]);
    }
    body
}

#[test]
fn budget_of_hoisted_bases_is_all_or_nothing() {
    // Exactly the budget: every access is split.
    let at_budget = MAX_BASES_PER_LOOP as u32;
    let mut func = counted_loop(IrType::I32, one_array_many_scales(at_budget));
    check(&mut func, MAX_BASES_PER_LOOP);
    assert_eq!(count(&func.blocks[0], is_gep), MAX_BASES_PER_LOOP);

    // One more, and they would replace only two live values (the array and
    // the addend): the loop is left alone entirely, not half rewritten.
    let mut func = counted_loop(IrType::I32, one_array_many_scales(at_budget + 1));
    check_untouched(&mut func);
}

#[test]
fn many_arrays_with_one_addend_are_all_split() {
    // Eight arrays indexed by one `inv + j`: eight hoisted bases replace
    // eight bases and one addend, so the budget does not apply.
    const ARRAYS: u32 = 8;
    let mut body = vec![
        binop(10, IrBinOp::Add, val(INV), val(J), IrType::I32),
        cast(11, 10, IrType::I32, IrType::I64),
        binop(12, IrBinOp::Shl, val(11), int(IrType::I64, 3), IrType::I64),
    ];
    for k in 0..ARRAYS {
        body.push(gep(60 + k, 40 + k, 12));
        body.push(store(val(J), 60 + k));
    }
    let mut func = counted_loop(IrType::I32, body);
    for k in 0..ARRAYS {
        func.blocks[0].instructions.push(Instruction::ParamRef {
            dest: Value(40 + k),
            param_idx: 4 + k as usize,
            ty: IrType::Ptr,
        });
    }
    assert!(ARRAYS as usize > MAX_BASES_PER_LOOP);
    check(&mut func, ARRAYS as usize);
    assert_eq!(count(&func.blocks[0], is_gep), ARRAYS as usize);
    assert_eq!(count(&func.blocks[2], |i| is_add_of(i, INV, J)), 0);
}

/// `for i { for j { base[i + j] = j } }`: the inner loop's invariant part is
/// the outer counter, and the hoisted base lands in the inner preheader --
/// inside the outer loop, once per outer iteration.
#[test]
fn nested_loop_hoists_into_the_inner_preheader() {
    const I: u32 = 30;
    const I_NEXT: u32 = 31;
    const OUTER_TEST: u32 = 32;
    let param = |dest: u32, index: usize, ty: IrType| Instruction::ParamRef {
        dest: Value(dest),
        param_idx: index,
        ty,
    };
    let mut func = IrFunction::new("nested".to_string(), IrType::I32, vec![], false);
    func.blocks = vec![
        block(
            0,
            vec![
                param(BASE, 0, IrType::Ptr),
                param(INV, 1, IrType::I32),
                param(N, 2, IrType::I32),
                param(BASE2, 3, IrType::Ptr),
            ],
            Terminator::Branch(BlockId(1)),
        ),
        // Outer header.
        block(
            1,
            vec![
                Instruction::Phi {
                    dest: Value(I),
                    ty: IrType::I32,
                    incoming: vec![(int(IrType::I32, 0), BlockId(0)), (val(I_NEXT), BlockId(5))],
                },
                Instruction::Cmp {
                    dest: Value(OUTER_TEST),
                    op: IrCmpOp::Slt,
                    lhs: val(I),
                    rhs: val(N),
                    ty: IrType::I32,
                },
            ],
            Terminator::CondBranch {
                cond: val(OUTER_TEST),
                true_label: BlockId(2),
                false_label: BlockId(6),
            },
        ),
        // Inner preheader.
        block(2, Vec::new(), Terminator::Branch(BlockId(3))),
        // Inner header.
        block(
            3,
            vec![
                Instruction::Phi {
                    dest: Value(J),
                    ty: IrType::I32,
                    incoming: vec![(int(IrType::I32, 0), BlockId(2)), (val(J_NEXT), BlockId(4))],
                },
                Instruction::Cmp {
                    dest: Value(TEST),
                    op: IrCmpOp::Slt,
                    lhs: val(J),
                    rhs: val(N),
                    ty: IrType::I32,
                },
            ],
            Terminator::CondBranch {
                cond: val(TEST),
                true_label: BlockId(4),
                false_label: BlockId(5),
            },
        ),
        // Inner body.
        block(
            4,
            vec![
                binop(10, IrBinOp::Add, val(I), val(J), IrType::I32),
                cast(11, 10, IrType::I32, IrType::I64),
                binop(12, IrBinOp::Shl, val(11), int(IrType::I64, 3), IrType::I64),
                gep(13, BASE, 12),
                store(val(J), 13),
                binop(
                    J_NEXT,
                    IrBinOp::Add,
                    val(J),
                    int(IrType::I32, 1),
                    IrType::I32,
                ),
            ],
            Terminator::Branch(BlockId(3)),
        ),
        // Outer latch.
        block(
            5,
            vec![binop(
                I_NEXT,
                IrBinOp::Add,
                val(I),
                int(IrType::I32, 1),
                IrType::I32,
            )],
            Terminator::Branch(BlockId(1)),
        ),
        block(6, Vec::new(), Terminator::Return(None)),
    ];
    func.next_value_id = 100;
    func.next_label = 7;
    check(&mut func, 1);
    assert_eq!(count(&func.blocks[2], is_gep), 1, "inner preheader");
    assert_eq!(count(&func.blocks[0], is_gep), 0, "not the function entry");
    assert_eq!(count(&func.blocks[4], |i| is_add_of(i, I, J)), 0);
}

// ---------------------------------------------------------------------------
// What is left alone
// ---------------------------------------------------------------------------

/// Run the pass and require that it changed nothing at all.
fn check_untouched(func: &mut IrFunction) {
    let before = format!("{:?}", func.blocks);
    assert_eq!(run(func), 0);
    assert_eq!(before, format!("{:?}", func.blocks));
}

#[test]
fn wrapv_declines_the_int_sum_but_not_the_pointer_width_sum() {
    let mut func = counted_loop(IrType::I32, int_index_body());
    func.pass_policy.wrap_signed_overflow = true;
    check_untouched(&mut func);

    let body = vec![
        binop(10, IrBinOp::Add, val(INV), val(J), IrType::I64),
        binop(12, IrBinOp::Shl, val(10), int(IrType::I64, 3), IrType::I64),
        gep(13, BASE, 12),
        store(val(J), 13),
    ];
    let mut func = counted_loop(IrType::I64, body);
    func.pass_policy.wrap_signed_overflow = true;
    check(&mut func, 1);
}

#[test]
fn unsigned_int_sum_is_never_split() {
    // `unsigned` wraps by definition: zext(a + b) != zext(a) + zext(b).
    let body = vec![
        binop(10, IrBinOp::Add, val(INV), val(J), IrType::U32),
        cast(11, 10, IrType::U32, IrType::I64),
        binop(12, IrBinOp::Shl, val(11), int(IrType::I64, 3), IrType::I64),
        gep(13, BASE, 12),
        store(val(J), 13),
    ];
    check_untouched(&mut counted_loop(IrType::U32, body));
}

#[test]
fn int_sum_computed_in_the_unsigned_ring_is_never_split() {
    // `(int)((unsigned)inv + (unsigned)j)`: the reinterpretation back to
    // `int` sits between the sum and the extension, and the sum wraps.
    let body = vec![
        binop(10, IrBinOp::Add, val(INV), val(J), IrType::U32),
        cast(9, 10, IrType::U32, IrType::I32),
        cast(11, 9, IrType::I32, IrType::I64),
        binop(12, IrBinOp::Shl, val(11), int(IrType::I64, 3), IrType::I64),
        gep(13, BASE, 12),
        store(val(J), 13),
    ];
    check_untouched(&mut counted_loop(IrType::I32, body));
}

#[test]
fn extension_or_scale_with_another_reader_is_left_alone() {
    // The extended sum is also stored.
    let mut body = int_index_body();
    body.push(store(val(11), BASE2));
    check_untouched(&mut counted_loop(IrType::I32, body));

    // The scaled sum is also stored.
    let mut body = int_index_body();
    body.push(store(val(12), BASE2));
    check_untouched(&mut counted_loop(IrType::I32, body));
}

#[test]
fn constant_addend_is_left_to_the_widener() {
    let mut body = int_index_body();
    body[0] = binop(10, IrBinOp::Add, val(J), int(IrType::I32, 1), IrType::I32);
    check_untouched(&mut counted_loop(IrType::I32, body));
}

#[test]
fn two_variant_or_two_invariant_addends_are_left_alone() {
    let mut body = int_index_body();
    body[0] = binop(10, IrBinOp::Add, val(J), val(J), IrType::I32);
    check_untouched(&mut counted_loop(IrType::I32, body));

    let mut body = int_index_body();
    body[0] = binop(10, IrBinOp::Add, val(INV), val(N), IrType::I32);
    check_untouched(&mut counted_loop(IrType::I32, body));
}

#[test]
fn base_defined_in_the_loop_is_left_alone() {
    let mut body = int_index_body();
    // The array base is recomputed every iteration.
    body.insert(0, gep(9, BASE2, J));
    body[4] = gep(13, 9, 12);
    check_untouched(&mut counted_loop(IrType::I32, body));
}

#[test]
fn subtraction_and_variable_scale_are_left_alone() {
    let mut body = int_index_body();
    body[0] = binop(10, IrBinOp::Sub, val(INV), val(J), IrType::I32);
    check_untouched(&mut counted_loop(IrType::I32, body));

    let mut body = int_index_body();
    body[2] = binop(12, IrBinOp::Mul, val(11), val(11), IrType::I64);
    check_untouched(&mut counted_loop(IrType::I32, body));
}

#[test]
fn loop_without_a_unique_outside_predecessor_is_left_alone() {
    // A second way into the header: B0 branches to B1 or to a new B4 that
    // also jumps to B1, so no single block can hold the hoisted code.
    let mut func = counted_loop(IrType::I32, int_index_body());
    func.blocks[0].terminator = Terminator::CondBranch {
        cond: val(N),
        true_label: BlockId(1),
        false_label: BlockId(4),
    };
    func.blocks
        .push(block(4, Vec::new(), Terminator::Branch(BlockId(1))));
    func.next_label = 5;
    for inst in &mut func.blocks[1].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            incoming.push((int(IrType::I32, 0), BlockId(4)));
        }
    }
    assert_verifies(&func);
    check_untouched(&mut func);
}

#[test]
fn function_without_a_loop_is_left_alone() {
    let mut func = IrFunction::new("straight".to_string(), IrType::I32, vec![], false);
    func.blocks = vec![
        block(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(BASE),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(INV),
                    param_idx: 1,
                    ty: IrType::I32,
                },
                Instruction::ParamRef {
                    dest: Value(N),
                    param_idx: 2,
                    ty: IrType::I32,
                },
            ],
            Terminator::Branch(BlockId(1)),
        ),
        block(
            1,
            vec![
                binop(10, IrBinOp::Add, val(INV), val(N), IrType::I32),
                cast(11, 10, IrType::I32, IrType::I64),
                gep(13, BASE, 11),
                store(val(N), 13),
            ],
            Terminator::Return(None),
        ),
    ];
    func.next_value_id = 100;
    func.next_label = 2;
    check_untouched(&mut func);
}
