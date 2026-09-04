//! Instruction selection: lower IR instructions to MachInst with virtual registers.
//!
//! Each IR instruction type has a lowering function that produces a sequence of
//! MachInst entries. Virtual registers (MachReg::Vreg) represent IR values that
//! will be assigned physical registers by the MachInst register allocator.
//! Pre-colored registers (MachReg::Phys) are used for x86 constraints like
//! division (rax:rdx) and shifts (rcx/%cl).

use super::machinst::*;
use crate::backend::regalloc::PhysReg;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
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
/// allocator's float bank; xmm0/xmm1 can only hold INCOMING float-parameter
/// homes — see `xmm_scratch_unsafe`). Mirrors `is_xmm_reg` in emit.rs.
fn is_xmm_phys(reg: PhysReg) -> bool {
    reg.0 >= 20 && reg.0 <= 33
}

/// True when the pre-colored xmm0/xmm1 scratch registers are unsafe to
/// clobber: float parameters are pre-colored to their INCOMING SysV
/// registers (xmm0 = PhysReg 18, xmm1 = PhysReg 19), below the allocator's
/// xmm pool (20..=33). The allocator itself never assigns 18/19, so any
/// entry there is a live incoming parameter home.
fn xmm_scratch_unsafe(ra: &FxHashMap<u32, PhysReg>) -> bool {
    ra.values().any(|r| r.0 == 18 || r.0 == 19)
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
        if let Some(imm) = const_as_imm32(rhs) {
            emit_mov_operand_r(lhs, dst, size, ra, out);
            let mask = if size == OpSize::S32 { 31 } else { 63 };
            out.push(MachInst::Shift {
                op: shift_op,
                amount: MachOperand::Imm(imm & mask),
                dst,
                size,
            });
        } else if try_lower_shiftx(shift_op, lhs, rhs, dst, size, ra, out) {
            // `shlx count, src, dst`: no %rcx pin, no source copy, 1 µop.
        } else {
            // Two-address hazard (same shape as the ALU case above): when
            // the count's home IS the destination register, `mov lhs,dst`
            // would overwrite the count before it is copied to %rcx.  Copy
            // the count out first in that case; otherwise keep lhs first so
            // a count already sitting in %rcx is not clobbered.
            let count_home_is_dst = matches!(rhs, Operand::Value(v)
                if v.0 != dest.0 && value_to_reg(v, ra) == dst && matches!(dst, MachReg::Phys(_)));
            if count_home_is_dst {
                emit_mov_operand_r(rhs, MachReg::Phys(RCX), size, ra, out);
                emit_mov_operand_r(lhs, dst, size, ra, out);
            } else {
                emit_mov_operand_r(lhs, dst, size, ra, out);
                emit_mov_operand_r(rhs, MachReg::Phys(RCX), size, ra, out);
            }
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
/// Fold an integer `Cast` whose source is a compile-time constant.
///
/// The MachInst layer copy-propagates `Copy dest, Const` into later uses, so a
/// `Cast` can legitimately see an `Operand::Const` source at any width.  The
/// result is the exact 64-bit register image C semantics demand:
///
/// 1. the raw payload is first interpreted at the SOURCE type (sign- or
///    zero-extended from `from_ty`'s width by `from_ty`'s signedness), then
/// 2. truncated to the DESTINATION width and re-extended by `to_ty`'s
///    signedness (a wider reader of the register must see the same value the
///    mature path and GCC/Clang produce).
///
/// Before this fold existed the lowering picked the move width from the
/// destination signedness alone, so `Cast I64->U64` of `0x8000000000000000`
/// was emitted as `movl $0` (gcc.c-torture `20020219-1.c` aborted at -O2),
/// `Cast I32(300)->U8` kept the 0x100 bit and `Cast U32(0xFFFFFFFF)->I64`
/// produced -1.
pub(crate) fn fold_int_cast_const(raw: i64, from_ty: IrType, to_ty: IrType) -> i64 {
    fn extend(v: i64, bits: u32, signed: bool) -> i64 {
        if bits >= 64 {
            return v;
        }
        let shift = 64 - bits;
        if signed {
            (v << shift) >> shift
        } else {
            ((v as u64) << shift >> shift) as i64
        }
    }
    let from_bits = (from_ty.size() as u32).saturating_mul(8).clamp(8, 64);
    let to_bits = (to_ty.size() as u32).saturating_mul(8).clamp(8, 64);
    let mathematical = extend(raw, from_bits, from_ty.is_signed());
    extend(mathematical, to_bits, to_ty.is_signed())
}

/// Materialise an exact 64-bit register image with the cheapest encoding:
/// `movl $imm32` (zero-extends, 5-6 bytes) when the image fits in 32 unsigned
/// bits, `movq $simm32` (sign-extends) or `movabsq` otherwise - decided by the
/// emitter from the value alone.  Never uses a narrow move, so no reader of
/// the destination register can observe stale upper bits.
fn emit_exact_imm64(image: i64, dst: MachReg, out: &mut Vec<MachInst>) {
    let size = if image >= 0 && image <= u32::MAX as i64 {
        OpSize::S32
    } else {
        OpSize::S64
    };
    out.push(MachInst::Mov {
        src: MachOperand::Imm(image),
        dst: MachOperand::Reg(dst),
        size,
    });
}

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

    // Constant source (MachInst const-propagation): fold at compile time
    // with full C conversion semantics and emit one exact move.  This
    // covers narrowing, same-size and widening casts uniformly.
    if let Operand::Const(c) = src {
        if !from_ty.is_float() && !to_ty.is_float() && from_ty.size() <= 8 && to_ty.size() <= 8 {
            let image = fold_int_cast_const(const_to_i64(c), from_ty, to_ty);
            emit_exact_imm64(image, dst, out);
            return;
        }
    }

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
/// How eagerly isel selects the BMI2 three-operand shifts.  Set once per
/// codegen run from `CodegenOptions` (tune row × `-mbmi2`); read by
/// `lower_binop`.  A process-wide cell because isel is a set of pure free
/// functions without a target context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShlxMode {
    /// No BMI2, or an explicit opt-out.
    Never,
    /// Cores where `SHL r,cl` is already 1 µop (Zen): the VEX form only
    /// pays when it removes a copy into %rcx or a copy of the source.
    WhenItSavesAMove,
    /// Cores where `SHL r,cl` is 2–3 µops (every Intel core): always.
    Always,
}

static SHLX_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn set_shlx_mode(m: ShlxMode) {
    SHLX_MODE.store(m as u8, std::sync::atomic::Ordering::Relaxed);
}

pub fn shlx_mode() -> ShlxMode {
    match SHLX_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => ShlxMode::WhenItSavesAMove,
        2 => ShlxMode::Always,
        _ => ShlxMode::Never,
    }
}

/// Try to lower a variable-count shift to `ShiftX`.  Returns false (with
/// `out` untouched) when a required operand has no physical home in a shape
/// the three-operand form can consume without an extra copy that the
/// legacy `Shift` would not also need, or when staging would clobber a
/// source.  Only S32/S64 exist for shlx.
fn try_lower_shiftx(
    shift_op: ShiftOp,
    lhs: &Operand,
    rhs: &Operand,
    dst: MachReg,
    size: OpSize,
    ra: &FxHashMap<u32, PhysReg>,
    out: &mut Vec<MachInst>,
) -> bool {
    let mode = shlx_mode();
    if mode == ShlxMode::Never || !matches!(size, OpSize::S32 | OpSize::S64) {
        return false;
    }
    let MachReg::Phys(dst_phys) = dst else {
        return false;
    };
    let phys_home = |op: &Operand| match op {
        Operand::Value(v) => match value_to_reg(v, ra) {
            MachReg::Phys(p) => Some(p),
            MachReg::Vreg(_) => None,
        },
        Operand::Const(_) => None,
    };
    let count_home = phys_home(rhs);
    let lhs_home = phys_home(lhs);
    let saves_move = count_home.is_some() || lhs_home.is_some();
    if mode == ShlxMode::WhenItSavesAMove && !saves_move {
        return false;
    }
    // Staging through %rcx is impossible when the destination *is* %rcx
    // and the other operand still has to be copied into it.
    let needs_rcx = count_home.is_none() || (lhs_home.is_none() && count_home == Some(dst_phys));
    if needs_rcx && dst_phys == RCX {
        return false;
    }
    let count = if needs_rcx {
        emit_mov_operand_r(rhs, MachReg::Phys(RCX), size, ra, out);
        MachReg::Phys(RCX)
    } else {
        MachReg::Phys(count_home.expect("checked"))
    };
    let src = match lhs_home {
        Some(p) => MachReg::Phys(p),
        None => {
            emit_mov_operand_r(lhs, dst, size, ra, out);
            dst
        }
    };
    out.push(MachInst::ShiftX {
        op: shift_op,
        count,
        src,
        dst,
        size,
    });
    true
}

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
            let name = d
                .split(|c: char| !c.is_ascii_alphanumeric())
                .next()
                .unwrap_or("other");
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

// ── Typed direct calls (MachInst::CallTyped) ─────────────────────────────
//
// The SysV integer-register call contract, lowered as one atomic machine
// instruction. The builder below is pure — it consumes pre-resolved
// argument homes and returns the execution-ordered move plan — so the
// full admission/ordering matrix is unit-testable without codegen state.
// The emit-side gate (try_lower_machinst) computes the homes from the
// allocator + stack-slot state and owns everything the pure builder must
// not see (caller-save intervals, PLT naming, inline-memcpy interception).

/// A pre-resolved argument source for the typed-call builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedCallSrc {
    /// Immediate argument. Any 64-bit value is representable: inside the
    /// sign-extended imm32 window the move is the regular `movq $imm`,
    /// outside it the emitter stages the full constant with
    /// `movabsq $imm, %reg` (inline 64-bit operands are unencodable and
    /// truncating them is a miscompile — the movabs form is both the
    /// shortest and the only sound rendering).
    Imm(i64),
    /// GPR home (never rax/rcx/rbp — the emit-side gate rejects those).
    Reg(PhysReg),
    /// Spill-slot / stack-slot read at the argument's width.
    Slot(i64),
    /// Address of a local alloca (`&x` passed by pointer). Lowered as a
    /// separate `Mov { src: AllocaAddr, dst: Reg(argreg) }` placed before
    /// the call (the resolver renders it as `leaq slot(%rbp), %reg` with
    /// the frame's addressing mode); the argument's own move is elided.
    /// Two guards make the pre-move sound: no other argument may source
    /// from the same ABI register, and the register must not be in the
    /// caller-save set (the pre-move precedes the saves).
    AllocaAddr(u32),
}

/// Why a call was refused the typed path (census label + unit-test oracle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedCallReject {
    /// An argument has no representable home (accumulator, xmm, alloca
    /// address, rax/rcx/rbp home, …) — the mature path stages it.
    ArgNotRepresentable(usize),
    /// The argument moves form a register-exchange cycle (e.g. both
    /// arguments homed in each other's ABI registers). The text path
    /// owns the hazard-spill machinery for this shape.
    MoveCycle,
    /// The return home is not a register or slot home.
    RetNotRepresentable,
}

impl TypedCallReject {
    /// Census label for the rejection reason.
    pub fn label(self) -> &'static str {
        match self {
            TypedCallReject::ArgNotRepresentable(_) => "Call(arg-unrepresentable)",
            TypedCallReject::MoveCycle => "Call(move-cycle)",
            TypedCallReject::RetNotRepresentable => "Call(ret-unrepresentable)",
        }
    }
}

/// The lowering plan for one typed call.
#[derive(Debug, Clone)]
pub struct TypedCallPlan {
    /// Argument moves in execution order (self-moves already elided).
    pub args: Vec<CallArgMove>,
    /// Return-value move, or None for void calls / rax-homed returns.
    pub ret: Option<CallRetMove>,
}

/// Build the argument/return move plan for a typed call.
///
/// `args` holds one entry per IR argument (None = not representable);
/// `arg_sizes` is the parallel ABI move width list; `ret` is the return
/// home (None = void call) as `Some((dst_source, width))` — the dst is
/// `Reg` or `Slot` only, `Imm` is meaningless for a destination.
///
/// Ordering contract: every move whose source is a register R is ordered
/// before every move whose destination is R (Kahn topological order over
/// the ≤6-node dependency graph). A cycle means two arguments live in each
/// other's ABI registers — the text path's hazard-spill area handles that
/// shape; the builder refuses it rather than emitting a swap on a register
/// the ABI does not promise.
pub fn build_typed_call(
    args: &[Option<TypedCallSrc>],
    arg_sizes: &[OpSize],
    ret: Option<(Option<TypedCallSrc>, OpSize)>,
) -> Result<TypedCallPlan, TypedCallReject> {
    build_typed_call_ex(args, arg_sizes, ret, None)
}

/// [`build_typed_call`] with an optional callee-pointer staging move for
/// indirect targets. The staging move joins the same pending list and is
/// ordered by the identical reader-before-writer rule: it reads the
/// callee's home and writes the target register, so any argument move
/// that reads the target register runs before it, and any argument move
/// that writes its source home runs after it. A callee already resident
/// in the target register is a self-move and elides the staging entirely.
pub fn build_typed_call_ex(
    args: &[Option<TypedCallSrc>],
    arg_sizes: &[OpSize],
    ret: Option<(Option<TypedCallSrc>, OpSize)>,
    callee: Option<(TypedCallSrc, PhysReg)>,
) -> Result<TypedCallPlan, TypedCallReject> {
    debug_assert_eq!(args.len(), arg_sizes.len());

    // 1. Resolve sources; elide self-moves (value already in place).
    struct PendingMove {
        src: MachOperand,
        dst_reg: PhysReg,
        size: OpSize,
    }
    let mut pending: Vec<PendingMove> = Vec::with_capacity(args.len() + 1);
    for (i, src) in args.iter().enumerate() {
        let size = arg_sizes[i];
        let dst_reg = SYSCALL_ARG_REGS[i];
        let src = match src {
            None => return Err(TypedCallReject::ArgNotRepresentable(i)),
            Some(TypedCallSrc::Imm(v)) => MachOperand::Imm(*v),
            Some(TypedCallSrc::Reg(r)) => {
                // Defensive re-check of the emit-side gate: rax is the
                // accumulator (not a stable home in this model), rbp is the
                // frame pointer in frame-based functions.
                if r.0 == RAX.0 || r.0 == RBP.0 {
                    return Err(TypedCallReject::ArgNotRepresentable(i));
                }
                if *r == dst_reg {
                    continue; // self-move: value already in place
                }
                MachOperand::Reg(MachReg::Phys(*r))
            }
            Some(TypedCallSrc::Slot(off)) => MachOperand::StackSlot(*off),
            Some(TypedCallSrc::AllocaAddr(id)) => {
                // Guard: no other argument may source from this ABI
                // register — the pre-move writes it before the call, and
                // the builder's topological order cannot interleave with
                // the CallTyped-internal moves.
                let conflicts = args.iter().enumerate().any(|(j, other)| {
                    j != i && matches!(other, Some(TypedCallSrc::Reg(r)) if *r == dst_reg)
                });
                if conflicts {
                    return Err(TypedCallReject::ArgNotRepresentable(i));
                }
                MachOperand::AllocaAddr(*id)
            }
        };
        pending.push(PendingMove { src, dst_reg, size });
    }
    // Callee staging for indirect targets: exactly one extra move with the
    // same edge semantics as an argument move. The destination is r10/r11,
    // which no argument move ever writes (arguments go to ABI registers
    // only), so the only ordering edges are: readers of the target register
    // before the staging, and writers of the staging's source home after
    // the staging — both handled by the shared topological rule below.
    if let Some((callee_src, target_reg)) = callee {
        let src = match callee_src {
            TypedCallSrc::Imm(v) => Some(MachOperand::Imm(v)),
            TypedCallSrc::Reg(r) if r == target_reg => {
                // Self-move: the callee pointer already sits in the
                // target register; no staging move.
                None
            }
            TypedCallSrc::Reg(r) => {
                // A callee homed in an ABI argument register is still
                // stageable: the staging reads it before any argument
                // move writes that register (topological rule).
                Some(MachOperand::Reg(MachReg::Phys(r)))
            }
            TypedCallSrc::Slot(off) => Some(MachOperand::StackSlot(off)),
            TypedCallSrc::AllocaAddr(id) => {
                // A callee pointer that is the address of a local
                // (trampoline buffer). The pre-move reads no register, but
                // it is hoisted before the caller-save spills (same as the
                // argument-side pre-moves), so its destination register
                // must not be read by any argument move — identical guard
                // to the argument-side AllocaAddr.
                let conflicts = args
                    .iter()
                    .any(|other| matches!(other, Some(TypedCallSrc::Reg(r)) if *r == target_reg));
                if conflicts {
                    return Err(TypedCallReject::MoveCycle);
                }
                Some(MachOperand::AllocaAddr(id))
            }
        };
        if let Some(src) = src {
            pending.push(PendingMove {
                src,
                dst_reg: target_reg,
                size: OpSize::S64,
            });
        }
    }

    // 2. Topologically order the moves. Precedence rule: for every register
    //    R, every move that READS R must execute before every move that
    //    WRITES R (the write destroys the home's value). So a move is
    //    executable exactly when no still-unexecuted move READS its
    //    destination register — executing it later would destroy that
    //    reader's source. (A pending *writer* of this move's source does
    //    NOT block it: this move reads first, the writer clobbers after.)
    //    A pass with no progress while unexecuted moves remain is a
    //    register-exchange cycle — refuse it to the text path, which owns
    //    the hazard-spill machinery.
    let mut ordered: Vec<CallArgMove> = Vec::with_capacity(pending.len());
    let mut executed = vec![false; pending.len()];
    while executed.iter().any(|&e| !e) {
        let mut progressed = false;
        for i in 0..pending.len() {
            if executed[i] {
                continue;
            }
            let dst_writes = pending[i].dst_reg;
            let blocked = pending.iter().enumerate().any(|(j, m)| {
                if j == i || executed[j] {
                    return false;
                }
                // AllocaAddr sources read no data register (the address is
                // frame-relative), so only register sources create edges.
                matches!(&m.src, MachOperand::Reg(MachReg::Phys(r)) if *r == dst_writes)
            });
            if !blocked {
                let m = &pending[i];
                ordered.push(CallArgMove {
                    src: m.src.clone(),
                    dst_reg: m.dst_reg,
                    size: m.size,
                });
                executed[i] = true;
                progressed = true;
            }
        }
        if !progressed {
            return Err(TypedCallReject::MoveCycle);
        }
    }

    // 3. Return home.
    let ret_move = match ret {
        None => None,
        Some((src, size)) => match src {
            None | Some(TypedCallSrc::Imm(_)) | Some(TypedCallSrc::AllocaAddr(_)) => {
                // A destination receives the return value; an immediate or
                // an alloca address is not a value home (the emit-side gate
                // rejects alloca destinations before the builder runs).
                return Err(TypedCallReject::RetNotRepresentable);
            }
            Some(TypedCallSrc::Reg(r)) => {
                // rax→rax is a no-op; anything else register-hosted is a
                // legal `mov %rax, %r?` copy — but keep the conservative
                // contract: only real homes (rbp excluded as for args).
                if r.0 == RBP.0 {
                    return Err(TypedCallReject::RetNotRepresentable);
                }
                if r.0 == RAX.0 {
                    None
                } else {
                    Some(CallRetMove {
                        dst: MachOperand::Reg(MachReg::Phys(r)),
                        size,
                    })
                }
            }
            Some(TypedCallSrc::Slot(off)) => Some(CallRetMove {
                dst: MachOperand::StackSlot(off),
                size,
            }),
        },
    };

    Ok(TypedCallPlan {
        args: ordered,
        ret: ret_move,
    })
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
    lower_instruction_typed_ss(inst, reg_assignments, alloca_slots, value_types, None, out)
}

/// Production entry point: additionally receives the certified-small-slot set
/// so copy relays can choose the full-width S64 form for 8-byte homes (see
/// the Copy arm in `lower_instruction_typed_inner`). Tests and the legacy
/// ctx entry go through `lower_instruction_typed` with `None`.
pub fn lower_instruction_typed_ss(
    inst: &Instruction,
    reg_assignments: &FxHashMap<u32, PhysReg>,
    alloca_slots: &FxHashMap<u32, i64>,
    value_types: Option<&FxHashMap<u32, crate::common::types::IrType>>,
    small_slots: Option<&FxHashSet<u32>>,
    out: &mut Vec<MachInst>,
) -> bool {
    let lowered = lower_instruction_typed_inner(
        inst,
        reg_assignments,
        alloca_slots,
        value_types,
        small_slots,
        out,
    );
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
    small_slots: Option<&FxHashSet<u32>>,
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
            volatile,
        } => {
            if ty.is_long_double() {
                return false;
            }
            // ── Typed i128 loads (the Load(other) census class) ──
            //
            // Mirror of the typed i128 store: the value's home is a
            // contiguous 16-byte slot, the source address a direct slot or
            // a GPR-held pointer, and the transfer is the atomic Mov128
            // (movdqu pair through the pre-colored xmm0 scratch,
            // alias-ordered) or the four-Mov GPR fallback through the
            // reserved rax/rdx when the scratch is live.  Segment
            // overrides stay on the mature path.
            if ty.is_128bit() {
                if *seg_override != AddressSpace::Default || *volatile {
                    return false;
                }
                let Some(&dslot) = alloca_slots.get(&dest.0) else {
                    return false;
                };
                if small_slots.is_some_and(|s| s.contains(&dest.0)) {
                    return false;
                }
                if ra.contains_key(&dest.0) {
                    return false;
                }
                let src = if let Some(&slot) = alloca_slots.get(&ptr.0) {
                    if small_slots.is_some_and(|s| s.contains(&ptr.0)) {
                        return false;
                    }
                    MachOperand::StackSlot(slot)
                } else if let Some(&base) = ra.get(&ptr.0) {
                    if is_xmm_phys(base) {
                        return false;
                    }
                    MachOperand::Mem {
                        base: MachReg::Phys(base),
                        offset: 0,
                    }
                } else {
                    return false;
                };
                let dst = MachOperand::StackSlot(dslot);
                if xmm_scratch_unsafe(ra) {
                    let dst_hi = MachOperand::StackSlot(dslot + 8);
                    out.push(MachInst::Mov {
                        src: src.clone(),
                        dst: MachOperand::Reg(MachReg::Phys(RAX)),
                        size: OpSize::S64,
                    });
                    let src_hi = match src {
                        MachOperand::StackSlot(slot) => MachOperand::StackSlot(slot + 8),
                        MachOperand::Mem { base, offset } => MachOperand::Mem {
                            base,
                            offset: offset + 8,
                        },
                        _ => return false,
                    };
                    out.push(MachInst::Mov {
                        src: src_hi,
                        dst: MachOperand::Reg(MachReg::Phys(RDX)),
                        size: OpSize::S64,
                    });
                    out.push(MachInst::Mov {
                        src: MachOperand::Reg(MachReg::Phys(RAX)),
                        dst,
                        size: OpSize::S64,
                    });
                    out.push(MachInst::Mov {
                        src: MachOperand::Reg(MachReg::Phys(RDX)),
                        dst: dst_hi,
                        size: OpSize::S64,
                    });
                    return true;
                }
                out.push(MachInst::Mov128 { src, dst });
                return true;
            }
            // Scalar float load: mirror of the float store subset. The dest
            // must be xmm-homed (direct) or spill-slot-homed (xmm0 scratch
            // relay); the address a direct alloca slot or a GPR-held pointer.
            // Constants and exotic addressing stay on the mature text path.
            if matches!(ty, IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64) {
                if *seg_override != AddressSpace::Default {
                    return false;
                }
                let size = if matches!(ty, IrType::F32 | IrType::D32) {
                    OpSize::S32
                } else {
                    OpSize::S64
                };
                // Destination home: xmm register (direct) or spill slot
                // (store the loaded value from the xmm0 scratch relay).
                let relay_dest: Option<i64> = match ra.get(&dest.0) {
                    Some(&dst_reg) if is_xmm_phys(dst_reg) => None,
                    Some(_) => return false,
                    None => match alloca_slots.get(&dest.0) {
                        Some(&dslot) => Some(dslot),
                        None => return false,
                    },
                };
                // Source address: direct alloca slot or GPR-held pointer.
                let src = if let Some(&slot) = alloca_slots.get(&ptr.0) {
                    MachOperand::StackSlot(slot)
                } else if let Some(&base) = ra.get(&ptr.0) {
                    if is_xmm_phys(base) {
                        return false;
                    }
                    MachOperand::Mem {
                        base: MachReg::Phys(base),
                        offset: 0,
                    }
                } else {
                    return false;
                };
                match relay_dest {
                    None => {
                        let dst =
                            MachOperand::Reg(MachReg::Phys(ra.get(&dest.0).copied().unwrap()));
                        out.push(MachInst::FMov { src, dst, size });
                    }
                    Some(dslot) => {
                        // The relay clobbers xmm0. Float parameters are
                        // pre-colored to their INCOMING register — xmm0/xmm1
                        // are PhysReg 18/19, below the allocator pool — so a
                        // live home there makes the scratch unsafe. Exact
                        // guard: the allocator never assigns 18/19, so an
                        // entry means a live incoming-ABI float param.
                        if xmm_scratch_unsafe(ra) {
                            return false;
                        }
                        // Load into the pre-colored xmm0 scratch, then store
                        // to the slot-homed destination.
                        out.push(MachInst::FMov {
                            src,
                            dst: MachOperand::Reg(MachReg::Phys(XMM0_SCRATCH)),
                            size,
                        });
                        out.push(MachInst::FMov {
                            src: MachOperand::Reg(MachReg::Phys(XMM0_SCRATCH)),
                            dst: MachOperand::StackSlot(dslot),
                            size,
                        });
                    }
                }
                return true;
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
            volatile,
        } => {
            if ty.is_long_double() {
                return false;
            }
            // ── Typed i128 stores (the Store(other) census class) ──
            //
            // The mature text path materializes the value into the
            // accumulator pair, SPILLS the pair (emit_save_acc_pair) and
            // stores both halves from the spill slots — 6-8 instructions
            // plus 16 bytes of dead frame traffic per store, where the
            // oracles use 2 (gcc: pxor+movaps / pool movdqa+movaps / two
            // direct pair stores).  Three typed forms, cheapest first:
            //
            //   * Const: the two 64-bit halves ARE the data — two S64 Movs
            //     at the destination and destination+8 (every u64 half is
            //     either an imm32-window `movq $imm` or takes the emitter's
            //     hardened wide-imm relay, which stores exactly 8 bytes at
            //     the operand's own size).  No staging register, no spill.
            //   * Value (slot-homed, the universal i128 home): the atomic
            //     Mov128 — `movdqu` load + `movdqu` store through the
            //     pre-colored xmm0 scratch; alias-ordered (the source is
            //     read in full before any destination byte is written).
            //   * Scratch-unsafe: the GPR fallback, four S64 Movs through
            //     the reserved rax/rdx with both loads before both stores.
            //
            // Destination subset: a direct (non-small) 16-byte slot or a
            // GPR-held pointer — the same addressing subset every other
            // typed store uses.  `movdqu`, never `movdqa`: frame slots sit
            // at 8 mod 16 and pointer targets may be packed-member aligned
            // (verified: packed i128 member stores run through this path
            // today).  A C `volatile` store keeps the mature path (the
            // implementation-defined volatile access granularity stays the
            // oracle-matching two 8-byte halves); segment overrides and
            // indexed addressing stay there too.
            if ty.is_128bit() {
                if *seg_override != AddressSpace::Default || *volatile {
                    return false;
                }
                let dst = if let Some(&slot) = alloca_slots.get(&ptr.0) {
                    if small_slots.is_some_and(|s| s.contains(&ptr.0)) {
                        return false;
                    }
                    MachOperand::StackSlot(slot)
                } else if let Some(&base) = ra.get(&ptr.0) {
                    if is_xmm_phys(base) {
                        return false;
                    }
                    MachOperand::Mem {
                        base: MachReg::Phys(base),
                        offset: 0,
                    }
                } else {
                    return false;
                };
                let dst_hi = match dst {
                    MachOperand::StackSlot(slot) => MachOperand::StackSlot(slot + 8),
                    MachOperand::Mem { base, offset } => MachOperand::Mem {
                        base,
                        offset: offset + 8,
                    },
                    _ => return false,
                };
                match val {
                    Operand::Const(c) => {
                        let bits: u128 = match c {
                            IrConst::I128(v) => *v as u128,
                            IrConst::Zero => 0,
                            _ => return false,
                        };
                        let lo = bits as u64 as i64;
                        let hi = (bits >> 64) as u64 as i64;
                        out.push(MachInst::Mov {
                            src: MachOperand::Imm(lo),
                            dst: dst.clone(),
                            size: OpSize::S64,
                        });
                        out.push(MachInst::Mov {
                            src: MachOperand::Imm(hi),
                            dst: dst_hi,
                            size: OpSize::S64,
                        });
                        return true;
                    }
                    Operand::Value(val_v) => {
                        // i128 values are slot-homed (the allocator never
                        // assigns them); a register entry means this is not
                        // a slot-homed i128 value.
                        if ra.contains_key(&val_v.0) {
                            return false;
                        }
                        let Some(&vslot) = alloca_slots.get(&val_v.0) else {
                            return false;
                        };
                        if small_slots.is_some_and(|s| s.contains(&val_v.0)) {
                            return false;
                        }
                        let src = MachOperand::StackSlot(vslot);
                        if xmm_scratch_unsafe(ra) {
                            // GPR fallback: load both halves into the
                            // reserved rax/rdx BEFORE either store — the
                            // destination may overlap the source slot.
                            out.push(MachInst::Mov {
                                src,
                                dst: MachOperand::Reg(MachReg::Phys(RAX)),
                                size: OpSize::S64,
                            });
                            out.push(MachInst::Mov {
                                src: MachOperand::StackSlot(vslot + 8),
                                dst: MachOperand::Reg(MachReg::Phys(RDX)),
                                size: OpSize::S64,
                            });
                            out.push(MachInst::Mov {
                                src: MachOperand::Reg(MachReg::Phys(RAX)),
                                dst: dst.clone(),
                                size: OpSize::S64,
                            });
                            out.push(MachInst::Mov {
                                src: MachOperand::Reg(MachReg::Phys(RDX)),
                                dst: dst_hi,
                                size: OpSize::S64,
                            });
                            return true;
                        }
                        out.push(MachInst::Mov128 { src, dst });
                        return true;
                    }
                }
            }
            // Scalar float store (F32/F64 and the D32/D64 bit-carriers).
            //
            // Typed subset, mirroring the SSE branch of emit_store_impl: the
            // value is homed in an xmm allocator register (direct single
            // move) or in a spill slot (two-move relay through the
            // pre-colored xmm0 scratch, which no allocator home can occupy),
            // and the address is a direct alloca slot or a GPR-held pointer.
            // Everything else -- GPR-homed values, xmm-based pointers,
            // indexed/folded GEP addressing, F128/x87 -- keeps the mature
            // emitter. Float CONSTANTS have their own branch below: routing
            // an opaque bit pattern through the xmm domain would be pure
            // overhead.
            if matches!(ty, IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64) {
                if *seg_override != AddressSpace::Default {
                    return false;
                }
                // ── Float-constant stores: the immediate-form frontier ──
                //
                // The value's bit pattern IS the data; there is nothing to
                // compute, so materializing it in the xmm domain (the mature
                // path's xorps / rodata-pool `movss .LCn(%rip), %xmm0` + SSE
                // store) spends a register, a pool entry and a load on
                // nothing. The integer immediate forms are strictly better
                // and strictly sound — a float store is a bit-exact move:
                //
                //   F32/D32:      `movl $bits, mem`          — ONE instruction;
                //                 every 32-bit pattern is a valid imm32
                //                 operand (C7 /0 id sign-extends, the store
                //                 writes the full 32 bits). GCC/Clang/ICX all
                //                 use the 2-instruction pool-load form for
                //                 non-zero constants.
                //   F64/D64 == 0: `movq $0, mem`            — ONE instruction
                //                 (sign-extended imm32 writes all 8 bytes);
                //                 beats the oracle xorps+movsd pair. The
                //                 bit pattern decides, not the float value:
                //                 -0.0 is NOT zero (its high half is set).
                //   F64/D64 fits i32 (as i64, sign-extended):
                //                 `movq $imm32, mem`       — ONE instruction
                //                 for -0.0, small denormals, 0xNNNNNNNN_
                //                 FFFFFFFF patterns.
                //   otherwise:    two `movl $lo / $hi, mem` halves — equal
                //                 instruction count to the oracle pool form
                //                 but no rodata entry, no load, no xmm
                //                 dependency; the two stores cover exactly
                //                 the bytes the one wide store covered.
                //
                // Destination subset is the same as the value path: direct
                // alloca slot or GPR-held pointer (StackSlot(slot+4) and
                // Mem{base, off+4} resolve to the upper half under both the
                // rbp and rsp addressing modes — the offset arithmetic is
                // frame-mode independent).
                if let Operand::Const(c) = val {
                    let (bits64, width32): (u64, bool) = match c {
                        IrConst::F32(v) => (v.to_bits() as u64, true),
                        IrConst::D32(v) => (*v as u64, true),
                        IrConst::F64(v) => (v.to_bits(), false),
                        IrConst::D64(v) => (*v, false),
                        _ => return false,
                    };
                    let dst = if let Some(&slot) = alloca_slots.get(&ptr.0) {
                        MachOperand::StackSlot(slot)
                    } else if let Some(&base) = ra.get(&ptr.0) {
                        if is_xmm_phys(base) {
                            return false;
                        }
                        MachOperand::Mem {
                            base: MachReg::Phys(base),
                            offset: 0,
                        }
                    } else {
                        return false;
                    };
                    if width32 {
                        out.push(MachInst::Mov {
                            src: MachOperand::Imm(bits64 as i32 as i64),
                            dst,
                            size: OpSize::S32,
                        });
                        return true;
                    }
                    if bits64 == 0 {
                        out.push(MachInst::Mov {
                            src: MachOperand::Imm(0),
                            dst,
                            size: OpSize::S64,
                        });
                        return true;
                    }
                    let as_i64 = bits64 as i64;
                    if as_i64 >= i32::MIN as i64 && as_i64 <= i32::MAX as i64 {
                        out.push(MachInst::Mov {
                            src: MachOperand::Imm(as_i64),
                            dst,
                            size: OpSize::S64,
                        });
                        return true;
                    }
                    // A C `volatile` store is ONE abstract access to the
                    // object (C11 5.1.2.3); splitting an 8-byte double into
                    // two 4-byte stores makes a torn intermediate state
                    // observable to a signal handler or a concurrent reader
                    // that the abstract machine forbids. The single-instruction
                    // immediate forms above (movl for F32/D32, movq $0 /
                    // movq $imm32 for F64/D64 that fit) are each a single
                    // access of the object's own width and stay allowed; only
                    // the two-half split is refused for volatile destinations.
                    // The mature path then emits the pool+movsd form (one
                    // store), matching gcc/clang/icc.
                    if *volatile {
                        return false;
                    }
                    // Both halves MUST be spelled as sign-extended imm32
                    // (`as i32`): the C7 /0 id store form takes a 32-bit
                    // operand and writes exactly 32 bits. Spelling the raw
                    // u32 (e.g. hi half 0xE0000000 as 4294443008) would
                    // leave the imm32 window, and the emitter's wide-
                    // immediate relay is a 64-bit store — it would clobber
                    // the 4 bytes after the slot (caught by fp_arith's
                    // NaN-constant store at -O2).
                    let (lo, hi) = (
                        bits64 as u32 as i32 as i64,
                        (bits64 >> 32) as u32 as i32 as i64,
                    );
                    let dst_hi = match dst {
                        MachOperand::StackSlot(slot) => MachOperand::StackSlot(slot + 4),
                        MachOperand::Mem { base, offset } => MachOperand::Mem {
                            base,
                            offset: offset + 4,
                        },
                        _ => return false,
                    };
                    out.push(MachInst::Mov {
                        src: MachOperand::Imm(lo),
                        dst,
                        size: OpSize::S32,
                    });
                    out.push(MachInst::Mov {
                        src: MachOperand::Imm(hi),
                        dst: dst_hi,
                        size: OpSize::S32,
                    });
                    return true;
                }
                let Operand::Value(val_v) = val else {
                    return false;
                };
                let size = if matches!(ty, IrType::F32 | IrType::D32) {
                    OpSize::S32
                } else {
                    OpSize::S64
                };
                // Value home: an xmm allocator register (direct), or a spill
                // slot (relayed through the pre-colored xmm0 scratch — x86
                // has no mem-to-mem SSE move). Anything else stays behind.
                let relay_slot: Option<i64> = match ra.get(&val_v.0) {
                    Some(&src_reg) if is_xmm_phys(src_reg) => None,
                    Some(_) => return false, // GPR-homed float: lowering defect class
                    None => match alloca_slots.get(&val_v.0) {
                        Some(&vslot) => Some(vslot),
                        None => return false,
                    },
                };
                // Destination address: direct alloca slot or GPR-held pointer.
                let dst = if let Some(&slot) = alloca_slots.get(&ptr.0) {
                    MachOperand::StackSlot(slot)
                } else if let Some(&base) = ra.get(&ptr.0) {
                    // x86 addressing has no xmm base; the text path relays
                    // such pointers through rcx -- leave that shape there.
                    if is_xmm_phys(base) {
                        return false;
                    }
                    MachOperand::Mem {
                        base: MachReg::Phys(base),
                        offset: 0,
                    }
                } else {
                    return false;
                };
                match relay_slot {
                    None => {
                        let src =
                            MachOperand::Reg(MachReg::Phys(ra.get(&val_v.0).copied().unwrap()));
                        out.push(MachInst::FMov { src, dst, size });
                    }
                    Some(vslot) => {
                        // The relay clobbers xmm0 — see the load arm's note:
                        // a live incoming-ABI float param homed in xmm0/xmm1
                        // (PhysReg 18/19) makes the scratch unsafe.
                        if xmm_scratch_unsafe(ra) {
                            return false;
                        }
                        // Load slot → xmm0 scratch, then store scratch → dst.
                        // xmm0 is never an allocator home (pool starts at
                        // xmm2), so the pre-colored scratch is conflict-free
                        // unless a float param lives there.
                        out.push(MachInst::FMov {
                            src: MachOperand::StackSlot(vslot),
                            dst: MachOperand::Reg(MachReg::Phys(XMM0_SCRATCH)),
                            size,
                        });
                        out.push(MachInst::FMov {
                            src: MachOperand::Reg(MachReg::Phys(XMM0_SCRATCH)),
                            dst,
                            size,
                        });
                    }
                }
                return true;
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
            //
            // SMALL-SLOT CERTIFICATION GATE (2026-09-02, kernel-boot zstd
            // ZSTD_decodeLiteralsBlock): the type width may only be trusted
            // when the destination value is certified small (4-byte slot).
            // MachInst register scheduling can leave a value with an 8-byte
            // home slot whose later uses reload it at 64-bit (a
            // CondBranch-on-value test re-materialized as
            // `mov (%rsp+off),%rax; test %rax,%rax`); a type-width 32-bit
            // copy relay then stores only the low half of that slot and the
            // 64-bit reload observes stale upper bytes. The classic path is
            // immune because it stores the u32 zero-extended in %rax at full
            // width. So: certified-small destination -> value-type width
            // (≤32-bit); anything else -> full-width S64 relay, whose
            // destination upper half is always defined for 64-bit readers.
            let dest_small = small_slots.map(|ss| ss.contains(&dest.0)).unwrap_or(false);
            let src_small = match src {
                Operand::Value(v) => small_slots.map(|ss| ss.contains(&v.0)).unwrap_or(false),
                Operand::Const(_) => true,
            };
            let narrow = dest_small && src_small;
            let copy_size = if narrow {
                let ty = value_types.and_then(|vt| {
                    let ty = match src {
                        Operand::Value(v) => vt.get(&v.0).copied(),
                        _ => None,
                    };
                    ty.or_else(|| vt.get(&dest.0).copied())
                });
                ty.map(crate::backend::x86::codegen::machinst::OpSize::from_ir_type)
                    .unwrap_or(crate::backend::x86::codegen::machinst::OpSize::S64)
            } else {
                crate::backend::x86::codegen::machinst::OpSize::S64
            };
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
