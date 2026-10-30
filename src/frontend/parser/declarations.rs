// Declaration parsing: external (top-level) and local (block-scope) declarations.
//
// External declarations handle both function definitions and variable/type
// declarations. The key challenge is disambiguating function definitions
// (which have a body) from declarations (which end with ';').
//
// K&R-style function parameters are also handled here, where parameter types
// are declared separately after the parameter name list.

use super::ast::*;
use super::parse::{ModeKind, ParsedDeclAttrs, Parser};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::AddressSpace;
use crate::frontend::lexer::token::TokenKind;

/// Context for declaration-level attributes that flow from `parse_external_decl`
/// into `parse_declaration_rest` and `parse_function_def`.
///
/// This groups the per-declarator GCC attributes, alignment info, and the
/// `__attribute__((common))` flag that would otherwise be passed as 15+
/// individual parameters.
struct DeclContext {
    /// Per-declarator attributes (constructor, destructor, weak, etc.)
    attrs: DeclAttributes,
    /// Combined alignment from `_Alignas`, declarator attrs, and post-declarator attrs
    alignment: Option<usize>,
    /// `_Alignas(type)` for deferred alignment resolution
    alignas_type: Option<TypeSpecifier>,
    /// `__attribute__((aligned(sizeof(type))))` for deferred sizeof
    alignment_sizeof_type: Option<TypeSpecifier>,
    /// `__attribute__((common))` - emit as common symbol
    is_common: bool,
    /// Visibility / TLS model written in the declaration SPECIFIERS: they
    /// apply to every declarator of the declaration, unlike the ones written
    /// after a declarator (see `parse_external_decl`).
    spec_visibility: Option<String>,
    spec_tls_model: Option<crate::common::tls::TlsModel>,
    spec_noinline: bool,
}

/// Parser-side mirror of `common::type_builder::apply_declaration_address_space`
/// (the two must stay in step). The named space goes to the innermost pointer:
/// the walk descends through pointers, arrays and function return types. A
/// function-pointer node (`FunctionPointer`) is itself a generic pointer, so
/// the walk continues into its return type, as for `struct c __seg_gs *(*f)(void)`.
/// Parser mirror of `common::type_builder::pointee_reaches_pointer_level`.
fn spec_reaches_pointer_level(ts: &TypeSpecifier) -> bool {
    match ts {
        TypeSpecifier::Pointer(..)
        | TypeSpecifier::FunctionPointer(..)
        | TypeSpecifier::BareFunction(..) => true,
        TypeSpecifier::Array(elem, _) => spec_reaches_pointer_level(elem),
        _ => false,
    }
}

fn apply_decl_address_space_to_spec(ts: &mut TypeSpecifier, space: AddressSpace) {
    if space == AddressSpace::Default {
        return;
    }
    match ts {
        TypeSpecifier::Pointer(inner, sp) => {
            if spec_reaches_pointer_level(inner) {
                apply_decl_address_space_to_spec(inner, space);
            } else {
                *sp = space;
            }
        }
        TypeSpecifier::Array(elem, _) => apply_decl_address_space_to_spec(elem, space),
        TypeSpecifier::FunctionPointer(ret, ..) | TypeSpecifier::BareFunction(ret, ..) => {
            apply_decl_address_space_to_spec(ret, space)
        }
        _ => {}
    }
}

impl Parser {
    /// The declaration omitted a function return type. Classification and
    /// dialect-dependent promotion are owned by the diagnostic engine.
    fn implicit_function_return_type(&mut self, span: Span) -> TypeSpecifier {
        self.diagnostics.warning_with_kind(
            "return type defaults to 'int'",
            span,
            crate::common::error::WarningKind::ImplicitInt,
        );
        TypeSpecifier::Int
    }

    /// Finish a file-level bare type declaration such as `struct S { ... };`.
    /// The transparent-union attribute belongs to this declaration alone and
    /// must not leak into the next one.
    fn parse_external_bare_type(&mut self, type_spec: TypeSpecifier, start: Span) -> ExternalDecl {
        self.consume_if(&TokenKind::Semicolon);
        let is_transparent_union = self.take_transparent_union();
        let mut d = Declaration::new(
            type_spec,
            Vec::new(),
            None,
            None,
            None,
            self.attrs.decl_address_space,
            self.attrs.parsing_vector_size.take(),
            self.attrs.parsing_ext_vector_nelem.take(),
            start,
        );
        d.set_vector_size_expr(self.attrs.parsing_vector_size_expr.take());
        d.set_static(self.attrs.parsing_static());
        d.set_extern(self.attrs.parsing_extern());
        d.set_typedef(self.attrs.parsing_typedef());
        d.set_const(self.attrs.parsing_const());
        d.set_constexpr(self.attrs.parsing_constexpr());
        d.set_constexpr_last(self.attrs.parsing_constexpr_last());
        d.set_volatile(self.attrs.parsing_volatile());
        d.set_pointer_const(self.attrs.parsing_pointer_const());
        d.set_thread_local(self.attrs.parsing_thread_local());
        d.set_transparent_union(is_transparent_union);
        ExternalDecl::Declaration(d)
    }

    pub(super) fn parse_external_decl(&mut self) -> Option<ExternalDecl> {
        self.attrs.parsing_attr_targets.clear();
        // Reset all declaration-level flags before parsing the next declaration.
        self.attrs = ParsedDeclAttrs::default();

        self.skip_gcc_extensions();
        // C23 attributes may precede a file-scope declaration/definition just
        // as they do a block-scope declaration.  The statement parser already
        // consumed this syntax, but the external-declaration path did not,
        // leaving `[[gnu::noipa]] int f(...)` to be parsed as an expression.
        self.skip_c23_attribute_lists();

        // Handle #pragma pack directives (emitted as synthetic tokens by preprocessor)
        while self.handle_pragma_pack_token() {
            // Consume semicolons after pragma pack synthetic tokens
            self.consume_if(&TokenKind::Semicolon);
        }

        // Handle #pragma GCC visibility push/pop directives
        while self.handle_pragma_visibility_token() {
            self.consume_if(&TokenKind::Semicolon);
        }

        if self.at_eof() {
            return None;
        }

        // Handle top-level asm("..."); directives - emit verbatim in assembly output
        if matches!(self.peek(), TokenKind::Asm) {
            return Some(self.parse_toplevel_asm());
        }

        // Handle _Static_assert at file scope
        if matches!(self.peek(), TokenKind::StaticAssert) {
            return Some(ExternalDecl::StaticAssert(self.parse_static_assert()));
        }

        let start = self.peek_span();
        let type_spec = self.parse_type_specifier().or_else(|| {
            // C89 implicit int: if no type specifier is found, check if this looks like
            // a function definition with implicit int return type (e.g., "main() { ... }"
            // or K&R-style "Proc_1(x) int x; { ... }").
            // The pattern is: identifier followed by '('.
            if let TokenKind::Identifier(_) = self.peek() {
                if self.pos + 1 < self.tokens.len()
                    && matches!(self.tokens[self.pos + 1].kind, TokenKind::LParen)
                {
                    return Some(self.implicit_function_return_type(start));
                }
            }
            None
        })?;
        // Capture the declaration's named address space before any declarator
        // (parameter lists in particular) is parsed; see ParsedDeclAttrs.
        self.attrs.decl_address_space = std::mem::take(&mut self.attrs.parsing_address_space);

        // Capture constructor/destructor from type-level attributes
        let type_level_ctor = self.attrs.parsing_constructor();
        let type_level_dtor = self.attrs.parsing_destructor();
        // Attributes in the declaration specifiers apply to EVERY declarator:
        // `__attribute__((visibility("hidden"))) int a, b;` hides both, while
        // `int c __attribute__((visibility("hidden"))), d;` hides only `c`
        // (GCC 16.2, measured). Snapshot the specifier-level values before the
        // first declarator's own attributes are merged in and taken.
        let spec_visibility = self.attrs.parsing_visibility.clone();
        let spec_tls_model = self.attrs.parsing_tls_model;
        let spec_noinline = self.attrs.parsing_noinline();

        // Bare type with no declarator (e.g., struct definition)
        if self.at_eof() || matches!(self.peek(), TokenKind::Semicolon) {
            return Some(self.parse_external_bare_type(type_spec, start));
        }

        // Handle post-type storage class specifiers (C allows "struct S typedef name;")
        self.consume_post_type_qualifiers();
        // Specifier-position applicability records are settled here, before
        // the declarator (parameters, post-declarator attributes) can add
        // unrelated entries to the pool.
        let mut spec_attr_targets = std::mem::take(&mut self.attrs.parsing_attr_targets);

        let (name, derived, decl_mode, decl_common, decl_aligned, _) =
            self.parse_declarator_with_attrs();
        let declarator_object_space = self.attrs.declarator_object_space;
        let declarator_object_volatile = self.attrs.declarator_object_volatile;
        let declarator_pointer_const = self.attrs.parsing_pointer_const();
        let declarator_object_quals = (
            self.attrs.declarator_object_restrict,
            self.attrs.declarator_object_atomic,
        );
        let declarator_base_volatile = self.attrs.declarator_base_volatile;
        let declarator_base_restrict = self.attrs.declarator_base_restrict;
        let (post_ctor, post_dtor, post_mode, post_common, post_aligned, first_asm_reg) =
            self.parse_asm_and_attributes();
        // Applicability is a property of the declaration, not of where the
        // attribute was written: `int x __attribute__((noreturn));` is the
        // same mistake as `__attribute__((noreturn)) int x;`, and GCC
        // diagnoses both. Every position feeds one pool, and the verdict is
        // taken once the declarator kind is known.
        spec_attr_targets.extend(std::mem::take(&mut self.attrs.parsing_attr_targets));
        self.check_attribute_targets(&type_spec, &derived, &spec_attr_targets, start);
        // `cleanup` names a function to run when a *block* scope ends. A
        // file-scope object never ends one, so GCC ignores the attribute
        // there (measured: `'cleanup' attribute ignored`); the generic
        // object/function rule cannot see the difference.
        // Same location rule as the kind check above: GCC reports at the
        // declaration, a leading attribute list included.
        let decl_loc = match self.attrs.first_attr_span {
            Some(attr) if attr.start < start.start => attr,
            _ => start,
        };
        // A `cleanup` list with the wrong number of arguments is an error and
        // gcc reports only that error, so the applicability warning is
        // suppressed for it (measured).
        let cleanup_arity_error = self.cleanup_diagnoses.iter().any(|(d, _)| {
            matches!(
                d,
                crate::frontend::parser::parse::CleanupDiagnosis::WrongArgumentCount(_)
            )
        });
        for (attr_name, _, _) in &spec_attr_targets {
            if attr_name == "cleanup" && !cleanup_arity_error {
                self.diagnostics.warning_with_kind(
                    "'cleanup' attribute ignored",
                    decl_loc,
                    crate::common::error::WarningKind::Attributes,
                );
            }
        }
        let mode_kind = decl_mode.or(post_mode);
        let is_common = decl_common || post_common;
        // Merge alignment from _Alignas (type specifier), declarator attrs, and post-declarator attrs.
        // Per C11, alignment can only increase (6.7.5), so take the maximum.
        let mut merged_alignment = self.attrs.parsed_alignas.take();
        let alignas_type = self.attrs.parsed_alignas_type.take();
        let alignment_sizeof_type = self.attrs.parsed_alignment_sizeof_type.take();
        for a in [decl_aligned, post_aligned].iter().copied().flatten() {
            merged_alignment = Some(merged_alignment.map_or(a, |prev| prev.max(a)));
        }
        // Merge all sources of constructor/destructor: type-level attrs, declarator-level attrs, post-declarator attrs
        let is_constructor = type_level_ctor || self.attrs.parsing_constructor() || post_ctor;
        let is_destructor = type_level_dtor || self.attrs.parsing_destructor() || post_dtor;
        // Capture alias/ifunc/weak/visibility/section/error attributes
        let is_weak = self.attrs.parsing_weak();
        let alias_target = self.attrs.parsing_alias_target.take();
        let ifunc_target = self.attrs.parsing_ifunc_target.take();
        // Use explicit __attribute__((visibility(...))) if present, otherwise fall back
        // to the current #pragma GCC visibility default (if any).
        let visibility = self
            .attrs
            .parsing_visibility
            .take()
            .or_else(|| self.pragma_default_visibility.clone());
        let section = self.attrs.parsing_section.take();
        let tls_model = self.attrs.parsing_tls_model.take();
        let is_error_attr = self.attrs.parsing_error_attr();
        let is_noreturn = self.attrs.parsing_noreturn();
        let cleanup_fn = self.attrs.parsing_cleanup_fn.take();
        let symver = self.attrs.parsing_symver.take();
        let target = self.attrs.parsing_target.take();
        let target_clones = self.attrs.parsing_target_clones.take();
        let is_uninitialized = self.attrs.parsing_uninitialized();
        self.attrs.set_uninitialized(false);
        let diagnose_as = self.attrs.parsing_diagnose_as.take();
        let is_used = self.attrs.parsing_used();
        let is_maybe_unused = self.attrs.parsing_maybe_unused();
        let is_fastcall = self.attrs.parsing_fastcall();
        let regparm = self.attrs.parsing_regparm;
        let is_naked = self.attrs.parsing_naked();
        let no_instrument = self.attrs.parsing_no_instrument();
        let is_noinline = self.attrs.parsing_noinline();
        let is_pure = self.attrs.parsing_pure();
        let is_const_attr = self.attrs.parsing_const_attr();
        let is_cold = self.attrs.parsing_cold();
        let is_stack_protect = self.attrs.parsing_stack_protect;
        self.attrs.parsing_stack_protect = false;
        let is_no_stack_protector = self.attrs.parsing_no_stack_protector;
        self.attrs.parsing_no_stack_protector = false;
        let externally_visible = self.attrs.parsing_externally_visible;
        self.attrs.parsing_externally_visible = false;
        let zero_call_used_regs = self.attrs.parsing_zero_call_used_regs.take();
        let no_sanitize = self.attrs.parsing_no_sanitize.take();

        // Build per-declarator attributes struct from the collected flags
        let mut decl_attrs = DeclAttributes::default();
        decl_attrs.parsing_attr_targets = spec_attr_targets;
        decl_attrs.object_space = declarator_object_space;
        decl_attrs.object_volatile = declarator_object_volatile;
        decl_attrs.pointer_const = declarator_pointer_const;
        (decl_attrs.object_restrict, decl_attrs.object_atomic) = declarator_object_quals;
        decl_attrs.base_volatile = declarator_base_volatile;
        decl_attrs.base_restrict = declarator_base_restrict;
        decl_attrs.set_constructor(is_constructor);
        decl_attrs.set_destructor(is_destructor);
        decl_attrs.set_weak(is_weak);
        decl_attrs.set_error_attr(is_error_attr);
        decl_attrs.set_noreturn(is_noreturn);
        decl_attrs.set_used(is_used);
        decl_attrs.set_maybe_unused(is_maybe_unused);
        decl_attrs.set_fastcall(is_fastcall);
        decl_attrs.regparm = regparm;
        decl_attrs.set_naked(is_naked);
        decl_attrs.set_no_instrument(no_instrument);
        decl_attrs.set_pure(is_pure);
        decl_attrs.set_const_attr(is_const_attr);
        decl_attrs.set_cold(is_cold);
        decl_attrs.set_uninitialized(is_uninitialized);
        decl_attrs.set_stack_protect(is_stack_protect);
        decl_attrs.set_no_stack_protector(is_no_stack_protector);
        decl_attrs.set_retain(self.attrs.parsing_retain());
        decl_attrs.set_noplt(self.attrs.parsing_noplt());
        decl_attrs.set_noinit(self.attrs.parsing_noinit());
        decl_attrs.set_persistent(self.attrs.parsing_persistent());
        decl_attrs.set_nocommon(self.attrs.parsing_nocommon());
        decl_attrs.set_nested_fn(self.attrs.parsing_nested_fn());
        decl_attrs.alias_target = alias_target;
        decl_attrs.ifunc_target = ifunc_target;
        decl_attrs.visibility = visibility;
        decl_attrs.tls_model = tls_model;
        decl_attrs.section = section;
        decl_attrs.tls_model = tls_model;
        decl_attrs.asm_register = first_asm_reg;
        decl_attrs.cleanup_fn = cleanup_fn;
        decl_attrs.symver = symver;
        decl_attrs.target = target;
        decl_attrs.target_clones = target_clones;
        decl_attrs.diagnose_as = diagnose_as;
        decl_attrs.externally_visible = externally_visible;
        decl_attrs.zero_call_used_regs = zero_call_used_regs;
        decl_attrs.no_sanitize = no_sanitize;

        // Apply __attribute__((mode(...))): transform type to specified bit-width
        let type_spec = if let Some(mk) = mode_kind {
            mk.apply(type_spec)
        } else {
            type_spec
        };

        // Determine if this is a function definition
        let is_funcdef = !derived.is_empty()
            && matches!(derived.last(), Some(DerivedDeclarator::Function(..)))
            && (matches!(self.peek(), TokenKind::LBrace) || self.is_type_specifier());

        // S13: record file-scope function no_instrument_function
        // attributes so a later DEFINITION inherits them (GCC merges
        // declaration attributes into the definition).
        if let Some(n) = &name {
            if matches!(derived.last(), Some(DerivedDeclarator::Function(..))) {
                if decl_attrs.is_no_instrument() {
                    self.func_decl_no_instrument.insert(n.clone());
                }
                if decl_attrs.is_stack_protect() {
                    self.func_decl_stack_protect.insert(n.clone());
                }
                if decl_attrs.is_no_stack_protector() {
                    self.func_decl_no_stack_protector.insert(n.clone());
                }
            }
        }
        // `noinline` is a function property, not a one-declaration parsing
        // hint.  Preserve it from a prior prototype through a later bare
        // definition just like no_instrument_function above.
        self.record_function_entity(name.as_deref(), &derived, is_noinline);

        if is_funcdef {
            self.parse_function_def(
                type_spec,
                name,
                derived,
                start,
                decl_attrs,
                merged_alignment,
            )
        } else {
            let ctx = DeclContext {
                attrs: decl_attrs,
                alignment: merged_alignment,
                alignas_type,
                alignment_sizeof_type,
                is_common,
                spec_visibility,
                spec_tls_model,
                spec_noinline,
            };
            self.parse_declaration_rest(type_spec, name, derived, start, ctx)
        }
    }

    /// `asm("...")` / `asm volatile ("...")` / `asm("...");` at file scope.
    ///
    /// Top-level asm is emitted verbatim, so the string pieces are concatenated
    /// here and never parsed as C. An empty directive still consumes the
    /// declaration: `asm("");` yields an empty declaration rather than falling
    /// through to the declarator parser (measured: gcc-16.2 accepts it).
    fn parse_toplevel_asm(&mut self) -> ExternalDecl {
        self.advance();
        self.consume_if(&TokenKind::Volatile);
        if matches!(self.peek(), TokenKind::LParen) {
            self.advance(); // consume (
            // Collect all string literal pieces (may be concatenated)
            let mut asm_str = String::new();
            loop {
                match self.peek() {
                    TokenKind::StringLiteral(s) => {
                        asm_str.push_str(s);
                        self.advance();
                    }
                    TokenKind::RParen | TokenKind::Eof => break,
                    _ => {
                        self.advance();
                    }
                }
            }
            if matches!(self.peek(), TokenKind::RParen) {
                self.advance();
            }
            self.consume_if(&TokenKind::Semicolon);
            if !asm_str.is_empty() {
                return ExternalDecl::TopLevelAsm(asm_str);
            }
            return ExternalDecl::Declaration(Declaration::empty());
        }
        self.consume_if(&TokenKind::Semicolon);
        ExternalDecl::Declaration(Declaration::empty())
    }

    /// Record facts about a FUNCTION entity, not a pointer object. Call this
    /// for every declarator, after its own trailing attributes are consumed.
    ///
    /// Two facts ride on the same shape test (`DerivedDeclarator::Function` last
    /// means the declared entity is a function, not a pointer to one):
    ///   * `noinline`, preserved from a prototype through a later bare
    ///     definition (GCC merges declaration attributes into the definition);
    ///   * the name itself, so an `asm "i"(f)` operand can be recognised as a
    ///     link-time constant (`function_names`, see `asm_validate`).
    fn record_function_entity(
        &mut self,
        name: Option<&str>,
        derived: &[DerivedDeclarator],
        noinline: bool,
    ) {
        if !matches!(derived.last(), Some(DerivedDeclarator::Function(..))) {
            return;
        }
        if let Some(name) = name {
            self.function_names.insert(name.to_owned());
            if noinline {
                self.func_decl_noinline.insert(name.to_owned());
            }
        }
    }

    /// An OBJECT declaration hides any same-named function: after
    /// `void f(void);` a block-scope `int f;` makes `"i"(f)` an ordinary
    /// object value, exactly as it hides a typedef name.  A declaration whose
    /// last declarator is a function still names the function entity (a
    /// redeclaration or definition), so it must not clear the record.
    fn record_object_shadows(&mut self, declarators: &[InitDeclarator]) {
        for decl in declarators {
            if !decl.name.is_empty()
                && !matches!(decl.derived.last(), Some(DerivedDeclarator::Function(..)))
            {
                self.function_names.remove(&decl.name);
            }
        }
    }

    /// Parse the rest of a function definition after the declarator.
    fn parse_function_def(
        &mut self,
        type_spec: TypeSpecifier,
        name: Option<String>,
        derived: Vec<DerivedDeclarator>,
        start: crate::common::source::Span,
        mut decl_attrs: DeclAttributes,
        alignment: Option<usize>,
    ) -> Option<ExternalDecl> {
        // The specifier-position applicability records ride `decl_attrs`
        // from the spec parse; the kind check takes them.
        let recorded = std::mem::take(&mut decl_attrs.parsing_attr_targets);
        self.check_attribute_targets(&type_spec, &derived, &recorded, start);
        self.attrs.set_typedef(false); // function defs are never typedefs
        // Two distinct anchors, both measured against gcc-16.2: a function
        // *definition* reports an unprototyped declarator at its name
        // (`int k() {}` -> `1:5`), while a declaration reports it at the
        // declaration's start (`int (*cb)();` -> `1:1`). The name must be
        // captured here: K&R parameter declarations and the body both parse
        // further declarations, and each of those overwrites the field.
        let def_name_span = self.attrs.last_decl_name_span;
        let (params, variadic, has_prototype) =
            if let Some(DerivedDeclarator::Function(p, v, has_prototype)) = derived.last() {
                (p.clone(), *v, *has_prototype)
            } else {
                (vec![], false, false)
            };

        // Handle K&R-style parameter declarations
        let is_kr_style = !matches!(self.peek(), TokenKind::LBrace);
        let final_params = if is_kr_style {
            self.parse_kr_params(params)
        } else {
            params
        };

        let is_static = self.attrs.parsing_static();
        let is_inline = self.attrs.parsing_inline();
        let is_extern = self.attrs.parsing_extern();
        let return_base_const = self.attrs.parsing_const();
        let is_gnu_inline = self.attrs.parsing_gnu_inline();
        let is_always_inline = self.attrs.parsing_always_inline();
        let is_noinline = self.attrs.parsing_noinline()
            || name
                .as_deref()
                .is_some_and(|n| self.func_decl_noinline.contains(n));

        // Build return type from derived declarators
        let mut return_type = self.build_return_type(type_spec, &derived);
        // `struct c __seg_gs *f(void) { ... }`: the declaration-level qualifier
        // qualifies the return type's innermost pointer, as it does for
        // prototypes (sema applies it to CType::Function's return type).
        apply_decl_address_space_to_spec(&mut return_type, self.attrs.decl_address_space);

        // Shadow typedef names used as parameter names, and hide enumerators
        // of the same name until the body ends.
        let enum_mark = self.enum_scope_log.len();
        let mut shadowed_added: Vec<String> = Vec::new();
        for param in &final_params {
            if let Some(ref pname) = param.name {
                if self.is_parser_enum_constant(pname) {
                    self.hide_enum_constant(pname);
                }
                if self.typedefs.contains(pname) && !self.shadowed_typedefs.contains(pname) {
                    self.shadowed_typedefs.insert(pname.clone());
                    shadowed_added.push(pname.clone());
                }
            }
        }
        // Clear noreturn so that _Noreturn/noreturn from the function definition
        // does not leak into local declarations inside the body.
        // parse_compound_stmt saves/restores attr flags, so this is safe.
        self.attrs.set_noreturn(false);
        // S13: GCC merges declaration attributes into the
        // definition; no_instrument_function on a prior file-scope
        // declaration (e.g. `int main () NOCHK;`) must suppress
        // instrumentation even when the definition does not re-state it.
        let mut decl_attrs = decl_attrs;
        if let Some(n) = name.as_deref() {
            if self.func_decl_stack_protect.contains(n) {
                decl_attrs.set_stack_protect(true);
            }
            if self.func_decl_no_stack_protector.contains(n) {
                decl_attrs.set_no_stack_protector(true);
            }
        }
        let no_instrument_merged = decl_attrs.is_no_instrument()
            || name
                .as_deref()
                .is_some_and(|n| self.func_decl_no_instrument.contains(n));
        if decl_attrs.ifunc_target.is_some() {
            self.emit_error(
                format!(
                    "cannot define both an 'ifunc' resolver and a body for '{}'",
                    name.as_deref().unwrap_or("<anonymous>")
                ),
                start,
            );
        }
        let open = self.peek_span();
        // In an optimizing compile, a local initialized with a constant or
        // an inline parameter may become an immediate only after optimization.
        // Preserve the translation-unit's defer mode through this body; the
        // post-optimization IR check rejects any dynamic operand that remains.
        let prior_asm_immediate_defer = self.asm_immediate_validation_deferred;
        self.asm_immediate_validation_deferred =
            prior_asm_immediate_defer || is_inline || is_always_inline;
        let saved_asm_deferral = self.asm_deferred_instantiation;
        self.asm_deferred_instantiation =
            saved_asm_deferral || is_inline || is_always_inline || is_gnu_inline;
        let body = self.parse_compound_stmt();
        self.asm_deferred_instantiation = saved_asm_deferral;
        self.asm_immediate_validation_deferred = prior_asm_immediate_defer;
        let body_span = self.through_previous_token(open);
        for n in shadowed_added {
            self.shadowed_typedefs.remove(&n);
        }
        self.restore_enum_scope(enum_mark);

        self.flush_declarator_verdicts(
            start,
            def_name_span,
            DerivedDeclarator::has_unprototyped_function_type(&derived),
            false,
        );
        Some(ExternalDecl::FunctionDef(FunctionDef {
            alignment,
            return_base_const,
            return_pointer_const_levels: crate::frontend::parser::ast::return_pointer_const_levels(
                &return_type,
                &derived,
            ),
            return_base_cvr: u8::from(return_base_const) * crate::frontend::parser::ast::CVR_CONST
                | u8::from(decl_attrs.base_volatile) * crate::frontend::parser::ast::CVR_VOLATILE
                | u8::from(decl_attrs.base_restrict) * crate::frontend::parser::ast::CVR_RESTRICT,
            return_star_cvr: crate::frontend::parser::ast::return_star_chain(
                &return_type,
                &derived,
            ),
            return_type,
            name: name.unwrap_or_default(),
            params: final_params,
            variadic,
            has_prototype: has_prototype && !is_kr_style,
            body,
            attrs: FunctionAttributes::for_definition(
                &decl_attrs,
                DefSpecifiers {
                    static_: is_static,
                    inline_: is_inline,
                    extern_: is_extern,
                    gnu_inline: is_gnu_inline,
                    always_inline: is_always_inline,
                    noinline: is_noinline,
                },
                no_instrument_merged,
            ),
            is_kr: is_kr_style,
            span: start,
            body_span,
        }))
    }

    /// Build the return type from derived declarators.
    /// For `int (*func())[3]`, we apply post-Function and pre-Function derivations.
    pub(super) fn build_return_type(
        &self,
        base_type: TypeSpecifier,
        derived: &[DerivedDeclarator],
    ) -> TypeSpecifier {
        let mut return_type = base_type;
        let func_pos = derived
            .iter()
            .position(|d| matches!(d, DerivedDeclarator::Function(..)));

        if let Some(fpos) = func_pos {
            // Apply post-Function derivations (Array/Pointer). Space mapping
            // follows pointee_space_before: each wrapper takes the previous
            // entry's space (a Function entry has none, so the first wrapper
            // after it takes Default).
            let mut pending_space = AddressSpace::Default;
            for d in &derived[fpos + 1..] {
                match d {
                    DerivedDeclarator::Array(size_expr) => {
                        return_type =
                            TypeSpecifier::Array(Box::new(return_type), size_expr.clone());
                    }
                    DerivedDeclarator::Pointer(space, _) => {
                        return_type = TypeSpecifier::Pointer(Box::new(return_type), pending_space);
                        pending_space = *space;
                    }
                    _ => {}
                }
            }
            // Apply pre-Function derivations.
            //
            // A (Pointer, FunctionPointer) pair — or a standalone FunctionPointer —
            // in the prefix is the "returns a function pointer" core produced by
            // `combine_declarator_parts` for declarators like
            // `int (*f(int))(const void*, int)`. It must become a
            // TypeSpecifier::FunctionPointer wrapping the return type built so
            // far (TypeSpecifier::FunctionPointer already denotes
            // Pointer(Function(...))). Silently skipping it (the old `_ => {}`)
            // truncated such return types to `int *`.
            let prefix = &derived[..fpos];
            let mut i = 0;
            while i < prefix.len() {
                match &prefix[i] {
                    DerivedDeclarator::Pointer(..) => {
                        if let Some(DerivedDeclarator::FunctionPointer(
                            params,
                            variadic,
                            has_prototype,
                        )) = prefix.get(i + 1)
                        {
                            return_type = TypeSpecifier::FunctionPointer(
                                Box::new(return_type),
                                params.clone(),
                                *variadic,
                                *has_prototype,
                            );
                            i += 2;
                        } else {
                            // Each star keeps its own address space, by the same
                            // pointee_space_before rule every other declarator
                            // path uses.  Hardcoding Default here is what made
                            // `T * __seg_gs *f(void)` return a Default-space
                            // pointer: the kernel's `struct perf_event *
                            // __percpu *` returns were then unassignable from
                            // their own casts (kernel/events/hw_breakpoint.c),
                            // while the identical spelling on a parameter, a
                            // variable or a member kept %gs.
                            return_type = TypeSpecifier::Pointer(
                                Box::new(return_type),
                                crate::common::type_builder::pointee_space_before(derived, i),
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
                    DerivedDeclarator::Array(size_expr) => {
                        return_type =
                            TypeSpecifier::Array(Box::new(return_type), size_expr.clone());
                        i += 1;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
        } else {
            // No Function in derived - just apply pointer derivations, with
            // the same pointee_space_before mapping as build_full_ctype.
            let mut pending_space = AddressSpace::Default;
            for d in derived {
                match d {
                    DerivedDeclarator::Pointer(space, _) => {
                        return_type = TypeSpecifier::Pointer(Box::new(return_type), pending_space);
                        pending_space = *space;
                    }
                    _ => break,
                }
            }
        }
        return_type
    }

    /// Parse K&R-style parameter declarations.
    /// In K&R style, the parameter list is just names, and type declarations follow.
    pub(super) fn parse_kr_params(&mut self, mut kr_params: Vec<ParamDecl>) -> Vec<ParamDecl> {
        while self.is_type_specifier() && !matches!(self.peek(), TokenKind::LBrace) {
            if let Some(type_spec) = self.parse_type_specifier() {
                loop {
                    let (pname, pderived) = self.parse_declarator();
                    if let Some(ref name) = pname {
                        let (
                            full_type,
                            fptr_params,
                            fptr_variadic,
                            fptr_has_prototype,
                            decayed_array,
                        ) = self.apply_kr_derivations(&type_spec, &pderived);
                        // Compute fptr_inner_ptr_depth for K&R function pointer params.
                        // Count Pointer entries AFTER FunctionPointer in derived list;
                        // these represent extra indirection (pointer-to-function-pointer).
                        let inner_depth = if fptr_params.is_some() {
                            let mut found_fptr = false;
                            let mut ptrs_after = 0u32;
                            for d in &pderived {
                                match d {
                                    DerivedDeclarator::FunctionPointer(..)
                                    | DerivedDeclarator::Function(..) => {
                                        found_fptr = true;
                                    }
                                    DerivedDeclarator::Pointer(..) if found_fptr => {
                                        ptrs_after += 1;
                                    }
                                    _ => {}
                                }
                            }
                            // inner_ptr_depth = 1 (the (*) syntax star) + ptrs_after
                            1 + ptrs_after
                        } else {
                            0
                        };
                        for param in kr_params.iter_mut() {
                            if param.name.as_deref() == Some(name.as_str()) {
                                param.type_spec = full_type.clone();
                                param.fptr_params = fptr_params.clone();
                                param.fptr_variadic = fptr_variadic;
                                param.fptr_has_prototype = fptr_has_prototype;
                                param.fptr_inner_ptr_depth = inner_depth;
                                param.decayed_array = decayed_array;
                                break;
                            }
                        }
                    }
                    if !self.consume_if(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect_after(&TokenKind::Semicolon, "after parameter declaration");
            } else {
                break;
            }
        }
        kr_params
    }

    /// Apply derived declarators to build a K&R parameter's full type.
    /// Returns (type_specifier, optional_fptr_params, fptr_variadic, fptr_has_prototype).
    /// For function pointer parameters like `int (*fp)(int, ...)`, this retains
    /// both the fixed parameters and the variadic tail, matching modern syntax.
    fn apply_kr_derivations(
        &self,
        type_spec: &TypeSpecifier,
        pderived: &[DerivedDeclarator],
    ) -> (TypeSpecifier, Option<Vec<ParamDecl>>, bool, bool, bool) {
        let mut full_type = type_spec.clone();

        // Check if this is a function pointer parameter.
        // For `int (*fp)(int)`, pderived = [Pointer, FunctionPointer([int], false)]
        // The Pointer is the syntax marker from `(*fp)`, not a real pointer level.
        // We need to: (1) not apply the syntax-marker Pointer, (2) extract the
        // FunctionPointer params, and (3) apply one Pointer wrap for decay.
        let fptr_info = pderived.iter().find_map(|d| {
            if let DerivedDeclarator::FunctionPointer(params, variadic, has_prototype) = d {
                Some((params.clone(), *variadic, *has_prototype))
            } else {
                None
            }
        });

        if let Some((fptr_params, variadic, has_prototype)) = fptr_info {
            // Function pointer parameter in K&R style.
            // Count how many Pointer derivations exist. For `int (*fp)()`:
            //   pderived = [Pointer, FunctionPointer([], false)]
            //   One Pointer is the syntax marker - skip it.
            //   Any additional Pointers are return-type pointer levels.
            // For `int *(*fp)()`:
            //   pderived = [Pointer, Pointer, FunctionPointer([], false)]
            //   First Pointer is return-type pointer, second is syntax marker.
            // The syntax-marker star is the last Pointer in pderived; any
            // further Pointers are return-type levels. Drop the marker, then
            // apply the remaining stars with the pointee_space_before chain
            // (see fold_simple_derived): the wrapper built at step k takes
            // the space stored at the PREVIOUS star (the first takes
            // Default).
            let mut star_spaces = pderived
                .iter()
                .filter_map(|d| match d {
                    DerivedDeclarator::Pointer(space, _) => Some(*space),
                    _ => None,
                })
                .collect::<Vec<_>>();
            star_spaces.pop();
            let mut pending = AddressSpace::Default;
            for space in star_spaces {
                full_type = TypeSpecifier::Pointer(Box::new(full_type), pending);
                pending = space;
            }
            // The final application is the function-pointer-to-pointer decay
            // wrapper: the syntax-marker star's own level, whose space is the
            // previous star's (see pointee_space_before).
            full_type = TypeSpecifier::Pointer(Box::new(full_type), pending);
            return (full_type, Some(fptr_params), variadic, has_prototype, false);
        }

        // Not a function pointer - apply all derivations normally.
        // Apply pointers, keeping each star's own address space
        // (`struct sd * __seg_gs *` declares a pointer whose pointee values
        // live in %gs — the kernel's percpu `T * __percpu *` members).
        for d in pderived {
            if let DerivedDeclarator::Pointer(space, _) = d {
                full_type = TypeSpecifier::Pointer(Box::new(full_type), *space);
            }
        }
        // Collect array dimensions
        let array_dims: Vec<_> = pderived
            .iter()
            .filter_map(|d| {
                if let DerivedDeclarator::Array(size) = d {
                    Some(size.clone())
                } else {
                    None
                }
            })
            .collect();
        // Array params: outermost dimension decays to pointer
        let decayed_array = !array_dims.is_empty();
        if decayed_array {
            for dim in array_dims.iter().skip(1).rev() {
                full_type = TypeSpecifier::Array(Box::new(full_type), dim.clone());
            }
            full_type = TypeSpecifier::Pointer(Box::new(full_type), AddressSpace::Default);
        }
        // Function params (bare function names) decay to pointers
        for d in pderived {
            if let DerivedDeclarator::Function(..) = d {
                full_type = TypeSpecifier::Pointer(Box::new(full_type), AddressSpace::Default);
            }
        }
        (full_type, None, false, false, decayed_array)
    }

    /// Finish the first declarator before parsing comma siblings. Values that
    /// belong to the declaration specifiers are restored from DeclContext.
    fn clear_completed_declarator_attributes(&mut self) {
        self.attrs.set_weak(false);
        self.attrs.first_attr_span = None;
        self.attrs.set_noplt(false);
        self.attrs.parsing_alias_target = None;
        self.attrs.parsing_ifunc_target = None;
        self.attrs.parsing_target_clones = None;
        self.attrs.parsing_externally_visible = false;
        self.attrs.parsing_zero_call_used_regs = None;
        self.attrs.parsing_no_sanitize = None;
        self.attrs.set_uninitialized(false);
        self.attrs.parsing_visibility = None;
        self.attrs.parsing_tls_model = None;
        self.attrs.parsing_section = None;
        self.attrs.set_error_attr(false);
        self.attrs.set_noreturn(false);
        self.attrs.set_nested_fn(false);
        self.attrs.set_pure(false);
        self.attrs.set_const_attr(false);
        self.attrs.set_cold(false);
        self.attrs.parsing_cleanup_fn = None;
        self.attrs.set_used(false);
        self.attrs.set_maybe_unused(false);
        self.attrs.set_fastcall(false);
        self.attrs.parsing_regparm = None;
        self.attrs.set_naked(false);
        self.attrs.set_no_instrument(false);
    }

    /// Parse the rest of a declaration (not a function definition).
    fn parse_declaration_rest(
        &mut self,
        type_spec: TypeSpecifier,
        name: Option<String>,
        derived: Vec<DerivedDeclarator>,
        start: crate::common::source::Span,
        mut ctx: DeclContext,
    ) -> Option<ExternalDecl> {
        // Placement-checked specifier attributes resolve against the first
        // declarator (the take also keeps records off later declarations).
        let recorded = std::mem::take(&mut ctx.attrs.parsing_attr_targets);
        // gcc anchors declaration-level verdicts at the declaration, and a
        // leading attribute list is part of it: `__attribute__((cleanup))
        // void *p;` anchors at the attribute, not at the type specifier
        // (measured). Taken here because finishing the first declarator
        // clears the record.
        let decl_loc = match self.attrs.first_attr_span {
            Some(attr) if attr.start < start.start => attr,
            _ => start,
        };
        self.check_attribute_targets(&type_spec, &derived, &recorded, start);
        let mut declarators = Vec::with_capacity(4);
        // Same snapshot rationale as parse_local_declaration: initializer
        // casts to segment-qualified pointers clear parsing_address_space
        // before Declaration::new would read it.
        let decl_addr_space = self.attrs.decl_address_space;
        let spec_maybe_unused = ctx.attrs.is_maybe_unused();
        // `noplt` written among the declaration specifiers applies to every
        // declarator of the statement (`__attribute__((noplt)) void f(), g();`
        // marks both, measured against gcc-16.2.0).
        let spec_noplt = ctx.attrs.is_noplt();
        let init = if self.consume_if(&TokenKind::Assign) {
            Some(self.parse_initializer())
        } else {
            None
        };
        let section = ctx.attrs.section.clone();
        declarators.push(InitDeclarator {
            name: name.unwrap_or_default(),
            derived,
            init,
            attrs: ctx.attrs,
            span: start,
        });

        let (extra_ctor, extra_dtor, _, extra_common, extra_aligned, extra_asm_reg) =
            self.parse_asm_and_attributes();
        // Merge post-declarator attributes into the most recently pushed declarator.
        // last_mut() is safe because we just pushed above.
        // Captured here because `last_decl` below borrows `declarators`
        // mutably, and the fan-out to every declarator has to wait until that
        // borrow ends. `auto` is a declaration-specifier, so it applies to each
        // declarator of the statement (`auto double f(double), h(double);`), not
        // just the last one.
        let nested_fn_decl_level = self.attrs.parsing_nested_fn();
        let Some(last_decl) = declarators.last_mut() else {
            return None;
        };
        if let Some(reg) = extra_asm_reg {
            last_decl.attrs.asm_register = Some(reg);
        }
        if extra_ctor {
            last_decl.attrs.set_constructor(true);
        }
        if extra_dtor {
            last_decl.attrs.set_destructor(true);
        }
        // Merge weak/alias/visibility from post-declarator attributes
        if self.attrs.parsing_weak() {
            last_decl.attrs.set_weak(true);
        }
        if let Some(ref target) = self.attrs.parsing_alias_target {
            last_decl.attrs.alias_target = Some(target.clone());
        }
        if let Some(ref target) = self.attrs.parsing_ifunc_target {
            last_decl.attrs.ifunc_target = Some(target.clone());
        }
        if let Some(ref tc) = self.attrs.parsing_target_clones {
            last_decl.attrs.target_clones = Some(tc.clone());
        }
        if self.attrs.parsing_externally_visible {
            last_decl.attrs.externally_visible = true;
        }
        if let Some(ref zr) = self.attrs.parsing_zero_call_used_regs {
            last_decl.attrs.zero_call_used_regs = Some(zr.clone());
        }
        if let Some(ref ns) = self.attrs.parsing_no_sanitize {
            last_decl.attrs.no_sanitize = Some(ns.clone());
        }
        if self.attrs.parsing_uninitialized() {
            last_decl.attrs.set_uninitialized(true);
        }
        if let Some(ref vis) = self.attrs.parsing_visibility {
            last_decl.attrs.visibility = Some(vis.clone());
        }
        if let Some(model) = self.attrs.parsing_tls_model.take() {
            last_decl.attrs.tls_model = Some(model);
        }
        if let Some(ref sect) = self.attrs.parsing_section {
            last_decl.attrs.section = Some(sect.clone());
        }
        if let Some(ref cleanup) = self.attrs.parsing_cleanup_fn {
            last_decl.attrs.cleanup_fn = Some(cleanup.clone());
        }
        if self.attrs.parsing_used() {
            last_decl.attrs.set_used(true);
        }
        // Object-placement attributes (GCC 16.2): carried on the declarator so
        // lowering can place the object and diagnose misuse.
        if self.attrs.parsing_retain() {
            last_decl.attrs.set_retain(true);
        }
        if self.attrs.parsing_noinit() {
            last_decl.attrs.set_noinit(true);
        }
        if self.attrs.parsing_persistent() {
            last_decl.attrs.set_persistent(true);
        }
        if self.attrs.parsing_nocommon() {
            last_decl.attrs.set_nocommon(true);
        }
        if self.attrs.parsing_maybe_unused() {
            last_decl.attrs.set_maybe_unused(true);
        }
        if self.attrs.parsing_noreturn() {
            last_decl.attrs.set_noreturn(true);
        }
        // The declaration-statement funnel covers every block-scope shape
        // (prototype-only, init-declarator, definition); the nested-function
        // fact must be carried here as well as in the file-scope and
        // init-declarator builders, or `auto double f(double);` inside a
        // function body is indistinguishable from `extern double f(double);`
        // and retypes the file-scope entity of the same name.
        if self.attrs.parsing_nested_fn() {
            last_decl.attrs.set_nested_fn(true);
        }
        if self.attrs.parsing_fastcall() {
            last_decl.attrs.set_fastcall(true);
        }
        if let Some(n) = self.attrs.parsing_regparm {
            last_decl.attrs.regparm = Some(n);
        }
        if self.attrs.parsing_naked() {
            last_decl.attrs.set_naked(true);
        }
        if self.attrs.parsing_no_instrument() {
            last_decl.attrs.set_no_instrument(true);
        }
        if self.attrs.parsing_pure() {
            last_decl.attrs.set_pure(true);
        }
        if self.attrs.parsing_const_attr() {
            last_decl.attrs.set_const_attr(true);
        }
        if self.attrs.parsing_cold() {
            last_decl.attrs.set_cold(true);
        }
        if self.attrs.parsing_error_attr() {
            last_decl.attrs.set_error_attr(true);
        }
        self.record_function_entity(
            Some(&last_decl.name),
            &last_decl.derived,
            self.attrs.parsing_noinline(),
        );
        self.attrs.set_noinline(false);

        if nested_fn_decl_level {
            for d in &mut declarators {
                d.attrs.set_nested_fn(true);
            }
        }
        self.clear_completed_declarator_attributes();
        ctx.is_common = ctx.is_common || extra_common;
        if let Some(a) = extra_aligned {
            ctx.alignment = Some(ctx.alignment.map_or(a, |prev| prev.max(a)));
        }

        // Parse additional declarators separated by commas
        while self.consume_if(&TokenKind::Comma) {
            // Specifier attributes apply to every sibling; attributes attached
            // to one declarator must not leak to the next one.
            self.attrs.set_noinline(ctx.spec_noinline);
            let (dname, dderived) = self.parse_declarator();
            let d_object_space = self.attrs.declarator_object_space;
            let d_object_volatile = self.attrs.declarator_object_volatile;
            let d_pointer_const = self.attrs.parsing_pointer_const();
            let d_object_quals = (
                self.attrs.declarator_object_restrict,
                self.attrs.declarator_object_atomic,
            );
            let d_base_volatile = self.attrs.declarator_base_volatile;
            let d_base_restrict = self.attrs.declarator_base_restrict;
            let (d_ctor, d_dtor, _, d_common, _, d_asm_reg) = self.parse_asm_and_attributes();
            ctx.is_common = ctx.is_common || d_common;
            let d_weak = self.attrs.parsing_weak();
            let d_alias = self.attrs.parsing_alias_target.take();
            let d_ifunc = self.attrs.parsing_ifunc_target.take();
            let d_vis = self
                .attrs
                .parsing_visibility
                .take()
                .or_else(|| ctx.spec_visibility.clone())
                .or_else(|| self.pragma_default_visibility.clone());
            let d_tls_model = self.attrs.parsing_tls_model.take().or(ctx.spec_tls_model);
            // Per-declarator section attribute overrides, otherwise inherit from
            // the declaration-level attribute (e.g. __attribute__((section(".x"))) int a, b;)
            let d_section = self
                .attrs
                .parsing_section
                .take()
                .or_else(|| section.clone());
            let d_cleanup_fn = self.attrs.parsing_cleanup_fn.take();
            let d_used = self.attrs.parsing_used();
            let d_maybe_unused = self.attrs.parsing_maybe_unused();
            let d_noreturn = self.attrs.parsing_noreturn();
            let d_pure = self.attrs.parsing_pure();
            let d_const_attr = self.attrs.parsing_const_attr();
            let d_error_attr = self.attrs.parsing_error_attr();
            self.attrs.set_weak(false);
            self.attrs.set_used(false);
            self.attrs.set_maybe_unused(false);
            self.attrs.set_fastcall(false);
            self.attrs.set_naked(false);
            self.attrs.set_no_instrument(false);
            self.attrs.set_noreturn(false);
            self.attrs.set_pure(false);
            self.attrs.set_const_attr(false);
            self.attrs.set_error_attr(false);
            let dinit = if self.consume_if(&TokenKind::Assign) {
                Some(self.parse_initializer())
            } else {
                None
            };
            let d_fastcall = self.attrs.parsing_fastcall();
            let d_regparm = self.attrs.parsing_regparm;
            declarators.push(InitDeclarator {
                name: dname.unwrap_or_default(),
                derived: dderived,
                init: dinit,
                attrs: {
                    let mut da = DeclAttributes::default();
                    da.set_constructor(d_ctor);
                    da.set_destructor(d_dtor);
                    da.set_weak(d_weak);
                    da.set_error_attr(d_error_attr);
                    da.set_noreturn(d_noreturn);
                    da.set_nested_fn(self.attrs.parsing_nested_fn());
                    da.set_pure(d_pure);
                    da.set_const_attr(d_const_attr);
                    da.set_used(d_used);
                    da.set_maybe_unused(d_maybe_unused || spec_maybe_unused);
                    da.set_noplt(spec_noplt);
                    da.set_fastcall(d_fastcall);
                    da.regparm = d_regparm;
                    da.alias_target = d_alias;
                    da.ifunc_target = d_ifunc;
                    da.visibility = d_vis;
                    da.tls_model = d_tls_model;
                    da.section = d_section;
                    da.object_space = d_object_space;
                    da.object_volatile = d_object_volatile;
                    da.pointer_const = d_pointer_const;
                    (da.object_restrict, da.object_atomic) = d_object_quals;
                    da.base_volatile = d_base_volatile;
                    da.base_restrict = d_base_restrict;
                    da.asm_register = d_asm_reg;
                    da.cleanup_fn = d_cleanup_fn;
                    da
                },
                span: start,
            });
            let (_, skip_aligned, skip_asm_reg) = self.skip_asm_and_attributes();
            let Some(last_decl) = declarators.last_mut() else {
                return None;
            };
            if self.attrs.parsing_maybe_unused() {
                last_decl.attrs.set_maybe_unused(true);
                self.attrs.set_maybe_unused(false);
            }
            if let Some(cleanup) = self.attrs.parsing_cleanup_fn.take() {
                last_decl.attrs.cleanup_fn = Some(cleanup);
            }
            if let Some(reg) = skip_asm_reg {
                last_decl.attrs.asm_register = Some(reg);
            }
            if let Some(a) = skip_aligned {
                ctx.alignment = Some(ctx.alignment.map_or(a, |prev| prev.max(a)));
            }
            self.record_function_entity(
                Some(&last_decl.name),
                &last_decl.derived,
                self.attrs.parsing_noinline(),
            );
            self.attrs.set_noinline(false);
            // A trailing attribute written on a later sibling is still a
            // declaration-level verdict for GCC: `int a, b __attribute__
            // ((noreturn));` warns once, at the declaration (measured). The
            // first declarator was checked before this loop; each sibling's
            // own placement records are settled here.
            let sibling_records = std::mem::take(&mut self.attrs.parsing_attr_targets);
            if !sibling_records.is_empty() {
                let sibling_derived = last_decl.derived.clone();
                self.check_attribute_targets(&type_spec, &sibling_derived, &sibling_records, start);
            }
        }

        // Register typedef names
        let is_typedef = self.attrs.parsing_typedef();
        let is_transparent_union = self.take_transparent_union();
        self.register_typedefs(&type_spec, &declarators, decl_addr_space);

        self.expect_after(&TokenKind::Semicolon, "after declaration");
        // `-Wstrict-prototypes` is a property of the declared types, so it is
        // read off the finished declarators (`int f();`, `int (*p)();`, and a
        // nested `void g(int (*h)());` all qualify) rather than tracked
        // through the parse.
        let unprototyped = declarators
            .iter()
            .any(|item| DerivedDeclarator::has_unprototyped_function_type(&item.derived));
        let mut d = Declaration::new(
            type_spec,
            declarators,
            ctx.alignment,
            ctx.alignas_type,
            ctx.alignment_sizeof_type,
            decl_addr_space,
            self.attrs.parsing_vector_size.take(),
            self.attrs.parsing_ext_vector_nelem.take(),
            start,
        );
        d.set_vector_size_expr(self.attrs.parsing_vector_size_expr.take());
        d.set_static(self.attrs.parsing_static());
        d.set_extern(self.attrs.parsing_extern());
        d.set_typedef(is_typedef);
        d.set_const(self.attrs.parsing_const());
        d.set_constexpr(self.attrs.parsing_constexpr());
        d.set_constexpr_last(self.attrs.parsing_constexpr_last());
        d.set_volatile(self.attrs.parsing_volatile());
        d.set_pointer_const(self.attrs.parsing_pointer_const());
        d.set_common(ctx.is_common);
        d.set_thread_local(self.attrs.parsing_thread_local());
        d.set_transparent_union(is_transparent_union);
        d.set_inline(self.attrs.parsing_inline());
        self.flush_declarator_verdicts(
            decl_loc,
            None,
            unprototyped,
            // A file-scope object has static storage duration, so `cleanup`
            // is ignored there whatever its argument looks like; only the
            // arity error survives (measured).
            false,
        );
        Some(ExternalDecl::Declaration(d))
    }

    /// The pending `transparent_union` attribute, cleared. Every path that
    /// builds a declaration takes it: one that left it set handed it to the
    /// NEXT declaration and lost it for its own -- `union U { int i; float f;
    /// } __attribute__((transparent_union));` was never transparent, so the
    /// call `f(7)` copied a union from address 7 at runtime.
    fn take_transparent_union(&mut self) -> bool {
        let pending = self.attrs.parsing_transparent_union();
        self.attrs.set_transparent_union(false);
        pending
    }

    /// Whether `expr` mentions the identifier `name` anywhere (a cheap guard
    /// against recording `int x = x;` as a constant).
    fn mentions_identifier(expr: &Expr, name: &str) -> bool {
        let mut found = false;
        let mut stack: Vec<&Expr> = Vec::new();
        // `for_each_child` is the one-step visitor; recurse through it so the
        // guard sees every nested occurrence without a second walker.
        stack.push(expr);
        while let Some(e) = stack.pop() {
            if let Expr::Identifier(n, _) = e
                && n == name
            {
                found = true;
                break;
            }
            e.for_each_child(&mut |child| stack.push(child));
        }
        found
    }

    /// Record `name` as an integer constant when a block-scope declaration is
    /// an integer scalar with a self-contained constant initializer.
    ///
    /// Deliberately conservative: pointers, aggregates, `volatile`, `extern`,
    /// `static` and anything the parser's own integer folder cannot evaluate
    /// are not recorded, so the `asm "i"` diagnostic is preserved for them —
    /// exactly the set GCC also rejects.  The declarator must be a plain name
    /// (no pointer/array/function derivation).
    pub(super) fn record_local_constant(
        &mut self,
        name: &str,
        derived: &[DerivedDeclarator],
        init: &Initializer,
    ) {
        if !derived.is_empty() || self.attrs.parsing_volatile() || self.attrs.parsing_static() {
            return;
        }
        // Only an expression initializer can be a scalar constant; a braced
        // list is an aggregate (or a scalar with a redundant brace, which is
        // not worth the special case).
        let Initializer::Expr(expr) = init else {
            return;
        };
        // A self-reference (`int x = x;`) must not resolve to itself.
        if Self::mentions_identifier(expr, name) {
            return;
        }
        // Constants already recorded in this or an enclosing block participate,
        // so `int fl = 3; __auto_type v = (1 << 0) | (fl << 8);` folds.  GCC
        // reaches the same answer by constant-propagating the local before it
        // re-checks the `asm` statement.
        let mut fold_enums = self.enum_constants.clone();
        for (n, val) in &self.local_constants {
            fold_enums.entry(n.clone()).or_insert(*val);
        }
        match Self::eval_const_int_expr_with_enums(expr, Some(&fold_enums), None) {
            Some(v) => {
                self.local_constants.insert(name.to_string(), v);
            }
            // A declaration without a foldable initializer SHADOWS any outer
            // record of the same name: `int v = 5; { int v; asm("i"(v)); }`
            // must not read the outer 5.
            None => {
                self.local_constants.remove(name);
            }
        }
    }

    /// Finish a block-scope bare type declaration such as `struct S { ... };`.
    /// The address space is lexical to this declaration: a nested declaration
    /// inside `typeof` must not donate its space to the enclosing one.
    fn parse_local_bare_type(
        &mut self,
        type_spec: TypeSpecifier,
        decl_addr_space: AddressSpace,
        is_static: bool,
        is_extern: bool,
        start: Span,
    ) -> Declaration {
        self.advance();
        let is_transparent_union = self.take_transparent_union();
        self.attrs.parsing_address_space = AddressSpace::Default;
        let mut d = Declaration::new(
            type_spec,
            Vec::new(),
            None,
            None,
            None,
            decl_addr_space,
            self.attrs.parsing_vector_size.take(),
            self.attrs.parsing_ext_vector_nelem.take(),
            start,
        );
        d.set_vector_size_expr(self.attrs.parsing_vector_size_expr.take());
        d.set_static(is_static);
        d.set_extern(is_extern);
        d.set_typedef(self.attrs.parsing_typedef());
        d.set_const(self.attrs.parsing_const());
        d.set_constexpr(self.attrs.parsing_constexpr());
        d.set_constexpr_last(self.attrs.parsing_constexpr_last());
        d.set_volatile(self.attrs.parsing_volatile());
        d.set_pointer_const(self.attrs.parsing_pointer_const());
        d.set_thread_local(self.attrs.parsing_thread_local());
        d.set_transparent_union(is_transparent_union);
        d
    }

    /// Register block-scope typedef names, or record the typedef/object shadows
    /// introduced by ordinary declarations.  Returns whether the declaration
    /// was a typedef so the caller can stamp the finished `Declaration`.
    fn register_local_typedefs(
        &mut self,
        declarators: &[InitDeclarator],
        decl_addr_space: AddressSpace,
    ) -> bool {
        let is_typedef = self.attrs.parsing_typedef();
        if is_typedef {
            self.record_typedef_object_spaces(declarators, decl_addr_space);
            for decl in declarators {
                if !decl.name.is_empty() {
                    self.typedefs.insert(decl.name.clone());
                    self.shadowed_typedefs.remove(&decl.name);
                }
            }
            self.attrs.set_typedef(false);
        } else {
            for decl in declarators {
                if !decl.name.is_empty() && self.typedefs.contains(&decl.name) {
                    self.shadowed_typedefs.insert(decl.name.clone());
                }
            }
            self.record_object_shadows(declarators);
        }
        is_typedef
    }

    pub(super) fn parse_local_declaration(&mut self) -> Option<Declaration> {
        // Residue from contexts that never reach a declaration-kind check
        // (parameter lists, abstract declarators) must not attribute to
        // this declaration.
        self.attrs.parsing_attr_targets.clear();
        // A leading attribute list of THIS declaration may already have been
        // consumed by the statement-level pre-scan; take the record so it
        // cannot leak into a later declaration (the file-scope path clears it
        // when it finishes a declarator, this one owns it from entry).
        let pre_attr_span = self.attrs.first_attr_span.take();
        self.attrs.last_decl_name_span = None;
        let start = self.peek_span();
        // Selective reset: only storage-class/qualifier flags need clearing here.
        // GCC attribute flags (constructor, weak, etc.) may carry over from the
        // type-specifier parse that already ran before we entered this path.
        // parse_external_decl does a full `attrs = default()` because it starts
        // a completely new declaration context.
        self.attrs.set_static(false);
        self.attrs.set_extern(false);
        self.attrs.set_thread_local(false);
        self.attrs.set_typedef(false);
        self.attrs.set_inline(false);
        self.attrs.set_const(false);
        self.attrs.set_constexpr(false);
        self.attrs.set_atomic(false);
        self.attrs.set_restrict(false);
        self.attrs.set_volatile(false);
        self.attrs.set_typedef_use(false);
        self.attrs.set_typedef_object_volatile(false);
        self.attrs.set_pointer_const(false);
        // A containing function or previous declaration must not donate its
        // retain attribute to a new block-scope declaration.
        self.attrs.set_retain(false);
        self.attrs.parsing_address_space = AddressSpace::Default;
        let type_spec = self.parse_type_specifier()?;
        let spec_maybe_unused = self.attrs.parsing_maybe_unused();
        self.attrs.set_maybe_unused(false);

        self.consume_post_type_qualifiers();
        // Specifier-position applicability records settle here (before the
        // declarator's parameters or trailing attributes join the pool).
        let mut spec_attr_targets_for_loop =
            Some(std::mem::take(&mut self.attrs.parsing_attr_targets));
        // Specifier-level visibility / TLS model apply to every declarator;
        // a declarator's own attribute applies to it alone (same rule as the
        // file-scope path). Both are TAKEN so neither can leak into a later
        // declaration of the block: a stale `tls_model("local-exec")` landing
        // on a later `extern __thread` would read another module's TLS
        // through a link-time offset that does not belong to it.
        let spec_visibility = self.attrs.parsing_visibility.take();
        let spec_tls_model = self.attrs.parsing_tls_model.take();
        let spec_uninitialized = self.attrs.parsing_uninitialized();
        self.attrs.set_uninitialized(false);
        // Specifier attributes apply to all comma siblings; declarator
        // attributes apply only to that sibling. Consume the parser bit before
        // an initializer can introduce attributes of its own.
        let spec_retain = self.attrs.parsing_retain();
        self.attrs.set_retain(false);
        let spec_noplt = self.attrs.parsing_noplt();
        self.attrs.set_noplt(false);

        // Snapshot the declaration's address space NOW: initializer parsing
        // can contain casts to segment-qualified pointers, and the abstract-
        // declarator path `mem::take`s parsing_address_space (types.rs), so
        // reading the flag after the declarators+inits are parsed loses the
        // qualifier. `unsigned long __seg_fs *p = (fsptr)40;` compiled its
        // `*p` without the %fs prefix — an absolute-address load (glibc TLS
        // stack-guard reads at %fs:16/%fs:40 became NULL-page dereferences).
        let decl_addr_space = self.attrs.parsing_address_space;

        let is_static = self.attrs.parsing_static();
        let is_extern = self.attrs.parsing_extern();

        let mut declarators = Vec::with_capacity(4);

        // Handle bare type with semicolon (struct/enum/union definition)
        if matches!(self.peek(), TokenKind::Semicolon) {
            return Some(self.parse_local_bare_type(
                type_spec,
                decl_addr_space,
                is_static,
                is_extern,
                start,
            ));
        }

        let mut mode_kind: Option<ModeKind> = None;
        let mut alignment: Option<usize> = None;
        loop {
            let (name, derived, decl_mode, _, decl_aligned, _) = self.parse_declarator_with_attrs();
            // The specifier records check the first declarator once; later
            // declarators of the statement inherit the same verdict.
            let mut pending = spec_attr_targets_for_loop.take().unwrap_or_default();
            let recorded = std::mem::take(&mut pending);
            self.check_attribute_targets(&type_spec, &derived, &recorded, start);
            let l_object_space = self.attrs.declarator_object_space;
            let l_object_volatile = self.attrs.declarator_object_volatile;
            let l_pointer_const = self.attrs.parsing_pointer_const();
            let l_object_quals = (
                self.attrs.declarator_object_restrict,
                self.attrs.declarator_object_atomic,
            );
            let l_base_volatile = self.attrs.declarator_base_volatile;
            let l_base_restrict = self.attrs.declarator_base_restrict;
            let (skip_mode, skip_aligned, skip_asm_reg) = self.skip_asm_and_attributes();
            let local_maybe_unused = spec_maybe_unused || self.attrs.parsing_maybe_unused();
            self.attrs.set_maybe_unused(false);
            let local_uninitialized = spec_uninitialized || self.attrs.parsing_uninitialized();
            self.attrs.set_uninitialized(false);
            let local_retain = spec_retain || self.attrs.parsing_retain();
            self.attrs.set_retain(false);
            let local_cleanup_fn = self.attrs.parsing_cleanup_fn.take();
            let local_section = self.attrs.parsing_section.take();
            let local_visibility = self
                .attrs
                .parsing_visibility
                .take()
                .or_else(|| spec_visibility.clone());
            let local_tls_model = self.attrs.parsing_tls_model.take().or(spec_tls_model);
            mode_kind = mode_kind.or(decl_mode).or(skip_mode);
            // Merge alignment from declarator and post-declarator attributes
            for a in [decl_aligned, skip_aligned].iter().copied().flatten() {
                alignment = Some(alignment.map_or(a, |prev| prev.max(a)));
            }
            let init = if self.consume_if(&TokenKind::Assign) {
                Some(self.parse_initializer())
            } else {
                None
            };
            // Record a block-scope integer constant so an `asm "i"` operand may
            // name it (see `local_constants` in the parser state).  A
            // declaration with no initializer still SHADOWS an outer record.
            if let Some(decl_name) = name.as_ref() {
                if !decl_name.is_empty() && !self.attrs.parsing_typedef() {
                    match init.as_ref() {
                        Some(init) => self.record_local_constant(decl_name, &derived, init),
                        None => {
                            self.local_constants.remove(decl_name.as_str());
                        }
                    }
                }
            }
            // Attributes parsed inside an initializer belong to that nested
            // construct, not to this declarator or the next comma sibling.
            self.attrs.set_retain(false);
            declarators.push(InitDeclarator {
                name: name.unwrap_or_default(),
                derived,
                init,
                attrs: {
                    let mut da = DeclAttributes::default();
                    da.object_space = l_object_space;
                    da.object_volatile = l_object_volatile;
                    da.pointer_const = l_pointer_const;
                    (da.object_restrict, da.object_atomic) = l_object_quals;
                    da.base_volatile = l_base_volatile;
                    da.base_restrict = l_base_restrict;
                    da.section = local_section;
                    da.tls_model = local_tls_model;
                    da.asm_register = skip_asm_reg;
                    da.cleanup_fn = local_cleanup_fn;
                    da.set_uninitialized(local_uninitialized);
                    da.set_used(self.attrs.parsing_used());
                    da.set_retain(local_retain);
                    da.set_maybe_unused(local_maybe_unused);
                    da.set_nested_fn(self.attrs.parsing_nested_fn());
                    da.set_noreturn(self.attrs.parsing_noreturn());
                    da.set_pure(self.attrs.parsing_pure());
                    da.set_const_attr(self.attrs.parsing_const_attr());
                    da.set_constructor(self.attrs.parsing_constructor());
                    da.set_destructor(self.attrs.parsing_destructor());
                    da.set_weak(self.attrs.parsing_weak());
                    da.set_noplt(spec_noplt);
                    da.set_fastcall(self.attrs.parsing_fastcall());
                    da.regparm = self.attrs.parsing_regparm;
                    da.set_naked(self.attrs.parsing_naked());
                    da.set_no_instrument(self.attrs.parsing_no_instrument());
                    da.set_error_attr(self.attrs.parsing_error_attr());
                    if let Some(ref target) = self.attrs.parsing_alias_target {
                        da.alias_target = Some(target.clone());
                    }
                    if let Some(ref target) = self.attrs.parsing_ifunc_target {
                        da.ifunc_target = Some(target.clone());
                    }
                    da.visibility = local_visibility;
                    da.tls_model = local_tls_model;
                    da
                },
                span: start,
            });
            let (_, post_init_aligned, _post_asm_reg) = self.skip_asm_and_attributes();
            let Some(last_decl) = declarators.last_mut() else {
                return None;
            };
            if self.attrs.parsing_maybe_unused() {
                last_decl.attrs.set_maybe_unused(true);
                self.attrs.set_maybe_unused(false);
            }
            if self.attrs.parsing_retain() {
                last_decl.attrs.set_retain(true);
                self.attrs.set_retain(false);
            }
            if self.attrs.parsing_noplt() {
                last_decl.attrs.set_noplt(true);
                self.attrs.set_noplt(false);
            }
            if let Some(a) = post_init_aligned {
                alignment = Some(alignment.map_or(a, |prev| prev.max(a)));
            }
            if !self.consume_if(&TokenKind::Comma) {
                break;
            }
        }

        // Apply __attribute__((mode(...))): transform type to specified bit-width
        let type_spec = if let Some(mk) = mode_kind {
            mk.apply(type_spec)
        } else {
            type_spec
        };

        // A block-scope declaration hides an enumerator of the same name.
        if self.block_depth > 0 {
            for decl in &declarators {
                if self.is_parser_enum_constant(&decl.name) {
                    self.hide_enum_constant(&decl.name);
                }
            }
        }

        // Register typedef names or shadow them for variable declarations
        let is_typedef = self.register_local_typedefs(&declarators, decl_addr_space);

        self.expect_after(&TokenKind::Semicolon, "after declaration");
        // The declaration's address space is lexical to it: consume it now
        // that every declarator has read it. A declaration NESTED inside
        // this one's type specifier (a statement expression inside typeof,
        // kernel per-CPU read macros) parses its own `const void __seg_gs *q`
        // and used to leave __seg_gs in the flag — the outer `typeof(({...}))
        // x` then inherited the space and tripped "'__seg_gs' specified for
        // auto variable" on code GCC 16.2 accepts.
        self.attrs.parsing_address_space = AddressSpace::Default;
        // Merge alignment from _Alignas (captured in parsed_alignas during type specifier parsing)
        // with alignment from __attribute__((aligned(N))) on declarators
        if let Some(a) = self.attrs.parsed_alignas.take() {
            alignment = Some(alignment.map_or(a, |prev| prev.max(a)));
        }
        let alignas_type = self.attrs.parsed_alignas_type.take();
        let alignment_sizeof_type = self.attrs.parsed_alignment_sizeof_type.take();
        let is_transparent_union = self.take_transparent_union();
        // gcc applies the `cleanup` argument-shape checks only to an
        // automatic object: the one target C23 6.7.12.2 lets the attribute
        // serve. Everything else is warned about and ignored.
        let cleanup_auto_object = !self.attrs.parsing_static()
            && !self.attrs.parsing_extern()
            && !self.attrs.parsing_typedef()
            && !declarators
                .iter()
                .all(|item| DerivedDeclarator::declares_function(&item.derived));
        // `-Wstrict-prototypes` is a property of the declared types, so it is
        // read off the finished declarators (`int f();`, `int (*p)();`, and a
        // nested `void g(int (*h)());` all qualify) rather than tracked
        // through the parse.
        let unprototyped = declarators
            .iter()
            .any(|item| DerivedDeclarator::has_unprototyped_function_type(&item.derived));
        let mut d = Declaration::new(
            type_spec,
            declarators,
            alignment,
            alignas_type,
            alignment_sizeof_type,
            decl_addr_space,
            self.attrs.parsing_vector_size.take(),
            self.attrs.parsing_ext_vector_nelem.take(),
            start,
        );
        d.set_vector_size_expr(self.attrs.parsing_vector_size_expr.take());
        d.set_static(is_static);
        d.set_extern(is_extern);
        d.set_typedef(is_typedef);
        d.set_const(self.attrs.parsing_const());
        d.set_constexpr(self.attrs.parsing_constexpr());
        d.set_constexpr_last(self.attrs.parsing_constexpr_last());
        d.set_volatile(self.attrs.parsing_volatile());
        d.set_pointer_const(self.attrs.parsing_pointer_const());
        d.set_thread_local(self.attrs.parsing_thread_local());
        d.set_transparent_union(is_transparent_union);
        // An attribute written after an initializer must not outlive this
        // declaration either (see the specifier snapshot above).
        self.attrs.parsing_visibility = None;
        self.attrs.parsing_tls_model = None;
        // gcc anchors a declaration's verdicts at its first token, and a
        // leading attribute list is part of it (`__attribute__((cleanup))
        // void *p = 0;` anchors at the attribute).
        let decl_loc = match pre_attr_span {
            Some(attr) if attr.start < start.start => attr,
            _ => start,
        };
        self.flush_declarator_verdicts(decl_loc, None, unprototyped, cleanup_auto_object);
        Some(d)
    }

    /// Parse an initializer: either a braced initializer list or a single expression.
    pub(super) fn parse_initializer(&mut self) -> Initializer {
        if matches!(self.peek(), TokenKind::LBrace) {
            let open = self.peek_span();
            // Initializer-budget frame: nested `{ { ... } }` initializer
            // lists recurse one frame per brace level; on exhaustion
            // consume nothing and return an empty list placeholder (the
            // budget diagnostic is already the TU verdict).
            if !self.enter_initializer_frame(open) {
                return Initializer::List(Vec::new());
            }
            self.advance();
            let mut items = Vec::with_capacity(8);
            while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
                let mut designators = Vec::with_capacity(4);
                // Parse designators: [idx] and .field
                loop {
                    if matches!(self.peek(), TokenKind::LBracket) {
                        let open = self.peek_span();
                        self.advance();
                        let lo = self.parse_expr();
                        if self.consume_if(&TokenKind::Ellipsis) {
                            // GCC range designator: [lo ... hi]
                            let hi = self.parse_expr();
                            self.expect_closing(&TokenKind::RBracket, open);
                            designators.push(Designator::Range(lo, hi));
                        } else {
                            self.expect_closing(&TokenKind::RBracket, open);
                            designators.push(Designator::Index(lo));
                        }
                    } else if self.consume_if(&TokenKind::Dot) {
                        if let TokenKind::Identifier(name) = self.peek() {
                            let name = name.clone();
                            self.advance();
                            designators.push(Designator::Field(name));
                        }
                    } else {
                        break;
                    }
                }
                // GNU old-style designator: field: value
                if designators.is_empty() {
                    if let TokenKind::Identifier(name) = self.peek() {
                        let name = name.clone();
                        if self.pos + 1 < self.tokens.len()
                            && matches!(self.tokens[self.pos + 1].kind, TokenKind::Colon)
                        {
                            self.advance(); // consume identifier
                            self.advance(); // consume colon
                            designators.push(Designator::Field(name));
                        }
                    }
                }
                if !designators.is_empty() && matches!(self.peek(), TokenKind::Assign) {
                    self.advance();
                }
                let init = self.parse_initializer();
                items.push(InitializerItem { designators, init });
                if !self.consume_if(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_closing(&TokenKind::RBrace, open);
            // Expand range designators [lo ... hi] into individual Index items
            let enums = if self.enum_constants.is_empty() {
                None
            } else {
                Some(&self.enum_constants)
            };
            let items = Self::expand_range_designators(items, enums);
            self.exit_parser_frame();
            Initializer::List(items)
        } else {
            Initializer::Expr(self.parse_assignment_expr())
        }
    }

    /// Expand GCC range designators `[lo ... hi] = val` into multiple items
    /// `[lo] = val, [lo+1] = val, ..., [hi] = val` so downstream code only
    /// sees `Designator::Index`.
    fn expand_range_designators(
        items: Vec<InitializerItem>,
        enum_consts: Option<&FxHashMap<String, i64>>,
    ) -> Vec<InitializerItem> {
        let mut result = Vec::with_capacity(items.len());
        for item in items {
            if let Some(range_pos) = item
                .designators
                .iter()
                .position(|d| matches!(d, Designator::Range(_, _)))
            {
                if let Designator::Range(ref lo_expr, ref hi_expr) = item.designators[range_pos] {
                    // Try to evaluate lo and hi as integer constants
                    let lo = Self::eval_const_int_expr_with_enums(lo_expr, enum_consts, None);
                    let hi = Self::eval_const_int_expr_with_enums(hi_expr, enum_consts, None);
                    if let (Some(lo_val), Some(hi_val)) = (lo, hi) {
                        for idx in lo_val..=hi_val {
                            let mut new_desigs = item.designators.clone();
                            new_desigs[range_pos] =
                                Designator::Index(Expr::IntLiteral(idx, Span::dummy()));
                            result.push(InitializerItem {
                                designators: new_desigs,
                                init: item.init.clone(),
                            });
                        }
                        continue;
                    }
                }
                // If we can't evaluate, keep as-is (will error later)
                result.push(item);
            } else {
                result.push(item);
            }
        }
        result
    }

    /// Constant expression evaluator for compile-time integer contexts.
    /// Handles integer/char literals, all arithmetic/bitwise/shift/logical/relational
    /// binary ops, unary ops (neg, bitwise not, logical not), and ternary expressions.
    /// Used for range designator bounds, alignment expressions, etc.
    /// The optional `enum_consts` map allows resolving enum constant identifiers.
    pub(super) fn eval_const_int_expr(expr: &Expr) -> Option<i64> {
        Self::eval_const_int_expr_with_enums(expr, None, None)
    }

    /// Evaluate a constant integer expression, with access to known enum constants
    /// and struct/union tag alignments for resolving tag-only __alignof__ references.
    pub(super) fn eval_const_int_expr_with_enums(
        expr: &Expr,
        enum_consts: Option<&FxHashMap<String, i64>>,
        tag_aligns: Option<&FxHashMap<String, usize>>,
    ) -> Option<i64> {
        match expr {
            Expr::IntLiteral(val, _) => Some(*val),
            Expr::UIntLiteral(val, _) => Some(*val as i64),
            Expr::LongLiteral(val, _) => Some(*val),
            Expr::ULongLiteral(val, _) => Some(*val as i64),
            Expr::LongLongLiteral(val, _) => Some(*val),
            Expr::ULongLongLiteral(val, _) => Some(*val as i64),
            Expr::CharLiteral(val, _) => Some(*val as i64),
            // Identifiers: look up enum constants if available
            Expr::Identifier(name, _) => enum_consts.and_then(|m| m.get(name.as_str()).copied()),
            // Compile-time bit builtins.  `sema`/`lowering` already fold these
            // through `common::const_eval`, but the *parser's* folder is the one
            // the `asm "i"` validator consults, and it saw none of them, so a
            // constant initializer built from a compile-time builtin (e.g.
            // `__builtin_clz(64) - 1`) fell out of the constant-expression class
            // and the statement was rejected.  The table is shared, so the two
            // folds can never disagree.
            Expr::FunctionCall(callee, args, _) => {
                let Expr::Identifier(name, _) = callee.as_ref() else {
                    return None;
                };
                if !name.starts_with("__builtin_") {
                    return None;
                }
                let folded = crate::common::const_eval::eval_builtin_call_with_mode(
                    name.as_str(),
                    args,
                    &|e| {
                        Self::eval_const_int_expr_with_enums(e, enum_consts, tag_aligns)
                            .map(crate::ir::reexports::IrConst::I64)
                    },
                    true,
                )?;
                folded.to_i64()
            }
            Expr::BinaryOp(op, lhs, rhs, _) => {
                let l = Self::eval_const_int_expr_with_enums(lhs, enum_consts, tag_aligns)?;
                let r = Self::eval_const_int_expr_with_enums(rhs, enum_consts, tag_aligns)?;
                match op {
                    BinOp::Add => Some(l.wrapping_add(r)),
                    BinOp::Sub => Some(l.wrapping_sub(r)),
                    BinOp::Mul => Some(l.wrapping_mul(r)),
                    BinOp::Div if r != 0 => {
                        // Unsigned operands must divide as u64: e.g.
                        // 18446744073709551615UL / 2 = 9223372036854775807,
                        // but the i64 view (-1) / 2 would be 0.
                        if Self::is_unsigned_int_expr(lhs) {
                            Some(((l as u64) / (r as u64)) as i64)
                        } else {
                            Some(l.wrapping_div(r))
                        }
                    }
                    BinOp::Mod if r != 0 => {
                        if Self::is_unsigned_int_expr(lhs) {
                            Some(((l as u64) % (r as u64)) as i64)
                        } else {
                            Some(l.wrapping_rem(r))
                        }
                    }
                    BinOp::Shl => Some(l.wrapping_shl(r as u32)),
                    BinOp::Shr => {
                        // Use logical (unsigned) shift when the LHS appears unsigned
                        if Self::is_unsigned_int_expr(lhs) {
                            Some((l as u64).wrapping_shr(r as u32) as i64)
                        } else {
                            Some(l.wrapping_shr(r as u32))
                        }
                    }
                    BinOp::BitAnd => Some(l & r),
                    BinOp::BitOr => Some(l | r),
                    BinOp::BitXor => Some(l ^ r),
                    BinOp::Eq => Some(if l == r { 1 } else { 0 }),
                    BinOp::Ne => Some(if l != r { 1 } else { 0 }),
                    BinOp::Lt => {
                        if Self::is_unsigned_int_expr(lhs) || Self::is_unsigned_int_expr(rhs) {
                            Some(if (l as u64) < (r as u64) { 1 } else { 0 })
                        } else {
                            Some(if l < r { 1 } else { 0 })
                        }
                    }
                    BinOp::Le => {
                        if Self::is_unsigned_int_expr(lhs) || Self::is_unsigned_int_expr(rhs) {
                            Some(if (l as u64) <= (r as u64) { 1 } else { 0 })
                        } else {
                            Some(if l <= r { 1 } else { 0 })
                        }
                    }
                    BinOp::Gt => {
                        if Self::is_unsigned_int_expr(lhs) || Self::is_unsigned_int_expr(rhs) {
                            Some(if (l as u64) > (r as u64) { 1 } else { 0 })
                        } else {
                            Some(if l > r { 1 } else { 0 })
                        }
                    }
                    BinOp::Ge => {
                        if Self::is_unsigned_int_expr(lhs) || Self::is_unsigned_int_expr(rhs) {
                            Some(if (l as u64) >= (r as u64) { 1 } else { 0 })
                        } else {
                            Some(if l >= r { 1 } else { 0 })
                        }
                    }
                    BinOp::LogicalAnd => Some(if l != 0 && r != 0 { 1 } else { 0 }),
                    BinOp::LogicalOr => Some(if l != 0 || r != 0 { 1 } else { 0 }),
                    _ => None,
                }
            }
            Expr::UnaryOp(UnaryOp::Neg, inner, _) => {
                Self::eval_const_int_expr_with_enums(inner, enum_consts, tag_aligns)
                    .map(|v| v.wrapping_neg())
            }
            Expr::UnaryOp(UnaryOp::BitNot, inner, _) => {
                Self::eval_const_int_expr_with_enums(inner, enum_consts, tag_aligns).map(|v| {
                    let result = !v;
                    // For unsigned int operands, truncate to 32 bits. The evaluator
                    // uses i64 for all values, so ~0u produces i64(-1) (all 64 bits set)
                    // instead of the correct 0xFFFFFFFF (32-bit all-ones).
                    if Self::is_unsigned_int_expr(inner) {
                        result & 0xFFFF_FFFF
                    } else {
                        result
                    }
                })
            }
            Expr::UnaryOp(UnaryOp::LogicalNot, inner, _) => {
                Self::eval_const_int_expr_with_enums(inner, enum_consts, tag_aligns)
                    .map(|v| if v == 0 { 1 } else { 0 })
            }
            Expr::UnaryOp(UnaryOp::Plus, inner, _) => {
                Self::eval_const_int_expr_with_enums(inner, enum_consts, tag_aligns)
            }
            Expr::Conditional(cond, then_expr, else_expr, _) => {
                let c = Self::eval_const_int_expr_with_enums(cond, enum_consts, tag_aligns)?;
                if c != 0 {
                    Self::eval_const_int_expr_with_enums(then_expr, enum_consts, tag_aligns)
                } else {
                    Self::eval_const_int_expr_with_enums(else_expr, enum_consts, tag_aligns)
                }
            }
            Expr::Cast(target_type, inner, _, _) => {
                let val = Self::eval_const_int_expr_with_enums(inner, enum_consts, tag_aligns)?;
                // Apply the cast: truncate to target type's bit width
                if let Some(size) = Self::try_sizeof_type_spec(target_type) {
                    let bits = size * 8;
                    if bits >= 64 && Self::is_unsigned_type_spec(target_type) {
                        // 64-bit unsigned cast: the parser evaluator uses i64 which can't
                        // represent the full unsigned range. Return None to defer to sema.
                        return None;
                    }
                    if bits == 0 || bits >= 64 {
                        Some(val)
                    } else {
                        let mask = (1u64 << bits) - 1;
                        let truncated = (val as u64) & mask;
                        if Self::is_unsigned_type_spec(target_type) {
                            // Unsigned: zero-extend
                            Some(truncated as i64)
                        } else {
                            // Signed: sign-extend
                            let sign_bit = 1u64 << (bits - 1);
                            if truncated & sign_bit != 0 {
                                Some((truncated | !mask) as i64)
                            } else {
                                Some(truncated as i64)
                            }
                        }
                    }
                } else {
                    // Can't determine type size (typedef, struct, etc.) -
                    // return None so static_assert defers to sema-level evaluation
                    None
                }
            }
            Expr::Sizeof(arg, _) => match arg.as_ref() {
                SizeofArg::Type(ts) => Self::try_sizeof_type_spec(ts).map(|s| s as i64),
                SizeofArg::Expr(_) => None,
            },
            Expr::Countof(arg, _) => match arg.as_ref() {
                SizeofArg::Type(TypeSpecifier::Array(_, Some(bound))) => {
                    Self::eval_const_int_expr_with_enums(bound, enum_consts, tag_aligns)
                }
                SizeofArg::Expr(Expr::StringLiteral(s, _) | Expr::WideStringLiteral(s, _)) => {
                    Some((s.chars().count() + 1) as i64)
                }
                SizeofArg::Expr(Expr::Char16StringLiteral(s, _)) => {
                    Some((s.encode_utf16().count() + 1) as i64)
                }
                _ => None,
            },
            // `None` when the parser cannot know the alignment exactly.
            Expr::Alignof(ts, _) => Self::alignof_type_spec(ts, tag_aligns).map(|a| a as i64),
            // __alignof(type) returns preferred alignment (8 for long long/double on i686)
            Expr::GnuAlignof(ts, _) => {
                Self::preferred_alignof_type_spec(ts, tag_aligns).map(|a| a as i64)
            }
            // __alignof__(expr): parser-level can't always determine type alignment
            // from an expression, so return None (let sema/lowerer handle it)
            Expr::AlignofExpr(_, _) | Expr::GnuAlignofExpr(_, _) => None,
            _ => None,
        }
    }

    /// Check if an expression has unsigned type.
    /// Used by the parser-level constant evaluator for:
    /// - Truncating bitwise NOT results to the correct width
    /// - Using unsigned comparison semantics for relational operators
    ///
    /// Only detects obvious cases from the AST structure; non-obvious cases are
    /// handled by the sema-level evaluator which has full type information.
    fn is_unsigned_int_expr(expr: &Expr) -> bool {
        match expr {
            Expr::UIntLiteral(..) | Expr::ULongLiteral(..) | Expr::ULongLongLiteral(..) => true,
            Expr::Cast(ts, _, _, _) => Self::is_unsigned_type_spec(ts),
            // Unary +/- preserve the signedness of the operand.
            // In C, -size_t still has type size_t (unsigned wraparound).
            Expr::UnaryOp(UnaryOp::Plus, inner, _) | Expr::UnaryOp(UnaryOp::Neg, inner, _) => {
                Self::is_unsigned_int_expr(inner)
            }
            // Binary ops: unsigned if either operand is unsigned (C promotion)
            Expr::BinaryOp(_, lhs, rhs, _) => {
                Self::is_unsigned_int_expr(lhs) || Self::is_unsigned_int_expr(rhs)
            }
            // sizeof yields size_t (unsigned). _Alignof yields size_t (unsigned).
            // These are unsigned per C11 6.5.3.4 and 6.5.3.
            Expr::Sizeof(..)
            | Expr::Countof(..)
            | Expr::Alignof(..)
            | Expr::GnuAlignof(..)
            | Expr::AlignofExpr(..)
            | Expr::GnuAlignofExpr(..) => true,
            _ => false,
        }
    }

    /// Consume post-type storage class specifiers and qualifiers.
    /// C allows "struct { int i; } typedef name;" and "char _Alignas(16) x;".
    /// This is shared between parse_external_decl and parse_local_declaration.
    pub(super) fn consume_post_type_qualifiers(&mut self) {
        loop {
            match self.peek() {
                TokenKind::Typedef => {
                    self.advance();
                    self.attrs.set_typedef(true);
                }
                TokenKind::Static => {
                    self.advance();
                    self.attrs.set_static(true);
                }
                TokenKind::Extern => {
                    self.advance();
                    self.attrs.set_extern(true);
                }
                TokenKind::Const => {
                    self.advance();
                    self.attrs.set_const(true);
                }
                TokenKind::Constexpr => {
                    self.advance();
                    self.attrs.set_const(true);
                    self.attrs.set_constexpr(true);
                }
                TokenKind::Volatile => {
                    self.advance();
                    self.attrs.set_volatile(true);
                }
                TokenKind::Restrict | TokenKind::Inline | TokenKind::Register => {
                    self.advance();
                }
                TokenKind::Atomic => {
                    // `double _Atomic x`: the trailing spelling of the C11
                    // 6.7.3 qualifier (the leading `_Atomic T` and
                    // `_Atomic(T)` forms parse elsewhere).
                    self.advance();
                    self.attrs.set_atomic(true);
                }
                TokenKind::Auto => {
                    self.advance();
                    // C gives `auto` no meaning for variables, and the tree has
                    // always ignored it; but at block scope on a *function* it is
                    // the only spelling that says "this is a GNU nested function,
                    // not a redeclaration of the external one". Dropping it made
                    // that distinction unrecoverable in sema.
                    self.attrs.set_nested_fn(true);
                }
                TokenKind::SegGs => {
                    self.advance();
                    self.attrs.parsing_address_space = AddressSpace::SegGs;
                }
                TokenKind::SegFs => {
                    self.advance();
                    self.attrs.parsing_address_space = AddressSpace::SegFs;
                }
                TokenKind::Alignas => {
                    self.advance();
                    if let Some(align) = self.parse_alignas_argument() {
                        self.attrs.parsed_alignas = Some(
                            self.attrs
                                .parsed_alignas
                                .map_or(align, |prev| prev.max(align)),
                        );
                    }
                }
                TokenKind::Attribute => {
                    let (_, aligned, _, _, _) = self.parse_gcc_attributes_spec();
                    if let Some(a) = aligned {
                        self.attrs.parsed_alignas =
                            Some(self.attrs.parsed_alignas.map_or(a, |prev| prev.max(a)));
                    }
                }
                TokenKind::Extension => {
                    self.advance();
                }
                _ => break,
            }
        }
    }

    /// Register typedef names from declarators, if parsing_typedef is set.
    /// Record the named address space each typedef object gets. The object
    /// space is the declaration's space unless a declarator pointer absorbed
    /// it (`typedef int __seg_gs *P;` makes P's objects generic; P's pointees
    /// keep the space through the typedef's CType). A typedef with no space
    /// clears any earlier record of the same name (redefinition or shadowing).
    fn record_typedef_object_spaces(
        &mut self,
        declarators: &[InitDeclarator],
        decl_space: AddressSpace,
    ) {
        for decl in declarators {
            if decl.name.is_empty() {
                continue;
            }
            let space = crate::common::type_builder::object_storage_address_space_with_object(
                decl_space,
                &decl.derived,
                decl.attrs.object_space,
            );
            if space == AddressSpace::Default {
                self.typedef_object_spaces.remove(&decl.name);
            } else {
                self.typedef_object_spaces.insert(decl.name.clone(), space);
            }
            if decl.attrs.object_volatile {
                self.typedef_object_volatile.insert(decl.name.clone());
            } else {
                self.typedef_object_volatile.remove(&decl.name);
            }
            if decl.attrs.base_volatile {
                self.typedef_base_volatile.insert(decl.name.clone());
            } else {
                self.typedef_base_volatile.remove(&decl.name);
            }
        }
    }

    fn register_typedefs(
        &mut self,
        type_spec: &TypeSpecifier,
        declarators: &[InitDeclarator],
        decl_space: AddressSpace,
    ) {
        if self.attrs.parsing_typedef() {
            self.record_typedef_object_spaces(declarators, decl_space);
            for decl in declarators {
                if !decl.name.is_empty() {
                    self.typedefs.insert(decl.name.clone());
                    // Record the typedef's full type (base spec + derived
                    // pointer/array layers) so parse-time constant
                    // expressions can resolve sizeof(typedef) — required for
                    // e.g. `__attribute__((vector_size(16 * sizeof(int64_t))))`.
                    self.typedef_type_specs.insert(
                        decl.name.clone(),
                        self.typedef_spec_with_derived(type_spec.clone(), &decl.derived),
                    );
                }
            }
            self.attrs.set_typedef(false);
        }
    }

    /// Fold declarator-derived pointer/array layers into a TypeSpecifier so
    /// parser-side `sizeof(typedef)` resolves through the stored spec.
    /// Function-derived layers are left as-is: vector_size on function
    /// typedefs is not a valid C construct and those stay sema-deferred.
    fn typedef_spec_with_derived(
        &self,
        mut spec: TypeSpecifier,
        derived: &[DerivedDeclarator],
    ) -> TypeSpecifier {
        let mut pending_space = AddressSpace::Default;
        for d in derived {
            match d {
                DerivedDeclarator::Pointer(space, _) => {
                    spec = TypeSpecifier::Pointer(Box::new(spec), pending_space);
                    pending_space = *space;
                }
                DerivedDeclarator::Array(n) => {
                    spec = TypeSpecifier::Array(Box::new(spec), n.clone());
                }
                _ => {}
            }
        }
        spec
    }

    /// Parse `_Static_assert(constant-expr, "message")` or the C23
    /// single-argument form `_Static_assert(constant-expr)`.
    ///
    /// Sema decides every assertion (`check_static_assert`): whether the
    /// condition is a constant depends on types and on which declaration of
    /// each identifier is visible -- an enumerator, a C23 `constexpr` object,
    /// or a plain object hiding either -- which only sema knows.
    pub(super) fn parse_static_assert(&mut self) -> StaticAssert {
        let assert_span = self.peek_span();
        self.advance(); // consume _Static_assert
        let open = self.peek_span();
        self.expect_context(&TokenKind::LParen, "after '_Static_assert'");

        // Parse the constant expression
        let expr = self.parse_assignment_expr();

        // Parse optional message: ", string-literal(s)"
        // Adjacent string literals are concatenated per C standard (translation phase 6).
        // The kernel uses patterns like: "min" "(" "x" ", " "y" ") error"
        let message = if self.consume_if(&TokenKind::Comma) {
            let mut msg = String::with_capacity(32);
            let mut found_string = false;
            while let TokenKind::StringLiteral(s) = self.peek() {
                msg.push_str(s);
                found_string = true;
                self.advance();
            }
            if found_string { Some(msg) } else { None }
        } else {
            None
        };

        self.expect_closing(&TokenKind::RParen, open);
        self.consume_if(&TokenKind::Semicolon);

        StaticAssert {
            cond: expr,
            message,
            span: assert_span,
        }
    }
}

#[cfg(test)]
mod seg_placement_tests {
    //! Mirrors `common::type_builder`'s placement tests on the parser's
    //! `TypeSpecifier`, so the two placement rules are checked against the same
    //! declarator shapes. Keep the case lists in step with that module.
    use super::*;

    fn base() -> TypeSpecifier {
        TypeSpecifier::Long
    }
    fn ptr(inner: TypeSpecifier) -> TypeSpecifier {
        TypeSpecifier::Pointer(Box::new(inner), AddressSpace::Default)
    }
    fn innermost_space(ts: &TypeSpecifier) -> Option<AddressSpace> {
        match ts {
            TypeSpecifier::Pointer(inner, sp) => match inner.as_ref() {
                TypeSpecifier::Pointer(..)
                | TypeSpecifier::Array(..)
                | TypeSpecifier::FunctionPointer(..)
                | TypeSpecifier::BareFunction(..) => innermost_space(inner),
                _ => Some(*sp),
            },
            TypeSpecifier::Array(elem, _) => innermost_space(elem),
            TypeSpecifier::FunctionPointer(ret, ..) | TypeSpecifier::BareFunction(ret, ..) => {
                innermost_space(ret)
            }
            _ => None,
        }
    }
    fn outer_space(ts: &TypeSpecifier) -> Option<AddressSpace> {
        match ts {
            TypeSpecifier::Pointer(_, sp) => Some(*sp),
            _ => None,
        }
    }

    #[test]
    fn plain_pointer() {
        let mut t = ptr(base());
        apply_decl_address_space_to_spec(&mut t, AddressSpace::SegGs);
        assert_eq!(outer_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn pointer_to_pointer_qualifies_inner_only() {
        let mut t = ptr(ptr(base()));
        apply_decl_address_space_to_spec(&mut t, AddressSpace::SegGs);
        assert_eq!(outer_space(&t), Some(AddressSpace::Default));
        assert_eq!(innermost_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn function_pointer_qualifies_return_pointer_not_itself() {
        // struct c __seg_gs *(*fp)(void)
        let mut t = TypeSpecifier::FunctionPointer(Box::new(ptr(base())), Vec::new(), false, true);
        apply_decl_address_space_to_spec(&mut t, AddressSpace::SegGs);
        assert_eq!(innermost_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn bare_function_qualifies_return_pointer() {
        // struct c __seg_gs *f(void)
        let mut t = TypeSpecifier::BareFunction(Box::new(ptr(base())), Vec::new(), false, true);
        apply_decl_address_space_to_spec(&mut t, AddressSpace::SegGs);
        assert_eq!(innermost_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn array_of_pointers_qualifies_elements() {
        // struct c __seg_gs *a[4]
        let mut t = TypeSpecifier::Array(Box::new(ptr(base())), None);
        apply_decl_address_space_to_spec(&mut t, AddressSpace::SegGs);
        assert_eq!(innermost_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn pointer_to_array_of_pointers_keeps_outer_generic() {
        // struct c __seg_gs *(*pa)[4]
        let mut t = ptr(TypeSpecifier::Array(Box::new(ptr(base())), None));
        apply_decl_address_space_to_spec(&mut t, AddressSpace::SegGs);
        assert_eq!(outer_space(&t), Some(AddressSpace::Default));
        assert_eq!(innermost_space(&t), Some(AddressSpace::SegGs));
    }

    #[test]
    fn default_space_is_a_no_op() {
        let mut t = ptr(ptr(base()));
        let before = format!("{t:?}");
        apply_decl_address_space_to_spec(&mut t, AddressSpace::Default);
        assert_eq!(format!("{t:?}"), before);
    }
}

#[cfg(test)]
mod noinline_declaration_tests {
    use super::*;
    use crate::frontend::lexer::Lexer;

    #[test]
    fn noinline_merges_each_function_declarator_without_leaking() {
        for (prototype, expected) in [
            (
                "int f(void) __attribute__((noinline)); int g(void);",
                [true, false],
            ),
            (
                "int g(void), f(void) __attribute__((noinline));",
                [true, false],
            ),
            (
                "int f(void) __attribute__((noinline)), g(void);",
                [true, false],
            ),
            (
                "__attribute__((noinline)) int f(void), g(void);",
                [true, true],
            ),
            (
                "int f(void); int f(void) __attribute__((noinline)); int f(void);",
                [true, false],
            ),
            ("int (*p)(void) __attribute__((noinline));", [false, false]),
            (
                "int (*f(void))(void) __attribute__((noinline));",
                [true, false],
            ),
        ] {
            let source =
                format!("{prototype} int f(void) {{ return 1; }} int g(void) {{ return 2; }}");
            let mut parser = Parser::new(Lexer::new(&source, 0).tokenize());
            let tu = parser.parse();
            assert_eq!(parser.error_count, 0, "{source}");
            let actual: Vec<_> = tu
                .decls
                .iter()
                .filter_map(|d| match d {
                    ExternalDecl::FunctionDef(f) => Some(f.attrs.is_noinline()),
                    _ => None,
                })
                .collect();
            assert_eq!(actual, expected, "{source}");
            assert!(!parser.func_decl_noinline.contains("p"));
        }
    }
}
