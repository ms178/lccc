//! Vector fill: strip-mine `for (i = 0; i < n; i++) p[i] = v` into 16-byte
//! stores of a splatted `v` (x86-64, SSE2 and up).
//!
//! `loop_memset` already turns byte-splat fills (`0`, `-1`, any `char`) into
//! `memset(3)`.  Every other fill -- a 16/32/64-bit element whose value is not
//! one repeated byte (`head[i] = 0xFFFF` in gzip's hash-chain setup, a float
//! `1.0f`, a run-time `v`) -- stayed a scalar loop of one store per element,
//! which neither GCC nor Clang leaves scalar at `-O2`.
//!
//! ## Shape
//!
//! The canonical while-form loop the late pipeline presents:
//!
//! ```text
//!   P:  ... br H
//!   H:  iv = phi [0, P], [iv + 1, B]
//!       c  = cmp {ult | ne | slt} iv, n          (n loop-invariant)
//!       condbr c, B, X
//!   B:  addr = GEP(base, s*iv + d)               (affine chain, 64-bit)
//!       store v, addr                            (v loop-invariant)
//!       iv + 1 ; br H
//! ```
//!
//! with `s` equal to the store width `w` in {2, 4, 8} (contiguous elements).
//!
//! ## Transform
//!
//! ```text
//!   VP: span = n clamped to >= 0 (signed exit test), in 64 bits
//!       vend = span & -L            (L = 32 / w elements per vector step)
//!       splat = VecPackI32x4(pattern(v))
//!   VH: vb = phi [0, VP], [vb + 32, VB] ; condbr vb <u vend*w, VB, H
//!   VB: VecStoreI32x4 splat -> base + vb + d ; ... + d + 16 ; br VH
//!   H:  iv = phi [vend, VH], [iv + 1, B]          (only the init changes)
//! ```
//!
//! The original loop is kept and runs the `< L` remaining iterations, so it
//! still defines every value the exit block reads: no exit-value
//! reconstruction, no new phi at `X`.
//!
//! ## Why this is exact
//!
//! * The loop body has no load and no other side effect, so the only
//!   observable effect is the final memory image of `[base + d, base + d +
//!   n*w)`: the vector steps write the same bytes with the same values for
//!   the elements `[0, vend)` and the scalar loop writes `[vend, n)`.
//!   Grouping the writes differently is only observable by a racing access,
//!   which is undefined in C.  `volatile` stores are refused.
//! * The vector byte offset `vb` is the scalar `s*iv` only when the scalar
//!   address arithmetic does not wrap or change sign: the chain must be
//!   64-bit, and a 32-bit induction variable may be widened by a sign
//!   extension only under a signed exit test (`0 <= iv < n <= INT_MAX`); an
//!   unsigned or `!=` test lets `iv` pass `2^31`, where `sext` and the vector
//!   offset disagree, so it needs a zero extension.
//! * A signed bound `n < 0` means zero iterations: `span` is clamped to 0, so
//!   `vend = 0` and the scalar loop exits immediately, as before.
//! * `n` is evaluated once in `VP` instead of every iteration: it is
//!   loop-invariant (defined outside the loop), so the values agree.
//!
//! Kill switch: `CCC_DISABLE_PASSES=vector_fill`.

use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IntrinsicOp, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand,
    Terminator, Value,
};

/// Bytes written per vector-loop iteration (two 16-byte stores: the store
/// pair GCC emits for the same loop, and enough to amortize the three loop
/// control instructions).
const STEP_BYTES: i64 = 32;

/// Rewrite every recognized fill loop of `func`; returns how many changed.
/// `simd` is the function's effective SSE2 availability.
pub fn run(func: &mut IrFunction, simd: bool) -> usize {
    if !simd || func.is_declaration {
        return 0;
    }
    let mut changed = 0;
    // Each rewrite adds blocks but never exposes a new candidate (the new
    // loop stores a vector, which is not a recognized scalar store); the
    // bound only guards the sweep.
    for _ in 0..64 {
        let Some(plan) = find_plan(func) else {
            break;
        };
        rewrite(func, &plan);
        changed += 1;
    }
    changed
}

/// A recognized fill loop.
#[derive(Debug, Clone)]
struct Plan {
    header: BlockId,
    pre: BlockId,
    /// Set when the scalar loop is provably dead after the vector loop (a
    /// constant trip count that is a multiple of the vector step, no loop
    /// value read outside the loop, no phi at the exit): the vector loop then
    /// leaves straight to this exit block.
    direct_exit: Option<BlockId>,
    /// The induction-variable phi and its type.
    iv_phi: Value,
    iv_ty: IrType,
    /// Exit test and loop-invariant bound.
    cmp: IrCmpOp,
    bound: Operand,
    /// Store: base pointer, constant byte displacement, element width.
    base: Value,
    disp: i64,
    width: u32,
    /// The stored value and the store's IR type.
    val: Operand,
    store_ty: IrType,
}

fn is_int(ty: IrType) -> bool {
    matches!(
        ty,
        IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
    )
}

fn int_width(ty: IrType) -> Option<u32> {
    match ty {
        IrType::I8 | IrType::U8 => Some(1),
        IrType::I16 | IrType::U16 => Some(2),
        IrType::I32 | IrType::U32 => Some(4),
        IrType::I64 | IrType::U64 => Some(8),
        _ => None,
    }
}

fn is_unsigned(ty: IrType) -> bool {
    matches!(ty, IrType::U8 | IrType::U16 | IrType::U32 | IrType::U64)
}

fn block_index(func: &IrFunction, label: BlockId) -> Option<usize> {
    func.blocks.iter().position(|b| b.label == label)
}

fn successors(t: &Terminator) -> Vec<BlockId> {
    match t {
        Terminator::Branch(l) => vec![*l],
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => vec![*true_label, *false_label],
        _ => Vec::new(),
    }
}

/// Predecessor labels of `label`, `None` if some terminator can reach blocks
/// this pass does not model (switches, indirect branches) -- then the
/// predecessor set is not provably complete.
fn predecessors(func: &IrFunction, label: BlockId) -> Option<Vec<BlockId>> {
    let mut preds = Vec::new();
    for b in &func.blocks {
        match &b.terminator {
            Terminator::Branch(_) | Terminator::CondBranch { .. } => {
                if successors(&b.terminator).contains(&label) {
                    preds.push(b.label);
                }
            }
            Terminator::Return(_) | Terminator::Unreachable => {}
            _ => return None,
        }
    }
    Some(preds)
}

/// Every value defined in `blocks`.
fn defined_in(func: &IrFunction, labels: &[BlockId]) -> Vec<u32> {
    func.blocks
        .iter()
        .filter(|b| labels.contains(&b.label))
        .flat_map(|b| b.instructions.iter().filter_map(Instruction::dest))
        .map(|v| v.0)
        .collect()
}

/// True when an instruction or terminator outside `labels` reads a value
/// defined inside them.
fn loop_values_escape(func: &IrFunction, labels: &[BlockId], defs: &[u32]) -> bool {
    func.blocks
        .iter()
        .filter(|b| !labels.contains(&b.label))
        .any(|b| {
            let mut hit = false;
            for inst in &b.instructions {
                inst.for_each_used_value(|v| hit |= defs.contains(&v));
            }
            b.terminator
                .for_each_used_value(|v| hit |= defs.contains(&v));
            hit
        })
}

fn find_plan(func: &IrFunction) -> Option<Plan> {
    (0..func.blocks.len()).find_map(|h| recognize(func, h))
}

fn recognize(func: &IrFunction, h: usize) -> Option<Plan> {
    let hb = &func.blocks[h];
    let Terminator::CondBranch {
        cond: Operand::Value(cond),
        true_label: body_label,
        false_label: exit_label,
    } = &hb.terminator
    else {
        return None;
    };
    if *body_label == hb.label || *exit_label == hb.label || body_label == exit_label {
        return None;
    }
    let b = block_index(func, *body_label)?;
    let bb = &func.blocks[b];
    if !matches!(bb.terminator, Terminator::Branch(l) if l == hb.label) {
        return None;
    }
    if predecessors(func, bb.label)? != vec![hb.label] {
        return None;
    }
    let preds = predecessors(func, hb.label)?;
    let [p0, p1] = preds.as_slice() else {
        return None;
    };
    let pre = if *p0 == bb.label { *p1 } else { *p0 };
    if pre == bb.label || pre == hb.label {
        return None;
    }

    // H: exactly the iv phi and the exit compare.
    let [
        Instruction::Phi {
            dest: iv_phi,
            ty: iv_ty,
            incoming,
        },
        Instruction::Cmp {
            dest: cmp_dest,
            op: cmp,
            lhs: Operand::Value(cmp_lhs),
            rhs: bound,
            ty: cmp_ty,
        },
    ] = hb.instructions.as_slice()
    else {
        return None;
    };
    if cmp_dest != cond || cmp_lhs != iv_phi || cmp_ty != iv_ty {
        return None;
    }
    if !matches!(int_width(*iv_ty), Some(4 | 8)) {
        return None;
    }
    if !matches!(cmp, IrCmpOp::Ult | IrCmpOp::Ne | IrCmpOp::Slt) {
        return None;
    }
    let signed_exit = *cmp == IrCmpOp::Slt;
    let [(init_a, blk_a), (init_b, blk_b)] = incoming.as_slice() else {
        return None;
    };
    let ((init, _), (next, _)) = if *blk_a == pre && *blk_b == bb.label {
        ((init_a, blk_a), (init_b, blk_b))
    } else if *blk_b == pre && *blk_a == bb.label {
        ((init_b, blk_b), (init_a, blk_a))
    } else {
        return None;
    };
    if !matches!(init, Operand::Const(c) if c.to_i64() == Some(0)) {
        return None;
    }
    let Operand::Value(next) = next else {
        return None;
    };
    let loop_defs = defined_in(func, &[hb.label, bb.label]);
    if let Operand::Value(v) = bound {
        if loop_defs.contains(&v.0) {
            return None;
        }
    }

    // B: the affine address chain, one store, and the iv increment.
    // `affine[v] = (scale, offset)`: v == scale*iv + offset, exactly.
    let mut affine: Vec<(u32, i64, i64)> = vec![(iv_phi.0, 1, 0)];
    let lookup = |aff: &[(u32, i64, i64)], op: &Operand| -> Option<(i64, i64)> {
        match op {
            Operand::Value(v) => aff.iter().find(|e| e.0 == v.0).map(|e| (e.1, e.2)),
            Operand::Const(_) => None,
        }
    };
    let mut gep: Option<(Value, Value, i64, i64)> = None; // (dest, base, scale, off)
    let mut store: Option<(Operand, Value, IrType)> = None;
    let mut saw_next = false;
    for inst in &bb.instructions {
        match inst {
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs: Operand::Value(l),
                rhs: Operand::Const(c),
                ty,
            } if dest == next && l == iv_phi && ty == iv_ty && c.to_i64() == Some(1) => {
                saw_next = true;
            }
            Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } => {
                let (s, o) = lookup(&affine, src)?;
                if !is_int(*from_ty) || int_width(*to_ty) != Some(8) {
                    return None;
                }
                match int_width(*from_ty)? {
                    8 => {}
                    // Widening a 32-bit iv: exact for a zero extension
                    // always, for a sign extension only while `iv < 2^31`,
                    // which the signed exit test guarantees.
                    4 if s == 1 && o == 0 && (is_unsigned(*from_ty) || signed_exit) => {}
                    _ => return None,
                }
                affine.push((dest.0, s, o));
            }
            Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs: Operand::Const(c),
                ty,
            } if int_width(*ty) == Some(8) => {
                let (s, o) = lookup(&affine, lhs)?;
                let k = c.to_i64()?;
                let (s, o) = match op {
                    IrBinOp::Shl if (0..8).contains(&k) => {
                        let m = 1i64 << k;
                        (s.checked_mul(m)?, o.checked_mul(m)?)
                    }
                    IrBinOp::Mul => (s.checked_mul(k)?, o.checked_mul(k)?),
                    IrBinOp::Add => (s, o.checked_add(k)?),
                    IrBinOp::Sub => (s, o.checked_sub(k)?),
                    _ => return None,
                };
                affine.push((dest.0, s, o));
            }
            // `iv*2` reaches here canonicalized as `x + x`.
            Instruction::BinOp {
                dest,
                op: op @ (IrBinOp::Add | IrBinOp::Sub),
                lhs,
                rhs: rhs @ Operand::Value(_),
                ty,
            } if int_width(*ty) == Some(8) => {
                let (sa, oa) = lookup(&affine, lhs)?;
                let (sb, ob) = lookup(&affine, rhs)?;
                let (s, o) = if *op == IrBinOp::Add {
                    (sa.checked_add(sb)?, oa.checked_add(ob)?)
                } else {
                    (sa.checked_sub(sb)?, oa.checked_sub(ob)?)
                };
                affine.push((dest.0, s, o));
            }
            Instruction::GetElementPtr {
                dest, base, offset, ..
            } if gep.is_none() => {
                if loop_defs.contains(&base.0) {
                    return None;
                }
                let (s, o) = match offset {
                    Operand::Const(c) => (0, c.to_i64()?),
                    off => lookup(&affine, off)?,
                };
                gep = Some((*dest, *base, s, o));
            }
            Instruction::Store {
                val,
                ptr,
                ty,
                seg_override,
                volatile,
            } if store.is_none() => {
                if *volatile || *seg_override != AddressSpace::Default {
                    return None;
                }
                store = Some((val.clone(), *ptr, *ty));
            }
            _ => return None,
        }
    }
    let (gep_dest, base, scale, disp) = gep?;
    let (val, ptr, store_ty) = store?;
    if !saw_next || ptr != gep_dest {
        return None;
    }
    let width = match store_ty {
        IrType::F32 => 4,
        IrType::F64 => 8,
        ty => int_width(ty)?,
    };
    // Width 1 is memset's (any byte is a splat); a non-unit stride is not a
    // fill.  The displacement must stay a legal x86 disp32 for `+16`.
    if width == 1 || scale != i64::from(width) || i32::try_from(disp + 16).is_err() {
        return None;
    }
    // Every intermediate of the chain must feed only the chain: an address
    // value read elsewhere would be a second effect of the loop.  Values of B
    // cannot be used outside the loop (B does not dominate the exit), so it
    // suffices that nothing in B other than the chain reads them -- the
    // match above admitted nothing else.
    match &val {
        Operand::Value(v) if loop_defs.contains(&v.0) => return None,
        Operand::Value(_) if matches!(store_ty, IrType::F32 | IrType::F64) => return None,
        Operand::Const(c) if const_bits(c, store_ty).is_none() => return None,
        _ => {}
    }
    // A constant trip count too small for one vector step gains nothing.
    let mut direct_exit = None;
    if let Operand::Const(c) = bound {
        let n = c.to_i64()?;
        let lanes = STEP_BYTES / i64::from(width);
        if n < lanes {
            return None;
        }
        if n % lanes == 0 && !loop_values_escape(func, &[hb.label, bb.label], &loop_defs) {
            let x = block_index(func, *exit_label)?;
            if !func.blocks[x]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Phi { .. }))
            {
                direct_exit = Some(*exit_label);
            }
        }
    }
    Some(Plan {
        header: hb.label,
        direct_exit,
        pre,
        iv_phi: *iv_phi,
        iv_ty: *iv_ty,
        cmp: *cmp,
        bound: bound.clone(),
        base,
        disp,
        width,
        val,
        store_ty,
    })
}

/// The stored bit pattern of a constant (low `width` bytes), or `None` when
/// the constant's kind does not match the store type.
fn const_bits(c: &IrConst, store_ty: IrType) -> Option<u64> {
    match (store_ty, c) {
        (IrType::F32, IrConst::F32(f)) => Some(u64::from(f.to_bits())),
        (IrType::F64, IrConst::F64(f)) => Some(f.to_bits()),
        (IrType::F32 | IrType::F64, _) => None,
        (ty, c) => {
            let w = int_width(ty)?;
            let bits = c.to_i64()? as u64;
            Some(if w == 8 {
                bits
            } else {
                bits & ((1u64 << (8 * w)) - 1)
            })
        }
    }
}

/// The four I32 lanes of a 16-byte vector holding `bits` repeated at `width`.
fn splat_lanes(bits: u64, width: u32) -> [i32; 4] {
    let lo = match width {
        2 => (bits & 0xFFFF) * 0x0001_0001,
        _ => bits & 0xFFFF_FFFF,
    } as u32 as i32;
    let hi = if width == 8 {
        (bits >> 32) as u32 as i32
    } else {
        lo
    };
    [lo, hi, lo, hi]
}

struct Emitter {
    next: u32,
}

impl Emitter {
    fn fresh(&mut self) -> Value {
        let v = Value(self.next);
        self.next += 1;
        v
    }
}

fn i64c(v: i64) -> Operand {
    Operand::Const(IrConst::I64(v))
}

fn rewrite(func: &mut IrFunction, plan: &Plan) {
    let mut em = Emitter {
        next: func.next_value_id,
    };
    let live_max = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
    let base_label = func.next_label.max(live_max.saturating_add(1));
    func.next_label = base_label.saturating_add(3);
    let (vp, vh, vb) = (
        BlockId(base_label),
        BlockId(base_label + 1),
        BlockId(base_label + 2),
    );
    let w = i64::from(plan.width);
    let lanes_per_step = STEP_BYTES / w;
    let shift = i64::from(plan.width.trailing_zeros());
    let wide = int_width(plan.iv_ty) == Some(8);

    // ---- VP: trip span, vector end, splat ----------------------------------------
    let mut pre = Vec::new();
    let span64 = if wide {
        plan.bound.clone()
    } else {
        let d = em.fresh();
        // Unsigned / `!=` exits count the bound as unsigned; `<s` as signed.
        let (from, to) = if plan.cmp == IrCmpOp::Slt {
            (IrType::I32, IrType::I64)
        } else {
            (IrType::U32, IrType::U64)
        };
        pre.push(Instruction::Cast {
            dest: d,
            src: plan.bound.clone(),
            from_ty: from,
            to_ty: to,
        });
        Operand::Value(d)
    };
    let span = if plan.cmp == IrCmpOp::Slt {
        let neg = em.fresh();
        pre.push(Instruction::Cmp {
            dest: neg,
            op: IrCmpOp::Slt,
            lhs: span64.clone(),
            rhs: i64c(0),
            ty: IrType::I64,
        });
        let clamped = em.fresh();
        pre.push(Instruction::Select {
            dest: clamped,
            cond: Operand::Value(neg),
            true_val: i64c(0),
            false_val: span64,
            ty: IrType::I64,
        });
        Operand::Value(clamped)
    } else {
        span64
    };
    let vend = em.fresh();
    pre.push(Instruction::BinOp {
        dest: vend,
        op: IrBinOp::And,
        lhs: span,
        rhs: i64c(-lanes_per_step),
        ty: IrType::I64,
    });
    let vend_bytes = em.fresh();
    pre.push(Instruction::BinOp {
        dest: vend_bytes,
        op: IrBinOp::Shl,
        lhs: Operand::Value(vend),
        rhs: i64c(shift),
        ty: IrType::I64,
    });
    let iv_init = if wide {
        Operand::Value(vend)
    } else {
        let d = em.fresh();
        pre.push(Instruction::Cast {
            dest: d,
            src: Operand::Value(vend),
            from_ty: IrType::U64,
            to_ty: plan.iv_ty,
        });
        Operand::Value(d)
    };
    let lanes: [Operand; 4] = match &plan.val {
        Operand::Const(c) => {
            // `recognize` admitted only constants with a bit pattern.
            let bits = const_bits(c, plan.store_ty).unwrap_or(0);
            splat_lanes(bits, plan.width).map(|x| Operand::Const(IrConst::I32(x)))
        }
        Operand::Value(v) => match plan.width {
            4 => std::array::from_fn(|_| Operand::Value(*v)),
            2 => {
                let z = em.fresh();
                pre.push(Instruction::Cast {
                    dest: z,
                    src: Operand::Value(*v),
                    from_ty: IrType::U16,
                    to_ty: IrType::U32,
                });
                let p = em.fresh();
                pre.push(Instruction::BinOp {
                    dest: p,
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(z),
                    rhs: Operand::Const(IrConst::I32(0x0001_0001)),
                    ty: IrType::U32,
                });
                std::array::from_fn(|_| Operand::Value(p))
            }
            _ => {
                let lo = em.fresh();
                pre.push(Instruction::Cast {
                    dest: lo,
                    src: Operand::Value(*v),
                    from_ty: IrType::U64,
                    to_ty: IrType::U32,
                });
                let sh = em.fresh();
                pre.push(Instruction::BinOp {
                    dest: sh,
                    op: IrBinOp::LShr,
                    lhs: Operand::Value(*v),
                    rhs: i64c(32),
                    ty: IrType::U64,
                });
                let hi = em.fresh();
                pre.push(Instruction::Cast {
                    dest: hi,
                    src: Operand::Value(sh),
                    from_ty: IrType::U64,
                    to_ty: IrType::U32,
                });
                [
                    Operand::Value(lo),
                    Operand::Value(hi),
                    Operand::Value(lo),
                    Operand::Value(hi),
                ]
            }
        },
    };
    let splat = em.fresh();
    pre.push(Instruction::Intrinsic {
        dest: Some(splat),
        op: IntrinsicOp::VecPackI32x4,
        dest_ptr: None,
        args: lanes.to_vec(),
    });

    // ---- VH / VB: the 32-byte store loop over the byte offset ------------------------
    let vb_phi = em.fresh();
    let vb_next = em.fresh();
    let vcond = em.fresh();
    let head = vec![
        Instruction::Phi {
            dest: vb_phi,
            ty: IrType::I64,
            incoming: vec![(i64c(0), vp), (Operand::Value(vb_next), vb)],
        },
        Instruction::Cmp {
            dest: vcond,
            op: IrCmpOp::Ult,
            lhs: Operand::Value(vb_phi),
            rhs: Operand::Value(vend_bytes),
            ty: IrType::I64,
        },
    ];
    let store_at = |disp: i64| Instruction::Intrinsic {
        dest: None,
        op: IntrinsicOp::VecStoreI32x4,
        dest_ptr: None,
        args: vec![
            Operand::Value(splat),
            Operand::Value(plan.base),
            Operand::Value(vb_phi),
            i64c(disp),
        ],
    };
    let body = vec![
        store_at(plan.disp),
        store_at(plan.disp + 16),
        Instruction::BinOp {
            dest: vb_next,
            op: IrBinOp::Add,
            lhs: Operand::Value(vb_phi),
            rhs: i64c(STEP_BYTES),
            ty: IrType::I64,
        },
    ];

    let block = |label, instructions, terminator| BasicBlock {
        label,
        instructions,
        terminator,
        source_spans: Vec::new(),
    };
    let new_blocks = [
        block(vp, pre, Terminator::Branch(vh)),
        block(
            vh,
            head,
            Terminator::CondBranch {
                cond: Operand::Value(vcond),
                true_label: vb,
                false_label: plan.direct_exit.unwrap_or(plan.header),
            },
        ),
        block(vb, body, Terminator::Branch(vh)),
    ];

    // ---- splice: P -> VP, H's phi enters from VH at the vector end -------------
    if let Some(p) = block_index(func, plan.pre) {
        match &mut func.blocks[p].terminator {
            Terminator::Branch(l) if *l == plan.header => *l = vp,
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                for l in [true_label, false_label] {
                    if *l == plan.header {
                        *l = vp;
                    }
                }
            }
            _ => {}
        }
    }
    let h = block_index(func, plan.header).unwrap_or(0);
    for inst in &mut func.blocks[h].instructions {
        if let Instruction::Phi { dest, incoming, .. } = inst {
            if *dest == plan.iv_phi {
                for (op, blk) in incoming.iter_mut() {
                    if *blk == plan.pre {
                        *op = iv_init.clone();
                        *blk = vh;
                    }
                }
            }
        }
    }
    for (k, b) in new_blocks.into_iter().enumerate() {
        func.blocks.insert(h + k, b);
    }
    func.next_value_id = em.next;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(v: u32) -> Operand {
        Operand::Value(Value(v))
    }

    fn bb(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    /// `for (i = 0; i < n; i++) p[i] = v` in the late while form:
    /// 0 = P, 1 = H, 2 = B, 3 = X.  Values: 1 base, 2 bound, 3 stored value,
    /// 10 iv phi, 11 cmp, 12 cast, 13 shl, 14 gep, 15 iv + 1.
    fn fill(
        iv_ty: IrType,
        cmp: IrCmpOp,
        cast_from: IrType,
        store_ty: IrType,
        v: Operand,
    ) -> IrFunction {
        let mut f = IrFunction::new("fill".into(), IrType::Void, vec![], false);
        f.next_value_id = 20;
        f.next_label = 4;
        let w = match store_ty {
            IrType::F32 => 4,
            IrType::F64 => 8,
            ty => int_width(ty).unwrap_or(1),
        };
        f.blocks.push(bb(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 1,
                    ty: iv_ty,
                },
            ],
            Terminator::Branch(BlockId(1)),
        ));
        f.blocks.push(bb(
            1,
            vec![
                Instruction::Phi {
                    dest: Value(10),
                    ty: iv_ty,
                    incoming: vec![(i64c(0), BlockId(0)), (val(15), BlockId(2))],
                },
                Instruction::Cmp {
                    dest: Value(11),
                    op: cmp,
                    lhs: val(10),
                    rhs: val(2),
                    ty: iv_ty,
                },
            ],
            Terminator::CondBranch {
                cond: val(11),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ));
        f.blocks.push(bb(
            2,
            vec![
                Instruction::Cast {
                    dest: Value(12),
                    src: val(10),
                    from_ty: cast_from,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(13),
                    op: IrBinOp::Shl,
                    lhs: val(12),
                    rhs: i64c(i64::from(w).trailing_zeros().into()),
                    ty: IrType::I64,
                },
                Instruction::GetElementPtr {
                    dest: Value(14),
                    base: Value(1),
                    offset: val(13),
                    ty: IrType::Ptr,
                },
                Instruction::Store {
                    val: v,
                    ptr: Value(14),
                    ty: store_ty,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::BinOp {
                    dest: Value(15),
                    op: IrBinOp::Add,
                    lhs: val(10),
                    rhs: i64c(1),
                    ty: iv_ty,
                },
            ],
            Terminator::Branch(BlockId(1)),
        ));
        f.blocks.push(bb(3, vec![], Terminator::Return(None)));
        f
    }

    fn u32_fill() -> IrFunction {
        fill(
            IrType::U64,
            IrCmpOp::Ult,
            IrType::U64,
            IrType::U32,
            Operand::Const(IrConst::I64(0xFFFF)),
        )
    }

    #[test]
    fn fill_loop_is_strip_mined_and_the_scalar_loop_resumes_at_the_vector_end() {
        let mut f = u32_fill();
        assert_eq!(run(&mut f, true), 1);
        let labels: Vec<u32> = f.blocks.iter().map(|b| b.label.0).collect();
        // VP, VH, VB sit between P and the original header.
        assert_eq!(labels, vec![0, 4, 5, 6, 1, 2, 3]);
        assert!(matches!(
            f.blocks[0].terminator,
            Terminator::Branch(BlockId(4))
        ));
        // The scalar header now enters from VH with the vector end.
        let Instruction::Phi { incoming, .. } = &f.blocks[4].instructions[0] else {
            panic!("H phi");
        };
        assert!(incoming.iter().any(|(_, b)| *b == BlockId(5)));
        assert!(!incoming.iter().any(|(_, b)| *b == BlockId(0)));
        // The vector body: two 16-byte stores of the splat, 32 bytes apart steps.
        let stores: Vec<&Instruction> = f.blocks[3]
            .instructions
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::Intrinsic {
                        op: IntrinsicOp::VecStoreI32x4,
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(stores.len(), 2);
        // The splat is the 32-bit pattern in every lane.
        let splat = f.blocks[1]
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::Intrinsic {
                    op: IntrinsicOp::VecPackI32x4,
                    args,
                    ..
                } => Some(args.clone()),
                _ => None,
            })
            .expect("splat");
        assert_eq!(splat, vec![Operand::Const(IrConst::I32(0xFFFF)); 4]);
        // A second run finds nothing new.
        assert_eq!(run(&mut f, true), 0);
    }

    #[test]
    fn splat_lanes_repeat_the_element_pattern() {
        assert_eq!(splat_lanes(0x1234, 2), [0x1234_1234; 4]);
        assert_eq!(splat_lanes(0xFFFF, 4), [0xFFFF; 4]);
        assert_eq!(
            splat_lanes(0x1122_3344_5566_7788, 8),
            [0x5566_7788, 0x1122_3344, 0x5566_7788, 0x1122_3344]
        );
        assert_eq!(const_bits(&IrConst::I64(-2), IrType::U16), Some(0xFFFE));
        assert_eq!(
            const_bits(&IrConst::F32(1.0), IrType::F32),
            Some(0x3F80_0000)
        );
        assert_eq!(const_bits(&IrConst::F64(1.0), IrType::F32), None);
    }

    #[test]
    fn refuses_what_it_cannot_prove() {
        // No SSE2.
        assert_eq!(run(&mut u32_fill(), false), 0);
        // Byte stores belong to memset.
        let mut f = fill(
            IrType::U64,
            IrCmpOp::Ult,
            IrType::U64,
            IrType::U8,
            Operand::Const(IrConst::I64(7)),
        );
        assert_eq!(run(&mut f, true), 0);
        // A sign-extended 32-bit iv under an unsigned (or `!=`) exit can pass
        // 2^31, where sext and the vector byte offset disagree.
        for cmp in [IrCmpOp::Ult, IrCmpOp::Ne] {
            let mut f = fill(
                IrType::I32,
                cmp,
                IrType::I32,
                IrType::U32,
                Operand::Const(IrConst::I64(9)),
            );
            assert_eq!(run(&mut f, true), 0, "{cmp:?}");
        }
        // ...but is exact under a signed exit, and zext is exact under any.
        let mut f = fill(
            IrType::I32,
            IrCmpOp::Slt,
            IrType::I32,
            IrType::U32,
            Operand::Const(IrConst::I64(9)),
        );
        assert_eq!(run(&mut f, true), 1);
        let mut f = fill(
            IrType::U32,
            IrCmpOp::Ne,
            IrType::U32,
            IrType::U32,
            Operand::Const(IrConst::I64(9)),
        );
        assert_eq!(run(&mut f, true), 1);
        // `<=` runs n + 1 times: not modelled.
        let mut f = fill(
            IrType::U64,
            IrCmpOp::Ule,
            IrType::U64,
            IrType::U32,
            Operand::Const(IrConst::I64(9)),
        );
        assert_eq!(run(&mut f, true), 0);
        // Volatile store.
        let mut f = u32_fill();
        if let Instruction::Store { volatile, .. } = &mut f.blocks[2].instructions[3] {
            *volatile = true;
        }
        assert_eq!(run(&mut f, true), 0);
        // A value computed inside the loop is not a fill.
        let mut f = fill(IrType::U64, IrCmpOp::Ult, IrType::U64, IrType::U64, val(13));
        assert_eq!(run(&mut f, true), 0);
        // A stride that is not the element width (p[2*i]).
        let mut f = u32_fill();
        if let Instruction::BinOp { rhs, .. } = &mut f.blocks[2].instructions[1] {
            *rhs = i64c(3);
        }
        assert_eq!(run(&mut f, true), 0);
        // Any other instruction in the body (here a load).
        let mut f = u32_fill();
        f.blocks[2].instructions.insert(
            0,
            Instruction::Load {
                dest: Value(16),
                ptr: Value(1),
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        );
        assert_eq!(run(&mut f, true), 0);
        // A runtime float value would need a bit move: not handled.
        let mut f = fill(IrType::U64, IrCmpOp::Ult, IrType::U64, IrType::F32, val(2));
        assert_eq!(run(&mut f, true), 0);
    }

    #[test]
    fn signed_exit_clamps_a_negative_bound_and_runtime_values_are_splatted() {
        let mut f = fill(IrType::I32, IrCmpOp::Slt, IrType::I32, IrType::U16, val(2));
        assert_eq!(run(&mut f, true), 1);
        let vp = &f.blocks[1].instructions;
        assert!(
            vp.iter().any(|i| matches!(i, Instruction::Select { .. })),
            "clamp"
        );
        assert!(
            vp.iter().any(|i| matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::Mul,
                    rhs: Operand::Const(IrConst::I32(0x0001_0001)),
                    ..
                }
            )),
            "u16 pattern doubling"
        );
        let mut f = fill(IrType::U64, IrCmpOp::Ult, IrType::U64, IrType::U64, val(2));
        assert_eq!(run(&mut f, true), 1);
        let vp = &f.blocks[1].instructions;
        assert!(
            vp.iter().any(|i| matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::LShr,
                    ..
                }
            )),
            "hi half"
        );
    }

    #[test]
    fn sixteen_bit_doubling_by_self_add_is_affine() {
        let mut f = fill(
            IrType::U64,
            IrCmpOp::Ult,
            IrType::U64,
            IrType::U16,
            Operand::Const(IrConst::I64(0xBEEF)),
        );
        // `iv*2` as the pipeline presents it: `x + x`.
        f.blocks[2].instructions[1] = Instruction::BinOp {
            dest: Value(13),
            op: IrBinOp::Add,
            lhs: val(12),
            rhs: val(12),
            ty: IrType::I64,
        };
        assert_eq!(run(&mut f, true), 1);
        // `x - x` is not the element stride.
        let mut f = fill(
            IrType::U64,
            IrCmpOp::Ult,
            IrType::U64,
            IrType::U16,
            Operand::Const(IrConst::I64(0xBEEF)),
        );
        f.blocks[2].instructions[1] = Instruction::BinOp {
            dest: Value(13),
            op: IrBinOp::Sub,
            lhs: val(12),
            rhs: val(12),
            ty: IrType::I64,
        };
        assert_eq!(run(&mut f, true), 0);
    }

    fn vh_exit(f: &IrFunction) -> BlockId {
        match f.blocks[2].terminator {
            Terminator::CondBranch { false_label, .. } => false_label,
            _ => panic!("VH must end in a conditional branch"),
        }
    }

    #[test]
    fn constant_multiple_trip_count_skips_the_dead_scalar_loop() {
        let with_bound = |n: i64| {
            let mut f = u32_fill();
            if let Instruction::Cmp { rhs, .. } = &mut f.blocks[1].instructions[1] {
                *rhs = i64c(n);
            }
            f
        };
        // 64 u32 elements = 8 vector steps exactly: VH leaves straight to X.
        let mut f = with_bound(64);
        assert_eq!(run(&mut f, true), 1);
        assert_eq!(vh_exit(&f), BlockId(3));
        // 65: one scalar element remains, so VH enters the scalar header.
        let mut f = with_bound(65);
        assert_eq!(run(&mut f, true), 1);
        assert_eq!(vh_exit(&f), BlockId(1));
        // Fewer elements than one vector step: nothing to gain.
        let mut f = with_bound(7);
        assert_eq!(run(&mut f, true), 0);
        // The exit reads the induction variable: the scalar header must run
        // (it defines the value), even though no element remains.
        let mut f = with_bound(64);
        f.blocks[3].terminator = Terminator::Return(Some(val(10)));
        assert_eq!(run(&mut f, true), 1);
        assert_eq!(vh_exit(&f), BlockId(1));
    }
}
