use super::*;

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn mask_of(width: usize) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        u64::MAX >> (64 - width)
    }
}
fn count_of(op: IrUnaryOp, x: u64, width: usize) -> u64 {
    let x = x & mask_of(width);
    match op {
        IrUnaryOp::Popcount => u64::from(x.count_ones()),
        IrUnaryOp::Parity => u64::from(x.count_ones() & 1),
        _ => unreachable!(),
    }
}

/// Evaluate an instruction list with the IR's own scalar semantics: binops ring
/// at their type, casts extend by the source's signedness, counts read their
/// operand's low `ty` bits.
fn execute(insts: &[Instruction], x: u64) -> u64 {
    let mut values = [0u64; 16];
    values[0] = x;
    let get = |o: &Operand, values: &[u64; 16]| match o {
        Operand::Value(v) => values[v.0 as usize],
        Operand::Const(c) => c.to_i128().unwrap_or(0) as u64,
    };
    for i in insts {
        let value = match i {
            Instruction::Copy { src, .. } => get(src, &values),
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } => {
                let raw = get(src, &values);
                let fw = from_ty.size() * 8;
                let tw = to_ty.size() * 8;
                let raw = raw & mask_of(fw);
                if tw > fw {
                    if from_ty.is_signed() {
                        // sign-extend the narrow pattern
                        let shift = 64 - fw;
                        ((raw << shift) as i64 >> shift) as u64
                    } else {
                        raw
                    }
                } else {
                    raw & mask_of(tw)
                }
            }
            Instruction::BinOp {
                op: IrBinOp::And,
                lhs,
                rhs,
                ty,
                ..
            } => (get(lhs, &values) & get(rhs, &values)) & mask_of(ty.size() * 8),
            Instruction::UnaryOp { op, src, ty, .. } => {
                count_of(*op, get(src, &values), ty.size() * 8)
            }
            other => panic!("unexpected instruction in replacement: {other:?}"),
        };
        values[i.dest().unwrap().0 as usize] = value;
    }
    let dest = insts.last().and_then(|i| i.dest()).unwrap_or(Value(0));
    values[dest.0 as usize]
}

/// Run the fold on `insts`, with `uses` counts as given, on the target named by
/// its ELF machine (x86-64 unless a test wants the narrow-count gate closed).
fn rewrite_on(
    machine: u16,
    insts: &[Instruction],
    uses: &[u32],
    fresh: Value,
) -> Option<Vec<Instruction>> {
    crate::common::types::set_target_elf_machine(machine);
    let mut defs = vec![None; 16];
    for i in insts {
        defs[i.dest().unwrap().0 as usize] = Some(i.clone());
    }
    fold(
        insts.last().unwrap(),
        &Context {
            defs: &defs,
            uses,
            visits: std::cell::Cell::new(512),
            disable_logic: false,
            wrap_signed_overflow: true,
        },
        fresh,
    )
}

const X86_64: u16 = 62;
const AARCH64: u16 = 183;

fn rewrite(insts: &[Instruction], uses: &[u32], fresh: Value) -> Option<Vec<Instruction>> {
    rewrite_on(X86_64, insts, uses, fresh)
}

/// `and(count@W, 1@M)`: the count is private (one use), the constant arrives
/// through the cast the frontend leaves behind.
fn masked(count_ty: IrType, mask_ty: IrType) -> Vec<Instruction> {
    vec![
        Instruction::UnaryOp {
            dest: Value(1),
            op: IrUnaryOp::Popcount,
            src: v(0),
            ty: count_ty,
        },
        Instruction::Cast {
            dest: Value(2),
            src: Operand::Const(IrConst::I64(1)),
            from_ty: IrType::I64,
            to_ty: mask_ty,
        },
        Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::And,
            lhs: v(1),
            rhs: v(2),
            ty: mask_ty,
        },
    ]
}

#[test]
fn a_masked_count_is_parity_at_every_width_pair() {
    let widths = [IrType::U8, IrType::U16, IrType::U32, IrType::U64];
    for count_ty in widths {
        for mask_ty in widths {
            let mut uses = [1u32; 16];
            uses[1] = 1; // the count is read exactly once: by the `and`
            if count_ty == mask_ty {
                // The same-width shape belongs to the simplifier's parity rule
                // (`fold_popcount_mask_to_parity`), which runs before this pass;
                // declining here keeps exactly one owner for the instruction.
                assert!(rewrite(&masked(count_ty, mask_ty), &uses, Value(9)).is_none());
                continue;
            }
            let replacement = rewrite(&masked(count_ty, mask_ty), &uses, Value(9)).unwrap();
            // The result is a parity, at the narrower of the two widths, and the
            // last instruction still defines the consumer's destination.
            assert_eq!(replacement.last().unwrap().dest(), Some(Value(3)));
            assert!(replacement.iter().any(|i| matches!(
                i,
                Instruction::UnaryOp {
                    op: IrUnaryOp::Parity,
                    ..
                }
            )));
            for x in [
                0u64,
                1,
                2,
                3,
                0x7f,
                0x80,
                0xff,
                0x100,
                0x8000,
                0xffff,
                0x8000_0000,
                0xffff_ffff,
                0x8000_0000_0000_0000,
                0xffff_ffff_ffff_ffff,
                0xa5a5_a5a5_a5a5_a5a5,
            ] {
                assert_eq!(
                    execute(&replacement, x),
                    count_of(IrUnaryOp::Parity, x, count_ty.size() * 8),
                    "{count_ty:?}/{mask_ty:?} at {x:#x}"
                );
            }
            for x in 0..256u64 {
                assert_eq!(
                    execute(&replacement, x),
                    u64::from(x.count_ones() & 1),
                    "{count_ty:?}/{mask_ty:?} at {x}"
                );
            }
        }
    }
}

/// `and(popcount@W(zext_{from}(v)), 1)`: the count is private, its operand is a
/// zero extension, and the mask arrives as a narrowed literal.
fn masked_narrow(count_ty: IrType, mask_ty: IrType, from: IrType) -> Vec<Instruction> {
    vec![
        Instruction::Cast {
            dest: Value(1),
            src: v(0),
            from_ty: from,
            to_ty: count_ty,
        },
        Instruction::UnaryOp {
            dest: Value(2),
            op: IrUnaryOp::Popcount,
            src: v(1),
            ty: count_ty,
        },
        Instruction::Cast {
            dest: Value(3),
            src: Operand::Const(IrConst::I64(1)),
            from_ty: IrType::I64,
            to_ty: mask_ty,
        },
        Instruction::BinOp {
            dest: Value(4),
            op: IrBinOp::And,
            lhs: v(2),
            rhs: v(3),
            ty: mask_ty,
        },
    ]
}

#[test]
fn a_masked_count_of_a_zero_extension_is_a_narrow_parity() {
    // `__builtin_parity` of a byte, after promotion to `int`: a 32-bit count of a
    // zero-extended byte, masked by an `int` 1.  Both facts apply at once, and
    // the count narrows to the byte it actually counts.
    for (count_ty, mask_ty, from) in [
        (IrType::U32, IrType::I32, IrType::U8),
        (IrType::U32, IrType::I32, IrType::U16),
        (IrType::U64, IrType::I32, IrType::U8),
        (IrType::U64, IrType::I32, IrType::U16),
    ] {
        let replacement =
            rewrite(&masked_narrow(count_ty, mask_ty, from), &[1; 16], Value(9)).unwrap();
        match &replacement[0] {
            Instruction::UnaryOp {
                op: IrUnaryOp::Parity,
                ty,
                ..
            } => assert_eq!(*ty, from, "the parity must run at {from:?}"),
            other => panic!("expected a narrow parity, got {other:?}"),
        }
        assert_eq!(replacement.last().unwrap().dest(), Some(Value(4)));
        for x in 0..=0xffffu64 {
            assert_eq!(
                execute(&replacement, x),
                count_of(
                    IrUnaryOp::Parity,
                    x & mask_of(from.size() * 8),
                    from.size() * 8
                ) & mask_of(mask_ty.size() * 8),
                "{count_ty:?}/{mask_ty:?} from {from:?} at {x}"
            );
        }
    }
}

/// `count(zext_narrow(v))`: the source is a zero extension.
fn narrowed(op: IrUnaryOp, from: IrType, to: IrType) -> Vec<Instruction> {
    vec![
        Instruction::Cast {
            dest: Value(1),
            src: v(0),
            from_ty: from,
            to_ty: to,
        },
        Instruction::UnaryOp {
            dest: Value(2),
            op,
            src: v(1),
            ty: to,
        },
    ]
}

/// `(from, to)` pairs the narrow-count rule accepts, per operation.  A parity is
/// shorter at any narrower width; a popcount only when the narrower scan is a
/// different algorithm (a byte's multiplier form, or 32 bits instead of 64), so
/// a 16-bit count over an already-promoted value is left promoted.
fn narrows(op: IrUnaryOp, from: IrType, to: IrType) -> bool {
    match op {
        IrUnaryOp::Parity => true,
        IrUnaryOp::Popcount => from.size() == 1 || (to.size() == 8 && from.size() <= 4),
        _ => false,
    }
}

#[test]
fn a_count_of_a_zero_extension_runs_at_the_narrow_width() {
    for op in [IrUnaryOp::Popcount, IrUnaryOp::Parity] {
        for (from, to) in [
            (IrType::U8, IrType::U32),
            (IrType::U16, IrType::U32),
            (IrType::U8, IrType::U64),
            (IrType::U16, IrType::U64),
            (IrType::U32, IrType::U64),
        ] {
            if !narrows(op, from, to) {
                assert!(rewrite(&narrowed(op, from, to), &[1; 16], Value(9)).is_none());
                continue;
            }
            let replacement = rewrite(&narrowed(op, from, to), &[1; 16], Value(9)).unwrap();
            match &replacement[0] {
                Instruction::UnaryOp { op: o, ty, .. } => {
                    assert_eq!(*o, op);
                    assert_eq!(*ty, from, "the count must run at {from:?}");
                }
                other => panic!("expected the narrow count first, got {other:?}"),
            }
            assert!(matches!(
                replacement[1],
                Instruction::Cast {
                    from_ty: f,
                    to_ty: t,
                    ..
                } if f == from && t == to
            ));
            for x in 0..=0xffffu64 {
                assert_eq!(
                    execute(&replacement, x),
                    count_of(op, x & mask_of(from.size() * 8), to.size() * 8),
                    "{op:?} {from:?}->{to:?} at {x}"
                );
            }
        }
    }
}

#[test]
fn sign_extension_shared_counts_and_other_masks_are_refused() {
    // A sign extension is not a zero extension: the count would see the added
    // high ones.
    assert!(
        rewrite(
            &narrowed(IrUnaryOp::Popcount, IrType::I8, IrType::I32),
            &[1; 16],
            Value(9)
        )
        .is_none()
    );
    assert!(
        rewrite(
            &narrowed(IrUnaryOp::Parity, IrType::I16, IrType::I32),
            &[1; 16],
            Value(9)
        )
        .is_none()
    );
    // A shared count keeps its other use.
    let mut uses = [1u32; 16];
    uses[1] = 2;
    assert!(rewrite(&masked(IrType::U64, IrType::U32), &uses, Value(9)).is_none());
    // Only the constant 1 is the parity question.
    let mut shape = masked(IrType::U64, IrType::U32);
    shape[1] = Instruction::Cast {
        dest: Value(2),
        src: Operand::Const(IrConst::I64(3)),
        from_ty: IrType::I64,
        to_ty: IrType::U32,
    };
    assert!(rewrite(&shape, &[1; 16], Value(9)).is_none());
    // A count shared at the same width is the simplifier's shape, not this one.
    let mut same = masked(IrType::U32, IrType::U32);
    same[1] = Instruction::Cast {
        dest: Value(2),
        src: Operand::Const(IrConst::I64(1)),
        from_ty: IrType::I64,
        to_ty: IrType::U32,
    };
    assert!(rewrite(&same, &[1; 16], Value(9)).is_none());
}

#[test]
fn replacements_use_the_fresh_id_contiguously() {
    // The driver hands out `fresh` and advances by the number of temporaries,
    // so every intermediate must be `fresh` itself (one temporary per rewrite).
    for insts in [
        masked(IrType::U64, IrType::U32),
        narrowed(IrUnaryOp::Parity, IrType::U8, IrType::U32),
    ] {
        let replacement = rewrite(&insts, &[1; 16], Value(9)).unwrap();
        assert_eq!(replacement.len(), 2);
        assert_eq!(replacement[0].dest(), Some(Value(9)));
    }
}

#[test]
fn other_targets_keep_the_promoted_form() {
    // The rewrite is sound everywhere, but only the x86 back ends widen a narrow
    // count for free; the Arm and Riscv64 emitters mask and copy, so narrowing
    // there would pay for the extension and save nothing.
    for insts in [
        narrowed(IrUnaryOp::Parity, IrType::U8, IrType::U32),
        narrowed(IrUnaryOp::Popcount, IrType::U8, IrType::U32),
    ] {
        assert!(rewrite_on(AARCH64, &insts, &[1; 16], Value(9)).is_none());
    }
    // The masked count is a win on every target -- it replaces a 64-bit scan
    // plus an `and` with a 64-bit parity -- it just does not narrow there.
    let replacement = rewrite_on(
        AARCH64,
        &masked(IrType::U64, IrType::U32),
        &[1; 16],
        Value(9),
    )
    .unwrap();
    match &replacement[0] {
        Instruction::UnaryOp {
            op: IrUnaryOp::Parity,
            ty,
            ..
        } => assert_eq!(*ty, IrType::U64),
        other => panic!("expected a full-width parity, got {other:?}"),
    }
}
