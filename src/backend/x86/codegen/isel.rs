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

/// True when a physical register is an XMM float home (xmm2..xmm15, the
/// allocator's float bank; xmm0/xmm1 are text-path scratch and never appear
/// as allocator homes). Mirrors `is_xmm_reg` in emit.rs.
fn is_xmm_phys(reg: PhysReg) -> bool {
    reg.0 >= 20 && reg.0 <= 33
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
        IrConst::D32(v) => *v as i64,
        IrConst::D64(v) => *v as i64,
        IrConst::LongDouble(v, _) => v.to_bits() as i64,
        IrConst::I128(v) => *v as i64, // truncate to low 64 bits
    }
}

/// Check if an operand is an immediate that fits in a signed 32-bit value.
fn const_as_imm32(op: &Operand) -> Option<i64> {
    const_as_imm32_size(op, OpSize::S64)
}

/// Immediate-32 encoding. 64-bit ops sign-extend imm32, so only the signed
/// i32 range is representable. 32-bit ops use the imm32 bit pattern verbatim,
/// so any value in `[0, u32::MAX]` encodes (as a signed i32 with the same
/// low 32 bits — `2654435761u` → `-1640531535`).
fn const_as_imm32_size(op: &Operand, size: OpSize) -> Option<i64> {
    match op {
        Operand::Const(c) => {
            let v = const_to_i64(c);
            if v >= i32::MIN as i64 && v <= i32::MAX as i64 {
                Some(v)
            } else if size == OpSize::S32 && v >= 0 && v <= u32::MAX as i64 {
                Some(v as i32 as i64)
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
    // materialize to the scratch register (rax) first. 32-bit ops can encode
    // any u32 bit pattern as a signed imm32, so skip the movabsq shuttle.
    if let MachOperand::Imm(v) = &src_op {
        if *v < i32::MIN as i64 || *v > i32::MAX as i64 {
            if size == OpSize::S32 && *v >= 0 && *v <= u32::MAX as i64 {
                out.push(MachInst::Alu {
                    op,
                    src: MachOperand::Imm(*v as i32 as i64),
                    dst,
                    size,
                });
                return;
            }
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
///
/// Returns false when this operation cannot be lowered safely under the
/// current register assignment (the caller must re-emit it through the
/// default, mature path — which sequences its moves with read-then-write
/// IR semantics). `out` is untouched when false is returned.
pub fn lower_binop(
    dest: &Value,
    op: IrBinOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) -> bool {
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
                return true;
            }
            (Operand::Value(base), Operand::Const(_)) => {
                if let Some(offset) = const_as_imm32(rhs) {
                    out.push(MachInst::Lea {
                        base: value_to_reg(base, ra),
                        index: None,
                        offset,
                        dst,
                    });
                    return true;
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
                    return true;
                }
            }
            _ => {}
        }
    }

    // ── Simple ALU operations (two-address form) ─────────────────────
    if let Some(alu_op) = binop_to_alu(op) {
        if op == IrBinOp::Mul {
            if let Some(imm) = const_as_imm32_size(rhs, size) {
                if let Some(scale) = lea_scale_for_mul(imm) {
                    emit_mov_operand_r(lhs, dst, size, ra, out);
                    out.push(MachInst::Lea {
                        base: dst,
                        index: Some((dst, scale)),
                        offset: 0,
                        dst,
                    });
                    return true;
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
                    return true;
                }
            }
        }
        // Two-address aliasing constraint: the lowered form
        // `mov lhs,dst; alu rhs,dst` reads rhs AFTER dst has been written,
        // but the allocator only guarantees the IR's read-then-write
        // semantics — homing rhs and dest in one register is legal there
        // (dest is born at this instruction, rhs dies at it). The machine
        // form would silently compute `lhs OP lhs` instead of
        // `lhs OP rhs`. Repro: sqlite3GetVarint's `b <<= 14; b |= *p` —
        // the byte load and dest homed to %edx produced `or %edx,%edx`,
        // dropping the byte and mis-decoding every 9-byte varint.
        //
        // Commutative ops (Add/And/Or/Xor/Mul) swap the operands so the
        // register-homed non-dest operand is the one that survives the
        // mov; Sub (and any shape where the surviving operand would still
        // alias dst) falls back to the mature emitter. Flags are not
        // observable through IR BinOp, so the swap is semantically
        // transparent.
        let rhs_home_is_dst = matches!(rhs, Operand::Value(v) if v.0 != dest.0)
            && matches!(rhs, Operand::Value(v) if value_to_reg(v, ra) == value_to_reg(dest, ra))
            && matches!(value_to_reg(dest, ra), MachReg::Phys(_));
        if rhs_home_is_dst {
            let lhs_survives = match lhs {
                Operand::Const(_) => true,
                Operand::Value(v) => value_to_reg(v, ra) != value_to_reg(dest, ra),
            };
            let commutative = !matches!(op, IrBinOp::Sub);
            if commutative && lhs_survives {
                emit_mov_operand_r(rhs, dst, size, ra, out);
                emit_alu_operand_r(alu_op, lhs, dst, size, ra, out);
                return true;
            }
            return false;
        }
        emit_mov_operand_r(lhs, dst, size, ra, out);
        emit_alu_operand_r(alu_op, rhs, dst, size, ra, out);
        return true;
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
        return true;
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
            true
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
            true
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
    // `cmp $0, %reg` ≡ `test %reg, %reg` for the integer condition codes we
    // emit (ZF/SF/CF/OF). `test` is shorter and does not take an immediate.
    let rhs_is_zero = matches!(rhs_op, MachOperand::Imm(0));
    if rhs_is_zero && matches!(lhs_op, MachOperand::Reg(_)) {
        out.push(MachInst::Test {
            lhs: lhs_op.clone(),
            rhs: lhs_op,
            size,
        });
    } else {
        out.push(MachInst::Cmp {
            lhs: lhs_op,
            rhs: rhs_op,
            size,
        });
    }
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
    if matches!(rhs_op, MachOperand::Imm(0)) && matches!(lhs_op, MachOperand::Reg(_)) {
        out.push(MachInst::Test {
            lhs: lhs_op.clone(),
            rhs: lhs_op,
            size,
        });
    } else {
        out.push(MachInst::Cmp {
            lhs: lhs_op,
            rhs: rhs_op,
            size,
        });
    }
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
/// Per-run ISel coverage census, printed at process exit when
/// `CCC_ISEL_STATS=1`.
///
/// "How much of codegen actually flows through MachInst?" is the structural
/// health metric for this layer, and it was previously unmeasurable: the
/// fallback to direct text emission is silent, so a shrinking coverage
/// fraction -- or a whole instruction class nobody noticed was excluded --
/// looks exactly like everything being fine. The counters make the gap
/// visible and rank it, so work targets the biggest class rather than the
/// most obvious one.
pub mod stats {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    pub static LOWERED: AtomicU64 = AtomicU64::new(0);
    pub static REJECTED: AtomicU64 = AtomicU64::new(0);

    static BY_KIND: Mutex<Vec<(&'static str, u64)>> = Mutex::new(Vec::new());

    pub fn enabled() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var("CCC_ISEL_STATS").is_ok())
    }

    /// Record a lowering that a caller handled outside `lower_instruction_typed`.
    pub fn note_lowered() {
        if enabled() {
            LOWERED.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a rejection with an explicit label, for callers that reject
    /// before `lower_instruction_typed` is reached.
    pub fn note_reject_named(kind: &'static str) {
        if enabled() {
            note_reject(kind);
        }
    }

    pub fn note_reject(kind: &'static str) {
        REJECTED.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut v) = BY_KIND.lock() {
            match v.iter_mut().find(|(k, _)| *k == kind) {
                Some((_, n)) => *n += 1,
                None => v.push((kind, 1)),
            }
        }
    }

    /// Print the census. Call once, at the end of compilation.
    pub fn report() {
        if !enabled() {
            return;
        }
        let lowered = LOWERED.load(Ordering::Relaxed);
        let rejected = REJECTED.load(Ordering::Relaxed);
        let total = lowered + rejected;
        if total == 0 {
            return;
        }
        eprintln!(
            "[ISEL-STATS] {} of {} instructions lowered through MachInst ({:.1}%)",
            lowered,
            total,
            lowered as f64 * 100.0 / total as f64
        );
        if let Ok(mut v) = BY_KIND.lock() {
            v.sort_by(|a, b| b.1.cmp(&a.1));
            for (kind, n) in v.iter().take(12) {
                eprintln!(
                    "[ISEL-STATS]   rejected {:>7}  {} ({:.1}% of all)",
                    n,
                    kind,
                    *n as f64 * 100.0 / total as f64
                );
            }
        }
    }
}

/// Discriminant name, for the coverage census.
fn kind_name(inst: &Instruction) -> &'static str {
    match inst {
        Instruction::BinOp { ty, .. } if ty.is_float() => "BinOp(float)",
        Instruction::BinOp { .. } => "BinOp(other)",
        Instruction::Load { ty, .. } if ty.is_float() => "Load(float)",
        Instruction::Load { .. } => "Load(other)",
        Instruction::Store { ty, .. } if ty.is_float() => "Store(float)",
        Instruction::Store { .. } => "Store(other)",
        Instruction::Cmp { ty, .. } if ty.is_float() => "Cmp(float)",
        Instruction::Cmp { .. } => "Cmp(other)",
        Instruction::Cast { .. } => "Cast",
        Instruction::UnaryOp { .. } => "UnaryOp",
        Instruction::Select { .. } => "Select",
        Instruction::Call { .. } => "Call",
        Instruction::CallIndirect { .. } => "CallIndirect",
        Instruction::Phi { .. } => "Phi",
        Instruction::Alloca { .. } => "Alloca",
        Instruction::Intrinsic { .. } => "Intrinsic",
        Instruction::InlineAsm { .. } => "InlineAsm",
        Instruction::Memcpy { .. } => "Memcpy",
        Instruction::AtomicLoad { .. }
        | Instruction::AtomicStore { .. }
        | Instruction::AtomicRmw { .. }
        | Instruction::AtomicCmpxchg { .. } => "Atomic*",
        Instruction::ParamRef { .. } => "ParamRef",
        Instruction::GetElementPtr { .. } => "GetElementPtr",
        Instruction::GlobalAddr { .. } => "GlobalAddr",
        Instruction::Copy { .. } => "Copy",
        Instruction::VaArg { .. } => "VaArg",
        Instruction::Fence { .. } => "Fence",
        other => {
            // Name the remaining variants rather than lumping them into a
            // bucket that hides the biggest opportunity.
            let d = format!("{:?}", other);
            let name = d.split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("other");
            Box::leak(format!("other:{}", name).into_boxed_str())
        }
    }
}

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

/// Lower an integer load into a register home, DEFINING the full 64-bit
/// home for narrow (8/16-bit) types.
///
/// A plain `Mov` at S8/S16 renders as `movb mem, %r9b` / `movw mem, %r9w`:
/// only the sub-register is written, bits [8:64) of the home stay undefined.
/// The typed ISel accepts loads of every integer width, but the BinOp arm
/// refuses sub-32-bit types, so narrow-homed values routinely cross into
/// the mature text path, whose lowering reads operands at their promoted
/// width (e.g. U8 division materializes `movq %r9, %rax; divq`) and at
/// raw 64-bit homes (SIB-index construction). pr60960 (v4qi lane division)
/// consumed exactly such a garbage-extended home and aborted.
///
/// Emit the extending 32-bit-destination forms instead — `movzbl`/`movzwl`
/// for unsigned sources, `movsbl`/`movswl` for signed ones. Writing the
/// 32-bit sub-register zeroes the upper half, so the whole home is defined:
/// narrow readers see the unchanged low byte, wide readers see the
/// canonical extension of the value. Same instruction length as the plain
/// forms, no partial-register false dependency — identical to the mature
/// text path's lowering and to GCC/Clang (see the Cast lowering's
/// zlib-ng `zng_emit_dist` note for the same doctrine).
fn narrow_defined_load(ty: IrType, size: OpSize, src: MachOperand, dst: MachReg) -> MachInst {
    if size == OpSize::S8 || size == OpSize::S16 {
        if ty.is_signed() {
            MachInst::Movsx {
                src,
                dst,
                from_size: size,
                to_size: OpSize::S32,
            }
        } else {
            MachInst::Movzx {
                src,
                dst,
                from_size: size,
                to_size: OpSize::S32,
            }
        }
    } else {
        MachInst::Mov {
            src,
            dst: MachOperand::Reg(dst),
            size,
        }
    }
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
    let lowered = lower_instruction_typed_inner(inst, reg_assignments, alloca_slots, value_types, out);
    if stats::enabled() {
        if lowered {
            stats::LOWERED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else {
            stats::note_reject(kind_name(inst));
        }
    }
    lowered
}

fn lower_instruction_typed_inner(
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
            if ty.is_128bit() {
                return false;
            }
            // Scalar float arithmetic (F32/F64) in the VEX three-operand
            // form. Subset: dest xmm-homed; lhs xmm-homed with rhs xmm- or
            // slot-homed; or (commutative ops only) rhs xmm-homed with lhs
            // slot-homed. Slot-homed operands become Vregs here and resolve
            // to StackSlot memory operands at flush time (resolve_stack_vregs);
            // an operand that resolves to nothing trips the fallback replay.
            // Constants and the sub/div-with-slot-lhs staging case (needs the
            // xmm0 scratch) stay on the mature text path.
            if matches!(ty, IrType::F32 | IrType::F64) {
                // Float division arrives as SDiv with a float type (the IR
                // has no separate FDiv); UDiv/Rem with float types do not
                // occur and stay on the text path.
                let fop = match op {
                    IrBinOp::Add => FAluOp::Add,
                    IrBinOp::Sub => FAluOp::Sub,
                    IrBinOp::Mul => FAluOp::Mul,
                    IrBinOp::SDiv => FAluOp::Div,
                    _ => return false,
                };
                let size = OpSize::from_ir_type(*ty);
                let Some(&dst_reg) = ra.get(&dest.0) else {
                    return false;
                };
                if !is_xmm_phys(dst_reg) {
                    return false;
                }
                let (Operand::Value(lhs_v), Operand::Value(rhs_v)) = (lhs, rhs) else {
                    return false;
                };
                let lhs_xmm = ra.get(&lhs_v.0).copied().filter(|r| is_xmm_phys(*r));
                let rhs_xmm = ra.get(&rhs_v.0).copied().filter(|r| is_xmm_phys(*r));
                let commutative = matches!(fop, FAluOp::Add | FAluOp::Mul);
                // NOTE: value_to_reg is NOT usable for the xmm side -- it
                // deliberately maps XMM homes to Vregs for the integer
                // path. Build the operands explicitly: xmm home -> Phys,
                // anything else -> Vreg (flush-time slot resolution).
                let reg_or_vreg = |vid: u32, xmm: Option<PhysReg>| match xmm {
                    Some(r) => MachOperand::Reg(MachReg::Phys(r)),
                    None => MachOperand::Reg(MachReg::Vreg(vid)),
                };
                let (src1, src2) = if let Some(l) = lhs_xmm {
                    // src1 = lhs; src2 = rhs (xmm register, or a vreg that
                    // flush-time resolution turns into a slot operand).
                    (l, reg_or_vreg(rhs_v.0, rhs_xmm))
                } else if commutative && rhs_xmm.is_some() {
                    // Swapped form, valid for add/mul only. This mirrors the
                    // text path's own swap when only the rhs is xmm-homed.
                    (rhs_xmm.unwrap(), reg_or_vreg(lhs_v.0, None))
                } else {
                    return false;
                };
                out.push(MachInst::FAlu {
                    op: fop,
                    src2,
                    src1: MachReg::Phys(src1),
                    dst: MachReg::Phys(dst_reg),
                    size,
                });
                return true;
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
            lower_binop(dest, *op, lhs, rhs, *ty, ra, out)
        }
        Instruction::Load {
            dest,
            ptr,
            ty,
            seg_override,
            ..
        } => {
            if ty.is_128bit() || ty.is_long_double() {
                return false;
            }
            // Scalar float load: mirror of the float store subset. The dest
            // must be xmm-homed (xmm2..xmm15); the address a direct alloca
            // slot or a GPR-held pointer. Slot-homed dests, constants and
            // exotic addressing stay on the mature text path.
            if matches!(ty, IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64) {
                if *seg_override != AddressSpace::Default {
                    return false;
                }
                let Some(&dst_reg) = ra.get(&dest.0) else {
                    return false;
                };
                if !is_xmm_phys(dst_reg) {
                    return false;
                }
                let size = if matches!(ty, IrType::F32 | IrType::D32) {
                    OpSize::S32
                } else {
                    OpSize::S64
                };
                let dst = MachOperand::Reg(MachReg::Phys(dst_reg));
                if let Some(&slot) = alloca_slots.get(&ptr.0) {
                    out.push(MachInst::FMov {
                        src: MachOperand::StackSlot(slot),
                        dst,
                        size,
                    });
                    return true;
                }
                if let Some(&base) = ra.get(&ptr.0) {
                    if is_xmm_phys(base) {
                        return false;
                    }
                    out.push(MachInst::FMov {
                        src: MachOperand::Mem {
                            base: MachReg::Phys(base),
                            offset: 0,
                        },
                        dst,
                        size,
                    });
                    return true;
                }
                return false;
            }
            if *seg_override != AddressSpace::Default {
                return false;
            }
            let size = OpSize::from_ir_type(*ty);
            let dst = value_to_reg(dest, ra);
            // Alloca: load directly from stack slot
            if let Some(&slot) = alloca_slots.get(&ptr.0) {
                out.push(narrow_defined_load(
                    *ty,
                    size,
                    MachOperand::StackSlot(slot),
                    dst,
                ));
                return true;
            }
            // Pointer in register: load via memory operand
            if let Some(&phys) = ra.get(&ptr.0) {
                out.push(narrow_defined_load(
                    *ty,
                    size,
                    MachOperand::Mem {
                        base: MachReg::Phys(phys),
                        offset: 0,
                    },
                    dst,
                ));
                return true;
            }
            // Pointer on stack: load ptr to rcx, then dereference
            let ptr_vreg = value_to_reg(ptr, ra);
            out.push(MachInst::Mov {
                src: MachOperand::Reg(ptr_vreg),
                dst: MachOperand::Reg(MachReg::Phys(RCX)),
                size: OpSize::S64,
            });
            out.push(narrow_defined_load(
                *ty,
                size,
                MachOperand::Mem {
                    base: MachReg::Phys(RCX),
                    offset: 0,
                },
                dst,
            ));
            true
        }
        Instruction::Store {
            val,
            ptr,
            ty,
            seg_override,
            ..
        } => {
            if ty.is_128bit() || ty.is_long_double() {
                return false;
            }
            // Scalar float store (F32/F64 and the D32/D64 bit-carriers).
            //
            // Typed subset, mirroring the SSE branch of emit_store_impl: the
            // value must already be homed in an xmm allocator register
            // (xmm2..xmm15) and the address must be a direct alloca slot or
            // a GPR-held pointer. Everything else -- slot-homed values (the
            // text path relays through an xmm scratch), constants, xmm-based
            // pointers, indexed/folded GEP addressing, F128/x87 -- keeps the
            // mature emitter.
            if matches!(ty, IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64) {
                if *seg_override != AddressSpace::Default {
                    return false;
                }
                let Operand::Value(val_v) = val else {
                    return false;
                };
                let Some(&src_reg) = ra.get(&val_v.0) else {
                    return false;
                };
                if !is_xmm_phys(src_reg) {
                    return false;
                }
                let size = if matches!(ty, IrType::F32 | IrType::D32) {
                    OpSize::S32
                } else {
                    OpSize::S64
                };
                let src = MachOperand::Reg(MachReg::Phys(src_reg));
                if let Some(&slot) = alloca_slots.get(&ptr.0) {
                    out.push(MachInst::FMov {
                        src,
                        dst: MachOperand::StackSlot(slot),
                        size,
                    });
                    return true;
                }
                if let Some(&base) = ra.get(&ptr.0) {
                    // x86 addressing has no xmm base; the text path relays
                    // such pointers through rcx -- leave that shape there.
                    if is_xmm_phys(base) {
                        return false;
                    }
                    out.push(MachInst::FMov {
                        src,
                        dst: MachOperand::Mem {
                            base: MachReg::Phys(base),
                            offset: 0,
                        },
                        size,
                    });
                    return true;
                }
                return false;
            }
            // Narrow stores are ordinary `Mov`s at OpSize::S8/S16 and the
            // emitter's size tables have always handled both; refusing them
            // only split contiguous MachInst runs and pushed byte traffic --
            // the dominant operation in the gzip / zlib-ng / expat workloads --
            // onto the untyped text path.
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
            // Scalar float copy: both sides must be xmm-homed (xmm2..xmm15),
            // which makes it a single register-to-register movss/movsd. Any
            // mixed or slot-homed shape needs the xmm scratch relay the text
            // path owns, so it stays there.
            let src_vid = match src {
                Operand::Value(v) => Some(v.0),
                Operand::Const(_) => None,
            };
            let float_ty = value_types.and_then(|vt| {
                src_vid
                    .and_then(|s| vt.get(&s).copied())
                    .or_else(|| vt.get(&dest.0).copied())
            });
            if let Some(ty) = float_ty {
                if matches!(ty, IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64) {
                    let (Some(src_v), Some(&src_reg)) = (src_vid, src_vid.and_then(|s| ra.get(&s)))
                    else {
                        return false;
                    };
                    let Some(&dst_reg) = ra.get(&dest.0) else {
                        return false;
                    };
                    if !is_xmm_phys(src_reg) || !is_xmm_phys(dst_reg) {
                        return false;
                    }
                    let size = if matches!(ty, IrType::F32 | IrType::D32) {
                        OpSize::S32
                    } else {
                        OpSize::S64
                    };
                    out.push(MachInst::FMov {
                        src: MachOperand::Reg(MachReg::Phys(src_reg)),
                        dst: MachOperand::Reg(MachReg::Phys(dst_reg)),
                        size,
                    });
                    return true;
                }
                // F128/long double and 128-bit copies: text path.
                if ty.is_long_double() || ty.is_128bit() {
                    return false;
                }
            }
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
        Instruction::GlobalAddr { dest, name, .. } => {
            // `leaq sym(%rip), %dst`. Previously bailed to the text emitter
            // because MachInst could not express "address of" as distinct from
            // "load from"; `LeaSym` now does.
            out.push(MachInst::LeaSym {
                sym: name.clone(),
                dst: value_to_reg(dest, ra),
            });
            true
        }
        Instruction::Alloca { .. } => {
            // Alloca produces no code (stack allocated in prologue).
            // Return true to avoid flushing the MachInst buffer.
            true
        }
        // ParamRef is DELIBERATELY not lowered here, despite being 11.7% of
        // all instructions and appearing to emit nothing.
        //
        // Treating it as a no-op like Alloca fails four corpus tests, all of
        // them parameter-related (x86_fpo_stack_params_many_args,
        // nested_nonlocal_goto_callee_saved, pgo_branchy, value_profiling):
        // a parameter is only already-in-place when it arrived in a register
        // and kept that home. Stack-passed arguments and nested-function
        // frames still need the text path to materialize them, and the buffer
        // flush that `false` triggers is what orders that against surrounding
        // MachInst code. Lowering ParamRef properly means modelling the
        // incoming-argument location, not asserting there is nothing to do.
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
            MachInst::Test {
                lhs: MachOperand::Reg(MachReg::Phys(RAX)),
                rhs: MachOperand::Reg(MachReg::Phys(RAX)),
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
    fn u32_mul_large_imm_uses_signed_imm32_not_movabs() {
        // Knuth multiplicative hash: 2654435761u as i32 is -1640531535.
        // Encoding it as imm32 avoids `movabsq` + `imull %eax, %edi`.
        let mut assignments = FxHashMap::default();
        assignments.insert(1, PhysReg(14)); // rdi
        assignments.insert(2, PhysReg(14));
        let mut out = Vec::new();
        lower_binop(
            &Value(2),
            IrBinOp::Mul,
            &Operand::Value(Value(1)),
            &Operand::Const(IrConst::I64(2654435761)),
            IrType::U32,
            &assignments,
            &mut out,
        );
        assert!(
            out.iter().any(|inst| matches!(
                inst,
                MachInst::Imul3 {
                    imm: -1640531535,
                    size: OpSize::S32,
                    ..
                }
            )),
            "{out:?}"
        );
        assert!(!out.iter().any(|inst| matches!(
            inst,
            MachInst::Mov {
                src: MachOperand::Imm(2654435761),
                ..
            }
        )));
    }

    #[test]
    fn cmp_against_zero_lowers_to_test() {
        let mut assignments = FxHashMap::default();
        assignments.insert(1, PhysReg(14));
        assignments.insert(2, PhysReg(1));
        let mut out = Vec::new();
        lower_cmp(
            &Value(2),
            IrCmpOp::Ne,
            &Operand::Value(Value(1)),
            &Operand::Const(IrConst::I64(0)),
            IrType::I32,
            &assignments,
            &mut out,
        );
        assert!(
            matches!(
                out.first(),
                Some(MachInst::Test {
                    size: OpSize::S32,
                    ..
                })
            ),
            "{out:?}"
        );
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
