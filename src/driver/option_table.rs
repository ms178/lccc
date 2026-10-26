//! Explicit contracts for options not handled by the stateful CLI parser.
//! No semantic, ABI or hardening request may be an implicit compatibility noop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    BenignNoop,
    Refused,
}

pub(super) struct Contract {
    pub spelling: &'static str,
    pub class: Class,
    pub rationale: &'static str,
}

pub(super) const CONTRACTS: &[Contract] = &[
    Contract {
        spelling: "-freg-struct-return",
        class: Class::Refused,
        rationale: "Changing the aggregate return ABI is not implemented.",
    },
    Contract {
        spelling: "-fno-stack-protector",
        class: Class::BenignNoop,
        rationale: "No stack canaries are emitted.",
    },
    Contract {
        spelling: "-fno-stack-clash-protection",
        class: Class::BenignNoop,
        rationale: "No stack probing instrumentation is emitted.",
    },
    Contract {
        spelling: "-fno-lto",
        class: Class::BenignNoop,
        rationale: "Objects contain native code, not LTO IR.",
    },
    Contract {
        spelling: "-fsanitize=none",
        class: Class::BenignNoop,
        rationale: "No sanitizer instrumentation is emitted.",
    },
    Contract {
        spelling: "-fno-strict-aliasing",
        class: Class::BenignNoop,
        rationale: "Alias analysis does not use type-based alias exclusions.",
    },
    Contract {
        spelling: "-fstack-clash-protection",
        class: Class::Refused,
        rationale: "Stack probing is not implemented.",
    },
    Contract {
        spelling: "-fsemantic-interposition",
        class: Class::BenignNoop,
        rationale: "Semantic interposition is the default assumption; nothing to change.",
    },
    Contract {
        spelling: "-fno-semantic-interposition",
        class: Class::BenignNoop,
        rationale: "The conservative interposition assumption is retained: correct code, missed optimisation only.",
    },
    Contract {
        spelling: "-fopenmp",
        class: Class::Refused,
        rationale: "OpenMP lowering and runtime linkage are not implemented.",
    },
    Contract {
        spelling: "-fanalyzer",
        class: Class::Refused,
        rationale: "The requested static analyzer is not implemented.",
    },
    Contract {
        spelling: "-fsplit-stack",
        class: Class::Refused,
        rationale: "Split-stack instrumentation is not implemented.",
    },
    Contract {
        spelling: "-fwhole-program",
        class: Class::Refused,
        rationale: "Whole-program module merging is not implemented.",
    },
];

pub(super) fn lookup(arg: &str) -> Option<Contract> {
    if let Some(c) = CONTRACTS.iter().find(|c| c.spelling == arg) {
        return Some(Contract {
            spelling: c.spelling,
            class: c.class,
            rationale: c.rationale,
        });
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
            class: Class::BenignNoop,
            rationale: "No sanitizer instrumentation exists to configure.",
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
            assert_eq!(lookup(flag).unwrap().class, Class::BenignNoop, "{flag}");
        }
        assert!(lookup("-fdefinitely-not-a-flag").is_none());
    }
}
