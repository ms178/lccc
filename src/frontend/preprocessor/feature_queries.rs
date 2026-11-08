//! GCC-compatible `__has_feature` / `__has_extension` capability queries.
//!
//! The query is a claim about THIS compiler, not a wish list copied from a
//! reference compiler.  A row answers 1 only when the named capability is
//! actually implemented; an unimplemented row answers 0 so headers take their
//! fallback instead of emitting code the front end cannot honor.
//!
//! # Answer rules (measured on GCC 16.2 across c89..gnu23)
//!
//! Three independent gates decide an answer, and conflating them is the bug
//! this module exists to prevent:
//!
//! 1. **Implemented here?**  If LCCC does not implement the capability, both
//!    operators answer 0 in every mode.  Truthfulness outranks compatibility.
//! 2. **Available in this translation unit?**  A mode requirement (an active
//!    sanitizer) gates *both* operators: `__has_extension(address_sanitizer)`
//!    is 0 without `-fsanitize=address`, because the capability is absent from
//!    the TU rather than merely non-standard.
//! 3. **Standard in this dialect?**  The dialect gate applies to the *feature*
//!    answer only.  `__has_feature(c_atomic)` is 0 in C99 while
//!    `__has_extension(c_atomic)` is 1: the compiler still accepts `_Atomic`
//!    there as an extension.  `-pedantic-errors` rejects extensions, so it
//!    collapses the extension answer onto the feature answer.
//!    `-Werror=pedantic` alone does NOT: it upgrades diagnostics, it does not
//!    withdraw the extension.
//!
//! Evidence: `engineering/evidence/2026-11-08-rucc-audit/feature-query-oracle.json`
//! (GCC 16.2 + Clang 23.1.5, 21 names x 18 modes).  The capability-row approach
//! was informed by the rucc audit (tamnd/rucc
//! 4f9f5d857349ffd8ddfdcdbd79d6dadb87dc50e9, crates/rucc-gnu, Apache-2.0);
//! no rucc source is copied here — LCCC is CC0 and the rows below are
//! independently measured against GCC and gated on LCCC's own capabilities.

/// Translation-unit mode relevant to feature queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FeatureMode {
    /// The selected dialect is C11 or later (`flag_isoc11`).
    pub c11: bool,
    /// The selected dialect is C23 or later (`flag_isoc23`).
    pub c23: bool,
    /// `-pedantic-errors` is active; `__has_extension` then answers as
    /// `__has_feature`.  `-Werror=pedantic` alone does NOT set this.
    pub pedantic_errors: bool,
    /// A GCC `-fsanitize=undefined` member (including `bounds-strict`) is
    /// active.  `float-divide-by-zero` alone is not an undefined-behavior
    /// sanitizer row in GCC 16.2.
    pub ubsan: bool,
}

impl Default for FeatureMode {
    fn default() -> Self {
        // Matches Preprocessor::new()'s GCC 16.2 default dialect, gnu23.
        Self {
            c11: true,
            c23: true,
            pedantic_errors: false,
            ubsan: false,
        }
    }
}

/// Dialect in which a row becomes a *standard* feature.  This gates the
/// `__has_feature` answer only; availability as an extension is dialect-free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialect {
    /// Standard in every supported dialect (GCC registers no dialect gate).
    Any,
    C11,
    C23,
}

/// A translation-unit requirement that gates BOTH operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Requires {
    None,
    /// An active `-fsanitize=undefined` member.
    Ubsan,
}

/// One capability row.
#[derive(Clone, Copy, Debug)]
struct Row {
    /// Truthfulness gate: LCCC implements the capability end to end.
    implemented: bool,
    /// Dialect from which the row is a standard feature.
    standard_from: Dialect,
    /// Mode requirement gating both operators.
    requires: Requires,
    /// GCC registers the row as an extension only: never a feature, and the
    /// first row `-pedantic-errors` withdraws.
    extension_only: bool,
}

const fn feature(implemented: bool, standard_from: Dialect) -> Row {
    Row {
        implemented,
        standard_from,
        requires: Requires::None,
        extension_only: false,
    }
}

const fn extension(implemented: bool) -> Row {
    Row {
        implemented,
        standard_from: Dialect::Any,
        requires: Requires::None,
        extension_only: true,
    }
}

const fn sanitized(implemented: bool) -> Row {
    Row {
        implemented,
        standard_from: Dialect::Any,
        requires: Requires::Ubsan,
        extension_only: false,
    }
}

/// Look up a capability row by its (already unarmoured) GCC name.
///
/// A `match` on `&str` rather than a linear scan over a table: rustc lowers it
/// to a length switch plus `memcmp`, so the cost is independent of the number
/// of rows and there is no second copy of the name list to drift out of sync.
///
/// Deliberately `implemented: false` until LCCC grows the capability:
///   * `attribute_deprecated_with_message`, `attribute_unavailable_with_message`,
///     `enumerator_attributes`: the parser accepts the spellings but sema does
///     not yet emit the deprecated/unavailable diagnostics, so a 1 would be a
///     false capability claim.
///   * `address_sanitizer`, `hwaddress_sanitizer`, `leak_sanitizer`,
///     `thread_sanitizer`: LCCC does not implement these sanitizers.
fn lookup(name: &str) -> Option<Row> {
    Some(match name {
        "c_alignas" => feature(true, Dialect::C11),
        "c_alignof" => feature(true, Dialect::C11),
        "c_atomic" => feature(true, Dialect::C11),
        "c_generic_selections" => feature(true, Dialect::C11),
        "c_static_assert" => feature(true, Dialect::C11),
        "c_thread_local" => feature(true, Dialect::C11),
        // GCC spells the C23 binary-literal row with its C++ origin name.
        "cxx_binary_literals" => feature(true, Dialect::C23),
        "tls" => feature(true, Dialect::Any),
        "undefined_behavior_sanitizer" => sanitized(true),
        "address_sanitizer" => feature(false, Dialect::Any),
        "hwaddress_sanitizer" => feature(false, Dialect::Any),
        "leak_sanitizer" => feature(false, Dialect::Any),
        "thread_sanitizer" => feature(false, Dialect::Any),
        "attribute_deprecated_with_message" => feature(false, Dialect::Any),
        "attribute_unavailable_with_message" => feature(false, Dialect::Any),
        "enumerator_attributes" => feature(false, Dialect::Any),
        "gnu_asm_goto_with_outputs" => extension(true),
        "gnu_asm_goto_with_outputs_full" => extension(true),
        _ => return None,
    })
}

/// GCC accepts the armoured spelling of a feature name (`__c_atomic__`).
/// Half armour (`__c_atomic`, `c_atomic__`) is a different, unknown identifier.
fn unarmour(name: &str) -> &str {
    name.strip_prefix("__")
        .and_then(|n| n.strip_suffix("__"))
        .filter(|n| !n.is_empty())
        .unwrap_or(name)
}

/// Answer `__has_feature(name)` (`extension == false`) or
/// `__has_extension(name)` (`extension == true`) in `mode`.
///
/// Total and panic-free for every input: unknown names, empty names and
/// arbitrary UTF-8 all answer `false`.
pub(super) fn resolve(name: &str, extension: bool, mode: FeatureMode) -> bool {
    let Some(row) = lookup(unarmour(name)) else {
        return false;
    };
    if !row.implemented {
        return false;
    }
    // Gate 2: a mode requirement withdraws the capability from this TU for
    // BOTH operators (an absent sanitizer is not merely "non-standard").
    match row.requires {
        Requires::None => {}
        Requires::Ubsan if !mode.ubsan => return false,
        Requires::Ubsan => {}
    }
    // `__has_extension` without -pedantic-errors asks only "is it available?",
    // which is dialect-independent: C99 still accepts _Atomic as an extension.
    if extension && !mode.pedantic_errors {
        return true;
    }
    // The feature answer (and the collapsed extension answer) needs the row to
    // be standard in the selected dialect and not extension-only.
    if row.extension_only {
        return false;
    }
    match row.standard_from {
        Dialect::Any => true,
        Dialect::C11 => mode.c11,
        Dialect::C23 => mode.c23,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C11_NAMES: &[&str] = &[
        "c_alignas",
        "c_alignof",
        "c_atomic",
        "c_generic_selections",
        "c_static_assert",
        "c_thread_local",
    ];

    const UNIMPLEMENTED: &[&str] = &[
        "address_sanitizer",
        "hwaddress_sanitizer",
        "leak_sanitizer",
        "thread_sanitizer",
        "attribute_deprecated_with_message",
        "attribute_unavailable_with_message",
        "enumerator_attributes",
    ];

    const ASM_GOTO: &[&str] = &[
        "gnu_asm_goto_with_outputs",
        "gnu_asm_goto_with_outputs_full",
    ];

    fn mode(c11: bool, c23: bool, pedantic_errors: bool, ubsan: bool) -> FeatureMode {
        FeatureMode {
            c11,
            c23,
            pedantic_errors,
            ubsan,
        }
    }

    /// The oracle's dialect axis: (c11, c23) for c89..gnu23.  C23 implies C11.
    const DIALECTS: [(bool, bool); 3] = [(false, false), (true, false), (true, true)];

    #[test]
    fn c11_rows_are_features_from_c11_and_extensions_before_it() {
        for name in C11_NAMES {
            for (c11, c23) in DIALECTS {
                let m = mode(c11, c23, false, false);
                assert_eq!(resolve(name, false, m), c11, "{name} feature in {m:?}");
                // Availability as an extension does not depend on the dialect.
                assert!(resolve(name, true, m), "{name} extension in {m:?}");
            }
        }
    }

    #[test]
    fn binary_literals_are_a_c23_feature_and_an_earlier_extension() {
        for (c11, c23) in DIALECTS {
            let m = mode(c11, c23, false, false);
            assert_eq!(
                resolve("cxx_binary_literals", false, m),
                c23,
                "feature in {m:?}"
            );
            assert!(
                resolve("cxx_binary_literals", true, m),
                "extension in {m:?}"
            );
        }
    }

    #[test]
    fn tls_is_a_feature_in_every_dialect() {
        for (c11, c23) in DIALECTS {
            for pedantic in [false, true] {
                let m = mode(c11, c23, pedantic, false);
                assert!(resolve("tls", false, m), "tls feature in {m:?}");
                assert!(resolve("tls", true, m), "tls extension in {m:?}");
            }
        }
    }

    #[test]
    fn asm_goto_with_outputs_is_never_a_feature() {
        for name in ASM_GOTO {
            for (c11, c23) in DIALECTS {
                assert!(!resolve(name, false, mode(c11, c23, false, false)));
                assert!(!resolve(name, false, mode(c11, c23, true, false)));
                assert!(resolve(name, true, mode(c11, c23, false, false)));
                // -pedantic-errors rejects the extension outright.
                assert!(!resolve(name, true, mode(c11, c23, true, false)));
            }
        }
    }

    #[test]
    fn pedantic_errors_collapses_extension_onto_feature() {
        for (c11, c23) in DIALECTS {
            let plain = mode(c11, c23, false, false);
            let strict = mode(c11, c23, true, false);
            for name in C11_NAMES
                .iter()
                .chain(["cxx_binary_literals", "tls"].iter())
            {
                assert_eq!(
                    resolve(name, true, strict),
                    resolve(name, false, plain),
                    "{name} must collapse in {strict:?}"
                );
            }
        }
    }

    #[test]
    fn ubsan_row_gates_both_operators_on_the_sanitizer_mode() {
        for (c11, c23) in DIALECTS {
            for pedantic in [false, true] {
                let off = mode(c11, c23, pedantic, false);
                assert!(!resolve("undefined_behavior_sanitizer", false, off));
                assert!(!resolve("undefined_behavior_sanitizer", true, off));
                let on = mode(c11, c23, pedantic, true);
                assert!(resolve("undefined_behavior_sanitizer", false, on));
                assert!(resolve("undefined_behavior_sanitizer", true, on));
            }
        }
    }

    #[test]
    fn armoured_spelling_matches_the_plain_name() {
        for (c11, c23) in DIALECTS {
            for pedantic in [false, true] {
                let m = mode(c11, c23, pedantic, false);
                for name in C11_NAMES.iter().chain(ASM_GOTO.iter()) {
                    let armoured = format!("__{name}__");
                    assert_eq!(resolve(&armoured, false, m), resolve(name, false, m));
                    assert_eq!(resolve(&armoured, true, m), resolve(name, true, m));
                }
            }
        }
    }

    #[test]
    fn half_armour_and_degenerate_names_are_unknown() {
        let m = mode(true, true, false, false);
        for name in [
            "__c_atomic",
            "c_atomic__",
            "__",
            "____",
            "",
            "_",
            "__tls",
            "tls__",
            "___tls__",
        ] {
            assert!(!resolve(name, false, m), "{name:?} feature");
            assert!(!resolve(name, true, m), "{name:?} extension");
        }
        // Triple armour leaves one layer behind, which is not a known name.
        assert!(!resolve("___c_atomic___", false, m));
    }

    #[test]
    fn unknown_and_unimplemented_rows_answer_zero_in_every_mode() {
        for c11 in [false, true] {
            for c23 in [false, true] {
                for pedantic in [false, true] {
                    for ubsan in [false, true] {
                        let m = mode(c11, c23, pedantic, ubsan);
                        for name in UNIMPLEMENTED.iter().chain(
                            [
                                "modules",
                                "__has_include",
                                "c_static_assert_no_message",
                                "objc_arc",
                                "blocks",
                                "c_alignas__",
                                "C_ATOMIC",
                                "c_atomic ",
                                "c_atomic)",
                                "\u{0}c_atomic",
                                "c_atomic\u{0}",
                                "féature",
                            ]
                            .iter(),
                        ) {
                            assert!(!resolve(name, false, m), "{name:?} feature in {m:?}");
                            assert!(!resolve(name, true, m), "{name:?} extension in {m:?}");
                        }
                    }
                }
            }
        }
    }

    /// Names are matched exactly: no prefix, suffix, case or whitespace
    /// normalization may smuggle an unknown name into a known row.
    #[test]
    fn lookup_is_exact() {
        assert!(lookup("c_atomic").is_some());
        for name in [
            "c_atomicX",
            "Xc_atomic",
            "c_atom",
            "C_atomic",
            " c_atomic",
            "c_atomic\n",
        ] {
            assert!(lookup(name).is_none(), "{name:?}");
        }
    }

    /// Every implemented row must be reachable through `resolve` in the
    /// permissive mode; a typo in a row name would otherwise silently answer 0
    /// forever and no dialect test would notice.
    #[test]
    fn every_implemented_row_is_reachable() {
        let m = mode(true, true, false, true);
        for name in C11_NAMES
            .iter()
            .chain(["cxx_binary_literals", "tls", "undefined_behavior_sanitizer"].iter())
            .chain(ASM_GOTO.iter())
        {
            assert!(lookup(name).is_some_and(|r| r.implemented), "{name} row");
            assert!(resolve(name, true, m), "{name} must be available in {m:?}");
        }
        for name in UNIMPLEMENTED {
            assert!(
                lookup(name).is_some_and(|r| !r.implemented),
                "{name} must stay unimplemented"
            );
        }
    }
}
