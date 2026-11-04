//! Statement-level reachability for return-path diagnostics.
//!
//! Continuations encode the destination of a transfer, not merely whether the
//! transfer ends its immediate statement. In particular, break exits a loop,
//! continue reaches its test/increment, and switch dispatch bypasses statements
//! before the first matching label. This is diagnostic analysis, not an IR
//! optimization or a replacement for expression evaluation/type checking.
use super::{BinOp, BlockItem, CompoundStmt, Expr, FxHashMap, SemanticAnalyzer, Stmt};
use crate::frontend::parser::ast::ForInit;

type NodeId = usize;
const END: NodeId = 0;
const DIVERGES: NodeId = 1;

enum Node {
    Stop,
    Next(NodeId),
    Branch(NodeId, NodeId),
    Dispatch(Vec<NodeId>),
}

struct Target<'a> {
    names: Vec<&'a str>,
    break_to: NodeId,
    continue_to: Option<NodeId>,
}

#[derive(Default)]
struct Switch {
    cases: Vec<NodeId>,
    has_default: bool,
}

struct Graph<'a, 's> {
    sema: &'s SemanticAnalyzer,
    nodes: Vec<Node>,
    labels: FxHashMap<&'a str, NodeId>,
    gotos: Vec<(NodeId, &'a str)>,
    computed_gotos: Vec<NodeId>,
    targets: Vec<Target<'a>>,
    switches: Vec<Switch>,
}

impl<'a, 's> Graph<'a, 's> {
    fn new(sema: &'s SemanticAnalyzer) -> Self {
        Self {
            sema,
            nodes: vec![Node::Stop, Node::Stop],
            labels: FxHashMap::default(),
            gotos: Vec::new(),
            computed_gotos: Vec::new(),
            targets: Vec::new(),
            switches: Vec::new(),
        }
    }

    fn node(&mut self, node: Node) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(node);
        id
    }

    fn branch(&self, cond: &Expr, yes: NodeId, no: NodeId) -> Node {
        let truth = self
            .sema
            .condition_truth
            .get(&cond.id())
            .copied()
            .unwrap_or_else(|| self.sema.constant_truth_value(cond));
        match truth {
            Some(true) => Node::Next(yes),
            Some(false) => Node::Next(no),
            None => Node::Branch(yes, no),
        }
    }

    fn target(&self, name: Option<&str>, is_continue: bool) -> NodeId {
        self.targets
            .iter()
            .rev()
            .find_map(|target| {
                if name.is_some_and(|name| !target.names.contains(&name)) {
                    None
                } else if is_continue {
                    target.continue_to
                } else {
                    Some(target.break_to)
                }
            })
            .unwrap_or(DIVERGES) // An invalid transfer is diagnosed by sema.
    }

    fn compound(&mut self, compound: &'a CompoundStmt, next: NodeId) -> NodeId {
        self.compound_with_names(compound, next, &[])
    }

    fn compound_with_names(
        &mut self,
        compound: &'a CompoundStmt,
        mut next: NodeId,
        names: &[&'a str],
    ) -> NodeId {
        // A C2y selection/loop initializer is represented by a synthetic
        // compound ending in the actual named loop/switch. Keep its target
        // names; an ordinary compound does not introduce a break target.
        let mut names = if compound.is_synthetic_init_scope {
            names
        } else {
            &[]
        };
        for item in compound.items.iter().rev() {
            match item {
                BlockItem::Statement(stmt) => {
                    next = self.stmt(stmt, next, names);
                    names = &[];
                }
                BlockItem::Declaration(decl) => next = self.declaration(decl, next),
                BlockItem::NestedFunction(_) | BlockItem::StaticAssert(_) => {}
            }
        }
        next
    }

    fn stmt(&mut self, stmt: &'a Stmt, next: NodeId, names: &[&'a str]) -> NodeId {
        match stmt {
            Stmt::Return(Some(expr), _) => self.expr(expr, DIVERGES),
            Stmt::Return(None, _) => DIVERGES,
            Stmt::Break(name, _) => self.target(name.as_deref(), false),
            Stmt::Continue(name, _) => self.target(name.as_deref(), true),
            Stmt::Goto(name, _) => {
                let node = self.node(Node::Next(DIVERGES));
                self.gotos.push((node, name));
                node
            }
            Stmt::GotoIndirect(expr, _) => {
                let node = self.node(Node::Stop);
                if let Expr::LabelAddr(label, _) = expr.as_ref() {
                    self.gotos.push((node, label));
                } else {
                    self.computed_gotos.push(node);
                }
                self.expr(expr, node)
            }
            Stmt::If(cond, yes, no, _) => {
                let yes = self.stmt(yes, next, &[]);
                let no = no.as_ref().map_or(next, |stmt| self.stmt(stmt, next, &[]));
                let branch = self.node(self.branch(cond, yes, no));
                self.expr(cond, branch)
            }
            Stmt::While(cond, body, _) | Stmt::DoWhile(body, cond, _) => {
                let test = self.node(Node::Stop);
                self.targets.push(Target {
                    names: names.to_vec(),
                    break_to: next,
                    continue_to: Some(test),
                });
                let body = self.stmt(body, test, &[]);
                let branch = self.node(self.branch(cond, body, next));
                let condition = self.expr(cond, branch);
                self.nodes[test] = Node::Next(condition);
                self.targets.pop();
                if matches!(stmt, Stmt::DoWhile(..)) {
                    body
                } else {
                    test
                }
            }
            Stmt::For(init, cond, inc, body, _) => {
                let test = self.node(Node::Stop);
                let increment = self.node(Node::Stop);
                self.targets.push(Target {
                    names: names.to_vec(),
                    break_to: next,
                    continue_to: Some(increment),
                });
                let body = self.stmt(body, increment, &[]);
                let branch = cond
                    .as_ref()
                    .map_or(Node::Next(body), |cond| self.branch(cond, body, next));
                let branch = self.node(branch);
                let condition = cond.as_ref().map_or(branch, |expr| self.expr(expr, branch));
                self.nodes[test] = Node::Next(condition);
                let advance = inc.as_ref().map_or(test, |expr| self.expr(expr, test));
                self.nodes[increment] = Node::Next(advance);
                self.targets.pop();
                match init.as_deref() {
                    Some(ForInit::Expr(expr)) => self.expr(expr, test),
                    Some(ForInit::Declaration(decl)) => self.declaration(decl, test),
                    None => test,
                }
            }
            Stmt::Switch(selector, body, _) => {
                self.targets.push(Target {
                    names: names.to_vec(),
                    break_to: next,
                    continue_to: None,
                });
                self.switches.push(Switch::default());
                self.stmt(body, next, &[]);
                let dispatch = self.switches.pop().unwrap_or_default();
                self.targets.pop();
                let mut cases = dispatch.cases;
                if !dispatch.has_default {
                    cases.push(next);
                }
                let dispatch = self.node(Node::Dispatch(cases));
                self.expr(selector, dispatch)
            }
            Stmt::Case(_, body, _) | Stmt::CaseRange(_, _, body, _) | Stmt::Default(body, _) => {
                let entry = self.stmt(body, next, &[]);
                if let Some(dispatch) = self.switches.last_mut() {
                    dispatch.cases.push(entry);
                    dispatch.has_default |= matches!(stmt, Stmt::Default(..));
                }
                entry
            }
            Stmt::Label(..) => {
                let mut labels = Vec::new();
                let mut inner = stmt;
                while let Stmt::Label(name, body, _) = inner {
                    labels.push(name.as_str());
                    inner = body;
                }
                let entry = self.stmt(inner, next, &labels);
                for label in labels {
                    self.labels.insert(label, entry);
                }
                entry
            }
            Stmt::Compound(body) => self.compound_with_names(body, next, names),
            Stmt::Expr(Some(expr)) => self.expr(expr, next),
            Stmt::InlineAsm { goto_labels, .. } if !goto_labels.is_empty() => {
                let jump = self.node(Node::Dispatch(vec![next]));
                // An asm goto may fall through or transfer to any listed label.
                // Resolve after the full statement tree has been visited.
                for label in goto_labels {
                    self.gotos.push((jump, label));
                }
                jump
            }
            Stmt::Declaration(decl) => self.declaration(decl, next),
            Stmt::Expr(None) | Stmt::InlineAsm { .. } => next,
        }
    }

    fn declaration(&mut self, decl: &'a super::Declaration, mut next: NodeId) -> NodeId {
        // Use the canonical visitor for bounds and initializer expressions.
        let mut expressions = Vec::new();
        for declarator in &decl.declarators {
            declarator.for_each_expr(&mut |expr| expressions.push(expr));
        }
        for expr in expressions.into_iter().rev() {
            next = self.expr(expr, next);
        }
        next
    }

    fn expr(&mut self, expr: &'a Expr, next: NodeId) -> NodeId {
        // Preserve the existing expression-level divergence contract. Direct
        // statement expressions share this graph so their gotos can resolve
        // against the enclosing statement tree, rather than a second scope.
        match expr {
            Expr::StmtExpr(body, _) => self.compound(body, next),
            Expr::Cast(_, inner, ..) => self.expr(inner, next),
            Expr::Conditional(cond, yes, no, _) => {
                let yes = self.expr(yes, next);
                let no = self.expr(no, next);
                let branch = self.node(self.branch(cond, yes, no));
                self.expr(cond, branch)
            }
            Expr::GnuConditional(cond, no, _) => {
                let no = self.expr(no, next);
                let branch = self.node(self.branch(cond, next, no));
                self.expr(cond, branch)
            }
            Expr::BinaryOp(op @ (BinOp::LogicalAnd | BinOp::LogicalOr), lhs, rhs, _) => {
                let right = self.expr(rhs, next);
                let (yes, no) = if *op == BinOp::LogicalAnd {
                    (right, next)
                } else {
                    (next, right)
                };
                let branch = self.node(self.branch(lhs, yes, no));
                self.expr(lhs, branch)
            }
            Expr::Comma(lhs, rhs, _) => {
                let right = self.expr(rhs, next);
                self.expr(lhs, right)
            }
            Expr::GenericSelection(controlling, associations, _) => {
                // The controlling expression is unevaluated; only the
                // association selected by the same type checker used by sema
                // contributes run-time control flow. Never visit all arms.
                self.sema
                    .expr_type_checker()
                    .generic_selection_index(controlling, associations)
                    .and_then(|index| associations.get(index))
                    .map_or(next, |association| self.expr(&association.expr, next))
            }
            // These operands are unevaluated. VLA bounds written in the
            // corresponding type names are a separate, still-conservative
            // limitation (see the follow-up audit note).
            Expr::Sizeof(..)
            | Expr::Countof(..)
            | Expr::Alignof(..)
            | Expr::GnuAlignof(..)
            | Expr::AlignofExpr(..)
            | Expr::GnuAlignofExpr(..)
            | Expr::TypeOperand(..)
            | Expr::BuiltinTypesCompatibleP(..)
            | Expr::LabelAddr(..) => next,
            _ if self.sema.is_noreturn_call(expr) => self.expr_children(expr, DIVERGES),
            _ => self.expr_children(expr, next),
        }
    }

    /// Traverse evaluated subexpressions in AST source order. Their C
    /// evaluation order is often unspecified, but every operand must be
    /// evaluated on a path that reaches the enclosing expression's
    /// continuation. Statement expressions nested in calls, assignments,
    /// subscripts, compound-literal initializers, and other ordinary operands
    /// therefore share the enclosing transfer targets and label namespace.
    fn expr_children(&mut self, expr: &'a Expr, mut next: NodeId) -> NodeId {
        let mut children = Vec::new();
        expr.for_each_child(&mut |child| children.push(child));
        for child in children.into_iter().rev() {
            next = self.expr(child, next);
        }
        next
    }

    fn reaches_end(mut self, entry: NodeId) -> bool {
        for (node, label) in self.gotos {
            if let Some(&target) = self.labels.get(label) {
                match &mut self.nodes[node] {
                    Node::Dispatch(edges) => edges.push(target),
                    node => *node = Node::Next(target),
                }
            }
        }
        // Computed-goto destination values are not tracked by this analysis.
        // All labels are a conservative superset; never assume it cannot exit.
        if !self.computed_gotos.is_empty() {
            let labels = self.labels.values().copied().collect();
            let hub = self.nodes.len();
            self.nodes.push(Node::Dispatch(labels));
            for node in self.computed_gotos {
                self.nodes[node] = Node::Next(hub);
            }
        }
        let mut seen = vec![false; self.nodes.len()];
        let mut pending = vec![entry];
        while let Some(node) = pending.pop() {
            if node == END {
                return true;
            }
            if std::mem::replace(&mut seen[node], true) {
                continue;
            }
            match &self.nodes[node] {
                Node::Stop => {}
                Node::Next(next) => pending.push(*next),
                Node::Branch(yes, no) => {
                    pending.push(*yes);
                    pending.push(*no);
                }
                Node::Dispatch(edges) => pending.extend(edges.iter().copied()),
            }
        }
        false
    }
}

impl SemanticAnalyzer {
    pub(super) fn compound_can_fall_through(&self, body: &CompoundStmt) -> bool {
        let mut graph = Graph::new(self);
        let entry = graph.compound(body, END);
        graph.reaches_end(entry)
    }
}
