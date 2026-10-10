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
///
/// Placement rule (shared with `apply_decl_address_space_to_spec` in
/// `frontend/parser/declarations.rs`, which must stay in step with it): the
/// named space qualifies the memory the INNERMOST pointer points into. The
/// walk descends through pointers, arrays and function return types until it
/// reaches a pointer whose direct pointee is not itself derived, and marks
/// that pointer. Outer levels stay ordinary memory:
///   * `T __seg_gs *p`        -> the pointer to T
///   * `T __seg_gs **pp`      -> the pointer to T (inner); `pp` stays generic
///   * `T __seg_gs *a[N]`     -> the element pointers
///   * `T __seg_gs *(*f)(void)` and `T __seg_gs *f(void)` -> the pointer in the
///     return type; the function pointer itself stays generic.
///
/// The pre-fix version applied the space to the outer pointer whenever the
/// inner type was a function, so `f()->member` loaded through an absolute
/// address (srcu_read_lock_fast's per-CPU counter pointer).
pub fn apply_declaration_address_space(ty: &mut CType, space: AddressSpace) {
    if space == AddressSpace::Default {
        return;
    }
    match ty {
        CType::Pointer(inner, sp) => {
            if matches!(
                inner.as_ref(),
                CType::Pointer(..) | CType::Array(..) | CType::Function(..)
            ) {
                apply_declaration_address_space(inner, space);
            } else {
                *sp = space;
            }
        }
        CType::Array(elem, _) => apply_declaration_address_space(elem, space),
        CType::Function(ft) => apply_declaration_address_space(&mut ft.return_type, space),
        _ => {}
    }
}

/// Does the named address space qualify the declared type as a whole?
///
/// `__seg_gs` written against a typedef name or `typeof` qualifies the type
/// itself, so for `__seg_gs __typeof__(T *) p;` the pointer object `p` is the
/// one in the segment (the kernel's per-CPU pointer variables). Written against
/// a base specifier (`__seg_gs T *p;`) it qualifies the base type, and so the
/// pointee. The two cases are distinguished by the type specifier alone.
///
/// A typedef or typeof base only qualifies the whole object when the declarator
/// adds no pointer level: `typedef long L; L __seg_gs *p;` makes `p` ordinary
/// memory pointing into %gs, exactly like `long __seg_gs *p;`. Without this
/// check the typedef path put `p` itself in %gs and loaded it through the
/// segment (silent miscompile).
pub fn named_space_qualifies_whole_type(
    type_spec: &TypeSpecifier,
    derived: &[DerivedDeclarator],
) -> bool {
    let typedef_base = matches!(
        type_spec,
        TypeSpecifier::TypedefName(_) | TypeSpecifier::Typeof(_) | TypeSpecifier::TypeofType(_)
    );
    typedef_base
        && !derived.iter().any(|d| {
            matches!(
                d,
                DerivedDeclarator::Pointer | DerivedDeclarator::FunctionPointer(..)
            )
        })
}

/// Apply a declaration's named address space to the declared object's type and
/// return the storage space of the object itself.
///
/// This is the single entry point for declaration sites (file scope, asm
/// register globals, extern re-declarations, locals). A whole-type qualifier
/// (`named_space_qualifies_whole_type`) is the object's own space and leaves
/// the type unchanged. Otherwise the space is placed on the declared type
/// (`apply_declaration_address_space`) and the storage follows
/// `declared_object_space`. With no CType available the declaration space is
/// the storage space, as before.
pub fn place_declared_space(
    ct: Option<&mut CType>,
    space: AddressSpace,
    type_spec: &TypeSpecifier,
    derived: &[DerivedDeclarator],
) -> AddressSpace {
    let whole_type = named_space_qualifies_whole_type(type_spec, derived);
    match ct {
        None => space,
        Some(ct) => {
            if !whole_type {
                apply_declaration_address_space(ct, space);
            }
            declared_object_space(space, ct, whole_type)
        }
    }
}

/// Address space in which the object a declaration declares is stored.
///
/// * `whole_type` (see `named_space_qualifies_whole_type`): the object is in
///   `space`, whatever its shape.
/// * Otherwise `space` qualifies the base type. A plain object
///   (`int __seg_gs x;`, `struct pcpu __seg_gs arr[4];`) lives in `space`.
///   Once the declared type has a pointer level (`struct c __seg_gs *g;`,
///   `struct c __seg_gs *a[4];`) the space belongs to the pointee, so the
///   pointer object is ordinary memory. Using `space` as the storage for these
///   made `g` load itself through %gs.
///
/// Call this after `apply_declaration_address_space` on the same type, and only
/// when `whole_type` is false (a whole-type qualifier does not reach the
/// pointee).
pub fn declared_object_space(space: AddressSpace, ty: &CType, whole_type: bool) -> AddressSpace {
    if space == AddressSpace::Default {
        AddressSpace::Default
    } else if whole_type {
        space
    } else if has_pointer_level(ty) {
        AddressSpace::Default
    } else {
        space
    }
}

fn has_pointer_level(ty: &CType) -> bool {
    match ty {
        CType::Pointer(..) => true,
        CType::Array(elem, _) => has_pointer_level(elem),
        _ => false,
    }
}

#[cfg(test)]
mod seg_placement_tests {
    use super::*;

    /// Pointee of every test pointer: a non-derived scalar.
    fn base() -> CType {
        CType::Long
    }
    fn gs_ptr_plain() -> CType {
        CType::Pointer(Box::new(base()), AddressSpace::Default)
    }
    fn func_returning(ret: CType) -> CType {
        CType::Function(Box::new(FunctionType {
            return_type: ret,
            params: Vec::new(),
            variadic: false,
        }))
    }
    /// Space of the pointer that directly points at the base struct, found by
    /// walking from `ty` down the pointer/array/function chain.
    fn innermost_space(ty: &CType) -> Option<AddressSpace> {
        match ty {
            CType::Pointer(inner, sp) => match inner.as_ref() {
                CType::Pointer(..) | CType::Array(..) | CType::Function(..) => {
                    innermost_space(inner)
                }
                _ => Some(*sp),
            },
            CType::Array(elem, _) => innermost_space(elem),
            CType::Function(ft) => innermost_space(&ft.return_type),
            _ => None,
        }
    }
    /// Space stored on the outermost pointer, if `ty` is a pointer.
    fn outer_space(ty: &CType) -> Option<AddressSpace> {
        match ty {
            CType::Pointer(_, sp) => Some(*sp),
            _ => None,
        }
    }

    #[test]
    fn plain_pointer_gets_its_own_space() {
        let mut t = gs_ptr_plain();
        apply_declaration_address_space(&mut t, AddressSpace::SegGs);
        assert_eq!(outer_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn pointer_to_pointer_qualifies_only_the_inner_level() {
        let mut t = CType::Pointer(Box::new(gs_ptr_plain()), AddressSpace::Default);
        apply_declaration_address_space(&mut t, AddressSpace::SegGs);
        assert_eq!(outer_space(&t), Some(AddressSpace::Default));
        assert_eq!(innermost_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn pointer_to_function_returning_gs_pointer_qualifies_the_return_pointer() {
        // struct c __seg_gs *(*fp)(void)
        let mut t = CType::Pointer(
            Box::new(func_returning(gs_ptr_plain())),
            AddressSpace::Default,
        );
        apply_declaration_address_space(&mut t, AddressSpace::SegGs);
        assert_eq!(
            outer_space(&t),
            Some(AddressSpace::Default),
            "fp itself stays generic"
        );
        assert_eq!(innermost_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn function_returning_gs_pointer_qualifies_the_return_pointer() {
        // struct c __seg_gs *f(void)
        let mut t = func_returning(gs_ptr_plain());
        apply_declaration_address_space(&mut t, AddressSpace::SegGs);
        assert_eq!(innermost_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn array_of_gs_pointers_qualifies_the_element_pointers() {
        // struct c __seg_gs *a[4]
        let mut t = CType::Array(Box::new(gs_ptr_plain()), Some(4));
        apply_declaration_address_space(&mut t, AddressSpace::SegGs);
        assert_eq!(innermost_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn pointer_to_array_of_gs_pointers_keeps_outer_pointer_generic() {
        // struct c __seg_gs *(*pa)[4]
        let mut t = CType::Pointer(
            Box::new(CType::Array(Box::new(gs_ptr_plain()), Some(4))),
            AddressSpace::Default,
        );
        apply_declaration_address_space(&mut t, AddressSpace::SegGs);
        assert_eq!(outer_space(&t), Some(AddressSpace::Default));
        assert_eq!(innermost_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn default_space_is_a_no_op() {
        let mut t = CType::Pointer(Box::new(gs_ptr_plain()), AddressSpace::Default);
        let before = t.clone();
        apply_declaration_address_space(&mut t, AddressSpace::Default);
        assert_eq!(t, before);
    }

    #[test]
    fn plain_object_lives_in_the_declared_space() {
        assert_eq!(
            declared_object_space(AddressSpace::SegGs, &CType::Int, false),
            AddressSpace::SegGs
        );
        assert_eq!(
            declared_object_space(
                AddressSpace::SegGs,
                &CType::Array(Box::new(CType::Int), Some(4)),
                false
            ),
            AddressSpace::SegGs
        );
        assert_eq!(
            declared_object_space(AddressSpace::SegGs, &base(), false),
            AddressSpace::SegGs
        );
    }

    #[test]
    fn whole_type_qualifier_puts_the_pointer_object_in_the_space() {
        // __seg_gs __typeof__(struct c *) p;  -> p itself is %gs (kernel per-CPU pointer)
        assert_eq!(
            declared_object_space(AddressSpace::SegGs, &gs_ptr_plain(), true),
            AddressSpace::SegGs
        );
        let arr = CType::Array(Box::new(gs_ptr_plain()), Some(4));
        assert_eq!(
            declared_object_space(AddressSpace::SegGs, &arr, true),
            AddressSpace::SegGs
        );
    }

    #[test]
    fn pointer_object_is_generic_even_when_pointee_is_gs() {
        assert_eq!(
            declared_object_space(AddressSpace::SegGs, &gs_ptr_plain(), false),
            AddressSpace::Default
        );
        let arr = CType::Array(Box::new(gs_ptr_plain()), Some(4));
        assert_eq!(
            declared_object_space(AddressSpace::SegGs, &arr, false),
            AddressSpace::Default
        );
        assert_eq!(
            declared_object_space(AddressSpace::Default, &base(), false),
            AddressSpace::Default
        );
    }

    #[test]
    fn typedef_base_qualifies_whole_object_only_without_declarator_pointer() {
        // typedef long L;  L __seg_gs x;   -> x itself is %gs
        let td = TypeSpecifier::TypedefName("L".to_string());
        assert!(named_space_qualifies_whole_type(&td, &[]));
        // typedef long L;  L __seg_gs arr[4];  -> arr is %gs
        assert!(named_space_qualifies_whole_type(
            &td,
            &[DerivedDeclarator::Array(None)]
        ));
        // typedef long L;  L __seg_gs *p;  -> p is generic, *p is %gs (D7)
        assert!(!named_space_qualifies_whole_type(
            &td,
            &[DerivedDeclarator::Pointer]
        ));
        // typedef long L;  L __seg_gs (*fp)(void);  -> fp is generic (D7)
        assert!(!named_space_qualifies_whole_type(
            &td,
            &[DerivedDeclarator::FunctionPointer(Vec::new(), false)]
        ));
        // A plain base type never qualifies the whole object by itself.
        assert!(!named_space_qualifies_whole_type(&TypeSpecifier::Int, &[]));
    }

    #[test]
    fn typedef_pointer_base_with_declarator_pointer_keeps_object_generic() {
        // typedef struct c *P;  P __seg_gs *pp;  -> pp is generic memory.
        let ct = CType::Pointer(Box::new(gs_ptr_plain()), AddressSpace::Default);
        let td = TypeSpecifier::TypedefName("P".to_string());
        let whole = named_space_qualifies_whole_type(&td, &[DerivedDeclarator::Pointer]);
        assert!(!whole);
        assert_eq!(
            declared_object_space(AddressSpace::SegGs, &ct, whole),
            AddressSpace::Default
        );
    }
}
