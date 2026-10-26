//! Shared type-system state for semantic analysis and IR lowering.
//!
//! `TypeContext` holds all type information that persists across function boundaries:
//! struct/union layouts, typedefs, enum constants, function typedef metadata, and
//! a type cache. It is populated by the sema pass and consumed/extended by the lowerer.
//!
//! This module lives in `frontend/sema/` because it is semantically part of the
//! sema output: sema creates, populates, and owns the TypeContext until it is
//! transferred to the lowerer. The lowerer imports from here, which is the correct
//! dependency direction (IR depends on frontend output, not the reverse).
//!
//! Scope management uses an undo-log pattern (`TypeScopeFrame`) rather than cloning
//! entire HashMaps at scope boundaries. This gives O(changes-in-scope) cost for
//! scope push/pop instead of O(total-map-size).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, CType, RcLayout, StructLayout};
use crate::frontend::parser::ast::{DerivedDeclarator, ParamDecl, TypeSpecifier};
use std::cell::RefCell;
use std::rc::Rc;

/// Information about a function typedef (e.g., `typedef int func_t(int, int);`).
/// Used to detect when a declaration like `func_t add;` is a function declaration
/// rather than a variable declaration.
///
/// Shared between sema (which collects typedef info) and lowering (which uses it
/// to resolve function declarations through typedefs).
#[derive(Debug, Clone)]
pub struct FunctionTypedefInfo {
    /// The return TypeSpecifier of the function typedef
    pub return_type: TypeSpecifier,
    /// Parameters of the function typedef
    pub params: Vec<ParamDecl>,
    /// Whether the function is variadic
    pub variadic: bool,
    /// Whether the function type has a prototype.
    pub has_prototype: bool,
}

/// Build the return `TypeSpecifier` encoded by `prefix` (the derived declarator
/// slice preceding a function or function-pointer's own syntax-marker/parameter
/// core), folding return-type `Pointer`, `Array`, and nested `(Pointer, FunctionPointer)`
/// layers in inside-out order.
pub fn build_prefix_return_type_spec(
    base_type: &TypeSpecifier,
    derived_full: &[DerivedDeclarator],
    prefix_len: usize,
) -> TypeSpecifier {
    let prefix = &derived_full[..prefix_len];
    let mut return_type = base_type.clone();
    let mut i = 0;
    while i < prefix.len() {
        match &prefix[i] {
            DerivedDeclarator::Pointer(_) => {
                if let Some(DerivedDeclarator::FunctionPointer(params, variadic, has_prototype)) =
                    prefix.get(i + 1)
                {
                    return_type = TypeSpecifier::FunctionPointer(
                        Box::new(return_type),
                        params.clone(),
                        *variadic,
                        *has_prototype,
                    );
                    i += 2;
                } else {
                    return_type = TypeSpecifier::Pointer(
                        Box::new(return_type),
                        crate::common::type_builder::pointee_space_before(derived_full, i),
                    );
                    i += 1;
                }
            }
            DerivedDeclarator::FunctionPointer(params, variadic, has_prototype) => {
                return_type = TypeSpecifier::FunctionPointer(
                    Box::new(return_type),
                    params.clone(),
                    *variadic,
                    *has_prototype,
                );
                i += 1;
            }
            DerivedDeclarator::Array(_) => {
                let start = i;
                while i < prefix.len() && matches!(&prefix[i], DerivedDeclarator::Array(_)) {
                    i += 1;
                }
                for j in (start..i).rev() {
                    if let DerivedDeclarator::Array(size_expr) = &prefix[j] {
                        return_type =
                            TypeSpecifier::Array(Box::new(return_type), size_expr.clone());
                    }
                }
            }
            DerivedDeclarator::Function(..) => {
                i += 1;
            }
        }
    }
    return_type
}

/// Extract function pointer typedef info from a declarator whose outermost
/// derived declarator is `FunctionPointer`.
///
/// Handles simple (`typedef void *(*lua_Alloc)(void *, ...)`), nested
/// (`typedef int (*(*(*fp3_t)(int))(long))(char)`), and pointer-to-array-returning
/// (`typedef int (*(*fp_arr_t)(int))[4]`) function-pointer typedefs, while
/// excluding function typedefs (`typedef void (*fn_t(int))(void)`), arrays of
/// function pointers (`typedef int (*fps_t[4])(int)`), and pointers to function
/// pointers (`typedef int (**fpp_t)(int)`).
pub fn extract_fptr_typedef_info(
    base_type: &TypeSpecifier,
    derived: &[DerivedDeclarator],
) -> Option<FunctionTypedefInfo> {
    let Some(DerivedDeclarator::FunctionPointer(params, variadic, has_prototype)) = derived.last()
    else {
        return None;
    };
    // Strip the entity's own `FunctionPointer` and its preceding syntax-marker `Pointer`.
    let prefix_len = if derived.len() >= 2
        && matches!(derived[derived.len() - 2], DerivedDeclarator::Pointer(_))
    {
        derived.len() - 2
    } else {
        derived.len() - 1
    };
    let return_type = build_prefix_return_type_spec(base_type, derived, prefix_len);
    Some(FunctionTypedefInfo {
        return_type,
        params: params.clone(),
        variadic: *variadic,
        has_prototype: *has_prototype,
    })
}

/// Records undo operations for type-system scoped state.
///
/// Pushed on scope entry, popped on scope exit. Tracks both newly-added keys
/// (removed on undo) and shadowed keys (restored to previous value on undo)
/// across enum_constants, struct_layouts, ctype_cache, and typedefs.
/// An enumeration constant: its value and its C type.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumConstant {
    pub value: i64,
    pub ty: CType,
}

impl EnumConstant {
    /// The constant as seen inside its own enum body, before the enum is
    /// complete: `int` when the value fits, else the smallest of `unsigned
    /// int`, `long long` or `unsigned long long` holding it.
    pub fn provisional(value: i64) -> Self {
        let ty = if i32::try_from(value).is_ok() {
            CType::Int
        } else if u32::try_from(value).is_ok() {
            CType::UInt
        } else if value >= 0 {
            CType::ULongLong
        } else {
            CType::LongLong
        };
        Self { value, ty }
    }
}

#[derive(Debug)]
pub struct TypeScopeFrame {
    /// Every write to `enum_constants` in this scope, with the value it
    /// replaced (`None` when the name did not exist before the scope).  Undone
    /// last-write-first, so a name written twice inside one scope is restored
    /// correctly without scanning the log.
    pub enums_changed: Vec<(String, Option<EnumConstant>)>,
    /// Enum tag definitions introduced or shadowed in this scope.  Tag
    /// identity is lexical: an inner `enum E` must not overwrite the completed
    /// outer `enum E` after this frame is popped.
    pub enum_types_added: Vec<String>,
    pub enum_types_shadowed: Vec<(String, crate::common::types::EnumType)>,
    pub enum_types_tracked: FxHashSet<String>,
    /// Keys newly inserted into `struct_layouts`.
    pub struct_layouts_added: Vec<String>,
    /// Keys that were overwritten in `struct_layouts`: (key, previous_value).
    /// Uses Rc<StructLayout> so saving/restoring is a cheap refcount bump.
    pub struct_layouts_shadowed: Vec<(String, RcLayout)>,
    /// Base record keys whose alias was newly inserted into `record_alias`.
    pub record_alias_added: Vec<String>,
    /// Base record keys whose alias was overwritten: (base, previous alias).
    pub record_alias_shadowed: Vec<(String, String)>,
    /// Keys newly inserted into `ctype_cache`.
    pub ctype_cache_added: Vec<String>,
    /// Keys that were overwritten in `ctype_cache`: (key, previous_value).
    pub ctype_cache_shadowed: Vec<(String, CType)>,
    /// Keys newly inserted into `typedefs`.
    pub typedefs_added: Vec<String>,
    /// Keys that were overwritten in `typedefs`: (key, previous_value).
    pub typedefs_shadowed: Vec<(String, CType)>,
    /// Keys newly inserted into `typedef_alignments`.
    pub typedef_alignments_added: Vec<String>,
    /// Keys that were overwritten in `typedef_alignments`: (key, previous_value).
    pub typedef_alignments_shadowed: Vec<(String, usize)>,
    /// Every change to `qualified_pointee_typedefs` in this scope, with the
    /// membership it replaced; undone last-write-first.
    pub qualified_pointee_changed: Vec<(String, bool)>,
    /// Same, for `const_base_typedefs`.
    pub const_base_changed: Vec<(String, bool)>,
}

impl TypeScopeFrame {
    fn new() -> Self {
        Self {
            enums_changed: Vec::new(),
            enum_types_added: Vec::new(),
            enum_types_shadowed: Vec::new(),
            enum_types_tracked: FxHashSet::default(),
            struct_layouts_added: Vec::new(),
            struct_layouts_shadowed: Vec::new(),
            record_alias_added: Vec::new(),
            record_alias_shadowed: Vec::new(),
            ctype_cache_added: Vec::new(),
            ctype_cache_shadowed: Vec::new(),
            typedefs_added: Vec::new(),
            typedefs_shadowed: Vec::new(),
            typedef_alignments_added: Vec::new(),
            typedef_alignments_shadowed: Vec::new(),
            qualified_pointee_changed: Vec::new(),
            const_base_changed: Vec::new(),
        }
    }
}

/// Module-level type-system state produced by sema and consumed by lowering.
///
/// Holds struct/union layouts, typedefs, enum constants, and type caches.
/// Populated by the sema pass during semantic analysis, then transferred
/// to the lowerer by ownership for IR emission.
#[derive(Debug)]
pub struct TypeContext {
    /// Struct/union layouts indexed by tag name.
    /// Uses Rc<StructLayout> so lookups/clones are cheap refcount bumps
    /// instead of deep-copying all field names and types.
    /// Wrapped in RefCell for interior mutability: type resolution methods
    /// that take &self (via the TypeConvertContext trait) may need to insert
    /// forward-declaration layouts when encountering struct/union types.
    pub struct_layouts: RefCell<FxHashMap<String, RcLayout>>,
    /// Base record key -> the key currently denoting that tag.
    ///
    /// Only non-identity while an inner-scope definition shadows an outer one
    /// (C 6.7.2.3): `struct.S` then denotes `struct.S#1` until the scope pops.
    /// Scoped like `struct_layouts` so shadowing unwinds with the scope.
    pub record_alias: RefCell<FxHashMap<String, String>>,
    /// Enum constants: value and C type, keyed by enumerator name.
    pub enum_constants: FxHashMap<String, EnumConstant>,
    /// Typedef mappings (name -> resolved CType)
    pub typedefs: FxHashMap<String, CType>,
    /// Per-typedef alignment overrides from `__attribute__((aligned(N)))` on typedef
    /// declarations.  E.g. `typedef struct S aligned_S __attribute__((aligned(32)));`
    /// stores `"aligned_S" -> 32`.  Consulted when computing field / variable alignment
    /// for declarations that use the typedef name.
    pub typedef_alignments: FxHashMap<String, usize>,
    /// Pointer typedefs whose pointee is `const`/`volatile` (`typedef const
    /// void *CNPT;`). `CType` carries no qualifiers, so the name is recorded
    /// here for the null pointer constant rule: `(CNPT)0` is not one. Scoped
    /// like `typedefs` (see [`Self::set_typedef_pointee_qualified`]).
    pub qualified_pointee_typedefs: FxHashSet<String>,
    /// Typedefs whose base specifier is `const` (`typedef const int CI;`,
    /// `typedef const char *CS;`; see `common::qualifiers`). Scoped like
    /// `typedefs` (see [`Self::set_typedef_const_base`]).
    pub const_base_typedefs: FxHashSet<String>,
    /// Function typedef info (bare function typedefs like `typedef int func_t(int)`)
    pub function_typedefs: FxHashMap<String, FunctionTypedefInfo>,
    /// Set of typedef names that are function pointer types
    /// (e.g., `typedef void *(*lua_Alloc)(void *, ...)`)
    pub func_ptr_typedefs: FxHashSet<String>,
    /// Function pointer typedef info (return type, params, variadic)
    pub func_ptr_typedef_info: FxHashMap<String, FunctionTypedefInfo>,
    /// Completed named enum definitions, keyed by tag name. Each scope frame
    /// records additions and shadowed values so body-less references resolve
    /// to the definition visible at that exact lexical point.
    pub enum_types: FxHashMap<String, crate::common::types::EnumType>,
    /// Return CType for known functions
    pub func_return_ctypes: FxHashMap<String, CType>,
    /// Cache for CType of named struct/union types.
    /// Uses RefCell because type_spec_to_ctype takes &self.
    pub ctype_cache: RefCell<FxHashMap<String, CType>>,
    /// Scope stack for type-system undo tracking (enum_constants, struct_layouts, ctype_cache).
    /// Wrapped in RefCell for interior mutability: scoped insertion methods
    /// called from &self contexts need to record undo entries.
    pub scope_stack: RefCell<Vec<TypeScopeFrame>>,
    /// Counter for anonymous struct/union CType keys generated from &self contexts.
    /// Uses Cell for interior mutability since type_spec_to_ctype takes &self.
    anon_ctype_counter: std::cell::Cell<u32>,
    /// Counter for shadowing record definitions; see `next_struct_variant_id`.
    struct_variant_counter: std::cell::Cell<u64>,
    /// Folded scalar and member values of C23 `(constexpr T){...}` compound literals,
    /// keyed by `(ExprId::of(cl_expr), path)` (`""` for scalar, `".a.b"` for member).
    pub constexpr_compound_literals: FxHashMap<
        (crate::frontend::parser::ast::ExprId, String),
        crate::common::symbol_table::FoldedValue,
    >,
}

// We cannot directly implement `StructLayoutProvider` for `TypeContext` because
// `struct_layouts` is behind a `RefCell`, and the trait returns `Option<&StructLayout>`
// which would borrow from a temporary `Ref` guard. Instead, callers should use
// `tc.borrow_struct_layouts()` to get the guard, then pass `&*guard` as the
// `&dyn StructLayoutProvider` (since `FxHashMap<String, RcLayout>` implements the trait).

impl TypeContext {
    pub fn new() -> Self {
        let mut tc = Self {
            struct_layouts: RefCell::new(FxHashMap::default()),
            record_alias: RefCell::new(FxHashMap::default()),
            enum_constants: FxHashMap::default(),
            typedefs: FxHashMap::default(),
            typedef_alignments: FxHashMap::default(),
            qualified_pointee_typedefs: FxHashSet::default(),
            const_base_typedefs: FxHashSet::default(),
            function_typedefs: FxHashMap::default(),
            func_ptr_typedefs: FxHashSet::default(),
            func_ptr_typedef_info: FxHashMap::default(),
            enum_types: FxHashMap::default(),
            func_return_ctypes: FxHashMap::default(),
            ctype_cache: RefCell::new(FxHashMap::default()),
            // Frames are lexical undo logs, not scope identities. File scope
            // deliberately has no disposable frame; its canonical depth is 0,
            // and frame index N corresponds to lexical depth N + 1.
            scope_stack: RefCell::new(Vec::new()),
            anon_ctype_counter: std::cell::Cell::new(0),
            struct_variant_counter: std::cell::Cell::new(0),
            constexpr_compound_literals: FxHashMap::default(),
        };
        tc.seed_builtin_typedefs();
        tc
    }

    /// Pre-populate typedef mappings for builtin/standard C types so that
    /// sema can correctly resolve types like `uint64_t`, `size_t`, etc.
    /// even when the source code does not `#include <stdint.h>`.
    ///
    /// Without this, sema's `resolve_typedef` falls back to `CType::Int`
    /// for unresolved typedefs, causing functions returning `uint64_t`
    /// to be recorded with a 32-bit return type.
    fn seed_builtin_typedefs(&mut self) {
        use crate::common::types::target_is_32bit;
        let is_32bit = target_is_32bit();

        // On ILP32 (i686): long=32bit, so 64-bit types must use LongLong
        // On LP64 (x86-64, arm64, riscv64): long=64bit
        let i64_type = if is_32bit {
            CType::LongLong
        } else {
            CType::Long
        };
        let u64_type = if is_32bit {
            CType::ULongLong
        } else {
            CType::ULong
        };
        let size_type = if is_32bit { CType::UInt } else { CType::ULong };
        let ssize_type = if is_32bit { CType::Int } else { CType::Long };
        let ptrdiff_type = if is_32bit { CType::Int } else { CType::Long };
        let intptr_type = if is_32bit { CType::Int } else { CType::Long };
        let uintptr_type = if is_32bit { CType::UInt } else { CType::ULong };
        let fast_s = if is_32bit { CType::Int } else { CType::Long };
        let fast_u = if is_32bit { CType::UInt } else { CType::ULong };
        let long_s = CType::Long;
        let long_u = CType::ULong;

        let builtins: &[(&str, CType)] = &[
            // <stddef.h>
            ("size_t", size_type.clone()),
            (
                crate::common::types::OFFSETOF_SIZE_TYPEDEF,
                size_type.clone(),
            ),
            ("ssize_t", ssize_type.clone()),
            ("ptrdiff_t", ptrdiff_type),
            ("wchar_t", crate::common::types::wchar_ctype()),
            ("wint_t", CType::UInt),
            // <stdint.h> - exact width types
            ("int8_t", CType::signed_char()),
            ("int16_t", CType::Short),
            ("int32_t", CType::Int),
            ("int64_t", i64_type.clone()),
            ("uint8_t", CType::UChar),
            ("uint16_t", CType::UShort),
            ("uint32_t", CType::UInt),
            ("uint64_t", u64_type.clone()),
            ("intptr_t", intptr_type),
            ("uintptr_t", uintptr_type),
            ("intmax_t", i64_type.clone()),
            ("uintmax_t", u64_type.clone()),
            // least types
            ("int_least8_t", CType::signed_char()),
            ("int_least16_t", CType::Short),
            ("int_least32_t", CType::Int),
            ("int_least64_t", i64_type.clone()),
            ("uint_least8_t", CType::UChar),
            ("uint_least16_t", CType::UShort),
            ("uint_least32_t", CType::UInt),
            ("uint_least64_t", u64_type.clone()),
            // fast types
            ("int_fast8_t", CType::signed_char()),
            ("int_fast16_t", fast_s.clone()),
            ("int_fast32_t", fast_s.clone()),
            ("int_fast64_t", i64_type.clone()),
            ("uint_fast8_t", CType::UChar),
            ("uint_fast16_t", fast_u.clone()),
            ("uint_fast32_t", fast_u),
            ("uint_fast64_t", u64_type.clone()),
            // <signal.h>
            ("sig_atomic_t", CType::Int),
            // <time.h>
            ("time_t", long_s.clone()),
            ("clock_t", long_s.clone()),
            (
                "timer_t",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            ("clockid_t", CType::Int),
            // <sys/types.h>
            ("off_t", long_s.clone()),
            ("pid_t", CType::Int),
            ("uid_t", CType::UInt),
            ("gid_t", CType::UInt),
            ("mode_t", CType::UInt),
            ("dev_t", u64_type.clone()),
            ("ino_t", u64_type.clone()),
            ("nlink_t", u64_type.clone()),
            ("blksize_t", long_s.clone()),
            ("blkcnt_t", long_s),
            // GNU/glibc common
            ("ulong", long_u),
            ("ushort", CType::UShort),
            ("uint", CType::UInt),
            ("__u8", CType::UChar),
            ("__u16", CType::UShort),
            ("__u32", CType::UInt),
            ("__u64", u64_type.clone()),
            ("__s8", CType::signed_char()),
            ("__s16", CType::Short),
            ("__s32", CType::Int),
            ("__s64", i64_type.clone()),
            // <locale.h>
            (
                "locale_t",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            // <pthread.h>
            ("pthread_t", size_type),
            (
                "pthread_mutex_t",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            (
                "pthread_cond_t",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            ("pthread_key_t", CType::UInt),
            (
                "pthread_attr_t",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            ("pthread_once_t", CType::Int),
            (
                "pthread_mutexattr_t",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            (
                "pthread_condattr_t",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            // <setjmp.h>
            (
                "jmp_buf",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            (
                "sigjmp_buf",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            // <stdio.h>
            (
                "FILE",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            ("fpos_t", ssize_type),
            // <dirent.h>
            (
                "DIR",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            // POSIX internal names
            ("__u_char", CType::UChar),
            ("__u_short", CType::UShort),
            ("__u_int", CType::UInt),
            ("__u_long", CType::ULong),
            ("__int8_t", CType::signed_char()),
            ("__int16_t", CType::Short),
            ("__int32_t", CType::Int),
            ("__uint8_t", CType::UChar),
            ("__uint16_t", CType::UShort),
            ("__uint32_t", CType::UInt),
            // __int64_t/__uint64_t: must be LongLong on ILP32, Long on LP64
            ("__int64_t", i64_type),
            ("__uint64_t", u64_type),
            // <stdarg.h> - va_list and related types.
            // Sema uses a pointer approximation; the IR lowerer (types_seed.rs)
            // applies the target-specific concrete ABI type.
            (
                "va_list",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            (
                "__builtin_va_list",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
            (
                "__gnuc_va_list",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
            ),
        ];
        for (name, ct) in builtins {
            self.typedefs.insert(name.to_string(), ct.clone());
        }
    }

    /// Borrow the struct layouts map immutably.
    /// Returns a `Ref` guard that derefs to `FxHashMap<String, RcLayout>`.
    /// The underlying `FxHashMap` implements `StructLayoutProvider`, so
    /// `&*guard` can be passed wherever `&dyn StructLayoutProvider` is needed.
    pub fn borrow_struct_layouts(&self) -> std::cell::Ref<'_, FxHashMap<String, RcLayout>> {
        self.struct_layouts.borrow()
    }

    /// Borrow the struct layouts map mutably.
    /// Returns a `RefMut` guard that derefs to `FxHashMap<String, RcLayout>`.
    pub fn borrow_struct_layouts_mut(&self) -> std::cell::RefMut<'_, FxHashMap<String, RcLayout>> {
        self.struct_layouts.borrow_mut()
    }

    /// Get the next anonymous struct/union ID for CType key generation.
    /// Safe to call from &self contexts (uses Cell for interior mutability).
    pub fn next_anon_struct_id(&self) -> u32 {
        let id = self.anon_ctype_counter.get();
        self.anon_ctype_counter.set(id + 1);
        id
    }

    /// Next id for a *shadowing* record definition: a `struct S { ... }` inside
    /// an inner scope when a different `struct S` is already visible.
    ///
    /// C 6.7.2.3 makes that a NEW type, incompatible with the outer one, so it
    /// cannot share the outer one's type key. The id is used as a `#N` suffix
    /// on the base key (`struct.S#1`); `CType::record_display_tag` strips it for
    /// diagnostics so users still see `struct S`.
    /// Next record-variant suffix. Variant keys must be unique for the whole
    /// translation unit, so this is a plain counter rather than anything keyed
    /// off the source: two shadowing definitions at the same nesting depth in
    /// different branches would collide on a depth- or scope-derived scheme.
    /// `u64` because uniqueness is the invariant the type system rests on and
    /// it is not worth reasoning about a wrap; 1.8e19 increments would need
    /// more record definitions than fit in addressable memory, and in a debug
    /// build the `+ 1` panics rather than silently reusing a key.
    pub fn next_struct_variant_id(&self) -> u64 {
        let id = self.struct_variant_counter.get();
        self.struct_variant_counter.set(id + 1);
        id
    }

    /// Record that `base` is currently denoted by `active`, scoped so the
    /// mapping disappears when the shadowing scope pops.
    pub fn set_record_alias_from_ref(&self, base: &str, active: &str) {
        let mut alias = self.record_alias.borrow_mut();
        let mut stack = self.scope_stack.borrow_mut();
        if let Some(frame) = stack.last_mut() {
            // Record the state as it was BEFORE this frame touched it, at most
            // once per base key. Two definitions of one tag in the same frame
            // (a redefinition -- REDEF-1, which lccc does not yet diagnose but
            // GCC does) would otherwise push two undo entries, and `pop_scope`
            // removes the `added` entries first and then replays the `shadowed`
            // ones, so the second entry would resurrect an alias belonging to a
            // scope that no longer exists. One entry per key makes the undo
            // idempotent instead of order-dependent.
            let tracked = frame.record_alias_added.iter().any(|k| k == base)
                || frame.record_alias_shadowed.iter().any(|(k, _)| k == base);
            if !tracked {
                match alias.get(base).cloned() {
                    Some(prev) => frame.record_alias_shadowed.push((base.to_string(), prev)),
                    None => frame.record_alias_added.push(base.to_string()),
                }
            }
        }
        alias.insert(base.to_string(), active.to_string());
    }

    /// The key that currently denotes record tag key `base`: the shadowing
    /// variant while one is live, otherwise `base` itself.
    ///
    /// Every place that turns a source-level tag into a record `CType` must go
    /// through this, or an inner-scope `struct S` value gets typed as the outer
    /// `struct S` and the two become indistinguishable to the type checker.
    ///
    /// Returns a borrow, not an owned copy: the overwhelmingly common case is
    /// "no shadowing in effect", and record type resolution happens for every
    /// elaborated type in the translation unit, so cloning here would be a
    /// pure waste.
    pub fn resolve_record_key<'a>(&self, base: &'a str) -> std::borrow::Cow<'a, str> {
        // Copy out under the guard: a Cow::Borrowed tied to the Ref guard would
        // not outlive it. The common path still allocates nothing.
        let active = self.record_alias.borrow().get(base).cloned();
        match active {
            Some(active) => std::borrow::Cow::Owned(active),
            None => std::borrow::Cow::Borrowed(base),
        }
    }

    /// Insert a struct layout from a &self context (interior mutability via RefCell).
    pub fn insert_struct_layout_from_ref(&self, key: &str, layout: StructLayout) {
        self.struct_layouts
            .borrow_mut()
            .insert(key.to_string(), Rc::new(layout));
    }

    /// Check if a struct key is currently shadowed by an inner scope redefinition.
    /// Returns true if any scope frame has saved a previous layout for this key,
    /// meaning the current layout in the map differs from an outer scope's definition.
    pub fn is_struct_key_shadowed(&self, key: &str) -> bool {
        let stack = self.scope_stack.borrow();
        for frame in stack.iter() {
            for (k, _) in &frame.struct_layouts_shadowed {
                if k == key {
                    return true;
                }
            }
        }
        false
    }

    /// Push a new type-system scope frame.
    pub fn push_scope(&mut self) {
        self.scope_stack.get_mut().push(TypeScopeFrame::new());
    }

    /// Pop the top type-system scope frame and undo changes to
    /// enum_constants, struct_layouts, ctype_cache, and typedefs.
    pub fn pop_scope(&mut self) {
        if let Some(frame) = self.scope_stack.get_mut().pop() {
            // Last-write-first: each entry holds the value that write
            // replaced, so restoring in reverse rebuilds the pre-scope state
            // exactly.  `None` marks a name this scope introduced.
            for (key, was_const) in frame.const_base_changed.into_iter().rev() {
                if was_const {
                    self.const_base_typedefs.insert(key);
                } else {
                    self.const_base_typedefs.remove(&key);
                }
            }
            for (key, was_qualified) in frame.qualified_pointee_changed.into_iter().rev() {
                if was_qualified {
                    self.qualified_pointee_typedefs.insert(key);
                } else {
                    self.qualified_pointee_typedefs.remove(&key);
                }
            }
            for (key, previous) in frame.enums_changed.into_iter().rev() {
                match previous {
                    Some(value) => {
                        self.enum_constants.insert(key, value);
                    }
                    None => {
                        self.enum_constants.remove(&key);
                    }
                }
            }
            for key in frame.enum_types_added {
                self.enum_types.remove(&key);
            }
            for (key, value) in frame.enum_types_shadowed {
                self.enum_types.insert(key, value);
            }
            let layouts = self.struct_layouts.get_mut();
            for key in frame.struct_layouts_added {
                layouts.remove(&key);
            }
            {
                let mut alias = self.record_alias.borrow_mut();
                for key in frame.record_alias_added {
                    alias.remove(&key);
                }
                for (key, val) in frame.record_alias_shadowed {
                    alias.insert(key, val);
                }
            }
            for (key, val) in frame.struct_layouts_shadowed {
                // Don't restore an empty forward-declaration layout over a full
                // definition.
                if val.fields.is_empty() {
                    if let Some(current) = layouts.get(&key) {
                        if !current.fields.is_empty() {
                            continue;
                        }
                    }
                }
                layouts.insert(key, val);
            }
            {
                let cache = self.ctype_cache.get_mut();
                for key in frame.ctype_cache_added {
                    cache.remove(&key);
                }
                for (key, val) in frame.ctype_cache_shadowed {
                    cache.insert(key, val);
                }
            }
            for key in frame.typedefs_added {
                self.typedefs.remove(&key);
            }
            for (key, val) in frame.typedefs_shadowed {
                self.typedefs.insert(key, val);
            }
            for key in frame.typedef_alignments_added {
                self.typedef_alignments.remove(&key);
            }
            for (key, val) in frame.typedef_alignments_shadowed {
                self.typedef_alignments.insert(key, val);
            }
        }
    }

    /// Return the visible enumerator and its lexical depth (file scope is 0).
    ///
    /// This pairs with `SymbolTable::lookup_with_depth`: enumerators and
    /// objects occupy C's same ordinary-identifier namespace even though their
    /// compiler payloads are intentionally stored separately.
    pub fn enum_constant_with_depth(&self, name: &str) -> Option<(&EnumConstant, usize)> {
        let constant = self.enum_constants.get(name)?;
        let depth = self
            .scope_stack
            .borrow()
            .iter()
            .enumerate()
            .rev()
            .find(|(_, frame)| {
                frame
                    .enums_changed
                    .iter()
                    .any(|(changed, _)| changed == name)
            })
            .map_or(0, |(index, _)| index + 1);
        Some((constant, depth))
    }

    /// Insert an enum constant, tracking the change in the current scope frame.
    ///
    /// The previous value is recorded too, so a block-scope enum that redefines
    /// an outer enumerator is undone at scope exit:
    ///
    /// ```c
    /// enum { A = 1 };
    /// void f(void) { enum { A = 2 }; }
    /// enum { B = A };   /* A is 1 here, not 2 */
    /// ```
    ///
    /// Restoring only *new* names let the inner value leak out of the block, so
    /// every later use of the outer `A` -- in sema and in the lowerer, which
    /// share this map -- silently became 2.  GCC 16.2 prints `1 1` for the
    /// program above; lccc printed `2 2`.
    pub fn insert_enum_scoped(&mut self, name: String, constant: EnumConstant) {
        let previous = self.enum_constants.get(&name).cloned();
        if let Some(frame) = self.scope_stack.get_mut().last_mut() {
            frame.enums_changed.push((name.clone(), previous));
        }
        self.enum_constants.insert(name, constant);
    }

    /// Give every enumerator of a just-completed enum its final type
    /// ([`EnumType::constant_ctype`]); inside the enum body they carried
    /// [`EnumConstant::provisional`] types. Returns that type.
    ///
    /// [`EnumType::constant_ctype`]: crate::common::types::EnumType::constant_ctype
    pub fn complete_enum_constants(
        &mut self,
        enum_type: &crate::common::types::EnumType,
        scoped: bool,
    ) -> CType {
        let ty = enum_type.constant_ctype();
        for (name, value) in &enum_type.variants {
            let constant = EnumConstant {
                value: *value,
                ty: ty.clone(),
            };
            if scoped {
                self.insert_enum_scoped(name.clone(), constant);
            } else {
                self.enum_constants.insert(name.clone(), constant);
            }
        }
        ty
    }

    /// The enum tag `name` declared in the innermost scope, if any (C11
    /// 6.7.2.3: only a declaration in the same scope redeclares the tag).
    pub fn enum_type_in_current_scope(
        &self,
        name: &str,
    ) -> Option<&crate::common::types::EnumType> {
        let in_scope = match self.scope_stack.borrow().last() {
            Some(frame) => frame.enum_types_tracked.contains(name),
            // File scope: no frame.
            None => true,
        };
        if in_scope {
            self.enum_types.get(name)
        } else {
            None
        }
    }

    /// Insert a completed enum tag definition, tracking lexical shadowing so
    /// body-less references after scope exit recover the outer definition.
    pub fn insert_enum_type_scoped(
        &mut self,
        name: String,
        enum_type: crate::common::types::EnumType,
    ) {
        if let Some(frame) = self.scope_stack.get_mut().last_mut() {
            if frame.enum_types_tracked.insert(name.clone()) {
                if let Some(previous) = self.enum_types.get(&name).cloned() {
                    frame.enum_types_shadowed.push((name.clone(), previous));
                } else {
                    frame.enum_types_added.push(name.clone());
                }
            }
        }
        self.enum_types.insert(name, enum_type);
    }

    /// Insert a struct layout, tracking the change in the current scope frame
    /// so it can be undone on scope exit.
    pub fn insert_struct_layout_scoped(&mut self, key: String, layout: StructLayout) {
        let layouts = self.struct_layouts.get_mut();
        if let Some(frame) = self.scope_stack.get_mut().last_mut() {
            if let Some(prev) = layouts.get(&key).cloned() {
                frame.struct_layouts_shadowed.push((key.clone(), prev));
            } else {
                frame.struct_layouts_added.push(key.clone());
            }
        }
        layouts.insert(key, Rc::new(layout));
    }

    /// Insert a typedef, tracking the change in the current scope frame
    /// so it can be undone on scope exit.
    pub fn insert_typedef_scoped(&mut self, name: String, ctype: CType) {
        if let Some(frame) = self.scope_stack.get_mut().last_mut() {
            if let Some(prev) = self.typedefs.get(&name).cloned() {
                frame.typedefs_shadowed.push((name.clone(), prev));
            } else {
                frame.typedefs_added.push(name.clone());
            }
        }
        self.typedefs.insert(name, ctype);
    }

    /// Record whether typedef `name` is a pointer to a qualified type, undone
    /// when the current scope is popped: a block-scope `typedef void *P;`
    /// must not erase the mark of a file-scope `typedef const void *P;` for
    /// the rest of the translation unit.
    pub fn set_typedef_pointee_qualified(&mut self, name: &str, qualified: bool) {
        let was_qualified = self.qualified_pointee_typedefs.contains(name);
        if was_qualified == qualified {
            return;
        }
        if let Some(frame) = self.scope_stack.get_mut().last_mut() {
            frame
                .qualified_pointee_changed
                .push((name.to_string(), was_qualified));
        }
        if qualified {
            self.qualified_pointee_typedefs.insert(name.to_string());
        } else {
            self.qualified_pointee_typedefs.remove(name);
        }
    }

    /// Record whether typedef `name` has a `const` base specifier, undone when
    /// the current scope is popped (see `set_typedef_pointee_qualified`).
    pub fn set_typedef_const_base(&mut self, name: &str, is_const: bool) {
        let was_const = self.const_base_typedefs.contains(name);
        if was_const == is_const {
            return;
        }
        if let Some(frame) = self.scope_stack.get_mut().last_mut() {
            frame.const_base_changed.push((name.to_string(), was_const));
        }
        if is_const {
            self.const_base_typedefs.insert(name.to_string());
        } else {
            self.const_base_typedefs.remove(name);
        }
    }

    /// Insert a typedef alignment, tracking the change in the current scope frame
    /// so it can be undone on scope exit.
    pub fn insert_typedef_alignment_scoped(&mut self, name: String, align: usize) {
        if let Some(frame) = self.scope_stack.get_mut().last_mut() {
            if let Some(prev) = self.typedef_alignments.get(&name).copied() {
                frame.typedef_alignments_shadowed.push((name.clone(), prev));
            } else {
                frame.typedef_alignments_added.push(name.clone());
            }
        }
        self.typedef_alignments.insert(name, align);
    }

    /// Invalidate a ctype_cache entry, tracking the change in the current scope frame
    /// so it can be restored on scope exit.
    pub fn invalidate_ctype_cache_scoped(&mut self, key: &str) {
        let prev = self.ctype_cache.get_mut().remove(key);
        if let Some(frame) = self.scope_stack.get_mut().last_mut() {
            if let Some(prev) = prev {
                frame.ctype_cache_shadowed.push((key.to_string(), prev));
            } else {
                frame.ctype_cache_added.push(key.to_string());
            }
        }
    }

    /// Insert a struct layout from a &self context (interior mutability via RefCell),
    /// tracking the change in the current scope frame so it can be undone on scope exit.
    ///
    /// Used by `type_spec_to_ctype` which takes &self but still needs to
    /// properly scope struct layout insertions within function bodies.
    pub fn insert_struct_layout_scoped_from_ref(&self, key: &str, layout: StructLayout) {
        let mut layouts = self.struct_layouts.borrow_mut();
        let mut stack = self.scope_stack.borrow_mut();
        if let Some(frame) = stack.last_mut() {
            if let Some(prev) = layouts.get(key).cloned() {
                frame.struct_layouts_shadowed.push((key.to_string(), prev));
            } else {
                frame.struct_layouts_added.push(key.to_string());
            }
        }
        layouts.insert(key.to_string(), Rc::new(layout));
    }

    /// Invalidate a ctype_cache entry from a &self context, tracking the change
    /// in the current scope frame so it can be restored on scope exit.
    pub fn invalidate_ctype_cache_scoped_from_ref(&self, key: &str) {
        let prev = self.ctype_cache.borrow_mut().remove(key);
        let mut stack = self.scope_stack.borrow_mut();
        if let Some(frame) = stack.last_mut() {
            if let Some(prev) = prev {
                frame.ctype_cache_shadowed.push((key.to_string(), prev));
            } else {
                frame.ctype_cache_added.push(key.to_string());
            }
        }
    }
}

#[cfg(test)]
mod record_alias_tests {
    use super::TypeContext;
    use std::borrow::Cow;

    #[test]
    fn absent_tag_borrows_rather_than_allocates() {
        // The common path: no alias means no String, which is the whole point
        // of returning a Cow here.
        let ctx = TypeContext::new();
        match ctx.resolve_record_key("struct.S") {
            Cow::Borrowed(b) => assert_eq!(b, "struct.S"),
            Cow::Owned(_) => panic!("expected a borrow on the no-alias path"),
        }
    }

    #[test]
    fn alias_is_visible_while_its_scope_lives() {
        let mut ctx = TypeContext::new();
        ctx.push_scope();
        ctx.set_record_alias_from_ref("struct.S", "struct.S#1");
        assert_eq!(ctx.resolve_record_key("struct.S").as_ref(), "struct.S#1");
    }

    #[test]
    fn pop_scope_undoes_the_alias() {
        let mut ctx = TypeContext::new();
        ctx.push_scope();
        ctx.set_record_alias_from_ref("struct.S", "struct.S#1");
        ctx.pop_scope();
        match ctx.resolve_record_key("struct.S") {
            Cow::Borrowed(b) => assert_eq!(b, "struct.S"),
            Cow::Owned(o) => panic!("alias outlived its scope: {o}"),
        }
    }

    #[test]
    fn nested_shadow_restores_the_outer_variant_not_the_base() {
        // Two nesting levels: leaving the inner scope must restore the OUTER
        // variant, not the bare base key, or the middle scope silently loses
        // its own shadowing.
        let mut ctx = TypeContext::new();
        ctx.push_scope();
        ctx.set_record_alias_from_ref("struct.S", "struct.S#1");
        ctx.push_scope();
        ctx.set_record_alias_from_ref("struct.S", "struct.S#2");
        assert_eq!(ctx.resolve_record_key("struct.S").as_ref(), "struct.S#2");
        ctx.pop_scope();
        assert_eq!(ctx.resolve_record_key("struct.S").as_ref(), "struct.S#1");
        ctx.pop_scope();
        assert_eq!(ctx.resolve_record_key("struct.S").as_ref(), "struct.S");
    }

    #[test]
    fn two_definitions_in_one_frame_do_not_leak_an_alias_past_the_pop() {
        // REDEF-1's structural half. A redefinition in the SAME frame used to
        // push an `added` entry and then a `shadowed` one; pop_scope removes
        // `added` first and then replays `shadowed`, so the alias came back
        // pointing at a variant from a scope that no longer exists -- a
        // dangling alias that would make every later `struct S` in the file
        // resolve to the dead inner record.
        let mut ctx = TypeContext::new();
        ctx.push_scope();
        ctx.set_record_alias_from_ref("struct.S", "struct.S#1");
        ctx.set_record_alias_from_ref("struct.S", "struct.S#2");
        assert_eq!(ctx.resolve_record_key("struct.S").as_ref(), "struct.S#2");
        ctx.pop_scope();
        assert_eq!(
            ctx.resolve_record_key("struct.S").as_ref(),
            "struct.S",
            "the alias must be gone, not resurrected"
        );
    }

    #[test]
    fn redefinition_over_a_shadowed_outer_alias_restores_the_outer_one() {
        // Same double-set, but with an outer alias to restore: the frame must
        // remember the value that was live BEFORE it started, not the
        // intermediate one.
        let mut ctx = TypeContext::new();
        ctx.push_scope();
        ctx.set_record_alias_from_ref("struct.S", "struct.S#1");
        ctx.push_scope();
        ctx.set_record_alias_from_ref("struct.S", "struct.S#2");
        ctx.set_record_alias_from_ref("struct.S", "struct.S#3");
        assert_eq!(ctx.resolve_record_key("struct.S").as_ref(), "struct.S#3");
        ctx.pop_scope();
        assert_eq!(ctx.resolve_record_key("struct.S").as_ref(), "struct.S#1");
        ctx.pop_scope();
        assert_eq!(ctx.resolve_record_key("struct.S").as_ref(), "struct.S");
    }

    #[test]
    fn variant_ids_are_unique_and_monotonic() {
        let ctx = TypeContext::new();
        let a = ctx.next_struct_variant_id();
        let b = ctx.next_struct_variant_id();
        assert_ne!(a, b);
        assert!(b > a, "ids must not repeat: {a} then {b}");
    }
}

#[cfg(test)]
mod enum_type_scope_tests {
    use super::TypeContext;
    use crate::common::types::EnumType;

    fn enum_type(name: &str, variant: (&str, i64), is_packed: bool) -> EnumType {
        EnumType {
            name: Some(name.to_string()),
            variants: vec![(variant.0.to_string(), variant.1)],
            is_packed,
            fixed: None,
        }
    }

    #[test]
    fn shadowed_enum_tag_restores_outer_definition() {
        let mut ctx = TypeContext::new();
        let outer = enum_type("E", ("NEG", -1), false);
        let inner = enum_type("E", ("ZERO", 0), false);
        ctx.insert_enum_type_scoped("E".into(), outer.clone());

        ctx.push_scope();
        ctx.insert_enum_type_scoped("E".into(), inner.clone());
        assert_eq!(ctx.enum_types.get("E"), Some(&inner));
        ctx.pop_scope();

        assert_eq!(ctx.enum_types.get("E"), Some(&outer));
    }

    #[test]
    fn enum_tag_introduced_in_scope_is_removed_and_nested_shadowing_unwinds() {
        let mut ctx = TypeContext::new();
        let outer = enum_type("E", ("NEG", -1), false);
        let inner = enum_type("E", ("ZERO", 0), false);
        let nested = enum_type("E", ("SMALL", 1), true);
        ctx.insert_enum_type_scoped("E".into(), outer.clone());

        ctx.push_scope();
        ctx.insert_enum_type_scoped("E".into(), inner.clone());
        ctx.push_scope();
        ctx.insert_enum_type_scoped("E".into(), nested);
        ctx.pop_scope();
        assert_eq!(ctx.enum_types.get("E"), Some(&inner));
        ctx.pop_scope();
        assert_eq!(ctx.enum_types.get("E"), Some(&outer));

        ctx.push_scope();
        ctx.insert_enum_type_scoped("Local".into(), enum_type("Local", ("L", -1), false));
        assert!(ctx.enum_types.contains_key("Local"));
        ctx.pop_scope();
        assert!(!ctx.enum_types.contains_key("Local"));
    }
}

#[cfg(test)]
mod ordinary_namespace_depth_tests {
    use super::{EnumConstant, TypeContext};
    use crate::common::symbol_table::{Symbol, SymbolTable};
    use crate::common::types::{AddressSpace, CType};

    fn object(name: &str) -> Symbol {
        Symbol {
            name: name.to_string(),
            ty: CType::Int,
            explicit_alignment: None,
            type_align: None,
            address_space: AddressSpace::Default,
        }
    }

    #[test]
    fn enumerator_and_object_maps_share_canonical_lexical_depths() {
        // Names resolve in the symbol table's one ordinary namespace; the
        // enumerator *values* live in `TypeContext::enum_constants`. The two
        // must bind and unwind at the same lexical depths.
        let mut types = TypeContext::new();
        let mut symbols = SymbolTable::new();
        let enumerator = |types: &mut TypeContext, symbols: &mut SymbolTable, name: &str, v| {
            types.insert_enum_scoped(name.to_string(), EnumConstant::provisional(v));
            symbols.declare(object(name));
            symbols.mark_enumerator(name);
        };

        enumerator(&mut types, &mut symbols, "file_enum", 1);
        symbols.declare(object("file_object"));
        assert_eq!(symbols.lookup_depth("file_enum"), Some(0));
        assert!(types.enum_constants.contains_key("file_enum"));
        assert_eq!(symbols.lookup_depth("file_object"), Some(0));

        for depth in 1..=3 {
            types.push_scope();
            symbols.push_scope();
            let enum_name = format!("enum_{depth}");
            let object_name = format!("object_{depth}");
            enumerator(&mut types, &mut symbols, &enum_name, depth as i64);
            symbols.declare(object(&object_name));
            assert_eq!(symbols.lookup_depth(&enum_name), Some(depth));
            assert!(symbols.lookup_ordinary(&enum_name).is_some_and(|(_, e)| e));
            assert!(types.enum_constants.contains_key(&enum_name));
            assert_eq!(symbols.lookup_depth(&object_name), Some(depth));
            assert!(
                symbols
                    .lookup_ordinary(&object_name)
                    .is_some_and(|(_, e)| !e)
            );
        }
        // An object in the innermost scope hides a file-scope enumerator.
        symbols.declare(object("file_enum"));
        assert!(
            symbols
                .lookup_ordinary("file_enum")
                .is_some_and(|(_, e)| !e)
        );

        for depth in (1..=3).rev() {
            types.pop_scope();
            symbols.pop_scope();
            let enum_name = format!("enum_{depth}");
            assert!(!types.enum_constants.contains_key(&enum_name));
            assert_eq!(symbols.lookup_depth(&enum_name), None);
            assert_eq!(symbols.lookup_depth(&format!("object_{depth}")), None);
        }
        assert!(symbols.lookup_ordinary("file_enum").is_some_and(|(_, e)| e));
        // File scope has no disposable undo frame, so an extra pop is a no-op.
        types.pop_scope();
        assert!(types.enum_constants.contains_key("file_enum"));
    }
}

#[cfg(test)]
mod qualified_pointee_typedef_tests {
    use super::TypeContext;

    #[test]
    fn block_scope_typedef_restores_the_outer_mark() {
        // `typedef const void *P;` at file scope, `typedef void *P;` in a
        // block: `(P)0` is a null pointer constant only inside the block.
        let mut ctx = TypeContext::new();
        ctx.set_typedef_pointee_qualified("P", true);
        ctx.push_scope();
        ctx.set_typedef_pointee_qualified("P", false);
        assert!(!ctx.qualified_pointee_typedefs.contains("P"));
        ctx.push_scope();
        ctx.set_typedef_pointee_qualified("P", true);
        ctx.set_typedef_pointee_qualified("Q", true);
        ctx.pop_scope();
        assert!(!ctx.qualified_pointee_typedefs.contains("P"));
        assert!(!ctx.qualified_pointee_typedefs.contains("Q"));
        ctx.pop_scope();
        assert!(ctx.qualified_pointee_typedefs.contains("P"));
    }
}
