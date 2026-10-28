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
        class: Class::Refused,
        rationale: "OpenMP lowering and runtime linkage are not implemented.",
        probe_id: None,
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
    // --- GP-22 priority inert set (accepted-and-inert must be proven, not assumed) ---
    Contract {
        spelling: "-ftest-coverage",
        class: Class::Inert,
        rationale: "No .gcno/.gcda emission and no __gcov_* symbols (GP-13 covers real gcov).",
        probe_id: Some("probe-test-coverage-inert"),
    },
    Contract {
        spelling: "-fprofile-arcs",
        class: Class::Inert,
        rationale: "No .gcno/.gcda emission and no __gcov_* symbols (GP-13 covers real gcov).",
        probe_id: Some("probe-profile-arcs-inert"),
    },
    Contract {
        spelling: "-Wattributes",
        class: Class::Inert,
        rationale: "Never fires — attribute diagnostics come through GP-01's registry instead.",
        probe_id: Some("probe-Wattributes-inert"),
    },
    Contract {
        spelling: "-Ofast",
        class: Class::Inert,
        rationale: "Beyond the tolerated tier; -O3 is the highest implemented optimisation level.",
        probe_id: Some("probe-Ofast-inert"),
    },
    Contract {
        spelling: "-Og",
        class: Class::Inert,
        rationale: "No debug-aware optimisation mode is implemented; -O0 is the debug baseline.",
        probe_id: Some("probe-Og-inert"),
    },
    Contract {
        spelling: "-static-pie",
        class: Class::Inert,
        rationale: "Produces ET_EXEC (no ASLR), not a static PIE; the warning documents this.",
        probe_id: Some("probe-static-pie-inert"),
    },
    Contract {
        spelling: "-pedantic",
        class: Class::Inert,
        rationale: "Accepted but does nothing until GP-10 implements ISO pedwarns.",
        probe_id: Some("probe-pedantic-inert"),
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
        assert!(lookup("-fdefinitely-not-a-flag").is_none());
        // Every Inert entry must carry a probe_id.
        for c in CONTRACTS {
            if c.class == Class::Inert {
                assert!(
                    c.probe_id.is_some(),
                    "{}: inert contract needs a probe_id",
                    c.spelling
                );
            }
        }
    }
}
