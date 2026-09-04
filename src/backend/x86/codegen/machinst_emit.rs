//! Emit AT&T x86-64 assembly from allocated MachInst sequences.
//!
//! After register allocation rewrites all MachReg::Vreg to MachReg::Phys,
//! this module pattern-matches each MachInst to produce text assembly.

use super::machinst::*;
use crate::backend::common::AsmOutput;
use crate::backend::regalloc::PhysReg;

/// Public accessor for register name (used by resolve_stack_vregs for AllocaAddr).
pub fn reg_name_pub(r: MachReg) -> &'static str {
    match r {
        MachReg::Phys(p) => reg_name(p),
        // Reaching here means register allocation left a virtual register
        // behind. Emitting a placeholder would hand the assembler
        // `%VREG_UNRESOLVED` and blame it on the wrong component; fail here,
        // where the invariant actually broke.
        MachReg::Vreg(id) => unreachable!("vreg{} reached assembly emission unallocated", id),
    }
}

/// Map a PhysReg to its 64-bit register name.
/// Extends the existing phys_reg_name to also handle rax(0) and rcx(7).
fn reg_name(reg: PhysReg) -> &'static str {
    match reg.0 {
        0 => "rax",
        1 => "rbx",
        2 => "r12",
        3 => "r13",
        4 => "r14",
        5 => "r15",
        6 => "rbp",
        7 => "rcx",
        10 => "r11",
        11 => "r10",
        12 => "r8",
        13 => "r9",
        14 => "rdi",
        15 => "rsi",
        16 => "rdx",
        // XMM registers for F64 allocation
        18 => "xmm0",
        19 => "xmm1",
        20 => "xmm2",
        21 => "xmm3",
        22 => "xmm4",
        23 => "xmm5",
        24 => "xmm6",
        25 => "xmm7",
        // xmm8-xmm15: additional caller-saved float homes (same convention
        // as emit.rs phys_reg_name). Reachable now that FMov lowers float
        // moves through MachInst.
        26 => "xmm8",
        27 => "xmm9",
        28 => "xmm10",
        29 => "xmm11",
        30 => "xmm12",
        31 => "xmm13",
        32 => "xmm14",
        33 => "xmm15",
        // NO SILENT FALLBACK. This arm used to return "rax", so an unexpected
        // register index produced a syntactically valid instruction naming the
        // WRONG register -- a silent miscompile, in the most frequently used
        // of the four size tables (the 32/16/8-bit ones have always trapped
        // here). A compiler crash is strictly preferable to wrong code, and
        // the whole 559-test corpus passes with this arm live, which is the
        // evidence that the fallback was masking nothing.
        _ => unreachable!("invalid machinst register index {}", reg.0),
    }
}

/// Map a PhysReg to its 32-bit sub-register name.
fn reg_name_32(reg: PhysReg) -> &'static str {
    match reg.0 {
        0 => "eax",
        1 => "ebx",
        2 => "r12d",
        3 => "r13d",
        4 => "r14d",
        5 => "r15d",
        6 => "ebp",
        7 => "ecx",
        10 => "r11d",
        11 => "r10d",
        12 => "r8d",
        13 => "r9d",
        14 => "edi",
        15 => "esi",
        16 => "edx",
        _ => unreachable!("invalid machinst register index {}", reg.0),
    }
}

/// Map a PhysReg to its 16-bit sub-register name.
fn reg_name_16(reg: PhysReg) -> &'static str {
    match reg.0 {
        0 => "ax",
        1 => "bx",
        2 => "r12w",
        3 => "r13w",
        4 => "r14w",
        5 => "r15w",
        6 => "bp",
        7 => "cx",
        10 => "r11w",
        11 => "r10w",
        12 => "r8w",
        13 => "r9w",
        14 => "di",
        15 => "si",
        16 => "dx",
        _ => unreachable!("invalid machinst register index {}", reg.0),
    }
}

/// Map a PhysReg to its 8-bit sub-register name.
fn reg_name_8(reg: PhysReg) -> &'static str {
    match reg.0 {
        0 => "al",
        1 => "bl",
        2 => "r12b",
        3 => "r13b",
        4 => "r14b",
        5 => "r15b",
        6 => "bpl",
        7 => "cl",
        10 => "r11b",
        11 => "r10b",
        12 => "r8b",
        13 => "r9b",
        14 => "dil",
        15 => "sil",
        16 => "dl",
        _ => unreachable!("invalid machinst register index {}", reg.0),
    }
}

/// Get the register name at a given operand size.
fn sized_reg_name(reg: PhysReg, size: OpSize) -> &'static str {
    match size {
        OpSize::S8 => reg_name_8(reg),
        OpSize::S16 => reg_name_16(reg),
        OpSize::S32 => reg_name_32(reg),
        OpSize::S64 => reg_name(reg),
    }
}

/// Format a MachReg as an AT&T register operand (%name).
fn fmt_reg(reg: &MachReg, size: OpSize) -> String {
    match reg {
        MachReg::Phys(r) => format!("%{}", sized_reg_name(*r, size)),
        MachReg::Vreg(id) => format!("%vreg{}", id), // shouldn't appear after allocation
    }
}

/// Format a MachOperand as AT&T assembly.
fn fmt_operand(op: &MachOperand, size: OpSize, out: &AsmOutput) -> String {
    match op {
        MachOperand::Reg(r) => fmt_reg(r, size),
        MachOperand::Imm(v) => format!("${}", v),
        MachOperand::Mem { base, offset } => {
            let base_name = fmt_reg(base, OpSize::S64);
            if *offset == 0 {
                format!("({})", base_name)
            } else {
                format!("{}({})", offset, base_name)
            }
        }
        MachOperand::MemIndex {
            base,
            index,
            scale,
            offset,
        } => {
            let base_name = fmt_reg(base, OpSize::S64);
            let index_name = fmt_reg(index, OpSize::S64);
            if *offset == 0 {
                format!("({}, {}, {})", base_name, index_name, scale)
            } else {
                format!("{}({}, {}, {})", offset, base_name, index_name, scale)
            }
        }
        MachOperand::StackSlot(slot_offset) => {
            if out.use_rsp_addressing {
                let rsp_off = out.rsp_frame_size + slot_offset;
                format!("{}(%rsp)", rsp_off)
            } else {
                format!("{}(%rbp)", slot_offset)
            }
        }
        MachOperand::RipRel(sym) => format!("{}(%rip)", sym),
        MachOperand::AllocaAddr(_) => {
            // AllocaAddr should have been resolved before emission.
            // If we reach here, emit a placeholder that will cause an assembler error.
            "%ALLOCA_ADDR_UNRESOLVED".to_string()
        }
    }
}

/// If `op` is an immediate outside the signed-32-bit range, materialize it
/// into %rax with movabsq and return a register operand (plus the emitted
/// movabsq line). x86 `cmp/test` (like all ALU ops) only support imm32
/// sign-extended operands; using the raw 64-bit immediate would truncate it
/// and miscompare (regression: simd_movnt's `lo == 0x1122334455667788ULL`
/// check compiled to `cmp $0x55667788`).
/// x86-64 ALU and compare immediates are sign-extended 32-bit. A wider
/// constant has to be loaded into a register first; emitting it inline is not
/// a silent truncation but a hard assembler error ("operand type mismatch for
/// `add'"), so any path that forgets this breaks the build.
///
/// The scratch is `%rax`, matching the accumulator convention the rest of this
/// backend uses. That is only sound while no *other* operand of the same
/// instruction already lives in `%rax` -- see [`assert_scratch_free`], which
/// makes the assumption checkable instead of implicit.
fn materialize_large_imm(op: &MachOperand, out: &mut AsmOutput) -> MachOperand {
    match op {
        MachOperand::Imm(v) if *v < i32::MIN as i64 || *v > i32::MAX as i64 => {
            out.emit_fmt(format_args!("    movabsq ${}, %rax", v));
            MachOperand::Reg(MachReg::Phys(super::machinst::RAX))
        }
        _ => op.clone(),
    }
}

/// True when `op` needs [`materialize_large_imm`].
fn needs_scratch(op: &MachOperand) -> bool {
    matches!(op, MachOperand::Imm(v) if *v < i32::MIN as i64 || *v > i32::MAX as i64)
}

/// Guard the `%rax`-as-scratch assumption in [`materialize_large_imm`].
///
/// If a large immediate has to be staged through `%rax` while another operand
/// of the same instruction already IS `%rax`, staging overwrites it and the
/// instruction computes the wrong thing -- silently, because the result still
/// assembles. No current lowering produces that shape (the codegen materializes
/// wide constants long before this point), so rather than emit a heavier
/// save/restore sequence for a path that never runs, make the invariant
/// explicit and let a debug build fail loudly the day it stops holding.
#[inline]
fn assert_scratch_free(imm: &MachOperand, other: &MachReg) {
    debug_assert!(
        !(needs_scratch(imm) && *other == MachReg::Phys(super::machinst::RAX)),
        "a wide immediate must be staged through %rax, but %rax is already an \
         operand of this instruction; the staging would clobber it"
    );
}

/// Map an XMM-allocated PhysReg to its scalar SSE register name.
///
/// FMov register operands live in the XMM domain by construction (the
/// lowering only routes values whose allocator home is xmm2..xmm15). A GPR
/// or unallocated register here is a lowering defect, not an input to paper
/// over: name it loudly rather than emit an instruction that assembles with
/// the wrong operand class.
fn freg_name(reg: PhysReg) -> &'static str {
    match reg.0 {
        18 => "xmm0",
        19 => "xmm1",
        20 => "xmm2",
        21 => "xmm3",
        22 => "xmm4",
        23 => "xmm5",
        24 => "xmm6",
        25 => "xmm7",
        26 => "xmm8",
        27 => "xmm9",
        28 => "xmm10",
        29 => "xmm11",
        30 => "xmm12",
        31 => "xmm13",
        32 => "xmm14",
        33 => "xmm15",
        _ => unreachable!("invalid machinst XMM register index {}", reg.0),
    }
}

/// ALU operation mnemonic.
fn alu_mnemonic(op: AluOp) -> &'static str {
    match op {
        AluOp::Add => "add",
        AluOp::Sub => "sub",
        AluOp::And => "and",
        AluOp::Or => "or",
        AluOp::Xor => "xor",
        AluOp::Imul => "imul",
    }
}

/// Shift operation mnemonic.
/// AT&T shift mnemonic for every operand width.
///
/// The table used to read `(Shl, S32) => "shll", (Shl, _) => "shlq"`, which
/// silently folded S8 and S16 into the 64-bit mnemonic. A byte shift then
/// emitted `shlq %dl` and a word shift `shlq %cx` -- both rejected outright by
/// the assembler ("`%dl' not allowed with `shlq'"). It had not fired only
/// because instruction selection did not yet route narrow shifts through
/// MachInst; the moment it did, the build would break. Found by the
/// assembler-differential test, which is exactly the class of defect a golden
/// test cannot catch, because the author would have written the same wrong
/// expectation.
fn shift_mnemonic(op: ShiftOp, size: OpSize) -> &'static str {
    match (op, size) {
        (ShiftOp::Shl, OpSize::S8) => "shlb",
        (ShiftOp::Shl, OpSize::S16) => "shlw",
        (ShiftOp::Shl, OpSize::S32) => "shll",
        (ShiftOp::Shl, OpSize::S64) => "shlq",
        (ShiftOp::Shr, OpSize::S8) => "shrb",
        (ShiftOp::Shr, OpSize::S16) => "shrw",
        (ShiftOp::Shr, OpSize::S32) => "shrl",
        (ShiftOp::Shr, OpSize::S64) => "shrq",
        (ShiftOp::Sar, OpSize::S8) => "sarb",
        (ShiftOp::Sar, OpSize::S16) => "sarw",
        (ShiftOp::Sar, OpSize::S32) => "sarl",
        (ShiftOp::Sar, OpSize::S64) => "sarq",
    }
}

/// Condition code suffix for Jcc/SetCC/CMov.
fn cc_suffix(cc: CondCode) -> &'static str {
    match cc {
        CondCode::E => "e",
        CondCode::Ne => "ne",
        CondCode::L => "l",
        CondCode::Le => "le",
        CondCode::G => "g",
        CondCode::Ge => "ge",
        CondCode::B => "b",
        CondCode::Be => "be",
        CondCode::A => "a",
        CondCode::Ae => "ae",
    }
}

/// Emit a single allocated MachInst as AT&T assembly text.
pub fn emit_machinst(inst: &MachInst, out: &mut AsmOutput) {
    match inst {
        MachInst::Mov { src, dst, size } => {
            // Skip self-moves (same register or same stack slot)
            // AllocaAddr → Reg: emit leaq instead of mov
            if let MachOperand::AllocaAddr(id) = src {
                if let MachOperand::Reg(r) = dst {
                    // This should have been resolved to a StackSlot-based leaq.
                    // If we reach here, the alloca wasn't resolved — emit placeholder.
                    out.emit_fmt(format_args!("    # ERROR: unresolved AllocaAddr({})", id));
                    return;
                }
            }
            match (src, dst) {
                (MachOperand::Reg(a), MachOperand::Reg(b)) if a == b => return,
                (MachOperand::StackSlot(a), MachOperand::StackSlot(b)) if a == b => return,
                // x86 can't do mem-to-mem moves. Use rax as relay. The relay
                // register must be named at the INSTRUCTION's operand size:
                // `movb …, %rax` / `movw …, %rax` are unencodable (mnemonic
                // size suffix must match the register operand width) — narrow
                // sizes reach this relay for 8/16-bit loads/stores, so name
                // the sub-register (%al/%ax) via the canonical size table.
                (MachOperand::StackSlot(_), MachOperand::StackSlot(_)) => {
                    let src_str = fmt_operand(src, *size, out);
                    let dst_str = fmt_operand(dst, *size, out);
                    let suffix = size.suffix();
                    let rax = sized_reg_name(RAX, *size);
                    out.emit_fmt(format_args!("    mov{} {}, %{}", suffix, src_str, rax));
                    out.emit_fmt(format_args!("    mov{} %{}, {}", suffix, rax, dst_str));
                    return;
                }
                (MachOperand::Mem { .. }, MachOperand::Mem { .. })
                | (MachOperand::Mem { .. }, MachOperand::StackSlot(_))
                | (MachOperand::StackSlot(_), MachOperand::Mem { .. }) => {
                    // Also mem-to-mem: use rax relay (size-correct name, see above)
                    let src_str = fmt_operand(src, *size, out);
                    let dst_str = fmt_operand(dst, *size, out);
                    let suffix = size.suffix();
                    let rax = sized_reg_name(RAX, *size);
                    out.emit_fmt(format_args!("    mov{} {}, %{}", suffix, src_str, rax));
                    out.emit_fmt(format_args!("    mov{} %{}, {}", suffix, rax, dst_str));
                    return;
                }
                _ => {}
            }
            let suffix = size.suffix();
            let src_str = fmt_operand(src, *size, out);
            let dst_str = fmt_operand(dst, *size, out);
            // Special case: movq $0 → xorl (shorter encoding)
            if let MachOperand::Imm(0) = src {
                if let MachOperand::Reg(MachReg::Phys(r)) = dst {
                    let r32 = reg_name_32(*r);
                    out.emit_fmt(format_args!("    xorl %{}, %{}", r32, r32));
                    return;
                }
            }
            // WIDTH CONTRACT (2026-09-03, VLA fill-store corruption; found
            // independently here and upstream as PR #363): a store whose
            // immediate lies outside the sign-extended imm32 window must
            // still write exactly `size` bytes. The IR that reaches here is
            // e.g. `Store { val: Const(I64(3041712678)), ty: U32 }` (an
            // unsigned 32-bit value held in an i64 constant); the store
            // semantics are "write the low `size` bits". Emitting `movq %rax`
            // for such a store writes 8 bytes, so the last element of a
            // 4-byte-strided VLA overran its allocation and clobbered the
            // adjacent frame slot (a[0]'s low word in the o2_vla_fill.c repro;
            // the saved VLA base pointer → SIGSEGV in the minimal case), or —
            // on the float side — an S32 hi-half store with the sign bit set
            // (0xE0000000) clobbered the 4 bytes after its destination.
            //
            // Two layered defenses, cheapest first:
            //   * NARROW FAST PATH (S8/S16/S32): mov{b,w,l} immediate fields
            //     are RAW {8,16,32}-bit values — they encode the full unsigned
            //     range, unlike the sign-extended imm32 form of the ALU ops —
            //     so the constant truncates to the destination width and a
            //     single sized move stores it directly, with no %rax staging:
            //         movl $3041712678, (%rax)      (mem / stack-slot dest)
            //         movl $3041712678, %eax        (reg dest: movl
            //                                        zero-extends into the
            //                                        full register)
            //     This is one instruction and touches no temporary register,
            //     where the relay below needs two instructions and clobbers
            //     %rax.  (Upstream PR #363 fixed the same defect with the
            //     relay alone; the direct form is strictly shorter.)
            //   * WIDE RELAY (S64 >i32, and any residual path): a 64-bit
            //     destination cannot take a direct imm, so stage through rax
            //     and store at the operand's OWN width (`sized_reg_name`) —
            //     never the 8-byte default. This is the emitter defining the
            //     behavior instead of trusting typed IR never to produce a
            //     wide store, and stays as defense in depth beneath the fast
            //     path.
            if let MachOperand::Imm(v) = src {
                if *v < i32::MIN as i64 || *v > i32::MAX as i64 {
                    if matches!(size, OpSize::S8 | OpSize::S16 | OpSize::S32) {
                        // Truncate to the destination width and emit the
                        // single sized move (raw imm field encodes all bits).
                        let mask: i64 = match size {
                            OpSize::S8 => 0xff,
                            OpSize::S16 => 0xffff,
                            _ => 0xffff_ffff,
                        };
                        let t = *v & mask;
                        match dst {
                            // A register dest takes the sized immediate
                            // directly: movl zero-extends the result into the
                            // full register (imm32 is a raw field, not the
                            // sign-extended ALU form), so the value is exact
                            // and the instruction count stays at one.
                            MachOperand::Reg(MachReg::Phys(r)) => {
                                let reg = sized_reg_name(*r, *size);
                                out.emit_fmt(format_args!("    mov{} ${}, %{}", suffix, t, reg));
                            }
                            _ => {
                                out.emit_fmt(format_args!("    mov{} ${}, {}", suffix, t, dst_str));
                            }
                        }
                        return;
                    }
                    if let MachOperand::Reg(MachReg::Phys(r)) = dst {
                        out.emit_fmt(format_args!("    movabsq ${}, %{}", v, reg_name(*r)));
                    } else {
                        out.emit_fmt(format_args!("    movabsq ${}, %rax", v));
                        let rax = sized_reg_name(RAX, *size);
                        out.emit_fmt(format_args!("    mov{} %{}, {}", suffix, rax, dst_str));
                    }
                    return;
                }
            }
            out.emit_fmt(format_args!("    mov{} {}, {}", suffix, src_str, dst_str));
        }

        MachInst::FMov { src, dst, size } => {
            let mnem = match size {
                OpSize::S32 => "movss",
                OpSize::S64 => "movsd",
                // 8/16-bit scalar SSE moves do not exist; the lowering only
                // produces F32/F64 (and the D32/D64 bit-carriers).
                _ => unreachable!("FMov with non-float size {size:?}"),
            };
            // Register operands format at their XMM name regardless of the
            // size suffix; memory operands reuse the S64 GPR formatting.
            let fmt = |op: &MachOperand| -> String {
                match op {
                    MachOperand::Reg(MachReg::Phys(r)) => format!("%{}", freg_name(*r)),
                    // A Vreg reaching emission means allocation was skipped
                    // (reg_name_pub traps on the same invariant).
                    MachOperand::Reg(MachReg::Vreg(id)) => {
                        unreachable!("vreg{id} reached FMov emission unallocated")
                    }
                    // Mem/Slot/RipRel bases are GPRs named at 64-bit width.
                    other => fmt_operand(other, OpSize::S64, out),
                }
            };
            match (src, dst) {
                // Self-move: nothing to do (same xmm home).
                (MachOperand::Reg(a), MachOperand::Reg(b)) if a == b => return,
                // Register-to-register copies use the VEX 3-operand form,
                // exactly like the text path's load_fp_to_reg: the legacy
                // 2-operand `movsd %src, %dst` is a MERGING move that reads
                // the destination's upper lane, creating a false dependence
                // on whatever last wrote it (nbody's sqrt loop serialised
                // 3.1x vs GCC from this before the text path switched).
                (MachOperand::Reg(_), MachOperand::Reg(_)) => {
                    let src_str = fmt(src);
                    let dst_str = fmt(dst);
                    out.emit_fmt(format_args!("    v{mnem} {src_str}, {src_str}, {dst_str}"));
                    return;
                }
                // x86 has no mem-to-mem SSE move; the lowering gate refuses
                // those shapes (the relay needs an xmm scratch the text path
                // owns). Trap rather than emit a broken relay through a
                // possibly-live register.
                (
                    MachOperand::Mem { .. } | MachOperand::StackSlot(_),
                    MachOperand::Mem { .. } | MachOperand::StackSlot(_),
                ) => {
                    unreachable!("FMov mem-to-mem is unencodable; the lowering must reject it")
                }
                // Immediates have no scalar-SSE form (constants materialize
                // through .rodata on the text path).
                (MachOperand::Imm(v), _) => {
                    unreachable!("FMov from immediate {v} is unencodable")
                }
                _ => {}
            }
            let src_str = fmt(src);
            let dst_str = fmt(dst);
            out.emit_fmt(format_args!("    {mnem} {src_str}, {dst_str}"));
        }

        MachInst::FAlu {
            op,
            src2,
            src1,
            dst,
            size,
        } => {
            let base = match op {
                FAluOp::Add => "vadd",
                FAluOp::Sub => "vsub",
                FAluOp::Mul => "vmul",
                FAluOp::Div => "vdiv",
            };
            let mnem = match size {
                OpSize::S32 => format!("{base}ss"),
                OpSize::S64 => format!("{base}sd"),
                _ => unreachable!("FAlu with non-float size {size:?}"),
            };
            let src2_str = match src2 {
                MachOperand::Reg(MachReg::Phys(r)) => format!("%{}", freg_name(*r)),
                MachOperand::Reg(MachReg::Vreg(id)) => {
                    unreachable!("vreg{id} reached FAlu emission unallocated")
                }
                MachOperand::Imm(v) => {
                    unreachable!("FAlu from immediate {v} is unencodable; constants materialize via .rodata on the text path")
                }
                other => fmt_operand(other, OpSize::S64, out),
            };
            let src1_str = match src1 {
                MachReg::Phys(r) => freg_name(*r),
                MachReg::Vreg(id) => {
                    unreachable!("vreg{id} reached FAlu emission unallocated (src1)")
                }
            };
            let dst_str = match dst {
                MachReg::Phys(r) => freg_name(*r),
                MachReg::Vreg(id) => {
                    unreachable!("vreg{id} reached FAlu emission unallocated (dst)")
                }
            };
            out.emit_fmt(format_args!(
                "    {mnem} {src2_str}, %{src1_str}, %{dst_str}"
            ));
        }

        MachInst::Alu { op, src, dst, size } => {
            let mnem = alu_mnemonic(*op);
            // THERE IS NO TWO-OPERAND 8-BIT `imul`. x86 offers only the
            // one-operand `imul r/m8` (AX = AL * r/m8); `imulb %al, %bl` is
            // rejected outright ("invalid instruction suffix for `imul'").
            //
            // The 32-bit form computes the identical low 8 bits, which is all
            // an S8 multiply is defined to produce, and is what every other
            // compiler emits for `char * char`. Widening the operands is
            // therefore exact for the bits that matter; it additionally writes
            // bits 8..31, which are dead for a value the IR typed as I8.
            //
            // Found by the randomized MachInst stress test -- an (op, width)
            // pair no hand-written case had instantiated.
            let size = &if matches!(op, AluOp::Imul) && matches!(size, OpSize::S8) {
                OpSize::S32
            } else {
                *size
            };
            let suffix = size.suffix();
            // A wide immediate cannot be an ALU operand; stage it first.
            assert_scratch_free(src, dst);
            let src = materialize_large_imm(src, out);
            let src_str = fmt_operand(&src, *size, out);
            let dst_str = fmt_reg(dst, *size);
            out.emit_fmt(format_args!(
                "    {}{} {}, {}",
                mnem, suffix, src_str, dst_str
            ));
        }

        MachInst::Imul3 {
            imm,
            src,
            dst,
            size,
        } => {
            let suffix = size.suffix();
            // `imul $imm, src, dst` takes a sign-extended imm32 like the ALU
            // forms. A wider multiplier must go through a register, and there
            // is no three-operand register form -- fall back to the
            // two-address sequence.
            if *imm < i32::MIN as i64 || *imm > i32::MAX as i64 {
                assert_scratch_free(&MachOperand::Imm(*imm), dst);
                let staged = materialize_large_imm(&MachOperand::Imm(*imm), out);
                let src_str = fmt_reg(src, *size);
                let dst_str = fmt_reg(dst, *size);
                if fmt_operand(&staged, *size, out) != dst_str {
                    out.emit_fmt(format_args!("    mov{} {}, {}", suffix, src_str, dst_str));
                }
                out.emit_fmt(format_args!(
                    "    imul{} {}, {}",
                    suffix,
                    fmt_operand(&staged, *size, out),
                    dst_str
                ));
                return;
            }
            let src_str = fmt_reg(src, *size);
            let dst_str = fmt_reg(dst, *size);
            out.emit_fmt(format_args!(
                "    imul{} ${}, {}, {}",
                suffix, imm, src_str, dst_str
            ));
        }

        MachInst::Neg { dst, size } => {
            let suffix = size.suffix();
            let dst_str = fmt_reg(dst, *size);
            out.emit_fmt(format_args!("    neg{} {}", suffix, dst_str));
        }

        MachInst::Not { dst, size } => {
            let suffix = size.suffix();
            let dst_str = fmt_reg(dst, *size);
            out.emit_fmt(format_args!("    not{} {}", suffix, dst_str));
        }

        MachInst::Shift {
            op,
            amount,
            dst,
            size,
        } => {
            let mnem = shift_mnemonic(*op, *size);
            let dst_str = fmt_reg(dst, *size);
            match amount {
                MachOperand::Imm(v) => {
                    out.emit_fmt(format_args!("    {} ${}, {}", mnem, v, dst_str));
                }
                _ => {
                    // Variable shift: amount must be in %cl
                    out.emit_fmt(format_args!("    {} %cl, {}", mnem, dst_str));
                }
            }
        }

        MachInst::ShiftX {
            op,
            count,
            src,
            dst,
            size,
        } => {
            let mnem = match (op, size) {
                (ShiftOp::Shl, OpSize::S64) => "shlxq",
                (ShiftOp::Shr, OpSize::S64) => "shrxq",
                (ShiftOp::Sar, OpSize::S64) => "sarxq",
                (ShiftOp::Shl, _) => "shlxl",
                (ShiftOp::Shr, _) => "shrxl",
                (ShiftOp::Sar, _) => "sarxl",
            };
            // shlx has no 8/16-bit form; isel only selects S32/S64.
            let sz = if *size == OpSize::S64 { OpSize::S64 } else { OpSize::S32 };
            out.emit_fmt(format_args!(
                "    {} {}, {}, {}",
                mnem,
                fmt_reg(count, sz),
                fmt_reg(src, sz),
                fmt_reg(dst, sz)
            ));
        }

        MachInst::Lea {
            base,
            index,
            offset,
            dst,
        } => {
            let base_str = fmt_reg(base, OpSize::S64);
            let dst_str = fmt_reg(dst, OpSize::S64);
            if let Some((idx, scale)) = index {
                let idx_str = fmt_reg(idx, OpSize::S64);
                if *offset == 0 {
                    out.emit_fmt(format_args!(
                        "    leaq ({}, {}, {}), {}",
                        base_str, idx_str, scale, dst_str
                    ));
                } else {
                    out.emit_fmt(format_args!(
                        "    leaq {}({}, {}, {}), {}",
                        offset, base_str, idx_str, scale, dst_str
                    ));
                }
            } else if *offset == 0 {
                // lea (%base), %dst — just a mov
                out.emit_fmt(format_args!("    movq {}, {}", base_str, dst_str));
            } else {
                out.emit_fmt(format_args!(
                    "    leaq {}({}), {}",
                    offset, base_str, dst_str
                ));
            }
        }

        MachInst::Cqto { size } => {
            if *size == OpSize::S32 {
                out.emit("    cltd");
            } else {
                out.emit("    cqto");
            }
        }

        MachInst::XorRdx => {
            out.emit("    xorl %edx, %edx");
        }

        MachInst::Div {
            divisor,
            signed,
            size,
        } => {
            let mnem = if *signed { "idiv" } else { "div" };
            let suffix = size.suffix();
            let div_str = fmt_operand(divisor, *size, out);
            out.emit_fmt(format_args!("    {}{} {}", mnem, suffix, div_str));
        }

        MachInst::Cmp { lhs, rhs, size } => {
            let suffix = size.suffix();
            let mut lhs = lhs.clone();
            let mut rhs = rhs.clone();
            // 64-bit immediates don't fit in the imm32 of cmp; materialize.
            if let MachOperand::Reg(r) = &rhs {
                assert_scratch_free(&lhs, r);
            }
            if let MachOperand::Reg(r) = &lhs {
                assert_scratch_free(&rhs, r);
            }
            lhs = materialize_large_imm(&lhs, out);
            rhs = materialize_large_imm(&rhs, out);
            // x86 cmp can't have two memory operands — load rhs to rax
            let both_mem = matches!(
                (&lhs, &rhs),
                (MachOperand::StackSlot(_), MachOperand::StackSlot(_))
                    | (MachOperand::Mem { .. }, MachOperand::Mem { .. })
                    | (MachOperand::StackSlot(_), MachOperand::Mem { .. })
                    | (MachOperand::Mem { .. }, MachOperand::StackSlot(_))
            );
            if both_mem {
                let rhs_str = fmt_operand(&rhs, *size, out);
                // Relay register named at operand size (%al/%ax/%eax/%rax):
                // a `movb …, %rax` relay load is unencodable.
                let rax = sized_reg_name(RAX, *size);
                out.emit_fmt(format_args!("    mov{} {}, %{}", suffix, rhs_str, rax));
                let lhs_str = fmt_operand(&lhs, *size, out);
                // AT&T: cmp rhs, lhs
                out.emit_fmt(format_args!("    cmp{} %{}, {}", suffix, rax, lhs_str));
            } else {
                let rhs_str = fmt_operand(&rhs, *size, out);
                let lhs_str = fmt_operand(&lhs, *size, out);
                out.emit_fmt(format_args!("    cmp{} {}, {}", suffix, rhs_str, lhs_str));
            }
        }

        MachInst::Test { lhs, rhs, size } => {
            let suffix = size.suffix();
            let mut lhs = lhs.clone();
            let mut rhs = rhs.clone();
            if let MachOperand::Reg(r) = &rhs {
                assert_scratch_free(&lhs, r);
            }
            if let MachOperand::Reg(r) = &lhs {
                assert_scratch_free(&rhs, r);
            }
            lhs = materialize_large_imm(&lhs, out);
            rhs = materialize_large_imm(&rhs, out);
            // x86 test can't have two memory operands — load one to rax
            let both_mem = matches!(
                (&lhs, &rhs),
                (MachOperand::StackSlot(_), MachOperand::StackSlot(_))
                    | (MachOperand::Mem { .. }, MachOperand::Mem { .. })
                    | (MachOperand::StackSlot(_), MachOperand::Mem { .. })
                    | (MachOperand::Mem { .. }, MachOperand::StackSlot(_))
            );
            if both_mem {
                let rhs_str = fmt_operand(&rhs, *size, out);
                // Relay register named at operand size (%al/%ax/%eax/%rax):
                // a `movb …, %rax` relay load is unencodable.
                let rax = sized_reg_name(RAX, *size);
                out.emit_fmt(format_args!("    mov{} {}, %{}", suffix, rhs_str, rax));
                let lhs_str = fmt_operand(&lhs, *size, out);
                out.emit_fmt(format_args!("    test{} %{}, {}", suffix, rax, lhs_str));
            } else {
                let rhs_str = fmt_operand(&rhs, *size, out);
                let lhs_str = fmt_operand(&lhs, *size, out);
                out.emit_fmt(format_args!("    test{} {}, {}", suffix, rhs_str, lhs_str));
            }
        }

        MachInst::SetCC { cc, dst } => {
            let cc_str = cc_suffix(*cc);
            let dst_str = fmt_reg(dst, OpSize::S8);
            out.emit_fmt(format_args!("    set{} {}", cc_str, dst_str));
        }

        MachInst::Mov128 { src, dst } => {
            // The atomic 16-byte transfer: load the whole source region into
            // the pre-colored xmm0 scratch, then store it to the destination.
            // The pair is data-dependent, so overlapping source/destination
            // regions stay correct. The isel refuses this instruction when
            // the scratch is live, so the clobber is conflict-free here.
            let src_str = fmt_operand(src, OpSize::S64, out);
            let dst_str = fmt_operand(dst, OpSize::S64, out);
            let scratch = freg_name(XMM0_SCRATCH);
            out.emit_fmt(format_args!("    movdqu {}, %{}", src_str, scratch));
            out.emit_fmt(format_args!("    movdqu %{}, {}", scratch, dst_str));
        }
        MachInst::Movzx {
            src,
            dst,
            from_size,
            to_size,
        } => {
            let src_str = fmt_operand(src, *from_size, out);
            // movzbl/movzwl always use 32-bit dest (implicit zero-extend to 64-bit).
            // A 32->64 zero-extension is a plain movl: writing a 32-bit
            // register zero-extends to the full 64-bit register on x86-64.
            // (The previous `_ => movzbl` fallback TRUNCATED 32-bit values to
            // 8 bits — miscompiling every U32->U64 zero-extending cast, e.g.
            // gzip's send_bits bit packing: wrong values -> corrupt output.)
            let mnem = match (from_size, to_size) {
                (OpSize::S8, _) => "movzbl",
                (OpSize::S16, _) => "movzwl",
                _ => "movl",
            };
            let actual_dst_size = if *to_size == OpSize::S64 {
                OpSize::S32
            } else {
                *to_size
            };
            let dst_str = fmt_reg(dst, actual_dst_size);
            out.emit_fmt(format_args!("    {} {}, {}", mnem, src_str, dst_str));
        }

        MachInst::Movsx {
            src,
            dst,
            from_size,
            to_size,
        } => {
            let src_str = fmt_operand(src, *from_size, out);
            let dst_str = fmt_reg(dst, *to_size);
            let mnem = match (from_size, to_size) {
                (OpSize::S8, OpSize::S32) => "movsbl",
                (OpSize::S16, OpSize::S32) => "movswl",
                (OpSize::S8, OpSize::S64) => "movsbq",
                (OpSize::S16, OpSize::S64) => "movswq",
                (OpSize::S32, OpSize::S64) => "movslq",
                _ => "movslq", // fallback
            };
            out.emit_fmt(format_args!("    {} {}, {}", mnem, src_str, dst_str));
        }

        MachInst::Cmov { cc, src, dst, size } => {
            let cc_str = cc_suffix(*cc);
            let suffix = size.suffix();
            let src_str = fmt_operand(src, *size, out);
            let dst_str = fmt_reg(dst, *size);
            out.emit_fmt(format_args!(
                "    cmov{}{} {}, {}",
                cc_str, suffix, src_str, dst_str
            ));
        }

        MachInst::Jcc { cc, target } => {
            let cc_str = cc_suffix(*cc);
            out.emit_fmt(format_args!("    j{} {}", cc_str, target));
        }

        MachInst::Jmp { target } => {
            out.emit_fmt(format_args!("    jmp {}", target));
        }

        MachInst::Label(name) => {
            out.emit_fmt(format_args!("{}:", name));
        }

        MachInst::Call { target } => {
            out.emit_fmt(format_args!("    call {}", target));
        }

        MachInst::CallTyped {
            caller_saves,
            args,
            target,
            ret,
        } => {
            // Phase 1: caller-save spills (Phase 2b of the mature path,
            // typed). Same slot addressing as every other MachOperand —
            // the StackSlot formatter applies the rsp/rbp frame mode.
            for (reg, slot) in caller_saves {
                out.emit_fmt(format_args!(
                    "    movq {}, {}",
                    fmt_reg(&MachReg::Phys(*reg), OpSize::S64),
                    fmt_operand(&MachOperand::StackSlot(*slot), OpSize::S64, out)
                ));
            }
            // Phase 2: argument moves in the lowering's execution order.
            // Immediate-width contract: 0 → xorl (3 bytes, full-register
            // zero, the GCC/Clang/ICC form); inside the sign-extended imm32
            // window → the regular mov; outside it → movabsq into the
            // destination register itself. A 64-bit constant cannot be
            // named by an inline mov operand (GAS: "operand type mismatch")
            // and truncating it would be a silent miscompile — staging it
            // through the destination register is both the shortest and the
            // only sound form. The move reads nothing, so it is a pure
            // writer in the lowering's topological order: every move that
            // reads this argument register executes before it.
            for m in args {
                match &m.src {
                    // Zero immediate: `xorl` zeroes the full register
                    // (32-bit destination zero-extends) in 3 bytes vs 7
                    // for `movq $0` — the GCC/Clang/ICC form.
                    MachOperand::Imm(0) => {
                        let r = fmt_reg(&MachReg::Phys(m.dst_reg), OpSize::S32);
                        out.emit_fmt(format_args!("    xorl {}, {}", r, r));
                    }
                    MachOperand::Imm(v) if *v < i32::MIN as i64 || *v > i32::MAX as i64 => {
                        out.emit_fmt(format_args!("    movabsq ${}, %{}", v, reg_name(m.dst_reg)));
                    }
                    _ => out.emit_fmt(format_args!(
                        "    mov{} {}, {}",
                        m.size.suffix(),
                        fmt_operand(&m.src, m.size, out),
                        fmt_reg(&MachReg::Phys(m.dst_reg), m.size)
                    )),
                }
            }
            // Phase 3: the call itself.
            match target {
                CallTarget::Direct(sym) => out.emit_fmt(format_args!("    call {}", sym)),
                CallTarget::Indirect(reg) => {
                    out.emit_fmt(format_args!("    call *%{}", reg_name(*reg)))
                }
            }
            // Phase 4: return home.
            if let Some(r) = ret {
                out.emit_fmt(format_args!(
                    "    mov{} %{}, {}",
                    r.size.suffix(),
                    sized_reg_name(RAX, r.size),
                    fmt_operand(&r.dst, r.size, out)
                ));
            }
            // Phase 5: caller-save restores.
            for (reg, slot) in caller_saves.iter().rev() {
                out.emit_fmt(format_args!(
                    "    movq {}, {}",
                    fmt_operand(&MachOperand::StackSlot(*slot), OpSize::S64, out),
                    fmt_reg(&MachReg::Phys(*reg), OpSize::S64)
                ));
            }
        }

        MachInst::LeaSym { sym, dst } => {
            // Always 64-bit: an address.
            out.emit_fmt(format_args!(
                "    leaq {}(%rip), {}",
                sym,
                fmt_reg(dst, OpSize::S64)
            ));
        }

        MachInst::Ret => {
            out.emit("    ret");
        }

        MachInst::Raw(text) => {
            out.emit(text);
        }
    }
}

/// Emit a sequence of allocated MachInsts as AT&T assembly.
pub fn emit_machinsts(insts: &[MachInst], out: &mut AsmOutput) {
    // Raptor Lake Optimization: Identify loop headers (labels targeted by backward branches)
    // and align them to 32-byte boundaries (.p2align 5) to maximize instruction fetch and
    // uOP cache port bandwidth!
    let mut defined_labels = crate::common::fx_hash::FxHashSet::default();
    let mut loop_headers = crate::common::fx_hash::FxHashSet::default();

    for inst in insts {
        match inst {
            MachInst::Label(name) => {
                defined_labels.insert(name.clone());
            }
            MachInst::Jmp { target } | MachInst::Jcc { target, .. } => {
                if defined_labels.contains(target) {
                    loop_headers.insert(target.clone());
                }
            }
            _ => {}
        }
    }

    for inst in insts {
        if let MachInst::Label(name) = inst {
            if loop_headers.contains(name) {
                out.emit("    .p2align 5");
            }
        }
        emit_machinst(inst, out);
    }
}
