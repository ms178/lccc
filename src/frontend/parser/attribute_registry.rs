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
    "assume_aligned",
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
    "fd_arg",
    "fd_arg_read",
    "fd_arg_write",
    "flag_enum",
    "flatten",
    "force_align_arg_pointer",
    "format",
    "format_arg",
    "gcc_struct",
    "gnu_inline",
    "hardbool",
    "hot",
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
    "nonnull_if_nonzero",
    "nonstring",
    "noplt",
    "noreturn",
    "nothrow",
    "null_terminated_string_arg",
    "objc_nullability",
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
    "sentinel",
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
    "transaction_safe",
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
/// Four implemented Clang extensions (`address_space`, `diagnose_as`,
/// `diagnose_as_builtin`, `ext_vector_type`) are intentionally not GCC-known;
/// classification checks this list first so lccc still accepts them.
const IMPLEMENTED: &[&str] = &[
    "address_space",
    "alias",
    "aligned",
    "always_inline",
    "cleanup",
    "cold",
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
    "nocommon",
    "noinit",
    "noinline",
    "noplt",
    "noreturn",
    "optimize",
    "packed",
    "persistent",
    "pure",
    "regparm",
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
    "unused",
    "used",
    "vector_size",
    "visibility",
    "warning",
    "weak",
    "weakref",
    "zero_call_used_regs",
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
    ("noplt", AttributeTarget::Function),
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
///
/// A convention attribute is never part of that inventory, even though it is
/// absent from [`IMPLEMENTED`]: its semantics are decided by [`abi_effect`]
/// before the classification funnel runs, and "parsed without applying" is a
/// false description of a name that is either already the target's convention
/// or is refused outright.
pub(crate) fn unimplemented_names() -> impl Iterator<Item = &'static str> {
    GCC_KNOWN
        .iter()
        .copied()
        .filter(|n| !IMPLEMENTED.contains(n))
        .filter(|n| !ABI_VERDICTS.iter().any(|(abi, _, _)| abi == n))
}

// ── ABI-selecting attributes ────────────────────────────────────────────────
//
// An attribute that names a *calling convention* is not like the others. Every
// other attribute lccc parses without applying is inert: ignoring `nonnull`
// loses an optimisation, ignoring `ms_abi` loses the ABI. A caller and a callee
// that disagree about where the arguments live still link, still run, and
// return garbage — which is the one failure mode this compiler must never
// produce silently (GP-02, risk R1 in
// `docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md`).
//
// So a convention attribute is classified into exactly one of four effects, and
// each verdict below was *measured* against gcc rather than chosen:
//
//   * [`AbiEffect::Default`] — the attribute names the convention this target
//     already emits, so applying it is applying nothing. That is not a no-op
//     hack: it is the correct implementation, and it is why `sysv_abi` and
//     `cdecl` can be *supported* without touching a single byte of codegen.
//   * [`AbiEffect::GccIgnored`] — gcc itself does not honour the attribute on
//     this target and says so. Matching that sentence is parity; staying quiet
//     would be a divergence in the other direction, and inventing an error
//     would reject code gcc compiles.
//   * [`AbiEffect::OnlyAvailable32Bit`] — gcc's third answer: the name is known,
//     the target is supported, but the attribute is meaningless here and gcc
//     says exactly `'x' attribute only available for 32-bit`.
//   * [`AbiEffect::Unsupported`] — a convention (or a record layout) this
//     compiler does not implement. Refusing with a named error keeps the
//     mismatch out of the object file; the alternative — emitting the default
//     convention under a foreign-ABI declaration — is precisely the R1 defect
//     this table exists to remove.
//
// The choice between `GccIgnored` and `Unsupported` is not a matter of taste:
// an attribute gcc *honours* must never be ignored by lccc (that is a silent
// ABI change), while an attribute gcc *ignores* must not become an error here
// (that rejects working gcc programs). Measured inventory, gcc 12.2.0,
// `-O2 -c` and `-m32 -O2 -c`, each name on a function definition, on a
// function pointer and on a struct definition:
//
//   x86-64                              i686
//   ms_abi            honours it        ms_abi            honours it (sret pops)
//   sysv_abi          silent, is default  sysv_abi        silent, is default
//   cdecl             `attribute ignored`  cdecl           silent, is default
//   stdcall/thiscall  `attribute ignored`  stdcall/thiscall honours them
//   fastcall          `attribute ignored`  fastcall        honours it (implemented)
//   sseregparm        `attribute ignored`  sseregparm      hard error without SSE2
//   regparm           silent, inert        regparm         honours it (implemented)
//   callee_pop_aggregate_return  `only available for 32-bit`  ... honours it
//   force_align_arg_pointer  silent, inert  ...             realigns the prologue
//   no_caller_saved_registers  accepts      ...             accepts
//   no_callee_saved_registers  `directive ignored`  ...     `directive ignored`
//   preserve_none              `directive ignored`  ...     `directive ignored`
//   ms_struct         changes layout      ms_struct       changes layout
//   gcc_struct        silent, is default  gcc_struct      silent, is default
//
// Two of those verdicts deserve their reasoning spelled out, because "just
// ignore it like gcc does" would be wrong for one and "just refuse it" would be
// wrong for the other:
//
//   * `no_callee_saved_registers` / `preserve_none` are *register-preservation*
//     promises, not argument-passing conventions. Ignoring one costs speed and
//     nothing else: lccc already preserves every callee-saved register, which
//     is the stronger side of the contract, so a body compiled by lccc stays
//     correct for a caller that relies on the attribute. gcc ignores both on
//     this target, and the gate re-measures that every run.
//   * `no_caller_saved_registers` is the opposite promise — the *callee* must
//     not clobber caller-saved registers, and the caller may then skip saving
//     them. lccc cannot yet prove that of a body it emits (gcc enforces it by
//     diagnosing forbidden `xmm`/`x87` use), so silently accepting it could
//     hand a caller a clobbered register. It is refused instead of ignored.
//
// Placement matters for the two record-layout names: on a `struct`/`union` tag
// gcc honours `ms_struct` (and measured: the layout really changes — a
// `{char; int:5; int:3;}` record grows from 4 to 8 bytes) and honours
// `gcc_struct` (the default), while on a function it *ignores* both with the
// usual sentence. [`abi_effect`] therefore takes the placement as well.
//
// Provenance: `tests/regression/check_abi_attribute_contract.py` re-measures
// every verdict against the live gcc oracle, in both targets and both
// placements, and compares the result with this table; a gcc that changes its
// mind fails the gate instead of silently redefining what "parity" means.

/// What a convention attribute means on the target being compiled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AbiEffect {
    /// Names the convention this target already emits: supported, silent.
    Default,
    /// gcc ignores it on this target and warns; match the wording exactly.
    GccIgnored,
    /// gcc's "known name, wrong word size" answer.
    OnlyAvailable32Bit,
    /// A convention or layout this compiler does not implement: refuse.
    Unsupported,
    /// A convention lccc genuinely lowers (`IrFunction::regparm` /
    /// `IrFunction::is_fastcall` on i686). The decision is handed back to the
    /// ordinary dispatcher, which is the only place that can emit it; the
    /// table records it so the gate can require gcc to observe the same
    /// behaviour the table claims.
    Implemented,
}

/// The x86 target the convention table is written for.
///
/// The table is an x86 contract: every name in it is an x86 calling
/// convention, and nowhere else does gcc or lccc have a meaning for it. Other
/// backends keep classifying these names through the generic inventory (which
/// is what they did before this table existed) rather than inheriting an x86
/// verdict; their own parity work owns them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AbiTarget {
    /// x86-64 SysV.
    X86_64,
    /// i686 SysV.
    I686,
    /// Any other back-end.
    Other,
}

/// The current compilation target, in this table's terms.
pub(crate) fn abi_target() -> AbiTarget {
    match crate::common::types::target_elf_machine() {
        62 => AbiTarget::X86_64,
        3 => AbiTarget::I686,
        _ => AbiTarget::Other,
    }
}

/// Where the attribute was written, for the names whose meaning depends on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AbiPlacement {
    /// On a `struct`/`union` tag (`struct __attribute__((ms_struct)) S {...}`).
    Record,
    /// Anywhere else: function, pointer, object, member.
    Other,
}

/// The verdicts, measured. Columns are x86-64 and i686; the two record-layout
/// names carry their record-tag verdict here and their elsewhere verdict in
/// [`abi_effect`], which is the only place placement is consulted.
pub(crate) const ABI_VERDICTS: &[(&str, AbiEffect, AbiEffect)] = &[
    ("ms_abi", AbiEffect::Unsupported, AbiEffect::Unsupported),
    ("sysv_abi", AbiEffect::Default, AbiEffect::Default),
    ("cdecl", AbiEffect::GccIgnored, AbiEffect::Default),
    ("stdcall", AbiEffect::GccIgnored, AbiEffect::Unsupported),
    ("thiscall", AbiEffect::GccIgnored, AbiEffect::Unsupported),
    ("fastcall", AbiEffect::GccIgnored, AbiEffect::Implemented),
    ("sseregparm", AbiEffect::GccIgnored, AbiEffect::Unsupported),
    ("regparm", AbiEffect::Default, AbiEffect::Implemented),
    (
        "callee_pop_aggregate_return",
        AbiEffect::OnlyAvailable32Bit,
        AbiEffect::Unsupported,
    ),
    (
        "force_align_arg_pointer",
        AbiEffect::Default,
        AbiEffect::Unsupported,
    ),
    (
        "no_caller_saved_registers",
        AbiEffect::Unsupported,
        AbiEffect::Unsupported,
    ),
    (
        "no_callee_saved_registers",
        AbiEffect::GccIgnored,
        AbiEffect::GccIgnored,
    ),
    (
        "preserve_none",
        AbiEffect::GccIgnored,
        AbiEffect::GccIgnored,
    ),
    ("ms_struct", AbiEffect::Unsupported, AbiEffect::Unsupported),
    ("gcc_struct", AbiEffect::Default, AbiEffect::Default),
];

/// The convention each ABI attribute selects, in the words the diagnostic uses.
pub(crate) fn abi_convention(name: &str) -> &'static str {
    match name {
        "ms_abi" => "the Microsoft x64 calling convention",
        "stdcall" | "thiscall" | "fastcall" => "a callee-pops calling convention",
        "sseregparm" => "the SSE register-argument convention",
        "ms_struct" => "Microsoft bit-field layout",
        "force_align_arg_pointer" => "a realigning prologue",
        "no_caller_saved_registers" => "the no-caller-saved-registers contract",
        "callee_pop_aggregate_return" => "a callee-pops aggregate return",
        _ => "a calling convention",
    }
}

/// Classify an ABI-selecting attribute, or `None` for every other name.
///
/// `is_32bit` is the compilation target, not the host: every verdict differs
/// per target because gcc's do. `placement` separates the record-tag meaning of
/// the two layout names from their meaningless-elsewhere one.
///
/// `None` is returned for the two conventions lccc genuinely implements —
/// `regparm` and `fastcall` on i686 — so that the ordinary dispatch reaches
/// them; intercepting an implemented convention here would replace working
/// codegen with a diagnostic.
pub(crate) fn abi_effect(
    name: &str,
    target: AbiTarget,
    placement: AbiPlacement,
) -> Option<AbiEffect> {
    let verdict = match target {
        AbiTarget::Other => return None,
        AbiTarget::X86_64 => {
            let (_, verdict, _) = lookup(name)?;
            *verdict
        }
        AbiTarget::I686 => {
            let (_, _, verdict) = lookup(name)?;
            *verdict
        }
    };
    // An implemented convention belongs to the dispatcher: it lowers into the
    // IR (`IrFunction::regparm`, `is_fastcall`) and from there into the i686
    // call ABI, so intercepting it here would replace working codegen with a
    // diagnostic.
    if verdict == AbiEffect::Implemented {
        return None;
    }
    // The layout names only mean something on a struct/union tag; gcc ignores
    // them everywhere else with the ordinary sentence (measured, -Wattributes).
    if placement == AbiPlacement::Other && matches!(canonical(name), "ms_struct" | "gcc_struct") {
        return Some(AbiEffect::GccIgnored);
    }
    Some(verdict)
}

/// Whether `name` carries a convention verdict at all.
///
/// Split out from [`abi_effect`] so the parser can reject the 99.9% of
/// attributes that are not conventions without reading the target out of its
/// thread-local (one TLS load per attribute would be a measurable cost in a
/// header-heavy translation unit, and this predicate is a compare against
/// fifteen static strings).
pub(crate) fn is_abi_attribute(name: &str) -> bool {
    lookup(name).is_some()
}

/// The table row for a name, in either spelling.
fn lookup(name: &str) -> Option<&'static (&'static str, AbiEffect, AbiEffect)> {
    let bare = canonical(name);
    ABI_VERDICTS.iter().find(|(n, _, _)| *n == bare)
}

/// Whether gcc's wording for an ignored ABI attribute is the *unknown-name*
/// sentence (`directive ignored`) rather than the known-but-ignored one.
///
/// Measured: gcc answers `'preserve_none' attribute directive ignored` and
/// `'no_callee_saved_registers' attribute directive ignored`, because neither
/// name exists as a register contract on these targets.
pub(crate) fn abi_gcc_directive_ignored(name: &str) -> bool {
    matches!(
        canonical(name),
        "preserve_none" | "no_callee_saved_registers"
    )
}
