//! X86Codegen: memory operations (load, store, memcpy, GEP, stack).

use super::emit::{is_xmm_reg, phys_reg_name, phys_reg_name_32, typed_phys_reg_name, X86Codegen};
use crate::backend::state::{SlotAddr, StackSlot};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, Operand, Value};

/// Printable immediate for a direct `movX $imm, <mem>` store that stages
/// nothing through %rax.
///
/// x86 stores take an immediate field of the STORE's width: `movb`/`movw`/
/// `movl` encode raw {8,16,32}-bit values (the same raw-field width contract
/// the MachInst emitter applies on its narrow fast path), so for every store
/// width <= 4 bytes ANY constant is directly encodable once truncated to the
/// destination width — including unsigned 32-bit constants like 3041712678
/// whose value sits above `i32::MAX` (the historical gate here rejected
/// them, forcing the `movabsq $imm,%rax; movl %eax,addr` relay that PR #364's
/// direct path only removed for the MachInst side). A 64-bit store cannot
/// take a full imm64 to memory, so it still needs the sign-extended imm32
/// form or the accumulator relay.
pub(super) fn direct_store_imm(imm: i64, ty: IrType) -> Option<i64> {
    if ty.is_float() || matches!(ty, IrType::I128 | IrType::U128 | IrType::F128) {
        return None;
    }
    if ty.size() >= 8 {
        // 64-bit destinations: only `movq $imm32` (sign-extended) is legal.
        return (imm >= i32::MIN as i64 && imm <= i32::MAX as i64).then_some(imm);
    }
    // Narrow destinations: keep the historical signed spelling when the
    // value fits an imm32 (small negatives print like GCC's `movl $-1`),
    // otherwise print the value truncated to the destination's raw field.
    if imm >= i32::MIN as i64 && imm <= i32::MAX as i64 {
        Some(imm)
    } else {
        let mask: i64 = match ty.size() {
            1 => 0xff,
            2 => 0xffff,
            _ => 0xffff_ffff,
        };
        Some(imm & mask)
    }
}

impl X86Codegen {
    /// S11: soundness gate for raw SIB-index use of `reg_assignments`.
    ///
    /// `build_indexed_gep_map` peels widening casts, `add(iv, const)` and
    /// power-of-two scale chains off a GEP offset, so the value feeding the
    /// SIB index can be NARROWER than 64 bits (e.g. an I32 call result reached
    /// by peeling the `Cast`+`Add` off `(i64)idx + 1` in `k[baz()+1]`).
    /// Narrow values have no defined 64-bit home form: `store_rax_to` /
    /// `store_eax_to` deliberately keep the whole accumulator (`movq`), and
    /// after a Call returning I32 the upper half of %rax is undefined
    /// (System V names only %eax as the return register). A SIB index reads
    /// the home register directly — no reload, no `movslq` — so the peel
    /// turns `p[idx]` into `mem(%base, %idx_home, scale)` with garbage in the
    /// upper half (pr110115: `16(%rsp,%r9,8)` with %r9 = 0x00000000FFFFFFFF
    /// instead of the required sign extension 0xFFFFFFFFFFFFFFFF ⇒ wild
    /// store ⇒ SIGSEGV).
    ///
    /// Values that appear in a 64-bit IR position are already covered: they
    /// land in `needs_sext_values`, whose def-side policy emits the
    /// extending move after the 32-bit op, so their homes carry the correct
    /// 64-bit form and this helper is a no-op for them (it re-extends the
    /// same low bits). The peeled index never reaches such a position, which
    /// is exactly the hole this closes.
    ///
    /// The extension happens IN PLACE on the home register, immediately
    /// before the SIB operand is consumed:
    ///   I8/I16/I32 → movsbq/movswq/movslq from the narrow sub-register
    ///   U8/U16/U32 → movzbq/movzwq/movl (zero-extension by definition)
    /// In-place extension is always sound for a narrow value:
    ///   * the SIB now reads exactly the 64-bit form the type promises;
    ///   * later 32-bit reloads (`movl %rNd`) still read the low 32 bits;
    ///   * later sext-aware reloads re-extend from the low bits, unchanged;
    ///   * the home only gains DEFINED upper bits — every form above
    ///     preserves the low bits bit-exactly and MOV never touches flags
    ///     (safe even with a fused-Cmp handshake pending).
    ///
    /// Returns false (caller refuses the fold) when the index has no register
    /// home, is XMM-homed, or its IR type is not statically narrow: extending
    /// a genuine 64-bit value in place would truncate it, and an unknown type
    /// must not be gambled on. Refusals fall back to the `leaq` path, which
    /// reloads the index through the sext-aware operand machinery.
    fn ensure_sib_index_form(&mut self, index: &Value) -> bool {
        let Some(&reg) = self.reg_assignments.get(&index.0) else {
            return false;
        };
        if is_xmm_reg(reg) {
            return false;
        }
        let Some(&ty) = self.value_types.get(&index.0) else {
            return false;
        };
        match ty {
            IrType::I64 | IrType::U64 | IrType::Ptr => true,
            IrType::I32 | IrType::U32 | IrType::I16 | IrType::U16 | IrType::I8 | IrType::U8 => {
                let r64 = phys_reg_name(reg);
                let instr = match ty {
                    IrType::I32 => format!("    movslq %{}, %{}", phys_reg_name_32(reg), r64),
                    IrType::U32 => format!(
                        "    movl %{}, %{}",
                        phys_reg_name_32(reg),
                        phys_reg_name_32(reg)
                    ),
                    IrType::I16 => format!(
                        "    movswq %{}, %{}",
                        typed_phys_reg_name(reg, IrType::I16),
                        r64
                    ),
                    IrType::U16 => format!(
                        "    movzwq %{}, %{}",
                        typed_phys_reg_name(reg, IrType::U16),
                        r64
                    ),
                    IrType::I8 => format!(
                        "    movsbq %{}, %{}",
                        typed_phys_reg_name(reg, IrType::I8),
                        r64
                    ),
                    IrType::U8 => format!(
                        "    movzbq %{}, %{}",
                        typed_phys_reg_name(reg, IrType::U8),
                        r64
                    ),
                    _ => unreachable!("narrow-type match is exhaustive above"),
                };
                self.state.emit_fmt(format_args!("{}", instr));
                true
            }
            _ => false,
        }
    }

    /// Try to emit a store using x86-64 SIB indexed addressing mode.
    /// Returns true if successful, false to fall back to normal codegen.
    ///
    /// First tries IVSR pattern detection (Phase 9b), then falls back to
    /// Phase 9 pattern (GEP with Mul/Shl offset).
    fn try_emit_indexed_store(&mut self, val: &Operand, ptr: &Value, ty: IrType) -> bool {
        // Phase 9b: Try IVSR pointer pattern first (most common in loops)
        if self.try_emit_ivsr_indexed_store(val, ptr, ty) {
            return true;
        }

        // Phase 9: Try non-IVSR pattern (explicit multiply/shift)
        self.try_emit_phase9_indexed_store(val, ptr, ty)
    }

    /// Try to emit indexed addressing for IVSR-transformed loop pointer stores.
    fn try_emit_ivsr_indexed_store(&mut self, val: &Operand, ptr: &Value, ty: IrType) -> bool {
        // Check if ptr is an IVSR pointer phi
        let ivsr_info = match self.ivsr_pointers.get(&ptr.0) {
            Some(info) => info.clone(),
            None => return false,
        };

        // Find the loop counter associated with this pointer
        let counter_val = match self.pointer_to_counter.get(&ptr.0) {
            Some(&counter_id) => Value(counter_id),
            None => return false,
        };

        // Check if both base and counter are in registers
        let base_reg = match self.reg_assignments.get(&ivsr_info.base_ptr.0) {
            Some(&reg) => phys_reg_name(reg),
            None => return false,
        };

        let index_reg = match self.reg_assignments.get(&counter_val.0) {
            Some(&reg) => phys_reg_name(reg),
            None => return false,
        };

        // Verify stride is a valid SIB scale
        if !Self::is_valid_sib_scale(ivsr_info.stride) {
            return false;
        }

        // Base and index must be different registers.
        if base_reg == index_reg {
            return false;
        }

        // Check if loading the store value would clobber base or index register
        if let Operand::Value(v) = val {
            if let Some(&val_reg) = self.reg_assignments.get(&v.0) {
                let val_name = phys_reg_name(val_reg);
                if val_name == base_reg || val_name == index_reg {
                    return false;
                }
            }
        }

        // Load the value to be stored into the accumulator/xmm register.
        // FP constants load directly from the rodata constant pool into xmm0.
        let fp_const = matches!(
            val,
            Operand::Const(IrConst::F64(_) | IrConst::F32(_) | IrConst::D64(_) | IrConst::D32(_))
        ) && matches!(ty, IrType::F64 | IrType::F32 | IrType::D64 | IrType::D32);
        if fp_const {
            self.emit_fp_operand_to_xmm(val, ty, "xmm0");
        } else {
            self.operand_to_rax(val);
        }

        // Determine store instruction and source register based on type
        let (store_instr, src_reg) = match ty {
            IrType::F64 => {
                if !fp_const {
                    // Convert from rax to xmm0
                    self.state.emit("    movq %rax, %xmm0");
                }
                ("movsd", "%xmm0")
            }
            IrType::F32 => {
                if !fp_const {
                    // Convert from rax to xmm0
                    self.state.emit("    movd %eax, %xmm0");
                }
                ("movss", "%xmm0")
            }
            IrType::I64 | IrType::U64 => ("movq", "%rax"),
            IrType::I32 | IrType::U32 => ("movl", "%eax"),
            IrType::I16 | IrType::U16 => ("movw", "%ax"),
            IrType::I8 | IrType::U8 => ("movb", "%al"),
            _ => return false,
        };

        // Emit indexed store: movX %src, (%base,%index,scale)
        if ivsr_info.init_offset == 0 {
            self.state.emit_fmt(format_args!(
                "    {} {}, (%{},%{},{})",
                store_instr, src_reg, base_reg, index_reg, ivsr_info.stride
            ));
        } else {
            self.state.emit_fmt(format_args!(
                "    {} {}, {}(%{},%{},{})",
                store_instr, src_reg, ivsr_info.init_offset, base_reg, index_reg, ivsr_info.stride
            ));
        }

        true
    }

    /// Phase 9 indexed addressing: detect GEP with multiply/shift offset for stores.
    /// Detects patterns like: `store val, (base + index*scale)` where:
    /// - base is in a register
    /// - index is in a register
    /// - scale is 1, 2, 4, or 8
    ///
    /// Emits: `mov %src, (%base_reg,%index_reg,scale)`
    fn try_emit_phase9_indexed_store(&mut self, val: &Operand, ptr: &Value, ty: IrType) -> bool {
        // Phase 9 decomposes a variable-offset GEP into SIB addressing:
        //   Store val, (GEP base, Mul(idx, scale))  →  movl %eax, (%base, %idx, scale)
        // However, variable-offset GEPs are always emitted as `leaq` instructions
        // (they are NOT in gep_fold_map, which only handles constant offsets).
        // By the time the Store is emitted, the GEP's source registers (base, idx)
        // may have been clobbered by the leaq destination or intervening instructions.
        // The GEP result is already computed in a register/slot, so use it directly.
        return false;

        // Check if ptr is defined by a GEP instruction
        let gep_inst = match self.get_defining_instruction(ptr.0) {
            Some(inst) => inst,
            None => return false,
        };

        let (gep_base, gep_offset) = match gep_inst {
            Instruction::GetElementPtr { base, offset, .. } => (base, offset),
            _ => return false,
        };

        // Check if offset is a Value (not a constant - those are handled by existing GEP folding)
        let offset_val = match gep_offset {
            Operand::Value(v) => v,
            _ => return false,
        };

        // Check if offset is defined by a multiply or shift (i*scale pattern)
        let (index_val, scale) = match self.get_defining_instruction(offset_val.0) {
            Some(Instruction::BinOp {
                op: IrBinOp::Mul,
                lhs: Operand::Value(idx),
                rhs: Operand::Const(c),
                ..
            }) => {
                // Pattern: index * const
                let scale_val = match c.to_i64() {
                    Some(v) => v,
                    None => return false,
                };
                if !Self::is_valid_sib_scale(scale_val) {
                    return false;
                }
                (idx, scale_val)
            }
            Some(Instruction::BinOp {
                op: IrBinOp::Shl,
                lhs: Operand::Value(idx),
                rhs: Operand::Const(c),
                ..
            }) => {
                // Pattern: index << shift_amount (equivalent to index * 2^shift)
                let shift = match c.to_i64() {
                    Some(v) if v >= 0 && v <= 3 => v, // shift of 0-3 gives scale of 1,2,4,8
                    _ => return false,
                };
                let scale_val = 1i64 << shift;
                (idx, scale_val)
            }
            _ => return false,
        };

        // Check if base and index both have register assignments
        let base_reg = match self.reg_assignments.get(&gep_base.0) {
            Some(&reg) => phys_reg_name(reg),
            None => return false,
        };

        let index_reg = match self.reg_assignments.get(&index_val.0) {
            Some(&reg) => phys_reg_name(reg),
            None => return false,
        };

        // Base and index must be different registers for SIB addressing.
        // If the register allocator assigned both the same register (e.g.,
        // because one value's live range ended and the register was reused),
        // the SIB computation would be wrong (base + base*scale instead of
        // base + index*scale).
        if base_reg == index_reg {
            return false;
        }

        // Check if loading the store value would clobber the base or index
        // register. This happens when the store value's register overlaps with
        // the base/index, or when operand_to_rax needs to use the register for
        // intermediate computations. If so, fall back to non-indexed store.
        if let Operand::Value(v) = val {
            if let Some(&val_reg) = self.reg_assignments.get(&v.0) {
                let val_name = phys_reg_name(val_reg);
                if val_name == base_reg || val_name == index_reg {
                    return false; // Register conflict, fall back
                }
            }
        }

        // Load the value to be stored into the accumulator/xmm register.
        // FP constants load directly from the rodata constant pool into xmm0.
        let fp_const = matches!(val, Operand::Const(IrConst::F64(_) | IrConst::F32(_)))
            && matches!(ty, IrType::F64 | IrType::F32);
        if fp_const {
            self.emit_fp_operand_to_xmm(val, ty, "xmm0");
        } else {
            self.operand_to_rax(val);
        }

        // Determine store instruction and source register based on type
        let (store_instr, src_reg) = match ty {
            IrType::F64 => {
                if !fp_const {
                    // Convert from rax to xmm0
                    self.state.emit("    movq %rax, %xmm0");
                }
                ("movsd", "%xmm0")
            }
            IrType::F32 => {
                if !fp_const {
                    // Convert from rax to xmm0
                    self.state.emit("    movd %eax, %xmm0");
                }
                ("movss", "%xmm0")
            }
            IrType::I64 | IrType::U64 => ("movq", "%rax"),
            IrType::I32 | IrType::U32 => ("movl", "%eax"),
            IrType::I16 | IrType::U16 => ("movw", "%ax"),
            IrType::I8 | IrType::U8 => ("movb", "%al"),
            _ => return false, // Unsupported type for indexed addressing
        };

        // Emit indexed store: movX %src, (%base,%index,scale)
        self.state.emit_fmt(format_args!(
            "    {} {}, (%{},%{},{})",
            store_instr, src_reg, base_reg, index_reg, scale
        ));

        true
    }

    /// Try to emit a load using x86-64 SIB indexed addressing mode.
    /// Returns true if successful, false to fall back to normal codegen.
    ///
    /// First tries IVSR pattern detection (Phase 9b), then falls back to
    /// Phase 9 pattern (GEP with Mul/Shl offset).
    fn try_emit_indexed_load(&mut self, dest: &Value, ptr: &Value, ty: IrType) -> bool {
        // Phase 9b: Try IVSR pointer pattern first (most common in loops)
        if self.try_emit_ivsr_indexed_load(dest, ptr, ty) {
            return true;
        }

        // Phase 9: Try non-IVSR pattern (explicit multiply/shift)
        self.try_emit_phase9_indexed_load(dest, ptr, ty)
    }

    /// Try to emit indexed addressing for IVSR-transformed loop pointers.
    /// Detects pattern: %ptr = Phi(%init, %next) where %next = GEP(%ptr, stride)
    /// and emits: movX (%base,%counter,scale), %dest
    fn try_emit_ivsr_indexed_load(&mut self, dest: &Value, ptr: &Value, ty: IrType) -> bool {
        // Check if ptr is an IVSR pointer phi
        let ivsr_info = match self.ivsr_pointers.get(&ptr.0) {
            Some(info) => info.clone(),
            None => return false,
        };

        // Find the loop counter associated with this pointer
        let counter_val = match self.pointer_to_counter.get(&ptr.0) {
            Some(&counter_id) => Value(counter_id),
            None => return false,
        };

        // Check if both base and counter are in registers
        let base_reg = match self.reg_assignments.get(&ivsr_info.base_ptr.0) {
            Some(&reg) => phys_reg_name(reg),
            None => return false,
        };

        let index_reg = match self.reg_assignments.get(&counter_val.0) {
            Some(&reg) => phys_reg_name(reg),
            None => return false,
        };

        // Verify stride is a valid SIB scale
        if !Self::is_valid_sib_scale(ivsr_info.stride) {
            return false;
        }

        // Determine load instruction and destination register based on type
        let (load_instr, dest_reg) = match ty {
            IrType::F64 => ("movsd", "%xmm0"),
            IrType::F32 => ("movss", "%xmm0"),
            IrType::I64 | IrType::U64 => ("movq", "%rax"),
            IrType::I32 | IrType::U32 => ("movl", "%eax"),
            IrType::I16 | IrType::U16 => ("movzwl", "%eax"),
            IrType::I8 | IrType::U8 => ("movzbl", "%eax"),
            _ => return false,
        };

        // Emit indexed load: movX (%base,%index,scale), %dest
        // Handle optional displacement for non-zero init_offset
        if ivsr_info.init_offset == 0 {
            self.state.emit_fmt(format_args!(
                "    {} (%{},%{},{}), {}",
                load_instr, base_reg, index_reg, ivsr_info.stride, dest_reg
            ));
        } else {
            self.state.emit_fmt(format_args!(
                "    {} {}(%{},%{},{}), {}",
                load_instr, ivsr_info.init_offset, base_reg, index_reg, ivsr_info.stride, dest_reg
            ));
        }

        // Update register cache - for FP types, value is in xmm0, for integers in rax
        match ty {
            IrType::F64 | IrType::F32 | IrType::D64 | IrType::D32 => {
                // For floating point, the value is in xmm0, not rax
                // We need to move it to rax for the common code path
                if matches!(ty, IrType::F64 | IrType::D64) {
                    self.state.emit("    movq %xmm0, %rax");
                } else {
                    self.state.emit("    movd %xmm0, %eax");
                }
                self.state.reg_cache.set_acc(dest.0, false);
            }
            _ => {
                // Integer types are already in rax
                self.state.reg_cache.set_acc(dest.0, false);
            }
        }

        true
    }

    /// Phase 9 indexed addressing: detect GEP with multiply/shift offset.
    /// Detects patterns like: `load (base + index*scale)` where:
    /// - base is in a register
    /// - index is in a register
    /// - scale is 1, 2, 4, or 8
    ///
    /// Emits: `mov (%base_reg,%index_reg,scale), %dest`
    fn try_emit_phase9_indexed_load(&mut self, dest: &Value, ptr: &Value, ty: IrType) -> bool {
        // Disabled: same issue as try_emit_phase9_indexed_store — variable-offset
        // GEPs are already emitted, so base/index registers may be stale.
        return false;

        // Check if ptr is defined by a GEP instruction
        let gep_inst = match self.get_defining_instruction(ptr.0) {
            Some(inst) => inst,
            None => return false,
        };

        let (gep_base, gep_offset) = match gep_inst {
            Instruction::GetElementPtr { base, offset, .. } => (base, offset),
            _ => return false,
        };

        // Check if offset is a Value (not a constant - those are handled by existing GEP folding)
        let offset_val = match gep_offset {
            Operand::Value(v) => v,
            _ => return false,
        };

        // Check if offset is defined by a multiply or shift (i*scale pattern)
        let (index_val, scale) = match self.get_defining_instruction(offset_val.0) {
            Some(Instruction::BinOp {
                op: IrBinOp::Mul,
                lhs: Operand::Value(idx),
                rhs: Operand::Const(c),
                ..
            }) => {
                // Pattern: index * const
                let scale_val = match c.to_i64() {
                    Some(v) => v,
                    None => return false,
                };
                if !Self::is_valid_sib_scale(scale_val) {
                    return false;
                }
                (idx, scale_val)
            }
            Some(Instruction::BinOp {
                op: IrBinOp::Shl,
                lhs: Operand::Value(idx),
                rhs: Operand::Const(c),
                ..
            }) => {
                // Pattern: index << shift_amount (equivalent to index * 2^shift)
                let shift = match c.to_i64() {
                    Some(v) if v >= 0 && v <= 3 => v, // shift of 0-3 gives scale of 1,2,4,8
                    _ => return false,
                };
                let scale_val = 1i64 << shift;
                (idx, scale_val)
            }
            _ => return false,
        };

        // Check if base and index both have register assignments
        let base_reg = match self.reg_assignments.get(&gep_base.0) {
            Some(&reg) => phys_reg_name(reg),
            None => return false,
        };

        let index_reg = match self.reg_assignments.get(&index_val.0) {
            Some(&reg) => phys_reg_name(reg),
            None => return false,
        };

        // Determine load instruction and destination register based on type
        let (load_instr, dest_reg) = match ty {
            IrType::F64 => ("movsd", "%xmm0"),
            IrType::F32 => ("movss", "%xmm0"),
            IrType::I64 | IrType::U64 => ("movq", "%rax"),
            IrType::I32 | IrType::U32 => ("movl", "%eax"),
            IrType::I16 | IrType::U16 => ("movzwl", "%eax"),
            IrType::I8 | IrType::U8 => ("movzbl", "%eax"),
            _ => return false, // Unsupported type for indexed addressing
        };

        // Emit indexed load: movX (%base,%index,scale), %dest
        self.state.emit_fmt(format_args!(
            "    {} (%{},%{},{}), {}",
            load_instr, base_reg, index_reg, scale, dest_reg
        ));

        // Update register cache - for FP types, value is in xmm0, for integers in rax
        match ty {
            IrType::F64 | IrType::F32 | IrType::D64 | IrType::D32 => {
                // For floating point, the value is in xmm0, not rax
                // We need to move it to rax for the common code path
                if matches!(ty, IrType::F64 | IrType::D64) {
                    self.state.emit("    movq %xmm0, %rax");
                } else {
                    self.state.emit("    movd %xmm0, %eax");
                }
                self.state.reg_cache.set_acc(dest.0, false);
            }
            _ => {
                // Integer types are already in rax
                self.state.reg_cache.set_acc(dest.0, false);
            }
        }

        true
    }

    // ---- Store/Load overrides ----

    pub(super) fn emit_store_impl(&mut self, val: &Operand, ptr: &Value, ty: IrType) {
        if ty == IrType::F128 {
            if let Operand::Const(IrConst::LongDouble(_, f128_bytes)) = val {
                let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(f128_bytes);
                let lo = u64::from_le_bytes(x87[0..8].try_into().unwrap());
                let hi_bytes: [u8; 8] = [x87[8], x87[9], 0, 0, 0, 0, 0, 0];
                let hi = u64::from_le_bytes(hi_bytes);
                if let Some(addr) = self.state.resolve_slot_addr(ptr.0) {
                    self.emit_f128_store_raw_bytes(&addr, ptr.0, 0, lo, hi);
                }
                return;
            }
            if let Operand::Const(IrConst::I128(c)) = val {
                // _Float128 constant as a 16-byte bit pattern (huge_valf128 etc.).
                let bytes = (*c as u128).to_le_bytes();
                let lo = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                let hi = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                if let Some(addr) = self.state.resolve_slot_addr(ptr.0) {
                    self.emit_f128_store_raw_bytes(&addr, ptr.0, 0, lo, hi);
                }
                return;
            }
            if let Operand::Value(v) = val {
                if self.state.f128_direct_slots.contains(&v.0) {
                    if let Some(src_slot) = self.state.get_slot(v.0) {
                        if let Some(dest_addr) = self.state.resolve_slot_addr(ptr.0) {
                            self.state.out.emit_instr_rbp("    fldt", src_slot.0);
                            self.emit_f128_fstpt(&dest_addr, ptr.0, 0);
                            return;
                        }
                    }
                }
            }
            self.operand_to_rax(val);
            if let Some(addr) = self.state.resolve_slot_addr(ptr.0) {
                self.emit_f128_store_f64_via_x87(&addr, ptr.0, 0);
            }
            return;
        }

        // Try indexed addressing mode first (Phase 9 optimization)
        if self.try_emit_indexed_store(val, ptr, ty) {
            // Indexed addressing succeeded - we're done!
            return;
        }

        // Scalar FP store: keep the value in the SSE domain instead of
        // round-tripping through %rax.
        if matches!(ty, IrType::F64 | IrType::F32 | IrType::D64 | IrType::D32) {
            // Decimal carriers: D64 moves like a 64-bit double, D32 like a
            // 32-bit float — bit-exact SSE moves, never a value conversion.
            let store_instr = if matches!(ty, IrType::F64 | IrType::D64) {
                "    movsd"
            } else {
                "    movss"
            };
            let addr = self.state.resolve_slot_addr(ptr.0);
            match addr {
                Some(SlotAddr::Direct(slot)) => {
                    let src = self.fp_store_value_xmm(val, ty);
                    self.state.out.emit_instr_reg_rbp(store_instr, src, slot.0);
                    return;
                }
                Some(SlotAddr::Indirect(slot)) => {
                    if let Some(&reg) = self.reg_assignments.get(&ptr.0) {
                        if !is_xmm_reg(reg) {
                            let reg_name = phys_reg_name(reg);
                            let src = self.fp_store_value_xmm(val, ty);
                            self.state.emit_fmt(format_args!(
                                "{} %{}, (%{})",
                                store_instr, src, reg_name
                            ));
                            return;
                        }
                    }
                    self.emit_load_ptr_from_slot_impl(slot, ptr.0);
                    let src = self.fp_store_value_xmm(val, ty);
                    self.state
                        .emit_fmt(format_args!("{} %{}, (%rcx)", store_instr, src));
                    return;
                }
                Some(SlotAddr::OverAligned(slot, id)) => {
                    self.emit_alloca_aligned_addr_impl(slot, id);
                    let src = self.fp_store_value_xmm(val, ty);
                    self.state
                        .emit_fmt(format_args!("{} %{}, (%rcx)", store_instr, src));
                    return;
                }
                Some(SlotAddr::Reg(reg)) => {
                    let src = self.fp_store_value_xmm(val, ty);
                    self.state.emit_fmt(format_args!(
                        "{} %{}, (%{})",
                        store_instr,
                        src,
                        phys_reg_name(reg)
                    ));
                    return;
                }
                None => {} // fall through to default
            }
        }

        // Constant-immediate store optimization: when storing a small constant,
        // emit `movX $IMM, ADDR` directly instead of loading the constant into
        // %rax and then storing through the accumulator. This saves 1-3 instructions
        // (eliminates xorl/movq for zero, or movq $IMM for other constants, plus
        // the emit_save_acc movq %rax, %rdx for indirect stores).
        if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128 | IrType::F128) {
            if let Operand::Const(c) = val {
                if let Some(imm) = c.to_i64() {
                    // Direct `movX $imm, ADDR` is legal whenever the value
                    // fits the store's immediate field — for narrow stores
                    // that is EVERY constant (see `direct_store_imm`), which
                    // removes the `movabsq $imm,%rax; movX %eax,ADDR` relay
                    // for unsigned 32-bit constants above i32::MAX too.
                    if let Some(imm_print) = direct_store_imm(imm, ty) {
                        if self.try_emit_const_store(imm_print, ptr, ty) {
                            return;
                        }
                    }
                }
            }
        }

        // Register-direct fast path: bypass the accumulator when both val
        // and ptr have register assignments. Saves 2-3 instructions per store.
        if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128 | IrType::F128) {
            if let Operand::Value(v) = val {
                let v_reg = self.reg_assignments.get(&v.0).copied();
                let p_reg = self.reg_assignments.get(&ptr.0).copied();

                // Both val and ptr in GPR registers: emit 1 instruction
                if let (Some(vr), Some(pr)) = (v_reg, p_reg) {
                    if !is_xmm_reg(vr) && !is_xmm_reg(pr) && !self.state.is_alloca(ptr.0) {
                        let store_instr = Self::mov_store_for_type(ty);
                        let v_name = typed_phys_reg_name(vr, ty);
                        let p_name = phys_reg_name(pr);
                        self.state.emit_fmt(format_args!(
                            "    {} %{}, (%{})",
                            store_instr, v_name, p_name
                        ));
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                }

                // Val in register, ptr on stack: 2 instructions (skip emit_save_acc)
                if let Some(vr) = v_reg {
                    if !is_xmm_reg(vr) && !self.state.is_alloca(ptr.0) {
                        if let Some(crate::backend::state::SlotAddr::Indirect(slot)) =
                            self.state.resolve_slot_addr(ptr.0)
                        {
                            self.emit_load_ptr_from_slot_impl(slot, ptr.0);
                            let store_instr = Self::mov_store_for_type(ty);
                            let v_name = typed_phys_reg_name(vr, ty);
                            self.state
                                .emit_fmt(format_args!("    {} %{}, (%rcx)", store_instr, v_name));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                    }
                }
            }
        }

        // register-direct F64/F32 store. When the value lives in an
        // XMM-allocated register, store it straight to memory (movsd/movss
        // %xmmN, (%ptr)) instead of the GPR round-trip (movq %xmmN,%rax;
        // movq %rax,(%ptr)) the default path pays on every store.
        if ty == IrType::F64 || ty == IrType::F32 {
            if let Operand::Value(v) = val {
                if let Some(v_reg) = self.reg_assignments.get(&v.0).copied() {
                    if is_xmm_reg(v_reg) {
                        let v_name = phys_reg_name(v_reg);
                        let mov = if ty == IrType::F64 { "movsd" } else { "movss" };
                        // ptr in a GPR-allocated register
                        if let Some(p_reg) = self.reg_assignments.get(&ptr.0).copied() {
                            if !is_xmm_reg(p_reg) && !self.state.is_alloca(ptr.0) {
                                let p_name = phys_reg_name(p_reg);
                                self.state.emit_fmt(format_args!(
                                    "    {} %{}, (%{})",
                                    mov, v_name, p_name
                                ));
                                self.state.reg_cache.invalidate_acc();
                                return;
                            }
                        }
                        // ptr is an alloca with a direct slot
                        if self.state.is_alloca(ptr.0)
                            && self.state.alloca_over_align(ptr.0).is_none()
                        {
                            if let Some(slot) = self.state.get_slot(ptr.0) {
                                let sr = self.slot_ref(slot.0);
                                self.state
                                    .emit_fmt(format_args!("    {} %{}, {}", mov, v_name, sr));
                                self.state.reg_cache.invalidate_acc();
                                return;
                            }
                        }
                        // general pointer: materialize to %rax and store
                        self.operand_to_reg(&Operand::Value(*ptr), "rax");
                        self.state
                            .emit_fmt(format_args!("    {} %{}, (%rax)", mov, v_name));
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                }
            }
        }

        // Fall back to default store logic
        crate::backend::traits::emit_store_default(self, val, ptr, ty);
    }

    /// Try to emit a constant-immediate store: `movX $IMM, ADDR`.
    /// Bypasses the accumulator entirely, saving 1-3 instructions.
    /// `imm` is the print value already validated by [`direct_store_imm`]:
    /// its low `store width` bits are exactly the bytes the store writes.
    fn try_emit_const_store(&mut self, imm: i64, ptr: &Value, ty: IrType) -> bool {
        let store_instr = Self::mov_store_for_type(ty);
        let addr = self.state.resolve_slot_addr(ptr.0);

        match addr {
            Some(SlotAddr::Direct(slot)) => {
                // Store constant directly to stack slot: movX $IMM, N(%rsp/%rbp)
                let out = &mut self.state.out;
                out.write_str("    ");
                out.write_str(store_instr);
                out.write_str(" $");
                out.write_i64(imm);
                out.write_str(", ");
                if out.use_rsp_addressing {
                    out.write_i64(out.rsp_frame_size + slot.0);
                    out.write_str("(%rsp)");
                } else {
                    out.write_i64(slot.0);
                    out.write_str("(%rbp)");
                }
                out.newline();
                self.state.reg_cache.invalidate_all();
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                true
            }
            Some(SlotAddr::Indirect(slot)) => {
                // Pointer is in a stack slot — load pointer to %rcx, then store immediate
                self.emit_load_ptr_from_slot_impl(slot, ptr.0);
                self.state
                    .emit_fmt(format_args!("    {} ${}, (%rcx)", store_instr, imm));
                self.state.reg_cache.invalidate_all();
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                true
            }
            Some(SlotAddr::Reg(reg)) => {
                // Pointer lives in a physical GPR (register-homed GEP result,
                // RA-26 caller home, ...): store the immediate directly through
                // it. Without this arm the store fell back to
                // `movq $IMM, %rax; movX %eax, (%reg)`, one instruction and an
                // accumulator clobber longer per store (IS-24).
                if !is_xmm_reg(reg) {
                    let reg_name = phys_reg_name(reg);
                    self.state.emit_fmt(format_args!(
                        "    {} ${}, (%{})",
                        store_instr, imm, reg_name
                    ));
                    self.state.reg_cache.invalidate_all();
                    self.flush_pending_vec_store_impl();
                    self.state.invalidate_vec_peephole();
                    true
                } else {
                    false
                }
            }
            Some(SlotAddr::OverAligned(slot, id)) => {
                // Over-aligned alloca: materialize its address into %rcx first,
                // then store the immediate through it — same shape the scalar
                // FP store path above uses.
                self.emit_alloca_aligned_addr_impl(slot, id);
                self.state
                    .emit_fmt(format_args!("    {} ${}, (%rcx)", store_instr, imm));
                self.state.reg_cache.invalidate_all();
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                true
            }
            _ => false, // no slot — fall back to default
        }
    }

    pub(super) fn emit_load_impl(&mut self, dest: &Value, ptr: &Value, ty: IrType) {
        // W2 fold handshake: only a redirecting path below may re-arm the
        // cast-skip; anything else leaves it None so the adjacent cast emits.
        self.fold_skip_cast = None;
        let fold_target = self.load_cast_fold.get(&dest.0).copied();
        if ty == IrType::F128 {
            if let Some(addr) = self.state.resolve_slot_addr(ptr.0) {
                self.emit_f128_fldt(&addr, ptr.0, 0);
                self.emit_f128_load_finish(dest);
                self.state.track_f128_load(dest.0, ptr.0, 0);
            }
            return;
        }

        // Try indexed addressing mode first (Phase 9 optimization)
        if self.try_emit_indexed_load(dest, ptr, ty) {
            // Indexed addressing succeeded - we're done!
            // The accumulator already holds dest's value (cache updated in try_emit_indexed_load)
            self.store_rax_to(dest);
            return;
        }

        // Scalar FP load: keep the value in the SSE domain — load directly
        // into an XMM register (the destination's own XMM register when it
        // has one) instead of round-tripping through %rax.
        if matches!(ty, IrType::F64 | IrType::F32 | IrType::D64 | IrType::D32) {
            let load_instr = if matches!(ty, IrType::F64 | IrType::D64) {
                "    movsd"
            } else {
                "    movss"
            };
            let dest_xmm = match self.reg_assignments.get(&dest.0) {
                Some(&r) if is_xmm_reg(r) => Some(phys_reg_name(r)),
                _ => None,
            };
            let target = dest_xmm.unwrap_or("xmm0");
            let addr = self.state.resolve_slot_addr(ptr.0);
            match addr {
                Some(SlotAddr::Direct(slot)) => {
                    self.state
                        .out
                        .emit_instr_rbp_reg(load_instr, slot.0, target);
                }
                Some(SlotAddr::Indirect(slot)) => {
                    if let Some(&reg) = self.reg_assignments.get(&ptr.0) {
                        if !is_xmm_reg(reg) {
                            let reg_name = phys_reg_name(reg);
                            self.state.emit_fmt(format_args!(
                                "{} (%{}), %{}",
                                load_instr, reg_name, target
                            ));
                            if dest_xmm.is_none() {
                                self.store_xmm_to(dest, "xmm0", ty);
                            }
                            return;
                        }
                    }
                    self.emit_load_ptr_from_slot_impl(slot, ptr.0);
                    self.state
                        .emit_fmt(format_args!("{} (%rcx), %{}", load_instr, target));
                }
                Some(SlotAddr::OverAligned(slot, id)) => {
                    self.emit_alloca_aligned_addr_impl(slot, id);
                    self.state
                        .emit_fmt(format_args!("{} (%rcx), %{}", load_instr, target));
                }
                Some(SlotAddr::Reg(reg)) => self.state.emit_fmt(format_args!(
                    "{} (%{}), %{}",
                    load_instr,
                    phys_reg_name(reg),
                    target
                )),
                None => {
                    // No address resolution — fall back to the default path.
                    crate::backend::traits::emit_load_default(self, dest, ptr, ty);
                    return;
                }
            }
            if dest_xmm.is_none() {
                self.store_xmm_to(dest, "xmm0", ty);
            }
            return;
        }

        // Register-direct load: when ptr has a register, load directly.
        // If dest ALSO has a register, load directly to dest (bypassing rax entirely).
        // Otherwise, load to rax and store via accumulator.
        if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128 | IrType::F128) {
            if let Some(p_reg) = self.reg_assignments.get(&ptr.0).copied() {
                if !is_xmm_reg(p_reg) && !self.state.is_alloca(ptr.0) {
                    let load_instr = self.mov_load_for_value(ty, dest.0);
                    let p_name = phys_reg_name(p_reg);
                    let use_32bit_dest = matches!(load_instr, "movl" | "movzbl" | "movzwl");
                    let use_16bit_dest = load_instr == "movw";
                    let use_8bit_dest = load_instr == "movb";

                    // W2 Load->Cast fold target takes precedence: load straight
                    // into the consumer Cast dest's register.
                    let fold_reg = fold_target.map(|(r, _)| r);
                    let d_reg_opt = fold_reg.or_else(|| self.reg_assignments.get(&dest.0).copied());
                    if let Some(d_reg) = d_reg_opt {
                        if !is_xmm_reg(d_reg) {
                            let d_name = if use_32bit_dest {
                                phys_reg_name_32(d_reg)
                            } else if use_16bit_dest || use_8bit_dest {
                                typed_phys_reg_name(d_reg, ty)
                            } else {
                                phys_reg_name(d_reg)
                            };
                            self.state.emit_fmt(format_args!(
                                "    {} (%{}), %{}",
                                load_instr, p_name, d_name
                            ));
                            if fold_reg.is_some() {
                                self.fold_skip_cast = fold_target.map(|(_, c)| c);
                            }
                            return;
                        }
                    }

                    // Dest is on stack — load to rax as before (no fold target).
                    let dest_reg = if use_32bit_dest {
                        "%eax"
                    } else if use_16bit_dest {
                        "%ax"
                    } else if use_8bit_dest {
                        "%al"
                    } else {
                        "%rax"
                    };
                    self.state.emit_fmt(format_args!(
                        "    {} (%{}), {}",
                        load_instr, p_name, dest_reg
                    ));
                    self.state.reg_cache.set_acc(dest.0, false);
                    self.store_rax_to(dest);
                    return;
                }
            }
        }

        // Accumulator-address load: a preceding GEP/Add often leaves the
        // pointer in %rax and records it in the accumulator cache.  The legacy
        // fallback immediately reloaded that same pointer into %rcx before the
        // dereference, producing `lea/add -> mov %rax,%rcx -> load (%rcx)` in
        // hot scalar loops.  x86 evaluates a memory address before writing the
        // load destination, so `movX (%rax),%eax/%rax` is equivalent and avoids
        // the relay.  This path is restricted to a verified cache hit and
        // scalar integer/pointer loads; float and wide paths retain their
        // dedicated ABI handling below.
        if !ty.is_float()
            && !matches!(ty, IrType::I128 | IrType::U128 | IrType::F128)
            && !self.state.is_alloca(ptr.0)
            && self.state.reg_cache.acc_has(ptr.0, false)
        {
            let load_instr = self.mov_load_for_value(ty, dest.0);
            let use_32bit_dest = matches!(load_instr, "movl" | "movzbl" | "movzwl");
            let use_16bit_dest = load_instr == "movw";
            let use_8bit_dest = load_instr == "movb";
            // W2 Load->Cast fold target takes precedence over dest's own home.
            let fold_reg = fold_target.map(|(r, _)| r);
            let d_reg_opt = fold_reg.or_else(|| self.reg_assignments.get(&dest.0).copied());
            if let Some(d_reg) = d_reg_opt {
                if !is_xmm_reg(d_reg) {
                    let d_name = if use_32bit_dest {
                        phys_reg_name_32(d_reg)
                    } else if use_16bit_dest || use_8bit_dest {
                        typed_phys_reg_name(d_reg, ty)
                    } else {
                        phys_reg_name(d_reg)
                    };
                    self.state
                        .emit_fmt(format_args!("    {} (%rax), %{}", load_instr, d_name));
                    if fold_reg.is_some() {
                        self.fold_skip_cast = fold_target.map(|(_, c)| c);
                    }
                    return;
                }
            }
            let dest_reg = if use_32bit_dest {
                "%eax"
            } else if use_16bit_dest {
                "%ax"
            } else if use_8bit_dest {
                "%al"
            } else {
                "%rax"
            };
            self.state
                .emit_fmt(format_args!("    {} (%rax), {}", load_instr, dest_reg));
            self.state.reg_cache.set_acc(dest.0, false);
            self.store_rax_to(dest);
            return;
        }

        // Register-direct load from alloca: when ptr is a stack-allocated local
        // variable and dest has a register, load directly to the register.
        // This bypasses the accumulator, saving one movq instruction.
        // SOUNDNESS: over-aligned allocas reserve size+(align-1) bytes and the
        // data lives at a RUNTIME-aligned address, not the slot base. A direct
        // slot load reads up to align-1 bytes before the object (zlib-ng
        // symptom: lanes[0] of an aligned(32) array loaded from the wrong
        // offset, corrupting the AVX2 adler32 horizontal sum). Fall through to
        // the default path, which materializes the aligned address.
        if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128 | IrType::F128) {
            // W2 Load->Cast fold target takes precedence over dest's own home.
            let fold_reg = fold_target.map(|(r, _)| r);
            let d_reg_opt = fold_reg.or_else(|| self.reg_assignments.get(&dest.0).copied());
            if let Some(d_reg) = d_reg_opt {
                if !is_xmm_reg(d_reg)
                    && self.state.is_alloca(ptr.0)
                    && self.state.alloca_over_align(ptr.0).is_none()
                {
                    if let Some(slot) = self.state.get_slot(ptr.0) {
                        let load_instr = Self::mov_load_for_type(ty);
                        // Match the destination width to the selected opcode.
                        // movzbl/movzwl and movl write a 32-bit register; the
                        // sign-extending forms and movq write a 64-bit register.
                        let d_name = if matches!(load_instr, "movl" | "movzbl" | "movzwl") {
                            phys_reg_name_32(d_reg)
                        } else {
                            phys_reg_name(d_reg)
                        };
                        let sr = self.slot_ref(slot.0);
                        self.state
                            .emit_fmt(format_args!("    {} {}, %{}", load_instr, sr, d_name));
                        self.state.reg_cache.invalidate_acc();
                        if fold_reg.is_some() {
                            self.fold_skip_cast = fold_target.map(|(_, c)| c);
                        }
                        return;
                    }
                }
            }
        }

        // register-direct F64/F32 load. When the destination is
        // XMM-allocated, load straight into it (movsd/movss (%ptr), %xmmN)
        // instead of the GPR round-trip (movq (%ptr),%rax; movq %rax,%xmmN).
        // Over-aligned allocas fall through (their data lives at a runtime-
        // aligned address, not the slot base).
        if ty == IrType::F64 || ty == IrType::F32 {
            if let Some(d_reg) = self.reg_assignments.get(&dest.0).copied() {
                if is_xmm_reg(d_reg) {
                    let d_name = phys_reg_name(d_reg);
                    let mov = if ty == IrType::F64 { "movsd" } else { "movss" };
                    // 1. Pointer already in a GPR-allocated register.
                    if let Some(p_reg) = self.reg_assignments.get(&ptr.0).copied() {
                        if !is_xmm_reg(p_reg) && !self.state.is_alloca(ptr.0) {
                            let p_name = phys_reg_name(p_reg);
                            self.state
                                .emit_fmt(format_args!("    {} (%{}), %{}", mov, p_name, d_name));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                    }
                    // 2. Alloca with a direct slot (data IS at the slot).
                    if self.state.is_alloca(ptr.0) && self.state.alloca_over_align(ptr.0).is_none()
                    {
                        if let Some(slot) = self.state.get_slot(ptr.0) {
                            let sr = self.slot_ref(slot.0);
                            self.state
                                .emit_fmt(format_args!("    {} {}, %{}", mov, sr, d_name));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                    }
                    // 3. General pointer (GEP into a global, IVSR phi, computed
                    //    address): materialize into %rax (or reuse it when the
                    //    accumulator cache already holds the pointer) and load
                    //    directly into the XMM home. This is the nbody inner-
                    //    loop case: the old path did movq (%ptr),%rax +
                    //    movq %rax,%xmmN.
                    self.operand_to_reg(&Operand::Value(*ptr), "rax");
                    self.state
                        .emit_fmt(format_args!("    {} (%rax), %{}", mov, d_name));
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
        }

        // Fall back to default load logic
        crate::backend::traits::emit_load_default(self, dest, ptr, ty);
    }

    pub(super) fn emit_store_with_const_offset_impl(
        &mut self,
        val: &Operand,
        base: &Value,
        offset: i64,
        ty: IrType,
    ) {
        if ty == IrType::F128 {
            if let Operand::Const(IrConst::LongDouble(_, f128_bytes)) = val {
                let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(f128_bytes);
                let lo = u64::from_le_bytes(x87[0..8].try_into().unwrap());
                let hi_bytes: [u8; 8] = [x87[8], x87[9], 0, 0, 0, 0, 0, 0];
                let hi = u64::from_le_bytes(hi_bytes);
                if let Some(addr) = self.state.resolve_slot_addr(base.0) {
                    self.emit_f128_store_raw_bytes(&addr, base.0, offset, lo, hi);
                }
                return;
            }
            if let Operand::Value(v) = val {
                if self.state.f128_direct_slots.contains(&v.0) {
                    if let Some(src_slot) = self.state.get_slot(v.0) {
                        if let Some(addr) = self.state.resolve_slot_addr(base.0) {
                            self.state.out.emit_instr_rbp("    fldt", src_slot.0);
                            self.emit_f128_fstpt(&addr, base.0, offset);
                            return;
                        }
                    }
                }
            }
            self.operand_to_rax(val);
            if let Some(addr) = self.state.resolve_slot_addr(base.0) {
                self.emit_f128_store_f64_via_x87(&addr, base.0, offset);
            }
            return;
        }
        // Scalar FP store: keep the value in the SSE domain instead of
        // round-tripping through %rax.
        if matches!(ty, IrType::F64 | IrType::F32 | IrType::D64 | IrType::D32) {
            let store_instr = if matches!(ty, IrType::F64 | IrType::D64) {
                "    movsd"
            } else {
                "    movss"
            };
            let addr = self.state.resolve_slot_addr(base.0);
            match addr {
                Some(SlotAddr::Direct(slot)) => {
                    let folded_slot = StackSlot(slot.0 + offset);
                    let src = self.fp_store_value_xmm(val, ty);
                    self.state
                        .out
                        .emit_instr_reg_rbp(store_instr, src, folded_slot.0);
                    return;
                }
                Some(SlotAddr::Indirect(slot)) => {
                    if let Some(&reg) = self.reg_assignments.get(&base.0) {
                        if !is_xmm_reg(reg) {
                            let reg_name = phys_reg_name(reg);
                            let src = self.fp_store_value_xmm(val, ty);
                            if offset != 0 {
                                self.state.emit_fmt(format_args!(
                                    "{} %{}, {}(%{})",
                                    store_instr, src, offset, reg_name
                                ));
                            } else {
                                self.state.emit_fmt(format_args!(
                                    "{} %{}, (%{})",
                                    store_instr, src, reg_name
                                ));
                            }
                            return;
                        }
                    }
                    self.emit_load_ptr_from_slot_impl(slot, base.0);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg_impl(offset);
                    }
                    let src = self.fp_store_value_xmm(val, ty);
                    self.state
                        .emit_fmt(format_args!("{} %{}, (%rcx)", store_instr, src));
                    return;
                }
                Some(SlotAddr::OverAligned(slot, id)) => {
                    self.emit_alloca_aligned_addr_impl(slot, id);
                    self.emit_add_offset_to_addr_reg_impl(offset);
                    let src = self.fp_store_value_xmm(val, ty);
                    self.state
                        .emit_fmt(format_args!("{} %{}, (%rcx)", store_instr, src));
                    return;
                }
                Some(SlotAddr::Reg(reg)) => {
                    let src = self.fp_store_value_xmm(val, ty);
                    let r = phys_reg_name(reg);
                    if offset != 0 {
                        self.state
                            .emit_fmt(format_args!("{} %{}, {}(%{})", store_instr, src, offset, r));
                    } else {
                        self.state
                            .emit_fmt(format_args!("{} %{}, (%{})", store_instr, src, r));
                    }
                    return;
                }
                None => {} // fall through to default
            }
        }

        // Non-F128: try constant-immediate store optimization first.
        if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128) {
            if let Operand::Const(c) = val {
                if let Some(imm) = c.to_i64() {
                    // The folded-offset destination accepts the same raw-field
                    // immediates as a base-register store (see
                    // `direct_store_imm`): fold `$imm`, offset AND the store
                    // width into one memory operand. Unsigned 32-bit constants
                    // above i32::MAX previously took the
                    // `movabsq $imm,%rax; movl %eax, off(%reg)` relay here.
                    if let Some(imm_print) = direct_store_imm(imm, ty) {
                        let store_instr = Self::mov_store_for_type(ty);
                        let addr = self.state.resolve_slot_addr(base.0);
                        match addr {
                            Some(SlotAddr::Direct(slot)) => {
                                let folded_slot = StackSlot(slot.0 + offset);
                                let out = &mut self.state.out;
                                out.write_str("    ");
                                out.write_str(store_instr);
                                out.write_str(" $");
                                out.write_i64(imm_print);
                                out.write_str(", ");
                                if out.use_rsp_addressing {
                                    out.write_i64(out.rsp_frame_size + folded_slot.0);
                                    out.write_str("(%rsp)");
                                } else {
                                    out.write_i64(folded_slot.0);
                                    out.write_str("(%rbp)");
                                }
                                out.newline();
                                self.state.reg_cache.invalidate_all();
                                self.flush_pending_vec_store_impl();
                                self.state.invalidate_vec_peephole();
                                return;
                            }
                            Some(SlotAddr::Indirect(slot)) => {
                                if std::env::var_os("LCCC_DBG_STORE").is_some() {
                                    eprintln!("[DBGSTORE] base={} get_slot={:?} reg_assign={:?} offset={}",
                                        base.0, self.state.get_slot(base.0),
                                        self.reg_assignments.get(&base.0), offset);
                                }
                                if let Some(&reg) = self.reg_assignments.get(&base.0) {
                                    let reg_name = phys_reg_name(reg);
                                    if offset != 0 {
                                        self.state.emit_fmt(format_args!(
                                            "    {} ${}, {}(%{})",
                                            store_instr, imm_print, offset, reg_name
                                        ));
                                    } else {
                                        self.state.emit_fmt(format_args!(
                                            "    {} ${}, (%{})",
                                            store_instr, imm_print, reg_name
                                        ));
                                    }
                                } else {
                                    self.emit_load_ptr_from_slot_impl(slot, base.0);
                                    if offset != 0 {
                                        self.emit_add_offset_to_addr_reg_impl(offset);
                                    }
                                    self.state.emit_fmt(format_args!(
                                        "    {} ${}, (%rcx)",
                                        store_instr, imm_print
                                    ));
                                }
                                self.state.reg_cache.invalidate_all();
                                self.flush_pending_vec_store_impl();
                                self.state.invalidate_vec_peephole();
                                return;
                            }
                            Some(SlotAddr::Reg(reg)) => {
                                // Base pointer is register-homed (RA-26 param
                                // home / register GEP): fold offset + immediate
                                // into one addressing form (IS-24). Without this
                                // arm the store materialized the constant via
                                // `movq $IMM, %rax` first.
                                if !is_xmm_reg(reg) {
                                    let reg_name = phys_reg_name(reg);
                                    if offset != 0 {
                                        self.state.emit_fmt(format_args!(
                                            "    {} ${}, {}(%{})",
                                            store_instr, imm_print, offset, reg_name
                                        ));
                                    } else {
                                        self.state.emit_fmt(format_args!(
                                            "    {} ${}, (%{})",
                                            store_instr, imm_print, reg_name
                                        ));
                                    }
                                    self.state.reg_cache.invalidate_all();
                                    self.flush_pending_vec_store_impl();
                                    self.state.invalidate_vec_peephole();
                                    return;
                                }
                            }
                            Some(SlotAddr::OverAligned(slot, id)) => {
                                // Over-aligned alloca base: materialize the
                                // aligned address, add the field offset, store
                                // the immediate through it.
                                self.emit_alloca_aligned_addr_impl(slot, id);
                                if offset != 0 {
                                    self.emit_add_offset_to_addr_reg_impl(offset);
                                }
                                self.state.emit_fmt(format_args!(
                                    "    {} ${}, (%rcx)",
                                    store_instr, imm_print
                                ));
                                self.state.reg_cache.invalidate_all();
                                self.flush_pending_vec_store_impl();
                                self.state.invalidate_vec_peephole();
                                return;
                            }
                            _ => {} // fall through to default path
                        }
                    }
                }
            }
        }

        // Register-direct store: when val has a register, store directly to memory
        // without loading to %rax first. Handles Direct (alloca) and Indirect (ptr) cases.
        // FP variant — an XMM-allocated F64/F32 value stores straight to the
        // folded-offset address (movsd/movss %xmmN, off(%base)) instead of the
        // GPR round-trip (movq %xmmN,%rax; movq %rax,off(%base)).  This fires
        // for both a GPR base register and a stack-local (alloca) destination;
        // before, the alloca case fell through to the accumulator round-trip,
        // which is the dominant cost of FP struct-field stores (struct_copy:
        // every `p.x = ...` paid movq %xmmN,%rax; movq %rax,slot).
        if ty == IrType::F64 || ty == IrType::F32 {
            if let Operand::Value(v) = val {
                if let Some(v_reg) = self.reg_assignments.get(&v.0).copied() {
                    if is_xmm_reg(v_reg) {
                        let mov = if ty == IrType::F64 { "movsd" } else { "movss" };
                        let v_name = phys_reg_name(v_reg);
                        if self.state.is_alloca(base.0) {
                            // Direct into the stack slot (aligned double/float).
                            if let Some(addr) = self.state.resolve_slot_addr(base.0) {
                                if let SlotAddr::Direct(slot) = addr {
                                    let folded_slot = StackSlot(slot.0 + offset);
                                    let sr = self.slot_ref(folded_slot.0);
                                    self.state
                                        .emit_fmt(format_args!("    {} %{}, {}", mov, v_name, sr));
                                    self.state.reg_cache.invalidate_acc();
                                    return;
                                }
                            }
                        } else if let Some(b_reg) = self.reg_assignments.get(&base.0).copied() {
                            if !is_xmm_reg(b_reg) {
                                let b_name = phys_reg_name(b_reg);
                                if offset != 0 {
                                    self.state.emit_fmt(format_args!(
                                        "    {} %{}, {}(%{})",
                                        mov, v_name, offset, b_name
                                    ));
                                } else {
                                    self.state.emit_fmt(format_args!(
                                        "    {} %{}, (%{})",
                                        mov, v_name, b_name
                                    ));
                                }
                                self.state.reg_cache.invalidate_acc();
                                return;
                            }
                        }
                    }
                }
            }
        }
        if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128 | IrType::F128) {
            if let Operand::Value(v) = val {
                if let Some(v_reg) = self.reg_assignments.get(&v.0).copied() {
                    if !is_xmm_reg(v_reg) {
                        let store_instr = Self::mov_store_for_type(ty);
                        let v_name = typed_phys_reg_name(v_reg, ty);
                        let addr = self.state.resolve_slot_addr(base.0);
                        match addr {
                            Some(SlotAddr::Direct(slot)) => {
                                let folded_slot = StackSlot(slot.0 + offset);
                                let sr = self.slot_ref(folded_slot.0);
                                self.state.emit_fmt(format_args!(
                                    "    {} %{}, {}",
                                    store_instr, v_name, sr
                                ));
                                return;
                            }
                            Some(SlotAddr::Indirect(_slot)) => {
                                if let Some(&b_reg) = self.reg_assignments.get(&base.0) {
                                    let b_name = phys_reg_name(b_reg);
                                    if offset != 0 {
                                        self.state.emit_fmt(format_args!(
                                            "    {} %{}, {}(%{})",
                                            store_instr, v_name, offset, b_name
                                        ));
                                    } else {
                                        self.state.emit_fmt(format_args!(
                                            "    {} %{}, (%{})",
                                            store_instr, v_name, b_name
                                        ));
                                    }
                                    return;
                                }
                                // base not in register — fall through to load base to %rcx
                            }
                            _ => {} // fall through
                        }
                    }
                }
            }
        }

        // Default GEP fold logic.
        let addr = self.state.resolve_slot_addr(base.0);
        if let Some(addr) = addr {
            let store_instr = Self::mov_store_for_type(ty);
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    // For over-aligned, load ptr first to rcx, then load value to rax
                    self.emit_alloca_aligned_addr_impl(slot, id);
                    self.emit_add_offset_to_addr_reg_impl(offset);
                    // rcx now holds the target address
                    self.operand_to_rax(val);
                    let store_reg = Self::reg_for_type("rax", ty);
                    self.state
                        .emit_fmt(format_args!("    {} %{}, (%rcx)", store_instr, store_reg));
                }
                SlotAddr::Direct(slot) => {
                    self.operand_to_rax(val);
                    let folded_slot = StackSlot(slot.0 + offset);
                    self.emit_typed_store_to_slot_impl(store_instr, ty, folded_slot);
                }
                SlotAddr::Indirect(slot) => {
                    if let Some(&reg) = self.reg_assignments.get(&base.0) {
                        self.operand_to_rax(val);
                        let reg_name = phys_reg_name(reg);
                        let store_reg = Self::reg_for_type("rax", ty);
                        if offset != 0 {
                            self.state.emit_fmt(format_args!(
                                "    {} %{}, {}(%{})",
                                store_instr, store_reg, offset, reg_name
                            ));
                        } else {
                            self.state.emit_fmt(format_args!(
                                "    {} %{}, (%{})",
                                store_instr, store_reg, reg_name
                            ));
                        }
                    } else {
                        // Load pointer to %rcx FIRST, then load value to %rax.
                        // This avoids emit_save_acc which can be clobbered by
                        // operand_to_rax if the value uses %r11/%rdx as scratch.
                        self.emit_load_ptr_from_slot_impl(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg_impl(offset);
                        }
                        // Now load value — %rcx is safe (not clobbered by operand_to_rax)
                        self.operand_to_rax(val);
                        let store_reg = Self::reg_for_type("rax", ty);
                        self.state
                            .emit_fmt(format_args!("    {} %{}, (%rcx)", store_instr, store_reg));
                    }
                }
                SlotAddr::Reg(reg) => {
                    self.operand_to_rax(val);
                    let r = phys_reg_name(reg);
                    let src = Self::reg_for_type("rax", ty);
                    if offset != 0 {
                        self.state.emit_fmt(format_args!(
                            "    {} %{}, {}(%{})",
                            store_instr, src, offset, r
                        ));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    {} %{}, (%{})", store_instr, src, r));
                    }
                }
            }
        } else if let Some(&b_reg) = self.reg_assignments.get(&base.0) {
            // Register-resident base with no slot home (fold accepted through
            // const_offset_fold_reg_base_ok).  The old code staged the value
            // into %rax and emitted NO store.  operand_to_rax only touches
            // %rax plus the %r11/%rdx staging scratches — the base was
            // verified to sit outside that set by the fold predicate.
            if !is_xmm_reg(b_reg) {
                self.operand_to_rax(val);
                let b_name = phys_reg_name(b_reg);
                let store_instr = Self::mov_store_for_type(ty);
                let store_reg = Self::reg_for_type("rax", ty);
                if offset != 0 {
                    self.state.emit_fmt(format_args!(
                        "    {} %{}, {}(%{})",
                        store_instr, store_reg, offset, b_name
                    ));
                } else {
                    self.state.emit_fmt(format_args!(
                        "    {} %{}, (%{})",
                        store_instr, store_reg, b_name
                    ));
                }
                self.state.reg_cache.invalidate_all();
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
            }
        } else {
            // No addr resolution — fall back
            self.operand_to_rax(val);
        }
    }

    pub(super) fn emit_load_with_const_offset_impl(
        &mut self,
        dest: &Value,
        base: &Value,
        offset: i64,
        ty: IrType,
    ) {
        if ty == IrType::F128 {
            if let Some(addr) = self.state.resolve_slot_addr(base.0) {
                self.emit_f128_fldt(&addr, base.0, offset);
                self.emit_f128_load_finish(dest);
            }
            return;
        }
        // Scalar FP load: keep the value in the SSE domain — load directly
        // into an XMM register (the destination's own XMM register when it
        // has one) instead of round-tripping through %rax. Covers Direct,
        // Indirect (register-held or slot-held base) and OverAligned bases,
        // so FP-struct field loads in loops never pay the GPR shuttle.
        if matches!(ty, IrType::F64 | IrType::F32 | IrType::D64 | IrType::D32) {
            let load_instr = if matches!(ty, IrType::F64 | IrType::D64) {
                "    movsd"
            } else {
                "    movss"
            };
            let dest_xmm = match self.reg_assignments.get(&dest.0) {
                Some(&r) if is_xmm_reg(r) => Some(phys_reg_name(r)),
                _ => None,
            };
            let target = dest_xmm.unwrap_or("xmm0");
            let addr = self.state.resolve_slot_addr(base.0);
            match addr {
                Some(SlotAddr::Direct(slot)) => {
                    let folded_slot = StackSlot(slot.0 + offset);
                    self.state
                        .out
                        .emit_instr_rbp_reg(load_instr, folded_slot.0, target);
                }
                Some(SlotAddr::Indirect(slot)) => {
                    if let Some(&reg) = self.reg_assignments.get(&base.0) {
                        if !is_xmm_reg(reg) {
                            let reg_name = phys_reg_name(reg);
                            if offset != 0 {
                                self.state.emit_fmt(format_args!(
                                    "{} {}(%{}), %{}",
                                    load_instr, offset, reg_name, target
                                ));
                            } else {
                                self.state.emit_fmt(format_args!(
                                    "{} (%{}), %{}",
                                    load_instr, reg_name, target
                                ));
                            }
                            if dest_xmm.is_none() {
                                self.store_xmm_to(dest, "xmm0", ty);
                            }
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                    }
                    self.emit_load_ptr_from_slot_impl(slot, base.0);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg_impl(offset);
                    }
                    self.state
                        .emit_fmt(format_args!("{} (%rcx), %{}", load_instr, target));
                }
                Some(SlotAddr::OverAligned(slot, id)) => {
                    self.emit_alloca_aligned_addr_impl(slot, id);
                    self.emit_add_offset_to_addr_reg_impl(offset);
                    self.state
                        .emit_fmt(format_args!("{} (%rcx), %{}", load_instr, target));
                }
                Some(SlotAddr::Reg(reg)) => {
                    let r = phys_reg_name(reg);
                    if offset != 0 {
                        self.state.emit_fmt(format_args!(
                            "{} {}(%{}), %{}",
                            load_instr, offset, r, target
                        ));
                    } else {
                        self.state
                            .emit_fmt(format_args!("{} (%{}), %{}", load_instr, r, target));
                    }
                }
                None => {
                    // Register-resident base with no slot home (fold accepted
                    // through const_offset_fold_reg_base_ok).  Dropping the
                    // load here silently read garbage; load through the base.
                    if let Some(&reg) = self.reg_assignments.get(&base.0) {
                        if !is_xmm_reg(reg) {
                            let reg_name = phys_reg_name(reg);
                            if offset != 0 {
                                self.state.emit_fmt(format_args!(
                                    "{} {}(%{}), %{}",
                                    load_instr, offset, reg_name, target
                                ));
                            } else {
                                self.state.emit_fmt(format_args!(
                                    "{} (%{}), %{}",
                                    load_instr, reg_name, target
                                ));
                            }
                        } else {
                            return;
                        }
                    } else {
                        return;
                    }
                }
            }
            if dest_xmm.is_none() {
                self.store_xmm_to(dest, "xmm0", ty);
            }
            // Conservative: a stale acc entry for a value that aliases the
            // loaded memory must not survive this load.
            self.state.reg_cache.invalidate_acc();
            return;
        }
        let addr = self.state.resolve_slot_addr(base.0);
        if let Some(addr) = addr {
            let load_instr = Self::mov_load_for_type(ty);
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_alloca_aligned_addr_impl(slot, id);
                    self.emit_add_offset_to_addr_reg_impl(offset);
                    self.emit_typed_load_indirect_impl(load_instr);
                }
                SlotAddr::Direct(slot) => {
                    let folded_slot = StackSlot(slot.0 + offset);
                    self.emit_typed_load_from_slot_impl(load_instr, folded_slot);
                }
                SlotAddr::Indirect(slot) => {
                    if let Some(&reg) = self.reg_assignments.get(&base.0) {
                        let reg_name = phys_reg_name(reg);

                        // Register-direct: if dest also has a register, load directly to it.
                        if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128) {
                            if let Some(&d_reg) = self.reg_assignments.get(&dest.0) {
                                if !is_xmm_reg(d_reg) {
                                    // movslq/movsbq/movswq need 64-bit dest.
                                    // movl/movzbl/movzwl use 32-bit dest (implicit
                                    // zero-extend). Keyed on the OPCODE, not the
                                    // IR type: a U8 load is `movzbl` → 32-bit dest
                                    // (`%r11d`), and emitting `%r11` is rejected by
                                    // GNU as (GAS oracle: "incorrect register %r11
                                    // used with l suffix").
                                    let load_instr = Self::mov_load_for_type(ty);
                                    let use_32bit_dest =
                                        matches!(load_instr, "movl" | "movzbl" | "movzwl");
                                    let d_name = if use_32bit_dest {
                                        phys_reg_name_32(d_reg)
                                    } else {
                                        phys_reg_name(d_reg)
                                    };
                                    if offset != 0 {
                                        self.state.emit_fmt(format_args!(
                                            "    {} {}(%{}), %{}",
                                            load_instr, offset, reg_name, d_name
                                        ));
                                    } else {
                                        self.state.emit_fmt(format_args!(
                                            "    {} (%{}), %{}",
                                            load_instr, reg_name, d_name
                                        ));
                                    }
                                    return;
                                }
                            }
                        }

                        let dest_reg = Self::load_dest_reg(ty);
                        if offset != 0 {
                            self.state.emit_fmt(format_args!(
                                "    {} {}(%{}), {}",
                                load_instr, offset, reg_name, dest_reg
                            ));
                        } else {
                            self.state.emit_fmt(format_args!(
                                "    {} (%{}), {}",
                                load_instr, reg_name, dest_reg
                            ));
                        }
                    } else {
                        self.emit_load_ptr_from_slot_impl(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg_impl(offset);
                        }
                        self.emit_typed_load_indirect_impl(load_instr);
                    }
                }
                SlotAddr::Reg(reg) => {
                    let r = phys_reg_name(reg);
                    if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128) {
                        if let Some(&dr) = self.reg_assignments.get(&dest.0) {
                            if !is_xmm_reg(dr) {
                                let u = matches!(load_instr, "movl" | "movzbl" | "movzwl");
                                let d = if u {
                                    phys_reg_name_32(dr)
                                } else if load_instr == "movb" || load_instr == "movw" {
                                    typed_phys_reg_name(dr, ty)
                                } else {
                                    phys_reg_name(dr)
                                };
                                if offset != 0 {
                                    self.state.emit_fmt(format_args!(
                                        "    {} {}(%{}), %{}",
                                        load_instr, offset, r, d
                                    ));
                                } else {
                                    self.state.emit_fmt(format_args!(
                                        "    {} (%{}), %{}",
                                        load_instr, r, d
                                    ));
                                }
                                return;
                            }
                        }
                    }
                    let d = Self::load_dest_reg(ty);
                    if offset != 0 {
                        self.state
                            .emit_fmt(format_args!("    {} {}(%{}), {}", load_instr, offset, r, d));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    {} (%{}), {}", load_instr, r, d));
                    }
                }
            }
            self.store_rax_to(dest);
        } else if let Some(&reg) = self.reg_assignments.get(&base.0) {
            // Register-resident base with no slot home (fold accepted through
            // const_offset_fold_reg_base_ok): load through the base register.
            // Previously this shape fell out with no instruction emitted.
            if !is_xmm_reg(reg) {
                let load_instr = Self::mov_load_for_type(ty);
                let reg_name = phys_reg_name(reg);
                if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128) {
                    if let Some(&d_reg) = self.reg_assignments.get(&dest.0) {
                        if !is_xmm_reg(d_reg) {
                            let use_32bit_dest = matches!(load_instr, "movl" | "movzbl" | "movzwl");
                            let use_16bit_dest = load_instr == "movw";
                            let use_8bit_dest = load_instr == "movb";
                            let d_name = if use_32bit_dest {
                                phys_reg_name_32(d_reg)
                            } else if use_16bit_dest || use_8bit_dest {
                                typed_phys_reg_name(d_reg, ty)
                            } else {
                                phys_reg_name(d_reg)
                            };
                            if offset != 0 {
                                self.state.emit_fmt(format_args!(
                                    "    {} {}(%{}), %{}",
                                    load_instr, offset, reg_name, d_name
                                ));
                            } else {
                                self.state.emit_fmt(format_args!(
                                    "    {} (%{}), %{}",
                                    load_instr, reg_name, d_name
                                ));
                            }
                            return;
                        }
                    }
                }
                let dest_reg = Self::load_dest_reg(ty);
                if offset != 0 {
                    self.state.emit_fmt(format_args!(
                        "    {} {}(%{}), {}",
                        load_instr, offset, reg_name, dest_reg
                    ));
                } else {
                    self.state.emit_fmt(format_args!(
                        "    {} (%{}), {}",
                        load_instr, reg_name, dest_reg
                    ));
                }
                self.store_rax_to(dest);
            }
        }
    }

    // ---- SIB indexed addressing (session 28): mem(,%idx,scale) ----
    //
    // Mirrors the session-27 i686 emitters on the 64-bit backend: a
    // variable-offset GEP `base + idx<<shift` becomes one SIB memory operand
    // instead of shift+add+lea+access.  Soundness contract: the prologue
    // wires collect_folded_gep_links_all (base AND index) into register
    // allocation, so both address registers survive to the access.  %rax is
    // not allocatable on x86-64, so rax staging can never collide with the
    // base/index registers.

    fn sib_mem64(base_reg: &str, index_reg: &str, shift: u8, disp: i64) -> String {
        let d = if disp == 0 {
            String::new()
        } else {
            format!("{}", disp)
        };
        if shift == 0 {
            format!("{}(%{}, %{})", d, base_reg, index_reg)
        } else {
            format!("{}(%{}, %{}, {})", d, base_reg, index_reg, 1u32 << shift)
        }
    }

    fn sib_mem64_sym(sym: &str, index_reg: &str, shift: u8, disp: i64) -> String {
        // AT&T `sym+disp(, %idx, scale)`. Concatenating the raw number
        // (`foo` + `4` → `foo4`) invents a different symbol. Negative
        // displacements already carry the minus sign.
        let head = if disp == 0 {
            sym.to_string()
        } else if disp > 0 {
            format!("{}+{}", sym, disp)
        } else {
            format!("{}{}", sym, disp)
        };
        if shift == 0 {
            format!("{}(, %{})", head, index_reg)
        } else {
            format!("{}(, %{}, {})", head, index_reg, 1u32 << shift)
        }
    }

    fn emit_load_indexed_common(
        &mut self,
        dest: &Value,
        index: &Value,
        shift: u8,
        ty: IrType,
        mem: String,
    ) -> bool {
        if shift > 3 {
            return false;
        }
        // FP loads stay in the SSE domain.
        if matches!(ty, IrType::F32 | IrType::F64) {
            let instr = if ty == IrType::F64 { "movsd" } else { "movss" };
            let dest_xmm = match self.reg_assignments.get(&dest.0) {
                Some(&r) if is_xmm_reg(r) => Some(phys_reg_name(r)),
                _ => None,
            };
            let target = dest_xmm.unwrap_or("xmm0");
            self.state
                .emit_fmt(format_args!("    {} {}, %{}", instr, mem, target));
            if dest_xmm.is_none() {
                self.store_xmm_to(dest, "xmm0", ty);
            }
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        // I64/U64 included: on x86-64 they are everyday scalar loads.  The
        // dead-producer skip in generation.rs assumes every consumer of a
        // foldable indexed GEP IS folded — a type the emitter rejects would
        // fall back to rematerialisation and read the skipped offset chain.
        if !matches!(
            ty,
            IrType::I8
                | IrType::U8
                | IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::I64
                | IrType::U64
                | IrType::Ptr
        ) {
            return false;
        }
        let load_instr = Self::mov_load_for_type(ty);
        // IS-15: honor the function-wide sext analysis for I32 loads. The
        // static table always answers `movslq` for I32, but when this value
        // has no 64-bit consumer (not in `needs_sext_values`) a plain `movl`
        // is correct and saves the implicit REX byte; downstream consumers
        // only read the low 32 bits. This mirrors what the non-indexed load
        // path (`mov_load_for_value`) already does.
        let load_instr = if ty == IrType::I32 {
            self.mov_load_for_value(ty, dest.0)
        } else {
            load_instr
        };
        let _ = index; // liveness handled by the RA link wiring
        if let Some(&d_reg) = self.reg_assignments.get(&dest.0) {
            if !is_xmm_reg(d_reg) {
                let use_32bit_dest = matches!(load_instr, "movl" | "movzbl" | "movzwl");
                let use_16bit_dest = load_instr == "movw";
                let use_8bit_dest = load_instr == "movb";
                let d_name = if use_32bit_dest {
                    phys_reg_name_32(d_reg)
                } else if use_16bit_dest || use_8bit_dest {
                    typed_phys_reg_name(d_reg, ty)
                } else {
                    phys_reg_name(d_reg)
                };
                self.state
                    .emit_fmt(format_args!("    {} {}, %{}", load_instr, mem, d_name));
                self.state.reg_cache.invalidate_acc();
                return true;
            }
        }
        let acc_reg = if matches!(load_instr, "movl" | "movzbl" | "movzwl") {
            "%eax"
        } else if load_instr == "movw" {
            "%ax"
        } else if load_instr == "movb" {
            "%al"
        } else {
            "%rax"
        };
        self.state
            .emit_fmt(format_args!("    {} {}, {}", load_instr, mem, acc_reg));
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
        true
    }

    fn emit_store_indexed_common(
        &mut self,
        val: &Operand,
        index: &Value,
        shift: u8,
        ty: IrType,
        mem: String,
    ) -> bool {
        if shift > 3 {
            return false;
        }
        // FP stores straight from the XMM home.
        if matches!(ty, IrType::F32 | IrType::F64) {
            let instr = if ty == IrType::F64 { "movsd" } else { "movss" };
            let src = self.fp_store_value_xmm(val, ty);
            self.state
                .emit_fmt(format_args!("    {} %{}, {}", instr, src, mem));
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        // I64/U64 included (see emit_load_indexed_common).
        if !matches!(
            ty,
            IrType::I8
                | IrType::U8
                | IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::I64
                | IrType::U64
                | IrType::Ptr
        ) {
            return false;
        }
        let store_instr = Self::mov_store_for_type(ty);
        let _ = index; // liveness handled by the RA link wiring
                       // Immediate-direct: one instruction, no rax staging.
        if let Operand::Const(c) = val {
            if let Some(imm) = c.to_i64() {
                if imm >= i32::MIN as i64 && imm <= i32::MAX as i64 {
                    let imm_out = match ty {
                        IrType::I8 | IrType::U8 => imm & 0xff,
                        IrType::I16 | IrType::U16 => imm & 0xffff,
                        _ => imm,
                    };
                    self.state
                        .emit_fmt(format_args!("    {} ${}, {}", store_instr, imm_out, mem));
                    return true;
                }
            }
        }
        self.operand_to_rax(val);
        let src = Self::reg_for_type("rax", ty);
        self.state
            .emit_fmt(format_args!("    {} %{}, {}", store_instr, src, mem));
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        true
    }

    /// SIB memory operand anchored at the frame register with an alloca slot
    /// offset: `slot+disp(%rbp/%rsp,%idx,scale)`. Mirrors `slot_ref`'s
    /// rsp-frame-size adjustment for `use_rsp_addressing` functions. When
    /// the final displacement is zero on an rbp frame, "0" is kept: a bare
    /// `(%rbp,%idx,s)` with no displacement is ambiguous in the encoder.
    fn sib_mem64_frame(&self, slot_off: i64, index_reg: &str, shift: u8, disp: i64) -> String {
        // NOTE: no leading '%' — the format literals below supply it
        // (`%{}`), matching phys_reg_name's no-prefix convention.
        let frame = if self.state.out.use_rsp_addressing {
            "rsp"
        } else {
            "rbp"
        };
        let off = if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size + slot_off
        } else {
            slot_off
        } + disp;
        // On an rbp frame the displacement is always materialised: a bare
        // `(%rbp,%idx,s)` with mod=00 would relocate the base to RIP. On an
        // rsp frame SIB base=rsp is unambiguous, so 0 can be omitted.
        let d = if off == 0 && frame == "rsp" {
            String::new()
        } else {
            format!("{}", off)
        };
        if shift == 0 {
            format!("{}(%{}, %{})", d, frame, index_reg)
        } else {
            format!("{}(%{}, %{}, {})", d, frame, index_reg, 1u32 << shift)
        }
    }

    /// Alloca-base arm for indexed addressing: the base is a frame slot
    /// (Direct), the SIB anchors at %rbp/%rsp. Returns None when the base is
    /// not a plain Direct alloca slot (register-held, Indirect, OverAligned).
    fn frame_sib_for_alloca_base(
        &self,
        base: &Value,
        index: &Value,
        shift: u8,
        disp: i64,
    ) -> Option<String> {
        let idx_reg = self.reg_assignments.get(&index.0).copied()?;
        if is_xmm_reg(idx_reg) {
            return None;
        }
        match self.state.resolve_slot_addr(base.0) {
            Some(SlotAddr::Direct(slot)) => {
                Some(self.sib_mem64_frame(slot.0 as i64, phys_reg_name(idx_reg), shift, disp))
            }
            _ => None,
        }
    }

    pub(super) fn emit_load_indexed_impl(
        &mut self,
        dest: &Value,
        base: &Value,
        index: &Value,
        shift: u8,
        disp: i64,
        ty: IrType,
    ) -> bool {
        let Some(&b) = self.reg_assignments.get(&base.0) else {
            // Alloca-base indexed addressing: `disp(%rbp/%rsp,%idx,scale)`.
            if !self.ensure_sib_index_form(index) {
                return false;
            }
            if let Some(mem) = self.frame_sib_for_alloca_base(base, index, shift, disp) {
                return self.emit_load_indexed_common(dest, index, shift, ty, mem);
            }
            return false;
        };
        if is_xmm_reg(b)
            || self
                .reg_assignments
                .get(&index.0)
                .copied()
                .map_or(true, is_xmm_reg)
        {
            return false;
        }
        if !self.ensure_sib_index_form(index) {
            return false;
        }
        let mem = Self::sib_mem64(
            phys_reg_name(b),
            phys_reg_name(self.reg_assignments[&index.0]),
            shift,
            disp,
        );
        self.emit_load_indexed_common(dest, index, shift, ty, mem)
    }

    pub(super) fn emit_store_indexed_impl(
        &mut self,
        val: &Operand,
        base: &Value,
        index: &Value,
        shift: u8,
        disp: i64,
        ty: IrType,
    ) -> bool {
        let Some(&b) = self.reg_assignments.get(&base.0) else {
            // Alloca-base indexed addressing: `disp(%rbp/%rsp,%idx,scale)`.
            if !self.ensure_sib_index_form(index) {
                return false;
            }
            if let Some(mem) = self.frame_sib_for_alloca_base(base, index, shift, disp) {
                return self.emit_store_indexed_common(val, index, shift, ty, mem);
            }
            return false;
        };
        if is_xmm_reg(b)
            || self
                .reg_assignments
                .get(&index.0)
                .copied()
                .map_or(true, is_xmm_reg)
        {
            return false;
        }
        if !self.ensure_sib_index_form(index) {
            return false;
        }
        let mem = Self::sib_mem64(
            phys_reg_name(b),
            phys_reg_name(self.reg_assignments[&index.0]),
            shift,
            disp,
        );
        self.emit_store_indexed_common(val, index, shift, ty, mem)
    }

    pub(super) fn emit_load_indexed_sym_impl(
        &mut self,
        dest: &Value,
        sym: &str,
        index: &Value,
        shift: u8,
        disp: i64,
        ty: IrType,
    ) -> bool {
        // `sym` may carry a composed constant displacement ("gh+2"). The GOT
        // verdict must be computed on the base symbol, exactly like
        // `rip_rel_blocked` does on the deciding side (`can_indexed_addr_fold`).
        // Passing the composed string made `needs_got_for_addr` miss
        // `local_symbols` under -fPIC, so the emitter refused a fold the
        // allocator had already committed to: the skipped GEP was then
        // rematerialised at the access site, clobbering %rax while it held the
        // acc-resident store value (speedtest1 HashFinal ICE).
        // C symbol names cannot contain '+'; '-' only occurs in the map's
        // displacement suffix.
        let base_sym = sym.split(['+', '-']).next().unwrap_or(sym);
        if self.state.needs_got_for_addr(base_sym) {
            return false;
        }
        let Some(&x) = self.reg_assignments.get(&index.0) else {
            return false;
        };
        if is_xmm_reg(x) {
            return false;
        }
        if !self.ensure_sib_index_form(index) {
            return false;
        }
        let index_name = phys_reg_name(x);
        let mem = if self.state.pic_mode {
            // x86 has no RIP-relative SIB form. Rebuild the legal direct
            // executable/local symbol in reserved %rcx, then consume the
            // scaled index in one memory operand. This avoids separately
            // materializing the shift, add, and derived pointer.
            self.state
                .out
                .emit_instr_sym_base_reg("    leaq", sym, "rip", "rcx");
            Self::sib_mem64("rcx", index_name, shift, disp)
        } else {
            Self::sib_mem64_sym(sym, index_name, shift, disp)
        };
        self.emit_load_indexed_common(dest, index, shift, ty, mem)
    }

    pub(super) fn emit_store_indexed_sym_impl(
        &mut self,
        val: &Operand,
        sym: &str,
        index: &Value,
        shift: u8,
        disp: i64,
        ty: IrType,
    ) -> bool {
        // Same basename rule as emit_load_indexed_sym_impl (see comment there):
        // the GOT check must match the deciding side or the fold is refused
        // after the allocator committed to it.
        let base_sym = sym.split(['+', '-']).next().unwrap_or(sym);
        if self.state.needs_got_for_addr(base_sym) {
            return false;
        }
        let Some(&x) = self.reg_assignments.get(&index.0) else {
            return false;
        };
        if is_xmm_reg(x) {
            return false;
        }
        if !self.ensure_sib_index_form(index) {
            return false;
        }
        let index_name = phys_reg_name(x);
        let mem = if self.state.pic_mode {
            self.state
                .out
                .emit_instr_sym_base_reg("    leaq", sym, "rip", "rcx");
            Self::sib_mem64("rcx", index_name, shift, disp)
        } else {
            Self::sib_mem64_sym(sym, index_name, shift, disp)
        };
        self.emit_store_indexed_common(val, index, shift, ty, mem)
    }

    pub(super) fn emit_typed_store_to_slot_impl(
        &mut self,
        instr: &'static str,
        ty: IrType,
        slot: StackSlot,
    ) {
        // The instruction suffix is authoritative here.  `reg_for_type`
        // describes the IR value, but a narrow copy must use the matching
        // architectural subregister; `%rax` with `movb`/`movw` is rejected by
        // GAS and, worse, used to hide this bug behind a late assembler error.
        let reg = match instr {
            "movb" => "al",
            "movw" => "ax",
            "movl" | "movzbl" | "movzwl" | "movsbl" | "movswl" => "eax",
            _ => Self::reg_for_type("rax", ty),
        };
        let out = &mut self.state.out;
        out.write_str("    ");
        out.write_str(instr);
        out.write_str(" %");
        out.write_str(reg);
        out.write_str(", ");
        if out.use_rsp_addressing {
            out.write_i64(out.rsp_frame_size + slot.0);
            out.write_str("(%rsp)");
        } else {
            out.write_i64(slot.0);
            out.write_str("(%rbp)");
        }
        out.newline();
    }

    pub(super) fn emit_typed_load_from_slot_impl(&mut self, instr: &'static str, slot: StackSlot) {
        // Select the architectural destination width of the move. `movb` and
        // `movw` appear in narrow temporary copy paths; spelling `%rax` there
        // is invalid assembly. Extending loads and movl write eax, while
        // full-width/sign-to-64 loads write rax.
        let dest_reg = match instr {
            "movb" => "%al",
            "movw" => "%ax",
            "movl" | "movzbl" | "movzwl" | "movsbl" | "movswl" => "%eax",
            _ => "%rax",
        };
        let out = &mut self.state.out;
        out.write_str("    ");
        out.write_str(instr);
        out.write_str(" ");
        if out.use_rsp_addressing {
            out.write_i64(out.rsp_frame_size + slot.0);
            out.write_str("(%rsp), ");
        } else {
            out.write_i64(slot.0);
            out.write_str("(%rbp), ");
        }
        out.write_str(dest_reg);
        out.newline();
    }

    pub(super) fn emit_save_acc_impl(&mut self) {
        // Save %rax to a scratch register before loading a pointer to %rcx.
        // Use %r11 when %rdx is allocated to a value (rdx is in the caller-saved pool).
        if self.reg_assignments.values().any(|r| r.0 == 16) {
            self.state.emit("    movq %rax, %r11");
        } else {
            self.state.emit("    movq %rax, %rdx");
        }
    }

    pub(super) fn emit_load_ptr_from_slot_impl(&mut self, slot: StackSlot, val_id: u32) {
        self.state.reg_cache.invalidate_sec(); // clobbers %rcx
        if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rcx");
        } else {
            self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rcx");
        }
    }

    pub(super) fn emit_typed_store_indirect_impl(&mut self, instr: &'static str, ty: IrType) {
        // Store from the accumulator (%rax) to the address in %rcx.
        // The value was loaded to %rax AFTER the pointer was loaded to %rcx,
        // so %rax holds the correct value.
        let store_reg = Self::reg_for_type("rax", ty);
        self.state
            .emit_fmt(format_args!("    {} %{}, (%rcx)", instr, store_reg));
    }

    pub(super) fn emit_typed_load_indirect_impl(&mut self, instr: &'static str) {
        // movl/movzbl/movzwl write a 32-bit register (implicit zero-extend);
        // %rax for movzbl/movzwl is rejected by GAS (GAS-oracle).
        let dest_reg = if matches!(instr, "movl" | "movzbl" | "movzwl") {
            "%eax"
        } else {
            "%rax"
        };
        self.state
            .emit_fmt(format_args!("    {} (%rcx), {}", instr, dest_reg));
    }

    pub(super) fn emit_add_offset_to_addr_reg_impl(&mut self, offset: i64) {
        self.state.out.emit_instr_imm_reg("    addq", offset, "rcx");
    }

    /// Compute the address of an alloca into `reg`, handling over-aligned allocas.
    pub(super) fn emit_alloca_addr_to(&mut self, reg: &str, val_id: u32, offset: i64) {
        if let Some(align) = self.state.alloca_over_align(val_id) {
            self.state.out.emit_instr_rbp_reg("    leaq", offset, reg);
            self.state
                .out
                .emit_instr_imm_reg("    addq", (align - 1) as i64, reg);
            self.state
                .out
                .emit_instr_imm_reg("    andq", -(align as i64), reg);
        } else {
            self.state.out.emit_instr_rbp_reg("    leaq", offset, reg);
        }
    }

    pub(super) fn emit_slot_addr_to_secondary_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
        self.state.reg_cache.invalidate_sec(); // clobbers %rcx
        if is_alloca {
            self.emit_alloca_addr_to("rcx", val_id, slot.0);
        } else if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rcx");
        } else {
            self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rcx");
        }
    }

    pub(super) fn emit_add_secondary_to_acc_impl(&mut self) {
        self.state.emit("    addq %rcx, %rax");
        self.state.reg_cache.invalidate_acc();
    }

    pub(super) fn emit_gep_direct_const_impl(&mut self, slot: StackSlot, offset: i64) {
        let folded = slot.0 + offset;
        self.state.out.emit_instr_rbp_reg("    leaq", folded, "rax");
        self.state.reg_cache.invalidate_acc();
    }

    pub(super) fn emit_gep_indirect_const_impl(
        &mut self,
        slot: StackSlot,
        offset: i64,
        val_id: u32,
    ) {
        if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rax");
        } else {
            self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
        }
        if offset != 0 {
            self.state
                .out
                .emit_instr_mem_reg("    leaq", offset, "rax", "rax");
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// Emit leaq (%base, %index), %dest for GEP with both operands in registers.
    /// If dest is also register-allocated, emits directly to the dest register.
    /// Otherwise, emits to %rax and stores via store_rax_to.
    pub(super) fn emit_leaq_base_index_impl(
        &mut self,
        base_reg: super::super::super::regalloc::PhysReg,
        index_reg: super::super::super::regalloc::PhysReg,
        dest: &Value,
        dest_phys: Option<super::super::super::regalloc::PhysReg>,
    ) {
        use super::emit::phys_reg_name;
        let base_name = phys_reg_name(base_reg);
        let index_name = phys_reg_name(index_reg);

        if let Some(dp) = dest_phys {
            let dest_name = phys_reg_name(dp);
            self.state.emit_fmt(format_args!(
                "    leaq (%{}, %{}), %{}",
                base_name, index_name, dest_name
            ));
            self.state.reg_cache.invalidate_acc();
        } else {
            self.state.emit_fmt(format_args!(
                "    leaq (%{}, %{}), %rax",
                base_name, index_name
            ));
            // store_rax_to establishes the acc-cache handoff (it marks dest
            // as the accumulator content, including the immediately-consumed
            // no-home case).  Invalidating AFTER it strands the value: the
            // consumer then finds no cache entry, no register, no slot and
            // fabricates a value (sqlite -O2 sqlite3KeyInfoAlloc: the
            // `p->aSortFlags = &p->aColl[N+X]` store read a fabricated 0).
            self.store_rax_to(dest);
        }
    }

    /// Emit `leaq sym(, %index, scale), %dest` for a GEP with a symbol
    /// (GlobalAddr) base and a register-resident index. Mirrors
    /// `emit_load_indexed_sym_impl` but emits `leaq` (address compute)
    /// instead of `movq` (load), and records the dest's home via the
    /// acc-cache handoff (store_rax_to) so the consumer does not hit the
    /// operand_to_rax "no register home" ICE.
    pub(super) fn emit_leaq_sym_index_impl(
        &mut self,
        dest: &Value,
        sym: &str,
        index: &Value,
        shift: u8,
        disp: i64,
    ) -> bool {
        let base_sym = sym.split(['+', '-']).next().unwrap_or(sym);
        if self.state.needs_got_for_addr(base_sym) {
            return false;
        }
        if shift > 3 {
            return false;
        }
        let Some(&x) = self.reg_assignments.get(&index.0) else {
            return false;
        };
        if is_xmm_reg(x) {
            return false;
        }
        let index_name = phys_reg_name(x);
        let mem = if self.state.pic_mode {
            self.state
                .out
                .emit_instr_sym_base_reg("    leaq", sym, "rip", "rcx");
            Self::sib_mem64("rcx", index_name, shift, disp)
        } else {
            Self::sib_mem64_sym(sym, index_name, shift, disp)
        };
        if let Some(&d_reg) = self.reg_assignments.get(&dest.0) {
            if !is_xmm_reg(d_reg) {
                let d_name = phys_reg_name(d_reg);
                self.state
                    .emit_fmt(format_args!("    leaq {}, %{}", mem, d_name));
                self.state.reg_cache.invalidate_acc();
                return true;
            }
        }
        self.state.emit_fmt(format_args!("    leaq {}, %rax", mem));
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
        true
    }

    pub(super) fn emit_gep_add_const_to_acc_impl(&mut self, offset: i64) {
        if offset != 0 {
            self.state.out.emit_instr_imm_reg("    addq", offset, "rax");
        }
        self.state.reg_cache.invalidate_acc();
    }

    pub(super) fn emit_add_imm_to_acc_impl(&mut self, imm: i64) {
        self.state.out.emit_instr_imm_reg("    addq", imm, "rax");
        self.state.reg_cache.invalidate_acc();
    }

    pub(super) fn emit_round_up_acc_to_16_impl(&mut self) {
        self.state.emit("    addq $15, %rax");
        self.state.emit("    andq $-16, %rax");
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    pub(super) fn emit_sub_sp_by_acc_impl(&mut self) {
        self.state.emit("    subq %rax, %rsp");
    }

    pub(super) fn emit_mov_sp_to_acc_impl(&mut self) {
        self.state.emit("    movq %rsp, %rax");
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    pub(super) fn emit_mov_acc_to_sp_impl(&mut self) {
        self.state.emit("    movq %rax, %rsp");
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    pub(super) fn emit_align_acc_impl(&mut self, align: usize) {
        self.state
            .out
            .emit_instr_imm_reg("    addq", (align - 1) as i64, "rax");
        self.state
            .out
            .emit_instr_imm_reg("    andq", -(align as i64), "rax");
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    pub(super) fn emit_memcpy_load_dest_addr_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
        if is_alloca {
            self.emit_alloca_addr_to("rdi", val_id, slot.0);
        } else if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rdi");
        } else {
            self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rdi");
        }
    }

    pub(super) fn emit_memcpy_load_src_addr_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
        if is_alloca {
            self.emit_alloca_addr_to("rsi", val_id, slot.0);
        } else if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rsi");
        } else {
            self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rsi");
        }
    }

    pub(super) fn emit_alloca_aligned_addr_impl(&mut self, slot: StackSlot, val_id: u32) {
        let align = self
            .state
            .alloca_over_align(val_id)
            .expect("alloca must have over-alignment for aligned addr emission");
        self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rcx");
        self.state
            .out
            .emit_instr_imm_reg("    addq", (align - 1) as i64, "rcx");
        self.state
            .out
            .emit_instr_imm_reg("    andq", -(align as i64), "rcx");
    }

    pub(super) fn emit_alloca_aligned_addr_to_acc_impl(&mut self, slot: StackSlot, val_id: u32) {
        let align = self
            .state
            .alloca_over_align(val_id)
            .expect("alloca must have over-alignment for aligned addr emission");
        self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rax");
        self.state
            .out
            .emit_instr_imm_reg("    addq", (align - 1) as i64, "rax");
        self.state
            .out
            .emit_instr_imm_reg("    andq", -(align as i64), "rax");
        self.state.reg_cache.invalidate_acc();
    }

    pub(super) fn emit_acc_to_secondary_impl(&mut self) {
        self.state.emit("    movq %rax, %rcx");
    }

    pub(super) fn emit_memcpy_store_dest_from_acc_impl(&mut self) {
        self.state.emit("    movq %rcx, %rdi");
    }

    pub(super) fn emit_memcpy_store_src_from_acc_impl(&mut self) {
        self.state.emit("    movq %rcx, %rsi");
    }

    /// Inline a fixed-size `memcpy(dest, src, size)` call (from
    /// `__builtin_memcpy` with a constant size). `dest`/`src` are generic
    /// pointer operands; load them into rdi/rsi and run the fixed-size copy.
    /// This turns the bundled SIMD headers' `__builtin_memcpy(&x, &y, 16)`
    /// software fallbacks into a single movdqu pair instead of a libc call.
    /// Register home of a value operand, if it has one (`None` for constants
    /// and stack-resident values).
    fn operand_reg_home(&self, op: &Operand) -> Option<u8> {
        match op {
            Operand::Value(v) => self.reg_assignments.get(&v.0).map(|r| r.0),
            Operand::Const(_) => None,
        }
    }

    /// Stage `dest → %rdi`, `src → %rsi` without the read-after-clobber
    /// hazard: with parameters kept in their ABI registers across inline
    /// expansions (`regalloc::x86_param_caller_homes_safe`), `memcpy(d, s)`
    /// called as `f(char *s, char *d)` has `s` homed in %rdi.  Loading `d`
    /// first would destroy it.  The order follows the homes; the full swap
    /// (`s` in %rdi *and* `d` in %rsi) is one `xchgq`.
    fn stage_copy_operands(&mut self, dest: &Operand, src: &Operand) {
        const RDI: u8 = 14;
        const RSI: u8 = 15;
        let src_in_rdi = self.operand_reg_home(src) == Some(RDI);
        let dest_in_rsi = self.operand_reg_home(dest) == Some(RSI);
        if src_in_rdi && dest_in_rsi {
            self.state.emit("    xchgq %rdi, %rsi");
            self.state.reg_cache.invalidate_all();
        } else if src_in_rdi {
            self.operand_to_reg(src, "rsi");
            self.operand_to_reg(dest, "rdi");
        } else {
            self.operand_to_reg(dest, "rdi");
            self.operand_to_reg(src, "rsi");
        }
    }

    /// Materialise the call result (= dest) from the copy kept in %rdx.
    /// %rdx is untouched by both expansions, is never a home of a value live
    /// across a call point, and every parameter is dead after the expansion
    /// whenever the allocator left parameters in caller-saved registers.
    fn store_inline_libc_result(&mut self, result: Option<&Value>) {
        if let Some(r) = result {
            self.state.emit("    movq %rdx, %rax");
            crate::backend::traits::ArchCodegen::emit_store_result(self, r);
        }
    }

    pub(super) fn emit_inline_memcpy_call_impl(
        &mut self,
        dest: &Operand,
        src: &Operand,
        size: usize,
        result: Option<&Value>,
    ) {
        self.stage_copy_operands(dest, src);
        if result.is_some() {
            self.state.emit("    movq %rdi, %rdx");
        }
        self.emit_memcpy_impl_impl(size);
        self.store_inline_libc_result(result);
    }

    pub(super) fn emit_inline_memmove_call_impl(
        &mut self,
        dest: &Operand,
        src: &Operand,
        size: usize,
    ) {
        // Source slot may still be a deferred vector result.
        self.flush_pending_vec_store_impl();
        self.operand_to_reg(dest, "rdi");
        self.operand_to_reg(src, "rsi");
        // memmove must handle overlapping ranges. With dst > src, a forward
        // copy would read source bytes that were already overwritten, so the
        // copy must run backward (DF=1). Direction flag is restored after.
        let lbl = self.state.next_label_id();
        let done = format!(".Lmmv_done_{}", lbl);
        let bwd = format!(".Lmmv_bwd_{}", lbl);
        self.state.emit("    cmpq %rsi, %rdi");
        self.state.emit_fmt(format_args!("    je {}", done)); // same address: nothing to copy
        self.state.emit_fmt(format_args!("    ja {}", bwd)); // dst > src: backward
        self.state
            .out
            .emit_instr_imm_reg("    movq", size as i64, "rcx");
        self.state.emit("    rep movsb");
        self.state.emit_fmt(format_args!("    jmp {}", done));
        self.state.emit_fmt(format_args!("{}:", bwd));
        self.state.emit("    std");
        self.state
            .out
            .emit_instr_imm_reg("    movq", size as i64, "rcx");
        // rep movsb with DF=1 copies from high to low: source end -> dest end.
        self.state.emit("    rep movsb");
        self.state.emit("    cld");
        self.state.emit_fmt(format_args!("{}:", done));
    }

    // ------------------------------------------------------------------
    // Fixed-size memset expansion.
    //
    // Every constant-size `memset` used to be a `call memset@PLT`: 10
    // instructions with a callee-saved spill for `memset(p, 0, 15)` where
    // Clang emits two overlapping `movq $0` stores and GCC a 8/4/2/1 ladder.
    // The lowering below is driven by the CPU tuning row
    // (`X86Tune::memset_strategy`, docs/CPU_MODEL_AUDIT.md §4):
    //
    // * size < 16             two overlapping scalar stores of the largest
    //                         power-of-two width ≤ size (Clang's shape; one
    //                         store fewer than GCC's ladder for 3/5/6/7/9..15).
    // * ≤ 8 vector stores     straight-line `movups`/`vmovdqu` (Clang
    //                         `MaxStoresPerMemset`); the last store overlaps
    //                         the previous one instead of a narrower ladder.
    // * ≥ rep_stosb_threshold `rep stosb` on ERMS rows (glibc
    //   (2048 B)              `__x86_rep_stosb_threshold`; glibc's own memset
    //                         takes this path at the same size).
    // * otherwise             counted loop, two vector stores per iteration,
    //                         vector remainder with one overlapping store.
    // * above ¼ L3 (ERMS) /   `LibCall` — not expanded (`inline_memset_len`
    //   8 KiB (no ERMS)       answers None), glibc's non-temporal path wins.
    // * `-mno-sse`            scalar 8-byte stores ≤ 64 B, `rep stosb` above
    //                         (kernel boot code, no libc, CR4.OSFXSR=0).
    //
    // The vector width follows the `-march` contract exactly like the block
    // copy path (`block_copy_vector_bytes`): 32-byte YMM only when AVX2 is
    // enabled and the row does not split 256-bit unaligned accesses; VEX-128
    // encodings whenever AVX2 is enabled so no legacy-SSE instruction is
    // issued while an upper YMM half may be dirty.
    // ------------------------------------------------------------------

    /// Store `width` ∈ {1,2,4,8} bytes of the fill pattern at `off(%rdi)`.
    /// `rax_pat` tracks whether %rax already holds the 64-bit broadcast of
    /// the fill byte (constant fills materialise it lazily, at most once).
    fn emit_memset_scalar_store(
        &mut self,
        off: i64,
        width: usize,
        const_byte: Option<u8>,
        rax_pat: &mut bool,
    ) {
        let mem = if off == 0 {
            "(%rdi)".to_string()
        } else {
            format!("{}(%rdi)", off)
        };
        match (width, const_byte) {
            (8, Some(b)) => {
                let pat = (b as u64).wrapping_mul(0x0101_0101_0101_0101);
                let sx = pat as i64;
                if i32::try_from(sx).is_ok() {
                    // 0x00 / 0xFF: sign-extended imm32 form (`movq $0` / `movq $-1`).
                    self.state
                        .emit_fmt(format_args!("    movq ${}, {}", sx, mem));
                } else {
                    if !*rax_pat {
                        self.state
                            .emit_fmt(format_args!("    movabsq $0x{:x}, %rax", pat));
                        *rax_pat = true;
                    }
                    self.state
                        .emit_fmt(format_args!("    movq %rax, {}", mem));
                }
            }
            (8, None) => self
                .state
                .emit_fmt(format_args!("    movq %rax, {}", mem)),
            (4, Some(b)) => self.state.emit_fmt(format_args!(
                "    movl $0x{:x}, {}",
                (b as u32).wrapping_mul(0x0101_0101),
                mem
            )),
            (4, None) => self
                .state
                .emit_fmt(format_args!("    movl %eax, {}", mem)),
            (2, Some(b)) => self.state.emit_fmt(format_args!(
                "    movw $0x{:x}, {}",
                (b as u16).wrapping_mul(0x0101),
                mem
            )),
            (2, None) => self
                .state
                .emit_fmt(format_args!("    movw %ax, {}", mem)),
            (_, Some(b)) => self
                .state
                .emit_fmt(format_args!("    movb $0x{:x}, {}", b, mem)),
            (_, None) => self
                .state
                .emit_fmt(format_args!("    movb %al, {}", mem)),
        }
    }

    /// Fill `[base, base+len)` (len < 16) with at most two overlapping
    /// scalar stores: width w = largest power of two ≤ len at `base`, and a
    /// second w-byte store ending exactly at `base+len` when len > w.
    fn emit_memset_scalar_tail(
        &mut self,
        base: i64,
        len: usize,
        const_byte: Option<u8>,
        rax_pat: &mut bool,
    ) {
        if len == 0 {
            return;
        }
        let w = if len >= 8 {
            8
        } else if len >= 4 {
            4
        } else if len >= 2 {
            2
        } else {
            1
        };
        self.emit_memset_scalar_store(base, w, const_byte, rax_pat);
        if len > w {
            self.emit_memset_scalar_store(base + (len - w) as i64, w, const_byte, rax_pat);
        }
    }

    /// Fill `[base, base+len)` (len ≥ 16) with straight-line vector stores:
    /// w = 32 when `use_ymm` and len ≥ 32, else 16; stores at base, base+w,
    /// …; the final store overlaps so that no narrower ladder is needed.
    fn emit_memset_vec_region(&mut self, base: i64, len: usize, use_ymm: bool, vex: bool) {
        debug_assert!(len >= 16);
        let (w, mv, reg) = if use_ymm && len >= 32 {
            (32usize, "vmovdqu", "%ymm0")
        } else if vex {
            (16usize, "vmovdqu", "%xmm0")
        } else {
            (16usize, "movups", "%xmm0")
        };
        let store = |this: &mut Self, off: i64| {
            if off == 0 {
                this.state.emit_fmt(format_args!("    {} {}, (%rdi)", mv, reg));
            } else {
                this.state
                    .emit_fmt(format_args!("    {} {}, {}(%rdi)", mv, reg, off));
            }
        };
        let mut off = 0usize;
        while off + w <= len {
            store(self, base + off as i64);
            off += w;
        }
        if off < len {
            store(self, base + (len - w) as i64);
        }
    }

    /// Materialise the fill pattern in %xmm0 (and %ymm0 when `use_ymm`).
    /// Zero and all-ones use the dependency-breaking idioms; other constant
    /// bytes broadcast a 32-bit immediate; a runtime byte broadcasts the
    /// byte staged in %ecx (raw under AVX2, 32-bit pattern under SSE2).
    fn emit_memset_vec_pattern(&mut self, const_byte: Option<u8>, use_ymm: bool, vex: bool) {
        match const_byte {
            Some(0) => {
                if vex {
                    // VEX-128 zeroing idiom zeroes the full %ymm0.
                    self.state.emit("    vpxor %xmm0, %xmm0, %xmm0");
                } else {
                    self.state.emit("    pxor %xmm0, %xmm0");
                }
            }
            Some(0xFF) => {
                if use_ymm {
                    self.state.emit("    vpcmpeqd %ymm0, %ymm0, %ymm0");
                } else if vex {
                    self.state.emit("    vpcmpeqd %xmm0, %xmm0, %xmm0");
                } else {
                    self.state.emit("    pcmpeqd %xmm0, %xmm0");
                }
            }
            Some(b) => {
                self.state.emit_fmt(format_args!(
                    "    movl $0x{:x}, %eax",
                    (b as u32).wrapping_mul(0x0101_0101)
                ));
                if use_ymm {
                    self.state.emit("    vmovd %eax, %xmm0");
                    self.state.emit("    vpbroadcastd %xmm0, %ymm0");
                } else if vex {
                    self.state.emit("    vmovd %eax, %xmm0");
                    self.state.emit("    vpbroadcastd %xmm0, %xmm0");
                } else {
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                }
            }
            None => {
                // Runtime byte.  AVX2: %ecx holds the raw value; broadcast
                // its low byte lane directly (GCC/Clang/ICX shape, 2
                // instructions).  SSE2: %ecx holds the 32-bit pattern
                // (imull $0x01010101 in the caller); `pshufd $0` replicates it.
                if use_ymm {
                    self.state.emit("    vmovd %ecx, %xmm0");
                    self.state.emit("    vpbroadcastb %xmm0, %ymm0");
                } else if vex {
                    self.state.emit("    vmovd %ecx, %xmm0");
                    self.state.emit("    vpbroadcastb %xmm0, %xmm0");
                } else {
                    self.state.emit("    movd %ecx, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                }
            }
        }
    }

    /// `rep stosb`: %al = fill byte, %rcx = count, %rdi = dest (DF = 0 per
    /// the SysV ABI at every call boundary).
    fn emit_memset_rep_stosb(&mut self, size: usize, const_byte: Option<u8>) {
        match const_byte {
            Some(0) => self.state.emit("    xorl %eax, %eax"),
            Some(b) => self
                .state
                .emit_fmt(format_args!("    movl ${}, %eax", b)),
            None => {} // %rax already holds the broadcast; %al is the byte.
        }
        self.state
            .out
            .emit_instr_imm_reg("    movq", size as i64, "rcx");
        self.state.emit("    rep stosb");
    }

    /// Inline `memset(dest, value, size)`; see the section comment above.
    pub(super) fn emit_inline_memset_call_impl(
        &mut self,
        dest: &Operand,
        value: &Operand,
        size: usize,
        result: Option<&Value>,
    ) {
        use crate::backend::x86::cpu_model::CopyStrategy;
        // %xmm0 is clobbered: the vector last-store peephole and the
        // deferred-store cache must not reuse a stale value (same soundness
        // rule as emit_memcpy_impl_impl).
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();

        let const_byte: Option<u8> = match value {
            Operand::Const(c) => c.to_i64().map(|v| v as u8),
            Operand::Value(_) => None,
        };
        // Stage the fill value before the destination: a runtime byte homed
        // in %rdi must be read before %rdi is overwritten, and no allocation
        // is ever homed in %rax, so the destination load cannot disturb it.
        let mut rax_pat = false;
        let vb = self.tune.block_copy_vector_bytes(self.avx2_enabled);
        let strategy = self.tune.memset_strategy(size, vb);
        // Which expansion consumes a runtime fill byte, decided up front so
        // the byte is staged where that expansion reads it (see below).
        let rep = size > 0
            && (self.no_sse && size > 64
                || !self.no_sse
                    && size >= 16
                    && strategy == CopyStrategy::RepMovsb
                    && std::env::var_os("CCC_NO_REP_MOVSB").is_none());
        let vector = !self.no_sse && size >= 16 && !rep;
        // Vector expansions broadcast from a GPR that is dead afterwards;
        // %rcx is used (not %rax) so the staging copy is never kept alive by
        // the peephole's conservative "return value live at ret" rule in a
        // void function, and the loop counter re-uses it only after the
        // broadcast.  Scalar tails and `rep stosb` read %rax/%al.
        let fill_gpr = if vector { "rcx" } else { "rax" };
        if const_byte.is_none() {
            self.operand_to_reg(value, fill_gpr);
        }
        self.operand_to_reg(dest, "rdi");
        if result.is_some() {
            self.state.emit("    movq %rdi, %rdx");
        }
        if size == 0 {
            self.store_inline_libc_result(result);
            self.state.reg_cache.invalidate_all();
            return;
        }

        // Runtime fill byte: broadcast only as wide as the chosen expansion
        // actually consumes (FOLLOWUP_CPU_MODEL #2).
        //   * `rep stosb` reads %al only — no broadcast at all.
        //   * Vector paths under AVX2 broadcast the byte lane directly
        //     (`vmovd` + `vpbroadcastb`, 2 µops, p5-free on GLC/RPC); the
        //     GPR multiply chain (movzbl + movabs + 3-cycle imul) is gone.
        //   * Scalar tails and the SSE2 baseline widen through the integer
        //     multiplier: a 32-bit `imull $0x01010101` (imm32, one
        //     instruction) is enough for a `movl` tail or a `pshufd`
        //     broadcast; only an 8-byte `movq %rax` store needs the 64-bit
        //     `movabsq` + `imulq` form.
        // `rax_pat` records that %rax holds the *64-bit* pattern; the
        // scalar-store helper relies on it only for 8-byte stores.
        if const_byte.is_none() {
            let vex = vb == 32;
            if rep {
                // %al already is the fill byte.
            } else if vector && vex {
                // Consumed by emit_memset_vec_pattern (vpbroadcastb from %ecx).
            } else if vector {
                // 32-bit pattern in %ecx for the SSE2 pshufd broadcast.
                self.state.emit("    movzbl %cl, %ecx");
                self.state.emit("    imull $0x1010101, %ecx, %ecx");
            } else if size < 8 {
                // 32-bit pattern: movl/movw/movb tails.
                self.state.emit("    movzbl %al, %eax");
                self.state.emit("    imull $0x1010101, %eax, %eax");
            } else {
                // 64-bit pattern: (uint8)c * 0x0101010101010101.
                self.state.emit("    movzbl %al, %eax");
                self.state.emit("    movabsq $0x101010101010101, %rcx");
                self.state.emit("    imulq %rcx, %rax");
                rax_pat = true;
            }
        }

        if self.no_sse {
            // No vector instruction may be emitted (CR4.OSFXSR may be 0).
            if size > 64 {
                self.emit_memset_rep_stosb(size, const_byte);
            } else {
                let mut off = 0usize;
                while off + 8 <= size {
                    self.emit_memset_scalar_store(off as i64, 8, const_byte, &mut rax_pat);
                    off += 8;
                }
                if off < size {
                    if size >= 8 {
                        self.emit_memset_scalar_store(
                            (size - 8) as i64,
                            8,
                            const_byte,
                            &mut rax_pat,
                        );
                    } else {
                        self.emit_memset_scalar_tail(0, size, const_byte, &mut rax_pat);
                    }
                }
            }
            self.store_inline_libc_result(result);
            self.state.reg_cache.invalidate_all();
            return;
        }

        if size < 16 {
            self.emit_memset_scalar_tail(0, size, const_byte, &mut rax_pat);
            self.store_inline_libc_result(result);
            self.state.reg_cache.invalidate_all();
            return;
        }

        if strategy == CopyStrategy::RepMovsb && std::env::var_os("CCC_NO_REP_MOVSB").is_none() {
            self.emit_memset_rep_stosb(size, const_byte);
            self.store_inline_libc_result(result);
            self.state.reg_cache.invalidate_all();
            return;
        }

        let vex = vb == 32;
        let use_ymm = vex && size >= 32;
        self.emit_memset_vec_pattern(const_byte, use_ymm, vex);

        if strategy == CopyStrategy::InlineUnrolled {
            self.emit_memset_vec_region(0, size, use_ymm, vex);
        } else {
            // Counted loop: two vector stores (2 × vb bytes) per iteration,
            // then the remainder relative to the advanced %rdi.  A remainder
            // shorter than 16 bytes is covered by one 16-byte store that
            // overlaps the already-filled prefix (negative displacement).
            let chunk = 2 * vb;
            let full_chunks = size / chunk;
            let remainder = size % chunk;
            debug_assert!(full_chunks > 0);
            let (mv, reg) = if use_ymm {
                ("vmovdqu", "%ymm0")
            } else if vex {
                ("vmovdqu", "%xmm0")
            } else {
                ("movups", "%xmm0")
            };
            self.state
                .out
                .emit_instr_imm_reg("    movq", full_chunks as i64, "rcx");
            let loop_label = format!(".Lmset_loop_{}", self.state.next_label_id());
            self.state.emit_fmt(format_args!("{}:", loop_label));
            self.state.emit_fmt(format_args!("    {} {}, (%rdi)", mv, reg));
            self.state
                .emit_fmt(format_args!("    {} {}, {}(%rdi)", mv, reg, vb));
            self.state
                .emit_fmt(format_args!("    addq ${}, %rdi", chunk));
            self.state.emit("    decq %rcx");
            self.state.emit_fmt(format_args!("    jne {}", loop_label));
            if remainder >= 16 {
                self.emit_memset_vec_region(0, remainder, use_ymm, vex);
            } else if remainder > 0 {
                let mv16 = if vex { "vmovdqu" } else { "movups" };
                self.state.emit_fmt(format_args!(
                    "    {} %xmm0, -{}(%rdi)",
                    mv16,
                    16 - remainder
                ));
            }
        }
        if use_ymm {
            // Arm the epilogue vzeroupper (upper %ymm0 half is dirty).
            self.state.dirty_upper_ymm = true;
        }
        self.store_inline_libc_result(result);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_memcpy_impl_impl(&mut self, size: usize) {
        // SOUNDNESS (v9 / BUG-004): 16/32-byte copies go through %xmm0/%ymm0/%ymm1.
        // The vector last-store peephole and deferred-store cache assume those
        // registers still hold the last SIMD intrinsic result. Clobbering them
        // here without invalidation made inlined fold_state_1
        //   *xmm_crc3 = _mm_xor_si128(x_low, x_high)
        // reuse a stale %xmm0 (the just-copied *crc3 = 0) instead of reloading
        // x_high from its slot. zlib-ng CRC32 then failed iff (n % 64) ∈ [16,31].
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();

        // A complete 64-byte assignment is the profitable boundary for YMM:
        // four memory instructions replace eight XMM instructions, and an
        // explicit vzeroupper prevents any later legacy-SSE transition. Smaller
        // 32/48-byte copies stay XMM (the vzeroupper cost erases their one- or
        // two-instruction saving in hot struct loops).
        if size == 64
            && self.avx2_enabled
            && !self.no_sse
            && std::env::var_os("CCC_NO_64B_YMM_COPY").is_none()
        {
            self.state.emit("    vmovdqu (%rsi), %ymm0");
            self.state.emit("    vmovdqu %ymm0, (%rdi)");
            self.state.emit("    vmovdqu 32(%rsi), %ymm1");
            self.state.emit("    vmovdqu %ymm1, 32(%rdi)");
            self.state.emit("    vzeroupper");
            // The upper halves are clean again right here; any later 256-bit
            // op re-arms the epilogue vzeroupper via dirty_upper_ymm.
            self.state.dirty_upper_ymm = false;
            self.state.reg_cache.invalidate_all();
            return;
        }

        if size <= 64 {
            let mut offset = 0usize;
            let mut remaining = size;
            // AVX->SSE TRANSITION: do not use 256-bit ymm for small inline
            // copies.
            //
            // A 32-byte chunk as one `vmovdqu %ymm0` saves exactly ONE
            // instruction over two 16-byte moves. But it leaves the upper half
            // of ymm0 dirty, and every legacy-SSE instruction executed
            // afterwards -- including ordinary scalar FP such as movsd/mulsd/
            // addsd, which this backend emits everywhere -- pays a CPU state
            // transition of roughly 70 cycles on Intel. A struct-copying loop
            // alternates between the two domains on every iteration and pays it
            // twice per iteration.
            //
            // Measured on tests/benchmark/programs/struct_copy.c (48-byte
            // Particle, three copies per iteration, 2,000,000 iterations),
            // -O2, same host, median of 7 runs:
            //
            //   ymm chunk + legacy SSE tail   5287 ms   (the shipped behaviour)
            //   ymm chunk + VEX tail          2516 ms   (removes one of two)
            //   no ymm, 16-byte moves          113 ms   <- 46x faster
            //   gcc -O2                         21 ms
            //
            // One saved instruction is not worth two state transitions, so
            // small copies stay in the 128-bit domain. The 16-byte arm below
            // keeps the VEX/legacy choice consistent for the same reason,
            // which matters if this constant is ever turned back on.
            //
            // This does NOT affect the large-copy path (size > 64), which uses
            // its own loop further down.
            const USE_YMM_FOR_SMALL_MEMCPY: bool = false;
            let mut used_ymm = false;
            while remaining > 0 {
                if USE_YMM_FOR_SMALL_MEMCPY && remaining >= 32 {
                    // Use AVX2 256-bit vmovdqu for 32-byte chunks (1 instruction
                    // instead of 2x movdqu). ymm scratch: ymm0 = xmm0's extension.
                    if offset == 0 {
                        self.state.emit("    vmovdqu (%rsi), %ymm0");
                        self.state.emit("    vmovdqu %ymm0, (%rdi)");
                    } else {
                        self.state
                            .emit_fmt(format_args!("    vmovdqu {}(%rsi), %ymm0", offset));
                        self.state
                            .emit_fmt(format_args!("    vmovdqu %ymm0, {}(%rdi)", offset));
                    }
                    offset += 32;
                    remaining -= 32;
                    used_ymm = true;
                } else if remaining >= 16 && !self.no_sse {
                    // GCC's -mno-sse contract forbids ALL xmm/ymm usage (the
                    // kernel decompressor and early boot run with CR4.OSFXSR=0,
                    // where any SSE instruction faults -- reproduced: 16-byte
                    // gate_desc copy in load_stage2_idt crashed the Cachymod
                    // 6.18.46 QEMU boot). With SSE disabled the 16 bytes fall
                    // through to two movq, matching GCC.
                    //
                    // AVX->SSE TRANSITION PENALTY. A 48-byte struct copies as
                    // one 32-byte ymm chunk plus a 16-byte tail. Emitting the
                    // tail as legacy SSE (`movdqu %xmm0`) while the upper half
                    // of ymm0 is still dirty from the vmovdqu above costs a
                    // full state transition on Intel -- measured on
                    // tests/benchmark/programs/struct_copy.c (3 such copies per
                    // iteration, 2M iterations): 5287 ms versus GCC's 21 ms.
                    //
                    // The fix is NOT to insert vzeroupper (that has its own
                    // cost and would run per copy); it is to keep the tail in
                    // the VEX domain. `vmovdqu %xmm0` zeroes the upper bits by
                    // definition, so no transition ever occurs. Same length,
                    // same semantics.
                    //
                    // Legacy SSE is still used when no ymm was touched (a plain
                    // 16..31 byte copy), so targets without AVX are unaffected.
                    let (ld, st) = if used_ymm {
                        ("    vmovdqu", "    vmovdqu")
                    } else {
                        ("    movdqu", "    movdqu")
                    };
                    if offset == 0 {
                        self.state.emit_fmt(format_args!("{} (%rsi), %xmm0", ld));
                        self.state.emit_fmt(format_args!("{} %xmm0, (%rdi)", st));
                    } else {
                        self.state
                            .emit_fmt(format_args!("{} {}(%rsi), %xmm0", ld, offset));
                        self.state
                            .emit_fmt(format_args!("{} %xmm0, {}(%rdi)", st, offset));
                    }
                    offset += 16;
                    remaining -= 16;
                } else if remaining >= 8 {
                    if offset == 0 {
                        self.state.emit("    movq (%rsi), %rax");
                        self.state.emit("    movq %rax, (%rdi)");
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movq {}(%rsi), %rax", offset));
                        self.state
                            .emit_fmt(format_args!("    movq %rax, {}(%rdi)", offset));
                    }
                    offset += 8;
                    remaining -= 8;
                } else if remaining >= 4 {
                    if offset == 0 {
                        self.state.emit("    movl (%rsi), %eax");
                        self.state.emit("    movl %eax, (%rdi)");
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movl {}(%rsi), %eax", offset));
                        self.state
                            .emit_fmt(format_args!("    movl %eax, {}(%rdi)", offset));
                    }
                    offset += 4;
                    remaining -= 4;
                } else if remaining >= 2 {
                    if offset == 0 {
                        self.state.emit("    movw (%rsi), %ax");
                        self.state.emit("    movw %ax, (%rdi)");
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movw {}(%rsi), %ax", offset));
                        self.state
                            .emit_fmt(format_args!("    movw %ax, {}(%rdi)", offset));
                    }
                    offset += 2;
                    remaining -= 2;
                } else {
                    if offset == 0 {
                        self.state.emit("    movb (%rsi), %al");
                        self.state.emit("    movb %al, (%rdi)");
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movb {}(%rsi), %al", offset));
                        self.state
                            .emit_fmt(format_args!("    movb %al, {}(%rdi)", offset));
                    }
                    offset += 1;
                    remaining -= 1;
                }
            }
        } else {
            // Under -mno-sse no vector instruction may be emitted at all
            // (kernel boot runs with CR4.OSFXSR=0): use rep movsb.
            if self.no_sse {
                self.state
                    .out
                    .emit_instr_imm_reg("    movq", size as i64, "rcx");
                self.state.emit("    rep movsb");
                self.state.reg_cache.invalidate_all();
                return;
            }
            // Copies > 64 bytes follow the CPU tuning model
            // (`X86Tune::memcpy_strategy`, docs/CPU_MODEL_AUDIT.md §4):
            //
            // * `rep movsb` at or above glibc's `__x86_rep_movsb_threshold`
            //   on ERMS/FSRM parts (2112 B with FSRM, 8192 B with 32-byte
            //   vectors on the older ERMS cores).  Three instructions, no
            //   call, no loop, byte-granular so no alignment prologue; it is
            //   the exact path glibc's own memmove-vec-unaligned-erms takes
            //   for the same size on the same hardware.
            // * otherwise a counted vector loop whose width follows the
            //   instruction-set contract: 32-byte `vmovdqu %ymm` ONLY when
            //   AVX2 is enabled by `-march` (and the part does not split
            //   256-bit unaligned loads, SNB/IVB), 16-byte SSE2 `movdqu`
            //   otherwise.  The previous lowering emitted `%ymm` here
            //   unconditionally, which SIGILLs on any pre-AVX host at plain
            //   `-march=x86-64` (regression: tests/regression/
            //   cpu_model_memcpy_raptorlake.c).
            use crate::backend::x86::cpu_model::CopyStrategy;
            let vb = self.tune.block_copy_vector_bytes(self.avx2_enabled);
            let strategy = self.tune.memcpy_strategy(size, vb);
            if strategy == CopyStrategy::RepMovsb
                && std::env::var_os("CCC_NO_REP_MOVSB").is_none()
            {
                self.state
                    .out
                    .emit_instr_imm_reg("    movq", size as i64, "rcx");
                self.state.emit("    rep movsb");
                self.state.reg_cache.invalidate_all();
                return;
            }
            let use_ymm = vb == 32;
            let (mv, r0, r1) = if use_ymm {
                ("vmovdqu", "%ymm0", "%ymm1")
            } else {
                ("movdqu", "%xmm0", "%xmm1")
            };
            // Up to eight vector stores (Clang `MaxStoresPerMemcpy`, GCC
            // `move_by_pieces`) are cheaper straight-line than as a 2–4
            // iteration counted loop: no %rcx/pointer increments, no taken
            // branches, and every load/store pair is independent so the
            // OoO core streams them.  Alternate the two scratch registers so
            // consecutive pairs never serialise on one register.
            let unrolled = strategy == CopyStrategy::InlineUnrolled;
            let chunk = if unrolled { vb } else { 2 * vb };
            let full_chunks = size / chunk;
            let remainder = size % chunk;

            if unrolled {
                for i in 0..full_chunks {
                    let reg = if i % 2 == 0 { r0 } else { r1 };
                    let off = i * vb;
                    self.state
                        .emit_fmt(format_args!("    {} {}(%rsi), {}", mv, off, reg));
                    self.state
                        .emit_fmt(format_args!("    {} {}, {}(%rdi)", mv, reg, off));
                }
                if use_ymm && full_chunks > 0 {
                    self.state.dirty_upper_ymm = true;
                }
            } else if full_chunks > 0 {
                // Counted loop: two vector moves (2 × vb bytes) per iteration.
                self.state
                    .out
                    .emit_instr_imm_reg("    movq", full_chunks as i64, "rcx");
                let loop_label = format!(".Lmcpy_loop_{}", self.state.next_label_id());
                self.state.emit_fmt(format_args!("{}:", loop_label));
                self.state
                    .emit_fmt(format_args!("    {} (%rsi), {}", mv, r0));
                self.state
                    .emit_fmt(format_args!("    {} {}, (%rdi)", mv, r0));
                self.state
                    .emit_fmt(format_args!("    {} {}(%rsi), {}", mv, vb, r1));
                self.state
                    .emit_fmt(format_args!("    {} {}, {}(%rdi)", mv, r1, vb));
                self.state
                    .emit_fmt(format_args!("    addq ${}, %rsi", chunk));
                self.state
                    .emit_fmt(format_args!("    addq ${}, %rdi", chunk));
                self.state.emit("    decq %rcx");
                self.state.emit_fmt(format_args!("    jne {}", loop_label));
                if use_ymm {
                    // The upper YMM halves are dirty: arm the epilogue
                    // vzeroupper so later legacy-SSE code (scalar FP in this
                    // backend) does not pay the AVX/SSE transition.
                    self.state.dirty_upper_ymm = true;
                }
            }

            // Remainder ladder.  After the counted loop %rsi/%rdi already
            // point past the copied prefix (offset 0); after the straight-line
            // form they do not, so the ladder continues at the prefix length.
            let mut offset = if unrolled { full_chunks * chunk } else { 0usize };
            let mut remaining = remainder;
            while remaining > 0 {
                if remaining >= 32 && use_ymm {
                    self.state
                        .emit_fmt(format_args!("    vmovdqu {}(%rsi), %ymm0", offset));
                    self.state
                        .emit_fmt(format_args!("    vmovdqu %ymm0, {}(%rdi)", offset));
                    self.state.dirty_upper_ymm = true;
                    offset += 32;
                    remaining -= 32;
                } else if remaining >= 16 && !self.no_sse {
                    // Stay in the VEX domain once a YMM register is dirty
                    // (see the small-copy path above for the measurement).
                    let mv16 = if use_ymm { "vmovdqu" } else { "movdqu" };
                    self.state
                        .emit_fmt(format_args!("    {} {}(%rsi), %xmm0", mv16, offset));
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, {}(%rdi)", mv16, offset));
                    offset += 16;
                    remaining -= 16;
                } else if remaining >= 8 {
                    self.state
                        .emit_fmt(format_args!("    movq {}(%rsi), %rax", offset));
                    self.state
                        .emit_fmt(format_args!("    movq %rax, {}(%rdi)", offset));
                    offset += 8;
                    remaining -= 8;
                } else if remaining >= 4 {
                    self.state
                        .emit_fmt(format_args!("    movl {}(%rsi), %eax", offset));
                    self.state
                        .emit_fmt(format_args!("    movl %eax, {}(%rdi)", offset));
                    offset += 4;
                    remaining -= 4;
                } else if remaining >= 2 {
                    self.state
                        .emit_fmt(format_args!("    movw {}(%rsi), %ax", offset));
                    self.state
                        .emit_fmt(format_args!("    movw %ax, {}(%rdi)", offset));
                    offset += 2;
                    remaining -= 2;
                } else {
                    self.state
                        .emit_fmt(format_args!("    movb {}(%rsi), %al", offset));
                    self.state
                        .emit_fmt(format_args!("    movb %al, {}(%rdi)", offset));
                    offset += 1;
                    remaining -= 1;
                }
            }
        }
    }

    // ---- Segment-prefixed memory ops ----

    pub(super) fn emit_seg_load_impl(
        &mut self,
        dest: &Value,
        ptr: &Value,
        ty: IrType,
        seg: AddressSpace,
    ) {
        let seg_prefix = match seg {
            AddressSpace::SegGs => "%gs:",
            AddressSpace::SegFs => "%fs:",
            AddressSpace::Default => {
                unreachable!("segment-prefixed op called with default address space")
            }
        };
        self.operand_to_rax(&Operand::Value(*ptr));
        self.state.emit("    movq %rax, %rcx");
        let load_instr = Self::mov_load_for_type(ty);
        let dest_reg = Self::load_dest_reg(ty);
        self.state.emit_fmt(format_args!(
            "    {} {}(%rcx), {}",
            load_instr, seg_prefix, dest_reg
        ));
        self.store_rax_to(dest);
    }

    pub(super) fn emit_seg_load_symbol_impl(
        &mut self,
        dest: &Value,
        sym: &str,
        ty: IrType,
        seg: AddressSpace,
    ) {
        let seg_prefix = match seg {
            AddressSpace::SegGs => "%gs:",
            AddressSpace::SegFs => "%fs:",
            AddressSpace::Default => {
                unreachable!("segment-prefixed op called with default address space")
            }
        };
        let load_instr = Self::mov_load_for_type(ty);
        let dest_reg = Self::load_dest_reg(ty);
        self.state.emit_fmt(format_args!(
            "    {} {}{}(%rip), {}",
            load_instr, seg_prefix, sym, dest_reg
        ));
        self.store_rax_to(dest);
    }

    /// Direct segment-relative access at constant offset: the GCC/Clang
    /// form for every glibc TLS macro. `movq %fs:16, %rax` (2 uops, no
    /// address materialization) replaces movq $16,%r; movq %r,%rcx;
    /// movq %fs:(%rcx),... The offset is printed signed decimal exactly
    /// like GAS 2.47 disassembles it.
    /// Direct segment-relative access at constant offset: the GCC/Clang
    /// form for every glibc TLS macro. `movq %fs:16, %rax` (no address
    /// materialization) replaces movq $16,%r; movq %r,%rcx;
    /// movq %fs:(%rcx),... The offset prints signed decimal, exactly the
    /// operand GAS 2.47 accepts and objdump prints.
    pub(super) fn emit_seg_load_const_addr_impl(
        &mut self,
        dest: &Value,
        addr: i64,
        ty: IrType,
        seg: AddressSpace,
    ) -> bool {
        let seg_prefix = match seg {
            AddressSpace::SegGs => "%gs:",
            AddressSpace::SegFs => "%fs:",
            AddressSpace::Default => return false,
        };
        if !matches!(
            ty,
            IrType::I8
                | IrType::U8
                | IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::I64
                | IrType::U64
                | IrType::Ptr
        ) {
            return false;
        }
        // 64-bit loads straight into a GPR home skip the rax round-trip.
        if matches!(ty, IrType::I64 | IrType::U64 | IrType::Ptr) {
            if let Some(&reg) = self.reg_assignments.get(&dest.0) {
                if !is_xmm_reg(reg) {
                    self.state.emit_fmt(format_args!(
                        "    movq {}{}, %{}",
                        seg_prefix,
                        addr,
                        phys_reg_name(reg)
                    ));
                    self.state.reg_cache.invalidate_acc();
                    return true;
                }
            }
        }
        let load_instr = Self::mov_load_for_type(ty);
        let dest_reg = Self::load_dest_reg(ty);
        self.state.emit_fmt(format_args!(
            "    {} {}{}, {}",
            load_instr, seg_prefix, addr, dest_reg
        ));
        self.store_rax_to(dest);
        true
    }

    pub(super) fn emit_seg_store_const_addr_impl(
        &mut self,
        val: &Operand,
        addr: i64,
        ty: IrType,
        seg: AddressSpace,
    ) -> bool {
        let seg_prefix = match seg {
            AddressSpace::SegGs => "%gs:",
            AddressSpace::SegFs => "%fs:",
            AddressSpace::Default => return false,
        };
        if !matches!(
            ty,
            IrType::I8
                | IrType::U8
                | IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::I64
                | IrType::U64
                | IrType::Ptr
        ) {
            return false;
        }
        let store_instr = Self::mov_store_for_type(ty);
        // Immediate store: mov $imm, %fs:OFF — zero register pressure.
        if let Operand::Const(c) = val {
            let is_32 = !matches!(ty, IrType::I64 | IrType::U64 | IrType::Ptr);
            if let Some(imm) = Self::const_as_imm32_typed(&Operand::Const(c.clone()), is_32) {
                self.state.emit_fmt(format_args!(
                    "    {} ${}, {}{}",
                    store_instr, imm, seg_prefix, addr
                ));
                return true;
            }
        }
        // Register-homed value: store straight from its home.
        if let Operand::Value(v) = val {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if !is_xmm_reg(reg) {
                    let rname = Self::reg_for_type(phys_reg_name(reg), ty);
                    self.state.emit_fmt(format_args!(
                        "    {} %{}, {}{}",
                        store_instr, rname, seg_prefix, addr
                    ));
                    return true;
                }
            }
        }
        self.operand_to_rax(val);
        self.state.emit_fmt(format_args!(
            "    {} %{}, {}{}",
            store_instr,
            Self::reg_for_type("rax", ty),
            seg_prefix,
            addr
        ));
        self.state.reg_cache.invalidate_acc();
        true
    }

    pub(super) fn emit_seg_store_impl(
        &mut self,
        val: &Operand,
        ptr: &Value,
        ty: IrType,
        seg: AddressSpace,
    ) {
        let seg_prefix = match seg {
            AddressSpace::SegGs => "%gs:",
            AddressSpace::SegFs => "%fs:",
            AddressSpace::Default => {
                unreachable!("segment-prefixed op called with default address space")
            }
        };
        // Operand-ordering hazard: %rdx/%rcx scratch bounces can CLOBBER the
        // other operand's register home before it is read. glibc's
        // __tls_init_tp hit exactly this: THREAD_SETMEM's fs-offset constant
        // (v = Copy(Const 1296)) was register-allocated to %rdx, the old
        // sequence moved the stored VALUE into %rdx first, and the store
        // then went through the value-as-address (*fs:&robust_head = ...)
        // — startup SIGSEGV for every external binary (LK-24). Saving the
        // value on the stack makes the sequence correct for EVERY possible
        // home assignment of val and ptr, including rax/rcx/rdx themselves.
        self.operand_to_rax(val);
        self.state.emit("    pushq %rax");
        self.operand_to_rax(&Operand::Value(*ptr));
        self.state.emit("    movq %rax, %rcx");
        self.state.emit("    popq %rax");
        let store_instr = Self::mov_store_for_type(ty);
        let store_reg = Self::reg_for_type("rax", ty);
        self.state.emit_fmt(format_args!(
            "    {} %{}, {}(%rcx)",
            store_instr, store_reg, seg_prefix
        ));
        self.state.reg_cache.invalidate_acc();
    }

    pub(super) fn emit_seg_store_symbol_impl(
        &mut self,
        val: &Operand,
        sym: &str,
        ty: IrType,
        seg: AddressSpace,
    ) {
        let seg_prefix = match seg {
            AddressSpace::SegGs => "%gs:",
            AddressSpace::SegFs => "%fs:",
            AddressSpace::Default => {
                unreachable!("segment-prefixed op called with default address space")
            }
        };
        self.operand_to_rax(val);
        let store_instr = Self::mov_store_for_type(ty);
        let store_reg = Self::reg_for_type("rax", ty);
        self.state.emit_fmt(format_args!(
            "    {} %{}, {}{}(%rip)",
            store_instr, store_reg, seg_prefix, sym
        ));
    }
}
