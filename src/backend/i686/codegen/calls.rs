//! I686Codegen: function call operations (cdecl calling convention).

use super::emit::I686Codegen;
use crate::backend::call_abi;
use crate::backend::generation::is_i128_type;
use crate::backend::narrow_copy::{NarrowCopy, eax_ferry, narrow_copy_plan, narrow_move_mnemonic};
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Operand, Value};

/// A/B gate for push-based outgoing-argument staging: the presence of
/// `CCC_NO_PUSH_ARG_STAGING` restores the `subl $N,%esp` + store marshalling
/// verbatim (same presence-means-off convention as the other `CCC_NO_*`
/// switches). Cached once: consulted per call site.
fn push_arg_staging_disabled() -> bool {
    static GATE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *GATE.get_or_init(|| std::env::var_os("CCC_NO_PUSH_ARG_STAGING").is_some())
}

impl I686Codegen {
    pub(super) fn call_abi_config_impl(&self) -> call_abi::CallAbiConfig {
        call_abi::CallAbiConfig {
            max_int_regs: self.regparm as usize,
            max_float_regs: 0,
            align_i128_pairs: false,
            f128_in_fp_regs: false,
            f128_in_gp_pairs: false,
            variadic_floats_in_gp: false,
            large_struct_by_ref: false,
            use_sysv_struct_classification: false,
            use_riscv_float_struct_classification: false,
            allow_struct_split_reg_stack: false,
            align_struct_pairs: false,
            sret_uses_dedicated_reg: false,
            gcc_regparm_mode: true,
            // GCC i386 cdecl never aligns stack args beyond the 4-byte slot
            // granularity (GCC 14.2 -m32 oracle: 3-int prefix + aligned(32)
            // struct -> arg offset 12, callee reads 4-granular via %ebp).
            // The caller packs 4-granular and va_arg walks 4-granular, so the
            // callee layout must agree — see CallAbiConfig::stack_arg_align_cap.
            stack_arg_align_cap: 4,
            vector_xmm_regs: 3,
            // The lowering decomposes Clang-convention ext vectors into
            // `[NoClass]`-tagged lanes only for this target.
            i386_ext_vector_lanes: true,
        }
    }

    /// Stack-argument alignment per the GCC i386 psABI (all offsets verified
    /// against the GCC 14.2 -m32 oracle):
    ///   * everything packs at 4-byte slot granularity — doubles (`f1(double,
    ///     int, double)`: c at offset 12), 12-byte x87 long double (`f3`),
    ///     8-byte-aligned structs (`f7(int, struct{double,double}, int)`: a at
    ///     offset 4) and even `aligned(16)` structs (`f8`: a at offset 4);
    ///   * EXCEPT TFmode 16-byte carriers (__float128/_Decimal128), which are
    ///     16-byte aligned within the outgoing area (`f2(__float128, int,
    ///     __float128)`: b at offset 16; `f6(int, __float128, int)`: q at
    ///     offset 16 — libgcc's __addtf3 likewise reads its by-value TF args
    ///     at entry_esp+20/+36, i.e. after the hidden ret pointer plus
    ///     12 bytes of alignment pad).
    fn stack_arg_align(
        &self,
        i: usize,
        ac: &call_abi::CallArgClass,
        struct_arg_aligns: &[Option<usize>],
        struct_arg_is_f128_sse: &[bool],
    ) -> usize {
        // A vector piece in an XMM register occupies no outgoing stack.
        if matches!(ac, call_abi::CallArgClass::StructXmmReg { .. }) {
            return 4;
        }
        let align = struct_arg_aligns.get(i).copied().flatten().unwrap_or(4);
        if struct_arg_is_f128_sse.get(i).copied().unwrap_or(false) && align >= 16 {
            16
        } else {
            4
        }
    }

    pub(super) fn emit_call_compute_stack_space_impl(
        &self,
        arg_classes: &[call_abi::CallArgClass],
        arg_types: &[IrType],
        struct_arg_aligns: &[Option<usize>],
        struct_arg_is_f128_sse: &[bool],
    ) -> usize {
        let mut total = 0usize;
        let mut max_arg_align = 4usize;
        for (i, ac) in arg_classes.iter().enumerate() {
            let ty = if i < arg_types.len() {
                arg_types[i]
            } else {
                IrType::I32
            };
            let align = self.stack_arg_align(i, ac, struct_arg_aligns, struct_arg_is_f128_sse);
            max_arg_align = max_arg_align.max(align);
            total = (total + align - 1) & !(align - 1);
            match ac {
                call_abi::CallArgClass::Stack => total += Self::stack_scalar_arg_size(ty),
                call_abi::CallArgClass::F128Stack => total += 12,
                call_abi::CallArgClass::I128Stack => total += 16,
                call_abi::CallArgClass::StructByValStack { size } => total += (*size + 3) & !3,
                call_abi::CallArgClass::LargeStructStack { size } => total += (*size + 3) & !3,
                call_abi::CallArgClass::ZeroSizeSkip => {}
                call_abi::CallArgClass::IntReg { .. } => {} // regparm: in register, no stack space
                call_abi::CallArgClass::I64RegPair { .. } => {} // regparm: register pair
                call_abi::CallArgClass::StructByValReg { .. } => {} // regparm: struct in registers
                call_abi::CallArgClass::StructXmmReg { .. } => {} // i386 vector piece in %xmmN
                _ => total += 4,
            }
        }
        // The outgoing area only needs the strictest alignment of the stack
        // boundary the TU asked for and the widest per-argument alignment
        // (TFmode carriers: 16). The previous unconditional 16-byte rounding
        // ignored -mpreferred-stack-boundary: with the Linux boot's
        // `-mpreferred-stack-boundary=2` GCC rounds the same call site to 4,
        // so up to 12 bytes per stack-arg call site were dead flat-image
        // weight (GCC 16.2 -Os -m16 -mregparm=3 oracle: `subl $12,%esp` for a
        // 3-arg call, not `subl $16,%esp`).
        let area_align = self.stack_boundary.max(max_arg_align as i64) as usize;
        (total + area_align - 1) & !(area_align - 1)
    }

    pub(super) fn emit_call_f128_pre_convert_impl(
        &mut self,
        _args: &[Operand],
        _arg_classes: &[call_abi::CallArgClass],
        _arg_types: &[IrType],
        _stack_arg_space: usize,
    ) -> usize {
        0 // No F128 pre-conversion needed on i686
    }

    /// True when `ac`/`ty` is a scalar argument the push-staging path can
    /// marshal with one `pushl`: the 4-byte `Stack` scalars and the same
    /// classes the staging loop's fallback arm stores as one 4-byte word.
    /// Everything with sub-4-byte granularity, wide (8/12/16-byte) footprint,
    /// struct copies, register-param classes and zero-size skips is excluded.
    /// Bytes a scalar `Stack`-class argument occupies in the i386 argument
    /// area: two words for the pair scalars (`_Decimal64` included: the
    /// emitter stores 8 bytes, so sizing it as 4 under-allocated the area
    /// and shifted every later argument), one word for everything else.
    /// The single source of truth for sizing, staging and emission.
    pub(super) fn stack_scalar_arg_size(ty: IrType) -> usize {
        if super::emit::is_pair_scalar(ty) {
            8
        } else {
            4
        }
    }

    fn push_staging_class_ok(ac: &call_abi::CallArgClass, ty: IrType) -> bool {
        if ty.size() > 4 {
            return false;
        }
        !matches!(
            ac,
            call_abi::CallArgClass::ZeroSizeSkip
                | call_abi::CallArgClass::IntReg { .. }
                | call_abi::CallArgClass::I64RegPair { .. }
                | call_abi::CallArgClass::StructByValReg { .. }
                | call_abi::CallArgClass::F128Stack
                | call_abi::CallArgClass::I128Stack
                | call_abi::CallArgClass::StructByValStack { .. }
                | call_abi::CallArgClass::LargeStructStack { .. }
                | call_abi::CallArgClass::StructSplitRegStack { .. }
                | call_abi::CallArgClass::StructXmmReg { .. }
        )
    }

    pub(super) fn emit_call_stack_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[call_abi::CallArgClass],
        arg_types: &[IrType],
        stack_arg_space: usize,
        _fptr_spill: usize,
        _f128_temp_space: usize,
        struct_arg_aligns: &[Option<usize>],
        struct_arg_is_f128_sse: &[bool],
    ) -> i64 {
        if stack_arg_space > 0 {
            // GCC-style push staging: when every stack argument is a 4-byte
            // scalar and the outgoing area is exactly the pushed bytes (no
            // alignment pad, boundary <= 4), push the arguments right-to-left
            // instead of `subl $N,%esp` + per-arg `movl %eax, N(%esp)` stores.
            // A `pushl %eax` is 2 bytes in 16-bit mode vs 5-8 for the store,
            // the `subl` disappears, and the callee's ownership of its
            // incoming argument slots makes the layout ABI-identical (GCC
            // 16.2 -Os -m16 -mregparm=3 oracle: `pushl $30; pushl $20; call
            // f`). esp_adjust tracks each push, so every later operand load
            // sees the shifted frame exactly as the subl model would. The
            // caller-side `addl $N,%esp` cleanup in emit_call_cleanup_impl
            // stays byte-identical because the pushed total equals the old
            // area size.
            let pushable: Vec<(usize, usize)> = {
                let mut v = Vec::new();
                let mut stack_offset: usize = 0;
                for (i, ac) in arg_classes.iter().enumerate() {
                    let ty = arg_types.get(i).copied().unwrap_or(IrType::I32);
                    let align =
                        self.stack_arg_align(i, ac, struct_arg_aligns, struct_arg_is_f128_sse);
                    stack_offset = (stack_offset + align - 1) & !(align - 1);
                    if Self::push_staging_class_ok(ac, ty) {
                        v.push((stack_offset, i));
                    }
                    stack_offset += match ac {
                        call_abi::CallArgClass::Stack => Self::stack_scalar_arg_size(ty),
                        call_abi::CallArgClass::F128Stack => 12,
                        call_abi::CallArgClass::I128Stack => 16,
                        call_abi::CallArgClass::StructByValStack { size }
                        | call_abi::CallArgClass::LargeStructStack { size } => (size + 3) & !3,
                        call_abi::CallArgClass::ZeroSizeSkip
                        | call_abi::CallArgClass::IntReg { .. }
                        | call_abi::CallArgClass::I64RegPair { .. }
                        | call_abi::CallArgClass::StructByValReg { .. }
                        | call_abi::CallArgClass::StructXmmReg { .. } => 0,
                        _ => 4,
                    };
                }
                v
            };
            if pushable.len() * 4 == stack_arg_space
                && self.stack_boundary <= 4
                && !push_arg_staging_disabled()
            {
                // Push right-to-left so slot 0 lands at the lowest address,
                // exactly the cdecl layout the callee expects.
                for &(_, i) in pushable.iter().rev() {
                    if let Operand::Const(c) = &args[i] {
                        // `const_stack_arg_imm` is the shared single source of
                        // truth for the 32-bit image of a constant argument;
                        // `pushl $imm` stores the same four bytes the store
                        // path would, and GAS picks the imm8 `6a` encoding
                        // whenever the value fits.
                        let imm = super::emit::const_stack_arg_imm(c);
                        emit!(self.state, "    pushl ${}", imm);
                        self.esp_adjust += 4;
                        continue;
                    }
                    self.operand_to_eax(&args[i]);
                    self.state.emit("    pushl %eax");
                    self.esp_adjust += 4;
                }
                return stack_arg_space as i64;
            }
            emit!(self.state, "    subl ${}, %esp", stack_arg_space);
            self.esp_adjust += stack_arg_space as i64;
        }

        let mut stack_offset: usize = 0;
        for (i, ac) in arg_classes.iter().enumerate() {
            // GCC i386: TFmode 16-byte carriers are 16-byte aligned in the
            // outgoing area; everything else packs at slot granularity.
            let align = self.stack_arg_align(i, ac, struct_arg_aligns, struct_arg_is_f128_sse);
            stack_offset = (stack_offset + align - 1) & !(align - 1);
            match ac {
                call_abi::CallArgClass::I128Stack => {
                    self.emit_call_i128_stack_arg(&args[i], stack_offset);
                    stack_offset += 16;
                }
                call_abi::CallArgClass::F128Stack => {
                    self.emit_call_f128_stack_arg(&args[i], stack_offset);
                    stack_offset += 12;
                }
                call_abi::CallArgClass::StructByValStack { size }
                | call_abi::CallArgClass::LargeStructStack { size } => {
                    let sz = *size;
                    self.emit_call_struct_stack_arg(&args[i], stack_offset, sz);
                    stack_offset += (sz + 3) & !3;
                }
                call_abi::CallArgClass::Stack => {
                    let ty = arg_types[i];
                    if super::emit::is_pair_scalar(ty) {
                        self.emit_call_8byte_stack_arg(&args[i], stack_offset);
                        stack_offset += Self::stack_scalar_arg_size(ty);
                    } else {
                        // A constant argument is stored directly; only operands
                        // that genuinely need `%eax` take the two-instruction
                        // path (see store_const_to_stack_arg).
                        if !self.store_const_to_stack_arg(&args[i], stack_offset) {
                            self.operand_to_eax(&args[i]);
                            emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
                        }
                        stack_offset += 4;
                    }
                }
                call_abi::CallArgClass::ZeroSizeSkip => {}
                call_abi::CallArgClass::IntReg { .. } => {} // regparm: handled in emit_call_reg_args
                call_abi::CallArgClass::I64RegPair { .. } => {} // regparm: register pair
                call_abi::CallArgClass::StructByValReg { .. } => {} // regparm: struct in registers
                call_abi::CallArgClass::StructXmmReg { .. } => {} // loaded in emit_call_reg_args
                _ => {
                    if !self.store_const_to_stack_arg(&args[i], stack_offset) {
                        self.operand_to_eax(&args[i]);
                        emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
                    }
                    stack_offset += 4;
                }
            }
        }

        stack_arg_space as i64
    }

    pub(super) fn emit_call_reg_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[call_abi::CallArgClass],
        _arg_types: &[IrType],
        _total_sp_adjust: i64,
        _f128_temp_space: usize,
        _stack_arg_space: usize,
        _struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>],
    ) {
        // i386 vector pieces: the operand is the address of the piece's 16
        // bytes. They load first, through %eax, because the regparm moves
        // below also stage through %eax and must be the last writers of the
        // GP argument registers; nothing after this point touches an XMM
        // register before the call.
        for (i, ac) in arg_classes.iter().enumerate() {
            if let call_abi::CallArgClass::StructXmmReg { reg_idx, .. } = ac {
                self.operand_to_eax(&args[i]);
                emit!(self.state, "    movups (%eax), %xmm{}", reg_idx);
            }
        }
        if self.current_call_regparm == 0 {
            return; // cdecl: no register args
        }
        // regparm register order: EAX (reg_idx 0), EDX (reg_idx 1), ECX (reg_idx 2).
        //
        // Args are emitted in DESCENDING base-register order so that %eax —
        // the staging register for operand_to_eax and the pointer register
        // for register-struct copies — is written by the very last move.
        // Register targets are disjoint across args, so descending order
        // guarantees no arg's staging clobbers an already-loaded target.
        let regparm_regs: &[&str] = &["%eax", "%edx", "%ecx"];
        let mut items: Vec<(usize, usize)> = Vec::new(); // (base_reg_idx, arg_idx)
        for (i, ac) in arg_classes.iter().enumerate() {
            match ac {
                call_abi::CallArgClass::IntReg { reg_idx } if *reg_idx < 3 => {
                    items.push((*reg_idx, i))
                }
                call_abi::CallArgClass::I64RegPair { base_reg_idx }
                    if *base_reg_idx + 1 < 3 + 1 =>
                {
                    items.push((*base_reg_idx, i))
                }
                call_abi::CallArgClass::StructByValReg { base_reg_idx, .. } => {
                    items.push((*base_reg_idx, i))
                }
                _ => {}
            }
        }
        items.sort_by(|a, b| b.0.cmp(&a.0));
        for &(_base, arg_i) in &items {
            match &arg_classes[arg_i] {
                call_abi::CallArgClass::IntReg { reg_idx } => {
                    let dest_reg = regparm_regs[*reg_idx];
                    self.operand_to_eax(&args[arg_i]);
                    if dest_reg != "%eax" {
                        emit!(self.state, "    movl %eax, {}", dest_reg);
                        self.state.reg_cache.invalidate_acc();
                    }
                }
                call_abi::CallArgClass::I64RegPair { base_reg_idx } => {
                    let lo = regparm_regs[*base_reg_idx];
                    let hi = regparm_regs[*base_reg_idx + 1];
                    match &args[arg_i] {
                        Operand::Const(c) => {
                            let v = c.to_i64().unwrap_or(0);
                            emit!(self.state, "    movl ${}, {}", (v & 0xFFFF_FFFF) as i32, lo);
                            emit!(
                                self.state,
                                "    movl ${}, {}",
                                ((v as u64) >> 32) as i32,
                                hi
                            );
                        }
                        Operand::Value(v) => {
                            if let Some(slot) = self.state.get_slot(v.0) {
                                // Direct mem->reg loads: no staging, order-safe.
                                let sr0 = self.slot_ref(slot);
                                let sr4 = self.slot_ref_offset(slot, 4);
                                emit!(self.state, "    movl {}, {}", sr4, hi);
                                emit!(self.state, "    movl {}, {}", sr0, lo);
                            } else {
                                // Wide values are always slotted on i686, but stay
                                // defensive: acc-pair produces eax(lo):edx(hi).
                                self.emit_load_acc_pair(&args[arg_i]);
                                if *base_reg_idx == 1 {
                                    self.state.emit("    movl %edx, %ecx");
                                    self.state.emit("    movl %eax, %edx");
                                }
                            }
                        }
                    }
                    self.state.reg_cache.invalidate_acc();
                }
                call_abi::CallArgClass::StructByValReg { base_reg_idx, size } => {
                    let words = size.div_ceil(4);
                    if let Operand::Value(v) = &args[arg_i] {
                        if self.state.is_alloca(v.0) {
                            if let Some(slot) = self.state.get_slot(v.0) {
                                // Direct slot-word -> reg loads (full-word reads:
                                // slots are 4-byte rounded, GCC reads word_mode
                                // chunks here too).
                                for k in (0..words).rev() {
                                    let sr = self.slot_ref_offset(slot, (k * 4) as i64);
                                    emit!(
                                        self.state,
                                        "    movl {}, {}",
                                        sr,
                                        regparm_regs[base_reg_idx + k]
                                    );
                                }
                            }
                        } else {
                            // Pointer to struct data: pointer staged in %eax;
                            // descending word order means an %eax target (only
                            // possible at word 0 when base==0) is written last.
                            self.operand_to_eax(&args[arg_i]);
                            for k in (0..words).rev() {
                                let off = (k * 4) as i64;
                                let defined = (*size as i64 - off).min(4);
                                debug_assert!(
                                    defined >= 1,
                                    "a rounded word always holds at least one defined byte"
                                );
                                if defined >= 4 {
                                    emit!(
                                        self.state,
                                        "    movl {}(%eax), {}",
                                        off,
                                        regparm_regs[base_reg_idx + k]
                                    );
                                    continue;
                                }
                                // regparm rounds an aggregate up to whole words, and
                                // the last word can hold as few as one defined byte.
                                // Loading it with `movl` reads up to 3 bytes past the
                                // C object: measured on -m32, sizes 1, 7, 9 and 15
                                // faulted as the caller while 8 and 16 did not -- the
                                // exact profile of word rounding, with the fixture's
                                // aggregate placed flush against a PROT_NONE page.
                                // Padding above the object is unspecified in the
                                // register, so the fix is to stop reading, not to zero
                                // what was never defined.  Staging goes through a
                                // transient word of our own stack because %eax is the
                                // only free GP here: every other one may already hold a
                                // staged argument.  The pointer is parked and restored
                                // so the descending order above (an %eax destination
                                // must be written last) still holds.
                                emit!(self.state, "    subl $8, %esp");
                                emit!(self.state, "    movl %eax, 4(%esp)");
                                for NarrowCopy { offset, width } in
                                    narrow_copy_plan(defined as usize, 2)
                                {
                                    let mn = narrow_move_mnemonic(width);
                                    let ferry = eax_ferry(width);
                                    emit!(self.state, "    movl 4(%esp), %eax");
                                    self.state.emit_fmt(format_args!(
                                        "    {mn} {}(%eax), {ferry}",
                                        off + offset as i64
                                    ));
                                    self.state.emit_fmt(format_args!(
                                        "    {mn} {ferry}, {}(%esp)",
                                        offset as i64
                                    ));
                                }
                                // `%eax` is both the ferry and, at word 0, a legal
                                // destination: `regparm_regs[0]` is `%eax`, and word 0
                                // is the last iteration of the descending loop.  The
                                // parked pointer must therefore be restored only when
                                // the destination is some other register -- writing
                                // `%eax` and then re-loading it from the frame puts the
                                // source *address* where the argument belongs, which
                                // is a silent miscompile of every 1-3 byte aggregate
                                // passed in the first regparm register (the value the
                                // callee sees is whatever the frame held, so it varies
                                // with -O level and with ASLR: measured as 200, 88 and
                                // 120 for one configuration across runs).  Skipping the
                                // restore is safe rather than merely lucky: the loop is
                                // over, `invalidate_acc` below tells the cache %eax is
                                // clobbered, and the next aggregate argument re-parks
                                // its own pointer via `operand_to_eax`.
                                let dest = regparm_regs[base_reg_idx + k];
                                if dest == "%eax" {
                                    emit!(self.state, "    movl (%esp), %eax");
                                } else {
                                    emit!(self.state, "    movl (%esp), {}", dest);
                                    // Still mid-loop: the next word's ferry loads read
                                    // the parked pointer back.
                                    emit!(self.state, "    movl 4(%esp), %eax");
                                }
                                emit!(self.state, "    addl $8, %esp");
                            }
                        }
                    }
                    self.state.reg_cache.invalidate_acc();
                }
                _ => {}
            }
        }
    }

    pub(super) fn emit_call_instruction_impl(
        &mut self,
        direct_name: Option<&str>,
        func_ptr: Option<&Operand>,
        indirect: bool,
        _stack_arg_space: usize,
    ) {
        // x87 registers are caller-saved: drop any cached st(0) copy first.
        self.flush_x87_pending_copy();
        if let Some(name) = direct_name {
            if self.state.needs_plt(name) {
                emit!(self.state, "    call {}@PLT", name);
            } else {
                emit!(self.state, "    call {}", name);
            }
        } else if indirect {
            if let Some(fptr) = func_ptr {
                if self.current_call_regparm > 0 {
                    // Under regparm, %eax/%edx/%ecx carry arguments — staging
                    // the target in %eax would destroy arg 0. Call through
                    // the value's home directly (register or stack slot);
                    // i686 values always have one or the other.
                    if let Operand::Value(v) = fptr {
                        if let Some(&phys) = self.reg_assignments.get(&v.0) {
                            emit!(
                                self.state,
                                "    call *%{}",
                                super::emit::phys_reg_name(phys)
                            );
                            return;
                        }
                        if let Some(slot) = self.state.get_slot(v.0) {
                            let sr = self.slot_ref(slot);
                            emit!(self.state, "    call *{}", sr);
                            return;
                        }
                    }
                    // Constant target (rare): safe only because no register
                    // argument exists if we got here without a home — still,
                    // prefer an absolute call over clobbering %eax.
                    if let Operand::Const(c) = fptr {
                        if let Some(v) = c.to_i64() {
                            emit!(self.state, "    call *${}", v);
                            return;
                        }
                    }
                }
                self.operand_to_eax(fptr);
            }
            self.state.emit("    call *%eax");
        }
    }

    pub(super) fn emit_call_cleanup_impl(
        &mut self,
        stack_arg_space: usize,
        _f128_temp_space: usize,
        _indirect: bool,
    ) {
        if stack_arg_space > 0 {
            emit!(self.state, "    addl ${}, %esp", stack_arg_space);
            self.esp_adjust -= stack_arg_space as i64;
        }
    }

    pub(super) fn emit_call_store_result_impl(&mut self, dest: &Value, return_type: IrType) {
        if return_type == IrType::Void {
            return;
        }
        if return_type == IrType::I64 || return_type == IrType::U64 || return_type == IrType::D64 {
            if let Some(slot) = self.state.get_slot(dest.0) {
                let sr0 = self.slot_ref(slot);
                let sr4 = self.slot_ref_offset(slot, 4);
                emit!(self.state, "    movl %eax, {}", sr0);
                emit!(self.state, "    movl %edx, {}", sr4);
            }
            self.state.reg_cache.invalidate_acc();
        } else if is_i128_type(return_type) {
            self.emit_call_store_i128_result(dest);
        } else if return_type.is_long_double() {
            self.emit_call_store_f128_result(dest);
        } else if return_type == IrType::F32 {
            self.emit_call_move_f32_to_acc();
            self.emit_store_result(dest);
        } else if return_type == IrType::F64 {
            self.emit_f64_store_from_x87(dest);
            self.state.reg_cache.invalidate_acc();
        } else {
            self.emit_store_result(dest);
        }
    }

    pub(super) fn emit_call_store_i128_result_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr0 = self.slot_ref(slot);
            let sr4 = self.slot_ref_offset(slot, 4);
            emit!(self.state, "    movl %eax, {}", sr0);
            emit!(self.state, "    movl %edx, {}", sr4);
        }
    }

    /// Store ECX and a deterministic padding word for a three-word i686
    /// integer ExtVector result. The first two words are stored by the regular
    /// I128 call-result path above.
    pub(super) fn emit_call_store_i686_ext_vector_gp3_result_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr8 = self.slot_ref_offset(slot, 8);
            let sr12 = self.slot_ref_offset(slot, 12);
            emit!(self.state, "    movl %ecx, {}", sr8);
            emit!(self.state, "    movl $0, {}", sr12);
        }
    }

    /// Store a 16-byte i386 vector piece returned in %xmm0 over the I128
    /// result slot (the regular I128 store above wrote EAX:EDX there).
    pub(super) fn emit_call_store_i686_xmm_result_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movups %xmm0, {}", sr);
        }
    }

    pub(super) fn emit_call_store_f128_result_impl(&mut self, dest: &Value) {
        // The callee returns the F128 value on st(0); the caller must pop
        // it in every case, or the entry leaks into subsequent code (the
        // x87 stack wraps after eight stray pushes). With a slot the
        // store-and-pop `fstpt` does both; without one the result is dead
        // and a bare pop balances the stack.
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    fstpt {}", sr);
            self.state.f128_direct_slots.insert(dest.0);
        } else {
            self.state.emit("    fstp %st(0)");
        }
    }

    pub(super) fn emit_call_move_f32_to_acc_impl(&mut self) {
        self.state.emit("    subl $4, %esp");
        self.state.emit("    fstps (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    addl $4, %esp");
    }

    pub(super) fn emit_call_move_f64_to_acc_impl(&mut self) {
        self.state.emit("    subl $8, %esp");
        self.state.emit("    fstpl (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    addl $8, %esp");
    }
}
