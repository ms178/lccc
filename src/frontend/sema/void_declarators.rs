//! The constraints C places on `void` in declarators, with GCC 16.2's
//! diagnostics (wording measured on `gcc-16 -std=gnu17 -c`):
//!
//! * An array element type must be complete (C11 6.7.6.2p1), and `void` never
//!   is: `void a[3]`, `typedef void V[2]`, `void (*p)[3]`, a `void` flexible
//!   member, a parameter `void a[2]` (before its adjustment to a pointer) and
//!   the type names in `sizeof(void[2])` or a cast are all rejected --
//!   "declaration of 'a' as array of voids", or "... of type name ...".
//! * An object with storage and a member must have a complete object type
//!   (C11 6.7p7, 6.7.2.1p3): `void x;` at block scope, `static void x;`,
//!   `struct { void v; }` -- "variable or field 'x' declared void".  A
//!   file-scope `void x;` without `static` is a tentative definition, which
//!   GCC diagnoses when the translation unit ends as "storage size of 'x'
//!   isn't known"; with an initializer it is "variable 'x' has initializer
//!   but incomplete type".  `extern void v;` is valid (its address may be
//!   taken).
//! * `void` as a parameter type means "no parameters" only when it is the
//!   sole, unnamed, unqualified parameter, without `...` (C11 6.7.6.3p10;
//!   GCC's `get_parm_info`): "'void' must be the only parameter and
//!   unnamed", "'void' must be the only parameter" (`f(void, ...)`), "'void'
//!   as only parameter may not be qualified" (`f(const void)`).  A K&R
//!   parameter declared `void` is "parameter 'x' declared with void type".
//! * A compound literal's type name must be a complete object type (C11
//!   6.5.2.5p1): `(void){0}` is "invalid initializer", `(void){}` "invalid
//!   use of void expression".
//!
//! lccc accepted all of these (20 of 23 first-measured shapes) and gave the
//! objects a layout -- `void a[3]` became 3 bytes of `.bss`,
//! `sizeof(void[2])` 2.
//!
//! The check reads the declarator syntax and never builds its `CType`:
//! resolving a type specifier is not free of effects here (an anonymous
//! struct definition is assigned a fresh key each time it is resolved, and
//! array bounds are diagnosed while they are evaluated), and sema has
//! already resolved each declarator once.  Only typedef names are looked up,
//! which is pure.  Without a function derivation, a declarator's derivations
//! apply to the base type in order (`build_full_ctype_with_base`: `void
//! *a[3]` is `[Pointer, Array]`, `void (*p)[3]` is `[Array, Pointer]`), so an
//! array's element is the base type exactly when the first derivation is an
//! array.  With one, the function applies first -- leading arrays wrap the
//! result (`void (*fp[3])(int)` is `[Array, Pointer, FunctionPointer]`) --
//! so the base is a return type and never an array element.
use crate::common::types::CType;
use crate::frontend::parser::ast::{DerivedDeclarator, TypeSpecifier};

/// What the declarator declares, as far as `void` is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VoidRole {
    /// An object that gets storage here: block scope without `extern`, or
    /// `static` at file scope.
    Object,
    /// A file-scope object with neither `static` nor `extern`: a tentative
    /// definition, or a definition when it has an initializer.
    External { initialized: bool },
    /// A struct or union member.
    Member,
    /// A prototype parameter: `sole` when it is the only one in its list,
    /// `variadic` when `...` follows, `qualified` when `const`/`volatile`.
    Parameter {
        sole: bool,
        variadic: bool,
        qualified: bool,
    },
    /// A parameter declared in a K&R declaration list.
    KrParameter,
    /// The type name of a compound literal, which must be a complete object
    /// type (C11 6.5.2.5p1); `empty` for `(T){}`.
    CompoundLiteral { empty: bool },
    /// Anything that needs no object type: a typedef, an `extern` object, a
    /// function declarator, a type name.
    Other,
}

/// Looks a typedef name up (already resolved by sema).
pub(super) type TypedefLookup<'a> = &'a dyn Fn(&str) -> Option<CType>;

/// Is the type specifier `void` (directly, through a typedef, or through
/// `typeof(type-name)`)?
fn spec_is_void(ts: &TypeSpecifier, typedefs: TypedefLookup<'_>) -> bool {
    match ts {
        TypeSpecifier::Void => true,
        TypeSpecifier::TypedefName(name) => matches!(typedefs(name), Some(CType::Void)),
        TypeSpecifier::TypeofType(inner) => spec_is_void(inner, typedefs),
        _ => false,
    }
}

/// Who an array of `void` formed inside a type specifier belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArrayOfVoidIn {
    /// The declarator being checked (`void[2]` in a type name, a member's
    /// or parameter's own array, `void (*)[2]` in a cast).
    Declarator,
    /// The type name inside `typeof(...)`, which GCC reports on its own.
    TypeofName,
}

/// Does the type specifier itself form an array of `void`?
fn spec_array_of_void(ts: &TypeSpecifier, typedefs: TypedefLookup<'_>) -> Option<ArrayOfVoidIn> {
    match ts {
        TypeSpecifier::Array(elem, _) if spec_is_void(elem, typedefs) => {
            Some(ArrayOfVoidIn::Declarator)
        }
        TypeSpecifier::Array(inner, _) | TypeSpecifier::Pointer(inner, _) => {
            spec_array_of_void(inner, typedefs)
        }
        TypeSpecifier::FunctionPointer(ret, ..) | TypeSpecifier::BareFunction(ret, ..) => {
            spec_array_of_void(ret, typedefs)
        }
        TypeSpecifier::TypeofType(inner) => {
            spec_array_of_void(inner, typedefs).map(|_| ArrayOfVoidIn::TypeofName)
        }
        _ => None,
    }
}

/// The diagnostic GCC 16.2 gives for the declarator `ts derived` named
/// `name` (`None` for a type name or an unnamed parameter), or `None` when
/// it is valid.
pub(super) fn void_declarator_error(
    ts: &TypeSpecifier,
    derived: &[DerivedDeclarator],
    name: Option<&str>,
    role: VoidRole,
    typedefs: TypedefLookup<'_>,
) -> Option<String> {
    let base_void = spec_is_void(ts, typedefs);
    let applies_function = derived.iter().any(|d| {
        matches!(
            d,
            DerivedDeclarator::Function(..) | DerivedDeclarator::FunctionPointer(..)
        )
    });
    let array_of_void = spec_array_of_void(ts, typedefs).or_else(|| {
        (base_void
            && !applies_function
            && matches!(derived.first(), Some(DerivedDeclarator::Array(_))))
        .then_some(ArrayOfVoidIn::Declarator)
    });
    if let Some(owner) = array_of_void {
        return Some(match (owner, name) {
            (ArrayOfVoidIn::Declarator, Some(n)) => {
                format!("declaration of '{n}' as array of voids")
            }
            _ => "declaration of type name as array of voids".to_string(),
        });
    }
    if !base_void || !derived.is_empty() {
        return None;
    }
    let name = name.unwrap_or_default();
    match role {
        VoidRole::Object | VoidRole::Member => {
            Some(format!("variable or field '{name}' declared void"))
        }
        VoidRole::External { initialized: false } => {
            Some(format!("storage size of '{name}' isn't known"))
        }
        VoidRole::External { initialized: true } => Some(format!(
            "variable '{name}' has initializer but incomplete type"
        )),
        VoidRole::Parameter { sole, .. } if !sole || !name.is_empty() => {
            Some("'void' must be the only parameter and unnamed".to_string())
        }
        VoidRole::Parameter {
            qualified: true, ..
        } => Some("'void' as only parameter may not be qualified".to_string()),
        VoidRole::Parameter { variadic: true, .. } => {
            Some("'void' must be the only parameter".to_string())
        }
        VoidRole::KrParameter => Some(format!("parameter '{name}' declared with void type")),
        VoidRole::CompoundLiteral { empty: false } => Some("invalid initializer".to_string()),
        VoidRole::CompoundLiteral { empty: true } => {
            Some("invalid use of void expression".to_string())
        }
        VoidRole::Parameter { .. } | VoidRole::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;

    fn no_typedefs(_: &str) -> Option<CType> {
        None
    }

    fn void_typedef(name: &str) -> Option<CType> {
        (name == "V").then_some(CType::Void)
    }

    fn err(ts: &TypeSpecifier, derived: &[DerivedDeclarator], role: VoidRole) -> Option<String> {
        void_declarator_error(ts, derived, Some("a"), role, &void_typedef)
    }

    fn arr() -> DerivedDeclarator {
        DerivedDeclarator::Array(None)
    }

    fn ptr() -> DerivedDeclarator {
        DerivedDeclarator::Pointer(AddressSpace::Default)
    }

    fn func() -> DerivedDeclarator {
        DerivedDeclarator::Function(Vec::new(), false, true)
    }

    const ALL_ROLES: [VoidRole; 10] = [
        VoidRole::Object,
        VoidRole::External { initialized: false },
        VoidRole::External { initialized: true },
        VoidRole::Member,
        VoidRole::Parameter {
            sole: false,
            variadic: false,
            qualified: false,
        },
        VoidRole::Parameter {
            sole: true,
            variadic: true,
            qualified: true,
        },
        VoidRole::KrParameter,
        VoidRole::CompoundLiteral { empty: false },
        VoidRole::CompoundLiteral { empty: true },
        VoidRole::Other,
    ];

    #[test]
    fn arrays_of_void_are_rejected_whatever_the_role() {
        let msg = Some("declaration of 'a' as array of voids".to_string());
        let void_name = TypeSpecifier::TypedefName("V".to_string());
        let shapes: [(&TypeSpecifier, Vec<DerivedDeclarator>); 5] = [
            (&TypeSpecifier::Void, vec![arr()]),               // void a[];
            (&TypeSpecifier::Void, vec![arr(), arr()]),        // void a[2][2];
            (&TypeSpecifier::Void, vec![arr(), ptr()]),        // void (*a)[3];
            (&TypeSpecifier::Void, vec![arr(), ptr(), ptr()]), // void (**a)[3];
            (&void_name, vec![arr()]),                         // V a[3];
        ];
        for (ts, derived) in &shapes {
            for role in ALL_ROLES {
                assert_eq!(err(ts, derived, role), msg, "{ts:?} {derived:?} {role:?}");
            }
        }
        let type_name = TypeSpecifier::Array(Box::new(TypeSpecifier::Void), None);
        assert_eq!(
            void_declarator_error(&type_name, &[], None, VoidRole::Other, &no_typedefs).as_deref(),
            Some("declaration of type name as array of voids")
        );
        assert_eq!(
            err(&type_name, &[], VoidRole::Member),
            msg,
            "struct {{ void a[]; }}"
        );
        let typeof_name = TypeSpecifier::TypeofType(Box::new(type_name.clone()));
        let cast = TypeSpecifier::Pointer(Box::new(type_name), AddressSpace::Default);
        assert!(err(&cast, &[], VoidRole::Other).is_some(), "(void (*)[2])0");
        assert_eq!(
            err(&typeof_name, &[ptr()], VoidRole::Object).as_deref(),
            Some("declaration of type name as array of voids"),
            "typeof(void[2]) *a;"
        );
    }

    #[test]
    fn pointers_and_functions_returning_void_are_fine() {
        let fptr = DerivedDeclarator::FunctionPointer(Vec::new(), false, true);
        for derived in [
            vec![ptr()],                      // void *a;
            vec![ptr(), arr()],               // void *a[3];
            vec![func()],                     // void a(void);
            vec![ptr(), func()],              // void *a(void);
            vec![arr(), ptr(), fptr.clone()], // void (*a[3])(int);
            vec![arr(), fptr],                // the same, as the parser may emit it
        ] {
            for role in ALL_ROLES {
                assert_eq!(
                    err(&TypeSpecifier::Void, &derived, role),
                    None,
                    "{derived:?}"
                );
            }
        }
        for role in ALL_ROLES {
            assert_eq!(err(&TypeSpecifier::Int, &[arr()], role), None);
            assert_eq!(err(&TypeSpecifier::Int, &[], role), None);
        }
    }

    #[test]
    fn void_objects_need_storage_to_be_wrong() {
        let v =
            |role| void_declarator_error(&TypeSpecifier::Void, &[], Some("x"), role, &no_typedefs);
        let declared_void = Some("variable or field 'x' declared void".to_string());
        assert_eq!(v(VoidRole::Object), declared_void);
        assert_eq!(v(VoidRole::Member), declared_void);
        assert_eq!(
            v(VoidRole::External { initialized: false }).as_deref(),
            Some("storage size of 'x' isn't known")
        );
        assert_eq!(
            v(VoidRole::External { initialized: true }).as_deref(),
            Some("variable 'x' has initializer but incomplete type")
        );
        assert_eq!(
            v(VoidRole::KrParameter).as_deref(),
            Some("parameter 'x' declared with void type")
        );
        assert_eq!(
            v(VoidRole::CompoundLiteral { empty: false }).as_deref(),
            Some("invalid initializer"),
            "(void){{0}}"
        );
        assert_eq!(
            v(VoidRole::CompoundLiteral { empty: true }).as_deref(),
            Some("invalid use of void expression"),
            "(void){{}}"
        );
        assert_eq!(v(VoidRole::Other), None, "extern void x; / typedef void x;");
        let typed = TypeSpecifier::TypedefName("V".to_string());
        assert!(
            err(&typed, &[], VoidRole::Member).is_some(),
            "struct {{ V v; }}"
        );
    }

    #[test]
    fn void_parameter_only_alone_unnamed_unqualified_and_without_ellipsis() {
        let only = Some("'void' must be the only parameter and unnamed".to_string());
        let p = |name, sole, variadic, qualified| {
            let role = VoidRole::Parameter {
                sole,
                variadic,
                qualified,
            };
            void_declarator_error(&TypeSpecifier::Void, &[], name, role, &no_typedefs)
        };
        assert_eq!(
            p(None, true, false, false),
            None,
            "f(V) with a void typedef"
        );
        assert_eq!(p(Some("x"), true, false, false), only, "f(void x)");
        assert_eq!(p(None, false, false, false), only, "f(int, void)");
        assert_eq!(p(Some("x"), true, true, true), only, "f(const void x, ...)");
        assert_eq!(
            p(None, true, true, false).as_deref(),
            Some("'void' must be the only parameter"),
            "f(void, ...)"
        );
        assert_eq!(
            p(None, true, true, true).as_deref(),
            Some("'void' as only parameter may not be qualified"),
            "f(const void, ...): the qualifier is reported first"
        );
    }
}
