//! The full adder (`X86Codegen::emit_carry_impl`) is emitted for every
//! place its operands and results can live -- a register, a stack slot, a
//! constant, a register that no longer holds the value -- and each
//! emission is executed on a small machine, for edge values, against
//! `CarryOp::evaluate`.
//!
//! The machine models what the emission depends on: registers at 8, 32 and
//! 64 bits, 8-byte stack cells, and the carry flag, which every arithmetic
//! instruction and `xor` rewrites and no move does. An emission that sets
//! the flag ahead of a load that clobbers it computes the wrong word here.

use super::*;
use crate::backend::state::StackSlot;
use crate::common::fx_hash::FxHashMap;
use crate::ir::reexports::{IntrinsicOp, IrConst};

const NAMES64: [&str; 16] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];
const NAMES32: [&str; 16] = [
    "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d", "r11d", "r12d",
    "r13d", "r14d", "r15d",
];
const NAMES8: [&str; 16] = [
    "al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil", "r8b", "r9b", "r10b", "r11b", "r12b",
    "r13b", "r14b", "r15b",
];

#[derive(Clone, Copy, Debug)]
enum Place {
    Imm(i64),
    Reg(usize, u32),
    Cell(i64),
}

fn place(text: &str) -> Place {
    let text = text.trim_ascii();
    if let Some(imm) = text.strip_prefix('$') {
        return Place::Imm(imm.parse().unwrap_or_else(|_| panic!("immediate {text}")));
    }
    if let Some(name) = text.strip_prefix('%') {
        for (names, bits) in [(&NAMES64, 64), (&NAMES32, 32), (&NAMES8, 8)] {
            if let Some(index) = names.iter().position(|n| *n == name) {
                return Place::Reg(index, bits);
            }
        }
        panic!("the machine has no register {text}");
    }
    let offset = text
        .strip_suffix("(%rbp)")
        .unwrap_or_else(|| panic!("the machine addresses frame cells only, not {text}"));
    Place::Cell(offset.parse().expect("frame offset"))
}

fn mask(bits: u32) -> u64 {
    if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

#[derive(Clone, Default)]
struct Machine {
    regs: [u64; 16],
    cells: FxHashMap<i64, u64>,
    carry: bool,
    saved_flags: Vec<bool>,
}

impl Machine {
    fn read(&self, place: Place, bits: u32) -> u64 {
        let value = match place {
            Place::Imm(imm) => imm as u64,
            Place::Reg(index, _) => self.regs[index],
            Place::Cell(offset) => *self
                .cells
                .get(&offset)
                .unwrap_or_else(|| panic!("read of the unwritten cell {offset}")),
        };
        value & mask(bits)
    }

    fn write(&mut self, place: Place, bits: u32, value: u64) {
        let value = value & mask(bits);
        match place {
            Place::Imm(_) => panic!("write to an immediate"),
            // A 32-bit write clears the upper half; a byte write keeps the
            // other seven.
            Place::Reg(index, _) if bits >= 32 => self.regs[index] = value,
            Place::Reg(index, _) => {
                self.regs[index] = (self.regs[index] & !mask(bits)) | value;
            }
            Place::Cell(offset) => {
                let old = self.cells.get(&offset).copied().unwrap_or(0);
                self.cells.insert(offset, (old & !mask(bits)) | value);
            }
        }
    }

    fn step(&mut self, line: &str) {
        let line = line.trim_ascii();
        let (mnemonic, operands) = line.split_once(' ').unwrap_or((line, ""));
        let operands: Vec<Place> = if operands.is_empty() {
            Vec::new()
        } else {
            operands.split(", ").map(place).collect()
        };
        let width = |suffix: Option<char>| match suffix {
            Some('q') => 64,
            Some('l') => 32,
            other => panic!("width of `{line}` ({other:?})"),
        };
        match mnemonic {
            "movq" | "movl" | "movabsq" => {
                let bits = width(mnemonic.chars().last());
                let value = self.read(operands[0], bits);
                self.write(operands[1], bits, value);
            }
            "movsbq" => {
                let value = self.read(operands[0], 8) as u8 as i8 as i64 as u64;
                self.write(operands[1], 64, value);
            }
            "movzbl" => {
                let value = self.read(operands[0], 8);
                self.write(operands[1], 32, value);
            }
            "movslq" => {
                let value = self.read(operands[0], 32) as u32 as i32 as i64 as u64;
                self.write(operands[1], 64, value);
            }
            "xorl" => {
                let value = self.read(operands[0], 32) ^ self.read(operands[1], 32);
                self.write(operands[1], 32, value);
                self.carry = false;
            }
            "btl" => {
                let Place::Imm(bit) = operands[0] else {
                    panic!("a constant bit: `{line}`");
                };
                self.carry = (self.read(operands[1], 32) >> bit) & 1 != 0;
            }
            "stc" => self.carry = true,
            "pushfq" => self.saved_flags.push(self.carry),
            "popfq" => self.carry = self.saved_flags.pop().expect("a pushed flag"),
            "addq" | "addl" | "adcq" | "adcl" | "subq" | "subl" | "sbbq" | "sbbl" => {
                let bits = width(mnemonic.chars().last());
                let carry_in = u128::from(self.carry && matches!(&mnemonic[..3], "adc" | "sbb"));
                let left = u128::from(self.read(operands[1], bits));
                let right = u128::from(self.read(operands[0], bits)) + carry_in;
                let result = if matches!(&mnemonic[..3], "add" | "adc") {
                    self.carry = left + right > u128::from(mask(bits));
                    left + right
                } else {
                    self.carry = left < right;
                    left.wrapping_sub(right)
                };
                self.write(operands[1], bits, result as u64);
            }
            "setc" => {
                let carry = u64::from(self.carry);
                self.write(operands[0], 8, carry);
            }
            other => panic!("the machine has no `{other}` ({line})"),
        }
    }
}

/// Where an operand lives.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Loc {
    Reg,
    Slot,
    /// A register is assigned, but the value has been moved out of it to
    /// its slot: the general loader has to fetch it.
    StaleReg,
    Const(i64),
}

/// Where a result goes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Home {
    Reg,
    Slot,
    /// The register of `x`, which dies here.
    RegOfX,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Halves {
    Pair,
    Word,
    Out,
}

const X: u32 = 1;
const Y: u32 = 2;
const C: u32 = 3;
const WORD: u32 = 4;
const OUT: u32 = 5;

/// r12, r13, r14, r15, rbx: the homes of x, y, c, the word and the carry.
fn home_reg(value: u32) -> (PhysReg, usize) {
    match value {
        X => (PhysReg(2), 12),
        Y => (PhysReg(3), 13),
        C => (PhysReg(4), 14),
        WORD => (PhysReg(5), 15),
        _ => (PhysReg(1), 3),
    }
}

fn slot(value: u32) -> i64 {
    -8 * i64::from(value)
}

struct Case {
    op: CarryOp,
    halves: Halves,
    x: Loc,
    y: Loc,
    c: Loc,
    word: Home,
    out: Home,
}

impl Case {
    fn operand(&self, value: u32, loc: Loc) -> Operand {
        match loc {
            Loc::Const(k) if value == C => Operand::Const(IrConst::I8(k as i8)),
            Loc::Const(k) if self.op.wide => Operand::Const(IrConst::I64(k)),
            Loc::Const(k) => Operand::Const(IrConst::I32(k as i32)),
            _ => Operand::Value(Value(value)),
        }
    }

    /// The emitted lines.
    fn emit(&self) -> Vec<String> {
        let mut cg = X86Codegen::new();
        cg.state.value_use_counts = vec![1; 8];
        let word_ty = self.op.word_type();
        for (value, loc, ty) in [
            (X, self.x, word_ty),
            (Y, self.y, word_ty),
            (C, self.c, IrType::I8),
        ] {
            cg.value_types.insert(value, ty);
            match loc {
                Loc::Reg => {
                    cg.reg_assignments.insert(value, home_reg(value).0);
                }
                Loc::Slot => {
                    cg.state
                        .value_locations
                        .insert(value, StackSlot(slot(value)));
                }
                Loc::StaleReg => {
                    cg.reg_assignments.insert(value, home_reg(value).0);
                    cg.home_clobbered.insert(value);
                    cg.state
                        .value_locations
                        .insert(value, StackSlot(slot(value)));
                }
                Loc::Const(_) => {}
            }
        }
        for (value, home, ty) in [(WORD, self.word, word_ty), (OUT, self.out, IrType::I8)] {
            cg.value_types.insert(value, ty);
            match home {
                Home::Reg => {
                    cg.reg_assignments.insert(value, home_reg(value).0);
                }
                Home::RegOfX => {
                    cg.reg_assignments.insert(value, home_reg(X).0);
                }
                Home::Slot => {
                    cg.state
                        .value_locations
                        .insert(value, StackSlot(slot(value)));
                }
            }
        }
        let args = [
            self.operand(X, self.x),
            self.operand(Y, self.y),
            self.operand(C, self.c),
        ];
        match self.halves {
            Halves::Pair => {
                assert!(cg.emit_carry_pair_impl(&Value(WORD), &Value(OUT), self.op, &args));
            }
            // Through the arm a half without its partner is emitted by.
            Halves::Word => cg.emit_intrinsic_impl(
                &Some(Value(WORD)),
                &IntrinsicOp::Carry(self.op),
                &None,
                &args,
            ),
            Halves::Out => cg.emit_intrinsic_impl(
                &Some(Value(OUT)),
                &IntrinsicOp::Carry(self.op.partner()),
                &None,
                &args,
            ),
        }
        assert_eq!(cg.state.out.rsp_frame_size, 0, "the stack is level again");
        cg.state
            .out
            .buf
            .lines()
            .map(|line| line.trim_ascii().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }

    /// Runs the emission for `x`, `y`, `c` and compares with the adder.
    fn run(&self, lines: &[String], x: u64, y: u64, c: bool) {
        // Everything the emission was not given holds something else.
        let mut machine = Machine {
            regs: [0xdead_beef_cafe_f00d; 16],
            carry: !c,
            ..Machine::default()
        };
        let word_mask = mask(if self.op.wide { 64 } else { 32 });
        let mut actual = [0u64, 0, 0];
        for (i, (value, loc, given)) in [(X, self.x, x), (Y, self.y, y), (C, self.c, u64::from(c))]
            .into_iter()
            .enumerate()
        {
            // A 32-bit word has anything above it, and a carry is a byte
            // in its slot.
            let stored = match value {
                C => given,
                _ => (given & word_mask) | (!word_mask & 0x5a5a_5a5a_0000_0000),
            };
            actual[i] = given;
            match loc {
                Loc::Reg => machine.regs[home_reg(value).1] = stored,
                Loc::Slot | Loc::StaleReg => {
                    let above = if value == C { 0x7700 } else { 0 };
                    machine.cells.insert(slot(value), stored | above);
                }
                Loc::Const(k) => actual[i] = k as u64,
            }
        }
        let (x, y, c) = (actual[0], actual[1], actual[2] & 1 != 0);
        for line in lines {
            machine.step(line);
        }
        let (want_word, want_out) = self.op.evaluate(x, y, c);
        let result = |machine: &Machine, value: u32, home: Home| match home {
            Home::Reg => machine.regs[home_reg(value).1],
            Home::RegOfX => machine.regs[home_reg(X).1],
            Home::Slot => machine.cells[&slot(value)],
        };
        let listing = || lines.join("\n");
        if self.halves != Halves::Out {
            assert_eq!(
                result(&machine, WORD, self.word),
                want_word,
                "word of {x:#x}, {y:#x}, {c}\n{}",
                listing()
            );
        }
        if self.halves != Halves::Word {
            // 0 or 1 in the whole register.
            assert_eq!(
                result(&machine, OUT, self.out),
                u64::from(want_out),
                "carry of {x:#x}, {y:#x}, {c}\n{}",
                listing()
            );
        }
        assert!(machine.saved_flags.is_empty(), "{}", listing());
    }
}

const EDGES: [u64; 9] = [
    0,
    1,
    0x7fff_ffff,
    0xffff_fffe,
    0xffff_ffff,
    0x1_0000_0000,
    0x8000_0000_0000_0000,
    0xffff_ffff_ffff_fffe,
    0xffff_ffff_ffff_ffff,
];

fn for_every_op(mut each: impl FnMut(CarryOp)) {
    for borrow in [false, true] {
        for wide in [false, true] {
            each(CarryOp {
                borrow,
                out: false,
                wide,
            });
        }
    }
}

#[test]
fn every_placement_of_operands_and_results_computes_the_adder() {
    let mut emissions = 0usize;
    for_every_op(|op| {
        let mut y_locs = vec![
            Loc::Reg,
            Loc::Slot,
            Loc::StaleReg,
            Loc::Const(0),
            Loc::Const(1),
            Loc::Const(-1),
            Loc::Const(0x7fff_ffff),
        ];
        if op.wide {
            // No immediate of `adc` has these.
            y_locs.extend([Loc::Const(0x1_2345_6789), Loc::Const(0x8000_0000)]);
        }
        for halves in [Halves::Pair, Halves::Word, Halves::Out] {
            for x in [
                Loc::Reg,
                Loc::Slot,
                Loc::StaleReg,
                Loc::Const(0),
                Loc::Const(-2),
            ] {
                for &y in &y_locs {
                    for c in [
                        Loc::Reg,
                        Loc::Slot,
                        Loc::StaleReg,
                        Loc::Const(0),
                        Loc::Const(1),
                    ] {
                        for (word, out) in [
                            (Home::Reg, Home::Reg),
                            (Home::Slot, Home::Slot),
                            (Home::Reg, Home::RegOfX),
                        ] {
                            if out == Home::RegOfX && x != Loc::Reg {
                                continue;
                            }
                            let case = Case {
                                op,
                                halves,
                                x,
                                y,
                                c,
                                word,
                                out,
                            };
                            let lines = case.emit();
                            emissions += 1;
                            for xv in EDGES {
                                for yv in EDGES {
                                    case.run(&lines, xv, yv, false);
                                    case.run(&lines, xv, yv, true);
                                }
                            }
                        }
                    }
                }
            }
        }
    });
    assert!(emissions > 3000, "{emissions}");
}

fn emitted(op: CarryOp, halves: Halves, x: Loc, y: Loc, c: Loc) -> Vec<String> {
    Case {
        op,
        halves,
        x,
        y,
        c,
        word: Home::Reg,
        out: Home::Reg,
    }
    .emit()
}

const ADD64: CarryOp = CarryOp {
    borrow: false,
    out: false,
    wide: true,
};

/// The shapes the chain is made of, as text.
#[test]
fn operands_in_registers_take_one_test_and_one_adc() {
    assert_eq!(
        emitted(ADD64, Halves::Pair, Loc::Reg, Loc::Reg, Loc::Reg),
        [
            "movq %r12, %rax",
            "btl $0, %r14d",
            "adcq %r13, %rax",
            "setc %cl",
            "movq %rax, %r15",
            "movzbl %cl, %eax",
            "movq %rax, %rbx",
        ]
    );
    let sub32 = CarryOp {
        borrow: true,
        out: false,
        wide: false,
    };
    assert_eq!(
        emitted(sub32, Halves::Word, Loc::Reg, Loc::Const(-1), Loc::Reg),
        [
            "movq %r12, %rax",
            "btl $0, %r14d",
            "sbbl $-1, %eax",
            "movq %rax, %r15",
        ]
    );
    assert_eq!(
        emitted(ADD64, Halves::Out, Loc::Reg, Loc::Reg, Loc::Slot),
        [
            "movq %r12, %rax",
            "btl $0, -24(%rbp)",
            "adcq %r13, %rax",
            "setc %al",
            "movzbl %al, %eax",
            "movq %rax, %rbx",
        ]
    );
}

/// A constant carry needs no test: 0 is the plain operation, 1 is `stc`.
#[test]
fn a_constant_carry_is_no_carry_or_stc() {
    for_every_op(|op| {
        let (plain, carrying) = if op.borrow {
            ("sub", "sbb")
        } else {
            ("add", "adc")
        };
        let lines = emitted(op, Halves::Pair, Loc::Reg, Loc::Reg, Loc::Const(0));
        assert!(lines.iter().any(|l| l.starts_with(plain)), "{lines:?}");
        assert!(
            !lines
                .iter()
                .any(|l| l.starts_with("bt") || l == "stc" || l.starts_with(carrying)),
            "{lines:?}"
        );
        let lines = emitted(op, Halves::Pair, Loc::Reg, Loc::Reg, Loc::Const(1));
        let stc = lines.iter().position(|l| l == "stc").expect("stc");
        assert!(lines[stc + 1].starts_with(carrying), "{lines:?}");
    });
}

/// Nothing that writes a flag stands between the instruction that sets the
/// carry and the one that reads it, whatever has to be loaded -- except
/// between `pushfq` and `popfq`.
#[test]
fn nothing_writes_the_flags_between_the_test_and_the_adc() {
    let locs = [
        Loc::Reg,
        Loc::Slot,
        Loc::StaleReg,
        Loc::Const(0),
        Loc::Const(0x1_2345_6789),
    ];
    for x in locs {
        for y in locs {
            for c in [Loc::Reg, Loc::Slot, Loc::StaleReg, Loc::Const(1)] {
                let lines = emitted(ADD64, Halves::Pair, x, y, c);
                let set = lines
                    .iter()
                    .position(|l| l.starts_with("btl") || l == "stc")
                    .unwrap_or_else(|| panic!("{lines:?}"));
                let read = lines
                    .iter()
                    .position(|l| l.starts_with("adcq"))
                    .expect("adc");
                let mut saved = false;
                for line in &lines[set + 1..read] {
                    match line.as_str() {
                        "pushfq" => saved = true,
                        "popfq" => saved = false,
                        _ => assert!(
                            saved || line.starts_with("mov"),
                            "`{line}` between the carry and its reader: {lines:?}"
                        ),
                    }
                }
                assert!(!saved, "{lines:?}");
                // The stack is used only where both the second operand and
                // the carry have to be fetched.
                assert_eq!(
                    lines.iter().any(|l| l == "pushfq"),
                    matches!(y, Loc::Slot | Loc::StaleReg) && c == Loc::StaleReg,
                    "{lines:?}"
                );
            }
        }
    }
}

/// The scratch registers hold something else afterwards, and the caches
/// that name their contents must not say otherwise.
#[test]
fn the_scratch_caches_are_dropped() {
    for halves in [Halves::Pair, Halves::Word, Halves::Out] {
        for y in [Loc::Reg, Loc::Slot, Loc::Const(0x1_2345_6789)] {
            let mut cg = X86Codegen::new();
            cg.state.value_use_counts = vec![1; 80];
            for (value, loc) in [(X, Loc::Reg), (Y, y), (C, Loc::Slot)] {
                cg.value_types.insert(value, IrType::U64);
                match loc {
                    Loc::Reg => {
                        cg.reg_assignments.insert(value, home_reg(value).0);
                    }
                    Loc::Slot => {
                        cg.state
                            .value_locations
                            .insert(value, StackSlot(slot(value)));
                    }
                    _ => {}
                }
            }
            cg.reg_assignments.insert(WORD, home_reg(WORD).0);
            cg.reg_assignments.insert(OUT, home_reg(OUT).0);
            cg.state.reg_cache.set_acc(77, false);
            cg.state.reg_cache.set_sec(78, false);
            let y_operand = match y {
                Loc::Const(k) => Operand::Const(IrConst::I64(k)),
                _ => Operand::Value(Value(Y)),
            };
            let args = [
                Operand::Value(Value(X)),
                y_operand,
                Operand::Value(Value(C)),
            ];
            let (word, out) = match halves {
                Halves::Pair => (Some(Value(WORD)), Some(Value(OUT))),
                Halves::Word => (Some(Value(WORD)), None),
                Halves::Out => (None, Some(Value(OUT))),
            };
            cg.emit_carry_impl(word.as_ref(), out.as_ref(), ADD64, &args);
            let text = &cg.state.out.buf;
            assert!(!cg.state.reg_cache.acc_has(77, false), "{halves:?}\n{text}");
            // %rcx is written by the `setc` of a pair and by a staged or
            // wide second operand.
            if halves == Halves::Pair || y != Loc::Reg {
                assert!(
                    !cg.state.reg_cache.sec_has(78, false),
                    "{halves:?} {y:?}\n{text}"
                );
            }
        }
    }
}
