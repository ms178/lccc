//! Runtime layouts of variably modified (VM) types reached through
//! expressions.
//!
//! A VLA's size and subscript strides exist only as IR values computed where
//! its declaration is reached (`LocalInfo::vla_size` / `vla_strides`); the
//! static `CType` of a VLA is an array of unknown bound. `typeof(e)` (and a
//! `typedef` or `sizeof` of it) must reuse those values: `typeof(m[i])` for
//! `int m[n][k]` is `int[k]` with the runtime `k` of `m`'s declaration, not
//! a re-evaluated bound and not a pointer-sized fallback. Its operand is
//! evaluated when its type is variably modified (C23 6.7.3.6p4).

use super::lower::Lowerer;
use crate::common::types::target_int_ir_type;
use crate::frontend::parser::ast::{DerivedDeclarator, Expr, TypeSpecifier};
use crate::ir::reexports::{IrBinOp, IrConst, Operand, Value};

/// The runtime layout of an object of VM type: `size` is its byte size when
/// that is a runtime value (`None` when it is a compile-time constant, e.g.
/// a pointer to a VLA); `strides[d]` is the runtime byte size of what `d + 1`
/// subscripts reach, as in `LocalInfo::vla_strides`.
#[derive(Clone, Debug, Default)]
pub(super) struct VmLayout {
    pub size: Option<Value>,
    pub strides: Vec<Option<Value>>,
}

impl VmLayout {
    fn is_variably_modified(&self) -> bool {
        self.size.is_some() || self.strides.iter().any(Option::is_some)
    }

    /// The layout of a pointer to an object of this layout (`&a`).
    fn address(self) -> VmLayout {
        let mut strides = Vec::with_capacity(self.strides.len() + 1);
        strides.push(self.size);
        strides.extend(self.strides);
        VmLayout {
            size: None,
            strides,
        }
    }

    /// The layout of an array of this layout after it decays to a pointer
    /// to its first element: the same strides, a constant (pointer) size. A
    /// pointer is unchanged.
    pub(super) fn decayed(self) -> VmLayout {
        VmLayout {
            size: None,
            strides: self.strides,
        }
    }

    /// The layout of what one subscript or dereference reaches.
    fn element(self) -> VmLayout {
        let mut strides = self.strides.into_iter();
        VmLayout {
            size: strides.next().flatten(),
            strides: strides.collect(),
        }
    }
}

/// One declarator level, in the order `build_full_ctype_with_base` applies
/// them (innermost first).
enum Level<'a> {
    Pointer,
    Array(Option<&'a Expr>),
}

impl Lowerer {
    /// The runtime layout of `expr`'s type when it is a VM type this
    /// function tracks: a VLA or pointer-to-VLA local, or what subscripts and
    /// dereferences of one reach. `None` when the type is not variably
    /// modified (or is one whose layout is not tracked here).
    pub(super) fn vm_layout_of_expr(&self, expr: &Expr) -> Option<VmLayout> {
        let layout = match expr {
            Expr::Identifier(name, _) => {
                if self.identifier_enumerator(expr, name).is_some() {
                    return None;
                }
                let info = self.func_state.as_ref()?.locals.get(name)?;
                VmLayout {
                    size: info.vla_size,
                    strides: info.vla_strides.clone(),
                }
            }
            // `a[i]` and `i[a]` both reach an element of the VM operand.
            Expr::ArraySubscript(lhs, rhs, _) => self
                .vm_layout_of_expr(lhs)
                .or_else(|| self.vm_layout_of_expr(rhs))?
                .element(),
            Expr::Deref(inner, _) => self.vm_layout_of_expr(inner)?.element(),
            Expr::AddressOf(inner, _) => self.vm_layout_of_expr(inner)?.address(),
            _ => return None,
        };
        layout.is_variably_modified().then_some(layout)
    }

    /// The operand of a `typeof(expr)` specifier (through `typeof(type)`
    /// wrappers of one).
    pub(super) fn typeof_operand(type_spec: &TypeSpecifier) -> Option<&Expr> {
        match type_spec {
            TypeSpecifier::Typeof(operand, _) => Some(operand),
            TypeSpecifier::TypeofType(inner) => Self::typeof_operand(inner),
            _ => None,
        }
    }

    /// For a `typeof(expr)` specifier of VM type: evaluate the operand when
    /// it may have side effects (it is evaluated, C23 6.7.3.6p4) and return
    /// its layout. `None` for any other specifier.
    pub(super) fn typeof_vm_base(&mut self, type_spec: &TypeSpecifier) -> Option<VmLayout> {
        let operand = Self::typeof_operand(type_spec)?;
        let layout = self.vm_layout_of_expr(operand)?;
        if operand.may_have_side_effects() {
            self.lower_expr_discarded(operand);
        }
        Some(layout)
    }

    /// The layout of an object declared with `derived` over a VM base type of
    /// layout `base` whose compile-time size (when `base.size` is `None`) is
    /// `base_const_size`. Array bounds that are not constant are evaluated
    /// here, once. `None` for a function declarator.
    pub(super) fn declared_vm_layout(
        &mut self,
        base: &VmLayout,
        base_const_size: usize,
        derived: &[DerivedDeclarator],
    ) -> Option<VmLayout> {
        let mut levels = Vec::with_capacity(derived.len());
        let mut i = 0;
        while i < derived.len() {
            match &derived[i] {
                DerivedDeclarator::Pointer(..) => {
                    levels.push(Level::Pointer);
                    i += 1;
                }
                DerivedDeclarator::Array(_) => {
                    let start = i;
                    while matches!(derived.get(i), Some(DerivedDeclarator::Array(_))) {
                        i += 1;
                    }
                    for d in derived[start..i].iter().rev() {
                        if let DerivedDeclarator::Array(bound) = d {
                            levels.push(Level::Array(bound.as_deref()));
                        }
                    }
                }
                DerivedDeclarator::Function(..) | DerivedDeclarator::FunctionPointer(..) => {
                    return None;
                }
            }
        }
        let ptr_int_ty = target_int_ir_type();
        // sizes[k]: the runtime size of the type after applying levels[..=k],
        // or its constant size.
        let mut runtime = base.size;
        let mut constant = base_const_size;
        let mut sizes: Vec<(Option<Value>, usize)> = Vec::with_capacity(levels.len());
        for level in &levels {
            match level {
                Level::Pointer => {
                    runtime = None;
                    constant = crate::common::types::target_ptr_size();
                }
                Level::Array(bound) => {
                    let count = bound.map(|b| match self.expr_as_array_size(b) {
                        Some(n) => Operand::Const(IrConst::ptr_int(n)),
                        None => {
                            let op = self.lower_expr(b);
                            let ty = self.value_ir_type(b);
                            self.emit_implicit_cast(op, ty, ptr_int_ty)
                        }
                    });
                    let count = count.unwrap_or(Operand::Const(IrConst::ptr_int(0)));
                    match (runtime, &count) {
                        (None, Operand::Const(c)) => {
                            constant = constant.saturating_mul(c.to_i64().unwrap_or(0) as usize);
                        }
                        (None, Operand::Value(_)) => {
                            runtime = Some(self.emit_binop_val(
                                IrBinOp::Mul,
                                count,
                                Operand::Const(IrConst::ptr_int(constant as i64)),
                                ptr_int_ty,
                            ));
                        }
                        (Some(size), _) => {
                            runtime = Some(self.emit_binop_val(
                                IrBinOp::Mul,
                                count,
                                Operand::Value(size),
                                ptr_int_ty,
                            ));
                        }
                    }
                }
            }
            sizes.push((runtime, constant));
        }
        // Outermost level first: subscript depth d reaches the type built
        // by the levels below it; past the declarator, the base's strides.
        let mut strides: Vec<Option<Value>> = (0..levels.len())
            .rev()
            .map(|k| if k == 0 { base.size } else { sizes[k - 1].0 })
            .collect();
        strides.extend(base.strides.iter().copied());
        let size = sizes.last().map_or(base.size, |s| s.0);
        Some(VmLayout { size, strides })
    }
}
