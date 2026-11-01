// Core Parser struct and basic helpers.
//
// The parser is split into focused modules:
//   - expressions.rs: operator precedence climbing (comma through primary)
//   - types.rs: type specifier collection and resolution
//   - statements.rs: all statement types + inline assembly
//   - declarations.rs: external and local declarations, initializers
//   - declarators.rs: C declarator syntax (pointers, arrays, function pointers)
//
// Each module adds methods to the Parser struct via `impl Parser` blocks.
// Methods are pub(super) so they can be called across modules within the parser.

use super::ast::*;
use super::attribute_registry;
use crate::common::error::DiagnosticEngine;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::AddressSpace;
use crate::frontend::lexer::token::{Token, TokenKind};
use std::sync::OnceLock;

/// GCC __attribute__((mode(...))) integer mode specifier.
/// Controls the bit-width of an integer type regardless of the base type keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModeKind {
    QI, // 8-bit (quarter integer)
    HI, // 16-bit (half integer)
    SI, // 32-bit (single integer)
    DI, // 64-bit (double integer)
    TI, // 128-bit (tetra integer)
}

impl ModeKind {
    /// Apply mode to a type specifier, preserving signedness.
    pub(super) fn apply(self, ts: TypeSpecifier) -> TypeSpecifier {
        let is_unsigned = matches!(
            ts,
            TypeSpecifier::UnsignedInt
                | TypeSpecifier::UnsignedLong
                | TypeSpecifier::UnsignedLongLong
                | TypeSpecifier::Unsigned
                | TypeSpecifier::UnsignedChar
                | TypeSpecifier::PlainUnsignedChar
                | TypeSpecifier::UnsignedShort
        );
        match self {
            ModeKind::QI => {
                if is_unsigned {
                    TypeSpecifier::UnsignedChar
                } else if crate::common::types::char_is_unsigned() {
                    TypeSpecifier::Char
                } else {
                    // GCC's c_common_type_for_mode(QImode, signed) is
                    // `signed char`, distinct from plain char.
                    TypeSpecifier::SignedChar
                }
            }
            ModeKind::HI => {
                if is_unsigned {
                    TypeSpecifier::UnsignedShort
                } else {
                    TypeSpecifier::Short
                }
            }
            ModeKind::SI => {
                if is_unsigned {
                    TypeSpecifier::UnsignedInt
                } else {
                    TypeSpecifier::Int
                }
            }
            ModeKind::DI => {
                if is_unsigned {
                    TypeSpecifier::UnsignedLongLong
                } else {
                    TypeSpecifier::LongLong
                }
            }
            ModeKind::TI => {
                if is_unsigned {
                    TypeSpecifier::UnsignedInt128
                } else {
                    TypeSpecifier::Int128
                }
            }
        }
    }
}

/// Bit masks for boolean flags in `ParsedDeclAttrs::flags`.
///
/// Each flag occupies one bit of a `u64`. A wider field keeps new parser-only
/// metadata out of declaration attributes when it has no code-generation role.
pub(super) mod parsed_attr_flag {
    // --- Storage-class specifiers ---
    /// `typedef` keyword encountered.
    pub const TYPEDEF: u64 = 1 << 0;
    /// `static` keyword encountered.
    pub const STATIC: u64 = 1 << 1;
    /// `extern` keyword encountered.
    pub const EXTERN: u64 = 1 << 2;
    /// `_Thread_local` or `__thread` encountered.
    pub const THREAD_LOCAL: u64 = 1 << 3;
    /// `inline` keyword encountered.
    pub const INLINE: u64 = 1 << 4;
    /// GNU `auto` at block scope: introduces a *nested function*, a distinct
    /// entity that shares only a basename with any file-scope symbol. Carried as
    /// a flag because the distinction is a storage-class fact, and the two cases
    /// need opposite handling in sema (merge into the translation-unit record
    /// versus do not touch it). Nothing else in the tree can recover it later:
    /// the type may be identical, which is exactly what a basename collision looks
    /// like before any name lookup.
    pub const NESTED_FN: u64 = 1 << 24;
    /// The declaration's specifiers name a typedef (its qualifiers apply here).
    pub const TYPEDEF_USE: u64 = 1 << 25;
    /// The typedef named by the specifiers has a volatile object qualifier
    /// (`typedef int *volatile P`): the object declared through it is volatile.
    pub const TYPEDEF_OBJECT_VOLATILE: u64 = 1 << 26;

    // --- Type qualifiers ---
    /// `const` qualifier encountered.
    pub const CONST: u64 = 1 << 5;
    /// `volatile` qualifier encountered.
    pub const VOLATILE: u64 = 1 << 6;

    // --- GCC function attributes ---
    /// `__attribute__((constructor))` encountered.
    pub const CONSTRUCTOR: u64 = 1 << 7;
    /// `__attribute__((destructor))` encountered.
    pub const DESTRUCTOR: u64 = 1 << 8;
    /// `__attribute__((weak))` encountered.
    pub const WEAK: u64 = 1 << 9;
    /// `__attribute__((used))` encountered.
    pub const USED: u64 = 1 << 10;
    /// `__attribute__((gnu_inline))` encountered.
    pub const GNU_INLINE: u64 = 1 << 11;
    /// `__attribute__((always_inline))` encountered.
    pub const ALWAYS_INLINE: u64 = 1 << 12;
    /// `__attribute__((noinline))` encountered.
    pub const NOINLINE: u64 = 1 << 13;
    /// `__attribute__((noreturn))` or `_Noreturn` encountered.
    pub const NORETURN: u64 = 1 << 14;
    /// `__attribute__((error("...")))` encountered.
    pub const ERROR_ATTR: u64 = 1 << 15;
    /// `__attribute__((transparent_union))` encountered.
    pub const TRANSPARENT_UNION: u64 = 1 << 16;
    /// `__attribute__((fastcall))` encountered (i386 fastcall calling convention).
    pub const FASTCALL: u64 = 1 << 17;
    /// `__attribute__((naked))` encountered — emit no prologue/epilogue.
    pub const NAKED: u64 = 1 << 18;
    /// `__attribute__((no_instrument_function))` encountered — skip the mcount
    /// / __fentry__ prologue emission. Critical for the Linux kernel's noinstr
    /// functions (early boot, NMI, RCU) where the tracer can't run.
    pub const NO_INSTRUMENT: u64 = 1 << 19;
    /// `__attribute__((pure))` encountered.
    pub const PURE: u64 = 1 << 20;
    /// `__attribute__((const))` encountered.
    pub const CONST_ATTR: u64 = 1 << 21;
    /// A `const` qualifier appeared AFTER a `*` in the declarator, i.e. the
    /// POINTER itself is const (`T *const p`), not the pointee (`const T *p`).
    /// Tracked separately from `CONST` because `Declaration::is_const()`
    /// describes the pointee for pointer declarators.
    pub const POINTER_CONST: u64 = 1 << 22;
    /// `__attribute__((cold))` encountered — the function is unlikely to
    /// execute (error paths, fatal handlers). Lowers to the `.text.unlikely`
    /// section unless an explicit `section(...)` overrides it.
    pub const COLD: u64 = 1 << 23;
    /// C23 `constexpr` storage-class specifier (6.7.1).
    pub const CONSTEXPR: u64 = 1 << 27;
    /// `_Atomic` qualifier or `_Atomic(type-name)` specifier in the
    /// specifiers, or after the `*` being parsed (see `skip_cv_qualifiers`).
    pub const ATOMIC: u64 = 1 << 28;
    /// `restrict` after the `*` being parsed.
    pub const RESTRICT: u64 = 1 << 29;
    /// `constexpr` came after `_Thread_local` or `auto`: GCC 16.2 names the
    /// later of the two first ("'constexpr' used with '_Thread_local'").
    pub const CONSTEXPR_LAST: u64 = 1 << 30;
    /// `__attribute__((uninitialized))` encountered — skip `-ftrivial-auto-var-init`.
    pub const UNINITIALIZED: u64 = 1 << 31;
    /// C23 `[[maybe_unused]]` / GNU `unused` attribute.
    pub const MAYBE_UNUSED: u64 = 1 << 32;
    /// `__attribute__((retain))` — section must survive section GC.
    pub const RETAIN: u64 = 1 << 33;
    /// `__attribute__((noinit))` — object lives in `.noinit`, never zeroed.
    pub const NOINIT: u64 = 1 << 34;
    /// `__attribute__((persistent))` — object lives in `.persistent`.
    pub const PERSISTENT: u64 = 1 << 35;
    /// `__attribute__((nocommon))` — tentative definition is not a common block.
    pub const NO_COMMON_ATTR: u64 = 1 << 36;
    /// `__attribute__((noplt))` — calls to this function go through the GOT
    /// instead of the PLT. Applies to the *declaration* being parsed; GCC only
    /// honours it for functions not defined in the same translation unit
    /// (measured 2026-10-30: a same-TU definition still gets `R_X86_64_PLT32`).
    pub const NOPLT: u64 = 1 << 37;
    /// The storage-class specifiers a declaration carries (C11 6.7.1; GNU
    /// `auto` is NESTED_FN). `register` is not tracked.
    pub const STORAGE_CLASSES: u64 =
        TYPEDEF | STATIC | EXTERN | THREAD_LOCAL | NESTED_FN | CONSTEXPR | CONSTEXPR_LAST;
}

/// Why a `cleanup` attribute cannot be honoured. Both forms are hard errors
/// in gcc-16.2 (measured), reported at the declaration's location.
#[derive(Clone, Copy, Debug)]
pub(super) enum CleanupDiagnosis {
    /// Zero arguments (bare `cleanup` or `cleanup()`) or more than one.
    WrongArgumentCount(usize),
    /// The single argument is not a bare identifier (`cleanup(1)`,
    /// `cleanup("f")`).
    NotAnIdentifier,
}

/// Accumulated storage-class specifiers, type qualifiers, and GCC attributes
/// parsed during declaration/type-specifier processing.
///
/// These fields are set by `parse_type_specifier` and `parse_gcc_attribute` as
/// keywords/attributes are encountered, then consumed and reset by declaration
/// builders in `declarations.rs`. Grouping them into a single struct replaces
/// scattered fields on `Parser`, enabling bulk reset via `Default::default()`
/// and making the "set then consume" lifecycle explicit.
///
/// Boolean attributes are stored as a packed bitfield (`flags`) for memory
/// efficiency. Accessor methods provide the same API as the old struct fields.
#[derive(Default, Clone)]
pub(super) struct ParsedDeclAttrs {
    /// Packed boolean flags — see `parsed_attr_flag` constants.
    pub(super) flags: u64,

    /// Specifier-position attribute records waiting for the
    /// declaration-kind applicability check (see
    /// `attribute_registry::AttributeTarget`); taken by the check or
    /// cleared by the next declaration boundary.
    pub(super) parsing_attr_targets: Vec<(
        String,
        crate::common::source::Span,
        super::attribute_registry::AttributeTarget,
    )>,
    /// Whether the attribute list being parsed sits on a `struct`/`union` tag.
    /// The two record-layout attributes (`ms_struct`, `gcc_struct`) mean
    /// different things there than anywhere else — gcc honours them on a tag
    /// and ignores them with a warning on a function — so the placement has to
    /// reach `attribute_registry::abi_effect`. Raised only around the
    /// tag-level attribute parse (`Parser::parse_record_attributes`).
    pub(super) parsing_record_attributes: bool,

    /// Location of the declarator name being parsed, when it is a plain
    /// identifier. GCC reports `-Wstrict-prototypes` for a *definition* at
    /// the name (`int k() {}` → the `k`), unlike a declaration, which it
    /// reports at the declaration's start.
    pub(super) last_decl_name_span: Option<crate::common::source::Span>,
    /// Location of the FIRST `__attribute__((...))` list of the declaration
    /// being parsed. GCC reports every placement verdict at the declaration's
    /// own location, which includes a leading attribute list: for
    /// `__attribute__((noreturn)) int x;` it points at column 1, not at the
    /// attribute or at `int` (measured on gcc-16.2.0).
    pub(super) first_attr_span: Option<crate::common::source::Span>,

    /// `__seg_gs` or `__seg_fs` qualifier encountered.
    pub parsing_address_space: AddressSpace,

    /// Named space of the object the last declarator names, from the qualifier
    /// after its last `*` (see `DeclAttributes::object_space`). Set by
    /// `parse_declarator_with_attrs`, read right after it.
    pub(super) declarator_object_space: AddressSpace,
    /// `volatile` of the object the last declarator names (see
    /// `DeclAttributes::object_volatile`). Set by `parse_declarator_with_attrs`.
    pub(super) declarator_object_volatile: bool,
    /// `volatile` written in the declaration's specifiers (the base type's
    /// qualifier: the object for a non-pointer, the pointee for a pointer).
    /// Set by `parse_declarator_with_attrs`; read by the typedef registration.
    pub(super) declarator_base_volatile: bool,
    /// The `volatile` flag is set only because a declarator star added it:
    /// the declaration-wide flag keeps the union of the specifiers' and every
    /// star's `volatile` (`int *volatile a, *b;` leaves it set for `b`), and
    /// this tells a later declarator that its specifiers did not write one.
    /// Cleared by every specifier `volatile`.
    pub(super) volatile_from_stars: bool,
    /// `restrict` in the specifiers of the last declarator's declaration (see
    /// `DeclAttributes::base_restrict`). Set by `parse_declarator_with_attrs`.
    pub(super) declarator_base_restrict: bool,
    /// `restrict` / `_Atomic` of the object the last declarator names (see
    /// `DeclAttributes::object_restrict`). Set by `parse_declarator_with_attrs`.
    pub(super) declarator_object_restrict: bool,
    pub(super) declarator_object_atomic: bool,
    /// The CVR bits written inside a parameter's first `[...]` (`int
    /// a[const 2]`), which qualify the pointer it is adjusted to (C23
    /// 6.7.6.3p7). Set by `parse_param_declarator_full`.
    pub(super) param_adjusted_cvr: u8,

    /// Named address space of the declaration being parsed, moved out of
    /// `parsing_address_space` right after its type specifier. Parameter
    /// lists and initializers reuse that slot, so the declaration's own
    /// qualifier must be captured here before any declarator is parsed.
    pub decl_address_space: AddressSpace,

    // --- GCC attributes with values ---
    /// `__attribute__((alias("target")))` target symbol name.
    pub parsing_alias_target: Option<String>,
    /// `__attribute__((ifunc("resolver")))` resolver symbol name.
    pub parsing_ifunc_target: Option<String>,
    /// `__attribute__((visibility("...")))` visibility string.
    pub parsing_visibility: Option<String>,
    /// `__attribute__((tls_model("...")))`, validated at the attribute.
    pub parsing_tls_model: Option<crate::common::tls::TlsModel>,
    /// `__attribute__((section("...")))` section name.
    pub parsing_section: Option<String>,
    /// `__attribute__((cleanup(func)))` cleanup function name.
    pub parsing_cleanup_fn: Option<String>,
    /// `__attribute__((counted_by(member)))` sibling count field name.
    pub parsing_counted_by: Option<String>,
    /// `__attribute__((symver("name@@VERSION")))` symbol version string.
    pub parsing_symver: Option<String>,
    /// `__attribute__((target("...")))` feature list, comma-joined.
    pub parsing_target: Option<String>,
    /// `__attribute__((target_clones("...")))` clone target list, comma-joined.
    pub parsing_target_clones: Option<String>,
    /// `__attribute__((__diagnose_as(__builtin_NAME, ...)))` — the builtin
    /// the declared function is an alias of (the kernel's fortify-string
    /// family).  Calls that survive inlining must fold to the builtin, or
    /// the extern declaration resolves to nothing at link time.
    pub parsing_diagnose_as: Option<String>,
    /// `__attribute__((regparm(N)))` register-argument count.
    pub parsing_regparm: Option<u8>,
    /// `__attribute__((stack_protect))` encountered.
    pub parsing_stack_protect: bool,
    /// `__attribute__((no_stack_protector))` encountered.
    pub parsing_no_stack_protector: bool,
    /// `__attribute__((externally_visible))` encountered.
    pub parsing_externally_visible: bool,
    /// `__attribute__((zero_call_used_regs("...")))` mode string.
    pub parsing_zero_call_used_regs: Option<String>,
    /// `__attribute__((no_sanitize("...")))` / `no_sanitize_undefined` list.
    pub parsing_no_sanitize: Option<String>,
    /// `__attribute__((vector_size(N)))` total vector size in bytes.
    pub parsing_vector_size: Option<usize>,
    /// `__attribute__((vector_size(EXPR)))` whose EXPR was not parse-time
    /// evaluable (e.g. `16 * sizeof(some_typedef)`): the raw expression is
    /// deferred to sema, where typedef sizes are fully resolved.
    pub parsing_vector_size_expr: Option<Box<Expr>>,
    /// `__attribute__((ext_vector_type(N)))` number of vector elements.
    /// Converted to total byte size in lowering using sizeof(element_type) * N.
    pub parsing_ext_vector_nelem: Option<usize>,

    // --- Alignment ---
    /// `_Alignas(N)` or `__attribute__((aligned(N)))` value.
    pub parsed_alignas: Option<usize>,
    /// `_Alignas(type)` type specifier (for deferred alignment resolution).
    pub parsed_alignas_type: Option<TypeSpecifier>,
    /// `__attribute__((aligned(sizeof(type))))` type (for deferred sizeof).
    pub parsed_alignment_sizeof_type: Option<TypeSpecifier>,
}

impl ParsedDeclAttrs {
    // --- flag getters ---

    #[inline]
    pub fn parsing_typedef(&self) -> bool {
        self.flags & parsed_attr_flag::TYPEDEF != 0
    }
    #[inline]
    pub fn parsing_static(&self) -> bool {
        self.flags & parsed_attr_flag::STATIC != 0
    }
    #[inline]
    pub fn parsing_extern(&self) -> bool {
        self.flags & parsed_attr_flag::EXTERN != 0
    }
    #[inline]
    pub fn parsing_nested_fn(&self) -> bool {
        self.flags & parsed_attr_flag::NESTED_FN != 0
    }
    #[inline]
    pub fn parsing_thread_local(&self) -> bool {
        self.flags & parsed_attr_flag::THREAD_LOCAL != 0
    }
    #[inline]
    pub fn parsing_inline(&self) -> bool {
        self.flags & parsed_attr_flag::INLINE != 0
    }
    #[inline]
    pub fn parsing_const(&self) -> bool {
        self.flags & parsed_attr_flag::CONST != 0
    }
    #[inline]
    pub fn parsing_pointer_const(&self) -> bool {
        self.flags & parsed_attr_flag::POINTER_CONST != 0
    }
    #[inline]
    pub fn parsing_volatile(&self) -> bool {
        self.flags & parsed_attr_flag::VOLATILE != 0
    }
    #[inline]
    pub fn parsing_constexpr(&self) -> bool {
        self.flags & parsed_attr_flag::CONSTEXPR != 0
    }
    #[inline]
    pub fn set_constexpr(&mut self, v: bool) {
        // `auto` is a storage class only once a type specifier accompanies
        // it; the specifier parser marks that case (`mark_constexpr_last`).
        let peer = parsed_attr_flag::THREAD_LOCAL;
        self.set_flag(
            parsed_attr_flag::CONSTEXPR_LAST,
            v && self.flags & peer != 0,
        );
        self.set_flag(parsed_attr_flag::CONSTEXPR, v)
    }
    /// `constexpr` follows an `auto` that a type specifier made a storage
    /// class.
    #[inline]
    pub fn mark_constexpr_last(&mut self) {
        self.set_flag(parsed_attr_flag::CONSTEXPR_LAST, true)
    }
    #[inline]
    pub fn parsing_constexpr_last(&self) -> bool {
        self.flags & parsed_attr_flag::CONSTEXPR_LAST != 0
    }
    /// Clear the storage-class specifiers, returning them for
    /// [`Self::restore_storage_classes`]: a parameter's specifiers are its
    /// own, and must neither inherit nor leave behind the enclosing
    /// declaration's.
    #[inline]
    pub fn take_storage_classes(&mut self) -> u64 {
        let bits = self.flags & parsed_attr_flag::STORAGE_CLASSES;
        self.flags &= !parsed_attr_flag::STORAGE_CLASSES;
        bits
    }
    #[inline]
    pub fn restore_storage_classes(&mut self, bits: u64) {
        self.flags = (self.flags & !parsed_attr_flag::STORAGE_CLASSES) | bits;
    }
    #[inline]
    pub fn parsing_atomic(&self) -> bool {
        self.flags & parsed_attr_flag::ATOMIC != 0
    }
    #[inline]
    pub fn set_atomic(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::ATOMIC, v)
    }
    #[inline]
    pub fn parsing_restrict(&self) -> bool {
        self.flags & parsed_attr_flag::RESTRICT != 0
    }
    #[inline]
    pub fn set_restrict(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::RESTRICT, v)
    }
    #[inline]
    pub fn parsing_typedef_use(&self) -> bool {
        self.flags & parsed_attr_flag::TYPEDEF_USE != 0
    }
    #[inline]
    pub fn parsing_typedef_object_volatile(&self) -> bool {
        self.flags & parsed_attr_flag::TYPEDEF_OBJECT_VOLATILE != 0
    }
    #[inline]
    pub fn parsing_constructor(&self) -> bool {
        self.flags & parsed_attr_flag::CONSTRUCTOR != 0
    }
    #[inline]
    pub fn parsing_destructor(&self) -> bool {
        self.flags & parsed_attr_flag::DESTRUCTOR != 0
    }
    #[inline]
    pub fn parsing_weak(&self) -> bool {
        self.flags & parsed_attr_flag::WEAK != 0
    }
    #[inline]
    pub fn parsing_used(&self) -> bool {
        self.flags & parsed_attr_flag::USED != 0
    }
    #[inline]
    pub fn parsing_retain(&self) -> bool {
        self.flags & parsed_attr_flag::RETAIN != 0
    }
    #[inline]
    pub fn set_retain(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::RETAIN, v)
    }
    #[inline]
    pub fn parsing_noplt(&self) -> bool {
        self.flags & parsed_attr_flag::NOPLT != 0
    }
    #[inline]
    pub fn set_noplt(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::NOPLT, v)
    }
    #[inline]
    pub fn parsing_noinit(&self) -> bool {
        self.flags & parsed_attr_flag::NOINIT != 0
    }
    #[inline]
    pub fn set_noinit(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::NOINIT, v)
    }
    #[inline]
    pub fn parsing_persistent(&self) -> bool {
        self.flags & parsed_attr_flag::PERSISTENT != 0
    }
    #[inline]
    pub fn set_persistent(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::PERSISTENT, v)
    }
    #[inline]
    pub fn parsing_nocommon(&self) -> bool {
        self.flags & parsed_attr_flag::NO_COMMON_ATTR != 0
    }
    #[inline]
    pub fn set_nocommon(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::NO_COMMON_ATTR, v)
    }
    #[inline]
    pub fn parsing_maybe_unused(&self) -> bool {
        self.flags & parsed_attr_flag::MAYBE_UNUSED != 0
    }
    #[inline]
    pub fn parsing_gnu_inline(&self) -> bool {
        self.flags & parsed_attr_flag::GNU_INLINE != 0
    }
    #[inline]
    pub fn parsing_always_inline(&self) -> bool {
        self.flags & parsed_attr_flag::ALWAYS_INLINE != 0
    }
    #[inline]
    pub fn parsing_noinline(&self) -> bool {
        self.flags & parsed_attr_flag::NOINLINE != 0
    }
    #[inline]
    pub fn parsing_noreturn(&self) -> bool {
        self.flags & parsed_attr_flag::NORETURN != 0
    }
    #[inline]
    pub fn parsing_error_attr(&self) -> bool {
        self.flags & parsed_attr_flag::ERROR_ATTR != 0
    }
    #[inline]
    pub fn parsing_transparent_union(&self) -> bool {
        self.flags & parsed_attr_flag::TRANSPARENT_UNION != 0
    }
    #[inline]
    pub fn parsing_fastcall(&self) -> bool {
        self.flags & parsed_attr_flag::FASTCALL != 0
    }
    #[inline]
    pub fn parsing_naked(&self) -> bool {
        self.flags & parsed_attr_flag::NAKED != 0
    }
    #[inline]
    pub fn parsing_no_instrument(&self) -> bool {
        self.flags & parsed_attr_flag::NO_INSTRUMENT != 0
    }
    #[inline]
    pub fn parsing_pure(&self) -> bool {
        self.flags & parsed_attr_flag::PURE != 0
    }
    #[inline]
    pub fn parsing_const_attr(&self) -> bool {
        self.flags & parsed_attr_flag::CONST_ATTR != 0
    }
    #[inline]
    pub fn parsing_cold(&self) -> bool {
        self.flags & parsed_attr_flag::COLD != 0
    }
    #[inline]
    pub fn parsing_uninitialized(&self) -> bool {
        self.flags & parsed_attr_flag::UNINITIALIZED != 0
    }

    // --- flag setters ---

    #[inline]
    pub fn set_typedef(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::TYPEDEF, v)
    }
    #[inline]
    pub fn set_static(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::STATIC, v)
    }
    #[inline]
    pub fn set_extern(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::EXTERN, v)
    }
    #[inline]
    pub fn set_nested_fn(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::NESTED_FN, v)
    }
    #[inline]
    pub fn set_thread_local(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::THREAD_LOCAL, v)
    }
    #[inline]
    pub fn set_inline(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::INLINE, v)
    }
    #[inline]
    pub fn set_const(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::CONST, v)
    }
    #[inline]
    pub fn set_pointer_const(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::POINTER_CONST, v)
    }
    #[inline]
    pub fn set_volatile(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::VOLATILE, v)
    }
    #[inline]
    pub fn set_typedef_use(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::TYPEDEF_USE, v)
    }
    #[inline]
    pub fn set_typedef_object_volatile(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::TYPEDEF_OBJECT_VOLATILE, v)
    }
    #[inline]
    pub fn set_constructor(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::CONSTRUCTOR, v)
    }
    #[inline]
    pub fn set_destructor(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::DESTRUCTOR, v)
    }
    #[inline]
    pub fn set_weak(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::WEAK, v)
    }
    #[inline]
    pub fn set_used(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::USED, v)
    }
    #[inline]
    pub fn set_maybe_unused(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::MAYBE_UNUSED, v)
    }
    #[inline]
    pub fn set_gnu_inline(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::GNU_INLINE, v)
    }
    #[inline]
    pub fn set_always_inline(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::ALWAYS_INLINE, v)
    }
    #[inline]
    pub fn set_noinline(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::NOINLINE, v)
    }
    #[inline]
    pub fn set_noreturn(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::NORETURN, v)
    }
    #[inline]
    pub fn set_error_attr(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::ERROR_ATTR, v)
    }
    #[inline]
    pub fn set_transparent_union(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::TRANSPARENT_UNION, v)
    }
    #[inline]
    pub fn set_fastcall(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::FASTCALL, v)
    }
    #[inline]
    pub fn set_uninitialized(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::UNINITIALIZED, v)
    }
    #[inline]
    pub fn set_naked(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::NAKED, v)
    }
    #[inline]
    pub fn set_no_instrument(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::NO_INSTRUMENT, v)
    }
    #[inline]
    pub fn set_pure(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::PURE, v)
    }
    #[inline]
    pub fn set_const_attr(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::CONST_ATTR, v)
    }
    #[inline]
    pub fn set_cold(&mut self, v: bool) {
        self.set_flag(parsed_attr_flag::COLD, v)
    }

    #[inline]
    fn set_flag(&mut self, mask: u64, v: bool) {
        if v {
            self.flags |= mask;
        } else {
            self.flags &= !mask;
        }
    }

    /// Save the packed boolean flags for later restoration.
    /// Used to prevent storage-class flags from leaking out of compound
    /// statements and typeof expressions.
    #[inline]
    pub fn save_flags(&self) -> u64 {
        self.flags
    }

    /// Restore previously saved packed boolean flags.
    #[inline]
    pub fn restore_flags(&mut self, saved: u64) {
        self.flags = saved;
    }
}

impl std::fmt::Debug for ParsedDeclAttrs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedDeclAttrs")
            .field("parsing_typedef", &self.parsing_typedef())
            .field("parsing_static", &self.parsing_static())
            .field("parsing_extern", &self.parsing_extern())
            .field("parsing_thread_local", &self.parsing_thread_local())
            .field("parsing_inline", &self.parsing_inline())
            .field("parsing_const", &self.parsing_const())
            .field("parsing_volatile", &self.parsing_volatile())
            .field("parsing_address_space", &self.parsing_address_space)
            .field("decl_address_space", &self.decl_address_space)
            .field("parsing_constructor", &self.parsing_constructor())
            .field("parsing_destructor", &self.parsing_destructor())
            .field("parsing_weak", &self.parsing_weak())
            .field("parsing_used", &self.parsing_used())
            .field("parsing_maybe_unused", &self.parsing_maybe_unused())
            .field("parsing_gnu_inline", &self.parsing_gnu_inline())
            .field("parsing_always_inline", &self.parsing_always_inline())
            .field("parsing_noinline", &self.parsing_noinline())
            .field("parsing_noreturn", &self.parsing_noreturn())
            .field("parsing_error_attr", &self.parsing_error_attr())
            .field(
                "parsing_transparent_union",
                &self.parsing_transparent_union(),
            )
            .field("parsing_fastcall", &self.parsing_fastcall())
            .field("parsing_regparm", &self.parsing_regparm)
            .field("parsing_cold", &self.parsing_cold())
            .field("parsing_alias_target", &self.parsing_alias_target)
            .field("parsing_ifunc_target", &self.parsing_ifunc_target)
            .field("parsing_visibility", &self.parsing_visibility)
            .field("parsing_tls_model", &self.parsing_tls_model)
            .field("parsing_section", &self.parsing_section)
            .field("parsing_cleanup_fn", &self.parsing_cleanup_fn)
            .field("parsing_vector_size", &self.parsing_vector_size)
            .field("parsing_ext_vector_nelem", &self.parsing_ext_vector_nelem)
            .field("parsed_alignas", &self.parsed_alignas)
            .field("parsed_alignas_type", &self.parsed_alignas_type)
            .field(
                "parsed_alignment_sizeof_type",
                &self.parsed_alignment_sizeof_type,
            )
            .finish()
    }
}

/// Recursive descent parser for C.
pub struct Parser {
    pub(super) tokens: Vec<Token>,
    pub(super) pos: usize,
    /// C23 dialect: `auto` without a type specifier infers the type
    /// (C23 6.7.10); the lexer already recognised the C23 keywords.
    pub(super) c23: bool,
    pub(super) typedefs: FxHashSet<String>,
    /// Type specifier each typedef name was declared with (parser-side, for
    /// parse-time sizeof(typedef) resolution in constant expressions such as
    /// `__attribute__((vector_size(16 * sizeof(int64_t))))`).  Chained
    /// typedefs, pointers and arrays fold into the stored spec recursively.
    pub(super) typedef_type_specs: FxHashMap<String, TypeSpecifier>,
    /// Named address space an object declared through a typedef name gets
    /// (`typedef int __seg_gs sint;` records `sint -> SegGs`). A use of the
    /// typedef name then sets `parsing_address_space` exactly as the
    /// `__seg_gs` keyword would, so the object, pointer, cast and parameter
    /// paths all see one qualifier. Absent means the typedef adds no space.
    pub(super) typedef_object_spaces: FxHashMap<String, AddressSpace>,
    /// Typedef names whose objects are volatile (`typedef volatile int VI;`,
    /// `typedef int *volatile VP;`). A use of the name applies `volatile` as
    /// its written text would. Same scoping as `typedef_object_spaces`.
    pub(super) typedef_object_volatile: FxHashSet<String>,
    /// Typedefs whose specifiers are `volatile`-qualified (`typedef volatile
    /// int VI;`, `typedef volatile int *VPI;`). Their base qualifier applies
    /// to each use, separately from the object qualifier above.
    pub(super) typedef_base_volatile: FxHashSet<String>,
    /// Typedef names shadowed by local variable declarations in the current scope.
    pub(super) shadowed_typedefs: FxHashSet<String>,
    /// Names declared as a FUNCTION (anywhere, any linkage): a designator of one
    /// of these is a link-time constant, which is what `asm "i"(fn)` needs.
    ///
    /// The distinction that matters is function vs object, not scope: GCC
    /// accepts `"i"(f)` for a *block-scope* function declaration too, and
    /// rejects it once a block-scope object shadows the name
    /// (`void target(void){}` then `int x;` → `"i"(target)` is still a symbol,
    /// `"i"(x)` is not). `shadowed_typedefs` is the precedent for the storage
    /// and its block save/restore; the removal on object declaration mirrors
    /// how a typedef name is hidden by a variable of the same name.
    pub(super) function_names: FxHashSet<String>,
    /// Accumulated declaration attributes from the current parse_type_specifier pass.
    /// Reset at the start of each top-level or local declaration.
    pub(super) attrs: ParsedDeclAttrs,
    /// Stack for #pragma pack alignment values.
    /// Current effective alignment is the last element (or None for default).
    pub(super) pragma_pack_stack: Vec<Option<usize>>,
    /// Current #pragma pack alignment. None means default (natural) alignment.
    pub(super) pragma_pack_align: Option<usize>,
    /// Stack for #pragma GCC visibility push/pop.
    /// Each entry is the visibility string (e.g., "hidden", "default").
    pub(super) pragma_visibility_stack: Vec<String>,
    /// Current default visibility from #pragma GCC visibility push(...).
    /// None means default visibility (no pragma active).
    pub(super) pragma_default_visibility: Option<String>,
    /// Unhonourable `cleanup` attributes seen while parsing the current
    /// declaration, drained by the declaration funnel and reported there at
    /// the declaration's own location: gcc-16 anchors `wrong number of
    /// arguments specified for 'cleanup' attribute` and `cleanup argument not
    /// an identifier` at the declaration, not at the attribute (measured with
    /// the attribute written first and after the type specifier). Kept on the
    /// parser and not on the attribute state because that state is
    /// snapshot-and-restored by parameter declarators and speculative parses,
    /// which dropped the record for block-scope declarations.
    pub(super) cleanup_diagnoses: Vec<(CleanupDiagnosis, crate::common::source::Span)>,
    /// Attributes already reported, so a declaration parsed twice (a
    /// speculative pass followed by the real one) reports each `cleanup`
    /// verdict exactly once.
    pub(super) reported_cleanup_attrs: crate::common::fx_hash::FxHashSet<u32>,
    /// S13: file-scope function names whose DECLARATION carried
    /// `no_instrument_function`. GCC merges declaration attributes into the
    /// definition (C redeclaration rules); the torture suite writes
    /// `int main () NOCHK;` with a bare definition, and without the merge
    /// the definition instruments anyway.
    pub(super) func_decl_no_instrument: FxHashSet<String>,
    /// File-scope function names whose prototype carried `stack_protect`.
    pub(super) func_decl_stack_protect: FxHashSet<String>,
    /// File-scope function names whose prototype carried `no_stack_protector`.
    pub(super) func_decl_no_stack_protector: FxHashSet<String>,
    /// File-scope functions whose any declaration carried `noinline`.
    /// Function attributes merge across redeclarations; a bare later
    /// definition must not silently become inlinable (frame-address.c).
    pub(super) func_decl_noinline: FxHashSet<String>,
    /// Count of parse errors encountered (invalid tokens at top level, etc.)
    pub error_count: usize,
    /// Number of currently-live expression-parse frames (E5 hardening).
    /// Bounded by `PARSER_FRAME_BUDGET`; see `enter_expr_frame`.
    pub(super) parser_frame_depth: u32,
    /// Defer immediate-only asm constness checks during an optimizing compile.
    /// GCC can fold a local initialized from constants (Linux's `__flags` in
    /// WARN/BUG helpers) or substitute an inline parameter only after parsing.
    /// The IR pipeline revalidates operands that remain dynamic after the
    /// optimization/inlining pipeline. Syntax-only mode leaves this false so
    /// it still diagnoses invalid dynamic operands without code generation.
    pub(super) asm_immediate_validation_deferred: bool,
    /// Set once the frame-budget diagnostic has been emitted, so a single
    /// pathological nesting level produces exactly one error, not a flood.
    pub(super) nesting_budget_diagnosed: bool,
    /// Structured diagnostic engine for error/warning reporting with source snippets.
    pub(super) diagnostics: DiagnosticEngine,
    /// Map of enum constant names to their integer values.
    /// Populated as enum definitions are parsed, so that later constant expressions
    /// (e.g., in __attribute__((aligned(1 << ENUM_CONST)))) can resolve them.
    pub(super) enum_constants: FxHashMap<String, i64>,
    /// Set of enum constant names whose values couldn't be evaluated at parse time
    /// (e.g., `MY_SIZE = sizeof(some_typedef)`). These are still valid constants,
    /// just not evaluable by our constant-expression evaluator.
    pub(super) unevaluable_enum_constants: FxHashSet<String>,
    /// Undo log for the two enumeration-constant tables: each entry is a name
    /// a block changed -- an enumerator it declared, or one an object, typedef
    /// or parameter of that name hides -- with its prior state (value, whether
    /// unevaluable). Leaving the block restores them, so a block-scope enum
    /// does not leak out (`void f(void) { enum { X = 1 }; }
    /// _Static_assert(X, "");` is an undeclared `X`, GCC 16.2) and a hidden
    /// enumerator is no constant inside (`int A; _Static_assert(A, "");`).
    pub(super) enum_scope_log: Vec<(String, Option<i64>, bool)>,
    /// Compound-statement nesting: zero at file scope, where nothing is undone.
    pub(super) block_depth: u32,
    /// True while parsing the body of a function that GCC re-instantiates after
    /// expansion (`inline`, `__inline__`, `always_inline`, `gnu_inline`).  The
    /// parser judges an `asm` immediate operand strictly (`constness` returns
    /// `NonConstant` for a parameter), but GCC defers exactly that judgement to
    /// instantiation, where the operand has already been substituted — so a
    /// body that is never instantiated here, or is instantiated with a
    /// constant, must not be rejected at parse time.  See `asm_validate`'s
    /// module docs for the measured matrix.
    pub(super) asm_deferred_instantiation: bool,
    /// Parser-visible integer constants introduced by an *initialised local
    /// object*: `__auto_type __flags = 1 | (2 << 8);` then `"i" (__flags)`.
    /// GCC accepts that because it folds the local before re-checking the
    /// `asm` statement at instantiation; the parser has no constant
    /// propagation, so it keeps this small map and drops every entry when the
    /// enclosing block ends.  A name that is ever assigned (or whose address is
    /// taken) is removed, so a stale value can never licence a wrong fold.
    /// Only integer scalars with a self-contained constant initializer are
    /// recorded; anything else is simply absent, which keeps the diagnostic.
    pub(super) local_constants: FxHashMap<String, i64>,
    /// `_Static_assert`s from struct member lists awaiting sema, emitted after
    /// the enclosing file-scope or block item (the members are not in scope
    /// as identifiers, so the assertion means the same there).
    pub(super) deferred_static_asserts: Vec<super::ast::StaticAssert>,
    /// Map of struct/union tag names to their computed alignments.
    /// Populated when a struct/union with fields is parsed, so that later
    /// __alignof__(struct tag) references can look up the correct alignment
    /// (especially important for packed structs where tag-only refs would
    /// otherwise incorrectly default to ptr_size).
    pub(super) struct_tag_alignments: FxHashMap<String, usize>,
}

impl Parser {
    /// Remove `name` from the enumeration-constant tables until the current
    /// scope ends (see `enum_scope_log`), recording its prior state.
    pub(super) fn hide_enum_constant(&mut self, name: &str) {
        let value = self.enum_constants.remove(name);
        let unevaluable = self.unevaluable_enum_constants.remove(name);
        self.enum_scope_log
            .push((name.to_string(), value, unevaluable));
    }

    /// Undo every enumeration-constant change logged since `mark`.
    pub(super) fn restore_enum_scope(&mut self, mark: usize) {
        while self.enum_scope_log.len() > mark {
            let Some((name, value, unevaluable)) = self.enum_scope_log.pop() else {
                break;
            };
            self.enum_constants.remove(&name);
            self.unevaluable_enum_constants.remove(&name);
            if let Some(v) = value {
                self.enum_constants.insert(name.clone(), v);
            }
            if unevaluable {
                self.unevaluable_enum_constants.insert(name);
            }
        }
    }

    /// Whether `name` is currently an enumeration constant to the parser.
    pub(super) fn is_parser_enum_constant(&self, name: &str) -> bool {
        self.enum_constants.contains_key(name) || self.unevaluable_enum_constants.contains(name)
    }

    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            c23: false,
            typedefs: Self::builtin_typedefs_cached(),
            typedef_type_specs: FxHashMap::default(),
            typedef_object_spaces: FxHashMap::default(),
            typedef_object_volatile: FxHashSet::default(),
            typedef_base_volatile: FxHashSet::default(),
            shadowed_typedefs: FxHashSet::default(),
            function_names: FxHashSet::default(),
            attrs: ParsedDeclAttrs::default(),
            pragma_pack_stack: Vec::with_capacity(4),
            pragma_pack_align: None,
            pragma_visibility_stack: Vec::with_capacity(4),
            pragma_default_visibility: None,
            func_decl_no_instrument: FxHashSet::default(),
            func_decl_stack_protect: FxHashSet::default(),
            func_decl_no_stack_protector: FxHashSet::default(),
            cleanup_diagnoses: Vec::new(),
            reported_cleanup_attrs: crate::common::fx_hash::FxHashSet::default(),
            func_decl_noinline: FxHashSet::default(),
            error_count: 0,
            parser_frame_depth: 0,
            asm_immediate_validation_deferred: false,
            nesting_budget_diagnosed: false,
            diagnostics: DiagnosticEngine::new(),
            enum_constants: FxHashMap::default(),
            unevaluable_enum_constants: FxHashSet::default(),
            enum_scope_log: Vec::new(),
            block_depth: 0,
            asm_deferred_instantiation: false,
            local_constants: FxHashMap::default(),
            deferred_static_asserts: Vec::new(),
            struct_tag_alignments: FxHashMap::default(),
        }
    }

    /// Set a pre-configured diagnostic engine on the parser.
    /// Called by the driver after setting up the source manager on the engine.
    pub fn set_diagnostics(&mut self, engine: DiagnosticEngine) {
        self.diagnostics = engine;
    }

    /// Select the C23 dialect (see the `c23` field).
    pub fn set_c23(&mut self, enabled: bool) {
        self.c23 = enabled;
    }

    /// Defer immediate-only inline-asm constness checks until optimized IR.
    /// The normal code-generation pipeline must enable this before parsing;
    /// syntax-only callers should leave it disabled.
    pub fn set_deferred_asm_immediate_validation(&mut self, enabled: bool) {
        self.asm_immediate_validation_deferred = enabled;
    }

    /// Take the diagnostic engine back from the parser (transfers ownership).
    /// The driver uses this to continue using the same engine across phases.
    pub fn take_diagnostics(&mut self) -> DiagnosticEngine {
        std::mem::take(&mut self.diagnostics)
    }

    /// Emit a parse error at the given span. Updates error_count and prints
    /// the error with source location and snippet (if source manager is set).
    pub(super) fn emit_error(&mut self, message: impl Into<String>, span: Span) {
        // Silent unwind after budget exhaustion: the single budget
        // diagnostic is the TU verdict; every later error is cascade
        // noise over placeholder ASTs.
        if self.nesting_budget_diagnosed {
            return;
        }
        self.error_count += 1;
        self.diagnostics.error(message, span);
    }

    /// Register one entry into the recursive expression-parse chain.
    ///
    /// WHY: every nesting level of a parenthesised expression re-enters the
    /// full precedence climb (`parse_expr` -> assignment -> conditional ->
    /// 10x binary/next-tighter -> cast -> unary -> postfix -> primary), and
    /// each of those frames carries `Expr`-sized by-value temporaries plus
    /// the big locals of `parse_primary_expr`/`parse_cast_expr`. Measured on
    /// the -O1 fastbuild, ~4800 nested-parenthesis levels overflow even the
    /// 64 MB compiler thread stack (a raw abort, rc=134; GCC 14 segfaults
    /// its cc1 on the same input). Left-associative chains do NOT recurse
    /// (they loop), so this bound bites only on genuinely deep nesting.
    ///
    /// The bound is on live parser frames, not source-nesting depth: real
    /// translation units (kernel, glibc, zlib-ng, expat, csmith output) stay
    /// orders of magnitude under it, while the adversarial class in
    /// `artifacts/repros/crash_synth_*` and beyond now degrades to a clean,
    /// GCC-compatible error instead of an abort.
    /// Budget for expression / initializer / record frames. Calibrated
    /// against the measured expression crash ceiling (~4800 nested-paren
    /// levels ≈ 62k actual frames overflow the 64 MB compiler thread):
    /// the budget trips at roughly half the overflow depth.
    pub(super) const PARSER_FRAME_BUDGET: u32 = 32_768;

    /// Budget for statement/block frames, LOWER than the expression budget
    /// because block frames cost ~4x more stack per counted frame
    /// (measured: ~14k compound-statement levels overflow before any
    /// 32k-frame budget could trip). Statements and compound statements
    /// both register frames, so one brace level costs two counted frames:
    /// the effective ceiling is ~4096 nested blocks, a ~3.5x margin under
    /// the measured overflow, robust across build profiles. Real TUs nest
    /// blocks single-digit-deep; the gate's deep-but-legal cases stay
    /// under it by an order of magnitude.
    pub(super) const BLOCK_FRAME_BUDGET: u32 = 8_192;

    /// Budget for nested struct/union DEFINITION frames, lower than the
    /// block budget for two independent reasons:
    ///   * real translation units nest record definitions single digits
    ///     deep (kernel, glibc, sqlite all measured <= ~8) -- 2048 levels
    ///     is >250x the worst real-world shape;
    ///   * the post-parse phases (sema layout, codegen) are quadratic on
    ///     record-nesting depth (measured 9.7 s for an 8000-deep named
    ///     chain), so keeping the legal depth under this bound caps the
    ///     worst-case compile of a hostile-but-legal TU at ~0.6 s while
    ///     the parser still holds a 25x margin over its measured ~50k
    ///     stack-overflow ceiling.
    pub(super) const TYPE_FRAME_BUDGET: u32 = 2_048;

    /// Bound on pointer-indirection levels within a single declarator.
    ///
    /// This class needs its own bound because it is not a recursion. `CType`
    /// is a recursive enum, so its derived `Clone`, `Drop` and `PartialEq`
    /// each recurse once per level, and unlike expressions, blocks,
    /// initializers and record definitions, pointer depth is a property of the
    /// FINISHED type rather than of the parse -- so no live-frame counter ever
    /// sees it. Measured under gdb: `int ****…p` with ~400k stars parsed
    /// cleanly and then aborted with rc=134 ("has overflowed its stack")
    /// inside `CType::clone`, on a file GCC 16.2 accepts. The declaration
    /// alone is enough; no call site is involved.
    ///
    /// 4096 is >1000x the deepest declarator in real code (kernel, glibc and
    /// sqlite all stay at four levels or fewer) while keeping a 4096-deep
    /// clone/drop comfortably inside even an 8 MB stack, so the bound holds
    /// regardless of which thread the type is destroyed on and regardless of
    /// build profile.
    pub(super) const POINTER_DEPTH_BUDGET: u32 = 4_096;

    /// One pointer-indirection level of a declarator. Unlike the recursion
    /// budgets this is a plain depth cap rather than a frame registration: the
    /// levels are counted as the `*` tokens are consumed, so there is nothing
    /// to unwind and no matching exit call.
    pub(super) fn enter_pointer_level(&mut self, depth: u32, span: Span) -> bool {
        if depth > Self::POINTER_DEPTH_BUDGET {
            if !self.nesting_budget_diagnosed {
                self.nesting_budget_diagnosed = true;
                self.error_count += 1;
                self.diagnostics.error(
                    format!(
                        "pointer nesting too deep (exceeds {} levels)",
                        Self::POINTER_DEPTH_BUDGET
                    ),
                    span,
                );
            }
            return false;
        }
        true
    }

    /// Shared budget machinery (generalized from the original
    /// expression-only counter): every recursive parser entry point that a
    /// hostile TU can drive to stack overflow registers a frame here.
    /// One live-frame counter covers all classes because the resource at
    /// risk is the SAME stack; one `nesting_budget_diagnosed` latch keeps
    /// the verdict at exactly one diagnostic regardless of which class
    /// trips first; all entry points unwind silently once it is set.
    fn enter_nesting_frame(&mut self, span: Span, class: &str, budget: u32) -> bool {
        if self.parser_frame_depth >= budget {
            if !self.nesting_budget_diagnosed {
                self.nesting_budget_diagnosed = true;
                self.error_count += 1;
                self.diagnostics.error(
                    format!("{} (exceeds {} parser frames)", class, budget),
                    span,
                );
            }
            return false;
        }
        self.parser_frame_depth += 1;
        true
    }

    pub(super) fn enter_expr_frame(&mut self, span: Span) -> bool {
        self.enter_nesting_frame(
            span,
            "expression nesting too deep",
            Self::PARSER_FRAME_BUDGET,
        )
    }

    /// Nested compound statements (`{ { { ... } } }`) recurse one
    /// parse_compound_stmt frame per brace level (block-class budget).
    pub(super) fn enter_block_frame(&mut self, span: Span) -> bool {
        self.enter_nesting_frame(span, "block nesting too deep", Self::BLOCK_FRAME_BUDGET)
    }

    /// Every statement parse registers a frame too: statement nesting
    /// (block bodies, loop/if bodies, statement expressions) recurses
    /// through parse_stmt, and counting it doubles the block-class
    /// accounting per brace level, buying margin against build-profile
    /// frame-size variation. Flat statement sequences enter and exit
    /// immediately, so they never accumulate.
    pub(super) fn enter_stmt_frame(&mut self, span: Span) -> bool {
        self.enter_nesting_frame(span, "block nesting too deep", Self::BLOCK_FRAME_BUDGET)
    }

    /// Nested brace-initializer lists recurse one parse_initializer frame
    /// per `{` level.
    pub(super) fn enter_initializer_frame(&mut self, span: Span) -> bool {
        self.enter_nesting_frame(
            span,
            "initializer nesting too deep",
            Self::PARSER_FRAME_BUDGET,
        )
    }

    /// Nested struct/union DEFINITIONS recurse one parse_struct_or_union
    /// frame per level (field type -> specifier -> nested record).
    /// TYPE_FRAME_BUDGET: see the constant's rationale (real TUs nest
    /// single digits deep; the bound also caps post-parse quadratic
    /// layout cost on hostile-but-legal input).
    pub(super) fn enter_record_frame(&mut self, span: Span) -> bool {
        self.enter_nesting_frame(span, "type nesting too deep", Self::TYPE_FRAME_BUDGET)
    }

    #[inline]
    pub(super) fn exit_parser_frame(&mut self) {
        debug_assert!(self.parser_frame_depth > 0);
        self.parser_frame_depth = self.parser_frame_depth.saturating_sub(1);
    }

    /// Panic-mode recovery: skip until a synchronization point.
    pub(super) fn sync_to(&mut self, stop: &[TokenKind]) {
        while !matches!(self.peek(), TokenKind::Eof) {
            let k = self.peek();
            if stop
                .iter()
                .any(|s| std::mem::discriminant(k) == std::mem::discriminant(s))
            {
                return;
            }
            if matches!(
                k,
                TokenKind::Semicolon
                    | TokenKind::RBrace
                    | TokenKind::Eof
                    | TokenKind::RParen
                    | TokenKind::RBracket
            ) {
                return;
            }
            self.advance();
        }
    }

    pub(super) fn sync_after_error(&mut self) {
        self.sync_to(&[TokenKind::Semicolon, TokenKind::RBrace]);
        if matches!(self.peek(), TokenKind::Semicolon) {
            self.advance();
        }
    }

    /// Standard C typedef names commonly provided by system headers.
    /// Since we don't actually include system headers, we pre-seed these.
    fn builtin_typedefs_cached() -> FxHashSet<String> {
        static CACHE: std::sync::LazyLock<FxHashSet<String>> =
            std::sync::LazyLock::new(Parser::builtin_typedefs);
        CACHE.clone()
    }

    fn builtin_typedefs() -> FxHashSet<String> {
        [
            // <stddef.h>
            "size_t",
            crate::common::types::OFFSETOF_SIZE_TYPEDEF,
            "ssize_t",
            "ptrdiff_t",
            "wchar_t",
            "wint_t",
            // <stdint.h>
            "int8_t",
            "int16_t",
            "int32_t",
            "int64_t",
            "uint8_t",
            "uint16_t",
            "uint32_t",
            "uint64_t",
            "intptr_t",
            "uintptr_t",
            "intmax_t",
            "uintmax_t",
            "int_least8_t",
            "int_least16_t",
            "int_least32_t",
            "int_least64_t",
            "uint_least8_t",
            "uint_least16_t",
            "uint_least32_t",
            "uint_least64_t",
            "int_fast8_t",
            "int_fast16_t",
            "int_fast32_t",
            "int_fast64_t",
            "uint_fast8_t",
            "uint_fast16_t",
            "uint_fast32_t",
            "uint_fast64_t",
            // <stdio.h>
            "FILE",
            "fpos_t",
            // <signal.h>
            "sig_atomic_t",
            // <time.h>
            "time_t",
            "clock_t",
            "timer_t",
            "clockid_t",
            // <sys/types.h>
            "off_t",
            "pid_t",
            "uid_t",
            "gid_t",
            "mode_t",
            "dev_t",
            "ino_t",
            "nlink_t",
            "blksize_t",
            "blkcnt_t",
            // GNU/glibc common types
            "ulong",
            "ushort",
            "uint",
            "__u8",
            "__u16",
            "__u32",
            "__u64",
            "__s8",
            "__s16",
            "__s32",
            "__s64",
            // <stdarg.h>
            "va_list",
            "__builtin_va_list",
            "__gnuc_va_list",
            // <locale.h>
            "locale_t",
            // <pthread.h>
            "pthread_t",
            "pthread_mutex_t",
            "pthread_cond_t",
            "pthread_key_t",
            "pthread_attr_t",
            "pthread_once_t",
            "pthread_mutexattr_t",
            "pthread_condattr_t",
            // <setjmp.h>
            "jmp_buf",
            "sigjmp_buf",
            // <dirent.h>
            "DIR",
            // GCC builtin NEON vector types (AArch64).
            // These are compiler-internal types used by bits/math-vector.h when
            // __GNUC_PREREQ(9, 0). They only appear in typedef/function declarations
            // behind #ifdef __aarch64__ guards, so listing them unconditionally is safe.
            "__Float32x4_t",
            "__Float64x2_t",
            // GCC builtin SVE scalable vector types (AArch64).
            // Used by bits/math-vector.h when __GNUC_PREREQ(10, 0).
            // SVE types are runtime-sized in real GCC, but we model them as fixed
            // 16-byte vectors for parsing (the functions using these are never called).
            "__SVFloat32_t",
            "__SVFloat64_t",
            "__SVBool_t",
            "__SVInt8_t",
            "__SVInt16_t",
            "__SVInt32_t",
            "__SVInt64_t",
            "__SVUint8_t",
            "__SVUint16_t",
            "__SVUint32_t",
            "__SVUint64_t",
            "__SVFloat16_t",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    pub fn parse(&mut self) -> TranslationUnit {
        // Named FS/GS spaces are x86 extensions. Diagnose even declaration-only
        // uses, which otherwise survive optimization without reaching the
        // backend's fail-closed panic. Tokens are already preprocessed, so a
        // qualifier in an inactive #if branch is not rejected. Keep this scan
        // off the x86 hot path (ELF EM_386=3, EM_X86_64=62).
        if !matches!(crate::common::types::target_elf_machine(), 3 | 62)
            && let Some(span) = self.tokens.iter().find_map(|token| {
                matches!(token.kind, TokenKind::SegGs | TokenKind::SegFs).then_some(token.span)
            })
        {
            self.emit_error("named address spaces require an x86 target", span);
            return TranslationUnit { decls: Vec::new() };
        }
        let mut decls = Vec::with_capacity(16);
        while !self.at_eof() {
            // The expression frame-budget error is fatal for the TU; stop
            // parsing instead of cascading thousands of recovery errors over
            // the (now meaningless) token remainder.
            if self.nesting_budget_diagnosed {
                break;
            }
            if let Some(decl) = self.parse_external_decl() {
                decls.push(decl);
                decls.extend(
                    self.deferred_static_asserts
                        .drain(..)
                        .map(ExternalDecl::StaticAssert),
                );
            } else {
                // Report error for unrecognized token at top level
                if !matches!(self.peek(), TokenKind::Semicolon | TokenKind::Eof) {
                    let span = self.peek_span();
                    self.emit_error(format!("expected declaration before {}", self.peek()), span);
                }
                self.advance();
            }
        }
        TranslationUnit { decls }
    }

    // === Token access helpers ===

    pub(super) fn at_eof(&self) -> bool {
        self.pos >= self.tokens.len() || matches!(self.tokens[self.pos].kind, TokenKind::Eof)
    }

    pub(super) fn peek(&self) -> &TokenKind {
        if self.pos < self.tokens.len() {
            &self.tokens[self.pos].kind
        } else {
            &TokenKind::Eof
        }
    }

    /// Look ahead `n` tokens (0 == peek). Returns Eof past the end.
    #[inline]
    pub(super) fn peek_nth(&self, n: usize) -> &TokenKind {
        let i = self.pos + n;
        if i < self.tokens.len() {
            &self.tokens[i].kind
        } else {
            &TokenKind::Eof
        }
    }

    /// `open` widened through the token just consumed (a body's `}`).
    pub(super) fn through_previous_token(&self, open: Span) -> Span {
        match self.pos.checked_sub(1).and_then(|i| self.tokens.get(i)) {
            Some(tok) if tok.span.file_id == open.file_id && tok.span.end > open.end => {
                Span::new(open.start, tok.span.end, open.file_id)
            }
            _ => open,
        }
    }

    pub(super) fn peek_span(&self) -> Span {
        if self.pos < self.tokens.len() {
            self.tokens[self.pos].span
        } else {
            Span::dummy()
        }
    }

    pub(super) fn advance(&mut self) -> &Token {
        if self.pos < self.tokens.len() {
            let tok = &self.tokens[self.pos];
            self.pos += 1;
            tok
        } else {
            &self.tokens[self.tokens.len() - 1]
        }
    }

    pub(super) fn expect(&mut self, expected: &TokenKind) -> Span {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            let span = self.peek_span();
            self.advance();
            span
        } else {
            let span = self.peek_span();
            self.emit_error(
                format!("expected {} before {}", expected, self.peek()),
                span,
            );
            if matches!(expected, TokenKind::Semicolon) {
                self.sync_after_error();
            }
            span
        }
    }

    /// Expect a token with a contextual description of what construct we're parsing.
    /// Produces messages like "expected ';' after return statement" instead of
    /// the generic "expected ';' before '}'".
    ///
    /// For semicolons: if the next token starts a new statement (e.g., '}', keyword),
    /// the error points at the previous token's end position and suggests inserting
    /// the missing token, which matches GCC's behavior.
    pub(super) fn expect_after(&mut self, expected: &TokenKind, context: &str) -> Span {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            let span = self.peek_span();
            self.advance();
            span
        } else {
            // The frame-budget diagnostic is fatal for the TU; suppress the
            // cascade of "expected ';'" recovery errors while the stub
            // unwinds (same policy as expect_closing).
            if self.nesting_budget_diagnosed {
                return self.peek_span();
            }
            let span = self.peek_span();
            let diag = crate::common::error::Diagnostic::error(format!(
                "expected {} {} before {}",
                expected,
                context,
                self.peek()
            ))
            .with_span(span)
            .with_fix_hint(format!("insert {}", expected));
            self.error_count += 1;
            self.diagnostics.emit(&diag);
            span
        }
    }

    /// Expect a closing delimiter (paren, brace, bracket) and if missing,
    /// attach a note pointing back to where the opening delimiter was,
    /// plus a fix-it hint suggesting the insertion.
    /// Produces messages like:
    ///   error: expected ')' before ';'
    ///   note: to match this '(' (at file.c:10:5)
    ///   fix-it hint: insert ')'
    pub(super) fn expect_closing(&mut self, expected: &TokenKind, open_span: Span) -> Span {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            let span = self.peek_span();
            self.advance();
            span
        } else {
            // The frame-budget diagnostic is fatal for the TU; do not cascade
            // thousands of "expected ')'" errors while the stub unwinds.
            if self.nesting_budget_diagnosed {
                return self.peek_span();
            }
            let span = self.peek_span();
            let open_tok = match expected {
                TokenKind::RParen => "'('",
                TokenKind::RBrace => "'{'",
                TokenKind::RBracket => "'['",
                _ => "'?'",
            };
            let diag = crate::common::error::Diagnostic::error(format!(
                "expected {} before {}",
                expected,
                self.peek()
            ))
            .with_span(span)
            .with_fix_hint(format!("insert {}", expected))
            .with_note(
                crate::common::error::Diagnostic::note(format!("to match this {}", open_tok))
                    .with_span(open_span),
            );
            self.error_count += 1;
            self.diagnostics.emit(&diag);
            span
        }
    }

    /// Expect a token with a contextual description of the construct being parsed.
    /// Unlike `expect()`, produces messages like "expected '(' after 'if'" instead of
    /// the generic "expected '(' before 'x'", and includes a fix-it hint.
    pub(super) fn expect_context(&mut self, expected: &TokenKind, context: &str) -> Span {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            let span = self.peek_span();
            self.advance();
            span
        } else {
            // See expect_closing: one fatal budget error, no cascade.
            if self.nesting_budget_diagnosed {
                return self.peek_span();
            }
            let span = self.peek_span();
            let diag = crate::common::error::Diagnostic::error(format!(
                "expected {} {} before {}",
                expected,
                context,
                self.peek()
            ))
            .with_span(span)
            .with_fix_hint(format!("insert {}", expected));
            self.error_count += 1;
            self.diagnostics.emit(&diag);
            span
        }
    }

    pub(super) fn consume_if(&mut self, kind: &TokenKind) -> bool {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    // === Type and qualifier helpers ===

    /// Check if the current position is a typedef name followed by ':',
    /// which means it's a label (not a declaration). In C, label names
    /// can shadow typedef names: `typedef struct Ins Ins; ... Ins: stmt;`
    pub(super) fn is_typedef_label(&self) -> bool {
        if let TokenKind::Identifier(name) = self.peek() {
            if self.typedefs.contains(name) && !self.shadowed_typedefs.contains(name) {
                // Check if next token is ':'
                return matches!(self.peek_nth(1), TokenKind::Colon);
            }
        }
        false
    }

    pub(super) fn is_type_specifier(&self) -> bool {
        match self.peek() {
            TokenKind::Void
            | TokenKind::Char
            | TokenKind::Short
            | TokenKind::Int
            | TokenKind::Long
            | TokenKind::Float
            | TokenKind::Double
            | TokenKind::Signed
            | TokenKind::Unsigned
            | TokenKind::Struct
            | TokenKind::Union
            | TokenKind::Enum
            | TokenKind::Const
            | TokenKind::Volatile
            | TokenKind::Static
            | TokenKind::Extern
            | TokenKind::Register
            | TokenKind::Typedef
            | TokenKind::Inline
            | TokenKind::Bool
            | TokenKind::Typeof
            | TokenKind::TypeofUnqual
            | TokenKind::Attribute
            | TokenKind::Extension
            | TokenKind::Noreturn
            | TokenKind::Restrict
            | TokenKind::Complex
            | TokenKind::Atomic
            | TokenKind::Auto
            | TokenKind::AutoType
            | TokenKind::Alignas
            | TokenKind::Builtin
            | TokenKind::Int128
            | TokenKind::UInt128
            | TokenKind::Float16
            | TokenKind::BFloat16
            | TokenKind::Float128
            | TokenKind::Decimal32
            | TokenKind::Decimal64
            | TokenKind::Decimal128
            | TokenKind::ThreadLocal
            | TokenKind::Constexpr
            | TokenKind::SegGs
            | TokenKind::SegFs => true,
            TokenKind::Identifier(name) => {
                self.typedefs.contains(name) && !self.shadowed_typedefs.contains(name)
            }
            _ => false,
        }
    }

    pub(super) fn skip_cv_qualifiers(&mut self) {
        loop {
            match self.peek() {
                TokenKind::Const | TokenKind::Restrict => {
                    let was_const = matches!(self.peek(), TokenKind::Const);
                    if !was_const {
                        self.attrs.set_restrict(true);
                    }
                    self.advance();
                    // `skip_cv_qualifiers` is called right after each `*` in a
                    // declarator, so a `const` seen here qualifies the POINTER
                    // (`T *const p`), not the pointee.  Record it: such a
                    // variable is genuinely read-only and, when its initializer
                    // needs a relocation, must be placed in `.data.rel.ro`
                    // rather than `.data` (see `classify_global`).
                    if was_const {
                        self.attrs.set_pointer_const(true);
                    }
                }
                // After a `*` only qualifiers can follow, so `_Atomic` is the
                // qualifier even before `(` (a parenthesized declarator):
                // `int *_Atomic p;` declares an atomic pointer (C11 6.7.3).
                TokenKind::Atomic => {
                    self.advance();
                    self.attrs.set_atomic(true);
                }
                TokenKind::Volatile => {
                    self.advance();
                    // Propagate volatile to the declaration so that
                    // `T *volatile p` marks `p` as volatile (preventing
                    // mem2reg from promoting it to a register).
                    self.attrs.set_volatile(true);
                }
                TokenKind::SegGs => {
                    self.advance();
                    self.attrs.parsing_address_space = AddressSpace::SegGs;
                }
                TokenKind::SegFs => {
                    self.advance();
                    self.attrs.parsing_address_space = AddressSpace::SegFs;
                }
                _ => break,
            }
        }
    }

    /// Skip C99 type qualifiers and 'static' inside array brackets.
    /// In C99 function parameter declarations, array dimensions can include:
    ///   [static restrict const 10], [restrict n], [const], [static 10], etc.
    /// We skip these qualifiers since they only affect optimization hints and
    /// don't change the type semantics (array params decay to pointers).
    /// Skip the qualifiers and `static` of an array declarator's `[...]`,
    /// returning their CVR bits (`ast::CVR_CONST` and friends).
    pub(super) fn skip_array_qualifiers(&mut self) -> u8 {
        let mut cvr = 0;
        loop {
            cvr |= match self.peek() {
                TokenKind::Const => CVR_CONST,
                TokenKind::Volatile => CVR_VOLATILE,
                TokenKind::Restrict => CVR_RESTRICT,
                TokenKind::Static | TokenKind::Atomic => 0,
                _ => return cvr,
            };
            self.advance();
        }
    }

    pub(super) fn skip_array_dimensions(&mut self) {
        while matches!(self.peek(), TokenKind::LBracket) {
            self.advance();
            while !matches!(self.peek(), TokenKind::RBracket | TokenKind::Eof) {
                self.advance();
            }
            self.consume_if(&TokenKind::RBracket);
        }
    }

    pub(super) fn compound_assign_op(&self) -> Option<BinOp> {
        match self.peek() {
            TokenKind::PlusAssign => Some(BinOp::Add),
            TokenKind::MinusAssign => Some(BinOp::Sub),
            TokenKind::StarAssign => Some(BinOp::Mul),
            TokenKind::SlashAssign => Some(BinOp::Div),
            TokenKind::PercentAssign => Some(BinOp::Mod),
            TokenKind::AmpAssign => Some(BinOp::BitAnd),
            TokenKind::PipeAssign => Some(BinOp::BitOr),
            TokenKind::CaretAssign => Some(BinOp::BitXor),
            TokenKind::LessLessAssign => Some(BinOp::Shl),
            TokenKind::GreaterGreaterAssign => Some(BinOp::Shr),
            _ => None,
        }
    }

    // === GCC extension helpers ===

    pub(super) fn skip_gcc_extensions(&mut self) {
        // `_spec`: records stay for the enclosing declaration's check.
        let (_, aligned, _, _, _) = self.parse_gcc_attributes_spec();
        if let Some(a) = aligned {
            self.attrs.parsed_alignas =
                Some(self.attrs.parsed_alignas.map_or(a, |prev| prev.max(a)));
        }
    }

    /// Consume C23 standard attributes in statement position: `[[fallthrough]]`,
    /// `[[maybe_unused]]`, `[[gnu::...]]`, `[[clang::...]]`, etc. The lexer
    /// produces two LBracket tokens for `[[`; consume through the matching
    /// `]]` pair. These are pure hints for LCCC, so they are skipped entirely
    /// (glibc's sysdeps/x86/cpu-features.c uses `[[fallthrough]];`).
    pub(super) fn skip_c23_attribute_lists(&mut self) {
        loop {
            if !matches!(self.peek(), TokenKind::LBracket) {
                break;
            }
            // Two-token lookahead: require `[[` (C23 attribute), not a lone `[`.
            let next_is_lb =
                self.pos + 1 < self.tokens.len() && matches!(self.peek_nth(1), TokenKind::LBracket);
            if !next_is_lb {
                break;
            }
            self.advance(); // first '['
            self.advance(); // second '['
            // Consume until the matching ']]' pair (respecting nesting of '[' ']'
            // inside attribute args, e.g. [[gnu::aligned(8)]]). When depth hits
            // zero at a ']', consume that ']' and its partner ']', then stop.
            let mut depth = 1usize;
            while self.pos < self.tokens.len() {
                match self.tokens[self.pos].kind {
                    TokenKind::LBracket => depth += 1,
                    TokenKind::RBracket => {
                        depth -= 1;
                        if depth == 0 {
                            self.advance(); // the ']' that closed the attribute
                            if self.pos < self.tokens.len()
                                && matches!(self.tokens[self.pos].kind, TokenKind::RBracket)
                            {
                                self.advance(); // partner ']'
                            }
                            break;
                        }
                    }
                    _ => {}
                }
                self.advance();
            }
        }
    }

    #[inline]
    pub(super) fn is_c23_attr_start(&self) -> bool {
        matches!(self.peek(), TokenKind::LBracket)
            && self.pos + 1 < self.tokens.len()
            && matches!(self.peek_nth(1), TokenKind::LBracket)
    }

    /// Parse a single `[[ ... ]]` C23 attribute specifier list, honoring both
    /// standard C23 attributes (`noreturn`, `unsequenced`, `reproducible`,
    /// `maybe_unused`, `nodiscard`, `deprecated`, `fallthrough`) and `gnu::` /
    /// `__gnu__::` scoped attributes (including `using gnu: ...`).
    fn parse_c23_attribute_specifier(
        &mut self,
        is_packed: &mut bool,
        aligned: &mut Option<usize>,
        mode_kind: &mut Option<ModeKind>,
        is_common: &mut bool,
        scalar_storage_order: &mut Option<bool>,
    ) {
        self.advance(); // first '['
        self.advance(); // second '['

        // Optional C23 `using <ns> :` prefix
        let mut default_ns: Option<String> = None;
        if let TokenKind::Identifier(kw) = self.peek()
            && kw == "using"
            && self.pos + 2 < self.tokens.len()
            && matches!(self.peek_nth(2), TokenKind::Colon)
            && !(self.pos + 3 < self.tokens.len() && matches!(self.peek_nth(3), TokenKind::Colon))
        {
            self.advance(); // 'using'
            if let TokenKind::Identifier(ns) = self.peek() {
                default_ns = Some(ns.clone());
                self.advance();
            }
            self.consume_if(&TokenKind::Colon);
        }

        while !self.at_eof() {
            if matches!(self.peek(), TokenKind::Comma) {
                self.advance();
                continue;
            }
            if matches!(self.peek(), TokenKind::RBracket) {
                self.advance();
                self.consume_if(&TokenKind::RBracket);
                break;
            }

            // Read optional `<scope> ::` prefix and attribute name (which may
            // be tokenized as an identifier or keyword like `const` / `_Noreturn`).
            let token_attr_name = |tk: &TokenKind| -> Option<String> {
                match tk {
                    TokenKind::Identifier(s) => Some(s.clone()),
                    TokenKind::Const => Some("const".to_string()),
                    TokenKind::Noreturn => Some("_Noreturn".to_string()),
                    _ => None,
                }
            };

            let Some(first_name) = token_attr_name(self.peek()) else {
                // Unexpected token inside `[[ ... ]]`: skip balanced to `]]`.
                let mut depth = 1usize;
                while self.pos < self.tokens.len() {
                    match self.tokens[self.pos].kind {
                        TokenKind::LBracket => depth += 1,
                        TokenKind::RBracket => {
                            depth -= 1;
                            if depth == 0 {
                                self.advance();
                                self.consume_if(&TokenKind::RBracket);
                                return;
                            }
                        }
                        _ => {}
                    }
                    self.advance();
                }
                return;
            };

            let (scope, attr_name) = if self.pos + 2 < self.tokens.len()
                && matches!(self.peek_nth(1), TokenKind::Colon)
                && matches!(self.peek_nth(2), TokenKind::Colon)
            {
                self.advance(); // scope
                self.advance(); // ':'
                self.advance(); // ':'
                match token_attr_name(self.peek()) {
                    Some(n) => (Some(first_name), n),
                    None => continue,
                }
            } else {
                (default_ns.clone(), first_name)
            };

            let norm_scope = scope.as_deref().map(|s| {
                s.strip_prefix("__")
                    .and_then(|r| r.strip_suffix("__"))
                    .unwrap_or(s)
            });
            let norm_name = attr_name
                .strip_prefix("__")
                .and_then(|r| r.strip_suffix("__"))
                .unwrap_or(attr_name.as_str());

            match norm_scope {
                None => match norm_name {
                    "maybe_unused" => {
                        self.attrs.set_maybe_unused(true);
                        self.advance();
                        if matches!(self.peek(), TokenKind::LParen) {
                            self.skip_balanced_parens();
                        }
                    }
                    "noreturn" | "_Noreturn" => {
                        self.attrs.set_noreturn(true);
                        // C23 spells the verdict differently from the GNU
                        // namespace: `int a [[noreturn]];` reports
                        // `standard 'noreturn' attribute can only be applied to
                        // functions`, while `[[gnu::noreturn]]` reports the
                        // generic sentence (both measured on gcc-16.2.0). The
                        // marker prefix keeps the two wordings apart in the
                        // shared placement check.
                        self.attrs.parsing_attr_targets.push((
                            "standard:noreturn".to_string(),
                            self.peek_span(),
                            crate::frontend::parser::attribute_registry::AttributeTarget::Function,
                        ));
                        self.advance();
                        if matches!(self.peek(), TokenKind::LParen) {
                            self.skip_balanced_parens();
                        }
                    }
                    "unsequenced" => {
                        self.attrs.set_const_attr(true);
                        self.advance();
                        if matches!(self.peek(), TokenKind::LParen) {
                            self.skip_balanced_parens();
                        }
                    }
                    "reproducible" => {
                        self.attrs.set_pure(true);
                        self.advance();
                        if matches!(self.peek(), TokenKind::LParen) {
                            self.skip_balanced_parens();
                        }
                    }
                    _ => {
                        // C23 attributes in the implementation namespace use
                        // the registry's three classes, with the C23 wording
                        // measured on gcc-16.2.0 (2026-10-27): a name no GCC
                        // attribute registry entry matches is reported as
                        // `'x' attribute ignored` (`int q [[frobnicate]];`),
                        // while a registry name is silent by default -- the
                        // same funnel `__attribute__((...))` goes through, so
                        // there is exactly one classification in the compiler.
                        use crate::frontend::parser::attribute_registry as registry;
                        if matches!(
                            registry::classify(&attr_name),
                            registry::AttributeClass::Unknown
                        ) {
                            let span = self.peek_span();
                            self.diagnostics.warning_with_kind(
                                format!("'{}' attribute ignored", registry::canonical(&attr_name)),
                                span,
                                crate::common::error::WarningKind::Attributes,
                            );
                        }
                        self.advance();
                        if matches!(self.peek(), TokenKind::LParen) {
                            self.skip_balanced_parens();
                        }
                    }
                },
                Some("gnu") => {
                    // `[[gnu::noreturn]] int x;` is the same mistake as
                    // `__attribute__((noreturn)) int x;` and carries the same
                    // verdict, so the C23 spelling records the placement the
                    // one funnel checks (measured: `'noreturn' attribute
                    // ignored`, `'packed' attribute ignored`).
                    use crate::frontend::parser::attribute_registry as registry;
                    let target = registry::target(&attr_name);
                    if target != registry::AttributeTarget::Any {
                        self.attrs.parsing_attr_targets.push((
                            registry::canonical(&attr_name).to_string(),
                            self.peek_span(),
                            target,
                        ));
                    }
                    self.dispatch_gcc_attribute(
                        &attr_name,
                        is_packed,
                        aligned,
                        mode_kind,
                        is_common,
                        scalar_storage_order,
                    );
                }
                Some(_) => {
                    self.advance();
                    if matches!(self.peek(), TokenKind::LParen) {
                        self.skip_balanced_parens();
                    }
                }
            }
        }
    }

    /// Parse __attribute__((...)), C23 [[...]], and __extension__, returning struct attribute flags.
    /// Returns (is_packed, aligned_value, mode_kind, is_common, scalar_storage_order).
    /// scalar_storage_order: Some(true) = big-endian, Some(false) = little-endian, None = unspecified.
    /// Spec-position entry: attribute records that measure a placement rule
    /// (see `attribute_registry`) stay on `self.attrs` for the declaration
    /// kind check; every other caller uses [`Self::parse_gcc_attributes`],
    /// which clears them so a parameter or struct attribute cannot leak
    /// into a later declaration's check.
    pub(super) fn parse_gcc_attributes_spec(
        &mut self,
    ) -> (bool, Option<usize>, Option<ModeKind>, bool, Option<bool>) {
        self.parse_gcc_attributes_inner()
    }

    /// `__attribute__((...))` on a `struct`/`union` tag.
    ///
    /// Identical to [`Self::parse_gcc_attributes`] except that the
    /// record-placement flag is raised for the duration, which is what lets
    /// `attribute_registry::abi_effect` tell `struct __attribute__((ms_struct))
    /// S {...}` (gcc honours it, so lccc must refuse rather than ignore) from
    /// `__attribute__((ms_struct)) void f(void)` (gcc ignores it with a
    /// warning, so lccc must do the same). The flag is restored, not cleared,
    /// so nested parses cannot observe a stale placement.
    pub(super) fn parse_record_attributes(
        &mut self,
    ) -> (bool, Option<usize>, Option<ModeKind>, bool, Option<bool>) {
        let saved = std::mem::replace(&mut self.attrs.parsing_record_attributes, true);
        let parsed = self.parse_gcc_attributes();
        self.attrs.parsing_record_attributes = saved;
        parsed
    }

    pub(super) fn parse_gcc_attributes(
        &mut self,
    ) -> (bool, Option<usize>, Option<ModeKind>, bool, Option<bool>) {
        let result = self.parse_gcc_attributes_inner();
        self.attrs.parsing_attr_targets.clear();
        self.attrs.first_attr_span = None;
        result
    }

    fn parse_gcc_attributes_inner(
        &mut self,
    ) -> (bool, Option<usize>, Option<ModeKind>, bool, Option<bool>) {
        let mut is_packed = false;
        let mut aligned = None;
        let mut mode_kind: Option<ModeKind> = None;
        let mut is_common = false;
        let mut scalar_storage_order: Option<bool> = None;
        loop {
            match self.peek() {
                TokenKind::Extension => {
                    self.advance();
                }
                TokenKind::LBracket if self.is_c23_attr_start() => {
                    self.attrs.first_attr_span.get_or_insert(self.peek_span());
                    self.parse_c23_attribute_specifier(
                        &mut is_packed,
                        &mut aligned,
                        &mut mode_kind,
                        &mut is_common,
                        &mut scalar_storage_order,
                    );
                }
                TokenKind::Attribute => {
                    self.attrs.first_attr_span.get_or_insert(self.peek_span());
                    self.advance();
                    if matches!(self.peek(), TokenKind::LParen) {
                        self.advance(); // outer (
                        if matches!(self.peek(), TokenKind::LParen) {
                            self.advance(); // inner (
                            self.parse_gcc_attribute_list(
                                &mut is_packed,
                                &mut aligned,
                                &mut mode_kind,
                                &mut is_common,
                                &mut scalar_storage_order,
                            );
                            if matches!(self.peek(), TokenKind::RParen) {
                                self.advance();
                            }
                        } else {
                            // Single-paren form
                            while !matches!(self.peek(), TokenKind::RParen | TokenKind::Eof) {
                                if let TokenKind::Identifier(name) = self.peek() {
                                    if name == "packed" || name == "__packed__" {
                                        is_packed = true;
                                    }
                                }
                                self.advance();
                            }
                        }
                        if matches!(self.peek(), TokenKind::RParen) {
                            self.advance();
                        }
                    }
                }
                _ => break,
            }
        }
        (
            is_packed,
            aligned,
            mode_kind,
            is_common,
            scalar_storage_order,
        )
    }

    /// Parse the comma-separated attribute list inside __attribute__((...)).
    fn parse_gcc_attribute_list(
        &mut self,
        is_packed: &mut bool,
        aligned: &mut Option<usize>,
        mode_kind: &mut Option<ModeKind>,
        is_common: &mut bool,
        scalar_storage_order: &mut Option<bool>,
    ) {
        loop {
            match self.peek() {
                TokenKind::Comma => {
                    self.advance();
                }
                TokenKind::RParen | TokenKind::Eof => break,
                TokenKind::Noreturn => {
                    // GCC (measured): a function is silent, an object warns
                    // `'noreturn' attribute ignored`.
                    self.attrs.parsing_attr_targets.push((
                        "noreturn".to_string(),
                        self.peek_span(),
                        attribute_registry::AttributeTarget::Function,
                    ));
                    self.attrs.set_noreturn(true);
                    self.advance();
                }
                TokenKind::Const => {
                    // The `.const` function attribute (measured: ignored on
                    // objects, `on function returning 'void'` for void).
                    self.attrs.parsing_attr_targets.push((
                        "const".to_string(),
                        self.peek_span(),
                        attribute_registry::AttributeTarget::Function,
                    ));
                    self.attrs.set_const_attr(true);
                    self.advance();
                }
                TokenKind::Identifier(name) => {
                    let name = name.clone();
                    let name_span = self.peek_span();
                    self.attribute_entry(
                        &name,
                        name_span,
                        is_packed,
                        aligned,
                        mode_kind,
                        is_common,
                        scalar_storage_order,
                    );
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    /// The classification funnel every list entry goes through: GCC's two
    /// `-Wattributes` phrasings for unknown and ignored attributes, the
    /// explicit-flag inventory for known-but-unimplemented names, and the
    /// applicability record for implemented ones.
    fn attribute_entry(
        &mut self,
        name: &str,
        name_span: crate::common::source::Span,
        is_packed: &mut bool,
        aligned: &mut Option<usize>,
        mode_kind: &mut Option<ModeKind>,
        is_common: &mut bool,
        scalar_storage_order: &mut Option<bool>,
    ) {
        use super::attribute_registry::{AttributeClass, AttributeTarget};
        // ABI-selecting attributes are decided BEFORE the generic funnel,
        // because their three answers are not the funnel's two. See
        // `attribute_registry::abi_effect`: a convention this target already
        // uses is supported (silence), one GCC ignores is warned about in GCC's
        // words, and one this compiler cannot honour is refused rather than
        // emitted as the wrong ABI.
        if self.attribute_abi_effect(name, name_span) {
            self.advance();
            if matches!(self.peek(), TokenKind::LParen) {
                self.skip_balanced_parens();
            }
            return;
        }
        match attribute_registry::classify(name) {
            AttributeClass::Unknown => {
                // GCC 16.2 verbatim (measured): the unknown-name wording.
                self.attribute_warning(
                    format!(
                        "\'{}\' attribute directive ignored",
                        attribute_registry::canonical(name)
                    ),
                    name_span,
                    false,
                );
                self.advance();
                if matches!(self.peek(), TokenKind::LParen) {
                    self.skip_balanced_parens();
                }
            }
            AttributeClass::Unimplemented => {
                // The generated inventory: only an explicit request shows it,
                // so valid uses keep GCC's own (silent) behaviour by default.
                self.attribute_warning(
                    format!(
                        "\'{}\' attribute ignored",
                        attribute_registry::canonical(name)
                    ),
                    name_span,
                    true,
                );
                self.advance();
                if matches!(self.peek(), TokenKind::LParen) {
                    self.skip_balanced_parens();
                }
            }
            AttributeClass::Implemented => {
                if attribute_registry::target(name) != AttributeTarget::Any {
                    // GCC prints and compares the canonical spelling
                    // (`__naked__` -> `naked`, measured); store it once so
                    // every placement message matches its wording.
                    self.attrs.parsing_attr_targets.push((
                        attribute_registry::canonical(name).to_string(),
                        name_span,
                        attribute_registry::target(name),
                    ));
                }
                self.dispatch_gcc_attribute(
                    name,
                    is_packed,
                    aligned,
                    mode_kind,
                    is_common,
                    scalar_storage_order,
                );
            }
        }
    }

    /// Decide an ABI-selecting attribute, returning whether this names one.
    ///
    /// Three verdicts, from `attribute_registry::abi_effect`:
    ///
    /// * the convention this target already uses — supported, silent. Applying
    ///   `sysv_abi` on x86-64 or `cdecl` on i686 *is* the codegen lccc emits, so
    ///   there is nothing to record and nothing to warn about.
    /// * a convention GCC ignores on this target — GCC's own sentence, emitted
    ///   by default (GCC's `-Wattributes` is on by default) and suppressible
    ///   with `-Wno-attributes`, which is exactly how GCC behaves.
    /// * a different convention — an error naming the attribute and the
    ///   convention. This is the R1 guard: a translation unit that declares a
    ///   Microsoft-ABI function must not be compiled into System-V code that
    ///   links happily and returns garbage.
    ///
    /// `CCC_ALLOW_UNSUPPORTED_ABI=1` downgrades the error to GCC's warning for
    /// the narrow case where every object in the program is built by lccc and
    /// the convention therefore never crosses a compiler boundary. It is a
    /// kill switch, not a default: the correct fix is to drop the attribute.
    /// Decide the fate of a convention attribute and consume it.
    ///
    /// Returns `false` for every name that is not a calling-convention
    /// attribute, so the ordinary classification funnel and dispatcher keep
    /// their behaviour; `true` means the attribute is fully handled here.
    ///
    /// This runs *before* the classification funnel on purpose. The funnel has
    /// two answers (`unknown`, `known-but-not-applied`), and a convention needs
    /// four (see [`super::attribute_registry::AbiEffect`]); routing a
    /// convention through the funnel is what produced the silent miscompile
    /// GP-02 describes — `__attribute__((ms_abi))` was classified as known and
    /// parsed without applying, so a caller and a callee could disagree about
    /// where every argument lives and still link.
    fn attribute_abi_effect(&mut self, name: &str, span: crate::common::source::Span) -> bool {
        use super::attribute_registry::{AbiEffect, AbiPlacement, abi_effect, abi_target};
        // Cheap rejection first: an attribute that is not a convention must
        // not pay for a target lookup.
        if !super::attribute_registry::is_abi_attribute(name) {
            return false;
        }
        let placement = if self.attrs.parsing_record_attributes {
            AbiPlacement::Record
        } else {
            AbiPlacement::Other
        };
        let target = abi_target();
        let Some(effect) = abi_effect(name, target, placement) else {
            return false;
        };
        let is_32bit = target == super::attribute_registry::AbiTarget::I686;
        let spelling = super::attribute_registry::canonical(name).to_string();
        // Even a target-ignored convention must obey the attribute grammar.
        // Count only top-level arguments, not commas in nested expressions.
        let expected = usize::from(matches!(
            spelling.as_str(),
            "regparm" | "callee_pop_aggregate_return"
        ));
        let found = self.abi_attribute_argument_count();
        if found != expected {
            self.emit_error(
                format!("wrong number of arguments specified for '{spelling}' attribute"),
                span,
            );
            self.diagnostics
                .note(format!("expected {expected}, found {found}"), span);
            return true;
        }
        match effect {
            // The attribute names the convention this target already emits.
            // Applying nothing is the whole implementation, and gcc is silent
            // here too, so silence is parity as well as correctness.
            AbiEffect::Default => {}
            // Unreachable: `abi_effect` hands implemented conventions back to
            // the dispatcher instead of returning a verdict for them. Kept
            // explicit so a future table edit cannot silently swallow one.
            AbiEffect::Implemented => return false,
            AbiEffect::GccIgnored => {
                let message = format!("'{spelling}' attribute ignored");
                // `explicit_only = false`: gcc emits this without being asked,
                // and it is the only signal that the declaration is not what it
                // looks like.
                self.attribute_warning(message, span, false);
            }
            AbiEffect::OnlyAvailable32Bit => {
                self.attribute_warning(
                    format!("'{spelling}' attribute only available for 32-bit"),
                    span,
                    false,
                );
            }
            AbiEffect::Unsupported => {
                let target = if is_32bit { "i686" } else { "x86-64" };
                let convention = super::attribute_registry::abi_convention(name);
                if std::env::var_os("CCC_ALLOW_UNSUPPORTED_ABI").is_some() {
                    self.attribute_warning(
                        format!(
                            "'{spelling}' attribute ignored (unimplemented ABI, allowed by \
                             CCC_ALLOW_UNSUPPORTED_ABI)"
                        ),
                        span,
                        false,
                    );
                } else {
                    self.emit_error(
                        format!(
                            "'{spelling}' attribute selects {convention}, which this compiler \
                             does not implement for {target}; refusing rather than emitting \
                             {target}'s default convention under a foreign-ABI declaration \
                             (define CCC_ALLOW_UNSUPPORTED_ABI only if every object in the \
                             program is built by this compiler)"
                        ),
                        span,
                    );
                }
            }
        }
        true
    }

    /// The cursor is on the attribute name; absence of parentheses means zero.
    fn abi_attribute_argument_count(&self) -> usize {
        if !matches!(self.peek_nth(1), TokenKind::LParen) {
            return 0;
        }
        if matches!(self.peek_nth(2), TokenKind::RParen) {
            return 0;
        }
        let mut count = 1;
        let mut depth = 1usize;
        let mut offset = 2;
        loop {
            match self.peek_nth(offset) {
                TokenKind::Eof => break,
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                TokenKind::Comma if depth == 1 => count += 1,
                _ => {}
            }
            offset += 1;
        }
        count
    }

    /// Emit a `-Wattributes` diagnostic at the attribute's name token (the
    /// GCC caret position, measured). `explicit_only` gates the
    /// known-but-unimplemented inventory behind a positive `-Wattributes`
    /// or `-Werror=attributes` request.
    fn attribute_warning(
        &mut self,
        message: String,
        span: crate::common::source::Span,
        explicit_only: bool,
    ) {
        if explicit_only
            && !self
                .diagnostics
                .warning_explicitly_requested(crate::common::error::WarningKind::Attributes)
        {
            return;
        }
        self.diagnostics.warning_with_kind(
            message,
            span,
            crate::common::error::WarningKind::Attributes,
        );
    }

    /// Report the declaration-level verdicts recorded while the declarators
    /// were parsed: `-Wstrict-prototypes` and a malformed `cleanup`
    /// attribute. Both belong to the declaration as a whole, and gcc-16
    /// reports them at the declaration's own location -- except for a
    /// function *definition*, where `-Wstrict-prototypes` points at the name.
    /// `decl_start` must already include a leading attribute list when the
    /// declaration has one (`__attribute__((cleanup)) void *p;` anchors at
    /// the attribute, measured).
    pub(super) fn flush_declarator_verdicts(
        &mut self,
        decl_start: crate::common::source::Span,
        definition_name: Option<crate::common::source::Span>,
        unprototyped: bool,
        cleanup_auto_object: bool,
    ) {
        self.attrs.last_decl_name_span = None;
        if unprototyped && !self.c23 {
            let span = definition_name.unwrap_or(decl_start);
            self.diagnostics.warning_with_kind(
                "function declaration isn't a prototype",
                span,
                crate::common::error::WarningKind::StrictPrototypes,
            );
        }
        // One report per attribute: a block-scope declaration can be parsed
        // speculatively and then for real, and both passes record the same
        // attribute. The argument count is checked before the argument's
        // shape, so an arity verdict wins over an identifier verdict for the
        // same attribute (measured: `cleanup` and `cleanup()` report the
        // count, `cleanup(1)` reports the shape).
        let mut by_attr: crate::common::fx_hash::FxHashMap<u32, CleanupDiagnosis> =
            crate::common::fx_hash::FxHashMap::default();
        for (diagnosis, attr_span) in std::mem::take(&mut self.cleanup_diagnoses) {
            let entry = by_attr.entry(attr_span.start);
            entry
                .and_modify(|existing| {
                    if matches!(diagnosis, CleanupDiagnosis::WrongArgumentCount(_))
                        && !matches!(existing, CleanupDiagnosis::WrongArgumentCount(_))
                    {
                        *existing = diagnosis;
                    }
                })
                .or_insert(diagnosis);
        }
        for (attr_start, diagnosis) in by_attr {
            if !self.reported_cleanup_attrs.insert(attr_start) {
                continue;
            }
            match diagnosis {
                CleanupDiagnosis::WrongArgumentCount(found) => {
                    self.diagnostics.error(
                        "wrong number of arguments specified for 'cleanup' attribute",
                        decl_start,
                    );
                    self.diagnostics
                        .note(format!("expected 1, found {found}"), decl_start);
                }
                // GCC checks the argument's *shape* only for a target the
                // attribute can actually apply to. On a function, or on an object
                // with static storage duration, the attribute is ignored and the
                // only diagnostic is the `'cleanup' attribute ignored` warning
                // (measured: `__attribute__((cleanup(1))) void f(void)` and
                // `__attribute__((cleanup(1))) void *g;` are warning-only, while
                // the same argument on a local is an error).
                CleanupDiagnosis::NotAnIdentifier if cleanup_auto_object => {
                    self.diagnostics
                        .error("cleanup argument not an identifier", decl_start);
                }
                CleanupDiagnosis::NotAnIdentifier => {}
            }
        }
    }

    /// The dedupe key for a verdict about one written attribute.
    ///
    /// Two attribute lists on one declaration are two occurrences and keep
    /// two verdicts (gcc reports both), while the same occurrence judged by
    /// two call sites stays one. The bias keeps a first-byte attribute out of
    /// the "no salt" value.
    pub(super) fn attribute_dedup_salt(span: crate::common::source::Span) -> u64 {
        u64::from(span.start) + 1
    }

    /// The declaration-kind half of the applicability rule: once the
    /// specifier and first declarator are known, each recorded
    /// placement-checked attribute either matches or warns exactly as GCC
    /// 16.2 does (measured phrasings, including `naked`'s dedicated wording
    /// and `const`/`pure` on a void-returning function).
    ///
    /// `start` is the declaration's own location and is what GCC reports for
    /// every one of these verdicts, whichever position the attribute was
    /// written in (measured: `int x __attribute__((noreturn));` and
    /// `__attribute__((noreturn)) int x;` both point at the declaration).
    #[track_caller]
    pub(super) fn check_attribute_targets(
        &mut self,
        type_spec: &TypeSpecifier,
        derived: &[crate::frontend::parser::ast::DerivedDeclarator],
        recorded: &[(
            String,
            crate::common::source::Span,
            attribute_registry::AttributeTarget,
        )],
        start: crate::common::source::Span,
    ) {
        use super::attribute_registry::AttributeTarget;
        if recorded.is_empty() {
            return;
        }
        // GCC's location for these verdicts is the declaration; a leading
        // attribute list is part of it, so the earliest list wins when it
        // starts before the specifier.
        let start = match self.attrs.first_attr_span {
            Some(attr) if attr.start < start.start => attr,
            _ => start,
        };
        // The final derived layer names the declaration kind: `void *f(int)`
        // is a function, while `int (*fp)(void)` is an object. This also
        // covers glibc's pointer-returning `__gnu_inline__` prototypes.
        let is_function =
            crate::frontend::parser::ast::DerivedDeclarator::declares_function(derived);
        let is_record = matches!(
            type_spec,
            TypeSpecifier::Struct(..) | TypeSpecifier::Union(..)
        );
        // A typedef's referent is not visible here: never claim a placement
        // violation for it (a false `attribute ignored` would be worse than
        // a missed one on a record typedef).
        let is_typedef = matches!(type_spec, TypeSpecifier::TypedefName(_));
        let returns_void = matches!(type_spec, TypeSpecifier::Void)
            && crate::frontend::parser::ast::DerivedDeclarator::return_pointer_depth(derived) == 0;
        for (name, span, target) in recorded {
            if returns_void && is_function && matches!(name.as_str(), "const" | "pure") {
                self.diagnostics.warning_with_kind_salted(
                    format!("\'{name}\' attribute on function returning 'void'"),
                    start,
                    crate::common::error::WarningKind::Attributes,
                    Self::attribute_dedup_salt(*span),
                );
                continue;
            }
            // `noreturn` on a declaration that names a function *type* --
            // a function, or any object of function-pointer type -- applies
            // to that function type, and gcc-16.2 is silent for it:
            // `int (*fp)(void) __attribute__((noreturn));` and
            // `typedef void (*handler)(int) __attribute__((noreturn));`
            // produce no diagnostic, while `int *ordinary` does (measured).
            // The attribute is carried by the type, not the object, so the
            // object-target verdict does not apply.
            if name == "noreturn" && Self::declares_function_type(derived) {
                continue;
            }
            // A `cleanup` list with the wrong number of arguments is an error,
            // and gcc reports only that error: the applicability warning is
            // suppressed (measured on a function target, where the warning
            // would otherwise also apply).
            if name == "cleanup"
                && self
                    .cleanup_diagnoses
                    .iter()
                    .any(|(d, _)| matches!(d, CleanupDiagnosis::WrongArgumentCount(_)))
            {
                continue;
            }
            let ok = match target {
                AttributeTarget::Any => true,
                AttributeTarget::Function => is_function,
                AttributeTarget::Object => !is_function,
                AttributeTarget::Record => is_record || is_typedef,
            };
            if !ok {
                // Two names have dedicated wordings, both measured on
                // gcc-16.2.0; every other misapplied attribute reuses the
                // generic sentence.
                let message = match name.as_str() {
                    "naked" => format!("\'{name}\' attribute only applies to functions"),
                    "noplt" => format!("\'{name}\' attribute is only applicable on functions"),
                    "standard:noreturn" => {
                        "standard 'noreturn' attribute can only be applied to functions".to_string()
                    }
                    _ => format!("\'{name}\' attribute ignored"),
                };
                // One verdict per attribute list: gcc-16.2 reports
                // `__attribute__((noreturn)) __attribute__((noreturn)) int x;`
                // twice, so the record's own span salts the dedupe key while
                // both diagnostics still point at the declaration.
                self.diagnostics.warning_with_kind_salted(
                    message,
                    start,
                    crate::common::error::WarningKind::Attributes,
                    Self::attribute_dedup_salt(*span),
                );
            }
        }
    }

    /// Whether the declarator declares a function *type* rather than a plain
    /// object: a function, or an object whose type involves a function
    /// declarator (`int (*fp)(void)`, `typedef void (*handler)(int)`).
    fn declares_function_type(derived: &[crate::frontend::parser::ast::DerivedDeclarator]) -> bool {
        derived.iter().any(|d| {
            matches!(
                d,
                crate::frontend::parser::ast::DerivedDeclarator::Function(..)
                    | crate::frontend::parser::ast::DerivedDeclarator::FunctionPointer(..)
            )
        })
    }

    /// Dispatch a single GCC attribute by name.
    fn dispatch_gcc_attribute(
        &mut self,
        name: &str,
        is_packed: &mut bool,
        aligned: &mut Option<usize>,
        mode_kind: &mut Option<ModeKind>,
        is_common: &mut bool,
        scalar_storage_order: &mut Option<bool>,
    ) {
        // Shared by both spellings: `[[gnu::ms_abi]]` reaches this dispatcher
        // directly instead of the `__attribute__` funnel, so the ABI decision
        // has to be here as well — a convention must not be refusable in one
        // syntax and silently ignored in the other. Hoisted above the `match`
        // rather than written as a guard arm: the decision mutably borrows the
        // parser, which a pattern guard may not do.
        let abi_span = self.peek_span();
        if self.attribute_abi_effect(name, abi_span) {
            self.advance();
            if matches!(self.peek(), TokenKind::LParen) {
                self.skip_balanced_parens();
            }
            return;
        }
        match name {
            "constructor" | "__constructor__" => {
                self.attrs.set_constructor(true);
                self.advance();
                if matches!(self.peek(), TokenKind::LParen) {
                    self.skip_balanced_parens();
                }
            }
            "destructor" | "__destructor__" => {
                self.attrs.set_destructor(true);
                self.advance();
                if matches!(self.peek(), TokenKind::LParen) {
                    self.skip_balanced_parens();
                }
            }
            "packed" | "__packed__" => {
                *is_packed = true;
                self.advance();
            }
            "scalar_storage_order" | "__scalar_storage_order__" => {
                self.advance();
                // GCC form: scalar_storage_order("big-endian" | "little-endian" | "default").
                // Only meaningful on struct/union definitions; resolved to the
                // reversed-relative-to-target flag by parse_struct_or_union.
                if let Some(arg) = self.parse_string_attr_arg() {
                    match arg.as_str() {
                        "big-endian" | "__big_endian__" => *scalar_storage_order = Some(true),
                        "little-endian" | "__little_endian__" => {
                            *scalar_storage_order = Some(false)
                        }
                        "default" | "__default__" => *scalar_storage_order = None,
                        _ => {
                            // Unknown argument: ignore like GCC's permerror.
                            *scalar_storage_order = None;
                        }
                    }
                }
            }
            "aligned" | "__aligned__" => {
                self.advance();
                if matches!(self.peek(), TokenKind::LParen) {
                    if let Some(align) = self.parse_alignment_expr() {
                        *aligned = Some(align);
                    }
                }
            }
            "common" | "__common__" => {
                *is_common = true;
                self.advance();
            }
            "transparent_union" | "__transparent_union__" => {
                self.attrs.set_transparent_union(true);
                self.advance();
            }
            "weak" | "__weak__" => {
                self.attrs.set_weak(true);
                self.advance();
            }
            "alias" | "__alias__" => {
                self.advance();
                self.attrs.parsing_alias_target = self.parse_string_attr_arg();
            }
            "ifunc" | "__ifunc__" => {
                let span = self.peek_span();
                self.advance();
                match self.parse_string_attr_arg() {
                    Some(s) if !s.is_empty() => self.attrs.parsing_ifunc_target = Some(s),
                    _ => self.emit_error(
                        "'ifunc' attribute requires a non-empty string argument",
                        span,
                    ),
                }
            }
            "weakref" | "__weakref__" => {
                self.attrs.set_weak(true);
                self.advance();
                self.attrs.parsing_alias_target = self.parse_string_attr_arg();
            }
            "visibility" | "__visibility__" => {
                self.advance();
                self.attrs.parsing_visibility = self.parse_string_attr_arg();
            }
            // GCC's diagnostics, verbatim (GCC 16.2 c-attribs.cc
            // handle_tls_model_attribute). Applying it to a non-TLS object is
            // diagnosed by lowering, which knows the storage class.
            "tls_model" | "__tls_model__" => {
                let span = self.peek_span();
                self.advance();
                match self.parse_string_attr_arg() {
                    Some(arg) => match crate::common::tls::TlsModel::parse(&arg) {
                        Some(model) => self.attrs.parsing_tls_model = Some(model),
                        None => self.emit_error(
                            "'tls_model' argument must be one of 'local-exec', 'initial-exec', 'local-dynamic', or 'global-dynamic'",
                            span,
                        ),
                    },
                    None => self.emit_error("'tls_model' argument not a string", span),
                }
            }
            "section" | "__section__" => {
                self.advance();
                self.attrs.parsing_section = self.parse_string_attr_arg();
            }
            // GCC 16.2 object-placement attributes (measured 2026-10-27):
            //   retain    -> own section `.data.<name>`/`.bss.<name>` + SHF_GNU_RETAIN
            //   noinit    -> `.noinit` (SHT_NOBITS); diagnosed on an initialized object
            //   persistent-> `.persistent` (PROGBITS); diagnosed on an uninitialized one
            "retain" | "__retain__" => {
                self.attrs.set_retain(true);
                self.advance();
            }
            // `noplt` is a property of the *callee*: it selects a GOT-indirect
            // call sequence for that symbol instead of a PLT stub. Recorded on
            // the declaration and honoured by the backends through
            // `BackendState::noplt_symbols`; GCC keeps it silent in every
            // placement `TARGETS` allows.
            "noplt" | "__noplt__" => {
                self.attrs.set_noplt(true);
                self.advance();
            }
            "noinit" | "__noinit__" => {
                self.attrs.set_noinit(true);
                self.advance();
            }
            "persistent" | "__persistent__" => {
                self.attrs.set_persistent(true);
                self.advance();
            }
            "nocommon" | "__nocommon__" => {
                self.attrs.set_nocommon(true);
                self.advance();
            }
            "__diagnose_as"
            | "diagnose_as"
            | "__diagnose_as__"
            | "__diagnose_as_builtin__"
            | "diagnose_as_builtin" => {
                self.advance();
                self.parse_diagnose_as_attr();
            }
            // __attribute__((target("bmi2,lzcnt"))) / target("bmi", "bmi2"):
            // per-function ISA for runtime-dispatched clones (zstd's `_bmi2`
            // bodies, xxHash/simdjson kernels). Consumed by the inliner's
            // ISA-subset rule and the x86-64 emitter's per-function ISA.
            "target" | "__target__" => {
                self.advance();
                self.attrs.parsing_target = self.parse_target_attr_arg();
            }
            "target_clones" | "__target_clones__" => {
                self.advance();
                self.attrs.parsing_target_clones = self.parse_target_attr_arg();
            }
            "uninitialized" | "__uninitialized__" => {
                self.attrs.set_uninitialized(true);
                self.advance();
                self.skip_optional_paren_arg();
            }
            "symver" | "__symver__" => {
                self.advance();
                self.attrs.parsing_symver = self.parse_string_attr_arg();
            }
            "error" | "__error__" | "warning" | "__warning__" => {
                self.attrs.set_error_attr(true);
                self.advance();
                self.skip_optional_paren_arg();
            }
            "noreturn" => {
                self.attrs.set_noreturn(true);
                self.advance();
            }
            "gnu_inline" | "__gnu_inline__" => {
                self.attrs.set_gnu_inline(true);
                self.advance();
            }
            "always_inline" | "__always_inline__" => {
                self.attrs.set_always_inline(true);
                self.advance();
            }
            "noinline" | "__noinline__" => {
                self.attrs.set_noinline(true);
                self.advance();
            }
            "cleanup" | "__cleanup__" => {
                self.advance();
                self.parse_cleanup_attr();
            }
            "counted_by" | "__counted_by__" => {
                self.advance();
                self.parse_counted_by_attr();
            }
            "mode" | "__mode__" => {
                self.advance();
                self.parse_mode_attr(mode_kind);
            }
            "vector_size" | "__vector_size__" => {
                self.advance();
                self.parse_vector_size_attr();
            }
            "ext_vector_type" | "__ext_vector_type__" => {
                self.advance();
                self.parse_ext_vector_type_attr();
            }
            "used" | "__used__" => {
                self.attrs.set_used(true);
                self.advance();
            }
            "unused" | "__unused__" => {
                self.attrs.set_maybe_unused(true);
                self.advance();
                self.skip_optional_paren_arg();
            }
            "fastcall" | "__fastcall__" => {
                let span = self.peek_span();
                self.attrs.set_fastcall(true);
                self.advance();
                // GCC: "fastcall and regparm attributes are not compatible"
                if self.attrs.parsing_regparm.is_some() {
                    self.emit_error("fastcall and regparm attributes are not compatible", span);
                }
            }
            "regparm" | "__regparm__" => {
                // GCC i386 `__attribute__((regparm(N)))`: pass the first N
                // integer arguments in EAX, EDX, ECX. The argument is
                // required (bare `regparm` is an error in GCC); values above
                // 3 warn and clamp to 3.
                let span = self.peek_span();
                self.advance();
                if !matches!(self.peek(), TokenKind::LParen) {
                    self.emit_error(
                        "wrong number of arguments specified for 'regparm' attribute",
                        span,
                    );
                } else if let Some(v) = self.parse_alignment_expr() {
                    let n = v.min(3) as u8;
                    if v > 3 {
                        self.diagnostics
                            .warning("argument to 'regparm' attribute larger than 3", span);
                    }
                    self.attrs.parsing_regparm = Some(n);
                    if self.attrs.parsing_fastcall() {
                        self.emit_error("fastcall and regparm attributes are not compatible", span);
                    }
                }
            }
            "naked" | "__naked__" => {
                self.attrs.set_naked(true);
                self.advance();
            }
            "pure" | "__pure__" => {
                self.attrs.set_pure(true);
                self.advance();
            }
            "const" | "__const__" => {
                self.attrs.set_const_attr(true);
                self.advance();
            }
            "cold" | "__cold__" => {
                self.attrs.set_cold(true);
                self.advance();
            }
            // __attribute__((no_instrument_function)) — skip the mcount/__fentry__
            // prologue. The Linux kernel marks early-boot, NMI, and other
            // contexts where the tracer infrastructure can't run with this.
            "no_instrument_function" | "__no_instrument_function__" => {
                self.attrs.set_no_instrument(true);
                self.advance();
            }
            "stack_protect" | "__stack_protect__" => {
                self.attrs.parsing_stack_protect = true;
                self.advance();
            }
            "no_stack_protector" | "__no_stack_protector__" => {
                self.attrs.parsing_no_stack_protector = true;
                self.advance();
            }
            "externally_visible" | "__externally_visible__" => {
                self.attrs.parsing_externally_visible = true;
                self.advance();
            }
            "zero_call_used_regs" | "__zero_call_used_regs__" => {
                let span = self.peek_span();
                self.advance();
                match self.parse_string_attr_arg() {
                    Some(mode) => {
                        if crate::backend::zero_call_used_regs::ZeroCallUsedRegs::parse(&mode)
                            .is_some()
                        {
                            self.attrs.parsing_zero_call_used_regs = Some(mode);
                        } else {
                            self.emit_error(
                                format!(
                                    "unrecognized 'zero_call_used_regs' attribute argument '{mode}'"
                                ),
                                span,
                            );
                        }
                    }
                    None => {
                        self.emit_error(
                            "'zero_call_used_regs' attribute requires a string argument",
                            span,
                        );
                    }
                }
            }
            "no_sanitize" | "__no_sanitize__" => {
                self.advance();
                if let Some(arg) = self.parse_target_attr_arg() {
                    match &mut self.attrs.parsing_no_sanitize {
                        Some(prev) if !prev.is_empty() => {
                            prev.push(',');
                            prev.push_str(&arg);
                        }
                        slot => *slot = Some(arg),
                    }
                }
            }
            "no_sanitize_undefined" | "__no_sanitize_undefined__" => {
                self.advance();
                self.skip_optional_paren_arg();
                match &mut self.attrs.parsing_no_sanitize {
                    Some(prev) if !prev.is_empty() => prev.push_str(",undefined"),
                    slot => *slot = Some("undefined".to_string()),
                }
            }
            "optimize" | "__optimize__" => {
                self.advance();
                // Recognize optimize("omit-frame-pointer") as equivalent to naked,
                // and optimize("no-stack-protector") as no_stack_protector.
                let mut found_omit_fp = false;
                let mut found_no_ssp = false;
                if matches!(self.peek(), TokenKind::LParen) {
                    // Peek at the string argument inside parens without consuming
                    if self.pos + 1 < self.tokens.len() {
                        if let TokenKind::StringLiteral(ref s) = self.tokens[self.pos + 1].kind {
                            if s.contains("omit-frame-pointer") {
                                found_omit_fp = true;
                            }
                            if s.contains("no-stack-protector") {
                                found_no_ssp = true;
                            }
                        }
                    }
                    self.skip_balanced_parens();
                }
                if found_omit_fp {
                    self.attrs.set_naked(true);
                }
                if found_no_ssp {
                    self.attrs.parsing_no_stack_protector = true;
                }
            }
            "address_space" | "__address_space__" => {
                self.advance();
                self.parse_address_space_attr();
            }
            _ => {
                // Reachable only through the C23 spelling `[[gnu::name]]`,
                // which dispatches here directly instead of passing the
                // `__attribute__((...))` classification funnel. Answer with
                // the same registry classes so both spellings agree:
                //   * not a GCC attribute at all -> `'x' attribute directive
                //     ignored`
                //   * GCC attribute whose semantics are not implemented ->
                //     `'x' attribute ignored` under an explicit `-Wattributes`
                //   * registry-implemented names never arrive here; their
                //     dispatch arms above already consumed them.
                use crate::frontend::parser::attribute_registry::{
                    self as registry, AttributeClass,
                };
                let span = self.peek_span();
                let spelling = registry::canonical(name).to_string();
                match registry::classify(name) {
                    AttributeClass::Unknown => self.diagnostics.warning_with_kind(
                        format!("'{spelling}' attribute directive ignored"),
                        span,
                        crate::common::error::WarningKind::Attributes,
                    ),
                    AttributeClass::Unimplemented => self.attribute_warning(
                        format!("'{spelling}' attribute ignored"),
                        span,
                        true,
                    ),
                    AttributeClass::Implemented => {}
                }
                self.advance();
                if matches!(self.peek(), TokenKind::LParen) {
                    self.skip_balanced_parens();
                }
            }
        }
    }

    /// Parse a parenthesized string argument: ("string1" "string2"...).
    /// Returns Some(concatenated) if non-empty, None otherwise.
    fn parse_string_attr_arg(&mut self) -> Option<String> {
        if !matches!(self.peek(), TokenKind::LParen) {
            return None;
        }
        self.advance(); // consume (
        let mut result = String::with_capacity(64);
        while let TokenKind::StringLiteral(s) = self.peek() {
            result.push_str(s);
            self.advance();
        }
        if matches!(self.peek(), TokenKind::RParen) {
            self.advance();
        }
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Parse `("a,b" "c", "d")`: adjacent literals concatenate (C string
    /// pasting), commas separate arguments; the result is comma-joined so
    /// `target("bmi", "bmi2")` and `target("bmi,bmi2")` are identical.
    /// Malformed contents are skipped to the matching paren.
    fn parse_target_attr_arg(&mut self) -> Option<String> {
        if !matches!(self.peek(), TokenKind::LParen) {
            return None;
        }
        self.advance(); // consume (
        let mut parts: Vec<String> = Vec::new();
        let mut cur = String::new();
        loop {
            match self.peek() {
                TokenKind::StringLiteral(s) => {
                    cur.push_str(s);
                    self.advance();
                }
                TokenKind::Comma => {
                    parts.push(std::mem::take(&mut cur));
                    self.advance();
                }
                TokenKind::RParen => {
                    self.advance();
                    break;
                }
                TokenKind::Eof => break,
                _ => {
                    // Not a string list: recover by skipping to the `)`
                    // that closes this argument list.
                    let mut depth = 1usize;
                    while depth > 0 && !matches!(self.peek(), TokenKind::Eof) {
                        match self.peek() {
                            TokenKind::LParen => depth += 1,
                            TokenKind::RParen => depth -= 1,
                            _ => {}
                        }
                        self.advance();
                    }
                    return None;
                }
            }
        }
        parts.push(cur);
        let joined = parts
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        (!joined.is_empty()).then_some(joined)
    }

    /// Skip an optional parenthesized argument (consuming everything inside).
    fn skip_optional_paren_arg(&mut self) {
        if !matches!(self.peek(), TokenKind::LParen) {
            return;
        }
        self.advance();
        while !matches!(self.peek(), TokenKind::RParen | TokenKind::Eof) {
            self.advance();
        }
        if matches!(self.peek(), TokenKind::RParen) {
            self.advance();
        }
    }

    /// Parse cleanup(func_name) attribute.
    /// Parse `__diagnose_as(__builtin_NAME, ...)`: record the builtin NAME
    /// (the `__builtin_` prefix is stripped) and skip the remaining
    /// argument-position list (analyzer metadata; every folding consumer
    /// maps the declared arguments onto the builtin identically).
    fn parse_diagnose_as_attr(&mut self) {
        if !matches!(self.peek(), TokenKind::LParen) {
            return;
        }
        self.advance();
        if let TokenKind::Identifier(name) = self.peek() {
            if let Some(builtin) = name.strip_prefix("__builtin_") {
                self.attrs.parsing_diagnose_as = Some(builtin.to_string());
            }
            self.advance();
        }
        // Skip the comma-separated argument-position list to the matching
        // closing paren (depth tracks nested parens inside the attr args).
        let mut depth = 1usize;
        while depth > 0 {
            match self.peek() {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => depth -= 1,
                TokenKind::Eof => break,
                _ => {}
            }
            self.advance();
        }
    }

    fn parse_counted_by_attr(&mut self) {
        if !matches!(self.peek(), TokenKind::LParen) {
            return;
        }
        self.advance();
        if let TokenKind::Identifier(member) = self.peek() {
            self.attrs.parsing_counted_by = Some(member.clone());
            self.advance();
        }
        while !matches!(self.peek(), TokenKind::RParen | TokenKind::Eof) {
            self.advance();
        }
        if matches!(self.peek(), TokenKind::RParen) {
            self.advance();
        }
    }

    fn parse_cleanup_attr(&mut self) {
        let attr_span = self.peek_span();
        if !matches!(self.peek(), TokenKind::LParen) {
            // `__attribute__((cleanup))`: GCC counts zero arguments.
            self.cleanup_diagnoses
                .push((CleanupDiagnosis::WrongArgumentCount(0), attr_span));
            return;
        }
        self.advance(); // '('
        // Split the argument list on top-level commas and keep the tokens of
        // each argument. The previous version consumed at most one identifier
        // and then gave up, which left the remaining tokens in the stream
        // (cascading parse errors for `cleanup()`) or accepted a malformed
        // attribute without a word (`cleanup(1)`).
        let mut groups: Vec<Vec<TokenKind>> = Vec::new();
        let mut current: Vec<TokenKind> = Vec::new();
        let mut depth = 0i32;
        loop {
            match self.peek().clone() {
                TokenKind::Eof => break,
                TokenKind::RParen if depth == 0 => {
                    self.advance();
                    break;
                }
                TokenKind::Comma if depth == 0 => {
                    self.advance();
                    groups.push(std::mem::take(&mut current));
                }
                tk @ (TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace) => {
                    depth += 1;
                    current.push(tk);
                    self.advance();
                }
                tk @ (TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace) => {
                    depth -= 1;
                    current.push(tk);
                    self.advance();
                }
                tk => {
                    current.push(tk);
                    self.advance();
                }
            }
        }
        if !current.is_empty() || !groups.is_empty() {
            groups.push(current);
        }
        match groups.len() {
            1 if groups[0].len() == 1 => {
                if let TokenKind::Identifier(name) = &groups[0][0] {
                    self.attrs.parsing_cleanup_fn = Some(name.clone());
                } else {
                    self.attrs.parsing_cleanup_fn = None;
                    self.cleanup_diagnoses
                        .push((CleanupDiagnosis::NotAnIdentifier, attr_span));
                }
            }
            1 => {
                // Several tokens that are not a bare identifier: `cleanup((f))`,
                // `cleanup(a b)`, `cleanup("f")`.
                self.attrs.parsing_cleanup_fn = None;
                self.cleanup_diagnoses
                    .push((CleanupDiagnosis::NotAnIdentifier, attr_span));
            }
            n => {
                self.attrs.parsing_cleanup_fn = None;
                self.cleanup_diagnoses
                    .push((CleanupDiagnosis::WrongArgumentCount(n), attr_span));
            }
        }
    }

    /// Parse mode(QI|HI|SI|DI|TI|word|pointer) attribute.
    fn parse_mode_attr(&mut self, mode_kind: &mut Option<ModeKind>) {
        if !matches!(self.peek(), TokenKind::LParen) {
            return;
        }
        self.advance();
        if let TokenKind::Identifier(mode_name) = self.peek() {
            let is_32bit = crate::common::types::target_is_32bit();
            *mode_kind = match mode_name.as_str() {
                "QI" | "__QI__" | "byte" | "__byte__" => Some(ModeKind::QI),
                "HI" | "__HI__" => Some(ModeKind::HI),
                "SI" | "__SI__" => Some(ModeKind::SI),
                "DI" | "__DI__" => Some(ModeKind::DI),
                "TI" | "__TI__" => {
                    if is_32bit {
                        let span = self.peek_span();
                        self.emit_error("TI mode is not supported on 32-bit targets", span);
                        *mode_kind
                    } else {
                        Some(ModeKind::TI)
                    }
                }
                "word" | "__word__" | "pointer" | "__pointer__" => {
                    if is_32bit {
                        Some(ModeKind::SI)
                    } else {
                        Some(ModeKind::DI)
                    }
                }
                _ => *mode_kind,
            };
        }
        while !matches!(self.peek(), TokenKind::RParen | TokenKind::Eof) {
            self.advance();
        }
        if matches!(self.peek(), TokenKind::RParen) {
            self.advance();
        }
    }

    /// Parse-time constant evaluation with parser-side typedef size
    /// resolution: handles `sizeof(typedef-name)` inside vector_size
    /// expressions (e.g. `16 * sizeof(int64_t)`), which the shared static
    /// evaluator cannot resolve because typedef sizes live in parser state
    /// (`typedef_type_specs`).  Falls back to `None` for struct/union-typedef
    /// sizes, which remain deferred to sema via
    /// `parsing_vector_size_expr`.
    fn eval_vector_size_with_typedefs(&self, expr: &Expr) -> Option<i64> {
        match expr {
            Expr::BinaryOp(op, lhs, rhs, _) => {
                let l = self.eval_vector_size_with_typedefs(lhs)?;
                let r = self.eval_vector_size_with_typedefs(rhs)?;
                match op {
                    BinOp::Add => Some(l.wrapping_add(r)),
                    BinOp::Sub => Some(l.wrapping_sub(r)),
                    BinOp::Mul => Some(l.wrapping_mul(r)),
                    BinOp::Div if r != 0 => Some(l.wrapping_div(r)),
                    BinOp::Mod if r != 0 => Some(l.wrapping_rem(r)),
                    BinOp::Shl => Some(l.wrapping_shl(r as u32)),
                    BinOp::Shr => Some(l.wrapping_shr(r as u32)),
                    BinOp::BitOr => Some(l | r),
                    BinOp::BitAnd => Some(l & r),
                    BinOp::BitXor => Some(l ^ r),
                    _ => None,
                }
            }
            Expr::UnaryOp(UnaryOp::Neg, inner, _) => {
                Some(self.eval_vector_size_with_typedefs(inner)?.wrapping_neg())
            }
            Expr::UnaryOp(UnaryOp::Plus, inner, _) => self.eval_vector_size_with_typedefs(inner),
            Expr::Sizeof(arg, _) => match arg.as_ref() {
                SizeofArg::Type(ts) => {
                    // Walk typedef chains to their base spec: the stored spec
                    // for a chained typedef (`typedef a_t b_t;`) is itself a
                    // TypedefName, and the static try_sizeof_type_spec cannot
                    // resolve those.  Chains are acyclic (a typedef name is
                    // only usable after its declaration), so this terminates.
                    let mut spec: &TypeSpecifier = ts;
                    loop {
                        match spec {
                            TypeSpecifier::TypedefName(name) => {
                                spec = self.typedef_type_specs.get(name)?;
                            }
                            other => {
                                return Some(Self::try_sizeof_type_spec(other)? as i64);
                            }
                        }
                    }
                }
                _ => None,
            },
            _ => Self::eval_const_int_expr(expr),
        }
    }

    /// Parse vector_size(expr) attribute.
    fn parse_vector_size_attr(&mut self) {
        if !matches!(self.peek(), TokenKind::LParen) {
            return;
        }
        self.advance();
        let expr = self.parse_assignment_expr();
        let enums = if self.enum_constants.is_empty() {
            None
        } else {
            Some(&self.enum_constants)
        };
        let tag_aligns = if self.struct_tag_alignments.is_empty() {
            None
        } else {
            Some(&self.struct_tag_alignments)
        };
        if let Some(size) = Self::eval_const_int_expr_with_enums(&expr, enums, tag_aligns) {
            self.attrs.parsing_vector_size = Some(size as usize);
        } else if let Some(size) = self.eval_vector_size_with_typedefs(&expr) {
            // The static evaluator cannot resolve sizeof(typedef-name)
            // (typedef sizes live in parser state); retry with the
            // typedef-aware walk so e.g. `vector_size(16 * sizeof(int64_t))`
            // resolves right here.
            self.attrs.parsing_vector_size = Some(size as usize);
        } else {
            // Not parse-time evaluable (e.g. sizeof(struct typedef)): defer
            // the raw expression to sema, where all sizes are resolved.
            self.attrs.parsing_vector_size_expr = Some(Box::new(expr));
        }
        if matches!(self.peek(), TokenKind::RParen) {
            self.advance();
        }
    }

    /// Parse ext_vector_type(N) attribute (Clang-style vector type).
    /// Stores the element count N; the total size is computed in lowering as N * sizeof(elem).
    fn parse_ext_vector_type_attr(&mut self) {
        if !matches!(self.peek(), TokenKind::LParen) {
            return;
        }
        self.advance();
        let expr = self.parse_assignment_expr();
        let enums = if self.enum_constants.is_empty() {
            None
        } else {
            Some(&self.enum_constants)
        };
        let tag_aligns = if self.struct_tag_alignments.is_empty() {
            None
        } else {
            Some(&self.struct_tag_alignments)
        };
        match Self::eval_const_int_expr_with_enums(&expr, enums, tag_aligns)
            // The static evaluator cannot see parser-side typedef sizes.
            // Match vector_size(expr)'s typedef-aware constant-expression
            // fallback rather than rejecting a valid sizeof(alias_t) count.
            .or_else(|| self.eval_vector_size_with_typedefs(&expr))
            .and_then(|nelem| usize::try_from(nelem).ok())
            .filter(|&nelem| nelem != 0)
        {
            Some(nelem) => self.attrs.parsing_ext_vector_nelem = Some(nelem),
            None => self.emit_error(
                "ext_vector_type requires a positive integer constant expression",
                expr.span(),
            ),
        }
        if matches!(self.peek(), TokenKind::RParen) {
            self.advance();
        }
    }

    /// Parse address_space(__seg_gs|__seg_fs) attribute.
    fn parse_address_space_attr(&mut self) {
        if !matches!(self.peek(), TokenKind::LParen) {
            return;
        }
        self.advance();
        match self.peek() {
            TokenKind::SegGs => {
                self.advance();
                self.attrs.parsing_address_space = AddressSpace::SegGs;
            }
            TokenKind::SegFs => {
                self.advance();
                self.attrs.parsing_address_space = AddressSpace::SegFs;
            }
            _ => {
                self.advance();
            }
        }
        if matches!(self.peek(), TokenKind::RParen) {
            self.advance();
        }
    }

    /// Skip __asm__("..."), __attribute__(...), and __extension__ after declarators.
    /// Returns (mode_kind, aligned_value, asm_register).
    pub(super) fn skip_asm_and_attributes(
        &mut self,
    ) -> (Option<ModeKind>, Option<usize>, Option<String>) {
        let (_, _, mk, _, aligned, asm_reg) = self.parse_asm_and_attributes();
        (mk, aligned, asm_reg)
    }

    /// Parse __asm__("..."), __attribute__(...), and __extension__ after declarators.
    /// Returns (is_constructor, is_destructor, mode_kind, is_common, aligned_value, asm_register).
    /// The asm_register captures the register name from `register var __asm__("regname")`.
    pub(super) fn parse_asm_and_attributes(
        &mut self,
    ) -> (
        bool,
        bool,
        Option<ModeKind>,
        bool,
        Option<usize>,
        Option<String>,
    ) {
        let mut is_constructor = false;
        let mut is_destructor = false;
        let mut mode_kind: Option<ModeKind> = None;
        let mut has_common = false;
        let mut aligned: Option<usize> = None;
        let mut asm_register: Option<String> = None;
        loop {
            match self.peek() {
                TokenKind::Asm => {
                    self.advance();
                    self.consume_if(&TokenKind::Volatile);
                    if matches!(self.peek(), TokenKind::LParen) {
                        // Try to extract the asm register name: __asm__("regname")
                        // This is a single string literal inside parentheses.
                        asm_register = asm_register.or_else(|| self.try_parse_asm_register_name());
                    }
                }
                TokenKind::Attribute => {
                    let (_, attr_aligned, mk, common, _) = self.parse_gcc_attributes();
                    mode_kind = mode_kind.or(mk);
                    has_common = has_common || common;
                    if let Some(a) = attr_aligned {
                        aligned = Some(aligned.map_or(a, |prev| prev.max(a)));
                    }
                    if self.attrs.parsing_constructor() {
                        is_constructor = true;
                    }
                    if self.attrs.parsing_destructor() {
                        is_destructor = true;
                    }
                }
                TokenKind::Extension => {
                    self.advance();
                }
                _ => break,
            }
        }
        (
            is_constructor,
            is_destructor,
            mode_kind,
            has_common,
            aligned,
            asm_register,
        )
    }

    /// Try to extract a label/register name from __asm__("name") on a declaration.
    /// Called when the current token is LParen after __asm__. Handles both:
    /// - Register pinning: `register int x __asm__("rbx")`
    /// - Linker symbol redirect: `extern int foo(...) __asm__("" "__xpg_strerror_r")`
    ///
    /// Supports concatenation of adjacent string literals (e.g., `__asm__("" "name")`).
    /// Returns Some("name") if a non-empty label was found, None otherwise.
    fn try_parse_asm_register_name(&mut self) -> Option<String> {
        // Save position so we can fall back
        let saved_pos = self.pos;
        self.advance(); // consume '('
        // Concatenate adjacent string literals: __asm__("" "name") -> "name"
        let mut combined = String::new();
        let mut found_string = false;
        while let TokenKind::StringLiteral(s) = self.peek() {
            combined.push_str(s);
            found_string = true;
            self.advance();
        }
        if found_string && matches!(self.peek(), TokenKind::RParen) {
            self.advance(); // consume ')'
            if !combined.is_empty() {
                // Strip leading '%' prefix from register names.
                // In C source, register names in asm() may use the GCC inline asm
                // convention with a '%' prefix (e.g., asm("%rdx") or asm("%" "rdx")),
                // but the actual register name used internally should be just "rdx".
                let combined = combined.trim_start_matches('%').to_string();
                if !combined.is_empty() {
                    return Some(combined);
                }
            }
            return None;
        }
        // Not a simple string literal sequence, restore and skip
        self.pos = saved_pos;
        self.skip_balanced_parens();
        None
    }

    // === Pragma pack handling ===

    /// Check if current token is a pragma pack directive and handle it.
    /// Returns true if a pragma pack token was consumed.
    pub(super) fn handle_pragma_pack_token(&mut self) -> bool {
        match self.peek() {
            TokenKind::PragmaPackSet(n) => {
                let n = *n;
                self.advance();
                // pack(0) means reset to default (natural alignment)
                self.pragma_pack_align = if n == 0 { None } else { Some(n) };
                true
            }
            TokenKind::PragmaPackPush(n) => {
                let n = *n;
                self.advance();
                // Push current alignment onto stack
                self.pragma_pack_stack.push(self.pragma_pack_align);
                // pack(push, 0) means push and reset to default (natural alignment)
                // pack(push, N) means push and set to N
                if n == 0 {
                    self.pragma_pack_align = None;
                } else {
                    self.pragma_pack_align = Some(n);
                }
                true
            }
            TokenKind::PragmaPackPushOnly => {
                self.advance();
                // pack(push) - push current alignment without changing it
                self.pragma_pack_stack.push(self.pragma_pack_align);
                true
            }
            TokenKind::PragmaPackPop => {
                self.advance();
                // Pop previous alignment from stack
                if let Some(prev) = self.pragma_pack_stack.pop() {
                    self.pragma_pack_align = prev;
                } else {
                    // Stack underflow: reset to default
                    self.pragma_pack_align = None;
                }
                true
            }
            TokenKind::PragmaPackReset => {
                self.advance();
                self.pragma_pack_align = None;
                true
            }
            _ => false,
        }
    }

    /// Handle #pragma GCC visibility push/pop synthetic tokens.
    /// Returns true if a token was consumed.
    pub(super) fn handle_pragma_visibility_token(&mut self) -> bool {
        match self.peek() {
            TokenKind::PragmaVisibilityPush(vis) => {
                let vis = vis.clone();
                self.advance();
                // Push current visibility and set new default. Keep `"default"`
                // explicitly on `pragma_default_visibility` so `#pragma GCC
                // visibility push(default)` overrides `-fvisibility=hidden`
                // while active, and lowering strips `"default"` to ELF
                // STV_DEFAULT afterwards.
                if let Some(ref current) = self.pragma_default_visibility {
                    self.pragma_visibility_stack.push(current.clone());
                } else {
                    // Push a sentinel for "no pragma active"
                    self.pragma_visibility_stack.push(String::new());
                }
                self.pragma_default_visibility = Some(vis);
                true
            }
            TokenKind::PragmaVisibilityPop => {
                self.advance();
                // Pop previous visibility
                if let Some(prev) = self.pragma_visibility_stack.pop() {
                    if prev.is_empty() {
                        self.pragma_default_visibility = None;
                    } else {
                        self.pragma_default_visibility = Some(prev);
                    }
                } else {
                    // Stack underflow: reset to no pragma
                    self.pragma_default_visibility = None;
                }
                true
            }
            _ => false,
        }
    }

    pub(super) fn skip_balanced_parens(&mut self) {
        if !matches!(self.peek(), TokenKind::LParen) {
            return;
        }
        let mut depth = 0i32;
        loop {
            match self.peek() {
                TokenKind::LParen => {
                    depth += 1;
                    self.advance();
                }
                TokenKind::RParen => {
                    depth -= 1;
                    self.advance();
                    if depth <= 0 {
                        break;
                    }
                }
                TokenKind::Eof => break,
                _ => {
                    self.advance();
                }
            }
        }
    }

    /// Skip optional GNU label attributes after a label colon.
    /// GNU C allows `label: __attribute__((unused));` to suppress unused-label warnings.
    /// We consume the entire `__attribute__((...))` token sequence and discard it.
    pub(super) fn skip_label_attributes(&mut self) {
        while matches!(self.peek(), TokenKind::Attribute) {
            self.advance(); // consume __attribute__
            // Expect __attribute__((...)) — two levels of parens
            if matches!(self.peek(), TokenKind::LParen) {
                self.skip_balanced_parens();
            }
        }
    }

    /// True when the next token terminates the enclosing block, i.e. a
    /// `case` / `default` / identifier label is the final item before the
    /// closing brace of a compound statement (or the token stream ends).
    ///
    /// This accepts the GNU/C23 form where a label directly precedes `}`:
    ///
    /// ```c
    /// switch (x) {
    ///     case 1: return 10;
    ///     case 2:          // falls out of the switch
    /// }
    /// ```
    ///
    /// Callers attach a null statement in that situation; execution simply
    /// continues with the statement following the enclosing block.
    pub(super) fn label_at_block_end(&self) -> bool {
        matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof)
    }

    /// Parse the parenthesized argument of `aligned(expr)` in __attribute__.
    /// Expects the opening `(` to be the current token (not yet consumed).
    /// Parses and evaluates a constant expression, consuming through the closing `)`.
    /// Returns Some(alignment) on success, None on failure.
    pub(super) fn parse_alignment_expr(&mut self) -> Option<usize> {
        if !matches!(self.peek(), TokenKind::LParen) {
            return None;
        }
        self.advance(); // consume opening (
        let expr = self.parse_assignment_expr();
        // Consume closing )
        if matches!(self.peek(), TokenKind::RParen) {
            self.advance();
        }
        // If the expression is sizeof(type), capture the type so sema/lowerer can
        // recompute with accurate struct/union layout info (the parser's sizeof
        // uses a conservative default for struct/union types).
        if let Expr::Sizeof(ref arg, _) = expr {
            if let SizeofArg::Type(ref ts) = **arg {
                self.attrs.parsed_alignment_sizeof_type = Some(ts.clone());
            }
        }
        let enums = if self.enum_constants.is_empty() {
            None
        } else {
            Some(&self.enum_constants)
        };
        let tag_aligns = if self.struct_tag_alignments.is_empty() {
            None
        } else {
            Some(&self.struct_tag_alignments)
        };
        Self::eval_const_int_expr_with_enums(&expr, enums, tag_aligns).map(|v| v as usize)
    }

    /// Parse the parenthesized argument of `_Alignas(...)`.
    /// _Alignas can take either a type-name or a constant expression.
    /// Returns Some(alignment) on success, None on failure.
    pub(super) fn parse_alignas_argument(&mut self) -> Option<usize> {
        if !matches!(self.peek(), TokenKind::LParen) {
            return None;
        }
        // Try type-name first using save/restore
        let save = self.pos;
        let save_typedef = self.attrs.parsing_typedef();
        self.advance(); // consume (
        if self.is_type_specifier() {
            // parse_nested_type_name scopes the pending address-space
            // qualifier: the alignas type-name argument must not steal an
            // enclosing declaration's `__seg_gs`/`__seg_fs`, and the
            // enclosing qualifier is restored before any return (including
            // the backtrack below).
            if let Some(result_type) = self.parse_nested_type_name() {
                if matches!(self.peek(), TokenKind::RParen) {
                    self.advance(); // consume )
                    // Save the type specifier so the lowerer can resolve typedefs
                    // and compute accurate alignment (parser can't resolve typedefs).
                    self.attrs.parsed_alignas_type = Some(result_type.clone());
                    let tag_aligns = if self.struct_tag_alignments.is_empty() {
                        None
                    } else {
                        Some(&self.struct_tag_alignments)
                    };
                    // `None` (typedef, enum, ...): the layout code resolves
                    // the saved type; no guessed number may stand in for it.
                    return Self::alignof_type_spec(&result_type, tag_aligns);
                }
            }
        }
        // Backtrack and try as constant expression
        self.pos = save;
        self.attrs.set_typedef(save_typedef);
        self.advance(); // consume (
        let expr = self.parse_assignment_expr();
        if matches!(self.peek(), TokenKind::RParen) {
            self.advance();
        }
        let enums = if self.enum_constants.is_empty() {
            None
        } else {
            Some(&self.enum_constants)
        };
        let tag_aligns = if self.struct_tag_alignments.is_empty() {
            None
        } else {
            Some(&self.struct_tag_alignments)
        };
        Self::eval_const_int_expr_with_enums(&expr, enums, tag_aligns).map(|v| v as usize)
    }

    /// Try to compute sizeof for a type specifier. Returns None for types
    /// whose size cannot be determined at parse time (struct, union, typedef).
    /// Use this in constant expression evaluation where a wrong default would
    /// cause spurious errors (e.g., _Static_assert).
    pub(super) fn try_sizeof_type_spec(ts: &TypeSpecifier) -> Option<usize> {
        use crate::common::types::target_ptr_size;
        let ptr_sz = target_ptr_size();
        match ts {
            TypeSpecifier::Void
            | TypeSpecifier::Bool
            | TypeSpecifier::Char
            | TypeSpecifier::SignedChar
            | TypeSpecifier::UnsignedChar
            | TypeSpecifier::PlainUnsignedChar => Some(1),
            TypeSpecifier::Short | TypeSpecifier::UnsignedShort => Some(2),
            TypeSpecifier::Int
            | TypeSpecifier::UnsignedInt
            | TypeSpecifier::Signed
            | TypeSpecifier::Unsigned
            | TypeSpecifier::Float => Some(4),
            TypeSpecifier::Long | TypeSpecifier::UnsignedLong => Some(ptr_sz),
            TypeSpecifier::LongLong | TypeSpecifier::UnsignedLongLong | TypeSpecifier::Double => {
                Some(8)
            }
            TypeSpecifier::Pointer(_, _)
            | TypeSpecifier::FunctionPointer(..)
            | TypeSpecifier::NullPtrT => Some(ptr_sz),
            TypeSpecifier::Int128 | TypeSpecifier::UnsignedInt128 => Some(16),
            TypeSpecifier::LongDouble => Some(if ptr_sz == 4 { 12 } else { 16 }),
            TypeSpecifier::ComplexFloat => Some(8),
            TypeSpecifier::ComplexDouble => Some(16),
            TypeSpecifier::ComplexLongDouble => Some(if ptr_sz == 4 { 24 } else { 32 }),
            TypeSpecifier::Array(elem, Some(size_expr)) => {
                let elem_size = Self::try_sizeof_type_spec(elem)?;
                let count = Self::eval_const_int_expr(size_expr)? as usize;
                Some(elem_size * count)
            }
            TypeSpecifier::Array(_, None) => Some(0),
            // Enum, struct, union, typedef: size not reliably known at parse time.
            // Enums can be __packed (1 byte), and struct/union sizes depend on layout.
            _ => None,
        }
    }

    /// Check if a type specifier is an unsigned type.
    /// Used by the parser-level constant evaluator for cast truncation.
    pub(super) fn is_unsigned_type_spec(ts: &TypeSpecifier) -> bool {
        matches!(
            ts,
            TypeSpecifier::UnsignedChar
                | TypeSpecifier::PlainUnsignedChar
                | TypeSpecifier::UnsignedShort
                | TypeSpecifier::UnsignedInt
                | TypeSpecifier::Unsigned
                | TypeSpecifier::UnsignedLong
                | TypeSpecifier::UnsignedLongLong
                | TypeSpecifier::UnsignedInt128
                | TypeSpecifier::Bool
                | TypeSpecifier::Pointer(_, _)
        )
    }

    /// Alignment (in bytes) of a type specifier when the parser knows it
    /// EXACTLY, else `None`. Typedef targets, enums (whose size depends on
    /// their values), `#pragma pack` records and other types needing sema are
    /// `None`: callers defer those (`_Alignas(type)` carries the type to the
    /// layout builder, `_Static_assert` goes to sema). A guess here once laid
    /// out `struct { char a; _Alignas(uint16_t) char b; }` as 16 bytes.
    /// The optional `tag_aligns` map provides previously-computed alignments for
    /// struct/union tags, enabling correct alignment for tag-only references
    /// (e.g., `__alignof__(struct packed_tag)` where the definition is elsewhere).
    pub(super) fn alignof_type_spec(
        ts: &TypeSpecifier,
        tag_aligns: Option<&FxHashMap<String, usize>>,
    ) -> Option<usize> {
        use crate::common::types::target_ptr_size;
        let ptr_sz = target_ptr_size();
        let align = match ts {
            TypeSpecifier::Void
            | TypeSpecifier::Bool
            | TypeSpecifier::Char
            | TypeSpecifier::SignedChar
            | TypeSpecifier::UnsignedChar
            | TypeSpecifier::PlainUnsignedChar => 1,
            TypeSpecifier::Short | TypeSpecifier::UnsignedShort => 2,
            TypeSpecifier::Int
            | TypeSpecifier::UnsignedInt
            | TypeSpecifier::Signed
            | TypeSpecifier::Unsigned
            | TypeSpecifier::Float => 4,
            TypeSpecifier::Long | TypeSpecifier::UnsignedLong => ptr_sz,
            // On i686 (ILP32), long long and double are aligned to 4 bytes per i386 SysV ABI
            TypeSpecifier::LongLong | TypeSpecifier::UnsignedLongLong | TypeSpecifier::Double => {
                if ptr_sz == 4 {
                    4
                } else {
                    8
                }
            }
            TypeSpecifier::Pointer(_, _)
            | TypeSpecifier::FunctionPointer(..)
            | TypeSpecifier::NullPtrT => ptr_sz,
            TypeSpecifier::Int128 | TypeSpecifier::UnsignedInt128 => 16,
            // On i686, long double is 80-bit x87 but aligned to 4 bytes
            TypeSpecifier::LongDouble => {
                if ptr_sz == 4 {
                    4
                } else {
                    16
                }
            }
            TypeSpecifier::ComplexFloat => 4,
            TypeSpecifier::ComplexDouble => {
                if ptr_sz == 4 {
                    4
                } else {
                    8
                }
            }
            TypeSpecifier::ComplexLongDouble => {
                if ptr_sz == 4 {
                    4
                } else {
                    16
                }
            }
            TypeSpecifier::Array(elem, _) => return Self::alignof_type_spec(elem, tag_aligns),
            TypeSpecifier::Struct(name, fields, is_packed, pragma_pack, struct_aligned, _)
            | TypeSpecifier::Union(name, fields, is_packed, pragma_pack, struct_aligned, _) => {
                // Cache hit for previously DEFINED named records: definitions
                // insert their final alignment into struct_tag_alignments the
                // moment their fields are parsed, so lookups bottom-up during
                // deep `struct { struct { ... } }` nesting are O(1) each.
                // Without this, every level re-walked the entire chain and
                // nesting time grew quadratically (measured: depth 8000 went
                // from 11.6 s to milliseconds).
                if let Some(tag) = name {
                    if let Some(a) = tag_aligns.and_then(|m| m.get(tag)) {
                        return Some(*a);
                    }
                }
                if pragma_pack.is_some() {
                    return None;
                }
                if *is_packed {
                    return Some(1);
                }
                // Explicit struct/union-level __attribute__((aligned(N))) overrides
                let mut align = struct_aligned.unwrap_or(0);
                // Compute max alignment from fields if available
                // A tag-only reference was answered by the cache above.
                let field_list = fields.as_ref()?;
                for field in field_list {
                    // A member's own alignment raises its type's (C11 6.7.5);
                    // a type-form `_Alignas` the parser could not size makes
                    // the record's alignment unknown here.
                    if field.alignas_type.is_some() && field.alignment.is_none() {
                        return None;
                    }
                    // Packed members and unnamed bit-fields (which do not
                    // align the record on SysV) are left to the layout code.
                    if field.is_packed || (field.name.is_none() && field.bit_width.is_some()) {
                        return None;
                    }
                    let natural = if field.derived.is_empty() {
                        Self::alignof_type_spec(&field.type_spec, tag_aligns)?
                    } else {
                        return None;
                    };
                    align = align.max(natural).max(field.alignment.unwrap_or(0));
                }
                // An empty record aligns to 1 (GNU C).
                align.max(1)
            }
            // Enum width depends on the values, typedefs on their target and
            // everything else here (vectors, typeof, ...) on sema.
            _ => return None,
        };
        Some(align)
    }

    /// Compute preferred (natural) alignment for a type specifier.
    /// Used by GCC's __alignof/__alignof__ which returns preferred alignment.
    /// On i686: __alignof__(long long) == 8, __alignof__(double) == 8,
    /// while _Alignof returns 4 for both (minimum ABI alignment).
    pub(super) fn preferred_alignof_type_spec(
        ts: &TypeSpecifier,
        tag_aligns: Option<&FxHashMap<String, usize>>,
    ) -> Option<usize> {
        use crate::common::types::target_ptr_size;
        let ptr_sz = target_ptr_size();
        if ptr_sz != 4 {
            // On 64-bit targets, preferred == ABI alignment
            return Self::alignof_type_spec(ts, tag_aligns);
        }
        // On i686: long long and double have preferred alignment of 8
        match ts {
            TypeSpecifier::LongLong | TypeSpecifier::UnsignedLongLong | TypeSpecifier::Double => {
                Some(8)
            }
            TypeSpecifier::ComplexDouble => Some(8),
            TypeSpecifier::Array(elem, _) => Self::preferred_alignof_type_spec(elem, tag_aligns),
            _ => Self::alignof_type_spec(ts, tag_aligns),
        }
    }
}

#[cfg(test)]
mod named_space_target_tests {
    use super::*;
    use crate::common::types::{set_target_elf_machine, target_elf_machine};
    use crate::frontend::lexer::Lexer;

    #[test]
    fn named_space_target_contract() {
        struct RestoreTarget(u16);
        impl Drop for RestoreTarget {
            fn drop(&mut self) {
                set_target_elf_machine(self.0);
            }
        }
        let _restore = RestoreTarget(target_elf_machine());
        for machine in [3, 62, 183, 243] {
            set_target_elf_machine(machine);
            for qualifier in ["__seg_gs", "__seg_fs", ""] {
                for source in [
                    format!("extern long {qualifier} value;"),
                    format!("long f(long {qualifier} *p) {{ return *p; }}"),
                    format!("void f(long {qualifier} *p) {{ *p = 7; }}"),
                ] {
                    let mut parser = Parser::new(Lexer::new(&source, 0).tokenize());
                    parser.parse();
                    assert_eq!(
                        parser.error_count != 0,
                        !qualifier.is_empty() && !matches!(machine, 3 | 62),
                        "machine={machine}, source={source}"
                    );
                }
            }
        }
    }
}
