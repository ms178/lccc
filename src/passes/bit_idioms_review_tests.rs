//! PR791 review: bit-precise IR fixtures, hostile widths, and interacting rewrites.
use super::*;
use crate::common::source::Span;
use crate::ir::reexports::{BasicBlock, BlockId, Terminator, Value};

struct Graph {
    insts: Vec<Instruction>,
    next: u32,
}
impl Graph {
    fn new() -> Self {
        Self {
            insts: Vec::new(),
            next: 0,
        }
    }
    fn put(&mut self, f: impl FnOnce(Value) -> Instruction) -> Operand {
        let v = Value(self.next);
        self.next += 1;
        self.insts.push(f(v));
        Operand::Value(v)
    }
    fn param(&mut self, idx: usize, ty: IrType) -> Operand {
        self.put(|dest| Instruction::ParamRef {
            dest,
            param_idx: idx,
            ty,
        })
    }
    fn bin(&mut self, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Operand {
        self.put(|dest| Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        })
    }
    fn cmp(&mut self, op: IrCmpOp, lhs: Operand, rhs: Operand, ty: IrType) -> Operand {
        self.put(|dest| Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        })
    }
    fn sel(&mut self, cond: Operand, true_val: Operand, false_val: Operand, ty: IrType) -> Operand {
        self.put(|dest| Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        })
    }
    fn cast(&mut self, src: Operand, from_ty: IrType, to_ty: IrType) -> Operand {
        self.put(|dest| Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        })
    }
    fn count(&mut self, mut x: Operand, trailing: bool) -> Operand {
        let mut n = c(0);
        for amount in if trailing {
            vec![32, 16, 8, 4, 2, 1]
        } else {
            vec![16, 8, 4, 2, 1]
        } {
            let cond = if trailing {
                let masked = self.bin(IrBinOp::And, x, c((1u64 << amount) - 1), IrType::U64);
                self.cmp(IrCmpOp::Eq, masked, c(0), IrType::U64)
            } else {
                self.cmp(IrCmpOp::Ule, x, c(u32::MAX as u64 >> amount), IrType::U32)
            };
            let added = self.bin(IrBinOp::Add, n, c(amount), IrType::U32);
            if amount != 1 {
                let ty = if trailing { IrType::U64 } else { IrType::U32 };
                let shifted = self.bin(
                    if trailing {
                        IrBinOp::LShr
                    } else {
                        IrBinOp::Shl
                    },
                    x,
                    c(amount),
                    ty,
                );
                x = self.sel(cond, shifted, x, ty);
            }
            n = self.sel(cond, added, n, IrType::U32);
        }
        n
    }
    fn swap(&mut self, x: Operand, amount: u64, mask: u64) -> Operand {
        let r = self.bin(IrBinOp::LShr, x, c(amount), IrType::U32);
        let r = self.bin(IrBinOp::And, r, c(mask), IrType::U32);
        let l = self.bin(IrBinOp::And, x, c(mask), IrType::U32);
        let l = self.bin(IrBinOp::Shl, l, c(amount), IrType::U32);
        self.bin(IrBinOp::Or, r, l, IrType::U32)
    }
    fn bytes(&mut self, mut x: Operand, reverse: bool) -> Operand {
        if reverse {
            for (n, m) in [(1, 0x55555555), (2, 0x33333333), (4, 0x0f0f0f0f)] {
                x = self.swap(x, n, m);
            }
        }
        x = self.swap(x, 8, 0x00ff00ff);
        let r = self.bin(IrBinOp::LShr, x, c(16), IrType::U32);
        let l = self.bin(IrBinOp::Shl, x, c(16), IrType::U32);
        self.bin(IrBinOp::Or, r, l, IrType::U32)
    }
    fn function(self, result: Operand) -> IrFunction {
        let mut f = IrFunction::new("review".into(), IrType::U32, vec![], false);
        let source_spans = (0..self.insts.len())
            .map(|i| Span::new(i as u32, i as u32 + 1, 1))
            .collect();
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: self.insts,
            source_spans,
            terminator: Terminator::Return(Some(result)),
        });
        f.next_value_id = self.next;
        f
    }
}
fn c(x: u64) -> Operand {
    Operand::Const(IrConst::I64(x as i64))
}
fn fixture(family: usize) -> (Graph, Operand) {
    let mut g = Graph::new();
    let x = g.param(
        0,
        if family == 1 {
            IrType::U64
        } else {
            IrType::U32
        },
    );
    let result = if family < 2 {
        g.count(x, family == 1)
    } else {
        g.bytes(x, family == 3)
    };
    (g, result)
}
fn definitions(insts: &[Instruction]) -> Vec<Option<Instruction>> {
    let max = insts
        .iter()
        .filter_map(Instruction::dest)
        .map(|v| v.0)
        .max()
        .unwrap_or(0);
    let mut d = vec![None; max as usize + 1];
    for i in insts {
        if let Some(v) = i.dest() {
            d[v.0 as usize] = Some(i.clone());
        }
    }
    d
}
fn matches(family: usize, result: Operand, defs: &[Option<Instruction>]) -> bool {
    match family {
        0 => match_clz32(result, defs),
        1 => match_ctz64(result, defs),
        2 => match_bswap32_network(result, defs),
        _ => match_bit_reverse32(result, defs),
    }
    .is_some()
}

#[test]
fn review_every_count_and_swap_node_checks_its_width() {
    for family in 0..4 {
        let (g, result) = fixture(family);
        assert!(
            matches(family, result, &definitions(&g.insts)),
            "positive {family}"
        );
        for i in 0..g.insts.len() {
            let mut mutated = g.insts.clone();
            let ty = match &mut mutated[i] {
                Instruction::BinOp { ty, .. }
                | Instruction::Cmp { ty, .. }
                | Instruction::Select { ty, .. } => ty,
                _ => continue,
            };
            // Opposite full-width ring, plus the narrow-ring mutations below.
            *ty = if ty.size() == 8 {
                IrType::U32
            } else {
                IrType::U64
            };
            assert!(
                !matches(family, result, &definitions(&mutated)),
                "family {family}, node {i}"
            );
            for narrow in [IrType::U8, IrType::U16] {
                match &mut mutated[i] {
                    Instruction::BinOp { ty, .. }
                    | Instruction::Cmp { ty, .. }
                    | Instruction::Select { ty, .. } => *ty = narrow,
                    _ => unreachable!(),
                }
                assert!(
                    !matches(family, result, &definitions(&mutated)),
                    "family {family}, node {i}, {narrow:?}"
                );
            }
        }
    }
}

#[test]
fn review_value_edges_do_not_cross_truncations() {
    for family in 0..4 {
        let (g, result) = fixture(family);
        for inst in &g.insts {
            let ty = match inst {
                Instruction::BinOp { ty, .. } | Instruction::Select { ty, .. } => *ty,
                _ => continue,
            };
            let old = inst.dest().unwrap();
            for copy_only in [false, true] {
                let mut defs = definitions(&g.insts);
                let a = Value(defs.len() as u32);
                let b = Value(a.0 + 1);
                let replacement = if copy_only { a } else { b };
                for node in defs.iter_mut().flatten() {
                    node.for_each_operand_mut(|op| {
                        if *op == Operand::Value(old) {
                            *op = Operand::Value(replacement);
                        }
                    });
                }
                if copy_only {
                    defs.push(Some(Instruction::Copy {
                        dest: a,
                        src: Operand::Value(old),
                    }));
                } else {
                    defs.push(Some(Instruction::Cast {
                        dest: a,
                        src: Operand::Value(old),
                        from_ty: ty,
                        to_ty: IrType::U8,
                    }));
                    defs.push(Some(Instruction::Cast {
                        dest: b,
                        src: Operand::Value(a),
                        from_ty: IrType::U8,
                        to_ty: ty,
                    }));
                }
                let root = if result == Operand::Value(old) {
                    Operand::Value(replacement)
                } else {
                    result
                };
                assert_eq!(
                    matches(family, root, &defs),
                    copy_only,
                    "family {family}, edge {old:?}"
                );
            }
        }
    }
}

// Independent evaluator of the deliberately small, unsigned fixture IR.
// Values are masked at the INSTRUCTION's width, including comparison operands.
fn evaluate(f: &IrFunction, args: &[u64]) -> u64 {
    let mut values = std::collections::HashMap::new();
    fn operand(op: Operand, values: &std::collections::HashMap<u32, u64>) -> u64 {
        match op {
            Operand::Value(v) => *values
                .get(&v.0)
                .expect("use before definition / lost producer"),
            Operand::Const(c) => c.to_i128().unwrap() as u64,
        }
    }
    for i in f.blocks.iter().flat_map(|b| &b.instructions) {
        let get = |op| operand(op, &values);
        let (result, ty) = match i {
            Instruction::ParamRef { param_idx, ty, .. } => (args[*param_idx], *ty),
            Instruction::Copy { src, .. } => (get(*src), IrType::U64),
            Instruction::Cast { src, to_ty, .. } => (get(*src), *to_ty),
            Instruction::BinOp {
                op, lhs, rhs, ty, ..
            } => {
                let bits = ty.size() * 8;
                let a = get(*lhs) & mask(bits);
                let b = get(*rhs);
                let r = match op {
                    IrBinOp::Add => a.wrapping_add(b),
                    IrBinOp::Sub => a.wrapping_sub(b),
                    IrBinOp::And => a & b,
                    IrBinOp::Or => a | b,
                    IrBinOp::Xor => a ^ b,
                    IrBinOp::Shl => a.wrapping_shl(b as u32),
                    IrBinOp::LShr => a.wrapping_shr(b as u32),
                    IrBinOp::RotateLeft => {
                        let n = b as usize % bits;
                        if n == 0 {
                            a
                        } else {
                            (a << n) | (a >> (bits - n))
                        }
                    }
                    IrBinOp::RotateRight => {
                        let n = b as usize % bits;
                        if n == 0 {
                            a
                        } else {
                            (a >> n) | (a << (bits - n))
                        }
                    }
                    _ => panic!("unsupported {op:?}"),
                };
                (r, *ty)
            }
            Instruction::Cmp {
                op, lhs, rhs, ty, ..
            } => {
                let m = mask(ty.size() * 8);
                let (a, b) = (get(*lhs) & m, get(*rhs) & m);
                (
                    (match op {
                        IrCmpOp::Eq => a == b,
                        IrCmpOp::Ne => a != b,
                        IrCmpOp::Ule => a <= b,
                        _ => panic!("{op:?}"),
                    }) as u64,
                    IrType::U32,
                )
            }
            Instruction::Select {
                cond,
                true_val,
                false_val,
                ty,
                ..
            } => (
                get(if get(*cond) != 0 {
                    *true_val
                } else {
                    *false_val
                }),
                *ty,
            ),
            Instruction::UnaryOp { op, src, ty, .. } => {
                let bits = ty.size() * 8;
                let x = get(*src) & mask(bits);
                if matches!(op, IrUnaryOp::ClzNonZero | IrUnaryOp::CtzNonZero) {
                    assert_ne!(x, 0, "invalid nonzero intrinsic");
                }
                let r = match op {
                    IrUnaryOp::Clz | IrUnaryOp::ClzNonZero => {
                        x.leading_zeros() as u64 - (64 - bits) as u64
                    }
                    IrUnaryOp::Ctz | IrUnaryOp::CtzNonZero => {
                        (x.trailing_zeros() as u64).min(bits as u64)
                    }
                    _ => panic!("{op:?}"),
                };
                (r, *ty)
            }
            _ => panic!("unsupported fixture instruction"),
        };
        assert!(
            values
                .insert(i.dest().unwrap().0, result & mask(ty.size() * 8))
                .is_none(),
            "duplicate definition"
        );
    }
    if let Terminator::Return(Some(op)) = f.blocks.last().unwrap().terminator {
        operand(op, &values)
    } else {
        panic!("no return")
    }
}
fn mask(bits: usize) -> u64 {
    u64::MAX >> (64 - bits)
}

#[test]
fn review_narrow_compare_has_an_observable_semantic_witness() {
    let (mut g, result) = fixture(0);
    let cmp = g
        .insts
        .iter_mut()
        .find(|i| matches!(i, Instruction::Cmp { .. }))
        .unwrap();
    if let Instruction::Cmp { ty, .. } = cmp {
        *ty = IrType::U16;
    }
    let mut f = g.function(result);
    let before = evaluate(&f, &[0x100000]);
    recognize_function(&mut f, false, 64, 8, false, true);
    assert_eq!(evaluate(&f, &[0x100000]), before);
}

#[test]
fn review_mixed_expansions_keep_coordinates_values_and_spans() {
    // All five transforms in one block; permutation changes which expansion
    // precedes the Boolean producer and exposes stale def_loc coordinates.
    for order in [[0, 1, 2, 3, 4], [1, 3, 0, 4, 2], [2, 0, 4, 3, 1]] {
        let mut g = Graph::new();
        let x = g.param(0, IrType::U64);
        let y = g.param(1, IrType::U32);
        let z = g.param(2, IrType::U32);
        let mut results = Vec::new();
        for kind in order {
            let r = match kind {
                0 => g.count(x, true),
                1 => g.count(y, false),
                2 => {
                    let narrow = g.cast(y, IrType::U32, IrType::U16);
                    let wide = g.cast(narrow, IrType::U16, IrType::U32);
                    let l = g.bin(IrBinOp::Shl, wide, c(8), IrType::U32);
                    let r = g.bin(IrBinOp::LShr, wide, c(8), IrType::U32);
                    let both = g.bin(IrBinOp::Or, l, r, IrType::U32);
                    g.cast(both, IrType::U32, IrType::U16)
                }
                4 => {
                    let narrow = g.cast(c(7), IrType::U8, IrType::U32);
                    let both = g.bin(IrBinOp::Xor, y, narrow, IrType::U32);
                    g.bin(IrBinOp::And, both, c(255), IrType::U32)
                }
                _ => {
                    let a = g.bin(IrBinOp::And, y, z, IrType::U32);
                    let b = g.bin(IrBinOp::And, y, x, IrType::U32);
                    g.bin(IrBinOp::Xor, a, b, IrType::U32)
                }
            };
            results.push(r);
        }
        let mut sum = c(0);
        for r in results {
            sum = g.bin(IrBinOp::Add, sum, r, IrType::U32);
        }
        let mut f = g.function(sum);
        let original = f.clone();
        let spans: std::collections::HashMap<_, _> = f.blocks[0]
            .instructions
            .iter()
            .zip(&f.blocks[0].source_spans)
            .map(|(i, s)| (i.dest().unwrap().0, *s))
            .collect();
        assert!(recognize_function(&mut f, false, 64, 8, false, true) >= 5);
        let b = &f.blocks[0];
        assert_eq!(b.source_spans.len(), b.instructions.len());
        for (i, s) in b.instructions.iter().zip(&b.source_spans) {
            if let Some(old) = spans.get(&i.dest().unwrap().0) {
                assert_eq!(s, old, "original dest's span moved");
            }
        }
        for args in [
            [0, 0, 0],
            [1, 2, 3],
            [1 << 63, u32::MAX as u64, 0xabcdef],
            [0x100000000, 0x10000, 7],
        ] {
            assert_eq!(
                evaluate(&f, &args),
                evaluate(&original, &args),
                "order {order:?}"
            );
        }
    }
}

#[test]
fn review_safe_count_expansions_are_exact_and_idempotent() {
    for trailing in [false, true] {
        for signed in [false, true] {
            let (mut g, result) = fixture(usize::from(trailing));
            if signed {
                for i in &mut g.insts {
                    match i {
                        Instruction::Select { ty, .. }
                        | Instruction::BinOp { ty, .. }
                        | Instruction::Cmp { ty, .. }
                            if *ty == IrType::U32 =>
                        {
                            *ty = IrType::I32
                        }
                        _ => {}
                    }
                }
            }
            let mut f = g.function(result);
            assert_eq!(recognize_function(&mut f, false, 64, 8, false, true), 1);
            let ty = if signed { IrType::I32 } else { IrType::U32 };
            assert!(f.blocks[0].instructions.iter().any(|i| i.dest()
                == match result {
                    Operand::Value(v) => Some(v),
                    _ => None,
                }
                && matches!(i,Instruction::Cast {to_ty,..} if *to_ty==ty)
                || matches!(i,Instruction::UnaryOp {op:IrUnaryOp::ClzNonZero,ty:t,..} if *t==ty)));
            let mut state = 0xabcdef1234567890u64;
            for i in 0..4096 {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let x = if i == 0 {
                    0
                } else if i <= 64 {
                    1u64 << (i - 1)
                } else {
                    state
                };
                let expected = if trailing {
                    x.trailing_zeros().min(63)
                } else {
                    (x as u32).leading_zeros().min(31)
                };
                assert_eq!(evaluate(&f, &[x]), expected as u64);
            }
            assert_eq!(recognize_function(&mut f, false, 64, 8, false, true), 0);
        }
    }
}

#[test]
fn review_ctz_accepts_both_condition_polarities_without_peeling_mask_casts() {
    for explicit in [false, true] {
        let (mut g, result) = fixture(1);
        let mut conditions = std::collections::HashMap::new();
        for i in &mut g.insts {
            if let Instruction::Cmp { dest, op, lhs, .. } = i {
                conditions.insert(*dest, *lhs);
                if explicit {
                    *op = IrCmpOp::Ne;
                }
            }
        }
        for i in &mut g.insts {
            if let Instruction::Select {
                cond: Operand::Value(v),
                true_val,
                false_val,
                ..
            } = i
            {
                if conditions.contains_key(v) {
                    std::mem::swap(true_val, false_val);
                }
            }
        }
        if !explicit {
            for i in &mut g.insts {
                if let Instruction::Select { cond, .. } = i {
                    if let Operand::Value(v) = *cond {
                        if let Some(x) = conditions.get(&v) {
                            *cond = *x;
                        }
                    }
                }
            }
        }
        assert!(matches(1, result, &definitions(&g.insts)));
        let mut f = g.function(result);
        let original = f.clone();
        assert_eq!(recognize_function(&mut f, false, 64, 8, false, true), 1);
        for x in [0, 1, 0x100, 0x10000, 1 << 32, 1 << 63, u64::MAX] {
            assert_eq!(evaluate(&f, &[x]), evaluate(&original, &[x]));
        }
    }
}

#[test]
fn review_cross_block_mask_pair_survives_expanding_consumer_block() {
    let mut g = Graph::new();
    let x = g.param(0, IrType::U64);
    let y = g.param(1, IrType::U32);
    let narrow = g.cast(c(7), IrType::U8, IrType::U32);
    let producer = g.bin(IrBinOp::Xor, y, narrow, IrType::U32);
    let split = g.insts.len();
    let count = g.count(x, true);
    let masked = g.bin(IrBinOp::And, producer, c(255), IrType::U32);
    let result = g.bin(IrBinOp::Add, count, masked, IrType::U32);
    let mut f = g.function(result);
    let instructions = f.blocks[0].instructions.split_off(split);
    let source_spans = f.blocks[0].source_spans.split_off(split);
    let terminator = std::mem::replace(&mut f.blocks[0].terminator, Terminator::Branch(BlockId(1)));
    f.blocks.push(BasicBlock {
        label: BlockId(1),
        instructions,
        source_spans,
        terminator,
    });
    let original = f.clone();
    assert_eq!(recognize_function(&mut f, false, 64, 8, false, true), 2);
    for args in [[0, 0], [1 << 63, 0x123456], [0x100, 255]] {
        assert_eq!(evaluate(&f, &args), evaluate(&original, &args));
    }
    for b in &f.blocks {
        assert_eq!(b.source_spans.len(), b.instructions.len());
    }
}

#[test]
fn review_literal_carriers_are_not_arithmetic_masks() {
    // All operation types have the expected width. Only the literal carrier
    // changes: its signed value must be interpreted in that operation's ring.
    for family in [0, 1] {
        let (mut g, result) = fixture(family);
        if family == 0 {
            let cmp = g
                .insts
                .iter_mut()
                .find(|i| matches!(i, Instruction::Cmp { .. }))
                .unwrap();
            if let Instruction::Cmp { rhs, .. } = cmp {
                *rhs = Operand::Const(IrConst::I16(-1));
            }
        } else {
            let and = g
                .insts
                .iter_mut()
                .find(|i| {
                    matches!(
                        i,
                        Instruction::BinOp {
                            op: IrBinOp::And,
                            ..
                        }
                    )
                })
                .unwrap();
            if let Instruction::BinOp { rhs, .. } = and {
                *rhs = Operand::Const(IrConst::I32(-1));
            }
        }
        assert!(!matches(family, result, &definitions(&g.insts)));
        let mut f = g.function(result);
        let x = if family == 0 { 0x100000 } else { 1u64 << 32 };
        let expected = evaluate(&f, &[x]);
        assert_eq!(expected, 31);
        recognize_function(&mut f, false, 64, 8, false, true);
        assert_eq!(evaluate(&f, &[x]), expected);
    }
    assert_eq!(
        const_in_ring(Operand::Const(IrConst::I16(-1)), 64),
        Some(u64::MAX)
    );
    assert_eq!(
        const_in_ring(Operand::Const(IrConst::I64(-1)), 32),
        Some(u32::MAX as u64)
    );
}

#[test]
fn review_increments_use_the_requested_ring_and_do_not_peel_casts() {
    let mut g = Graph::new();
    let base = g.param(0, IrType::U64);
    let added = g.bin(IrBinOp::Add, base, c(1), IrType::U64);
    let truncated = g.cast(c(256), IrType::U32, IrType::U8);
    let widened = g.cast(truncated, IrType::U8, IrType::U32);
    let defs = definitions(&g.insts);
    assert!(is_incremented(added, base, 1, 64, &defs));
    assert!(!is_incremented(added, base, 1, 32, &defs));
    assert!(is_incremented(c(0), c(u32::MAX as u64), 1, 32, &defs));
    assert!(!is_incremented(c(0), c(u32::MAX as u64), 1, 64, &defs));
    assert!(is_incremented(
        c(1u64 << 32),
        c(u32::MAX as u64),
        1,
        32,
        &defs
    ));
    assert!(!is_incremented(widened, c(255), 1, 32, &defs));
}

#[test]
fn review_count_literals_accept_ring_equivalent_carriers() {
    for family in [0, 1] {
        let (mut g, result) = fixture(family);
        // Count constants have U32 consumers. Add a full modulus to their
        // storage carrier, not to shifts or the U64 mask's comparison.
        for i in &mut g.insts {
            if let Instruction::BinOp {
                op: IrBinOp::Add,
                lhs,
                rhs,
                ..
            } = i
            {
                for operand in [lhs, rhs] {
                    if let Operand::Const(value) = operand {
                        *value = IrConst::I64((value.to_i128().unwrap() + (1i128 << 32)) as i64);
                    }
                }
            }
            if let Instruction::Select {
                false_val: Operand::Const(value),
                ty: IrType::U32,
                ..
            } = i
            {
                if value.to_i128() == Some(0) {
                    *value = IrConst::I64(1i64 << 32);
                }
            }
        }
        assert!(matches(family, result, &definitions(&g.insts)));
        let mut f = g.function(result);
        let original = f.clone();
        assert_eq!(recognize_function(&mut f, false, 64, 8, false, true), 1);
        for x in [0, 1, 0x100, 1u64 << 31, 1u64 << 32, 1u64 << 63, u64::MAX] {
            assert_eq!(evaluate(&f, &[x]), evaluate(&original, &[x]));
        }
    }
}

#[test]
fn review_narrow_rotate_range_proof_uses_promoted_ring() {
    // I8(-128) in an I32 operation is 0xffffff80, NOT 0x80.
    // The old carrier-view range check admitted a 16-bit rotate and changed
    // this expression from 0xffff to 0x80ff. This is an IR witness, not a
    // claim that the C frontend currently emits this literal spelling.
    let mut g = Graph::new();
    let x = Operand::Const(IrConst::I8(-128));
    let left = g.bin(IrBinOp::Shl, x, c(8), IrType::U32);
    let right = g.bin(IrBinOp::LShr, x, c(8), IrType::U32);
    let combined = g.bin(IrBinOp::Or, left, right, IrType::U32);
    let result = g.cast(combined, IrType::U32, IrType::U16);
    assert!(!high_bits_zero(x, 16, &definitions(&g.insts)));
    assert!(match_narrow_rotate(combined, 16, &definitions(&g.insts)).is_none());
    let mut f = g.function(result);
    assert_eq!(evaluate(&f, &[]), 0xffff);
    recognize_function(&mut f, false, 64, 8, false, true);
    assert_eq!(evaluate(&f, &[]), 0xffff);
}

#[test]
fn review_mask_range_proofs_preserve_constant_casts_and_signs() {
    let mut g = Graph::new();
    let x = g.param(0, IrType::U32);
    let fake_mask = Operand::Const(IrConst::I8(-1));
    let and = g.bin(IrBinOp::And, x, fake_mask, IrType::U32);
    let narrow = g.cast(c(255), IrType::U32, IrType::I8);
    let extended = g.cast(narrow, IrType::I8, IrType::U32);
    let cast_mask = g.bin(IrBinOp::And, x, extended, IrType::U32);
    let defs = definitions(&g.insts);
    assert!(!high_bits_zero(and, 8, &defs));
    assert!(!high_bits_zero(cast_mask, 8, &defs));
    assert!(!fits_low_bits(fake_mask, 8, 32, &defs, &[]));
    assert!(fits_low_bits(fake_mask, 8, 8, &defs, &[]));
    assert_eq!(const_in_ring(c(0), 128), None);
}

#[test]
fn review_same_condition_selects_preserve_polarity_widths_and_spans() {
    for ty in [IrType::I8, IrType::U16, IrType::I32, IrType::U64] {
        for unrelated in [false, true] {
            let mut g = Graph::new();
            let cond = g.param(0, IrType::U64);
            let other = g.param(1, IrType::U64);
            let inner_cond = if unrelated { other } else { cond };
            let yes = g.sel(inner_cond, c(7), c(8), ty);
            let no = g.sel(inner_cond, c(9), c(10), ty);
            let result = g.sel(cond, yes, no, ty);
            let mut f = g.function(result);
            let original = f.clone();
            assert_eq!(
                recognize_function(&mut f, false, 64, 8, false, true),
                usize::from(!unrelated)
            );
            for a in [0, 1, 1u64 << 32, u64::MAX] {
                for b in [0, 1] {
                    assert_eq!(evaluate(&f, &[a, b]), evaluate(&original, &[a, b]));
                }
            }
            assert_eq!(f.blocks[0].source_spans, original.blocks[0].source_spans);
            assert_eq!(recognize_function(&mut f, false, 64, 8, false, true), 0);
        }
    }
    // A low-byte predicate cannot inherit a full-width nonzero proof.
    let mut g = Graph::new();
    let cond = g.param(0, IrType::U64);
    let truncated = g.cast(cond, IrType::U64, IrType::U8);
    let yes = g.sel(truncated, c(7), c(8), IrType::U32);
    let result = g.sel(cond, yes, c(10), IrType::U32);
    let mut f = g.function(result);
    assert_eq!(recognize_function(&mut f, false, 64, 8, false, true), 0);
    assert_eq!(evaluate(&f, &[256]), 8);
}

#[test]
fn review_crc_cast_distribution_matches_bitvectors_for_xor_or_and_both_roles() {
    for (narrow, mask) in [(IrType::U8, 255u64), (IrType::U16, 65535u64)] {
        for op in [IrBinOp::Xor, IrBinOp::Or] {
            for swap in [false, true] {
                let mut g = Graph::new();
                let wide = g.param(0, IrType::U32);
                let small = g.param(1, narrow);
                let extended = g.cast(small, narrow, IrType::U32);
                let (lhs, rhs) = if swap {
                    (extended, wide)
                } else {
                    (wide, extended)
                };
                let combined = g.bin(op, lhs, rhs, IrType::U32);
                let truncated = g.cast(combined, IrType::U32, narrow);
                let result = g.cast(truncated, narrow, IrType::U32);
                let mut f = g.function(result);
                assert!(recognize_function(&mut f, false, 64, 8, false, true) > 0);
                for wide in [0u64, 1, 127, 255, 256, 65535, 0x8000_0000, 0xffff_ffff] {
                    for small in (0..=255).chain([256, 32768, 65535]).filter(|x| *x <= mask) {
                        let expected = match op {
                            IrBinOp::Xor => wide ^ small,
                            IrBinOp::Or => wide | small,
                            _ => unreachable!(),
                        } & mask;
                        assert_eq!(
                            evaluate(&f, &[wide, small]),
                            expected,
                            "{narrow:?} {op:?} swap={swap} {wide:x} {small:x}"
                        );
                    }
                }
            }
        }
    }
}
