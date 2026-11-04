//! RiscvCodegen: LP64D floating-point aggregate results.
//!
//! A struct the hardware floating-point convention flattens
//! (`FpAggClass::OneFloat` / `TwoFloats` / `FloatAndInt` / `IntAndFloat`) is
//! returned the way it would be passed as the first named argument: its
//! float members in fa0/fa1, its integer member in a0 (psABI "Hardware
//! Floating-point Calling Convention").  The IR keeps the sret shape
//! (`call_abi::hidden_fp_agg_return`): the callee homes the hidden pointer
//! as a private frame buffer and loads the registers from it on return; the
//! caller stores the registers through its own buffer after the call.

use super::emit::RiscvCodegen;
use crate::common::types::FpAggClass;
use crate::ir::reexports::{Operand, Value};

/// One register-sized member of a flattened aggregate: its register, byte
/// offset and size.
#[derive(Clone, Copy)]
pub(super) enum FpAggPiece {
    Fp {
        reg: &'static str,
        offset: usize,
        size: u8,
    },
    Int {
        reg: &'static str,
        offset: usize,
        size: usize,
    },
}

/// The result registers of `class`, in register order.  An AAPCS64 class
/// has none here (only the AArch64 lowering produces one).
pub(super) fn fp_agg_return_pieces(class: FpAggClass) -> [Option<FpAggPiece>; 2] {
    match class {
        FpAggClass::OneFloat { size } => [
            Some(FpAggPiece::Fp {
                reg: "fa0",
                offset: 0,
                size,
            }),
            None,
        ],
        FpAggClass::TwoFloats {
            lo_size,
            hi_size,
            hi_offset,
        } => [
            Some(FpAggPiece::Fp {
                reg: "fa0",
                offset: 0,
                size: lo_size,
            }),
            Some(FpAggPiece::Fp {
                reg: "fa1",
                offset: hi_offset,
                size: hi_size,
            }),
        ],
        FpAggClass::FloatAndInt {
            float_size,
            float_offset,
            int_offset,
            int_size,
        } => [
            Some(FpAggPiece::Fp {
                reg: "fa0",
                offset: float_offset,
                size: float_size,
            }),
            Some(FpAggPiece::Int {
                reg: "a0",
                offset: int_offset,
                size: int_size,
            }),
        ],
        FpAggClass::IntAndFloat {
            float_size,
            int_offset,
            int_size,
            float_offset,
        } => [
            Some(FpAggPiece::Int {
                reg: "a0",
                offset: int_offset,
                size: int_size,
            }),
            Some(FpAggPiece::Fp {
                reg: "fa0",
                offset: float_offset,
                size: float_size,
            }),
        ],
        FpAggClass::Hfa { .. } => [None, None],
    }
}

/// The exact-width load of an integer member of `size` bytes.  Narrow
/// members zero-extend like GCC's RISC-V `LOAD_EXTEND_OP` (a 32-bit member
/// sign-extends, the RV64 canonical form); a wider access would read past
/// the member -- misaligned, and past the object when the member ends it.
pub(super) fn int_member_load(size: usize) -> &'static str {
    match size {
        1 => "lbu",
        2 => "lhu",
        4 => "lw",
        _ => "ld",
    }
}

/// The exact-width store of an integer member of `size` bytes.
pub(super) fn int_member_store(size: usize) -> &'static str {
    match size {
        1 => "sb",
        2 => "sh",
        4 => "sw",
        _ => "sd",
    }
}

impl RiscvCodegen {
    /// Load the float member `size` bytes wide at `offset(base)` into
    /// `freg`.  A half (no Zfh in the baseline) is NaN-boxed through t1/t2.
    pub(super) fn emit_load_fp_member(&mut self, freg: &str, base: &str, offset: usize, size: u8) {
        match size {
            8 => self
                .state
                .emit_fmt(format_args!("    fld {freg}, {offset}({base})")),
            2 => {
                self.state
                    .emit_fmt(format_args!("    lhu t1, {offset}({base})"));
                self.state.emit("    lui t2, 0xffff0");
                self.state.emit("    or t1, t1, t2");
                self.state.emit_fmt(format_args!("    fmv.w.x {freg}, t1"));
            }
            _ => self
                .state
                .emit_fmt(format_args!("    flw {freg}, {offset}({base})")),
        }
    }

    /// Store float member `freg` (`size` bytes) to `offset(base)`.
    fn emit_store_fp_member(&mut self, freg: &str, base: &str, offset: usize, size: u8) {
        match size {
            8 => self
                .state
                .emit_fmt(format_args!("    fsd {freg}, {offset}({base})")),
            2 => {
                self.state.emit_fmt(format_args!("    fmv.x.w t1, {freg}"));
                self.state
                    .emit_fmt(format_args!("    sh t1, {offset}({base})"));
            }
            _ => self
                .state
                .emit_fmt(format_args!("    fsw {freg}, {offset}({base})")),
        }
    }

    /// Return of a flattened float aggregate: `val` is the hidden buffer
    /// pointer the body wrote the result through.
    pub(super) fn emit_return_fp_agg(&mut self, val: &Operand, class: FpAggClass) {
        self.operand_to_t0(val);
        for piece in fp_agg_return_pieces(class).into_iter().flatten() {
            match piece {
                FpAggPiece::Fp { reg, offset, size } => {
                    self.emit_load_fp_member(reg, "t0", offset, size)
                }
                FpAggPiece::Int { reg, offset, size } => self.state.emit_fmt(format_args!(
                    "    {} {reg}, {offset}(t0)",
                    int_member_load(size)
                )),
            }
        }
    }

    /// After a call returning a flattened float aggregate: store its result
    /// registers through the caller's buffer (`buffer`, argument 0 of the
    /// sret shape) and give `dest` the buffer's address.  `operand_to_t0`
    /// touches neither fa0/fa1 nor a0: the buffer pointer is live across
    /// the call, so it is homed in an s-register or a frame slot.
    pub(super) fn emit_call_store_fp_agg_return_impl(
        &mut self,
        buffer: &Operand,
        class: FpAggClass,
        dest: Option<&Value>,
    ) {
        // t0 was clobbered by the call whatever the cache last recorded.
        self.state.reg_cache.invalidate_all();
        self.operand_to_t0(buffer);
        for piece in fp_agg_return_pieces(class).into_iter().flatten() {
            match piece {
                FpAggPiece::Fp { reg, offset, size } => {
                    self.emit_store_fp_member(reg, "t0", offset, size)
                }
                FpAggPiece::Int { reg, offset, size } => self.state.emit_fmt(format_args!(
                    "    {} {reg}, {offset}(t0)",
                    int_member_store(size)
                )),
            }
        }
        if let Some(dest) = dest {
            self.store_t0_to(dest);
        }
    }
}
