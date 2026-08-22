//! X86Codegen: variadic argument handling (va_arg, va_start, va_copy).
//!
//! System V AMD64 ABI (psABI §3.5.7):
//!   typedef struct {
//!       unsigned int gp_offset;       // +0
//!       unsigned int fp_offset;       // +4
//!       void *overflow_arg_area;      // +8
//!       void *reg_save_area;          // +16
//!   } va_list[1];
//!
//! Register-save area layout (offsets from reg_save_area):
//!   [0..48)   : %rdi,%rsi,%rdx,%rcx,%r8,%r9   (6 × 8)
//!   [48..176) : %xmm0..%xmm7                  (8 × 16)
//!
//! Shapes below are validated against GCC 15.3 / Clang 21.1 / ICX on the
//! Compiler Explorer oracle (see /oracle/ORACLE_FINDINGS.md):
//!   - hot path = register fetch with `ja` → memory and fall-through;
//!   - the compared offset is REUSED as the index (`movl %eax,%edx` zero-
//!     extends, no `movslq`), and `addq 16(%rcx),%rdx` folds reg_save_area
//!     into the address (one load-op, no separate base load);
//!   - scalar FP stays in the SSE domain (`store_xmm_to`), never bouncing
//!     through %rax;
//!   - long double keeps full x87 80-bit precision (fldt + fstpt), and
//!     __int128 uses 2 GP slots with a 16-aligned overflow fallback.
//!
//! Correctness is a hard constraint: fast wrong codegen is worthless.

use super::emit::{phys_reg_name, X86Codegen};
use crate::common::types::{EightbyteClass, IrType};
use crate::ir::reexports::Value;

/// gp_offset exclusive upper bound (6 integer arg regs × 8 bytes).
const GP_MAX: i64 = 48;
/// fp_offset exclusive upper bound (8 SSE arg regs × 16 bytes, starting at 48).
const FP_MAX: i64 = 176;
/// Size of one GP save slot.
const GP_SLOT: i64 = 8;
/// Size of one SSE save slot.
const FP_SLOT: i64 = 16;

impl X86Codegen {
    // =========================================================================
    // va_arg — scalar
    // =========================================================================

    pub(super) fn emit_va_arg_impl(
        &mut self,
        dest: &Value,
        va_list_ptr: &Value,
        result_ty: IrType,
    ) {
        // long double (x87 80-bit in a 16-byte slot): always MEMORY.
        if result_ty.is_long_double() {
            self.emit_va_arg_long_double(dest, va_list_ptr);
            return;
        }
        // __int128 / unsigned __int128: 2 × INTEGER, 16-aligned on overflow.
        if result_ty.is_128bit() {
            self.emit_va_arg_i128(dest, va_list_ptr);
            return;
        }
        // float / double (SSE class).
        if result_ty.is_float() {
            self.emit_va_arg_fp(dest, va_list_ptr, result_ty);
            return;
        }
        // integer / pointer (INTEGER class, one GP register).
        self.emit_va_arg_gp(dest, va_list_ptr, result_ty);
    }

    /// INTEGER class, single eightbyte (int, long, pointer, …).
    ///
    /// The fetch width follows `mov_load_for_type` exactly (movslq for I32 —
    /// matching LCCC's sign-extended-slot convention — movq for 64-bit types),
    /// so the upper bits of a 32-bit argument register are never trusted.
    fn emit_va_arg_gp(&mut self, dest: &Value, va_list_ptr: &Value, result_ty: IrType) {
        let label_mem = self.state.fresh_label("va_arg_mem");
        let label_end = self.state.fresh_label("va_arg_end");

        self.load_va_list_ptr_to_rcx(va_list_ptr);

        // if (gp_offset >= 48) goto mem;   fall-through = registers.
        self.state.emit("    movl (%rcx), %eax");
        self.state
            .out
            .emit_instr_imm_reg("    cmpl", GP_MAX - GP_SLOT, "eax");
        self.state.out.emit_jcc_label("    ja", &label_mem);

        // ---- register path ----
        self.state.emit("    movl %eax, %edx"); // index (zero-extends)
        self.state.emit("    addl $8, %eax"); // new offset
        self.state.emit("    addq 16(%rcx), %rdx"); // %rdx = reg_save_area + offset
        self.state.emit("    movl %eax, (%rcx)");
        let load_instr = Self::mov_load_for_type(result_ty);
        let load_dest = Self::load_dest_reg(result_ty);
        self.state
            .emit_fmt(format_args!("    {} (%rdx), {}", load_instr, load_dest));
        self.state.out.emit_jmp_label(&label_end);

        // ---- overflow path ----
        self.state.out.emit_named_label(&label_mem);
        self.state.emit("    movq 8(%rcx), %rdx");
        self.state.emit("    leaq 8(%rdx), %rax");
        self.state.emit("    movq %rax, 8(%rcx)");
        self.state
            .emit_fmt(format_args!("    {} (%rdx), {}", load_instr, load_dest));

        self.state.out.emit_named_label(&label_end);
        self.store_rax_to(dest);
        self.state.reg_cache.invalidate_all();
    }

    /// SSE class scalar (float / double).
    ///
    /// Keeps the value in %xmm0 and stores via `store_xmm_to` — no GPR
    /// round-trip, no domain crossing (the deficit LCCC's FP path historically
    /// shared with naive compilers; GCC/ICX both stay in the SSE domain here).
    fn emit_va_arg_fp(&mut self, dest: &Value, va_list_ptr: &Value, result_ty: IrType) {
        let is_f32 = result_ty == IrType::F32;

        self.load_va_list_ptr_to_rcx(va_list_ptr);

        // -mno-sse: va_start forced fp_offset = 176, so every FP vararg comes
        // from overflow — and NO SSE instruction may be emitted. Load the raw
        // bit pattern in the GPR domain (the backend's FP-without-SSE model).
        if self.no_sse {
            self.state.emit("    movq 8(%rcx), %rdx");
            if is_f32 {
                self.state.emit("    movl (%rdx), %eax");
            } else {
                self.state.emit("    movq (%rdx), %rax");
            }
            self.state.emit("    addq $8, %rdx");
            self.state.emit("    movq %rdx, 8(%rcx)");
            self.store_rax_to(dest);
            self.state.reg_cache.invalidate_all();
            return;
        }

        let label_mem = self.state.fresh_label("va_arg_fmem");
        let label_end = self.state.fresh_label("va_arg_fend");

        // if (fp_offset >= 176) goto mem;   fall-through = registers.
        self.state.emit("    movl 4(%rcx), %eax");
        self.state
            .out
            .emit_instr_imm_reg("    cmpl", FP_MAX - FP_SLOT, "eax");
        self.state.out.emit_jcc_label("    ja", &label_mem);

        // ---- register path ----
        self.state.emit("    movl %eax, %edx"); // index
        self.state.emit("    addl $16, %eax"); // new offset
        self.state.emit("    addq 16(%rcx), %rdx"); // %rdx = reg_save_area + offset
        self.state.emit("    movl %eax, 4(%rcx)");
        if is_f32 {
            // float occupies the low 32 bits of its 16-byte XMM save slot.
            self.state.emit("    movss (%rdx), %xmm0");
        } else {
            self.state.emit("    movsd (%rdx), %xmm0");
        }
        self.state.out.emit_jmp_label(&label_end);

        // ---- overflow path ----
        self.state.out.emit_named_label(&label_mem);
        self.state.emit("    movq 8(%rcx), %rdx");
        if is_f32 {
            self.state.emit("    movss (%rdx), %xmm0");
        } else {
            self.state.emit("    movsd (%rdx), %xmm0");
        }
        self.state.emit("    addq $8, %rdx");
        self.state.emit("    movq %rdx, 8(%rcx)");

        self.state.out.emit_named_label(&label_end);
        // Stay in the SSE domain end-to-end (matches GCC/ICX va_arg(FP)).
        self.store_xmm_to(dest, "xmm0", result_ty);
        self.state.reg_cache.invalidate_all();
    }

    /// long double: always from overflow, 16-byte aligned, full 80-bit x87.
    ///
    /// The old code did `fldt → fstpl → %rax`, silently truncating the 80-bit
    /// value to an f64 bit pattern — wrong ABI value and a precision loss for
    /// any `va_arg(ap, long double)`. `emit_f128_load_finish` stores the full
    /// 80-bit value to dest (fstpt) and a truncated f64 shadow in %rax for the
    /// legacy GPR-representation paths, exactly like the F128 load path.
    fn emit_va_arg_long_double(&mut self, dest: &Value, va_list_ptr: &Value) {
        self.load_va_list_ptr_to_rcx(va_list_ptr);

        // Align overflow_arg_area up to 16 (psABI: align > 8 → 16-aligned).
        self.state.emit("    movq 8(%rcx), %rdx");
        self.state.emit("    addq $15, %rdx");
        self.state.emit("    andq $-16, %rdx");

        // Full 80-bit extended precision into ST(0).
        self.state.emit("    fldt (%rdx)");

        // Advance past the 16-byte slot and write back.
        self.state.emit("    addq $16, %rdx");
        self.state.emit("    movq %rdx, 8(%rcx)");

        self.emit_f128_load_finish(dest);
        self.state.reg_cache.invalidate_all();
    }

    /// __int128 / unsigned __int128: 2 GP registers, or 16-byte-aligned overflow.
    fn emit_va_arg_i128(&mut self, dest: &Value, va_list_ptr: &Value) {
        let label_mem = self.state.fresh_label("va_i128_mem");
        let label_end = self.state.fresh_label("va_i128_end");

        self.load_va_list_ptr_to_rcx(va_list_ptr);

        // Need 2 GP slots: fits iff gp_offset <= 48 - 16 = 32.
        self.state.emit("    movl (%rcx), %edx");
        self.state
            .out
            .emit_instr_imm_reg("    cmpl", GP_MAX - 2 * GP_SLOT, "edx");
        self.state.out.emit_jcc_label("    ja", &label_mem);

        // ---- register path (GCC/ICX shape) ----
        self.state.emit("    movl %edx, %eax"); // index
        self.state.emit("    addl $16, %edx"); // new offset
        self.state.emit("    addq 16(%rcx), %rax"); // %rax = reg_save_area + offset
        self.state.emit("    movl %edx, (%rcx)");
        // i128 accumulator convention: %rax = low, %rdx = high.
        self.state.emit("    movq 8(%rax), %rdx"); // high
        self.state.emit("    movq (%rax), %rax"); // low
        self.state.out.emit_jmp_label(&label_end);

        // ---- overflow: 16-byte align, fetch 16 bytes, advance 16 ----
        self.state.out.emit_named_label(&label_mem);
        self.state.emit("    movq 8(%rcx), %rsi");
        self.state.emit("    addq $15, %rsi");
        self.state.emit("    andq $-16, %rsi");
        self.state.emit("    movq (%rsi), %rax"); // low
        self.state.emit("    movq 8(%rsi), %rdx"); // high
        self.state.emit("    addq $16, %rsi");
        self.state.emit("    movq %rsi, 8(%rcx)");

        self.state.out.emit_named_label(&label_end);
        self.store_rax_rdx_to(dest);
        self.state.reg_cache.invalidate_all();
    }

    // =========================================================================
    // va_arg — aggregates
    // =========================================================================

    /// MEMORY-class (or unknown) struct: always from overflow_arg_area.
    ///
    /// `align` is the struct's alignment (threaded from the lowering). Types
    /// whose alignment exceeds 8 bytes get their overflow slot 16-aligned
    /// (psABI); the old code ignored alignment and always advanced by 8, which
    /// both misaligned the fetch and desynchronised every later argument.
    pub(super) fn emit_va_arg_struct_impl(
        &mut self,
        dest_ptr: &Value,
        va_list_ptr: &Value,
        size: usize,
        align: usize,
    ) {
        self.emit_va_arg_struct_overflow(dest_ptr, va_list_ptr, size, align);
    }

    /// va_arg for aggregates with SysV eightbyte classification.
    pub(super) fn emit_va_arg_struct_ex_impl(
        &mut self,
        dest_ptr: &Value,
        va_list_ptr: &Value,
        size: usize,
        align: usize,
        eightbyte_classes: &[EightbyteClass],
    ) {
        // Empty classification = MEMORY class (size > 16, unaligned, X87, …).
        if eightbyte_classes.is_empty() {
            self.emit_va_arg_struct_overflow(dest_ptr, va_list_ptr, size, align);
            return;
        }

        let gp_count = eightbyte_classes
            .iter()
            .filter(|c| matches!(c, EightbyteClass::Integer | EightbyteClass::NoClass))
            .count() as i64;
        let fp_count = eightbyte_classes
            .iter()
            .filter(|c| matches!(c, EightbyteClass::Sse))
            .count() as i64;

        // Register demand that can never be satisfied (defensive: the classifier
        // cannot emit more than 2 GP / 2 SSE eightbytes for a <=16-byte struct).
        if gp_count * GP_SLOT > GP_MAX || fp_count * FP_SLOT > FP_MAX - 48 {
            self.emit_va_arg_struct_overflow(dest_ptr, va_list_ptr, size, align);
            return;
        }

        let label_mem = self.state.fresh_label("va_struct_mem");
        let label_end = self.state.fresh_label("va_struct_end");

        self.load_va_list_ptr_to_rcx(va_list_ptr);
        self.load_ptr_to_reg(dest_ptr, "rdi");

        // ---- availability checks: BOTH classes must fit, else whole struct ----
        // is fetched from overflow (the ABI never splits a struct across the
        // register/overflow boundary).
        if gp_count > 0 {
            let thr = GP_MAX - gp_count * GP_SLOT;
            self.state.emit("    movl (%rcx), %eax");
            self.state.out.emit_instr_imm_reg("    cmpl", thr, "eax");
            self.state.out.emit_jcc_label("    ja", &label_mem);
        }
        if fp_count > 0 {
            let thr = FP_MAX - fp_count * FP_SLOT;
            self.state.emit("    movl 4(%rcx), %edx");
            self.state.out.emit_instr_imm_reg("    cmpl", thr, "edx");
            self.state.out.emit_jcc_label("    ja", &label_mem);
        }

        // ==== Register path (fall-through). %eax = gp_offset, %edx = fp_offset. ====
        self.emit_va_arg_struct_register_path(eightbyte_classes, gp_count, fp_count, &label_end);

        // ==== Memory (overflow) path. %rcx = &va_list, %rdi = dest still live. ====
        self.state.out.emit_named_label(&label_mem);
        self.emit_va_arg_struct_overflow_body(size, align);

        self.state.out.emit_named_label(&label_end);
        self.state.reg_cache.invalidate_all();
    }

    /// Register-save-area fetch for a classified struct, given the compared
    /// offsets already in %eax (gp) / %edx (fp) and dest in %rdi.
    ///
    /// GP eightbytes are fetched LAST (into %rax), because that clobbers the
    /// gp offset; the two-GP case copies the offset into %edx first (fp_count
    /// == 0 leaves %edx free). Offsets are updated with direct memory RMWs,
    /// which is the smallest encoding (matches ICX's shape, avoids GCC's
    /// SSE-domain packed-offset trick).
    fn emit_va_arg_struct_register_path(
        &mut self,
        classes: &[EightbyteClass],
        gp_count: i64,
        fp_count: i64,
        label_end: &str,
    ) {
        self.state.emit("    movq 16(%rcx), %rsi"); // reg_save_area

        match (gp_count, fp_count) {
            // Pure SSE: one or two eightbytes; %edx indexes both.
            (0, 1) | (0, 2) => {
                self.state.emit("    movsd (%rsi,%rdx), %xmm0");
                self.state.emit("    movsd %xmm0, (%rdi)");
                if fp_count == 2 {
                    self.state.emit("    movsd 16(%rsi,%rdx), %xmm0");
                    self.state.emit("    movsd %xmm0, 8(%rdi)");
                    self.state.emit("    addl $32, 4(%rcx)");
                } else {
                    self.state.emit("    addl $16, 4(%rcx)");
                }
            }
            // Pure GP, one eightbyte.
            (1, 0) => {
                self.state.emit("    movq (%rsi,%rax), %rax");
                self.state.emit("    movq %rax, (%rdi)");
                self.state.emit("    addl $8, (%rcx)");
            }
            // Pure GP, two eightbytes: copy the offset into %edx (free) so the
            // first fetch into %rax cannot clobber the index.
            (2, 0) => {
                self.state.emit("    movl %eax, %edx");
                self.state.emit("    movq (%rsi,%rdx), %rax");
                self.state.emit("    movq %rax, (%rdi)");
                self.state.emit("    movq 8(%rsi,%rdx), %rax");
                self.state.emit("    movq %rax, 8(%rdi)");
                self.state.emit("    addl $16, (%rcx)");
            }
            // Mixed [Integer,Sse] or [Sse,Integer]: FP first (no GPR clobber),
            // GP last (clobbers %eax after its address is consumed).
            _ => {
                let fp_pos = classes
                    .iter()
                    .position(|c| matches!(c, EightbyteClass::Sse))
                    .unwrap_or(0);
                let gp_pos = classes
                    .iter()
                    .position(|c| matches!(c, EightbyteClass::Integer | EightbyteClass::NoClass))
                    .unwrap_or(1);
                let fp_off = (fp_pos * 8) as i64;
                let gp_off = (gp_pos * 8) as i64;
                self.state.emit("    movsd (%rsi,%rdx), %xmm0");
                self.state
                    .emit_fmt(format_args!("    movsd %xmm0, {}(%rdi)", fp_off));
                self.state.emit("    movq (%rsi,%rax), %rax");
                self.state
                    .emit_fmt(format_args!("    movq %rax, {}(%rdi)", gp_off));
                self.state.emit("    addl $8, (%rcx)");
                self.state.emit("    addl $16, 4(%rcx)");
            }
        }

        self.state.out.emit_jmp_label(label_end);
    }

    /// Copy `size` bytes from overflow_arg_area to dest, aligning the source to
    /// `align` first (only when align > 8), and advancing overflow by
    /// round_up(size, 8).
    fn emit_va_arg_struct_overflow(
        &mut self,
        dest_ptr: &Value,
        va_list_ptr: &Value,
        size: usize,
        align: usize,
    ) {
        self.load_va_list_ptr_to_rcx(va_list_ptr);
        self.load_ptr_to_reg(dest_ptr, "rdi");
        self.emit_va_arg_struct_overflow_body(size, align);
        self.state.reg_cache.invalidate_all();
    }

    /// Core overflow copy. Precondition: %rcx = &va_list, %rdi = dest.
    /// Uses %rsi as the (aligned) source cursor; %xmm0/%rax as copy scratch.
    fn emit_va_arg_struct_overflow_body(&mut self, size: usize, align: usize) {
        self.state.emit("    movq 8(%rcx), %rsi"); // overflow_arg_area
        if align > 8 {
            debug_assert!(
                align.is_power_of_two(),
                "struct align must be a power of two"
            );
            self.state
                .emit_fmt(format_args!("    addq ${}, %rsi", align - 1));
            self.state
                .emit_fmt(format_args!("    andq ${}, %rsi", -(align as i64)));
        }

        // Widest-available moves, matching GCC/ICX/Clang (16-byte movdqu pairs,
        // then 8-byte movq, then a byte-tail). %rsi is not advanced by the copy
        // itself — all offsets are compile-time — so the writeback below can
        // compute overflow + advance directly from the aligned base.
        let mut off: i64 = 0;
        let mut remaining = size;
        if !self.no_sse {
            while remaining >= 16 {
                self.state
                    .emit_fmt(format_args!("    movdqu {}(%rsi), %xmm0", off));
                self.state
                    .emit_fmt(format_args!("    movdqu %xmm0, {}(%rdi)", off));
                off += 16;
                remaining -= 16;
            }
        }
        while remaining >= 8 {
            self.state
                .out
                .emit_instr_mem_reg("    movq", off, "rsi", "rax");
            self.state
                .out
                .emit_instr_reg_mem("    movq", "rax", off, "rdi");
            off += 8;
            remaining -= 8;
        }
        if remaining > 0 {
            self.emit_partial_copy(off, remaining);
        }

        // Advance overflow_arg_area past the struct (round_up(size, 8)).
        let advance = (size.div_ceil(8) * 8) as i64;
        self.state
            .emit_fmt(format_args!("    addq ${}, %rsi", advance));
        self.state.emit("    movq %rsi, 8(%rcx)");
    }

    // =========================================================================
    // va_start
    // =========================================================================

    pub(super) fn emit_va_start_impl(&mut self, va_list_ptr: &Value) {
        // &va_list → %rax
        if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rax");
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            if self.state.is_alloca(va_list_ptr.0) {
                self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rax");
            } else {
                self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
            }
        }

        // gp_offset = #named GP eightbytes * 8, fp_offset = 48 + #named FP * 16.
        // Two 32-bit stores (GCC's shape) — smaller than Clang's movabsq+movq
        // packed store and equal uop count on modern cores.
        let gp_offset = (self.num_named_int_params.min(6) * 8) as u32;
        self.state
            .out
            .emit_instr_imm_mem("    movl", gp_offset as i64, 0, "rax");
        let fp_offset = if self.no_sse {
            FP_MAX as u32
        } else {
            (48 + self.num_named_fp_params.min(8) * 16) as u32
        };
        self.state
            .out
            .emit_instr_imm_mem("    movl", fp_offset as i64, 4, "rax");

        // overflow_arg_area = first caller stack arg + named_stack_bytes.
        // In rbp mode the first stack arg is at 16(%rbp) (ret addr + saved rbp);
        // in FPO mode it is at 8 past the virtual rbp (ret addr only). The
        // rsp-aware emitter adds the frame size in FPO mode.
        let stack_base: i64 = if self.state.omit_frame_pointer { 8 } else { 16 };
        let overflow_offset = stack_base + self.num_named_stack_bytes as i64;
        self.state
            .out
            .emit_instr_rbp_reg("    leaq", overflow_offset as i64, "rcx");
        self.state.emit("    movq %rcx, 8(%rax)");

        // reg_save_area (negative offset from rbp).
        let reg_save = self.reg_save_area_offset;
        self.state
            .out
            .emit_instr_rbp_reg("    leaq", reg_save, "rcx");
        self.state.emit("    movq %rcx, 16(%rax)");

        self.state.reg_cache.invalidate_all();
    }

    // =========================================================================
    // va_copy
    // =========================================================================

    pub(super) fn emit_va_copy_impl(&mut self, dest_ptr: &Value, src_ptr: &Value) {
        self.load_ptr_to_reg(src_ptr, "rsi");
        self.load_ptr_to_reg(dest_ptr, "rdi");

        // va_list is exactly 24 bytes: one 16-byte move + one 8-byte move
        // (4 instructions, 2 loads + 2 stores — the GCC/ICX shape) instead of
        // 6 movq (6 loads/stores).
        if !self.no_sse {
            self.state.emit("    movdqu (%rsi), %xmm0");
            self.state.emit("    movq 16(%rsi), %rax");
            self.state.emit("    movdqu %xmm0, (%rdi)");
            self.state.emit("    movq %rax, 16(%rdi)");
        } else {
            self.state.emit("    movq (%rsi), %rax");
            self.state.emit("    movq %rax, (%rdi)");
            self.state.emit("    movq 8(%rsi), %rax");
            self.state.emit("    movq %rax, 8(%rdi)");
            self.state.emit("    movq 16(%rsi), %rax");
            self.state.emit("    movq %rax, 16(%rdi)");
        }
        self.state.reg_cache.invalidate_all();
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    /// Load a pointer-typed Value into `reg` (a 64-bit register name).
    ///
    /// Delegates to `value_to_reg`, which handles register-assigned pointers,
    /// spilled pointer slots, AND over-aligned allocas (the address is
    /// `(slot + align - 1) & -align`, not the raw slot — required so an
    /// `_Alignas(32)` struct buffer is written exactly where its consumers
    /// read it).
    fn load_ptr_to_reg(&mut self, ptr: &Value, reg: &str) {
        self.value_to_reg(ptr, reg);
    }

    /// Copy the last `remaining` (1..7) bytes of a struct from `offset(%rsi)`
    /// to `offset(%rdi)` using the widest natural store at each step.
    fn emit_partial_copy(&mut self, offset: i64, remaining: usize) {
        let mut off = offset;
        let mut left = remaining;
        if left >= 4 {
            self.state
                .out
                .emit_instr_mem_reg("    movl", off, "rsi", "eax");
            self.state
                .out
                .emit_instr_reg_mem("    movl", "eax", off, "rdi");
            off += 4;
            left -= 4;
        }
        if left >= 2 {
            self.state
                .out
                .emit_instr_mem_reg("    movzwl", off, "rsi", "eax");
            self.state
                .out
                .emit_instr_reg_mem("    movw", "ax", off, "rdi");
            off += 2;
            left -= 2;
        }
        if left >= 1 {
            self.state
                .out
                .emit_instr_mem_reg("    movzbl", off, "rsi", "eax");
            self.state
                .out
                .emit_instr_reg_mem("    movb", "al", off, "rdi");
        }
    }
}
