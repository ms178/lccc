//! Loop-carried store-to-load forwarding (distance-1 predictive commoning).
//!
//! Recurrence loops of the form
//!
//! ```c
//! for (i = lo; i <= hi; i++) {
//!     a[i] = a[i - 1] + p;          // prefix sums, IIR filters, DP tables,
//!     ...                           // Adler rolling sums, LCG tables, ...
//! }
//! ```
//!
//! store `a[i]` in iteration n and load `a[i-1]` in iteration n+1 through the
//! **same address**. Without this pass the load is a store-to-load-forwarding
//! round trip through the L1D (≈5 cycles on Raptor Lake, on the loop's
//! critical path); GCC keeps the value in a register (`tree-predcom`,
//! distance 1) and the loop runs at 1 cycle/iteration. lccc measured 2.19×
//! slower than GCC on `tls_seg_access` for exactly this reason.
//!
//! The pass turns the load into a header phi:
//!
//! ```text
//! preheader:  q0 = load  addr_L(0)            // first iteration's value
//! header:     q  = phi(q0 [preheader], v [latch])
//! body:       store v -> addr_S(n)            // unchanged
//!             (load addr_L(n) deleted; all uses renamed to q)
//! ```
//!
//! # Address model
//!
//! Every pointer expression that reaches a loop load/store is normalised to
//! an affine chain of recurrences over the loop's iteration number `n`:
//!
//! ```text
//! addr(n) = root + Σ c_k·inv_k + stride·n + base
//! ```
//!
//! where `root` is a loop-invariant pointer (`GlobalAddr`, `Alloca`, or an
//! opaque invariant pointer), `inv_k` are loop-invariant pointer-width
//! integers, and `stride`/`base` are compile-time constants. Header phis with
//! a constant step (`i = phi(init, i + c)`, marching pointers
//! `p = phi(p0, p + 8)`), `GetElementPtr`, `Add`/`Sub`/`Shl`/`Mul` by
//! constants, `Copy` and same-width `Cast` are folded; everything else is
//! opaque and fails the candidate closed. Only pointer-width integer
//! arithmetic is modelled, so wrapping narrow induction variables can never
//! be mistaken for linear addresses (`iv_widen` runs first and widens the
//! common `int i` shapes).
//!
//! # Legality (all required; every check fails closed)
//!
//! 1. Natural loop with a unique preheader and a single latch.
//! 2. `S` (store) and `L` (load) are non-volatile, default address space,
//!    identical `IrType`, `S` dominates the latch (executes every iteration).
//! 3. `lin(L.ptr)` and `lin(S.ptr)` have the same root object, identical
//!    invariant terms, identical non-zero stride `a` with `|a| ≥ size`, and
//!    `base_S = base_L + a` — i.e. `addr_L(n+1) == addr_S(n)` and the two
//!    accesses in the same iteration never overlap.
//! 4. No instruction in the loop has unmodelled memory effects (calls,
//!    memcpy, intrinsics, atomics, inline asm, va_*, stack ops ...).
//! 5. Every other store `T` in the loop is provably disjoint from `L` for
//!    all iteration pairs: distinct object roots (two different globals, two
//!    different allocas, or global vs alloca), or the same root with the
//!    same stride and invariant terms whose byte window
//!    `(base_T - base_L) mod |a|` never intersects `L`'s window
//!    (interleaved struct fields). Anything else — unknown roots, different
//!    strides — rejects the candidate.
//! 6. The preheader load `addr_L(0)` must be safe to execute even when the
//!    loop body would not run: either the root is a `GlobalAddr`/`Alloca`
//!    of known size with no invariant terms and `[base_L, base_L+size)`
//!    inside the object, or `L`'s block dominates the latch **and** every
//!    exiting block (so `L` executes before any exit whenever the loop is
//!    entered at all).
//! 7. The loop's header does not already carry more than
//!    `MAX_CARRIED_PHIS` values (register-pressure guard; one saved load is
//!    not worth a spill).
//!
//! # Soundness argument
//!
//! Let `M(x)` be the memory at address `x`. `q_0 = M(addr_L(0))` at the
//! preheader; by (5) and (3) no store between the preheader and `L` in
//! iteration 0 touches `addr_L(0)` (`S(0)` writes `addr_L(0)+a`, disjoint by
//! `|a| ≥ size`), so `q_0 == L_0`. For n ≥ 0, `S(n)` writes `v_n` to
//! `addr_S(n) = addr_L(n+1)`; between `S(n)` and `L(n+1)` the only stores are
//! later stores of iteration n and earlier stores of iteration n+1, all of
//! which are either `S` at a different iteration (distinct address, `a ≠ 0`,
//! `|a| ≥ size`) or provably disjoint `T`s (5). Hence `L_{n+1} == v_n ==
//! q_{n+1}`. `q` is defined in the header, which dominates every use of `L`,
//! so renaming is semantics-preserving. The store itself is kept: memory
//! stays observable.
//!
//! Kill switch: `CCC_NO_LOOP_CARRIED_FWD=1`.
//! Explainability: `CCC_DEBUG_LOOP_CARRIED_FWD=1` prints applied rewrites and
//! every rejected (load, store) pair that matched the address shape.

use super::loop_analysis::{self, DominanceChecker, NaturalLoop};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::{target_ptr_size, AddressSpace, IrType};
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BlockId, Instruction, IrBinOp, IrConst, IrFunction, IrModule, Operand, Value,
};

/// Loop-carried phi budget per header (GPR-class). Beyond this the scan
/// allocator is already at its limit on x86-64 (14 allocatable GPRs minus
/// scratch); adding a carried value costs a spill that outweighs one load.
const MAX_CARRIED_PHIS: usize = 10;
/// Recursion fuel for the affine address normaliser.
const LIN_FUEL: u32 = 24;
/// Upper bound on rewrites per function per round (compile-time guard).
const MAX_REWRITES: usize = 64;
/// Fixpoint rounds: a rewrite can expose a chained recurrence
/// (`a[i] = a[i-1] + a[i-2]` needs two rounds).
const MAX_ROUNDS: usize = 3;

#[derive(Clone, Copy)]
struct Env {
    disabled: bool,
    debug: bool,
}

impl Env {
    fn read() -> Self {
        Self {
            disabled: std::env::var_os("CCC_NO_LOOP_CARRIED_FWD").is_some(),
            debug: std::env::var_os("CCC_DEBUG_LOOP_CARRIED_FWD").is_some(),
        }
    }
}

/// Identity of the object a normalised address points into.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Root {
    Global(String),
    Alloca(u32),
    /// Loop-invariant pointer of unknown provenance (parameter, loaded
    /// pointer, ...). Two `Opaque` roots are the same object iff they are
    /// the same SSA value; an `Opaque` root may alias *anything* else.
    Opaque(u32),
}

/// `root + Σ terms + stride·n + base`, terms sorted by value id.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Lin {
    root: Option<Root>,
    terms: Vec<(u32, i64)>,
    stride: i64,
    base: i64,
}

impl Lin {
    fn constant(c: i64) -> Self {
        Lin {
            root: None,
            terms: Vec::new(),
            stride: 0,
            base: c,
        }
    }

    fn of_root(root: Root) -> Self {
        Lin {
            root: Some(root),
            terms: Vec::new(),
            stride: 0,
            base: 0,
        }
    }

    fn add(mut self, other: Lin) -> Option<Lin> {
        if self.root.is_some() && other.root.is_some() {
            return None;
        }
        if self.root.is_none() {
            self.root = other.root;
        }
        self.stride = self.stride.checked_add(other.stride)?;
        self.base = self.base.checked_add(other.base)?;
        for (v, c) in other.terms {
            match self.terms.iter_mut().find(|(w, _)| *w == v) {
                Some(e) => e.1 = e.1.checked_add(c)?,
                None => self.terms.push((v, c)),
            }
        }
        self.terms.retain(|(_, c)| *c != 0);
        self.terms.sort_unstable();
        Some(self)
    }

    fn scale(mut self, k: i64) -> Option<Lin> {
        if self.root.is_some() {
            return None;
        }
        self.stride = self.stride.checked_mul(k)?;
        self.base = self.base.checked_mul(k)?;
        for t in &mut self.terms {
            t.1 = t.1.checked_mul(k)?;
        }
        self.terms.retain(|(_, c)| *c != 0);
        Some(self)
    }

    fn same_shape(&self, other: &Lin) -> bool {
        self.root == other.root && self.terms == other.terms && self.stride == other.stride
    }
}

/// Per-function definition table: value id → (block index, instruction index).
struct Defs {
    at: Vec<Option<(u32, u32)>>,
}

impl Defs {
    fn build(func: &IrFunction, bound: usize) -> Self {
        let mut at = vec![None; bound];
        for (bi, b) in func.blocks.iter().enumerate() {
            for (ii, inst) in b.instructions.iter().enumerate() {
                if let Some(d) = inst.dest() {
                    if (d.0 as usize) < bound {
                        at[d.0 as usize] = Some((bi as u32, ii as u32));
                    }
                }
            }
        }
        Defs { at }
    }

    #[inline]
    fn get(&self, v: Value) -> Option<(usize, usize)> {
        self.at
            .get(v.0 as usize)
            .copied()
            .flatten()
            .map(|(b, i)| (b as usize, i as usize))
    }
}

struct Ctx<'a> {
    func: &'a IrFunction,
    dom: &'a DominanceChecker,
    idom: &'a [usize],
    defs: &'a Defs,
    globals: &'a FxHashMap<String, usize>,
    ptr_ty_size: usize,
    env: Env,
}

struct LoopShape<'a> {
    lp: &'a NaturalLoop,
    header: usize,
    preheader: usize,
    latch: usize,
    preheader_label: BlockId,
    latch_label: BlockId,
    exiting: Vec<usize>,
}

fn const_i64(c: &IrConst) -> Option<i64> {
    match c {
        IrConst::I64(v) => Some(*v),
        IrConst::I32(v) => Some(*v as i64),
        IrConst::I16(v) => Some(*v as i64),
        IrConst::I8(v) => Some(*v as i64),
        _ => None,
    }
}

impl<'a> Ctx<'a> {
    #[inline]
    fn inst(&self, b: usize, i: usize) -> &Instruction {
        &self.func.blocks[b].instructions[i]
    }

    fn in_loop(&self, lp: &NaturalLoop, v: Value) -> bool {
        match self.defs.get(v) {
            Some((b, _)) => lp.contains(b),
            None => false,
        }
    }

    fn is_ptr_width_int(&self, ty: IrType) -> bool {
        ty.is_integer() && ty.size() == self.ptr_ty_size
    }

    /// Classify a loop-invariant value as a pointer root or an integer term.
    fn invariant(&self, v: Value) -> Option<Lin> {
        let mut cur = v;
        for _ in 0..LIN_FUEL {
            // A value without a definition (never happens for well-formed
            // SSA; parameters arrive via `ParamRef`) fails closed.
            let (b, i) = self.defs.get(cur)?;
            match self.inst(b, i) {
                Instruction::GlobalAddr { name, .. } => {
                    return Some(Lin::of_root(Root::Global(name.clone())))
                }
                Instruction::Alloca { dest, .. } => {
                    return Some(Lin::of_root(Root::Alloca(dest.0)))
                }
                Instruction::Copy {
                    src: Operand::Value(s),
                    ..
                } => cur = *s,
                Instruction::Copy {
                    src: Operand::Const(c),
                    ..
                } => return const_i64(c).map(Lin::constant),
                Instruction::GetElementPtr { base, offset, .. } => {
                    let bl = self.invariant(*base)?;
                    bl.root.as_ref()?;
                    let ol = self.invariant_operand(offset)?;
                    if ol.root.is_some() {
                        return None;
                    }
                    return bl.add(ol);
                }
                Instruction::BinOp {
                    op, lhs, rhs, ty, ..
                } if self.is_ptr_width_int(*ty) => {
                    return match op {
                        IrBinOp::Add => {
                            let a = self.invariant_operand(lhs)?;
                            a.add(self.invariant_operand(rhs)?)
                        }
                        IrBinOp::Sub => {
                            let a = self.invariant_operand(lhs)?;
                            a.add(self.invariant_operand(rhs)?.scale(-1)?)
                        }
                        IrBinOp::Shl => {
                            let Operand::Const(c) = rhs else { return None };
                            let k = const_i64(c)?;
                            if !(0..=31).contains(&k) {
                                return None;
                            }
                            self.invariant_operand(lhs)?.scale(1i64 << k)
                        }
                        IrBinOp::Mul => {
                            let (x, c) = match (lhs, rhs) {
                                (x, Operand::Const(c)) => (x, c),
                                (Operand::Const(c), x) => (x, c),
                                _ => return None,
                            };
                            self.invariant_operand(x)?.scale(const_i64(c)?)
                        }
                        // Any other pointer-width integer op: an opaque term.
                        _ => Some(Lin {
                            root: None,
                            terms: vec![(cur.0, 1)],
                            stride: 0,
                            base: 0,
                        }),
                    };
                }
                Instruction::Cast {
                    src: Operand::Value(sv),
                    from_ty,
                    to_ty,
                    ..
                } if from_ty.size() == to_ty.size()
                    && (to_ty.is_integer() || *to_ty == IrType::Ptr)
                    && (from_ty.is_integer() || *from_ty == IrType::Ptr) =>
                {
                    cur = *sv
                }
                inst => {
                    return match inst.result_type() {
                        Some(IrType::Ptr) => Some(Lin::of_root(Root::Opaque(cur.0))),
                        Some(ty) if self.is_ptr_width_int(ty) => Some(Lin {
                            root: None,
                            terms: vec![(cur.0, 1)],
                            stride: 0,
                            base: 0,
                        }),
                        _ => None,
                    }
                }
            }
        }
        None
    }

    fn invariant_operand(&self, op: &Operand) -> Option<Lin> {
        match op {
            Operand::Const(c) => const_i64(c).map(Lin::constant),
            Operand::Value(v) => self.invariant(*v),
        }
    }

    /// Latch-side step of a header phi: `next == phi + c` (pointer-width
    /// integer add/sub of a constant, or a GEP by a constant byte offset).
    /// Copies are looked through.
    fn phi_step(&self, lp: &NaturalLoop, phi: Value, next: &Operand) -> Option<i64> {
        let Operand::Value(mut cur) = next else {
            return None;
        };
        for _ in 0..LIN_FUEL {
            let (b, i) = self.defs.get(cur)?;
            if !lp.contains(b) {
                return None;
            }
            match self.inst(b, i) {
                Instruction::Copy {
                    src: Operand::Value(s),
                    ..
                } => cur = *s,
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ty,
                    ..
                } if self.is_ptr_width_int(*ty) => {
                    return match (lhs, rhs) {
                        (Operand::Value(v), Operand::Const(c)) if *v == phi => const_i64(c),
                        (Operand::Const(c), Operand::Value(v)) if *v == phi => const_i64(c),
                        _ => None,
                    }
                }
                Instruction::BinOp {
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(v),
                    rhs: Operand::Const(c),
                    ty,
                    ..
                } if *v == phi && self.is_ptr_width_int(*ty) => {
                    return const_i64(c)?.checked_neg()
                }
                Instruction::GetElementPtr {
                    base,
                    offset: Operand::Const(c),
                    ..
                } if *base == phi => return const_i64(c),
                _ => return None,
            }
        }
        None
    }

    /// Affine normalisation of an operand relative to `sh`'s loop.
    fn lin(&self, sh: &LoopShape, op: &Operand, fuel: u32) -> Option<Lin> {
        if fuel == 0 {
            return None;
        }
        let v = match op {
            Operand::Const(c) => return const_i64(c).map(Lin::constant),
            Operand::Value(v) => *v,
        };
        if !self.in_loop(sh.lp, v) {
            return self.invariant(v);
        }
        let (b, i) = self.defs.get(v)?;
        match self.inst(b, i) {
            Instruction::Phi { incoming, ty, .. } => {
                if b != sh.header || incoming.len() != 2 {
                    return None;
                }
                if !(*ty == IrType::Ptr || self.is_ptr_width_int(*ty)) {
                    return None;
                }
                let (init, next) = match (&incoming[0], &incoming[1]) {
                    ((a, la), (n, ln)) if *la == sh.preheader_label && *ln == sh.latch_label => {
                        (a, n)
                    }
                    ((n, ln), (a, la)) if *la == sh.preheader_label && *ln == sh.latch_label => {
                        (a, n)
                    }
                    _ => return None,
                };
                let step = self.phi_step(sh.lp, v, next)?;
                let mut l = self.lin(sh, init, fuel - 1)?;
                l.stride = l.stride.checked_add(step)?;
                Some(l)
            }
            Instruction::Copy { src, .. } => self.lin(sh, src, fuel - 1),
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } => {
                // Same-width int↔int / int↔ptr reinterpretations are linear;
                // narrowing or widening is not (wrap / sign extension).
                let same = from_ty.size() == to_ty.size()
                    && (to_ty.is_integer() || *to_ty == IrType::Ptr)
                    && (from_ty.is_integer() || *from_ty == IrType::Ptr);
                if same {
                    self.lin(sh, src, fuel - 1)
                } else {
                    None
                }
            }
            Instruction::GetElementPtr { base, offset, .. } => {
                let bl = self.lin(sh, &Operand::Value(*base), fuel - 1)?;
                bl.root.as_ref()?;
                let ol = self.lin(sh, offset, fuel - 1)?;
                if ol.root.is_some() {
                    return None;
                }
                bl.add(ol)
            }
            Instruction::BinOp {
                op, lhs, rhs, ty, ..
            } => {
                if !self.is_ptr_width_int(*ty) {
                    return None;
                }
                match op {
                    IrBinOp::Add => {
                        let a = self.lin(sh, lhs, fuel - 1)?;
                        let b = self.lin(sh, rhs, fuel - 1)?;
                        a.add(b)
                    }
                    IrBinOp::Sub => {
                        let a = self.lin(sh, lhs, fuel - 1)?;
                        let b = self.lin(sh, rhs, fuel - 1)?.scale(-1)?;
                        a.add(b)
                    }
                    IrBinOp::Shl => {
                        let Operand::Const(c) = rhs else { return None };
                        let k = const_i64(c)?;
                        if !(0..=31).contains(&k) {
                            return None;
                        }
                        self.lin(sh, lhs, fuel - 1)?.scale(1i64 << k)
                    }
                    IrBinOp::Mul => {
                        let (x, c) = match (lhs, rhs) {
                            (x, Operand::Const(c)) => (x, c),
                            (Operand::Const(c), x) => (x, c),
                            _ => return None,
                        };
                        let k = const_i64(c)?;
                        self.lin(sh, x, fuel - 1)?.scale(k)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Is a preheader load of `addr_L(0)` guaranteed not to fault?
    fn preheader_load_safe(
        &self,
        sh: &LoopShape,
        l: &Lin,
        size: usize,
        load_block: usize,
    ) -> bool {
        // (a) in-bounds access of an object with statically known size.
        if l.terms.is_empty() {
            let obj_size = match &l.root {
                Some(Root::Global(name)) => self.globals.get(name).copied(),
                Some(Root::Alloca(id)) => match self.defs.get(Value(*id)) {
                    Some((b, i)) => match self.inst(b, i) {
                        Instruction::Alloca { size, .. } => Some(*size),
                        _ => None,
                    },
                    None => None,
                },
                _ => None,
            };
            if let Some(obj) = obj_size {
                if obj > 0 && l.base >= 0 && (l.base as u128) + (size as u128) <= obj as u128 {
                    return true;
                }
            }
        }
        // (b) the load executes before any exit whenever the loop is entered.
        if self.dom.dominates(load_block, sh.latch)
            && sh.exiting.iter().all(|&e| self.dom.dominates(load_block, e))
        {
            return true;
        }
        // (c) some non-volatile access of the same address (same root, same
        // invariant terms, same byte offset, at least as wide) already
        // executed on every path to the preheader: `a[n] = k; for (i = n-1;
        // ...) a[i] = f(a[i+1])`. Walk the preheader and its dominators.
        if self.dominating_access_covers(sh, l, size) {
            return true;
        }
        // (d) the header's exit test is a compare of a header phi with a
        // constant init against a constant, and it passes on the first
        // iteration: the body (and the load, if it dominates the latch)
        // executes at least once whenever the preheader does.
        self.dom.dominates(load_block, sh.latch) && self.first_trip_proven(sh)
    }

    fn dominating_access_covers(&self, sh: &LoopShape, l: &Lin, size: usize) -> bool {
        const BUDGET: usize = 256;
        let mut seen = 0usize;
        let mut b = sh.preheader;
        loop {
            for inst in self.func.blocks[b].instructions.iter().rev() {
                seen += 1;
                if seen > BUDGET {
                    return false;
                }
                let (ptr, ty, vol, seg) = match inst {
                    Instruction::Load {
                        ptr,
                        ty,
                        volatile,
                        seg_override,
                        ..
                    }
                    | Instruction::Store {
                        ptr,
                        ty,
                        volatile,
                        seg_override,
                        ..
                    } => (*ptr, *ty, *volatile, *seg_override),
                    _ => continue,
                };
                if vol || seg != AddressSpace::Default || ty.size() < size {
                    continue;
                }
                let Some(al) = self.invariant(ptr) else { continue };
                if al.root == l.root && al.terms == l.terms && al.stride == 0 && al.base == l.base {
                    return true;
                }
            }
            let idom = self.idom[b];
            if idom == b || idom == usize::MAX || idom >= self.func.blocks.len() {
                return false;
            }
            b = idom;
        }
    }

    fn first_trip_proven(&self, sh: &LoopShape) -> bool {
        use crate::ir::reexports::{IrCmpOp, Terminator};
        let header = &self.func.blocks[sh.header];
        let Terminator::CondBranch {
            cond: Operand::Value(cv),
            true_label,
            false_label,
        } = &header.terminator
        else {
            return false;
        };
        let Some((cb, ci)) = self.defs.get(*cv) else { return false };
        if cb != sh.header {
            return false;
        }
        let Instruction::Cmp { op, lhs, rhs, ty, .. } = self.inst(cb, ci) else {
            return false;
        };
        if !ty.is_integer() {
            return false;
        }
        // Resolve each side to its first-iteration constant value.
        let first = |o: &Operand| -> Option<i128> {
            match o {
                Operand::Const(c) => const_i64(c).map(|v| v as i128),
                Operand::Value(v) => {
                    let (b, i) = self.defs.get(*v)?;
                    if b != sh.header {
                        return None;
                    }
                    let Instruction::Phi { incoming, .. } = self.inst(b, i) else {
                        return None;
                    };
                    let (init, _) = incoming.iter().find(|(_, l)| *l == sh.preheader_label)?;
                    match init {
                        Operand::Const(c) => const_i64(c).map(|v| v as i128),
                        _ => None,
                    }
                }
            }
        };
        let (Some(a), Some(b)) = (first(lhs), first(rhs)) else {
            return false;
        };
        let bits = (ty.size() * 8) as u32;
        let mask: i128 = if bits >= 128 { -1 } else { (1i128 << bits) - 1 };
        let (ua, ub) = (a & mask, b & mask);
        let taken = match op {
            IrCmpOp::Eq => a == b,
            IrCmpOp::Ne => a != b,
            IrCmpOp::Slt => a < b,
            IrCmpOp::Sle => a <= b,
            IrCmpOp::Sgt => a > b,
            IrCmpOp::Sge => a >= b,
            IrCmpOp::Ult => ua < ub,
            IrCmpOp::Ule => ua <= ub,
            IrCmpOp::Ugt => ua > ub,
            IrCmpOp::Uge => ua >= ub,
        };
        let label_to_idx = |l: &BlockId| self.func.blocks.iter().position(|bb| bb.label == *l);
        let target = if taken { true_label } else { false_label };
        match label_to_idx(target) {
            Some(t) => sh.lp.contains(t) && t != sh.header,
            None => false,
        }
    }
}

/// Instructions the analysis models precisely. Anything else that could
/// touch memory rejects the loop (default-closed, see `store_load_forward.rs`).
fn is_modelled(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Load { .. }
            | Instruction::Store { .. }
            | Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GlobalAddr { .. }
            | Instruction::Phi { .. }
            | Instruction::Select { .. }
            | Instruction::ParamRef { .. }
            | Instruction::LabelAddr { .. }
    )
}

struct MemOp {
    block: usize,
    idx: usize,
    is_store: bool,
    volatile: bool,
    seg: AddressSpace,
    ty: IrType,
    lin: Option<Lin>,
}

struct Rewrite {
    load_block: usize,
    load_idx: usize,
    load_dest: Value,
    ty: IrType,
    store_val: Operand,
    lin0: Lin,
}

/// Can store `t` ever overlap load address `l` (any iteration pair)?
/// Conservative: `true` unless disjointness is proven.
fn may_overlap(t: &MemOp, l: &Lin, l_size: usize) -> bool {
    let Some(tl) = &t.lin else { return true };
    let t_size = t.ty.size();
    if t_size == 0 {
        return true;
    }
    match (&tl.root, &l.root) {
        (Some(Root::Global(a)), Some(Root::Global(b))) if a != b => return false,
        (Some(Root::Alloca(a)), Some(Root::Alloca(b))) if a != b => return false,
        (Some(Root::Global(_)), Some(Root::Alloca(_)))
        | (Some(Root::Alloca(_)), Some(Root::Global(_))) => return false,
        (Some(Root::Opaque(_)), _) | (_, Some(Root::Opaque(_))) => {
            // An opaque pointer may alias any other object; only the same
            // SSA pointer with the same shape can be reasoned about.
            if tl.root != l.root {
                return true;
            }
        }
        _ => {}
    }
    if !tl.same_shape(l) || l.stride == 0 {
        return true;
    }
    let a = l.stride.unsigned_abs() as i128;
    let d = ((tl.base as i128 - l.base as i128) % a + a) % a; // in [0, a)
    let t_end = d + t_size as i128;
    let l_size = l_size as i128;
    // Within one stride period T occupies [d, d+t_size) and L occupies
    // [0, l_size). They overlap when T starts inside L's window, or when T
    // wraps past the period end back into [0, t_end - a).
    d < l_size || t_end > a
}

fn plan_loop(cx: &Ctx, sh: &LoopShape, out: &mut Vec<Rewrite>) {
    // Register-pressure guard.
    let carried = cx.func.blocks[sh.header]
        .instructions
        .iter()
        .filter(|i| matches!(i, Instruction::Phi { .. }))
        .count();
    if carried > MAX_CARRIED_PHIS {
        return;
    }
    let mut ops: Vec<MemOp> = Vec::new();
    let mut blocks: Vec<usize> = sh.lp.body.iter().copied().collect();
    blocks.sort_unstable();
    for &b in &blocks {
        for (i, inst) in cx.func.blocks[b].instructions.iter().enumerate() {
            if !is_modelled(inst) {
                if cx.env.debug {
                    eprintln!(
                        "[lcfwd] {}: loop@{} rejected: unmodelled memory effect {:?}",
                        cx.func.name,
                        sh.header,
                        std::mem::discriminant(inst)
                    );
                }
                return;
            }
            match inst {
                Instruction::Load {
                    ptr,
                    ty,
                    seg_override,
                    volatile,
                    ..
                } => ops.push(MemOp {
                    block: b,
                    idx: i,
                    is_store: false,
                    volatile: *volatile,
                    seg: *seg_override,
                    ty: *ty,
                    lin: cx.lin(sh, &Operand::Value(*ptr), LIN_FUEL),
                }),
                Instruction::Store {
                    ptr,
                    ty,
                    seg_override,
                    volatile,
                    ..
                } => ops.push(MemOp {
                    block: b,
                    idx: i,
                    is_store: true,
                    volatile: *volatile,
                    seg: *seg_override,
                    ty: *ty,
                    lin: cx.lin(sh, &Operand::Value(*ptr), LIN_FUEL),
                }),
                _ => {}
            }
        }
    }
    if !ops.iter().any(|o| o.is_store) {
        return;
    }
    let mut claimed_loads: FxHashSet<(usize, usize)> = FxHashSet::default();
    for (si, s) in ops.iter().enumerate() {
        if !s.is_store || s.volatile || s.seg != AddressSpace::Default {
            continue;
        }
        let Some(sl) = &s.lin else { continue };
        if sl.root.is_none() || sl.stride == 0 {
            continue;
        }
        if !cx.dom.dominates(s.block, sh.latch) {
            continue;
        }
        let size = s.ty.size();
        if size == 0 || (sl.stride.unsigned_abs() as usize) < size {
            continue;
        }
        for l in ops.iter() {
            if l.is_store || l.volatile || l.seg != AddressSpace::Default || l.ty != s.ty {
                continue;
            }
            if claimed_loads.contains(&(l.block, l.idx)) {
                continue;
            }
            let Some(ll) = &l.lin else { continue };
            if !ll.same_shape(sl) || ll.base.checked_add(sl.stride) != Some(sl.base) {
                continue;
            }
            // Candidate pair: L(n+1) == S(n). Every other store must be
            // provably disjoint from L across all iterations.
            let conflict = ops
                .iter()
                .enumerate()
                .filter(|(ti, t)| *ti != si && t.is_store)
                .find(|(_, t)| may_overlap(t, ll, size));
            if let Some((_, t)) = conflict {
                if cx.env.debug {
                    eprintln!(
                        "[lcfwd] {}: loop@{} pair (load b{}i{}, store b{}i{}) rejected: \
                         store b{}i{} may alias (load {:?}, store {:?})",
                        cx.func.name, sh.header, l.block, l.idx, s.block, s.idx, t.block, t.idx, ll, t.lin
                    );
                }
                continue;
            }
            if !cx.preheader_load_safe(sh, ll, size, l.block) {
                if cx.env.debug {
                    eprintln!(
                        "[lcfwd] {}: loop@{} pair (load b{}i{}, store b{}i{}) rejected: \
                         preheader load not provably safe",
                        cx.func.name, sh.header, l.block, l.idx, s.block, s.idx
                    );
                }
                continue;
            }
            let Instruction::Load { dest, .. } = cx.inst(l.block, l.idx) else {
                continue;
            };
            let Instruction::Store { val, .. } = cx.inst(s.block, s.idx) else {
                continue;
            };
            claimed_loads.insert((l.block, l.idx));
            out.push(Rewrite {
                load_block: l.block,
                load_idx: l.idx,
                load_dest: *dest,
                ty: l.ty,
                store_val: val.clone(),
                lin0: ll.clone(),
            });
            if cx.env.debug {
                eprintln!(
                    "[lcfwd] {}: loop@{} forward store b{}i{} -> load b{}i{} ({:?}, stride {}, base {})",
                    cx.func.name, sh.header, s.block, s.idx, l.block, l.idx, l.ty, ll.stride, ll.base
                );
            }
            if out.len() >= MAX_REWRITES {
                return;
            }
            break; // one load per store per round
        }
    }
}

fn alloc_value(func: &mut IrFunction) -> Value {
    let v = Value(func.next_value_id);
    func.next_value_id += 1;
    v
}

fn push_inst(func: &mut IrFunction, block: usize, inst: Instruction) {
    let b = &mut func.blocks[block];
    if !b.source_spans.is_empty() {
        b.source_spans.push(Span::dummy());
    }
    b.instructions.push(inst);
}

fn insert_inst(func: &mut IrFunction, block: usize, at: usize, inst: Instruction) {
    let b = &mut func.blocks[block];
    if !b.source_spans.is_empty() {
        b.source_spans.insert(at, Span::dummy());
    }
    b.instructions.insert(at, inst);
}

fn remove_inst(func: &mut IrFunction, block: usize, at: usize) {
    let b = &mut func.blocks[block];
    if !b.source_spans.is_empty() {
        b.source_spans.remove(at);
    }
    b.instructions.remove(at);
}

/// Build `root + Σ c_k·inv_k + base` at the end of `preheader`.
fn materialise_addr(
    func: &mut IrFunction,
    preheader: usize,
    root: Value,
    lin: &Lin,
    int_ty: IrType,
) -> Value {
    let mut ptr = root;
    if !lin.terms.is_empty() {
        let mut acc: Option<Value> = None;
        for &(v, c) in &lin.terms {
            let term = if c == 1 {
                Value(v)
            } else {
                let t = alloc_value(func);
                push_inst(
                    func,
                    preheader,
                    Instruction::BinOp {
                        dest: t,
                        op: IrBinOp::Mul,
                        lhs: Operand::Value(Value(v)),
                        rhs: Operand::Const(IrConst::I64(c)),
                        ty: int_ty,
                    },
                );
                t
            };
            acc = Some(match acc {
                None => term,
                Some(a) => {
                    let s = alloc_value(func);
                    push_inst(
                        func,
                        preheader,
                        Instruction::BinOp {
                            dest: s,
                            op: IrBinOp::Add,
                            lhs: Operand::Value(a),
                            rhs: Operand::Value(term),
                            ty: int_ty,
                        },
                    );
                    s
                }
            });
        }
        let g = alloc_value(func);
        push_inst(
            func,
            preheader,
            Instruction::GetElementPtr {
                dest: g,
                base: ptr,
                offset: Operand::Value(acc.expect("non-empty terms")),
                ty: IrType::I8,
            },
        );
        ptr = g;
    }
    if lin.base != 0 {
        let g = alloc_value(func);
        push_inst(
            func,
            preheader,
            Instruction::GetElementPtr {
                dest: g,
                base: ptr,
                offset: Operand::Const(IrConst::I64(lin.base)),
                ty: IrType::I8,
            },
        );
        ptr = g;
    }
    ptr
}

/// The SSA value that carries `lin.root`. For globals, the `GlobalAddr`
/// reachable from the load's own address chain is used — by construction it
/// dominates the loop and therefore the preheader.
fn root_value(func: &IrFunction, defs: &Defs, lin: &Lin, load_ptr: Value) -> Option<Value> {
    match lin.root.as_ref()? {
        Root::Alloca(id) | Root::Opaque(id) => Some(Value(*id)),
        Root::Global(name) => {
            let mut cur = load_ptr;
            for _ in 0..LIN_FUEL {
                let (b, i) = defs.get(cur)?;
                match &func.blocks[b].instructions[i] {
                    Instruction::GlobalAddr { dest, name: n } if n == name => return Some(*dest),
                    Instruction::GetElementPtr { base, .. } => cur = *base,
                    Instruction::Copy {
                        src: Operand::Value(s),
                        ..
                    }
                    | Instruction::Cast {
                        src: Operand::Value(s),
                        ..
                    } => cur = *s,
                    Instruction::Phi { incoming, .. } => {
                        // Follow the incoming that is defined outside the
                        // phi's own block (the init side).
                        let mut next = None;
                        for (o, _) in incoming {
                            if let Operand::Value(s) = o {
                                if defs.get(*s).map(|(db, _)| db) != Some(b) {
                                    next = Some(*s);
                                    break;
                                }
                            }
                        }
                        cur = next?;
                    }
                    _ => return None,
                }
            }
            None
        }
    }
}

fn run_round(func: &mut IrFunction, globals: &FxHashMap<String, usize>, env: Env) -> usize {
    let cfg = CfgAnalysis::build(func);
    let loops = loop_analysis::find_merged_natural_loops(
        func.blocks.len(),
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    );
    if loops.is_empty() {
        return 0;
    }
    let bound = func.sound_next_value_id();
    func.next_value_id = bound;
    let dom = DominanceChecker::new(func.blocks.len(), &cfg.idom);
    let defs = Defs::build(func, bound as usize);
    let ptr_ty_size = target_ptr_size();
    let int_ty = if ptr_ty_size == 8 {
        IrType::I64
    } else {
        IrType::I32
    };

    struct Plan {
        header: usize,
        preheader: usize,
        ph_label: BlockId,
        latch_label: BlockId,
        rewrites: Vec<Rewrite>,
    }
    let mut plans: Vec<Plan> = Vec::new();
    {
        let cx = Ctx {
            func: &*func,
            dom: &dom,
            idom: &cfg.idom,
            defs: &defs,
            globals,
            ptr_ty_size,
            env,
        };
        for lp in &loops {
            let Some(preheader) = lp.find_preheader(&cfg.preds) else {
                continue;
            };
            let Some(latch) = lp.single_latch(&cfg.preds) else {
                continue;
            };
            let sh = LoopShape {
                lp,
                header: lp.header,
                preheader,
                latch,
                preheader_label: func.blocks[preheader].label,
                latch_label: func.blocks[latch].label,
                exiting: lp.exiting_blocks(&cfg.succs),
            };
            let mut rewrites = Vec::new();
            plan_loop(&cx, &sh, &mut rewrites);
            if !rewrites.is_empty() {
                plans.push(Plan {
                    header: sh.header,
                    preheader,
                    ph_label: sh.preheader_label,
                    latch_label: sh.latch_label,
                    rewrites,
                });
            }
        }
    }
    if plans.is_empty() {
        return 0;
    }

    // Apply. Loads are deleted last, from high to low index, so recorded
    // positions stay valid; renames are applied globally at the end (a
    // load's dest may be another rewrite's store value).
    let mut rename: FxHashMap<u32, Value> = FxHashMap::default();
    let mut deletions: Vec<(usize, usize)> = Vec::new();
    let mut applied = 0usize;
    for plan in plans {
        for rw in plan.rewrites {
            let Instruction::Load { ptr: load_ptr, .. } =
                func.blocks[rw.load_block].instructions[rw.load_idx].clone()
            else {
                continue;
            };
            let Some(root) = root_value(func, &defs, &rw.lin0, load_ptr) else {
                if env.debug {
                    eprintln!(
                        "[lcfwd] {}: could not materialise root for load {:?}",
                        func.name, rw.load_dest
                    );
                }
                continue;
            };
            let addr0 = materialise_addr(func, plan.preheader, root, &rw.lin0, int_ty);
            let q0 = alloc_value(func);
            push_inst(
                func,
                plan.preheader,
                Instruction::Load {
                    dest: q0,
                    ptr: addr0,
                    ty: rw.ty,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            );
            let q = alloc_value(func);
            insert_inst(
                func,
                plan.header,
                0,
                Instruction::Phi {
                    dest: q,
                    ty: rw.ty,
                    incoming: vec![
                        (Operand::Value(q0), plan.ph_label),
                        (rw.store_val.clone(), plan.latch_label),
                    ],
                },
            );
            // The header phi insertion shifted every recorded index in that
            // header by one (this rewrite's load and earlier deletions).
            for d in deletions.iter_mut() {
                if d.0 == plan.header {
                    d.1 += 1;
                }
            }
            let load_idx = if rw.load_block == plan.header {
                rw.load_idx + 1
            } else {
                rw.load_idx
            };
            deletions.push((rw.load_block, load_idx));
            rename.insert(rw.load_dest.0, q);
            applied += 1;
        }
    }
    if applied == 0 {
        return 0;
    }
    // Resolve rename chains (q of one rewrite may be the store value of
    // another whose load was itself renamed).
    let resolve = |mut v: Value| {
        for _ in 0..LIN_FUEL {
            match rename.get(&v.0) {
                Some(n) if *n != v => v = *n,
                _ => break,
            }
        }
        v
    };
    for b in func.blocks.iter_mut() {
        for inst in b.instructions.iter_mut() {
            inst.for_each_value_use_mut(|v| *v = resolve(*v));
            inst.for_each_operand_mut(|o| {
                if let Operand::Value(v) = o {
                    *v = resolve(*v);
                }
            });
        }
        b.terminator.for_each_operand_mut(|o| {
            if let Operand::Value(v) = o {
                *v = resolve(*v);
            }
        });
    }
    deletions.sort_unstable_by(|a, b| b.cmp(a));
    deletions.dedup();
    for (b, i) in deletions {
        debug_assert!(matches!(
            func.blocks[b].instructions[i],
            Instruction::Load { .. }
        ));
        remove_inst(func, b, i);
    }
    applied
}

/// Module entry: global object sizes feed the dereferenceability proof.
pub(crate) fn run_module(module: &mut IrModule) -> usize {
    let env = Env::read();
    if env.disabled {
        return 0;
    }
    let globals: FxHashMap<String, usize> = module
        .globals
        .iter()
        .map(|g| (g.name.clone(), g.size))
        .collect();
    module.for_each_function(|f| {
        if f.blocks.is_empty() {
            return 0;
        }
        let mut total = 0;
        for _ in 0..MAX_ROUNDS {
            let n = run_round(f, &globals, env);
            if n == 0 {
                break;
            }
            total += n;
        }
        total
    })
}
