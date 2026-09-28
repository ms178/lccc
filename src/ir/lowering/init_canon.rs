//! Canonical form of struct, union and array-of-aggregate initializers
//! (C11 6.7.9).
//!
//! An initializer list as written may elide braces (p20), carry designator
//! chains (`.in.m[1] = ...`, `[1].a[2] = ...`), continue after a designation
//! with the subobject that follows the designated one *at the designation's
//! depth* (p17), and initialize a subobject more than once: the last
//! initializer wins, and a braced one replaces the whole subobject (p19).
//! Every storage-specific lowering (block-scope stores, static byte images,
//! static images with relocations, compound literals) used to interpret
//! those rules on its own, each differently.  This pass resolves them once,
//! against the object's type, into a canonical list:
//!
//! * a struct/union member is initialized at most once, in member order,
//!   designated by name (`.m = ...`; members of an anonymous struct/union by
//!   their own names), by a braced list, an expression of the member's own
//!   struct/union type, or a scalar expression;
//! * an element of an array of aggregates is initialized at most once, in
//!   index order, `[i]`-designated where elements are skipped;
//! * an array of scalars keeps its initializers as ONE braced list relative
//!   to the array (the runs written for it anywhere in the list, in order),
//!   which the array planner (`array_init_plan`) resolves; a string that
//!   initializes a whole character array stays the string;
//! * an aggregate reset to all zeros by a later `{}` is absent, and excess
//!   initializers are dropped (the front end diagnoses them).
//!
//! Most initializers are canonical as written: [`Lowerer::is_canonical_list`]
//! checks that in one pass without allocating, and such lists are lowered as
//! they are.  Shapes outside this model - flexible array members, vector
//! members met by brace elision, a member overridden inside a struct-typed
//! expression that initialized it - are also lowered as written.

use super::array_init_plan::ArrayGeometry;
use super::lower::Lowerer;
use super::string_init::StringInit;
use crate::common::source::Span;
use crate::common::types::{CType, InitFieldResolution, RcLayout, StructLayout};
use crate::frontend::parser::ast::{Designator, Expr, Initializer, InitializerItem};

/// The resolved initializer of one subobject.
enum Node {
    /// A scalar, or a whole struct/union from an expression of its type.
    Expr(Expr),
    /// A struct or union: one slot per member (a union keeps at most one).
    Record(Vec<Option<Node>>),
    /// An array of aggregates: one slot per element initialized so far.
    Array(Vec<Option<Node>>),
    /// An array of scalars: its initializers relative to the array.
    Scalars(Vec<InitializerItem>),
    /// Kept as written: a braced scalar, or a vector's initializer.
    Raw(Initializer),
}

/// How a subobject type takes initializers.
enum Kind {
    Leaf,
    Scalars(ArrayGeometry),
    Record(RcLayout),
    Array {
        elem: CType,
        len: usize,
    },
    /// Outside the model: see the module documentation.
    Opaque,
}

/// A shape this pass does not model; the list is lowered as written.
struct Unsupported;

/// One aggregate on the path from the braced list's object to the current
/// subobject, and the position within it.
struct Frame {
    kind: Kind,
    idx: usize,
}

impl Frame {
    fn len(&self) -> usize {
        match &self.kind {
            Kind::Record(layout) => layout.fields.len(),
            Kind::Array { len, .. } => *len,
            _ => 0,
        }
    }

    fn is_union(&self) -> bool {
        matches!(&self.kind, Kind::Record(layout) if layout.is_union)
    }
}

fn is_unnamed_bitfield(layout: &StructLayout, idx: usize) -> bool {
    let f = &layout.fields[idx];
    f.name.is_empty() && f.bit_width.is_some()
}

fn is_anonymous_member(layout: &StructLayout, idx: usize) -> bool {
    let f = &layout.fields[idx];
    f.name.is_empty() && f.bit_width.is_none() && matches!(f.ty, CType::Struct(_) | CType::Union(_))
}

fn index_expr(i: usize) -> Expr {
    Expr::IntLiteral(i as i64, Span::dummy())
}

fn empty_node(kind: &Kind) -> Option<Node> {
    match kind {
        Kind::Record(layout) => Some(Node::Record(
            (0..layout.fields.len()).map(|_| None).collect(),
        )),
        Kind::Array { .. } => Some(Node::Array(Vec::new())),
        _ => None,
    }
}

impl Lowerer {
    fn init_kind(&self, ty: &CType) -> Kind {
        match ty {
            CType::Struct(_) | CType::Union(_) => match self.get_struct_layout_for_ctype(ty) {
                Some(layout) => Kind::Record(layout),
                None => Kind::Opaque,
            },
            CType::Array(elem, Some(len)) => {
                if let Some(geo) = self.scalar_array_geometry(ty) {
                    Kind::Scalars(geo)
                } else if matches!(self.init_kind(elem), Kind::Opaque) {
                    Kind::Opaque
                } else {
                    Kind::Array {
                        elem: (**elem).clone(),
                        len: *len,
                    }
                }
            }
            CType::Array(_, None) | CType::Vector(..) => Kind::Opaque,
            _ => Kind::Leaf,
        }
    }

    /// Whether `e` initializes a whole struct/union of type `ty` (p13).
    fn expr_initializes_record(&self, e: &Expr, ty: &CType) -> bool {
        match (self.get_expr_ctype(e), ty) {
            (Some(CType::Struct(a)), CType::Struct(b))
            | (Some(CType::Union(a)), CType::Union(b)) => a == *b,
            _ => false,
        }
    }

    /// The canonical form of the braced initializer `items` of an object of
    /// type `ty` (an array without a bound gets `len` elements), or `None`
    /// when the list is canonical already or outside the model: the caller
    /// then lowers `items` as they are.
    pub(super) fn canonical_init_items(
        &self,
        items: &[InitializerItem],
        ty: &CType,
        len: Option<usize>,
    ) -> Option<Vec<InitializerItem>> {
        let ty = match (ty, len) {
            (CType::Array(elem, None), Some(n)) => CType::Array(elem.clone(), Some(n)),
            _ => ty.clone(),
        };
        let kind = self.init_kind(&ty);
        if !matches!(kind, Kind::Record(_) | Kind::Array { .. })
            || self.is_canonical_list(items, &kind)
        {
            return None;
        }
        let node = self.build_braced(items, kind).ok()?;
        match self.emit_node(node, &ty).ok()? {
            Initializer::List(items) => Some(items),
            Initializer::Expr(_) => None,
        }
    }

    // -------------------------------------------------------------------------
    // Fast path
    // -------------------------------------------------------------------------

    /// Whether the braced list `items` for an aggregate of kind `kind` is in
    /// canonical form.
    fn is_canonical_list(&self, items: &[InitializerItem], kind: &Kind) -> bool {
        match kind {
            Kind::Record(layout) => {
                if layout.is_union && items.len() > 1 {
                    return false;
                }
                let mut next = 0usize;
                for item in items {
                    let idx = match item.designators.as_slice() {
                        [] => {
                            while next < layout.fields.len() && is_unnamed_bitfield(layout, next) {
                                next += 1;
                            }
                            next
                        }
                        [Designator::Field(name)] => {
                            match layout.fields.iter().position(|f| f.name == *name) {
                                Some(idx) if idx >= next => idx,
                                _ => return false,
                            }
                        }
                        _ => return false,
                    };
                    if idx >= layout.fields.len() || is_anonymous_member(layout, idx) {
                        return false;
                    }
                    if !self.is_canonical_member(&item.init, &layout.fields[idx].ty) {
                        return false;
                    }
                    next = idx + 1;
                }
                true
            }
            Kind::Array { elem, len } => {
                let mut next = 0usize;
                for item in items {
                    let idx = match item.designators.as_slice() {
                        [] => next,
                        [Designator::Index(e)] => match self.eval_const_expr_for_designator(e) {
                            Some(idx) if idx >= next => idx,
                            _ => return false,
                        },
                        _ => return false,
                    };
                    if idx >= *len || !self.is_canonical_member(&item.init, elem) {
                        return false;
                    }
                    next = idx + 1;
                }
                true
            }
            _ => false,
        }
    }

    /// Whether `init`, the initializer of a subobject of type `ty` within a
    /// canonical list, is itself canonical: no brace elision into it.
    fn is_canonical_member(&self, init: &Initializer, ty: &CType) -> bool {
        match (self.init_kind(ty), init) {
            (Kind::Leaf, _) => true,
            (Kind::Scalars(_), Initializer::List(_)) => true,
            (Kind::Scalars(geo), Initializer::Expr(e)) => match ty {
                CType::Array(elem, _) => {
                    geo.strides.len() == 1 && StringInit::for_elem_ctype(e, elem).is_some()
                }
                _ => false,
            },
            (kind @ (Kind::Record(_) | Kind::Array { .. }), Initializer::List(sub)) => {
                self.is_canonical_list(sub, &kind)
            }
            (Kind::Record(_), Initializer::Expr(e)) => self.expr_initializes_record(e, ty),
            (Kind::Array { .. }, Initializer::Expr(_)) => false,
            // Flexible array members and vectors keep the lowerings' own
            // handling, whatever their form.
            (Kind::Opaque, _) => true,
        }
    }

    // -------------------------------------------------------------------------
    // Resolution
    // -------------------------------------------------------------------------

    /// Resolve the braced list `items` of an aggregate of kind `kind`.
    fn build_braced(&self, items: &[InitializerItem], kind: Kind) -> Result<Node, Unsupported> {
        let mut root = empty_node(&kind).ok_or(Unsupported)?;
        let mut frames = vec![Frame { kind, idx: 0 }];
        let mut i = 0usize;
        while i < items.len() {
            let item = &items[i];
            if let Some(expanded) = self.expand_aggregate_range(item, &frames[0])? {
                // `[lo ... hi] = x` at an array-of-aggregates level: each
                // index in turn; the next initializer follows `hi`.
                for one in &expanded {
                    let member = self.designate(&mut root, &mut frames, &one.designators)?;
                    self.place(&mut root, &mut frames, one, member, &[])?;
                }
                i += 1;
                continue;
            }
            let member = if item.designators.is_empty() {
                None
            } else {
                self.designate(&mut root, &mut frames, &item.designators)?
            };
            i += self.place(&mut root, &mut frames, item, member, &items[i + 1..])?;
        }
        Ok(root)
    }

    /// An item whose designation crosses a GNU range at an array-of-
    /// aggregates level, expanded into one item per index (`None` for items
    /// without such a range; a range inside an array of scalars is the
    /// planner's).
    fn expand_aggregate_range(
        &self,
        item: &InitializerItem,
        list_frame: &Frame,
    ) -> Result<Option<Vec<InitializerItem>>, Unsupported> {
        let Some(pos) = item
            .designators
            .iter()
            .position(|d| matches!(d, Designator::Range(..)))
        else {
            return Ok(None);
        };
        // Is the range at an array-of-aggregates level?  Walk the types.
        let mut walked: Option<Kind> = None;
        for d in &item.designators[..pos] {
            let kind = walked.as_ref().unwrap_or(&list_frame.kind);
            let sub = match (kind, d) {
                (Kind::Record(layout), Designator::Field(name)) => {
                    match layout.fields.iter().find(|f| f.name == *name) {
                        Some(f) => f.ty.clone(),
                        None => return Err(Unsupported),
                    }
                }
                (Kind::Array { elem, .. }, Designator::Index(_)) => elem.clone(),
                (Kind::Scalars(_), _) => return Ok(None),
                _ => return Err(Unsupported),
            };
            walked = Some(self.init_kind(&sub));
        }
        match walked.as_ref().unwrap_or(&list_frame.kind) {
            Kind::Array { .. } => {}
            Kind::Scalars(_) => return Ok(None),
            _ => return Err(Unsupported),
        }
        let Designator::Range(lo, hi) = &item.designators[pos] else {
            unreachable!("found above")
        };
        let (Some(lo), Some(hi)) = (
            self.eval_const_expr_for_designator(lo),
            self.eval_const_expr_for_designator(hi),
        ) else {
            return Err(Unsupported);
        };
        if lo > hi {
            return Err(Unsupported);
        }
        Ok(Some(
            (lo..=hi)
                .map(|k| {
                    let mut designators = item.designators.clone();
                    designators[pos] = Designator::Index(index_expr(k));
                    InitializerItem {
                        designators,
                        init: item.init.clone(),
                    }
                })
                .collect(),
        ))
    }

    /// Follow the designation `desigs` from the braced list's object,
    /// leaving the frames at the designated subobject.  When the designation
    /// enters an array of scalars, returns the part relative to that array
    /// (the planner resolves it).
    fn designate<'d>(
        &self,
        root: &mut Node,
        frames: &mut Vec<Frame>,
        desigs: &'d [Designator],
    ) -> Result<Option<&'d [Designator]>, Unsupported> {
        frames.truncate(1);
        let mut k = 0usize;
        loop {
            let top = frames.last_mut().expect("the list's own frame");
            match (&top.kind, &desigs[k]) {
                (Kind::Record(layout), Designator::Field(name)) => {
                    let resolution = layout.resolve_init_field(
                        Some(name),
                        0,
                        &*self.types.borrow_struct_layouts(),
                    );
                    match resolution {
                        Some(InitFieldResolution::Direct(idx)) => top.idx = idx,
                        Some(InitFieldResolution::AnonymousMember { anon_field_idx, .. }) => {
                            // Enter the anonymous member and look the name up
                            // again there (it may be nested further).
                            top.idx = anon_field_idx;
                            self.enter(root, frames)?;
                            continue;
                        }
                        None => return Err(Unsupported),
                    }
                }
                (Kind::Array { len, .. }, Designator::Index(e)) => {
                    let len = *len;
                    let idx = self.eval_const_expr_for_designator(e).ok_or(Unsupported)?;
                    if idx >= len {
                        return Err(Unsupported);
                    }
                    top.idx = idx;
                }
                _ => return Err(Unsupported),
            }
            k += 1;
            if k == desigs.len() {
                return Ok(None);
            }
            let sub_ty = Self::subobject_ty(frames.last().expect("frame"));
            match self.init_kind(&sub_ty) {
                Kind::Record(_) | Kind::Array { .. } => self.enter(root, frames)?,
                Kind::Scalars(_) => return Ok(Some(&desigs[k..])),
                Kind::Leaf | Kind::Opaque => return Err(Unsupported),
            }
        }
    }

    fn subobject_ty(frame: &Frame) -> CType {
        match &frame.kind {
            Kind::Record(layout) => layout.fields[frame.idx].ty.clone(),
            Kind::Array { elem, .. } => elem.clone(),
            _ => unreachable!("frames are aggregates"),
        }
    }

    /// The node of the aggregate the top frame describes.
    fn parent_node<'n>(root: &'n mut Node, frames: &[Frame]) -> &'n mut Node {
        let mut node = root;
        for f in &frames[..frames.len() - 1] {
            node = Self::child_slot(node, f.idx)
                .as_mut()
                .expect("an entered subobject has a node");
        }
        node
    }

    /// The slot of the current subobject (the top frame's position).
    fn current_slot<'n>(root: &'n mut Node, frames: &[Frame]) -> &'n mut Option<Node> {
        let idx = frames.last().expect("frame").idx;
        Self::child_slot(Self::parent_node(root, frames), idx)
    }

    fn child_slot(node: &mut Node, idx: usize) -> &mut Option<Node> {
        match node {
            Node::Record(fields) => &mut fields[idx],
            Node::Array(elems) => {
                if elems.len() <= idx {
                    elems.resize_with(idx + 1, || None);
                }
                &mut elems[idx]
            }
            _ => unreachable!("only aggregates have children"),
        }
    }

    /// Store `node` as the current subobject; a union keeps only its
    /// latest-initialized member.
    fn store(root: &mut Node, frames: &[Frame], node: Option<Node>) {
        if frames.last().expect("frame").is_union() {
            if let Node::Record(fields) = Self::parent_node(root, frames) {
                fields.iter_mut().for_each(|slot| *slot = None);
            }
        }
        *Self::current_slot(root, frames) = node;
    }

    /// Make the current subobject (an aggregate) the new top frame, giving
    /// it a node to collect its members into.  Members it already has are
    /// kept: brace elision and designators refine a subobject, only a braced
    /// initializer replaces it.
    fn enter(&self, root: &mut Node, frames: &mut Vec<Frame>) -> Result<(), Unsupported> {
        let sub_ty = Self::subobject_ty(frames.last().expect("frame"));
        let kind = self.init_kind(&sub_ty);
        let fits = match (&kind, &*Self::current_slot(root, frames)) {
            (Kind::Record(_), Some(Node::Record(_)))
            | (Kind::Array { .. }, Some(Node::Array(_))) => true,
            // A member overridden inside the expression that initialized
            // the whole struct: not modelled.
            (_, Some(Node::Expr(_))) => return Err(Unsupported),
            _ => false,
        };
        let node = if fits {
            Self::current_slot(root, frames).take()
        } else {
            Some(empty_node(&kind).ok_or(Unsupported)?)
        };
        // For a union, entering another member discards the previous one.
        Self::store(root, frames, node);
        frames.push(Frame { kind, idx: 0 });
        Ok(())
    }

    /// Move past the current subobject.
    fn advance(frame: &mut Frame) {
        if frame.is_union() {
            // A union is complete once a member is initialized.
            frame.idx = frame.len();
        } else {
            frame.idx += 1;
        }
    }

    /// Initialize the current subobject (after `designate`, or the next one
    /// in order) from `item`, descending through elided braces; `member` is
    /// the designation relative to an array of scalars that the designation
    /// entered, and `rest` the items after `item` (an elided run into an
    /// array of scalars takes some of them).  Returns the items consumed.
    fn place(
        &self,
        root: &mut Node,
        frames: &mut Vec<Frame>,
        item: &InitializerItem,
        member: Option<&[Designator]>,
        rest: &[InitializerItem],
    ) -> Result<usize, Unsupported> {
        let positional = member.is_none() && item.designators.is_empty();
        loop {
            // Find the current subobject: skip unnamed bit-fields, and close
            // complete aggregates (never the list's own object).
            {
                let top = frames.last_mut().expect("frame");
                if positional {
                    if let Kind::Record(layout) = &top.kind {
                        while top.idx < layout.fields.len() && is_unnamed_bitfield(layout, top.idx)
                        {
                            top.idx += 1;
                        }
                    }
                }
                if top.idx >= top.len() {
                    if frames.len() == 1 {
                        // Excess initializer (diagnosed by the front end).
                        return Ok(1);
                    }
                    frames.pop();
                    Self::advance(frames.last_mut().expect("frame"));
                    continue;
                }
            }
            let sub_ty = Self::subobject_ty(frames.last().expect("frame"));
            let kind = self.init_kind(&sub_ty);
            if let Some(rel) = member {
                // `.a[i]... = x, y, ...` into an array of scalars.
                let Kind::Scalars(geo) = kind else {
                    unreachable!("designate stops at arrays of scalars")
                };
                return self.place_scalar_run(root, frames, &sub_ty, &geo, rel, item, rest);
            }
            match (&item.init, kind) {
                (Initializer::List(sub), kind @ (Kind::Record(_) | Kind::Array { .. })) => {
                    let node = self.build_braced(sub, kind)?;
                    Self::store(root, frames, Some(node));
                }
                (Initializer::List(sub), Kind::Scalars(_)) => {
                    Self::store(root, frames, Some(Node::Scalars(sub.clone())));
                }
                (Initializer::List(sub), Kind::Leaf) => {
                    // `{}` resets a scalar to zero; `{ x }` is kept as written.
                    let node = (!sub.is_empty()).then(|| Node::Raw(item.init.clone()));
                    Self::store(root, frames, node);
                }
                (Initializer::Expr(e), Kind::Leaf) => {
                    Self::store(root, frames, Some(Node::Expr(e.clone())));
                }
                (Initializer::Expr(e), Kind::Scalars(geo)) => {
                    let whole_string = match &sub_ty {
                        CType::Array(elem, _) => {
                            geo.strides.len() == 1 && StringInit::for_elem_ctype(e, elem).is_some()
                        }
                        _ => false,
                    };
                    if !whole_string {
                        return self.place_scalar_run(root, frames, &sub_ty, &geo, &[], item, rest);
                    }
                    let string = InitializerItem {
                        designators: Vec::new(),
                        init: item.init.clone(),
                    };
                    Self::store(root, frames, Some(Node::Scalars(vec![string])));
                }
                (Initializer::Expr(e), Kind::Record(_))
                    if self.expr_initializes_record(e, &sub_ty) =>
                {
                    Self::store(root, frames, Some(Node::Expr(e.clone())));
                }
                (Initializer::Expr(_), Kind::Record(_) | Kind::Array { .. }) => {
                    // Brace elision: the expression initializes the first
                    // scalar inside this aggregate (p20).
                    self.enter(root, frames)?;
                    continue;
                }
                (init, Kind::Opaque) => {
                    // A vector member takes a braced list or a vector value as
                    // written; flexible array members are not modelled.
                    let whole = match init {
                        Initializer::List(_) => true,
                        Initializer::Expr(e) => {
                            matches!(self.get_expr_ctype(e), Some(CType::Vector(..)))
                        }
                    };
                    if !matches!(sub_ty, CType::Vector(..)) || !whole {
                        return Err(Unsupported);
                    }
                    Self::store(root, frames, Some(Node::Raw(item.init.clone())));
                }
            }
            Self::advance(frames.last_mut().expect("frame"));
            return Ok(1);
        }
    }

    /// Append a brace-elided run (`item` designated `rel` relative to the
    /// array, then as many of `rest` as the planner takes) to the current
    /// array-of-scalars subobject, and move past it.
    #[allow(clippy::too_many_arguments)]
    fn place_scalar_run(
        &self,
        root: &mut Node,
        frames: &mut [Frame],
        sub_ty: &CType,
        geo: &ArrayGeometry,
        rel: &[Designator],
        item: &InitializerItem,
        rest: &[InitializerItem],
    ) -> Result<usize, Unsupported> {
        let taken = self.elided_run_len(rel, &item.init, rest, geo);
        let mut list = match Self::current_slot(root, frames).take() {
            Some(Node::Scalars(list)) => list,
            _ => Vec::new(),
        };
        // A whole-array string followed by element initializers: spell the
        // string out element by element so that the later ones override.
        if let ([only], CType::Array(elem, _)) = (list.as_slice(), sub_ty) {
            if let (true, Initializer::Expr(e)) = (only.designators.is_empty(), &only.init) {
                if geo.strides.len() == 1 {
                    if let Some(s) = StringInit::for_elem_ctype(e, elem) {
                        list = s
                            .stored_units(geo.total * geo.elem_size())
                            .into_iter()
                            .map(|u| InitializerItem {
                                designators: Vec::new(),
                                init: Initializer::Expr(Expr::IntLiteral(
                                    i64::from(u),
                                    Span::dummy(),
                                )),
                            })
                            .collect();
                    }
                }
            }
        }
        // The runs are concatenated into one list, so a run names its start
        // explicitly (a positional run starts at element 0 of the array).
        let designators = if rel.is_empty() && !list.is_empty() {
            vec![Designator::Index(index_expr(0))]
        } else {
            rel.to_vec()
        };
        list.push(InitializerItem {
            designators,
            init: item.init.clone(),
        });
        list.extend(rest[..taken - 1].iter().cloned());
        // Through `store`: in a union this member replaces the others.
        Self::store(root, frames, Some(Node::Scalars(list)));
        Self::advance(frames.last_mut().expect("frame"));
        Ok(taken)
    }

    // -------------------------------------------------------------------------
    // Emission
    // -------------------------------------------------------------------------

    fn emit_node(&self, node: Node, ty: &CType) -> Result<Initializer, Unsupported> {
        Ok(match node {
            Node::Expr(e) => Initializer::Expr(e),
            Node::Raw(init) => init,
            Node::Scalars(mut list) => {
                // A whole-array string stays a string.
                let sole_string = match (list.as_slice(), ty) {
                    ([only], CType::Array(elem, _)) if only.designators.is_empty() => {
                        matches!(&only.init, Initializer::Expr(e)
                            if StringInit::for_elem_ctype(e, elem).is_some())
                    }
                    _ => false,
                };
                if sole_string {
                    list.pop().expect("one item").init
                } else {
                    Initializer::List(list)
                }
            }
            Node::Record(fields) => {
                let Kind::Record(layout) = self.init_kind(ty) else {
                    return Err(Unsupported);
                };
                let mut out = Vec::new();
                self.emit_record_fields(fields, &layout, &mut out)?;
                Initializer::List(out)
            }
            Node::Array(elems) => {
                let CType::Array(elem, _) = ty else {
                    return Err(Unsupported);
                };
                let mut out = Vec::new();
                let mut next = 0usize;
                for (i, slot) in elems.into_iter().enumerate() {
                    let Some(node) = slot else { continue };
                    if Self::is_empty_aggregate(&node) {
                        continue;
                    }
                    let designators = if i == next {
                        Vec::new()
                    } else {
                        vec![Designator::Index(index_expr(i))]
                    };
                    out.push(InitializerItem {
                        designators,
                        init: self.emit_node(node, elem)?,
                    });
                    next = i + 1;
                }
                Initializer::List(out)
            }
        })
    }

    /// An aggregate initialized to all zeros (`{}`, or overridden so): it
    /// adds nothing to the zero-filled object.
    fn is_empty_aggregate(node: &Node) -> bool {
        match node {
            Node::Record(slots) | Node::Array(slots) => slots
                .iter()
                .all(|s| s.as_ref().is_none_or(Self::is_empty_aggregate)),
            Node::Scalars(items) => items.is_empty(),
            Node::Expr(_) | Node::Raw(_) => false,
        }
    }

    /// The members of a record, designated by name; the members of an
    /// anonymous struct/union member are named directly (their names
    /// resolve through it).
    fn emit_record_fields(
        &self,
        fields: Vec<Option<Node>>,
        layout: &StructLayout,
        out: &mut Vec<InitializerItem>,
    ) -> Result<(), Unsupported> {
        for (idx, slot) in fields.into_iter().enumerate() {
            let Some(node) = slot else { continue };
            let field = &layout.fields[idx];
            if is_anonymous_member(layout, idx) {
                match (node, self.init_kind(&field.ty)) {
                    (Node::Record(inner), Kind::Record(inner_layout)) => {
                        self.emit_record_fields(inner, &inner_layout, out)?
                    }
                    _ => return Err(Unsupported),
                }
                continue;
            }
            if Self::is_empty_aggregate(&node) {
                continue;
            }
            out.push(InitializerItem {
                designators: vec![Designator::Field(field.name.clone())],
                init: self.emit_node(node, &field.ty)?,
            });
        }
        Ok(())
    }
}
