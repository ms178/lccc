//! The GCC 16.2 attribute registry: which names GCC knows, which ones lccc
//! actually applies, and where each implemented attribute may appear.
//!
//! Every entry is measured against gcc-16.2.0 by
//! `tests/regression/check_attribute_diagnostics.py`, which also pins the
//! generated inventory of GCC-known-but-unimplemented attributes (the list
//! only shrinks). Classification drives the two GCC `-Wattributes` phrasings:
//!
//! * unknown name -> `'name' attribute directive ignored`
//! * known but misapplied, or known-but-unimplemented (under an explicit
//!   `-Wattributes`) -> `'name' attribute ignored`
//!
//! Names written `__name__` are accepted as their bare spelling, matching
//! the dispatcher's paired arms.

/// Where an attribute may appear (measured on gcc-16.2.0).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AttributeTarget {
    /// Valid on declarations of any kind.
    Any,
    /// Functions only (a non-function warns `'name' attribute ignored`;
    /// `naked` uses gcc's dedicated `only applies to functions` wording).
    Function,
    /// `struct`/`union` definitions only.
    Record,
    /// Objects (variables) only; function declarations warn.
    Object,
}

/// The three GCC 16.2 `-Wattributes` classes lccc distinguishes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AttributeClass {
    /// GCC knows no such attribute: `directive ignored` (default-on).
    Unknown,
    /// GCC knows it; lccc parses it without applying it. Diagnosed under an
    /// explicit `-Wattributes`/`-Werror=attributes` request, which is what
    /// keeps the generated inventory visible without inventing warnings GCC
    /// does not emit on valid uses.
    Unimplemented,
    /// lccc applies it; `target` may still reject a misplacement.
    Implemented,
}

/// Names gcc-16.2.0 accepts (no `directive ignored` on a well-formed probe),
/// measured 2026-10-28 by `attrgen` over the candidate union of the parity
/// assessment's inventory and the dispatcher's arms. The gate re-verifies
/// every name against a live gcc-16.
const GCC_KNOWN: &[&str] = &[
    "access",
    "alias",
    "aligned",
    "alloc_align",
    "alloc_size",
    "always_inline",
    "artificial",
    "assume",
    "btf_decl_tag",
    "btf_type_tag",
    "callee_pop_aggregate_return",
    "cdecl",
    "cf_check",
    "cleanup",
    "cold",
    "common",
    "const",
    "constructor",
    "copy",
    "counted_by",
    "deprecated",
    "designated_init",
    "destructor",
    "error",
    "expected_throw",
    "externally_visible",
    "fallthrough",
    "fastcall",
    "flag_enum",
    "flatten",
    "force_align_arg_pointer",
    "format",
    "format_arg",
    "gcc_struct",
    "gnu_inline",
    "hardbool",
    "ifunc",
    "indirect_branch",
    "indirect_return",
    "interrupt",
    "leaf",
    "malloc",
    "may_alias",
    "mode",
    "ms_abi",
    "ms_hook_prologue",
    "ms_struct",
    "musttail",
    "naked",
    "no_address_safety_analysis",
    "no_callee_saved_registers",
    "no_caller_saved_registers",
    "no_icf",
    "no_instrument_function",
    "no_profile_instrument_function",
    "no_reorder",
    "no_sanitize",
    "no_sanitize_address",
    "no_sanitize_coverage",
    "no_sanitize_thread",
    "no_sanitize_undefined",
    "no_split_stack",
    "no_stack_limit",
    "no_stack_protector",
    "nocf_check",
    "noclone",
    "nocommon",
    "nodirect_extern_access",
    "noinit",
    "noinline",
    "noipa",
    "nonnull",
    "nonstring",
    "noplt",
    "noreturn",
    "nothrow",
    "null_terminated_string_arg",
    "objc_root_class",
    "optimize",
    "packed",
    "patchable_function_entry",
    "persistent",
    "preserve_none",
    "pure",
    "regparm",
    "reproducible",
    "retain",
    "returns_nonnull",
    "returns_twice",
    "scalar_storage_order",
    "section",
    "signed_bool_precision",
    "simd",
    "sseregparm",
    "stack_protect",
    "stdcall",
    "strict_flex_array",
    "strub",
    "symver",
    "sysv_abi",
    "tainted_args",
    "target",
    "target_clones",
    "target_version",
    "thiscall",
    "tls_model",
    "transaction_callable",
    "transaction_may_cancel_outer",
    "transaction_pure",
    "transaction_safe_dynamic",
    "transaction_unsafe",
    "transparent_union",
    "unavailable",
    "uninitialized",
    "unsequenced",
    "unused",
    "used",
    "vector_mask",
    "vector_size",
    "visibility",
    "warn_if_not_aligned",
    "warn_unused",
    "warn_unused_result",
    "warning",
    "weak",
    "weakref",
    "zero_call_used_regs",
];

/// Names the dispatcher applies (the `match` in `parse.rs`, normalized).
/// Anything in [`GCC_KNOWN`] outside this list is [`AttributeClass::Unimplemented`].
const IMPLEMENTED: &[&str] = &[
    "address_space",
    "aligned",
    "always_inline",
    "common",
    "const",
    "constructor",
    "counted_by",
    "destructor",
    "diagnose_as",
    "diagnose_as_builtin",
    "error",
    "ext_vector_type",
    "externally_visible",
    "fastcall",
    "gnu_inline",
    "ifunc",
    "mode",
    "naked",
    "no_instrument_function",
    "no_sanitize",
    "no_sanitize_undefined",
    "no_stack_protector",
    "noreturn",
    "noinline",
    "optimize",
    "packed",
    "pure",
    "regparm",
    "scalar_storage_order",
    "section",
    "stack_protect",
    "symver",
    "target",
    "target_clones",
    "tls_model",
    "transparent_union",
    "uninitialized",
    "unused",
    "used",
    "vector_size",
    "visibility",
    "warning",
    "weak",
    "weakref",
    "zero_call_used_regs",
    "alias",
    "cleanup",
    "cold",
];

/// Measured applicability (gcc-16.2.0, 2026-10-28): each entry warns
/// `'name' attribute ignored` on the wrong target and stays silent on the
/// right one; `naked`'s wrong-target wording is dedicated.
const TARGETS: &[(&str, AttributeTarget)] = &[
    ("constructor", AttributeTarget::Function),
    ("destructor", AttributeTarget::Function),
    ("noreturn", AttributeTarget::Function),
    ("noinline", AttributeTarget::Function),
    ("always_inline", AttributeTarget::Function),
    ("cold", AttributeTarget::Function),
    ("flatten", AttributeTarget::Function),
    ("target", AttributeTarget::Function),
    ("target_clones", AttributeTarget::Function),
    ("const", AttributeTarget::Function),
    ("pure", AttributeTarget::Function),
    ("naked", AttributeTarget::Function),
    ("gnu_inline", AttributeTarget::Function),
    ("packed", AttributeTarget::Record),
    ("cleanup", AttributeTarget::Object),
];

/// Strip the `__name__` spelling to the canonical bare name.
pub(crate) fn canonical(name: &str) -> &str {
    name.strip_prefix("__")
        .and_then(|n| n.strip_suffix("__"))
        .unwrap_or(name)
}

/// Classify a written attribute name for the diagnostic funnel.
pub(crate) fn classify(name: &str) -> AttributeClass {
    let bare = canonical(name);
    if IMPLEMENTED.contains(&bare) {
        AttributeClass::Implemented
    } else if GCC_KNOWN.contains(&bare) {
        AttributeClass::Unimplemented
    } else {
        AttributeClass::Unknown
    }
}

/// The measured placement rule for an implemented attribute.
pub(crate) fn target(name: &str) -> AttributeTarget {
    let bare = canonical(name);
    TARGETS
        .iter()
        .find(|(n, _)| *n == bare)
        .map_or(AttributeTarget::Any, |(_, t)| *t)
}

/// Whether lccc knows the name at all (implemented or inventoried).
pub(crate) fn is_gcc_known(name: &str) -> bool {
    GCC_KNOWN.contains(&canonical(name))
}

/// Names in [`GCC_KNOWN`] that lccc parses without applying: the generated
/// inventory the gate pins (and that `--dump-attribute-inventory` prints).
pub(crate) fn unimplemented_names() -> impl Iterator<Item = &'static str> {
    GCC_KNOWN
        .iter()
        .copied()
        .filter(|n| !IMPLEMENTED.contains(n))
}
