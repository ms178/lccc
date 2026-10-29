//! Predefined macros and target configuration.
//!
//! Contains the predefined macro tables (standard C, platform, GCC compat,
//! type limits, float characteristics, etc.) and architecture-specific
//! setup for aarch64 and riscv64.

use std::path::PathBuf;

use super::macro_defs::{MacroDef, MacroTable};
use super::pipeline::Preprocessor;

/// LCCC_SYSROOT-aware candidate mapping for built-in include discovery.
///
/// This legacy overlay is not GNU `--sysroot`: when `LCCC_SYSROOT` is set,
/// an existing prefixed candidate is preferred, but the original host path
/// remains a fallback. Only selected target-specific defaults use it. It can
/// help consume an unpacked multilib tree but does not isolate header search.
/// See the linker-side overlay in `backend/common.rs` for CRT/library lookup.
fn sysroot_candidate(path: &str) -> PathBuf {
    if let Ok(root) = std::env::var("LCCC_SYSROOT") {
        if !root.is_empty() {
            let prefixed = format!("{}{}", root.trim_end_matches('/'), path);
            if PathBuf::from(&prefixed).is_dir() {
                return PathBuf::from(prefixed);
            }
        }
    }
    PathBuf::from(path)
}

/// Object-like predefined macros, grouped by category; the order matches
/// GCC's own predefined-macro output so a `-dM -E` diff reads cleanly.
const PREDEFINED_OBJECT_MACROS: &[(&str, &str)] = &[
    // Standard C
    ("__STDC__", "1"),
    ("__STDC_VERSION__", "202311L"), // C23, GCC 16.2 default dialect
    ("__STDC_HOSTED__", "1"),
    // C23 #embed result constants (N3018). Defined unconditionally,
    // matching lccc's permissive-extension dialect policy (#embed is
    // accepted in every -std= mode, like GNU default modes accept
    // newer-dialect features as extensions).
    ("__STDC_EMBED_NOT_FOUND__", "0"),
    ("__STDC_EMBED_FOUND__", "1"),
    ("__STDC_EMBED_EMPTY__", "2"),
    // Platform
    ("__linux__", "1"),
    ("__linux", "1"),
    ("linux", "1"),
    ("__gnu_linux__", "1"),
    ("__unix__", "1"),
    ("__unix", "1"),
    ("unix", "1"),
    ("__LP64__", "1"),
    ("_LP64", "1"),
    // Default arch: x86_64 (overridden by set_target)
    ("__x86_64__", "1"),
    ("__x86_64", "1"),
    ("__amd64__", "1"),
    ("__amd64", "1"),
    // GCC compatibility claim. The compiler targets **GCC 16.2** semantics
    // (see docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md), so it claims 16.2:
    // a claim lower than the implemented language level makes headers take
    // the pre-16 code paths (`__GNUC_PREREQ`), while a claim higher than the
    // implementation would be an over-claim. Keep this in lockstep with
    // `-dumpversion`/`-dumpfullversion` (src/driver/cli.rs) and with the
    // `GCC_VERSION_*` constants below; `tests/regression/check_macro_parity.sh`
    // fails if they diverge.
    ("__GNUC__", "16"),
    ("__GNUC_MINOR__", "2"),
    ("__GNUC_PATCHLEVEL__", "0"),
    ("__VERSION__", "\"16.2.0\""),
    // C99 inline semantics: tells gnulib and other libraries that
    // plain `inline` provides an inline definition only (no external symbol).
    ("__GNUC_STDC_INLINE__", "1"),
    // --- GCC 16.2 predefined environment (measured 2026-10-27) -------
    // Spellings and values are byte-identical to `gcc-16 -dM -E` on
    // x86-64 so the macro-parity gate diffs to zero on these rows.
    ("__BIGGEST_ALIGNMENT__", "16"),
    ("__CHAR8_TYPE__", "unsigned char"),
    ("__FINITE_MATH_ONLY__", "0"),
    ("__FLOAT_WORD_ORDER__", "__ORDER_LITTLE_ENDIAN__"),
    ("__SIZEOF_FLOAT80__", "16"),
    ("__GXX_ABI_VERSION", "1021"),
    ("__REGISTER_PREFIX__", ""),
    ("__SIG_ATOMIC_TYPE__", "int"),
    ("__GCC_HAVE_DWARF2_CFI_ASM", "1"),
    ("__GCC_HAVE_SYNC_COMPARE_AND_SWAP_1", "1"),
    ("__GCC_HAVE_SYNC_COMPARE_AND_SWAP_2", "1"),
    ("__GCC_HAVE_SYNC_COMPARE_AND_SWAP_4", "1"),
    ("__GCC_HAVE_SYNC_COMPARE_AND_SWAP_8", "1"),
    ("__GCC_ATOMIC_TEST_AND_SET_TRUEVAL", "1"),
    ("__GCC_ATOMIC_BOOL_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_CHAR_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_CHAR8_T_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_CHAR16_T_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_CHAR32_T_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_SHORT_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_INT_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_LONG_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_LLONG_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_WCHAR_T_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_POINTER_LOCK_FREE", "2"),
    ("__STDC_IEC_559__", "1"),
    ("__STDC_IEC_559_COMPLEX__", "1"),
    ("__STDC_IEC_60559_BFP__", "201404L"),
    ("__STDC_IEC_60559_COMPLEX__", "201404L"),
    ("__STDC_ISO_10646__", "201706L"),
    ("__FLT_IS_IEC_60559__", "1"),
    ("__DBL_IS_IEC_60559__", "1"),
    ("__LDBL_IS_IEC_60559__", "1"),
    (
        "__FLT_NORM_MAX__",
        "3.40282346638528859811704183484516925e+38F",
    ),
    (
        "__DBL_NORM_MAX__",
        "((double)1.79769313486231570814527423731704357e+308L)",
    ),
    (
        "__LDBL_NORM_MAX__",
        "1.18973149535723176502126385303097021e+4932L",
    ),
    ("__INT8_MAX__", "0x7f"),
    ("__INT16_MAX__", "0x7fff"),
    ("__INT32_MAX__", "0x7fffffff"),
    ("__INT64_MAX__", "0x7fffffffffffffffL"),
    ("__INTMAX_MAX__", "0x7fffffffffffffffL"),
    ("__INTPTR_MAX__", "0x7fffffffffffffffL"),
    ("__UINT8_MAX__", "0xff"),
    ("__UINT16_MAX__", "0xffff"),
    ("__UINT32_MAX__", "0xffffffffU"),
    ("__UINT64_MAX__", "0xffffffffffffffffUL"),
    ("__UINTMAX_MAX__", "0xffffffffffffffffUL"),
    ("__UINTPTR_MAX__", "0xffffffffffffffffUL"),
    ("__INT_LEAST8_MAX__", "0x7f"),
    ("__INT_LEAST16_MAX__", "0x7fff"),
    ("__INT_LEAST32_MAX__", "0x7fffffff"),
    ("__INT_LEAST64_MAX__", "0x7fffffffffffffffL"),
    ("__UINT_LEAST8_MAX__", "0xff"),
    ("__UINT_LEAST16_MAX__", "0xffff"),
    ("__UINT_LEAST32_MAX__", "0xffffffffU"),
    ("__UINT_LEAST64_MAX__", "0xffffffffffffffffUL"),
    ("__INT_FAST8_MAX__", "0x7f"),
    ("__INT_FAST16_MAX__", "0x7fffffffffffffffL"),
    ("__INT_FAST32_MAX__", "0x7fffffffffffffffL"),
    ("__INT_FAST64_MAX__", "0x7fffffffffffffffL"),
    ("__UINT_FAST8_MAX__", "0xff"),
    ("__UINT_FAST16_MAX__", "0xffffffffffffffffUL"),
    ("__UINT_FAST32_MAX__", "0xffffffffffffffffUL"),
    ("__UINT_FAST64_MAX__", "0xffffffffffffffffUL"),
    // sizeof macros
    ("__SIZEOF_POINTER__", "8"),
    ("__SIZEOF_INT__", "4"),
    ("__SIZEOF_LONG__", "8"),
    ("__SIZEOF_LONG_LONG__", "8"),
    ("__SIZEOF_SHORT__", "2"),
    ("__SIZEOF_FLOAT__", "4"),
    ("__SIZEOF_DOUBLE__", "8"),
    ("__SIZEOF_SIZE_T__", "8"),
    ("__SIZEOF_PTRDIFF_T__", "8"),
    ("__SIZEOF_WCHAR_T__", "4"),
    ("__SIZEOF_INT128__", "16"),
    ("__SIZEOF_WINT_T__", "4"),
    // Byte order
    ("__BYTE_ORDER__", "__ORDER_LITTLE_ENDIAN__"),
    ("__ORDER_LITTLE_ENDIAN__", "1234"),
    ("__ORDER_BIG_ENDIAN__", "4321"),
    // Type limits
    ("__CHAR_BIT__", "8"),
    ("__INT_MAX__", "2147483647"),
    ("__LONG_MAX__", "9223372036854775807L"),
    ("__LONG_LONG_MAX__", "9223372036854775807LL"),
    ("__SCHAR_MAX__", "127"),
    ("__SHRT_MAX__", "32767"),
    ("__SIZE_MAX__", "18446744073709551615UL"),
    ("__PTRDIFF_MAX__", "9223372036854775807L"),
    ("__WCHAR_MAX__", "2147483647"),
    ("__WCHAR_MIN__", "(-2147483647-1)"),
    ("__WINT_MAX__", "4294967295U"),
    ("__WINT_MIN__", "0U"),
    ("__SIG_ATOMIC_MAX__", "2147483647"),
    ("__SIG_ATOMIC_MIN__", "(-2147483647-1)"),
    // Type names
    ("__SIZE_TYPE__", "long unsigned int"),
    ("__PTRDIFF_TYPE__", "long int"),
    ("__WCHAR_TYPE__", "int"),
    ("__WINT_TYPE__", "unsigned int"),
    ("__CHAR16_TYPE__", "short unsigned int"),
    ("__CHAR32_TYPE__", "unsigned int"),
    ("__INTMAX_TYPE__", "long int"),
    ("__UINTMAX_TYPE__", "long unsigned int"),
    ("__INT8_TYPE__", "signed char"),
    ("__INT16_TYPE__", "short int"),
    ("__INT32_TYPE__", "int"),
    ("__INT64_TYPE__", "long int"),
    ("__UINT8_TYPE__", "unsigned char"),
    ("__UINT16_TYPE__", "short unsigned int"),
    ("__UINT32_TYPE__", "unsigned int"),
    ("__UINT64_TYPE__", "long unsigned int"),
    ("__INTPTR_TYPE__", "long int"),
    ("__UINTPTR_TYPE__", "long unsigned int"),
    ("__INT_LEAST8_TYPE__", "signed char"),
    ("__INT_LEAST16_TYPE__", "short int"),
    ("__INT_LEAST32_TYPE__", "int"),
    ("__INT_LEAST64_TYPE__", "long int"),
    ("__UINT_LEAST8_TYPE__", "unsigned char"),
    ("__UINT_LEAST16_TYPE__", "short unsigned int"),
    ("__UINT_LEAST32_TYPE__", "unsigned int"),
    ("__UINT_LEAST64_TYPE__", "long unsigned int"),
    ("__INT_FAST8_TYPE__", "signed char"),
    ("__INT_FAST16_TYPE__", "long int"),
    ("__INT_FAST32_TYPE__", "long int"),
    ("__INT_FAST64_TYPE__", "long int"),
    ("__UINT_FAST8_TYPE__", "unsigned char"),
    ("__UINT_FAST16_TYPE__", "long unsigned int"),
    ("__UINT_FAST32_TYPE__", "long unsigned int"),
    ("__UINT_FAST64_TYPE__", "long unsigned int"),
    // FLT characteristics
    ("__FLT_MANT_DIG__", "24"),
    ("__FLT_DIG__", "6"),
    ("__FLT_MIN_EXP__", "(-125)"),
    ("__FLT_MIN_10_EXP__", "(-37)"),
    ("__FLT_MAX_EXP__", "128"),
    ("__FLT_MAX_10_EXP__", "38"),
    ("__FLT_MAX__", "3.40282346638528859811704183484516925e+38F"),
    ("__FLT_MIN__", "1.17549435082228750796873653722224568e-38F"),
    (
        "__FLT_EPSILON__",
        "1.19209289550781250000000000000000000e-7F",
    ),
    ("__FLT_RADIX__", "2"),
    // IR operations use their declared FP precision. In particular,
    // i686's x87 cache must round through F64 before forwarding.
    // GCC's real <float.h> expands FLT_EVAL_METHOD to these names.
    ("__FLT_EVAL_METHOD__", "0"),
    ("__FLT_EVAL_METHOD_TS_18661_3__", "0"),
    (
        "__FLT_DENORM_MIN__",
        "1.40129846432481707092372958328991613e-45F",
    ),
    // DBL characteristics
    ("__DBL_MANT_DIG__", "53"),
    ("__DBL_DIG__", "15"),
    ("__DBL_MIN_EXP__", "(-1021)"),
    ("__DBL_MIN_10_EXP__", "(-307)"),
    ("__DBL_MAX_EXP__", "1024"),
    ("__DBL_MAX_10_EXP__", "308"),
    ("__DBL_MAX__", "1.79769313486231570814527423731704357e+308"),
    ("__DBL_MIN__", "2.22507385850720138309023271733240406e-308"),
    (
        "__DBL_EPSILON__",
        "2.22044604925031308084726333618164062e-16",
    ),
    (
        "__DBL_DENORM_MIN__",
        "4.94065645841246544176568792868221372e-324",
    ),
    // LDBL characteristics
    ("__LDBL_MANT_DIG__", "64"),
    ("__LDBL_DIG__", "18"),
    ("__LDBL_MIN_EXP__", "(-16381)"),
    ("__LDBL_MIN_10_EXP__", "(-4931)"),
    ("__LDBL_MAX_EXP__", "16384"),
    ("__LDBL_MAX_10_EXP__", "4932"),
    (
        "__LDBL_MAX__",
        "1.18973149535723176502126385303097021e+4932L",
    ),
    (
        "__LDBL_MIN__",
        "3.36210314311209350626267781732175260e-4932L",
    ),
    (
        "__LDBL_EPSILON__",
        "1.08420217248550443400745280086994171e-19L",
    ),
    (
        "__LDBL_DENORM_MIN__",
        "3.64519953188247460252840593361941982e-4951L",
    ),
    ("__SIZEOF_LONG_DOUBLE__", "16"),
    // Float feature flags
    ("__FLT_HAS_INFINITY__", "1"),
    ("__FLT_HAS_QUIET_NAN__", "1"),
    ("__FLT_HAS_DENORM__", "1"),
    ("__DBL_HAS_INFINITY__", "1"),
    ("__DBL_HAS_QUIET_NAN__", "1"),
    ("__DBL_HAS_DENORM__", "1"),
    ("__LDBL_HAS_INFINITY__", "1"),
    ("__LDBL_HAS_QUIET_NAN__", "1"),
    ("__LDBL_HAS_DENORM__", "1"),
    ("__FLT_DECIMAL_DIG__", "9"),
    ("__DBL_DECIMAL_DIG__", "17"),
    ("__LDBL_DECIMAL_DIG__", "21"),
    ("__DECIMAL_DIG__", "21"),
    // _Float128 characteristics (IEEE binary128; GCC-compatible values).
    // glibc's float128_private.h keys off __FLT128_MANT_DIG__ etc.
    ("__SIZEOF_FLOAT128__", "16"),
    ("__FLT128_MANT_DIG__", "113"),
    ("__FLT128_DIG__", "33"),
    ("__FLT128_MIN_EXP__", "(-16381)"),
    ("__FLT128_MIN_10_EXP__", "(-4931)"),
    ("__FLT128_MAX_EXP__", "16384"),
    ("__FLT128_MAX_10_EXP__", "4932"),
    ("__FLT128_DECIMAL_DIG__", "36"),
    (
        "__FLT128_MAX__",
        "1.18973149535723176508575932662800702e+4932F128",
    ),
    (
        "__FLT128_NORM_MAX__",
        "1.18973149535723176508575932662800702e+4932F128",
    ),
    (
        "__FLT128_EPSILON__",
        "1.92592994438723585305597794258492732e-34F128",
    ),
    (
        "__FLT128_MIN__",
        "3.36210314311209350626267781732175260e-4932F128",
    ),
    (
        "__FLT128_DENORM_MIN__",
        "6.47517511943802511092443895822764655e-4966F128",
    ),
    (
        "__FLT128_TRUE_MIN__",
        "6.47517511943802511092443895822764655e-4966F128",
    ),
    ("__FLT128_HAS_INFINITY__", "1"),
    ("__FLT128_HAS_QUIET_NAN__", "1"),
    ("__FLT128_HAS_DENORM__", "1"),
    // ISO C23 _Float16 & GCC/Clang __bf16 characteristics (exact GCC 16.2 values).
    ("__FLT16_MANT_DIG__", "11"),
    ("__FLT16_DIG__", "3"),
    ("__FLT16_MIN_EXP__", "(-13)"),
    ("__FLT16_MIN_10_EXP__", "(-4)"),
    ("__FLT16_MAX_EXP__", "16"),
    ("__FLT16_MAX_10_EXP__", "4"),
    ("__FLT16_DECIMAL_DIG__", "5"),
    (
        "__FLT16_MAX__",
        "6.55040000000000000000000000000000000e+4F16",
    ),
    (
        "__FLT16_NORM_MAX__",
        "6.55040000000000000000000000000000000e+4F16",
    ),
    (
        "__FLT16_EPSILON__",
        "9.76562500000000000000000000000000000e-4F16",
    ),
    (
        "__FLT16_MIN__",
        "6.10351562500000000000000000000000000e-5F16",
    ),
    (
        "__FLT16_DENORM_MIN__",
        "5.96046447753906250000000000000000000e-8F16",
    ),
    ("__FLT16_HAS_INFINITY__", "1"),
    ("__FLT16_HAS_QUIET_NAN__", "1"),
    ("__FLT16_HAS_DENORM__", "1"),
    ("__FLT16_IS_IEC_60559__", "1"),
    ("__BFLT16_MANT_DIG__", "8"),
    ("__BFLT16_DIG__", "2"),
    ("__BFLT16_MIN_EXP__", "(-125)"),
    ("__BFLT16_MIN_10_EXP__", "(-37)"),
    ("__BFLT16_MAX_EXP__", "128"),
    ("__BFLT16_MAX_10_EXP__", "38"),
    ("__BFLT16_DECIMAL_DIG__", "4"),
    (
        "__BFLT16_MAX__",
        "3.38953138925153547590470800371487867e+38BF16",
    ),
    (
        "__BFLT16_NORM_MAX__",
        "3.38953138925153547590470800371487867e+38BF16",
    ),
    (
        "__BFLT16_EPSILON__",
        "7.81250000000000000000000000000000000e-3BF16",
    ),
    (
        "__BFLT16_MIN__",
        "1.17549435082228750796873653722224568e-38BF16",
    ),
    (
        "__BFLT16_DENORM_MIN__",
        "9.18354961579912115600575419704879436e-41BF16",
    ),
    ("__BFLT16_HAS_INFINITY__", "1"),
    ("__BFLT16_HAS_QUIET_NAN__", "1"),
    ("__BFLT16_HAS_DENORM__", "1"),
    ("__BFLT16_IS_IEC_60559__", "0"),
    // C23 decimal floating point (_Decimal32/64/128, IEEE 754-2008
    // BID). GCC defines the __DEC*_*__ family and __STDC_DEC_FP__
    // whenever DFP is enabled.
    ("__SIZEOF_DECIMAL32__", "4"),
    ("__SIZEOF_DECIMAL64__", "8"),
    ("__SIZEOF_DECIMAL128__", "16"),
    ("__DEC32_MANT_DIG__", "7"),
    ("__DEC32_MIN_EXP__", "(-94)"),
    ("__DEC32_MAX_EXP__", "97"),
    ("__DEC32_DIG__", "7"),
    ("__DEC32_MIN__", "1e-95DF"),
    ("__DEC32_MAX__", "9.999999e96DF"),
    ("__DEC32_EPSILON__", "1e-6DF"),
    ("__DEC32_SUBNORMAL_MIN__", "0.000001e-95DF"),
    ("__DEC64_MANT_DIG__", "16"),
    ("__DEC64_MIN_EXP__", "(-382)"),
    ("__DEC64_MAX_EXP__", "385"),
    ("__DEC64_DIG__", "16"),
    ("__DEC64_MIN__", "1e-383DD"),
    ("__DEC64_MAX__", "9.999999999999999e384DD"),
    ("__DEC64_EPSILON__", "1e-15DD"),
    ("__DEC64_SUBNORMAL_MIN__", "0.000000000000001e-383DD"),
    ("__DEC128_MANT_DIG__", "34"),
    ("__DEC128_MIN_EXP__", "(-6142)"),
    ("__DEC128_MAX_EXP__", "6145"),
    ("__DEC128_DIG__", "34"),
    ("__DEC128_MIN__", "1e-6143DL"),
    (
        "__DEC128_MAX__",
        "9.999999999999999999999999999999999e6144DL",
    ),
    ("__DEC128_EPSILON__", "1e-33DL"),
    (
        "__DEC128_SUBNORMAL_MIN__",
        "0.000000000000000000000000000000001e-6143DL",
    ),
    ("__DEC_EVAL_METHOD__", "2"),
    ("__DECIMAL_BID_FORMAT__", "1"),
    ("__STDC_DEC_FP__", "200704L"),
    ("__STDC_IEC_60559_TYPES_EXT__", "202311L"),
    // GCC extensions
    ("__GNUC_VA_LIST", "1"),
    ("__extension__", ""),
    // NOTE: GNU keyword aliases (__inline__, __volatile__, __asm__, __const__,
    // __restrict__, __signed__, __typeof__) are handled as keyword tokens in
    // the lexer (token.rs), not as macros, because GCC treats them as reserved
    // keywords immune to #define redefinition.
    // __alignof/__alignof__ are handled as keyword tokens (GnuAlignof)
    // in the lexer, not as macros - they return preferred alignment,
    // which differs from C11 _Alignof on i686.
    // Named address spaces (Linux kernel): __seg_gs/__seg_fs are handled
    // as keyword tokens in the lexer (token.rs), not as macros.
    // _Float128 / __float128 are REAL builtin types (lexer keyword
    // TokenKind::Float128, IEEE binary128, soft-float arithmetic via the
    // libgcc __addtf3 family) — NOT macros. Defining them as macros
    // would shadow the keyword and silently convert to 80-bit long
    // double. _Float32/_Float64/_Float32x/_Float64x share the formats
    // of standard types on x86-64, so they map via macros (GCC-compatible).
    ("_Float32", "float"),
    ("_Float64", "double"),
    ("_Float32x", "double"),
    ("_Float64x", "long double"),
    // MSVC integer type specifiers
    ("__int8", "char"),
    ("__int16", "short"),
    ("__int32", "int"),
    ("__int64", "long long"),
    // ELF ABI
    ("__USER_LABEL_PREFIX__", ""),
    // GNU C attribute macros (strip)
    ("__LEAF", ""),
    ("__LEAF_ATTR", ""),
    ("__wur", ""),
    // Date/time: __DATE__ and __TIME__ are defined dynamically below
    // so they reflect either SOURCE_DATE_EPOCH or the current compile time.
    // GCC atomic lock-free macros
    ("__GCC_ATOMIC_BOOL_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_CHAR_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_CHAR16_T_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_CHAR32_T_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_WCHAR_T_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_SHORT_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_INT_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_LONG_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_LLONG_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_POINTER_LOCK_FREE", "2"),
    // ELF
    ("__ELF__", "1"),
    // Note: __PIC__/__pic__ are conditionally defined via set_pic(),
    // not here, so they are only present when -fPIC is active.
    // __CET__ is NOT predefined here: GCC only defines it under an
    // active -fcf-protection (see set_cet(), called from the driver
    // with the parsed flag). Predefining it unconditionally broke
    // glibc --disable-cet (undefined _dl_cet_* at the ld.so link).
    // SSE/MMX feature macros: SSE2 is baseline for x86_64.
    // Removed for non-x86_64 targets in set_target().
    // Many projects (dr_libs, minimp3, stb_image, etc.) use #ifdef __SSE2__
    // to enable SIMD code paths.
    ("__SSE__", "1"),
    ("__SSE2__", "1"),
    ("__MMX__", "1"),
    ("__SSE_MATH__", "1"),
    ("__SSE2_MATH__", "1"),
    // Pragma support flags
    ("__PRAGMA_REDEFINE_EXTNAME", "1"),
];

/// Function-like predefined macros: (name, params, body).
const PREDEFINED_FUNC_MACROS: &[(&str, &[&str], &str)] = &[
    (
        "__builtin_offsetof",
        &["type", "member"],
        // The cast type is `OFFSETOF_SIZE_TYPEDEF` (common/types.rs).
        "((__lccc_offsetof_size_t)&((type *)0)->member)",
    ),
    // Integer-constant construction macros. GCC predefines these (they
    // are how <stdint.h> implements INT64_C without knowing the target's
    // type spelling); measured byte-identical to `gcc-16 -dM -E`.
    ("__INT8_C", &["c"], "c"),
    ("__INT16_C", &["c"], "c"),
    ("__INT32_C", &["c"], "c"),
    ("__INT64_C", &["c"], "c ## L"),
    ("__INTMAX_C", &["c"], "c ## L"),
    ("__UINT8_C", &["c"], "c"),
    ("__UINT16_C", &["c"], "c"),
    ("__UINT32_C", &["c"], "c ## U"),
    ("__UINT64_C", &["c"], "c ## UL"),
    ("__UINTMAX_C", &["c"], "c ## UL"),
    // __has_builtin, __has_attribute, __has_feature, __has_extension,
    // __has_include, and __has_include_next are NOT defined as macros.
    // They are handled as special preprocessor operators:
    // - #ifdef checks use is_defined() which special-cases them
    // - #if evaluation uses resolve_defined_in_expr() in expr_eval.rs
];

impl Preprocessor {
    /// Define standard predefined macros.
    ///
    /// Object-like macros are defined via a static table to keep this compact.
    /// Function-like macros (with parameters) are defined individually below.
    pub(super) fn define_predefined_macros(&mut self) {
        // The tables are module-level constants: the function is the
        // sequencing (table, then `-fshort-wchar` overrides, then date/time,
        // then the function-like set), not the data.
        for &(name, body) in PREDEFINED_OBJECT_MACROS {
            self.define_simple_macro(name, body);
        }

        // `-fshort-wchar` rewrites exactly the five `wchar_t` macros below and
        // nothing else (`__WINT_*` stay 32-bit, verified against GCC 14.2's
        // `-dM -E` output for both flag settings). The table above holds the
        // default spellings; re-defining here keeps the flag override in one
        // place instead of threading a runtime value through a `const` table.
        if crate::common::types::short_wchar() {
            self.define_simple_macro("__SIZEOF_WCHAR_T__", "2");
            self.define_simple_macro("__WCHAR_MAX__", "0xffff");
            self.define_simple_macro("__WCHAR_MIN__", "0");
            self.define_simple_macro("__WCHAR_WIDTH__", "16");
            self.define_simple_macro("__WCHAR_TYPE__", "short unsigned int");
        }

        let (date_macro, time_macro) = Self::current_date_time_macros();
        self.define_simple_macro("__DATE__", &date_macro);
        self.define_simple_macro("__TIME__", &time_macro);

        // Note: __builtin_expect is handled as a real builtin (not a macro)
        // to properly evaluate side effects in the second argument.
        for &(name, params, body) in PREDEFINED_FUNC_MACROS {
            self.macros.define(MacroDef {
                name: name.to_string(),
                is_function_like: true,
                params: params.iter().map(|s| s.to_string()).collect(),
                is_variadic: false,
                has_named_variadic: false,
                body: body.to_string(),
            });
        }
    }

    /// Helper to define a simple object-like macro.
    pub(super) fn define_simple_macro(&mut self, name: &str, body: &str) {
        self.macros.define(MacroDef {
            name: name.to_string(),
            is_function_like: false,
            params: Vec::new(),
            is_variadic: false,
            has_named_variadic: false,
            body: body.to_string(),
        });
    }

    /// Redefine one of the function-like predefined macros for a target whose
    /// integer model differs from the x86-64 table above.
    pub(super) fn define_function_macro(&mut self, name: &str, params: &[&str], body: &str) {
        self.macros.define(MacroDef {
            name: name.to_string(),
            is_function_like: true,
            params: params.iter().map(|p| p.to_string()).collect(),
            is_variadic: false,
            has_named_variadic: false,
            body: body.to_string(),
        });
    }

    /// Return C-standard `__DATE__` and `__TIME__` macro replacement lists.
    ///
    /// `SOURCE_DATE_EPOCH` is honored for reproducible builds.  Without it we
    /// use the current system time.  The conversion is UTC-only and uses safe
    /// `std` code to preserve CCC's zero-dependency / no-new-unsafe policy.
    fn current_date_time_macros() -> (String, String) {
        let secs = std::env::var("SOURCE_DATE_EPOCH")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
            });
        let (year, month, day, hour, min, sec) = Self::unix_utc_to_ymdhms(secs);
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let month_name = MONTHS[(month.saturating_sub(1) as usize).min(11)];
        (
            format!("\"{} {:>2} {}\"", month_name, day, year),
            format!("\"{:02}:{:02}:{:02}\"", hour, min, sec),
        )
    }

    /// Convert a Unix timestamp to UTC calendar components using Howard
    /// Hinnant's civil date algorithm.  Handles pre-epoch values too, which is
    /// useful for SOURCE_DATE_EPOCH test cases.
    fn unix_utc_to_ymdhms(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
        let days = secs.div_euclid(86_400);
        let sod = secs.rem_euclid(86_400);
        let hour = (sod / 3600) as u32;
        let min = ((sod % 3600) / 60) as u32;
        let sec = (sod % 60) as u32;

        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096).div_euclid(365);
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2).div_euclid(153);
        let d = doy - (153 * mp + 2).div_euclid(5) + 1;
        let m = mp + if mp < 10 { 3 } else { -9 };
        let year = y + if m <= 2 { 1 } else { 0 };
        (year, m as u32, d as u32, hour, min, sec)
    }

    /// Locate the bundled `include/` directory shipped alongside the binary.
    ///
    /// Walks up to 5 parent directories from the canonicalized executable path
    /// looking for an `include/` directory that contains `emmintrin.h`.
    /// Falls back to the compile-time `CARGO_MANIFEST_DIR/include` path.
    /// Returns `Some(path)` when a valid bundled include directory is found.
    pub fn bundled_include_dir() -> Option<PathBuf> {
        // Try to find the include dir relative to the running binary.
        if let Ok(exe) = std::env::current_exe() {
            if let Ok(canonical) = exe.canonicalize() {
                let mut dir = canonical.as_path().parent();
                for _ in 0..5 {
                    if let Some(d) = dir {
                        let candidate = d.join("include");
                        if candidate.join("emmintrin.h").is_file() {
                            return Some(candidate);
                        }
                        dir = d.parent();
                    } else {
                        break;
                    }
                }
            }
        }

        // Standard system package fallback on Arch/CachyOS
        let pkg_fallback = PathBuf::from("/usr/lib/lccc/include");
        if pkg_fallback.join("emmintrin.h").is_file() {
            return Some(pkg_fallback);
        }

        // Compile-time fallback: CARGO_MANIFEST_DIR/include
        let fallback = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include");
        if fallback.join("emmintrin.h").is_file() {
            return Some(fallback);
        }

        None
    }

    /// Get default system include paths (arch-neutral only).
    pub(super) fn default_system_include_paths() -> Vec<PathBuf> {
        let mut paths = Vec::with_capacity(8);
        // Bundled include directory takes priority over system GCC headers
        if let Some(bundled) = Self::bundled_include_dir() {
            paths.push(bundled);
        }

        // Dynamically find GCC include directories (highly robust and future-proof)
        for gcc_target in &[
            "x86_64-pc-linux-gnu",
            "x86_64-linux-gnu",
            "x86_64-redhat-linux",
            "i686-linux-gnu",
            "i686-redhat-linux",
            "aarch64-linux-gnu",
            "riscv64-linux-gnu",
        ] {
            let gcc_dir = format!("/usr/lib/gcc/{}", gcc_target);
            if let Ok(entries) = std::fs::read_dir(&gcc_dir) {
                for entry in entries.flatten() {
                    let candidate = entry.path().join("include");
                    if candidate.is_dir() {
                        paths.push(candidate);
                    }
                }
            }
        }

        // Only include arch-neutral paths here; arch-specific paths are added by set_target
        let candidates = [
            "/usr/local/include",
            "/usr/include/x86_64-pc-linux-gnu",
            "/usr/include/x86_64-linux-gnu",
            "/usr/include",
        ];
        for candidate in &candidates {
            let path = PathBuf::from(candidate);
            if path.is_dir() {
                paths.push(path);
            }
        }
        paths
    }

    /// Define `__STRICT_ANSI__` for strict ISO C modes (`-std=c99`, `-std=c11`, etc.).
    /// GCC defines this macro when `-std=cXX` (non-GNU) modes are used, and many
    /// headers (glibc's `<features.h>`, CPython's `pymacro.h`) check for it to
    /// gate GNU extensions like `typeof`.
    pub fn set_strict_ansi(&mut self, strict: bool) {
        if strict {
            self.define_simple_macro("__STRICT_ANSI__", "1");
        } else {
            self.macros.undefine("__STRICT_ANSI__");
        }
    }

    /// Set inline semantics mode: GNU89 vs C99.
    /// When `gnu89` is true, defines `__GNUC_GNU_INLINE__` and undefines `__GNUC_STDC_INLINE__`.
    /// When `gnu89` is false (default), `__GNUC_STDC_INLINE__` remains defined.
    /// GCC sets `__GNUC_GNU_INLINE__` with `-fgnu89-inline` or `-std=gnu89`,
    /// and `__GNUC_STDC_INLINE__` with `-std=gnu99` and later.
    pub fn set_gnu89_inline(&mut self, gnu89: bool) {
        if gnu89 {
            self.macros.undefine("__GNUC_STDC_INLINE__");
            self.define_simple_macro("__GNUC_GNU_INLINE__", "1");
        } else {
            self.macros.undefine("__GNUC_GNU_INLINE__");
            self.define_simple_macro("__GNUC_STDC_INLINE__", "1");
        }
    }

    /// Define/undefine `__EXCEPTIONS` based on `-fexceptions` (GCC-compatible).
    /// GCC defines `__EXCEPTIONS` when exceptions are enabled (-fexceptions for C,
    /// always for C++ unless -fno-exceptions). glibc's stdio-lock.h selects its
    /// lock-helper variant via `#ifdef __EXCEPTIONS`.
    pub fn set_exceptions(&mut self, enabled: bool) {
        if enabled {
            self.define_simple_macro("__EXCEPTIONS", "1");
        } else {
            self.macros.undefine("__EXCEPTIONS");
        }
    }

    /// Define __OPTIMIZE__ and __OPTIMIZE_SIZE__ based on the optimization level.
    ///
    /// GCC defines __OPTIMIZE__ for any optimization level >= 1 (-O, -O1, -O2, -O3, -Os, -Oz).
    /// GCC defines __OPTIMIZE_SIZE__ for -Os and -Oz.
    /// The Linux kernel relies on __OPTIMIZE__ for BUILD_BUG() and related compile-time
    /// assertion macros that expand to noreturn function calls when optimization is enabled.
    pub fn set_optimize(&mut self, optimize: bool, optimize_size: bool) {
        if optimize {
            self.define_simple_macro("__OPTIMIZE__", "1");
            self.macros.undefine("__NO_INLINE__");
        } else {
            self.macros.undefine("__OPTIMIZE__");
            // GCC predefines `__NO_INLINE__` exactly when it is not optimizing;
            // glibc's <string.h> and several allocators key `extern inline`
            // decisions off it.
            self.define_simple_macro("__NO_INLINE__", "1");
        }
        if optimize_size {
            self.define_simple_macro("__OPTIMIZE_SIZE__", "1");
        } else {
            self.macros.undefine("__OPTIMIZE_SIZE__");
        }
    }

    /// Define `__FAST_MATH__` when the command line explicitly permits FP
    /// reassociation. Ordinary optimization levels must not define it.
    pub fn set_fast_math(&mut self, enabled: bool) {
        if enabled {
            self.define_simple_macro("__FAST_MATH__", "1");
        } else {
            self.macros.undefine("__FAST_MATH__");
        }
    }

    /// Define or undefine __CET__ based on -fcf-protection.
    ///
    /// GCC defines `__CET__` ONLY when -fcf-protection is active (=1 branch,
    /// =2 return, =3 full); `-fcf-protection=none` or absence leaves it
    /// undefined. lccc used to predefine __CET__=3 unconditionally (for
    /// libffi's ENDBR_PRESENT), which dragged CET-only code into builds that
    /// disabled it: glibc --disable-cet compiles rtld WITHOUT dl-cet.c, but
    /// sysdeps/x86_64/sysdep.h saw __CET__ and made dl_main call
    /// _dl_cet_check/_dl_cet_setup_features — undefined symbols at the ld.so
    /// link.
    pub fn set_cet(&mut self, value: Option<&str>) {
        match value {
            Some(v) => self.define_simple_macro("__CET__", v),
            None => self.macros.undefine("__CET__"),
        }
    }

    /// Define or undefine GCC stack-protector feature macros (`__SSP*__`).
    pub fn set_ssp(&mut self, mac: Option<(&str, &str)>) {
        for name in [
            "__SSP__",
            "__SSP_ALL__",
            "__SSP_STRONG__",
            "__SSP_EXPLICIT__",
        ] {
            self.macros.undefine(name);
        }
        if let Some((name, val)) = mac {
            self.define_simple_macro(name, val);
        }
    }

    /// Define or undefine __PIC__/__pic__ based on whether PIC mode is active.
    /// GCC defines these to 1 for -fpic and 2 for -fPIC; we always use 2.
    /// When PIC is disabled (e.g. -fno-PIC), these must not be defined, as
    /// kernel code (RIP_REL_REF) checks `#ifndef __pic__` to decide whether
    /// to use RIP-relative inline asm for position-independent references.
    pub fn set_pic(&mut self, enabled: bool) {
        if enabled {
            self.define_simple_macro("__PIC__", "2");
            self.define_simple_macro("__pic__", "2");
        } else {
            self.macros.undefine("__PIC__");
            self.macros.undefine("__pic__");
        }
    }

    /// Define or undefine GCC-compatible PIE macros. PIE also defines PIC, but
    /// the driver calls the two setters separately so explicit `-fPIC` does
    /// not falsely advertise `__PIE__`.
    pub fn set_pie(&mut self, enabled: bool) {
        if enabled {
            self.define_simple_macro("__PIE__", "2");
            self.define_simple_macro("__pie__", "2");
        } else {
            self.macros.undefine("__PIE__");
            self.macros.undefine("__pie__");
        }
    }

    /// Define/undefine the baseline SSE/MMX macros to the invocation's
    /// resolved levels. The baseline macros are pre-defined in
    /// PREDEFINED_OBJECT_MACROS (the x86 default state: SSE/SSE2/MMX all on),
    /// so an OFF level must actively UNDEFINE its macro — the old
    /// `-mno-sse` early return left `__SSE2__` visible and the kernel's NAP
    /// governor compiled its `#ifdef __SSE2__` vector-extension code in a
    /// translation unit built with FPU_KILL_FLAGS, i.e. SSE codegen was both
    /// requested-off and semantically unavailable (idle context, no
    /// kernel_fpu_begin()).
    ///
    /// The levels are independent (GCC 16.2 / Clang 23.1.3, measured):
    /// i386 `-mno-sse2` keeps `__SSE__`; i386 `-march=i686` (and other
    /// pre-SSE `-march=` spellings) drops `__SSE__`, `__SSE2__` AND `__MMX__`;
    /// `-mno-sse` never touches `__MMX__` (MMX is independent of SSE).
    /// `__MMX_WITH_SSE__` is x86-64-only (neither oracle defines it for
    /// i386) and follows the SSE2 level, not the SSE1 level (GCC 16.2,
    /// measured: `-m32 -mno-sse2` keeps `__SSE__` but drops it). The `_MATH`
    /// pair follows lccc's scalar FP codegen (`apply_sse_macro_levels`).
    /// Must be called after `set_target()` (it reads the active arch).
    pub fn set_sse_macros(&mut self, sse_on: bool, sse2_on: bool, mmx_on: bool) {
        let is_x86_64 = self.macros.is_defined("__x86_64__");

        apply_sse_macro_levels(&mut self.macros, is_x86_64, sse_on, sse2_on, mmx_on);
    }

    /// Publish `__FXSR__` for the active x86 target (the driver resolves
    /// the level: always on x86-64, the `-march` row's Fxsr bit on i686;
    /// measured against GCC 16.2, which ignores -mno-sse/-mno-mmx/
    /// -mgeneral-regs-only for this macro). No-op on non-x86 targets.
    pub fn set_fxsr_macro(&mut self, on: bool) {
        let is_x86 = self.macros.is_defined("__x86_64__") || self.macros.is_defined("__i386__");
        if !is_x86 {
            return;
        }
        if on {
            self.define_simple_macro("__FXSR__", "1");
        } else {
            self.macros.undefine("__FXSR__");
        }
    }

    /// Define extended SIMD feature macros (__SSE3__, __AVX__, __AVX2__, etc.)
    /// when the corresponding -msse3, -mavx, -mavx2 flags are passed.
    /// Projects like blosc use #ifdef __AVX2__ to select optimized code paths.
    /// Must be called after set_sse_macros().
    pub fn set_extended_simd_macros(
        &mut self,
        sse3: bool,
        ssse3: bool,
        sse4_1: bool,
        sse4_2: bool,
        avx: bool,
        avx2: bool,
        aes: bool,
        pclmul: bool,
        f16c: bool,
        fma: bool,
        bmi: bool,
        bmi2: bool,
        lzcnt: bool,
        popcnt: bool,
        movbe: bool,
        rdrnd: bool,
        avx512f: bool,
        avx512cd: bool,
        avx512dq: bool,
        avx512bw: bool,
        avx512vl: bool,
        avx512ifma: bool,
        avx512vbmi: bool,
        avx512vbmi2: bool,
        avx512vnni: bool,
        avx512bitalg: bool,
        avx512vpopcntdq: bool,
        avx512bf16: bool,
        avx512fp16: bool,
        avx512er: bool,
        avx512pf: bool,
        avx512vp2intersect: bool,
        avx512bmm: bool,
        avxvnni: bool,
        avxifma: bool,
        avxneconvert: bool,
        avx10_1: bool,
        avx10_2: bool,
        gfni: bool,
        vaes: bool,
        vpclmulqdq: bool,
        avxvnniint8: bool,
        avxvnniint16: bool,
        sha512: bool,
        sm3: bool,
        sm4: bool,
        movrs: bool,
        amx_tile: bool,
        amx_int8: bool,
        amx_bf16: bool,
        cmpccxadd: bool,
        apxf: bool,
    ) {
        // Only define SSE/AVX macros for x86 targets.
        let is_x86 = self.macros.is_defined("__x86_64__") || self.macros.is_defined("__i386__");
        if !is_x86 {
            return;
        }
        if sse3 {
            self.define_simple_macro("__SSE3__", "1");
        }
        if ssse3 {
            self.define_simple_macro("__SSSE3__", "1");
        }
        if sse4_1 {
            self.define_simple_macro("__SSE4_1__", "1");
        }
        if sse4_2 {
            self.define_simple_macro("__SSE4_2__", "1");
        }
        if avx {
            self.define_simple_macro("__AVX__", "1");
        }
        if avx2 {
            self.define_simple_macro("__AVX2__", "1");
        }
        if aes {
            self.define_simple_macro("__AES__", "1");
        }
        if pclmul {
            self.define_simple_macro("__PCLMUL__", "1");
        }
        if f16c {
            self.define_simple_macro("__F16C__", "1");
        }
        if fma {
            self.define_simple_macro("__FMA__", "1");
        }
        if bmi {
            self.define_simple_macro("__BMI__", "1");
        }
        if bmi2 {
            self.define_simple_macro("__BMI2__", "1");
        }
        if lzcnt {
            self.define_simple_macro("__LZCNT__", "1");
        }
        // GCC defines __POPCNT__ under -mpopcnt (the codegen lowering is
        // resolved-popcnt-gated; the macro must mirror it so #ifdef-guarded
        // _mm_popcnt_* paths agree with the scalar contract).
        if popcnt {
            self.define_simple_macro("__POPCNT__", "1");
        }
        if movbe {
            self.define_simple_macro("__MOVBE__", "1");
        }
        if rdrnd {
            self.define_simple_macro("__RDRND__", "1");
        }
        // AVX-512 family macros.
        if avx512f {
            self.define_simple_macro("__AVX512F__", "1");
        }
        if avx512cd {
            self.define_simple_macro("__AVX512CD__", "1");
        }
        if avx512dq {
            self.define_simple_macro("__AVX512DQ__", "1");
        }
        if avx512bw {
            self.define_simple_macro("__AVX512BW__", "1");
        }
        if avx512vl {
            self.define_simple_macro("__AVX512VL__", "1");
        }
        if avx512ifma {
            self.define_simple_macro("__AVX512IFMA__", "1");
        }
        if avx512vbmi {
            self.define_simple_macro("__AVX512VBMI__", "1");
        }
        if avx512vbmi2 {
            self.define_simple_macro("__AVX512VBMI2__", "1");
        }
        if avx512vnni {
            self.define_simple_macro("__AVX512VNNI__", "1");
        }
        if avx512bitalg {
            self.define_simple_macro("__AVX512BITALG__", "1");
        }
        if avx512vpopcntdq {
            self.define_simple_macro("__AVX512VPOPCNTDQ__", "1");
        }
        if avx512bf16 {
            self.define_simple_macro("__AVX512BF16__", "1");
        }
        if avx512fp16 {
            self.define_simple_macro("__AVX512FP16__", "1");
        }
        if avx512er {
            self.define_simple_macro("__AVX512ER__", "1");
        }
        if avx512pf {
            self.define_simple_macro("__AVX512PF__", "1");
        }
        if avx512vp2intersect {
            self.define_simple_macro("__AVX512VP2INTERSECT__", "1");
        }
        if avx512bmm {
            self.define_simple_macro("__AVX512BMM__", "1");
        }
        // AVX-VNNI / AVX10 / GFNI / VAES / VPCLMULQDQ macros.
        if avxvnni {
            self.define_simple_macro("__AVXVNNI__", "1");
        }
        if avxifma {
            self.define_simple_macro("__AVXIFMA__", "1");
        }
        if avxneconvert {
            self.define_simple_macro("__AVXNECONVERT__", "1");
        }
        if avx10_1 {
            self.define_simple_macro("__AVX10_1__", "1");
        }
        if avx10_2 {
            self.define_simple_macro("__AVX10_2__", "1");
        }
        if gfni {
            self.define_simple_macro("__GFNI__", "1");
        }
        if vaes {
            self.define_simple_macro("__VAES__", "1");
        }
        if vpclmulqdq {
            self.define_simple_macro("__VPCLMULQDQ__", "1");
        }
        if avxvnniint8 {
            self.define_simple_macro("__AVXVNNIINT8__", "1");
        }
        if avxvnniint16 {
            self.define_simple_macro("__AVXVNNIINT16__", "1");
        }
        if sha512 {
            self.define_simple_macro("__SHA512__", "1");
        }
        if sm3 {
            self.define_simple_macro("__SM3__", "1");
        }
        if sm4 {
            self.define_simple_macro("__SM4__", "1");
        }
        if movrs {
            self.define_simple_macro("__MOVRS__", "1");
        }
        if amx_tile {
            self.define_simple_macro("__AMX_TILE__", "1");
        }
        if amx_int8 {
            self.define_simple_macro("__AMX_INT8__", "1");
        }
        if amx_bf16 {
            self.define_simple_macro("__AMX_BF16__", "1");
        }
        if cmpccxadd {
            self.define_simple_macro("__CMPCCXADD__", "1");
        }
        if apxf {
            self.define_simple_macro("__APX_F__", "1");
        }
    }

    /// Set the target architecture, updating predefined macros and include paths.
    fn undefine_x86_baseline_macros(&mut self) {
        for m in [
            "__x86_64__",
            "__x86_64",
            "__amd64__",
            "__amd64",
            "__CET__",
            "__SSE__",
            "__SSE2__",
            "__MMX__",
            "__SSE_MATH__",
            "__SSE2_MATH__",
        ] {
            self.macros.undefine(m);
        }
    }

    pub fn set_target(&mut self, target: &str) {
        self.invalidate_include_search();
        match target {
            "aarch64" => {
                self.undefine_x86_baseline_macros();
                // Define aarch64 macros
                self.define_simple_macro("__aarch64__", "1");
                self.define_simple_macro("__ARM_64BIT_STATE", "1");
                self.define_simple_macro("__ARM_ARCH", "8");
                self.define_simple_macro("__ARM_ARCH_8A", "1");
                self.define_simple_macro("__ARM_ARCH_ISA_A64", "1");
                self.define_simple_macro("__ARM_ARCH_PROFILE", "65"); // 'A'
                // Floating-point and SIMD
                self.define_simple_macro("__ARM_FP", "14"); // 0b1110: half+single+double precision
                self.define_simple_macro("__ARM_NEON", "1");
                self.define_simple_macro("__ARM_FP16_ARGS", "1");
                self.define_simple_macro("__ARM_FP16_FORMAT_IEEE", "1");
                // ABI
                self.define_simple_macro("__ARM_PCS_AAPCS64", "1");
                self.define_simple_macro("__ARM_SIZEOF_MINIMAL_ENUM", "4");
                self.define_simple_macro("__ARM_SIZEOF_WCHAR_T", "4");
                // Features
                self.define_simple_macro("__ARM_FEATURE_CLZ", "1");
                self.define_simple_macro("__ARM_FEATURE_FMA", "1");
                self.define_simple_macro("__ARM_FEATURE_IDIV", "1");
                self.define_simple_macro("__ARM_FEATURE_NUMERIC_MAXMIN", "1");
                self.define_simple_macro("__ARM_FEATURE_UNALIGNED", "1");
                self.define_simple_macro("__ARM_ALIGN_MAX_PWR", "28");
                self.define_simple_macro("__ARM_ALIGN_MAX_STACK_PWR", "16");
                self.define_simple_macro("__AARCH64EL__", "1");
                self.define_simple_macro("__AARCH64_CMODEL_SMALL__", "1");
                // Replace x86 include paths with aarch64 paths
                self.system_include_paths.retain(|p| {
                    let s = p.to_string_lossy();
                    !s.contains("x86_64")
                });
                let aarch64_paths = [
                    "/usr/lib/gcc/aarch64-linux-gnu/16/include",
                    "/usr/lib/gcc/aarch64-linux-gnu/15/include",
                    "/usr/lib/gcc/aarch64-linux-gnu/14/include",
                    "/usr/lib/gcc/aarch64-linux-gnu/13/include",
                    "/usr/lib/gcc/aarch64-linux-gnu/12/include",
                    "/usr/lib/gcc/aarch64-linux-gnu/11/include",
                    "/usr/lib/gcc-cross/aarch64-linux-gnu/16/include",
                    "/usr/lib/gcc-cross/aarch64-linux-gnu/15/include",
                    "/usr/lib/gcc-cross/aarch64-linux-gnu/14/include",
                    "/usr/lib/gcc-cross/aarch64-linux-gnu/13/include",
                    "/usr/lib/gcc-cross/aarch64-linux-gnu/12/include",
                    "/usr/lib/gcc-cross/aarch64-linux-gnu/11/include",
                    "/usr/aarch64-linux-gnu/include",
                    "/usr/include/aarch64-linux-gnu",
                ];
                self.insert_arch_paths_after_bundled(&aarch64_paths);
                // AArch64 uses IEEE 754 binary128 for long double (not x87 80-bit)
                self.override_ldbl_binary128();
            }
            "riscv64" => {
                // Remove x86 macros
                self.macros.undefine("__x86_64__");
                self.macros.undefine("__x86_64");
                self.macros.undefine("__amd64__");
                self.macros.undefine("__amd64");
                self.macros.undefine("__CET__");
                self.macros.undefine("__SSE__");
                self.macros.undefine("__SSE2__");
                self.macros.undefine("__MMX__");
                self.macros.undefine("__SSE_MATH__");
                self.macros.undefine("__SSE2_MATH__");
                // Define riscv64 macros
                self.define_simple_macro("__riscv", "1");
                self.define_simple_macro("__riscv_xlen", "64");
                // Floating-point: double-precision (D extension)
                self.define_simple_macro("__riscv_flen", "64");
                self.define_simple_macro("__riscv_float_abi_double", "1");
                self.define_simple_macro("__riscv_fdiv", "1");
                self.define_simple_macro("__riscv_fsqrt", "1");
                // ISA extensions (RV64GC = IMAFDCZicsr_Zifencei)
                self.define_simple_macro("__riscv_atomic", "1");
                self.define_simple_macro("__riscv_mul", "1");
                self.define_simple_macro("__riscv_muldiv", "1");
                self.define_simple_macro("__riscv_div", "1");
                self.define_simple_macro("__riscv_compressed", "1");
                // Extension version macros (XYYYZZZZ format: e.g. 2001000 = v2.1.0)
                self.define_simple_macro("__riscv_i", "2001000");
                self.define_simple_macro("__riscv_m", "2000000");
                self.define_simple_macro("__riscv_a", "2001000");
                self.define_simple_macro("__riscv_f", "2002000");
                self.define_simple_macro("__riscv_d", "2002000");
                self.define_simple_macro("__riscv_c", "2000000");
                self.define_simple_macro("__riscv_zicsr", "2000000");
                self.define_simple_macro("__riscv_zifencei", "2000000");
                self.define_simple_macro("__riscv_arch_test", "1");
                self.define_simple_macro("__riscv_cmodel_medany", "1");
                // Replace x86 include paths with riscv64 paths
                self.system_include_paths.retain(|p| {
                    let s = p.to_string_lossy();
                    !s.contains("x86_64")
                });
                let riscv_paths = [
                    "/usr/lib/gcc/riscv64-linux-gnu/16/include",
                    "/usr/lib/gcc/riscv64-linux-gnu/15/include",
                    "/usr/lib/gcc/riscv64-linux-gnu/14/include",
                    "/usr/lib/gcc/riscv64-linux-gnu/13/include",
                    "/usr/lib/gcc/riscv64-linux-gnu/12/include",
                    "/usr/lib/gcc/riscv64-linux-gnu/11/include",
                    "/usr/lib/gcc-cross/riscv64-linux-gnu/16/include",
                    "/usr/lib/gcc-cross/riscv64-linux-gnu/15/include",
                    "/usr/lib/gcc-cross/riscv64-linux-gnu/14/include",
                    "/usr/lib/gcc-cross/riscv64-linux-gnu/13/include",
                    "/usr/lib/gcc-cross/riscv64-linux-gnu/12/include",
                    "/usr/lib/gcc-cross/riscv64-linux-gnu/11/include",
                    "/usr/riscv64-linux-gnu/include",
                    "/usr/include/riscv64-linux-gnu",
                ];
                self.insert_arch_paths_after_bundled(&riscv_paths);
                // RISC-V uses IEEE 754 binary128 for long double (not x87 80-bit)
                self.override_ldbl_binary128();
            }
            "i686" | "i386" => self.apply_i686_target(),
            _ => {
                // x86_64 is already the default
            }
        }
    }

    /// Apply the i686/i386 (ILP32) target overrides.
    ///
    /// Split out of `set_target`, whose arm had grown past the file's
    /// function-length budget; the sequence is order-sensitive (the x86-64
    /// spellings are undone or replaced one by one), so it stays together.
    fn apply_i686_target(&mut self) {
        // Remove x86-64 macros (keep x86 general macros)
        self.macros.undefine("__x86_64__");
        self.macros.undefine("__x86_64");
        self.macros.undefine("__amd64__");
        self.macros.undefine("__amd64");
        self.macros.undefine("__LP64__");
        self.macros.undefine("_LP64");
        self.macros.undefine("__SIZEOF_INT128__");
        // i686 baseline does not include SSE (GCC only enables SSE with
        // -march=pentium4 or higher). Remove SSE macros to match GCC.
        self.macros.undefine("__SSE__");
        self.macros.undefine("__SSE2__");
        self.macros.undefine("__MMX__");
        self.macros.undefine("__SSE_MATH__");
        self.macros.undefine("__SSE2_MATH__");
        // i686-linux-gnu-gcc -m32 does NOT define __CET__ (CET is
        // disabled by -m32).  We must match this because .S assembly
        // files are assembled by GCC, and if the C code expects
        // ENDBR_PRESENT (44-byte trampolines) but the assembly
        // produces non-ENDBR trampolines (40 bytes), the mismatch
        // causes crashes (e.g. libffi closures).
        self.macros.undefine("__CET__");
        // Define i686/i386 macros
        self.define_simple_macro("__i386__", "1");
        self.define_simple_macro("__i386", "1");
        self.define_simple_macro("i386", "1");
        self.define_simple_macro("__i686__", "1");
        self.define_simple_macro("__i686", "1");
        self.define_simple_macro("__ILP32__", "1");
        self.define_simple_macro("_ILP32", "1");
        // ILP32 data model: pointer/long/size_t are 4 bytes
        self.define_simple_macro("__SIZEOF_POINTER__", "4");
        self.define_simple_macro("__SIZEOF_LONG__", "4");
        self.define_simple_macro("__SIZEOF_SIZE_T__", "4");
        self.define_simple_macro("__SIZEOF_PTRDIFF_T__", "4");
        // Long double is 12 bytes on i686 (80-bit x87 + 4 bytes padding)
        self.define_simple_macro("__SIZEOF_LONG_DOUBLE__", "12");
        // The 80-bit x87 format is 12 bytes (10 payload + 2 padding) on
        // i386; the x86-64 table's 16 is the 128-bit format.
        self.define_simple_macro("__SIZEOF_FLOAT80__", "12");
        // ILP32 width and suffix overrides. The x86-64 table spells every
        // 64-bit limit with a single `L` and defines the pointer-width
        // limits from the LP64 model; on ILP32 `long` is 32 bits, so
        // `0x7fffffffffffffffL` would name a 32-bit constant and
        // `__INT64_C(x)` would build a `long` that cannot hold it.
        // Values and spellings measured against `gcc-16 -m32 -dM -E`
        // (2026-10-28).
        self.define_simple_macro("__INTPTR_MAX__", "0x7fffffff");
        self.define_simple_macro("__UINTPTR_MAX__", "0xffffffffU");
        self.define_simple_macro("__INT_FAST16_MAX__", "0x7fffffff");
        self.define_simple_macro("__UINT_FAST16_MAX__", "0xffffffffU");
        self.define_simple_macro("__INT_FAST32_MAX__", "0x7fffffff");
        self.define_simple_macro("__UINT_FAST32_MAX__", "0xffffffffU");
        // `int_fast32_t` is `unsigned int` here, not `unsigned long`.
        self.define_simple_macro("__UINT_FAST32_TYPE__", "unsigned int");
        self.define_simple_macro("__INT64_MAX__", "0x7fffffffffffffffLL");
        self.define_simple_macro("__UINT64_MAX__", "0xffffffffffffffffULL");
        self.define_simple_macro("__INT_LEAST64_MAX__", "0x7fffffffffffffffLL");
        self.define_simple_macro("__UINT_LEAST64_MAX__", "0xffffffffffffffffULL");
        self.define_simple_macro("__INT_FAST64_MAX__", "0x7fffffffffffffffLL");
        self.define_simple_macro("__UINT_FAST64_MAX__", "0xffffffffffffffffULL");
        self.define_simple_macro("__INTMAX_MAX__", "0x7fffffffffffffffLL");
        self.define_simple_macro("__UINTMAX_MAX__", "0xffffffffffffffffULL");
        self.define_function_macro("__INT64_C", &["c"], "c ## LL");
        self.define_function_macro("__UINT64_C", &["c"], "c ## ULL");
        self.define_function_macro("__INTMAX_C", &["c"], "c ## LL");
        self.define_function_macro("__UINTMAX_C", &["c"], "c ## ULL");
        // Type limits for ILP32
        self.define_simple_macro("__LONG_MAX__", "2147483647L");
        self.define_simple_macro("__SIZE_MAX__", "4294967295U");
        self.define_simple_macro("__PTRDIFF_MAX__", "2147483647");
        // Override <limits.h> macros for ILP32 (long is 32-bit)
        self.define_simple_macro("LONG_MIN", "(-2147483647L-1L)");
        self.define_simple_macro("LONG_MAX", "2147483647L");
        self.define_simple_macro("ULONG_MAX", "4294967295UL");
        // Override <stdint.h> macros for ILP32 (pointer/size_t are 32-bit)
        self.define_simple_macro("INTPTR_MIN", "(-2147483647-1)");
        self.define_simple_macro("INTPTR_MAX", "2147483647");
        self.define_simple_macro("UINTPTR_MAX", "4294967295U");
        self.define_simple_macro("SIZE_MAX", "4294967295U");
        self.define_simple_macro("PTRDIFF_MIN", "(-2147483647-1)");
        self.define_simple_macro("PTRDIFF_MAX", "2147483647");
        // Type names for ILP32 (long is 32-bit, so many types change)
        self.define_simple_macro("__SIZE_TYPE__", "unsigned int");
        self.define_simple_macro("__PTRDIFF_TYPE__", "int");
        self.define_simple_macro("__INTMAX_TYPE__", "long long int");
        self.define_simple_macro("__UINTMAX_TYPE__", "long long unsigned int");
        self.define_simple_macro("__INT64_TYPE__", "long long int");
        self.define_simple_macro("__UINT64_TYPE__", "long long unsigned int");
        self.define_simple_macro("__INTPTR_TYPE__", "int");
        self.define_simple_macro("__UINTPTR_TYPE__", "unsigned int");
        self.define_simple_macro("__INT_LEAST64_TYPE__", "long long int");
        self.define_simple_macro("__UINT_LEAST64_TYPE__", "long long unsigned int");
        self.define_simple_macro("__INT_FAST16_TYPE__", "int");
        self.define_simple_macro("__INT_FAST32_TYPE__", "int");
        self.define_simple_macro("__INT_FAST64_TYPE__", "long long int");
        self.define_simple_macro("__UINT_FAST16_TYPE__", "unsigned int");
        self.define_simple_macro("__UINT_FAST64_TYPE__", "long long unsigned int");
        // Replace x86-64 include paths with i686 paths. The GCC
        // *header* directory must survive this filter: on multilib
        // layouts (Debian/Ubuntu gcc-multilib, which is also what
        // real `gcc -m32` uses) /usr/lib/gcc/x86_64-linux-gnu/<v>/
        // include serves BOTH word sizes — stddef.h, stdarg.h,
        // stdatomic.h and the -march-guarded intrinsic headers are
        // word-size-agnostic or macro-guarded. Dropping it left -m32
        // compiles without stddef.h/stdarg.h entirely. Only the
        // x86-64 *C library* dirs (glibc's /usr/include/x86_64-
        // linux-gnu bits) are x86-64-specific and get filtered.
        self.system_include_paths.retain(|p| {
            let s = p.to_string_lossy();
            if s.contains("/usr/lib/gcc/x86_64-") {
                return true;
            }
            !s.contains("x86_64")
        });
        let i686_paths = [
            "/usr/lib/gcc-cross/i686-linux-gnu/16/include",
            "/usr/lib/gcc-cross/i686-linux-gnu/15/include",
            "/usr/lib/gcc-cross/i686-linux-gnu/14/include",
            "/usr/lib/gcc-cross/i686-linux-gnu/13/include",
            "/usr/lib/gcc-cross/i686-linux-gnu/12/include",
            "/usr/lib/gcc-cross/i686-linux-gnu/11/include",
            "/usr/lib/gcc/i686-linux-gnu/16/include",
            "/usr/lib/gcc/i686-linux-gnu/15/include",
            "/usr/lib/gcc/i686-linux-gnu/14/include",
            "/usr/lib/gcc/i686-linux-gnu/13/include",
            "/usr/lib/gcc/i686-linux-gnu/12/include",
            "/usr/lib/gcc/i686-linux-gnu/11/include",
            "/usr/i686-linux-gnu/include",
            "/usr/include/i386-linux-gnu",
        ];
        let mapped: Vec<String> = i686_paths
            .iter()
            .map(|p| sysroot_candidate(p).to_string_lossy().into_owned())
            .collect();
        let mapped_refs: Vec<&str> = mapped.iter().map(String::as_str).collect();
        self.insert_arch_paths_after_bundled(&mapped_refs);
        // GNU --sysroot remaps the *generic* C-library include dirs
        // too, not only the arch-specific ones.  Without this the
        // unprefixed host /usr/include still answers first for
        // <stdio.h> & friends whenever the arch dirs don't carry the
        // header themselves, so a rootless multilib sysroot on a host
        // whose glibc headers are NOT single-tree i386-compatible
        // (Debian multiarch layouts, cross-only CI images) resolves
        // x86-64-flavored headers for i686 compiles and fails with
        // missing bits/*.h.  Mapped generic dirs are appended after
        // the arch dirs (still ahead of every unprefixed fallback);
        // with no LCCC_SYSROOT nothing changes at all.
        if std::env::var("LCCC_SYSROOT").is_ok_and(|r| !r.is_empty()) {
            const GENERIC_LIB_DIRS: [&str; 2] = ["/usr/local/include", "/usr/include"];
            let mapped_generic: Vec<String> = GENERIC_LIB_DIRS
                .iter()
                .map(|p| sysroot_candidate(p).to_string_lossy().into_owned())
                .collect();
            let generic_refs: Vec<&str> = mapped_generic.iter().map(String::as_str).collect();
            self.insert_arch_paths_after_bundled(&generic_refs);
        }
        // Override width macros for ILP32 (pointer/long/size_t/ptrdiff are 32-bit)
        self.define_simple_macro("__LONG_WIDTH__", "32");
        self.define_simple_macro("__PTRDIFF_WIDTH__", "32");
        self.define_simple_macro("__SIZE_WIDTH__", "32");
        self.define_simple_macro("__INTPTR_WIDTH__", "32");
        self.define_simple_macro("__INT_FAST16_WIDTH__", "32");
        self.define_simple_macro("__INT_FAST32_WIDTH__", "32");
        // i686 uses the same x87 80-bit long double format as x86-64
        // (LDBL macros are already set correctly), but sizeof differs (12 vs 16)

        // Swap the x86-64 multiarch include dir for the i386 one.
        // Debian puts the 32-bit `bits/` headers under
        // /usr/include/i386-linux-gnu, and the arm above deliberately
        // ships only arch-neutral paths, so without this every 32-bit
        // compile fails at `#include <stdio.h>` with
        // "bits/libc-header-start.h: No such file or directory" -- the
        // x86-64 dir would be searched first and is wrong for the ABI.
        // Order is preserved: the i386 dir takes the slot the x86-64
        // one occupied, so /usr/local/include still wins and /usr/include
        // is still the fallback.
        // Exact match on the /usr/include multiarch dir only.  A
        // substring test also caught /usr/lib/gcc/x86_64-linux-gnu/<ver>
        // /include, which holds GCC's own freestanding headers
        // (stddef.h, stdarg.h) and is *shared* between -m32 and -m64 --
        // replacing it broke every 32-bit compile on <stdio.h>.
        let mut swapped = false;
        for slot in self.system_include_paths.iter_mut() {
            if slot == std::path::Path::new("/usr/include/x86_64-linux-gnu") {
                *slot = sysroot_candidate("/usr/include/i386-linux-gnu");
                swapped = true;
                break;
            }
        }
        if !swapped {
            // No x86-64 multiarch dir was present (e.g. a bare
            // /usr/include install); insert the i386 dir ahead of
            // /usr/include so the 32-bit headers win.
            let at = self
                .system_include_paths
                .iter()
                .position(|p| p == std::path::Path::new("/usr/include"))
                .unwrap_or(self.system_include_paths.len());
            self.system_include_paths
                .insert(at, sysroot_candidate("/usr/include/i386-linux-gnu"));
        }
    }

    /// Insert architecture-specific include paths, keeping the bundled include
    /// directory first so our simplified SSE/intrinsic headers take priority over
    /// the system GCC cross-compiler headers (which use unsupported builtins).
    fn insert_arch_paths_after_bundled(&mut self, arch_paths: &[&str]) {
        // Find the index after the bundled include dir (if present).
        // The bundled dir is always the first entry added by default_system_include_paths().
        let insert_pos = if let Some(bundled) = Self::bundled_include_dir() {
            self.system_include_paths
                .iter()
                .position(|p| *p == bundled)
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            0
        };
        let mut offset = 0;
        for p in arch_paths {
            let path = PathBuf::from(p);
            if path.is_dir() {
                self.system_include_paths.insert(insert_pos + offset, path);
                offset += 1;
            }
        }
    }

    /// Override long double macros from x87 80-bit extended to IEEE 754 binary128.
    /// Called by set_target() for aarch64 and riscv64 which use quad precision.
    fn override_ldbl_binary128(&mut self) {
        // GCC predefined macros (__LDBL_*__)
        self.define_simple_macro("__LDBL_MANT_DIG__", "113");
        self.define_simple_macro("__LDBL_DIG__", "33");
        self.define_simple_macro(
            "__LDBL_EPSILON__",
            "1.92592994438723585305597794258492732e-34L",
        );
        self.define_simple_macro(
            "__LDBL_MAX__",
            "1.18973149535723176508575932662800702e+4932L",
        );
        self.define_simple_macro(
            "__LDBL_MIN__",
            "3.36210314311209350626267781732175260e-4932L",
        );
        self.define_simple_macro(
            "__LDBL_DENORM_MIN__",
            "6.47517511943802511092443895822764655e-4966L",
        );
        self.define_simple_macro("__LDBL_DECIMAL_DIG__", "36");
        self.define_simple_macro("__DECIMAL_DIG__", "36");
        // MIN_EXP, MAX_EXP, MIN_10_EXP, MAX_10_EXP are the same for x87 and binary128
        // (both use 15-bit exponent fields), so no override needed.

        // <float.h> macros (LDBL_*)
        self.define_simple_macro("LDBL_MANT_DIG", "113");
        self.define_simple_macro("LDBL_DIG", "33");
        self.define_simple_macro("DECIMAL_DIG", "36");
    }

    /// Override RISC-V preprocessor macros based on -mabi= and -march= flags.
    ///
    /// The kernel uses `-mabi=lp64` (soft-float ABI) and `-march=rv64imac...`
    /// (no F/D extensions). When these flags are set, we must adjust:
    /// - Float ABI macros: `__riscv_float_abi_soft` vs `__riscv_float_abi_double`
    /// - `__riscv_flen`: only defined when FPU is available
    /// - Extension macros: `__riscv_f`, `__riscv_d`, `__riscv_fdiv`, `__riscv_fsqrt`
    pub fn set_riscv_abi(&mut self, abi: &str) {
        match abi {
            "lp64" => {
                // Soft-float ABI (no FPU registers for argument passing).
                // Undefine double-float macros and define soft-float.
                self.macros.undefine("__riscv_float_abi_double");
                self.macros.undefine("__riscv_flen");
                self.macros.undefine("__riscv_fdiv");
                self.macros.undefine("__riscv_fsqrt");
                self.define_simple_macro("__riscv_float_abi_soft", "1");
            }
            "lp64f" => {
                // Single-float ABI.
                self.macros.undefine("__riscv_float_abi_double");
                self.define_simple_macro("__riscv_float_abi_single", "1");
                self.define_simple_macro("__riscv_flen", "32");
            }
            "lp64d" => {
                // Double-float ABI (default) - macros already set by set_target.
            }
            _ => {
                // Unknown ABI value - leave defaults (lp64d) in place.
                // This covers ilp32* ABIs and any future additions.
            }
        }
    }

    /// Override RISC-V extension macros based on -march= flag.
    ///
    /// The kernel uses -march=rv64imac... (no F/D extensions). When the march
    /// string doesn't contain 'f' or 'd' (or 'g' which implies both), we must
    /// remove F/D extension macros that set_target unconditionally defines.
    pub fn set_riscv_march(&mut self, march: &str) {
        // Extract the base ISA string (strip rv32/rv64 prefix for extension parsing).
        let exts = if let Some(rest) = march.strip_prefix("rv64") {
            rest
        } else if let Some(rest) = march.strip_prefix("rv32") {
            rest
        } else {
            march
        };
        // 'g' = imafd, so check for 'g' as well.
        // NOTE: This simple character check may false-positive on sub-extension names
        // (e.g., 'f' in "zifencei"). In practice, kernel -march strings use the
        // underscore-separated format (rv64imac_zicsr_zifencei) where single-letter
        // extensions precede the first underscore, so this heuristic works correctly.
        let has_f = exts.contains('f') || exts.contains('g');
        let has_d = exts.contains('d') || exts.contains('g');

        if !has_f {
            self.macros.undefine("__riscv_f");
            self.macros.undefine("__riscv_fdiv");
            self.macros.undefine("__riscv_fsqrt");
        }
        if !has_d {
            self.macros.undefine("__riscv_d");
        }
        if !has_f && !has_d {
            self.macros.undefine("__riscv_flen");
        }
    }
}

/// Apply the granular SSE/SSE2/MMX macro state to a preprocessor macro
/// table — the core of [`Preprocessor::set_sse_macros`], factored free so
/// the oracle-verified matrix is testable without a full preprocessor.
///
/// Non-x86 targets (no `__x86_64__` and no `__i386__`) get none of the
/// family at all. The level macros follow their levels exactly (see
/// `set_sse_macros` for the oracle provenance); the `_MATH` pair states
/// which scalar math lccc's codegen does in SSE (on i686 only F32, and only
/// with SSE2); `__MMX_WITH_SSE__` is granted only on x86-64, following the
/// SSE2 level (GCC 16.2, measured).
pub(crate) fn apply_sse_macro_levels(
    macros: &mut MacroTable,
    is_x86_64: bool,
    sse_on: bool,
    sse2_on: bool,
    mmx_on: bool,
) {
    if !is_x86_64 && !macros.is_defined("__i386__") {
        return;
    }

    let mut set = |name: &str, on: bool| {
        if on {
            macros.define(MacroDef {
                name: name.to_string(),
                is_function_like: false,
                params: Vec::new(),
                is_variadic: false,
                has_named_variadic: false,
                body: "1".to_string(),
            });
        } else {
            macros.undefine(name);
        }
    };
    set("__SSE__", sse_on);
    set("__SSE2__", sse2_on);
    set("__MMX__", mmx_on);
    // `_MATH`: SSE does the scalar float math, so it states lccc's own
    // codegen, not just the ISA level. x86-64 does F32/F64 in SSE/SSE2. The
    // i686 backend does F32 in SSE only with SSE2 (`sse2_scalar_enabled`;
    // an SSE1-only CPU stays on x87, `fmuls`) and F64 always on x87 (with
    // declared-type rounding), so `__SSE_MATH__` follows SSE2 and
    // `__SSE2_MATH__` is never defined: the GCC 16.2 macros of its
    // F32-in-SSE, F64-on-x87 mode (`-m32 -msse -mno-sse2 -mfpmath=sse`).
    // Copying Clang's i386 pair (it really does SSE2 math) told code keyed
    // on `__SSE2_MATH__` that doubles are never x87-evaluated.
    set("__SSE_MATH__", if is_x86_64 { sse_on } else { sse2_on });
    set("__SSE2_MATH__", is_x86_64 && sse2_on);
    // GCC 16.2, measured: x86-64-only, and it tracks the SSE2 level, NOT
    // the SSE1 level — `-mno-sse2` (SSE1 still granted) drops it, exactly
    // like `__SSE2__`; i386 never gets it, even at the x86-64 baseline
    // (`-m32` defines `__SSE2__` but not `__MMX_WITH_SSE__`).
    if is_x86_64 {
        set("__MMX_WITH_SSE__", mmx_on && sse2_on);
    }
}

#[cfg(test)]
mod sse_macro_tests {
    use super::*;
    use crate::frontend::preprocessor::macro_defs::{MacroDef, MacroTable};

    fn i386_table() -> MacroTable {
        let mut macros = MacroTable::new();
        macros.define(MacroDef {
            name: "__i386__".to_string(),
            is_function_like: false,
            params: Vec::new(),
            is_variadic: false,
            has_named_variadic: false,
            body: "1".to_string(),
        });
        macros
    }

    /// `[__SSE__, __SSE2__, __MMX__, __SSE_MATH__, __SSE2_MATH__, __MMX_WITH_SSE__]`
    fn defined_set(macros: &MacroTable) -> [bool; 6] {
        [
            macros.is_defined("__SSE__"),
            macros.is_defined("__SSE2__"),
            macros.is_defined("__MMX__"),
            macros.is_defined("__SSE_MATH__"),
            macros.is_defined("__SSE2_MATH__"),
            macros.is_defined("__MMX_WITH_SSE__"),
        ]
    }

    // Oracle-verified matrix (GCC 16.2 / Clang 23.1.3, i386 `-m32` family).
    // The driver resolves [sse, sse2, mmx]; `set_sse_macros` must publish
    // exactly the oracle macro set for the plain level macros. The `_MATH`
    // pair states lccc's i686 codegen (F32 in SSE with SSE2, F64 on x87):
    // `__SSE_MATH__` follows SSE2, `__SSE2_MATH__` never appears, as in
    // GCC 16.2's `-msse -mno-sse2 -mfpmath=sse` mode; `__MMX_WITH_SSE__`
    // is x86-64-only, so `set_sse_macros` (i386 path) never defines it.
    #[test]
    fn sse_macros_follow_granular_levels() {
        for (sse, sse2, mmx) in [
            (true, true, true),    // -m32 baseline (-march=x86-64 default)
            (true, false, true),   // -m32 -mno-sse2
            (false, false, true),  // -m32 -mno-sse
            (false, false, false), // -m32 -march=i686 (and -mgeneral-regs-only)
            (true, false, true),   // -m32 -march=i686 -msse: SSE revives, MMX with it
            (true, true, false),   // -m32 -march=i686 -msse2: SSE+MMX revive with SSE2
        ] {
            let mut macros = i386_table();
            apply_sse_macro_levels(&mut macros, false, sse, sse2, mmx);
            let d = defined_set(&macros);
            assert_eq!(
                d,
                [sse, sse2, mmx, sse2, false, false],
                "levels (sse={sse}, sse2={sse2}, mmx={mmx})"
            );
        }
    }

    // State tracking across changes: a state must clear what a prior state
    // defined and redefine what it grants — no stale level may survive.
    #[test]
    fn sse_macros_track_state_changes() {
        let mut macros = i386_table();
        apply_sse_macro_levels(&mut macros, false, true, true, true); // baseline
        apply_sse_macro_levels(&mut macros, false, false, false, false); // pre-SSE -march
        assert!(!macros.is_defined("__SSE__"));
        assert!(!macros.is_defined("__SSE2__"));
        assert!(!macros.is_defined("__MMX__"));
        assert!(!macros.is_defined("__SSE_MATH__"));
        assert!(!macros.is_defined("__SSE2_MATH__"));
        apply_sse_macro_levels(&mut macros, false, true, false, true); // -msse revival
        assert!(macros.is_defined("__SSE__"));
        assert!(macros.is_defined("__MMX__"));
        // SSE1 alone: F32 stays on x87.
        assert!(!macros.is_defined("__SSE_MATH__"));
        assert!(!macros.is_defined("__SSE2__"));
        assert!(!macros.is_defined("__SSE2_MATH__"));
        // `__MMX_WITH_SSE__` is published by the x86-64 path, never here.
        assert!(!macros.is_defined("__MMX_WITH_SSE__"));
    }

    // Driver-facing API: the same granular levels through the real
    // preprocessor (target state included).
    #[test]
    fn sse_only_i686_preserves_sse_but_not_sse2_macros() {
        let mut pp = Preprocessor::new();
        pp.set_target("i686");
        pp.set_sse_macros(true, false, true); // -m32 -mno-sse2
        assert!(pp.macros.is_defined("__SSE__"));
        assert!(!pp.macros.is_defined("__SSE2__"));
        assert!(!pp.macros.is_defined("__SSE2_MATH__"));
        assert!(pp.macros.is_defined("__MMX__"));

        pp.set_sse_macros(true, true, true); // -m32 baseline
        assert!(pp.macros.is_defined("__SSE__"));
        assert!(pp.macros.is_defined("__SSE2__"));
        // F32 in SSE, F64 on x87.
        assert!(pp.macros.is_defined("__SSE_MATH__"));
        assert!(!pp.macros.is_defined("__SSE2_MATH__"));

        pp.set_sse_macros(false, false, true); // -m32 -mno-sse (MMX keeps)
        assert!(!pp.macros.is_defined("__SSE__"));
        assert!(!pp.macros.is_defined("__SSE2__"));
        assert!(pp.macros.is_defined("__MMX__"));
    }

    #[test]
    fn sse2_denial_does_not_erase_independent_x86_64_sse_macros() {
        let mut pp = Preprocessor::new();
        pp.set_target("x86_64");
        pp.set_sse_macros(true, false, true); // -mno-sse2 on x86-64
        assert!(pp.macros.is_defined("__SSE__"));
        assert!(pp.macros.is_defined("__SSE_MATH__"));
        assert!(pp.macros.is_defined("__MMX__"));
        // GCC 16.2 (measured): `__MMX_WITH_SSE__` tracks the SSE2 level, so
        // the explicit SSE2 denial drops it even though SSE1 is preserved.
        assert!(!pp.macros.is_defined("__MMX_WITH_SSE__"));
        assert!(!pp.macros.is_defined("__SSE2__"));
        assert!(!pp.macros.is_defined("__SSE2_MATH__"));

        pp.set_sse_macros(true, true, true); // baseline again
        assert!(pp.macros.is_defined("__MMX_WITH_SSE__"));
        pp.set_sse_macros(true, false, true); // denial again clears it
        assert!(!pp.macros.is_defined("__MMX_WITH_SSE__"));
    }
}
