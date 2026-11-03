//! GCC 17-git C-torture builtin intrinsics with GCC 16.2 semantics.
//!
//! These four builtins cannot be `LibcAlias`es:
//!
//! * `__builtin_memcmp_eq` returns **0/1**, not `memcmp`'s signed byte
//!   difference (verified: gcc-16.2 `-O2` and `-O0` both print `1 0 -1` for
//!   unequal/equal/`memcmp` on volatile arrays). Aliasing it to `memcmp`
//!   would silently change every equality test's false value.
//! * `__builtin_stack_save` / `__builtin_stack_restore` are stack-pointer
//!   intrinsics, not functions; the IR already has `StackSave`/`StackRestore`
//!   for VLA scope cleanup, so reuse exactly that machinery.
//! * `__builtin_cexpi(x)` = `cos(x) + i*sin(x)` has no libc symbol. Calling
//!   `sin` and `cos` separately costs two libcalls; glibc `sincos` computes
//!   both in ONE call. The complex result is built in LCCC's pointer-backed
//!   complex representation, so no target's complex *return* ABI (x86-64
//!   xmm0/xmm1 vs i686 hidden sret) is involved.
//!
//! All four lowerings are ABI-correct on both x86_64 and i686; none of them
//! introduces a new external symbol beyond glibc/libm.

use super::lower::Lowerer;
use crate::common::types::{AddressSpace, IrType, target_is_32bit};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{
    CallInfo, Instruction, IrCmpOp, IrConst, Operand,
};

impl Lowerer {
    /// `int __builtin_memcmp_eq(const void *p, const void *q, size_t n)`
    ///
    /// GCC's contract is a BOOLEAN: 0 when the first `n` bytes are equal,
    /// exactly 1 otherwise. We emit one `memcmp` call (the library's
    /// vectorised implementation is faster than any inline byte loop we could
    /// emit) and normalise with `cmp != 0`, which is a single `setne` on
    /// x86-64/i686. The `size_t` conversion is part of the builtin's
    /// prototype and MUST be applied even when `memcmp` was never declared:
    /// `pr108892.c` passes the int literal `2` in a `size_t` slot.
    pub(super) fn lower_builtin_memcmp_eq(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.len() != 3 {
            // Sema reports the arity error; keep lowering deterministic.
            for arg in args {
                self.lower_expr(arg);
            }
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let mut arg_vals: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
        let mut arg_types: Vec<IrType> = args.iter().map(|a| self.value_ir_type(a)).collect();
        let size_t = if target_is_32bit() {
            IrType::U32
        } else {
            IrType::U64
        };
        if arg_types[2] != size_t
            && arg_types[2] != IrType::Ptr
            && arg_types[2].is_integer()
        {
            arg_vals[2] = self.emit_implicit_cast(arg_vals[2].clone(), arg_types[2], size_t);
            arg_types[2] = size_t;
        }
        let raw = self.fresh_value();
        self.emit(Instruction::Call {
            func: "memcmp".to_string(),
            info: CallInfo {
                dest: Some(raw),
                args: arg_vals,
                arg_types,
                return_type: IrType::I32,
                num_fixed_args: 3,
                assume_library: true,
                ..CallInfo::default()
            },
        });
        // emit_cmp_val produces an I32 0/1 flag (IR comparisons themselves
        // are I8; the helper widens), matching `__builtin_memcmp_eq`'s int.
        let flag = self.emit_cmp_val(
            IrCmpOp::Ne,
            Operand::Value(raw),
            Operand::Const(IrConst::I32(0)),
            IrType::I32,
        );
        Some(Operand::Value(flag))
    }

    /// `void *__builtin_stack_save(void)` -> opaque stack watermark.
    pub(super) fn lower_builtin_stack_save(&mut self, args: &[Expr]) -> Option<Operand> {
        // GCC documents zero arguments. Evaluate any stray args for their
        // side effects rather than dropping them (an argument containing a
        // function call must still be called).
        for arg in args {
            self.lower_expr(arg);
        }
        let dest = self.fresh_value();
        self.emit(Instruction::StackSave { dest });
        Some(Operand::Value(dest))
    }

    /// `void __builtin_stack_restore(void *p)` -> void.
    pub(super) fn lower_builtin_stack_restore(&mut self, args: &[Expr]) -> Option<Operand> {
        let ptr = args
            .first()
            .map(|a| self.lower_expr(a))
            .unwrap_or(Operand::Const(IrConst::I64(0)));
        let value = self.operand_to_value(ptr);
        self.emit(Instruction::StackRestore { ptr: value });
        for arg in args.iter().skip(1) {
            self.lower_expr(arg);
        }
        // A void expression still needs an Operand for the expression
        // machinery; callers of void builtins discard it.
        Some(Operand::Const(IrConst::I64(0)))
    }

    /// `_Complex double __builtin_cexpi(double x)`
    ///
    /// `cexp(i*x) = cos(x) + i*sin(x)`. `sincos(x, &sin, &cos)` is glibc's
    /// single-call primitive for exactly this pair. The two results are
    /// stored into a 16-byte stack object in LCCC's complex representation
    /// (real at +0, imag at +8), which is what every complex consumer
    /// (`__real`, `__imag`, complex arithmetic, complex returns) expects.
    pub(super) fn lower_builtin_cexpi(&mut self, args: &[Expr]) -> Option<Operand> {
        let x = args
            .first()
            .map(|a| self.lower_expr_with_type(a, IrType::F64))
            .unwrap_or(Operand::Const(IrConst::F64(0.0)));
        let x_val = self.operand_to_value(x);

        let sin_slot = self.fresh_value();
        let cos_slot = self.fresh_value();
        for slot in [sin_slot, cos_slot] {
            self.emit(Instruction::Alloca {
                dest: slot,
                ty: IrType::Ptr,
                size: 8,
                align: 8,
                volatile: false,
                semantic_volatile: false,
            });
        }
        self.emit(Instruction::Call {
            func: "sincos".to_string(),
            info: CallInfo {
                dest: None,
                args: vec![
                    Operand::Value(x_val),
                    Operand::Value(sin_slot),
                    Operand::Value(cos_slot),
                ],
                arg_types: vec![IrType::F64, IrType::Ptr, IrType::Ptr],
                return_type: IrType::Void,
                num_fixed_args: 3,
                assume_library: true,
                ..CallInfo::default()
            },
        });
        let sin_val = self.fresh_value();
        self.emit(Instruction::Load {
            dest: sin_val,
            ptr: sin_slot,
            ty: IrType::F64,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        let cos_val = self.fresh_value();
        self.emit(Instruction::Load {
            dest: cos_val,
            ptr: cos_slot,
            ty: IrType::F64,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        let complex = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: complex,
            ty: IrType::Ptr,
            size: 16,
            align: 8,
            volatile: false,
            semantic_volatile: false,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(cos_val),
            ptr: complex,
            ty: IrType::F64,
            seg_override: AddressSpace::Default,
        });
        let imag_ptr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: imag_ptr,
            base: complex,
            offset: Operand::Const(IrConst::I64(8)),
            ty: IrType::I8,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(sin_val),
            ptr: imag_ptr,
            ty: IrType::F64,
            seg_override: AddressSpace::Default,
        });
        Some(Operand::Value(complex))
    }
}
