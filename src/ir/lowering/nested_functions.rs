//! GNU C nested function lowering (GCC extension).
//!
//! A nested function is lowered to a separate IR function under a mangled
//! name (`parent.name`, further nested: `parent.name.child`). Enclosing
//! locals referenced from the nested subtree are captured through a **frame
//! struct** allocated in the owning function:
//!
//! ```text
//! frame layout (16-byte aligned):
//!   [+0]  link      — parent frame pointer (only if this fn is nested)
//!   [+8]  saved rbp — non-local goto save area (only when targeted)
//!   [+16] saved rsp — non-local goto save area (only when targeted)
//!   [..]  captured locals, each 16-byte aligned
//! ```
//!
//! The capture mechanism is retroactive: when lowering reaches a nested
//! definition, captured locals declared earlier are MOVED into the frame —
//! their alloca is deleted, a dominating `GEP(frame, off)` is spliced into
//! the entry block, and every use of the old alloca (in blocks lowered so
//! far, including the parameter pre-stores) is rewritten. This keeps the
//! parent's earlier accesses correct without a full pre-scan of the body.
//!
//! The nested function receives the enclosing frame pointer through the
//! static chain register (%r10 on x86-64): `GetStaticChain` at entry, then
//! `GEP`/`Load` chains per captured variable. Direct calls emit
//! `SetStaticChain` before the `Call`; taking the address of a chain-using
//! nested function builds a stack trampoline (`InitTrampoline`).

use super::definitions::LocalInfo;
use super::func_state::{FrameState, NestedFnEntry, NonlocalGotoTarget};
use super::lower::Lowerer;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::frontend::parser::ast::{
    BlockItem, CompoundStmt, Expr, ForInit, FunctionDef, Initializer, SizeofArg, Stmt,
    TypeSpecifier,
};
use crate::ir::reexports::{Instruction, IrConst, Operand, Terminator, Value};

/// Result of the capture analysis over a nested-function subtree.
#[derive(Default, Debug)]
pub(super) struct NestedCapture {
    /// Names of visible enclosing locals referenced from the subtree
    /// (excluding shadowing by the subtree's own declarations).
    pub vars: FxHashSet<String>,
    /// Label names referenced via `goto` from the subtree that do NOT
    /// resolve to a label defined inside the subtree (non-local gotos).
    pub goto_labels: FxHashSet<String>,
    /// Label names referenced via `&&label` from the subtree that do not
    /// resolve to a subtree-local label.
    pub addr_labels: FxHashSet<String>,
    /// Whether any nested function is defined inside the subtree.
    pub has_nested: bool,
}

impl Lowerer {
    // ─────────────────────────────────────────────────────────────────────
    // Capture analysis: an AST walk that emulates lexical scoping well
    // enough to decide, for each identifier reference in the nested
    // subtree, whether it names an enclosing local.
    // ─────────────────────────────────────────────────────────────────────

    /// Walk a nested-function body collecting references to enclosing
    /// locals and non-local labels. `visible` holds the names of enclosing
    /// locals visible at the definition site (params + locals declared so
    /// far in every enclosing scope), `enclosing_labels` the label names
    /// and `__label__`-declared names of enclosing functions/blocks.
    pub(super) fn analyze_nested_captures(
        &self,
        nf: &FunctionDef,
        visible: &FxHashSet<String>,
        enclosing_labels: &FxHashSet<String>,
    ) -> NestedCapture {
        let mut result = NestedCapture::default();
        // Scope stack of names declared INSIDE the nested subtree. The
        // function's own parameters shadow enclosing locals from the start.
        let mut scopes: Vec<FxHashSet<String>> = vec![FxHashSet::default()];
        for p in &nf.params {
            for expr in &p.vla_size_exprs {
                self.capture_expr(expr, visible, &mut scopes, &mut result);
            }
            let ts = self.resolve_type_spec(&p.type_spec);
            if let TypeSpecifier::Pointer(inner, _) = ts {
                let dim_infos = self.collect_vla_dims(inner);
                for d in dim_infos {
                    if let Some(ref expr) = d.dim_expr {
                        self.capture_expr(expr, visible, &mut scopes, &mut result);
                    }
                }
            }
        }
        if let Some(top) = scopes.last_mut() {
            for p in &nf.params {
                if let Some(pn) = &p.name {
                    top.insert(pn.clone());
                }
            }
        }
        self.capture_compound(
            &nf.body,
            visible,
            enclosing_labels,
            &mut scopes,
            &mut result,
        );
        result
    }

    fn capture_declare(scopes: &mut [FxHashSet<String>], name: &str) {
        if let Some(top) = scopes.last_mut() {
            top.insert(name.to_string());
        }
    }

    fn capture_name_visible_in_subtree(scopes: &[FxHashSet<String>], name: &str) -> bool {
        scopes.iter().any(|s| s.contains(name))
    }

    fn capture_compound(
        &self,
        compound: &CompoundStmt,
        visible: &FxHashSet<String>,
        enclosing_labels: &FxHashSet<String>,
        scopes: &mut Vec<FxHashSet<String>>,
        result: &mut NestedCapture,
    ) {
        // Mirror lower_compound_stmt's scope rule: a scope is pushed when
        // the block contains declarations or nested definitions.
        let has_bindings = compound.items.iter().any(|i| {
            matches!(i, BlockItem::Declaration(_)) || matches!(i, BlockItem::NestedFunction(_))
        });
        if has_bindings {
            scopes.push(FxHashSet::default());
        }
        // Labels defined (or __label__-declared) in this block shadow
        // enclosing labels for references INSIDE this block.
        let mut local_labels: FxHashSet<String> = FxHashSet::default();
        for lbl in &compound.local_labels {
            local_labels.insert(lbl.clone());
        }
        for item in &compound.items {
            match item {
                BlockItem::Declaration(decl) => {
                    for d in &decl.declarators {
                        if !d.name.is_empty() {
                            Self::capture_declare(scopes, &d.name);
                        }
                        // Initializers are expressions in the nested subtree
                        // too. Omitting them missed the most common capture
                        // shape: `T local = helper(enclosing_a, enclosing_b)`.
                        if let Some(init) = &d.init {
                            self.capture_initializer(init, visible, scopes, result);
                        }
                    }
                }
                BlockItem::Statement(stmt) => {
                    self.capture_stmt(
                        stmt,
                        visible,
                        enclosing_labels,
                        scopes,
                        &mut local_labels,
                        result,
                    );
                }
                BlockItem::NestedFunction(nf) => {
                    result.has_nested = true;
                    // Recurse: the nested function's own subtree, with its
                    // params pre-seeded as its innermost scope. References
                    // to OUR visible set from inside count as captures of
                    // the same enclosing locals (they resolve to the same
                    // storage), so the walk uses the same `visible`.
                    scopes.push(FxHashSet::default());
                    for p in &nf.params {
                        if let Some(pn) = &p.name {
                            Self::capture_declare(scopes, pn);
                        }
                    }
                    self.capture_compound(&nf.body, visible, enclosing_labels, scopes, result);
                    scopes.pop();
                }
            }
        }
        if has_bindings {
            scopes.pop();
        }
    }

    fn capture_stmt(
        &self,
        stmt: &Stmt,
        visible: &FxHashSet<String>,
        enclosing_labels: &FxHashSet<String>,
        scopes: &mut Vec<FxHashSet<String>>,
        local_labels: &mut FxHashSet<String>,
        result: &mut NestedCapture,
    ) {
        match stmt {
            Stmt::Expr(Some(e)) => {
                self.capture_expr(e, visible, scopes, result);
            }
            Stmt::Expr(None) | Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Return(Some(e), _) => self.capture_expr(e, visible, scopes, result),
            Stmt::Return(None, _) => {}
            Stmt::If(c, t, e, _) => {
                self.capture_expr(c, visible, scopes, result);
                self.capture_stmt(t, visible, enclosing_labels, scopes, local_labels, result);
                if let Some(e) = e {
                    self.capture_stmt(e, visible, enclosing_labels, scopes, local_labels, result);
                }
            }
            Stmt::While(c, b, _) | Stmt::DoWhile(b, c, _) => {
                self.capture_expr(c, visible, scopes, result);
                self.capture_stmt(b, visible, enclosing_labels, scopes, local_labels, result);
            }
            Stmt::For(init, c, inc, b, _) => {
                let mut pushed = false;
                if let Some(init) = init {
                    match init.as_ref() {
                        ForInit::Declaration(decl) => {
                            scopes.push(FxHashSet::default());
                            pushed = true;
                            for d in &decl.declarators {
                                if !d.name.is_empty() {
                                    Self::capture_declare(scopes, &d.name);
                                }
                            }
                        }
                        ForInit::Expr(e) => self.capture_expr(e, visible, scopes, result),
                    }
                }
                if let Some(c) = c {
                    self.capture_expr(c, visible, scopes, result);
                }
                if let Some(inc) = inc {
                    self.capture_expr(inc, visible, scopes, result);
                }
                self.capture_stmt(b, visible, enclosing_labels, scopes, local_labels, result);
                if pushed {
                    scopes.pop();
                }
            }
            Stmt::Compound(compound) => {
                self.capture_compound(compound, visible, enclosing_labels, scopes, result);
            }
            Stmt::Switch(c, b, _) => {
                self.capture_expr(c, visible, scopes, result);
                self.capture_stmt(b, visible, enclosing_labels, scopes, local_labels, result);
            }
            Stmt::Case(c, b, _) => {
                self.capture_expr(c, visible, scopes, result);
                self.capture_stmt(b, visible, enclosing_labels, scopes, local_labels, result);
            }
            Stmt::CaseRange(l, h, b, _) => {
                self.capture_expr(l, visible, scopes, result);
                self.capture_expr(h, visible, scopes, result);
                self.capture_stmt(b, visible, enclosing_labels, scopes, local_labels, result);
            }
            Stmt::Default(b, _) => {
                self.capture_stmt(b, visible, enclosing_labels, scopes, local_labels, result);
            }
            Stmt::Goto(label, _) => {
                // Non-local when the label is neither subtree-local nor
                // declared in this block, but IS an enclosing label.
                if !Self::capture_name_visible_in_subtree(scopes, label)
                    && !local_labels.contains(label)
                    && enclosing_labels.contains(label)
                {
                    result.goto_labels.insert(label.clone());
                }
            }
            Stmt::GotoIndirect(e, _) => {
                self.capture_expr(e, visible, scopes, result);
            }
            Stmt::Label(name, inner, _) => {
                local_labels.insert(name.clone());
                self.capture_stmt(
                    inner,
                    visible,
                    enclosing_labels,
                    scopes,
                    local_labels,
                    result,
                );
            }
            Stmt::Declaration(decl) => {
                for d in &decl.declarators {
                    if !d.name.is_empty() {
                        Self::capture_declare(scopes, &d.name);
                    }
                    if let Some(init) = &d.init {
                        self.capture_initializer(init, visible, scopes, result);
                    }
                }
            }
            Stmt::InlineAsm {
                outputs,
                inputs,
                goto_labels,
                ..
            } => {
                // Output operands take the variable's address: an output
                // naming an enclosing local captures it.
                for op in outputs {
                    if let Expr::Identifier(name, _) = &op.expr {
                        if visible.contains(name)
                            && !Self::capture_name_visible_in_subtree(scopes, name)
                        {
                            result.vars.insert(name.clone());
                        }
                    } else {
                        self.capture_expr(&op.expr, visible, scopes, result);
                    }
                }
                for op in inputs {
                    if let Expr::Identifier(name, _) = &op.expr {
                        if visible.contains(name)
                            && !Self::capture_name_visible_in_subtree(scopes, name)
                        {
                            result.vars.insert(name.clone());
                        }
                    } else {
                        self.capture_expr(&op.expr, visible, scopes, result);
                    }
                }
                for lbl in goto_labels {
                    if !Self::capture_name_visible_in_subtree(scopes, lbl)
                        && !local_labels.contains(lbl)
                        && enclosing_labels.contains(lbl)
                    {
                        result.goto_labels.insert(lbl.clone());
                    }
                }
            }
        }
    }

    fn capture_expr(
        &self,
        expr: &Expr,
        visible: &FxHashSet<String>,
        scopes: &mut Vec<FxHashSet<String>>,
        result: &mut NestedCapture,
    ) {
        match expr {
            Expr::Identifier(name, _) => {
                if visible.contains(name) && !Self::capture_name_visible_in_subtree(scopes, name) {
                    result.vars.insert(name.clone());
                }
            }
            Expr::LabelAddr(name, _) => {
                // Handled by the caller's label bookkeeping: &&label of an
                // enclosing function's label is a cross-function constant.
                // Subtree-local labels are recorded in `local_labels` by
                // capture_stmt; the caller decides with its own view. Here
                // we record the reference; the caller filters.
                result.addr_labels.insert(name.clone());
            }
            Expr::BinaryOp(_, a, b, _)
            | Expr::ArraySubscript(a, b, _)
            | Expr::Assign(a, b, _)
            | Expr::CompoundAssign(_, a, b, _)
            | Expr::Comma(a, b, _) => {
                self.capture_expr(a, visible, scopes, result);
                self.capture_expr(b, visible, scopes, result);
            }
            Expr::UnaryOp(_, a, _)
            | Expr::PostfixOp(_, a, _)
            | Expr::AddressOf(a, _)
            | Expr::Deref(a, _)
            | Expr::Cast(_, a, _)
            | Expr::GnuAlignofExpr(a, _)
            | Expr::AlignofExpr(a, _) => {
                self.capture_expr(a, visible, scopes, result);
            }
            Expr::Sizeof(inner, _) => {
                if let SizeofArg::Expr(a) = inner.as_ref() {
                    self.capture_expr(a, visible, scopes, result);
                }
            }
            Expr::Conditional(c, t, f, _) => {
                self.capture_expr(c, visible, scopes, result);
                self.capture_expr(t, visible, scopes, result);
                self.capture_expr(f, visible, scopes, result);
            }
            Expr::GnuConditional(c, e, _) => {
                self.capture_expr(c, visible, scopes, result);
                self.capture_expr(e, visible, scopes, result);
            }
            Expr::FunctionCall(f, args, _) => {
                self.capture_expr(f, visible, scopes, result);
                for a in args {
                    self.capture_expr(a, visible, scopes, result);
                }
            }
            Expr::MemberAccess(a, _, _) | Expr::PointerMemberAccess(a, _, _) => {
                self.capture_expr(a, visible, scopes, result);
            }
            Expr::CompoundLiteral(_, init, _) => {
                self.capture_initializer(init, visible, scopes, result);
            }
            Expr::StmtExpr(compound, _) => {
                self.capture_compound(compound, visible, &FxHashSet::default(), scopes, result);
            }
            Expr::VaArg(a, _, _) | Expr::ConvertVector(a, _, _) => {
                self.capture_expr(a, visible, scopes, result);
            }
            Expr::GenericSelection(c, assoc, _) => {
                self.capture_expr(c, visible, scopes, result);
                for a in assoc {
                    self.capture_expr(&a.expr, visible, scopes, result);
                }
            }
            // Literals, type-only constructs.
            Expr::IntLiteral(..)
            | Expr::UIntLiteral(..)
            | Expr::LongLiteral(..)
            | Expr::ULongLiteral(..)
            | Expr::LongLongLiteral(..)
            | Expr::ULongLongLiteral(..)
            | Expr::FloatLiteral(..)
            | Expr::FloatLiteralF32(..)
            | Expr::FloatLiteralLongDouble(..)
            | Expr::FloatLiteralF128(..)
            | Expr::FloatLiteralDecimal(..)
            | Expr::ImaginaryLiteral(..)
            | Expr::ImaginaryLiteralF32(..)
            | Expr::ImaginaryLiteralLongDouble(..)
            | Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..)
            | Expr::CharLiteral(..)
            | Expr::Alignof(_, _)
            | Expr::GnuAlignof(_, _)
            | Expr::BuiltinTypesCompatibleP(_, _, _, _, _) => {}
        }
    }

    fn capture_initializer(
        &self,
        init: &Initializer,
        visible: &FxHashSet<String>,
        scopes: &mut Vec<FxHashSet<String>>,
        result: &mut NestedCapture,
    ) {
        match init {
            Initializer::Expr(e) => self.capture_expr(e, visible, scopes, result),
            Initializer::List(items) => {
                for item in items {
                    self.capture_initializer(&item.init, visible, scopes, result);
                }
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Retroactive framing + nested function lowering.
    // ─────────────────────────────────────────────────────────────────────

    /// Ensure the current function has a frame struct, creating the alloca
    /// (deferred to the entry block) and reserving the header words if
    /// absent. Returns the frame state (cloned snapshot).
    fn ensure_frame(&mut self, need_link: bool) -> FrameState {
        if self.func().frame.is_none() {
            let dest = self.fresh_value();
            // Size placeholder: patched at finalize. Alignment 16 keeps every
            // member naturally aligned regardless of type.
            self.func_mut().entry_allocas.push(Instruction::Alloca {
                dest,
                ty: IrType::Ptr,
                size: 16,
                align: 16,
                volatile: false,
                semantic_volatile: false,
            });
            let mut cursor = 0i64;
            let mut has_link = false;
            if need_link {
                // Link word at offset 0.
                has_link = true;
                cursor = 8;
            }
            self.func_mut().frame = Some(FrameState {
                alloca: dest,
                size: 16,
                align: 16,
                has_link,
                goto_save: None,
                cursor,
            });
        }
        self.func().frame.clone().unwrap()
    }

    /// Add a member to the frame; returns its byte offset.
    fn frame_add_member(&mut self, size: usize) -> i64 {
        // 16-byte align every member for simplicity and safety.
        let mut frame = self.ensure_frame(false).clone();
        let off = (frame.cursor + 15) & !15;
        frame.cursor = off + size.max(1) as i64;
        frame.size = (frame.cursor as usize + 15) & !15;
        let size_field = frame.size;
        let alloca_id = frame.alloca.0;
        self.func_mut().frame = Some(frame);
        // Patch the alloca's size.
        for inst in self.func_mut().entry_allocas.iter_mut() {
            if let Instruction::Alloca { dest, size, .. } = inst {
                if dest.0 == alloca_id {
                    *size = size_field;
                }
            }
        }
        off
    }

    /// Reserve the non-local goto save area in the frame; returns
    /// (rbp_off, rsp_off).
    fn frame_reserve_goto_save(&mut self) -> (i64, i64) {
        let mut frame = self.ensure_frame(false).clone();
        if let Some((a, b)) = frame.goto_save {
            return (a, b);
        }
        let mut off = frame.cursor;
        if frame.has_link && off < 8 {
            off = 8;
        }
        off = (off + 7) & !7; // words are 8-aligned
        let rbp_off = off;
        let rsp_off = off + 8;
        // Callee-saved register save area (SysV x86-64): rbx, r12..r15
        // follow the two pointer words. The non-local goto restores the
        // FULL register state of the target frame's entry — without this,
        // code after the landing label reads callee-saved values left by
        // the (abandoned) nested call chain (920501-7: `a` in %rbx).
        frame.cursor = off + 8 * 7;
        frame.size = (frame.cursor as usize + 15) & !15;
        let save = Some((rbp_off, rsp_off));
        frame.goto_save = save;
        let size_field = frame.size;
        let alloca_id = frame.alloca.0;
        self.func_mut().frame = Some(frame);
        for inst in self.func_mut().entry_allocas.iter_mut() {
            if let Instruction::Alloca { dest, size, .. } = inst {
                if dest.0 == alloca_id {
                    *size = size_field;
                }
            }
        }
        (rbp_off, rsp_off)
    }

    /// Rewrite every use of `old` (a Value id) to `new` across all blocks
    /// emitted so far in the current function (instructions and
    /// terminators).
    fn rewrite_value_uses(&mut self, old: u32, new: Value) {
        // The current block being built buffers its instructions in `instrs`
        // (they move into `blocks` at the next terminator): rewrite BOTH.
        for inst in self.func_mut().instrs.iter_mut() {
            inst.for_each_value_use_mut(|v| {
                if v.0 == old {
                    *v = new;
                }
            });
            inst.for_each_operand_mut(|op| {
                if let Operand::Value(v) = op {
                    if v.0 == old {
                        *op = Operand::Value(new);
                    }
                }
            });
        }
        for block in self.func_mut().blocks.iter_mut() {
            for inst in block.instructions.iter_mut() {
                inst.for_each_value_use_mut(|v| {
                    if v.0 == old {
                        *v = new;
                    }
                });
                inst.for_each_operand_mut(|op| {
                    if let Operand::Value(v) = op {
                        if v.0 == old {
                            *op = Operand::Value(new);
                        }
                    }
                });
            }
            block.terminator.for_each_operand_mut(|op| {
                if let Operand::Value(v) = op {
                    if v.0 == old {
                        *op = Operand::Value(new);
                    }
                }
            });
        }
    }

    /// Remove the alloca instruction for `id` from the deferred entry list
    /// (the storage moved into the frame).
    fn remove_entry_alloca(&mut self, id: u32) {
        self.func_mut()
            .entry_allocas
            .retain(|inst| !matches!(inst, Instruction::Alloca { dest, .. } if dest.0 == id));
        // Also from the entry block in case finalize already ran (it has
        // not — we are mid-body — but defensive).
        if let Some(block) = self.func_mut().blocks.first_mut() {
            let mut keep = Vec::with_capacity(block.instructions.len());
            let mut spans = std::mem::take(&mut block.source_spans);
            let mut span_iter = spans.drain(..);
            for inst in block.instructions.drain(..) {
                let is_target = matches!(&inst, Instruction::Alloca { dest, .. } if dest.0 == id);
                if !is_target {
                    keep.push(inst);
                } else {
                    span_iter.next();
                }
            }
            let mut rest: Vec<_> = span_iter.collect();
            keep.extend(std::iter::empty());
            block.instructions = keep;
            let mut all = Vec::new();
            all.extend(rest.drain(..));
            block.source_spans = all;
        }
    }

    /// Emit a dominating GEP for a frame member (into the deferred entry
    /// list, which finalize splices into the entry block).
    fn frame_member_gep(&mut self, frame_alloca: Value, off: i64) -> Value {
        let dest = self.fresh_value();
        self.func_mut()
            .entry_allocas
            .push(Instruction::GetElementPtr {
                dest,
                base: frame_alloca,
                offset: Operand::Const(IrConst::ptr_int(off)),
                ty: IrType::Ptr,
            });
        dest
    }

    /// Collect the visible-locals name set of the current function (params
    /// + all currently-declared locals).
    fn visible_local_names(&self) -> FxHashSet<String> {
        self.func().locals.keys().cloned().collect()
    }

    /// Collect the visible label names of the current function: all user
    /// labels created so far (forward gotos pre-create them) and all
    /// `__label__` declarations in scope.
    fn visible_label_names(&self) -> FxHashSet<String> {
        let mut set: FxHashSet<String> = FxHashSet::default();
        for key in self.func().user_labels.keys() {
            // Keys are "funcname::resolvedlabel" — strip the prefix.
            if let Some(pos) = key.rfind("::") {
                set.insert(key[pos + 2..].to_string());
            }
        }
        for scope in self.local_label_scopes.iter() {
            for name in scope.keys() {
                set.insert(name.clone());
            }
        }
        // Labels defined in the body so far (Statement::Label) are covered
        // by user_labels (created on demand), plus any block __label__s in
        // scope.
        set
    }

    /// Lower a nested function definition encountered in the current
    /// function's body.
    pub(super) fn lower_nested_function_def(&mut self, nf: &FunctionDef) {
        // 1. Capture analysis over the nested subtree.
        let visible = self.visible_local_names();
        let enclosing_labels = self.visible_label_names();
        let capture = self.analyze_nested_captures(nf, &visible, &enclosing_labels);
        if std::env::var_os("CCC_DEBUG_NESTED_CAPTURE").is_some() {
            eprintln!(
                "[NESTED_CAPTURE] parent={} child={} visible={:?} captured={:?}",
                self.func().name,
                nf.name,
                visible,
                capture.vars
            );
        }

        // Filter addr_labels: only those that are enclosing labels (not
        // subtree-local). The walker records every &&label; subtree-local
        // ones resolve to the nested function's own labels. Determine the
        // subtree-local label set: labels defined in nf.body + its
        // __label__ decls + nested descendants'.
        let subtree_labels = collect_subtree_labels(&nf.body);
        let addr_labels: FxHashSet<String> = capture
            .addr_labels
            .iter()
            .filter(|l| !subtree_labels.contains(*l))
            .cloned()
            .collect();

        // 2. Retroactive framing of captured locals in the CURRENT function.
        let parent_is_nested = self.func().nesting_depth > 0;
        let frame = self.ensure_frame(parent_is_nested);
        let frame_alloca = frame.alloca;

        // 2a. Captured variables -> frame members.
        // bool marks VLA descriptors (frame stores {base pointer, runtime size})
        // rather than fixed-size inline storage.
        let mut captured: Vec<(String, LocalInfo, i64, bool)> = Vec::new();
        let mut static_captured: Vec<(String, LocalInfo)> = Vec::new();
        let mut transitive_captured: Vec<(String, LocalInfo, usize, i64)> = Vec::new();
        for name in &capture.vars {
            let info = match self.func().locals.get(name) {
                Some(i) => i.clone(),
                None => continue, // resolved differently at lowering time
            };
            if info.static_global_name.is_some() {
                // Static locals are globals: the nested function reaches
                // them via GlobalAddr, no chain needed.
                static_captured.push((name.clone(), info));
                continue;
            }
            // Transitive capture: the variable's storage in the CURRENT
            // function is itself a chain access into an ANCESTOR frame
            // (recorded in chain_locals). The child must NOT copy it into
            // the current frame — it reaches the same ancestor storage by
            // walking one more frame link from its own chain.
            if let Some(&(parent_hops, ancestor_off)) = self.func().chain_locals.get(&info.alloca.0)
            {
                let hops = parent_hops + 1;
                // Access path in the child: chain -> `hops` link loads ->
                // GEP(ancestor_off). Built lazily at 3e; record the path
                // facts now.
                transitive_captured.push((name.clone(), info, hops, ancestor_off));
                continue;
            }
            if let Some(vla_size) = info.vla_size {
                // A VLA cannot be moved into the fixed-size static-chain frame.
                // Capture a descriptor instead: its dynamic base pointer and
                // runtime byte size. The child loads both, so indexing and
                // sizeof(VLA) retain the parent's live allocation.
                let off = if let Some(&existing) = self.func().frame_members.get(name) {
                    existing
                } else {
                    let off = self.frame_add_member(16);
                    self.func_mut().frame_members.insert(name.clone(), off);
                    off
                };
                let base_slot = self.frame_member_gep(frame_alloca, off);
                self.emit(Instruction::Store {
                    val: Operand::Value(info.alloca),
                    ptr: base_slot,
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                });
                let size_slot = self.frame_member_gep(frame_alloca, off + 8);
                self.emit(Instruction::Store {
                    val: Operand::Value(vla_size),
                    ptr: size_slot,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                });
                captured.push((name.clone(), info, off, true));
                continue;
            }
            // MEMOIZED FRAMING: if a previous nested function already
            // moved this local into the frame, reuse the identical offset —
            // the earlier function's body has that offset baked in, and a
            // second member would never be written (siblings test bug).
            let off = if let Some(&existing) = self.func().frame_members.get(name) {
                existing
            } else {
                let size = info.alloc_size.max(1);
                let off = self.frame_add_member(size);
                self.func_mut().frame_members.insert(name.clone(), off);
                // New dominating GEP.
                let gep = self.frame_member_gep(frame_alloca, off);
                // Rewrite all existing uses of the old alloca.
                let old_alloca = info.alloca;
                self.rewrite_value_uses(old_alloca.0, gep);
                self.remove_entry_alloca(old_alloca.0);
                // Update the local's storage.
                if let Some(l) = self.func_mut().locals.get_mut(name) {
                    l.alloca = gep;
                }
                // Also update scope-frame cleanup vars referencing the alloca.
                for f in self.func_mut().scope_stack.iter_mut() {
                    for (_, v) in f.cleanup_vars.iter_mut() {
                        if v.0 == old_alloca.0 {
                            *v = gep;
                        }
                    }
                }
                off
            };
            captured.push((name.clone(), info, off, false));
        }

        // 2b. Non-local gotos: create labels in the parent, reserve the
        // save area, emit NonlocalGotoSave.
        let mut nlgoto_targets: Vec<(String, NonlocalGotoTarget)> = Vec::new();
        let mut need_save = false;
        for label in capture.goto_labels.iter().chain(addr_labels.iter()) {
            if capture.goto_labels.contains(label) {
                need_save = true;
            }
        }
        let (rbp_off, rsp_off) = if need_save {
            self.frame_reserve_goto_save()
        } else {
            (0, 0)
        };
        if need_save {
            self.func_mut()
                .entry_allocas
                .push(Instruction::NonlocalGotoSave {
                    frame: frame_alloca,
                    rbp_off,
                    rsp_off,
                });
        }
        // Named-label aliases: block IDs are renumbered per function and
        // target blocks may be merged away, so cross-function references
        // (non-local goto targets, &&label values) use stable named labels
        // EMITTED BY THE PARENT at the label statement itself (an InlineAsm
        // label instruction survives every optimization).
        let parent_name = self.func().name.clone();
        let mut alias_for: FxHashMap<String, String> = FxHashMap::default();
        for label in capture.goto_labels.iter().chain(addr_labels.iter()) {
            let resolved = self.resolve_local_label(label);
            if alias_for.contains_key(&resolved) {
                continue; // one alias per label serves both uses
            }
            let alias = format!(".LNLG.{}.{}", parent_name, resolved);
            alias_for.insert(resolved.clone(), alias.clone());
            // Queue the alias at the PARENT's label statement.
            self.func_mut()
                .pending_label_aliases
                .entry(resolved)
                .or_default()
                .push(alias.clone());
            // KEEP THE LABEL BLOCK ALIVE: the only in-function reference may
            // be none at all (the goto comes from a nested function), so
            // dead-block elimination would remove the block — and with it
            // the InlineAsm alias. global_init_label_blocks is the existing
            // keep-alive set for externally-referenced label blocks.
            let block = self.get_or_create_user_label(label);
            if !self.func().global_init_label_blocks.contains(&block) {
                self.func_mut().global_init_label_blocks.push(block);
            }
        }
        for label in &capture.goto_labels {
            let resolved = self.resolve_local_label(label);
            let alias = alias_for
                .get(&resolved)
                .cloned()
                .unwrap_or_else(|| format!(".LNLG.{}.{}", parent_name, resolved));
            nlgoto_targets.push((
                label.clone(),
                NonlocalGotoTarget {
                    up: 0, // from the nested fn: its chain IS this frame
                    rbp_off,
                    rsp_off,
                    label: alias,
                },
            ));
        }
        // &&label of a parent label: the child materializes the address as a
        // GlobalAddr of the named alias (registered above).
        let mut addr_label_aliases: FxHashMap<String, String> = FxHashMap::default();
        for label in &addr_labels {
            let resolved = self.resolve_local_label(label);
            if let Some(alias) = alias_for.get(&resolved) {
                addr_label_aliases.insert(label.clone(), alias.clone());
            }
        }

        // 3. Save the parent's build state and switch to the nested one.
        let mangled = format!("{}.{}", self.func().name, nf.name);
        let parent_state = self.func_state.take().expect("parent func_state");
        let parent_nesting_depth = parent_state.nesting_depth;
        let parent_label_scopes = self.local_label_scopes.clone();

        self.func_state = Some(super::func_state::FunctionBuildState::new(
            mangled.clone(),
            IrType::I32, // patched below via compute_function_return_type
            false,
        ));
        self.func_mut().nesting_depth = parent_nesting_depth + 1;
        // Lexical scoping: nested functions defined in enclosing scopes
        // (siblings, uncles, ...) stay callable from the child. Chain-walk
        // depths are relative to the child's nesting level.
        for (name, entry) in parent_state.nested_functions.iter() {
            self.func_mut()
                .nested_functions
                .insert(name.clone(), entry.clone());
        }

        // 3a. Mangled clone for sig-based lowering.
        let mut nf_clone = nf.clone();
        nf_clone.name = mangled.clone();

        // 3b. Inherit the parent's label scopes so __label__ names resolve
        // identically (the nested body sees the enclosing block's local
        // labels).
        for scope in parent_label_scopes.iter() {
            self.local_label_scopes.push(scope.clone());
        }

        // 3c. Chain: GetStaticChain at entry (conservative: every nested
        // function gets one; unused chains are dead code the backend
        // removes when no GEP reads them).
        let chain_val = self.fresh_value();
        self.func_mut()
            .entry_allocas
            .push(Instruction::GetStaticChain { dest: chain_val });
        self.func_mut().chain_value = Some(chain_val);
        // The nested function's OWN frame, created EAGERLY with an
        // initialized link word: any descendant may capture variables from
        // levels above this one and walk the chain THROUGH this frame.
        // (Every nested function is itself nested, so the link is always
        // required — the check is on the function being defined, not on
        // whether ITS parent is nested.)
        {
            let frame = self.ensure_frame(true);
            let link_ptr = self.fresh_value();
            self.func_mut()
                .entry_allocas
                .push(Instruction::GetElementPtr {
                    dest: link_ptr,
                    base: frame.alloca,
                    offset: Operand::Const(IrConst::ptr_int(0)),
                    ty: IrType::Ptr,
                });
            self.func_mut().entry_allocas.push(Instruction::Store {
                val: Operand::Value(chain_val),
                ptr: link_ptr,
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
                volatile: false,
            });
        }

        // 3d. Params + return type via the standard paths.
        let return_is_bool = self.is_type_bool(&nf.return_type);
        self.func_mut().return_is_bool = return_is_bool;
        let return_type = self.compute_function_return_type(&nf_clone);
        self.func_mut().return_type = return_type;
        let param_info = self.build_ir_params(&nf_clone);
        let entry_label = self.fresh_label();
        self.start_block(entry_label);
        self.func_mut().sret_ptr = None;

        // 3e. Pre-register captured enclosing locals in the nested
        // function's locals map with chain-walk address values.
        let mut statics_to_register: Vec<(String, LocalInfo)> = Vec::new();
        for (name, info, off, is_vla) in &captured {
            let info = info.clone();
            let off = *off;
            // Access path: base = chain; 0 link loads (parent is direct).
            let addr = self.frame_member_gep(chain_val, off);
            let mut new_info = info;
            if *is_vla {
                let base = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: base,
                    ptr: addr,
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                });
                let size_addr = self.frame_member_gep(chain_val, off + 8);
                let size = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: size,
                    ptr: size_addr,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                });
                new_info.alloca = base;
                new_info.vla_size = Some(size);
                // A descendant needs a descriptor-aware transitive capture;
                // do not mislabel this loaded pointer as inline frame storage.
            } else {
                new_info.alloca = addr;
                self.func_mut().chain_locals.insert(addr.0, (0, off));
            }
            new_info.static_global_name = None;
            self.func_mut().locals.insert(name.clone(), new_info);
        }
        // Transitive captures: chain -> `hops` link loads -> GEP(off).
        // Each link load reads link[0] of the frame at that level; the
        // GEPs are entry-deferred so they dominate all uses.
        for (name, info, hops, off) in &transitive_captured {
            let info = info.clone();
            let (hops, off) = (*hops, *off);
            let mut cur = chain_val;
            for _ in 0..hops {
                let link_gep = self.fresh_value();
                self.func_mut()
                    .entry_allocas
                    .push(Instruction::GetElementPtr {
                        dest: link_gep,
                        base: cur,
                        offset: Operand::Const(IrConst::ptr_int(0)),
                        ty: IrType::Ptr,
                    });
                let next = self.fresh_value();
                self.func_mut().entry_allocas.push(Instruction::Load {
                    volatile: false,
                    dest: next,
                    ptr: link_gep,
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                });
                cur = next;
            }
            let addr = self.frame_member_gep(cur, off);
            let mut new_info = info;
            new_info.alloca = addr;
            new_info.static_global_name = None;
            self.func_mut().locals.insert(name.clone(), new_info);
            self.func_mut().chain_locals.insert(addr.0, (hops, off));
        }
        for (name, info) in static_captured {
            statics_to_register.push((name, info));
        }
        // Static locals captured by reference: register them in the nested
        // function with their static global name; accesses lower through
        // GlobalAddr exactly like in the parent (statics have function-wide
        // lifetime, so no chain is needed).
        for (name, mut info) in statics_to_register {
            info.alloca = Value(u32::MAX); // never dereferenced: GlobalAddr path
            self.func_mut().locals.insert(name, info);
        }

        // Allocate function params and evaluate VLA parameter expressions
        // now that enclosing captured variables are in scope.
        self.allocate_function_params(&nf_clone, &param_info);
        self.evaluate_vla_param_side_effects(&nf_clone);
        self.handle_kr_float_promotion(&nf_clone);

        // 3f. Non-local goto targets for the nested function, keyed by the
        // scope-qualified name that lower_goto_stmt resolves through the
        // (inherited) label scopes.
        for (name, target) in &nlgoto_targets {
            let resolved = self.resolve_local_label(name);
            self.func_mut()
                .nonlocal_labels
                .insert(resolved, target.clone());
        }
        // &&label of enclosing labels: GlobalAddr of the named alias. The
        // alias map is consulted by the LabelAddr lowering hook.
        self.func_mut().addr_label_aliases = addr_label_aliases;

        // 3g. Label-depth prescan + lower the body.
        let label_depths = Self::prescan_label_depths(&nf.body);
        self.func_mut().user_label_depths = label_depths;
        self.func_mut().is_inline_candidate = false;
        // SELF-REFERENCE: a recursive nested function (`void y(a) { ... y(a-1); }`)
        // must resolve its own name BEFORE its body is lowered — the parent
        // registers it only after (step 3j), which left `y` undefined.
        self.func_mut().nested_functions.insert(
            nf.name.clone(),
            NestedFnEntry {
                mangled: mangled.clone(),
                depth: parent_nesting_depth + 1,
                uses_chain: true,
            },
        );

        self.lower_compound_stmt(&nf.body);

        // 3h. Finalize the nested function.
        let uses_sret = param_info.uses_sret;
        self.finalize_nested_function(&nf_clone, return_type, param_info.params, uses_sret);

        // 3i. Restore the parent state.
        self.func_state = Some(parent_state);
        for _ in 0..parent_label_scopes.len() {
            self.local_label_scopes.pop();
        }

        // 3j. Register the nested function for call resolution (parent scope).
        let uses_chain = true; // conservative (see 3c)
        self.func_mut().declare_nested_fn(
            &nf.name,
            NestedFnEntry {
                mangled,
                depth: parent_nesting_depth + 1,
                uses_chain,
            },
        );
        self.defined_functions.insert(nf_clone.name.clone());
    }

    /// Finalize a nested function: splice entry allocas, default-return,
    /// build the IrFunction (static, noinline, used), push to the module.
    fn finalize_nested_function(
        &mut self,
        nf: &FunctionDef,
        return_type: IrType,
        params: Vec<crate::ir::reexports::IrParam>,
        uses_sret: bool,
    ) {
        if !self.func_mut().instrs.is_empty()
            || self.func_mut().blocks.is_empty()
            || !matches!(
                self.func_mut().blocks.last().map(|b| &b.terminator),
                Some(Terminator::Return(_))
            )
        {
            let ret_op = if return_type == IrType::Void {
                None
            } else {
                Some(Operand::Const(IrConst::I32(0)))
            };
            self.terminate(Terminator::Return(ret_op));
        }

        // Splice deferred entry instructions (frame/chain/GEPs) into the
        // entry block, right after the leading alloca run.
        let entry_allocas = std::mem::take(&mut self.func_mut().entry_allocas);
        if !entry_allocas.is_empty() {
            if let Some(entry_block) = self.func_mut().blocks.first_mut() {
                let insert_pos = entry_block
                    .instructions
                    .iter()
                    .position(|inst| !matches!(inst, Instruction::Alloca { .. }))
                    .unwrap_or(entry_block.instructions.len());
                let n = entry_allocas.len();
                entry_block
                    .instructions
                    .splice(insert_pos..insert_pos, entry_allocas);
                if !entry_block.source_spans.is_empty() {
                    let dummy = vec![crate::common::source::Span::dummy(); n];
                    entry_block
                        .source_spans
                        .splice(insert_pos..insert_pos, dummy);
                }
            }
        }

        let next_val = self.func_mut().next_value;
        let param_alloca_vals = std::mem::take(&mut self.func_mut().param_alloca_values);
        let ret_eightbyte_classes = self
            .func_meta
            .sigs
            .get(&nf.name)
            .map(|s| s.ret_eightbyte_classes.clone())
            .unwrap_or_default();
        let ret_is_f128_sse = self
            .func_meta
            .sigs
            .get(&nf.name)
            .map(|s| s.ret_is_f128_sse)
            .unwrap_or(false);
        let blocks = std::mem::take(&mut self.func_mut().blocks);
        let next_label = self.next_label;
        let ir_func = crate::ir::reexports::IrFunction {
            name: nf.name.clone(),
            return_type,
            params,
            blocks,
            is_variadic: nf.variadic,
            is_declaration: false,
            is_static: true,
            is_inline: false,
            is_always_inline: false,
            is_noinline: true,
            next_value_id: next_val,
            fp_expr_tags: Default::default(),
            next_label,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: true,
            has_inlined_calls: false,
            is_fastcall: false,
            is_naked: false,
            no_instrument: false,
            param_alloca_values: param_alloca_vals,
            uses_sret,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes,
            ret_is_f128_sse,
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
        };
        self.module.functions.push(ir_func);
        // Note: the module field is accessed via self.module — verify the
        // Lowerer exposes it.
    }
}

/// Collect every label name defined or `__label__`-declared inside a
/// subtree (used to distinguish subtree-local from enclosing labels).
fn collect_subtree_labels(body: &CompoundStmt) -> FxHashSet<String> {
    let mut set: FxHashSet<String> = FxHashSet::default();
    collect_labels_compound(body, &mut set);
    set
}

fn collect_labels_compound(compound: &CompoundStmt, set: &mut FxHashSet<String>) {
    for lbl in &compound.local_labels {
        set.insert(lbl.clone());
    }
    for item in &compound.items {
        match item {
            BlockItem::Statement(stmt) => collect_labels_stmt(stmt, set),
            BlockItem::NestedFunction(nf) => collect_labels_compound(&nf.body, set),
            BlockItem::Declaration(_) => {}
        }
    }
}

fn collect_labels_stmt(stmt: &Stmt, set: &mut FxHashSet<String>) {
    match stmt {
        Stmt::Label(name, inner, _) => {
            set.insert(name.clone());
            collect_labels_stmt(inner, set);
        }
        Stmt::Compound(c) => collect_labels_compound(c, set),
        Stmt::If(_, t, e, _) => {
            collect_labels_stmt(t, set);
            if let Some(e) = e {
                collect_labels_stmt(e, set);
            }
        }
        Stmt::While(_, b, _) | Stmt::DoWhile(b, _, _) | Stmt::Switch(_, b, _) => {
            collect_labels_stmt(b, set)
        }
        Stmt::For(_, _, _, b, _) => collect_labels_stmt(b, set),
        Stmt::Case(_, b, _) | Stmt::CaseRange(_, _, b, _) | Stmt::Default(b, _) => {
            collect_labels_stmt(b, set)
        }
        Stmt::InlineAsm { goto_labels, .. } => {
            for l in goto_labels {
                set.insert(l.clone());
            }
        }
        _ => {}
    }
}

impl Lowerer {
    /// Recursively register signatures of nested functions defined in
    /// `parent_body` under `parent_name.<nested>` names.
    pub(super) fn register_nested_function_metas(
        &mut self,
        parent_name: &str,
        parent_body: &CompoundStmt,
    ) {
        for item in &parent_body.items {
            if let BlockItem::NestedFunction(nf) = item {
                let mangled = format!("{}.{}", parent_name, nf.name);
                self.register_function_meta(
                    &mangled,
                    &nf.return_type,
                    0,
                    &nf.params,
                    nf.variadic,
                    true, // internal: nested functions are static
                    nf.is_kr,
                );
                self.register_nested_function_metas(&mangled, &nf.body);
            }
        }
    }

    /// Resolve a plain identifier to a visible nested function in the
    /// current function (scope map lookup).
    pub(super) fn lookup_nested_function(&self, name: &str) -> Option<NestedFnEntry> {
        self.func().nested_functions.get(name).cloned()
    }

    /// Compute the static-chain operand for calling `callee` from the
    /// current function: the frame pointer of the callee's parent,
    /// reached via the current frame or chain-link walks.
    fn chain_operand_for_callee(&mut self, callee: &NestedFnEntry) -> Option<Value> {
        let cur_depth = self.func().nesting_depth;
        if callee.depth == cur_depth + 1 {
            // Direct child of the current function: chain = my frame.
            let frame = self.ensure_frame(false);
            Some(frame.alloca)
        } else if callee.depth <= cur_depth {
            // An ancestor's child (sibling at an outer level): walk links
            // up from my chain. My chain points at my parent's frame
            // (depth cur_depth-1); each link load goes one further up.
            let mut chain = self.func().chain_value?;
            let hops = cur_depth - callee.depth; // frames between
            for _ in 0..hops {
                let next = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: next,
                    ptr: chain,
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                });
                let gep = self.fresh_value();
                self.emit(Instruction::GetElementPtr {
                    dest: gep,
                    base: next,
                    offset: Operand::Const(IrConst::ptr_int(0)),
                    ty: IrType::Ptr,
                });
                chain = gep;
            }
            Some(chain)
        } else {
            None
        }
    }

    /// Lower a call to a visible nested function: SetStaticChain + direct
    /// Call under the mangled name. Returns true when handled.
    pub(super) fn try_lower_nested_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: crate::common::source::Span,
    ) -> Option<Operand> {
        let entry = self.lookup_nested_function(name)?;
        let chain = self.chain_operand_for_callee(&entry)?;
        // Lower the call to the mangled name FIRST (argument expressions may
        // themselves contain nested-function calls that clobber the chain
        // register), then splice SetStaticChain immediately before the Call
        // instruction. Emitting it before argument evaluation lost the chain
        // whenever an argument contained another nested call.
        let pre_len = self.func_mut().instrs.len();
        let ident = Expr::Identifier(entry.mangled.clone(), span);
        let result = self.lower_function_call(&ident, args);
        let mangled = entry.mangled.clone();
        let instrs = &mut self.func_mut().instrs;
        // Scan BACKWARD: this invocation's Call is the LAST one to the
        // mangled name in the appended range (argument expressions may have
        // emitted calls to the same nested function earlier in the range).
        let mut call_pos = None;
        for (i, inst) in instrs.iter().enumerate().skip(pre_len).rev() {
            if let Instruction::Call { func, .. } = inst {
                if func == &mangled {
                    call_pos = Some(i);
                    break;
                }
            }
        }
        if let Some(pos) = call_pos {
            instrs.insert(
                pos,
                Instruction::SetStaticChain {
                    src: Operand::Value(chain),
                },
            );
        }
        Some(result)
    }

    /// Lower the address of a nested function: a trampoline in the current
    /// frame (chain-using) or the plain function address (no chain).
    pub(super) fn lower_nested_function_address(&mut self, name: &str) -> Option<Operand> {
        let entry = self.lookup_nested_function(name)?;
        if !entry.uses_chain {
            let addr = self.fresh_value();
            self.emit(Instruction::GlobalAddr {
                dest: addr,
                name: entry.mangled.clone(),
            });
            return Some(Operand::Value(addr));
        }
        let chain = self.chain_operand_for_callee(&entry)?;
        // 24-byte executable trampoline buffer on the stack (32 for
        // alignment headroom).
        let buffer = self.emit_entry_alloca(IrType::Ptr, 32, 16, false);
        self.emit(Instruction::InitTrampoline {
            buffer,
            chain: Operand::Value(chain),
            func: entry.mangled.clone(),
        });
        Some(Operand::Value(buffer))
    }
}
