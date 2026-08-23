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
        let (name, derived, decl_mode, _decl_common, decl_aligned, _) =
            self.parse_declarator_with_attrs();
        let (_post_ctor, _post_dtor, post_mode, _post_common, post_aligned, _first_asm_reg) =
            self.parse_asm_and_attributes();
        let mode_kind = decl_mode.or(post_mode);
        let mut alignment = decl_aligned;
        if let Some(a) = post_aligned {
            alignment = Some(alignment.map_or(a, |prev: usize| prev.max(a)));
        }

        // A nested function definition requires a plain function declarator
        // (a name plus a trailing Function(...) derivation — not a pointer
        // to function, not an array) followed by '{' (or K&R parameter
        // declarations then '{').
        let name = name?;
        if derived.is_empty() {
            return None;
        }
        if !matches!(derived.last(), Some(crate::frontend::parser::ast::DerivedDeclarator::Function(_, _))) {
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

        let mut attrs = crate::frontend::parser::ast::FunctionAttributes::new();
        attrs.set_static(is_static);
        attrs.set_inline(is_inline);
        attrs.set_always_inline(is_always_inline);
        attrs.set_noinline(is_noinline);

        // __attribute__((mode(...))) applies to the return type.
        let type_spec = if let Some(mk) = mode_kind {
            mk.apply(type_spec)
        } else {
            type_spec
        };
        let return_type = self.build_return_type(type_spec, &derived);
        // Alignment/asm-register attributes have no meaning on a function
        // definition; they are intentionally dropped (GCC warns similarly).

        Some(FunctionDef {
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
