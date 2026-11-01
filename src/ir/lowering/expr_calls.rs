//! Function call lowering: argument preparation, call dispatch, and result handling.
//!
//! Extracted from expr.rs. This module handles:
//! - `lower_function_call`: the main entry point for FunctionCall expressions
//! - `lower_call_arguments`: argument evaluation with implicit casts and promotions
//! - `emit_call_instruction`: dispatch to direct, function-pointer, or indirect calls
//! - `classify_struct_return`: shared sret/two-reg classification logic
//! - Helpers: maybe_narrow_call_result, is_function_variadic, get_func_ptr_return_ir_type

use super::aggregate_abi::AggregateReturn;
use super::lower::Lowerer;
use crate::common::types::{AddressSpace, CType, IrType, target_int_ir_type};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{CallInfo, Instruction, IrBinOp, IrConst, Operand, Terminator, Value};

impl Lowerer {
    /// Whether a freshly lowered operand's defining instruction materialises a
    /// pointer. Semantic expression recovery is best-effort; this def-site
    /// query is the narrow tie-breaker used at call-conversion boundaries.
    fn operand_is_materialized_pointer(&self, operand: Operand) -> bool {
        let Operand::Value(value) = operand else {
            return false;
        };
        self.func_state
            .as_ref()
            .and_then(|state| {
                state
                    .instrs
                    .iter()
                    .rev()
                    .find(|inst| inst.dest() == Some(value))
            })
            .and_then(Instruction::result_type)
            == Some(IrType::Ptr)
    }

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
    ///
    /// The same holds at file scope, and the `known_functions` veto that used to
    /// exempt it was the bug: `globals` only ever holds *objects* (see
    /// `global_decl.rs`/`stmt_init.rs`), while `known_functions` holds every
    /// function name sema reported -- including the implicit library prototypes
    /// `declare_implicit_functions` pre-declares (`strlen`, `memcpy`, `malloc`,
    /// `printf`, ...) and the basenames of nested functions. A name present in
    /// both is therefore exactly a data object shadowing a library name, which
    /// is the one case that must answer "indirect call". Answering "direct call"
    /// instead emitted `call strlen` against a `size_t (*)(const char *)` object,
    /// the linker happily bound it to that object's address, and the program
    /// jumped into its own data (measured: SIGSEGV, gcc 14.2 exits 0). A C
    /// program cannot declare a function and an object of one name in the same
    /// scope (C11 6.7p4), so no valid program loses a direct call here.
    pub(super) fn is_func_ptr_variable(&self, name: &str) -> bool {
        // When called outside a function context (e.g., during global constant
        // evaluation), there are no locals, so this is not a func ptr variable.
        let func_state = match self.func_state.as_ref() {
            Some(fs) => fs,
            None => return self.globals.contains_key(name),
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
            // No local with this name: a global object of this name is a pointer
            // to call through, whatever else shares the basename.
            self.globals.contains_key(name)
        }
    }

    /// True when `name`, **as spelled at the use site**, denotes something other
    /// than the library function of that name: a pointer object at any scope
    /// (`is_func_ptr_variable`) or a GNU nested function visible here
    /// (`lookup_nested_function`). Those are the predicates `lower_function_call`
    /// uses to choose between a direct call, an indirect call and the static-chain
    /// path, so a fold that consults them agrees with the call the lowerer emits
    /// by construction -- a rule restated in two vocabularies is a rule that
    /// drifts apart.
    ///
    /// Keying on the spelled name is also what keeps the compiler's own interface
    /// foldable: a shadowing declaration carries the bare basename, so a
    /// `__builtin_*` spelling never matches one. Measured against gcc 14.2 inside
    /// a function that defines a nested `strlen` returning 91: bare
    /// `strlen("abc")` calls the nested function and yields 91, while
    /// `__builtin_strlen("abc")` still folds to 3 -- including in constant
    /// expression contexts (`enum { E = __builtin_strlen("abc") }`,
    /// `int arr[__builtin_strlen("abcd")]`).
    pub(super) fn library_name_is_shadowed(&self, name: &str) -> bool {
        self.is_func_ptr_variable(name) || self.lookup_nested_function(name).is_some()
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
        // i686 ext vectors returned in registers (lane words, one floating
        // lane, or one XMM piece); the call result is rebuilt into an object.
        let i686_ext_vector_reg_return_ctype = if crate::common::types::target_is_32bit() {
            call_return_ct
                .clone()
                .filter(|ct| ct.i686_ext_vector_ret_carrier().is_some())
        } else {
            None
        };

        // Determine sret/two-reg return convention
        // `f128_sret_ret`: the callee returns a _Float128/_Decimal128 through
        // the i686 hidden-pointer convention — the call result is the sret
        // buffer ADDRESS, and the 16-byte carrier value must be loaded out of
        // it (see the sret result handling below).
        let mut f128_sret_ret = false;
        // The return convention of this call: the callee's signature for a
        // direct call; for an indirect call (no FuncSig) the callee
        // expression's own function type, classified like a definition.
        let mut ret_abi = self.call_aggregate_return(stripped_func);
        if let Expr::Identifier(name, _) = stripped_func {
            if !self.is_func_ptr_variable(name) {
                f128_sret_ret = self
                    .func_meta
                    .sigs
                    .get(name.as_str())
                    .is_some_and(|s| s.ret_is_f128_sse && s.sret_size.is_some());
            }
        }
        if let Some((_, classes)) = i686_ext_vector_reg_return_ctype
            .as_ref()
            .and_then(CType::i686_ext_vector_ret_carrier)
        {
            // Carrier markers for the backend: three INTEGER classes for the
            // EAX:EDX:ECX lane-word bundle, `[Sse, SseUp]` for `%xmm0`.
            if !classes.is_empty() {
                ret_abi.classes = classes;
            }
        }
        let sret_size = ret_abi.sret_size;
        let two_reg_size = ret_abi.two_reg_size;

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
        self.diagnose_call_ext_vector_abi(
            stripped_func,
            call_return_ct.as_ref(),
            &param_ctypes_for_decompose,
            args,
        );
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
        // `_Float128`/`_Decimal128` scalars: the operand is the 16-byte value
        // itself (backend class F128SseReg). A 16-byte `[SSE, SSEUP]`
        // aggregate or vector also travels in one XMM register, but its
        // operand is an address: the backend recognises it from the
        // eightbyte classes (StructXmmReg), so it must NOT be flagged here.
        let original_f128_flags: Vec<bool> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if let Some(p) = param_ctypes_for_decompose.as_ref().and_then(|v| v.get(i)) {
                    return matches!(p, CType::Float128 | CType::Decimal128) || p.is_half_float();
                }
                self.get_expr_ctype(a).is_some_and(|ct| {
                    matches!(ct, CType::Float128 | CType::Decimal128) || ct.is_half_float()
                })
            })
            .collect();
        let mut struct_arg_is_f128_sse = Vec::with_capacity(arg_vals.len());
        for (arg_idx, flag) in original_f128_flags.into_iter().enumerate() {
            if let Some((count, xmm_pieces)) = ext_vector_arg_lanes.get(arg_idx).copied().flatten()
            {
                // XMM pieces reuse the 16-byte stack alignment of the i686
                // `__float128` carrier; scalarized lanes are plain slots.
                struct_arg_is_f128_sse.extend(std::iter::repeat_n(xmm_pieces, count));
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
                    // The hidden sret pointer is argument 0 and counts as
                    // named: `num_fixed_args` indexes `args`.
                    let sret_slots = usize::from(sret_alloca.is_some());
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
                                    CType::ExtVector(..)
                                        if crate::common::types::target_is_32bit() =>
                                    {
                                        ct.i386_ext_vector_ir_args()
                                    }
                                    _ => 1,
                                }
                            })
                            .sum::<usize>()
                            + sret_slots
                    } else if !sig.param_types.is_empty() {
                        sig.param_types.len() + sret_slots
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
            &ret_abi,
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

        if let Some(vector_ct) = i686_ext_vector_reg_return_ctype.as_ref() {
            return self.unpack_i686_ext_vector_reg_return(dest, call_ret_ty, vector_ct);
        }
        // X87-class aggregate (`struct { long double }`): the call produced
        // the long double from %st(0); give it the aggregate's storage. The
        // two-lane `long double` ext vector also pops lane 1 from %st(1)
        // (`GetReturnF128Second`, as for `_Complex long double`) into the
        // 32-byte, 32-aligned vector object (lanes at 0 and 10).
        if ret_abi.x87 || ret_abi.x87_pair {
            let lane1 = ret_abi.x87_pair.then(|| {
                let lane1 = self.fresh_value();
                self.emit(Instruction::GetReturnF128Second { dest: lane1 });
                lane1
            });
            let (size, align) = if ret_abi.x87_pair { (32, 32) } else { (16, 16) };
            let alloca = self.fresh_value();
            self.emit(Instruction::Alloca {
                dest: alloca,
                ty: IrType::F128,
                size,
                align,
                volatile: false,
                semantic_volatile: false,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(dest),
                ptr: alloca,
                ty: IrType::F128,
                seg_override: AddressSpace::Default,
            });
            if let Some(lane1) = lane1 {
                // Lane 1 sits 10 bytes in (`X87_LANE_STRIDE`); a 16-byte
                // F128 store there would be fine for the padding but the
                // lane helper keeps one definition of the x87 lane image.
                let lane1_ptr = self.emit_gep_offset(
                    alloca,
                    crate::common::types::X87_LANE_STRIDE,
                    IrType::F128,
                );
                self.store_x87_lane(
                    lane1_ptr,
                    Operand::Value(lane1),
                    false,
                    AddressSpace::Default,
                );
            }
            return Operand::Value(alloca);
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
        //
        // A FILE-SCOPE pointer object shadows it exactly the same way and is
        // not in `locals`, which is how `static int (*abs)(int) = my_abs;`
        // still came back as the constant 5 for `abs(-5)` after the local test
        // below was added (measured against gcc 14.2, which calls `my_abs`).
        // `is_func_ptr_variable` is the predicate the rest of this function
        // already uses to choose between a direct and an indirect call, so
        // consulting it here keeps "expand the builtin" and "call the library
        // symbol" from disagreeing about what the identifier denotes. The local
        // test is kept alongside it: it also declines for a local that is not a
        // pointer at all, which is invalid C but must not be expanded either.
        let callee_is_local = matches!(func, Expr::Identifier(n, _)
            if self.func_state.as_ref().is_some_and(|fs| fs.locals.contains_key(n.as_str())));
        let Expr::Identifier(name, _) = func else {
            return None;
        };
        if callee_is_local || self.is_func_ptr_variable(name) {
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
    /// The argument of a GCC `transparent_union` parameter whose type is a
    /// MEMBER's, as the union's FIRST member value -- the scalar such a
    /// parameter travels as (GCC passes the union with its first member's
    /// ABI, and the callee side reads it so) -- with that member's IR type.
    /// None keeps the argument as lowered: `key` is not a transparent union
    /// with a scalar first member, or the argument is the union itself (an
    /// aggregate argument, whose ABI path loads the first member through
    /// its address). A member value used to be passed through unchanged,
    /// right only when it already had the first member's representation:
    /// `f(1.5f)` for `union { int i; float f; }` passed the float in %xmm0
    /// (GCC: its bits in %edi), and before the union was marked at all,
    /// `f(7)` was copied as a union from address 7.
    ///
    /// Sema admitted the argument (`transparent_member_accepts`): it matches
    /// a member exactly, is pointer-shaped (pointer, array, function
    /// designator) for a pointer member, has an enum's integer type, or is a
    /// null pointer constant for a pointer member. It is converted to that
    /// member, then reinterpreted as the first member through a union
    /// temporary unless both share an IR type.
    fn transparent_union_arg(
        &mut self,
        val: Operand,
        arg_ty: IrType,
        arg_ct: Option<&CType>,
        key: &str,
    ) -> Option<(Operand, IrType)> {
        let ct = arg_ct.filter(|ct| !matches!(ct, CType::Union(k) if k.as_ref() == key))?;
        let (size, first, member) = {
            let layouts = self.types.borrow_struct_layouts();
            let layout = layouts.get(key).filter(|l| l.is_transparent_union)?;
            let members: Vec<&CType> = layout.fields.iter().map(|f| &f.ty).collect();
            let first = (*members.first()?).clone();
            if !(first.is_integer() || first.is_pointer_like()) {
                return None;
            }
            let is_ptr_member = |m: &&&CType| m.is_pointer_like();
            let pointer_shaped = ct.is_pointer_like() || matches!(ct, CType::Function(_));
            let member = members
                .iter()
                .find(|m| **m == ct)
                .or_else(|| {
                    pointer_shaped
                        .then(|| members.iter().find(is_ptr_member))
                        .flatten()
                })
                .or_else(|| {
                    ct.is_integer()
                        .then(|| {
                            members
                                .iter()
                                .find(|m| m.is_integer() && m.size() == ct.size())
                                .or_else(|| members.iter().find(is_ptr_member))
                        })
                        .flatten()
                })
                .map_or_else(|| first.clone(), |m| (*m).clone());
            (layout.size, first, member)
        };
        if member.is_struct_or_union() || member.is_vector() {
            return None;
        }
        let first_ty = IrType::from_ctype(&first);
        let member_ty = IrType::from_ctype(&member);
        let converted = self.emit_implicit_cast(val, arg_ty, member_ty);
        if member_ty == first_ty {
            return Some((converted, first_ty));
        }
        let alloca = self.fresh_value();
        let carrier = Self::packed_store_type(size);
        self.emit(Instruction::Alloca {
            dest: alloca,
            size,
            ty: carrier,
            align: 0,
            volatile: false,
            semantic_volatile: false,
        });
        // A member narrower than the union leaves the rest unspecified (GCC
        // passes whatever the stack held); zero it for a deterministic image.
        if self.ctype_size(&member) < size {
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::zero(carrier)),
                ptr: alloca,
                ty: carrier,
                seg_override: AddressSpace::Default,
            });
        }
        self.emit(Instruction::Store {
            volatile: false,
            val: converted,
            ptr: alloca,
            ty: member_ty,
            seg_override: AddressSpace::Default,
        });
        let dest = self.fresh_value();
        self.emit(Instruction::Load {
            dest,
            ptr: alloca,
            ty: first_ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        Some((Operand::Value(dest), first_ty))
    }

    /// Spill a 9..=16-byte aggregate returned in a register pair (RAX:RDX,
    /// x0:x1, a0:a1; carried as `I128`) into a fresh object.
    ///
    /// The high eightbyte is stored at the object's exact remaining width
    /// when that is a machine size (12-byte `{u32 x3}` -> one `U32` store at
    /// offset 8), so the stores match the member loads that read them back
    /// and stay inside the object. For the odd remainders (3, 5, 6, 7 bytes)
    /// the object is rounded up to the 16-byte register image and the high
    /// half is stored whole: storing a full I64 at offset 8 of a 12-byte
    /// object, as this used to, writes past the allocation in IR terms.
    fn unpack_two_reg_return(&mut self, dest: Value, size: usize) -> Operand {
        debug_assert!(
            (9..=16).contains(&size),
            "two-register aggregate of {size} bytes"
        );
        let (hi_ty, alloc_size) = match size.saturating_sub(8) {
            1 => (IrType::U8, size),
            2 => (IrType::U16, size),
            4 => (IrType::U32, size),
            8 => (IrType::I64, size),
            _ => (IrType::I64, 16),
        };
        let alloca = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: alloca,
            ty: IrType::Ptr,
            size: alloc_size,
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
            to_ty: hi_ty,
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
            ty: hi_ty,
            seg_override: AddressSpace::Default,
        });
        Operand::Value(alloca)
    }

    /// Rebuild an i686 Clang integer ExtVector from its EAX:EDX:ECX ABI word
    /// bundle. The call result is a scalar transport value, not the vector's
    /// padded object representation. Materialize the carrier in memory before
    /// loading lanes: i686 I128 arithmetic intentionally models the low 64-bit
    /// pair, so shifting an I128 by 64 would lose the ECX word.
    pub(super) fn unpack_i686_ext_vector_gp_return(
        &mut self,
        dest: Value,
        return_type: IrType,
        vector_ct: &CType,
    ) -> Operand {
        let words = vector_ct
            .i686_ext_vector_gp_word_count()
            .expect("caller only selects the i686 integer ExtVector GP ABI");
        let (elem_ct, lanes) = vector_ct.vector_lanes();
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
                let lane_value =
                    self.i386_bool_lane_bit(vector_ct, Operand::Value(lane_value), IrType::I32);
                self.store_vector_lane(
                    result,
                    vector_ct,
                    lane,
                    lane_value,
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

    /// The C type an argument is passed as: its prototype parameter's type,
    /// else (variadic or unprototyped calls) the argument expression's own.
    pub(super) fn call_arg_ctype(
        &self,
        param_ctypes: &Option<Vec<CType>>,
        arg_idx: usize,
        arg: &Expr,
    ) -> Option<CType> {
        match param_ctypes.as_ref().and_then(|p| p.get(arg_idx)) {
            Some(ct) => Some(ct.clone()),
            None => self.get_expr_ctype(arg),
        }
    }

    /// Lower i686 ExtVector call arguments to Clang 23.1.3's i386 carriers
    /// (`CType::i386_ext_vector_arg`): scalarized lanes, each its own stack
    /// slot; widened vectors as 16-byte `[Sse, SseUp]` pieces (`%xmm0-2`,
    /// then 16-byte-aligned stack); 8-byte integer images as a by-value
    /// 8-byte aggregate. Returns, per original argument, how many IR arguments it
    /// became and whether those are 16-byte-aligned XMM pieces, so the call
    /// metadata (fixed-argument count, carrier flags) can be expanded.
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
        call_is_variadic: bool,
    ) -> Vec<Option<(usize, bool)>> {
        use crate::common::types::{EightbyteClass, I386ExtVectorCarrier};
        let mut expansions = vec![None; args.len()];
        if !crate::common::types::target_is_32bit() {
            return expansions;
        }

        // No earlier i686 pass splits an argument (complex values travel as
        // structs), so operand i is still argument i.
        debug_assert_eq!(arg_vals.len(), args.len());
        let old_vals = std::mem::take(arg_vals);
        let old_types = std::mem::take(arg_types);
        let old_sizes = std::mem::take(struct_arg_sizes);
        let old_aligns = std::mem::take(struct_arg_aligns);
        let old_classes = std::mem::take(struct_arg_classes);
        let old_riscv_classes = std::mem::take(struct_arg_riscv_float_classes);
        let mut new_vals = Vec::with_capacity(old_vals.len());
        let mut new_types = Vec::with_capacity(old_types.len());
        let mut new_sizes = Vec::with_capacity(old_sizes.len());
        let mut new_aligns = Vec::with_capacity(old_aligns.len());
        let mut new_classes = Vec::with_capacity(old_classes.len());
        let mut new_riscv_classes = Vec::with_capacity(old_riscv_classes.len());

        for (cursor, &value) in old_vals.iter().enumerate() {
            let arg = args.get(cursor);

            let vector_ct = arg.and_then(|arg| self.call_arg_ctype(param_ctypes, cursor, arg));

            let carrier = vector_ct.as_ref().and_then(CType::i386_ext_vector_arg);
            // Captured before the let-else moves `vector_ct` (the else arm
            // decides the pre-SSE variadic 16-byte-class promotion).
            let variadic_16b_simd = call_is_variadic
                && vector_ct
                    .as_ref()
                    .is_some_and(|ct| ct.is_i386_16b_simd_vector());
            let (Some(arg), Some(vector_ct), Some(carrier)) = (arg, vector_ct, carrier) else {
                // A 16-byte SIMD vector with no XMM carrier (pre-SSE
                // baseline / -mno-sse) is ordinarily the plain 4-granular
                // by-value aggregate: the Clang-side fork of the measured
                // pre-SSE callee conventions (GCC 16.2 8-aligns the 16B
                // vector slot in fixed arguments — documented fork; lccc
                // follows Clang's 4-granular placement).
                //
                // Exception: in a VARIADIC call the GCC 4.6 va rule
                // 16-byte-aligns the 16-byte class inside the variadic area
                // on ANY baseline (measured: GCC 16.2 and Clang 23.1.3
                // callers both pad the vector to a 16-byte boundary, and
                // both callees realign the va_arg cursor). Emit the same
                // 16-byte `[Sse, SseUp]` piece the XMM carrier uses: the
                // variadic classify arm never assigns an XMM register to
                // it (always `StructByValStack { 16 }`), and the
                // `(pieces = 1, true)` expansion gives it the 16-byte
                // stack alignment the `__float128` carrier shares
                // (struct_arg_is_f128_sse).
                if variadic_16b_simd {
                    // The aggregate operand of a by-value vector already is
                    // the 16-byte object address (no XMM image needed).
                    let base = self.operand_to_value(value);
                    new_vals.push(Operand::Value(base));
                    new_types.push(IrType::Ptr);
                    new_sizes.push(Some(16));
                    new_aligns.push(Some(16));
                    new_classes.push(vec![EightbyteClass::Sse, EightbyteClass::SseUp]);
                    new_riscv_classes.push(None);
                    expansions[cursor] = Some((1, true));
                } else {
                    new_vals.push(value);
                    new_types.push(old_types.get(cursor).copied().unwrap_or(IrType::Ptr));
                    new_sizes.push(old_sizes.get(cursor).copied().flatten());
                    new_aligns.push(old_aligns.get(cursor).copied().flatten());
                    new_classes.push(old_classes.get(cursor).cloned().unwrap_or_default());
                    new_riscv_classes.push(old_riscv_classes.get(cursor).copied().flatten());
                }
                continue;
            };
            let base = self.operand_to_value(value);
            let address_space = self.aggregate_lvalue_space(arg);
            let volatile = self.expr_access_is_volatile(arg);
            match carrier {
                I386ExtVectorCarrier::Lanes => {
                    let (elem_ct, lanes) = vector_ct.vector_lanes();
                    let lane_ty = if matches!(elem_ct, CType::Bool) {
                        IrType::I32
                    } else {
                        IrType::from_ctype(elem_ct)
                    };
                    for lane in 0..lanes {
                        let lane_value =
                            self.load_vector_lane(base, &vector_ct, lane, address_space, volatile);
                        new_vals.push(lane_value);
                        new_types.push(lane_ty);
                        new_sizes.push(None);
                        new_aligns.push(None);
                        // `[NoClass]` tags this scalar as an i686 ExtVector
                        // lane; regparm/fastcall keep it on the stack without
                        // consuming their next ordinary GP argument register.
                        new_classes.push(vec![EightbyteClass::NoClass]);
                        new_riscv_classes.push(None);
                    }
                    expansions[cursor] = Some((lanes, false));
                }
                I386ExtVectorCarrier::Xmm { pieces } => {
                    let image = self.i386_ext_vector_xmm_image(
                        base,
                        &vector_ct,
                        pieces,
                        address_space,
                        volatile,
                    );
                    for piece in 0..pieces {
                        let ptr = if piece == 0 {
                            image
                        } else {
                            self.emit_gep_offset(image, 16 * piece, IrType::Ptr)
                        };
                        new_vals.push(Operand::Value(ptr));
                        new_types.push(IrType::Ptr);
                        new_sizes.push(Some(16));
                        new_aligns.push(Some(16));
                        new_classes.push(vec![EightbyteClass::Sse, EightbyteClass::SseUp]);
                        new_riscv_classes.push(None);
                    }
                    expansions[cursor] = Some((pieces, true));
                }
                I386ExtVectorCarrier::Image64
                | I386ExtVectorCarrier::Unsupported
                | I386ExtVectorCarrier::Scalar
                | I386ExtVectorCarrier::GpLanes
                | I386ExtVectorCarrier::Sret => {
                    // (`Unsupported` shapes are diagnosed for calls that can
                    // reach foreign code; see diagnose_call_ext_vector_abi.)
                    // The object bytes as one by-value stack aggregate (an
                    // i64 image occupies two 4-byte-aligned slots).
                    new_vals.push(Operand::Value(base));
                    new_types.push(IrType::Ptr);
                    new_sizes.push(Some(vector_ct.size()));
                    new_aligns.push(Some(4));
                    new_classes.push(Vec::new());
                    new_riscv_classes.push(None);
                }
            }
        }
        *arg_vals = new_vals;
        *arg_types = new_types;
        *struct_arg_sizes = new_sizes;
        *struct_arg_aligns = new_aligns;
        *struct_arg_classes = new_classes;
        *struct_arg_riscv_float_classes = new_riscv_classes;
        expansions
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
                // The defining instruction is the final authority for a
                // materialised SSA value. CType/expression recovery is
                // necessarily heuristic in large preprocessed translation
                // units; if it disagrees with `Load { ty: Ptr }`, using its
                // I32 fallback here creates malformed IR and truncates the
                // pointer before the call (glibc vfprintf's end_of_fmt).
                if self.operand_is_materialized_pointer(val) {
                    // Only strengthen to Ptr. Aggregate rvalues deliberately
                    // use integer ABI carriers for small structs; adopting
                    // their carrier type breaks the struct-argument path.
                    arg_ty = IrType::Ptr;
                }

                // A raw GCC 128-bit vector builtin (__builtin_ia32_shufps,
                // __builtin_ia32_maxps, ...) yields its vector BY VALUE as a
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
                            return cast_val;
                        }
                    }
                }

                // Spill packed struct/vector data to a temporary alloca so
                // that the call argument is a pointer (address) rather than
                // raw data.  This is needed for any expression that produces
                // packed aggregate data: direct function calls, statement
                // expressions wrapping calls, ternaries, comma expressions,
                // etc.  The vector case (a <=8-byte SSE-class vector returning
                // in %xmm0 as a packed scalar) was previously missed, so
                // passing `f(v)` as an argument dereferenced the bit-pattern
                // as a pointer and segfaulted (AUD-VABI-4 chained-call
                // witness).
                {
                    // Spill when the expression yields PACKED data; keep the
                    // ADDRESS when it yields an object. `expr_produces_packed_
                    // struct_data` already classifies every carrier: a call
                    // result is packed iff the callee returns it in registers
                    // (GP, one-XMM, two-register, x87), while a MEMORY-class
                    // return — vector_size V1SF/V1DF per GCC 16.2.0 — comes
                    // back as the sret buffer ADDRESS and must be forwarded
                    // as an address; spilling that address as if it were the
                    // value pushed 8 bytes of pointer bits as the argument.
                    let produces_packed_aggregate = {
                        let ct = self.get_expr_ctype(a);
                        let is_agg = ct
                            .as_ref()
                            .is_some_and(|c| c.is_struct_or_union() || c.is_vector());
                        is_agg && self.expr_produces_packed_struct_data(a)
                    };
                    if produces_packed_aggregate {
                        let agg_size = if let Some(ct) = self.get_expr_ctype(a) {
                            if ct.is_vector() {
                                ct.size()
                            } else {
                                self.struct_value_size(a).unwrap_or(8)
                            }
                        } else {
                            self.struct_value_size(a).unwrap_or(8)
                        };
                        let alloc_size = if agg_size > 0 { agg_size } else { 8 };
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
                    // A by-value vector argument is now the ADDRESS of its
                    // object (packed carriers were spilled just above), whatever
                    // carrier type `get_expr_type` reports for a call result
                    // (i686 ext vectors: I32/I64/I128 words, an x87 lane, or an
                    // XMM image). Casting that carrier type to the parameter's
                    // Ptr would reinterpret the address (an I128 cast is an
                    // ICE on i686).
                    if self.get_expr_ctype(a).is_some_and(|ct| ct.is_vector()) {
                        arg_ty = IrType::Ptr;
                    }
                }

                // A GCC transparent_union parameter travels as its FIRST
                // member (see `transparent_union_arg`).
                if let Some(CType::Union(key)) = param_ctypes.as_ref().and_then(|p| p.get(i)) {
                    let arg_ct = self.get_expr_ctype(a);
                    if let Some((v, ty)) =
                        self.transparent_union_arg(val, arg_ty, arg_ct.as_ref(), key)
                    {
                        val = v;
                        arg_ty = ty;
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
                            return self.emit_expr_bool_conversion(val, arg_ty, a);
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
                                if pctypes[i].is_soft_float() || arg_ct.is_soft_float() {
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
                // Default argument promotions (C11 6.5.2.2p6 / ISO/IEC TS 18661-3):
                // _Float16 and __bf16 do NOT undergo default argument promotion!
                let needs_promotion =
                    (pre_call_variadic || param_types.is_some() || is_unprototyped)
                        && !self.expr_ctype(a).is_half_float();
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

        // The C type that decides how each argument travels (C11 6.5.2.2p7).
        // A prototyped non-aggregate parameter receives the argument
        // converted as if by assignment, so the parameter's type decides:
        // a _Float128 converted to a `double` parameter is an F64 in an XMM
        // register, not a 16-byte carrier, and a `double` converted to a
        // _Float128 parameter IS one (taking the expression type passed the
        // 0/1 of `take_bool(q)` as the address of a 16-byte aggregate, and
        // misaligned the i686 slot of `f(1, d)` for `f(int, _Float128)`).
        // An aggregate argument already has its parameter's compatible type
        // and keeps the expression type, which also resolves vector typedefs
        // that a first-pass signature may lack; a transparent-union
        // parameter receives a member value; variadic and unprototyped
        // arguments are their (promoted) expression type.
        let arg_abi_ctypes: Vec<Option<CType>> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let expr_ct = self.get_expr_ctype(a);
                let is_aggregate = |ct: &CType| ct.is_struct_or_union() || ct.is_vector();
                match param_ctypes.as_ref().and_then(|p| p.get(i)) {
                    Some(p) if !is_aggregate(p) && !expr_ct.as_ref().is_some_and(is_aggregate) => {
                        Some(p.clone())
                    }
                    _ => expr_ct,
                }
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
                    let registered = if i < sizes.len() { sizes[i] } else { None };
                    if registered.is_some() {
                        return registered;
                    }
                    // Fall back to expression-based inference for unregistered types
                    let ctype = arg_abi_ctypes[i].clone();
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
                        // GNU and ext vectors alike are passed by value
                        // (an ext vector's object includes its padding).
                        Some(ref ct) if ct.is_vector() => Some(ct.size()),
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
                    let ctype = arg_abi_ctypes[i].clone();
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
                        // GNU and ext vectors alike are passed by value
                        // (an ext vector's object includes its padding).
                        Some(ref ct) if ct.is_vector() => Some(ct.size()),
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
        let struct_arg_aligns: Vec<Option<usize>> = arg_abi_ctypes
            .iter()
            .enumerate()
            .map(|(i, ctype)| {
                struct_arg_sizes.get(i).copied().flatten()?;
                match ctype {
                    Some(ct @ CType::Struct(_)) | Some(ct @ CType::Union(_)) => self
                        .get_struct_layout_for_ctype(ct)
                        .map(|layout| layout.align),
                    // Vectors carry their natural alignment (size for ext
                    // vectors: 32 for 32-byte, 64 for 64-byte). The x86-64
                    // backend uses this both for the per-argument padding in
                    // the outgoing area and to trigger the GCC ≥ 4.6 dynamic
                    // %rsp realignment (emit_call_stack_args_impl): a
                    // MEMORY-class argument with alignment A > 16 must sit at
                    // an address ≡ 0 (mod A), because every callee — GCC,
                    // Clang and lccc's own prologue/va_arg — rounds the
                    // incoming slot up to A. Without it a 32-byte vector was
                    // pushed at the 16-aligned %rsp and the callee read 16
                    // bytes of garbage on half of all stack residues
                    // (ABI-EXTVEC-VA-1). The parameter side already records
                    // `ctype_align` for vector params (func_lowering), so the
                    // caller and callee layouts agree. ARM (cap 16) clamps
                    // this through `stack_arg_align_cap`; i686 applies its
                    // explicit wide-GNU-vector natural-alignment exception.
                    Some(ct) if ct.is_vector() => Some(self.ctype_align(ct)),
                    // _Float128/_Decimal128 args are 16-byte TFmode carriers:
                    // the i686 stack layout aligns them to 16 in the outgoing
                    // area (GCC i386 psABI), mirroring the struct_align Some(16)
                    // the parameter side registers (func_lowering).
                    Some(CType::Float128) | Some(CType::Decimal128) => Some(16),
                    _ => None,
                }
            })
            .collect();

        let (struct_arg_classes, struct_arg_riscv_float_classes) =
            self.call_arg_abi_classes(func_name, args, &struct_arg_sizes);

        (
            arg_vals,
            arg_types,
            struct_arg_sizes,
            struct_arg_aligns,
            struct_arg_classes,
            struct_arg_riscv_float_classes,
        )
    }

    /// Per-argument ABI class tables for a call: the SysV eightbyte classes
    /// and the RISC-V float classes, taken from the callee's registered
    /// signature where it covers the argument and derived from the argument
    /// expression otherwise.  `struct_arg_sizes` is the by-value size table
    /// the caller built for the same `args`.
    fn call_arg_abi_classes(
        &self,
        func_name: Option<&str>,
        args: &[Expr],
        struct_arg_sizes: &[Option<usize>],
    ) -> (
        Vec<Vec<crate::common::types::EightbyteClass>>,
        Vec<Option<crate::common::types::RiscvFloatClass>>,
    ) {
        // Build struct_arg_classes: propagate per-eightbyte SysV ABI classification from FuncSig.
        // For variadic args beyond fixed params, infer classification from expression CType
        // so that struct fields are correctly split between GP and SSE registers.
        //
        // Invariant: an argument without a by-value size carries no classes.
        // Such an argument is a scalar or a pointer (a variable-size aggregate
        // passed by reference, see `struct_arg_sizes` above), and the backend
        // reads eightbyte classes only alongside a size -- except for the
        // i686 ExtVector lane marker, which the i686 decomposition attaches
        // AFTER this point and which `CallAbiConfig::i386_ext_vector_lanes`
        // gates to that target. Deriving the classes from an argument the
        // size pass declined would hand that marker's bytes to x86-64.
        let registered_classes = func_name.and_then(|n| {
            self.func_meta
                .sigs
                .get(n)
                .map(|s| s.param_struct_classes.clone())
        });
        let struct_arg_classes: Vec<Vec<crate::common::types::EightbyteClass>> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if struct_arg_sizes.get(i).copied().flatten().is_none() {
                    return Vec::new();
                }
                match registered_classes.as_ref().and_then(|c| c.get(i)) {
                    Some(classes) if !classes.is_empty() => classes.clone(),
                    // Infer eightbyte classification from the expression CType
                    // for variadic struct args beyond the declared parameters.
                    _ => self.infer_struct_eightbyte_classes(a),
                }
            })
            .collect();

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

        (struct_arg_classes, struct_arg_riscv_float_classes)
    }

    /// Infer SysV ABI eightbyte classification for a struct argument expression.
    /// This is used for variadic struct args that don't have pre-registered classification
    /// from the function signature (since the signature only covers fixed params).
    fn infer_struct_eightbyte_classes(
        &self,
        expr: &Expr,
    ) -> Vec<crate::common::types::EightbyteClass> {
        if let Some(ctype) = self.get_expr_ctype(expr) {
            // Native AVX vectors use the same register classification for
            // direct and indirect calls. Only the memory fallback carries
            // the i386 natural-stack-alignment marker.
            if let Some((classes, _)) = self.sysv_arg_abi(&ctype) {
                return classes;
            }
            if let Some(classes) = self.i386_gnu_vector_memory_classes(&ctype) {
                return classes;
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
        ret_abi: &AggregateReturn,
    ) -> IrType {
        let calling_convention = self
            .get_expr_ctype(func)
            .and_then(|ct| match ct {
                CType::Function(ft) => Some(ft.calling_convention),
                CType::Pointer(inner, _) => match *inner {
                    CType::Function(ft) => Some(ft.calling_convention),
                    _ => None,
                },
                _ => None,
            })
            .unwrap_or_default();
        let mut indirect_ret_ty = self.get_func_ptr_return_ir_type(func);
        if ret_abi.two_reg_size.is_some() {
            indirect_ret_ty = IrType::I128;
        } else if ret_abi.x87 || ret_abi.x87_pair {
            indirect_ret_ty = IrType::F128;
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
                            is_sret: ret_abi.sret_size.is_some(),
                            calling_convention,
                            is_fastcall: false,
                            regparm: None,
                            is_pure: false,
                            is_const: false,
                            assume_library: false,
                            ret_eightbyte_classes: ret_abi.classes.clone(),
                            ret_is_f128_sse: ret_abi.one_xmm,
                            cold: false,
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
                            is_sret: ret_abi.sret_size.is_some(),
                            calling_convention,
                            is_fastcall: callee_is_fastcall,
                            regparm: callee_regparm,
                            is_pure,
                            is_const,
                            assume_library: false,
                            ret_eightbyte_classes: ret_abi.classes.clone(),
                            ret_is_f128_sse: call_ret_is_f128_sse,
                            cold: false,
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
                        is_sret: ret_abi.sret_size.is_some(),
                        calling_convention,
                        is_fastcall: false,
                        regparm: None,
                        is_pure: false,
                        is_const: false,
                        assume_library: false,
                        ret_eightbyte_classes: ret_abi.classes.clone(),
                        // From the callee's type: `indirect_aggregate_return`.
                        ret_is_f128_sse: ret_abi.one_xmm,
                        cold: false,
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
                            is_sret: ret_abi.sret_size.is_some(),
                            calling_convention,
                            is_fastcall: callee_is_fastcall,
                            regparm: callee_regparm,
                            is_pure,
                            is_const,
                            assume_library: false,
                            ret_eightbyte_classes: ret_abi.classes.clone(),
                            ret_is_f128_sse: call_ret_is_f128_sse,
                            cold: false,
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
                            is_sret: ret_abi.sret_size.is_some(),
                            calling_convention,
                            is_fastcall: false,
                            regparm: None,
                            is_pure: false,
                            is_const: false,
                            assume_library: false,
                            ret_eightbyte_classes: ret_abi.classes.clone(),
                            // From the callee's type: `indirect_aggregate_return`.
                            ret_is_f128_sse: ret_abi.one_xmm,
                            cold: false,
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
            if let Some((carrier, _)) = ret_ctype.i686_ext_vector_ret_carrier() {
                return carrier;
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
            // AUD-VABI-4: a ONE-LANE FP vector's indirect-call IR carrier,
            // per spelling, exactly as the pinned oracles measure them:
            //   - `ext_vector_type` double: SSE, %xmm0 (Clang 23.1.3 grid)
            //     → F64.
            //   - `ext_vector_type` float: INTEGER bit pattern in
            //     %eax/%rax (the same grid; the scalar F32 demotion made
            //     the indirect caller read %xmm0 against an INTEGER callee
            //     — the fptr seam the one-lane fixtures caught at -O1).
            //   - `vector_size` V1SF/V1DF: MEMORY both ways (GCC 16.2.0),
            //     the call is sret and yields the buffer ADDRESS, like the
            //     direct-call signature carrier.
            // Before this arm existed at all, a vector return kept the Ptr
            // carrier that `from_ctype` assigns; the caller then fell
            // through to the pointer-return path (store %rax) while the
            // callee wrote %xmm0, and the result was read as garbage.
            other if !is_32bit && other.is_vector() && other.elem_is_fp_scalar() => {
                // No unwrap: `vector_info` is Some for every vector shape,
                // but the census ratchet counts panic sites — stay None-safe.
                let is_ext = matches!(other, CType::ExtVector(..));
                match other.vector_info().map(|(e, _)| e.size()) {
                    Some(8) if is_ext => IrType::F64,
                    Some(4) if is_ext => IrType::I32,
                    Some(_) if !is_ext => IrType::Ptr,
                    _ => IrType::I64,
                }
            }
            // Small (<=8-byte) integer/fp vectors that aren't one-lane FP:
            // I64 carrier (matches register_function_sig's <=8-byte branch).
            other if !is_32bit && other.is_vector() && other.size() <= 8 => IrType::I64,
            // Padded/9-16-byte vectors use I128 so the caller receives the
            // two-eightbyte bundle and the struct-return split stores the
            // halves correctly.
            other if !is_32bit && other.is_vector() && other.size() <= 16 => IrType::I128,
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
