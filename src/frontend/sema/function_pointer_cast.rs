//! GCC's deliberately permissive -Wcast-function-type predicate, not general
//! C type compatibility. Pointer parameters, equal-width non-boolean integers,
//! unprototyped parameter lists and void (*)(void) have distinct exemptions.
use crate::common::types::{CType, FunctionType, ctypes_compatible};

fn signature(ty: &CType) -> Option<&FunctionType> {
    match ty {
        CType::Function(ft) => Some(ft),
        CType::Pointer(inner, _) => match inner.as_ref() {
            CType::Function(ft) => Some(ft),
            _ => None,
        },
        _ => None,
    }
}

fn benign(a: &CType, b: &CType) -> bool {
    let pointer = |t: &CType| {
        matches!(
            t,
            CType::Pointer(..) | CType::Array(..) | CType::Function(_)
        )
    };
    (pointer(a) && pointer(b))
        || ctypes_compatible(a, b)
        || (a.is_integer()
            && b.is_integer()
            && !matches!(a, CType::Bool)
            && !matches!(b, CType::Bool)
            && a.size() == b.size())
}

pub(super) fn incompatible(from: &CType, to: &CType) -> bool {
    let (Some(a), Some(b)) = (signature(from), signature(to)) else {
        return false;
    };
    let wildcard = |f: &FunctionType| {
        f.return_type == CType::Void && f.has_prototype && f.params.is_empty() && !f.variadic
    };
    if wildcard(a) || wildcard(b) {
        return false;
    }
    if !benign(&a.return_type, &b.return_type) {
        return true;
    }
    if !a.has_prototype || !b.has_prototype {
        return false;
    }
    let arity_ok = match (a.variadic, b.variadic) {
        (false, false) => a.params.len() == b.params.len(),
        (true, false) => b.params.len() >= a.params.len(),
        (false, true) => a.params.len() >= b.params.len(),
        (true, true) => true,
    };
    !arity_ok
        || a.params
            .iter()
            .zip(&b.params)
            .any(|((x, _), (y, _))| !benign(x, y))
}
