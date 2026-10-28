//! The GCC 16.2 attribute-name registry.
//!
//! `-Wattributes` is only useful if the compiler can tell three different
//! situations apart, and GCC does:
//!
//! * **implemented** — the attribute has semantics here (table `IMPLEMENTED`);
//! * **benign** — GCC accepts it and its effect is confined to diagnostics or
//!   to an optimisation the compiler is free not to perform. GCC prints no
//!   diagnostic for these and neither do we (table `BENIGN`); dropping them
//!   cannot change the meaning of a correct program;
//! * **unimplemented** — GCC accepts it and it *does* change the ABI, the
//!   object layout, the object placement or a caller-visible guarantee. We do
//!   not implement it, so silence would be a lie: `-Wattributes` reports
//!   `'<name>' attribute ignored`, which is exactly the class of silent
//!   semantic divergence pillar 1 forbids (measured: `ms_abi` silently changed
//!   the calling convention and GCC↔lccc cross-object calls returned garbage).
//!
//! A name in none of the tables is not a GCC attribute at all: GCC answers
//! `'<name>' attribute directive ignored`, and so do we.
//!
//! The `IMPLEMENTED` and `UNIMPLEMENTED` membership is enforced by
//! `tests/regression/check_attribute_diagnostics.sh`, which also pins the
//! benign table so that a newly added GCC attribute cannot slip in unnoticed:
//! adding a name to `BENIGN` requires a note saying why ignoring it is safe.

/// What the compiler does with an attribute it recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Semantics implemented by the parser and/or the backend.
    Implemented,
    /// Accepted and ignored: the effect is diagnostic-only or an optimisation,
    /// so ignoring it cannot change a correct program's meaning.
    Benign,
    /// Not implemented, and ignoring it changes the program: diagnosed.
    Unimplemented,
}

/// Attributes this compiler implements with real semantics.
///
/// Membership rule, applied name by name against the dispatcher: the attribute
/// must reach a `ParsedDeclAttrs`/`DeclAttributes` flag or an IR/`backend` fact
/// that changes code, layout, placement or linkage. An attribute whose only
/// effect is a diagnostic GCC may or may not raise, or an optimisation the
/// compiler is free to skip, is `BENIGN` instead — claiming it here would be
/// the over-claim this table exists to prevent.
///
/// Audited 2026-10-28: `access`, `alloc_align`, `alloc_size`, `counted_by`,
/// `deprecated`, `fallthrough`, `format`, `format_arg`, `leaf`, `malloc`,
/// `maybe_unused`, `nodiscard`, `nonnull`, `nothrow`, `returns_nonnull`,
/// `sentinel`, `unavailable`, `unused` and `warn_unused_result` appeared only in
/// the `__has_attribute` answer list, so they were moved to `BENIGN`.
/// `maybe_unused`/`unused` *do* set `ParsedDeclAttrs::maybe_unused`, but that
/// flag only suppresses a diagnostic this compiler never emits, so the flag
/// still has no observable effect.
const IMPLEMENTED: &[&str] = &[
    "alias",
    "aligned",
    "always_inline",
    "cleanup",
    "cold",
    "common",
    "const",
    "constructor",
    "destructor",
    "error",
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
    "nocommon",
    "noinit",
    "noinline",
    "noreturn",
    "optimize",
    "packed",
    "persistent",
    "pure",
    "regparm",
    "reproducible",
    "retain",
    "scalar_storage_order",
    "section",
    "stack_protect",
    "symver",
    "target",
    "target_clones",
    "tls_model",
    "transparent_union",
    "uninitialized",
    "unsequenced",
    "used",
    "vector_size",
    "visibility",
    "warning",
    "weak",
    "weakref",
    "zero_call_used_regs",
];

/// GCC attributes this compiler accepts and drops.
///
/// Every entry was audited against the dispatcher: none of them has an arm that
/// changes code, layout or placement, and none has a semantics that a correct
/// program can observe. Their effect is confined to
///
/// * a diagnostic GCC may raise (`format`, `nonnull`, `warn_unused_result`,
///   `deprecated`, `nodiscard`, `sentinel`, `access`, `alloc_size`,
///   `alloc_align`, `counted_by`, `unavailable`, `fallthrough`, `unused`,
///   `maybe_unused`, `warn_if_not_aligned`), or
/// * an optimisation the compiler is free to skip (`malloc`, `leaf`, `nothrow`,
///   `hot`, `flatten`, `noclone`, `noipa`, `no_reorder`, `no_icf`, ...).
///
/// Dropping them cannot change what a correct program computes, which is why
/// GCC prints no diagnostic for them either.
const BENIGN: &[&str] = &[
    "access",
    "alloc_align",
    "alloc_size",
    "artificial",
    "assume",
    "assume_aligned",
    "btf_decl_tag",
    "btf_type_tag",
    "copy",
    "counted_by",
    "deprecated",
    "designated_init",
    "expected_throw",
    "fallthrough",
    "fd_arg",
    "fd_arg_read",
    "fd_arg_write",
    "flag_enum",
    "flatten",
    "format",
    "format_arg",
    "hot",
    "ignored",
    "leaf",
    "malloc",
    "may_alias",
    "maybe_unused",
    "no_address_safety_analysis",
    "no_icf",
    "no_profile_instrument_function",
    "no_reorder",
    "no_sanitize_address",
    "no_sanitize_coverage",
    "no_sanitize_thread",
    "no_split_stack",
    "no_stack_limit",
    "noclone",
    "nodirect_extern_access",
    "nodiscard",
    "noipa",
    "nonnull",
    "nonnull_if_nonzero",
    "nonstring",
    "noplt",
    "nothrow",
    "null_terminated_string_arg",
    "objc_nullability",
    "objc_root_class",
    "returns_nonnull",
    "sentinel",
    "strict_flex_array",
    "sysv_abi",
    "tainted_args",
    "transaction_pure",
    "transaction_safe",
    "unavailable",
    "unused",
    "unused_result",
    "warn_if_not_aligned",
    "warn_unused",
    "warn_unused_result",
];

/// GCC attributes that change code generation, layout, placement or an ABI
/// contract, and that this compiler does not implement.
///
/// Accepting these without a diagnostic silently changes the meaning of the
/// program (or, worse, of the object files it links against), so they are
/// reported through `-Wattributes` as `'<name>' attribute ignored`.
///
/// Membership rule, applied name by name: an attribute belongs here only if
/// ignoring it can change what the program *computes* or how it links. A
/// calling-convention attribute (`ms_abi`, `cdecl`, `stdcall`, `thiscall`,
/// `sseregparm`, `callee_pop_aggregate_return`) changes the ABI, a layout
/// attribute (`ms_struct`, `gcc_struct`, `signed_bool_precision`,
/// `vector_mask`) changes object layout, `force_align_arg_pointer`,
/// `interrupt`, `naked`-like prologue attributes change the entry sequence,
/// and `returns_twice`, `strub`, `musttail`, `patchable_function_entry`,
/// `cf_check`, `nocf_check`, `indirect_branch`, `preserve_none`,
/// `no_caller_saved_registers`, `no_callee_saved_registers`, `hardbool`,
/// `simd`, `target_version` and `ms_hook_prologue` each change what the
/// generated code may assume about itself or its caller.
const UNIMPLEMENTED: &[&str] = &[
    "callee_pop_aggregate_return",
    "cdecl",
    "cf_check",
    "force_align_arg_pointer",
    "gcc_struct",
    "hardbool",
    "indirect_branch",
    "indirect_return",
    "interrupt",
    "ms_abi",
    "ms_hook_prologue",
    "ms_struct",
    "musttail",
    "no_callee_saved_registers",
    "no_caller_saved_registers",
    "nocf_check",
    "patchable_function_entry",
    "preserve_none",
    "returns_twice",
    "signed_bool_precision",
    "simd",
    "sseregparm",
    "stdcall",
    "strub",
    "target_version",
    "thiscall",
    "vector_mask",
];

/// Strip the `__name__` spelling GCC allows on both sides.
pub fn normalize(name: &str) -> &str {
    name.strip_prefix("__")
        .and_then(|rest| rest.strip_suffix("__"))
        .unwrap_or(name)
}

/// Classify an attribute name as GCC 16.2 would, as far as this compiler is
/// concerned. `None` means the name is not a GCC attribute.
pub fn classify(name: &str) -> Option<Disposition> {
    let name = normalize(name);
    if IMPLEMENTED.contains(&name) {
        Some(Disposition::Implemented)
    } else if BENIGN.contains(&name) {
        Some(Disposition::Benign)
    } else if UNIMPLEMENTED.contains(&name) {
        Some(Disposition::Unimplemented)
    } else {
        None
    }
}

/// Whether the name is a GCC 16.2 attribute this compiler accepts without a
/// diagnostic (implemented or benign). Used by the C23 `[[...]]` path, which
/// reports unknown attributes itself.
pub fn accepted_silently(name: &str) -> bool {
    matches!(
        classify(name),
        Some(Disposition::Implemented) | Some(Disposition::Benign)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_strips_double_underscores() {
        assert_eq!(normalize("__packed__"), "packed");
        assert_eq!(normalize("packed"), "packed");
        assert_eq!(normalize("__packed"), "__packed");
    }

    #[test]
    fn classification_covers_the_three_classes() {
        assert_eq!(classify("packed"), Some(Disposition::Implemented));
        assert_eq!(classify("__nothrow__"), Some(Disposition::Benign));
        assert_eq!(classify("ms_abi"), Some(Disposition::Unimplemented));
        assert_eq!(classify("frobnicate"), None);
    }

    #[test]
    fn tables_do_not_overlap() {
        for (a, b) in [
            (IMPLEMENTED, BENIGN),
            (IMPLEMENTED, UNIMPLEMENTED),
            (BENIGN, UNIMPLEMENTED),
        ] {
            for name in a {
                assert!(!b.contains(name), "{name} is in two tables");
            }
        }
    }

    #[test]
    fn tables_are_sorted_and_unique() {
        for table in [IMPLEMENTED, BENIGN, UNIMPLEMENTED] {
            let mut sorted = table.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted, table, "table must be sorted and duplicate-free");
        }
    }
}
