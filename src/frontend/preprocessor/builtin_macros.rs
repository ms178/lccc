//! Compiler-owned built-in macros.
//!
//! Only macros that GCC itself predefines live here. Everything a C library
//! header owns (`CHAR_BIT`, `INT8_C`, `NULL`, `offsetof`, `EOF`, `BUFSIZ`,
//! `SEEK_*`, `EXIT_SUCCESS`, `FLT_MAX`, `PRIu64`, `errno` values, signal
//! numbers, ...) is left to that header: system headers are read (they always
//! were — see `includes.rs`), so predefining their macros shadowed the real
//! definitions and made `#ifdef`-based feature detection answer a question the
//! translation unit never asked. Measured before the change: 211 macros
//! defined by lccc and by no GCC 16.2 preprocessor invocation, every one of
//! them owned by libc.
//!
//! Deliberate violation of the rule, kept because it is a GCC predefine:
//! `__ATOMIC_*` (GCC predefines these; `<stdatomic.h>` uses them).

use super::macro_defs::{MacroDef, MacroTable};

/// Helper to define a simple object-like macro.
fn def(macros: &mut MacroTable, name: &str, body: &str) {
    macros.define(MacroDef {
        is_time_builtin: false,
        name: name.to_string(),
        is_function_like: false,
        params: Vec::new(),
        is_variadic: false,
        has_named_variadic: false,
        body: body.to_string(),
    });
}

/// Define the compiler-owned predefined macros (see the module docs).
pub fn define_builtin_macros(macros: &mut MacroTable) {
    define_stdatomic_macros(macros);
    define_extra_gcc_predefines(macros);
    // assert() is NOT defined here. It comes from the system <assert.h> header,
    // which properly handles NDEBUG. Code that uses assert() without including
    // <assert.h> will correctly get a compilation error.
    define_type_traits_macros(macros);
}

/// Define all stdbool.h macros (called on #include <stdbool.h>).
pub fn define_stdbool_true_false(macros: &mut MacroTable) {
    def(macros, "bool", "_Bool");
    def(macros, "true", "1");
    def(macros, "false", "0");
    def(macros, "__bool_true_false_are_defined", "1");
}

/// `__ATOMIC_*` memory-order constants (GCC predefines these).
fn define_stdatomic_macros(macros: &mut MacroTable) {
    def(macros, "__ATOMIC_RELAXED", "0");
    def(macros, "__ATOMIC_CONSUME", "1");
    def(macros, "__ATOMIC_ACQUIRE", "2");
    def(macros, "__ATOMIC_RELEASE", "3");
    def(macros, "__ATOMIC_ACQ_REL", "4");
    def(macros, "__ATOMIC_SEQ_CST", "5");
}

// static_assert is handled as a keyword (TokenKind::StaticAssert) rather
// than a preprocessor macro. Both _Static_assert and static_assert are
// recognized by the lexer and evaluated by the parser.

/// Type-related GCC built-in macros
fn define_type_traits_macros(macros: &mut MacroTable) {
    // __attribute__ - just strip it (make it a no-op)
    // This is handled in the parser typically, but some code uses it in macros
    def(macros, "__CHAR_BIT__", "8");
    def(macros, "__INT_MAX__", "2147483647");
    def(macros, "__LONG_MAX__", "9223372036854775807L");
    def(macros, "__LONG_LONG_MAX__", "9223372036854775807LL");
    def(macros, "__SHRT_MAX__", "32767");
    def(macros, "__SCHAR_MAX__", "127");
    def(macros, "__WCHAR_MAX__", crate::common::types::wchar_max());
    def(macros, "__SIZE_MAX__", "18446744073709551615UL");

    // Type width macros (GCC/Clang extension)
    def(macros, "__CHAR_WIDTH__", "8");
    def(macros, "__SCHAR_WIDTH__", "8");
    def(macros, "__SHRT_WIDTH__", "16");
    def(macros, "__INT_WIDTH__", "32");
    def(macros, "__LONG_WIDTH__", "64");
    def(macros, "__LONG_LONG_WIDTH__", "64");
    def(macros, "__PTRDIFF_WIDTH__", "64");
    def(macros, "__SIZE_WIDTH__", "64");
    def(
        macros,
        "__WCHAR_WIDTH__",
        crate::common::types::wchar_width(),
    );
    def(macros, "__WINT_WIDTH__", "32");
    def(macros, "__INTMAX_WIDTH__", "64");
    def(macros, "__INTPTR_WIDTH__", "64");
    def(macros, "__SIG_ATOMIC_WIDTH__", "32");
    // Fixed-width integer type widths (C23 / GCC extension)
    def(macros, "__INT_LEAST8_WIDTH__", "8");
    def(macros, "__INT_LEAST16_WIDTH__", "16");
    def(macros, "__INT_LEAST32_WIDTH__", "32");
    def(macros, "__INT_LEAST64_WIDTH__", "64");
    def(macros, "__INT_FAST8_WIDTH__", "8");
    def(macros, "__INT_FAST16_WIDTH__", "64");
    def(macros, "__INT_FAST32_WIDTH__", "64");
    def(macros, "__INT_FAST64_WIDTH__", "64");
    def(
        macros,
        "__SIZEOF_WCHAR_T__",
        crate::common::types::wchar_sizeof(),
    );

    // Byte order
    def(macros, "__BYTE_ORDER__", "1234");
    def(macros, "__ORDER_LITTLE_ENDIAN__", "1234");
    def(macros, "__ORDER_BIG_ENDIAN__", "4321");
    def(macros, "__ORDER_PDP_ENDIAN__", "3412");

    // __CHAR_UNSIGNED__: defined iff plain `char` is unsigned, read from the
    // same `char_is_unsigned()` the type system consults, so the type, this
    // macro and glibc's <limits.h> (which derives CHAR_MIN/CHAR_MAX from it)
    // give one answer.  The driver records the target default (unsigned on
    // AArch64 and RISC-V SysV, signed on x86) and then the flag overrides,
    // all before preprocessing starts.
    if crate::common::types::char_is_unsigned() {
        def(macros, "__CHAR_UNSIGNED__", "1");
    }
    // __WCHAR_UNSIGNED__: wchar_t is signed on Linux (unlike Windows).
    // Do not define it (it would be detected as "defined" by #ifdef).

    // Additional GCC version macros
    def(macros, "__GNUC_PATCHLEVEL__", "0");

    // C11/C17 feature test macros. `__STDC_NO_ATOMICS__` is deliberately NOT
    // defined: `_Atomic`, <stdatomic.h> and the `__atomic_*` builtins work, so
    // claiming otherwise is a capability denial that configures disable
    // atomics (measured: lccc defined it while GCC 16.2 did not).
    def(macros, "__STDC_UTF_16__", "1");
    def(macros, "__STDC_UTF_32__", "1");
    // NOTE: `__STDC_NO_ATOMICS__` is deliberately NOT predefined. LCCC
    // implements the __atomic_* builtins (GCC predefines none of the
    // glibc-facing atomics claims either); claiming no-atomics broke
    // <stdatomic.h>'s capability path (GP-05 measurement).
    // Note: We support _Complex types, so __STDC_NO_COMPLEX__ is NOT defined.
    // __STDC_NO_THREADS__ is NOT defined because we link against glibc which provides <threads.h>
    // VLA declarations lower to DynAlloca and are supported. Do not define
    // __STDC_NO_VLA__: autoconf uses its presence as a hard capability denial.
}

/// GCC 16.2 predefines LCCC lacked, derived from a complete `gcc-16.2 -dM
/// -E` differential (2026-10-28, GP-05): the compiler-owned
/// `__U<INT…>_MAX__`/`_C()` limit family, the atomics lock-free and
/// compare-exchange claims glibc's fast paths test, the IEC-559/ISO-10646
/// claims, segment-address support, and the binary32/64/128 `_FloatN`
/// capability probes. Values are verbatim GCC output. GCC-only names NOT
/// added here (decimal-FP `__DEC*` macros, unverified `__GCC_*` asm/CFI
/// claims, HLE hints, C++ ABI identity) remain pinned as honest gaps in
/// `tests/regression/check_macro_parity.py`.
const EXTRA_GCC_PREDEFINES: &[(&str, &str)] = &[
    ("_STDC_PREDEF_H", "1"),
    ("__BIGGEST_ALIGNMENT__", "16"),
    ("__BITINT_MAXWIDTH__", "65535"),
    ("__CHAR8_TYPE__", "unsigned char"),
    ("__DBL_IS_IEC_60559__", "1"),
    (
        "__DBL_NORM_MAX__",
        "((double)1.79769313486231570814527423731704357e+308L)",
    ),
    ("__FINITE_MATH_ONLY__", "0"),
    ("__FLOAT_WORD_ORDER__", "__ORDER_LITTLE_ENDIAN__"),
    ("__FLT128_IS_IEC_60559__", "1"),
    ("__FLT32X_DECIMAL_DIG__", "17"),
    (
        "__FLT32X_DENORM_MIN__",
        "4.94065645841246544176568792868221372e-324F32x",
    ),
    ("__FLT32X_DIG__", "15"),
    (
        "__FLT32X_EPSILON__",
        "2.22044604925031308084726333618164062e-16F32x",
    ),
    ("__FLT32X_HAS_DENORM__", "1"),
    ("__FLT32X_HAS_INFINITY__", "1"),
    ("__FLT32X_HAS_QUIET_NAN__", "1"),
    ("__FLT32X_IS_IEC_60559__", "1"),
    ("__FLT32X_MANT_DIG__", "53"),
    ("__FLT32X_MAX_10_EXP__", "308"),
    ("__FLT32X_MAX_EXP__", "1024"),
    (
        "__FLT32X_MAX__",
        "1.79769313486231570814527423731704357e+308F32x",
    ),
    ("__FLT32X_MIN_10_EXP__", "(-307)"),
    ("__FLT32X_MIN_EXP__", "(-1021)"),
    (
        "__FLT32X_MIN__",
        "2.22507385850720138309023271733240406e-308F32x",
    ),
    (
        "__FLT32X_NORM_MAX__",
        "1.79769313486231570814527423731704357e+308F32x",
    ),
    ("__FLT32_DECIMAL_DIG__", "9"),
    (
        "__FLT32_DENORM_MIN__",
        "1.40129846432481707092372958328991613e-45F32",
    ),
    ("__FLT32_DIG__", "6"),
    (
        "__FLT32_EPSILON__",
        "1.19209289550781250000000000000000000e-7F32",
    ),
    ("__FLT32_HAS_DENORM__", "1"),
    ("__FLT32_HAS_INFINITY__", "1"),
    ("__FLT32_HAS_QUIET_NAN__", "1"),
    ("__FLT32_IS_IEC_60559__", "1"),
    ("__FLT32_MANT_DIG__", "24"),
    ("__FLT32_MAX_10_EXP__", "38"),
    ("__FLT32_MAX_EXP__", "128"),
    (
        "__FLT32_MAX__",
        "3.40282346638528859811704183484516925e+38F32",
    ),
    ("__FLT32_MIN_10_EXP__", "(-37)"),
    ("__FLT32_MIN_EXP__", "(-125)"),
    (
        "__FLT32_MIN__",
        "1.17549435082228750796873653722224568e-38F32",
    ),
    (
        "__FLT32_NORM_MAX__",
        "3.40282346638528859811704183484516925e+38F32",
    ),
    ("__FLT64X_DECIMAL_DIG__", "21"),
    (
        "__FLT64X_DENORM_MIN__",
        "3.64519953188247460252840593361941982e-4951F64x",
    ),
    ("__FLT64X_DIG__", "18"),
    (
        "__FLT64X_EPSILON__",
        "1.08420217248550443400745280086994171e-19F64x",
    ),
    ("__FLT64X_HAS_DENORM__", "1"),
    ("__FLT64X_HAS_INFINITY__", "1"),
    ("__FLT64X_HAS_QUIET_NAN__", "1"),
    ("__FLT64X_IS_IEC_60559__", "1"),
    ("__FLT64X_MANT_DIG__", "64"),
    ("__FLT64X_MAX_10_EXP__", "4932"),
    ("__FLT64X_MAX_EXP__", "16384"),
    (
        "__FLT64X_MAX__",
        "1.18973149535723176502126385303097021e+4932F64x",
    ),
    ("__FLT64X_MIN_10_EXP__", "(-4931)"),
    ("__FLT64X_MIN_EXP__", "(-16381)"),
    (
        "__FLT64X_MIN__",
        "3.36210314311209350626267781732175260e-4932F64x",
    ),
    (
        "__FLT64X_NORM_MAX__",
        "1.18973149535723176502126385303097021e+4932F64x",
    ),
    ("__FLT64_DECIMAL_DIG__", "17"),
    (
        "__FLT64_DENORM_MIN__",
        "4.94065645841246544176568792868221372e-324F64",
    ),
    ("__FLT64_DIG__", "15"),
    (
        "__FLT64_EPSILON__",
        "2.22044604925031308084726333618164062e-16F64",
    ),
    ("__FLT64_HAS_DENORM__", "1"),
    ("__FLT64_HAS_INFINITY__", "1"),
    ("__FLT64_HAS_QUIET_NAN__", "1"),
    ("__FLT64_IS_IEC_60559__", "1"),
    ("__FLT64_MANT_DIG__", "53"),
    ("__FLT64_MAX_10_EXP__", "308"),
    ("__FLT64_MAX_EXP__", "1024"),
    (
        "__FLT64_MAX__",
        "1.79769313486231570814527423731704357e+308F64",
    ),
    ("__FLT64_MIN_10_EXP__", "(-307)"),
    ("__FLT64_MIN_EXP__", "(-1021)"),
    (
        "__FLT64_MIN__",
        "2.22507385850720138309023271733240406e-308F64",
    ),
    (
        "__FLT64_NORM_MAX__",
        "1.79769313486231570814527423731704357e+308F64",
    ),
    ("__FLT_IS_IEC_60559__", "1"),
    (
        "__FLT_NORM_MAX__",
        "3.40282346638528859811704183484516925e+38F",
    ),
    ("__FXSR__", "1"),
    ("__GCC_ATOMIC_CHAR8_T_LOCK_FREE", "2"),
    ("__GCC_ATOMIC_TEST_AND_SET_TRUEVAL", "1"),
    ("__GCC_HAVE_SYNC_COMPARE_AND_SWAP_1", "1"),
    ("__GCC_HAVE_SYNC_COMPARE_AND_SWAP_2", "1"),
    ("__GCC_HAVE_SYNC_COMPARE_AND_SWAP_4", "1"),
    ("__GCC_HAVE_SYNC_COMPARE_AND_SWAP_8", "1"),
    ("__GCC_IEC_559", "2"),
    ("__GCC_IEC_559_COMPLEX", "2"),
    ("__GNUC_EXECUTION_CHARSET_NAME", "\"UTF-8\""),
    ("__GNUC_WIDE_EXECUTION_CHARSET_NAME", "\"UTF-32LE\""),
    ("__INT16_MAX__", "0x7fff"),
    ("__INT32_MAX__", "0x7fffffff"),
    ("__INT64_MAX__", "0x7fffffffffffffffL"),
    ("__INT8_MAX__", "0x7f"),
    ("__INTMAX_MAX__", "0x7fffffffffffffffL"),
    ("__INTPTR_MAX__", "0x7fffffffffffffffL"),
    ("__INT_FAST16_MAX__", "0x7fffffffffffffffL"),
    ("__INT_FAST32_MAX__", "0x7fffffffffffffffL"),
    ("__INT_FAST64_MAX__", "0x7fffffffffffffffL"),
    ("__INT_FAST8_MAX__", "0x7f"),
    ("__INT_LEAST16_MAX__", "0x7fff"),
    ("__INT_LEAST32_MAX__", "0x7fffffff"),
    ("__INT_LEAST64_MAX__", "0x7fffffffffffffffL"),
    ("__INT_LEAST8_MAX__", "0x7f"),
    ("__LDBL_IS_IEC_60559__", "1"),
    (
        "__LDBL_NORM_MAX__",
        "1.18973149535723176502126385303097021e+4932L",
    ),
    ("__NO_INLINE__", "1"),
    ("__SEG_FS", "1"),
    ("__SEG_GS", "1"),
    ("__SIG_ATOMIC_TYPE__", "int"),
    ("__SIZEOF_FLOAT80__", "16"),
    ("__STDC_IEC_559_COMPLEX__", "1"),
    ("__STDC_IEC_559__", "1"),
    ("__STDC_IEC_60559_BFP__", "201404L"),
    ("__STDC_IEC_60559_COMPLEX__", "201404L"),
    ("__STDC_ISO_10646__", "201706L"),
    ("__UINT16_MAX__", "0xffff"),
    ("__UINT32_MAX__", "0xffffffffU"),
    ("__UINT64_MAX__", "0xffffffffffffffffUL"),
    ("__UINT8_MAX__", "0xff"),
    ("__UINTMAX_MAX__", "0xffffffffffffffffUL"),
    ("__UINTPTR_MAX__", "0xffffffffffffffffUL"),
    ("__UINT_FAST16_MAX__", "0xffffffffffffffffUL"),
    ("__UINT_FAST32_MAX__", "0xffffffffffffffffUL"),
    ("__UINT_FAST64_MAX__", "0xffffffffffffffffUL"),
    ("__UINT_FAST8_MAX__", "0xff"),
    ("__UINT_LEAST16_MAX__", "0xffff"),
    ("__UINT_LEAST32_MAX__", "0xffffffffU"),
    ("__UINT_LEAST64_MAX__", "0xffffffffffffffffUL"),
    ("__UINT_LEAST8_MAX__", "0xff"),
];

const EXTRA_GCC_FUNC_PREDEFINES: &[(&str, &[&str], &str)] = &[
    ("__INT16_C", &["c"], "c"),
    ("__INT32_C", &["c"], "c"),
    ("__INT64_C", &["c"], "c ## L"),
    ("__INT8_C", &["c"], "c"),
    ("__INTMAX_C", &["c"], "c ## L"),
    ("__UINT16_C", &["c"], "c"),
    ("__UINT32_C", &["c"], "c ## U"),
    ("__UINT64_C", &["c"], "c ## UL"),
    ("__UINT8_C", &["c"], "c"),
    ("__UINTMAX_C", &["c"], "c ## UL"),
];

fn define_extra_gcc_predefines(macros: &mut MacroTable) {
    for &(name, body) in EXTRA_GCC_PREDEFINES {
        def(macros, name, body);
    }
    for &(name, params, body) in EXTRA_GCC_FUNC_PREDEFINES {
        macros.define(MacroDef {
            is_time_builtin: false,
            name: name.to_string(),
            is_function_like: true,
            params: params.iter().map(|p| p.to_string()).collect(),
            is_variadic: false,
            has_named_variadic: false,
            body: body.to_string(),
        });
    }
}
