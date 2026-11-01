// Declarator parsing: handles the C declarator syntax (the part after the type
// specifier that defines the name and type modifiers like pointers, arrays,
// and function parameters).
//
// C declarators follow an "inside-out" rule: int (*fp)(int) means fp is a
// pointer to a function returning int, read from the name outward. This module
// handles the recursive parsing needed for this grammar.

use super::ast::*;
use super::parse::{ModeKind, Parser};
use crate::common::source::Span;
use crate::common::types::AddressSpace;
use crate::frontend::lexer::token::TokenKind;

/// One parenthesized/top-level grammar production of an abstract declarator:
/// `stars  ( inner_level )?  suffixes*`.
///
/// Walking from the outermost level inward (`level -> level.core -> ...`) visits
/// type modifiers from the base type specifier inward toward the omitted identifier,
/// matching C11 6.7.7's inside-out abstract-declarator semantics at arbitrary
/// nesting depth while preserving per-star address-space and CVR qualifiers.
pub(super) struct AbstractDeclaratorLevel {
    /// Leading `*` (or `^`) qualifiers in left-to-right written order: `(address_space, cvr_bits)`.
    pub stars: Vec<(AddressSpace, u8)>,
    /// Optional grouped `( abstract-declarator )` core.
    pub core: Option<Box<AbstractDeclaratorLevel>>,
    /// Direct postfix `[size]` and `(params)` suffixes in left-to-right written order.
    pub suffixes: Vec<AbstractDeclSuffix>,
}

pub(super) enum AbstractDeclSuffix {
    Array(Option<Box<Expr>>),
    Function(Vec<ParamDecl>, bool, bool),
}

impl AbstractDeclaratorLevel {
    pub(super) fn is_empty(&self) -> bool {
        self.stars.is_empty() && self.core.is_none() && self.suffixes.is_empty()
    }

    /// Fold this abstract declarator tree onto `(base, base_space)`, optionally
    /// recording per-layer CVR qualifier bitmasks for `__builtin_types_compatible_p`.
    pub(super) fn apply_to(
        &self,
        mut ty: TypeSpecifier,
        mut space: AddressSpace,
        mut quals: Option<&mut Vec<u8>>,
    ) -> TypeSpecifier {
        let mut cur = Some(self);
        while let Some(level) = cur {
            for &(star_space, star_cvr) in &level.stars {
                ty = match ty {
                    TypeSpecifier::BareFunction(ret, params, variadic, has_prototype)
                        if space == AddressSpace::Default =>
                    {
                        TypeSpecifier::FunctionPointer(ret, params, variadic, has_prototype)
                    }
                    other => TypeSpecifier::Pointer(Box::new(other), space),
                };
                if let Some(q) = quals.as_deref_mut() {
                    q.push(star_cvr);
                }
                space = star_space;
            }
            for suffix in level.suffixes.iter().rev() {
                match suffix {
                    AbstractDeclSuffix::Array(dim) => {
                        ty = TypeSpecifier::Array(Box::new(ty), dim.clone());
                        if let Some(q) = quals.as_deref_mut() {
                            q.push(0);
                        }
                    }
                    AbstractDeclSuffix::Function(params, variadic, has_prototype) => {
                        ty = TypeSpecifier::BareFunction(
                            Box::new(ty),
                            params.clone(),
                            *variadic,
                            *has_prototype,
                        );
                        if let Some(q) = quals.as_deref_mut() {
                            q.clear();
                            q.push(0);
                        }
                        space = AddressSpace::Default;
                    }
                }
            }
            cur = level.core.as_deref();
        }
        ty
    }
}

impl Parser {
    pub(super) fn parse_declarator(&mut self) -> (Option<String>, Vec<DerivedDeclarator>) {
        let (name, derived, _, _, _, _) = self.parse_declarator_with_attrs();
        (name, derived)
    }

    /// Parse a declarator, also returning attribute info:
    /// (name, derived, mode_kind, has_common, aligned_value, is_packed)
    pub(super) fn parse_declarator_with_attrs(
        &mut self,
    ) -> (
        Option<String>,
        Vec<DerivedDeclarator>,
        Option<ModeKind>,
        bool,
        Option<usize>,
        bool,
    ) {
        let mut derived = Vec::with_capacity(4);

        let mut pre_aligned: Option<usize> = None;
        let mut is_packed = false;
        // `_spec` keeps the placement records for the declaration's
        // `-Wattributes` verdict; the clearing variant is for attribute
        // lists that belong to something other than a declaration.
        let (pre_packed, pre_align, _, _, _) = self.parse_gcc_attributes_spec();
        is_packed = is_packed || pre_packed;
        if let Some(a) = pre_align {
            pre_aligned = Some(pre_aligned.map_or(a, |prev: usize| prev.max(a)));
        }

        // Parse pointer(s) with optional qualifiers and attributes.
        //
        // POINTER_CONST must describe the DECLARED OBJECT's own
        // qualification -- the star immediately left of the identifier --
        // and never an inner pointer level.  Stars are consumed
        // left-to-right, so every iteration first CLEARS the flag; after
        // the loop it holds the last star's qualifier run, which is
        // exactly the outermost level:
        //
        //     char *const p       -> true   (the one star is outermost)
        //     short *const *gp    -> false  (gp stays writable; the const
        //                                     qualifies its POINTEE)
        //     int *const a, *b;   -> a: true, b: false (no cross-declarator
        //                                     leakage either)
        //
        // The previous accumulate-across-stars behavior classified the
        // mutable `gp` above as read-only, so `classify_global` routed it
        // to `.data.rel.ro`; the dynamic loader's RELRO mprotect made the
        // page read-only and the first run-time store through/to `gp`
        // took SIGSEGV (found by Csmith seed 20260928 vs GCC; minimal
        // repro tests/regression/pointer_const_multi_level_relro.c).
        // Bounded: see POINTER_DEPTH_BUDGET. The span is taken BEFORE
        // consuming so the diagnostic points at the offending `*`.
        // `volatile` written before the first `*` belongs to the specifier
        // (not one an earlier declarator's star left in the flag).
        let spec_volatile = self.attrs.parsing_volatile() && !self.attrs.volatile_from_stars;
        // Likewise `restrict` (legal there only on a typedef'd pointer base).
        let spec_restrict = self.attrs.parsing_restrict();
        let typedef_use = self.attrs.parsing_typedef_use();
        let typedef_object_volatile = self.attrs.parsing_typedef_object_volatile();
        // `_Atomic` in the specifiers, and `restrict`/`_Atomic` after the
        // most recent `*`, by the same rule.
        let spec_atomic = self.attrs.parsing_atomic();
        let (object_space, last_star_volatile, last_star_restrict, last_star_atomic) =
            self.parse_pointer_prefix(&mut derived, spec_restrict, spec_atomic);

        // Parse the direct-declarator part
        // Space of the object declared by a parenthesised inner declarator
        // (`long (*__seg_gs ap)[4]`). The inner call records it in
        // `declarator_object_space`, which the outer call must not overwrite.
        let mut inner_object_space = AddressSpace::Default;
        // The inner declarator's object qualifiers: when it has a `*`, the
        // declared object is that pointer (`int (*_Atomic p)[3]`).
        let mut inner_object_quals = (false, false);
        let (name, inner_derived) = if let TokenKind::Identifier(n) = self.peek() {
            let n = n.clone();
            // A *definition* reports `-Wstrict-prototypes` at its name, a
            // declaration at its start (gcc-16, measured). Parameter lists
            // parsed below would overwrite this, so they save and restore it.
            self.attrs.last_decl_name_span = Some(self.peek_span());
            self.advance();
            (Some(n), Vec::new())
        } else if matches!(self.peek(), TokenKind::LParen) && self.is_paren_declarator() {
            let save = self.pos;
            self.advance(); // consume '('
            let (inner_name, inner_derived) = self.parse_declarator();
            inner_object_space = self.attrs.declarator_object_space;
            inner_object_quals = (
                self.attrs.declarator_object_restrict,
                self.attrs.declarator_object_atomic,
            );
            if !self.consume_if(&TokenKind::RParen) {
                self.pos = save;
                inner_object_space = AddressSpace::Default;
                inner_object_quals = (false, false);
                (None, Vec::new())
            } else {
                (inner_name, inner_derived)
            }
        } else {
            (None, Vec::new())
        };

        // Parse outer suffixes: array dimensions and function params.  A
        // parenthesised inner declarator without `*` (`(g(void))`) ends in the
        // suffix the outer ones wrap directly.
        let inner_derived_has_pointer = inner_derived
            .iter()
            .any(|d| matches!(d, DerivedDeclarator::Pointer(..)));
        let mut outer_suffixes = Vec::with_capacity(4);
        let mut innermost_suffix_is_function = if inner_derived
            .iter()
            .any(|d| matches!(d, DerivedDeclarator::Pointer(..)))
        {
            None
        } else {
            inner_derived.last().and_then(|d| match d {
                DerivedDeclarator::Function(..) => Some(true),
                DerivedDeclarator::Array(_) => Some(false),
                _ => None,
            })
        };
        loop {
            match self.peek() {
                TokenKind::LBracket if !self.is_c23_attr_start() => {
                    let open_bracket = self.peek_span();
                    self.check_suffix_composition(
                        innermost_suffix_is_function,
                        false,
                        name.as_deref(),
                        open_bracket,
                    );
                    innermost_suffix_is_function = Some(false);
                    self.advance();
                    // C99 array parameter declarators can have qualifiers and 'static':
                    // [static restrict const 10], [restrict n], [const], etc.
                    // Skip all qualifiers and 'static' before the size expression.
                    self.skip_array_qualifiers();
                    let size = if matches!(self.peek(), TokenKind::RBracket) {
                        None
                    } else if matches!(self.peek(), TokenKind::Star)
                        && self.pos + 1 < self.tokens.len()
                        && matches!(self.tokens[self.pos + 1].kind, TokenKind::RBracket)
                    {
                        // C99 VLA star syntax: [*] or [const *] means unspecified VLA size
                        self.advance(); // consume '*'
                        None
                    } else {
                        Some(Box::new(self.parse_expr()))
                    };
                    self.expect_closing(&TokenKind::RBracket, open_bracket);
                    outer_suffixes.push(DerivedDeclarator::Array(size));
                }
                TokenKind::LParen => {
                    let open_paren = self.peek_span();
                    self.check_suffix_composition(
                        innermost_suffix_is_function,
                        true,
                        name.as_deref(),
                        open_paren,
                    );
                    innermost_suffix_is_function = Some(true);
                    let saved_name_span = self.attrs.last_decl_name_span;
                    let (params, variadic, has_prototype) = self.parse_param_list();
                    self.attrs.last_decl_name_span = saved_name_span;
                    // `-Wstrict-prototypes`: `int f();` declares a function but
                    // not a prototype. GCC's verdict belongs to the whole
                    // declaration (it also fires for an unprototyped type
                    // nested in a parameter) and is reported at the
                    // declaration's location, so the fact is recorded here and
                    // reported by the declaration funnel. C23 makes `()` a
                    // prototype, which is why the flag is only set for the
                    // older dialects (measured: `-std=gnu17` warns,
                    // `-std=c23` does not).
                    let _ = has_prototype;
                    outer_suffixes.push(DerivedDeclarator::Function(
                        params,
                        variadic,
                        has_prototype,
                    ));
                }
                _ => break,
            }
        }

        // Combine using inside-out rule
        let combined = self.combine_declarator_parts(derived, inner_derived, outer_suffixes);

        let (post_packed, post_aligned, mode_kind, has_common, _) =
            self.parse_gcc_attributes_spec();
        is_packed = is_packed || post_packed;
        let aligned = match (pre_aligned, post_aligned) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };

        // The outermost stars name the object when present; otherwise the
        // parenthesised declarator's object space stands.
        // The object is volatile by its last declarator part only: a pointer
        // level's qualifier, or the specifier's when nothing is derived. An
        // array or function object is not itself volatile.
        // A typedef'd volatile pointer declared as an array (`VIP arr[2]`, with
        // `typedef int *volatile VIP`) makes every element a volatile object.
        // For an array the base flag is the element qualifier, so it carries
        // the typedef's object qualifier here. Pointer derivations (`VIP *pp`)
        // are not widened: their pointee qualifier cannot be expressed by this
        // single bit (see `pointee_expr_is_volatile`).
        let typedef_element_volatile = typedef_use
            && typedef_object_volatile
            && !combined.is_empty()
            && combined
                .iter()
                .all(|d| matches!(d, DerivedDeclarator::Array(_)));
        self.attrs.declarator_base_volatile = spec_volatile || typedef_element_volatile;
        self.attrs.declarator_base_restrict = spec_restrict;
        // Locals and globals read the declaration-wide flag (see
        // `d.set_volatile(attrs.parsing_volatile())` in declarations.rs), so the
        // element qualifier must also be carried there.
        if typedef_element_volatile {
            self.attrs.set_volatile(true);
        }
        self.attrs.declarator_object_volatile = match combined.last() {
            // A typedef name's object qualifier is the typedef's own; the base
            // qualifier (`spec_volatile` on a typedef use) is the pointee/element.
            None if typedef_use => typedef_object_volatile,
            None => spec_volatile,
            Some(DerivedDeclarator::Pointer(..)) => last_star_volatile.unwrap_or(false),
            Some(_) => false,
        };
        let inner_has_pointer = inner_derived_has_pointer;
        let all_arrays = !combined.is_empty()
            && combined
                .iter()
                .all(|d| matches!(d, DerivedDeclarator::Array(_)));
        (
            self.attrs.declarator_object_restrict,
            self.attrs.declarator_object_atomic,
        ) = if inner_has_pointer {
            inner_object_quals
        } else {
            match combined.last() {
                None => (false, spec_atomic),
                Some(DerivedDeclarator::Pointer(..)) => (last_star_restrict, last_star_atomic),
                Some(_) if all_arrays => (false, spec_atomic),
                Some(_) => (false, false),
            }
        };
        self.attrs.declarator_object_space = if object_space == AddressSpace::Default {
            inner_object_space
        } else {
            object_space
        };
        (name, combined, mode_kind, has_common, aligned, is_packed)
    }

    /// Consume the leading `*` levels of a declarator and return the declared
    /// object's address space plus the qualifier run of the last (outermost)
    /// star.  Each level keeps its own CVR/address-space bits in `derived`;
    /// only the last level describes the object itself.  The caller supplies
    /// the specifier-level `restrict`/`_Atomic` flags to restore after each
    /// star, so a pointee qualifier cannot leak into the next level.
    fn parse_pointer_prefix(
        &mut self,
        derived: &mut Vec<DerivedDeclarator>,
        spec_restrict: bool,
        spec_atomic: bool,
    ) -> (AddressSpace, Option<bool>, bool, bool) {
        let mut star_depth = 0u32;
        // Space written after the most recent `*`. After the loop it is the
        // space after the LAST `*`, the one adjacent to the name: that pointer
        // is the declared object (`long *__seg_gs p` puts `p` in %gs), while a
        // qualifier on an earlier `*` belongs to the pointee (`long *__seg_gs *p`).
        let mut object_space = AddressSpace::Default;
        // `volatile` written after the most recent `*` (the last one is the
        // object's own qualifier).
        let mut last_star_volatile: Option<bool> = None;
        let mut last_star_restrict = false;
        let mut last_star_atomic = false;
        loop {
            let star_span = self.peek_span();
            if !self.consume_if(&TokenKind::Star) {
                break;
            }
            star_depth += 1;
            if !self.enter_pointer_level(star_depth, star_span) {
                break;
            }
            // Only the outermost level's qualifiers describe the object.
            self.attrs.set_pointer_const(false);
            // The base-specifier space already sits in the slot; keep it, and
            // collect only the qualifier written after this `*`.
            let base_space =
                std::mem::replace(&mut self.attrs.parsing_address_space, AddressSpace::Default);
            // Isolate this star's `volatile` so the last star's can be read
            // back; the declaration-wide flag keeps the union, as before.
            let outer_volatile = self.attrs.parsing_volatile();
            self.attrs.set_volatile(false);
            // Likewise `restrict`/`_Atomic`: the specifiers' `_Atomic` is the
            // pointee's (`_Atomic int *p`), not this star's.
            self.attrs.set_restrict(false);
            self.attrs.set_atomic(false);
            self.skip_cv_qualifiers();
            let star_volatile = self.attrs.parsing_volatile();
            self.attrs.set_volatile(outer_volatile || star_volatile);
            if star_volatile && !outer_volatile {
                self.attrs.volatile_from_stars = true;
            }
            last_star_volatile = Some(star_volatile);
            last_star_restrict = self.attrs.parsing_restrict();
            last_star_atomic = self.attrs.parsing_atomic();
            self.attrs.set_restrict(spec_restrict);
            self.attrs.set_atomic(spec_atomic);
            let star_space = std::mem::replace(&mut self.attrs.parsing_address_space, base_space);
            // Each star keeps its own qualifiers (see `DerivedDeclarator::Pointer`):
            // the space and the CVR run written after it. A middle level's
            // `const` (`int *const *p`) is invisible in `CType`, so it has to
            // ride on the declarator or the read-only check cannot see it.
            let star_cvr = u8::from(self.attrs.parsing_pointer_const())
                | (u8::from(star_volatile) << 1)
                | (u8::from(last_star_restrict) << 2);
            // The last star's is also the declared object's space.
            derived.push(DerivedDeclarator::Pointer(star_space, star_cvr));
            object_space = star_space;
            self.skip_gcc_extensions();
        }
        (
            object_space,
            last_star_volatile,
            last_star_restrict,
            last_star_atomic,
        )
    }

    /// Determine if a '(' starts a parenthesized declarator vs. a parameter list.
    pub(super) fn is_paren_declarator(&self) -> bool {
        let mut idx = self.pos + 1;
        while idx + 1 < self.tokens.len()
            && matches!(self.tokens[idx].kind, TokenKind::LBracket)
            && matches!(self.tokens[idx + 1].kind, TokenKind::LBracket)
        {
            idx += 2;
            let mut depth = 1usize;
            while idx < self.tokens.len() {
                match self.tokens[idx].kind {
                    TokenKind::LBracket => depth += 1,
                    TokenKind::RBracket => {
                        depth -= 1;
                        if depth == 0 {
                            idx += 1;
                            if idx < self.tokens.len()
                                && matches!(self.tokens[idx].kind, TokenKind::RBracket)
                            {
                                idx += 1;
                            }
                            break;
                        }
                    }
                    _ => {}
                }
                idx += 1;
            }
        }
        if idx >= self.tokens.len() {
            return false;
        }
        match &self.tokens[idx].kind {
            TokenKind::Star | TokenKind::Caret => true,
            TokenKind::LParen | TokenKind::LBracket => true,
            TokenKind::Attribute | TokenKind::Extension => true,
            TokenKind::Identifier(name) => {
                // Typedef name -> parameter list; regular name -> declarator
                !self.typedefs.contains(name) || self.shadowed_typedefs.contains(name)
            }
            TokenKind::RParen | TokenKind::Ellipsis => false,
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
            | TokenKind::Noreturn
            | TokenKind::Restrict
            | TokenKind::Complex
            | TokenKind::Atomic
            | TokenKind::Auto
            | TokenKind::Alignas
            | TokenKind::Builtin => false,
            _ => false,
        }
    }

    /// Combine declarator parts using C's inside-out rule.
    ///
    /// For `int (*fp)(int)`:
    ///   outer_pointers: []
    ///   inner_derived: [Pointer]
    ///   outer_suffixes: [Function([int])]
    ///   Result: [Pointer, FunctionPointer([int])]
    /// C11 6.7.6.3p1 and 6.7.6.2p1: a function cannot return an array or a
    /// function, and an array element cannot be a function.  Within one
    /// direct declarator each suffix derives the type of the one before it
    /// (`g(void)[2]`: `g` is a function returning an array), so the pair of
    /// adjacent suffixes decides it; `(*f(void))[3]` is valid because the
    /// `*` separates them.  `prev_is_function` describes the suffix the new
    /// one follows (`None`: there is none, or a `*` intervenes).  Wording is
    /// GCC 16.2's.
    fn check_suffix_composition(
        &mut self,
        prev_is_function: Option<bool>,
        next_is_function: bool,
        name: Option<&str>,
        span: Span,
    ) {
        let subject = name.map_or_else(|| "type name".to_string(), |n| format!("'{n}'"));
        let message = match (prev_is_function, next_is_function) {
            (Some(true), false) => format!("{subject} declared as function returning an array"),
            (Some(true), true) => format!("{subject} declared as function returning a function"),
            (Some(false), true) => format!("declaration of {subject} as array of functions"),
            (Some(false), false) | (None, _) => return,
        };
        self.emit_error(message, span);
    }

    fn combine_declarator_parts(
        &self,
        mut outer_pointers: Vec<DerivedDeclarator>,
        inner_derived: Vec<DerivedDeclarator>,
        outer_suffixes: Vec<DerivedDeclarator>,
    ) -> Vec<DerivedDeclarator> {
        if inner_derived.is_empty() && outer_suffixes.is_empty() {
            return outer_pointers;
        }

        if inner_derived.is_empty() {
            outer_pointers.extend(outer_suffixes);
            return outer_pointers;
        }

        // Check for function pointer: inner has Pointer(s), outer starts with Function
        let inner_only_ptr_and_array = inner_derived.iter().all(|d| {
            matches!(
                d,
                DerivedDeclarator::Pointer(..) | DerivedDeclarator::Array(_)
            )
        });
        let inner_has_pointer = inner_derived
            .iter()
            .any(|d| matches!(d, DerivedDeclarator::Pointer(..)));
        let outer_starts_with_function = matches!(
            outer_suffixes.first(),
            Some(DerivedDeclarator::Function(..))
        );

        if inner_only_ptr_and_array
            && inner_has_pointer
            && outer_starts_with_function
            && outer_suffixes.len() == 1
        {
            // In C's inside-out reading of declarations:
            //   outer_pointers = return-type pointers (e.g., the `*` in `int *`)
            //   inner_derived = declarator pointers/arrays from inside parens
            //   outer_suffixes = [Function(...)]
            //
            // The LAST Pointer in inner_derived is the function pointer syntax
            // marker (the `(*` in `(*fp)`). Any preceding Pointers in inner_derived
            // are extra indirection levels (e.g., `(**fpp)` has two Pointers — the
            // first is extra indirection, the second is the syntax marker).
            //
            // Layout: outer_pointers ++ [syntax_marker_ptr, FunctionPointer] ++ extra_inner_ptrs ++ inner_arrays
            //
            // Example: `int *(*fp)(int)` → outer=[Pointer], inner=[Pointer]
            //   result: [Pointer, Pointer, FunctionPointer] — one return-type ptr, one syntax ptr
            //
            // Example: `int (**fpp)(int,int)` → outer=[], inner=[Pointer, Pointer]
            //   result: [Pointer, FunctionPointer, Pointer] — syntax ptr + fptr, then extra indirection
            //
            // Example: `int *(**fpp)(int)` → outer=[Pointer], inner=[Pointer, Pointer]
            //   result: [Pointer, Pointer, FunctionPointer, Pointer]
            let mut result = outer_pointers;

            // Count inner pointers. The last one is the function pointer syntax marker.
            // All others are extra indirection levels placed AFTER the FunctionPointer.
            let inner_ptr_count = inner_derived
                .iter()
                .filter(|d| matches!(d, DerivedDeclarator::Pointer(..)))
                .count();
            // The syntax marker is the inner declarator's LAST `*`; it owns the
            // qualifiers of that star (`int (*const fp)(void)` declares a
            // const function-pointer object). The entry is rebuilt below, so
            // carry the CVR run over instead of dropping it with the pointer.
            let marker_cvr = inner_derived
                .iter()
                .rev()
                .find_map(|d| match d {
                    DerivedDeclarator::Pointer(_, cvr) => Some(*cvr),
                    _ => None,
                })
                .unwrap_or(0);
            let extra_indirection_ptrs = if inner_ptr_count > 0 {
                inner_ptr_count - 1
            } else {
                0
            };

            // Emit the function pointer syntax marker + FunctionPointer
            result.push(DerivedDeclarator::Pointer(
                AddressSpace::Default,
                marker_cvr,
            ));
            if let Some(DerivedDeclarator::Function(params, variadic, has_prototype)) =
                outer_suffixes.into_iter().next()
            {
                result.push(DerivedDeclarator::FunctionPointer(
                    params,
                    variadic,
                    has_prototype,
                ));
            }

            // Emit extra indirection Pointers (beyond the syntax marker)
            for _ in 0..extra_indirection_ptrs {
                result.push(DerivedDeclarator::Pointer(AddressSpace::Default, 0));
            }

            // Emit inner arrays (for array of function pointers, e.g., `int (*fps[10])(int)`)
            for d in &inner_derived {
                if matches!(d, DerivedDeclarator::Array(_)) {
                    result.push(d.clone());
                }
            }

            return result;
        }

        // Check for pointer-to-array: inner contains Pointer(s) with optional arrays,
        // outer is all Array(s). Handle inside-out rule correctly:
        //
        // For `int (*p)[3][6]`: inner=[Pointer], outer=[Array(3), Array(6)]
        //   Split inner at pointer: pre_ptr=[], post_ptr=[]
        //   Result: [] ++ [Array(3), Array(6)] ++ [Pointer] ++ [] = [Array(3), Array(6), Pointer]
        //
        // For `int ((*(p))[3])[6]`: inner=[Array(3), Pointer], outer=[Array(6)]
        //   Split inner at pointer: pre_ptr=[Array(3)], post_ptr=[]
        //   Result: [Array(3)] ++ [Array(6)] ++ [Pointer] ++ [] = [Array(3), Array(6), Pointer]
        //
        // For `int (*ptrs[2])[4]`: inner=[Pointer, Array(2)], outer=[Array(4)]
        //   Split inner at pointer: pre_ptr=[], post_ptr=[Array(2)]
        //   Result: [] ++ [Array(4)] ++ [Pointer] ++ [Array(2)] = [Array(4), Pointer, Array(2)]
        let outer_only_arrays = outer_suffixes
            .iter()
            .all(|d| matches!(d, DerivedDeclarator::Array(_)));
        if inner_only_ptr_and_array && inner_has_pointer && outer_only_arrays {
            // Split inner_derived at the last Pointer:
            // - pre_ptr_arrays: arrays before the last pointer (part of pointee type)
            // - post_ptr_arrays: arrays after the last pointer (variable's own array dimensions)
            let last_ptr_idx = inner_derived
                .iter()
                .rposition(|d| matches!(d, DerivedDeclarator::Pointer(..)))
                .expect("inner_has_pointer is true, so a Pointer must exist");
            let mut result = outer_pointers;
            // 1. Arrays from inner that come before the pointer (pointee array dimensions)
            for d in &inner_derived[..last_ptr_idx] {
                if matches!(d, DerivedDeclarator::Array(_)) {
                    result.push(d.clone());
                }
            }
            // 2. Outer array suffixes (also pointee dimensions)
            result.extend(outer_suffixes);
            // 3. Pointer(s)
            for d in &inner_derived[..=last_ptr_idx] {
                if matches!(d, DerivedDeclarator::Pointer(..)) {
                    result.push(d.clone());
                }
            }
            // 4. Arrays from inner that come after the pointer (variable's array dims)
            for d in &inner_derived[last_ptr_idx + 1..] {
                result.push(d.clone());
            }
            return result;
        }

        // Function declarator returning a function pointer (C89 "obscure" form),
        // e.g. sqlite3 fts3_hash.c:
        //   static int (*ftsHashFunction(int keyClass))(const void*,int) { ... }
        // and the nested variant
        //   int (*(*f(int))(char))(long);
        //
        // Shape: the parenthesized inner declarator is itself a *function*
        // declarator (`*name(A)` → inner = [Pointer, Function(A), ...]) and the
        // outer suffix is the parameter list of the *returned* function pointer
        // (`(B)` → outer_suffixes = [Function(B)]).
        //
        // C reads this inside-out: `f` is a function taking A returning a
        // pointer to a function taking B returning the base type. In the
        // derived-list encoding consumed by `build_full_ctype_with_base` and
        // `build_return_type`, that is:
        //   outer_pointers ++ [Pointer, FunctionPointer(B)] ++ inner[1..]
        // i.e. the inner's leading syntax-marker `*` binds to the outer suffix
        // to form the returned-function-pointer core, and the rest of the inner
        // declarator (ending in Function(A) — the function's own parameters,
        // which funcdef detection expects at derived.last()) follows.
        //
        // For the nested variant, inner is already the combined
        // [Pointer, FunctionPointer(char), Function(int)] from the recursive
        // parse, and the same rewrite yields
        // [Pointer, FunctionPointer(long), Pointer, FunctionPointer(char), Function(int)].
        //
        // Restricted to shapes where the inner's leading `*` is the free syntax
        // marker for the outer suffix: the inner must end in Function (the
        // function's own parameter list) and no Pointer in the remainder may
        // bind directly to a Function (which would mean a
        // pointer-to-pointer-to-function return like `int (**f(int))(char)`;
        // those fall through to the general case rather than being silently
        // mis-encoded). This accepts arbitrary nesting depth, since each
        // recursive combine emits `[Pointer, FunctionPointer, ...]` chains in
        // which every Pointer is immediately consumed by a FunctionPointer.
        // When the parenthesized inner declarator already contains a `Function`
        // or `FunctionPointer` core (e.g. `*g(int)`, `*(*p)(int)`, `**f(int)`,
        // `*(*p[3])(int)`, `*get_arr(int)`, `*(*fp)(int)`), split `inner_derived`
        // at that FIRST function/function-pointer core (including its preceding
        // syntax-marker `Pointer`):
        //   inner_prefix = &inner_derived[..fn_core_idx]
        //   inner_rest   = &inner_derived[fn_core_idx..]
        // `inner_prefix` holds the return-type modifiers of the inner callable
        // that were written inside the current `(...)` group, so outer `[N]` or
        // `(params)` suffixes bind to `inner_prefix` in exact inside-out order
        // before `inner_rest` (the inner callable's own core and outer modifiers).
        let first_fn_core_idx = inner_derived
            .iter()
            .position(|d| {
                matches!(
                    d,
                    DerivedDeclarator::Function(..) | DerivedDeclarator::FunctionPointer(..)
                )
            })
            .map(|pos| {
                if matches!(inner_derived[pos], DerivedDeclarator::FunctionPointer(..))
                    && pos > 0
                    && matches!(inner_derived[pos - 1], DerivedDeclarator::Pointer(..))
                {
                    pos - 1
                } else {
                    pos
                }
            });

        if let Some(fn_core_idx) = first_fn_core_idx {
            let inner_prefix = &inner_derived[..fn_core_idx];
            let inner_rest = &inner_derived[fn_core_idx..];

            if !outer_suffixes.is_empty()
                && outer_only_arrays
                && let Some(last_ptr_idx) = inner_prefix
                    .iter()
                    .rposition(|d| matches!(d, DerivedDeclarator::Pointer(..)))
            {
                // Callable returning pointer-to-array:
                //   `int (*get_arr(int))[4]` -> [Array(4), Pointer, Function([int])]
                //   `int (*(*fp)(int))[4]`   -> [Array(4), Pointer, Pointer, FunctionPointer([int])]
                let mut result = outer_pointers;
                for d in &inner_prefix[..last_ptr_idx] {
                    if matches!(d, DerivedDeclarator::Array(_)) {
                        result.push(d.clone());
                    }
                }
                result.extend(outer_suffixes);
                for d in &inner_prefix[..=last_ptr_idx] {
                    if matches!(d, DerivedDeclarator::Pointer(..)) {
                        result.push(d.clone());
                    }
                }
                result.extend_from_slice(&inner_prefix[last_ptr_idx + 1..]);
                result.extend_from_slice(inner_rest);
                return result;
            }

            if outer_starts_with_function
                && outer_suffixes.len() == 1
                && let Some(first_ptr_idx) = inner_prefix
                    .iter()
                    .position(|d| matches!(d, DerivedDeclarator::Pointer(..)))
            {
                // Callable (function, N-level function pointer, or array/pointer
                // of function pointers) returning a function pointer (or pointer
                // to function pointer):
                //   `int (*g(int))(char)`          -> [Pointer, FP([char]), Function([int])]
                //   `void (*(*p)(int))(char)`      -> [Pointer, FP([char]), Pointer, FP([int])]
                //   `int (*(*(*p)(int))(long))(c)` -> [Pointer, FP([c]), Pointer, FP([long]), Pointer, FP([int])]
                //   `int (**f(int))(char)`         -> [Pointer, FP([char]), Pointer, Function([int])]
                //   `int (*(*p[3])(int))(char)`    -> [Pointer, FP([char]), Pointer, FP([int]), Array(3)]
                let mut result = outer_pointers;
                result.extend_from_slice(&inner_prefix[..first_ptr_idx]);
                result.push(inner_prefix[first_ptr_idx].clone());
                if let Some(DerivedDeclarator::Function(params, variadic, has_prototype)) =
                    outer_suffixes.into_iter().next()
                {
                    result.push(DerivedDeclarator::FunctionPointer(
                        params,
                        variadic,
                        has_prototype,
                    ));
                }
                result.extend_from_slice(&inner_prefix[first_ptr_idx + 1..]);
                result.extend_from_slice(inner_rest);
                return result;
            }
        }

        // General case: outer_pointers ++ outer_suffixes ++ inner_derived
        outer_pointers.extend(outer_suffixes);
        outer_pointers.extend(inner_derived);
        outer_pointers
    }

    /// Parse a function parameter list: (params...) or (void) or ().
    ///
    /// The alignment pending when a parameter list begins belongs to the
    /// ENCLOSING declaration, not to the parameters.  For
    /// `__attribute__((aligned(64))) void f(int x)` the specifier-level
    /// alignment is already in `parsed_alignas` when the `(` is reached, and the
    /// per-parameter capture inside the list merges into that slot and takes it,
    /// so the function silently lost its own alignment -- for every NON-EMPTY
    /// parameter list, while `f(void)` and `f()` kept it.  Measured against GCC
    /// 14, which honours the attribute in both spellings: the earlier version of
    /// this file honoured it only for parameterless functions, in both the
    /// definition channel and the prototype channel (the prototype channel has
    /// the same take, so a declaration `aligned(64) void f(int x);` registered
    /// nothing).  Hold the value aside for the duration of the list instead: the
    /// per-parameter take then only ever sees a parameter's OWN attributes,
    /// which is also what removes the leak into the next parameter that the take
    /// exists to prevent.
    pub(super) fn parse_param_list(&mut self) -> (Vec<ParamDecl>, bool, bool) {
        let decl_alignas = self.attrs.parsed_alignas.take();
        let decl_alignas_type = self.attrs.parsed_alignas_type.take();
        // The enclosing declaration's volatile and typedef-use state is read
        // after its declarator (`volatile int f(int a)`), so a parameter's
        // own specifiers must not leave it behind.
        let decl_volatile = self.attrs.parsing_volatile();
        let decl_typedef_use = self.attrs.parsing_typedef_use();
        let decl_typedef_object = self.attrs.parsing_typedef_object_volatile();
        let decl_storage = self.attrs.take_storage_classes();
        let result = self.parse_param_list_inner();
        // Single restore point, deliberately: every exit from the list --
        // including the K&R identifier-list path -- goes through here, so no
        // early return can drop an enclosing declaration's alignment.
        self.attrs.parsed_alignas = decl_alignas;
        self.attrs.parsed_alignas_type = decl_alignas_type;
        self.attrs.set_volatile(decl_volatile);
        self.attrs.set_typedef_use(decl_typedef_use);
        self.attrs.set_typedef_object_volatile(decl_typedef_object);
        self.attrs.restore_storage_classes(decl_storage);
        result
    }

    fn parse_param_list_inner(&mut self) -> (Vec<ParamDecl>, bool, bool) {
        let open = self.peek_span();
        self.expect_context(&TokenKind::LParen, "for parameter list");
        let mut params = Vec::with_capacity(8);
        let mut variadic = false;

        if matches!(self.peek(), TokenKind::RParen) {
            self.advance();
            // C23 changes an empty parameter list into a prototype with no
            // parameters. Earlier dialects retain the historical unspecified
            // parameter list; the dialect is selected before parsing.
            return (params, variadic, self.c23);
        }

        // Handle (void)
        if matches!(self.peek(), TokenKind::Void) {
            let save = self.pos;
            self.advance();
            if matches!(self.peek(), TokenKind::RParen) {
                self.advance();
                // `(void)` is an explicit zero-parameter prototype in every C
                // dialect, including C90 and GNU17.
                return (params, variadic, true);
            }
            self.pos = save;
        }

        // Check for K&R-style identifier list
        if let TokenKind::Identifier(name) = self.peek() {
            if (!self.typedefs.contains(name) || self.shadowed_typedefs.contains(name))
                && !self.is_type_specifier()
            {
                return self.parse_kr_identifier_list();
            }
        }

        loop {
            if matches!(self.peek(), TokenKind::Ellipsis) {
                self.advance();
                variadic = true;
                break;
            }

            // Save noreturn before skip_gcc_extensions() so that a noreturn attribute
            // on a function pointer parameter (e.g. `__attribute__((__noreturn__)) fn_ptr_t`)
            // doesn't leak to the enclosing function declaration.
            let saved_noreturn = self.attrs.parsing_noreturn();
            // Parse pre-name GCC attributes (e.g. `__attribute__((aligned(16))) int x`)
            // instead of skipping them, so a pre-name aligned attribute is
            // honored exactly like the _Alignas pre-name spelling.
            let (_, pre_align, _, _, _) = self.parse_gcc_attributes();
            if let Some(a) = pre_align {
                self.attrs.parsed_alignas = Some(
                    self.attrs
                        .parsed_alignas
                        .map_or(a, |prev: usize| prev.max(a)),
                );
            }
            // Save and reset parsing_const to detect if this parameter's base type is const.
            let saved_const = self.attrs.parsing_const();
            self.attrs.set_const(false);
            // The pointer-star `const` is per parameter too (`f(int a, int
            // *const p)` must not leak p's star const into `a`'s declarator);
            // save the enclosing declaration's state and restore it below
            // alongside `saved_const`.
            let saved_pointer_const = self.attrs.parsing_pointer_const();
            self.attrs.set_pointer_const(false);
            // Likewise `restrict`: an earlier parameter's star or specifier
            // (`int *restrict *a, const char **b`) is not `b`'s.
            let saved_restrict = self.attrs.parsing_restrict();
            self.attrs.set_restrict(false);
            // Each parameter's volatile and typedef qualifiers are its own
            // (`VI p, int y`: `y` is not volatile), so clear them before its
            // specifiers are parsed.
            self.attrs.set_volatile(false);
            self.attrs.set_typedef_use(false);
            self.attrs.set_typedef_object_volatile(false);
            self.attrs.set_noreturn(saved_noreturn);
            // Each parameter's named address space is its own: clear the slot so
            // a qualifier left over from the enclosing declaration cannot be
            // taken as this parameter's.
            self.attrs.parsing_address_space = AddressSpace::Default;
            self.attrs.take_storage_classes();
            if let Some(mut type_spec) = self.parse_type_specifier() {
                // Snapshot the declaration-level named address space NOW, before
                // the declarator parser runs (the same rule struct fields follow
                // in parse_struct_fields). `struct c __seg_gs *p` records
                // `__seg_gs` in `parsing_address_space`; without this read the
                // parameter's pointer was built with AddressSpace::Default and
                // every `p->member` load was emitted without the %gs prefix
                // (kernel: srcu_read_unlock_fast's per-CPU counter pointer).
                let param_addr_space = std::mem::take(&mut self.attrs.parsing_address_space);
                let param_span = self.peek_span();
                // Capture whether the base type (before pointer declarators) was const.
                // For `const int *p`, parsing_const is true here; the `*` is handled below.
                let param_is_const = self.attrs.parsing_const();
                let param_is_volatile = self.attrs.parsing_volatile();
                let param_base_restrict = self.attrs.parsing_restrict();
                let (
                    name,
                    pointer_depth,
                    array_dims,
                    is_func_ptr,
                    ptr_to_array_dims,
                    fptr_param_decls,
                    fptr_variadic,
                    fptr_has_prototype,
                    inner_ptr_depth,
                    param_is_restrict,
                    paren_ptr_total,
                    star_quals,
                    paren_star_quals,
                    outer_fn_returns,
                ) = self.parse_param_declarator_full();
                let adjusted_cvr = self.attrs.param_adjusted_cvr;
                self.check_parameter_storage(
                    name.as_deref(),
                    param_span,
                    param_addr_space,
                    pointer_depth == 0
                        && inner_ptr_depth == 0
                        && array_dims.is_empty()
                        && ptr_to_array_dims.is_empty()
                        && !is_func_ptr,
                );
                // Post-name GCC attributes: `int x __attribute__((aligned(32)))`.
                // The parameter declarator parser does not consume them; parse
                // them here and merge `aligned` with any pre-name/_Alignas
                // alignment already captured.
                let (_, post_align, _, _, _) = self.parse_gcc_attributes();
                if let Some(a) = post_align {
                    self.attrs.parsed_alignas = Some(
                        self.attrs
                            .parsed_alignas
                            .map_or(a, |prev: usize| prev.max(a)),
                    );
                }
                // Direct `__attribute__((aligned(N)))` / `_Alignas(...)`
                // spellings on the parameter: captured during declarator
                // parsing. `take()` also prevents leakage into the NEXT
                // parameter (the save/reset pattern above covers const and
                // noreturn, but parsed_alignas was never reset here).
                let param_alignment = self.attrs.parsed_alignas.take();
                let param_alignas_type = self.attrs.parsed_alignas_type.take();
                self.skip_gcc_extensions();

                // Apply pointer levels. A named address space qualifies the
                // memory the INNERMOST pointer points into (GCC: `T __seg_gs **pp`
                // makes only `*pp` a %gs access), so it lands on the first level
                // built, exactly as `apply_declaration_address_space` places it
                // for declarations. The outer levels stay ordinary memory.
                //
                // The pointer levels split around a pointee ARRAY suffix.  A
                // parenthesized pointer-to-array declarator binds its stars from
                // the OUTSIDE (`(*p)[N]` is `Pointer(Array(T, N))`, and
                // `(**p)[N]` — the kernel's `const char (**names)[ETH_GSTRING_LEN]`
                // in net/ethtool/privflags.c — is `Pointer(Pointer(Array(T, N)))`),
                // while the leading stars bind to the base type from the inside
                // (`int *(*p)[N]` is `Pointer(Array(Pointer(int), N))`).  Applying
                // every pointer level first built the first shape as
                // `Pointer(Array(Pointer(T), N))` — one pointer level attached to
                // the wrong side of the array — so the kernel rejected a correct
                // argument to `ethnl_get_priv_flags_info`.
                let leading_depth = if paren_ptr_total > 0 {
                    pointer_depth + 1 - paren_ptr_total
                } else {
                    pointer_depth
                };
                let trailing_depth = if paren_ptr_total > 0 {
                    paren_ptr_total
                } else {
                    0
                };
                for level in 0..leading_depth {
                    // Level L's pointee is the storage of the pointer built by the
                    // star before it, so it takes that star's qualifier.
                    let space = if level == 0 {
                        param_addr_space
                    } else {
                        star_quals
                            .get(level as usize - 1)
                            .map(|(space, _)| *space)
                            .unwrap_or(AddressSpace::Default)
                    };
                    type_spec = TypeSpecifier::Pointer(Box::new(type_spec), space);
                }
                // Pointer-to-array: int (*p)[N][M], int (**p)[N][M]
                if !ptr_to_array_dims.is_empty() {
                    for dim in ptr_to_array_dims.iter().rev() {
                        type_spec = TypeSpecifier::Array(Box::new(type_spec), dim.clone());
                    }
                }
                // CVR of each pointer level in creation order: wrap `L` builds
                // level `L+1`, and the star written at that position owns its
                // qualifiers (`int *const *p`: level 1 is const). Stars inside
                // the parenthesised group are counted separately and keep their
                // own representation; unknown levels stay 0.
                let mut pointer_quals = vec![0u8; pointer_depth as usize];
                for (index, (_, cvr)) in star_quals.iter().enumerate() {
                    if let Some(slot) = pointer_quals.get_mut(index) {
                        *slot = *cvr;
                    }
                }
                for _ in 0..trailing_depth {
                    type_spec = TypeSpecifier::Pointer(Box::new(type_spec), AddressSpace::Default);
                }

                // Wrap any outer return-type function-pointer layers (from outermost
                // to innermost) before decaying the parameter's own function/array core.
                for (ret_params, ret_variadic, ret_has_prototype, extra_ptrs) in
                    outer_fn_returns.into_iter().rev()
                {
                    type_spec = TypeSpecifier::FunctionPointer(
                        Box::new(type_spec),
                        ret_params,
                        ret_variadic,
                        ret_has_prototype,
                    );
                    for _ in 0..extra_ptrs {
                        type_spec =
                            TypeSpecifier::Pointer(Box::new(type_spec), AddressSpace::Default);
                    }
                }

                // Array params: outermost dimension decays to pointer.
                // Preserve the outermost dimension expression (if any) so side effects
                // like `a++` in `int b[a++]` can be evaluated during IR lowering.
                // (Inner dimensions are preserved in type_spec and evaluated by
                // compute_vla_param_strides).
                let mut vla_size_exprs = Vec::with_capacity(2);
                if !array_dims.is_empty() {
                    if let Some(Some(expr)) = array_dims.first() {
                        vla_size_exprs.push((**expr).clone());
                    }
                    for dim in array_dims.iter().skip(1).rev() {
                        type_spec = TypeSpecifier::Array(Box::new(type_spec), dim.clone());
                    }
                    type_spec = TypeSpecifier::Pointer(Box::new(type_spec), AddressSpace::Default);
                }

                // Function pointers decay to pointer
                if is_func_ptr {
                    type_spec = TypeSpecifier::Pointer(Box::new(type_spec), AddressSpace::Default);
                }

                let param_pointer_const = self.attrs.parsing_pointer_const();
                self.attrs.set_const(saved_const);
                self.attrs.set_pointer_const(saved_pointer_const);
                self.attrs.set_restrict(saved_restrict);
                self.attrs.set_noreturn(saved_noreturn);
                params.push(ParamDecl {
                    type_spec,
                    name,
                    alignment: param_alignment,
                    alignas_type: param_alignas_type,
                    fptr_params: fptr_param_decls,
                    fptr_variadic,
                    fptr_has_prototype,
                    is_const: param_is_const,
                    is_volatile: param_is_volatile,
                    is_restrict: param_is_restrict,
                    vla_size_exprs,
                    decayed_array: !array_dims.is_empty(),
                    fptr_inner_ptr_depth: inner_ptr_depth,
                    pointer_depth,
                    pointer_const: param_pointer_const,
                    pointer_quals,
                    base_restrict: param_base_restrict,
                    paren_star_quals,
                    adjusted_cvr,
                });
            } else {
                self.attrs.set_const(saved_const);
                self.attrs.set_pointer_const(saved_pointer_const);
                self.attrs.set_restrict(saved_restrict);
                self.attrs.set_noreturn(saved_noreturn);
                break;
            }

            if !self.consume_if(&TokenKind::Comma) {
                break;
            }
        }

        self.expect_closing(&TokenKind::RParen, open);
        (params, variadic, true)
    }

    /// A parameter's storage constraints are independent of declarator
    /// construction. Check them once after its shape is known, retaining the
    /// original parameter span and register-storage exception.
    fn check_parameter_storage(
        &mut self,
        name: Option<&str>,
        span: Span,
        space: AddressSpace,
        direct_object: bool,
    ) {
        if self.attrs.take_storage_classes() != 0 {
            let message = match name {
                Some(name) => format!("storage class specified for parameter '{name}'"),
                None => "storage class specified for unnamed parameter".to_string(),
            };
            self.emit_error(message, span);
        }
        // Parameters have automatic storage; a named space can qualify a
        // pointee but cannot be the storage of a direct parameter object.
        if space != AddressSpace::Default && direct_object {
            self.emit_error(
                format!("'{space}' specified for parameter '{}'", name.unwrap_or("")),
                span,
            );
        }
    }

    /// The last parenthesised star describes the parameter object's storage,
    /// unlike earlier stars that describe pointees. Preserve the distinct
    /// GCC wording for unnamed parenthesised parameters.
    fn check_param_group_space(&mut self, name: Option<&str>, space: AddressSpace, span: Span) {
        if space != AddressSpace::Default {
            let message = match name {
                Some(name) => format!("'{space}' specified for parameter '{name}'"),
                None => format!("'{space}' specified for unnamed parameter"),
            };
            self.emit_error(message, span);
        }
    }

    /// One parameter star's exact CVR and named space. Reset per-star flags,
    /// then restore the declaration-wide unions; do not rescan qualifier tokens.
    fn parse_param_star_quals(&mut self) -> (AddressSpace, u8) {
        let base_space =
            std::mem::replace(&mut self.attrs.parsing_address_space, AddressSpace::Default);
        self.attrs.set_pointer_const(false);
        let outer_volatile = self.attrs.parsing_volatile();
        let outer_restrict = self.attrs.parsing_restrict();
        self.attrs.set_volatile(false);
        self.attrs.set_restrict(false);
        self.skip_cv_qualifiers();
        let star_volatile = self.attrs.parsing_volatile();
        let star_restrict = self.attrs.parsing_restrict();
        let cvr = u8::from(self.attrs.parsing_pointer_const())
            | (u8::from(star_volatile) << 1)
            | (u8::from(star_restrict) << 2);
        self.attrs.set_volatile(outer_volatile || star_volatile);
        self.attrs.set_restrict(outer_restrict || star_restrict);
        (
            std::mem::replace(&mut self.attrs.parsing_address_space, base_space),
            cvr,
        )
    }

    /// Parse a K&R-style identifier list: foo(a, b, c)
    fn parse_kr_identifier_list(&mut self) -> (Vec<ParamDecl>, bool, bool) {
        let mut params = Vec::with_capacity(8);
        while let TokenKind::Identifier(n) = self.peek() {
            let n = n.clone();
            self.advance();
            params.push(ParamDecl {
                type_spec: TypeSpecifier::Int, // K&R default type
                name: Some(n),
                alignment: None,
                alignas_type: None,
                fptr_params: None,
                fptr_variadic: false,
                fptr_has_prototype: false,
                is_const: false,
                is_volatile: false,
                is_restrict: false,
                vla_size_exprs: Vec::new(),
                decayed_array: false,
                fptr_inner_ptr_depth: 0,
                pointer_depth: 0,
                pointer_const: false,
                pointer_quals: Vec::new(),
                base_restrict: false,
                parsing_record_attributes: false,
                paren_star_quals: Vec::new(),
                adjusted_cvr: 0,
            });
            if !self.consume_if(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen);
        (params, false, false)
    }

    /// Parse a parameter declarator with full type information.
    /// Returns (name, pointer_depth, array_dims, is_func_ptr, ptr_to_array_dims,
    /// fptr_params, fptr_variadic, fptr_has_prototype, fptr_inner_ptr_depth).
    #[allow(clippy::type_complexity)]
    pub(super) fn parse_param_declarator_full(
        &mut self,
    ) -> (
        Option<String>,
        u32,
        Vec<Option<Box<Expr>>>,
        bool,
        Vec<Option<Box<Expr>>>,
        Option<Vec<ParamDecl>>,
        bool,
        bool,
        u32,
        bool,
        // Pointer levels contributed by a parenthesized pointer-to-array
        // declarator (`(*p)[N]` → 1, `(**p)[N]` → 2), 0 otherwise.  The
        // CALLER needs the split: these bind to the array suffix from the
        // OUTSIDE, while the leading stars bind to the base type from the
        // inside (see the construction in parse_param_list).
        u32,
        Vec<(AddressSpace, u8)>,
        // The parenthesised stars' CVR bits (see `ParamDecl::paren_star_quals`).
        Vec<u8>,
        Vec<(Vec<ParamDecl>, bool, bool, u32)>,
    ) {
        let param_start = self.peek_span();
        let mut pointer_depth: u32 = 0;
        let mut is_restrict = false;
        // Space written after each leading `*`, in written order (see
        // `DerivedDeclarator::Pointer`): `star_spaces[k]` qualifies the pointee
        // of the pointer built by star k+2.
        let mut star_quals: Vec<(AddressSpace, u8)> = Vec::new();
        loop {
            // Bounded: see POINTER_DEPTH_BUDGET. The span is taken before
            // consuming so the diagnostic points at the offending `*`.
            let star_span = self.peek_span();
            if !self.consume_if(&TokenKind::Star) {
                break;
            }
            pointer_depth += 1;
            if !self.enter_pointer_level(pointer_depth, star_span) {
                break;
            }
            let quals = self.parse_param_star_quals();
            is_restrict |= quals.1 & CVR_RESTRICT != 0;
            star_quals.push(quals);
            // Also skip __attribute__(...) after pointer qualifiers.
            // E.g., `int * __attribute__((unused)) p` is valid GNU C.
            self.skip_gcc_extensions();
        }
        let mut array_dims: Vec<Option<Box<Expr>>> = Vec::new();
        let mut is_func_ptr = false;
        let mut ptr_to_array_dims: Vec<Option<Box<Expr>>> = Vec::new();
        let mut fptr_params: Option<Vec<ParamDecl>> = None;
        let mut fptr_variadic = false;
        let mut fptr_has_prototype = false;
        let mut fptr_inner_ptr_depth: u32 = 0;
        let mut paren_ptr_total: u32 = 0;
        let mut paren_star_quals: Vec<u8> = Vec::new();
        let mut outer_fn_returns: Vec<(Vec<ParamDecl>, bool, bool, u32)> = Vec::new();

        let had_paren_decl = matches!(self.peek(), TokenKind::LParen) && self.is_paren_declarator();
        let name = if had_paren_decl {
            self.parse_paren_param_declarator(
                &mut pointer_depth,
                &mut array_dims,
                &mut is_func_ptr,
                &mut ptr_to_array_dims,
                &mut fptr_params,
                &mut fptr_variadic,
                &mut fptr_has_prototype,
                &mut fptr_inner_ptr_depth,
                &mut paren_ptr_total,
                &mut paren_star_quals,
                &mut outer_fn_returns,
            )
        } else if let TokenKind::Identifier(n) = self.peek() {
            let n = n.clone();
            self.advance();
            Some(n)
        } else {
            None
        };

        // Parse trailing array dimensions
        self.attrs.param_adjusted_cvr = 0;
        while matches!(self.peek(), TokenKind::LBracket) && !self.is_c23_attr_start() {
            self.advance();
            // C99 array qualifiers (static, restrict, const, volatile); the
            // first dimension's qualify the adjusted pointer.
            let quals = self.skip_array_qualifiers();
            if array_dims.is_empty() {
                self.attrs.param_adjusted_cvr = quals;
            }
            if matches!(self.peek(), TokenKind::RBracket) {
                array_dims.push(None);
                self.advance();
            } else if matches!(self.peek(), TokenKind::Star)
                && self.pos + 1 < self.tokens.len()
                && matches!(self.tokens[self.pos + 1].kind, TokenKind::RBracket)
            {
                // C99 VLA star syntax: [*] or [static *] means unspecified VLA size
                self.advance(); // consume '*'
                array_dims.push(None);
                self.advance(); // consume ']'
            } else {
                let dim_expr = self.parse_expr();
                array_dims.push(Some(Box::new(dim_expr)));
                self.expect(&TokenKind::RBracket);
            }
        }

        // Trailing function parameter list means this parameter has function type,
        // which in C decays to a function pointer (C11 6.7.6.3p8).
        // E.g., `int f(union U callback(void))` → callback has type `union U (*)(void)`
        // Also handles abstract declarators: `void (Dat *)` → function taking Dat*, returning void
        if matches!(self.peek(), TokenKind::LParen) {
            is_func_ptr = true;
            let (fp_params, variadic, has_prototype) = self.parse_param_list();
            fptr_params = Some(fp_params);
            fptr_variadic = variadic;
            fptr_has_prototype = has_prototype;
        }

        // The last leading `*` qualifier names the parameter object itself
        // (`long *__seg_gs p`), which GCC rejects, ONLY when no parenthesized
        // pointer/function-pointer or trailing array/function layer makes that
        // leading `*` a pointee/return-type pointer (e.g. `int * __seg_gs (*p)[4]`
        // or `int * __seg_gs *p[4]`, where the parameter itself is ordinary).
        if !had_paren_decl && !is_func_ptr && array_dims.is_empty() {
            if let Some(&(object_space, _)) = star_quals.last() {
                if object_space != AddressSpace::Default {
                    self.emit_error(
                        format!(
                            "'{}' specified for parameter '{}'",
                            object_space,
                            name.as_deref().unwrap_or("")
                        ),
                        param_start,
                    );
                }
            }
        }

        (
            name,
            pointer_depth,
            array_dims,
            is_func_ptr,
            ptr_to_array_dims,
            fptr_params,
            fptr_variadic,
            fptr_has_prototype,
            fptr_inner_ptr_depth,
            is_restrict,
            paren_ptr_total,
            star_quals,
            paren_star_quals,
            outer_fn_returns,
        )
    }

    /// Parse a parenthesized parameter declarator: (*name)(params), (name), etc.
    #[allow(clippy::too_many_arguments)]
    fn parse_paren_param_declarator(
        &mut self,
        pointer_depth: &mut u32,
        array_dims: &mut Vec<Option<Box<Expr>>>,
        is_func_ptr: &mut bool,
        ptr_to_array_dims: &mut Vec<Option<Box<Expr>>>,
        fptr_params: &mut Option<Vec<ParamDecl>>,
        fptr_variadic: &mut bool,
        fptr_has_prototype: &mut bool,
        fptr_inner_ptr_depth: &mut u32,
        paren_ptr_total: &mut u32,
        paren_star_quals: &mut Vec<u8>,
        outer_fn_returns: &mut Vec<(Vec<ParamDecl>, bool, bool, u32)>,
    ) -> Option<String> {
        let save = self.pos;
        self.advance(); // consume '('

        // Skip __attribute__ / __extension__ before pointer declarator
        // e.g. void foo(void (__attribute__((ms_abi)) *handler)(int))
        self.skip_gcc_extensions();

        if matches!(self.peek(), TokenKind::LBracket) && !self.is_c23_attr_start() {
            // Abstract array declarator in parens: ([4]) or ([])
            // E.g., `int f(int ([4]))` is equivalent to `int f(int *)`
            while matches!(self.peek(), TokenKind::LBracket) && !self.is_c23_attr_start() {
                self.advance();
                self.skip_array_qualifiers();
                if matches!(self.peek(), TokenKind::RBracket) {
                    array_dims.push(None);
                    self.advance();
                } else {
                    let dim_expr = self.parse_expr();
                    array_dims.push(Some(Box::new(dim_expr)));
                    self.expect(&TokenKind::RBracket);
                }
            }
            self.expect(&TokenKind::RParen);
            None
        } else if matches!(self.peek(), TokenKind::Star) {
            // Function pointer or pointer-to-array: (*name)(params) or (*name)[N]
            let mut inner_ptr_depth = 0u32;
            // The space written after the last inner `*` qualifies the object
            // being declared (`(*__seg_gs p)`), as in the outer `*` loop, and
            // is rejected below. The earlier stars' spaces qualify pointees.
            let mut object_space = AddressSpace::Default;
            let mut object_span = self.peek_span();
            loop {
                let star_span = self.peek_span();
                if !self.consume_if(&TokenKind::Star) {
                    break;
                }
                inner_ptr_depth += 1;
                if !self.enter_pointer_level(inner_ptr_depth, star_span) {
                    break;
                }
                let (space, cvr) = self.parse_param_star_quals();
                paren_star_quals.push(cvr);
                object_space = space;
                object_span = star_span;
                // Also skip __attribute__(...) after pointer qualifiers.
                // E.g., `void (*__attribute__((unused)) fp)(int)` is valid GNU C.
                self.skip_gcc_extensions();
            }
            let mut nested_group_name = None;
            if matches!(self.peek(), TokenKind::LParen) && self.is_paren_declarator() {
                let inner_save = self.pos;
                let inner_attrs = self.attrs.clone();
                let mut nested_ptr_depth = 0u32;
                let mut nested_array_dims = Vec::new();
                let mut nested_is_func_ptr = false;
                let mut nested_ptr_to_array_dims = Vec::new();
                let mut nested_fptr_params = None;
                let mut nested_fptr_variadic = false;
                let mut nested_fptr_has_prototype = false;
                let mut nested_inner_ptr_depth = 0u32;
                let mut nested_paren_ptr_total = 0u32;
                let mut nested_paren_star_quals = Vec::new();
                let mut nested_outer_fn_returns = Vec::new();
                let nested_name = self.parse_paren_param_declarator(
                    &mut nested_ptr_depth,
                    &mut nested_array_dims,
                    &mut nested_is_func_ptr,
                    &mut nested_ptr_to_array_dims,
                    &mut nested_fptr_params,
                    &mut nested_fptr_variadic,
                    &mut nested_fptr_has_prototype,
                    &mut nested_inner_ptr_depth,
                    &mut nested_paren_ptr_total,
                    &mut nested_paren_star_quals,
                    &mut nested_outer_fn_returns,
                );
                if nested_is_func_ptr && self.consume_if(&TokenKind::RParen) {
                    if matches!(self.peek(), TokenKind::LParen) {
                        let (ret_params, ret_variadic, ret_has_prototype) = self.parse_param_list();
                        outer_fn_returns.extend(nested_outer_fn_returns);
                        outer_fn_returns.push((
                            ret_params,
                            ret_variadic,
                            ret_has_prototype,
                            inner_ptr_depth.saturating_sub(1),
                        ));
                        *is_func_ptr = true;
                        *fptr_params = nested_fptr_params;
                        *fptr_variadic = nested_fptr_variadic;
                        *fptr_has_prototype = nested_fptr_has_prototype;
                        *fptr_inner_ptr_depth = nested_inner_ptr_depth;
                        *array_dims = nested_array_dims;
                        return nested_name;
                    }
                }
                // A nested group of plain stars closed by this group's `)`
                // (`(*(*p))[3]`, `(*(*fp))(int)`): its stars bind outside the
                // array or parameter list exactly like this group's own, so
                // they add to its depth. Rewinding and keeping only the name
                // dropped them (`int (*(*p))[3]` was typed `int (*)[3]`).
                let plain_stars = !nested_is_func_ptr
                    && nested_array_dims.is_empty()
                    && nested_ptr_to_array_dims.is_empty()
                    && nested_paren_ptr_total == 0
                    && nested_outer_fn_returns.is_empty()
                    && nested_fptr_params.is_none()
                    && matches!(self.peek(), TokenKind::RParen);
                if plain_stars {
                    inner_ptr_depth += nested_ptr_depth;
                    // Written after this group's stars, so further out.
                    paren_star_quals.extend(nested_paren_star_quals);
                    nested_group_name = Some(nested_name);
                } else {
                    self.pos = inner_save;
                    self.attrs = inner_attrs;
                }
            }
            let name = if let Some(nested_name) = nested_group_name {
                nested_name
            } else if let TokenKind::Identifier(n) = self.peek() {
                let n = n.clone();
                self.advance();
                Some(n)
            } else if matches!(self.peek(), TokenKind::LParen) {
                self.extract_paren_name()
            } else {
                None
            };
            // A parameter has automatic storage and cannot live in a named
            // space; GCC 14.2 rejects `void f(long (*__seg_gs p)[4])` and the
            // unnamed form. Same rule as the non-parenthesised declarator.
            self.check_param_group_space(name.as_deref(), object_space, object_span);
            *pointer_depth += inner_ptr_depth.saturating_sub(1);
            // Parse array dimensions inside parens: (*a[]) or (*a[N])
            // These represent "array of pointers" and need to be propagated
            // to the caller so array-to-pointer decay is properly applied.
            // E.g. int (*a[]) = array of ptr-to-int = int **a (after decay)
            let mut inner_array_dims = Vec::with_capacity(8);
            while matches!(self.peek(), TokenKind::LBracket) && !self.is_c23_attr_start() {
                self.advance();
                self.skip_array_qualifiers();
                if matches!(self.peek(), TokenKind::RBracket) {
                    inner_array_dims.push(None);
                    self.advance();
                } else {
                    let dim_expr = self.parse_expr();
                    inner_array_dims.push(Some(Box::new(dim_expr)));
                    self.expect(&TokenKind::RBracket);
                }
            }
            self.expect(&TokenKind::RParen);
            if !inner_array_dims.is_empty() && !matches!(self.peek(), TokenKind::LParen) {
                // Array of pointers: (*a[]) or (*a[N])
                // The * gives one level of pointer, and the array dims are
                // propagated for the caller to apply array-to-pointer decay.
                // E.g. int (*a[]) → array_dims=[None], pointer_depth+=1 → int **a
                *pointer_depth += 1;
                *array_dims = inner_array_dims;
            } else if matches!(self.peek(), TokenKind::LParen) {
                // Function pointer: (*fp)(params) or (*fp[])(params)
                *is_func_ptr = true;
                *fptr_inner_ptr_depth = inner_ptr_depth;
                let (fp_params, variadic, has_prototype) = self.parse_param_list();
                *fptr_params = Some(fp_params);
                *fptr_variadic = variadic;
                *fptr_has_prototype = has_prototype;
            } else if matches!(self.peek(), TokenKind::LBracket) && !self.is_c23_attr_start() {
                // Pointer-to-array: (*p)[N], (**p)[N]
                //
                // Every star counts: `(*p)[N]` has the syntax-marker star and
                // `(**p)[N]` has the marker plus one indirection level, and
                // BOTH bind to the array from the outside.  The caller builds
                // `Pointer^paren_ptr_total(Array^dims(base))`, which is why the
                // count is reported separately from `pointer_depth` (that one
                // keeps the leading stars, which bind to the base from the
                // inside).
                *paren_ptr_total = inner_ptr_depth;
                while matches!(self.peek(), TokenKind::LBracket) && !self.is_c23_attr_start() {
                    self.advance();
                    self.skip_array_qualifiers();
                    if matches!(self.peek(), TokenKind::RBracket) {
                        ptr_to_array_dims.push(None);
                        self.advance();
                    } else {
                        let dim_expr = self.parse_expr();
                        ptr_to_array_dims.push(Some(Box::new(dim_expr)));
                        self.expect(&TokenKind::RBracket);
                    }
                }
            } else {
                *pointer_depth += 1;
            }
            name
        } else if self.consume_if(&TokenKind::Caret) {
            // Block pointer (Apple extension)
            let name = if let TokenKind::Identifier(n) = self.peek() {
                let n = n.clone();
                self.advance();
                Some(n)
            } else {
                None
            };
            self.expect(&TokenKind::RParen);
            if matches!(self.peek(), TokenKind::LParen) {
                self.skip_balanced_parens();
            }
            name
        } else if let TokenKind::Identifier(_) = self.peek() {
            // Parenthesized name: (name), (name)(params), or (name(params))
            let name = if let TokenKind::Identifier(n) = self.peek() {
                let n = n.clone();
                self.advance();
                Some(n)
            } else {
                None
            };
            // Check for function parameter list INSIDE the outer parens:
            // E.g., `int (fn_op(void *, object, object))` where the parameter list
            // is inside the parenthesized declarator. This declares fn_op as having
            // function type `int(void *, object, object)` which decays to a function
            // pointer per C11 6.7.6.3p8.
            if matches!(self.peek(), TokenKind::LParen) {
                *is_func_ptr = true;
                let (fp_params, variadic, has_prototype) = self.parse_param_list();
                *fptr_params = Some(fp_params);
                *fptr_variadic = variadic;
                *fptr_has_prototype = has_prototype;
            }
            self.expect(&TokenKind::RParen);
            self.skip_array_dimensions();
            // Trailing (params) outside the parens means function-type parameter decay.
            // E.g., `int (f)(int)` → f has function type, decays to function pointer.
            // Parse the param list to preserve function type information.
            if !*is_func_ptr && matches!(self.peek(), TokenKind::LParen) {
                *is_func_ptr = true;
                let (fp_params, variadic, has_prototype) = self.parse_param_list();
                *fptr_params = Some(fp_params);
                *fptr_variadic = variadic;
                *fptr_has_prototype = has_prototype;
            }
            name
        } else if matches!(self.peek(), TokenKind::LParen) {
            // Nested parens: ((name)), ((*name)), ((name)(params)), or ((type))
            let inner_save = self.pos;
            let name = self.extract_paren_name();
            if name.is_some() {
                // Successfully extracted a name. Check for function param lists
                // inside the outer parens, e.g. ((fnc)(int)) or ((*fp)(int))
                self.skip_array_dimensions();
                // Trailing (params) inside outer parens means function pointer.
                // E.g., `int ((*f)(int))` or `int ((f)(int))`
                // Parse the param list to preserve function type information.
                if matches!(self.peek(), TokenKind::LParen) {
                    *is_func_ptr = true;
                    let (fp_params, variadic, has_prototype) = self.parse_param_list();
                    *fptr_params = Some(fp_params);
                    *fptr_variadic = variadic;
                    *fptr_has_prototype = has_prototype;
                }
            } else {
                // extract_paren_name failed (e.g. ((int)) where inner content
                // is a type, not a name). Restore position and skip the inner
                // content as balanced parentheses.
                self.pos = inner_save;
                self.skip_balanced_parens();
            }
            self.expect(&TokenKind::RParen);
            // Also skip trailing suffixes after the outer parens
            self.skip_array_dimensions();
            // Trailing (params) after outer parens also means function type decay.
            // Parse the param list to preserve function type information.
            if matches!(self.peek(), TokenKind::LParen) {
                *is_func_ptr = true;
                let (fp_params, variadic, has_prototype) = self.parse_param_list();
                *fptr_params = Some(fp_params);
                *fptr_variadic = variadic;
                *fptr_has_prototype = has_prototype;
            }
            name
        } else {
            self.pos = save;
            None
        }
    }

    /// Extract a name from nested parentheses: (name), ((name)), (*(name)), etc.
    pub(super) fn extract_paren_name(&mut self) -> Option<String> {
        if !matches!(self.peek(), TokenKind::LParen) {
            if let TokenKind::Identifier(n) = self.peek() {
                let n = n.clone();
                self.advance();
                return Some(n);
            }
            return None;
        }
        self.advance(); // consume '('
        if matches!(self.peek(), TokenKind::Star) {
            self.advance();
            self.skip_cv_qualifiers();
        }
        let name = if matches!(self.peek(), TokenKind::LParen) {
            self.extract_paren_name()
        } else if let TokenKind::Identifier(n) = self.peek() {
            let n = n.clone();
            self.advance();
            Some(n)
        } else {
            None
        };
        self.consume_if(&TokenKind::RParen);
        name
    }

    /// Parse one grammar level of an abstract declarator (`stars (inner)? suffixes*`),
    /// tracking cumulative pointer depth against `POINTER_DEPTH_BUDGET` and
    /// preserving `attrs.{parsing_const, parsing_volatile, parsing_address_space}`.
    pub(super) fn parse_abstract_declarator_level(
        &mut self,
        ptr_budget: &mut u32,
    ) -> AbstractDeclaratorLevel {
        let mut stars = Vec::new();
        loop {
            let star_span = self.peek_span();
            let is_caret =
                !matches!(self.peek(), TokenKind::Star) && matches!(self.peek(), TokenKind::Caret);
            if !self.consume_if(&TokenKind::Star) && !self.consume_if(&TokenKind::Caret) {
                break;
            }
            *ptr_budget += 1;
            if !self.enter_pointer_level(*ptr_budget, star_span) {
                break;
            }
            let saved_const = self.attrs.parsing_const();
            let saved_volatile = self.attrs.parsing_volatile();
            let saved_space = self.attrs.parsing_address_space;
            let saved_pointer_const = self.attrs.parsing_pointer_const();
            self.attrs.set_const(false);
            self.attrs.set_volatile(false);
            self.attrs.set_pointer_const(false);
            self.attrs.parsing_address_space = AddressSpace::Default;

            let q_start = self.pos;
            self.skip_cv_qualifiers();
            let restrict_bit = u8::from(
                self.tokens[q_start..self.pos]
                    .iter()
                    .any(|tok| matches!(tok.kind, TokenKind::Restrict)),
            ) << 2;
            // `skip_cv_qualifiers` records a star's `const` in
            // `pointer_const` (see its doc), not in the base-specifier
            // `const` flag: reading `parsing_const()` here silently dropped
            // every type-name `*const`, so `*(int *const *)p = 0` passed.
            let star_cvr = u8::from(self.attrs.parsing_pointer_const())
                | (u8::from(self.attrs.parsing_volatile()) << 1)
                | restrict_bit;
            let star_space = self.attrs.parsing_address_space;

            self.attrs.set_const(saved_const);
            self.attrs.set_volatile(saved_volatile);
            self.attrs.set_pointer_const(saved_pointer_const);
            self.attrs.parsing_address_space = saved_space;

            self.skip_gcc_extensions();
            if is_caret {
                // Apple block pointer inside type-name may optionally carry an unused name
                if matches!(self.peek(), TokenKind::Identifier(_)) {
                    self.advance();
                }
            }
            stars.push((star_space, star_cvr));
        }

        let core = self
            .try_parse_grouped_abstract_declarator(ptr_budget)
            .map(Box::new);

        let mut suffixes = Vec::new();
        let mut prev_suffix_is_function: Option<bool> = None;
        loop {
            match self.peek() {
                TokenKind::LBracket if !self.is_c23_attr_start() => {
                    let open_bracket = self.peek_span();
                    self.check_suffix_composition(
                        prev_suffix_is_function,
                        false,
                        None,
                        open_bracket,
                    );
                    prev_suffix_is_function = Some(false);
                    self.advance();
                    self.skip_array_qualifiers();
                    let size = if matches!(self.peek(), TokenKind::RBracket) {
                        None
                    } else if matches!(self.peek(), TokenKind::Star)
                        && self.pos + 1 < self.tokens.len()
                        && matches!(self.tokens[self.pos + 1].kind, TokenKind::RBracket)
                    {
                        self.advance();
                        None
                    } else {
                        Some(Box::new(self.parse_expr()))
                    };
                    self.expect_closing(&TokenKind::RBracket, open_bracket);
                    suffixes.push(AbstractDeclSuffix::Array(size));
                }
                TokenKind::LParen => {
                    // When no grouped core preceded `(`, distinguish a bare-function
                    // parameter list (`void (int)`, `int (void)`, `int ()`) from an
                    // expression call argument list (`(T(x))`) during speculative
                    // cast lookahead by requiring `)`, `...`, or a type specifier.
                    let save = self.pos;
                    let saved_attrs = self.attrs.clone();
                    self.advance();
                    let looks_like_param_list = core.is_some()
                        || matches!(self.peek(), TokenKind::RParen | TokenKind::Ellipsis)
                        || self.is_type_specifier();
                    self.pos = save;
                    self.attrs = saved_attrs;
                    if !looks_like_param_list {
                        break;
                    }
                    let open_paren = self.peek_span();
                    self.check_suffix_composition(prev_suffix_is_function, true, None, open_paren);
                    prev_suffix_is_function = Some(true);
                    let (params, variadic, has_prototype) = self.parse_param_list();
                    suffixes.push(AbstractDeclSuffix::Function(
                        params,
                        variadic,
                        has_prototype,
                    ));
                }
                _ => break,
            }
        }

        AbstractDeclaratorLevel {
            stars,
            core,
            suffixes,
        }
    }

    /// Try to parse a parenthesized abstract declarator `( abstract-declarator )`
    /// as opposed to a function parameter list `( parameter-type-list )`.
    /// Restores parser position and attribute state if not a grouped abstract declarator.
    fn try_parse_grouped_abstract_declarator(
        &mut self,
        ptr_budget: &mut u32,
    ) -> Option<AbstractDeclaratorLevel> {
        if !matches!(self.peek(), TokenKind::LParen) {
            return None;
        }
        let open_span = self.peek_span();
        if !self.enter_record_frame(open_span) {
            return None;
        }
        let save_pos = self.pos;
        let save_attrs = self.attrs.clone();
        let save_budget = *ptr_budget;

        self.advance(); // consume '('
        self.skip_gcc_extensions();

        // A parameter list begins with `)`, `...`, or a declaration-specifier
        // (`void`, `int`, typedef name, qualifier, etc.), whereas a grouped
        // abstract declarator begins with `*`, `^`, `[`, or a nested grouped `(`.
        if matches!(self.peek(), TokenKind::RParen | TokenKind::Ellipsis)
            || self.is_type_specifier()
        {
            self.pos = save_pos;
            self.attrs = save_attrs;
            self.exit_parser_frame();
            return None;
        }

        let inner = self.parse_abstract_declarator_level(ptr_budget);
        if !inner.is_empty() && self.consume_if(&TokenKind::RParen) {
            self.exit_parser_frame();
            Some(inner)
        } else {
            self.pos = save_pos;
            self.attrs = save_attrs;
            *ptr_budget = save_budget;
            self.exit_parser_frame();
            None
        }
    }
}
