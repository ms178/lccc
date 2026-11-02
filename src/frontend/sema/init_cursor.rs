//! The C initializer walk (C11 6.7.9p17-p23): which subobject each
//! initializer expression of a braced list initializes.
//!
//! A list initializes its object's subobjects in order; a designation resets
//! the position (p17-p18); an expression whose type is not that of an
//! aggregate subobject initializes the aggregate's first scalar instead,
//! taking further expressions for the rest ("brace elision", p20); a brace-
//! enclosed list always initializes exactly the current subobject. A union
//! takes one member (the first, or the designated one); unnamed bit-fields
//! take nothing (p9). A string literal initializes a whole character array
//! (p14-p15).
//!
//! The walk is pure: the caller supplies record layouts, designator index
//! values and expression types, so it is unit-tested without sema.

use crate::common::types::CType;
use crate::frontend::parser::ast::{Designator, Expr, Initializer, InitializerItem};

/// One initialized scalar or whole-aggregate subobject.
pub(super) struct InitLeaf<'a> {
    /// The subobject's type.
    pub ty: CType,
    /// The member path from the initialized object: `.a.b` for members (an
    /// anonymous member adds no component), `[3]` for array elements.
    pub path: String,
    /// The initializer expression.
    pub expr: &'a Expr,
    /// The width of the bit-field this leaf initializes, if it is one.
    pub bit_width: Option<u32>,
}

/// A member of a record that an initializer can name or reach positionally.
pub(super) struct InitMember {
    /// Empty for an anonymous struct or union member.
    pub name: String,
    pub ty: CType,
    /// The width of a (named) bit-field member.
    pub bit_width: Option<u32>,
}

/// What the walk needs to know from its context.
pub(super) trait InitContext {
    /// The initializable members of a struct or union, in declaration order,
    /// without unnamed bit-fields. `None` when the layout is unknown.
    fn members(&self, record: &CType) -> Option<Vec<InitMember>>;
    /// The value of an array designator's index expression.
    fn index_value(&self, expr: &Expr) -> Option<usize>;
    /// The type of an initializer expression.
    fn expr_type(&self, expr: &Expr) -> Option<CType>;
}

struct Frame {
    ty: CType,
    /// The next subobject to initialize.
    pos: usize,
    path: String,
    /// `ty` is a union: after one member it is complete.
    union: bool,
}

/// The leaves of `init` initializing an object of type `ty`. Items beyond the
/// end of the object (excess initializers) and designators that do not
/// resolve are skipped; diagnosing them is not this walk's job.
pub(super) fn init_leaves<'a>(
    cx: &dyn InitContext,
    ty: &CType,
    init: &'a Initializer,
) -> Vec<InitLeaf<'a>> {
    let mut out = Vec::new();
    walk(cx, ty, String::new(), None, init, &mut out);
    out
}

fn walk<'a>(
    cx: &dyn InitContext,
    ty: &CType,
    path: String,
    bit_width: Option<u32>,
    init: &'a Initializer,
    out: &mut Vec<InitLeaf<'a>>,
) {
    match init {
        Initializer::Expr(expr) => {
            if let Expr::CompoundLiteral(_, cl_init, _, _) = expr {
                if is_aggregate(ty) && initializes_whole(cx, ty, expr) {
                    walk(cx, ty, path, bit_width, cl_init, out);
                    return;
                }
            }
            if is_aggregate(ty) && !initializes_whole(cx, ty, expr) {
                // `int a[2] = 1;`-like: the expression initializes the first
                // scalar (an error for arrays elsewhere; harmless here).
                let items = [InitializerItem {
                    designators: Vec::new(),
                    init: Initializer::Expr(expr.clone()),
                }];
                let mut tmp = Vec::new();
                walk_list(cx, ty, path, &items, &mut tmp);
                // Re-borrow from the original expression: same node.
                out.extend(tmp.into_iter().map(|leaf| InitLeaf {
                    ty: leaf.ty,
                    path: leaf.path,
                    expr,
                    bit_width: leaf.bit_width,
                }));
            } else {
                out.push(InitLeaf {
                    ty: ty.clone(),
                    path,
                    expr,
                    bit_width,
                });
            }
        }
        Initializer::List(items) => {
            if is_aggregate(ty) {
                walk_list(cx, ty, path, items, out);
            } else if let Some(first) = items.first() {
                // `int x = { 5 };`: the scalar's single initializer.
                if first.designators.is_empty() {
                    walk(cx, ty, path, bit_width, &first.init, out);
                }
            }
        }
    }
}

fn walk_list<'a>(
    cx: &dyn InitContext,
    ty: &CType,
    path: String,
    items: &'a [InitializerItem],
    out: &mut Vec<InitLeaf<'a>>,
) {
    let mut stack = vec![frame(ty, path)];
    'items: for item in items {
        if !item.designators.is_empty() {
            stack.truncate(1);
            let last = item.designators.len() - 1;
            for (k, designator) in item.designators.iter().enumerate() {
                if !designate(cx, &mut stack, designator) {
                    continue 'items;
                }
                if k < last {
                    let Some((child_ty, child_path, _)) = current_child(cx, &stack) else {
                        continue 'items;
                    };
                    stack.push(frame(&child_ty, child_path));
                }
            }
        }
        loop {
            // Leave completed aggregates (brace elision continues in the
            // enclosing one).
            while stack.last().is_some_and(|top| frame_done(cx, top)) {
                if stack.len() == 1 {
                    break 'items; // excess initializers
                }
                stack.pop();
                let Some(parent) = stack.last_mut() else {
                    break 'items;
                };
                advance(parent);
            }
            let Some((child_ty, child_path, child_bits)) = current_child(cx, &stack) else {
                break 'items;
            };
            match &item.init {
                Initializer::List(_) => {
                    walk(cx, &child_ty, child_path, child_bits, &item.init, out);
                    let Some(top) = stack.last_mut() else {
                        break 'items;
                    };
                    advance(top);
                    break;
                }
                Initializer::Expr(expr) => {
                    if is_aggregate(&child_ty) && !initializes_whole(cx, &child_ty, expr) {
                        stack.push(frame(&child_ty, child_path));
                        continue;
                    }
                    out.push(InitLeaf {
                        ty: child_ty,
                        path: child_path,
                        expr,
                        bit_width: child_bits,
                    });
                    let Some(top) = stack.last_mut() else {
                        break 'items;
                    };
                    advance(top);
                    break;
                }
            }
        }
    }
}

fn frame(ty: &CType, path: String) -> Frame {
    Frame {
        ty: ty.clone(),
        pos: 0,
        path,
        union: matches!(ty, CType::Union(_)),
    }
}

fn is_aggregate(ty: &CType) -> bool {
    matches!(ty, CType::Struct(_) | CType::Union(_) | CType::Array(..))
}

/// Whether `expr` initializes the whole aggregate `ty` rather than its first
/// scalar: a string literal for an array of a character (or wide character)
/// type, or an expression of the same record type.
fn initializes_whole(cx: &dyn InitContext, ty: &CType, expr: &Expr) -> bool {
    match ty {
        CType::Array(elem, _) => {
            matches!(
                expr,
                Expr::StringLiteral(..)
                    | Expr::WideStringLiteral(..)
                    | Expr::Char16StringLiteral(..)
            ) && elem.is_integer()
        }
        CType::Struct(_) | CType::Union(_) => cx.expr_type(expr).as_ref() == Some(ty),
        _ => true,
    }
}

fn frame_done(cx: &dyn InitContext, f: &Frame) -> bool {
    match &f.ty {
        CType::Array(_, Some(n)) => f.pos >= *n,
        CType::Array(_, None) => false,
        CType::Struct(_) | CType::Union(_) => {
            if f.union && f.pos == usize::MAX {
                return true;
            }
            cx.members(&f.ty).is_none_or(|m| f.pos >= m.len())
        }
        _ => true,
    }
}

fn advance(f: &mut Frame) {
    f.pos = if f.union { usize::MAX } else { f.pos + 1 };
}

/// The type, path and bit-field width of the frame's current subobject.
fn current_child(cx: &dyn InitContext, stack: &[Frame]) -> Option<(CType, String, Option<u32>)> {
    let f = stack.last()?;
    match &f.ty {
        CType::Array(elem, _) => Some(((**elem).clone(), format!("{}[{}]", f.path, f.pos), None)),
        CType::Struct(_) | CType::Union(_) => {
            let members = cx.members(&f.ty)?;
            let m = members.get(f.pos)?;
            let path = if m.name.is_empty() {
                f.path.clone()
            } else {
                format!("{}.{}", f.path, m.name)
            };
            Some((m.ty.clone(), path, m.bit_width))
        }
        _ => None,
    }
}

/// Point the innermost frame at the subobject `designator` names; a member
/// of an anonymous member pushes frames for the anonymous members between.
fn designate(cx: &dyn InitContext, stack: &mut Vec<Frame>, designator: &Designator) -> bool {
    let Some(f) = stack.last_mut() else {
        return false;
    };
    match (designator, &f.ty) {
        (Designator::Index(e) | Designator::Range(e, _), CType::Array(_, len)) => {
            let Some(i) = cx.index_value(e) else {
                return false;
            };
            if len.is_some_and(|n| i >= n) {
                return false;
            }
            f.pos = i;
            // A range designator initializes every element of the range with
            // the same expression; its first element is representative.
            true
        }
        (Designator::Field(name), CType::Struct(_) | CType::Union(_)) => {
            let Some(chain) = member_chain(cx, &f.ty, name) else {
                return false;
            };
            let last = chain.len() - 1;
            for (k, pos) in chain.into_iter().enumerate() {
                let Some(top) = stack.last_mut() else {
                    return false;
                };
                top.pos = pos;
                if k < last {
                    let Some((child_ty, child_path, _)) = current_child(cx, stack) else {
                        return false;
                    };
                    stack.push(frame(&child_ty, child_path));
                }
            }
            true
        }
        _ => false,
    }
}

/// The member positions leading to `name` in `record`, through anonymous
/// members (C11 6.7.2.1p13).
fn member_chain(cx: &dyn InitContext, record: &CType, name: &str) -> Option<Vec<usize>> {
    let members = cx.members(record)?;
    if let Some(i) = members.iter().position(|m| m.name == name) {
        return Some(vec![i]);
    }
    for (i, m) in members.iter().enumerate() {
        if m.name.is_empty() && matches!(m.ty, CType::Struct(_) | CType::Union(_)) {
            if let Some(mut rest) = member_chain(cx, &m.ty, name) {
                rest.insert(0, i);
                return Some(rest);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::source::Span;

    /// Records by key: `S` = { int a[2]; int b; }, `U` = union { int i;
    /// double d; }, `A` = { struct { int x, y; }; int z; } (anonymous member),
    /// `N` = { S s; double t; }.
    struct Cx;
    impl InitContext for Cx {
        fn members(&self, record: &CType) -> Option<Vec<InitMember>> {
            let m = |name: &str, ty: CType| InitMember {
                name: name.to_string(),
                ty,
                bit_width: None,
            };
            let key = match record {
                CType::Struct(k) | CType::Union(k) => k.to_string(),
                _ => return None,
            };
            Some(match key.as_str() {
                "S" => vec![
                    m("a", CType::Array(Box::new(CType::Int), Some(2))),
                    m("b", CType::Int),
                ],
                "U" => vec![m("i", CType::Int), m("d", CType::Double)],
                "XY" => vec![m("x", CType::Int), m("y", CType::Int)],
                "A" => vec![m("", CType::Struct("XY".into())), m("z", CType::Int)],
                "N" => vec![m("s", CType::Struct("S".into())), m("t", CType::Double)],
                _ => return None,
            })
        }
        fn index_value(&self, expr: &Expr) -> Option<usize> {
            match expr {
                Expr::IntLiteral(v, _) => usize::try_from(*v).ok(),
                _ => None,
            }
        }
        fn expr_type(&self, expr: &Expr) -> Option<CType> {
            match expr {
                Expr::Identifier(name, _) if name == "s_obj" => Some(CType::Struct("S".into())),
                _ => Some(CType::Int),
            }
        }
    }

    fn int(v: i64) -> Initializer {
        Initializer::Expr(Expr::IntLiteral(v, Span::dummy()))
    }
    fn item(init: Initializer) -> InitializerItem {
        InitializerItem {
            designators: Vec::new(),
            init,
        }
    }
    fn field(name: &str, init: Initializer) -> InitializerItem {
        InitializerItem {
            designators: vec![Designator::Field(name.to_string())],
            init,
        }
    }
    fn list(items: Vec<InitializerItem>) -> Initializer {
        Initializer::List(items)
    }
    fn paths(ty: &CType, init: &Initializer) -> Vec<String> {
        init_leaves(&Cx, ty, init)
            .into_iter()
            .map(|l| {
                let v = match l.expr {
                    Expr::IntLiteral(v, _) => v.to_string(),
                    Expr::Identifier(n, _) => n.clone(),
                    _ => "?".to_string(),
                };
                format!("{}={}", l.path, v)
            })
            .collect()
    }

    #[test]
    fn brace_elision_fills_nested_arrays_then_members() {
        let s = CType::Struct("S".into());
        let init = list(vec![item(int(1)), item(int(2)), item(int(3))]);
        assert_eq!(paths(&s, &init), [".a[0]=1", ".a[1]=2", ".b=3"]);
    }

    #[test]
    fn braced_sublist_initializes_exactly_one_subobject() {
        let s = CType::Struct("S".into());
        let init = list(vec![item(list(vec![item(int(1))])), item(int(3))]);
        assert_eq!(paths(&s, &init), [".a[0]=1", ".b=3"]);
    }

    #[test]
    fn designator_resets_the_position() {
        let s = CType::Struct("S".into());
        let init = list(vec![field("b", int(9)), item(int(4))]);
        // After `.b`, the next positional item is excess (S has no member after b).
        assert_eq!(paths(&s, &init), [".b=9"]);
        let a = CType::Array(Box::new(CType::Int), Some(4));
        let idx = InitializerItem {
            designators: vec![Designator::Index(Expr::IntLiteral(2, Span::dummy()))],
            init: int(7),
        };
        let init = list(vec![idx, item(int(8))]);
        assert_eq!(paths(&a, &init), ["[2]=7", "[3]=8"]);
    }

    #[test]
    fn union_takes_one_member() {
        let u = CType::Union("U".into());
        assert_eq!(paths(&u, &list(vec![item(int(1)), item(int(2))])), [".i=1"]);
        assert_eq!(paths(&u, &list(vec![field("d", int(5))])), [".d=5"]);
    }

    #[test]
    fn anonymous_members_are_transparent_in_paths_and_designators() {
        let a = CType::Struct("A".into());
        let init = list(vec![item(int(1)), item(int(2)), item(int(3))]);
        assert_eq!(paths(&a, &init), [".x=1", ".y=2", ".z=3"]);
        let init = list(vec![field("y", int(5)), item(int(6))]);
        assert_eq!(paths(&a, &init), [".y=5", ".z=6"]);
    }

    #[test]
    fn same_type_expression_initializes_the_whole_record() {
        let n = CType::Struct("N".into());
        let s_obj = Initializer::Expr(Expr::Identifier("s_obj".into(), Span::dummy()));
        let init = list(vec![item(s_obj), item(int(2))]);
        assert_eq!(paths(&n, &init), [".s=s_obj", ".t=2"]);
    }

    #[test]
    fn scalar_in_braces_and_unknown_size_arrays() {
        assert_eq!(paths(&CType::Int, &list(vec![item(int(5))])), ["=5"]);
        let a = CType::Array(Box::new(CType::Int), None);
        let init = list(vec![item(int(1)), item(int(2)), item(int(3))]);
        assert_eq!(paths(&a, &init), ["[0]=1", "[1]=2", "[2]=3"]);
    }
}
