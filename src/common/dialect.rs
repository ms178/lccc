//! Per-dialect language capabilities.
//!
//! A C compiler has a handful of independent dialect switches -- ISO revision,
//! GNU extensions on or off -- but the *decisions* the front end has to make are
//! not "which revision is this", they are questions like "is `static_assert` a
//! keyword here", "may a character constant carry a `u8` prefix", "does
//! `va_start` take one argument".  Those answers used to be open-coded as
//! `if c23` / `if gnu_extensions || c23` in the lexer, the preprocessor, the
//! parser and the bundled headers, which is how a C23 spelling ends up accepted
//! by `-std=c17` in one place and rejected in another.
//!
//! This type is the single place where such a question is answered.  It holds
//! no state beyond the two switches it is constructed from, so it can be built
//! cheaply at any decision site (`Dialect::new(c23, gnu)`) and remains correct
//! for callers that own their own flags, such as the unit tests that drive a
//! lexer directly.
//!
//! Every capability below was measured against GCC 16.2.0 and Clang 23.1.4 with
//! `-fsyntax-only` across `c89 c99 c11 c17 c23 gnu17 gnu23`; the gate
//! `tests/regression/check_c23_inventory.py` re-measures them on every run, so a
//! claim here that stops being true fails CI rather than rotting in a comment.
//! Where the two compilers disagree, GCC is the reference (the rule this
//! project validates strict-vs-GNU acceptance against).

/// The dialect switches the front end acts on.
///
/// `c23` means "the C23 revision of the language, either `-std=c23` or
/// `-std=gnu23`"; `gnu` means "GNU extensions are enabled", which is true for
/// every `-std=gnu*` dialect and false for `-ansi`, `-std=c89` ... `-std=c23`.
/// The default dialect is `gnu23`, matching GCC 16.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dialect {
    /// The translation unit is compiled as C23 (or a later revision).
    pub(crate) c23: bool,
    /// GNU extensions are enabled.
    pub(crate) gnu: bool,
}

impl Dialect {
    /// The capability set for a pair of switches.
    pub(crate) const fn new(c23: bool, gnu: bool) -> Self {
        Self { c23, gnu }
    }

    // --- C23 keywords (6.4.1) ---------------------------------------------
    //
    // C23 turned a group of `<...h>` macros into keywords.  Before C23 the
    // spellings are ordinary identifiers: `void static_assert(void);` and
    // `int alignof;` are valid C17, and `alignas`/`alignof`/`static_assert`
    // reach the language through `<stdalign.h>` and `<assert.h>`, which is why
    // treating them as keywords early both rejects valid C17 and accepts C23 in
    // a C17 mode.  GCC applies this boundary in `gnu17`/`gnu11`/... as well,
    // so it is keyed on the revision alone and not on the GNU switch.

    /// `static_assert` is a keyword (C23); `_Static_assert` always is.
    pub(crate) const fn keyword_static_assert(self) -> bool {
        self.c23
    }

    /// `alignas` is a keyword (C23); `_Alignas` always is.
    pub(crate) const fn keyword_alignas(self) -> bool {
        self.c23
    }

    /// `alignof` is a keyword (C23); `_Alignof` always is.
    pub(crate) const fn keyword_alignof(self) -> bool {
        self.c23
    }

    /// `bool`, `true` and `false` are keywords (C23, 6.2.5p2/6.4.1).
    pub(crate) const fn keyword_bool(self) -> bool {
        self.c23
    }

    /// `thread_local`, `constexpr` and `nullptr` are keywords (C23).
    pub(crate) const fn keyword_c23_objects(self) -> bool {
        self.c23
    }

    // --- GNU-reserved spellings -------------------------------------------

    /// Bare `typeof` is a GNU extension that C23 also standardised, so GCC
    /// accepts it in `gnu*` and in C23 but not in `-std=c17`.  The reserved
    /// spellings `__typeof__`/`__typeof` are keywords everywhere and are not
    /// covered by this capability.
    pub(crate) const fn keyword_typeof(self) -> bool {
        self.gnu || self.c23
    }

    /// Bare `typeof_unqual` is *only* C23: GCC accepts it in `-std=c23` and
    /// `-std=gnu23` and rejects it in `gnu17`, where only the reserved
    /// `__typeof_unqual__` spelling exists.  Measured on GCC 16.2.0; this is
    /// the one place where a C23 type operator is narrower than `typeof`.
    pub(crate) const fn keyword_typeof_unqual(self) -> bool {
        self.c23
    }

    /// Bare `asm` is a GNU extension only: GCC accepts it in `gnu*` dialects and
    /// rejects it in `-std=c23`.  `__asm__`/`__asm` are keywords everywhere.
    pub(crate) const fn keyword_asm(self) -> bool {
        self.gnu
    }

    // --- Literal prefixes --------------------------------------------------

    /// `u8'x'` is a character constant from C23 (6.4.4.4).  Before C23 `u8` is
    /// an ordinary identifier and the quote begins a separate narrow constant,
    /// which is why GCC reports `'u8' undeclared` instead of accepting the
    /// literal.  `u8"..."` is a string prefix from C11 and is not affected.
    pub(crate) const fn u8_char_literal(self) -> bool {
        self.c23
    }

    // --- Preprocessing -----------------------------------------------------

    /// `#elifdef` / `#elifndef` are directives from C23 (6.10.2).  GCC makes
    /// them an error in a strict pre-C23 dialect ("invalid preprocessing
    /// directive") and enables them without a diagnostic in `gnu*` (where they
    /// are an extension) and in C23.  Clang warns and accepts everywhere; GCC's
    /// behaviour is the one this compiler follows, because accepting the
    /// directive in `-std=c17` would silently change the meaning of a
    /// conditional that GCC rejects.
    pub(crate) const fn pp_elifdef(self) -> bool {
        self.gnu || self.c23
    }

    // --- Library facilities expressed as macros ----------------------------

    /// `va_start(ap)` with a single argument is C23 (N2975).  Up to C17 the
    /// macro requires the last named parameter, and GCC's `<stdarg.h>` enforces
    /// that in `gnu17` as well as `c17`, so the one-argument form is keyed on
    /// the revision only.
    pub(crate) const fn va_start_one_argument(self) -> bool {
        self.c23
    }
}

#[cfg(test)]
mod tests {
    use super::Dialect;

    /// The matrix the gate measures, asserted here as well so that a change to
    /// this file fails the unit tests before it reaches the integration gate.
    #[test]
    fn capability_matrix_matches_gcc_16_2() {
        // (c23, gnu, static_assert/alignas/alignof/bool, typeof, typeof_unqual,
        //  asm, u8 char literal, #elifdef, one-argument va_start)
        let matrix = [
            (
                false, false, false, false, false, false, false, false, false,
            ),
            (false, true, false, true, false, true, false, true, false),
            (true, false, true, true, true, false, true, true, true),
            (true, true, true, true, true, true, true, true, true),
        ];
        for (c23, gnu, c23_kw, typeof_kw, typeof_unqual_kw, asm_kw, u8_char, elifdef, va_start) in
            matrix
        {
            let d = Dialect::new(c23, gnu);
            let label = format!("c23={c23} gnu={gnu}");
            assert_eq!(d.keyword_static_assert(), c23_kw, "static_assert {label}");
            assert_eq!(d.keyword_alignas(), c23_kw, "alignas {label}");
            assert_eq!(d.keyword_alignof(), c23_kw, "alignof {label}");
            assert_eq!(d.keyword_bool(), c23_kw, "bool {label}");
            assert_eq!(d.keyword_typeof(), typeof_kw, "typeof {label}");
            assert_eq!(
                d.keyword_typeof_unqual(),
                typeof_unqual_kw,
                "typeof_unqual {label}"
            );
            assert_eq!(d.keyword_asm(), asm_kw, "asm {label}");
            assert_eq!(d.u8_char_literal(), u8_char, "u8 char {label}");
            assert_eq!(d.pp_elifdef(), elifdef, "#elifdef {label}");
            assert_eq!(
                d.va_start_one_argument(),
                va_start,
                "one-argument va_start {label}"
            );
        }
    }

    /// `typeof_unqual` is the case a blanket "C23 keywords are also GNU
    /// extensions" rule gets wrong, so it gets its own assertion.
    #[test]
    fn typeof_unqual_is_c23_only_while_typeof_is_not() {
        let gnu17 = Dialect::new(false, true);
        assert!(gnu17.keyword_typeof());
        assert!(!gnu17.keyword_typeof_unqual());
        assert!(!Dialect::new(false, false).keyword_typeof());
        assert!(Dialect::new(true, false).keyword_typeof_unqual());
    }

    /// Bare `asm` runs the other way round: a GNU extension that C23 did not
    /// adopt, so it must stay off in a strict C23 mode.
    #[test]
    fn bare_asm_stays_a_gnu_extension_in_c23() {
        assert!(!Dialect::new(true, false).keyword_asm());
        assert!(Dialect::new(true, true).keyword_asm());
    }
}
