//! Builtin function call lowering: __builtin_* dispatch and X86 SSE intrinsics.
//!
//! This module handles the top-level dispatch for all __builtin_* functions,
//! complex number intrinsics, and X86 SSE/CRC intrinsics. Extracted helpers
//! live in sibling modules:
//! - expr_builtins_intrin.rs: integer bit-manipulation (clz, ctz, bswap, etc.)
//! - expr_builtins_overflow.rs: overflow-checking arithmetic
//! - expr_builtins_fpclass.rs: FP classification (fpclassify, isnan, isinf, etc.)

use super::lower::Lowerer;
use crate::common::types::{target_is_32bit, AddressSpace, CType, IrType};
use crate::frontend::parser::ast::{Expr, UnaryOp};
use crate::frontend::sema::builtins::{self, BuiltinIntrinsic, BuiltinKind};
use crate::ir::reexports::{
    CallInfo, Instruction, IntrinsicOp, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Terminator,
    Value,
};

impl Lowerer {
    /// Determine the return type for creal/crealf/creall/cimag/cimagf/cimagl based on function name.
    pub(super) fn creal_return_type(name: &str) -> IrType {
        match name {
            "crealf" | "__builtin_crealf" | "cimagf" | "__builtin_cimagf" => IrType::F32,
            "creall" | "__builtin_creall" | "cimagl" | "__builtin_cimagl" => IrType::F128,
            _ => IrType::F64, // creal, cimag, __builtin_creal, __builtin_cimag
        }
    }

    /// Expand `abs`/`labs`/`llabs` (and the `__builtin_*` spellings) to
    /// `select(x < 0, -x, x)`. Matches GCC `-fbuiltin`: a later user
    /// definition of the same name does not intercept the call.
    fn try_lower_abs_builtin(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        let ty = match name {
            "abs" | "__builtin_abs" => IrType::I32,
            "labs" | "__builtin_labs" => {
                if target_is_32bit() {
                    IrType::I32
                } else {
                    IrType::I64
                }
            }
            "llabs" | "__builtin_llabs" => IrType::I64,
            _ => return None,
        };
        if args.is_empty() {
            return Some(Operand::Const(IrConst::from_i64(0, ty)));
        }
        let arg = self.lower_expr_with_type(&args[0], ty);
        let zero = Operand::Const(IrConst::from_i64(0, ty));
        let is_neg = self.emit_cmp_val(IrCmpOp::Slt, arg, zero, ty);
        let neg = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest: neg,
            op: IrUnaryOp::Neg,
            src: arg,
            ty,
        });
        let result = self.fresh_value();
        self.emit(Instruction::Select {
            dest: result,
            cond: Operand::Value(is_neg),
            true_val: Operand::Value(neg),
            false_val: arg,
            ty,
        });
        Some(Operand::Value(result))
    }

    /// Try to lower a __builtin_* call. Returns Some(result) if handled.
    pub(super) fn try_lower_builtin_call(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        // GCC -fbuiltin (the default) expands abs/labs/llabs as the corresponding
        // integer absolute-value even when the user later provides a definition
        // of the same name. The user body is still emitted (so taking the
        // address of llabs works) but CALLS use the builtin. gcc.c-torture
        // 20021127-1 defines `llabs` to abort() and requires the builtin.
        // This must run BEFORE the is_defined override below.
        if let Some(result) = self.try_lower_abs_builtin(name, args) {
            return Some(result);
        }

        // If the user has defined a function body with a builtin name, prefer the
        // user's definition over the builtin. This matches GCC's behavior: a user
        // can define `double *__builtin_alloca() { return malloc(...); }` and GCC
        // will call that function instead of emitting the builtin alloca.
        // We check sema's is_defined flag (not known_functions, which includes mere
        // declarations) so that `void *__builtin_alloca(size_t);` alone still uses
        // the builtin, while a user-provided function body overrides it.
        //
        // EXCEPTION: SSE/SIMD intrinsic wrappers (_mm_*, _mm256_*) from our own
        // emmintrin.h headers are defined as static inline functions whose bodies
        // reference __builtin_ia32_vec_init_* (Nop stubs). The builtin intercept
        // MUST take priority for these, otherwise the Nop stubs return 0 and the
        // __CCC_M128I_FROM_BUILTIN macro dereferences NULL.
        if let Some(func_info) = self.sema_functions.get(name) {
            if func_info.is_defined {
                // Don't let is_defined bypass SSE/SIMD intrinsics -- those wrapper
                // bodies (from emmintrin.h etc.) exist only as fallback for compilers
                // that don't intercept them.  We identify them by name prefix rather
                // than by BuiltinKind::Intrinsic, because non-SSE intrinsics like
                // __builtin_alloca should still be overridable by user definitions.
                let is_sse_intrinsic = name.starts_with("__builtin_ia32_")
                    || name.starts_with("_mm_")
                    || name.starts_with("_mm256_")
                    || name.starts_with("_mm512_");
                if !is_sse_intrinsic {
                    return None;
                }
            }
        }

        // Compile-time folding of string-literal strcmp/strncmp/memcmp.
        // GCC folds these at -O0 already; the kernel RELIES on the fold for
        // its "unknown operator throws linker error" idiom (fair.c
        // vruntime_cmp uses __builtin_strcmp(OP, "<") in an if/else ladder
        // whose dead branch calls the undefined __BUILD_BUG_vruntime_cmp).
        // Without folding, the branch survives into codegen and vmlinux
        // fails to link. Fold only when BOTH arguments are string literals —
        // everything else lowers normally.
        if matches!(
            name,
            "__builtin_strcmp" | "__builtin_strncmp" | "__builtin_memcmp"
        ) {
            fn literal_of(e: &Expr) -> Option<&str> {
                match e {
                    Expr::StringLiteral(s, _) => Some(s.as_str()),
                    _ => None,
                }
            }
            if args.len() >= 2 {
                if let (Some(a), Some(b)) = (literal_of(&args[0]), literal_of(&args[1])) {
                    let n = if name == "__builtin_strcmp" {
                        Some(usize::MAX)
                    } else {
                        match args.get(2).and_then(|e| self.eval_const_expr(e)) {
                            Some(IrConst::I64(n)) if n >= 0 => Some(n as usize),
                            Some(IrConst::I32(n)) if n >= 0 => Some(n as usize),
                            // strncmp/memcmp with non-constant length: no fold.
                            _ => None,
                        }
                    };
                    if let Some(n) = n {
                        let ab = a.as_bytes();
                        let bb = b.as_bytes();
                        // Compare including the NUL terminator (strcmp stops
                        // there; memcmp of literal arrays sees the NUL too).
                        let mut result: i64 = 0;
                        let mut i = 0usize;
                        while i < n {
                            let ca = if i < ab.len() { ab[i] } else { 0 };
                            let cb = if i < bb.len() { bb[i] } else { 0 };
                            if ca != cb {
                                result = ca as i64 - cb as i64;
                                break;
                            }
                            // For strcmp/strncmp: stop at NUL.
                            if name != "__builtin_memcmp" && ca == 0 {
                                break;
                            }
                            if i >= ab.len() && i >= bb.len() {
                                break;
                            }
                            i += 1;
                        }
                        return Some(Operand::Const(IrConst::I32(result.signum() as i32)));
                    }
                }
            }
        }

        // Handle __builtin_choose_expr(const_expr, expr1, expr2)
        // This is a compile-time selection: if const_expr is nonzero, returns expr1, else expr2.
        if name == "__builtin_choose_expr" {
            if args.len() >= 3 {
                let condition = self.eval_const_expr(&args[0]);
                let is_nonzero = match condition {
                    Some(IrConst::I64(v)) => v != 0,
                    Some(IrConst::I32(v)) => v != 0,
                    Some(IrConst::I128(v)) => v != 0,
                    Some(IrConst::F64(v)) => v != 0.0,
                    Some(IrConst::F32(v)) => v != 0.0,
                    _ => {
                        // Try lowering to get a constant
                        let cond_val = self.lower_expr(&args[0]);
                        match cond_val {
                            Operand::Const(IrConst::I64(v)) => v != 0,
                            Operand::Const(IrConst::I32(v)) => v != 0,
                            _ => true, // default to first expr if indeterminate
                        }
                    }
                };
                return Some(if is_nonzero {
                    self.lower_expr(&args[1])
                } else {
                    self.lower_expr(&args[2])
                });
            }
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // Handle alloca specially - it needs dynamic stack allocation
        match name {
            "__builtin_alloca" | "__builtin_alloca_with_align" | "alloca" => {
                if let Some(size_expr) = args.first() {
                    let size_operand = self.lower_expr(size_expr);
                    let align = if name == "__builtin_alloca_with_align" && args.len() >= 2 {
                        // align is in bits
                        if let Expr::IntLiteral(bits, _) = &args[1] {
                            (*bits as usize) / 8
                        } else {
                            16
                        }
                    } else {
                        16 // default alignment
                    };
                    let dest = self.fresh_value();
                    self.emit(Instruction::DynAlloca {
                        dest,
                        size: size_operand,
                        align,
                    });
                    return Some(Operand::Value(dest));
                }
                return Some(Operand::Const(IrConst::I64(0)));
            }
            _ => {}
        }
        // Handle va_start/va_end/va_copy specially.
        // These builtins need a pointer to the va_list struct.
        // Use lower_va_list_pointer which correctly handles both local va_list
        // variables (array type, needs address-of) and va_list parameters
        // (pointer type after array-to-pointer decay, needs value load).
        match name {
            "__builtin_va_start" => {
                if let Some(ap_expr) = args.first() {
                    let ap_ptr_op = self.lower_va_list_pointer(ap_expr);
                    let ap_ptr = self.operand_to_value(ap_ptr_op);
                    self.emit(Instruction::VaStart {
                        va_list_ptr: ap_ptr,
                    });
                }
                return Some(Operand::Const(IrConst::I64(0)));
            }
            "__builtin_va_end" => {
                if let Some(ap_expr) = args.first() {
                    let ap_ptr_op = self.lower_va_list_pointer(ap_expr);
                    let ap_ptr = self.operand_to_value(ap_ptr_op);
                    self.emit(Instruction::VaEnd {
                        va_list_ptr: ap_ptr,
                    });
                }
                return Some(Operand::Const(IrConst::I64(0)));
            }
            "__builtin_va_copy" => {
                if args.len() >= 2 {
                    let dest_op = self.lower_va_list_pointer(&args[0]);
                    let src_op = self.lower_va_list_pointer(&args[1]);
                    let dest_ptr = self.operand_to_value(dest_op);
                    let src_ptr = self.operand_to_value(src_op);
                    self.emit(Instruction::VaCopy { dest_ptr, src_ptr });
                }
                return Some(Operand::Const(IrConst::I64(0)));
            }
            _ => {}
        }

        // __builtin_expect(exp, c) - branch prediction hint.
        // Returns exp unchanged, but must evaluate c for side effects.
        // __builtin_expect_with_probability(exp, c, prob) - same with probability.
        if name == "__builtin_expect" || name == "__builtin_expect_with_probability" {
            let result = if let Some(first) = args.first() {
                self.lower_expr(first)
            } else {
                Operand::Const(IrConst::I64(0))
            };
            // Evaluate remaining arguments for their side effects
            for arg in args.iter().skip(1) {
                self.lower_expr(arg);
            }
            return Some(result);
        }

        // __builtin_prefetch(addr, [rw], [locality]) - no-op performance hint
        if name == "__builtin_prefetch" {
            for arg in args {
                self.lower_expr(arg);
            }
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // __builtin_unreachable() - marks code path as unreachable. Emit a trap instruction
        // (ud2 on x86, brk #0 on ARM, ebreak on RISC-V) via Terminator::Unreachable.
        // This must NOT generate a call to abort() because abort() may not exist
        // (e.g., in kernel code).
        if name == "__builtin_unreachable" {
            self.terminate(Terminator::Unreachable);
            // Start a new (unreachable) block so subsequent code can still be lowered
            let dead_label = self.fresh_label();
            self.start_block(dead_label);
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // __builtin_trap() - generates a trap instruction to intentionally crash.
        // Like unreachable, this must emit the hardware trap directly, not call abort().
        if name == "__builtin_trap" {
            self.terminate(Terminator::Unreachable);
            let dead_label = self.fresh_label();
            self.start_block(dead_label);
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // Handle atomic builtins (delegated to expr_atomics.rs)
        if let Some(result) = self.try_lower_atomic_builtin(name, args) {
            return Some(result);
        }

        let builtin_info = builtins::resolve_builtin(name)?;
        match &builtin_info.kind {
            BuiltinKind::LibcAlias(libc_name) => {
                let mut arg_types: Vec<IrType> =
                    args.iter().map(|a| self.get_expr_type(a)).collect();
                let mut arg_vals: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let dest = self.fresh_value();
                // The builtin is callable without a prior libc prototype
                // (`__builtin_printf("%g", f)` in a TU that never includes
                // <stdio.h>).  The variadic contract of the printf family is
                // part of the builtin's own signature, so it must not depend
                // on whether the user happened to declare the libc symbol:
                // without it, a `float` argument was passed as a 4-byte F32
                // in an XMM register and printf read garbage (C11 6.5.2.2p6
                // requires float->double, char/short->int promotion).
                let libc_sig = self.func_meta.sigs.get(libc_name.as_str());
                let (variadic, n_fixed) = match libc_sig {
                    Some(sig) if sig.is_variadic => (true, sig.param_types.len()),
                    // Unprototyped declaration (`int printf();`): the default
                    // argument promotions apply to EVERY argument (C11
                    // 6.5.2.2p6), and the callee is the real variadic libc
                    // function — use the builtin's own fixed-arity contract.
                    Some(sig) if sig.param_types.is_empty() => {
                        match builtin_variadic_fixed_arity(libc_name.as_str()) {
                            Some(n) => (true, n),
                            None => (true, arg_vals.len()),
                        }
                    }
                    Some(_) => (false, arg_vals.len()),
                    None => match builtin_variadic_fixed_arity(libc_name.as_str()) {
                        Some(n) => (true, n),
                        None => (false, arg_vals.len()),
                    },
                };
                let libc_return_type = libc_sig.map(|s| s.return_type);
                if variadic {
                    self.promote_variadic_args(&mut arg_vals, &mut arg_types, n_fixed);
                }
                let return_type = self
                    .builtin_return_type(name)
                    .or(libc_return_type)
                    .unwrap_or(crate::common::types::target_int_ir_type());
                let struct_arg_sizes = vec![None; arg_vals.len()];
                self.emit(Instruction::Call {
                    func: libc_name.clone(),
                    info: CallInfo {
                        dest: Some(dest),
                        args: arg_vals,
                        arg_types,
                        return_type,
                        is_variadic: variadic,
                        num_fixed_args: n_fixed,
                        struct_arg_sizes,
                        struct_arg_aligns: vec![],
                        struct_arg_classes: Vec::new(),
                        struct_arg_riscv_float_classes: Vec::new(),
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false,
                        is_fastcall: false,
                        is_pure: false,
                        is_const: false,
                        ret_eightbyte_classes: Vec::new(),
                        ret_is_f128_sse: false,
                    },
                });
                Some(Operand::Value(dest))
            }
            BuiltinKind::Identity => Some(
                args.first()
                    .map_or(Operand::Const(IrConst::I64(0)), |a| self.lower_expr(a)),
            ),
            BuiltinKind::ConstantF128(bytes) => {
                // _Float128 constant: full 16-byte binary128 payload carried
                // as an I128 bit pattern (the F128-const materializers in
                // calls/returns/memory all speak I128).
                let bits = u128::from_be_bytes(*bytes) as i128;
                Some(Operand::Const(IrConst::I128(bits)))
            }
            BuiltinKind::ConstantF64(val) => {
                let is_float_variant = name == "__builtin_inff"
                    || name == "__builtin_huge_valf"
                    || name == "__builtin_nanf";
                let is_long_double_variant = name == "__builtin_infl"
                    || name == "__builtin_huge_vall"
                    || name == "__builtin_nanl";
                if is_float_variant {
                    Some(Operand::Const(IrConst::F32(*val as f32)))
                } else if is_long_double_variant {
                    Some(Operand::Const(IrConst::long_double(*val)))
                } else {
                    Some(Operand::Const(IrConst::F64(*val)))
                }
            }
            BuiltinKind::Intrinsic(intrinsic) => {
                self.lower_builtin_intrinsic(intrinsic, name, args)
            }
        }
    }

    /// Lower a builtin intrinsic (the BuiltinKind::Intrinsic arm of try_lower_builtin_call).
    fn lower_builtin_intrinsic(
        &mut self,
        intrinsic: &BuiltinIntrinsic,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        match intrinsic {
            BuiltinIntrinsic::FpCompare => self.lower_fp_compare(name, args),
            BuiltinIntrinsic::AddOverflow => self.lower_overflow_builtin(name, args, IrBinOp::Add),
            BuiltinIntrinsic::SubOverflow => self.lower_overflow_builtin(name, args, IrBinOp::Sub),
            BuiltinIntrinsic::MulOverflow => self.lower_overflow_builtin(name, args, IrBinOp::Mul),
            BuiltinIntrinsic::AddOverflowP => self.lower_overflow_p_builtin(args, IrBinOp::Add),
            BuiltinIntrinsic::SubOverflowP => self.lower_overflow_p_builtin(args, IrBinOp::Sub),
            BuiltinIntrinsic::MulOverflowP => self.lower_overflow_p_builtin(args, IrBinOp::Mul),
            BuiltinIntrinsic::Clz => self.lower_unary_intrinsic(name, args, IrUnaryOp::Clz),
            BuiltinIntrinsic::Ctz => self.lower_unary_intrinsic(name, args, IrUnaryOp::Ctz),
            BuiltinIntrinsic::Ffs => self.lower_ffs_intrinsic(name, args),
            BuiltinIntrinsic::Clrsb => self.lower_clrsb_intrinsic(name, args),
            BuiltinIntrinsic::Bswap => self.lower_bswap_intrinsic(name, args),
            BuiltinIntrinsic::Popcount => {
                self.lower_unary_intrinsic(name, args, IrUnaryOp::Popcount)
            }
            BuiltinIntrinsic::Parity => self.lower_parity_intrinsic(name, args),
            BuiltinIntrinsic::ComplexReal => self.lower_complex_part(name, args, true),
            BuiltinIntrinsic::ComplexImag => self.lower_complex_part(name, args, false),
            BuiltinIntrinsic::ComplexConj => {
                if !args.is_empty() {
                    Some(self.lower_complex_conj(&args[0]))
                } else {
                    Some(Operand::Const(IrConst::F64(0.0)))
                }
            }
            BuiltinIntrinsic::Fence => Some(Operand::Const(IrConst::I64(0))),
            BuiltinIntrinsic::FpClassify => self.lower_builtin_fpclassify(args),
            BuiltinIntrinsic::IsNan => self.lower_builtin_isnan(args),
            BuiltinIntrinsic::IsInf => self.lower_builtin_isinf(args),
            BuiltinIntrinsic::IsFinite => self.lower_builtin_isfinite(args),
            BuiltinIntrinsic::IsNormal => self.lower_builtin_isnormal(args),
            BuiltinIntrinsic::SignBit => self.lower_builtin_signbit(args),
            BuiltinIntrinsic::IsInfSign => self.lower_builtin_isinf_sign(args),
            BuiltinIntrinsic::Alloca => {
                // Handled earlier in try_lower_builtin_call - should not reach here
                Some(Operand::Const(IrConst::I64(0)))
            }
            BuiltinIntrinsic::ComplexConstruct => self.lower_complex_construct(args),
            BuiltinIntrinsic::VaStart | BuiltinIntrinsic::VaEnd | BuiltinIntrinsic::VaCopy => {
                unreachable!("va builtins handled earlier by name match")
            }
            BuiltinIntrinsic::ConstantP => self.lower_constant_p(args),
            // __builtin_object_size(ptr, type) -> compile-time object size, or unknown
            // For types 0 and 1: return (size_t)-1 when size is unknown
            // For types 2 and 3: return 0 when size is unknown
            // We conservatively return "unknown" since we don't do points-to analysis.
            BuiltinIntrinsic::ObjectSize => {
                let obj_type = if args.len() >= 2 {
                    match self.eval_const_expr(&args[1]) {
                        Some(IrConst::I64(v)) => v,
                        Some(IrConst::I32(v)) => v as i64,
                        _ => 0,
                    }
                } else {
                    0
                };
                // Fast path: string literal. Size is strlen+1 (NUL included).
                // Highest-ROI object_size case for fortify; every serious C
                // compiler folds this without full points-to analysis.
                if let Some(arg0) = args.first() {
                    if let Expr::StringLiteral(s, _) = arg0 {
                        let sz = (s.len() + 1) as i64;
                        if args.len() >= 2 {
                            let _ = self.lower_expr(&args[1]);
                        }
                        return Some(Operand::Const(IrConst::I64(sz)));
                    }
                }
                // Conservative unknown: evaluate args for side effects.
                for arg in args {
                    let _ = self.lower_expr(arg);
                }
                // Types 0 and 1: maximum estimate -> -1 (unknown)
                // Types 2 and 3: minimum estimate -> 0 (unknown)
                let result = if obj_type == 2 || obj_type == 3 {
                    0i64
                } else {
                    -1i64
                };
                Some(Operand::Const(IrConst::I64(result)))
            }
            // __builtin_classify_type(expr) -> GCC type class integer
            BuiltinIntrinsic::ClassifyType => {
                let result = if let Some(arg) = args.first() {
                    // Special case: bare function identifiers decay to pointer
                    // type (class 5). expr_ctype may return CType::Int as
                    // fallback since functions aren't stored as variables.
                    if let Expr::Identifier(fname, _) = arg {
                        if self.known_functions.contains(fname.as_str()) {
                            5i64 // pointer_type_class (function decays to pointer)
                        } else {
                            let ctype = self.expr_ctype(arg);
                            classify_ctype(&ctype)
                        }
                    } else {
                        let ctype = self.expr_ctype(arg);
                        classify_ctype(&ctype)
                    }
                } else {
                    0i64 // no_type_class
                };
                Some(Operand::Const(IrConst::I64(result)))
            }
            // Nop intrinsic - just evaluate args for side effects and return 0
            BuiltinIntrinsic::Nop => {
                for arg in args {
                    self.lower_expr(arg);
                }
                Some(Operand::Const(IrConst::I64(0)))
            }
            // __builtin_cpu_init() - no-op on glibc systems (auto-initialized)
            BuiltinIntrinsic::CpuInit => Some(Operand::Const(IrConst::I32(0))),
            // __builtin_cpu_supports("feature") - returns 1 if feature is enabled for target.
            // For Raptor Lake (-march=raptorlake) we enable AES, PCLMUL, AVX, AVX2, etc.
            // This matches GCC's behavior where __builtin_cpu_supports checks runtime CPUID,
            // but for PGO we want to enable the optimized path when the compile-time target supports it.
            BuiltinIntrinsic::CpuSupports => {
                for arg in args {
                    self.lower_expr(arg);
                }
                // Compile-time fold against the project's compile target
                // (x86-64 Raptor Lake). The old behavior returned 1 for EVERY
                // feature except avx512* - including features Raptor Lake
                // does not have (AMX, XOP, SSE4A, ...), which turned runtime
                // dispatch into guaranteed SIGILL paths. Fold from an exact
                // allowlist instead; UNKNOWN features fold to 0 (the safe
                // direction for dispatchers: they fall back to generic code).
                let mut ret = 0;
                if !args.is_empty() {
                    if let crate::frontend::parser::ast::Expr::StringLiteral(s, _) = &args[0] {
                        let feat = s.to_ascii_lowercase();
                        // Features present on Raptor Lake (and the x86-64-v3
                        // baseline this project targets).
                        const PRESENT: &[&str] = &[
                            "cmov",
                            "mmx",
                            "sse",
                            "sse2",
                            "sse3",
                            "ssse3",
                            "sse4.1",
                            "sse4.2",
                            "popcnt",
                            "avx",
                            "avx2",
                            "fma",
                            "bmi",
                            "bmi2",
                            "lzcnt",
                            "movbe",
                            "f16c",
                            "aes",
                            "pclmul",
                            "pclmulqdq",
                            "rdrnd",
                            "rdrand",
                            "rdseed",
                            "sha",
                            "adx",
                            "fsgsbase",
                            "xsave",
                            "xsaveopt",
                            "xsavec",
                            "avxvnni",
                            "vaes",
                            "vpclmulqdq",
                            "gfni",
                            "cx16",
                            "fxsr",
                            "osxsave",
                        ];
                        if PRESENT.contains(&feat.as_str()) {
                            ret = 1;
                        }
                        // Everything else (avx512*, amx*, xop, sse4a, fma4,
                        // tbm, prefetchwt1, ...) folds to 0.
                    }
                }
                Some(Operand::Const(IrConst::I32(ret)))
            }
            // X86 SSE/AES/CRC intrinsics - delegated to lower_x86_intrinsic
            BuiltinIntrinsic::X86Lfence
            | BuiltinIntrinsic::X86Mfence
            | BuiltinIntrinsic::X86Sfence
            | BuiltinIntrinsic::X86Pause
            | BuiltinIntrinsic::X86Vzeroupper
            | BuiltinIntrinsic::X86Rdtsc
            | BuiltinIntrinsic::X86Rdtscp
            | BuiltinIntrinsic::X86Clflush
            | BuiltinIntrinsic::X86Movnti
            | BuiltinIntrinsic::X86Movnti64
            | BuiltinIntrinsic::X86Movntdq
            | BuiltinIntrinsic::X86Movntpd
            | BuiltinIntrinsic::X86Loaddqu
            | BuiltinIntrinsic::X86Pcmpeqb128
            | BuiltinIntrinsic::X86Pcmpeqd128
            | BuiltinIntrinsic::X86Psubusb128
            | BuiltinIntrinsic::X86Psubsb128
            | BuiltinIntrinsic::X86Por128
            | BuiltinIntrinsic::X86Pand128
            | BuiltinIntrinsic::X86Pxor128
            | BuiltinIntrinsic::X86Set1Epi8
            | BuiltinIntrinsic::X86Set1Epi32
            | BuiltinIntrinsic::X86Aesenc128
            | BuiltinIntrinsic::X86Aesenclast128
            | BuiltinIntrinsic::X86Aesdec128
            | BuiltinIntrinsic::X86Aesdeclast128
            | BuiltinIntrinsic::X86Aesimc128
            | BuiltinIntrinsic::X86Aeskeygenassist128
            | BuiltinIntrinsic::X86Pclmulqdq128
            | BuiltinIntrinsic::X86Pslldqi128
            | BuiltinIntrinsic::X86Psrldqi128
            | BuiltinIntrinsic::X86Psllqi128
            | BuiltinIntrinsic::X86Psrlqi128
            | BuiltinIntrinsic::X86Pshufd128
            | BuiltinIntrinsic::X86Loadldi128
            | BuiltinIntrinsic::X86Paddw128
            | BuiltinIntrinsic::X86Psubw128
            | BuiltinIntrinsic::X86Pmulhw128
            | BuiltinIntrinsic::X86Pmaddwd128
            | BuiltinIntrinsic::X86Pcmpgtw128
            | BuiltinIntrinsic::X86Pcmpgtb128
            | BuiltinIntrinsic::X86Psllwi128
            | BuiltinIntrinsic::X86Psrlwi128
            | BuiltinIntrinsic::X86Psrawi128
            | BuiltinIntrinsic::X86Psradi128
            | BuiltinIntrinsic::X86Pslldi128
            | BuiltinIntrinsic::X86Psrldi128
            | BuiltinIntrinsic::X86Paddd128
            | BuiltinIntrinsic::X86Psubd128
            | BuiltinIntrinsic::X86Packssdw128
            | BuiltinIntrinsic::X86Packsswb128
            | BuiltinIntrinsic::X86Packuswb128
            | BuiltinIntrinsic::X86Punpcklbw128
            | BuiltinIntrinsic::X86Punpckhbw128
            | BuiltinIntrinsic::X86Punpcklwd128
            | BuiltinIntrinsic::X86Punpckhwd128
            | BuiltinIntrinsic::X86Set1Epi16
            | BuiltinIntrinsic::X86Pinsrw128
            | BuiltinIntrinsic::X86Cvtsi32Si128
            | BuiltinIntrinsic::X86Pshuflw128
            | BuiltinIntrinsic::X86Pshufhw128
            | BuiltinIntrinsic::X86Storedqu
            | BuiltinIntrinsic::X86Storeldi128
            | BuiltinIntrinsic::X86Pmovmskb128
            | BuiltinIntrinsic::X86Pextrw128
            | BuiltinIntrinsic::X86Cvtsi128Si32
            | BuiltinIntrinsic::X86Cvtsi128Si64
            | BuiltinIntrinsic::X86Crc32_8
            | BuiltinIntrinsic::X86Crc32_16
            | BuiltinIntrinsic::X86Crc32_32
            | BuiltinIntrinsic::X86Crc32_64
            | BuiltinIntrinsic::X86Pinsrd128
            | BuiltinIntrinsic::X86Pextrd128
            | BuiltinIntrinsic::X86Pinsrb128
            | BuiltinIntrinsic::X86Pextrb128
            | BuiltinIntrinsic::X86Pinsrq128
            | BuiltinIntrinsic::X86Pextrq128
            | BuiltinIntrinsic::X86Paddb128
            | BuiltinIntrinsic::X86Psubb128
            | BuiltinIntrinsic::X86Psubusw128
            | BuiltinIntrinsic::X86Psadbw128
            | BuiltinIntrinsic::X86Pmullw128
            | BuiltinIntrinsic::X86Pmaddubsw128
            | BuiltinIntrinsic::X86Phaddw128
            | BuiltinIntrinsic::X86Phaddd128
            | BuiltinIntrinsic::X86Pshufb128
            | BuiltinIntrinsic::X86Pabsb128
            | BuiltinIntrinsic::X86Pabsw128
            | BuiltinIntrinsic::X86Pabsd128
            | BuiltinIntrinsic::X86Palignr128
            | BuiltinIntrinsic::X86Pmaxub128
            | BuiltinIntrinsic::X86Pminub128
            | BuiltinIntrinsic::X86Pblendvb128
            | BuiltinIntrinsic::X86MaxPs128
            | BuiltinIntrinsic::X86MinPs128
            | BuiltinIntrinsic::X86ShufPsValue
            | BuiltinIntrinsic::X86CvtPd2PsValue
            | BuiltinIntrinsic::X86Vextractf128V
            | BuiltinIntrinsic::X86MaxPs256V
            | BuiltinIntrinsic::X86MinPs256V
            | BuiltinIntrinsic::X86AndPs256V
            | BuiltinIntrinsic::X86CmpPs256V
            | BuiltinIntrinsic::X86Pblendw128
            | BuiltinIntrinsic::X86Pmovzxbw128
            | BuiltinIntrinsic::X86Pmovzxwd128
            | BuiltinIntrinsic::X86Psllw128
            | BuiltinIntrinsic::X86Psrlw128
            | BuiltinIntrinsic::X86Loadu256
            | BuiltinIntrinsic::X86Storeu256
            | BuiltinIntrinsic::X86Load256
            | BuiltinIntrinsic::X86Store256
            | BuiltinIntrinsic::X86Paddb256
            | BuiltinIntrinsic::X86Paddw256
            | BuiltinIntrinsic::X86Paddd256
            | BuiltinIntrinsic::X86Psubb256
            | BuiltinIntrinsic::X86Psubw256
            | BuiltinIntrinsic::X86Psubusw256
            | BuiltinIntrinsic::X86Psadbw256
            | BuiltinIntrinsic::X86Pmaddubsw256
            | BuiltinIntrinsic::X86Pmaddwd256
            | BuiltinIntrinsic::X86Pcmpeqb256
            | BuiltinIntrinsic::X86Pmovmskb256
            | BuiltinIntrinsic::X86Pshufb256
            | BuiltinIntrinsic::X86Pabsb256
            | BuiltinIntrinsic::X86Pabsw256
            | BuiltinIntrinsic::X86Pmaxub256
            | BuiltinIntrinsic::X86Pminub256
            | BuiltinIntrinsic::X86Pxor256
            | BuiltinIntrinsic::X86Por256
            | BuiltinIntrinsic::X86Pand256
            | BuiltinIntrinsic::X86Psllidi256
            | BuiltinIntrinsic::X86Psrlidi256
            | BuiltinIntrinsic::X86Psllwi256
            | BuiltinIntrinsic::X86Psrlwi256
            | BuiltinIntrinsic::X86Broadcast128to256
            | BuiltinIntrinsic::X86Zext128to256
            | BuiltinIntrinsic::X86Cast256to128
            | BuiltinIntrinsic::X86Insert128to256
            | BuiltinIntrinsic::X86SetEpi8_256
            | BuiltinIntrinsic::X86SetEpi16_256
            | BuiltinIntrinsic::X86SetEpi32_256
            | BuiltinIntrinsic::X86SetEpi64x256
            | BuiltinIntrinsic::X86Permutevar8x32
            | BuiltinIntrinsic::X86Dpbusd128
            | BuiltinIntrinsic::X86Dpbusds128
            | BuiltinIntrinsic::X86Dpwusd128
            | BuiltinIntrinsic::X86Dpwusds128
            | BuiltinIntrinsic::X86Dpbusd256
            | BuiltinIntrinsic::X86Dpbusds256
            | BuiltinIntrinsic::X86Dpwusd256
            | BuiltinIntrinsic::X86Dpwusds256
            | BuiltinIntrinsic::X86Dpbssd128
            | BuiltinIntrinsic::X86Dpbssds128
            | BuiltinIntrinsic::X86Dpbsud128
            | BuiltinIntrinsic::X86Dpbsuds128
            | BuiltinIntrinsic::X86Dpbuud128
            | BuiltinIntrinsic::X86Dpbuuds128
            | BuiltinIntrinsic::X86Dpbssd256
            | BuiltinIntrinsic::X86Dpbssds256
            | BuiltinIntrinsic::X86Dpbsud256
            | BuiltinIntrinsic::X86Dpbsuds256
            | BuiltinIntrinsic::X86Dpbuud256
            | BuiltinIntrinsic::X86Dpbuuds256
            | BuiltinIntrinsic::X86Dpwuud128
            | BuiltinIntrinsic::X86Dpwuuds128
            | BuiltinIntrinsic::X86Dpwssd128
            | BuiltinIntrinsic::X86Dpwssds128
            | BuiltinIntrinsic::X86Dpwuud256
            | BuiltinIntrinsic::X86Dpwuuds256
            | BuiltinIntrinsic::X86Dpwssd256
            | BuiltinIntrinsic::X86Dpwssds256
            | BuiltinIntrinsic::X86Gf2p8mulb128
            | BuiltinIntrinsic::X86Gf2p8affineqb128
            | BuiltinIntrinsic::X86Gf2p8affineinvqb128
            | BuiltinIntrinsic::X86Aesenc256
            | BuiltinIntrinsic::X86Aesenclast256
            | BuiltinIntrinsic::X86Aesdec256
            | BuiltinIntrinsic::X86Aesdeclast256
            | BuiltinIntrinsic::X86Vpclmulqdq256
            | BuiltinIntrinsic::X86Paddusb128
            | BuiltinIntrinsic::X86Paddsb128
            | BuiltinIntrinsic::X86Paddusw128
            | BuiltinIntrinsic::X86Paddsw128
            | BuiltinIntrinsic::X86Psubsw128
            | BuiltinIntrinsic::X86Pandn128
            | BuiltinIntrinsic::X86Pcmpeqw128
            | BuiltinIntrinsic::X86Pcmpgtd128
            | BuiltinIntrinsic::X86Pavgb128
            | BuiltinIntrinsic::X86Pavgw128
            | BuiltinIntrinsic::X86Pminsw128
            | BuiltinIntrinsic::X86Pmaxsw128
            | BuiltinIntrinsic::X86Pmulhuw128
            | BuiltinIntrinsic::X86Paddq128
            | BuiltinIntrinsic::X86Psubq128
            | BuiltinIntrinsic::X86Punpckldq128
            | BuiltinIntrinsic::X86Punpckhdq128
            | BuiltinIntrinsic::X86Punpcklqdq128
            | BuiltinIntrinsic::X86Punpckhqdq128
            | BuiltinIntrinsic::X86Setzero128
            | BuiltinIntrinsic::X86Testz128
            | BuiltinIntrinsic::X86Pmulld256
            | BuiltinIntrinsic::X86Psubd256
            | BuiltinIntrinsic::X86Paddq256
            | BuiltinIntrinsic::X86Psubq256
            | BuiltinIntrinsic::X86Pandn256
            | BuiltinIntrinsic::X86Pcmpeqd256
            | BuiltinIntrinsic::X86Pcmpeqq256
            | BuiltinIntrinsic::X86Pcmpgtd256
            | BuiltinIntrinsic::X86Pcmpgtq256
            | BuiltinIntrinsic::X86Extracti128
            | BuiltinIntrinsic::X86Setzero256
            | BuiltinIntrinsic::X86AddPs256
            | BuiltinIntrinsic::X86SubPs256
            | BuiltinIntrinsic::X86MulPs256
            | BuiltinIntrinsic::X86AddPd256
            | BuiltinIntrinsic::X86SubPd256
            | BuiltinIntrinsic::X86MulPd256
            | BuiltinIntrinsic::X86LoaduPs256
            | BuiltinIntrinsic::X86StoreuPs256
            | BuiltinIntrinsic::X86LoaduPd256
            | BuiltinIntrinsic::X86StoreuPd256
            | BuiltinIntrinsic::X86Permute2x128
            | BuiltinIntrinsic::X86Permute4x64
            | BuiltinIntrinsic::X86Pshufd256
            | BuiltinIntrinsic::X86Punpcklbw256
            | BuiltinIntrinsic::X86Punpckhbw256
            | BuiltinIntrinsic::X86Punpcklwd256
            | BuiltinIntrinsic::X86Punpckhwd256
            | BuiltinIntrinsic::X86Punpckldq256
            | BuiltinIntrinsic::X86Punpckhdq256
            | BuiltinIntrinsic::X86Punpcklqdq256
            | BuiltinIntrinsic::X86Punpckhqdq256
            | BuiltinIntrinsic::X86Pslldqi256
            | BuiltinIntrinsic::X86Psrldqi256
            | BuiltinIntrinsic::X86Psllqi256
            | BuiltinIntrinsic::X86Psrlqi256
            | BuiltinIntrinsic::X86Pmullw256
            | BuiltinIntrinsic::X86Pmulhw256
            | BuiltinIntrinsic::X86Pminsd256
            | BuiltinIntrinsic::X86Pmaxsd256
            | BuiltinIntrinsic::X86Pmovzxbw256
            | BuiltinIntrinsic::X86Pmovzxbd256
            | BuiltinIntrinsic::X86Pmovzxwd256
            | BuiltinIntrinsic::X86Pmovsxbw256
            | BuiltinIntrinsic::X86Pmovsxbd256
            | BuiltinIntrinsic::X86Pmovsxwd256
            | BuiltinIntrinsic::X86Psrawi256
            | BuiltinIntrinsic::X86Psradi256
            | BuiltinIntrinsic::X86Packssdw256
            | BuiltinIntrinsic::X86Packuswb256
            | BuiltinIntrinsic::X86Phaddw256
            | BuiltinIntrinsic::X86Phaddd256
            | BuiltinIntrinsic::X86Pabsd256
            | BuiltinIntrinsic::X86Pmuludq256
            | BuiltinIntrinsic::X86XorPs
            | BuiltinIntrinsic::X86AndPs
            | BuiltinIntrinsic::X86OrPs
            | BuiltinIntrinsic::X86AddPs
            | BuiltinIntrinsic::X86SubPs
            | BuiltinIntrinsic::X86MulPs
            | BuiltinIntrinsic::X86AddPd
            | BuiltinIntrinsic::X86SubPd
            | BuiltinIntrinsic::X86MulPd
            | BuiltinIntrinsic::X86MulEpu32
            | BuiltinIntrinsic::X86MulEpi32
            | BuiltinIntrinsic::X86MulloEpi32
            | BuiltinIntrinsic::X86Pcmpgtb256
            | BuiltinIntrinsic::X86CastReinterpret => self.lower_x86_intrinsic(intrinsic, args),
            // __builtin_shufflevector(v1, v2, i0, i1, i2, i3): 4-lane 32-bit
            // lane gather with constant indices (Clang/GCC generic shuffle).
            BuiltinIntrinsic::ShuffleVector => self.lower_shufflevector(args),
            // GCC vector extension: __builtin_shuffle(v, mask) |
            // __builtin_shuffle(v1, v2, mask). result[i] comes from
            // concat(v1, v2)[mask[i] mod N] (N = element count of v1; the
            // 2-arg form uses v twice). Fully unrolled runtime-indexed
            // gather: correct for constant AND variable masks, any element
            // width, any power-of-two-or-not lane count.
            BuiltinIntrinsic::Shuffle => self.lower_shuffle(args),
            // Generic SIMD family: __lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}
            BuiltinIntrinsic::LcccSimd => self.lower_lccc_simd(name, args),
            // __builtin___*_chk: fortification builtins forward to unchecked libc equivalents.
            // Each __builtin___X_chk(args..., extra_check_args...) becomes X(args...).
            BuiltinIntrinsic::FortifyChk => self.lower_fortify_chk(name, args),
            // __builtin_va_arg_pack / __builtin_va_arg_pack_len: forward the
            // wrapper's caller variadic arguments during inlining.  Lowered
            // to a zero-arg sentinel CALL (`__lccc_va_arg_pack` /
            // `__lccc_va_arg_pack_len`) that the inliner expands when the
            // enclosing always_inline variadic wrapper is cloned into a call
            // site: the sentinel call is deleted and the call site's
            // arguments beyond the wrapper's named parameters are spliced
            // into the consuming call's argument list (va_arg_pack_len
            // becomes an I32 constant — the count of forwarded arguments).
            // If the sentinel ever survives inlining (wrapper not inlined),
            // the undefined symbol `__lccc_va_arg_pack` at link time mirrors
            // GCC's "va_arg_pack must be inlined" diagnostic class.
            BuiltinIntrinsic::VaArgPack => {
                for arg in args {
                    self.lower_expr(arg);
                }
                let sentinel = if name == "__builtin_va_arg_pack" {
                    "__lccc_va_arg_pack"
                } else {
                    "__lccc_va_arg_pack_len"
                };
                let dest = self.fresh_value();
                let struct_arg_sizes = vec![];
                self.emit(Instruction::Call {
                    func: sentinel.to_string(),
                    info: CallInfo {
                        dest: Some(dest),
                        args: vec![],
                        arg_types: vec![],
                        return_type: IrType::I32,
                        is_variadic: false,
                        num_fixed_args: 0,
                        struct_arg_sizes,
                        struct_arg_aligns: vec![],
                        struct_arg_classes: Vec::new(),
                        struct_arg_riscv_float_classes: Vec::new(),
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false,
                        is_fastcall: false,
                        is_pure: false,
                        is_const: false,
                        ret_eightbyte_classes: Vec::new(),
                        ret_is_f128_sse: false,
                    },
                });
                Some(Operand::Value(dest))
            }
            // __builtin_thread_pointer() -> returns the TLS base address (thread pointer)
            BuiltinIntrinsic::ThreadPointer => {
                let dest_val = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(dest_val),
                    op: IntrinsicOp::ThreadPointer,
                    dest_ptr: None,
                    args: vec![],
                });
                Some(Operand::Value(dest_val))
            }

            // __builtin_frame_address(level) / __builtin_return_address(level)
            // Only level 0 is supported; higher levels return 0.
            BuiltinIntrinsic::BuiltinSetjmp => {
                let buffer = args
                    .first()
                    .map(|arg| self.lower_expr(arg))
                    .unwrap_or(Operand::Const(IrConst::I64(0)));
                let dest = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: IntrinsicOp::BuiltinSetjmp,
                    dest_ptr: None,
                    args: vec![buffer],
                });
                Some(Operand::Value(dest))
            }
            BuiltinIntrinsic::ApplyArgs => {
                // __builtin_apply_args():
                //   1. area_size = Intrinsic{ApplyArgsAreaSize} (target-specific
                //      constant computed by the backend, which knows the
                //      incoming argument classification)
                //   2. area = DynAlloca(area_size, align=16) — survives across
                //      calls (it lives inside our own frame) and forces a
                //      frame pointer, which the i686 save arm relies on.
                //   3. Intrinsic{SaveApplyArgs, dest_ptr: area}
                // The result is the area pointer, valid as __builtin_apply's
                // second argument.
                let size_val = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(size_val),
                    op: IntrinsicOp::ApplyArgsAreaSize,
                    dest_ptr: None,
                    args: Vec::new(),
                });
                let area = self.fresh_value();
                self.emit(Instruction::DynAlloca {
                    dest: area,
                    size: Operand::Value(size_val),
                    align: 16,
                });
                self.emit(Instruction::Intrinsic {
                    dest: None,
                    op: IntrinsicOp::SaveApplyArgs,
                    dest_ptr: Some(area),
                    args: vec![Operand::Value(area)],
                });
                Some(Operand::Value(area))
            }
            BuiltinIntrinsic::Apply => {
                // __builtin_apply(func, args, size):
                //   result = DynAlloca(APPLY_RESULT_SIZE, align=16)
                //   Intrinsic{DoBuiltinApply, args: [func, args, result, size]}
                // The backend restores the argument registers from the save
                // area (i686 additionally re-stages `size` bytes of stack
                // arguments), performs the indirect call, and captures the
                // return value (rax/rdx/xmm0 resp. eax/edx) into the result
                // area.  The result area pointer is __builtin_apply's return
                // value (consumed by __builtin_return).
                let func = args
                    .first()
                    .map(|arg| self.lower_expr(arg))
                    .unwrap_or(Operand::Const(IrConst::I64(0)));
                let save_area = args
                    .get(1)
                    .map(|arg| self.lower_expr(arg))
                    .unwrap_or(Operand::Const(IrConst::I64(0)));
                let size = args
                    .get(2)
                    .map(|arg| self.lower_expr(arg))
                    .unwrap_or(Operand::Const(IrConst::I64(0)));
                let result = self.fresh_value();
                // Result block: x86-64 stores rax(8) + rdx(8) + xmm0(16);
                // i686 stores eax(4) + edx(4).  A uniform 32 bytes keeps the
                // lowering target-independent.
                self.emit(Instruction::DynAlloca {
                    dest: result,
                    size: Operand::Const(IrConst::I64(32)),
                    align: 16,
                });
                self.emit(Instruction::Intrinsic {
                    dest: None,
                    op: IntrinsicOp::DoBuiltinApply,
                    dest_ptr: None,
                    args: vec![func, save_area, Operand::Value(result), size],
                });
                Some(Operand::Value(result))
            }
            BuiltinIntrinsic::BuiltinReturn => {
                // __builtin_return(block): restore the return registers from
                // the block, then return from the current function.  Code
                // after the builtin is unreachable; start a dead block like
                // __builtin_unreachable does.
                let block = args
                    .first()
                    .map(|arg| self.lower_expr(arg))
                    .unwrap_or(Operand::Const(IrConst::I64(0)));
                self.emit(Instruction::Intrinsic {
                    dest: None,
                    op: IntrinsicOp::RestoreApplyResult,
                    dest_ptr: None,
                    args: vec![block],
                });
                self.terminate(Terminator::Return(None));
                let dead_label = self.fresh_label();
                self.start_block(dead_label);
                Some(Operand::Const(IrConst::I64(0)))
            }
            BuiltinIntrinsic::BuiltinLongjmp => {
                let buffer = args
                    .first()
                    .map(|arg| self.lower_expr(arg))
                    .unwrap_or(Operand::Const(IrConst::I64(0)));
                // Evaluate the required value argument for source side effects;
                // GCC constrains it to one for __builtin_longjmp.
                let value = args
                    .get(1)
                    .map(|arg| self.lower_expr(arg))
                    .unwrap_or(Operand::Const(IrConst::I32(1)));
                self.emit(Instruction::Intrinsic {
                    dest: None,
                    op: IntrinsicOp::BuiltinLongjmp,
                    dest_ptr: None,
                    args: vec![buffer, value],
                });
                Some(Operand::Const(IrConst::I32(0)))
            }
            BuiltinIntrinsic::FrameAddress | BuiltinIntrinsic::ReturnAddress => {
                // Evaluate the level argument
                let level = if !args.is_empty() {
                    self.lower_expr(&args[0])
                } else {
                    Operand::Const(IrConst::I64(0))
                };
                // Only level 0 is supported; for other levels return NULL
                let is_level_zero = matches!(
                    &level,
                    Operand::Const(IrConst::I64(0)) | Operand::Const(IrConst::I32(0))
                );
                if is_level_zero {
                    let op = if *intrinsic == BuiltinIntrinsic::FrameAddress {
                        IntrinsicOp::FrameAddress
                    } else {
                        IntrinsicOp::ReturnAddress
                    };
                    let dest_val = self.fresh_value();
                    self.emit(Instruction::Intrinsic {
                        dest: Some(dest_val),
                        op,
                        dest_ptr: None,
                        args: vec![],
                    });
                    Some(Operand::Value(dest_val))
                } else {
                    // TODO: support non-zero levels by walking the frame pointer chain
                    // Non-zero levels: return NULL (matching GCC behavior for non-optimized)
                    Some(Operand::Const(IrConst::I64(0)))
                }
            }
        }
    }

    /// Lower __builtin___X_chk fortification builtins by forwarding all arguments
    /// to the glibc __X_chk runtime function (e.g., __builtin___snprintf_chk -> __snprintf_chk).
    /// This avoids infinite recursion when glibc's fortification headers redefine functions
    /// like snprintf as always_inline wrappers that call __builtin___snprintf_chk.
    fn lower_fortify_chk(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        // Map __builtin___X_chk to the glibc runtime __X_chk function.
        // All arguments are forwarded as-is (the runtime function does the checking).
        let libc_chk_name: &str = match name {
            "__builtin___memcpy_chk" => "__memcpy_chk",
            "__builtin___memmove_chk" => "__memmove_chk",
            "__builtin___memset_chk" => "__memset_chk",
            "__builtin___strcpy_chk" => "__strcpy_chk",
            "__builtin___strncpy_chk" => "__strncpy_chk",
            "__builtin___strcat_chk" => "__strcat_chk",
            "__builtin___strncat_chk" => "__strncat_chk",
            "__builtin___sprintf_chk" => "__sprintf_chk",
            "__builtin___snprintf_chk" => "__snprintf_chk",
            "__builtin___dprintf_chk" => "__dprintf_chk",
            "__builtin___vsprintf_chk" => "__vsprintf_chk",
            "__builtin___vsnprintf_chk" => "__vsnprintf_chk",
            "__builtin___printf_chk" => "__printf_chk",
            "__builtin___fprintf_chk" => "__fprintf_chk",
            "__builtin___vprintf_chk" => "__vprintf_chk",
            "__builtin___vfprintf_chk" => "__vfprintf_chk",
            "__builtin___mempcpy_chk" => "__mempcpy_chk",
            "__builtin___stpcpy_chk" => "__stpcpy_chk",
            "__builtin___stpncpy_chk" => "__stpncpy_chk",
            _ => {
                // Unknown _chk variant: lower all args for side effects, return 0
                for arg in args {
                    self.lower_expr(arg);
                }
                return Some(Operand::Const(IrConst::I64(0)));
            }
        };

        // Only the non-v* printf-family _chk builtins are truly variadic (they take ...).
        // The v* variants (vsprintf_chk, etc.) take a va_list argument instead, which is
        // a regular pointer parameter, not variadic.
        let is_variadic = matches!(
            name,
            "__builtin___sprintf_chk"
                | "__builtin___snprintf_chk"
                | "__builtin___dprintf_chk"
                | "__builtin___printf_chk"
                | "__builtin___fprintf_chk"
        );

        // Lower all arguments in order
        let mut arg_vals: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
        let mut arg_types: Vec<IrType> = args.iter().map(|a| self.get_expr_type(a)).collect();

        let dest = self.fresh_value();
        let return_type = self
            .builtin_return_type(name)
            .unwrap_or(crate::common::types::target_int_ir_type());
        // The fortified printf family has a fixed prefix (flag/size/format
        // operands) followed by `...`.  Everything past the prefix is a
        // variadic argument and must receive the default argument
        // promotions, exactly as for the unfortified builtin: glibc's
        // <stdio.h> rewrites `printf("%g", f)` into
        // `__builtin___printf_chk(1, "%g", f)` and a float passed as F32
        // there was read by glibc as a double.  The count also matters for
        // the SysV `%al` XMM-count and the i686 stack layout.
        let n_fixed = if is_variadic {
            builtin_variadic_fixed_arity(libc_chk_name).unwrap_or(arg_vals.len())
        } else {
            arg_vals.len()
        };
        if is_variadic {
            self.promote_variadic_args(&mut arg_vals, &mut arg_types, n_fixed);
        }
        let struct_arg_sizes = vec![None; arg_vals.len()];
        self.emit(Instruction::Call {
            func: libc_chk_name.to_string(),
            info: CallInfo {
                dest: Some(dest),
                args: arg_vals,
                arg_types,
                return_type,
                is_variadic,
                num_fixed_args: n_fixed,
                struct_arg_sizes,
                struct_arg_aligns: vec![],
                struct_arg_classes: Vec::new(),
                struct_arg_riscv_float_classes: Vec::new(),
                struct_arg_is_f128_sse: Vec::new(),
                is_sret: false,
                is_fastcall: false,
                is_pure: false,
                is_const: false,
                ret_eightbyte_classes: Vec::new(),
                ret_is_f128_sse: false,
            },
        });
        Some(Operand::Value(dest))
    }

    /// Apply the C11 6.5.2.2p6 default argument promotions to every argument at
    /// index >= `n_fixed` of a variadic call: `float` -> `double`,
    /// `char`/`short` (any signedness) -> `int`.  `arg_types` is rewritten in
    /// lock-step so the backend classifies the promoted value (F64 travels in
    /// an XMM register and is counted in `%al`; I32 is a GPR/stack slot).
    /// `_Bool` is already materialised as an I8 0/1 and widens to `int` here as
    /// well, which is what the C standard requires.
    pub(super) fn promote_variadic_args(
        &mut self,
        arg_vals: &mut [Operand],
        arg_types: &mut [IrType],
        n_fixed: usize,
    ) {
        for i in n_fixed..arg_vals.len() {
            let ty = arg_types[i];
            let promoted = match ty {
                IrType::F32 => IrType::F64,
                IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 => IrType::I32,
                _ => continue,
            };
            let val = std::mem::replace(&mut arg_vals[i], Operand::Const(IrConst::I64(0)));
            arg_vals[i] = self.emit_implicit_cast(val, ty, promoted);
            arg_types[i] = promoted;
        }
    }

    /// Lower __builtin_is{greater,less,unordered,...} float comparison builtins.
    /// Promotes operands to the widest float type, then emits the appropriate comparison.
    fn lower_fp_compare(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        if args.len() < 2 {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let lhs_ty = self.get_expr_type(&args[0]);
        let rhs_ty = self.get_expr_type(&args[1]);
        // Promote to the widest floating-point type among the two operands.
        let cmp_ty = if lhs_ty == IrType::F128 || rhs_ty == IrType::F128 {
            IrType::F128
        } else if lhs_ty == IrType::F64 || rhs_ty == IrType::F64 {
            IrType::F64
        } else if lhs_ty == IrType::F32 || rhs_ty == IrType::F32 {
            IrType::F32
        } else {
            IrType::F64
        };
        let mut lhs = self.lower_expr(&args[0]);
        let mut rhs = self.lower_expr(&args[1]);
        if lhs_ty != cmp_ty {
            let conv = self.emit_cast_val(lhs, lhs_ty, cmp_ty);
            lhs = Operand::Value(conv);
        }
        if rhs_ty != cmp_ty {
            let conv = self.emit_cast_val(rhs, rhs_ty, cmp_ty);
            rhs = Operand::Value(conv);
        }

        // __builtin_isunordered: returns 1 if either operand is NaN.
        // Emit (a != a) | (b != b) since NaN is the only value where x != x.
        if name == "__builtin_isunordered" {
            let lhs_nan = self.emit_cmp_val(IrCmpOp::Ne, lhs, lhs, cmp_ty);
            let rhs_nan = self.emit_cmp_val(IrCmpOp::Ne, rhs, rhs, cmp_ty);
            let dest = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(lhs_nan),
                Operand::Value(rhs_nan),
                IrType::I32,
            );
            return Some(Operand::Value(dest));
        }

        // __builtin_islessgreater: returns 1 if a < b or a > b (NOT for NaN).
        // Emit (a < b) | (a > b).
        if name == "__builtin_islessgreater" {
            let lt = self.emit_cmp_val(IrCmpOp::Slt, lhs, rhs, cmp_ty);
            let gt = self.emit_cmp_val(IrCmpOp::Sgt, lhs, rhs, cmp_ty);
            let dest = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(lt),
                Operand::Value(gt),
                IrType::I32,
            );
            return Some(Operand::Value(dest));
        }

        let cmp_op = match name {
            "__builtin_isgreater" => IrCmpOp::Sgt,
            "__builtin_isgreaterequal" => IrCmpOp::Sge,
            "__builtin_isless" => IrCmpOp::Slt,
            "__builtin_islessequal" => IrCmpOp::Sle,
            _ => IrCmpOp::Eq,
        };
        let dest = self.emit_cmp_val(cmp_op, lhs, rhs, cmp_ty);
        Some(Operand::Value(dest))
    }

    /// Lower creal/cimag builtins. `is_real` selects real vs imaginary part.
    fn lower_complex_part(&mut self, name: &str, args: &[Expr], is_real: bool) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::F64(0.0)));
        }
        let arg_ctype = self.expr_ctype(&args[0]);
        let target_ty = Self::creal_return_type(name);
        if arg_ctype.is_complex() {
            let val = if is_real {
                self.lower_complex_real_part(&args[0])
            } else {
                self.lower_complex_imag_part(&args[0])
            };
            let comp_ty = Self::complex_component_ir_type(&arg_ctype);
            if comp_ty != target_ty {
                Some(self.emit_implicit_cast(val, comp_ty, target_ty))
            } else {
                Some(val)
            }
        } else if is_real {
            let val = self.lower_expr(&args[0]);
            let val_ty = self.get_expr_type(&args[0]);
            Some(self.emit_implicit_cast(val, val_ty, target_ty))
        } else {
            // Imaginary part of a non-complex value is zero
            Some(match target_ty {
                IrType::F32 => Operand::Const(IrConst::F32(0.0)),
                IrType::F128 => Operand::Const(IrConst::long_double(0.0)),
                _ => Operand::Const(IrConst::F64(0.0)),
            })
        }
    }

    /// Whether it is safe to materialize an operand solely for a deferred
    /// IsConstant query. The builtin never evaluates its argument; lowering a
    /// call, increment, volatile/global load, or dereference would introduce an
    /// observable operation that the source program does not perform.
    fn constant_query_can_lower(&self, expr: &Expr) -> bool {
        match expr {
            Expr::IntLiteral(..)
            | Expr::UIntLiteral(..)
            | Expr::LongLiteral(..)
            | Expr::ULongLiteral(..)
            | Expr::LongLongLiteral(..)
            | Expr::ULongLongLiteral(..)
            | Expr::FloatLiteral(..)
            | Expr::FloatLiteralF32(..)
            | Expr::FloatLiteralLongDouble(..)
            | Expr::FloatLiteralF128(..)
            | Expr::CharLiteral(..)
            | Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..) => true,
            Expr::Identifier(name, _) => self
                .func()
                .locals
                .get(name)
                .is_some_and(|local| !local.is_array && !local.base_type_volatile),
            Expr::BinaryOp(_, lhs, rhs, _) | Expr::Comma(lhs, rhs, _) => {
                self.constant_query_can_lower(lhs) && self.constant_query_can_lower(rhs)
            }
            Expr::UnaryOp(op, inner, _) => {
                !matches!(op, UnaryOp::PreInc | UnaryOp::PreDec)
                    && self.constant_query_can_lower(inner)
            }
            Expr::Cast(_, inner, _) => self.constant_query_can_lower(inner),
            Expr::Conditional(cond, yes, no, _) => {
                self.constant_query_can_lower(cond)
                    && self.constant_query_can_lower(yes)
                    && self.constant_query_can_lower(no)
            }
            Expr::GnuConditional(cond, no, _) => {
                self.constant_query_can_lower(cond) && self.constant_query_can_lower(no)
            }
            // All remaining forms either perform work (call/assignment/va_arg),
            // access memory through an lvalue, or need statement/initializer
            // lowering. If not already proven constant above, GCC's answer is
            // conservatively zero and no IR for the argument may be emitted.
            _ => false,
        }
    }

    /// Lower __builtin_constant_p(expr): 1 if compile-time constant, 0 otherwise.
    /// Per GCC semantics, the argument is NOT evaluated (no side effects).
    fn lower_constant_p(&mut self, args: &[Expr]) -> Option<Operand> {
        let Some(arg) = args.first() else {
            return Some(Operand::Const(IrConst::I32(0)));
        };
        // If already a compile-time constant at lowering time, resolve immediately.
        // String literals are constant addresses even though numeric const-eval
        // intentionally has no IrConst representation for an address.
        let constant_literal_subscript = matches!(
            arg,
            Expr::ArraySubscript(base, index, _)
                if matches!(base.as_ref(),
                    Expr::StringLiteral(..)
                    | Expr::WideStringLiteral(..)
                    | Expr::Char16StringLiteral(..))
                && self.eval_const_expr(index).is_some()
        );
        if self.eval_const_expr(arg).is_some()
            || constant_literal_subscript
            || matches!(
                arg,
                Expr::StringLiteral(..)
                    | Expr::WideStringLiteral(..)
                    | Expr::Char16StringLiteral(..)
            )
        {
            return Some(Operand::Const(IrConst::I32(1)));
        }
        if !self.constant_query_can_lower(arg) {
            return Some(Operand::Const(IrConst::I32(0)));
        }
        // Defer in every function, not only inline candidates. Ordinary local
        // propagation is enough to make a value constant (`int n=sizeof(int)`),
        // and GCC reports that at -O1+ even in `main`. Eagerly returning zero
        // for non-inline functions permanently hid those opportunities.
        // The lowered value is used only as a constness query; later folding
        // resolves it to one after mem2reg/constant propagation, or the final
        // resolver turns it into zero.
        let src = self.lower_expr(arg);
        let src_ty = self.get_expr_type(arg);
        let dest = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest,
            op: IrUnaryOp::IsConstant,
            src,
            ty: src_ty,
        });
        Some(Operand::Value(dest))
    }

    /// Lower __builtin_complex(real, imag): construct a complex value on the stack.
    fn lower_complex_construct(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.len() < 2 {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let real_val = self.lower_expr(&args[0]);
        let imag_val = self.lower_expr(&args[1]);
        let arg_ty = self.get_expr_type(&args[0]);
        let (comp_ty, complex_size, comp_size) = if arg_ty == IrType::F32 {
            (IrType::F32, 8usize, 4usize)
        } else {
            (IrType::F64, 16usize, 8usize)
        };
        let alloca = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: alloca,
            ty: IrType::Ptr,
            size: complex_size,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val: real_val,
            ptr: alloca,
            ty: comp_ty,
            seg_override: AddressSpace::Default,
        });
        let imag_ptr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: imag_ptr,
            base: alloca,
            offset: Operand::Const(IrConst::I64(comp_size as i64)),
            ty: IrType::I8,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val: imag_val,
            ptr: imag_ptr,
            ty: comp_ty,
            seg_override: AddressSpace::Default,
        });
        Some(Operand::Value(alloca))
    }

    /// Lower a 128-bit vector ARGUMENT expression to a pointer at its
    /// 16 data bytes. Vector lvalues already lower to their alloca POINTER;
    /// a nested raw builtin (Vec128Value) yields a packed I128 VALUE, which
    /// must be spilled to a fresh slot first. Distinguishing via the C type
    /// (is_vector => pointer convention) is the load-bearing check: spilling
    /// a pointer, or passing an I128 through unspilled, both corrupt lanes.
    fn vec128_arg_ptr(&mut self, arg: &Expr) -> Operand {
        // By-value producers are exactly the raw 128-bit builtins (they have
        // no vector CType on purpose — see expr_types.rs). Everything else
        // with 16-byte vector shape lowers to an alloca POINTER.
        let is_by_value = matches!(arg,
            Expr::FunctionCall(callee, _, _)
                if matches!(&**callee, Expr::Identifier(n, _)
                    if matches!(n.as_str(),
                        "__builtin_ia32_maxps" | "__builtin_ia32_minps"
                        | "__builtin_ia32_shufps" | "__builtin_shufflevector"
                        | "__builtin_ia32_vextractf128_ps256")));
        let op = self.lower_expr(arg);
        if !is_by_value {
            return op; // alloca pointer convention
        }
        // By-value packed I128 (nested raw builtin): spill to a slot.
        let slot = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: slot,
            ty: IrType::Ptr,
            size: 16,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        });
        let val = self.operand_to_value(op);
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(val),
            ptr: slot,
            ty: IrType::I128,
            seg_override: AddressSpace::Default,
        });
        Operand::Value(slot)
    }

    /// Load the 16 bytes at `slot` and pack them into a by-value I128
    /// (lo | hi << 64) — the value representation every 16-byte vector
    /// expression uses in lccc.
    fn pack_vec128_from_slot(&mut self, slot: crate::ir::reexports::Value) -> Operand {
        let lo = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: lo,
            ptr: slot,
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
        });
        let hi_ptr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: hi_ptr,
            base: slot,
            offset: Operand::Const(IrConst::I64(8)),
            ty: IrType::I8,
        });
        let hi = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: hi,
            ptr: hi_ptr,
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
        });
        let lo128 = self.fresh_value();
        self.emit(Instruction::Cast {
            dest: lo128,
            src: Operand::Value(lo),
            from_ty: IrType::U64,
            to_ty: IrType::I128,
        });
        let hi128 = self.fresh_value();
        self.emit(Instruction::Cast {
            dest: hi128,
            src: Operand::Value(hi),
            from_ty: IrType::U64,
            to_ty: IrType::I128,
        });
        let shifted = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: shifted,
            op: IrBinOp::Shl,
            lhs: Operand::Value(hi128),
            rhs: Operand::Const(IrConst::I64(64)),
            ty: IrType::I128,
        });
        let packed = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: packed,
            op: IrBinOp::Or,
            lhs: Operand::Value(shifted),
            rhs: Operand::Value(lo128),
            ty: IrType::I128,
        });
        Operand::Value(packed)
    }

    /// __builtin_shufflevector(v1, v2, i0, i1, i2, i3) for 4-lane 32-bit
    /// vectors: gather each result lane from the concatenation v1||v2
    /// (idx 0-3 = v1 lane idx, 4-7 = v2 lane idx-4; negative = don\'t-care,
    /// we pick v1 lane 0). Indices must be integer constant expressions —
    /// matching the C builtin\'s contract. Scalar 4x32 gather through slots:
    /// correctness-first (a later peephole can fuse the shufps pattern).
    /// Lower __builtin_shuffle (GCC vector extension).
    ///
    /// result[i] = concat(v1, v2)[mask[i] mod N] where N is the element count
    /// of the first vector (2-arg form behaves as if v2 = v1). Mask elements
    /// are interpreted as unsigned; only the low log2(2N) bits select, which
    /// is exactly an unsigned remainder. The gather is fully unrolled with
    /// runtime indices: no constant-mask special case, so variable masks are
    /// equally correct. gcc.c-torture pr85331 (V2DF reverse) and pr94591
    /// (V2SI identity) drive the rules.
    fn lower_shuffle(&mut self, args: &[Expr]) -> Option<Operand> {
        let (vec_args, mask_expr): (&[Expr], &Expr) = match args.len() {
            2 => (&args[..1], &args[1]),
            3 => (&args[..2], &args[2]),
            n => panic!(
                "__builtin_shuffle: expected (v, mask) or (v1, v2, mask), got {} args",
                n
            ),
        };

        // Element geometry from the first vector's CType.
        let v0_ctype = self.expr_ctype(&vec_args[0]);
        let (elem_size, total_bytes) = match &v0_ctype {
            CType::Vector(elem, bytes) => (self.ctype_size(elem), *bytes),
            other => panic!(
                "__builtin_shuffle: first operand must be a vector type, got {:?}",
                other
            ),
        };
        if elem_size == 0 || total_bytes == 0 || total_bytes % elem_size != 0 {
            panic!(
                "__builtin_shuffle: degenerate vector geometry (elem={}, total={})",
                elem_size, total_bytes
            );
        }
        let n_elems: i64 = (total_bytes / elem_size) as i64;
        // Mask element width from the mask operand's CType; the mask must
        // have the same element count as the result.
        let mask_ctype = self.expr_ctype(mask_expr);
        let mask_elem_size = match &mask_ctype {
            CType::Vector(elem, bytes) => {
                let es = self.ctype_size(elem);
                if es == 0 || bytes % es != 0 || (bytes / es) as i64 != n_elems {
                    panic!(
                        "__builtin_shuffle: mask element count mismatch (mask {} elems, vector {} elems)",
                        if es > 0 { bytes / es } else { 0 },
                        n_elems
                    );
                }
                es
            }
            other => panic!(
                "__builtin_shuffle: mask must be a vector type, got {:?}",
                other
            ),
        };

        let ir_ty = match elem_size {
            1 => IrType::I8,
            2 => IrType::I16,
            4 => IrType::I32,
            8 => IrType::I64,
            16 => IrType::I128,
            other => panic!("__builtin_shuffle: unsupported element size {}", other),
        };
        let mask_ir_ty = match mask_elem_size {
            1 => IrType::U8,
            2 => IrType::U16,
            4 => IrType::U32,
            8 => IrType::U64,
            other => panic!("__builtin_shuffle: unsupported mask element size {}", other),
        };

        // Lower operands (vector-by-memory convention: alloca pointers).
        let mut src_vals: Vec<Value> = Vec::with_capacity(vec_args.len());
        for a in vec_args {
            let op = self.lower_expr(a);
            src_vals.push(self.operand_to_value(op));
        }
        let mask_op = self.lower_expr(mask_expr);
        let mask_ptr = self.operand_to_value(mask_op);

        let result = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: result,
            ty: IrType::Ptr,
            size: total_bytes,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        });

        // Unsigned remainder by mask element count: exact for every GCC
        // vector lane count; for power-of-two lane counts the backend
        // lowers URem against a constant to an AND (no division).
        for lane in 0..n_elems {
            let mptr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: mptr,
                base: mask_ptr,
                offset: Operand::Const(IrConst::I64(lane * mask_elem_size as i64)),
                ty: IrType::I8,
            });
            let mval = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: mval,
                ptr: mptr,
                ty: mask_ir_ty,
                seg_override: AddressSpace::Default,
            });
            // m = mask_elem % total_selectable (N for 2-arg, 2N for 3-arg)
            let selectable = n_elems * if vec_args.len() == 2 { 1 } else { 2 };
            let m_idx = if selectable > 0 && (selectable & (selectable - 1)) == 0 {
                let and_val = self.fresh_value();
                self.emit(Instruction::BinOp {
                    dest: and_val,
                    op: IrBinOp::And,
                    lhs: Operand::Value(mval),
                    rhs: Operand::Const(IrConst::I64(selectable - 1)),
                    ty: mask_ir_ty,
                });
                and_val
            } else {
                let rem_val = self.fresh_value();
                self.emit(Instruction::BinOp {
                    dest: rem_val,
                    op: IrBinOp::URem,
                    lhs: Operand::Value(mval),
                    rhs: Operand::Const(IrConst::I64(selectable)),
                    ty: mask_ir_ty,
                });
                rem_val
            };
            // 3-arg form: base = m < N ? v1 : v2; offset within base = m mod N.
            let (base, elem_idx) = if vec_args.len() == 3 {
                let in_first = self.fresh_value();
                self.emit(Instruction::Cmp {
                    dest: in_first,
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(m_idx),
                    rhs: Operand::Const(IrConst::I64(n_elems)),
                    ty: mask_ir_ty,
                });
                let idx_in_vec = if n_elems > 0 && (n_elems & (n_elems - 1)) == 0 {
                    let a = self.fresh_value();
                    self.emit(Instruction::BinOp {
                        dest: a,
                        op: IrBinOp::And,
                        lhs: Operand::Value(m_idx),
                        rhs: Operand::Const(IrConst::I64(n_elems - 1)),
                        ty: mask_ir_ty,
                    });
                    a
                } else {
                    let r = self.fresh_value();
                    self.emit(Instruction::BinOp {
                        dest: r,
                        op: IrBinOp::URem,
                        lhs: Operand::Value(m_idx),
                        rhs: Operand::Const(IrConst::I64(n_elems)),
                        ty: mask_ir_ty,
                    });
                    r
                };
                let sel = self.fresh_value();
                self.emit(Instruction::Select {
                    dest: sel,
                    cond: Operand::Value(in_first),
                    true_val: Operand::Value(src_vals[0]),
                    false_val: Operand::Value(src_vals[1]),
                    ty: IrType::Ptr,
                });
                (sel, idx_in_vec)
            } else {
                (src_vals[0], m_idx)
            };
            // Widening the (possibly narrow) index to I64 for the address math.
            let idx64 = self.fresh_value();
            self.emit(Instruction::Cast {
                dest: idx64,
                src: Operand::Value(elem_idx),
                from_ty: mask_ir_ty,
                to_ty: IrType::I64,
            });
            let off_bytes = self.fresh_value();
            self.emit(Instruction::BinOp {
                dest: off_bytes,
                op: IrBinOp::Mul,
                lhs: Operand::Value(idx64),
                rhs: Operand::Const(IrConst::I64(elem_size as i64)),
                ty: IrType::I64,
            });
            let sptr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: sptr,
                base,
                offset: Operand::Value(off_bytes),
                ty: IrType::I8,
            });
            let lane_val = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: lane_val,
                ptr: sptr,
                ty: ir_ty,
                seg_override: AddressSpace::Default,
            });
            let dptr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: dptr,
                base: result,
                offset: Operand::Const(IrConst::I64(lane * elem_size as i64)),
                ty: IrType::I8,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(lane_val),
                ptr: dptr,
                ty: ir_ty,
                seg_override: AddressSpace::Default,
            });
        }
        // Return convention (must match lower_vector_assign /
        // rhs_is_small_vector_call): vectors <= 8 bytes travel as packed
        // register values (the consumer spills them back to memory);
        // vectors > 8 bytes travel by memory — the result alloca pointer.
        if total_bytes <= 8 {
            let packed_ty = Self::packed_store_type(total_bytes);
            let packed = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: packed,
                ptr: result,
                ty: packed_ty,
                seg_override: AddressSpace::Default,
            });
            Some(Operand::Value(packed))
        } else {
            Some(Operand::Value(result))
        }
    }

    fn lower_shufflevector(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.len() != 6 {
            panic!("__builtin_shufflevector: only the 4-lane 32-bit form (2 vectors + 4 indices) is supported, got {} args", args.len());
        }
        let v1 = self.vec128_arg_ptr(&args[0]);
        let v2 = self.vec128_arg_ptr(&args[1]);
        let v1p = self.operand_to_value(v1);
        let v2p = self.operand_to_value(v2);
        let result = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: result,
            ty: IrType::Ptr,
            size: 16,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        });
        for lane in 0..4usize {
            let idx = match self.eval_const_expr(&args[2 + lane]) {
                Some(IrConst::I64(v)) => v,
                Some(IrConst::I32(v)) => v as i64,
                Some(IrConst::I8(v)) => v as i64,
                Some(IrConst::I16(v)) => v as i64,
                other => panic!(
                    "__builtin_shufflevector: lane index {} is not an integer constant ({:?})",
                    lane, other
                ),
            };
            let (src, off) = if idx < 0 {
                (v1p, 0i64)
            } else if idx < 4 {
                (v1p, idx * 4)
            } else if idx < 8 {
                (v2p, (idx - 4) * 4)
            } else {
                panic!(
                    "__builtin_shufflevector: lane index {} out of range for 4-lane vectors",
                    idx
                );
            };
            let src_ptr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: src_ptr,
                base: src,
                offset: Operand::Const(IrConst::I64(off)),
                ty: IrType::I8,
            });
            let lane_val = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: lane_val,
                ptr: src_ptr,
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            });
            let dst_ptr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: dst_ptr,
                base: result,
                offset: Operand::Const(IrConst::I64((lane * 4) as i64)),
                ty: IrType::I8,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(lane_val),
                ptr: dst_ptr,
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            });
        }
        // Raw builtin: return the vector BY VALUE (packed I128).
        Some(self.pack_vec128_from_slot(result))
    }

    /// Lower an X86 SSE/AES/CRC intrinsic to IR.
    ///
    /// Classifies the intrinsic into one of four emission patterns:
    /// - Fence: no args, no dest (lfence, mfence, sfence, pause)
    /// - PtrStore: first arg is dest pointer, remaining args are data (movnti, storedqu, etc.)
    /// - Vec128: allocates 16-byte result slot, returns pointer (SSE/AES packed ops)
    /// - Scalar: returns a scalar value in a dest register (pmovmskb, crc32, pextrw, etc.)
    fn lower_x86_intrinsic(
        &mut self,
        intrinsic: &BuiltinIntrinsic,
        args: &[Expr],
    ) -> Option<Operand> {
        let op = x86_intrinsic_op(intrinsic);
        match x86_intrinsic_kind(intrinsic) {
            X86IntrinsicKind::Fence => {
                self.emit(Instruction::Intrinsic {
                    dest: None,
                    op,
                    dest_ptr: None,
                    args: vec![],
                });
                Some(Operand::Const(IrConst::I64(0)))
            }
            X86IntrinsicKind::VoidArgs => {
                let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(Instruction::Intrinsic {
                    dest: None,
                    op,
                    dest_ptr: None,
                    args: arg_ops,
                });
                Some(Operand::Const(IrConst::I64(0)))
            }
            X86IntrinsicKind::PtrStore => {
                // Defensive: never panic the compiler on a short argument list.
                if args.len() < 2 {
                    for a in args {
                        let _ = self.lower_expr(a);
                    }
                    return Some(Operand::Const(IrConst::I64(0)));
                }
                let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let ptr_val = self.operand_to_value(arg_ops[0].clone());
                self.emit(Instruction::Intrinsic {
                    dest: None,
                    op,
                    dest_ptr: Some(ptr_val),
                    args: vec![arg_ops[1].clone()],
                });
                Some(Operand::Const(IrConst::I64(0)))
            }
            X86IntrinsicKind::Vec128 | X86IntrinsicKind::Vec256 => {
                let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let result_size = match x86_intrinsic_kind(intrinsic) {
                    X86IntrinsicKind::Vec128 => 16,
                    X86IntrinsicKind::Vec256 => 32,
                    _ => unreachable!(),
                };
                let result_alloca = self.fresh_value();
                let align = match result_size {
                    32 => 32usize,
                    _ => 16usize,
                };
                self.emit(Instruction::Alloca {
                    dest: result_alloca,
                    ty: IrType::Ptr,
                    size: result_size,
                    align,
                    volatile: false,
                    semantic_volatile: false,
                });
                let dest_val = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(dest_val),
                    op,
                    dest_ptr: Some(result_alloca),
                    args: arg_ops,
                });
                Some(Operand::Value(result_alloca))
            }
            X86IntrinsicKind::Vec128Value => {
                // GCC's __builtin_ia32_* return the vector BY VALUE — unlike
                // the _mm_* wrappers, whose bundled-header definition routes
                // through __lccc_simd* and dereferences the returned pointer
                // itself (__CCC_M128_FROM_BUILTIN = *(__m128*)(e)). A caller
                // of the raw builtin has no such deref: handing back the
                // alloca POINTER made the function return `lea slot(%rsp)`
                // + cqto — an address and sign-extension garbage instead of
                // the 16 data bytes (kernel NAP governor's v4sf clamp
                // produced -3e+34). Materialize the I128 value: two I64
                // halves loaded from the result slot, widened and packed —
                // the same representation ordinary v4sf loads lower to.
                // Vector args go through vec128_arg_ptr: vector lvalues are
                // already alloca pointers, nested raw-builtin results are
                // packed I128 values that must be spilled to a slot first.
                // Integer args (the imm8 of shufps/vextractf128) pass through.
                let arg_ops: Vec<Operand> = args
                    .iter()
                    .map(|a| {
                        let is_int = self
                            .get_expr_ctype(a)
                            .map(|ct| ct.is_integer())
                            .unwrap_or(false);
                        if is_int {
                            self.lower_expr(a)
                        } else {
                            self.vec128_arg_ptr(a)
                        }
                    })
                    .collect();

                let result_alloca = self.fresh_value();
                self.emit(Instruction::Alloca {
                    dest: result_alloca,
                    ty: IrType::Ptr,
                    size: 16,
                    align: 16,
                    volatile: false,
                    semantic_volatile: false,
                });
                let dest_val = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(dest_val),
                    op,
                    dest_ptr: Some(result_alloca),
                    args: arg_ops,
                });
                // lo half
                let lo = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: lo,
                    ptr: result_alloca,
                    ty: IrType::I64,
                    seg_override: crate::common::types::AddressSpace::Default,
                });
                // hi half: result_alloca + 8 (GEP so analyses see a proper derived pointer)
                let hi_ptr = self.fresh_value();
                self.emit(Instruction::GetElementPtr {
                    dest: hi_ptr,
                    base: result_alloca,
                    offset: Operand::Const(IrConst::I64(8)),
                    ty: IrType::I8,
                });
                let hi = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: hi,
                    ptr: hi_ptr,
                    ty: IrType::I64,
                    seg_override: crate::common::types::AddressSpace::Default,
                });
                // ILP32 targets have no I128 value representation (the i686
                // backend's I128 arithmetic is a libgcc-helper shape for
                // __int128, not a general 128-bit scalar), so the x86-64
                // pack-into-I128 trick does not port. There the aggregate
                // convention IS pointer-based: hand back the result alloca
                // pointer and let the caller copy the 16 bytes like any other
                // aggregate expression.
                if self.target == crate::backend::Target::I686 {
                    return Some(Operand::Value(result_alloca));
                }
                let lo128 = self.fresh_value();
                self.emit(Instruction::Cast {
                    dest: lo128,
                    src: Operand::Value(lo),
                    from_ty: IrType::U64,
                    to_ty: IrType::I128,
                });
                let hi128 = self.fresh_value();
                self.emit(Instruction::Cast {
                    dest: hi128,
                    src: Operand::Value(hi),
                    from_ty: IrType::U64,
                    to_ty: IrType::I128,
                });
                let shifted = self.fresh_value();
                self.emit(Instruction::BinOp {
                    dest: shifted,
                    op: IrBinOp::Shl,
                    lhs: Operand::Value(hi128),
                    rhs: Operand::Const(IrConst::I64(64)),
                    ty: IrType::I128,
                });
                let packed = self.fresh_value();
                self.emit(Instruction::BinOp {
                    dest: packed,
                    op: IrBinOp::Or,
                    lhs: Operand::Value(shifted),
                    rhs: Operand::Value(lo128),
                    ty: IrType::I128,
                });
                Some(Operand::Value(packed))
            }
            X86IntrinsicKind::Scalar => {
                let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let dest_val = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(dest_val),
                    op,
                    dest_ptr: None,
                    args: arg_ops,
                });
                Some(Operand::Value(dest_val))
            }
            X86IntrinsicKind::CastReinterpret => {
                // Free 128-bit reinterpret cast (_mm_castsi128_ps etc.): the
                // value is a pointer to 16 bytes; the cast just re-labels it.
                if let Some(a) = args.first() {
                    Some(self.lower_expr(a))
                } else {
                    Some(Operand::Const(IrConst::I64(0)))
                }
            }
        }
    }
}

/// Emission pattern for X86 SSE/AES/CRC intrinsics.
enum X86IntrinsicKind {
    /// No arguments, no destination (fence/barrier ops).
    Fence,
    /// Arguments passed through, no destination (clflush).
    VoidArgs,
    /// First arg is dest pointer, second is data (non-temporal stores).
    PtrStore,
    /// 128-bit vector result allocated on stack, returns pointer.
    Vec128,
    /// 128-bit vector returned BY VALUE as packed I128 (raw GCC
    /// __builtin_ia32_* semantics — no header wrapper dereferences it).
    Vec128Value,
    /// 256-bit vector result allocated on stack.
    Vec256,
    /// Scalar result in a dest register.
    Scalar,
    /// Free reinterpret cast: returns the (single) argument unchanged.
    CastReinterpret,
}

/// Map a BuiltinIntrinsic to its emission pattern.
/// Number of fixed (non-variadic) parameters of the variadic libc / glibc
/// fortification functions reachable through `__builtin_*`.  This is the
/// builtin's own contract and is consulted when the translation unit never
/// declared the libc symbol (or declared it without a prototype), so the
/// trailing arguments still receive the default argument promotions and the
/// SysV XMM count in `%al` is correct.
fn builtin_variadic_fixed_arity(libc_name: &str) -> Option<usize> {
    Some(match libc_name {
        "printf" => 1,
        "fprintf" | "sprintf" | "dprintf" => 2,
        "snprintf" => 3,
        // __printf_chk(flag, fmt, ...)
        "__printf_chk" => 2,
        // __fprintf_chk(fp, flag, fmt, ...)
        "__fprintf_chk" => 3,
        // __sprintf_chk(s, flag, slen, fmt, ...)
        "__sprintf_chk" => 4,
        // __snprintf_chk(s, maxlen, flag, slen, fmt, ...)
        "__snprintf_chk" => 5,
        // __dprintf_chk(fd, flag, fmt, ...)
        "__dprintf_chk" => 3,
        _ => return None,
    })
}

fn x86_intrinsic_kind(intrinsic: &BuiltinIntrinsic) -> X86IntrinsicKind {
    match intrinsic {
        BuiltinIntrinsic::X86Lfence
        | BuiltinIntrinsic::X86Mfence
        | BuiltinIntrinsic::X86Sfence
        | BuiltinIntrinsic::X86Pause
        | BuiltinIntrinsic::X86Vzeroupper => X86IntrinsicKind::Fence,

        BuiltinIntrinsic::X86Clflush => X86IntrinsicKind::VoidArgs,

        BuiltinIntrinsic::X86Movnti
        | BuiltinIntrinsic::X86Movnti64
        | BuiltinIntrinsic::X86Movntdq
        | BuiltinIntrinsic::X86Movntpd
        | BuiltinIntrinsic::X86Storedqu
        | BuiltinIntrinsic::X86Storeldi128
        | BuiltinIntrinsic::X86Storeu256
        | BuiltinIntrinsic::X86Store256
        | BuiltinIntrinsic::X86StoreuPs256
        | BuiltinIntrinsic::X86StoreuPd256 => X86IntrinsicKind::PtrStore,

        BuiltinIntrinsic::X86Pmovmskb128
        | BuiltinIntrinsic::X86Pextrw128
        | BuiltinIntrinsic::X86Cvtsi128Si32
        | BuiltinIntrinsic::X86Cvtsi128Si64
        | BuiltinIntrinsic::X86Crc32_8
        | BuiltinIntrinsic::X86Crc32_16
        | BuiltinIntrinsic::X86Crc32_32
        | BuiltinIntrinsic::X86Crc32_64
        | BuiltinIntrinsic::X86Pextrd128
        | BuiltinIntrinsic::X86Pextrb128
        | BuiltinIntrinsic::X86Pextrq128
        | BuiltinIntrinsic::X86Pmovmskb256
        | BuiltinIntrinsic::X86Rdtsc
        | BuiltinIntrinsic::X86Rdtscp
        | BuiltinIntrinsic::X86Testz128 => X86IntrinsicKind::Scalar,

        BuiltinIntrinsic::X86Loadu256
        | BuiltinIntrinsic::X86Load256
        | BuiltinIntrinsic::X86Paddb256
        | BuiltinIntrinsic::X86Paddw256
        | BuiltinIntrinsic::X86Paddd256
        | BuiltinIntrinsic::X86Psubb256
        | BuiltinIntrinsic::X86Psubw256
        | BuiltinIntrinsic::X86Psubusw256
        | BuiltinIntrinsic::X86Psadbw256
        | BuiltinIntrinsic::X86Pmaddubsw256
        | BuiltinIntrinsic::X86Pmaddwd256
        | BuiltinIntrinsic::X86Pcmpeqb256
        | BuiltinIntrinsic::X86Pcmpgtb256
        | BuiltinIntrinsic::X86Pshufb256
        | BuiltinIntrinsic::X86Pabsb256
        | BuiltinIntrinsic::X86Pabsw256
        | BuiltinIntrinsic::X86Pmaxub256
        | BuiltinIntrinsic::X86Pminub256
        | BuiltinIntrinsic::X86Pxor256
        | BuiltinIntrinsic::X86Por256
        | BuiltinIntrinsic::X86Pand256
        | BuiltinIntrinsic::X86Psllidi256
        | BuiltinIntrinsic::X86Psrlidi256
        | BuiltinIntrinsic::X86Psllwi256
        | BuiltinIntrinsic::X86Psrlwi256
        | BuiltinIntrinsic::X86Broadcast128to256
        | BuiltinIntrinsic::X86Zext128to256
        | BuiltinIntrinsic::X86Insert128to256
        | BuiltinIntrinsic::X86SetEpi8_256
        | BuiltinIntrinsic::X86SetEpi16_256
        | BuiltinIntrinsic::X86SetEpi32_256
        | BuiltinIntrinsic::X86SetEpi64x256
        | BuiltinIntrinsic::X86Permutevar8x32
        | BuiltinIntrinsic::X86Dpbusd256
        | BuiltinIntrinsic::X86Dpbusds256
        | BuiltinIntrinsic::X86Dpwusd256
        | BuiltinIntrinsic::X86Dpwusds256
        | BuiltinIntrinsic::X86Dpbssd256
        | BuiltinIntrinsic::X86Dpbssds256
        | BuiltinIntrinsic::X86Dpbsud256
        | BuiltinIntrinsic::X86Dpbsuds256
        | BuiltinIntrinsic::X86Dpbuud256
        | BuiltinIntrinsic::X86Dpbuuds256
        | BuiltinIntrinsic::X86Dpwuud256
        | BuiltinIntrinsic::X86Dpwuuds256
        | BuiltinIntrinsic::X86Dpwssd256
        | BuiltinIntrinsic::X86Dpwssds256
        | BuiltinIntrinsic::X86Aesenc256
        | BuiltinIntrinsic::X86Aesenclast256
        | BuiltinIntrinsic::X86Aesdec256
        | BuiltinIntrinsic::X86Aesdeclast256
        | BuiltinIntrinsic::X86Vpclmulqdq256
        | BuiltinIntrinsic::X86Pmulld256
        | BuiltinIntrinsic::X86Psubd256
        | BuiltinIntrinsic::X86Paddq256
        | BuiltinIntrinsic::X86Psubq256
        | BuiltinIntrinsic::X86Pandn256
        | BuiltinIntrinsic::X86Pcmpeqd256
        | BuiltinIntrinsic::X86Pcmpeqq256
        | BuiltinIntrinsic::X86Pcmpgtd256
        | BuiltinIntrinsic::X86Pcmpgtq256
        | BuiltinIntrinsic::X86Setzero256
        | BuiltinIntrinsic::X86AddPs256
        | BuiltinIntrinsic::X86SubPs256
        | BuiltinIntrinsic::X86MulPs256
        | BuiltinIntrinsic::X86AddPd256
        | BuiltinIntrinsic::X86SubPd256
        | BuiltinIntrinsic::X86MulPd256
        | BuiltinIntrinsic::X86LoaduPs256
        | BuiltinIntrinsic::X86LoaduPd256
        | BuiltinIntrinsic::X86Permute2x128
        | BuiltinIntrinsic::X86Permute4x64
        | BuiltinIntrinsic::X86Pshufd256
        | BuiltinIntrinsic::X86Punpcklbw256
        | BuiltinIntrinsic::X86Punpckhbw256
        | BuiltinIntrinsic::X86Punpcklwd256
        | BuiltinIntrinsic::X86Punpckhwd256
        | BuiltinIntrinsic::X86Punpckldq256
        | BuiltinIntrinsic::X86Punpckhdq256
        | BuiltinIntrinsic::X86Punpcklqdq256
        | BuiltinIntrinsic::X86Punpckhqdq256
        | BuiltinIntrinsic::X86Pslldqi256
        | BuiltinIntrinsic::X86Psrldqi256
        | BuiltinIntrinsic::X86Psllqi256
        | BuiltinIntrinsic::X86Psrlqi256
        | BuiltinIntrinsic::X86Pmullw256
        | BuiltinIntrinsic::X86Pmulhw256
        | BuiltinIntrinsic::X86Pminsd256
        | BuiltinIntrinsic::X86Pmaxsd256
        | BuiltinIntrinsic::X86Pmovzxbw256
        | BuiltinIntrinsic::X86Pmovzxbd256
        | BuiltinIntrinsic::X86Pmovzxwd256
        | BuiltinIntrinsic::X86Pmovsxbw256
        | BuiltinIntrinsic::X86Pmovsxbd256
        | BuiltinIntrinsic::X86Pmovsxwd256
        | BuiltinIntrinsic::X86Psrawi256
        | BuiltinIntrinsic::X86Psradi256
        | BuiltinIntrinsic::X86Packssdw256
        | BuiltinIntrinsic::X86Packuswb256
        | BuiltinIntrinsic::X86Phaddw256
        | BuiltinIntrinsic::X86Phaddd256
        | BuiltinIntrinsic::X86Pabsd256
        | BuiltinIntrinsic::X86Pmuludq256 => X86IntrinsicKind::Vec256,
        BuiltinIntrinsic::X86CastReinterpret => X86IntrinsicKind::CastReinterpret,
        // Raw GCC vector builtins (no _mm_ wrapper): value semantics.
        BuiltinIntrinsic::X86MaxPs128 | BuiltinIntrinsic::X86MinPs128 => {
            X86IntrinsicKind::Vec128Value
        }
        // __builtin_ia32_shufps returns BY VALUE (raw GCC builtin, no wrapper
        // deref); the imm8 rides as the last argument, which is exactly the
        // (a, b, imm) shape the ShufPs128 backend op consumes.
        BuiltinIntrinsic::X86ShufPsValue => X86IntrinsicKind::Vec128Value,
        // __builtin_ia32_cvtpd2ps returns v4sf BY VALUE (raw GCC builtin).
        BuiltinIntrinsic::X86CvtPd2PsValue => X86IntrinsicKind::Vec128Value,
        // vextractf128_ps256 returns a 16-byte vector BY VALUE from a ymm arg.
        BuiltinIntrinsic::X86Vextractf128V => X86IntrinsicKind::Vec128Value,
        // 256-bit raw builtins: v8sf is pointer/sret convention everywhere,
        // so the Vec256 pointer-returning arm is already value-correct.
        BuiltinIntrinsic::X86MaxPs256V
        | BuiltinIntrinsic::X86MinPs256V
        | BuiltinIntrinsic::X86AndPs256V
        | BuiltinIntrinsic::X86CmpPs256V => X86IntrinsicKind::Vec256,
        _ => X86IntrinsicKind::Vec128,
    }
}

/// Map a BuiltinIntrinsic to its corresponding IntrinsicOp.
fn x86_intrinsic_op(intrinsic: &BuiltinIntrinsic) -> IntrinsicOp {
    match intrinsic {
        BuiltinIntrinsic::X86Rdtsc => IntrinsicOp::Rdtsc,
        BuiltinIntrinsic::X86Rdtscp => IntrinsicOp::Rdtscp,
        BuiltinIntrinsic::X86Lfence => IntrinsicOp::Lfence,
        BuiltinIntrinsic::X86Mfence => IntrinsicOp::Mfence,
        BuiltinIntrinsic::X86Sfence => IntrinsicOp::Sfence,
        BuiltinIntrinsic::X86Pause => IntrinsicOp::Pause,
        BuiltinIntrinsic::X86Vzeroupper => IntrinsicOp::Vzeroupper,
        BuiltinIntrinsic::X86ShufPsValue => IntrinsicOp::ShufPs128,
        BuiltinIntrinsic::X86CvtPd2PsValue => IntrinsicOp::CvtPd2Ps128,
        BuiltinIntrinsic::X86Vextractf128V => IntrinsicOp::Vextractf128,
        BuiltinIntrinsic::X86MaxPs256V => IntrinsicOp::MaxPs256,
        BuiltinIntrinsic::X86MinPs256V => IntrinsicOp::MinPs256,
        // andps is pure bitwise: pand256 is bit-identical on any lane type.
        BuiltinIntrinsic::X86AndPs256V => IntrinsicOp::Pand256,
        BuiltinIntrinsic::X86CmpPs256V => IntrinsicOp::CmpPs256,
        BuiltinIntrinsic::X86Clflush => IntrinsicOp::Clflush,
        BuiltinIntrinsic::X86Movnti => IntrinsicOp::Movnti,
        BuiltinIntrinsic::X86Movnti64 => IntrinsicOp::Movnti64,
        BuiltinIntrinsic::X86Movntdq => IntrinsicOp::Movntdq,
        BuiltinIntrinsic::X86Movntpd => IntrinsicOp::Movntpd,
        BuiltinIntrinsic::X86Loaddqu => IntrinsicOp::Loaddqu,
        BuiltinIntrinsic::X86Pcmpeqb128 => IntrinsicOp::Pcmpeqb128,
        BuiltinIntrinsic::X86Pcmpeqd128 => IntrinsicOp::Pcmpeqd128,
        BuiltinIntrinsic::X86Psubusb128 => IntrinsicOp::Psubusb128,
        BuiltinIntrinsic::X86Psubsb128 => IntrinsicOp::Psubsb128,
        BuiltinIntrinsic::X86Por128 => IntrinsicOp::Por128,
        BuiltinIntrinsic::X86Pand128 => IntrinsicOp::Pand128,
        BuiltinIntrinsic::X86Pxor128 => IntrinsicOp::Pxor128,
        // Float SSE ops map to the equivalent bitwise/arithmetic 128-bit ops.
        // xorps/andps/orps are pure bitwise: pxor/pand/por is identical.
        // addps/subps/mulps are true float ops.
        BuiltinIntrinsic::X86XorPs => IntrinsicOp::Pxor128,
        BuiltinIntrinsic::X86AndPs => IntrinsicOp::Pand128,
        BuiltinIntrinsic::X86OrPs => IntrinsicOp::Por128,
        BuiltinIntrinsic::X86AddPs => IntrinsicOp::AddPs128,
        BuiltinIntrinsic::X86SubPs => IntrinsicOp::SubPs128,
        BuiltinIntrinsic::X86MulPs => IntrinsicOp::MulPs128,
        BuiltinIntrinsic::X86AddPd => IntrinsicOp::AddPd128,
        BuiltinIntrinsic::X86SubPd => IntrinsicOp::SubPd128,
        BuiltinIntrinsic::X86MulPd => IntrinsicOp::MulPd128,
        BuiltinIntrinsic::X86MulEpu32 => IntrinsicOp::Pmuludq128,
        BuiltinIntrinsic::X86MulEpi32 => IntrinsicOp::Pmuldq128,
        BuiltinIntrinsic::X86MulloEpi32 => IntrinsicOp::Pmulld128,
        BuiltinIntrinsic::X86CastReinterpret => IntrinsicOp::CastReinterpret128,
        BuiltinIntrinsic::X86Set1Epi8 => IntrinsicOp::SetEpi8,
        BuiltinIntrinsic::X86Set1Epi32 => IntrinsicOp::SetEpi32,
        BuiltinIntrinsic::X86Aesenc128 => IntrinsicOp::Aesenc128,
        BuiltinIntrinsic::X86Aesenclast128 => IntrinsicOp::Aesenclast128,
        BuiltinIntrinsic::X86Aesdec128 => IntrinsicOp::Aesdec128,
        BuiltinIntrinsic::X86Aesdeclast128 => IntrinsicOp::Aesdeclast128,
        BuiltinIntrinsic::X86Aesimc128 => IntrinsicOp::Aesimc128,
        BuiltinIntrinsic::X86Aeskeygenassist128 => IntrinsicOp::Aeskeygenassist128,
        BuiltinIntrinsic::X86Pclmulqdq128 => IntrinsicOp::Pclmulqdq128,
        BuiltinIntrinsic::X86Pslldqi128 => IntrinsicOp::Pslldqi128,
        BuiltinIntrinsic::X86Psrldqi128 => IntrinsicOp::Psrldqi128,
        BuiltinIntrinsic::X86Psllqi128 => IntrinsicOp::Psllqi128,
        BuiltinIntrinsic::X86Psrlqi128 => IntrinsicOp::Psrlqi128,
        BuiltinIntrinsic::X86Pshufd128 => IntrinsicOp::Pshufd128,
        BuiltinIntrinsic::X86Loadldi128 => IntrinsicOp::Loadldi128,
        BuiltinIntrinsic::X86Paddw128 => IntrinsicOp::Paddw128,
        BuiltinIntrinsic::X86Psubw128 => IntrinsicOp::Psubw128,
        BuiltinIntrinsic::X86Pmulhw128 => IntrinsicOp::Pmulhw128,
        BuiltinIntrinsic::X86Pmaddwd128 => IntrinsicOp::Pmaddwd128,
        BuiltinIntrinsic::X86Pcmpgtw128 => IntrinsicOp::Pcmpgtw128,
        BuiltinIntrinsic::X86Pcmpgtb128 => IntrinsicOp::Pcmpgtb128,
        BuiltinIntrinsic::X86Psllwi128 => IntrinsicOp::Psllwi128,
        BuiltinIntrinsic::X86Psrlwi128 => IntrinsicOp::Psrlwi128,
        BuiltinIntrinsic::X86Psrawi128 => IntrinsicOp::Psrawi128,
        BuiltinIntrinsic::X86Psradi128 => IntrinsicOp::Psradi128,
        BuiltinIntrinsic::X86Pslldi128 => IntrinsicOp::Pslldi128,
        BuiltinIntrinsic::X86Psrldi128 => IntrinsicOp::Psrldi128,
        BuiltinIntrinsic::X86Paddd128 => IntrinsicOp::Paddd128,
        BuiltinIntrinsic::X86Psubd128 => IntrinsicOp::Psubd128,
        BuiltinIntrinsic::X86Packssdw128 => IntrinsicOp::Packssdw128,
        BuiltinIntrinsic::X86Packsswb128 => IntrinsicOp::Packsswb128,
        BuiltinIntrinsic::X86Packuswb128 => IntrinsicOp::Packuswb128,
        BuiltinIntrinsic::X86Punpcklbw128 => IntrinsicOp::Punpcklbw128,
        BuiltinIntrinsic::X86Punpckhbw128 => IntrinsicOp::Punpckhbw128,
        BuiltinIntrinsic::X86Punpcklwd128 => IntrinsicOp::Punpcklwd128,
        BuiltinIntrinsic::X86Punpckhwd128 => IntrinsicOp::Punpckhwd128,
        BuiltinIntrinsic::X86Set1Epi16 => IntrinsicOp::SetEpi16,
        BuiltinIntrinsic::X86Pinsrw128 => IntrinsicOp::Pinsrw128,
        BuiltinIntrinsic::X86Cvtsi32Si128 => IntrinsicOp::Cvtsi32Si128,
        BuiltinIntrinsic::X86Pshuflw128 => IntrinsicOp::Pshuflw128,
        BuiltinIntrinsic::X86Pshufhw128 => IntrinsicOp::Pshufhw128,
        BuiltinIntrinsic::X86Storedqu => IntrinsicOp::Storedqu,
        BuiltinIntrinsic::X86Storeldi128 => IntrinsicOp::Storeldi128,
        BuiltinIntrinsic::X86Pmovmskb128 => IntrinsicOp::Pmovmskb128,
        BuiltinIntrinsic::X86Pextrw128 => IntrinsicOp::Pextrw128,
        BuiltinIntrinsic::X86Cvtsi128Si32 => IntrinsicOp::Cvtsi128Si32,
        BuiltinIntrinsic::X86Cvtsi128Si64 => IntrinsicOp::Cvtsi128Si64,
        BuiltinIntrinsic::X86Crc32_8 => IntrinsicOp::Crc32_8,
        BuiltinIntrinsic::X86Crc32_16 => IntrinsicOp::Crc32_16,
        BuiltinIntrinsic::X86Crc32_32 => IntrinsicOp::Crc32_32,
        BuiltinIntrinsic::X86Crc32_64 => IntrinsicOp::Crc32_64,
        // SSE4.1 insert/extract
        BuiltinIntrinsic::X86Pinsrd128 => IntrinsicOp::Pinsrd128,
        BuiltinIntrinsic::X86Pextrd128 => IntrinsicOp::Pextrd128,
        BuiltinIntrinsic::X86Pinsrb128 => IntrinsicOp::Pinsrb128,
        BuiltinIntrinsic::X86Pextrb128 => IntrinsicOp::Pextrb128,
        BuiltinIntrinsic::X86Pinsrq128 => IntrinsicOp::Pinsrq128,
        BuiltinIntrinsic::X86Pextrq128 => IntrinsicOp::Pextrq128,
        BuiltinIntrinsic::X86Paddb128 => IntrinsicOp::Paddb128,
        BuiltinIntrinsic::X86Psubb128 => IntrinsicOp::Psubb128,
        BuiltinIntrinsic::X86Psubusw128 => IntrinsicOp::Psubusw128,
        BuiltinIntrinsic::X86Psadbw128 => IntrinsicOp::Psadbw128,
        BuiltinIntrinsic::X86Pmullw128 => IntrinsicOp::Pmullw128,
        BuiltinIntrinsic::X86Pmaddubsw128 => IntrinsicOp::Pmaddubsw128,
        BuiltinIntrinsic::X86Phaddw128 => IntrinsicOp::Phaddw128,
        BuiltinIntrinsic::X86Phaddd128 => IntrinsicOp::Phaddd128,
        BuiltinIntrinsic::X86Pshufb128 => IntrinsicOp::Pshufb128,
        BuiltinIntrinsic::X86Pabsb128 => IntrinsicOp::Pabsb128,
        BuiltinIntrinsic::X86Pabsw128 => IntrinsicOp::Pabsw128,
        BuiltinIntrinsic::X86Pabsd128 => IntrinsicOp::Pabsd128,
        BuiltinIntrinsic::X86Palignr128 => IntrinsicOp::Palignr128,
        BuiltinIntrinsic::X86Pmaxub128 => IntrinsicOp::Pmaxub128,
        BuiltinIntrinsic::X86Pminub128 => IntrinsicOp::Pminub128,
        BuiltinIntrinsic::X86MaxPs128 => IntrinsicOp::MaxPs128,
        BuiltinIntrinsic::X86MinPs128 => IntrinsicOp::MinPs128,
        BuiltinIntrinsic::X86Pblendvb128 => IntrinsicOp::Pblendvb128,
        BuiltinIntrinsic::X86Pblendw128 => IntrinsicOp::Pblendw128,
        BuiltinIntrinsic::X86Pmovzxbw128 => IntrinsicOp::Pmovzxbw128,
        BuiltinIntrinsic::X86Pmovzxwd128 => IntrinsicOp::Pmovzxwd128,
        BuiltinIntrinsic::X86Psllw128 => IntrinsicOp::Psllw128,
        BuiltinIntrinsic::X86Psrlw128 => IntrinsicOp::Psrlw128,
        BuiltinIntrinsic::X86Loadu256 => IntrinsicOp::Loadu256,
        BuiltinIntrinsic::X86Storeu256 => IntrinsicOp::Storeu256,
        BuiltinIntrinsic::X86Load256 => IntrinsicOp::Load256,
        BuiltinIntrinsic::X86Store256 => IntrinsicOp::Store256,
        BuiltinIntrinsic::X86Paddb256 => IntrinsicOp::Paddb256,
        BuiltinIntrinsic::X86Paddw256 => IntrinsicOp::Paddw256,
        BuiltinIntrinsic::X86Paddd256 => IntrinsicOp::Paddd256,
        BuiltinIntrinsic::X86Psubb256 => IntrinsicOp::Psubb256,
        BuiltinIntrinsic::X86Psubw256 => IntrinsicOp::Psubw256,
        BuiltinIntrinsic::X86Psubusw256 => IntrinsicOp::Psubusw256,
        BuiltinIntrinsic::X86Psadbw256 => IntrinsicOp::Psadbw256,
        BuiltinIntrinsic::X86Pmaddubsw256 => IntrinsicOp::Pmaddubsw256,
        BuiltinIntrinsic::X86Pmaddwd256 => IntrinsicOp::Pmaddwd256,
        BuiltinIntrinsic::X86Pcmpeqb256 => IntrinsicOp::Pcmpeqb256,
        BuiltinIntrinsic::X86Pcmpgtb256 => IntrinsicOp::Pcmpgtb256,
        BuiltinIntrinsic::X86Pmovmskb256 => IntrinsicOp::Pmovmskb256,
        BuiltinIntrinsic::X86Pshufb256 => IntrinsicOp::Pshufb256,
        BuiltinIntrinsic::X86Pabsb256 => IntrinsicOp::Pabsb256,
        BuiltinIntrinsic::X86Pabsw256 => IntrinsicOp::Pabsw256,
        BuiltinIntrinsic::X86Pmaxub256 => IntrinsicOp::Pmaxub256,
        BuiltinIntrinsic::X86Pminub256 => IntrinsicOp::Pminub256,
        BuiltinIntrinsic::X86Pxor256 => IntrinsicOp::Pxor256,
        BuiltinIntrinsic::X86Por256 => IntrinsicOp::Por256,
        BuiltinIntrinsic::X86Pand256 => IntrinsicOp::Pand256,
        BuiltinIntrinsic::X86Psllidi256 => IntrinsicOp::Psllidi256,
        BuiltinIntrinsic::X86Psrlidi256 => IntrinsicOp::Psrlidi256,
        BuiltinIntrinsic::X86Psllwi256 => IntrinsicOp::Psllwi256,
        BuiltinIntrinsic::X86Psrlwi256 => IntrinsicOp::Psrlwi256,
        BuiltinIntrinsic::X86Broadcast128to256 => IntrinsicOp::Broadcast128to256,
        BuiltinIntrinsic::X86Zext128to256 => IntrinsicOp::Zext128to256,
        BuiltinIntrinsic::X86Cast256to128 => IntrinsicOp::Cast256to128,
        BuiltinIntrinsic::X86Insert128to256 => IntrinsicOp::Insert128to256,
        BuiltinIntrinsic::X86SetEpi8_256 => IntrinsicOp::SetEpi8_256,
        BuiltinIntrinsic::X86SetEpi16_256 => IntrinsicOp::SetEpi16_256,
        BuiltinIntrinsic::X86Dpbusd128 => IntrinsicOp::Dpbusd128,
        BuiltinIntrinsic::X86Dpbusds128 => IntrinsicOp::Dpbusds128,
        BuiltinIntrinsic::X86Dpwusd128 => IntrinsicOp::Dpwusd128,
        BuiltinIntrinsic::X86Dpwusds128 => IntrinsicOp::Dpwusds128,
        BuiltinIntrinsic::X86Dpbusd256 => IntrinsicOp::Dpbusd256,
        BuiltinIntrinsic::X86Dpbusds256 => IntrinsicOp::Dpbusds256,
        BuiltinIntrinsic::X86Dpwusd256 => IntrinsicOp::Dpwusd256,
        BuiltinIntrinsic::X86Dpwusds256 => IntrinsicOp::Dpwusds256,
        BuiltinIntrinsic::X86Dpbssd128 => IntrinsicOp::Dpbssd128,
        BuiltinIntrinsic::X86Dpbssds128 => IntrinsicOp::Dpbssds128,
        BuiltinIntrinsic::X86Dpbsud128 => IntrinsicOp::Dpbsud128,
        BuiltinIntrinsic::X86Dpbsuds128 => IntrinsicOp::Dpbsuds128,
        BuiltinIntrinsic::X86Dpbuud128 => IntrinsicOp::Dpbuud128,
        BuiltinIntrinsic::X86Dpbuuds128 => IntrinsicOp::Dpbuuds128,
        BuiltinIntrinsic::X86Dpbssd256 => IntrinsicOp::Dpbssd256,
        BuiltinIntrinsic::X86Dpbssds256 => IntrinsicOp::Dpbssds256,
        BuiltinIntrinsic::X86Dpbsud256 => IntrinsicOp::Dpbsud256,
        BuiltinIntrinsic::X86Dpbsuds256 => IntrinsicOp::Dpbsuds256,
        BuiltinIntrinsic::X86Dpbuud256 => IntrinsicOp::Dpbuud256,
        BuiltinIntrinsic::X86Dpbuuds256 => IntrinsicOp::Dpbuuds256,
        BuiltinIntrinsic::X86Dpwuud128 => IntrinsicOp::Dpwuud128,
        BuiltinIntrinsic::X86Dpwuuds128 => IntrinsicOp::Dpwuuds128,
        BuiltinIntrinsic::X86Dpwssd128 => IntrinsicOp::Dpwssd128,
        BuiltinIntrinsic::X86Dpwssds128 => IntrinsicOp::Dpwssds128,
        BuiltinIntrinsic::X86Dpwuud256 => IntrinsicOp::Dpwuud256,
        BuiltinIntrinsic::X86Dpwuuds256 => IntrinsicOp::Dpwuuds256,
        BuiltinIntrinsic::X86Dpwssd256 => IntrinsicOp::Dpwssd256,
        BuiltinIntrinsic::X86Dpwssds256 => IntrinsicOp::Dpwssds256,
        BuiltinIntrinsic::X86Gf2p8mulb128 => IntrinsicOp::Gf2p8mulb128,
        BuiltinIntrinsic::X86Gf2p8affineqb128 => IntrinsicOp::Gf2p8affineqb128,
        BuiltinIntrinsic::X86Gf2p8affineinvqb128 => IntrinsicOp::Gf2p8affineinvqb128,
        BuiltinIntrinsic::X86Aesenc256 => IntrinsicOp::Aesenc256,
        BuiltinIntrinsic::X86Aesenclast256 => IntrinsicOp::Aesenclast256,
        BuiltinIntrinsic::X86Aesdec256 => IntrinsicOp::Aesdec256,
        BuiltinIntrinsic::X86Aesdeclast256 => IntrinsicOp::Aesdeclast256,
        BuiltinIntrinsic::X86Vpclmulqdq256 => IntrinsicOp::Vpclmulqdq256,
        BuiltinIntrinsic::X86SetEpi32_256 => IntrinsicOp::SetEpi32_256,
        BuiltinIntrinsic::X86SetEpi64x256 => IntrinsicOp::SetEpi64x256,
        BuiltinIntrinsic::X86Paddusb128 => IntrinsicOp::Paddusb128,
        BuiltinIntrinsic::X86Paddsb128 => IntrinsicOp::Paddsb128,
        BuiltinIntrinsic::X86Paddusw128 => IntrinsicOp::Paddusw128,
        BuiltinIntrinsic::X86Paddsw128 => IntrinsicOp::Paddsw128,
        BuiltinIntrinsic::X86Psubsw128 => IntrinsicOp::Psubsw128,
        BuiltinIntrinsic::X86Pandn128 => IntrinsicOp::Pandn128,
        BuiltinIntrinsic::X86Pcmpeqw128 => IntrinsicOp::Pcmpeqw128,
        BuiltinIntrinsic::X86Pcmpgtd128 => IntrinsicOp::Pcmpgtd128,
        BuiltinIntrinsic::X86Pavgb128 => IntrinsicOp::Pavgb128,
        BuiltinIntrinsic::X86Pavgw128 => IntrinsicOp::Pavgw128,
        BuiltinIntrinsic::X86Pminsw128 => IntrinsicOp::Pminsw128,
        BuiltinIntrinsic::X86Pmaxsw128 => IntrinsicOp::Pmaxsw128,
        BuiltinIntrinsic::X86Pmulhuw128 => IntrinsicOp::Pmulhuw128,
        BuiltinIntrinsic::X86Paddq128 => IntrinsicOp::Paddq128,
        BuiltinIntrinsic::X86Psubq128 => IntrinsicOp::Psubq128,
        BuiltinIntrinsic::X86Punpckldq128 => IntrinsicOp::Punpckldq128,
        BuiltinIntrinsic::X86Punpckhdq128 => IntrinsicOp::Punpckhdq128,
        BuiltinIntrinsic::X86Punpcklqdq128 => IntrinsicOp::Punpcklqdq128,
        BuiltinIntrinsic::X86Punpckhqdq128 => IntrinsicOp::Punpckhqdq128,
        BuiltinIntrinsic::X86Setzero128 => IntrinsicOp::Setzero128,
        BuiltinIntrinsic::X86Testz128 => IntrinsicOp::Testz128,
        BuiltinIntrinsic::X86Pmulld256 => IntrinsicOp::Pmulld256,
        BuiltinIntrinsic::X86Psubd256 => IntrinsicOp::Psubd256,
        BuiltinIntrinsic::X86Paddq256 => IntrinsicOp::Paddq256,
        BuiltinIntrinsic::X86Psubq256 => IntrinsicOp::Psubq256,
        BuiltinIntrinsic::X86Pandn256 => IntrinsicOp::Pandn256,
        BuiltinIntrinsic::X86Pcmpeqd256 => IntrinsicOp::Pcmpeqd256,
        BuiltinIntrinsic::X86Pcmpeqq256 => IntrinsicOp::Pcmpeqq256,
        BuiltinIntrinsic::X86Pcmpgtd256 => IntrinsicOp::Pcmpgtd256,
        BuiltinIntrinsic::X86Pcmpgtq256 => IntrinsicOp::Pcmpgtq256,
        BuiltinIntrinsic::X86Extracti128 => IntrinsicOp::Extracti128,
        BuiltinIntrinsic::X86Setzero256 => IntrinsicOp::Setzero256,
        BuiltinIntrinsic::X86AddPs256 => IntrinsicOp::AddPs256,
        BuiltinIntrinsic::X86SubPs256 => IntrinsicOp::SubPs256,
        BuiltinIntrinsic::X86MulPs256 => IntrinsicOp::MulPs256,
        BuiltinIntrinsic::X86AddPd256 => IntrinsicOp::AddPd256,
        BuiltinIntrinsic::X86SubPd256 => IntrinsicOp::SubPd256,
        BuiltinIntrinsic::X86MulPd256 => IntrinsicOp::MulPd256,
        BuiltinIntrinsic::X86LoaduPs256 => IntrinsicOp::LoaduPs256,
        BuiltinIntrinsic::X86StoreuPs256 => IntrinsicOp::StoreuPs256,
        BuiltinIntrinsic::X86LoaduPd256 => IntrinsicOp::LoaduPd256,
        BuiltinIntrinsic::X86StoreuPd256 => IntrinsicOp::StoreuPd256,
        BuiltinIntrinsic::X86Permute2x128 => IntrinsicOp::Permute2x128,
        BuiltinIntrinsic::X86Permute4x64 => IntrinsicOp::Permute4x64,
        BuiltinIntrinsic::X86Permutevar8x32 => IntrinsicOp::Permutevar8x32,
        BuiltinIntrinsic::X86Pshufd256 => IntrinsicOp::Pshufd256,
        BuiltinIntrinsic::X86Punpcklbw256 => IntrinsicOp::Punpcklbw256,
        BuiltinIntrinsic::X86Punpckhbw256 => IntrinsicOp::Punpckhbw256,
        BuiltinIntrinsic::X86Punpcklwd256 => IntrinsicOp::Punpcklwd256,
        BuiltinIntrinsic::X86Punpckhwd256 => IntrinsicOp::Punpckhwd256,
        BuiltinIntrinsic::X86Punpckldq256 => IntrinsicOp::Punpckldq256,
        BuiltinIntrinsic::X86Punpckhdq256 => IntrinsicOp::Punpckhdq256,
        BuiltinIntrinsic::X86Punpcklqdq256 => IntrinsicOp::Punpcklqdq256,
        BuiltinIntrinsic::X86Punpckhqdq256 => IntrinsicOp::Punpckhqdq256,
        BuiltinIntrinsic::X86Pslldqi256 => IntrinsicOp::Pslldqi256,
        BuiltinIntrinsic::X86Psrldqi256 => IntrinsicOp::Psrldqi256,
        BuiltinIntrinsic::X86Psllqi256 => IntrinsicOp::Psllqi256,
        BuiltinIntrinsic::X86Psrlqi256 => IntrinsicOp::Psrlqi256,
        BuiltinIntrinsic::X86Pmullw256 => IntrinsicOp::Pmullw256,
        BuiltinIntrinsic::X86Pmulhw256 => IntrinsicOp::Pmulhw256,
        BuiltinIntrinsic::X86Pminsd256 => IntrinsicOp::Pminsd256,
        BuiltinIntrinsic::X86Pmaxsd256 => IntrinsicOp::Pmaxsd256,
        BuiltinIntrinsic::X86Pmovzxbw256 => IntrinsicOp::Pmovzxbw256,
        BuiltinIntrinsic::X86Pmovzxbd256 => IntrinsicOp::Pmovzxbd256,
        BuiltinIntrinsic::X86Pmovzxwd256 => IntrinsicOp::Pmovzxwd256,
        BuiltinIntrinsic::X86Pmovsxbw256 => IntrinsicOp::Pmovsxbw256,
        BuiltinIntrinsic::X86Pmovsxbd256 => IntrinsicOp::Pmovsxbd256,
        BuiltinIntrinsic::X86Pmovsxwd256 => IntrinsicOp::Pmovsxwd256,
        BuiltinIntrinsic::X86Psrawi256 => IntrinsicOp::Psrawi256,
        BuiltinIntrinsic::X86Psradi256 => IntrinsicOp::Psradi256,
        BuiltinIntrinsic::X86Packssdw256 => IntrinsicOp::Packssdw256,
        BuiltinIntrinsic::X86Packuswb256 => IntrinsicOp::Packuswb256,
        BuiltinIntrinsic::X86Phaddw256 => IntrinsicOp::Phaddw256,
        BuiltinIntrinsic::X86Phaddd256 => IntrinsicOp::Phaddd256,
        BuiltinIntrinsic::X86Pabsd256 => IntrinsicOp::Pabsd256,
        BuiltinIntrinsic::X86Pmuludq256 => IntrinsicOp::Pmuludq256,
        _ => unreachable!(
            "x86_intrinsic_op called with non-X86 intrinsic: {:?}",
            intrinsic
        ),
    }
}

/// Classify a CType according to GCC's __builtin_classify_type conventions.
/// Returns the GCC type class integer:
///   0 = no_type_class (void)
///   1 = integer_type_class
///   2 = char_type_class
///   3 = enumeral_type_class
///   4 = boolean_type_class
///   5 = pointer_type_class
///   6 = reference_type_class (C++ only, not used in C)
///   8 = real_type_class
///   9 = complex_type_class
///  12 = record_type_class (struct)
///  13 = union_type_class
///  14 = array_type_class
///  16 = lang_type_class
fn classify_ctype(ty: &CType) -> i64 {
    // GCC __builtin_classify_type returns integer type classes.
    // GCC treats char, _Bool, and enum as integer_type_class (1) in practice.
    match ty {
        CType::Void => 0, // no_type_class
        CType::Bool
        | CType::Char
        | CType::UChar
        | CType::Short
        | CType::UShort
        | CType::Int
        | CType::UInt
        | CType::Long
        | CType::ULong
        | CType::LongLong
        | CType::ULongLong
        | CType::Int128
        | CType::UInt128
        | CType::Enum(_) => 1, // integer_type_class
        CType::Float | CType::Double | CType::LongDouble | CType::Float128 => 8, // real_type_class
        CType::Decimal32 | CType::Decimal64 | CType::Decimal128 => 8, // real_type_class (decimal FP)
        CType::ComplexFloat | CType::ComplexDouble | CType::ComplexLongDouble => 9, // complex_type_class
        CType::Pointer(_, _) => 5, // pointer_type_class
        CType::Array(_, _) => 5,   // GCC decays arrays to pointers
        CType::Function(_) => 5,   // function decays to pointer
        CType::Struct(_) => 12,    // record_type_class
        CType::Union(_) => 13,     // union_type_class
        CType::Vector(_, _) => 14, // array_type_class (GCC classifies vectors here)
    }
}
