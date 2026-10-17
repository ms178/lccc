//! Function call lowering: argument preparation, call dispatch, and result handling.
//!
//! Extracted from expr.rs. This module handles:
//! - `lower_function_call`: the main entry point for FunctionCall expressions
//! - `lower_call_arguments`: argument evaluation with implicit casts and promotions
//! - `emit_call_instruction`: dispatch to direct, function-pointer, or indirect calls
//! - `classify_struct_return`: shared sret/two-reg classification logic
//! - Helpers: maybe_narrow_call_result, is_function_variadic, get_func_ptr_return_ir_type

use super::lower::Lowerer;
use crate::common::types::{AddressSpace, CType, IrType, target_int_ir_type};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{CallInfo, Instruction, IrBinOp, IrConst, Operand, Terminator, Value};

impl Lowerer {
    /// Classify a struct return size into sret (hidden pointer) or two-register return.
    /// Returns (sret_size, two_reg_size).
    ///
    /// On x86-64/arm/riscv (64-bit targets):
    ///   - size > 16: sret (hidden pointer)
    ///   - 9-16 bytes: two-register return (packed into I128, returned in rax:rdx / x0:x1 / a0:a1)
    ///   - 1-8 bytes: small struct (packed into I64, returned in rax / x0 / a0)
    ///
    /// On i686 (32-bit target):
    ///   - ALL structs use sret (hidden pointer) regardless of size.
    ///     The i386 System V ABI always returns structs via a caller-provided
    ///     hidden pointer (first stack argument), with `ret $4` to pop it.
    pub(super) fn classify_struct_return(size: usize) -> (Option<usize>, Option<usize>) {
        let ptr_size = crate::common::types::target_ptr_size();
        if ptr_size <= 4 {
            // 32-bit target (i686): i386 SysV ABI always uses sret for structs.
            (Some(size), None)
        } else {
            // 64-bit target: register pair is 16 bytes
            if size > 16 {
                (Some(size), None)
            } else if size > 8 {
                (None, Some(size))
            } else {
                (None, None)
            }
        }
    }

    /// Whether a call through the function-pointer variable `name` (the callee
    /// expression `callee`) is variadic: the declared signature when the
    /// declarator spelled one, else the pointed-to function type of the
    /// variable's `c_type` (typedef-declared pointers such as
    /// `typedef int (*pf)(const char *, ...); void g(pf f)` carry `...` only
    /// there; missing it drops AL on x86-64 and misplaces FP varargs).
    pub(super) fn fptr_variable_is_variadic(&self, name: &str, callee: &Expr) -> bool {
        if let Some(sig) = self.fptr_variable_sig(name) {
            return sig.is_variadic;
        }
        self.get_expr_ctype(callee)
            .and_then(|ct| ct.get_function_type().map(|ft| ft.variadic))
            .unwrap_or(false)
    }

    /// Check if an identifier is a function pointer variable rather than a direct function.
    /// A local variable always shadows a known function of the same name (e.g., a parameter
    /// named `free` that is a function pointer should produce an indirect call, not a direct
    /// call to the libc `free` symbol).
    pub(super) fn is_func_ptr_variable(&self, name: &str) -> bool {
        // When called outside a function context (e.g., during global constant
        // evaluation), there are no locals, so this is not a func ptr variable.
        let func_state = match self.func_state.as_ref() {
            Some(fs) => fs,
            None => return self.globals.contains_key(name) && !self.known_functions.contains(name),
        };
        if func_state.locals.contains_key(name) {
            // Local variable exists with this name. Check if it's a function pointer
            // via its declared signature (LocalInfo::fptr_sig) or c_type.
            if self.local_fptr_sig(name).is_some() {
                return true;
            }
            // Also check the local's CType (typedef-declared function pointers carry no fptr_sig)
            if let Some(local_info) = func_state.locals.get(name) {
                if let Some(ref cty) = local_info.c_type {
                    if cty.is_function_pointer() {
                        return true;
                    }
                }
            }
            // Local exists but is not a function pointer - it's a regular variable
            // that happens to shadow a known function name. Not a func ptr variable.
            false
        } else {
            // No local with this name. Check globals, excluding known functions.
            self.globals.contains_key(name) && !self.known_functions.contains(name)
        }
    }

    pub(super) fn lower_function_call(&mut self, func: &Expr, args: &[Expr]) -> Operand {
        // Strip Deref layers that are semantically no-ops.
        // In C, dereferencing a function pointer is a no-op: (*fp)() == fp().
        // But dereferencing a pointer-to-function-pointer is a real load:
        // (*fpp)() where fpp is int (**)(int,int) must load the function pointer.
        // Only strip a Deref if the inner expression is itself a function pointer
        // or function designator (checked via is_function_pointer_deref).
        let mut stripped_func = func;
        while let Expr::Deref(inner, _) = stripped_func {
            if self.is_function_pointer_deref(inner) {
                stripped_func = inner;
            } else {
                break;
            }
        }

        // Strip AddressOf wrapping a function name: (&func)(args) => func(args).
        // In C, taking the address of a function and calling through it is equivalent
        // to calling the function directly. The kernel's static_call mechanism uses
        // this pattern: ({ ...; (&__SCT__trampoline); })(args) and requires a direct
        // call instruction so the call site can be patched at runtime.
        if let Expr::AddressOf(inner, _) = stripped_func {
            if let Expr::Identifier(name, _) = inner.as_ref() {
                if !self.is_func_ptr_variable(name) {
                    stripped_func = inner;
                }
            }
        }

        // GNU C nested function call: if the identifier names a visible
        // nested function, route through the static-chain call path.
        if let Expr::Identifier(name, span) = stripped_func {
            if self.lookup_nested_function(name).is_some() {
                return self
                    .try_lower_nested_call(name, args, *span)
                    .unwrap_or(Operand::Const(IrConst::ptr_int(0)));
            }
        }

        // Resolve _Generic selections to the selected expression.
        // Without this, _Generic(expr, type1: func1, type2: func2)(args) would
        // fall through to the catch-all indirect call path, which dereferences
        // the function address instead of calling directly.
        if let Expr::GenericSelection(controlling, associations, _) = stripped_func {
            stripped_func = self.resolve_generic_selection(controlling, associations);
        }
        // Use the resolved expression for emit_call_instruction too
        let effective_func = stripped_func;

        // Resolve __builtin_* functions and their attribute-driven aliases
        // first — unless the identifier names a local binding (which shadows
        // the library name, C11 6.2.1p4).
        if let Some(folded) = self.try_fold_builtin_or_attr_call(stripped_func, args) {
            return folded;
        }

        // Clang's i686 integer ext-vectors use the GP return words stored in
        // the padded IR carrier; retain the CType to unpack those words after
        // the call and pass the third word through the backend call hook.
        let mut call_return_ct = if let Expr::Identifier(name, _) = stripped_func {
            if self.is_func_ptr_variable(name) {
                self.fptr_variable_sig(name)
                    .and_then(|sig| sig.return_ctype.clone())
            } else {
                self.func_meta
                    .sigs
                    .get(name.as_str())
                    .and_then(|sig| sig.return_ctype.clone())
            }
        } else {
            None
        };
        if call_return_ct.is_none() {
            call_return_ct = self.call_return_ctype(effective_func);
        }
        let i686_ext_vector_gp_return_ctype = if crate::common::types::target_is_32bit() {
            call_return_ct.filter(|ct| ct.i686_ext_vector_gp_word_count().is_some())
        } else {
            None
        };
        let i686_ext_vector_gp_return_words = i686_ext_vector_gp_return_ctype
            .as_ref()
            .and_then(CType::i686_ext_vector_gp_word_count)
            .unwrap_or(0);

        // Determine sret/two-reg return convention
        // `f128_sret_ret`: the callee returns a _Float128/_Decimal128 through
        // the i686 hidden-pointer convention — the call result is the sret
        // buffer ADDRESS, and the 16-byte carrier value must be loaded out of
        // it (see the sret result handling below).
        let mut f128_sret_ret = false;
        // AUD-VABI-1: the indirect-call paths below have no FuncSig to read, so
        // the "ONE 16-byte %xmm" carrier for a vector-returning indirect call
        // has to come from the callee expression's own function type.  Without
        // it `pick_impl(0)(a, b, c)` read the result from %rdx:%rax while the
        // callee (whatever compiled it) had written %xmm0.
        let indirect_vector_carrier = if crate::common::types::target_elf_machine() == 62 {
            self.call_return_ctype(effective_func)
                .and_then(|ct| ct.sysv_vector_carrier())
        } else {
            None
        };
        let (indirect_ret_classes, indirect_ret_is_one_xmm) = match &indirect_vector_carrier {
            Some((classes, one_xmm)) => (classes.clone(), *one_xmm),
            None => (Vec::new(), false),
        };
        let (sret_size, two_reg_size, mut call_ret_classes) =
            if let Expr::Identifier(name, _) = stripped_func {
                if self.is_func_ptr_variable(name) {
                    // Indirect call through function pointer variable
                    // TODO: compute ret_eightbyte_classes from the function pointer's
                    // return type to support mixed SSE/INTEGER struct returns via fptrs
                    match self.get_call_return_struct_size(effective_func) {
                        Some(size) => {
                            let (s, t) = Self::classify_struct_return(size);
                            (s, t, indirect_ret_classes.clone())
                        }
                        None => (None, None, Vec::new()),
                    }
                } else {
                    // Direct function call - look up by function name
                    let sig = self.func_meta.sigs.get(name.as_str());
                    f128_sret_ret = sig.is_some_and(|s| s.ret_is_f128_sse && s.sret_size.is_some());
                    (
                        sig.and_then(|s| s.sret_size),
                        sig.and_then(|s| s.two_reg_ret_size),
                        sig.map(|s| s.ret_eightbyte_classes.clone())
                            .unwrap_or_default(),
                    )
                }
            } else {
                // Non-identifier function expression (e.g., array[i]())
                // TODO: compute ret_eightbyte_classes from expression return type
                // to support mixed SSE/INTEGER struct returns via indirect calls
                match self.get_call_return_struct_size(effective_func) {
                    Some(size) => {
                        let (s, t) = Self::classify_struct_return(size);
                        (s, t, indirect_ret_classes.clone())
                    }
                    None => (None, None, Vec::new()),
                }
            };
        if i686_ext_vector_gp_return_words == 3 {
            call_ret_classes = vec![
                crate::common::types::EightbyteClass::Integer,
                crate::common::types::EightbyteClass::Integer,
                crate::common::types::EightbyteClass::Integer,
            ];
        }

        // Lower arguments with implicit casts
        let (
            mut arg_vals,
            mut arg_types,
            mut struct_arg_sizes,
            mut struct_arg_aligns,
            mut struct_arg_classes,
            mut struct_arg_riscv_float_classes,
        ) = self.lower_call_arguments(effective_func, args);

        // Detect variadic status early (needed for complex arg decomposition)
        let call_is_variadic = if let Expr::Identifier(name, _) = stripped_func {
            if self.is_func_ptr_variable(name) {
                // Call through a function-pointer variable: the POINTED-TO
                // signature decides.  Its `...` makes the call variadic —
                // floats belong in GP regs on rv64, AL=1 on x86-64, GP-only
                // on aarch64.  is_function_variadic only knows function
                // definitions, so `int (*fp)(char*, ...) -> f(fp);
                // (*fp)(b, "%.0f", 5.0)` sent the double to fa0 instead of
                // a2 and aborted glibc's vsprintf (gcc.c-torture 930513-1).
                self.fptr_variable_is_variadic(name, stripped_func)
            } else {
                self.is_function_variadic(name)
            }
        } else {
            false
        };

        // Decompose complex double/float arguments into (real, imag) pairs for ABI compliance
        let param_ctypes_from_sig = if let Expr::Identifier(name, _) = stripped_func {
            let sig_for_decompose = if self.is_func_ptr_variable(name) {
                self.fptr_variable_sig(name)
            } else {
                self.func_meta.sigs.get(name.as_str())
            };
            sig_for_decompose
                .map(|s| s.param_ctypes.clone())
                .filter(|v| !v.is_empty())
        } else {
            None
        };
        // Minimal function-pointer signatures may know the IR parameter types
        // but not retain their CTypes (notably for typedef/declarator paths).
        // Recover those CTypes from the callee's function CType so i686
        // ExtVector arguments can still be lane-expanded for indirect calls.
        let param_ctypes_for_decompose = param_ctypes_from_sig.or_else(|| {
            self.extract_fn_ptr_param_info(effective_func)
                .map(|(_, param_ctypes, _, _)| param_ctypes)
                .filter(|v| !v.is_empty())
        });
        self.decompose_complex_call_args(
            &mut arg_vals,
            &mut arg_types,
            &mut struct_arg_sizes,
            &mut struct_arg_aligns,
            &mut struct_arg_classes,
            &param_ctypes_for_decompose,
            args,
            call_is_variadic,
        );
        let ext_vector_arg_lanes = self.decompose_i686_ext_vector_call_args(
            &mut arg_vals,
            &mut arg_types,
            &mut struct_arg_sizes,
            &mut struct_arg_aligns,
            &mut struct_arg_classes,
            &mut struct_arg_riscv_float_classes,
            &param_ctypes_for_decompose,
            args,
            call_is_variadic,
        );

        let dest = self.fresh_value();

        // For sret calls, allocate space and prepend hidden pointer argument
        let sret_alloca = if let Some(size) = sret_size {
            // The buffer's alignment MUST be the returned aggregate's natural
            // alignment (SysV: the callee writes the return value through the
            // hidden pointer — with aligned SSE stores, i.e. movaps, whenever
            // that alignment is 16, e.g. structs containing long double).
            // Recording `align: 0` made the FPO frame layout place the buffer
            // at an address ≡ 8 (mod 16); any GCC-compiled callee then faulted
            // with #GP on `movaps %xmm,(%rdi)` — SIGSEGV with si_addr = 0
            // (stress abi family, seed 0, lccc-caller/gcc-callee pairing;
            // frame-parity dependent, hence seed-shape dependent).
            // Direct calls read the alignment from the callee's recorded
            // return CType; indirect calls resolve it from the function
            // pointer's type via get_call_return_struct_align (same type
            // query as the sret SIZE, so the two can never disagree).
            let sret_align = match stripped_func {
                Expr::Identifier(name, _) if !self.is_func_ptr_variable(name) => self
                    .func_meta
                    .sigs
                    .get(name.as_str())
                    .and_then(|s| s.return_ctype.as_ref())
                    .map(|ct| self.ctype_align(ct).max(1))
                    .or_else(|| self.get_call_return_struct_align(stripped_func))
                    .unwrap_or(0),
                _ => self
                    .get_call_return_struct_align(stripped_func)
                    .unwrap_or(0),
            };
            let alloca = self.fresh_value();
            self.emit(Instruction::Alloca {
                dest: alloca,
                ty: IrType::Ptr,
                size,
                align: sret_align,
                volatile: false,
                semantic_volatile: false,
            });
            arg_vals.insert(0, Operand::Value(alloca));
            arg_types.insert(0, IrType::Ptr);
            struct_arg_sizes.insert(0, None); // sret pointer is not a struct arg
            struct_arg_aligns.insert(0, None);
            struct_arg_classes.insert(0, Vec::new()); // sret pointer has no eightbyte classes
            struct_arg_riscv_float_classes.insert(0, None); // sret pointer has no riscv float class
            Some(alloca)
        } else {
            None
        };

        // _Float128 call args: ONE 16-byte XMM register each (SysV psABI).
        // For prototyped positions the value reaching the backend has the
        // PARAM's type (S4 converts or raw-casts every prototyped arg), so
        // the ABI flag follows the declared param: arg-derived flags lie
        // for converted args (double->F128 passed integer classes, sending
        // the backend down its x87 spilling dance -> segfault; F128->double
        // would conversely pass SSE classes for an F64 value).
        // Unknown/unprototyped/variadic-tail/complex-callee positions keep
        // the argument expression's own type (covers indirect calls and
        // variadic positions, as before).
        // AUD-VABI-1: a 16-byte VECTOR argument is carried in ONE 16-byte xmm
        // register just like _Float128 (its eightbytes are SSE + SSEUP, the
        // upper half of the same register).  Without this the [Sse, Sse]
        // classes below send it through the two-register struct path, so a
        // gcc-compiled callee read the vector from xmm0 while lccc's caller
        // had put its halves in a GP pair.
        let one_xmm_carrier = |ct: &CType| -> bool {
            ct.sysv_vector_carrier().is_some_and(|(_, one_xmm)| one_xmm)
                && crate::common::types::target_elf_machine() == 62
        };
        let original_f128_flags: Vec<bool> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if let Some(p) = param_ctypes_for_decompose.as_ref().and_then(|v| v.get(i)) {
                    return matches!(p, CType::Float128 | CType::Decimal128) || one_xmm_carrier(p);
                }
                self.get_expr_ctype(a).is_some_and(|ct| {
                    matches!(ct, CType::Float128 | CType::Decimal128) || one_xmm_carrier(&ct)
                })
            })
            .collect();
        let mut struct_arg_is_f128_sse = Vec::with_capacity(arg_vals.len());
        for (arg_idx, flag) in original_f128_flags.into_iter().enumerate() {
            if let Some(lanes) = ext_vector_arg_lanes.get(arg_idx).copied().flatten() {
                struct_arg_is_f128_sse.extend(std::iter::repeat_n(false, lanes));
            } else {
                struct_arg_is_f128_sse.push(flag);
            }
        }
        if sret_alloca.is_some() {
            struct_arg_is_f128_sse.insert(0, false);
        }

        // Determine number of fixed args for variadic calls
        let (call_variadic, num_fixed_args) = if let Expr::Identifier(name, _) = stripped_func {
            let variadic = call_is_variadic;
            let n_fixed = if variadic {
                let variadic_sig = if self.is_func_ptr_variable(name) {
                    self.fptr_variable_sig(name)
                } else {
                    self.func_meta.sigs.get(name.as_str())
                };
                if let Some(sig) = variadic_sig {
                    let fixed_param_ctypes = if !sig.param_ctypes.is_empty() {
                        Some(&sig.param_ctypes)
                    } else {
                        param_ctypes_for_decompose.as_ref()
                    };
                    if let Some(param_ctypes) = fixed_param_ctypes {
                        let decomposes_cld = self.decomposes_complex_long_double();
                        let decomposes_cd = self.decomposes_complex_double();
                        let decomposes_cf = self.decomposes_complex_float();
                        param_ctypes
                            .iter()
                            .map(|ct| {
                                match ct {
                                    CType::ComplexDouble if decomposes_cd => 2,
                                    // Fixed ComplexFloat params are decomposed into 2 FP regs
                                    // on 64-bit targets (not x86-64 packed, not i686 struct)
                                    CType::ComplexFloat
                                        if decomposes_cf && !self.uses_packed_complex_float() =>
                                    {
                                        2
                                    }
                                    CType::ComplexLongDouble if decomposes_cld => 2,
                                    CType::ExtVector(_, layout)
                                        if crate::common::types::target_is_32bit() =>
                                    {
                                        layout.lanes
                                    }
                                    _ => 1,
                                }
                            })
                            .sum()
                    } else if !sig.param_types.is_empty() {
                        sig.param_types.len()
                    } else {
                        arg_vals.len()
                    }
                } else {
                    arg_vals.len()
                }
            } else {
                arg_vals.len()
            };
            (variadic, n_fixed)
        } else {
            (false, arg_vals.len())
        };

        // Dispatch: direct call, function pointer call, or indirect call
        let call_ret_ty = self.emit_call_instruction(
            effective_func,
            dest,
            arg_vals,
            arg_types,
            struct_arg_sizes,
            struct_arg_aligns,
            struct_arg_classes,
            struct_arg_riscv_float_classes,
            struct_arg_is_f128_sse,
            call_variadic,
            num_fixed_args,
            two_reg_size,
            sret_size,
            call_ret_classes,
            // AUD-VABI-1: "the indirect callee returns a 16-byte vector in ONE
            // %xmm" -- derived from the callee expression's function type,
            // since these paths have no FuncSig to consult.
            indirect_ret_is_one_xmm,
        );

        // After call to noreturn function, emit unreachable and start dead block.
        // Unlike error_functions (which skip the call entirely), noreturn functions
        // are real functions that get called but never return (e.g., panic, abort).
        //
        // SHADOWING: when a function-pointer variable/parameter shadows the
        // name (the boot decompressor's `void (*error)(char *x)` parameter vs
        // error.h's `void error(char *) __noreturn`), the call is INDIRECT
        // through the variable's value — emit_call_instruction dispatches it
        // via is_func_ptr_variable/load_func_ptr_variable — and the shadowed
        // function's noreturn attribute must NOT apply: the pointer target
        // may perfectly well return. Keying this test on the name alone
        // terminated every such call site with Unreachable and emitted ud2
        // after the indirect call (userspace zstd oracle SIGILL; the
        // lccc-built preboot decompressor lost its post-error() control
        // flow). Gate on the same predicate the call dispatch uses.
        if let Expr::Identifier(name, _) = stripped_func {
            if self.noreturn_functions.contains(name) && !self.is_func_ptr_variable(name) {
                self.terminate(Terminator::Unreachable);
                let dead_label = self.fresh_label();
                self.start_block(dead_label);
                return Operand::Const(IrConst::ptr_int(0));
            }
        }

        // For sret calls, the struct data is now in the alloca - return its address
        if let Some(alloca) = sret_alloca {
            if f128_sret_ret {
                // _Float128/_Decimal128 (i686 TFmode sret): the value is the
                // 16-byte carrier CONTENTS, not the buffer address — load the
                // gp-view value out of the buffer.
                let val = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: val,
                    ptr: alloca,
                    ty: IrType::U128,
                    seg_override: crate::common::types::AddressSpace::Default,
                });
                return Operand::Value(val);
            }
            return Operand::Value(alloca);
        }

        // For two-register struct returns (9-16 bytes), unpack I128 into struct alloca
        if let Some(size) = two_reg_size {
            return self.unpack_two_reg_return(dest, size);
        }

        if let Some(vector_ct) = i686_ext_vector_gp_return_ctype.as_ref() {
            return self.unpack_i686_ext_vector_gp_return(dest, call_ret_ty, vector_ct);
        }

        // For complex returns (non-sret), store into complex alloca
        if sret_size.is_none() {
            // First try the function name lookup (for direct calls)
            let ret_ct = if let Expr::Identifier(name, _) = stripped_func {
                self.types.func_return_ctypes.get(name).cloned()
            } else {
                None
            };
            // Fall back to extracting return CType from the expression's type
            // (handles indirect calls through function pointers)
            let ret_ct = ret_ct.or_else(|| self.get_func_ptr_return_ctype(stripped_func));
            if let Some(ret_ct) = ret_ct {
                if let Some(result) = self.handle_complex_return(dest, &ret_ct) {
                    return result;
                }
            }
        }

        // Narrow the result if the return type is sub-64-bit integer
        self.maybe_narrow_call_result(dest, call_ret_ty)
    }

    /// Unpack a two-register (I128) struct return into a struct alloca.

    /// The builtin fold ladder for a call whose callee is a plain
    /// identifier that no local binding shadows: fold `__builtin_*` calls,
    /// then `__attribute__((__diagnose_as(__builtin_NAME, ...)))` aliases,
    /// then neutralize surviving `__attribute__((error(...)))` calls.
    /// `Some(operand)` means the call is fully handled and must not reach
    /// the ordinary call path.
    fn try_fold_builtin_or_attr_call(&mut self, func: &Expr, args: &[Expr]) -> Option<Operand> {
        // A block-scope object or parameter (`llf_t abs`,
        // `void *(*memcpy)(...)`) shadows the library function of that name
        // (C11 6.2.1p4); expanding the builtin anyway silently replaced a
        // call through `i64 (*abs)(i64)` with a constant-folded `int abs(int)`.
        let callee_is_local = matches!(func, Expr::Identifier(n, _)
            if self.func_state.as_ref().is_some_and(|fs| fs.locals.contains_key(n.as_str())));
        let Expr::Identifier(name, _) = func else {
            return None;
        };
        if callee_is_local {
            return None;
        }
        if let Some(result) = self.try_lower_builtin_call(name, args) {
            return Some(result);
        }

        // __attribute__((__diagnose_as(__builtin_NAME, ...))) folding
        // (GCC semantics): a call to a diagnosed declaration that
        // survived inlining IS a call to the builtin.  The kernel's
        // fortify-string family (__FORTIFY_INLINE = extern __always_inline)
        // depends on BOTH halves of GCC's ladder — inline, else fold —
        // because the declarations are extern and resolve to nothing at
        // link time.  A missed fold here surfaced as undefined
        // `__fortify_strlen` / `fortify_memset_chk` at the vmlinux link.
        // GCC folds EVERY call to a __diagnose_as declaration to the
        // named builtin — the attribute is a declarative alias, not an
        // inlining fallback.  For the fortify family the definitions
        // reduce to the underlying builtin plus __builtin_constant_p-
        // guarded COMPILE-TIME diagnostics; folding drops only those
        // diagnostics (a missed lint), never a defined program's
        // runtime semantics — the same license GCC's own fold runs on.
        let alias = self.diagnose_as_builtins.get(name).cloned();
        if std::env::var_os("CCC_DBG_DA").is_some() && name.contains("strlen") {
            eprintln!(
                "[DA-call] {} map={:?}",
                name,
                self.diagnose_as_builtins.get(name)
            );
        }
        if let Some(builtin) = alias {
            // try_lower_builtin_call dispatches on the canonical
            // __builtin_-prefixed spelling.
            let builtin_name = format!("__builtin_{builtin}");
            if let Some(result) = self.try_lower_builtin_call(&builtin_name, args) {
                return Some(result);
            }
            // The builtin name is not one this backend expands: fall
            // through to the ordinary call (same behavior as before the
            // attribute existed) rather than fabricating semantics.
        }

        // Functions declared with __attribute__((error("..."))) are compile-time
        // assertion functions (e.g., kernel's __bad_mask, __field_overflow).
        // In GCC, these calls are eliminated by inlining + constant folding,
        // and if they survive, GCC emits a compile error. Since CCC's inliner
        // may not inline all call sites (due to budget/round limits), we must
        // handle surviving calls gracefully.
        //
        // Instead of emitting a trap (which causes kernel BRK crashes when
        // always_inline functions like cpucap_is_possible aren't fully inlined),
        // we simply skip the call. This avoids referencing the undefined symbol
        // and makes the standalone function body harmless. After inlining, the
        // dead code containing this no-op is eliminated by constant folding and DCE.
        //
        // TODO: Once the inliner can fully inline all always_inline chains
        // (eliminating all error function call sites), restore this to emit a
        // compile error (matching GCC behavior) instead of silently dropping.
        if self.error_functions.contains(name) {
            // No-op: skip the call. The code path should be unreachable after
            // inlining and constant folding. If it IS reached at runtime,
            // execution continues harmlessly (which is better than crashing).
            return Some(Operand::Const(IrConst::ptr_int(0)));
        }
        None
    }
    fn unpack_two_reg_return(&mut self, dest: Value, size: usize) -> Operand {
        let alloca = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: alloca,
            ty: IrType::Ptr,
            size,
            align: 0,
            volatile: false,
            semantic_volatile: false,
        });
        // Extract low 8 bytes (rax)
        let lo = self.fresh_value();
        self.emit(Instruction::Cast {
            dest: lo,
            src: Operand::Value(dest),
            from_ty: IrType::I128,
            to_ty: IrType::I64,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(lo),
            ptr: alloca,
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
        });
        // Extract high bytes (rdx): shift right by 64
        let shifted = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: shifted,
            op: IrBinOp::LShr,
            lhs: Operand::Value(dest),
            rhs: Operand::Const(IrConst::I64(64)),
            ty: IrType::I128,
        });
        let hi = self.fresh_value();
        self.emit(Instruction::Cast {
            dest: hi,
            src: Operand::Value(shifted),
            from_ty: IrType::I128,
            to_ty: IrType::I64,
        });
        let hi_ptr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: hi_ptr,
            base: alloca,
            offset: Operand::Const(IrConst::I64(8)),
            ty: IrType::I64,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(hi),
            ptr: hi_ptr,
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
        });
        Operand::Value(alloca)
    }

    /// Rebuild an i686 Clang integer ExtVector from its EAX:EDX:ECX ABI word
    /// bundle. The call result is a scalar transport value, not the vector's
    /// padded object representation. Materialize the carrier in memory before
    /// loading lanes: i686 I128 arithmetic intentionally models the low 64-bit
    /// pair, so shifting an I128 by 64 would lose the ECX word.
    fn unpack_i686_ext_vector_gp_return(
        &mut self,
        dest: Value,
        return_type: IrType,
        vector_ct: &CType,
    ) -> Operand {
        let words = vector_ct
            .i686_ext_vector_gp_word_count()
            .expect("caller only selects the i686 integer ExtVector GP ABI");
        let (elem_ct, lanes) = vector_ct
            .vector_info()
            .expect("ExtVector must carry its logical lane count");
        let elem_size = elem_ct.size();
        // See try_i686_ext_vector_gp_return: bundle is bounded at three 32-bit
        // words or one 64-bit word. Callee and caller must agree, otherwise
        // the ECX word spill in the backend writes to an unrelated slot.
        debug_assert!(
            words <= 3 && ((elem_size <= 4 && words == lanes) || (elem_size == 8 && lanes == 1)),
            "i686 ExtVector GP return unpack: words={words} elem_size={elem_size} lanes={lanes} exceeds the EAX:EDX:ECX bundle",
        );
        let size = self.ctype_size(vector_ct);
        let align = self.ctype_align(vector_ct);

        // Preserve all three GP words from the backend's I128 result slot.
        // I32/I64 results write only their live low words; zero the unused
        // tail so that the full-width carrier store is deterministic.
        let bundle = self.emit_entry_alloca(IrType::Ptr, 16, 4, false);
        self.zero_init_alloca(bundle, 16);
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(dest),
            ptr: bundle,
            ty: return_type,
            seg_override: AddressSpace::Default,
        });

        let result = self.emit_entry_alloca(IrType::Ptr, size, align, false);
        self.zero_init_alloca(result, size);
        if elem_size <= 4 {
            debug_assert_eq!(words, lanes);
            for lane in 0..lanes {
                let lane_ptr = self.emit_gep_offset(bundle, lane * 4, IrType::I32);
                let lane_value = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: lane_value,
                    ptr: lane_ptr,
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                });
                self.store_vector_lane(
                    result,
                    vector_ct,
                    lane,
                    Operand::Value(lane_value),
                    IrType::I32,
                    AddressSpace::Default,
                    false,
                );
            }
        } else {
            debug_assert_eq!(elem_size, 8);
            debug_assert_eq!(lanes, 1);
            let lane_value = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: lane_value,
                ptr: bundle,
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            });
            self.store_vector_lane(
                result,
                vector_ct,
                0,
                Operand::Value(lane_value),
                IrType::I64,
                AddressSpace::Default,
                false,
            );
        }
        Operand::Value(result)
    }

    /// Handle complex return types (ComplexFloat, ComplexDouble, ComplexLongDouble).
    /// Returns Some(result) if the return was complex, None otherwise.
    /// Note: on i686, ComplexDouble and ComplexLongDouble use sret (handled before this),
    /// so this function only needs to handle ComplexFloat on i686.
    fn handle_complex_return(&mut self, dest: Value, ret_ct: &CType) -> Option<Operand> {
        match ret_ct {
            CType::ComplexFloat => {
                if self.uses_packed_complex_float() {
                    // x86-64: two packed F32 returned in xmm0 as F64
                    // Store the raw 8 bytes (two F32s) into an alloca
                    let alloca = self.fresh_value();
                    self.emit(Instruction::Alloca {
                        dest: alloca,
                        ty: IrType::Ptr,
                        size: 8,
                        align: 0,
                        volatile: false,
                        semantic_volatile: false,
                    });
                    self.emit(Instruction::Store {
                        volatile: false,
                        val: Operand::Value(dest),
                        ptr: alloca,
                        ty: IrType::F64,
                        seg_override: AddressSpace::Default,
                    });
                    Some(Operand::Value(alloca))
                } else if !self.decomposes_complex_float() {
                    // i686: two packed F32 returned in eax:edx as I64
                    // Store the raw 8 bytes (two F32s) into an alloca
                    let alloca = self.fresh_value();
                    self.emit(Instruction::Alloca {
                        dest: alloca,
                        ty: IrType::Ptr,
                        size: 8,
                        align: 0,
                        volatile: false,
                        semantic_volatile: false,
                    });
                    self.emit(Instruction::Store {
                        volatile: false,
                        val: Operand::Value(dest),
                        ptr: alloca,
                        ty: IrType::I64,
                        seg_override: AddressSpace::Default,
                    });
                    Some(Operand::Value(alloca))
                } else {
                    // ARM/RISC-V: real F32 in first FP reg (dest), imag F32 in second FP reg
                    let imag_val = self.fresh_value();
                    self.emit(Instruction::GetReturnF32Second { dest: imag_val });
                    let alloca = self.fresh_value();
                    self.emit(Instruction::Alloca {
                        dest: alloca,
                        ty: IrType::Ptr,
                        size: 8,
                        align: 0,
                        volatile: false,
                        semantic_volatile: false,
                    });
                    // Store real part (F32) at offset 0
                    self.emit(Instruction::Store {
                        volatile: false,
                        val: Operand::Value(dest),
                        ptr: alloca,
                        ty: IrType::F32,
                        seg_override: AddressSpace::Default,
                    });
                    // Store imag part (F32) at offset 4
                    let imag_ptr = self.fresh_value();
                    let ptr_int_ty = crate::common::types::target_int_ir_type();
                    self.emit(Instruction::BinOp {
                        dest: imag_ptr,
                        op: IrBinOp::Add,
                        lhs: Operand::Value(alloca),
                        rhs: Operand::Const(IrConst::ptr_int(4)),
                        ty: ptr_int_ty,
                    });
                    self.emit(Instruction::Store {
                        volatile: false,
                        val: Operand::Value(imag_val),
                        ptr: imag_ptr,
                        ty: IrType::F32,
                        seg_override: AddressSpace::Default,
                    });
                    Some(Operand::Value(alloca))
                }
            }
            CType::ComplexDouble if self.decomposes_complex_double() => {
                // x86-64/ARM64/RISC-V: _Complex double: real in first FP reg (dest),
                // imag in second FP reg (second return)
                let imag_val = self.fresh_value();
                self.emit(Instruction::GetReturnF64Second { dest: imag_val });
                let alloca = self.fresh_value();
                self.emit(Instruction::Alloca {
                    dest: alloca,
                    ty: IrType::Ptr,
                    size: 16,
                    align: 0,
                    volatile: false,
                    semantic_volatile: false,
                });
                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(dest),
                    ptr: alloca,
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                });
                let imag_ptr = self.fresh_value();
                let ptr_int_ty = crate::common::types::target_int_ir_type();
                self.emit(Instruction::BinOp {
                    dest: imag_ptr,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(alloca),
                    rhs: Operand::Const(IrConst::ptr_int(8)),
                    ty: ptr_int_ty,
                });
                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(imag_val),
                    ptr: imag_ptr,
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                });
                Some(Operand::Value(alloca))
            }
            CType::ComplexLongDouble if self.returns_complex_long_double_in_regs() => {
                // x86-64: _Complex long double returns real in x87 st(0), imag in x87 st(1).
                // After the call, emit_call_store_result stores st(0) (real) into dest.
                // GetReturnF128Second reads the next x87 value (former st(1), now st(0) after first fstpt).
                let imag_val = self.fresh_value();
                self.emit(Instruction::GetReturnF128Second { dest: imag_val });
                let alloca = self.fresh_value();
                self.emit(Instruction::Alloca {
                    dest: alloca,
                    ty: IrType::Ptr,
                    size: 32,
                    align: 16,
                    volatile: false,
                    semantic_volatile: false,
                });
                // Store real part (F128) at offset 0
                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(dest),
                    ptr: alloca,
                    ty: IrType::F128,
                    seg_override: AddressSpace::Default,
                });
                // Store imag part (F128) at offset 16
                let imag_ptr = self.fresh_value();
                let ptr_int_ty = crate::common::types::target_int_ir_type();
                self.emit(Instruction::BinOp {
                    dest: imag_ptr,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(alloca),
                    rhs: Operand::Const(IrConst::ptr_int(16)),
                    ty: ptr_int_ty,
                });
                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(imag_val),
                    ptr: imag_ptr,
                    ty: IrType::F128,
                    seg_override: AddressSpace::Default,
                });
                Some(Operand::Value(alloca))
            }
            _ => None,
        }
    }

    /// Scalarize fixed i686 ExtVector call arguments to match Clang's lane-wise
    /// stack ABI. `vector_size` and long-double ext-vectors keep their existing
    /// by-value aggregate path. Returns each original argument's lane count so
    /// call metadata (fixed-argument count and carrier flags) can be expanded.
    fn decompose_i686_ext_vector_call_args(
        &mut self,
        arg_vals: &mut Vec<Operand>,
        arg_types: &mut Vec<IrType>,
        struct_arg_sizes: &mut Vec<Option<usize>>,
        struct_arg_aligns: &mut Vec<Option<usize>>,
        struct_arg_classes: &mut Vec<Vec<crate::common::types::EightbyteClass>>,
        struct_arg_riscv_float_classes: &mut Vec<Option<crate::common::types::RiscvFloatClass>>,
        param_ctypes: &Option<Vec<CType>>,
        args: &[Expr],
        _is_variadic_call: bool,
    ) -> Vec<Option<usize>> {
        let mut lane_counts = vec![None; args.len()];
        if !crate::common::types::target_is_32bit() {
            return lane_counts;
        }
        let Some(param_ctypes) = param_ctypes.as_ref() else {
            return lane_counts;
        };

        let old_vals = std::mem::take(arg_vals);
        let old_types = std::mem::take(arg_types);
        let old_sizes = std::mem::take(struct_arg_sizes);
        let old_aligns = std::mem::take(struct_arg_aligns);
        let old_classes = std::mem::take(struct_arg_classes);
        let old_riscv_classes = std::mem::take(struct_arg_riscv_float_classes);
        let mut cursor = 0usize;
        let mut new_vals = Vec::with_capacity(old_vals.len());
        let mut new_types = Vec::with_capacity(old_types.len());
        let mut new_sizes = Vec::with_capacity(old_sizes.len());
        let mut new_aligns = Vec::with_capacity(old_aligns.len());
        let mut new_classes = Vec::with_capacity(old_classes.len());
        let mut new_riscv_classes = Vec::with_capacity(old_riscv_classes.len());

        for (arg_idx, arg) in args.iter().enumerate() {
            let ext_vector = param_ctypes.get(arg_idx).and_then(|ct| match ct {
                CType::ExtVector(elem, layout) => Some((ct, elem.as_ref(), layout.lanes)),
                _ => None,
            });
            if let Some((vector_ct, elem_ct, lanes)) = ext_vector {
                let Some(value) = old_vals.get(cursor).copied() else {
                    break;
                };
                let base = self.operand_to_value(value);
                let address_space = self.aggregate_lvalue_space(arg);
                let volatile = self.expr_access_is_volatile(arg);
                let lane_ty = if matches!(elem_ct, CType::Bool) {
                    IrType::I32
                } else {
                    IrType::from_ctype(elem_ct)
                };
                for lane in 0..lanes {
                    let lane_value =
                        self.load_vector_lane(base, vector_ct, lane, address_space, volatile);
                    new_vals.push(lane_value);
                    new_types.push(lane_ty);
                    new_sizes.push(None);
                    new_aligns.push(None);
                    // `[NoClass]` tags this scalar as an i686 ExtVector lane;
                    // regparm/fastcall keep it on the stack without consuming
                    // their next ordinary GP argument register.
                    new_classes.push(vec![crate::common::types::EightbyteClass::NoClass]);
                    new_riscv_classes.push(None);
                }
                lane_counts[arg_idx] = Some(lanes);
                cursor += 1;
            } else {
                let Some(value) = old_vals.get(cursor).copied() else {
                    break;
                };
                new_vals.push(value);
                new_types.push(old_types.get(cursor).copied().unwrap_or(IrType::Ptr));
                new_sizes.push(old_sizes.get(cursor).copied().flatten());
                new_aligns.push(old_aligns.get(cursor).copied().flatten());
                new_classes.push(old_classes.get(cursor).cloned().unwrap_or_default());
                new_riscv_classes.push(old_riscv_classes.get(cursor).copied().flatten());
                cursor += 1;
            }
        }
        // Keep any remaining operands intact if a prior ABI decomposition
        // produced extra entries; this is a defensive preservation path.
        while cursor < old_vals.len() {
            new_vals.push(old_vals[cursor]);
            new_types.push(old_types.get(cursor).copied().unwrap_or(IrType::Ptr));
            new_sizes.push(old_sizes.get(cursor).copied().flatten());
            new_aligns.push(old_aligns.get(cursor).copied().flatten());
            new_classes.push(old_classes.get(cursor).cloned().unwrap_or_default());
            new_riscv_classes.push(old_riscv_classes.get(cursor).copied().flatten());
            cursor += 1;
        }
        *arg_vals = new_vals;
        *arg_types = new_types;
        *struct_arg_sizes = new_sizes;
        *struct_arg_aligns = new_aligns;
        *struct_arg_classes = new_classes;
        *struct_arg_riscv_float_classes = new_riscv_classes;
        lane_counts
    }

    /// Lower function call arguments, applying implicit casts for parameter types
    /// and default argument promotions for variadic args.
    /// Returns (arg_vals, arg_types, struct_arg_sizes, struct_arg_aligns, struct_arg_classes) where struct_arg_sizes[i] is
    /// Some(size) if the ith argument is a struct/union passed by value, and struct_arg_aligns[i]
    /// is Some(align) for struct args.
    pub(super) fn lower_call_arguments(
        &mut self,
        func: &Expr,
        args: &[Expr],
    ) -> (
        Vec<Operand>,
        Vec<IrType>,
        Vec<Option<usize>>,
        Vec<Option<usize>>,
        Vec<Vec<crate::common::types::EightbyteClass>>,
        Vec<Option<crate::common::types::RiscvFloatClass>>,
    ) {
        // Extract function name from direct calls, or the underlying variable name
        // from indirect calls through function pointers (e.g., (*afp)(args) -> "afp").
        let func_name = match func {
            Expr::Identifier(name, _) => Some(name.as_str()),
            Expr::Deref(inner, _) => {
                if let Expr::Identifier(name, _) = inner.as_ref() {
                    Some(name.as_str())
                } else {
                    None
                }
            }
            _ => None,
        };
        // When the callee is a function pointer variable, prefer its own signature
        // over sigs. This prevents a parameter named e.g. `round` from picking up
        // the seeded `double round(double)` library signature instead of the
        // actual function pointer's signature.
        let sig = func_name.and_then(|name| {
            if self.is_func_ptr_variable(name) {
                self.fptr_variable_sig(name)
            } else {
                self.func_meta.sigs.get(name)
            }
        });

        // Named/local function-pointer variables can have a minimal FuncSig
        // whose IR parameter types are known but whose CTypes were not retained.
        // Fall back to the callee CType when the signature is absent or lacks
        // parameter CTypes; this also preserves ExtVector lane decomposition
        // and the usual implicit argument conversions for indirect calls.
        let inferred_from_ctype = if sig.is_none_or(|s| s.param_ctypes.is_empty()) {
            self.extract_fn_ptr_param_info(func)
        } else {
            None
        };

        // If the signature has an empty parameter list (unprototyped function like `int f()`),
        // all arguments need default argument promotions (float->double, char/short->int).
        let is_unprototyped = sig.is_some_and(|s| s.param_types.is_empty());
        let param_types: Option<Vec<IrType>> = sig
            .map(|s| s.param_types.clone())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                inferred_from_ctype
                    .as_ref()
                    .map(|(pt, _, _, _)| pt.clone())
                    .filter(|v| !v.is_empty())
            });
        let param_ctypes: Option<Vec<CType>> = sig
            .map(|s| s.param_ctypes.clone())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                inferred_from_ctype
                    .as_ref()
                    .map(|(_, pc, _, _)| pc.clone())
                    .filter(|v| !v.is_empty())
            });
        let param_bool_flags: Option<Vec<bool>> = sig
            .map(|s| s.param_bool_flags.clone())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                inferred_from_ctype
                    .as_ref()
                    .map(|(_, _, pb, _)| pb.clone())
                    .filter(|v| !v.is_empty())
            });
        let pre_call_variadic = func_name.is_some_and(|name| self.is_function_variadic(name))
            || inferred_from_ctype
                .as_ref()
                .is_some_and(|(_, _, _, variadic)| *variadic);

        let mut arg_types = Vec::with_capacity(args.len());
        // Track argument indices where a complex expression was converted to a
        // scalar type (e.g., complex-to-bool). These should NOT get a
        // struct_arg_size based on the original expression's CType.
        let mut complex_converted_to_scalar: Vec<bool> = vec![false; args.len()];
        let arg_vals: Vec<Operand> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let mut val = self.lower_aggregate_rvalue(a);
                // A call argument is a **conversion boundary**, so the source
                // type must be the type of the value `lower_expr` actually
                // produced, not the storage-level query.
                //
                // `get_expr_type` widens an arithmetic `BinaryOp` to the
                // target's storage int (`I64` on LP64) because an `IntLiteral`
                // operand reports `I64`. The emitted `BinOp` however carries
                // the true C operation type (`i + 1` is `int`, `ty: I32`), so
                // the two disagree and the disagreement is a *width lie*:
                //
                //   __builtin_memcmp(p, "abcdefg", i + 1)
                //     -> Call memcmp, args [.., v8:I32], arg_types [.., I64]
                //
                // At -O0 `v8` lives in a 4-byte slot and the 8-byte argument
                // load pulled the neighbouring slot in as the high half, so
                // `%rdx` became `0x1_00000007` and memcmp ran off the end of
                // both buffers (gcc.c-torture `pr59229.c`). With a prototype
                // in scope the same lie produced `Cast { from_ty: I64,
                // to_ty: U64 }` — a same-width no-op where C requires the
                // **sign extension** of a negative `int` to `size_t`.
                //
                // `value_ir_type` is the query documented for exactly this
                // (it differs from `get_expr_type` only for `BinaryOp` and
                // `!x`, which are precisely the expressions whose materialised
                // width can be narrower than their storage width).
                let mut arg_ty = self.value_ir_type(a);

                // A raw GCC 128-bit vector builtin (__builtin_ia32_shufps,
                // __builtin_shufflevector, ...) yields its vector BY VALUE as a
                // packed I128; the struct/vector argument-passing path below
                // expects a POINTER to the 16 data bytes (like any vector lvalue
                // argument). Spill to a slot and continue as a pointer arg —
                // without the type override, the param-cast path saw I128 and
                // emitted __floattisf on the POINTER (r4.c repro segfault).
                if arg_ty == IrType::I128 {
                    if let Expr::FunctionCall(callee, _, _) = a {
                        if let Expr::Identifier(bn, _) = &**callee {
                            if matches!(
                                bn.as_str(),
                                "__builtin_ia32_maxps"
                                    | "__builtin_ia32_minps"
                                    | "__builtin_ia32_shufps"
                                    | "__builtin_shufflevector"
                                    | "__builtin_ia32_vextractf128_ps256"
                            ) {
                                let slot = self.fresh_value();
                                self.emit(Instruction::Alloca {
                                    dest: slot,
                                    ty: IrType::Ptr,
                                    size: 16,
                                    align: 0,
                                    volatile: false,
                                    semantic_volatile: false,
                                });
                                let v = self.operand_to_value(val);
                                self.emit(Instruction::Store {
                                    volatile: false,
                                    val: Operand::Value(v),
                                    ptr: slot,
                                    ty: IrType::I128,
                                    seg_override: crate::common::types::AddressSpace::Default,
                                });
                                val = Operand::Value(slot);
                                arg_ty = IrType::Ptr;
                            }
                        }
                    }
                }

                // Convert complex arguments to the declared parameter complex type if they differ
                if let Some(ref pctypes) = param_ctypes {
                    if i < pctypes.len() && pctypes[i].is_complex() {
                        let arg_ct = self.expr_ctype(a);
                        if arg_ct.is_complex() && arg_ct != pctypes[i] {
                            let ptr = self.operand_to_value(val);
                            val = self.complex_to_complex(ptr, &arg_ct, &pctypes[i]);
                        } else if !arg_ct.is_complex() {
                            val = self.real_to_complex(val, &arg_ct, &pctypes[i]);
                        }
                    } else if i < pctypes.len() && !pctypes[i].is_complex() {
                        let arg_ct = self.expr_ctype(a);
                        if arg_ct.is_complex() {
                            let ptr = self.operand_to_value(val);
                            // Check if target param is _Bool: use both real and imag per C11 6.3.1.2
                            let is_param_bool = param_bool_flags
                                .as_ref()
                                .and_then(|bf| bf.get(i).copied())
                                .unwrap_or(false);
                            if is_param_bool {
                                let cast_val = self.lower_complex_to_bool(ptr, &arg_ct);
                                arg_types.push(IrType::I8);
                                complex_converted_to_scalar[i] = true;
                                return cast_val;
                            }
                            let real_part = self.load_complex_real(ptr, &arg_ct);
                            let comp_ir_ty = Self::complex_component_ir_type(&arg_ct);
                            let param_ty = param_types
                                .as_ref()
                                .and_then(|pt| pt.get(i).copied())
                                .unwrap_or(comp_ir_ty);
                            let cast_val = self.emit_implicit_cast(real_part, comp_ir_ty, param_ty);
                            arg_types.push(param_ty);
                            complex_converted_to_scalar[i] = true;
                            return cast_val;
                        }
                    }
                }

                // Spill packed struct data to a temporary alloca so that the call
                // argument is a pointer (address) rather than raw data.  This is
                // needed for any expression that produces packed struct data:
                // direct function calls, statement expressions wrapping calls,
                // ternaries, comma expressions, etc.
                {
                    let is_struct_ret = matches!(
                        self.get_expr_ctype(a),
                        Some(CType::Struct(_)) | Some(CType::Union(_))
                    );
                    if is_struct_ret && self.expr_produces_packed_struct_data(a) {
                        let struct_size = self.struct_value_size(a).unwrap_or(8);
                        let alloc_size = if struct_size > 0 { struct_size } else { 8 };
                        let alloca = self.fresh_value();
                        let store_ty = Self::packed_store_type(alloc_size);
                        self.emit(Instruction::Alloca {
                            dest: alloca,
                            size: alloc_size,
                            ty: store_ty,
                            align: 0,
                            volatile: false,
                            semantic_volatile: false,
                        });
                        self.emit(Instruction::Store {
                            volatile: false,
                            val,
                            ptr: alloca,
                            ty: store_ty,
                            seg_override: AddressSpace::Default,
                        });
                        val = Operand::Value(alloca);
                    }
                }

                let is_bool_param = param_bool_flags
                    .as_ref()
                    .is_some_and(|flags| i < flags.len() && flags[i]);

                if let Some(ref ptypes) = param_types {
                    if i < ptypes.len() {
                        let param_ty = ptypes[i];
                        arg_types.push(param_ty);
                        if is_bool_param {
                            // For _Bool params, normalize at source type before truncation.
                            return self.emit_bool_normalize_typed(val, arg_ty);
                        }
                        // C23 decimals (either side) route through the libbid
                        // helpers, mirroring the _Float128 soft-float routing
                        // at init/assign/return: a raw IR cast would emit a
                        // binary conversion on BID carriers (silent miscompile).
                        // _Float128 itself routes here too (same bug class: a
                        // raw double->U128 bit-cast segfaults soft-float code).
                        if let Some(ref pctypes) = param_ctypes {
                            if i < pctypes.len() {
                                let arg_ct = self.expr_ctype(a);
                                if pctypes[i].is_decimal()
                                    || arg_ct.is_decimal()
                                    || pctypes[i] == CType::Float128
                                    || arg_ct == CType::Float128
                                {
                                    return self.convert_scalar_ctype(
                                        val,
                                        arg_ty,
                                        &arg_ct,
                                        &pctypes[i],
                                    );
                                }
                            }
                        }
                        let cast_val = self.emit_implicit_cast(val, arg_ty, param_ty);
                        return cast_val;
                    }
                }
                // Default argument promotions (C11 6.5.2.2p6): for variadic args,
                // args beyond known params, or all args for unprototyped functions:
                // - float -> double
                // - char/short (signed or unsigned) -> int
                let needs_promotion = pre_call_variadic || param_types.is_some() || is_unprototyped;
                if needs_promotion {
                    if arg_ty == IrType::F32 {
                        arg_types.push(IrType::F64);
                        return self.emit_implicit_cast(val, IrType::F32, IrType::F64);
                    }
                    if matches!(arg_ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16) {
                        arg_types.push(IrType::I32);
                        return self.emit_implicit_cast(val, arg_ty, IrType::I32);
                    }
                }
                arg_types.push(arg_ty);
                val
            })
            .collect();

        // Build struct_arg_sizes: for each arg, check if it's a struct/union by value
        let func_name = if let Expr::Identifier(name, _) = func {
            Some(name.as_str())
        } else {
            None
        };
        let struct_arg_sizes: Vec<Option<usize>> = if let Some(ref sizes) =
            func_name.and_then(|n| {
                self.func_meta
                    .sigs
                    .get(n)
                    .map(|s| s.param_struct_sizes.clone())
            }) {
            // Use pre-registered struct sizes from function metadata.
            // For variadic _Complex long double args beyond fixed params, infer size
            // (param_struct_sizes only covers declared parameters).
            // Also override None entries for vector types, since the first-pass
            // registration may not have resolved vector typedefs yet.
            let decomposes_cld = self.decomposes_complex_long_double();
            args.iter()
                .enumerate()
                .map(|(i, a)| {
                    // If Stage A already converted this complex argument to a scalar
                    // (e.g., complex-to-bool or complex-to-real), the value is no
                    // longer a struct-like type and must not get a struct_arg_size.
                    if complex_converted_to_scalar.get(i).copied().unwrap_or(false) {
                        return None;
                    }
                    let registered = if i < sizes.len() { sizes[i] } else { None };
                    if registered.is_some() {
                        return registered;
                    }
                    // Fall back to expression-based inference for unregistered types
                    let ctype = self.get_expr_ctype(a);
                    let decomposes_cd = self.decomposes_complex_double();
                    let decomposes_cf = self.decomposes_complex_float();
                    match ctype {
                        Some(CType::Struct(_)) | Some(CType::Union(_)) => {
                            if pre_call_variadic && self.dynamic_struct_value_size(a).is_some() {
                                // GCC passes variable-size aggregate variadic
                                // arguments by reference on the supported ABIs.
                                // Do not mark them as by-value struct args with
                                // size zero, or call lowering consumes no GP
                                // register for the pointer (20020412-1.c).
                                None
                            } else {
                                self.struct_value_size(a)
                            }
                        }
                        Some(CType::Vector(_, total_size)) => Some(total_size),
                        // Clang ext_vector_type: by-value aggregate varargs on
                        // x86-64 (SysV carrier classes; see
                        // `infer_struct_eightbyte_classes`).  32-bit targets
                        // keep their legacy pointer convention so the i686
                        // consumer side is not switched under this fix.
                        Some(ref ct @ CType::ExtVector(..))
                            if !crate::common::types::target_is_32bit() =>
                        {
                            Some(ct.size())
                        }
                        Some(CType::ComplexLongDouble) if !decomposes_cld => {
                            Some(CType::ComplexLongDouble.size())
                        }
                        // i686: ComplexDouble (16 bytes) and ComplexFloat (8 bytes) are
                        // passed as structs on the stack when not decomposed.
                        Some(CType::ComplexDouble) if !decomposes_cd => {
                            Some(CType::ComplexDouble.size())
                        }
                        Some(CType::ComplexFloat) if !decomposes_cf => {
                            Some(CType::ComplexFloat.size())
                        }
                        // _Float128/_Decimal128: 16-byte TFmode carriers (the
                        // i686 stack layout aligns them; x86-64 puts them in
                        // an XMM register via the is_f128_sse class).
                        Some(CType::Float128) | Some(CType::Decimal128) => Some(16),
                        _ => None,
                    }
                })
                .collect()
        } else {
            // Infer from argument expressions
            let decomposes_cld = self.decomposes_complex_long_double();
            let decomposes_cd = self.decomposes_complex_double();
            let decomposes_cf = self.decomposes_complex_float();
            args.iter()
                .enumerate()
                .map(|(i, a)| {
                    // If Stage A already converted this complex argument to a scalar
                    // (e.g., complex-to-bool or complex-to-real), the value is no
                    // longer a struct-like type and must not get a struct_arg_size.
                    if complex_converted_to_scalar.get(i).copied().unwrap_or(false) {
                        return None;
                    }
                    let ctype = self.get_expr_ctype(a);
                    match ctype {
                        Some(CType::Struct(_)) | Some(CType::Union(_)) => {
                            if pre_call_variadic && self.dynamic_struct_value_size(a).is_some() {
                                // GCC passes variable-size aggregate variadic
                                // arguments by reference on the supported ABIs.
                                // Do not mark them as by-value struct args with
                                // size zero, or call lowering consumes no GP
                                // register for the pointer (20020412-1.c).
                                None
                            } else {
                                self.struct_value_size(a)
                            }
                        }
                        Some(CType::Vector(_, total_size)) => {
                            Some(total_size) // Vector types are passed by value like structs
                        }
                        // x86-64 ext_vector_type by-value varargs (see the
                        // signature-known branch above); 32-bit targets keep
                        // their legacy pointer convention.
                        Some(ref ct @ CType::ExtVector(..))
                            if !crate::common::types::target_is_32bit() =>
                        {
                            Some(ct.size())
                        }
                        Some(CType::ComplexLongDouble) if !decomposes_cld => {
                            Some(CType::ComplexLongDouble.size())
                        }
                        // i686: ComplexDouble and ComplexFloat passed as structs on stack
                        Some(CType::ComplexDouble) if !decomposes_cd => {
                            Some(CType::ComplexDouble.size())
                        }
                        Some(CType::ComplexFloat) if !decomposes_cf => {
                            Some(CType::ComplexFloat.size())
                        }
                        Some(CType::Float128) | Some(CType::Decimal128) => Some(16),
                        _ => None,
                    }
                })
                .collect()
        };

        // Build struct_arg_aligns: for each struct arg, record its alignment.
        // This is used by RISC-V to even-align register pairs for 2×XLEN-aligned structs
        // (e.g., struct containing long double with 16-byte alignment).
        let struct_arg_aligns: Vec<Option<usize>> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                struct_arg_sizes.get(i).copied().flatten()?;
                let ctype = self.get_expr_ctype(a);
                match ctype {
                    Some(ref ct @ CType::Struct(_)) | Some(ref ct @ CType::Union(_)) => self
                        .get_struct_layout_for_ctype(ct)
                        .map(|layout| layout.align),
                    // _Float128/_Decimal128 args are 16-byte TFmode carriers:
                    // the i686 stack layout aligns them to 16 in the outgoing
                    // area (GCC i386 psABI), mirroring the struct_align Some(16)
                    // the parameter side registers (func_lowering).
                    Some(CType::Float128) | Some(CType::Decimal128) => Some(16),
                    // Vector varargs on x86-64: the SysV carrier alignment
                    // (16 for padded SSE shapes, storage alignment otherwise).
                    // 32-bit targets keep the legacy pointer convention.
                    // Vector varargs on x86-64: the SysV carrier alignment
                    // (16 for padded SSE shapes, storage alignment otherwise).
                    // Other targets keep their existing convention.
                    Some(ct) if ct.is_vector() && self.target == crate::backend::Target::X86_64 => {
                        Some(ct.align_ctx(&*self.types.borrow_struct_layouts()))
                    }
                    _ => None,
                }
            })
            .collect();

        // Build struct_arg_classes: propagate per-eightbyte SysV ABI classification from FuncSig.
        // For variadic args beyond fixed params, infer classification from expression CType
        // so that struct fields are correctly split between GP and SSE registers.
        let struct_arg_classes: Vec<Vec<crate::common::types::EightbyteClass>> =
            if let Some(ref classes) = func_name.and_then(|n| {
                self.func_meta
                    .sigs
                    .get(n)
                    .map(|s| s.param_struct_classes.clone())
            }) {
                args.iter()
                    .enumerate()
                    .map(|(i, a)| {
                        if i < classes.len() && !classes[i].is_empty() {
                            classes[i].clone()
                        } else {
                            // Infer eightbyte classification from expression CType for variadic struct args
                            self.infer_struct_eightbyte_classes(a)
                        }
                    })
                    .collect()
            } else {
                args.iter()
                    .map(|a| self.infer_struct_eightbyte_classes(a))
                    .collect()
            };

        // Build struct_arg_riscv_float_classes: propagate RISC-V float classification from FuncSig.
        // Variadic args beyond fixed params get None — on RISC-V LP64D, variadic
        // struct args are passed in GP registers, not FP registers.
        let struct_arg_riscv_float_classes: Vec<Option<crate::common::types::RiscvFloatClass>> =
            if let Some(ref classes) = func_name.and_then(|n| {
                self.func_meta
                    .sigs
                    .get(n)
                    .map(|s| s.param_riscv_float_classes.clone())
            }) {
                args.iter()
                    .enumerate()
                    .map(|(i, _a)| if i < classes.len() { classes[i] } else { None })
                    .collect()
            } else {
                args.iter().map(|_a| None).collect()
            };

        (
            arg_vals,
            arg_types,
            struct_arg_sizes,
            struct_arg_aligns,
            struct_arg_classes,
            struct_arg_riscv_float_classes,
        )
    }

    /// Infer SysV ABI eightbyte classification for a struct argument expression.
    /// This is used for variadic struct args that don't have pre-registered classification
    /// from the function signature (since the signature only covers fixed params).
    fn infer_struct_eightbyte_classes(
        &self,
        expr: &Expr,
    ) -> Vec<crate::common::types::EightbyteClass> {
        if let Some(ctype) = self.get_expr_ctype(expr) {
            // Vector-shaped variadic arguments classify through the SysV
            // vector carrier, not through struct layouts: an 8-byte vector is
            // ONE SSE eightbyte (low half of an XMM), a 9..=16-byte padded
            // vector is the full register, bit-packed bool carriers and the
            // tracked one-lane float shapes are INTEGER.  Before this, the
            // lookup returned an empty classification (= MEMORY) and every
            // vector vararg degraded to the pointer convention, which no
            // other x86-64 compiler reads (measured: GCC 16 passes
            // vector_size(8)/(16) varargs in XMM and counts them in %al).
            if self.target == crate::backend::Target::X86_64 {
                if let Some((classes, _pair)) = ctype.sysv_vector_carrier() {
                    return classes;
                }
            }
            if let Some(layout) = self.get_struct_layout_for_ctype(&ctype) {
                return layout.classify_sysv_eightbytes(&*self.types.borrow_struct_layouts());
            }
        }
        Vec::new()
    }

    /// Emit the actual call instruction (direct, indirect via fptr, or general indirect).
    /// Returns the effective return type for narrowing.
    fn emit_call_instruction(
        &mut self,
        func: &Expr,
        dest: Value,
        arg_vals: Vec<Operand>,
        arg_types: Vec<IrType>,
        struct_arg_sizes: Vec<Option<usize>>,
        struct_arg_aligns: Vec<Option<usize>>,
        struct_arg_classes: Vec<Vec<crate::common::types::EightbyteClass>>,
        struct_arg_riscv_float_classes: Vec<Option<crate::common::types::RiscvFloatClass>>,
        struct_arg_is_f128_sse: Vec<bool>,
        is_variadic: bool,
        num_fixed_args: usize,
        two_reg_size: Option<usize>,
        sret_size: Option<usize>,
        call_ret_classes: Vec<crate::common::types::EightbyteClass>,
        indirect_ret_is_one_xmm: bool,
    ) -> IrType {
        let mut indirect_ret_ty = self.get_func_ptr_return_ir_type(func);
        if two_reg_size.is_some() {
            indirect_ret_ty = IrType::I128;
        }

        match func {
            Expr::Identifier(name, _) => {
                if self.is_func_ptr_variable(name) {
                    // Function pointer variable: load pointer and call indirect
                    let func_ptr = self.load_func_ptr_variable(name);
                    // TODO: Support fastcall through function pointers (requires tracking
                    // calling convention in function pointer types)
                    self.emit(Instruction::CallIndirect {
                        func_ptr: Operand::Value(func_ptr),
                        info: CallInfo {
                            dest: Some(dest),
                            args: arg_vals,
                            arg_types,
                            return_type: indirect_ret_ty,
                            is_variadic,
                            num_fixed_args,
                            struct_arg_sizes,
                            struct_arg_aligns,
                            struct_arg_classes,
                            struct_arg_riscv_float_classes,
                            struct_arg_is_f128_sse: struct_arg_is_f128_sse.clone(),
                            is_sret: sret_size.is_some(),
                            is_fastcall: false,
                            regparm: None,
                            is_pure: false,
                            is_const: false,
                            ret_eightbyte_classes: call_ret_classes,
                            ret_is_f128_sse: indirect_ret_is_one_xmm,
                        },
                    });
                    indirect_ret_ty
                } else {
                    // Direct call - apply __asm__("label") linker symbol redirect if present
                    let call_name = self.resolve_ref_name(name.as_str());
                    let sig = self.func_meta.sigs.get(name.as_str());
                    let mut ret_ty = sig.map(|s| s.return_type).unwrap_or(target_int_ir_type());
                    if sig.and_then(|s| s.two_reg_ret_size).is_some() {
                        ret_ty = IrType::I128;
                    }
                    // On 32-bit targets, all struct/union returns use sret (the
                    // i386 SysV ABI never returns structs in registers). This
                    // block handles the fallback case where an indirect call
                    // has a missing signature.
                    if crate::common::types::target_is_32bit() {
                        if let Some(s) = sig {
                            if s.sret_size.is_none() && s.two_reg_ret_size.is_none() {
                                if let Some(ref rc) = s.return_ctype {
                                    if rc.is_struct_or_union() {
                                        ret_ty = IrType::Ptr;
                                    }
                                }
                            }
                        }
                    }
                    let callee_is_fastcall = self.fastcall_functions.contains(name.as_str());
                    let mut callee_regparm = self.regparm_functions.get(name.as_str()).copied();
                    if callee_is_fastcall {
                        callee_regparm = None;
                    }
                    let is_pure = self.pure_functions.contains(name.as_str());
                    let is_const = self.const_functions.contains(name.as_str());
                    let call_ret_is_f128_sse = sig.map(|s| s.ret_is_f128_sse).unwrap_or(false);
                    self.emit(Instruction::Call {
                        func: call_name,
                        info: CallInfo {
                            dest: Some(dest),
                            args: arg_vals,
                            arg_types,
                            return_type: ret_ty,
                            is_variadic,
                            num_fixed_args,
                            struct_arg_sizes,
                            struct_arg_aligns,
                            struct_arg_classes,
                            struct_arg_riscv_float_classes,
                            struct_arg_is_f128_sse: struct_arg_is_f128_sse.clone(),
                            is_sret: sret_size.is_some(),
                            is_fastcall: callee_is_fastcall,
                            regparm: callee_regparm,
                            is_pure,
                            is_const,
                            ret_eightbyte_classes: call_ret_classes,
                            ret_is_f128_sse: call_ret_is_f128_sse,
                        },
                    });
                    ret_ty
                }
            }
            Expr::Deref(inner, _) => {
                // In C, dereferencing a function pointer is a no-op: (*fp)(args) == fp(args).
                // But dereferencing a pointer-to-function-pointer is a real load:
                // (*fpp)(args) where fpp is func_ptr* needs to load the func_ptr first.
                // Use the comprehensive is_function_pointer_deref check which also handles
                // cases where c_type is None but a declared fptr_sig or known_functions provide info.
                let is_noop_deref = self.is_function_pointer_deref(inner);
                // A call through a dereferenced function pointer still follows
                // the pointed-to type's signature: if that type declares `...`,
                // the call IS variadic (floats belong in GP regs on rv64, AL=1
                // on x86-64, GP-only on aarch64).  The hard-coded `false` sent
                // the 5.0 of `(*fp)(buf, "%.0f", 5.0)` to fa0 instead of a2 and
                // aborted glibc's vsprintf on rv64 (gcc.c-torture 930513-1).
                let deref_variadic = match &**inner {
                    Expr::Identifier(name, _) => self.fptr_variable_is_variadic(name, inner),
                    _ => false,
                };
                let n = arg_vals.len();
                let sas = struct_arg_sizes;
                let saa = struct_arg_aligns;
                let sac = struct_arg_classes;
                let sarfc = struct_arg_riscv_float_classes;
                let func_ptr = if is_noop_deref {
                    // No-op dereference: (*fp)() == fp()
                    self.lower_expr(inner)
                } else {
                    // Real dereference needed: load through the pointer to get the func ptr
                    self.lower_expr(func)
                };
                self.emit(Instruction::CallIndirect {
                    func_ptr,
                    info: CallInfo {
                        dest: Some(dest),
                        args: arg_vals,
                        arg_types,
                        return_type: indirect_ret_ty,
                        is_variadic: deref_variadic,
                        num_fixed_args: n,
                        struct_arg_sizes: sas,
                        struct_arg_aligns: saa,
                        struct_arg_classes: sac,
                        struct_arg_riscv_float_classes: sarfc,
                        struct_arg_is_f128_sse: struct_arg_is_f128_sse.clone(),
                        is_sret: sret_size.is_some(),
                        is_fastcall: false,
                        regparm: None,
                        is_pure: false,
                        is_const: false,
                        ret_eightbyte_classes: call_ret_classes,
                        // AUD-VABI-1: the same carrier rule as the other
                        // indirect arms -- see `indirect_vector_carrier`.
                        ret_is_f128_sse: indirect_ret_is_one_xmm,
                    },
                });
                indirect_ret_ty
            }
            _ => {
                let sas = struct_arg_sizes;
                let saa = struct_arg_aligns;
                let sac = struct_arg_classes;
                let sarfc = struct_arg_riscv_float_classes;
                let func_ptr = self.lower_expr(func);

                // Check if the lowered expression is a GlobalAddr of a known function.
                // This handles patterns like ({ ...; (&func); })(args) where a
                // statement expression yields a function address. The kernel's
                // static_call mechanism uses this pattern and requires a direct call
                // instruction so the call site can be patched at runtime.
                let direct_func_name = if let Operand::Value(v) = func_ptr {
                    let instrs = &self
                        .func_state
                        .as_ref()
                        .expect("func_state must exist during function lowering")
                        .instrs;
                    let found = instrs.iter().rev().find_map(|inst| {
                        if let Instruction::GlobalAddr { dest, ref name } = *inst {
                            if dest == v && self.known_functions.contains(name) {
                                return Some(name.clone());
                            }
                        }
                        None
                    });
                    found
                } else {
                    None
                };

                if let Some(call_name) = direct_func_name {
                    // Emit a direct call instead of indirect.
                    let call_name = self.resolve_ref_name(call_name.as_str());
                    let sig = self.func_meta.sigs.get(call_name.as_str());
                    let mut ret_ty = sig.map(|s| s.return_type).unwrap_or(indirect_ret_ty);
                    if sig.and_then(|s| s.two_reg_ret_size).is_some() {
                        ret_ty = IrType::I128;
                    }
                    if crate::common::types::target_is_32bit() {
                        if let Some(s) = sig {
                            if s.sret_size.is_none() && s.two_reg_ret_size.is_none() {
                                if let Some(ref rc) = s.return_ctype {
                                    if rc.is_struct_or_union() {
                                        ret_ty = IrType::I64;
                                    }
                                }
                            }
                        }
                    }
                    let callee_is_fastcall = self.fastcall_functions.contains(call_name.as_str());
                    let mut callee_regparm =
                        self.regparm_functions.get(call_name.as_str()).copied();
                    if callee_is_fastcall {
                        callee_regparm = None;
                    }
                    let is_pure = self.pure_functions.contains(call_name.as_str());
                    let is_const = self.const_functions.contains(call_name.as_str());
                    let call_ret_is_f128_sse = sig.map(|s| s.ret_is_f128_sse).unwrap_or(false);
                    self.emit(Instruction::Call {
                        func: call_name,
                        info: CallInfo {
                            dest: Some(dest),
                            args: arg_vals,
                            arg_types,
                            return_type: ret_ty,
                            is_variadic,
                            num_fixed_args,
                            struct_arg_sizes: sas,
                            struct_arg_aligns: saa,
                            struct_arg_classes: sac,
                            struct_arg_riscv_float_classes: sarfc,
                            struct_arg_is_f128_sse: struct_arg_is_f128_sse.clone(),
                            is_sret: sret_size.is_some(),
                            is_fastcall: callee_is_fastcall,
                            regparm: callee_regparm,
                            is_pure,
                            is_const,
                            ret_eightbyte_classes: call_ret_classes,
                            ret_is_f128_sse: call_ret_is_f128_sse,
                        },
                    });
                    ret_ty
                } else {
                    self.emit(Instruction::CallIndirect {
                        func_ptr,
                        info: CallInfo {
                            dest: Some(dest),
                            args: arg_vals,
                            arg_types,
                            return_type: indirect_ret_ty,
                            is_variadic,
                            num_fixed_args,
                            struct_arg_sizes: sas,
                            struct_arg_aligns: saa,
                            struct_arg_classes: sac,
                            struct_arg_riscv_float_classes: sarfc,
                            struct_arg_is_f128_sse: struct_arg_is_f128_sse.clone(),
                            is_sret: sret_size.is_some(),
                            is_fastcall: false,
                            regparm: None,
                            is_pure: false,
                            is_const: false,
                            ret_eightbyte_classes: call_ret_classes,
                            // AUD-VABI-1: the same carrier rule as the other
                            // indirect arms -- see `indirect_vector_carrier`.
                            ret_is_f128_sse: indirect_ret_is_one_xmm,
                        },
                    });
                    indirect_ret_ty
                }
            }
        }
    }

    /// Load a function pointer from a local or global variable.
    fn load_func_ptr_variable(&mut self, name: &str) -> Value {
        let base_addr = if let Some(info) = self.func_mut().locals.get(name).cloned() {
            if let Some(ref global_name) = info.static_global_name {
                let addr = self.fresh_value();
                self.emit(Instruction::GlobalAddr {
                    dest: addr,
                    name: global_name.clone(),
                });
                addr
            } else {
                info.alloca
            }
        } else {
            // Global function pointer
            let addr = self.fresh_value();
            self.emit(Instruction::GlobalAddr {
                dest: addr,
                name: name.to_string(),
            });
            addr
        };
        let ptr_val = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: ptr_val,
            ptr: base_addr,
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
        });
        ptr_val
    }

    /// Narrow call result if return type is smaller than the widened operation type.
    /// 128-bit return values are already correctly handled.
    pub(super) fn maybe_narrow_call_result(&mut self, dest: Value, ret_ty: IrType) -> Operand {
        let wt = crate::common::types::widened_op_type(ret_ty);
        if ret_ty != wt && ret_ty != IrType::Ptr && ret_ty != IrType::Void && ret_ty.is_integer() {
            let narrowed = self.emit_cast_val(Operand::Value(dest), wt, ret_ty);
            Operand::Value(narrowed)
        } else {
            Operand::Value(dest)
        }
    }

    /// Check if a function is variadic.
    pub(super) fn is_function_variadic(&self, name: &str) -> bool {
        if let Some(sig) = self.func_meta.sigs.get(name) {
            return sig.is_variadic;
        }
        matches!(
            name,
            "printf"
                | "fprintf"
                | "sprintf"
                | "snprintf"
                | "scanf"
                | "sscanf"
                | "fscanf"
                | "dprintf"
                | "vprintf"
                | "vfprintf"
                | "vsprintf"
                | "vsnprintf"
                | "syslog"
                | "err"
                | "errx"
                | "warn"
                | "warnx"
                | "asprintf"
                | "vasprintf"
                | "open"
                | "fcntl"
                | "ioctl"
                | "execl"
                | "execlp"
                | "execle"
        )
    }

    /// Extract the return CType from a function pointer expression.
    /// Used to detect complex return types for indirect calls.
    pub(super) fn get_func_ptr_return_ctype(&self, func_expr: &Expr) -> Option<CType> {
        let mut expr = func_expr;
        loop {
            if let Some(ctype) = self.get_expr_ctype(expr) {
                return Self::extract_return_ctype_from_func_type(&ctype);
            }
            if let Expr::Deref(inner, _) = expr {
                expr = inner;
            } else {
                return None;
            }
        }
    }

    /// Extract the return CType from a function or function pointer CType.
    fn extract_return_ctype_from_func_type(ctype: &CType) -> Option<CType> {
        match ctype {
            CType::Pointer(inner, _) => match inner.as_ref() {
                CType::Function(ft) => Some(ft.return_type.clone()),
                _ => None,
            },
            CType::Function(ft) => Some(ft.return_type.clone()),
            _ => None,
        }
    }

    /// Determine the return type of a function pointer expression for indirect calls.
    /// Strips Deref layers since dereferencing a function pointer is a no-op in C.
    fn get_func_ptr_return_ir_type(&self, func_expr: &Expr) -> IrType {
        if let Some(ctype) = self.get_expr_ctype(func_expr) {
            return self.extract_return_type_from_ctype(&ctype);
        }
        // Strip all Deref layers (dereferencing function pointers is a no-op)
        let mut expr = func_expr;
        while let Expr::Deref(inner, _) = expr {
            expr = inner;
            if let Some(ctype) = self.get_expr_ctype(expr) {
                return self.extract_return_type_from_ctype(&ctype);
            }
        }
        target_int_ir_type()
    }

    pub(super) fn extract_return_type_from_ctype(&self, ctype: &CType) -> IrType {
        let returns_cld_in_regs = self.returns_complex_long_double_in_regs();
        match ctype {
            CType::Pointer(inner, _) => match inner.as_ref() {
                CType::Function(ft) => {
                    Self::func_return_ir_type(&ft.return_type, returns_cld_in_regs)
                }
                CType::Pointer(ret, _) => match ret.as_ref() {
                    CType::Float => IrType::F32,
                    CType::Double => IrType::F64,
                    _ => target_int_ir_type(),
                },
                CType::Float => IrType::F32,
                CType::Double => IrType::F64,
                _ => target_int_ir_type(),
            },
            CType::Function(ft) => Self::func_return_ir_type(&ft.return_type, returns_cld_in_regs),
            _ => target_int_ir_type(),
        }
    }

    /// Map a function's return CType to the IR type used for the call instruction.
    /// Complex types return in FP registers (F64 for both ComplexFloat and ComplexDouble),
    /// not as Ptr which is what IrType::from_ctype gives.
    fn func_return_ir_type(ret_ctype: &CType, _returns_cld_in_regs: bool) -> IrType {
        let is_32bit = crate::common::types::target_is_32bit();
        if is_32bit {
            if let Some(words) = ret_ctype.i686_ext_vector_gp_word_count() {
                return match words {
                    1 => IrType::I32,
                    2 => IrType::I64,
                    3 => IrType::I128,
                    _ => unreachable!("i686 ext-vector GP return word count is bounded"),
                };
            }
        }
        match ret_ctype {
            // Complex float:
            //   x86-64: both F32 parts packed into xmm0 as F64
            //   ARM/RISC-V: real part in first FP reg as F32
            //   i686: both F32 parts packed into eax:edx as I64
            CType::ComplexFloat => {
                if is_32bit {
                    IrType::I64
                } else {
                    IrType::F64
                }
            }
            // Complex double:
            //   x86-64/ARM64/RISC-V: real part in first FP reg as F64
            //   i686: 16 bytes, uses sret (shouldn't reach here, but default to Ptr)
            CType::ComplexDouble => {
                if is_32bit {
                    IrType::Ptr
                } else {
                    IrType::F64
                }
            }
            // Complex long double:
            //   x86-64: returns via x87 st(0)/st(1), real part as F128
            //   i686: 24 bytes, uses sret (shouldn't reach here, but default to Ptr)
            //   Other targets: handled via sret before reaching this point
            CType::ComplexLongDouble => {
                if is_32bit {
                    IrType::Ptr
                } else {
                    IrType::F128
                }
            }
            // On 32-bit targets, ALL struct/union returns use sret (hidden pointer).
            // The i386 SysV ABI never returns structs in registers.
            // This case shouldn't normally be reached for sret calls, but return Ptr
            // as a safe default matching the hidden pointer convention.
            CType::Struct(_) | CType::Union(_) if is_32bit => IrType::Ptr,
            other => IrType::from_ctype(other),
        }
    }

    /// Extract parameter type information from a function pointer expression's CType.
    /// This is used as a fallback when the callee is a complex expression (e.g.,
    /// struct member function pointer like `stats->compute_stats(...)`) and we
    /// don't have a named function signature to look up.
    ///
    /// Returns `Some((param_types, param_ctypes, param_bool_flags))` if the
    /// callee's CType is a function pointer with known parameter types.
    fn extract_fn_ptr_param_info(
        &self,
        func_expr: &Expr,
    ) -> Option<(Vec<IrType>, Vec<CType>, Vec<bool>, bool)> {
        let ctype = self.get_expr_ctype(func_expr)?;
        let ft = ctype.get_function_type()?;
        if ft.params.is_empty() {
            return None; // Unprototyped function - no parameter info
        }
        let param_types: Vec<IrType> = ft
            .params
            .iter()
            .map(|(ct, _)| IrType::from_ctype(ct))
            .collect();
        let param_ctypes: Vec<CType> = ft.params.iter().map(|(ct, _)| ct.clone()).collect();
        let param_bool_flags: Vec<bool> = ft
            .params
            .iter()
            .map(|(ct, _)| matches!(ct, CType::Bool))
            .collect();
        Some((param_types, param_ctypes, param_bool_flags, ft.variadic))
    }
}
