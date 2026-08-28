//! AArch64 NEON/SIMD intrinsic emission and F128 (quad-precision) soft-float helpers.
//!
//! NEON intrinsics: SSE-equivalent operations via 128-bit NEON instructions.
//! F128: IEEE 754 binary128 via compiler-rt/libgcc soft-float libcalls.

use super::emit::{ArmCodegen, arm_fp_name, arm_vector_name, callee_saved_name, is_arm_fp_phys};
use crate::common::types::IrType;
use crate::ir::reexports::{IntrinsicOp, Operand, Value};

impl ArmCodegen {
    /// If `val_id` is register-allocated to a NEON register, return its 128-bit
    /// `vN` name (allocator IDs 40..55 map to v16..v31).
    fn assigned_vector_reg(&self, val_id: u32) -> Option<String> {
        self.reg_assignments.get(&val_id).and_then(|&phys| {
            if is_arm_fp_phys(phys) {
                Some(arm_vector_name(phys))
            } else {
                None
            }
        })
    }

    /// Make a 128-bit vector operand available in a NEON register and return
    /// its `vN` name: the assigned register when the value is register-allocated,
    /// otherwise the value is loaded from its stack slot into `qreg`.
    fn load_vector_value_128(&mut self, op: &Operand, qreg: &str) -> String {
        if let Operand::Value(v) = op {
            if let Some(name) = self.assigned_vector_reg(v.0) {
                return name;
            }
            if let Some(slot) = self.state.get_slot(v.0) {
                self.emit_load_from_sp(qreg, slot.0, "ldr");
            }
        }
        qreg.replacen('q', "v", 1)
    }

    fn store_vector_value_128(&mut self, dest: &Value, qreg: &str) {
        self.state.vector_values.insert(dest.0);
        if let Some(name) = self.assigned_vector_reg(dest.0) {
            let src = qreg.replacen('q', "v", 1);
            self.state
                .emit_fmt(format_args!("    mov {}.16b, {}.16b", name, src));
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_sp(qreg, slot.0, "str");
        }
    }

    pub(super) fn emit_neon_binary_128(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        neon_inst: &str,
    ) {
        // Load first 128-bit operand pointer into x0, then load q0
        self.operand_to_x0(&args[0]);
        self.state.emit("    ldr q0, [x0]");
        // Load second 128-bit operand pointer into x1, then load q1
        match &args[1] {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    if self.state.is_alloca(v.0) {
                        self.emit_alloca_addr("x1", v.0, slot.0);
                    } else {
                        self.emit_load_from_sp("x1", slot.0, "ldr");
                    }
                }
            }
            Operand::Const(_) => {
                self.operand_to_x0(&args[1]);
                self.state.emit("    mov x1, x0");
            }
        }
        self.state.emit("    ldr q1, [x1]");
        // Apply the binary NEON operation
        self.state
            .emit_fmt(format_args!("    {} v0.16b, v0.16b, v1.16b", neon_inst));
        // Store result to dest_ptr
        self.load_ptr_to_reg(dest_ptr, "x0");
        self.state.emit("    str q0, [x0]");
    }

    /// Store a scalar result from x0 (or w0) into the dest stack slot.
    fn store_scalar_dest(&mut self, dest: &Option<Value>, reg: &str) {
        if let Some(d) = dest {
            if let Some(slot) = self.state.get_slot(d.0) {
                self.emit_store_to_sp(reg, slot.0, "str");
            }
        }
    }

    /// Emit a unary F64 operation: apply `op_inst` with the operand in d0, store result.
    /// Register-aware: reads/writes allocated FP registers directly when present.
    fn emit_f64_unary_neon(&mut self, dest: &Option<Value>, args: &[Operand], op_inst: &str) {
        // When the dest has an FP register, compute straight into it.
        let dest_reg = dest.and_then(|d| {
            self.reg_assignments.get(&d.0).and_then(|&phys| {
                if is_arm_fp_phys(phys) {
                    Some(arm_fp_name(phys, IrType::F64))
                } else {
                    None
                }
            })
        });
        if let Some(dreg) = dest_reg {
            let src = self.float_operand_reg(&args[0], IrType::F64, "d0");
            self.state
                .emit_fmt(format_args!("    {} {}, {}", op_inst, dreg, src));
            return;
        }
        self.float_operand_to_reg(&args[0], IrType::F64, "d0");
        self.state.emit_fmt(format_args!("    {} d0, d0", op_inst));
        if let Some(d) = dest {
            self.store_float_reg(d, IrType::F64, "d0");
        }
    }

    /// Emit a unary F32 operation: apply `op_inst` with the operand in s0, store result.
    fn emit_f32_unary_neon(&mut self, dest: &Option<Value>, args: &[Operand], op_inst: &str) {
        self.float_operand_to_reg(&args[0], IrType::F32, "s0");
        self.state.emit_fmt(format_args!("    {} s0, s0", op_inst));
        if let Some(d) = dest {
            self.store_float_reg(d, IrType::F32, "s0");
        }
    }

    /// Emit a non-temporal store: load value from args[0], store to dest_ptr.
    fn emit_nontemporal_store(
        &mut self,
        dest_ptr: &Option<Value>,
        args: &[Operand],
        save_reg: &str,
        val_reg: &str,
    ) {
        if let Some(ptr) = dest_ptr {
            self.operand_to_x0(&args[0]);
            self.state
                .emit_fmt(format_args!("    mov {}, {}", save_reg, val_reg));
            self.load_ptr_to_reg(ptr, "x0");
            self.state
                .emit_fmt(format_args!("    str {}, [x0]", save_reg));
        }
    }

    /// Materialize the effective address `args[base_idx] + args[base_idx+1]`
    /// into a register and return its name.
    ///
    /// EVERY register-based Vec* load/store intrinsic carries a (base,
    /// byte-offset) pair — the byte-IV reduction rewrite passes the loop's
    /// marching offset as args[1]. The original ARM lowerings read only
    /// args[0]: the vector loop then loads the SAME lanes every iteration
    /// (sum of 512 i32s returned 512*(a[0]+a[1]) — found via the map-vec
    /// backport's differential test, present in every prior ARM build).
    /// x86's lowering honors the offset; this helper is the ARM equivalent.
    ///
    /// A constant 0 offset keeps the fast path (base register used as-is).
    fn vec_addr_from_args(&mut self, base: &Operand, offset: Option<&Operand>) -> String {
        use crate::ir::reexports::IrConst;
        let zero_off = match offset {
            None => true,
            Some(Operand::Const(c)) => c.to_i64() == Some(0),
            _ => false,
        };
        let base_phys = self.operand_reg(base).filter(|r| !is_arm_fp_phys(*r));
        if zero_off {
            if let Some(r) = base_phys {
                return callee_saved_name(r).to_string();
            }
            self.operand_to_x0(base);
            self.state.emit("    mov x10, x0");
            return "x10".to_string();
        }
        // Non-zero offset: x10 = base + offset. x10 is the designated
        // scratch for address formation in this backend (never allocated).
        let off = offset.expect("checked above");
        match (
            base_phys,
            self.operand_reg(off).filter(|r| !is_arm_fp_phys(*r)),
        ) {
            (Some(b), Some(o)) => {
                self.state.emit_fmt(format_args!(
                    "    add x10, {}, {}",
                    callee_saved_name(b),
                    callee_saved_name(o)
                ));
            }
            (Some(b), None) => {
                if let Operand::Const(c) = off {
                    let k = c.to_i64().unwrap_or(0);
                    if (0..=4095).contains(&k) {
                        self.state.emit_fmt(format_args!(
                            "    add x10, {}, #{}",
                            callee_saved_name(b),
                            k
                        ));
                        return "x10".to_string();
                    }
                }
                self.operand_to_x0(off);
                self.state
                    .emit_fmt(format_args!("    add x10, {}, x0", callee_saved_name(b)));
            }
            (None, _) => {
                // Base not register-resident: build base in x10 first, then
                // add the offset through x0 (operand_to_x0 may clobber x0
                // only — x10 survives).
                self.operand_to_x0(base);
                self.state.emit("    mov x10, x0");
                self.operand_to_x0(off);
                self.state.emit("    add x10, x10, x0");
            }
        }
        self.state.reg_cache.invalidate_acc();
        "x10".to_string()
    }

    pub(super) fn emit_intrinsic_arm(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        match op {
            IntrinsicOp::Lfence | IntrinsicOp::Mfence => {
                self.state.emit("    dmb ish");
            }
            IntrinsicOp::Sfence => {
                self.state.emit("    dmb ishst");
            }
            IntrinsicOp::Pause => {
                self.state.emit("    yield");
            }
            IntrinsicOp::Clflush => {
                // ARM has no direct clflush; use dc civac (clean+invalidate to PoC)
                self.operand_to_x0(&args[0]);
                self.state.emit("    dc civac, x0");
            }
            IntrinsicOp::Movnti => {
                self.emit_nontemporal_store(dest_ptr, args, "w9", "w0");
            }
            IntrinsicOp::Movnti64 => {
                self.emit_nontemporal_store(dest_ptr, args, "x9", "x0");
            }
            IntrinsicOp::Movntdq | IntrinsicOp::Movntpd => {
                // Non-temporal 128-bit store: dest_ptr = target, args[0] = source ptr
                if let Some(ptr) = dest_ptr {
                    self.operand_to_x0(&args[0]);
                    self.state.emit("    ldr q0, [x0]");
                    self.load_ptr_to_reg(ptr, "x0");
                    self.state.emit("    str q0, [x0]");
                }
            }
            IntrinsicOp::Loaddqu => {
                // Load 128-bit unaligned: args[0] = source ptr, dest_ptr = result storage
                if let Some(dptr) = dest_ptr {
                    self.operand_to_x0(&args[0]);
                    self.state.emit("    ldr q0, [x0]");
                    self.load_ptr_to_reg(dptr, "x0");
                    self.state.emit("    str q0, [x0]");
                }
            }
            IntrinsicOp::Storedqu => {
                // Store 128-bit unaligned: dest_ptr = target ptr, args[0] = source data ptr
                if let Some(ptr) = dest_ptr {
                    self.operand_to_x0(&args[0]);
                    self.state.emit("    ldr q0, [x0]");
                    self.load_ptr_to_reg(ptr, "x0");
                    self.state.emit("    str q0, [x0]");
                }
            }
            IntrinsicOp::Pcmpeqb128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_neon_binary_128(dptr, args, "cmeq");
                }
            }
            IntrinsicOp::Pcmpeqd128 => {
                if let Some(dptr) = dest_ptr {
                    // For 32-bit lane equality, load q regs, use cmeq with .4s arrangement
                    self.operand_to_x0(&args[0]);
                    self.state.emit("    ldr q0, [x0]");
                    if let Operand::Value(v) = &args[1] {
                        self.load_ptr_to_reg(v, "x1");
                    } else {
                        self.operand_to_x0(&args[1]);
                        self.state.emit("    mov x1, x0");
                    }
                    self.state.emit("    ldr q1, [x1]");
                    self.state.emit("    cmeq v0.4s, v0.4s, v1.4s");
                    self.load_ptr_to_reg(dptr, "x0");
                    self.state.emit("    str q0, [x0]");
                }
            }
            IntrinsicOp::Psubusb128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_neon_binary_128(dptr, args, "uqsub");
                }
            }
            IntrinsicOp::Psubsb128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_neon_binary_128(dptr, args, "sqsub");
                }
            }
            IntrinsicOp::Por128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_neon_binary_128(dptr, args, "orr");
                }
            }
            IntrinsicOp::Pand128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_neon_binary_128(dptr, args, "and");
                }
            }
            IntrinsicOp::Pxor128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_neon_binary_128(dptr, args, "eor");
                }
            }
            IntrinsicOp::Pmovmskb128 => {
                // Extract the high bit of each byte in a 128-bit vector into a 16-bit mask.
                // NEON has no pmovmskb equivalent, so we use a multi-step sequence:
                //   1. Load 128-bit data into v0
                //   2. Shift right each byte by 7 to isolate the sign bit
                //   3. Multiply by power-of-2 bit positions, then add across lanes
                self.operand_to_x0(&args[0]);
                self.state.emit("    ldr q0, [x0]");
                self.state.emit("    ushr v0.16b, v0.16b, #7");
                // Load bit position constants: [1,2,4,8,16,32,64,128] repeated
                self.state.emit("    movz x0, #0x0201");
                self.state.emit("    movk x0, #0x0804, lsl #16");
                self.state.emit("    movk x0, #0x2010, lsl #32");
                self.state.emit("    movk x0, #0x8040, lsl #48");
                self.state.emit("    fmov d1, x0");
                self.state.emit("    mov v1.d[1], x0");
                self.state.emit("    mul v0.16b, v0.16b, v1.16b");
                // Split and sum each half
                self.state.emit("    ext v1.16b, v0.16b, v0.16b, #8");
                self.state.emit("    addv b0, v0.8b");
                self.state.emit("    umov w0, v0.b[0]");
                self.state.emit("    addv b1, v1.8b");
                self.state.emit("    umov w1, v1.b[0]");
                self.state.emit("    orr w0, w0, w1, lsl #8");
                self.store_scalar_dest(dest, "x0");
            }
            IntrinsicOp::SetEpi8 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_x0(&args[0]);
                    self.state.emit("    dup v0.16b, w0");
                    self.load_ptr_to_reg(dptr, "x0");
                    self.state.emit("    str q0, [x0]");
                }
            }
            IntrinsicOp::SetEpi32 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_x0(&args[0]);
                    self.state.emit("    dup v0.4s, w0");
                    self.load_ptr_to_reg(dptr, "x0");
                    self.state.emit("    str q0, [x0]");
                }
            }
            IntrinsicOp::Crc32_8
            | IntrinsicOp::Crc32_16
            | IntrinsicOp::Crc32_32
            | IntrinsicOp::Crc32_64 => {
                let is_64 = matches!(op, IntrinsicOp::Crc32_64);
                let (save_reg, crc_inst) = match op {
                    IntrinsicOp::Crc32_8 => ("w9", "crc32cb w9, w9, w0"),
                    IntrinsicOp::Crc32_16 => ("w9", "crc32ch w9, w9, w0"),
                    IntrinsicOp::Crc32_32 => ("w9", "crc32cw w9, w9, w0"),
                    IntrinsicOp::Crc32_64 => ("x9", "crc32cx w9, w9, x0"),
                    _ => unreachable!(),
                };
                self.operand_to_x0(&args[0]);
                self.state.emit_fmt(format_args!(
                    "    mov {}, {}",
                    save_reg,
                    if is_64 { "x0" } else { "w0" }
                ));
                self.operand_to_x0(&args[1]);
                self.state.emit_fmt(format_args!("    {}", crc_inst));
                self.state.emit("    mov x0, x9");
                self.store_scalar_dest(dest, "x0");
            }
            IntrinsicOp::FrameAddress => {
                self.state.emit("    mov x0, x29");
                self.store_scalar_dest(dest, "x0");
            }
            IntrinsicOp::ReturnAddress => {
                // x30 (lr) is clobbered by bl instructions, so read from stack
                self.state.emit("    ldr x0, [x29, #8]");
                self.store_scalar_dest(dest, "x0");
            }
            IntrinsicOp::ThreadPointer => {
                // __builtin_thread_pointer(): read TLS base from tpidr_el0
                self.state.emit("    mrs x0, tpidr_el0");
                self.store_scalar_dest(dest, "x0");
            }
            IntrinsicOp::SqrtF64 => self.emit_f64_unary_neon(dest, args, "fsqrt"),
            IntrinsicOp::SqrtF32 => self.emit_f32_unary_neon(dest, args, "fsqrt"),
            IntrinsicOp::FabsF64 => self.emit_f64_unary_neon(dest, args, "fabs"),
            IntrinsicOp::FabsF32 => self.emit_f32_unary_neon(dest, args, "fabs"),
            // ── Scalar FP round/FMA/copysign ────────────────────────────────
            // x86 lowered these to SSE ROUNDSD/VFMA/VANDPD; without arms here
            // they hit the silent catch-all and produced uninitialized
            // registers (observed: rint/floor/ceil/trunc/copysign/fma all
            // returned 0.0 on aarch64, vex_move_false_dep outputs DIFFER).
            IntrinsicOp::RoundScalarF64(imm) => {
                // x86 ROUNDSD imm8 rounding modes map 1:1 onto AArch64
                // FRINT* variants; bit 3 (precision mask) is not emitted.
                let frint = match imm & 3 {
                    0 => "frintn", // round to nearest, ties to even
                    1 => "frintm", // toward -inf (floor)
                    2 => "frintp", // toward +inf (ceil)
                    3 => "frintz", // toward zero (trunc)
                    _ => unreachable!("ROUND imm8 mode bits are 2 bits wide"),
                };
                self.emit_f64_unary_neon(dest, args, frint);
            }
            IntrinsicOp::RoundScalarF32(imm) => {
                let frint = match imm & 3 {
                    0 => "frintn",
                    1 => "frintm",
                    2 => "frintp",
                    3 => "frintz",
                    _ => unreachable!("ROUND imm8 mode bits are 2 bits wide"),
                };
                self.emit_f32_unary_neon(dest, args, frint);
            }
            IntrinsicOp::FmaScalarF64 => {
                // dest = a*b + c, single rounding: native fmadd.
                self.float_operand_to_reg(&args[0], IrType::F64, "d0");
                self.float_operand_to_reg(&args[1], IrType::F64, "d1");
                self.float_operand_to_reg(&args[2], IrType::F64, "d2");
                self.state.emit("    fmadd d0, d0, d1, d2");
                if let Some(d) = dest {
                    self.store_float_reg(d, IrType::F64, "d0");
                }
            }
            IntrinsicOp::FmaScalarF32 => {
                self.float_operand_to_reg(&args[0], IrType::F32, "s0");
                self.float_operand_to_reg(&args[1], IrType::F32, "s1");
                self.float_operand_to_reg(&args[2], IrType::F32, "s2");
                self.state.emit("    fmadd s0, s0, s1, s2");
                if let Some(d) = dest {
                    self.store_float_reg(d, IrType::F32, "s0");
                }
            }
            IntrinsicOp::CopysignF64 => {
                // result = |x| with y's sign bit: pure integer bit ops on
                // the F64 bit patterns (no branch, no libm call).
                self.float_operand_to_reg(&args[0], IrType::F64, "d0");
                self.float_operand_to_reg(&args[1], IrType::F64, "d1");
                self.state.emit("    fmov x0, d0");
                self.state.emit("    fmov x1, d1");
                self.state.emit("    and x0, x0, #0x7fffffffffffffff");
                self.state.emit("    and x1, x1, #0x8000000000000000");
                self.state.emit("    orr x0, x0, x1");
                self.state.emit("    fmov d0, x0");
                if let Some(d) = dest {
                    self.store_float_reg(d, IrType::F64, "d0");
                }
            }
            IntrinsicOp::CopysignF32 => {
                self.float_operand_to_reg(&args[0], IrType::F32, "s0");
                self.float_operand_to_reg(&args[1], IrType::F32, "s1");
                self.state.emit("    fmov w0, s0");
                self.state.emit("    fmov w1, s1");
                self.state.emit("    and w0, w0, #0x7fffffff");
                self.state.emit("    and w1, w1, #0x80000000");
                self.state.emit("    orr w0, w0, w1");
                self.state.emit("    fmov s0, w0");
                if let Some(d) = dest {
                    self.store_float_reg(d, IrType::F32, "s0");
                }
            }
            // x86-specific SSE/AES-NI/CLMUL intrinsics - these are x86-only and should
            // not appear in ARM codegen in practice. Cross-compiled code that conditionally
            // uses these behind #ifdef __x86_64__ will have the calls dead-code eliminated.
            // TODO: consider emitting a runtime trap instead of silent zeros
            IntrinsicOp::Aesenc128
            | IntrinsicOp::Aesenclast128
            | IntrinsicOp::Aesdec128
            | IntrinsicOp::Aesdeclast128
            | IntrinsicOp::Aesimc128
            | IntrinsicOp::Aeskeygenassist128
            | IntrinsicOp::Pclmulqdq128
            | IntrinsicOp::Pslldqi128
            | IntrinsicOp::Psrldqi128
            | IntrinsicOp::Psllqi128
            | IntrinsicOp::Psrlqi128
            | IntrinsicOp::Pshufd128
            | IntrinsicOp::Loadldi128
            | IntrinsicOp::Paddw128
            | IntrinsicOp::Psubw128
            | IntrinsicOp::Pmulhw128
            | IntrinsicOp::Pmaddwd128
            | IntrinsicOp::Pcmpgtw128
            | IntrinsicOp::Pcmpgtb128
            | IntrinsicOp::Psllwi128
            | IntrinsicOp::Psrlwi128
            | IntrinsicOp::Psrawi128
            | IntrinsicOp::Psradi128
            | IntrinsicOp::Pslldi128
            | IntrinsicOp::Psrldi128
            | IntrinsicOp::Paddd128
            | IntrinsicOp::Psubd128
            | IntrinsicOp::Packssdw128
            | IntrinsicOp::Packsswb128
            | IntrinsicOp::Packuswb128
            | IntrinsicOp::Punpcklbw128
            | IntrinsicOp::Punpckhbw128
            | IntrinsicOp::Punpcklwd128
            | IntrinsicOp::Punpckhwd128
            | IntrinsicOp::SetEpi16
            | IntrinsicOp::Pinsrw128
            | IntrinsicOp::Pextrw128
            | IntrinsicOp::Storeldi128
            | IntrinsicOp::Cvtsi128Si32
            | IntrinsicOp::Cvtsi32Si128
            | IntrinsicOp::Cvtsi128Si64
            | IntrinsicOp::Pshuflw128
            | IntrinsicOp::Pshufhw128
            | IntrinsicOp::Pinsrd128
            | IntrinsicOp::Pextrd128
            | IntrinsicOp::Pinsrb128
            | IntrinsicOp::Pextrb128
            | IntrinsicOp::Pinsrq128
            | IntrinsicOp::Pextrq128
            | IntrinsicOp::FmaF64x4
            | IntrinsicOp::FmaF64x4Hoisted
            | IntrinsicOp::FmaF64x4SIB
            | IntrinsicOp::LoadF64x4
            | IntrinsicOp::LoadF64x2
            | IntrinsicOp::LoadI32x8
            | IntrinsicOp::LoadI32x4
            | IntrinsicOp::AddF64x4
            | IntrinsicOp::AddF64x2
            | IntrinsicOp::MulF64x4
            | IntrinsicOp::MulF64x2
            | IntrinsicOp::AddI32x8
            | IntrinsicOp::AddI32x4
            | IntrinsicOp::HorizontalAddF64x4
            | IntrinsicOp::HorizontalAddF64x2
            | IntrinsicOp::HorizontalAddI32x8
            | IntrinsicOp::HorizontalAddI32x4 => {
                // x86-only: zero dest if present
                if let Some(dptr) = dest_ptr {
                    if let Some(slot) = self.state.get_slot(dptr.0) {
                        self.state
                            .emit_fmt(format_args!("    add x9, sp, #{}", slot.0));
                        self.state.emit("    stp xzr, xzr, [x9]");
                    }
                }
            }

            IntrinsicOp::FmaF64x2 => {
                if let Some(c_ptr) = dest_ptr {
                    // Preserve all three addresses before using SIMD registers;
                    // operand_to_x0 may itself need x9 for an indirect value.
                    self.operand_to_x0(&args[0]);
                    self.state.emit("    mov x10, x0");
                    self.operand_to_x0(&args[1]);
                    self.state.emit("    mov x11, x0");
                    self.operand_to_x0(&Operand::Value(*c_ptr));
                    self.state.emit("    mov x12, x0");
                    self.state.emit("    ldr d0, [x10]");
                    self.state.emit("    ldr q1, [x11]");
                    self.state.emit("    ldr q2, [x12]");
                    self.state.emit("    dup v0.2d, v0.d[0]");
                    self.state.emit("    fmla v2.2d, v1.2d, v0.2d");
                    self.state.emit("    str q2, [x12]");
                }
            }

            IntrinsicOp::BroadcastLoadF64 => {
                self.operand_to_x0(&args[0]);
                self.state.emit("    ldr d15, [x0]");
                self.state.emit("    dup v15.2d, v15.d[0]");
            }

            IntrinsicOp::FmaF64x2Hoisted => {
                if let Some(c_ptr) = dest_ptr {
                    let b_phys = self.operand_reg(&args[0]).filter(|r| !is_arm_fp_phys(*r));
                    let c_phys = self
                        .operand_reg(&Operand::Value(*c_ptr))
                        .filter(|r| !is_arm_fp_phys(*r));
                    let b_addr = b_phys.map(callee_saved_name).unwrap_or("x11");
                    let c_addr = c_phys.map(callee_saved_name).unwrap_or("x12");
                    if b_phys.is_none() {
                        self.operand_to_x0(&args[0]);
                        self.state.emit("    mov x11, x0");
                    }
                    if c_phys.is_none() {
                        self.operand_to_x0(&Operand::Value(*c_ptr));
                        self.state.emit("    mov x12, x0");
                    }
                    self.state
                        .emit_fmt(format_args!("    ldr q1, [{}]", b_addr));
                    self.state
                        .emit_fmt(format_args!("    ldr q2, [{}]", c_addr));
                    self.state.emit("    fmla v2.2d, v1.2d, v15.2d");
                    self.state
                        .emit_fmt(format_args!("    str q2, [{}]", c_addr));
                    self.state
                        .emit_fmt(format_args!("    ldr q1, [{}, #16]", b_addr));
                    self.state
                        .emit_fmt(format_args!("    ldr q2, [{}, #16]", c_addr));
                    self.state.emit("    fmla v2.2d, v1.2d, v15.2d");
                    self.state
                        .emit_fmt(format_args!("    str q2, [{}, #16]", c_addr));
                }
            }

            IntrinsicOp::VecZeroI64x2 => {
                if let Some(d) = dest {
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        self.state.vector_values.insert(d.0);
                        self.state
                            .emit_fmt(format_args!("    eor {0}.16b, {0}.16b, {0}.16b", name));
                    } else {
                        self.state.emit("    eor v0.16b, v0.16b, v0.16b");
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            IntrinsicOp::VecLoadWidenI32ToI64x2 => {
                if let Some(d) = dest {
                    let addr = self.vec_addr_from_args(&args[0], args.get(1));
                    self.state.emit_fmt(format_args!("    ldr d0, [{}]", addr));
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        self.state.vector_values.insert(d.0);
                        self.state
                            .emit_fmt(format_args!("    sxtl {}.2d, v0.2s", name));
                    } else {
                        self.state.emit("    sxtl v0.2d, v0.2s");
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            IntrinsicOp::VecAddI64x2 | IntrinsicOp::VecMulI64x2 => {
                if let Some(d) = dest {
                    let a = self.load_vector_value_128(&args[0], "q0");
                    let b = self.load_vector_value_128(&args[1], "q1");
                    if *op == IntrinsicOp::VecAddI64x2 {
                        if let Some(name) = self.assigned_vector_reg(d.0) {
                            self.state.vector_values.insert(d.0);
                            self.state.emit_fmt(format_args!(
                                "    add {}.2d, {}.2d, {}.2d",
                                name, a, b
                            ));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    add v0.2d, {}.2d, {}.2d", a, b));
                            self.store_vector_value_128(d, "q0");
                        }
                    } else {
                        // NEON has no 64-bit integer MUL (armasm rejects
                        // `mul v.2d`). Expand per 64-bit lane into GPRs:
                        // wrapping i64 multiply is exactly `mul x, x, x`.
                        // Scratch is confined to x0/x9; q0 (a) and q1 (b)
                        // stay intact until each lane's product is written.
                        let lane = |rd: &str, idx: u32| -> String {
                            format!("    mov {}.d[{}], x0", rd, idx)
                        };
                        let compute = |idx: u32| -> [String; 3] {
                            [
                                format!("    umov x0, {}.d[{}]", a, idx),
                                format!("    umov x9, {}.d[{}]", b, idx),
                                "    mul x0, x0, x9".to_string(),
                            ]
                        };
                        if let Some(name) = self.assigned_vector_reg(d.0) {
                            self.state.vector_values.insert(d.0);
                            for ins in compute(0) {
                                self.state.emit(&ins);
                            }
                            self.state.emit(lane(&name, 0).as_str());
                            for ins in compute(1) {
                                self.state.emit(&ins);
                            }
                            self.state.emit(lane(&name, 1).as_str());
                        } else {
                            for ins in compute(0) {
                                self.state.emit(&ins);
                            }
                            self.state.emit(lane("v0", 0).as_str());
                            for ins in compute(1) {
                                self.state.emit(&ins);
                            }
                            self.state.emit(lane("v0", 1).as_str());
                            self.store_vector_value_128(d, "q0");
                        }
                    }
                }
            }

            IntrinsicOp::VecHorizontalAddI64x2 => {
                let a = self.load_vector_value_128(&args[0], "q0");
                self.state.emit_fmt(format_args!("    addp d0, {}.2d", a));
                self.state.emit("    fmov x0, d0");
                if let Some(d) = dest {
                    self.store_x0_to(d);
                }
            }

            // NEON smaxv: horizontal signed max of 4 I32 lanes into a scalar.
            // The result is a full-width I32 (smaxv writes s0's low 32 bits);
            // fmov w0 zero-extends into x0, matching the I32 store contract.
            IntrinsicOp::VecHorizontalMaxI32x4 => {
                let a = self.load_vector_value_128(&args[0], "q0");
                self.state.emit_fmt(format_args!("    smaxv s0, {}.4s", a));
                self.state.emit("    fmov w0, s0");
                if let Some(d) = dest {
                    self.store_x0_to(d);
                }
            }

            // NEON sadalp: accumulate sign-extended adjacent pairs of a 4×I32
            // vector into a 2×I64 accumulator (one instruction per 4 elements).
            IntrinsicOp::VecSadalpI32x4 => {
                if let Some(d) = dest {
                    let acc = self.load_vector_value_128(&args[0], "q0");
                    let src = self.load_vector_value_128(&args[1], "q1");
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        self.state.vector_values.insert(d.0);
                        if name != acc {
                            self.state
                                .emit_fmt(format_args!("    mov {}.16b, {}.16b", name, acc));
                        }
                        self.state
                            .emit_fmt(format_args!("    sadalp {}.2d, {}.4s", name, src));
                    } else {
                        if acc != "v0" {
                            self.state
                                .emit_fmt(format_args!("    mov v0.16b, {}.16b", acc));
                        }
                        self.state
                            .emit_fmt(format_args!("    sadalp v0.2d, {}.4s", src));
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            // NEON smlal/smlal2: accumulate sign-extended products of the low
            // (2s) or high (4s) halves of two 4×I32 vectors into a 2×I64 acc.
            IntrinsicOp::VecSmlalLoI32x4 | IntrinsicOp::VecSmlalHiI32x4 => {
                if let Some(d) = dest {
                    let acc = self.load_vector_value_128(&args[0], "q0");
                    let a = self.load_vector_value_128(&args[1], "q1");
                    let b = self.load_vector_value_128(&args[2], "q2");
                    let (mnemonic, suffix) = if *op == IntrinsicOp::VecSmlalLoI32x4 {
                        ("smlal", "2s")
                    } else {
                        ("smlal2", "4s")
                    };
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        self.state.vector_values.insert(d.0);
                        if name != acc {
                            self.state
                                .emit_fmt(format_args!("    mov {}.16b, {}.16b", name, acc));
                        }
                        self.state.emit_fmt(format_args!(
                            "    {} {}.2d, {}.{}, {}.{}",
                            mnemonic, name, a, suffix, b, suffix
                        ));
                    } else {
                        if acc != "v0" {
                            self.state
                                .emit_fmt(format_args!("    mov v0.16b, {}.16b", acc));
                        }
                        self.state.emit_fmt(format_args!(
                            "    {} v0.2d, {}.{}, {}.{}",
                            mnemonic, a, suffix, b, suffix
                        ));
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            // Register-based vector operations (NEON 128-bit).
            // Two-wide F64 (2×.2d) and four-wide I32 (4×.4s) for reductions.
            IntrinsicOp::VecLoadF64x2
            | IntrinsicOp::VecLoadF32x4
            | IntrinsicOp::VecLoadI32x4
            | IntrinsicOp::VecLoadI64x2 => {
                if let Some(d) = dest {
                    let addr = self.vec_addr_from_args(&args[0], args.get(1));
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        // Load directly into the assigned register (no copy).
                        self.state.vector_values.insert(d.0);
                        let qname = name.replacen('v', "q", 1);
                        self.state
                            .emit_fmt(format_args!("    ldr {}, [{}]", qname, addr));
                    } else {
                        self.state.emit_fmt(format_args!("    ldr q0, [{}]", addr));
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            IntrinsicOp::VecZeroF64x2
            | IntrinsicOp::VecZeroF32x4
            | IntrinsicOp::VecZeroI32x4 => {
                if let Some(d) = dest {
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        self.state.vector_values.insert(d.0);
                        self.state
                            .emit_fmt(format_args!("    eor {0}.16b, {0}.16b, {0}.16b", name));
                    } else {
                        self.state.emit("    eor v0.16b, v0.16b, v0.16b");
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            IntrinsicOp::VecAddF64x2
            | IntrinsicOp::VecMulF64x2
            | IntrinsicOp::VecAddF32x4
            | IntrinsicOp::VecMulF32x4
            | IntrinsicOp::VecAddI32x4
            | IntrinsicOp::VecSmaxI32x4
            | IntrinsicOp::VecSubF64x2
            | IntrinsicOp::VecDivF64x2
            | IntrinsicOp::VecSubF32x4
            | IntrinsicOp::VecDivF32x4
            | IntrinsicOp::VecMulI32x4 => {
                if let Some(d) = dest {
                    let a = self.load_vector_value_128(&args[0], "q0");
                    let b = self.load_vector_value_128(&args[1], "q1");
                    let (mnemonic, suffix) = match op {
                        IntrinsicOp::VecAddF64x2 => ("fadd", "2d"),
                        IntrinsicOp::VecMulF64x2 => ("fmul", "2d"),
                        IntrinsicOp::VecSubF64x2 => ("fsub", "2d"),
                        IntrinsicOp::VecDivF64x2 => ("fdiv", "2d"),
                        IntrinsicOp::VecAddF32x4 => ("fadd", "4s"),
                        IntrinsicOp::VecMulF32x4 => ("fmul", "4s"),
                        IntrinsicOp::VecSubF32x4 => ("fsub", "4s"),
                        IntrinsicOp::VecDivF32x4 => ("fdiv", "4s"),
                        IntrinsicOp::VecAddI32x4 => ("add", "4s"),
                        IntrinsicOp::VecSmaxI32x4 => ("smax", "4s"),
                        _ => ("mul", "4s"),
                    };
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        self.state.vector_values.insert(d.0);
                        self.state.emit_fmt(format_args!(
                            "    {} {}.{}, {}.{}, {}.{}",
                            mnemonic, name, suffix, a, suffix, b, suffix
                        ));
                    } else {
                        self.state.emit_fmt(format_args!(
                            "    {} v0.{}, {}.{}, {}.{}",
                            mnemonic, suffix, a, suffix, b, suffix
                        ));
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            IntrinsicOp::VecBroadcastF64x2 | IntrinsicOp::VecBroadcastF32x4 => {
                if let Some(d) = dest {
                    let (ty, lane, suffix) = if matches!(op, IntrinsicOp::VecBroadcastF64x2) {
                        (IrType::F64, "d", "2d")
                    } else {
                        (IrType::F32, "s", "4s")
                    };
                    self.float_operand_to_reg(
                        &args[0],
                        ty,
                        if ty == IrType::F64 { "d0" } else { "s0" },
                    );
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        self.state.vector_values.insert(d.0);
                        self.state
                            .emit_fmt(format_args!("    dup {}.{}, v0.{}[0]", name, suffix, lane));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    dup v0.{}, v0.{}[0]", suffix, lane));
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            IntrinsicOp::VecBroadcastI32x4 => {
                if let Some(d) = dest {
                    self.operand_to_x0(&args[0]);
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        self.state.vector_values.insert(d.0);
                        self.state.emit_fmt(format_args!("    dup {}.4s, w0", name));
                    } else {
                        self.state.emit("    dup v0.4s, w0");
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            IntrinsicOp::VecBroadcastI64x2 => {
                if let Some(d) = dest {
                    self.operand_to_x0(&args[0]);
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        self.state.vector_values.insert(d.0);
                        self.state.emit_fmt(format_args!("    dup {}.2d, x0", name));
                    } else {
                        self.state.emit("    dup v0.2d, x0");
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            IntrinsicOp::VecSqrtF64x2 | IntrinsicOp::VecSqrtF32x4 => {
                if let Some(d) = dest {
                    let a = self.load_vector_value_128(&args[0], "q0");
                    let suffix = if matches!(op, IntrinsicOp::VecSqrtF64x2) {
                        "2d"
                    } else {
                        "4s"
                    };
                    if let Some(name) = self.assigned_vector_reg(d.0) {
                        self.state.vector_values.insert(d.0);
                        self.state.emit_fmt(format_args!(
                            "    fsqrt {}.{}, {}.{}",
                            name, suffix, a, suffix
                        ));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    fsqrt v0.{}, {}.{}", suffix, a, suffix));
                        self.store_vector_value_128(d, "q0");
                    }
                }
            }

            IntrinsicOp::VecStoreF64x2
            | IntrinsicOp::VecStoreF32x4
            | IntrinsicOp::VecStoreI32x4
            | IntrinsicOp::VecStoreI64x2 => {
                // Store one 128-bit vector to dest_ptr.
                if dest_ptr.is_some() {
                    let src = self.load_vector_value_128(&args[0], "q0");
                    let addr = if args.len() >= 3 {
                        self.vec_addr_from_args(&args[1], args.get(2))
                    } else {
                        let ptr = dest_ptr.unwrap();
                        let base_phys = self
                            .operand_reg(&Operand::Value(ptr))
                            .filter(|r| !is_arm_fp_phys(*r));
                        if let Some(reg) = base_phys {
                            callee_saved_name(reg).to_string()
                        } else {
                            self.operand_to_x0(&Operand::Value(ptr));
                            "x0".to_string()
                        }
                    };
                    self.state.emit_fmt(format_args!(
                        "    str {}, [{}]",
                        src.replacen('v', "q", 1),
                        addr
                    ));
                }
            }

            IntrinsicOp::VecHorizontalAddF64x2 => {
                let a = self.load_vector_value_128(&args[0], "q0");
                self.state.emit_fmt(format_args!("    faddp d0, {}.2d", a));
                if let Some(d) = dest {
                    self.store_float_reg(d, IrType::F64, "d0");
                }
            }

            IntrinsicOp::VecHorizontalAddI32x4 => {
                let a = self.load_vector_value_128(&args[0], "q0");
                self.state.emit_fmt(format_args!("    addv s0, {}.4s", a));
                self.state.emit("    fmov w0, s0");
                if let Some(d) = dest {
                    self.store_x0_to(d);
                }
            }
            IntrinsicOp::VecHorizontalAddF32x4 => {
                let a = self.load_vector_value_128(&args[0], "q0");
                self.state.emit_fmt(format_args!("    addv s0, {}.4s", a));
                if let Some(d) = dest {
                    self.store_float_reg(d, IrType::F32, "s0");
                }
            }

            // Not-yet-implemented register-based vector intrinsics for ARM.
            IntrinsicOp::VecLoadF64x4
            | IntrinsicOp::VecLoadI32x8
            | IntrinsicOp::VecAddF64x4
            | IntrinsicOp::VecMulF64x4
            | IntrinsicOp::VecAddI32x8
            | IntrinsicOp::VecHorizontalAddF64x4
            | IntrinsicOp::VecHorizontalAddI32x8
            | IntrinsicOp::VecZeroF64x4
            | IntrinsicOp::VecZeroI32x8 => {
                unimplemented!("4-wide/AVX vector intrinsics not implemented for ARM");
            }
            // Long-double / _Float128 sign-bit intrinsics. AArch64 long
            // double is IEEE binary128: sign bit 127 = bit 63 of the HIGH
            // qword (unlike x87 80-bit, where x86 patches byte 9). Pure GPR
            // bit ops on the slot-homed 16 bytes — no soft-float call, no
            // rounding of the quiet-bit payload, NaN sign preserved.
            IntrinsicOp::LDFabs | IntrinsicOp::F128Fabs => {
                if let Some(d) = dest {
                    self.emit_f128_operand_to_q0_full(&args[0]);
                    self.state.emit("    mov x0, v0.d[0]");
                    self.state.emit("    mov x9, v0.d[1]");
                    self.state.emit("    movz x10, #0x8000, lsl #48");
                    self.state.emit("    bic x9, x9, x10");
                    self.state.emit("    fmov d0, x0");
                    self.state.emit("    mov v0.d[1], x9");
                    if let Some(slot) = self.state.get_slot(d.0) {
                        self.emit_f128_store_q0_to_slot(slot);
                        // The dest slot now holds full-precision f128; mark
                        // it so later loads take the 16-byte path instead of
                        // the f64-extend fallback (wrong for copysign/fabs).
                        self.state.track_f128_self(d.0);
                    } else {
                        unimplemented!("LDFabs: dest is not slot-homed");
                    }
                }
            }
            IntrinsicOp::F128Neg => {
                if let Some(d) = dest {
                    self.emit_f128_operand_to_q0_full(&args[0]);
                    self.state.emit("    mov x0, v0.d[0]");
                    self.state.emit("    mov x9, v0.d[1]");
                    self.state.emit("    movz x10, #0x8000, lsl #48");
                    self.state.emit("    eor x9, x9, x10");
                    self.state.emit("    fmov d0, x0");
                    self.state.emit("    mov v0.d[1], x9");
                    if let Some(slot) = self.state.get_slot(d.0) {
                        self.emit_f128_store_q0_to_slot(slot);
                        self.state.track_f128_self(d.0);
                    } else {
                        unimplemented!("F128Neg: dest is not slot-homed");
                    }
                }
            }
            IntrinsicOp::LDCopysign | IntrinsicOp::F128Copysign => {
                // copysign(x, y): result = {x.lo, (x.hi & ~SIGN) | (y.hi & SIGN)}.
                if let Some(d) = dest {
                    self.emit_f128_operand_to_q0_full(&args[0]);
                    self.state.emit("    mov x0, v0.d[0]");
                    self.state.emit("    mov x1, v0.d[1]");
                    self.emit_f128_operand_to_q0_full(&args[1]);
                    self.state.emit("    mov x9, v0.d[1]");
                    self.state.emit("    movz x10, #0x8000, lsl #48");
                    self.state.emit("    and x9, x9, x10");
                    self.state.emit("    mvn x10, x10");
                    self.state.emit("    and x1, x1, x10");
                    self.state.emit("    orr x1, x1, x9");
                    self.state.emit("    fmov d0, x0");
                    self.state.emit("    mov v0.d[1], x1");
                    if let Some(slot) = self.state.get_slot(d.0) {
                        self.emit_f128_store_q0_to_slot(slot);
                        self.state.track_f128_self(d.0);
                    } else {
                        unimplemented!("LDCopysign: dest is not slot-homed");
                    }
                }
            }

            // Any vector intrinsic that reaches here has no NEON lowering.
            // Dropping it silently would emit a read of an uninitialized
            // vector register (observed: VecLoadI64x2 lowered to nothing and
            // the reduction accumulated garbage). Fail the compilation loudly
            // instead — a frontend gate that leaks an x86-shaped op to ARM is
            // a bug we want to see, not paper over.
            IntrinsicOp::VecAddF32x8
            | IntrinsicOp::VecBroadcastF32x8
            | IntrinsicOp::VecBroadcastF64x4
            | IntrinsicOp::VecBroadcastI32x8
            | IntrinsicOp::VecDivF32x8
            | IntrinsicOp::VecFmaF32x8
            | IntrinsicOp::VecFmaF64x4
            | IntrinsicOp::VecHorizontalAddF32x8
            | IntrinsicOp::VecHorizontalMaxI32x8
            | IntrinsicOp::VecLoadF32x8
            | IntrinsicOp::VecMaddF32x8
            | IntrinsicOp::VecMaddF64x4
            | IntrinsicOp::VecMaxI32x8
            | IntrinsicOp::VecMulF32x8
            | IntrinsicOp::VecMulI32x8
            | IntrinsicOp::VecSqrtF32x8
            | IntrinsicOp::VecStoreF32x8
            | IntrinsicOp::VecStoreF64x4
            | IntrinsicOp::VecStoreI32x8
            | IntrinsicOp::VecWidenAddI32x4ToI64x2
            | IntrinsicOp::VecWidenMaskedAddI32x4ToI64x2 => {
                unimplemented!(
                    "AArch64: no NEON lowering for vector intrinsic {:?} (x86-only shape leaked past the vectorizer target gate)",
                    op
                );
            }
            _ => { /* x86-only scalar/SSE-builtin op on arm: no-op */ }
        }
    }

    // ---- F128 (long double / IEEE quad precision) soft-float helpers ----
    //
    // On AArch64, long double is IEEE 754 binary128 (16 bytes).
    // Hardware has no quad-precision FP ops, so we use compiler-rt/libgcc soft-float:
    //   Comparison: __eqtf2, __lttf2, __letf2, __gttf2, __getf2
    //   Arithmetic: __addtf3, __subtf3, __multf3, __divtf3
    //   Conversion: __extenddftf2 (f64->f128), __trunctfdf2 (f128->f64)
    // ABI: f128 passed/returned in Q registers (q0, q1). Int result in w0/x0.
}
