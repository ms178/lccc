//! Explicit contracts for options not handled by the stateful CLI parser.
//! Every accepted spelling is either *implemented* (has an observable effect)
//! or *inert* (accepted for compatibility, provably no effect).  No semantic,
//! ABI or hardening request may be an implicit compatibility noop: that is
//! GP-22's option-effect ledger.
//!
//! The `probe_id` names the regression-gate probe that observes the effect
//! (a compiled fixture's artifact, section list, symbol, or diagnostic line).
//! An `Inert` entry without a `probe_id` is a ledger hole — the gate fails it.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    /// Accepted and implemented: the option changes codegen, diagnostics, or
    /// output in an observable way.
    Implemented,
    /// Accepted but provably inert: the spelling is tolerated for GCC
    /// compatibility but lccc performs no action.  Every `Inert` entry must
    /// carry a `probe_id` naming the gate that verifies the inertness.
    Inert,
    /// Refused: the option is rejected with a diagnostic naming it.
    Refused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Contract {
    pub spelling: &'static str,
    pub class: Class,
    /// The observable effect, or why the option is inert/refused.
    pub rationale: &'static str,
    /// Probe id for the regression gate that observes this option's effect.
    /// Required for `Inert`; optional for `Implemented`; unused for `Refused`.
    pub probe_id: Option<&'static str>,
}

pub(super) const CONTRACTS: &[Contract] = &[
    // --- Refused: not implemented, rejected by name ---
    Contract {
        spelling: "-freg-struct-return",
        class: Class::Refused,
        rationale: "Changing the aggregate return ABI is not implemented.",
        probe_id: None,
    },
    Contract {
        spelling: "-fopenmp",
        class: Class::Implemented,
        // LCCC implements OpenMP's SINGLE-THREAD SERIAL execution model:
        // every `#pragma omp` directive is a no-op, which is a conforming
        // OpenMP execution (one thread, one team).  Measured gcc-16 16.2.0
        // emits no GOMP_* call at all for the gcc.c-torture/compile/pr89235.c
        // `declare simd` shape that this option gate used to reject.  Directives
        // that need a device or a runtime entry point (target/offload,
        // omp_get_*) are NOT silently parallelised: the generated program is
        // the serial program, and any runtime symbol is resolved by the
        // linker exactly as for any other external function.  This is a real
        // observable effect (`_OPENMP 202111`, measured against gcc 16.2),
        // not an inert compatibility no-op.
        rationale: "Serial OpenMP execution model: _OPENMP=202111 and directives lower to their single-thread equivalents; no GOMP runtime linkage.",
        probe_id: Some("probe-fopenmp-serial"),
    },
    Contract {
        spelling: "-fanalyzer",
        class: Class::Refused,
        rationale: "The requested static analyzer is not implemented.",
        probe_id: None,
    },
    Contract {
        spelling: "-fsplit-stack",
        class: Class::Refused,
        rationale: "Split-stack instrumentation is not implemented.",
        probe_id: None,
    },
    // --- Inert: accepted for compatibility, provably no effect ---
    Contract {
        spelling: "-fno-lto",
        class: Class::Inert,
        rationale: "Objects contain native code, not LTO IR.",
        probe_id: Some("probe-lto-no-artifact"),
    },
    Contract {
        spelling: "-fsanitize=none",
        class: Class::Inert,
        rationale: "No sanitizer instrumentation is emitted.",
        probe_id: Some("probe-sanitize-none-no-artifact"),
    },
    Contract {
        spelling: "-fno-strict-aliasing",
        class: Class::Inert,
        rationale: "Alias analysis does not use type-based alias exclusions.",
        probe_id: Some("probe-strict-aliasing-no-effect"),
    },
    Contract {
        spelling: "-fsemantic-interposition",
        class: Class::Inert,
        rationale: "Semantic interposition is the default assumption; nothing to change.",
        probe_id: Some("probe-interposition-default"),
    },
    Contract {
        spelling: "-fno-semantic-interposition",
        class: Class::Inert,
        rationale: "The conservative interposition assumption is retained: correct code, missed optimisation only.",
        probe_id: Some("probe-interposition-conservative"),
    },
    // --- GP-22 priority set ----------------------------------------------
    // These two are REFUSED, not inert: cli.rs's explicit above-the-fold arm
    // produces a diagnostic naming the missing gcov emission.  The previous
    // `Inert` rows contradicted that observable and gave the ledger a false
    // "accepted with no effect" claim.  scripts/option_effect_ledger.py
    // executes every row against the built compiler, so this cannot drift
    // back silently.
    Contract {
        spelling: "-ftest-coverage",
        class: Class::Refused,
        rationale: "gcov coverage is not implemented: no .gcno/.gcda and no __gcov_* counters (GP-13).",
        probe_id: None,
    },
    Contract {
        spelling: "-fprofile-arcs",
        class: Class::Refused,
        rationale: "gcov coverage is not implemented: no .gcno/.gcda and no __gcov_* counters (GP-13).",
        probe_id: None,
    },
    Contract {
        spelling: "-Wattributes",
        class: Class::Inert,
        rationale: "Never fires — attribute diagnostics come through GP-01's registry instead.",
        probe_id: Some("probe-Wattributes-inert"),
    },
    Contract {
        spelling: "-Ofast",
        class: Class::Implemented,
        rationale: "-O3 with the full -ffast-math umbrella: __FAST_MATH__=1, FP reassociation, fast contraction; a packed reduction vectorizes.",
        probe_id: Some("probe-Ofast-fast-math"),
    },
    // --- Origin-aware fast-math sub-options (2026-11-06) --------------------
    // These three have an observable effect in ISOLATION, so the ledger's
    // single-option probe can verify them on each of the four arch drivers.
    // Their counterparts (-fno-fast-math, -fno-finite-math-only,
    // -fmath-errno, -fno-math-errno's inverse, -f{no-,}rounding-math,
    // -f{no-,}trapping-math, -f{no-,}associative-math) are explicit cli.rs
    // arms whose effect is only visible relative to a preceding fast-math
    // umbrella; declaring them Inert here would be a false claim. Their
    // order-dependent state machine is pinned exhaustively by
    // tests/corpus/test_fast_math_contracts.py (44 flag vectors against
    // GCC 16.2 and Clang 23.1.5) instead.
    Contract {
        spelling: "-ffast-math",
        class: Class::Implemented,
        rationale: "Hard umbrella: __FAST_MATH__=1, __FINITE_MATH_ONLY__=1, __NO_MATH_ERRNO__=1 and FP reassociation/vectorization.",
        probe_id: Some("probe-ffast-math-macro"),
    },
    Contract {
        spelling: "-ffinite-math-only",
        class: Class::Implemented,
        rationale: "Assumes no NaN/Inf: __FINITE_MATH_ONLY__=1 and folds the four FP classification builtins to constants while keeping argument side effects.",
        probe_id: Some("probe-finite-math-only-macro"),
    },
    Contract {
        spelling: "-fno-math-errno",
        class: Class::Implemented,
        rationale: "Sets __NO_MATH_ERRNO__=1 and disables errno-setting math calls.",
        probe_id: Some("probe-no-math-errno-macro"),
    },
    Contract {
        spelling: "-Og",
        class: Class::Implemented,
        rationale: "-O1 codegen with optimize_debug=true and frame-pointer omission: __OPTIMIZE__=1; distinct from -O2 on the probe.",
        probe_id: Some("probe-Og-debug-opt"),
    },
    Contract {
        spelling: "-static-pie",
        class: Class::Refused,
        rationale: "Static PIE link layout requires special crt objects and is not supported by the driver.",
        probe_id: None,
    },
    Contract {
        spelling: "-pedantic",
        class: Class::Inert,
        rationale: "Accepted but does nothing until GP-10 implements ISO pedwarns.",
        probe_id: Some("probe-pedantic-inert"),
    },
    // The kernel passes this to every translation unit. Nothing here derives
    // null-check facts from dereference UB in either direction, so the state
    // the flag asks for is already the state — the *inverse* request
    // (`-fdelete-null-pointer-checks`, delete the guards) is the one that must
    // stay unimplemented, and it stays on cli.rs's tolerated path because it
    // deliberately has no row here.
    Contract {
        spelling: "-fno-delete-null-pointer-checks",
        class: Class::Inert,
        rationale: "No dereference-implies-nonnull fold exists, so no null guard is ever deleted; the requested state is the emitted state.",
        probe_id: Some("probe-nullptr-checks-inert"),
    },
];

/// Look up an option's contract.  Returns `None` for spellings not in the
/// ledger — those flow through the tolerated/unknown-option path in `cli.rs`.
pub(super) fn lookup(arg: &str) -> Option<Contract> {
    if let Some(c) = CONTRACTS.iter().find(|c| c.spelling == arg) {
        return Some(*c);
    }
    let rationale = if arg == "-flto" || arg.starts_with("-flto=") {
        "LTO IR production and link-time consumption are not implemented."
    } else if arg.starts_with("-fsanitize-recover")
        || arg.starts_with("-fsanitize-undefined")
        || arg == "-fno-sanitize"
        || arg.starts_with("-fno-sanitize=")
        || arg.starts_with("-fno-sanitize-recover")
        || arg.starts_with("-fno-sanitize-trap")
        || arg.starts_with("-fno-sanitize-undefined")
    {
        // Configuration OF sanitizer behaviour, or disabling sanitizer
        // instrumentation: with no sanitizer enabled by default, the
        // requested state is already the state.
        return Some(Contract {
            spelling: "option family",
            class: Class::Inert,
            rationale: "No sanitizer instrumentation exists to configure.",
            probe_id: Some("probe-sanitize-config-inert"),
        });
    } else if arg.starts_with("-fsanitize") {
        "The requested sanitizer instrumentation is not implemented."
    } else if arg.starts_with("-fno-profile")
        || matches!(
            arg,
            "-fno-auto-profile" | "-fno-test-coverage" | "-fno-branch-probabilities"
        )
    {
        "Disabling individual profile modes is not implemented."
    } else if arg.starts_with("-finstrument-functions-exclude-file-list")
        || arg.starts_with("-finstrument-functions-exclude-function-list")
    {
        "Function-instrumentation exclusion lists are not implemented."
    } else {
        return None;
    };
    Some(Contract {
        spelling: "option family",
        class: Class::Refused,
        rationale,
        probe_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn driver_option_contract_table() {
        for c in CONTRACTS {
            assert_eq!(lookup(c.spelling).unwrap().class, c.class);
            assert!(!c.rationale.is_empty());
        }
        for flag in [
            "-flto",
            "-flto=auto",
            "-fsanitize=address",
            "-fno-profile-arcs",
            "-fno-test-coverage",
            "-fno-branch-probabilities",
            "-fno-auto-profile",
            "-finstrument-functions-exclude-file-list=generated.c",
            "-finstrument-functions-exclude-function-list=helper",
        ] {
            assert_eq!(lookup(flag).unwrap().class, Class::Refused, "{flag}");
        }
        for flag in [
            "-fno-sanitize=all",
            "-fno-sanitize=address",
            "-fno-sanitize=undefined",
            "-fno-sanitize-recover=all",
        ] {
            assert_eq!(lookup(flag).unwrap().class, Class::Inert, "{flag}");
        }
        // The kernel's off-request is a ledger row, not just a bare match
        // arm: the row is what makes GP-22 account for a flag every
        // translation unit of a kernel build passes. Its probe is the guard
        // ratchet in tests/regression/check_nullptr_check_policy.sh.
        assert_eq!(
            lookup("-fno-delete-null-pointer-checks").unwrap().class,
            Class::Inert
        );
        assert!(lookup("-fdefinitely-not-a-flag").is_none());
        // Every accepted option carries a probe id; a refused one must not
        // (there is no effect to observe).  scripts/option_effect_ledger.py
        // additionally executes each row against the built compiler, so a
        // row that merely *looks* right cannot survive.
        let mut probes = std::collections::HashSet::new();
        for c in CONTRACTS {
            match c.class {
                Class::Inert | Class::Implemented => assert!(
                    c.probe_id.is_some(),
                    "{}: accepted contract needs a probe_id",
                    c.spelling
                ),
                Class::Refused => assert!(
                    c.probe_id.is_none(),
                    "{}: refused contract must not carry a probe_id",
                    c.spelling
                ),
            }
            if let Some(probe) = c.probe_id {
                assert!(probes.insert(probe), "duplicate probe_id {probe}");
            }
        }
        // The five rows corrected by the GP-22 executable-ledger audit.  A
        // classification that contradicts observable behaviour is a lie in
        // the trust ledger, so it is pinned here as well as in the gate.
        for flag in ["-ftest-coverage", "-fprofile-arcs", "-static-pie"] {
            assert_eq!(lookup(flag).unwrap().class, Class::Refused, "{flag}");
        }
        for flag in ["-Ofast", "-Og"] {
            assert_eq!(lookup(flag).unwrap().class, Class::Implemented, "{flag}");
        }
    }
}
