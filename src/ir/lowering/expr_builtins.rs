//! Builtin function call lowering: __builtin_* dispatch and X86 SSE intrinsics.
//!
//! This module handles the top-level dispatch for all __builtin_* functions,
//! complex number intrinsics, and X86 SSE/CRC intrinsics. Extracted helpers
//! live in sibling modules:
//! - expr_builtins_intrin.rs: integer bit-manipulation (clz, ctz, bswap, etc.)
//! - expr_builtins_overflow.rs: overflow-checking arithmetic
//! - expr_builtins_fpclass.rs: FP classification (fpclassify, isnan, isinf, etc.)

use crate::frontend::parser::ast::Expr;
use crate::frontend::sema::builtins::{self, BuiltinKind, BuiltinIntrinsic};
use crate::ir::reexports::{
    CallInfo,
    Instruction,
    IntrinsicOp,
    IrBinOp,
    IrCmpOp,
    IrConst,
    IrUnaryOp,
    Operand,
    Terminator,
};
use crate::common::types::{AddressSpace, IrType, CType};
use super::lower::Lowerer;

impl Lowerer {
    /// Determine the return type for creal/crealf/creall/cimag/cimagf/cimagl based on function name.
    pub(super) fn creal_return_type(name: &str) -> IrType {
        match name {
            "crealf" | "__builtin_crealf" | "cimagf" | "__builtin_cimagf" => IrType::F32,
            "creall" | "__builtin_creall" | "cimagl" | "__builtin_cimagl" => IrType::F128,
            _ => IrType::F64, // creal, cimag, __builtin_creal, __builtin_cimag
        }
    }

    /// Try to lower a __builtin_* call. Returns Some(result) if handled.
    pub(super) fn try_lower_builtin_call(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
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
            "__builtin_alloca" | "__builtin_alloca_with_align" => {
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
                    self.emit(Instruction::DynAlloca { dest, size: size_operand, align });
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
                    self.emit(Instruction::VaStart { va_list_ptr: ap_ptr });
                }
                return Some(Operand::Const(IrConst::I64(0)));
            }
            "__builtin_va_end" => {
                if let Some(ap_expr) = args.first() {
                    let ap_ptr_op = self.lower_va_list_pointer(ap_expr);
                    let ap_ptr = self.operand_to_value(ap_ptr_op);
                    self.emit(Instruction::VaEnd { va_list_ptr: ap_ptr });
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
                let arg_types: Vec<IrType> = args.iter().map(|a| self.get_expr_type(a)).collect();
                let arg_vals: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let dest = self.fresh_value();
                let libc_sig = self.func_meta.sigs.get(libc_name.as_str());
                let variadic = libc_sig.is_some_and(|s| s.is_variadic);
                let n_fixed = if variadic {
                    libc_sig.map(|s| s.param_types.len()).unwrap_or(arg_vals.len())
                } else { arg_vals.len() };
                let return_type = Self::builtin_return_type(name)
                    .or_else(|| libc_sig.map(|s| s.return_type))
                    .unwrap_or(crate::common::types::target_int_ir_type());
                let struct_arg_sizes = vec![None; arg_vals.len()];
                self.emit(Instruction::Call {
                    func: libc_name.clone(),
                    info: CallInfo {
                        dest: Some(dest), args: arg_vals, arg_types,
                        return_type, is_variadic: variadic, num_fixed_args: n_fixed,
                        struct_arg_sizes, struct_arg_aligns: vec![], struct_arg_classes: Vec::new(),
                        struct_arg_riscv_float_classes: Vec::new(),
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false, is_fastcall: false,
                        ret_eightbyte_classes: Vec::new(),
                        ret_is_f128_sse: false,
                    },
                });
                Some(Operand::Value(dest))
            }
            BuiltinKind::Identity => {
                Some(args.first().map_or(Operand::Const(IrConst::I64(0)), |a| self.lower_expr(a)))
            }
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
    fn lower_builtin_intrinsic(&mut self, intrinsic: &BuiltinIntrinsic, name: &str, args: &[Expr]) -> Option<Operand> {
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
            BuiltinIntrinsic::Popcount => self.lower_unary_intrinsic(name, args, IrUnaryOp::Popcount),
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
            BuiltinIntrinsic::Fence => {
                Some(Operand::Const(IrConst::I64(0)))
            }
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
                // Evaluate args for side effects
                for arg in args {
                    self.lower_expr(arg);
                }
                let obj_type = if args.len() >= 2 {
                    match self.eval_const_expr(&args[1]) {
                        Some(IrConst::I64(v)) => v,
                        Some(IrConst::I32(v)) => v as i64,
                        _ => 0,
                    }
                } else {
                    0
                };
                // Types 0 and 1: maximum estimate -> -1 (unknown)
                // Types 2 and 3: minimum estimate -> 0 (unknown)
                let result = if obj_type == 2 || obj_type == 3 { 0i64 } else { -1i64 };
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
            BuiltinIntrinsic::CpuInit => {
                Some(Operand::Const(IrConst::I32(0)))
            }
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
                            "cmov", "mmx", "sse", "sse2", "sse3", "ssse3",
                            "sse4.1", "sse4.2", "popcnt", "avx", "avx2",
                            "fma", "bmi", "bmi2", "lzcnt", "movbe", "f16c",
                            "aes", "pclmul", "pclmulqdq", "rdrnd", "rdrand",
                            "rdseed", "sha", "adx", "fsgsbase", "xsave",
                            "xsaveopt", "xsavec", "avxvnni", "vaes",
                            "vpclmulqdq", "gfni", "cx16", "fxsr", "osxsave",
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
            BuiltinIntrinsic::X86Lfence | BuiltinIntrinsic::X86Mfence
            | BuiltinIntrinsic::X86Sfence | BuiltinIntrinsic::X86Pause | BuiltinIntrinsic::X86Vzeroupper
            | BuiltinIntrinsic::X86Rdtsc | BuiltinIntrinsic::X86Rdtscp
            | BuiltinIntrinsic::X86Clflush
            | BuiltinIntrinsic::X86Movnti | BuiltinIntrinsic::X86Movnti64
            | BuiltinIntrinsic::X86Movntdq | BuiltinIntrinsic::X86Movntpd
            | BuiltinIntrinsic::X86Loaddqu | BuiltinIntrinsic::X86Pcmpeqb128
            | BuiltinIntrinsic::X86Pcmpeqd128 | BuiltinIntrinsic::X86Psubusb128
            | BuiltinIntrinsic::X86Psubsb128
            | BuiltinIntrinsic::X86Por128 | BuiltinIntrinsic::X86Pand128
            | BuiltinIntrinsic::X86Pxor128 | BuiltinIntrinsic::X86Set1Epi8
            | BuiltinIntrinsic::X86Set1Epi32 | BuiltinIntrinsic::X86Aesenc128
            | BuiltinIntrinsic::X86Aesenclast128 | BuiltinIntrinsic::X86Aesdec128
            | BuiltinIntrinsic::X86Aesdeclast128 | BuiltinIntrinsic::X86Aesimc128
            | BuiltinIntrinsic::X86Aeskeygenassist128 | BuiltinIntrinsic::X86Pclmulqdq128
            | BuiltinIntrinsic::X86Pslldqi128 | BuiltinIntrinsic::X86Psrldqi128
            | BuiltinIntrinsic::X86Psllqi128 | BuiltinIntrinsic::X86Psrlqi128
            | BuiltinIntrinsic::X86Pshufd128 | BuiltinIntrinsic::X86Loadldi128
            | BuiltinIntrinsic::X86Paddw128 | BuiltinIntrinsic::X86Psubw128
            | BuiltinIntrinsic::X86Pmulhw128 | BuiltinIntrinsic::X86Pmaddwd128
            | BuiltinIntrinsic::X86Pcmpgtw128 | BuiltinIntrinsic::X86Pcmpgtb128
            | BuiltinIntrinsic::X86Psllwi128 | BuiltinIntrinsic::X86Psrlwi128
            | BuiltinIntrinsic::X86Psrawi128 | BuiltinIntrinsic::X86Psradi128
            | BuiltinIntrinsic::X86Pslldi128 | BuiltinIntrinsic::X86Psrldi128
            | BuiltinIntrinsic::X86Paddd128 | BuiltinIntrinsic::X86Psubd128
            | BuiltinIntrinsic::X86Packssdw128 | BuiltinIntrinsic::X86Packsswb128 | BuiltinIntrinsic::X86Packuswb128
            | BuiltinIntrinsic::X86Punpcklbw128 | BuiltinIntrinsic::X86Punpckhbw128
            | BuiltinIntrinsic::X86Punpcklwd128 | BuiltinIntrinsic::X86Punpckhwd128
            | BuiltinIntrinsic::X86Set1Epi16 | BuiltinIntrinsic::X86Pinsrw128
            | BuiltinIntrinsic::X86Cvtsi32Si128 | BuiltinIntrinsic::X86Pshuflw128
            | BuiltinIntrinsic::X86Pshufhw128
            | BuiltinIntrinsic::X86Storedqu | BuiltinIntrinsic::X86Storeldi128
            | BuiltinIntrinsic::X86Pmovmskb128 | BuiltinIntrinsic::X86Pextrw128
            | BuiltinIntrinsic::X86Cvtsi128Si32 | BuiltinIntrinsic::X86Cvtsi128Si64
            | BuiltinIntrinsic::X86Crc32_8 | BuiltinIntrinsic::X86Crc32_16
            | BuiltinIntrinsic::X86Crc32_32 | BuiltinIntrinsic::X86Crc32_64
            | BuiltinIntrinsic::X86Pinsrd128 | BuiltinIntrinsic::X86Pextrd128
            | BuiltinIntrinsic::X86Pinsrb128 | BuiltinIntrinsic::X86Pextrb128
            | BuiltinIntrinsic::X86Pinsrq128 | BuiltinIntrinsic::X86Pextrq128
            | BuiltinIntrinsic::X86Paddb128 | BuiltinIntrinsic::X86Psubb128
            | BuiltinIntrinsic::X86Psubusw128 | BuiltinIntrinsic::X86Psadbw128
            | BuiltinIntrinsic::X86Pmullw128 | BuiltinIntrinsic::X86Pmaddubsw128
            | BuiltinIntrinsic::X86Phaddw128 | BuiltinIntrinsic::X86Phaddd128
            | BuiltinIntrinsic::X86Pshufb128 | BuiltinIntrinsic::X86Pabsb128
            | BuiltinIntrinsic::X86Pabsw128 | BuiltinIntrinsic::X86Pabsd128
            | BuiltinIntrinsic::X86Palignr128 | BuiltinIntrinsic::X86Pmaxub128
            | BuiltinIntrinsic::X86Pminub128 | BuiltinIntrinsic::X86Pblendvb128
            | BuiltinIntrinsic::X86Pblendw128
            | BuiltinIntrinsic::X86Pmovzxbw128 | BuiltinIntrinsic::X86Pmovzxwd128
            | BuiltinIntrinsic::X86Psllw128 | BuiltinIntrinsic::X86Psrlw128
            | BuiltinIntrinsic::X86Loadu256 | BuiltinIntrinsic::X86Storeu256
            | BuiltinIntrinsic::X86Load256 | BuiltinIntrinsic::X86Store256
            | BuiltinIntrinsic::X86Paddb256 | BuiltinIntrinsic::X86Paddw256
            | BuiltinIntrinsic::X86Paddd256 | BuiltinIntrinsic::X86Psubb256
            | BuiltinIntrinsic::X86Psubw256 | BuiltinIntrinsic::X86Psubusw256
            | BuiltinIntrinsic::X86Psadbw256 | BuiltinIntrinsic::X86Pmaddubsw256
            | BuiltinIntrinsic::X86Pmaddwd256 | BuiltinIntrinsic::X86Pcmpeqb256
            | BuiltinIntrinsic::X86Pmovmskb256 | BuiltinIntrinsic::X86Pshufb256
            | BuiltinIntrinsic::X86Pabsb256 | BuiltinIntrinsic::X86Pabsw256
            | BuiltinIntrinsic::X86Pmaxub256 | BuiltinIntrinsic::X86Pminub256
            | BuiltinIntrinsic::X86Pxor256 | BuiltinIntrinsic::X86Por256
            | BuiltinIntrinsic::X86Pand256 | BuiltinIntrinsic::X86Psllidi256
            | BuiltinIntrinsic::X86Psrlidi256 | BuiltinIntrinsic::X86Psllwi256
            | BuiltinIntrinsic::X86Psrlwi256 | BuiltinIntrinsic::X86Broadcast128to256
            | BuiltinIntrinsic::X86Zext128to256 | BuiltinIntrinsic::X86Cast256to128
            | BuiltinIntrinsic::X86Insert128to256 | BuiltinIntrinsic::X86SetEpi8_256
            | BuiltinIntrinsic::X86SetEpi16_256
            | BuiltinIntrinsic::X86SetEpi32_256 | BuiltinIntrinsic::X86SetEpi64x256
            | BuiltinIntrinsic::X86Permutevar8x32
            | BuiltinIntrinsic::X86Dpbusd128 | BuiltinIntrinsic::X86Dpbusds128
            | BuiltinIntrinsic::X86Dpwusd128 | BuiltinIntrinsic::X86Dpwusds128
            | BuiltinIntrinsic::X86Dpbusd256 | BuiltinIntrinsic::X86Dpbusds256
            | BuiltinIntrinsic::X86Dpwusd256 | BuiltinIntrinsic::X86Dpwusds256
            | BuiltinIntrinsic::X86Dpbssd128 | BuiltinIntrinsic::X86Dpbssds128
            | BuiltinIntrinsic::X86Dpbsud128 | BuiltinIntrinsic::X86Dpbsuds128
            | BuiltinIntrinsic::X86Dpbuud128 | BuiltinIntrinsic::X86Dpbuuds128
            | BuiltinIntrinsic::X86Dpbssd256 | BuiltinIntrinsic::X86Dpbssds256
            | BuiltinIntrinsic::X86Dpbsud256 | BuiltinIntrinsic::X86Dpbsuds256
            | BuiltinIntrinsic::X86Dpbuud256 | BuiltinIntrinsic::X86Dpbuuds256
            | BuiltinIntrinsic::X86Dpwuud128 | BuiltinIntrinsic::X86Dpwuuds128
            | BuiltinIntrinsic::X86Dpwssd128 | BuiltinIntrinsic::X86Dpwssds128
            | BuiltinIntrinsic::X86Dpwuud256 | BuiltinIntrinsic::X86Dpwuuds256
            | BuiltinIntrinsic::X86Dpwssd256 | BuiltinIntrinsic::X86Dpwssds256
            | BuiltinIntrinsic::X86Gf2p8mulb128 | BuiltinIntrinsic::X86Gf2p8affineqb128
            | BuiltinIntrinsic::X86Gf2p8affineinvqb128
            | BuiltinIntrinsic::X86Aesenc256 | BuiltinIntrinsic::X86Aesenclast256
            | BuiltinIntrinsic::X86Aesdec256 | BuiltinIntrinsic::X86Aesdeclast256
            | BuiltinIntrinsic::X86Vpclmulqdq256
            | BuiltinIntrinsic::X86Paddusb128 | BuiltinIntrinsic::X86Paddsb128
            | BuiltinIntrinsic::X86Paddusw128 | BuiltinIntrinsic::X86Paddsw128
            | BuiltinIntrinsic::X86Psubsw128 | BuiltinIntrinsic::X86Pandn128
            | BuiltinIntrinsic::X86Pcmpeqw128 | BuiltinIntrinsic::X86Pcmpgtd128
            | BuiltinIntrinsic::X86Pavgb128 | BuiltinIntrinsic::X86Pavgw128
            | BuiltinIntrinsic::X86Pminsw128 | BuiltinIntrinsic::X86Pmaxsw128
            | BuiltinIntrinsic::X86Pmulhuw128 | BuiltinIntrinsic::X86Paddq128
            | BuiltinIntrinsic::X86Psubq128 | BuiltinIntrinsic::X86Punpckldq128
            | BuiltinIntrinsic::X86Punpckhdq128 | BuiltinIntrinsic::X86Punpcklqdq128
            | BuiltinIntrinsic::X86Punpckhqdq128 | BuiltinIntrinsic::X86Setzero128
            | BuiltinIntrinsic::X86Testz128
            | BuiltinIntrinsic::X86Pmulld256 | BuiltinIntrinsic::X86Psubd256
            | BuiltinIntrinsic::X86Paddq256 | BuiltinIntrinsic::X86Psubq256
            | BuiltinIntrinsic::X86Pandn256 | BuiltinIntrinsic::X86Pcmpeqd256
            | BuiltinIntrinsic::X86Pcmpeqq256 | BuiltinIntrinsic::X86Pcmpgtd256
            | BuiltinIntrinsic::X86Pcmpgtq256 | BuiltinIntrinsic::X86Extracti128
            | BuiltinIntrinsic::X86Setzero256
            | BuiltinIntrinsic::X86AddPs256 | BuiltinIntrinsic::X86SubPs256
            | BuiltinIntrinsic::X86MulPs256 | BuiltinIntrinsic::X86AddPd256
            | BuiltinIntrinsic::X86SubPd256 | BuiltinIntrinsic::X86MulPd256
            | BuiltinIntrinsic::X86LoaduPs256 | BuiltinIntrinsic::X86StoreuPs256
            | BuiltinIntrinsic::X86LoaduPd256 | BuiltinIntrinsic::X86StoreuPd256
            | BuiltinIntrinsic::X86Permute2x128 | BuiltinIntrinsic::X86Permute4x64
            | BuiltinIntrinsic::X86Pshufd256
            | BuiltinIntrinsic::X86Punpcklbw256 | BuiltinIntrinsic::X86Punpckhbw256
            | BuiltinIntrinsic::X86Punpcklwd256 | BuiltinIntrinsic::X86Punpckhwd256
            | BuiltinIntrinsic::X86Punpckldq256 | BuiltinIntrinsic::X86Punpckhdq256
            | BuiltinIntrinsic::X86Punpcklqdq256 | BuiltinIntrinsic::X86Punpckhqdq256
            | BuiltinIntrinsic::X86Pslldqi256 | BuiltinIntrinsic::X86Psrldqi256
            | BuiltinIntrinsic::X86Psllqi256 | BuiltinIntrinsic::X86Psrlqi256
            | BuiltinIntrinsic::X86Pmullw256 | BuiltinIntrinsic::X86Pmulhw256
            | BuiltinIntrinsic::X86Pminsd256 | BuiltinIntrinsic::X86Pmaxsd256
            | BuiltinIntrinsic::X86Pmovzxbw256 | BuiltinIntrinsic::X86Pmovzxbd256
            | BuiltinIntrinsic::X86Pmovzxwd256
            | BuiltinIntrinsic::X86Pmovsxbw256 | BuiltinIntrinsic::X86Pmovsxbd256
            | BuiltinIntrinsic::X86Pmovsxwd256
            | BuiltinIntrinsic::X86Psrawi256 | BuiltinIntrinsic::X86Psradi256
            | BuiltinIntrinsic::X86Packssdw256 | BuiltinIntrinsic::X86Packuswb256
            | BuiltinIntrinsic::X86Phaddw256 | BuiltinIntrinsic::X86Phaddd256
            | BuiltinIntrinsic::X86Pabsd256 | BuiltinIntrinsic::X86Pmuludq256
            | BuiltinIntrinsic::X86XorPs | BuiltinIntrinsic::X86AndPs
            | BuiltinIntrinsic::X86OrPs | BuiltinIntrinsic::X86AddPs
            | BuiltinIntrinsic::X86SubPs | BuiltinIntrinsic::X86MulPs
            | BuiltinIntrinsic::X86AddPd | BuiltinIntrinsic::X86SubPd
            | BuiltinIntrinsic::X86MulPd
            | BuiltinIntrinsic::X86MulEpu32 | BuiltinIntrinsic::X86MulEpi32
            | BuiltinIntrinsic::X86MulloEpi32
            | BuiltinIntrinsic::X86Pcmpgtb256
            | BuiltinIntrinsic::X86CastReinterpret => {
                self.lower_x86_intrinsic(intrinsic, args)
            }
            // Generic SIMD family: __lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}
            BuiltinIntrinsic::LcccSimd => {
                self.lower_lccc_simd(name, args)
            }
            // __builtin___*_chk: fortification builtins forward to unchecked libc equivalents.
            // Each __builtin___X_chk(args..., extra_check_args...) becomes X(args...).
            BuiltinIntrinsic::FortifyChk => {
                self.lower_fortify_chk(name, args)
            }
            // TODO: __builtin_va_arg_pack / __builtin_va_arg_pack_len are stubbed
            // to return 0. Proper implementation requires forwarding the caller's
            // variadic args during inlining. Since _FORTIFY_SOURCE is disabled,
            // this code path should not be reached in practice.
            BuiltinIntrinsic::VaArgPack => {
                for arg in args {
                    self.lower_expr(arg);
                }
                Some(Operand::Const(IrConst::I32(0)))
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
            BuiltinIntrinsic::FrameAddress | BuiltinIntrinsic::ReturnAddress => {
                // Evaluate the level argument
                let level = if !args.is_empty() {
                    self.lower_expr(&args[0])
                } else {
                    Operand::Const(IrConst::I64(0))
                };
                // Only level 0 is supported; for other levels return NULL
                let is_level_zero = matches!(&level, Operand::Const(IrConst::I64(0)) | Operand::Const(IrConst::I32(0)));
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
        let is_variadic = matches!(name,
            "__builtin___sprintf_chk" | "__builtin___snprintf_chk"
            | "__builtin___printf_chk" | "__builtin___fprintf_chk"
        );

        // Lower all arguments in order
        let arg_vals: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
        let arg_types: Vec<IrType> = args.iter().map(|a| self.get_expr_type(a)).collect();

        let dest = self.fresh_value();
        let return_type = Self::builtin_return_type(name)
            .unwrap_or(crate::common::types::target_int_ir_type());
        let n_fixed = arg_vals.len(); // All explicitly passed args are "fixed" from our perspective
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
                ret_eightbyte_classes: Vec::new(),
                ret_is_f128_sse: false,
            },
        });
        Some(Operand::Value(dest))
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
            let dest = self.emit_binop_val(IrBinOp::Or, Operand::Value(lhs_nan), Operand::Value(rhs_nan), IrType::I32);
            return Some(Operand::Value(dest));
        }

        // __builtin_islessgreater: returns 1 if a < b or a > b (NOT for NaN).
        // Emit (a < b) | (a > b).
        if name == "__builtin_islessgreater" {
            let lt = self.emit_cmp_val(IrCmpOp::Slt, lhs, rhs, cmp_ty);
            let gt = self.emit_cmp_val(IrCmpOp::Sgt, lhs, rhs, cmp_ty);
            let dest = self.emit_binop_val(IrBinOp::Or, Operand::Value(lt), Operand::Value(gt), IrType::I32);
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

    /// Lower __builtin_constant_p(expr): 1 if compile-time constant, 0 otherwise.
    /// Per GCC semantics, the argument is NOT evaluated (no side effects).
    /// In inline candidates, emits a deferred IsConstant instruction for post-optimization resolution.
    fn lower_constant_p(&mut self, args: &[Expr]) -> Option<Operand> {
        let Some(arg) = args.first() else {
            return Some(Operand::Const(IrConst::I32(0)));
        };
        // If already a compile-time constant at lowering time, resolve immediately
        if self.eval_const_expr(arg).is_some() {
            return Some(Operand::Const(IrConst::I32(1)));
        }
        // In non-inline-candidate functions, non-constant expressions always resolve to 0.
        // The argument is NOT evaluated per GCC semantics -- __builtin_constant_p
        // never has side effects regardless of its argument.
        if !self.func().is_inline_candidate {
            return Some(Operand::Const(IrConst::I32(0)));
        }
        // For inline candidates, emit an IsConstant instruction to be resolved after
        // optimization. We lower the expression to get a value reference, but it will
        // only be used to check constness, not for side effects. After inlining,
        // if the argument becomes constant, constant_fold resolves IsConstant to 1.
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
        self.emit(Instruction::Alloca { dest: alloca, ty: IrType::Ptr, size: complex_size, align: 0, volatile: false, semantic_volatile: false });
        self.emit(Instruction::Store { val: real_val, ptr: alloca, ty: comp_ty, seg_override: AddressSpace::Default });
        let imag_ptr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: imag_ptr, base: alloca,
            offset: Operand::Const(IrConst::I64(comp_size as i64)),
            ty: IrType::I8,
        });
        self.emit(Instruction::Store { val: imag_val, ptr: imag_ptr, ty: comp_ty, seg_override: AddressSpace::Default });
        Some(Operand::Value(alloca))
    }

    /// Lower an X86 SSE/AES/CRC intrinsic to IR.
    ///
    /// Classifies the intrinsic into one of four emission patterns:
    /// - Fence: no args, no dest (lfence, mfence, sfence, pause)
    /// - PtrStore: first arg is dest pointer, remaining args are data (movnti, storedqu, etc.)
    /// - Vec128: allocates 16-byte result slot, returns pointer (SSE/AES packed ops)
    /// - Scalar: returns a scalar value in a dest register (pmovmskb, crc32, pextrw, etc.)
    fn lower_x86_intrinsic(&mut self, intrinsic: &BuiltinIntrinsic, args: &[Expr]) -> Option<Operand> {
        let op = x86_intrinsic_op(intrinsic);
        match x86_intrinsic_kind(intrinsic) {
            X86IntrinsicKind::Fence => {
                self.emit(Instruction::Intrinsic { dest: None, op, dest_ptr: None, args: vec![] });
                Some(Operand::Const(IrConst::I64(0)))
            }
            X86IntrinsicKind::VoidArgs => {
                let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(Instruction::Intrinsic { dest: None, op, dest_ptr: None, args: arg_ops });
                Some(Operand::Const(IrConst::I64(0)))
            }
            X86IntrinsicKind::PtrStore => {
                let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let ptr_val = self.operand_to_value(arg_ops[0]);
                self.emit(Instruction::Intrinsic { dest: None, op, dest_ptr: Some(ptr_val), args: vec![arg_ops[1]] });
                Some(Operand::Const(IrConst::I64(0)))
            }
            X86IntrinsicKind::Vec128 | X86IntrinsicKind::Vec256 => {
                let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let result_size = match x86_intrinsic_kind(intrinsic) { X86IntrinsicKind::Vec128 => 16, X86IntrinsicKind::Vec256 => 32, _ => unreachable!() };
                let result_alloca = self.fresh_value();
                self.emit(Instruction::Alloca { dest: result_alloca, ty: IrType::Ptr, size: result_size, align: 0, volatile: false, semantic_volatile: false });
                let dest_val = self.fresh_value();
                self.emit(Instruction::Intrinsic { dest: Some(dest_val), op, dest_ptr: Some(result_alloca), args: arg_ops });
                Some(Operand::Value(result_alloca))
            }
            X86IntrinsicKind::Scalar => {
                let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let dest_val = self.fresh_value();
                self.emit(Instruction::Intrinsic { dest: Some(dest_val), op, dest_ptr: None, args: arg_ops });
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
fn x86_intrinsic_kind(intrinsic: &BuiltinIntrinsic) -> X86IntrinsicKind {
    match intrinsic {
        BuiltinIntrinsic::X86Lfence | BuiltinIntrinsic::X86Mfence
        | BuiltinIntrinsic::X86Sfence | BuiltinIntrinsic::X86Pause | BuiltinIntrinsic::X86Vzeroupper => X86IntrinsicKind::Fence,

        BuiltinIntrinsic::X86Clflush => X86IntrinsicKind::VoidArgs,

        BuiltinIntrinsic::X86Movnti | BuiltinIntrinsic::X86Movnti64
        | BuiltinIntrinsic::X86Movntdq | BuiltinIntrinsic::X86Movntpd
        | BuiltinIntrinsic::X86Storedqu | BuiltinIntrinsic::X86Storeldi128
        | BuiltinIntrinsic::X86Storeu256 | BuiltinIntrinsic::X86Store256
        | BuiltinIntrinsic::X86StoreuPs256 | BuiltinIntrinsic::X86StoreuPd256 => X86IntrinsicKind::PtrStore,

        BuiltinIntrinsic::X86Pmovmskb128 | BuiltinIntrinsic::X86Pextrw128
        | BuiltinIntrinsic::X86Cvtsi128Si32 | BuiltinIntrinsic::X86Cvtsi128Si64
        | BuiltinIntrinsic::X86Crc32_8 | BuiltinIntrinsic::X86Crc32_16
        | BuiltinIntrinsic::X86Crc32_32 | BuiltinIntrinsic::X86Crc32_64
        | BuiltinIntrinsic::X86Pextrd128 | BuiltinIntrinsic::X86Pextrb128
        | BuiltinIntrinsic::X86Pextrq128 | BuiltinIntrinsic::X86Pmovmskb256
        | BuiltinIntrinsic::X86Rdtsc | BuiltinIntrinsic::X86Rdtscp
        | BuiltinIntrinsic::X86Testz128 => X86IntrinsicKind::Scalar,

        BuiltinIntrinsic::X86Loadu256 | BuiltinIntrinsic::X86Load256
        | BuiltinIntrinsic::X86Paddb256 | BuiltinIntrinsic::X86Paddw256 | BuiltinIntrinsic::X86Paddd256
        | BuiltinIntrinsic::X86Psubb256 | BuiltinIntrinsic::X86Psubw256 | BuiltinIntrinsic::X86Psubusw256
        | BuiltinIntrinsic::X86Psadbw256 | BuiltinIntrinsic::X86Pmaddubsw256 | BuiltinIntrinsic::X86Pmaddwd256
        | BuiltinIntrinsic::X86Pcmpeqb256 | BuiltinIntrinsic::X86Pcmpgtb256
        | BuiltinIntrinsic::X86Pshufb256 | BuiltinIntrinsic::X86Pabsb256
        | BuiltinIntrinsic::X86Pabsw256 | BuiltinIntrinsic::X86Pmaxub256 | BuiltinIntrinsic::X86Pminub256
        | BuiltinIntrinsic::X86Pxor256 | BuiltinIntrinsic::X86Por256 | BuiltinIntrinsic::X86Pand256
        | BuiltinIntrinsic::X86Psllidi256 | BuiltinIntrinsic::X86Psrlidi256 | BuiltinIntrinsic::X86Psllwi256
        | BuiltinIntrinsic::X86Psrlwi256 | BuiltinIntrinsic::X86Broadcast128to256 | BuiltinIntrinsic::X86Zext128to256
        | BuiltinIntrinsic::X86Insert128to256 | BuiltinIntrinsic::X86SetEpi8_256
        | BuiltinIntrinsic::X86SetEpi16_256 | BuiltinIntrinsic::X86SetEpi32_256
        | BuiltinIntrinsic::X86SetEpi64x256 | BuiltinIntrinsic::X86Permutevar8x32
        | BuiltinIntrinsic::X86Dpbusd256 | BuiltinIntrinsic::X86Dpbusds256
        | BuiltinIntrinsic::X86Dpwusd256 | BuiltinIntrinsic::X86Dpwusds256 | BuiltinIntrinsic::X86Dpbssd256
        | BuiltinIntrinsic::X86Dpbssds256 | BuiltinIntrinsic::X86Dpbsud256 | BuiltinIntrinsic::X86Dpbsuds256
        | BuiltinIntrinsic::X86Dpbuud256 | BuiltinIntrinsic::X86Dpbuuds256 | BuiltinIntrinsic::X86Dpwuud256
        | BuiltinIntrinsic::X86Dpwuuds256 | BuiltinIntrinsic::X86Dpwssd256 | BuiltinIntrinsic::X86Dpwssds256
        | BuiltinIntrinsic::X86Aesenc256 | BuiltinIntrinsic::X86Aesenclast256 | BuiltinIntrinsic::X86Aesdec256
        | BuiltinIntrinsic::X86Aesdeclast256 | BuiltinIntrinsic::X86Vpclmulqdq256
        | BuiltinIntrinsic::X86Pmulld256 | BuiltinIntrinsic::X86Psubd256
        | BuiltinIntrinsic::X86Paddq256 | BuiltinIntrinsic::X86Psubq256
        | BuiltinIntrinsic::X86Pandn256 | BuiltinIntrinsic::X86Pcmpeqd256
        | BuiltinIntrinsic::X86Pcmpeqq256 | BuiltinIntrinsic::X86Pcmpgtd256
        | BuiltinIntrinsic::X86Pcmpgtq256 | BuiltinIntrinsic::X86Setzero256
        | BuiltinIntrinsic::X86AddPs256 | BuiltinIntrinsic::X86SubPs256
        | BuiltinIntrinsic::X86MulPs256 | BuiltinIntrinsic::X86AddPd256
        | BuiltinIntrinsic::X86SubPd256 | BuiltinIntrinsic::X86MulPd256
        | BuiltinIntrinsic::X86LoaduPs256 | BuiltinIntrinsic::X86LoaduPd256
        | BuiltinIntrinsic::X86Permute2x128 | BuiltinIntrinsic::X86Permute4x64
        | BuiltinIntrinsic::X86Pshufd256
        | BuiltinIntrinsic::X86Punpcklbw256 | BuiltinIntrinsic::X86Punpckhbw256
        | BuiltinIntrinsic::X86Punpcklwd256 | BuiltinIntrinsic::X86Punpckhwd256
        | BuiltinIntrinsic::X86Punpckldq256 | BuiltinIntrinsic::X86Punpckhdq256
        | BuiltinIntrinsic::X86Punpcklqdq256 | BuiltinIntrinsic::X86Punpckhqdq256
        | BuiltinIntrinsic::X86Pslldqi256 | BuiltinIntrinsic::X86Psrldqi256
        | BuiltinIntrinsic::X86Psllqi256 | BuiltinIntrinsic::X86Psrlqi256
        | BuiltinIntrinsic::X86Pmullw256 | BuiltinIntrinsic::X86Pmulhw256
        | BuiltinIntrinsic::X86Pminsd256 | BuiltinIntrinsic::X86Pmaxsd256
        | BuiltinIntrinsic::X86Pmovzxbw256 | BuiltinIntrinsic::X86Pmovzxbd256
        | BuiltinIntrinsic::X86Pmovzxwd256
        | BuiltinIntrinsic::X86Pmovsxbw256 | BuiltinIntrinsic::X86Pmovsxbd256
        | BuiltinIntrinsic::X86Pmovsxwd256
        | BuiltinIntrinsic::X86Psrawi256 | BuiltinIntrinsic::X86Psradi256
        | BuiltinIntrinsic::X86Packssdw256 | BuiltinIntrinsic::X86Packuswb256
        | BuiltinIntrinsic::X86Phaddw256 | BuiltinIntrinsic::X86Phaddd256
        | BuiltinIntrinsic::X86Pabsd256 | BuiltinIntrinsic::X86Pmuludq256 => X86IntrinsicKind::Vec256,
        BuiltinIntrinsic::X86CastReinterpret => X86IntrinsicKind::CastReinterpret,
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
        _ => unreachable!("x86_intrinsic_op called with non-X86 intrinsic: {:?}", intrinsic),
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
        CType::Void => 0,            // no_type_class
        CType::Bool
        | CType::Char | CType::UChar
        | CType::Short | CType::UShort
        | CType::Int | CType::UInt
        | CType::Long | CType::ULong
        | CType::LongLong | CType::ULongLong
        | CType::Int128 | CType::UInt128
        | CType::Enum(_) => 1,       // integer_type_class
        CType::Float | CType::Double | CType::LongDouble | CType::Float128 => 8, // real_type_class
        CType::ComplexFloat | CType::ComplexDouble
        | CType::ComplexLongDouble => 9, // complex_type_class
        CType::Pointer(_, _) => 5,      // pointer_type_class
        CType::Array(_, _) => 5,     // GCC decays arrays to pointers
        CType::Function(_) => 5,     // function decays to pointer
        CType::Struct(_) => 12,      // record_type_class
        CType::Union(_) => 13,       // union_type_class
        CType::Vector(_, _) => 14,   // array_type_class (GCC classifies vectors here)
    }
}
