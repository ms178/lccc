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
    // We support _Complex types, so __STDC_NO_COMPLEX__ is NOT defined.
    // __STDC_NO_THREADS__ is NOT defined: glibc provides <threads.h>.
    // VLA declarations lower to DynAlloca, so __STDC_NO_VLA__ is NOT defined.
}
