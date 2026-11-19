//! AVX/AVX2 memory intrinsics that touch memory without ordinary cache
//! semantics: the non-temporal stores (`vmovnt*`), the streaming load
//! (`vmovntdqa`), and the sign-bit-masked loads and stores (`vmaskmov*` /
//! `vpmaskmov*`).
//!
//! Register discipline (shared with `intrinsics_simd.rs`):
//! - Each emitter first commits any deferred vector store and invalidates
//!   the vector peephole, so no cached register can alias the memory the
//!   instruction touches.
//! - The data operands are staged in the reserved scratch pair (`%xmm0`/
//!   `%xmm1` or `%ymm0`/`%ymm1`). The 256-bit operand loads go through
//!   `avx_load_arg_to`, which may read 32 bytes; the 128-bit operands go
//!   through `vex128_source`, which reads exactly 16 bytes.
//! - Only the pointer is dereferenced, and only by the instruction that
//!   the mask governs (`vmaskmov`), so masked-off lanes never fault.
//! - The non-temporal and masked stores write user memory, so they
//!   invalidate the vector peephole afterwards: a later load from that
//!   address must not be forwarded from a register cache.

use super::emit::X86Codegen;
use crate::ir::reexports::{IntrinsicOp, Operand, Value};

impl X86Codegen {
    /// `vmovntdq`/`vmovntpd`/`vmovntps` of a 256-bit value to `(ptr)`.
    /// The VEX.256 streaming store faults unless the destination is 32-byte
    /// aligned; the front end's `_mm256_stream_*` contract puts that
    /// requirement on the caller, exactly as GCC and Clang do.
    pub(super) fn emit_avx_movnt_256(
        &mut self,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        let Some(ptr) = dest_ptr else { return };
        let inst = match op {
            IntrinsicOp::Movntdq256 => "vmovntdq",
            IntrinsicOp::Movntpd256 => "vmovntpd",
            IntrinsicOp::Movntps256 => "vmovntps",
            _ => return,
        };
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.avx_load_arg_to(&args[0], "ymm0");
        self.value_to_reg(ptr, "rax");
        self.state
            .emit_fmt(format_args!("    {} %ymm0, (%rax)", inst));
        self.state.invalidate_vec_peephole();
    }

    /// `movntdqa` (SSE4.1, 128-bit) and `vmovntdqa` (AVX2, 256-bit): a
    /// non-temporal load from `(ptr)` into the result. The source must be
    /// 16- or 32-byte aligned respectively.
    pub(super) fn emit_movntdqa(
        &mut self,
        op: &IntrinsicOp,
        dest: &Option<Value>,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        let Some(dst) = dest_ptr.as_ref().or(dest.as_ref()) else {
            return;
        };
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.operand_to_reg(&args[0], "rax");
        match op {
            IntrinsicOp::Movntdqa128 => {
                self.state.emit("    movntdqa (%rax), %xmm0");
                self.sse_store_dest(dst, "xmm0");
            }
            IntrinsicOp::Movntdqa256 => {
                self.state.emit("    vmovntdqa (%rax), %ymm0");
                self.avx_store_dest(dst);
            }
            _ => {}
        }
    }

    /// `vmaskmov{ps,pd}` / `vpmaskmov{d,q}`, load and store forms.
    ///
    /// Load: `args = [ptr, mask]`, result = lanes whose mask sign bit is set,
    /// the rest zero. Store: `dest_ptr = ptr`, `args = [mask, value]`, only the
    /// selected lanes are written. In AT&T operand order the load is
    /// `vmaskmov (mem), %mask, %dst` and the store is `vmaskmov %val, %mask, (mem)`.
    pub(super) fn emit_avx_maskmov(
        &mut self,
        op: &IntrinsicOp,
        dest: &Option<Value>,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        let Some((mnemonic, width, is_store)) = op.vmaskmov_form() else {
            return;
        };
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        if is_store {
            let Some(ptr) = dest_ptr else { return };
            // The store reads the mask and the value; both are register
            // operands here, so the VEX encoding needs no memory staging.
            let (mask, value) = if width == 32 {
                self.avx_load_arg_to(&args[0], "ymm1");
                self.avx_load_arg_to(&args[1], "ymm0");
                ("%ymm1".to_string(), "%ymm0".to_string())
            } else {
                let mask = self.vex128_source(&args[0], "xmm1");
                let value = self.vex128_source(&args[1], "xmm0");
                (mask, value)
            };
            self.value_to_reg(ptr, "rax");
            self.state
                .emit_fmt(format_args!("    {} {}, {}, (%rax)", mnemonic, value, mask));
            self.state.invalidate_vec_peephole();
            return;
        }
        let Some(dst) = dest_ptr.as_ref().or(dest.as_ref()) else {
            return;
        };
        if width == 32 {
            self.avx_load_arg_to(&args[1], "ymm1");
            self.operand_to_reg(&args[0], "rax");
            self.state.emit_fmt(format_args!(
                "    {} (%rax), %ymm1, %ymm0",
                mnemonic
            ));
            self.avx_store_dest(dst);
        } else {
            let mask = self.vex128_source(&args[1], "xmm1");
            self.operand_to_reg(&args[0], "rax");
            self.state.emit_fmt(format_args!(
                "    {} (%rax), {}, %xmm0",
                mnemonic, mask
            ));
            self.vex128_commit_dest(dst, None);
        }
    }
}
