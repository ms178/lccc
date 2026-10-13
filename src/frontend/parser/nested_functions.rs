//! GNU C nested function definitions (GCC extension).
//!
//! A nested function is defined at block scope:
//!
//! ```c
//! int f1(void) {
//!     int i = 0;
//!     int f2(int *p) { i = 1; return *p + 1; }   // nested definition
//!     return f0(f2, &i);                          // address taken
//! }
//! ```
//!
//! Parsing strategy: the declaration grammar at block scope is a prefix of
//! the nested-definition grammar (`type-spec declarator(params)` followed by
//! `{` instead of `;`/`=`/`,`), plus the C89 implicit-int shape
//! (`ident(params) {`). `try_parse_nested_function_def` speculatively parses
//! the declaration head and, on mismatch, restores the saved parser state so
//! the caller falls back to ordinary declaration/statement parsing.
//!
//! The detection predicate `looks_like_implicit_int_nested_function` is a
//! pure token scan (no state mutation): an identifier followed by a balanced
//! parameter list whose next token is `{` — an expression statement can
//! never be followed by a bare block, so this shape is unambiguous.

use super::parse::Parser;
use crate::common::source::Span;
use crate::frontend::lexer::token::TokenKind;
use crate::frontend::parser::ast::FunctionDef;

impl Parser {
    /// Token-level probe: does the current position start an implicit-int
    /// nested function definition (`ident ( params ) {`)?
    ///
    /// Pure lookahead: consumes nothing and mutates no parser state.
    pub(super) fn looks_like_implicit_int_nested_function(&self) -> bool {
        let TokenKind::Identifier(_) = self.peek() else {
            return false;
        };
        // Must be followed by '('.
        if !matches!(self.peek_nth(1), TokenKind::LParen) {
            return false;
        }
        // Scan to the matching ')'.
        let mut i = self.pos + 2;
        let mut depth = 1usize;
        while i < self.tokens.len() {
            match self.tokens[i].kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                TokenKind::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        if i >= self.tokens.len() {
            return false;
        }
        // Skip trailing __attribute__((...)) groups, then require '{'.
        let mut j = i + 1;
        loop {
            match self.tokens.get(j).map(|t| &t.kind) {
                Some(TokenKind::Attribute) => {
                    // __attribute__ ( ( ... ) )
                    j += 1;
                    // expect '(' '('
                    let mut depth = 0usize;
                    let mut opened = false;
                    while j < self.tokens.len() {
                        match self.tokens[j].kind {
                            TokenKind::LParen => {
                                depth += 1;
                                opened = true;
                            }
                            TokenKind::RParen => {
                                depth -= 1;
                                if opened && depth == 0 {
                                    break;
                                }
                            }
                            TokenKind::Eof => return false,
                            _ => {}
                        }
                        j += 1;
                    }
                    j += 1;
                }
                _ => break,
            }
        }
        matches!(self.tokens.get(j).map(|t| &t.kind), Some(TokenKind::LBrace))
    }

    /// Speculatively parse a nested function definition at block scope.
    /// Returns `None` (with the parser state fully restored) when the item
    /// turns out to be an ordinary declaration.
    pub(super) fn try_parse_nested_function_def(&mut self) -> Option<FunctionDef> {
        // Snapshot all state the speculative parse may mutate.
        let saved_pos = self.pos;
        let saved_attrs = self.attrs.clone();
        let saved_typedefs = self.typedefs.clone();
        let saved_shadowed = self.shadowed_typedefs.clone();
        let saved_typedef_spaces = self.typedef_object_spaces.clone();
        let saved_typedef_volatile = self.typedef_object_volatile.clone();
        let saved_typedef_base_volatile = self.typedef_base_volatile.clone();
        let saved_pragma_pack = self.pragma_pack_align.clone();
        let saved_pack_stack = self.pragma_pack_stack.clone();
        let saved_enum_consts = self.enum_constants.clone();
        let saved_unevaluable = self.unevaluable_enum_constants.clone();
        let saved_struct_aligns = self.struct_tag_alignments.clone();
        let saved_errors = self.error_count;

        match self.parse_nested_function_def_inner() {
            Some(fdef) => Some(fdef),
            None => {
                // Roll back every mutated field: the caller re-parses the
                // item as a normal declaration/statement.
                self.pos = saved_pos;
                self.attrs = saved_attrs;
                self.typedefs = saved_typedefs;
                self.shadowed_typedefs = saved_shadowed;
                self.typedef_object_spaces = saved_typedef_spaces;
                self.typedef_object_volatile = saved_typedef_volatile;
                self.typedef_base_volatile = saved_typedef_base_volatile;
                self.pragma_pack_align = saved_pragma_pack;
                self.pragma_pack_stack = saved_pack_stack;
                self.enum_constants = saved_enum_consts;
                self.unevaluable_enum_constants = saved_unevaluable;
                self.struct_tag_alignments = saved_struct_aligns;
                self.error_count = saved_errors;
                None
            }
        }
    }

    fn parse_nested_function_def_inner(&mut self) -> Option<FunctionDef> {
        // Type specifier: absent for the implicit-int shape.
        let has_type = self.is_type_specifier() && !self.is_typedef_label();
        let start: Span = self.peek_span();
        let type_spec = if has_type {
            self.parse_type_specifier()?
        } else {
            // Implicit int: must be `ident (` — checked by the caller's
            // predicate, re-verified here for safety.
            if let TokenKind::Identifier(_) = self.peek() {
                if matches!(self.peek_nth(1), TokenKind::LParen) {
                    crate::frontend::parser::ast::TypeSpecifier::Int
                } else {
                    return None;
                }
            } else {
                return None;
            }
        };
        self.consume_post_type_qualifiers();

        // One declarator: the function name and its parameter list.
        let (name, derived, decl_mode, _decl_common, _decl_aligned, _) =
            self.parse_declarator_with_attrs();
        let (_post_ctor, _post_dtor, post_mode, _post_common, _post_aligned, _first_asm_reg) =
            self.parse_asm_and_attributes();
        // A nested definition carries no alignment.  GCC applies none either:
        // measured on GCC 14 at -O0 and -O2, with and without `noinline`, an
        // `aligned(64)` on a nested function changes nothing observable in the
        // emitted asm, because the nested body is emitted as part of its parent.
        // So there is no oracle to match here, and honouring the attribute would
        // be a divergence rather than parity.  (The TOP-LEVEL definition channel
        // does have an oracle and is wired -- see `FunctionDef::alignment`.)
        let mode_kind = decl_mode.or(post_mode);
        // A function *definition*'s own `aligned(...)` attribute is not applied
        // here; it is taken from the definition's PROTOTYPES instead (see the
        // note at the `FunctionDef` construction below).  Computing it from the
        // definition's declarator attributes produced a value nothing could
        // consume, because a nested function's IR name is mangled
        // (`parent.inner`) while `function_alignments` is keyed by the plain
        // name.  The bindings are underscored at the destructuring sites above so
        // that the intent is visible where the values are discarded.

        // A nested function definition requires a plain function declarator
        // (a name plus a trailing Function(...) derivation — not a pointer
        // to function, not an array) followed by '{' (or K&R parameter
        // declarations then '{').
        let name = name?;
        if derived.is_empty() {
            return None;
        }
        if !matches!(
            derived.last(),
            Some(crate::frontend::parser::ast::DerivedDeclarator::Function(
                _,
                _
            ))
        ) {
            return None;
        }
        // The Function derivation must be the OUTERMOST one (last in the
        // chain): `void *x() { }` is a nested function returning void*
        // (derived = [Pointer, Function]); `int (*fp)(void)` is a
        // pointer-to-function VARIABLE (derived ends with Pointer) and is
        // rejected by the last()-is-Function check above.

        // Body or K&R parameter declarations then body.
        if !matches!(self.peek(), TokenKind::LBrace) && !self.is_type_specifier() {
            return None;
        }

        let (params, variadic) = match derived.last() {
            Some(crate::frontend::parser::ast::DerivedDeclarator::Function(p, v)) => {
                (p.clone(), *v)
            }
            _ => unreachable!(),
        };

        // K&R-style parameter declarations between ')' and '{'.
        let is_kr_style = !matches!(self.peek(), TokenKind::LBrace);
        let final_params = if is_kr_style {
            self.parse_kr_params(params)
        } else {
            params
        };

        let is_static = self.attrs.parsing_static();
        let is_inline = self.attrs.parsing_inline();
        let is_always_inline = self.attrs.parsing_always_inline();
        let is_noinline = self.attrs.parsing_noinline();
        // The definition's own attribute set, collected the way the file-scope
        // path collects it -- and consumed, so a nested function's
        // `__attribute__((...))` cannot also be read by the declarations that
        // follow it in the enclosing block.  Copying only four flags would lose
        // `noreturn`, `pure`, `const` and `cold` on a nested definition; see
        // `FunctionAttributes::for_definition`.
        let spec = crate::frontend::parser::ast::DefSpecifiers {
            static_: is_static,
            inline_: is_inline,
            extern_: self.attrs.parsing_extern(),
            gnu_inline: self.attrs.parsing_gnu_inline(),
            always_inline: is_always_inline,
            noinline: is_noinline,
        };
        let mut def_attrs = crate::frontend::parser::ast::DeclAttributes::default();
        def_attrs.set_constructor(self.attrs.parsing_constructor());
        def_attrs.set_destructor(self.attrs.parsing_destructor());
        def_attrs.set_weak(self.attrs.parsing_weak());
        def_attrs.set_used(self.attrs.parsing_used());
        def_attrs.set_fastcall(self.attrs.parsing_fastcall());
        def_attrs.regparm = self.attrs.parsing_regparm;
        def_attrs.set_naked(self.attrs.parsing_naked());
        def_attrs.set_no_instrument(self.attrs.parsing_no_instrument());
        def_attrs.set_noreturn(self.attrs.parsing_noreturn());
        def_attrs.set_pure(self.attrs.parsing_pure());
        def_attrs.set_const_attr(self.attrs.parsing_const_attr());
        def_attrs.set_cold(self.attrs.parsing_cold());
        def_attrs.set_error_attr(self.attrs.parsing_error_attr());
        def_attrs.alias_target = self.attrs.parsing_alias_target.take();
        def_attrs.visibility = self.attrs.parsing_visibility.take();
        def_attrs.section = self.attrs.parsing_section.take();
        def_attrs.symver = self.attrs.parsing_symver.take();
        def_attrs.diagnose_as = self.attrs.parsing_diagnose_as.take();
        def_attrs.cleanup_fn = self.attrs.parsing_cleanup_fn.take();

        // Shadow typedef names used as parameter names (same as file scope).
        let mut shadowed_added: Vec<String> = Vec::new();
        for param in &final_params {
            if let Some(ref pname) = param.name {
                if self.typedefs.contains(pname) && !self.shadowed_typedefs.contains(pname) {
                    self.shadowed_typedefs.insert(pname.clone());
                    shadowed_added.push(pname.clone());
                }
            }
        }
        self.attrs.set_noreturn(false);
        let body = self.parse_compound_stmt();
        for n in shadowed_added {
            self.shadowed_typedefs.remove(&n);
        }

        let attrs = crate::frontend::parser::ast::FunctionAttributes::for_definition(
            &def_attrs,
            spec,
            def_attrs.is_no_instrument(),
        );

        // __attribute__((mode(...))) applies to the return type.
        let type_spec = if let Some(mk) = mode_kind {
            mk.apply(type_spec)
        } else {
            type_spec
        };
        let return_type = self.build_return_type(type_spec, &derived);
        // Asm-register attributes have no meaning on a function definition and are
        // dropped.  Alignment does have one, and is carried on the `FunctionDef`
        // rather than applied here: the key codegen looks up is the EMITTED name,
        // and a nested function's emitted name is mangled (`parent.inner`), which is
        // only known once the lowering side has built the `IrFunction`.
        Some(FunctionDef {
            alignment: None,
            return_type,
            name,
            params: final_params,
            variadic,
            body,
            attrs,
            is_kr: is_kr_style,
            span: start,
        })
    }
}

#[cfg(test)]
mod nested_definition_attribute_tests {
    use crate::frontend::lexer::Lexer;
    use crate::frontend::parser::Parser;
    use crate::frontend::parser::ast::{BlockItem, ExternalDecl, FunctionAttributes};

    /// Every nested definition in `source`, in order, with its attributes.
    fn nested_attrs(source: &str) -> Vec<FunctionAttributes> {
        let tokens = Lexer::new(source, 0).tokenize();
        let mut parser = Parser::new(tokens);
        let tu = parser.parse();
        assert_eq!(parser.error_count, 0, "fixture must parse");
        let mut found = Vec::new();
        for decl in &tu.decls {
            if let ExternalDecl::FunctionDef(def) = decl {
                for item in &def.body.items {
                    if let BlockItem::NestedFunction(nested) = item {
                        found.push(nested.attrs.clone());
                    }
                }
            }
        }
        assert!(!found.is_empty(), "fixture must define a nested function");
        found
    }

    /// `_Noreturn` on a nested *definition* is a contract about that function.
    /// Until both definition paths shared one attribute assembler the nested
    /// parser copied four flags and dropped this one, so analysis saw a
    /// returning function: `int g(void){ _Noreturn auto void n(void){for(;;);}
    /// n(); }` warns under GCC and was silent here, which is the diagnostic face
    /// of the same wrong fallthrough answer a caller-side analysis would use.
    #[test]
    fn noreturn_in_decl_specifier_position_reaches_the_nested_definition() {
        let attrs = nested_attrs(
            "int g(void) { _Noreturn auto void n(void) { for (;;) {} } n(); return 0; }\n",
        );
        assert!(
            attrs[0].is_noreturn(),
            "the nested definition's own _Noreturn must be its attribute"
        );
    }

    /// The attributes a definition can state after its declarator are the same
    /// facts, and several of them (`pure`, `const`, `cold`) are optimization
    /// contracts the frontend must not lose.  GCC rejects this spelling on a
    /// definition outright -- measured on gcc 14, "attributes should be specified
    /// before the declarator in a function definition" -- so honouring it is a
    /// deliberate superset of the oracle, not a divergence from a working
    /// behaviour: the alternative is silently discarding what the user wrote.
    #[test]
    fn attributes_after_the_declarator_reach_the_nested_definition() {
        let attrs = nested_attrs(
            "int g(void) { auto int n(void) __attribute__((pure, cold, section(\"hot\"))) \
             { return 1; } return n(); }\n",
        );
        let n = &attrs[0];
        assert!(n.is_pure(), "pure must reach the AST");
        assert!(n.is_cold(), "cold must reach the AST");
        assert_eq!(
            n.section.as_deref(),
            Some("hot"),
            "section must reach the AST"
        );
    }

    /// The definition's attributes are consumed by that definition.  Reading the
    /// parser's accumulated state without clearing it, as a path that copies
    /// only some fields would do for everything it did not copy, would leave `noreturn` set for the next
    /// declaration in the enclosing block -- which is how a returning nested
    /// function following a noreturn one would inherit a contract it does not
    /// have (pinned at the diagnostic level by `q4` in
    /// `tests/regression/check_scoped_noreturn_diagnostics.sh`).
    #[test]
    fn one_nested_definitions_attributes_do_not_become_the_next_ones() {
        let attrs = nested_attrs(
            "int g(void) {\n\
             \x20   _Noreturn auto void n(void) { for (;;) {} }\n\
             \x20   auto void m(void) { return; }\n\
             \x20   m();\n\
             }\n",
        );
        assert_eq!(attrs.len(), 2, "both definitions must be parsed as nested");
        assert!(attrs[0].is_noreturn(), "the first states noreturn");
        assert!(!attrs[1].is_noreturn(), "the second must not inherit it");
    }
}
