//! Shared planning for arrays whose leaf subobjects are Clang `ext_vector_type` objects.
//!
//! Vector objects are storage aggregates but their logical lanes belong to the
//! vector initializer, not to the containing array. This planner resolves the
//! containing array dimensions and designators once, then hands the selected
//! initializer for each vector object to either the global byte-image writer or
//! the local vector-copy/store lowering. It keeps padded vector size distinct
//! from both the lane count and the pointer-sized IR carrier.

use super::lower::Lowerer;
use crate::common::types::CType;
use crate::frontend::parser::ast::{Designator, Initializer, InitializerItem};

/// A flat, row-major plan: one optional initializer per vector object. Missing
/// entries are zero-initialized by the destination's initial byte image.
pub(super) struct VectorArrayInitPlan {
    pub(super) vector_type: CType,
    pub(super) elements: Vec<Option<Initializer>>,
}

/// Return true when `ty` is an array (possibly multidimensional) whose leaf is
/// an ext-vector object. Incomplete dimensions are deliberately not considered
/// an ext-vector-array shape until the declaration's bound has been completed.
pub(super) fn is_ext_vector_array_type(ty: &CType) -> bool {
    vector_array_leaf_count(ty).is_some_and(|(is_array, _)| is_array)
}

fn vector_array_leaf_count(ty: &CType) -> Option<(bool, usize)> {
    match ty {
        CType::ExtVector(..) => Some((false, 1)),
        CType::Array(element, Some(count)) => {
            let (_, child_count) = vector_array_leaf_count(element)?;
            Some((true, count.checked_mul(child_count)?))
        }
        _ => None,
    }
}

fn vector_leaf_type(ty: &CType) -> Option<CType> {
    match ty {
        CType::ExtVector(..) => Some(ty.clone()),
        CType::Array(element, Some(_)) => vector_leaf_type(element),
        _ => None,
    }
}

impl Lowerer {
    /// Resolve nested array braces/designators to the vector-object initializer
    /// at each row-major leaf. `array_type` must be a complete array of vectors.
    pub(super) fn plan_vector_array_init(
        &self,
        items: &[InitializerItem],
        array_type: &CType,
    ) -> Option<VectorArrayInitPlan> {
        let (is_array, total) = vector_array_leaf_count(array_type)?;
        if !is_array {
            return None;
        }
        let vector_type = vector_leaf_type(array_type)?;
        let canonical = self.canonical_init_items(items, array_type, None);
        let normalized = canonical.as_deref().unwrap_or(items);
        let mut elements = vec![None; total];
        self.plan_vector_array_level(normalized, array_type, 0, &mut elements)?;
        Some(VectorArrayInitPlan {
            vector_type,
            elements,
        })
    }

    fn plan_vector_array_level(
        &self,
        items: &[InitializerItem],
        array_type: &CType,
        base: usize,
        elements: &mut [Option<Initializer>],
    ) -> Option<()> {
        let CType::Array(element_type, Some(array_len)) = array_type else {
            return None;
        };
        let (_, leaves_per_element) = vector_array_leaf_count(element_type)?;
        let mut next_index = 0usize;

        for item in items {
            let index = match item.designators.as_slice() {
                [] => next_index,
                [Designator::Index(index_expr)] => {
                    self.eval_const_expr_for_designator(index_expr)?
                }
                // Multi-level designators are normalized into nested array
                // lists by canonical_init_items. Do not guess if a malformed
                // or unsupported designator remains.
                _ => return None,
            };
            if index >= *array_len {
                return None;
            }
            let child_base = base.checked_add(index.checked_mul(leaves_per_element)?)?;

            match (element_type.as_ref(), &item.init) {
                (CType::ExtVector(..), init) => {
                    *elements.get_mut(child_base)? = Some(init.clone());
                }
                (CType::Array(..), Initializer::List(sub_items)) => {
                    let end = child_base.checked_add(leaves_per_element)?;
                    elements.get_mut(child_base..end)?.fill(None);
                    self.plan_vector_array_level(sub_items, element_type, child_base, elements)?;
                }
                // A braced subarray replaces that entire subobject. An
                // expression cannot initialize an array object in C; valid
                // brace-elided vector expressions are expanded by the shared
                // initializer canonicalizer before reaching this point.
                (CType::Array(..), Initializer::Expr(_)) => return None,
                _ => return None,
            }
            next_index = index.saturating_add(1);
        }
        Some(())
    }
}
