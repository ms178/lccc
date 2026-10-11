//! Shared type-building utilities used by both sema and lowering.
//!
//! This module contains the canonical implementations of functions that convert
//! AST type syntax (TypeSpecifier + DerivedDeclarator chains) into CType values.
//!
//! The core `TypeConvertContext` trait provides a single `resolve_type_spec_to_ctype`
//! default method that handles all 22 primitive C types, pointers, arrays, and
//! function pointers identically. Only 4 cases differ between sema and lowering
//! (typedef names, struct/union, enum, typeof), so implementors provide just those
//! via required trait methods. This ensures primitive type mapping can never diverge.

use crate::common::types::{AddressSpace, CType, FunctionType};
use crate::frontend::parser::ast::{
    DerivedDeclarator, EnumVariant, Expr, ParamDecl, StructFieldDecl, TypeSpecifier,
};

/// Trait for contexts that can resolve types and evaluate constant expressions.
///
/// Both sema and lowering implement this trait. The shared `resolve_type_spec_to_ctype`
/// default method handles all primitive types, pointers, arrays, and function pointers.
/// Implementors only provide the 4 divergent methods for typedef, struct/union, enum,
/// and typeof resolution.
pub trait TypeConvertContext {
    /// Resolve a typedef name to its CType.
    /// Sema: looks up in type_context.typedefs.
    /// Lowering: also checks function pointer typedefs for richer type info.
    fn resolve_typedef(&self, name: &str) -> CType;

    /// Resolve a struct or union definition to its CType.
    /// Both phases compute layout, but lowering has caching and forward-declaration logic.
    fn resolve_struct_or_union(
        &self,
        name: &Option<String>,
        fields: &Option<Vec<StructFieldDecl>>,
        is_union: bool,
        is_packed: bool,
        pragma_pack: Option<usize>,
        struct_aligned: Option<usize>,
        reverse_sso: bool,
    ) -> CType;

    /// Resolve an enum type to its CType.
    /// Sema: returns CType::Enum with name info.
    /// Lowering: returns CType::Int (enums are ints at IR level).
    fn resolve_enum(
        &self,
        name: &Option<String>,
        variants: &Option<Vec<EnumVariant>>,
        is_packed: bool,
    ) -> CType;

    /// Resolve typeof(expr) to a CType.
    /// Sema: returns CType::Int (doesn't have full expr type resolution yet).
    /// Lowering: evaluates the expression's type.
    fn resolve_typeof_expr(&self, expr: &Expr) -> CType;

    /// Try to evaluate a constant expression to a usize (for array sizes).
    /// Returns None if the expression cannot be evaluated at compile time.
    fn eval_const_expr_as_usize(&self, expr: &Expr) -> Option<usize>;

    /// Convert a TypeSpecifier to a CType.
    ///
    /// This default implementation handles all shared cases (22 primitive types,
    /// Pointer, Array, FunctionPointer, TypeofType, AutoType) and delegates to
    /// the 4 required trait methods for the divergent cases.
    fn resolve_type_spec_to_ctype(&self, spec: &TypeSpecifier) -> CType {
        match spec {
            // === 22 primitive types (identical in sema and lowering) ===
            TypeSpecifier::Void => CType::Void,
            TypeSpecifier::Char => CType::Char,
            TypeSpecifier::UnsignedChar => CType::UChar,
            TypeSpecifier::Short => CType::Short,
            TypeSpecifier::UnsignedShort => CType::UShort,
            TypeSpecifier::Bool => CType::Bool,
            TypeSpecifier::Int | TypeSpecifier::Signed => CType::Int,
            TypeSpecifier::UnsignedInt | TypeSpecifier::Unsigned => CType::UInt,
            TypeSpecifier::Long => CType::Long,
            TypeSpecifier::UnsignedLong => CType::ULong,
            TypeSpecifier::LongLong => CType::LongLong,
            TypeSpecifier::UnsignedLongLong => CType::ULongLong,
            TypeSpecifier::Int128 => CType::Int128,
            TypeSpecifier::UnsignedInt128 => CType::UInt128,
            TypeSpecifier::Float => CType::Float,
            TypeSpecifier::Double => CType::Double,
            TypeSpecifier::Float128 => CType::Float128,
            TypeSpecifier::Decimal32 => CType::Decimal32,
            TypeSpecifier::Decimal64 => CType::Decimal64,
            TypeSpecifier::Decimal128 => CType::Decimal128,
            TypeSpecifier::LongDouble => CType::LongDouble,
            TypeSpecifier::ComplexFloat => CType::ComplexFloat,
            TypeSpecifier::ComplexDouble => CType::ComplexDouble,
            TypeSpecifier::ComplexLongDouble => CType::ComplexLongDouble,

            // === Compound types (shared logic) ===
            TypeSpecifier::Pointer(inner, addr_space) => {
                // A pointer chain is a LINEAR SPINE -- `int ****p` is
                // Pointer(Pointer(Pointer(Pointer(Int)))) -- so walking it is a
                // loop, not a recursion. This arm used to recurse one frame per
                // `*`, on a path with no frame budget at all: the parser's
                // PARSER_FRAME_BUDGET machinery covers expressions, blocks,
                // initializers and record definitions, but not this. Measured
                // under gdb, ~400k stars abort with rc=134 inside this very arm
                // ("has overflowed its stack") on a file GCC 16.2 accepts, and
                // the declaration alone is enough to trigger it -- no call site
                // involved. Collecting the address spaces on the way down and
                // wrapping on the way up moves the cost from the thread stack to
                // the heap, so the legal depth is bounded by memory instead of
                // by a fixed stack size.
                //
                // The single-level fast path keeps the common case identical to
                // the old code: no scratch allocation, one call, one Box. Only
                // a genuinely multi-level pointer pays for the Vec, and even
                // then it trades N call frames for N pushes, which is cheaper.
                if !matches!(&**inner, TypeSpecifier::Pointer(..)) {
                    return CType::Pointer(
                        Box::new(self.resolve_type_spec_to_ctype(inner)),
                        *addr_space,
                    );
                }
                let mut spaces: Vec<AddressSpace> = vec![*addr_space];
                let mut node: &TypeSpecifier = inner;
                while let TypeSpecifier::Pointer(next, space) = node {
                    spaces.push(*space);
                    node = next.as_ref();
                }
                let mut ty = self.resolve_type_spec_to_ctype(node);
                for space in spaces.into_iter().rev() {
                    ty = CType::Pointer(Box::new(ty), space);
                }
                ty
            }
            TypeSpecifier::Array(elem, size_expr) => {
                let elem_ctype = self.resolve_type_spec_to_ctype(elem);
                let size = size_expr
                    .as_ref()
                    .and_then(|e| self.eval_const_expr_as_usize(e));
                CType::Array(Box::new(elem_ctype), size)
            }
            TypeSpecifier::FunctionPointer(return_type, params, variadic) => {
                let ret_ctype = self.resolve_type_spec_to_ctype(return_type);
                let param_ctypes: Vec<(CType, Option<String>)> = params
                    .iter()
                    .map(|p| {
                        let ty = self.resolve_type_spec_to_ctype(&p.type_spec);
                        (ty, p.name.clone())
                    })
                    .collect();
                CType::Pointer(
                    Box::new(CType::Function(Box::new(FunctionType {
                        return_type: ret_ctype,
                        params: param_ctypes,
                        variadic: *variadic,
                    }))),
                    AddressSpace::Default,
                )
            }
            TypeSpecifier::BareFunction(return_type, params, variadic) => {
                // Bare function type (no pointer wrapper) — produced by typeof on
                // function names. Resolves to CType::Function, NOT Pointer(Function).
                let ret_ctype = self.resolve_type_spec_to_ctype(return_type);
                let param_ctypes: Vec<(CType, Option<String>)> = params
                    .iter()
                    .map(|p| {
                        let ty = self.resolve_type_spec_to_ctype(&p.type_spec);
                        (ty, p.name.clone())
                    })
                    .collect();
                CType::Function(Box::new(FunctionType {
                    return_type: ret_ctype,
                    params: param_ctypes,
                    variadic: *variadic,
                }))
            }
            TypeSpecifier::TypeofType(inner) => self.resolve_type_spec_to_ctype(inner),
            TypeSpecifier::AutoType => CType::Int,

            // === Divergent cases (delegated to implementors) ===
            TypeSpecifier::TypedefName(name) => self.resolve_typedef(name),
            TypeSpecifier::Struct(
                name,
                fields,
                is_packed,
                pragma_pack,
                struct_aligned,
                reverse_sso,
            ) => self.resolve_struct_or_union(
                name,
                fields,
                false,
                *is_packed,
                *pragma_pack,
                *struct_aligned,
                reverse_sso.is_some_and(|v| v),
            ),
            TypeSpecifier::Union(
                name,
                fields,
                is_packed,
                pragma_pack,
                struct_aligned,
                reverse_sso,
            ) => self.resolve_struct_or_union(
                name,
                fields,
                true,
                *is_packed,
                *pragma_pack,
                *struct_aligned,
                reverse_sso.is_some_and(|v| v),
            ),
            TypeSpecifier::Enum(name, variants, is_packed) => {
                self.resolve_enum(name, variants, *is_packed)
            }
            TypeSpecifier::Typeof(expr) => self.resolve_typeof_expr(expr),
            TypeSpecifier::Vector(inner, total_bytes) => {
                let elem_ctype = self.resolve_type_spec_to_ctype(inner);
                CType::Vector(Box::new(elem_ctype), *total_bytes)
            }
        }
    }
}

/// Find the start index of the function pointer core in a derived declarator list.
///
/// The function pointer core is one of:
/// - `[Pointer, FunctionPointer]` — the `(*name)(params)` syntax
/// - Standalone `FunctionPointer` — direct function pointer declarator
/// - Standalone `Function` — function declaration (not pointer)
///
/// Returns `Some(index)` where the core begins, or `None` if no function
/// pointer/function declarator is present.
fn find_function_pointer_core(derived: &[DerivedDeclarator]) -> Option<usize> {
    for i in 0..derived.len() {
        // Look for Pointer followed by FunctionPointer
        if matches!(&derived[i], DerivedDeclarator::Pointer)
            && i + 1 < derived.len()
            && matches!(&derived[i + 1], DerivedDeclarator::FunctionPointer(_, _))
        {
            return Some(i);
        }
        // Standalone FunctionPointer
        if matches!(&derived[i], DerivedDeclarator::FunctionPointer(_, _)) {
            return Some(i);
        }
        // Standalone Function (for function declarations)
        if matches!(&derived[i], DerivedDeclarator::Function(_, _)) {
            return Some(i);
        }
    }
    None
}

/// Convert a ParamDecl list to a list of (CType, Option<name>) pairs.
///
/// Uses the provided `TypeConvertContext` to resolve each parameter's type.
/// Convert one parameter declaration to the CType it denotes.
///
/// This is the single implementation, and it exists because there used to be
/// two. This one resolved only `p.type_spec`; `SemanticAnalyzer::param_decl_ctype`
/// resolved the function-pointer fields as well. A parameter list reaches the
/// compiler by two routes -- a prototype goes through `build_full_ctype_with_base`
/// (here) and a definition through the analyzer -- so the same signature was
/// typed differently depending on whether the body had been seen yet:
///
///     void f(int (*p)(int));              // prototype: p is `int *`
///     void f(int (*p)(int)) { ... }       // definition: p is `int (*)(int)`
///
/// and `void g(void) { int (*q)(int); f(q); }` was a hard type error against the
/// prototype while the identical call to the definition was fine. The analyzer's
/// doc comment already recorded this failure mode for the SQLite amalgamation
/// (`int (*xStress)(void*, PgHdr*)` typed as `int *`); it was fixed on one side
/// of the duplication and not the other. Both sides now call this.
pub fn param_decl_to_ctype(ctx: &dyn TypeConvertContext, param: &ParamDecl) -> CType {
    if let Some(ref fptr_params) = param.fptr_params {
        // `float (*func)(float, float)`: the specifier is the pointee function's
        // return type, and the parenthesised declarator's own parameter list
        // rides on `fptr_params`.
        let return_ctype = ctx.resolve_type_spec_to_ctype(&param.type_spec);
        let actual_return = match return_ctype {
            CType::Pointer(inner, _) => *inner,
            other => other,
        };
        let param_types: Vec<(CType, Option<String>)> = fptr_params
            .iter()
            .map(|p| (param_decl_to_ctype(ctx, p), p.name.clone()))
            .collect();
        let func_type = CType::Function(Box::new(FunctionType {
            return_type: actual_return,
            params: param_types,
            variadic: param.fptr_variadic,
        }));
        let mut result = CType::Pointer(Box::new(func_type), AddressSpace::Default);
        // `(**fpp)(int)` is a pointer to a function pointer; CType would
        // otherwise erase the distinction from `void *(*fp)(size_t)`.
        for _ in 1..param.fptr_inner_ptr_depth.max(1) {
            result = CType::Pointer(Box::new(result), AddressSpace::Default);
        }
        return result;
    }
    // C23 6.7.6.3p8: a parameter declared as an array is adjusted to a pointer
    // to its first element, and one declared as a function to a pointer to that
    // function. Both adjustments belong to the parameter, not to the caller.
    match ctx.resolve_type_spec_to_ctype(&param.type_spec) {
        CType::Array(elem, _) => CType::Pointer(elem, AddressSpace::Default),
        CType::Function(ft) => CType::Pointer(Box::new(CType::Function(ft)), AddressSpace::Default),
        other => other,
    }
}

fn convert_param_decls_to_ctypes(
    ctx: &dyn TypeConvertContext,
    params: &[ParamDecl],
) -> Vec<(CType, Option<String>)> {
    params
        .iter()
        .map(|p| (param_decl_to_ctype(ctx, p), p.name.clone()))
        .collect()
}

/// Build a full CType from a TypeSpecifier and DerivedDeclarator chain.
///
/// The derived list is produced by the parser's declarator handling, which stores
/// declarators outer-to-inner. For building the CType, we process inner-to-outer
/// (the C "inside-out" declarator rule).
///
/// This is the single canonical implementation used by both sema and lowering.
///
/// Examples (derived list → CType):
/// - `int **p`: [Pointer, Pointer] → Pointer(Pointer(Int))
/// - `int *arr[3]`: [Pointer, Array(3)] → Array(Pointer(Int), 3)
/// - `int (*fp)(int)`: [Pointer, FunctionPointer([int])] → Pointer(Function(Int→Int))
/// - `int (*fp[3])(int)`: [Array(3), Pointer, FunctionPointer([int])] → Array(Pointer(Function(Int→Int)), 3)
/// - `Page *(*xFetch)(int)`: [Pointer, Pointer, FunctionPointer([int])] → Pointer(Function(Pointer(Page)→...))
pub fn build_full_ctype(
    ctx: &dyn TypeConvertContext,
    type_spec: &crate::frontend::parser::ast::TypeSpecifier,
    derived: &[DerivedDeclarator],
) -> CType {
    let base = ctx.resolve_type_spec_to_ctype(type_spec);
    build_full_ctype_with_base(ctx, base, derived)
}

/// Build a full CType from an already-resolved base type and derived declarators.
/// Use this instead of `build_full_ctype` when the base type has already been
/// resolved (e.g., to avoid re-resolving anonymous struct type specs which would
/// generate different anonymous struct keys for the same declaration).
pub fn build_full_ctype_with_base(
    ctx: &dyn TypeConvertContext,
    base: CType,
    derived: &[DerivedDeclarator],
) -> CType {
    let fptr_idx = find_function_pointer_core(derived);

    if let Some(fp_start) = fptr_idx {
        // Build the function pointer type.
        // Pointer declarators in the prefix (before fp_start) are part of the
        // return type, not outer wrappers. E.g. for `Page *(*xFetch)(int)`:
        //   derived = [Pointer, Pointer, FunctionPointer([int])]
        //   prefix  = [Pointer]  — the `*` on return type `Page *`
        //   core    = [Pointer, FunctionPointer] — the `(*)(int)` syntax
        // We fold prefix Pointer declarators into the base to form the return type.
        let mut result = base;
        for d in &derived[..fp_start] {
            if matches!(d, DerivedDeclarator::Pointer) {
                result = CType::Pointer(Box::new(result), AddressSpace::Default);
            }
            // Array declarators in prefix are outer wrappers, handled after the core.
        }

        // Process from fp_start to end (the function pointer core and any
        // additional inner wrappers after it)
        let mut i = fp_start;
        while i < derived.len() {
            match &derived[i] {
                DerivedDeclarator::Pointer => {
                    if i + 1 < derived.len()
                        && matches!(
                            &derived[i + 1],
                            DerivedDeclarator::FunctionPointer(_, _)
                                | DerivedDeclarator::Function(_, _)
                        )
                    {
                        let (params, variadic) = match &derived[i + 1] {
                            DerivedDeclarator::FunctionPointer(p, v)
                            | DerivedDeclarator::Function(p, v) => (p, *v),
                            _ => unreachable!(
                                "expected FunctionPointer/Function declarator after Pointer"
                            ),
                        };
                        let param_types = convert_param_decls_to_ctypes(ctx, params);
                        let func_type = CType::Function(Box::new(FunctionType {
                            return_type: result,
                            params: param_types,
                            variadic,
                        }));
                        result = CType::Pointer(Box::new(func_type), AddressSpace::Default);
                        i += 2;
                    } else {
                        result = CType::Pointer(Box::new(result), AddressSpace::Default);
                        i += 1;
                    }
                }
                DerivedDeclarator::FunctionPointer(params, variadic) => {
                    let param_types = convert_param_decls_to_ctypes(ctx, params);
                    let func_type = CType::Function(Box::new(FunctionType {
                        return_type: result,
                        params: param_types,
                        variadic: *variadic,
                    }));
                    result = CType::Pointer(Box::new(func_type), AddressSpace::Default);
                    i += 1;
                }
                DerivedDeclarator::Function(params, variadic) => {
                    let param_types = convert_param_decls_to_ctypes(ctx, params);
                    let func_type = CType::Function(Box::new(FunctionType {
                        return_type: result,
                        params: param_types,
                        variadic: *variadic,
                    }));
                    result = func_type;
                    i += 1;
                }
                DerivedDeclarator::Array(size_expr) => {
                    // Array declarators after the function pointer core are outer
                    // wrappers (e.g., array-of-function-pointers when inner_derived
                    // had [Pointer, Array(N)] which the parser emits as
                    // [Pointer, FunctionPointer, Array(N)]).
                    let size = size_expr
                        .as_ref()
                        .and_then(|e| ctx.eval_const_expr_as_usize(e));
                    result = CType::Array(Box::new(result), size);
                    i += 1;
                }
            }
        }

        // Apply outer wrappers from the prefix (before fp_start).
        // Only Array declarators in the prefix are true outer wrappers
        // (e.g., `int (*fp[10])(void)` = array of function pointers).
        // Pointer declarators in the prefix were already folded into the return type.
        let prefix = &derived[..fp_start];
        for d in prefix.iter().rev() {
            if let DerivedDeclarator::Array(size_expr) = d {
                let size = size_expr
                    .as_ref()
                    .and_then(|e| ctx.eval_const_expr_as_usize(e));
                result = CType::Array(Box::new(result), size);
            }
        }

        result
    } else {
        // No function pointer — simple case: apply pointers and arrays
        let mut result = base;
        let mut i = 0;
        while i < derived.len() {
            match &derived[i] {
                DerivedDeclarator::Pointer => {
                    result = CType::Pointer(Box::new(result), AddressSpace::Default);
                    i += 1;
                }
                DerivedDeclarator::Array(_) => {
                    // Collect consecutive array dimensions
                    let start = i;
                    while i < derived.len() && matches!(&derived[i], DerivedDeclarator::Array(_)) {
                        i += 1;
                    }
                    // Apply in reverse: innermost (rightmost) dimension wraps first
                    for j in (start..i).rev() {
                        if let DerivedDeclarator::Array(size_expr) = &derived[j] {
                            let size = size_expr
                                .as_ref()
                                .and_then(|e| ctx.eval_const_expr_as_usize(e));
                            result = CType::Array(Box::new(result), size);
                        }
                    }
                }
                _ => {
                    i += 1;
                }
            }
        }
        result
    }
}

/// Plant a declaration-level address-space qualifier (`__seg_fs`/`__seg_gs`)
/// on the pointer level it qualifies in C: the INNERMOST one.
///
/// GCC named address spaces qualify the *pointee* memory:
///   `T __seg_fs *p`   — p (ordinary memory) points into %fs
///   `T __seg_fs **pp` — pp and *pp are ordinary; **pp reads %fs
///   `T __seg_fs *a[4]`— each element points into %fs
/// In this IR, `CType::Pointer(pointee, AddressSpace)` carries "the space the
/// pointer points into", so the qualifier belongs on the innermost Pointer
/// (the one whose pointee is the qualified data), reached through Array
/// elements and outer Pointer levels.
///
/// The parser records the qualifier on the whole `Declaration`
/// (`parsing_address_space`), but `build_full_ctype*` used to hardcode
/// `AddressSpace::Default` for every derived Pointer, so any *named* variable
/// or typedef of a segment pointer silently lost its qualifier and its
/// dereferences read absolute addresses (glibc TLS: `%fs:16`/`%fs:40`
/// stack-guard loads compiled to NULL-page loads). Only direct
/// `*(T __seg_fs *)N` casts survived, via TypeSpecifier::Pointer's own field.
/// Returns `true` when the space was absorbed by a pointer's POINTEE, `false`
/// when it was not consumed and therefore belongs to the declared object's own
/// storage. The distinction is not cosmetic: for `T __seg_gs v;` the object `v`
/// lives in the segment (`&v` is `__seg_gs T *`), but for `T __seg_gs *p;` the
/// space qualifies the type *name* — GCC reads a named address space as a type
/// qualifier, exactly like `const T *p` — so `p` is a pointer INTO the segment
/// while the object `p` itself has default storage and `&p` is
/// `__seg_gs T **` with a default-space outer pointer. Sema folds this return
/// value into `Symbol::address_space`; getting it wrong double-applies the
/// qualifier (observed as `have '__seg_gs __seg_gs unsigned long **'` on
/// include/linux/percpu-refcount.h).
pub fn apply_declaration_address_space(ty: &mut CType, space: AddressSpace) -> bool {
    if space == AddressSpace::Default {
        return false;
    }
    match ty {
        CType::Pointer(inner, sp) => {
            if matches!(inner.as_ref(), CType::Pointer(..)) {
                apply_declaration_address_space(inner, space)
            } else {
                *sp = space;
                true
            }
        }
        CType::Array(elem, _) => apply_declaration_address_space(elem, space),
        // `T __seg_gs *f(void)`: the qualifier belongs to the pointer in the
        // return type, not to the function designator itself. Without this arm
        // a function returning a per-CPU pointer (srcu_read_lock_fast) reported
        // a plain pointer at every call site, so a correctly qualified argument
        // was rejected as "incompatible pointer type".
        CType::Function(ft) => apply_declaration_address_space(&mut ft.return_type, space),
        // A plain object (or an array of plain objects): the data IS the object,
        // so the space is the object's own storage and nothing absorbed it.
        _ => false,
    }
}

/// Whether the declarator itself supplies a pointer level (`*` or a
/// function-pointer `(*f)`), as opposed to the pointer coming from the type
/// specifier through a typedef or `typeof` name.
///
/// Measured against gcc 14.2 (see [`apply_declaration_address_space_for_object`]):
/// this is the discriminator between the two spellings of a per-CPU object.
pub fn declarator_supplies_pointer(derived: &[DerivedDeclarator]) -> bool {
    derived.iter().any(|d| {
        matches!(
            d,
            DerivedDeclarator::Pointer | DerivedDeclarator::FunctionPointer(_, _)
        )
    })
}

/// Declaration-level address space for an OBJECT declaration — the rule that
/// decides between the pointee and the object's storage.
///
/// gcc applies the space to the type *as it stands at that point in the
/// declaration*:
///
/// ```c
/// typedef int *intp;
/// static int        __seg_gs *p1;   /* declarator supplies `*`  */
/// static __seg_gs   intp       p2;   /* specifier supplies `*`  */
/// ```
///
/// * `p1`: the space qualifies the base type, so the declarator's `*` makes it
///   `__seg_gs int *` — a pointer into the segment.  `typeof(p1)` is
///   `__seg_gs int *`.  (Absorbed into the pointee: the existing rule.)
/// * `p2`: the specifier's type is already `int *`; the space cannot qualify
///   the pointee of a type it never touched, so gcc leaves the VALUE's type
///   plain (`typeof(p2) == int *`) and the space qualifies the object's
///   storage — `typeof(&p2) == __seg_gs int **`.
///
/// Linux's `DEFINE_PER_CPU(type, name)` is the second spelling
/// (`__PCPU_ATTRS(sec) __typeof__(type) name`), and `per_cpu(var, cpu)`
/// dereferences `PERCPU_PTR(&var)` — which is compiled as
/// `(TYPEOF_UNQUAL(*(&var)) *)(unsigned long)(&var)`.  With the space on the
/// pointee instead of the storage, that chain yields a `__seg_gs`-qualified
/// pointer where gcc yields a plain one, and `device_unregister(per_cpu(...))`
/// is rejected (drivers/base/cacheinfo.c).  Storage placement also makes the
/// object's own reads/writes segment-relative, which is what the kernel means.
///
/// Returns whether the type absorbed the space (pointee placement). Callers
/// that declare objects must use the result to set the object's storage space
/// (see sema's `object_address_space` / the lowerer's `var.address_space`).
pub fn apply_declaration_address_space_for_object(
    ty: &mut CType,
    space: AddressSpace,
    derived: &[DerivedDeclarator],
) -> bool {
    if space == AddressSpace::Default {
        return false;
    }
    if !declarator_supplies_pointer(derived) {
        // The pointer level (if any) came from the specifier: the space is the
        // object's storage and the value's type is left alone.
        return false;
    }
    apply_declaration_address_space(ty, space)
}

/// The object's OWN storage space for a declaration — the exact complement of
/// [`apply_declaration_address_space_for_object`], sharing its predicate so the
/// two can never drift.
///
/// A pointer supplied by the DECLARATOR absorbs the qualifier into its pointee,
/// so the declared object keeps DEFAULT storage: gcc reads element `a[1]` of
///
/// ```c
/// int __seg_gs *a[4];
/// ```
/// as `movq 8+a(%rip), %rdx` and only the pointee dereference carries the
/// override (`movl %gs:(%rdx), %eax`).  The object rule alone was not enough:
/// the storage slot (`GlobalInfo::var.address_space` / `LocalInfo::var`) was
/// filled from `decl.address_space` unconditionally, which turned that array
/// into a genuinely segment-relative object — every element load became
/// `movq %gs:a+8(%rip)` (segment base + symbol offset, i.e. a wild address
/// once a GS base is installed) and the recursive `off = p->next`
/// segment-pointer walk read the wrong node forever (reduced: `int __seg_gs
/// *garr[4]; int *loadidx(void){ return garr[1]; }`).  Storage space is
/// therefore the declaration's qualifier ONLY when no declarator pointer
/// absorbed it.
pub fn object_storage_address_space(
    space: AddressSpace,
    derived: &[DerivedDeclarator],
) -> AddressSpace {
    if space == AddressSpace::Default || declarator_supplies_pointer(derived) {
        AddressSpace::Default
    } else {
        space
    }
}

#[cfg(test)]
mod address_space_tests {
    use super::*;

    /// The object's own storage space is the declaration space only when no
    /// pointer in the declarator absorbed it. These four shapes are measured
    /// against gcc 14.2 (see the function's contract) and are what sema folds
    /// into `Symbol::address_space`; getting any of them backwards either
    /// breaks `&percpu_obj` (kernel/softirq.c) or double-qualifies the pointee
    /// (include/linux/percpu-refcount.h).
    #[test]
    fn object_space_is_absorbed_by_pointer_declarators() {
        // `T __seg_gs v;` — plain object: the object itself lives in the segment.
        let mut ty = CType::Int;
        assert!(!apply_declaration_address_space(
            &mut ty,
            AddressSpace::SegGs
        ));
        assert_eq!(ty, CType::Int);

        // `T __seg_gs *p;` — a pointer: the space qualifies the POINTEE, so the
        // object `p` keeps default storage and `&p` is `__seg_gs T **`.
        let mut ty = CType::Pointer(Box::new(CType::Int), AddressSpace::Default);
        assert!(apply_declaration_address_space(
            &mut ty,
            AddressSpace::SegGs
        ));
        assert_eq!(
            ty,
            CType::Pointer(Box::new(CType::Int), AddressSpace::SegGs)
        );

        // `T __seg_gs **p;` — outer pointer levels are traversed to the innermost
        // pointer, so the space still lands on the data pointer.
        let mut ty = CType::Pointer(
            Box::new(CType::Pointer(Box::new(CType::Int), AddressSpace::Default)),
            AddressSpace::Default,
        );
        assert!(apply_declaration_address_space(
            &mut ty,
            AddressSpace::SegGs
        ));
        assert_eq!(
            ty,
            CType::Pointer(
                Box::new(CType::Pointer(Box::new(CType::Int), AddressSpace::SegGs)),
                AddressSpace::Default
            )
        );

        // `T __seg_fs f(void);` — the space belongs to the returned pointer, not
        // to the function designator (srcu_read_lock_fast call sites).
        let mut ty = CType::Function(Box::new(FunctionType {
            return_type: CType::Pointer(Box::new(CType::Int), AddressSpace::Default),
            params: Vec::new(),
            variadic: false,
        }));
        assert!(apply_declaration_address_space(
            &mut ty,
            AddressSpace::SegFs
        ));

        // Default space is a no-op and always reports "nothing absorbed".
        let mut ty = CType::Pointer(Box::new(CType::Int), AddressSpace::Default);
        assert!(!apply_declaration_address_space(
            &mut ty,
            AddressSpace::Default
        ));
    }

    /// The object rule adds the declarator to the decision (gcc 14.2,
    /// `spacecases.c`): a pointer supplied by the DECLARATOR absorbs the
    /// space into its pointee; a pointer supplied by the TYPE SPECIFIER
    /// (typedef/typeof, i.e. the kernel's `DEFINE_PER_CPU` spelling) leaves
    /// the value's type plain and the space on the object's storage.
    #[test]
    fn object_rule_distinguishes_declarator_from_specifier_pointers() {
        let ptr = CType::Pointer(Box::new(CType::Int), AddressSpace::Default);

        // `int __percpu *p;` — declarator supplies the `*`: pointee placement,
        // exactly like the raw function.
        let mut ty = ptr.clone();
        assert!(apply_declaration_address_space_for_object(
            &mut ty,
            AddressSpace::SegGs,
            &[DerivedDeclarator::Pointer]
        ));
        assert_eq!(
            ty,
            CType::Pointer(Box::new(CType::Int), AddressSpace::SegGs)
        );

        // `__percpu intp p;` (intp = int *) — specifier supplies the pointer:
        // the type must come back UNCHANGED (plain `int *`) and the space is
        // the object's storage.
        let mut ty = ptr.clone();
        assert!(!apply_declaration_address_space_for_object(
            &mut ty,
            AddressSpace::SegGs,
            &[]
        ));
        assert_eq!(ty, ptr);

        // `__percpu int **p;` — pointer levels in the declarator absorb, and
        // the walk still reaches the innermost pointer: the result is
        // `Pointer(Pointer(Int, SegGs), Default)`, i.e. `__seg_gs int **`
        // (gcc 14.2 agrees: `typeof(p) == __seg_gs int **`).
        let mut ty = CType::Pointer(Box::new(ptr.clone()), AddressSpace::Default);
        assert!(apply_declaration_address_space_for_object(
            &mut ty,
            AddressSpace::SegGs,
            &[DerivedDeclarator::Pointer, DerivedDeclarator::Pointer]
        ));
        assert_eq!(
            ty,
            CType::Pointer(
                Box::new(CType::Pointer(Box::new(CType::Int), AddressSpace::SegGs)),
                AddressSpace::Default
            )
        );

        // `__percpu struct device *f(void);` — a function returning a pointer:
        // the `*` is in the declarator, so the return type absorbs.
        let mut ty = CType::Function(Box::new(FunctionType {
            return_type: ptr.clone(),
            params: Vec::new(),
            variadic: false,
        }));
        assert!(apply_declaration_address_space_for_object(
            &mut ty,
            AddressSpace::SegGs,
            &[
                DerivedDeclarator::Pointer,
                DerivedDeclarator::Function(Vec::new(), false)
            ]
        ));

        // `__percpu int arr[4];` — no pointer anywhere: storage placement.
        let mut ty = CType::Array(Box::new(CType::Int), Some(4));
        assert!(!apply_declaration_address_space_for_object(
            &mut ty,
            AddressSpace::SegGs,
            &[DerivedDeclarator::Array(None)]
        ));
        assert_eq!(ty, CType::Array(Box::new(CType::Int), Some(4)));

        // Storage complement — the SAME predicate, so the object's storage and
        // its pointee placement can never disagree.  Only a declaration whose
        // pointer level did NOT come from the declarator puts its storage in
        // the segment; `int __seg_gs *a[4]` is a NORMAL array of pointers into
        // the segment (gcc reads `a[1]` as `8+a(%rip)`), while
        // `__seg_gs intp d[4]` (pointer typedef) stores the array itself in the
        // segment (gcc: `%gs:8+d(%rip)`).
        assert_eq!(
            object_storage_address_space(
                AddressSpace::SegGs,
                &[DerivedDeclarator::Pointer, DerivedDeclarator::Array(None)]
            ),
            AddressSpace::Default
        );
        assert_eq!(
            object_storage_address_space(AddressSpace::SegGs, &[DerivedDeclarator::Array(None)]),
            AddressSpace::SegGs
        );
        assert_eq!(
            object_storage_address_space(AddressSpace::SegGs, &[]),
            AddressSpace::SegGs
        );
        assert_eq!(
            object_storage_address_space(AddressSpace::SegFs, &[DerivedDeclarator::Pointer]),
            AddressSpace::Default
        );
        assert_eq!(
            object_storage_address_space(AddressSpace::Default, &[]),
            AddressSpace::Default
        );

        // Function pointers count as declarator-supplied pointers.
        assert!(declarator_supplies_pointer(&[
            DerivedDeclarator::Pointer,
            DerivedDeclarator::FunctionPointer(Vec::new(), false)
        ]));
        assert!(!declarator_supplies_pointer(&[
            DerivedDeclarator::Array(None),
            DerivedDeclarator::Function(Vec::new(), false)
        ]));
    }
}
