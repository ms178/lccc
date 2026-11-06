//! ArmCodegen: AAPCS64 homogeneous aggregates (HFA / HVA / short vectors).
//!
//! An aggregate classified `FpAggClass::Hfa { member_size, count }` by the
//! lowering crosses calls one member per SIMD register: argument members in
//! consecutive v-registers (`CallArgClass::StructHfaReg`), results in
//! v0..v(count-1) through the hidden return buffer
//! (`call_abi::hidden_fp_agg_return`), and variadic members in 16-byte slots
//! of the `va_list` register save area.  Every transfer is a straight
//! load/store between the aggregate's memory image and the registers, so the
//! members never round-trip through general registers.

use super::emit::ArmCodegen;
use crate::backend::call_abi::{CallArgClass, ParamClass};
use crate::backend::generation::ParamAllocaIndex;
use crate::common::types::FpAggClass;
use crate::ir::reexports::{IrFunction, Operand, Value};

/// The name of SIMD register `idx` viewed at `member_size` bytes: `h`, `s`,
/// `d` or `q`.
pub(super) fn hfa_vreg(member_size: usize, idx: usize) -> String {
    let prefix = match member_size {
        2 => 'h',
        4 => 's',
        8 => 'd',
        _ => 'q',
    };
    format!("{prefix}{idx}")
}

impl ArmCodegen {
    /// Move `count` members of `member_size` bytes between v[`first`..] and
    /// the aggregate at `[base]` (member `i` at byte `i * member_size`):
    /// `ldp`/`stp` for each register pair (the offsets, at most 48, are
    /// always in the scaled 7-bit range), a single `ldr`/`str` for an odd
    /// tail.  Half-precision members have no pair form.
    pub(super) fn emit_hfa_transfer(
        &mut self,
        load: bool,
        base: &str,
        first: usize,
        count: usize,
        member_size: usize,
    ) {
        let (single, pair) = if load { ("ldr", "ldp") } else { ("str", "stp") };
        let mut i = 0;
        while i < count {
            let off = i * member_size;
            let reg = hfa_vreg(member_size, first + i);
            if member_size != 2 && i + 1 < count {
                let reg2 = hfa_vreg(member_size, first + i + 1);
                self.state
                    .emit_fmt(format_args!("    {pair} {reg}, {reg2}, [{base}, #{off}]"));
                i += 2;
            } else {
                self.state
                    .emit_fmt(format_args!("    {single} {reg}, [{base}, #{off}]"));
                i += 1;
            }
        }
    }

    /// Call phase 3d: load every HFA argument into its v-registers.  Runs
    /// after all other register phases: none of them writes a v-register an
    /// HFA owns, and the f128 conversions of the FP phase (libcalls that
    /// clobber v0-v7) are behind it.  x17 is the address scratch, as for
    /// the by-value GP aggregates.
    pub(super) fn emit_call_hfa_reg_args(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        slot_adjust: i64,
        needs_adjusted_load: bool,
    ) {
        for (arg, class) in args.iter().zip(arg_classes) {
            if let CallArgClass::StructHfaReg {
                fp_idx,
                count,
                member_size,
            } = *class
            {
                self.emit_load_arg_to_reg(arg, "x17", slot_adjust, 0, needs_adjusted_load);
                self.emit_hfa_transfer(true, "x17", fp_idx, count, member_size);
            }
        }
    }

    /// Prologue phase 1b: store HFA parameters from their v-registers into
    /// their homes, and home the hidden return pointer of an FP-register
    /// result as the address of the private return buffer.  Runs before any
    /// prologue code that may clobber v0-v7.
    pub(super) fn emit_store_hfa_params(
        &mut self,
        func: &IrFunction,
        param_homes: &ParamAllocaIndex,
        param_classes: &[ParamClass],
    ) {
        for (i, class) in param_classes.iter().enumerate().take(func.params.len()) {
            let (fp_idx, count, member_size) = match *class {
                ParamClass::StructHfaReg {
                    fp_idx,
                    count,
                    member_size,
                } => (fp_idx, count, member_size),
                ParamClass::RegReturnBuffer { .. } => {
                    let Some(offset) = self.fp_ret_buffer.map(|(offset, _)| offset) else {
                        continue;
                    };
                    let Some((slot, _, dest_val)) = self.resolve_param_slot(param_homes, i) else {
                        continue;
                    };
                    let over_aligned = self.home_over_aligned(dest_val.0);
                    if over_aligned {
                        self.emit_alloca_addr("x11", dest_val.0, slot.0);
                    }
                    self.emit_add_sp_offset("x9", offset);
                    self.emit_param_home_store_impl(over_aligned, "str", "x9", slot, 0);
                    continue;
                }
                _ => continue,
            };
            if param_homes.get(i).is_none() {
                continue;
            }
            let Some((slot, _, dest_val)) = self.resolve_param_slot(param_homes, i) else {
                continue;
            };
            let over_aligned = self.home_over_aligned(dest_val.0);
            if over_aligned {
                self.emit_alloca_addr("x11", dest_val.0, slot.0);
            }
            for m in 0..count {
                let reg = hfa_vreg(member_size, fp_idx + m);
                self.emit_param_home_store_impl(
                    over_aligned,
                    "str",
                    &reg,
                    slot,
                    (m * member_size) as i64,
                );
            }
        }
    }

    /// Whether the parameter home `alloca_id` is over-aligned past the
    /// 16-byte frame alignment (its address is computed into x11).
    fn home_over_aligned(&self, alloca_id: u32) -> bool {
        self.state
            .alloca_over_align(alloca_id)
            .filter(|&a| a > 16)
            .is_some()
    }

    /// Return of an FP-register aggregate: `val` is the hidden buffer
    /// pointer the body wrote the result through; load the members into
    /// v0..v(count-1).
    pub(super) fn emit_return_fp_agg(&mut self, val: &Operand, class: FpAggClass) {
        if let Some((member_size, count)) = class.hfa() {
            self.operand_to_x0(val);
            self.emit_hfa_transfer(true, "x0", 0, count, member_size);
        }
    }

    /// After a call returning an FP-register aggregate: store v0..v(count-1)
    /// through the caller's buffer (`buffer`, argument 0 of the sret shape)
    /// and give `dest` -- the sret shape's pointer result -- its address.
    pub(super) fn emit_call_store_fp_agg_return_impl(
        &mut self,
        buffer: &Operand,
        class: FpAggClass,
        dest: Option<&Value>,
    ) {
        let Some((member_size, count)) = class.hfa() else {
            debug_assert!(
                false,
                "AArch64 FP-register result must be an HFA: {class:?}"
            );
            return;
        };
        self.state.reg_cache.invalidate_all();
        self.emit_load_arg_to_reg(buffer, "x17", 0, 0, false);
        self.emit_hfa_transfer(false, "x17", 0, count, member_size);
        if let Some(dest) = dest {
            self.state.emit("    mov x0, x17");
            self.store_x0_to(dest);
        }
    }

    /// `va_arg` of a homogeneous aggregate: `count` members of
    /// `member_size` bytes, `size` bytes in memory, stack alignment
    /// `align`.  Passed in registers, member `i` sits at the bottom of the
    /// `i`-th 16-byte slot of the register save area (`__vr_top +
    /// __vr_offs`); the members are gathered into the contiguous copy at
    /// `dest_ptr`.  When fewer than `count` slots remain the aggregate is
    /// on the stack laid out as in memory (C.3/C.6), and `__vr_offs` stays
    /// advanced past 0, so later FP arguments are read from the stack too
    /// -- GCC's `aarch64_gimplify_va_arg_expr`, which updates the offset
    /// before testing it.
    pub(super) fn emit_va_arg_hfa_impl(
        &mut self,
        dest_ptr: &Value,
        va_list_ptr: &Value,
        size: usize,
        align: usize,
        member_size: usize,
        count: usize,
    ) {
        let label_id = self.state.next_label_id();
        let label_stack = format!(".Lva_hfa_stack_{label_id}");
        let label_done = format!(".Lva_hfa_done_{label_id}");
        self.load_va_list_ptr(va_list_ptr, "x1");
        self.load_dest_ptr(dest_ptr, "x4");

        // __vr_offs (a negative i32 at [ap, #28] while register slots
        // remain): non-negative means every slot is consumed.
        self.state.emit("    ldrsw x2, [x1, #28]");
        self.state
            .emit_fmt(format_args!("    tbz x2, #63, {label_stack}"));
        self.state
            .emit_fmt(format_args!("    adds x3, x2, #{}", count * 16));
        self.state.emit("    str w3, [x1, #28]");
        self.state.emit_fmt(format_args!("    b.gt {label_stack}"));

        // Register path: gather the members out of their 16-byte slots.
        self.state.emit("    ldr x5, [x1, #16]");
        self.state.emit("    add x5, x5, x2");
        for i in 0..count {
            let src = i * 16;
            let dst = i * member_size;
            match member_size {
                2 => {
                    self.state
                        .emit_fmt(format_args!("    ldrh w6, [x5, #{src}]"));
                    self.state
                        .emit_fmt(format_args!("    strh w6, [x4, #{dst}]"));
                }
                4 => {
                    self.state
                        .emit_fmt(format_args!("    ldr w6, [x5, #{src}]"));
                    self.state
                        .emit_fmt(format_args!("    str w6, [x4, #{dst}]"));
                }
                8 => {
                    self.state
                        .emit_fmt(format_args!("    ldr x6, [x5, #{src}]"));
                    self.state
                        .emit_fmt(format_args!("    str x6, [x4, #{dst}]"));
                }
                _ => {
                    self.state
                        .emit_fmt(format_args!("    ldp x6, x7, [x5, #{src}]"));
                    self.state
                        .emit_fmt(format_args!("    stp x6, x7, [x4, #{dst}]"));
                }
            }
        }
        self.state.emit_fmt(format_args!("    b {label_done}"));

        // Stack path: the memory image at the (16-aligned for a quad or
        // 128-bit-vector base) cursor, which then advances by the size
        // rounded up to 8.
        self.state.emit_fmt(format_args!("{label_stack}:"));
        self.state.emit("    ldr x5, [x1]");
        if align > 8 {
            self.state.emit("    add x5, x5, #15");
            self.state.emit("    and x5, x5, #-16");
        }
        let full = size / 8;
        for q in 0..full {
            let off = q * 8;
            self.state
                .emit_fmt(format_args!("    ldr x6, [x5, #{off}]"));
            self.state
                .emit_fmt(format_args!("    str x6, [x4, #{off}]"));
        }
        if size % 8 != 0 {
            self.emit_partial_struct_copy((full * 8) as i64, size % 8, "x5", "x4");
        }
        self.state
            .emit_fmt(format_args!("    add x5, x5, #{}", size.div_ceil(8) * 8));
        self.state.emit("    str x5, [x1]");
        self.state.emit_fmt(format_args!("{label_done}:"));
        self.state.reg_cache.invalidate_all();
    }
}
