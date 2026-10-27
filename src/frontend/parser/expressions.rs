// Expression parsing: precedence climbing from comma expression down to primary.
//
// The expression parser implements C's operator precedence through a table-driven
// approach. Binary operators (logical, bitwise, relational, arithmetic) are handled
// by a shared `parse_binary_expr` method parameterized on `PrecedenceLevel`, which
// maps tokens to operators and recurses to the next-tighter level. This eliminates
// the repetitive per-level parsing functions while keeping the code clear.
//
// Call hierarchy (loosest to tightest binding):
//   parse_expr -> parse_assignment_expr -> parse_conditional_expr
//   -> parse_binary_expr(LogicalOr) -> ... -> parse_binary_expr(Multiplicative)
//   -> parse_cast_expr -> parse_unary_expr -> parse_postfix_expr
//   -> parse_primary_expr

use super::ast::*;
use super::parse::Parser;
use crate::common::source::Span;
use crate::common::types::AddressSpace;
use crate::frontend::lexer::token::{TokenKind, UnicodeCharKind};

/// Build the AST node for an `L"..."` literal, honouring `-fshort-wchar`.
///
/// Under `-fshort-wchar` the C type of `L"..."` is `unsigned short[]` — GCC's
/// `__WCHAR_TYPE__` is `short unsigned int` and its code units are UTF-16 —
/// which is *exactly* the type and encoding this compiler already implements
/// for `u"..."`. Rewriting the node here therefore reuses one existing, tested
/// 16-bit wide-literal path (sema types, `sizeof`, array initialisation, the
/// backend's `.short` emission) instead of adding a second parallel one that
/// would have to be kept in step at every one of those sites.
///
/// Doing it at parse time is the only place where a single edit covers them
/// all: the AST node alone decides the element width from here on.
fn wide_string_expr(text: String, span: Span) -> Expr {
    if crate::common::types::short_wchar() {
        Expr::Char16StringLiteral(text, span)
    } else {
        Expr::WideStringLiteral(text, span)
    }
}

/// C operator precedence levels (loosest to tightest binding).
/// Used by the table-driven binary expression parser.
#[derive(Debug, Clone, Copy)]
enum PrecedenceLevel {
    LogicalOr,
    LogicalAnd,
    BitwiseOr,
    BitwiseXor,
    BitwiseAnd,
    Equality,
    Relational,
    Shift,
    Additive,
    Multiplicative,
}

impl Parser {
    /// Consume any pending `__attribute__((vector_size(N)))` or `__attribute__((ext_vector_type(N)))`
    /// that was parsed during a type specifier and wrap the given TypeSpecifier in a
    /// `TypeSpecifier::Vector` or `TypeSpecifier::ExtVector` node. This is needed for casts,
    /// compound literals, sizeof, and _Alignof where the attribute is not captured in a Declaration.
    pub(super) fn apply_pending_vector_attr(&mut self, ts: TypeSpecifier) -> TypeSpecifier {
        if let Some(total_bytes) = self.attrs.parsing_vector_size.take() {
            return TypeSpecifier::Vector(Box::new(ts), total_bytes);
        }
        if let Some(nelem) = self.attrs.parsing_ext_vector_nelem.take() {
            // Type names (casts/compound literals/sizeof) do not go through the
            // declaration's sema/lowering vector-size resolver. Resolve the
            // actual target width here, including chains of typedefs, rather
            // than guessing LP64 or inventing a four-byte unknown type.
            let total = self
                .ext_vector_element_storage_size(&ts)
                .and_then(|width| nelem.checked_mul(width));
            if total.is_some() {
                return TypeSpecifier::ExtVector(Box::new(ts), nelem);
            }
            self.emit_error(
                "invalid or unresolvable ext_vector_type element size",
                self.peek_span(),
            );
        }
        ts
    }

    /// Storage width of an ext_vector_type lane in a type-name context. This
    /// is deliberately restricted to scalar element types: pointer, array,
    /// record, and vector typedefs cannot be safely treated as four-byte
    /// scalars. Typedef cycles (even in a malformed AST) fail closed.
    fn ext_vector_element_storage_size(&self, ts: &TypeSpecifier) -> Option<usize> {
        let mut spec = ts;
        let mut seen = crate::common::fx_hash::FxHashSet::default();
        loop {
            match spec {
                TypeSpecifier::TypedefName(name) => {
                    if !seen.insert(name) {
                        return None;
                    }
                    spec = self.typedef_type_specs.get(name)?;
                }
                TypeSpecifier::TypeofType(inner) => spec = inner,
                // Only the *existence* of a width matters here (the total is
                // used to validate the element type); the object size and the
                // 10-byte x87 lane stride are resolved by `ExtVectorLayout`.
                TypeSpecifier::LongDouble => return Some(16),
                TypeSpecifier::Char
                | TypeSpecifier::SignedChar
                | TypeSpecifier::UnsignedChar
                | TypeSpecifier::PlainUnsignedChar
                | TypeSpecifier::Short
                | TypeSpecifier::UnsignedShort
                | TypeSpecifier::Int
                | TypeSpecifier::UnsignedInt
                | TypeSpecifier::Signed
                | TypeSpecifier::Unsigned
                | TypeSpecifier::Long
                | TypeSpecifier::UnsignedLong
                | TypeSpecifier::LongLong
                | TypeSpecifier::UnsignedLongLong
                | TypeSpecifier::Float
                | TypeSpecifier::Double
                | TypeSpecifier::Int128
                | TypeSpecifier::UnsignedInt128
                | TypeSpecifier::Bool => return Self::try_sizeof_type_spec(spec),
                _ => return None,
            }
        }
    }

    pub(super) fn parse_expr(&mut self) -> Expr {
        let lhs = self.parse_assignment_expr();
        if matches!(self.peek(), TokenKind::Comma) {
            let span = self.peek_span();
            self.advance();
            let rhs = self.parse_expr();
            Expr::Comma(Box::new(lhs), Box::new(rhs), span)
        } else {
            lhs
        }
    }

    pub(super) fn parse_assignment_expr(&mut self) -> Expr {
        let lhs = self.parse_conditional_expr();

        match self.peek() {
            TokenKind::Assign => {
                let span = self.peek_span();
                self.advance();
                let rhs = self.parse_assignment_expr();
                Expr::Assign(Box::new(lhs), Box::new(rhs), span)
            }
            _ => {
                if let Some(op) = self.compound_assign_op() {
                    let span = self.peek_span();
                    self.advance();
                    let rhs = self.parse_assignment_expr();
                    Expr::CompoundAssign(op, Box::new(lhs), Box::new(rhs), span)
                } else {
                    lhs
                }
            }
        }
    }

    fn parse_conditional_expr(&mut self) -> Expr {
        let cond = self.parse_binary_expr(PrecedenceLevel::LogicalOr);
        if self.consume_if(&TokenKind::Question) {
            let span = cond.span();
            // GNU extension: `cond ? : else_expr` (omitted middle operand)
            // Condition is evaluated once and used as the then-value if truthy.
            if self.peek() == &TokenKind::Colon {
                self.expect_context(&TokenKind::Colon, "in conditional expression");
                let else_expr = self.parse_conditional_expr();
                Expr::GnuConditional(Box::new(cond), Box::new(else_expr), span)
            } else {
                let then_expr = self.parse_expr();
                self.expect_context(&TokenKind::Colon, "in conditional expression");
                let else_expr = self.parse_conditional_expr();
                Expr::Conditional(
                    Box::new(cond),
                    Box::new(then_expr),
                    Box::new(else_expr),
                    span,
                )
            }
        } else {
            cond
        }
    }

    /// Map a token to a binary operator at the current precedence level, if applicable.
    fn token_to_binop(&self, token: &TokenKind, level: PrecedenceLevel) -> Option<BinOp> {
        match (token, level) {
            (TokenKind::PipePipe, PrecedenceLevel::LogicalOr) => Some(BinOp::LogicalOr),
            (TokenKind::AmpAmp, PrecedenceLevel::LogicalAnd) => Some(BinOp::LogicalAnd),
            (TokenKind::Pipe, PrecedenceLevel::BitwiseOr) => Some(BinOp::BitOr),
            (TokenKind::Caret, PrecedenceLevel::BitwiseXor) => Some(BinOp::BitXor),
            (TokenKind::Amp, PrecedenceLevel::BitwiseAnd) => Some(BinOp::BitAnd),
            (TokenKind::EqualEqual, PrecedenceLevel::Equality) => Some(BinOp::Eq),
            (TokenKind::BangEqual, PrecedenceLevel::Equality) => Some(BinOp::Ne),
            (TokenKind::Less, PrecedenceLevel::Relational) => Some(BinOp::Lt),
            (TokenKind::LessEqual, PrecedenceLevel::Relational) => Some(BinOp::Le),
            (TokenKind::Greater, PrecedenceLevel::Relational) => Some(BinOp::Gt),
            (TokenKind::GreaterEqual, PrecedenceLevel::Relational) => Some(BinOp::Ge),
            (TokenKind::LessLess, PrecedenceLevel::Shift) => Some(BinOp::Shl),
            (TokenKind::GreaterGreater, PrecedenceLevel::Shift) => Some(BinOp::Shr),
            (TokenKind::Plus, PrecedenceLevel::Additive) => Some(BinOp::Add),
            (TokenKind::Minus, PrecedenceLevel::Additive) => Some(BinOp::Sub),
            (TokenKind::Star, PrecedenceLevel::Multiplicative) => Some(BinOp::Mul),
            (TokenKind::Slash, PrecedenceLevel::Multiplicative) => Some(BinOp::Div),
            (TokenKind::Percent, PrecedenceLevel::Multiplicative) => Some(BinOp::Mod),
            _ => None,
        }
    }

    /// Parse a left-associative binary expression at the given precedence level.
    /// This is the shared core that replaces 10 nearly-identical parsing functions.
    fn parse_binary_expr(&mut self, level: PrecedenceLevel) -> Expr {
        let span = self.peek_span();
        if !self.enter_expr_frame(span) {
            return Expr::IntLiteral(0, span);
        }
        let r = self.parse_binary_expr_inner(level);
        self.exit_parser_frame();
        r
    }

    #[inline(always)]
    fn parse_binary_expr_inner(&mut self, level: PrecedenceLevel) -> Expr {
        let mut lhs = self.parse_next_tighter(level);
        while let Some(op) = self.token_to_binop(self.peek(), level) {
            let span = self.peek_span();
            self.advance();
            let rhs = self.parse_next_tighter(level);
            lhs = Expr::BinaryOp(op, Box::new(lhs), Box::new(rhs), span);
        }
        lhs
    }

    /// Parse the next tighter precedence level.
    fn parse_next_tighter(&mut self, level: PrecedenceLevel) -> Expr {
        let span = self.peek_span();
        if !self.enter_expr_frame(span) {
            return Expr::IntLiteral(0, span);
        }
        let r = self.parse_next_tighter_inner(level);
        self.exit_parser_frame();
        r
    }

    #[inline(always)]
    fn parse_next_tighter_inner(&mut self, level: PrecedenceLevel) -> Expr {
        match level {
            PrecedenceLevel::LogicalOr => self.parse_binary_expr(PrecedenceLevel::LogicalAnd),
            PrecedenceLevel::LogicalAnd => self.parse_binary_expr(PrecedenceLevel::BitwiseOr),
            PrecedenceLevel::BitwiseOr => self.parse_binary_expr(PrecedenceLevel::BitwiseXor),
            PrecedenceLevel::BitwiseXor => self.parse_binary_expr(PrecedenceLevel::BitwiseAnd),
            PrecedenceLevel::BitwiseAnd => self.parse_binary_expr(PrecedenceLevel::Equality),
            PrecedenceLevel::Equality => self.parse_binary_expr(PrecedenceLevel::Relational),
            PrecedenceLevel::Relational => self.parse_binary_expr(PrecedenceLevel::Shift),
            PrecedenceLevel::Shift => self.parse_binary_expr(PrecedenceLevel::Additive),
            PrecedenceLevel::Additive => self.parse_binary_expr(PrecedenceLevel::Multiplicative),
            PrecedenceLevel::Multiplicative => self.parse_cast_expr(),
        }
    }

    /// Parse a cast expression: (type-name)expr, compound literal (type-name){...},
    /// or fall through to unary expression.
    pub(super) fn parse_cast_expr(&mut self) -> Expr {
        let span = self.peek_span();
        if !self.enter_expr_frame(span) {
            return Expr::IntLiteral(0, span);
        }
        let r = self.parse_cast_expr_inner();
        self.exit_parser_frame();
        r
    }

    /// Does the type name starting at the current token qualify its POINTEE
    /// with `const` or `volatile`? Only qualifiers written before the first `*`
    /// of the type name qualify the pointee: `const void *` does, while
    /// `void *const` and `void *volatile` qualify the pointer itself, and the
    /// cast type `void *` stays unqualified (C23 6.3.2.3p3, GCC 14). The scan
    /// stops at that `*` or at the type name's closing `)`.
    fn type_name_qualifiers(&self) -> CastQualifiers {
        let mut q = CastQualifiers::default();
        let mut depth = 0usize;
        let mut stars = 0usize;
        let mut before_first_star = true;
        let mut volatile_before_star = false;
        for n in 0..64 {
            match self.peek_nth(n) {
                TokenKind::Const if before_first_star => {
                    q.pointee_qualified = true;
                    q.base_const = true;
                }
                TokenKind::Volatile if before_first_star => {
                    q.pointee_qualified = true;
                    if depth == 0 {
                        volatile_before_star = true;
                    }
                }
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                TokenKind::Star if depth == 0 => {
                    stars += 1;
                    before_first_star = false;
                }
                _ => {}
            }
        }
        // `volatile` must qualify the pointee of a single-level pointer:
        // `(volatile T **)` has a plain `T *` pointee, not a volatile one.
        q.pointee_volatile = volatile_before_star && stars == 1;
        q
    }

    /// Scan top-level storage-class and const specifiers inside `( ... )` before `)`.
    /// Returns `(storage, has_any_storage_class, invalid_storage_class_name)`.
    fn scan_compound_literal_storage(
        &self,
    ) -> (
        crate::frontend::parser::ast::CompoundLiteralStorage,
        bool,
        Option<&'static str>,
    ) {
        let mut st = crate::frontend::parser::ast::CompoundLiteralStorage::default();
        let mut any_sc = false;
        let mut invalid_sc = None;
        let mut depth = 0usize;
        let mut stars = 0usize;
        let mut const_before_star = false;
        let mut const_after_last_star = false;
        for n in 0..64 {
            match self.peek_nth(n) {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                TokenKind::Star if depth == 0 => {
                    stars += 1;
                    const_after_last_star = false;
                }
                TokenKind::Const if depth == 0 => {
                    if stars == 0 {
                        const_before_star = true;
                    } else {
                        const_after_last_star = true;
                    }
                }
                TokenKind::Static if depth == 0 => {
                    st.is_static = true;
                    any_sc = true;
                }
                TokenKind::Constexpr if depth == 0 => {
                    st.is_constexpr = true;
                    st.is_const = true;
                    any_sc = true;
                }
                TokenKind::Register if depth == 0 => {
                    st.is_register = true;
                    any_sc = true;
                }
                TokenKind::ThreadLocal if depth == 0 => {
                    st.is_thread_local = true;
                    any_sc = true;
                }
                TokenKind::Extern if depth == 0 => {
                    any_sc = true;
                    invalid_sc.get_or_insert("extern");
                }
                TokenKind::Typedef if depth == 0 => {
                    any_sc = true;
                    invalid_sc.get_or_insert("typedef");
                }
                TokenKind::Auto if depth == 0 => {
                    any_sc = true;
                    invalid_sc.get_or_insert("auto");
                }
                _ => {}
            }
        }
        if (stars == 0 && const_before_star) || (stars > 0 && const_after_last_star) {
            st.is_const = true;
        }
        (st, any_sc, invalid_sc)
    }

    fn validate_compound_literal_storage(
        &mut self,
        st: &crate::frontend::parser::ast::CompoundLiteralStorage,
        invalid_sc: Option<&'static str>,
        span: Span,
    ) {
        if let Some(bad) = invalid_sc {
            self.emit_error(
                format!("compound literal cannot have '{bad}' storage class"),
                span,
            );
            return;
        }
        if (st.is_register && (st.is_static || st.is_thread_local))
            || (st.is_thread_local && st.is_constexpr)
        {
            self.emit_error(
                "conflicting storage-class specifiers in compound literal".to_string(),
                span,
            );
        }
    }

    #[inline(always)]
    fn parse_cast_expr_inner(&mut self) -> Expr {
        if matches!(self.peek(), TokenKind::LParen) {
            let save = self.pos;
            let save_flags = self.attrs.flags;
            let save_vector_size = self.attrs.parsing_vector_size.take();
            let save_ext_vector = self.attrs.parsing_ext_vector_nelem.take();
            self.advance();
            if self.is_type_specifier() {
                let qualifiers = self.type_name_qualifiers();
                let (cl_storage, any_sc, invalid_sc) = self.scan_compound_literal_storage();
                if let Some(mut result_type) = self.parse_nested_type_name() {
                    // If __attribute__((vector_size(N))) was parsed, wrap the type
                    result_type = self.apply_pending_vector_attr(result_type);
                    if matches!(self.peek(), TokenKind::RParen) {
                        let span = self.peek_span();
                        // The cast expression's span covers the whole
                        // `(type)expr` form, starting at the `(`: storing only
                        // the closing paren put GCC-style diagnostics on the
                        // `)` instead of the expression start (measured
                        // against gcc-16.2.0 columns for
                        // `g((double*)0)` and `return (char*)0`).
                        let lparen_span = self.tokens[save].span;
                        let cast_span = crate::common::source::Span::new(
                            lparen_span.start,
                            span.end,
                            span.file_id,
                        );
                        self.advance();
                        // Check for compound literal: (type){...}
                        if matches!(self.peek(), TokenKind::LBrace) {
                            self.validate_compound_literal_storage(&cl_storage, invalid_sc, span);
                            self.attrs.flags = save_flags;
                            self.attrs.parsing_vector_size = save_vector_size;
                            self.attrs.parsing_ext_vector_nelem = save_ext_vector;
                            let init = self.parse_initializer();
                            let lit = Expr::CompoundLiteral(
                                result_type,
                                Box::new(init),
                                span,
                                cl_storage,
                            );
                            return self.parse_postfix_ops(lit);
                        }
                        if any_sc {
                            self.emit_error(
                                "storage class specified for type name".to_string(),
                                span,
                            );
                        }
                        self.attrs.flags = save_flags;
                        self.attrs.parsing_vector_size = save_vector_size;
                        self.attrs.parsing_ext_vector_nelem = save_ext_vector;
                        let expr = self.parse_cast_expr();
                        return Expr::Cast(result_type, Box::new(expr), cast_span, qualifiers);
                    }
                }
            }
            self.pos = save;
            self.attrs.flags = save_flags;
            self.attrs.parsing_vector_size = save_vector_size;
            self.attrs.parsing_ext_vector_nelem = save_ext_vector;
        }
        self.parse_unary_expr()
    }

    fn parse_unary_expr(&mut self) -> Expr {
        let span = self.peek_span();
        if !self.enter_expr_frame(span) {
            return Expr::IntLiteral(0, span);
        }
        let r = self.parse_unary_expr_inner();
        self.exit_parser_frame();
        r
    }

    #[inline(always)]
    fn parse_unary_expr_inner(&mut self) -> Expr {
        match self.peek() {
            TokenKind::AmpAmp => {
                // GCC extension: &&label (address of label, for computed goto)
                let span = self.peek_span();
                if self.pos + 1 < self.tokens.len() {
                    if let TokenKind::Identifier(ref name) = *self.peek_nth(1) {
                        let label_name = name.clone();
                        self.advance(); // consume &&
                        self.advance(); // consume identifier
                        return Expr::LabelAddr(label_name, span);
                    }
                }
                self.parse_postfix_expr()
            }
            TokenKind::RealPart => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_cast_expr();
                Expr::UnaryOp(UnaryOp::RealPart, Box::new(expr), span)
            }
            TokenKind::ImagPart => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_cast_expr();
                Expr::UnaryOp(UnaryOp::ImagPart, Box::new(expr), span)
            }
            TokenKind::PlusPlus => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_unary_expr();
                Expr::UnaryOp(UnaryOp::PreInc, Box::new(expr), span)
            }
            TokenKind::MinusMinus => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_unary_expr();
                Expr::UnaryOp(UnaryOp::PreDec, Box::new(expr), span)
            }
            TokenKind::Plus => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_cast_expr();
                Expr::UnaryOp(UnaryOp::Plus, Box::new(expr), span)
            }
            TokenKind::Minus => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_cast_expr();
                Expr::UnaryOp(UnaryOp::Neg, Box::new(expr), span)
            }
            TokenKind::Tilde => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_cast_expr();
                Expr::UnaryOp(UnaryOp::BitNot, Box::new(expr), span)
            }
            TokenKind::Bang => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_cast_expr();
                Expr::UnaryOp(UnaryOp::LogicalNot, Box::new(expr), span)
            }
            TokenKind::Amp => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_cast_expr();
                Expr::AddressOf(Box::new(expr), span)
            }
            TokenKind::Star => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_cast_expr();
                Expr::Deref(Box::new(expr), span)
            }
            TokenKind::Sizeof => self.parse_sizeof_or_countof_expr(false),
            TokenKind::Countof => self.parse_sizeof_or_countof_expr(true),
            TokenKind::Alignof => {
                let span = self.peek_span();
                self.advance();
                // _Alignof(type) - C11 standard, returns minimum ABI alignment
                let open = self.peek_span();
                self.expect_context(&TokenKind::LParen, "after '_Alignof'");
                if let Some(mut result_type) = self.parse_nested_type_name() {
                    result_type = self.apply_pending_vector_attr(result_type);
                    self.expect_closing(&TokenKind::RParen, open);
                    Expr::Alignof(result_type, span)
                } else {
                    // GCC extension: __alignof__(expr) - alignment of expression's type
                    let expr = self.parse_assignment_expr();
                    self.expect_closing(&TokenKind::RParen, open);
                    Expr::AlignofExpr(Box::new(expr), span)
                }
            }
            TokenKind::GnuAlignof => {
                let span = self.peek_span();
                self.advance();
                // __alignof / __alignof__ - GCC extension, returns preferred alignment
                let open = self.peek_span();
                self.expect_context(&TokenKind::LParen, "after '__alignof__'");
                if let Some(mut result_type) = self.parse_nested_type_name() {
                    result_type = self.apply_pending_vector_attr(result_type);
                    self.expect_closing(&TokenKind::RParen, open);
                    Expr::GnuAlignof(result_type, span)
                } else {
                    let expr = self.parse_assignment_expr();
                    self.expect_closing(&TokenKind::RParen, open);
                    Expr::GnuAlignofExpr(Box::new(expr), span)
                }
            }
            _ => self.parse_postfix_expr(),
        }
    }

    /// Parse `sizeof` or `_Countof` expression. Handles both `op(type-name)` and `op expr`.
    fn parse_sizeof_or_countof_expr(&mut self, is_countof: bool) -> Expr {
        let span = self.peek_span();
        self.advance(); // consume 'sizeof' or '_Countof'
        let wrap = |arg: SizeofArg| {
            if is_countof {
                Expr::Countof(Box::new(arg), span)
            } else {
                Expr::Sizeof(Box::new(arg), span)
            }
        };
        if matches!(self.peek(), TokenKind::LParen) {
            let save = self.pos;
            let save_flags = self.attrs.flags;
            let save_vector_size = self.attrs.parsing_vector_size.take();
            let save_ext_vector = self.attrs.parsing_ext_vector_nelem.take();
            self.advance();
            if self.is_type_specifier() {
                let (cl_storage, any_sc, invalid_sc) = self.scan_compound_literal_storage();
                // parse_nested_type_name scopes the pending address-space
                // qualifier (see its doc comment); the sizeof(type-name)
                // argument is a nested type-name exactly like a cast's.
                if let Some(mut result_type) = self.parse_nested_type_name() {
                    // If __attribute__((vector_size(N))) was parsed, wrap the type
                    result_type = self.apply_pending_vector_attr(result_type);
                    if matches!(self.peek(), TokenKind::RParen) {
                        let lit_span = self.peek_span();
                        self.expect(&TokenKind::RParen);
                        // `sizeof (T){...}` is `sizeof unary-expression`
                        // applied to a compound literal (C11 6.5.3, 6.5.2.5),
                        // postfix operators included: `sizeof (T[]){1, 2}`
                        // is the literal's size, `sizeof (S){0}.m` a member's.
                        self.attrs.flags = save_flags;
                        self.attrs.parsing_vector_size = save_vector_size;
                        self.attrs.parsing_ext_vector_nelem = save_ext_vector;
                        let arg = if matches!(self.peek(), TokenKind::LBrace) {
                            self.validate_compound_literal_storage(
                                &cl_storage,
                                invalid_sc,
                                lit_span,
                            );
                            let init = self.parse_initializer();
                            let lit = Expr::CompoundLiteral(
                                result_type,
                                Box::new(init),
                                lit_span,
                                cl_storage,
                            );
                            SizeofArg::Expr(self.parse_postfix_ops(lit))
                        } else {
                            if any_sc {
                                self.emit_error(
                                    "storage class specified for type name".to_string(),
                                    lit_span,
                                );
                            }
                            SizeofArg::Type(result_type)
                        };
                        return wrap(arg);
                    }
                }
            }
            self.pos = save;
            self.attrs.flags = save_flags;
            self.attrs.parsing_vector_size = save_vector_size;
            self.attrs.parsing_ext_vector_nelem = save_ext_vector;
        }
        let expr = self.parse_unary_expr();
        wrap(SizeofArg::Expr(expr))
    }

    fn try_parse_compound_literal(&mut self) -> Option<Expr> {
        if !matches!(self.peek(), TokenKind::LParen) {
            return None;
        }
        let save = self.pos;
        let save_flags = self.attrs.flags;
        let save_vector_size = self.attrs.parsing_vector_size.take();
        let save_ext_vector = self.attrs.parsing_ext_vector_nelem.take();
        self.advance();
        if self.is_type_specifier() {
            let (cl_storage, _any_sc, invalid_sc) = self.scan_compound_literal_storage();
            if let Some(mut result_type) = self.parse_nested_type_name() {
                result_type = self.apply_pending_vector_attr(result_type);
                if matches!(self.peek(), TokenKind::RParen) {
                    let span = self.peek_span();
                    self.advance();
                    if matches!(self.peek(), TokenKind::LBrace) {
                        self.validate_compound_literal_storage(&cl_storage, invalid_sc, span);
                        self.attrs.flags = save_flags;
                        self.attrs.parsing_vector_size = save_vector_size;
                        self.attrs.parsing_ext_vector_nelem = save_ext_vector;
                        let init = self.parse_initializer();
                        return Some(Expr::CompoundLiteral(
                            result_type,
                            Box::new(init),
                            span,
                            cl_storage,
                        ));
                    }
                }
            }
        }
        self.pos = save;
        self.attrs.flags = save_flags;
        self.attrs.parsing_vector_size = save_vector_size;
        self.attrs.parsing_ext_vector_nelem = save_ext_vector;
        None
    }

    fn parse_postfix_expr(&mut self) -> Expr {
        let span = self.peek_span();
        if !self.enter_expr_frame(span) {
            return Expr::IntLiteral(0, span);
        }
        let expr = if let Some(cl) = self.try_parse_compound_literal() {
            cl
        } else {
            self.parse_primary_expr()
        };
        let r = self.parse_postfix_ops(expr);
        self.exit_parser_frame();
        r
    }

    /// Parse postfix operators ([], ., ->, ++, --, function call) applied to an initial expression.
    fn parse_postfix_ops(&mut self, mut expr: Expr) -> Expr {
        loop {
            match self.peek() {
                TokenKind::LParen => {
                    // Function call
                    let open = self.peek_span();
                    self.advance();
                    let mut args = Vec::with_capacity(8);
                    if !matches!(self.peek(), TokenKind::RParen) {
                        args.push(self.parse_assignment_expr());
                        while self.consume_if(&TokenKind::Comma) {
                            args.push(self.parse_assignment_expr());
                        }
                    }
                    self.expect_closing(&TokenKind::RParen, open);
                    expr = Expr::FunctionCall(Box::new(expr), args, open);
                }
                TokenKind::LBracket => {
                    let open = self.peek_span();
                    self.advance();
                    let index = self.parse_expr();
                    self.expect_closing(&TokenKind::RBracket, open);
                    expr = Expr::ArraySubscript(Box::new(expr), Box::new(index), open);
                }
                TokenKind::Dot => {
                    let span = self.peek_span();
                    self.advance();
                    let field = if let TokenKind::Identifier(name) = self.peek() {
                        let name = name.clone();
                        self.advance();
                        name
                    } else {
                        String::new()
                    };
                    expr = Expr::MemberAccess(Box::new(expr), field, span);
                }
                TokenKind::Arrow => {
                    let span = self.peek_span();
                    self.advance();
                    let field = if let TokenKind::Identifier(name) = self.peek() {
                        let name = name.clone();
                        self.advance();
                        name
                    } else {
                        String::new()
                    };
                    expr = Expr::PointerMemberAccess(Box::new(expr), field, span);
                }
                TokenKind::PlusPlus => {
                    let span = self.peek_span();
                    self.advance();
                    expr = Expr::PostfixOp(PostfixOp::PostInc, Box::new(expr), span);
                }
                TokenKind::MinusMinus => {
                    let span = self.peek_span();
                    self.advance();
                    expr = Expr::PostfixOp(PostfixOp::PostDec, Box::new(expr), span);
                }
                _ => break,
            }
        }

        expr
    }

    fn parse_primary_expr(&mut self) -> Expr {
        let span = self.peek_span();
        if !self.enter_expr_frame(span) {
            return Expr::IntLiteral(0, span);
        }
        let r = self.parse_primary_expr_inner();
        self.exit_parser_frame();
        r
    }

    #[inline(always)]
    fn parse_primary_expr_inner(&mut self) -> Expr {
        match self.peek() {
            TokenKind::IntLiteral(val) => {
                let val = *val;
                let span = self.peek_span();
                self.advance();
                Expr::IntLiteral(val, span)
            }
            TokenKind::UIntLiteral(val) => {
                let val = *val;
                let span = self.peek_span();
                self.advance();
                Expr::UIntLiteral(val, span)
            }
            TokenKind::LongLiteral(val) => {
                let val = *val;
                let span = self.peek_span();
                self.advance();
                Expr::LongLiteral(val, span)
            }
            TokenKind::ULongLiteral(val) => {
                let val = *val;
                let span = self.peek_span();
                self.advance();
                Expr::ULongLiteral(val, span)
            }
            TokenKind::LongLongLiteral(val) => {
                let val = *val;
                let span = self.peek_span();
                self.advance();
                Expr::LongLongLiteral(val, span)
            }
            TokenKind::ULongLongLiteral(val) => {
                let val = *val;
                let span = self.peek_span();
                self.advance();
                Expr::ULongLongLiteral(val, span)
            }
            TokenKind::FloatLiteral(val) => {
                let val = *val;
                let span = self.peek_span();
                self.advance();
                Expr::FloatLiteral(val, span)
            }
            TokenKind::FloatLiteralF32(val) => {
                let val = *val;
                let span = self.peek_span();
                self.advance();
                Expr::FloatLiteralF32(val, span)
            }
            TokenKind::FloatLiteralLongDouble(val, bytes) => {
                let val = *val;
                let bytes = *bytes;
                let span = self.peek_span();
                self.advance();
                Expr::FloatLiteralLongDouble(val, bytes, span)
            }
            TokenKind::FloatLiteralF128(val, bytes) => {
                let val = *val;
                let bytes = *bytes;
                let span = self.peek_span();
                self.advance();
                Expr::FloatLiteralF128(val, bytes, span)
            }
            TokenKind::FloatLiteralDecimal(width, bytes) => {
                let width = *width;
                let bytes = *bytes;
                let span = self.peek_span();
                self.advance();
                Expr::FloatLiteralDecimal(width, bytes, span)
            }
            TokenKind::FloatLiteralHalf(is_bf16, bits) => {
                let is_bf16 = *is_bf16;
                let bits = *bits;
                let span = self.peek_span();
                self.advance();
                Expr::FloatLiteralHalf(is_bf16, bits, span)
            }
            TokenKind::ImaginaryLiteral(val) => {
                let val = *val;
                let span = self.peek_span();
                self.advance();
                Expr::ImaginaryLiteral(val, span)
            }
            TokenKind::ImaginaryLiteralF32(val) => {
                let val = *val;
                let span = self.peek_span();
                self.advance();
                Expr::ImaginaryLiteralF32(val, span)
            }
            TokenKind::ImaginaryLiteralLongDouble(val, bytes) => {
                let val = *val;
                let bytes = *bytes;
                let span = self.peek_span();
                self.advance();
                Expr::ImaginaryLiteralLongDouble(val, bytes, span)
            }
            TokenKind::StringLiteral(s) => {
                let mut result = String::with_capacity(s.len() + 16);
                result.push_str(s);
                let span = self.peek_span();
                self.advance();
                // Concatenate adjacent string literals. If any is wide/char16, result upgrades.
                let mut is_wide = false;
                let mut is_char16 = false;
                loop {
                    match self.peek() {
                        TokenKind::StringLiteral(s2) => {
                            result.push_str(s2);
                            self.advance();
                        }
                        TokenKind::WideStringLiteral(s2) => {
                            result.push_str(s2);
                            is_wide = true;
                            self.advance();
                        }
                        TokenKind::Char16StringLiteral(s2) => {
                            result.push_str(s2);
                            is_char16 = true;
                            self.advance();
                        }
                        _ => break,
                    }
                }
                if is_wide {
                    wide_string_expr(result, span)
                } else if is_char16 {
                    Expr::Char16StringLiteral(result, span)
                } else {
                    Expr::StringLiteral(result, span)
                }
            }
            TokenKind::WideStringLiteral(s) => {
                let mut result = String::with_capacity(s.len() + 16);
                result.push_str(s);
                let span = self.peek_span();
                self.advance();
                // Concatenate adjacent string literals (wide + narrow = wide)
                while let TokenKind::StringLiteral(s2)
                | TokenKind::WideStringLiteral(s2)
                | TokenKind::Char16StringLiteral(s2) = self.peek()
                {
                    result.push_str(s2);
                    self.advance();
                }
                wide_string_expr(result, span)
            }
            TokenKind::Char16StringLiteral(s) => {
                let mut result = s.clone();
                let span = self.peek_span();
                self.advance();
                // Concatenate adjacent string literals (char16 + narrow = char16, char16 + wide = wide)
                let mut is_wide = false;
                loop {
                    match self.peek() {
                        TokenKind::StringLiteral(s2) | TokenKind::Char16StringLiteral(s2) => {
                            result.push_str(s2);
                            self.advance();
                        }
                        TokenKind::WideStringLiteral(s2) => {
                            result.push_str(s2);
                            is_wide = true;
                            self.advance();
                        }
                        _ => break,
                    }
                }
                if is_wide {
                    wide_string_expr(result, span)
                } else {
                    Expr::Char16StringLiteral(result, span)
                }
            }
            TokenKind::CharLiteral(c) => {
                let c = *c;
                let span = self.peek_span();
                self.advance();
                Expr::CharLiteral(c, span)
            }
            // C23 `true` / `false`: integer constants of type bool with value
            // 1 / 0 (C23 6.4.4.6), exactly `(bool)1` / `(bool)0`.
            TokenKind::True | TokenKind::False => {
                let value = i64::from(matches!(self.peek(), TokenKind::True));
                let span = self.peek_span();
                self.advance();
                Expr::Cast(
                    TypeSpecifier::Bool,
                    Box::new(Expr::IntLiteral(value, span)),
                    span,
                    CastQualifiers::default(),
                )
            }
            // C23 `nullptr` (6.4.4.6): the null pointer constant of type
            // `nullptr_t`, i.e. the null value `(nullptr_t)0`.
            TokenKind::Nullptr => {
                let span = self.peek_span();
                self.advance();
                Expr::Cast(
                    TypeSpecifier::NullPtrT,
                    Box::new(Expr::IntLiteral(0, span)),
                    span,
                    CastQualifiers::default(),
                )
            }
            // `u8'x'`, `u'x'`, `U'x'`: the code unit in its unsigned type.
            TokenKind::UnicodeCharLiteral(value, kind) => {
                let ty = match kind {
                    UnicodeCharKind::Utf8 => TypeSpecifier::UnsignedChar,
                    UnicodeCharKind::Utf16 => TypeSpecifier::UnsignedShort,
                    UnicodeCharKind::Utf32 => TypeSpecifier::UnsignedInt,
                };
                let value = i64::from(*value);
                let span = self.peek_span();
                self.advance();
                Expr::Cast(
                    ty,
                    Box::new(Expr::IntLiteral(value, span)),
                    span,
                    CastQualifiers::default(),
                )
            }
            TokenKind::Identifier(name) => {
                let name = name.clone();
                let span = self.peek_span();
                self.advance();
                if name == "__builtin_convertvector" && matches!(self.peek(), TokenKind::LParen) {
                    return self.parse_convertvector_expr(span);
                }
                if name == "__builtin_bit_cast" && matches!(self.peek(), TokenKind::LParen) {
                    return self.parse_bit_cast_expr(span);
                }
                Expr::Identifier(name, span)
            }
            TokenKind::LParen => {
                let open = self.peek_span();
                self.advance();
                // Check for GCC statement expression: ({ stmt; stmt; expr; })
                if matches!(self.peek(), TokenKind::LBrace) {
                    let span = self.peek_span();
                    let compound = self.parse_compound_stmt();
                    self.expect_closing(&TokenKind::RParen, open);
                    Expr::StmtExpr(compound, span)
                } else {
                    let expr = self.parse_expr();
                    self.expect_closing(&TokenKind::RParen, open);
                    expr
                }
            }
            TokenKind::Generic => self.parse_generic_selection(),
            TokenKind::Asm => {
                // GCC asm expression
                let span = self.peek_span();
                self.advance();
                self.consume_if(&TokenKind::Volatile);
                if matches!(self.peek(), TokenKind::LParen) {
                    self.skip_balanced_parens();
                }
                Expr::IntLiteral(0, span)
            }
            TokenKind::BuiltinVaArg => self.parse_va_arg_expr(),
            TokenKind::BuiltinTypesCompatibleP => self.parse_types_compatible_expr(),
            TokenKind::Typeof | TokenKind::TypeofUnqual => {
                let span = self.peek_span();
                self.advance();
                if matches!(self.peek(), TokenKind::LParen) {
                    self.skip_balanced_parens();
                }
                Expr::IntLiteral(0, span)
            }
            TokenKind::Builtin => {
                let span = self.peek_span();
                self.advance();
                Expr::Identifier("__builtin_va_list".to_string(), span)
            }
            TokenKind::Extension => {
                self.advance();
                self.parse_cast_expr()
            }
            _ => {
                let span = self.peek_span();
                self.emit_error(format!("expected expression before {}", self.peek()), span);
                self.advance();
                Expr::IntLiteral(0, span)
            }
        }
    }

    fn parse_convertvector_expr(&mut self, span: Span) -> Expr {
        let open = self.peek_span();
        self.advance();
        let src_expr = self.parse_assignment_expr();
        self.expect_context(
            &TokenKind::Comma,
            "between '__builtin_convertvector' arguments",
        );
        let type_spec = self.parse_va_arg_type();
        self.expect_closing(&TokenKind::RParen, open);
        Expr::ConvertVector(Box::new(src_expr), type_spec, span)
    }

    fn parse_bit_cast_expr(&mut self, span: Span) -> Expr {
        let open = self.peek_span();
        self.advance();
        let type_spec = self.parse_va_arg_type();
        self.expect_context(&TokenKind::Comma, "between '__builtin_bit_cast' arguments");
        let src_expr = self.parse_assignment_expr();
        self.expect_closing(&TokenKind::RParen, open);
        Expr::BitCast(type_spec, Box::new(src_expr), span)
    }

    fn parse_va_arg_expr(&mut self) -> Expr {
        let span = self.peek_span();
        self.advance();
        let open = self.peek_span();
        self.expect_context(&TokenKind::LParen, "after '__builtin_va_arg'");
        let ap_expr = self.parse_assignment_expr();
        self.expect_context(&TokenKind::Comma, "between '__builtin_va_arg' arguments");
        let type_spec = self.parse_va_arg_type();
        self.expect_closing(&TokenKind::RParen, open);
        Expr::VaArg(Box::new(ap_expr), type_spec, span)
    }

    fn parse_types_compatible_expr(&mut self) -> Expr {
        let span = self.peek_span();
        self.advance();
        let open = self.peek_span();
        self.expect_context(&TokenKind::LParen, "after '__builtin_types_compatible_p'");
        let (type1, qualifiers1) = self.parse_compatible_type();
        self.expect_context(
            &TokenKind::Comma,
            "between '__builtin_types_compatible_p' arguments",
        );
        let (type2, qualifiers2) = self.parse_compatible_type();
        self.expect_closing(&TokenKind::RParen, open);
        Expr::BuiltinTypesCompatibleP(type1, type2, qualifiers1, qualifiers2, span)
    }

    /// Parse _Generic(controlling_expr, type: expr, ..., default: expr)
    fn parse_generic_selection(&mut self) -> Expr {
        let span = self.peek_span();
        self.advance(); // consume _Generic
        let open = self.peek_span();
        self.expect_context(&TokenKind::LParen, "after '_Generic'");
        // C2y N3260 (GCC 16.2 and Clang 23.1.3 accept it in C23 mode): the
        // controlling operand may be a type name, which is not converted.
        let controlling = if self.is_type_specifier() {
            let operand_span = self.peek_span();
            match self.parse_generic_type_name() {
                Some((ts, is_const)) => Expr::TypeOperand(Box::new(ts), is_const, operand_span),
                None => self.parse_assignment_expr(),
            }
        } else {
            self.parse_assignment_expr()
        };
        self.expect_context(&TokenKind::Comma, "after '_Generic' controlling expression");
        let mut associations = Vec::with_capacity(8);
        loop {
            if matches!(self.peek(), TokenKind::RParen) {
                break;
            }
            let (type_spec, is_const) = if matches!(self.peek(), TokenKind::Default) {
                self.advance();
                (None, false)
            } else {
                match self.parse_generic_type_name() {
                    Some((ts, is_const)) => (Some(ts), is_const),
                    None => (None, false),
                }
            };
            self.expect_context(&TokenKind::Colon, "in '_Generic' association");
            let expr = self.parse_assignment_expr();
            associations.push(GenericAssociation {
                type_spec,
                expr,
                is_const,
            });
            if !self.consume_if(&TokenKind::Comma) {
                break;
            }
        }
        self.expect_closing(&TokenKind::RParen, open);
        Expr::GenericSelection(Box::new(controlling), associations, span)
    }

    /// A `_Generic` type name -- an association type or a type operand --
    /// with whether `const` qualifies its base type (the pointee, for a
    /// pointer such as `const int *`), which `GenericAssociation::is_const`
    /// records because `CType` carries no qualifiers.
    fn parse_generic_type_name(&mut self) -> Option<(TypeSpecifier, bool)> {
        // Reset parsing_const so a `const` in this type name is detected.
        let saved_const = self.attrs.parsing_const();
        self.attrs.set_const(false);
        // A nested type name: scope the pending address-space qualifier so
        // its abstract declarator cannot steal an enclosing declaration's
        // `__seg_gs`/`__seg_fs`; the enclosing qualifier is restored on every
        // path.
        let enclosing_address_space = self.attrs.parsing_address_space;
        self.attrs.parsing_address_space = AddressSpace::Default;
        let result = self.parse_type_specifier().map(|ts| {
            // `const` seen before the pointer declarators qualifies the base
            // type: for `const int *`, the pointee.
            let base_const = self.attrs.parsing_const();
            (self.parse_abstract_declarator_suffix(ts), base_const)
        });
        self.attrs.parsing_address_space = enclosing_address_space;
        self.attrs.set_const(saved_const);
        result
    }
}
