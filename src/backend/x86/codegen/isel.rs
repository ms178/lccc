//! Instruction selection: lower IR instructions to MachInst with virtual registers.
//!
//! Each IR instruction type has a lowering function that produces a sequence of
//! MachInst entries. Virtual registers (MachReg::Vreg) represent IR values that
//! will be assigned physical registers by the MachInst register allocator.
//! Pre-colored registers (MachReg::Phys) are used for x86 constraints like
//! division (rax:rdx) and shifts (rcx/%cl).

use super::machinst::*;
use crate::backend::regalloc::PhysReg;
use crate::common::fx_hash::FxHashMap;
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{
    BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Terminator, Value,
};

// ── Helpers ──────────────────────────────────────────────────────────────

/// Convert an IR Value to a MachReg, using its physical register if already
/// allocated by the main register allocator.
fn value_to_reg(v: &Value, reg_assignments: &FxHashMap<u32, PhysReg>) -> MachReg {
    if let Some(&phys) = reg_assignments.get(&v.0) {
        // XMM registers (20-25) are for floats — shouldn't appear in integer MachInst.
        // Treat them as vregs so they get spilled to stack (safe fallback).
        if phys.0 >= 20 {
            return MachReg::Vreg(v.0);
        }
        MachReg::Phys(phys)
    } else {
        MachReg::Vreg(v.0)
    }
}

/// Convert an IR Operand to a MachOperand, using physical registers for
/// values that already have register assignments from the main allocator.
fn lower_operand_with_regs(op: &Operand, reg_assignments: &FxHashMap<u32, PhysReg>) -> MachOperand {
    match op {
        Operand::Value(v) => MachOperand::Reg(value_to_reg(v, reg_assignments)),
        Operand::Const(c) => MachOperand::Imm(const_to_i64(c)),
    }
}

/// Convert an IR Operand to a MachOperand (without register lookup — for internal use).
fn lower_operand(op: &Operand) -> MachOperand {
    match op {
        Operand::Value(v) => MachOperand::Reg(MachReg::Vreg(v.0)),
        Operand::Const(c) => MachOperand::Imm(const_to_i64(c)),
    }
}

/// Convert an IrConst to an i64 value.
fn const_to_i64(c: &IrConst) -> i64 {
    match c {
        IrConst::I8(v) => *v as i64,
        IrConst::I16(v) => *v as i64,
        IrConst::I32(v) => *v as i64,
        IrConst::I64(v) => *v,
        IrConst::Zero => 0,
        // Float/i128/LongDouble constants: use bit representation
        IrConst::F32(v) => v.to_bits() as i64,
        IrConst::F64(v) => v.to_bits() as i64,
        IrConst::LongDouble(v, _) => v.to_bits() as i64,
        IrConst::I128(v) => *v as i64, // truncate to low 64 bits
    }
}

/// Check if an operand is an immediate that fits in a signed 32-bit value.
fn const_as_imm32(op: &Operand) -> Option<i64> {
    match op {
        Operand::Const(c) => {
            let v = const_to_i64(c);
            if v >= i32::MIN as i64 && v <= i32::MAX as i64 {
                Some(v)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Emit a move from an IR Operand to a register, using physical registers
/// for values already allocated by the main register allocator.
fn emit_mov_operand_r(
    op: &Operand,
    dst: MachReg,
    size: OpSize,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) {
    // v12 Fix E: narrow (S8/S16) Copies to a REGISTER must use the
    // zero-extending form, matching the cast path's "no stale upper bits"
    // principle (isel.rs ~526). A plain `movb $1, %dil` writes only 8 bits;
    // if a later consumer reads the full 32/64 bits (e.g. `movslq %edi`),
    // the stale upper bytes — often a leftover format-string pointer from a
    // preceding printf call — make the sign-extend garbage. `movzbl`/
    // `movzwl` (for Value sources) and `movl $imm` (for Const, zero-extends
    // to 64 bits) give every wider reader a defined value, same as the cast
    // lowering. Only fires for REGISTER destinations; memory (slot) stores
    // keep the narrow size to avoid clobbering a neighbouring 4-byte slot.
    let narrow = matches!(size, OpSize::S8 | OpSize::S16);
    match op {
        Operand::Value(v) => {
            let src_reg = value_to_reg(v, ra);
            if narrow {
                // Even when src_reg == dst, emit movzx — the register may
                // hold stale upper bytes from a prior narrow write.
                out.push(MachInst::Movzx {
                    src: MachOperand::Reg(src_reg),
                    dst,
                    from_size: size,
                    to_size: OpSize::S32,
                });
            } else if src_reg != dst {
                out.push(MachInst::Mov {
                    src: MachOperand::Reg(src_reg),
                    dst: MachOperand::Reg(dst),
                    size,
                });
            }
        }
        Operand::Const(c) => {
            let val = const_to_i64(c);
            if narrow {
                // movl $imm zero-extends to 64 bits — no stale upper bytes.
                out.push(MachInst::Mov {
                    src: MachOperand::Imm(val),
                    dst: MachOperand::Reg(dst),
                    size: OpSize::S32,
                });
            } else {
                out.push(MachInst::Mov {
                    src: MachOperand::Imm(val),
                    dst: MachOperand::Reg(dst),
                    size,
                });
            }
        }
    }
}

/// Emit a move (legacy wrapper without reg_assignments — for internal helpers).
fn emit_mov_operand(op: &Operand, dst: MachReg, size: OpSize, out: &mut Vec<MachInst>) {
    emit_mov_operand_r(op, dst, size, &FxHashMap::default(), out);
}

/// Emit an ALU instruction with an IR Operand as source.
/// For large immediates that don't fit in i32, materialize to rax first.
fn emit_alu_operand_r(
    op: AluOp,
    src: &Operand,
    dst: MachReg,
    size: OpSize,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) {
    let src_op = lower_operand_with_regs(src, ra);
    // x86 ALU instructions only support i32 immediates. For larger values,
    // materialize to the scratch register (rax) first.
    if let MachOperand::Imm(v) = &src_op {
        if *v < i32::MIN as i64 || *v > i32::MAX as i64 {
            out.push(MachInst::Mov {
                src: MachOperand::Imm(*v),
                dst: MachOperand::Reg(MachReg::Phys(RAX)),
                size,
            });
            out.push(MachInst::Alu {
                op,
                src: MachOperand::Reg(MachReg::Phys(RAX)),
                dst,
                size,
            });
            return;
        }
    }
    out.push(MachInst::Alu {
        op,
        src: src_op,
        dst,
        size,
    });
}

/// Map IrBinOp to AluOp for simple two-address operations.
fn binop_to_alu(op: IrBinOp) -> Option<AluOp> {
    match op {
        IrBinOp::Add => Some(AluOp::Add),
        IrBinOp::Sub => Some(AluOp::Sub),
        IrBinOp::And => Some(AluOp::And),
        IrBinOp::Or => Some(AluOp::Or),
        IrBinOp::Xor => Some(AluOp::Xor),
        IrBinOp::Mul => Some(AluOp::Imul),
        _ => None,
    }
}

/// Map IrBinOp to ShiftOp.
fn binop_to_shift(op: IrBinOp) -> Option<ShiftOp> {
    match op {
        IrBinOp::Shl => Some(ShiftOp::Shl),
        IrBinOp::LShr => Some(ShiftOp::Shr),
        IrBinOp::AShr => Some(ShiftOp::Sar),
        _ => None,
    }
}

/// Check if an immediate is a LEA scale factor (3, 5, or 9).
/// Returns the scale (2, 4, or 8) for the LEA index.
fn lea_scale_for_mul(imm: i64) -> Option<u8> {
    match imm {
        3 => Some(2), // lea (%r, %r, 2), %r  → r * 3
        5 => Some(4), // lea (%r, %r, 4), %r  → r * 5
        9 => Some(8), // lea (%r, %r, 8), %r  → r * 9
        _ => None,
    }
}

// ── BinOp Lowering ───────────────────────────────────────────────────────

/// Lower an IR BinOp instruction to MachInst sequence.
pub fn lower_binop(
    dest: &Value,
    op: IrBinOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) {
    let size = OpSize::from_ir_type(ty);
    let dst = value_to_reg(dest, ra);

    // 64-bit Add is an address-generation operation regardless of whether the
    // C source spells it as pointer arithmetic or integer arithmetic. Use LEA
    // whenever both inputs fit its base/index/displacement form. Unlike the old
    // mov+add sequence this is one uop and does not create a flags dependency.
    if op == IrBinOp::Add && size == OpSize::S64 {
        match (lhs, rhs) {
            (Operand::Value(base), Operand::Value(index)) => {
                out.push(MachInst::Lea {
                    base: value_to_reg(base, ra),
                    index: Some((value_to_reg(index, ra), 1)),
                    offset: 0,
                    dst,
                });
                return;
            }
            (Operand::Value(base), Operand::Const(_)) => {
                if let Some(offset) = const_as_imm32(rhs) {
                    out.push(MachInst::Lea {
                        base: value_to_reg(base, ra),
                        index: None,
                        offset,
                        dst,
                    });
                    return;
                }
            }
            (Operand::Const(_), Operand::Value(base)) => {
                if let Some(offset) = const_as_imm32(lhs) {
                    out.push(MachInst::Lea {
                        base: value_to_reg(base, ra),
                        index: None,
                        offset,
                        dst,
                    });
                    return;
                }
            }
            _ => {}
        }
    }

    // ── Simple ALU operations (two-address form) ─────────────────────
    if let Some(alu_op) = binop_to_alu(op) {
        if op == IrBinOp::Mul {
            if let Some(imm) = const_as_imm32(rhs) {
                if let Some(scale) = lea_scale_for_mul(imm) {
                    emit_mov_operand_r(lhs, dst, size, ra, out);
                    out.push(MachInst::Lea {
                        base: dst,
                        index: Some((dst, scale)),
                        offset: 0,
                        dst,
                    });
                    return;
                }
                if imm != 0 && imm != 1 {
                    let src = match lhs {
                        Operand::Value(v) => value_to_reg(v, ra),
                        Operand::Const(_) => {
                            emit_mov_operand_r(lhs, dst, size, ra, out);
                            dst
                        }
                    };
                    out.push(MachInst::Imul3 {
                        imm,
                        src,
                        dst,
                        size,
                    });
                    return;
                }
            }
        }
        emit_mov_operand_r(lhs, dst, size, ra, out);
        emit_alu_operand_r(alu_op, rhs, dst, size, ra, out);
        return;
    }

    // ── Shift operations ─────────────────────────────────────────────
    if let Some(shift_op) = binop_to_shift(op) {
        emit_mov_operand_r(lhs, dst, size, ra, out);
        if let Some(imm) = const_as_imm32(rhs) {
            let mask = if size == OpSize::S32 { 31 } else { 63 };
            out.push(MachInst::Shift {
                op: shift_op,
                amount: MachOperand::Imm(imm & mask),
                dst,
                size,
            });
        } else {
            emit_mov_operand_r(rhs, MachReg::Phys(RCX), size, ra, out);
            out.push(MachInst::Shift {
                op: shift_op,
                amount: MachOperand::Reg(MachReg::Phys(RCX)),
                dst,
                size,
            });
        }
        return;
    }

    // ── Division and remainder ───────────────────────────────────────
    match op {
        IrBinOp::SDiv | IrBinOp::SRem => {
            emit_mov_operand_r(lhs, MachReg::Phys(RAX), size, ra, out);
            out.push(MachInst::Cqto { size });
            let mut divisor_op = lower_operand_with_regs(rhs, ra);
            // x86 div/idiv has no immediate form. Materialize constants in the
            // dedicated rcx scratch register rather than emitting invalid
            // `idivq $imm` text (SQLite amalgamation assembler failure).
            if let MachOperand::Imm(value) = divisor_op {
                out.push(MachInst::Mov {
                    src: MachOperand::Imm(value),
                    dst: MachOperand::Reg(MachReg::Phys(RCX)),
                    size,
                });
                divisor_op = MachOperand::Reg(MachReg::Phys(RCX));
            }
            out.push(MachInst::Div {
                divisor: divisor_op,
                signed: true,
                size,
            });
            let result_phys = if op == IrBinOp::SDiv { RAX } else { RDX };
            out.push(MachInst::Mov {
                src: MachOperand::Reg(MachReg::Phys(result_phys)),
                dst: MachOperand::Reg(dst),
                size,
            });
        }
        IrBinOp::UDiv | IrBinOp::URem => {
            emit_mov_operand_r(lhs, MachReg::Phys(RAX), size, ra, out);
            out.push(MachInst::XorRdx);
            let mut divisor_op = lower_operand_with_regs(rhs, ra);
            if let MachOperand::Imm(value) = divisor_op {
                out.push(MachInst::Mov {
                    src: MachOperand::Imm(value),
                    dst: MachOperand::Reg(MachReg::Phys(RCX)),
                    size,
                });
                divisor_op = MachOperand::Reg(MachReg::Phys(RCX));
            }
            out.push(MachInst::Div {
                divisor: divisor_op,
                signed: false,
                size,
            });
            let result_phys = if op == IrBinOp::UDiv { RAX } else { RDX };
            out.push(MachInst::Mov {
                src: MachOperand::Reg(MachReg::Phys(result_phys)),
                dst: MachOperand::Reg(dst),
                size,
            });
        }
        _ => unreachable!("unhandled binop: {:?}", op),
    }
}

// ── Load / Store / Copy ──────────────────────────────────────────────────

/// Lower an IR Load: dest = *ptr.
pub fn lower_load(
    dest: &Value,
    ptr: &Value,
    ty: IrType,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) {
    let size = OpSize::from_ir_type(ty);
    let dst = value_to_reg(dest, ra);
    let base = value_to_reg(ptr, ra);
    out.push(MachInst::Mov {
        src: MachOperand::Mem { base, offset: 0 },
        dst: MachOperand::Reg(dst),
        size,
    });
}

/// Lower an IR Store: *ptr = val.
pub fn lower_store(
    val: &Operand,
    ptr: &Value,
    ty: IrType,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) {
    let size = OpSize::from_ir_type(ty);
    let base = value_to_reg(ptr, ra);
    let src = lower_operand_with_regs(val, ra);
    out.push(MachInst::Mov {
        src,
        dst: MachOperand::Mem { base, offset: 0 },
        size,
    });
}

/// Lower an IR Copy: dest = src.
/// `size` must match the value's IR width so slot-to-slot relays stay
/// width-consistent with 4-byte small spill slots.
pub fn lower_copy(
    dest: &Value,
    src: &Operand,
    size: crate::backend::x86::codegen::machinst::OpSize,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) {
    let dst = value_to_reg(dest, ra);
    emit_mov_operand_r(src, dst, size, ra, out);
}

// ── Comparison ───────────────────────────────────────────────────────────

/// Map IrCmpOp to CondCode.
fn cmp_to_cc(op: IrCmpOp) -> CondCode {
    match op {
        IrCmpOp::Eq => CondCode::E,
        IrCmpOp::Ne => CondCode::Ne,
        IrCmpOp::Slt => CondCode::L,
        IrCmpOp::Sle => CondCode::Le,
        IrCmpOp::Sgt => CondCode::G,
        IrCmpOp::Sge => CondCode::Ge,
        IrCmpOp::Ult => CondCode::B,
        IrCmpOp::Ule => CondCode::Be,
        IrCmpOp::Ugt => CondCode::A,
        IrCmpOp::Uge => CondCode::Ae,
    }
}

/// Lower an IR Cmp: dest = lhs CMP rhs (boolean result).
pub fn lower_cmp(
    dest: &Value,
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) {
    let size = OpSize::from_ir_type(ty);
    let dst = value_to_reg(dest, ra);
    let cc = cmp_to_cc(op);
    let mut lhs_op = lower_operand_with_regs(lhs, ra);
    let mut rhs_op = lower_operand_with_regs(rhs, ra);

    // AT&T cmp encodes `cmp rhs,lhs`; lhs is the ModRM destination and cannot
    // be an immediate. Materialize it in rax. A rhs immediate wider than imm32
    // uses rcx so it cannot overwrite an lhs already materialized in rax.
    if let MachOperand::Imm(value) = lhs_op {
        out.push(MachInst::Mov {
            src: MachOperand::Imm(value),
            dst: MachOperand::Reg(MachReg::Phys(RAX)),
            size,
        });
        lhs_op = MachOperand::Reg(MachReg::Phys(RAX));
    }
    if let MachOperand::Imm(value) = rhs_op {
        if value < i32::MIN as i64 || value > i32::MAX as i64 {
            out.push(MachInst::Mov {
                src: MachOperand::Imm(value),
                dst: MachOperand::Reg(MachReg::Phys(RCX)),
                size,
            });
            rhs_op = MachOperand::Reg(MachReg::Phys(RCX));
        }
    }
    out.push(MachInst::Cmp {
        lhs: lhs_op,
        rhs: rhs_op,
        size,
    });
    out.push(MachInst::SetCC { cc, dst });
    out.push(MachInst::Movzx {
        src: MachOperand::Reg(dst),
        dst,
        from_size: OpSize::S8,
        to_size: OpSize::S32,
    });
}

/// Lower a fused Cmp + CondBranch (no boolean materialization).
pub fn lower_cmp_branch(
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    true_block: BlockId,
    false_block: BlockId,
    out: &mut Vec<MachInst>,
) {
    let size = OpSize::from_ir_type(ty);
    let cc = cmp_to_cc(op);
    let lhs_op = lower_operand(lhs);
    let rhs_op = lower_operand(rhs);
    out.push(MachInst::Cmp {
        lhs: lhs_op,
        rhs: rhs_op,
        size,
    });
    out.push(MachInst::Jcc {
        cc,
        target: format!(".LBB{}", true_block.0),
    });
    out.push(MachInst::Jmp {
        target: format!(".LBB{}", false_block.0),
    });
}

// ── Cast ─────────────────────────────────────────────────────────────────

/// Lower an IR Cast (integer-to-integer only; float casts go through Raw).
pub fn lower_cast(
    dest: &Value,
    src: &Operand,
    from_ty: IrType,
    to_ty: IrType,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) {
    let dst = value_to_reg(dest, ra);
    let from_size = OpSize::from_ir_type(from_ty);
    let to_size = OpSize::from_ir_type(to_ty);

    if to_size as u8 <= from_size as u8 {
        // Narrowing or same-size cast.  A plain truncating move (movb/movw/
        // movl) would leave the destination register's UPPER BITS STALE.
        // x86 code may legitimately consume the value at a wider width:
        // the folded-SIB index path reads a never-materialized cast result
        // straight from its (possibly die-at-birth-shared) home at 64-bit
        // width — zlib-ng's zng_emit_dist computed `code` as uint8_t and
        // folded it into `extra_dbits(%rcx,%r10,4)`; the stale high bytes of
        // %r10 made the index garbage and segfaulted out of bounds.  Emit
        // the extending forms instead: same length, no partial-register
        // false dependency, and every wider reader sees a defined value
        // (matches the mature path and GCC/Clang lowering).
        match src {
            Operand::Const(_) => {
                // movl $imm zero-extends; movq $imm carries the sign-extended
                // mathematical value for signed sources.  movzx/movsx have no
                // immediate forms, so pick the width by destination sign.
                let size = if to_ty.is_signed() {
                    OpSize::S64
                } else {
                    OpSize::S32
                };
                emit_mov_operand_r(src, dst, size, ra, out);
            }
            Operand::Value(v) => {
                let src_reg = value_to_reg(v, ra);
                match (to_size, to_ty.is_signed()) {
                    (OpSize::S8, true) => out.push(MachInst::Movsx {
                        src: MachOperand::Reg(src_reg),
                        dst,
                        from_size: OpSize::S8,
                        to_size: OpSize::S32,
                    }),
                    (OpSize::S8, false) => out.push(MachInst::Movzx {
                        src: MachOperand::Reg(src_reg),
                        dst,
                        from_size: OpSize::S8,
                        to_size: OpSize::S32,
                    }),
                    (OpSize::S16, true) => out.push(MachInst::Movsx {
                        src: MachOperand::Reg(src_reg),
                        dst,
                        from_size: OpSize::S16,
                        to_size: OpSize::S32,
                    }),
                    (OpSize::S16, false) => out.push(MachInst::Movzx {
                        src: MachOperand::Reg(src_reg),
                        dst,
                        from_size: OpSize::S16,
                        to_size: OpSize::S32,
                    }),
                    // I32 narrowing: movslq sign-extends to 64 bits; movl
                    // zero-extends — mirror the mature path exactly.
                    // Movzx/S32->S32 lowers to `movl` but, unlike a plain
                    // Mov, is never elided as a self-move: a die-at-birth
                    // shared home must still be re-zero-extended.
                    (OpSize::S32, true) => out.push(MachInst::Movsx {
                        src: MachOperand::Reg(src_reg),
                        dst,
                        from_size: OpSize::S32,
                        to_size: OpSize::S64,
                    }),
                    (OpSize::S32, false) => out.push(MachInst::Movzx {
                        src: MachOperand::Reg(src_reg),
                        dst,
                        from_size: OpSize::S32,
                        to_size: OpSize::S32,
                    }),
                    // Same-size 64-bit: identical bits.
                    _ => emit_mov_operand_r(src, dst, to_size, ra, out),
                }
            }
        }
        return;
    }

    let src_reg = match src {
        Operand::Value(v) => value_to_reg(v, ra),
        Operand::Const(_) => {
            emit_mov_operand_r(src, dst, to_size, ra, out);
            return;
        }
    };

    // Widening extension is determined solely by the SOURCE type. C converts
    // a signed negative source to its mathematical value before conversion to
    // a wider unsigned destination, so I32(-1)->U64 must sign-extend to
    // UINT64_MAX. Conversely U32->I64 zero-extends. Including destination
    // unsignedness here zero-extended negative SQLite VDBE values and caused
    // PRAGMA integrity_check to produce no result row.
    if from_ty.is_unsigned() {
        out.push(MachInst::Movzx {
            src: MachOperand::Reg(src_reg),
            dst,
            from_size,
            to_size,
        });
    } else {
        out.push(MachInst::Movsx {
            src: MachOperand::Reg(src_reg),
            dst,
            from_size,
            to_size,
        });
    }
}

// ── Unary Operations ─────────────────────────────────────────────────────

/// Lower an IR UnaryOp (neg, not only; bswap/clz/ctz/popcount go through Raw).
pub fn lower_unaryop(
    dest: &Value,
    op: IrUnaryOp,
    src: &Operand,
    ty: IrType,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) -> bool {
    let size = OpSize::from_ir_type(ty);
    let dst = value_to_reg(dest, ra);

    match op {
        IrUnaryOp::Neg => {
            emit_mov_operand_r(src, dst, size, ra, out);
            out.push(MachInst::Neg { dst, size });
            true
        }
        IrUnaryOp::Not => {
            emit_mov_operand_r(src, dst, size, ra, out);
            out.push(MachInst::Not { dst, size });
            true
        }
        _ => false,
    }
}

// ── Select (conditional move) ────────────────────────────────────────────

/// Lower an IR Select: dest = cond ? true_val : false_val.
pub fn lower_select(
    dest: &Value,
    cond: &Operand,
    true_val: &Operand,
    false_val: &Operand,
    ty: IrType,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) {
    let size = OpSize::from_ir_type(ty);
    let dst = value_to_reg(dest, ra);
    emit_mov_operand_r(false_val, dst, size, ra, out);
    emit_mov_operand_r(true_val, MachReg::Phys(RAX), size, ra, out);
    let cond_op = lower_operand_with_regs(cond, ra);
    out.push(MachInst::Test {
        lhs: cond_op.clone(),
        rhs: cond_op,
        size: OpSize::S64,
    });
    out.push(MachInst::Cmov {
        cc: CondCode::Ne,
        src: MachOperand::Reg(MachReg::Phys(RAX)),
        dst,
        size,
    });
}

// ── GEP (pointer arithmetic) ─────────────────────────────────────────────

/// Lower an IR GetElementPtr: dest = base + offset.
pub fn lower_gep(
    dest: &Value,
    base: &Value,
    offset: &Operand,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) {
    let dst = value_to_reg(dest, ra);
    let base_reg = value_to_reg(base, ra);

    if let Some(imm) = const_as_imm32(offset) {
        if imm == 0 {
            if base_reg != dst {
                out.push(MachInst::Mov {
                    src: MachOperand::Reg(base_reg),
                    dst: MachOperand::Reg(dst),
                    size: OpSize::S64,
                });
            }
        } else {
            out.push(MachInst::Lea {
                base: base_reg,
                index: None,
                offset: imm,
                dst,
            });
        }
        return;
    }

    if let Operand::Value(index) = offset {
        // x86 has a native base+index addressing calculation. The old
        // MachInst lowering emitted `mov base,dst; add index,dst`, doubling
        // instruction count in pointer-heavy gzip/SQLite loops and creating an
        // unnecessary flags dependency. LEA is one flag-neutral instruction.
        out.push(MachInst::Lea {
            base: base_reg,
            index: Some((value_to_reg(index, ra), 1)),
            offset: 0,
            dst,
        });
        return;
    }

    // Defensive fallback for any future non-constant/non-value operand kind.
    out.push(MachInst::Mov {
        src: MachOperand::Reg(base_reg),
        dst: MachOperand::Reg(dst),
        size: OpSize::S64,
    });
    out.push(MachInst::Alu {
        op: AluOp::Add,
        src: lower_operand_with_regs(offset, ra),
        dst,
        size: OpSize::S64,
    });
}

// ── Terminator lowering ──────────────────────────────────────────────────

/// Lower a conditional branch (non-fused): test cond, jne true, jmp false.
pub fn lower_cond_branch(
    cond: &Operand,
    true_block: BlockId,
    false_block: BlockId,
    out: &mut Vec<MachInst>,
) {
    let cond_op = lower_operand(cond);
    out.push(MachInst::Test {
        lhs: cond_op.clone(),
        rhs: cond_op,
        size: OpSize::S64,
    });
    out.push(MachInst::Jcc {
        cc: CondCode::Ne,
        target: format!(".LBB{}", true_block.0),
    });
    out.push(MachInst::Jmp {
        target: format!(".LBB{}", false_block.0),
    });
}

// ── Block-level lowering (integration entry point) ───────────────────────

/// Check if an IR instruction can be lowered to MachInst.
/// Instructions that can't are emitted as Raw passthrough via the existing codegen.
pub fn can_lower(inst: &Instruction) -> bool {
    match inst {
        Instruction::BinOp { ty, .. } => !ty.is_float() && !ty.is_128bit(),
        Instruction::Load { ty, .. } => !ty.is_float() && !ty.is_128bit() && !ty.is_long_double(),
        Instruction::Store { ty, .. } => !ty.is_float() && !ty.is_128bit() && !ty.is_long_double(),
        Instruction::Copy { .. } => true,
        Instruction::Cmp { ty, .. } => !ty.is_float() && !ty.is_128bit(),
        Instruction::Cast { from_ty, to_ty, .. } => {
            !from_ty.is_float()
                && !to_ty.is_float()
                && !from_ty.is_128bit()
                && !to_ty.is_128bit()
                && !from_ty.is_long_double()
                && !to_ty.is_long_double()
        }
        Instruction::UnaryOp { op, ty, .. } => {
            !ty.is_float() && !ty.is_128bit() && matches!(op, IrUnaryOp::Neg | IrUnaryOp::Not)
        }
        Instruction::Select { ty, .. } => !ty.is_float() && !ty.is_128bit(),
        Instruction::GetElementPtr { .. } => true,
        Instruction::GlobalAddr { .. } => true,
        _ => false,
    }
}

/// Context for ISel: provides slot information for alloca-aware lowering.
pub struct ISelContext<'a> {
    pub reg_assignments: &'a FxHashMap<u32, PhysReg>,
    /// Maps alloca value IDs to their stack slot offsets.
    /// Used for Load/Store from allocas (direct slot access).
    pub alloca_slots: &'a FxHashMap<u32, i64>,
}

/// Lower a single IR instruction to MachInst.
/// Returns true if lowered, false if it should use Raw passthrough.
pub fn lower_instruction(
    inst: &Instruction,
    reg_assignments: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) -> bool {
    lower_instruction_ctx(inst, reg_assignments, &FxHashMap::default(), out)
}

/// Lower with full context (alloca slots for Load/Store).
pub fn lower_instruction_ctx(
    inst: &Instruction,
    reg_assignments: &FxHashMap<u32, PhysReg>,
    alloca_slots: &FxHashMap<u32, i64>,
    out: &mut Vec<MachInst>,
) -> bool {
    lower_instruction_typed(inst, reg_assignments, alloca_slots, None, out)
}

/// Lower with full context plus a value-type map for width-consistent
/// Copy lowering. `value_types` may be None (tests); copies then keep the
/// historical 64-bit width.
pub fn lower_instruction_typed(
    inst: &Instruction,
    reg_assignments: &FxHashMap<u32, PhysReg>,
    alloca_slots: &FxHashMap<u32, i64>,
    value_types: Option<&FxHashMap<u32, crate::common::types::IrType>>,
    out: &mut Vec<MachInst>,
) -> bool {
    // For values that already have register allocations from the existing
    // allocator, use their physical register directly (MachReg::Phys).
    // The MachInst allocator only handles the remaining Vreg values.

    let ra = reg_assignments;
    match inst {
        Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => {
            if ty.is_float() || ty.is_128bit() {
                return false;
            }
            // BitTest is lowered by the text path, which uses BT plus
            // register/stack-aware SETCC materialization and keeps the i32
            // result zero-extended. MachInst currently has no BT opcode.
            if *op == IrBinOp::BitTest {
                return false;
            }
            // Only handle I32/U32/I64/U64/Ptr — sub-32-bit types have complex
            // register sub-register interactions that need special handling.
            if matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16) {
                return false;
            }
            lower_binop(dest, *op, lhs, rhs, *ty, ra, out);
            true
        }
        Instruction::Load {
            dest,
            ptr,
            ty,
            seg_override,
            ..
        } => {
            if ty.is_float() || ty.is_128bit() || ty.is_long_double() {
                return false;
            }
            if matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16) {
                return false;
            }
            if *seg_override != AddressSpace::Default {
                return false;
            }
            let size = OpSize::from_ir_type(*ty);
            let dst = value_to_reg(dest, ra);
            // Alloca: load directly from stack slot
            if let Some(&slot) = alloca_slots.get(&ptr.0) {
                out.push(MachInst::Mov {
                    src: MachOperand::StackSlot(slot),
                    dst: MachOperand::Reg(dst),
                    size,
                });
                return true;
            }
            // Pointer in register: load via memory operand
            if let Some(&phys) = ra.get(&ptr.0) {
                out.push(MachInst::Mov {
                    src: MachOperand::Mem {
                        base: MachReg::Phys(phys),
                        offset: 0,
                    },
                    dst: MachOperand::Reg(dst),
                    size,
                });
                return true;
            }
            // Pointer on stack: load ptr to rcx, then dereference
            let ptr_vreg = value_to_reg(ptr, ra);
            out.push(MachInst::Mov {
                src: MachOperand::Reg(ptr_vreg),
                dst: MachOperand::Reg(MachReg::Phys(RCX)),
                size: OpSize::S64,
            });
            out.push(MachInst::Mov {
                src: MachOperand::Mem {
                    base: MachReg::Phys(RCX),
                    offset: 0,
                },
                dst: MachOperand::Reg(dst),
                size,
            });
            true
        }
        Instruction::Store {
            val,
            ptr,
            ty,
            seg_override,
            ..
        } => {
            if ty.is_float() || ty.is_128bit() || ty.is_long_double() {
                return false;
            }
            if matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16) {
                return false;
            }
            if *seg_override != AddressSpace::Default {
                return false;
            }
            let size = OpSize::from_ir_type(*ty);
            let src = lower_operand_with_regs(val, ra);
            // Alloca: store directly to stack slot
            if let Some(&slot) = alloca_slots.get(&ptr.0) {
                out.push(MachInst::Mov {
                    src,
                    dst: MachOperand::StackSlot(slot),
                    size,
                });
                return true;
            }
            // Pointer in register: store via memory operand
            if let Some(&phys) = ra.get(&ptr.0) {
                out.push(MachInst::Mov {
                    src,
                    dst: MachOperand::Mem {
                        base: MachReg::Phys(phys),
                        offset: 0,
                    },
                    size,
                });
                return true;
            }
            // Pointer on stack: load ptr to rcx, then store through it
            let ptr_vreg = value_to_reg(ptr, ra);
            out.push(MachInst::Mov {
                src: MachOperand::Reg(ptr_vreg),
                dst: MachOperand::Reg(MachReg::Phys(RCX)),
                size: OpSize::S64,
            });
            out.push(MachInst::Mov {
                src,
                dst: MachOperand::Mem {
                    base: MachReg::Phys(RCX),
                    offset: 0,
                },
                size,
            });
            true
        }
        Instruction::Copy { dest, src } => {
            // Width-consistent copy: a ≤32-bit value spilled to a 4-byte small
            // slot must be moved with a 32-bit mov. The historical
            // unconditional S64 relay (`movq slot,%rax; movq %rax,slot`)
            // reads 4 stale neighbour bytes out of a small src slot AND
            // clobbers the neighbour of a small dst slot (the -O0 loop-phi
            // o0_phi_multidef miscompile). The Copy instruction itself is
            // type-polymorphic, so the width comes from the value-type map
            // (propagated Copy/producer types); unknown types keep S64,
            // which matches the 8-byte slots untyped values receive.
            let copy_size = value_types.and_then(|vt| {
                let ty = match src {
                    Operand::Value(v) => vt.get(&v.0).copied(),
                    _ => None,
                };
                let ty = ty.or_else(|| vt.get(&dest.0).copied());
                ty.map(crate::backend::x86::codegen::machinst::OpSize::from_ir_type)
            });
            let copy_size = copy_size.unwrap_or(crate::backend::x86::codegen::machinst::OpSize::S64);
            lower_copy(dest, src, copy_size, ra, out);
            true
        }
        Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => {
            if ty.is_float() || ty.is_128bit() {
                return false;
            }
            // Dest must have a register for SetCC (writes to 8-bit register).
            // Stack-only dests are rejected by try_lower_machinst.
            if !ra.contains_key(&dest.0) {
                return false;
            }
            lower_cmp(dest, *op, lhs, rhs, *ty, ra, out);
            true
        }
        Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        } => {
            if from_ty.is_float() || to_ty.is_float() {
                return false;
            }
            if from_ty.is_128bit() || to_ty.is_128bit() {
                return false;
            }
            if from_ty.is_long_double() || to_ty.is_long_double() {
                return false;
            }
            // Dest must have a register for Movsx/Movzx.
            if !ra.contains_key(&dest.0) {
                return false;
            }
            lower_cast(dest, src, *from_ty, *to_ty, ra, out);
            true
        }
        Instruction::UnaryOp { dest, op, src, ty } => {
            if ty.is_float() || ty.is_128bit() {
                return false;
            }
            if matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16) {
                return false;
            }
            lower_unaryop(dest, *op, src, *ty, ra, out)
        }
        Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        } => {
            if ty.is_float() || ty.is_128bit() {
                return false;
            }
            // cmov doesn't exist for 8-bit operands — fall back
            if matches!(ty, IrType::I8 | IrType::U8) {
                return false;
            }
            lower_select(dest, cond, true_val, false_val, *ty, ra, out);
            true
        }
        Instruction::GetElementPtr {
            dest, base, offset, ..
        } => {
            // Only non-alloca base with register (alloca GEP needs leaq)
            if !ra.contains_key(&base.0) {
                return false;
            }
            lower_gep(dest, base, offset, ra, out);
            true
        }
        Instruction::GlobalAddr { .. } => {
            // GlobalAddr needs leaq symbol(%rip) which isn't directly
            // expressible in MachInst Mov (would produce movq, not leaq).
            // Handled by the default codegen path for now.
            false
        }
        Instruction::Alloca { .. } => {
            // Alloca produces no code (stack allocated in prologue).
            // Return true to avoid flushing the MachInst buffer.
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widening_cast_uses_source_signedness() {
        let mut assignments = FxHashMap::default();
        assignments.insert(1, PhysReg(1));
        assignments.insert(2, PhysReg(2));

        let mut signed_to_unsigned = Vec::new();
        lower_cast(
            &Value(2),
            &Operand::Value(Value(1)),
            IrType::I32,
            IrType::U64,
            &assignments,
            &mut signed_to_unsigned,
        );
        assert!(matches!(
            signed_to_unsigned.as_slice(),
            [MachInst::Movsx {
                from_size: OpSize::S32,
                to_size: OpSize::S64,
                ..
            }]
        ));

        let mut unsigned_to_signed = Vec::new();
        lower_cast(
            &Value(2),
            &Operand::Value(Value(1)),
            IrType::U32,
            IrType::I64,
            &assignments,
            &mut unsigned_to_signed,
        );
        assert!(matches!(
            unsigned_to_signed.as_slice(),
            [MachInst::Movzx {
                from_size: OpSize::S32,
                to_size: OpSize::S64,
                ..
            }]
        ));
    }

    #[test]
    fn narrowing_cast_extends_the_register() {
        // A truncating `movb`/`movw` would leave the destination register's
        // upper bits stale.  The folded-SIB index path reads a
        // never-materialized cast result straight from its (possibly
        // die-at-birth-shared) home at 64-bit width — zlib-ng's
        // zng_emit_dist computed `code` as uint8_t and folded it into
        // `extra_dbits(%rcx,%r10,4)`, and the stale high bytes made the
        // index garbage (out-of-bounds segfault).  Narrowing casts must
        // therefore extend, matching the mature path and GCC/Clang.
        let mut assignments = FxHashMap::default();
        assignments.insert(1, PhysReg(1));
        assignments.insert(2, PhysReg(2));

        let mut u32_to_u8 = Vec::new();
        lower_cast(
            &Value(2),
            &Operand::Value(Value(1)),
            IrType::U32,
            IrType::U8,
            &assignments,
            &mut u32_to_u8,
        );
        assert!(matches!(
            u32_to_u8.as_slice(),
            [MachInst::Movzx {
                from_size: OpSize::S8,
                to_size: OpSize::S32,
                ..
            }]
        ));

        let mut i32_to_i8 = Vec::new();
        lower_cast(
            &Value(2),
            &Operand::Value(Value(1)),
            IrType::I32,
            IrType::I8,
            &assignments,
            &mut i32_to_i8,
        );
        assert!(matches!(
            i32_to_i8.as_slice(),
            [MachInst::Movsx {
                from_size: OpSize::S8,
                to_size: OpSize::S32,
                ..
            }]
        ));

        let mut u64_to_u32 = Vec::new();
        lower_cast(
            &Value(2),
            &Operand::Value(Value(1)),
            IrType::U64,
            IrType::U32,
            &assignments,
            &mut u64_to_u32,
        );
        // movl-via-Movzx: always emitted (a plain Mov would be elided as a
        // self-move on a die-at-birth shared home and skip the extension).
        assert!(matches!(
            u64_to_u32.as_slice(),
            [MachInst::Movzx {
                from_size: OpSize::S32,
                to_size: OpSize::S32,
                ..
            }]
        ));

        let mut const_to_u8 = Vec::new();
        lower_cast(
            &Value(2),
            &Operand::Const(IrConst::I64(0x1000)),
            IrType::I64,
            IrType::U8,
            &assignments,
            &mut const_to_u8,
        );
        // movl $imm zero-extends; movzx has no immediate form.
        assert!(matches!(
            const_to_u8.as_slice(),
            [MachInst::Mov {
                size: OpSize::S32,
                ..
            }]
        ));
    }

    #[test]
    fn compare_with_immediate_lhs_materializes_destination_operand() {
        let mut assignments = FxHashMap::default();
        assignments.insert(1, PhysReg(1));
        let mut out = Vec::new();
        lower_cmp(
            &Value(1),
            IrCmpOp::Eq,
            &Operand::Const(IrConst::I64(0)),
            &Operand::Const(IrConst::I64(0)),
            IrType::I64,
            &assignments,
            &mut out,
        );
        assert!(matches!(
            out.first(),
            Some(MachInst::Mov {
                src: MachOperand::Imm(0),
                dst: MachOperand::Reg(MachReg::Phys(RAX)),
                size: OpSize::S64,
            })
        ));
        assert!(out.iter().any(|inst| matches!(
            inst,
            MachInst::Cmp {
                lhs: MachOperand::Reg(MachReg::Phys(RAX)),
                rhs: MachOperand::Imm(0),
                size: OpSize::S64,
            }
        )));
    }

    #[test]
    fn immediate_divisor_is_materialized_in_rcx() {
        let mut assignments = FxHashMap::default();
        assignments.insert(1, PhysReg(1));
        assignments.insert(2, PhysReg(2));
        let mut out = Vec::new();
        lower_binop(
            &Value(2),
            IrBinOp::SDiv,
            &Operand::Value(Value(1)),
            &Operand::Const(IrConst::I64(8)),
            IrType::I64,
            &assignments,
            &mut out,
        );
        assert!(out.iter().any(|inst| matches!(
            inst,
            MachInst::Mov {
                src: MachOperand::Imm(8),
                dst: MachOperand::Reg(MachReg::Phys(RCX)),
                size: OpSize::S64,
            }
        )));
        assert!(out.iter().any(|inst| matches!(
            inst,
            MachInst::Div {
                divisor: MachOperand::Reg(MachReg::Phys(RCX)),
                signed: true,
                size: OpSize::S64,
            }
        )));
        assert!(!out.iter().any(|inst| matches!(
            inst,
            MachInst::Div {
                divisor: MachOperand::Imm(_),
                ..
            }
        )));
    }

    #[test]
    fn i64_add_lowers_to_single_lea() {
        let mut assignments = FxHashMap::default();
        assignments.insert(1, PhysReg(1));
        assignments.insert(2, PhysReg(2));
        assignments.insert(3, PhysReg(3));
        let mut out = Vec::new();
        lower_binop(
            &Value(3),
            IrBinOp::Add,
            &Operand::Value(Value(1)),
            &Operand::Value(Value(2)),
            IrType::I64,
            &assignments,
            &mut out,
        );
        assert!(matches!(
            out.as_slice(),
            [MachInst::Lea {
                base: MachReg::Phys(PhysReg(1)),
                index: Some((MachReg::Phys(PhysReg(2)), 1)),
                offset: 0,
                dst: MachReg::Phys(PhysReg(3)),
            }]
        ));
    }

    #[test]
    fn variable_gep_lowers_to_single_lea() {
        let mut assignments = FxHashMap::default();
        assignments.insert(1, PhysReg(1));
        assignments.insert(2, PhysReg(2));
        assignments.insert(3, PhysReg(3));
        let mut out = Vec::new();
        lower_gep(
            &Value(3),
            &Value(1),
            &Operand::Value(Value(2)),
            &assignments,
            &mut out,
        );
        assert!(matches!(
            out.as_slice(),
            [MachInst::Lea {
                base: MachReg::Phys(PhysReg(1)),
                index: Some((MachReg::Phys(PhysReg(2)), 1)),
                offset: 0,
                dst: MachReg::Phys(PhysReg(3)),
            }]
        ));
    }
}
