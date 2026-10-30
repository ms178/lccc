// Statement parsing: all C statement types including inline assembly.
//
// Handles: return, if/else, while, do-while, for, switch/case/default,
// break, continue, goto (including computed goto), labels, compound
// statements, and inline assembly (GCC syntax).

use super::asm_validate;
use super::ast::*;
use super::parse::Parser;
use crate::common::encoding::narrow_string_byte_carrier_to_utf8;
use crate::common::source::Span;
use crate::frontend::lexer::token::TokenKind;

impl Parser {
    pub(super) fn parse_compound_stmt(&mut self) -> CompoundStmt {
        let open_brace = self.peek_span();
        // Block-budget frame: nested `{ { ... } }` recurse one frame per
        // brace level; on exhaustion consume NOTHING and hand back an
        // empty block so the caller's progress guarantee unwinds safely.
        if !self.enter_block_frame(open_brace) {
            return CompoundStmt {
                items: Vec::new(),
                local_labels: Vec::new(),
                is_synthetic_init_scope: false,
            };
        }
        self.expect(&TokenKind::LBrace);
        let mut items = Vec::with_capacity(16);
        let mut local_labels = Vec::with_capacity(8);

        // Save typedef shadowing state for this scope
        // Integer constants recorded for `asm "i"` operands follow block scope:
        // a nested block INHERITS the enclosing records (the kernel writes
        // `do { __auto_type __flags = …; do { asm…} while (0); } while (0)`, so
        // the statement always sits one block deeper than its initializer), and
        // exiting restores exactly what the enclosing scope had, so a name
        // redeclared or reused in a sibling block cannot inherit a stale value.
        // `clone` (not `take`) is what makes the inheritance work; an empty map
        // — the overwhelmingly common case — clones for free.
        let saved_local_constants = self.local_constants.clone();
        let saved_shadowed = self.shadowed_typedefs.clone();
        let enum_mark = self.enum_scope_log.len();
        self.block_depth += 1;

        // Save declaration attribute flags so that storage-class specifiers
        // (extern, static, typedef, etc.) from declarations inside this
        // compound statement do not leak into the enclosing context.
        // This is critical for statement expressions inside typeof():
        //   typeof(({ extern void f(void); 42; })) x = 10;
        // Without this, the `extern` from `f` leaks and makes `x` extern.
        let saved_attr_flags = self.attrs.save_flags();

        // Parse GNU __label__ declarations at the start of the block.
        // These must appear before any statements or declarations.
        // Syntax: __label__ ident1, ident2, ... ;
        while matches!(self.peek(), TokenKind::GnuLabel) {
            self.advance(); // consume __label__
            // Parse comma-separated list of label names
            loop {
                if let TokenKind::Identifier(name) = self.peek() {
                    local_labels.push(name.clone());
                    self.advance();
                }
                if !self.consume_if(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_after(&TokenKind::Semicolon, "after __label__ declaration");
        }

        'body: while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            // E5 termination guarantee: once the expression frame budget is
            // exhausted the guarded expression entry points return
            // placeholders WITHOUT consuming tokens. Without this check a
            // statement that fails to parse an expression would spin this
            // loop forever on the same token (hostile input like
            // `({ ({ ... ({ ) ... }) }) })` beyond the budget). The budget
            // diagnostic was already emitted; unwind silently like the
            // top-level loop does.
            if self.nesting_budget_diagnosed {
                break;
            }
            let pos_before = self.pos;
            // Single-tail iteration: every path exits through the labelled
            // block so the progress guarantee below ENFORCES consumption on
            // all of them (`break 'item` replaces what used to be
            // `continue`, which silently skipped the check).
            'item: {
                self.skip_gcc_extensions();
                // Handle #pragma pack directives within function bodies
                while self.handle_pragma_pack_token() {
                    self.consume_if(&TokenKind::Semicolon);
                }
                // Handle #pragma GCC visibility push/pop within function bodies
                while self.handle_pragma_visibility_token() {
                    self.consume_if(&TokenKind::Semicolon);
                }
                if matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
                    break 'body;
                }
                // Handle __label__ declarations that appear after __extension__
                if matches!(self.peek(), TokenKind::GnuLabel) {
                    self.advance();
                    loop {
                        if let TokenKind::Identifier(name) = self.peek() {
                            local_labels.push(name.clone());
                            self.advance();
                        }
                        if !self.consume_if(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect_after(&TokenKind::Semicolon, "after __label__ declaration");
                    break 'item;
                }
                if matches!(self.peek(), TokenKind::StaticAssert) {
                    let assertion = self.parse_static_assert();
                    items.push(BlockItem::StaticAssert(assertion));
                } else if self.is_type_specifier() && !self.is_typedef_label() {
                    // GNU C nested function definition? (`int f(int *p) { ... }`
                    // at block scope). Try to parse one; on mismatch the parser
                    // state is restored and the item falls through to a normal
                    // local declaration.
                    if let Some(nf) = self.try_parse_nested_function_def() {
                        items.push(BlockItem::NestedFunction(nf));
                        break 'item;
                    }
                    if let Some(decl) = self.parse_local_declaration() {
                        items.push(BlockItem::Declaration(decl));
                    }
                } else if self.looks_like_implicit_int_nested_function() {
                    // C89 implicit-int nested function (`retframe_block() { ... }`
                    // or K&R `f(a) int a; { ... }` at block scope): an identifier
                    // directly followed by a parameter list and a body cannot be
                    // an expression statement (that would require a ';').
                    if let Some(nf) = self.try_parse_nested_function_def() {
                        items.push(BlockItem::NestedFunction(nf));
                        break 'item;
                    }
                    let stmt = self.parse_stmt();
                    items.push(BlockItem::Statement(stmt));
                } else {
                    let stmt = self.parse_stmt();
                    items.push(BlockItem::Statement(stmt));
                }
            }
            // `_Static_assert`s from struct member lists in this item.
            items.extend(
                self.deferred_static_asserts
                    .drain(..)
                    .map(BlockItem::StaticAssert),
            );
            // Universal progress guarantee, ENFORCED: no statement parse
            // may return having consumed nothing, or this loop would retry
            // the same token forever (defense in depth on top of the
            // budget check). Runs on every path, including the labelled
            // early exits above.
            if self.pos == pos_before {
                self.advance();
            }
        }

        self.expect_closing(&TokenKind::RBrace, open_brace);
        self.shadowed_typedefs = saved_shadowed;
        self.local_constants = saved_local_constants;
        self.block_depth -= 1;
        self.restore_enum_scope(enum_mark);
        self.attrs.restore_flags(saved_attr_flags);
        self.exit_parser_frame();
        CompoundStmt {
            items,
            local_labels,
            is_synthetic_init_scope: false,
        }
    }

    /// Statement-budget wrapper: statement nesting (compound bodies, if /
    /// loop / case bodies, statement expressions) recurses through this
    /// dispatcher one frame per level. On exhaustion return an empty
    /// statement WITHOUT consuming tokens so every caller's progress
    /// guarantee unwinds safely.
    /// Drop the parsed-constant record of every identifier assigned by `expr`.
    ///
    /// `int v = 5; v = 6; asm("" :: "i"(v));` must be rejected: the record is
    /// only a licence for a *constant* operand, so any store to the name
    /// withdraws it.  Compound assignment, `++`/`--`, and `&v` follow the same
    /// rule.  Being conservative here costs at most a missed acceptance; the
    /// alternative — keeping a stale value — would licence wrong code.
    pub(super) fn invalidate_assigned_constants(&mut self, expr: &Expr) {
        if self.local_constants.is_empty() {
            return;
        }
        let mut stack: Vec<&Expr> = vec![expr];
        while let Some(e) = stack.pop() {
            match e {
                Expr::Assign(lhs, rhs, _) | Expr::CompoundAssign(_, lhs, rhs, _) => {
                    Self::forget_lvalue(lhs, &mut self.local_constants);
                    stack.push(lhs);
                    stack.push(rhs);
                }
                Expr::PostfixOp(_, target, _) | Expr::UnaryOp(_, target, _) => {
                    Self::forget_lvalue(target, &mut self.local_constants);
                    stack.push(target);
                }
                Expr::AddressOf(target, _) => {
                    Self::forget_lvalue(target, &mut self.local_constants);
                    stack.push(target);
                }
                _ => e.for_each_child(&mut |c| stack.push(c)),
            }
        }
    }

    /// Remove the name an lvalue expression designates, if any.
    fn forget_lvalue(lhs: &Expr, constants: &mut crate::common::fx_hash::FxHashMap<String, i64>) {
        match lhs {
            Expr::Identifier(name, _) => {
                constants.remove(name.as_str());
            }
            Expr::Cast(_, inner, _, _) | Expr::BitCast(_, inner, _) => {
                Self::forget_lvalue(inner, constants)
            }
            _ => {}
        }
    }

    pub(super) fn parse_stmt(&mut self) -> Stmt {
        let stmt = self.parse_stmt_guarded();
        // Any store in this statement withdraws the parsed-constant record of
        // the names it writes (see `invalidate_assigned_constants`).
        if let Stmt::Expr(Some(expr)) = &stmt {
            self.invalidate_assigned_constants(expr);
        }
        stmt
    }

    /// Frame-guarded entry point; the real dispatcher is `parse_stmt_inner`.
    fn parse_stmt_guarded(&mut self) -> Stmt {
        if !self.enter_stmt_frame(self.peek_span()) {
            return Stmt::Expr(None);
        }
        let stmt = self.parse_stmt_inner();
        self.exit_parser_frame();
        stmt
    }

    fn parse_stmt_inner(&mut self) -> Stmt {
        // C23 / GNU extension: declarations are allowed in statement position.
        // This handles declarations after labels (e.g., `label: int x = 5;`),
        // after case/default, and other contexts where parse_stmt() is called.
        // We skip __extension__ first since it can precede declarations.
        self.skip_gcc_extensions();
        // C23 standard attributes in statement position: `[[fallthrough]];`,
        // `[[maybe_unused]] int x;`, `[[gnu::...]]` etc. are hints; consume the
        // whole `[[ ... ]]` list so the guarded statement parses normally
        // (glibc's cpu-features.c uses `[[fallthrough]]`).
        self.skip_c23_attribute_lists();
        if self.is_type_specifier() && !self.is_typedef_label() {
            if let Some(decl) = self.parse_local_declaration() {
                return Stmt::Declaration(decl);
            }
            // If parse_local_declaration returns None (e.g. _Static_assert),
            // fall through to parse a null statement or the next thing
            return Stmt::Expr(None);
        }
        match self.peek() {
            TokenKind::Return => {
                let span = self.peek_span();
                self.advance();
                let expr = if matches!(self.peek(), TokenKind::Semicolon) {
                    None
                } else {
                    Some(self.parse_expr())
                };
                self.expect_after(&TokenKind::Semicolon, "after return statement");
                Stmt::Return(expr, span)
            }
            TokenKind::If => {
                let span = self.peek_span();
                self.advance();
                let open = self.peek_span();
                self.expect_context(&TokenKind::LParen, "after 'if'");
                let saved_shadowed = self.shadowed_typedefs.clone();
                let enum_mark = self.enum_scope_log.len();
                let (init_item, cond) = self.parse_selection_header();
                self.expect_closing(&TokenKind::RParen, open);
                let then_stmt = self.parse_stmt();
                let else_stmt = if self.consume_if(&TokenKind::Else) {
                    Some(Box::new(self.parse_stmt()))
                } else {
                    None
                };
                self.shadowed_typedefs = saved_shadowed;
                self.restore_enum_scope(enum_mark);
                let if_stmt = Stmt::If(cond, Box::new(then_stmt), else_stmt, span);
                if let Some(init) = init_item {
                    Stmt::Compound(CompoundStmt {
                        items: vec![init, BlockItem::Statement(if_stmt)],
                        local_labels: Vec::new(),
                        is_synthetic_init_scope: true,
                    })
                } else {
                    if_stmt
                }
            }
            TokenKind::While => {
                let span = self.peek_span();
                self.advance();
                let open = self.peek_span();
                self.expect_context(&TokenKind::LParen, "after 'while'");
                let cond = self.parse_expr();
                self.expect_closing(&TokenKind::RParen, open);
                let body = self.parse_stmt();
                Stmt::While(cond, Box::new(body), span)
            }
            TokenKind::Do => {
                let span = self.peek_span();
                self.advance();
                let body = self.parse_stmt();
                self.expect_after(&TokenKind::While, "at end of do-while statement");
                let open = self.peek_span();
                self.expect_context(&TokenKind::LParen, "after 'while'");
                let cond = self.parse_expr();
                self.expect_closing(&TokenKind::RParen, open);
                self.expect_after(&TokenKind::Semicolon, "after do-while statement");
                Stmt::DoWhile(Box::new(body), cond, span)
            }
            TokenKind::For => self.parse_for_stmt(),
            TokenKind::LBrace => {
                let compound = self.parse_compound_stmt();
                Stmt::Compound(compound)
            }
            TokenKind::Break => {
                let span = self.peek_span();
                self.advance();
                let label = if let TokenKind::Identifier(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    Some(name)
                } else {
                    None
                };
                self.expect_after(&TokenKind::Semicolon, "after break statement");
                Stmt::Break(label, span)
            }
            TokenKind::Continue => {
                let span = self.peek_span();
                self.advance();
                let label = if let TokenKind::Identifier(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    Some(name)
                } else {
                    None
                };
                self.expect_after(&TokenKind::Semicolon, "after continue statement");
                Stmt::Continue(label, span)
            }
            TokenKind::Switch => {
                let span = self.peek_span();
                self.advance();
                let open = self.peek_span();
                self.expect_context(&TokenKind::LParen, "after 'switch'");
                let saved_shadowed = self.shadowed_typedefs.clone();
                let enum_mark = self.enum_scope_log.len();
                let (init_item, expr) = self.parse_selection_header();
                self.expect_closing(&TokenKind::RParen, open);
                let body = self.parse_stmt();
                self.shadowed_typedefs = saved_shadowed;
                self.restore_enum_scope(enum_mark);
                let sw_stmt = Stmt::Switch(expr, Box::new(body), span);
                if let Some(init) = init_item {
                    Stmt::Compound(CompoundStmt {
                        items: vec![init, BlockItem::Statement(sw_stmt)],
                        local_labels: Vec::new(),
                        is_synthetic_init_scope: true,
                    })
                } else {
                    sw_stmt
                }
            }
            TokenKind::Case => {
                let span = self.peek_span();
                self.advance();
                let expr = self.parse_expr();
                if self.consume_if(&TokenKind::Ellipsis) {
                    // GNU case range extension: case low ... high:
                    let high = self.parse_expr();
                    self.expect_context(&TokenKind::Colon, "after 'case' expression");
                    let stmt = self.parse_label_body();
                    Stmt::CaseRange(expr, high, Box::new(stmt), span)
                } else {
                    self.expect_context(&TokenKind::Colon, "after 'case' expression");
                    let stmt = self.parse_label_body();
                    Stmt::Case(expr, Box::new(stmt), span)
                }
            }
            TokenKind::Default => {
                let span = self.peek_span();
                self.advance();
                self.expect_context(&TokenKind::Colon, "after 'default'");
                let stmt = self.parse_label_body();
                Stmt::Default(Box::new(stmt), span)
            }
            TokenKind::Goto => {
                let span = self.peek_span();
                self.advance();
                if matches!(self.peek(), TokenKind::Star) {
                    // Computed goto: goto *expr;
                    self.advance();
                    let expr = self.parse_expr();
                    self.expect_after(&TokenKind::Semicolon, "after goto statement");
                    Stmt::GotoIndirect(Box::new(expr), span)
                } else {
                    let label = if let TokenKind::Identifier(name) = self.peek() {
                        let name = name.clone();
                        self.advance();
                        name
                    } else {
                        String::new()
                    };
                    self.expect_after(&TokenKind::Semicolon, "after goto statement");
                    Stmt::Goto(label, span)
                }
            }
            TokenKind::Identifier(name) => {
                // Check for label (identifier followed by colon)
                let name_clone = name.clone();
                let span = self.peek_span();
                if self.pos + 1 < self.tokens.len()
                    && matches!(self.tokens[self.pos + 1].kind, TokenKind::Colon)
                {
                    self.advance(); // identifier
                    self.advance(); // colon
                    // Skip optional label attributes: `label: __attribute__((unused));`
                    // In GNU C, labels can have attributes (e.g., unused, hot, cold).
                    // We consume and discard them since they only affect diagnostics.
                    self.skip_label_attributes();
                    let stmt = self.parse_label_body();
                    Stmt::Label(name_clone, Box::new(stmt), span)
                } else {
                    let expr = self.parse_expr();
                    self.expect_after(&TokenKind::Semicolon, "after expression");
                    Stmt::Expr(Some(expr))
                }
            }
            TokenKind::Asm => self.parse_inline_asm(),
            TokenKind::Semicolon => {
                self.advance();
                Stmt::Expr(None)
            }
            _ => {
                let expr = self.parse_expr();
                self.expect_after(&TokenKind::Semicolon, "after expression");
                Stmt::Expr(Some(expr))
            }
        }
    }

    /// Parse the statement that a label (`case`, `default`, or `name:`)
    /// applies to.
    ///
    /// C23 §6.8.1 (and GCC 11+ in every language mode) allow a label to be the
    /// last thing in a compound statement: `switch (x) { case 0: return 1;
    /// case 2: }` or `{ ...; out: }`. Before C23 a label had to be followed by
    /// a statement, which forced the idiom `case 2: ;`. The trailing label
    /// denotes a null statement, so it is parsed as one instead of emitting
    /// "expected expression before '}'" and desynchronising the parser for
    /// the rest of the function (the stress lab's `switch` family emits this
    /// shape and GCC 12 accepts it).
    fn parse_label_body(&mut self) -> Stmt {
        // GNU/C23 6.8.1: a label may be the last item of a compound statement
        // (`case 2:` directly before `}`).  `label_at_block_end` also accepts
        // a truncated token stream (Eof) so malformed input degrades to a
        // null-statement label instead of an unhandled parse failure.
        if self.label_at_block_end() {
            return Stmt::Expr(None);
        }
        self.parse_stmt()
    }

    /// Parse `(cond)` or C2y/GCC 15+ `(init; cond)` inside `if` / `switch`.
    fn parse_selection_header(&mut self) -> (Option<BlockItem>, Expr) {
        if self.consume_if(&TokenKind::Semicolon) {
            return (
                Some(BlockItem::Statement(Stmt::Expr(None))),
                self.parse_expr(),
            );
        }
        // Optional leading C23 attribute `[[maybe_unused]] int x = f();` or type specifier.
        let starts_decl = self.is_type_specifier() || self.is_c23_attr_start();
        if starts_decl {
            let decl = self.parse_local_declaration();
            let cond = self.parse_expr();
            let item = decl
                .map(BlockItem::Declaration)
                .unwrap_or(BlockItem::Statement(Stmt::Expr(None)));
            return (Some(item), cond);
        }
        let first = self.parse_expr();
        if self.consume_if(&TokenKind::Semicolon) {
            let cond = self.parse_expr();
            (Some(BlockItem::Statement(Stmt::Expr(Some(first)))), cond)
        } else {
            (None, first)
        }
    }

    /// Parse a for statement: for (init; cond; inc) body
    fn parse_for_stmt(&mut self) -> Stmt {
        let span = self.peek_span();
        self.advance();
        let open = self.peek_span();
        self.expect_context(&TokenKind::LParen, "after 'for'");

        let saved_shadowed = self.shadowed_typedefs.clone();
        let enum_mark = self.enum_scope_log.len();

        let init = if matches!(self.peek(), TokenKind::Semicolon) {
            self.advance();
            None
        } else if self.is_type_specifier() || self.is_c23_attr_start() {
            let decl = self.parse_local_declaration();
            decl.map(|d| Box::new(ForInit::Declaration(d)))
        } else {
            let expr = self.parse_expr();
            self.expect_after(&TokenKind::Semicolon, "in for statement initializer");
            Some(Box::new(ForInit::Expr(expr)))
        };

        let cond = if matches!(self.peek(), TokenKind::Semicolon) {
            None
        } else {
            Some(self.parse_expr())
        };
        self.expect_after(&TokenKind::Semicolon, "in for statement condition");

        let inc = if matches!(self.peek(), TokenKind::RParen) {
            None
        } else {
            Some(self.parse_expr())
        };
        self.expect_closing(&TokenKind::RParen, open);

        let body = self.parse_stmt();
        self.shadowed_typedefs = saved_shadowed;
        self.restore_enum_scope(enum_mark);
        Stmt::For(init, cond, inc, Box::new(body), span)
    }

    // === Inline assembly parsing ===

    fn parse_inline_asm(&mut self) -> Stmt {
        self.advance(); // consume 'asm' / '__asm__'
        // Skip optional qualifiers: volatile, goto, inline
        while matches!(self.peek(), TokenKind::Volatile)
            || matches!(self.peek(), TokenKind::Goto)
            || matches!(self.peek(), TokenKind::Inline)
        {
            self.advance();
        }
        let open = self.peek_span();
        self.expect_context(&TokenKind::LParen, "after 'asm'");

        let template = self.parse_asm_string();

        let mut outputs = Vec::new();
        let mut inputs = Vec::with_capacity(8);
        let mut clobbers = Vec::with_capacity(8);
        let mut goto_labels = Vec::with_capacity(8);

        // First colon: outputs
        if matches!(self.peek(), TokenKind::Colon) {
            self.advance();
            outputs = self.parse_asm_operands(false);

            // Second colon: inputs
            if matches!(self.peek(), TokenKind::Colon) {
                self.advance();
                inputs = self.parse_asm_operands(true);

                // Third colon: clobbers
                if matches!(self.peek(), TokenKind::Colon) {
                    self.advance();
                    clobbers = self.parse_asm_clobbers();

                    // Fourth colon: goto labels
                    if matches!(self.peek(), TokenKind::Colon) {
                        self.advance();
                        goto_labels = self.parse_asm_goto_labels();
                    }
                }
            }
        }

        self.expect_closing(&TokenKind::RParen, open);
        self.validate_inline_asm(&template, &outputs, &inputs, goto_labels.len(), open);
        self.consume_if(&TokenKind::Semicolon);

        Stmt::InlineAsm {
            template,
            outputs,
            inputs,
            clobbers,
            goto_labels,
        }
    }

    /// Reject the `asm` operand and template shapes GCC rejects.
    ///
    /// The back ends' `classify_constraint` has a `_ => {}` catch-all that maps
    /// an unmodelled constraint letter to a general-purpose register, so
    /// `asm("" : "=r"(r) : "z"(x))` used to compile here and emit the wrong
    /// register class. Diagnosing at parse time is the only point where the
    /// operand's *source* role (output vs input) and the operand count are
    /// still explicit. See `super::asm_validate` for the measured GCC 16.2
    /// message set.
    fn validate_inline_asm(
        &mut self,
        template: &str,
        outputs: &[AsmOperand],
        inputs: &[AsmOperand],
        label_count: usize,
        span: Span,
    ) {
        if outputs.is_empty() && inputs.is_empty() && label_count == 0 && !template.contains('%') {
            return;
        }
        let is_x86 = matches!(crate::common::types::target_elf_machine(), 3 | 62);
        // Fold the operand expressions first: the fold borrows the parser's
        // name tables immutably, and the diagnostics below need `&mut self`.
        // Block-scope integer constants participate in the same fold as
        // enumerators so `asm("" :: "i"(flags))` can be resolved later.
        let mut fold_enums = self.enum_constants.clone();
        for (name, value) in &self.local_constants {
            fold_enums.entry(name.clone()).or_insert(*value);
        }
        let ctx = asm_validate::SymbolContext {
            typedefs: &self.typedefs,
            enums: &fold_enums,
            functions: &self.function_names,
        };
        let defer_immediate_constness = self.asm_immediate_validation_deferred;
        let constness: Vec<asm_validate::Constness> = outputs
            .iter()
            .chain(inputs.iter())
            .map(|op| {
                let value = asm_validate::constness(
                    &op.expr,
                    &ctx,
                    self.asm_deferred_instantiation,
                );
                if defer_immediate_constness && value == asm_validate::Constness::NonConstant {
                    // Preserve the existing post-optimization deferral for
                    // immediate-only constraints; dynamic operands are checked
                    // once optimization has had a chance to fold them.
                    asm_validate::Constness::Unknown
                } else {
                    value
                }
            })
            .collect();
        let out_view: Vec<asm_validate::Operand<'_>> = outputs
            .iter()
            .enumerate()
            .map(|(i, op)| asm_validate::Operand {
                constraint: op.constraint.as_str(),
                constant: constness[i],
                name: op.name.as_deref(),
            })
            .collect();
        let in_view: Vec<asm_validate::Operand<'_>> = inputs
            .iter()
            .enumerate()
            .map(|(i, op)| asm_validate::Operand {
                constraint: op.constraint.as_str(),
                constant: constness[outputs.len() + i],
                name: op.name.as_deref(),
            })
            .collect();
        for problem in asm_validate::validate(template, &out_view, &in_view, label_count, is_x86) {
            let at = problem
                .operand
                .and_then(|i| outputs.iter().chain(inputs.iter()).nth(i))
                .map_or(span, |op| op.expr.span());
            if problem.is_error {
                self.emit_error(problem.message, at);
            } else {
                self.diagnostics.warning(problem.message, at);
            }
        }
    }

    fn parse_asm_string(&mut self) -> String {
        // Narrow string tokens deliberately carry one Rust char per C source
        // byte so ordinary `char[]` initializers preserve `\\xNN` exactly.
        // An asm template is textual output instead: decode a valid UTF-8
        // carrier only *after* concatenating adjacent literals, because a
        // multibyte character may straddle two C string-literal fragments.
        let mut byte_carrier = String::with_capacity(64);
        while let TokenKind::StringLiteral(s) = self.peek() {
            byte_carrier.push_str(s);
            self.advance();
        }
        narrow_string_byte_carrier_to_utf8(&byte_carrier).unwrap_or(byte_carrier)
    }

    fn parse_asm_operands(&mut self, is_input: bool) -> Vec<AsmOperand> {
        let mut operands = Vec::with_capacity(8);
        if matches!(self.peek(), TokenKind::Colon | TokenKind::RParen) {
            return operands;
        }
        loop {
            let operand = self.parse_one_asm_operand(is_input);
            operands.push(operand);
            if !self.consume_if(&TokenKind::Comma) {
                break;
            }
        }
        operands
    }

    fn parse_one_asm_operand(&mut self, is_input: bool) -> AsmOperand {
        // Optional [name]
        let name = if matches!(self.peek(), TokenKind::LBracket) {
            let open = self.peek_span();
            self.advance();
            let n = if let TokenKind::Identifier(id) = self.peek() {
                let id = id.clone();
                self.advance();
                Some(id)
            } else {
                None
            };
            self.expect_closing(&TokenKind::RBracket, open);
            n
        } else {
            None
        };

        // Constraint string (may be concatenated)
        let constraint = if let TokenKind::StringLiteral(s) = self.peek() {
            let span = self.peek_span();
            let mut full = s.clone();
            self.advance();
            while let TokenKind::StringLiteral(s2) = self.peek() {
                full.push_str(s2);
                self.advance();
            }
            // GCC 16.2's constraint-direction contract (measured exact
            // wordings): inputs may not carry the output-only `=`/`+`
            // modifiers, and outputs must ask for a written location with
            // one of them. Checked here so a bad statement never reaches
            // lowering, exactly like GCC's parse-time rejection.
            if is_input {
                if full.contains('=') {
                    self.emit_error("input operand constraint contains '='", span);
                } else if full.contains('+') {
                    self.emit_error("input operand constraint contains '+'", span);
                }
            } else if !full.is_empty() {
                if !full.contains('=') && !full.contains('+') {
                    self.emit_error("output operand constraint lacks '='", span);
                } else if let Some(comma) = full.find(',') {
                    // Alternatives after the first may not re-introduce the
                    // write modifiers (measured on gcc-16.2: `=r,=r` ->
                    // `operand constraint contains incorrectly positioned
                    // '+' or '='`).
                    let rest = &full[comma + 1..];
                    if rest.contains('=') || rest.contains('+') {
                        self.emit_error(
                            "operand constraint contains incorrectly positioned '+' or '='",
                            span,
                        );
                    }
                }
            }
            full
        } else {
            String::new()
        };

        // (expr)
        let open = self.peek_span();
        self.expect_context(&TokenKind::LParen, "in asm operand");
        let expr = self.parse_expr();
        self.expect_closing(&TokenKind::RParen, open);

        AsmOperand {
            name,
            constraint,
            expr,
        }
    }

    fn parse_asm_clobbers(&mut self) -> Vec<String> {
        let mut clobbers = Vec::with_capacity(8);
        if matches!(self.peek(), TokenKind::Colon | TokenKind::RParen) {
            return clobbers;
        }
        while let TokenKind::StringLiteral(s) = self.peek() {
            let mut full = s.clone();
            self.advance();
            while let TokenKind::StringLiteral(s2) = self.peek() {
                full.push_str(s2);
                self.advance();
            }
            clobbers.push(full);
            if !self.consume_if(&TokenKind::Comma) {
                break;
            }
        }
        clobbers
    }

    /// Parse the goto labels section (fourth colon) of an asm goto statement.
    /// Labels are comma-separated identifiers: `asm goto("..." : : : : label1, label2)`
    fn parse_asm_goto_labels(&mut self) -> Vec<String> {
        let mut labels = Vec::with_capacity(4);
        if matches!(self.peek(), TokenKind::RParen) {
            return labels;
        }
        while let TokenKind::Identifier(name) = self.peek() {
            labels.push(name.clone());
            self.advance();
            if !self.consume_if(&TokenKind::Comma) {
                break;
            }
        }
        labels
    }
}
