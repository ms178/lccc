//! X86Codegen: return value operations.

use super::emit::X86Codegen;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrConst, Operand, Value};

impl X86Codegen {
    pub(super) fn emit_return_impl(&mut self, val: Option<&Operand>, frame_size: i64) {
        use crate::backend::state::SlotAddr;
        if let Some(val) = val {
            // A register-valued vector return (`try_lower_vector_return_simd`):
            // the psABI carrier is `%xmm0` (16 bytes) or `%ymm0` (32 bytes,
            // AVX ABI level). The value's home is an XMM/YMM register or
            // its protected slot; one move/load puts it in place.
            if let Operand::Value(v) = val {
                if self.state.vector_values.contains(&v.0) {
                    if self.state.vector128_values.contains(&v.0) {
                        self.sse_load_arg(val, "xmm0");
                        self.emit_epilogue_and_ret_impl(frame_size);
                    } else {
                        self.avx_load_arg_to(val, "ymm0");
                        // No `vzeroupper` on this exit (see the hidden-buffer
                        // path below): it would zero the upper lanes of
                        // the value just loaded.
                        let dirty = std::mem::replace(&mut self.state.dirty_upper_ymm, false);
                        self.emit_epilogue_and_ret_impl(frame_size);
                        self.state.dirty_upper_ymm = dirty;
                    }
                    return;
                }
            }
            let ret_ty = self.current_return_type;
            if matches!(
                ret_ty,
                IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64
            ) && matches!(val, Operand::Value(v)
                    if self.state.direct_fp_result == Some(v.0))
            {
                self.emit_epilogue_and_ret_impl(frame_size);
                return;
            }
            // _Float128 (IEEE binary128): the 16-byte value returns in xmm0.
            if self.func_ret_is_f128_sse {
                self.emit_f128_sse_operand_to_xmm(val, "xmm0");
                self.emit_epilogue_and_ret_impl(frame_size);
                return;
            }
            // A `__m256` / `__m512` aggregate under the AVX ABI level: the IR
            // returns the hidden pointer (the private frame buffer the
            // callee's returns copied into); the psABI wants the bytes in
            // `%ymm0` / `%zmm0`. `%rax` carries nothing meaningful after
            // this -- the caller stores the register through its own
            // buffer.
            if self.vector_ret_buffer_bytes > 0 {
                self.operand_to_rax(val);
                let (mov, reg) = super::calls::vector_reg_move(self.vector_ret_buffer_bytes, 0);
                self.state
                    .emit_fmt(format_args!("    {mov} (%rax), %{reg}"));
                // No `vzeroupper` on this exit: it would zero the upper
                // lanes of the value just loaded. GCC/Clang skip it as
                // well (a `%ymm0`/`%zmm0` return has an AVX-aware caller).
                let dirty = std::mem::replace(&mut self.state.dirty_upper_ymm, false);
                self.emit_epilogue_and_ret_impl(frame_size);
                self.state.dirty_upper_ymm = dirty;
                return;
            }
            if ret_ty.is_long_double() {
                if let Operand::Value(v) = val {
                    if self.state.f128_direct_slots.contains(&v.0) {
                        if let Some(slot) = self.state.get_slot(v.0) {
                            self.state.out.emit_instr_rbp("    fldt", slot.0);
                            self.emit_epilogue_and_ret_impl(frame_size);
                            return;
                        }
                    }
                    if let Some((ptr_id, _offset, _is_indirect)) = self.state.get_f128_source(v.0) {
                        if let Some(addr) = self.state.resolve_slot_addr(ptr_id) {
                            match addr {
                                SlotAddr::Direct(slot) => {
                                    self.state.out.emit_instr_rbp("    fldt", slot.0);
                                }
                                SlotAddr::OverAligned(slot, id) => {
                                    self.emit_alloca_aligned_addr_impl(slot, id);
                                    self.state.emit("    fldt (%rcx)");
                                }
                                SlotAddr::Indirect(slot) => {
                                    self.emit_load_ptr_from_slot_impl(slot, ptr_id);
                                    self.state.emit("    fldt (%rcx)");
                                }
                                SlotAddr::Reg(reg) => {
                                    self.emit_reg_to_addr(reg);
                                    self.state.emit("    fldt (%rcx)");
                                }
                            }
                            self.emit_epilogue_and_ret_impl(frame_size);
                            return;
                        }
                    }
                }
                if let Operand::Const(IrConst::LongDouble(_, f128_bytes)) = val {
                    let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(f128_bytes);
                    self.state.emit("    subq $16, %rsp");
                    let lo = u64::from_le_bytes(x87[0..8].try_into().unwrap());
                    let hi = u16::from_le_bytes(x87[8..10].try_into().unwrap());
                    self.state
                        .out
                        .emit_instr_imm_reg("    movabsq", lo as i64, "rax");
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", hi as i64, "rax");
                    self.state.emit("    movq %rax, 8(%rsp)");
                    self.state.emit("    fldt (%rsp)");
                    self.state.emit("    addq $16, %rsp");
                    self.emit_epilogue_and_ret_impl(frame_size);
                    return;
                }
            }
        }
        // A 32-bit-or-smaller integer returned from a function is represented in
        // %eax with bits 32..63 zeroed (SysV AMD64).  The generic loader uses
        // 64-bit moves for GPR homes, which leaves artificial 64-bit copies in
        // tiny boolean/classifier functions.  Emit the zero-extending 32-bit
        // load directly; this matches GCC/Clang and avoids exposing a 64-bit
        // return live range to the register allocator.
        let ret_ty = self.current_return_type;
        if ret_ty.is_integer() && ret_ty.size() <= 4 {
            if let Some(val) = val {
                self.operand_to_rax_i32(val);
            }
            self.emit_epilogue_and_ret_impl(frame_size);
            return;
        }
        // IS-29a: scalar FP returns go straight into %xmm0 through the FP
        // operand loader — one move from an XMM home (`vmovsd %xmmN, %xmm0`)
        // or a direct slot/constant load. The generic default routes the
        // value through the GPR accumulator (`operand_to_rax` +
        // `movq %rax, %xmm0`), bouncing the double through %rax (or a stack
        // slot) on every FP return — the libm round-family regression class
        // (PERF-1: 2.99× GCC on `vroundsd` wrappers).
        if matches!(
            ret_ty,
            IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64
        ) {
            if let Some(val) = val {
                self.load_fp_to_xmm0(val, ret_ty);
            }
            self.emit_epilogue_and_ret_impl(frame_size);
            return;
        }
        // One SSE eightbyte (`struct { float x, y; }`, `struct { double }`,
        // an 8-byte vector): the aggregate's bits, carried in the IR as a
        // packed integer, leave in the low half of %xmm0 -- not %rax. A
        // 64-bit carrier moves straight from its register or slot; a narrower
        // one is widened through the accumulator first.
        if self.func_ret_classes[..] == [crate::common::types::EightbyteClass::Sse]
            && !ret_ty.is_float()
            && !ret_ty.is_128bit()
        {
            if let Some(val) = val {
                if matches!(ret_ty, IrType::I64 | IrType::U64 | IrType::Ptr) {
                    self.load_bits64_to_xmm0(val);
                } else {
                    self.operand_to_rax(val);
                    self.state.emit("    movq %rax, %xmm0");
                }
            }
            self.emit_epilogue_and_ret_impl(frame_size);
            return;
        }
        crate::backend::traits::emit_return_default(self, val, frame_size);
    }

    pub(super) fn current_return_type_impl(&self) -> IrType {
        self.current_return_type
    }

    pub(super) fn emit_return_f32_to_reg_impl(&mut self) {
        self.state.emit("    movd %eax, %xmm0");
    }

    pub(super) fn emit_return_f64_to_reg_impl(&mut self) {
        self.state.emit("    movq %rax, %xmm0");
    }

    pub(super) fn emit_return_i128_to_regs_impl(&mut self) {
        // For pure INTEGER+INTEGER returns, rax:rdx already correct — noop.
        // For mixed INTEGER+SSE or SSE+INTEGER returns, move the SSE eightbyte
        // from the GP register to xmm0 per SysV AMD64 ABI.
        // Use func_ret_classes (set once in prologue) so that intervening call
        // instructions cannot clobber the function's own return classification.
        use crate::common::types::EightbyteClass;
        if self.func_ret_classes.len() == 2 {
            let (c0, c1) = (self.func_ret_classes[0], self.func_ret_classes[1]);
            match (c0, c1) {
                (EightbyteClass::Integer, EightbyteClass::Sse) => {
                    // Second eightbyte is SSE: move rdx -> xmm0
                    self.state.emit("    movq %rdx, %xmm0");
                }
                (EightbyteClass::Sse, EightbyteClass::Integer) => {
                    // First eightbyte is SSE: rax -> xmm0, rdx -> rax
                    self.state.emit("    movq %rax, %xmm0");
                    self.state.emit("    movq %rdx, %rax");
                }
                (EightbyteClass::Sse, EightbyteClass::Sse) => {
                    // Both SSE: rax -> xmm0, rdx -> xmm1. When the lowering
                    // routed the high eightbyte through an explicit
                    // SetReturnF64Second (IS-02 all-SSE struct returns),
                    // xmm1 already holds it and %rdx is zero — write xmm0
                    // only.
                    self.state.emit("    movq %rax, %xmm0");
                    if !self.func_set_second_ret {
                        self.state.emit("    movq %rdx, %xmm1");
                    }
                }
                _ => {} // INTEGER+INTEGER: already correct
            }
        }
    }

    pub(super) fn emit_return_f128_to_reg_impl(&mut self) {
        // F128 (long double) must be returned in x87 st(0) per SysV ABI.
        // rax has f64 bit pattern; push to stack, load with fldl.
        self.state.emit("    pushq %rax");
        self.state.emit("    fldl (%rsp)");
        self.state.emit("    addq $8, %rsp");
    }

    pub(super) fn emit_return_int_to_reg_impl(&mut self) {
        // rax already holds the return value per SysV ABI — noop
    }

    pub(super) fn emit_get_return_f64_second_impl(&mut self, dest: &Value) {
        self.store_xmm_to(dest, "xmm1", IrType::F64);
    }

    pub(super) fn emit_set_return_f64_second_impl(&mut self, src: &Operand) {
        // The imaginary half of `_Complex double` is returned in %xmm1.  The
        // old implementation only handled stack-resident SSA values; if the
        // imaginary value was register-allocated (the optimized common case),
        // it emitted nothing and left %xmm1 stale/uninitialized.  Route through
        // the normal FP operand loader so XMM/GPR/slot/constant sources are all
        // handled identically to call arguments and comparisons.
        self.emit_fp_operand_to_xmm(src, IrType::F64, "xmm1");
        self.state.reg_cache.invalidate_all();
        // Record that xmm1 now carries an explicit second-half value so the
        // [Sse, Sse] two-eightbyte primary return does not clobber it with
        // %rdx (IS-02 SSE-class struct returns).
        self.func_set_second_ret = true;
    }

    pub(super) fn emit_get_return_f32_second_impl(&mut self, dest: &Value) {
        self.store_xmm_to(dest, "xmm1", IrType::F32);
    }

    pub(super) fn emit_set_return_f32_second_impl(&mut self, src: &Operand) {
        self.emit_fp_operand_to_xmm(src, IrType::F32, "xmm1");
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_get_return_f128_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.state.out.emit_instr_rbp("    fstpt", slot.0);
            self.state.out.emit_instr_rbp("    fldt", slot.0);
            self.state.emit("    subq $8, %rsp");
            self.state.emit("    fstpl (%rsp)");
            self.state.emit("    popq %rax");
            self.state.park_acc(dest.0, false);
            self.state.f128_direct_slots.insert(dest.0);
        } else {
            self.state.emit("    fstp %st(0)");
        }
    }

    pub(super) fn emit_set_return_f128_second_impl(&mut self, src: &Operand) {
        match src {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.state.out.emit_instr_rbp("    fldt", slot.0);
                }
            }
            _ => {
                self.operand_to_rax(src);
                self.state.emit("    pushq %rax");
                self.state.emit("    fildq (%rsp)");
                self.state.emit("    addq $8, %rsp");
            }
        }
    }
}
