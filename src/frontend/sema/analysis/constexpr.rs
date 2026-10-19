//! C23 `constexpr` objects (6.7.1p5-p6, p12; 6.6p7-p8).
//!
//! A `constexpr` object is a named constant: its initializer must be a
//! constant of a kind fixed by the object's type, the value must be exactly
//! representable in that type, and the object may then appear wherever a
//! constant of that kind may -- integer ones (and integer members of
//! `constexpr` structures) in integer constant expressions. The diagnostics
//! and their wording follow GCC 16.2, which implements the constraints as
//! written; Clang 23.1.3 accepts two of them (`constexpr _Bool b =
//! __builtin_inf() == __builtin_inf();` and a tag defined in the
//! declaration), which the standard does not allow.

use super::super::init_cursor::{InitContext, InitLeaf, InitMember, init_leaves};
use super::{SemanticAnalyzer, fold_to_type, integer_representable, integer_value_in_type};
use crate::common::source::Span;
use crate::common::symbol_table::FoldedValue;
use crate::common::type_builder;
use crate::common::types::CType;
use crate::frontend::parser::ast::{
    Declaration, DerivedDeclarator, Expr, GenericAssociation, InitDeclarator, Initializer,
    TypeSpecifier,
};
use crate::ir::reexports::IrConst;

const NOT_ICE: &str = "'constexpr' integer initializer is not an integer constant expression";
const NOT_REPRESENTABLE: &str = "'constexpr' initializer not representable in type of object";
const NOT_NULL: &str = "'constexpr' pointer initializer is not null";
const NOT_CONSTANT: &str = "initializer element is not constant";

impl InitContext for SemanticAnalyzer {
    fn members(&self, record: &CType) -> Option<Vec<InitMember>> {
        let (CType::Struct(key) | CType::Union(key)) = record else {
            return None;
        };
        let layouts = self.result.type_context.borrow_struct_layouts();
        let layout = layouts.get(&**key)?;
        Some(
            layout
                .fields
                .iter()
                // An unnamed bit-field takes no initializer (C11 6.7.9p9).
                .filter(|f| !(f.name.is_empty() && f.bit_width.is_some()))
                .map(|f| InitMember {
                    name: f.name.clone(),
                    ty: f.ty.clone(),
                    bit_width: f.bit_width,
                })
                .collect(),
        )
    }

    fn index_value(&self, expr: &Expr) -> Option<usize> {
        usize::try_from(self.eval_const_expr_value(expr)?.to_i128()?).ok()
    }

    fn expr_type(&self, expr: &Expr) -> Option<CType> {
        self.expr_type_checker().infer_expr_ctype(expr)
    }
}

impl SemanticAnalyzer {
    /// The rules on a `constexpr` declaration itself, before its type is
    /// registered (a tag it would introduce must not exist yet to be found
    /// new).
    pub(super) fn check_constexpr_declaration(&mut self, decl: &Declaration) {
        let mut errors: Vec<(String, Span)> = Vec::new();
        if decl.is_typedef() {
            errors.push(("'constexpr' used with 'typedef'".into(), decl.span));
            self.emit_constexpr_errors(errors);
            return;
        }
        if decl.is_extern() {
            errors.push(("'constexpr' used with 'extern'".into(), decl.span));
        }
        // GCC 16.2 names the later of two conflicting specifiers first.
        let conflict = |peer: &str| {
            if decl.is_constexpr_last() {
                format!("'constexpr' used with {peer}")
            } else {
                format!("{peer} used with 'constexpr'")
            }
        };
        if decl.is_thread_local() {
            errors.push((conflict("'_Thread_local'"), decl.span));
        }
        // `auto` beside a type specifier is the storage class; alone it
        // infers the type (6.7.10), which `constexpr` allows.
        if decl
            .declarators
            .first()
            .is_some_and(|d| d.attrs.is_nested_fn())
            && !matches!(decl.type_spec, TypeSpecifier::AutoType)
        {
            errors.push((conflict("'auto'"), decl.span));
        }
        // 6.7.1p12: an underspecified declaration declares no tag.
        if let Some(message) = self.underspecified_tag(&decl.type_spec) {
            errors.push((message, decl.span));
        }
        if let Some(second) = decl.declarators.get(1) {
            errors.push((
                "'constexpr' may only be used with a single declarator".into(),
                second.span,
            ));
        }
        for d in &decl.declarators {
            let ty = type_builder::build_full_ctype(self, &decl.type_spec, &d.derived);
            if d.init.is_none() || matches!(ty, CType::Function(_)) {
                errors.push((
                    "'constexpr' requires an initialized data declaration".into(),
                    decl.span,
                ));
            }
            // 6.7.1p5: no volatile, restrict or atomic object. An array's
            // object qualifier is its elements'.
            let all_arrays = !d.derived.is_empty()
                && d.derived
                    .iter()
                    .all(|x| matches!(x, DerivedDeclarator::Array(_)));
            let volatile = d.attrs.object_volatile || (all_arrays && d.attrs.base_volatile);
            if volatile || d.attrs.object_restrict || d.attrs.object_atomic {
                errors.push(("invalid qualifiers for 'constexpr' object".into(), d.span));
            }
        }
        self.emit_constexpr_errors(errors);
    }

    /// The diagnostic for a tag a `constexpr` declaration's specifier
    /// defines, or declares for the first time.
    fn underspecified_tag(&self, spec: &TypeSpecifier) -> Option<String> {
        let (kind, tag, defines) = match spec {
            TypeSpecifier::Struct(tag, fields, ..) => ("struct", tag, fields.is_some()),
            TypeSpecifier::Union(tag, fields, ..) => ("union", tag, fields.is_some()),
            TypeSpecifier::Enum(tag, variants, ..) => ("enum", tag, variants.is_some()),
            _ => return None,
        };
        let name = tag.as_deref().unwrap_or("<anonymous>");
        if defines {
            return Some(format!(
                "'{kind} {name}' defined in underspecified object declaration"
            ));
        }
        let tag = tag.as_deref()?;
        let known = if kind == "enum" {
            self.result.type_context.enum_types.contains_key(tag)
        } else {
            let base = format!("{kind}.{tag}");
            let key = self.result.type_context.resolve_record_key(&base);
            self.result
                .type_context
                .borrow_struct_layouts()
                .contains_key(key.as_ref())
        };
        (!known).then(|| format!("'{kind} {name}' declared in underspecified object declaration"))
    }

    /// The rules on a `constexpr` object's initializer (6.7.1p5-p6), then
    /// its registration as a named constant.
    pub(super) fn check_constexpr_initializer(
        &mut self,
        decl: &Declaration,
        init_decl: &InitDeclarator,
        init: &Initializer,
    ) {
        let Some(ty) = self
            .symbol_table
            .lookup(&init_decl.name)
            .map(|s| s.ty.clone())
        else {
            return;
        };
        if matches!(ty, CType::Function(_)) {
            return;
        }
        // The leaves that are no constant at all, by the static-initializer
        // rules (arithmetic or address constants), whatever the storage
        // duration: they get the general diagnostic, the others the
        // constexpr-specific ones (GCC: `(long)&f` is "not an integer
        // constant expression" even at block scope).
        let non_constant = {
            let type_checker = self.expr_type_checker();
            let eval = |e: &Expr| self.eval_initializer_value(e);
            let ctype = |e: &Expr| type_checker.infer_expr_ctype(e);
            let select = |controlling: &Expr, associations: &[GenericAssociation]| {
                type_checker.generic_selection_index(controlling, associations)
            };
            super::super::static_init::StaticInitChecker::new(
                &self.symbol_table,
                &eval,
                &ctype,
                &select,
            )
            .non_constant_leaves(init)
        };
        // A file-scope or `static` object's non-constant leaves were already
        // reported by `check_object_initializer`.
        let reported = decl.is_static() || self.symbol_table.at_file_scope();
        let leaves = init_leaves(self, &ty, init);
        // Members without an initializer are zero (C11 6.7.9p10); the
        // explicit values, recorded after, replace them.
        let explicit: Vec<&str> = leaves.iter().map(|l| l.path.as_str()).collect();
        let mut members = Vec::new();
        self.implicit_zero_members(&ty, "", &explicit, &mut members);
        let mut errors: Vec<(String, Span)> = Vec::new();
        for leaf in leaves {
            let span = leaf.expr.span();
            if non_constant.contains(&span) {
                if !reported {
                    errors.push((NOT_CONSTANT.into(), span));
                }
                continue;
            }
            match self.constexpr_leaf(&leaf) {
                Err(message) => errors.push((message.into(), span)),
                Ok(Some(value)) if leaf.path.starts_with('.') && !leaf.path.contains('[') => {
                    members.push((leaf.path.clone(), value));
                }
                Ok(_) => {}
            }
        }
        self.emit_constexpr_errors(errors);
        self.symbol_table.mark_constexpr(&init_decl.name, members);
    }

    /// One initializer leaf of a `constexpr` object: its error, or its
    /// value converted to the leaf's type when that is a scalar the named
    /// constant rules can use.
    fn constexpr_leaf(&self, leaf: &InitLeaf<'_>) -> Result<Option<FoldedValue>, &'static str> {
        let expr = leaf.expr;
        let checker = self.expr_type_checker();
        let source = checker.infer_expr_ctype(expr);
        if leaf.ty.is_integer() {
            if !checker.is_integer_constant_expr(expr) {
                return Err(NOT_ICE);
            }
            let value = self
                .eval_const_expr_value(expr)
                .and_then(|c| c.to_i128())
                .ok_or(NOT_ICE)?;
            let value = source
                .as_ref()
                .map_or(value, |s| integer_value_in_type(value, s));
            if !integer_representable(value, &leaf.ty)
                || leaf
                    .bit_width
                    .is_some_and(|w| !bit_field_representable(value, w, &leaf.ty))
            {
                return Err(NOT_REPRESENTABLE);
            }
            return Ok(fold_to_type(IrConst::I128(value), None, &leaf.ty));
        }
        if let Some(real) = real_type(&leaf.ty) {
            // A complex initializer value is not folded here; only a real
            // one has a representability verdict.
            let Some(value) = self.eval_const_expr_value(expr) else {
                return Ok(None);
            };
            if !float_representable(value, source.as_ref(), &real) {
                return Err(NOT_REPRESENTABLE);
            }
            if leaf.ty.is_complex() {
                return Ok(None);
            }
            return Ok(fold_to_type(value, source.as_ref(), &real));
        }
        if matches!(leaf.ty, CType::Pointer(..)) {
            let null = checker.is_null_pointer_constant(expr)
                || matches!(expr, Expr::Identifier(name, _) if self.symbol_table.is_constexpr(name));
            return if null { Ok(None) } else { Err(NOT_NULL) };
        }
        Ok(None)
    }

    /// The zero value of every scalar member of `ty` (at `prefix`) reachable
    /// by `.` -- array elements are not named constants here. A union holds
    /// only its active member: the one an explicit path reaches, else the
    /// first.
    fn implicit_zero_members(
        &self,
        ty: &CType,
        prefix: &str,
        explicit: &[&str],
        out: &mut Vec<(String, FoldedValue)>,
    ) {
        let Some(members) = self.members(ty) else {
            if !prefix.is_empty() {
                if let Some(zero) = fold_to_type(IrConst::I64(0), None, ty) {
                    out.push((prefix.to_string(), zero));
                }
            }
            return;
        };
        let active = if matches!(ty, CType::Union(_)) {
            let reached = members
                .iter()
                .position(|m| self.member_reached(m, prefix, explicit));
            match reached.or((!members.is_empty()).then_some(0)) {
                Some(i) => i..i + 1,
                None => 0..0,
            }
        } else {
            0..members.len()
        };
        for m in &members[active] {
            if matches!(m.ty, CType::Array(..)) {
                continue;
            }
            let path = if m.name.is_empty() {
                prefix.to_string()
            } else {
                format!("{prefix}.{}", m.name)
            };
            self.implicit_zero_members(&m.ty, &path, explicit, out);
        }
    }

    /// Whether an explicit initializer path reaches member `m` of the record
    /// at `prefix` (an anonymous member: any member inside it).
    fn member_reached(&self, m: &InitMember, prefix: &str, explicit: &[&str]) -> bool {
        if m.name.is_empty() {
            return self.members(&m.ty).is_some_and(|inner| {
                inner
                    .iter()
                    .any(|i| self.member_reached(i, prefix, explicit))
            });
        }
        let path = format!("{prefix}.{}", m.name);
        explicit.iter().any(|p| {
            p.strip_prefix(path.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
        })
    }

    fn emit_constexpr_errors(&self, errors: Vec<(String, Span)>) {
        let mut diagnostics = self.diagnostics.borrow_mut();
        for (message, span) in errors {
            diagnostics.error(message, span);
        }
    }
}

/// Whether `value` fits a bit-field of `width` bits and integer type `ty`
/// (a plain `int` bit-field is signed, as in GCC).
fn bit_field_representable(value: i128, width: u32, ty: &CType) -> bool {
    if width == 0 || width >= 128 {
        return true;
    }
    let signed = match ty {
        CType::Enum(e) => e.compatible_ctype().is_signed(),
        other => other.is_signed(),
    };
    if signed {
        let half = 1i128 << (width - 1);
        (-half..half).contains(&value)
    } else {
        (0..(1i128 << width)).contains(&value)
    }
}

/// The real type of a real or complex floating type.
fn real_type(ty: &CType) -> Option<CType> {
    match ty {
        CType::Float | CType::ComplexFloat => Some(CType::Float),
        CType::Double | CType::ComplexDouble => Some(CType::Double),
        CType::LongDouble | CType::ComplexLongDouble => Some(CType::LongDouble),
        _ => None,
    }
}

/// Whether the integer `value` converts to a binary floating type with a
/// `mantissa`-bit significand without rounding.
fn integer_fits_mantissa(value: i128, mantissa: u32) -> bool {
    let magnitude = value.unsigned_abs();
    magnitude == 0 || (magnitude >> magnitude.trailing_zeros()) < (1u128 << mantissa)
}

/// Whether the constant `value` (of type `source`) converts to the real
/// floating type `real` exactly (6.7.1p6). Overflow to an infinity of the
/// same sign is a rounding, and so not exact, unless the value is already
/// infinite; a NaN converts to a NaN.
fn float_representable(value: IrConst, source: Option<&CType>, real: &CType) -> bool {
    let float_exact = |v: f64| v.is_nan() || f64::from(v as f32) == v;
    match value {
        IrConst::LongDouble(approx, bytes) => {
            // Exact in double only if the binary128 image has no bits beyond
            // the double's (it is the double widened).
            let fits_double = crate::ir::constants::f64_to_f128_bytes(approx) == bytes;
            match real {
                CType::LongDouble => true,
                CType::Double => fits_double,
                _ => fits_double && float_exact(approx),
            }
        }
        IrConst::F32(_) => true,
        IrConst::F64(v) => !matches!(real, CType::Float) || float_exact(v),
        IrConst::D32(_) | IrConst::D64(_) => true,
        other => {
            let Some(raw) = other.to_i128() else {
                return true;
            };
            let v = source.map_or(raw, |s| integer_value_in_type(raw, s));
            let mantissa = match real {
                CType::Float => 24,
                CType::Double => 53,
                // binary128 (AArch64, RISC-V) or x87 extended (x86).
                _ if crate::common::types::target_long_double_is_f128() => 113,
                _ => 64,
            };
            integer_fits_mantissa(v, mantissa)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_ranges_and_bool() {
        assert!(integer_representable(255, &CType::UChar));
        assert!(!integer_representable(256, &CType::UChar));
        assert!(!integer_representable(-1, &CType::UInt));
        assert!(!integer_representable(128, &CType::SChar));
        assert!(integer_representable(-128, &CType::Char));
        assert!(integer_representable(1, &CType::Bool));
        assert!(!integer_representable(2, &CType::Bool));
        assert!(integer_representable(
            i128::from(u64::MAX),
            &CType::ULongLong
        ));
        assert!(!integer_representable(-1, &CType::ULongLong));
    }

    #[test]
    fn bit_field_ranges() {
        assert!(bit_field_representable(7, 3, &CType::UInt));
        assert!(!bit_field_representable(8, 3, &CType::UInt));
        assert!(bit_field_representable(-4, 3, &CType::Int));
        assert!(!bit_field_representable(4, 3, &CType::Int));
        assert!(!bit_field_representable(-1, 3, &CType::UInt));
    }

    #[test]
    fn unsigned_results_are_reinterpreted() {
        // `sizeof(int) - 5`: an unsigned long, so 2^64 - 1, not -1.
        assert_eq!(
            integer_value_in_type(-1, &CType::ULong),
            i128::from(u64::MAX)
        );
        assert_eq!(integer_value_in_type(-1, &CType::Int), -1);
        assert_eq!(
            integer_value_in_type(0x8000_0000, &CType::Int),
            -0x8000_0000
        );
    }

    #[test]
    fn floating_representability_is_exact() {
        assert!(!float_representable(IrConst::F64(0.1), None, &CType::Float));
        assert!(float_representable(IrConst::F64(0.5), None, &CType::Float));
        assert!(float_representable(
            IrConst::F64(f64::INFINITY),
            None,
            &CType::Float
        ));
        assert!(!float_representable(
            IrConst::F64(1e300),
            None,
            &CType::Float
        ));
        assert!(float_representable(
            IrConst::F64(f64::NAN),
            None,
            &CType::Float
        ));
        assert!(float_representable(
            IrConst::F64(0.1),
            None,
            &CType::LongDouble
        ));
        // 2^24 + 1 needs 25 significant bits.
        assert!(!float_representable(
            IrConst::I64(16_777_217),
            Some(&CType::Int),
            &CType::Float
        ));
        assert!(float_representable(
            IrConst::I64(16_777_216),
            Some(&CType::Int),
            &CType::Float
        ));
        assert!(float_representable(
            IrConst::I64(1 << 40),
            Some(&CType::Long),
            &CType::Float
        ));
    }
}
