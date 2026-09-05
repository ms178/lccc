//! Maps GCC __builtin_* function names to their libc/standard equivalents.
//!
//! Many C programs use GCC builtins (e.g., __builtin_abort, __builtin_memcpy).
//! We map these to their standard library equivalents so the linker can resolve them.

use crate::common::fx_hash::FxHashMap;
use std::sync::LazyLock;

/// Static mapping of __builtin_* names to their libc equivalents.
static BUILTIN_MAP: LazyLock<FxHashMap<&'static str, BuiltinInfo>> = LazyLock::new(|| {
    let mut m = FxHashMap::default();

    // Abort/exit
    // Note: __builtin_trap and __builtin_unreachable are handled directly in
    // expr_builtins.rs as Terminator::Unreachable (emitting ud2/brk/ebreak),
    // not as calls to abort(). This is critical for kernel code where abort()
    // doesn't exist.
    m.insert("__builtin_abort", BuiltinInfo::simple("abort"));
    m.insert("__builtin_exit", BuiltinInfo::simple("exit"));

    // Memory functions
    m.insert("__builtin_memcpy", BuiltinInfo::simple("memcpy"));
    m.insert("__builtin_mempcpy", BuiltinInfo::simple("mempcpy"));
    m.insert("__builtin_memmove", BuiltinInfo::simple("memmove"));
    m.insert("__builtin_memset", BuiltinInfo::simple("memset"));
    m.insert("__builtin_memcmp", BuiltinInfo::simple("memcmp"));
    m.insert("__builtin_strlen", BuiltinInfo::simple("strlen"));
    m.insert("__builtin_strcpy", BuiltinInfo::simple("strcpy"));
    m.insert("__builtin_stpcpy", BuiltinInfo::simple("stpcpy"));
    m.insert("__builtin_memset", BuiltinInfo::simple("memset"));
    m.insert("__builtin_memcmp", BuiltinInfo::simple("memcmp"));
    m.insert("__builtin_strlen", BuiltinInfo::simple("strlen"));
    m.insert("__builtin_strcpy", BuiltinInfo::simple("strcpy"));
    m.insert("__builtin_strncpy", BuiltinInfo::simple("strncpy"));
    m.insert("__builtin_strcmp", BuiltinInfo::simple("strcmp"));
    m.insert("__builtin_strncmp", BuiltinInfo::simple("strncmp"));
    m.insert("__builtin_strcat", BuiltinInfo::simple("strcat"));
    m.insert("__builtin_strchr", BuiltinInfo::simple("strchr"));
    m.insert("__builtin_strrchr", BuiltinInfo::simple("strrchr"));
    m.insert("__builtin_strstr", BuiltinInfo::simple("strstr"));
    // Used by the Linux kernel's include/linux/fortify-string.h.
    m.insert("__builtin_strncat", BuiltinInfo::simple("strncat"));
    m.insert("__builtin_memchr", BuiltinInfo::simple("memchr"));
    m.insert("__builtin_strspn", BuiltinInfo::simple("strspn"));
    m.insert("__builtin_strcspn", BuiltinInfo::simple("strcspn"));
    m.insert("__builtin_strpbrk", BuiltinInfo::simple("strpbrk"));
    m.insert("__builtin_stpncpy", BuiltinInfo::simple("stpncpy"));

    // Math functions
    m.insert("__builtin_abs", BuiltinInfo::simple("abs"));
    m.insert("__builtin_labs", BuiltinInfo::simple("labs"));
    m.insert("__builtin_llabs", BuiltinInfo::simple("llabs"));
    m.insert("__builtin_fabs", BuiltinInfo::simple("fabs"));
    m.insert("__builtin_fabsf", BuiltinInfo::simple("fabsf"));
    m.insert("__builtin_fabsl", BuiltinInfo::simple("fabsl"));
    m.insert("__builtin_sqrt", BuiltinInfo::simple("sqrt"));
    // C99 fused multiply-add: MUST reach the FmaScalar intrinsic for
    // single-rounding semantics (glibc math-use-builtins-fma.h).
    m.insert("__builtin_fma", BuiltinInfo::simple("fma"));
    m.insert("__builtin_fmaf", BuiltinInfo::simple("fmaf"));
    m.insert("__builtin_sqrtl", BuiltinInfo::simple("sqrtl"));
    m.insert("__builtin_sqrtf", BuiltinInfo::simple("sqrtf"));
    m.insert("__builtin_sin", BuiltinInfo::simple("sin"));
    m.insert("__builtin_sinf", BuiltinInfo::simple("sinf"));
    m.insert("__builtin_cos", BuiltinInfo::simple("cos"));
    m.insert("__builtin_cosf", BuiltinInfo::simple("cosf"));
    m.insert("__builtin_log", BuiltinInfo::simple("log"));
    m.insert("__builtin_logf", BuiltinInfo::simple("logf"));
    m.insert("__builtin_log2", BuiltinInfo::simple("log2"));
    m.insert("__builtin_exp", BuiltinInfo::simple("exp"));
    m.insert("__builtin_expf", BuiltinInfo::simple("expf"));
    m.insert("__builtin_pow", BuiltinInfo::simple("pow"));
    m.insert("__builtin_powf", BuiltinInfo::simple("powf"));
    m.insert("__builtin_floor", BuiltinInfo::simple("floor"));
    m.insert("__builtin_floorf", BuiltinInfo::simple("floorf"));
    m.insert("__builtin_ceil", BuiltinInfo::simple("ceil"));
    m.insert("__builtin_ceilf", BuiltinInfo::simple("ceilf"));
    m.insert("__builtin_trunc", BuiltinInfo::simple("trunc"));
    m.insert("__builtin_truncf", BuiltinInfo::simple("truncf"));
    m.insert("__builtin_rint", BuiltinInfo::simple("rint"));
    m.insert("__builtin_rintf", BuiltinInfo::simple("rintf"));
    m.insert("__builtin_nearbyint", BuiltinInfo::simple("nearbyint"));
    m.insert("__builtin_nearbyintf", BuiltinInfo::simple("nearbyintf"));
    m.insert("__builtin_roundeven", BuiltinInfo::simple("roundeven"));
    m.insert("__builtin_roundevenf", BuiltinInfo::simple("roundevenf"));
    m.insert("__builtin_round", BuiltinInfo::simple("round"));
    m.insert("__builtin_roundf", BuiltinInfo::simple("roundf"));
    m.insert("__builtin_fmin", BuiltinInfo::simple("fmin"));
    m.insert("__builtin_fmax", BuiltinInfo::simple("fmax"));
    m.insert("__builtin_copysign", BuiltinInfo::simple("copysign"));
    m.insert("__builtin_copysignf", BuiltinInfo::simple("copysignf"));
    m.insert("__builtin_copysignl", BuiltinInfo::simple("copysignl"));
    m.insert("__builtin_nextafter", BuiltinInfo::simple("nextafter"));
    m.insert("__builtin_nextafterf", BuiltinInfo::simple("nextafterf"));
    m.insert("__builtin_nextafterl", BuiltinInfo::simple("nextafterl"));
    // Long-double (x87 80-bit) math builtins: GCC provides __builtin_<fn>l for
    // the whole C99 libm set. Each must be aliased to its libc name, otherwise
    // the literal symbol "__builtin_truncl" stays unresolved at link time.
    m.insert("__builtin_truncl", BuiltinInfo::simple("truncl"));
    m.insert("__builtin_floorl", BuiltinInfo::simple("floorl"));
    m.insert("__builtin_ceill", BuiltinInfo::simple("ceill"));
    m.insert("__builtin_rintl", BuiltinInfo::simple("rintl"));
    m.insert("__builtin_nearbyintl", BuiltinInfo::simple("nearbyintl"));
    m.insert("__builtin_roundl", BuiltinInfo::simple("roundl"));
    m.insert("__builtin_lroundl", BuiltinInfo::simple("lroundl"));
    m.insert("__builtin_llroundl", BuiltinInfo::simple("llroundl"));
    m.insert("__builtin_lrintl", BuiltinInfo::simple("lrintl"));
    m.insert("__builtin_llrintl", BuiltinInfo::simple("llrintl"));
    m.insert("__builtin_fmodl", BuiltinInfo::simple("fmodl"));
    m.insert("__builtin_remainderl", BuiltinInfo::simple("remainderl"));
    m.insert("__builtin_powl", BuiltinInfo::simple("powl"));
    m.insert("__builtin_sinl", BuiltinInfo::simple("sinl"));
    m.insert("__builtin_cosl", BuiltinInfo::simple("cosl"));
    m.insert("__builtin_tanl", BuiltinInfo::simple("tanl"));
    m.insert("__builtin_asinl", BuiltinInfo::simple("asinl"));
    m.insert("__builtin_acosl", BuiltinInfo::simple("acosl"));
    m.insert("__builtin_atanl", BuiltinInfo::simple("atanl"));
    m.insert("__builtin_atan2l", BuiltinInfo::simple("atan2l"));
    m.insert("__builtin_sinhl", BuiltinInfo::simple("sinhl"));
    m.insert("__builtin_coshl", BuiltinInfo::simple("coshl"));
    m.insert("__builtin_tanhl", BuiltinInfo::simple("tanhl"));
    m.insert("__builtin_asinhl", BuiltinInfo::simple("asinhl"));
    m.insert("__builtin_acoshl", BuiltinInfo::simple("acoshl"));
    m.insert("__builtin_atanhl", BuiltinInfo::simple("atanhl"));
    m.insert("__builtin_exp2l", BuiltinInfo::simple("exp2l"));
    m.insert("__builtin_expm1l", BuiltinInfo::simple("expm1l"));
    m.insert("__builtin_log1pl", BuiltinInfo::simple("log1pl"));
    m.insert("__builtin_log10l", BuiltinInfo::simple("log10l"));
    m.insert("__builtin_log2l", BuiltinInfo::simple("log2l"));
    m.insert("__builtin_fmaxl", BuiltinInfo::simple("fmaxl"));
    m.insert("__builtin_fminl", BuiltinInfo::simple("fminl"));
    m.insert("__builtin_fdiml", BuiltinInfo::simple("fdiml"));
    m.insert("__builtin_fmal", BuiltinInfo::simple("fmal"));
    m.insert("__builtin_hypotl", BuiltinInfo::simple("hypotl"));
    m.insert("__builtin_cbrtl", BuiltinInfo::simple("cbrtl"));
    m.insert("__builtin_frexpl", BuiltinInfo::simple("frexpl"));
    m.insert("__builtin_ldexpl", BuiltinInfo::simple("ldexpl"));
    m.insert("__builtin_scalbnl", BuiltinInfo::simple("scalbnl"));
    m.insert("__builtin_scalblnl", BuiltinInfo::simple("scalblnl"));
    m.insert("__builtin_modfl", BuiltinInfo::simple("modfl"));
    m.insert("__builtin_erfl", BuiltinInfo::simple("erfl"));
    m.insert("__builtin_erfcl", BuiltinInfo::simple("erfcl"));
    m.insert("__builtin_tgammal", BuiltinInfo::simple("tgammal"));
    m.insert("__builtin_lgammal", BuiltinInfo::simple("lgammal"));
    m.insert("__builtin_j0l", BuiltinInfo::simple("j0l"));
    m.insert("__builtin_j1l", BuiltinInfo::simple("j1l"));
    m.insert("__builtin_jnl", BuiltinInfo::simple("jnl"));
    m.insert("__builtin_y0l", BuiltinInfo::simple("y0l"));
    m.insert("__builtin_y1l", BuiltinInfo::simple("y1l"));
    m.insert("__builtin_ynl", BuiltinInfo::simple("ynl"));
    // TODO: __builtin_nan(s) ignores the string payload argument (NaN payload).
    // For common usage with "" this is correct; full payload support needs custom lowering.
    m.insert("__builtin_nan", BuiltinInfo::constant_f64(f64::NAN));
    m.insert("__builtin_nanf", BuiltinInfo::constant_f64(f64::NAN));
    m.insert("__builtin_inf", BuiltinInfo::constant_f64(f64::INFINITY));
    m.insert("__builtin_inff", BuiltinInfo::constant_f64(f64::INFINITY));
    m.insert("__builtin_infl", BuiltinInfo::constant_f64(f64::INFINITY));
    m.insert(
        "__builtin_huge_val",
        BuiltinInfo::constant_f64(f64::INFINITY),
    );
    m.insert(
        "__builtin_huge_valf",
        BuiltinInfo::constant_f64(f64::INFINITY),
    );
    m.insert(
        "__builtin_huge_vall",
        BuiltinInfo::constant_f64(f64::INFINITY),
    );
    m.insert("__builtin_nanl", BuiltinInfo::constant_f64(f64::NAN));
    // _Float128 variants (IEEE binary128): +Inf = 0x7FFF0000...0,
    // qNaN = 0x7FFF8000...0 (big-endian byte order in the 16-byte payload).
    m.insert(
        "__builtin_huge_valf128",
        BuiltinInfo::constant_f128([0x7F, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    );
    m.insert(
        "__builtin_inff128",
        BuiltinInfo::constant_f128([0x7F, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    );
    m.insert(
        "__builtin_nanf128",
        BuiltinInfo::constant_f128([0x7F, 0xFF, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    );
    m.insert(
        "__builtin_copysignf128",
        BuiltinInfo::simple("__copysigntf3"),
    );
    m.insert("__builtin_fabsf128", BuiltinInfo::simple("__fabstf2"));

    // I/O
    m.insert("__builtin_printf", BuiltinInfo::simple("printf"));
    m.insert("__builtin_fprintf", BuiltinInfo::simple("fprintf"));
    m.insert("__builtin_sprintf", BuiltinInfo::simple("sprintf"));
    m.insert("__builtin_snprintf", BuiltinInfo::simple("snprintf"));
    m.insert("__builtin_dprintf", BuiltinInfo::simple("dprintf"));
    m.insert("__builtin_puts", BuiltinInfo::simple("puts"));
    m.insert("__builtin_putchar", BuiltinInfo::simple("putchar"));

    // Allocation
    m.insert("__builtin_malloc", BuiltinInfo::simple("malloc"));
    m.insert("__builtin_calloc", BuiltinInfo::simple("calloc"));
    m.insert("__builtin_realloc", BuiltinInfo::simple("realloc"));
    m.insert("__builtin_free", BuiltinInfo::simple("free"));

    // Stack allocation - handled specially in try_lower_builtin_call as DynAlloca
    m.insert(
        "__builtin_alloca",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Alloca),
    );
    m.insert(
        "__builtin_alloca_with_align",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Alloca),
    );
    // Bare `alloca` is a builtin in GNU modes, exactly like GCC: code that
    // calls it without including <alloca.h> (30+ GCC torture execute tests)
    // must get the DynAlloca lowering, not an undefined external call.
    // A user-defined function body named `alloca` still overrides this via
    // the is_defined check in try_lower_builtin_call. glibc is unaffected
    // (<alloca.h> #defines alloca to __builtin_alloca).
    // Credit: Agent B torture triage.
    m.insert("alloca", BuiltinInfo::intrinsic(BuiltinIntrinsic::Alloca));

    // Return address / frame address / thread pointer
    m.insert(
        "__builtin_return_address",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ReturnAddress),
    );
    m.insert(
        "__builtin_frame_address",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FrameAddress),
    );
    m.insert(
        "__builtin_setjmp",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::BuiltinSetjmp),
    );
    m.insert(
        "__builtin_longjmp",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::BuiltinLongjmp),
    );
    m.insert(
        "__builtin_apply_args",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ApplyArgs),
    );
    m.insert(
        "__builtin_apply",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Apply),
    );
    m.insert(
        "__builtin_return",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::BuiltinReturn),
    );
    m.insert("__builtin_extract_return_addr", BuiltinInfo::identity());
    m.insert(
        "__builtin_thread_pointer",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ThreadPointer),
    );
    m.insert(
        "__builtin_ia32_rdtsc",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Rdtsc),
    );
    m.insert(
        "__builtin_ia32_rdtscp",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Rdtscp),
    );

    // Compiler hints (these become no-ops or identity)
    m.insert("__builtin_expect", BuiltinInfo::identity()); // returns first arg
    m.insert("__builtin_expect_with_probability", BuiltinInfo::identity());
    m.insert("__builtin_assume_aligned", BuiltinInfo::identity());

    // Type queries (compile-time constants)
    m.insert(
        "__builtin_constant_p",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ConstantP),
    );
    m.insert(
        "__builtin_object_size",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ObjectSize),
    );
    m.insert(
        "__builtin_dynamic_object_size",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ObjectSize),
    );
    m.insert(
        "__builtin_classify_type",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ClassifyType),
    );

    // Fortification builtins: __builtin___*_chk variants used by glibc's _FORTIFY_SOURCE.
    // These forward all arguments to the glibc __*_chk runtime function.
    m.insert(
        "__builtin___memcpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___memmove_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___memset_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___strcpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___strncpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___strcat_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___strncat_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___sprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___snprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___vsprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___vsnprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___printf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___fprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___vprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___vfprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___dprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___mempcpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___stpcpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___stpncpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin_va_arg_pack",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::VaArgPack),
    );
    m.insert(
        "__builtin_va_arg_pack_len",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::VaArgPack),
    );
    // Note: __builtin_types_compatible_p is handled as a special AST node (BuiltinTypesCompatibleP),
    // parsed directly in the parser and evaluated at compile-time in the lowerer.

    // Floating-point comparison builtins
    m.insert(
        "__builtin_isgreater",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );
    m.insert(
        "__builtin_isgreaterequal",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );
    m.insert(
        "__builtin_isless",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );
    m.insert(
        "__builtin_islessequal",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );
    m.insert(
        "__builtin_islessgreater",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );
    m.insert(
        "__builtin_isunordered",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );

    // Floating-point classification builtins
    m.insert(
        "__builtin_fpclassify",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpClassify),
    );
    for name in ["__builtin_isnan", "__builtin_isnanf", "__builtin_isnanl"] {
        m.insert(name, BuiltinInfo::intrinsic(BuiltinIntrinsic::IsNan));
    }
    for name in ["__builtin_isinf", "__builtin_isinff", "__builtin_isinfl"] {
        m.insert(name, BuiltinInfo::intrinsic(BuiltinIntrinsic::IsInf));
    }
    for name in [
        "__builtin_isfinite",
        "__builtin_isfinitef",
        "__builtin_isfinitel",
    ] {
        m.insert(name, BuiltinInfo::intrinsic(BuiltinIntrinsic::IsFinite));
    }
    for name in [
        "__builtin_isnormal",
        "__builtin_isnormalf",
        "__builtin_isnormall",
    ] {
        m.insert(name, BuiltinInfo::intrinsic(BuiltinIntrinsic::IsNormal));
    }
    m.insert(
        "__builtin_signbit",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SignBit),
    );
    m.insert(
        "__builtin_signbitf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SignBit),
    );
    m.insert(
        "__builtin_signbitl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SignBit),
    );
    m.insert(
        "__builtin_isinf_sign",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::IsInfSign),
    );

    // Bit manipulation
    m.insert(
        "__builtin_clz",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clz),
    );
    m.insert(
        "__builtin_clzl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clz),
    );
    m.insert(
        "__builtin_clzll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clz),
    );
    m.insert(
        "__builtin_ctz",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ctz),
    );
    m.insert(
        "__builtin_ctzl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ctz),
    );
    m.insert(
        "__builtin_ctzll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ctz),
    );
    m.insert(
        "__builtin_popcount",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Popcount),
    );
    m.insert(
        "__builtin_popcountl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Popcount),
    );
    m.insert(
        "__builtin_popcountll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Popcount),
    );
    m.insert(
        "__builtin_bswap16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Bswap),
    );
    m.insert(
        "__builtin_bswap32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Bswap),
    );
    m.insert(
        "__builtin_bswap64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Bswap),
    );
    m.insert(
        "__builtin_ffs",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ffs),
    );
    m.insert(
        "__builtin_ffsl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ffs),
    );
    m.insert(
        "__builtin_ffsll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ffs),
    );
    m.insert(
        "__builtin_parity",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Parity),
    );
    m.insert(
        "__builtin_parityl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Parity),
    );
    m.insert(
        "__builtin_parityll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Parity),
    );
    m.insert(
        "__builtin_clrsb",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clrsb),
    );
    m.insert(
        "__builtin_clrsbl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clrsb),
    );
    m.insert(
        "__builtin_clrsbll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clrsb),
    );

    // Overflow-checking arithmetic builtins
    // Generic (type-deduced from arguments):
    m.insert(
        "__builtin_add_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_sub_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_mul_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    // Signed int variants:
    m.insert(
        "__builtin_sadd_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_saddl_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_saddll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_ssub_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_ssubl_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_ssubll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_smul_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    m.insert(
        "__builtin_smull_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    m.insert(
        "__builtin_smulll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    // Unsigned int variants:
    m.insert(
        "__builtin_uadd_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_uaddl_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_uaddll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_usub_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_usubl_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_usubll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_umul_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    m.insert(
        "__builtin_umull_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    m.insert(
        "__builtin_umulll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );

    // Overflow-checking predicate builtins (GCC 7+):
    // __builtin_{add,sub,mul}_overflow_p(a, b, (T)0) -> 1 if op overflows type T
    // These are like the non-_p variants but don't store a result.
    m.insert(
        "__builtin_add_overflow_p",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflowP),
    );
    m.insert(
        "__builtin_sub_overflow_p",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflowP),
    );
    m.insert(
        "__builtin_mul_overflow_p",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflowP),
    );

    // Atomics (map to libc atomic helpers for now)
    m.insert(
        "__sync_synchronize",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Fence),
    );

    // Complex number functions (C99 <complex.h>)
    m.insert(
        "creal",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "crealf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "creall",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "cimag",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "cimagf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "cimagl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "__builtin_creal",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "__builtin_crealf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "__builtin_creall",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "__builtin_cimag",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "__builtin_cimagf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "__builtin_cimagl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "conj",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );
    m.insert(
        "conjf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );
    m.insert(
        "conjl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );
    m.insert(
        "__builtin_conj",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );
    m.insert(
        "__builtin_conjf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );
    m.insert(
        "__builtin_conjl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );

    // Complex construction
    m.insert(
        "__builtin_complex",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConstruct),
    );

    // Variadic argument builtins - these are handled specially in IR lowering
    // (expr.rs try_lower_builtin_call), but must be registered here so sema
    // does not emit "implicit declaration" warnings. Those warnings break
    // configure scripts that check stderr for errors (e.g., zlib).
    m.insert(
        "__builtin_va_start",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::VaStart),
    );
    m.insert(
        "__builtin_va_end",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::VaEnd),
    );
    m.insert(
        "__builtin_va_copy",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::VaCopy),
    );

    // Prefetch (no-op, handled separately in lowering)
    m.insert(
        "__builtin_prefetch",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Nop),
    );

    // CPU feature detection builtins
    // __builtin_cpu_init() initializes CPU feature detection; on glibc systems
    // this is automatic, so we emit it as a no-op.
    m.insert(
        "__builtin_cpu_init",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::CpuInit),
    );
    // __builtin_cpu_supports("feature") returns nonzero if the CPU supports
    // the named feature. We conservatively return 0 (not supported) so that
    // code always takes the safe fallback path.
    m.insert(
        "__builtin_cpu_supports",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::CpuSupports),
    );

    // Cache flush - maps to __clear_cache runtime function (provided by libgcc/glibc).
    // On x86 this is a no-op (cache coherent), on ARM/RISC-V it flushes icache.
    m.insert(
        "__builtin___clear_cache",
        BuiltinInfo::simple("__clear_cache"),
    );

    // Vector construction builtins used by SSE header wrapper functions.
    // The actual _mm_set1_* calls are intercepted as direct builtins, but the
    // function bodies in emmintrin.h still reference these, so register as Nop
    // to avoid linker errors from the compiled (but never called) wrappers.
    m.insert(
        "__builtin_ia32_vec_init_v16qi",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Nop),
    );
    m.insert(
        "__builtin_ia32_vec_init_v4si",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Nop),
    );
    m.insert(
        "__builtin_ia32_vec_init_v8hi",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Nop),
    );

    // x86 SSE/SSE2/SSE4.2 intrinsic builtins (__builtin_ia32_* names)
    m.insert(
        "__builtin_ia32_lfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Lfence),
    );
    m.insert(
        "__builtin_ia32_mfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Mfence),
    );
    m.insert(
        "__builtin_ia32_sfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Sfence),
    );
    m.insert(
        "__builtin_ia32_clflush",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Clflush),
    );
    m.insert(
        "__builtin_ia32_pause",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pause),
    );
    m.insert(
        "__builtin_ia32_movnti",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movnti),
    );
    m.insert(
        "__builtin_ia32_movnti64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movnti64),
    );
    m.insert(
        "__builtin_ia32_movntdq",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movntdq),
    );
    m.insert(
        "__builtin_ia32_movntpd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movntpd),
    );
    m.insert(
        "__builtin_ia32_loaddqu",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loaddqu),
    );
    m.insert(
        "__builtin_ia32_storedqu",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storedqu),
    );
    m.insert(
        "__builtin_ia32_pcmpeqb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqb128),
    );
    m.insert(
        "__builtin_ia32_pcmpeqd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqd128),
    );
    m.insert(
        "__builtin_ia32_psubusb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubusb128),
    );
    m.insert(
        "__builtin_ia32_psubsb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubsb128),
    );
    m.insert(
        "__builtin_ia32_por128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Por128),
    );
    m.insert(
        "__builtin_ia32_pand128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pand128),
    );
    m.insert(
        "__builtin_ia32_pxor128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pxor128),
    );
    m.insert(
        "__builtin_ia32_pmovmskb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovmskb128),
    );
    m.insert(
        "__builtin_ia32_set1_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Set1Epi8),
    );
    m.insert(
        "__builtin_ia32_set1_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Set1Epi32),
    );
    // Generic SIMD intrinsic family (op encoded in the name):
    // __lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}
    m.insert(
        "__lccc_simd128_i",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::LcccSimd),
    );
    m.insert(
        "__lccc_simd128_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::LcccSimd),
    );
    m.insert(
        "__lccc_simd128_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::LcccSimd),
    );
    m.insert(
        "__lccc_simd256_i",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::LcccSimd),
    );
    m.insert(
        "__lccc_simd256_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::LcccSimd),
    );
    m.insert(
        "__lccc_simd256_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::LcccSimd),
    );
    m.insert(
        "__lccc_simd512_i",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::LcccSimd),
    );
    m.insert(
        "__lccc_simd512_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::LcccSimd),
    );
    m.insert(
        "__lccc_simd512_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::LcccSimd),
    );
    m.insert(
        "__builtin_ia32_crc32qi",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_8),
    );
    m.insert(
        "__builtin_ia32_crc32hi",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_16),
    );
    m.insert(
        "__builtin_ia32_crc32si",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_32),
    );
    m.insert(
        "__builtin_ia32_crc32di",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_64),
    );

    // x86 AES-NI builtins
    m.insert(
        "__builtin_ia32_aesenc128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenc128),
    );
    m.insert(
        "__builtin_ia32_aesenclast128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenclast128),
    );
    m.insert(
        "__builtin_ia32_aesdec128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdec128),
    );
    m.insert(
        "__builtin_ia32_aesdeclast128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdeclast128),
    );
    m.insert(
        "__builtin_ia32_aesimc128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesimc128),
    );
    m.insert(
        "__builtin_ia32_aeskeygenassist128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aeskeygenassist128),
    );
    // x86 CLMUL
    m.insert(
        "__builtin_ia32_pclmulqdq128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pclmulqdq128),
    );
    // x86 SSE2 shift/shuffle builtins
    m.insert(
        "__builtin_ia32_pslldqi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pslldqi128),
    );
    m.insert(
        "__builtin_ia32_psrldqi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrldqi128),
    );
    m.insert(
        "__builtin_ia32_psllqi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllqi128),
    );
    m.insert(
        "__builtin_ia32_psrlqi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlqi128),
    );
    m.insert(
        "__builtin_ia32_pshufd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufd128),
    );
    m.insert(
        "__builtin_ia32_loadldi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loadldi128),
    );

    // Direct _mm_* function name mappings (bypass wrapper functions, avoid ABI issues)
    m.insert(
        "_mm_loadu_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loaddqu),
    );
    m.insert(
        "_mm_load_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loaddqu),
    );
    m.insert(
        "_mm_storeu_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storedqu),
    );
    m.insert(
        "_mm_store_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storedqu),
    );
    m.insert(
        "_mm_set1_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Set1Epi8),
    );
    m.insert(
        "_mm_set1_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Set1Epi32),
    );
    // _mm_setzero_si128 is handled by its header inline function (returns compound literal)
    // Do NOT map it here -- it takes 0 args, unlike Set1Epi8 which takes 1.
    m.insert(
        "_mm_cmpeq_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqb128),
    );
    m.insert(
        "_mm_cmpeq_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqd128),
    );
    m.insert(
        "_mm_subs_epu8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubusb128),
    );
    m.insert(
        "_mm_subs_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubsb128),
    );
    m.insert(
        "_mm_or_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Por128),
    );
    m.insert(
        "_mm_and_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pand128),
    );
    m.insert(
        "_mm_xor_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pxor128),
    );
    // Float SSE ops: the bundled headers implement these via scalar
    // __builtin_memcpy fallbacks; mapping them to real intrinsics lets the
    // backend emit xorps/pxor etc. instead of ~6 instructions + a libc call.
    m.insert(
        "_mm_xor_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86XorPs),
    );
    m.insert(
        "_mm_xor_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86XorPs),
    );
    m.insert(
        "_mm_and_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86AndPs),
    );
    m.insert(
        "_mm_and_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86AndPs),
    );
    m.insert(
        "_mm_or_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86OrPs),
    );
    m.insert(
        "_mm_or_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86OrPs),
    );
    m.insert(
        "_mm_add_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86AddPs),
    );
    m.insert(
        "_mm_sub_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SubPs),
    );
    m.insert(
        "_mm_mul_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MulPs),
    );
    m.insert(
        "_mm_mul_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MulPd),
    );
    m.insert(
        "_mm_mul_epu32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MulEpu32),
    );
    m.insert(
        "_mm_mul_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MulEpi32),
    );
    m.insert(
        "_mm_mullo_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MulloEpi32),
    );
    m.insert(
        "_mm_add_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86AddPd),
    );
    m.insert(
        "_mm_sub_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SubPd),
    );
    // Free 128-bit reinterpret casts: _mm_castsi128_ps, _mm_castps_si128, ...
    // GCC/ICC lower these to nothing; the bundled header routes them through
    // __builtin_memcpy. Recognize them so they become pass-throughs.
    m.insert(
        "_mm_castsi128_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castps_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castsi128_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castpd_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castps_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castpd_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castsi64_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castps_si64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castsi64_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castpd_si64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castsi128_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castpd_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_castps_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CastReinterpret),
    );
    m.insert(
        "_mm_movemask_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovmskb128),
    );
    m.insert(
        "_mm_stream_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movntdq),
    );
    m.insert(
        "_mm_stream_si64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movnti64),
    );
    m.insert(
        "_mm_stream_si32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movnti),
    );
    m.insert(
        "_mm_stream_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movntpd),
    );
    m.insert(
        "_mm_lfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Lfence),
    );
    m.insert(
        "_mm_mfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Mfence),
    );
    m.insert(
        "_mm_sfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Sfence),
    );
    m.insert(
        "_mm_clflush",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Clflush),
    );
    m.insert(
        "_mm_pause",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pause),
    );
    m.insert(
        "_mm256_zeroupper",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Vzeroupper),
    );
    m.insert(
        "_mm_crc32_u8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_8),
    );
    m.insert(
        "_mm_crc32_u16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_16),
    );
    m.insert(
        "_mm_crc32_u32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_32),
    );
    m.insert(
        "_mm_crc32_u64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_64),
    );
    // Direct _mm_* AES-NI/CLMUL/shift mappings
    m.insert(
        "_mm_aesenc_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenc128),
    );
    m.insert(
        "_mm_aesenclast_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenclast128),
    );
    m.insert(
        "_mm_aesdec_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdec128),
    );
    m.insert(
        "_mm_aesdeclast_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdeclast128),
    );
    m.insert(
        "_mm_aesimc_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesimc128),
    );
    m.insert(
        "_mm_aeskeygenassist_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aeskeygenassist128),
    );
    m.insert(
        "_mm_clmulepi64_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pclmulqdq128),
    );
    m.insert(
        "_mm_slli_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pslldqi128),
    );
    m.insert(
        "_mm_srli_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrldqi128),
    );
    m.insert(
        "_mm_slli_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllqi128),
    );
    m.insert(
        "_mm_srli_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlqi128),
    );
    m.insert(
        "_mm_shuffle_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufd128),
    );
    m.insert(
        "_mm_loadl_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loadldi128),
    );

    // New SSE2 packed 16-bit _mm_* mappings
    m.insert(
        "_mm_add_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddw128),
    );
    m.insert(
        "_mm_sub_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubw128),
    );
    m.insert(
        "_mm_mulhi_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmulhw128),
    );
    m.insert(
        "_mm_madd_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaddwd128),
    );
    m.insert(
        "_mm_cmpgt_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtw128),
    );
    m.insert(
        "_mm_cmpgt_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtb128),
    );
    m.insert(
        "_mm_slli_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllwi128),
    );
    m.insert(
        "_mm_srli_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlwi128),
    );
    m.insert(
        "_mm_srai_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrawi128),
    );
    m.insert(
        "_mm_srai_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psradi128),
    );
    m.insert(
        "_mm_slli_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pslldi128),
    );
    m.insert(
        "_mm_srli_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrldi128),
    );
    // New SSE2 packed 32-bit _mm_* mappings
    m.insert(
        "_mm_add_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddd128),
    );
    m.insert(
        "_mm_sub_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubd128),
    );
    // New SSE2 pack/unpack _mm_* mappings
    m.insert(
        "_mm_packs_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packssdw128),
    );
    m.insert(
        "_mm_packs_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packsswb128),
    );
    m.insert(
        "_mm_packus_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packuswb128),
    );
    m.insert(
        "_mm_unpacklo_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklbw128),
    );
    m.insert(
        "_mm_unpackhi_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhbw128),
    );
    m.insert(
        "_mm_unpacklo_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklwd128),
    );
    m.insert(
        "_mm_unpackhi_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhwd128),
    );
    // New SSE2 set/insert/extract/convert _mm_* mappings
    m.insert(
        "_mm_set1_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Set1Epi16),
    );
    m.insert(
        "_mm_insert_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrw128),
    );
    m.insert(
        "_mm_extract_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrw128),
    );
    m.insert(
        "_mm_storel_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storeldi128),
    );
    m.insert(
        "_mm_cvtsi128_si32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi128Si32),
    );
    m.insert(
        "_mm_cvtsi32_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi32Si128),
    );
    m.insert(
        "_mm_cvtsi128_si64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi128Si64),
    );
    m.insert(
        "_mm_cvtsi128_si64x",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi128Si64),
    );
    m.insert(
        "_mm_shufflelo_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshuflw128),
    );
    m.insert(
        "_mm_shufflehi_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufhw128),
    );
    // SSE4.1 _mm_* direct name mappings
    m.insert(
        "_mm_insert_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrd128),
    );
    m.insert(
        "_mm_extract_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrd128),
    );
    m.insert(
        "_mm_insert_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrb128),
    );
    m.insert(
        "_mm_extract_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrb128),
    );
    m.insert(
        "_mm_insert_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrq128),
    );
    m.insert(
        "_mm_extract_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrq128),
    );

    // New SSE2/SSSE3/SSE4.1 _mm_* mappings
    m.insert(
        "_mm_add_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddb128),
    );
    m.insert(
        "_mm_sub_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubb128),
    );
    m.insert(
        "_mm_subs_epu16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubusw128),
    );
    m.insert(
        "_mm_sad_epu8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psadbw128),
    );
    m.insert(
        "_mm_mullo_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmullw128),
    );
    m.insert(
        "_mm_maddubs_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaddubsw128),
    );
    m.insert(
        "_mm_hadd_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Phaddw128),
    );
    m.insert(
        "_mm_hadd_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Phaddd128),
    );
    m.insert(
        "_mm_shuffle_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufb128),
    );
    m.insert(
        "_mm_abs_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsb128),
    );
    m.insert(
        "_mm_abs_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsw128),
    );
    m.insert(
        "_mm_abs_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsd128),
    );
    m.insert(
        "_mm_alignr_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Palignr128),
    );
    m.insert(
        "_mm_max_epu8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaxub128),
    );
    m.insert(
        "_mm_min_epu8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pminub128),
    );
    m.insert(
        "_mm_blendv_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pblendvb128),
    );
    m.insert(
        "_mm_blend_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pblendw128),
    );
    m.insert(
        "__builtin_ia32_pblendw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pblendw128),
    );
    m.insert(
        "_mm_cvtepu8_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovzxbw128),
    );
    m.insert(
        "_mm_cvtepu16_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovzxwd128),
    );
    m.insert(
        "_mm_sll_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllw128),
    );
    m.insert(
        "_mm_srl_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlw128),
    );
    // __builtin_ia32_* names for new ops
    // GCC vector-extension FP min/max: used by kernel code that cannot
    // include userspace intrin headers (the NAP cpuidle governor's
    // kernel_fpu_begin() translation units operate on
    // `float __attribute__((vector_size(16)))` directly).
    m.insert(
        "__builtin_ia32_maxps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MaxPs128),
    );
    m.insert(
        "__builtin_ia32_minps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MinPs128),
    );
    // Raw GCC builtins used by the NAP governor's SSE2/AVX2 NN kernels.
    m.insert(
        "__builtin_ia32_shufps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86ShufPsValue),
    );
    // glibc sysdeps/x86/fpu/sincosf_poly.h: v4sf = __builtin_ia32_cvtpd2ps(v2df).
    m.insert(
        "__builtin_ia32_cvtpd2ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CvtPd2PsValue),
    );
    m.insert(
        "__builtin_shufflevector",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ShuffleVector),
    );
    m.insert(
        "__builtin_shuffle",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Shuffle),
    );
    m.insert(
        "__builtin_ia32_maxps256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MaxPs256V),
    );
    m.insert(
        "__builtin_ia32_minps256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MinPs256V),
    );
    m.insert(
        "__builtin_ia32_andps256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86AndPs256V),
    );
    m.insert(
        "__builtin_ia32_cmpps256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86CmpPs256V),
    );
    m.insert(
        "__builtin_ia32_vextractf128_ps256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Vextractf128V),
    );
    m.insert(
        "__builtin_ia32_vzeroupper",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Vzeroupper),
    );
    m.insert(
        "__builtin_ia32_paddb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddb128),
    );
    m.insert(
        "__builtin_ia32_psubb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubb128),
    );
    m.insert(
        "__builtin_ia32_psubusw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubusw128),
    );
    m.insert(
        "__builtin_ia32_psadbw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psadbw128),
    );
    m.insert(
        "__builtin_ia32_pmullw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmullw128),
    );
    m.insert(
        "__builtin_ia32_pmaddubsw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaddubsw128),
    );
    m.insert(
        "__builtin_ia32_phaddw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Phaddw128),
    );
    m.insert(
        "__builtin_ia32_phaddd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Phaddd128),
    );
    m.insert(
        "__builtin_ia32_pshufb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufb128),
    );
    m.insert(
        "__builtin_ia32_pabsb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsb128),
    );
    m.insert(
        "__builtin_ia32_pabsw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsw128),
    );
    m.insert(
        "__builtin_ia32_pabsd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsd128),
    );
    m.insert(
        "__builtin_ia32_palignr128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Palignr128),
    );
    m.insert(
        "__builtin_ia32_pmaxub128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaxub128),
    );
    m.insert(
        "__builtin_ia32_pminub128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pminub128),
    );
    m.insert(
        "__builtin_ia32_pblendvb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pblendvb128),
    );
    m.insert(
        "__builtin_ia32_pmovzxbw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovzxbw128),
    );
    m.insert(
        "__builtin_ia32_pmovzxwd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovzxwd128),
    );
    // AVX2 256-bit _mm256_* mappings
    m.insert(
        "_mm256_loadu_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loadu256),
    );
    m.insert(
        "_mm256_storeu_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storeu256),
    );
    m.insert(
        "_mm256_load_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Load256),
    );
    m.insert(
        "_mm256_store_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Store256),
    );
    m.insert(
        "_mm256_add_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddb256),
    );
    m.insert(
        "_mm256_add_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddw256),
    );
    m.insert(
        "_mm256_add_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddd256),
    );
    m.insert(
        "_mm256_sub_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubb256),
    );
    m.insert(
        "_mm256_sub_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubw256),
    );
    m.insert(
        "_mm256_subs_epu16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubusw256),
    );
    m.insert(
        "_mm256_sad_epu8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psadbw256),
    );
    m.insert(
        "_mm256_maddubs_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaddubsw256),
    );
    m.insert(
        "_mm256_madd_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaddwd256),
    );
    m.insert(
        "_mm256_cmpeq_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqb256),
    );
    m.insert(
        "_mm256_cmpgt_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtb256),
    );
    m.insert(
        "_mm256_movemask_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovmskb256),
    );
    m.insert(
        "_mm256_shuffle_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufb256),
    );
    m.insert(
        "_mm256_abs_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsb256),
    );
    m.insert(
        "_mm256_abs_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsw256),
    );
    m.insert(
        "_mm256_max_epu8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaxub256),
    );
    m.insert(
        "_mm256_min_epu8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pminub256),
    );
    m.insert(
        "_mm256_xor_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pxor256),
    );
    m.insert(
        "_mm256_or_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Por256),
    );
    m.insert(
        "_mm256_and_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pand256),
    );
    m.insert(
        "_mm256_slli_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllidi256),
    );
    m.insert(
        "_mm256_srli_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlidi256),
    );
    m.insert(
        "_mm256_slli_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllwi256),
    );
    m.insert(
        "_mm256_srli_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlwi256),
    );
    m.insert(
        "_mm256_broadcastsi128_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Broadcast128to256),
    );
    m.insert(
        "_mm256_zextsi128_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Zext128to256),
    );
    m.insert(
        "_mm256_castsi256_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cast256to128),
    );
    m.insert(
        "_mm256_inserti128_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Insert128to256),
    );
    m.insert(
        "_mm256_set1_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SetEpi8_256),
    );
    m.insert(
        "_mm256_set1_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SetEpi16_256),
    );
    m.insert(
        "_mm256_set1_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SetEpi32_256),
    );
    m.insert(
        "_mm256_set1_epi64x",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SetEpi64x256),
    );
    m.insert(
        "_mm256_permutevar8x32_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Permutevar8x32),
    );
    // __builtin_ia32_* names for AVX2
    m.insert(
        "__builtin_ia32_loaddqu256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loadu256),
    );
    m.insert(
        "__builtin_ia32_storedqu256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storeu256),
    );
    m.insert(
        "__builtin_ia32_paddb256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddb256),
    );
    m.insert(
        "__builtin_ia32_paddw256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddw256),
    );
    m.insert(
        "__builtin_ia32_paddd256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddd256),
    );
    m.insert(
        "__builtin_ia32_psubb256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubb256),
    );
    m.insert(
        "__builtin_ia32_psubw256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubw256),
    );
    m.insert(
        "__builtin_ia32_psubusw256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubusw256),
    );
    m.insert(
        "__builtin_ia32_psadbw256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psadbw256),
    );
    m.insert(
        "__builtin_ia32_pmaddubsw256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaddubsw256),
    );
    m.insert(
        "__builtin_ia32_pmaddwd256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaddwd256),
    );
    m.insert(
        "__builtin_ia32_pcmpeqb256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqb256),
    );
    m.insert(
        "__builtin_ia32_pmovmskb256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovmskb256),
    );
    m.insert(
        "__builtin_ia32_pshufb256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufb256),
    );
    m.insert(
        "__builtin_ia32_pabsb256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsb256),
    );
    m.insert(
        "__builtin_ia32_pabsw256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsw256),
    );
    m.insert(
        "__builtin_ia32_pmaxub256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaxub256),
    );
    m.insert(
        "__builtin_ia32_pminub256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pminub256),
    );
    m.insert(
        "__builtin_ia32_pxor256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pxor256),
    );
    m.insert(
        "__builtin_ia32_por256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Por256),
    );
    m.insert(
        "__builtin_ia32_pand256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pand256),
    );
    m.insert(
        "__builtin_ia32_pslldi256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllidi256),
    );
    m.insert(
        "__builtin_ia32_psrldi256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlidi256),
    );
    m.insert(
        "__builtin_ia32_psllwi256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllwi256),
    );
    m.insert(
        "__builtin_ia32_psrlwi256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlwi256),
    );
    m.insert(
        "__builtin_ia32_vpbroadcastb256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SetEpi8_256),
    );
    m.insert(
        "__builtin_ia32_vpbroadcastw256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SetEpi16_256),
    );
    m.insert(
        "__builtin_ia32_vpbroadcastd256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SetEpi32_256),
    );
    m.insert(
        "__builtin_ia32_vpbroadcastq256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SetEpi64x256),
    );
    m.insert(
        "__builtin_ia32_permvarsi256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Permutevar8x32),
    );
    m.insert(
        "_mm_dpbusd_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusd128),
    );
    m.insert(
        "_mm_dpbusds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusds128),
    );
    m.insert(
        "_mm_dpwusd_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusd128),
    );
    m.insert(
        "_mm_dpwusds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusds128),
    );
    m.insert(
        "_mm256_dpbusd_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusd256),
    );
    m.insert(
        "_mm256_dpbusds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusds256),
    );
    m.insert(
        "_mm256_dpwusd_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusd256),
    );
    m.insert(
        "_mm256_dpwusds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusds256),
    );
    m.insert(
        "_mm_dpbssd_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbssd128),
    );
    m.insert(
        "_mm_dpbssds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbssds128),
    );
    m.insert(
        "_mm_dpbsud_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbsud128),
    );
    m.insert(
        "_mm_dpbsuds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbsuds128),
    );
    m.insert(
        "_mm_dpbuud_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbuud128),
    );
    m.insert(
        "_mm_dpbuuds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbuuds128),
    );
    m.insert(
        "_mm256_dpbssd_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbssd256),
    );
    m.insert(
        "_mm256_dpbssds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbssds256),
    );
    m.insert(
        "_mm256_dpbsud_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbsud256),
    );
    m.insert(
        "_mm256_dpbsuds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbsuds256),
    );
    m.insert(
        "_mm256_dpbuud_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbuud256),
    );
    m.insert(
        "_mm256_dpbuuds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbuuds256),
    );
    m.insert(
        "_mm_dpwuud_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuud128),
    );
    m.insert(
        "_mm_dpwuuds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuuds128),
    );
    m.insert(
        "_mm_dpwssd_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssd128),
    );
    m.insert(
        "_mm_dpwssds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssds128),
    );
    m.insert(
        "_mm256_dpwuud_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuud256),
    );
    m.insert(
        "_mm256_dpwuuds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuuds256),
    );
    m.insert(
        "_mm256_dpwssd_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssd256),
    );
    m.insert(
        "_mm256_dpwssds_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssds256),
    );
    m.insert(
        "_mm_gf2p8mul_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Gf2p8mulb128),
    );
    m.insert(
        "_mm_gf2p8affine_epi64_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Gf2p8affineqb128),
    );
    m.insert(
        "_mm_gf2p8affineinv_epi64_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Gf2p8affineinvqb128),
    );
    m.insert(
        "_mm256_aesenc_epi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenc256),
    );
    m.insert(
        "_mm256_aesenclast_epi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenclast256),
    );
    m.insert(
        "_mm256_aesdec_epi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdec256),
    );
    m.insert(
        "_mm256_aesdeclast_epi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdeclast256),
    );
    m.insert(
        "_mm256_clmulepi64_epi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Vpclmulqdq256),
    );
    m.insert(
        "__builtin_ia32_vpdpbusd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusd128),
    );
    m.insert(
        "__builtin_ia32_vpdpbusd256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusd256),
    );
    m.insert(
        "__builtin_ia32_vpdpbusds128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusds128),
    );
    m.insert(
        "__builtin_ia32_vpdpbusds256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusds256),
    );
    m.insert(
        "__builtin_ia32_vpdpwusd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusd128),
    );
    m.insert(
        "__builtin_ia32_vpdpwusd256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusd256),
    );
    m.insert(
        "__builtin_ia32_vpdpwusds128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusds128),
    );
    m.insert(
        "__builtin_ia32_vpdpwusds256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusds256),
    );
    m.insert(
        "__builtin_ia32_vpdpbssd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbssd128),
    );
    m.insert(
        "__builtin_ia32_vpdpbssd256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbssd256),
    );
    m.insert(
        "__builtin_ia32_vpdpbssds128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbssds128),
    );
    m.insert(
        "__builtin_ia32_vpdpbssds256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbssds256),
    );
    m.insert(
        "__builtin_ia32_vpdpbsud128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbsud128),
    );
    m.insert(
        "__builtin_ia32_vpdpbsud256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbsud256),
    );
    m.insert(
        "__builtin_ia32_vpdpbsuds128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbsuds128),
    );
    m.insert(
        "__builtin_ia32_vpdpbsuds256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbsuds256),
    );
    m.insert(
        "__builtin_ia32_vpdpbuud128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbuud128),
    );
    m.insert(
        "__builtin_ia32_vpdpbuud256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbuud256),
    );
    m.insert(
        "__builtin_ia32_vpdpbuuds128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbuuds128),
    );
    m.insert(
        "__builtin_ia32_vpdpbuuds256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbuuds256),
    );
    m.insert(
        "__builtin_ia32_vpdpwuud128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuud128),
    );
    m.insert(
        "__builtin_ia32_vpdpwuud256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuud256),
    );
    m.insert(
        "__builtin_ia32_vpdpwuuds128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuuds128),
    );
    m.insert(
        "__builtin_ia32_vpdpwuuds256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuuds256),
    );
    m.insert(
        "__builtin_ia32_vpdpwssd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssd128),
    );
    m.insert(
        "__builtin_ia32_vpdpwssd256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssd256),
    );
    m.insert(
        "__builtin_ia32_vpdpwssds128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssds128),
    );
    m.insert(
        "__builtin_ia32_vpdpwssds256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssds256),
    );
    m.insert(
        "__builtin_ia32_gf2p8mulb",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Gf2p8mulb128),
    );
    m.insert(
        "__builtin_ia32_gf2p8affineqb",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Gf2p8affineqb128),
    );
    m.insert(
        "__builtin_ia32_gf2p8affineinvqb",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Gf2p8affineinvqb128),
    );
    m.insert(
        "__builtin_ia32_aesenc256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenc256),
    );
    m.insert(
        "__builtin_ia32_aesenclast256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenclast256),
    );
    m.insert(
        "__builtin_ia32_aesdec256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdec256),
    );
    m.insert(
        "__builtin_ia32_aesdeclast256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdeclast256),
    );
    m.insert(
        "__builtin_ia32_vpclmulqdq256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Vpclmulqdq256),
    );
    // GCC-compatible aliases (GCC 16 uses _avx_ infix for the VEX VNNI forms,
    // and an odd dpwsud spelling for INT16 vpdpwusd; accept both spellings).
    m.insert(
        "_mm_dpbusd_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusd128),
    );
    m.insert(
        "_mm_dpbusds_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusds128),
    );
    m.insert(
        "_mm_dpwusd_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusd128),
    );
    m.insert(
        "_mm_dpwusds_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusds128),
    );
    m.insert(
        "_mm256_dpbusd_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusd256),
    );
    m.insert(
        "_mm256_dpbusds_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpbusds256),
    );
    m.insert(
        "_mm256_dpwusd_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusd256),
    );
    m.insert(
        "_mm256_dpwusds_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusds256),
    );
    m.insert(
        "_mm_dpwsud_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusd128),
    );
    m.insert(
        "_mm_dpwsuds_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusds128),
    );
    m.insert(
        "_mm256_dpwsud_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusd256),
    );
    m.insert(
        "_mm256_dpwsuds_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwusds256),
    );
    m.insert(
        "_mm_dpwssd_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssd128),
    );
    m.insert(
        "_mm_dpwssds_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssds128),
    );
    m.insert(
        "_mm256_dpwssd_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssd256),
    );
    m.insert(
        "_mm256_dpwssds_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwssds256),
    );
    m.insert(
        "_mm_dpwuud_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuud128),
    );
    m.insert(
        "_mm_dpwuuds_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuuds128),
    );
    m.insert(
        "_mm256_dpwuud_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuud256),
    );
    m.insert(
        "_mm256_dpwuuds_avx_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Dpwuuds256),
    );
    m.insert(
        "__builtin_ia32_vzextsi128_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Zext128to256),
    );
    m.insert(
        "__builtin_ia32_vinserti128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Insert128to256),
    );
    m.insert(
        "__builtin_ia32_vextracti128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cast256to128),
    );
    m.insert(
        "__builtin_ia32_vbroadcasti128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Broadcast128to256),
    );
    m.insert(
        "__builtin_ia32_inserti128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Insert128to256),
    );
    m.insert(
        "__builtin_ia32_extracti128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cast256to128),
    );
    // __builtin_ia32_* names for the new SSE2 operations
    m.insert(
        "__builtin_ia32_paddw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddw128),
    );
    m.insert(
        "__builtin_ia32_psubw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubw128),
    );
    m.insert(
        "__builtin_ia32_pmulhw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmulhw128),
    );
    m.insert(
        "__builtin_ia32_pmaddwd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaddwd128),
    );
    m.insert(
        "__builtin_ia32_pcmpgtw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtw128),
    );
    m.insert(
        "__builtin_ia32_pcmpgtb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtb128),
    );
    m.insert(
        "__builtin_ia32_psllwi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllwi128),
    );
    m.insert(
        "__builtin_ia32_psrlwi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlwi128),
    );
    m.insert(
        "__builtin_ia32_psrawi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrawi128),
    );
    m.insert(
        "__builtin_ia32_psradi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psradi128),
    );
    m.insert(
        "__builtin_ia32_pslldi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pslldi128),
    );
    m.insert(
        "__builtin_ia32_psrldi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrldi128),
    );
    m.insert(
        "__builtin_ia32_paddd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddd128),
    );
    m.insert(
        "__builtin_ia32_psubd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubd128),
    );
    m.insert(
        "__builtin_ia32_packssdw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packssdw128),
    );
    m.insert(
        "__builtin_ia32_packsswb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packsswb128),
    );
    m.insert(
        "__builtin_ia32_packuswb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packuswb128),
    );
    m.insert(
        "__builtin_ia32_punpcklbw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklbw128),
    );
    m.insert(
        "__builtin_ia32_punpckhbw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhbw128),
    );
    m.insert(
        "__builtin_ia32_punpcklwd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklwd128),
    );
    m.insert(
        "__builtin_ia32_punpckhwd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhwd128),
    );
    m.insert(
        "__builtin_ia32_pinsrw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrw128),
    );
    m.insert(
        "__builtin_ia32_pextrw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrw128),
    );
    m.insert(
        "__builtin_ia32_storeldi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storeldi128),
    );
    m.insert(
        "__builtin_ia32_cvtsi128si32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi128Si32),
    );
    m.insert(
        "__builtin_ia32_cvtsi32si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi32Si128),
    );
    m.insert(
        "__builtin_ia32_cvtsi128si64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi128Si64),
    );
    m.insert(
        "__builtin_ia32_pshuflw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshuflw128),
    );
    m.insert(
        "__builtin_ia32_pshufhw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufhw128),
    );

    // x86 SSE4.1 insert/extract builtins (__builtin_ia32_* names)
    m.insert(
        "__builtin_ia32_pinsrd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrd128),
    );
    m.insert(
        "__builtin_ia32_pextrd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrd128),
    );
    m.insert(
        "__builtin_ia32_pinsrb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrb128),
    );
    m.insert(
        "__builtin_ia32_pextrb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrb128),
    );
    m.insert(
        "__builtin_ia32_pinsrq128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrq128),
    );
    m.insert(
        "__builtin_ia32_pextrq128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrq128),
    );

    // Hardware emission for ops that previously compiled as scalar C loops.
    m.insert(
        "_mm_adds_epu8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddusb128),
    );
    m.insert(
        "_mm_adds_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddsb128),
    );
    m.insert(
        "_mm_adds_epu16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddusw128),
    );
    m.insert(
        "_mm_adds_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddsw128),
    );
    m.insert(
        "_mm_subs_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubsw128),
    );
    m.insert(
        "_mm_andnot_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pandn128),
    );
    m.insert(
        "_mm_andnot_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pandn128),
    );
    m.insert(
        "_mm_andnot_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pandn128),
    );
    m.insert(
        "_mm_cmpeq_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqw128),
    );
    m.insert(
        "_mm_cmpgt_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtd128),
    );
    m.insert(
        "_mm_avg_epu8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pavgb128),
    );
    m.insert(
        "_mm_avg_epu16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pavgw128),
    );
    m.insert(
        "_mm_min_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pminsw128),
    );
    m.insert(
        "_mm_max_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaxsw128),
    );
    m.insert(
        "_mm_mulhi_epu16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmulhuw128),
    );
    m.insert(
        "_mm_add_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddq128),
    );
    m.insert(
        "_mm_sub_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubq128),
    );
    m.insert(
        "_mm_unpacklo_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckldq128),
    );
    m.insert(
        "_mm_unpackhi_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhdq128),
    );
    m.insert(
        "_mm_unpacklo_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklqdq128),
    );
    m.insert(
        "_mm_unpackhi_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhqdq128),
    );
    m.insert(
        "_mm_setzero_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Setzero128),
    );
    m.insert(
        "_mm_setzero_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Setzero128),
    );
    m.insert(
        "_mm_setzero_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Setzero128),
    );
    m.insert(
        "_mm_testz_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Testz128),
    );

    m.insert(
        "_mm256_mullo_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmulld256),
    );
    m.insert(
        "_mm256_sub_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubd256),
    );
    m.insert(
        "_mm256_add_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddq256),
    );
    m.insert(
        "_mm256_sub_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubq256),
    );
    m.insert(
        "_mm256_andnot_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pandn256),
    );
    m.insert(
        "_mm256_andnot_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pandn256),
    );
    m.insert(
        "_mm256_andnot_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pandn256),
    );
    m.insert(
        "_mm256_cmpeq_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqd256),
    );
    m.insert(
        "_mm256_cmpeq_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqq256),
    );
    m.insert(
        "_mm256_cmpgt_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtd256),
    );
    m.insert(
        "_mm256_cmpgt_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtq256),
    );
    m.insert(
        "_mm256_extracti128_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Extracti128),
    );
    m.insert(
        "_mm256_extractf128_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Extracti128),
    );
    m.insert(
        "_mm256_extractf128_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Extracti128),
    );
    m.insert(
        "_mm256_setzero_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Setzero256),
    );
    m.insert(
        "_mm256_setzero_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Setzero256),
    );
    m.insert(
        "_mm256_setzero_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Setzero256),
    );
    m.insert(
        "_mm256_add_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86AddPs256),
    );
    m.insert(
        "_mm256_sub_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SubPs256),
    );
    m.insert(
        "_mm256_mul_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MulPs256),
    );
    m.insert(
        "_mm256_add_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86AddPd256),
    );
    m.insert(
        "_mm256_sub_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86SubPd256),
    );
    m.insert(
        "_mm256_mul_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86MulPd256),
    );
    m.insert(
        "_mm256_loadu_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86LoaduPs256),
    );
    m.insert(
        "_mm256_storeu_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86StoreuPs256),
    );
    m.insert(
        "_mm256_loadu_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86LoaduPd256),
    );
    m.insert(
        "_mm256_storeu_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86StoreuPd256),
    );
    m.insert(
        "_mm256_permute2x128_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Permute2x128),
    );
    m.insert(
        "_mm256_permute2f128_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Permute2x128),
    );
    m.insert(
        "_mm256_permute2f128_ps",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Permute2x128),
    );
    m.insert(
        "_mm256_permute2f128_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Permute2x128),
    );
    m.insert(
        "_mm256_permute4x64_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Permute4x64),
    );
    m.insert(
        "_mm256_shuffle_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufd256),
    );
    m.insert(
        "_mm256_unpacklo_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklbw256),
    );
    m.insert(
        "_mm256_unpackhi_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhbw256),
    );
    m.insert(
        "_mm256_unpacklo_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklwd256),
    );
    m.insert(
        "_mm256_unpackhi_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhwd256),
    );
    m.insert(
        "_mm256_unpacklo_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckldq256),
    );
    m.insert(
        "_mm256_unpackhi_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhdq256),
    );
    m.insert(
        "_mm256_unpacklo_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklqdq256),
    );
    m.insert(
        "_mm256_unpackhi_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhqdq256),
    );
    m.insert(
        "_mm256_slli_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pslldqi256),
    );
    m.insert(
        "_mm256_srli_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrldqi256),
    );
    m.insert(
        "_mm256_bslli_epi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pslldqi256),
    );
    m.insert(
        "_mm256_bsrli_epi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrldqi256),
    );
    m.insert(
        "_mm256_slli_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllqi256),
    );
    m.insert(
        "_mm256_srli_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlqi256),
    );
    m.insert(
        "_mm256_mullo_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmullw256),
    );
    m.insert(
        "_mm256_mulhi_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmulhw256),
    );
    m.insert(
        "_mm256_min_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pminsd256),
    );
    m.insert(
        "_mm256_max_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaxsd256),
    );
    m.insert(
        "_mm256_cvtepu8_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovzxbw256),
    );
    m.insert(
        "_mm256_cvtepu8_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovzxbd256),
    );
    m.insert(
        "_mm256_cvtepu16_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovzxwd256),
    );
    m.insert(
        "_mm256_cvtepi8_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovsxbw256),
    );
    m.insert(
        "_mm256_cvtepi8_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovsxbd256),
    );
    m.insert(
        "_mm256_cvtepi16_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovsxwd256),
    );
    m.insert(
        "_mm256_srai_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrawi256),
    );
    m.insert(
        "_mm256_srai_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psradi256),
    );
    m.insert(
        "_mm256_packs_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packssdw256),
    );
    m.insert(
        "_mm256_packus_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packuswb256),
    );
    m.insert(
        "_mm256_hadd_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Phaddw256),
    );
    m.insert(
        "_mm256_hadd_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Phaddd256),
    );
    m.insert(
        "_mm256_abs_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pabsd256),
    );
    m.insert(
        "_mm256_mul_epu32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmuludq256),
    );
    m.insert(
        "_mm256_castsi128_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Zext128to256),
    );
    m.insert(
        "_mm256_castps128_ps256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Zext128to256),
    );
    m.insert(
        "_mm256_castpd128_pd256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Zext128to256),
    );
    m.insert(
        "_mm256_castps256_ps128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cast256to128),
    );
    m.insert(
        "_mm256_castpd256_pd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cast256to128),
    );
    m.insert(
        "_mm256_lddqu_si256",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loadu256),
    );

    m
});

/// How a builtin should be handled during lowering.
#[derive(Debug, Clone)]
pub struct BuiltinInfo {
    pub kind: BuiltinKind,
}

/// The kind of builtin behavior.
#[derive(Debug, Clone)]
pub enum BuiltinKind {
    /// Map directly to a libc function call.
    LibcAlias(String),
    /// Return the first argument unchanged (__builtin_expect).
    Identity,
    /// Evaluate to a compile-time float constant.
    ConstantF64(f64),
    /// Evaluate to a compile-time _Float128 constant (full 16-byte IEEE-754
    /// binary128 payload; glibc math uses __builtin_huge_valf128 etc.).
    ConstantF128([u8; 16]),
    /// Requires special codegen (CLZ, CTZ, popcount, bswap, etc.).
    Intrinsic(BuiltinIntrinsic),
}

/// Intrinsics that need target-specific codegen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinIntrinsic {
    Clz,
    Ctz,
    /// ffs(x) = x == 0 ? 0 : ctz(x) + 1 — find first set bit (1-indexed)
    Ffs,
    Clrsb,
    Popcount,
    Bswap,
    Fence,
    FpCompare,
    Parity,
    /// creal/crealf/creall: extract real part of complex number
    ComplexReal,
    /// cimag/cimagf/cimagl: extract imaginary part of complex number
    ComplexImag,
    /// conj/conjf/conjl: compute complex conjugate
    ComplexConj,
    /// __builtin_fpclassify(nan, inf, norm, subnorm, zero, x) -> int
    FpClassify,
    /// __builtin_isnan(x) -> int (1 if NaN, 0 otherwise)
    IsNan,
    /// __builtin_isinf(x) -> int (1 if +/-inf, 0 otherwise)
    IsInf,
    /// __builtin_isfinite(x) -> int (1 if finite, 0 otherwise)
    IsFinite,
    /// __builtin_isnormal(x) -> int (1 if normal, 0 otherwise)
    IsNormal,
    /// __builtin_signbit(x) -> int (nonzero if sign bit set)
    SignBit,
    /// __builtin_isinf_sign(x) -> int (-1 if -inf, 1 if +inf, 0 otherwise)
    IsInfSign,
    /// __builtin_alloca(size) -> dynamic stack allocation
    Alloca,
    /// __builtin_complex(real, imag) -> construct complex number
    ComplexConstruct,
    /// __builtin_va_start(ap, last) -> initialize va_list (lowered specially in IR)
    VaStart,
    /// __builtin_va_end(ap) -> cleanup va_list (lowered specially in IR)
    VaEnd,
    /// __builtin_va_copy(dest, src) -> copy va_list (lowered specially in IR)
    VaCopy,
    /// __builtin_constant_p(expr) -> 1 if expr is a compile-time constant, 0 otherwise
    ConstantP,
    /// __builtin_object_size(ptr, type) -> size of object ptr points to, or -1/0 if unknown
    ObjectSize,
    /// __builtin_classify_type(expr) -> integer type class of the expression's type
    ClassifyType,
    /// No-op builtin (evaluates args, returns 0)
    Nop,
    /// __builtin_cpu_init() - no-op, glibc handles CPU feature detection init
    CpuInit,
    /// __builtin_cpu_supports("feature") - conservatively returns 0 (unsupported)
    CpuSupports,
    /// __builtin_add_overflow(a, b, result_ptr) -> bool (1 if overflow)
    AddOverflow,
    /// __builtin_sub_overflow(a, b, result_ptr) -> bool (1 if overflow)
    SubOverflow,
    /// __builtin_mul_overflow(a, b, result_ptr) -> bool (1 if overflow)
    MulOverflow,
    /// __builtin_add_overflow_p(a, b, (T)0) -> bool (1 if a+b overflows type T)
    AddOverflowP,
    /// __builtin_sub_overflow_p(a, b, (T)0) -> bool (1 if a-b overflows type T)
    SubOverflowP,
    /// __builtin_mul_overflow_p(a, b, (T)0) -> bool (1 if a*b overflows type T)
    MulOverflowP,
    /// __builtin_frame_address(level) -> returns frame pointer
    FrameAddress,
    /// __builtin_return_address(level) -> returns return address
    ReturnAddress,
    /// __builtin_setjmp(buffer) -> returns 0 initially, 1 after builtin longjmp.
    BuiltinSetjmp,
    /// __builtin_longjmp(buffer, 1) -> non-local transfer.
    BuiltinLongjmp,
    /// __builtin_apply_args() -> void* save area with the incoming arguments.
    ApplyArgs,
    /// __builtin_apply(func, args, size) -> void* result block.
    Apply,
    /// __builtin_return(block) -> returns from the current function.
    BuiltinReturn,
    /// __builtin_ia32_rdtsc() - 64-bit timestamp counter
    X86Rdtsc,
    /// __builtin_ia32_rdtscp(&aux) - rdtscp with aux store
    X86Rdtscp,
    /// Generic SIMD intrinsic family: __lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}.
    /// The instruction mnemonic is part of the name; lowering maps it to an
    /// IntrinsicOp via a table (single audit point for ~1500 intrinsics).
    LcccSimd,
    // X86 SSE intrinsics
    X86Lfence,
    X86Mfence,
    X86Sfence,
    X86Pause,
    X86Vzeroupper,
    X86Clflush,
    X86Movnti,
    X86Movnti64,
    X86Movntdq,
    X86Movntpd,
    X86Loaddqu,
    X86Storedqu,
    X86Pcmpeqb128,
    X86Pcmpeqd128,
    X86Psubusb128,
    X86Psubsb128,
    X86Por128,
    X86Pand128,
    X86Pxor128,
    X86Pmovmskb128,
    X86Set1Epi8,
    X86Set1Epi32,
    X86Crc32_8,
    X86Crc32_16,
    X86Crc32_32,
    X86Crc32_64,
    // AES-NI intrinsics
    X86Aesenc128,
    X86Aesenclast128,
    X86Aesdec128,
    X86Aesdeclast128,
    X86Aesimc128,
    X86Aeskeygenassist128,
    // CLMUL
    X86Pclmulqdq128,
    // SSE2 shift/shuffle
    X86Pslldqi128, // _mm_slli_si128 (byte shift left)
    X86Psrldqi128, // _mm_srli_si128 (byte shift right)
    X86Psllqi128,  // _mm_slli_epi64 (bit shift left per 64-bit lane)
    X86Psrlqi128,  // _mm_srli_epi64 (bit shift right per 64-bit lane)
    X86Pshufd128,  // _mm_shuffle_epi32
    X86Loadldi128, // _mm_loadl_epi64 (load low 64 bits)
    // SSE2 packed 16-bit operations
    X86Paddw128,   // _mm_add_epi16 (PADDW)
    X86Psubw128,   // _mm_sub_epi16 (PSUBW)
    X86Pmulhw128,  // _mm_mulhi_epi16 (PMULHW)
    X86Pmaddwd128, // _mm_madd_epi16 (PMADDWD)
    X86Pcmpgtw128, // _mm_cmpgt_epi16 (PCMPGTW)
    X86Pcmpgtb128, // _mm_cmpgt_epi8 (PCMPGTB)
    X86Psllwi128,  // _mm_slli_epi16 (PSLLW imm)
    X86Psrlwi128,  // _mm_srli_epi16 (PSRLW imm)
    X86Psrawi128,  // _mm_srai_epi16 (PSRAW imm)
    X86Psradi128,  // _mm_srai_epi32 (PSRAD imm)
    X86Pslldi128,  // _mm_slli_epi32 (PSLLD imm)
    X86Psrldi128,  // _mm_srli_epi32 (PSRLD imm)
    // SSE2 packed 32-bit operations
    X86Paddd128, // _mm_add_epi32 (PADDD)
    X86Psubd128, // _mm_sub_epi32 (PSUBD)
    // SSE2 pack/unpack
    X86Packssdw128,  // _mm_packs_epi32 (PACKSSDW)
    X86Packsswb128,  // _mm_packs_epi16 (PACKSSWB)
    X86Packuswb128,  // _mm_packus_epi16 (PACKUSWB)
    X86Punpcklbw128, // _mm_unpacklo_epi8 (PUNPCKLBW)
    X86Punpckhbw128, // _mm_unpackhi_epi8 (PUNPCKHBW)
    X86Punpcklwd128, // _mm_unpacklo_epi16 (PUNPCKLWD)
    X86Punpckhwd128, // _mm_unpackhi_epi16 (PUNPCKHWD)
    // SSE2 set/insert/extract/convert
    X86Set1Epi16,    // _mm_set1_epi16 (splat 16-bit)
    X86Pinsrw128,    // _mm_insert_epi16 (PINSRW)
    X86Pextrw128,    // _mm_extract_epi16 (PEXTRW)
    X86Storeldi128,  // _mm_storel_epi64 (MOVQ store)
    X86Cvtsi128Si32, // _mm_cvtsi128_si32 (MOVD extract)
    X86Cvtsi32Si128, // _mm_cvtsi32_si128 (MOVD insert)
    X86Cvtsi128Si64, // _mm_cvtsi128_si64
    X86Pshuflw128,   // _mm_shufflelo_epi16 (PSHUFLW)
    X86Pshufhw128,   // _mm_shufflehi_epi16 (PSHUFHW)
    // SSE4.1 insert/extract operations
    X86Pinsrd128,     // _mm_insert_epi32 (PINSRD)
    X86Pextrd128,     // _mm_extract_epi32 (PEXTRD)
    X86Pinsrb128,     // _mm_insert_epi8 (PINSRB)
    X86Pextrb128,     // _mm_extract_epi8 (PEXTRB)
    X86Pinsrq128,     // _mm_insert_epi64 (PINSRQ)
    X86Pextrq128,     // _mm_extract_epi64 (PEXTRQ)
    X86Paddb128,      // _mm_add_epi8 (PADDB)
    X86Psubb128,      // _mm_sub_epi8 (PSUBB)
    X86Psubusw128,    // _mm_subs_epu16 (PSUBUSW)
    X86Psadbw128,     // _mm_sad_epu8 (PSADBW)
    X86Pmullw128,     // _mm_mullo_epi16 (PMULLW)
    X86Pmaddubsw128,  // _mm_maddubs_epi16 (PMADDUBSW, SSSE3)
    X86Phaddw128,     // _mm_hadd_epi16 (PHADDW, SSSE3)
    X86Phaddd128,     // _mm_hadd_epi32 (PHADDD, SSSE3)
    X86Pshufb128,     // _mm_shuffle_epi8 (PSHUFB, SSSE3)
    X86Pabsb128,      // _mm_abs_epi8 (PABSB, SSSE3)
    X86Pabsw128,      // _mm_abs_epi16 (PABSW, SSSE3)
    X86Pabsd128,      // _mm_abs_epi32 (PABSD, SSSE3)
    X86Palignr128,    // _mm_alignr_epi8 (PALIGNR, SSSE3)
    X86Pmaxub128,     // _mm_max_epu8 (PMAXUB)
    X86Pminub128,     // _mm_min_epu8 (PMINUB)
    X86MaxPs128,      // __builtin_ia32_maxps (MAXPS) — GCC vector-extension builtin
    X86MinPs128,      // __builtin_ia32_minps (MINPS)
    X86ShufPsValue,   // __builtin_ia32_shufps (SHUFPS imm8) — by-value raw builtin
    X86CvtPd2PsValue, // __builtin_ia32_cvtpd2ps (CVTPD2PS) — by-value raw builtin
    ShuffleVector,    // __builtin_shufflevector (4-lane 32-bit shuffles)
    Shuffle,          // __builtin_shuffle(v[, v2], mask) — GCC vector-extension permute
    X86MaxPs256V,     // __builtin_ia32_maxps256 (VMAXPS ymm)
    X86MinPs256V,     // __builtin_ia32_minps256 (VMINPS ymm)
    X86AndPs256V,     // __builtin_ia32_andps256 (VANDPS ymm, bitwise)
    X86CmpPs256V,     // __builtin_ia32_cmpps256 (VCMPPS ymm, imm8)
    X86Vextractf128V, // __builtin_ia32_vextractf128_ps256 (VEXTRACTF128)
    X86Pblendvb128,   // _mm_blendv_epi8 (PBLENDVB, SSE4.1)
    X86Pblendw128,    // _mm_blend_epi16 (PBLENDW, SSE4.1)
    X86Pmovzxbw128,   // _mm_cvtepu8_epi16 (PMOVZXBW, SSE4.1)
    X86Pmovzxwd128,   // _mm_cvtepu16_epi32 (PMOVZXWD, SSE4.1)
    X86Psllw128,      // _mm_sll_epi16 (PSLLW variable)
    X86Psrlw128,      // _mm_srl_epi16 (PSRLW variable)
    // AVX2 256-bit
    X86Loadu256,
    X86Storeu256,
    X86Load256,
    X86Store256,
    X86Paddb256,
    X86Paddw256,
    X86Paddd256,
    X86Psubb256,
    X86Psubw256,
    X86Psubusw256,
    X86Psadbw256,
    X86Pmaddubsw256,
    X86Pmaddwd256,
    X86Pcmpeqb256,
    X86Pcmpgtb256,
    X86Pmovmskb256,
    X86Pshufb256,
    X86Pabsb256,
    X86Pabsw256,
    X86Pmaxub256,
    X86Pminub256,
    X86Pxor256,
    X86Por256,
    X86Pand256,
    X86Psllidi256,
    X86Psrlidi256,
    X86Psllwi256,
    X86Psrlwi256,
    X86Broadcast128to256,
    X86Zext128to256,
    X86Cast256to128,
    X86Insert128to256,
    X86SetEpi8_256,
    X86SetEpi16_256,
    X86SetEpi32_256,
    X86SetEpi64x256,
    X86Permutevar8x32,
    /// Float SSE ops. The bundled headers implement these via scalar
    /// __builtin_memcpy fallbacks (catastrophic); map the common ones to the
    /// native instructions. xorps/andps/orps are bitwise — pxor/pand/por is
    /// the identical operation on the same registers.
    X86XorPs,
    X86AndPs,
    X86OrPs,
    X86AddPs,
    X86SubPs,
    X86MulPs,
    X86AddPd,
    X86SubPd,
    X86MulPd,
    X86MulEpu32,
    X86MulEpi32,
    X86MulloEpi32,
    /// Free 128-bit reinterpret casts (no instruction): _mm_castsi128_ps etc.
    X86CastReinterpret,
    // AVX-VNNI / AVX-VNNI-INT8 / AVX-VNNI-INT16
    X86Dpbusd128,
    X86Dpbusds128,
    X86Dpwusd128,
    X86Dpwusds128,
    X86Dpbusd256,
    X86Dpbusds256,
    X86Dpwusd256,
    X86Dpwusds256,
    X86Dpbssd128,
    X86Dpbssds128,
    X86Dpbsud128,
    X86Dpbsuds128,
    X86Dpbuud128,
    X86Dpbuuds128,
    X86Dpbssd256,
    X86Dpbssds256,
    X86Dpbsud256,
    X86Dpbsuds256,
    X86Dpbuud256,
    X86Dpbuuds256,
    X86Dpwuud128,
    X86Dpwuuds128,
    X86Dpwssd128,
    X86Dpwssds128,
    X86Dpwuud256,
    X86Dpwuuds256,
    X86Dpwssd256,
    X86Dpwssds256,
    // GFNI
    X86Gf2p8mulb128,
    X86Gf2p8affineqb128,
    X86Gf2p8affineinvqb128,
    // VAES 256 + VPCLMULQDQ 256
    X86Aesenc256,
    X86Aesenclast256,
    X86Aesdec256,
    X86Aesdeclast256,
    X86Vpclmulqdq256,
    // SSE2/SSE4.1 previously scalar
    X86Paddusb128,
    X86Paddsb128,
    X86Paddusw128,
    X86Paddsw128,
    X86Psubsw128,
    X86Pandn128,
    X86Pcmpeqw128,
    X86Pcmpgtd128,
    X86Pavgb128,
    X86Pavgw128,
    X86Pminsw128,
    X86Pmaxsw128,
    X86Pmulhuw128,
    X86Paddq128,
    X86Psubq128,
    X86Punpckldq128,
    X86Punpckhdq128,
    X86Punpcklqdq128,
    X86Punpckhqdq128,
    X86Setzero128,
    X86Testz128,
    // AVX/AVX2 previously scalar
    X86Pmulld256,
    X86Psubd256,
    X86Paddq256,
    X86Psubq256,
    X86Pandn256,
    X86Pcmpeqd256,
    X86Pcmpeqq256,
    X86Pcmpgtd256,
    X86Pcmpgtq256,
    X86Extracti128,
    X86Setzero256,
    X86AddPs256,
    X86SubPs256,
    X86MulPs256,
    X86AddPd256,
    X86SubPd256,
    X86MulPd256,
    X86LoaduPs256,
    X86StoreuPs256,
    X86LoaduPd256,
    X86StoreuPd256,
    X86Permute2x128,
    X86Permute4x64,
    X86Pshufd256,
    X86Punpcklbw256,
    X86Punpckhbw256,
    X86Punpcklwd256,
    X86Punpckhwd256,
    X86Punpckldq256,
    X86Punpckhdq256,
    X86Punpcklqdq256,
    X86Punpckhqdq256,
    X86Pslldqi256,
    X86Psrldqi256,
    X86Psllqi256,
    X86Psrlqi256,
    X86Pmullw256,
    X86Pmulhw256,
    X86Pminsd256,
    X86Pmaxsd256,
    X86Pmovzxbw256,
    X86Pmovzxbd256,
    X86Pmovzxwd256,
    X86Pmovsxbw256,
    X86Pmovsxbd256,
    X86Pmovsxwd256,
    X86Psrawi256,
    X86Psradi256,
    X86Packssdw256,
    X86Packuswb256,
    X86Phaddw256,
    X86Phaddd256,
    X86Pabsd256,
    X86Pmuludq256,
    /// __builtin___*_chk: fortification builtins that forward to unchecked libc equivalents
    FortifyChk,
    /// __builtin_va_arg_pack(): used in always_inline fortification wrappers, returns 0
    VaArgPack,
    /// __builtin_thread_pointer(): returns the thread pointer (TLS base address)
    ThreadPointer,
}

impl BuiltinInfo {
    fn simple(libc_name: &str) -> Self {
        Self {
            kind: BuiltinKind::LibcAlias(libc_name.to_string()),
        }
    }

    fn identity() -> Self {
        Self {
            kind: BuiltinKind::Identity,
        }
    }

    fn constant_f128(bytes: [u8; 16]) -> Self {
        BuiltinInfo {
            kind: BuiltinKind::ConstantF128(bytes),
        }
    }

    fn constant_f64(val: f64) -> Self {
        Self {
            kind: BuiltinKind::ConstantF64(val),
        }
    }

    fn intrinsic(intr: BuiltinIntrinsic) -> Self {
        Self {
            kind: BuiltinKind::Intrinsic(intr),
        }
    }
}

/// Look up a function name and return its builtin info, if it's a known builtin.
pub fn resolve_builtin(name: &str) -> Option<&'static BuiltinInfo> {
    if let Some(info) = BUILTIN_MAP.get(name) {
        return Some(info);
    }
    // Generic SIMD family: __lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}
    // — prefix-registered (the mnemonic is part of the name).
    for prefix in [
        "__lccc_simd128_i_",
        "__lccc_simd128_ps_",
        "__lccc_simd128_pd_",
        "__lccc_simd256_i_",
        "__lccc_simd256_ps_",
        "__lccc_simd256_pd_",
        "__lccc_simd512_i_",
        "__lccc_simd512_ps_",
        "__lccc_simd512_pd_",
    ] {
        if name.starts_with(prefix) {
            return BUILTIN_MAP.get("__lccc_simd512_i");
        }
    }
    None
}

/// Check if a name is a known builtin function.
///
/// This includes both explicitly registered builtins (in BUILTIN_MAP) and
/// atomic/sync builtins that are handled by pattern matching in the IR lowering
/// code (expr_atomics.rs). The atomic builtins must be recognized here so that
/// sema does not emit "implicit declaration" warnings for them.
pub fn is_builtin(name: &str) -> bool {
    if BUILTIN_MAP.contains_key(name) {
        return true;
    }
    // Generic SIMD family prefix (mnemonic suffix in the name).
    if name.starts_with("__lccc_simd128_")
        || name.starts_with("__lccc_simd256_")
        || name.starts_with("__lccc_simd512_")
    {
        return true;
    }
    // Builtins handled by name in try_lower_builtin_call (before map lookup)
    if matches!(
        name,
        "__builtin_choose_expr" | "__builtin_unreachable" | "__builtin_trap"
    ) {
        return true;
    }
    // Atomic builtins handled by pattern matching in expr_atomics.rs
    is_atomic_builtin(name)
}

/// Check if a name is an atomic/sync builtin handled by the IR lowering code.
/// These are dispatched by name pattern in try_lower_atomic_builtin() and
/// classify_fetch_op()/classify_op_fetch() rather than through the BUILTIN_MAP.
fn is_atomic_builtin(name: &str) -> bool {
    // __atomic_* family (C11-style)
    if name.starts_with("__atomic_") {
        if matches!(
            name,
            "__atomic_fetch_add"
                | "__atomic_fetch_sub"
                | "__atomic_fetch_and"
                | "__atomic_fetch_or"
                | "__atomic_fetch_xor"
                | "__atomic_fetch_nand"
                | "__atomic_add_fetch"
                | "__atomic_sub_fetch"
                | "__atomic_and_fetch"
                | "__atomic_or_fetch"
                | "__atomic_xor_fetch"
                | "__atomic_nand_fetch"
                | "__atomic_exchange_n"
                | "__atomic_exchange"
                | "__atomic_compare_exchange_n"
                | "__atomic_compare_exchange"
                | "__atomic_load_n"
                | "__atomic_load"
                | "__atomic_store_n"
                | "__atomic_store"
                | "__atomic_test_and_set"
                | "__atomic_clear"
                | "__atomic_thread_fence"
                | "__atomic_signal_fence"
                | "__atomic_is_lock_free"
                | "__atomic_always_lock_free"
        ) {
            return true;
        }
        // Also recognize size-suffixed variants: __atomic_load_4, __atomic_store_2, etc.
        return normalize_atomic_size_suffix(name).is_some();
    }
    // __sync_* family (legacy GCC-style)
    // GCC also provides size-suffixed variants (e.g., __sync_fetch_and_add_4,
    // __sync_fetch_and_add_8) which are the actual library entry points.
    // Strip the _N suffix before matching.
    if name.starts_with("__sync_") {
        let base = strip_sync_size_suffix(name);
        return matches!(
            base,
            "__sync_fetch_and_add"
                | "__sync_fetch_and_sub"
                | "__sync_fetch_and_and"
                | "__sync_fetch_and_or"
                | "__sync_fetch_and_xor"
                | "__sync_fetch_and_nand"
                | "__sync_add_and_fetch"
                | "__sync_sub_and_fetch"
                | "__sync_and_and_fetch"
                | "__sync_or_and_fetch"
                | "__sync_xor_and_fetch"
                | "__sync_nand_and_fetch"
                | "__sync_val_compare_and_swap"
                | "__sync_bool_compare_and_swap"
                | "__sync_lock_test_and_set"
                | "__sync_lock_release"
                | "__sync_synchronize"
        );
    }
    false
}

/// Strip size suffix (_1, _2, _4, _8, _16) from GCC __sync_* builtin names.
/// E.g., "__sync_fetch_and_add_8" -> "__sync_fetch_and_add".
/// Returns the name unchanged if no suffix is present.
pub fn strip_sync_size_suffix(name: &str) -> &str {
    if let Some(base) = name
        .strip_suffix("_1")
        .or_else(|| name.strip_suffix("_2"))
        .or_else(|| name.strip_suffix("_4"))
        .or_else(|| name.strip_suffix("_8"))
        .or_else(|| name.strip_suffix("_16"))
    {
        base
    } else {
        name
    }
}

/// Normalize size-suffixed __atomic_* builtins to their canonical form.
/// GCC provides size-suffixed variants (e.g., __atomic_load_4, __atomic_store_2)
/// as libatomic entry points that are also recognized as compiler builtins.
/// These follow direct-value semantics (like __atomic_load_n, __atomic_store_n).
///
/// Returns Some(canonical_name) if a size suffix was stripped, None otherwise.
pub fn normalize_atomic_size_suffix(name: &str) -> Option<&'static str> {
    if !name.starts_with("__atomic_") {
        return None;
    }
    // Try to strip a size suffix
    let base = name
        .strip_suffix("_1")
        .or_else(|| name.strip_suffix("_2"))
        .or_else(|| name.strip_suffix("_4"))
        .or_else(|| name.strip_suffix("_8"))
        .or_else(|| name.strip_suffix("_16"))?;
    // Map to canonical _n variant for ops that have pointer-return vs direct-return distinction
    match base {
        "__atomic_load" => Some("__atomic_load_n"),
        "__atomic_store" => Some("__atomic_store_n"),
        "__atomic_exchange" => Some("__atomic_exchange_n"),
        "__atomic_compare_exchange" => Some("__atomic_compare_exchange_n"),
        // For fetch-op and op-fetch variants, base name is already correct
        "__atomic_fetch_add"
        | "__atomic_fetch_sub"
        | "__atomic_fetch_and"
        | "__atomic_fetch_or"
        | "__atomic_fetch_xor"
        | "__atomic_fetch_nand"
        | "__atomic_add_fetch"
        | "__atomic_sub_fetch"
        | "__atomic_and_fetch"
        | "__atomic_or_fetch"
        | "__atomic_xor_fetch"
        | "__atomic_nand_fetch" => {
            // Return the base name by matching it to a static str
            match base {
                "__atomic_fetch_add" => Some("__atomic_fetch_add"),
                "__atomic_fetch_sub" => Some("__atomic_fetch_sub"),
                "__atomic_fetch_and" => Some("__atomic_fetch_and"),
                "__atomic_fetch_or" => Some("__atomic_fetch_or"),
                "__atomic_fetch_xor" => Some("__atomic_fetch_xor"),
                "__atomic_fetch_nand" => Some("__atomic_fetch_nand"),
                "__atomic_add_fetch" => Some("__atomic_add_fetch"),
                "__atomic_sub_fetch" => Some("__atomic_sub_fetch"),
                "__atomic_and_fetch" => Some("__atomic_and_fetch"),
                "__atomic_or_fetch" => Some("__atomic_or_fetch"),
                "__atomic_xor_fetch" => Some("__atomic_xor_fetch"),
                "__atomic_nand_fetch" => Some("__atomic_nand_fetch"),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Parameter types of the libc functions that `BuiltinKind::LibcAlias` maps to,
/// for the families whose signature contains a **`size_t`** (or another
/// parameter wider than `int`).
///
/// # Why this table exists
///
/// A `__builtin_memcmp(p, q, n)` call is legal in a translation unit that never
/// declares `memcmp`, so the lowering cannot rely on a user prototype to supply
/// the parameter conversions. Without them the `int`-typed length expression
/// `i + 1` was handed to the callee as a raw 32-bit value in a `size_t` slot:
///
///   * at `-O0` the value lives in a 4-byte stack slot, and the 8-byte argument
///     load pulled the adjacent slot in as the high half — `%rdx` became
///     `0x1_00000007` and `memcmp` ran off the end of both buffers
///     (gcc.c-torture/execute/`pr59229.c`);
///   * a *negative* `int` length was zero-extended where C requires the
///     `int -> size_t` conversion to sign-extend first.
///
/// # Scope
///
/// Only the entries whose parameters are genuinely wider than `int` are listed;
/// families where every parameter is `int`/`double`/pointer need no conversion
/// and are deliberately absent so the table stays reviewable. `Ptr` entries are
/// placeholders that keep the positional index aligned — the lowering skips
/// pointer slots (a pointer argument is never arithmetically converted).
///
/// `size_t` / `ssize_t` are pointer-width, so the table is target-dependent and
/// must be queried, not cached.
pub fn libc_alias_param_types(libc_name: &str) -> Option<Vec<crate::common::types::IrType>> {
    use crate::common::types::{target_ptr_size, IrType};
    // size_t: unsigned, pointer-width (LP64 -> U64, ILP32 -> U32).
    let size_t = if target_ptr_size() == 8 {
        IrType::U64
    } else {
        IrType::U32
    };
    let p = IrType::Ptr;
    let i32t = IrType::I32;
    let out: Vec<IrType> = match libc_name {
        // void *memcpy(void *, const void *, size_t)
        "memcpy" | "mempcpy" | "memmove" => vec![p, p, size_t],
        // void *memset(void *, int, size_t)
        "memset" => vec![p, i32t, size_t],
        // int memcmp(const void *, const void *, size_t)
        "memcmp" | "bcmp" => vec![p, p, size_t],
        // void *memchr(const void *, int, size_t)
        "memchr" | "rawmemchr" | "memrchr" => vec![p, i32t, size_t],
        // char *strncpy/strncat(char *, const char *, size_t)
        "strncpy" | "strncat" | "stpncpy" => vec![p, p, size_t],
        // int strncmp(const char *, const char *, size_t)
        "strncmp" | "strncasecmp" => vec![p, p, size_t],
        // size_t strnlen(const char *, size_t)
        "strnlen" => vec![p, size_t],
        // void *__builtin___memcpy_chk(void *, const void *, size_t, size_t)
        "__memcpy_chk" | "__mempcpy_chk" | "__memmove_chk" => vec![p, p, size_t, size_t],
        "__memset_chk" => vec![p, i32t, size_t, size_t],
        "__strncpy_chk" | "__strncat_chk" | "__stpncpy_chk" => vec![p, p, size_t, size_t],
        "__strcpy_chk" | "__strcat_chk" | "__stpcpy_chk" => vec![p, p, size_t],
        _ => return None,
    };
    Some(out)
}

#[cfg(test)]
mod libc_alias_param_type_tests {
    use super::libc_alias_param_types;
    use crate::common::types::{target_ptr_size, IrType};

    fn size_t() -> IrType {
        if target_ptr_size() == 8 {
            IrType::U64
        } else {
            IrType::U32
        }
    }

    #[test]
    fn size_taking_families_are_covered() {
        // The regression that motivated the table.
        assert_eq!(
            libc_alias_param_types("memcmp"),
            Some(vec![IrType::Ptr, IrType::Ptr, size_t()])
        );
        // memset's second parameter is `int`, NOT the char being stored.
        assert_eq!(
            libc_alias_param_types("memset"),
            Some(vec![IrType::Ptr, IrType::I32, size_t()])
        );
        assert_eq!(
            libc_alias_param_types("memcpy"),
            Some(vec![IrType::Ptr, IrType::Ptr, size_t()])
        );
        assert_eq!(
            libc_alias_param_types("strncmp"),
            Some(vec![IrType::Ptr, IrType::Ptr, size_t()])
        );
        assert_eq!(libc_alias_param_types("strnlen"), Some(vec![IrType::Ptr, size_t()]));
        // Both _chk length parameters are size_t.
        assert_eq!(
            libc_alias_param_types("__memcpy_chk"),
            Some(vec![IrType::Ptr, IrType::Ptr, size_t(), size_t()])
        );
    }

    #[test]
    fn size_t_is_unsigned_so_int_lengths_sign_extend_then_widen() {
        // A signed source converted to an UNSIGNED wider type must go through
        // a sign extension (C11 6.3.1.3): `(size_t)(int)-1 == SIZE_MAX`. The
        // table must therefore say `U64`, not `I64`; the cast emitter keys the
        // extension off the SOURCE signedness.
        let st = size_t();
        assert!(!st.is_signed(), "size_t must be unsigned");
        assert_eq!(st.size(), target_ptr_size(), "size_t must be pointer-width");
    }

    #[test]
    fn families_without_wide_parameters_are_absent() {
        // Everything here is int/double/pointer only: no conversion needed,
        // and listing them would be unreviewable noise.
        for n in ["strlen", "strcmp", "strcpy", "abs", "sqrt", "abort", "printf"] {
            assert_eq!(libc_alias_param_types(n), None, "{n} must not be listed");
        }
    }
}
