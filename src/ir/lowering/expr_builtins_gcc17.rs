//! GCC 17-git C-torture builtin intrinsics with GCC 16.2 semantics.
//!
//! These five builtins cannot be `LibcAlias`es:
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
//!   complex representation, so no target's complex *return* ABI is involved.
//! * `__builtin_cpow(x, y)` forwards to libm `cpow`, but libm uses the
//!   TARGET's complex ABI (x86-64: real/imag in xmm0/xmm1 and a two-SSE
//!   return; i686: 16-byte aggregates on the stack and a hidden sret
//!   pointer). LCCC complex values are pointer-backed, so the operands must
//!   be decomposed into scalar F64s and the two return conventions handled
//!   separately. A plain LibcAlias would emit `IrType::Ptr` as the return
//!   carrier and read the complex result out of `%rax` -- a miscompile.
//!
//! All five lowerings are ABI-correct on both x86_64 and i686; none of them
//! introduces a new external symbol beyond glibc/libm.

use super::lower::Lowerer;
use crate::backend::Target;
use crate::common::types::{AddressSpace, CType, IrType, target_is_32bit};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{CallInfo, Instruction, IrCmpOp, IrConst, Operand, Value};

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
        if arg_types[2] != size_t && arg_types[2] != IrType::Ptr && arg_types[2].is_integer() {
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
        let sin_val = self.load_f64(sin_slot);
        let cos_val = self.load_f64(cos_slot);
        Some(Operand::Value(self.build_complex_f64(cos_val, sin_val)))
    }

    /// `_Complex double __builtin_cpow(_Complex double x, _Complex double y)`
    ///
    /// Decompose both operands into `(real, imag)` F64 scalars and call libm
    /// `cpow`. On x86-64 SysV a `_Complex double` occupies two SSE argument
    /// registers and is returned in xmm0/xmm1; on i686 SysV the two 16-byte
    /// aggregates live on the stack and the result is written through a
    /// hidden sret pointer that is the FIRST stack argument. Passing four
    /// adjacent F64 values reproduces that exact stack/register layout on
    /// both targets without depending on LCCC's aggregate-by-value ABI.
    pub(super) fn lower_builtin_cpow(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.len() != 2 {
            for arg in args {
                self.lower_expr(arg);
            }
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let (xr, xi) = self.complex_parts_f64(&args[0]);
        let (yr, yi) = self.complex_parts_f64(&args[1]);
        let scalars = [xr, xi, yr, yi];

        if self.target == Target::I686 {
            // Hidden sret buffer IS the pointer-backed complex result: the
            // callee writes real at +0 and imag at +8 into it.
            let out = self.alloca(16, 8);
            let mut call_args = vec![Operand::Value(out)];
            let mut call_types = vec![IrType::Ptr];
            for v in scalars {
                call_args.push(Operand::Value(v));
                call_types.push(IrType::F64);
            }
            self.emit(Instruction::Call {
                func: "cpow".to_string(),
                info: CallInfo {
                    dest: None,
                    args: call_args,
                    arg_types: call_types,
                    return_type: IrType::Void,
                    num_fixed_args: 5,
                    is_sret: true,
                    assume_library: true,
                    ..CallInfo::default()
                },
            });
            return Some(Operand::Value(out));
        }

        let real = self.fresh_value();
        self.emit(Instruction::Call {
            func: "cpow".to_string(),
            info: CallInfo {
                dest: Some(real),
                args: scalars.iter().map(|&v| Operand::Value(v)).collect(),
                arg_types: vec![IrType::F64; 4],
                return_type: IrType::F64,
                num_fixed_args: 4,
                assume_library: true,
                ..CallInfo::default()
            },
        });
        let imag = self.fresh_value();
        self.emit(Instruction::GetReturnF64Second { dest: imag });
        Some(Operand::Value(self.build_complex_f64(real, imag)))
    }

    /// Extract `(real, imag)` as F64 scalars from an expression, converting
    /// `_Complex float` / `_Complex long double` components and promoting a
    /// real scalar to `(x, 0)` exactly as C's usual arithmetic conversions do.
    fn complex_parts_f64(&mut self, arg: &Expr) -> (Value, Value) {
        let ctype = self.expr_ctype(arg);
        let (ptr, comp_ty, comp_size) = match ctype {
            CType::ComplexDouble => {
                let lowered = self.lower_expr(arg);
                (self.operand_to_value(lowered), IrType::F64, 8)
            }
            CType::ComplexFloat => {
                let lowered = self.lower_expr(arg);
                (self.operand_to_value(lowered), IrType::F32, 4)
            }
            CType::ComplexLongDouble => {
                let lowered = self.lower_expr(arg);
                (self.operand_to_value(lowered), IrType::F128, 16)
            }
            _ => {
                let lowered = self.lower_expr_with_type(arg, IrType::F64);
                let real = self.operand_to_value(lowered);
                let imag = self.operand_to_value(Operand::Const(IrConst::F64(0.0)));
                return (real, imag);
            }
        };
        let real_raw = self.fresh_value();
        self.emit(Instruction::Load {
            dest: real_raw,
            ptr,
            ty: comp_ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        let imag_ptr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: imag_ptr,
            base: ptr,
            offset: Operand::Const(IrConst::I64(comp_size)),
            ty: IrType::I8,
        });
        let imag_raw = self.fresh_value();
        self.emit(Instruction::Load {
            dest: imag_raw,
            ptr: imag_ptr,
            ty: comp_ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        let real = self.to_f64(Operand::Value(real_raw), comp_ty);
        let imag = self.to_f64(Operand::Value(imag_raw), comp_ty);
        (real, imag)
    }

    fn to_f64(&mut self, src: Operand, from: IrType) -> Value {
        if from == IrType::F64 {
            return self.operand_to_value(src);
        }
        self.emit_cast_val(src, from, IrType::F64)
    }

    fn load_f64(&mut self, ptr: Value) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::Load {
            dest,
            ptr,
            ty: IrType::F64,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        dest
    }

    fn alloca(&mut self, size: usize, align: usize) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest,
            ty: IrType::Ptr,
            size,
            align,
            volatile: false,
            semantic_volatile: false,
        });
        dest
    }

    /// Materialise a `_Complex double` in LCCC's pointer-backed form:
    /// 16 bytes, real at +0 and imag at +8.
    fn build_complex_f64(&mut self, real: Value, imag: Value) -> Value {
        let complex = self.alloca(16, 8);
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(real),
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
            val: Operand::Value(imag),
            ptr: imag_ptr,
            ty: IrType::F64,
            seg_override: AddressSpace::Default,
        });
        complex
    }
}
