//! Initializers of arrays of scalars, of any rank (C11 6.7.9).
//!
//! An initializer list for `T a[D0][D1]...[Dn]` may elide inner braces
//! (p20), designate any element or subarray (`[i][j] = ...`, p17, and GNU
//! `[lo ... hi]`), continue after a designation with the next element at
//! the designation's depth, initialize a row of a character array with a
//! string literal (p14), and initialize an element more than once: the last
//! initializer wins, and a braced one replaces its whole subarray (p19).
//! Every brace level is bounded by its subobject, so excess initializers in
//! a braced row are dropped there (the front end diagnoses them) instead of
//! spilling into the next row.
//!
//! [`Lowerer::plan_array_init`] resolves all of this once into an
//! [`ArrayInitPlan`]: the final initializer of every element that ends up
//! non-zero-initialized, keyed by the element's flat index.  Overridden
//! initializers are gone from the plan (p19 footnote: they need not be
//! evaluated), so the storage-specific consumers - block-scope stores,
//! static byte images, static element lists with relocations - only write
//! each element once and never have to undo an earlier write (a relocation
//! overridden by a constant, a braced row that clears earlier elements).
//!
//! Struct and union members that are arrays of scalars reach the planner
//! through the canonical initializer form (`init_canon`): the runs written
//! for the member anywhere in the enclosing list are one list relative to
//! the member, so the member is planned exactly like a whole array.

use std::collections::BTreeMap;

use super::lower::Lowerer;
use super::string_init::StringInit;
use crate::common::types::{CType, IrType};
use crate::frontend::parser::ast::{Designator, Expr, Initializer, InitializerItem};
use crate::ir::reexports::{GlobalInit, IrConst, Operand, Value};

/// Shape of a sized array whose innermost element is a scalar.
#[derive(Clone, Debug)]
pub(super) struct ArrayGeometry {
    /// IR type of the innermost element.
    pub base_ty: IrType,
    /// The innermost element is `_Bool` (values are normalised to 0/1).
    pub is_bool: bool,
    /// The innermost element is `long double`.
    pub is_long_double: bool,
    /// Byte size of one element at each level: `strides[0]` is the size of
    /// an element of the array itself, the last entry the scalar size.
    pub strides: Vec<usize>,
    /// Number of scalars in the whole array.
    pub total: usize,
}

impl ArrayGeometry {
    /// Byte size of one scalar.
    pub fn elem_size(&self) -> usize {
        *self
            .strides
            .last()
            .expect("an array has at least one level")
    }

    /// Scalars in one element at level `k`.
    fn elems_at(&self, strides: &[usize], k: usize) -> usize {
        strides[k] / self.elem_size()
    }
}

/// The final initializer of one element.
#[derive(Clone, Copy, Debug)]
pub(super) enum ElemInit<'a> {
    /// An initializer expression, converted to the element type on use.
    Expr(&'a Expr),
    /// A code unit of a string literal that initialized a character row.
    Unit(u32),
}

/// Resolved initializer of an array of scalars: the elements not listed
/// are zero.
#[derive(Default, Debug)]
pub(super) struct ArrayInitPlan<'a> {
    /// Flat element index -> (source order, initializer).  The source order
    /// lets consumers evaluate side-effecting initializers in the order
    /// they were written (gcc's order; the standard leaves it unspecified).
    elems: BTreeMap<usize, (usize, ElemInit<'a>)>,
    seq: usize,
    /// One past the last element any initializer reached (explicit zeros
    /// and empty braces included): what sizes `T a[] = { ... }`.
    extent: usize,
}

impl<'a> ArrayInitPlan<'a> {
    fn set(&mut self, flat: usize, init: ElemInit<'a>) {
        self.seq += 1;
        self.elems.insert(flat, (self.seq, init));
    }

    /// Reset `n` elements from `flat` to zero (a braced subobject or a string
    /// row replaces everything written there before).
    fn clear(&mut self, flat: usize, n: usize) {
        if n == 0 {
            return;
        }
        let doomed: Vec<usize> = self.elems.range(flat..flat + n).map(|(&k, _)| k).collect();
        for k in doomed {
            self.elems.remove(&k);
        }
    }

    /// Elements in flat-index order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, ElemInit<'a>)> + '_ {
        self.elems.iter().map(|(&k, &(_, init))| (k, init))
    }

    /// Elements in source order.
    pub fn in_source_order(&self) -> Vec<(usize, ElemInit<'a>)> {
        let mut v: Vec<(usize, usize, ElemInit<'a>)> = self
            .elems
            .iter()
            .map(|(&k, &(seq, init))| (seq, k, init))
            .collect();
        v.sort_unstable_by_key(|e| e.0);
        v.into_iter().map(|(_, k, init)| (k, init)).collect()
    }

    /// Number of explicitly initialized elements.
    pub fn len(&self) -> usize {
        self.elems.len()
    }

    /// Whether any initializer expression satisfies `pred`.
    pub fn any_expr(&self, mut pred: impl FnMut(&Expr) -> bool) -> bool {
        self.elems.values().any(|(_, init)| match init {
            ElemInit::Expr(e) => pred(e),
            ElemInit::Unit(_) => false,
        })
    }
}

/// How a struct walker meets an array-of-scalars member at `items[0]`.
pub(super) enum MemberArrayInit<'a> {
    /// The member's own initializer: a braced list, or a string for a
    /// character array.
    Whole(&'a Initializer),
    /// A brace-elided run (p20): the first item re-designated relative to
    /// the member, then the undesignated items that continue inside it.
    Run(Vec<InitializerItem>),
}

impl Lowerer {
    /// Geometry of a sized array of scalars of any rank: integer, `_Bool`,
    /// enumeration, pointer and real floating elements.  `None` for arrays
    /// of aggregates, complex or vector elements, and arrays of unknown
    /// bound at any level.
    pub(super) fn scalar_array_geometry(&self, arr: &CType) -> Option<ArrayGeometry> {
        let CType::Array(elem, Some(n)) = arr else {
            return None;
        };
        let mut strides = Vec::new();
        let mut total = *n;
        let mut cur: &CType = elem;
        loop {
            strides.push(self.resolve_ctype_size(cur));
            match cur {
                CType::Array(inner, Some(m)) => {
                    total *= *m;
                    cur = inner;
                }
                CType::Array(_, None) => return None,
                _ => break,
            }
        }
        let scalar = cur.is_integer()
            || matches!(
                cur,
                CType::Pointer(..) | CType::Float | CType::Double | CType::LongDouble
            );
        if !scalar || strides.last().copied().unwrap_or(0) == 0 {
            return None;
        }
        Some(ArrayGeometry {
            base_ty: IrType::from_ctype(cur),
            is_bool: *cur == CType::Bool,
            is_long_double: *cur == CType::LongDouble,
            strides,
            total,
        })
    }

    /// `ty` with the bound of an outermost `[]` supplied from the object's
    /// size (`int a[][3] = { ... }` after the initializer sized it).
    pub(super) fn complete_array_ctype(&self, ty: &CType, total_size: usize) -> CType {
        match ty {
            CType::Array(elem, None) => {
                let esz = self.resolve_ctype_size(elem).max(1);
                CType::Array(elem.clone(), Some(total_size / esz))
            }
            _ => ty.clone(),
        }
    }

    /// Resolve the braced initializer list `items` of a whole array.
    pub(super) fn plan_array_init<'a>(
        &self,
        items: &'a [InitializerItem],
        geo: &ArrayGeometry,
    ) -> ArrayInitPlan<'a> {
        let mut plan = ArrayInitPlan::default();
        let mut flat = 0usize;
        self.plan_level(items, geo, &geo.strides, 0, geo.total, &mut flat, &mut plan);
        plan
    }

    /// Resolve a string literal that initializes a whole character array
    /// (`char s[4] = "ab";` or a member `.s = "ab"`).
    pub(super) fn plan_array_string<'a>(
        &self,
        init: StringInit<'a>,
        geo: &ArrayGeometry,
    ) -> ArrayInitPlan<'a> {
        let mut plan = ArrayInitPlan::default();
        Self::place_row(init, 0, geo.total, geo, &mut plan);
        plan
    }

    /// Geometry of a flexible array member `T m[]` of scalars (or of arrays
    /// of scalars) initialized by `init` (GNU: static objects only), sized
    /// exactly as an unsized array (C11 6.7.9p22) would be.  `None` for
    /// other element types and for an initializer that is neither braced
    /// nor a string, which leaves the elided form to the legacy path.
    pub(super) fn fam_scalar_geometry(
        &self,
        elem: &CType,
        init: &Initializer,
    ) -> Option<ArrayGeometry> {
        let n = self.unsized_array_len(init, elem)?;
        self.scalar_array_geometry(&CType::Array(Box::new(elem.clone()), Some(n)))
    }

    /// Bound of `T a[] = init` (C11 6.7.9p22) for a scalar or scalar-array
    /// `elem`, or a character `elem` initialized by a string; `None` for
    /// other element types.
    pub(super) fn unsized_array_len(&self, init: &Initializer, elem: &CType) -> Option<usize> {
        if let Some(s) = StringInit::for_init_ctype(init, elem) {
            return Some(s.elems_with_nul());
        }
        let Initializer::List(items) = init else {
            return None;
        };
        // Enough elements for every initializer: each undesignated one
        // reaches at most one element further than the previous one, and a
        // designation restarts from the element it names.
        let mut bound = items.len();
        for item in items {
            let hi = match item.designators.first() {
                Some(Designator::Index(e)) => self.eval_const_expr_for_designator(e),
                Some(Designator::Range(_, hi)) => self.eval_const_expr_for_designator(hi),
                _ => None,
            };
            if let Some(hi) = hi {
                bound = bound.max(hi.checked_add(1 + items.len())?);
            }
        }
        let arr = CType::Array(Box::new(elem.clone()), Some(bound));
        let geo = self.scalar_array_geometry(&arr)?;
        let plan = self.plan_array_init(items, &geo);
        Some(plan.extent.div_ceil(geo.elems_at(&geo.strides, 0)))
    }

    /// Items taken by a brace-elided run into an array of scalars (p20):
    /// the first item (designated by `rel` relative to the array, or
    /// starting at element 0 when `rel` is empty) and the undesignated
    /// items after it until the array is full.  At least 1.
    pub(super) fn elided_run_len(
        &self,
        rel: &[Designator],
        first: &Initializer,
        rest: &[InitializerItem],
        geo: &ArrayGeometry,
    ) -> usize {
        let head = [InitializerItem {
            designators: rel.to_vec(),
            init: first.clone(),
        }];
        let mut scratch = ArrayInitPlan::default();
        let mut flat = 0usize;
        self.plan_level(
            &head,
            geo,
            &geo.strides,
            0,
            geo.total,
            &mut flat,
            &mut scratch,
        );
        let mut taken = 1usize;
        for item in rest {
            if flat >= geo.total || !item.designators.is_empty() {
                break;
            }
            let one = std::slice::from_ref(item);
            self.plan_level(
                one,
                geo,
                &geo.strides,
                0,
                geo.total,
                &mut flat,
                &mut scratch,
            );
            taken += 1;
        }
        taken
    }

    /// One brace level.  `strides` describes the subobject the level
    /// initializes (a suffix of `geo.strides`), which occupies the elements
    /// `start..limit`; `flat` is the current position.  Initializers past
    /// `limit` are excess and place nothing.
    #[allow(clippy::too_many_arguments)]
    fn plan_level<'a>(
        &self,
        items: &'a [InitializerItem],
        geo: &ArrayGeometry,
        strides: &[usize],
        start: usize,
        limit: usize,
        flat: &mut usize,
        plan: &mut ArrayInitPlan<'a>,
    ) {
        // `{ "ab" }` for a character array: the string initializes it whole.
        if strides.len() == 1 {
            if let Some(init) =
                StringInit::sole_item(items).and_then(|e| StringInit::for_elem_ir(e, geo.base_ty))
            {
                Self::place_row(init, start, limit - start, geo, plan);
                *flat = limit;
                plan.extent = plan.extent.max(limit);
                return;
            }
        }
        for item in items {
            if item.designators.is_empty() {
                if *flat >= limit {
                    continue; // excess initializer
                }
                // The current subobject is the outermost one that starts at
                // `flat`: brace-elided subarrays that are complete are
                // closed, and a braced list opens the element there.
                let rel = *flat - start;
                let k = (0..strides.len())
                    .find(|&k| rel % geo.elems_at(strides, k) == 0)
                    .unwrap_or(strides.len() - 1);
                let pos = *flat;
                *flat = self.place_at(&item.init, geo, strides, k, pos, plan);
                plan.extent = plan.extent.max(*flat);
            } else {
                let mut targets = Vec::new();
                let span = limit - start;
                if !self.designated_targets(
                    &item.designators,
                    geo,
                    strides,
                    start,
                    span,
                    &mut targets,
                ) {
                    continue; // out-of-range designator (diagnosed by the front end)
                }
                let depth = item.designators.len() - 1;
                for pos in targets {
                    *flat = self.place_at(&item.init, geo, strides, depth, pos, plan);
                    plan.extent = plan.extent.max(*flat);
                }
            }
        }
    }

    /// Positions of the elements a designator chain names (several for a
    /// GNU range), relative to the subobject at `start` described by
    /// `strides` and `span` elements long.  False if a designator is out of
    /// range or not an index.
    fn designated_targets(
        &self,
        desigs: &[Designator],
        geo: &ArrayGeometry,
        strides: &[usize],
        start: usize,
        span: usize,
        out: &mut Vec<usize>,
    ) -> bool {
        if desigs.len() > strides.len() {
            return false;
        }
        let mut bases = vec![start];
        for (k, d) in desigs.iter().enumerate() {
            let n = geo.elems_at(strides, k);
            // Elements at this level: the enclosing subobject's size over n.
            let count = if k == 0 {
                span / n
            } else {
                strides[k - 1] / strides[k]
            };
            let (lo, hi) = match d {
                Designator::Index(e) => match self.eval_const_expr_for_designator(e) {
                    Some(i) => (i, i),
                    None => return false,
                },
                Designator::Range(lo, hi) => match (
                    self.eval_const_expr_for_designator(lo),
                    self.eval_const_expr_for_designator(hi),
                ) {
                    (Some(lo), Some(hi)) if lo <= hi => (lo, hi),
                    _ => return false,
                },
                Designator::Field(_) => return false,
            };
            if hi >= count {
                return false;
            }
            bases = bases
                .iter()
                .flat_map(|&b| (lo..=hi).map(move |i| b + i * n))
                .collect();
        }
        out.extend(bases);
        true
    }

    /// Initialize the element at level `k` of `strides` that starts at
    /// `pos` from `init`; returns the position after the initialized
    /// subobject (where the next undesignated initializer goes).
    fn place_at<'a>(
        &self,
        init: &'a Initializer,
        geo: &ArrayGeometry,
        strides: &[usize],
        k: usize,
        pos: usize,
        plan: &mut ArrayInitPlan<'a>,
    ) -> usize {
        let n = geo.elems_at(strides, k);
        let sub = &strides[k + 1..];
        match init {
            Initializer::List(items) if sub.is_empty() => {
                // A braced scalar: `{ x }`, or `{}` for zero.
                plan.clear(pos, 1);
                match items.first().map(|i| &i.init) {
                    Some(Initializer::Expr(e)) => plan.set(pos, ElemInit::Expr(e)),
                    Some(inner @ Initializer::List(_)) => {
                        self.place_at(inner, geo, strides, k, pos, plan);
                    }
                    None => {}
                }
                pos + 1
            }
            Initializer::List(items) => {
                plan.clear(pos, n);
                let mut f = pos;
                self.plan_level(items, geo, sub, pos, pos + n, &mut f, plan);
                pos + n
            }
            Initializer::Expr(e) => {
                if sub.len() == 1 {
                    if let Some(s) = StringInit::for_elem_ir(e, geo.base_ty) {
                        // A string initializes a whole character row.
                        Self::place_row(s, pos, n, geo, plan);
                        return pos + n;
                    }
                }
                if sub.is_empty() {
                    plan.set(pos, ElemInit::Expr(e));
                    pos + 1
                } else {
                    // Brace elision: the expression initializes the first
                    // scalar (or row) of this subarray, and the subarray
                    // stays open for the initializers that follow.
                    self.place_at(init, geo, strides, k + 1, pos, plan)
                }
            }
        }
    }

    /// A string literal initializing the `n` elements at `pos`: its code
    /// units that fit, the terminator if there is room, zeros after.
    fn place_row<'a>(
        s: StringInit<'a>,
        pos: usize,
        n: usize,
        geo: &ArrayGeometry,
        plan: &mut ArrayInitPlan<'a>,
    ) {
        plan.clear(pos, n);
        for (i, u) in s.stored_units(n * geo.elem_size()).into_iter().enumerate() {
            if u != 0 {
                plan.set(pos + i, ElemInit::Unit(u));
            }
        }
    }

    /// The initializer of an array-of-scalars member of type `field_ty`
    /// that a struct walker meets at `items[0]`, designated `rel` relative
    /// to the member (empty when the item is positional or names just the
    /// member), and the number of items it takes.
    pub(super) fn member_array_init<'a>(
        &self,
        rel: &[Designator],
        items: &'a [InitializerItem],
        field_ty: &CType,
        geo: &ArrayGeometry,
    ) -> (MemberArrayInit<'a>, usize) {
        let first = &items[0];
        if rel.is_empty() {
            let whole = match (&first.init, field_ty) {
                (Initializer::List(_), _) => true,
                (Initializer::Expr(e), CType::Array(elem, _)) => {
                    geo.strides.len() == 1 && StringInit::for_elem_ctype(e, elem).is_some()
                }
                _ => false,
            };
            if whole {
                return (MemberArrayInit::Whole(&first.init), 1);
            }
        }
        let n = self.elided_run_len(rel, &first.init, &items[1..], geo);
        let mut run = Vec::with_capacity(n);
        run.push(InitializerItem {
            designators: rel.to_vec(),
            init: first.init.clone(),
        });
        run.extend_from_slice(&items[1..n]);
        (MemberArrayInit::Run(run), n)
    }

    /// Plan a member initializer from [`Self::member_array_init`].
    pub(super) fn plan_member_array<'a>(
        &self,
        init: &'a MemberArrayInit<'_>,
        geo: &ArrayGeometry,
    ) -> ArrayInitPlan<'a> {
        match init {
            MemberArrayInit::Whole(init) => self.plan_array_initializer(init, geo),
            MemberArrayInit::Run(items) => self.plan_array_init(items, geo),
        }
    }

    // -------------------------------------------------------------------------
    // Consumers
    // -------------------------------------------------------------------------

    /// The constant value of an element initializer, converted to the
    /// element type; `None` for an expression that is not an arithmetic
    /// constant (an address: the caller needs a relocation).
    pub(super) fn plan_elem_const(
        &self,
        init: ElemInit<'_>,
        geo: &ArrayGeometry,
    ) -> Option<IrConst> {
        let c = match init {
            ElemInit::Unit(u) => return Some(IrConst::from_i64(i64::from(u), geo.base_ty)),
            ElemInit::Expr(e) => {
                let raw = self.eval_const_expr(e)?;
                if geo.is_bool {
                    raw.bool_normalize()
                } else {
                    let src = self.get_expr_type(e);
                    self.coerce_const_to_type_with_src(raw, geo.base_ty, src)
                }
            }
        };
        Some(Self::maybe_promote_long_double(c, geo.is_long_double))
    }

    /// Whether the plan needs relocations (an element initialized by an
    /// address constant) rather than a plain constant image.
    pub(super) fn plan_needs_relocations(
        &self,
        plan: &ArrayInitPlan<'_>,
        geo: &ArrayGeometry,
    ) -> bool {
        plan.iter()
            .any(|(_, init)| self.plan_elem_const(init, geo).is_none())
    }

    /// Write the plan into a static byte image at `offset` (the image is
    /// zero-filled, which supplies the elements not in the plan).  Elements
    /// that are not arithmetic constants are skipped; callers route plans
    /// with addresses to [`Self::plan_to_global_elems`].
    pub(super) fn plan_to_bytes(
        &self,
        plan: &ArrayInitPlan<'_>,
        geo: &ArrayGeometry,
        bytes: &mut [u8],
        offset: usize,
    ) {
        let esz = geo.elem_size();
        for (flat, init) in plan.iter() {
            let Some(c) = self.plan_elem_const(init, geo) else {
                continue;
            };
            let at = offset + flat * esz;
            let le = c.to_le_bytes();
            let n = le.len().min(esz);
            if let Some(dst) = bytes.get_mut(at..at + n) {
                dst.copy_from_slice(&le[..n]);
            }
        }
    }

    /// The plan as a list of per-element constants (all `geo.total` of
    /// them), for arrays without relocations.
    pub(super) fn plan_to_consts(
        &self,
        plan: &ArrayInitPlan<'_>,
        geo: &ArrayGeometry,
    ) -> Vec<IrConst> {
        let zero = self.typed_zero_const(geo.base_ty, geo.is_long_double);
        let mut out = vec![zero; geo.total];
        for (flat, init) in plan.iter() {
            if let Some(c) = self.plan_elem_const(init, geo) {
                out[flat] = c;
            }
        }
        out
    }

    /// The plan as one `GlobalInit` per element (all `geo.total` of them):
    /// constants, or addresses that become relocations.
    pub(super) fn plan_to_global_elems(
        &mut self,
        plan: &ArrayInitPlan<'_>,
        geo: &ArrayGeometry,
    ) -> Vec<GlobalInit> {
        let esz = geo.elem_size();
        let mut out: Vec<GlobalInit> = (0..geo.total).map(|_| GlobalInit::Zero).collect();
        for (flat, init) in plan.iter() {
            out[flat] = match (self.plan_elem_const(init, geo), init) {
                (Some(c), _) => GlobalInit::Scalar(c),
                (None, ElemInit::Expr(e)) => {
                    let mut parts = Vec::new();
                    self.collect_compound_init_element(
                        &Initializer::Expr(e.clone()),
                        &mut parts,
                        esz,
                    );
                    match parts.into_iter().next() {
                        Some(GlobalInit::Scalar(c)) => GlobalInit::Scalar(c.coerce_to(geo.base_ty)),
                        Some(other) => other,
                        None => GlobalInit::Zero,
                    }
                }
                (None, ElemInit::Unit(_)) => unreachable!("code units are constants"),
            };
        }
        out
    }

    /// Write the plan into a static image with relocations at `offset`:
    /// constants into `bytes` (zero-filled by the caller), addresses into
    /// `ptr_ranges` as (byte offset, relocation).
    pub(super) fn plan_to_image(
        &mut self,
        plan: &ArrayInitPlan<'_>,
        geo: &ArrayGeometry,
        offset: usize,
        bytes: &mut [u8],
        ptr_ranges: &mut Vec<(usize, GlobalInit)>,
    ) {
        let esz = geo.elem_size();
        for (flat, init) in plan.iter() {
            let at = offset + flat * esz;
            let value = match (self.plan_elem_const(init, geo), init) {
                (Some(c), _) => Some(c),
                (None, ElemInit::Expr(e)) => {
                    let mut parts = Vec::new();
                    self.collect_compound_init_element(
                        &Initializer::Expr(e.clone()),
                        &mut parts,
                        esz,
                    );
                    match parts.into_iter().next() {
                        Some(GlobalInit::Scalar(c)) => Some(c.coerce_to(geo.base_ty)),
                        Some(GlobalInit::Zero) | None => None,
                        Some(GlobalInit::GlobalLabelDiff(a, b, _)) => {
                            ptr_ranges.push((at, GlobalInit::GlobalLabelDiff(a, b, esz)));
                            None
                        }
                        Some(reloc) => {
                            ptr_ranges.push((at, reloc));
                            None
                        }
                    }
                }
                (None, ElemInit::Unit(_)) => unreachable!("code units are constants"),
            };
            if let Some(c) = value {
                let le = c.to_le_bytes();
                let n = le.len().min(esz);
                if let Some(dst) = bytes.get_mut(at..at + n) {
                    dst.copy_from_slice(&le[..n]);
                }
            }
        }
    }

    /// Store the plan into a local object at `offset`.  The caller has
    /// zero-filled the object when the plan does not cover every element.
    pub(super) fn emit_plan_to_alloca(
        &mut self,
        plan: &ArrayInitPlan<'_>,
        geo: &ArrayGeometry,
        alloca: Value,
        offset: usize,
    ) {
        let esz = geo.elem_size();
        for (flat, init) in plan.in_source_order() {
            let val = match init {
                ElemInit::Unit(u) => Operand::Const(IrConst::from_i64(i64::from(u), geo.base_ty)),
                ElemInit::Expr(e) => {
                    let v = self.lower_expr(e);
                    let src = self.get_expr_type(e);
                    if geo.is_bool {
                        self.emit_bool_normalize_typed(v, src)
                    } else {
                        self.emit_implicit_cast(v, src, geo.base_ty)
                    }
                }
            };
            self.emit_array_element_store(alloca, val, offset + flat * esz, geo.base_ty);
        }
    }

    /// Store a planned local array of scalars at `offset` of `alloca`,
    /// zero-filling first what the plan leaves out unless the caller has
    /// (`zeroed`).
    pub(super) fn store_array_plan(
        &mut self,
        plan: &ArrayInitPlan<'_>,
        geo: &ArrayGeometry,
        alloca: Value,
        offset: usize,
        zeroed: bool,
    ) {
        if !zeroed && plan.len() < geo.total {
            self.zero_init_region(alloca, offset, geo.total * geo.elem_size());
        }
        self.emit_plan_to_alloca(plan, geo, alloca, offset);
    }

    /// Plan any initializer of an array of scalars: a braced list, or a
    /// string for a character array.  Anything else initializes nothing.
    pub(super) fn plan_array_initializer<'a>(
        &self,
        init: &'a Initializer,
        geo: &ArrayGeometry,
    ) -> ArrayInitPlan<'a> {
        match init {
            Initializer::List(items) => self.plan_array_init(items, geo),
            Initializer::Expr(e) => match StringInit::for_elem_ir(e, geo.base_ty) {
                Some(s) if geo.strides.len() == 1 => self.plan_array_string(s, geo),
                _ => ArrayInitPlan::default(),
            },
        }
    }
}
