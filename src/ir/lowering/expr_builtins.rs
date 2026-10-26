//! Builtin function call lowering: __builtin_* dispatch and X86 SSE intrinsics.
//!
//! This module handles the top-level dispatch for all __builtin_* functions,
//! complex number intrinsics, and X86 SSE/CRC intrinsics. Extracted helpers
//! live in sibling modules:
//! - expr_builtins_intrin.rs: integer bit-manipulation (clz, ctz, bswap, etc.)
//! - expr_builtins_overflow.rs: overflow-checking arithmetic
//! - expr_builtins_fpclass.rs: FP classification (fpclassify, isnan, isinf, etc.)

use super::lower::Lowerer;
use crate::common::const_eval as shared_const_eval;
use crate::common::source::Span;
use crate::common::types::{AddressSpace, CType, IrType, target_is_32bit};
use crate::frontend::parser::ast::{Expr, TypeSpecifier, UnaryOp};
use crate::frontend::sema::builtins::{self, BuiltinIntrinsic, BuiltinKind};
use crate::ir::reexports::{
    BasicBlock, CallInfo, Instruction, IntrinsicOp, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand,
    PrefetchHint, Terminator, Value,
};

/// How `__builtin_cpu_supports("f")` is answered (AUD-CPU-1).
///
/// The answer is a runtime CPUID bit test (see `crate::common::cpu_probe`) for
/// every name this compiler models on x86, and the constant 0 on targets that
/// cannot run x86 code.  There is deliberately no "the target guarantees it, so
/// fold to 1" case: measured against the competition (GCC 16.2, Clang 23.1, ICX
/// latest, `-O2 -march=x86-64` and `-march=x86-64-v3`/`raptorlake`), all three
/// compile `!!__builtin_cpu_supports("sse2")` to a load from `__cpu_model`
/// *even when the effective target enables the feature* — GCC probes `sse2`
/// with `-mavxvnni` enabled and probes `avxvnni` under `-march=raptorlake`
/// while itself emitting AVX-VNNI instructions elsewhere.  Their contract (and
/// GCC's documentation: "returns a positive integer if the running CPU
/// supports the feature") is about the CPU that *executes* the program, so
/// folding a guaranteed feature to 1 would answer 1 on a machine that cannot
/// execute it — the one answer the reference compilers never give.
enum CpuSupport {
    /// The answer is a runtime CPUID bit test (see `crate::common::cpu_probe`).
    Runtime(crate::common::cpu_probe::Probe),
    /// No runtime CPU property exists for this target (AArch64/RISC-V): the
    /// conservative 0 is the only sound answer, and — unlike an unknown x86
    /// name — it is not diagnosed, because these targets have their own
    /// feature-name space GCC accepts that LCCC does not model yet.
    Unsupported,
}

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
        // -ffreestanding/-fno-builtin disables hosted-library aliases such as
        // abs() and creal(). Explicit compiler builtins, atomics, SIMD header
        // interfaces, and alloca remain compiler interfaces.
        let compiler_builtin = name.starts_with("__builtin_")
            || name.starts_with("__atomic_")
            || name.starts_with("__sync_")
            || name.starts_with("__lccc_")
            || name.starts_with("_mm_")
            || name.starts_with("_mm256_")
            || name.starts_with("_mm512_")
            || name == "alloca";
        if !compiler_builtin && !self.builtin_policy.may_assume(name) {
            return None;
        }

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

        // __builtin_choose_expr(c, a, b) is `a` or `b` itself, chosen at
        // compile time (`resolve_builtin_choose_expr`).
        if name == "__builtin_choose_expr" {
            if args.len() >= 3 {
                let chosen = self.resolve_builtin_choose_expr(args);
                return Some(self.lower_expr(chosen));
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

        // __builtin_prefetch(addr, [rw], [locality]) - architectural
        // prefetch hint.
        //
        // GCC 16 semantics (`expand_builtin_prefetch`): `rw` and `locality`
        // must be integer constant expressions (an error otherwise), `rw` in
        // 0..=2 (read, write, instruction fetch) and `locality` in 0..=3; an
        // out-of-range value is a warning and is replaced by ZERO. They
        // default to 0 and 3. This lowering is the only place that sees the
        // builtin's arguments (sema registers it as a plain intrinsic), so
        // it owns those diagnostics — see `prefetch_hint`.
        //
        // The hint is only a hint: dropping it (non-constant arguments, or a
        // backend without prefetch support) can never change program
        // semantics, and the instruction can never fault — every address is
        // architecturally legal for a prefetch.
        if name == "__builtin_prefetch" {
            let addr = args.first().map(|arg| self.lower_expr(arg));
            let rw = args.get(1).map(|e| self.prefetch_hint_arg(e));
            let locality = args.get(2).map(|e| self.prefetch_hint_arg(e));
            let (hint, diags) = prefetch_hint(rw, locality);
            for (diag, idx) in diags {
                let span = args[idx].span();
                match diag {
                    PrefetchDiag::NotConstant(msg) => self.emit_error(msg, span),
                    PrefetchDiag::OutOfRange(msg) => self.emit_warning(msg, span),
                }
            }
            let hint = hint.encode();
            // Evaluate any arguments beyond the first for their side effects
            // (GCC's frontend rejects >3 arguments, but lowering must stay
            // total).
            let extra: Vec<Operand> = args.iter().skip(1).map(|a| self.lower_expr(a)).collect();
            let _ = extra;
            if let Some(addr) = addr {
                self.emit(Instruction::Intrinsic {
                    dest: None,
                    op: IntrinsicOp::Prefetch(hint),
                    dest_ptr: None,
                    args: vec![addr],
                });
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
        // A NaN builtin whose payload is not a string literal cannot fold: it
        // is a call to the library function (GCC: `__builtin_nan(s)` calls
        // `nan(s)`), not the canonical NaN.
        let nan_call;
        let kind = match (&builtin_info.kind, nan_library_function(name)) {
            (BuiltinKind::ConstantF64(_) | BuiltinKind::ConstantF128(_), Some(lib))
                if args
                    .first()
                    .is_some_and(|a| !matches!(a, Expr::StringLiteral(..))) =>
            {
                nan_call = BuiltinKind::LibcAlias(lib.to_string());
                &nan_call
            }
            (kind, _) => kind,
        };
        match kind {
            BuiltinKind::LibcAlias(libc_name) => {
                // Source types must describe the values `lower_expr` really
                // produces (see the long note in `expr_calls.rs`):
                // `get_expr_type` reports the storage-level width and lies
                // about `i + 1`, which is an `int`.
                let mut arg_types: Vec<IrType> =
                    args.iter().map(|a| self.value_ir_type(a)).collect();
                let mut arg_vals: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                // A `__builtin_*` call carries the libc function's signature
                // whether or not the TU declared the libc symbol, so the
                // parameter conversions are part of the builtin's contract.
                // Without them `__builtin_memcmp(p, q, i + 1)` passed a raw
                // 32-bit `int` where `size_t` was expected: at -O0 the 8-byte
                // argument load read the neighbouring 4-byte slot as the high
                // half (gcc.c-torture `pr59229.c`), and a negative length was
                // zero-extended instead of sign-extended.
                if let Some(params) = builtins::libc_alias_param_types(libc_name.as_str()) {
                    for (i, &param_ty) in params.iter().enumerate() {
                        if i >= arg_vals.len() {
                            break;
                        }
                        let from = arg_types[i];
                        if from == param_ty
                            || from == IrType::Ptr
                            || param_ty == IrType::Ptr
                            || !from.is_integer()
                            || !param_ty.is_integer()
                        {
                            continue;
                        }
                        arg_vals[i] = self.emit_implicit_cast(arg_vals[i].clone(), from, param_ty);
                        arg_types[i] = param_ty;
                    }
                }
                // -----------------------------------------------------------------
                // Constant-size `__builtin_memcpy` / `__builtin_mempcpy`.  The
                // libc forwarding below would emit a real `call memcpy`, but a copy
                // of compile-time-constant size is better modelled by the native IR
                // `Memcpy` instruction: the backend inlines small copies, and the
                // SROA / store-load-forwarding passes can then forward a load out of
                // the destination (read64's `__builtin_memcpy(&v, p, 8); return v`
                // collapses to a single unaligned load) instead of leaving a
                // store-to-temp + reload in every hot loop that does an unaligned
                // 64-bit read.  This fires ONLY for the *builtin* spelling, which by
                // construction names the libc function; a plain `memcpy(...)` call
                // (which could be a user-provided function of that name) stays a
                // call, preserving exact user semantics.  `memmove` is never
                // rewritten because its overlap contract is different.
                //
                // SOUNDNESS for the native Memcpy: C `memcpy` already has the
                // no-overlap precondition (overlap is UB), which is exactly the
                // contract `Instruction::Memcpy` assumes, so lowering a valid
                // `memcpy` to it is value-preserving.  The result of `memcpy` is the
                // destination pointer, so on success we yield that operand directly.
                if libc_name == "memcpy" && arg_vals.len() == 3 {
                    // The size argument is promoted to `size_t` (U64) by the
                    // libc-alias cast above, so even a literal `8` lowers to a
                    // ZExt/Cast *value*, not a `Const`.  Const-evaluate the
                    // source expression directly rather than inspecting the
                    // lowered operand.
                    let size = match self.eval_const_expr(&args[2]) {
                        Some(IrConst::I64(v)) if v > 0 => v as usize,
                        Some(IrConst::I32(v)) if v > 0 => v as usize,
                        Some(IrConst::I128(v)) if v > 0 && v <= usize::MAX as i128 => v as usize,
                        _ => 0,
                    };
                    if size > 0 {
                        if let (Operand::Value(dp), Operand::Value(sp)) =
                            (&arg_vals[0], &arg_vals[1])
                        {
                            let dp = *dp;
                            let sp = *sp;
                            // A whole-register vector copy (`memcpy(&v, p,
                            // sizeof v)` / `memcpy(p, &v, sizeof v)`) is a
                            // packed load + store the vector passes see
                            // through (`vec_ssa` dissolves the temporary).
                            if self.try_emit_vector_memcpy(&args[0], &args[1], dp, sp, size) {
                                return Some(Operand::Value(dp));
                            }
                            self.emit(Instruction::Memcpy {
                                dest: dp,
                                src: sp,
                                size,
                            });
                            // `__builtin_memcpy` returns its destination.
                            return Some(Operand::Value(dp));
                        }
                    }
                }
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
                        regparm: None,
                        is_pure: false,
                        is_const: false,
                        // Spelled `__builtin_<name>`: the library function by definition.
                        assume_library: true,
                        ret_eightbyte_classes: Vec::new(),
                        ret_is_f128_sse: false,
                        cold: false,
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
                // calls/returns/memory all speak I128). The shared folder
                // owns NaN payloads (`__builtin_nanf128("7")`).
                if let Some(c) = shared_const_eval::eval_builtin_call(name, args, &|_| None) {
                    return Some(Operand::Const(c));
                }
                let bits = u128::from_be_bytes(*bytes) as i128;
                Some(Operand::Const(IrConst::I128(bits)))
            }
            BuiltinKind::ConstantF64(val) => {
                // NaN payloads, signaling NaNs and long double NaNs in the
                // target format: one implementation, shared with the
                // static-initializer folder.
                if let Some(c) = shared_const_eval::eval_builtin_call(name, args, &|_| None) {
                    return Some(Operand::Const(c));
                }
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

    // --- Builtin-call diagnostics (AUD-ICE-1 / AUD-DIAG-1) -------------------
    //
    // Ill-typed builtin calls used to `panic!` ("internal error: ...") out of
    // the lowering of *invalid user code*.  A user typo must never look like a
    // compiler crash: every reachable panic becomes a diagnostic with a source
    // span, and the call lowers to a shape-correct placeholder so the rest of
    // lowering stays total (the driver aborts the compile on the emitted
    // error, before any pass or codegen sees the module).

    /// Span to report for a builtin call: the first argument, or a dummy when
    /// the call has no arguments (GCC points at the builtin's own token; the
    /// argument is the closest span this lowering stage has).
    fn builtin_arg_span(args: &[Expr]) -> Span {
        args.first().map(|a| a.span()).unwrap_or_else(Span::dummy)
    }

    /// Placeholder result for a builtin call that was rejected with a
    /// diagnostic.  Shaped like the vector-by-memory return convention so the
    /// discarded IR stays well-formed.
    fn rejected_vector_operand(&mut self, bytes: usize) -> Operand {
        let result = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: result,
            ty: IrType::Ptr,
            size: bytes,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        });
        Operand::Value(result)
    }

    /// Preserve the 16-byte source / 32-byte destination contract across
    /// public struct wrappers and GCC native-vector builtin expressions.
    fn lower_avx_widen(
        &mut self,
        op: crate::common::avx_widen::AvxWiden,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        self.check_x86_intrinsic_feature(&BuiltinIntrinsic::X86AvxWiden(op), name, args);
        if args.len() != 1 {
            return self.reject_builtin_call(format!("'{}' requires 1 argument", name), args, 32);
        }
        if !matches!(
            self.target,
            crate::backend::Target::X86_64 | crate::backend::Target::I686
        ) {
            return self.reject_builtin_call(
                format!("'{}' is not available on {}", name, self.target.triple()),
                args,
                32,
            );
        }
        if name.starts_with("__builtin_ia32_") {
            let valid = self.get_expr_ctype(&args[0]).is_some_and(|ctype| {
                ctype.vector_info().is_some_and(|(element, lanes)| {
                    element.is_integer()
                        && !matches!(element, CType::Bool)
                        && element.size() * 8 == op.source_bits()
                        && lanes == 128 / op.source_bits()
                })
            });
            if !valid {
                return self.reject_builtin_call(
                    format!(
                        "'{}' requires a 128-bit vector of {}-bit integers",
                        name,
                        op.source_bits()
                    ),
                    args,
                    32,
                );
            }
        }
        let input = self.vec128_arg_ptr(&args[0]);
        let result = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: result,
            ty: IrType::Ptr,
            size: 32,
            align: 32,
            volatile: false,
            semantic_volatile: false,
        });
        let dest = self.fresh_value();
        self.emit(Instruction::Intrinsic {
            dest: Some(dest),
            op: IntrinsicOp::AvxWiden(op),
            dest_ptr: Some(result),
            args: vec![input],
        });
        Some(Operand::Value(result))
    }

    /// VTEST returns scalar flags from native vector operands. In particular,
    /// it must not be lowered to PTEST, which observes every payload bit.
    fn lower_avx_test(
        &mut self,
        test: crate::common::avx_test::AvxTest,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        self.check_x86_intrinsic_feature(&BuiltinIntrinsic::X86AvxTest(test), name, args);
        if args.len() != 2 {
            return self.reject_builtin_call(
                format!("'{}' requires 2 arguments", name),
                args,
                test.width(),
            );
        }
        if !matches!(
            self.target,
            crate::backend::Target::X86_64 | crate::backend::Target::I686
        ) {
            return self.reject_builtin_call(
                format!("'{}' is not available on {}", name, self.target.triple()),
                args,
                test.width(),
            );
        }
        if name.starts_with("__builtin_ia32_") {
            let expected = if test.double {
                CType::Double
            } else {
                CType::Float
            };
            let valid = args.iter().all(|arg| {
                self.get_expr_ctype(arg).is_some_and(|ctype| {
                    ctype.vector_info().is_some_and(|(element, lanes)| {
                        *element == expected && lanes * element.size() == test.width()
                    })
                })
            });
            if !valid {
                return self.reject_builtin_call(
                    format!(
                        "'{}' requires two {}-bit floating-point vectors",
                        name,
                        test.width() * 8
                    ),
                    args,
                    test.width(),
                );
            }
        }
        let operands = args
            .iter()
            .map(|arg| {
                if test.wide {
                    self.lower_expr(arg)
                } else {
                    self.vec128_arg_ptr(arg)
                }
            })
            .collect();
        let dest = self.fresh_value();
        self.emit(Instruction::Intrinsic {
            dest: Some(dest),
            op: IntrinsicOp::AvxTest(test),
            dest_ptr: None,
            args: operands,
        });
        Some(Operand::Value(dest))
    }

    /// Reject a builtin call, lower the arguments for their side effects, and
    /// return a placeholder of `bytes` bytes.
    pub(super) fn reject_builtin_call(
        &mut self,
        message: String,
        args: &[Expr],
        bytes: usize,
    ) -> Option<Operand> {
        self.diagnostics
            .borrow_mut()
            .error(message, Self::builtin_arg_span(args));
        for a in args {
            let _ = self.lower_expr(a);
        }
        Some(self.rejected_vector_operand(bytes))
    }

    /// Resolve the argument of `__builtin_cpu_supports` (AUD-CPU-1).
    ///
    /// The answer is always the runtime CPUID probe
    /// (`crate::common::cpu_probe`) — see [`CpuSupport`] for the measured
    /// competition behavior behind that decision.  A non-literal argument or a
    /// name no compiler accepts is a hard error with GCC's own wording;
    /// silently folding it to 0 is the AUD-SILENT-ZERO-1 defect this replaces.
    fn resolve_cpu_support(&self, args: &[Expr]) -> Result<CpuSupport, String> {
        let name = match args {
            [crate::frontend::parser::ast::Expr::StringLiteral(s, _)] => s,
            [] => return Err("too few arguments to function '__builtin_cpu_supports'".to_string()),
            [_, _, ..] => {
                return Err("too many arguments to function '__builtin_cpu_supports'".to_string());
            }
            _ => {
                return Err("parameter to builtin must be a string constant or literal".to_string());
            }
        };
        // A runtime probe needs CPUID; on AArch64/RISC-V the question has no
        // runtime answer here, so it keeps the conservative 0 and no
        // diagnostic (their feature names are GCC's per-architecture set —
        // `fp`, `crypto`, ... — which LCCC does not model yet: rejecting them
        // would turn accepted GCC code into a compile error).
        if !matches!(
            self.target,
            crate::backend::Target::X86_64 | crate::backend::Target::I686
        ) {
            // AUD-CPU-1 (3): a name from the x86 feature set is a question this
            // target cannot answer at all, and silently folding it to 0 turns
            // `__builtin_cpu_supports("avx2") ? fast() : slow()` into a silent
            // slow path (GCC diagnoses the name on every non-x86 target).  A
            // name from another architecture's set keeps the conservative 0:
            // LCCC does not model those sets yet, and rejecting them would turn
            // accepted GCC code into a compile error.
            if crate::common::x86_features::X86Feature::parse(name).is_some() {
                return Err(format!("parameter to builtin not valid: {name}"));
            }
            return Ok(CpuSupport::Unsupported);
        }
        match crate::common::cpu_probe::lookup(name) {
            Some(probe) => Ok(CpuSupport::Runtime(probe)),
            None => Err(format!("parameter to builtin not valid: {name}")),
        }
    }

    /// Record that this function reads a probe word, so the runtime CPU probe
    /// words are initialised in it (AUD-CPU-1).
    ///
    /// The initialising call is materialised once per function, guarded by the
    /// ready flag, by [`Self::splice_cpu_probe_init`] at function finalisation —
    /// a probe inside a conditional branch must not depend on that branch
    /// having been taken, and guarding the call in the entry block also means
    /// the probe itself stays a three-instruction bit test at every use.
    pub(super) fn mark_cpu_probe_init(&mut self) {
        if let Some(state) = self.func_state.as_mut() {
            state.cpu_probe_init = true;
        }
        crate::common::cpu_probe::mark_runtime_needed();
    }

    /// Materialise the guarded probe initialiser in the function just lowered,
    /// when any probe site asked for it.
    ///
    /// The entry block is split at the first probe-word read into
    ///
    /// ```text
    ///   entry:  <head>                  ; arguments, allocas, anything before the first read
    ///           ready = load __lccc_cpu_ready
    ///           if (ready != 0) goto hot else goto cold
    ///   cold:   call __lccc_cpu_init    ; runs at most once per program
    ///           goto hot
    ///   hot:    <tail>                  ; the first probe-word read and everything after
    /// ```
    ///
    /// so that
    ///
    /// * the words are initialised before *every* read of them: the guard
    ///   dominates the hot block, which holds the first read and all later
    ///   ones;
    /// * a probe that runs after the first initialisation costs a compare and
    ///   a not-taken branch instead of a call into the initialiser's
    ///   prologue, a register save around it and the caller-side spill of
    ///   everything live across the call.  Measured with callgrind on
    ///   `tests/workloads/dispatch` (x86-64, `-O2`): the unconditional call
    ///   cost ~11 instructions per probe, the inline guard ~2 (plus the
    ///   register pressure the cold call still implies);
    /// * the initialiser still runs lazily from user code, so a translation
    ///   unit that probes stays correct without any ELF constructor — the
    ///   property that motivates the lazy model over GCC's
    ///   `__cpu_indicator_init` constructor
    ///   (`engineering/cpu-supports-runtime-model.md`, §3);
    /// * `cold` is appended last, so block layout keeps the once-per-program
    ///   path out of the fall-through line;
    /// * the head of the entry block — in particular the leading parameter
    ///   allocas, which `mem2reg` identifies positionally — never moves.
    ///
    /// An entry block whose moved range carries phis falls back to an
    /// unconditional call placed exactly like before: phi operands name their
    /// predecessor block, so moving a phi means rewriting the incoming edges
    /// of the block that owns it, which is only reachable when a loop re-enters
    /// the entry block *and* the first probe read sits after a phi.  The
    /// fallback is correct, just slower on the hot path.
    pub(super) fn splice_cpu_probe_init(&mut self) {
        if !self.func_state.as_ref().is_some_and(|s| s.cpu_probe_init) {
            return;
        }
        let (entry_label, pos, splittable) = {
            let Some(f) = self.func_state.as_ref() else {
                return;
            };
            let Some(entry) = f.blocks.first() else {
                return;
            };
            let pos = entry
                .instructions
                .iter()
                .position(|inst| {
                    matches!(inst, Instruction::GlobalAddr { name, .. }
                        if crate::common::cpu_probe::is_probe_word_symbol(name))
                })
                .unwrap_or(entry.instructions.len());
            let splittable = !entry.instructions[pos..]
                .iter()
                .any(|inst| matches!(inst, Instruction::Phi { .. }));
            (entry.label, pos, splittable)
        };

        if !splittable {
            let Some(entry) = self.func_state.as_mut().and_then(|s| s.blocks.first_mut()) else {
                return;
            };
            entry.instructions.insert(pos, Self::cpu_probe_init_call());
            if !entry.source_spans.is_empty() {
                entry.source_spans.insert(pos, Span::dummy());
            }
            return;
        }

        let hot = self.fresh_label();
        let cold = self.fresh_label();
        let ready_addr = self.fresh_value();
        let ready_val = self.fresh_value();
        let ready_cond = self.fresh_value();

        let Some(f) = self.func_state.as_mut() else {
            return;
        };
        let Some(entry) = f.blocks.first_mut() else {
            return;
        };
        let tracked = !entry.source_spans.is_empty();
        let tail = entry.instructions.split_off(pos);
        let tail_spans = if tracked {
            entry.source_spans.split_off(pos)
        } else {
            Vec::new()
        };
        let old_terminator = std::mem::replace(
            &mut entry.terminator,
            Terminator::CondBranch {
                cond: Operand::Value(ready_cond),
                true_label: hot,
                false_label: cold,
            },
        );
        entry.instructions.push(Instruction::GlobalAddr {
            dest: ready_addr,
            name: crate::common::cpu_probe::READY_SYMBOL.to_string(),
        });
        entry.instructions.push(Instruction::Load {
            dest: ready_val,
            ptr: ready_addr,
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        entry.instructions.push(Instruction::Cmp {
            dest: ready_cond,
            op: IrCmpOp::Ne,
            lhs: Operand::Value(ready_val),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: IrType::I32,
        });
        if tracked {
            entry.source_spans.extend([Span::dummy(); 3]);
        }
        f.blocks.push(BasicBlock {
            label: hot,
            instructions: tail,
            terminator: old_terminator,
            source_spans: tail_spans,
        });
        f.blocks.push(BasicBlock {
            label: cold,
            instructions: vec![Self::cpu_probe_init_call()],
            terminator: Terminator::Branch(hot),
            source_spans: if tracked {
                vec![Span::dummy()]
            } else {
                Vec::new()
            },
        });
        // The edges that used to leave the entry block leave `hot` now, so phi
        // operands in the successor blocks must name `hot` instead of the
        // entry label.  (`entry` keeps its own phis: those name its
        // predecessors, which did not change.)
        for block in f.blocks.iter_mut() {
            for inst in block.instructions.iter_mut() {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (_, from) in incoming.iter_mut() {
                        if *from == entry_label {
                            *from = hot;
                        }
                    }
                }
            }
        }
    }

    /// The `call __lccc_cpu_init` instruction (AUD-CPU-1).
    fn cpu_probe_init_call() -> Instruction {
        Instruction::Call {
            func: crate::common::cpu_probe::INIT_SYMBOL.to_string(),
            info: CallInfo {
                dest: None,
                args: Vec::new(),
                arg_types: Vec::new(),
                return_type: IrType::Void,
                is_variadic: false,
                num_fixed_args: 0,
                ..Default::default()
            },
        }
    }

    /// Emit the read of one cached probe word: `(word & mask) != 0`.
    ///
    /// The load is a plain, non-volatile `i32` load from a `.local` `.bss`
    /// symbol the driver defines in this same translation unit, so it needs no
    /// relocation model support beyond an ordinary global and is CSE-able by
    /// later passes when the answer is re-used.
    pub(super) fn emit_cpu_probe_read(&mut self, probe: crate::common::cpu_probe::Probe) -> Value {
        let addr = self.fresh_value();
        self.emit(Instruction::GlobalAddr {
            dest: addr,
            name: probe.word.symbol().to_string(),
        });
        let word = self.fresh_value();
        self.emit(Instruction::Load {
            dest: word,
            ptr: addr,
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        let masked = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: masked,
            op: IrBinOp::And,
            lhs: Operand::Value(word),
            rhs: Operand::Const(IrConst::I32(probe.bit_mask() as i32)),
            ty: IrType::I32,
        });
        self.emit_cmp_val(
            IrCmpOp::Ne,
            Operand::Value(masked),
            Operand::Const(IrConst::I32(0)),
            IrType::I32,
        )
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
            // Preserve LCCC's deterministic zero extension for direct builtins:
            // Clz(0)/Ctz(0) == operand width. CVP specializes these to the
            // branchless NonZero forms only where control flow proves the
            // source nonzero, retaining compatibility and optimal guarded loops.
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
            BuiltinIntrinsic::ObjectSize => self.lower_object_size_builtin(name, args),
            BuiltinIntrinsic::StdcBit => self.lower_stdc_bit_builtin(name, args),
            BuiltinIntrinsic::CountedByRef => self.lower_counted_by_ref(args),
            BuiltinIntrinsic::AssocBarrier => self.lower_assoc_barrier(args),
            BuiltinIntrinsic::ClearPadding => self.lower_clear_padding(args),
            BuiltinIntrinsic::Gcc16GenericBit => self.lower_gcc16_generic_bit_builtin(name, args),
            BuiltinIntrinsic::Gcc16Carry => self.lower_gcc16_carry_builtin(name, args),
            BuiltinIntrinsic::Gcc16BswapBitrev => self.lower_gcc16_bswap_bitrev_builtin(name, args),
            BuiltinIntrinsic::IsSignaling => self.lower_gcc16_issignaling(args),
            BuiltinIntrinsic::IsEqSig => self.lower_gcc16_iseqsig(args),
            BuiltinIntrinsic::SpeculationSafeValue => self.lower_gcc16_speculation_safe(args),
            // __builtin_classify_type(expr) -> GCC type class integer
            BuiltinIntrinsic::ClassifyType => Some(Operand::Const(IrConst::I64(
                self.classify_type_of(args.first()),
            ))),
            // Nop intrinsic - just evaluate args for side effects and return 0
            BuiltinIntrinsic::Nop => {
                for arg in args {
                    self.lower_expr(arg);
                }
                Some(Operand::Const(IrConst::I64(0)))
            }
            // __builtin_cpu_init() - ensure the runtime CPU probe words are
            // populated.  GCC's libgcc fills `__cpu_model` from a constructor
            // (priority 101) and documents this builtin for the ifunc resolvers
            // that run before it; LCCC initialises lazily instead — one guarded
            // call per function that probes, which needs no runtime constructor
            // and therefore also works in freestanding translation units.  The
            // value is 0, matching the way GCC accepts it in an int context.
            BuiltinIntrinsic::CpuInit => {
                self.mark_cpu_probe_init();
                Some(Operand::Const(IrConst::I32(0)))
            }
            // __builtin_cpu_supports("feature") - answer the question GCC
            // answers (AUD-CPU-1): 1 iff the CPU executing this code has the
            // feature, at run time, for *every* name — the target's feature set
            // never turns the answer into a constant (GCC, Clang and ICX probe
            // unconditionally; see `CpuSupport` above).  A non-literal argument
            // or an unknown name is a hard error with GCC's wording.
            //
            // Inside a constant-expression context there is no function to
            // emit the probe into (a static initialiser: `static int v =
            // __builtin_cpu_supports("avx2");`), so the context is diagnosed as
            // non-constant — exactly what all three reference compilers do.
            BuiltinIntrinsic::CpuSupports => {
                match self.resolve_cpu_support(args) {
                    Ok(CpuSupport::Runtime(probe)) if self.func_state.is_some() => {
                        for arg in args {
                            let _ = self.lower_expr(arg);
                        }
                        self.mark_cpu_probe_init();
                        Some(Operand::Value(self.emit_cpu_probe_read(probe)))
                    }
                    Ok(CpuSupport::Runtime(_)) => {
                        // A runtime answer needs IR, and this path has no
                        // function to emit it into: the only way a lowered
                        // `__builtin_cpu_supports` reaches here is a
                        // constant-expression context — a static initialiser —
                        // where GCC, Clang and ICX all refuse to compile with
                        // this exact wording (`static int v =
                        // __builtin_cpu_supports("avx2");` is rejected by all
                        // three; measured).  Diagnose and keep the discarded
                        // value well-formed.
                        for arg in args {
                            let _ = self.lower_expr(arg);
                        }
                        self.diagnostics.borrow_mut().error(
                            "initializer element is not constant",
                            Self::builtin_arg_span(args),
                        );
                        Some(Operand::Const(IrConst::I32(0)))
                    }
                    Ok(CpuSupport::Unsupported) => {
                        // AArch64/RISC-V: the question has no x86 answer at
                        // all, so the conservative 0 is the constant answer.
                        for arg in args {
                            let _ = self.lower_expr(arg);
                        }
                        Some(Operand::Const(IrConst::I32(0)))
                    }
                    Err(message) => {
                        self.diagnostics
                            .borrow_mut()
                            .error(message, Self::builtin_arg_span(args));
                        for arg in args {
                            let _ = self.lower_expr(arg);
                        }
                        Some(Operand::Const(IrConst::I32(0)))
                    }
                }
            }
            BuiltinIntrinsic::X86Bzhi32 => self.lower_bzhi_builtin(intrinsic, false, name, args),
            BuiltinIntrinsic::X86Bzhi64 => self.lower_bzhi_builtin(intrinsic, true, name, args),
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
            | BuiltinIntrinsic::X86AddSd
            | BuiltinIntrinsic::X86SubSd
            | BuiltinIntrinsic::X86MulSd
            | BuiltinIntrinsic::X86DivSd
            | BuiltinIntrinsic::X86MinSd
            | BuiltinIntrinsic::X86MaxSd
            | BuiltinIntrinsic::X86SqrtSd
            | BuiltinIntrinsic::X86AddSs
            | BuiltinIntrinsic::X86SubSs
            | BuiltinIntrinsic::X86MulSs
            | BuiltinIntrinsic::X86DivSs
            | BuiltinIntrinsic::X86MinSs
            | BuiltinIntrinsic::X86MaxSs
            | BuiltinIntrinsic::X86SqrtSs
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
            | BuiltinIntrinsic::X86Cvtsd2Si64
            | BuiltinIntrinsic::X86Cvtss2Si64
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
            | BuiltinIntrinsic::X86Phsubw128
            | BuiltinIntrinsic::X86Phsubd128
            | BuiltinIntrinsic::X86Phaddsw128
            | BuiltinIntrinsic::X86Phsubsw128
            | BuiltinIntrinsic::X86Psignb128
            | BuiltinIntrinsic::X86Psignw128
            | BuiltinIntrinsic::X86Psignd128
            | BuiltinIntrinsic::X86Pmulhrsw128
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
            | BuiltinIntrinsic::X86SetzeroPs128
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
            | BuiltinIntrinsic::X86AndnPs
            | BuiltinIntrinsic::X86XorPd
            | BuiltinIntrinsic::X86AndPd
            | BuiltinIntrinsic::X86OrPd
            | BuiltinIntrinsic::X86AndnPd
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
            | BuiltinIntrinsic::X86CastReinterpret => {
                self.check_x86_intrinsic_feature(intrinsic, name, args);
                self.lower_x86_intrinsic(intrinsic, name, args)
            }
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
            BuiltinIntrinsic::X86AvxWiden(op) => self.lower_avx_widen(*op, name, args),
            BuiltinIntrinsic::X86AvxTest(test) => self.lower_avx_test(*test, name, args),
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
                        regparm: None,
                        is_pure: false,
                        is_const: false,
                        assume_library: false,
                        ret_eightbyte_classes: Vec::new(),
                        ret_is_f128_sse: false,
                        cold: false,
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
                regparm: None,
                is_pure: false,
                is_const: false,
                // Spelled `__builtin___<name>_chk`: the fortified builtin by definition.
                assume_library: true,
                ret_eightbyte_classes: Vec::new(),
                ret_is_f128_sse: false,
                cold: false,
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
            let val_ty = self.value_ir_type(&args[0]);
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
            Expr::Cast(_, inner, _, _) => self.constant_query_can_lower(inner),
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
        // A string literal's address is constant even though numeric const-eval
        // has no IrConst for an address (GCC's rule, shared with the static
        // initializer evaluators). A character read from one at a constant
        // index is the constant GCC -O1+ reports (`"x"[0]`, `*"x"`).
        let is_literal = |e: &Expr| {
            matches!(
                e,
                Expr::StringLiteral(..)
                    | Expr::WideStringLiteral(..)
                    | Expr::Char16StringLiteral(..)
            )
        };
        let constant_literal_char = match arg {
            Expr::ArraySubscript(base, index, _) => {
                is_literal(base) && self.eval_const_expr(index).is_some()
            }
            Expr::Deref(inner, _) => is_literal(inner),
            _ => false,
        };
        let cast_is_nop = |target: &TypeSpecifier| {
            let ty = self.type_spec_to_ctype(target);
            matches!(ty, CType::Pointer(..))
                || (ty.is_integer() && ty.size() == crate::common::types::target_ptr_size())
        };
        if self.eval_const_expr(arg).is_some()
            || constant_literal_char
            || crate::common::const_eval::is_string_literal_address(arg, &cast_is_nop, &|e| {
                self.eval_const_expr(e)
            })
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

    /// Native vectors, including nested GNU builtin results and equal-width
    /// casts, share the ordinary vector object-address IR representation.
    fn vec128_arg_ptr(&mut self, arg: &Expr) -> Operand {
        self.lower_expr(arg)
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
        // Every rejection below used to be a `panic!`, i.e. an ICE on input a
        // user can write: a mask whose element count differs from the vector's
        // (`v8si` mask for a `v4si`), a non-vector mask, a non-vector first
        // operand. GCC diagnoses all three; so does this now. The returned
        // `None` means "not lowered as this builtin", and the caller's
        // ordinary-call fallback cannot panic. Reporting with the *operand's*
        // span matters: `lower_shuffle` is reached from a nested expression,
        // and a location-less error in a 50k-line TU is not actionable.
        let (vec_args, mask_expr): (&[Expr], &Expr) = match args.len() {
            2 => (&args[..1], &args[1]),
            3 => (&args[..2], &args[2]),
            n => {
                // GCC: "wrong number of arguments to '__builtin_shuffle'".
                let _ = n;
                return self.reject_builtin_call(
                    "wrong number of arguments to '__builtin_shuffle'".to_string(),
                    args,
                    16,
                );
            }
        };

        // Element geometry from the first vector's CType.
        let v0_ctype = self.expr_ctype(&vec_args[0]);
        let (elem_size, total_bytes) = match &v0_ctype {
            CType::Vector(elem, bytes) => (self.ctype_size(elem), *bytes),
            other => {
                return self.reject_builtin_call(
                    format!(
                        "'__builtin_shuffle' first argument must be a vector type, got {:?}",
                        other
                    ),
                    args,
                    16,
                );
            }
        };
        if elem_size == 0 || total_bytes == 0 || total_bytes % elem_size != 0 {
            return self.reject_builtin_call(
                format!(
                    "'__builtin_shuffle' argument vector has degenerate geometry (element {} bytes, total {} bytes)",
                    elem_size, total_bytes
                ),
                args,
                total_bytes.max(16),
            );
            return None;
        }
        let n_elems: i64 = (total_bytes / elem_size) as i64;
        // Mask element class and width from the mask operand's CType.  GCC:
        // "'__builtin_shuffle' last argument must be an integer vector".
        let mask_ctype = self.expr_ctype(mask_expr);
        let mask_elem_size = match &mask_ctype {
            CType::Vector(elem, bytes) => {
                if !elem.is_integer() {
                    return self.reject_builtin_call(
                        "'__builtin_shuffle' last argument must be an integer vector".to_string(),
                        args,
                        total_bytes,
                    );
                }
                let es = self.ctype_size(elem);
                if es == 0 || *bytes == 0 || bytes % es != 0 || (bytes / es) as i64 != n_elems {
                    // GCC: "number of elements of the argument vector(s) and
                    // the mask vector should be the same".
                    return self.reject_builtin_call(
                        "'__builtin_shuffle' number of elements of the argument vector(s) \
                         and the mask vector should be the same"
                            .to_string(),
                        args,
                        total_bytes,
                    );
                    return None;
                }
                if es == 0 || bytes % es != 0 || (bytes / es) as i64 != n_elems {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "__builtin_shuffle: mask element count must match the vector's ({} mask element(s) for {} vector element(s))",
                            if es > 0 { bytes / es } else { 0 },
                            n_elems
                        ),
                        mask_expr.span(),
                    );
                    return None;
                }
                es
            }
            _ => {
                return self.reject_builtin_call(
                    "'__builtin_shuffle' last argument must be an integer vector".to_string(),
                    args,
                    total_bytes,
                );
            }
        };

        let ir_ty = match elem_size {
            1 => IrType::I8,
            2 => IrType::I16,
            4 => IrType::I32,
            8 => IrType::I64,
            16 => IrType::I128,
            other => {
                return self.reject_builtin_call(
                    format!("'__builtin_shuffle' unsupported element size {}", other),
                    args,
                    total_bytes,
                );
            }
        };
        let mask_ir_ty = match mask_elem_size {
            1 => IrType::U8,
            2 => IrType::U16,
            4 => IrType::U32,
            8 => IrType::U64,
            other => {
                return self.reject_builtin_call(
                    format!(
                        "'__builtin_shuffle' unsupported mask element size {}",
                        other
                    ),
                    args,
                    total_bytes,
                );
            }
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
            // `vec_args` holds the VECTOR arguments only, so its length is 1
            // for the 2-argument form `(v, mask)` and 2 for the 3-argument
            // form `(v1, v2, mask)`.  The selectable index range is
            // accordingly N (the 2-argument form behaves as if both operands
            // were `v`, so the index is reduced modulo N) or 2N (the index
            // picks a lane of the concatenation).  Both this multiplier and
            // the select below were keyed on the wrong length: `== 2` meant a
            // 2-argument mask was reduced modulo 2N -- letting a mask element
            // >= N read past the vector -- and a 3-argument mask modulo N,
            // which made the "is it the first vector?" test below a constant
            // true.  Together they compiled `__builtin_shuffle(a, b,
            // (v4si){4, 5, 6, 7})` into `a` unchanged.
            let selectable = n_elems * if vec_args.len() >= 2 { 2 } else { 1 };
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
            let (base, elem_idx) = if vec_args.len() >= 2 {
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
            return self.reject_builtin_call(
                "wrong number of arguments to '__builtin_shufflevector' (this implementation supports two vectors and four indices)".to_string(), args, 16,
            );
        }
        for arg in &args[..2] {
            if let Some(ct) = self.get_expr_ctype(arg) {
                if !ct
                    .vector_info()
                    .is_some_and(|(element, lanes)| element.size() == 4 && lanes == 4)
                {
                    return self.reject_builtin_call(
                        "'__builtin_shufflevector' requires four-lane 32-bit vector operands"
                            .to_string(),
                        args,
                        16,
                    );
                }
            }
        }
        if let (Some(first), Some(second)) =
            (self.get_expr_ctype(&args[0]), self.get_expr_ctype(&args[1]))
        {
            if let (Some((a, _)), Some((b, _))) = (first.vector_info(), second.vector_info()) {
                if a != b {
                    return self.reject_builtin_call(
                        "'__builtin_shufflevector' requires matching vector element types"
                            .to_string(),
                        args,
                        16,
                    );
                }
            }
        }
        let mut indices = [0i64; 4];
        for (lane, index) in indices.iter_mut().enumerate() {
            *index = match self.eval_const_expr(&args[2 + lane]) {
                Some(IrConst::I64(v)) => v,
                Some(IrConst::I32(v)) => v as i64,
                Some(IrConst::I8(v)) => v as i64,
                Some(IrConst::I16(v)) => v as i64,
                _ => return self.reject_builtin_call(
                    format!("argument {} of '__builtin_shufflevector' must be an integer constant expression", 3 + lane), args, 16,
                ),
            };
            if !(-1..8).contains(index) {
                return self.reject_builtin_call(
                    format!(
                        "invalid element index '{}' to '__builtin_shufflevector'",
                        index
                    ),
                    args,
                    16,
                );
            }
            // -1 is an unspecified lane. Choose lane zero of the first input.
            if *index == -1 {
                *index = 0;
            }
        }
        // Evaluate both operands once even if only one supplies output lanes.
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
        let first_only = indices.iter().all(|&i| i < 4);
        let second_only = indices.iter().all(|&i| i >= 4);
        if (first_only || second_only)
            && matches!(
                self.target,
                crate::backend::Target::X86_64 | crate::backend::Target::I686
            )
            && crate::common::x86_features::effective()
                .contains(crate::common::x86_features::X86Feature::Sse2)
        {
            // PSHUFD copies arbitrary 32-bit bit patterns, including floats;
            // it is not a floating-point operation and raises no FP exception.
            let immediate = indices
                .iter()
                .enumerate()
                .fold(0i32, |imm, (lane, &i)| imm | ((i as i32 & 3) << (lane * 2)));
            // Canonical typed homes let vecssa expose a register permutation
            // web. Keep volatile and named-address-space accesses on their
            // existing memory path, and do not mix F32 and I32 home families.
            let shape = self
                .get_expr_ctype(&args[0])
                .and_then(|ct| super::vector_simd::simd_shape(&ct));
            if let Some(shape) = shape.filter(|_| {
                !self.expr_access_is_volatile(&args[0])
                    && !self.expr_access_is_volatile(&args[1])
                    && self.aggregate_lvalue_space(&args[0]) == AddressSpace::Default
                    && self.aggregate_lvalue_space(&args[1]) == AddressSpace::Default
            }) {
                let input = self.emit_vec_load(&shape, if first_only { v1p } else { v2p });
                let shuffled = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(shuffled),
                    op: IntrinsicOp::VecShufdI32x4,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(input),
                        Operand::Const(IrConst::I32(immediate)),
                    ],
                });
                self.emit_vec_store(&shape, shuffled, result);
                return Some(Operand::Value(result));
            }
            self.emit(Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::Pshufd128,
                dest_ptr: Some(result),
                args: vec![
                    Operand::Value(if first_only { v1p } else { v2p }),
                    Operand::Const(IrConst::I32(immediate)),
                ],
            });
            return Some(Operand::Value(result));
        }
        // Mixed sources and non-SSE2 targets retain the portable scalar gather.
        for (lane, &index) in indices.iter().enumerate() {
            let (src, offset) = if index < 4 {
                (v1p, index * 4)
            } else {
                (v2p, (index - 4) * 4)
            };
            let src_ptr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: src_ptr,
                base: src,
                offset: Operand::Const(IrConst::I64(offset)),
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
        // Typed vector expressions always carry the address of their data,
        // just like vector lvalues and __builtin_shuffle results >8 bytes.
        Some(Operand::Value(result))
    }

    /// Lower an X86 SSE/AES/CRC intrinsic to IR.
    ///
    /// Classifies the intrinsic into one of four emission patterns:
    /// - Fence: no args, no dest (lfence, mfence, sfence, pause)
    /// - PtrStore: first arg is dest pointer, remaining args are data (movnti, storedqu, etc.)
    /// - Vec128: allocates 16-byte result slot, returns pointer (SSE/AES packed ops)
    /// - Scalar: returns a scalar value in a dest register (pmovmskb, crc32, pextrw, etc.)
    /// AUD-CPU-3: diagnose an intrinsic whose instruction-set extension the
    /// requested target level does not have.
    ///
    /// `IntrinsicOp` is target-independent, so nothing downstream stopped a
    /// `crc32`/`aesenc`/`pclmulqdq` from being emitted for a target that had
    /// explicitly been told not to assume the extension: `-march=x86-64`
    /// (SSE2 only) compiled `_mm_crc32_u8` to `crc32b`, i.e. to an illegal
    /// instruction on exactly the hardware the user declared -- a SIGILL with
    /// no diagnostic at any stage.  GCC and Clang reject the same source
    /// ("target specific option mismatch" / "requires target feature"), and
    /// the permission consulted here is the SAME `effective()` set
    /// `__builtin_cpu_supports` folds against (AUD-CPU-1), so the two can
    /// never disagree.
    ///
    /// x86 targets only: on AArch64/RISC-V every x86 extension is absent, and
    /// those intrinsics are handled by the AUD-SILENT-ZERO-1 trap instead of a
    /// second, differently-worded failure path (target-scoped intrinsics are a
    /// separate item in the follow-up document).
    ///
    /// The call is still lowered after the error is recorded: the driver
    /// rejects the translation unit on any lowering error, and bailing here
    /// would only add a way to skip the diagnostic.
    fn check_x86_intrinsic_feature(
        &mut self,
        intrinsic: &BuiltinIntrinsic,
        name: &str,
        args: &[Expr],
    ) {
        if let Some(requirement) = x86_intrinsic_required_feature(intrinsic) {
            self.check_x86_feature_requirement(requirement, name, args);
        }
    }

    pub(super) fn check_x86_simd_feature(
        &mut self,
        feature: crate::common::x86_features::X86Feature,
        flag: &'static str,
        name: &str,
        args: &[Expr],
    ) {
        self.check_x86_feature_requirement(
            IntrinsicRequirement {
                feature,
                flag,
                also: None,
            },
            name,
            args,
        );
    }

    fn check_x86_feature_requirement(
        &mut self,
        requirement: IntrinsicRequirement,
        name: &str,
        args: &[Expr],
    ) {
        let span = args
            .first()
            .map(|a| a.span())
            .unwrap_or_else(|| crate::common::source::Span::new(0, 0, 0));
        // AUD-SILENT-ZERO-1 / AUD-CPU-3: this function owns the *permission*
        // half — on x86 the intrinsic exists, but the invocation may not have
        // granted the extension it needs.  A foreign target is
        // `lower_x86_intrinsic`'s rejection to make (it runs after this check
        // and names the target: "'_mm_add_ps' is not available on aarch64");
        // diagnosing it here as well would print two errors for one call.
        // 3 = EM_386 (i686), 62 = EM_X86_64.
        if !matches!(crate::common::types::target_elf_machine(), 3 | 62) {
            return;
        }
        let mut permitted = crate::common::x86_features::effective().contains(requirement.feature);
        // Legacy 128-bit extensions keep the same register/ABI class under a
        // function target attribute. Honor that local permission without leaking
        // it into another function or claiming per-function wider-vector support.
        use crate::common::x86_features::X86Feature as F;
        let needed = match requirement.feature {
            F::Sse => Some(0),
            F::Sse2 => Some(1),
            F::Sse3 => Some(2),
            F::Avx2 => Some(7),
            F::Avx => Some(6),
            F::Ssse3 => Some(3),
            F::Sse41 => Some(4),
            F::Sse42 => Some(5),
            _ => None,
        };
        if let Some(function) = self.func_state.as_ref() {
            for request in &function.target_features {
                let (enabled, feature) = request
                    .strip_prefix("no-")
                    .map_or((true, request.as_str()), |f| (false, f));
                // Exact target-feature grants also cover FMA and AVX-512
                // subsets, which are independent features rather than rungs
                // in the legacy SSE/AVX level ladder. An enabled AVX-512
                // subset implies its F base; disabling one subset does not
                // revoke the independent F capability.
                let avx512_feature = matches!(
                    feature,
                    "avx512f"
                        | "avx512bw"
                        | "avx512dq"
                        | "avx512vl"
                        | "avx512cd"
                        | "avx512ifma"
                        | "avx512vbmi"
                        | "avx512vbmi2"
                        | "avx512vnni"
                        | "avx512bitalg"
                        | "avx512vpopcntdq"
                        | "avx512bf16"
                        | "avx512fp16"
                        | "avx512er"
                        | "avx512pf"
                        | "avx512vp2intersect"
                        | "avx512bmm"
                );
                let avx512_bw_dependent = matches!(
                    requirement.feature,
                    F::Avx512Vbmi
                        | F::Avx512Vbmi2
                        | F::Avx512BitAlg
                        | F::Avx512Bf16
                        | F::Avx512Fp16
                        | F::Avx512Bmm
                );
                if feature == "avx512f" {
                    if avx512_feature {
                        permitted = enabled;
                    }
                    continue;
                }
                if !enabled && feature == "avx512bw" && avx512_bw_dependent {
                    permitted = false;
                    continue;
                }
                if feature == requirement.feature.name() {
                    permitted = enabled;
                    continue;
                }
                if enabled && requirement.feature == F::Avx512F && avx512_feature {
                    permitted = true;
                    continue;
                }
                if enabled
                    && requirement.feature == F::Avx512Bw
                    && matches!(
                        feature,
                        "avx512vbmi" | "avx512vbmi2" | "avx512bitalg" | "avx512bf16" | "avx512fp16"
                    )
                {
                    permitted = true;
                    continue;
                }
                if enabled && requirement.feature == F::Avx512Dq && feature == "avx512vp2intersect"
                {
                    permitted = true;
                    continue;
                }
                if !enabled
                    && requirement.feature == F::Fma
                    && matches!(
                        feature,
                        "sse" | "sse2" | "sse3" | "ssse3" | "sse4.1" | "sse4.2" | "avx"
                    )
                {
                    permitted = false;
                    continue;
                }
                let Some(needed) = needed else {
                    continue;
                };
                let level = match feature {
                    "sse" => 0,
                    "sse2" => 1,
                    "sse3" => 2,
                    "ssse3" => 3,
                    "sse4.1" => 4,
                    "sse4.2" => 5,
                    "avx" => 6,
                    "avx2" => 7,
                    _ => continue,
                };
                if enabled && level >= needed {
                    permitted = true;
                }
                if !enabled && level <= needed {
                    permitted = false;
                }
            }
        }
        if permitted {
            return;
        }
        self.diagnostics.borrow_mut().error(
            format!(
                "`{}` requires the {} instruction-set extension ({}){}, which the requested target does not have",
                name,
                requirement.feature.name(),
                requirement.flag,
                requirement.also.unwrap_or("")
            ),
            span,
        );
    }

    /// `__builtin_ia32_bzhi_si`/`_di`: BMI2 `bzhi` (see `IrBinOp::Bzhi` for
    /// the exact semantics: index bits 7:0, identity from the width on).
    ///
    /// x86-64 lowers to the IR operation, which the backend emits as one
    /// `bzhi` and which constant-folds and simplifies exactly.  i686 has no
    /// `Bzhi` lowering, so `_si` becomes the equivalent select with every
    /// shift count in range (`k & 31`); `_di` is 64-bit-only, as in GCC.  The
    /// BMI2 permission is checked like every other x86 extension intrinsic.
    fn lower_bzhi_builtin(
        &mut self,
        intrinsic: &BuiltinIntrinsic,
        wide: bool,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        let target = self.target;
        let available = match target {
            crate::backend::Target::X86_64 => true,
            crate::backend::Target::I686 => !wide,
            _ => false,
        };
        if !available {
            let arch = match target {
                crate::backend::Target::Aarch64 => "aarch64",
                crate::backend::Target::Riscv64 => "riscv64",
                crate::backend::Target::I686 => "i686",
                other => other.triple(),
            };
            return self.reject_builtin_call(
                format!("'{name}' is not available on {arch}"),
                args,
                8,
            );
        }
        self.check_x86_intrinsic_feature(intrinsic, name, args);
        let [src, index] = args else {
            return self.reject_builtin_call(
                format!("'{name}' takes exactly two arguments, {} given", args.len()),
                args,
                8,
            );
        };
        let ty = if wide { IrType::U64 } else { IrType::U32 };
        let src = self.lower_expr_with_type(src, ty);
        let index = self.lower_expr_with_type(index, ty);
        if target == crate::backend::Target::X86_64 {
            let dest = self.fresh_value();
            self.emit(Instruction::BinOp {
                dest,
                op: IrBinOp::Bzhi,
                lhs: src,
                rhs: index,
                ty,
            });
            return Some(Operand::Value(dest));
        }
        // i686, 32-bit: k = index & 0xFF; k >= 32 ? src : src & ((1 << (k & 31)) - 1).
        let k = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: k,
            op: IrBinOp::And,
            lhs: index,
            rhs: Operand::Const(IrConst::from_i64(0xFF, ty)),
            ty,
        });
        let count = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: count,
            op: IrBinOp::And,
            lhs: Operand::Value(k),
            rhs: Operand::Const(IrConst::from_i64(31, ty)),
            ty,
        });
        let bit = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: bit,
            op: IrBinOp::Shl,
            lhs: Operand::Const(IrConst::from_i64(1, ty)),
            rhs: Operand::Value(count),
            ty,
        });
        let mask = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: mask,
            op: IrBinOp::Sub,
            lhs: Operand::Value(bit),
            rhs: Operand::Const(IrConst::from_i64(1, ty)),
            ty,
        });
        let kept = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: kept,
            op: IrBinOp::And,
            lhs: src,
            rhs: Operand::Value(mask),
            ty,
        });
        let whole = self.emit_cmp_val(
            IrCmpOp::Uge,
            Operand::Value(k),
            Operand::Const(IrConst::from_i64(32, ty)),
            ty,
        );
        let dest = self.fresh_value();
        self.emit(Instruction::Select {
            dest,
            cond: Operand::Value(whole),
            true_val: src,
            false_val: Operand::Value(kept),
            ty,
        });
        Some(Operand::Value(dest))
    }

    /// Builtins bypass ordinary call marshalling, but still require C parameter
    /// conversions. Retain vector/address operands as object carriers.
    pub(super) fn lower_typed_x86_builtin_argument(
        &mut self,
        name: &str,
        arg: &Expr,
        index: usize,
    ) -> Operand {
        let target = match crate::common::x86_builtin_types::argument_scalar_types(name) {
            Some(types) => types.get(index).copied().flatten(),
            None => self
                .func_meta
                .sigs
                .get(name)
                .and_then(|signature| signature.param_ctypes.get(index))
                .filter(|ty| ty.is_integer() || ty.is_floating())
                .map(IrType::from_ctype),
        };
        match target {
            Some(target) => {
                let source = self.get_expr_type(arg);
                let value = self.lower_expr(arg);
                // Preserve constant immediates even at O0: a numeric C
                // conversion must not turn a valid immediate into an SSA value.
                let folded = match &value {
                    Operand::Const(IrConst::F32(value)) => {
                        IrConst::cast_float_to_target(*value as f64, target)
                    }
                    Operand::Const(IrConst::F64(value)) => {
                        IrConst::cast_float_to_target(*value, target)
                    }
                    Operand::Const(IrConst::LongDouble(_, bytes)) => {
                        IrConst::cast_long_double_to_target(bytes, target)
                    }
                    _ => None,
                };
                match folded {
                    Some(value) => Operand::Const(value),
                    None => self.emit_implicit_cast(value, source, target),
                }
            }
            // No redundant type inference or signature-map probe for native
            // vector/address operands already described by canonical metadata.
            None => self.lower_expr(arg),
        }
    }

    fn lower_x86_intrinsic(
        &mut self,
        intrinsic: &BuiltinIntrinsic,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        // AUD-SILENT-ZERO-1: an x86-only intrinsic compiled for a foreign
        // target used to lower to a silently zeroed result in the ARM/RISC-V
        // backends -- a wrong answer instead of a rejection.  GCC rejects the
        // builtin outright on those targets; so do we, at the one place every
        // x86 intrinsic passes through.
        if !matches!(
            self.target,
            crate::backend::Target::X86_64 | crate::backend::Target::I686
        ) {
            let arch = match self.target {
                crate::backend::Target::Aarch64 => "aarch64",
                crate::backend::Target::Riscv64 => "riscv64",
                other => other.triple(),
            };
            return self.reject_builtin_call(
                format!("'{}' is not available on {}", name, arch),
                args,
                16,
            );
        }
        if self.target == crate::backend::Target::I686
            && crate::common::x86_builtin_types::requires_64_bit(name)
        {
            return self.reject_builtin_call(
                format!("'{name}' is only available in 64-bit x86 mode"),
                args,
                16,
            );
        }
        // Native packed arithmetic belongs to the ordinary typed Vec* /
        // vecSSA pipeline, rather than address-only legacy temporaries.
        // Require the actual lane type and width: raw GNU arguments with a
        // different register image must retain the legacy bit interpretation.
        use crate::frontend::parser::ast::BinOp;
        let packed = match intrinsic {
            BuiltinIntrinsic::X86AddPs => Some((BinOp::Add, CType::Float, 16)),
            BuiltinIntrinsic::X86SubPs => Some((BinOp::Sub, CType::Float, 16)),
            BuiltinIntrinsic::X86MulPs => Some((BinOp::Mul, CType::Float, 16)),
            BuiltinIntrinsic::X86AddPd => Some((BinOp::Add, CType::Double, 16)),
            BuiltinIntrinsic::X86SubPd => Some((BinOp::Sub, CType::Double, 16)),
            BuiltinIntrinsic::X86MulPd => Some((BinOp::Mul, CType::Double, 16)),
            BuiltinIntrinsic::X86AddPs256 => Some((BinOp::Add, CType::Float, 32)),
            BuiltinIntrinsic::X86SubPs256 => Some((BinOp::Sub, CType::Float, 32)),
            BuiltinIntrinsic::X86MulPs256 => Some((BinOp::Mul, CType::Float, 32)),
            BuiltinIntrinsic::X86AddPd256 => Some((BinOp::Add, CType::Double, 32)),
            BuiltinIntrinsic::X86SubPd256 => Some((BinOp::Sub, CType::Double, 32)),
            BuiltinIntrinsic::X86MulPd256 => Some((BinOp::Mul, CType::Double, 32)),
            _ => None,
        };
        if let (Some((binary, element, width)), [lhs, rhs]) = (packed, args) {
            if let Some(ct) = self.get_expr_ctype(lhs) {
                let same_lanes = |ty: &CType| {
                    ty.size() == width && ty.vector_info().is_some_and(|(e, _)| *e == element)
                };
                if same_lanes(&ct) && self.get_expr_ctype(rhs).is_some_and(|ty| same_lanes(&ty)) {
                    if let Some(result) = self.try_lower_vector_binary_simd(&binary, lhs, rhs, &ct)
                    {
                        return Some(result);
                    }
                }
            }
        }
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
                let arg_ops: Vec<Operand> = args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| self.lower_typed_x86_builtin_argument(name, arg, index))
                    .collect();
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
                let arg_ops: Vec<Operand> = args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| self.lower_typed_x86_builtin_argument(name, arg, index))
                    .collect();
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
                // GNU's byte-lane shift/align builtins take BIT counts;
                // the instruction IR and public _mm APIs take byte counts.
                let bit_count = name.starts_with("__builtin_ia32_")
                    && matches!(
                        intrinsic,
                        BuiltinIntrinsic::X86Pslldqi128
                            | BuiltinIntrinsic::X86Psrldqi128
                            | BuiltinIntrinsic::X86Pslldqi256
                            | BuiltinIntrinsic::X86Psrldqi256
                            | BuiltinIntrinsic::X86Palignr128
                    );
                let normalized_count = if bit_count {
                    let count = args
                        .last()
                        .and_then(|arg| self.eval_const_expr(arg))
                        .and_then(|value| match value {
                            IrConst::F32(value) => {
                                IrConst::cast_float_to_target(value as f64, IrType::I32)
                            }
                            IrConst::F64(value) => {
                                IrConst::cast_float_to_target(value, IrType::I32)
                            }
                            IrConst::LongDouble(_, bytes) => {
                                IrConst::cast_long_double_to_target(&bytes, IrType::I32)
                            }
                            other => Some(other.narrowed_to(IrType::I32)),
                        })
                        .and_then(|value| value.to_i64());
                    let Some(count) =
                        count.filter(|count| (0..=2040).contains(count) && count % 8 == 0)
                    else {
                        return self.reject_builtin_call(
                            format!("'{name}' requires a constant bit count in 0..2040 divisible by eight"),
                            args,
                            16,
                        );
                    };
                    Some(count / 8)
                } else {
                    None
                };
                let mut arg_ops: Vec<Operand> = args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| self.lower_typed_x86_builtin_argument(name, arg, index))
                    .collect();
                if let (Some(count), Some(last)) = (normalized_count, arg_ops.last_mut()) {
                    *last = Operand::Const(IrConst::I64(count));
                }
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
            X86IntrinsicKind::Scalar => {
                let arg_ops: Vec<Operand> = args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| self.lower_typed_x86_builtin_argument(name, arg, index))
                    .collect();
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

/// One `__builtin_prefetch` hint argument after constant evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefetchArg {
    /// An integer constant expression with this value (wide enough that no
    /// out-of-range value can wrap into the valid domain).
    Value(i128),
    /// Not an integer constant expression (a variable, a float, ...).
    NotConstant,
}

/// A diagnostic produced while decoding the hint arguments, worded exactly
/// like GCC 16 so build logs and `-Werror` behave identically.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PrefetchDiag {
    NotConstant(&'static str),
    OutOfRange(&'static str),
}

/// GCC 16 `expand_builtin_prefetch`: map the optional `rw` and `locality`
/// arguments onto the [`PrefetchHint`] payload, plus the diagnostics to emit
/// (paired with the index of the offending call argument).
///
/// * absent: `rw = 0`, `locality = 3`;
/// * not an integer constant: an error, and the argument is treated as 0;
/// * `rw` outside 0..=2 or `locality` outside 0..=3: a warning, and the
///   argument is treated as 0 (GCC's "using zero" — NOT the default 3);
/// * `rw == 2` (instruction-fetch intent, GCC 15+) is a read hint.
fn prefetch_hint(
    rw: Option<PrefetchArg>,
    locality: Option<PrefetchArg>,
) -> (PrefetchHint, Vec<(PrefetchDiag, usize)>) {
    let mut diags = Vec::new();
    let rw = match rw {
        None => 0,
        Some(PrefetchArg::NotConstant) => {
            diags.push((
                PrefetchDiag::NotConstant(
                    "second argument to '__builtin_prefetch' must be a constant",
                ),
                1,
            ));
            0
        }
        Some(PrefetchArg::Value(v @ 0..=2)) => v,
        Some(PrefetchArg::Value(_)) => {
            diags.push((
                PrefetchDiag::OutOfRange(
                    "invalid second argument to '__builtin_prefetch'; using zero",
                ),
                1,
            ));
            0
        }
    };
    let locality = match locality {
        None => 3,
        Some(PrefetchArg::NotConstant) => {
            diags.push((
                PrefetchDiag::NotConstant(
                    "third argument to '__builtin_prefetch' must be a constant",
                ),
                2,
            ));
            0
        }
        Some(PrefetchArg::Value(v @ 0..=3)) => v as u8,
        Some(PrefetchArg::Value(_)) => {
            diags.push((
                PrefetchDiag::OutOfRange(
                    "invalid third argument to '__builtin_prefetch'; using zero",
                ),
                2,
            ));
            0
        }
    };
    (PrefetchHint::new(rw == 1, locality), diags)
}

impl Lowerer {
    /// Constant-evaluate one hint argument. Any integer constant expression
    /// counts (`enum` constants, `sizeof`, parenthesised arithmetic), exactly
    /// like GCC; a float or a runtime value is `NotConstant`.
    fn prefetch_hint_arg(&self, expr: &Expr) -> PrefetchArg {
        match self.eval_const_expr(expr) {
            Some(IrConst::I128(v)) => PrefetchArg::Value(v),
            Some(c) => c
                .to_i64()
                .map_or(PrefetchArg::NotConstant, |v| PrefetchArg::Value(v as i128)),
            None => PrefetchArg::NotConstant,
        }
    }
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
        | BuiltinIntrinsic::X86Cvtsd2Si64
        | BuiltinIntrinsic::X86Cvtss2Si64
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
        // Native C vectors share object-address IR carriers on both x86 modes.
        BuiltinIntrinsic::X86MaxPs128 | BuiltinIntrinsic::X86MinPs128 => X86IntrinsicKind::Vec128,
        BuiltinIntrinsic::X86ShufPsValue => X86IntrinsicKind::Vec128,
        BuiltinIntrinsic::X86CvtPd2PsValue => X86IntrinsicKind::Vec128,
        BuiltinIntrinsic::X86Vextractf128V => X86IntrinsicKind::Vec128,
        BuiltinIntrinsic::X86MaxPs256V
        | BuiltinIntrinsic::X86MinPs256V
        | BuiltinIntrinsic::X86AndPs256V
        | BuiltinIntrinsic::X86CmpPs256V => X86IntrinsicKind::Vec256,
        _ => X86IntrinsicKind::Vec128,
    }
}

/// One row of the AUD-CPU-3 permission table: what an intrinsic's instruction
/// needs before the invocation may contain it.
struct IntrinsicRequirement {
    /// The feature whose bit in [`crate::common::x86_features::effective`] is
    /// the permission (the fold for a conjunction is inside the fold, so one
    /// lookup is enough).
    feature: crate::common::x86_features::X86Feature,
    /// The `-m` flag that grants `feature`, for the diagnostic.
    flag: &'static str,
    /// A conjunct the feature's own bit does not spell out.  The 256-bit
    /// VAES/VPCLMULQDQ forms are the case: their permission is the extension
    /// *and* the 256-bit vector class, and a user who passed `-mvaes` on a
    /// target without AVX2 needs to be told the second half, not the first.
    also: Option<&'static str>,
}

// Direct calls to these builtins are lowered at the call site, never by
// calling the C wrapper body. Address-taken wrappers still need their body.
pub(super) fn is_feature_checked_x86_builtin(name: &str) -> bool {
    matches!(builtins::resolve_builtin(name).map(|info| &info.kind),
        Some(BuiltinKind::Intrinsic(intrinsic)) if x86_intrinsic_required_feature(intrinsic).is_some())
}

/// SSE1 and SSE2 share XMM carriers but must retain independent permissions.
fn legacy_xmm_intrinsic_required_feature(
    intrinsic: &BuiltinIntrinsic,
) -> Option<IntrinsicRequirement> {
    use crate::common::x86_features::X86Feature as F;
    use BuiltinIntrinsic as B;
    Some(match intrinsic {
        // Public and raw spellings share an intrinsic identity. SSE1 floating
        // arithmetic must not bypass permission checks, nor imply SSE2 integer
        // or double arithmetic merely because the register class is the same.
        B::X86SetzeroPs128
        | B::X86XorPs
        | B::X86AndPs
        | B::X86OrPs
        | B::X86AndnPs
        | B::X86AddPs
        | B::X86SubPs
        | B::X86MulPs
        | B::X86AddSs
        | B::X86SubSs
        | B::X86MulSs
        | B::X86DivSs
        | B::X86MinSs
        | B::X86MaxSs
        | B::X86SqrtSs
        | B::X86MaxPs128
        | B::X86MinPs128
        | B::X86ShufPsValue
        | B::X86Cvtss2Si64 => IntrinsicRequirement {
            feature: F::Sse,
            flag: "-msse",
            also: None,
        },
        // Legacy XMM integer, packed-double and scalar-double operations.
        // An SSE1-only ABI does not grant these SSE2 APIs / instructions.
        B::X86Setzero128
        | B::X86CastReinterpret
        | B::X86Lfence
        | B::X86Mfence
        | B::X86Movnti
        | B::X86Movnti64
        | B::X86Movntdq
        | B::X86Movntpd
        | B::X86Loaddqu
        | B::X86Storedqu
        | B::X86Pcmpeqb128
        | B::X86Pcmpeqd128
        | B::X86Psubusb128
        | B::X86Psubsb128
        | B::X86Psubusw128
        | B::X86Por128
        | B::X86Pand128
        | B::X86Pxor128
        | B::X86Pmovmskb128
        | B::X86Set1Epi8
        | B::X86Set1Epi32
        | B::X86Pshufd128
        | B::X86Loadldi128
        | B::X86Paddw128
        | B::X86Psubw128
        | B::X86Pmulhw128
        | B::X86Pmaddwd128
        | B::X86Pcmpgtw128
        | B::X86Pcmpgtb128
        | B::X86Paddd128
        | B::X86Psubd128
        | B::X86Packssdw128
        | B::X86Packsswb128
        | B::X86Packuswb128
        | B::X86Punpcklbw128
        | B::X86Punpckhbw128
        | B::X86Punpcklwd128
        | B::X86Punpckhwd128
        | B::X86Set1Epi16
        | B::X86Pinsrw128
        | B::X86Pextrw128
        | B::X86Storeldi128
        | B::X86Cvtsi128Si32
        | B::X86Cvtsi32Si128
        | B::X86Cvtsi128Si64
        | B::X86Cvtsd2Si64
        | B::X86Pshuflw128
        | B::X86Pshufhw128
        | B::X86Paddb128
        | B::X86Psubb128
        | B::X86Psadbw128
        | B::X86Pmullw128
        | B::X86Pmaxub128
        | B::X86Pminub128
        | B::X86CvtPd2PsValue
        | B::X86Psllw128
        | B::X86Psrlw128
        | B::X86XorPd
        | B::X86AndPd
        | B::X86OrPd
        | B::X86AndnPd
        | B::X86AddPd
        | B::X86SubPd
        | B::X86MulPd
        | B::X86AddSd
        | B::X86SubSd
        | B::X86MulSd
        | B::X86DivSd
        | B::X86MinSd
        | B::X86MaxSd
        | B::X86SqrtSd
        | B::X86MulEpu32
        | B::X86Paddusb128
        | B::X86Paddsb128
        | B::X86Paddusw128
        | B::X86Paddsw128
        | B::X86Psubsw128
        | B::X86Pandn128
        | B::X86Pcmpeqw128
        | B::X86Pcmpgtd128
        | B::X86Pavgb128
        | B::X86Pavgw128
        | B::X86Pminsw128
        | B::X86Pmaxsw128
        | B::X86Pmulhuw128
        | B::X86Paddq128
        | B::X86Psubq128
        | B::X86Punpckldq128
        | B::X86Punpckhdq128
        | B::X86Punpcklqdq128
        | B::X86Punpckhqdq128
        | B::X86Pslldqi128
        | B::X86Psrldqi128
        | B::X86Psllqi128
        | B::X86Psrlqi128
        | B::X86Psllwi128
        | B::X86Psrlwi128
        | B::X86Psrawi128
        | B::X86Psradi128
        | B::X86Pslldi128
        | B::X86Psrldi128 => IntrinsicRequirement {
            feature: F::Sse2,
            flag: "-msse2",
            also: None,
        },
        _ => return None,
    })
}

/// Map a BuiltinIntrinsic to its corresponding IntrinsicOp.
/// AUD-CPU-3: the instruction-set extension an x86 intrinsic's instruction
/// belongs to, for the families whose permission bit exists in
/// [`crate::common::x86_features`].
///
/// Only *unambiguous* families are listed.  The 256-bit AES/CLMUL forms are
/// VAES/VPCLMULQDQ and the three dot-product groups are AVX-VNNI,
/// AVX-VNNI-INT8 and AVX-VNNI-INT16, whose permissions are the conjunction the
/// fold already encodes (AVX class AND the extension's own request),
/// so a single lookup covers both halves; the SHA/GFNI/AVX-512 families still
/// have no bit -- see the follow-up document's AUD-CPU-3 entry.
fn x86_intrinsic_required_feature(intrinsic: &BuiltinIntrinsic) -> Option<IntrinsicRequirement> {
    use crate::common::x86_features::X86Feature as F;
    use BuiltinIntrinsic as B;
    if let Some(requirement) = legacy_xmm_intrinsic_required_feature(intrinsic) {
        return Some(requirement);
    }
    Some(match intrinsic {
        // Packed floating-point YMM operations are AVX, not AVX2, on both
        // x86 ABIs. Never let an absent permission fall through to emission.
        B::X86Setzero256
        | B::X86AddPs256
        | B::X86SubPs256
        | B::X86MulPs256
        | B::X86AddPd256
        | B::X86SubPd256
        | B::X86MulPd256
        | B::X86LoaduPs256
        | B::X86StoreuPs256
        | B::X86LoaduPd256
        | B::X86StoreuPd256
        | B::X86MaxPs256V
        | B::X86MinPs256V
        | B::X86AndPs256V
        | B::X86CmpPs256V => IntrinsicRequirement {
            feature: F::Avx,
            flag: "-mavx",
            also: None,
        },
        B::X86Paddb256
        | B::X86Paddw256
        | B::X86Paddd256
        | B::X86Paddq256
        | B::X86Psubb256
        | B::X86Psubw256
        | B::X86Psubd256
        | B::X86Psubq256
        | B::X86Pmulld256
        | B::X86Pmullw256
        | B::X86Pmulhw256
        | B::X86Pmuludq256
        | B::X86Psllidi256
        | B::X86Psrlidi256
        | B::X86Psllwi256
        | B::X86Psrlwi256
        | B::X86Psllqi256
        | B::X86Psrlqi256
        | B::X86Psrawi256
        | B::X86Psradi256
        | B::X86Pslldqi256
        | B::X86Psrldqi256 => IntrinsicRequirement {
            feature: F::Avx2,
            flag: "-mavx2",
            also: None,
        },
        B::X86AvxWiden(_) => IntrinsicRequirement {
            feature: F::Avx2,
            flag: "-mavx2",
            also: None,
        },
        B::X86AvxTest(_) => IntrinsicRequirement {
            feature: F::Avx,
            flag: "-mavx",
            also: None,
        },
        // Public headers must not silently introduce SSSE3 above a declared
        // SSE2 ceiling. Include the raw builtin aliases in this same gate.
        B::X86Pmaddubsw128
        | B::X86Phaddw128
        | B::X86Phaddd128
        | B::X86Phsubw128
        | B::X86Phsubd128
        | B::X86Phaddsw128
        | B::X86Phsubsw128
        | B::X86Psignb128
        | B::X86Psignw128
        | B::X86Psignd128
        | B::X86Pmulhrsw128
        | B::X86Pshufb128
        | B::X86Pabsb128
        | B::X86Pabsw128
        | B::X86Pabsd128
        | B::X86Palignr128 => IntrinsicRequirement {
            feature: F::Ssse3,
            flag: "-mssse3",
            also: None,
        },
        B::X86Pinsrd128
        | B::X86Pextrd128
        | B::X86Pinsrb128
        | B::X86Pextrb128
        | B::X86Pinsrq128
        | B::X86Pextrq128
        | B::X86Pblendvb128
        | B::X86Pblendw128
        | B::X86Pmovzxbw128
        | B::X86Pmovzxwd128
        | B::X86MulEpi32
        | B::X86MulloEpi32
        | B::X86Testz128 => IntrinsicRequirement {
            feature: F::Sse41,
            flag: "-msse4.1",
            also: None,
        },
        // SSE4.2 introduced CRC32: `_mm_crc32_u8` and its width siblings.
        B::X86Crc32_8 | B::X86Crc32_16 | B::X86Crc32_32 | B::X86Crc32_64 => IntrinsicRequirement {
            feature: F::Sse42,
            flag: "-msse4.2",
            also: None,
        },
        // BMI2 BZHI (`_bzhi_u32`/`_bzhi_u64`).
        B::X86Bzhi32 | B::X86Bzhi64 => IntrinsicRequirement {
            feature: F::Bmi2,
            flag: "-mbmi2",
            also: None,
        },
        // AES-NI, 128-bit forms (the `_mm_*`/`__builtin_ia32_*` spellings that
        // map onto the legacy-encoded instructions).
        B::X86Aesenc128
        | B::X86Aesenclast128
        | B::X86Aesdec128
        | B::X86Aesdeclast128
        | B::X86Aesimc128
        | B::X86Aeskeygenassist128 => IntrinsicRequirement {
            feature: F::Aes,
            flag: "-maes",
            also: None,
        },
        B::X86Pclmulqdq128 => IntrinsicRequirement {
            feature: F::Pclmul,
            flag: "-mpclmul",
            also: None,
        },
        // VAES: the EVEX 256-bit AES round instructions.  A v3 target without
        // `-mvaes` is exactly the SIGILL case `_mm_crc32_u8` under
        // `-march=x86-64` was: measured emitting `vaesenc %ymm1, %ymm0, %ymm0`
        // on a target whose own class does not contain it.
        B::X86Aesenc256 | B::X86Aesenclast256 | B::X86Aesdec256 | B::X86Aesdeclast256 => {
            IntrinsicRequirement {
                feature: F::Vaes,
                flag: "-mvaes",
                also: Some(" and AVX (VEX.256)"),
            }
        }
        // VPCLMULQDQ: the 256-bit carry-less multiply.  Separate bit from
        // VAES, so the two must not share a row.  Both 256-bit forms need AVX
        // only, not AVX2 (measured on gcc 14: `-mvaes -mno-avx2` accepts).
        B::X86Vpclmulqdq256 => IntrinsicRequirement {
            feature: F::Vpclmulqdq,
            flag: "-mvpclmulqdq",
            also: Some(" and AVX (VEX.256)"),
        },
        // AVX-VNNI: `vpdpbusd(s)` and `vpdpwssd(s)`, the VEX-encoded
        // signed-byte-times-unsigned-byte and signed-word dot products.  This
        // is the family the audit's live SIGILL case belongs to: measured
        // emitting `vpdpbusd %ymm1, %ymm0, %ymm0` for `_mm_dpbusd_epi32`
        // under `-march=x86-64`, i.e. an Alder-Lake-era instruction inside a
        // v1 object.
        B::X86Dpbusd128
        | B::X86Dpbusds128
        | B::X86Dpwssd128
        | B::X86Dpwssds128
        | B::X86Dpbusd256
        | B::X86Dpbusds256
        | B::X86Dpwssd256
        | B::X86Dpwssds256 => IntrinsicRequirement {
            feature: F::AvxVnni,
            flag: "-mavxvnni",
            also: Some(" and a 256-bit vector class (AVX2 or AVX-512)"),
        },
        // AVX-VNNI-INT8: the byte forms where either input may be signed.
        // A separate extension with its own flag; `-mavxvnni` does not grant
        // it, and the two must not share a row.
        B::X86Dpbssd128
        | B::X86Dpbssds128
        | B::X86Dpbsud128
        | B::X86Dpbsuds128
        | B::X86Dpbuud128
        | B::X86Dpbuuds128
        | B::X86Dpbssd256
        | B::X86Dpbssds256
        | B::X86Dpbsud256
        | B::X86Dpbsuds256
        | B::X86Dpbuud256
        | B::X86Dpbuuds256 => IntrinsicRequirement {
            feature: F::AvxVnniInt8,
            flag: "-mavxvnniint8",
            also: Some(" and a 256-bit vector class (AVX2 or AVX-512)"),
        },
        // GFNI: the Galois-field byte instructions.  Legacy-encoded, so the
        // requirement is the extension's flag (plus the xmm class every x86-64
        // target has); `-mno-avx` deliberately does not appear in the
        // conjunction, because GCC does not require AVX either.
        B::X86Gf2p8mulb128 | B::X86Gf2p8affineqb128 | B::X86Gf2p8affineinvqb128 => {
            IntrinsicRequirement {
                feature: F::Gfni,
                flag: "-mgfni",
                also: None,
            }
        }
        // AVX-VNNI-INT16: the unsigned-word forms.
        B::X86Dpwusd128
        | B::X86Dpwusds128
        | B::X86Dpwuud128
        | B::X86Dpwuuds128
        | B::X86Dpwusd256
        | B::X86Dpwusds256
        | B::X86Dpwuud256
        | B::X86Dpwuuds256 => IntrinsicRequirement {
            feature: F::AvxVnniInt16,
            flag: "-mavxvnniint16",
            also: Some(" and a 256-bit vector class (AVX2 or AVX-512)"),
        },
        _ => return None,
    })
}

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
        // SSE float mappings: logical forms match pand/por/pxor bit-for-bit; add/sub/mul stay FP.
        BuiltinIntrinsic::X86XorPs => IntrinsicOp::XorPs128,
        BuiltinIntrinsic::X86AndPs => IntrinsicOp::AndPs128,
        BuiltinIntrinsic::X86OrPs => IntrinsicOp::OrPs128,
        BuiltinIntrinsic::X86AndnPs => IntrinsicOp::AndnPs128,
        BuiltinIntrinsic::X86XorPd => IntrinsicOp::XorPd128,
        BuiltinIntrinsic::X86AndPd => IntrinsicOp::AndPd128,
        BuiltinIntrinsic::X86OrPd => IntrinsicOp::OrPd128,
        BuiltinIntrinsic::X86AndnPd => IntrinsicOp::AndnPd128,
        BuiltinIntrinsic::X86AddPs => IntrinsicOp::AddPs128,
        BuiltinIntrinsic::X86SubPs => IntrinsicOp::SubPs128,
        BuiltinIntrinsic::X86MulPs => IntrinsicOp::MulPs128,
        BuiltinIntrinsic::X86AddPd => IntrinsicOp::AddPd128,
        BuiltinIntrinsic::X86AddSd => IntrinsicOp::AddSd128,
        BuiltinIntrinsic::X86SubSd => IntrinsicOp::SubSd128,
        BuiltinIntrinsic::X86MulSd => IntrinsicOp::MulSd128,
        BuiltinIntrinsic::X86DivSd => IntrinsicOp::DivSd128,
        BuiltinIntrinsic::X86MinSd => IntrinsicOp::MinSd128,
        BuiltinIntrinsic::X86MaxSd => IntrinsicOp::MaxSd128,
        BuiltinIntrinsic::X86SqrtSd => IntrinsicOp::SqrtSd128,
        BuiltinIntrinsic::X86AddSs => IntrinsicOp::AddSs128,
        BuiltinIntrinsic::X86SubSs => IntrinsicOp::SubSs128,
        BuiltinIntrinsic::X86MulSs => IntrinsicOp::MulSs128,
        BuiltinIntrinsic::X86DivSs => IntrinsicOp::DivSs128,
        BuiltinIntrinsic::X86MinSs => IntrinsicOp::MinSs128,
        BuiltinIntrinsic::X86MaxSs => IntrinsicOp::MaxSs128,
        BuiltinIntrinsic::X86SqrtSs => IntrinsicOp::SqrtSs128,
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
        BuiltinIntrinsic::X86Cvtsd2Si64 => IntrinsicOp::Cvtsd2Si64,
        BuiltinIntrinsic::X86Cvtss2Si64 => IntrinsicOp::Cvtss2Si64,
        BuiltinIntrinsic::X86Crc32_8 => IntrinsicOp::Crc32_8,
        BuiltinIntrinsic::X86Crc32_16 => IntrinsicOp::Crc32_16,
        BuiltinIntrinsic::X86Crc32_32 => IntrinsicOp::Crc32_32,
        BuiltinIntrinsic::X86Crc32_64 => IntrinsicOp::Crc32_64,
        BuiltinIntrinsic::X86Pinsrd128 => IntrinsicOp::Pinsrd128, // SSE4.1 insert/extract
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
        BuiltinIntrinsic::X86Phsubw128 => IntrinsicOp::Phsubw128,
        BuiltinIntrinsic::X86Phsubd128 => IntrinsicOp::Phsubd128,
        BuiltinIntrinsic::X86Phaddsw128 => IntrinsicOp::Phaddsw128,
        BuiltinIntrinsic::X86Phsubsw128 => IntrinsicOp::Phsubsw128,
        BuiltinIntrinsic::X86Psignb128 => IntrinsicOp::Psignb128,
        BuiltinIntrinsic::X86Psignw128 => IntrinsicOp::Psignw128,
        BuiltinIntrinsic::X86Psignd128 => IntrinsicOp::Psignd128,
        BuiltinIntrinsic::X86Pmulhrsw128 => IntrinsicOp::Pmulhrsw128,
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
        BuiltinIntrinsic::X86SetzeroPs128 => IntrinsicOp::SetzeroPs128,
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

/// The C library function a non-constant NaN builtin calls.
fn nan_library_function(name: &str) -> Option<&'static str> {
    Some(match name {
        "__builtin_nan" => "nan",
        "__builtin_nanf" => "nanf",
        "__builtin_nanl" => "nanl",
        "__builtin_nanf128" | "__builtin_nanq" => "nanf128",
        _ => return None,
    })
}

#[cfg(test)]
mod prefetch_hint_tests {
    //! `prefetch_hint` is the whole of `__builtin_prefetch`'s argument
    //! semantics: the payload it returns is what every backend's mnemonic
    //! table is indexed by. Every expectation below is GCC 16.2's measured
    //! behaviour (`gcc-16 -O2 -S`, x86-64 and `-m32 -msse`; AArch64 via the
    //! `pftype[rw & 1][locality]` table), including the diagnostics' text.
    use super::{PrefetchArg, PrefetchDiag, prefetch_hint};
    use crate::ir::reexports::PrefetchHint;

    fn v(x: i128) -> Option<PrefetchArg> {
        Some(PrefetchArg::Value(x))
    }

    #[test]
    fn defaults_are_read_with_high_locality() {
        assert_eq!(
            prefetch_hint(None, None),
            (PrefetchHint::new(false, 3), vec![])
        );
        assert_eq!(
            prefetch_hint(v(1), None),
            (PrefetchHint::new(true, 3), vec![])
        );
    }

    #[test]
    fn write_intent_keeps_its_locality() {
        // Defect 15: `rw = 1` used to collapse every locality onto one
        // payload, so `(p, 1, 0)` became an L1 hint instead of a streaming
        // one (GCC: `prefetchnta` / `PSTL1STRM`).
        for rw in 0..=1i128 {
            for loc in 0..=3i128 {
                let (hint, diags) = prefetch_hint(v(rw), v(loc));
                assert!(diags.is_empty());
                assert_eq!(hint, PrefetchHint::new(rw == 1, loc as u8));
                assert_eq!(PrefetchHint::decode(hint.encode()), hint);
            }
        }
    }

    #[test]
    fn instruction_fetch_intent_is_a_read_hint() {
        // GCC 15+ accepts `rw = 2` without a diagnostic and lowers it with the
        // read table on every target.
        for loc in 0..=3i128 {
            assert_eq!(
                prefetch_hint(v(2), v(loc)),
                (PrefetchHint::new(false, loc as u8), vec![])
            );
        }
    }

    #[test]
    fn out_of_range_values_warn_and_use_zero() {
        let rw_msg =
            PrefetchDiag::OutOfRange("invalid second argument to '__builtin_prefetch'; using zero");
        let loc_msg =
            PrefetchDiag::OutOfRange("invalid third argument to '__builtin_prefetch'; using zero");
        // (p, -1, 3): GCC warns once and emits `prefetcht0`.
        assert_eq!(
            prefetch_hint(v(-1), v(3)),
            (PrefetchHint::new(false, 3), vec![(rw_msg.clone(), 1)])
        );
        // (p, 1, 9): locality becomes ZERO, not the default 3 (`prefetchnta`).
        assert_eq!(
            prefetch_hint(v(1), v(9)),
            (PrefetchHint::new(true, 0), vec![(loc_msg.clone(), 2)])
        );
        // A value that would wrap into range at 64 bits stays out of range.
        assert_eq!(
            prefetch_hint(v((1i128 << 64) + 1), v(-1)),
            (PrefetchHint::new(false, 0), vec![(rw_msg, 1), (loc_msg, 2)])
        );
    }

    #[test]
    fn non_constant_arguments_are_errors() {
        let (hint, diags) = prefetch_hint(
            Some(PrefetchArg::NotConstant),
            Some(PrefetchArg::NotConstant),
        );
        assert_eq!(hint, PrefetchHint::new(false, 0));
        assert_eq!(
            diags,
            vec![
                (
                    PrefetchDiag::NotConstant(
                        "second argument to '__builtin_prefetch' must be a constant"
                    ),
                    1
                ),
                (
                    PrefetchDiag::NotConstant(
                        "third argument to '__builtin_prefetch' must be a constant"
                    ),
                    2
                ),
            ]
        );
    }

    #[test]
    fn every_payload_decodes_to_a_valid_hint() {
        for p in 0u8..=255 {
            let h = PrefetchHint::decode(p);
            assert!(h.locality <= 3);
            assert_eq!(h.encode(), p & 7);
        }
    }
}

#[cfg(test)]
mod legacy_constructor_permission_tests {
    use super::{BuiltinKind, builtins, x86_intrinsic_required_feature};
    use crate::common::x86_features::X86Feature as F;

    #[test]
    fn zero_constructors_keep_their_api_feature_identity() {
        for (name, expected) in [
            ("_mm_setzero_ps", F::Sse),
            ("_mm_setzero_pd", F::Sse2),
            ("_mm_setzero_si128", F::Sse2),
            ("_mm256_setzero_ps", F::Avx),
            ("_mm256_setzero_pd", F::Avx),
            ("_mm256_setzero_si256", F::Avx),
        ] {
            let info = builtins::resolve_builtin(name).expect("known zero constructor");
            let BuiltinKind::Intrinsic(intrinsic) = &info.kind else {
                panic!("{name} is not an intrinsic");
            };
            assert_eq!(
                x86_intrinsic_required_feature(intrinsic).map(|r| r.feature),
                Some(expected),
                "{name}",
            );
        }
    }
}
