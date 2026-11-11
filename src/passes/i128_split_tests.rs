//! Translation validation for the 128-bit split: every test builds a small
//! function, runs the pass, and executes the function before and after on a
//! reference interpreter.  The interpreter's own 128-bit arithmetic is pinned
//! against Rust's `u128`/`i128`, so "before == after" is not a tautology.

use super::*;
use crate::ir::reexports::{BasicBlock, BlockId, CallInfo, Terminator};

const OUT: u32 = 1_000_000;
const MEM: u32 = 1_000_001;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Val {
    Int(u128),
    Addr(u32, u64),
}

fn bits(ty: IrType) -> u32 {
    match ty {
        IrType::I8 | IrType::U8 => 8,
        IrType::I16 | IrType::U16 => 16,
        IrType::I32 | IrType::U32 => 32,
        IrType::I64 | IrType::U64 | IrType::Ptr => 64,
        IrType::I128 | IrType::U128 => 128,
        other => panic!("interpreter has no width for {other:?}"),
    }
}

fn mask(value: u128, ty: IrType) -> u128 {
    match bits(ty) {
        128 => value,
        width => value & ((1u128 << width) - 1),
    }
}

fn signed(value: u128, ty: IrType) -> i128 {
    let shift = 128 - bits(ty);
    ((mask(value, ty) << shift) as i128) >> shift
}

/// Executes the subset of the IR the pass reads and writes.
struct Machine {
    values: FxHashMap<u32, Val>,
    memory: FxHashMap<u32, Vec<u8>>,
    params: Vec<Val>,
}

impl Machine {
    fn new(params: &[Val]) -> Self {
        let mut memory = FxHashMap::default();
        memory.insert(OUT, vec![0u8; 256]);
        Self {
            values: FxHashMap::default(),
            memory,
            params: params.to_vec(),
        }
    }

    fn operand(&self, op: &Operand) -> Val {
        match op {
            Operand::Value(v) => *self
                .values
                .get(&v.0)
                .unwrap_or_else(|| panic!("value {} read before its definition", v.0)),
            Operand::Const(IrConst::Zero) => Val::Int(0),
            Operand::Const(IrConst::I128(v)) => Val::Int(*v as u128),
            Operand::Const(c) => Val::Int(i128::from(c.to_i64().expect("integer literal")) as u128),
        }
    }

    fn int(&self, op: &Operand) -> u128 {
        match self.operand(op) {
            Val::Int(v) => v,
            Val::Addr(..) => panic!("address used as an integer"),
        }
    }

    fn address(&self, ptr: Value) -> (u32, usize) {
        match self.values[&ptr.0] {
            Val::Addr(object, offset) => (object, offset as usize),
            Val::Int(_) => panic!("integer used as an address"),
        }
    }

    fn binop(op: IrBinOp, a: u128, b: u128, ty: IrType) -> u128 {
        let (a, b) = (mask(a, ty), mask(b, ty));
        let width = bits(ty);
        let count = || {
            assert!(b < u128::from(width), "shift count {b} at width {width}");
            b as u32
        };
        let value = match op {
            IrBinOp::Add => a.wrapping_add(b),
            IrBinOp::Sub => a.wrapping_sub(b),
            IrBinOp::Mul => a.wrapping_mul(b),
            IrBinOp::And => a & b,
            IrBinOp::Or => a | b,
            IrBinOp::Xor => a ^ b,
            IrBinOp::Shl => a << count(),
            IrBinOp::LShr => a >> count(),
            IrBinOp::AShr => (signed(a, ty) >> count()) as u128,
            IrBinOp::UMulHigh => {
                assert_eq!(width, 64);
                (a * b) >> 64
            }
            IrBinOp::SMulHigh => {
                assert_eq!(width, 64);
                ((signed(a, ty) * signed(b, ty)) >> 64) as u128
            }
            other => panic!("interpreter has no {other:?}"),
        };
        mask(value, ty)
    }

    fn compare(op: IrCmpOp, a: u128, b: u128, ty: IrType) -> bool {
        let (ua, ub) = (mask(a, ty), mask(b, ty));
        let (sa, sb) = (signed(a, ty), signed(b, ty));
        match op {
            IrCmpOp::Eq => ua == ub,
            IrCmpOp::Ne => ua != ub,
            IrCmpOp::Ult => ua < ub,
            IrCmpOp::Ule => ua <= ub,
            IrCmpOp::Ugt => ua > ub,
            IrCmpOp::Uge => ua >= ub,
            IrCmpOp::Slt => sa < sb,
            IrCmpOp::Sle => sa <= sb,
            IrCmpOp::Sgt => sa > sb,
            IrCmpOp::Sge => sa >= sb,
        }
    }

    fn step(&mut self, inst: &Instruction) {
        match inst {
            Instruction::Alloca { dest, size, .. } => {
                self.memory.insert(dest.0, vec![0u8; *size]);
                self.values.insert(dest.0, Val::Addr(dest.0, 0));
            }
            Instruction::ParamRef {
                dest, param_idx, ..
            } => {
                self.values.insert(dest.0, self.params[*param_idx]);
            }
            Instruction::GetElementPtr {
                dest, base, offset, ..
            } => {
                let (object, at) = self.address(*base);
                let offset = self.int(offset) as u64;
                self.values
                    .insert(dest.0, Val::Addr(object, at as u64 + offset));
            }
            Instruction::Load { dest, ptr, ty, .. } => {
                let (object, at) = self.address(*ptr);
                let width = (bits(*ty) / 8) as usize;
                let mut bytes = [0u8; 16];
                bytes[..width].copy_from_slice(&self.memory[&object][at..at + width]);
                self.values
                    .insert(dest.0, Val::Int(u128::from_le_bytes(bytes)));
            }
            Instruction::Store { val, ptr, ty, .. } => {
                let (object, at) = self.address(*ptr);
                let width = (bits(*ty) / 8) as usize;
                let bytes = mask(self.int(val), *ty).to_le_bytes();
                self.memory.get_mut(&object).expect("object")[at..at + width]
                    .copy_from_slice(&bytes[..width]);
            }
            Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } => {
                let source = self.int(src);
                let extended = if from_ty.is_signed() {
                    signed(source, *from_ty) as u128
                } else {
                    mask(source, *from_ty)
                };
                self.values.insert(dest.0, Val::Int(mask(extended, *to_ty)));
            }
            Instruction::Copy { dest, src } => {
                let value = self.operand(src);
                self.values.insert(dest.0, value);
            }
            Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => {
                let value = Self::binop(*op, self.int(lhs), self.int(rhs), *ty);
                self.values.insert(dest.0, Val::Int(value));
            }
            Instruction::UnaryOp { dest, op, src, ty } => {
                let source = mask(self.int(src), *ty);
                let value = match op {
                    IrUnaryOp::Neg => 0u128.wrapping_sub(source),
                    IrUnaryOp::Not => !source,
                    other => panic!("interpreter has no {other:?}"),
                };
                self.values.insert(dest.0, Val::Int(mask(value, *ty)));
            }
            Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => {
                let holds = Self::compare(*op, self.int(lhs), self.int(rhs), *ty);
                self.values.insert(dest.0, Val::Int(u128::from(holds)));
            }
            Instruction::Select {
                dest,
                cond,
                true_val,
                false_val,
                ..
            } => {
                let chosen = if self.int(cond) != 0 {
                    true_val
                } else {
                    false_val
                };
                let value = self.operand(chosen);
                self.values.insert(dest.0, value);
            }
            other => panic!("interpreter has no {other:?}"),
        }
    }

    fn run(&mut self, func: &IrFunction) -> Option<Val> {
        let mut block = 0usize;
        let mut previous: Option<BlockId> = None;
        for _ in 0..10_000 {
            let current = &func.blocks[block];
            // Phis read their operands simultaneously on block entry.
            let mut merged: Vec<(u32, Val)> = Vec::new();
            for inst in &current.instructions {
                let Instruction::Phi { dest, incoming, .. } = inst else {
                    break;
                };
                let from = previous.expect("phi in the entry block");
                let (op, _) = incoming
                    .iter()
                    .find(|(_, edge)| *edge == from)
                    .expect("phi has no entry for the executed edge");
                merged.push((dest.0, self.operand(op)));
            }
            let phis = merged.len();
            for (dest, value) in merged {
                self.values.insert(dest, value);
            }
            for inst in &current.instructions[phis..] {
                self.step(inst);
            }
            let target = match &current.terminator {
                Terminator::Return(value) => return value.as_ref().map(|op| self.operand(op)),
                Terminator::Branch(label) => *label,
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } => {
                    if self.int(cond) != 0 {
                        *true_label
                    } else {
                        *false_label
                    }
                }
                other => panic!("interpreter has no {other:?}"),
            };
            previous = Some(current.label);
            block = func
                .blocks
                .iter()
                .position(|b| b.label == target)
                .expect("branch target");
        }
        panic!("interpreter step limit");
    }
}

/// Straight-line function builder.  Parameter 0 is the output buffer.
struct Build {
    next: u32,
    insts: Vec<Instruction>,
    outputs: u64,
}

impl Build {
    fn new() -> Self {
        let mut build = Self {
            next: 0,
            insts: Vec::new(),
            outputs: 0,
        };
        let out = build.fresh();
        build.insts.push(Instruction::ParamRef {
            dest: out,
            param_idx: 0,
            ty: IrType::Ptr,
        });
        build
    }

    fn fresh(&mut self) -> Value {
        self.next += 1;
        Value(self.next - 1)
    }

    fn param(&mut self, index: usize, ty: IrType) -> Operand {
        let dest = self.fresh();
        self.insts.push(Instruction::ParamRef {
            dest,
            param_idx: index,
            ty,
        });
        Operand::Value(dest)
    }

    fn pointer(&mut self, index: usize) -> Value {
        match self.param(index, IrType::Ptr) {
            Operand::Value(value) => value,
            Operand::Const(_) => unreachable!("a parameter is a value"),
        }
    }

    fn cast(&mut self, src: Operand, from_ty: IrType, to_ty: IrType) -> Operand {
        let dest = self.fresh();
        self.insts.push(Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        });
        Operand::Value(dest)
    }

    fn bin(&mut self, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Operand {
        let dest = self.fresh();
        self.insts.push(Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        });
        Operand::Value(dest)
    }

    fn cmp(&mut self, op: IrCmpOp, lhs: Operand, rhs: Operand, ty: IrType) -> Operand {
        let dest = self.fresh();
        self.insts.push(Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        });
        Operand::Value(dest)
    }

    fn load(&mut self, ptr: Value, ty: IrType) -> Operand {
        let dest = self.fresh();
        self.insts.push(Instruction::Load {
            dest,
            ptr,
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        Operand::Value(dest)
    }

    fn store(&mut self, val: Operand, ptr: Value, ty: IrType) {
        self.insts.push(Instruction::Store {
            val,
            ptr,
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
    }

    fn local(&mut self) -> Value {
        let dest = self.fresh();
        self.insts.push(Instruction::Alloca {
            dest,
            ty: IrType::U128,
            size: 16,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        });
        dest
    }

    /// A general 128-bit value assembled inside the island from two words.
    fn wide(&mut self, lo: Operand, hi: Operand) -> Operand {
        let lo = self.cast(lo, IrType::U64, IrType::U128);
        let hi = self.cast(hi, IrType::U64, IrType::U128);
        let hi = self.bin(
            IrBinOp::Shl,
            hi,
            Operand::Const(IrConst::I32(64)),
            IrType::U128,
        );
        self.bin(IrBinOp::Or, lo, hi, IrType::U128)
    }

    fn emit_word(&mut self, word: Operand) {
        let address = self.fresh();
        self.insts.push(Instruction::GetElementPtr {
            dest: address,
            base: Value(0),
            offset: c64(self.outputs as i64 * 8),
            ty: IrType::Ptr,
        });
        self.outputs += 1;
        self.store(word, address, IrType::U64);
    }

    /// Observe both words of a 128-bit value through truncating casts.
    fn emit_wide(&mut self, value: Operand) {
        let lo = self.cast(value, IrType::U128, IrType::U64);
        let shifted = self.bin(
            IrBinOp::LShr,
            value,
            Operand::Const(IrConst::I32(64)),
            IrType::U128,
        );
        let hi = self.cast(shifted, IrType::U128, IrType::U64);
        self.emit_word(lo);
        self.emit_word(hi);
    }

    fn finish(self, result: Option<Operand>) -> IrFunction {
        function(
            vec![block(0, self.insts, Terminator::Return(result))],
            self.next,
        )
    }
}

fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(label),
        instructions,
        terminator,
        source_spans: Vec::new(),
        source_inline_ids: Vec::new(),
    }
}

fn function(blocks: Vec<BasicBlock>, next_value_id: u32) -> IrFunction {
    let mut func = IrFunction::new("split_test".into(), IrType::U64, vec![], false);
    func.blocks = blocks;
    func.next_value_id = next_value_id;
    func
}

fn instructions(func: &IrFunction) -> impl Iterator<Item = &Instruction> {
    func.blocks.iter().flat_map(|b| &b.instructions)
}

fn is_wide_instruction(inst: &Instruction) -> bool {
    inst.result_type().is_some_and(is_wide)
        || matches!(inst, Instruction::Store { ty, .. } | Instruction::Cmp { ty, .. } if is_wide(*ty))
        || matches!(inst, Instruction::Cast { from_ty, .. } if is_wide(*from_ty))
        || matches!(inst, Instruction::Alloca { ty, .. } if is_wide(*ty))
}

fn wide_instructions(func: &IrFunction) -> usize {
    instructions(func)
        .filter(|inst| is_wide_instruction(inst))
        .count()
}

fn count_ops(func: &IrFunction, wanted: IrBinOp) -> usize {
    instructions(func)
        .filter(|inst| matches!(inst, Instruction::BinOp { op, .. } if *op == wanted))
        .count()
}

const EDGES: [u64; 14] = [
    0,
    1,
    2,
    0x7fff_ffff,
    0x8000_0000,
    0xffff_ffff,
    0x1_0000_0000,
    0x7fff_ffff_ffff_ffff,
    0x8000_0000_0000_0000,
    0xffff_ffff_ffff_ffff,
    0xffff_ffff_ffff_fffe,
    0x9e37_79b9_7f4a_7c15,
    0xc2b2_ae3d_27d4_eb4f,
    0x0123_4567_89ab_cdef,
];

/// Xorshift stream: deterministic, so a failure reproduces.
fn random_words(seed: u64) -> impl Iterator<Item = u64> {
    let mut state = seed | 1;
    std::iter::repeat_with(move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    })
}

/// Parameter tuples: every pair of edges, then pseudo-random fill.
fn inputs(arity: usize, count: usize) -> Vec<Vec<u64>> {
    let mut all = Vec::new();
    for &a in &EDGES {
        for &b in &EDGES {
            let mut row = vec![a, b];
            row.resize(arity.max(2), a ^ b.rotate_left(17));
            row.truncate(arity);
            all.push(row);
        }
    }
    let mut stream = random_words(0x243f_6a88_85a3_08d3);
    while all.len() < count {
        all.push(
            (0..arity)
                .map(|_| stream.next().expect("endless"))
                .collect(),
        );
    }
    all
}

struct Observed {
    out: Vec<u8>,
    mem: Vec<u8>,
    result: Option<Val>,
}

fn execute(func: &IrFunction, args: &[u64], mem: &[u8]) -> Observed {
    let mut params = vec![Val::Addr(OUT, 0)];
    params.extend(args.iter().map(|&a| Val::Int(u128::from(a))));
    params.push(Val::Addr(MEM, 0));
    let mut machine = Machine::new(&params);
    machine.memory.insert(MEM, mem.to_vec());
    let result = machine.run(func);
    Observed {
        out: machine.memory[&OUT].clone(),
        mem: machine.memory[&MEM].clone(),
        result,
    }
}

fn out_wide(observed: &Observed, index: usize) -> u128 {
    let at = index * 16;
    u128::from_le_bytes(observed.out[at..at + 16].try_into().expect("16 bytes"))
}

fn out_word(observed: &Observed, index: usize) -> u64 {
    let at = index * 8;
    u64::from_le_bytes(observed.out[at..at + 8].try_into().expect("8 bytes"))
}

/// Run the pass and require that `func` and the split function agree on
/// every input.  Returns the split function for shape assertions.
fn validate(func: &IrFunction, arity: usize, mem: &[u8], samples: usize) -> IrFunction {
    let mut split = func.clone();
    let changed = run_function(&mut split);
    assert!(changed > 0, "the island was not split: {func:#?}");
    assert_eq!(
        wide_instructions(&split),
        0,
        "128-bit instructions survive: {split:#?}"
    );
    for args in inputs(arity, samples) {
        let before = execute(func, &args, mem);
        let after = execute(&split, &args, mem);
        assert_eq!(before.out, after.out, "output buffer, inputs {args:x?}");
        assert_eq!(before.mem, after.mem, "memory, inputs {args:x?}");
        assert_eq!(before.result, after.result, "result, inputs {args:x?}");
    }
    split
}

fn assert_untouched(mut func: IrFunction, why: &str) {
    let before = format!("{func:?}");
    assert_eq!(run_function(&mut func), 0, "{why}");
    assert_eq!(before, format!("{func:?}"), "{why}");
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    Zext64,
    Sext64,
    Sext32,
    Zext8,
    Full,
    Literal(i128),
}

impl Shape {
    /// Build the operand from parameters `first` and `first + 1`.
    fn build(self, b: &mut Build, first: usize) -> Operand {
        match self {
            Shape::Zext64 => {
                let p = b.param(first, IrType::U64);
                b.cast(p, IrType::U64, IrType::U128)
            }
            Shape::Sext64 => {
                let p = b.param(first, IrType::I64);
                b.cast(p, IrType::I64, IrType::I128)
            }
            Shape::Sext32 => {
                let p = b.param(first, IrType::I64);
                let narrow = b.cast(p, IrType::I64, IrType::I32);
                b.cast(narrow, IrType::I32, IrType::U128)
            }
            Shape::Zext8 => {
                let p = b.param(first, IrType::I64);
                let narrow = b.cast(p, IrType::I64, IrType::U8);
                b.cast(narrow, IrType::U8, IrType::I128)
            }
            Shape::Full => {
                let lo = b.param(first, IrType::U64);
                let hi = b.param(first + 1, IrType::U64);
                b.wide(lo, hi)
            }
            Shape::Literal(value) => Operand::Const(IrConst::I128(value)),
        }
    }

    /// The value [`Self::build`] denotes for the arguments `args`
    /// (`args[0]` is parameter 1).
    fn reference(self, args: &[u64], first: usize) -> u128 {
        let p = args[first - 1];
        match self {
            Shape::Zext64 => u128::from(p),
            Shape::Sext64 => i128::from(p as i64) as u128,
            Shape::Sext32 => i128::from(p as i32) as u128,
            Shape::Zext8 => u128::from(p as u8),
            Shape::Full => (u128::from(args[first]) << 64) | u128::from(p),
            Shape::Literal(value) => value as u128,
        }
    }
}

const SHAPES: [Shape; 8] = [
    Shape::Zext64,
    Shape::Sext64,
    Shape::Sext32,
    Shape::Zext8,
    Shape::Full,
    Shape::Literal(0x9e37_79b9_7f4a_7c15),
    Shape::Literal(-3),
    Shape::Literal((0x0123_4567 << 64) | 0x89ab_cdef_0011_2233),
];

#[test]
fn the_interpreter_agrees_with_rust_on_128_bit_arithmetic() {
    let max = u128::MAX;
    let ty = IrType::U128;
    assert_eq!(Machine::binop(IrBinOp::Add, max, 2, ty), 1);
    assert_eq!(Machine::binop(IrBinOp::Sub, 0, 1, ty), max);
    assert_eq!(
        Machine::binop(IrBinOp::Mul, max, max, ty),
        max.wrapping_mul(max)
    );
    assert_eq!(
        Machine::binop(IrBinOp::AShr, 1 << 127, 127, IrType::I128),
        max
    );
    assert_eq!(Machine::binop(IrBinOp::LShr, 1 << 127, 127, ty), 1);
    assert_eq!(
        Machine::binop(IrBinOp::UMulHigh, u128::from(u64::MAX), 2, IrType::U64),
        1
    );
    assert_eq!(
        Machine::binop(IrBinOp::SMulHigh, u128::from(u64::MAX), 2, IrType::I64),
        u128::from(u64::MAX)
    );
    assert!(Machine::compare(IrCmpOp::Slt, max, 0, IrType::I128));
    assert!(!Machine::compare(IrCmpOp::Ult, max, 0, ty));
}

#[test]
fn every_binary_operation_matches_u128_arithmetic_for_every_operand_shape() {
    type Reference = fn(u128, u128) -> u128;
    let ops: [(IrBinOp, Reference); 6] = [
        (IrBinOp::Add, u128::wrapping_add),
        (IrBinOp::Sub, u128::wrapping_sub),
        (IrBinOp::Mul, u128::wrapping_mul),
        (IrBinOp::And, |a, b| a & b),
        (IrBinOp::Or, |a, b| a | b),
        (IrBinOp::Xor, |a, b| a ^ b),
    ];
    for (op, reference) in ops {
        for left in SHAPES {
            for right in SHAPES {
                if matches!((left, right), (Shape::Literal(_), Shape::Literal(_))) {
                    continue;
                }
                let mut b = Build::new();
                let lhs = left.build(&mut b, 1);
                let rhs = right.build(&mut b, 3);
                let result = b.bin(op, lhs, rhs, IrType::U128);
                b.emit_wide(result);
                let func = b.finish(None);
                let split = validate(&func, 4, &[], 260);
                for args in inputs(4, 260) {
                    let expected = reference(left.reference(&args, 1), right.reference(&args, 3));
                    let observed = execute(&split, &args, &[]);
                    assert_eq!(
                        out_wide(&observed, 0),
                        expected,
                        "{op:?} {left:?} {right:?} {args:x?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_constant_shift_matches_for_every_count() {
    for op in [IrBinOp::Shl, IrBinOp::LShr, IrBinOp::AShr] {
        for count in 0..128u32 {
            let mut b = Build::new();
            let value = Shape::Full.build(&mut b, 1);
            let ty = if op == IrBinOp::AShr {
                IrType::I128
            } else {
                IrType::U128
            };
            let shifted = b.bin(op, value, Operand::Const(IrConst::I32(count as i32)), ty);
            b.emit_wide(shifted);
            let func = b.finish(None);
            let split = validate(&func, 2, &[], 220);
            for args in inputs(2, 220) {
                let source = Shape::Full.reference(&args, 1);
                let expected = match op {
                    IrBinOp::Shl => source << count,
                    IrBinOp::LShr => source >> count,
                    _ => ((source as i128) >> count) as u128,
                };
                let observed = execute(&split, &args, &[]);
                assert_eq!(
                    out_wide(&observed, 0),
                    expected,
                    "{op:?} by {count} {args:x?}"
                );
            }
        }
    }
}

#[test]
fn every_comparison_matches_including_equal_high_words() {
    type Predicate = fn(u128, u128) -> bool;
    let predicates: [(IrCmpOp, Predicate); 10] = [
        (IrCmpOp::Eq, |a, b| a == b),
        (IrCmpOp::Ne, |a, b| a != b),
        (IrCmpOp::Ult, |a, b| a < b),
        (IrCmpOp::Ule, |a, b| a <= b),
        (IrCmpOp::Ugt, |a, b| a > b),
        (IrCmpOp::Uge, |a, b| a >= b),
        (IrCmpOp::Slt, |a, b| (a as i128) < (b as i128)),
        (IrCmpOp::Sle, |a, b| (a as i128) <= (b as i128)),
        (IrCmpOp::Sgt, |a, b| (a as i128) > (b as i128)),
        (IrCmpOp::Sge, |a, b| (a as i128) >= (b as i128)),
    ];
    for (op, reference) in predicates {
        // `shared_high`: both operands take their high word from the same
        // parameter, so the low words decide.
        for shared_high in [false, true] {
            let mut b = Build::new();
            let lhs = Shape::Full.build(&mut b, 1);
            let rhs = if shared_high {
                let lo = b.param(3, IrType::U64);
                let hi = b.param(2, IrType::U64);
                b.wide(lo, hi)
            } else {
                Shape::Full.build(&mut b, 3)
            };
            let flag = b.cmp(op, lhs, rhs, IrType::U128);
            let word = b.cast(flag, IrType::I8, IrType::U64);
            b.emit_word(word);
            let func = b.finish(None);
            let split = validate(&func, 4, &[], 400);
            for args in inputs(4, 400) {
                let a = Shape::Full.reference(&args, 1);
                let c = if shared_high {
                    (u128::from(args[1]) << 64) | u128::from(args[2])
                } else {
                    Shape::Full.reference(&args, 3)
                };
                let observed = execute(&split, &args, &[]);
                assert_eq!(
                    out_word(&observed, 0),
                    u64::from(reference(a, c)),
                    "{op:?} shared_high={shared_high} {args:x?}"
                );
            }
        }
    }
}

#[test]
fn comparison_against_a_literal_and_of_one_value_with_itself() {
    for op in [IrCmpOp::Eq, IrCmpOp::Ne, IrCmpOp::Ult, IrCmpOp::Sge] {
        let mut b = Build::new();
        let value = Shape::Full.build(&mut b, 1);
        let literal = Operand::Const(IrConst::I128(1 << 64));
        let against_literal = b.cmp(op, value, literal, IrType::U128);
        let against_self = b.cmp(op, value, value, IrType::U128);
        for flag in [against_literal, against_self] {
            let word = b.cast(flag, IrType::I8, IrType::U64);
            b.emit_word(word);
        }
        validate(&b.finish(None), 2, &[], 220);
    }
}

#[test]
fn negate_and_complement_match() {
    type Reference = fn(u128) -> u128;
    let ops: [(IrUnaryOp, Reference); 2] = [
        (IrUnaryOp::Neg, |v| 0u128.wrapping_sub(v)),
        (IrUnaryOp::Not, |v| !v),
    ];
    for (op, reference) in ops {
        for shape in SHAPES {
            if matches!(shape, Shape::Literal(_)) {
                continue;
            }
            let mut b = Build::new();
            let src = shape.build(&mut b, 1);
            let dest = b.fresh();
            b.insts.push(Instruction::UnaryOp {
                dest,
                op,
                src,
                ty: IrType::I128,
            });
            b.emit_wide(Operand::Value(dest));
            let func = b.finish(None);
            let split = validate(&func, 2, &[], 220);
            for args in inputs(2, 220) {
                let observed = execute(&split, &args, &[]);
                assert_eq!(
                    out_wide(&observed, 0),
                    reference(shape.reference(&args, 1)),
                    "{op:?} {shape:?} {args:x?}"
                );
            }
        }
    }
}

#[test]
fn products_use_exactly_the_multiplies_their_operands_need() {
    let build = |left: Shape, right: Shape| {
        let mut b = Build::new();
        let lhs = left.build(&mut b, 1);
        let rhs = right.build(&mut b, 3);
        let product = b.bin(IrBinOp::Mul, lhs, rhs, IrType::U128);
        b.emit_wide(product);
        validate(&b.finish(None), 4, &[], 30)
    };
    let census = |func: &IrFunction| {
        (
            count_ops(func, IrBinOp::Mul),
            count_ops(func, IrBinOp::UMulHigh),
            count_ops(func, IrBinOp::SMulHigh),
        )
    };
    // 64x64 -> 128, unsigned and signed: one low and one high multiply.
    assert_eq!(census(&build(Shape::Zext64, Shape::Zext64)), (1, 1, 0));
    assert_eq!(census(&build(Shape::Sext64, Shape::Sext64)), (1, 0, 1));
    assert_eq!(census(&build(Shape::Sext64, Shape::Literal(-3))), (1, 0, 1));
    assert_eq!(census(&build(Shape::Zext64, Shape::Literal(7))), (1, 1, 0));
    // One general operand: one cross product more.
    assert_eq!(census(&build(Shape::Full, Shape::Zext64)), (2, 1, 0));
    assert_eq!(census(&build(Shape::Zext64, Shape::Full)), (2, 1, 0));
    // A sign extension beside a zero extension is not a signed pair.
    assert_eq!(census(&build(Shape::Zext64, Shape::Sext64)), (2, 1, 0));
    // The general product: two cross products.
    assert_eq!(census(&build(Shape::Full, Shape::Full)), (3, 1, 0));
}

/// The shape the x86-64 carry peepholes fuse: the low add, its carry test,
/// A 16-byte transfer through a *parameter base* — the shape the Microsoft
/// variadic cursor has — is a whole-object move the backend already lowers
/// exactly (`Mov128` through `%xmm0`).  Splitting it would replace that with two
/// 8-byte moves through the same pointer, which the wide-instruction model does
/// not describe; that was the `check_ms_abi_varargs.sh` miscompile, so the pass
/// must leave the island alone.
#[test]
fn a_transfer_through_a_parameter_base_is_left_whole() {
    let mut b = Build::new();
    let base = b.pointer(3);
    let second = b.fresh();
    b.insts.push(Instruction::GetElementPtr {
        dest: second,
        base,
        offset: c64(16),
        ty: IrType::Ptr,
    });
    let x = b.load(base, IrType::U128);
    let y = b.load(second, IrType::U128);
    let scaled = Shape::Zext64.build(&mut b, 1);
    let product = b.bin(IrBinOp::Mul, x, scaled, IrType::U128);
    let sum = b.bin(IrBinOp::Add, product, y, IrType::U128);
    b.store(sum, base, IrType::U128);
    assert_untouched(
        b.finish(None),
        "a transfer through a parameter base is a whole-object move",
    );
}

/// The same `GetElementPtr`-derived address off a *local* home: the derived
/// address aliases both halves at their real offsets, so the home is not a pair
/// of independently renamable words either.
#[test]
fn a_home_reached_through_a_derived_address_is_left_whole() {
    let mut b = Build::new();
    let home = b.local();
    let second = b.fresh();
    b.insts.push(Instruction::GetElementPtr {
        dest: second,
        base: home,
        offset: c64(8),
        ty: IrType::Ptr,
    });
    let low = b.load(second, IrType::U64);
    let widened = b.bin(IrBinOp::Mul, low, Operand::Const(IrConst::I64(3)), IrType::U64);
    b.store(widened, second, IrType::U64);
    let wide = b.load(home, IrType::U128);
    let scaled = Shape::Zext64.build(&mut b, 1);
    let sum = b.bin(IrBinOp::Add, wide, scaled, IrType::U128);
    b.store(sum, home, IrType::U128);
    assert_untouched(
        b.finish(None),
        "a home behind a derived address stays whole",
    );
}

/// A local accumulator that never escapes: every transfer is through the home
/// itself and no result leaves whole, so the load, the arithmetic and the store
/// all split together and no 128-bit instruction survives.
#[test]
fn a_local_accumulator_that_stays_local_splits_completely() {
    let mut b = Build::new();
    let home = b.local();
    b.store(
        Operand::Const(IrConst::I128(i128::MAX - 1)),
        home,
        IrType::U128,
    );
    let old = b.load(home, IrType::U128);
    let lhs = Shape::Zext64.build(&mut b, 1);
    let rhs = Shape::Zext64.build(&mut b, 2);
    let product = b.bin(IrBinOp::Mul, lhs, rhs, IrType::U128);
    let sum = b.bin(IrBinOp::Add, old, product, IrType::U128);
    b.store(sum, home, IrType::U128);
    let func = b.finish(None);

    let mut split = func.clone();
    assert!(run_function(&mut split) > 0, "nothing split: {func:#?}");
    assert_eq!(
        wide_instructions(&split),
        0,
        "the accumulator island splits as a whole: {split:#?}"
    );
    assert!(
        !split.blocks[0]
            .instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::Alloca { ty, .. } if is_wide(*ty))),
        "the 16-byte home becomes two 8-byte slots"
    );
}



#[test]
fn phi_and_select_merge_each_word_separately() {
    // entry: branch on p1 <u p2; the arms compute different wide values;
    // join: phi, then a select between the phi and a third value.
    let v = Value;
    let (out, p1, p2, p3) = (v(0), v(1), v(2), v(3));
    let (cond, a, b_val, wide3) = (v(4), v(5), v(6), v(7));
    let (then_value, else_value, merged, chosen) = (v(8), v(9), v(10), v(11));
    let (lo, shifted, hi, lo_at, hi_at) = (v(12), v(13), v(14), v(15), v(16));
    let param = |dest, index, ty| Instruction::ParamRef {
        dest,
        param_idx: index,
        ty,
    };
    let widen = |dest, src: Value, from_ty| Instruction::Cast {
        dest,
        src: Operand::Value(src),
        from_ty,
        to_ty: IrType::I128,
    };
    let entry = block(
        0,
        vec![
            param(out, 0, IrType::Ptr),
            param(p1, 1, IrType::U64),
            param(p2, 2, IrType::U64),
            param(p3, 3, IrType::U64),
            widen(a, p1, IrType::I64),
            widen(b_val, p2, IrType::U64),
            widen(wide3, p3, IrType::I64),
            Instruction::Cmp {
                dest: cond,
                op: IrCmpOp::Ult,
                lhs: Operand::Value(p1),
                rhs: Operand::Value(p2),
                ty: IrType::U64,
            },
        ],
        Terminator::CondBranch {
            cond: Operand::Value(cond),
            true_label: BlockId(1),
            false_label: BlockId(2),
        },
    );
    let arm = |label, dest, op| {
        block(
            label,
            vec![Instruction::BinOp {
                dest,
                op,
                lhs: Operand::Value(a),
                rhs: Operand::Value(b_val),
                ty: IrType::I128,
            }],
            Terminator::Branch(BlockId(3)),
        )
    };
    let store = |val: Value, ptr| Instruction::Store {
        val: Operand::Value(val),
        ptr,
        ty: IrType::U64,
        seg_override: AddressSpace::Default,
        volatile: false,
    };
    let join = block(
        3,
        vec![
            Instruction::Phi {
                dest: merged,
                ty: IrType::I128,
                incoming: vec![
                    (Operand::Value(then_value), BlockId(1)),
                    (Operand::Value(else_value), BlockId(2)),
                ],
            },
            Instruction::Select {
                dest: chosen,
                cond: Operand::Value(p3),
                true_val: Operand::Value(merged),
                false_val: Operand::Value(wide3),
                ty: IrType::I128,
            },
            Instruction::Cast {
                dest: lo,
                src: Operand::Value(chosen),
                from_ty: IrType::I128,
                to_ty: IrType::U64,
            },
            Instruction::BinOp {
                dest: shifted,
                op: IrBinOp::AShr,
                lhs: Operand::Value(chosen),
                rhs: Operand::Const(IrConst::I32(64)),
                ty: IrType::I128,
            },
            Instruction::Cast {
                dest: hi,
                src: Operand::Value(shifted),
                from_ty: IrType::I128,
                to_ty: IrType::U64,
            },
            Instruction::GetElementPtr {
                dest: lo_at,
                base: out,
                offset: c64(0),
                ty: IrType::Ptr,
            },
            Instruction::GetElementPtr {
                dest: hi_at,
                base: out,
                offset: c64(8),
                ty: IrType::Ptr,
            },
            store(lo, lo_at),
            store(hi, hi_at),
        ],
        Terminator::Return(None),
    );
    let func = function(
        vec![
            entry,
            arm(1, then_value, IrBinOp::Mul),
            arm(2, else_value, IrBinOp::Sub),
            join,
        ],
        17,
    );
    let split = validate(&func, 3, &[], 400);
    let phis = split.blocks[3]
        .instructions
        .iter()
        .take_while(|inst| matches!(inst, Instruction::Phi { .. }))
        .count();
    assert_eq!(phis, 2, "one phi per word, both in the phi prefix");
    for args in inputs(3, 400) {
        let (p1, p2, p3) = (args[0], args[1], args[2]);
        let a = i128::from(p1 as i64) as u128;
        let b = u128::from(p2);
        let merged = if p1 < p2 {
            a.wrapping_mul(b)
        } else {
            a.wrapping_sub(b)
        };
        let expected = if p3 != 0 {
            merged
        } else {
            i128::from(p3 as i64) as u128
        };
        assert_eq!(out_wide(&execute(&split, &args, &[]), 0), expected);
    }
}

/// A loop-carried 128-bit accumulator: the phi closes the island through the
/// back edge, which is the shape of the bignum carry.
#[test]
fn a_loop_carried_accumulator_is_split_through_its_phi() {
    let v = Value;
    let (out, p1, p2, count) = (v(0), v(1), v(2), v(3));
    let (factor, step) = (v(4), v(5));
    let (i, acc, product, next_acc, next_i, again) = (v(6), v(7), v(8), v(9), v(10), v(11));
    let (lo, shifted, hi, hi_at) = (v(12), v(13), v(14), v(15));
    let param = |dest, index| Instruction::ParamRef {
        dest,
        param_idx: index,
        ty: IrType::U64,
    };
    let widen = |dest, src: Value| Instruction::Cast {
        dest,
        src: Operand::Value(src),
        from_ty: IrType::U64,
        to_ty: IrType::U128,
    };
    let entry = block(
        0,
        vec![
            Instruction::ParamRef {
                dest: out,
                param_idx: 0,
                ty: IrType::Ptr,
            },
            param(p1, 1),
            param(p2, 2),
            param(count, 3),
            widen(factor, p1),
            widen(step, p2),
        ],
        Terminator::Branch(BlockId(1)),
    );
    let body = block(
        1,
        vec![
            Instruction::Phi {
                dest: i,
                ty: IrType::U64,
                incoming: vec![(c64(0), BlockId(0)), (Operand::Value(next_i), BlockId(1))],
            },
            Instruction::Phi {
                dest: acc,
                ty: IrType::U128,
                incoming: vec![
                    (Operand::Const(IrConst::I128(1)), BlockId(0)),
                    (Operand::Value(next_acc), BlockId(1)),
                ],
            },
            Instruction::BinOp {
                dest: product,
                op: IrBinOp::Mul,
                lhs: Operand::Value(acc),
                rhs: Operand::Value(factor),
                ty: IrType::U128,
            },
            Instruction::BinOp {
                dest: next_acc,
                op: IrBinOp::Add,
                lhs: Operand::Value(product),
                rhs: Operand::Value(step),
                ty: IrType::U128,
            },
            Instruction::BinOp {
                dest: next_i,
                op: IrBinOp::Add,
                lhs: Operand::Value(i),
                rhs: c64(1),
                ty: IrType::U64,
            },
            Instruction::Cmp {
                dest: again,
                op: IrCmpOp::Ult,
                lhs: Operand::Value(next_i),
                rhs: Operand::Value(count),
                ty: IrType::U64,
            },
        ],
        Terminator::CondBranch {
            cond: Operand::Value(again),
            true_label: BlockId(1),
            false_label: BlockId(2),
        },
    );
    let exit = block(
        2,
        vec![
            Instruction::Cast {
                dest: lo,
                src: Operand::Value(next_acc),
                from_ty: IrType::U128,
                to_ty: IrType::U64,
            },
            Instruction::BinOp {
                dest: shifted,
                op: IrBinOp::LShr,
                lhs: Operand::Value(next_acc),
                rhs: Operand::Const(IrConst::I32(64)),
                ty: IrType::U128,
            },
            Instruction::Cast {
                dest: hi,
                src: Operand::Value(shifted),
                from_ty: IrType::U128,
                to_ty: IrType::U64,
            },
            Instruction::GetElementPtr {
                dest: hi_at,
                base: out,
                offset: c64(8),
                ty: IrType::Ptr,
            },
            Instruction::Store {
                val: Operand::Value(lo),
                ptr: out,
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
            Instruction::Store {
                val: Operand::Value(hi),
                ptr: hi_at,
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ],
        Terminator::Return(None),
    );
    let func = function(vec![entry, body, exit], 16);
    let mut split = func.clone();
    assert!(run_function(&mut split) > 0);
    assert_eq!(wide_instructions(&split), 0, "{split:#?}");
    for args in inputs(2, 240) {
        for trips in [1u64, 2, 3, 7] {
            let args = [args[0], args[1], trips];
            let mut expected = 1u128;
            for _ in 0..trips {
                expected = expected
                    .wrapping_mul(u128::from(args[0]))
                    .wrapping_add(u128::from(args[1]));
            }
            let before = execute(&func, &args, &[]);
            let after = execute(&split, &args, &[]);
            assert_eq!(out_wide(&before, 0), expected, "{args:x?}");
            assert_eq!(out_wide(&after, 0), expected, "{args:x?}");
        }
    }
}

fn returned_product() -> (Build, Operand) {
    let mut b = Build::new();
    let lhs = Shape::Zext64.build(&mut b, 1);
    let rhs = Shape::Zext64.build(&mut b, 2);
    let product = b.bin(IrBinOp::Mul, lhs, rhs, IrType::U128);
    (b, product)
}

#[test]
fn an_island_with_an_escaping_value_is_left_exactly_as_it_was() {
    // Returned whole.
    let (b, product) = returned_product();
    assert_untouched(b.finish(Some(product)), "returned value");

    // Passed to a call.
    let (mut b, product) = returned_product();
    let info = CallInfo {
        args: vec![product],
        ..CallInfo::default()
    };
    b.insts.push(Instruction::Call {
        func: "sink".into(),
        info,
    });
    assert_untouched(b.finish(None), "call argument");

    // Shifted by a variable count.
    let (mut b, product) = returned_product();
    let count = b.param(3, IrType::I32);
    let shifted = b.bin(IrBinOp::LShr, product, count, IrType::U128);
    b.emit_wide(shifted);
    assert_untouched(b.finish(None), "variable shift count");

    // Divided: a library call in the backend.
    let (mut b, product) = returned_product();
    let divisor = Shape::Zext64.build(&mut b, 3);
    let quotient = b.bin(IrBinOp::UDiv, product, divisor, IrType::U128);
    b.emit_wide(quotient);
    assert_untouched(b.finish(None), "division");

    // Converted to floating point.
    let (mut b, product) = returned_product();
    let _ = b.cast(product, IrType::U128, IrType::F64);
    assert_untouched(b.finish(None), "floating conversion");

    // A narrower literal in a 128-bit position has no agreed extension.
    let (mut b, product) = returned_product();
    let sum = b.bin(
        IrBinOp::Add,
        product,
        Operand::Const(IrConst::I64(-1)),
        IrType::U128,
    );
    b.emit_wide(sum);
    assert_untouched(b.finish(None), "narrow literal operand");

    // A 128-bit parameter reference.
    let mut b = Build::new();
    let wide_param = b.param(1, IrType::U128);
    let doubled = b.bin(IrBinOp::Add, wide_param, wide_param, IrType::U128);
    b.emit_wide(doubled);
    assert_untouched(b.finish(None), "parameter reference");
}

#[test]
fn volatile_and_segment_accesses_keep_their_island_whole() {
    for (volatile, seg) in [(true, AddressSpace::Default), (false, AddressSpace::SegGs)] {
        let mut b = Build::new();
        let base = b.pointer(3);
        let loaded = b.fresh();
        b.insts.push(Instruction::Load {
            dest: loaded,
            ptr: base,
            ty: IrType::U128,
            seg_override: seg,
            volatile,
        });
        let addend = Shape::Zext64.build(&mut b, 1);
        let sum = b.bin(IrBinOp::Add, Operand::Value(loaded), addend, IrType::U128);
        b.emit_wide(sum);
        assert_untouched(b.finish(None), "volatile or segment load");

        let (mut b, product) = returned_product();
        let base = b.pointer(3);
        b.insts.push(Instruction::Store {
            val: product,
            ptr: base,
            ty: IrType::U128,
            seg_override: seg,
            volatile,
        });
        assert_untouched(b.finish(None), "volatile or segment store");
    }
}

#[test]
fn an_island_that_only_moves_data_keeps_its_16_byte_moves() {
    let mut b = Build::new();
    let base = b.pointer(3);
    let loaded = b.load(base, IrType::U128);
    let copy = b.fresh();
    b.insts.push(Instruction::Copy {
        dest: copy,
        src: loaded,
    });
    let target = b.fresh();
    b.insts.push(Instruction::GetElementPtr {
        dest: target,
        base,
        offset: c64(16),
        ty: IrType::Ptr,
    });
    b.store(Operand::Value(copy), target, IrType::U128);
    assert_untouched(b.finish(None), "pure copy");
}

/// `home += zext(p1) * zext(p2)` then `return home` -- the accumulator is
/// read whole by the return, so it stays in memory; the arithmetic feeding

#[test]
fn a_parameter_home_is_never_split_before_or_after_inlining() {
    // A parameter home is the *caller's* object: its address arrives from the
    // ABI and the prologue writes it, so the pass must not rename its words in
    // either run.  The value that flows *through* it is a different matter — it
    // is an ordinary SSA value and splits when its island is closed.
    let build = || {
        let mut b = Build::new();
        let home = b.local();
        let loaded = b.load(home, IrType::U128);
        let addend = Shape::Zext64.build(&mut b, 1);
        let sum = b.bin(IrBinOp::Add, loaded, addend, IrType::U128);
        b.emit_wide(sum);
        let mut func = b.finish(None);
        func.param_alloca_values = vec![home];
        func
    };
    for run in ["before inlining", "after inlining"] {
        let mut func = build();
        let untouched = format!("{func:?}");
        assert_eq!(
            run_function(&mut func),
            0,
            "{run}: a parameter home is not splittable on its own"
        );
        assert_eq!(
            untouched,
            format!("{func:?}"),
            "{run}: a function the pass declines must be left byte-for-byte alone"
        );
    }
}

#[test]
fn a_wide_read_through_a_computed_pointer_is_left_whole() {
    // The miscompile this covers: a 16-byte load through a pointer the IR
    // cannot name (the Microsoft `va_arg` cursor, a parameter base, any
    // computed address) is a whole-object transfer the backend lowers as one
    // `Mov128`.  Splitting it emitted two 8-byte loads through the *same*
    // pointer, and the wide-instruction model read the pair-relay slot instead
    // (`engineering/REDTEAM-2026-11-11-PR1178.md` §1.2).
    let mut b = Build::new();
    let pointer = b.pointer(0);
    let loaded = b.load(pointer, IrType::U128);
    let one = Shape::Zext64.build(&mut b, 1);
    let computed = b.bin(IrBinOp::Add, loaded, one, IrType::U128);
    b.emit_wide(computed);
    let mut func = b.finish(None);
    let untouched = format!("{func:?}");
    assert_eq!(
        run_function(&mut func),
        0,
        "a load through a non-home pointer must not open an island"
    );
    assert_eq!(untouched, format!("{func:?}"));
}

#[test]
fn a_function_that_is_not_in_ssa_form_is_left_alone() {
    let (mut b, product) = returned_product();
    b.emit_wide(product);
    let Operand::Value(twice) = product else {
        unreachable!("a product is a value")
    };
    b.insts.push(Instruction::Copy {
        dest: twice,
        src: product,
    });
    assert_untouched(b.finish(None), "second definition of one value");
}

#[test]
fn the_local_folder_agrees_with_wrapping_64_bit_arithmetic() {
    let ops = [
        IrBinOp::Add,
        IrBinOp::Sub,
        IrBinOp::Mul,
        IrBinOp::And,
        IrBinOp::Or,
        IrBinOp::Xor,
        IrBinOp::Shl,
        IrBinOp::LShr,
        IrBinOp::AShr,
        IrBinOp::UMulHigh,
        IrBinOp::SMulHigh,
    ];
    let literals: Vec<i64> = EDGES
        .iter()
        .copied()
        .chain(random_words(7).take(26))
        .map(|w| w as i64)
        .chain([-1, 63])
        .collect();
    let unknown = Operand::Value(Value(7));
    for op in ops {
        let ty = if matches!(op, IrBinOp::AShr | IrBinOp::SMulHigh) {
            IrType::I64
        } else {
            IrType::U64
        };
        let is_shift = matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr);
        for &a in &literals {
            for &b in &literals {
                let b = if is_shift { b & 63 } else { b };
                let exact = Machine::binop(op, a as u64 as u128, b as u64 as u128, ty) as u64;
                assert_eq!(
                    fold_bin(op, &c64(a), &c64(b)),
                    Some(c64(exact as i64)),
                    "{op:?} {a:#x} {b:#x}"
                );
            }
            // One literal operand: any fold must hold for every value of the
            // other operand.
            let b = if is_shift { a & 63 } else { a };
            for (lhs, rhs, literal_is_left) in [(c64(a), unknown, true), (unknown, c64(b), false)] {
                let Some(folded) = fold_bin(op, &lhs, &rhs) else {
                    continue;
                };
                for x in EDGES.iter().copied().chain(random_words(11).take(50)) {
                    let x = if is_shift && literal_is_left {
                        x & 63
                    } else {
                        x
                    };
                    let (l, r) = if literal_is_left {
                        (a as u64, x)
                    } else {
                        (x, b as u64)
                    };
                    let exact = Machine::binop(op, u128::from(l), u128::from(r), ty) as u64;
                    let got = match folded {
                        Operand::Value(_) => x,
                        other => const64(&other).expect("64-bit literal") as u64,
                    };
                    assert_eq!(got, exact, "{op:?} {lhs:?} {rhs:?} with {x:#x}");
                }
            }
        }
    }
}
